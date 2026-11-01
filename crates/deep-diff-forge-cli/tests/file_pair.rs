// DDF integration: R02, R03, R07. CLI contract for the direct two-file review, exercised through the compiled binary with Git as the oracle for the sealed bytes.
// Design/status: README "Compare two files"
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fdeep-diff-forge-cli--tests--file_pair
//! `deep-diff-forge [--] <old> <new>`: the outputs of `--stdin-patch` over Git's exact bytes, the
//! refusals with their registered exit codes, and the repository around the files kept out.
#![cfg(unix)]
#![expect(
    clippy::disallowed_methods,
    reason = "integration test: drives real processes, the environment and wall-clock time"
)]

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const ISOLATE: [(&str, &str); 2] = [
    ("GIT_CONFIG_GLOBAL", "/dev/null"),
    ("GIT_CONFIG_NOSYSTEM", "1"),
];

/// A scratch directory, removed on drop.
struct Dir(PathBuf);

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

impl Dir {
    fn new(name: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("ddf-file-pair-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        Self(std::fs::canonicalize(root).unwrap())
    }

    fn file(&self, name: &str, bytes: &[u8]) -> String {
        let path = self.0.join(name);
        std::fs::write(&path, bytes).unwrap();
        path.to_str().unwrap().to_owned()
    }

    fn path(&self, name: &str) -> String {
        self.0.join(name).to_str().unwrap().to_owned()
    }
}

fn ddf_in(cwd: &Path, env: &[(&str, &str)], args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .current_dir(cwd)
        .envs(env.iter().copied())
        .args(args)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn ddf(args: &[&str]) -> (i32, String, String) {
    ddf_in(&std::env::temp_dir(), &ISOLATE, args)
}

/// What an isolated Git prints for the pair, run from `/` with the flags DDF uses: the oracle for
/// the bytes DDF seals.
fn oracle_patch(old: &str, new: &str) -> Vec<u8> {
    let out = Command::new("git")
        .current_dir("/")
        .envs(ISOLATE)
        .args([
            "diff",
            "--no-index",
            "--no-color",
            "--binary",
            "--full-index",
            "--src-prefix=a/",
            "--dst-prefix=b/",
            "--",
        ])
        .args([&old[1..], &new[1..]])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    out.stdout
}

fn json(text: &str) -> serde_json::Value {
    serde_json::from_str(text).unwrap_or_else(|e| panic!("{e}: {text}"))
}

#[test]
fn two_files_are_reviewed_in_each_output_mode_over_git_s_exact_bytes() {
    let dir = Dir::new("modes");
    let old = dir.file("old.rs", b"fn a() {}\nfn b() {}\n");
    let new = dir.file("new.rs", b"fn a() {}\nfn c() {}\n");
    let seal = deep_diff_forge_core::sha256_hex(&oracle_patch(&old, &new));

    let (code, stdout, stderr) = ddf(&[&old, &new, "--json"]);
    assert_eq!(code, 0, "{stderr}");
    let review = json(&stdout);
    assert_eq!(review["schema"], "deep-diff-forge.review.v0");
    assert_eq!(review["input_sha256"], seal.as_str());
    assert_eq!(review["files"][0]["path"], &new[1..]);
    assert_eq!(review["summary"]["additions"], 1);
    assert_eq!(review["summary"]["deletions"], 1);

    let (code, stdout, _) = ddf(&[&old, &new]);
    assert_eq!(code, 0);
    assert!(
        stdout.contains(&format!("modified  {}", &new[1..])),
        "{stdout}"
    );
    for (mode, schema) in [
        (&["--rank", "--json"][..], "deep-diff-forge.rank.v0"),
        (&["--cluster", "--json"], "deep-diff-forge.cluster.v0"),
        (&["--outline"], "deep-diff-forge.review-outline.v0"),
    ] {
        let args: Vec<&str> = [old.as_str(), new.as_str()]
            .into_iter()
            .chain(mode.iter().copied())
            .collect();
        let (code, stdout, stderr) = ddf(&args);
        assert_eq!(code, 0, "{mode:?}: {stderr}");
        let document = json(&stdout);
        assert_eq!(document["schema"], schema, "{mode:?}");
        assert_eq!(document["input_sha256"], seal.as_str(), "{mode:?}");
    }
    let (code, stdout, _) = ddf(&[&old, &new, "--jsonl"]);
    assert_eq!(code, 0);
    assert_eq!(json(stdout.trim())["event"], "diff.file");
    let (code, stdout, _) = ddf(&[&old, &new, "--layout", "inline"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("+ fn c() {}"), "{stdout}");
}

#[test]
fn binary_files_are_reviewed_as_a_binary_change() {
    let dir = Dir::new("binary");
    let old = dir.file("a.bin", b"a\0b");
    let new = dir.file("b.bin", b"a\0c");
    let (code, stdout, stderr) = ddf(&[&old, &new, "--json"]);
    assert_eq!(code, 0, "{stderr}");
    let review = json(&stdout);
    assert_eq!(review["files"][0]["status"], "binary_changed");
    assert_eq!(
        review["input_sha256"],
        deep_diff_forge_core::sha256_hex(&oracle_patch(&old, &new)).as_str()
    );
}

#[test]
fn identical_files_are_an_empty_review_that_the_input_contracts_refuse() {
    let dir = Dir::new("same");
    let old = dir.file("a.txt", b"same\n");
    let new = dir.file("b.txt", b"same\n");
    let (code, stdout, _) = ddf(&[&old, &new, "--json"]);
    assert_eq!(code, 0);
    assert_eq!(json(&stdout)["files"], serde_json::json!([]));
    let (code, stdout, stderr) = ddf(&[&old, &new, "--require-files"]);
    assert_eq!((code, stdout.as_str()), (7, ""), "{stderr}");
    assert!(stderr.contains("refused: 0 files"), "{stderr}");
}

/// A side that vanishes after DDF's checks makes Git exit 1 with no patch and "Could not
/// access" on stderr; that is exit 3, never an empty review with exit 0. A stub Git first on
/// `PATH` stands in for the race, which cannot be timed from outside.
#[test]
fn a_file_git_cannot_open_after_the_checks_is_exit_3_not_an_empty_review() {
    let dir = Dir::new("vanished");
    let old = dir.file("a.txt", b"a\n");
    let new = dir.file("b.txt", b"b\n");
    let bin = dir.0.join("bin");
    std::fs::create_dir(&bin).unwrap();
    let stub = bin.join("git");
    std::fs::write(
        &stub,
        "#!/bin/sh\necho \"error: Could not access 'gone'\" >&2\nexit 1\n",
    )
    .unwrap();
    std::fs::set_permissions(&stub, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let env = [ISOLATE[0], ISOLATE[1], ("PATH", bin.to_str().unwrap())];
    let (code, stdout, stderr) = (0..100)
        .map(|_| ddf_in(&std::env::temp_dir(), &env, &[&old, &new, "--json"]))
        .find(|(_, _, stderr)| !stderr.contains("os error 26"))
        .unwrap();
    assert_eq!((code, stdout.as_str()), (3, ""), "{stderr}");
    assert_eq!(
        stderr,
        "error: direct comparison failed: `git diff --no-index` exited with status 1: \
         error: Could not access 'gone'\n"
    );
}

#[test]
fn unreadable_and_non_regular_paths_are_refused_with_registered_codes() {
    let dir = Dir::new("refusals");
    let file = dir.file("file.txt", b"x\n");
    let missing = dir.path("missing.txt");
    std::os::unix::fs::symlink(&file, dir.0.join("link")).unwrap();
    let link = dir.path("link");
    let fifo = dir.path("fifo");
    assert!(
        Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success()
    );
    let directory = dir.0.to_str().unwrap().to_owned();
    for (args, code, message) in [
        (
            vec!["--", missing.as_str(), file.as_str()],
            3,
            "cannot read the old file: ",
        ),
        (
            vec![file.as_str(), missing.as_str()],
            3,
            "cannot read the new file: ",
        ),
        (vec![link.as_str(), file.as_str()], 4, "old file refused: "),
        (
            vec![file.as_str(), link.as_str()],
            4,
            "is a symbolic link, not a regular file",
        ),
        (
            vec![directory.as_str(), file.as_str()],
            4,
            "is a directory, not a regular file",
        ),
        (
            vec![file.as_str(), fifo.as_str()],
            4,
            "is a special file (FIFO, socket or device)",
        ),
    ] {
        let (got, stdout, stderr) = ddf(&args);
        assert_eq!(got, code, "{args:?}: {stderr}");
        assert_eq!(stdout, "", "{args:?}");
        assert!(stderr.contains(message), "{args:?}: {stderr}");
        assert!(
            stderr.starts_with("error: direct comparison failed: "),
            "{stderr}"
        );
    }
}

/// A first word that names neither a command nor a file is a usage error, as a misspelt command
/// always was; `--` marks it as a path, and then it is a missing file (exit 3).
#[test]
fn a_first_word_that_names_nothing_is_an_unknown_command_unless_marked_with_dash_dash() {
    let dir = Dir::new("note");
    let file = dir.file("file.txt", b"x\n");
    let (code, stdout, stderr) = ddf(&["revew", &file]);
    assert_eq!((code, stdout.as_str()), (2, ""), "{stderr}");
    assert_eq!(
        stderr,
        "unknown command: revew\nnote: no file has that name either; put `--` first to compare two paths\n"
    );
    for args in [
        &["--", "revew", file.as_str()][..],
        &[file.as_str(), "revew"],
    ] {
        let (code, _, stderr) = ddf(args);
        assert_eq!(code, 3, "{args:?}: {stderr}");
        assert!(stderr.contains("cannot read the "), "{args:?}: {stderr}");
    }
}

/// Sizes are checked before Git reads anything: a sparse file one byte over the budget is
/// refused, and identical files exactly at it are not.
#[test]
fn inputs_over_the_byte_budget_are_refused_before_git_reads_them() {
    let dir = Dir::new("budget");
    let budget = deep_diff_forge_patch::DEFAULT_BYTE_BUDGET as u64;
    let sparse = |name: &str, len: u64| {
        let path = dir.path(name);
        std::fs::File::create(&path).unwrap().set_len(len).unwrap();
        path
    };
    let (half, over) = (sparse("half", budget / 2), sparse("over", budget / 2 + 1));
    let (code, stdout, stderr) = ddf(&[&half, &half]);
    assert_eq!(
        (code, stdout.as_str()),
        (0, "0 file(s) changed\n"),
        "{stderr}"
    );
    let (code, stdout, stderr) = ddf(&[&half, &over]);
    assert_eq!((code, stdout.as_str()), (4, ""), "{stderr}");
    assert!(
        stderr.contains(&format!(
            "old and new file refused: {} + {} bytes exceed the {budget}-byte input budget",
            budget / 2,
            budget / 2 + 1
        )),
        "{stderr}"
    );
}

#[test]
fn malformed_invocations_are_usage_errors_before_any_file_is_read() {
    let dir = Dir::new("usage");
    let file = dir.file("file.txt", b"x\n");
    for (args, message) in [
        (
            vec!["--", file.as_str()],
            "a direct comparison needs two paths",
        ),
        (vec!["--"], "a direct comparison needs two paths"),
        (
            vec![file.as_str(), file.as_str(), "--jsno"],
            "unknown option: --jsno",
        ),
        (
            vec![file.as_str(), file.as_str(), "extra"],
            "unexpected argument: extra",
        ),
        (
            vec![file.as_str(), file.as_str(), "--json", "--jsonl"],
            "mutually exclusive",
        ),
        (
            vec![file.as_str(), file.as_str(), "--parallel", "2"],
            "--parallel requires --cluster",
        ),
        // Bug hunt 2026-11-01: `--jsonl` won and the `--hunks --path` check never ran.
        (
            vec![
                file.as_str(),
                file.as_str(),
                "--jsonl",
                "--hunks",
                "--path",
                "nothing",
            ],
            "--hunks and --jsonl are separate modes",
        ),
    ] {
        let (code, stdout, stderr) = ddf(&args);
        assert_eq!((code, stdout.as_str()), (2, ""), "{args:?}: {stderr}");
        assert!(stderr.contains(message), "{args:?}: {stderr}");
    }
    // A lone unknown word is still an unknown command, not half of a pair.
    let (code, _, stderr) = ddf(&["frobnicate", "--json"]);
    assert_eq!(code, 2);
    assert!(stderr.contains("unknown command: frobnicate"), "{stderr}");
}

/// Files named like commands are compared once `--` marks them as paths.
#[test]
fn dash_dash_compares_files_named_like_commands() {
    let dir = Dir::new("names");
    dir.file("doctor", b"a\n");
    dir.file("chain", b"b\n");
    let (code, stdout, stderr) = ddf_in(&dir.0, &ISOLATE, &["--", "doctor", "chain"]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("1 file(s) changed"), "{stdout}");
}

#[test]
fn help_after_the_paths_prints_the_direct_comparison_usage() {
    let (code, stdout, _) = ddf(&["old.rs", "new.rs", "--help"]);
    assert_eq!(code, 0);
    assert!(
        stdout.contains("USAGE:\n  deep-diff-forge <old> <new> [<--stdin-patch output flags>]"),
        "{stdout}"
    );
}

/// The files sit in a repository whose own configuration names a clean filter, and the user's
/// configuration asks for CRLF conversion and an attributes file with another filter. None of it
/// applies: no filter runs and the sealed bytes are the isolated oracle's.
#[test]
fn neither_a_surrounding_repository_nor_user_attributes_change_or_run_anything() {
    let dir = Dir::new("hostile");
    let marker = dir.path("FILTER-RAN");
    let git = |args: &[&str]| {
        let status = Command::new("git")
            .current_dir(&dir.0)
            .envs(ISOLATE)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    };
    git(&["init", "-q"]);
    let filter = format!("touch {marker}; cat");
    git(&["config", "filter.evil.clean", &filter]);
    dir.file(".gitattributes", b"* filter=evil\n");
    let attributes = dir.file("user-attributes", b"* filter=user\n");
    let global = dir.file(
        "user-config",
        format!(
            "[core]\n\tautocrlf = true\n\tattributesFile = {attributes}\n[filter \"user\"]\n\tclean = \"{filter}\"\n"
        )
        .as_bytes(),
    );
    let old = dir.file("old.txt", b"one\r\ntwo\r\n");
    let new = dir.file("new.txt", b"one\r\nthree\r\n");
    let env = [
        ("GIT_CONFIG_GLOBAL", global.as_str()),
        ("GIT_CONFIG_NOSYSTEM", "1"),
    ];
    let (code, stdout, stderr) = ddf_in(&dir.0, &env, &["old.txt", "new.txt", "--json"]);
    assert_eq!(code, 0, "{stderr}");
    assert!(!Path::new(&marker).exists(), "a filter ran");
    assert_eq!(
        json(&stdout)["input_sha256"],
        deep_diff_forge_core::sha256_hex(&oracle_patch(&old, &new)).as_str()
    );
}

/// Every mode `chain-contract` and `cluster-contract` list as shipped runs.
#[test]
fn the_contracts_list_only_modes_that_run() {
    let dir = Dir::new("contracts");
    let old = dir.file("old.txt", b"a\n");
    let new = dir.file("new.txt", b"b\n");
    let status = Command::new("git")
        .current_dir(&dir.0)
        .envs(ISOLATE)
        .args(["init", "-q"])
        .status()
        .unwrap();
    assert!(status.success());
    let patch = String::from_utf8(oracle_patch(&old, &new)).unwrap();
    let mut modes = 0;
    for contract in ["chain-contract", "cluster-contract"] {
        let (_, text, _) = ddf(&[contract]);
        let (_, shipped) = text.split_once("SHIPPED MODE").unwrap();
        let (shipped, _) = shipped.split_once("\n\n").unwrap();
        for line in shipped.lines().skip(1) {
            let Some(mode) = line.trim().strip_prefix("deep-diff-forge ") else {
                continue;
            };
            // Optional parts (`[--json]`) are dropped; the mode must run without them.
            let mut depth = 0usize;
            let required: String = mode
                .chars()
                .filter(|c| {
                    match c {
                        '[' => depth += 1,
                        ']' => depth -= 1,
                        _ => return depth == 0,
                    }
                    false
                })
                .collect();
            let args: Vec<&str> = required
                .split_whitespace()
                .map(|word| match word {
                    "<old>" => old.as_str(),
                    "<new>" => new.as_str(),
                    other => other,
                })
                .collect();
            let mut child = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
                .current_dir(&dir.0)
                .envs(ISOLATE)
                .args(&args)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            let _ = child.stdin.take().unwrap().write_all(patch.as_bytes());
            let out = child.wait_with_output().unwrap();
            assert_eq!(out.status.code(), Some(0), "{contract}: {mode}: {out:?}");
            modes += 1;
        }
    }
    assert_eq!(modes, 6);
}
