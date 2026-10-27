// DDF integration: R02, R03, R07. CLI contract for structured Git selections, exercised through the compiled binary against a real temporary repository.
// Design/status: docs/RECOMMENDATION_INTEGRATION.md#patch-truth
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fdeep-diff-forge-cli--tests--git_input
//! `--git-working-tree`, `--git-staged`, `--git-range`: flag conflicts are
//! usage errors, outputs reuse the `--stdin-patch` documents, and paths come
//! from raw records.
#![cfg(unix)]
#![expect(
    clippy::disallowed_methods,
    reason = "integration test: drives real processes, the environment and wall-clock time"
)]

use std::path::{Path, PathBuf};
use std::process::Command;

const ISOLATE: [(&str, &str); 2] = [
    ("GIT_CONFIG_GLOBAL", "/dev/null"),
    ("GIT_CONFIG_NOSYSTEM", "1"),
];

fn git(root: &Path, args: &[&str]) {
    let out = Command::new("git")
        .current_dir(root)
        .envs(ISOLATE)
        .args([
            "-c",
            "user.name=ddf",
            "-c",
            "user.email=ddf@example.invalid",
        ])
        .args(["-c", "commit.gpgSign=false"])
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn ddf(args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .envs(ISOLATE)
        .args(args)
        .output()
        .unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

struct Repo(PathBuf);

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

impl Repo {
    fn arg(&self) -> &str {
        self.0.to_str().unwrap()
    }
}

/// Committed base, then a Unicode edit, a mode-only change under `x b/y`, and
/// an untracked file.
fn repo(name: &str, dirty: bool) -> Repo {
    use std::os::unix::fs::PermissionsExt as _;
    let root = std::env::temp_dir().join(format!("ddf-cli-git-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("x b")).unwrap();
    let r = Repo(root);
    git(&r.0, &["init", "-q"]);
    std::fs::write(r.0.join("café.txt"), b"old\n").unwrap();
    std::fs::write(r.0.join("x b/y"), b"y\n").unwrap();
    std::fs::set_permissions(r.0.join("x b/y"), std::fs::Permissions::from_mode(0o644)).unwrap();
    git(&r.0, &["add", "-A"]);
    git(&r.0, &["commit", "-q", "-m", "base"]);
    if dirty {
        std::fs::write(r.0.join("café.txt"), b"new\n").unwrap();
        std::fs::set_permissions(r.0.join("x b/y"), std::fs::Permissions::from_mode(0o755))
            .unwrap();
        std::fs::write(r.0.join("untracked.txt"), b"u\n").unwrap();
    }
    r
}

fn json(text: &str) -> serde_json::Value {
    serde_json::from_str(text).unwrap_or_else(|e| panic!("{e}: {text}"))
}

/// Negative control: without the explicit check these combinations would run
/// one input source and silently ignore the other flag.
#[test]
fn conflicting_input_flags_are_usage_errors() {
    for args in [
        &["--stdin-patch", "--git-staged"][..],
        &["--stdin-patch", "--json", "--git-working-tree"],
        &["--git-staged", "--stdin-patch"],
        &["--git-staged", "--git-working-tree"],
        &["--git-range"],
        &["--git-range", "--json"],
        &["--git-range", "HEAD"],
        &["--git-staged", "--include-untracked"],
        &["--git-working-tree", "--repo"],
    ] {
        let (code, stdout, stderr) = ddf(args);
        assert_eq!(code, 2, "{args:?}: {stderr}");
        assert!(stdout.is_empty(), "{args:?}");
        assert!(stderr.starts_with("error: "), "{args:?}: {stderr}");
    }
}

/// Negative control: if `--git-working-tree` were display-header or stdin
/// capture, the untracked file would be missing and `x b/y` would be named `y`.
#[test]
fn working_tree_json_names_raw_paths_and_includes_untracked() {
    let r = repo("wt", true);
    let (code, stdout, stderr) = ddf(&[
        "--git-working-tree",
        "--include-untracked",
        "--repo",
        r.arg(),
        "--json",
    ]);
    assert_eq!(code, 0, "{stderr}");
    let doc = json(&stdout);
    assert_eq!(doc["schema"], "deep-diff-forge.review.v0");
    let mut paths: Vec<&str> = doc["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    paths.sort_unstable();
    assert_eq!(paths, ["café.txt", "untracked.txt", "x b/y"]);

    // The acquisition document seals the same bytes as the review document.
    let (code, meta, stderr) = ddf(&[
        "--git-working-tree",
        "--include-untracked",
        "--repo",
        r.arg(),
        "--acquisition-json",
    ]);
    assert_eq!(code, 0, "{stderr}");
    let meta = json(&meta);
    assert_eq!(meta["schema"], "deep-diff-forge.git-acquisition.v0");
    assert_eq!(meta["input_sha256"], doc["input_sha256"]);
    let spacey = meta["changes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["new_path"]["utf8"] == "x b/y")
        .unwrap();
    assert_eq!(spacey["new_path"]["hex"], "7820622f79");
    assert_eq!(
        (spacey["old_mode"].as_str(), spacey["new_mode"].as_str()),
        (Some("100644"), Some("100755"))
    );
    let untracked = meta["changes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["origin"] == "untracked")
        .unwrap();
    assert_eq!(untracked["status"], "added");

    // Without --include-untracked the file is absent; rank uses the same files.
    let (code, ranked, _) = ddf(&["--git-working-tree", "--repo", r.arg(), "--rank", "--json"]);
    assert_eq!(code, 0);
    let ranked = json(&ranked);
    assert_eq!(ranked["schema"], "deep-diff-forge.rank.v0");
    assert_eq!(ranked["ranked"].as_array().unwrap().len(), 2);
}

/// Negative control: without the guards a clean tree would look like a
/// zero-risk review.
#[test]
fn clean_tree_with_require_hunks_is_refused() {
    let r = repo("clean", false);
    let (code, stdout, stderr) = ddf(&["--git-working-tree", "--repo", r.arg(), "--require-hunks"]);
    assert_eq!(code, 7, "{stderr}");
    assert_eq!(stdout, "");
    let (code, stdout, _) = ddf(&["--git-staged", "--repo", r.arg()]);
    assert_eq!(code, 0);
    assert!(stdout.contains("0 file(s) changed"));
}

#[test]
fn range_selection_reviews_commits_and_bad_revisions_fail() {
    let r = repo("range", true);
    git(&r.0, &["add", "-A"]);
    git(&r.0, &["commit", "-q", "-m", "candidate"]);
    let (code, stdout, stderr) = ddf(&["--git-range", "HEAD~1..HEAD", "--repo", r.arg()]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("3 file(s) changed"), "{stdout}");
    let (code, _, stderr) = ddf(&["--git-range", "HEAD..nope", "--repo", r.arg()]);
    assert_eq!(code, 2, "{stderr}");
}

#[test]
fn outside_a_repository_is_an_acquisition_failure() {
    let dir = Repo(std::env::temp_dir().join(format!("ddf-cli-git-none-{}", std::process::id())));
    std::fs::create_dir_all(&dir.0).unwrap();
    // Stop discovery at the temporary directory so an enclosing repo cannot leak in.
    let out = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .envs(ISOLATE)
        .env("GIT_CEILING_DIRECTORIES", std::env::temp_dir())
        .args(["--git-staged", "--repo", dir.arg()])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(3), "{stderr}");
    assert!(stdout.is_empty());
    assert!(stderr.contains("structured Git acquisition failed"));
}
