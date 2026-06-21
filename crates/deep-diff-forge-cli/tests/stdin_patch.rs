//! CLI contract tests for `--stdin-patch`, exercising the real compiled binary
//! across the process boundary (stdin, stdout, stderr, exit codes).

use std::io::Write as _;
use std::process::{Command, Stdio};

fn run(args: &[&str], stdin: &str) -> (i32, String, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn binary");
    child
        .stdin
        .take()
        .expect("stdin handle")
        .write_all(stdin.as_bytes())
        .expect("write stdin");
    let out = child.wait_with_output().expect("wait for binary");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

const PATCH: &str = "--- a/x\n+++ b/x\n@@ -1,1 +1,1 @@\n-old\n+new\n";

#[test]
fn json_mode_emits_schema_on_stdout() {
    let (code, stdout, _) = run(&["--stdin-patch", "--json"], PATCH);
    assert_eq!(code, 0);
    assert!(stdout.contains("\"schema\": \"deep-diff-forge.review.v0\""));
}

#[test]
fn json_mode_reports_counts() {
    let (code, stdout, _) = run(&["--stdin-patch", "--json"], PATCH);
    assert_eq!(code, 0);
    assert!(stdout.contains("\"additions\": 1"));
    assert!(stdout.contains("\"deletions\": 1"));
}

#[test]
fn human_mode_summarises_files() {
    let (code, stdout, _) = run(&["--stdin-patch"], PATCH);
    assert_eq!(code, 0);
    assert!(stdout.contains('x'));
    assert!(stdout.contains("1 file(s) changed"));
}

#[test]
fn empty_input_is_zero_files_not_an_error() {
    let (code, stdout, _) = run(&["--stdin-patch"], "");
    assert_eq!(code, 0);
    assert!(stdout.contains("0 file(s) changed"));
}

#[test]
fn malformed_patch_exits_four_with_stderr_diagnostic() {
    let bad = "--- a/x\n+++ b/x\n+stray addition with no hunk\n";
    let (code, stdout, stderr) = run(&["--stdin-patch"], bad);
    assert_eq!(code, 4);
    assert!(stdout.is_empty());
    assert!(stderr.contains("patch parse failed"));
}

#[test]
fn jsonl_mode_streams_one_event_per_file() {
    let two =
        "--- a/a\n+++ b/a\n@@ -1,1 +1,1 @@\n-a\n+A\n--- a/b\n+++ b/b\n@@ -1,1 +1,1 @@\n-b\n+B\n";
    let (code, stdout, _) = run(&["--stdin-patch", "--jsonl"], two);
    assert_eq!(code, 0);
    assert_eq!(stdout.lines().count(), 2);
    assert!(stdout.contains("\"event\":\"diff.file\""));
}

#[test]
fn jsonl_malformed_exits_four() {
    let bad = "--- a/x\n+++ b/x\n+stray\n";
    let (code, stdout, stderr) = run(&["--stdin-patch", "--jsonl"], bad);
    assert_eq!(code, 4);
    assert!(stdout.is_empty());
    assert!(!stderr.is_empty());
}

#[test]
fn rank_json_emits_schema_and_orders_public_api_first() {
    let two = "\
diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,1 +1,1 @@
-a
+b
diff --git a/tests/it.rs b/tests/it.rs
--- a/tests/it.rs
+++ b/tests/it.rs
@@ -1,1 +1,1 @@
-a
+b
";
    let (code, stdout, _) = run(&["--stdin-patch", "--rank", "--json"], two);
    assert_eq!(code, 0);
    assert!(stdout.contains("\"schema\": \"deep-diff-forge.rank.v0\""));
    assert!(stdout.contains("public_api_surface"));
    // src/lib.rs (public api) should appear before tests/it.rs in the output.
    let lib_pos = stdout.find("src/lib.rs").unwrap();
    let test_pos = stdout.find("tests/it.rs").unwrap();
    assert!(lib_pos < test_pos);
}

#[test]
fn cluster_json_emits_schema_and_receipt() {
    let two = "\
diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,1 +1,1 @@
-a
+b
diff --git a/src/other.rs b/src/other.rs
--- a/src/other.rs
+++ b/src/other.rs
@@ -1,1 +1,1 @@
-a
+b
";
    let (code, stdout, _) = run(
        &["--stdin-patch", "--cluster", "--parallel", "2", "--json"],
        two,
    );
    assert_eq!(code, 0);
    assert!(stdout.contains("\"schema\": \"deep-diff-forge.cluster.v0\""));
    assert!(stdout.contains("\"parallelism\": \"fixed:2\""));
    assert!(stdout.contains("\"join_policy\": \"ranked-review-order\""));
    assert!(stdout.contains("src/lib.rs"));
}

#[test]
fn cluster_human_prints_receipt_footer() {
    let (code, stdout, _) = run(
        &["--stdin-patch", "--cluster", "--parallel", "serial"],
        PATCH,
    );
    assert_eq!(code, 0);
    assert!(stdout.contains("cluster:"));
    assert!(stdout.contains("parallelism=serial"));
}

#[test]
fn rank_human_lists_files_and_count() {
    let (code, stdout, _) = run(&["--stdin-patch", "--rank"], PATCH);
    assert_eq!(code, 0);
    assert!(stdout.contains("1 file(s) ranked"));
}

#[test]
fn inline_layout_renders_header() {
    let (code, stdout, _) = run(&["--stdin-patch", "--layout", "inline"], PATCH);
    assert_eq!(code, 0);
    assert!(stdout.contains("modified  x"));
}

#[test]
fn help_exits_zero() {
    let (code, stdout, _) = run(&["--help"], "");
    assert_eq!(code, 0);
    assert!(stdout.contains("--stdin-patch"));
}

#[test]
fn unknown_command_exits_two() {
    let (code, _, stderr) = run(&["frobnicate"], "");
    assert_eq!(code, 2);
    assert!(stderr.contains("unknown command"));
}

#[test]
fn diagnostics_go_to_stderr_not_stdout() {
    let bad = "--- a/x\n+++ b/x\n-stray removal with no hunk\n";
    let (_, stdout, stderr) = run(&["--stdin-patch", "--json"], bad);
    assert!(stdout.is_empty(), "stdout must stay clean on error");
    assert!(!stderr.is_empty(), "stderr must carry the diagnostic");
}
