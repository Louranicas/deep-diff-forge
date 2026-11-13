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

/// Run `deep-diff-forge agent hook <event> --client claude <extra…>` with `input` on stdin.
fn hook(event: &str, extra: &[&str], input: &Value) -> (i32, String, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .envs(ISOLATE)
        .args(["agent", "hook", event, "--client", "claude"])
        .args(extra)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
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

/// A repository with one commit: a source file and a policy.
fn repository(scratch: &Scratch) -> PathBuf {
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
/// with the code in `exit_file` (so a test can change the answer between runs).
fn judge(scratch: &Scratch, verdict: &Value) -> (PathBuf, PathBuf, PathBuf) {
    let (script, log, exit_file) = (
        scratch.join("judge"),
        scratch.join("judge.log"),
        scratch.join("judge.exit"),
    );
    std::fs::write(&exit_file, "0").unwrap();
    let body = format!(
        "#!/bin/sh\n\
         printf 'ARGS %s\\n' \"$*\" >> '{log}'\n\
         while [ $# -gt 0 ]; do\n\
           case \"$1\" in\n\
             --policy) printf 'POLICY %s' \"$(cat \"$2\")\" >> '{log}'; printf '\\n' >> '{log}';;\n\
             --patch) printf 'PATCH-FILES %s\\n' \"$(grep -c '^diff --git' \"$2\")\" >> '{log}';;\n\
           esac\n\
           shift\n\
         done\n\
         cat <<'VERDICT'\n{verdict}\nVERDICT\n\
         exit \"$(cat '{exit}')\"\n",
        log = log.display(),
        exit = exit_file.display(),
    );
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
    (script, log, exit_file)
}

fn verdict(outcome: &str, rule: &str, signal: &str, path: &str) -> Value {
    json!({"schema": "forge-warden.verdict.v2", "verdict": outcome, "rules": [
        {"id": rule, "kind": "forbid_signal", "severity": outcome, "outcome": outcome,
         "why": "A silenced check hides the next real bug.",
         "violations": [{"signal": signal, "path": path, "message": "m"}]}
    ]})
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
    // a wrong event or client is a usage error, exit 2, nothing on stdout
    for args in [
        vec!["agent", "hook", "pre-tool", "--client", "claude"],
        vec!["agent", "hook", "stop"],
        vec!["agent", "hook", "stop", "--client", "codex"],
        vec!["agent", "hook"],
    ] {
        let out = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
            .args(&args)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        assert!(out.stdout.is_empty(), "{args:?}");
    }
}

#[test]
fn a_stop_is_judged_against_the_policy_at_the_base_commit() {
    let scratch = Scratch::new("hook-stop");
    let root = repository(&scratch);
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

/// The judge refuses this binary as a ranker (exit 4) and admits one from `--ranker-dir`.
#[test]
fn a_stop_offers_other_rankers_when_the_policy_does_not_admit_this_one() {
    let scratch = Scratch::new("hook-rankers");
    let root = repository(&scratch);
    std::fs::write(
        root.join("a.rs"),
        "fn a() {}\n#[allow(dead_code)]\nfn b() {}\n",
    )
    .unwrap();
    let rankers = scratch.join("rankers");
    std::fs::create_dir(&rankers).unwrap();
    std::fs::write(rankers.join("ddf-0.8.4"), "stand-in ranker\n").unwrap();
    let log = scratch.join("judge.log");
    let script = scratch.join("judge");
    // exit 4 unless the ranker is the admitted one, then pass
    let body = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\ncase \"$*\" in *ddf-0.8.4*) exit 0;; *) exit 4;; esac\n",
        log.display()
    );
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
            .contains("admits no ranker this machine has"),
        "{out}"
    );
}

#[test]
fn session_start_states_the_contract_and_whether_a_stop_can_be_judged() {
    let scratch = Scratch::new("hook-start");
    let root = repository(&scratch);
    let event = input("SessionStart", &root, "s", false);
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
        stdout.contains("forge-warden was not found: stops will be reported as UNJUDGED."),
        "{stdout}"
    );
    let (script, _, _) = judge(&scratch, &json!({}));
    let (_, stdout, _) = hook(
        "session-start",
        &["--judge", script.to_str().unwrap()],
        &event,
    );
    assert!(
        stdout.contains("A judge and a policy were found: stops will be judged."),
        "{stdout}"
    );
    let (_, stdout, _) = hook(
        "session-start",
        &["--judge", script.to_str().unwrap(), "--policy", "none.toml"],
        &event,
    );
    assert!(
        stdout.contains("HEAD has no policy file: stops will be reported as UNJUDGED."),
        "{stdout}"
    );
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
