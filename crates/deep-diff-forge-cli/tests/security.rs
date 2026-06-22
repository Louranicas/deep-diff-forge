//! End-to-end terminal-injection regression tests.
//!
//! A unified diff is fully attacker-controlled. A reviewer who renders an
//! attacker's patch must not have ANSI/CSI/OSC escape sequences from the diff
//! body, file paths, or symbol names reach their terminal. These tests feed a
//! patch laced with raw escape sequences through every human-facing output mode
//! and assert that no raw `ESC` (0x1b) byte survives to stdout.

use std::io::Write as _;
use std::process::{Command, Stdio};

const ESC: u8 = 0x1b;
const BEL: u8 = 0x07;

/// A unified diff whose path AND body lines carry terminal escape sequences:
/// an OSC window-title set + OSC-52 clipboard write + CSI screen-clear.
fn evil_patch() -> Vec<u8> {
    let mut p = Vec::new();
    p.extend_from_slice(b"--- a/evil");
    p.push(ESC);
    p.extend_from_slice(b"]0;pwned");
    p.push(BEL);
    p.extend_from_slice(b".rs\n+++ b/evil");
    p.push(ESC);
    p.extend_from_slice(b"]0;pwned");
    p.push(BEL);
    p.extend_from_slice(b".rs\n@@ -1,2 +1,2 @@\n-old");
    p.push(ESC);
    p.extend_from_slice(b"[2J\n+new");
    p.push(ESC);
    p.extend_from_slice(b"]52;c;ZXZpbA==");
    p.push(BEL);
    p.extend_from_slice(b"\n ctx");
    p.push(ESC);
    p.extend_from_slice(b"[1;1H\n");
    p
}

fn run_with_stdin(args: &[&str], stdin: &[u8]) -> (i32, Vec<u8>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(stdin)
        .expect("write stdin");
    let out = child.wait_with_output().expect("wait");
    (out.status.code().unwrap_or(-1), out.stdout)
}

fn assert_no_raw_esc(mode: &str, stdout: &[u8]) {
    assert!(
        !stdout.contains(&ESC),
        "raw ESC (0x1b) leaked to stdout in mode `{mode}` — terminal injection"
    );
    // The escaped, inert form must be present so we know content was shown.
    assert!(
        String::from_utf8_lossy(stdout).contains("\\x1b"),
        "mode `{mode}` did not render the escaped form (sanitizer not applied?)"
    );
}

#[test]
fn summary_mode_neutralizes_escapes() {
    let (code, out) = run_with_stdin(&["--stdin-patch"], &evil_patch());
    assert_eq!(code, 0);
    assert_no_raw_esc("--stdin-patch (summary)", &out);
}

#[test]
fn inline_layout_neutralizes_escapes() {
    let (code, out) = run_with_stdin(&["--stdin-patch", "--layout", "inline"], &evil_patch());
    assert_eq!(code, 0);
    assert_no_raw_esc("--layout inline", &out);
}

#[test]
fn side_by_side_layout_neutralizes_escapes() {
    let (code, out) = run_with_stdin(
        &["--stdin-patch", "--layout", "side-by-side"],
        &evil_patch(),
    );
    assert_eq!(code, 0);
    assert_no_raw_esc("--layout side-by-side", &out);
}

#[test]
fn rank_mode_neutralizes_path_escapes() {
    // --rank prints the (attacker-controlled) path in the human table.
    let (code, out) = run_with_stdin(&["--stdin-patch", "--rank"], &evil_patch());
    assert_eq!(code, 0);
    assert!(
        !out.contains(&ESC),
        "raw ESC leaked via --rank path rendering"
    );
}

#[test]
fn json_mode_is_already_safe() {
    // The machine path uses json_escape, which neutralises ESC and the C1 block.
    let (code, out) = run_with_stdin(&["--stdin-patch", "--json"], &evil_patch());
    assert_eq!(code, 0);
    assert!(!out.contains(&ESC), "raw ESC leaked via --json");
}

#[test]
fn json_empty_input_schema_snapshot_is_stable() {
    let (code, out) = run_with_stdin(&["--stdin-patch", "--json"], b"");
    assert_eq!(code, 0);
    let text = String::from_utf8(out).expect("json output is utf8");
    assert!(text.contains("\"schema\": \"deep-diff-forge.review.v0\""));
    assert!(text.contains("\"files\": []"));
    assert!(text.contains("\"files_changed\": 0"));
    assert!(text.contains("\"additions\": 0"));
    assert!(text.contains("\"deletions\": 0"));
    assert!(text.contains("\"semantic_fallbacks\": 0"));
}

#[test]
fn jsonl_empty_input_is_zero_read_success() {
    let (code, out) = run_with_stdin(&["--stdin-patch", "--jsonl"], b"");
    assert_eq!(code, 0);
    assert!(
        out.is_empty(),
        "empty patch should stream zero JSONL events"
    );
}

#[test]
fn trojan_source_bidi_override_is_neutralized() {
    // CVE-2021-42574: a diff body carrying U+202E (RLO, UTF-8 `e2 80 ae`) could
    // visually reorder code so a reviewer sees something other than what runs.
    // The human render must escape it to a visible \u{202e}, not pass it through.
    let mut patch = Vec::new();
    patch.extend_from_slice(b"--- a/x.rs\n+++ b/x.rs\n@@ -1,1 +1,1 @@\n-let ok = a;\n+let ok = a;");
    patch.extend_from_slice(&[0xe2, 0x80, 0xae]); // U+202E RLO
    patch.extend_from_slice(b" // evil\n");
    let (code, out) = run_with_stdin(&["--stdin-patch", "--layout", "inline"], &patch);
    assert_eq!(code, 0);
    assert!(
        !out.windows(3).any(|w| w == [0xe2, 0x80, 0xae]),
        "raw U+202E bidi override must not reach the terminal"
    );
    assert!(
        String::from_utf8_lossy(&out).contains("\\u{202e}"),
        "the bidi override should be shown as a visible escape"
    );
}
