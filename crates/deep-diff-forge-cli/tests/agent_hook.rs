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
            && message.contains("That is not a pass."),
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
            "your uncommitted and staged changes, and each commit made since this session began (commit {}) that your branches, tags, stash, HEAD or their history name, are judged",
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

const GATE: [&str; 14] = [
    "git",
    "repository",
    "policy",
    "judge",
    "chain",
    "policy_protects_itself",
    "patch",
    "commits",
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

/// The fourteen gate checks, in order, every one ready but those named in `not`.
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
                "commits",
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
/// What every stop after the first says of a session that no session-start hook recorded.
const START_NOT_RECORDED_AGAIN: &str = "deep-diff-forge: 1 thing in this project is not judged: what was committed before this session's first hook.";

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
    // (the session had no session-start hook, which every stop of it says)
    assert_eq!(
        system_message(&hook_on_path("stop", &[&bin], &event).1),
        START_NOT_RECORDED_AGAIN
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
/// (From inside the Git directory, which Git cannot place: `gits_words_are_never_cut_away`.)
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
    assert_eq!(seen.matches("ARGS check ").count(), 10, "{seen}");
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
    // Round 14 (HR5-M-3): a failure is its unit's, so the linked tree's blocks for itself
    let said = blocked(&decision);
    assert!(
        said.contains(&tree(&top, "FAIL, which stands")) && said.contains(&tree(&linked, "FAIL")),
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
    // Round 15 (HR6-M-1): the index of the new branch holds another change than the tree,
    // and the failure in it is the tree's, which has blocked: it stands, and does not block
    // for the index again
    let decision = stop_from(&root, "s", &script);
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
    // nothing to judge: there is no commit. That its files are not judged is said at every
    // stop, in full once and then in one line (round 14: what is known in the project and
    // not judged is never silence); a repository with no commit and no file says nothing.
    let said = allowed(&stop_from(&root, "s", &script)).to_owned();
    assert!(
        said.starts_with("deep-diff-forge: The repository \"")
            && said.contains("has no commit yet, so there is no commit to measure this session's changes from: its files were not judged."),
        "{said}"
    );
    for _ in 0..2 {
        assert_eq!(
            allowed(&stop_from(&root, "s", &script)),
            "deep-diff-forge: 1 thing in this project is not judged: the files of this repository, which has no commit yet."
        );
    }
    let empty = scratch.join("empty");
    std::fs::create_dir(&empty).unwrap();
    git(&empty, &["init", "--quiet"]);
    start(&empty, "e");
    assert_eq!(stop_from(&empty, "e", &script), Value::Null);
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
    // Round 16 (HR7-M-1): two of the items are repaired. What is left has blocked and is as
    // it was, so it stands; it blocked again, as "another failure". The lower count is what
    // is remembered: when the items come back, that is new, and blocks once.
    write_program(&script, &judge_script(&log, &exit_file, &citing(1)));
    assert!(allowed(&stop(false)).starts_with("deep-diff-forge: FAIL STANDS. "));
    write_program(&script, &judge_script(&log, &exit_file, &citing(3)));
    blocked(&stop(false));
    assert!(allowed(&stop(false)).starts_with("deep-diff-forge: FAIL STANDS. "));

    // The state cannot be kept: its directory can be read and not written (not as root, who
    // writes it all the same). A failure that has not blocked before then blocks at every
    // stop, and each time the stop says why.
    let sessions = sessions();
    std::fs::set_permissions(&sessions, std::fs::Permissions::from_mode(0o500)).unwrap();
    if std::fs::write(sessions.join("probe"), "x").is_err() {
        write_program(&script, &judge_script(&log, &exit_file, &citing(5)));
        let not_kept = "The hook could not keep its state (its state could not be saved: ";
        // at every new turn it blocks, and says why it will again
        for _ in 0..2 {
            let decision = stop(false);
            let reason = blocked(&decision);
            assert!(
                reason.contains(not_kept) && reason.contains("the same failure can block again"),
                "{reason}"
            );
        }
        // Round 13 (HR4-M-3): when the client says a stop hook has just blocked, a hook that
        // cannot remember does not block twice running
        let decision = stop(true);
        let said = allowed(&decision);
        assert!(
            said.starts_with(
                "deep-diff-forge: FAIL STANDS. This failure is not blocked because the client says"
            ) && said.contains(not_kept)
                && said.contains("check_suppressed in \"a.rs\""),
            "{said}"
        );
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
    // three files: the state, the list of commits that were there when the session began,
    // and the session's lock, which a hook holds while it writes
    assert_eq!(files(&xdg.join("deep-diff-forge/hooks")), 3);
    assert_eq!(files(&sessions()), 0);
    let (_, stdout, _) = hook_in("post-tool", &[], &event, |command| {
        command
            .env("XDG_STATE_HOME", "relative/state")
            .current_dir(&root);
    });
    assert!(stdout.contains("check_suppressed"), "{stdout}");
    assert_eq!(files(&sessions()), 3);
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
    // cannot be known, so it is said: in full once, and in one line at every later stop of
    // the session, for it stays true (round 14: what is true at a stop is said at that stop).
    let said_again = |stdout: &str| assert_eq!(system_message(stdout), START_NOT_RECORDED_AGAIN);
    only_the_note(&stop("clean", &root, None));
    said_again(&stop("clean", &root, None));
    // The reviewer's T2a: the work is committed just before the stop.
    silence_a_lint(&root);
    git(&root, &["commit", "--quiet", "-am", "work"]);
    only_the_note(&stop("committed", &root, None));
    said_again(&stop("committed", &root, None));
    // The same when the client names the project and the session is somewhere else.
    only_the_note(&stop("named", &outside, Some(&root)));
    said_again(&stop("named", &outside, Some(&root)));
    assert!(!log.exists(), "the judge was run with nothing to judge");
    // A session that started is not told this: its clean stop says nothing.
    start(&root, "started");
    assert_eq!(stop("started", &root, None), "");

    // With something left in the tree the judge decides, and the same sentence is added.
    std::fs::write(root.join("new.rs"), "#[allow(unused)]\nfn n() {}\n").unwrap();
    std::fs::write(&exit_file, "0").unwrap();
    only_the_note(&stop("passes", &root, None));
    said_again(&stop("passes", &root, None));
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
    said_again(&stop("unborn", &unborn, None));
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
    // Not the message of the commit above: the same tree, parent, author and message within
    // one second is the same commit, which the reflog held before this session began, and a
    // commit that was in the repository before the session is not the session's (seen once,
    // with a release build, as a stop that said nothing).
    git(&root, &["commit", "--quiet", "-am", "usual work"]);
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
    // not silent, and not a pass. Round 13 (HR4-L-4): the commit blocks at the stop that
    // first judges it; then it stands
    blocked(&stop_from(&root, "s", &script));
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
    // in full once; in one line at every later stop
    assert_eq!(
        allowed(&stop_from(&root, "2e", &script)),
        START_NOT_RECORDED_AGAIN
    );

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
/// project's patch shows a submodule only as the commit it is at). Each is named now: in
/// full once, and in one line at every later stop while it is true (round 14).
#[test]
fn another_repository_and_a_submodule_with_changes_are_named_at_every_stop() {
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
    // in full once, then in one line while the session is there; from the project itself
    // there is nothing to say
    assert!(
        allowed(&stop_from(&other, "3a", &script))
            .starts_with("deep-diff-forge: 1 thing in this project is not judged: the repository ")
    );
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
    assert_eq!(
        allowed(&stop_from(&root, "3c", &script)),
        "deep-diff-forge: 1 thing in this project is not judged: the submodule \"vendor/sub\"."
    );
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
    assert!(
        allowed(&stop_from(&inside, "3c-inside", &script))
            .starts_with("deep-diff-forge: 2 things in this project are not judged: ")
    );
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
    // Round 13 (HR4-L-4): the commit blocks once, at the stop that first judges it; at turn 2
    // and turn 3, with nothing changed, there is no block again
    blocked(&stop(false));
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
/// and first sleeps for the seconds written in `judge.sleep` when that file is there. While
/// `judge.flaky` is there it gives no verdict on a patch with `SECRET`: as many times as the
/// file says (once when it is empty), then the file is gone. The
/// other stand-in gives one answer whatever it is shown, so it cannot tell a stop that judged
/// the right commit from one that judged another.
fn reading_judge(scratch: &Scratch) -> (PathBuf, PathBuf) {
    const READ: &str = r#"printf 'PATCH-NAMES %s\n' "$(grep '^diff --git' "$patch" | tr '\n' ' ')" >> '@LOG@'
if [ -f '@SLEEP@' ]; then sleep "$(cat '@SLEEP@')"; fi
if [ -f '@FLAKY@' ] && grep -q '^+.*SECRET' "$patch"; then left="$(cat '@FLAKY@')"; left=$((${left:-1} - 1)); if [ "$left" -le 0 ]; then rm -f '@FLAKY@'; else echo "$left" > '@FLAKY@'; fi; echo '{"exit":3,"name":"io","message":"the ranker did not answer in time","next":"retry"}' >&2; exit 3; fi
if grep -q '^+.*NOVERDICT' "$patch"; then echo '{"exit":3,"name":"io","message":"patch parse failed","next":"fix_input"}' >&2; exit 3; fi
if grep -q '^+.*SECRET' "$patch"; then code=7; elif grep -q '^+.*HOLDME' "$patch"; then code=8; else code=0; fi"#;
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
    let read = READ
        .replace("@LOG@", &log.display().to_string())
        .replace(
            "@SLEEP@",
            &scratch.join("judge.sleep").display().to_string(),
        )
        .replace(
            "@FLAKY@",
            &scratch.join("judge.flaky").display().to_string(),
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
    // Committed as it is. Round 13 (HR4-L-4): a failing commit blocks at the stop that first
    // judges it, whatever has blocked before as an uncommitted change (the match was by rule,
    // signal, path and count, which another credential in the same file meets too). After
    // that it stands.
    let kept = commit_all(&root, "keep the key");
    let reason = blocked(&stop_from(&root, "s", &script)).to_owned();
    assert!(
        reason.contains(&format!("Commit {kept}: FAIL\n")),
        "{reason}"
    );
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
/// once. Past that a stop named nothing more beside its verdict, and did not say so. Since
/// round 14 everything that is not said in full is in the line that counts such things.
#[test]
fn more_things_beside_a_verdict_than_a_stop_says_in_full_are_counted() {
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
    // Round 14: no list that ends in silence. Five are said in full at a stop, the others
    // are counted on one line; at every stop all thirty-three are in the one or the other.
    for stop in 0..8 {
        let decision = stop_from(&root, "s", &script);
        let said = allowed(&decision);
        let full = said.matches("is a Git repository of its own").count();
        // (a session remembers thirty-two things: the thirty-third is counted from then on)
        assert_eq!(
            full,
            32_usize.saturating_sub(5 * stop).min(5),
            "stop {stop}"
        );
        let counted = 33 - full;
        assert!(
            said.contains(&format!(
                "{counted} things in this project are not judged: "
            )) && said.ends_with(&format!(", and {} more.", counted - 3)),
            "stop {stop}: {said}"
        );
    }
}

/// Third independent review of the hooks, HR3-L-3: another working tree of the project that
/// lies inside its tree was called "a Git repository of its own". It is a working tree of
/// the project: nothing is said of it while nothing in it is uncommitted (it was then called
/// "not judged" at every stop: seventh independent review, HR7-L-5); a commit made in it is
/// the session's and is judged from the project's own directory, and its uncommitted change
/// when the session is in it.
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
    assert_eq!(stop_from(&root, "s", &script), Value::Null);
    assert_eq!(stop_from(&inner, "s", &script), Value::Null);
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

// ---- The fourth independent review of the hooks (2026-11-14, NO-GO, narrower) ----

/// A commit whose patch the reading judge gives no verdict for: it adds a line with `NOVERDICT`.
fn unjudgeable(root: &Path, name: &str) -> String {
    std::fs::write(root.join(name), "// NOVERDICT\n").unwrap();
    commit_all(root, name)
}

/// A stop of `session` with the client's `stop_hook_active` as given.
fn stop_active(root: &Path, session: &str, judge: &Path, active: bool) -> Value {
    let (code, stdout, stderr) = hook(
        "stop",
        &["--judge", judge.to_str().unwrap()],
        &input("Stop", root, session, active),
    );
    assert_eq!((code, stderr.as_str()), (0, ""), "{stdout}");
    serde_json::from_str(&stdout).unwrap_or(Value::Null)
}

/// A commit that adds one file with a credential, made with plumbing: it is never on a `HEAD`
/// and no reflog names it. Its parent is the commit `HEAD` names.
fn plumbing_commit(root: &Path, name: &str) -> String {
    let out = Command::new("sh")
        .current_dir(root)
        .envs(ISOLATE)
        .envs([
            ("GIT_AUTHOR_NAME", "ddf"),
            ("GIT_AUTHOR_EMAIL", "ddf@example.invalid"),
            ("GIT_COMMITTER_NAME", "ddf"),
            ("GIT_COMMITTER_EMAIL", "ddf@example.invalid"),
        ])
        .args([
            "-c",
            "blob=$(printf 'const KEY: &str = \"SECRET\";\\n' | git hash-object -w --stdin) && tree=$(printf '100644 blob %s\\t%s\\n' \"$blob\" \"$1\" | git mktree) && git commit-tree \"$tree\" -p HEAD -m plumbing",
            "sh",
            name,
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

/// Fourth independent review of the hooks, HR4-H-1, the reviewer's I1 and I2. A credential is
/// staged, the file is repaired and not staged again: the index holds what the working tree
/// no longer does, and the next plain `git commit` records it. The stop was silent. What is
/// staged is a unit now, judged when it is another change than the working tree's.
#[test]
fn what_is_staged_and_no_longer_in_the_working_tree_is_judged() {
    let scratch = Scratch::new("hook-staged");
    let root = repository(&scratch);
    let (script, log) = reading_judge(&scratch);
    let top = std::fs::canonicalize(&root).unwrap();
    let staged = uncommitted_in(&top).replace("Uncommitted changes", "Staged changes");
    let with_key = "fn a() {}\nfn b() {}\nconst KEY: &str = \"SECRET\";\n";

    // the ordinary case: everything staged, nothing changed since: one patch, one run
    start(&root, "all-staged");
    std::fs::write(root.join("a.rs"), "fn a() {}\nfn b() {}\nfn c() {}\n").unwrap();
    git(&root, &["add", "--all"]);
    assert_eq!(stop_from(&root, "all-staged", &script), Value::Null);
    assert_eq!(runs(&log), 1);

    // I1: staged with a credential, repaired in the working tree, not staged again
    start(&root, "i1");
    std::fs::write(root.join("a.rs"), with_key).unwrap();
    git(&root, &["add", "--all"]);
    std::fs::write(root.join("a.rs"), "fn a() {}\nfn b() {}\nfn d() {}\n").unwrap();
    assert_eq!(git(&root, &["status", "--porcelain=v1"]), "MM a.rs\n");
    let decision = stop_from(&root, "i1", &script);
    let reason = blocked(&decision);
    assert!(
        reason.contains(&format!("{staged}: FAIL\n- rule no-secrets: "))
            && !reason.contains("Uncommitted changes in"),
        "{reason}"
    );
    // it stands while it is there, and is forgotten when the index is put right
    assert!(
        allowed(&stop_from(&root, "i1", &script))
            .contains(&format!("{staged}: FAIL, which stands\n"))
    );
    git(&root, &["add", "--all"]);
    assert_eq!(stop_from(&root, "i1", &script), Value::Null);

    // I2: a new file with a credential is staged, then deleted from the working tree
    start(&root, "i2");
    secret(&root, "k.rs");
    git(&root, &["add", "k.rs"]);
    std::fs::remove_file(root.join("k.rs")).unwrap();
    let decision = stop_from(&root, "i2", &script);
    assert!(
        blocked(&decision).contains(&format!("{staged}: FAIL\n")),
        "{decision}"
    );
    git(&root, &["reset", "--quiet", "--hard"]);

    // the same in a linked working tree the session is in
    let linked = scratch.join("linked");
    let path = linked.to_str().unwrap();
    git(&root, &["worktree", "add", "--quiet", "-b", "side", path]);
    start(&root, "linked");
    std::fs::write(linked.join("a.rs"), with_key).unwrap();
    git(&linked, &["add", "--all"]);
    std::fs::write(linked.join("a.rs"), "fn a() {}\nfn b() {}\n").unwrap();
    let decision = stop_from(&linked, "linked", &script);
    let reason = blocked(&decision);
    let there = uncommitted_in(&std::fs::canonicalize(&linked).unwrap())
        .replace("Uncommitted changes", "Staged changes");
    assert!(reason.contains(&format!("{there}: FAIL\n")), "{reason}");
}

/// Fourth independent review of the hooks, HR4-H-2, the reviewer's `t6c` and `t6b`. With
/// thirteen failures standing, a commit that got no verdict was not said at all, and a commit
/// that failed blocked the stop without being named or its rule shown: the twelve units
/// listed were the oldest. What blocks now is said first, then what got no verdict, and what
/// stands is a count.
#[test]
fn a_stop_names_what_blocks_now_however_many_failures_stand() {
    let scratch = Scratch::new("hook-many-standing");
    let root = repository(&scratch);
    let (script, _) = reading_judge(&scratch);
    start(&root, "s");
    for n in 0..13 {
        secret(&root, &format!("k{n:02}.rs"));
        commit_all(&root, "a key");
    }
    let reason = blocked(&stop_from(&root, "s", &script)).to_owned();
    assert_eq!(
        reason.matches(": FAIL\n- rule no-secrets: ").count(),
        13,
        "{reason}"
    );
    let stands = allowed(&stop_from(&root, "s", &script)).to_owned();
    assert!(
        stands.starts_with("deep-diff-forge: FAIL STANDS. ")
            && stands.contains("13 earlier failures stand: Commit ")
            && stands.contains(", and 10 more.\n"),
        "{stands}"
    );
    // a commit that gets no verdict: said, by its id, at the head of what the stop says
    let odd = unjudgeable(&root, "odd.rs");
    let said = allowed(&stop_from(&root, "s", &script)).to_owned();
    assert!(
        said.starts_with("deep-diff-forge: UNJUDGED. ")
            && said.contains(&format!("1 commit of this session has no verdict: {odd} ("))
            && said.contains("13 earlier failures stand: "),
        "{said}"
    );
    // a commit that fails: it blocks, and the reason names it and its rule before all else
    secret(&root, "own.rs");
    let own = commit_all(&root, "the session's own key");
    let reason = blocked(&stop_from(&root, "s", &script)).to_owned();
    let named = reason
        .find(&format!("Commit {own}: FAIL\n- rule no-secrets: "))
        .unwrap_or_else(|| panic!("the commit that blocks is not named: {reason}"));
    let standing = reason.find("13 earlier failures stand: ").unwrap();
    assert!(named < standing && reason.contains(&odd), "{reason}");
    assert!(allowed(&stop_from(&root, "s", &script)).contains("14 earlier failures stand: "));
}

/// Round 15 (VD4R14-M-1): after five attempts none of the three commits of session `n1` is
/// given up. They take turns, one a stop, the one with the fewest attempts first: each comes
/// round again, and all three are named at every stop with what is true of them.
fn three_take_turns(root: &Path, script: &Path, log: &Path) {
    for turn in 0..7 {
        let before = runs(log);
        let said = allowed(&stop_from(root, "n1", script)).to_owned();
        assert!(
            said.contains("3 commits of this session have no verdict: ")
                && said.contains(&format!(
                    "(no verdict in {} attempts; it is asked about again in its turn, one such commit at each stop; the last reason: ",
                    6 + turn / 3
                )),
            "turn {turn}: {said}"
        );
        assert_eq!(
            runs(log),
            before + 1,
            "turn {turn}: one commit is asked about"
        );
        assert_eq!(
            shown(log).last().map(String::as_str),
            Some(format!("odd{}.rs", turn % 3).as_str()),
            "turn {turn}"
        );
    }
}

/// Fourth independent review of the hooks, HR4-M-2, the reviewer's N1 and N2. A commit that
/// got no verdict was asked about again, first, at every stop, and three of them ended the
/// judging of every later commit for the rest of the session. A commit whose patch is refused
/// while the chain gives verdicts is said once, remembered, and never stops the judging of
/// the next. Only a chain that gives no verdict ends the asking, and only for that stop.
#[test]
fn a_commit_with_no_verdict_never_stops_the_judging_of_later_commits() {
    let scratch = Scratch::new("hook-no-verdict");
    let root = repository(&scratch);
    let (script, log) = reading_judge(&scratch);
    // N1: three commits with no verdict, then a credential, before the first stop. The first
    // three asks fail and nothing has a verdict yet, so the judge is asked about its probe:
    // the chain works, and the asking goes on.
    start(&root, "n1");
    let odd: Vec<String> = (0..3)
        .map(|n| unjudgeable(&root, &format!("odd{n}.rs")))
        .collect();
    secret(&root, "k.rs");
    let key = commit_all(&root, "a key");
    let reason = blocked(&stop_from(&root, "n1", &script)).to_owned();
    assert!(
        reason.contains(&format!("Commit {key}: FAIL\n")),
        "{reason}"
    );
    assert!(
        reason.contains("3 commits of this session have no verdict: ")
            && odd.iter().all(|id| reason.contains(&format!("{id} ("))),
        "{reason}"
    );
    // Round 14 (HR5-H-1): a commit with no verdict is not done. It is named at every stop,
    // and asked about again: at every stop for five attempts, then in its turn.
    for attempt in 2..=5 {
        let before = runs(&log);
        let said = allowed(&stop_from(&root, "n1", &script)).to_owned();
        assert!(
            said.starts_with("deep-diff-forge: UNJUDGED. ")
                && said.contains("3 commits of this session have no verdict: ")
                && said.contains(&format!("Commit {key}: FAIL, which stands\n")),
            "{attempt}: {said}"
        );
        // the three commits, and the probe once nothing had a verdict
        assert_eq!(runs(&log), before + 4, "attempt {attempt}");
    }
    three_take_turns(&root, &script, &log);

    // N2: no-verdict commits and harmless ones in turn; a stop; then the credential
    start(&root, "n2");
    for n in 0..3 {
        unjudgeable(&root, &format!("more{n}.rs"));
        std::fs::write(root.join(format!("fine{n}.rs")), "fn f() {}\n").unwrap();
        commit_all(&root, "harmless");
    }
    assert!(allowed(&stop_from(&root, "n2", &script)).starts_with("deep-diff-forge: UNJUDGED. "));
    secret(&root, "k2.rs");
    let key = commit_all(&root, "another key");
    let reason = blocked(&stop_from(&root, "n2", &script)).to_owned();
    assert!(
        reason.contains(&format!("Commit {key}: FAIL\n"))
            && reason.contains("3 commits of this session have no verdict: "),
        "{reason}"
    );

    // The chain gives no verdict (here: a judge that cannot read its environment): after
    // three asks and the probe nothing more is asked at that stop, the rest are counted,
    // nothing is final, and the next stop, with a judge that works, has them all judged.
    start(&root, "chain");
    for n in 0..4 {
        std::fs::write(root.join(format!("later{n}.rs")), "fn f() {}\n").unwrap();
        commit_all(&root, "harmless");
    }
    secret(&root, "k3.rs");
    let key = commit_all(&root, "a third key");
    let broken = scratch.join("broken-judge");
    let calls = scratch.join("broken.log");
    write_program(
        &broken,
        &format!(
            "#!/bin/sh\necho x >> '{}'\necho '{{\"exit\":3,\"name\":\"io\",\"message\":\"no Git allowlist enrolled\",\"next\":\"check_environment\"}}' >&2\nexit 3\n",
            calls.display()
        ),
    );
    for stop in 1..=6 {
        let said = allowed(&stop_from(&root, "chain", &broken)).to_owned();
        // at the first stop two commits were not reached; from the second on all five have
        // been asked about, and none is given up
        let named = if stop == 1 {
            said.contains("2 commits of this session: UNJUDGED\n  not judged: ")
                && said.contains("3 commits of this session have no verdict: ")
        } else {
            said.contains("5 commits of this session have no verdict: ")
        };
        assert!(
            named
                && said.contains("no Git allowlist enrolled")
                && !said.contains("could not be judged in"),
            "stop {stop}: {said}"
        );
        // three commits and the probe were asked about, not all five commits
        assert_eq!(
            std::fs::read_to_string(&calls).unwrap().lines().count(),
            4 * stop
        );
    }
    let reason = blocked(&stop_from(&root, "chain", &script)).to_owned();
    assert!(
        reason.contains(&format!("Commit {key}: FAIL\n")) && !reason.contains("no verdict"),
        "{reason}"
    );
}

/// Fourth independent review of the hooks, HR4-M-3, the reviewer's `t7`. With a state that
/// cannot be written nothing is remembered, and a failing commit, which cannot be repaired,
/// blocked every stop for good. The client's own `stop_hook_active` is honoured then, and
/// only then: a hook that cannot remember does not block twice running.
#[test]
fn a_failing_commit_does_not_block_for_ever_when_the_state_cannot_be_kept() {
    use std::os::unix::fs::PermissionsExt as _;
    let scratch = Scratch::new("hook-unwritable-commit");
    let root = repository(&scratch);
    let (script, _) = reading_judge(&scratch);
    start(&root, "s");
    let sessions = sessions();
    std::fs::set_permissions(&sessions, std::fs::Permissions::from_mode(0o500)).unwrap();
    // not as root, who writes there all the same
    if std::fs::write(sessions.join("probe"), "x").is_err() {
        secret(&root, "k.rs");
        let key = commit_all(&root, "a key");
        git(&root, &["reset", "--quiet", "--hard", "HEAD~1"]);
        let decision = stop_active(&root, "s", &script, false);
        let reason = blocked(&decision);
        assert!(
            reason.contains(&format!("Commit {key}: FAIL\n"))
                && reason.contains("The hook could not keep its state ("),
            "{reason}"
        );
        for _ in 0..3 {
            let decision = stop_active(&root, "s", &script, true);
            let said = allowed(&decision);
            assert!(
                said.starts_with("deep-diff-forge: FAIL STANDS. This failure is not blocked because the client says this stop already follows a block and the hook cannot keep its own record")
                    && !said.contains("when it blocked")
                    && said.contains(&format!("Commit {key}: FAIL\n"))
                    && said.contains("The hook could not keep its state ("),
                "{said}"
            );
        }
        // a new turn (the client is not continuing after a block): it blocks again, said
        blocked(&stop_active(&root, "s", &script, false));
    }
    std::fs::set_permissions(&sessions, std::fs::Permissions::from_mode(0o700)).unwrap();
    // with a state that can be kept, `stop_hook_active` decides nothing
    secret(&root, "k2.rs");
    commit_all(&root, "another key");
    blocked(&stop_active(&root, "s", &script, true));
}

/// Fourth independent review of the hooks, HR4-M-1, the reviewer's A1 and A4, and the weak
/// test "no hook test has a merge commit". A commit that was in the repository when the
/// session began (here: fetched before it) is not the session's when the session pulls it:
/// the stop does not judge it, and every message that names the range says so, the advice
/// that reports the commit's content included. A merge commit the session makes is judged
/// against its first parent, so what a merge brings in is judged with it.
#[test]
fn a_commit_that_was_there_before_the_session_is_not_judged_and_that_is_said() {
    const ALREADY: &str = "a commit that was already in the repository when the session began is not judged at a stop, also when it is pulled or merged in";
    let scratch = Scratch::new("hook-already-there");
    let root = repository(&scratch);
    let (script, log) = reading_judge(&scratch);
    let theirs = scratch.join("theirs");
    let clone = [
        "clone",
        "--quiet",
        "--no-local",
        root.to_str().unwrap(),
        theirs.to_str().unwrap(),
    ];
    git(scratch.path(), &clone);
    std::fs::write(
        theirs.join("k.rs"),
        "pub const K: &str = \"AKIAIOSFODNN7EXAMPLE\";\n",
    )
    .unwrap();
    commit_all(&theirs, "theirs: a key");
    git(
        &root,
        &["remote", "add", "origin", theirs.to_str().unwrap()],
    );
    // fetched before the session: an editor's background fetch, another session
    git(&root, &["fetch", "--quiet", "origin"]);
    let said = start(&root, "a1");
    assert!(
        said.contains("Not judged: a commit that was already in this repository when the session began, also when you pull, merge, rebase onto or reset to it"),
        "{said}"
    );
    git(&root, &["merge", "--quiet", "--ff-only", "origin/main"]);
    let (_, advice, _) = hook("post-tool", &[], &input("PostToolUse", &root, "a1", false));
    assert!(
        advice.contains("secret_like at \\\"k.rs\\\"") && advice.contains(ALREADY),
        "{advice}"
    );
    assert_eq!(stop_from(&root, "a1", &script), Value::Null);
    assert_eq!(runs(&log), 0);

    // A4: a local branch from before the session, merged after a commit of the session's
    // own: the merge commit is the session's, and holds what the branch brought
    git(&root, &["checkout", "--quiet", "-b", "wip", "HEAD~1"]);
    secret(&root, "w.rs");
    commit_all(&root, "wip: a key");
    git(&root, &["checkout", "--quiet", "main"]);
    start(&root, "a4");
    std::fs::write(root.join("own.rs"), "fn own() {}\n").unwrap();
    commit_all(&root, "the session's own");
    git(&root, &["merge", "--quiet", "--no-edit", "wip"]);
    let merge = short(&root, "HEAD");
    let reason = blocked(&stop_from(&root, "a4", &script)).to_owned();
    assert!(
        reason.contains(&format!("Commit {merge}: FAIL\n")) && reason.contains(ALREADY),
        "{reason}"
    );
    assert_eq!(shown(&log), ["own.rs", "w.rs"]);
}

/// The fourth review's weak tests: no test needed the refs as a source, because every commit
/// under a branch, a tag or the stash had been on a `HEAD`, so a reflog named it too. Three
/// commits made with plumbing, each named by one ref and by no reflog, are each judged.
#[test]
fn a_commit_that_only_a_branch_a_tag_or_the_stash_names_is_judged() {
    let scratch = Scratch::new("hook-ref-only");
    let root = repository(&scratch);
    let (script, _) = reading_judge(&scratch);
    start(&root, "s");
    let mut named = Vec::new();
    for (name, file) in [
        ("refs/heads/only-a-branch", "kb.rs"),
        ("refs/tags/only-a-tag", "kt.rs"),
        ("refs/stash", "ks.rs"),
    ] {
        let commit = plumbing_commit(&root, file);
        git(&root, &["update-ref", name, &commit]);
        named.push(commit[..12].to_owned());
    }
    assert_eq!(git(&root, &["reflog", "show", "refs/stash"]), "");
    assert_eq!(git(&root, &["reflog"]).lines().count(), 1);
    let reason = blocked(&stop_from(&root, "s", &script)).to_owned();
    for id in &named {
        assert!(
            reason.contains(&format!("Commit {id}: FAIL\n")),
            "{id}: {reason}"
        );
    }
}

/// Fourth independent review of the hooks, HR4-L-1 and HR4-L-5. A linked working tree that
/// was removed by hand, with a plain directory put at its path, made every session one that
/// could not be recorded. It is passed over and named. And another working tree of the
/// project that has changes which are not committed, while the session is not in it, was
/// silent: it is named as not judged (both in full once, then in one line at every stop).
#[test]
fn other_working_trees_that_are_gone_or_have_changes_are_named_at_every_stop() {
    let scratch = Scratch::new("hook-other-trees");
    let root = repository(&scratch);
    let (script, _) = reading_judge(&scratch);
    let (gone, other) = (scratch.join("gone"), scratch.join("other"));
    for (tree, branch) in [(&gone, "gone"), (&other, "other")] {
        git(
            &root,
            &[
                "worktree",
                "add",
                "--quiet",
                "-b",
                branch,
                tree.to_str().unwrap(),
            ],
        );
    }
    std::fs::remove_dir_all(&gone).unwrap();
    std::fs::create_dir(&gone).unwrap();

    let said = start(&root, "s");
    assert!(!said.contains("could not be listed"), "{said}");
    secret(&other, "k.rs");
    let decision = stop_from(&root, "s", &script);
    let said = allowed(&decision);
    assert!(
        said.contains("of this project is gone or is no working tree any more")
            && said.contains("of this project has changes that are not committed, and the session's directory is not in it: they were not judged"),
        "{said}"
    );
    // at every stop while it is true: in one line after the first
    assert!(allowed(&stop_from(&root, "s", &script)).starts_with(
        "deep-diff-forge: 2 things in this project are not judged: the working tree "
    ));
    // the session is recorded all the same: its commit is judged
    secret(&root, "k2.rs");
    let key = commit_all(&root, "a key");
    assert!(blocked(&stop_from(&root, "s", &script)).contains(&format!("Commit {key}: FAIL\n")));
    // and from inside the other tree its change is judged
    blocked(&stop_from(&other, "s", &script));
}

/// Fourth independent review of the hooks, HR4-L-2. In a shallow clone a commit at the edge
/// has no parent the repository holds; judged "from nothing" its change was the whole tree.
/// It is UNJUDGED by its id, with that reason, at every stop while the clone is shallow
/// there; and when the clone has its history (`git fetch --unshallow`, which the reason
/// names) it is judged. It stayed "shallow" for the rest of the session (sixth independent
/// review of the hooks, HR6-L-2).
#[test]
fn a_commit_at_the_edge_of_a_shallow_clone_is_not_judged_from_nothing() {
    let scratch = Scratch::new("hook-shallow");
    let origin = repository(&scratch);
    let (script, log) = reading_judge(&scratch);
    std::fs::write(origin.join("b.rs"), "fn c() {}\n").unwrap();
    commit_all(&origin, "second");
    let root = scratch.join("shallow");
    let url = format!("file://{}", origin.display());
    git(
        scratch.path(),
        &[
            "clone",
            "--quiet",
            "--depth",
            "1",
            &url,
            root.to_str().unwrap(),
        ],
    );
    start(&root, "s");
    secret(&origin, "c.rs");
    commit_all(&origin, "third");
    git(
        &root,
        &["fetch", "--quiet", "--depth", "1", "origin", "main"],
    );
    git(&root, &["reset", "--quiet", "--hard", "FETCH_HEAD"]);
    let tip = short(&root, "HEAD");
    // said at every stop, not once; and nothing is asked of the judge
    for _ in 0..3 {
        let said = allowed(&stop_from(&root, "s", &script)).to_owned();
        assert!(
            said.contains(&format!(
                "1 commit of this session has no verdict: {tip} (this clone is shallow"
            )),
            "{said}"
        );
    }
    assert_eq!(runs(&log), 0);
    // the history is fetched: the commit has its parent now, is judged, and blocks
    git(&root, &["fetch", "--quiet", "--unshallow", "origin"]);
    let reason = blocked(&stop_from(&root, "s", &script)).to_owned();
    assert!(
        reason.contains(&format!("Commit {tip}: FAIL\n")),
        "{reason}"
    );
    assert_eq!(shown(&log), ["c.rs"]);
}

/// Sixth independent review of the hooks, HR6-L-3. One commit that gets no verdict and
/// nothing else to judge: the judge's probe was asked only after three failed asks, so the
/// chain was never seen to work, no attempt counted, and every stop of the session asked
/// about the commit again. The probe is asked whenever an ask failed and no patch got a
/// verdict: the attempts count, and after five the commit is asked about in its turn.
#[test]
fn one_commit_with_no_verdict_has_its_attempts_counted() {
    let scratch = Scratch::new("hook-one-no-verdict");
    let root = repository(&scratch);
    let (script, log) = reading_judge(&scratch);
    start(&root, "s");
    let odd = unjudgeable(&root, "odd.rs");
    for attempt in 1..=7 {
        let before = runs(&log);
        let said = allowed(&stop_from(&root, "s", &script)).to_owned();
        assert!(
            said.contains(&format!("1 commit of this session has no verdict: {odd} (")),
            "{attempt}: {said}"
        );
        // the commit and the probe; in its turn the commit alone, and it says so
        assert_eq!(
            runs(&log) - before,
            if attempt <= 5 { 2 } else { 1 },
            "{attempt}"
        );
        assert_eq!(
            said.contains(&format!("(no verdict in {attempt} attempts; ")),
            attempt > 5,
            "{attempt}: {said}"
        );
    }
    let state = states_in(&sessions());
    let lines = std::fs::read_to_string(state[0].with_extension("judged")).unwrap();
    assert_eq!(
        lines
            .lines()
            .filter_map(|line| line[40..].split(':').nth(1))
            .collect::<Vec<_>>(),
        ["n1@1", "n2@2", "n3@3", "n4@4", "n5@5", "n6@6", "n7@7"],
        "{lines}"
    );
}

/// Fourth independent review of the hooks, HR4-L-3. The commits that have their answer are a
/// file beside the state, with no bound that makes one be forgotten (the oldest of 4,096 used
/// to go to the tips, and an ancestor that never got a verdict went with it). The state names
/// its lines by a digest and a count; a file without them is not believed, and what stands in
/// it after them is not read.
#[test]
fn the_commits_that_have_their_answer_are_a_file_the_state_names() {
    let scratch = Scratch::new("hook-judged-file");
    let root = repository(&scratch);
    let (script, _) = reading_judge(&scratch);
    start(&root, "s");
    let no_verdict = unjudgeable(&root, "odd.rs");
    std::fs::write(root.join("fine.rs"), "fn f() {}\n").unwrap();
    let harmless = commit_all(&root, "harmless");
    secret(&root, "k.rs");
    let key = commit_all(&root, "a key");
    blocked(&stop_from(&root, "s", &script));
    let state = states_in(&sessions());
    let file = state[0].with_extension("judged");
    let lines = std::fs::read_to_string(&file).unwrap();
    let said_of = |id: &str| -> String {
        let line = lines.lines().find(|line| line.starts_with(id)).unwrap();
        line[40..].to_owned()
    };
    assert_eq!(said_of(&harmless), ":p");
    // (with the names of its rules: round 15, HR6-M-2)
    assert_eq!(said_of(&key), ":f:no-secrets");
    // no verdict is not an answer that ends anything: one attempt, with the reason
    assert!(said_of(&no_verdict).starts_with(":n1@1:"), "{lines}");
    let kept: Value = serde_json::from_str(&std::fs::read_to_string(&state[0]).unwrap()).unwrap();
    assert_eq!(kept["judged_count"], 3);
    // a line after the ones the state names is not read
    std::fs::write(&file, format!("{lines}{}:p\n", "a".repeat(40))).unwrap();
    assert!(
        allowed(&stop_from(&root, "s", &script))
            .contains(&format!("Commit {key}: FAIL, which stands\n"))
    );
    // cut short or gone, it is not another list: UNJUDGED, with the remedy
    for text in [Some(lines.lines().next().unwrap().to_owned() + "\n"), None] {
        match &text {
            Some(text) => std::fs::write(&file, text).unwrap(),
            None => std::fs::remove_file(&file).unwrap(),
        }
        let decision = stop_from(&root, "s", &script);
        let said = allowed(&decision);
        assert!(
            said.starts_with("deep-diff-forge: UNJUDGED. ")
                && said.contains("list of the commits that were judged")
                && said.contains("start a new session"),
            "{said}"
        );
        // HR5-L-6: and a resumed start of such a session says so too
        assert!(start(&root, "s").contains("list of the commits that were judged"));
    }
}

// ---- Round 13 of Vesper's review and the fifth independent review (2026-11-14, both NO-GO) ----
// The rule: a stop prints nothing only when every unit of the session has a pass.

/// Review V4 round 13, VD4R13-M-1, as reproduced, and the fifth independent review, HR5-H-1.
/// The judge gives no verdict once for the commit with the credential (a transient failure)
/// and passes the commit after it. The commit was written down as done, said once, and from
/// the next stop on the session was silent with a credential on its branch. A commit with no
/// verdict is not done: it is named, asked about again, and blocks when it gets its verdict.
#[test]
fn a_commit_whose_judging_failed_once_is_asked_again_and_blocks() {
    let scratch = Scratch::new("hook-transient");
    let root = repository(&scratch);
    let (script, log) = reading_judge(&scratch);
    start(&root, "s");
    secret(&root, "k.rs");
    let key = commit_all(&root, "a key");
    std::fs::write(root.join("fine.rs"), "fn f() {}\n").unwrap();
    commit_all(&root, "harmless");
    std::fs::write(scratch.join("judge.flaky"), "").unwrap();
    let said = allowed(&stop_from(&root, "s", &script)).to_owned();
    assert!(
        said.starts_with("deep-diff-forge: UNJUDGED. ")
            && said.contains(&format!("1 commit of this session has no verdict: {key} (")),
        "{said}"
    );
    assert_eq!(runs(&log), 2);
    let reason = blocked(&stop_from(&root, "s", &script)).to_owned();
    assert!(
        reason.contains(&format!("Commit {key}: FAIL\n- rule no-secrets: "))
            && !reason.contains("no verdict"),
        "{reason}"
    );
    assert_eq!(runs(&log), 3);
}

/// Fifth independent review of the hooks, HR5-M-4. A commit the judge holds, first judged at
/// a stop that blocked for something else, was shown there among the rest and never again.
/// What the judge holds is said at every stop.
#[test]
fn what_the_judge_holds_is_said_at_every_stop() {
    let scratch = Scratch::new("hook-holds");
    let root = repository(&scratch);
    let (script, _) = reading_judge(&scratch);
    start(&root, "s");
    std::fs::write(root.join("h.rs"), "// HOLDME\n").unwrap();
    let held = commit_all(&root, "for a person");
    secret(&root, "k.rs");
    let key = commit_all(&root, "a key");
    let reason = blocked(&stop_from(&root, "s", &script)).to_owned();
    assert!(
        reason.contains(&format!("Commit {key}: FAIL\n"))
            && reason.contains(&format!("Commit {held}: HOLD\n")),
        "{reason}"
    );
    for _ in 0..3 {
        let said = allowed(&stop_from(&root, "s", &script)).to_owned();
        assert!(
            said.starts_with("deep-diff-forge: HOLD. ")
                // with the name of its rule, at every stop (round 15, HR6-M-2: a commit
                // held at a crowded stop was named by its id alone from then on)
                && said.contains(&format!(
                    "1 held by the judge, for a person: Commit {held} (no-secrets).\n"
                ))
                && said.contains(&format!(
                    "Commit {key}: FAIL, which stands\n- rule no-secrets\n"
                )),
            "{said}"
        );
    }
}

/// Fifth independent review of the hooks, HR5-M-1 and HR5-M-2. A unit whose rules were longer
/// than the message had room for was shown with its name alone; and many commits that failed
/// at one stop made a message with no bound, the session's own commit last. A message stays
/// under 9,000 characters; what blocks is shown the newest first, each with its rules while
/// there is room, then with the names of its rules, then counted; a rule's name and first
/// path are never dropped, and a path that is cut keeps its file name.
#[test]
fn a_stops_message_is_bounded_and_what_blocks_keeps_its_rule() {
    let scratch = Scratch::new("hook-budget");
    let root = repository(&scratch);
    // one rule that names 4,000 long paths, in the uncommitted change
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
    start(&root, "long");
    silence_a_lint(&root);
    let reason = blocked(&stop_from(&root, "long", &script)).to_owned();
    assert!(reason.chars().count() < 9_000, "{}", reason.chars().count());
    // the rule, and its paths with their file names kept, as many as the message shows
    assert!(
        reason.contains("- rule no-silent-suppression: ")
            && reason.contains("/0000.rs\"")
            && reason.contains("/0019.rs\"")
            && reason.contains("and 3980 more, not listed."),
        "{reason}"
    );
    git(&root, &["checkout", "--quiet", "--", "a.rs"]);

    crowded_stop(&scratch, &root);
}

/// The second half of [`a_stops_message_is_bounded_and_what_blocks_keeps_its_rule`]: forty
/// commits that fail at one stop, the session's own the newest (as few as show all three
/// forms: a stop judges what its forty-five seconds allow, and on a loaded host sixty were
/// not all judged in them). With them, since the sixth independent review of the hooks
/// (HR6-L-4, "weak tests"): commits the judge holds, commits with no verdict, and things
/// beside the verdict, so that every kind of line is in one message.
fn crowded_stop(scratch: &Scratch, root: &Path) {
    let (script, _) = reading_judge(scratch);
    start(root, "many");
    let odd: Vec<String> = (0..3)
        .map(|n| unjudgeable(root, &format!("odd{n}.rs")))
        .collect();
    std::fs::write(root.join("h.rs"), "// HOLDME\n").unwrap();
    let held = commit_all(root, "for a person");
    for n in 0..39 {
        secret(root, &format!("pulled{n:02}.rs"));
        commit_all(root, "a key from elsewhere");
    }
    secret(root, "own.rs");
    let own = commit_all(root, "the session's own key");
    for n in 0..2 {
        let nested = root.join(format!("vendor{n}"));
        std::fs::create_dir(&nested).unwrap();
        git(&nested, &["init", "--quiet"]);
    }
    let reason = blocked(&stop_from(root, "many", &script)).to_owned();
    assert!(reason.chars().count() < 9_000, "{}", reason.chars().count());
    let named = reason
        .find(&format!("Commit {own}: FAIL\n- rule no-secrets: "))
        .unwrap_or_else(|| panic!("the session's own commit is not named: {reason}"));
    assert!(named < 1_000, "{named}: {reason}");
    // with their rules while there is room, then twenty with the names of their rules, then
    // a count: every one of the forty is in one of the three
    let with_rules = reason.matches(": FAIL\n- rule no-secrets: ").count();
    assert_eq!(
        reason.matches(": FAIL (no-secrets)\n").count(),
        20,
        "{reason}"
    );
    let counted: usize = reason
        .split_once(" more commits fail.")
        .and_then(|(before, _)| before.rsplit_once("and "))
        .and_then(|(_, count)| count.parse().ok())
        .unwrap_or_else(|| panic!("the rest are not counted: {reason}"));
    assert!(with_rules >= 8 && counted > 0, "{with_rules} {counted}");
    assert_eq!(with_rules + 20 + counted, 40, "{reason}");
    // what the judge holds, with the name of its rule; the commits with no verdict; and
    // what is beside the verdict: each is in the same message, whole
    assert!(
        reason.contains(&format!("Commit {held}: HOLD\n- rule no-secrets: "))
            || reason.contains(&format!(
                "1 held by the judge, for a person: Commit {held} (no-secrets).\n"
            )),
        "{reason}"
    );
    assert!(
        reason.contains("3 commits of this session have no verdict: ")
            && odd.iter().all(|id| reason.contains(&format!("{id} ("))),
        "{reason}"
    );
    assert!(
        reason.contains("The directory \"vendor0\" in ") && reason.contains("\"vendor1\""),
        "{reason}"
    );
    // afterwards they stand, as a count, beside the other lines of every stop
    let later = allowed(&stop_from(root, "many", &script)).to_owned();
    assert!(
        later.contains("40 earlier failures stand: ")
            && later.contains(&format!(
                "1 held by the judge, for a person: Commit {held} (no-secrets).\n"
            ))
            && later.contains("3 commits of this session have no verdict: ")
            && later.contains("2 things in this project are not judged: the nested repository \"vendor0\", the nested repository \"vendor1\"."),
        "{later}"
    );
}

/// Fifth independent review of the hooks, HR5-M-3. A credential is staged and blocks while
/// the working tree holds it too; the file is repaired and not staged again. The staged
/// change took over "has already blocked" from the working tree's, so the stop after an
/// incomplete repair did not block. The failure of what is staged is its own: it blocks, and
/// says what clears it.
#[test]
fn what_is_staged_blocks_for_itself_after_the_working_tree_was_repaired() {
    let scratch = Scratch::new("hook-staged-own");
    let root = repository(&scratch);
    let (script, _) = reading_judge(&scratch);
    let top = std::fs::canonicalize(&root).unwrap();
    let staged = uncommitted_in(&top).replace("Uncommitted changes", "Staged changes");
    start(&root, "s");
    std::fs::write(
        root.join("a.rs"),
        "fn a() {}\nfn b() {}\nconst KEY: &str = \"SECRET\";\n",
    )
    .unwrap();
    git(&root, &["add", "--all"]);
    blocked(&stop_from(&root, "s", &script));
    // repaired in the working tree, not staged again: the index still holds the credential
    std::fs::write(root.join("a.rs"), "fn a() {}\nfn b() {}\nfn c() {}\n").unwrap();
    let reason = blocked(&stop_from(&root, "s", &script)).to_owned();
    assert!(
        reason.contains(&format!("{staged}: FAIL\n- rule no-secrets: "))
            && reason.contains("stage the repaired file (`git add`) or take the old content out of the index (`git restore --staged`)"),
        "{reason}"
    );
    let stands = allowed(&stop_from(&root, "s", &script)).to_owned();
    assert!(
        stands.contains(&format!("{staged}: FAIL, which stands\n")),
        "{stands}"
    );
    git(&root, &["add", "--all"]);
    assert_eq!(stop_from(&root, "s", &script), Value::Null);
}

/// Fifth independent review of the hooks, HR5-L-1, HR5-L-5 and HR5-L-6. An entry made with
/// `git add -N` is no staged content (it was read as a staged empty file, and judged). A
/// bare repository's own entry in the list of working trees is no working tree (it was said
/// to have changes). And `agent doctor --gate` does not say that the patch link is ready
/// where the index has unmerged paths: a stop there has no verdict on what is staged.
#[test]
fn an_intent_to_add_a_bare_repository_and_unmerged_paths_are_read_as_what_they_are() {
    let scratch = Scratch::new("hook-index-lows");
    let root = repository(&scratch);
    let (script, log) = reading_judge(&scratch);
    start(&root, "ita");
    std::fs::write(root.join("n.rs"), "fn n() {}\n").unwrap();
    git(&root, &["add", "--intent-to-add", "n.rs"]);
    assert_eq!(stop_from(&root, "ita", &script), Value::Null);
    // one patch: the working tree's; the index holds no content of its own
    assert_eq!(runs(&log), 1);
    git(&root, &["add", "--all"]);
    git(&root, &["commit", "--quiet", "-m", "n"]);

    // a working tree of a bare repository: the bare repository itself is not looked at
    let bare = scratch.join("bare.git");
    let clone = [
        "clone",
        "--quiet",
        "--bare",
        root.to_str().unwrap(),
        bare.to_str().unwrap(),
    ];
    git(scratch.path(), &clone);
    let tree = scratch.join("tree");
    git(
        &bare,
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "work",
            tree.to_str().unwrap(),
        ],
    );
    start(&tree, "bare");
    assert_eq!(stop_from(&tree, "bare", &script), Value::Null);
    secret(&tree, "k.rs");
    blocked(&stop_from(&tree, "bare", &script));

    // unmerged paths in the index
    git(&root, &["checkout", "--quiet", "-b", "other"]);
    std::fs::write(root.join("a.rs"), "fn other() {}\n").unwrap();
    commit_all(&root, "other");
    git(&root, &["checkout", "--quiet", "main"]);
    std::fs::write(root.join("a.rs"), "fn main_side() {}\n").unwrap();
    commit_all(&root, "main side");
    let merged = Command::new("git")
        .current_dir(&root)
        .envs(ISOLATE)
        .args([
            "-c",
            "user.name=ddf",
            "-c",
            "user.email=ddf@example.invalid",
        ])
        .args(["merge", "--quiet", "other"])
        .output()
        .unwrap();
    assert!(!merged.status.success(), "the merge was to conflict");
    let (_, doc, _) = doctor_gate(&root, &home(), &["--judge", script.to_str().unwrap()]);
    let patch = gate_checks(&doc)
        .into_iter()
        .find(|(name, _)| name == "patch");
    assert_eq!(patch, Some(("patch".to_owned(), false)), "{doc:#}");
    // (said as what it is, with what to do: Git's "error building trees" was all it said;
    // sixth independent review of the hooks, HR6-L-5)
    assert!(
        detail(&doc, "patch").contains(
            "UNJUDGED for what is staged: the index has unmerged paths (a merge, a rebase or a cherry-pick stopped on a conflict)"
        ),
        "{doc:#}"
    );
    // and the start of a session there names it, instead of a promise
    let (_, said, _) = hook(
        "session-start",
        &["--judge", script.to_str().unwrap()],
        &input("SessionStart", &root, "unmerged", false),
    );
    assert!(
        said.contains("patch: a stop here would be UNJUDGED for what is staged")
            && !said.contains("stops will be judged"),
        "{said}"
    );
    let stop = allowed(&stop_from(&root, "unmerged", &script)).to_owned();
    assert!(
        stop.contains("what is staged cannot be judged: the index has unmerged paths (")
            && stop.contains("resolve them and `git add` the files"),
        "{stop}"
    );
}

/// The rule, for what a stop names beside its verdict. A nested repository that was left out,
/// another working tree with changes, and working trees beyond the sixteen a stop looks at
/// were each said once in a session, or not at all (the seventeenth tree). Each is said at
/// every stop while it is true, in full the first time and then in the line that counts
/// them, and no more when it is no longer true. A stop where everything passes and such a
/// thing is true is not silent.
#[test]
fn what_is_in_the_project_and_not_judged_is_said_at_every_stop() {
    let scratch = Scratch::new("hook-notes-every-stop");
    let root = repository(&scratch);
    let (script, _) = reading_judge(&scratch);
    let nested = root.join("vendor");
    std::fs::create_dir(&nested).unwrap();
    git(&nested, &["init", "--quiet"]);
    std::fs::write(nested.join("x"), "x\n").unwrap();
    let other = scratch.join("other");
    git(
        &root,
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "other",
            other.to_str().unwrap(),
        ],
    );
    start(&root, "s");
    std::fs::write(other.join("new.rs"), "fn n() {}\n").unwrap();
    let said = allowed(&stop_from(&root, "s", &script)).to_owned();
    assert!(
        said.contains("The directory \"vendor\" in ")
            && said.contains(
                "has changes that are not committed, and the session's directory is not in it"
            ),
        "{said}"
    );
    for _ in 0..3 {
        let said = allowed(&stop_from(&root, "s", &script)).to_owned();
        assert!(
            said.starts_with("deep-diff-forge: 2 things in this project are not judged: the nested repository \"vendor\", the working tree \""),
            "{said}"
        );
    }
    // no longer true: no longer said
    std::fs::remove_dir_all(&nested).unwrap();
    std::fs::remove_file(other.join("new.rs")).unwrap();
    assert_eq!(stop_from(&root, "s", &script), Value::Null);
    // eighteen more working trees: sixteen are looked at, and the rest are counted
    for n in 0..17 {
        let tree = scratch.join(format!("tree{n:02}"));
        git(
            &root,
            &[
                "worktree",
                "add",
                "--quiet",
                "--detach",
                tree.to_str().unwrap(),
            ],
        );
    }
    let said = allowed(&stop_from(&root, "s", &script)).to_owned();
    assert!(
        said.contains("This project has 2 more working trees than the 16 a stop looks at"),
        "{said}"
    );
    assert_eq!(
        allowed(&stop_from(&root, "s", &script)),
        "deep-diff-forge: 1 thing in this project is not judged: 2 more working trees that were not looked at."
    );
}

/// Fifth independent review of the hooks, HR5-L-2, the weak test "a policy with no probe". A
/// policy with no line has no probe, so a stop whose first three asks get no verdict cannot
/// ask whether the chain works: it asks no more, and says how many commits wait. The commit
/// with the credential is asked about first at the next stop, and blocks. One commit with no
/// verdict used to end the judging of every later one, at every stop.
#[test]
fn a_policy_with_no_probe_still_has_every_commit_judged() {
    let scratch = Scratch::new("hook-no-probe");
    let root = repository(&scratch);
    let (script, log) = reading_judge(&scratch);
    std::fs::write(root.join("warden.toml"), "").unwrap();
    commit_all(&root, "a policy with no line");
    start(&root, "s");
    for n in 0..3 {
        unjudgeable(&root, &format!("odd{n}.rs"));
    }
    secret(&root, "k.rs");
    let key = commit_all(&root, "a key");
    let said = allowed(&stop_from(&root, "s", &script)).to_owned();
    assert!(
        said.contains("1 commit of this session: UNJUDGED\n")
            && said.contains("3 commits of this session have no verdict: "),
        "{said}"
    );
    assert_eq!(runs(&log), 3);
    let reason = blocked(&stop_from(&root, "s", &script)).to_owned();
    assert!(
        reason.contains(&format!("Commit {key}: FAIL\n")),
        "{reason}"
    );
}

/// Review V4 round 14, VD4R14-M-1, as reproduced. A commit with a credential; a harmless
/// commit before each stop; a judge that gives no verdict on the credential's patch the first
/// five times and answers for every other patch, so that the chain is seen working and each
/// attempt counts. After five attempts the commit was asked about no more: the judge
/// recovered, the failing verdict was never obtained, and no stop of the session blocked. It
/// is asked about again in its turn, and the stop after the recovery blocks.
#[test]
fn a_commit_that_had_no_verdict_five_times_is_asked_about_again_and_blocks() {
    let scratch = Scratch::new("hook-attempts");
    let root = repository(&scratch);
    let (script, log) = reading_judge(&scratch);
    start(&root, "s");
    secret(&root, "k.rs");
    let key = commit_all(&root, "a key");
    std::fs::write(scratch.join("judge.flaky"), "5\n").unwrap();
    let harmless = |stop: usize| {
        std::fs::write(root.join(format!("fine{stop}.rs")), "fn f() {}\n").unwrap();
        commit_all(&root, "harmless");
    };
    for stop in 1..=5 {
        harmless(stop);
        let said = allowed(&stop_from(&root, "s", &script)).to_owned();
        assert!(
            said.starts_with("deep-diff-forge: UNJUDGED. ")
                && said.contains(&format!("1 commit of this session has no verdict: {key} (")),
            "stop {stop}: {said}"
        );
    }
    assert!(
        !scratch.join("judge.flaky").exists(),
        "five times no verdict"
    );
    // the judge answers again: the commit is asked about, after the new one, and blocks
    harmless(6);
    let before = runs(&log);
    let reason = blocked(&stop_from(&root, "s", &script)).to_owned();
    assert!(
        reason.contains(&format!("Commit {key}: FAIL\n")),
        "{reason}"
    );
    assert_eq!(runs(&log), before + 2);
    // (the commit that waits for its turn is asked about before the new one: round 16)
    let asked = shown(&log);
    assert_eq!(asked[asked.len() - 2..], ["k.rs", "fine6.rs"]);
    // like any commit that failed: it has blocked once, and stands
    harmless(7);
    assert!(
        allowed(&stop_from(&root, "s", &script))
            .contains(&format!("Commit {key}: FAIL, which stands\n"))
    );

    // Two such commits take turns, the older first: one is asked about at a stop, and the
    // other says that it waits for its turn.
    start(&root, "two");
    secret(&root, "k1.rs");
    let first = commit_all(&root, "a key");
    secret(&root, "k2.rs");
    let second = commit_all(&root, "another key");
    std::fs::write(scratch.join("judge.flaky"), "10\n").unwrap();
    for stop in 1..=5 {
        harmless(10 + stop);
        let said = allowed(&stop_from(&root, "two", &script)).to_owned();
        assert!(
            said.contains("2 commits of this session have no verdict: "),
            "stop {stop}: {said}"
        );
    }
    assert!(!scratch.join("judge.flaky").exists());
    let before = runs(&log);
    let reason = blocked(&stop_from(&root, "two", &script)).to_owned();
    assert!(
        reason.contains(&format!("Commit {first}: FAIL\n"))
            && reason.contains(&format!(
                "1 commit of this session has no verdict: {second} (no verdict in 5 attempts; it is asked about again in its turn, one such commit at each stop; the last reason: "
            )),
        "{reason}"
    );
    assert_eq!(
        runs(&log),
        before + 1,
        "one commit is asked about in its turn"
    );
    let reason = blocked(&stop_from(&root, "two", &script)).to_owned();
    assert!(
        reason.contains(&format!("Commit {second}: FAIL\n"))
            && reason.contains(&format!("Commit {first}: FAIL, which stands\n")),
        "{reason}"
    );
}

/// Sixth independent review of the hooks, HR6-M-1 (a), as a table over stops. A failure that
/// has blocked and stands is forgotten only by a stop that looked for it and did not find it.
/// The staged change had an identity of its own and was a unit only when it differed from
/// the working tree's, so `git add -A` and one more edit of any file made a failure that
/// stood block again, every other turn, with advice that was not true there.
#[test]
fn a_staged_failure_that_has_not_changed_never_blocks_again() {
    let scratch = Scratch::new("hook-staged-table");
    let root = repository(&scratch);
    let (script, _) = reading_judge(&scratch);
    let top = std::fs::canonicalize(&root).unwrap();
    let staged = uncommitted_in(&top).replace("Uncommitted changes", "Staged changes");
    // (a message names its units when there are more than one)
    let stands = |words: &str, step: &str| {
        let said = allowed(&stop_from(&root, "s", &script)).to_owned();
        assert!(
            said.starts_with("deep-diff-forge: FAIL STANDS. ")
                && said.contains("- rule no-secrets")
                && (words.is_empty() || said.contains(&format!("{words}: FAIL, which stands\n"))),
            "{step}: {said}"
        );
        said
    };
    let mut edits = 0;
    let mut edit_another = || {
        edits += 1;
        std::fs::write(root.join("other.rs"), format!("// edit {edits}\n")).unwrap();
    };
    start(&root, "s");
    // 1: a credential is staged, and the tree holds the same: one block, then it stands
    secret(&root, "k.rs");
    git(&root, &["add", "--all"]);
    let reason = blocked(&stop_from(&root, "s", &script)).to_owned();
    assert!(reason.contains("- rule no-secrets: "), "{reason}");
    stands("", "1");
    // 2: another file is edited and not staged. The index and the tree differ now, and the
    // failing path is the same in both: it stands, and nothing says "stage the repaired file"
    edit_another();
    let said = stands("", "2");
    assert!(
        !said.contains("Staged changes") && !said.contains("git add"),
        "{said}"
    );
    // 3: the failing path is repaired in the tree and the repair is not staged: what a plain
    // `git commit` would record still fails, which blocks once, with advice that is true
    std::fs::write(root.join("k.rs"), "const KEY: &str = \"\";\n").unwrap();
    let reason = blocked(&stop_from(&root, "s", &script)).to_owned();
    assert!(
        reason.contains(&format!("{staged}: FAIL\n- rule no-secrets: "))
            && reason.contains("stage the repaired file (`git add`)"),
        "{reason}"
    );
    stands(&staged, "3");
    edit_another();
    stands(&staged, "3, another edit");
    // 4: the repair is staged: forgotten, and everything passes
    git(&root, &["add", "--all"]);
    assert_eq!(stop_from(&root, "s", &script), Value::Null);
    edit_another();
    assert_eq!(stop_from(&root, "s", &script), Value::Null);

    // 5: the reviewer's sequence. A fixture with a key is told to the person at turn 1 and
    // left as it is; the session goes on staging and editing. It blocked at turns 2, 4, 6.
    start(&root, "t");
    secret(&root, "fixture.rs");
    blocked(&stop_from(&root, "t", &script));
    for turn in 2..=7 {
        if turn % 2 == 0 {
            git(&root, &["add", "--all"]);
            edit_another();
        } else {
            git(&root, &["add", "--all"]);
        }
        let said = allowed(&stop_from(&root, "t", &script)).to_owned();
        assert!(
            said.starts_with("deep-diff-forge: FAIL STANDS. ") && !said.contains("Staged changes"),
            "turn {turn}: {said}"
        );
    }
}

/// HR6-M-1 (b). A failing change in a linked working tree blocks once, from inside it. A stop
/// made from the project's own directory does not look at that tree: it names the tree and
/// says that a failure stands there, and forgets nothing, so the next stop from inside the
/// tree does not block again. It blocked at every return. A tree that a stop finds with
/// nothing uncommitted has no such failure left: when the failure comes back, it blocks.
#[test]
fn a_failure_in_a_working_tree_that_was_not_looked_at_stands() {
    let scratch = Scratch::new("hook-linked-stands");
    let root = repository(&scratch);
    let (script, _) = reading_judge(&scratch);
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
    start(&root, "s");
    secret(&linked, "k.rs");
    blocked(&stop_from(&linked, "s", &script));
    for round in 0..3 {
        let inside = allowed(&stop_from(&linked, "s", &script)).to_owned();
        assert!(
            inside.starts_with("deep-diff-forge: FAIL STANDS. "),
            "round {round}: {inside}"
        );
        // from the project: the tree is not looked at, and what stands there is said, in
        // full the first time and on the line that counts such things at every stop after
        // (it was said once: seventh independent review of the hooks, HR7-L-3)
        let outside = allowed(&stop_from(&root, "s", &script)).to_owned();
        let named = if round == 0 {
            outside.contains("has changes that are not committed, and the session's directory is not in it")
                && outside.contains("A failure there has blocked a stop of this session and stands: it was not looked at at this stop.")
        } else {
            outside.starts_with(
                "deep-diff-forge: 1 thing in this project is not judged: the working tree \"",
            ) && outside.ends_with(" (a failure stands there).")
        };
        assert!(named, "round {round}: {outside}");
    }
    // the tree is cleaned while the session is elsewhere, and a stop from the project sees
    // that: the failure is gone, so when it comes back it is new, and blocks
    std::fs::remove_file(linked.join("k.rs")).unwrap();
    assert_eq!(stop_from(&root, "s", &script), Value::Null);
    secret(&linked, "k.rs");
    blocked(&stop_from(&linked, "s", &script));
}

/// Sixth independent review of the hooks, suspected from reading: two stops of one session at
/// the same time could write their lists and states across each other, after which the state
/// named a list that was not in the file and every later stop was UNJUDGED. A stop holds a
/// lock on a file beside the session's state for as long as it runs: the second waits three
/// seconds, then says that another stop of the session is running, and judges nothing.
#[test]
fn two_stops_of_one_session_do_not_run_at_once() {
    let scratch = Scratch::new("hook-one-stop");
    let root = repository(&scratch);
    let (script, log) = reading_judge(&scratch);
    start(&root, "s");
    secret(&root, "k.rs");
    let key = commit_all(&root, "a key");
    std::fs::write(scratch.join("judge.sleep"), "7\n").unwrap();
    let (first, second) = std::thread::scope(|scope| {
        let slow = scope.spawn(|| {
            home_in(&scratch);
            stop_from(&root, "s", &script)
        });
        // the first stop holds the lock from before it reads its state; when its judge has
        // begun (and sleeps), it holds it
        let asked = std::time::Instant::now();
        while runs(&log) == 0 {
            assert!(
                asked.elapsed() < std::time::Duration::from_secs(20),
                "the first stop did not reach its judge in twenty seconds"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let second = stop_from(&root, "s", &script);
        (slow.join().unwrap(), second)
    });
    let waited = allowed(&second);
    assert!(
        waited.starts_with("deep-diff-forge: UNJUDGED. ")
            && waited.contains("another hook of this session is running"),
        "{waited}"
    );
    assert!(
        blocked(&first).contains(&format!("Commit {key}: FAIL\n")),
        "{first}"
    );
    assert_eq!(runs(&log), 1, "the second stop ran the judge");
    // afterwards the session goes on: the failure stands, and nothing is UNJUDGED
    std::fs::remove_file(scratch.join("judge.sleep")).unwrap();
    assert!(
        allowed(&stop_from(&root, "s", &script))
            .contains(&format!("Commit {key}: FAIL, which stands\n"))
    );
    // a session with no state has no lock file: a stop that records nothing leaves nothing
    let nowhere = scratch.join("nowhere");
    std::fs::create_dir(&nowhere).unwrap();
    assert_eq!(stop_from(&nowhere, "none", &script), Value::Null);
    let locks = std::fs::read_dir(sessions())
        .unwrap()
        .filter(|entry| {
            entry
                .as_ref()
                .unwrap()
                .path()
                .extension()
                .is_some_and(|kind| kind == "lock")
        })
        .count();
    assert_eq!(locks, 1);
}

/// The rule of the hooks as a table over stops, with a judge: every kind of unit in every
/// outcome, three stops each with nothing changed between them. A stop is silent only when
/// the unit has a pass; a failure blocks once and then stands; what is held and what has no
/// verdict is said at every stop; and nothing blocks twice. The unit test of the decision
/// sees one stop of units it is handed: it cannot see which units a stop makes, or what a
/// sequence of stops remembers and forgets (sixth independent review of the hooks, "weak
/// tests").
#[test]
fn a_stop_is_silent_only_for_a_pass_at_each_of_three_stops() {
    const SECRET: &str = "const KEY: &str = \"SECRET\";\n";
    let scratch = Scratch::new("hook-rule-table");
    let base = repository(&scratch);
    let (script, _) = reading_judge(&scratch);
    let flaky = scratch.join("judge.flaky");
    let outcomes: [(&str, &str, [&str; 3]); 5] = [
        ("pass", "fn f() {}\n", ["", "", ""]),
        ("hold", "// HOLDME\n", ["HOLD", "HOLD", "HOLD"]),
        ("fail", SECRET, ["block", "FAIL STANDS", "FAIL STANDS"]),
        (
            "no verdict",
            "// NOVERDICT\n",
            ["UNJUDGED", "UNJUDGED", "UNJUDGED"],
        ),
        (
            "no verdict once",
            SECRET,
            ["UNJUDGED", "block", "FAIL STANDS"],
        ),
    ];
    let mut row = 0;
    for kind in ["uncommitted", "staged", "commit"] {
        for (outcome, text, expected) in outcomes {
            row += 1;
            let root = scratch.join(format!("row{row}"));
            git(
                scratch.path(),
                &[
                    "clone",
                    "--quiet",
                    base.to_str().unwrap(),
                    root.to_str().unwrap(),
                ],
            );
            let session = format!("row{row}");
            start(&root, &session);
            std::fs::write(root.join("u.rs"), text).unwrap();
            let mut named = String::new();
            match kind {
                "staged" => {
                    // the index holds it and the working tree does not: another change
                    git(&root, &["add", "u.rs"]);
                    std::fs::write(root.join("u.rs"), "fn other() {}\n").unwrap();
                    named = "Staged changes in ".to_owned();
                }
                "commit" => named = format!("Commit {}", commit_all(&root, "the unit")),
                _ => {}
            }
            if outcome == "no verdict once" {
                std::fs::write(&flaky, "").unwrap();
            }
            for (stop, word) in expected.iter().enumerate() {
                let decision = stop_from(&root, &session, &script);
                let at = format!("{kind}, {outcome}, stop {}: {decision}", stop + 1);
                let said = match *word {
                    "" => {
                        assert_eq!(decision, Value::Null, "{at}");
                        continue;
                    }
                    "block" => blocked(&decision),
                    word => {
                        let said = allowed(&decision);
                        assert!(said.contains(word), "{at}");
                        said
                    }
                };
                // a commit that has no verdict is named by its id alone
                let named = if kind == "commit" && *word == "UNJUDGED" {
                    named.trim_start_matches("Commit ")
                } else {
                    named.as_str()
                };
                assert!(said.contains(named), "{at}");
            }
            assert!(!flaky.exists(), "{kind}, {outcome}");
        }
    }
    assert_eq!(row, 15);
}

/// A stand-in judge that lists what it finds, file by file, as the real one does: a file that
/// adds lines with `SECRET` is a violation of the rule `no-secrets`, which fails, with one
/// piece of evidence for each such line; a file that adds a line with `HOLDME` is a violation
/// of `no-silent-suppression`, which holds; a patch that adds a line with `NOTEME` is held
/// for a note, with every rule passed; one that adds `NOVERDICT` gets no verdict. The other
/// stand-ins give one fixed violation whatever they are shown, so they cannot tell a failure
/// of which a part was repaired from the same failure.
fn listing_judge(scratch: &Scratch) -> (PathBuf, PathBuf) {
    const SCRIPT: &str = r#"#!/bin/sh
printf 'ARGS %s\n' "$*" >> '@LOG@'
policy=; patch=; ranker=
while [ $# -gt 0 ]; do
  case "$1" in --policy) policy="$2";; --patch) patch="$2";; --ranker) ranker="$2";; esac
  shift
done
printf 'PATCH-NAMES %s\n' "$(grep '^diff --git' "$patch" | tr '\n' ' ')" >> '@LOG@'
if grep -q '^+.*NOVERDICT' "$patch"; then echo '{"exit":3,"name":"io","message":"patch parse failed","next":"fix_input"}' >&2; exit 3; fi
sum() { if command -v sha256sum >/dev/null 2>&1; then sha256sum < "$1"; else shasum -a 256 < "$1"; fi | cut -d' ' -f1; }
p="$(sum "$policy")"; f="$(sum "$patch")"; r="$(sum "$ranker")"
list() {
  awk -v word="$1" -v signal="$2" -v address="$f" '
    /^diff --git / { file = $NF; sub(/^b\//, "", file) }
    /^\+\+\+ / { next }
    /^\+/ { if (index($0, word)) { if (!(file in n)) order[++files] = file; n[file]++ } }
    END {
      for (i = 1; i <= files; i++) {
        file = order[i]
        printf "%s{\"path\":\"%s\",\"message\":\"m\",\"signal\":\"%s\",\"evidence\":[", (i > 1 ? "," : ""), file, signal
        for (j = 1; j <= n[file]; j++) printf "%s{\"address\":\"%s\",\"side\":\"new\",\"line\":%d}", (j > 1 ? "," : ""), address, j
        printf "]}"
      }
    }' "$patch"
}
secrets="$(list SECRET secret_like)"; holds="$(list HOLDME check_suppressed)"
notes=; if grep -q '^+.*NOTEME' "$patch"; then notes='{"code":"patch_unverifiable","detail":"a note of the stand-in judge"}'; fi
if [ -n "$secrets" ]; then code=7; said=fail; name=policy_fail; action=change_input
elif [ -n "$holds$notes" ]; then code=8; said=hold; name=policy_hold; action=review_evidence
else code=0; said=pass; name=pass; action=proceed; fi
o1=pass; if [ -n "$secrets" ]; then o1=fail; fi
o2=pass; if [ -n "$holds" ]; then o2=hold; fi
count() { n=0; if [ "$o1" = "$1" ]; then n=$((n+1)); fi; if [ "$o2" = "$1" ]; then n=$((n+1)); fi; echo "$n"; }
cat <<VERDICT
{"schema":"forge-warden.verdict.v2","tool":{"name":"forge-warden","version":"0.2.0"},"evaluated_on":"2026-11-15","seals":{"policy_sha256":"$p","rank_sha256":"$f","patch_sha256":"$f","patch_facts_sha256":null,"provenance":"produced","ranked_by":{"version":"stand-in","sha256":"$r"},"git":{"path":"/usr/bin/git","version":"git version 2.56.0","sha256":"$p","admission":"approved","allowlist_sha256":"$f"}},"verdict":"$said","exit":{"code":$code,"name":"$name","action":"$action"},"summary":{"files":1,"rules":2,"passed":$(count pass),"waived":0,"held":$(count hold),"failed":$(count fail)},"rules":[{"id":"no-secrets","kind":"forbid_signal","severity":"fail","outcome":"$o1","why":"A credential committed once is compromised.","violations":[$secrets]},{"id":"no-silent-suppression","kind":"forbid_signal","severity":"hold","outcome":"$o2","why":"A silenced check hides the next real bug.","violations":[$holds]}],"expired_waivers":[],"unused_waivers":[],"notes":[$notes]}
VERDICT
exit "$code"
"#;
    let (script, log) = (scratch.join("listing-judge"), scratch.join("listing.log"));
    write_program(
        &script,
        &SCRIPT.replace("@LOG@", &log.display().to_string()),
    );
    (script, log)
}

/// Write `count` lines that the listing judge counts as key-like values into `name`.
fn secrets(root: &Path, name: &str, count: usize) {
    use std::fmt::Write as _;
    let mut text = String::new();
    for n in 0..count {
        let _ = writeln!(text, "const KEY{n}: &str = \"SECRET\";");
    }
    std::fs::write(root.join(name), text).unwrap();
}

/// Seventh independent review of the hooks, HR7-M-1: "the same failure" is its lines. The
/// identity of a unit's failure was one digest over all of them, so repairing one of two
/// failing things made the other block again, untouched and already told to a person; and
/// a staged failure that the working tree's failure covered for a while was forgotten, and
/// blocked a second time when the tree was repaired again.
///
/// | at this stop | the stop |
/// | --- | --- |
/// | two failing things (`x.rs` with two lines, `y.rs`) | blocks |
/// | nothing changed | stands |
/// | `y.rs` is repaired | stands: `x.rs` is as it was |
/// | `z.rs` is added, failing | blocks, and says that `z.rs` is new and `x.rs` stands |
/// | the count of `x.rs` falls (two lines to one) | stands |
/// | it rises again | blocks |
/// | `x.rs` is staged, and so is the same line in the index and in the tree | stands |
/// | `x.rs` is repaired in the tree and the repair is not staged | blocks once, for "Staged changes" |
/// | nothing changed | stands |
/// | `x.rs` fails in the tree again | blocks: it had gone from the tree and came back |
/// | `x.rs` is repaired in the tree again, the index as it was | stands: the index was not touched |
/// | everything is committed | blocks once, for the commit: a commit is judged once, by its own change |
/// | nothing changed | stands |
#[test]
fn what_is_left_of_a_failure_does_not_block_again() {
    let scratch = Scratch::new("hook-lines");
    let root = repository(&scratch);
    let (script, _) = listing_judge(&scratch);
    let top = std::fs::canonicalize(&root).unwrap();
    let staged = uncommitted_in(&top).replace("Uncommitted changes", "Staged changes");
    let stands = |row: &str| -> String {
        let said = allowed(&stop_from(&root, "s", &script)).to_owned();
        assert!(
            said.starts_with("deep-diff-forge: FAIL STANDS. "),
            "{row}: {said}"
        );
        said
    };
    let blocks = |row: &str| -> String {
        let decision = stop_from(&root, "s", &script);
        assert_eq!(decision["decision"], "block", "{row}: {decision}");
        decision["reason"].as_str().unwrap().to_owned()
    };
    start(&root, "s");
    secrets(&root, "x.rs", 2);
    secrets(&root, "y.rs", 1);
    let first = blocks("two failing things");
    assert!(
        first.contains("secret_like in \"x.rs\"") && first.contains("secret_like in \"y.rs\""),
        "{first}"
    );
    stands("nothing changed");
    std::fs::remove_file(root.join("y.rs")).unwrap();
    let left = stands("one is repaired");
    assert!(
        left.contains("\"x.rs\"") && !left.contains("y.rs"),
        "{left}"
    );
    secrets(&root, "z.rs", 1);
    let added = blocks("one is added");
    let (new, old) = (
        added
            .find("New at this stop:\n")
            .unwrap_or_else(|| panic!("{added}")),
        added
            .find("Stands beside it, as said before")
            .unwrap_or_else(|| panic!("{added}")),
    );
    assert!(
        new < old
            && added[new..old].contains("\"z.rs\"")
            && !added[new..old].contains("\"x.rs\"")
            && added[old..].contains("- rule no-secrets: \"x.rs\""),
        "{added}"
    );
    secrets(&root, "x.rs", 1);
    stands("the count falls");
    secrets(&root, "x.rs", 2);
    blocks("the count rises again");
    git(&root, &["add", "x.rs"]);
    let moved = stands("the line is staged too");
    assert!(!moved.contains("Staged changes"), "{moved}");
    std::fs::write(root.join("x.rs"), "fn repaired() {}\n").unwrap();
    let index = blocks("repaired in the tree, not staged");
    assert!(
        index.contains(&format!("{staged}: FAIL\n")) && index.contains("stage the repaired file"),
        "{index}"
    );
    stands("the index as it was");
    secrets(&root, "x.rs", 2);
    blocks("it fails in the tree again");
    std::fs::write(root.join("x.rs"), "fn repaired() {}\n").unwrap();
    let again = stands("repaired in the tree again");
    assert!(
        again.contains(&format!("{staged}: FAIL, which stands\n")),
        "{again}"
    );
    // to a commit: what the index holds (the two lines of x.rs) and z.rs
    secrets(&root, "x.rs", 2);
    let commit = commit_all(&root, "with the keys");
    let committed = blocks("committed");
    assert!(
        committed.contains(&format!("Commit {commit}: FAIL\n")),
        "{committed}"
    );
    let after = allowed(&stop_from(&root, "s", &script)).to_owned();
    assert!(
        after.contains(&format!("Commit {commit}: FAIL, which stands\n")),
        "{after}"
    );
}

/// Review V4 round 15, VD4R15-M-2, and the seventh independent review, HR7-H-1 and HR7-L-2:
/// a working tree is known by what it is, not by where it is. A failure was remembered with
/// the tree's path, and its key held the path as a message shows it.
///
/// | what happens to a linked working tree whose failure has blocked | a stop made from inside it then |
/// | --- | --- |
/// | the times of its administrative files set anew (`touch`) | stands |
/// | `git gc`, and `git worktree repair` with nothing to repair | stands |
/// | `git worktree move` | stands: it is the same tree with the same failure |
/// | moved with `mv`, then `git worktree repair` | stands |
/// | `git worktree remove`, then `add` at the same path, with another failure | blocks |
/// | removed and added at the same path, with the same failure word for word | blocks: it is a new tree |
/// | its directory deleted, `git worktree prune`, `add` at the same path, the same file | blocks |
/// | removed for good | nothing is said of it: its failure is not in the project any more |
/// | a second tree, whose long path is shown as the first one's | blocks for its own failure |
#[test]
#[expect(
    clippy::too_many_lines,
    reason = "one table: each thing that can happen to a working tree, in turn"
)]
fn a_working_tree_is_known_by_what_it_is_not_by_where_it_is() {
    let scratch = Scratch::new("hook-tree-instance");
    let root = repository(&scratch);
    let (script, _) = reading_judge(&scratch);
    let add = |path: &Path, branch: &str| {
        git(
            &root,
            &[
                "worktree",
                "add",
                "--quiet",
                "-b",
                branch,
                path.to_str().unwrap(),
            ],
        );
    };
    let stands = |tree: &Path, row: &str| {
        let said = allowed(&stop_from(tree, "s", &script)).to_owned();
        assert!(
            said.starts_with("deep-diff-forge: FAIL STANDS. "),
            "{row}: {said}"
        );
    };
    start(&root, "s");
    let (first, moved, again) = (
        scratch.join("wt"),
        scratch.join("wt-moved"),
        scratch.join("wt-again"),
    );
    add(&first, "b1");
    secret(&first, "k.rs");
    blocked(&stop_from(&first, "s", &script));
    stands(&first, "nothing changed");
    // Review V4 round 16, VD4R16-L-1: the times of the tree's administrative files are set
    // anew, as `touch` and a restore do. The tree was known by the modification time of its
    // `commondir` too, so it became another tree and its unchanged failure blocked again.
    let admin = root.join(".git/worktrees/wt");
    let later = std::fs::metadata(&admin).unwrap().modified().unwrap()
        + std::time::Duration::from_secs(3_600);
    for name in ["commondir", "gitdir", "HEAD", "."] {
        std::fs::File::open(admin.join(name))
            .unwrap()
            .set_modified(later)
            .unwrap();
    }
    stands(&first, "the times of its administrative files set anew");
    git(&root, &["gc", "--quiet"]);
    stands(&first, "git gc");
    git(&root, &["worktree", "repair"]);
    stands(&first, "git worktree repair with nothing to repair");
    // moved by Git, and moved by hand and repaired: the same tree
    git(
        &root,
        &[
            "worktree",
            "move",
            first.to_str().unwrap(),
            moved.to_str().unwrap(),
        ],
    );
    stands(&moved, "git worktree move");
    std::fs::rename(&moved, &again).unwrap();
    git(&root, &["worktree", "repair", again.to_str().unwrap()]);
    stands(&again, "mv and git worktree repair");
    // removed, and a tree made at the same path: another tree, whatever fails in it
    let anew = |branch: &str| {
        git(
            &root,
            &["worktree", "remove", "--force", again.to_str().unwrap()],
        );
        add(&again, branch);
    };
    anew("b2");
    secret(&again, "other.rs");
    blocked(&stop_from(&again, "s", &script));
    stands(&again, "the new tree's own failure");
    anew("b3");
    secret(&again, "other.rs");
    blocked(&stop_from(&again, "s", &script));
    // the directory deleted, the tree pruned, and the same file made again at the same path
    std::fs::remove_dir_all(&again).unwrap();
    git(&root, &["worktree", "prune"]);
    add(&again, "b4");
    secret(&again, "other.rs");
    blocked(&stop_from(&again, "s", &script));
    stands(&again, "after the block");
    // removed for good: a stop from the project has nothing to say of it, and forgets it
    git(
        &root,
        &["worktree", "remove", "--force", again.to_str().unwrap()],
    );
    assert_eq!(stop_from(&root, "s", &script), Value::Null);
    add(&again, "b5");
    secret(&again, "other.rs");
    blocked(&stop_from(&again, "s", &script));

    // two trees whose paths are shown the same: over 120 characters, alike at both ends
    let long = |client: &str| {
        scratch
            .join("c".repeat(40))
            .join(client)
            .join("m".repeat(80))
            .join("tree")
    };
    let (one, two) = (long("client-aaaa"), long("client-bbbb"));
    for (tree, branch) in [(&one, "l1"), (&two, "l2")] {
        std::fs::create_dir_all(tree.parent().unwrap()).unwrap();
        add(tree, branch);
    }
    start(&root, "long");
    secret(&one, "k.rs");
    blocked(&stop_from(&one, "long", &script));
    secret(&two, "k.rs");
    blocked(&stop_from(&two, "long", &script));
    assert!(
        allowed(&stop_from(&one, "long", &script)).starts_with("deep-diff-forge: FAIL STANDS. ")
    );
}

/// The lines of the session's file of judged commits, by commit: what each is written down
/// as. One session's file is expected.
fn judged_lines() -> std::collections::BTreeMap<String, String> {
    let files: Vec<PathBuf> = std::fs::read_dir(sessions())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|kind| kind == "judged"))
        .collect();
    assert!(files.len() <= 1, "{files:?}");
    files
        .first()
        .map(|file| std::fs::read_to_string(file).unwrap())
        .unwrap_or_default()
        .lines()
        .filter_map(|line| line.split_once(':'))
        .map(|(id, said)| (id.to_owned(), said.to_owned()))
        .collect()
}

/// A stand-in for Git, in a directory to put in front of `PATH`: it runs `before` (a piece
/// of shell that sees the arguments) and then the real Git with the same arguments. The
/// stand-in is what makes a stop slow where a test needs a slow one: a large repository's
/// `git status`, not the judge.
fn git_with(scratch: &Scratch, name: &str, before: &str) -> PathBuf {
    let real = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|dir| dir.join("git"))
        .find(|path| path.is_file())
        .expect("git is on PATH");
    let dir = scratch.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    write_program(
        &dir.join("git"),
        &format!("#!/bin/sh\n{before}\nexec '{}' \"$@\"\n", real.display()),
    );
    dir
}

/// A stop of `session` from `cwd` with the directory `first` in front of `PATH`.
fn stop_with_path(first: &Path, cwd: &Path, session: &str, judge: &Path) -> Value {
    let mut dirs = vec![first.to_path_buf()];
    dirs.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    let (code, stdout, stderr) = hook_in(
        "stop",
        &["--judge", judge.to_str().unwrap()],
        &input("Stop", cwd, session, false),
        |command| {
            command.env("PATH", std::env::join_paths(dirs).unwrap());
        },
    );
    assert_eq!((code, stderr.as_str()), (0, ""), "{stdout}");
    if stdout.is_empty() {
        Value::Null
    } else {
        serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("{e}: {stdout:?}"))
    }
}

/// Eighth independent review of the hooks, HR8-M-1, as reproduced, and the rows it asks for.
/// A linked working tree whose paths Git records as relative ones (`git worktree add
/// --relative-paths`, `worktree.useRelativePaths`, `git worktree repair --relative-paths`;
/// Git 2.48 and later) is the same tree from outside as from inside. The path in its
/// `gitdir` file was used as it stood, from the hook's own directory, so from the project
/// the tree had another identity: the stop forgot what had blocked there, did not say that
/// a failure stands, and the unchanged failure blocked again at every return.
///
/// | how the tree's paths are recorded | in the tree, from the project, back in the tree |
/// | --- | --- |
/// | made with `--relative-paths` | stands; "a failure stands there"; stands |
/// | made with `worktree.useRelativePaths=true` | the same |
/// | made absolute, then `git worktree repair --relative-paths` | the same: it is the same tree |
/// | and back, `git worktree repair --no-relative-paths` | the same |
#[test]
#[expect(
    clippy::too_many_lines,
    reason = "one table: each way a tree comes to have relative paths, in turn"
)]
fn a_working_tree_recorded_by_relative_paths_is_the_same_tree_from_outside() {
    let scratch = Scratch::new("hook-relative-paths");
    let root = repository(&scratch);
    let (script, _) = reading_judge(&scratch);
    let tree = |name: &str| scratch.join(name);
    let tried = Command::new("git")
        .current_dir(&root)
        .envs(ISOLATE)
        .args(["worktree", "add", "--quiet", "--relative-paths", "-b", "r1"])
        .arg(tree("rel"))
        .output()
        .unwrap();
    if !tried.status.success() {
        // a Git before 2.48 has no relative paths for working trees: nothing to hold
        assert!(
            String::from_utf8_lossy(&tried.stderr).contains("relative-paths"),
            "{}",
            String::from_utf8_lossy(&tried.stderr)
        );
        return;
    }
    let recorded = |name: &str| -> String {
        std::fs::read_to_string(root.join(".git/worktrees").join(name).join("gitdir")).unwrap()
    };
    start(&root, "s");
    let stands = |at: &Path, row: &str| {
        let said = allowed(&stop_from(at, "s", &script)).to_owned();
        assert!(
            said.starts_with("deep-diff-forge: FAIL STANDS. "),
            "{row}: {said}"
        );
    };
    // from the project: every tree where a failure has blocked is said to hold one
    let outside = |standing: usize, row: &str| {
        let said = allowed(&stop_from(&root, "s", &script)).to_owned();
        assert_eq!(
            said.matches("A failure there has blocked a stop of this session and stands")
                .count()
                + said.matches("(a failure stands there)").count(),
            standing,
            "{row}: {said}"
        );
    };
    let row = |name: &str, standing: usize, how: &str| {
        secret(&tree(name), "k.rs");
        blocked(&stop_from(&tree(name), "s", &script));
        stands(&tree(name), how);
        outside(standing, how);
        stands(&tree(name), how);
        outside(standing, how);
        stands(&tree(name), how);
    };
    assert!(recorded("rel").starts_with("../"), "{}", recorded("rel"));
    row("rel", 1, "made with --relative-paths");

    git(&root, &["config", "worktree.useRelativePaths", "true"]);
    git(
        &root,
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "r2",
            tree("cfg").to_str().unwrap(),
        ],
    );
    git(&root, &["config", "--unset", "worktree.useRelativePaths"]);
    assert!(recorded("cfg").starts_with("../"), "{}", recorded("cfg"));
    row("cfg", 2, "made with worktree.useRelativePaths");

    // made absolute; then its paths are written as relative ones, and as absolute ones again
    git(
        &root,
        &[
            "worktree",
            "add",
            "--quiet",
            "--no-relative-paths",
            "-b",
            "r3",
            tree("abs").to_str().unwrap(),
        ],
    );
    assert!(recorded("abs").starts_with('/'), "{}", recorded("abs"));
    row("abs", 3, "made with absolute paths");
    git(
        &root,
        &[
            "worktree",
            "repair",
            "--relative-paths",
            tree("abs").to_str().unwrap(),
        ],
    );
    assert!(recorded("abs").starts_with("../"), "{}", recorded("abs"));
    stands(&tree("abs"), "repair --relative-paths");
    outside(3, "repair --relative-paths");
    stands(&tree("abs"), "repair --relative-paths, back in the tree");
    git(
        &root,
        &[
            "worktree",
            "repair",
            "--no-relative-paths",
            tree("abs").to_str().unwrap(),
        ],
    );
    assert!(recorded("abs").starts_with('/'), "{}", recorded("abs"));
    stands(&tree("abs"), "repair --no-relative-paths");
    outside(3, "repair --no-relative-paths");
    stands(&tree("rel"), "the first tree, after all of it");
}

/// Eighth independent review of the hooks, HR8-L-2, as reproduced in small. The project's
/// own change has a credential, and the project has three other working trees in which
/// `git status` is slow (a large tree; here a stand-in Git that sleeps four seconds). A
/// stop looked at the other trees first, each with all the time it had left, and judged its
/// own change with what remained: with sixteen such trees that was nothing, at every stop.
/// The own change is judged first; the other trees come last, in six seconds of their own,
/// and the ones that were not reached are counted and said.
#[test]
fn the_sessions_own_change_is_judged_before_other_trees_are_looked_at() {
    let scratch = Scratch::new("hook-own-first");
    let root = repository(&scratch);
    let (script, log) = reading_judge(&scratch);
    for n in 0..3 {
        git(
            &root,
            &[
                "worktree",
                "add",
                "--quiet",
                "--detach",
                scratch.join(format!("other{n}")).to_str().unwrap(),
            ],
        );
    }
    start(&root, "s");
    secret(&root, "k.rs");
    let slow = git_with(
        &scratch,
        "slow-git",
        &format!(
            "case \" $* \" in *\" status \"*) case \"$(pwd)\" in */other[0-9]) printf 'STATUS\\n' >> '{}'; sleep 4;; esac;; esac",
            log.display()
        ),
    );
    let began = std::time::Instant::now();
    let decision = stop_with_path(&slow, &root, "s", &script);
    let took = began.elapsed();
    let reason = blocked(&decision);
    assert!(
        reason.contains("3 working trees of this project were not reached in that time"),
        "{reason}"
    );
    // the judge was shown the change before any other tree was looked at
    let seen = std::fs::read_to_string(&log).unwrap();
    let (judged, looked) = (
        seen.find("PATCH-NAMES ").expect("the judge was run"),
        seen.find("STATUS\n").expect("another tree was looked at"),
    );
    assert!(judged < looked, "{seen}");
    // and the looking has its own bound: six seconds for all of them, not four for each
    assert!(
        took < std::time::Duration::from_secs(30),
        "the stop took {took:?}"
    );
    // nothing was forgotten or made up for the trees that were not reached: the next stop
    // (with a Git that answers) finds them clean and says nothing of them
    let said = allowed(&stop_from(&root, "s", &script)).to_owned();
    assert!(
        said.starts_with("deep-diff-forge: FAIL STANDS. ") && !said.contains("not looked at"),
        "{said}"
    );
}

/// A session with three commits that have no verdict, for the stops that come to their
/// commits late: `a` and `b` have had more than five attempts and take turns, one a stop
/// (`b` was asked about longest ago); `c` has had one. The judge never answers for any of
/// them, and from now on takes a second and a half for every patch. Gives the full id of
/// each with the file it changes.
fn three_commits_with_no_verdict(
    root: &Path,
    scratch: &Scratch,
    judge: &Path,
) -> Vec<(String, &'static str)> {
    std::fs::write(scratch.join("judge.flaky"), "999\n").unwrap();
    start(root, "s");
    let mut commits = Vec::new();
    for name in ["ka.rs", "kb.rs"] {
        secret(root, name);
        commit_all(root, name);
        commits.push((git(root, &["rev-parse", "HEAD"]).trim().to_owned(), name));
    }
    // five stops count an attempt for both (the judge answers its probe); then each has
    // its turn once
    for _ in 0..7 {
        allowed(&stop_from(root, "s", judge));
    }
    secret(root, "kc.rs");
    commit_all(root, "kc.rs");
    commits.push((git(root, &["rev-parse", "HEAD"]).trim().to_owned(), "kc.rs"));
    allowed(&stop_from(root, "s", judge));
    let written = judged_lines();
    // (`a` had its turn again at the stop that first asked about `c`: `b` is next)
    assert!(
        written[&commits[0].0].starts_with("n7@")
            && written[&commits[1].0].starts_with("n6@")
            && written[&commits[2].0].starts_with("n1@"),
        "{written:?}"
    );
    std::fs::write(scratch.join("judge.sleep"), "1.5\n").unwrap();
    commits
}

/// The judging of a stop in a project with one working tree and no submodules: its limit
/// of 45 seconds less the 4 it keeps for remembering and saying (README, "The order of a
/// stop, and its time").
const JUDGING: f64 = 41.0;
/// With less than this of its judging left a stop asks about no commit (README, the same
/// table).
const ASK_FLOOR: f64 = 3.0;

/// One stop that came to its commits late, and what the test measured of it.
struct Late {
    decision: Value,
    /// The files of the patches the judge was shown at it, in order.
    submitted: Vec<String>,
    /// The session's file of judged commits before the stop and after it.
    before: std::collections::BTreeMap<String, String>,
    after: std::collections::BTreeMap<String, String>,
    /// The most of its judging the stop can have had left when it came to its commits:
    /// [`JUDGING`] less the time from its first call of Git (its own clock began before
    /// that) to the end of the slow one.
    left_at_commits: f64,
    /// The least of its judging the stop can have had left at anything it decided about a
    /// commit: [`JUDGING`] less the time from just before the hook was started (its clock
    /// began after that) to its end (it had decided before that).
    left_at_end: f64,
}

/// One stop that comes to its commits `late` seconds after its first call of Git, because
/// Git is slow: a stand-in whose last call before the commits are asked about does not
/// return before then.
///
/// The stand-in slept a fixed time, counted from wherever the stop had got to. On a busy
/// machine the stop reached that call later, by the seconds its own first steps took
/// there, and came to its commits with less of its judging left than the test meant: a
/// test of "four seconds are left" found three, and failed for the machine's load (review
/// V4 round 19, the two tests that failed in a full run and passed alone). The stand-in
/// waits for a moment on the wall clock instead: `late` seconds after the stop's first
/// call of Git, which this function watches for. What the machine still adds (the hook's
/// own steps before its first call and after the slow one) is measured and returned, so
/// that a test can tell a stop that had its time from one that did not.
fn late_stop(root: &Path, scratch: &Scratch, judge: &Path, log: &Path, late: f64) -> Late {
    use std::time::{Duration, Instant, SystemTime};
    let dir = (0..1_000)
        .map(|n| scratch.join(format!("late-stop.{n}")))
        .find(|dir| !dir.exists())
        .unwrap();
    let (calls, wake, returned) = (dir.join("calls"), dir.join("wake"), dir.join("woke"));
    std::fs::create_dir_all(&calls).unwrap();
    let flag = scratch.join("git.sleep");
    std::fs::write(&flag, "the next listing of commits waits\n").unwrap();
    // Every call leaves a file (when it began is the file's time). The listing of the
    // commits waits until the test says the moment has come; at most 65 seconds.
    let slow = git_with(
        scratch,
        "slow-git",
        &format!(
            ": > '{calls}'/$$\ncase \" $* \" in *\" --topo-order \"*) if mv '{flag}' '{flag}.used' 2>/dev/null; then n=0; while [ ! -e '{wake}' ] && [ \"$n\" -lt 1300 ]; do sleep 0.05; n=$((n + 1)); done; : > '{woke}'; fi;; esac",
            calls = calls.display(),
            flag = flag.display(),
            wake = wake.display(),
            woke = returned.display(),
        ),
    );
    // When the stop's first call of Git began: watched for, for at most a minute.
    let first_call = move |calls: &Path| -> Option<SystemTime> {
        std::fs::read_dir(calls)
            .ok()?
            .filter_map(|entry| entry.ok()?.metadata().ok()?.modified().ok())
            .min()
    };
    let waker = {
        let (calls, wake) = (calls.clone(), wake.clone());
        std::thread::spawn(move || {
            let give_up = Instant::now() + Duration::from_secs(60);
            let first = loop {
                if let Some(first) = first_call(&calls) {
                    break Some(first);
                }
                if Instant::now() >= give_up {
                    break None;
                }
                std::thread::sleep(Duration::from_millis(2));
            };
            if let Some(first) = first
                && let Ok(wait) =
                    (first + Duration::from_secs_f64(late)).duration_since(SystemTime::now())
            {
                std::thread::sleep(wait.min(Duration::from_secs(60)));
            }
            std::fs::write(&wake, "").unwrap();
            first
        })
    };
    let before = judged_lines();
    let shown_before = shown(log).len();
    let started = SystemTime::now();
    let decision = stop_with_path(&slow, root, "s", judge);
    let ended = SystemTime::now();
    let first = waker.join().unwrap().expect("the stop made no call of Git");
    assert!(!flag.exists(), "the stand-in Git was not used");
    let woken = std::fs::metadata(&returned).unwrap().modified().unwrap();
    let seconds = |from: SystemTime, to: SystemTime| {
        to.duration_since(from)
            .map_or(0.0, |time| time.as_secs_f64())
    };
    Late {
        decision,
        submitted: shown(log)[shown_before..].to_vec(),
        before,
        after: judged_lines(),
        left_at_commits: JUDGING - seconds(first, woken),
        left_at_end: JUDGING - seconds(started, ended),
    }
}

/// Of the two commits that take turns (`a` and `b` of [`three_commits_with_no_verdict`]),
/// the file of the one whose turn it is by the session's file of judged commits: the one
/// asked about longest ago (`n<attempts>@<stop>:…`).
fn in_turn(
    commits: &[(String, &'static str)],
    written: &std::collections::BTreeMap<String, String>,
) -> &'static str {
    let asked_at = |id: &String| -> u64 {
        let line = &written[id];
        let stop = line.split_once('@').unwrap().1.split_once(':').unwrap().0;
        stop.parse().unwrap()
    };
    commits[..2]
        .iter()
        .min_by_key(|(id, _)| asked_at(id))
        .unwrap()
        .1
}

/// What the eighth and the ninth independent review ask of a stop that comes to its
/// commits late (HR8-L-1, HR9-L-2), in both directions and commit by commit: the session's
/// file says a commit was asked about at this stop exactly when the judge was shown its
/// patch at this stop.
fn written_is_what_was_asked(
    commits: &[(String, &'static str)],
    submitted: &[String],
    before: &std::collections::BTreeMap<String, String>,
    after: &std::collections::BTreeMap<String, String>,
    decision: &Value,
) {
    for (id, file) in commits {
        let asked = submitted.iter().any(|shown| shown == file);
        assert_eq!(
            after[id] != before[id],
            asked,
            "{file}: written down as {} (before: {}), and the judge was shown {submitted:?}: {decision}",
            after[id],
            before[id]
        );
    }
}

/// Eighth independent review of the hooks, HR8-L-1, and the ninth, HR9-L-2, at the edge
/// they were found at: a stop that comes to its commits with under four seconds of its
/// judging left, and a judge that takes a second and a half. The commit whose turn it is
/// is submitted; the judge answers "no verdict"; under three seconds are left then.
///
/// - The eighth review's build did not start the judge (its time was "what is left less
///   five seconds") and wrote the commit down as asked.
/// - The ninth review's build ran the judge, wrote nothing, and said "not asked at this
///   stop for lack of time": so the commit kept its turn, and a session whose stops all
///   come to their commits like this asked about the same commit at every stop.
///
/// An ask the judge was run for is written down, whatever time is left after it: at the
/// second such stop it is the other commit's turn.
#[test]
fn an_ask_is_written_down_when_it_was_made_and_only_then() {
    let scratch = Scratch::new("hook-ask-made");
    let root = repository(&scratch);
    let (script, log) = reading_judge(&scratch);
    let commits = three_commits_with_no_verdict(&root, &scratch, &script);
    let turns = [commits[1].1, commits[0].1];
    let mut first_asked = Vec::new();
    let mut without_time = 0;
    while first_asked.len() < turns.len() {
        // 41 seconds of judging, and the stop comes to its commits with 4 of them at most
        let late = late_stop(&root, &scratch, &script, &log, 37.0);
        let decision = &late.decision;
        written_is_what_was_asked(
            &commits,
            &late.submitted,
            &late.before,
            &late.after,
            decision,
        );
        assert!(
            late.left_at_commits < JUDGING - 37.0 + 0.05,
            "{}",
            late.left_at_commits
        );
        let Some(first) = late.submitted.first() else {
            // No commit was asked about. That is right only for a stop that had under
            // three seconds when it decided so, and this one decided before it ended: a
            // stop that ended with three seconds of its judging left had the time.
            assert!(
                late.left_at_end < ASK_FLOOR + 0.05,
                "the stop asked about no commit and ended with at least {:.2} seconds of its judging left: {decision}",
                late.left_at_end
            );
            // The machine took the stop's time (its own steps took over a second of the
            // four). Nothing was written and the turn is whose it was: the stop is made
            // again, a few times at most; the test passes only on stops that asked.
            without_time += 1;
            assert!(
                without_time <= 4,
                "five stops came to their commits without the time to ask about one: {decision}"
            );
            continue;
        };
        assert_eq!(*first, in_turn(&commits, &late.before), "{decision}");
        // the commit that was not reached says so, and keeps what it had
        if !late.submitted.iter().any(|file| file == "kc.rs") {
            assert!(
                decision
                    .to_string()
                    .contains("not asked at this stop for lack of time"),
                "{decision}"
            );
        }
        first_asked.push(first.clone());
    }
    // the turn moves on: `b`, then `a`
    assert_eq!(first_asked, turns, "the same commit was asked about twice");
    println!("stops made again for lack of time: {without_time}");
}

/// The other two edges of a stop that comes to its commits late (ninth independent review
/// of the hooks, the table under HR9-L-2). With under three seconds left no commit is
/// asked about and nothing is written: each is said not to have been asked. With over
/// seven seconds the commit whose turn it is and the one with fewer attempts are both
/// asked about, and both are written down.
#[test]
fn a_stop_with_no_time_asks_nothing_and_one_with_seven_seconds_asks_both() {
    let scratch = Scratch::new("hook-ask-edges");
    let root = repository(&scratch);
    let (script, log) = reading_judge(&scratch);
    let commits = three_commits_with_no_verdict(&root, &scratch, &script);
    let mut again = 0;
    // Under two seconds of judging left when the commits are listed: that is measured,
    // and holds whatever the machine does (it can only take more of the stop's time).
    loop {
        let late = late_stop(&root, &scratch, &script, &log, 39.2);
        let decision = &late.decision;
        written_is_what_was_asked(
            &commits,
            &late.submitted,
            &late.before,
            &late.after,
            decision,
        );
        assert!(
            late.left_at_commits < JUDGING - 39.2 + 0.05,
            "{}",
            late.left_at_commits
        );
        assert_eq!(late.submitted, Vec::<String>::new(), "{decision}");
        assert_eq!(late.before, late.after);
        // (a stop whose time ran out while its commits were still being listed did not
        // come to them at all: it says that, and is made again)
        if decision.to_string().contains("could not be listed") {
            again += 1;
            assert!(again <= 3, "{decision}");
            continue;
        }
        assert!(
            decision
                .to_string()
                .contains("not asked at this stop for lack of time"),
            "{decision}"
        );
        break;
    }
    // Over seven seconds left: the commit whose turn it is, then `c`.
    loop {
        let late = late_stop(&root, &scratch, &script, &log, 33.5);
        let decision = &late.decision;
        written_is_what_was_asked(
            &commits,
            &late.submitted,
            &late.before,
            &late.after,
            decision,
        );
        if let Some(first) = late.submitted.first() {
            assert_eq!(*first, in_turn(&commits, &late.before), "{decision}");
        }
        if late.submitted.len() >= 2 {
            assert_eq!(late.submitted[1], commits[2].1, "{decision}");
            break;
        }
        // Fewer than both were asked about. That is right only for a stop that ran out
        // of its time: one that ended with three seconds of its judging left had the
        // time for the next ask. Otherwise the machine took it, and the stop is made
        // again (the turn has moved on if one was asked; `c` is still the other).
        assert!(
            late.left_at_end < ASK_FLOOR + 0.05,
            "{:?} only, and the stop ended with at least {:.2} seconds of its judging left: {decision}",
            late.submitted,
            late.left_at_end
        );
        again += 1;
        assert!(
            again <= 4,
            "the stops came to their commits without the time the test needs: {decision}"
        );
    }
    println!("stops made again for lack of time: {again}");
}

/// Review V4 round 17, VD4R17-L-1, as reproduced. The session began in the project's main
/// working tree and its directory is a linked working tree now; both have a change. The
/// project's tree was read and judged first whatever the stop was in, so a slow run there
/// left the stop's own tree about five seconds, and it was UNJUDGED at every such stop. The
/// tree the stop is in comes first; the project's tree is judged after it.
#[test]
fn the_tree_a_stop_is_in_is_judged_before_the_projects_tree() {
    let scratch = Scratch::new("hook-own-tree-first");
    let root = repository(&scratch);
    let (script, log) = reading_judge(&scratch);
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
    start(&root, "s");
    secret(&root, "main.rs");
    secret(&linked, "linked.rs");
    blocked(&stop_from(&linked, "s", &script));
    let submitted: Vec<String> = shown(&log)
        .into_iter()
        .filter(|file| Path::new(file).extension().is_some_and(|kind| kind == "rs"))
        .collect();
    assert_eq!(submitted, ["linked.rs", "main.rs"]);
    // from the project's own tree the project's tree is the one the stop is in
    start(&root, "from-main");
    let before = shown(&log).len();
    blocked(&stop_from(&root, "from-main", &script));
    assert_eq!(shown(&log)[before..], ["main.rs".to_owned()]);

    // The slow case: the judge takes a minute for the main tree's patch and six seconds
    // for the linked tree's. The linked tree, which the stop is in, gets its verdict and
    // blocks; the main tree's run is ended when its time is up and is said to have no
    // verdict.
    std::fs::write(
        root.join("main.rs"),
        "const KEY: &str = \"SECRET\"; // SLOWMAIN\n",
    )
    .unwrap();
    std::fs::write(
        linked.join("linked.rs"),
        "const KEY: &str = \"SECRET\"; // SLOWLINKED\n",
    )
    .unwrap();
    let slow = scratch.join("slow-judge");
    write_program(
        &slow,
        &format!(
            "#!/bin/sh\npatch=; prev=\nfor a in \"$@\"; do if [ \"$prev\" = --patch ]; then patch=\"$a\"; fi; prev=\"$a\"; done\nif [ -n \"$patch\" ] && grep -q '^+.*SLOWMAIN' \"$patch\"; then sleep 60; fi\nif [ -n \"$patch\" ] && grep -q '^+.*SLOWLINKED' \"$patch\"; then sleep 6; fi\nexec '{}' \"$@\"\n",
            script.display()
        ),
    );
    start(&root, "slow");
    let began = std::time::Instant::now();
    let reason = blocked(&stop_from(&linked, "slow", &slow)).to_owned();
    assert!(
        began.elapsed() < std::time::Duration::from_secs(44),
        "{:?}",
        began.elapsed()
    );
    let top = |tree: &Path| std::fs::canonicalize(tree).unwrap();
    assert!(
        reason.contains(&format!("{}: FAIL\n", uncommitted_in(&top(&linked))))
            && reason.contains(&format!("{}: UNJUDGED\n", uncommitted_in(&top(&root)))),
        "{reason}"
    );
}

/// Ninth independent review of the hooks, HR9-M-1, as reproduced: 257 failing files have
/// blocked; then one of them is staged, as it is; then two more; then all. With more than
/// 256 lines every partial `git add` blocked again, as "Staged changes", for files that had
/// not changed and had already blocked (the staged lines were compared with the tree's
/// failure as it is remembered, which past 256 lines is one mark and covers no single
/// line). Staging does not make a failure block again, whatever its size. And HR9-L-3,
/// the second part: when the failure then grows by two files, the block says which are new
/// and that the others stand; it was a plain "FAIL" of all 259.
#[test]
fn staging_a_part_of_a_large_failure_does_not_block_again() {
    let scratch = Scratch::new("hook-stage-part");
    let root = repository(&scratch);
    let (script, _) = listing_judge(&scratch);
    start(&root, "s");
    std::fs::create_dir(root.join("g")).unwrap();
    for file in 0..257 {
        secrets(&root, &format!("g/k{file}.rs"), 1);
    }
    blocked(&stop_from(&root, "s", &script));
    let stands = |row: &str| {
        let said = allowed(&stop_from(&root, "s", &script)).to_owned();
        assert!(
            said.starts_with("deep-diff-forge: FAIL STANDS. ")
                && !said.contains("Staged changes in ")
                && !said.contains("could not be kept"),
            "{row}: {said}"
        );
    };
    stands("nothing staged");
    git(&root, &["add", "g/k1.rs"]);
    stands("one of the 257 staged");
    git(&root, &["add", "g/k2.rs", "g/k3.rs"]);
    stands("two more staged");
    git(&root, &["add", "--all"]);
    stands("all staged");
    git(&root, &["restore", "--staged", "g/k5.rs"]);
    stands("one taken out of the index again");
    // The failure grows by two files. It is remembered by a digest (257 lines), so the
    // block is for all of it and says that it is not all new.
    secrets(&root, "g/new-a.rs", 1);
    secrets(&root, "g/new-b.rs", 1);
    let reason = blocked(&stop_from(&root, "s", &script)).to_owned();
    assert!(
        reason.contains("A failure here has blocked before and is not what it was")
            && !reason.contains("New at this stop"),
        "{reason}"
    );
    stands("after the block");
}

/// Ninth independent review of the hooks, HR9-L-3, the second part: a failure of 256 lines
/// is remembered by its lines; it grows by two files, past the number of lines that are
/// kept one by one. The block was a plain "FAIL" of all 258, with no word that 256 of them
/// had been said: the new verdict was folded into a digest before it was compared. The
/// lines are compared one by one whatever their number, so the block says which are new
/// and that the others stand.
#[test]
fn a_failure_that_grows_past_the_lines_kept_says_what_is_new() {
    let scratch = Scratch::new("hook-grows-past");
    let root = repository(&scratch);
    let (script, _) = listing_judge(&scratch);
    start(&root, "s");
    std::fs::create_dir(root.join("g")).unwrap();
    for file in 0..256 {
        secrets(&root, &format!("g/k{file}.rs"), 1);
    }
    blocked(&stop_from(&root, "s", &script));
    secrets(&root, "g/new-a.rs", 1);
    secrets(&root, "g/new-b.rs", 1);
    let reason = blocked(&stop_from(&root, "s", &script)).to_owned();
    assert!(
        reason.contains("New at this stop:\n")
            && reason.contains("Stands beside it, as said before")
            && reason.contains("g/new-a.rs")
            && reason.contains("g/new-b.rs")
            && !reason.contains("is not what it was"),
        "{reason}"
    );
    assert!(allowed(&stop_from(&root, "s", &script)).starts_with("deep-diff-forge: FAIL STANDS. "));
}

/// Ninth independent review of the hooks, HR9-L-3, the first part. What "stands" means for
/// a change that is not committed (the judge names the same rules on the same paths as
/// many times; the hook does not know whether they are the same lines) was said under the
/// head FAIL STANDS alone. A failure that stands is the same thing under any head: here
/// the stop's head is HOLD, because the session also has a commit the judge holds.
#[test]
fn what_stands_means_is_said_under_every_head() {
    const MEANS: &str = "Of a change that is not committed, \"stands\" and \"has blocked before\" mean: the judge names the same rules on the same paths as it did then, as many times or fewer. Whether they are the same lines of those files the hook does not know.";
    let scratch = Scratch::new("hook-stands-means");
    let root = repository(&scratch);
    let (script, _) = listing_judge(&scratch);
    start(&root, "s");
    secrets(&root, "k.rs", 1);
    let first = blocked(&stop_from(&root, "s", &script)).to_owned();
    assert!(
        !first.contains(MEANS),
        "nothing has blocked before: {first}"
    );
    let alone = allowed(&stop_from(&root, "s", &script)).to_owned();
    assert!(
        alone.starts_with("deep-diff-forge: FAIL STANDS. ") && alone.contains(MEANS),
        "{alone}"
    );
    // a commit the judge holds, beside the failure that stands
    std::fs::write(root.join("h.rs"), "// HOLDME\n").unwrap();
    git(&root, &["add", "h.rs"]);
    git(&root, &["commit", "--quiet", "-m", "held"]);
    let held = allowed(&stop_from(&root, "s", &script)).to_owned();
    assert!(
        held.starts_with("deep-diff-forge: HOLD. ")
            && held.contains("FAIL, which stands")
            && held.contains(MEANS),
        "{held}"
    );
    // and in a block, where what has blocked before stands beside what is new
    secrets(&root, "k2.rs", 1);
    let block = blocked(&stop_from(&root, "s", &script)).to_owned();
    assert!(
        block.contains("Stands beside it, as said before") && block.contains(MEANS),
        "{block}"
    );
}

/// A stated limit, and review V4 round 18, VD4R18-L-1: a repository with more than 256
/// working trees. The hook does not read the history of so many (the commits of a session
/// are found from the `HEAD` and the reflog of every working tree), so the session's
/// commits cannot be told from the ones that were there, and that is said at every stop.
/// What is not committed is judged all the same, whenever the session began:
///
/// - A session that began while the repository had 256 or fewer.
/// - A session that begins in such a repository. Its start could not list the commits that
///   were there and recorded nothing at all, so every stop met the same failure and was
///   wholly UNJUDGED: a credential in the tree the stop is in was never put to the judge.
///   The project and the base are recorded, with the fact that there is no list.
#[test]
#[expect(
    clippy::too_many_lines,
    reason = "one session's stops in such a repository, then what the gate and the state say of it"
)]
fn a_repository_with_more_working_trees_than_are_read_says_so() {
    const WHY: &str = "the repository has more working trees than a hook reads the history of";
    let scratch = Scratch::new("hook-many-trees");
    let root = repository(&scratch);
    let (script, _) = reading_judge(&scratch);
    // (a long path: nothing of it may take the room of the sentence checked below)
    let trees = scratch.join(format!("trees-{}", "t".repeat(140)));
    std::fs::create_dir(&trees).unwrap();
    let add = |n: usize| {
        git(
            &root,
            &[
                "worktree",
                "add",
                "--quiet",
                "--detach",
                "--no-checkout",
                trees.join(format!("t{n}")).to_str().unwrap(),
            ],
        );
    };
    // 256 working trees with the project's own: as many as are read
    for n in 0..255 {
        add(n);
    }
    start(&root, "before");
    add(255);
    let started = start(&root, "after");
    assert!(
        started.contains("deep-diff-forge is watching the changes of this session")
            && started.contains(WHY),
        "{started}"
    );
    // nothing changed and nothing committed: not silent, in neither session
    for session in ["before", "after"] {
        let said = allowed(&stop_from(&root, session, &script)).to_owned();
        assert!(
            said.starts_with("deep-diff-forge: UNJUDGED. ")
                && said.contains(WHY)
                && !said.contains("invalid Git selection"),
            "{session}: {said}"
        );
    }
    secret(&root, "k.rs");
    for session in ["before", "after"] {
        let reason = blocked(&stop_from(&root, session, &script)).to_owned();
        assert!(
            reason.contains("- rule no-secrets")
                && reason.contains("The commits of this session: UNJUDGED")
                && reason.contains(WHY),
            "{session}: {reason}"
        );
        let stands = allowed(&stop_from(&root, session, &script)).to_owned();
        assert!(
            stands.contains("FAIL, which stands")
                && stands.contains("The commits of this session: UNJUDGED"),
            "{session}: {stands}"
        );
        // The reason is whole, what follows from it first: it was cut after "they are
        // not" and closed with a full stop, so the stop never said that the commits are
        // not judged whatever is committed, nor that what is not committed is (eleventh
        // independent review of the hooks, HR11-L-3).
        let line = stands
            .lines()
            .skip_while(|line| *line != "The commits of this session: UNJUDGED")
            .nth(1)
            .unwrap_or_default();
        let (head, tail) = if session == "after" {
            (
                "  not judged: not in this session, whatever is committed (what is not committed is judged): the commits that were there when it began could not be listed (",
                "forgets the entries of trees that are gone)).",
            )
        } else {
            // (this session has its list; what it made since cannot be listed)
            (
                "  not judged: the commits made since the session began could not be listed: ",
                "forgets the entries of trees that are gone).",
            )
        };
        assert!(
            line.starts_with(head) && line.ends_with(tail) && !line.contains('…'),
            "{session}: {line:?}"
        );
    }
    // The gate says what a session here is: recorded, with its commits UNJUDGED and what is
    // not committed judged (a mutation run on `07df120` took that arm away, and nothing
    // read the detail).
    let (_, doc, _) = doctor_gate(&root, &home(), &[]);
    let commits = doc["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|check| check["name"] == "commits")
        .unwrap();
    assert!(
        commits["ok"] == false
            && commits["detail"].as_str().unwrap().starts_with(
                "the commits of a session here would be UNJUDGED at every stop (what is not committed is judged): "
            )
            && commits["detail"].as_str().unwrap().contains(WHY),
        "{commits}"
    );
    // the state says that there is no list, and names none
    let kept: Vec<Value> = states_in(&sessions())
        .iter()
        .map(|state| serde_json::from_str(&std::fs::read_to_string(state).unwrap()).unwrap())
        .collect();
    assert_eq!(
        kept.iter()
            .filter(|state| state["project"]["no_baseline"].is_string()
                && state["project"]["tips_count"] == 0)
            .count(),
        1,
        "{kept:?}"
    );
}

/// A bare repository at `repo.git` under `scratch`, with a working tree inside it for
/// each name, on a branch of that name: (the bare repository, the trees).
fn bare_with_trees(scratch: &Scratch, names: &[&str]) -> (PathBuf, Vec<PathBuf>) {
    let origin = repository(scratch);
    let bare = scratch.join("repo.git");
    git(
        scratch.path(),
        &[
            "clone",
            "--quiet",
            "--bare",
            origin.to_str().unwrap(),
            bare.to_str().unwrap(),
        ],
    );
    let trees = names
        .iter()
        .map(|name| {
            let tree = bare.join(name);
            git(
                &bare,
                &[
                    "worktree",
                    "add",
                    "--quiet",
                    "-b",
                    &format!("branch-{name}"),
                    tree.to_str().unwrap(),
                ],
            );
            tree
        })
        .collect();
    (bare, trees)
}

/// Eleventh independent review of the hooks, HR11-M-1, by its two reproductions. In a bare
/// repository with its working trees inside it every tree is under the Git common
/// directory, and every failure of Git for a directory there was read as "one of Git's own
/// files":
///
/// - A tree that was pruned while its directory was away (its `.git` file names an
///   administrative directory that is gone), with credentials in it: a stop made from
///   inside it printed nothing. It is UNJUDGED, with Git's own words, as it is in the
///   ordinary layout.
/// - A Git that fails for the tree at one stop: the stop was not UNJUDGED, and said of the
///   tree the session is in that "the session's directory is not in it". It is UNJUDGED,
///   the tree is not called one the session is not in, and nothing of it is forgotten.
///
/// A directory inside the Git directory is not taken for Git's own either: Git cannot place
/// it, and a stop from it is UNJUDGED with Git's words (review V4 round 21, VD4R21-M-1).
#[test]
fn a_tree_git_cannot_place_is_unjudged_and_never_taken_for_gits_own_files() {
    let scratch = Scratch::new("hook-bare-unplaced");
    let (bare, trees) = bare_with_trees(&scratch, &["main", "feature", "other"]);
    let (main, feature, other) = (&trees[0], &trees[1], &trees[2]);
    // "the session's directory is not in it", said of `tree` (it is said, truly, of a tree
    // the stop is not in)
    let not_in = |said: &str, tree: &Path| {
        said.contains(&format!(
            "{}\" of this project has changes that are not committed, and the session's directory is not in it",
            std::fs::canonicalize(tree).unwrap().display()
        ))
    };
    let (script, log) = reading_judge(&scratch);
    start(main, "s");
    for tree in [feature, other] {
        secret(tree, "key.rs");
        blocked(&stop_from(tree, "s", &script));
    }

    // `feature` is pruned while its directory is away, and comes back with one more key
    let away = scratch.join("feature-away");
    std::fs::rename(feature, &away).unwrap();
    git(&bare, &["worktree", "prune"]);
    std::fs::rename(&away, feature).unwrap();
    secret(feature, "second.rs");
    assert!(feature.join(".git").is_file());
    for cwd in [feature.clone(), feature.join("deep-in-it")] {
        std::fs::create_dir_all(&cwd).unwrap();
        let said = allowed(&stop_from(&cwd, "s", &script)).to_owned();
        assert!(
            said.starts_with("deep-diff-forge: UNJUDGED. ")
                && said.contains("could not be found out")
                && !not_in(&said, feature),
            "{}: {said}",
            cwd.display()
        );
    }

    // Git fails for `other` at one stop (from its top, and from a directory inside it)
    let failing = git_with(
        &scratch,
        "failing-git",
        "case \" $* \" in *\" rev-parse \"*) case \"$(pwd) $*\" in */repo.git/other*) echo 'fatal: made to fail by the test' >&2; exit 128;; esac;; esac",
    );
    std::fs::create_dir(other.join("deep")).unwrap();
    for (n, cwd) in [other.clone(), other.join("deep")].into_iter().enumerate() {
        let before = shown(&log).len();
        let said = allowed(&stop_with_path(&failing, &cwd, "s", &script)).to_owned();
        assert!(
            said.starts_with("deep-diff-forge: UNJUDGED. ")
                && said.contains("could not be found out")
                && !not_in(&said, other),
            "{}: {said}",
            cwd.display()
        );
        // the tree is named for what it is here, with the failure that stands in it: in
        // full the first time, and on the line that counts such things at every later stop
        if n == 0 {
            assert!(
                said.contains("whether it is in that tree could not be found out at this stop")
                    && said
                        .contains("A failure there has blocked a stop of this session and stands"),
                "{said}"
            );
        } else {
            assert!(
                said.contains("(the session's directory may be in it) (a failure stands there)"),
                "{said}"
            );
        }
        assert_eq!(
            shown(&log)[before..],
            Vec::<String>::new(),
            "a patch of a tree that was not found was put to the judge"
        );
    }
    // nothing of it was forgotten: with a Git that answers, the failure stands
    let stands = allowed(&stop_from(other, "s", &script)).to_owned();
    assert!(
        stands.starts_with("deep-diff-forge: FAIL STANDS. "),
        "{stands}"
    );

    // a directory inside the Git directory is no more Git's own than any other that Git
    // cannot place: UNJUDGED with Git's words, said to be inside the Git directory, and the
    // failure that stands in `other` is named
    let failing = git_with(
        &scratch,
        "failing-git-2",
        "case \" $* \" in *\" rev-parse \"*) case \"$(pwd) $*\" in */repo.git/objects*|*/repo.git/worktrees/*) echo 'fatal: made to fail by the test' >&2; exit 128;; esac;; esac",
    );
    for inside in [bare.join("objects"), bare.join("worktrees/other")] {
        let said = allowed(&stop_with_path(&failing, &inside, "s", &script)).to_owned();
        assert!(
            said.starts_with("deep-diff-forge: UNJUDGED. ")
                && said.contains("fatal: made to fail by the test. Whether the session's directory is another working tree could not be found out: it is inside the Git directory")
                && said.contains("(a failure stands there)"),
            "{}: {said}",
            inside.display()
        );
    }
}

/// What the session's state remembers of failures that have blocked (its only session).
fn remembered_lines() -> Vec<String> {
    let state = states_in(&sessions());
    assert_eq!(state.len(), 1, "{state:?}");
    let kept: Value = serde_json::from_str(&std::fs::read_to_string(&state[0]).unwrap()).unwrap();
    kept["blocked"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry.as_str().unwrap().to_owned())
        .collect()
}

/// Remove both records of the linked tree `tree`: its `.git` file and its administrative
/// directory. Its directory and its files stay.
fn remove_both_records(tree: &Path) {
    let pointer = std::fs::read_to_string(tree.join(".git")).unwrap();
    let admin = PathBuf::from(pointer.trim_end().strip_prefix("gitdir: ").unwrap());
    let admin = if admin.is_absolute() {
        admin
    } else {
        tree.join(admin)
    };
    assert!(admin.is_dir(), "{}", admin.display());
    std::fs::remove_file(tree.join(".git")).unwrap();
    std::fs::remove_dir_all(&admin).unwrap();
}

/// A stop from `cwd`, a directory Git cannot place as a working tree ("must be run in a work
/// tree"): UNJUDGED, Git's words first, never "Git's own", and nothing remembered forgotten.
fn unplaced_stop(cwd: &Path, session: &str, script: &Path, inside_git_dir: bool) -> String {
    let before = remembered_lines();
    let said = allowed(&stop_from(cwd, session, script)).to_owned();
    assert!(
        said.starts_with("deep-diff-forge: UNJUDGED. ")
            && said.contains(
                "`git rev-parse` failed: fatal: this operation must be run in a work tree. "
            )
            && said.contains("could not be found out")
            && !said.contains("Git's own"),
        "{}: {said}",
        cwd.display()
    );
    assert_eq!(
        said.contains("could not be found out: it is inside the Git directory"),
        inside_git_dir,
        "{}: {said}",
        cwd.display()
    );
    assert_eq!(remembered_lines(), before, "{}", cwd.display());
    said
}

/// Review V4 round 20, VD4R20-M-1, as reproduced. In a bare repository with its working
/// trees inside it, a linked tree's `.git` file and its administrative directory are both
/// removed while its directory and its files stay, and a second key is written there. Git
/// cannot place the directory ("must be run in a work tree"), and a directory under the Git
/// directory with no `.git` entry above it was taken for one of Git's own files: the stop
/// made from it printed nothing, and the failure that had blocked there was forgotten. That
/// no `.git` entry is there proves nothing, and neither does where it lies (review V4 round
/// 21, VD4R21-M-1): the stop is UNJUDGED with Git's words, the second key is put to no
/// judge, and nothing remembered is forgotten. So is a stop from the Git directory itself
/// and from the directories Git makes in it: Git cannot place them either.
#[test]
fn a_tree_whose_two_records_were_removed_is_unjudged_and_keeps_what_blocked_there() {
    let scratch = Scratch::new("hook-bare-records-gone");
    let (bare, trees) = bare_with_trees(&scratch, &["main", "feature"]);
    let (main, feature) = (&trees[0], &trees[1]);
    let (script, log) = reading_judge(&scratch);
    start(main, "s");
    secret(feature, "first.rs");
    blocked(&stop_from(feature, "s", &script));
    assert_ne!(
        remembered_lines(),
        Vec::<String>::new(),
        "nothing was remembered"
    );

    remove_both_records(feature);
    secret(feature, "second.rs");
    std::fs::create_dir(feature.join("deep")).unwrap();
    // a plain directory someone made in the bare repository is not known to be Git's own
    // either: nothing tells it from the tree above
    let plain = bare.join("plain");
    std::fs::create_dir(&plain).unwrap();
    for cwd in [
        feature.clone(),
        feature.join("deep"),
        feature.clone(),
        plain,
        bare.clone(),
        bare.join("refs/heads"),
        bare.join("objects"),
    ] {
        unplaced_stop(&cwd, "s", &script, true);
    }
    assert!(
        !shown(&log).iter().any(|names| names.contains("second.rs")),
        "{:?}",
        shown(&log)
    );
}

/// Review V4 round 21, VD4R21-M-1, and the thirteenth independent review of the hooks,
/// HR13-L-1, as reproduced. `git worktree add` puts a linked tree at `repo.git/hooks/feature`,
/// `repo.git/logs/feat` or `repo.git/svn`, under names Git uses for directories of its own.
/// With the tree's two records removed, a directory under one of seventeen such names was
/// taken for one of Git's own files: the stop from the tree printed nothing with a second key
/// in it, and the failure that had blocked there was forgotten. Nothing on disk tells such a
/// tree from Git's own files any more: the stop is UNJUDGED with Git's words, the second key
/// is put to no judge, nothing remembered is forgotten, and it is never silent.
#[test]
fn a_tree_under_a_name_git_uses_whose_records_were_removed_is_unjudged() {
    for (n, name) in ["hooks/feature", "logs/feat", "svn"]
        .into_iter()
        .enumerate()
    {
        let scratch = Scratch::new(&format!("hook-bare-named-{n}"));
        let (_, trees) = bare_with_trees(&scratch, &["main", name]);
        let (main, tree) = (&trees[0], &trees[1]);
        let (script, log) = reading_judge(&scratch);
        start(main, "s");
        secret(tree, "first.rs");
        blocked(&stop_from(tree, "s", &script));
        assert_ne!(remembered_lines(), Vec::<String>::new(), "{name}");
        remove_both_records(tree);
        secret(tree, "second.rs");
        std::fs::create_dir(tree.join("deep")).unwrap();
        for cwd in [tree.clone(), tree.join("deep"), tree.clone()] {
            unplaced_stop(&cwd, "s", &script, true);
        }
        assert!(
            !shown(&log).iter().any(|names| names.contains("second.rs")),
            "{name}: {:?}",
            shown(&log)
        );
    }
}

/// Thirteenth independent review of the hooks, HR13-L-2, as reproduced: a stop made from a
/// directory inside the Git directory (`.git/NOTES_MERGE_WORKTREE`, where `git notes merge`
/// has a person resolve a conflict) with a long path. Its reason gave the path first and
/// Git's words after it, and a message shows a reason up to 300 characters: "…: fatal: this
/// operation must b…" was all that could be read. Git's words come first.
#[test]
fn gits_words_are_never_cut_away() {
    let scratch = Scratch::new("hook-git-words-first");
    let root = repository(&scratch);
    let (script, _) = reading_judge(&scratch);
    start(&root, "s");
    let deep = root
        .join(".git/NOTES_MERGE_WORKTREE")
        .join("n".repeat(100))
        .join("m".repeat(100));
    std::fs::create_dir_all(&deep).unwrap();
    let said = unplaced_stop(&deep, "s", &script, true);
    let line = said
        .lines()
        .find(|line| line.trim_start().starts_with("not judged: "))
        .unwrap_or_else(|| panic!("{said}"));
    assert!(
        line.contains("fatal: this operation must be run in a work tree."),
        "{line}"
    );
    // the project is judged wherever the session is, also from the Git directory itself
    // (the stop from there was one that judged one tree, before review V4 round 21)
    secret(&root, "a.rs");
    let reason = blocked(&stop_from(&root.join(".git"), "s", &script)).to_owned();
    assert!(
        reason.contains(&uncommitted_in(&std::fs::canonicalize(&root).unwrap()))
            && reason.contains("could not be found out: it is inside the Git directory"),
        "{reason}"
    );
}

/// Review V4 round 21, VD4R21-L-1, as reproduced: the session's directory is given as a
/// relative path, `../linked/locked/sub`, under a directory of a linked tree that cannot be
/// searched. The path was kept as written, no tree's path was under it, and the linked tree
/// the session was in was named with "the session's directory is not in it". It is anchored
/// at the hook's own directory, as the system anchors it.
#[test]
fn a_relative_directory_that_cannot_be_resolved_is_anchored() {
    use std::os::unix::fs::PermissionsExt as _;
    let scratch = Scratch::new("hook-relative-locked");
    let root = repository(&scratch);
    let (script, _) = reading_judge(&scratch);
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
    start(&root, "s");
    secret(&linked, "linked.rs");
    blocked(&stop_from(&linked, "s", &script));
    let locked = linked.join("locked");
    std::fs::create_dir_all(locked.join("sub")).unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    // (whoever may search every directory, as root may, has no such directory: that
    // trigger is passed)
    let unsearchable = std::fs::read_dir(&locked).is_err();
    let said = unsearchable.then(|| {
        let (code, stdout, stderr) = hook_in(
            "stop",
            &["--judge", script.to_str().unwrap()],
            &input("Stop", Path::new("../linked/locked/sub"), "s", false),
            |command| {
                command.current_dir(&root);
            },
        );
        assert_eq!((code, stderr.as_str()), (0, ""), "{stdout}");
        serde_json::from_str::<Value>(&stdout).unwrap()
    });
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
    if let Some(said) = said {
        let said = allowed(&said);
        assert!(
            said.starts_with("deep-diff-forge: UNJUDGED. ")
                && !said.contains("the session's directory is not in it")
                && said.contains(
                    "/linked\" of this project, and whether it is in that tree could not be found out at this stop"
                ),
            "{said}"
        );
    }
    let stands = allowed(&stop_from(&linked, "s", &script)).to_owned();
    assert!(
        stands.starts_with("deep-diff-forge: FAIL STANDS. "),
        "{stands}"
    );
}

/// Thirteenth independent review of the hooks, suspected, reproduced in round 22: the
/// session's directory is given with a `..` after a symbolic link, under a directory that
/// cannot be searched. `alias/../linked` was taken out by name: the stop named the tree
/// `linked` beside `alias` as one the session's directory may be in, and the tree it is in on
/// disk, where `alias` points, as one it "is not in". A `..` steps up from where the part
/// before it leads.
#[test]
fn a_parent_step_after_a_link_is_taken_where_the_link_leads() {
    use std::os::unix::fs::PermissionsExt as _;
    let scratch = Scratch::new("hook-dotdot-link");
    let root = repository(&scratch);
    let (script, _) = reading_judge(&scratch);
    let x = scratch.join("x");
    std::fs::create_dir_all(x.join("deep/inner")).unwrap();
    let (real, decoy) = (x.join("deep/linked"), x.join("linked"));
    for (tree, branch) in [(&real, "real"), (&decoy, "decoy")] {
        git(
            &root,
            &[
                "worktree",
                "add",
                "--quiet",
                "-b",
                branch,
                tree.to_str().unwrap(),
            ],
        );
    }
    std::os::unix::fs::symlink(x.join("deep/inner"), x.join("alias")).unwrap();
    start(&root, "s");
    secret(&real, "real.rs");
    blocked(&stop_from(&real, "s", &script));
    let locked = real.join("locked");
    std::fs::create_dir_all(locked.join("sub")).unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    let unsearchable = std::fs::read_dir(&locked).is_err();
    let said = unsearchable.then(|| stop_from(&x.join("alias/../linked/locked/sub"), "s", &script));
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
    if let Some(said) = said {
        let said = allowed(&said);
        // (a long path is cut in the middle where a message shows it: its end is kept)
        assert!(
            said.starts_with("deep-diff-forge: UNJUDGED. ")
                && !said.contains("the session's directory is not in it")
                && said.contains(
                    "/deep/linked\" of this project, and whether it is in that tree could not be found out at this stop"
                )
                && !said.contains(
                    "/x/linked\" of this project, and whether it is in that tree could not be found out"
                ),
            "{said}"
        );
    }
}

/// Thirteenth independent review of the hooks, suspected, reproduced in round 22: a
/// `GIT_CEILING_DIRECTORIES` in the session's environment at a linked tree's top made Git say
/// "not a git repository" for a directory below it: the tree was not judged from there, and
/// was named as one "the session's directory is not in". The hooks' Git calls have no
/// ceiling.
#[test]
fn a_ceiling_in_the_environment_does_not_hide_the_tree() {
    let scratch = Scratch::new("hook-ceiling");
    let root = repository(&scratch);
    let (script, log) = reading_judge(&scratch);
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
    let sub = linked.join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    start(&root, "s");
    secret(&linked, "linked.rs");
    let (code, stdout, stderr) = hook_in(
        "stop",
        &["--judge", script.to_str().unwrap()],
        &input("Stop", &sub, "s", false),
        |command| {
            command.env("GIT_CEILING_DIRECTORIES", &linked);
        },
    );
    assert_eq!((code, stderr.as_str()), (0, ""), "{stdout}");
    let out: Value = serde_json::from_str(&stdout).unwrap();
    let reason = blocked(&out);
    assert!(
        reason.contains(&uncommitted_in(&std::fs::canonicalize(&linked).unwrap()))
            && !reason.contains("the session's directory is not in it"),
        "{reason}"
    );
    assert_eq!(shown(&log), ["linked.rs"]);
}

/// Twelfth independent review of the hooks, HR12-L-4, as reproduced: the session's
/// directory is under a directory of a linked tree that cannot be searched (mode 000). The
/// stop is rightly UNJUDGED with the operating system's words, but the directory, which
/// could not be resolved, was "none", and the linked tree it is under was named with "the
/// session's directory is not in it", which is never to be said of that tree. The path as
/// it is written says which tree it is under.
#[test]
fn a_directory_that_cannot_be_resolved_is_still_under_its_tree() {
    use std::os::unix::fs::PermissionsExt as _;
    let scratch = Scratch::new("hook-locked-cwd");
    let root = repository(&scratch);
    let (script, _) = reading_judge(&scratch);
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
    start(&root, "s");
    secret(&linked, "linked.rs");
    blocked(&stop_from(&linked, "s", &script));
    let locked = linked.join("locked");
    std::fs::create_dir_all(locked.join("sub")).unwrap();
    // the same directory by a symbolic link to the tree: the links of the part that can be
    // resolved are followed
    let alias = scratch.join("alias");
    std::os::unix::fs::symlink(&linked, &alias).unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    // (whoever may search every directory, as root may, has no such directory: that
    // trigger is passed)
    let unsearchable = std::fs::read_dir(&locked).is_err();
    let said = unsearchable.then(|| {
        [
            stop_from(&alias.join("locked/sub"), "s", &script),
            stop_from(&locked.join("sub"), "s", &script),
        ]
    });
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
    if let Some([first, again]) = said {
        let (first, again) = (allowed(&first).to_owned(), allowed(&again).to_owned());
        for said in [&first, &again] {
            assert!(
                said.starts_with("deep-diff-forge: UNJUDGED. ")
                    && said.contains("could not be looked at")
                    && !said.contains("the session's directory is not in it"),
                "{said}"
            );
        }
        assert!(
            first.contains(
                "/linked\" of this project, and whether it is in that tree could not be found out at this stop"
            ) && first.contains("A failure there has blocked a stop of this session and stands"),
            "{first}"
        );
        assert!(
            again.contains("(the session's directory may be in it) (a failure stands there)"),
            "{again}"
        );
    }
    // nothing of the tree was forgotten
    let stands = allowed(&stop_from(&linked, "s", &script)).to_owned();
    assert!(
        stands.starts_with("deep-diff-forge: FAIL STANDS. "),
        "{stands}"
    );
}

/// Eleventh independent review of the hooks, HR11-L-5, as reproduced: the session's
/// directory was a directory of a linked working tree and has been removed (a build
/// directory the agent was in). A directory that is gone "added nothing", so the tree the
/// session is in was not judged at any stop, and was named with "the session's directory
/// is not in it". The nearest directory above it that is there says which tree it was in.
#[test]
fn a_directory_that_is_gone_is_in_the_tree_it_was_in() {
    let not_in = |said: &str, tree: &Path| {
        said.contains(&format!(
            "{}\" of this project has changes that are not committed, and the session's directory is not in it",
            std::fs::canonicalize(tree).unwrap().display()
        ))
    };
    let scratch = Scratch::new("hook-gone-in-linked");
    let root = repository(&scratch);
    let (script, log) = reading_judge(&scratch);
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
    start(&root, "s");
    let gone = linked.join("build/out");
    std::fs::create_dir_all(&gone).unwrap();
    std::fs::remove_dir_all(linked.join("build")).unwrap();
    secret(&linked, "linked.rs");
    let reason = blocked(&stop_from(&gone, "s", &script)).to_owned();
    assert!(
        reason.contains(&uncommitted_in(&std::fs::canonicalize(&linked).unwrap()))
            && !not_in(&reason, &linked),
        "{reason}"
    );
    assert_eq!(shown(&log), ["linked.rs"]);
    let stands = allowed(&stop_from(&gone, "s", &script)).to_owned();
    assert!(
        stands.starts_with("deep-diff-forge: FAIL STANDS. ")
            && !stands.contains("the session's directory is not in it"),
        "{stands}"
    );
    // a directory that is gone in the project's own tree, and one nothing of which is
    // there: the project is judged as before, and the linked tree is one the session is
    // not in
    for (n, cwd) in [root.join("gone/deep"), PathBuf::from("nowhere/at/all")]
        .into_iter()
        .enumerate()
    {
        let said = allowed(&stop_from(&cwd, "s", &script)).to_owned();
        assert!(
            !said.contains("could not be found out"),
            "{}: {said}",
            cwd.display()
        );
        // (the sentence is said in full once in a session; then the tree's name, with the
        // failure that stands there, is on the line that counts such things)
        if n == 0 {
            assert!(
                not_in(&said, &linked)
                    && said
                        .contains("A failure there has blocked a stop of this session and stands"),
                "{said}"
            );
        } else {
            assert!(
                said.contains("(a failure stands there)") && !not_in(&said, &linked),
                "{said}"
            );
        }
    }
}

/// Eleventh independent review of the hooks, HR11-L-2, as reproduced, and two more ways to
/// the same place. A stop made from another working tree looks at a tree with `git
/// status`, and took its "nothing" for "nothing there": what had blocked in that tree was
/// forgotten. But `git status` is not the reading a stop judges by. A change in a file
/// marked assume-unchanged, and one that a clean filter of the repository takes out, are
/// in the judge's patch and not in `git status`: after such a failure had blocked in a
/// linked tree, a stop from the project's tree printed nothing, and back in the linked
/// tree the unchanged failure blocked a second time. Where something is remembered of a
/// tree, only the reader the judge's patch is made by says that nothing is there:
///
/// | the linked tree | from the project's tree | back in the linked tree |
/// | --- | --- | --- |
/// | a key in a file marked assume-unchanged | named, "a failure stands there" | stands |
/// | that file as it was committed | nothing said: read, and forgotten | — |
/// | a key that a clean filter takes out | named, "a failure stands there" | stands |
/// | that file marked skip-worktree as well | "could not be looked at", stands | stands |
#[test]
fn a_look_at_another_tree_forgets_only_what_the_judges_own_reading_finds_gone() {
    let scratch = Scratch::new("hook-look-is-no-read");
    let root = repository(&scratch);
    let (script, _) = reading_judge(&scratch);
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
    start(&root, "s");
    let committed = std::fs::read_to_string(linked.join("a.rs")).unwrap();
    let key = || {
        std::fs::write(
            linked.join("a.rs"),
            format!("{committed}const KEY: &str = \"SECRET\";\n"),
        )
        .unwrap();
    };
    let status = || git(&linked, &["status", "--porcelain"]);
    // what the stop from the project's tree must say, and the stop back in the tree
    // (the sentence in full is said once in a session; the name is on the counting line
    // at every stop)
    let named_and_stands = |how: &str, in_full: bool| {
        let outside = allowed(&stop_from(&root, "s", &script)).to_owned();
        let full = outside
            .contains("A failure there has blocked a stop of this session and stands")
            && outside.contains(
                "`git status` there shows none of them; the patch a stop judges by has them",
            );
        assert_eq!(full, in_full, "{how}: {outside}");
        assert!(
            full || outside.contains("(a failure stands there)"),
            "{how}: {outside}"
        );
        let inside = allowed(&stop_from(&linked, "s", &script)).to_owned();
        assert!(
            inside.starts_with("deep-diff-forge: FAIL STANDS. "),
            "{how}: {inside}"
        );
    };

    // a key in a file marked assume-unchanged
    key();
    git(&linked, &["update-index", "--assume-unchanged", "a.rs"]);
    assert_eq!(status(), "", "git status shows the change");
    blocked(&stop_from(&linked, "s", &script));
    named_and_stands("assume-unchanged", true);
    // The file is as it was committed again: the judge's own reading finds nothing, the
    // failure is forgotten, and the stop from the project's tree says nothing.
    std::fs::write(linked.join("a.rs"), &committed).unwrap();
    assert_eq!(stop_from(&root, "s", &script), Value::Null);
    git(&linked, &["update-index", "--no-assume-unchanged", "a.rs"]);

    // a key that a clean filter of the repository takes out of what would be committed
    git(&root, &["config", "filter.scrub.clean", "sed /SECRET/d"]);
    std::fs::create_dir_all(root.join(".git/info")).unwrap();
    std::fs::write(root.join(".git/info/attributes"), "a.rs filter=scrub\n").unwrap();
    key();
    // (`git add` puts what the filter leaves, the committed bytes, in the index, with the
    // file's size and time: from then on `git status` has nothing to show)
    git(&linked, &["add", "a.rs"]);
    assert_eq!(status(), "", "git status shows the change");
    // (it was forgotten above, and is back: it blocks)
    blocked(&stop_from(&linked, "s", &script));
    named_and_stands("a clean filter", false);

    // the path marked skip-worktree as well: the judge's reading refuses such a tree, so
    // nothing is known of it from outside, and nothing is forgotten
    git(&linked, &["update-index", "--skip-worktree", "a.rs"]);
    let outside = allowed(&stop_from(&root, "s", &script)).to_owned();
    assert!(
        outside.contains("has changes that are not committed could not be found out (")
            && outside.contains("skip-worktree")
            && outside.contains("A failure there has blocked a stop of this session and stands"),
        "{outside}"
    );
    let again = allowed(&stop_from(&root, "s", &script)).to_owned();
    assert!(
        again.contains("(it could not be looked at) (a failure stands there)"),
        "{again}"
    );
    git(&linked, &["update-index", "--no-skip-worktree", "a.rs"]);
    let inside = allowed(&stop_from(&linked, "s", &script)).to_owned();
    assert!(
        inside.starts_with("deep-diff-forge: FAIL STANDS. "),
        "{inside}"
    );
}

/// The full id of the commit `HEAD` names in `root`.
fn head_id(root: &Path) -> String {
    git(root, &["rev-parse", "HEAD"]).trim().to_owned()
}

/// A stand-in Git that runs `what` (a piece of shell) where it would make the empty
/// repository a commit's patch is made in (`git init` in a scratch directory `commit-N`),
/// and only there.
fn git_at_commit_patch(scratch: &Scratch, name: &str, what: &str) -> PathBuf {
    git_with(
        scratch,
        name,
        &format!(
            "case \" $* \" in *\" init \"*) case \"$(pwd)\" in */commit-*) {what};; esac;; esac"
        ),
    )
}

/// Mutation run on `07df120`, `Judge::broken` and the count of what waits: a judge that
/// cannot be prepared (here it is not found) makes every commit of the session wait, counted,
/// with the reason, and nothing is written of them: none was asked about. With the arm that
/// says why taken away, each commit was written down as asked; with `*=` for `+=` the first
/// one was not counted.
#[test]
fn a_judge_that_cannot_be_prepared_leaves_every_commit_waiting_and_writes_nothing() {
    let scratch = Scratch::new("hook-broken-bench");
    let root = repository(&scratch);
    start(&root, "s");
    for n in 0..2 {
        std::fs::write(root.join(format!("c{n}.rs")), format!("fn c{n}() {{}}\n")).unwrap();
        commit_all(&root, &format!("c{n}"));
    }
    for _ in 0..2 {
        let said = allowed(&stop_from(
            &root,
            "s",
            Path::new("/nonexistent/forge-warden"),
        ))
        .to_owned();
        assert!(
            said.starts_with("deep-diff-forge: UNJUDGED. ")
                && said.contains("2 commits of this session")
                && said.contains("forge-warden was not found"),
            "{said}"
        );
        assert_eq!(
            judged_lines(),
            std::collections::BTreeMap::new(),
            "a commit was written down as asked"
        );
    }
}

/// Mutation run on `07df120`, `judge_commits`: what the hook proves of a commit itself is
/// final at once. A commit whose patch is over the patch builder's size is written down as
/// that (`x:`), and is never asked about again; with the arm's guard taken away it was
/// written as a commit with no verdict and asked about at every stop.
#[test]
fn a_commit_over_the_patch_builders_size_is_final_at_once() {
    let scratch = Scratch::new("hook-commit-too-large");
    let root = repository(&scratch);
    let (script, log) = reading_judge(&scratch);
    start(&root, "s");
    // 48 MiB of two-byte lines: its patch adds a `+` to each, over the 64 MiB a patch may have
    std::fs::write(root.join("big.txt"), "a\n".repeat(24 * 1024 * 1024)).unwrap();
    commit_all(&root, "big");
    let id = head_id(&root);
    for _ in 0..2 {
        let said = allowed(&stop_from(&root, "s", &script)).to_owned();
        assert!(said.contains(&id[..12]), "{said}");
        let written = judged_lines();
        assert!(written[&id].starts_with("x:"), "{written:?}");
    }
    assert_eq!(shown(&log), Vec::<String>::new(), "the judge was asked");
}

/// Mutation run on `07df120`, `judge_commits`: a commit whose patch Git cannot make (here
/// Git fails where it would make it) was not put to the judge, and not for lack of time. It
/// is written down as asked at this stop with the attempts it had (`n0`): the judge's
/// probe, which is asked because an ask failed, does not count an attempt for a commit the
/// judge never saw. With the arm's guard taken away it was counted (`n1`), and with `*=`
/// for `+=` the probe was not asked.
#[test]
fn a_commit_whose_patch_git_cannot_make_keeps_its_attempts() {
    let scratch = Scratch::new("hook-commit-git-fails");
    let root = repository(&scratch);
    let (script, log) = reading_judge(&scratch);
    start(&root, "s");
    std::fs::write(root.join("c.rs"), "fn c() {}\n").unwrap();
    commit_all(&root, "c");
    let id = head_id(&root);
    let failing = git_at_commit_patch(
        &scratch,
        "failing-git",
        "echo 'fatal: made to fail by the test' >&2; exit 128",
    );
    let said = allowed(&stop_with_path(&failing, &root, "s", &script)).to_owned();
    assert!(
        said.contains(&id[..12]) && said.contains("the commit could not be read from Git"),
        "{said}"
    );
    let written = judged_lines();
    assert!(
        written[&id].starts_with("n0@") && written[&id].contains("made to fail by the test"),
        "{written:?}"
    );
    // the probe was asked, and nothing else
    assert_eq!(shown(&log), ["warden.toml"]);
    // with a Git that works it is judged, and passes
    assert_eq!(stop_from(&root, "s", &script), Value::Null);
    assert!(judged_lines()[&id].starts_with('p'));
}

/// Mutation run on `07df120`, `judge_commits`: a stop whose time ends while Git makes a
/// commit's patch (here Git hangs there) did not ask about it. Nothing is written for it, it
/// is counted as waiting, and it is asked about at the next stop. With the guard of that arm
/// taken away it was written down as asked; with `-=` for `+=` the hook ended in a panic,
/// and with `*=` it was not counted.
#[test]
fn a_stop_whose_time_ends_while_git_makes_a_commits_patch_asks_nothing() {
    let scratch = Scratch::new("hook-commit-git-hangs");
    let root = repository(&scratch);
    let (script, log) = reading_judge(&scratch);
    start(&root, "s");
    std::fs::write(root.join("c.rs"), "const KEY: &str = \"SECRET\";\n").unwrap();
    commit_all(&root, "c");
    let id = head_id(&root);
    let hanging = git_at_commit_patch(&scratch, "hanging-git", "sleep 50");
    let began = std::time::Instant::now();
    let said = allowed(&stop_with_path(&hanging, &root, "s", &script)).to_owned();
    assert!(
        began.elapsed() < std::time::Duration::from_secs(46),
        "{:?}",
        began.elapsed()
    );
    assert!(
        said.starts_with("deep-diff-forge: UNJUDGED. ")
            && said.contains("1 commit of this session")
            && said.contains("they were not judged yet"),
        "{said}"
    );
    assert!(!judged_lines().contains_key(&id), "{:?}", judged_lines());
    assert_eq!(shown(&log), Vec::<String>::new());
    // the next stop asks, and the commit blocks
    let reason = blocked(&stop_from(&root, "s", &script)).to_owned();
    assert!(
        reason.contains(&format!("Commit {}: FAIL", &id[..12])),
        "{reason}"
    );
}

/// The session's lock file, once a hook of the session has made it.
fn lock_file() -> PathBuf {
    let locks: Vec<PathBuf> = std::fs::read_dir(sessions())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|kind| kind == "lock"))
        .collect();
    assert_eq!(locks.len(), 1, "{locks:?}");
    locks[0].clone()
}

/// Hold an exclusive `flock` on `path` (made when it is not there) from another process
/// until `go` is removed. Returns when the lock is held.
fn hold_lock(path: &Path, go: &Path) -> std::process::Child {
    std::fs::write(go, "").unwrap();
    let held = go.with_extension("held");
    // (`-o`: the command does not hold the file open; only `flock` itself does)
    let child = Command::new("flock")
        .arg("-o")
        .arg(path)
        .args([
            "sh",
            "-c",
            "touch \"$1\"; while [ -e \"$2\" ]; do sleep 0.02; done",
            "sh",
        ])
        .arg(&held)
        .arg(go)
        .stdin(Stdio::null())
        .spawn()
        .unwrap();
    let asked = std::time::Instant::now();
    while !held.exists() {
        assert!(
            asked.elapsed() < std::time::Duration::from_secs(20),
            "no lock"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    child
}

/// Whether a process of this user other than `but` has `path` open (a link under
/// `/proc/*/fd` names it).
fn opened_by_a_process(path: &Path, but: u32) -> bool {
    let but = but.to_string();
    std::fs::read_dir("/proc")
        .unwrap()
        .flatten()
        .any(|process| {
            process.file_name().to_str() != Some(but.as_str())
                && std::fs::read_dir(process.path().join("fd"))
                    .into_iter()
                    .flatten()
                    .flatten()
                    .any(|fd| std::fs::read_link(fd.path()).is_ok_and(|to| to == path))
        })
}

/// Mutation run on `07df120`, `session_lock`. A stop waits up to three seconds for another
/// hook of its session to let go of the session's lock: one that lets go in time leaves it
/// the session (with the guard of the waiting arm taken away, or its `!`, the stop gave up at
/// once). And the lock it then takes must be on the file at the lock's path: a hook that
/// leaves the session takes that file away, and another may have made a new one and hold it.
/// Comparing the inode or the device (`||` for `&&`) took a lock on a file that was gone
/// for one on the file that is there, and two stops of one session ran at once.
#[test]
fn a_stop_waits_for_the_session_s_lock_and_takes_only_the_one_that_is_there() {
    let scratch = Scratch::new("hook-lock-wait");
    let root = repository(&scratch);
    let (script, _) = reading_judge(&scratch);
    start(&root, "s");
    secret(&root, "k.rs");
    let lock = lock_file();
    // another hook holds the lock for half a second: the stop waits, then judges
    let go = scratch.join("go-1");
    let mut holder = hold_lock(&lock, &go);
    let reason = std::thread::scope(|scope| {
        let stop = scope.spawn(|| {
            home_in(&scratch);
            stop_from(&root, "s", &script)
        });
        std::thread::sleep(std::time::Duration::from_millis(500));
        std::fs::remove_file(&go).unwrap();
        stop.join().unwrap()
    });
    assert!(holder.wait().unwrap().success());
    blocked(&reason);

    // The stop opens the lock file while another hook holds it. Then that file is taken
    // away, a new one is made at its path and held by a third hook, and the first lets go:
    // the stop must not take the lock of the file that is gone.
    let (old, new) = (scratch.join("go-2"), scratch.join("go-3"));
    let mut first = hold_lock(&lock, &old);
    let lock_path = std::fs::canonicalize(&lock).unwrap();
    let (said, mut third) = std::thread::scope(|scope| {
        let stop = scope.spawn(|| {
            home_in(&scratch);
            stop_from(&root, "s", &script)
        });
        // (the stop has the file open: it waits for the lock, three seconds at most)
        let asked = std::time::Instant::now();
        while !opened_by_a_process(&lock_path, first.id()) {
            assert!(
                asked.elapsed() < std::time::Duration::from_secs(20),
                "the stop never opened the lock file"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        std::fs::remove_file(&lock).unwrap();
        let third = hold_lock(&lock, &new);
        std::fs::remove_file(&old).unwrap();
        (stop.join().unwrap(), third)
    });
    std::fs::remove_file(&new).unwrap();
    assert!(first.wait().unwrap().success() && third.wait().unwrap().success());
    let said = allowed(&said).to_owned();
    assert!(
        said.starts_with("deep-diff-forge: UNJUDGED. ")
            && said.contains("another hook of this session is running"),
        "{said}"
    );
}

/// Linked working trees `t0`, `t1`, … of `root` under `scratch`, each with a credential that
/// has blocked a stop made from inside it.
fn trees_where_a_failure_stands(
    scratch: &Scratch,
    root: &Path,
    judge: &Path,
    count: usize,
) -> Vec<PathBuf> {
    (0..count)
        .map(|n| {
            let tree = scratch.join(format!("t{n}"));
            git(
                root,
                &[
                    "worktree",
                    "add",
                    "--quiet",
                    "-b",
                    &format!("t{n}"),
                    tree.to_str().unwrap(),
                ],
            );
            secret(&tree, "k.rs");
            blocked(&stop_from(&tree, "s", judge));
            tree
        })
        .collect()
}

/// Mutation run on `07df120`, `other_trees`: the trees a stop had no time to look at are
/// counted, with the failures that stand in them, both where `git status` did not end in
/// a tree's share of the time and where a tree's share was under the least a look is given
/// (200 ms). Six trees where a failure stands; `git status` hangs in the first four looked at
/// (each is stopped at the end of its share, half of what is left: 3, 1.5, 0.75, 0.375
/// seconds), so the fifth has under 0.2 and is not looked at; the sixth has what is left,
/// and is looked at. The counts were caught by no test (`*` or `-` for `+` there), nor was
/// `==` for `<`, which looked at the fifth with what little was left.
#[test]
fn trees_not_looked_at_for_lack_of_time_are_counted_with_what_stands_in_them() {
    let scratch = Scratch::new("hook-late-trees");
    let root = repository(&scratch);
    let (script, _) = reading_judge(&scratch);
    start(&root, "s");
    let trees = trees_where_a_failure_stands(&scratch, &root, &script, 6);
    let calls = scratch.join("status-calls");
    let slow = git_with(
        &scratch,
        "slow-status",
        &format!(
            "case \" $* \" in *\" status \"*) case \"$(pwd)\" in */t[0-9]*) echo >> '{calls}'; if [ \"$(wc -l < '{calls}')\" -le 4 ]; then sleep 30; fi;; esac;; esac",
            calls = calls.display()
        ),
    );
    let said = allowed(&stop_with_path(&slow, &root, "s", &script)).to_owned();
    assert!(
        said.contains("5 working trees of this project were not reached in that time")
            && said
                .contains("Failures that have blocked a stop of this session stand in 5 of them"),
        "{said}"
    );
    // the sixth was looked at, and is named for what it has
    let named = trees
        .iter()
        .filter(|tree| {
            said.contains(&format!(
                "{}\" of this project has changes that are not committed",
                std::fs::canonicalize(tree).unwrap().display()
            ))
        })
        .count();
    assert_eq!(named, 1, "{said}");
    assert_eq!(
        std::fs::read_to_string(&calls).unwrap().lines().count(),
        5,
        "a tree whose share was under 0.2 seconds was looked at"
    );
}

/// Mutation run on `07df120`, `judge_units`: the submodules are looked at in half of the
/// six seconds of looking, so that a `git status` that hangs there leaves the other working
/// trees the other half (`* 2` for `/ 2` gave it all of it, and a linked tree with a change
/// was "not looked at for lack of time").
#[test]
fn a_submodule_look_that_hangs_leaves_the_other_trees_their_half() {
    let scratch = Scratch::new("hook-slow-submodules");
    let root = repository(&scratch);
    let (script, _) = reading_judge(&scratch);
    std::fs::write(root.join(".gitmodules"), "").unwrap();
    commit_all(&root, "submodules: none yet");
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
    start(&root, "s");
    secret(&linked, "k.rs");
    let slow = git_with(
        &scratch,
        "slow-submodules",
        "case \" $* \" in *\" --ignore-submodules=none \"*) sleep 30;; esac",
    );
    let said = allowed(&stop_with_path(&slow, &root, "s", &script)).to_owned();
    assert!(
        said.contains(&format!(
            "{}\" of this project has changes that are not committed, and the session's directory is not in it",
            std::fs::canonicalize(&linked).unwrap().display()
        )) && said.contains("Whether a submodule of "),
        "{said}"
    );
}

/// Eleventh independent review of the hooks, HR11-L-6, the first place: "a run that was cut
/// is written down as asked and cut, with the time it had" was not true of a run that had
/// all that was left of the stop (three seconds or more, under thirty): it was written with
/// no time. The judge takes 15 seconds on the working tree's change and hangs on the
/// session's one commit, the last ask, which has what is left. (A mutation run on `07df120`
/// also left `>` for `<` in the choice of the sentence alive.)
#[test]
fn a_run_that_had_all_that_was_left_is_written_with_that_time() {
    let scratch = Scratch::new("hook-run-had-the-rest");
    let root = repository(&scratch);
    let (inner, _) = reading_judge(&scratch);
    let judge = wrapping_judge(
        &scratch,
        "slow-then-hanging-judge",
        "if [ -n \"$patch\" ] && grep -q '^+.*SLOW' \"$patch\"; then sleep 15; fi; if [ -n \"$patch\" ] && grep -q '^+.*HANG' \"$patch\"; then sleep 60; fi",
        &inner,
    );
    start(&root, "s");
    std::fs::write(root.join("h.rs"), "// HANG\n").unwrap();
    commit_all(&root, "h");
    let id = head_id(&root);
    std::fs::write(root.join("s.rs"), "// SLOW\n").unwrap();
    let said = allowed(&stop_from(&root, "s", &judge)).to_owned();
    let written = judged_lines();
    let line = &written[&id];
    let had: f64 = line
        .split_once("(this run had ")
        .and_then(|(_, rest)| rest.split_once(" seconds, all that was left of the stop's judging)"))
        .and_then(|(seconds, _)| seconds.parse().ok())
        .unwrap_or_else(|| panic!("{line}\n{said}"));
    assert!(
        line.starts_with('n') && (3.0..30.0).contains(&had),
        "{line}"
    );
}

/// Mutation run on `07df120`, `subject`: a stop that is its session's first hook, in a
/// repository with no commit, goes by what its own record found. With the guard taken away
/// it asked Git a second time whether the repository has a commit, so a Git that answered
/// once and failed the second time made the stop UNJUDGED where the record had found that
/// there was nothing to judge.
#[test]
fn a_first_stop_in_a_repository_with_no_commit_goes_by_its_own_record() {
    let scratch = Scratch::new("hook-no-commit-first-stop");
    home_in(&scratch);
    let (script, _) = reading_judge(&scratch);
    let root = scratch.join("new");
    std::fs::create_dir(&root).unwrap();
    git(&root, &["init", "--quiet"]);
    std::fs::write(root.join("a.rs"), "fn a() {}\n").unwrap();
    let asked = scratch.join("asked");
    let once = git_with(
        &scratch,
        "git-once",
        &format!(
            "case \" $* \" in *\" rev-list --max-count=1 --all \"*) echo >> '{asked}'; if [ \"$(wc -l < '{asked}')\" -ge 2 ]; then echo 'fatal: made to fail by the test' >&2; exit 128; fi;; esac",
            asked = asked.display()
        ),
    );
    let said = allowed(&stop_with_path(&once, &root, "s", &script)).to_owned();
    assert!(
        !said.contains("UNJUDGED")
            && said.contains(
                "has no commit yet, so there is no commit to measure this session's changes from"
            ),
        "{said}"
    );
}

/// A judge that wraps another: `before` is a piece of shell that sees the patch it is asked
/// about as `$patch` (empty for anything that is no judging) and runs first; then the
/// judge `inner` is run with the same arguments.
fn wrapping_judge(scratch: &Scratch, name: &str, before: &str, inner: &Path) -> PathBuf {
    let script = scratch.join(name);
    write_program(
        &script,
        &format!(
            "#!/bin/sh\npatch=; prev=\nfor a in \"$@\"; do if [ \"$prev\" = --patch ]; then patch=\"$a\"; fi; prev=\"$a\"; done\n{before}\nexec '{}' \"$@\"\n",
            inner.display()
        ),
    );
    script
}

/// Tenth independent review of the hooks, HR10-M-1, by its three triggers. A staged failure
/// has blocked and stands (the key is staged; the working tree was repaired and the repair
/// not staged). Then, at one stop, the working tree cannot be read: an untracked file that
/// may not be opened; a path marked skip-worktree; a Git that fails. That stop did not
/// look at the index, and remembered it as clean: at the next stop the staged failure,
/// untouched in the index and already told, blocked a second time, and no stop had said
/// anything of memory. A stop that could not read forgets nothing, and says that what has
/// blocked of the staged change stands.
#[test]
fn a_stop_that_could_not_read_the_tree_forgets_nothing_of_it() {
    use std::os::unix::fs::PermissionsExt as _;
    let scratch = Scratch::new("hook-not-read");
    let root = repository(&scratch);
    let (script, _) = reading_judge(&scratch);
    start(&root, "s");
    secret(&root, "k.rs");
    git(&root, &["add", "k.rs"]);
    std::fs::write(root.join("k.rs"), "fn repaired() {}\n").unwrap();
    let reason = blocked(&stop_from(&root, "s", &script)).to_owned();
    assert!(reason.contains("Staged changes in "), "{reason}");
    let stands = |row: &str| {
        let said = allowed(&stop_from(&root, "s", &script)).to_owned();
        assert!(
            said.contains("Staged changes in ") && said.contains("FAIL, which stands"),
            "{row}: {said}"
        );
    };
    let not_read = |row: &str, stop: Value| {
        let said = allowed(&stop).to_owned();
        assert!(
            said.starts_with("deep-diff-forge: UNJUDGED. ")
                && said.contains("Uncommitted changes in ")
                && said.contains("Staged changes in ")
                && said.contains("what has blocked of it before stands"),
            "{row}: {said}"
        );
    };
    stands("nothing happened");

    // an untracked file that may not be read (a file another tool left behind)
    let locked = root.join("locked.bin");
    std::fs::write(&locked, "x").unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    // (whoever may read every file, as root may, has no such file: that trigger is passed)
    if std::fs::read(&locked).is_err() {
        not_read("an unreadable file", stop_from(&root, "s", &script));
        not_read("again", stop_from(&root, "s", &script));
    }
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o644)).unwrap();
    std::fs::remove_file(&locked).unwrap();
    stands("the unreadable file is gone; the index is as it was");

    // a path marked skip-worktree
    git(&root, &["update-index", "--skip-worktree", "a.rs"]);
    not_read("skip-worktree", stop_from(&root, "s", &script));
    git(&root, &["update-index", "--no-skip-worktree", "a.rs"]);
    stands("the mark is taken away; the index is as it was");

    // a Git that fails at one stop, where the tree's files are listed
    let flag = scratch.join("git.fail");
    std::fs::write(&flag, "").unwrap();
    let failing = git_with(
        &scratch,
        "failing-git",
        &format!(
            "case \" $* \" in *\" ls-files \"*) if mv '{flag}' '{flag}.used' 2>/dev/null; then echo 'fatal: made to fail by the test' >&2; exit 128; fi;; esac",
            flag = flag.display()
        ),
    );
    not_read("Git failed", stop_with_path(&failing, &root, "s", &script));
    assert!(!flag.exists(), "the stand-in Git was not used");
    stands("Git answers again; the index is as it was");
}

/// Tenth independent review of the hooks, HR10-M-2, as reproduced: 300 failing files are
/// staged and all repaired in the working tree (the repair not staged): the staged change
/// blocks. Then the keys are back in 100 files of the tree (that blocks as the tree's
/// failure, which is documented); then 50 of those are repaired again in the tree; then
/// the rest. The index is never touched. At the partial repair the stop blocked again for
/// "Staged changes": the mark of all 300 was remembered, and nothing of a mark answered for
/// a part of it. A mark that is exactly what the index holds says that every staged line
/// has blocked.
#[test]
fn a_large_staged_failure_stands_whatever_the_tree_does() {
    let scratch = Scratch::new("hook-staged-mark");
    let root = repository(&scratch);
    let (script, _) = listing_judge(&scratch);
    start(&root, "s");
    std::fs::create_dir(root.join("g")).unwrap();
    for file in 0..300 {
        secrets(&root, &format!("g/k{file}.rs"), 1);
    }
    git(&root, &["add", "--all"]);
    let index = git(&root, &["write-tree"]);
    let repair = |files: std::ops::Range<usize>| {
        for file in files {
            std::fs::write(root.join(format!("g/k{file}.rs")), "fn repaired() {}\n").unwrap();
        }
    };
    let staged_stands = |row: &str| {
        let decision = stop_from(&root, "s", &script);
        let said = allowed(&decision).to_owned();
        assert!(
            said.contains("Staged changes in ")
                && !said.contains("could not keep")
                && !said.contains("remembered as a whole"),
            "{row}: {said}"
        );
    };
    repair(0..300);
    let reason = blocked(&stop_from(&root, "s", &script)).to_owned();
    assert!(reason.contains("Staged changes in "), "{reason}");
    staged_stands("nothing happened");
    // the keys are back in 100 files of the tree: the tree's own failure blocks, once
    for file in 0..100 {
        secrets(&root, &format!("g/k{file}.rs"), 1);
    }
    // (one unit blocks: the tree's; what is staged is named beside it as what stands)
    let reason = blocked(&stop_from(&root, "s", &script)).to_owned();
    assert!(
        reason.contains("Uncommitted changes in ")
            && reason.matches("\": FAIL\n").count() == 1
            && reason.contains("\": FAIL, which stands\n"),
        "what is staged blocked again when the tree had a part of it: {reason}"
    );
    staged_stands("a part is in the tree again");
    repair(0..50);
    staged_stands("50 of those repaired again in the tree");
    repair(50..100);
    staged_stands("the rest repaired again");
    assert_eq!(git(&root, &["write-tree"]), index, "the index was touched");
}

/// Tenth independent review of the hooks, HR10-L-1, as reproduced: the sentence a stop says
/// when it gives up what it remembered of a staged change is reached by a run of the hooks,
/// with one working tree and 300 failing lines, and must be true of that stop. 300 files
/// staged have blocked and the tree has them again; then one file is taken out of the
/// index. What had blocked was remembered as one mark of all 300 (more than 256 lines),
/// and the index no longer holds exactly that: the stop says so, with that cause. It began
/// "This session has more failing lines than the hook keeps one by one (2048)", which was
/// not so. The block it announces comes when the tree is repaired again.
#[test]
fn giving_up_what_was_remembered_of_a_staged_change_says_what_happened() {
    let scratch = Scratch::new("hook-staged-gone");
    let root = repository(&scratch);
    let (script, _) = listing_judge(&scratch);
    start(&root, "s");
    std::fs::create_dir(root.join("g")).unwrap();
    let keys = || {
        for file in 0..300 {
            secrets(&root, &format!("g/k{file}.rs"), 1);
        }
    };
    let repair = || {
        for file in 0..300 {
            std::fs::write(root.join(format!("g/k{file}.rs")), "fn repaired() {}\n").unwrap();
        }
    };
    keys();
    git(&root, &["add", "--all"]);
    repair();
    blocked(&stop_from(&root, "s", &script));
    keys();
    blocked(&stop_from(&root, "s", &script));
    let said = allowed(&stop_from(&root, "s", &script)).to_owned();
    assert!(!said.contains("is not known any more"), "{said}");
    // one file leaves the index; the tree is as it was
    git(&root, &["reset", "--quiet", "--", "g/k0.rs"]);
    let said = allowed(&stop_from(&root, "s", &script)).to_owned();
    assert!(
        said.contains(
            "Of what is staged in 1 working tree, the hook remembered what had blocked as a whole"
        ) && said.contains("the index no longer holds exactly that")
            && said.contains("may block once more")
            && !said.contains("more failing lines than the hook keeps"),
        "{said}"
    );
    // said once, at the stop where it happened
    let again = allowed(&stop_from(&root, "s", &script)).to_owned();
    assert!(!again.contains("is not known any more"), "{again}");
    // the tree repaired again: what is staged blocks once more, as the stop said it may
    // That stop is the block the sentence warned of, with the index untouched since: it
    // does not say again that the index "no longer holds" what it holds, nor warn of a
    // block that cannot come any more, since everything the index holds is remembered
    // from here on (eleventh independent review of the hooks, HR11-L-4).
    let index = git(&root, &["write-tree"]);
    repair();
    let reason = blocked(&stop_from(&root, "s", &script)).to_owned();
    assert!(
        reason.contains("Staged changes in ")
            && !reason.contains("the index no longer holds exactly that")
            && !reason.contains("may block once more"),
        "{reason}"
    );
    let stands = allowed(&stop_from(&root, "s", &script)).to_owned();
    assert!(stands.contains("FAIL, which stands"), "{stands}");
    // and none comes: the tree has the keys again (its own failure), and is repaired again
    keys();
    blocked(&stop_from(&root, "s", &script));
    repair();
    let stands = allowed(&stop_from(&root, "s", &script)).to_owned();
    assert!(stands.contains("FAIL, which stands"), "{stands}");
    assert_eq!(git(&root, &["write-tree"]), index, "the index was touched");
}

/// Eleventh independent review of the hooks, HR11-L-4, as reproduced (`mi7.py 600`). 600
/// failing files are staged and repaired in the tree; the tree has 300 of them again; one
/// more failing file is staged: that blocks, and the stop says, truly, that the digest it had
/// is given up and that what is staged may block once more when the tree is repaired again.
/// Then the tree is repaired with the index untouched: that is the block it warned of, and
/// it said the same sentence again, of an index that "no longer holds" what it held, and
/// warned of a block that could not come any more (none does: the last stop below).
#[test]
fn a_digest_given_up_is_not_said_again_at_the_block_it_warned_of() {
    const GIVEN_UP: &str = "the index no longer holds exactly that";
    let scratch = Scratch::new("hook-given-up-once");
    let root = repository(&scratch);
    let (script, _) = listing_judge(&scratch);
    start(&root, "s");
    std::fs::create_dir(root.join("g")).unwrap();
    let keys = |files: std::ops::Range<usize>| {
        for file in files {
            secrets(&root, &format!("g/k{file}.rs"), 1);
        }
    };
    let repair = || {
        for file in 0..600 {
            std::fs::write(root.join(format!("g/k{file}.rs")), "fn repaired() {}\n").unwrap();
        }
    };
    keys(0..600);
    git(&root, &["add", "--all"]);
    repair();
    blocked(&stop_from(&root, "s", &script));
    keys(0..300);
    blocked(&stop_from(&root, "s", &script));
    allowed(&stop_from(&root, "s", &script));
    // one more failing file staged, and repaired in the tree
    secrets(&root, "g/dup.rs", 1);
    git(&root, &["add", "g/dup.rs"]);
    std::fs::write(root.join("g/dup.rs"), "fn repaired() {}\n").unwrap();
    let index = git(&root, &["write-tree"]);
    let reason = blocked(&stop_from(&root, "s", &script)).to_owned();
    assert!(
        reason.contains(GIVEN_UP) && reason.contains("may block once more"),
        "{reason}"
    );
    allowed(&stop_from(&root, "s", &script));
    // the tree repaired, the index untouched: the block that was announced, said as itself
    repair();
    let reason = blocked(&stop_from(&root, "s", &script)).to_owned();
    assert!(
        reason.contains("Staged changes in ")
            && !reason.contains(GIVEN_UP)
            && !reason.contains("may block once more"),
        "{reason}"
    );
    allowed(&stop_from(&root, "s", &script));
    // the tree's own failure again, then repaired again: what is staged stands
    keys(0..100);
    blocked(&stop_from(&root, "s", &script));
    repair();
    let stands = allowed(&stop_from(&root, "s", &script)).to_owned();
    assert!(stands.contains("FAIL, which stands"), "{stands}");
    assert_eq!(git(&root, &["write-tree"]), index, "the index was touched");
}

/// Tenth independent review of the hooks, HR10-M-3, as reproduced: a bare repository with
/// its working trees inside it (`repo.git/main`, `repo.git/feature`), which is how many
/// people keep working trees. Every tree is under the Git common directory there, and a
/// directory under it was taken for one of Git's own files: a stop made from `feature`,
/// in a session whose project is `main`, did not judge `feature` at all, and said of it
/// "the session's directory is not in it … a stop made from inside it judges them". The
/// tree the stop is in is judged, and first; so is a tree nested in the project
/// (`main/.claude/worktrees/x`); and a stop from inside the Git directory judges no tree of
/// its own there.
#[test]
fn working_trees_inside_a_bare_repository_are_judged_from_inside() {
    let scratch = Scratch::new("hook-bare-inside");
    let origin = repository(&scratch);
    let (script, log) = reading_judge(&scratch);
    let bare = scratch.join("repo.git");
    git(
        scratch.path(),
        &[
            "clone",
            "--quiet",
            "--bare",
            origin.to_str().unwrap(),
            bare.to_str().unwrap(),
        ],
    );
    let (main, feature) = (bare.join("main"), bare.join("feature"));
    let nested = main.join(".claude/worktrees/x");
    for (tree, branch) in [(&main, "m"), (&feature, "f"), (&nested, "x")] {
        git(
            &bare,
            &[
                "worktree",
                "add",
                "--quiet",
                "-b",
                branch,
                tree.to_str().unwrap(),
            ],
        );
    }
    start(&main, "s");
    for (tree, file) in [(&feature, "feature.rs"), (&nested, "nested.rs")] {
        secret(tree, file);
        // from the tree's top and from a directory inside it
        std::fs::create_dir(tree.join("deep")).unwrap();
        let before = shown(&log).len();
        let reason = blocked(&stop_from(&tree.join("deep"), "s", &script)).to_owned();
        assert!(
            reason.contains(&uncommitted_in(&std::fs::canonicalize(tree).unwrap())),
            "{reason}"
        );
        assert_eq!(shown(&log)[before], file, "{:?}", shown(&log));
        let said = allowed(&stop_from(tree, "s", &script)).to_owned();
        assert!(
            said.starts_with("deep-diff-forge: FAIL STANDS. ")
                && !said.contains("the session's directory is not in it"),
            "{said}"
        );
    }
    // from the project the two are named, as trees where a failure stands
    let outside = allowed(&stop_from(&main, "s", &script)).to_owned();
    assert_eq!(
        outside
            .matches("A failure there has blocked a stop of this session and stands")
            .count()
            + outside.matches("(a failure stands there)").count(),
        2,
        "{outside}"
    );
    // Git cannot place a directory inside the Git directory: the stop is UNJUDGED with its
    // words (review V4 round 21, VD4R21-M-1), the project is judged, nothing else
    let before = runs(&log);
    for inside in [
        bare.clone(),
        bare.join("objects"),
        bare.join("worktrees/feature"),
    ] {
        let said = allowed(&stop_from(&inside, "s", &script)).to_owned();
        assert!(
            said.starts_with("deep-diff-forge: UNJUDGED. ")
                && said.contains("could not be found out: it is inside the Git directory"),
            "{}: {said}",
            inside.display()
        );
    }
    assert_eq!(
        runs(&log),
        before,
        "a judge was run from inside the Git directory"
    );
}

/// Tenth independent review of the hooks, HR10-L-2: the time the stop's own tree has when
/// the stop is made from a linked working tree. Time for two runs was held back for the
/// project's tree before it was known whether that tree had anything to judge: the own
/// tree's run had 15 of the 35 seconds, and with a judge that takes 17 it was UNJUDGED
/// while 20 seconds went unused. The project's tree is looked at first (one `git status`):
/// clean, nothing is held back for it; with a change, one run's time is.
#[test]
fn the_stops_own_tree_has_the_time_the_projects_tree_does_not_need() {
    let scratch = Scratch::new("hook-own-time");
    let root = repository(&scratch);
    let (script, _) = reading_judge(&scratch);
    let slow = wrapping_judge(
        &scratch,
        "slow-judge",
        "if [ -n \"$patch\" ] && grep -q '^+.*SLOWLINKED' \"$patch\"; then sleep 17; fi",
        &script,
    );
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
    std::fs::write(
        linked.join("linked.rs"),
        "const KEY: &str = \"SECRET\"; // SLOWLINKED\n",
    )
    .unwrap();
    let own = uncommitted_in(&std::fs::canonicalize(&linked).unwrap());
    // the project's tree is clean: the own tree's run has 25 seconds
    start(&root, "clean");
    let reason = blocked(&stop_from(&linked, "clean", &slow)).to_owned();
    assert!(reason.contains("- rule no-secrets"), "{reason}");
    // the project's tree has a change too: the own tree is first, with 20 seconds
    start(&root, "both");
    secret(&root, "main.rs");
    let reason = blocked(&stop_from(&linked, "both", &slow)).to_owned();
    assert!(
        reason.contains(&format!("{own}: FAIL\n"))
            && reason.contains(&format!(
                "{}: FAIL\n",
                uncommitted_in(&std::fs::canonicalize(&root).unwrap())
            )),
        "{reason}"
    );
}

/// Tenth independent review of the hooks, HR10-L-3, as reproduced in its shortest form. A
/// judge hangs on two commits: one that waits for its turn (more than five attempts) and
/// one with fewer attempts. A third commit, never asked about, has a credential. The first
/// run had what was left less five seconds (30 at most) and the second all the rest, so
/// nothing was left for the third, at every stop, and it was never submitted, while each
/// stop said "they are judged at the next stop". An ask leaves three seconds for each ask
/// owed after it: the third commit is submitted at the first stop after it is made, and
/// blocks; the two that hang are written down as asked and cut, with the time they had.
#[test]
fn a_judge_that_hangs_on_two_commits_does_not_keep_a_third_from_being_asked() {
    let scratch = Scratch::new("hook-hanging-judge");
    let root = repository(&scratch);
    let (script, log) = reading_judge(&scratch);
    let hang = scratch.join("hang.on");
    // for a patch marked HANG: no verdict at once, and a hang once `hang.on` is there
    let judge = wrapping_judge(
        &scratch,
        "hanging-judge",
        &format!(
            "if [ -n \"$patch\" ] && grep -q '^+.*HANG' \"$patch\"; then if [ -f '{}' ]; then printf 'PATCH-NAMES %s\\n' \"$(grep '^diff --git' \"$patch\" | tr '\\n' ' ')\" >> '{}'; sleep 100; fi; echo '{{\"exit\":3,\"name\":\"io\",\"message\":\"no answer\",\"next\":\"retry\"}}' >&2; exit 3; fi",
            hang.display(),
            log.display()
        ),
        &script,
    );
    start(&root, "s");
    let commit = |file: &str, text: &str| -> String {
        std::fs::write(root.join(file), text).unwrap();
        commit_all(&root, file);
        git(&root, &["rev-parse", "HEAD"]).trim().to_owned()
    };
    let turn = commit("t.rs", "// HANG t\n");
    for _ in 0..3 {
        allowed(&stop_from(&root, "s", &judge));
    }
    let fewer = commit("a.rs", "fn a() {}\n// HANG a\n");
    for _ in 0..3 {
        allowed(&stop_from(&root, "s", &judge));
    }
    let written = judged_lines();
    assert!(
        written[&turn].starts_with("n6@") && written[&fewer].starts_with("n3@"),
        "{written:?}"
    );
    // from now on the judge hangs on both; and a commit with a credential is made
    std::fs::write(&hang, "").unwrap();
    let key = commit("key.rs", "const KEY: &str = \"SECRET\";\n");
    let before = shown(&log).len();
    let began = std::time::Instant::now();
    let reason = blocked(&stop_from(&root, "s", &judge)).to_owned();
    assert!(
        began.elapsed() < std::time::Duration::from_secs(44),
        "{:?}",
        began.elapsed()
    );
    assert!(
        reason.contains(&format!("Commit {}: FAIL\n", &key[..12])),
        "{reason}"
    );
    assert_eq!(shown(&log)[before..], ["t.rs", "a.rs", "key.rs"]);
    let written = judged_lines();
    assert!(written[&key].starts_with("f:"), "{written:?}");
    for (id, line) in [(&turn, "n7@"), (&fewer, "n4@")] {
        assert!(
            written[id].starts_with(line) && written[id].contains("this run had "),
            "{written:?}"
        );
    }
}

/// Review V4 round 16, VD4R16-L-1, the last row of the table: a copy of the whole project is
/// put back at the project's path (a restore from a backup). Every inode is new, the Git
/// directory's too, so this is no tree of the session's project that came back as another:
/// it is another repository at the project's path, like a clone put there, and a stop is not
/// judged and says so, from the project and from its linked working tree alike. The failure
/// that had blocked is not said to stand and does not pass in silence.
#[test]
fn a_copy_of_the_project_at_its_path_is_not_the_project() {
    let scratch = Scratch::new("hook-copied-project");
    let root = repository(&scratch);
    let (script, log) = reading_judge(&scratch);
    start(&root, "s");
    let tree = scratch.join("wt");
    git(
        &root,
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "b1",
            tree.to_str().unwrap(),
        ],
    );
    secret(&tree, "k.rs");
    blocked(&stop_from(&tree, "s", &script));
    let old = scratch.join("project-old");
    std::fs::rename(&root, &old).unwrap();
    let copied = std::process::Command::new("cp")
        .args(["-R", "-p"])
        .arg(&old)
        .arg(&root)
        .status()
        .unwrap();
    assert!(copied.success());
    // Git takes the copy for the project, with its linked tree
    assert!(git(&tree, &["status", "--porcelain=v1"]).contains("k.rs"));
    let recorded = std::fs::canonicalize(&root).unwrap();
    let judged = runs(&log);
    for cwd in [&tree, &root] {
        let decision = stop_from(cwd, "s", &script);
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
    assert_eq!(runs(&log), judged, "the copy was judged");
    // a new session there has the tree's failure judged: it blocks, as new
    start(&root, "s2");
    blocked(&stop_from(&tree, "s2", &script));
}

/// Review V4 round 16, VD4R16-M-1, as reproduced: nine linked working trees, each with a
/// failure of 256 lines, and a stop made from each. The session's state kept 2,048 failing
/// lines and cut the oldest from the front with no word, so the ninth tree pushed out the
/// first one's lines, and the first, unchanged and already told to a person, blocked a
/// second time. Now a tree whose lines do not fit is remembered by a mark of all of them:
/// it stands while it is what it was, and blocks as a whole, saying why, when a part of
/// it changes; a tree that is remembered line by line stands when a part of it is repaired.
/// No stop says that anything was lost, because nothing was.
#[test]
fn a_failure_that_stands_is_not_pushed_out_by_the_failures_of_other_trees() {
    let scratch = Scratch::new("hook-blocked-bound");
    let root = repository(&scratch);
    let (script, _) = listing_judge(&scratch);
    start(&root, "s");
    let tree = |n: usize| scratch.join(format!("wt{n}"));
    let stands = |n: usize, row: &str| {
        let said = allowed(&stop_from(&tree(n), "s", &script)).to_owned();
        assert!(
            said.starts_with("deep-diff-forge: FAIL STANDS. ")
                && !said.contains("could not be kept"),
            "tree {n}, {row}: {said}"
        );
    };
    let remembered = || -> Vec<String> {
        let state = states_in(&sessions());
        let kept: Value =
            serde_json::from_str(&std::fs::read_to_string(&state[0]).unwrap()).unwrap();
        kept["blocked"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry.as_str().unwrap().to_owned())
            .collect()
    };
    for n in 1..=9 {
        git(
            &root,
            &[
                "worktree",
                "add",
                "--quiet",
                "-b",
                &format!("b{n}"),
                tree(n).to_str().unwrap(),
            ],
        );
        for file in 0..256 {
            secrets(&tree(n), &format!("k{file}.rs"), 1);
        }
        let reason = blocked(&stop_from(&tree(n), "s", &script)).to_owned();
        assert!(!reason.contains("could not be kept"), "tree {n}: {reason}");
        assert!(
            remembered().len() <= 2_048,
            "tree {n}: {}",
            remembered().len()
        );
    }
    // 2,304 lines do not fit: two trees are one mark each, seven are kept line by line
    let kept = remembered();
    assert_eq!(kept.len(), 2 + 7 * 256, "{}", kept.len());
    assert_eq!(
        kept.iter()
            .filter(|entry| entry.rsplit_once('/').unwrap().1.starts_with("all"))
            .count(),
        2
    );
    // the first tree, unchanged, and the second: each has blocked, and stands
    stands(1, "unchanged, after eight other trees");
    stands(2, "unchanged");
    stands(9, "unchanged");
    // By now the third tree is remembered by its mark. One of its files is repaired: it
    // blocks again, as a whole, and says that it is not all new; then it stands.
    std::fs::remove_file(tree(3).join("k0.rs")).unwrap();
    let reason = blocked(&stop_from(&tree(3), "s", &script)).to_owned();
    assert!(
        reason.contains("A failure here has blocked before and is not what it was")
            && !reason.contains("New at this stop"),
        "{reason}"
    );
    stands(3, "after its block");
    // the ninth is remembered line by line: with one file repaired it stands
    std::fs::remove_file(tree(9).join("k0.rs")).unwrap();
    stands(9, "one part repaired");
    assert!(remembered().len() <= 2_048);
}

/// Review V4 round 15, VD4R15-M-1, as reproduced. A commit with a credential has had no
/// verdict five times and waits for its turn. Three later commits get no verdict either, and
/// the judge has no probe to answer (a policy with no line): they stood in front, the asking
/// of every stop ended with them, their attempts never counted, and the commit that waited
/// was not submitted again, although the judge would have failed it. The ask of the commit
/// whose turn it is comes first.
#[test]
fn a_commit_that_waits_for_its_turn_is_asked_about_whatever_stands_in_front() {
    let scratch = Scratch::new("hook-turn-first");
    let root = repository(&scratch);
    let (script, log) = reading_judge(&scratch);
    std::fs::write(root.join("warden.toml"), "").unwrap();
    commit_all(&root, "a policy with no line");
    start(&root, "s");
    secret(&root, "k.rs");
    let key = commit_all(&root, "a key");
    // no verdict for the key five times, at stops where another commit gets one
    std::fs::write(scratch.join("judge.flaky"), "5\n").unwrap();
    for stop in 1..=5 {
        std::fs::write(root.join(format!("fine{stop}.rs")), "fn f() {}\n").unwrap();
        commit_all(&root, "harmless");
        let said = allowed(&stop_from(&root, "s", &script)).to_owned();
        assert!(
            said.contains(&format!("has no verdict: {key} (")),
            "stop {stop}: {said}"
        );
    }
    assert!(
        !scratch.join("judge.flaky").exists(),
        "five times no verdict"
    );
    // three commits the judge gives no verdict for
    for n in 0..3 {
        unjudgeable(&root, &format!("odd{n}.rs"));
    }
    let before = shown(&log).len();
    let reason = blocked(&stop_from(&root, "s", &script)).to_owned();
    assert!(
        reason.contains(&format!("Commit {key}: FAIL\n")),
        "{reason}"
    );
    assert_eq!(
        shown(&log)[before],
        "k.rs",
        "the commit whose turn it is was not asked first"
    );
}

/// The property that holds the order of asking: every commit without a verdict is submitted
/// to the judge at least once in as many stops as there are such commits, whatever the
/// judge answers. Here it never answers for eight commits: three have had five attempts and
/// take turns, five have fewer. No other patch gets a verdict and the policy has no probe,
/// so no attempt counts and the asking of each stop ends after three asks that failed. The
/// submissions are counted from the judge's own log.
#[test]
fn every_commit_without_a_verdict_is_submitted_in_turn() {
    let scratch = Scratch::new("hook-every-commit");
    let root = repository(&scratch);
    let (script, log) = reading_judge(&scratch);
    std::fs::write(root.join("warden.toml"), "").unwrap();
    commit_all(&root, "a policy with no line");
    start(&root, "s");
    let slow: Vec<String> = (0..3).map(|n| format!("slow{n}.rs")).collect();
    for name in &slow {
        unjudgeable(&root, name);
    }
    // until the three have had five attempts that count (a harmless commit gets a verdict
    // at each of these stops), which their line says
    for stop in 0..12 {
        std::fs::write(root.join(format!("fine{stop}.rs")), "fn f() {}\n").unwrap();
        commit_all(&root, "harmless");
        let said = allowed(&stop_from(&root, "s", &script)).to_owned();
        if said.contains("it is asked about again in its turn") {
            break;
        }
        assert!(stop < 11, "the three never had five attempts: {said}");
    }
    let fast: Vec<String> = (0..5).map(|n| format!("fast{n}.rs")).collect();
    for name in &fast {
        unjudgeable(&root, name);
    }
    // which commits each of twenty stops submits
    let mut stops: Vec<Vec<String>> = Vec::new();
    let file_of = |id: &str| -> String {
        git(&root, &["show", "--format=", "--name-only", id])
            .trim()
            .to_owned()
    };
    for _ in 0..20 {
        let before = shown(&log).len();
        let written = judged_lines();
        // every one of the eight is named or counted at every stop: on the line of those
        // with no verdict, or among those that wait to be asked about for the first time
        let said = allowed(&stop_from(&root, "s", &script)).to_owned();
        let count = |words: &str| -> usize {
            said.lines()
                .find_map(|line| line.split_once(words)?.0.trim().parse().ok())
                .unwrap_or(0)
        };
        assert_eq!(
            count(" commits of this session have no verdict: ")
                + count(" commits of this session: UNJUDGED")
                + count(" commit of this session: UNJUDGED"),
            8,
            "{said}"
        );
        stops.push(shown(&log)[before..].to_vec());
        // Eighth independent review of the hooks, HR8-L-1: what the session's file says
        // was asked at this stop is what the judge was shown at this stop, commit by commit.
        let submitted = stops.last().unwrap();
        for (id, line) in judged_lines() {
            if written.get(&id) != Some(&line) {
                assert!(
                    submitted.contains(&file_of(&id)),
                    "{id} is written down as asked ({line}) and the judge was not shown it: {submitted:?}"
                );
            }
        }
    }
    let waited = |name: &String| -> usize {
        // the longest run of stops in which it was not submitted
        let mut longest = 0;
        let mut run = 0;
        for submitted in &stops {
            run = if submitted.contains(name) { 0 } else { run + 1 };
            longest = longest.max(run);
        }
        longest
    };
    for name in &slow {
        assert!(
            waited(name) < 3,
            "{name} waited {} stops: {stops:?}",
            waited(name)
        );
    }
    for name in &fast {
        assert!(
            waited(name) < 5,
            "{name} waited {} stops: {stops:?}",
            waited(name)
        );
    }
    // and one stop asks about one commit in its turn, and at most three others
    assert!(
        stops.iter().all(|submitted| submitted.len() <= 4),
        "{stops:?}"
    );
}

/// Seventh independent review of the hooks, HR7-L-5. A linked working tree inside the
/// project's own tree (Claude Code's `.claude/worktrees/`) was a "nested repository left
/// out": it was said to be not judged at every stop, also at the stop made from inside it
/// that had judged it, and from the project it was counted twice. It is a working tree of
/// the project: judged from inside, and named from outside only while it has changes.
#[test]
fn a_working_tree_inside_the_project_is_a_working_tree_of_it() {
    let scratch = Scratch::new("hook-inner-tree");
    let root = repository(&scratch);
    let (script, _) = reading_judge(&scratch);
    let inner = root.join(".claude/worktrees/feat");
    std::fs::create_dir_all(inner.parent().unwrap()).unwrap();
    git(
        &root,
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "feat",
            inner.to_str().unwrap(),
        ],
    );
    start(&root, "s");
    // nothing changed anywhere: nothing to say, from either
    assert_eq!(stop_from(&root, "s", &script), Value::Null);
    assert_eq!(stop_from(&inner, "s", &script), Value::Null);
    // a harmless change inside it: judged from inside (and passes), named once from outside
    std::fs::write(inner.join("new.rs"), "fn n() {}\n").unwrap();
    assert_eq!(stop_from(&inner, "s", &script), Value::Null);
    let said = allowed(&stop_from(&root, "s", &script)).to_owned();
    assert!(
        said.contains(
            "has changes that are not committed, and the session's directory is not in it"
        ) && !said.contains("left out")
            && !said.contains("things in this project"),
        "{said}"
    );
    let again = allowed(&stop_from(&root, "s", &script)).to_owned();
    assert!(
        again.starts_with(
            "deep-diff-forge: 1 thing in this project is not judged: the working tree \""
        ),
        "{again}"
    );
    // a failure inside it blocks from inside, and from outside it is said to stand there,
    // at every stop
    secret(&inner, "k.rs");
    blocked(&stop_from(&inner, "s", &script));
    for _ in 0..3 {
        let outside = allowed(&stop_from(&root, "s", &script)).to_owned();
        assert!(
            outside.contains("A failure there has blocked a stop of this session and stands")
                || outside.contains("(a failure stands there)"),
            "{outside}"
        );
    }
    assert!(
        allowed(&stop_from(&inner, "s", &script)).starts_with("deep-diff-forge: FAIL STANDS. ")
    );
}

/// Seventh independent review of the hooks, HR7-L-6, and the sixth, HR6-M-2, through a judge:
/// what holds a commit is said at every stop. A commit held for a note, which is no rule,
/// kept no reason: later stops named it and no more. And a commit that fails for one rule
/// and is held for another names both for as long as it stands.
#[test]
fn what_a_commit_is_held_for_is_said_at_every_stop() {
    let scratch = Scratch::new("hook-held-for");
    let root = repository(&scratch);
    let (script, _) = listing_judge(&scratch);
    start(&root, "s");
    std::fs::write(root.join("n.rs"), "// NOTEME\n").unwrap();
    let noted = commit_all(&root, "held for a note");
    let first = allowed(&stop_from(&root, "s", &script)).to_owned();
    assert!(
        first.starts_with("deep-diff-forge: HOLD. ")
            && first.contains("a note of the stand-in judge"),
        "{first}"
    );
    for _ in 0..2 {
        let later = allowed(&stop_from(&root, "s", &script)).to_owned();
        assert!(
            later.contains(&format!(
                "1 held by the judge, for a person: Commit {noted} (the judge's words: "
            )) && later.contains("patch_unverifiable"),
            "{later}"
        );
    }
    std::fs::write(
        root.join("both.rs"),
        "const KEY: &str = \"SECRET\";\n// HOLDME\n",
    )
    .unwrap();
    let both = commit_all(&root, "fails and is held");
    let reason = blocked(&stop_from(&root, "s", &script)).to_owned();
    assert!(
        reason.contains(&format!("Commit {both}: FAIL\n- rule no-secrets: "))
            && reason.contains("- rule no-silent-suppression (held, for a person): "),
        "{reason}"
    );
    for _ in 0..2 {
        let later = allowed(&stop_from(&root, "s", &script)).to_owned();
        assert!(
            later.contains(&format!(
                "Commit {both}: FAIL, which stands\n- rule no-secrets\n- rule no-silent-suppression (held, for a person)\n"
            )),
            "{later}"
        );
    }
}

/// A change that is stashed and brought back blocks three times, and each block is for
/// something else: the uncommitted change; the stash's commit, which is a commit of the
/// session that holds the credential (a commit is judged once, by its own change, whatever
/// has blocked as an uncommitted change: its lines are a rule, a path and a count, and
/// another credential in the same file has the same); and the change again when it is back
/// in the working tree, which a stop had seen without it. Stated, so that it is a decision
/// and not an accident (seventh independent review of the hooks).
#[test]
fn a_stashed_change_blocks_as_a_commit_and_again_when_it_is_back() {
    let scratch = Scratch::new("hook-stash-round-trip");
    let root = repository(&scratch);
    let (script, _) = listing_judge(&scratch);
    start(&root, "s");
    secrets(&root, "k.rs", 1);
    let tree = blocked(&stop_from(&root, "s", &script)).to_owned();
    assert!(!tree.contains("Commit "), "{tree}");
    git(&root, &["stash", "push", "--quiet", "--include-untracked"]);
    let stash = blocked(&stop_from(&root, "s", &script)).to_owned();
    assert!(
        stash.contains(": FAIL\n") && stash.contains("Commit "),
        "{stash}"
    );
    git(&root, &["stash", "pop", "--quiet"]);
    let back = blocked(&stop_from(&root, "s", &script)).to_owned();
    assert!(back.contains("secret_like in \"k.rs\""), "{back}");
    // and then nothing blocks: the change stands, and so does the stash's commit
    let after = allowed(&stop_from(&root, "s", &script)).to_owned();
    assert!(
        after.starts_with("deep-diff-forge: FAIL STANDS. "),
        "{after}"
    );
}

/// Review V4 round 15, VD4R15-M-3. Two stops that were both their session's first hook took
/// no lock, because there was no state yet: both judged, both blocked for the same failure,
/// both wrote. Every stop takes the session's lock, which is named from the session's id.
#[test]
fn two_first_stops_of_a_session_do_not_run_at_once() {
    let scratch = Scratch::new("hook-first-stops");
    let root = repository(&scratch);
    let (script, log) = reading_judge(&scratch);
    secret(&root, "k.rs");
    std::fs::write(scratch.join("judge.sleep"), "5\n").unwrap();
    let both: Vec<Value> = std::thread::scope(|scope| {
        let stops: Vec<_> = (0..2)
            .map(|_| {
                scope.spawn(|| {
                    home_in(&scratch);
                    stop_from(&root, "first", &script)
                })
            })
            .collect();
        stops.into_iter().map(|stop| stop.join().unwrap()).collect()
    });
    let blocks = both
        .iter()
        .filter(|said| said["decision"] == "block")
        .count();
    let waited = both
        .iter()
        .filter(|said| {
            said["systemMessage"]
                .as_str()
                .is_some_and(|text| text.contains("another hook of this session is running"))
        })
        .count();
    assert_eq!((blocks, waited), (1, 1), "{both:?}");
    assert_eq!(runs(&log), 1, "both stops ran the judge");
}

/// A lock that outlives its holder would stop a session for good. The lock is a `flock`,
/// which ends with the process that holds it: a stop that is killed while its judge runs
/// holds nothing, and has written nothing, so the next stop judges everything and blocks.
#[test]
fn a_stop_that_is_killed_holds_no_lock_and_loses_no_turn() {
    let scratch = Scratch::new("hook-killed-stop");
    let root = repository(&scratch);
    let (script, log) = reading_judge(&scratch);
    start(&root, "s");
    secret(&root, "k.rs");
    let key = commit_all(&root, "a key");
    std::fs::write(scratch.join("judge.sleep"), "6\n").unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"));
    command
        .envs(ISOLATE)
        .env("HOME", home())
        .env_remove("XDG_STATE_HOME")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("CLAUDE_PROJECT_DIR")
        .args(["agent", "hook", "stop", "--client", "claude", "--judge"])
        .arg(&script)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut stop = command.spawn().unwrap();
    stop.stdin
        .take()
        .unwrap()
        .write_all(input("Stop", &root, "s", false).to_string().as_bytes())
        .unwrap();
    let asked = std::time::Instant::now();
    while runs(&log) == 0 {
        assert!(
            asked.elapsed() < std::time::Duration::from_secs(20),
            "the stop did not reach its judge in twenty seconds"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    stop.kill().unwrap();
    stop.wait().unwrap();
    std::fs::remove_file(scratch.join("judge.sleep")).unwrap();
    let reason = blocked(&stop_from(&root, "s", &script)).to_owned();
    assert!(
        reason.contains(&format!("Commit {key}: FAIL\n")),
        "{reason}"
    );
}

/// Seventh independent review of the hooks, HR7-L-4. An advice hook read the session's
/// state, took its time with Git, and wrote the whole state back: with it the list of what
/// had blocked as it was before a stop, in between, had seen the failure repaired and
/// forgotten it. The failure "had blocked" again, and did not block when it came back. An
/// advice hook writes what it owns onto the state as it is when it writes.
#[test]
fn an_advice_hook_does_not_write_back_what_a_stop_has_forgotten() {
    let scratch = Scratch::new("hook-advice-late");
    let root = repository(&scratch);
    let (script, _) = reading_judge(&scratch);
    // a Git that is slow once, for the advice hook alone
    let slow = scratch.join("slow");
    std::fs::create_dir(&slow).unwrap();
    let real = String::from_utf8(
        Command::new("sh")
            .args(["-c", "command -v git"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    let mark = scratch.join("slow.mark");
    write_program(
        &slow.join("git"),
        &format!(
            "#!/bin/sh\nif [ ! -e '{mark}' ]; then : > '{mark}'; sleep 3; fi\nexec '{real}' \"$@\"\n",
            mark = mark.display(),
            real = real.trim()
        ),
    );
    start(&root, "s");
    silence_a_lint(&root);
    secret(&root, "k.rs");
    blocked(&stop_from(&root, "s", &script));
    std::thread::scope(|scope| {
        let advice = scope.spawn(|| {
            home_in(&scratch);
            let path = format!("{}:{}", slow.display(), std::env::var("PATH").unwrap());
            hook_in(
                "post-tool",
                &[],
                &input("PostToolUse", &root, "s", false),
                |command| {
                    command.env("PATH", path);
                },
            )
        });
        // the advice hook has read the state and waits for its Git
        let asked = std::time::Instant::now();
        while !mark.exists() {
            assert!(
                asked.elapsed() < std::time::Duration::from_secs(20),
                "the advice hook did not reach Git in twenty seconds"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        // meanwhile the key is repaired, and a stop sees that
        std::fs::remove_file(root.join("k.rs")).unwrap();
        assert_eq!(stop_from(&root, "s", &script), Value::Null);
        advice.join().unwrap();
    });
    // the key is back: it blocks, as a failure that had gone
    secret(&root, "k.rs");
    blocked(&stop_from(&root, "s", &script));
}

/// The sixteen working trees a stop looks at, and the memory of failures: a failure that has
/// blocked in the last of eighteen linked trees is not forgotten by a stop from the project,
/// and a stop made from inside the tree afterwards does not block again. And it is said:
/// the trees were looked at in the order Git lists them, so this one was beyond the sixteen,
/// only counted, and no stop from the project said that a failure stands (eighth
/// independent review of the hooks, HR8-L-3). The trees where a failure stands are looked at
/// and named first, at every stop, whatever else there is to name.
#[test]
fn a_failure_in_a_tree_beyond_the_sixteen_stands() {
    let scratch = Scratch::new("hook-beyond-sixteen");
    let root = repository(&scratch);
    let (script, _) = reading_judge(&scratch);
    let add = |name: &str| -> PathBuf {
        let tree = scratch.join(name);
        git(
            &root,
            &[
                "worktree",
                "add",
                "--quiet",
                "--detach",
                tree.to_str().unwrap(),
            ],
        );
        tree
    };
    for n in 0..18 {
        add(&format!("tree{n:02}"));
    }
    // In the order Git lists them (the project first): the failure is in the tree listed
    // last, which is beyond the sixteen in that order, and four of the trees listed first
    // have changes of their own, which are named too.
    let listed: Vec<PathBuf> = git(&root, &["worktree", "list", "--porcelain"])
        .lines()
        .filter_map(|line| line.strip_prefix("worktree "))
        .map(PathBuf::from)
        .collect();
    assert_eq!(listed.len(), 19);
    for tree in &listed[1..5] {
        std::fs::write(tree.join("new.rs"), "fn n() {}\n").unwrap();
    }
    let last = listed[18].clone();
    start(&root, "s");
    secret(&last, "k.rs");
    blocked(&stop_from(&last, "s", &script));
    let shown = format!(
        "the working tree \"{}\" (a failure stands there)",
        std::fs::canonicalize(&last).unwrap().display()
    );
    let outside = allowed(&stop_from(&root, "s", &script)).to_owned();
    assert!(
        outside.contains("This project has 2 more working trees than the 16 a stop looks at")
            && outside.contains(
                "A failure there has blocked a stop of this session and stands: it was not looked at at this stop."
            ),
        "{outside}"
    );
    for stop in 2..=3 {
        let outside = allowed(&stop_from(&root, "s", &script)).to_owned();
        assert!(
            outside.contains(&format!(
                "6 things in this project are not judged: {shown}, "
            )),
            "stop {stop}: {outside}"
        );
    }
    let inside = allowed(&stop_from(&last, "s", &script)).to_owned();
    assert!(
        inside.starts_with("deep-diff-forge: FAIL STANDS. "),
        "{inside}"
    );
}
