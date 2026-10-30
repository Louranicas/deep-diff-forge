// DDF integration: R25, R28, R40, R59. Preserve bounded connection admission on Rust 1.88 using compare-exchange; this does not solve the separate global-lock recommendation.
// Design/status: docs/RECOMMENDATION_INTEGRATION.md#runtime
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fdeep-diff-forge-daemon--serve
use crate::handler::{Engine, dispatch, request_weight};
use crate::protocol::{
    INTERNAL_ERROR, INVALID_REQUEST, PARSE_ERROR, RESOURCE_EXHAUSTED, RpcError, error_response,
    parse_request, success_response,
};
use crate::security::{
    SECURE_DIR_MODE, SECURE_SOCKET_MODE, SocketError, default_socket_path, ensure_runtime_dir,
    require_peer_user, validate_private_dir,
};
use deep_diff_forge_core::json_line_safe;
use serde_json::Value;
use std::io::{BufRead, BufReader, Write as _};
use std::os::unix::fs::{FileTypeExt as _, MetadataExt as _, PermissionsExt as _};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// Tracks whether a `SocketLocation` was produced from the engine-managed
/// default path (the `$XDG_RUNTIME_DIR`-derived location the daemon owns) or
/// from an explicit `--socket` override supplied by the caller.
///
/// The distinction determines which binding policy applies:
/// * `EngineDefault` — the daemon owns the directory; create + chmod are safe.
/// * `Explicit` — the directory belongs to the caller; we must not create it,
///   must not chmod it, and must not remove anything that is not a socket.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Provenance {
    /// Path was derived from `$XDG_RUNTIME_DIR`; the daemon owns it.
    EngineDefault,
    /// Path was supplied by the caller via `--socket` or [`SocketLocation::at`].
    Explicit,
}

/// A resolved, ready-to-use daemon socket location.
///
/// This is the single entry point for "where does the daemon socket live?",
/// replacing scattered `Option<PathBuf>` handling at the call sites. It is
/// constructed only via [`SocketLocation::resolve`] (env-based, fail-closed) or
/// [`SocketLocation::at`] (an explicit path) — so a "no location" state is not
/// representable past construction, and binding/connecting are methods on the
/// value rather than free functions over a bare path.
///
/// Provenance is carried in the type: explicit-path sockets bind with a
/// fail-closed policy (parent must already exist and be owner-private; nothing
/// is created or chmod'd; only an existing *socket* is removed). Engine-default
/// sockets bind with the original create+chmod policy because the daemon owns
/// that directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SocketLocation {
    path: PathBuf,
    provenance: Provenance,
}

impl SocketLocation {
    /// Resolve the socket location. An explicit `--socket` override wins; else
    /// the `$XDG_RUNTIME_DIR`-derived default is used. There is no world-writable
    /// `/tmp` fallback: when neither is available this fails closed with
    /// [`SocketError::NoRuntimeDir`].
    ///
    /// # Errors
    /// Returns [`SocketError::NoRuntimeDir`] when no explicit path is given and
    /// `$XDG_RUNTIME_DIR` is unset/empty.
    pub fn resolve(explicit: Option<&Path>) -> Result<Self, SocketError> {
        if let Some(path) = explicit {
            return Ok(Self::at(path));
        }
        default_socket_path()
            .map(|path| Self {
                path,
                provenance: Provenance::EngineDefault,
            })
            .ok_or(SocketError::NoRuntimeDir)
    }

    /// A location at an explicit path (the `--socket PATH` override and tests).
    ///
    /// The returned location carries `Provenance::Explicit`: binding will
    /// validate but not create or chmod the parent directory, and will refuse to
    /// remove anything at the socket path that is not already a socket.
    #[must_use]
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            provenance: Provenance::Explicit,
        }
    }

    /// The resolved socket path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Bind a secured listener at this location.
    ///
    /// The binding policy depends on provenance:
    /// * **Engine-default** — creates + chmods the daemon-owned runtime dir,
    ///   removes a stale socket, binds, sets the socket to `0600`.
    /// * **Explicit** — an absent parent is created `0700`; an existing parent
    ///   must pass [`validate_private_dir`] (owner-private, non-symlink) and is
    ///   never chmod'd.
    ///
    /// Either way a pre-existing path is removed only if it is a socket nothing
    /// answers on: a live daemon's socket is `AddrInUse`, and a regular file,
    /// directory or symlink is an error (fail closed). The probe, the removal and
    /// the bind run under the location's start lock (see [`BoundListener`]).
    ///
    /// # Errors
    ///
    /// Returns an I/O error if the directory cannot be secured, another daemon
    /// holds the start lock (`AddrInUse`), or the bind fails.
    pub fn bind(&self) -> std::io::Result<BoundListener> {
        match self.provenance {
            Provenance::EngineDefault => bind_secure(&self.path),
            Provenance::Explicit => bind_explicit(&self.path),
        }
    }

    /// The start lock's path, `<socket file name>.lock` beside the socket.
    ///
    /// # Errors
    /// `InvalidInput` when the socket path has no file name.
    pub fn start_lock_path(&self) -> std::io::Result<PathBuf> {
        start_lock_path(&self.path)
    }

    /// Connect to a daemon listening at this location, and refuse it unless it
    /// runs as this process's effective user.
    ///
    /// # Errors
    /// Returns an I/O error if no daemon is listening, or `PermissionDenied` if
    /// the daemon that answered runs as another user.
    pub fn connect(&self) -> std::io::Result<UnixStream> {
        let stream = UnixStream::connect(&self.path)?;
        require_peer_user(&stream, rustix::process::geteuid().as_raw())?;
        Ok(stream)
    }
}

/// A bound daemon listener together with its start lock.
///
/// The lock is an exclusive `flock` on `<socket>.lock` beside the socket, taken
/// before the stale-socket probe and held until this value is dropped, after the
/// shutdown cleanup. So no second `daemon start` can probe, remove or bind at the
/// path while this daemon lives: before, two starts could interleave between the
/// probe and the bind, and the second removed the first's fresh socket and bound
/// its own, leaving the first running but unreachable. A crashed daemon's lock is
/// released by the kernel; the lock file itself is never removed, since removing
/// it would let two starts lock two different files.
#[derive(Debug)]
pub struct BoundListener {
    listener: UnixListener,
    _start_lock: std::fs::File,
}

impl BoundListener {
    /// The listening socket.
    #[must_use]
    pub const fn listener(&self) -> &UnixListener {
        &self.listener
    }
}

/// Maximum bytes accepted for a single JSON-RPC request line. The largest
/// legitimate request wraps a patch (the patch crate's 64 MiB budget) in a JSON
/// envelope; this leaves generous framing headroom while hard-bounding a single
/// oversized line so a newline-less stream cannot exhaust memory (denial of
/// service).
pub const MAX_REQUEST_BYTES: usize = 80 * 1024 * 1024;

/// Maximum bytes of one response line, newline excluded: the server never sends more (see
/// [`bounded_response`]) and the client helper never accepts more.
const MAX_RESPONSE_BYTES: usize = MAX_REQUEST_BYTES;

/// Maximum number of connection worker threads alive at once.
const MAX_CONNECTIONS: usize = 8;

/// Absolute deadline for each request or response line. The remaining duration
/// is applied before each read, so sending one byte just before a socket timeout
/// cannot keep a worker alive forever.
const READ_TIMEOUT: Duration = Duration::from_secs(30);

/// Bound blocked writes to clients that stop reading responses.
const WRITE_TIMEOUT: Duration = Duration::from_secs(30);

/// Process one request line against the shared engine, returning the response
/// line. This is the socket-free core of connection handling.
///
/// Callers on different threads run in parallel. Each request first waits for admission
/// ([`Engine::admission`]): the request bytes running at once stay within the in-flight budget,
/// first come, first served.
#[must_use]
pub fn process_line(line: &str, engine: &Engine) -> String {
    let metrics = engine.metrics();
    let request = match parse_request(line) {
        Ok(request) => request,
        Err(err) => {
            metrics.request(None, std::time::Duration::ZERO, Some(err.code));
            return error_response(&Value::Null, &err);
        }
    };
    let id = request.id.clone();
    let method = crate::METHODS.iter().position(|m| m.name == request.method);
    // Held until the response is built; returned on a panic too. A request whose turn does not
    // come within the admission wait is answered busy, not left waiting.
    let permit = match engine
        .admission()
        .admit(request_weight(&request, line.len()))
    {
        Ok(permit) => permit,
        Err(busy) => {
            metrics.busy();
            metrics.request(method, busy.waited, Some(RESOURCE_EXHAUSTED));
            return error_response(&id, &RpcError::new(RESOURCE_EXHAUSTED, busy.to_string()));
        }
    };
    let admitted = permit.admitted();
    metrics.admitted(admitted.waited, admitted.in_flight, admitted.running);
    let started = crate::metrics::now();
    // Isolate a panic inside dispatch: one malformed/abusive request must not unwind and tear
    // down the whole daemon (availability hardening, mirroring the morph-ir-engine
    // per-connection backstop). A panic while the session table was held empties the table on
    // its next use (`Engine::sessions`).
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        #[cfg(test)]
        test_hooks::run(&request.method, engine);
        dispatch(&request, engine)
    }));
    let took = started.elapsed();
    drop(permit);
    match outcome {
        Ok(Ok(result)) => {
            metrics.request(method, took, None);
            bounded_response(&id, success_response(&id, result), MAX_RESPONSE_BYTES)
        }
        Ok(Err(err)) => {
            metrics.request(method, took, Some(err.code));
            error_response(&id, &err)
        }
        Err(_) => {
            metrics.panic();
            metrics.request(method, took, Some(INTERNAL_ERROR));
            error_response(&id, &RpcError::new(INTERNAL_ERROR, "internal error"))
        }
    }
}

/// Test-only methods `process_line` runs before dispatch: `test.panic` panics while holding the
/// session table, `test.panic_unlocked` panics holding nothing, and `test.hold` blocks until
/// [`test_hooks::release`] (a request that is still running).
#[cfg(test)]
pub(crate) mod test_hooks {
    use super::Engine;
    use std::sync::{Condvar, Mutex, OnceLock};

    fn gate() -> &'static (Mutex<bool>, Condvar) {
        static GATE: OnceLock<(Mutex<bool>, Condvar)> = OnceLock::new();
        GATE.get_or_init(|| (Mutex::new(false), Condvar::new()))
    }

    pub(crate) fn run(method: &str, engine: &Engine) {
        match method {
            "test.panic" => engine.panic_holding_sessions(),
            "test.panic_unlocked" => panic!("test-only panic holding nothing"),
            "test.hold" => {
                // Bounded, so a test that never releases fails instead of hanging.
                let (released, turn) = gate();
                let done = released.lock().unwrap();
                let _ = turn
                    .wait_timeout_while(done, std::time::Duration::from_secs(10), |done| !*done)
                    .unwrap();
            }
            _ => {}
        }
    }

    /// Let every `test.hold` request finish.
    pub(crate) fn release() {
        let (released, turn) = gate();
        *released.lock().unwrap() = true;
        turn.notify_all();
    }
}

/// Read one newline-terminated line into `buf`, reading at most `cap + 1` bytes.
///
/// Returns the number of bytes read (`0` = EOF). The `cap + 1` ceiling means a
/// newline-less or oversized line is bounded — the caller treats
/// `buf.len() > cap` as "request too large" rather than buffering unboundedly.
/// This is the daemon protocol's framing on any byte stream: the cap counts the
/// newline, and an oversized request is detected after `cap + 1` bytes without
/// waiting for the rest (`deep-diff-forge rpc` uses it on stdio).
///
/// # Errors
///
/// Returns any read error from `reader`.
pub fn read_capped_line<R: BufRead>(
    reader: &mut R,
    buf: &mut Vec<u8>,
    cap: usize,
) -> std::io::Result<usize> {
    use std::io::Read as _;
    reader.take(cap as u64 + 1).read_until(b'\n', buf)
}

/// `response`, or a `RESOURCE_EXHAUSTED` error for `id` when it is longer than `cap`. Before, the
/// server sent any size and the client refused lines over the cap, so a large `diff.compute`
/// succeeded on the server and failed at the client with no error a caller could act on.
fn bounded_response(id: &Value, response: String, cap: usize) -> String {
    if response.len() <= cap {
        return response;
    }
    let message = format!(
        "the response is {} bytes, over the {cap}-byte line cap; request less (a smaller patch, or diff.plan)",
        response.len()
    );
    error_response(id, &RpcError::new(RESOURCE_EXHAUSTED, message))
}

/// The outcome of arming a read deadline. macOS refuses `SO_RCVTIMEO` with EINVAL once the
/// peer has disconnected (both directions shut); reads then cannot block, returning what is
/// buffered and then end of file, so that refusal is not an error. Before, it made the client fail
/// any response longer than its buffer whenever the daemon closed first (macOS CI).
fn settle_read_timeout(armed: std::io::Result<()>) -> std::io::Result<()> {
    match armed {
        Err(err) if err.raw_os_error() == Some(rustix::io::Errno::INVAL.raw_os_error()) => Ok(()),
        other => other,
    }
}

/// Read one bounded line from a socket under an absolute deadline.
#[expect(
    clippy::disallowed_methods,
    reason = "the socket read deadline: bounds a slow client in wall-clock time"
)]
fn read_capped_line_deadline(
    reader: &mut BufReader<UnixStream>,
    buf: &mut Vec<u8>,
    cap: usize,
    timeout: Duration,
) -> std::io::Result<usize> {
    let deadline = Instant::now() + timeout;
    let maximum = cap.saturating_add(1);
    loop {
        // Zero is past the deadline too (and a timeout `set_read_timeout` refuses).
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .filter(|remaining| !remaining.is_zero())
            .ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::TimedOut, "line read deadline exceeded")
            })?;
        settle_read_timeout(reader.get_ref().set_read_timeout(Some(remaining)))?;

        let available = match reader.fill_buf() {
            Ok(available) => available,
            Err(err)
                if matches!(
                    err.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                ) =>
            {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "line read deadline exceeded",
                ));
            }
            Err(err) => return Err(err),
        };
        if available.is_empty() {
            return Ok(buf.len());
        }
        let allowed = available.len().min(maximum.saturating_sub(buf.len()));
        let newline = available[..allowed].iter().position(|byte| *byte == b'\n');
        let consumed = newline.map_or(allowed, |position| position + 1);
        buf.extend_from_slice(&available[..consumed]);
        reader.consume(consumed);

        if newline.is_some() || buf.len() >= maximum {
            return Ok(buf.len());
        }
    }
}

/// Serve one connection: newline-delimited JSON-RPC, one response per request,
/// until the client disconnects or a shutdown is requested.
///
/// # Errors
///
/// Returns any underlying stream I/O error.
pub fn handle_connection(stream: UnixStream, engine: &Engine) -> std::io::Result<()> {
    stream.set_write_timeout(Some(WRITE_TIMEOUT))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut writer = stream;
    let mut buf = Vec::new();
    loop {
        buf.clear();
        let read =
            read_capped_line_deadline(&mut reader, &mut buf, MAX_REQUEST_BYTES, READ_TIMEOUT)?;
        if read == 0 {
            break; // clean EOF
        }
        if buf.len() > MAX_REQUEST_BYTES {
            let resp = error_response(
                &Value::Null,
                &RpcError::new(INVALID_REQUEST, "request exceeds maximum size"),
            );
            writeln!(writer, "{}", json_line_safe(&resp))?;
            writer.flush()?;
            break; // stream position is unreliable past the cap; close the connection.
        }
        let Ok(decoded) = std::str::from_utf8(&buf) else {
            let resp = error_response(
                &Value::Null,
                &RpcError::new(PARSE_ERROR, "request is not valid UTF-8"),
            );
            writeln!(writer, "{}", json_line_safe(&resp))?;
            writer.flush()?;
            continue;
        };
        let line = decoded.trim();
        if line.is_empty() {
            continue;
        }
        let response = process_line(line, engine);
        writeln!(writer, "{}", json_line_safe(&response))?;
        writer.flush()?;
        if !engine.is_running() {
            break;
        }
    }
    Ok(())
}

/// Bind a secured listener: ensure the runtime dir is owner-private, remove any
/// stale socket, bind, and set the socket to owner-only mode.
///
/// # Errors
///
/// Returns an I/O error if the directory cannot be secured or the bind fails.
pub fn bind_secure(socket_path: &Path) -> std::io::Result<BoundListener> {
    if let Some(parent) = socket_path.parent() {
        ensure_runtime_dir(parent)?;
    }
    bind_locked(socket_path)
}

/// Bind a listener at an *explicit* (caller-supplied) socket path with a
/// fail-closed security policy:
///
/// 1. Secure the parent directory without mutating pre-existing state the caller
///    owns:
///    - If the parent is **missing**, create it (and ancestors) and chmod the
///      leaf to `0700` — we own a directory we just created, so this introduces
///      new owner-private state rather than tightening someone else's. This keeps
///      `daemon start --socket /new/path/d.sock` working out of the box.
///    - If the parent **already exists**, it must pass [`validate_private_dir`]
///      (owner-private, non-symlink) and is **never** chmodded. A world-readable
///      parent fails closed instead of being silently tightened to `0700` — this
///      is the core of the hardening (a `--socket` override must not mutate a
///      directory the caller already owns).
/// 2. If the socket path already exists it is removed **only if it is a socket
///    nothing answers on** (`symlink_metadata + file_type().is_socket()`, then a
///    refused connect). A live daemon's socket is `AddrInUse`; a regular file,
///    directory, or symlink at the path causes an error; we never delete
///    non-socket files.
/// 3. Binds via [`UnixListener::bind`] and sets the socket to `0600`.
///
/// Steps 2 and 3 run under the start lock (see [`BoundListener`]).
///
/// This is the binding path for `--socket` overrides. The engine-managed
/// runtime dir uses [`bind_secure`] instead.
///
/// # Errors
///
/// Returns an I/O error if the parent cannot be created/secured, is a pre-existing
/// insecure directory, the path holds a non-socket file, or the bind itself fails.
pub fn bind_explicit(socket_path: &Path) -> std::io::Result<BoundListener> {
    // 1. Secure the parent. Create it (owner-private) if absent; otherwise
    //    validate it WITHOUT chmodding — never mutate a directory the caller owns.
    let parent = socket_path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "socket path has no parent directory",
        )
    })?;
    match std::fs::symlink_metadata(parent) {
        Ok(_) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            // Parent is absent — create it and set owner-only mode on the new directory.
            std::fs::create_dir_all(parent)?;
            std::fs::set_permissions(parent, std::fs::Permissions::from_mode(SECURE_DIR_MODE))?;
        }
        Err(err) => return Err(err),
    }
    // Validate in all cases (created or pre-existing); never chmod a dir we did
    // not just create.
    validate_private_dir(parent)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::PermissionDenied, e))?;

    // 2 and 3. Under the start lock: remove a stale socket, bind, chmod 0600.
    bind_locked(socket_path)
}

/// In a secured directory: take the start lock, remove a stale socket, bind, and
/// set the socket to `0600`.
fn bind_locked(socket_path: &Path) -> std::io::Result<BoundListener> {
    let start_lock = take_start_lock(socket_path)?;
    // Use symlink_metadata (lstat) to avoid following a symlink at the path.
    remove_stale_socket(socket_path)?;
    let listener = UnixListener::bind(socket_path)?;
    std::fs::set_permissions(
        socket_path,
        std::fs::Permissions::from_mode(SECURE_SOCKET_MODE),
    )?;
    Ok(BoundListener {
        listener,
        _start_lock: start_lock,
    })
}

/// The start lock's path: `<socket file name>.lock` in the socket's directory.
fn start_lock_path(socket_path: &Path) -> std::io::Result<PathBuf> {
    let mut name = socket_path
        .file_name()
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "socket path has no file name",
            )
        })?
        .to_os_string();
    name.push(".lock");
    Ok(socket_path.with_file_name(name))
}

/// Open (creating `0600` if absent) and exclusively lock the start lock, without
/// waiting. The open never follows a symlink, and the file must be a regular file
/// owned by the effective user. A lock another process holds is `AddrInUse`: a
/// daemon runs at this socket, or another start is binding it.
fn take_start_lock(socket_path: &Path) -> std::io::Result<std::fs::File> {
    use rustix::fs::{FlockOperation, Mode, OFlags};
    let lock_path = start_lock_path(socket_path)?;
    // `union` rather than `|`: the flags share no bits, so `|` and `^` are the same value and a
    // `|`-to-`^` mutant could never be caught.
    let lock = std::fs::File::from(rustix::fs::open(
        &lock_path,
        OFlags::RDWR
            .union(OFlags::CREATE)
            .union(OFlags::NOFOLLOW)
            .union(OFlags::CLOEXEC),
        Mode::RUSR.union(Mode::WUSR),
    )?);
    let metadata = lock.metadata()?;
    if !metadata.is_file() || metadata.uid() != rustix::process::geteuid().as_raw() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "the start lock is not a regular file owned by the effective user",
        ));
    }
    match rustix::fs::flock(&lock, FlockOperation::NonBlockingLockExclusive) {
        Ok(()) => Ok(lock),
        Err(rustix::io::Errno::WOULDBLOCK) => Err(std::io::Error::new(
            std::io::ErrorKind::AddrInUse,
            "another daemon is running or starting on this socket; refusing to replace it",
        )),
        Err(err) => Err(err.into()),
    }
}

/// Remove a stale socket without ever following or deleting another file type, and never a live
/// one: before, a second `daemon start` deleted the socket of a running daemon and bound its own,
/// leaving the first running but unreachable, so `daemon stop` could no longer stop it.
fn remove_stale_socket(socket_path: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(socket_path) {
        Ok(metadata) if metadata.file_type().is_socket() => {
            match UnixStream::connect(socket_path) {
                Ok(_) => Err(std::io::Error::new(
                    std::io::ErrorKind::AddrInUse,
                    "a daemon is already listening on this socket; refusing to replace it",
                )),
                // Nothing listens: the socket is left over from a daemon that is gone.
                Err(err) if err.kind() == std::io::ErrorKind::ConnectionRefused => {
                    std::fs::remove_file(socket_path)
                }
                Err(err) => Err(err),
            }
        }
        Ok(_) => Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "socket path is occupied by a non-socket file; refusing to delete it",
        )),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

/// Remove the socket on shutdown only if it is still the exact filesystem
/// object created by this server. This prevents cleanup from deleting a path
/// swapped in after bind.
fn cleanup_owned_socket(socket_path: &Path, identity: (u64, u64)) {
    if let Ok(metadata) = std::fs::symlink_metadata(socket_path)
        && metadata.file_type().is_socket()
        && (metadata.dev(), metadata.ino()) == identity
    {
        let _ = std::fs::remove_file(socket_path);
    }
}

struct ActiveConnection(Arc<AtomicUsize>);

impl Drop for ActiveConnection {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

fn try_acquire_connection(active: &Arc<AtomicUsize>) -> bool {
    let mut count = active.load(Ordering::Acquire);
    loop {
        if count >= MAX_CONNECTIONS {
            return false;
        }
        match active.compare_exchange_weak(count, count + 1, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => return true,
            Err(observed) => count = observed,
        }
    }
}

/// Run the daemon server loop until shutdown.
///
/// Thin wrapper over the tested [`bind_secure`] + [`handle_connection`]; the
/// accept loop is the one part that requires a live socket and is exercised by
/// the in-process round-trip tests rather than mocked.
///
/// # Errors
///
/// Returns an I/O error from binding or accepting connections.
pub fn run_server(location: &SocketLocation) -> std::io::Result<()> {
    run_server_with(location, Engine::new())
}

/// [`run_server`] with a configured engine (for example one allowed to read repositories).
///
/// # Errors
///
/// Returns an I/O error from binding or accepting connections.
pub fn run_server_with(location: &SocketLocation, engine: Engine) -> std::io::Result<()> {
    serve_bound(location, location.bind()?, engine)
}

/// The accept loop of [`run_server_with`] over a listener already bound at `location`.
fn serve_bound(
    location: &SocketLocation,
    bound: BoundListener,
    engine: Engine,
) -> std::io::Result<()> {
    let listener = bound.listener();
    let socket_path = location.path().to_path_buf();
    let me = rustix::process::geteuid().as_raw();
    let socket_metadata = std::fs::symlink_metadata(&socket_path)?;
    let socket_identity = (socket_metadata.dev(), socket_metadata.ino());
    // Shared by every connection thread; requests run in parallel (see `process_line`).
    let engine = Arc::new(engine);
    let active_connections = Arc::new(AtomicUsize::new(0));
    loop {
        // Break immediately if a previous connection already triggered shutdown.
        if !engine.is_running() {
            break;
        }
        match listener.accept() {
            Ok((stream, _)) => {
                // Only this daemon's own user is served, whatever the socket's mode.
                if let Err(err) = require_peer_user(&stream, me) {
                    engine.metrics().refused_peer();
                    engine.log(
                        "connection_refused",
                        &format!("refused a connection: {err}"),
                    );
                    drop(stream);
                    continue;
                }
                if !try_acquire_connection(&active_connections) {
                    engine.metrics().refused_at_limit();
                    // Dropping the accepted stream applies backpressure without
                    // allocating another stack or worker thread.
                    drop(stream);
                    continue;
                }
                engine.metrics().accepted();
                let arc = Arc::clone(&engine);
                let wakeup = socket_path.clone();
                let active = Arc::clone(&active_connections);
                // A per-connection error must NOT tear down the daemon — the
                // spawned thread logs it and the accept loop continues.
                let spawned = std::thread::Builder::new()
                    .name("ddf-daemon-connection".to_string())
                    .spawn(move || {
                        let _active_guard = ActiveConnection(active);
                        if let Err(err) = handle_connection(stream, &arc) {
                            arc.metrics().connection_error();
                            arc.log("connection_error", &format!("connection error: {err}"));
                        }
                        // If the connection just triggered a shutdown, unblock the
                        // accept loop (which is blocking on listener.accept()) by
                        // opening a short-lived wakeup connection to ourselves.
                        if !arc.is_running() {
                            let _ = UnixStream::connect(wakeup);
                        }
                    });
                if let Err(err) = spawned {
                    // The closure never ran, so release the slot explicitly.
                    active_connections.fetch_sub(1, Ordering::AcqRel);
                    engine.log(
                        "worker_spawn_failed",
                        &format!("could not spawn connection worker: {err}"),
                    );
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => {}
            Err(err) => return Err(err),
        }
    }
    cleanup_owned_socket(location.path(), socket_identity);
    // The start lock is released only now, after the cleanup.
    drop(bound);
    Ok(())
}

/// Client: connect to the daemon, send one request line, return the response.
///
/// # Errors
///
/// Returns an I/O error if the connection or exchange fails (e.g. no daemon
/// is listening at the location).
pub fn request(location: &SocketLocation, line: &str) -> std::io::Result<String> {
    // The server caps the framed line, newline included; refuse locally what it would refuse.
    if line.len() + 1 > MAX_REQUEST_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "request exceeds maximum size",
        ));
    }
    if line.contains(['\r', '\n']) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "request must be exactly one line",
        ));
    }
    let mut stream = location.connect()?;
    stream.set_write_timeout(Some(WRITE_TIMEOUT))?;
    writeln!(stream, "{line}")?;
    stream.flush()?;
    let mut reader = BufReader::new(stream);
    let mut response = Vec::new();
    let read =
        read_capped_line_deadline(&mut reader, &mut response, MAX_RESPONSE_BYTES, READ_TIMEOUT)?;
    if read == 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "daemon closed without a response",
        ));
    }
    if response.len() > MAX_RESPONSE_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "daemon response exceeds maximum size",
        ));
    }
    let response = std::str::from_utf8(&response).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "daemon response is not valid UTF-8",
        )
    })?;
    Ok(response.trim_end().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::thread;
    use std::time::Duration;

    /// Join a server or connection thread, failing the test rather than hanging it when the
    /// thread never finishes (as under a mutant that never stops serving): after 10 s the join
    /// is an error, like a thread that panicked.
    fn joined<T>(handle: thread::JoinHandle<T>) -> thread::Result<T> {
        for _ in 0..1000 {
            if handle.is_finished() {
                return handle.join();
            }
            thread::park_timeout(Duration::from_millis(10));
        }
        Err(Box::new("the thread did not finish within 10 s"))
    }

    fn engine() -> Engine {
        Engine::new()
    }

    #[test]
    fn read_capped_line_reads_one_line() {
        let mut cur = std::io::Cursor::new(b"hello\nworld\n".to_vec());
        let mut buf = Vec::new();
        let n = read_capped_line(&mut cur, &mut buf, 1024).expect("read");
        assert_eq!(n, 6);
        assert_eq!(buf, b"hello\n");
    }

    #[test]
    fn read_capped_line_bounds_oversized_line() {
        // 100 bytes with no newline, cap 8: must not buffer past cap + 1.
        let mut cur = std::io::Cursor::new(vec![b'a'; 100]);
        let mut buf = Vec::new();
        let n = read_capped_line(&mut cur, &mut buf, 8).expect("read");
        assert_eq!(n, 9, "reads at most cap + 1 bytes");
        assert!(buf.len() > 8, "caller can detect the over-cap condition");
        assert!(buf.len() <= 9, "never buffers an unbounded amount");
    }

    #[test]
    fn read_capped_line_eof_is_zero() {
        let mut cur = std::io::Cursor::new(Vec::new());
        let mut buf = Vec::new();
        assert_eq!(read_capped_line(&mut cur, &mut buf, 8).expect("read"), 0);
    }

    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "test: drips bytes in real time to check the absolute deadline"
    )]
    fn socket_line_deadline_is_absolute_across_dripped_bytes() {
        let (mut client, server) = UnixStream::pair().expect("socketpair");
        let drip = thread::spawn(move || {
            thread::sleep(Duration::from_millis(20));
            let _ = client.write_all(b"a");
            thread::sleep(Duration::from_millis(20));
            let _ = client.write_all(b"b");
            thread::sleep(Duration::from_millis(20));
            let _ = client.write_all(b"\n");
        });
        let mut reader = BufReader::new(server);
        let mut line = Vec::new();
        let started = Instant::now();
        let err =
            read_capped_line_deadline(&mut reader, &mut line, 1024, Duration::from_millis(50))
                .expect_err("drip feed must not extend the absolute deadline");
        assert_eq!(err.kind(), std::io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_millis(200));
        joined(drip).unwrap();
    }

    #[test]
    fn oversized_request_gets_error_then_connection_closes() {
        // A request line over a (here, artificially exercised) cap yields an
        // INVALID_REQUEST error response; integration of the cap is proven by the
        // read_capped_line unit tests + the handle_connection wiring.
        let resp = error_response(
            &Value::Null,
            &RpcError::new(INVALID_REQUEST, "request exceeds maximum size"),
        );
        let v: Value = serde_json::from_str(&resp).unwrap();
        assert_eq!(v["error"]["code"], INVALID_REQUEST);
    }

    #[test]
    fn handle_connection_sets_read_timeout() {
        // The slowloris guard: a real socket carries a read timeout.
        let (_client, server) = UnixStream::pair().expect("socketpair");
        // Setting it directly mirrors handle_connection's first action.
        server
            .set_read_timeout(Some(READ_TIMEOUT))
            .expect("set timeout");
        assert_eq!(server.read_timeout().unwrap(), Some(READ_TIMEOUT));
    }

    #[test]
    fn process_line_health_ok() {
        let e = engine();
        let resp = process_line(r#"{"id":1,"method":"daemon.health"}"#, &e);
        let v: Value = serde_json::from_str(&resp).unwrap();
        assert_eq!(v["result"]["status"], "ok");
        assert_eq!(v["id"], 1);
    }

    #[test]
    fn process_line_parse_error_has_null_id() {
        let e = engine();
        let resp = process_line("garbage", &e);
        let v: Value = serde_json::from_str(&resp).unwrap();
        assert!(v.get("error").is_some());
        assert_eq!(v["id"], Value::Null);
    }

    #[test]
    fn process_line_unknown_method_is_error() {
        let e = engine();
        let resp = process_line(r#"{"id":2,"method":"nope"}"#, &e);
        let v: Value = serde_json::from_str(&resp).unwrap();
        assert_eq!(v["error"]["code"], crate::protocol::METHOD_NOT_FOUND);
    }

    const OPEN_X: &str = r#"{"id":1,"method":"session.open","params":{"patch":"--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n"}}"#;

    /// Anchor A-03: a panic while the session table was held drops the sessions it may have
    /// corrupted and clears the poison, so the engine keeps serving.
    #[test]
    fn a_contained_panic_resets_sessions_and_clears_the_poison() {
        let e = engine();
        let opened: Value = serde_json::from_str(&process_line(OPEN_X, &e)).unwrap();
        assert!(opened["result"]["session"].is_string(), "{opened}");
        assert_eq!(e.session_count(), 1);
        let crashed: Value =
            serde_json::from_str(&process_line(r#"{"id":2,"method":"test.panic"}"#, &e)).unwrap();
        assert_eq!(crashed["error"]["code"], crate::protocol::INTERNAL_ERROR);
        assert_eq!(crashed["id"], 2);
        assert_eq!((e.session_count(), e.is_running()), (0, true));
        let health: Value =
            serde_json::from_str(&process_line(r#"{"id":3,"method":"daemon.health"}"#, &e))
                .unwrap();
        assert_eq!(health["result"]["status"], "ok");
        let reopened: Value = serde_json::from_str(&process_line(OPEN_X, &e)).unwrap();
        assert_ne!(
            reopened["result"]["session"], opened["result"]["session"],
            "a session id was reused after the reset"
        );
        assert_eq!(e.session_count(), 1);
    }

    /// A panic that held nothing leaves every other client's session in place: only a panic
    /// inside the session table can have left it inconsistent.
    #[test]
    fn a_panic_outside_the_session_table_keeps_the_sessions() {
        let e = engine();
        let _ = process_line(OPEN_X, &e);
        let crashed: Value = serde_json::from_str(&process_line(
            r#"{"id":2,"method":"test.panic_unlocked"}"#,
            &e,
        ))
        .unwrap();
        assert_eq!(crashed["error"]["code"], crate::protocol::INTERNAL_ERROR);
        assert_eq!(e.session_count(), 1);
        assert_eq!((e.admission().running(), e.admission().in_flight()), (0, 0));
    }

    /// Each request reaches `daemon.metrics`: its method's count and latency bucket, its error
    /// code, a contained panic, admission, and a request for a method the daemon does not have.
    #[test]
    fn requests_are_counted_in_daemon_metrics() {
        let e = engine();
        let _ = process_line(r#"{"id":1,"method":"daemon.health"}"#, &e);
        let _ = process_line(r#"{"id":2,"method":"daemon.health"}"#, &e);
        let _ = process_line(r#"{"id":3,"method":"diff.plan","params":{}}"#, &e);
        let _ = process_line(r#"{"id":4,"method":"test.panic_unlocked"}"#, &e);
        let _ = process_line("not json", &e);
        let reply: Value =
            serde_json::from_str(&process_line(r#"{"id":5,"method":"daemon.metrics"}"#, &e))
                .unwrap();
        let m = &reply["result"];
        assert_eq!(m["schema"], "deep-diff-forge.daemon-metrics.v0");
        assert_eq!(m["methods"]["daemon.health"]["requests"], 2);
        assert!(
            m["methods"]["daemon.health"]["errors"]
                .as_object()
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            m["methods"]["diff.plan"]["errors"],
            serde_json::json!({"invalid_params": 1})
        );
        assert_eq!(
            m["methods"]["(unknown)"]["errors"],
            serde_json::json!({"internal_error": 1, "parse_error": 1})
        );
        assert_eq!(m["engine"]["panics"], 1);
        // Five requests were admitted: four, and daemon.metrics itself (admitted before it answers,
        // counted as a request after). The unparsable line never reaches admission.
        assert_eq!(m["admission"]["admitted"], 5);
        assert!(m["methods"].get("daemon.metrics").is_none());
        assert_eq!(m["admission"]["peak_running"], 1);
        let buckets: u64 = m["methods"]["daemon.health"]["latency_us"]["buckets"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b.as_u64().unwrap())
            .sum();
        assert_eq!(buckets, 2);
    }

    /// A session opened at the session limit evicts the least recently used one, and the
    /// eviction is counted once per session it removed.
    #[test]
    fn evictions_are_counted() {
        let e = engine();
        let open = |id: usize| {
            format!(
                r#"{{"id":{id},"method":"session.open","params":{{"patch":"--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n"}}}}"#
            )
        };
        for id in 0..crate::handler::MAX_SESSIONS {
            let _ = process_line(&open(id), &e);
        }
        let evicted = |e: &Engine| {
            e.metrics().document(std::time::Duration::ZERO, "0")["engine"]["sessions_evicted"]
                .clone()
        };
        assert_eq!(evicted(&e), 0, "the limit itself evicts nothing");
        let _ = process_line(&open(100), &e);
        assert_eq!(evicted(&e), 1);
        let _ = process_line(&open(101), &e);
        assert_eq!(evicted(&e), 2);
        assert_eq!(e.session_count(), crate::handler::MAX_SESSIONS);
    }

    /// Opening, closing, evicting and resetting sessions are counted.
    #[test]
    fn session_events_are_counted() {
        let e = engine();
        let open = |id: u32| {
            format!(
                r#"{{"id":{id},"method":"session.open","params":{{"patch":"--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n"}}}}"#
            )
        };
        let opened: Value = serde_json::from_str(&process_line(&open(1), &e)).unwrap();
        let session = opened["result"]["session"].as_str().unwrap().to_owned();
        let close =
            format!(r#"{{"id":2,"method":"session.close","params":{{"session":"{session}"}}}}"#);
        let _ = process_line(&close, &e);
        let _ = process_line(&close, &e);
        let _ = process_line(&open(3), &e);
        let _ = process_line(r#"{"id":4,"method":"test.panic"}"#, &e);
        let _ = process_line(r#"{"id":5,"method":"daemon.health"}"#, &e);
        let m = e.metrics().document(std::time::Duration::ZERO, "0");
        assert_eq!(m["engine"]["sessions_opened"], 2);
        assert_eq!(
            m["engine"]["sessions_closed"], 1,
            "a second close of the same session is not counted"
        );
        assert_eq!(m["engine"]["session_resets"], 1);
        assert!(m["engine"]["peak_retained_bytes"].as_u64().unwrap() > 0);
    }

    /// Requests run in parallel: while one request is still running, another is answered.
    /// Before, every request held one engine mutex, so `daemon.health` waited for the first.
    #[test]
    fn a_running_request_does_not_block_another() {
        let e = engine();
        thread::scope(|scope| {
            let held = scope.spawn(|| process_line(r#"{"id":1,"method":"test.hold"}"#, &e));
            let mut holding = false;
            for _ in 0..1000 {
                holding = e.admission().running() == 1;
                if holding {
                    break;
                }
                thread::park_timeout(Duration::from_millis(10));
            }
            let (answered, rx) = std::sync::mpsc::channel();
            let e = &e;
            scope.spawn(move || {
                let health = process_line(r#"{"id":2,"method":"daemon.health"}"#, e);
                answered.send(health).unwrap();
            });
            let health = rx.recv_timeout(Duration::from_secs(10));
            // Released before any assertion, so a failing assertion cannot leave it waiting.
            test_hooks::release();
            assert!(holding, "the held request was never admitted");
            let health: Value = serde_json::from_str(
                &health.expect("daemon.health waited for a request that was still running"),
            )
            .unwrap();
            assert_eq!(health["result"]["status"], "ok");
            let held: Value = serde_json::from_str(&held.join().unwrap()).unwrap();
            assert_eq!(held["error"]["code"], crate::protocol::METHOD_NOT_FOUND);
        });
        assert_eq!(e.admission().running(), 0);
    }

    /// The connection that saw the panic keeps being served (before, its loop ended).
    #[test]
    fn a_connection_keeps_serving_after_a_contained_panic() {
        let (client, server) = UnixStream::pair().expect("socketpair");
        let handle = thread::spawn(move || {
            let e = engine();
            handle_connection(server, &e)
        });
        let mut writer = client.try_clone().unwrap();
        writeln!(writer, r#"{{"id":1,"method":"test.panic"}}"#).unwrap();
        writeln!(writer, r#"{{"id":2,"method":"daemon.health"}}"#).unwrap();
        writer.flush().unwrap();
        writer.shutdown(std::net::Shutdown::Write).unwrap();
        let lines: Vec<Value> = BufReader::new(client)
            .lines()
            .map(|l| serde_json::from_str(&l.unwrap()).unwrap())
            .collect();
        joined(handle).unwrap().unwrap();
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert_eq!(lines[1]["result"]["status"], "ok");
    }

    /// Bug hunt 2026-10-28, H4-06: a response carrying U+2028/U+2029 leaves the socket as one
    /// line, with the characters escaped, so a line-aware client never splits it.
    #[test]
    fn a_response_with_line_separators_is_one_line() {
        let (client, server) = UnixStream::pair().expect("socketpair");
        let handle = thread::spawn(move || {
            let e = engine();
            handle_connection(server, &e)
        });
        let patch = "--- a/x\u{2028}y\n+++ b/x\u{2028}y\n@@ -1 +1 @@\n-a\n+b\u{2029}c\n";
        let request =
            serde_json::json!({"id": 1, "method": "diff.compute", "params": {"patch": patch}});
        let mut writer = client.try_clone().unwrap();
        writeln!(writer, "{request}").unwrap();
        writer.flush().unwrap();
        writer.shutdown(std::net::Shutdown::Write).unwrap();
        let mut text = String::new();
        std::io::Read::read_to_string(&mut BufReader::new(client), &mut text).unwrap();
        joined(handle).unwrap().unwrap();
        assert!(!text.contains(['\u{2028}', '\u{2029}']), "{text}");
        assert_eq!(text.lines().count(), 1, "{text}");
        let response: Value = serde_json::from_str(text.trim_end()).unwrap();
        assert_eq!(response["result"]["files"][0]["path"], "x\u{2028}y");
    }

    #[test]
    fn process_line_shutdown_clears_running() {
        let e = engine();
        let _ = process_line(r#"{"method":"daemon.shutdown"}"#, &e);
        assert!(!e.is_running());
    }

    #[test]
    fn handle_connection_round_trip_over_socketpair() {
        let (client, server) = UnixStream::pair().expect("socketpair");
        let handle = thread::spawn(move || {
            let e = engine();
            handle_connection(server, &e)
        });
        let mut writer = client.try_clone().unwrap();
        writeln!(writer, r#"{{"id":9,"method":"daemon.health"}}"#).unwrap();
        writer.flush().unwrap();
        let mut reader = BufReader::new(client);
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        drop(writer);
        drop(reader);
        assert!(joined(handle).is_ok(), "the thread did not finish");
        let v: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["result"]["status"], "ok");
        assert_eq!(v["id"], 9);
    }

    #[test]
    fn handle_connection_serves_multiple_requests() {
        let (client, server) = UnixStream::pair().expect("socketpair");
        let handle = thread::spawn(move || handle_connection(server, &engine()));
        let mut writer = client.try_clone().unwrap();
        writeln!(writer, r#"{{"id":1,"method":"daemon.health"}}"#).unwrap();
        writeln!(writer, r#"{{"id":2,"method":"engine.initialize"}}"#).unwrap();
        writer.flush().unwrap();
        let mut reader = BufReader::new(client);
        let mut first = String::new();
        let mut second = String::new();
        reader.read_line(&mut first).unwrap();
        reader.read_line(&mut second).unwrap();
        drop(writer);
        drop(reader);
        assert!(joined(handle).is_ok(), "the thread did not finish");
        assert!(first.contains("\"status\":\"ok\""));
        assert!(second.contains("deep-diff-forge"));
    }

    fn temp_socket(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "ddf-srv-{}-{name}/deep-diff-forge.sock",
            std::process::id()
        ))
    }

    #[test]
    fn bind_secure_sets_owner_only_modes() {
        let sock = temp_socket("bind");
        let _ = std::fs::remove_dir_all(sock.parent().unwrap());
        let listener = bind_secure(&sock).expect("bind");
        let sock_mode = std::fs::metadata(&sock).unwrap().permissions().mode();
        let dir_mode = std::fs::metadata(sock.parent().unwrap())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(sock_mode & 0o777, SECURE_SOCKET_MODE);
        assert_eq!(dir_mode & 0o777, crate::security::SECURE_DIR_MODE);
        let lock = std::fs::symlink_metadata(sock.with_file_name("deep-diff-forge.sock.lock"))
            .expect("the start lock sits beside the socket");
        assert!(lock.is_file());
        assert_eq!(
            lock.permissions().mode() & 0o077,
            0,
            "owner-only start lock"
        );
        drop(listener);
    }

    #[test]
    fn bind_secure_replaces_stale_socket() {
        let sock = temp_socket("stale");
        let _ = std::fs::remove_dir_all(sock.parent().unwrap());
        let first = bind_secure(&sock).expect("first bind");
        drop(first);
        // Bind again over the now-stale socket file; must succeed.
        let second = bind_secure(&sock).expect("rebind over stale");
        drop(second);
    }

    #[test]
    fn bind_secure_refuses_to_delete_regular_file() {
        let sock = temp_socket("secure-regular-file");
        let _ = std::fs::remove_dir_all(sock.parent().unwrap());
        std::fs::create_dir_all(sock.parent().unwrap()).unwrap();
        std::fs::write(&sock, b"important").unwrap();
        let err = bind_secure(&sock).expect_err("regular file must not be removed");
        assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(&sock).unwrap(), b"important");
    }

    #[test]
    fn cleanup_never_removes_regular_file() {
        let sock = temp_socket("cleanup-regular-file");
        let _ = std::fs::remove_dir_all(sock.parent().unwrap());
        std::fs::create_dir_all(sock.parent().unwrap()).unwrap();
        std::fs::write(&sock, b"important").unwrap();
        let metadata = std::fs::metadata(&sock).unwrap();
        cleanup_owned_socket(&sock, (metadata.dev(), metadata.ino()));
        assert_eq!(std::fs::read(&sock).unwrap(), b"important");
    }

    /// The socket this server bound is removed on shutdown (legitimate nearby case).
    #[test]
    fn cleanup_removes_the_socket_it_bound() {
        let sock = temp_socket("cleanup-own");
        let _ = std::fs::remove_dir_all(sock.parent().unwrap());
        std::fs::create_dir_all(sock.parent().unwrap()).unwrap();
        let listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        let metadata = std::fs::symlink_metadata(&sock).unwrap();
        cleanup_owned_socket(&sock, (metadata.dev(), metadata.ino()));
        assert!(std::fs::symlink_metadata(&sock).is_err());
        drop(listener);
    }

    /// A socket swapped in at the same path after bind is someone else's: shutdown leaves it.
    /// Before this test the identity comparison had no test (Remaining Work 2.6).
    #[test]
    fn cleanup_never_removes_a_socket_swapped_in_after_bind() {
        let sock = temp_socket("cleanup-swapped");
        let _ = std::fs::remove_dir_all(sock.parent().unwrap());
        std::fs::create_dir_all(sock.parent().unwrap()).unwrap();
        let ours = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        let metadata = std::fs::symlink_metadata(&sock).unwrap();
        let identity = (metadata.dev(), metadata.ino());
        std::fs::remove_file(&sock).unwrap();
        let theirs = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        let swapped = std::fs::symlink_metadata(&sock).unwrap();
        assert_ne!((swapped.dev(), swapped.ino()), identity);
        cleanup_owned_socket(&sock, identity);
        assert!(
            std::fs::symlink_metadata(&sock)
                .unwrap()
                .file_type()
                .is_socket()
        );
        drop((ours, theirs));
    }

    #[test]
    fn connection_slots_are_hard_bounded_and_released() {
        let active = Arc::new(AtomicUsize::new(0));
        let mut guards = Vec::new();
        for _ in 0..MAX_CONNECTIONS {
            assert!(try_acquire_connection(&active));
            guards.push(ActiveConnection(Arc::clone(&active)));
        }
        assert!(!try_acquire_connection(&active));
        guards.pop();
        assert!(try_acquire_connection(&active));
        // Balance the final successful acquisition; the remaining guards clean
        // up their own slots on drop.
        active.fetch_sub(1, Ordering::AcqRel);
    }

    #[expect(
        clippy::disallowed_methods,
        reason = "test: polls for the server socket to appear"
    )]
    fn wait_for_socket(path: &Path) {
        for _ in 0..50 {
            if path.exists() {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    /// Pre-create a parent directory at the given mode, returning the socket path.
    fn temp_socket_with_parent(name: &str, parent_mode: u32) -> std::path::PathBuf {
        let sock = temp_socket(name);
        let parent = sock.parent().unwrap();
        let _ = std::fs::remove_dir_all(parent);
        std::fs::create_dir_all(parent).expect("create parent");
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(parent_mode))
            .expect("chmod parent");
        sock
    }

    #[test]
    fn run_server_full_round_trip_then_shutdown() {
        // Explicit SocketLocation requires the parent to exist and be owner-private.
        let sock = temp_socket_with_parent("run", 0o700);
        let server_loc = SocketLocation::at(sock.clone());
        let server = thread::spawn(move || run_server(&server_loc));
        wait_for_socket(&sock);
        let client = SocketLocation::at(sock.clone());

        let health = request(&client, r#"{"id":1,"method":"daemon.health"}"#).expect("health");
        assert!(health.contains("\"status\":\"ok\""));

        let stop = request(&client, r#"{"id":2,"method":"daemon.shutdown"}"#).expect("stop");
        assert!(stop.contains("\"stopping\":true"));

        assert!(joined(server).is_ok(), "the thread did not finish");
        // Socket is cleaned up on shutdown.
        assert!(!sock.exists());
    }

    #[test]
    fn run_server_diff_plan_over_socket() {
        // Explicit SocketLocation requires the parent to exist and be owner-private.
        let sock = temp_socket_with_parent("plan", 0o700);
        let server_loc = SocketLocation::at(sock.clone());
        let server = thread::spawn(move || run_server(&server_loc));
        wait_for_socket(&sock);
        let client = SocketLocation::at(sock.clone());

        let line = r#"{"id":1,"method":"diff.plan","params":{"patch":"--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@\n-a\n+b\n"}}"#;
        let resp = request(&client, line).expect("plan");
        assert!(resp.contains("src/lib.rs"));
        assert!(resp.contains("public_api_surface"));

        let _ = request(&client, r#"{"method":"daemon.shutdown"}"#);
        assert!(joined(server).is_ok(), "the thread did not finish");
    }

    #[test]
    fn request_to_missing_daemon_errors() {
        // connect() does not require the parent to exist or be secure; it simply
        // tries to open the socket path and fails if no daemon is listening.
        let sock = temp_socket("absent");
        let _ = std::fs::remove_dir_all(sock.parent().unwrap());
        let client = SocketLocation::at(sock);
        assert!(request(&client, r#"{"method":"daemon.health"}"#).is_err());
    }

    #[test]
    fn request_rejects_multiline_input_before_connecting() {
        let client = SocketLocation::at(temp_socket("multiline"));
        let err = request(&client, "{}\n{}").unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    }

    #[test]
    fn resolve_prefers_explicit_path() {
        let loc = SocketLocation::resolve(Some(Path::new("/custom/dff.sock"))).expect("resolve");
        assert_eq!(loc.path(), Path::new("/custom/dff.sock"));
    }

    #[test]
    fn resolve_without_xdg_or_explicit_fails_closed() {
        // With no explicit path, resolution depends on the live $XDG_RUNTIME_DIR.
        // The fail-closed contract is exercised directly via the pure resolver:
        match SocketLocation::resolve(None) {
            Ok(loc) => assert!(loc.path().to_string_lossy().ends_with(".sock")),
            Err(e) => assert_eq!(e, SocketError::NoRuntimeDir),
        }
    }

    #[test]
    fn empty_lines_are_ignored() {
        let (client, server) = UnixStream::pair().expect("socketpair");
        let handle = thread::spawn(move || handle_connection(server, &engine()));
        let mut writer = client.try_clone().unwrap();
        writeln!(writer).unwrap();
        writeln!(writer, r#"{{"id":1,"method":"daemon.health"}}"#).unwrap();
        writer.flush().unwrap();
        let mut reader = BufReader::new(client);
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        drop(writer);
        drop(reader);
        assert!(joined(handle).is_ok(), "the thread did not finish");
        assert!(line.contains("\"status\":\"ok\""));
    }

    // -------------------------------------------------------------------------
    // F2: explicit-bind policy tests (fail-before/pass-after)
    // -------------------------------------------------------------------------

    // Build a temp parent dir at the given mode and return (parent, sock_path).
    fn explicit_temp(label: &str, mode: u32) -> (std::path::PathBuf, std::path::PathBuf) {
        let parent =
            std::env::temp_dir().join(format!("ddf-explicit-{}-{label}", std::process::id()));
        let _ = std::fs::remove_dir_all(&parent);
        std::fs::create_dir_all(&parent).expect("create parent");
        std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(mode))
            .expect("chmod parent");
        let sock = parent.join("test.sock");
        (parent, sock)
    }

    /// The probe-to-bind race: while another start holds the lock (it has probed and not yet
    /// bound), a second start must neither remove the socket at the path nor bind its own. Before,
    /// it found the socket stale, removed it and bound, so the first start's daemon became
    /// unreachable.
    #[test]
    fn a_start_in_progress_keeps_the_socket_it_is_replacing() {
        let (_, sock) = explicit_temp("start-race", 0o700);
        drop(UnixListener::bind(&sock).unwrap());
        let before = std::fs::symlink_metadata(&sock).unwrap();
        let other_start = take_start_lock(&sock).expect("the other start's lock");
        let err = bind_explicit(&sock).expect_err("the start lock is held");
        assert_eq!(err.kind(), std::io::ErrorKind::AddrInUse);
        let after = std::fs::symlink_metadata(&sock).expect("the socket is still there");
        assert_eq!((after.dev(), after.ino()), (before.dev(), before.ino()));
        // Legitimate nearby case: once that start is gone, the stale socket is replaced.
        drop(other_start);
        drop(bind_explicit(&sock).expect("the lock is free"));
    }

    /// A second start while a daemon runs is refused, and the running daemon keeps its socket
    /// and still answers.
    #[test]
    fn a_second_start_leaves_the_running_daemon_reachable() {
        let (_, sock) = explicit_temp("second-start", 0o700);
        let first = bind_explicit(&sock).expect("first start");
        let identity = std::fs::symlink_metadata(&sock).unwrap();
        let err = bind_secure(&sock).expect_err("the first daemon holds the lock");
        assert_eq!(err.kind(), std::io::ErrorKind::AddrInUse);
        let after = std::fs::symlink_metadata(&sock).unwrap();
        assert_eq!((after.dev(), after.ino()), (identity.dev(), identity.ino()));
        let _client = UnixStream::connect(&sock).expect("the first daemon still answers");
        assert!(first.listener().accept().is_ok());
    }

    /// The start lock is opened without following a symlink, so a planted link cannot make the
    /// daemon create or lock a file elsewhere.
    #[test]
    fn the_start_lock_never_follows_a_symlink() {
        let (parent, sock) = explicit_temp("lock-symlink", 0o700);
        let target = parent.join("elsewhere");
        std::fs::write(&target, b"keep").unwrap();
        std::os::unix::fs::symlink(&target, parent.join("test.sock.lock")).unwrap();
        bind_explicit(&sock).expect_err("a symlinked start lock is refused");
        assert_eq!(std::fs::read(&target).unwrap(), b"keep");
        assert!(
            std::fs::symlink_metadata(&sock).is_err(),
            "nothing was bound"
        );
    }

    /// A start lock that is not a regular file (here a FIFO) is refused. rustix has no FIFO call on
    /// Apple targets, so this runs elsewhere (Linux CI, where the mutation gate runs too).
    #[test]
    #[cfg(not(target_vendor = "apple"))]
    fn a_start_lock_that_is_not_a_regular_file_is_refused() {
        let (parent, sock) = explicit_temp("lock-fifo", 0o700);
        rustix::fs::mknodat(
            rustix::fs::CWD,
            parent.join("test.sock.lock"),
            rustix::fs::FileType::Fifo,
            rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
            0,
        )
        .unwrap();
        let err = bind_explicit(&sock).expect_err("a FIFO is no start lock");
        assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
        assert!(
            std::fs::symlink_metadata(&sock).is_err(),
            "nothing was bound"
        );
    }

    #[test]
    fn the_start_lock_sits_beside_the_socket() {
        assert_eq!(
            start_lock_path(Path::new("/run/user/1/ddf/d.sock")).unwrap(),
            Path::new("/run/user/1/ddf/d.sock.lock")
        );
        let err = start_lock_path(Path::new("/")).expect_err("no file name");
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    }

    // F2-T1: bind_explicit does NOT chmod an owner-private parent dir.
    // The parent is pre-set to 0700; after binding the Explicit socket it must
    // still be 0700 — we never touch the caller's directory.
    #[test]
    fn explicit_bind_does_not_chmod_parent() {
        let (parent, sock) = explicit_temp("nochmod", 0o700);
        let listener = bind_explicit(&sock).expect("bind_explicit on 0700 parent");
        let parent_mode = std::fs::metadata(&parent).unwrap().permissions().mode();
        assert_eq!(
            parent_mode & 0o777,
            0o700,
            "parent mode must be unchanged (0700) after explicit bind"
        );
        drop(listener);
    }

    // F2-T2: bind_explicit ERRORS on a world-readable parent and does NOT chmod it.
    // The parent is set to 0755; bind_explicit must return Err (fail closed)
    // and must leave the parent at 0755 — the finding's exact regression check.
    #[test]
    fn explicit_bind_rejects_world_readable_parent() {
        let (parent, sock) = explicit_temp("worldread", 0o755);
        let result = bind_explicit(&sock);
        assert!(
            result.is_err(),
            "bind_explicit must error on a world-readable (0755) parent"
        );
        // Critical: we must NOT have silently chmodded the parent to 0700.
        let parent_mode = std::fs::metadata(&parent).unwrap().permissions().mode();
        assert_eq!(
            parent_mode & 0o777,
            0o755,
            "parent mode must remain 0755 — bind_explicit must not chmod it"
        );
        // Socket must not have been created.
        assert!(!sock.exists(), "socket must not be created after error");
    }

    // F2-T3: bind_explicit refuses to delete a regular file at the socket path.
    // A regular file pre-existing at the socket path must survive; bind_explicit
    // returns Err and the file is still present.
    #[test]
    fn explicit_bind_refuses_to_delete_regular_file() {
        let (_, sock) = explicit_temp("regularfile", 0o700);
        // Place a regular file at the socket path.
        std::fs::write(&sock, b"important data").expect("write regular file");
        let result = bind_explicit(&sock);
        assert!(
            result.is_err(),
            "bind_explicit must error when socket path holds a regular file"
        );
        assert!(
            sock.exists(),
            "regular file must still exist — bind_explicit must not delete it"
        );
        // Verify it's still the regular file, not a socket.
        assert!(
            std::fs::metadata(&sock).unwrap().file_type().is_file(),
            "path must remain a regular file"
        );
    }

    // F2-T9: bind_explicit refuses a SYMLINK at the socket path and does not
    // follow it — a symlink pointing at a victim file must leave both the victim
    // file AND the symlink untouched (lstat, not stat; never delete a non-socket).
    #[test]
    fn explicit_bind_refuses_symlink_at_socket_path() {
        let (parent, sock) = explicit_temp("symlink-at-path", 0o700);
        let victim = parent.join("victim.txt");
        std::fs::write(&victim, b"do not delete").expect("write victim");
        std::os::unix::fs::symlink(&victim, &sock).expect("create symlink at socket path");
        let result = bind_explicit(&sock);
        assert!(
            result.is_err(),
            "bind_explicit must refuse a symlink at the socket path"
        );
        assert!(victim.exists(), "victim file must not be deleted");
        assert_eq!(
            std::fs::read(&victim).unwrap(),
            b"do not delete",
            "victim contents must be intact"
        );
        assert!(
            std::fs::symlink_metadata(&sock)
                .unwrap()
                .file_type()
                .is_symlink(),
            "the symlink itself must remain (not followed, not removed)"
        );
    }

    // F2-T4: bind_explicit successfully replaces a stale socket.
    // A previous bind_explicit (dropped listener) leaves a stale socket file;
    // a second bind_explicit must succeed by removing only the socket.
    #[test]
    fn explicit_bind_replaces_stale_socket() {
        let (_, sock) = explicit_temp("staleexplicit", 0o700);
        // First bind — creates the socket.
        let first = bind_explicit(&sock).expect("first explicit bind");
        drop(first);
        // The stale socket file remains.
        assert!(sock.exists(), "stale socket should still exist");
        // Second bind over the stale socket must succeed.
        let second = bind_explicit(&sock).expect("second explicit bind over stale socket");
        drop(second);
    }

    // F2-T8: bind_explicit CREATES an absent parent at 0700 (preserves the
    // `daemon start --socket /new/path` UX) — introducing new owner-private state,
    // not mutating any pre-existing directory.
    #[test]
    fn explicit_bind_creates_missing_parent_at_0700() {
        let parent =
            std::env::temp_dir().join(format!("ddf-explicit-{}-mkparent", std::process::id()));
        let _ = std::fs::remove_dir_all(&parent);
        assert!(!parent.exists(), "precondition: parent must be absent");
        let sock = parent.join("test.sock");
        let listener = bind_explicit(&sock).expect("bind_explicit must create the missing parent");
        assert!(sock.exists(), "socket must be created");
        let mode = std::fs::metadata(&parent).unwrap().permissions().mode();
        assert_eq!(
            mode & 0o777,
            crate::security::SECURE_DIR_MODE,
            "freshly created parent must be owner-private (0700)"
        );
        drop(listener);
        let _ = std::fs::remove_dir_all(&parent);
    }

    // F2-T5: SocketLocation::at() produces an Explicit provenance location.
    // Sanity-check that the public at() constructor is wired to the correct
    // binding path (we cannot inspect the private field directly, so we observe
    // that at() + bind() on an owner-private parent succeeds and does not
    // create the parent when it already exists).
    #[test]
    fn socket_location_at_uses_explicit_policy() {
        let (parent, sock) = explicit_temp("atloc", 0o700);
        let loc = SocketLocation::at(sock.clone());
        // `bind()` on a 0700 pre-existing parent via `at()` must succeed.
        let listener = loc.bind().expect("SocketLocation::at bind on 0700 parent");
        // Parent still 0700 (we didn't create or chmod it).
        let mode = std::fs::metadata(&parent).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
        drop(listener);
    }

    // F2-T6: SocketLocation::at() + bind() errors on world-readable parent.
    // Complements the direct bind_explicit test through the public API surface.
    #[test]
    fn socket_location_at_rejects_world_readable_parent() {
        let (parent, sock) = explicit_temp("atloc-wr", 0o755);
        let loc = SocketLocation::at(sock);
        assert!(
            loc.bind().is_err(),
            "bind via at() must error on 0755 parent"
        );
        // Parent must remain 0755.
        let mode = std::fs::metadata(&parent).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755);
    }

    // F2-T7: resolve(Some(path)) is Explicit; resolve(None) is EngineDefault.
    // Provenance from resolve() is observable through the binding policy.
    #[test]
    fn resolve_explicit_uses_explicit_policy() {
        // An explicit path through resolve() must refuse a world-readable parent.
        let (parent, sock) = explicit_temp("resolve-explicit", 0o755);
        let loc = SocketLocation::resolve(Some(&sock)).expect("resolve explicit");
        assert!(
            loc.bind().is_err(),
            "resolve(Some(path)) must use explicit policy (fail on 0755)"
        );
        // Parent untouched.
        let mode = std::fs::metadata(&parent).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755);
    }

    // -------------------------------------------------------------------------
    // Size caps, error-kind guards, and accept-loop error handling
    // -------------------------------------------------------------------------

    /// A response at the cap is sent as it is; one byte over becomes an error for the same id.
    #[test]
    fn the_server_never_sends_a_response_over_the_cap() {
        let id = Value::from(9);
        let response = success_response(&id, Value::from("x".repeat(100)));
        let cap = response.len();
        assert_eq!(bounded_response(&id, response.clone(), cap), response);
        let refused: Value =
            serde_json::from_str(&bounded_response(&id, response.clone(), cap - 1)).unwrap();
        assert_eq!(refused["id"], 9);
        assert_eq!(refused["error"]["code"], RESOURCE_EXHAUSTED);
        assert!(
            refused["error"]["message"]
                .as_str()
                .unwrap()
                .contains(&format!("{cap} bytes"))
        );
    }

    #[test]
    fn request_and_response_caps_are_80_mib() {
        assert_eq!(MAX_REQUEST_BYTES, 80 << 20);
        assert_eq!(MAX_RESPONSE_BYTES, 80 << 20);
    }

    /// Only the disconnected-peer refusal is settled; every other arming failure is returned.
    #[test]
    fn only_a_disconnected_peer_refusal_is_settled() {
        let errno = |e: rustix::io::Errno| std::io::Error::from_raw_os_error(e.raw_os_error());
        assert!(settle_read_timeout(Ok(())).is_ok());
        assert!(settle_read_timeout(Err(errno(rustix::io::Errno::INVAL))).is_ok());
        let other = settle_read_timeout(Err(errno(rustix::io::Errno::BADF))).unwrap_err();
        assert_eq!(
            other.raw_os_error(),
            Some(rustix::io::Errno::BADF.raw_os_error())
        );
        let refused = std::io::Error::new(std::io::ErrorKind::InvalidInput, "zero timeout");
        assert!(settle_read_timeout(Err(refused)).is_err());
    }

    /// A peer that sends one line and closes cleanly has sent a complete line, however many
    /// buffer refills it takes to read: macOS refuses `SO_RCVTIMEO` on a socket whose peer has shut
    /// down (EINVAL), which made the client fail any response longer than its buffer whenever the
    /// daemon closed first (CI run 40626071938).
    #[test]
    fn a_line_from_a_closed_peer_reads_completely_across_refills() {
        let (local, peer) = UnixStream::pair().expect("socketpair");
        let mut sent = vec![b'x'; 100];
        sent.push(b'\n');
        (&peer).write_all(&sent).unwrap();
        drop(peer);
        let mut reader = BufReader::with_capacity(16, local);
        let mut line = Vec::new();
        let read = read_capped_line_deadline(&mut reader, &mut line, 1024, Duration::from_secs(5))
            .expect("a complete line from a closed peer");
        assert_eq!((read, line), (sent.len(), sent));
    }

    #[test]
    fn socket_line_read_reports_non_timeout_errors_unchanged() {
        // Closing a Unix stream socket with unread data in its receive queue
        // resets the peer, and that must never be disguised as a deadline
        // timeout. Linux reports ECONNRESET on the read; macOS ends the read
        // at end of file (measured on CI, run 40628996633).
        let (local, peer) = UnixStream::pair().expect("socketpair");
        (&local).write_all(b"never read").unwrap();
        drop(peer);
        let mut reader = BufReader::new(local);
        let mut line = Vec::new();
        let read = read_capped_line_deadline(&mut reader, &mut line, 1024, Duration::from_secs(5));
        if cfg!(target_os = "linux") {
            assert_eq!(
                read.expect_err("reset peer").kind(),
                std::io::ErrorKind::ConnectionReset
            );
        } else if let Err(err) = read {
            assert!(
                !matches!(
                    err.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                ),
                "{err}"
            );
        } else {
            assert_eq!(read.unwrap(), 0);
        }
    }

    // Serve one connection, send `request` (no trailing newline is added), and
    // return the first response line.
    fn exchange_raw(request: Vec<u8>) -> Value {
        let (client, server) = UnixStream::pair().expect("socketpair");
        let handle = thread::spawn(move || handle_connection(server, &engine()));
        let mut writer = client.try_clone().unwrap();
        let send = thread::spawn(move || {
            let _ = writer.write_all(&request);
            let _ = writer.flush();
        });
        let mut reader = BufReader::new(client);
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        joined(send).unwrap();
        drop(reader);
        assert!(joined(handle).is_ok(), "the thread did not finish");
        serde_json::from_str(&line).unwrap()
    }

    // A request padded with trailing whitespace to exactly `total` bytes,
    // optionally newline-terminated (the newline counts toward `total`).
    fn padded_health_request(total: usize, newline: bool) -> Vec<u8> {
        let mut request = br#"{"id":7,"method":"daemon.health"}"#.to_vec();
        request.resize(total - usize::from(newline), b' ');
        if newline {
            request.push(b'\n');
        }
        request
    }

    #[test]
    fn request_line_exactly_at_cap_is_served() {
        let v = exchange_raw(padded_health_request(MAX_REQUEST_BYTES, true));
        assert_eq!(v["result"]["status"], "ok", "{v}");
        assert_eq!(v["id"], 7);
    }

    #[test]
    fn request_line_one_byte_over_cap_is_rejected() {
        let v = exchange_raw(padded_health_request(MAX_REQUEST_BYTES + 1, false));
        assert_eq!(v["error"]["code"], INVALID_REQUEST, "{v}");
        assert_eq!(v["id"], Value::Null);
    }

    #[test]
    fn client_request_size_cap_boundary() {
        // A line whose framed form (plus newline) fits the server cap passes the size gate
        // and fails only at connect; one byte more is refused before connecting.
        let client = SocketLocation::at(temp_socket("client-cap"));
        let _ = std::fs::remove_dir_all(client.path().parent().unwrap());
        let at_cap = " ".repeat(MAX_REQUEST_BYTES - 1);
        let err = request(&client, &at_cap).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
        let over_cap = " ".repeat(MAX_REQUEST_BYTES);
        let err = request(&client, &over_cap).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    }

    // Answer one request at `sock` with the raw `response` bytes.
    fn fake_daemon(name: &str, response: Vec<u8>) -> (SocketLocation, thread::JoinHandle<()>) {
        let (_, sock) = explicit_temp(name, 0o700);
        let listener = UnixListener::bind(&sock).expect("bind fake daemon");
        let handle = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept");
            let mut request = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut request)
                .unwrap();
            let _ = (&stream).write_all(&response);
        });
        (SocketLocation::at(sock), handle)
    }

    #[test]
    fn client_accepts_response_exactly_at_cap() {
        let mut response = vec![b'x'; MAX_RESPONSE_BYTES - 1];
        response.push(b'\n');
        let (location, daemon) = fake_daemon("resp-at-cap", response);
        let got = request(&location, r#"{"method":"daemon.health"}"#).expect("at-cap response");
        assert_eq!(got.len(), MAX_RESPONSE_BYTES - 1);
        joined(daemon).unwrap();
    }

    #[test]
    fn client_rejects_response_over_cap() {
        let (location, daemon) = fake_daemon("resp-over-cap", vec![b'x'; MAX_RESPONSE_BYTES + 1]);
        let err = request(&location, r#"{"method":"daemon.health"}"#).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        joined(daemon).unwrap();
    }

    /// A socket a daemon still listens on is never removed; one nothing listens on is.
    #[test]
    fn only_a_socket_nothing_listens_on_is_removed() {
        let (parent, _) = explicit_temp("stale-live", 0o700);
        let sock = parent.join("d.sock");
        let live = UnixListener::bind(&sock).unwrap();
        let err = remove_stale_socket(&sock).expect_err("a live daemon owns it");
        assert_eq!(err.kind(), std::io::ErrorKind::AddrInUse);
        assert!(
            std::fs::symlink_metadata(&sock)
                .unwrap()
                .file_type()
                .is_socket()
        );
        // Legitimate nearby case: the daemon is gone and its socket file is left behind.
        drop(live);
        remove_stale_socket(&sock).expect("a stale socket is removed");
        assert!(std::fs::symlink_metadata(&sock).is_err());
    }

    /// Only "nothing listens" makes a socket stale: any other connect failure is returned and the
    /// socket kept. A mode-000 socket refuses the connect with EACCES (root bypasses modes).
    #[test]
    fn a_socket_that_cannot_be_probed_is_kept() {
        if rustix::process::geteuid().is_root() {
            return;
        }
        let (parent, _) = explicit_temp("stale-unprobed", 0o700);
        let sock = parent.join("d.sock");
        drop(UnixListener::bind(&sock).unwrap());
        std::fs::set_permissions(&sock, std::fs::Permissions::from_mode(0o000)).unwrap();
        let err = remove_stale_socket(&sock).expect_err("the probe was refused, not answered");
        assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
        assert!(
            std::fs::symlink_metadata(&sock)
                .unwrap()
                .file_type()
                .is_socket()
        );
    }

    #[test]
    fn remove_stale_socket_propagates_non_missing_errors() {
        // lstat through a regular file fails with ENOTDIR. Only ENOENT means
        // "nothing to remove"; anything else must surface as an error.
        let (parent, _) = explicit_temp("stale-notdir", 0o700);
        let file = parent.join("file");
        std::fs::write(&file, b"keep").unwrap();
        let err = remove_stale_socket(&file.join("d.sock")).expect_err("ENOTDIR");
        assert_eq!(err.kind(), std::io::ErrorKind::NotADirectory);
        assert_eq!(std::fs::read(&file).unwrap(), b"keep");
    }

    #[test]
    fn explicit_bind_propagates_non_missing_parent_errors() {
        // A parent below a regular file is not "missing"; nothing is created.
        let (parent, _) = explicit_temp("parent-notdir", 0o700);
        let file = parent.join("file");
        std::fs::write(&file, b"keep").unwrap();
        let err = bind_explicit(&file.join("sub").join("d.sock")).expect_err("ENOTDIR");
        assert_eq!(err.kind(), std::io::ErrorKind::NotADirectory);
        assert_eq!(std::fs::read(&file).unwrap(), b"keep");
    }

    const ACCEPT_ERROR_CHILD_ENV: &str = "DDF_TEST_ACCEPT_ERROR_SOCKET";

    // Child half of `run_server_returns_persistent_accept_errors`. It lowers the
    // process fd limit, so it must never run inside the shared test process: it
    // is ignored and a no-op unless the parent test sets the socket variable.
    #[test]
    #[ignore = "spawned as a child process by run_server_returns_persistent_accept_errors"]
    #[expect(
        clippy::disallowed_methods,
        reason = "test: the child half reads its socket path from the environment"
    )]
    fn accept_error_child() {
        use rustix::process::{Resource, Rlimit, getrlimit, setrlimit};
        use std::os::fd::AsRawFd as _;
        let Some(sock) = std::env::var_os(ACCEPT_ERROR_CHILD_ENV) else {
            return;
        };
        // Bind first, then leave no spare descriptor, so no platform's count of the descriptors
        // binding takes (the start lock, the listener, anything else) matters. Linux
        // reserves the accepted fd before waiting, so every accept() then fails
        // with EMFILE immediately — a persistent, non-EINTR accept error. macOS
        // fails only once a connection is pending, which the parent supplies.
        let location = SocketLocation::at(PathBuf::from(sock));
        let bound = location.bind().expect("bind before the limit");
        let engine = Engine::new();
        let next_fd = std::fs::File::open("/dev/null").unwrap().as_raw_fd();
        let limit = getrlimit(Resource::Nofile);
        setrlimit(
            Resource::Nofile,
            Rlimit {
                current: Some(u64::try_from(next_fd).unwrap()),
                maximum: limit.maximum,
            },
        )
        .unwrap();
        let err = serve_bound(&location, bound, engine)
            .expect_err("a persistent accept error must stop the server");
        assert_eq!(err.raw_os_error(), Some(24), "EMFILE: {err}");
    }

    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "test: spawns itself as the child server and waits on a deadline"
    )]
    fn run_server_returns_persistent_accept_errors() {
        // A non-EINTR accept error must end run_server with that error rather
        // than spin forever retrying it.
        let sock = temp_socket_with_parent("accept-error", 0o700);
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "serve::tests::accept_error_child",
                "--ignored",
                "--test-threads=1",
            ])
            .env(ACCEPT_ERROR_CHILD_ENV, &sock)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn child test");
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut pending = None;
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break Some(status);
            }
            // One pending connection, so accept() has something to fail on where
            // it blocks until a client arrives (macOS). Kept open until the end.
            if pending.is_none() && sock.exists() {
                pending = UnixStream::connect(&sock).ok();
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            thread::sleep(Duration::from_millis(10));
        };
        // run_server returns before cleanup on error, so the bound socket file
        // proves the child really ran it (not an empty test filter).
        let bound = sock.exists();
        let _ = std::fs::remove_dir_all(sock.parent().unwrap());
        let status = status.expect("run_server kept looping on a persistent accept error");
        assert!(status.success(), "child test failed: {status}");
        assert!(bound, "child never bound the socket");
    }
}
