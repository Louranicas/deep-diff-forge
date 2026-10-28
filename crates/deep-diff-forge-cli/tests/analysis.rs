//! Source-pair adapter integration, including fail-closed path and byte validation.
#![expect(
    clippy::disallowed_methods,
    reason = "integration test: drives real processes, the environment and wall-clock time"
)]

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};
const PATCH: &str = "--- a/x.rs\n+++ b/x.rs\n@@ -1 +1 @@\n-fn a() {}\n+fn b() {}\n";
fn setup(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("ddf-analysis-{}-{label}", std::process::id()));
    fs::create_dir_all(root.join("old")).unwrap();
    fs::create_dir_all(root.join("new")).unwrap();
    fs::write(root.join("old/x.rs"), "fn a() {}\n").unwrap();
    fs::write(root.join("new/x.rs"), "fn b() {}\n").unwrap();
    fs::write(root.join("change.patch"), PATCH).unwrap();
    root
}
fn run(root: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .arg("analyze")
        .arg(root.join("change.patch"))
        .arg(root.join("old"))
        .arg(root.join("new"))
        .output()
        .unwrap()
}
#[test]
fn analyze_exports_deterministic_scoped_semantics() {
    let root = setup("success");
    let first = run(&root);
    let second = run(&root);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert_eq!(first.stdout, second.stdout);
    let json = String::from_utf8(first.stdout).unwrap();
    for field in [
        "deep-diff-forge.analysis.v1",
        "\"behavior_verified\":false",
        "\"coverage\":\"analyzed\"",
        "\"semantic_spans\":[{",
        "\"snapshot_scope\":\"supplied_changed_sources\"",
    ] {
        assert!(json.contains(field), "{field}: {json}");
    }
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn wrong_source_bytes_and_traversal_are_rejected() {
    let root = setup("failure");
    fs::write(root.join("new/x.rs"), "unrelated\n").unwrap();
    assert_eq!(run(&root).status.code(), Some(4));
    fs::write(
        root.join("change.patch"),
        PATCH.replace("x.rs", "../outside.rs"),
    )
    .unwrap();
    assert_eq!(run(&root).status.code(), Some(4));
    fs::remove_dir_all(root).unwrap();
}
#[cfg(unix)]
#[test]
fn source_symlinks_are_rejected() {
    let root = setup("symlink");
    fs::remove_file(root.join("old/x.rs")).unwrap();
    std::os::unix::fs::symlink(root.join("new/x.rs"), root.join("old/x.rs")).unwrap();
    let output = run(&root);
    assert_eq!(output.status.code(), Some(4));
    assert!(String::from_utf8_lossy(&output.stderr).contains("symlink rejected"));
    fs::remove_dir_all(root).unwrap();
}

/// A wrong number of paths or an option is a usage error (exit 2, fix the command line). Before,
/// a missing path exited 4, which tells a caller to change the input instead.
#[test]
fn analyze_takes_exactly_three_paths() {
    for args in [
        &[][..],
        &["change.patch", "old"][..],
        &["change.patch", "old", "new", "extra"][..],
        &["--json", "change.patch", "old", "new"][..],
    ] {
        let out = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
            .arg("analyze")
            .args(args)
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        assert_eq!(out.stdout, b"", "{args:?}");
        assert!(
            String::from_utf8_lossy(&out.stderr).contains(
                "usage: deep-diff-forge analyze <patch-file> <base-directory> <candidate-directory>"
            ),
            "{args:?}"
        );
    }
}
