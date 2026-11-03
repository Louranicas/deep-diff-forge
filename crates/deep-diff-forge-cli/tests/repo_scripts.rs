//! The repository's gate scripts fail closed: the pre-push secret hook, the diff-scoped mutation
//! gate (anchor B-01), the fuzz runner and the release-tag signer. Each is run for real against a scratch repository,
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

/// Runs the script in `repo`, signing with `<scratch>/key`.
fn sign_tag(s: &Scratch, repo: &Path) -> Output {
    run(Command::new("bash")
        .current_dir(repo)
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "user.signingkey")
        .env("GIT_CONFIG_VALUE_0", s.0.join("key.pub"))
        .args([&format!("{ROOT}/scripts/sign_release_tag.sh"), "1.2.3"]))
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

// --- scripts/verify_release_tag.sh --------------------------------------------------------

fn verify_tag(repo: &Path, tag: &str, signers: &Path) -> Output {
    run(Command::new("bash")
        .current_dir(repo)
        .arg(format!("{ROOT}/scripts/verify_release_tag.sh"))
        .arg(tag)
        .arg(signers))
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

#[test]
fn the_release_workflow_verifies_the_tag_against_main_s_signers_before_building() {
    let workflow =
        std::fs::read_to_string(format!("{ROOT}/.github/workflows/release.yml")).unwrap();
    let verify = workflow
        .find("scripts/verify_release_tag.sh")
        .expect("release.yml verifies the tag signature");
    assert!(
        workflow[..verify].contains("git show FETCH_HEAD:.github/allowed_signers"),
        "the signers come from main, not the tagged commit"
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
