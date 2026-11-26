//! The repository's gate scripts fail closed: the pre-push secret hook, the diff-scoped mutation
//! gate (anchor B-01), the fuzz runner and the release-tag signer; the local gate's step runner,
//! the gate's test step and the macOS-like temp directory are run as well. Each is run for real against a scratch repository,
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

/// A scratch directory's path and its guard, which removes it when dropped.
struct Scratch(
    PathBuf,
    #[expect(dead_code, reason = "held for its Drop, which removes the directory")]
    deep_diff_forge_testkit::Scratch,
);

fn scratch(name: &str) -> Scratch {
    let guard = deep_diff_forge_testkit::Scratch::new(&format!("scripts-{name}"));
    Scratch(guard.path().to_owned(), guard)
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
    // `ISOLATE` keeps out the global and system files, but configuration also arrives in the
    // environment: `git -c` exports `GIT_CONFIG_PARAMETERS`, and `just release-tag` is run
    // with the signing keys in `GIT_CONFIG_COUNT`. Inherited, `tag.gpgSign=true` turned the
    // lightweight tag below into an annotated one that asked for an editor. A command that
    // sets its own `GIT_CONFIG_COUNT` (`sign_tag`) keeps it.
    if !cmd.get_envs().any(|(key, _)| key == "GIT_CONFIG_COUNT") {
        cmd.env_remove("GIT_CONFIG_COUNT");
    }
    let out = cmd
        .env_remove("GIT_CONFIG_PARAMETERS")
        .envs(ISOLATE)
        .output()
        .unwrap();
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

/// C1 of 0.11.1: the mutants' timeout comes from the baseline, not a fixed `--timeout`, which
/// bounds the unmutated baseline too (measured, cargo-mutants 27.1.0: "TIMEOUT Unmutated
/// baseline", exit 4) while the whole workspace's tests take about 35 minutes on CI's runner
/// (2,043 s and 2,106 s, the gate job of runs 45701563166 and 45670721783): `--timeout 300`
/// failed every shard before a mutant was tested. The floor is not under that baseline.
#[test]
fn diff_mutation_derives_the_timeout_from_the_baseline() {
    let s = scratch("mutation-timeout");
    let (repo, path) = mutation_repo(&s);
    commit(&repo, "crates/x/src/lib.rs", "fn a() -> i32 { 1 }\n");
    diff_mutation(&repo, &path, &s.0.join("out"));
    let log = std::fs::read_to_string(s.0.join("cargo.log")).unwrap();
    let args: Vec<&str> = log
        .lines()
        .find(|line| line.starts_with("mutants --no-config --in-diff "))
        .expect("cargo mutants was not run")
        .split_whitespace()
        .collect();
    let value = |flag: &str| -> Option<f64> {
        let at = args.iter().position(|arg| *arg == flag)?;
        args.get(at + 1)?.parse().ok()
    };
    assert!(
        !args
            .iter()
            .any(|arg| ["--timeout", "-t"].contains(arg) || arg.starts_with("--timeout=")),
        "a fixed --timeout also cuts the baseline: {args:?}"
    );
    assert!(
        value("--timeout-multiplier").is_some_and(|multiplier| multiplier >= 1.0),
        "{args:?}"
    );
    assert!(
        value("--minimum-test-timeout").is_some_and(|floor| floor >= 2106.0),
        "the floor must not be under the baseline measured on CI's runner: {args:?}"
    );
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
    // Every `*.patch` under fixtures/patch, regressions included (the script's `find -name` is
    // case-sensitive, and so is this).
    let mut fixtures = Vec::new();
    let mut pending = vec![PathBuf::from(format!("{ROOT}/fixtures/patch"))];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|ext| ext == "patch") {
                fixtures.push(path.file_name().unwrap().to_str().unwrap().to_owned());
            }
        }
    }
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

#[test]
fn fuzz_refuses_to_choose_a_corpus_root_itself() {
    // It fell back to /tmp, which here is a tmpfs with an inode limit, lost at power-off.
    let s = scratch("fuzz-noroot");
    tool(&s.0.join("bin/cargo"), "exit 0\n");
    let out = run(Command::new("bash")
        .env("PATH", path_with(&s.0.join("bin")))
        .env_remove("DDF_FUZZ_CORPUS_ROOT")
        .env_remove("RUNNER_TEMP")
        .args([
            &format!("{ROOT}/scripts/fuzz_target.sh"),
            "patch_parser",
            "1",
        ]));
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stderr).contains("DDF_FUZZ_CORPUS_ROOT"));
}

// --- scripts/campaign_local.sh ------------------------------------------------------------

/// Runs the campaign for 1 s a target with a cargo stand-in that logs each fuzz run's target to
/// `<scratch>/runs.log` and fails, as cargo fuzz does on a crash, for the target `crash`.
fn campaign(s: &Scratch, crash: &str) -> (Output, Vec<String>, String) {
    let runs = s.0.join("runs.log");
    tool(
        &s.0.join("bin/cargo"),
        &format!(
            "[ \"$2\" = fuzz ] || {{ echo cargo-stand-in; exit 0; }}\necho \"$4 $7\" >> '{}'\n[ \"$4\" != '{crash}' ] || exit 77\n",
            runs.display()
        ),
    );
    let out = run(Command::new("bash")
        .env("PATH", path_with(&s.0.join("bin")))
        .env("DDF_FUZZ_CORPUS_ROOT", s.0.join("root"))
        .args([&format!("{ROOT}/scripts/campaign_local.sh"), "1"]));
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let receipt = stdout
        .lines()
        .find_map(|line| line.strip_prefix("receipt: "))
        .map(|path| std::fs::read_to_string(path).unwrap())
        .unwrap_or_default();
    let logged = std::fs::read_to_string(&runs).unwrap_or_default();
    (out, logged.lines().map(str::to_owned).collect(), receipt)
}

#[test]
fn campaign_runs_every_listed_target_and_fails_on_one_crash() {
    let list =
        run(Command::new("bash").args([&format!("{ROOT}/scripts/fuzz_target.sh"), "--list"]));
    let targets: Vec<String> = String::from_utf8_lossy(&list.stdout)
        .lines()
        .map(str::to_owned)
        .collect();
    assert!(targets.len() >= 10, "{list:?}");

    let s = scratch("campaign-green");
    let (out, runs, receipt) = campaign(&s, "no-such-target");
    assert!(out.status.success(), "{out:?}");
    let each: Vec<String> = targets
        .iter()
        .map(|t| format!("{t} -max_total_time=1"))
        .collect();
    assert_eq!(
        runs, each,
        "every listed target once, in order, for the time given"
    );
    for t in &targets {
        assert!(
            receipt.contains(&format!("run: {t} rc=0 ")),
            "{t}: {receipt}"
        );
    }
    let commit = run(Command::new("git").args(["-C", ROOT, "rev-parse", "HEAD"]));
    for line in [
        format!("commit: {}", String::from_utf8_lossy(&commit.stdout).trim()),
        "toolchain: nightly-2026-10-25: cargo-stand-in".to_owned(),
        "seconds per target: 1".to_owned(),
        format!("result: GREEN, {} targets clean", targets.len()),
    ] {
        assert!(receipt.contains(&line), "{line}: {receipt}");
    }

    // One target crashes: the campaign stops there, red, and names it.
    let crash = &targets[targets.len() / 2];
    let s = scratch("campaign-red");
    let (out, runs, receipt) = campaign(&s, crash);
    assert_eq!(
        out.status.code(),
        Some(77),
        "the crash is the campaign's status: {out:?}"
    );
    assert_eq!(runs, each[..=targets.len() / 2], "it stops at the crash");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains(&format!("fuzz target {crash} failed")),
        "{out:?}"
    );
    assert!(
        receipt.contains(&format!("result: RED at {crash} (rc 77)")),
        "{receipt}"
    );

    let justfile = std::fs::read_to_string(format!("{ROOT}/justfile")).unwrap();
    assert!(
        justfile.contains(
            "[group(\"release\")]\nfuzz-campaign seconds=\"300\":\n    scripts/campaign_local.sh "
        ),
        "the campaign is a release recipe"
    );
}

/// macOS's `/bin/bash` is 3.2: `mapfile` was "command not found" there and the target list then
/// unbound under `set -u` (the 0.11.0 macOS CI run). A run on Linux cannot show that, so the
/// script's code (its comments aside) is read for what bash 3.2 lacks.
#[test]
fn the_campaign_uses_nothing_bash_3_2_lacks() {
    let script = std::fs::read_to_string(format!("{ROOT}/scripts/campaign_local.sh")).unwrap();
    let code: Vec<&str> = script
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .collect();
    assert!(code.len() > 40, "{}", code.len());
    for newer in [
        "mapfile",
        "readarray",
        "declare -A",
        "declare -n",
        "local -A",
        "local -n",
        ",,}",
        "^^}",
        "|&",
        "&>>",
        "coproc",
        "globstar",
        ";;&",
        "EPOCHREALTIME",
        "EPOCHSECONDS",
        "wait -n",
        "[-1]",
        "@Q}",
    ] {
        let found: Vec<&&str> = code.iter().filter(|line| line.contains(newer)).collect();
        assert!(
            found.is_empty(),
            "{newer:?}, which bash 3.2 lacks: {found:?}"
        );
    }
}

// --- scripts/gate_steps.sh (the step runner of scripts/ci_local.sh) -------------------------

/// P7: each step's line carries its time, and what it skipped in one form; a failed step shows
/// the end of its output; the receipt lists every step with its status, seconds and skip.
#[test]
fn the_gate_says_how_long_each_step_took_and_what_it_skipped() {
    let s = scratch("gate-steps");
    let out = run(Command::new("bash")
        .current_dir(&s.0)
        .args([
            "-c",
            r#"gate_steps="$1/steps.tsv"
. "$2/scripts/gate_steps.sh"
step "quick" true
step "slow" sleep 1
step "corpus" sh -c 'echo working; echo "skipped: no corpus here (c)"'
step "tests" printf 'test result: ok. 3 passed; 0 failed; 2 ignored; 0 measured\ntest result: ok. 1 passed; 0 failed; 1 ignored; 0 measured\n'
step "matrix" printf 'real rows: 0 run, 12 skipped\nskipped: 12 rows (12 no FW_BIN)\nrows red: 0\n'
step "red" sh -c 'echo one; echo two; exit 3'
gate_receipt "$1/receipt.json" 0123abc
echo "failed: $failed""#,
            "bash",
        ])
        .arg(&s.0)
        .arg(ROOT));
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(out.status.success(), "{out:?}");
    // `<status> <name> (<seconds> s<; what it said>)`: the seconds and the rest
    let line = |status: &str, name: &str| -> (f64, String) {
        let head = format!("{status} {name} (");
        let line = stdout
            .lines()
            .find(|line| line.starts_with(&head))
            .unwrap_or_else(|| panic!("no line {head}: {stdout}"));
        let inner = line[head.len()..].strip_suffix(')').unwrap();
        let (seconds, rest) = inner.split_once(" s").unwrap();
        (seconds.parse().unwrap(), rest.to_owned())
    };
    // (bash 3.2, macOS's, has no EPOCHREALTIME: the clock is then whole seconds, and a step
    // that crosses a second's edge takes 1.0)
    let (quick, said) = line("PASS", "quick");
    let (slow, _) = line("PASS", "slow");
    assert!((1.0..30.0).contains(&slow), "{stdout}");
    assert!(quick <= slow && said.is_empty(), "{stdout}");
    assert_eq!(line("PASS", "corpus").1, "; skipped: no corpus here (c)");
    assert_eq!(line("PASS", "tests").1, "; skipped: 3 tests ignored");
    assert_eq!(
        line("PASS", "matrix").1,
        "; real rows: 0 run, 12 skipped; skipped: 12 rows (12 no FW_BIN)"
    );
    assert_eq!(line("FAIL", "red").1, "");
    assert!(stdout.contains("\n    one\n    two\n"), "{stdout}");
    assert!(stdout.ends_with("failed: 1\n"), "{stdout}");

    let receipt: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(s.0.join("receipt.json")).unwrap()).unwrap();
    assert_eq!(receipt["commit"], "0123abc");
    assert_eq!(receipt["failed"], 1);
    assert_eq!(receipt["uncommitted_changes"], false);
    assert!(
        receipt["started"].as_str().unwrap().ends_with('Z'),
        "{receipt}"
    );
    let steps = receipt["steps"].as_array().unwrap();
    let names: Vec<&str> = steps.iter().map(|s| s["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["quick", "slow", "corpus", "tests", "matrix", "red"]);
    let status: Vec<&str> = steps
        .iter()
        .map(|s| s["status"].as_str().unwrap())
        .collect();
    assert_eq!(status, ["PASS", "PASS", "PASS", "PASS", "PASS", "FAIL"]);
    assert!(
        steps[1]["seconds"]
            .as_f64()
            .is_some_and(|seconds| (seconds - slow).abs() < 1e-9),
        "{receipt}"
    );
    let skipped: Vec<Option<&str>> = steps.iter().map(|s| s["skipped"].as_str()).collect();
    assert_eq!(
        skipped,
        [
            None,
            None,
            Some("no corpus here (c)"),
            Some("3 tests ignored"),
            Some("12 rows (12 no FW_BIN)"),
            None
        ]
    );
}

// --- scripts/long_tmpdir.sh ---------------------------------------------------------------

/// The macOS-like temp directory: a link of the length asked for to a real, private directory
/// of the longer length, both directly under the base; 49 and 57 by default where the base
/// leaves room (a base under `/tmp` does; one under a longer TMPDIR is refused, saying why).
#[test]
fn long_tmpdir_makes_a_link_as_long_as_macos_s_to_a_longer_real_directory() {
    let s = scratch("long-tmpdir");
    let base = std::fs::canonicalize(&s.0).unwrap();
    let n = base.as_os_str().len();
    let long_tmpdir = |lengths: Option<(usize, usize)>| {
        let mut command = Command::new("bash");
        command
            .arg(format!("{ROOT}/scripts/long_tmpdir.sh"))
            .arg(&s.0)
            .env_remove("LONG_TMPDIR_LINK")
            .env_remove("LONG_TMPDIR_REAL");
        if let Some((link, real)) = lengths {
            command
                .env("LONG_TMPDIR_LINK", link.to_string())
                .env("LONG_TMPDIR_REAL", real.to_string());
        }
        run(&mut command)
    };
    let link_made = |out: &Output, link_len: usize, real_len: usize| -> PathBuf {
        assert!(out.status.success(), "{out:?}");
        let link = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim_end());
        let real = std::fs::canonicalize(&link).unwrap();
        assert!(
            std::fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink(),
            "{link:?}"
        );
        assert_eq!(
            (link.as_os_str().len(), real.as_os_str().len()),
            (link_len, real_len),
            "{link:?} -> {real:?}"
        );
        assert_eq!(
            (link.parent(), real.parent()),
            (Some(base.as_path()), Some(base.as_path()))
        );
        let mode = std::fs::metadata(&real).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700, "{real:?}");
        link
    };
    let first = link_made(&long_tmpdir(Some((n + 16, n + 24))), n + 16, n + 24);
    let second = link_made(&long_tmpdir(Some((n + 16, n + 24))), n + 16, n + 24);
    assert_ne!(first, second, "a second run made the same link");
    // the defaults, as macOS has them
    let out = long_tmpdir(None);
    if n + 3 <= 49 {
        link_made(&out, 49, 57);
    } else {
        assert_eq!(out.status.code(), Some(2), "{out:?}");
        assert!(
            String::from_utf8_lossy(&out.stderr).contains(&format!(
                "is {n} characters: too long for a link of 49 and a directory of 57"
            )),
            "{out:?}"
        );
    }
    // no room for the names, a real path shorter than the link, a base that is no directory
    for (lengths, base) in [
        (Some((n + 2, n + 24)), s.0.clone()),
        (Some((n + 24, n + 16)), s.0.clone()),
        (None, s.0.join("absent")),
    ] {
        let mut command = Command::new("bash");
        command
            .arg(format!("{ROOT}/scripts/long_tmpdir.sh"))
            .arg(&base);
        if let Some((link, real)) = lengths {
            command
                .env("LONG_TMPDIR_LINK", link.to_string())
                .env("LONG_TMPDIR_REAL", real.to_string());
        }
        let out = run(&mut command);
        assert_eq!(out.status.code(), Some(2), "{lengths:?} {base:?}: {out:?}");
        assert!(out.stdout.is_empty(), "{out:?}");
    }
}

// --- scripts/sign_release_tag.sh ----------------------------------------------------------

/// A repository with one commit and a fresh SSH key at `<scratch>/key`; `.github/allowed_signers`
/// lists that key when `trusted`, and another fresh key otherwise.
fn signing_repo(s: &Scratch, trusted: bool) -> PathBuf {
    let repo = s.0.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    commit(&repo, "f", "hi\n");
    for key in ["key", "other"] {
        let out = run(Command::new("ssh-keygen")
            .args(["-q", "-t", "ed25519", "-N", "", "-f"])
            .arg(s.0.join(key)));
        assert!(out.status.success(), "{out:?}");
    }
    let listed =
        std::fs::read_to_string(s.0.join(if trusted { "key.pub" } else { "other.pub" })).unwrap();
    std::fs::create_dir_all(repo.join(".github")).unwrap();
    std::fs::write(
        repo.join(".github/allowed_signers"),
        format!("ddf@example.invalid namespaces=\"git\" {listed}"),
    )
    .unwrap();
    repo
}

/// Runs the script in `repo`, signing `v1.2.3` with `<scratch>/key`.
fn sign_tag(s: &Scratch, repo: &Path) -> Output {
    sign_version(s, repo, "1.2.3")
}

/// Runs the script in `repo`, signing `v<version>` at HEAD with `<scratch>/key`.
fn sign_version(s: &Scratch, repo: &Path, version: &str) -> Output {
    run(Command::new("bash")
        .current_dir(repo)
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "user.signingkey")
        .env("GIT_CONFIG_VALUE_0", s.0.join("key.pub"))
        .args([&format!("{ROOT}/scripts/sign_release_tag.sh"), version]))
}

fn tags(repo: &Path) -> String {
    git(repo, &["tag", "--list"])
}

#[test]
fn a_release_tag_signed_by_an_allowed_key_is_kept_and_verifies() {
    let s = scratch("sign-ok");
    let repo = signing_repo(&s, true);
    let out = sign_tag(&s, &repo);
    assert!(out.status.success(), "{out:?}");
    assert_eq!(tags(&repo), "v1.2.3");
    let tag = git(&repo, &["cat-file", "-p", "v1.2.3"]);
    assert!(tag.contains("-----BEGIN SSH SIGNATURE-----"), "{tag}");
}

#[test]
fn a_release_tag_signed_by_a_key_not_in_allowed_signers_is_deleted() {
    let s = scratch("sign-untrusted");
    let repo = signing_repo(&s, false);
    let out = sign_tag(&s, &repo);
    assert!(!out.status.success(), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .contains("not signed by a key in .github/allowed_signers"),
        "{out:?}"
    );
    assert_eq!(
        tags(&repo),
        "",
        "the unverified tag must not survive to be pushed"
    );
}

#[test]
fn no_release_tag_is_made_without_allowed_signers() {
    let s = scratch("sign-none");
    let repo = signing_repo(&s, true);
    std::fs::remove_file(repo.join(".github/allowed_signers")).unwrap();
    let out = sign_tag(&s, &repo);
    assert!(!out.status.success(), "{out:?}");
    assert_eq!(tags(&repo), "");
}

#[test]
fn the_release_recipe_tags_only_through_the_signing_script() {
    let justfile = std::fs::read_to_string(format!("{ROOT}/justfile")).unwrap();
    let recipe = justfile
        .split("\nrelease-tag version:\n")
        .nth(1)
        .and_then(|rest| rest.split("\n\n").next())
        .expect("the release-tag recipe");
    assert!(
        !recipe.contains("git tag "),
        "the recipe must not tag directly:\n{recipe}"
    );
    let sign = recipe
        .find("scripts/sign_release_tag.sh")
        .expect("the recipe signs the tag");
    let push = recipe.find("git push").expect("the recipe pushes the tag");
    assert!(
        sign < push,
        "the tag is signed and verified before it is pushed"
    );
    let signers = std::fs::read_to_string(format!("{ROOT}/.github/allowed_signers")).unwrap();
    for line in signers
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
    {
        assert!(
            line.contains("namespaces=\"git\" ssh-"),
            "allowed_signers line: {line}"
        );
        assert!(
            !line.contains("PRIVATE"),
            "allowed_signers holds public keys only"
        );
    }
}

/// Review of 0.8.4, VR84B-M-2: `git diff --quiet` and `git diff --cached --quiet` pass with an
/// untracked file, which Cargo can read (an untracked `build.rs`), so the gate could run on
/// more than the commit that is tagged. The recipe refuses any `git status --porcelain` output
/// (untracked files included) before the gate and after it. A scratch run of the recipe with the
/// gate, signing and push stubbed is in the 0.8.4 evidence; this holds its shape.
#[test]
fn the_release_recipe_refuses_any_status_output_before_and_after_the_gate() {
    let justfile = std::fs::read_to_string(format!("{ROOT}/justfile")).unwrap();
    let recipe = justfile
        .split("\nrelease-tag version:\n")
        .nth(1)
        .and_then(|rest| rest.split("\n\n").next())
        .expect("the release-tag recipe");
    let check = r#"if [ -n "$(git status --porcelain)" ]; then"#;
    let gate = recipe
        .find("just gate-release")
        .expect("the recipe runs the gate");
    let checks: Vec<usize> = recipe.match_indices(check).map(|(i, _)| i).collect();
    assert!(
        checks.len() == 2 && checks[0] < gate && gate < checks[1],
        "a status check before and after the gate:\n{recipe}"
    );
    assert!(
        !recipe.contains("git diff --quiet"),
        "git diff --quiet does not see untracked files:\n{recipe}"
    );
}

// --- scripts/verify_release_tag.sh --------------------------------------------------------

/// Runs the release workflow's check of `tag`, with `main` the commit `main` points at.
fn verify_tag_on(repo: &Path, tag: &str, signers: &Path, main: &str) -> Output {
    run(Command::new("bash")
        .current_dir(repo)
        .arg(format!("{ROOT}/scripts/verify_release_tag.sh"))
        .arg(tag)
        .arg(signers)
        .arg(main))
}

/// [`verify_tag_on`] with `main` at the repository's `main` branch.
fn verify_tag(repo: &Path, tag: &str, signers: &Path) -> Output {
    let main = git(repo, &["rev-parse", "main"]);
    verify_tag_on(repo, tag, signers, &main)
}

#[test]
fn the_release_workflow_accepts_only_a_tag_signed_by_an_allowed_key() {
    let s = scratch("verify-tag");
    let repo = signing_repo(&s, true);
    let signers = repo.join(".github/allowed_signers");
    // Signed by the allowed key, through the same script `just release-tag` uses.
    assert!(sign_tag(&s, &repo).status.success());
    let out = verify_tag(&repo, "v1.2.3", &signers);
    assert!(out.status.success(), "{out:?}");
    // Annotated but unsigned, and lightweight: neither carries a signature.
    git(&repo, &["tag", "-a", "v2.0.0", "-m", "v2.0.0"]);
    git(&repo, &["tag", "v3.0.0"]);
    for tag in ["v2.0.0", "v3.0.0"] {
        let out = verify_tag(&repo, tag, &signers);
        assert!(!out.status.success(), "{tag}: {out:?}");
    }
    // Signed, but by a key the signers file does not list.
    let other = s.0.join("other-signers");
    std::fs::write(
        &other,
        format!(
            "ddf@example.invalid namespaces=\"git\" {}",
            std::fs::read_to_string(s.0.join("other.pub")).unwrap()
        ),
    )
    .unwrap();
    assert!(!verify_tag(&repo, "v1.2.3", &other).status.success());
    // No signers at all.
    assert!(
        !verify_tag(&repo, "v1.2.3", &s.0.join("missing"))
            .status
            .success()
    );
}

/// Low-severity finding of 2026-11-04: a tag signed by an allowed key, but on a commit that never
/// reached `main`, was released. The check now refuses it, and still accepts a tag that `main`
/// has moved past.
#[test]
fn the_release_workflow_refuses_a_signed_tag_on_a_commit_not_on_main() {
    let s = scratch("verify-main");
    let repo = signing_repo(&s, true);
    let signers = repo.join(".github/allowed_signers");
    // Committed on `main`, so switching branches keeps it.
    let listed = std::fs::read_to_string(&signers).unwrap();
    commit(&repo, ".github/allowed_signers", &listed);
    assert!(sign_tag(&s, &repo).status.success());
    // A side branch, never merged, with a correctly signed tag on its commit.
    git(&repo, &["checkout", "-q", "-b", "side"]);
    commit(&repo, "side.txt", "unreviewed\n");
    assert!(sign_version(&s, &repo, "2.0.0").status.success());
    // `main` moves on past v1.2.3.
    git(&repo, &["checkout", "-q", "main"]);
    commit(&repo, "later.txt", "later\n");
    let out = verify_tag(&repo, "v2.0.0", &signers);
    assert!(!out.status.success(), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("not on main"),
        "{out:?}"
    );
    let out = verify_tag(&repo, "v1.2.3", &signers);
    assert!(out.status.success(), "{out:?}");
    // The `main` commit is required: without it the check refuses rather than skipping.
    let out = run(Command::new("bash")
        .current_dir(&repo)
        .arg(format!("{ROOT}/scripts/verify_release_tag.sh"))
        .arg("v1.2.3")
        .arg(&signers));
    assert!(!out.status.success(), "{out:?}");
}

#[test]
fn the_release_workflow_verifies_the_tag_against_main_s_signers_before_building() {
    let workflow =
        std::fs::read_to_string(format!("{ROOT}/.github/workflows/release.yml")).unwrap();
    let verify = workflow
        .find("scripts/verify_release_tag.sh")
        .expect("release.yml verifies the tag signature");
    let fetch_main = workflow[..verify]
        .rfind("git fetch --quiet --no-tags origin main")
        .expect("the gate fetches main");
    assert!(
        workflow[fetch_main..verify].contains("main=\"$(git rev-parse FETCH_HEAD)\"")
            && workflow[fetch_main..verify]
                .contains("git show \"${main}:.github/allowed_signers\""),
        "the signers come from main, not the tagged commit"
    );
    assert!(
        workflow[verify..].starts_with(
            "scripts/verify_release_tag.sh \"${tag}\" \"${RUNNER_TEMP}/allowed_signers\" \"${main}\""
        ),
        "the check is given main's commit, so a tag off main is refused"
    );
    let gate_job = &workflow[workflow.find("  supply-chain:").unwrap()..verify];
    assert!(
        gate_job.contains("fetch-depth: 0"),
        "the gate checks out full history, which the on-main check needs"
    );
    let gate = workflow.find("  supply-chain:").unwrap();
    let build = workflow.find("\n  build:").unwrap();
    assert!(
        gate < verify && verify < build,
        "the check is in the supply-chain gate"
    );
    assert!(
        workflow[build..].contains("needs: [quality, supply-chain]"),
        "build waits for the gate"
    );
}

// --- scripts/security/generate_spdx_sbom.py -----------------------------------------------

/// Expert circle 2026-11-03 (C6): each registry package in the committed SBOM carries the
/// SHA-256 Cargo.lock records for its `.crate` archive, the checksum that identifies its source.
#[test]
fn the_sbom_checksums_are_the_crate_archive_checksums_cargo_lock_records() {
    let lock = std::fs::read_to_string(format!("{ROOT}/Cargo.lock")).unwrap();
    let sbom = std::fs::read_to_string(format!("{ROOT}/sbom.spdx.json")).unwrap();
    let mut checked = 0;
    for entry in lock.split("[[package]]").skip(1) {
        let field = |key: &str| {
            entry
                .lines()
                .find_map(|l| l.strip_prefix(&format!("{key} = \"")))
                .map(|v| v.trim_end_matches('"').to_owned())
        };
        if let Some(sum) = field("checksum") {
            assert!(
                sbom.contains(&format!("\"checksumValue\": \"{sum}\"")),
                "{entry}"
            );
            checked += 1;
        }
    }
    assert!(checked > 50, "only {checked} registry packages found");
}

// --- scripts/package_release.py -----------------------------------------------------------

/// Inputs for one pack: an executable and two documents, with their mtimes set to `stamp`.
fn release_inputs(dir: &Path, stamp: &str) -> Vec<PathBuf> {
    std::fs::create_dir_all(dir).unwrap();
    let bin = dir.join("deep-diff-forge");
    tool(&bin, "echo ddf\n");
    std::fs::write(dir.join("README.md"), "read me\n").unwrap();
    std::fs::write(dir.join("LICENSE-MIT"), "mit\n").unwrap();
    let files = vec![bin, dir.join("README.md"), dir.join("LICENSE-MIT")];
    for file in &files {
        let out = run(Command::new("touch").args(["-t", stamp]).arg(file));
        assert!(out.status.success(), "{out:?}");
    }
    files
}

fn package(out: &Path, files: &[PathBuf], epoch: &str) -> Vec<u8> {
    std::fs::create_dir_all(out).unwrap();
    let done = run(Command::new("python3")
        .arg("-I")
        .arg(format!("{ROOT}/scripts/package_release.py"))
        .arg("ddf-v9.9.9-test")
        .arg(out)
        .args(files)
        .env("SOURCE_DATE_EPOCH", epoch));
    assert!(done.status.success(), "{done:?}");
    std::fs::read(out.join("ddf-v9.9.9-test.tar.gz")).unwrap()
}

/// Expert circle 2026-11-03 (C14): packing the same files twice, from copies with different
/// mtimes, gives the same bytes; every entry is sorted, owned by 0:0, stamped with
/// `SOURCE_DATE_EPOCH` and has a fixed mode.
#[test]
fn a_release_archive_is_the_same_bytes_from_the_same_inputs() {
    let s = scratch("package");
    let first = package(
        &s.0.join("out1"),
        &release_inputs(&s.0.join("a"), "202001010000"),
        "1700000000",
    );
    let second = package(
        &s.0.join("out2"),
        &release_inputs(&s.0.join("b"), "202511031200"),
        "1700000000",
    );
    assert_eq!(first, second, "two packs of the same inputs differ");
    assert_ne!(
        first,
        package(
            &s.0.join("out3"),
            &release_inputs(&s.0.join("c"), "202001010000"),
            "1700000001"
        ),
        "the epoch is what stamps the archive"
    );
    let listing = run(Command::new("python3").arg("-I").arg("-c").arg(
        "import sys,tarfile\nfor m in tarfile.open(sys.argv[1]).getmembers(): print(m.name, m.uid, m.gid, repr(m.uname), m.mtime, oct(m.mode))",
    ).arg(s.0.join("out1/ddf-v9.9.9-test.tar.gz")));
    assert!(listing.status.success(), "{listing:?}");
    assert_eq!(
        String::from_utf8_lossy(&listing.stdout),
        "ddf-v9.9.9-test 0 0 '' 1700000000 0o755\n\
         ddf-v9.9.9-test/LICENSE-MIT 0 0 '' 1700000000 0o644\n\
         ddf-v9.9.9-test/README.md 0 0 '' 1700000000 0o644\n\
         ddf-v9.9.9-test/deep-diff-forge 0 0 '' 1700000000 0o755\n"
    );
}

/// The control the test above guards against: `tar czf`, as release.yml packed before, records
/// the inputs' mtimes, so the same files packed from two copies differ.
#[test]
fn plain_tar_czf_is_not_reproducible() {
    let s = scratch("package-control");
    let mut packs = Vec::new();
    for (dir, stamp) in [("a", "202001010000"), ("b", "202511031200")] {
        let root = s.0.join(dir);
        release_inputs(&root.join("dist"), stamp);
        let out = run(Command::new("tar")
            .current_dir(&root)
            .args(["czf", "x.tar.gz", "dist"]));
        assert!(out.status.success(), "{out:?}");
        packs.push(std::fs::read(root.join("x.tar.gz")).unwrap());
    }
    assert_ne!(packs[0], packs[1]);
}

#[test]
fn release_yml_packs_through_the_reproducible_script() {
    let workflow =
        std::fs::read_to_string(format!("{ROOT}/.github/workflows/release.yml")).unwrap();
    assert!(workflow.contains("scripts/package_release.py"));
    assert!(workflow.contains("SOURCE_DATE_EPOCH=\"$(git log -1 --format=%ct)\""));
    assert!(
        !workflow
            .lines()
            .any(|l| !l.trim_start().starts_with('#') && l.contains("tar czf")),
        "release.yml packs with tar czf again"
    );
}

#[test]
fn package_release_refuses_without_an_epoch() {
    let s = scratch("package-epoch");
    let files = release_inputs(&s.0.join("a"), "202001010000");
    let out = run(Command::new("python3")
        .arg("-I")
        .arg(format!("{ROOT}/scripts/package_release.py"))
        .arg("x")
        .arg(&s.0)
        .args(&files)
        .env_remove("SOURCE_DATE_EPOCH"));
    assert!(!out.status.success(), "{out:?}");
    assert!(!s.0.join("x.tar.gz").exists());
}

// --- scripts/test_step.sh (the gate's test step) -----------------------------------------

/// The names `LOAD_SENSITIVE` lists, read as `the_load_sensitive_list_names_tests_that_exist`
/// (in `agent_hook.rs`) reads them: one a line, `#` starts a comment, blank lines are skipped.
fn listed(list: &str) -> Vec<String> {
    list.lines()
        .filter_map(|line| line.split('#').next().map(str::trim))
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect()
}

/// A stand-in `cargo` that writes each call's arguments as a line of `$CALLS`, answers a call
/// with `--test-threads=1` with a passing line for each name after `--` (all but `$DROP`), and
/// exits with `$PASS1` on any other call.
const CARGO: &str = r#"printf '%s\n' "$*" >>"$CALLS"
case " $* " in
*" --test-threads=1 "*)
  after=
  for a in "$@"; do
    if [ -n "$after" ]; then
      case $a in --*) ;; "$DROP") ;; *) echo "test $a ... ok" ;; esac
    fi
    [ "$a" = -- ] && after=1
  done
  exit 0 ;;
esac
exit "${PASS1:-0}"
"#;

/// Runs `scripts/test_step.sh` in `checkout` with the stand-in `cargo`: (its output, the
/// calls, each split into words).
fn test_step(s: &Scratch, checkout: &Path, pass1: u8, drop: &str) -> (Output, Vec<Vec<String>>) {
    let bin = s.0.join("bin");
    tool(&bin.join("cargo"), CARGO);
    let calls = s.0.join("calls");
    let _ = std::fs::remove_file(&calls);
    let out = run(Command::new("bash")
        .arg(format!("{ROOT}/scripts/test_step.sh"))
        .current_dir(checkout)
        .env("PATH", path_with(&bin))
        .env("CALLS", &calls)
        .env("PASS1", pass1.to_string())
        .env("DROP", drop));
    let calls = std::fs::read_to_string(&calls)
        .unwrap_or_default()
        .lines()
        .map(|line| line.split_whitespace().map(str::to_owned).collect())
        .collect();
    (out, calls)
}

/// P5b: the gate's test step runs the workspace's tests less the load-sensitive ones, then
/// those alone on one thread: every name it skips it runs, both passes go on past a failing
/// test binary (`--no-fail-fast`), and the second pass runs even when the first failed. The
/// list is read from `tests/LOAD_SENSITIVE` and nowhere else.
#[test]
fn the_test_step_runs_every_load_sensitive_test_alone_after_the_others() {
    let s = scratch("test-step");
    let names = listed(
        &std::fs::read_to_string(format!(
            "{ROOT}/crates/deep-diff-forge-cli/tests/LOAD_SENSITIVE"
        ))
        .unwrap(),
    );
    assert!(!names.is_empty(), "LOAD_SENSITIVE names no test");
    let (out, calls) = test_step(&s, Path::new(ROOT), 0, "");
    let said = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(out.status.success(), "{out:?}");
    assert_eq!(calls.len(), 2, "{calls:?}");
    let (first, second) = (&calls[0], &calls[1]);
    let after = |call: &[String]| -> Vec<String> {
        call.iter()
            .skip_while(|word| *word != "--")
            .skip(1)
            .cloned()
            .collect()
    };
    let before = |call: &[String]| -> Vec<String> {
        call.iter()
            .take_while(|word| *word != "--")
            .cloned()
            .collect()
    };
    assert_eq!(
        before(first),
        ["test", "--workspace", "--locked", "--no-fail-fast"],
        "{first:?}"
    );
    let rest = after(first);
    assert_eq!(rest[0], "--exact", "{first:?}");
    assert!(
        rest[1..]
            .chunks(2)
            .all(|pair| pair.len() == 2 && pair[0] == "--skip"),
        "{first:?}"
    );
    let skipped: Vec<String> = rest[1..].chunks(2).map(|pair| pair[1].clone()).collect();
    assert_eq!(skipped, names, "pass 1 must skip exactly the listed names");
    assert_eq!(
        before(second),
        [
            "test",
            "--workspace",
            "--locked",
            "--no-fail-fast",
            "--tests"
        ],
        "{second:?}"
    );
    let alone = after(second);
    assert_eq!(alone[..2], ["--exact", "--test-threads=1"], "{second:?}");
    assert_eq!(
        alone[2..],
        names[..],
        "every listed name that pass 1 skips must run alone in pass 2"
    );
    assert!(said.contains("pass 1 rc=0, pass 2 rc=0"), "{said}");

    // a failing first pass: the second still runs, and the step fails
    let (out, calls) = test_step(&s, Path::new(ROOT), 101, "");
    assert!(
        !out.status.success() && calls.len() == 2,
        "{out:?} {calls:?}"
    );
    // a listed test the second pass did not run: named, and the step fails
    let (out, _) = test_step(&s, Path::new(ROOT), 0, &names[0]);
    let said = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        !out.status.success()
            && said.contains(&format!(
                "pass 2 did not run each listed test once: {} (0 runs)",
                names[0]
            )),
        "{said}"
    );

    // the list as the test of agent_hook.rs reads it; a name twice is refused before any run
    let checkout = s.0.join("checkout");
    let list = checkout.join("crates/deep-diff-forge-cli/tests/LOAD_SENSITIVE");
    std::fs::create_dir_all(list.parent().unwrap()).unwrap();
    std::fs::write(&list, "# a comment\n\n  one_test  # why\ntwo_test\n").unwrap();
    let (out, calls) = test_step(&s, &checkout, 0, "");
    assert!(out.status.success(), "{out:?}");
    assert_eq!(after(&calls[1])[2..], ["one_test", "two_test"]);
    std::fs::write(&list, "one_test\ntwo_test\none_test\n").unwrap();
    let (out, calls) = test_step(&s, &checkout, 0, "");
    let said = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        !out.status.success() && calls.is_empty() && said.contains("names one_test twice"),
        "{said}"
    );
    std::fs::write(&list, "# none\n").unwrap();
    let (out, calls) = test_step(&s, &checkout, 0, "");
    assert!(!out.status.success() && calls.is_empty(), "{out:?}");
}

// --- release.yml's own checks, run for real (lane T3e of 0.11.1) ----------------------------

fn read(path: &str) -> String {
    std::fs::read_to_string(format!("{ROOT}/{path}")).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// The lines of job `job` of a workflow (fixed two-space layout), without comment lines.
fn workflow_job(workflow: &str, job: &str) -> Vec<String> {
    let mut inside = false;
    let mut lines = Vec::new();
    for line in workflow.lines().skip_while(|line| *line != "jobs:").skip(1) {
        if let Some(key) = line
            .strip_prefix("  ")
            .and_then(|rest| rest.strip_suffix(':'))
            .filter(|key| !key.starts_with(' ') && !key.contains(' '))
        {
            inside = key == job;
            continue;
        }
        if inside && !line.trim_start().starts_with('#') {
            lines.push(line.to_owned());
        }
    }
    assert!(!lines.is_empty(), "the workflow has no job `{job}`");
    lines
}

/// The steps of a job's lines, each as its lines (a step starts at `      - `; a job-level key
/// ends the step before it).
fn workflow_steps(job: &[String]) -> Vec<Vec<String>> {
    let mut steps: Vec<Vec<String>> = Vec::new();
    let mut open = false;
    for line in job {
        if line.starts_with("      - ") {
            steps.push(Vec::new());
            open = true;
        } else if line.starts_with("    ") && !line.starts_with("     ") {
            open = false;
        }
        if open && let Some(step) = steps.last_mut() {
            step.push(line.clone());
        }
    }
    steps
}

/// The step named `name` of job `job`.
fn workflow_step(workflow: &str, job: &str, name: &str) -> Vec<String> {
    workflow_steps(&workflow_job(workflow, job))
        .into_iter()
        .find(|step| step[0].trim_start() == format!("- name: {name}"))
        .unwrap_or_else(|| panic!("job `{job}` has no step `{name}`"))
}

/// A step's `run:`: its one line, or its `|` block without the block's indentation.
fn run_of(step: &[String]) -> String {
    let at = step
        .iter()
        .position(|line| {
            let rest = line.trim_start();
            rest.strip_prefix("- ").unwrap_or(rest).starts_with("run:")
        })
        .unwrap_or_else(|| panic!("the step has no run: {step:#?}"));
    let column = step[at].find("run:").unwrap();
    let value = step[at][column + "run:".len()..].trim();
    if value != "|" {
        return format!("{value}\n");
    }
    let indent = column + 2;
    let mut script = String::new();
    for line in step[at + 1..]
        .iter()
        .take_while(|line| line.trim().is_empty() || line.len() - line.trim_start().len() >= indent)
    {
        script.push_str(line.get(indent..).unwrap_or_default());
        script.push('\n');
    }
    script
}

#[test]
fn the_workflow_step_reader_reads_jobs_steps_and_runs() {
    let workflow = "on:\njobs:\n  a:\n    steps:\n      - name: one\n        run: echo one\n      - run: |\n          two\n  b:\n    name: bee\n    steps:\n      # - name: one\n      - name: one\n        env:\n          X: y\n        run: |\n          set -e\n\n            indented\n      - uses: x\n    timeout-minutes: 3\n";
    assert_eq!(run_of(&workflow_step(workflow, "a", "one")), "echo one\n");
    let a = workflow_steps(&workflow_job(workflow, "a"));
    assert_eq!(run_of(&a[1]), "two\n");
    assert_eq!(
        run_of(&workflow_step(workflow, "b", "one")),
        "set -e\n\n  indented\n"
    );
    let b = workflow_job(workflow, "b");
    assert_eq!(b[0], "    name: bee");
    let steps = workflow_steps(&b);
    assert_eq!(steps.len(), 2);
    assert_eq!(steps[1], ["      - uses: x"]);
}

/// A scratch directory with release.yml's step `step` of job `job` as `step.sh` and a copy of
/// the workspace manifest, which the step reads its version from.
fn release_step(s: &Scratch, job: &str, step: &str) -> (Vec<String>, String) {
    let workflow = read(".github/workflows/release.yml");
    let lines = workflow_step(&workflow, job, step);
    let script = run_of(&lines);
    assert!(
        !script.contains("${{"),
        "the step's script must read only its environment: {script}"
    );
    std::fs::write(s.0.join("step.sh"), &script).unwrap();
    std::fs::copy(format!("{ROOT}/Cargo.toml"), s.0.join("Cargo.toml")).unwrap();
    let manifest = read("Cargo.toml");
    let version = manifest
        .split("[workspace.package]")
        .nth(1)
        .and_then(|rest| {
            rest.lines()
                .find_map(|line| line.strip_prefix("version = \""))
        })
        .and_then(|rest| rest.strip_suffix('"'))
        .expect("the workspace version")
        .to_owned();
    (lines, version)
}

/// SECURITY.md: "A tag must exactly match the workspace version." release.yml's supply-chain
/// job checks it on every tag run, and the build (so the release and publication after it)
/// waits for that job (`release_workflow.rs`,
/// `both_release_runs_gate_build_run_and_check_every_target`). The check's own script is run
/// here, as the runner runs it (`bash -e`), with the pushed tag's name in `RELEASE_TAG`.
#[test]
fn a_release_tag_that_is_not_the_workspace_version_stops_the_release() {
    let s = scratch("release-tag-version");
    let (lines, version) = release_step(
        &s,
        "supply-chain",
        "verify release tag matches workspace version",
    );
    let has = |want: &str| lines.iter().any(|line| line.trim() == want);
    assert!(
        has("if: startsWith(github.ref, 'refs/tags/v')"),
        "the check runs on every run of a `v*` tag: {lines:#?}"
    );
    assert!(
        has("RELEASE_TAG: ${{ github.ref_name }}"),
        "the check reads the pushed tag's name: {lines:#?}"
    );
    let check = |tag: &str| {
        run(Command::new("bash")
            .args(["-e", "step.sh"])
            .current_dir(&s.0)
            .env("RELEASE_TAG", tag))
    };
    let good = check(&format!("v{version}"));
    assert!(
        good.status.success(),
        "the tag of the workspace version must pass: {good:?}"
    );
    for tag in [
        format!("v{version}.1"),
        format!("v{version}-rc.1"),
        version.clone(),
        format!("V{version}"),
        "v0.0.1".to_owned(),
    ] {
        let out = check(&tag);
        assert!(
            !out.status.success(),
            "the tag {tag} of a {version} workspace passed the check: {out:?}"
        );
        assert!(
            String::from_utf8_lossy(&out.stdout).contains(&format!(
                "::error::release tag {tag} does not match workspace version v{version}"
            )),
            "the refusal names the tag and the version: {out:?}"
        );
    }
}

/// SECURITY.md: "… an open gate without a token fails the job instead of passing without
/// publishing." The publish job's script runs here as the runner runs it, with stand-ins for
/// `curl` (crates.io's answer), `cargo` (publication) and `sleep`, each recording its call.
/// That the job runs only on a tag with `DDF_CRATES_IO` open, in the `release` environment, is
/// `crate_edges.rs` (`release_publication_is_owner_gated_and_the_dry_run_publishes_nothing`).
#[test]
fn an_open_crates_io_gate_without_a_token_fails_before_publishing() {
    let s = scratch("publish-gate");
    let (lines, version) = release_step(&s, "publish", "publish crates in dependency order");
    assert!(
        lines
            .iter()
            .any(|line| line.trim() == "CARGO_REGISTRY_TOKEN: ${{ secrets.CARGO_REGISTRY_TOKEN }}"),
        "the step takes the token from the environment's secret: {lines:#?}"
    );
    let bin = s.0.join("bin");
    tool(
        &bin.join("curl"),
        "printf 'curl %s\\n' \"$*\" >>\"$CALLS\"\nexit \"$CURL_EXIT\"\n",
    );
    tool(
        &bin.join("cargo"),
        "printf 'cargo %s\\n' \"$*\" >>\"$CALLS\"\n",
    );
    tool(&bin.join("sleep"), "exit 0\n");
    let path = path_with(&bin);
    let calls = s.0.join("calls");
    let publish = |token: Option<&str>, curl_exit: &str| {
        std::fs::write(&calls, "").unwrap();
        let mut cmd = Command::new("bash");
        cmd.args(["-e", "step.sh"])
            .current_dir(&s.0)
            .env("PATH", &path)
            .env("CALLS", &calls)
            .env("CURL_EXIT", curl_exit)
            .env("GITHUB_REF_NAME", format!("v{version}"));
        match token {
            Some(token) => cmd.env("CARGO_REGISTRY_TOKEN", token),
            None => cmd.env_remove("CARGO_REGISTRY_TOKEN"),
        };
        let out = run(&mut cmd);
        (out, std::fs::read_to_string(&calls).unwrap())
    };
    // No token (a secret that is not set expands to nothing), while crates.io has none of the
    // crates yet: the job fails at once, and crates.io is not even asked.
    for token in [None, Some("")] {
        let (out, called) = publish(token, "22");
        assert!(
            !out.status.success(),
            "an open gate without a token passed (token {token:?}): {out:?}"
        );
        assert!(
            String::from_utf8_lossy(&out.stdout).contains(
                "::error::DDF_CRATES_IO is open but CARGO_REGISTRY_TOKEN is not set in the release environment"
            ),
            "the failure says why (token {token:?}): {out:?}"
        );
        assert_eq!(
            called, "",
            "crates.io was asked or published to without a token ({token:?})"
        );
    }
    // Control: with a token, and every crate already published, the script runs to its end and
    // publishes nothing.
    let (out, called) = publish(Some("token"), "0");
    assert!(out.status.success(), "{out:?}");
    assert!(
        called.lines().count() >= 10 && called.lines().all(|line| line.starts_with("curl ")),
        "{called}"
    );
}

// --- scripts/release_local.sh: built and smoked as release.yml, signed by the owner only
// (lane T3e of 0.11.1) --------------------------------------------------------------------

/// The lines of the shell function `name` of a script, from `name() {` to the line `}`, without
/// comment lines and without the bodies of its quoted here-documents (`<<'EOF'`): the
/// receipt's Python names the checks the build ran, which is not running them.
fn shell_function<'a>(script: &'a str, name: &str) -> Vec<&'a str> {
    let head = format!("{name}() {{");
    let mut lines = Vec::new();
    let mut heredoc: Option<&str> = None;
    for line in script.lines().skip_while(|line| *line != head).skip(1) {
        if let Some(end) = heredoc {
            if line.trim() == end {
                heredoc = None;
            }
            continue;
        }
        if line == "}" {
            break;
        }
        if line.trim_start().starts_with('#') {
            continue;
        }
        if let Some((_, rest)) = line.split_once("<<'") {
            heredoc = rest.split('\'').next();
        }
        lines.push(line);
    }
    assert!(!lines.is_empty(), "no shell function {name}");
    lines
}

/// The words of a `cargo build` command, up to and including `--target`.
fn build_command(line: &str) -> Vec<&str> {
    let words: Vec<&str> = line[line.find("cargo build").expect("a cargo build")..]
        .split_whitespace()
        .collect();
    let end = words
        .iter()
        .position(|word| *word == "--target")
        .map_or(words.len(), |at| at + 1);
    words[..end].to_vec()
}

/// SECURITY.md: "`scripts/release_local.sh build` … builds the commit twice from fresh exports
/// with `release.yml`'s cargo command and stops unless both binaries have one SHA-256, then runs
/// the same smoke and glibc check, writes the SBOM, archive, receipt and `SHA256SUMS`, and signs
/// nothing." The two builds and the one SHA-256, and what is written, are held by the gate's
/// "release-script self-test" (`scripts/test_release_local.sh`, cases 6 and 1); this holds the
/// command, the smoke and the signing. That `release.yml` is not used, and where the owner
/// builds, are the owner's practice, not the repository's.
#[test]
fn the_local_release_builds_and_smokes_as_release_yml_and_signs_nothing() {
    let workflow = read(".github/workflows/release.yml");
    let script = read("scripts/release_local.sh");
    let ci = run_of(&workflow_step(&workflow, "build", "build release binary"));
    let local = shell_function(&script, "build_at");
    let local = local
        .iter()
        .find(|line| line.contains("cargo build"))
        .expect("build_at runs cargo build");
    assert_eq!(
        build_command(local),
        build_command(&ci),
        "scripts/release_local.sh builds with another command than release.yml's"
    );
    let smoke = run_of(&workflow_step(
        &workflow,
        "build",
        "smoke the release binary",
    ));
    let build = shell_function(&script, "do_build");
    for part in [
        "--version",
        "--stdin-patch --json --require-files",
        "fixtures/patch/basic.patch",
        "grep -q '\"schema\": \"deep-diff-forge.review.v0\"'",
        "scripts/check_glibc_floor.sh",
    ] {
        assert!(
            smoke.contains(part),
            "release.yml's smoke no longer runs {part:?}: hold this check to what it runs now"
        );
        assert!(
            build.iter().any(|line| line.contains(part)),
            "scripts/release_local.sh's build does not run release.yml's smoke part {part:?}"
        );
    }
    let signing: Vec<&str> = script
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .filter(|line| {
            [
                "-Y sign", "git tag", "gpg ", "minisign", "cosign", "signify",
            ]
            .iter()
            .any(|command| line.contains(command))
        })
        .filter(|line| {
            let rest = line.trim_start();
            !(rest.starts_with("printf ") || rest.starts_with("echo "))
        })
        .collect();
    assert!(
        signing.is_empty(),
        "scripts/release_local.sh signs (it only prints the owner's commands): {signing:#?}"
    );
}

#[test]
fn the_release_script_readers_read_functions_and_commands() {
    let script = "a() {\n  # cargo build x\n  one\n  python3 - <<'EOF'\nchecks = [\"scripts/check_glibc_floor.sh\"]\n}\nEOF\n  two\n}\nb() {\n  cargo_env cargo build --release -p x --target \"$t\") \\\n}\n";
    assert_eq!(
        shell_function(script, "a"),
        ["  one", "  python3 - <<'EOF'", "  two"]
    );
    let b = shell_function(script, "b");
    assert_eq!(
        build_command(b[0]),
        ["cargo", "build", "--release", "-p", "x", "--target"]
    );
    assert_eq!(
        build_command("run: cargo build --locked"),
        ["cargo", "build", "--locked"]
    );
}

/// SECURITY.md: "The owner signs `SHA256SUMS` with the release key in the namespace
/// `deep-diff-forge-release-sha256sums` and publishes the signed tag and the files on GitLab,
/// Gitea and Forgejo." The signing and the release pages are the owner's acts. What the
/// repository holds of them: the namespace is the one `scripts/release_local.sh verify` checks
/// (the self-test's case 1b fails under another) and one every release key may sign in, and
/// `just release-tag` pushes the signed tag to those three remotes and no other.
#[test]
fn the_release_namespace_and_remotes_are_the_ones_security_md_names() {
    const NAMESPACE: &str = "deep-diff-forge-release-sha256sums";
    let words = read("SECURITY.md")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let sentence = format!(
        "The owner signs `SHA256SUMS` with the release key in the namespace `{NAMESPACE}` and publishes the signed tag and the files on GitLab, Gitea and Forgejo."
    );
    assert!(
        words.contains(&sentence),
        "SECURITY.md no longer says {sentence:?}: hold this check to what it says now"
    );
    let script = read("scripts/release_local.sh");
    assert!(
        script.lines().any(|line| line == format!("ns={NAMESPACE}")),
        "scripts/release_local.sh checks SHA256SUMS.sig in another namespace than {NAMESPACE}"
    );
    let signers = read(".github/release_allowed_signers");
    let keys: Vec<&str> = signers
        .lines()
        .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
        .collect();
    assert!(!keys.is_empty(), "no release key");
    for key in keys {
        let namespaces = key
            .split_whitespace()
            .nth(1)
            .and_then(|field| field.strip_prefix("namespaces=\""))
            .and_then(|field| field.strip_suffix('"'))
            .unwrap_or_default();
        assert!(
            namespaces
                .split(',')
                .any(|namespace| namespace == NAMESPACE),
            "a release key may not sign in {NAMESPACE}: namespaces {namespaces:?}"
        );
    }
    let justfile = read("justfile");
    let recipe = justfile
        .split("\nrelease-tag version:\n")
        .nth(1)
        .and_then(|rest| rest.split("\n\n").next())
        .expect("the release-tag recipe");
    let lines: Vec<&str> = recipe.lines().map(str::trim).collect();
    let loops: Vec<&&str> = lines
        .iter()
        .filter(|line| line.starts_with("for remote in "))
        .collect();
    assert_eq!(
        loops, [&"for remote in gitlab gitea forgejo; do"; 2],
        "the recipe checks and pushes to GitLab, Gitea and Forgejo, and to no other remote"
    );
    let pushes: Vec<&&str> = lines
        .iter()
        .filter(|line| line.contains("git push"))
        .collect();
    assert_eq!(
        pushes,
        [&"git push \"${remote}\" \"v${want}\""],
        "the signed tag is pushed only inside the loop over the three remotes"
    );
}

// --- gitleaks: the hook without its scanner, and CI's full-history scan (lane T3e of 0.11.1) --

/// `PATH` without any directory that holds an executable `name`. Should that leave no `git` or
/// `bash` (the hook's interpreter, found through `env`), they are linked into `keep`, first.
fn path_without(name: &str, keep: &Path) -> String {
    let path = std::env::var("PATH").unwrap_or_default();
    let holds = |dir: &str, tool: &str| {
        Path::new(dir)
            .join(tool)
            .metadata()
            .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
    };
    let dirs: Vec<&str> = path
        .split(':')
        .filter(|dir| !dir.is_empty() && !holds(dir, name))
        .collect();
    std::fs::create_dir_all(keep).unwrap();
    for tool in ["git", "bash"] {
        if !dirs.iter().any(|dir| holds(dir, tool)) {
            let found = path
                .split(':')
                .find(|dir| holds(dir, tool))
                .unwrap_or_else(|| panic!("no {tool} on PATH"));
            std::os::unix::fs::symlink(Path::new(found).join(tool), keep.join(tool)).unwrap();
        }
    }
    format!("{}:{}", keep.display(), dirs.join(":"))
}

/// SECURITY.md: the pre-push hook "refuses a push when gitleaks finds a secret or is missing".
/// A find is `the_pre_push_hook_scans_a_force_push_over_a_remote_tip_never_fetched`, the clean
/// push its control; this is the missing scanner: neither on `PATH` nor in `~/.local/bin`.
#[test]
fn the_pre_push_hook_refuses_a_push_when_gitleaks_is_missing() {
    let s = scratch("prepush-missing");
    let (a, _) = diverged_remote(&s);
    commit(&a, "clean.txt", "nothing secret\n");
    let path = path_without("gitleaks", &s.0.join("keep"));
    let home = s.0.join("home");
    std::fs::create_dir_all(&home).unwrap();
    // Positive control: the hook's two places hold no gitleaks.
    let found = run(Command::new("bash")
        .args([
            "-c",
            "command -v gitleaks || test -e \"$HOME/.local/bin/gitleaks\"",
        ])
        .env("PATH", &path)
        .env("HOME", &home));
    assert!(
        !found.status.success(),
        "gitleaks is still found: {}",
        String::from_utf8_lossy(&found.stdout)
    );
    let pushed = run(Command::new("git")
        .current_dir(&a)
        .env("PATH", &path)
        .env("HOME", &home)
        .args(["push", "origin", "HEAD:refs/heads/unscanned"]));
    let stderr = String::from_utf8_lossy(&pushed.stderr);
    assert!(
        !pushed.status.success(),
        "a push without gitleaks was accepted: {stderr}"
    );
    assert!(
        stderr.contains("pre-push: gitleaks not found; refusing push"),
        "the refusal says the scanner is missing: {stderr}"
    );
    assert_eq!(
        git(&s.0.join("remote.git"), &["branch", "--list", "unscanned"]),
        "",
        "the unscanned commits reached the remote"
    );
}

/// SECURITY.md: "the required `secrets` CI job (`secret scan (gitleaks)`, full history) and the
/// versioned pre-push hook `scripts/hooks/pre-push` (`just install-hooks`)". That the job is
/// required is a ruleset of the GitHub repository, not a file of this one.
#[test]
fn ci_scans_the_whole_history_for_secrets_and_installs_the_versioned_hook() {
    let workflow = read(".github/workflows/ci.yml");
    let job = workflow_job(&workflow, "secrets");
    assert_eq!(job[0], "    name: secret scan (gitleaks)");
    let steps = workflow_steps(&job);
    let checkout = steps
        .iter()
        .find(|step| step[0].contains("uses: actions/checkout@"))
        .expect("the secret scan checks out");
    assert!(
        checkout.iter().any(|line| line.trim() == "fetch-depth: 0"),
        "the secret scan's checkout is shallow, so it scans only part of the history: {checkout:#?}"
    );
    let scan = run_of(&workflow_step(
        &workflow,
        "secrets",
        "gitleaks (pinned release, checksum-verified)",
    ));
    let command = scan
        .lines()
        .find(|line| line.contains("gitleaks git "))
        .expect("the job runs gitleaks over the repository");
    assert!(
        command.contains("--exit-code 1") && !command.contains("--log-opts"),
        "the scan must fail on a find and cover every commit: {command}"
    );
    assert!(
        read("justfile").contains(
            "\ninstall-hooks:\n    install -m 0755 scripts/hooks/pre-push \"$(git rev-parse --git-common-dir)/hooks/pre-push\"\n"
        ),
        "`just install-hooks` installs the versioned hook"
    );
}

// --- scripts/security/generate_spdx_sbom.py: reproducible (lane T3e of 0.11.1) -------------

/// Runs the generator three times on a stand-in graph (cargo is not run: `_run_metadata` and
/// `_lock_checksums` are replaced), the second the same as the first, the third with one version
/// changed; prints, per run, the file, its `created`, its namespace and the SHA-256 of what the
/// namespace is to address (its packages and relationships, as the generator serialises them).
const SBOM_DRIVER: &str = r#"import contextlib, hashlib, importlib.util, io, json, sys
spec = importlib.util.spec_from_file_location("gen", sys.argv[1])
gen = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gen)
def graph(version):
    return {"packages": [
                {"id": "app", "name": "app", "version": "1.0.0", "source": None, "license": "MIT"},
                {"id": "dep", "name": "dep", "version": version,
                 "source": "registry+https://example.invalid/index", "license": "MIT"}],
            "workspace_members": ["app"],
            "resolve": {"nodes": [{"id": "app", "deps": [{"pkg": "dep"}]}]}}
gen._lock_checksums = lambda: {}
runs = []
for version in ["2.0.0", "2.0.0", "2.0.1"]:
    gen._run_metadata = lambda version=version: graph(version)
    with contextlib.redirect_stdout(io.StringIO()):
        gen.main()
    text = gen.OUTPUT.read_text()
    doc = json.loads(text)
    content = json.dumps({"packages": doc["packages"], "relationships": doc["relationships"]}, sort_keys=True)
    runs.append({"text": text, "created": doc["creationInfo"]["created"],
                 "namespace": doc["documentNamespace"],
                 "content": hashlib.sha256(content.encode()).hexdigest()})
print(json.dumps(runs))
"#;

/// SECURITY.md: "The committed SBOM is reproducible: its namespace is content-addressed and
/// `created` comes from the last `Cargo.lock` commit." The gate's SBOM step compares a
/// regenerated SBOM with the committed one but ignores `created`; this runs the generator in a
/// scratch repository whose last commit is not the last commit to `Cargo.lock`.
#[test]
fn the_sbom_namespace_is_its_content_s_hash_and_created_the_last_cargo_lock_commit() {
    let s = scratch("sbom-created");
    let repo = s.0.join("repo");
    let generator = repo.join("scripts/security/generate_spdx_sbom.py");
    std::fs::create_dir_all(generator.parent().unwrap()).unwrap();
    std::fs::copy(
        format!("{ROOT}/scripts/security/generate_spdx_sbom.py"),
        &generator,
    )
    .unwrap();
    git(&repo, &["init", "-q"]);
    for (file, epoch) in [("Cargo.lock", "1700000000"), ("other.txt", "1800000000")] {
        std::fs::write(repo.join(file), format!("{file}\n")).unwrap();
        git(&repo, &["add", file]);
        let date = format!("@{epoch} +0000");
        let done = run(Command::new("git")
            .current_dir(&repo)
            .args(["-c", "commit.gpgSign=false", "commit", "-qm", file])
            .env("GIT_AUTHOR_DATE", &date)
            .env("GIT_COMMITTER_DATE", &date));
        assert!(done.status.success(), "{done:?}");
    }
    let out = run(Command::new("python3")
        .arg("-I")
        .arg("-c")
        .arg(SBOM_DRIVER)
        .arg(&generator)
        .env_remove("SOURCE_DATE_EPOCH"));
    assert!(out.status.success(), "{out:?}");
    let runs: Vec<serde_json::Value> = serde_json::from_slice(&out.stdout).unwrap();
    let field = |at: usize, key: &str| runs[at][key].as_str().unwrap().to_owned();
    assert_eq!(
        field(0, "created"),
        "2023-11-14T22:13:20Z",
        "`created` is the time of the last commit to Cargo.lock (1700000000), not of HEAD's or the clock's"
    );
    assert_eq!(
        field(0, "text"),
        field(1, "text"),
        "the same graph gives the same SBOM, byte for byte"
    );
    for at in 0..3 {
        let content = field(at, "content");
        assert!(
            field(at, "namespace").ends_with(&format!("/sbom/{}", &content[..32])),
            "the namespace is not addressed by the SBOM's content: {} for {content}",
            field(at, "namespace")
        );
    }
    assert_ne!(
        field(0, "namespace"),
        field(2, "namespace"),
        "another graph has another namespace"
    );
}
