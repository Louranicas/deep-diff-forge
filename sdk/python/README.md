# deep-diff-forge (Python client)

A zero-dependency, standard-library Python client (Python ≥ 3.9, version
`0.11.0`) for [Deep-Diff-Forge](https://github.com/Louranicas/deep-diff-forge)'s
daemon protocol (**JSON-RPC 2.0, engine protocol 3**, one message per line). It
drives the `deep-diff-forge` binary, which it does not ship: install a release
binary or build it with Cargo first.

## Installation

Not on PyPI; install from a local checkout or directly from Git:

```bash
python3 -m pip install ./sdk/python
# or from Git:
python3 -m pip install "deep-diff-forge @ git+https://github.com/Louranicas/deep-diff-forge@v0.11.0#subdirectory=sdk/python"
```

## Transports

Two clients expose the exact same methods over the same JSON-RPC 2.0 protocol:

- **`StdioClient(command=("deep-diff-forge", "rpc"), timeout=30.0)`** — spawns
  `deep-diff-forge rpc` over stdin/stdout pipes; no daemon needed. Use as a
  context manager (`with StdioClient() as client:`) or call `close()`.
- **`SocketClient(path, timeout=30.0)`** — connects to a running
  `deep-diff-forge daemon` Unix domain socket (one connection per call). Before
  sending any bytes, verifies the daemon's effective UID matches the caller
  (`SO_PEERCRED` on Linux, `LOCAL_PEERCRED` on macOS).

## Client Methods

Both `StdioClient` and `SocketClient` provide:

| Method | Daemon RPC | Returns |
| --- | --- | --- |
| `review_patch(patch, require_files=False, require_hunks=False)` | `diff.compute` | `deep-diff-forge.review.v0` dict |
| `rank_patch(patch, require_files=False, require_hunks=False)` | `diff.rank` | `deep-diff-forge.rank.v0` dict, including `canonical_json` (the exact bytes `--rank --json` prints, newline included) |
| `diagnostics_patch(patch, require_files=False, require_hunks=False, max_bytes=None)` | `diff.diagnostics` (protocol 3) | `Diagnostics` (`dict` subclass) whose `canonical_bytes` are the exact bytes `--diagnostics --json` prints, verified against `page.document_sha256` |
| `verify_findings(patch, findings, require_files=False, require_hunks=False)` | `findings.verify` (protocol 2+) | `deep-diff-forge.findings.v0` dict (`claim_verified: false`); validates findings UTF-8 before sending (lone surrogates raise `ValueError` naming the finding index) |
| `methods()` | `engine.methods` | list of method descriptors (`name`, `params`, `result`, `since`) |
| `call(method, **params)` | any of the 17 daemon methods | raw JSON-RPC `result` (use for `review.outline`, `review.hunks`, `review.change`, `daemon.health`, `daemon.status`, `daemon.metrics`, `session.open`, `session.snapshot`, `session.close`, etc.) |

## Usage Examples

### Review, rank, diagnose and verify findings via `StdioClient`

```python
from deep_diff_forge import StdioClient, ProtocolTooOld, RpcError

patch = "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@\n-fn a() {}\n+unsafe fn a() {}\n"

with StdioClient() as client:
    review = client.review_patch(patch, require_files=True, require_hunks=True)
    ranking = client.rank_patch(patch, require_files=True, require_hunks=True)
    # Exact canonical bytes matching `deep-diff-forge --stdin-patch --rank --json`:
    rank_bytes = ranking["canonical_json"].encode("utf-8")

    # Protocol 3 diagnostics (paged automatically and verified against document_sha256):
    try:
        diag = client.diagnostics_patch(patch, require_files=True, require_hunks=True)
        print(diag["counts"], len(diag.canonical_bytes))
    except ProtocolTooOld as err:
        print(f"Daemon speaks protocol {err.daemon}, need {err.needed} for {err.method}")

    # Verify structured agent findings against the sealed review:
    verdict = client.verify_findings(
        patch,
        [{
            "agent": "reviewer",
            "severity": "high",
            "title": "New unsafe fn",
            "body": "Public API became unsafe.",
            "path": "src/lib.rs",
            "line": 1,
            "side": "new",
        }],
    )
    print(verdict["counts"])
```

### Budgeted views (`review.outline`, `review.hunks`, `review.change`) via `call()`

```python
from deep_diff_forge import StdioClient

# Pass --allow-repo to enable review.change on that repository root:
with StdioClient(command=("deep-diff-forge", "rpc", "--allow-repo", "/path/to/repo")) as client:
    outline = client.call(
        "review.change",
        repo="/path/to/repo",
        selection="working_tree",
        view="outline",
        max_bytes=16384,
    )
    first_addr = outline["files"][0]["hunks"][0]["address"]
    hunks = client.call(
        "review.change",
        repo="/path/to/repo",
        selection="working_tree",
        view="hunks",
        addresses=[first_addr],
    )
```

### Talking to a running daemon via `SocketClient`

```python
import subprocess
from deep_diff_forge import SocketClient

socket_path = subprocess.check_output(["deep-diff-forge", "daemon", "path"], text=True).strip()
client = SocketClient(socket_path, timeout=30.0)
health = client.call("daemon.health")
print(health["version"], health["protocol"])  # e.g. 0.11.0 3
```

## Protocol 3 Diagnostics & Error Handling

`diagnostics_patch(...)` negotiates `engine.initialize(protocol=3)` once per
call, follows `dg0` cursors page by page, rebuilds the canonical byte form, and
checks SHA-256 equality against `page.document_sha256`.

Exported exceptions (`__all__`):

- **`RpcError(code, message)`** — a JSON-RPC or domain error returned by the
  daemon (for example `4` patch parse failed, `7` input contract refused, `3`
  Git failed, `8` resource exhausted, `2` protocol unsupported, `-32601` method
  not found). For `diff.diagnostics`, a parse failure names where the patch
  failed and never echoes patch text.
- **`ProtocolTooOld(daemon, needed, method)`** — raised by `diagnostics_patch`
  against a daemon older than engine protocol 3 (or one that claims protocol ≥ 3
  yet answers `-32601`). Nothing is synthesized from older methods.
- **`TransportError`** — the daemon or `rpc` child could not be reached,
  exceeded `timeout` seconds (covering both write and read as one deadline),
  returned malformed/non-UTF-8/deeply-nested JSON or a mismatched response ID,
  sent an answer line over `MAX_REQUEST_BYTES` (80 MiB counting its newline), or
  was called after `close()`. A misbehaving `StdioClient` child (or one that
  answers before consuming the full request) is killed; an early refusal sent by
  the daemon is still read and raised as `RpcError`.
- **`PagingError(pages, problem)`** (subclass of `TransportError`) — pages
  violated the `diff.diagnostics` paging contract or failed `document_sha256`
  verification.
- **`UnknownMembers(pages, members)`** (subclass of `PagingError`) — pages carry
  additive `v0` members unknown to this SDK version (at most 8 names, escaped
  and truncated to 64 characters), so canonical bytes cannot be rebuilt and
  checked; upgrade the SDK.
- **`ValueError`** — raised before sending if a request contains a lone
  surrogate (unencodable in UTF-8) or exceeds `MAX_REQUEST_BYTES` (80 MiB).

## Running Tests

The test suite exercises both `StdioClient` and `SocketClient` against the built
`deep-diff-forge` binary:

```bash
cargo build -p deep-diff-forge-cli
DDF_BIN=target/debug/deep-diff-forge python3 -I sdk/python/test_deep_diff_forge.py
```

The full protocol and all 17 daemon methods are documented in
[docs/API_AND_IPC.md](https://github.com/Louranicas/deep-diff-forge/blob/main/docs/API_AND_IPC.md).
