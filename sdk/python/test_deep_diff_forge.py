"""Tests for the Python client against the real binary: DDF_BIN names it (default: target/debug).

Run: DDF_BIN=target/debug/deep-diff-forge python3 -I sdk/python/test_deep_diff_forge.py
"""
import json
import os
import pathlib
import socket
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
                self.assertEqual(client.rank_patch(PATCH), cli_review(PATCH, "--rank"))
                with self.assertRaises(ddf.RpcError) as failed:
                    client.review_patch("--- a/x\n+++ b/x\n+stray\n")
                self.assertEqual(failed.exception.code, 4)
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


def _connected(path):
    connection = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    connection.connect(str(path))
    return connection


if __name__ == "__main__":
    unittest.main(verbosity=1)
