//! The repository's gate scripts fail closed: the pre-push secret hook, the diff-scoped mutation
//! gate (anchor B-01) and the fuzz runner. Each is run for real against a scratch repository,
//! with a recording stand-in for the expensive tool it hands off to (gitleaks, cargo mutants,
//! cargo fuzz), so what is checked is the script's own decision.
#![cfg(unix)]
#![expect(
    clippy::disallowed_methods,
    reason = "integration test: drives git, bash and stand-in tools, and reads PATH"
)]

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

const ISOLATE: [(&str, &str); 6] = [
    ("GIT_CONFIG_GLOBAL", "/dev/null"),
    ("GIT_CONFIG_NOSYSTEM", "1"),
    ("GIT_AUTHOR_NAME", "ddf"),
    ("GIT_AUTHOR_EMAIL", "ddf@example.invalid"),
    ("GIT_COMMITTER_NAME", "ddf"),
    ("GIT_COMMITTER_EMAIL", "ddf@example.invalid"),
];

struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn scratch(name: &str) -> Scratch {
    let root = std::env::temp_dir().join(format!("ddf-scripts-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    Scratch(root)
}

/// Writes an executable shell script.
fn tool(path: &Path, body: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, format!("#!/bin/sh\n{body}")).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// `PATH` with `bin` first, so its stand-ins shadow the real tools.
fn path_with(bin: &Path) -> String {
    let rest = std::env::var("PATH").unwrap_or_default();
    format!("{}:{rest}", bin.display())
}

fn run(cmd: &mut Command) -> Output {
    let out = cmd.envs(ISOLATE).output().unwrap();
    assert!(
        !String::from_utf8_lossy(&out.stderr).contains("command not found"),
        "{cmd:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = run(Command::new("git")
        .current_dir(dir)
        .args([
            "-c",
            "commit.gpgSign=false",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args));
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

fn commit(dir: &Path, file: &str, text: &str) {
    let path = dir.join(file);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-qm", file]);
}

// --- scripts/hooks/pre-push ---------------------------------------------------------------

/// A gitleaks stand-in with 8.30.1's behaviour: it scans `git log -p <log-opts>` for a
/// `ghp_` token and exits 1 on a find, but a range git cannot resolve is logged and reported as
/// a clean scan of 0 commits, exit 0.
const GITLEAKS: &str = r#"for a in "$@"; do case "$a" in --log-opts=*) range="${a#--log-opts=}";; esac; done
if ! out=$(git log -p $range 2>&1); then echo "fatal: $out; 0 commits scanned; no leaks found" >&2; exit 0; fi
if printf '%s' "$out" | grep -q 'ghp_'; then echo "leaks found: 1" >&2; exit 1; fi
echo "no leaks found" >&2
"#;

/// A bare remote whose `main` moved on from a commit that clone `a` never fetched; `a` has the
/// pre-push hook installed and the gitleaks stand-in on its PATH.
fn diverged_remote(s: &Scratch) -> (PathBuf, String) {
    let remote = s.0.join("remote.git");
    std::fs::create_dir_all(&remote).unwrap();
    git(&remote, &["init", "-q", "--bare"]);
    let (a, b) = (s.0.join("a"), s.0.join("b"));
    git(&s.0, &["clone", "-q", "remote.git", "a"]);
    commit(&a, "f", "hi\n");
    git(&a, &["push", "-q", "origin", "HEAD:main"]);
    git(&s.0, &["clone", "-q", "-b", "main", "remote.git", "b"]);
    commit(&b, "g", "other\n");
    git(&b, &["push", "-q", "origin", "main"]);
    let hook = a.join(".git/hooks/pre-push");
    std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
    std::fs::copy(format!("{ROOT}/scripts/hooks/pre-push"), &hook).unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    tool(&s.0.join("bin/gitleaks"), GITLEAKS);
    (a, path_with(&s.0.join("bin")))
}

fn push(repo: &Path, path: &str, args: &[&str]) -> Output {
    run(Command::new("git")
        .current_dir(repo)
        .env("PATH", path)
        .arg("push")
        .args(args))
}

#[test]
fn the_pre_push_hook_scans_a_force_push_over_a_remote_tip_never_fetched() {
    let s = scratch("prepush-secret");
    let (a, path) = diverged_remote(&s);
    let remote_tip = git(&s.0.join("b"), &["rev-parse", "HEAD"]);
    commit(
        &a,
        "cfg.py",
        // Built at run time, so no token-shaped literal sits in the repository (gitleaks).
        &format!("token = \"ghp_{}\"\n", "a1".repeat(18)),
    );
    // Control: a new branch is scanned against the remote-tracking refs and refused.
    let new_branch = push(&a, &path, &["origin", "HEAD:refs/heads/ctl"]);
    assert!(!new_branch.status.success(), "{new_branch:?}");
    // The remote tip is not in this clone, so `<tip>..HEAD` cannot be resolved; before the fix
    // the scan covered 0 commits, passed, and the token reached the remote.
    let forced = push(&a, &path, &["--force", "origin", "HEAD:main"]);
    let stderr = String::from_utf8_lossy(&forced.stderr);
    assert!(!forced.status.success(), "push accepted: {stderr}");
    assert!(stderr.contains("push refused"), "{stderr}");
    let on_remote = git(&s.0.join("remote.git"), &["rev-parse", "main"]);
    assert_eq!(on_remote, remote_tip, "the token reached the remote");
}

#[test]
fn the_pre_push_hook_lets_a_clean_force_push_over_an_unfetched_tip_through() {
    let s = scratch("prepush-clean");
    let (a, path) = diverged_remote(&s);
    commit(&a, "clean.txt", "nothing secret\n");
    let forced = push(&a, &path, &["--force", "origin", "HEAD:main"]);
    assert!(
        forced.status.success(),
        "{}",
        String::from_utf8_lossy(&forced.stderr)
    );
    assert_eq!(
        git(&s.0.join("remote.git"), &["rev-parse", "main"]),
        git(&a, &["rev-parse", "HEAD"])
    );
}

// --- scripts/diff_mutation.sh -------------------------------------------------------------

/// A repository whose base commit has one crate file, and the cargo stand-in that records the
/// arguments `cargo mutants` would have been given.
fn mutation_repo(s: &Scratch) -> (PathBuf, String) {
    let repo = s.0.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    commit(&repo, "crates/x/src/lib.rs", "fn a() {}\n");
    let log = s.0.join("cargo.log");
    tool(
        &s.0.join("bin/cargo"),
        &format!("echo \"$@\" >> '{}'\n", log.display()),
    );
    (repo, path_with(&s.0.join("bin")))
}

fn diff_mutation(dir: &Path, path: &str, out: &Path) -> Output {
    let out = run(Command::new("bash")
        .current_dir(dir)
        .env("PATH", path)
        .env("DDF_MUTATION_OUT", out)
        .args([&format!("{ROOT}/scripts/diff_mutation.sh"), "HEAD~1", "0/4"]));
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

fn mutants_runs(s: &Scratch) -> usize {
    std::fs::read_to_string(s.0.join("cargo.log"))
        .unwrap_or_default()
        .lines()
        .filter(|line| line.starts_with("mutants --no-config --in-diff "))
        .count()
}

#[test]
fn diff_mutation_tests_rust_files_whose_names_git_quotes_or_tabs() {
    let s = scratch("mutation-names");
    let (repo, path) = mutation_repo(&s);
    std::fs::write(repo.join("crates/x/src/é.rs"), "fn b() -> i32 { 1 + 2 }\n").unwrap();
    commit(&repo, "crates/x/src/sp ace.rs", "fn c() -> i32 { 3 }\n");
    let out = s.0.join("out");
    diff_mutation(&repo, &path, &out);
    assert_eq!(mutants_runs(&s), 1, "cargo mutants was not run");
    // cargo mutants refuses a C-quoted `+++ "b/..."` header outright (measured, 27.1.0).
    let diff = std::fs::read_to_string(out.join("pr.diff")).unwrap();
    assert!(diff.contains("\n+++ b/crates/x/src/é.rs\n"), "{diff}");
}

#[test]
fn diff_mutation_gives_the_same_answer_from_a_subdirectory() {
    let s = scratch("mutation-subdir");
    let (repo, path) = mutation_repo(&s);
    commit(&repo, "crates/x/src/normal.rs", "fn n() -> i32 { 4 }\n");
    // A relative output directory is the caller's, not the repository root's.
    let sub = repo.join("crates/x");
    diff_mutation(&sub, &path, Path::new("rel-out"));
    assert_eq!(mutants_runs(&s), 1, "cargo mutants was not run");
    assert!(sub.join("rel-out/pr.diff").is_file());
}

#[test]
fn diff_mutation_without_rust_changes_passes_without_mutating() {
    let s = scratch("mutation-none");
    let (repo, path) = mutation_repo(&s);
    commit(&repo, "crates/x/README.md", "docs\n");
    let out = diff_mutation(&repo, &path, &s.0.join("out"));
    assert_eq!(mutants_runs(&s), 0);
    assert!(String::from_utf8_lossy(&out.stdout).contains("no Rust changes under crates/"));
    git(&repo, &["rm", "-q", "crates/x/src/lib.rs"]);
    git(&repo, &["commit", "-qm", "delete"]);
    diff_mutation(&repo, &path, &s.0.join("out"));
    assert_eq!(mutants_runs(&s), 0, "a deleted file has no lines to mutate");
}

// --- scripts/fuzz_target.sh ---------------------------------------------------------------

#[test]
fn fuzz_list_reads_every_bin_however_its_table_is_laid_out() {
    let s = scratch("fuzz-list");
    std::fs::create_dir_all(s.0.join("scripts")).unwrap();
    std::fs::create_dir_all(s.0.join("fuzz")).unwrap();
    std::fs::copy(
        format!("{ROOT}/scripts/fuzz_target.sh"),
        s.0.join("scripts/fuzz_target.sh"),
    )
    .unwrap();
    let manifest = "[package]\nname = \"f\"\n\n[[bin]]\nname = \"plain\"\npath = \"p.rs\"\n\n\
        [[bin]]\npath = \"a.rs\"\nname = \"path_first\"\n\n[[bin]]\nname=\"no_spaces\"\n\n\
        [[bin]]\n# a comment\nname = 'single' # trailing\n";
    std::fs::write(s.0.join("fuzz/Cargo.toml"), manifest).unwrap();
    let out = run(Command::new("bash").args([
        s.0.join("scripts/fuzz_target.sh").to_str().unwrap(),
        "--list",
    ]));
    assert!(out.status.success(), "{out:?}");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .collect::<Vec<_>>(),
        ["plain", "path_first", "no_spaces", "single"]
    );
}

#[test]
fn fuzz_list_matches_every_bin_header_of_the_real_manifest() {
    let manifest = std::fs::read_to_string(format!("{ROOT}/fuzz/Cargo.toml")).unwrap();
    let headers = manifest.lines().filter(|l| l.trim() == "[[bin]]").count();
    let out = run(Command::new("bash").args([&format!("{ROOT}/scripts/fuzz_target.sh"), "--list"]));
    assert!(out.status.success(), "{out:?}");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).lines().count(),
        headers
    );
    assert!(
        headers >= 10,
        "only {headers} [[bin]] tables: the count is broken"
    );
}

#[test]
fn fuzz_runs_on_a_fresh_private_corpus_and_never_writes_through_a_planted_one() {
    let s = scratch("fuzz-corpus");
    let root = s.0.join("root");
    let planted = root.join("ddf-patch_parser-corpus");
    std::fs::create_dir_all(&planted).unwrap();
    let victim = s.0.join("victim.txt");
    std::fs::write(&victim, "precious\n").unwrap();
    std::os::unix::fs::symlink(&victim, planted.join("basic.patch")).unwrap();
    std::fs::write(planted.join("attacker-input"), "planted\n").unwrap();
    // The cargo stand-in records the corpus it was given (argument 5) and what is in it, then
    // fails as cargo fuzz does on a crash.
    let log = s.0.join("corpus.log");
    tool(
        &s.0.join("bin/cargo"),
        &format!(
            "echo \"$5\" > '{0}'; ls -A \"$5\" >> '{0}'; stat -c %a \"$5\" 2>/dev/null >> '{0}' || stat -f %Lp \"$5\" >> '{0}'; exit 3\n",
            log.display()
        ),
    );
    let out = run(Command::new("bash")
        .env("PATH", path_with(&s.0.join("bin")))
        .env("DDF_FUZZ_CORPUS_ROOT", &root)
        .env_remove("RUNNER_TEMP")
        .args([
            &format!("{ROOT}/scripts/fuzz_target.sh"),
            "patch_parser",
            "1",
        ]));
    assert_eq!(
        out.status.code(),
        Some(3),
        "cargo fuzz's status is the script's: {out:?}"
    );
    assert_eq!(std::fs::read_to_string(&victim).unwrap(), "precious\n");
    let logged = std::fs::read_to_string(&log).unwrap();
    let mut lines = logged.lines();
    let corpus = PathBuf::from(lines.next().unwrap());
    assert_ne!(corpus, planted);
    assert!(corpus.starts_with(&root), "{}", corpus.display());
    let mode = lines.next_back().unwrap();
    assert_eq!(mode, "700", "the corpus is private");
    let mut seeded: Vec<_> = lines.map(str::to_owned).collect();
    seeded.sort();
    let mut fixtures: Vec<_> = std::fs::read_dir(format!("{ROOT}/fixtures/patch"))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        // The script's `*.patch` glob is case-sensitive, and so is this.
        .filter(|name| {
            Path::new(name)
                .extension()
                .is_some_and(|ext| ext == "patch")
        })
        .collect();
    fixtures.sort();
    assert_eq!(
        seeded, fixtures,
        "the corpus holds the committed seeds only"
    );
    assert!(!corpus.exists(), "the corpus is removed after the run");
}

#[test]
fn fuzz_creates_a_corpus_root_that_does_not_exist_yet() {
    // Before the private corpus, `mkdir -p` made the whole path; `mktemp -d` alone makes only
    // the last component, so a root not yet made failed the run before cargo fuzz started.
    let s = scratch("fuzz-root");
    let root = s.0.join("not/yet/made");
    let log = s.0.join("corpus.log");
    tool(
        &s.0.join("bin/cargo"),
        &format!("echo \"$5\" > '{}'\n", log.display()),
    );
    let out = run(Command::new("bash")
        .env("PATH", path_with(&s.0.join("bin")))
        .env("DDF_FUZZ_CORPUS_ROOT", &root)
        .env_remove("RUNNER_TEMP")
        .args([
            &format!("{ROOT}/scripts/fuzz_target.sh"),
            "patch_parser",
            "1",
        ]));
    assert!(out.status.success(), "{out:?}");
    let corpus = PathBuf::from(std::fs::read_to_string(&log).unwrap().trim());
    assert!(corpus.starts_with(&root), "{}", corpus.display());
    assert!(root.is_dir(), "the root is kept");
    assert!(!corpus.exists(), "the corpus is removed after the run");
}
