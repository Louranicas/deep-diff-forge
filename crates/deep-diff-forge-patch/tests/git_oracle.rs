// DDF integration: R02, R03, R04, R05, R06, R07, R44, R46. Preserve source bytes and exact original input independently of normalized display; use Git as the application oracle.
// Design/status: docs/RECOMMENDATION_INTEGRATION.md#patch-truth
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fdeep-diff-forge-patch--tests--git_oracle
//! Independent oracle: Git must apply our rendered patch to the exact base bytes.
use deep_diff_forge_patch::{parse, render_unified};
use std::{fs, path::Path, process::Command};
fn git(root: &Path, args: &[&str]) -> Vec<u8> {
    let out = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}
#[test]
fn reconstructed_patches_apply_to_exact_base() {
    let root = std::env::temp_dir().join(format!("ddf-git-oracle-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    git(&root, &["init", "-q"]);
    let cases: Vec<(&str, Vec<u8>, Vec<u8>)> = vec![
        ("eof", b"old".to_vec(), b"new".to_vec()),
        ("old-eof", b"old".to_vec(), b"new\n".to_vec()),
        ("new-eof", b"old\n".to_vec(), b"new".to_vec()),
        (
            "mixed",
            b"first\r\nold\nlast".to_vec(),
            b"first\r\nnew\nlast".to_vec(),
        ),
        ("crlf", b"old\r\n".to_vec(), b"new\r\n".to_vec()),
        (
            "\u{00e9}\tquoted\".txt",
            b"old\n".to_vec(),
            b"new\n".to_vec(),
        ),
        ("binary", (0..=255).collect(), (0..=255).rev().collect()),
    ];
    for (path, old, new) in cases {
        fs::write(root.join(path), &old).unwrap();
        git(&root, &["add", "--", path]);
        fs::write(root.join(path), &new).unwrap();
        let patch = String::from_utf8(git(
            &root,
            &[
                "diff",
                "--binary",
                "--full-index",
                "--no-ext-diff",
                "--no-textconv",
                "--",
                path,
            ],
        ))
        .unwrap();
        let files = parse(&patch).unwrap();
        assert_eq!(files[0].path, path);
        let rendered = render_unified(&files);
        fs::write(root.join(path), old).unwrap();
        fs::write(root.join("apply.patch"), rendered).unwrap();
        git(&root, &["apply", "--check", "apply.patch"]);
        git(&root, &["apply", "apply.patch"]);
        assert_eq!(fs::read(root.join(path)).unwrap(), new, "{path}");
        git(&root, &["add", "--", path]);
    }
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn unsupported_and_invalid_paths_fail_explicitly() {
    assert!(parse("diff --cc file\n").is_err());
    assert!(parse("diff --git \"a/\\377\" \"b/\\377\"\n").is_err());
}
#[test]
fn malformed_or_orphaned_eof_markers_fail_instead_of_normalizing() {
    assert!(parse("--- a/x\n+++ b/x\n\\ No newline at end of file\n").is_err());
    assert!(
        parse("--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n\\ No newline at end invalid\n").is_err()
    );
    assert!(parse("--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n\\ No newline at end of file\n\\ No newline at end of file\n").is_err());
}
