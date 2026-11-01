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
import struct
import subprocess
import time

__all__ = ["RpcError", "TransportError", "StdioClient", "SocketClient"]

# The workspace version this client was released with (cli/tests/python_sdk.rs keeps it equal).
__version__ = "0.5.0"

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
        try:
            request = line.encode() + b"\n"
        except UnicodeEncodeError as error:
            # Python strings can hold a lone surrogate; UTF-8, and so the daemon, cannot.
            raise ValueError(f"the request holds a string that is not valid Unicode: {error.reason} "
                             f"at character {error.start}") from error
        if len(request) > MAX_REQUEST_BYTES:
            raise ValueError("request exceeds the daemon's maximum size")
        answer = self._exchange(request)
        try:
            response = json.loads(answer.decode("utf-8"))
            return _result(response, request_id)
        except TransportError as error:
            self._invalid(error)
        except ValueError as error:
            # An answer that is not UTF-8 JSON is a transport failure, as in the Rust SDK. The
            # daemon's own errors (RpcError) pass through: the child answered properly.
            self._invalid(TransportError(f"not a valid response: {error}"))

    def _invalid(self, error):
        """Raise ``error``, the transport failure for an answer that is not a valid response."""
        raise error

    def review_patch(self, patch, require_files=False, require_hunks=False):
        """The ``deep-diff-forge.review.v0`` document for ``patch``."""
        return self.call("diff.compute", patch=patch, require_files=require_files, require_hunks=require_hunks)

    def rank_patch(self, patch, require_files=False, require_hunks=False):
        """The sealed ``deep-diff-forge.rank.v0`` document for ``patch``: its files by review risk."""
        return self.call("diff.rank", patch=patch, require_files=require_files, require_hunks=require_hunks)

    def verify_findings(self, patch, findings, require_files=False, require_hunks=False):
        """The sealed ``deep-diff-forge.findings.v0`` verdict on ``findings`` (finding objects) for
        ``patch``: each accepted with its anchor normalised, or refused with a reason. Protocol 2.

        The findings travel as one request, so a finding holding a string UTF-8 cannot carry (a lone
        surrogate, which ``json.loads`` accepts) raises ``ValueError`` naming its index before
        anything is sent; the CLI, reading JSONL bytes, refuses such a line as ``malformed``."""
        findings = list(findings)
        for index, finding in enumerate(findings):
            try:
                json.dumps(finding, ensure_ascii=False).encode()
            except UnicodeEncodeError as error:
                raise ValueError(f"finding {index} holds a string that is not valid Unicode: {error.reason}") from error
        return self.call("findings.verify", patch=patch, findings=findings, require_files=require_files,
                         require_hunks=require_hunks)

    def methods(self):
        """Every daemon method with its params, result and protocol version."""
        return self.call("engine.methods")["methods"]


class StdioClient(_Client):
    """``deep-diff-forge rpc`` as a child process; close it (or use ``with``) to end it. Each request
    must be taken and its answer arrive within ``timeout`` seconds, and the answer must fit the
    daemon's size cap and be a valid response; otherwise the child is killed and ``TransportError``
    raised, so a stuck, runaway or misbehaving child cannot hang or exhaust the caller. The
    deadline covers the write too: a child that stops reading cannot block a large request."""

    def __init__(self, command=("deep-diff-forge", "rpc"), timeout=30.0):
        super().__init__()
        self._timeout = timeout
        self._buffer = b""
        self._process = subprocess.Popen(list(command), stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                         stderr=subprocess.DEVNULL)
        # Both pipes are non-blocking and driven by one selector under one deadline: a blocking
        # write of a request larger than the pipe buffer would hang on a child that stops reading,
        # before any deadline was armed.
        os.set_blocking(self._process.stdin.fileno(), False)
        os.set_blocking(self._process.stdout.fileno(), False)

    def _exchange(self, request):
        deadline = time.monotonic() + self._timeout
        pending = memoryview(request)
        stdin, stdout = self._process.stdin.fileno(), self._process.stdout.fileno()
        with selectors.DefaultSelector() as selector:
            selector.register(stdin, selectors.EVENT_WRITE)
            selector.register(stdout, selectors.EVENT_READ)
            # Read while writing, so a child that answers (or refuses) early cannot deadlock us.
            while pending or b"\n" not in self._buffer:
                if len(self._buffer) > MAX_REQUEST_BYTES:
                    self._fail("the answer exceeds the daemon's maximum size")
                remaining = deadline - time.monotonic()
                ready = selector.select(remaining) if remaining > 0 else []
                if not ready:
                    self._fail(f"no answer within {self._timeout} s")
                for key, _ in ready:
                    if key.fd == stdin:
                        pending = self._write(selector, pending)
                    else:
                        chunk = os.read(stdout, 1 << 16)
                        if not chunk and b"\n" not in self._buffer:
                            self._fail("deep-diff-forge rpc closed without answering")
                        if not chunk:
                            pending = pending[:0]  # it answered, then closed: nothing more to send
                        self._buffer += chunk
        line, _, self._buffer = self._buffer.partition(b"\n")
        return line

    def _write(self, selector, pending):
        """Write what the pipe takes of ``pending``; the rest, or nothing once it is all written."""
        try:
            pending = pending[os.write(self._process.stdin.fileno(), pending):]
        except BlockingIOError:
            return pending
        except BrokenPipeError:
            # The child stopped reading: it may still answer (a refusal), or close without one.
            pending = pending[:0]
        if not pending:
            selector.unregister(self._process.stdin.fileno())
        return pending

    def _invalid(self, error):
        self._process.kill()
        self._process.wait()
        raise error

    def _fail(self, message):
        self._invalid(TransportError(message))

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


def _peer_uid(connection):
    """The effective user ID of the daemon at the other end, or None where Python cannot read it.

    Linux reads ``SO_PEERCRED`` (pid, uid, gid); macOS reads ``LOCAL_PEERCRED`` (a ``xucred``
    whose second field is the uid). The daemon checks its clients itself on every platform.
    """
    if hasattr(socket, "SO_PEERCRED"):
        _, uid, _ = struct.unpack("3i", connection.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED,
                                                             struct.calcsize("3i")))
        return uid
    if hasattr(socket, "LOCAL_PEERCRED"):
        sol_local = getattr(socket, "SOL_LOCAL", 0)
        raw = connection.getsockopt(sol_local, socket.LOCAL_PEERCRED, 256)
        return struct.unpack_from("=II", raw)[1]
    return None


class SocketClient(_Client):
    """A running ``deep-diff-forge daemon``; one connection per call, as the daemon expects."""

    def __init__(self, path, timeout=30.0):
        super().__init__()
        self._path = str(path)
        self._timeout = timeout

    def _exchange(self, request):
        # A missing socket, a refused connection or a timeout is "could not be reached", the
        # same TransportError the stdio client raises, not a bare OSError.
        try:
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
                connection.settimeout(self._timeout)
                connection.connect(self._path)
                # Refuse a daemon of another user before sending it anything, as the Rust client does.
                uid = _peer_uid(connection)
                if uid is not None and uid != os.geteuid():
                    raise TransportError(f"the daemon at {self._path} runs as uid {uid}, not uid {os.geteuid()}")
                connection.sendall(request)
                # Bytes, not text: the caller decodes under its TransportError mapping.
                with connection.makefile("rb") as reader:
                    answer = reader.readline(MAX_REQUEST_BYTES + 1)
        except OSError as error:
            raise TransportError(f"the daemon at {self._path} could not be reached: {error}") from error
        if not answer.endswith(b"\n"):
            raise TransportError("the daemon closed without a complete answer")
        return answer
