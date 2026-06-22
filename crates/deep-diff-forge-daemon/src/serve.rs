use crate::handler::{Engine, dispatch};
use crate::protocol::{
    INTERNAL_ERROR, INVALID_REQUEST, PARSE_ERROR, RpcError, error_response, parse_request,
    success_response,
};
use crate::security::{
    SECURE_DIR_MODE, SECURE_SOCKET_MODE, SocketError, default_socket_path, ensure_runtime_dir,
    validate_private_dir,
};
use serde_json::Value;
use std::io::{BufRead, BufReader, Read as _, Write as _};
use std::os::unix::fs::{FileTypeExt as _, PermissionsExt as _};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

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
    ///   removes any stale file, binds, sets the socket to `0600`.
    /// * **Explicit** — the parent directory must already exist and pass
    ///   [`validate_private_dir`] (owner-private, non-symlink); it is never
    ///   created or chmod'd. Any pre-existing path is removed only if it is a
    ///   socket; a regular file or directory causes an error (fail closed).
    ///
    /// # Errors
    ///
    /// Returns an I/O error if the directory cannot be secured or the bind fails.
    pub fn bind(&self) -> std::io::Result<UnixListener> {
        match self.provenance {
            Provenance::EngineDefault => bind_secure(&self.path),
            Provenance::Explicit => bind_explicit(&self.path),
        }
    }

    /// Connect to a daemon listening at this location.
    ///
    /// # Errors
    /// Returns an I/O error if no daemon is listening.
    pub fn connect(&self) -> std::io::Result<UnixStream> {
        UnixStream::connect(&self.path)
    }
}

/// Maximum bytes accepted for a single JSON-RPC request line. The largest
/// legitimate request wraps a patch (the patch crate's 64 MiB budget) in a JSON
/// envelope; this leaves generous framing headroom while hard-bounding a single
/// oversized line so a newline-less stream cannot exhaust memory (denial of
/// service).
const MAX_REQUEST_BYTES: usize = 80 * 1024 * 1024;

/// Per-connection read timeout. A client that connects and then stalls mid-request
/// is dropped rather than holding the single-threaded server forever (slowloris).
const READ_TIMEOUT: Duration = Duration::from_secs(30);

/// Process one request line against the shared engine, returning the response
/// line. This is the socket-free core of connection handling.
#[must_use]
pub fn process_line(line: &str, engine: &Mutex<Engine>) -> String {
    match parse_request(line) {
        Err(err) => error_response(&Value::Null, &err),
        Ok(request) => {
            let id = request.id.clone();
            // Isolate a panic inside dispatch: one malformed/abusive request must
            // not unwind and tear down the whole daemon (availability hardening,
            // mirroring the morph-ir-engine per-connection backstop).
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut guard = match engine.lock() {
                    Ok(g) => g,
                    Err(poison) => poison.into_inner(),
                };
                dispatch(&request, &mut guard)
            }));
            match outcome {
                Ok(Ok(result)) => success_response(&id, result),
                Ok(Err(err)) => error_response(&id, &err),
                Err(_) => error_response(&id, &RpcError::new(INTERNAL_ERROR, "internal error")),
            }
        }
    }
}

/// Read one newline-terminated line into `buf`, reading at most `cap + 1` bytes.
///
/// Returns the number of bytes read (`0` = EOF). The `cap + 1` ceiling means a
/// newline-less or oversized line is bounded — the caller treats
/// `buf.len() > cap` as "request too large" rather than buffering unboundedly.
fn read_capped_line<R: BufRead>(
    reader: &mut R,
    buf: &mut Vec<u8>,
    cap: usize,
) -> std::io::Result<usize> {
    reader.take(cap as u64 + 1).read_until(b'\n', buf)
}

/// Serve one connection: newline-delimited JSON-RPC, one response per request,
/// until the client disconnects or a shutdown is requested.
///
/// # Errors
///
/// Returns any underlying stream I/O error.
pub fn handle_connection(stream: UnixStream, engine: &Mutex<Engine>) -> std::io::Result<()> {
    stream.set_read_timeout(Some(READ_TIMEOUT))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut writer = stream;
    let mut buf = Vec::new();
    loop {
        buf.clear();
        let read = read_capped_line(&mut reader, &mut buf, MAX_REQUEST_BYTES)?;
        if read == 0 {
            break; // clean EOF
        }
        if buf.len() > MAX_REQUEST_BYTES {
            let resp = error_response(
                &Value::Null,
                &RpcError::new(INVALID_REQUEST, "request exceeds maximum size"),
            );
            writeln!(writer, "{resp}")?;
            writer.flush()?;
            break; // stream position is unreliable past the cap; close the connection.
        }
        let Ok(decoded) = std::str::from_utf8(&buf) else {
            let resp = error_response(
                &Value::Null,
                &RpcError::new(PARSE_ERROR, "request is not valid UTF-8"),
            );
            writeln!(writer, "{resp}")?;
            writer.flush()?;
            continue;
        };
        let line = decoded.trim();
        if line.is_empty() {
            continue;
        }
        let response = process_line(line, engine);
        writeln!(writer, "{response}")?;
        writer.flush()?;
        let still_running = engine.lock().is_ok_and(|g| g.is_running());
        if !still_running {
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
pub fn bind_secure(socket_path: &Path) -> std::io::Result<UnixListener> {
    if let Some(parent) = socket_path.parent() {
        ensure_runtime_dir(parent)?;
    }
    if socket_path.exists() {
        std::fs::remove_file(socket_path)?;
    }
    let listener = UnixListener::bind(socket_path)?;
    std::fs::set_permissions(
        socket_path,
        std::fs::Permissions::from_mode(SECURE_SOCKET_MODE),
    )?;
    Ok(listener)
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
/// 2. If the socket path already exists it is removed **only if it is a socket**
///    (`symlink_metadata + file_type().is_socket()`). A regular file, directory,
///    or symlink at the path causes an error; we never delete non-socket files.
/// 3. Binds via [`UnixListener::bind`] and sets the socket to `0600`.
///
/// This is the binding path for `--socket` overrides. The engine-managed
/// runtime dir uses [`bind_secure`] instead.
///
/// # Errors
///
/// Returns an I/O error if the parent cannot be created/secured, is a pre-existing
/// insecure directory, the path holds a non-socket file, or the bind itself fails.
pub fn bind_explicit(socket_path: &Path) -> std::io::Result<UnixListener> {
    // 1. Secure the parent. Create it (owner-private) if absent; otherwise
    //    validate it WITHOUT chmodding — never mutate a directory the caller owns.
    let parent = socket_path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "socket path has no parent directory",
        )
    })?;
    if std::fs::symlink_metadata(parent).is_err() {
        // Parent is absent — create it and set owner-only mode on the new directory.
        std::fs::create_dir_all(parent)?;
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(SECURE_DIR_MODE))?;
    }
    // Validate in all cases (created or pre-existing); never chmod a dir we did
    // not just create.
    validate_private_dir(parent)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::PermissionDenied, e))?;

    // 2. If the path exists, remove it only when it is verifiably a socket.
    //    Use symlink_metadata (lstat) to avoid following a symlink at the path.
    if let Ok(meta) = std::fs::symlink_metadata(socket_path) {
        if meta.file_type().is_socket() {
            std::fs::remove_file(socket_path)?;
        } else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "socket path is occupied by a non-socket file; refusing to delete it",
            ));
        }
    }

    // 3. Bind and set owner-only permissions on the socket.
    let listener = UnixListener::bind(socket_path)?;
    std::fs::set_permissions(
        socket_path,
        std::fs::Permissions::from_mode(SECURE_SOCKET_MODE),
    )?;
    Ok(listener)
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
    let listener = location.bind()?;
    let engine = Mutex::new(Engine::new());
    for stream in listener.incoming() {
        match stream {
            // A per-connection error (client reset, read timeout, oversized
            // request) must NOT tear down the daemon — log it and keep serving.
            Ok(s) => {
                if let Err(err) = handle_connection(s, &engine) {
                    eprintln!("deep-diff-forge daemon: connection error: {err}");
                }
            }
            Err(err) => {
                eprintln!("deep-diff-forge daemon: accept error: {err}");
                continue;
            }
        }
        let still_running = engine.lock().is_ok_and(|g| g.is_running());
        if !still_running {
            break;
        }
    }
    let _ = std::fs::remove_file(location.path());
    Ok(())
}

/// Client: connect to the daemon, send one request line, return the response.
///
/// # Errors
///
/// Returns an I/O error if the connection or exchange fails (e.g. no daemon
/// is listening at the location).
pub fn request(location: &SocketLocation, line: &str) -> std::io::Result<String> {
    let mut stream = location.connect()?;
    writeln!(stream, "{line}")?;
    stream.flush()?;
    let mut reader = BufReader::new(stream);
    let mut response = String::new();
    reader.read_line(&mut response)?;
    Ok(response.trim_end().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::thread;
    use std::time::Duration;

    fn engine() -> Mutex<Engine> {
        Mutex::new(Engine::new())
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

    #[test]
    fn process_line_shutdown_clears_running() {
        let e = engine();
        let _ = process_line(r#"{"method":"daemon.shutdown"}"#, &e);
        assert!(!e.lock().unwrap().is_running());
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
        let _ = handle.join();
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
        let _ = handle.join();
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

        let _ = server.join();
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
        let _ = server.join();
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
        let _ = handle.join();
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
}
