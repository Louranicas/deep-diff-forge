#!/usr/bin/env bash
# The protocol-2 transcript (F7.d1): every request of fixtures/protocol/v2-requests.txt answered
# by ONE `rpc` process of the published v0.10.0 binary, the binary's SHA-256 in the first line.
# crates/deep-diff-forge-cli/tests/protocol_compat.rs replays it under the replay rule of
# docs/reference/mcp-sdk-daemon.md. A request whose patch raises a label a 0.11.0 register row
# widens or changes is refused: on the label list below always, and on TREE's own ranking too
# when TREE (a build of the tree) is given, which also catches a widening the list does not name.
# `--check BINARY` records twice and fails unless the two recordings differ only at the
# run-dependent values RUN_DEPENDENT names (the replay test's own list, rulings R-F2/F3).
set -euo pipefail
PUBLISHED=121503a8d292da951c31e33e8b59bf33b54dd316e3d6ec3055e5c4bfac18e9ff
usage="usage: $0 BINARY OUT [TREE] | $0 --check BINARY  (BINARY: the published v0.10.0 binary, sha256 $PUBLISHED)"
CHECK=
if [ "${1:-}" = --check ]; then CHECK=1; set -- "${2:-}" "${TMPDIR:-/tmp}/ddf-transcript-check.$$"; fi
[ $# -ge 2 ] && [ -n "$1" ] || { echo "$usage" >&2; exit 2; }
BINARY=$1 OUT=$2 TREE=${3:-}
REQUESTS="$(dirname "$0")/../fixtures/protocol/v2-requests.txt"
[ "$(sha256sum < "$BINARY" | cut -c1-64)" = "$PUBLISHED" ] || { echo "error: $BINARY is not the published v0.10.0 binary" >&2; exit 2; }
trap 'rm -f "$OUT.responses" "$OUT.second"' EXIT
"$BINARY" rpc < "$REQUESTS" > "$OUT.responses"
[ -z "$CHECK" ] || "$BINARY" rpc < "$REQUESTS" > "$OUT.second"
python3 -P -E - "$BINARY" "$TREE" "$REQUESTS" "$OUT.responses" "$OUT" "$PUBLISHED" "$CHECK" <<'EOF'
import json, os, subprocess, sys
binary, tree, requests, responses, out, sha, check = sys.argv[1:]
WIDENED = {"check_suppressed", "trivial_assert", "error_swallowed", "panic_path_added", "test_skipped", "build_hook", "unsafe_added"}
lines = [l.rstrip("\n") for l in open(requests, encoding="utf-8")]
answers = [l.rstrip("\n") for l in open(responses, encoding="utf-8")]
assert len(lines) == len(answers), f"{len(lines)} requests, {len(answers)} answers"
RUN_DEPENDENT = {"daemon.health": [["pid"]], "daemon.status": [["uptime_secs"]], "daemon.metrics": [["admission", "wait_us"]]
                 + [["methods", "*", "latency_us", m] for m in ("buckets", "p50_at_most", "p95_at_most", "p99_at_most")]}
if check:
    def method(line):
        try: return json.loads(line).get("method")
        except (ValueError, AttributeError): return None
    def diff(a, b, path, out):
        if isinstance(a, dict) and isinstance(b, dict) and a.keys() == b.keys():
            for k in a: diff(a[k], b[k], path + [k], out)
        elif isinstance(a, list) and isinstance(b, list) and len(a) == len(b):
            for x, y in zip(a, b): diff(x, y, path, out)
        elif a != b or type(a) is not type(b): out.append(path)
    def listed(m, path):
        return any(len(p) == len(path) and all(x in ("*", y) for x, y in zip(p, path)) for p in RUN_DEPENDENT.get(m, []))
    second = [l.rstrip("\n") for l in open(responses[: -len(".responses")] + ".second", encoding="utf-8")]
    found, unlisted = [], []
    for n, (q, a, b) in enumerate(zip(lines, answers, second), 2):
        paths = []
        diff(json.loads(a), json.loads(b), [], paths)
        paths = [p[1:] if p[:1] == ["result"] else ["<answer>"] + p for p in paths]
        found += [(n, method(q), ".".join(p)) for p in paths]
        unlisted += [(n, ".".join(p)) for p in paths if not listed(method(q), p)]
    print(f"check: {len(found)} values differ between two recordings, on transcript lines {sorted({(n, m) for n, m, _ in found})}; outside the run-dependent list: {unlisted[:20]}")
    sys.exit(1 if unlisted or len(second) != len(answers) else 0)
for line in lines:
    try: patch = json.loads(line)["params"]["patch"]
    except (ValueError, KeyError, TypeError): continue
    if not isinstance(patch, str): continue
    for b in filter(None, (binary, tree)):
        run = subprocess.run([b, "--stdin-patch", "--rank", "--json", "--allow-empty"], input=patch.encode(), capture_output=True)
        ranked = json.loads(run.stdout)["ranked"] if run.returncode == 0 else []
        hit = WIDENED & {s for f in ranked for s in f["signals"]}
        if hit: sys.exit(f"error: a request's patch raises {sorted(hit)} on {b}: {line[:120]}")
head = {"transcript": "deep-diff-forge engine protocol 2", "binary": "the published v0.10.0 binary", "binary_sha256": sha, "recorder": "scripts/record_protocol_transcript.sh", "requests": len(lines)}
with open(out + ".tmp", "w", encoding="utf-8", newline="\n") as f:
    f.write(json.dumps(head) + "\n")
    for q, a in zip(lines, answers): f.write(json.dumps({"request": q, "response": a}, ensure_ascii=False) + "\n")
os.replace(out + ".tmp", out)
print(f"recorded {len(lines)} requests into {out}")
EOF
