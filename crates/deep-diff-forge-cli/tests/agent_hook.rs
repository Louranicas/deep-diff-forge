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
/// It appends its arguments, the policy it was given, the number of files in the patch and the
/// number of lines the patch adds that silence a lint to `log`, prints `verdict` and exits
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
    --patch) patch="$2"; printf 'PATCH-FILES %s\n' "$(grep -c '^diff --git' "$2")" >> '@LOG@'; printf 'PATCH-ALLOWS %s\n' "$(grep -c '^+#\[allow' "$2")" >> '@LOG@';;
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
    // HR2-M-4: nor at a later turn of the same session (a failure blocks once in a session);
    // another session's first stop is blocked by it
    let out: Value = serde_json::from_str(&stop("s1", false).1).unwrap();
    assert!(out.get("decision").is_none(), "{out}");
    assert!(out["systemMessage"].as_str().unwrap().starts_with(
        "deep-diff-forge: FAIL STANDS. This failure has already blocked a stop of this session"
    ));
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
    // (before its last line the judge writes more than a kibibyte of other lines: a mutation
    // run left the cap of what is read of them, 64 KiB, alive as 64 + 1024 bytes)
    let body = "#!/bin/sh\ni=0; while [ $i -lt 60 ]; do echo 'warning: a line of sixty characters that says nothing at all' >&2; i=$((i+1)); done\necho '{\"exit\":3,\"name\":\"io\",\"message\":\"Git admission: no Git allowlist enrolled\",\"next\":\"check_environment\"}' >&2\nexit 3\n";
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
    // Six more that come before it by name and are not admitted: with this binary, the eight
    // candidates a stop tries. Their listing is past a kibibyte (a mutation run left the cap
    // of what is read of it, 256 KiB, alive as 256 + 1024 bytes).
    for n in 0..6 {
        let name = format!("a-stand-in-ranker-that-the-policy-does-not-admit-{n}");
        std::fs::write(rankers.join(name), format!("not admitted {n}\n")).unwrap();
    }
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
    assert_eq!(std::fs::read_to_string(&log).unwrap().lines().count(), 8);

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
        stdout.starts_with("deep-diff-forge is watching the changes of this session in \""),
        "{stdout}"
    );
    // HR2-L-3: the project is named, so a person can see which repository is watched
    assert!(
        stdout.starts_with(&format!(
            "deep-diff-forge is watching the changes of this session in \"{}\".\n",
            std::fs::canonicalize(&root).unwrap().display()
        )),
        "{stdout}"
    );
    // HR2-H-1: what is judged is said exactly, not as "everything, committed or not"
    assert!(
        stdout.contains(&format!(
            "your uncommitted changes, and each commit made since this session began (commit {}) that your branches, tags, stash, HEAD or their history name, are judged",
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

const GATE: [&str; 13] = [
    "git",
    "repository",
    "policy",
    "judge",
    "chain",
    "policy_protects_itself",
    "patch",
    "state_directory",
    "hooks_enabled",
    "hooks_registered",
    "hook_timeout",
    "start_hook_registered",
    "advice_hook_registered",
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

/// The stop hook is not registered as the gate wants it, so it has no timeout either; the
/// hooks of the other two events are.
const NOT_REGISTERED: [&str; 2] = ["hooks_registered", "hook_timeout"];

/// No settings file registers any hook of this program.
const NOT_INSTALLED: [&str; 4] = [
    "hooks_registered",
    "hook_timeout",
    "start_hook_registered",
    "advice_hook_registered",
];

/// The thirteen gate checks, in order, every one ready but those named in `not`.
fn gate_but(not: &[&str]) -> Vec<(String, bool)> {
    assert!(not.iter().all(|name| GATE.contains(name)), "{not:?}");
    GATE.iter()
        .map(|name| ((*name).to_owned(), !not.contains(name)))
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

/// A settings document whose stop hook is `command`, with the hooks of the other two events
/// as `agent setup --hooks` prints them for this binary.
fn settings_with(command: &str) -> String {
    let other = |word: &str| {
        json!([{"hooks": [{"type": "command", "command": format!(
            "'{}' agent hook {word} --client claude",
            this_binary().display()
        )}]}])
    };
    json!({"hooks": {
        "SessionStart": other("session-start"),
        "PostToolUse": other("post-tool"),
        "Stop": [{"hooks": [{"type": "command", "command": command}]}],
    }})
    .to_string()
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
    assert_eq!(gate_checks(&doc), gate_but(&NOT_INSTALLED));
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
            (7, gate_but(&NOT_REGISTERED)),
            "{command}"
        );
        assert!(
            detail(&doc, "hooks_registered").contains(why),
            "{command}: {doc:#}"
        );
    }
    std::fs::write(&local, stop_hook_settings(&script)).unwrap();
    let (code, doc, stderr) = doctor_gate(&root, &home, &judge_arg);
    assert_eq!(gate_checks(&doc), gate_but(&[]));
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
    assert_eq!((code, gate_checks(&doc)), (0, gate_but(&[])));

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
    assert_eq!((code, gate_checks(&doc)), (0, gate_but(&[])));

    // a policy that lets a change to itself pass is not ready
    std::fs::write(&exit_file, "0").unwrap();
    let (code, doc, _) = doctor_gate(&root, &home, &judge_arg);
    assert_eq!(
        (code, gate_checks(&doc)),
        (7, gate_but(&["policy_protects_itself"]))
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
        (7, gate_but(&["chain", "policy_protects_itself"]))
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
            gate_but(&[
                "judge",
                "chain",
                "policy_protects_itself",
                "hooks_registered",
                "hook_timeout"
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
            gate_but(&[
                "policy",
                "chain",
                "policy_protects_itself",
                "hooks_registered",
                "hook_timeout"
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
            gate_but(&[
                "repository",
                "policy",
                "chain",
                "policy_protects_itself",
                "patch",
                "hooks_enabled",
                "hooks_registered",
                "hook_timeout",
                "start_hook_registered",
                "advice_hook_registered"
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
    // What stays in the repository's directory once the hook is done is one empty mark: that
    // the trouble with the sessions' directory was said, which no state could remember.
    let left: Vec<PathBuf> = std::fs::read_dir(&own)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(left.len(), 1, "{left:?}");
    assert!(
        left[0]
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("said-")
            && std::fs::metadata(&left[0]).unwrap().len() == 0,
        "{left:?}"
    );
    let state = states_in(&sessions);
    assert_eq!(state.len(), 1, "{state:?}");
    assert_eq!(mode(&state[0]), 0o600);
    // said once: the state was read back
    assert_eq!(hook("post-tool", &[], &event).1, "");
}

/// Second independent review of the hooks, HR2-M-1. A state file that is there and cannot be
/// read as a state is never "no state". Read as that, it made the next hook the session's
/// first, and what had been committed until then passed in silence. A link is not followed,
/// and nothing is recorded over what stands there: a stop is UNJUDGED and says so every time,
/// the start says that stops will not be judged, and advice says it once.
#[test]
fn a_state_file_that_cannot_be_read_is_never_no_state() {
    let scratch = Scratch::new("hook-state-unreadable");
    let root = repository(&scratch);
    silence_a_lint(&root);
    let event = input("PostToolUse", &root, "s", false);
    assert!(
        hook("post-tool", &[], &event)
            .1
            .contains("check_suppressed")
    );
    let state = states_in(&sessions());
    assert_eq!(state.len(), 1, "{state:?}");
    let stop = || {
        let (code, stdout, _) = hook("stop", &[], &input("Stop", &root, "s", false));
        assert_eq!(code, 0);
        system_message(&stdout)
    };
    let target = scratch.join("target.json");
    std::fs::copy(&state[0], &target).unwrap();
    let before = std::fs::read(&target).unwrap();
    std::fs::remove_file(&state[0]).unwrap();
    std::os::unix::fs::symlink(&target, &state[0]).unwrap();
    for _ in 0..2 {
        let said = stop();
        assert!(
            said.starts_with(
                "deep-diff-forge: UNJUDGED. The stop hook did not run: this session's state file "
            ) && said.contains("cannot be read as a state (it cannot be opened")
                && said.contains("nothing is recorded over it"),
            "{said}"
        );
    }
    let said = system_message(&hook("post-tool", &[], &event).1);
    assert!(said.contains("cannot be read as a state"), "{said}");
    assert_eq!(hook("post-tool", &[], &event).1, "");
    assert!(
        std::fs::symlink_metadata(&state[0])
            .unwrap()
            .file_type()
            .is_symlink(),
        "the link was replaced"
    );
    assert_eq!(std::fs::read(&target).unwrap(), before);

    // A state file over the bound, and one that is not a state: the same, and neither is
    // replaced. (This test used to assert "over the bound is the same as no state".)
    std::fs::remove_file(&state[0]).unwrap();
    let huge = std::fs::File::create(&state[0]).unwrap();
    huge.set_len(64 * 1024 * 1024).unwrap();
    drop(huge);
    let said = stop();
    assert!(
        said.contains("UNJUDGED") && said.contains("it is larger than any state the hooks write"),
        "{said}"
    );
    assert_eq!(
        std::fs::metadata(&state[0]).unwrap().len(),
        64 * 1024 * 1024
    );
    std::fs::write(&state[0], "{\"schema\":\"deep-diff-forge.hook-state.v1\"}").unwrap();
    let said = stop();
    assert!(
        said.contains("UNJUDGED")
            && said.contains("it is not a state of this version of the hooks"),
        "{said}"
    );
    let started = start(&root, "s");
    assert!(
        started.starts_with("deep-diff-forge: the stops of this session will not be judged")
            && started.contains("cannot be read as a state"),
        "{started}"
    );
    assert_eq!(
        std::fs::read_to_string(&state[0]).unwrap(),
        "{\"schema\":\"deep-diff-forge.hook-state.v1\"}"
    );
    // With the file removed the session is recorded anew, and its first stop says that its
    // start was not recorded.
    std::fs::remove_file(&state[0]).unwrap();
    let (_, stdout, _) = hook("post-tool", &[], &event);
    assert!(stdout.contains("check_suppressed"), "{stdout}");
    assert!(stop().contains("This session's start was not recorded"));
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
        assert!(!reason.contains("Uncommitted"), "{session}: {reason}");
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

    // Round 11: a nested repository with no commit (`git init` in a new directory). Git
    // cannot stage one, so every stop of such a tree was UNJUDGED (and before round 10,
    // silent). It is left out of the staged tree like the one that has a commit, the project
    // is judged, and what was left out is named.
    let empty = root.join("scratch");
    std::fs::create_dir(&empty).unwrap();
    git(&empty, &["init", "--quiet"]);
    std::fs::write(empty.join("y"), "y\n").unwrap();
    start(&root, "empty-nested");
    let decision = stop_from(&empty, "empty-nested", &script);
    let reason = blocked(&decision);
    assert!(reason.contains("check_suppressed in \"a.rs\""), "{reason}");
    for nested in ["scratch", "vendor"] {
        assert!(
            reason.contains(&format!(
                "The directory \"{nested}\" in \"{}\" is a Git repository of its own and no part of what this project would commit: it was left out and not judged.",
                std::fs::canonicalize(&root).unwrap().display()
            )),
            "{reason}"
        );
    }
    // HR2-M-3: and the repository the session is standing in is named as not judged
    assert!(
        reason.contains("The session's directory is in another repository, ")
            && reason.contains("nothing in it was judged"),
        "{reason}"
    );
    // each of these is said once in a session
    let again = stop_from(&empty, "empty-nested", &script);
    let stands = allowed(&again);
    assert!(
        stands.starts_with("deep-diff-forge: FAIL STANDS. ")
            && !stands.contains("is a Git repository of its own")
            && !stands.contains("another repository"),
        "{stands}"
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
    let path = linked.to_str().unwrap();
    git(&root, &["worktree", "add", "--quiet", "-b", "side", path]);
    let linked = std::fs::canonicalize(&linked).unwrap();
    let top = std::fs::canonicalize(&root).unwrap();
    // a tree is named as every path is: quoted, and cut at 120 characters
    let tree = |path: &Path, outcome: &str| format!("{}: {outcome}\n", uncommitted_in(path));

    // the change is in the linked tree, where the session is; the project itself is clean
    start(&root, "s1");
    silence_a_lint(&linked);
    let decision = stop_from(&linked, "s1", &script);
    let reason = blocked(&decision);
    // Round 12: a tree with no change is not listed
    assert!(
        !reason.contains(&uncommitted_in(&top)) && reason.contains(&tree(&linked, "FAIL")),
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
    // committed there: the commit is the session's and is judged; neither tree has anything
    // uncommitted
    start(&root, "s2");
    git(
        &linked,
        &["commit", "--quiet", "-am", "work in the linked tree"],
    );
    let work = short(&linked, "HEAD");
    let decision = stop_from(&linked, "s2", &script);
    let reason = blocked(&decision);
    assert!(
        reason.contains(&format!("Commit {work}: FAIL\n"))
            && !reason.contains("Uncommitted changes in "),
        "{reason}"
    );

    // the change is in the project, the session is in the (clean) linked tree
    git(&linked, &["reset", "--quiet", "--hard", "main"]);
    start(&root, "s3");
    silence_a_lint(&root);
    let decision = stop_from(&linked, "s3", &script);
    let reason = blocked(&decision);
    assert!(
        reason.contains(&tree(&top, "FAIL")) && !reason.contains(&uncommitted_in(&linked)),
        "{reason}"
    );
    // both changed in the same way: one patch, shown to the judge once. Round 12: a failure
    // of an uncommitted change is known by its rules, signals, paths and numbers of items,
    // whichever tree it is in, and this one has blocked: it stands in both.
    let runs = || {
        std::fs::read_to_string(&log)
            .unwrap()
            .matches("ARGS ")
            .count()
    };
    let before = runs();
    silence_a_lint(&linked);
    let decision = stop_from(&linked, "s3", &script);
    let said = allowed(&decision);
    assert!(
        said.starts_with("deep-diff-forge: FAIL STANDS. ")
            && said.contains(&tree(&top, "FAIL, which stands"))
            && said.contains(&tree(&linked, "FAIL, which stands")),
        "{said}"
    );
    assert_eq!(runs(), before + 1);
    // changed differently: each is shown to the judge as its own patch
    let before = runs();
    std::fs::write(linked.join("new.rs"), "#[allow(unused)]\nfn n() {}\n").unwrap();
    let decision = stop_from(&linked, "s3", &script);
    let said = allowed(&decision);
    assert!(
        said.contains(&tree(&top, "FAIL, which stands"))
            && said.contains(&tree(&linked, "FAIL, which stands")),
        "{said}"
    );
    let seen = std::fs::read_to_string(&log).unwrap();
    assert_eq!(runs(), before + 2, "{seen}");

    // a session that began in the linked tree has that tree as its project, and one tree
    git(&root, &["checkout", "--quiet", "--", "a.rs"]);
    start(&linked, "s4");
    let decision = stop_from(&linked, "s4", &script);
    let reason = blocked(&decision);
    assert!(!reason.contains("Uncommitted changes in "), "{reason}");
    // nothing was left behind, and the hooks' directory is the repository's, not the tree's
    assert!(root.join(".git/deep-diff-forge").is_dir());
    assert_eq!(
        std::fs::read_dir(root.join(".git/deep-diff-forge"))
            .unwrap()
            .count(),
        0
    );
    assert_eq!(
        git(&linked, &["status", "--porcelain=v1"]),
        " M a.rs\n?? new.rs\n"
    );
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
    let since = format!(
        "changes since this session began (commit {}; this includes commits pulled or made by others in that time)",
        &base[..7]
    );

    start(&root, "s");
    silence_a_lint(&root);
    git(&root, &["commit", "--quiet", "-am", "work"]);
    let work = short(&root, "HEAD");
    assert_eq!(git(&root, &["status", "--porcelain=v1"]), "");
    let decision = stop_from(&root, "s", &script);
    let reason = blocked(&decision);
    assert!(reason.contains(&since), "{reason}");
    assert!(
        reason.contains(&format!("Commit {work}: FAIL\n")),
        "{reason}"
    );
    // the branch has only gone on from the base: nothing says that it left it
    assert!(!reason.contains("the branch has moved"), "{reason}");
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
    let policy = short(&root, "HEAD");
    std::fs::write(root.join("new.rs"), "#[allow(unused)]\nfn n() {}\n").unwrap();
    let decision = stop_from(&root, "s", &script);
    let reason = blocked(&decision);
    // Round 12: three units. The uncommitted new file and the commit of the policy are judged
    // at this stop; the commit of the work was judged at the first one and stands.
    assert!(
        reason.contains(&format!(
            "{}: FAIL\n",
            uncommitted_in(&std::fs::canonicalize(&root).unwrap())
        )) && reason.contains(&format!("Commit {work}: FAIL, which stands\n"))
            && reason.contains(&format!("Commit {policy}: FAIL\n")),
        "{reason}"
    );
    let seen = std::fs::read_to_string(&log).unwrap();
    assert!(!seen.contains("loosened"), "{seen}");
    // three runs of the judge: the first stop, and at this one the new file and the commit
    // of the policy; each by the base's policy, each a patch of one file
    assert_eq!(
        seen.matches("POLICY # the policy at the base commit\n")
            .count(),
        3,
        "{seen}"
    );
    assert_eq!(seen.matches("PATCH-FILES 1\n").count(), 3, "{seen}");

    // A later start of the same session (a resume, a compaction) does not move the base.
    let again = start(&root, "s");
    assert!(
        again.contains(&format!(
            "(commit {}, recorded when the session first started)",
            &base[..7]
        )),
        "{again}"
    );
    // nothing changed since the last stop: the failure has blocked once and stands
    assert!(allowed(&stop_from(&root, "s", &script)).starts_with("deep-diff-forge: FAIL STANDS. "));

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
/// `HEAD` on history that does not contain the base: the message says the branch moved, and
/// the policy is still the base's. Since round 12 the uncommitted change is measured from
/// the tree's own `HEAD`, so a branch that was only put back has nothing to judge. A base
/// whose commit is gone from the repository has no policy to read: a stop with something to
/// judge is UNJUDGED and says so.
#[test]
fn a_base_the_branch_left_is_still_judged_and_one_that_is_gone_is_unjudged() {
    let scratch = Scratch::new("hook-base-moved");
    let root = repository(&scratch);
    let (script, log, _) = failing_judge(&scratch);
    let first = git(&root, &["rev-parse", "HEAD"]);
    std::fs::write(root.join("b.rs"), "fn c() {}\n").unwrap();
    git(&root, &["add", "b.rs"]);
    git(&root, &["commit", "--quiet", "-m", "second"]);
    let second = git(&root, &["rev-parse", "HEAD"]);

    start(&root, "s");
    git(&root, &["reset", "--quiet", "--hard", first.trim()]);
    // Round 12: a branch put back to a commit that was there before holds no commit of the
    // session and leaves nothing uncommitted, so there is nothing to judge.
    assert_eq!(stop_from(&root, "s", &script), Value::Null);
    assert_eq!(runs(&log), 0);
    // an uncommitted change there is judged, by the policy of the commit the session began
    // from, and the message says that the branch has left that commit
    silence_a_lint(&root);
    let decision = stop_from(&root, "s", &script);
    let reason = blocked(&decision);
    assert!(
        reason.contains(&format!(
            "changes since this session began (commit {}; this includes commits pulled or made by others in that time); the branch has moved and no longer contains that commit",
            &second[..7]
        )),
        "{reason}"
    );
    // the same when HEAD names no commit at all any more (an orphan branch): the tree is
    // measured from the session's base then
    git(&root, &["checkout", "--quiet", "--orphan", "wip"]);
    let decision = stop_from(&root, "s", &script);
    // the same failure as a moment ago: it has blocked once and is still there, so it stands
    assert!(
        allowed(&decision).starts_with("deep-diff-forge: FAIL STANDS. ")
            && allowed(&decision).contains("the branch has moved"),
        "{decision}"
    );

    // the commit itself is gone
    git(&root, &["checkout", "--quiet", "-f", "main"]);
    git(&root, &["reflog", "expire", "--expire=now", "--all"]);
    git(&root, &["gc", "--quiet", "--prune=now"]);
    // nothing to judge, so nothing needs the policy of the commit that is gone
    assert_eq!(stop_from(&root, "s", &script), Value::Null);
    // with something to judge the policy cannot be read: UNJUDGED, said
    silence_a_lint(&root);
    let before = runs(&log);
    let decision = stop_from(&root, "s", &script);
    let said = allowed(&decision);
    assert!(
        said.starts_with("deep-diff-forge: UNJUDGED. ")
            && said.contains("the policy could not be read from Git"),
        "{said}"
    );
    assert_eq!(runs(&log), before);
    // advice says it once, then stays silent
    let event = input("PostToolUse", &root, "s", false);
    let (_, stdout, _) = hook("post-tool", &[], &event);
    assert!(
        system_message(&stdout).contains(&format!(
            "the commit this session began from ({}) can no longer be read",
            &second[..7]
        )),
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
    use std::os::unix::fs::PermissionsExt as _;
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
    // nothing repaired: the same failure does not block twice, at this turn or the next
    assert!(allowed(&stop(true)).starts_with("deep-diff-forge: FAIL STANDS. "));
    assert!(allowed(&stop(false)).starts_with("deep-diff-forge: FAIL STANDS. "));
    // two more items under the same rule in the same file: a new failure
    write_program(&script, &judge_script(&log, &exit_file, &citing(3)));
    blocked(&stop(true));
    assert!(allowed(&stop(true)).starts_with("deep-diff-forge: FAIL STANDS. "));
    // Round 12 (HR3-M-3): the first one is no longer there at that stop, so it was forgotten;
    // when it comes back it blocks again, once
    write_program(&script, &judge_script(&log, &exit_file, &citing(1)));
    blocked(&stop(false));
    assert!(allowed(&stop(false)).starts_with("deep-diff-forge: FAIL STANDS. "));

    // The state cannot be kept: its directory can be read and not written (not as root, who
    // writes it all the same). A failure that has not blocked before then blocks at every
    // stop, and each time the stop says why.
    let sessions = sessions();
    std::fs::set_permissions(&sessions, std::fs::Permissions::from_mode(0o500)).unwrap();
    if std::fs::write(sessions.join("probe"), "x").is_err() {
        write_program(&script, &judge_script(&log, &exit_file, &citing(5)));
        for active in [false, true] {
            let decision = stop(active);
            let reason = blocked(&decision);
            assert!(
                reason
                    .contains("The hook could not keep its state (its state could not be saved: ")
                    && reason.contains("the same failure can block again"),
                "{reason}"
            );
        }
    }
    std::fs::set_permissions(&sessions, std::fs::Permissions::from_mode(0o700)).unwrap();
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
    let all = |not: &[&str]| (if not.is_empty() { 0 } else { 7 }, gate_but(not));
    let gate = || {
        let (code, doc, _) = doctor_gate(&root, &home, &judge_arg);
        ((code, gate_checks(&doc)), doc)
    };
    let patch_fails = ["patch"];
    assert_eq!(gate().0, all(&[]));

    // a path marked skip-worktree: a stop here is UNJUDGED
    git(&root, &["update-index", "--skip-worktree", "a.rs"]);
    let (found, doc) = gate();
    assert_eq!(found, all(&patch_fails));
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
        assert_eq!(found, all(&patch_fails));
        assert!(
            detail(&doc, "patch").contains("a stop here would be UNJUDGED"),
            "{doc:#}"
        );
    }
    std::fs::remove_file(&unreadable).unwrap();
    // a tree with a change: the patch can be made, and the gate says how large it is
    silence_a_lint(&root);
    let (found, doc) = gate();
    assert_eq!(found, all(&[]));
    assert!(
        detail(&doc, "patch").starts_with("the patch a stop would judge can be made ("),
        "{doc:#}"
    );
    assert_eq!(git(&root, &["status", "--porcelain=v1"]), " M a.rs\n");

    // `disableAllHooks`: no hook runs, however well it is registered
    let mut disabled = registered.clone();
    disabled["disableAllHooks"] = json!(true);
    write(&user, &disabled);
    let hooks_off = ["hooks_enabled"];
    let (found, doc) = gate();
    assert_eq!(found, all(&hooks_off));
    assert!(
        detail(&doc, "hooks_enabled").contains("sets disableAllHooks"),
        "{doc:#}"
    );
    // a file that wins over it switches them on again, and one that wins switches them off
    std::fs::create_dir(root.join(".claude")).unwrap();
    let local = root.join(".claude/settings.local.json");
    write(&local, &json!({"disableAllHooks": false}));
    assert_eq!(gate().0, all(&[]));
    write(&user, &registered);
    write(&local, &json!({"disableAllHooks": true}));
    assert_eq!(gate().0, all(&hooks_off));
    std::fs::remove_file(&local).unwrap();

    // the Stop hook's timeout: under what a stop can take, the client ends the hook
    let timed = |timeout: Value| {
        let mut settings = registered.clone();
        settings["hooks"]["Stop"][0]["hooks"][0]["timeout"] = timeout;
        settings
    };
    let too_short = ["hook_timeout"];
    for timeout in [json!(1), json!(49), json!("60")] {
        write(&user, &timed(timeout.clone()));
        let (found, doc) = gate();
        assert_eq!(found, all(&too_short), "{timeout}");
        assert!(
            detail(&doc, "hook_timeout").contains("under the 50 seconds a stop can take"),
            "{doc:#}"
        );
    }
    for timeout in [json!(50), json!(60), json!(600), json!(null)] {
        write(&user, &timed(timeout.clone()));
        assert_eq!(gate().0, all(&[]), "{timeout}");
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
    assert_eq!(found, all(&NOT_REGISTERED));
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

    // An input of exactly the cap is read; and one four mebibytes over it is refused after all
    // of it was taken from the pipe (the writer here would fail on a pipe that was closed
    // early). A mutation run left both bounds alive.
    let cap = 32 * 1024 * 1024;
    let frame = with("Stop", "last_assistant_message", 0).to_string().len();
    let exact = with("Stop", "last_assistant_message", cap - frame);
    assert_eq!(exact.to_string().len(), cap);
    let (_, stdout, _) = hook("stop", &["--judge", script.to_str().unwrap()], &exact);
    assert!(
        allowed(&serde_json::from_str(&stdout).unwrap())
            .starts_with("deep-diff-forge: FAIL STANDS. "),
        "{}",
        &stdout[..stdout.len().min(300)]
    );
    let over = with("Stop", "last_assistant_message", cap + 4 * 1024 * 1024);
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
    // two files: the state, and the list of commits that were there when the session began
    assert_eq!(files(&xdg.join("deep-diff-forge/hooks")), 2);
    assert_eq!(files(&sessions()), 0);
    let (_, stdout, _) = hook_in("post-tool", &[], &event, |command| {
        command
            .env("XDG_STATE_HOME", "relative/state")
            .current_dir(&root);
    });
    assert!(stdout.contains("check_suppressed"), "{stdout}");
    assert_eq!(files(&sessions()), 2);
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
    const NOT_RECORDED: &str = "This session's start was not recorded: its first hook here ran after the session had begun, so work committed before that hook was not judged. For a session to be judged from its start, the session-start hook must be installed before the session begins";
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
    // the same failure stands at the next stop (a failure blocks once), without the sentence
    let again = system_message(&stop("fails", &root, None));
    assert!(
        again.starts_with("deep-diff-forge: FAIL STANDS. ") && !again.contains(NOT_RECORDED),
        "{again}"
    );

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

// ---- Round 10 of Vesper's review and the second independent review (2026-11-14, both NO-GO) ----

/// The number of times the stand-in judge was run, by its log.
fn runs(log: &Path) -> usize {
    std::fs::read_to_string(log).map_or(0, |seen| seen.matches("ARGS check ").count())
}

/// Second independent review of the hooks, HR2-H-1. Only the working tree was compared with
/// the commit the session began from, so work the session committed was judged only while the
/// tree still held it. The ordinary branch workflow (branch, commit, switch back), a commit of
/// a staged file that was edited again, and a commit the tree was put back from all ended in a
/// stop with no output and the judge never asked. A stop now also has judged what the session
/// committed. Since round 12 each commit is judged by itself and named by its id: the commits
/// that a branch, a tag, the stash, `HEAD` or their reflogs lead to.
#[test]
fn commits_the_session_made_are_judged_wherever_a_ref_names_them() {
    let scratch = Scratch::new("hook-session-commits");
    let root = repository(&scratch);
    let (script, log, _) = failing_judge(&scratch);
    let top = std::fs::canonicalize(&root).unwrap();
    let base = git(&root, &["rev-parse", "HEAD"]);
    let base = base.trim();
    let harmless = "fn a() {}\nfn b() {}\nfn c() {}\n";
    // back to the base on `main`, with nothing left of the case before
    let reset = || {
        git(&root, &["checkout", "--quiet", "-f", "main"]);
        git(&root, &["reset", "--quiet", "--hard", base]);
        git(&root, &["clean", "--quiet", "-fdx"]);
    };

    // Before any of it: refs that hold nothing new are not judged. A branch and a tag made at
    // the base, and a branch moved back, leave a clean stop silent.
    start(&root, "nothing-new");
    git(&root, &["branch", "at-base"]);
    git(&root, &["tag", "v0"]);
    assert_eq!(stop_from(&root, "nothing-new", &script), Value::Null);
    assert_eq!(runs(&log), 0);

    // 2b: the ordinary branch workflow. Branch, commit, switch back.
    start(&root, "2b");
    git(&root, &["checkout", "--quiet", "-b", "feature"]);
    silence_a_lint(&root);
    git(&root, &["commit", "--quiet", "-am", "work"]);
    let work = short(&root, "HEAD");
    git(&root, &["checkout", "--quiet", "main"]);
    assert_eq!(git(&root, &["status", "--porcelain=v1"]), "");
    let decision = stop_from(&root, "2b", &script);
    let reason = blocked(&decision);
    // Round 12: the commit is named by its id; a tree with no change is not listed, and no
    // name of a ref is in the message
    assert!(
        reason.contains(&format!(
            "Commit {work}: FAIL\n- rule no-silent-suppression: "
        )),
        "{reason}"
    );
    assert!(
        !reason.contains("refs/") && !reason.contains("Uncommitted changes in"),
        "{reason}"
    );
    let seen = std::fs::read_to_string(&log).unwrap();
    assert!(seen.ends_with("PATCH-FILES 1\nPATCH-ALLOWS 1\n"), "{seen}");
    assert_eq!(runs(&log), 1);

    // 11b: committed, pushed, and the branch put back. The reflog of `HEAD` names the commit
    // (the remote-tracking ref does too, which makes no commit the session's).
    reset();
    git(&root, &["branch", "--quiet", "-D", "feature"]);
    let origin = scratch.join("origin.git");
    git(
        scratch.path(),
        &["init", "--quiet", "--bare", origin.to_str().unwrap()],
    );
    git(
        &root,
        &["remote", "add", "origin", origin.to_str().unwrap()],
    );
    git(&root, &["push", "--quiet", "origin", "main"]);
    start(&root, "11b");
    silence_a_lint(&root);
    git(&root, &["commit", "--quiet", "-am", "work"]);
    let work = short(&root, "HEAD");
    git(&root, &["push", "--quiet", "origin", "main"]);
    git(&root, &["reset", "--quiet", "--hard", base]);
    let decision = stop_from(&root, "11b", &script);
    let reason = blocked(&decision);
    assert!(
        reason.contains(&format!("Commit {work}: FAIL\n")),
        "{reason}"
    );

    // 2f: staged, edited again, committed without adding again. `HEAD` holds what the working
    // file no longer does: the uncommitted change and the commit are two units, and the
    // commit is the one with the lint.
    reset();
    start(&root, "2f");
    silence_a_lint(&root);
    git(&root, &["add", "--all"]);
    std::fs::write(root.join("a.rs"), harmless).unwrap();
    git(&root, &["commit", "--quiet", "-m", "add c"]);
    let work = short(&root, "HEAD");
    assert_eq!(git(&root, &["status", "--porcelain=v1"]), " M a.rs\n");
    let before = runs(&log);
    let decision = stop_from(&root, "2f", &script);
    let reason = blocked(&decision);
    assert!(
        reason.contains(&format!("{}: FAIL\n", uncommitted_in(&top)))
            && reason.contains(&format!("Commit {work}: FAIL\n")),
        "{reason}"
    );
    assert_eq!(runs(&log), before + 2);
    // the uncommitted change first (it takes the lint out again), then the commit
    let seen = std::fs::read_to_string(&log).unwrap();
    assert!(
        seen.contains("PATCH-FILES 1\nPATCH-ALLOWS 0\nARGS"),
        "{seen}"
    );
    assert!(seen.ends_with("PATCH-ALLOWS 1\n"), "{seen}");
}

/// HR2-H-1, the two cases that need intent: the work is committed and then taken out of the
/// working tree again, by putting the tree back from the base, or by dropping the file from
/// the index and ignoring it. `HEAD` still holds it, and what `HEAD` holds is judged.
#[test]
fn a_commit_the_working_tree_no_longer_shows_is_judged() {
    let scratch = Scratch::new("hook-session-commits-3");
    let root = repository(&scratch);
    let (script, _, _) = failing_judge(&scratch);
    let base = git(&root, &["rev-parse", "HEAD"]);
    let base = base.trim();
    let reset = || {
        git(&root, &["checkout", "--quiet", "-f", "main"]);
        git(&root, &["reset", "--quiet", "--hard", base]);
        git(&root, &["clean", "--quiet", "-fdx"]);
    };

    // 2c: committed, then the working tree put back from the base without a commit.
    reset();
    start(&root, "2c");
    silence_a_lint(&root);
    git(&root, &["commit", "--quiet", "-am", "work"]);
    let work = short(&root, "HEAD");
    git(&root, &["checkout", "--quiet", base, "--", "."]);
    let decision = stop_from(&root, "2c", &script);
    assert!(
        blocked(&decision).contains(&format!("Commit {work}: FAIL\n")),
        "{decision}"
    );

    // 2d: a new file committed, then dropped from the index and ignored.
    reset();
    start(&root, "2d");
    std::fs::create_dir(root.join("build")).unwrap();
    std::fs::write(
        root.join("build/gen.rs"),
        "#[allow(dead_code)]\nfn g() {}\n",
    )
    .unwrap();
    git(&root, &["add", "build/gen.rs"]);
    git(&root, &["commit", "--quiet", "-m", "work"]);
    let work = short(&root, "HEAD");
    git(&root, &["rm", "--quiet", "--cached", "build/gen.rs"]);
    std::fs::write(root.join(".git/info/exclude"), "build/\n").unwrap();
    let decision = stop_from(&root, "2d", &script);
    assert!(
        blocked(&decision).contains(&format!("Commit {work}: FAIL\n")),
        "{decision}"
    );
    std::fs::write(root.join(".git/info/exclude"), "").unwrap();
}

/// HR2-H-1, the other names a commit can have, and the usual case. The stash and a tag name
/// commits as a branch does. A commit on the branch the session is on, with a clean tree, is
/// one run of the judge. Since round 12 there is no bound of four patches: every commit is
/// judged, and each once in a session.
#[test]
fn the_stash_a_tag_and_many_branches_name_commits_too() {
    let scratch = Scratch::new("hook-session-commits-2");
    let root = repository(&scratch);
    let (script, log, exit_file) = failing_judge(&scratch);
    let base = git(&root, &["rev-parse", "HEAD"]);
    let base = base.trim();
    let reset = || {
        git(&root, &["checkout", "--quiet", "-f", "main"]);
        git(&root, &["reset", "--quiet", "--hard", base]);
        git(&root, &["clean", "--quiet", "-fdx"]);
    };

    // The stash and a tag name commits too: work that was stashed away, and a commit only a
    // tag still names.
    reset();
    start(&root, "stash");
    silence_a_lint(&root);
    git(&root, &["stash", "--quiet"]);
    assert_eq!(git(&root, &["status", "--porcelain=v1"]), "");
    let stashed = short(&root, "refs/stash");
    let decision = stop_from(&root, "stash", &script);
    assert!(
        blocked(&decision).contains(&format!("Commit {stashed}: FAIL\n")),
        "{decision}"
    );
    git(&root, &["stash", "drop", "--quiet"]);
    start(&root, "tag");
    git(&root, &["checkout", "--quiet", "--detach"]);
    silence_a_lint(&root);
    git(&root, &["commit", "--quiet", "-am", "work"]);
    let work = short(&root, "HEAD");
    git(&root, &["tag", "kept"]);
    git(&root, &["checkout", "--quiet", "main"]);
    let decision = stop_from(&root, "tag", &script);
    assert!(
        blocked(&decision).contains(&format!("Commit {work}: FAIL\n")),
        "{decision}"
    );
    git(&root, &["tag", "-d", "kept"]);

    // The usual stop stays one run of the judge: committed on the branch the session is on,
    // with a clean tree, there is one commit to judge and nothing uncommitted.
    reset();
    start(&root, "usual");
    silence_a_lint(&root);
    git(&root, &["commit", "--quiet", "-am", "work"]);
    let work = short(&root, "HEAD");
    let before = runs(&log);
    let decision = stop_from(&root, "usual", &script);
    let reason = blocked(&decision);
    assert!(
        reason.contains(&format!("Commit {work}: FAIL\n")),
        "{reason}"
    );
    assert_eq!(runs(&log), before + 1);

    // Round 12: no bound of four patches. Six branches with a commit each are six commits,
    // each judged, and each once in the session: the second stop asks the judge nothing.
    reset();
    std::fs::write(&exit_file, "0").unwrap();
    start(&root, "many");
    for n in 0..6 {
        git(
            &root,
            &["checkout", "--quiet", "-b", &format!("b{n}"), base],
        );
        std::fs::write(root.join(format!("f{n}.rs")), "fn f() {}\n").unwrap();
        git(&root, &["add", "--all"]);
        git(&root, &["commit", "--quiet", "-m", "one file"]);
    }
    git(&root, &["checkout", "--quiet", "main"]);
    let before = runs(&log);
    assert_eq!(stop_from(&root, "many", &script), Value::Null);
    assert_eq!(runs(&log), before + 6);
    assert_eq!(stop_from(&root, "many", &script), Value::Null);
    assert_eq!(runs(&log), before + 6);
}

/// Second independent review of the hooks, HR2-M-1, the reviewer's case: a failure with 4,000
/// violations on long paths. The state kept one line for each, wrote 1.1 MB, and the next hook
/// read it as no state: the base became the commit `HEAD` named then, and the same failure,
/// committed, passed in silence. The state keeps a digest of the failure now and is read back.
#[test]
fn a_failure_with_thousands_of_violations_is_remembered() {
    let scratch = Scratch::new("hook-large-failure");
    let root = repository(&scratch);
    let (script, log, exit_file) = failing_judge(&scratch);
    let mut document = verdict("fail", "no-silent-suppression", "check_suppressed", "a.rs");
    let violations: Vec<Value> = (0..4_000)
        .map(|n| {
            json!({"signal": "check_suppressed", "message": "m",
                   "path": format!("{}/{n:04}.rs", "p".repeat(230))})
        })
        .collect();
    document["rules"][0]["violations"] = json!(violations);
    write_program(&script, &judge_script(&log, &exit_file, &document));
    let base = git(&root, &["rev-parse", "HEAD"]);

    start(&root, "s");
    silence_a_lint(&root);
    blocked(&stop_from(&root, "s", &script));
    let state = states_in(&sessions());
    assert_eq!(state.len(), 1, "{state:?}");
    let kept = std::fs::read_to_string(&state[0]).unwrap();
    assert!(kept.len() < 4_096, "{} bytes", kept.len());
    assert!(kept.contains(base.trim()), "{kept}");
    // the agent commits everything; the hook after that tool call, then the stop
    git(&root, &["commit", "--quiet", "-am", "work"]);
    assert_eq!(
        hook("post-tool", &[], &input("PostToolUse", &root, "s", false)).0,
        0
    );
    let kept = std::fs::read_to_string(&state[0]).unwrap();
    assert!(kept.contains(base.trim()), "the base was forgotten: {kept}");
    // not silent, and not a pass: the same failure, which has blocked once, stands
    for _ in 0..2 {
        let decision = stop_from(&root, "s", &script);
        assert!(
            allowed(&decision).starts_with("deep-diff-forge: FAIL STANDS. "),
            "{decision}"
        );
    }
}

/// Second independent review of the hooks, HR2-M-2. Only a stop that was itself the session's
/// first hook said that the start was not recorded. A commit is made by a tool call, and the
/// hook after that call ran first and recorded in silence: with the three printed hooks the
/// sentence could never be said for a commit the agent made. Whoever records without a
/// session-start hook, the first stop says it, once.
#[test]
fn a_start_that_was_not_recorded_is_said_at_the_stop_whoever_recorded() {
    const NOT_RECORDED: &str = "This session's start was not recorded: its first hook here ran after the session had begun, so work committed before that hook was not judged.";
    let scratch = Scratch::new("hook-late-start");
    let root = repository(&scratch);
    let (script, log, exit_file) = failing_judge(&scratch);
    std::fs::write(&exit_file, "0").unwrap();
    let post = |session: &str| {
        hook(
            "post-tool",
            &[],
            &input("PostToolUse", &root, session, false),
        )
    };

    // 2e: no session-start hook ran; the work is committed; the hook after the tool call,
    // then the stop
    silence_a_lint(&root);
    git(&root, &["commit", "--quiet", "-am", "work"]);
    assert_eq!(post("2e"), (0, String::new(), String::new()));
    let decision = stop_from(&root, "2e", &script);
    let said = allowed(&decision);
    assert!(
        said.starts_with(&format!("deep-diff-forge: {NOT_RECORDED}")),
        "{said}"
    );
    assert_eq!(runs(&log), 0);
    // once: the next stop of the session is ordinary
    assert_eq!(stop_from(&root, "2e", &script), Value::Null);

    // 2a: a started session loses its state in the same tool call as a commit
    start(&root, "2a");
    std::fs::write(root.join("b.rs"), "#[allow(unused)]\nfn n() {}\n").unwrap();
    git(&root, &["add", "--all"]);
    git(&root, &["commit", "--quiet", "-m", "more work"]);
    for entry in std::fs::read_dir(sessions()).unwrap() {
        std::fs::remove_file(entry.unwrap().path()).unwrap();
    }
    assert_eq!(post("2a").1, "");
    let decision = stop_from(&root, "2a", &script);
    assert!(allowed(&decision).contains(NOT_RECORDED), "{decision}");
    // and the messages of such a session do not say "since this session began"
    std::fs::write(&exit_file, "7").unwrap();
    std::fs::write(root.join("c.rs"), "#[allow(unused)]\nfn m() {}\n").unwrap();
    let decision = stop_from(&root, "2a", &script);
    let reason = blocked(&decision);
    assert!(
        reason.contains("which HEAD named when this session's hooks first ran")
            && !reason.contains("since this session began")
            && !reason.contains(NOT_RECORDED),
        "{reason}"
    );
    // a session that did start is never told this
    start(&root, "started");
    assert_eq!(post("started").0, 0);
    let decision = stop_from(&root, "started", &script);
    assert!(!blocked(&decision).contains(NOT_RECORDED), "{decision}");
}

/// Second independent review of the hooks, HR2-M-3. A repository other than the session's
/// project was never judged and nothing was said, also when the session's directory was in it
/// at the stop; the same for changes inside a submodule that are not committed there (the
/// project's patch shows a submodule only as the commit it is at). Each is named now, once in
/// a session.
#[test]
fn another_repository_and_a_submodule_with_changes_are_named_once() {
    let scratch = Scratch::new("hook-other-repository");
    let root = repository(&scratch);
    let (script, _, exit_file) = failing_judge(&scratch);
    std::fs::write(&exit_file, "0").unwrap();
    let top = std::fs::canonicalize(&root).unwrap();
    let other = scratch.join("other");
    std::fs::create_dir(&other).unwrap();
    git(&other, &["init", "--quiet"]);
    std::fs::write(other.join("a.rs"), "fn a() {}\n").unwrap();
    git(&other, &["add", "--all"]);
    git(&other, &["commit", "--quiet", "-m", "base"]);
    let other_top = std::fs::canonicalize(&other).unwrap();

    // 3a: the session's project is one repository, and it does its work in another
    start(&root, "3a");
    silence_a_lint(&other);
    let decision = stop_from(&other, "3a", &script);
    let said = allowed(&decision);
    assert_eq!(
        said,
        format!(
            "deep-diff-forge: The session's directory is in another repository, \"{}\", which is not this session's project (\"{}\"): nothing in it was judged.",
            other_top.display(),
            top.display()
        )
    );
    // once for that repository; from the project itself there is nothing to say
    assert_eq!(stop_from(&other, "3a", &script), Value::Null);
    assert_eq!(stop_from(&root, "3a", &script), Value::Null);

    // 3c: a real submodule, with a change inside it that is not committed there
    git(
        &root,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "--quiet",
            "add",
            other.to_str().unwrap(),
            "vendor/sub",
        ],
    );
    git(&root, &["commit", "--quiet", "-m", "add a submodule"]);
    start(&root, "3c");
    let inside = root.join("vendor/sub");
    silence_a_lint(&inside);
    assert_eq!(git(&root, &["status", "--porcelain=v1"]), " M vendor/sub\n");
    let submodule = format!(
        "The submodule \"vendor/sub\" of \"{}\" has changes that are not committed in it: they were not judged (the project's patch shows a submodule only as the commit it is at).",
        top.display()
    );
    // from the top of the project
    let decision = stop_from(&root, "3c", &script);
    assert_eq!(allowed(&decision), format!("deep-diff-forge: {submodule}"));
    assert_eq!(stop_from(&root, "3c", &script), Value::Null);
    // and from inside the submodule, which is a repository of its own: both things are said
    start(&root, "3c-inside");
    let decision = stop_from(&inside, "3c-inside", &script);
    let said = allowed(&decision);
    assert!(
        said.contains(&submodule)
            && said.contains("The session's directory is in another repository, ")
            && said.contains("vendor/sub\", which is not this session's project"),
        "{said}"
    );
    assert_eq!(stop_from(&inside, "3c-inside", &script), Value::Null);
}

/// Second independent review of the hooks, HR2-M-4. With a base that does not move, a failure
/// stayed for the whole session and blocked the first stop of every later turn, also after a
/// person had let it stand and it was committed. A failure blocks once in a session; after
/// that it is said to stand, at every stop, and left for a person. And no message calls the
/// range the session's own work or asks it to repair what it may not have made: a pull brings
/// other people's commits into the range.
#[test]
fn a_failure_blocks_once_in_a_session_also_over_later_turns() {
    let scratch = Scratch::new("hook-fail-once");
    let root = repository(&scratch);
    let (script, _, _) = failing_judge(&scratch);
    let stop = |active: bool| -> Value {
        let (code, stdout, _) = hook(
            "stop",
            &["--judge", script.to_str().unwrap()],
            &input("Stop", &root, "s", active),
        );
        assert_eq!(code, 0);
        serde_json::from_str(&stdout).unwrap()
    };
    let stands = |decision: &Value| {
        let said = allowed(decision);
        assert!(
            said.starts_with("deep-diff-forge: FAIL STANDS. This failure has already blocked a stop of this session"),
            "{said}"
        );
    };
    start(&root, "s");
    start(&root, "other");
    silence_a_lint(&root);
    // 5a: it fails and blocks; continuing, it stands; the person accepts it and it is committed
    let first = stop(false);
    let reason = blocked(&first);
    for unfit in ["your changes", "Repair what the rules name"] {
        assert!(!reason.contains(unfit), "{unfit}: {reason}");
    }
    assert!(
        reason.contains("this includes commits pulled or made by others in that time")
            && reason.contains("what they name in work it did not make is for a person"),
        "{reason}"
    );
    stands(&stop(true));
    git(&root, &["commit", "--quiet", "-am", "accepted"]);
    // turn 2 and turn 3, nothing changed: no block again
    for _ in 0..2 {
        stands(&stop(false));
        stands(&stop(true));
    }
    // 11c: another session in the same checkout is blocked by it once of its own, not more
    let other = |active: bool| -> Value {
        let (_, stdout, _) = hook(
            "stop",
            &["--judge", script.to_str().unwrap()],
            &input("Stop", &root, "other", active),
        );
        serde_json::from_str(&stdout).unwrap()
    };
    blocked(&other(false));
    stands(&other(false));
    // a failure that grows is a new one, in the first session as well
    std::fs::write(root.join("new.rs"), "#[allow(unused)]\nfn n() {}\n").unwrap();
    let advice = hook("post-tool", &[], &input("PostToolUse", &root, "s", false)).1;
    assert!(
        advice.contains("new findings in the changes since this session began")
            && !advice.contains("your changes"),
        "{advice}"
    );
    blocked(&stop(false));
    stands(&stop(false));
}

/// Review V4 round 10, VD4R10-H-1, exactly as reproduced: the project is moved away and a clean
/// clone of it is put at its recorded path; the stop names the old path as the project and has
/// its directory in the moved original. The clean clone was taken for the project, the changed
/// original for an unrelated repository, and the stop said nothing. A project is the
/// repository that was recorded (the inode of its Git common directory) at the path it was
/// recorded at: since round 12 a project that was moved is not judged where the session
/// finds it, but is UNJUDGED like the clone, with the remedy (a new session).
#[test]
fn a_clone_put_at_the_projects_path_is_not_the_project() {
    use std::os::unix::fs::MetadataExt as _;
    let scratch = Scratch::new("hook-moved-project");
    let root = repository(&scratch);
    let (script, log, _) = failing_judge(&scratch);
    let moved = scratch.join("project-moved");
    let stop = |session: &str, cwd: &Path, project: Option<&Path>| -> Value {
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
        if stdout.is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&stdout).unwrap()
        }
    };
    let (code, _, _) = hook_in(
        "session-start",
        &[],
        &input("SessionStart", &root, "s", false),
        |command| {
            command.env("CLAUDE_PROJECT_DIR", &root);
        },
    );
    assert_eq!(code, 0);
    let recorded = std::fs::canonicalize(&root).unwrap();
    // HR3-M-4: the repository is recorded by the inode of its Git directory, with no device
    // number before it (that one is given when the file system is mounted)
    let state = states_in(&sessions());
    let kept: Value = serde_json::from_str(&std::fs::read_to_string(&state[0]).unwrap()).unwrap();
    assert_eq!(
        kept["project"]["id"],
        std::fs::metadata(root.join(".git"))
            .unwrap()
            .ino()
            .to_string()
    );
    silence_a_lint(&root);
    std::fs::rename(&root, &moved).unwrap();
    git(
        scratch.path(),
        &[
            "clone",
            "--quiet",
            "--no-local",
            moved.to_str().unwrap(),
            root.to_str().unwrap(),
        ],
    );
    assert_eq!(git(&root, &["status", "--porcelain=v1"]), "");
    assert_eq!(git(&moved, &["status", "--porcelain=v1"]), " M a.rs\n");

    // Vesper's stop (the client still names the old path, the session is in the moved
    // original), the same with no project named, and from anywhere else: the clone at the
    // recorded path is not the project, and the stop says so instead of judging it clean.
    // Round 12: the moved original is not judged in this session either (the project is a
    // repository at a path); the message names the remedy.
    for (cwd, project) in [
        (&moved, Some(root.as_path())),
        (&moved, None),
        (&root, Some(root.as_path())),
        (&root, None),
        (&scratch.join("nowhere"), None),
    ] {
        let decision = stop("s", cwd, project);
        let said = allowed(&decision);
        assert!(
            said.starts_with("deep-diff-forge: UNJUDGED. ")
                && said.contains(&format!(
                    "the repository at \"{}\" is not the one this session started in",
                    recorded.display()
                ))
                && said.contains("start a new session to have the work there judged"),
            "{said}"
        );
    }
    assert_eq!(runs(&log), 0);
    // advice says it once, and the start of the session says it too
    let event = input("PostToolUse", &root, "s", false);
    assert!(
        system_message(&hook("post-tool", &[], &event).1)
            .contains("is not the one this session started in")
    );
    assert_eq!(hook("post-tool", &[], &event).1, "");
    assert!(start(&root, "s").contains("is not the one this session started in"));
    // the remedy: a new session in the moved original has its work judged
    start(&moved, "s2");
    std::fs::write(moved.join("new.rs"), "#[allow(unused)]\nfn n() {}\n").unwrap();
    let decision = stop("s2", &moved, None);
    assert!(
        blocked(&decision).contains("check_suppressed in \"a.rs\""),
        "{decision}"
    );
    assert_eq!(runs(&log), 1);
}

/// The second review's "weak tests": the two refusals in finding the project had no test, so a
/// hook that silently took whatever repository it found would have passed every test. A
/// project the client names that is another repository than the recorded one, and a recorded
/// project that is gone, are each UNJUDGED with the paths named.
#[test]
fn a_project_that_is_another_or_is_gone_is_refused_by_name() {
    let scratch = Scratch::new("hook-locate-refusals");
    let root = repository(&scratch);
    let (script, log, _) = failing_judge(&scratch);
    let other = scratch.join("other");
    std::fs::create_dir(&other).unwrap();
    git(&other, &["init", "--quiet"]);
    std::fs::write(other.join("a.rs"), "fn a() {}\n").unwrap();
    git(&other, &["add", "--all"]);
    git(&other, &["commit", "--quiet", "-m", "base"]);
    start(&root, "s");
    silence_a_lint(&root);
    silence_a_lint(&other);
    let recorded = std::fs::canonicalize(&root).unwrap();

    // the client names another repository than the one recorded (the reviewer's 3d)
    let (code, stdout, _) = hook_in(
        "stop",
        &["--judge", script.to_str().unwrap()],
        &input("Stop", &root, "s", false),
        |command| {
            command.env("CLAUDE_PROJECT_DIR", &other);
        },
    );
    assert_eq!(code, 0);
    let said = system_message(&stdout);
    assert_eq!(
        said,
        format!(
            "deep-diff-forge: UNJUDGED. This stop was not judged: this session began in \"{}\", and the project found now is \"{}\"; start a new session to have the work there judged. That is not a pass.",
            recorded.display(),
            std::fs::canonicalize(&other).unwrap().display()
        )
    );
    // the recorded project is gone from its path. Round 12: also when the session is in the
    // directory it was moved to (a project is a repository at a path)
    std::fs::rename(&root, scratch.join("put-away")).unwrap();
    for cwd in [&other, &scratch.join("nowhere"), &scratch.join("put-away")] {
        let decision = stop_from(cwd, "s", &script);
        assert_eq!(
            allowed(&decision),
            format!(
                "deep-diff-forge: UNJUDGED. This stop was not judged: the project this session began in, \"{}\", is no longer a Git working tree; start a new session to have the work there judged. That is not a pass.",
                recorded.display()
            )
        );
    }
    assert_eq!(runs(&log), 0, "another repository was judged in its place");
    // the remedy: a new session where the project is now has its work judged
    start(&scratch.join("put-away"), "s2");
    std::fs::write(
        scratch.join("put-away/new.rs"),
        "#[allow(unused)]\nfn n() {}\n",
    )
    .unwrap();
    let decision = stop_from(&scratch.join("put-away"), "s2", &script);
    assert!(
        blocked(&decision).contains("check_suppressed in \"a.rs\"") && runs(&log) == 1,
        "{decision}"
    );
}

/// Second independent review of the hooks, HR2-L-1 and HR2-M-2: the gate said every link was
/// ready with no usable state directory (every hook then says that it did not run) and with
/// only the stop hook registered (the judged range depends on the session-start hook, and a
/// commit is seen as it is made only by the hook after a tool call).
#[test]
fn doctor_gate_checks_the_state_directory() {
    let (scratch, root, home, script, _log, _exit_file) = gate_fixture("doctor-gate-state");
    let judge_arg = ["--judge", script.to_str().unwrap()];
    std::fs::create_dir(home.join(".claude")).unwrap();
    let user = home.join(".claude/settings.json");
    let registered: Value = serde_json::from_str(&stop_hook_settings(&script)).unwrap();
    std::fs::write(&user, registered.to_string()).unwrap();
    let gate = |shape: &dyn Fn(&mut Command)| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"));
        command
            .current_dir(&root)
            .envs(ISOLATE)
            .env("HOME", &home)
            .env_remove("XDG_STATE_HOME")
            .args(["agent", "doctor", "--gate", "--json"])
            .args(judge_arg);
        shape(&mut command);
        let out = command.output().unwrap();
        let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
        ((out.status.code().unwrap(), gate_checks(&doc)), doc)
    };
    let (found, doc) = gate(&|_| {});
    assert_eq!(found, (0, gate_but(&[])), "{doc:#}");
    assert_eq!(
        detail(&doc, "state_directory"),
        sessions().display().to_string()
    );
    assert_eq!(std::fs::read_dir(sessions()).unwrap().count(), 0);

    // no state directory at all: neither HOME nor XDG_STATE_HOME names one. The user's
    // settings are then not found either, so they are in the project for this run.
    std::fs::create_dir(root.join(".claude")).unwrap();
    let local = root.join(".claude/settings.local.json");
    std::fs::write(&local, registered.to_string()).unwrap();
    let (found, doc) = gate(&|command| {
        command.env_remove("HOME");
    });
    assert_eq!(found, (7, gate_but(&["state_directory"])), "{doc:#}");
    assert!(
        detail(&doc, "state_directory").contains("there is no state directory"),
        "{doc:#}"
    );
    // a state directory inside the project's working tree, one that is a link, and one that
    // cannot be written
    let (found, doc) = gate(&|command| {
        command.env("XDG_STATE_HOME", root.join(".state"));
    });
    assert_eq!(found, (7, gate_but(&["state_directory"])), "{doc:#}");
    assert!(
        detail(&doc, "state_directory").contains("lies inside the project's working tree"),
        "{doc:#}"
    );
    assert!(!root.join(".state").exists(), "the gate made the directory");
    let linked = scratch.join("linked-state");
    std::fs::create_dir_all(linked.join("deep-diff-forge")).unwrap();
    std::os::unix::fs::symlink(scratch.path(), linked.join("deep-diff-forge/hooks")).unwrap();
    let (found, doc) = gate(&|command| {
        command.env("XDG_STATE_HOME", &linked);
    });
    assert_eq!(found, (7, gate_but(&["state_directory"])), "{doc:#}");
    assert!(
        detail(&doc, "state_directory").contains("is not a directory of its own"),
        "{doc:#}"
    );
    std::fs::remove_file(&local).unwrap();
}

/// Second independent review of the hooks, HR2-M-2 and HR2-L-1: the gate looked for the stop
/// hook alone. The judged range depends on the session-start hook, and a commit is seen as it
/// is made only by the hook after a tool call; each is a link of its own now.
#[test]
fn doctor_gate_checks_that_the_other_two_hooks_are_registered() {
    let (_scratch, root, home, script, _log, _exit_file) = gate_fixture("doctor-gate-hooks");
    let judge_arg = ["--judge", script.to_str().unwrap()];
    std::fs::create_dir(home.join(".claude")).unwrap();
    let user = home.join(".claude/settings.json");
    let registered: Value = serde_json::from_str(&stop_hook_settings(&script)).unwrap();
    let gate = || {
        let (code, doc, _) = doctor_gate(&root, &home, &judge_arg);
        ((code, gate_checks(&doc)), doc)
    };

    // only the stop hook is registered (the reviewer's 10a), then each of the others alone
    let only = |events: &[&str]| {
        let mut settings = registered.clone();
        for event in ["SessionStart", "PostToolUse", "Stop"] {
            if !events.contains(&event) {
                settings["hooks"].as_object_mut().unwrap().remove(event);
            }
        }
        std::fs::write(&user, settings.to_string()).unwrap();
    };
    only(&["Stop"]);
    let (found, doc) = gate();
    assert_eq!(
        found,
        (
            7,
            gate_but(&["start_hook_registered", "advice_hook_registered"])
        ),
        "{doc:#}"
    );
    assert!(
        detail(&doc, "start_hook_registered")
            .contains("no Claude Code settings file registers a SessionStart hook of this program")
            && detail(&doc, "start_hook_registered")
                .contains("measured from its first tool call or stop, not from its start"),
        "{doc:#}"
    );
    only(&["Stop", "PostToolUse"]);
    assert_eq!(gate().0, (7, gate_but(&["start_hook_registered"])));
    only(&["Stop", "SessionStart"]);
    assert_eq!(gate().0, (7, gate_but(&["advice_hook_registered"])));
    // a hook of another program, and one a matcher narrows, are not it
    let mut settings = registered.clone();
    settings["hooks"]["SessionStart"][0]["hooks"][0]["command"] =
        json!("/bin/echo agent hook session-start --client claude");
    settings["hooks"]["PostToolUse"][0]["matcher"] = json!("Edit|Write");
    std::fs::write(&user, settings.to_string()).unwrap();
    let (found, doc) = gate();
    assert_eq!(
        found,
        (
            7,
            gate_but(&["start_hook_registered", "advice_hook_registered"])
        ),
        "{doc:#}"
    );
    assert!(
        detail(&doc, "start_hook_registered")
            .contains("does not run this program by its own absolute path")
            && detail(&doc, "advice_hook_registered").contains("has a matcher"),
        "{doc:#}"
    );
    // the settings `agent setup --hooks` prints register all three
    let out = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .env("PATH", script.parent().unwrap())
        .args(["agent", "setup", "--client", "claude", "--hooks", "--json"])
        .output()
        .unwrap();
    let printed: Value = serde_json::from_slice(&out.stdout).unwrap();
    for (event, word) in [
        ("SessionStart", "session-start"),
        ("PostToolUse", "post-tool"),
        ("Stop", "stop"),
    ] {
        assert!(
            printed["hooks"][event][0]["hooks"][0]["command"]
                .as_str()
                .unwrap()
                .contains(&format!(" agent hook {word} --client claude")),
            "{printed:#}"
        );
        assert!(printed["hooks"][event][0].get("matcher").is_none());
    }
}

/// Second independent review of the hooks, HR2-L-2, and the state directory inside the working
/// tree. When the state cannot be opened or kept, the advice hook said "the hook did not run"
/// after every tool call: the once-in-a-session rule needs the state. It says so once now,
/// remembered by an empty mark beside the scratch directory, and a stop still says it every
/// time. A state directory inside the project's working tree is refused: its files would be in
/// every patch, and cleaning the tree would remove the session's base.
#[test]
fn a_state_that_cannot_be_kept_is_said_once_by_advice_and_always_by_a_stop() {
    use std::os::unix::fs::PermissionsExt as _;
    let scratch = Scratch::new("hook-state-trouble");
    let root = repository(&scratch);
    silence_a_lint(&root);
    let own = root.join(".git/deep-diff-forge");
    let marks = || {
        std::fs::read_dir(&own).map_or(0, |entries| {
            entries
                .filter(|entry| {
                    entry
                        .as_ref()
                        .unwrap()
                        .file_name()
                        .to_string_lossy()
                        .starts_with("said-")
                })
                .count()
        })
    };
    let run = |word: &str, name: &str, session: &str, shape: &dyn Fn(&mut Command)| {
        let (code, stdout, stderr) =
            hook_in(word, &[], &input(name, &root, session, false), |command| {
                shape(command);
            });
        assert_eq!((code, stderr.as_str()), (0, ""), "{stdout}");
        stdout
    };

    // 6a: no state directory at all. Three tool calls: one message, then silence.
    let no_home = |command: &mut Command| {
        command.env_remove("HOME");
    };
    let said = system_message(&run("post-tool", "PostToolUse", "6a", &no_home));
    assert!(
        said.contains("the hook did not run: there is no state directory")
            && said.contains("Said once in this session"),
        "{said}"
    );
    for _ in 0..2 {
        assert_eq!(run("post-tool", "PostToolUse", "6a", &no_home), "");
    }
    assert_eq!(marks(), 1);
    // a stop says it every time
    for _ in 0..2 {
        assert!(
            system_message(&run("stop", "Stop", "6a", &no_home)).starts_with(
                "deep-diff-forge: UNJUDGED. The stop hook did not run: there is no state directory"
            )
        );
    }
    // another session is told once of its own
    assert_ne!(run("post-tool", "PostToolUse", "6a-other", &no_home), "");
    assert_eq!(run("post-tool", "PostToolUse", "6a-other", &no_home), "");

    // 6c: the state directory is made read-only after the start (not as root, who writes it).
    start(&root, "6c");
    let sessions = sessions();
    std::fs::set_permissions(&sessions, std::fs::Permissions::from_mode(0o500)).unwrap();
    if std::fs::write(sessions.join("probe"), "x").is_err() {
        let none = |_: &mut Command| {};
        let said = system_message(&run("post-tool", "PostToolUse", "6c", &none));
        assert!(
            said.contains("the hook did not run: its state could not be saved"),
            "{said}"
        );
        for _ in 0..2 {
            assert_eq!(run("post-tool", "PostToolUse", "6c", &none), "");
        }
    }
    std::fs::set_permissions(&sessions, std::fs::Permissions::from_mode(0o700)).unwrap();

    // The state directory lies inside the project's working tree.
    let inside = |command: &mut Command| {
        command.env("XDG_STATE_HOME", root.join(".state"));
    };
    let refused = "lies inside the project's working tree";
    let stop = system_message(&run("stop", "Stop", "in-tree", &inside));
    assert!(
        stop.starts_with(
            "deep-diff-forge: UNJUDGED. This stop was not judged: the state directory "
        ) && stop.contains(refused),
        "{stop}"
    );
    let started = run("session-start", "SessionStart", "in-tree", &inside);
    assert!(
        started.starts_with("deep-diff-forge: the stops of this session will not be judged")
            && started.contains(refused),
        "{started}"
    );
    assert!(system_message(&run("post-tool", "PostToolUse", "in-tree", &inside)).contains(refused));
    assert_eq!(run("post-tool", "PostToolUse", "in-tree", &inside), "");
    assert!(
        !root.join(".state").exists(),
        "a state was written into the working tree"
    );
    assert_eq!(git(&root, &["status", "--porcelain=v1"]), " M a.rs\n");
}

// ---- Round 11 of Vesper's review and the third independent review (2026-11-14, both NO-GO) ----

/// The first twelve digits of the commit `rev` names in `root`: how a stop names a commit.
fn short(root: &Path, rev: &str) -> String {
    git(root, &["rev-parse", rev])[..12].to_owned()
}

/// The uncommitted change of the working tree at `top` as a stop names it: the path quoted,
/// and cut at 120 characters as every path is.
fn uncommitted_in(top: &Path) -> String {
    let whole = top.display().to_string();
    let mut cut: String = whole.chars().take(120).collect();
    if cut.len() < whole.len() {
        cut.push('…');
    }
    format!("Uncommitted changes in \"{cut}\"")
}

/// The state files in `dir`, without the lists of commits that are kept beside them.
fn states_in(dir: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|kind| kind == "json"))
        .collect();
    found.sort();
    found
}

/// A stand-in judge that reads the patch it is shown: it fails a patch that adds a line with
/// `SECRET` in it and passes every other. It logs the files each patch names (`PATCH-NAMES`),
/// and first sleeps for the seconds written in `judge.sleep` when that file is there. The
/// other stand-in gives one answer whatever it is shown, so it cannot tell a stop that judged
/// the right commit from one that judged another.
fn reading_judge(scratch: &Scratch) -> (PathBuf, PathBuf) {
    const READ: &str = r#"printf 'PATCH-NAMES %s\n' "$(grep '^diff --git' "$patch" | tr '\n' ' ')" >> '@LOG@'
if [ -f '@SLEEP@' ]; then sleep "$(cat '@SLEEP@')"; fi
if grep -q '^+.*SECRET' "$patch"; then code=7; else code=0; fi"#;
    let (script, log, exit_file) = (
        scratch.join("judge"),
        scratch.join("judge.log"),
        scratch.join("judge.exit"),
    );
    let body = judge_script(
        &log,
        &exit_file,
        &verdict("fail", "no-secrets", "secret_like", "k.rs"),
    );
    let told = format!("code=\"$(cat '{}')\"", exit_file.display());
    assert!(body.contains(&told), "{body}");
    let read = READ.replace("@LOG@", &log.display().to_string()).replace(
        "@SLEEP@",
        &scratch.join("judge.sleep").display().to_string(),
    );
    write_program(&script, &body.replace(&told, &read));
    (script, log)
}

/// The files of each patch the reading judge was shown, in the order it was shown them.
fn shown(log: &Path) -> Vec<String> {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| line.strip_prefix("PATCH-NAMES "))
        .map(|names| {
            names
                .split(' ')
                .filter_map(|word| word.strip_prefix("b/"))
                .collect::<Vec<_>>()
                .join(",")
        })
        .collect()
}

/// Write a file the reading judge fails.
fn secret(root: &Path, name: &str) {
    std::fs::write(root.join(name), "const KEY: &str = \"SECRET\";\n").unwrap();
}

/// Commit everything in `root`, and give the commit as a stop names it.
fn commit_all(root: &Path, message: &str) -> String {
    git(root, &["add", "--all"]);
    git(root, &["commit", "--quiet", "-m", message]);
    short(root, "HEAD")
}

/// Review V4 round 11, VD4R11-H-1, as reproduced: a credential is committed and the commit is
/// reverted. The branch keeps the commit, the net patch from the session's base to the tip is
/// empty, and a stop that judged that net patch said nothing and never asked the judge. Each
/// commit of the session is judged by its own change now, once.
#[test]
fn a_credential_that_was_committed_and_then_reverted_is_judged() {
    let scratch = Scratch::new("hook-commit-revert");
    let root = repository(&scratch);
    let (script, log) = reading_judge(&scratch);
    start(&root, "s");
    secret(&root, "k.rs");
    let added = commit_all(&root, "add a key");
    git(&root, &["revert", "--no-edit", "HEAD"]);
    let reverted = short(&root, "HEAD");
    assert_eq!(git(&root, &["status", "--porcelain=v1"]), "");
    assert_eq!(git(&root, &["diff", "--stat", "HEAD~2", "HEAD"]), "");

    let decision = stop_from(&root, "s", &script);
    let reason = blocked(&decision);
    assert!(
        reason.contains(&format!("Commit {added}: FAIL\n- rule no-secrets: ")),
        "{reason}"
    );
    assert!(!reason.contains(&reverted), "{reason}");
    assert!(reason.contains("secret_like in \"k.rs\""), "{reason}");
    // the judge was shown each commit by its own change: the one that adds the file, and the
    // one that removes it
    assert_eq!(shown(&log), ["k.rs", "k.rs"]);
    // A commit is judged once in a session: its failure stands, by its name, and the judge is
    // not asked again.
    for _ in 0..2 {
        let again = stop_from(&root, "s", &script);
        let said = allowed(&again);
        assert!(
            said.starts_with("deep-diff-forge: FAIL STANDS. ")
                && said.contains(&format!("Commit {added}: FAIL, which stands\n")),
            "{said}"
        );
    }
    assert_eq!(runs(&log), 2);
    // another branch that names the same commit does not make it block again (HR3-L-1)
    git(&root, &["branch", "also", &added]);
    git(&root, &["checkout", "--quiet", "also"]);
    assert!(allowed(&stop_from(&root, "s", &script)).starts_with("deep-diff-forge: FAIL STANDS. "));
    assert_eq!(runs(&log), 2);
}

/// Third independent review of the hooks, HR3-M-1: a commit adds a credential and a second
/// commit removes the file. Nothing is left in the tree or in the net change, and the first
/// commit is in the branch's history all the same. It is judged.
#[test]
fn a_commit_that_a_later_commit_undoes_is_judged() {
    let scratch = Scratch::new("hook-commit-undone");
    let root = repository(&scratch);
    let (script, log) = reading_judge(&scratch);
    start(&root, "s");
    secret(&root, "k.rs");
    let added = commit_all(&root, "add a key");
    std::fs::remove_file(root.join("k.rs")).unwrap();
    let removed = commit_all(&root, "remove the key");
    let decision = stop_from(&root, "s", &script);
    let reason = blocked(&decision);
    assert!(
        reason.contains(&format!("Commit {added}: FAIL\n")) && !reason.contains(&removed),
        "{reason}"
    );
    assert_eq!(runs(&log), 2);
}

/// Third independent review of the hooks, HR3-H-1. `git stash -u` puts the untracked files in
/// a commit of their own, the third parent of the stash, which is in no patch from the base to
/// the stash; and an older entry of the stash is named by the stash's reflog alone. Both were
/// silent. Each is a commit the stash leads to, and is judged as one.
#[test]
fn a_stash_of_untracked_files_and_an_older_stash_entry_are_judged() {
    let scratch = Scratch::new("hook-stash-entries");
    let root = repository(&scratch);
    let (script, log) = reading_judge(&scratch);

    start(&root, "untracked");
    secret(&root, "k.rs");
    git(&root, &["stash", "--quiet", "--include-untracked"]);
    assert_eq!(git(&root, &["status", "--porcelain=v1"]), "");
    let untracked = short(&root, "refs/stash^3");
    let decision = stop_from(&root, "untracked", &script);
    assert!(
        blocked(&decision).contains(&format!("Commit {untracked}: FAIL\n")),
        "{decision}"
    );
    // one patch had anything in it: the commit of the untracked file
    assert_eq!(shown(&log), ["k.rs"]);
    git(&root, &["stash", "drop", "--quiet"]);

    start(&root, "older");
    std::fs::write(
        root.join("a.rs"),
        "fn a() {}\nfn b() {}\nconst KEY: &str = \"SECRET\";\n",
    )
    .unwrap();
    git(&root, &["stash", "--quiet"]);
    let older = short(&root, "refs/stash");
    std::fs::write(root.join("a.rs"), "fn a() {}\nfn b() {}\nfn c() {}\n").unwrap();
    git(&root, &["stash", "--quiet"]);
    let newer = short(&root, "refs/stash");
    assert_ne!(newer, older);
    let decision = stop_from(&root, "older", &script);
    let reason = blocked(&decision);
    assert!(
        reason.contains(&format!("Commit {older}: FAIL\n")) && !reason.contains(&newer),
        "{reason}"
    );
}

/// Third independent review of the hooks, HR3-L-6. A commit made on a detached `HEAD` that
/// was then left is named by the reflog of `HEAD` alone, and a commit on the detached `HEAD`
/// of another working tree by that tree's `HEAD` (or, once it was left, by that tree's
/// reflog). No ref names either, so no stop found them.
#[test]
fn a_commit_left_on_a_detached_head_or_made_in_another_working_tree_is_judged() {
    let scratch = Scratch::new("hook-detached");
    let root = repository(&scratch);
    let (script, _) = reading_judge(&scratch);
    let linked = scratch.join("linked");
    git(
        &root,
        &[
            "worktree",
            "add",
            "--quiet",
            "--detach",
            linked.to_str().unwrap(),
        ],
    );

    start(&root, "detached");
    git(&root, &["checkout", "--quiet", "--detach"]);
    secret(&root, "k.rs");
    let left = commit_all(&root, "on a detached HEAD");
    git(&root, &["checkout", "--quiet", "main"]);
    assert_eq!(git(&root, &["status", "--porcelain=v1"]), "");
    let decision = stop_from(&root, "detached", &script);
    assert!(
        blocked(&decision).contains(&format!("Commit {left}: FAIL\n")),
        "{decision}"
    );

    // in the other working tree, while the session stays in the project
    start(&root, "linked");
    secret(&linked, "k2.rs");
    let there = commit_all(&linked, "in the linked tree");
    let decision = stop_from(&root, "linked", &script);
    assert!(
        blocked(&decision).contains(&format!("Commit {there}: FAIL\n")),
        "{decision}"
    );

    // and one that the other tree has left again
    start(&root, "linked-left");
    secret(&linked, "k3.rs");
    let gone = commit_all(&linked, "in the linked tree, then left");
    git(&linked, &["checkout", "--quiet", "--detach", "main"]);
    let decision = stop_from(&root, "linked-left", &script);
    let reason = blocked(&decision);
    assert!(
        reason.contains(&format!("Commit {gone}: FAIL\n")),
        "{reason}"
    );
    // the commit before it was there when this session began: it is not this session's
    assert!(!reason.contains(&there), "{reason}");
}

/// Third independent review of the hooks, HR3-M-3. A failure was remembered for the whole
/// session, so one that was repaired and then brought back never blocked again. It is
/// forgotten at the stop that finds the change without it. And a failure that has blocked as
/// an uncommitted change does not block a second time when it is committed as it is: the
/// commit is said to stand.
#[test]
fn a_failure_that_was_repaired_and_comes_back_blocks_again() {
    let scratch = Scratch::new("hook-fail-returns");
    let root = repository(&scratch);
    let (script, _) = reading_judge(&scratch);
    let stands = |decision: &Value| {
        let said = allowed(decision);
        assert!(said.starts_with("deep-diff-forge: FAIL STANDS. "), "{said}");
    };
    start(&root, "s");
    secret(&root, "k.rs");
    blocked(&stop_from(&root, "s", &script));
    stands(&stop_from(&root, "s", &script));
    // repaired: the stop passes
    std::fs::remove_file(root.join("k.rs")).unwrap();
    assert_eq!(stop_from(&root, "s", &script), Value::Null);
    // the same failure again: it blocks again, once
    secret(&root, "k.rs");
    blocked(&stop_from(&root, "s", &script));
    stands(&stop_from(&root, "s", &script));
    // committed as it is: told already, so the commit stands, at this stop and at later ones
    let kept = commit_all(&root, "keep the key");
    for _ in 0..2 {
        let decision = stop_from(&root, "s", &script);
        stands(&decision);
        assert!(
            allowed(&decision).contains(&format!("Commit {kept}: FAIL, which stands\n")),
            "{decision}"
        );
    }
    // a commit that fails and was never told blocks
    secret(&root, "other.rs");
    let untold = commit_all(&root, "another key");
    let decision = stop_from(&root, "s", &script);
    let reason = blocked(&decision);
    assert!(
        reason.contains(&format!("Commit {untold}: FAIL\n"))
            && reason.contains(&format!("Commit {kept}: FAIL, which stands\n")),
        "{reason}"
    );
}

/// Third independent review of the hooks, HR3-H-2 and HR3-M-4. The commits that were there
/// when a session began are a file beside its state: ids only, with no bound a repository
/// reaches, named in the state by its SHA-256 and its length. A list that is gone, cut or
/// longer is not believed (a commit added to it would be one that "was there before"): the
/// stop is UNJUDGED and names the remedy. The repository's identity in the state is the inode
/// of its Git directory and no device number.
#[test]
fn a_list_of_commits_that_is_gone_or_was_changed_is_never_another_list() {
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
    let scratch = Scratch::new("hook-tips-file");
    let root = repository(&scratch);
    let (script, log) = reading_judge(&scratch);
    start(&root, "s");
    let state = states_in(&sessions());
    assert_eq!(state.len(), 1, "{state:?}");
    let tips = state[0].with_extension("tips");
    let recorded = std::fs::read_to_string(&tips).unwrap();
    assert_eq!(
        recorded,
        git(&root, &["rev-parse", "HEAD"]),
        "one commit was there: its id, and nothing else"
    );
    assert_eq!(
        std::fs::metadata(&tips).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let kept: Value = serde_json::from_str(&std::fs::read_to_string(&state[0]).unwrap()).unwrap();
    assert_eq!(kept["project"]["tips_count"], 1);
    assert_eq!(
        kept["project"]["id"],
        std::fs::metadata(root.join(".git"))
            .unwrap()
            .ino()
            .to_string()
    );

    secret(&root, "k.rs");
    commit_all(&root, "add a key");
    let head = git(&root, &["rev-parse", "HEAD"]);
    for (what, text) in [
        ("longer", Some(format!("{recorded}{head}"))),
        ("cut", Some(String::new())),
        ("gone", None),
    ] {
        match &text {
            Some(text) => std::fs::write(&tips, text).unwrap(),
            None => std::fs::remove_file(&tips).unwrap(),
        }
        let decision = stop_from(&root, "s", &script);
        let said = allowed(&decision);
        assert!(
            said.starts_with("deep-diff-forge: UNJUDGED. ")
                && said.contains("list of the commits that were there when it began")
                && said.contains("start a new session"),
            "{what}: {said}"
        );
    }
    assert_eq!(runs(&log), 0);
    // as it was recorded it is believed, and the commit is judged
    std::fs::write(&tips, recorded).unwrap();
    blocked(&stop_from(&root, "s", &script));
}

/// Third independent review of the hooks, HR3-H-2: the first 4,096 refs by name were
/// recorded, so in a repository with more the stash and the tags were not, and a stash or a
/// commit on a new branch was UNJUDGED or silent. Nothing is cut off now: with 5,000 branches
/// and tags the session is recorded, a stash is judged, and so is a commit on a new branch.
#[test]
fn a_repository_with_five_thousand_branches_and_tags_is_recorded_whole() {
    let scratch = Scratch::new("hook-many-refs");
    let root = repository(&scratch);
    let (script, _) = reading_judge(&scratch);
    let base = git(&root, &["rev-parse", "HEAD"]);
    let base = base.trim();
    // written straight into the packed refs, beside whatever Git has packed there itself
    let packed_refs = root.join(".git/packed-refs");
    let mut lines: Vec<String> = std::fs::read_to_string(&packed_refs)
        .unwrap_or_default()
        .lines()
        .filter(|line| !line.starts_with('#'))
        .map(str::to_owned)
        .collect();
    assert!(lines.iter().all(|line| !line.starts_with('^')), "{lines:?}");
    lines.extend((0..2_500).flat_map(|n| {
        [
            format!("{base} refs/heads/b{n:04}"),
            format!("{base} refs/tags/t{n:04}"),
        ]
    }));
    lines.sort_by(|one, other| one.split(' ').nth(1).cmp(&other.split(' ').nth(1)));
    std::fs::write(
        &packed_refs,
        format!(
            "# pack-refs with: peeled fully-peeled sorted \n{}\n",
            lines.join("\n")
        ),
    )
    .unwrap();
    assert_eq!(
        git(&root, &["for-each-ref", "--format=x"]).lines().count(),
        5_001
    );

    start(&root, "s");
    // a stash: `refs/stash` sorts after every branch
    std::fs::write(
        root.join("a.rs"),
        "fn a() {}\nfn b() {}\nconst KEY: &str = \"SECRET\";\n",
    )
    .unwrap();
    git(&root, &["stash", "--quiet"]);
    let stashed = short(&root, "refs/stash");
    let decision = stop_from(&root, "s", &script);
    assert!(
        blocked(&decision).contains(&format!("Commit {stashed}: FAIL\n")),
        "{decision}"
    );
    // a commit on a new branch whose name sorts after all of them
    git(&root, &["checkout", "--quiet", "-b", "zz-late"]);
    secret(&root, "k.rs");
    let late = commit_all(&root, "work");
    git(&root, &["checkout", "--quiet", "main"]);
    let decision = stop_from(&root, "s", &script);
    let reason = blocked(&decision);
    assert!(
        reason.contains(&format!("Commit {late}: FAIL\n"))
            && reason.contains(&format!("Commit {stashed}: FAIL, which stands\n"))
            && !reason.contains("UNJUDGED"),
        "{reason}"
    );
}

/// Third independent review of the hooks, HR3-M-2: of the tips that had changed, the first
/// sixteen by name were examined and the first four patches judged, so twenty branches whose
/// names sort before the session's own left its commit unjudged. There is no order by name
/// now, and no such bound: every commit is judged, and each once in a session.
#[test]
fn the_sessions_commit_is_judged_whatever_its_branch_is_called() {
    let scratch = Scratch::new("hook-branch-names");
    let root = repository(&scratch);
    let (script, log) = reading_judge(&scratch);
    let base = git(&root, &["rev-parse", "HEAD"]);
    let base = base.trim();
    start(&root, "s");
    for n in 0..20 {
        git(
            &root,
            &["checkout", "--quiet", "-b", &format!("aa{n:02}"), base],
        );
        std::fs::write(root.join(format!("f{n:02}.rs")), "fn f() {}\n").unwrap();
        commit_all(&root, "one harmless file");
    }
    git(&root, &["checkout", "--quiet", "-b", "zz-own", base]);
    secret(&root, "k.rs");
    let own = commit_all(&root, "the session's own work");
    git(&root, &["checkout", "--quiet", "main"]);

    let decision = stop_from(&root, "s", &script);
    let reason = blocked(&decision);
    assert!(
        reason.contains(&format!("Commit {own}: FAIL\n")) && !reason.contains("UNJUDGED"),
        "{reason}"
    );
    assert_eq!(runs(&log), 21);
    // nothing is judged twice
    assert!(allowed(&stop_from(&root, "s", &script)).starts_with("deep-diff-forge: FAIL STANDS. "));
    assert_eq!(runs(&log), 21);
}

/// The reviews' lists of weak tests: a merge base that is not the session's base. A commit on
/// an older branch, the commits a pull brings, and the commits a rebase writes are each judged
/// by their own change, against their own first parent; and patches that are the same are
/// judged once.
#[test]
fn a_commit_is_judged_by_its_own_change_on_an_older_branch_after_a_pull_and_a_rebase() {
    let scratch = Scratch::new("hook-own-change");
    let root = repository(&scratch);
    let (script, log) = reading_judge(&scratch);
    git(&root, &["branch", "old"]);
    std::fs::write(root.join("b.rs"), "fn c() {}\n").unwrap();
    commit_all(&root, "second");
    // somebody else's repository, with two commits to pull: a harmless one and one with a key
    let theirs = scratch.join("theirs");
    git(
        scratch.path(),
        &[
            "clone",
            "--quiet",
            "--no-local",
            root.to_str().unwrap(),
            theirs.to_str().unwrap(),
        ],
    );
    std::fs::write(theirs.join("h.rs"), "fn h() {}\n").unwrap();
    commit_all(&theirs, "theirs: harmless");
    secret(&theirs, "s.rs");
    let pulled = commit_all(&theirs, "theirs: a key");

    // an older branch: what lies between it and the session's base is not the commit's change
    start(&root, "old");
    git(&root, &["checkout", "--quiet", "old"]);
    secret(&root, "k.rs");
    let on_old = commit_all(&root, "on the older branch");
    git(&root, &["checkout", "--quiet", "main"]);
    let decision = stop_from(&root, "old", &script);
    assert!(
        blocked(&decision).contains(&format!("Commit {on_old}: FAIL\n")),
        "{decision}"
    );
    assert_eq!(shown(&log), ["k.rs"]);

    // a pull: the commits it brings are the session's from then on, each judged by itself
    start(&root, "pull");
    git(
        &root,
        &[
            "pull",
            "--quiet",
            "--ff-only",
            theirs.to_str().unwrap(),
            "main",
        ],
    );
    let decision = stop_from(&root, "pull", &script);
    assert!(
        blocked(&decision).contains(&format!("Commit {pulled}: FAIL\n")),
        "{decision}"
    );
    assert_eq!(shown(&log)[1..], ["h.rs", "s.rs"]);

    // a rebase: the commit it writes is a new commit. Here it has the same patch as the one
    // it was written from, which the reflog still names: the two are judged once, and the
    // commit on `main` by itself.
    start(&root, "rebase");
    git(&root, &["checkout", "--quiet", "-b", "topic"]);
    std::fs::write(root.join("t.rs"), "fn t() {}\n").unwrap();
    commit_all(&root, "topic work");
    git(&root, &["checkout", "--quiet", "main"]);
    std::fs::write(root.join("m.rs"), "fn m() {}\n").unwrap();
    commit_all(&root, "main moves on");
    git(&root, &["rebase", "--quiet", "main", "topic"]);
    let before = runs(&log);
    assert_eq!(stop_from(&root, "rebase", &script), Value::Null);
    let mut seen = shown(&log)[before..].to_vec();
    seen.sort();
    assert_eq!(seen, ["m.rs", "t.rs"]);
    // all three are remembered: the next stop asks the judge nothing
    assert_eq!(stop_from(&root, "rebase", &script), Value::Null);
    assert_eq!(runs(&log), before + 2);
}

/// The reviews' lists of weak tests: the order in which a stop judges when not everything
/// fits its time. The commits are judged the oldest first; those that did not fit are counted
/// and said to be UNJUDGED, never passed over, and are judged at the next stop, where no
/// commit that already has a verdict is judged again.
///
/// This test takes about forty seconds: it runs one stop up to its deadline, with a judge
/// that takes thirteen seconds for each patch.
#[test]
fn commits_are_judged_the_oldest_first_and_the_rest_at_the_next_stop() {
    let scratch = Scratch::new("hook-commit-order");
    let root = repository(&scratch);
    let (script, log) = reading_judge(&scratch);
    start(&root, "s");
    let names: Vec<String> = (1..=5).map(|n| format!("f{n}.rs")).collect();
    for name in &names {
        std::fs::write(root.join(name), "fn f() {}\n").unwrap();
        commit_all(&root, name);
    }
    std::fs::write(scratch.join("judge.sleep"), "13").unwrap();
    let decision = stop_from(&root, "s", &script);
    let said = allowed(&decision);
    let first = shown(&log);
    assert!(
        !first.is_empty() && first.len() < names.len(),
        "{first:?}: {said}"
    );
    assert_eq!(first[..], names[..first.len()], "not the oldest first");
    assert!(
        said.starts_with("deep-diff-forge: UNJUDGED. Not everything of this change (")
            && said.contains(" of this session: UNJUDGED\n")
            && said.contains("they are judged at the next stop")
            && said.ends_with("That is not a pass."),
        "{said}"
    );

    // the next stop, with a judge that answers at once: the rest, the oldest first
    std::fs::remove_file(scratch.join("judge.sleep")).unwrap();
    assert_eq!(stop_from(&root, "s", &script), Value::Null);
    let all = shown(&log);
    let rest = &all[first.len()..];
    assert!(
        !rest.is_empty() && rest.windows(2).all(|pair| pair[0] < pair[1]),
        "{all:?}"
    );
    assert_eq!(rest.last(), names.last(), "{all:?}");
    for name in &names {
        assert!(all.contains(name), "{name} was never judged: {all:?}");
    }
    // every commit has its verdict: a third stop asks nothing
    let before = runs(&log);
    assert_eq!(stop_from(&root, "s", &script), Value::Null);
    assert_eq!(runs(&log), before);
}

/// Third independent review of the hooks, HR3-L-2: a session remembers 32 things as said
/// once. Past that a stop named nothing more beside its verdict, and did not say so. It says
/// once that there is more.
#[test]
fn more_things_beside_a_verdict_than_a_session_remembers_are_said_to_be_more() {
    let scratch = Scratch::new("hook-many-notes");
    let root = repository(&scratch);
    let (script, _) = reading_judge(&scratch);
    start(&root, "s");
    for n in 0..33 {
        let nested = root.join(format!("n{n:02}"));
        std::fs::create_dir(&nested).unwrap();
        git(&nested, &["init", "--quiet"]);
        std::fs::write(nested.join("x"), "x\n").unwrap();
    }
    let decision = stop_from(&root, "s", &script);
    let said = allowed(&decision);
    assert_eq!(
        said.matches("is a Git repository of its own").count(),
        32,
        "{said}"
    );
    assert!(
        said.ends_with(
            "More things were left out or not judged than this session's hooks name one by one: the others are not named from here on."
        ),
        "{said}"
    );
    assert_eq!(stop_from(&root, "s", &script), Value::Null);
}

/// Third independent review of the hooks, HR3-L-3: another working tree of the project that
/// lies inside its tree was called "a Git repository of its own". It is named as what it is;
/// a commit made in it is the session's and is judged from the project's own directory, and
/// its uncommitted change when the session is in it.
#[test]
fn a_linked_working_tree_inside_the_project_is_named_as_what_it_is() {
    let scratch = Scratch::new("hook-inner-worktree");
    let root = repository(&scratch);
    let (script, _) = reading_judge(&scratch);
    let inner = root.join("inner");
    git(
        &root,
        &[
            "worktree",
            "add",
            "--quiet",
            "--detach",
            inner.to_str().unwrap(),
        ],
    );
    start(&root, "s");
    let decision = stop_from(&root, "s", &script);
    let said = allowed(&decision);
    assert!(
        said.contains("The directory \"inner\" in ")
            && said.contains("is another working tree of this project (a linked worktree)")
            && !said.contains("a Git repository of its own"),
        "{said}"
    );
    secret(&inner, "k.rs");
    let made = commit_all(&inner, "in the inner tree");
    let decision = stop_from(&root, "s", &script);
    assert!(
        blocked(&decision).contains(&format!("Commit {made}: FAIL\n")),
        "{decision}"
    );
    secret(&inner, "k2.rs");
    let decision = stop_from(&inner, "s", &script);
    let reason = blocked(&decision);
    assert!(
        reason.contains(&format!(
            "{}: FAIL\n",
            uncommitted_in(&std::fs::canonicalize(&inner).unwrap())
        )) && reason.contains(&format!("Commit {made}: FAIL, which stands\n")),
        "{reason}"
    );
}

// ---- A mutation run on the hooks (2026-11-14): changes no test noticed ----

/// A file where the directory of the sessions' state should be is said as what it is. The
/// check that says so was alive when taken out: the state file under it then "cannot be
/// opened", which is another failure with other words, and no test read the words.
#[test]
fn a_file_where_the_sessions_directory_should_be_is_said_as_what_it_is() {
    const NOT_ITS_OWN: &str =
        "is not a directory of its own (a symbolic link or a file stands there)";
    let scratch = Scratch::new("hook-state-dir-file");
    let root = repository(&scratch);
    let (script, log, _) = failing_judge(&scratch);
    silence_a_lint(&root);
    let sessions = sessions();
    std::fs::create_dir_all(sessions.parent().unwrap()).unwrap();
    std::fs::write(&sessions, "not a directory\n").unwrap();
    let (code, stdout, _) = hook("post-tool", &[], &input("PostToolUse", &root, "s", false));
    assert_eq!(code, 0);
    let said = system_message(&stdout);
    assert!(said.contains(NOT_ITS_OWN), "{said}");
    let decision = stop_from(&root, "s", &script);
    let said = allowed(&decision);
    assert!(
        said.starts_with("deep-diff-forge: UNJUDGED. ") && said.contains(NOT_ITS_OWN),
        "{said}"
    );
    assert_eq!(runs(&log), 0);
    assert_eq!(
        std::fs::read_to_string(&sessions).unwrap(),
        "not a directory\n"
    );
}

/// A hook that changes nothing in its session's state writes nothing: with the state's
/// directory read-only, a stop with nothing to judge and a hook after a tool call with
/// nothing new stay silent. (The test of "was anything changed" was alive when it always
/// said yes: the state was written again each time, and nothing looked.)
#[test]
fn a_hook_that_changes_nothing_in_its_state_writes_nothing() {
    use std::os::unix::fs::PermissionsExt as _;
    let scratch = Scratch::new("hook-state-unchanged");
    let root = repository(&scratch);
    let (script, _, _) = failing_judge(&scratch);
    start(&root, "s");
    let sessions = sessions();
    std::fs::set_permissions(&sessions, std::fs::Permissions::from_mode(0o500)).unwrap();
    // not as root, who writes there all the same
    if std::fs::write(sessions.join("probe"), "x").is_err() {
        assert_eq!(stop_from(&root, "s", &script), Value::Null);
        let event = input("PostToolUse", &root, "s", false);
        assert_eq!(
            hook("post-tool", &[], &event),
            (0, String::new(), String::new())
        );
    }
    std::fs::set_permissions(&sessions, std::fs::Permissions::from_mode(0o700)).unwrap();
}

/// What `agent doctor --gate` says in four places no test read, and one path no test took: a
/// timeout that is long enough; a start hook that names a judge; the other two hooks outside a
/// repository; the hooks' own directory when it cannot be used; and a judge that refuses this
/// binary as a ranker and admits another.
#[test]
fn doctor_gate_says_what_it_found_where_no_test_read_it() {
    let (scratch, root, home, script, _log, _exit_file) = gate_fixture("doctor-gate-words");
    let judge_arg = ["--judge", script.to_str().unwrap()];
    std::fs::create_dir(home.join(".claude")).unwrap();
    let user = home.join(".claude/settings.json");
    let registered: Value = serde_json::from_str(&stop_hook_settings(&script)).unwrap();
    let gate = |root: &Path, extra: &[&str]| {
        let (code, doc, _) = doctor_gate(root, &home, extra);
        ((code, gate_checks(&doc)), doc)
    };
    let mut settings = registered.clone();
    settings["hooks"]["Stop"][0]["hooks"][0]["timeout"] = json!(60);
    std::fs::write(&user, settings.to_string()).unwrap();
    let (found, doc) = gate(&root, &judge_arg);
    assert_eq!(found, (0, gate_but(&[])), "{doc:#}");
    assert_eq!(
        detail(&doc, "hook_timeout"),
        "the stop hook's timeout is 60 seconds (50 or more are needed)"
    );

    // a start hook that names a judge is held to it: the one that was checked, or another
    let start = |judge: &Path| {
        json!(format!(
            "'{}' agent hook session-start --client claude --judge '{}'",
            this_binary().display(),
            judge.display()
        ))
    };
    settings["hooks"]["SessionStart"][0]["hooks"][0]["command"] =
        start(&std::fs::canonicalize(&script).unwrap());
    std::fs::write(&user, settings.to_string()).unwrap();
    let (found, doc) = gate(&root, &judge_arg);
    assert_eq!(found, (0, gate_but(&[])), "{doc:#}");
    settings["hooks"]["SessionStart"][0]["hooks"][0]["command"] = start(Path::new("/usr/bin/true"));
    std::fs::write(&user, settings.to_string()).unwrap();
    let (found, doc) = gate(&root, &judge_arg);
    assert_eq!(found, (7, gate_but(&["start_hook_registered"])), "{doc:#}");
    assert!(
        detail(&doc, "start_hook_registered")
            .contains("names another judge than the one checked here"),
        "{doc:#}"
    );
    std::fs::write(&user, registered.to_string()).unwrap();

    // outside a repository the other two hooks are not checked, and are said so in those words
    let outside = scratch.join("outside");
    std::fs::create_dir(&outside).unwrap();
    let (_, doc) = gate(&outside, &judge_arg);
    for name in ["start_hook_registered", "advice_hook_registered"] {
        assert_eq!(detail(&doc, name), "not checked: it needs the repository");
    }

    // the hooks' own directory in the repository cannot be used: the patch and the chain say why
    let own = root.join(".git/deep-diff-forge");
    std::fs::remove_dir(&own).unwrap();
    std::fs::write(&own, "a file\n").unwrap();
    let (found, doc) = gate(&root, &judge_arg);
    assert_eq!(found.0, 7, "{doc:#}");
    for name in ["patch", "chain"] {
        assert!(
            detail(&doc, name).contains("is not a directory of its own"),
            "{name}: {doc:#}"
        );
    }
    std::fs::remove_file(&own).unwrap();

    // the judge refuses this binary as a ranker (exit 4) and admits another: the chain is ready
    let rankers = scratch.join("rankers");
    std::fs::create_dir(&rankers).unwrap();
    std::fs::write(rankers.join("ddf-0.8.4"), "stand-in ranker\n").unwrap();
    let picky = scratch.join("admits-one-ranker");
    write_program(
        &picky,
        &format!(
            "#!/bin/sh\ncase \"$*\" in *ddf-0.8.4*) exec '{}' \"$@\";; *) exit 4;; esac\n",
            script.display()
        ),
    );
    let (_, doc) = gate(
        &root,
        &[
            "--judge",
            picky.to_str().unwrap(),
            "--ranker-dir",
            rankers.to_str().unwrap(),
        ],
    );
    let chain = gate_checks(&doc)
        .into_iter()
        .find(|(name, _)| name == "chain");
    assert_eq!(chain, Some(("chain".to_owned(), true)), "{doc:#}");
}
