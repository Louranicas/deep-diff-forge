//! CLI contract tests for `review --probe` (headless render).

#![expect(
    clippy::disallowed_methods,
    reason = "integration test: drives real processes, the environment and wall-clock time"
)]

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
    // A command that refuses its arguments exits without reading stdin, closing the pipe first;
    // that is not a failure (the exit code is what is checked). Any other write error is.
    if let Err(error) = child
        .stdin
        .take()
        .expect("stdin")
        .write_all(stdin.as_bytes())
    {
        assert_eq!(error.kind(), std::io::ErrorKind::BrokenPipe, "{error}");
    }
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
    assert_eq!(stderr.len(), 0);
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
    assert_eq!(stdout.len(), 0);
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

/// An unknown `--cmd` name is a usage error that names the commands there are. Before, it fell
/// through to the first command (`rank`) and exited 0, so a typo looked like an answer.
#[test]
fn an_unknown_palette_command_is_refused() {
    let (code, stdout, stderr) = run(&["review", "--probe", "--cmd", "rnak"], PATCH);
    assert_eq!(code, 2, "{stderr}");
    assert_eq!(stdout, "");
    assert!(stderr.contains("unknown review command rnak;"), "{stderr}");
    assert!(
        stderr
            .contains("rank, outline, cluster, summary, notes, review, daemon, learning, maturity"),
        "{stderr}"
    );
}

/// Every documented command name, and the full `review json` label, opens its own panel.
#[test]
fn each_palette_command_opens_its_own_panel() {
    for (name, title) in [
        ("rank", "rank"),
        ("outline", "outline"),
        ("maturity", "maturity"),
        ("review json", "review"),
    ] {
        let (code, stdout, stderr) = run(
            &["review", "--probe", "--cols", "120", "--cmd", name],
            PATCH,
        );
        assert_eq!(code, 0, "{name}: {stderr}");
        assert!(stdout.contains(&format!("╭ {title}")), "{name}:\n{stdout}");
    }
}

/// A probe size that is not a number is a usage error; before, it silently became the default.
#[test]
fn a_probe_size_that_is_not_a_number_is_refused() {
    for flag in ["--cols", "--rows"] {
        let (code, stdout, stderr) = run(&["review", "--probe", flag, "wide"], PATCH);
        assert_eq!(code, 2, "{flag}: {stderr}");
        assert_eq!(stdout, "");
        assert!(
            stderr.contains(&format!(
                "{flag} takes a number of terminal cells, not wide"
            )),
            "{stderr}"
        );
    }
    let (code, stdout, _) = run(&["review", "--probe", "--cols", "50", "--rows", "9"], PATCH);
    assert_eq!(code, 0);
    assert_eq!(stdout.lines().count(), 9);
    assert!(stdout.lines().all(|l| l.chars().count() <= 50));
}

/// Bug hunt 2026-10-28, H4-08: at 80 columns the notes panel cut the hazard warning after "what
/// you see", losing "may not be what applies.". Panel lines wrap at its width now.
#[test]
fn the_notes_panel_wraps_a_long_warning() {
    let patch = "--- a/../x.rs\n+++ b/../x.rs\n@@ -1 +1 @@\n-a\n+b\n";
    let (code, stdout, stderr) = run(
        &[
            "review", "--probe", "--cols", "80", "--rows", "25", "--cmd", "notes",
        ],
        patch,
    );
    assert_eq!(code, 0, "{stderr}");
    let flat = stdout.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(flat.contains("may not be what applies."), "{stdout}");
    assert!(stdout.lines().all(|l| l.chars().count() <= 80), "{stdout}");
}
