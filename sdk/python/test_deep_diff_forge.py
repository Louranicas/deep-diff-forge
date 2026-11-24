"""Tests for the Python client against the real binary: DDF_BIN names it (default: target/debug).

Run: DDF_BIN=target/debug/deep-diff-forge python3 -I sdk/python/test_deep_diff_forge.py
"""
import hashlib
import json
import os
import pathlib
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import time
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import deep_diff_forge as ddf  # noqa: E402

REPO = pathlib.Path(__file__).resolve().parents[2]
BIN = os.environ.get("DDF_BIN", str(REPO / "target" / "debug" / "deep-diff-forge"))
PATCH = "--- a/x\n+++ b/x\n@@ -1,1 +1,1 @@\n-old\n+new\n"
REFUSAL = '{"jsonrpc":"2.0","id":1,"error":{"code":-32600,"message":"too long"}}'
# A stand-in for `deep-diff-forge rpc` that answers each request line with an empty result.
ECHO = ("import json, sys\nfor line in sys.stdin:\n    request = json.loads(line)\n"
        "    print(json.dumps({'jsonrpc': '2.0', 'id': request['id'], 'result': {}}), flush=True)")


TRANSCRIPT = REPO / "fixtures" / "protocol" / "v2-transcript.jsonl"
MARKER = "zq7MARKER\u202e"


def paging_patch():
    """Six files of process spawns (three cut at 32 rows), a hidden character in each path and a
    line holding a bidi control: a diagnostics document of many pages at the smallest budget."""
    text = ""
    for f in range(6):
        path = f"src/m{f}/{'n' * 30 * f}\u200b.rs"
        lines = 25 + 5 * f
        text += f"diff --git a/{path} b/{path}\nnew file mode 100644\n--- /dev/null\n+++ b/{path}\n@@ -0,0 +1,{lines} @@\n"
        text += "".join(f'+std::process::Command::new("p{i}");\n' for i in range(lines))
    return text + "diff --git a/x.py b/x.py\n--- a/x.py\n+++ b/x.py\n@@ -1 +1 @@\n-a = 1\n+a = eval(s)  # \u202e\n"


def cli_diagnostics(patch):
    """The bytes ``--stdin-patch --diagnostics --json`` prints."""
    return subprocess.run([BIN, "--stdin-patch", "--diagnostics", "--json"], input=patch.encode(),
                          capture_output=True, check=True).stdout


# A stand-in daemon for ``StdioClient``: argv[1] is ``transcript`` (answer as the published 0.10.0
# binary's recorded transcript answers a request of the same method and ``protocol`` param) or a
# JSON file of real pages and argv[2] how to tamper with them; ``engine.initialize`` is protocol 3.
STUB = r"""
import json, sys
source, mode = sys.argv[1], sys.argv[2]
if mode == "transcript":
    records = []
    for line in open(source, encoding="utf-8").read().splitlines()[1:]:
        record = json.loads(line)
        try:
            asked = json.loads(record["request"])
        except ValueError:
            continue
        params = asked.get("params") if isinstance(asked, dict) else None
        if isinstance(asked, dict) and isinstance(params, (dict, type(None))):
            records.append((asked.get("method"), (params or {}).get("protocol"), json.loads(record["response"])))
elif mode not in ("speaks2", "claims3"):
    pages = json.load(open(source, encoding="utf-8"))
for line in sys.stdin:
    request = json.loads(line)
    method, params = request["method"], request.get("params") or {}
    if mode == "transcript":
        answer = next((r for m, v, r in records if m == method and v == params.get("protocol")), None)
        answer = dict(answer or {"jsonrpc": "2.0", "error": {"code": -32601, "message": "absent"}})
        if method == "diff.rank":
            answer = {"jsonrpc": "2.0", "error": {"code": -32000, "message": "diff.rank was asked"}}
    elif method == "engine.initialize":
        answer = {"jsonrpc": "2.0", "result": {"protocol": 2 if mode == "speaks2" else 3}}
    elif mode == "claims3":
        answer = {"jsonrpc": "2.0", "error": {"code": -32601, "message": "method not found: diff.diagnostics"}}
    else:
        cursor = params.get("cursor")
        index = 0 if cursor is None else next((i + 1 for i, p in enumerate(pages)
                                               if p["page"]["next_cursor"] == cursor), len(pages))
        if index >= len(pages):
            answer = {"jsonrpc": "2.0", "error": {"code": -32602, "message": "invalid cursor"}}
        else:
            page = pages[index]
            last = len(pages) - 1
            if mode == "total" and index > 0:
                page["page"]["total"] += 1
            if mode == "sha" and index == 1:
                page["page"]["document_sha256"] = "0" * 64
            if mode == "first" and index == 1:
                page["page"]["first"] -= 1
            if mode == "early" and index == 1:
                page["page"]["next_cursor"] = None
            if mode == "hash" and index == 1:
                page["diagnostics"][0]["line"] = 99999
            if mode == "repeat" and index == 2:
                page["page"]["next_cursor"] = pages[1]["page"]["next_cursor"]
            if mode == "endless" and index == last:
                page["page"]["next_cursor"] = "dg0.0000000000000000.0000000000000000.1"
            if mode == "past" and index == last:
                page["diagnostics"].append(page["diagnostics"][0])
                page["page"]["count"] += 1
            if mode == "empty" and index == 2:
                page["page"]["count"], page["diagnostics"] = 0, []
            answer = {"jsonrpc": "2.0", "result": page}
    answer["id"] = request["id"]
    print(json.dumps(answer), flush=True)
"""


def cli_review(patch, *mode):
    out = subprocess.run([BIN, "--stdin-patch", "--json", *mode], input=patch, capture_output=True, text=True,
                         check=True)
    return json.loads(out.stdout)


class Stdio(unittest.TestCase):
    def test_review_patch_matches_the_cli(self):
        with ddf.StdioClient((BIN, "rpc")) as client:
            self.assertEqual(client.review_patch(PATCH), cli_review(PATCH))
            self.assertIn("diff.compute", [m["name"] for m in client.methods()])

    def test_rank_patch_matches_the_cli(self):
        with ddf.StdioClient((BIN, "rpc")) as client:
            ranked = client.rank_patch(PATCH, require_files=True)
            self.assertEqual(json.loads(ranked.pop("canonical_json")), cli_review(PATCH, "--rank"))
            self.assertEqual(ranked, cli_review(PATCH, "--rank"))
            self.assertEqual(ranked["schema"], "deep-diff-forge.rank.v0")
            with self.assertRaises(ddf.RpcError) as refused:
                client.rank_patch("", require_files=True)
            self.assertEqual(refused.exception.code, 7)

    def test_verify_findings_matches_the_cli(self):
        findings = [
            {"agent": "codex", "severity": "high", "title": "t", "body": "b", "path": "x", "line": 1},
            {"agent": "codex", "severity": "low", "title": "u", "body": "c", "path": "nope"},
        ]
        with ddf.StdioClient((BIN, "rpc")) as client:
            verdict = client.verify_findings(PATCH, findings)
        self.assertEqual(verdict["schema"], "deep-diff-forge.findings.v0")
        self.assertEqual(verdict["counts"], {"accepted": 1, "refused": 1, "over_limit": 0})
        self.assertEqual(verdict["refused"][0]["reason"], "unknown_path")
        self.assertIs(verdict["claim_verified"], False)
        # The CLI over the same findings as the daemon's JSONL (one compact object per line, keys
        # sorted, UTF-8 unescaped) gives the same document, findings_sha256 included.
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / "f.jsonl"
            path.write_text("".join(json.dumps(f, separators=(",", ":"), sort_keys=True, ensure_ascii=False) + "\n"
                                    for f in findings), encoding="utf-8")
            out = subprocess.run([BIN, "findings", "verify", "--findings", str(path)], input=PATCH,
                                 capture_output=True, text=True, check=True)
        self.assertEqual(json.loads(out.stdout), verdict)

    def test_errors_keep_their_codes(self):
        with ddf.StdioClient((BIN, "rpc")) as client:
            with self.assertRaises(ddf.RpcError) as refused:
                client.review_patch("", require_files=True)
            self.assertEqual((refused.exception.code, refused.exception.message),
                             (7, "refused: 0 files in input (--require-files)"))
            with self.assertRaises(ddf.RpcError) as unknown:
                client.call("no.such")
            self.assertEqual(unknown.exception.code, -32601)
            # An invalid request is answered with id null (JSON-RPC 2.0 §5): still the daemon's error.
            with self.assertRaises(ddf.RpcError) as invalid:
                client.call("")
            self.assertEqual(invalid.exception.code, -32600)
            self.assertEqual(client.call("daemon.health")["status"], "ok")

    def test_a_request_larger_than_the_pipe_buffer_is_answered(self):
        # The request is written in chunks as the child takes them (well over the 64 KiB pipe).
        patch = "--- a/x\n+++ b/x\n@@ -1,1 +1,1 @@\n-old\n+" + "n" * (1 << 20) + "\n"
        with ddf.StdioClient((BIN, "rpc")) as client:
            self.assertEqual(client.review_patch(patch), cli_review(patch))

    def test_a_lone_surrogate_is_refused_locally_naming_its_finding(self):
        findings = [{"agent": "codex", "severity": "low", "title": "t", "body": "b", "path": "x"},
                    {"agent": "codex", "severity": "low", "title": "t\ud800", "body": "b", "path": "x"}]
        with ddf.StdioClient((BIN, "rpc")) as client:
            with self.assertRaisesRegex(ValueError, "^finding 1 holds a string that is not valid Unicode"):
                client.verify_findings(PATCH, findings)
            with self.assertRaisesRegex(ValueError, "not valid Unicode: surrogates not allowed at character"):
                client.review_patch("\udfff")
            # Nothing was sent: the child is alive and the next call is answered in step.
            self.assertEqual(client.verify_findings(PATCH, findings[:1])["counts"]["accepted"], 1)

    def test_an_oversize_request_is_refused_locally(self):
        with ddf.StdioClient((BIN, "rpc")) as client:
            with self.assertRaises(ValueError):
                client.review_patch("x" * ddf.MAX_REQUEST_BYTES)


class Diagnostics(unittest.TestCase):
    def test_diagnostics_patch_matches_the_cli(self):
        patch = paging_patch()
        expected = cli_diagnostics(patch)
        with ddf.StdioClient((BIN, "rpc")) as client:
            for max_bytes in (4096, None):
                document = client.diagnostics_patch(patch, max_bytes=max_bytes)
                self.assertIsInstance(document, ddf.Diagnostics)
                self.assertEqual(document.canonical_bytes, expected)
                self.assertEqual(document, json.loads(expected))
                self.assertEqual(list(document), ["schema", "input_sha256", "tool", "judge", "counts", "cut",
                                                  "diagnostics", "texts"])
            self.assertGreater(len(document["diagnostics"]), 40)
            with self.assertRaises(ddf.RpcError) as refused:
                client.diagnostics_patch("", require_files=True)
            self.assertEqual((refused.exception.code, refused.exception.message),
                             (7, "refused: 0 files in input (--require-files)"))

    def test_a_parse_failure_names_no_text_of_the_patch(self):
        with ddf.StdioClient((BIN, "rpc")) as client:
            for broken in (f"--- a/x\n+++ b/x\n@@ -1,{MARKER} +1 @@\n-a\n+b\n", f"--- a/x\n+++ b/x\n+{MARKER}\n"):
                with self.assertRaises(ddf.RpcError) as caught:
                    client.diagnostics_patch(broken)
                self.assertEqual(caught.exception.code, 4)
                self.assertNotIn("zq7MARKER", str(caught.exception))
                self.assertNotIn("\u202e", str(caught.exception))
                # The same failure on another method still quotes the line, as released.
                with self.assertRaises(ddf.RpcError) as released:
                    client.rank_patch(broken)
                self.assertIn("zq7MARKER", released.exception.message)

    def test_an_old_daemon_raises_protocol_too_old(self):
        stub = ddf.StdioClient((sys.executable, "-I", "-c", STUB, str(TRANSCRIPT), "transcript"), timeout=10)
        with stub:
            with self.assertRaises(ddf.ProtocolTooOld) as caught:
                stub.diagnostics_patch(PATCH)
        error = caught.exception
        self.assertEqual((error.daemon, error.needed, error.method), (2, 3, "diff.diagnostics"))
        self.assertEqual(str(error), "protocol too old: diff.diagnostics needs engine protocol 3; the daemon speaks 2")
        speaks2 = ddf.StdioClient((sys.executable, "-I", "-c", STUB, str(TRANSCRIPT), "speaks2"), timeout=10)
        with speaks2:
            with self.assertRaises(ddf.ProtocolTooOld) as answered_two:
                speaks2.diagnostics_patch(PATCH)
        self.assertEqual(answered_two.exception.daemon, 2)

    def test_a_daemon_that_breaks_paging_raises_a_named_error(self):
        patch = paging_patch()
        pages, cursor = [], None
        with ddf.StdioClient((BIN, "rpc")) as client:
            while True:
                params = {"patch": patch, "max_bytes": 4096}
                if cursor is not None:
                    params["cursor"] = cursor
                page = client.call("diff.diagnostics", **params)
                pages.append(page)
                cursor = page["page"]["next_cursor"]
                if cursor is None:
                    break
        self.assertGreaterEqual(len(pages), 4)
        last = len(pages) - 1
        cases = [("ok", None, None), ("total", 2, "total changed between pages"),
                 ("sha", 2, "document_sha256 changed between pages"),
                 ("first", 2, "a page does not start where the last one ended"),
                 ("early", 2, "the cursors ended before the document did"),
                 ("hash", len(pages), "the rebuilt document does not hash to document_sha256"),
                 ("repeat", 3, "a cursor came back"),
                 ("endless", last + 1, "the cursors did not end with the document"),
                 ("empty", 3, "a page holds no diagnostic"),
                 ("past", len(pages), "a page goes past total")]
        with tempfile.TemporaryDirectory() as directory:
            source = pathlib.Path(directory) / "pages.json"
            source.write_text(json.dumps(pages), encoding="utf-8")
            for mode, at, problem in cases:
                with ddf.StdioClient((sys.executable, "-I", "-c", STUB, str(source), mode), timeout=10) as stub:
                    if problem is None:
                        self.assertEqual(stub.diagnostics_patch(patch).canonical_bytes, cli_diagnostics(patch))
                        continue
                    with self.assertRaises(ddf.PagingError, msg=mode) as caught:
                        stub.diagnostics_patch(patch)
                self.assertEqual((caught.exception.pages, caught.exception.problem), (at, problem), mode)
                self.assertIsInstance(caught.exception, ddf.TransportError)


    def test_a_member_this_sdk_does_not_know_is_named(self):
        patch = paging_patch()
        pages, cursor = [], None
        with ddf.StdioClient((BIN, "rpc")) as client:
            while True:
                params = {"patch": patch, "max_bytes": 4096}
                if cursor is not None:
                    params["cursor"] = cursor
                page = client.call("diff.diagnostics", **params)
                pages.append(page)
                cursor = page["page"]["next_cursor"]
                if cursor is None:
                    break
        command = cli_diagnostics(patch).decode()
        top = command.replace('\n  "cut": ', '\n  "future": 1,\n  "cut": ', 1)
        first = command.index('\n    {"key"') + 1
        close = command.rindex("}", 0, command.index("\n", first))
        inner = command[:close] + ', "x\\u202e": true' + command[close:]
        for document, name, tamper in ((top, "future", lambda k, p: p.__setitem__("future", 1)),
                                       (inner, "diagnostics[].x\\u202e",
                                        lambda k, p: k == 0 and p["diagnostics"][0].__setitem__("x\u202e", True))):
            digest = hashlib.sha256(document.encode()).hexdigest()
            newer = json.loads(json.dumps(pages))
            for k, page in enumerate(newer):
                page["page"]["document_sha256"] = digest
                tamper(k, page)
            with tempfile.TemporaryDirectory() as directory:
                source = pathlib.Path(directory) / "pages.json"
                source.write_text(json.dumps(newer), encoding="utf-8")
                with ddf.StdioClient((sys.executable, "-I", "-c", STUB, str(source), "ok"), timeout=10) as stub:
                    with self.assertRaises(ddf.UnknownMembers) as caught:
                        stub.diagnostics_patch(patch)
            error = caught.exception
            self.assertEqual((error.pages, error.members), (len(pages), [name]))
            self.assertIn("upgrade the SDK", str(error))
            self.assertNotIn("\u202e", str(error))
            self.assertIsInstance(error, ddf.PagingError)

    def test_a_daemon_that_claims_protocol_3_without_the_method_is_named(self):
        error = ddf.ProtocolTooOld(3, 3, "diff.diagnostics")
        self.assertEqual(str(error), "protocol too old: the daemon reported engine protocol 3 and does not know "
                                     "diff.diagnostics")
        claims = ddf.StdioClient((sys.executable, "-I", "-c", STUB, str(TRANSCRIPT), "claims3"), timeout=10)
        with claims:
            with self.assertRaises(ddf.ProtocolTooOld) as caught:
                claims.diagnostics_patch(PATCH)
        self.assertEqual((caught.exception.daemon, str(caught.exception)), (3, str(error)))


class Responses(unittest.TestCase):
    """The answer is checked before it is believed, errors included (council sdc1)."""

    def test_valid_answers(self):
        self.assertEqual(ddf._result({"jsonrpc": "2.0", "id": 1, "result": {"a": 1}}, 1), {"a": 1})
        with self.assertRaises(ddf.RpcError) as error:
            ddf._result({"jsonrpc": "2.0", "id": 1, "error": {"code": 7, "message": "m"}}, 1)
        self.assertEqual(error.exception.code, 7)
        with self.assertRaises(ddf.RpcError) as invalid:
            ddf._result({"jsonrpc": "2.0", "id": None, "error": {"code": -32600, "message": "m"}}, 1)
        self.assertEqual(invalid.exception.code, -32600)

    def test_invalid_answers_are_transport_errors(self):
        for bad in [
            {"jsonrpc": "2.0", "id": None, "result": {}},
            {"jsonrpc": "2.0", "error": {"code": -32600, "message": "no id at all"}},
            {"jsonrpc": "2.0", "id": 999, "error": {"code": 4, "message": "another call"}},
            {"jsonrpc": "2.0", "id": 2, "result": {}},
            {"jsonrpc": "2.0", "id": True, "result": {}},
            {"jsonrpc": "1.0", "id": 1, "result": {}},
            {"id": 1, "result": {}},
            {"jsonrpc": "2.0", "id": 1},
            {"jsonrpc": "2.0", "id": 1, "result": 1, "error": {"code": 1, "message": "m"}},
            {"jsonrpc": "2.0", "id": 1, "error": {"code": "4", "message": "m"}},
            {"jsonrpc": "2.0", "id": 1, "error": {"code": 4}},
            [],
        ]:
            with self.subTest(bad=bad), self.assertRaises(ddf.TransportError):
                ddf._result(bad, 1)


class Bounds(unittest.TestCase):
    """A child that never answers, or answers without end, cannot hang or exhaust the caller."""

    def test_a_silent_child_times_out_and_is_killed(self):
        client = ddf.StdioClient((sys.executable, "-c", "import time; time.sleep(60)"), timeout=0.5)
        started = time.monotonic()
        with self.assertRaises(ddf.TransportError):
            client.call("daemon.health")
        self.assertLess(time.monotonic() - started, 10)
        self.assertIsNotNone(client._process.poll())

    def test_a_child_that_stops_reading_cannot_block_a_large_request(self):
        # Before, the write blocked once the pipe buffer filled, before any deadline was armed.
        client = ddf.StdioClient((sys.executable, "-c", "import time; time.sleep(60)"), timeout=0.5)
        started = time.monotonic()
        with self.assertRaisesRegex(ddf.TransportError, "no answer within 0.5 s"):
            client.review_patch("x" * (1 << 20))
        self.assertLess(time.monotonic() - started, 10)
        self.assertIsNotNone(client._process.poll())

    def test_an_answer_before_the_request_is_taken_is_still_read(self):
        # A child that answers and exits without reading: the broken pipe ends the write, and its
        # answer (here a refusal) is the call's, not "not reading".
        answer = '{"jsonrpc":"2.0","id":1,"error":{"code":-32600,"message":"too long"}}'
        client = ddf.StdioClient((sys.executable, "-c", f"print({answer!r}, flush=True)"), timeout=10)
        with self.assertRaises(ddf.RpcError) as refused:
            client.review_patch("x" * (1 << 20))
        self.assertEqual(refused.exception.code, -32600)
        client.close()

    def test_an_early_answer_from_a_child_that_stops_reading_but_stays_alive_is_read(self):
        # Before, the loop kept waiting to write the rest until the deadline, though the whole
        # answer was already in hand. The unsent tail would garble the next request, so the
        # out-of-step child is killed once its answer is taken.
        answer = '{"jsonrpc":"2.0","id":1,"error":{"code":-32600,"message":"too long"}}'
        code = f"import time\nprint({answer!r}, flush=True)\ntime.sleep(60)"
        client = ddf.StdioClient((sys.executable, "-c", code), timeout=10)
        started = time.monotonic()
        with self.assertRaises(ddf.RpcError) as refused:
            client.review_patch("x" * (1 << 20))
        self.assertEqual(refused.exception.code, -32600)
        self.assertLess(time.monotonic() - started, 5)
        self.assertIsNotNone(client._process.poll(), "the out-of-step child is killed")
        with self.assertRaises(ddf.TransportError):
            client.call("daemon.health")

    def test_an_answer_that_is_not_a_valid_response_is_a_transport_error_and_kills_the_child(self):
        # Each answer is the child's source expression, so a long one need not fit in its argv.
        for answer, reason in [("b'notjson'", "not a valid response: Expecting value"),
                               ("b'\\xff'", "not a valid response: 'utf-8' codec can't decode"),
                               ("""b'{"jsonrpc":"2.0","id":9,"result":{}}'""", "response id 9 does not match 1"),
                               # Before, json.loads raised a bare RecursionError and the child lived on.
                               ("b'[' * 200_000", "not a valid response: ")]:
            with self.subTest(answer=answer):
                code = ("import sys, time\nsys.stdin.readline()\n"
                        f"sys.stdout.buffer.write({answer} + b'\\n'); sys.stdout.flush(); time.sleep(60)")
                client = ddf.StdioClient((sys.executable, "-c", code), timeout=10)
                with self.assertRaisesRegex(ddf.TransportError, reason):
                    client.call("daemon.health")
                self.assertIsNotNone(client._process.poll(), "the child is killed")

    def test_a_runaway_answer_is_cut_off(self):
        original = ddf.MAX_REQUEST_BYTES
        ddf.MAX_REQUEST_BYTES = 1 << 16
        try:
            client = ddf.StdioClient((sys.executable, "-c", "import sys\nwhile True: sys.stdout.write('x' * 65536); sys.stdout.flush()"), timeout=10)
            with self.assertRaises(ddf.TransportError):
                client.call("daemon.health")
            self.assertIsNotNone(client._process.poll())
        finally:
            ddf.MAX_REQUEST_BYTES = original


    def test_the_cap_is_the_one_line_cap_of_the_daemon_reference(self):
        reference = " ".join((REPO / "docs" / "reference" / "mcp-sdk-daemon.md").read_text(encoding="utf-8").split())
        self.assertIn(f"at most 80 MiB ({ddf.MAX_REQUEST_BYTES:,} bytes, `MAX_REQUEST_BYTES`) counting its newline", reference)

    def test_a_child_that_answers_and_breaks_the_pipe_is_killed(self):
        # G-SDK-1.1. The child closes its stdin, so the write breaks, then answers and stays alive.
        # Before, the broken pipe emptied the unsent request, so the out-of-step child lived on.
        code = ("import os, sys, time\nos.close(0)\ntime.sleep(0.5)\n"
                f"print({REFUSAL!r}, flush=True)\ntime.sleep(60)")
        client = ddf.StdioClient((sys.executable, "-c", code), timeout=10)
        try:
            with self.assertRaises(ddf.RpcError) as refused:
                client.review_patch("x" * (1 << 20))
            self.assertEqual(refused.exception.code, -32600)
            self.assertIsNotNone(client._process.poll(), "the out-of-step child is killed")
            with self.assertRaisesRegex(ddf.TransportError, "has ended: it answered before taking"):
                client.call("daemon.health")
        finally:
            _reap(client)

    def test_a_call_after_close_is_a_transport_error(self):
        # G-SDK-1.3. Before, the closed stdin raised a bare ValueError.
        client = ddf.StdioClient((sys.executable, "-c", ECHO), timeout=10)
        try:
            self.assertEqual(client.call("daemon.health"), {})
            client.close()
            self.assertTrue(client._process.stdout.closed, "both pipes are closed")
            with self.assertRaisesRegex(ddf.TransportError, "has ended: the client was closed"):
                client.call("daemon.health")
            client.close()
        finally:
            _reap(client)

    def test_a_failed_read_of_the_answer_is_a_transport_error(self):
        # G-SDK-1.2. A read that would block is waited out; any other read error is a transport
        # failure that kills the child. Before, both escaped as bare OSErrors.
        for raised, expected in [(BlockingIOError(11, "spurious"), None),
                                 (OSError(5, "Input/output error"), "reading the answer .* failed: .*Input/output")]:
            with self.subTest(raised=type(raised).__name__):
                client = ddf.StdioClient((sys.executable, "-c", ECHO), timeout=10)
                real, armed = ddf.os.read, [True]

                def flaky(fd, size, _raised=raised, _client=client):
                    if armed[0] and fd == _client._process.stdout.fileno():
                        armed[0] = False
                        raise _raised
                    return real(fd, size)

                ddf.os.read = flaky
                try:
                    if expected is None:
                        self.assertEqual(client.call("daemon.health"), {})
                    else:
                        with self.assertRaisesRegex(ddf.TransportError, expected):
                            client.call("daemon.health")
                        self.assertIsNotNone(client._process.poll(), "the child is killed")
                finally:
                    ddf.os.read = real
                    _reap(client)

    def test_a_large_answer_is_read_in_linear_time(self):
        # A 64 MiB answer in 64 KiB writes. Before, each chunk copied and rescanned the whole buffer:
        # 14 s on the machine that measured it, against 0.3 s now.
        code = ("import sys, time\nsys.stdin.readline()\nw = sys.stdout.buffer\n"
                "w.write(b'{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":\"'); w.flush()\n"
                "for _ in range(1024): w.write(b'a' * 65536); w.flush()\n"
                "w.write(b'\"}\\n'); w.flush()\ntime.sleep(60)")
        client = ddf.StdioClient((sys.executable, "-c", code), timeout=120)
        try:
            started = time.monotonic()
            self.assertEqual(len(client.call("daemon.health")), 64 << 20)
            self.assertLess(time.monotonic() - started, 5, "a 64 MiB answer is read in linear time")
        finally:
            _reap(client)

    def test_an_answer_line_at_the_cap_is_read_and_one_byte_over_is_refused(self):
        # One cap rule: an answer line is at most MAX_REQUEST_BYTES bytes, its newline counted. The
        # lowered cap is not a multiple of a 64 KiB read, so the newline arrives past it in one read.
        for extra in (0, 1):
            with self.subTest(over=extra), _lowered_cap(100_000):
                code = f"import sys, time\nsys.stdin.readline()\nsys.stdout.buffer.write({_capped_line(extra)!r}); sys.stdout.flush(); time.sleep(60)"
                client = ddf.StdioClient((sys.executable, "-c", code), timeout=10)
                try:
                    if extra:
                        with self.assertRaisesRegex(ddf.TransportError, "the answer exceeds the daemon's maximum size"):
                            client.call("daemon.health")
                    else:
                        self.assertEqual(client.call("daemon.health"), {})
                finally:
                    _reap(client)


class Socket(unittest.TestCase):
    def test_an_unreachable_daemon_is_a_transport_error(self):
        with tempfile.TemporaryDirectory() as directory:
            missing = pathlib.Path(directory) / "absent.sock"
            with self.assertRaises(ddf.TransportError) as caught:
                ddf.SocketClient(missing, timeout=1).call("daemon.health")
            self.assertIsInstance(caught.exception.__cause__, OSError)
            not_a_socket = pathlib.Path(directory) / "file"
            not_a_socket.write_text("x")
            with self.assertRaises(ddf.TransportError):
                ddf.SocketClient(not_a_socket, timeout=1).call("daemon.health")

    def test_a_live_daemon_answers_like_the_cli(self):
        with tempfile.TemporaryDirectory() as directory:
            os.chmod(directory, 0o700)
            path = pathlib.Path(directory) / "d.sock"
            daemon = subprocess.Popen([BIN, "daemon", "start", "--foreground", "--socket", str(path)],
                                      stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            try:
                client = ddf.SocketClient(path, timeout=10)
                deadline = time.monotonic() + 10
                while True:
                    try:
                        client.call("daemon.health")
                        break
                    except ddf.TransportError:
                        if time.monotonic() > deadline:
                            raise
                        time.sleep(0.05)
                self.assertEqual(client.review_patch(PATCH), cli_review(PATCH))
                ranked = client.rank_patch(PATCH)
                self.assertEqual(json.loads(ranked.pop("canonical_json")), cli_review(PATCH, "--rank"))
                self.assertEqual(ranked, cli_review(PATCH, "--rank"))
                with self.assertRaises(ddf.RpcError) as failed:
                    client.review_patch("--- a/x\n+++ b/x\n+stray\n")
                self.assertEqual(failed.exception.code, 4)
                patch = paging_patch()
                self.assertEqual(client.diagnostics_patch(patch, max_bytes=4096).canonical_bytes,
                                 cli_diagnostics(patch))
                client.call("daemon.shutdown")
                self.assertEqual(daemon.wait(timeout=10), 0)
            finally:
                if daemon.poll() is None:
                    daemon.kill()

    def test_an_answer_that_is_not_utf8_json_is_a_transport_error(self):
        for answer, reason in [(b"\xff\n", "'utf-8' codec can't decode"), (b"notjson\n", "Expecting value"),
                               (b"[" * 200_000 + b"\n", "")]:
            with self.subTest(answer=answer[:16]), tempfile.TemporaryDirectory() as directory:
                path = pathlib.Path(directory) / "d.sock"
                with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as listener:
                    listener.bind(str(path))
                    listener.listen(1)

                    def serve():
                        connection, _ = listener.accept()
                        with connection:
                            connection.recv(1 << 16)
                            connection.sendall(answer)

                    server = threading.Thread(target=serve)
                    server.start()
                    try:
                        with self.assertRaisesRegex(ddf.TransportError, f"not a valid response: {reason}"):
                            ddf.SocketClient(path, timeout=5).call("daemon.health")
                    finally:
                        server.join(timeout=10)

    def test_a_daemon_of_another_user_is_refused_before_anything_is_sent(self):
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / "d.sock"
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as listener:
                listener.bind(str(path))
                listener.listen(2)
                me = os.geteuid()
                with _connected(path) as probe:
                    self.assertEqual(ddf._peer_uid(probe), me)
                listener.accept()[0].close()
                original = ddf.os.geteuid
                ddf.os.geteuid = lambda: me + 1
                try:
                    with self.assertRaises(ddf.TransportError) as caught:
                        ddf.SocketClient(path, timeout=1).call("daemon.health")
                finally:
                    ddf.os.geteuid = original
                self.assertIn(f"runs as uid {me}, not uid {me + 1}", str(caught.exception))
                accepted, _ = listener.accept()
                with accepted:
                    accepted.settimeout(1)
                    self.assertEqual(accepted.recv(1), b"", "nothing was sent to that daemon")


    def test_an_early_refusal_over_a_socket_is_read_not_lost(self):
        # G-SDK-1.4. The daemon takes 16 bytes of a 4 MiB request, refuses it and closes with the
        # rest unread, so the send breaks (a broken pipe or a reset). Before, the refusal was lost
        # and the call raised "could not be reached".
        refusal = b'{"jsonrpc":"2.0","id":null,"error":{"code":-32000,"message":"read budget full"}}\n'

        def refuse(connection):
            connection.recv(16)
            connection.sendall(refusal)

        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / "d.sock"
            with _serving(path, refuse):
                with self.assertRaises(ddf.RpcError) as refused:
                    ddf.SocketClient(path, timeout=10).review_patch("y" * (4 << 20))
            self.assertEqual(refused.exception.code, -32000)

    def test_the_socket_deadline_is_overall(self):
        # A daemon that sends one byte every 0.4 s and never a newline. Before, each read had the
        # whole timeout again, so the call lasted until the daemon stopped (4.8 s for 12 bytes).
        def trickle(connection):
            connection.recv(1 << 16)
            for _ in range(12):
                try:
                    connection.sendall(b" ")
                except OSError:
                    return
                time.sleep(0.4)

        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / "d.sock"
            with _serving(path, trickle):
                started = time.monotonic()
                with self.assertRaisesRegex(ddf.TransportError, "no answer from the daemon at .* within 1 s"):
                    ddf.SocketClient(path, timeout=1).call("daemon.health")
                self.assertLess(time.monotonic() - started, 3, "the deadline covers the whole call")

    @unittest.skipUnless(hasattr(socket, "SO_PEERCRED"), "SO_PEERCRED is Linux's")
    def test_peer_uid_reads_an_unsigned_uid(self):
        # G-SDK-1.5. uid_t is unsigned: a uid of 2^31 + 5 was read as -2147483643 with "3i".
        class Credentials:
            def getsockopt(self, level, option, size):
                return struct.pack("3I", 1234, 2**31 + 5, 0)[:size]

        self.assertEqual(ddf._peer_uid(Credentials()), 2**31 + 5)

    def test_an_answer_line_at_the_cap_is_read_and_one_byte_over_is_refused(self):
        # One cap rule: an answer line is at most MAX_REQUEST_BYTES bytes, its newline counted.
        # Before, the socket client read up to one byte more. The lowered cap is not a multiple of
        # a 64 KiB read, so the newline arrives past it in one read.
        for extra in (0, 1):
            with self.subTest(over=extra), _lowered_cap(100_000), tempfile.TemporaryDirectory() as directory:
                path = pathlib.Path(directory) / "d.sock"
                line = _capped_line(extra)

                def answer(connection, _line=line):
                    connection.recv(1 << 16)
                    connection.sendall(_line)

                with _serving(path, answer):
                    client = ddf.SocketClient(path, timeout=10)
                    if extra:
                        with self.assertRaisesRegex(ddf.TransportError, "the answer exceeds the daemon's maximum size"):
                            client.call("daemon.health")
                    else:
                        self.assertEqual(client.call("daemon.health"), {})


def _reap(client):
    """Kill a test's child by its pid if it still runs, so no test leaves a process behind."""
    if client._process.poll() is None:
        os.kill(client._process.pid, 9)
        client._process.wait()


class _lowered_cap:
    """MAX_REQUEST_BYTES lowered to ``cap`` while the block runs."""

    def __init__(self, cap):
        self.cap = cap

    def __enter__(self):
        self.original, ddf.MAX_REQUEST_BYTES = ddf.MAX_REQUEST_BYTES, self.cap

    def __exit__(self, *_):
        ddf.MAX_REQUEST_BYTES = self.original


def _capped_line(extra):
    """A valid answer to request 1 of exactly the (lowered) cap plus ``extra`` bytes, newline counted."""
    body = b'{"jsonrpc":"2.0","id":1,"result":{}'
    return body + b" " * (ddf.MAX_REQUEST_BYTES - len(body) - 2 + extra) + b"}\n"


class _serving:
    """A stand-in daemon on a Unix socket at ``path``: a thread that serves one connection with
    ``handler`` and is joined when the block ends."""

    def __init__(self, path, handler):
        self.listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.listener.bind(str(path))
        self.listener.listen(1)
        self.handler = handler
        self.thread = threading.Thread(target=self._serve, daemon=True)

    def _serve(self):
        with self.listener:
            connection, _ = self.listener.accept()
            with connection:
                self.handler(connection)

    def __enter__(self):
        self.thread.start()

    def __exit__(self, *_):
        self.thread.join(timeout=10)


def _connected(path):
    connection = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    connection.connect(str(path))
    return connection


if __name__ == "__main__":
    unittest.main(verbosity=1)
