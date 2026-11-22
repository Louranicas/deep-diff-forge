// DDF integration: R02, R03, R07. S3.5: where "no change" stays a clean answer once `--stdin-patch` refuses input in which Git finds no patch.
// Design/status: docs/RECOMMENDATION_INTEGRATION.md#patch-truth
// Module index: docs/RECOMMENDATION_INTEGRATION.md#patch-truth
//! The controls of S3.5 (ruling S3.5.0): the Git selections on a clean tree and a file pair of
//! identical files still answer zero files with exit 0 (Git made that answer: no change, not
//! "no patch found"), `--allow-empty` is admitted with `--stdin-patch` only, and a hook stop with
//! no change still passes without starting its judge or a ranker.
#![cfg(unix)]
#![expect(
    clippy::disallowed_methods,
    reason = "integration test: drives real processes, the environment and wall-clock time"
)]

use deep_diff_forge_testkit::Scratch;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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
    assert!(out.status.success(), "git {args:?}");
}

/// The binary with `args`, `stdin` on its standard input and `home` as its home.
fn ddf(args: &[&str], stdin: &str, home: &Path) -> (i32, String, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .envs(ISOLATE)
        .env("HOME", home)
        .env_remove("XDG_STATE_HOME")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("CLAUDE_PROJECT_DIR")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // A command line refused before stdin is read may close it first.
    let _ = child.stdin.take().unwrap().write_all(stdin.as_bytes());
    let out = child.wait_with_output().unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// A committed repository with nothing changed since, and a home beside it.
fn clean_repository(scratch: &Scratch) -> (PathBuf, PathBuf) {
    let (root, home) = (scratch.join("repo"), scratch.join("home"));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    git(&root, &["init", "-q"]);
    std::fs::write(root.join("a.rs"), "fn a() {}\n").unwrap();
    std::fs::write(root.join("warden.toml"), "# the policy\n").unwrap();
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-q", "-m", "base"]);
    (root, home)
}

#[test]
fn a_clean_tree_still_ranks_zero_files_in_git_modes() {
    let scratch = Scratch::new("cli-empty-git");
    let (root, home) = clean_repository(&scratch);
    let repo = root.to_str().unwrap();
    for mode in [
        &["--git-working-tree"][..],
        &["--git-staged"],
        &["--git-range", "HEAD..HEAD"],
    ] {
        let args = [mode, &["--repo", repo, "--rank", "--json"]].concat();
        let (code, stdout, stderr) = ddf(&args, "", &home);
        assert_eq!(code, 0, "{args:?}: {stderr}");
        let ranked: serde_json::Value = serde_json::from_str(&stdout).unwrap();
        assert_eq!(ranked["ranked"], serde_json::json!([]), "{args:?}");
        // `--allow-empty` is `--stdin-patch`'s alone: a Git selection refuses it as unknown.
        let args = [mode, &["--repo", repo, "--allow-empty"]].concat();
        let (code, _, stderr) = ddf(&args, "", &home);
        assert_eq!(code, 2, "--allow-empty admitted with {args:?}: {stderr}");
    }
    // A file pair of identical files: no change, zero files, exit 0; no `--allow-empty` there.
    let (a, b) = (root.join("a.rs"), scratch.join("b.rs"));
    std::fs::write(&b, "fn a() {}\n").unwrap();
    let (a, b) = (a.to_str().unwrap(), b.to_str().unwrap());
    let (code, stdout, stderr) = ddf(&["--", a, b], "", &home);
    assert_eq!(
        (code, stdout.as_str()),
        (0, "0 file(s) changed\n"),
        "{stderr}"
    );
    let (code, _, stderr) = ddf(&["--", a, b, "--allow-empty"], "", &home);
    assert_eq!(code, 2, "--allow-empty admitted with a file pair: {stderr}");
}

/// Ruling S3.5.0 point 5: the hook never sends an empty patch. A stop with no change passes
/// (exit 0, nothing said) and starts neither its judge nor, through it, this binary as the
/// ranker, which would now refuse the empty patch with exit 4.
#[test]
fn a_stop_with_no_change_still_passes_without_a_ranker() {
    let scratch = Scratch::new("cli-empty-hook");
    let (root, home) = clean_repository(&scratch);
    let (judge, log) = (scratch.join("judge"), scratch.join("judge.log"));
    // The judge would run this binary as its ranker and log that it ran. Written by a child
    // process, so no file this test had open for writing is executed (ETXTBSY).
    let body = format!(
        "#!/bin/sh\necho ran >> '{}'\nexec '{}' --stdin-patch --rank --json\n",
        log.display(),
        env!("CARGO_BIN_EXE_deep-diff-forge")
    );
    let mut writer = Command::new("sh")
        .args(["-c", "cat > \"$1\" && chmod 755 \"$1\"", "sh"])
        .arg(&judge)
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    writer
        .stdin
        .take()
        .unwrap()
        .write_all(body.as_bytes())
        .unwrap();
    assert!(writer.wait().unwrap().success());
    let hook = |event: &str, name: &str| {
        let input = serde_json::json!({
            "hook_event_name": name,
            "session_id": "s1",
            "cwd": root,
            "stop_hook_active": false,
            "tool_name": "Bash",
        });
        let judge = judge.to_str().unwrap();
        let args = ["agent", "hook", event, "--client", "claude"];
        let extra: &[&str] = if event == "stop" {
            &["--judge", judge]
        } else {
            &[]
        };
        ddf(&[&args[..], extra].concat(), &input.to_string(), &home)
    };
    let (code, _, stderr) = hook("session-start", "SessionStart");
    assert_eq!((code, stderr.as_str()), (0, ""));
    assert_eq!(hook("stop", "Stop"), (0, String::new(), String::new()));
    assert!(!log.exists(), "the judge ran on a stop with no change");
    // What the ranker would answer if it were sent the empty patch: the refusal.
    let (code, stdout, _) = ddf(&["--stdin-patch", "--rank", "--json"], "", &home);
    assert_eq!((code, stdout.as_str()), (4, ""));
}
