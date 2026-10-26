// DDF slice C: `deep-diff-forge rpc`, the daemon's JSON-RPC over stdio, served in-process.
// Design/status: docs/ROADMAP.md#status-2026-10-26 and docs/API_AND_IPC.md.
//! The daemon protocol without a daemon: each stdin line is answered by the same
//! `deep_diff_forge_daemon::process_line` the socket server uses, under the same request cap and
//! the same framing rules, so an SDK can speak one protocol over a socket or a pipe.

use deep_diff_forge_daemon::{
    Engine, INVALID_REQUEST, MAX_REQUEST_BYTES, PARSE_ERROR, RpcError, error_response, process_line,
};
use serde_json::Value;
use std::io::{BufRead, Write};
use std::sync::Mutex;

/// Answer stdin until it closes or a `daemon.shutdown` request is answered.
pub fn serve(input: &mut impl BufRead, output: &mut impl Write) -> std::io::Result<()> {
    serve_with(input, output, MAX_REQUEST_BYTES)
}

/// [`serve`] with an explicit request cap.
fn serve_with(
    input: &mut impl BufRead,
    output: &mut impl Write,
    cap: usize,
) -> std::io::Result<()> {
    let engine = Mutex::new(Engine::new());
    while let Some(line) = crate::mcp::read_line(input, cap)? {
        let Ok(bytes) = line else {
            // As on the socket: past the cap the stream position is unreliable, so stop.
            let error = RpcError::new(INVALID_REQUEST, "request exceeds maximum size");
            writeln!(output, "{}", error_response(&Value::Null, &error))?;
            output.flush()?;
            return Ok(());
        };
        let response = match std::str::from_utf8(&bytes) {
            Ok(text) if text.trim().is_empty() => continue,
            Ok(text) => process_line(text.trim(), &engine),
            Err(_) => error_response(
                &Value::Null,
                &RpcError::new(PARSE_ERROR, "request is not valid UTF-8"),
            ),
        };
        writeln!(output, "{response}")?;
        output.flush()?;
        if !engine.lock().is_ok_and(|engine| engine.is_running()) {
            return Ok(());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(input: &[u8], cap: usize) -> Vec<Value> {
        let input = input.to_vec();
        let (send, receive) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut out = Vec::new();
            serve_with(&mut input.as_slice(), &mut out, cap).unwrap();
            send.send(out)
        });
        let out = receive
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("rpc never stopped reading");
        String::from_utf8(out)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    #[test]
    fn answers_like_the_socket_daemon() {
        let out = run(b"{\"id\":1,\"method\":\"daemon.health\"}\n\n{\"id\":2,\"method\":\"no.such\"}\nnot json\n", 1024);
        assert_eq!(out.len(), 3);
        assert_eq!(
            (out[0]["id"].as_i64(), out[0]["result"]["status"].as_str()),
            (Some(1), Some("ok"))
        );
        assert_eq!(out[1]["error"]["code"], -32601);
        assert_eq!(
            (out[2]["id"].is_null(), out[2]["error"]["code"].as_i64()),
            (true, Some(PARSE_ERROR))
        );
    }

    #[test]
    fn shutdown_ends_the_session() {
        let out = run(
            b"{\"id\":1,\"method\":\"daemon.shutdown\"}\n{\"id\":2,\"method\":\"daemon.health\"}\n",
            1024,
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["result"]["stopping"], true);
    }

    #[test]
    fn an_oversize_request_is_refused_and_closes_the_stream() {
        let mut input = vec![b'x'; 300];
        input.extend_from_slice(b"\n{\"id\":2,\"method\":\"daemon.health\"}\n");
        let out = run(&input, 256);
        assert_eq!(out.len(), 1);
        assert_eq!(
            (
                out[0]["error"]["code"].as_i64(),
                out[0]["error"]["message"].as_str()
            ),
            (Some(INVALID_REQUEST), Some("request exceeds maximum size"))
        );
    }

    #[test]
    fn invalid_utf8_is_a_parse_error_and_serving_continues() {
        let out = run(b"\xff\n{\"id\":3,\"method\":\"daemon.health\"}\n", 1024);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0]["error"]["code"], PARSE_ERROR);
        assert_eq!(out[1]["id"], 3);
    }

    #[test]
    fn the_cap_is_the_daemon_socket_cap() {
        assert_eq!(MAX_REQUEST_BYTES, 80 * 1024 * 1024);
    }
}
