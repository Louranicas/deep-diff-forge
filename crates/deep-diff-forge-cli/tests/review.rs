//! CLI contract tests for `review --probe` (headless render).

use std::io::Write as _;
use std::process::{Command, Stdio};

fn run(args: &[&str], stdin: &str) -> (i32, String, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(stdin.as_bytes())
        .expect("write");
    let out = child.wait_with_output().expect("wait");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

const PATCH: &str = "\
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

#[test]
fn probe_renders_the_review_frame() {
    let (code, stdout, _) = run(&["review", "--probe"], PATCH);
    assert_eq!(code, 0);
    // Chrome: the menu bar and the ranked file tree.
    assert!(stdout.contains("deep-diff-forge"));
    assert!(stdout.contains("Files (ranked)"));
    // The selected file's path titles the diff pane.
    assert!(stdout.contains("src/lib.rs"));
}

#[test]
fn review_help_does_not_require_a_tty_or_patch() {
    let (code, stdout, stderr) = run(&["review", "--help"], "");
    assert_eq!(code, 0);
    assert!(stderr.is_empty());
    assert!(stdout.contains("deep-diff-forge review"));
    assert!(stdout.contains("--probe"));
    assert!(stdout.contains("--cmd NAME"));
}

#[test]
fn probe_shows_diff_and_engine_note() {
    let (code, stdout, _) = run(&["review", "--probe"], PATCH);
    assert_eq!(code, 0);
    // The diff pane header carries the file status and a hunk header.
    assert!(stdout.contains("modified"));
    assert!(stdout.contains("@@"));
    // lib.rs is a public-API surface, so the engine emits a grounded inline
    // note — proving the annotations wiring reaches the headless frame.
    assert!(stdout.contains("system note"));
}

#[test]
fn probe_on_malformed_patch_exits_four() {
    let (code, stdout, stderr) = run(&["review", "--probe"], "--- a/x\n+++ b/x\n+stray\n");
    assert_eq!(code, 4);
    assert!(stdout.is_empty());
    assert!(stderr.contains("patch parse failed"));
}

#[test]
fn probe_empty_patch_renders_placeholder() {
    let (code, stdout, _) = run(&["review", "--probe"], "");
    assert_eq!(code, 0);
    assert!(stdout.contains("no files"));
}

#[test]
fn probe_status_bar_advertises_the_viewed_key_and_progress() {
    // A wide frame so the right-aligned state is not truncated.
    let (code, stdout, _) = run(&["review", "--probe", "--cols", "200"], PATCH);
    assert_eq!(code, 0);
    // The left-aligned hints advertise the new key…
    assert!(
        stdout.contains("v reviewed"),
        "status hints should advertise the viewed key"
    );
    // …and the right-aligned state shows review progress (PATCH has 2 files).
    assert!(
        stdout.contains("viewed:0/2"),
        "a fresh review should report 0 of 2 reviewed"
    );
}
