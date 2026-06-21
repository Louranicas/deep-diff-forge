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
    assert!(stdout.contains("Files (ranked)"));
    assert!(stdout.contains("Detail"));
    assert!(stdout.contains("src/lib.rs"));
}

#[test]
fn probe_shows_ranked_detail() {
    let (code, stdout, _) = run(&["review", "--probe"], PATCH);
    assert_eq!(code, 0);
    // The first (public-API) file is selected; detail shows its status.
    assert!(stdout.contains("status:"));
    assert!(stdout.contains("modified"));
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
