// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fdeep-diff-forge-cli--rpc
// DDF slice C: `deep-diff-forge rpc`, the daemon's JSON-RPC over stdio, served in-process.
// Design/status: docs/ROADMAP.md#status-2026-10-26 and docs/API_AND_IPC.md.
//! The daemon protocol without a daemon: each stdin line is answered by the same
//! `deep_diff_forge_daemon::process_line` the socket server uses, under the same request cap and
//! the same framing rules, so an SDK can speak one protocol over a socket or a pipe.

use deep_diff_forge_core::json_line_safe;
use deep_diff_forge_daemon::{
    Engine, INVALID_REQUEST, MAX_REQUEST_BYTES, PARSE_ERROR, RpcError, error_response,
    process_line, read_capped_line,
};
use serde_json::Value;
use std::io::{BufRead, Write};
use std::sync::Mutex;

/// Answer stdin until it closes or a `daemon.shutdown` request is answered.
pub fn serve(input: &mut impl BufRead, output: &mut impl Write) -> std::io::Result<()> {
    serve_with(input, output, MAX_REQUEST_BYTES)
}

/// [`serve`] with a configured engine (for example one allowed to read repositories).
pub fn serve_engine(
    input: &mut impl BufRead,
    output: &mut impl Write,
    engine: Engine,
) -> std::io::Result<()> {
    serve_capped(input, output, MAX_REQUEST_BYTES, engine)
}

/// [`serve`] with an explicit request cap. The framing is the socket server's own
/// (`read_capped_line`): the cap counts the newline, and an oversized request is refused as soon
/// as `cap + 1` bytes have arrived, then the stream is closed (council sdc1). Crate-visible so
/// the `stdio_rpc` fuzz target can drive small caps.
pub(crate) fn serve_with(
    input: &mut impl BufRead,
    output: &mut impl Write,
    cap: usize,
) -> std::io::Result<()> {
    serve_capped(input, output, cap, Engine::new())
}

fn serve_capped(
    input: &mut impl BufRead,
    output: &mut impl Write,
    cap: usize,
    engine: Engine,
) -> std::io::Result<()> {
    let engine = Mutex::new(engine);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        if read_capped_line(input, &mut buf, cap)? == 0 {
            return Ok(());
        }
        if buf.len() > cap {
            let error = RpcError::new(INVALID_REQUEST, "request exceeds maximum size");
            writeln!(
                output,
                "{}",
                json_line_safe(&error_response(&Value::Null, &error))
            )?;
            output.flush()?;
            return Ok(());
        }
        let response = match std::str::from_utf8(&buf) {
            Ok(text) if text.trim().is_empty() => continue,
            Ok(text) => process_line(text.trim(), &engine),
            Err(_) => error_response(
                &Value::Null,
                &RpcError::new(PARSE_ERROR, "request is not valid UTF-8"),
            ),
        };
        writeln!(output, "{}", json_line_safe(&response))?;
        output.flush()?;
        if !engine.lock().is_ok_and(|engine| engine.is_running()) {
            return Ok(());
        }
    }
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

    /// The cap counts the newline, as on the socket: `cap` bytes plus a newline is one too many.
    #[test]
    fn the_cap_counts_the_newline() {
        let request = br#"{"id":1,"method":"daemon.health"}"#;
        let mut input = request.to_vec();
        input.push(b'\n');
        assert_eq!(run(&input, request.len() + 1)[0]["id"], 1);
        assert_eq!(
            run(&input, request.len())[0]["error"]["code"],
            INVALID_REQUEST
        );
    }

    /// An oversized request with no newline yet is refused at once: the stream is not drained.
    #[test]
    fn an_unterminated_oversize_request_is_refused_without_waiting() {
        let (reader, mut writer) = std::io::pipe().unwrap();
        writer.write_all(&[b'x'; 64]).unwrap(); // writer stays open: no newline, no EOF
        let (send, receive) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut out = Vec::new();
            serve_with(&mut std::io::BufReader::new(reader), &mut out, 32).unwrap();
            send.send(out)
        });
        let out = receive
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("rpc waited for a newline");
        assert!(
            String::from_utf8(out)
                .unwrap()
                .contains("request exceeds maximum size")
        );
        drop(writer);
    }

    #[test]
    fn the_cap_is_the_daemon_socket_cap() {
        assert_eq!(MAX_REQUEST_BYTES, 80 * 1024 * 1024);
    }
}
