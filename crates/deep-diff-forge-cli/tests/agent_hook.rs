//! `agent hook` from the outside: the binary is run the way Claude Code runs a hook (JSON on
//! stdin, a decision on stdout, exit 0) against real repositories, with a stand-in for the judge.
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

thread_local! {
    /// The home directory of the test on this thread: where its hooks keep the state of their
    /// sessions and look for the user's settings. Every test has its own, so a session id used
    /// by two tests is two sessions.
    static HOME: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

/// Make `home` in `scratch` the home of this test's hooks.
fn home_in(scratch: &Scratch) -> PathBuf {
    let home = scratch.join("home");
    std::fs::create_dir_all(&home).unwrap();
    HOME.with(|slot| *slot.borrow_mut() = Some(home.clone()));
    home
}

fn home() -> PathBuf {
    HOME.with(|slot| slot.borrow().clone())
        .expect("a test names its home first: repository() or home_in()")
}

/// Where the hooks of this test keep the state of their sessions.
fn sessions() -> PathBuf {
    home().join(".local/state/deep-diff-forge/hooks")
}

/// Run `deep-diff-forge agent hook <event> --client claude <extra…>` with `input` on stdin.
fn hook(event: &str, extra: &[&str], input: &Value) -> (i32, String, String) {
    hook_in(event, extra, input, |_| {})
}

/// [`hook`] in an environment `shape` changes. Without that it is the one a client gives a
/// hook, less what the machine running the tests would add: this test's own home, no project
/// directory named by a client, no configuration of the user or the system.
fn hook_in(
    event: &str,
    extra: &[&str],
    input: &Value,
    shape: impl FnOnce(&mut Command),
) -> (i32, String, String) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"));
    command
        .envs(ISOLATE)
        .env("HOME", home())
        .env_remove("XDG_STATE_HOME")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("CLAUDE_PROJECT_DIR")
        .args(["agent", "hook", event, "--client", "claude"])
        .args(extra)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    shape(&mut command);
    let mut child = command.spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.to_string().as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    (
        out.status.code().unwrap(),
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

fn input(event: &str, cwd: &Path, session: &str, active: bool) -> Value {
    json!({
        "hook_event_name": event,
        "session_id": session,
        "cwd": cwd,
        "stop_hook_active": active,
        "tool_name": "Bash",
    })
}

/// A repository with one commit: a source file and a policy. The scratch directory also
/// becomes the home of this test's hooks.
fn repository(scratch: &Scratch) -> PathBuf {
    home_in(scratch);
    let root = scratch.join("repo");
    std::fs::create_dir(&root).unwrap();
    git(&root, &["init", "--quiet"]);
    std::fs::write(root.join("a.rs"), "fn a() {}\nfn b() {}\n").unwrap();
    std::fs::write(
        root.join("warden.toml"),
        "# the policy at the base commit\n",
    )
    .unwrap();
    git(&root, &["add", "--all"]);
    git(&root, &["commit", "--quiet", "-m", "base"]);
    root
}

/// A stand-in for forge-warden, written by a child process: a file this process had open for
/// writing can still be busy when another test thread forks (ETXTBSY).
///
/// It appends its arguments and the policy it was given to `log`, prints `verdict` and exits
/// with the code in `exit_file` (so a test can change the answer between runs). Like the real
/// judge it prints a whole document that says one thing: sealed to the policy, the patch and
/// the ranker it was given, with the outcome, the exit triple and the summary that belong to
/// the exit code (0 pass, 7 fail, 8 hold).
fn judge(scratch: &Scratch, verdict: &Value) -> (PathBuf, PathBuf, PathBuf) {
    let (script, log, exit_file) = (
        scratch.join("judge"),
        scratch.join("judge.log"),
        scratch.join("judge.exit"),
    );
    std::fs::write(&exit_file, "0").unwrap();
    write_program(&script, &judge_script(&log, &exit_file, verdict));
    (script, log, exit_file)
}

/// The stand-in judge's script. The placeholders in `verdict` (see [`verdict`]) are filled in
/// when it runs.
fn judge_script(log: &Path, exit_file: &Path, verdict: &Value) -> String {
    const SCRIPT: &str = r#"#!/bin/sh
printf 'ARGS %s\n' "$*" >> '@LOG@'
policy=; patch=; ranker=
while [ $# -gt 0 ]; do
  case "$1" in
    --policy) policy="$2"; printf 'POLICY %s' "$(cat "$2")" >> '@LOG@'; printf '\n' >> '@LOG@';;
    --patch) patch="$2"; printf 'PATCH-FILES %s\n' "$(grep -c '^diff --git' "$2")" >> '@LOG@';;
    --ranker) ranker="$2";;
  esac
  shift
done
sum() { if command -v sha256sum >/dev/null 2>&1; then sha256sum < "$1"; else shasum -a 256 < "$1"; fi | cut -d' ' -f1; }
code="$(cat '@EXIT@')"
case "$code" in
  0) said=pass; name=pass; action=proceed; counts='"passed":1,"waived":0,"held":0,"failed":0';;
  7) said=fail; name=policy_fail; action=change_input; counts='"passed":0,"waived":0,"held":0,"failed":1';;
  8) said=hold; name=policy_hold; action=review_evidence; counts='"passed":0,"waived":0,"held":1,"failed":0';;
  *) said=none; name=none; action=none; counts='"passed":0,"waived":0,"held":0,"failed":0';;
esac
p="$(sum "$policy")"; f="$(sum "$patch")"; r="$(sum "$ranker")"
seals="{\"policy_sha256\":\"$p\",\"rank_sha256\":\"$f\",\"patch_sha256\":\"$f\",\"patch_facts_sha256\":null,\"provenance\":\"produced\",\"ranked_by\":{\"version\":\"stand-in\",\"sha256\":\"$r\"},\"git\":{\"path\":\"/usr/bin/git\",\"version\":\"git version 2.56.0\",\"sha256\":\"$p\",\"admission\":\"approved\",\"allowlist_sha256\":\"$f\"}}"
sed -e "s/\"@VERDICT@\"/\"$said\"/g" -e "s/\"@CODE@\"/$code/" -e "s/\"@NAME@\"/\"$name\"/" \
    -e "s/\"@ACTION@\"/\"$action\"/" -e "s|\"@SEALS@\"|$seals|" -e "s/\"@COUNTS@\":0/$counts/" <<'VERDICT'
@VERDICT_DOCUMENT@
VERDICT
exit "$code"
"#;
    SCRIPT
        .replace("@LOG@", &log.display().to_string())
        .replace("@EXIT@", &exit_file.display().to_string())
        .replace("@VERDICT_DOCUMENT@", &verdict.to_string())
}

/// Write an executable script through a child process (see [`judge`]).
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

/// A verdict document for the stand-in judge, with one rule. The document's outcome, its
/// rule's outcome, its exit triple, its summary and its seals are filled in by the judge when
/// it runs, from the exit code it is told to give: like the real judge, it never contradicts
/// itself.
fn verdict(outcome: &str, rule: &str, signal: &str, path: &str) -> Value {
    json!({
        "schema": "forge-warden.verdict.v2",
        "tool": {"name": "forge-warden", "version": "0.2.0"},
        "evaluated_on": "2026-11-14",
        "seals": "@SEALS@",
        "verdict": "@VERDICT@",
        "exit": {"code": "@CODE@", "name": "@NAME@", "action": "@ACTION@"},
        "summary": {"files": 1, "rules": 1, "@COUNTS@": 0},
        "rules": [
            {"id": rule, "kind": "forbid_signal",
             "severity": if outcome == "fail" { "fail" } else { "hold" },
             "outcome": "@VERDICT@",
             "why": "A silenced check hides the next real bug.",
             "violations": [{"signal": signal, "path": path, "message": "m"}]}
        ],
        "expired_waivers": [],
        "unused_waivers": [],
        "notes": [],
    })
}

#[test]
fn post_tool_says_a_new_finding_once_and_changes_nothing_in_the_work_tree() {
    let scratch = Scratch::new("hook-post-tool");
    let root = repository(&scratch);
    let event = input("PostToolUse", &root, "s1", false);

    // nothing changed: nothing said
    assert_eq!(
        hook("post-tool", &[], &event),
        (0, String::new(), String::new())
    );

    // an edit that silences a lint, as a shell command would leave it
    std::fs::write(
        root.join("a.rs"),
        "fn a() {}\n#[allow(dead_code)]\nfn b() {}\n",
    )
    .unwrap();
    let status = git(&root, &["status", "--porcelain=v1"]);
    let (code, stdout, stderr) = hook("post-tool", &[], &event);
    assert_eq!((code, stderr.as_str()), (0, ""), "{stdout}");
    let out: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(out["hookSpecificOutput"]["hookEventName"], "PostToolUse");
    let advice = out["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(
        advice.contains(
            "- check_suppressed at \"a.rs\" line 2 (new side, read as lexical). Accepted: "
        ),
        "{advice}"
    );
    assert!(
        !advice.contains("dead_code"),
        "the line's text was repeated: {advice}"
    );
    assert!(out.get("decision").is_none(), "advice must not block");
    // the hook left the working tree and the index as they were
    assert_eq!(git(&root, &["status", "--porcelain=v1"]), status);

    // the same finding is not said twice in the session, also after the line moved
    assert_eq!(hook("post-tool", &[], &event).1, "");
    std::fs::write(
        root.join("a.rs"),
        "// moved\nfn a() {}\n#[allow(dead_code)]\nfn b() {}\n",
    )
    .unwrap();
    assert_eq!(hook("post-tool", &[], &event).1, "");
    // another session has not been told yet
    let other = input("PostToolUse", &root, "s2", false);
    assert!(
        hook("post-tool", &[], &other)
            .1
            .contains("check_suppressed")
    );
    // a second finding in the first session is said, and only that one
    std::fs::write(
        root.join("a.rs"),
        "// moved\nfn a() {}\n#[allow(dead_code)]\nfn b() {}\nfn c() { unsafe { d() } }\n",
    )
    .unwrap();
    let (_, stdout, _) = hook("post-tool", &[], &event);
    assert!(
        stdout.contains("unsafe_added") && !stdout.contains("check_suppressed"),
        "{stdout}"
    );
    // a new file that is not tracked yet is part of the change
    std::fs::write(root.join("new.rs"), "#[allow(unused)]\nfn n() {}\n").unwrap();
    assert!(hook("post-tool", &[], &event).1.contains("\\\"new.rs\\\""));
}

#[test]
fn a_hook_outside_a_repository_or_with_bad_input_decides_nothing() {
    let scratch = Scratch::new("hook-nowhere");
    home_in(&scratch);
    let plain = scratch.join("plain");
    std::fs::create_dir(&plain).unwrap();
    for event in ["PostToolUse", "Stop", "SessionStart"] {
        let word = match event {
            "PostToolUse" => "post-tool",
            "Stop" => "stop",
            _ => "session-start",
        };
        assert_eq!(
            hook(word, &[], &input(event, &plain, "s", false)),
            (0, String::new(), String::new()),
            "{event}"
        );
    }
    // input for another event, or no JSON at all: the user is told, nothing is blocked
    for bad in [input("Stop", &plain, "s", false), json!("text")] {
        let (code, stdout, _) = hook("post-tool", &[], &bad);
        assert_eq!(code, 0);
        let out: Value = serde_json::from_str(&stdout).unwrap();
        let message = out["systemMessage"].as_str().unwrap();
        assert!(
            message.starts_with("deep-diff-forge: the hook did not run: ")
                && message.ends_with("Nothing was judged."),
            "{message}"
        );
        assert!(out.get("decision").is_none());
    }
}

/// Independent review of the hooks, HR-M-5. A command line a hook cannot read ended with exit
/// 2, which Claude Code reads from a `Stop` hook as a block with the usage text as its reason:
/// one mistyped option in a settings file held every stop. It is said like every other
/// failure now: exit 0, nothing on the error stream, a message for the user, and for a stop
/// the word UNJUDGED.
#[test]
fn a_command_line_a_hook_cannot_read_is_said_and_never_blocks() {
    let scratch = Scratch::new("hook-usage");
    let root = repository(&scratch);
    let run = |args: &[&str]| {
        let mut child = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
            .envs(ISOLATE)
            .env("HOME", home())
            .args(["agent", "hook"])
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        // the input a client would write: a hook that cannot read its arguments ignores it
        let _ = child
            .stdin
            .take()
            .unwrap()
            .write_all(input("Stop", &root, "s", false).to_string().as_bytes());
        let out = child.wait_with_output().unwrap();
        assert_eq!(out.status.code(), Some(0), "{args:?}");
        assert!(
            out.stderr.is_empty(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        system_message(std::str::from_utf8(&out.stdout).unwrap())
    };
    // the reviewer's four, for a stop: each is UNJUDGED with the reason and the usage
    for (args, why) in [
        (
            vec![
                "stop", "--client", "claude", "--judge", "/a", "--judge", "/b",
            ],
            "--judge given more than once",
        ),
        (
            vec!["stop", "--client", "claude", "--bogus"],
            "unknown option: --bogus",
        ),
        (
            vec!["stop", "--client", "claude", "--policy"],
            "--policy needs a value",
        ),
        (
            vec!["stop", "--client", "claude", "--ranker-dir", "/x", "extra"],
            "unexpected argument: extra",
        ),
        (vec!["stop"], "name one event and --client claude"),
        (
            vec!["stop", "--client", "codex"],
            "name one event and --client claude",
        ),
    ] {
        let said = run(&args);
        assert!(
            said.starts_with("deep-diff-forge: UNJUDGED. The stop hook did not run: ")
                && said.contains(why)
                && said.contains("(usage: deep-diff-forge agent hook ")
                && said.ends_with("that is not a pass."),
            "{args:?}: {said}"
        );
    }
    // for the other events, and when no event can be read: the hook did not run
    for (args, why) in [
        (
            vec!["post-tool", "--client", "claude", "--bogus"],
            "unknown option: --bogus",
        ),
        (vec!["session-start", "--client"], "--client needs a value"),
        (
            vec!["pre-tool", "--client", "claude"],
            "name one event and --client claude",
        ),
        (vec![], "name one event and --client claude"),
    ] {
        let said = run(&args);
        assert!(
            said.starts_with("deep-diff-forge: the hook did not run: ")
                && said.contains(why)
                && said.ends_with("Nothing was judged."),
            "{args:?}: {said}"
        );
    }
}

#[test]
fn a_stop_is_judged_against_the_policy_at_the_base_commit() {
    let scratch = Scratch::new("hook-stop");
    let root = repository(&scratch);
    // the session starts first, as it does with the printed settings: a stop that is its
    // session's first hook is never silent
    start(&root, "s1");
    let (script, log, exit_file) = judge(
        &scratch,
        &verdict("fail", "no-silent-suppression", "check_suppressed", "a.rs"),
    );
    let judge_arg = script.to_str().unwrap();
    let stop = |session: &str, active: bool| {
        hook(
            "stop",
            &["--judge", judge_arg],
            &input("Stop", &root, session, active),
        )
    };

    // nothing changed: nothing to judge, the judge is not started
    assert_eq!(stop("s1", false), (0, String::new(), String::new()));
    assert!(!log.exists());

    // the change also rewrites the policy; the judge must still get the committed one
    std::fs::write(
        root.join("a.rs"),
        "fn a() {}\n#[allow(dead_code)]\nfn b() {}\n",
    )
    .unwrap();
    std::fs::write(root.join("warden.toml"), "# loosened by the change\n").unwrap();

    // pass: the stop is allowed and nothing is said
    assert_eq!(stop("s1", false), (0, String::new(), String::new()));
    let seen = std::fs::read_to_string(&log).unwrap();
    assert!(
        seen.contains("POLICY # the policy at the base commit\n"),
        "{seen}"
    );
    assert!(!seen.contains("loosened"), "{seen}");
    assert!(seen.contains("PATCH-FILES 2\n"), "{seen}");
    assert!(
        seen.contains("ARGS check --policy ") && seen.contains(" --json\n"),
        "{seen}"
    );
    assert!(seen.contains(&format!(
        " --ranker {} ",
        std::fs::canonicalize(env!("CARGO_BIN_EXE_deep-diff-forge"))
            .unwrap()
            .display()
    )));

    // hold: allowed, reported for a person
    std::fs::write(&exit_file, "8").unwrap();
    let (code, stdout, _) = stop("s1", false);
    assert_eq!(code, 0);
    let out: Value = serde_json::from_str(&stdout).unwrap();
    assert!(out.get("decision").is_none(), "{out}");
    // the stand-in's document has no rule with outcome hold, so the hold names no rule
    assert!(
        out["systemMessage"]
            .as_str()
            .unwrap()
            .starts_with("deep-diff-forge: HOLD. ")
    );
}

/// A fail holds the stop once; the same failure does not hold it twice; and what is not a verdict
/// is unjudged. The hook's files stay inside the Git directory.
#[test]
fn a_failed_stop_blocks_once_and_an_unjudged_stop_never_blocks() {
    let scratch = Scratch::new("hook-stop-fail");
    let root = repository(&scratch);
    let (script, _, exit_file) = judge(
        &scratch,
        &verdict("fail", "no-silent-suppression", "check_suppressed", "a.rs"),
    );
    let judge_arg = script.to_str().unwrap();
    let stop = |session: &str, active: bool| {
        hook(
            "stop",
            &["--judge", judge_arg],
            &input("Stop", &root, session, active),
        )
    };
    std::fs::write(
        root.join("a.rs"),
        "fn a() {}\n#[allow(dead_code)]\nfn b() {}\n",
    )
    .unwrap();
    std::fs::write(root.join("warden.toml"), "# loosened by the change\n").unwrap();

    // fail: blocked, with the rule, the signal, the path and what answers it
    std::fs::write(&exit_file, "7").unwrap();
    let (code, stdout, _) = stop("s1", false);
    assert_eq!(code, 0);
    let out: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(out["decision"], "block");
    let reason = out["reason"].as_str().unwrap();
    assert!(
        reason
            .contains("- rule no-silent-suppression: A silenced check hides the next real bug.\n"),
        "{reason}"
    );
    assert!(
        reason.contains("check_suppressed in \"a.rs\". Accepted: Fix what the check reports."),
        "{reason}"
    );

    // the client continues because of the block and stops again with nothing repaired:
    // the same failure does not block twice
    let (_, stdout, _) = stop("s1", true);
    let out: Value = serde_json::from_str(&stdout).unwrap();
    assert!(out.get("decision").is_none(), "{out}");
    assert!(
        out["systemMessage"]
            .as_str()
            .unwrap()
            .starts_with("deep-diff-forge: FAIL STANDS. ")
    );
    // a new stop of the same session blocks again; so does another session's first stop
    assert_eq!(
        serde_json::from_str::<Value>(&stop("s1", false).1).unwrap()["decision"],
        "block"
    );
    assert_eq!(
        serde_json::from_str::<Value>(&stop("s2", true).1).unwrap()["decision"],
        "block"
    );

    // the judge ends in a way that is not a verdict: unjudged, allowed, said
    std::fs::write(&exit_file, "3").unwrap();
    let out: Value = serde_json::from_str(&stop("s1", false).1).unwrap();
    let message = out["systemMessage"].as_str().unwrap();
    assert!(
        message.starts_with("deep-diff-forge: UNJUDGED. ")
            && message.ends_with("That is not a pass."),
        "{message}"
    );

    // the hook's files are inside the Git directory; the working tree shows only the change
    assert_eq!(
        git(&root, &["status", "--porcelain=v1"]),
        " M a.rs\n M warden.toml\n"
    );
    assert!(root.join(".git/deep-diff-forge").is_dir());
    let left: Vec<String> = std::fs::read_dir(root.join(".git/deep-diff-forge"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("tmp-"))
        .collect();
    assert_eq!(
        left,
        Vec::<String>::new(),
        "a scratch directory was left behind"
    );
}

#[test]
fn a_stop_with_no_judge_or_no_policy_is_unjudged_and_never_blocks() {
    let scratch = Scratch::new("hook-unjudged");
    let root = repository(&scratch);
    std::fs::write(
        root.join("a.rs"),
        "fn a() {}\n#[allow(dead_code)]\nfn b() {}\n",
    )
    .unwrap();
    let said = |extra: &[&str], active: bool| -> String {
        let (code, stdout, _) = hook("stop", extra, &input("Stop", &root, "s", active));
        assert_eq!(code, 0);
        let out: Value = serde_json::from_str(&stdout).unwrap();
        assert!(out.get("decision").is_none(), "{out}");
        out["systemMessage"].as_str().unwrap().to_owned()
    };
    for active in [false, true] {
        let message = said(&["--judge", "/nonexistent/forge-warden"], active);
        assert!(
            message.starts_with("deep-diff-forge: UNJUDGED. "),
            "{message}"
        );
        assert!(message.contains("forge-warden was not found"), "{message}");
    }
    let message = said(&["--policy", "policy/other.toml"], false);
    assert!(
        message.contains("the base commit has no policy file \"policy/other.toml\""),
        "{message}"
    );
    // a policy that exists only in the working tree is not a policy
    std::fs::write(root.join("only-here.toml"), "x\n").unwrap();
    let message = said(&["--policy", "only-here.toml"], false);
    assert!(
        message.contains("the base commit has no policy file"),
        "{message}"
    );
}

/// When the judge gives no verdict, what it said about why reaches the user. A live run with a
/// real forge-warden found this: a host with no Git allowlist enrolled was reported as a ranker
/// failure, because only the exit code was read.
#[test]
fn an_unjudged_stop_passes_on_what_the_judge_said() {
    let scratch = Scratch::new("hook-judge-words");
    let root = repository(&scratch);
    start(&root, "s");
    std::fs::write(
        root.join("a.rs"),
        "fn a() {}\n#[allow(dead_code)]\nfn b() {}\n",
    )
    .unwrap();
    let script = scratch.join("judge");
    let body = "#!/bin/sh\necho '{\"exit\":3,\"name\":\"io\",\"message\":\"Git admission: no Git allowlist enrolled\",\"next\":\"check_environment\"}' >&2\nexit 3\n";
    let mut writer = Command::new("sh")
        .args(["-c", "cat > \"$1\" && chmod 755 \"$1\"", "sh"])
        .arg(&script)
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
    let (code, stdout, _) = hook(
        "stop",
        &["--judge", script.to_str().unwrap()],
        &input("Stop", &root, "s", false),
    );
    assert_eq!(code, 0);
    let out: Value = serde_json::from_str(&stdout).unwrap();
    assert!(out.get("decision").is_none(), "{out}");
    let message = out["systemMessage"].as_str().unwrap();
    assert!(
        message.contains(
            "the judge could not read its inputs or its environment (exit 3): io: Git admission: no Git allowlist enrolled"
        ),
        "{message}"
    );
    assert!(message.ends_with("That is not a pass."), "{message}");
}

/// The judge refuses this binary as a ranker (exit 4) and admits one from `--ranker-dir`.
#[test]
fn a_stop_offers_other_rankers_when_the_policy_does_not_admit_this_one() {
    let scratch = Scratch::new("hook-rankers");
    let root = repository(&scratch);
    start(&root, "s");
    std::fs::write(
        root.join("a.rs"),
        "fn a() {}\n#[allow(dead_code)]\nfn b() {}\n",
    )
    .unwrap();
    let rankers = scratch.join("rankers");
    std::fs::create_dir(&rankers).unwrap();
    std::fs::write(rankers.join("ddf-0.8.4"), "stand-in ranker\n").unwrap();
    // exit 4 unless the ranker is the admitted one; then the stand-in judge passes
    let (passing, _, _) = judge(&scratch, &verdict("pass", "size", "", ""));
    let log = scratch.join("calls.log");
    let script = scratch.join("admits-one-ranker");
    write_program(
        &script,
        &format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\ncase \"$*\" in *ddf-0.8.4*) exec '{}' \"$@\";; *) exit 4;; esac\n",
            log.display(),
            passing.display()
        ),
    );

    let event = input("Stop", &root, "s", false);
    let with_dir = hook(
        "stop",
        &[
            "--judge",
            script.to_str().unwrap(),
            "--ranker-dir",
            rankers.to_str().unwrap(),
        ],
        &event,
    );
    assert_eq!(
        with_dir,
        (0, String::new(), String::new()),
        "an admitted ranker passes"
    );
    assert_eq!(std::fs::read_to_string(&log).unwrap().lines().count(), 2);

    // without the directory no ranker is admitted: unjudged, and the reason says so
    let (code, stdout, _) = hook("stop", &["--judge", script.to_str().unwrap()], &event);
    assert_eq!(code, 0);
    let out: Value = serde_json::from_str(&stdout).unwrap();
    assert!(out.get("decision").is_none());
    assert!(
        out["systemMessage"]
            .as_str()
            .unwrap()
            .contains("the judge refused the policy, the ranking or the ranker (exit 4)"),
        "{out}"
    );
}

/// Independent review of the hooks, HR-L-3. The start of a session said "stops will be judged"
/// whenever a judge and a policy file were found, also where every stop was UNJUDGED. It
/// checks the chain now, the way `agent doctor --gate` does, and promises judgment only when
/// every link is ready; otherwise it says what is missing.
#[test]
fn session_start_states_the_contract_and_whether_a_stop_can_be_judged() {
    let (_scratch, root, home, script, _log, exit_file) = gate_fixture("hook-start");
    let event = input("SessionStart", &root, "s", false);
    let base = git(&root, &["rev-parse", "HEAD"]);
    let (code, stdout, _) = hook(
        "session-start",
        &["--judge", "/nonexistent/forge-warden"],
        &event,
    );
    assert_eq!(code, 0);
    assert!(
        stdout.starts_with("deep-diff-forge is watching the changes of this session.\n"),
        "{stdout}"
    );
    assert!(
        stdout.contains(&format!(
            "everything changed since this session began (commit {}), committed or not, is judged",
            &base[..7]
        )),
        "{stdout}"
    );
    assert!(
        stdout.contains(
            "The chain is not ready, so stops may be UNJUDGED: judge: forge-warden was not found"
        ),
        "{stdout}"
    );
    assert!(!stdout.contains("stops will be judged"), "{stdout}");

    // A judge and a policy are found, as before, but no settings file registers the stop
    // hook, and then the judge gives no verdict: this used to read "stops will be judged".
    let judge_arg = ["--judge", script.to_str().unwrap()];
    let (_, stdout, _) = hook("session-start", &judge_arg, &event);
    assert!(
        stdout.contains("The chain is not ready, so stops may be UNJUDGED: hooks_registered: no Claude Code settings file registers a stop hook"),
        "{stdout}"
    );
    std::fs::create_dir(home.join(".claude")).unwrap();
    std::fs::write(
        home.join(".claude/settings.json"),
        stop_hook_settings(&script),
    )
    .unwrap();
    std::fs::write(&exit_file, "3").unwrap();
    let (_, stdout, _) = hook("session-start", &judge_arg, &event);
    assert!(
        stdout.contains("The chain is not ready, so stops may be UNJUDGED: chain: ")
            && stdout.contains("(exit 3)"),
        "{stdout}"
    );
    assert!(!stdout.contains("stops will be judged"), "{stdout}");

    // every link ready: the promise, and a later start of the same session keeps its commit
    std::fs::write(&exit_file, "8").unwrap();
    let (_, stdout, _) = hook("session-start", &judge_arg, &event);
    assert!(
        stdout.contains("- The chain was checked the way `deep-diff-forge agent doctor --gate` checks it: stops will be judged.\n"),
        "{stdout}"
    );
    assert!(
        stdout.contains(&format!(
            "(commit {}, recorded when the session first started)",
            &base[..7]
        )),
        "{stdout}"
    );
    // a policy that does not protect its own file: judged, and said to be weak
    std::fs::write(&exit_file, "0").unwrap();
    let (_, stdout, _) = hook("session-start", &judge_arg, &event);
    assert!(
        stdout.contains("- Stops will be judged. One link is weak: policy_protects_itself: "),
        "{stdout}"
    );
    // another policy name: the link that is missing is named
    let (_, stdout, _) = hook(
        "session-start",
        &[judge_arg[0], judge_arg[1], "--policy", "none.toml"],
        &event,
    );
    assert!(
        stdout.contains("policy: the commit this session began from (")
            && stdout.contains("has no policy file \"none.toml\""),
        "{stdout}"
    );
    // the start left nothing behind in the repository
    assert_eq!(git(&root, &["status", "--porcelain"]), "");
}

#[test]
fn setup_prints_the_hook_settings_and_writes_nothing() {
    let run = |args: &[&str]| {
        let out = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
            .args(args)
            .output()
            .unwrap();
        (
            out.status.code().unwrap(),
            String::from_utf8(out.stdout).unwrap(),
            String::from_utf8(out.stderr).unwrap(),
        )
    };
    let (code, stdout, _) = run(&["agent", "setup", "--client", "claude", "--hooks"]);
    assert_eq!(code, 0);
    assert!(
        stdout.contains("# Claude Code hooks: merge into .claude/settings.json"),
        "{stdout}"
    );
    for word in ["session-start", "post-tool", "stop"] {
        assert!(
            stdout.contains(&format!(" agent hook {word} --client claude")),
            "{word}: {stdout}"
        );
    }
    let (code, stdout, _) = run(&["agent", "setup", "--client", "claude", "--hooks", "--json"]);
    assert_eq!(code, 0);
    let doc: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(doc["schema"], "deep-diff-forge.agent-setup.v0");
    assert_eq!(doc["hooks"]["PostToolUse"][0]["hooks"][0]["timeout"], 5);
    // without --hooks the document is as before
    let (_, stdout, _) = run(&["agent", "setup", "--client", "claude", "--json"]);
    let doc: Value = serde_json::from_str(&stdout).unwrap();
    assert!(doc.get("hooks").is_none());
    // hooks for Codex are refused, not silently left out
    let (code, stdout, stderr) = run(&["agent", "setup", "--client", "codex", "--hooks"]);
    assert_eq!((code, stdout.as_str()), (2, ""));
    assert!(
        stderr.contains("--hooks is for --client claude"),
        "{stderr}"
    );
}

/// `agent doctor --gate` in a repository, with `home` as the user's home.
fn doctor_gate(root: &Path, home: &Path, extra: &[&str]) -> (i32, Value, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .current_dir(root)
        .envs(ISOLATE)
        .env("HOME", home)
        .args(["agent", "doctor", "--gate", "--json"])
        .args(extra)
        .output()
        .unwrap();
    (
        out.status.code().unwrap(),
        serde_json::from_slice(&out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

const GATE: [&str; 10] = [
    "git",
    "repository",
    "policy",
    "judge",
    "chain",
    "policy_protects_itself",
    "patch",
    "hooks_enabled",
    "hooks_registered",
    "hook_timeout",
];

/// The gate checks of a doctor document, as `(name, ok)`.
fn gate_checks(doc: &Value) -> Vec<(String, bool)> {
    doc["checks"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| GATE.contains(&c["name"].as_str().unwrap()))
        .map(|c| {
            (
                c["name"].as_str().unwrap().to_owned(),
                c["ok"].as_bool().unwrap(),
            )
        })
        .collect()
}

/// Every link is ready but the registration: no stop hook, so no timeout of one either.
const NOT_REGISTERED: [bool; 10] = [true, true, true, true, true, true, true, true, false, false];

/// The ten gate checks with these answers, in order.
fn gate_names(ok: [bool; 10]) -> Vec<(String, bool)> {
    GATE.iter()
        .zip(ok)
        .map(|(name, ok)| ((*name).to_owned(), ok))
        .collect()
}

/// A repository, an empty home, and a judge that holds the probe (exit 8): the policy protects
/// its own file. Returns the scratch (kept alive), the repository, the home, the judge, its log
/// and the file that sets its exit code.
fn gate_fixture(name: &str) -> (Scratch, PathBuf, PathBuf, PathBuf, PathBuf, PathBuf) {
    let scratch = Scratch::new(name);
    let root = repository(&scratch);
    let home = home();
    let (script, log, exit_file) = judge(
        &scratch,
        &verdict(
            "hold",
            "policy-is-protected",
            "protected_path",
            "warden.toml",
        ),
    );
    std::fs::write(&exit_file, "8").unwrap();
    (scratch, root, home, script, log, exit_file)
}

/// A settings document whose stop hook is `command`.
fn settings_with(command: &str) -> String {
    json!({"hooks": {"Stop": [{"hooks": [{"type": "command", "command": command}]}]}}).to_string()
}

/// The stop command `agent setup --hooks` prints for this binary and `judge`.
fn stop_command(judge: &Path) -> String {
    format!(
        "'{}' agent hook stop --client claude --judge '{}'",
        this_binary().display(),
        std::fs::canonicalize(judge).unwrap().display()
    )
}

/// This binary by its own absolute path, as `agent setup --hooks` prints it.
fn this_binary() -> PathBuf {
    std::fs::canonicalize(env!("CARGO_BIN_EXE_deep-diff-forge")).unwrap()
}

/// A settings document that registers the stop hook with `judge`.
fn stop_hook_settings(judge: &Path) -> String {
    settings_with(&stop_command(judge))
}

fn detail(doc: &Value, name: &str) -> String {
    doc["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == name)
        .unwrap()["detail"]
        .as_str()
        .unwrap()
        .to_owned()
}

/// `agent doctor --gate` says, link by link, whether a stop here would be judged, and exits 7
/// unless every link is ready.
#[test]
fn doctor_gate_checks_every_link_of_a_judged_stop() {
    let (_scratch, root, home, script, log, _exit_file) = gate_fixture("doctor-gate");
    let judge_arg = ["--judge", script.to_str().unwrap()];

    // everything but the registration
    let (code, doc, stderr) = doctor_gate(&root, &home, &judge_arg);
    assert_eq!(doc["schema"], "deep-diff-forge.agent-doctor.v0");
    assert_eq!(gate_checks(&doc), gate_names(NOT_REGISTERED));
    assert_eq!((code, doc["ok"].as_bool()), (7, Some(false)), "{stderr}");
    assert!(stderr.contains("the chain is not ready"), "{stderr}");
    assert!(
        detail(&doc, "chain").starts_with("the judge gave a verdict (exit 8) with ranker "),
        "{doc:#}"
    );
    // the judge was given the committed policy and a patch of one file: the probe
    let seen = std::fs::read_to_string(&log).unwrap();
    assert!(
        seen.contains("POLICY # the policy at the base commit\n")
            && seen.contains("PATCH-FILES 1\n"),
        "{seen}"
    );
    // the probe changed nothing in the repository
    assert_eq!(git(&root, &["status", "--porcelain"]), "");

    // registered in the project's settings: the chain is ready
    std::fs::create_dir(root.join(".claude")).unwrap();
    let local = root.join(".claude/settings.local.json");
    // Review V4 round 2, VD4R2-H-1: text that only holds the words is no stop hook, and
    // neither is the right command for another program, another judge or another policy.
    let other_judge = root.parent().unwrap().join("other-judge");
    write_program(&other_judge, "#!/bin/sh\nexit 0\n");
    let right = stop_command(&script);
    // Review V4 round 3, VD4R3-H-1: a shell runs `A=ddf agent …` as the program `agent`, and a
    // link to this binary can be pointed elsewhere later. Both stand in the repository.
    std::os::unix::fs::symlink(this_binary(), root.join("A=ddf")).unwrap();
    std::os::unix::fs::symlink(this_binary(), root.join("ddf-link")).unwrap();
    let judge_path = std::fs::canonicalize(&script).unwrap();
    let by = |program: &str| {
        format!(
            "{program} agent hook stop --client claude --judge '{}'",
            judge_path.display()
        )
    };
    let not_by_its_path = "does not run this program by its own absolute path";
    for (command, why) in [
        ("true # agent hook stop".to_owned(), "is not the command"),
        ("echo agent hook stop".to_owned(), "is not the command"),
        (format!("{right} || true"), "is not the command"),
        (format!("true # {right}"), "is not the command"),
        (by("/bin/sh"), not_by_its_path),
        (by("A=ddf"), not_by_its_path),
        (by("./ddf-link"), not_by_its_path),
        (
            by(&format!("'{}'", root.join("ddf-link").display())),
            not_by_its_path,
        ),
        (by("deep-diff-forge"), not_by_its_path),
        (stop_command(&other_judge), "the judge checked here"),
        (
            format!(
                "'{}' agent hook stop --client claude",
                this_binary().display()
            ),
            "the judge checked here",
        ),
        (
            format!("{right} --policy other.toml"),
            "another policy file",
        ),
        (format!("{right} --ranker-dir /tmp"), "offers other rankers"),
    ] {
        std::fs::write(&local, settings_with(&command)).unwrap();
        let (code, doc, _) = doctor_gate(&root, &home, &judge_arg);
        assert_eq!(
            (code, gate_checks(&doc)),
            (7, gate_names(NOT_REGISTERED)),
            "{command}"
        );
        assert!(
            detail(&doc, "hooks_registered").contains(why),
            "{command}: {doc:#}"
        );
    }
    std::fs::write(&local, stop_hook_settings(&script)).unwrap();
    let (code, doc, stderr) = doctor_gate(&root, &home, &judge_arg);
    assert_eq!(gate_checks(&doc), gate_names([true; 10]));
    assert_eq!(
        (code, doc["ok"].as_bool(), stderr.as_str()),
        (0, Some(true), "")
    );
    // or in the user's
    std::fs::remove_file(&local).unwrap();
    std::fs::create_dir(home.join(".claude")).unwrap();
    std::fs::write(
        home.join(".claude/settings.json"),
        stop_hook_settings(&script),
    )
    .unwrap();
    let (code, doc, _) = doctor_gate(&root, &home, &judge_arg);
    assert_eq!((code, gate_checks(&doc)), (0, gate_names([true; 10])));

    // without --gate the document has none of these checks and the exit is as before
    let out = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .current_dir(&root)
        .args(["agent", "doctor", "--json"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(gate_checks(&doc), []);
}

/// Each link of the chain is broken on its own: the gate names it, exits 7, and reports what
/// depends on a missing link as not checked, never as fine.
#[test]
fn doctor_gate_names_the_link_that_is_missing() {
    let (scratch, root, home, script, _log, exit_file) = gate_fixture("doctor-gate-broken");
    let judge_arg = ["--judge", script.to_str().unwrap()];
    std::fs::create_dir(home.join(".claude")).unwrap();
    std::fs::write(
        home.join(".claude/settings.json"),
        stop_hook_settings(&script),
    )
    .unwrap();
    let (code, doc, _) = doctor_gate(&root, &home, &judge_arg);
    assert_eq!((code, gate_checks(&doc)), (0, gate_names([true; 10])));

    // a policy that lets a change to itself pass is not ready
    std::fs::write(&exit_file, "0").unwrap();
    let (code, doc, _) = doctor_gate(&root, &home, &judge_arg);
    assert_eq!(
        (code, gate_checks(&doc)),
        (
            7,
            gate_names([true, true, true, true, true, false, true, true, true, true])
        )
    );
    assert!(
        detail(&doc, "policy_protects_itself").contains("any change could loosen the policy"),
        "{doc:#}"
    );

    // a judge that gives no verdict
    std::fs::write(&exit_file, "3").unwrap();
    let (code, doc, _) = doctor_gate(&root, &home, &judge_arg);
    assert_eq!(
        (code, gate_checks(&doc)),
        (
            7,
            gate_names([true, true, true, true, false, false, true, true, true, true])
        )
    );
    assert!(
        detail(&doc, "policy_protects_itself").starts_with("not checked: "),
        "{doc:#}"
    );

    // no judge, and no policy under the name asked for
    std::fs::write(&exit_file, "8").unwrap();
    let missing = scratch.join("no-such-judge");
    let (code, doc, _) = doctor_gate(&root, &home, &["--judge", missing.to_str().unwrap()]);
    assert_eq!(
        (code, gate_checks(&doc)),
        (
            7,
            gate_names([
                true, true, true, false, false, false, true, true, false, false
            ])
        )
    );
    let (code, doc, _) = doctor_gate(
        &root,
        &home,
        &[judge_arg[0], judge_arg[1], "--policy", "other.toml"],
    );
    assert_eq!(
        (code, gate_checks(&doc)),
        (
            7,
            gate_names([
                true, true, false, true, false, false, true, true, false, false
            ])
        )
    );
    assert!(
        detail(&doc, "chain").starts_with("not checked: "),
        "{doc:#}"
    );

    // outside a repository
    let outside = scratch.join("outside");
    std::fs::create_dir(&outside).unwrap();
    let (code, doc, _) = doctor_gate(&outside, &home, &judge_arg);
    assert_eq!(
        (code, gate_checks(&doc)),
        (
            7,
            gate_names([
                true, false, false, true, false, false, false, false, false, false
            ])
        )
    );
}

/// Run a hook with `path` as the whole `PATH` (plus the system's, so `git` and `sh` are found).
fn hook_on_path(event: &str, path: &[&Path], input: &Value) -> (i32, String, String) {
    let mut dirs: Vec<PathBuf> = path.iter().map(|dir| dir.to_path_buf()).collect();
    dirs.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    hook_in(event, &[], input, |command| {
        command.env("PATH", std::env::join_paths(dirs).unwrap());
    })
}

fn system_message(stdout: &str) -> String {
    let out: Value = serde_json::from_str(stdout).unwrap_or_else(|e| panic!("{e}: {stdout:?}"));
    assert!(out.get("decision").is_none(), "{out}");
    out["systemMessage"].as_str().unwrap().to_owned()
}

/// Review V4, VD4-H-1: a program that is merely named `forge-warden` and exits 0 passed a stop
/// in silence. An exit code alone is no verdict now, and a judge inside the repository it would
/// judge is not run at all.
#[test]
fn a_program_that_is_only_named_like_the_judge_cannot_pass_a_stop() {
    let scratch = Scratch::new("hook-fake-judge");
    let root = repository(&scratch);
    std::fs::write(
        root.join("a.rs"),
        "fn a() {}\n#[allow(dead_code)]\nfn b() {}\n",
    )
    .unwrap();
    let event = input("Stop", &root, "s", false);

    // Vesper's reproduction: first on PATH, exits 0, prints nothing
    let bin = scratch.join("bin");
    std::fs::create_dir(&bin).unwrap();
    write_program(&bin.join("forge-warden"), "#!/bin/sh\nexit 0\n");
    let (code, stdout, _) = hook_on_path("stop", &[&bin], &event);
    assert_eq!(code, 0);
    let said = system_message(&stdout);
    assert!(
        said.contains("UNJUDGED")
            && said.contains("the judge exited 0 (pass) but that is not a verdict: it printed no verdict document"),
        "{said}"
    );

    // one that prints a whole pass document, but not one sealed to this policy and this patch
    let zeros = "0".repeat(64);
    let unsealed = json!({
        "schema": "forge-warden.verdict.v2",
        "tool": {"name": "forge-warden", "version": "0.2.0"},
        "evaluated_on": "2026-11-14",
        "seals": {"policy_sha256": zeros, "rank_sha256": zeros, "patch_sha256": zeros,
                  "patch_facts_sha256": null, "provenance": "produced",
                  "ranked_by": {"version": "x", "sha256": zeros},
                  "git": {"path": "/usr/bin/git", "version": "v", "sha256": zeros,
                          "admission": "approved", "allowlist_sha256": zeros}},
        "verdict": "pass",
        "exit": {"code": 0, "name": "pass", "action": "proceed"},
        "summary": {"files": 1, "rules": 1, "passed": 1, "waived": 0, "held": 0, "failed": 0},
        "rules": [{"id": "size", "kind": "max_score", "severity": "hold", "outcome": "pass",
                   "why": "w", "violations": []}],
        "expired_waivers": [],
        "unused_waivers": [],
        "notes": [],
    });
    write_program(
        &bin.join("forge-warden"),
        &format!("#!/bin/sh\ncat <<'VERDICT'\n{unsealed}\nVERDICT\nexit 0\n"),
    );
    let (_, stdout, _) = hook_on_path("stop", &[&bin], &event);
    let said = system_message(&stdout);
    assert!(
        said.contains("its verdict is not sealed to the policy it was given"),
        "{said}"
    );
    // review V4 rounds 2 and 3: the same sealed to nothing, with half its members, saying two
    // things, or as the reviewer wrote it (a wrong date, no Git, an empty summary and no
    // rules, under the exit name of a fail) is not a verdict either
    let theirs = {
        let mut theirs = unsealed.clone();
        theirs["evaluated_on"] = json!("not-a-date");
        theirs["seals"].as_object_mut().unwrap().remove("git");
        theirs["seals"]["ranked_by"] = json!({});
        theirs["summary"] = json!({});
        theirs["rules"] = json!([]);
        theirs["exit"]["name"] = json!("policy_fail");
        theirs
    };
    for (text, why) in [
        (
            theirs.to_string(),
            "is not a whole forge-warden.verdict.v2",
        ),
        (
            r#"{"schema":"forge-warden.verdict.v2","verdict":"pass","seals":{"provenance":"produced"},"rules":[]}"#.to_owned(),
            "is not a whole forge-warden.verdict.v2",
        ),
        (
            unsealed.to_string().replace(r#""code":0"#, r#""code":7"#),
            "`verdict` and `exit` are missing or do not belong together",
        ),
        (
            unsealed
                .to_string()
                .replacen(r#""verdict":"pass""#, r#""verdict":"fail","verdict":"pass""#, 1),
            "no verdict document that can be read",
        ),
    ] {
        write_program(
            &bin.join("forge-warden"),
            &format!("#!/bin/sh\ncat <<'VERDICT'\n{text}\nVERDICT\nexit 0\n"),
        );
        let (_, stdout, _) = hook_on_path("stop", &[&bin], &event);
        let said = system_message(&stdout);
        assert!(said.contains("UNJUDGED") && said.contains(why), "{why}: {said}");
    }

    // a real verdict from the same place passes in silence: the control
    let (script, _, _) = judge(&scratch, &verdict("pass", "size", "", ""));
    // copied by a child process, like every program a test starts afterwards (ETXTBSY)
    let copied = Command::new("cp")
        .arg(&script)
        .arg(bin.join("forge-warden.new"))
        .status()
        .unwrap();
    assert!(copied.success());
    std::fs::rename(bin.join("forge-warden.new"), bin.join("forge-warden")).unwrap();
    assert_eq!(
        hook_on_path("stop", &[&bin], &event),
        (0, String::new(), String::new())
    );
}

/// The other control of VD4-H-1: a judge inside the repository it would judge is not run, on
/// `PATH` or by name, and a `PATH` entry that is not absolute is not searched.
#[test]
fn a_judge_inside_the_repository_it_would_judge_is_not_run() {
    let scratch = Scratch::new("hook-judge-inside");
    let root = repository(&scratch);
    std::fs::write(
        root.join("a.rs"),
        "fn a() {}\n#[allow(dead_code)]\nfn b() {}\n",
    )
    .unwrap();
    let event = input("Stop", &root, "s", false);
    let (script, _, _) = judge(&scratch, &verdict("pass", "size", "", ""));
    let inside = root.join("tools");
    std::fs::create_dir(&inside).unwrap();
    std::fs::copy(&script, inside.join("forge-warden")).unwrap();
    let (_, stdout, _) = hook_on_path("stop", &[&inside], &event);
    let said = system_message(&stdout);
    assert!(
        said.contains("is inside the repository it would judge"),
        "{said}"
    );
    let named = inside.join("forge-warden");
    let (_, stdout, _) = hook("stop", &["--judge", named.to_str().unwrap()], &event);
    assert!(
        system_message(&stdout).contains("is inside the repository it would judge"),
        "{stdout}"
    );
    // nor one inside the Git directory
    let hidden = root.join(".git/forge-warden");
    std::fs::copy(&script, &hidden).unwrap();
    let (_, stdout, _) = hook("stop", &["--judge", hidden.to_str().unwrap()], &event);
    assert!(
        system_message(&stdout).contains("is inside the repository it would judge"),
        "{stdout}"
    );
    // a PATH entry that is not absolute is not searched: here it would be the repository
    let (_, stdout, _) = hook_on_path("stop", &[Path::new("tools")], &event);
    let said = system_message(&stdout);
    assert!(
        said.contains("forge-warden was not found") || said.contains("is not a verdict"),
        "{said}"
    );
    assert!(!said.contains("inside the repository"), "{said}");
}

/// Review V4, VD4-M-1: each directory of the hooks must be a directory of its own (the scratch
/// directory's parent under the Git directory, and the directory of the sessions' state in the
/// user's state directory), and a state file is read only if it is a small regular file.
#[test]
fn the_state_is_kept_in_a_directory_of_its_own_and_read_with_a_bound() {
    let scratch = Scratch::new("hook-state-dir");
    let root = repository(&scratch);
    std::fs::write(
        root.join("a.rs"),
        "fn a() {}\n#[allow(dead_code)]\nfn b() {}\n",
    )
    .unwrap();
    let event = input("PostToolUse", &root, "s", false);
    let own = root.join(".git/deep-diff-forge");
    let sessions = sessions();

    // a symbolic link where a directory should be: nothing is written through it
    let elsewhere = scratch.join("elsewhere");
    std::fs::create_dir(&elsewhere).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &own).unwrap();
    let (code, stdout, _) = hook("post-tool", &[], &event);
    assert_eq!(code, 0);
    let said = system_message(&stdout);
    assert!(
        said.contains("the hook did not run") && said.contains("is not a directory of its own"),
        "{said}"
    );
    assert_eq!(std::fs::read_dir(&elsewhere).unwrap().count(), 0);
    std::fs::remove_file(&own).unwrap();
    // the same for the sessions' directory; the session used here is another one, whose
    // state was never written
    let other = input("PostToolUse", &root, "other", false);
    std::fs::rename(&sessions, scratch.join("sessions-kept")).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &sessions).unwrap();
    let (code, stdout, _) = hook("post-tool", &[], &other);
    assert_eq!(code, 0);
    let said = system_message(&stdout);
    assert!(
        said.contains("the hook did not run") && said.contains("is not a directory of its own"),
        "{said}"
    );
    assert_eq!(std::fs::read_dir(&elsewhere).unwrap().count(), 0);
    std::fs::remove_file(&sessions).unwrap();
    std::fs::rename(scratch.join("sessions-kept"), &sessions).unwrap();

    // each directory is made for this user only, and the state file too
    let (_, stdout, _) = hook("post-tool", &[], &event);
    assert!(stdout.contains("check_suppressed"), "{stdout}");
    let mode = |path: &Path| {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::symlink_metadata(path)
            .unwrap()
            .permissions()
            .mode()
            & 0o777
    };
    assert_eq!(mode(&own), 0o700);
    assert_eq!(mode(&sessions), 0o700);
    // nothing of the hook stays in the repository's directory once it is done
    assert_eq!(std::fs::read_dir(&own).unwrap().count(), 0);
    let state: Vec<PathBuf> = std::fs::read_dir(&sessions)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(state.len(), 1, "{state:?}");
    assert_eq!(mode(&state[0]), 0o600);
    // said once: the state was read back
    assert_eq!(hook("post-tool", &[], &event).1, "");

    // a state file that is a link is not followed: the finding is said again, and the link is
    // replaced by a file, its target untouched
    let target = scratch.join("target.json");
    std::fs::copy(&state[0], &target).unwrap();
    let before = std::fs::read(&target).unwrap();
    std::fs::remove_file(&state[0]).unwrap();
    std::os::unix::fs::symlink(&target, &state[0]).unwrap();
    let (_, stdout, _) = hook("post-tool", &[], &event);
    assert!(stdout.contains("check_suppressed"), "{stdout}");
    assert!(std::fs::symlink_metadata(&state[0]).unwrap().is_file());
    assert_eq!(std::fs::read(&target).unwrap(), before);

    // a state file over the bound is not read: the same as no state
    let huge = std::fs::File::create(&state[0]).unwrap();
    huge.set_len(64 * 1024 * 1024).unwrap();
    drop(huge);
    let (_, stdout, _) = hook("post-tool", &[], &event);
    assert!(stdout.contains("check_suppressed"), "{stdout}");
    assert!(std::fs::metadata(&state[0]).unwrap().len() < 4096);
}

/// `agent setup --hooks` names the judge it finds by its absolute path, in the commands that
/// use one, and says so when there is none.
#[test]
fn setup_names_the_judge_it_finds_by_its_path() {
    let scratch = Scratch::new("hook-setup-judge");
    let bin = scratch.join("bin");
    std::fs::create_dir(&bin).unwrap();
    write_program(&bin.join("forge-warden"), "#!/bin/sh\nexit 0\n");
    let run = |path: &Path, json: bool| {
        let mut args = vec!["agent", "setup", "--client", "claude", "--hooks"];
        if json {
            args.push("--json");
        }
        let out = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
            .env("PATH", path)
            .args(args)
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(0));
        String::from_utf8(out.stdout).unwrap()
    };
    let found = std::fs::canonicalize(bin.join("forge-warden")).unwrap();
    let doc: Value = serde_json::from_str(&run(&bin, true)).unwrap();
    assert_eq!(doc["judge"], found.to_str().unwrap());
    for (event, judged) in [
        ("SessionStart", true),
        ("Stop", true),
        ("PostToolUse", false),
    ] {
        let command = doc["hooks"][event][0]["hooks"][0]["command"]
            .as_str()
            .unwrap();
        assert_eq!(
            command.ends_with(&format!(" --judge {}", found.display())),
            judged,
            "{event}: {command}"
        );
    }
    assert!(
        run(&bin, false).contains("# The judge is named by its path as found now: "),
        "the text form names the judge"
    );
    // none on PATH: no --judge, and the text says what follows
    let empty = scratch.join("empty");
    std::fs::create_dir(&empty).unwrap();
    let doc: Value = serde_json::from_str(&run(&empty, true)).unwrap();
    assert!(doc["judge"].is_null());
    assert!(
        !doc["hooks"]["Stop"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .contains("--judge")
    );
    assert!(run(&empty, false).contains("# forge-warden was not found on PATH"));
}

/// The stop hook's helper lists the candidate rankers and hashes each: this binary first, then
/// the files of the ranker directory by name, with the reason for one that is not hashed.
#[test]
fn the_seal_helper_lists_each_ranker_with_its_digest_or_its_refusal() {
    let scratch = Scratch::new("hook-seal-helper");
    let dir = scratch.join("rankers");
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("a-small"), "abc").unwrap();
    std::fs::File::create(dir.join("b-huge"))
        .unwrap()
        .set_len(256 * 1024 * 1024 + 1)
        .unwrap();
    std::fs::create_dir(dir.join("c-directory")).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .args(["agent", "hook", "seal-rankers", "--ranker-dir"])
        .arg(&dir)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let list: Value = serde_json::from_slice(&out.stdout).unwrap();
    let list = list.as_array().unwrap();
    assert_eq!(list.len(), 3, "{list:?}");
    assert_eq!(list[0]["path"], this_binary().to_str().unwrap());
    assert_eq!(list[0]["sha256"].as_str().unwrap().len(), 64);
    assert_eq!(
        list[1]["sha256"],
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(list[2]["refused"], "is over 256 MiB");
    assert!(list[2].get("sha256").is_none());
}

/// Review V4 round 7, VD4R7-L-1: a file whose name is not UTF-8 cannot be written in the helper's
/// list, so it takes none of the seven places: the seven files after it are all offered. Before,
/// it took the first place, came back as a null path, and the seventh file was never tried.
/// (Linux only: other systems refuse to create such a name.)
#[cfg(target_os = "linux")]
#[test]
fn a_name_the_helper_cannot_write_takes_no_place() {
    use std::os::unix::ffi::OsStrExt as _;
    let scratch = Scratch::new("hook-seal-names");
    let dir = scratch.join("rankers");
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join(std::ffi::OsStr::from_bytes(b"a\xff")), "x").unwrap();
    for index in 1..=8 {
        std::fs::write(dir.join(format!("b{index}")), "abc").unwrap();
    }
    let out = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .args(["agent", "hook", "seal-rankers", "--ranker-dir"])
        .arg(&dir)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let list: Value = serde_json::from_slice(&out.stdout).unwrap();
    let names: Vec<String> = list
        .as_array()
        .unwrap()
        .iter()
        .skip(1)
        .map(|entry| {
            let path = entry["path"].as_str().expect("every path is a string");
            path.rsplit('/').next().unwrap().to_owned()
        })
        .collect();
    assert_eq!(names, ["b1", "b2", "b3", "b4", "b5", "b6", "b7"]);
}

/// Review V4 round 8, the rest of VD4R7-L-1: this binary's own path can be the one that cannot
/// be written. Started from a copy whose name is not UTF-8, the helper lists eight files of the
/// directory and no null path. Before, it listed itself as null and stopped at `b7`.
#[cfg(target_os = "linux")]
#[test]
fn a_binary_whose_own_path_cannot_be_written_takes_no_place() {
    use std::os::unix::ffi::OsStrExt as _;
    let scratch = Scratch::new("hook-seal-own-name");
    let dir = scratch.join("rankers");
    std::fs::create_dir(&dir).unwrap();
    for index in 1..=9 {
        std::fs::write(dir.join(format!("b{index}")), "abc").unwrap();
    }
    let copy = scratch.join(std::ffi::OsStr::from_bytes(b"ddf-\xff"));
    // Copied by a child process: a file this process had open for writing can still be busy
    // when another test thread forks, and starting it then fails (ETXTBSY).
    let copied = Command::new("cp")
        .arg(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .arg(&copy)
        .status()
        .unwrap();
    assert!(copied.success());
    let out = Command::new(&copy)
        .args(["agent", "hook", "seal-rankers", "--ranker-dir"])
        .arg(&dir)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let list: Value = serde_json::from_slice(&out.stdout).unwrap();
    let names: Vec<String> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            let path = entry["path"].as_str().expect("every path is a string");
            path.rsplit('/').next().unwrap().to_owned()
        })
        .collect();
    assert_eq!(names, ["b1", "b2", "b3", "b4", "b5", "b6", "b7", "b8"]);
}

// ---- Independent review of the hooks (2026-11-14, NO-GO after nine rounds) and its fixes ----

/// A stand-in judge that fails every change it is shown (exit 7, one failing rule).
fn failing_judge(scratch: &Scratch) -> (PathBuf, PathBuf, PathBuf) {
    let (script, log, exit_file) = judge(
        scratch,
        &verdict("fail", "no-silent-suppression", "check_suppressed", "a.rs"),
    );
    std::fs::write(&exit_file, "7").unwrap();
    (script, log, exit_file)
}

/// The change the tests of a stop make: a silenced lint in `a.rs`.
fn silence_a_lint(root: &Path) {
    std::fs::write(
        root.join("a.rs"),
        "fn a() {}\n#[allow(dead_code)]\nfn b() {}\n",
    )
    .unwrap();
}

/// Start `session` in `root`, as Claude Code does before anything else of a session.
fn start(root: &Path, session: &str) -> String {
    let (code, stdout, stderr) = hook(
        "session-start",
        &[],
        &input("SessionStart", root, session, false),
    );
    assert_eq!((code, stderr.as_str()), (0, ""), "{stdout}");
    stdout
}

/// What a stop of `session` decides when the session's directory is `cwd`.
fn stop_from(cwd: &Path, session: &str, judge: &Path) -> Value {
    let (code, stdout, stderr) = hook(
        "stop",
        &["--judge", judge.to_str().unwrap()],
        &input("Stop", cwd, session, false),
    );
    assert_eq!((code, stderr.as_str()), (0, ""), "{stdout}");
    if stdout.is_empty() {
        Value::Null
    } else {
        serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("{e}: {stdout:?}"))
    }
}

/// The reason a stop was blocked with; panics when it was not blocked.
fn blocked(decision: &Value) -> &str {
    assert_eq!(decision["decision"], "block", "{decision}");
    decision["reason"].as_str().unwrap()
}

/// The message of a stop that was allowed with something said; panics on a block or silence.
fn allowed(decision: &Value) -> &str {
    assert!(decision.get("decision").is_none(), "{decision}");
    decision["systemMessage"]
        .as_str()
        .unwrap_or_else(|| panic!("the stop said nothing: {decision}"))
}

/// Independent review of the hooks, HR-H-1. The repository that was judged was the one the
/// session's current directory was in at the stop. After `cd` into a nested repository,
/// another repository, a directory outside any, or one that was gone, the project's change
/// was not judged and the stop passed with no output. The project is fixed by the session's
/// first hook now (or named by the client), and is judged wherever the session has gone.
#[test]
fn the_project_is_judged_wherever_the_session_has_gone() {
    let scratch = Scratch::new("hook-project");
    let root = repository(&scratch);
    let (script, log, _) = failing_judge(&scratch);
    let outside = scratch.join("outside");
    std::fs::create_dir(&outside).unwrap();
    let other = scratch.join("other-repository");
    std::fs::create_dir(&other).unwrap();
    git(&other, &["init", "--quiet"]);
    std::fs::write(other.join("x"), "x\n").unwrap();
    git(&other, &["add", "x"]);
    git(&other, &["commit", "--quiet", "-m", "x"]);
    // a nested repository with a commit of its own, as a vendored checkout or a fixture is
    let nested = root.join("vendor");
    std::fs::create_dir(&nested).unwrap();
    git(&nested, &["init", "--quiet"]);
    std::fs::write(nested.join("x"), "x\n").unwrap();
    git(&nested, &["add", "x"]);
    git(&nested, &["commit", "--quiet", "-m", "x"]);
    std::fs::create_dir(root.join("sub")).unwrap();

    for (session, cwd) in [
        ("top", root.clone()),
        ("nested", nested.clone()),
        ("gone", root.join("gone")),
        ("outside", outside.clone()),
        ("other", other.clone()),
        ("sub", root.join("sub")),
        ("git", root.join(".git")),
    ] {
        // every session starts in the project, on a clean tree, then makes the change
        git(&root, &["checkout", "--quiet", "--", "a.rs"]);
        start(&root, session);
        silence_a_lint(&root);
        let decision = stop_from(&cwd, session, &script);
        let reason = blocked(&decision);
        assert!(
            reason.contains("check_suppressed in \"a.rs\""),
            "{session}: {reason}"
        );
        // one tree was judged: the session is in no other working tree of this repository
        assert!(!reason.contains("Working tree "), "{session}: {reason}");
    }

    // With no hook before the stop, the directory the client names as the project decides.
    let named = |session: &str, cwd: &Path, project: &Path| {
        let (code, stdout, _) = hook_in(
            "stop",
            &["--judge", script.to_str().unwrap()],
            &input("Stop", cwd, session, false),
            |command| {
                command.env("CLAUDE_PROJECT_DIR", project);
            },
        );
        assert_eq!(code, 0);
        stdout
    };
    for (session, cwd) in [("named-outside", &outside), ("named-nested", &nested)] {
        let out: Value = serde_json::from_str(&named(session, cwd, &root)).unwrap();
        assert!(
            blocked(&out).contains("check_suppressed in \"a.rs\""),
            "{out}"
        );
    }
    // a project directory that is in no repository, or not absolute, does not decide: the
    // session's own directory does
    for (session, project) in [
        ("named-plain", outside.as_path()),
        ("named-dot", Path::new(".")),
    ] {
        let out: Value = serde_json::from_str(&named(session, &root, project)).unwrap();
        assert!(
            blocked(&out).contains("check_suppressed in \"a.rs\""),
            "{out}"
        );
    }
    // the judge was shown the project's change every time
    let seen = std::fs::read_to_string(&log).unwrap();
    assert_eq!(seen.matches("ARGS check ").count(), 11, "{seen}");
    assert!(!seen.contains("PATCH-FILES 0"), "{seen}");

    // A nested repository with no commit cannot be staged by Git at all (`git init` in a new
    // directory). The stop is then UNJUDGED and says so; before, it was silent.
    let empty = root.join("scratch");
    std::fs::create_dir(&empty).unwrap();
    git(&empty, &["init", "--quiet"]);
    std::fs::write(empty.join("y"), "y\n").unwrap();
    start(&root, "empty-nested");
    let decision = stop_from(&empty, "empty-nested", &script);
    let said = allowed(&decision);
    assert!(
        said.starts_with("deep-diff-forge: UNJUDGED. ")
            && said.contains("the change could not be read from Git"),
        "{said}"
    );
}

/// Independent review of the hooks, HR-H-1, the linked working tree. A session that entered
/// another working tree of its repository (`git worktree add`, Claude Code's own worktrees)
/// has two trees. Both are judged against the commit the session began from; one message names
/// each. Before, only the tree the session's directory was in was looked at.
#[test]
fn a_linked_working_tree_the_session_entered_is_judged_too() {
    let scratch = Scratch::new("hook-worktree");
    let root = repository(&scratch);
    let (script, log, _) = failing_judge(&scratch);
    let linked = scratch.join("linked");
    git(
        &root,
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "side",
            linked.to_str().unwrap(),
        ],
    );
    let linked = std::fs::canonicalize(&linked).unwrap();
    let top = std::fs::canonicalize(&root).unwrap();
    // a tree is named as every path is: quoted, and cut at 120 characters
    let tree = |path: &Path, outcome: &str| {
        let whole = path.display().to_string();
        let mut cut: String = whole.chars().take(120).collect();
        if cut.len() < whole.len() {
            cut.push('…');
        }
        format!("Working tree \"{cut}\": {outcome}\n")
    };

    // the change is in the linked tree, where the session is; the project itself is clean
    start(&root, "s1");
    silence_a_lint(&linked);
    let decision = stop_from(&linked, "s1", &script);
    let reason = blocked(&decision);
    assert!(
        reason.contains(&tree(&top, "no change")) && reason.contains(&tree(&linked, "FAIL")),
        "{reason}"
    );
    assert!(reason.contains("check_suppressed in \"a.rs\""), "{reason}");
    // from a subdirectory of the linked tree as well
    std::fs::create_dir(linked.join("deep")).unwrap();
    start(&root, "s1b");
    let decision = stop_from(&linked.join("deep"), "s1b", &script);
    assert!(
        blocked(&decision).contains(&tree(&linked, "FAIL")),
        "{decision}"
    );
    // committed there: still judged, from the commit the session began from
    start(&root, "s2");
    git(
        &linked,
        &["commit", "--quiet", "-am", "work in the linked tree"],
    );
    let decision = stop_from(&linked, "s2", &script);
    assert!(
        blocked(&decision).contains(&tree(&linked, "FAIL")),
        "{decision}"
    );

    // the change is in the project, the session is in the (clean) linked tree
    git(&linked, &["reset", "--quiet", "--hard", "main"]);
    start(&root, "s3");
    silence_a_lint(&root);
    let decision = stop_from(&linked, "s3", &script);
    let reason = blocked(&decision);
    assert!(
        reason.contains(&tree(&top, "FAIL")) && reason.contains(&tree(&linked, "no change")),
        "{reason}"
    );
    // both changed: both are shown to the judge, each as its own patch
    let before = std::fs::read_to_string(&log)
        .unwrap()
        .matches("ARGS ")
        .count();
    silence_a_lint(&linked);
    let decision = stop_from(&linked, "s3", &script);
    let reason = blocked(&decision);
    assert!(
        reason.contains(&tree(&top, "FAIL")) && reason.contains(&tree(&linked, "FAIL")),
        "{reason}"
    );
    let seen = std::fs::read_to_string(&log).unwrap();
    assert_eq!(seen.matches("ARGS ").count(), before + 2, "{seen}");
    assert!(
        seen.contains("change-0.patch") && seen.contains("change-1.patch"),
        "{seen}"
    );

    // a session that began in the linked tree has that tree as its project, and one tree
    git(&root, &["checkout", "--quiet", "--", "a.rs"]);
    start(&linked, "s4");
    let decision = stop_from(&linked, "s4", &script);
    let reason = blocked(&decision);
    assert!(!reason.contains("Working tree "), "{reason}");
    // nothing was left behind, and the hooks' directory is the repository's, not the tree's
    assert!(root.join(".git/deep-diff-forge").is_dir());
    assert_eq!(
        std::fs::read_dir(root.join(".git/deep-diff-forge"))
            .unwrap()
            .count(),
        0
    );
    assert_eq!(git(&linked, &["status", "--porcelain=v1"]), " M a.rs\n");
}

/// Independent review of the hooks, HR-H-2. The patch was `HEAD` against the working tree, with
/// `HEAD` read at the stop, and the policy was read from that same commit. An agent that
/// committed its work stopped on a clean tree: nothing to judge, no output. A policy change it
/// committed was the policy for the rest of the session. The base is the commit the session
/// began from now, for the patch and for the policy.
#[test]
fn work_and_policy_the_session_committed_are_judged_from_where_it_began() {
    let scratch = Scratch::new("hook-committed");
    let root = repository(&scratch);
    let (script, log, _) = failing_judge(&scratch);
    let base = git(&root, &["rev-parse", "HEAD"]);
    let since = format!("changes since this session began (commit {})", &base[..7]);

    start(&root, "s");
    silence_a_lint(&root);
    git(&root, &["commit", "--quiet", "-am", "work"]);
    assert_eq!(git(&root, &["status", "--porcelain=v1"]), "");
    let decision = stop_from(&root, "s", &script);
    let reason = blocked(&decision);
    assert!(reason.contains(&since), "{reason}");
    assert!(reason.contains("check_suppressed in \"a.rs\""), "{reason}");
    assert!(
        std::fs::read_to_string(&log)
            .unwrap()
            .contains("PATCH-FILES 1\n"),
        "the judge was not shown the committed change"
    );
    // advice is about the same range: a finding in committed work is said
    let (_, stdout, _) = hook("post-tool", &[], &input("PostToolUse", &root, "s", false));
    assert!(
        stdout.contains("check_suppressed at \\\"a.rs\\\"") && stdout.contains(&since),
        "{stdout}"
    );

    // the session loosens the policy, commits that, then leaves another change uncommitted
    std::fs::write(root.join("warden.toml"), "# loosened by the session\n").unwrap();
    git(
        &root,
        &["commit", "--quiet", "-am", "policy: drop the rules"],
    );
    std::fs::write(root.join("new.rs"), "#[allow(unused)]\nfn n() {}\n").unwrap();
    let decision = stop_from(&root, "s", &script);
    blocked(&decision);
    let seen = std::fs::read_to_string(&log).unwrap();
    assert!(!seen.contains("loosened"), "{seen}");
    assert_eq!(
        seen.matches("POLICY # the policy at the base commit\n")
            .count(),
        2,
        "{seen}"
    );
    // the committed work, the committed policy and the new file
    assert!(seen.contains("PATCH-FILES 3\n"), "{seen}");

    // A later start of the same session (resume, compact, clear) does not move the base.
    let again = start(&root, "s");
    assert!(
        again.contains(&format!(
            "(commit {}, recorded when the session first started)",
            &base[..7]
        )),
        "{again}"
    );
    blocked(&stop_from(&root, "s", &script));

    // A stop that is its session's first hook can only measure from where HEAD is then, and
    // says exactly that instead of "since this session began".
    let decision = stop_from(&root, "late", &script);
    let reason = blocked(&decision);
    let head = git(&root, &["rev-parse", "HEAD"]);
    assert!(
        reason.contains(&format!(
            "changes since commit {}, which HEAD named when this session's hooks first ran",
            &head[..7]
        )),
        "{reason}"
    );
    assert!(!reason.contains("since this session began"), "{reason}");
}

/// HR-H-2, the base when history moves under the session. A reset or another branch leaves
/// `HEAD` on history that does not contain the base: its tree is still what the working tree is
/// compared with, and the message says the branch moved. A base whose commit is gone from the
/// repository cannot be compared with: the stop is UNJUDGED and says so.
#[test]
fn a_base_the_branch_left_is_still_judged_and_one_that_is_gone_is_unjudged() {
    let scratch = Scratch::new("hook-base-moved");
    let root = repository(&scratch);
    let (script, _, _) = failing_judge(&scratch);
    let first = git(&root, &["rev-parse", "HEAD"]);
    std::fs::write(root.join("b.rs"), "fn c() {}\n").unwrap();
    git(&root, &["add", "b.rs"]);
    git(&root, &["commit", "--quiet", "-m", "second"]);
    let second = git(&root, &["rev-parse", "HEAD"]);

    start(&root, "s");
    git(&root, &["reset", "--quiet", "--hard", first.trim()]);
    let decision = stop_from(&root, "s", &script);
    let reason = blocked(&decision);
    assert!(
        reason.contains(&format!(
            "changes since this session began (commit {}); the branch has moved and no longer contains that commit",
            &second[..7]
        )),
        "{reason}"
    );
    // the same when HEAD names no commit at all any more (an orphan branch)
    git(&root, &["checkout", "--quiet", "--orphan", "wip"]);
    let decision = stop_from(&root, "s", &script);
    assert!(
        blocked(&decision).contains("the branch has moved"),
        "{decision}"
    );

    // the commit itself is gone
    git(&root, &["checkout", "--quiet", "-f", "main"]);
    git(&root, &["reflog", "expire", "--expire=now", "--all"]);
    git(&root, &["gc", "--quiet", "--prune=now"]);
    let decision = stop_from(&root, "s", &script);
    let said = allowed(&decision);
    assert!(
        said.starts_with("deep-diff-forge: UNJUDGED. ")
            && said.contains(&format!(
                "the commit this session began from ({}) can no longer be read",
                &second[..7]
            )),
        "{said}"
    );
    // advice says it once, then stays silent
    let event = input("PostToolUse", &root, "s", false);
    let (_, stdout, _) = hook("post-tool", &[], &event);
    assert!(
        system_message(&stdout).contains("can no longer be read"),
        "{stdout}"
    );
    assert_eq!(hook("post-tool", &[], &event).1, "");
}

/// HR-H-2 and HR-M-1, a session with no commit to begin from. In a repository with no commit
/// at all there is nothing to judge, and that is recorded: a stop after the first commit is
/// UNJUDGED, not judged from that commit as if nothing had come before. On an orphan branch in
/// a repository that has commits the stop is UNJUDGED from the start; it used to be silent.
#[test]
fn a_session_that_began_with_no_commit_is_never_judged_as_if_it_had_one() {
    let scratch = Scratch::new("hook-no-commit");
    home_in(&scratch);
    let (script, log, _) = failing_judge(&scratch);
    let root = scratch.join("new");
    std::fs::create_dir(&root).unwrap();
    git(&root, &["init", "--quiet"]);
    std::fs::write(root.join("warden.toml"), "# a policy\n").unwrap();
    silence_a_lint(&root);

    let said = start(&root, "s");
    assert!(said.contains("This repository has no commit yet"), "{said}");
    assert!(!said.contains("stops will be judged"), "{said}");
    // nothing to judge, and nothing said: there is no commit
    assert_eq!(stop_from(&root, "s", &script), Value::Null);
    assert_eq!(
        hook("post-tool", &[], &input("PostToolUse", &root, "s", false)).1,
        ""
    );
    // the first commit is made during the session
    git(&root, &["add", "--all"]);
    git(&root, &["commit", "--quiet", "-m", "first"]);
    let decision = stop_from(&root, "s", &script);
    let unjudged = allowed(&decision);
    assert!(
        unjudged.starts_with("deep-diff-forge: UNJUDGED. ")
            && unjudged.contains("the session began before the first commit of this repository"),
        "{unjudged}"
    );
    // also when the tree changes again afterwards: still no base
    std::fs::write(root.join("new.rs"), "#[allow(unused)]\nfn n() {}\n").unwrap();
    assert!(allowed(&stop_from(&root, "s", &script)).contains("UNJUDGED"));
    assert!(!log.exists(), "the judge was run with no base");

    // an orphan branch in a repository that has commits (the reviewer's case 8d)
    git(&root, &["checkout", "--quiet", "--orphan", "wip"]);
    let decision = stop_from(&root, "orphan", &script);
    let unjudged = allowed(&decision);
    assert!(
        unjudged.contains("UNJUDGED") && unjudged.contains("an orphan branch"),
        "{unjudged}"
    );
    let said = start(&root, "orphan-start");
    assert!(
        said.contains("an orphan branch") && said.contains("will be UNJUDGED"),
        "{said}"
    );
    // and it stays so for that session once the branch has a commit
    git(&root, &["commit", "--quiet", "-m", "on the orphan branch"]);
    assert!(allowed(&stop_from(&root, "orphan", &script)).contains("an orphan branch"));
}

/// Independent review of the hooks, HR-M-1. SECURITY.md said "A failure is never nothing to
/// judge", and every failure to find the repository was: Git not on `PATH`, Git refusing the
/// repository for its owner, a directory inside `.git`, a directory that is gone. Each is said
/// now: a stop is UNJUDGED with the reason, a start says that stops will not be judged, and
/// advice says it once in a session (HR-L-2), not after every tool call.
#[test]
fn a_failure_to_find_the_repository_is_said_and_never_silent() {
    let scratch = Scratch::new("hook-no-git");
    let root = repository(&scratch);
    let (script, log, _) = failing_judge(&scratch);
    silence_a_lint(&root);
    let empty = scratch.join("no-git-here");
    std::fs::create_dir(&empty).unwrap();
    let judge_arg = ["--judge", script.to_str().unwrap()];
    let no_git = |event: &str, name: &str, session: &str| {
        hook_in(
            event,
            &judge_arg[..if event == "post-tool" { 0 } else { 2 }],
            &input(name, &root, session, false),
            |command| {
                command.env("PATH", &empty);
            },
        )
    };

    // Git is not on the hook's PATH
    let (code, stdout, _) = no_git("stop", "Stop", "s");
    assert_eq!(code, 0);
    let said = system_message(&stdout);
    assert!(
        said.starts_with("deep-diff-forge: UNJUDGED. This stop was not judged: Git could not say which repository")
            && said.ends_with("That is not a pass."),
        "{said}"
    );
    let (code, stdout, _) = no_git("session-start", "SessionStart", "s");
    assert_eq!(code, 0);
    assert!(
        stdout.starts_with("deep-diff-forge: the stops of this session will not be judged and no advice will be given: Git could not say which repository"),
        "{stdout}"
    );
    let (_, stdout, _) = no_git("post-tool", "PostToolUse", "s");
    let said = system_message(&stdout);
    assert!(
        said.contains("the hook did not run: Git could not say which repository")
            && said.contains("Said once in this session"),
        "{said}"
    );
    for _ in 0..3 {
        assert_eq!(no_git("post-tool", "PostToolUse", "s").1, "");
    }
    // a stop says it every time; another session is told once of its own
    assert!(system_message(&no_git("stop", "Stop", "s").1).contains("UNJUDGED"));
    assert_ne!(no_git("post-tool", "PostToolUse", "s2").1, "");
    // the same for a session that has a project recorded
    start(&root, "known");
    assert!(system_message(&no_git("stop", "Stop", "known").1).contains("UNJUDGED"));

    // Git refuses the repository for its owner, where this Git can be made to (its own test
    // switch; the reviewer's case 8b, the one that happens by accident on a mounted volume)
    let refused = Command::new("git")
        .current_dir(&root)
        .envs(ISOLATE)
        .env("GIT_TEST_ASSUME_DIFFERENT_OWNER", "1")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .unwrap();
    if !refused.status.success() {
        let (_, stdout, _) = hook_in(
            "stop",
            &judge_arg,
            &input("Stop", &root, "owner", false),
            |command| {
                command.env("GIT_TEST_ASSUME_DIFFERENT_OWNER", "1");
            },
        );
        let said = system_message(&stdout);
        assert!(
            said.contains("UNJUDGED") && said.contains("dubious ownership"),
            "{said}"
        );
    }

    // the session's directory is inside the Git directory, or gone, and nothing is recorded
    for (session, cwd, why) in [
        (
            "in-git",
            root.join(".git"),
            "Git could not say which repository",
        ),
        (
            "gone",
            root.join("gone"),
            "does not exist, and no project is recorded",
        ),
    ] {
        let decision = stop_from(&cwd, session, &script);
        let said = allowed(&decision);
        assert!(
            said.contains("UNJUDGED") && said.contains(why),
            "{session}: {said}"
        );
    }
    assert!(
        !log.exists(),
        "the judge was run although nothing was found"
    );
}

/// Independent review of the hooks, "weak tests": the rule that a patch which cannot be made
/// is UNJUDGED lived in the hook and no test of the hook made one fail. A path marked
/// skip-worktree and an untracked file that cannot be read are both: a stop says UNJUDGED with
/// the reason, and advice says it once (HR-L-2: it used to say it after every tool call, or,
/// for a sparse checkout, never).
#[test]
fn a_patch_that_cannot_be_made_is_unjudged_at_a_stop_and_told_once_as_advice() {
    use std::os::unix::fs::PermissionsExt as _;
    let scratch = Scratch::new("hook-no-patch");
    let root = repository(&scratch);
    let (script, log, _) = failing_judge(&scratch);
    silence_a_lint(&root);
    let advice = |session: &str| {
        hook(
            "post-tool",
            &[],
            &input("PostToolUse", &root, session, false),
        )
        .1
    };

    git(&root, &["update-index", "--skip-worktree", "a.rs"]);
    let decision = stop_from(&root, "skip", &script);
    let said = allowed(&decision);
    assert!(
        said.starts_with("deep-diff-forge: UNJUDGED. ") && said.contains("skip-worktree"),
        "{said}"
    );
    let first = advice("skip");
    assert!(system_message(&first).contains("skip-worktree"), "{first}");
    assert_eq!(advice("skip"), "");
    assert_eq!(advice("skip"), "");
    git(&root, &["update-index", "--no-skip-worktree", "a.rs"]);

    // an untracked file this account cannot read: `git add` fails (not as root, who reads it)
    let secret = root.join("unreadable.txt");
    std::fs::write(&secret, "x\n").unwrap();
    std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::read(&secret).is_err() {
        let decision = stop_from(&root, "unreadable", &script);
        let said = allowed(&decision);
        assert!(
            said.starts_with("deep-diff-forge: UNJUDGED. ")
                && said.contains("the change could not be read from Git"),
            "{said}"
        );
        let first = advice("unreadable");
        assert!(
            system_message(&first).contains("the change could not be read from Git"),
            "{first}"
        );
        assert_eq!(advice("unreadable"), "");
        // the condition ends: advice is given again
        std::fs::remove_file(&secret).unwrap();
        assert!(advice("unreadable").contains("check_suppressed"));
    }
    assert!(!log.exists(), "the judge was run without a patch");
}

/// Independent review of the hooks, HR-M-2, at the hook. A file under an ignore rule that was
/// added to the index with `git add -f` is in the next commit; the patch left it out, so a
/// stop passed in silence. Both of the reviewer's cases: a rule in `.gitignore`, and one
/// written to `.git/info/exclude` during the session.
#[test]
fn a_file_added_to_the_index_is_judged_even_when_an_ignore_rule_names_it() {
    let scratch = Scratch::new("hook-forced");
    let root = repository(&scratch);
    let (script, log, _) = failing_judge(&scratch);
    std::fs::write(root.join(".gitignore"), "build/\n").unwrap();
    git(&root, &["add", ".gitignore"]);
    git(&root, &["commit", "--quiet", "-m", "ignore build"]);
    start(&root, "s");

    std::fs::create_dir(root.join("build")).unwrap();
    std::fs::write(
        root.join("build/gen.rs"),
        "#[allow(dead_code)]\nfn g() {}\n",
    )
    .unwrap();
    // ignored and not in the index: not part of any commit, and not judged
    assert_eq!(stop_from(&root, "s", &script), Value::Null);
    git(&root, &["add", "-f", "build/gen.rs"]);
    blocked(&stop_from(&root, "s", &script));
    let (_, stdout, _) = hook("post-tool", &[], &input("PostToolUse", &root, "s", false));
    assert!(
        stdout.contains("check_suppressed at \\\"build/gen.rs\\\""),
        "{stdout}"
    );

    std::fs::write(root.join(".git/info/exclude"), "hidden.rs\n").unwrap();
    std::fs::write(root.join("hidden.rs"), "#[allow(unused)]\nfn h() {}\n").unwrap();
    git(&root, &["add", "-f", "hidden.rs"]);
    let (_, stdout, _) = hook("post-tool", &[], &input("PostToolUse", &root, "s", false));
    assert!(
        stdout.contains("check_suppressed at \\\"hidden.rs\\\""),
        "{stdout}"
    );
    let seen = std::fs::read_to_string(&log).unwrap();
    assert_eq!(seen.matches("PATCH-FILES 1\n").count(), 1, "{seen}");
    // the hook staged nothing in the repository's own index
    assert_eq!(
        git(&root, &["status", "--porcelain=v1"]),
        "A  build/gen.rs\nA  hidden.rs\n"
    );
}

/// Independent review of the hooks, HR-L-1: a hook that its client ended left its scratch
/// directory, with a copy of the patch and of every new object, until a process with the same
/// id came by. The next hook of the repository removes the directories of processes that are
/// gone. (Where there is a `/proc` to ask.)
#[cfg(target_os = "linux")]
#[test]
fn a_scratch_directory_a_killed_hook_left_is_removed_by_the_next_hook() {
    let scratch = Scratch::new("hook-stale-tmp");
    let root = repository(&scratch);
    silence_a_lint(&root);
    let own = root.join(".git/deep-diff-forge");
    // what a stop that was killed leaves; no process has an id above 2^22
    let left = own.join("tmp-4999999");
    std::fs::create_dir_all(left.join("work/judge.git/objects")).unwrap();
    std::fs::write(left.join("change-0.patch"), "a copy of the patch").unwrap();
    std::fs::write(left.join("policy.toml"), "a copy of the policy").unwrap();
    let alive = own.join("tmp-1");
    std::fs::create_dir(&alive).unwrap();
    let (_, stdout, _) = hook("post-tool", &[], &input("PostToolUse", &root, "s", false));
    assert!(stdout.contains("check_suppressed"), "{stdout}");
    assert!(
        !left.exists(),
        "the directory of a process that is gone was left"
    );
    assert!(
        alive.is_dir(),
        "the directory of a process that lives was removed"
    );
    let names: Vec<String> = std::fs::read_dir(&own)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["tmp-1"]);
}

/// Independent review of the hooks, HR-L-4. "FAIL STANDS" was decided by rule, signal and path:
/// blocked for one credential in a file, the agent could add two more and the next stop was
/// allowed. The number of items the judge cites is part of what "the same failure" is. And a
/// state that cannot be kept is said with the decision, where the stop used to block again and
/// again with no word of why.
#[test]
fn a_new_item_blocks_again_and_a_state_that_cannot_be_kept_is_said() {
    let scratch = Scratch::new("hook-fail-stands");
    let root = repository(&scratch);
    let (script, log, exit_file) = failing_judge(&scratch);
    // the judge's document as forge-warden writes one: each violation cites its items
    let citing = |items: usize| {
        let mut document = verdict("fail", "no-silent-suppression", "check_suppressed", "a.rs");
        let evidence: Vec<Value> = (1..=items)
            .map(|line| json!({"address": "a".repeat(64), "side": "new", "line": line}))
            .collect();
        document["rules"][0]["violations"][0]["evidence"] = json!(evidence);
        document
    };
    let stop = |active: bool| -> Value {
        let (code, stdout, _) = hook(
            "stop",
            &["--judge", script.to_str().unwrap()],
            &input("Stop", &root, "s", active),
        );
        assert_eq!(code, 0);
        serde_json::from_str(&stdout).unwrap()
    };
    silence_a_lint(&root);
    write_program(&script, &judge_script(&log, &exit_file, &citing(1)));
    blocked(&stop(false));
    // nothing repaired: the same failure does not block twice
    assert!(allowed(&stop(true)).starts_with("deep-diff-forge: FAIL STANDS. "));
    blocked(&stop(false));
    // two more items under the same rule in the same file: a new failure
    write_program(&script, &judge_script(&log, &exit_file, &citing(3)));
    blocked(&stop(true));
    assert!(allowed(&stop(true)).starts_with("deep-diff-forge: FAIL STANDS. "));

    // the state file cannot be replaced: a directory stands at its name
    let state: Vec<PathBuf> = std::fs::read_dir(sessions())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(state.len(), 1, "{state:?}");
    std::fs::remove_file(&state[0]).unwrap();
    std::fs::create_dir(&state[0]).unwrap();
    std::fs::write(state[0].join("in-the-way"), "x").unwrap();
    for active in [false, true] {
        let decision = stop(active);
        let reason = blocked(&decision);
        assert!(
            reason.contains("The hook could not keep its state (its state could not be saved: ")
                && reason.contains("the same failure can block again"),
            "{reason}"
        );
    }
}

/// Independent review of the hooks, HR-M-4. The gate said every link was ready where a stop in
/// the same tree was UNJUDGED (a path marked skip-worktree, an untracked file that cannot be
/// read: it never made the patch) and where no hook ran at all (`disableAllHooks`, a timeout
/// of one second: it read only the command of a registered hook). It makes the patch a stop
/// would make now, and reads the settings whole.
#[test]
fn doctor_gate_checks_the_patch_and_the_settings_a_stop_depends_on() {
    use std::os::unix::fs::PermissionsExt as _;
    let (_scratch, root, home, script, _log, _exit_file) = gate_fixture("doctor-gate-stop");
    let judge_arg = ["--judge", script.to_str().unwrap()];
    std::fs::create_dir(home.join(".claude")).unwrap();
    let user = home.join(".claude/settings.json");
    let registered: Value = serde_json::from_str(&stop_hook_settings(&script)).unwrap();
    let write = |file: &Path, settings: &Value| std::fs::write(file, settings.to_string()).unwrap();
    write(&user, &registered);
    let all = |ok: [bool; 10]| (if ok == [true; 10] { 0 } else { 7 }, gate_names(ok));
    let gate = || {
        let (code, doc, _) = doctor_gate(&root, &home, &judge_arg);
        ((code, gate_checks(&doc)), doc)
    };
    let patch_fails = [true, true, true, true, true, true, false, true, true, true];
    assert_eq!(gate().0, all([true; 10]));

    // a path marked skip-worktree: a stop here is UNJUDGED
    git(&root, &["update-index", "--skip-worktree", "a.rs"]);
    let (found, doc) = gate();
    assert_eq!(found, all(patch_fails));
    let why = detail(&doc, "patch");
    assert!(
        why.contains("a stop here would be UNJUDGED") && why.contains("skip-worktree"),
        "{why}"
    );
    git(&root, &["update-index", "--no-skip-worktree", "a.rs"]);
    // an untracked file this account cannot read (not as root, who reads it)
    let unreadable = root.join("unreadable.txt");
    std::fs::write(&unreadable, "x\n").unwrap();
    std::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::read(&unreadable).is_err() {
        let (found, doc) = gate();
        assert_eq!(found, all(patch_fails));
        assert!(
            detail(&doc, "patch").contains("a stop here would be UNJUDGED"),
            "{doc:#}"
        );
    }
    std::fs::remove_file(&unreadable).unwrap();
    // a tree with a change: the patch can be made, and the gate says how large it is
    silence_a_lint(&root);
    let (found, doc) = gate();
    assert_eq!(found, all([true; 10]));
    assert!(
        detail(&doc, "patch").starts_with("the patch a stop would judge can be made ("),
        "{doc:#}"
    );
    assert_eq!(git(&root, &["status", "--porcelain=v1"]), " M a.rs\n");

    // `disableAllHooks`: no hook runs, however well it is registered
    let mut disabled = registered.clone();
    disabled["disableAllHooks"] = json!(true);
    write(&user, &disabled);
    let hooks_off = [true, true, true, true, true, true, true, false, true, true];
    let (found, doc) = gate();
    assert_eq!(found, all(hooks_off));
    assert!(
        detail(&doc, "hooks_enabled").contains("sets disableAllHooks"),
        "{doc:#}"
    );
    // a file that wins over it switches them on again, and one that wins switches them off
    std::fs::create_dir(root.join(".claude")).unwrap();
    let local = root.join(".claude/settings.local.json");
    write(&local, &json!({"disableAllHooks": false}));
    assert_eq!(gate().0, all([true; 10]));
    write(&user, &registered);
    write(&local, &json!({"disableAllHooks": true}));
    assert_eq!(gate().0, all(hooks_off));
    std::fs::remove_file(&local).unwrap();

    // the Stop hook's timeout: under what a stop can take, the client ends the hook
    let timed = |timeout: Value| {
        let mut settings = registered.clone();
        settings["hooks"]["Stop"][0]["hooks"][0]["timeout"] = timeout;
        settings
    };
    let too_short = [true, true, true, true, true, true, true, true, true, false];
    for timeout in [json!(1), json!(49), json!("60")] {
        write(&user, &timed(timeout.clone()));
        let (found, doc) = gate();
        assert_eq!(found, all(too_short), "{timeout}");
        assert!(
            detail(&doc, "hook_timeout").contains("under the 50 seconds a stop can take"),
            "{doc:#}"
        );
    }
    for timeout in [json!(50), json!(60), json!(600), json!(null)] {
        write(&user, &timed(timeout.clone()));
        assert_eq!(gate().0, all([true; 10]), "{timeout}");
    }
    // the settings `agent setup --hooks` prints give the stop hook time enough
    let out = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .args(["agent", "setup", "--client", "claude", "--hooks", "--json"])
        .output()
        .unwrap();
    let printed: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(printed["hooks"]["Stop"][0]["hooks"][0]["timeout"], 60);
    // a hook of another type holding the same words is no stop hook
    let mut prompt = registered.clone();
    prompt["hooks"]["Stop"][0]["hooks"][0]["type"] = json!("prompt");
    write(&user, &prompt);
    let (found, _) = gate();
    assert_eq!(
        found,
        all([true, true, true, true, true, true, true, true, false, false])
    );
}

/// HR-H-2, a tree that is not clean when the session begins: what was uncommitted then is
/// inside what is judged, and the start says so (a known limit; start sessions on a clean
/// tree).
#[test]
fn a_session_that_starts_on_a_changed_tree_is_told_that_it_is_judged_with_it() {
    let scratch = Scratch::new("hook-start-dirty");
    let root = repository(&scratch);
    let clean = start(&root, "clean");
    assert!(!clean.contains("already differs"), "{clean}");
    silence_a_lint(&root);
    let dirty = start(&root, "dirty");
    assert!(
        dirty.contains("- The working tree already differs from that commit: what was uncommitted when this session began is judged together with your changes.\n"),
        "{dirty}"
    );
    // the start changed nothing and left nothing behind
    assert_eq!(git(&root, &["status", "--porcelain=v1"]), " M a.rs\n");
    assert_eq!(
        std::fs::read_dir(root.join(".git/deep-diff-forge"))
            .unwrap()
            .count(),
        0
    );
}

/// Independent review of the hooks, HR-L-2, the input cap. A `PostToolUse` input carries the
/// tool's whole answer; at 1 MiB one large answer made the hook say "the hook input is not
/// JSON" after that tool call, and the same cap made a stop "not run". An input of 1.5 MB is
/// read now; one over the cap of 32 MiB is refused by its size, in words that say so.
#[test]
fn a_large_tool_answer_is_read_and_one_over_the_cap_is_refused_by_name() {
    let scratch = Scratch::new("hook-large-input");
    let root = repository(&scratch);
    let (script, _, _) = failing_judge(&scratch);
    silence_a_lint(&root);
    let with = |event: &str, member: &str, bytes: usize| {
        let mut large = input(event, &root, "s", false);
        large[member] = json!("y".repeat(bytes));
        large
    };
    let (code, stdout, _) = hook(
        "post-tool",
        &[],
        &with("PostToolUse", "tool_response", 1_500_000),
    );
    assert_eq!(code, 0);
    assert!(stdout.contains("check_suppressed"), "{stdout}");
    let (_, stdout, _) = hook(
        "stop",
        &["--judge", script.to_str().unwrap()],
        &with("Stop", "last_assistant_message", 1_500_000),
    );
    let decision: Value = serde_json::from_str(&stdout).unwrap();
    blocked(&decision);

    let over = with("Stop", "last_assistant_message", 33 * 1024 * 1024);
    let (code, stdout, _) = hook("stop", &["--judge", script.to_str().unwrap()], &over);
    assert_eq!(code, 0);
    let said = system_message(&stdout);
    assert!(
        said.starts_with(
            "deep-diff-forge: UNJUDGED. The stop hook did not run: its input is over 32 MiB"
        ),
        "{said}"
    );
}

/// The state of a session is found by the session's id alone, in the user's state directory:
/// `$XDG_STATE_HOME` when it is absolute, else `.local/state` of the home directory. Without
/// either there is nowhere to remember the project and the base, and the hook says that it did
/// not run; a stop then says UNJUDGED.
#[test]
fn the_state_of_a_session_is_in_the_users_state_directory_or_the_hook_says_it_has_none() {
    let scratch = Scratch::new("hook-state-home");
    let root = repository(&scratch);
    silence_a_lint(&root);
    let event = input("PostToolUse", &root, "s", false);
    let files = |dir: &Path| std::fs::read_dir(dir).map_or(0, Iterator::count);

    // an absolute XDG_STATE_HOME wins over the home directory; a relative one is not used
    let xdg = scratch.join("xdg-state");
    let (_, stdout, _) = hook_in("post-tool", &[], &event, |command| {
        command.env("XDG_STATE_HOME", &xdg);
    });
    assert!(stdout.contains("check_suppressed"), "{stdout}");
    assert_eq!(files(&xdg.join("deep-diff-forge/hooks")), 1);
    assert_eq!(files(&sessions()), 0);
    let (_, stdout, _) = hook_in("post-tool", &[], &event, |command| {
        command
            .env("XDG_STATE_HOME", "relative/state")
            .current_dir(&root);
    });
    assert!(stdout.contains("check_suppressed"), "{stdout}");
    assert_eq!(files(&sessions()), 1);
    assert!(!root.join("relative").exists());
    // nothing of a session is kept in the repository
    assert_eq!(files(&root.join(".git/deep-diff-forge")), 0);

    // neither: said, for a stop as UNJUDGED, and never a block
    for (word, name, starts) in [
        (
            "post-tool",
            "PostToolUse",
            "deep-diff-forge: the hook did not run: ",
        ),
        (
            "stop",
            "Stop",
            "deep-diff-forge: UNJUDGED. The stop hook did not run: ",
        ),
    ] {
        let (code, stdout, _) = hook_in(word, &[], &input(name, &root, "s", false), |command| {
            command.env_remove("HOME");
        });
        assert_eq!(code, 0);
        let said = system_message(&stdout);
        assert!(
            said.starts_with(starts) && said.contains("there is no state directory"),
            "{said}"
        );
    }
}

/// Round 10 of the review of the hooks, after HR-H-2. A stop that is the first hook of its
/// session records the project and the commit `HEAD` names at that very stop: it knows nothing
/// of what the session did before. On a clean tree it said nothing, so a session that
/// committed its work with no session-start hook installed still passed in silence. Such a
/// stop is never silent now: it is allowed (or blocked, held, unjudged, as the judge says of
/// what is left in the tree) and says that the session's start was not recorded. It says so
/// once: the next stop finds the state this one wrote. Only where no project can be found at
/// all does it stay silent.
#[test]
fn a_stop_that_is_its_sessions_first_hook_is_never_silent() {
    const NOT_RECORDED: &str = "This session's start was not recorded: this stop is the first hook of the session here, so work committed before it was not judged. For a session to be judged from its start, the session-start hook must be installed before the session begins";
    let scratch = Scratch::new("hook-first-stop");
    let root = repository(&scratch);
    let (script, log, exit_file) = judge(
        &scratch,
        &verdict("fail", "no-silent-suppression", "check_suppressed", "a.rs"),
    );
    let outside = scratch.join("outside");
    std::fs::create_dir(&outside).unwrap();
    let stop = |session: &str, cwd: &Path, project: Option<&Path>| {
        let (code, stdout, stderr) = hook_in(
            "stop",
            &["--judge", script.to_str().unwrap()],
            &input("Stop", cwd, session, false),
            |command| {
                if let Some(project) = project {
                    command.env("CLAUDE_PROJECT_DIR", project);
                }
            },
        );
        assert_eq!((code, stderr.as_str()), (0, ""), "{stdout}");
        stdout
    };
    // The whole message of a stop that has nothing else to say: for the user, not a block.
    let only_the_note = |stdout: &str| {
        let said = system_message(stdout);
        assert!(
            said.starts_with(&format!("deep-diff-forge: {NOT_RECORDED}")),
            "{said}"
        );
    };

    // A clean repository and a first stop: whether the session committed anything before
    // cannot be known, so it is said. Once: the second stop of the session is ordinary.
    only_the_note(&stop("clean", &root, None));
    assert_eq!(stop("clean", &root, None), "");
    // The reviewer's T2a: the work is committed just before the stop.
    silence_a_lint(&root);
    git(&root, &["commit", "--quiet", "-am", "work"]);
    only_the_note(&stop("committed", &root, None));
    assert_eq!(stop("committed", &root, None), "");
    // The same when the client names the project and the session is somewhere else.
    only_the_note(&stop("named", &outside, Some(&root)));
    assert_eq!(stop("named", &outside, Some(&root)), "");
    assert!(!log.exists(), "the judge was run with nothing to judge");
    // A session that started is not told this: its clean stop says nothing.
    start(&root, "started");
    assert_eq!(stop("started", &root, None), "");

    // With something left in the tree the judge decides, and the same sentence is added.
    std::fs::write(root.join("new.rs"), "#[allow(unused)]\nfn n() {}\n").unwrap();
    std::fs::write(&exit_file, "0").unwrap();
    only_the_note(&stop("passes", &root, None));
    assert_eq!(stop("passes", &root, None), "");
    std::fs::write(&exit_file, "8").unwrap();
    let held = system_message(&stop("holds", &root, None));
    assert!(
        held.starts_with("deep-diff-forge: HOLD. ") && held.contains(NOT_RECORDED),
        "{held}"
    );
    assert!(!system_message(&stop("holds", &root, None)).contains(NOT_RECORDED));
    std::fs::write(&exit_file, "3").unwrap();
    let unjudged = system_message(&stop("unjudged", &root, None));
    assert!(
        unjudged.starts_with("deep-diff-forge: UNJUDGED. ") && unjudged.contains(NOT_RECORDED),
        "{unjudged}"
    );
    std::fs::write(&exit_file, "7").unwrap();
    let decision: Value = serde_json::from_str(&stop("fails", &root, None)).unwrap();
    let reason = blocked(&decision);
    assert!(
        reason.contains("check_suppressed in \"a.rs\"") && reason.contains(NOT_RECORDED),
        "{reason}"
    );
    let again: Value = serde_json::from_str(&stop("fails", &root, None)).unwrap();
    assert!(!blocked(&again).contains(NOT_RECORDED), "{again}");

    // The session's directory is a repository of its own (the reviewer's T1a and T1d, with
    // nothing recorded and no project named): that repository is all the hook can find, and
    // it says that it knows no more. Without a commit there, and with one.
    let unborn = scratch.join("unborn");
    std::fs::create_dir(&unborn).unwrap();
    git(&unborn, &["init", "--quiet"]);
    only_the_note(&stop("unborn", &unborn, None));
    assert_eq!(stop("unborn", &unborn, None), "");
    let other = scratch.join("other");
    std::fs::create_dir(&other).unwrap();
    git(&other, &["init", "--quiet"]);
    std::fs::write(other.join("x"), "x\n").unwrap();
    git(&other, &["add", "x"]);
    git(&other, &["commit", "--quiet", "-m", "x"]);
    only_the_note(&stop("other", &other, None));
    // an orphan branch: UNJUDGED, with the sentence
    git(&other, &["checkout", "--quiet", "--orphan", "wip"]);
    let orphan = system_message(&stop("orphan", &other, None));
    assert!(
        orphan.contains("an orphan branch") && orphan.contains(NOT_RECORDED),
        "{orphan}"
    );

    // No project at all: no repository around the session's directory, none named, none
    // recorded. There is no reason to think the session has one, and nothing is said.
    assert_eq!(stop("nowhere", &outside, None), "");
    assert_eq!(stop("nowhere-named", &outside, Some(&outside)), "");
}
