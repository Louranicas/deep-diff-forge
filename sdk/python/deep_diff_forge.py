# Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fsdk--python--deep_diff_forge
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
import os
import selectors
import socket
import subprocess
import time

__all__ = ["RpcError", "TransportError", "StdioClient", "SocketClient"]

# The daemon refuses a request line longer than this (deep_diff_forge_daemon::MAX_REQUEST_BYTES).
MAX_REQUEST_BYTES = 80 * 1024 * 1024


class RpcError(Exception):
    """A JSON-RPC error answer."""

    def __init__(self, code, message):
        super().__init__(f"{code}: {message}")
        self.code = code
        self.message = message


class TransportError(Exception):
    """The peer could not be reached, or its answer was not a valid response to the request."""


def _result(response, request_id):
    """The result of a JSON-RPC 2.0 response to ``request_id``. The version, the id and the error
    object's shape are checked before anything in it is believed, errors included."""
    if not isinstance(response, dict) or response.get("jsonrpc") != "2.0":
        raise TransportError("not a JSON-RPC 2.0 response")
    # An invalid request is answered with id null (JSON-RPC 2.0 §5); one request per exchange makes
    # that error this call's, so only an error may carry it.
    null_error = "id" in response and response["id"] is None and "result" not in response
    if (response.get("id") != request_id or isinstance(response.get("id"), bool)) and not null_error:
        raise TransportError(f"response id {response.get('id')!r} does not match {request_id}")
    if ("result" in response) == ("error" in response):
        raise TransportError("a response must have exactly one of result and error")
    if "error" in response:
        error = response["error"]
        code = error.get("code") if isinstance(error, dict) else None
        message = error.get("message") if isinstance(error, dict) else None
        if not isinstance(code, int) or isinstance(code, bool) or not isinstance(message, str):
            raise TransportError("the error has no integer code and string message")
        raise RpcError(code, message)
    return response["result"]


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
        return _result(json.loads(self._exchange(line)), request_id)

    def review_patch(self, patch, require_files=False, require_hunks=False):
        """The ``deep-diff-forge.review.v0`` document for ``patch``."""
        return self.call("diff.compute", patch=patch, require_files=require_files, require_hunks=require_hunks)

    def rank_patch(self, patch, require_files=False, require_hunks=False):
        """The sealed ``deep-diff-forge.rank.v0`` document for ``patch``: its files by review risk."""
        return self.call("diff.rank", patch=patch, require_files=require_files, require_hunks=require_hunks)

    def methods(self):
        """Every daemon method with its params, result and protocol version."""
        return self.call("engine.methods")["methods"]


class StdioClient(_Client):
    """``deep-diff-forge rpc`` as a child process; close it (or use ``with``) to end it. Each answer
    must arrive within ``timeout`` seconds and fit the daemon's size cap; otherwise the child is
    killed and ``TransportError`` raised, so a stuck or runaway child cannot hang or exhaust the
    caller."""

    def __init__(self, command=("deep-diff-forge", "rpc"), timeout=30.0):
        super().__init__()
        self._timeout = timeout
        self._buffer = b""
        self._process = subprocess.Popen(list(command), stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                         stderr=subprocess.DEVNULL)
        os.set_blocking(self._process.stdout.fileno(), False)

    def _exchange(self, line):
        try:
            self._process.stdin.write(line.encode() + b"\n")
            self._process.stdin.flush()
        except BrokenPipeError:
            self._fail("deep-diff-forge rpc is not reading")
        return self._read_line().decode()

    def _read_line(self):
        deadline = time.monotonic() + self._timeout
        fd = self._process.stdout.fileno()
        with selectors.DefaultSelector() as selector:
            selector.register(fd, selectors.EVENT_READ)
            while b"\n" not in self._buffer:
                if len(self._buffer) > MAX_REQUEST_BYTES:
                    self._fail("the answer exceeds the daemon's maximum size")
                remaining = deadline - time.monotonic()
                if remaining <= 0 or not selector.select(remaining):
                    self._fail(f"no answer within {self._timeout} s")
                chunk = os.read(fd, 1 << 16)
                if not chunk:
                    self._fail("deep-diff-forge rpc closed without answering")
                self._buffer += chunk
        line, _, self._buffer = self._buffer.partition(b"\n")
        return line

    def _fail(self, message):
        self._process.kill()
        self._process.wait()
        raise TransportError(message)

    def close(self):
        if self._process.stdin and not self._process.stdin.closed:
            try:
                self._process.stdin.close()
            except BrokenPipeError:
                pass
        try:
            self._process.wait(timeout=self._timeout)
        except subprocess.TimeoutExpired:
            self._process.kill()
            self._process.wait()

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
        # A missing socket, a refused connection or a timeout is "could not be reached", the
        # same TransportError the stdio client raises, not a bare OSError.
        try:
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
                connection.settimeout(self._timeout)
                connection.connect(self._path)
                connection.sendall((line + "\n").encode())
                with connection.makefile("r", encoding="utf-8") as reader:
                    answer = reader.readline(MAX_REQUEST_BYTES + 1)
        except OSError as error:
            raise TransportError(f"the daemon at {self._path} could not be reached: {error}") from error
        if not answer.endswith("\n"):
            raise TransportError("the daemon closed without a complete answer")
        return answer
