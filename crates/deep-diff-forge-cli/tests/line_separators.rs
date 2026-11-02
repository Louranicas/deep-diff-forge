//! Bug hunt 2026-10-28, H4-06: U+2028 and U+2029 in a patch never split a JSON record for a
//! line-aware reader, on any machine surface: `--json`, `--jsonl`, `rpc` and MCP. Bug hunt
//! 2026-11-01: nor do `DEL` and the C1 controls (the 8-bit CSI and OSC introducers) reach a
//! terminal raw from the serde-built views (`--hunks`, `--outline`, `rpc`, MCP).
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

/// A path, `@@` heading and line carrying the 8-bit CSI (`U+009B`), OSC (`U+009D`) and `DEL`.
const C1_PATCH: &str =
    "--- a/p\u{9b}2J\n+++ b/p\u{9b}2J\n@@ -1 +1 @@ fn \u{9d}0;T\u{7}\n-a\n+\u{9b}2J\u{7f}b\n";

fn assert_no_raw_c1(text: &str) {
    assert!(
        !text.chars().any(|c| matches!(u32::from(c), 0x7f..=0x9f)),
        "{text:?}"
    );
}

#[test]
fn del_and_c1_are_escaped_by_every_serde_built_view() {
    for mode in ["--hunks", "--outline"] {
        let out = run(&["--stdin-patch", mode], C1_PATCH);
        assert_no_raw_c1(&out);
        let page: serde_json::Value = serde_json::from_str(&out).unwrap();
        let path = if mode == "--hunks" {
            &page["hunks"][0]["path"]
        } else {
            &page["files"][0]["path"]
        };
        assert_eq!(
            path, "p\u{9b}2J",
            "{mode}: the decoded document is unchanged"
        );
    }
    let hunks: serde_json::Value =
        serde_json::from_str(&run(&["--stdin-patch", "--hunks"], C1_PATCH)).unwrap();
    assert_eq!(hunks["hunks"][0]["lines"][1]["text"], "\u{9b}2J\u{7f}b");
    assert_eq!(hunks["hunks"][0]["section"], "fn \u{9d}0;T\u{7}");

    let request =
        serde_json::json!({"id": 1, "method": "review.hunks", "params": {"patch": C1_PATCH}});
    let rpc_out = run(&["rpc"], &format!("{request}\n"));
    assert_no_raw_c1(&rpc_out);
    let rpc = assert_records(&rpc_out, 1);
    assert_eq!(rpc[0]["result"]["hunks"][0]["path"], "p\u{9b}2J");

    let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#;
    let call = serde_json::json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "review_hunks", "arguments": {"patch": C1_PATCH}}});
    let mcp_out = run(&["mcp"], &format!("{init}\n{call}\n"));
    assert_no_raw_c1(&mcp_out);
    let mcp = assert_records(&mcp_out, 2);
    assert_eq!(
        mcp[1]["result"]["structuredContent"]["hunks"][0]["path"],
        "p\u{9b}2J"
    );
}

/// A page's budget counts the escapes: each two-byte C1 character is written as six bytes, so a
/// page of them, measured raw, was printed well over `--max-bytes` (and over MCP's `max_bytes`).
#[test]
fn a_page_of_c1_text_fits_its_budget_as_printed() {
    use std::fmt::Write as _;
    // Quotes double again in MCP's text copy, so its fit loop starts near the budget.
    let mut patch = String::from("--- a/x\n+++ b/x\n@@ -0,0 +1,200 @@\n");
    for _ in 0..200 {
        let _ = writeln!(patch, "+{}", "\"\u{9b}".repeat(15));
    }
    let printed = run(&["--stdin-patch", "--hunks", "--max-bytes", "8192"], &patch);
    assert!(printed.trim_end().len() <= 8192, "{}", printed.len());

    let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#;
    let call = serde_json::json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "review_hunks", "arguments": {"patch": patch, "max_bytes": 8192}}});
    let out = run(&["mcp"], &format!("{init}\n{call}\n"));
    let answer = out.lines().nth(1).unwrap();
    let result = serde_json::to_string(
        &serde_json::from_str::<serde_json::Value>(answer).unwrap()["result"],
    )
    .unwrap();
    assert!(
        deep_diff_forge_core::json_line_safe(&result).len() <= 8192,
        "{}",
        answer.len()
    );
}

/// Review 2026-11-01: the pretty-printed documents go through the same escaping. Before,
/// `findings verify` printed a finding's patch path, and `deploy doctor --json` its socket path,
/// with the raw 8-bit CSI.
#[test]
fn del_and_c1_are_escaped_by_the_pretty_printed_documents() {
    let dir = std::env::temp_dir().join(format!("ddf-c1-pretty-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let findings = dir.join("f.jsonl");
    let finding = serde_json::json!({"agent": "a", "severity": "low", "title": "t\u{9d}", "body": "b\u{7f}", "path": "p\u{9b}2J"});
    std::fs::write(&findings, format!("{finding}\n")).unwrap();
    let out = run(
        &[
            "findings",
            "verify",
            "--findings",
            findings.to_str().unwrap(),
        ],
        C1_PATCH,
    );
    assert_no_raw_c1(&out);
    let doc: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(doc["accepted"][0]["path"], "p\u{9b}2J", "{out}");

    // No daemon listens there, so doctor exits 6; its document is what is tested.
    let socket = dir.join("s\u{9b}k");
    let doctor = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .args(["deploy", "doctor", "--json", "--socket"])
        .arg(&socket)
        .output()
        .unwrap();
    let text = String::from_utf8(doctor.stdout).unwrap();
    assert_no_raw_c1(&text);
    let doc: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert!(
        doc.to_string().contains('\u{9b}'),
        "the socket path is reported: {text}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
