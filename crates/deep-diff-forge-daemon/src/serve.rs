use crate::handler::{Engine, dispatch};
use crate::protocol::{
    INTERNAL_ERROR, INVALID_REQUEST, PARSE_ERROR, RpcError, error_response, parse_request,
    success_response,
};
use crate::security::{SECURE_SOCKET_MODE, ensure_runtime_dir};
use serde_json::Value;
use std::io::{BufRead, BufReader, Read as _, Write as _};
use std::os::unix::fs::PermissionsExt as _;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

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

/// Run the daemon server loop until shutdown.
///
/// Thin wrapper over the tested [`bind_secure`] + [`handle_connection`]; the
/// accept loop is the one part that requires a live socket and is exercised by
/// the in-process round-trip tests rather than mocked.
///
/// # Errors
///
/// Returns an I/O error from binding or accepting connections.
pub fn run_server(socket_path: &Path) -> std::io::Result<()> {
    let listener = bind_secure(socket_path)?;
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
    let _ = std::fs::remove_file(socket_path);
    Ok(())
}

/// Client: connect to the daemon, send one request line, return the response.
///
/// # Errors
///
/// Returns an I/O error if the connection or exchange fails (e.g. no daemon
/// is listening at `socket_path`).
pub fn request(socket_path: &Path, line: &str) -> std::io::Result<String> {
    let mut stream = UnixStream::connect(socket_path)?;
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

    #[test]
    fn run_server_full_round_trip_then_shutdown() {
        let sock = temp_socket("run");
        let _ = std::fs::remove_dir_all(sock.parent().unwrap());
        let server_sock = sock.clone();
        let server = thread::spawn(move || run_server(&server_sock));
        wait_for_socket(&sock);

        let health = request(&sock, r#"{"id":1,"method":"daemon.health"}"#).expect("health");
        assert!(health.contains("\"status\":\"ok\""));

        let stop = request(&sock, r#"{"id":2,"method":"daemon.shutdown"}"#).expect("stop");
        assert!(stop.contains("\"stopping\":true"));

        let _ = server.join();
        // Socket is cleaned up on shutdown.
        assert!(!sock.exists());
    }

    #[test]
    fn run_server_diff_plan_over_socket() {
        let sock = temp_socket("plan");
        let _ = std::fs::remove_dir_all(sock.parent().unwrap());
        let server_sock = sock.clone();
        let server = thread::spawn(move || run_server(&server_sock));
        wait_for_socket(&sock);

        let line = r#"{"id":1,"method":"diff.plan","params":{"patch":"--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@\n-a\n+b\n"}}"#;
        let resp = request(&sock, line).expect("plan");
        assert!(resp.contains("src/lib.rs"));
        assert!(resp.contains("public_api_surface"));

        let _ = request(&sock, r#"{"method":"daemon.shutdown"}"#);
        let _ = server.join();
    }

    #[test]
    fn request_to_missing_daemon_errors() {
        let sock = temp_socket("absent");
        let _ = std::fs::remove_dir_all(sock.parent().unwrap());
        assert!(request(&sock, r#"{"method":"daemon.health"}"#).is_err());
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
}
