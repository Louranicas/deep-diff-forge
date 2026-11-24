"""Pages of the diagnostics document, for scripts/contract_matrix.sh (F4.d).

Run as `pages.py rpc|mcp <deep-diff-forge> <patch file> <max_bytes>`. It asks the binary's
`rpc` server (`diff.diagnostics`) or its MCP server (`diagnose_patch`) for every page of the
patch's `deep-diff-forge.diagnostics.v0` document, following `page.next_cursor`, and puts the
document together again as values: the first page's header without `page`, every page's `cut`
entries and diagnostics in order, and the union of `texts`. It prints one line and exits 0 when
that equals the document `--stdin-patch --diagnostics --json` prints, every page names the
SHA-256 of those bytes as `document_sha256`, the pages hold `total` diagnostics, and there is
more than one page; else it says what differs and exits 1.
"""

import hashlib
import json
import subprocess
import sys

HEADER = ("schema", "input_sha256", "tool", "judge", "counts")


def rpc_pages(ddf, patch, max_bytes):
    server = subprocess.Popen([ddf, "rpc"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
    try:
        cursor, n = None, 0
        while True:
            params = {"patch": patch, "max_bytes": max_bytes}
            if cursor:
                params["cursor"] = cursor
            n += 1
            server.stdin.write(json.dumps({"jsonrpc": "2.0", "id": n, "method": "diff.diagnostics",
                                           "params": params}) + "\n")
            server.stdin.flush()
            answer = json.loads(server.stdout.readline())
            if "error" in answer:
                raise SystemExit(f"rpc error: {answer['error']}")
            yield answer["result"]
            cursor = answer["result"]["page"]["next_cursor"]
            if not cursor or n > 10000:
                return
    finally:
        server.stdin.close()
        server.wait(timeout=30)


def mcp_pages(ddf, patch, max_bytes):
    server = subprocess.Popen([ddf, "mcp"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)

    def ask(n, method, params):
        server.stdin.write(json.dumps({"jsonrpc": "2.0", "id": n, "method": method, "params": params}) + "\n")
        server.stdin.flush()
        while True:
            answer = json.loads(server.stdout.readline())
            if answer.get("id") == n:
                return answer

    try:
        ask(0, "initialize", {"protocolVersion": "2025-06-18", "capabilities": {},
                              "clientInfo": {"name": "contract-matrix", "version": "0"}})
        server.stdin.write(json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}) + "\n")
        cursor, n = None, 0
        while True:
            arguments = {"patch": patch, "max_bytes": max_bytes}
            if cursor:
                arguments["cursor"] = cursor
            n += 1
            answer = ask(n, "tools/call", {"name": "diagnose_patch", "arguments": arguments})
            result = answer.get("result") or {}
            if "error" in answer or result.get("isError"):
                raise SystemExit(f"mcp error: {json.dumps(answer)[:300]}")
            page = result["structuredContent"]
            yield page
            cursor = page["page"]["next_cursor"]
            if not cursor or n > 10000:
                return
    finally:
        server.stdin.close()
        server.wait(timeout=30)


def main():
    surface, ddf, patch_file, max_bytes = sys.argv[1], sys.argv[2], sys.argv[3], int(sys.argv[4])
    patch = open(patch_file, encoding="utf-8").read()
    whole = subprocess.run([ddf, "--stdin-patch", "--diagnostics", "--json"], input=patch.encode(),
                           capture_output=True, check=True).stdout
    document, digest = json.loads(whole), hashlib.sha256(whole).hexdigest()
    pages = list((rpc_pages if surface == "rpc" else mcp_pages)(ddf, patch, max_bytes))
    built = {k: pages[0][k] for k in HEADER}
    built["cut"] = [c for p in pages for c in p["cut"]]
    built["diagnostics"] = [g for p in pages for g in p["diagnostics"]]
    texts = {}
    for p in pages:
        texts.update(p["texts"])
    built["texts"] = texts
    wrong = [k for k in document if document[k] != built.get(k)]
    wrong += [f"page {i} names document_sha256 {p['page']['document_sha256']}"
              for i, p in enumerate(pages) if p["page"]["document_sha256"] != digest]
    total = pages[0]["page"]["total"]
    if len(built["diagnostics"]) != total:
        wrong.append(f"{len(built['diagnostics'])} diagnostics, total {total}")
    if len(pages) < 2:
        wrong.append("one page only")
    if wrong:
        print(f"{surface}: {len(pages)} pages; differs: {', '.join(wrong)}")
        return 1
    print(f"{surface}: {len(pages)} pages of {total} diagnostics at max_bytes {max_bytes}, the document again")
    return 0


if __name__ == "__main__":
    sys.exit(main())
