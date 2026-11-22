# Module index: docs/ARCHITECTURE.md#crates-as-built-030
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
__version__ = "0.11.0-dev"

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
        except (ValueError, RecursionError) as error:
            # An answer that is not UTF-8 JSON is a transport failure, as in the Rust SDK; so is
            # one nested too deeply for json.loads, which raises RecursionError, not ValueError.
            # The daemon's own errors (RpcError) pass through: the child answered properly.
            self._invalid(TransportError(f"not a valid response: {error}"))

    def _invalid(self, error):
        """Raise ``error``, the transport failure for an answer that is not a valid response."""
        raise error

    def review_patch(self, patch, require_files=False, require_hunks=False):
        """The ``deep-diff-forge.review.v0`` document for ``patch``."""
        return self.call("diff.compute", patch=patch, require_files=require_files, require_hunks=require_hunks)

    def rank_patch(self, patch, require_files=False, require_hunks=False):
        """The sealed ``deep-diff-forge.rank.v0`` document for ``patch``: its files by review risk.

        Its ``canonical_json`` member is the document's exact bytes, what ``deep-diff-forge
        --stdin-patch --rank --json`` prints (newline included): seal or compare those, not a
        re-serialisation of the dict."""
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
    """``deep-diff-forge rpc`` as a child process; close it (or use ``with``) to end it. Each answer
    must arrive within ``timeout`` seconds, and the answer line must fit the daemon's size cap
    (``MAX_REQUEST_BYTES``, its newline counted) and be a valid response; otherwise the child is
    killed and ``TransportError`` raised, so a stuck, runaway or misbehaving child cannot hang or
    exhaust the caller. The deadline covers the write too: a child that stops reading cannot block
    a large request. An answer that arrives before the whole request is taken is returned, and the
    child, now out of step with the stream, is killed, also when it broke the pipe. Once the client
    is closed or its child killed, both pipes are closed and every call raises ``TransportError``."""

    def __init__(self, command=("deep-diff-forge", "rpc"), timeout=30.0):
        super().__init__()
        self._timeout = timeout
        self._buffer = bytearray()
        self._ended = None
        self._process = subprocess.Popen(list(command), stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                         stderr=subprocess.DEVNULL)
        # Both pipes are non-blocking and driven by one selector under one deadline: a blocking
        # write of a request larger than the pipe buffer would hang on a child that stops reading,
        # before any deadline was armed.
        os.set_blocking(self._process.stdin.fileno(), False)
        os.set_blocking(self._process.stdout.fileno(), False)

    def _exchange(self, request):
        if self._ended:
            raise TransportError(f"deep-diff-forge rpc has ended: {self._ended}")
        deadline = time.monotonic() + self._timeout
        pending = memoryview(request)
        stdin, stdout = self._process.stdin.fileno(), self._process.stdout.fileno()
        # The buffer grows in place and only new bytes are searched, so a long answer is read in
        # time linear in its size (before, each chunk copied and rescanned the whole buffer).
        newline = self._buffer.find(b"\n")
        with selectors.DefaultSelector() as selector:
            selector.register(stdin, selectors.EVENT_WRITE)
            selector.register(stdout, selectors.EVENT_READ)
            # Read while writing, so a child that answers (or refuses) early cannot deadlock us.
            # A whole answer line ends the exchange even if part of the request is unsent.
            while newline < 0:
                if len(self._buffer) >= MAX_REQUEST_BYTES:
                    self._fail("the answer exceeds the daemon's maximum size")
                remaining = deadline - time.monotonic()
                ready = selector.select(remaining) if remaining > 0 else []
                if not ready:
                    self._fail(f"no answer within {self._timeout} s")
                for key, _ in ready:
                    if key.fd == stdin:
                        pending = self._write(selector, pending)
                        continue
                    try:
                        chunk = os.read(stdout, 1 << 16)
                    except BlockingIOError:
                        continue
                    except OSError as error:
                        self._fail(f"reading the answer of deep-diff-forge rpc failed: {error}")
                    if not chunk:
                        self._fail("deep-diff-forge rpc closed without answering")
                    scanned = len(self._buffer)
                    self._buffer += chunk
                    newline = self._buffer.find(b"\n", scanned)
        # The cap counts the newline, as the daemon's does for a request line.
        if newline + 1 > MAX_REQUEST_BYTES:
            self._fail("the answer exceeds the daemon's maximum size")
        line = bytes(self._buffer[:newline])
        del self._buffer[:newline + 1]
        if pending is None or pending:
            # It answered before taking the whole request: the unsent tail would garble the next
            # one, so the out-of-step child is killed and the next call is a TransportError.
            self._end("it answered before taking the whole request and was killed")
        return line

    def _write(self, selector, pending):
        """Write what the pipe takes of ``pending`` and return the rest: empty once it is all
        written, None once the child stopped reading (a broken pipe: the request was not taken)."""
        try:
            pending = pending[os.write(self._process.stdin.fileno(), pending):]
        except BlockingIOError:
            return pending
        except BrokenPipeError:
            # The child stopped reading: it may still answer (a refusal), or close without one.
            pending = None
        if not pending:
            selector.unregister(self._process.stdin.fileno())
        return pending

    def _end(self, reason):
        """End the child (killed unless it already exited) and close both pipes."""
        if self._process.poll() is None:
            self._process.kill()
        self._process.wait()
        for pipe in (self._process.stdin, self._process.stdout):
            try:
                pipe.close()
            except OSError:
                pass
        self._ended = reason

    def _invalid(self, error):
        self._end("it was killed after a failed call")
        raise error

    def _fail(self, message):
        self._invalid(TransportError(message))

    def close(self):
        if self._ended:
            return
        try:
            self._process.stdin.close()
        except BrokenPipeError:
            pass
        try:
            self._process.wait(timeout=self._timeout)
        except subprocess.TimeoutExpired:
            pass
        self._end("the client was closed")

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.close()


def _peer_uid(connection):
    """The effective user ID of the daemon at the other end, or None where Python cannot read it.

    Linux reads ``SO_PEERCRED`` (pid, uid, gid: a signed ``pid_t`` and two unsigned ids, so a uid
    of 2^31 or more is not read as negative); macOS reads ``LOCAL_PEERCRED`` (a ``xucred`` whose
    second field is the uid). The daemon checks its clients itself on every platform.
    """
    if hasattr(socket, "SO_PEERCRED"):
        _, uid, _ = struct.unpack("iII", connection.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED,
                                                              struct.calcsize("iII")))
        return uid
    if hasattr(socket, "LOCAL_PEERCRED"):
        sol_local = getattr(socket, "SOL_LOCAL", 0)
        raw = connection.getsockopt(sol_local, socket.LOCAL_PEERCRED, 256)
        return struct.unpack_from("=II", raw)[1]
    return None


class SocketClient(_Client):
    """A running ``deep-diff-forge daemon``; one connection per call, as the daemon expects. The
    whole call, connecting, sending and reading the answer, must end within ``timeout`` seconds,
    and the answer line must fit the daemon's size cap (``MAX_REQUEST_BYTES``, its newline
    counted). An answer the daemon sends before it has taken the whole request (a refusal) is
    read, not lost."""

    def __init__(self, path, timeout=30.0):
        super().__init__()
        self._path = str(path)
        self._timeout = timeout

    def _exchange(self, request):
        # A missing socket, a refused connection or a failed exchange is "could not be reached",
        # the same TransportError the stdio client raises, not a bare OSError.
        deadline = time.monotonic() + self._timeout
        try:
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
                return self._converse(connection, request, deadline)
        except socket.timeout as error:
            raise TransportError(f"no answer from the daemon at {self._path} within {self._timeout} s") from error
        except OSError as error:
            raise TransportError(f"the daemon at {self._path} could not be reached: {error}") from error

    def _converse(self, connection, request, deadline):
        def arm():
            # One deadline for the whole call: each step waits only for what is left of it. Before,
            # each step had the whole timeout, so a daemon sending a byte at a time held the caller.
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise socket.timeout("the deadline passed")
            connection.settimeout(remaining)

        arm()
        connection.connect(self._path)
        # Refuse a daemon of another user before sending it anything, as the Rust client does.
        uid = _peer_uid(connection)
        if uid is not None and uid != os.geteuid():
            raise TransportError(f"the daemon at {self._path} runs as uid {uid}, not uid {os.geteuid()}")
        # A daemon that refuses a request early (its read budget is full) answers and closes before
        # the rest arrives, so the send breaks with the answer already waiting: read it, and report
        # the broken send only when there is none (as the Rust client does).
        unsent, broken = memoryview(request), None
        while unsent:
            arm()
            try:
                unsent = unsent[connection.send(unsent):]
            except (BrokenPipeError, ConnectionResetError) as error:
                broken = error
                break
        # Bytes, not text: the caller decodes under its TransportError mapping.
        answer, newline = bytearray(), -1
        while newline < 0:
            if len(answer) >= MAX_REQUEST_BYTES:
                raise TransportError("the answer exceeds the daemon's maximum size")
            arm()
            chunk = connection.recv(1 << 16)
            if not chunk:
                if broken is not None:
                    raise broken
                raise TransportError("the daemon closed without a complete answer")
            scanned = len(answer)
            answer += chunk
            newline = answer.find(b"\n", scanned)
        # The cap counts the newline, as the daemon's does for a request line.
        if newline + 1 > MAX_REQUEST_BYTES:
            raise TransportError("the answer exceeds the daemon's maximum size")
        return bytes(answer[:newline + 1])
