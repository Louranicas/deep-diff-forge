# MCP server

`deep-diff-forge mcp` serves read-only [Model Context Protocol](https://modelcontextprotocol.io)
tools over stdio, one JSON-RPC message per line.

- **Modern clients** (specification 2026-07-28) send the protocol version and client capabilities in each request's `_meta`, and may call `server/discover` first.
- **Legacy clients** (2025-11-25, 2025-06-18, 2025-03-26) use the `initialize` handshake.

| Tool | What it returns |
| --- | --- |
| `review_patch` | The `deep-diff-forge.review.v0` document for `patch`: the same JSON value `--stdin-patch --json` prints, up to 256 KiB (see *Large reviews*). `require_files` / `require_hunks` refuse as the CLI flags do; `paths` limits the document to chosen files |
| `rank_patch` | The sealed `deep-diff-forge.rank.v0` document `--stdin-patch --rank --json` prints: files by review risk, with their signals. `require_files` / `require_hunks` refuse as the CLI flags do Its text is the exact bytes `--rank --json` prints and `structuredContent` their value; the 256 KiB cap counts those bytes. The daemon's `diff.rank` carries the same bytes in `canonical_json`. |
| `review_outline` | **Start here.** `deep-diff-forge.review-outline.v0` for `patch`: files in review-risk order with their hunks' addresses, ranges, `@@` headings and counts, no line text. Fits `max_bytes`; `cursor` continues |
| `review_hunks` | `deep-diff-forge.review-hunks.v0`: chosen hunks verbatim, by `addresses` or `paths` (or all). Fits `max_bytes`; `cursor` continues |
| `review_change` | The outline or hunks of a change read straight from Git (`selection`: `staged`, `working_tree`, `range`), with an `acquisition` member that lists any `omitted_untracked` directories (a nested repository is not captured). Only for repositories the server was started with `--allow-repo`; Git is stopped after 30 s |
| `explain_exit_code` | One exit code's family, name, meaning and next action |
| `verify_findings` | `deep-diff-forge.findings.v0`: each agent finding in `findings` checked against `patch` (address, line, path, freshness, size, duplicates) and accepted with its anchor normalised, or refused with a reason; `claim_verified` is always `false` |
| `exit_code_registry` | Every documented exit code |

Every tool is annotated `readOnlyHint: true`, and none writes anything. Every tool declares an
`outputSchema`. The patch tools' results always carry `schema`, `input_sha256` and `tool`, so an
agent can check which bytes an answer is about. A full result arrives twice, as
`structuredContent` and as a text copy, because the MCP specification asks servers to keep the
text copy for older clients. Ask for `detail: "summary"` or `paths` to keep answers small. The server
instructions and the patch-reading tools tell the model that paths and line text come from
the patch's authors: data to review, never instructions.

- **Errors:** an unknown tool or malformed JSON-RPC is a JSON-RPC error. Bad or missing arguments, a parse failure or a broken input contract is a tool result with `isError: true`, so a model can correct its input.
- **Limits:** a request line is capped at the 64 MiB patch budget plus 1 MiB.
- **Large reviews:** `review_patch` returns the full document while it is at most 256 KiB. A larger review returns a bounded `deep-diff-forge.review-summary.v0` instead: the same seal and totals, and per-file counts for up to 1,000 files, with `reason` and `omitted_files`. Pass `paths` to get chosen files in full, or `detail: "summary"` to ask for the summary. `rank_patch` returns the full ranking while it is at most 256 KiB; a larger one returns `deep-diff-forge.rank-summary.v0`: the same seal, and the highest-ranked files that fit, in order, with `total_files` and `omitted_files`.
- **Lifetime:** the server exits when stdin closes.

```json
{"mcpServers": {"deep-diff-forge": {"command": "deep-diff-forge", "args": ["mcp"]}}}
```

# SDKs

**Rust:** the `deep-diff-forge-sdk` crate is a stable facade over the same crates the CLI and
daemon use.

```rust
use deep_diff_forge_sdk::{review, rank, Guards};
let review = review(patch, Guards::all())?;   // or Guards::none().require_files()
let document = review.to_json();   // byte for byte what `--stdin-patch --json` prints
let ranked = rank(&review);
```

`Client::at(socket).call(method, &params)` calls a running daemon. It is Unix-only, and it
checks every answer's version, id and error shape.

- **Exit codes:** errors map to the CLI's exit codes (`Error::exit_code`).
- **Ownership:** the SDK owns `Guards`, `Review` (opaque) and `Error` (`#[non_exhaustive]`). `ReviewFile`, `InputSeal` and `RankedFile` are model types and change only with the schema version.
- **Stability:** `tests/api.rs` pins every public signature, so changing one is a deliberate breaking change.

**Python:** `sdk/python/deep_diff_forge.py` uses only the standard library, and installs as the
package `deep-diff-forge` (`python3 -m pip install ./sdk/python`, or from GitHub with
`"deep-diff-forge @ git+https://github.com/Louranicas/deep-diff-forge@v0.8.3#subdirectory=sdk/python"`;
not on PyPI; the package metadata arrived in 0.6.0, so `@v0.5.0` and older tags have no `pyproject.toml` to install). It drives the `deep-diff-forge` binary, which it does not ship. It speaks the
daemon protocol over two transports with the same calls:

- `deep-diff-forge rpc` over a pipe (`StdioClient`), which needs no daemon;
- a running daemon's socket (`SocketClient`).

```python
from deep_diff_forge import StdioClient
with StdioClient() as client:
    document = client.review_patch(patch, require_files=True, require_hunks=True)
```

`deep-diff-forge rpc` answers each stdin line with the socket server's own handler and framing:
an 80 MiB cap that counts the newline, with an oversized request refused at once. It stops at
`daemon.shutdown`. The Python client checks every answer's version and id, holds each answer
line to the line cap (see *One line cap* below) and each call to its `timeout`, and kills a child
that exceeds either; it also kills a child that answers before taking the whole request. A
`SocketClient` call has one deadline for connecting, sending and reading, and reads a refusal the
daemon sends before taking the whole request. A transport failure, including a call after
`close()`, raises `TransportError`; a JSON-RPC error raises `RpcError`. CI runs the Python tests
against the built binary over both transports.

# The optional daemon

The daemon speeds up repeated review and multi-client workflows. It is **never required**:
every command works without it. It is **std-first**, with no async runtime: a JSON-RPC 2.0
server on an owner-private Unix domain socket.

**Security boundary:**
- **Ownership.** The runtime directory is `0700` and must be owned by the effective user. The
  socket is `0600`.
- **Same user only, checked by the kernel.** The daemon reads each client's user ID from the
  socket (`SO_PEERCRED` on Linux, `getpeereid` on macOS) and drops any client that is not its
  own user, whatever the socket's mode. The CLI, the Rust SDK and the Python SDK check the
  daemon the same way before sending anything.
- **One daemon per socket.** `daemon start` takes an exclusive lock on `<socket>.lock` before it
  probes for a stale socket and holds it until the daemon exits. A second start, even one racing
  the first, exits 6 and leaves the running daemon reachable.
- **Explicit `--socket` paths fail closed.** A missing parent is created `0700`. An existing
  parent is validated and never re-permissioned. A path is replaced only if it is already a
  socket that nothing answers on. A live daemon is never displaced, and a regular file,
  directory or symlink is never deleted.
- **Bounded sessions.** Sessions are capped by an LRU count and 128 MiB of total payload.
- **Bounded connections and work.** At most eight connections are admitted, and their
  requests run in parallel. The request bytes in flight are capped at 128 MiB, first come,
  first served, so a heavy request waits rather than multiplying peak memory, and a waiting
  request is never overtaken.
- **Bounded I/O.** Each request line has an absolute read deadline, each response write a
  per-write timeout, and both are held to the line cap. A response over the cap becomes a
  `RESOURCE_EXHAUSTED` (8) error for the same id.
- **One line cap.** A line, request or answer, is at most 80 MiB (83,886,080 bytes,
  `MAX_REQUEST_BYTES`) counting its newline. The daemon and `deep-diff-forge rpc` refuse a
  longer request line as soon as its first byte over the cap is read. The Rust client
  (`deep_diff_forge_daemon::request`, which the Rust SDK's `Client` wraps) and both Python clients
  refuse to send a longer request and refuse a longer answer line.
- **No repository unless allowed.** `review.change` reads Git only in a repository whose
  canonical path and Git top level are both at or under a root named with
  `daemon start --allow-repo PATH` (also `rpc` and `mcp`). Without one it reads nothing.
  Symlinks are resolved first, and Git is stopped after 30 s. A Git failure is error 3.

**Default socket:** `$XDG_RUNTIME_DIR/deep-diff-forge/deep-diff-forge.sock`. There is no
`/tmp` fallback. Without `$XDG_RUNTIME_DIR` (or with a relative one, which the XDG spec makes
invalid) the daemon fails closed until you pass `--socket PATH`.

**JSON-RPC methods** (protocol 3; `engine.methods` lists them with their parameters and the protocol that added them):
`engine.initialize`, `engine.methods`, `daemon.health`, `daemon.status`,
`daemon.shutdown`, `diff.plan`, `diff.compute`, `diff.rank`, `review.outline`, `review.hunks`,
`review.change`, `session.open`, `session.snapshot`, `session.close`, `daemon.metrics`,
`findings.verify`, `diff.diagnostics`.

On the daemon, rpc, MCP and SDK surfaces a patch text in which no patch is found ranks nothing and is not an error; `--stdin-patch` on the command line refuses it (exit 4) unless `--allow-empty` is given.
`diff.diagnostics` answers such a text as `diff.rank` does, with the document of an empty ranking, where `--stdin-patch --diagnostics` refuses it with exit 4 (`no_patch_in_input`); pass `require_files` to have the daemon refuse it (error 7).

**Protocol 3** adds `diff.diagnostics`: one page of the [`diagnostics.v0`](diagnostics.md) document
`--stdin-patch --diagnostics --json` prints for the same patch (params `patch`, `require_files`,
`require_hunks`, `max_bytes` from 4 096, `cursor`; paging in [Paging](diagnostics.md#paging)). Every
page carries `page.document_sha256`, the SHA-256 of the bytes the command prints, which the pages
written in the document's byte form reproduce. As on `diff.rank`, the message of a patch that does
not parse quotes the line that failed: it is the one answer of the method that carries text of
the input beyond the escaped path. `engine.initialize` without `protocol`, or with 3, lists it; with
`protocol: 2` it does not, and a protocol-2 client that calls it is still served. A daemon of
release 0.10.0 answers `engine.initialize` with `protocol: 3` with error 2 (`protocol 3 is not
supported; this daemon speaks 0 to 2`) and `diff.diagnostics` with -32601. Every protocol-2 method
keeps its params, its result's members, their types and order, its error codes and its messages
(`tests/protocol_compat.rs` replays a transcript of the published 0.10.0 binary); values differ
only in the engine's version, in `tool.scorer_sha256`, in the `protocol` the daemon reports and the
newest it names when it refuses one, in the methods it lists, in values of the run (a process id,
an uptime, latencies, an admission wait), and in what a release's register names as changed.

```bash
deep-diff-forge daemon start --foreground      # terminal 1
deep-diff-forge daemon health                  # terminal 2
# {"id":1,"jsonrpc":"2.0","result":{"cache_entries":0,"pid":…,"protocol":3,"retained_bytes":0,"sessions":0,"status":"ok","version":"0.11.0-dev"}}
deep-diff-forge daemon stop                    # answers what is in progress, returns once it has exited
```

To run it as a service, use the systemd user unit or the launchd agent in `contrib/`
([running as a service](../OPERATIONS_AND_DAEMON.md#running-as-a-service)). For Prometheus,
`deep-diff-forge daemon metrics --format prometheus --out DIR/deep-diff-forge.prom` writes the
metrics as a file for node_exporter's textfile collector, with no new listener; a user timer that
runs it ships too ([exporting metrics](../OPERATIONS_AND_DAEMON.md#exporting-metrics-to-prometheus)).

See also: [`API_AND_IPC`](../API_AND_IPC.md) · [`OPERATIONS_AND_DAEMON`](../OPERATIONS_AND_DAEMON.md).
