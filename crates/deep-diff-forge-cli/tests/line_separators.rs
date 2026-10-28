//! Bug hunt 2026-10-28, H4-06: U+2028 and U+2029 in a patch never split a JSON record for a
//! line-aware reader, on any machine surface: `--json`, `--jsonl`, `rpc` and MCP.
#![expect(
    clippy::disallowed_methods,
    reason = "integration test: drives the binary"
)]

use std::io::Write as _;
use std::process::{Command, Stdio};

const PATCH: &str = "--- a/x\u{2028}y\n+++ b/x\u{2028}y\n@@ -1 +1 @@\n-a\n+b\u{2029}c\n";

fn run(args: &[&str], stdin: &str) -> String {
    let mut child = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

/// Every record is one line even to a Unicode line reader, and still carries the characters.
fn assert_records(text: &str, records: usize) -> Vec<serde_json::Value> {
    assert!(!text.contains(['\u{2028}', '\u{2029}']), "{text}");
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(lines.len(), records, "{text}");
    lines
        .iter()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[test]
fn line_separators_are_escaped_on_every_machine_surface() {
    let document: serde_json::Value =
        serde_json::from_str(&run(&["--stdin-patch", "--json"], PATCH)).unwrap();
    assert_eq!(document["files"][0]["path"], "x\u{2028}y");
    assert!(!run(&["--stdin-patch", "--json"], PATCH).contains('\u{2028}'));

    let jsonl = run(&["--stdin-patch", "--jsonl"], PATCH);
    assert!(!jsonl.contains(['\u{2028}', '\u{2029}']), "{jsonl}");

    let request =
        serde_json::json!({"id": 1, "method": "diff.compute", "params": {"patch": PATCH}});
    let rpc = assert_records(&run(&["rpc"], &format!("{request}\n")), 1);
    assert_eq!(rpc[0]["result"]["files"][0]["path"], "x\u{2028}y");

    let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#;
    let call = serde_json::json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "review_patch", "arguments": {"patch": PATCH}}});
    let mcp = assert_records(&run(&["mcp"], &format!("{init}\n{call}\n")), 2);
    assert_eq!(
        mcp[1]["result"]["structuredContent"]["files"][0]["path"],
        "x\u{2028}y"
    );
}
