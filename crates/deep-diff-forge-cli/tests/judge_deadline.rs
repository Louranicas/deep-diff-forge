//! A stop's time bound holds against a judge that leaves a process behind (S3.17a.a). The judge
//! runs in a process group of its own, and the group is killed when the judge exits, so a
//! grandchild that keeps the judge's output open can no longer hold the stop: on 0.9.0 a judge
//! that exited at once while a `sleep 75` held its output stretched a stop to 75 s.
#![cfg(target_os = "linux")]
#![expect(
    clippy::disallowed_methods,
    reason = "integration test: drives real processes, the environment and wall-clock time"
)]

use deep_diff_forge_testkit::Scratch;
use serde_json::json;
use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const ISOLATE: [(&str, &str); 2] = [
    ("GIT_CONFIG_GLOBAL", "/dev/null"),
    ("GIT_CONFIG_NOSYSTEM", "1"),
];

/// Everything a stop does must be done in this time (`STOP_LIMIT` in `hook_run.rs`, stated in
/// `docs/reference/agent-hooks.md`). Here the judge exits at once and the stop takes about a
/// second, so the bound leaves some 44 s of margin for a loaded machine; the grandchild sleeps
/// longer than the bound and than the 50 s a registered Stop hook gets, so a stop that waits
/// for it fails the bound whatever the load.
const STOP_BOUND: Duration = Duration::from_secs(45);
/// How long the judge's grandchild sleeps; also the mark that tells it from any other process.
const GRANDCHILD_SLEEP: &str = "97";

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
        .args([
            "-c",
            "init.defaultBranch=main",
            "-c",
            "commit.gpgSign=false",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}");
}

/// Run `agent hook <event> --client claude <extra…>` the way Claude Code does.
fn hook(home: &Path, root: &Path, event: &str, name: &str, extra: &[&str]) -> (i32, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .envs(ISOLATE)
        .env("HOME", home)
        .env_remove("XDG_STATE_HOME")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("CLAUDE_PROJECT_DIR")
        .args(["agent", "hook", event, "--client", "claude"])
        .args(extra)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let input = json!({"hook_event_name": name, "session_id": "s", "cwd": root,
                       "stop_hook_active": false});
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.to_string().as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    (
        out.status.code().unwrap(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

/// Whether `pid` still runs the grandchild's `sleep` (a zombie or a reused pid is not it).
/// Waits up to two seconds for a kill to land.
fn grandchild_alive(pid: &str) -> bool {
    let started = Instant::now();
    loop {
        let proc = format!("/proc/{pid}");
        let cmdline = std::fs::read(format!("{proc}/cmdline")).unwrap_or_default();
        let stat = std::fs::read_to_string(format!("{proc}/stat")).unwrap_or_default();
        let zombie = stat
            .rsplit(')')
            .next()
            .is_some_and(|rest| rest.trim_start().starts_with('Z'));
        let alive = !zombie && cmdline == format!("sleep\0{GRANDCHILD_SLEEP}\0").into_bytes();
        if !alive || started.elapsed() > Duration::from_secs(2) {
            return alive;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_judge_that_leaves_a_grandchild_holding_stdout_cannot_stretch_the_stop() {
    let scratch = Scratch::new("judge-deadline");
    let home = scratch.join("home");
    let root = scratch.join("repo");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir(&root).unwrap();
    git(&root, &["init", "--quiet"]);
    std::fs::write(root.join("a.rs"), "fn a() {}\n").unwrap();
    std::fs::write(root.join("warden.toml"), "# policy\n").unwrap();
    git(&root, &["add", "--all"]);
    git(&root, &["commit", "--quiet", "-m", "base"]);
    // The stand-in judge: a grandchild keeps its output open, it exits at once with no verdict.
    // Written by a child process, so no open handle of this process can make it busy (ETXTBSY).
    let (judge, pids) = (scratch.join("judge"), scratch.join("grandchildren"));
    let body = format!(
        "#!/bin/sh\nsleep {GRANDCHILD_SLEEP} &\necho \"$!\" >> '{}'\nexit 0\n",
        pids.display()
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

    let (code, _) = hook(&home, &root, "session-start", "SessionStart", &[]);
    assert_eq!(code, 0);
    std::fs::write(root.join("a.rs"), "fn a() {}\nfn b() {}\n").unwrap();
    let started = Instant::now();
    let (code, stdout) = hook(
        &home,
        &root,
        "stop",
        "Stop",
        &["--judge", judge.to_str().unwrap()],
    );
    let took = started.elapsed();

    let pids = std::fs::read_to_string(&pids).unwrap_or_default();
    let alive: Vec<&str> = pids.lines().filter(|pid| grandchild_alive(pid)).collect();
    for pid in pids.lines() {
        let _ = Command::new("kill").args(["-KILL", pid]).status();
    }
    assert_eq!(code, 0, "{stdout}");
    assert!(!pids.is_empty(), "the judge was never run: {stdout}");
    assert!(
        stdout.contains("UNJUDGED"),
        "a judge that gave no verdict passed or blocked the stop: {stdout}"
    );
    assert!(
        took < STOP_BOUND,
        "the judge's grandchild held the stop for {took:?}, past its {STOP_BOUND:?} bound"
    );
    assert!(
        alive.is_empty(),
        "the judge's grandchildren {alive:?} outlived the stop"
    );
}
