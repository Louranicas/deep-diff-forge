//! Slice D: `deep-diff-forge mcp` over real stdio — framing, both protocol eras, and parity of
//! `review_patch` with `deep-diff-forge --stdin-patch --json`.
#![expect(
    clippy::disallowed_methods,
    reason = "integration test: drives real processes, the environment and wall-clock time"
)]

use serde_json::{Value, json};
use std::io::{Read as _, Write as _};
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
    // Drain the pipes while waiting: a response larger than the pipe buffer would otherwise block
    // the server's write and look like a hang.
    let drain = |mut pipe: Box<dyn std::io::Read + Send>| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            pipe.read_to_end(&mut bytes).unwrap();
            bytes
        })
    };
    let stdout = drain(Box::new(child.stdout.take().unwrap()));
    let stderr = drain(Box::new(child.stderr.take().unwrap()));
    // A server that never exits fails here instead of hanging the suite.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if std::time::Instant::now() > deadline {
            child.kill().unwrap();
            panic!("the server did not exit when stdin closed");
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    let stdout = String::from_utf8(stdout.join().unwrap()).unwrap();
    // Nothing but JSON-RPC responses on stdout, one per line.
    let messages = stdout
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap_or_else(|e| panic!("{e}: {l}")))
        .inspect(|m| assert_eq!(m["jsonrpc"], "2.0", "{m}"))
        .collect();
    (
        status.code().unwrap(),
        messages,
        String::from_utf8(stderr.join().unwrap()).unwrap(),
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

fn call(name: &str, arguments: &Value) -> Value {
    let request = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"_meta": meta(), "name": name, "arguments": arguments}});
    let (code, messages, stderr) = exchange(&[request.to_string()]);
    assert_eq!(code, 0, "{stderr}");
    messages.into_iter().next().unwrap()["result"].clone()
}

/// A patch whose full review is over the 256 KiB cap: 400 files of 20 changed lines each.
fn big_patch() -> String {
    use std::fmt::Write as _;
    let mut patch = String::new();
    for f in 0..400 {
        let _ = write!(
            patch,
            "diff --git a/f{f}.rs b/f{f}.rs\n--- a/f{f}.rs\n+++ b/f{f}.rs\n@@ -1,10 +1,10 @@\n"
        );
        for i in 0..10 {
            let _ = write!(
                patch,
                "-let old_{i} = \"{}\";\n+let new_{i} = \"{}\";\n",
                "x".repeat(40),
                "y".repeat(40)
            );
        }
    }
    patch
}

/// Anchor A-07: a review over the cap comes back as a bounded summary, not one enormous line (a
/// 30 MB patch produced a 178 MB response). Before, the full document was always returned.
#[test]
fn an_oversized_review_returns_a_bounded_summary() {
    let patch = big_patch();
    let full = cli_review(&patch);
    assert!(full.to_string().len() > 256 * 1024);
    let result = call("review_patch", &json!({"patch": patch}));
    assert_eq!(result["isError"], false, "{result}");
    let summary = &result["structuredContent"];
    assert_eq!(summary["schema"], "deep-diff-forge.review-summary.v0");
    assert_eq!(summary["input_sha256"], full["input_sha256"]);
    assert_eq!(summary["summary"], full["summary"]);
    assert_eq!(summary["files"].as_array().unwrap().len(), 400);
    assert_eq!(summary["omitted_files"], 0);
    assert_eq!(
        summary["files"][7],
        json!({"path": "f7.rs", "status": "modified", "hunks": 1, "additions": 10, "deletions": 10})
    );
    assert!(summary["reason"].as_str().unwrap().contains("pass paths"));
    assert!(result.to_string().len() < 256 * 1024);
}

/// The legitimate side: under the cap the document is the CLI's, and `paths` zooms in to full
/// detail for chosen files while keeping the whole patch's seal.
#[test]
fn paths_return_chosen_files_in_full_and_small_reviews_are_unchanged() {
    let small = call("review_patch", &json!({"patch": PATCH}));
    assert_eq!(small["structuredContent"], cli_review(PATCH));

    let patch = big_patch();
    let full = cli_review(&patch);
    let zoom = call(
        "review_patch",
        &json!({"patch": patch, "paths": ["f3.rs", "f9.rs"]}),
    );
    let doc = &zoom["structuredContent"];
    assert_eq!(doc["schema"], "deep-diff-forge.review.v0");
    assert_eq!(doc["input_sha256"], full["input_sha256"]);
    assert_eq!(doc["selected"], json!(["f3.rs", "f9.rs"]));
    assert_eq!(doc["files"], json!([full["files"][3], full["files"][9]]));

    let asked = call(
        "review_patch",
        &json!({"patch": PATCH, "detail": "summary"}),
    );
    assert_eq!(asked["structuredContent"]["reason"], "summary requested");
}

#[test]
fn bad_detail_or_paths_are_tool_errors() {
    for arguments in [
        json!({"patch": PATCH, "detail": "full"}),
        json!({"patch": PATCH, "paths": "src/lib.rs"}),
        json!({"patch": PATCH, "paths": [1]}),
        json!({"patch": PATCH, "paths": ["nope.rs"]}),
    ] {
        let result = call("review_patch", &arguments);
        assert_eq!(result["isError"], true, "{arguments}: {result}");
    }
    let missing = call(
        "review_patch",
        &json!({"patch": PATCH, "paths": ["nope.rs"]}),
    );
    assert_eq!(
        missing["content"][0]["text"],
        "no file in the patch is named nope.rs"
    );
}
