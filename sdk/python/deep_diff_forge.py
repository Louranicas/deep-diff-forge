"""Thin Python client for Deep-Diff-Forge's daemon protocol (JSON-RPC 2.0, one message per line).

Standard library only. Two transports speak the same protocol:

- ``StdioClient`` runs ``deep-diff-forge rpc`` and talks to it over a pipe; no daemon is needed.
- ``SocketClient`` connects to a running ``deep-diff-forge daemon`` over its Unix socket.

``review_patch`` returns the ``deep-diff-forge.review.v0`` document that
``deep-diff-forge --stdin-patch --json`` prints for the same bytes. A JSON-RPC error raises
``RpcError``, whose ``code`` is the daemon's: 4 for a patch that does not parse, 7 for a broken
input contract (``require_files``, ``require_hunks``), -32601 for an unknown method.
"""
import itertools
import json
import socket
import subprocess

__all__ = ["RpcError", "StdioClient", "SocketClient"]

# The daemon refuses a request line longer than this (deep_diff_forge_daemon::MAX_REQUEST_BYTES).
MAX_REQUEST_BYTES = 80 * 1024 * 1024


class RpcError(Exception):
    """A JSON-RPC error answer."""

    def __init__(self, code, message):
        super().__init__(f"{code}: {message}")
        self.code = code
        self.message = message


class _Client:
    def __init__(self):
        self._ids = itertools.count(1)

    def call(self, method, **params):
        """Call ``method`` with keyword ``params`` and return its result."""
        request_id = next(self._ids)
        line = json.dumps({"jsonrpc": "2.0", "id": request_id, "method": method, "params": params},
                          separators=(",", ":"), ensure_ascii=False)
        if len(line.encode()) + 1 > MAX_REQUEST_BYTES:
            raise ValueError("request exceeds the daemon's maximum size")
        response = json.loads(self._exchange(line))
        if "error" in response:
            raise RpcError(response["error"]["code"], response["error"]["message"])
        if response.get("id") != request_id:
            raise RpcError(-32603, f"response id {response.get('id')!r} does not match {request_id}")
        return response["result"]

    def review_patch(self, patch, require_files=False, require_hunks=False):
        """The ``deep-diff-forge.review.v0`` document for ``patch``."""
        return self.call("diff.compute", patch=patch, require_files=require_files, require_hunks=require_hunks)

    def methods(self):
        """Every daemon method with its params, result and protocol version."""
        return self.call("engine.methods")["methods"]


class StdioClient(_Client):
    """``deep-diff-forge rpc`` as a child process; close it (or use ``with``) to end it."""

    def __init__(self, command=("deep-diff-forge", "rpc")):
        super().__init__()
        self._process = subprocess.Popen(list(command), stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                         stderr=subprocess.DEVNULL, text=True, encoding="utf-8")

    def _exchange(self, line):
        self._process.stdin.write(line + "\n")
        self._process.stdin.flush()
        answer = self._process.stdout.readline()
        if not answer:
            raise RpcError(-32603, "deep-diff-forge rpc closed without answering")
        return answer

    def close(self):
        if self._process.stdin and not self._process.stdin.closed:
            self._process.stdin.close()
        self._process.wait(timeout=10)

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.close()


class SocketClient(_Client):
    """A running ``deep-diff-forge daemon``; one connection per call, as the daemon expects."""

    def __init__(self, path, timeout=30.0):
        super().__init__()
        self._path = str(path)
        self._timeout = timeout

    def _exchange(self, line):
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
            connection.settimeout(self._timeout)
            connection.connect(self._path)
            connection.sendall((line + "\n").encode())
            with connection.makefile("r", encoding="utf-8") as reader:
                answer = reader.readline(MAX_REQUEST_BYTES + 1)
        if not answer.endswith("\n"):
            raise RpcError(-32603, "the daemon closed without a complete answer")
        return answer
