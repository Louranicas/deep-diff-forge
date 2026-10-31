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

`SocketClient` speaks the same protocol to a running `deep-diff-forge daemon`. The protocol and
every method are documented in
[docs/API_AND_IPC.md](https://github.com/Louranicas/deep-diff-forge/blob/main/docs/API_AND_IPC.md).
