// DDF integration: R02, R03, R04, R05, R06, R07, R44, R46. Preserve source bytes and exact original input independently of normalized display; use Git as the application oracle.
// Design/status: docs/RECOMMENDATION_INTEGRATION.md#patch-truth
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fdeep-diff-forge-patch--tests--git_oracle
//! Independent oracle: Git must apply our rendered patch to the exact base bytes.
#![expect(
    clippy::disallowed_methods,
    reason = "integration test: drives real processes, the environment and wall-clock time"
)]

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

/// Run `git apply --check` on `patch` in `root`; true when Git accepts it.
fn git_accepts(root: &Path, patch: &str) -> bool {
    fs::write(root.join("probe.patch"), patch).unwrap();
    Command::new("git")
        .current_dir(root)
        .args(["apply", "--check", "probe.patch"])
        .output()
        .unwrap()
        .status
        .success()
}

/// Bug hunt 2026-10-28, H1-09 and H4-07: DDF refuses exactly the rename and copy targets and
/// the modes Git refuses, and labels what Git writes for a copy, a mode change and a type
/// change as Git means them.
#[test]
fn unsafe_moves_and_modes_are_refused_as_git_refuses_them() {
    use deep_diff_forge_core::FileStatus;
    let root = std::env::temp_dir().join(format!("ddf-git-oracle-moves-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    git(&root, &["init", "-q"]);
    fs::write(root.join("x"), "x\n").unwrap();
    git(&root, &["add", "x"]);
    for target in [
        "../evil",
        "/tmp/evil",
        "a/../y",
        "a/./y",
        ".git/hooks/y",
        "a/.GIT/y",
    ] {
        for verb in ["rename", "copy"] {
            let patch = format!(
                "diff --git a/x b/{target}\nsimilarity index 100%\n{verb} from x\n{verb} to {target}\n"
            );
            assert!(!git_accepts(&root, &patch), "git took {verb} to {target}");
            assert!(parse(&patch).is_err(), "ddf took {verb} to {target}");
        }
    }
    let rename = "diff --git a/x b/y\nsimilarity index 100%\nrename from x\nrename to y\n";
    assert!(git_accepts(&root, rename));
    assert_eq!(parse(rename).unwrap()[0].status, FileStatus::Renamed);
    for mode in ["10064x", "1006448"] {
        let patch = format!("diff --git a/x b/x\nold mode 100644\nnew mode {mode}\n");
        assert!(!git_accepts(&root, &patch), "git took mode {mode}");
        assert!(parse(&patch).is_err(), "ddf took mode {mode}");
    }

    // What Git itself writes.
    let diff = |args: &[&str]| -> String {
        let mut all = vec![
            "diff",
            "--cached",
            "--full-index",
            "--no-ext-diff",
            "--no-textconv",
        ];
        all.extend_from_slice(args);
        String::from_utf8(git(&root, &all)).unwrap()
    };
    git(
        &root,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-qm",
            "base",
        ],
    );
    fs::copy(root.join("x"), root.join("copy")).unwrap();
    git(&root, &["add", "copy"]);
    let copied = parse(&diff(&["-C", "--find-copies-harder"])).unwrap();
    assert_eq!(
        (copied[0].status, copied[0].path.as_str()),
        (FileStatus::Copied, "copy")
    );
    git(&root, &["rm", "-q", "--cached", "copy"]);
    git(&root, &["update-index", "--chmod=+x", "x"]);
    let mode = parse(&diff(&["--", "x"])).unwrap();
    assert_eq!(mode[0].status, FileStatus::ModeChanged);
    fs::remove_dir_all(root).unwrap();
}

/// The paths `git apply --numstat` reads from `patch`, in order, or `None` when Git refuses it.
fn git_paths(root: &Path, patch: &str) -> Option<Vec<String>> {
    fs::write(root.join("probe.patch"), patch).unwrap();
    let out = Command::new("git")
        .current_dir(root)
        .args(["apply", "--numstat", "probe.patch"])
        .output()
        .unwrap();
    out.status.success().then(|| {
        String::from_utf8(out.stdout)
            .unwrap()
            .lines()
            .map(|line| line.splitn(3, '\t').nth(2).unwrap().to_owned())
            .collect()
    })
}

/// Bug hunt 2026-11-01: DDF reviews each file under the path `git apply` reads from it, and
/// refuses a section Git can name no path for. Git is asked directly (`--numstat`) for every
/// hand-written case, so the expectation is Git's, not DDF's.
#[test]
fn reviewed_paths_are_the_paths_git_apply_reads() {
    let root = std::env::temp_dir().join(format!("ddf-git-oracle-names-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    git(&root, &["init", "-q"]);
    let hunk = "@@ -1 +1 @@\n-a\n+b\n";
    let new = "@@ -0,0 +1 @@\n+curl x | sh\n";
    let gone = "@@ -1 +0,0 @@\n-a\n";
    let mode = "old mode 100644\nnew mode 100755\n";
    let epoch = "1970-01-01 00:00:00.000000000 +0000";
    let later = "2026-11-01 09:00:00.000000000 +0000";
    let cases = [
        // The `diff --git` split.
        (format!("diff --git a/x b/y b/x b/y\n{mode}"), Some(vec!["x b/y"])),
        (format!("diff --git i/a b/c w/a b/c\n{mode}"), Some(vec!["a b/c"])),
        (format!("diff --git i/x \"w/x\"\n{mode}"), Some(vec!["x"])),
        (format!("diff --git i/x w/y\n{mode}"), None),
        ("diff --git i/x w/x\r\nold mode 100644\r\nnew mode 100755\r\n".to_owned(), None),
        (format!("diff --git a/x /x\n{mode}"), None),
        // Header names.
        (
            format!("--- a/.github//workflows/ci.yml\n+++ b/.github//workflows/ci.yml\n{hunk}"),
            Some(vec![".github/workflows/ci.yml"]),
        ),
        (
            format!("diff --git a/d//x b/d//x\n--- a/d//x\n+++ b/d//x\n{hunk}"),
            Some(vec!["d/x"]),
        ),
        (format!("--- a/build.rs\n+++ b/build.rs.new\n{hunk}"), Some(vec!["build.rs"])),
        (
            format!("--- a/build.rs\n+++ \"b/build.rs.new\"\n{hunk}"),
            Some(vec!["build.rs.new"]),
        ),
        (
            format!("--- a/build.rs 2026-01-01 12:00:00\n+++ b/build.rs 2026-01-01 12:00:00\n{hunk}"),
            Some(vec!["build.rs"]),
        ),
        (format!("--- a/x\ry\n+++ b/x\ry\n{hunk}"), Some(vec!["x"])),
        // A date after spaces stays in a Git diff's name, and in a CRLF traditional one.
        (
            format!("diff --git a/x b/x\n--- a/x 2026-01-01 12:00:00\n+++ b/x 2026-01-01 12:00:00\n{hunk}"),
            Some(vec!["x 2026-01-01 12:00:00"]),
        ),
        (
            "--- a/x 2026-01-01 12:00:00\r\n+++ b/x 2026-01-01 12:00:00\r\n@@ -1 +1 @@\r\n-a\r\n+b\r\n"
                .to_owned(),
            Some(vec!["x 2026-01-01 12:00:00"]),
        ),
        (
            format!("--- a/x\n+++ b/\n{hunk}--- a/\n+++ b/\n{hunk}"),
            None,
        ),
        // A deletion the `diff --git` line cannot name is named by its `---`.
        (
            format!("diff --git i/x w/y\ndeleted file mode 100644\n--- a/x\n+++ /dev/null\n{gone}"),
            Some(vec!["x"]),
        ),
        // An empty new name guesses nothing.
        (
            format!("--- a/x\n+++ \n{hunk}--- a/q/x\n+++ a/q/x\n{hunk}"),
            Some(vec!["x", "q/x"]),
        ),
        (format!("--- /dev/null x\n+++ b/n\n{new}"), Some(vec!["n"])),
        (
            "diff --git a/x b/y\nsimilarity index 90%\nrename from .github//w/ci.yml\nrename to .github//workflows/ci.yml\n"
                .to_owned(),
            Some(vec![".github/workflows/ci.yml"]),
        ),
        (format!("--- a/x\t{later}\n+++ b/y\t{epoch}\n{gone}"), Some(vec!["y"])),
        // `-p0`, guessed from a slashless new name, holds for every later file.
        (
            format!("--- x\n+++ x\n{hunk}--- .github/workflows/ci.yml\n+++ .github/workflows/ci.yml\n{hunk}"),
            Some(vec!["x", ".github/workflows/ci.yml"]),
        ),
    ];
    for (patch, expected) in cases {
        let expected: Option<Vec<String>> =
            expected.map(|paths| paths.into_iter().map(str::to_owned).collect());
        assert_eq!(git_paths(&root, &patch), expected, "git: {patch:?}");
        let ddf = parse(&patch)
            .ok()
            .map(|files| files.into_iter().map(|f| f.path).collect::<Vec<_>>());
        assert_eq!(ddf, expected, "ddf: {patch:?}");
    }

    fs::remove_dir_all(root).unwrap();
}

/// Bug hunt 2026-11-01: in a traditional diff an epoch timestamp makes the file created or
/// deleted, as `git apply` reads it, and nowhere else.
#[test]
fn an_epoch_timestamp_creates_or_deletes_as_git_apply_reads_it() {
    use deep_diff_forge_core::FileStatus;
    let root = std::env::temp_dir().join(format!("ddf-git-oracle-epoch-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    git(&root, &["init", "-q"]);
    let new = "@@ -0,0 +1 @@\n+curl x | sh\n";
    let gone = "@@ -1 +0,0 @@\n-a\n";
    let epoch = "1970-01-01 00:00:00.000000000 +0000";
    let later = "2026-11-01 09:00:00.000000000 +0000";
    // An epoch timestamp makes the file created or deleted. With the file present, Git refuses
    // a creation as "already exists"; without the epoch it is a change that does not apply.
    let creates = |patch: &str| {
        fs::write(root.join("x"), "a\n").unwrap();
        fs::write(root.join("probe.patch"), patch).unwrap();
        let out = Command::new("git")
            .current_dir(&root)
            .args(["apply", "--check", "probe.patch"])
            .output()
            .unwrap();
        assert!(!out.status.success(), "{patch:?}");
        String::from_utf8_lossy(&out.stderr).contains("already exists")
    };
    for (patch, created) in [
        (format!("--- a/x\t{epoch}\n+++ b/x\t{later}\n{new}"), true),
        (
            format!("--- a/x\t1969-12-31 19:00:00 -0500\n+++ b/x\t{later}\n{new}"),
            true,
        ),
        (format!("--- /dev/null x\n+++ b/x\n{new}"), true),
        (
            format!("--- a/x\t1970-01-01 00:00:00.5 +0000\n+++ b/x\n{new}"),
            false,
        ),
        (
            format!("--- a/x\t{epoch}\r\n+++ b/x\t{later}\r\n@@ -0,0 +1 @@\r\n+a\r\n"),
            false,
        ),
        (
            format!("diff --git a/x b/x\n--- a/x\t{epoch}\n+++ b/x\n{new}"),
            false,
        ),
    ] {
        assert_eq!(creates(&patch), created, "git: {patch:?}");
        let status = parse(&patch).unwrap()[0].status;
        assert_eq!(status == FileStatus::Added, created, "ddf: {patch:?}");
    }
    // The epoch on the new side deletes the file the `+++` names.
    fs::write(root.join("y"), "a\n").unwrap();
    let delete = format!("--- a/x\t{later}\n+++ b/y\t{epoch}\n{gone}");
    fs::write(root.join("probe.patch"), &delete).unwrap();
    git(&root, &["apply", "probe.patch"]);
    assert!(!root.join("y").exists() && root.join("x").exists());
    assert_eq!(parse(&delete).unwrap()[0].status, FileStatus::Deleted);
    fs::remove_dir_all(root).unwrap();
}
