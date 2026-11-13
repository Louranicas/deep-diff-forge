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

/// When the judge gives no verdict, what it said about why reaches the user. A live run with a
/// real forge-warden found this: a host with no Git allowlist enrolled was reported as a ranker
/// failure, because only the exit code was read.
#[test]
fn an_unjudged_stop_passes_on_what_the_judge_said() {
    let scratch = Scratch::new("hook-judge-words");
    let root = repository(&scratch);
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

const GATE: [&str; 7] = [
    "git",
    "repository",
    "policy",
    "judge",
    "chain",
    "policy_protects_itself",
    "hooks_registered",
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

/// The seven gate checks with these answers, in order.
fn gate_names(ok: [bool; 7]) -> Vec<(String, bool)> {
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
    let home = scratch.join("home");
    std::fs::create_dir(&home).unwrap();
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
    assert_eq!(
        gate_checks(&doc),
        gate_names([true, true, true, true, true, true, false])
    );
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
            (7, gate_names([true, true, true, true, true, true, false])),
            "{command}"
        );
        assert!(
            detail(&doc, "hooks_registered").contains(why),
            "{command}: {doc:#}"
        );
    }
    std::fs::write(&local, stop_hook_settings(&script)).unwrap();
    let (code, doc, stderr) = doctor_gate(&root, &home, &judge_arg);
    assert_eq!(gate_checks(&doc), gate_names([true; 7]));
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
    assert_eq!((code, gate_checks(&doc)), (0, gate_names([true; 7])));

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
    assert_eq!((code, gate_checks(&doc)), (0, gate_names([true; 7])));

    // a policy that lets a change to itself pass is not ready
    std::fs::write(&exit_file, "0").unwrap();
    let (code, doc, _) = doctor_gate(&root, &home, &judge_arg);
    assert_eq!(
        (code, gate_checks(&doc)),
        (7, gate_names([true, true, true, true, true, false, true]))
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
        (7, gate_names([true, true, true, true, false, false, true]))
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
            gate_names([true, true, true, false, false, false, false])
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
            gate_names([true, true, false, true, false, false, false])
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
            gate_names([true, false, false, true, false, false, false])
        )
    );
}

/// Run a hook with `path` as the whole `PATH` (plus the system's, so `git` and `sh` are found).
fn hook_on_path(event: &str, path: &[&Path], input: &Value) -> (i32, String, String) {
    let mut dirs: Vec<PathBuf> = path.iter().map(|dir| dir.to_path_buf()).collect();
    dirs.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    let mut child = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .envs(ISOLATE)
        .env("PATH", std::env::join_paths(dirs).unwrap())
        .args(["agent", "hook", event, "--client", "claude"])
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
    std::fs::copy(&script, bin.join("forge-warden.new")).unwrap();
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

/// Review V4, VD4-M-1: the hook's directory under the Git directory must be a directory of its
/// own, and a state file is read only if it is a small regular file.
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

    // a symbolic link where the directory should be: nothing is written through it
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

    // the directory is made for this user only, and the state file too
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
    let state: Vec<PathBuf> = std::fs::read_dir(&own)
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
