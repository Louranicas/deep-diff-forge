# deep-diff-forge (Python client)

A standard-library client for [Deep-Diff-Forge](https://github.com/Louranicas/deep-diff-forge)'s
daemon protocol (JSON-RPC 2.0, one message per line). It drives the `deep-diff-forge` binary,
which it does not ship: install a release binary or build it with Cargo first.

```bash
python3 -m pip install ./sdk/python
```

```python
from deep_diff_forge import StdioClient   # `deep-diff-forge rpc` over a pipe, no daemon
with StdioClient() as client:
    document = client.review_patch(patch, require_files=True, require_hunks=True)
```

`SocketClient` speaks the same protocol to a running `deep-diff-forge daemon`.

A JSON-RPC error raises `RpcError` with the daemon's `code`. Everything else that goes wrong in
transport raises `TransportError`: a daemon or child that cannot be reached, sends no answer
within `timeout` seconds (for `SocketClient`, one deadline for the whole call), sends an answer
line over 80 MiB counting its newline (`MAX_REQUEST_BYTES`, the daemon's own line cap), or sends
an answer that is not a valid response to the request; and any call after `close()`. A child that
misbehaves, or that answers before taking the whole request, is killed. A refusal the daemon
sends before taking the whole request is read and raised as its `RpcError`, not lost. The protocol and
every method are documented in
[docs/API_AND_IPC.md](https://github.com/Louranicas/deep-diff-forge/blob/main/docs/API_AND_IPC.md).
