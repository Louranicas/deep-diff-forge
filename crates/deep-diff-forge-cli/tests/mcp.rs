//! Slice D: `deep-diff-forge mcp` over real stdio — framing, both protocol eras, and parity of
//! `review_patch` with `deep-diff-forge --stdin-patch --json`.
use serde_json::{Value, json};
use std::io::Write as _;
use std::process::{Command, Stdio};

const PATCH: &str = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 +1,2 @@\n fn a() {}\n-fn b() {}\n+fn b() -> u8 { 1 }\n";

fn exchange(lines: &[String]) -> (i32, Vec<Value>, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    for line in lines {
        writeln!(stdin, "{line}").unwrap();
    }
    drop(stdin); // end of input is the shutdown signal
    // A server that never exits fails here instead of hanging the suite.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while child.try_wait().unwrap().is_none() {
        if std::time::Instant::now() > deadline {
            child.kill().unwrap();
            panic!("the server did not exit when stdin closed");
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let out = child.wait_with_output().unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    // Nothing but JSON-RPC responses on stdout, one per line.
    let messages = stdout
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap_or_else(|e| panic!("{e}: {l}")))
        .inspect(|m| assert_eq!(m["jsonrpc"], "2.0", "{m}"))
        .collect();
    (
        out.status.code().unwrap(),
        messages,
        String::from_utf8(out.stderr).unwrap(),
    )
}

fn meta() -> Value {
    json!({"io.modelcontextprotocol/protocolVersion": "2026-07-28", "io.modelcontextprotocol/clientCapabilities": {}})
}

fn cli_review(patch: &str) -> Value {
    let mut child = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .args(["--stdin-patch", "--json"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(patch.as_bytes())
        .unwrap();
    serde_json::from_slice(&child.wait_with_output().unwrap().stdout).unwrap()
}

#[test]
fn a_modern_client_discovers_lists_and_reviews_with_cli_parity() {
    let requests = [
        json!({"jsonrpc": "2.0", "id": "d", "method": "server/discover", "params": {"_meta": meta()}}),
        json!({"jsonrpc": "2.0", "method": "notifications/cancelled", "params": {"requestId": "x"}}),
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {"_meta": meta()}}),
        json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"_meta": meta(), "name": "review_patch", "arguments": {"patch": PATCH, "require_files": true, "require_hunks": true}}}),
    ];
    let (code, out, _) = exchange(&requests.iter().map(Value::to_string).collect::<Vec<_>>());
    assert_eq!(code, 0);
    assert_eq!(out.len(), 3, "the notification gets no response: {out:?}");
    assert_eq!(out[0]["id"], "d");
    assert_eq!(out[0]["result"]["supportedVersions"], json!(["2026-07-28"]));
    assert_eq!(out[1]["result"]["tools"].as_array().unwrap().len(), 4);
    let result = &out[2]["result"];
    assert_eq!(
        (result["isError"].as_bool(), result["resultType"].as_str()),
        (Some(false), Some("complete"))
    );
    assert_eq!(result["structuredContent"], cli_review(PATCH));
    let text: Value = serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(text, result["structuredContent"]);
}

#[test]
fn a_legacy_client_uses_the_handshake() {
    let requests = [
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "legacy", "version": "1"}}}),
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "explain_exit_code", "arguments": {"code": 4}}}),
    ];
    let (code, out, _) = exchange(&requests.iter().map(Value::to_string).collect::<Vec<_>>());
    assert_eq!(code, 0);
    assert_eq!(out[0]["result"]["protocolVersion"], "2025-11-25");
    assert_eq!(out[0]["result"]["serverInfo"]["name"], "deep-diff-forge");
    assert_eq!(
        out[1]["result"]["structuredContent"]["name"],
        "invalid_input"
    );
}

#[test]
fn garbage_lines_get_errors_and_the_server_keeps_serving() {
    let ping = json!({"jsonrpc": "2.0", "id": 9, "method": "ping", "params": {"_meta": meta()}})
        .to_string();
    let (code, out, _) = exchange(&["not json".into(), String::new(), ping]);
    assert_eq!(code, 0);
    assert_eq!(out.len(), 2);
    assert_eq!(
        (out[0]["id"].is_null(), out[0]["error"]["code"].as_i64()),
        (true, Some(-32700))
    );
    assert_eq!(out[1]["id"], 9);
}

#[test]
fn mcp_takes_no_arguments() {
    let out = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .args(["mcp", "--write"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(out.stdout.len(), 0);
}

/// The transport's failure modes decide the exit code: a reader that went away is a clean end
/// (exit 0); any other write failure is an I/O failure (exit 3).
#[test]
fn a_closed_reader_ends_cleanly_and_a_failed_write_is_an_io_failure() {
    let ping = format!(
        "{}\n",
        json!({"jsonrpc": "2.0", "id": 1, "method": "ping", "params": {"_meta": meta()}})
    );
    for (stdout, expected) in [
        (Stdio::piped(), 0),
        (
            Stdio::from(
                std::fs::OpenOptions::new()
                    .write(true)
                    .open("/dev/full")
                    .unwrap(),
            ),
            3,
        ),
    ] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
            .arg("mcp")
            .stdin(Stdio::piped())
            .stdout(stdout)
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        drop(child.stdout.take()); // for the pipe: the reader is gone before anything is written
        let mut stdin = child.stdin.take().unwrap();
        stdin.write_all(ping.as_bytes()).unwrap();
        drop(stdin);
        assert_eq!(child.wait().unwrap().code(), Some(expected));
    }
}
