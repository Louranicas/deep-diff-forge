//! `agent check` from the outside (F3, part 1): the judgement a stop gives the uncommitted
//! change, on demand, with the session's base and the installed judge; the session's state is
//! read and never written; a check beside a live stop leaves the stop's scratch alone. Real
//! repositories, a stand-in for the judge (the helpers are copied from `tests/agent_hook.rs`).
#![cfg(unix)]
#![expect(
    clippy::disallowed_methods,
    reason = "integration test: drives real processes, the environment and wall-clock time"
)]

use deep_diff_forge_testkit::Scratch;
use serde_json::{Value, json};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const ISOLATE: [(&str, &str); 2] = [
    ("GIT_CONFIG_GLOBAL", "/dev/null"),
    ("GIT_CONFIG_NOSYSTEM", "1"),
];

fn git(root: &Path, args: &[&str]) -> String {
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
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

fn this_binary() -> PathBuf {
    std::fs::canonicalize(env!("CARGO_BIN_EXE_deep-diff-forge")).unwrap()
}

/// The binary, with `home` as its home, no project named by a client and no configuration of
/// the user or the system.
fn ddf(home: &Path, cwd: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"));
    command
        .current_dir(cwd)
        .envs(ISOLATE)
        .env("HOME", home)
        .env_remove("XDG_STATE_HOME")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("CLAUDE_PROJECT_DIR")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

fn finish(mut command: Command, stdin: &str) -> (i32, String, String) {
    let mut child = command.spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    (
        out.status.code().unwrap(),
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

fn hook(home: &Path, event: &str, extra: &[&str], input: &Value) -> (i32, String, String) {
    let mut command = ddf(home, home);
    command
        .args(["agent", "hook", event, "--client", "claude"])
        .args(extra);
    finish(command, &input.to_string())
}

fn input(event: &str, cwd: &Path, session: &str) -> Value {
    json!({"hook_event_name": event, "session_id": session, "cwd": cwd,
           "stop_hook_active": false, "tool_name": "Bash"})
}

/// `agent check <args…>` run in `cwd`, with `path` as `PATH` when given.
fn check(home: &Path, cwd: &Path, args: &[&str], path: Option<&Path>) -> (i32, String, String) {
    let mut command = ddf(home, cwd);
    command.args(["agent", "check"]).args(args);
    if let Some(path) = path {
        command.env("PATH", format!("{}:/usr/bin:/bin", path.display()));
    }
    finish(command, "")
}

/// A repository with one commit: a source file and a strict policy (`# strict`). The home of
/// its hooks is `scratch/home`.
fn repository(scratch: &Scratch) -> (PathBuf, PathBuf) {
    let home = scratch.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let root = scratch.join("repo");
    std::fs::create_dir(&root).unwrap();
    git(&root, &["init", "--quiet"]);
    std::fs::write(root.join("a.rs"), "fn a() {}\nfn b() {}\n").unwrap();
    std::fs::write(root.join("warden.toml"), "# strict\n").unwrap();
    git(&root, &["add", "--all"]);
    git(&root, &["commit", "--quiet", "-m", "base"]);
    (home, root)
}

/// The change every test makes: a silenced lint in `a.rs`.
fn silence_a_lint(root: &Path) {
    std::fs::write(
        root.join("a.rs"),
        "fn a() {}\n#[allow(dead_code)]\nfn b() {}\n",
    )
    .unwrap();
}

/// Write an executable script through a child process (a file this process had open for
/// writing can still be busy when another test thread forks: ETXTBSY).
fn write_program(path: &Path, body: &str) {
    let mut writer = Command::new("sh")
        .args(["-c", "cat > \"$1\" && chmod 755 \"$1\"", "sh"])
        .arg(path)
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
}

/// A stand-in for forge-warden that prints a whole, sealed `forge-warden.verdict.v2` (as the
/// one of `tests/agent_hook.rs`). Its answer: `pass`, `fail`, or `policy` (fail when the policy
/// it is given says `# strict`, else pass). With `wait`, it first writes the directory of the
/// patch it was given to `wait/started` and waits, at most 20 seconds, for `wait/release`.
fn judge(path: &Path, answer: &str, wait: Option<&Path>) {
    const SCRIPT: &str = r#"#!/bin/sh
policy=; patch=; ranker=
while [ $# -gt 0 ]; do
  case "$1" in
    --policy) policy="$2";; --patch) patch="$2";; --ranker) ranker="$2";;
  esac
  shift
done
wait='@WAIT@'
if [ -n "$wait" ]; then
  dirname "$patch" > "$wait/started.part" && mv "$wait/started.part" "$wait/started"
  n=0; while [ ! -e "$wait/release" ] && [ $n -lt 200 ]; do sleep 0.1; n=$((n+1)); done
fi
code=0
case '@ANSWER@' in
  fail) code=7;;
  policy) if grep -q '^# strict' "$policy"; then code=7; fi;;
esac
sum() { sha256sum < "$1" | cut -d' ' -f1; }
if [ "$code" = 7 ]; then said=fail; name=policy_fail; action=change_input; counts='"passed":0,"waived":0,"held":0,"failed":1'
else said=pass; name=pass; action=proceed; counts='"passed":1,"waived":0,"held":0,"failed":0'; fi
p="$(sum "$policy")"; f="$(sum "$patch")"; r="$(sum "$ranker")"
seals="{\"policy_sha256\":\"$p\",\"rank_sha256\":\"$f\",\"patch_sha256\":\"$f\",\"patch_facts_sha256\":null,\"provenance\":\"produced\",\"ranked_by\":{\"version\":\"stand-in\",\"sha256\":\"$r\"},\"git\":{\"path\":\"/usr/bin/git\",\"version\":\"git version 2.56.0\",\"sha256\":\"$p\",\"admission\":\"approved\",\"allowlist_sha256\":\"$f\"}}"
sed -e "s/\"@VERDICT@\"/\"$said\"/g" -e "s/\"@CODE@\"/$code/" -e "s/\"@NAME@\"/\"$name\"/" \
    -e "s/\"@ACTION@\"/\"$action\"/" -e "s|\"@SEALS@\"|$seals|" -e "s/\"@COUNTS@\":0/$counts/" <<'VERDICT'
@VERDICT_DOCUMENT@
VERDICT
exit "$code"
"#;
    let verdict = json!({
        "schema": "forge-warden.verdict.v2",
        "tool": {"name": "forge-warden", "version": "0.2.0"},
        "evaluated_on": "2026-11-14",
        "seals": "@SEALS@",
        "verdict": "@VERDICT@",
        "exit": {"code": "@CODE@", "name": "@NAME@", "action": "@ACTION@"},
        "summary": {"files": 1, "rules": 1, "@COUNTS@": 0},
        "rules": [{"id": "no-silent-suppression", "kind": "forbid_signal", "severity": "fail",
                   "outcome": "@VERDICT@", "why": "A silenced check hides the next real bug.",
                   "violations": [{"signal": "check_suppressed", "path": "a.rs", "message": "m"}]}],
        "expired_waivers": [], "unused_waivers": [], "notes": [],
    });
    let wait = wait
        .map(|dir| dir.display().to_string())
        .unwrap_or_default();
    write_program(
        path,
        &SCRIPT
            .replace("@WAIT@", &wait)
            .replace("@ANSWER@", answer)
            .replace("@VERDICT_DOCUMENT@", &verdict.to_string()),
    );
}

/// F3.b: the command's usage, the options this build refuses, and the schema of its
/// document, which `docs/reference/output-and-exit-codes.md` names.
#[test]
fn agent_check_says_its_usage_and_names_its_schema() {
    let scratch = Scratch::new("agent-check-usage");
    let home = scratch.join("home");
    let outside = scratch.join("outside");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    let usage = "deep-diff-forge agent check [--session ID | --base REV] [--judge PATH] [--policy NAME] [--ranker-dir DIR] [--json]";
    let (code, _, stderr) = finish(
        {
            let mut command = ddf(&home, &outside);
            command.arg("agent");
            command
        },
        "",
    );
    assert_eq!(code, 2);
    assert!(stderr.contains(usage), "{stderr}");
    for (args, said) in [
        (&["--range", "a..b"][..], "not in this build"),
        (&["--require-pass"][..], "not in this build"),
        (
            &["--session", "s", "--base", "HEAD"][..],
            "exclude each other",
        ),
        (&["--teleport"][..], "--teleport"),
    ] {
        let (code, stdout, stderr) = check(&home, &outside, args, None);
        assert_eq!((code, stdout.as_str()), (2, ""), "{args:?}: {stderr}");
        assert!(
            stderr.contains(said) && stderr.contains(usage),
            "{args:?}: {stderr}"
        );
    }
    let (code, stdout, _) = check(&home, &outside, &["--help"], None);
    assert_eq!(code, 0);
    assert!(
        stdout.contains("check [--session ID | --base REV]"),
        "{stdout}"
    );
    // outside any repository: the document, with the reason, and no unit
    let mut command = ddf(&home, &outside);
    command
        .args(["agent", "check", "--json"])
        .env("GIT_CEILING_DIRECTORIES", scratch.join(""));
    let (code, stdout, stderr) = finish(command, "");
    assert_eq!((code, stderr.as_str()), (0, ""));
    let doc: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(doc["schema"], "deep-diff-forge.agent-check.v0");
    assert_eq!(doc["units"], json!([]));
    assert!(
        doc["unjudged"]
            .as_str()
            .is_some_and(|why| why.contains("not in a Git repository")),
        "{doc}"
    );
    assert_eq!(doc["stop"]["whole_answer"], false);
    let table = include_str!("../../../docs/reference/output-and-exit-codes.md");
    assert!(
        table.contains("`agent check --json`")
            && table.contains("`deep-diff-forge.agent-check.v0`"),
        "the schema row is missing"
    );
}

/// Ends the stop it holds if the test ends before the stop does.
struct Running(std::process::Child);

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// F3.a: a check run in another PID namespace (a container) beside a live stop. The scratch
/// directory of a hook was named by its process id and removed by the next hook when `/proc`
/// had no such id: in another namespace `/proc` has none of the stop's, so the check removed
/// the stop's scratch in the middle of its judging. A scratch directory is owned by a lock now.
#[test]
fn a_check_in_another_pid_namespace_leaves_a_live_stop_s_scratch() {
    let namespaces = Command::new("unshare")
        .args([
            "--user",
            "--map-root-user",
            "--pid",
            "--fork",
            "--mount-proc",
            "true",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    if !namespaces {
        eprintln!("SKIPPED: user and PID namespaces cannot be made here (unshare)");
        return;
    }
    let scratch = Scratch::new("agent-check-pid-namespace");
    let (home, root) = repository(&scratch);
    let (slow, fast, wait) = (
        scratch.join("slow-judge"),
        scratch.join("judge"),
        scratch.join("wait"),
    );
    std::fs::create_dir(&wait).unwrap();
    judge(&slow, "fail", Some(&wait));
    judge(&fast, "pass", None);
    let (code, _, _) = hook(
        &home,
        "session-start",
        &[],
        &input("SessionStart", &root, "s1"),
    );
    assert_eq!(code, 0);
    silence_a_lint(&root);

    let (mut stop, stops) = a_stop_that_waits(&home, &root, &slow, &wait);

    let mut command = Command::new("unshare");
    command
        .args([
            "--user",
            "--map-root-user",
            "--pid",
            "--fork",
            "--mount-proc",
        ])
        .arg(this_binary())
        .args([
            "agent",
            "check",
            "--json",
            "--judge",
            fast.to_str().unwrap(),
        ])
        .current_dir(&root)
        .envs(ISOLATE)
        .env("HOME", &home)
        .env_remove("XDG_STATE_HOME")
        .env_remove("CLAUDE_PROJECT_DIR")
        .stdin(Stdio::null());
    let out = command.output().unwrap();
    let doc: Value = serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&out.stderr)));
    assert_eq!(doc["units"][0]["outcome"], "pass", "{doc}");
    assert!(
        stops.is_dir(),
        "a check in another PID namespace removed the live stop's scratch directory {}",
        stops.display()
    );

    let out = released(&mut stop, &wait);
    assert!(
        out.contains("\"decision\":\"block\""),
        "the stop's outcome changed: {out}"
    );
    assert!(!stops.exists(), "the stop left its scratch directory");
}

/// A stop of session `s1` started in the background with the judge `slow`, once that judge
/// has started: the stop, and its scratch directory (where the judge was given its patch).
fn a_stop_that_waits(home: &Path, root: &Path, slow: &Path, wait: &Path) -> (Running, PathBuf) {
    let mut command = ddf(home, home);
    command.args([
        "agent",
        "hook",
        "stop",
        "--client",
        "claude",
        "--judge",
        slow.to_str().unwrap(),
    ]);
    let mut stop = Running(command.spawn().unwrap());
    stop.0
        .stdin
        .take()
        .unwrap()
        .write_all(input("Stop", root, "s1").to_string().as_bytes())
        .unwrap();
    let started = wait.join("started");
    let until = Instant::now() + Duration::from_secs(20);
    while !started.exists() && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(50));
    }
    // the judge is given its patch in the stop's scratch directory
    let stops = PathBuf::from(
        std::fs::read_to_string(&started)
            .expect("the stop's judge did not start")
            .trim(),
    );
    let name = stops.file_name().unwrap().to_string_lossy().into_owned();
    assert!(
        name.starts_with("tmp-") && stops.is_dir(),
        "{}",
        stops.display()
    );
    (stop, stops)
}

/// Let the waiting judge of `stop` answer, and what the stop then printed.
fn released(stop: &mut Running, wait: &Path) -> String {
    std::fs::write(wait.join("release"), "").unwrap();
    let Running(child) = stop;
    let until = Instant::now() + Duration::from_secs(40);
    while child.try_wait().unwrap().is_none() && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(50));
    }
    let mut stdout = String::new();
    std::io::Read::read_to_string(child.stdout.as_mut().unwrap(), &mut stdout).unwrap();
    stdout
}
