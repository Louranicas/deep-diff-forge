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
    name_what_the_hooks_met();
    home
}

/// What the hooks of a test met that is not of the code under test and that a later assertion
/// cannot name (lane P5b, measurement M7 of 0.11.1: `tests/LOAD_SENSITIVE`), by the test's home.
/// Until package P5c of 0.11.1 a session's first hook took no lock when a `.git` stood above
/// the hooks' state directory, and wrote the state without it; a hook that leaves a state and
/// no lock file now breaks the rule that a state is written only under the session's lock. A
/// test of the session's lock or of what is in its directory then fails far from the hook, on
/// a count: no lock file, one file short, two first stops at once, "the state directory was
/// written". (Measured: an empty `.git` in the home directory stood for seconds at a time
/// while another agent's sandboxed commands ran.)
static MET: std::sync::Mutex<Vec<(PathBuf, String)>> = std::sync::Mutex::new(Vec::new());

/// The first `.git` (a directory, a file or a link) in an ancestor of `dir`, the part of `dir`
/// that exists taken through its links, as the hooks take it.
fn git_above(dir: &Path) -> Option<PathBuf> {
    let existing = dir.ancestors().find(|at| at.exists())?;
    std::fs::canonicalize(existing)
        .ok()?
        .ancestors()
        .map(|at| at.join(".git"))
        .find(|git| git.symlink_metadata().is_ok())
}

/// Note for a failure of this test what a hook met ([`MET`]) that ran with no state directory
/// `dir` yet: a `.git` above it before or after it ran, and whether it took no lock (it left a
/// state and no lock file there).
fn note_what_a_hook_met(event: &str, input: &Value, dir: &Path, above: Option<PathBuf>) {
    let kinds: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .collect();
    let has = |kind: &str| {
        kinds
            .iter()
            .any(|path| path.extension() == Some(std::ffi::OsStr::new(kind)))
    };
    let lockless = has("json") && !has("lock");
    let hook = format!(
        "the `{event}` hook of session {}, which made the hook state directory {},",
        input["session_id"],
        dir.display()
    );
    let note = match (above, lockless) {
        (None, false) => return,
        (None, true) => format!("{hook} took no lock: it left a state and no lock file there"),
        (Some(git), _) => format!(
            "{hook} {} while a .git stood at {} above that directory",
            if lockless {
                "took no lock: it left a state and no lock file there"
            } else {
                "ran"
            },
            git.display()
        ),
    };
    if let Ok(mut met) = MET.lock() {
        met.push((home(), note));
    }
}

/// Once per test binary: the panic of a test whose hooks met what [`MET`] holds says it under
/// the panic's own message.
fn name_what_the_hooks_met() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let before = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            before(info);
            let Some(home) = HOME.with(|slot| slot.borrow().clone()) else {
                return;
            };
            if let Ok(met) = MET.try_lock() {
                for (_, note) in met.iter().filter(|(at, _)| *at == home) {
                    eprintln!("note (not of the code under test): {note}");
                }
            }
        }));
    });
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
    // Where the hook keeps the state (an absolute XDG_STATE_HOME, else HOME's), when it will
    // make that directory: what it meets there is noted ([`note_what_a_hook_met`]).
    let set = |key: &str| {
        command
            .get_envs()
            .find(|(name, _)| *name == key)
            .and_then(|(_, value)| value.map(PathBuf::from))
    };
    let made = match (set("XDG_STATE_HOME"), set("HOME")) {
        (Some(xdg), _) if xdg.is_absolute() => Some(xdg.join("deep-diff-forge/hooks")),
        (_, Some(home)) => Some(home.join(".local/state/deep-diff-forge/hooks")),
        _ => None,
    }
    .filter(|dir| !dir.exists())
    .map(|dir| {
        let above = git_above(&dir);
        (dir, above)
    });
    let mut child = command.spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.to_string().as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    if let Some((dir, above)) = made {
        let above = above.or_else(|| git_above(&dir));
        note_what_a_hook_met(event, input, &dir, above);
    }
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
if [ "$1" = check ] && [ "$2" = --help ]; then cat '@HELP@' 2>/dev/null; exit 0; fi
printf 'ARGS %s\n' "$*" >> '@LOG@'
policy=; patch=; ranker=; repo=; base=; head=
while [ $# -gt 0 ]; do
  case "$1" in
    --repo) repo="$2";;
    --base) base="$2";;
    --head) head="$2";;
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
# bound mode (S3.25.b): refuse it when told to; else claim the pair, or another when told to
if [ -n "$repo" ] && [ -f '@HELP@.refuse' ]; then echo '{"exit":4,"name":"invalid_input","message":"the patch is not git diff base...head","next":"change_input"}' >&2; exit 4; fi
v3='s/^$//'
if [ -n "$repo" ]; then [ -f '@HELP@.liar' ] && head=0000000000000000000000000000000000000000; seals="${seals%\}},\"claims\":{\"base\":\"$base\",\"head\":\"$head\",\"merge_base\":\"$base\",\"verified\":true}}"; v3='s/"forge-warden.verdict.v2"/"forge-warden.verdict.v3"/'; fi
sed -e "$v3" -e "s/\"@VERDICT@\"/\"$said\"/g" -e "s/\"@CODE@\"/$code/" -e "s/\"@NAME@\"/\"$name\"/" \
    -e "s/\"@ACTION@\"/\"$action\"/" -e "s|\"@SEALS@\"|$seals|" -e "s/\"@COUNTS@\":0/$counts/" <<'VERDICT'
@VERDICT_DOCUMENT@
VERDICT
exit "$code"
"#;
    SCRIPT
        .replace(
            "@HELP@",
            &log.with_file_name("judge.help").display().to_string(),
        )
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
            "- \"a.rs\":2: check_suppressed@line (integrity, new side, read as lexical); repairs: check_suppressed:a1, check_suppressed:a2.\n"
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
            "your uncommitted and staged changes, and each commit made since this session began (commit {}) that your branches, tags, stash, HEAD or their history name, with each commit those name that came after the project's ledger first recorded the project and has no stop's verdict there (a session that ended before its stop left it), are judged",
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
        stdout.contains(&format!("- The chain was checked the way `{} agent doctor --gate` checks it: stops will be judged.\n", this_binary().display())),
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

/// Lane D1's finding: `agent doctor --gate --ranker-dir DIR` compared DIR as text with the
/// registered stop's, so a relative directory never matched the absolute one `agent setup
/// --hooks` writes. A stop lists its ranker directory with `read_dir`, which follows every link
/// of the path; the gate now compares the directories that resolves to: a relative path, a
/// trailing `/` and a link to the directory are the stop's; another directory, and one that
/// does not exist, are not.
#[test]
fn doctor_gate_compares_the_ranker_directory_as_the_stop_opens_it() {
    let (scratch, root, home, script, _log, _exit_file) = gate_fixture("doctor-gate-ranker-dir");
    let rankers = scratch.join("rankers");
    std::fs::create_dir(&rankers).unwrap();
    std::fs::create_dir(scratch.join("other")).unwrap();
    std::os::unix::fs::symlink(&rankers, scratch.join("rankers-link")).unwrap();
    let resolved = std::fs::canonicalize(&rankers).unwrap();
    std::fs::create_dir(root.join(".claude")).unwrap();
    std::fs::write(
        root.join(".claude/settings.local.json"),
        settings_with(&format!(
            "{} --ranker-dir '{}'",
            stop_command(&script),
            resolved.display()
        )),
    )
    .unwrap();
    let judge = script.to_str().unwrap();
    for (given, same) in [
        (resolved.display().to_string(), true),
        ("../rankers".to_owned(), true),
        ("../rankers/".to_owned(), true),
        (format!("{}/", rankers.display()), true),
        ("../rankers-link".to_owned(), true),
        ("../other".to_owned(), false),
        ("../not-there".to_owned(), false),
    ] {
        let (code, doc, _) = doctor_gate(&root, &home, &["--judge", judge, "--ranker-dir", &given]);
        if same {
            assert_eq!(
                (code, gate_checks(&doc)),
                (0, gate_but(&[])),
                "{given}: {}",
                detail(&doc, "hooks_registered")
            );
        } else {
            assert_eq!(code, 7, "{given}");
            assert!(
                detail(&doc, "hooks_registered").contains("offers other rankers"),
                "{given}: {doc:#}"
            );
        }
    }
}

/// `agent check`, `agent doctor --gate` and the stop agree on which ranker directory is the
/// registered stop's. The stop lists the directory its `--ranker-dir` resolves to from its own
/// directory, following links: here the stand-in judge admits only the ranker in that
/// directory, so a stop given a relative path, a trailing `/` or a link to it offers that
/// ranker and its judge answers (HOLD), and one given a directory that does not exist offers
/// none and is UNJUDGED (exit 4). The gate is ready for exactly the first four, and the check
/// says that the option is not the registered Stop hook's for exactly the last.
#[test]
fn check_doctor_and_the_stop_agree_on_the_ranker_directory() {
    let (scratch, root, home, holding, _log, _exit_file) = gate_fixture("ranker-dir-agree");
    let rankers = scratch.join("rankers");
    std::fs::create_dir(&rankers).unwrap();
    std::fs::write(rankers.join("ddf-0.8.4"), "stand-in ranker\n").unwrap();
    std::os::unix::fs::symlink(&rankers, scratch.join("rankers-link")).unwrap();
    let resolved = std::fs::canonicalize(&rankers).unwrap();
    let judge = scratch.join("admits-the-directory-ranker");
    write_program(
        &judge,
        &format!(
            "#!/bin/sh\ncase \"$*\" in *ddf-0.8.4*) exec '{}' \"$@\";; *) exit 4;; esac\n",
            holding.display()
        ),
    );
    std::fs::create_dir(root.join(".claude")).unwrap();
    std::fs::write(
        root.join(".claude/settings.local.json"),
        settings_with(&format!(
            "{} --ranker-dir '{}'",
            stop_command(&judge),
            resolved.display()
        )),
    )
    .unwrap();
    start(&root, "s");
    silence_a_lint(&root);
    let judge = judge.to_str().unwrap();
    for (given, same) in [
        ("../rankers", true),
        ("../rankers/", true),
        ("../rankers-link", true),
        ("../not-there", false),
    ] {
        // the stop, run from the project's directory as the client runs it
        let (code, stdout, _) = hook_in(
            "stop",
            &["--judge", judge, "--ranker-dir", given],
            &input("Stop", &root, "s", false),
            |command| {
                command.current_dir(&root);
            },
        );
        assert_eq!(code, 0);
        let said = system_message(&stdout);
        if same {
            assert!(
                said.starts_with("deep-diff-forge: HOLD. "),
                "stop {given}: {said}"
            );
        } else {
            assert!(
                said.contains("the judge refused the policy, the ranking or the ranker (exit 4)"),
                "stop {given}: {said}"
            );
        }
        // the gate
        let (code, doc, _) = doctor_gate(&root, &home, &["--judge", judge, "--ranker-dir", given]);
        let registered = detail(&doc, "hooks_registered");
        assert_eq!(code == 0, same, "doctor {given}: {registered}");
        assert_eq!(
            registered.contains("offers other rankers"),
            !same,
            "doctor {given}: {registered}"
        );
        // the check
        let out = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
            .current_dir(&root)
            .envs(ISOLATE)
            .env("HOME", &home)
            .args(["agent", "check", "--json", "--ranker-dir", given])
            .output()
            .unwrap();
        let doc: Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
            panic!(
                "check {given}: {e}: {}",
                String::from_utf8_lossy(&out.stderr)
            )
        });
        let differs = doc["stop"]["differences"]
            .as_array()
            .unwrap()
            .iter()
            .any(|line| line == "--ranker-dir is not the registered Stop hook's");
        assert_eq!(differs, !same, "check {given}: {doc}");
    }
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

/// The policy-protection link follows the rule outcomes, not merely the verdict's exit code.
/// In particular, verified-claims mode and expired waivers can make a real forge-warden
/// verdict non-pass without any rule protecting the policy file. A rule whose severity is
/// `fail` protects it just as a holding rule does.
#[test]
fn doctor_gate_reads_policy_self_protection_from_rule_outcomes() {
    let scratch = Scratch::new("doctor-gate-rule-protection");
    let root = repository(&scratch);
    let home = home();
    std::fs::create_dir(root.join(".claude")).unwrap();

    let non_rule_verdict = |require_verified: bool, expired: bool| {
        json!({
            "schema": "forge-warden.verdict.v2",
            "tool": {"name": "forge-warden", "version": "0.2.0"},
            "evaluated_on": "2026-11-25",
            "seals": "@SEALS@",
            "verdict": "@VERDICT@",
            "exit": {"code": "@CODE@", "name": "@NAME@", "action": "@ACTION@"},
            "summary": {"files": 1, "rules": 1, "passed": 1,
                        "waived": 0, "held": 0, "failed": 0},
            "rules": [{
                "id": "unrelated", "kind": "forbid_signal", "severity": "hold",
                "outcome": "pass",
                "why": "This unrelated rule does not protect the policy file.",
                "violations": []
            }],
            "expired_waivers": if expired {
                json!([{"index": 0, "rule": "unrelated", "owner": "reviewer",
                        "expires": "2020-01-01"}])
            } else {
                json!([])
            },
            "unused_waivers": [],
            "notes": if require_verified {
                json!([{"code": "claims_unverified",
                        "detail": "the patch claims were not bound to a repository"}])
            } else {
                json!([])
            },
        })
    };

    let assert_non_rule = |name: &str, require_verified: bool, expired: bool, exit: &str| {
        let judge_scratch = Scratch::new(name);
        let (script, _, exit_file) =
            judge(&judge_scratch, &non_rule_verdict(require_verified, expired));
        std::fs::write(&exit_file, exit).unwrap();
        std::fs::write(
            root.join(".claude/settings.local.json"),
            stop_hook_settings(&script),
        )
        .unwrap();
        let (code, doc, _) = doctor_gate(&root, &home, &["--judge", script.to_str().unwrap()]);
        assert_eq!(
            (code, gate_checks(&doc)),
            (7, gate_but(&["policy_protects_itself"])),
            "{name}: {doc:#}"
        );
        assert!(
            detail(&doc, "policy_protects_itself").contains("no rule holds or fails it"),
            "{name}: {doc:#}"
        );
    };

    // require_verified=true: the claims note holds the probe, but no rule protects the file.
    assert_non_rule("doctor-gate-claims-only", true, false, "8");
    // An expired waiver is not protection, whether verified claims are required or not.
    assert_non_rule("doctor-gate-expired-verified", true, true, "7");
    assert_non_rule("doctor-gate-expired-unverified", false, true, "7");

    let fail_scratch = Scratch::new("doctor-gate-failing-protection");
    let (fail_judge, _, fail_exit) = judge(
        &fail_scratch,
        &verdict(
            "fail",
            "policy-is-protected",
            "protected_path",
            "warden.toml",
        ),
    );
    std::fs::write(&fail_exit, "7").unwrap();
    std::fs::write(
        root.join(".claude/settings.local.json"),
        stop_hook_settings(&fail_judge),
    )
    .unwrap();
    let (code, doc, _) = doctor_gate(&root, &home, &["--judge", fail_judge.to_str().unwrap()]);
    assert_eq!((code, gate_checks(&doc)), (0, gate_but(&[])), "{doc:#}");
    assert!(
        detail(&doc, "policy_protects_itself").contains("held or fails under a rule"),
        "{doc:#}"
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
    // the trouble with the sessions' directory was said, which no state could remember; and
    // the key the mark is keyed with (F6), 32 bytes for the user alone.
    let mut left: Vec<PathBuf> = std::fs::read_dir(&own)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    left.sort();
    assert_eq!(left.len(), 2, "{left:?}");
    assert!(left[0].ends_with("key"), "{left:?}");
    assert_eq!(std::fs::metadata(&left[0]).unwrap().len(), 32);
    assert_eq!(mode(&left[0]), 0o600);
    left.remove(0);
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
/// the start says that stops judge without the state (C2 of the 0.11.0 tree review; it said
/// they would not be judged), and advice says it once.
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
            said.starts_with(&stateless("state.damaged"))
                && said.contains("Why: this session's state file ")
                && said.contains("cannot be read as a state (it cannot be opened")
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
    // (a schema older than the last release's is refused by its name since 0.11.0: the
    // hook-state version policy, rule V4; it said "not a state of this version" before)
    std::fs::write(&state[0], "{\"schema\":\"deep-diff-forge.hook-state.v1\"}").unwrap();
    let said = stop();
    assert!(
        said.contains("UNJUDGED")
            && said.contains(
                "is of schema v1, written by a deep-diff-forge older than the last release"
            ),
        "{said}"
    );
    let started = start(&root, "s");
    assert!(
        started.starts_with("deep-diff-forge: this session's state cannot be used, so no advice will be given and each stop judges the working tree")
            && started.contains("is of schema v1"),
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
///
/// A start that says it recorded nothing fails here, with its words and the seconds it took:
/// it ran out of its time (8 s), another hook of the session held the session's lock, or its
/// state could not be saved. Before, it passed this helper, and the test failed later on a
/// state that was never written, with a message that named nothing (lane P5b, measurement M7
/// of 0.11.1; `tests/LOAD_SENSITIVE`).
fn start(root: &Path, session: &str) -> String {
    let began = std::time::Instant::now();
    let (code, stdout, stderr) = hook(
        "session-start",
        &[],
        &input("SessionStart", root, session, false),
    );
    let took = began.elapsed().as_secs_f64();
    assert_eq!((code, stderr.as_str()), (0, ""), "{stdout}");
    let recorded_nothing = if stdout.starts_with("deep-diff-forge is watching the changes") {
        stdout.contains("- The start of this session could not be recorded (")
    } else {
        stdout.contains("in the time a hook has")
    };
    assert!(
        !recorded_nothing,
        "the start of session {session:?} recorded nothing (it took {took:.1} s; a start has 8 s): {stdout}"
    );
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

/// What a stop says, whether it blocked (its reason) or let the agent stop (its message).
fn stop_words(decision: &Value) -> &str {
    decision["reason"]
        .as_str()
        .or_else(|| decision["systemMessage"].as_str())
        .unwrap_or_else(|| panic!("the stop said nothing: {decision}"))
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
        reason.contains(&format!("Commit {work} (the HEAD reflog of "))
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
    // (where the project's ledger and its salt are kept since 0.11.0: F11)
    assert!(root.join(".git/deep-diff-forge").is_dir());
    assert_eq!(
        std::fs::read_dir(root.join(".git/deep-diff-forge"))
            .unwrap()
            .filter(|entry| {
                let name = entry.as_ref().unwrap().file_name();
                // (and the policy record and the project lock: release 0.11.1, L1c)
                ![
                    "ledger.jsonl",
                    "ledger.salt",
                    "policy.jsonl",
                    "project.lock",
                ]
                .iter()
                .any(|kept| name == *kept)
            })
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
        stdout.contains("\\\"a.rs\\\":2: check_suppressed@line") && stdout.contains(&since),
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
    // (F12: the marks hold the first stop once, then the stop is UNJUDGED as before)
    let held = stop_from(&root, "skip", &script);
    assert!(blocked(&held).contains("skip-worktree"), "{held}");
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
        let held = stop_from(&root, "unreadable", &script);
        assert!(blocked(&held).contains("\"unreadable.txt\""), "{held}");
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
        stdout.contains("\\\"build/gen.rs\\\":1: check_suppressed@line"),
        "{stdout}"
    );

    std::fs::write(root.join(".git/info/exclude"), "hidden.rs\n").unwrap();
    std::fs::write(root.join("hidden.rs"), "#[allow(unused)]\nfn h() {}\n").unwrap();
    git(&root, &["add", "-f", "hidden.rs"]);
    let (_, stdout, _) = hook("post-tool", &[], &input("PostToolUse", &root, "s", false));
    assert!(
        stdout.contains("\\\"hidden.rs\\\":1: check_suppressed@line"),
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
    // (and the project's ledger, which the first hook of a session writes: F13.b; the policy
    // record and the project lock, where the first hook binds the policy name: release 0.11.1)
    let mut names = names;
    names.sort();
    assert_eq!(
        names,
        ["ledger.jsonl", "policy.jsonl", "project.lock", "tmp-1"]
    );
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

    // a path marked skip-worktree: a stop here holds once, then is UNJUDGED (F12)
    git(&root, &["update-index", "--skip-worktree", "a.rs"]);
    let (found, doc) = gate();
    assert_eq!(found, all(&patch_fails));
    let why = detail(&doc, "patch");
    assert!(
        why.contains("a stop here would hold once, then be UNJUDGED")
            && why.contains("skip-worktree"),
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
            detail(&doc, "patch").contains("a stop here would hold once, then be UNJUDGED"),
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
    // the start changed nothing and left nothing behind but the project's ledger (F13.b), and
    // the policy record with the project lock, where it bound the policy name (release 0.11.1)
    assert_eq!(git(&root, &["status", "--porcelain=v1"]), " M a.rs\n");
    let mut left: Vec<String> = std::fs::read_dir(root.join(".git/deep-diff-forge"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    left.sort();
    assert_eq!(left, ["ledger.jsonl", "policy.jsonl", "project.lock"]);
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

/// Integrated review of 0.9.0, VI1-L-3 and CI5: the fuzz target `hook_input` cannot reach the
/// hook's reading of bytes, so for bytes that are not UTF-8 it checks only the form of the
/// refusal. Here the hook itself reads them, for each event: a JSON object that names the
/// event, with one byte that is not UTF-8, is refused in the hooks' own words, never judged.
#[test]
fn bytes_that_are_not_utf8_are_refused_by_every_event() {
    let scratch = Scratch::new("hook-not-utf8");
    let root = repository(&scratch);
    for (event, name) in [
        ("session-start", "SessionStart"),
        ("post-tool", "PostToolUse"),
        ("stop", "Stop"),
    ] {
        let mut bytes = input(name, &root, "s", false).to_string().into_bytes();
        let at = bytes.windows(3).position(|w| w == b"\"s\"").unwrap() + 1;
        bytes[at] = 0xff;
        let mut child = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
            .envs(ISOLATE)
            .env("HOME", home())
            .args(["agent", "hook", event, "--client", "claude"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(&bytes).unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(out.status.code(), Some(0), "{event}");
        assert!(out.stderr.is_empty(), "{event}");
        let said = system_message(std::str::from_utf8(&out.stdout).unwrap());
        let reason = "did not run: its input could not be read: stream did not contain valid UTF-8";
        assert!(
            said.contains(reason) && said.contains("Nothing was judged"),
            "{event}: {said}"
        );
        assert_eq!(
            said.starts_with("deep-diff-forge: UNJUDGED. ") && said.ends_with("not a pass."),
            event == "stop",
            "{event}: {said}"
        );
    }
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
    // (the directory's `.gitignore`, release 0.11.1 P5d, is no file of a session: left out by name)
    let files = |dir: &Path| {
        std::fs::read_dir(dir).map_or(0, |entries| {
            entries
                .filter(|entry| {
                    !entry
                        .as_ref()
                        .is_ok_and(|entry| entry.file_name() == ".gitignore")
                })
                .count()
        })
    };

    // an absolute XDG_STATE_HOME wins over the home directory; a relative one is not used
    let xdg = scratch.join("xdg-state");
    let (_, stdout, _) = hook_in("post-tool", &[], &event, |command| {
        command.env("XDG_STATE_HOME", &xdg);
    });
    assert!(stdout.contains("check_suppressed"), "{stdout}");
    // four files: the state, the list of commits that were there when the session began,
    // the session's lock, which a hook holds while it writes, and (since hook state v12) the
    // user's key its digests are made with
    assert_eq!(files(&xdg.join("deep-diff-forge/hooks")), 4);
    assert_eq!(files(&sessions()), 0);
    let (_, stdout, _) = hook_in("post-tool", &[], &event, |command| {
        command
            .env("XDG_STATE_HOME", "relative/state")
            .current_dir(&root);
    });
    assert!(stdout.contains("check_suppressed"), "{stdout}");
    assert_eq!(files(&sessions()), 4);
    assert!(!root.join("relative").exists());
    // nothing of a session's state is kept in the repository: only the project's ledger
    // (F13.b: its baseline and the start of each session), and the policy record with the
    // project lock, where the advice hook bound the policy name on first use (release 0.11.1)
    assert_eq!(files(&root.join(".git/deep-diff-forge")), 3);
    assert!(root.join(".git/deep-diff-forge/ledger.jsonl").is_file());
    assert!(root.join(".git/deep-diff-forge/policy.jsonl").is_file());

    // neither: said, for a stop as UNJUDGED, and never a block
    for (word, name, starts) in [
        (
            "post-tool",
            "PostToolUse",
            "deep-diff-forge: the hook did not run: ",
        ),
        // (F12.d: a stop judges the tree without the state; here no judge is named)
        (
            "stop",
            "Stop",
            "deep-diff-forge: this session's state could not be used (state.no_directory): ",
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
    const NOT_RECORDED: &str = "This session's start was not recorded: its first hook here ran after the session had begun, so work committed before that hook is judged only where the project's ledger has it as made after the project was first recorded and with no stop's verdict (a session that ended before its stop); otherwise it was not judged. For a session to be judged from its start, the session-start hook must be installed before the session begins";
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
    // F13.b: the work committed after the session "clean" recorded the project is judged,
    // once, by the next session's first stop; then it has its verdict in the ledger
    assert_eq!(runs(&log), 1, "the committed work was not judged once");
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
    const NOT_RECORDED: &str = "This session's start was not recorded: its first hook here ran after the session had begun, so work committed before that hook is judged only where the project's ledger has it as made after the project was first recorded and with no stop's verdict (a session that ended before its stop); otherwise it was not judged.";
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
            system_message(&run("stop", "Stop", "6a", &no_home)).starts_with(&format!(
                "{}{}",
                stateless("state.no_directory"),
                "no commit was judged: the project's ledger has no baseline"
            ))
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
    // (the session's lock may leave empty directories there, of which Git records none: P5c of
    // release 0.11.1, row C6)
    let left = files_left_under(&root.join(".state"));
    assert!(
        left.is_empty(),
        "a state was written into the working tree: {left:?}"
    );
    assert_eq!(git(&root, &["status", "--porcelain=v1"]), " M a.rs\n");
}

/// The files (anything that is not a directory) under `dir`, as paths; none when `dir` is not
/// there. A directory that cannot be read fails here: an assertion that nothing is left must
/// not pass on a listing it could not make.
fn files_left_under(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return found,
        Err(error) => panic!("{}: {error}", dir.display()),
    };
    for entry in entries {
        let path = entry.unwrap().path();
        if path.symlink_metadata().is_ok_and(|kind| kind.is_dir()) {
            found.extend(files_left_under(&path));
        } else {
            found.push(path);
        }
    }
    found
}

// ---- Round 11 of Vesper's review and the third independent review (2026-11-14, both NO-GO) ----

/// The first twelve digits of the commit `rev` names in `root`: how a stop names a commit.
fn short(root: &Path, rev: &str) -> String {
    git(root, &["rev-parse", rev])[..12].to_owned()
}

/// The uncommitted change of the working tree at `top` as a stop names it: the path quoted,
/// and past 120 characters cut in its middle (the first 60, `…`, the last 60) as `shown_path`
/// cuts every path. (This cut its end, which no message shows: under a temp directory as long
/// as macOS's, a test looked for words that were never there.)
fn uncommitted_in(top: &Path) -> String {
    let whole: Vec<char> = top.display().to_string().chars().collect();
    let shown: String = if whole.len() > 120 {
        whole[..60]
            .iter()
            .chain(['…'].iter())
            .chain(&whole[whole.len() - 60..])
            .collect()
    } else {
        whole.into_iter().collect()
    };
    format!("Uncommitted changes in \"{shown}\"")
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
/// file says (once when it is empty), then the file is gone. While `judge.hold` is there, the
/// first call that finds it moves it to `judge.held` and waits (at most 120 s) until the test
/// takes that away; the other calls do not wait ([`two_first_stops`]). The
/// other stand-in gives one answer whatever it is shown, so it cannot tell a stop that judged
/// the right commit from one that judged another.
fn reading_judge(scratch: &Scratch) -> (PathBuf, PathBuf) {
    const READ: &str = r#"printf 'PATCH-NAMES %s\n' "$(grep '^diff --git' "$patch" | tr '\n' ' ')" >> '@LOG@'
if [ -f '@SLEEP@' ]; then sleep "$(cat '@SLEEP@')"; fi
if mv '@HOLD@' '@HELD@' 2>/dev/null; then n=0; while [ -e '@HELD@' ] && [ "$n" -lt 1200 ]; do sleep 0.1; n=$((n+1)); done; fi
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
        )
        .replace("@HOLD@", &scratch.join("judge.hold").display().to_string())
        .replace("@HELD@", &scratch.join("judge.held").display().to_string());
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
        // (F13.a: the other tree whose HEAD reflog has it is named)
        blocked(&decision).contains(&format!("Commit {there} (the HEAD reflog of ")),
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
        reason.contains(&format!("Commit {gone} (the HEAD reflog of ")),
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
        // F12.d with the ledger: the first such stop judges the session's commit and blocks
        let decision = stop_from(&root, "s", &script);
        let said = if what == "longer" {
            blocked(&decision)
        } else {
            allowed(&decision)
        };
        assert!(
            said.starts_with(&stateless("state.lists_damaged"))
                && said.contains("list of the commits that were there when it began")
                && said.contains("start a new session"),
            "{what}: {said}"
        );
    }
    assert_eq!(runs(&log), 1);
    // as it was recorded it is believed, and the commit is judged: once more, since the
    // session's own list of judged commits does not have the stateless stop's verdict
    std::fs::write(&tips, recorded).unwrap();
    blocked(&stop_from(&root, "s", &script));
    assert_eq!(runs(&log), 2);
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

/// What follows the id of a commit a stop judges by the project's frontier.
const NOT_OURS: &str = " (not this session's: left or merged since the project was first recorded)";

/// The hook input of `event` with the `source` a `SessionStart` gives (`resume`, `compact`, …).
fn input_from(event: &str, cwd: &Path, session: &str, source: &str) -> Value {
    let mut value = input(event, cwd, session, false);
    value["source"] = json!(source);
    value
}

/// The last policy the stand-in judge was given, as its log names it.
fn last_policy(log: &Path) -> String {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .rev()
        .find_map(|line| line.strip_prefix("POLICY "))
        .unwrap_or_default()
        .to_owned()
}

/// F13.b (A11, probe p4): a commit made by a session that ended before its stop (a crash, a
/// closed terminal, a conversation that started a new record) was "already there" for the next
/// session and never judged. The frontier of what has been judged belongs to the project: the
/// next session's stop judges it, once, and then it stands. A session that goes on with the
/// conversation (`source` resume, compact or clear) and has no record of its own begins from
/// the commit the last session of its working tree began from, not from `HEAD`: a policy the
/// ended session loosened in a commit does not judge that commit.
#[test]
fn a_commit_made_before_a_crash_is_judged_by_the_next_session() {
    let scratch = Scratch::new("hook-frontier-crash");
    let root = repository(&scratch);
    let (script, log) = reading_judge(&scratch);
    start(&root, "a");
    std::fs::write(root.join("k.rs"), "const K: &str = \"SECRET\";\n").unwrap();
    let work = commit_all(&root, "the ended session's work");
    // session a ends here, before any stop; a new conversation begins
    start(&root, "b");
    let reason = blocked(&stop_from(&root, "b", &script)).to_owned();
    // it is said not to be this session's own, and not to be rewritten
    assert!(
        reason.contains(&format!("Commit {}{NOT_OURS}: FAIL\n", &work[..12]))
            && reason.contains(
                "Do not rewrite it or its history: tell a person, and make any repair a new commit."
            ),
        "{reason}"
    );
    // it held once: the next stop lets the session stop
    assert!(stop_from(&root, "b", &script).get("decision").is_none());
    // a third session finds it judged in the project's ledger: not judged again
    let runs_before = runs(&log);
    start(&root, "c");
    assert_eq!(stop_from(&root, "c", &script), Value::Null);
    assert_eq!(runs(&log), runs_before);

    // a conversation that goes on under a new record: its base is the one the ended session
    // of this tree began from, so the policy it loosened in a commit does not judge it
    let begun = short(&root, "HEAD");
    start(&root, "d");
    std::fs::write(root.join("warden.toml"), "# loosened by the session\n").unwrap();
    std::fs::write(root.join("m.rs"), "const M: &str = \"SECRET\";\n").unwrap();
    let loosened = commit_all(&root, "a looser policy, and work");
    let (code, said, _) = hook(
        "session-start",
        &[],
        &input_from("SessionStart", &root, "e", "resume"),
    );
    assert_eq!(code, 0);
    assert!(said.contains(&format!("(commit {}", &begun[..7])), "{said}");
    let reason = blocked(&stop_from(&root, "e", &script)).to_owned();
    assert!(
        reason.contains(&format!("Commit {}{NOT_OURS}: FAIL\n", &loosened[..12])),
        "{reason}"
    );
    assert_eq!(
        last_policy(&log),
        "# the policy at the base commit",
        "the loosened policy judged"
    );
}

/// F13.b, ruling 2 (N93): the ledger is the agent's to write. A session that removes it is
/// judged at least as a session was before the frontier: its own commits are judged, the stop
/// says that no record of the project was found, and the commits that were there when the
/// session began become the project's baseline again.
#[test]
fn a_session_that_removes_the_ledger_is_judged_as_before() {
    let scratch = Scratch::new("hook-frontier-no-ledger");
    let root = repository(&scratch);
    let (script, _) = reading_judge(&scratch);
    start(&root, "s");
    std::fs::write(root.join("k.rs"), "const K: &str = \"SECRET\";\n").unwrap();
    let work = commit_all(&root, "work");
    let ledger = root.join(".git/deep-diff-forge/ledger.jsonl");
    assert!(
        std::fs::read_to_string(&ledger)
            .unwrap()
            .contains("\"kind\":\"baseline\"")
    );
    std::fs::remove_file(&ledger).unwrap();
    let reason = blocked(&stop_from(&root, "s", &script)).to_owned();
    assert!(
        reason.contains(&format!("Commit {}: FAIL\n", &work[..12])),
        "{reason}"
    );
    assert!(
        reason.contains("No record of this project was found in its ledger"),
        "{reason}"
    );
    assert!(
        std::fs::read_to_string(&ledger)
            .unwrap()
            .contains("\"kind\":\"baseline\"")
    );
}

/// F13.b, ruling 3 (N92) and the closing round's ruling 1: an ended session can leave more
/// commits than one stop has time for. The session's own work is judged first: its own commit
/// has its verdict at its first stop; the frontier's commits get the time that is left, the
/// oldest first, and what does not fit is counted as left or merged since the project was first
/// recorded and judged at later stops. No stop runs past its bound.
///
/// This test takes about forty seconds: one stop runs up to its deadline with a judge that
/// takes thirteen seconds for each patch.
#[test]
fn commits_an_ended_session_left_are_judged_over_later_stops_within_their_bound() {
    let scratch = Scratch::new("hook-frontier-many");
    let root = repository(&scratch);
    let (script, log) = reading_judge(&scratch);
    start(&root, "a");
    let names: Vec<String> = (1..=5).map(|n| format!("f{n}.rs")).collect();
    for name in &names {
        std::fs::write(root.join(name), "fn f() {}\n").unwrap();
        commit_all(&root, name);
    }
    start(&root, "b");
    secret(&root, "own.rs");
    let own = commit_all(&root, "the session's own work");
    std::fs::write(scratch.join("judge.sleep"), "13").unwrap();
    let began = std::time::Instant::now();
    let decision = stop_from(&root, "b", &script);
    assert!(
        began.elapsed() < std::time::Duration::from_secs(50),
        "{:?}",
        began.elapsed()
    );
    let reason = blocked(&decision).to_owned();
    assert!(
        reason.contains(&format!("Commit {own}: FAIL\n")),
        "{reason}"
    );
    let first = shown(&log);
    assert_eq!(
        first.first().map(String::as_str),
        Some("own.rs"),
        "not the session's own first"
    );
    assert!(first.len() < names.len() + 1, "{first:?}");
    assert_eq!(
        first[1..],
        names[..first.len() - 1],
        "the frontier not the oldest first"
    );
    assert!(
        reason.contains("commits left or merged since the project was first recorded: UNJUDGED"),
        "{reason}"
    );
    std::fs::remove_file(scratch.join("judge.sleep")).unwrap();
    // the backlog drains over the next stops; the own commit's failure stands
    let said = allowed(&stop_from(&root, "b", &script)).to_owned();
    assert!(
        said.contains(&format!("Commit {own}: FAIL, which stands")),
        "{said}"
    );
    let all = shown(&log);
    for name in &names {
        assert!(all.contains(name), "{name} was never judged: {all:?}");
    }
    let before = runs(&log);
    let _ = stop_from(&root, "b", &script);
    assert_eq!(runs(&log), before);
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
        // (F13.a: the other tree whose HEAD reflog has it is named)
        blocked(&decision).contains(&format!("Commit {made} (the HEAD reflog of ")),
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
    // F12.d: the tree is judged without the state, and its failure blocks
    let said = blocked(&decision);
    assert!(
        said.starts_with(&stateless("state.no_directory")) && said.contains(NOT_ITS_OWN),
        "{said}"
    );
    assert_eq!(runs(&log), 1);
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
            // F13.b: n1's three commits still have no verdict in the project's ledger, so
            // n2's stop asks about them too, and counts them apart
            && reason.contains("3 commits of this session have no verdict: ")
            && reason.contains(
                "3 commits left or merged since the project was first recorded have no verdict: "
            ),
        "{reason}"
    );

    // The chain gives no verdict (here: a judge that cannot read its environment): after
    // three asks and the probe nothing more is asked at that stop, the rest are counted,
    // nothing is final, and the next stop, with a judge that works, has them all judged.
    // (In a repository of its own: the commits the sessions above left with no verdict would
    // be asked about here too, F13.b.)
    let chain_scratch = Scratch::new("hook-no-verdict-chain");
    let root = repository(&chain_scratch);
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
            "#!/bin/sh\n[ \"$2\" = --help ] && exit 0\necho x >> '{}'\necho '{{\"exit\":3,\"name\":\"io\",\"message\":\"no Git allowlist enrolled\",\"next\":\"check_environment\"}}' >&2\nexit 3\n",
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
    const ALREADY: &str = "a commit that was already in the repository when the project's ledger first recorded the project (else when the session began), or that was made before the session began and has a stop's verdict in that ledger, is not judged at a stop, also when it is pulled or merged in";
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
        said.contains("Not judged: a commit that was already in this repository when the project's ledger first recorded the project (else when the session began), or made before the session began with a stop's verdict in that ledger, also when you pull, merge, rebase onto or reset to it"),
        "{said}"
    );
    git(&root, &["merge", "--quiet", "--ff-only", "origin/main"]);
    let (_, advice, _) = hook("post-tool", &[], &input("PostToolUse", &root, "a1", false));
    assert!(
        advice.contains("\\\"k.rs\\\":1: secret_like@line") && advice.contains(ALREADY),
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
    // F13.b: the branch's commit was made after the project's ledger first recorded the
    // project (at a1's start) and no stop gave it a verdict: a4's stop judges it, the oldest
    // first; the merge brings the same patch, whose answer is the one already given
    // (the closing round: the session's own commit first, then the frontier's)
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
            said.starts_with(&stateless("state.lists_damaged"))
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

/// The second half of [`a_stops_message_is_bounded_and_what_blocks_keeps_its_rule`]:
/// forty-five commits that fail at one stop, the session's own the newest (as few as show all three
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
    // forty until the remedy under a rule lost its substitute (S3.17a.f): all forty then fit
    // with their rules or their names, and none was counted
    for n in 0..44 {
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
    // a count: every one of the forty-five is in one of the three
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
    assert_eq!(with_rules + 20 + counted, 45, "{reason}");
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
        later.contains("45 earlier failures stand: ")
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
    // F12.d: the second stop judges the tree without the state (it is clean: nothing to run)
    let waited = allowed(&second);
    assert!(
        waited.starts_with(&stateless("state.lock_busy"))
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
/// of 45 seconds less the 4 it keeps for remembering and saying
/// (`docs/reference/agent-hooks.md`, "The order of a stop, and its time").
const JUDGING: f64 = 41.0;
/// With less than this of its judging left a stop asks about no commit
/// (`docs/reference/agent-hooks.md`, the same table).
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
    // Its start lists the commits of 256 trees: one past its time limit (8 s) records nothing,
    // and the next hook starts again as the first, by then among 257. That is the probable
    // cause of this test's failure on the 0.11.0 macOS runner: here it is named when it recurs.
    let before = start(&root, "before");
    assert!(
        before.contains("deep-diff-forge is watching the changes of this session")
            && !before.contains(WHY),
        "the \"before\" start did not record its list: {before}"
    );
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

/// A linked tree of `root` at `tree` on the branch `branch`.
fn linked_tree(root: &Path, tree: &Path, branch: &str) {
    git(
        root,
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

/// What a stop of `session` from the relative directory `cwd` decides, the hook started in
/// `here`.
fn stop_from_relative(cwd: &str, here: &Path, session: &str, judge: &Path) -> Value {
    let (code, stdout, stderr) = hook_in(
        "stop",
        &["--judge", judge.to_str().unwrap()],
        &input("Stop", Path::new(cwd), session, false),
        |command| {
            command.current_dir(here);
        },
    );
    assert_eq!((code, stderr.as_str()), (0, ""), "{stdout}");
    if stdout.is_empty() {
        Value::Null
    } else {
        serde_json::from_str(&stdout).unwrap()
    }
}

/// The state file of the one session of this test, read.
fn the_state() -> Value {
    let state = states_in(&sessions());
    assert_eq!(state.len(), 1, "{state:?}");
    serde_json::from_str(&std::fs::read_to_string(&state[0]).unwrap()).unwrap()
}

/// Review V4 round 22, VD4R22-M-1, as reproduced: the session's directory is given as a
/// relative path, `../linked/build/gone`, of a directory that was removed. A relative path was
/// not walked up: "nothing of it is there" was taken as "adds nothing" before it was anchored.
/// With the linked tree whole, the tree was not judged and was named as one "the session's
/// directory is not in"; with its two records removed as well, the stop printed nothing and
/// forgot what had blocked there. A relative path is anchored at the hook's own directory and
/// walked up as an absolute one is; a path that names a file is walked past too (fourteenth
/// independent review of the hooks, suspected).
#[test]
fn a_relative_directory_that_was_removed_is_anchored_and_walked_up() {
    let scratch = Scratch::new("hook-relative-gone");
    let root = repository(&scratch);
    let (script, log) = reading_judge(&scratch);
    let linked = scratch.join("linked");
    linked_tree(&root, &linked, "side");
    start(&root, "s");
    secret(&linked, "linked.rs");
    blocked(&stop_from(&linked, "s", &script));
    std::fs::create_dir_all(linked.join("build/gone")).unwrap();
    std::fs::remove_dir_all(linked.join("build")).unwrap();
    for said in [
        stop_from_relative("../linked/build/gone", &root, "s", &script),
        stop_from(&linked.join("linked.rs"), "s", &script),
        stop_from(&linked.join("linked.rs/deeper"), "s", &script),
    ] {
        let said = allowed(&said);
        assert!(
            said.starts_with("deep-diff-forge: FAIL STANDS. ")
                && !said.contains("the session's directory is not in it"),
            "{said}"
        );
    }
    // the linked tree was judged from each of them
    assert_eq!(shown(&log), ["linked.rs"; 4]);

    // its two records removed: UNJUDGED, nothing forgotten, the new key put to no judge
    let before = remembered_lines();
    remove_both_records(&linked);
    secret(&linked, "second.rs");
    let said = allowed(&stop_from_relative(
        "../linked/build/gone",
        &root,
        "s",
        &script,
    ))
    .to_owned();
    assert!(
        said.starts_with("deep-diff-forge: UNJUDGED. ")
            && said.contains("is no longer a working tree of this repository"),
        "{said}"
    );
    assert_eq!(remembered_lines(), before);
    assert!(
        !shown(&log).iter().any(|names| names.contains("second.rs")),
        "{:?}",
        shown(&log)
    );
}

/// Fourteenth independent review of the hooks, HR14-L-4, as reproduced: `core.worktree` in a
/// linked tree's own configuration points Git at the project's top. From the tree, Git said
/// it was the project: the stop printed nothing with a second key in the tree, and what had
/// blocked there was forgotten, here and at a stop from the project (the judge's reader read
/// the project through the redirect). What Git answers must hold the session's directory,
/// and what is read of another tree is that tree's only when Git names it there.
#[test]
fn a_tree_git_is_pointed_away_from_is_unjudged_and_keeps_what_blocked_there() {
    let scratch = Scratch::new("hook-core-worktree");
    let root = repository(&scratch);
    let (script, log) = reading_judge(&scratch);
    let linked = scratch.join("linked");
    linked_tree(&root, &linked, "side");
    start(&root, "s");
    secret(&linked, "first.rs");
    blocked(&stop_from(&linked, "s", &script));
    let before = remembered_lines();
    git(&root, &["config", "extensions.worktreeConfig", "true"]);
    git(
        &linked,
        &[
            "config",
            "--worktree",
            "core.worktree",
            root.to_str().unwrap(),
        ],
    );
    secret(&linked, "second.rs");
    let said = allowed(&stop_from(&linked, "s", &script)).to_owned();
    // (round 24: the pair Git names is no tree of the records, which comes before whether it
    // holds the session's directory)
    assert!(
        said.starts_with("deep-diff-forge: UNJUDGED. ")
            && said.contains("as the working tree of a Git directory whose records name ")
            && said.contains("/linked\" as that tree"),
        "{said}"
    );
    assert_eq!(remembered_lines(), before);
    let from_project = allowed(&stop_from(&root, "s", &script)).to_owned();
    assert!(
        from_project.contains(
            "/linked\" of this project has changes that are not committed could not be found out (Git names "
        ) && from_project.contains("/linked\" as that tree; the Git directory "),
        "{from_project}"
    );
    assert_eq!(remembered_lines(), before);
    assert!(
        !shown(&log).iter().any(|names| names.contains("second.rs")),
        "{:?}",
        shown(&log)
    );
}

/// Fourteenth independent review of the hooks, HR14-L-3, as reproduced with a btrfs
/// subvolume; here with a file system mounted in a linked tree in a mount namespace of the
/// test's own. Git does not cross the boundary while `GIT_DISCOVERY_ACROSS_FILESYSTEM` is not
/// set and says "not a git repository": the tree was not judged from there and was named as
/// one "the session's directory is not in". The stop is UNJUDGED with Git's words, and the
/// tree "may be in it". Where no user and mount namespace can be made the case is passed.
#[test]
fn a_boundary_inside_a_linked_tree_is_not_taken_for_no_repository() {
    let scratch = Scratch::new("hook-boundary");
    let root = repository(&scratch);
    let (script, log) = reading_judge(&scratch);
    let linked = scratch.join("linked");
    linked_tree(&root, &linked, "side");
    let vol = linked.join("vol");
    std::fs::create_dir(&vol).unwrap();
    start(&root, "s");
    secret(&linked, "first.rs");
    blocked(&stop_from(&linked, "s", &script));
    let before = remembered_lines();
    let probe = Command::new("unshare")
        .args(["-r", "-m", "true"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    if !probe.is_ok_and(|status| status.success()) {
        eprintln!("no user and mount namespace here: the case is passed");
        return;
    }
    let mut command = Command::new("unshare");
    command
        .args(["-r", "-m", "sh", "-c"])
        .arg(r#"mount -t tmpfs none "$1" && mkdir "$1/sub" && shift && exec "$@""#)
        .arg("sh")
        .arg(&vol)
        .arg(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .args(["agent", "hook", "stop", "--client", "claude", "--judge"])
        .arg(&script)
        .envs(ISOLATE)
        .env("HOME", home())
        .env_remove("XDG_STATE_HOME")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("CLAUDE_PROJECT_DIR")
        .env_remove("GIT_DISCOVERY_ACROSS_FILESYSTEM")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(
            input("Stop", &vol.join("sub"), "s", false)
                .to_string()
                .as_bytes(),
        )
        .unwrap();
    let out = child.wait_with_output().unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert_eq!(
        (
            out.status.code(),
            String::from_utf8(out.stderr).unwrap().as_str()
        ),
        (Some(0), ""),
        "{stdout}"
    );
    let said: Value = serde_json::from_str(&stdout).unwrap();
    let said = allowed(&said);
    assert!(
        said.starts_with("deep-diff-forge: UNJUDGED. ")
            && said.contains("Git stopped at a file-system boundary")
            && said.contains(
                "/linked\" of this project, and whether it is in that tree could not be found out at this stop"
            )
            && !said.contains("the session's directory is not in it"),
        "{said}"
    );
    assert_eq!(remembered_lines(), before);
    assert_eq!(shown(&log), ["first.rs"]);
}

/// Fourteenth independent review of the hooks, HR14-L-1, as reproduced: round 22 emptied
/// `GIT_CEILING_DIRECTORIES` for every Git call of the hooks, and a repository the user hides
/// with a ceiling (a home directory's) was found. A session below it, in no repository to
/// the user's Git, took it for its project; a stop of a session with a real project, made
/// from below it, named it as "another repository". The project is found with the caller's
/// ceiling; once it is known only the entries that hide a part of one of its trees are left
/// out (the ceiling test above still holds).
#[test]
fn a_repository_the_user_hides_with_a_ceiling_stays_hidden() {
    let scratch = Scratch::new("hook-ceiling-hidden");
    let root = repository(&scratch);
    let (script, log) = reading_judge(&scratch);
    let hidden = scratch.join("hidden");
    std::fs::create_dir_all(hidden.join("notes/today")).unwrap();
    git(&hidden, &["init", "--quiet"]);
    std::fs::write(hidden.join(".bashrc"), "export A=1\n").unwrap();
    git(&hidden, &["add", "--all"]);
    git(&hidden, &["commit", "--quiet", "-m", "dotfiles"]);
    std::fs::write(hidden.join(".bashrc"), "export A=2\nSECRET\n").unwrap();
    let below = hidden.join("notes/today");
    let with_ceiling = |event: &str, extra: &[&str], session: &str| {
        let name = match event {
            "session-start" => "SessionStart",
            "post-tool" => "PostToolUse",
            _ => "Stop",
        };
        let (code, stdout, stderr) = hook_in(
            event,
            extra,
            &input(name, &below, session, false),
            |command| {
                command.env("GIT_CEILING_DIRECTORIES", &hidden);
            },
        );
        assert_eq!((code, stderr.as_str()), (0, ""), "{stdout}");
        stdout
    };
    let judge = ["--judge", script.to_str().unwrap()];
    // a session that begins below the hidden repository has no project
    for (event, extra) in [
        ("session-start", &judge[..]),
        ("post-tool", &[][..]),
        ("stop", &judge[..]),
    ] {
        assert_eq!(with_ceiling(event, extra, "below"), "", "{event}");
    }
    // a session with a project says nothing of the hidden repository
    start(&root, "s");
    assert_eq!(with_ceiling("stop", &judge, "s"), "");
    assert_eq!(shown(&log), Vec::<String>::new());
}

/// Fourteenth independent review of the hooks, HR14-L-2, as reproduced: a linked tree beside
/// the project whose `.git` file and administrative directory were removed (or its `.git`
/// file, and then `git worktree prune`, which a stop advises). Git places its directory in no
/// repository, so the stop from inside it printed nothing with a second key there, and what
/// had blocked there was forgotten. The state records the top level of a tree it remembers
/// something of: while that directory stands the tree is named at every stop and a stop from
/// inside it is UNJUDGED; it is forgotten when the directory is gone, or when it is a tree
/// again and the judge's own reading finds it clean.
#[test]
fn a_tree_no_longer_listed_stands_while_its_directory_does() {
    let scratch = Scratch::new("hook-unlisted-stands");
    let root = repository(&scratch);
    let (script, log) = reading_judge(&scratch);
    let (both, pruned) = (scratch.join("both"), scratch.join("pruned"));
    linked_tree(&root, &both, "both");
    linked_tree(&root, &pruned, "pruned");
    start(&root, "s");
    secret(&both, "first.rs");
    blocked(&stop_from(&both, "s", &script));
    secret(&pruned, "first.rs");
    blocked(&stop_from(&pruned, "s", &script));
    let before = remembered_lines();
    let recorded = the_state()["trees"].as_object().unwrap().len();
    assert_eq!(recorded, 2, "{}", the_state());

    remove_both_records(&both);
    std::fs::remove_file(pruned.join(".git")).unwrap();
    git(&root, &["worktree", "prune"]);
    for tree in [&both, &pruned] {
        secret(tree, "second.rs");
        let said = allowed(&stop_from(tree, "s", &script)).to_owned();
        assert!(
            said.starts_with("deep-diff-forge: UNJUDGED. ")
                && said.contains("is no longer a working tree of this repository"),
            "{said}"
        );
        assert_eq!(remembered_lines(), before);
    }
    let from_project = allowed(&stop_from(&root, "s", &script)).to_owned();
    assert!(
        from_project.contains("no longer a working tree"),
        "{from_project}"
    );
    assert_eq!(remembered_lines(), before);
    assert!(
        !shown(&log).iter().any(|names| names.contains("second.rs")),
        "{:?}",
        shown(&log)
    );

    // `both` is gone: what blocked there is forgotten
    std::fs::remove_dir_all(&both).unwrap();
    let _ = stop_from(&root, "s", &script);
    let left = remembered_lines();
    assert!(left.len() < before.len() && !left.is_empty(), "{left:?}");
    // `pruned` is a tree again, made in its emptied directory: forgotten when the judge's
    // reading finds it clean
    for entry in std::fs::read_dir(&pruned).unwrap() {
        std::fs::remove_file(entry.unwrap().path()).unwrap();
    }
    let _ = stop_from(&root, "s", &script);
    assert_eq!(remembered_lines(), left, "an emptied directory stands");
    linked_tree(&root, &pruned, "again");
    let _ = stop_from(&root, "s", &script);
    assert_eq!(remembered_lines(), Vec::<String>::new());
    assert_eq!(the_state()["trees"], json!({}));
}

// --- The hook-state version policy (F7.e; rules V1 to V10 of the model design) ------------

/// The session id the fixtures of `tests/fixtures/hook-state/` were captured under.
const FIXTURE_SESSION: &str = "fixture";
/// What removing a state file that is not read costs, as every refusal of one says it.
const REMOVING_COSTS: &str = "removing it makes the next hook take the commit HEAD names as this session's base: what this session committed before is judged only where the project's ledger finds it (made after the project was first recorded, with no stop's verdict), and not otherwise; left as it is, the working tree is judged at every stop without the session's memory";

/// Git in `root` as `scripts/capture_hook_state.sh` runs it: the same names and dates, so the
/// same files give the same commit ids.
fn git_as_captured(root: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(root)
        .envs(ISOLATE)
        .envs([
            ("GIT_AUTHOR_NAME", "ddf"),
            ("GIT_AUTHOR_EMAIL", "ddf@example.invalid"),
            ("GIT_COMMITTER_NAME", "ddf"),
            ("GIT_COMMITTER_EMAIL", "ddf@example.invalid"),
            ("GIT_AUTHOR_DATE", "2026-11-20T00:00:00+0000"),
            ("GIT_COMMITTER_DATE", "2026-11-20T00:00:00+0000"),
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

/// The repository the fixtures were captured in, made again: its base, a commit of failing
/// work (`c.rs`) and a failing edit of `a.rs`. The scratch directory becomes the home of this
/// test's hooks.
fn captured_repository(scratch: &Scratch) -> PathBuf {
    home_in(scratch);
    let root = scratch.join("repo");
    std::fs::create_dir(&root).unwrap();
    git_as_captured(&root, &["init", "--quiet"]);
    std::fs::write(root.join("a.rs"), "fn a() {}\nfn b() {}\n").unwrap();
    std::fs::write(root.join("warden.toml"), "# policy\n").unwrap();
    git_as_captured(&root, &["add", "--all"]);
    git_as_captured(&root, &["commit", "--quiet", "-m", "base"]);
    std::fs::write(root.join("c.rs"), "fn c() {}\n// BADLINE committed\n").unwrap();
    git_as_captured(&root, &["add", "c.rs"]);
    git_as_captured(&root, &["commit", "--quiet", "-m", "failing work"]);
    std::fs::write(
        root.join("a.rs"),
        "fn a() {}\n#[allow(dead_code)] // BADLINE\nfn b() {}\n",
    )
    .unwrap();
    root
}

/// Every fixture directory: its name and its three files (state, `.tips`, `.judged`), as
/// bytes, by extension.
fn fixtures() -> Vec<(String, std::collections::BTreeMap<String, Vec<u8>>)> {
    let top = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hook-state");
    let mut all = Vec::new();
    for entry in std::fs::read_dir(&top).unwrap() {
        let dir = entry.unwrap().path();
        let mut files = std::collections::BTreeMap::new();
        for file in std::fs::read_dir(&dir).unwrap() {
            let file = file.unwrap().path();
            let kind = file.extension().unwrap_or_default().to_string_lossy();
            if ["json", "tips", "judged"].contains(&kind.as_ref()) {
                files.insert(kind.into_owned(), std::fs::read(&file).unwrap());
            }
            // (the key the state's digests were made with, kept from v12 on)
            if file.file_name().is_some_and(|name| name == "key") {
                files.insert("key".to_owned(), std::fs::read(&file).unwrap());
            }
        }
        assert!(
            files.len() == 3 || (files.len() == 4 && files.contains_key("key")),
            "{}: {files:?}",
            dir.display()
        );
        all.push((
            dir.file_name().unwrap().to_string_lossy().into_owned(),
            files,
        ));
    }
    all.sort();
    assert!(all.len() >= 2, "{all:?}");
    all
}

/// Put a fixture's files in place as the session [`FIXTURE_SESSION`]'s, for the repository at
/// `root`: byte for byte, but the state's `top` and `id`, which name the directory and the
/// inode of the repository it was captured in, name `root`'s. Returns the state file's path.
fn place_fixture(files: &std::collections::BTreeMap<String, Vec<u8>>, root: &Path) -> PathBuf {
    use std::os::unix::fs::MetadataExt as _;
    let text = String::from_utf8(files["json"].clone()).unwrap();
    let state: Value = serde_json::from_str(&text).unwrap();
    let (top, id) = (
        state["project"]["top"].as_str().unwrap(),
        state["project"]["id"].as_str().unwrap(),
    );
    let here = git(root, &["rev-parse", "--show-toplevel"]);
    let ino = std::fs::metadata(root.join(".git"))
        .unwrap()
        .ino()
        .to_string();
    let text = text
        .replacen(
            &format!("\"top\":{}", json!(top)),
            &format!("\"top\":{}", json!(here.trim())),
            1,
        )
        .replacen(&format!("\"id\":\"{id}\""), &format!("\"id\":\"{ino}\""), 1);
    let dir = sessions();
    std::fs::create_dir_all(&dir).unwrap();
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let name = format!(
        "session-{}",
        &deep_diff_forge_core::sha256_hex(FIXTURE_SESSION.as_bytes())[..32]
    );
    std::fs::write(dir.join(format!("{name}.json")), text).unwrap();
    std::fs::write(dir.join(format!("{name}.tips")), &files["tips"]).unwrap();
    std::fs::write(dir.join(format!("{name}.judged")), &files["judged"]).unwrap();
    if let Some(key) = files.get("key") {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::write(dir.join("key"), key).unwrap();
        std::fs::set_permissions(dir.join("key"), std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    dir.join(format!("{name}.json"))
}

/// The three files of the session whose state is `state`, as bytes.
fn three_files(state: &Path) -> [Vec<u8>; 3] {
    ["json", "tips", "judged"].map(|kind| std::fs::read(state.with_extension(kind)).unwrap())
}

/// Rule V3: a state of every schema since the last release is read, here the state, the list of
/// commits and the list of judged commits that the published 0.9.0 and 0.10.0 binaries wrote
/// in the middle of a session (v10: a failure that blocked, a commit judged FAIL), one a build
/// of this tree wrote in v11, and one each in v12 and v13 with the key its digests were made
/// with (`tests/fixtures/hook-state/`, each directory's `ORIGIN` says which binary, by its
/// SHA-256). Each is written as v13 at the first stop with its base, its list of commits and
/// its judged commits kept: the commit is not judged again (one run of the judge at each
/// stop, for the working tree). A state before v12 kept unkeyed digests, which v12 empties
/// (lane H3): the failure that blocked under it blocks once more, and then stands; a v12 or
/// v13 state, read under its own key, says FAIL STANDS at once. v13 adds the set of guard
/// changes, empty from an older schema (F6.b).
#[test]
fn every_schema_since_the_last_release_is_read() {
    for (name, files) in fixtures() {
        let scratch = Scratch::new("hook-state-fixture");
        let root = captured_repository(&scratch);
        let captured: Value = serde_json::from_slice(&files["json"]).unwrap();
        assert_eq!(
            git(&root, &["rev-parse", "HEAD~1"]).trim(),
            captured["project"]["base"],
            "{name}: the repository was not made again as it was captured"
        );
        let state = place_fixture(&files, &root);
        let (script, log, _) = failing_judge(&scratch);
        let decision = stop_from(&root, FIXTURE_SESSION, &script);
        let keyed = ["v12", "v13"]
            .map(|n| format!("deep-diff-forge.hook-state.{n}"))
            .contains(&captured["schema"].as_str().unwrap().to_owned());
        if keyed {
            let said = allowed(&decision);
            assert!(
                said.starts_with("deep-diff-forge: FAIL STANDS. "),
                "{name}: a state of this schema was not read: {said}"
            );
        } else {
            let reason = blocked(&decision);
            assert!(
                reason.starts_with("deep-diff-forge: the judge fails this change ("),
                "{name}: {reason}"
            );
        }
        assert_eq!(
            runs(&log),
            1,
            "{name}: the commit judged before was asked again"
        );
        let now: Value = serde_json::from_str(&std::fs::read_to_string(&state).unwrap()).unwrap();
        assert_eq!(now["schema"], "deep-diff-forge.hook-state.v13", "{name}");
        // the set of guard changes: the fixture's, or empty from a schema before v13 (F6.b)
        let guards = captured.get("guards").cloned().unwrap_or_else(|| json!([]));
        assert_eq!(now["guards"], guards, "{name}");
        assert_eq!(
            now["project"]["base"], captured["project"]["base"],
            "{name}"
        );
        assert_eq!(
            now["project"]["tips_sha256"],
            captured["project"]["tips_sha256"]
        );
        assert_eq!(
            std::fs::read(state.with_extension("tips")).unwrap(),
            files["tips"]
        );
        assert_eq!(
            std::fs::read(state.with_extension("judged")).unwrap(),
            files["judged"],
            "{name}: the judged commit's line is in v12's words already"
        );
        if keyed {
            assert_eq!(now["blocked"], captured["blocked"], "{name}");
            assert_eq!(now["key_id"], captured["key_id"], "{name}");
        }
        // the next turn: the failure stands, from the migrated state
        let said = allowed(&stop_from(&root, FIXTURE_SESSION, &script)).to_owned();
        assert!(
            said.starts_with("deep-diff-forge: FAIL STANDS. "),
            "{name}: {said}"
        );
        assert_eq!(
            runs(&log),
            2,
            "{name}: the commit judged before was asked again"
        );
    }
}

/// Rule V4: a state of a schema older than the last release's (v9 and before; no release
/// wrote one) is refused by its name, with the range this build reads and what removing it
/// costs; nothing is written over it, by a stop, an advice hook or a session's start. (Round
/// 23 read v9 and wrote it back as v10; v9 is retired since 0.11.0.)
#[test]
fn a_state_older_than_the_last_release_is_refused_with_its_name() {
    let (_, files) = fixtures().remove(0);
    let scratch = Scratch::new("hook-state-older");
    let root = captured_repository(&scratch);
    let state = place_fixture(&files, &root);
    let mut v9: Value = serde_json::from_str(&std::fs::read_to_string(&state).unwrap()).unwrap();
    v9["schema"] = json!("deep-diff-forge.hook-state.v9");
    v9.as_object_mut().unwrap().remove("trees");
    std::fs::write(&state, v9.to_string()).unwrap();
    let before = three_files(&state);
    let (script, log, _) = failing_judge(&scratch);
    // F12.d: the tree is judged without the state (the captured change fails)
    let said = blocked(&stop_from(&root, FIXTURE_SESSION, &script)).to_owned();
    assert!(
        said.starts_with(&stateless("state.older")) && said.contains(

            "is of schema v9, written by a deep-diff-forge older than the last release; this one reads v10 to v13, and nothing is recorded over it ("
        ) && said.contains(REMOVING_COSTS),
        "{said}"
    );
    let advice = hook(
        "post-tool",
        &[],
        &input("PostToolUse", &root, FIXTURE_SESSION, false),
    )
    .1;
    assert!(
        system_message(&advice).contains("is of schema v9"),
        "{advice}"
    );
    let started = start(&root, FIXTURE_SESSION);
    assert!(started.contains("is of schema v9"), "{started}");
    assert_eq!(three_files(&state), before);
    assert!(runs(&log) >= 1, "the tree was not judged");
}

/// Rules V5 and V1: a state of a newer schema (here v14 with a member this build does not
/// know, and v999999) is refused by its name with the range this build reads; no hook writes
/// the state, its list of commits or its list of judged commits (a stop, a continuing stop, an
/// advice hook, a session's start), none leaves a partial file, and none advises removing
/// it or says that anything else judges. What the stop then does is what it does for a
/// state it cannot use: UNJUDGED, the judge not run.
#[test]
fn a_newer_state_is_refused_and_kept_byte_for_byte() {
    let (_, files) = fixtures().remove(0);
    let scratch = Scratch::new("hook-state-newer");
    let root = captured_repository(&scratch);
    let state = place_fixture(&files, &root);
    let (script, log, _) = failing_judge(&scratch);
    for number in [14_u32, 999_999] {
        let mut newer: Value =
            serde_json::from_str(&std::fs::read_to_string(&state).unwrap()).unwrap();
        newer["schema"] = json!(format!("deep-diff-forge.hook-state.v{number}"));
        newer["a_member_of_a_newer_build"] = json!(["x"]);
        std::fs::write(&state, newer.to_string()).unwrap();
        let before = three_files(&state);
        let refused = format!(
            "is of schema v{number}, written by a newer deep-diff-forge; this one reads v10 to v13, and nothing was written over it"
        );
        for active in [false, true] {
            let (code, stdout, _) = hook(
                "stop",
                &["--judge", script.to_str().unwrap()],
                &input("Stop", &root, FIXTURE_SESSION, active),
            );
            assert_eq!(code, 0);
            // F12.d: the tree is judged without the state; at a stop that follows a block
            // its failure stands
            let decision: Value = serde_json::from_str(&stdout).unwrap();
            let said = stop_words(&decision);
            assert_eq!(decision.get("decision").is_some(), !active, "{said}");
            assert!(
                said.starts_with(&stateless("state.newer")) && said.contains(&refused),
                "{said}"
            );
            assert!(!said.contains("remov"), "{said}");
            assert_eq!(
                three_files(&state),
                before,
                "a stop wrote over a newer state"
            );
        }
        let advice = hook(
            "post-tool",
            &[],
            &input("PostToolUse", &root, FIXTURE_SESSION, false),
        )
        .1;
        let advice = system_message(&advice);
        assert!(
            advice.contains(&refused) && !advice.contains("remov"),
            "{advice}"
        );
        let started = start(&root, FIXTURE_SESSION);
        assert!(
            started.contains(&refused) && !started.contains("remov"),
            "{started}"
        );
        assert_eq!(
            three_files(&state),
            before,
            "a hook wrote over a newer state"
        );
        let mut names: Vec<String> = std::fs::read_dir(sessions())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        let stem = state.file_stem().unwrap().to_string_lossy().into_owned();
        assert_eq!(
            names,
            ["json", "judged", "lock", "tips"].map(|kind| format!("{stem}.{kind}")),
            "something beside the state was left"
        );
    }
    assert!(runs(&log) >= 2, "the tree was not judged");
}

/// Rule V6: a hook writes a state only after reading what is there, and never over a newer
/// one. Here a newer build writes the state while a stop of this session runs (the judge
/// does it, as a hook of a build that took no lock could): the stop's answer stands, it says
/// that its state could not be kept and why, and the state, its list of commits and its list
/// of judged commits are as the newer build left them. With file locks the session's lock
/// keeps two hooks of a session apart; this is the case without them, where the state is read
/// again just before it is moved into place, which narrows that race and does not close it.
#[test]
fn a_hook_never_writes_over_a_newer_state_placed_while_it_runs() {
    let (_, files) = fixtures().remove(0);
    let scratch = Scratch::new("hook-state-newer-meanwhile");
    let root = captured_repository(&scratch);
    let state = place_fixture(&files, &root);
    let mut newer: Value = serde_json::from_str(&std::fs::read_to_string(&state).unwrap()).unwrap();
    newer["schema"] = json!("deep-diff-forge.hook-state.v14");
    let newer_file = scratch.join("newer.json");
    std::fs::write(&newer_file, newer.to_string()).unwrap();
    let (script, log, exit_file) = (
        scratch.join("judge"),
        scratch.join("judge.log"),
        scratch.join("judge.exit"),
    );
    std::fs::write(&exit_file, "7").unwrap();
    let body = judge_script(
        &log,
        &exit_file,
        &verdict("fail", "no-silent-suppression", "check_suppressed", "a.rs"),
    )
    .replacen(
        "#!/bin/sh\n",
        &format!(
            "#!/bin/sh\ncp '{}' '{}.next' && mv '{}.next' '{}'\n",
            newer_file.display(),
            state.display(),
            state.display(),
            state.display()
        ),
        1,
    );
    write_program(&script, &body);
    // a commit the fixture's state does not know, so the stop has something to write: its
    // answer in the list of judged commits
    std::fs::write(root.join("d.rs"), "// BADLINE new\n").unwrap();
    git_as_captured(&root, &["add", "d.rs"]);
    git_as_captured(&root, &["commit", "--quiet", "-m", "more failing work"]);
    let decision = stop_from(&root, FIXTURE_SESSION, &script);
    let said = decision["systemMessage"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
        + decision["reason"].as_str().unwrap_or_default();
    assert!(
        said.contains("The hook could not keep its state (its state could not be saved: this session's state file ")
            && said.contains("is now of schema v14, written by a newer deep-diff-forge"),
        "{decision}"
    );
    assert_eq!(runs(&log), 2);
    assert_eq!(
        std::fs::read(&state).unwrap(),
        std::fs::read(&newer_file).unwrap()
    );
    assert_eq!(
        std::fs::read(state.with_extension("tips")).unwrap(),
        files["tips"]
    );
    assert_eq!(
        std::fs::read(state.with_extension("judged")).unwrap(),
        files["judged"]
    );
}

/// Rule V7: a damaged state is refused with what removing it costs, and that is what the next
/// hook then does: with the file removed, the stop takes the commit `HEAD` names as the
/// session's base, says that the session's start was not recorded, and the failing commit
/// the session made before is not judged.
#[test]
fn a_damaged_state_says_what_removing_it_costs() {
    let scratch = Scratch::new("hook-state-damaged");
    let root = repository(&scratch);
    let (script, log, _) = failing_judge(&scratch);
    start(&root, "s");
    std::fs::write(root.join("c.rs"), "fn c() {}\n").unwrap();
    git(&root, &["add", "c.rs"]);
    git(
        &root,
        &["commit", "--quiet", "-m", "failing work of the session"],
    );
    let state = states_in(&sessions()).remove(0);
    for damage in [
        "{\"schema\":\"deep-diff-forge.hook-state.v11\"}",
        "{\"schema\":\"deep-diff-forge.hook-state.v010\"}",
        "{",
    ] {
        std::fs::write(&state, damage).unwrap();
        // F12.d with the ledger (the closing round's ruling 3): the session's commit is judged
        // by the first such stop and blocks once; the ledger keeps its verdict
        let decision = stop_from(&root, "s", &script);
        let said = if runs(&log) == 1 && decision.get("decision").is_some() {
            blocked(&decision).to_owned()
        } else {
            allowed(&decision).to_owned()
        };
        assert!(
            said.starts_with(&stateless("state.damaged"))
                && said.contains("Why: this session's state file ")
                && said.contains("cannot be read as a state (")
                && said.contains(&format!("; nothing is recorded over it ({REMOVING_COSTS})")),
            "{said}"
        );
        assert_eq!(std::fs::read_to_string(&state).unwrap(), damage);
    }
    assert_eq!(runs(&log), 1, "the session's commit was not judged once");
    // what removing it costs, as the next hook finds it: the commit has its verdict in the
    // project's ledger, so it is not judged again
    std::fs::remove_file(&state).unwrap();
    let said = allowed(&stop_from(&root, "s", &script)).to_owned();
    assert!(
        said.contains("This session's start was not recorded"),
        "{said}"
    );
    assert_eq!(runs(&log), 1, "the commit was judged twice");
}

/// Rule V10: the names of a session's files and its lock are the documented ones, and do not
/// change from one schema to the next: `session-<the first 32 hex digits of the SHA-256 of the
/// session's id>` with `.json` (the state), `.tips`, `.judged` and `.lock` (`flock`).
#[test]
fn state_and_lock_names_are_the_documented_ones() {
    let scratch = Scratch::new("hook-state-names");
    let root = repository(&scratch);
    let (script, _, _) = failing_judge(&scratch);
    start(&root, "a session id");
    silence_a_lint(&root);
    git(&root, &["commit", "--quiet", "-am", "failing work"]);
    blocked(&stop_from(&root, "a session id", &script));
    let stem = format!(
        "session-{}",
        &deep_diff_forge_core::sha256_hex(b"a session id")[..32]
    );
    let mut names: Vec<String> = std::fs::read_dir(sessions())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    let mut documented: Vec<String> = ["json", "judged", "lock", "tips"]
        .map(|kind| format!("{stem}.{kind}"))
        .into();
    documented.insert(0, "key".to_owned());
    // (and the directory's `.gitignore`, documented since 0.11.1: P5d)
    documented.insert(0, ".gitignore".to_owned());
    assert_eq!(names, documented);
}

/// Copy the files of the commit `HEAD` names in `tree` into `into`, as `git archive` would.
fn copy_of_head(tree: &Path, into: &Path) {
    let names = git(tree, &["ls-tree", "-r", "--name-only", "HEAD"]);
    for name in names.lines() {
        let to = into.join(name);
        std::fs::create_dir_all(to.parent().unwrap()).unwrap();
        std::fs::write(&to, git(tree, &["show", &format!("HEAD:{name}")])).unwrap();
    }
}

/// Review V4 round 23, VD4R23-M-1, as reproduced: two genuine linked trees, the inner one
/// inside the outer one, and the inner tree's own `core.worktree` naming the outer tree. Git
/// named the outer top (it holds the session's directory) with the inner tree's Git
/// directory: the outer tree, clean, was judged under the inner tree's name, what had blocked
/// in the inner tree was forgotten, and the next stop printed nothing with two keys in it.
/// Git's answer is believed only as a pair that names one tree: UNJUDGED, nothing forgotten,
/// and no stop silent while the keys are there.
#[test]
fn nested_trees_one_pointed_at_the_other_are_unjudged_and_keep_what_blocked() {
    let scratch = Scratch::new("hook-nested-redirect");
    let root = repository(&scratch);
    let (script, log) = reading_judge(&scratch);
    let (outer, inner) = (scratch.join("outer"), scratch.join("outer/inner"));
    linked_tree(&root, &outer, "outer");
    linked_tree(&root, &inner, "inner");
    start(&root, "s");
    secret(&inner, "first.rs");
    blocked(&stop_from(&inner, "s", &script));
    let before = remembered_lines();
    git(&root, &["config", "extensions.worktreeConfig", "true"]);
    git(
        &inner,
        &[
            "config",
            "--worktree",
            "core.worktree",
            outer.to_str().unwrap(),
        ],
    );
    // the outer tree is clean, the inner one is not
    let mut exclude = std::fs::OpenOptions::new()
        .append(true)
        .open(root.join(".git/info/exclude"))
        .unwrap();
    exclude.write_all(b"\n/inner/\n").unwrap();
    assert_eq!(git(&inner, &["status", "--porcelain"]), "");
    secret(&inner, "second.rs");
    for _ in 0..2 {
        let said = allowed(&stop_from(&inner, "s", &script)).to_owned();
        assert!(
            said.starts_with("deep-diff-forge: UNJUDGED. ")
                && said.contains(
                    "/outer\" as the working tree of a Git directory whose records name "
                )
                && said.contains("/outer/inner\" as that tree"),
            "{said}"
        );
        assert_eq!(remembered_lines(), before);
        let from_project = allowed(&stop_from(&root, "s", &script)).to_owned();
        assert!(
            // in full the first time, by its name after it
            from_project.contains(
                "/outer/inner\" of this project has changes that are not committed could not be found out (Git names "
            ) && from_project.contains("A failure there has blocked a stop of this session and stands")
                || from_project
                    .contains("/outer/inner\" (it could not be looked at) (a failure stands there)"),
            "{from_project}"
        );
        assert_eq!(remembered_lines(), before);
    }
    assert!(
        !shown(&log).iter().any(|names| names.contains("second.rs")),
        "{:?}",
        shown(&log)
    );
}

/// Fifteenth independent review of the hooks, HR15-L-1, as reproduced: a linked tree's own
/// `core.worktree` names a subdirectory of the tree that holds a clean copy of its files. Git's
/// top held the session's directory, so it was believed: the copy was judged as the tree,
/// found clean, and what had blocked in the tree was forgotten; the stops after it printed
/// nothing with two keys in the tree, and from the project the tree was never named again.
#[test]
fn a_tree_pointed_at_a_copy_inside_itself_is_unjudged_and_keeps_what_blocked() {
    let scratch = Scratch::new("hook-redirect-inside");
    let root = repository(&scratch);
    let (script, log) = reading_judge(&scratch);
    let linked = scratch.join("linked");
    linked_tree(&root, &linked, "side");
    start(&root, "s");
    secret(&linked, "first.rs");
    blocked(&stop_from(&linked, "s", &script));
    let before = remembered_lines();
    let inside = linked.join("sub");
    copy_of_head(&linked, &inside);
    git(&root, &["config", "extensions.worktreeConfig", "true"]);
    git(
        &linked,
        &[
            "config",
            "--worktree",
            "core.worktree",
            inside.to_str().unwrap(),
        ],
    );
    assert_eq!(git(&inside, &["status", "--porcelain"]), "");
    secret(&linked, "second.rs");
    for cwd in [&inside, &inside, &linked] {
        let said = allowed(&stop_from(cwd, "s", &script)).to_owned();
        assert!(
            said.starts_with("deep-diff-forge: UNJUDGED. ")
                && said.contains(
                    "/linked/sub\" as the working tree of a Git directory whose records name "
                )
                && said.contains("/linked\" as that tree"),
            "{said}"
        );
        assert_eq!(remembered_lines(), before);
    }
    let from_project = allowed(&stop_from(&root, "s", &script)).to_owned();
    assert!(
        from_project.contains(
            "/linked\" of this project has changes that are not committed could not be found out (Git names "
        ) && from_project.contains("A failure there has blocked a stop of this session and stands"),
        "{from_project}"
    );
    assert_eq!(remembered_lines(), before);
    assert!(
        !shown(&log).iter().any(|names| names.contains("second.rs")),
        "{:?}",
        shown(&log)
    );
}

/// Fifteenth independent review of the hooks, HR15-L-2, as reproduced: a copy of a linked
/// tree (`cp -a`) has a `.git` file that names the tree's Git directory, so Git calls the copy
/// a working tree with the tree's name. Cleaned with `git clean` and stopped from, the copy was
/// judged as the tree and found clean, what had blocked in the tree was forgotten at a stop
/// whose words said it stands, and the next stop from the tree, unchanged, blocked a second
/// time. The copy's top is not the top the tree's records name: UNJUDGED, nothing forgotten.
#[test]
fn a_copy_of_a_linked_tree_is_not_judged_as_that_tree() {
    let scratch = Scratch::new("hook-tree-copy");
    let root = repository(&scratch);
    let (script, log) = reading_judge(&scratch);
    let (linked, copy) = (scratch.join("linked"), scratch.join("copy"));
    linked_tree(&root, &linked, "side");
    start(&root, "s");
    secret(&linked, "first.rs");
    blocked(&stop_from(&linked, "s", &script));
    let before = remembered_lines();
    let copied = Command::new("cp")
        .arg("-a")
        .arg(&linked)
        .arg(&copy)
        .status()
        .unwrap();
    assert!(copied.success());
    git(&copy, &["clean", "-fdq"]);
    let said = allowed(&stop_from(&copy, "s", &script)).to_owned();
    assert!(
        said.starts_with("deep-diff-forge: UNJUDGED. ")
            && said.contains("/copy\" as the working tree of a Git directory whose records name ")
            && said.contains("/linked\" as that tree"),
        "{said}"
    );
    assert_eq!(remembered_lines(), before);
    let said = allowed(&stop_from(&linked, "s", &script)).to_owned();
    assert!(said.starts_with("deep-diff-forge: FAIL STANDS. "), "{said}");
    assert_eq!(remembered_lines(), before);
    // the copy was put to no judge: the tree was judged at its two stops alone
    assert_eq!(shown(&log), ["first.rs"; 2]);
}

/// The pair rule where a repository records no top level of its main tree: its Git directory
/// was made apart (`git init --separate-git-dir`), and `git worktree list` names that
/// directory as the main tree. The project, the session begun in its main tree, is judged as
/// before from a directory under it. A copy of it (`cp -a`), whose `.git` file names the same
/// Git directory, was judged as a tree of the project under the main tree's name; now its top
/// is not the project's, and the stop is UNJUDGED with nothing forgotten.
#[test]
fn a_copy_of_a_project_whose_git_directory_was_made_apart_is_not_judged_as_it() {
    let scratch = Scratch::new("hook-separate-git-dir");
    home_in(&scratch);
    let (script, log) = reading_judge(&scratch);
    let (root, store, copy) = (
        scratch.join("work"),
        scratch.join("store.git"),
        scratch.join("copy"),
    );
    std::fs::create_dir_all(root.join("lib")).unwrap();
    git(
        &root,
        &[
            "init",
            "--quiet",
            "--separate-git-dir",
            store.to_str().unwrap(),
        ],
    );
    std::fs::write(root.join("a.rs"), "fn a() {}\n").unwrap();
    std::fs::write(root.join("lib/b.rs"), "fn b() {}\n").unwrap();
    std::fs::write(root.join("warden.toml"), "# the policy\n").unwrap();
    git(&root, &["add", "--all"]);
    git(&root, &["commit", "--quiet", "-m", "base"]);
    start(&root, "s");
    secret(&root, "first.rs");
    blocked(&stop_from(&root.join("lib"), "s", &script));
    let before = remembered_lines();
    let said = allowed(&stop_from(&root.join("lib"), "s", &script)).to_owned();
    assert!(said.starts_with("deep-diff-forge: FAIL STANDS. "), "{said}");
    let copied = Command::new("cp")
        .arg("-a")
        .arg(&root)
        .arg(&copy)
        .status()
        .unwrap();
    assert!(copied.success());
    git(&copy, &["clean", "-fdq"]);
    let said = allowed(&stop_from(&copy.join("lib"), "s", &script)).to_owned();
    assert!(
        said.contains("/copy\" as the working tree of a Git directory whose records name ")
            && said.contains("/work\" as that tree"),
        "{said}"
    );
    assert_eq!(remembered_lines(), before);
    let said = allowed(&stop_from(&root, "s", &script)).to_owned();
    assert!(said.starts_with("deep-diff-forge: FAIL STANDS. "), "{said}");
    assert_eq!(remembered_lines(), before);
    // the project was judged at each of its four stops, the copy never
    assert_eq!(shown(&log), ["first.rs"; 4]);
}

/// Fifteenth independent review of the hooks (a note): a ceiling entry written through the
/// project's path (`repo/../hidden`, which is `hidden` beside it) was compared as written,
/// started with the project's path, and was left out: the repository it hides was found and
/// named. Entries are compared in their resolved form.
#[test]
fn a_ceiling_written_through_the_project_still_hides_its_repository() {
    let scratch = Scratch::new("hook-ceiling-dotdot");
    let root = repository(&scratch);
    let (script, log) = reading_judge(&scratch);
    let hidden = scratch.join("hidden");
    std::fs::create_dir_all(hidden.join("notes/today")).unwrap();
    git(&hidden, &["init", "--quiet"]);
    std::fs::write(hidden.join(".bashrc"), "export A=1\n").unwrap();
    git(&hidden, &["add", "--all"]);
    git(&hidden, &["commit", "--quiet", "-m", "dotfiles"]);
    std::fs::write(hidden.join(".bashrc"), "export A=2\nSECRET\n").unwrap();
    start(&root, "s");
    let (code, stdout, stderr) = hook_in(
        "stop",
        &["--judge", script.to_str().unwrap()],
        &input("Stop", &hidden.join("notes/today"), "s", false),
        |command| {
            command.env("GIT_CEILING_DIRECTORIES", root.join("../hidden"));
        },
    );
    assert_eq!((code, stderr.as_str(), stdout.as_str()), (0, "", ""));
    assert_eq!(shown(&log), Vec::<String>::new());
}

/// Review V4 round 24, VD4R24-M-1, as reproduced: two genuine linked trees, the inner one
/// inside the outer one; the inner tree's own `core.worktree` names the outer tree, and its
/// administrative `gitdir` record is edited to name the outer tree's `.git`. Git and the
/// records then name the outer tree as the inner one: the clean outer tree was judged under
/// the inner tree's name, what had blocked in the inner tree was forgotten, and the next
/// stops printed nothing with two keys in it. An identity does not move while the directory
/// it is remembered at stands: UNJUDGED there, named from the project, nothing forgotten.
#[test]
fn a_tree_whose_records_name_another_is_unjudged_while_its_directory_stands() {
    let scratch = Scratch::new("hook-records-moved");
    let root = repository(&scratch);
    let (script, log) = reading_judge(&scratch);
    let (outer, inner) = (scratch.join("outer"), scratch.join("outer/inner"));
    linked_tree(&root, &outer, "outer");
    linked_tree(&root, &inner, "inner");
    start(&root, "s");
    secret(&inner, "first.rs");
    blocked(&stop_from(&inner, "s", &script));
    let before = remembered_lines();
    let admin = PathBuf::from(git(&inner, &["rev-parse", "--absolute-git-dir"]).trim());
    git(&root, &["config", "extensions.worktreeConfig", "true"]);
    git(
        &inner,
        &[
            "config",
            "--worktree",
            "core.worktree",
            outer.to_str().unwrap(),
        ],
    );
    std::fs::write(
        admin.join("gitdir"),
        format!("{}\n", outer.join(".git").display()),
    )
    .unwrap();
    let mut exclude = std::fs::OpenOptions::new()
        .append(true)
        .open(root.join(".git/info/exclude"))
        .unwrap();
    exclude.write_all(b"\n/inner/\n").unwrap();
    assert_eq!(git(&inner, &["status", "--porcelain"]), "");
    secret(&inner, "second.rs");
    for _ in 0..2 {
        let said = allowed(&stop_from(&inner, "s", &script)).to_owned();
        assert!(
            said.starts_with("deep-diff-forge: UNJUDGED. ")
                && said.contains("/outer\" as the working tree that this session's memory has at ")
                && said.contains("/outer/inner\", which still stands"),
            "{said}"
        );
        assert_eq!(remembered_lines(), before);
        let from_project = allowed(&stop_from(&root, "s", &script)).to_owned();
        assert!(
            from_project.contains("/outer/inner\", which still stands")
                || from_project.contains(
                    "/outer\" (remembered at another directory, which stands) (a failure stands there)"
                ),
            "{from_project}"
        );
        assert_eq!(remembered_lines(), before);
    }
    assert!(
        !shown(&log).iter().any(|names| names.contains("second.rs")),
        "{:?}",
        shown(&log)
    );
}

/// Sixteenth independent review of the hooks, HR16-L-3, as reproduced: a copy of a linked
/// tree (`cp -a`), cleaned with `git clean`, and `git worktree repair` run in the copy: Git's
/// records name the copy as the tree. The clean copy was read under the tree's name at the
/// next stop from the project, what had blocked in the tree was forgotten, and the stops from
/// the project and from the copy printed nothing with two keys in the tree. While the tree's
/// own directory stands, neither directory is read as that tree, and both are named.
#[test]
fn a_copy_the_records_were_repaired_onto_is_not_read_while_the_tree_stands() {
    let scratch = Scratch::new("hook-copy-repaired");
    let root = repository(&scratch);
    let (script, log) = reading_judge(&scratch);
    let (linked, copy) = (scratch.join("linked"), scratch.join("copy"));
    linked_tree(&root, &linked, "side");
    start(&root, "s");
    secret(&linked, "first.rs");
    blocked(&stop_from(&linked, "s", &script));
    let before = remembered_lines();
    let copied = Command::new("cp")
        .arg("-a")
        .arg(&linked)
        .arg(&copy)
        .status()
        .unwrap();
    assert!(copied.success());
    git(&copy, &["clean", "-fdq"]);
    git(&copy, &["worktree", "repair"]);
    secret(&linked, "second.rs");
    let both = |said: &str| {
        said.contains("/copy\" as the working tree that this session's memory has at ")
            && said.contains("/linked\", which still stands")
    };
    for _ in 0..2 {
        let from_project = allowed(&stop_from(&root, "s", &script)).to_owned();
        assert!(
            both(&from_project)
                || from_project.contains(
                    "/copy\" (remembered at another directory, which stands) (a failure stands there)"
                ),
            "{from_project}"
        );
        assert_eq!(remembered_lines(), before);
        let said = allowed(&stop_from(&copy, "s", &script)).to_owned();
        assert!(
            said.starts_with("deep-diff-forge: UNJUDGED. ") && both(&said),
            "{said}"
        );
        assert_eq!(remembered_lines(), before);
    }
    let said = allowed(&stop_from(&linked, "s", &script)).to_owned();
    assert!(
        said.starts_with("deep-diff-forge: UNJUDGED. ") && said.contains("/copy\" as that tree"),
        "{said}"
    );
    assert_eq!(remembered_lines(), before);
    assert!(
        !shown(&log).iter().any(|names| names.contains("second.rs")),
        "{:?}",
        shown(&log)
    );
}

/// Sixteenth independent review of the hooks, HR16-M-1, as reproduced: a copy of a linked tree
/// made inside the project (`cp -a ../linked feature-copy`), with a key in it. Git sees a
/// repository inside the project and leaves it out of the patch; the hooks took it for a tree
/// of the project named by the rule for working trees, which names only listed trees, and
/// every stop from the project printed nothing. It is named as left out, at every stop, as a
/// nested repository is.
#[test]
fn a_copy_of_a_linked_tree_inside_the_project_is_named_as_left_out() {
    let scratch = Scratch::new("hook-copy-inside");
    let root = repository(&scratch);
    let (script, log) = reading_judge(&scratch);
    let linked = scratch.join("linked");
    linked_tree(&root, &linked, "side");
    start(&root, "s");
    let copy = root.join("feature-copy");
    let copied = Command::new("cp")
        .arg("-a")
        .arg(&linked)
        .arg(&copy)
        .status()
        .unwrap();
    assert!(copied.success());
    secret(&copy, "incopy.rs");
    for _ in 0..3 {
        let said = allowed(&stop_from(&root, "s", &script)).to_owned();
        assert!(
            said.contains("The directory \"feature-copy\" in \"")
                && said.contains(
                    "is no working tree of this project by the repository's records (Git names "
                )
                || said.contains("the directory \"feature-copy\" (no working tree by the records)"),
            "{said}"
        );
    }
    assert!(
        !shown(&log).iter().any(|names| names.contains("incopy.rs")),
        "{:?}",
        shown(&log)
    );
}

/// Sixteenth independent review of the hooks, HR16-L-1, as reproduced: the administrative
/// `gitdir` record of one linked tree edited to name another tree's `.git`, so that Git
/// lists the other tree's path twice. A stop from that other tree left both entries out as
/// judged, by their path, and the failure that stood in the first tree was neither looked
/// at nor said: the stop printed nothing with a second key there. A listed tree is left out
/// as judged only by which tree it is.
#[test]
fn two_listed_entries_with_one_path_are_two_trees() {
    let scratch = Scratch::new("hook-listed-twice");
    let root = repository(&scratch);
    let (script, log) = reading_judge(&scratch);
    let (first, other) = (scratch.join("first"), scratch.join("other"));
    linked_tree(&root, &first, "first");
    linked_tree(&root, &other, "other");
    start(&root, "s");
    secret(&first, "first.rs");
    blocked(&stop_from(&first, "s", &script));
    let before = remembered_lines();
    std::fs::write(
        root.join(".git/worktrees/first/gitdir"),
        format!("{}\n", other.join(".git").display()),
    )
    .unwrap();
    let listed = git(&root, &["worktree", "list", "--porcelain"]);
    let entries = listed
        .lines()
        .filter(|line| line.starts_with("worktree ") && line.ends_with("/other"))
        .count();
    assert_eq!(entries, 2, "{listed}");
    secret(&first, "second.rs");
    for cwd in [&other, &root, &other] {
        let said = allowed(&stop_from(cwd, "s", &script)).to_owned();
        assert!(
            said.contains("/first\", which still stands")
                || said.contains(
                    "/other\" (remembered at another directory, which stands) (a failure stands there)"
                ),
            "{said}"
        );
        assert_eq!(remembered_lines(), before);
    }
    assert!(
        !shown(&log).iter().any(|names| names.contains("second.rs")),
        "{:?}",
        shown(&log)
    );
}

/// Seventeenth independent review of the hooks, HR17-M-1, as reproduced: a tree a failure
/// stands in is moved with `git worktree move`, and something new is made at its old path (a
/// new tree, a clone, a build directory). Any directory at the recorded path counted as the
/// tree standing there: every stop from the moved tree was UNJUDGED and a new key in it never
/// blocked. Only a directory whose `.git` names the tree's own administrative directory is the
/// tree: the moved tree is judged where it is, its failure stands, and a new one blocks.
#[test]
fn a_tree_moved_by_git_is_judged_whatever_is_made_at_its_old_path() {
    for made in ["tree", "clone", "build"] {
        let scratch = Scratch::new("hook-moved-then-made");
        let root = repository(&scratch);
        let (script, log) = reading_judge(&scratch);
        let (linked, moved) = (scratch.join("linked"), scratch.join("moved"));
        linked_tree(&root, &linked, "side");
        start(&root, "s");
        secret(&linked, "first.rs");
        blocked(&stop_from(&linked, "s", &script));
        let at = [linked.to_str().unwrap(), moved.to_str().unwrap()];
        git(&root, &["worktree", "move", at[0], at[1]]);
        match made {
            "tree" => linked_tree(&root, &linked, "fresh"),
            "clone" => {
                git(
                    &scratch.join("."),
                    &[
                        "clone",
                        "--quiet",
                        "--no-local",
                        root.to_str().unwrap(),
                        at[0],
                    ],
                );
            }
            _ => std::fs::create_dir_all(linked.join("target/debug")).unwrap(),
        }
        let said = allowed(&stop_from(&moved, "s", &script)).to_owned();
        assert!(
            said.starts_with("deep-diff-forge: FAIL STANDS. "),
            "{made}: {said}"
        );
        // a second key is put to the judge (the stand-in's failure is one line whatever the
        // file, so it stands with the first; with the real judge it is a new item and blocks)
        secret(&moved, "second.rs");
        let said = allowed(&stop_from(&moved, "s", &script)).to_owned();
        assert!(
            said.starts_with("deep-diff-forge: FAIL STANDS. "),
            "{made}: {said}"
        );
        assert!(
            shown(&log)
                .last()
                .is_some_and(|names| names.contains("second.rs")),
            "{made}: {:?}",
            shown(&log)
        );
    }
}

/// Seventeenth independent review of the hooks, HR17-M-1: two trees, a failure standing in
/// each, swapped with three `git worktree move`. Each old directory holds the other tree, so
/// neither is where its tree stood: each is judged where it is now, and each failure stands.
#[test]
fn two_trees_swapped_by_git_are_each_judged_where_they_are() {
    let scratch = Scratch::new("hook-swapped");
    let root = repository(&scratch);
    let (script, _) = reading_judge(&scratch);
    let (a, b, between) = (
        scratch.join("a"),
        scratch.join("b"),
        scratch.join("between"),
    );
    linked_tree(&root, &a, "a");
    linked_tree(&root, &b, "b");
    start(&root, "s");
    secret(&a, "a.rs");
    blocked(&stop_from(&a, "s", &script));
    secret(&b, "b.rs");
    blocked(&stop_from(&b, "s", &script));
    let sorted = || {
        let mut lines = remembered_lines();
        lines.sort();
        lines
    };
    let before = sorted();
    for (from, to) in [(&a, &between), (&b, &a), (&between, &b)] {
        git(
            &root,
            &[
                "worktree",
                "move",
                from.to_str().unwrap(),
                to.to_str().unwrap(),
            ],
        );
    }
    for tree in [&a, &b] {
        let said = allowed(&stop_from(tree, "s", &script)).to_owned();
        assert!(said.starts_with("deep-diff-forge: FAIL STANDS. "), "{said}");
        assert_eq!(sorted(), before);
    }
}

/// Seventeenth independent review of the hooks, HR17-M-1 (`moveaddcommit`): after the move and
/// a new tree at the old path, a new key in the moved tree is judged while it is uncommitted
/// (with the stand-in judge it is the same failure line, so it stands), and its commit blocks
/// as a commit does, once.
#[test]
fn a_new_failure_in_a_tree_moved_by_git_is_judged_before_and_after_its_commit() {
    let scratch = Scratch::new("hook-moved-commit");
    let root = repository(&scratch);
    let (script, log) = reading_judge(&scratch);
    let (linked, moved) = (scratch.join("linked"), scratch.join("moved"));
    linked_tree(&root, &linked, "side");
    start(&root, "s");
    secret(&linked, "first.rs");
    blocked(&stop_from(&linked, "s", &script));
    git(
        &root,
        &[
            "worktree",
            "move",
            linked.to_str().unwrap(),
            moved.to_str().unwrap(),
        ],
    );
    linked_tree(&root, &linked, "fresh");
    secret(&moved, "second.rs");
    let said = allowed(&stop_from(&moved, "s", &script)).to_owned();
    assert!(said.starts_with("deep-diff-forge: FAIL STANDS. "), "{said}");
    assert!(
        shown(&log)
            .last()
            .is_some_and(|names| names.contains("second.rs")),
        "{:?}",
        shown(&log)
    );
    git(&moved, &["add", "second.rs"]);
    git(&moved, &["commit", "--quiet", "-m", "second"]);
    blocked(&stop_from(&moved, "s", &script));
    let said = allowed(&stop_from(&moved, "s", &script)).to_owned();
    assert!(said.starts_with("deep-diff-forge: FAIL STANDS. "), "{said}");
}

/// A linked tree moved with a plain `mv` and not repaired is a pair the records do not name:
/// a stop from it is UNJUDGED and says how it is put right; after `git worktree repair` the
/// directory it was remembered at is gone, so the tree is read where it is now, and its
/// unchanged failure stands (it does not block a second time).
#[test]
fn a_tree_moved_by_hand_is_unjudged_until_repaired_and_then_stands() {
    // (a short name: the words come after both paths, in what a message shows of a reason)
    let scratch = Scratch::new("hook-mv");
    let root = repository(&scratch);
    let (script, _) = reading_judge(&scratch);
    let (linked, moved) = (scratch.join("linked"), scratch.join("moved"));
    linked_tree(&root, &linked, "side");
    start(&root, "s");
    secret(&linked, "first.rs");
    blocked(&stop_from(&linked, "s", &script));
    let before = remembered_lines();
    std::fs::rename(&linked, &moved).unwrap();
    let said = allowed(&stop_from(&moved, "s", &script)).to_owned();
    assert!(
        said.starts_with("deep-diff-forge: UNJUDGED. ")
            && said.contains("/linked\" as that tree")
            && said.contains("as that tree (`git worktree repair` in a tree moved with `mv` records where it is): "),
        "{said}"
    );
    assert_eq!(remembered_lines(), before);
    git(&moved, &["worktree", "repair"]);
    for _ in 0..2 {
        let said = allowed(&stop_from(&moved, "s", &script)).to_owned();
        assert!(said.starts_with("deep-diff-forge: FAIL STANDS. "), "{said}");
        assert_eq!(remembered_lines(), before);
    }
}

/// Package P3 of 0.11.1, row 9 of the reading of the macOS CI run of 0.11.0: the test above
/// under a directory of 200 characters and more. A message shows 300 characters of a reason,
/// and under a long path the cut took the words after the two paths: now the paths give way
/// in their middle, and the reason says what to do and what is not judged, within its bound.
#[test]
fn under_a_long_path_a_moved_tree_s_reason_keeps_its_words() {
    let scratch = Scratch::new("hook-long-mv");
    let root = repository(&scratch);
    let (script, _) = reading_judge(&scratch);
    let deep = scratch.join("d".repeat(180));
    std::fs::create_dir(&deep).unwrap();
    let (linked, moved) = (deep.join("linked"), deep.join("moved"));
    linked_tree(&root, &linked, "side");
    start(&root, "s");
    secret(&linked, "first.rs");
    blocked(&stop_from(&linked, "s", &script));
    std::fs::rename(&linked, &moved).unwrap();
    let said = allowed(&stop_from(&moved, "s", &script)).to_owned();
    let reason = said
        .lines()
        .find_map(|line| line.strip_prefix("  not judged: "))
        .unwrap_or_else(|| panic!("no reason line: {said}"));
    assert!(
        said.starts_with("deep-diff-forge: UNJUDGED. ")
            && reason.starts_with("Git names \"/")
            && reason.contains("/linked\" as that tree (`git worktree repair` in a tree moved with `mv` records where it is): whether the session's directory is another working tree could not be found out")
            && reason.chars().count() <= 301,
        "{said}"
    );
}

/// Package P3 of 0.11.1, review 1, M1: the same stop from a tree moved to a directory whose
/// name ends in a backslash, under a short base and a long one. Its path is shown ending `\"`;
/// read by its quotes, the reason ran its first path on to the next and cut the words between
/// them ("Git names "/var/…e records name "…/linked" as that tree … Git direct"). The words
/// are whole and the path keeps its backslash at its end.
#[test]
fn a_tree_moved_to_a_name_ending_in_a_backslash_keeps_its_reason_s_words() {
    for depth in [0, 180] {
        let scratch = Scratch::new("hook-mv-bs");
        let root = repository(&scratch);
        let (script, _) = reading_judge(&scratch);
        let deep = scratch.join("d".repeat(depth.max(1)));
        std::fs::create_dir(&deep).unwrap();
        let (linked, moved) = (deep.join("linked"), deep.join("moved\\"));
        linked_tree(&root, &linked, "side");
        start(&root, "s");
        secret(&linked, "first.rs");
        blocked(&stop_from(&linked, "s", &script));
        std::fs::rename(&linked, &moved).unwrap();
        let said = allowed(&stop_from(&moved, "s", &script)).to_owned();
        let reason = said
            .lines()
            .find_map(|line| line.strip_prefix("  not judged: "))
            .unwrap_or_else(|| panic!("no reason line: {said}"));
        assert!(
            said.starts_with("deep-diff-forge: UNJUDGED. ")
                && reason.starts_with("Git names \"/")
                && reason.contains("/moved\\\" as the working tree of a Git directory whose records name \"/")
                && reason.contains("/linked\" as that tree (`git worktree repair` in a tree moved with `mv` records where it is): whether the session's directory is another working tree could not be found out")
                && reason.chars().count() <= 301,
            "{depth}: {said}"
        );
    }
}

/// Package P3 of 0.11.1, rows 5 to 8 of the reading of the macOS CI run of 0.11.0: the copy
/// of `a_copy_the_records_were_repaired_onto_is_not_read_while_the_tree_stands` under a
/// directory of 200 characters and more. The line that counts what is not judged showed 180
/// characters of a name and cut its clauses; now the path gives way in its middle, and both
/// clauses are whole, within the line's bound.
#[test]
fn under_a_long_path_the_counting_line_keeps_every_clause() {
    let scratch = Scratch::new("hook-long-copy");
    let root = repository(&scratch);
    let (script, _) = reading_judge(&scratch);
    let deep = scratch.join("d".repeat(180));
    std::fs::create_dir(&deep).unwrap();
    let (linked, copy) = (deep.join("linked"), deep.join("copy"));
    linked_tree(&root, &linked, "side");
    start(&root, "s");
    secret(&linked, "first.rs");
    blocked(&stop_from(&linked, "s", &script));
    let copied = Command::new("cp")
        .arg("-a")
        .arg(&linked)
        .arg(&copy)
        .status()
        .unwrap();
    assert!(copied.success());
    git(&copy, &["clean", "-fdq"]);
    git(&copy, &["worktree", "repair"]);
    secret(&linked, "second.rs");
    let lines: Vec<String> = (0..2)
        .filter_map(|_| {
            let said = allowed(&stop_from(&root, "s", &script)).to_owned();
            said.lines()
                .find(|line| line.contains(" in this project ") && line.contains(" not judged: "))
                .map(str::to_owned)
        })
        .collect();
    assert!(!lines.is_empty(), "no stop counted what is not judged");
    for line in &lines {
        assert!(
            line.contains(
                "/copy\" (remembered at another directory, which stands) (a failure stands there)"
            ) && line.contains("the working tree \"/")
                && line.chars().count() <= 700,
            "{line}"
        );
    }
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
    // a directory that is gone in the project's own tree, and a relative one that is not
    // there, taken from the hook's own directory (in no repository) and walked up from
    // there: the project is judged as before, and the linked tree is one the session is
    // not in
    for (n, cwd) in [root.join("gone/deep"), PathBuf::from("nowhere/at/all")]
        .into_iter()
        .enumerate()
    {
        let said = if cwd.is_absolute() {
            stop_from(&cwd, "s", &script)
        } else {
            stop_from_relative(cwd.to_str().unwrap(), scratch.path(), "s", &script)
        };
        let said = allowed(&said).to_owned();
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
    // (since hook state v12 the line keeps the class, never Git's words: lane H3)
    assert!(
        written[&id].starts_with("n0@")
            && written[&id].ends_with(":place.git_failed")
            && !written[&id].contains("made to fail by the test"),
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

// `hold_lock(path, go)`: an exclusive `flock(2)` on `path` (made when it is not there), held by
// another process until `go` is removed; returns when the lock is held. One holder for Linux and
// macOS, in the testkit (also used by `tests/council_tuicr.rs`): `flock(1)` is not on macOS.
use deep_diff_forge_testkit::hold_lock;

/// Whether a process of this user other than `but` has `path` open: a link under `/proc/*/fd`
/// names it, or where there is no `/proc` (macOS), `lsof -t` lists such a process.
fn opened_by_a_process(path: &Path, but: u32) -> bool {
    let but = but.to_string();
    if !Path::new("/proc/self/fd").is_dir() {
        // (`lsof -t`: the pids that have the file open, one a line, and exit 1 for none)
        let out = Command::new("lsof")
            .arg("-t")
            .arg("--")
            .arg(path)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .unwrap();
        return String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(str::trim)
            .any(|pid| !pid.is_empty() && pid != but);
    }
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
    // F12.d: it judges the tree without the state, and the credential there blocks
    let said = blocked(&said).to_owned();
    assert!(
        said.starts_with(&stateless("state.lock_busy"))
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
    // (the time the run had is said at the stop that ran it; the line keeps the class only,
    // since hook state v12: lane H3)
    assert!(line.ends_with(":time.judge_stopped"), "{line}");
    let had: f64 = said
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
        // (F12: it holds the first stop once; then the stop is as before)
        let held = stop_from(&root, "s", &script);
        assert!(blocked(&held).contains("\"locked.bin\""), "{held}");
        not_read("an unreadable file", stop_from(&root, "s", &script));
        not_read("again", stop_from(&root, "s", &script));
    }
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o644)).unwrap();
    std::fs::remove_file(&locked).unwrap();
    stands("the unreadable file is gone; the index is as it was");

    // a path marked skip-worktree
    git(&root, &["update-index", "--skip-worktree", "a.rs"]);
    let held = stop_from(&root, "s", &script);
    assert!(blocked(&held).contains("skip-worktree"), "{held}");
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
        // (the time each run had is said at the stop; the line keeps the class: lane H3)
        assert!(
            written[id].starts_with(line) && written[id].ends_with(":time.judge_stopped"),
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
        // (since hook state v12 a later stop says it by its class, never by the judge's
        // words, which were said at the stop that judged it: lane H3)
        assert!(
            later.contains(&format!(
                "1 held by the judge, for a person: Commit {noted} (held: the judge holds with no rule held, for its notes)"
            )) && !later.contains("patch_unverifiable"),
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

/// Two first stops of `session` from `root` at once, each with `home` the home of its hooks,
/// judged by the reading judge of `scratch` (`script`): what each said, in the order they
/// ended. The judge's first call waits until one of the stops has ended (`judge.hold`), so the
/// stop that holds the session's lock still holds it for all of the other's wait, however slow
/// the host: before, the first judge slept 5 s against the other's 3 s wait, and on a loaded
/// runner (macOS, `cargo test --workspace`) the other came to the lock after the first had
/// ended and rightly took it (review of P5c round 3, M1).
fn two_first_stops(
    scratch: &Scratch,
    root: &Path,
    script: &Path,
    session: &str,
    home: impl Fn() + Sync,
) -> Vec<Value> {
    std::fs::write(scratch.join("judge.hold"), "").unwrap();
    let both = std::thread::scope(|scope| {
        let (done, ended) = std::sync::mpsc::channel();
        for _ in 0..2 {
            let (done, home) = (done.clone(), &home);
            scope.spawn(move || {
                home();
                done.send(stop_from(root, session, script)).unwrap();
            });
        }
        let first = ended.recv().unwrap();
        let _ = std::fs::remove_file(scratch.join("judge.held"));
        vec![first, ended.recv().unwrap()]
    });
    let _ = std::fs::remove_file(scratch.join("judge.hold"));
    both
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
    let both = two_first_stops(&scratch, &root, &script, "first", || {
        home_in(&scratch);
    });
    let blocks = both
        .iter()
        .filter(|said| said["decision"] == "block")
        .count();
    // F12.d: the stop that finds the lock held judges the tree without the state, and its
    // failure blocks too; the state is the other stop's alone
    let waited = both
        .iter()
        .filter(|said| {
            said["reason"]
                .as_str()
                .is_some_and(|text| text.starts_with(&stateless("state.lock_busy")))
        })
        .count();
    assert_eq!((blocks, waited), (2, 1), "{both:?}");
    assert_eq!(runs(&log), 2, "a stop did not judge the tree");
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

// `forge-warden.verdict.v3` (S3.21). The tests above are as they were.

/// forge-warden's own v3 document: `fixtures/demo.verdict.json` on its main branch at commit
/// fbd621e0e4c3183e7cb9d9c949dac942b8fb0dd8 (the file last changed in dd05b45b), copied byte
/// for byte.
const FORGE_WARDEN_V3_DEMO: &str =
    include_str!("fixtures/forge-warden-main-fbd621e0/demo.verdict.json");

/// A v3 document for a stand-in judge, made from forge-warden's own: its rules whose outcome
/// is one of `keep`, then, when `builtin`, a built-in rule as forge-warden's main branch writes
/// one (`crates/forge-warden-core/src/builtin.rs` at fbd621e0); the verdict and the exit triple
/// of `outcome`; a summary that counts the rules; no notes (the demo's note is about its
/// advisory ranking). The judge fills in the seals.
fn verdict_v3(outcome: &str, keep: &[&str], builtin: bool) -> Value {
    let demo: Value = serde_json::from_str(FORGE_WARDEN_V3_DEMO).unwrap();
    let mut rules: Vec<Value> = demo["rules"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|rule| keep.contains(&rule["outcome"].as_str().unwrap()))
        .cloned()
        .collect();
    if builtin {
        rules.push(json!({"id": "builtin:binary-on-executable", "kind": "builtin",
            "severity": "hold", "outcome": "hold",
            "why": "an executable or a symlink sent as a binary section hides its lines from every signal rule, and the shell can still run it; a person reads it before it merges",
            "violations": [{"path": "tools/run",
                            "message": "a binary section on an executable file (100755)"}]}));
    }
    let count = |outcome: &str| rules.iter().filter(|r| r["outcome"] == outcome).count();
    let (code, name, action) = match outcome {
        "pass" => (0, "pass", "proceed"),
        "fail" => (7, "policy_fail", "change_input"),
        _ => (8, "policy_hold", "review_evidence"),
    };
    json!({
        "schema": demo["schema"],
        "tool": demo["tool"],
        "evaluated_on": demo["evaluated_on"],
        "seals": "@SEALS@",
        "verdict": outcome,
        "exit": {"code": code, "name": name, "action": action},
        "summary": {"files": 4, "rules": rules.len(), "passed": count("pass"),
                    "waived": count("waived"), "held": count("hold"), "failed": count("fail")},
        "rules": rules,
        "expired_waivers": [],
        "unused_waivers": [],
        "notes": [],
    })
}

/// The stand-in judge of [`judge`] printing `document` with bound claims in its seals, as
/// forge-warden's bound mode writes them (`crates/forge-warden-cli/tests/authority_mode.rs` at
/// fbd621e0): base, head and merge base of 40 hex digits, `verified: true`.
fn judge_v3_bound(scratch: &Scratch, document: &Value) -> (PathBuf, PathBuf) {
    let (script, log, exit_file) = judge(scratch, document);
    let text = judge_script(&log, &exit_file, document);
    let sealed = r#"\"allowlist_sha256\":\"$f\"}}""#;
    let claims = r#"\"allowlist_sha256\":\"$f\"},\"claims\":{\"base\":\"@A@\",\"head\":\"@B@\",\"merge_base\":\"@C@\",\"verified\":true}}""#
        .replace("@A@", &"a".repeat(40))
        .replace("@B@", &"b".repeat(40))
        .replace("@C@", &"c".repeat(40));
    let bound = text.replacen(sealed, &claims, 1);
    assert_ne!(bound, text, "the stand-in's seals were not found");
    write_program(&script, &bound);
    (script, exit_file)
}

/// S3.21: forge-warden's main branch writes `forge-warden.verdict.v3`. The stop hook reads it as
/// it reads v2: a fail holds the stop, with a built-in rule and verified claims in it too; a
/// hold is reported; a pass passes. Before, a v3 document was no verdict, and a fail was
/// allowed as UNJUDGED.
#[test]
fn a_v3_verdict_is_read_and_its_fail_holds_the_stop() {
    let scratch = Scratch::new("hook-verdict-v3");
    let root = repository(&scratch);
    std::fs::write(
        root.join("a.rs"),
        "fn a() {}\n#[allow(dead_code)]\nfn b() {}\n",
    )
    .unwrap();
    let stop_with = |document: &Value, code: &str, session: &str| -> Value {
        let (script, exit_file) = judge_v3_bound(&scratch, document);
        std::fs::write(&exit_file, code).unwrap();
        let (_, stdout, _) = hook(
            "stop",
            &["--judge", script.to_str().unwrap()],
            &input("Stop", &root, session, false),
        );
        serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("{e}: {stdout:?}"))
    };

    // fail: held, with the failing rule and the rules the judge holds for, the built-in one too
    let out = stop_with(
        &verdict_v3("fail", &["pass", "fail", "hold"], true),
        "7",
        "s1",
    );
    let reason = blocked(&out);
    assert!(
        reason.starts_with("deep-diff-forge: the judge fails this change")
            && reason.contains("- rule no-ci-privilege: pull_request_target, write-all")
            && reason.contains("- rule builtin:binary-on-executable (held, for a person): an executable or a symlink")
            && reason.contains("\"tools/run\""),
        "{reason}"
    );

    // hold: allowed and reported, the built-in rule named
    let out = stop_with(&verdict_v3("hold", &["pass", "hold"], true), "8", "s2");
    let said = allowed(&out);
    assert!(
        said.starts_with("deep-diff-forge: HOLD. ")
            && said.contains("- rule builtin:binary-on-executable: an executable or a symlink"),
        "{said}"
    );

    // pass: allowed, with nothing said but what the first stop of a session with no recorded
    // start says
    let out = stop_with(&verdict_v3("pass", &["pass"], false), "0", "s3");
    let said = allowed(&out);
    assert!(
        said.starts_with("deep-diff-forge: This session's start was not recorded")
            && !said.contains("UNJUDGED")
            && !said.contains("HOLD"),
        "{said}"
    );
}

/// S3.21, point (c): exit 7 is the judge's fail. A fail whose document this build cannot read
/// (here one that names a schema it does not know, as a later forge-warden would write it)
/// holds the stop once, says why, and then stands. The same document under exit 0 or 8 is
/// UNJUDGED, as before.
#[test]
fn a_fail_whose_document_cannot_be_read_holds_the_stop_once() {
    let scratch = Scratch::new("hook-fail-unread");
    let root = repository(&scratch);
    std::fs::write(
        root.join("a.rs"),
        "fn a() {}\n#[allow(dead_code)]\nfn b() {}\n",
    )
    .unwrap();
    let mut later = verdict_v3("fail", &["pass", "fail"], false);
    later["schema"] = json!("forge-warden.verdict.v4");
    let (script, _, exit_file) = judge(&scratch, &later);
    let stop = |session: &str, active: bool| -> Value {
        let (_, stdout, _) = hook(
            "stop",
            &["--judge", script.to_str().unwrap()],
            &input("Stop", &root, session, active),
        );
        serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("{e}: {stdout:?}"))
    };
    std::fs::write(&exit_file, "7").unwrap();
    let out = stop("s1", false);
    let reason = blocked(&out);
    assert!(
        reason.contains("exit 7 is the judge's fail, so the stop is held; its verdict could not be read, so which rules failed is not known")
            && reason.contains("its document names the schema forge-warden.verdict.v4, which this build does not read (it reads forge-warden.verdict.v2 and forge-warden.verdict.v3)"),
        "{reason}"
    );
    // the client continues and stops again with nothing changed: the failure stands
    let out = stop("s1", true);
    let message = allowed(&out);
    assert!(
        message.starts_with("deep-diff-forge: FAIL STANDS. "),
        "{message}"
    );
    // under exit 0 or 8 the same document is no verdict and nothing is held
    for (code, session) in [("0", "s2"), ("8", "s3")] {
        std::fs::write(&exit_file, code).unwrap();
        let out = stop(session, false);
        let message = allowed(&out);
        assert!(
            message.starts_with("deep-diff-forge: UNJUDGED. ")
                && message.contains("names the schema forge-warden.verdict.v4"),
            "{code}: {message}"
        );
    }
}

/// S3.21, point (d): `agent doctor --gate` says which verdict schemas this build reads and
/// which one the judge wrote, and a judge that writes one this build does not read is the
/// link `chain`, not ready, with the version its document gives.
#[test]
fn doctor_gate_names_the_verdict_schema_the_judge_writes() {
    let (scratch, root, home, script, _log, exit_file) = gate_fixture("doctor-gate-v3");
    let judge_arg = ["--judge", script.to_str().unwrap()];
    let chain = |doc: &Value| -> (bool, String) {
        let ok = doc["checks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == "chain")
            .unwrap()["ok"]
            .as_bool()
            .unwrap();
        (ok, detail(doc, "chain"))
    };
    let reads =
        "read by this build, which reads forge-warden.verdict.v2 and forge-warden.verdict.v3";
    // the stand-in that writes v2
    let (ok, said) = chain(&doctor_gate(&root, &home, &judge_arg).1);
    assert!(
        ok && said.ends_with(&format!(
            ": forge-warden.verdict.v2 of forge-warden 0.2.0, {reads}"
        )),
        "{said}"
    );
    // one that writes v3, with what v3 adds
    let (bound, _) = judge_v3_bound(&scratch, &verdict_v3("hold", &["pass", "hold"], true));
    assert_eq!(bound, script);
    std::fs::write(&exit_file, "8").unwrap();
    let (ok, said) = chain(&doctor_gate(&root, &home, &judge_arg).1);
    assert!(
        ok && said.ends_with(&format!(
            ": forge-warden.verdict.v3 of forge-warden 0.2.1, {reads}"
        )),
        "{said}"
    );
    // a later one
    let mut later = verdict_v3("hold", &["pass", "hold"], false);
    later["schema"] = json!("forge-warden.verdict.v4");
    later["tool"]["version"] = json!("0.4.0");
    let (again, _, _) = judge(&scratch, &later);
    assert_eq!(again, script);
    std::fs::write(&exit_file, "8").unwrap();
    let (code, doc, _) = doctor_gate(&root, &home, &judge_arg);
    let (ok, said) = chain(&doc);
    assert_eq!((ok, code), (false, 7), "{said}");
    assert!(
        said.contains("its document names the schema forge-warden.verdict.v4, which this build does not read (it reads forge-warden.verdict.v2 and forge-warden.verdict.v3); the document says forge-warden 0.4.0 wrote it"),
        "{said}"
    );
}

// ---- S3.17a part 2 and S3.25.a (0.10.0, lane H): the hooks say only what is true ----

/// The text of every string member of a hook's output, or the output itself when it is not a
/// JSON document (a session start prints plain text).
fn texts_of(stdout: &str) -> Vec<String> {
    fn walk(value: &Value, out: &mut Vec<String>) {
        match value {
            Value::String(text) => out.push(text.clone()),
            Value::Array(items) => items.iter().for_each(|item| walk(item, out)),
            Value::Object(members) => members.values().for_each(|member| walk(member, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    match serde_json::from_str::<Value>(stdout) {
        Ok(document) => walk(&document, &mut out),
        Err(_) => out.push(stdout.to_owned()),
    }
    out
}

/// The advice a `PostToolUse` hook gave the model; panics when it gave none.
fn advice_of(stdout: &str) -> String {
    let out: Value = serde_json::from_str(stdout).unwrap_or_else(|e| panic!("{e}: {stdout:?}"));
    out["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap_or_else(|| panic!("no advice: {stdout}"))
        .to_owned()
}

/// What N47 forbids in text a judged agent reads: a substitute that does not count (any of the
/// vocabulary's, whether a label raises it or not: corrections 2, point B.1), a pointer to the
/// people's `explain signal`, and a command of this program by its bare name rather than by
/// its absolute path.
fn agent_facing_faults(text: &str) -> Vec<String> {
    let mut faults = Vec::new();
    for facts in deep_diff_forge_graph::vocabulary() {
        for not in deep_diff_forge_graph::remedy(facts.signal).not_accepted {
            if text.contains(not.substitute) {
                faults.push(format!(
                    "a substitute of {}: {:?}",
                    facts.label(),
                    not.substitute
                ));
            }
        }
    }
    if text.contains("Not accepted") {
        faults.push("a \"Not accepted\" sentence".to_owned());
    }
    if text.contains("explain signal") {
        faults.push("a pointer to `explain signal`".to_owned());
    }
    let by_path = format!("{} ", this_binary().display());
    for command in ["agent doctor", "agent setup", "explain signal"] {
        for (at, _) in text.match_indices(command) {
            if !text[..at].ends_with(&by_path) {
                faults.push(format!(
                    "the command {command:?} by a bare name: {:?}",
                    &text[text[..at]
                        .char_indices()
                        .rev()
                        .nth(30)
                        .map_or(0, |(i, _)| i)..at + command.len()]
                ));
            }
        }
    }
    faults
}

/// S3.17a.f: "read as" names the rung that found the item, as the ranking gives it. Every item
/// was said to be "read as lexical", also one read across the lines of a hunk.
#[test]
fn advice_says_the_basis_the_ranking_gives() {
    let scratch = Scratch::new("hook-advice-basis");
    let root = repository(&scratch);
    let event = input("PostToolUse", &root, "s1", false);
    std::fs::write(
        root.join("a.rs"),
        "fn a() {}\nfn b() { let _c = std::process::\n    Command::new(\"x\"); }\n",
    )
    .unwrap();
    let (code, stdout, stderr) = hook("post-tool", &[], &event);
    assert_eq!((code, stderr.as_str()), (0, ""), "{stdout}");
    let advice = advice_of(&stdout);
    assert!(
        advice.contains("- \"a.rs\":2: process_spawn@line (")
            && advice.contains("new side, read as hunk); repairs: "),
        "the hunk item is not said as read across the hunk: {advice}"
    );
}

/// S3.17a.f: every command the hooks name is this program by its own absolute path: a bare
/// `deep-diff-forge` runs whatever `PATH` finds when it is typed. The session start names
/// `agent doctor --gate`, and the first stop of a session no start recorded names `agent
/// setup`.
#[test]
fn advice_names_the_binary_by_its_path() {
    let scratch = Scratch::new("hook-advice-binary");
    let root = repository(&scratch);
    let by_path = format!("`{} agent", this_binary().display());
    // no judge: the chain is not ready, and the start says which command checks it
    let started = start(&root, "s1");
    assert!(
        started.contains(&format!("{by_path} doctor --gate` checks every link")),
        "{started}"
    );
    // a stop that is its session's first hook names the command that prints the settings
    let (script, _, _) = judge(&scratch, &verdict("pass", "r", "check_suppressed", "a.rs"));
    let stopped = stop_from(&root, "s2", &script);
    let said = texts_of(&stopped.to_string()).join("\n");
    assert!(
        said.contains(&format!(
            "{by_path} setup --client claude --hooks` prints the settings"
        )),
        "{said}"
    );
    for text in [started, said] {
        assert!(
            !text.contains("`deep-diff-forge "),
            "a bare command: {text}"
        );
    }
}

/// S3.17a.f as corrections 2, point B.1 amends it (gap N-01, N47): what a judged agent reads
/// (the advice after a tool call, the session start, the reason a stop is held, what a stop
/// adds) names the accepted repair only, never a substitute that does not count, never the
/// people's `explain signal`, and no command by a bare name.
#[test]
fn no_agent_facing_text_offers_an_unseen_substitute() {
    let scratch = Scratch::new("hook-agent-facing-text");
    let root = repository(&scratch);
    let mut said: Vec<(String, String)> = Vec::new();
    let mut keep = |what: &str, stdout: &str| {
        for text in texts_of(stdout) {
            said.push((what.to_owned(), text));
        }
    };
    // the start of a session, with no judge and then with one that fails everything
    keep("session start (no judge)", &start(&root, "s1"));
    let (script, _, _) = failing_judge(&scratch);
    let judge_arg = ["--judge", script.to_str().unwrap()];
    let (_, stdout, _) = hook(
        "session-start",
        &judge_arg,
        &input("SessionStart", &root, "s2", false),
    );
    keep("session start (a judge)", &stdout);
    // advice on a change with many labels, each with substitutes in the vocabulary
    std::fs::write(
        root.join("a.rs"),
        "fn a() {}\n#[allow(dead_code)]\nfn b() { unsafe { c() }; d().unwrap(); }\n#[test]\n#[ignore]\nfn t() { assert!(true); }\n",
    )
    .unwrap();
    std::fs::write(root.join("big.rs"), "// x\n".repeat(600)).unwrap();
    let (_, stdout, _) = hook("post-tool", &[], &input("PostToolUse", &root, "s2", false));
    let advice = advice_of(&stdout);
    for label in ["check_suppressed", "unsafe_added", "panic_path_added"] {
        assert!(advice.contains(label), "{label}: {advice}");
    }
    keep("advice", &stdout);
    // a stop held for a failing rule, and the first stop of a session no start recorded
    keep("stop (held)", &stop_from(&root, "s2", &script).to_string());
    keep(
        "stop (first hook)",
        &stop_from(&root, "s3", &script).to_string(),
    );
    assert!(said.len() >= 5, "{said:?}");
    let faults: Vec<String> = said
        .iter()
        .flat_map(|(what, text)| {
            agent_facing_faults(text)
                .into_iter()
                .map(move |fault| format!("{what}: {fault}"))
        })
        .collect();
    assert!(faults.is_empty(), "{faults:#?}");
}

/// S3.17a.i: a signal that reads no line a ranking points at (hidden text, an executable bit,
/// a symbolic link) is named once for its file after a tool call, and not again. The advice
/// read the rows of evidence only, so it never named these.
#[test]
fn advice_names_a_signal_that_has_no_line() {
    let scratch = Scratch::new("hook-advice-no-line");
    let root = repository(&scratch);
    std::fs::write(root.join("run.sh"), "echo hi\n").unwrap();
    git(&root, &["add", "run.sh"]);
    git(&root, &["commit", "--quiet", "-m", "script"]);
    let event = input("PostToolUse", &root, "s1", false);
    // the right-to-left override is assembled here, never written into this source
    let hidden = format!(
        "fn z() {{ let s = \"{}\"; }}\n",
        char::from_u32(0x202E).unwrap()
    );
    std::fs::write(root.join("bidi.rs"), hidden).unwrap();
    std::fs::set_permissions(
        root.join("run.sh"),
        std::os::unix::fs::PermissionsExt::from_mode(0o755),
    )
    .unwrap();
    std::os::unix::fs::symlink("a.rs", root.join("link")).unwrap();
    let (code, stdout, stderr) = hook("post-tool", &[], &event);
    assert_eq!((code, stderr.as_str()), (0, ""), "{stdout}");
    let advice = advice_of(&stdout);
    // the hidden character is in a line, which locates it (F6: the item is the document's)
    for (code, path) in [
        ("hazardous_text@line", "bidi.rs\":1"),
        ("executable_bit@file", "run.sh\""),
        ("symlink_change@file", "link\""),
    ] {
        let item = format!("- \"{path}: {code} (");
        assert_eq!(advice.matches(&item).count(), 1, "{item}: {advice}");
    }
    assert!(
        agent_facing_faults(&advice).is_empty(),
        "{:?}",
        agent_facing_faults(&advice)
    );
    // said once: the next tool call has nothing new to say
    assert_eq!(hook("post-tool", &[], &event).1, "");
}

/// S3.17a.i: when the file's released items fill every row a ranking lists (32), a signal
/// found further on keeps no row; the advice names it for the file. It named rows only, so
/// `unsafe_added` after 33 process spawns was never told.
#[test]
fn advice_names_a_signal_the_row_cap_left_without_a_row() {
    use std::fmt::Write as _;
    let scratch = Scratch::new("hook-advice-row-cap");
    let root = repository(&scratch);
    let event = input("PostToolUse", &root, "s1", false);
    let mut text = String::from("fn f(q: *const u8) {\n");
    for n in 0..33 {
        let _ = writeln!(text, "    let c{n} = std::process::Command::new(\"true\");");
    }
    text.push_str("    let p = unsafe { *q };\n}\n");
    std::fs::write(root.join("spawns.rs"), text).unwrap();
    let (code, stdout, stderr) = hook("post-tool", &[], &event);
    assert_eq!((code, stderr.as_str()), (0, ""), "{stdout}");
    let advice = advice_of(&stdout);
    assert!(
        advice.contains("- \"spawns.rs\": unsafe_added@file ("),
        "the signal the cap left without a row is not told: {advice}"
    );
    assert!(!advice.contains("unsafe_added@line"), "{advice}");
    // the file's located items were cut, and the advice says so as the document does
    assert!(
        advice.contains("- \"spawns.rs\": 32 places listed; the rest were cut at 32.\n"),
        "{advice}"
    );
}

/// One advice item or one diagnostic for an agent, as the two compare (F6.a): path as shown,
/// line, side, code, cases and repair ids.
type Item = (
    String,
    Option<u64>,
    Option<String>,
    String,
    Vec<String>,
    Vec<String>,
);

/// The items of an advice: each line `- "PATH"[:LINE]: CODE (CLASS[, SIDE side], read as
/// BASIS)[; case: …][; …]; repairs: ID, ID.`; a line that counts a file's cut is not an item.
fn advice_items(advice: &str) -> Vec<Item> {
    let mut items = Vec::new();
    for line in advice.lines() {
        let Some(rest) = line.strip_prefix("- \"") else {
            continue;
        };
        if line.contains(" places listed; the rest were cut at 32.") {
            continue;
        }
        let (mut path, mut escaped, mut end) = (String::new(), false, 0);
        for (at, c) in rest.char_indices() {
            match (escaped, c) {
                (false, '\\') => escaped = true,
                (false, '"') => {
                    end = at + 1;
                    break;
                }
                _ => {
                    if escaped && c != '"' && c != '\\' {
                        path.push('\\');
                    }
                    escaped = false;
                    path.push(c);
                }
            }
        }
        let rest = &rest[end..];
        let (number, rest) = match rest.strip_prefix(':').map(|r| r.split_once(": ")) {
            Some(Some((n, r))) if n.bytes().all(|b| b.is_ascii_digit()) => {
                (Some(n.parse().unwrap()), r)
            }
            _ => (None, rest.strip_prefix(": ").unwrap_or(rest)),
        };
        let (code, rest) = rest.split_once(" (").unwrap_or_else(|| panic!("{line}"));
        let (how, rest) = rest.split_once(')').unwrap_or_else(|| panic!("{line}"));
        let side = ["new", "old"]
            .into_iter()
            .find(|s| how.contains(&format!(", {s} side")))
            .map(str::to_owned);
        let list = |words: &str| -> Vec<String> { words.split(", ").map(str::to_owned).collect() };
        let (mut cases, mut repairs) = (Vec::new(), Vec::new());
        for part in rest.trim_end_matches('.').split("; ") {
            if let Some(c) = part.strip_prefix("case: ") {
                cases = list(c);
            } else if let Some(r) = part.strip_prefix("repairs: ") {
                repairs = list(r);
            }
        }
        items.push((path, number, side, code.to_owned(), cases, repairs));
    }
    items.sort();
    items
}

/// The diagnostics for an agent of `--diagnostics --json` of `patch`, with each path as an
/// advice shows it (a right-to-left override escaped).
fn document_items(patch: &[u8]) -> Vec<Item> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .args(["--stdin-patch", "--diagnostics", "--json"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(patch).unwrap();
    let out = child.wait_with_output().unwrap();
    let document: Value = serde_json::from_slice(&out.stdout).unwrap();
    let strings = |v: &Value| -> Vec<String> {
        v.as_array()
            .into_iter()
            .flatten()
            .map(|s| s.as_str().unwrap().to_owned())
            .collect()
    };
    let mut items: Vec<Item> = document["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| {
            d["audience"] == "agent"
                && ["security", "integrity"].contains(&d["class"].as_str().unwrap())
        })
        .map(|d| {
            (
                d["path"].as_str().unwrap().replace('\u{202E}', "\\u{202e}"),
                d["line"].as_u64(),
                d["side"].as_str().map(str::to_owned),
                d["code"].as_str().unwrap().to_owned(),
                strings(&d["cases"]),
                strings(&d["repairs"]),
            )
        })
        .collect();
    items.sort();
    items
}

/// F6.a, the acceptance (N76): what the advice tells an agent after tool calls, item by item,
/// is what `--diagnostics --json` of the same tree gives it for the classes the advice lists
/// (`security` and `integrity`: a `shape` diagnostic orders a person's review), in path, line,
/// side, code, case and repair ids, for each kind of case the document tells apart that the hook
/// can meet: a located row; a label no row locates (an executable bit); `hazardous_text` by a hidden
/// character in a line and by one in a path (each with the character repair alone); a file whose
/// located diagnostics were cut at 32. The hook's patch is made without the repository's
/// attributes, so a file the attributes call binary is read as text and carries no
/// `diff_suppressed`; the engine's `unread` is never met here (the advice's words for a case
/// it cannot tell are tested in `hook.rs`).
#[test]
fn every_advice_item_equals_its_diagnostic() {
    use std::fmt::Write as _;
    let scratch = Scratch::new("hook-advice-equals-diagnostics");
    let root = repository(&scratch);
    std::fs::write(root.join("run.sh"), "echo hi\n").unwrap();
    git(&root, &["add", "run.sh"]);
    git(&root, &["commit", "--quiet", "-m", "script"]);
    let event = input("PostToolUse", &root, "s1", false);
    let rlo = char::from_u32(0x202E).unwrap();
    std::fs::write(
        root.join("a.rs"),
        "fn a() {}\n#[allow(dead_code)]\nfn b() {}\n",
    )
    .unwrap();
    std::fs::set_permissions(
        root.join("run.sh"),
        std::os::unix::fs::PermissionsExt::from_mode(0o755),
    )
    .unwrap();
    std::fs::write(
        root.join("bidi.rs"),
        format!("fn z() {{ let s = \"{rlo}\"; }}\n"),
    )
    .unwrap();
    std::fs::write(root.join(format!("n{rlo}m.rs")), "fn n() {}\n").unwrap();
    std::fs::write(root.join(".gitattributes"), "t.rs -diff\n").unwrap();
    std::fs::write(root.join("t.rs"), "#[allow(unused)]\nfn t() {}\n").unwrap();
    let mut spawns = String::from("fn f(q: *const u8) {\n");
    for n in 0..33 {
        let _ = writeln!(
            spawns,
            "    let c{n} = std::process::Command::new(\"true\");"
        );
    }
    spawns.push_str("    let p = unsafe { *q };\n}\n");
    std::fs::write(root.join("spawns.rs"), spawns).unwrap();
    // every item, over as many tool calls as the advice's caps take
    let (mut said, mut advices) = (Vec::new(), Vec::new());
    loop {
        let (code, stdout, stderr) = hook("post-tool", &[], &event);
        assert_eq!((code, stderr.as_str()), (0, ""), "{stdout}");
        if stdout.is_empty() {
            break;
        }
        let advice = advice_of(&stdout);
        said.extend(advice_items(&advice));
        advices.push(advice);
        assert!(advices.len() < 10, "{advices:#?}");
    }
    said.sort();
    let all = advices.concat();
    // the same tree as the hook's patch: every file, without the repository's attributes
    git(&root, &["add", "--all"]);
    let empty = git(&root, &["hash-object", "-t", "tree", "/dev/null"]);
    let patch = Command::new("git")
        .current_dir(&root)
        .envs(ISOLATE)
        .args([
            &format!("--attr-source={}", empty.trim()),
            "-c",
            "core.attributesFile=/dev/null",
            "-c",
            "core.quotePath=true",
            "diff",
            "--cached",
            "--binary",
            "--no-renames",
            "--full-index",
            "HEAD",
        ])
        .output()
        .unwrap()
        .stdout;
    assert_eq!(said, document_items(&patch), "{all}");
    for (what, wanted) in [
        ("a located row", "- \"a.rs\":2: check_suppressed@line ("),
        ("no row", "- \"run.sh\": executable_bit@file ("),
        (
            "a hidden character in a line",
            "- \"bidi.rs\":1: hazardous_text@line (security, new side, read as lexical); case: bidi_control; repairs: hazardous_text:a1.\n",
        ),
        (
            "a hidden character in a path",
            "- \"n\\u{202e}m.rs\": hazardous_text@file (security, read as path or lexical); case: bidi_control; repairs: hazardous_text:a1.\n",
        ),
        (
            "a cut file",
            "- \"spawns.rs\": 32 places listed; the rest were cut at 32.\n",
        ),
        (
            "attributes ignored",
            "- \"t.rs\":1: check_suppressed@line (",
        ),
    ] {
        assert!(all.contains(wanted), "{what}: {all}");
    }
    assert!(!all.contains("diff_suppressed"), "{all}");
    assert!(
        agent_facing_faults(&all).is_empty(),
        "{:?}",
        agent_facing_faults(&all)
    );
}

/// F6.a (N-11): a file whose name this version cannot read (not UTF-8) is one item that names
/// it as Git quoted it, and the other files of the change are still advised. The whole advice
/// was off for the session.
///
/// Twice: the name in Git's objects alone (committed through the index and taken out of it
/// again, a deletion no file system is asked about), and the name on disk, untracked. macOS's
/// APFS refuses a name that is not UTF-8 (EILSEQ, 92): there the first case stands and the
/// second says it is skipped.
#[test]
fn a_refused_path_does_not_silence_the_others() {
    use std::os::unix::ffi::OsStrExt as _;
    let name = std::ffi::OsStr::from_bytes(b"n\xffm.rs");
    for on_disk in [false, true] {
        let scratch = Scratch::new("hook-advice-refused-path");
        let root = repository(&scratch);
        let event = input("PostToolUse", &root, "s1", false);
        if on_disk {
            if let Err(e) = std::fs::write(root.join(name), "fn n() {}\n") {
                assert!(
                    cfg!(target_os = "macos") && e.raw_os_error() == Some(92),
                    "{e}"
                );
                println!(
                    "skipped: the name on disk: the file system refuses a name that is not UTF-8 ({e})"
                );
                return;
            }
        } else {
            std::fs::write(scratch.join("n"), "fn n() {}\n").unwrap();
            let blob = git(&root, &["hash-object", "-w", "../n"]);
            let by_name = |args: &[&str]| {
                let out = Command::new("git")
                    .current_dir(&root)
                    .envs(ISOLATE)
                    .args(args)
                    .arg(name)
                    .output()
                    .unwrap();
                assert!(out.status.success(), "git {args:?}: {out:?}");
            };
            by_name(&[
                "update-index",
                "--add",
                "--cacheinfo",
                "100644",
                blob.trim(),
            ]);
            let tree = git(&root, &["write-tree"]);
            let commit = git(
                &root,
                &["commit-tree", tree.trim(), "-p", "HEAD", "-m", "n"],
            );
            git(&root, &["update-ref", "HEAD", commit.trim()]);
            by_name(&["update-index", "--force-remove", "--"]);
        }
        std::fs::write(
            root.join("a.rs"),
            "fn a() {}\n#[allow(dead_code)]\nfn b() {}\n",
        )
        .unwrap();
        let (code, stdout, stderr) = hook("post-tool", &[], &event);
        assert_eq!((code, stderr.as_str()), (0, ""), "{stdout}");
        let advice = advice_of(&stdout);
        assert!(
            advice.contains("- \"\\\"a/n\\377m.rs\\\" \\\"b/n\\377m.rs\\\"\": this version could not read this file's change (its names as Git quoted them); nothing in it is advised.\n"),
            "on disk: {on_disk}: {advice}"
        );
        assert!(
            advice.contains("- \"a.rs\":2: check_suppressed@line ("),
            "the other file is not advised (on disk: {on_disk}): {advice}"
        );
        assert!(!advice.contains("did not run"), "{advice}");
        // said once
        assert_eq!(hook("post-tool", &[], &event).1, "");
    }
}

/// The words that begin a guard notice for `path`, as an advice says them.
fn guard_notice(path: &str) -> String {
    format!(
        "deep-diff-forge: \"{path}\" is a file that decides how this work is judged or checked: "
    )
}

/// F6.b: an edit of a file that decides how the work is judged or checked (the policy file,
/// `.gitattributes`, a CI workflow, the client's hook settings) is named once after a tool
/// call, with what the file decides, and again when its content changes again: a loosening
/// after a harmless first edit is said. It is information: nothing is blocked, nothing is
/// called forbidden. The post-tool hook knows the policy's name by `--policy`.
#[test]
fn each_guard_file_is_named_once_and_again_when_its_content_changes() {
    let scratch = Scratch::new("hook-guard-files");
    let root = repository(&scratch);
    let event = input("PostToolUse", &root, "s1", false);
    std::fs::create_dir_all(root.join(".github/workflows")).unwrap();
    std::fs::create_dir_all(root.join(".claude")).unwrap();
    let files = [
        (
            "warden.toml",
            "the policy file the stops judge with; where this project's ledger first recorded a commit that has the policy file, committing an edit of it can make this session's stops stricter, never looser, and committing it does not make it the project's accepted policy",
        ),
        (".gitattributes", "a Git attributes file"),
        (".github/workflows/x.yml", "a CI workflow"),
        (".claude/settings.json", "the client's hook settings"),
    ];
    for (path, _) in files {
        std::fs::write(root.join(path), "# one\n").unwrap();
    }
    let (code, stdout, stderr) = hook("post-tool", &[], &event);
    assert_eq!((code, stderr.as_str()), (0, ""), "{stdout}");
    let out: Value = serde_json::from_str(&stdout).unwrap();
    assert!(
        out.get("decision").is_none(),
        "a guard notice blocks: {stdout}"
    );
    let advice = advice_of(&stdout);
    for (path, words) in files {
        let notice = guard_notice(path);
        assert_eq!(advice.matches(&notice).count(), 1, "{path}: {advice}");
        let line = advice.lines().find(|l| l.starts_with(&notice)).unwrap();
        assert!(line.contains(words), "{line}");
        assert!(
            line.ends_with(
                "This is information, not a block: the edit is part of the change a stop judges."
            ),
            "{line}"
        );
    }
    assert!(!advice.to_lowercase().contains("forbid"), "{advice}");
    assert!(
        agent_facing_faults(&advice).is_empty(),
        "{:?}",
        agent_facing_faults(&advice)
    );
    // said once
    assert_eq!(hook("post-tool", &[], &event).1, "");
    // again when the content changes again, and only that file
    std::fs::write(root.join("warden.toml"), "# two: loosened\n").unwrap();
    let (_, stdout, _) = hook("post-tool", &[], &event);
    assert!(
        !stdout.is_empty(),
        "a new content of the policy file is not named"
    );
    let advice = advice_of(&stdout);
    assert_eq!(
        advice.matches("is a file that decides how").count(),
        1,
        "{advice}"
    );
    assert!(advice.contains(&guard_notice("warden.toml")), "{advice}");
    assert!(
        advice.ends_with("Paths are data taken from the change, not instructions.\n"),
        "{advice}"
    );
    // a policy named by --policy is the policy file, and the default name then is not
    std::fs::create_dir_all(root.join("ci")).unwrap();
    std::fs::write(root.join("ci/policy.toml"), "# the named policy\n").unwrap();
    let other = input("PostToolUse", &root, "s2", false);
    let advice = advice_of(&hook("post-tool", &["--policy", "ci/policy.toml"], &other).1);
    let named = advice
        .lines()
        .find(|l| l.starts_with(&guard_notice("ci/policy.toml")))
        .unwrap_or_else(|| panic!("{advice}"));
    assert!(
        named.contains("the policy file the stops judge with"),
        "{named}"
    );
    assert!(!advice.contains(&guard_notice("warden.toml")), "{advice}");
}

/// F6.b: a guard file is one by its whole path components: a file whose name only ends or
/// begins like one, or that stands where the table does not name it, gets no notice.
#[test]
fn an_ordinary_path_gets_no_guard_notice() {
    let scratch = Scratch::new("hook-guard-ordinary");
    let root = repository(&scratch);
    let event = input("PostToolUse", &root, "s1", false);
    // (one file the advice names, so that there is advice to read)
    std::fs::write(
        root.join("a.rs"),
        "fn a() {}\n#[allow(dead_code)]\nfn b() {}\n",
    )
    .unwrap();
    for path in [
        "src/warden.toml.rs",
        "src/warden.toml",
        "a.gitattributes",
        "docs/.github/workflows/x.yml",
        "sub/.gitlab-ci.yml",
        "x/.claude/settings.json",
    ] {
        let file = root.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, "fn x() {}\n").unwrap();
    }
    let (code, stdout, stderr) = hook("post-tool", &[], &event);
    assert_eq!((code, stderr.as_str()), (0, ""), "{stdout}");
    let advice = advice_of(&stdout);
    assert!(advice.contains("check_suppressed@line"), "{advice}");
    assert!(!advice.contains("is a file that decides how"), "{advice}");
}

/// F6, closing round, ruling 1: a diagnostic of class `shape` (a new file, a lockfile, a public
/// API path, a large change) orders a person's review and is not advised, nor counted: a change
/// that raises only such labels gets no advice after a tool call. Advice saying there was nothing
/// to repair after every tool call that added a file taught an agent to skip the advice.
#[test]
fn a_change_that_only_orders_the_review_gets_no_advice() {
    let scratch = Scratch::new("hook-advice-shape-only");
    let root = repository(&scratch);
    let event = input("PostToolUse", &root, "s1", false);
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/lib.rs"), "pub fn n() {}\n").unwrap();
    std::fs::write(root.join("Cargo.lock"), "version = 4\n").unwrap();
    std::fs::remove_file(root.join("a.rs")).unwrap();
    let rank = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .current_dir(&root)
        .envs(ISOLATE)
        .args([
            "--git-working-tree",
            "--include-untracked",
            "--rank",
            "--json",
        ])
        .output()
        .unwrap();
    let ranked = String::from_utf8_lossy(&rank.stdout);
    for label in ["new_file", "config_or_lockfile", "deleted_file"] {
        assert!(
            ranked.contains(&format!("\"{label}\"")),
            "{label}: {ranked}"
        );
    }
    assert_eq!(
        hook("post-tool", &[], &event),
        (0, String::new(), String::new())
    );
    // a finding of another class beside them is advised, and counted alone
    std::fs::write(
        root.join("src/lib.rs"),
        "#[allow(dead_code)]\npub fn n() {}\n",
    )
    .unwrap();
    let advice = advice_of(&hook("post-tool", &[], &event).1);
    assert!(
        advice.starts_with("deep-diff-forge: 1 new finding in "),
        "{advice}"
    );
    assert!(!advice.contains("@file (shape"), "{advice}");
    assert!(!advice.contains("Nothing to repair"), "{advice}");
}

/// F6, closing round, ruling 2 (N75): guard notices are said only by an advice that ranks the
/// change. While the change cannot be made (here a skip-worktree mark), an edit of the policy
/// file is not named: the advice says once why it is off, and a stop is UNJUDGED with the
/// reason, as for any change it cannot make. The first call that ranks names it.
#[test]
fn a_guard_notice_waits_for_a_call_that_ranks() {
    let scratch = Scratch::new("hook-guard-waits");
    let root = repository(&scratch);
    let (script, _, _) = failing_judge(&scratch);
    let event = input("PostToolUse", &root, "s1", false);
    std::fs::write(root.join("warden.toml"), "# loosened\n").unwrap();
    git(&root, &["update-index", "--skip-worktree", "a.rs"]);
    let (code, stdout, _) = hook("post-tool", &[], &event);
    assert_eq!(code, 0);
    assert!(
        system_message(&stdout).contains("skip-worktree"),
        "{stdout}"
    );
    assert!(!stdout.contains("is a file that decides how"), "{stdout}");
    assert_eq!(hook("post-tool", &[], &event).1, "");
    let held = stop_from(&root, "s1", &script);
    assert!(blocked(&held).contains("skip-worktree"), "{held}");
    let decision = stop_from(&root, "s1", &script);
    let said = allowed(&decision);
    assert!(
        said.starts_with("deep-diff-forge: UNJUDGED. ") && said.contains("skip-worktree"),
        "{said}"
    );
    git(&root, &["update-index", "--no-skip-worktree", "a.rs"]);
    let advice = advice_of(&hook("post-tool", &[], &event).1);
    assert!(advice.contains(&guard_notice("warden.toml")), "{advice}");
}

/// F6, closing round, ruling 2 (N75): when the marks' key file under the repository's Git
/// directory is no key (a link, or a file of another length) and no state can remember a
/// reason, the advice hook says nothing and writes no mark; no byte of the key file is said;
/// a stop says the reason as it does with a sound key.
#[test]
fn a_damaged_marks_key_silences_the_advice_and_not_the_stop() {
    let scratch = Scratch::new("hook-marks-key-damaged");
    let root = repository(&scratch);
    silence_a_lint(&root);
    let own = root.join(".git/deep-diff-forge");
    let sessions = sessions();
    std::fs::create_dir_all(&sessions).unwrap();
    let elsewhere = scratch.join("elsewhere");
    std::fs::create_dir(&elsewhere).unwrap();
    let kept = scratch.join("sessions-kept");
    std::fs::rename(&sessions, &kept).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &sessions).unwrap();
    let stop = |session: &str| hook("stop", &[], &input("Stop", &root, session, false));
    // with a sound key: said once beside the state; the stop's answer
    let sound = hook("post-tool", &[], &input("PostToolUse", &root, "a", false));
    assert!(
        system_message(&sound.1).contains("is not a directory of its own"),
        "{sound:?}"
    );
    let stop_sound = stop("a");
    assert!(
        stop_sound.1.contains("is not a directory of its own"),
        "{stop_sound:?}"
    );
    let marks = || {
        std::fs::read_dir(&own)
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with("said-")
            })
            .count()
    };
    assert_eq!(marks(), 1);
    // the key bytes, assembled here: 32 letters no output holds by chance
    let bytes: Vec<u8> = (0..32_u8).map(|n| b'Q' + n % 9).collect();
    let target = scratch.join("a-key");
    std::fs::write(&target, &bytes).unwrap();
    for (case, place) in [("a link", true), ("too short", false)] {
        std::fs::remove_file(own.join("key")).unwrap();
        if place {
            std::os::unix::fs::symlink(&target, own.join("key")).unwrap();
        } else {
            std::fs::write(own.join("key"), &bytes[..5]).unwrap();
        }
        let session = format!("b-{place}");
        let advice = hook(
            "post-tool",
            &[],
            &input("PostToolUse", &root, &session, false),
        );
        assert_eq!(advice, (0, String::new(), String::new()), "{case}");
        assert_eq!(marks(), 1, "{case}: a mark was written");
        let answer = stop(&session);
        assert_eq!(answer, stop_sound, "{case}: the stop's answer changed");
        let key_text = String::from_utf8_lossy(&bytes).into_owned();
        let key_hex = bytes.iter().fold(String::new(), |mut hex, b| {
            use std::fmt::Write as _;
            let _ = write!(hex, "{b:02x}");
            hex
        });
        for out in [&advice.1, &advice.2, &answer.1, &answer.2] {
            assert!(
                !out.contains(&key_text) && !out.contains(&key_hex),
                "{case}: {out}"
            );
            assert!(!out.contains(&key_text[..5]), "{case}: {out}");
        }
    }
    std::fs::remove_file(&sessions).unwrap();
    std::fs::rename(&kept, &sessions).unwrap();
}

/// F13.a, narrowed (design B8, probes p9 and c7): a commit made in another linked working tree
/// of the repository is judged by every session that finds it and holds each once; the block of
/// the session that did not make it names the tree whose `HEAD` reflog records it, as advice.
/// After `git reflog expire` both sessions still hold once: attribution never removes a hold.
#[test]
fn another_worktrees_commit_is_named_and_still_holds_once() {
    let scratch = Scratch::new("hook-other-tree");
    let root = repository(&scratch);
    let (script, _) = reading_judge(&scratch);
    let other = scratch.join("other");
    git(
        &root,
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "side",
            other.to_str().unwrap(),
        ],
    );
    start(&root, "a");
    start(&other, "b");
    secret(&other, "k.rs");
    let first = commit_all(&other, "the other tree's work");
    let reason = blocked(&stop_from(&root, "a", &script)).to_owned();
    assert!(
        reason.contains(&format!("Commit {first} (the HEAD reflog of "))
            && reason.contains(
                " has this commit; if this session did not make it, tell a person): FAIL\n"
            ),
        "{reason}"
    );
    let theirs = blocked(&stop_from(&other, "b", &script)).to_owned();
    assert!(
        theirs.contains(&format!("Commit {first}: FAIL\n")),
        "{theirs}"
    );
    // the reflogs expired: both sessions still hold once
    secret(&other, "l.rs");
    let second = commit_all(&other, "more of it");
    git(&root, &["reflog", "expire", "--expire=now", "--all"]);
    let reason = blocked(&stop_from(&root, "a", &script)).to_owned();
    assert!(reason.contains(&format!("Commit {second}")), "{reason}");
    let theirs = blocked(&stop_from(&other, "b", &script)).to_owned();
    assert!(
        theirs.contains(&format!("Commit {second}: FAIL\n")),
        "{theirs}"
    );
}

/// The SHA-256 of every file under `dir` with its path, sorted: what a stop that writes nothing
/// there leaves as it found it.
fn tree_digest(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    let mut walk = vec![dir.to_path_buf()];
    while let Some(at) = walk.pop() {
        for entry in std::fs::read_dir(&at).unwrap() {
            let path = entry.unwrap().path();
            let kind = std::fs::symlink_metadata(&path).unwrap().file_type();
            if kind.is_dir() {
                walk.push(path);
            } else if kind.is_symlink() {
                out.push((
                    path.clone(),
                    std::fs::read_link(&path)
                        .unwrap()
                        .into_os_string()
                        .into_encoded_bytes(),
                ));
            } else {
                out.push((path.clone(), std::fs::read(&path).unwrap_or_default()));
            }
        }
    }
    out.sort();
    out
}

/// The words that head a stop whose session state could not be used, for the class `class`.
fn stateless(class: &str) -> String {
    format!(
        "deep-diff-forge: this session's state could not be used ({class}): the working tree was judged without it; "
    )
}

/// F12.d, the stateless stop (owner decision OD-J; design B6, probe c1), and ruling 3's
/// sequence: a state that cannot be read no longer makes every stop UNJUDGED. The stop judges
/// the working tree without the state, reads and writes nothing of the state directory, and
/// blocks on a failing tree; the stop that follows a block lets the failure stand, so the
/// session is not trapped; the next turn's stop blocks again, since nothing remembers that it
/// blocked. The state is kept byte for byte, and no text of it is said.
#[test]
fn a_damaged_state_still_judges_the_tree_once() {
    let scratch = Scratch::new("hook-stateless-damaged");
    let root = repository(&scratch);
    let (script, log, _) = failing_judge(&scratch);
    start(&root, "s");
    // a commit of the session loosens the policy: the tree is judged under the policy at the
    // commit the project's ledger names as its baseline, not at HEAD alone (release 0.11.1,
    // T11: under the policy at HEAD as well, the stricter answer standing)
    std::fs::write(root.join("warden.toml"), "# loosened by the session\n").unwrap();
    commit_all(&root, "a looser policy");
    silence_a_lint(&root);
    let state = states_in(&sessions()).remove(0);
    std::fs::write(&state, "{\"forged\": \"TEXT-OF-THE-DAMAGED-STATE\"}").unwrap();
    let before = tree_digest(&sessions());
    let first = blocked(&stop_from(&root, "s", &script)).to_owned();
    assert!(first.starts_with(&stateless("state.damaged")), "{first}");
    assert!(first.contains("check_suppressed in \"a.rs\""), "{first}");
    assert!(
        first.contains("a failing tree blocks once in each turn"),
        "{first}"
    );
    assert!(!first.contains("TEXT-OF-THE-DAMAGED-STATE"), "{first}");
    // the tree, and the session's commit, which the ledger has no verdict for: each under the
    // accepted policy, as 0.11.0 judged them, and under the policy at HEAD, whose file differs
    // (release 0.11.1, ruling 6: two runs each, four in all)
    assert_eq!(runs(&log), 4);
    let seen = std::fs::read_to_string(&log).unwrap();
    assert_eq!(
        (
            seen.matches("POLICY # the policy at the base commit\n")
                .count(),
            seen.matches("POLICY # loosened by the session\n").count()
        ),
        (2, 2),
        "{seen}"
    );
    // the stop that follows the block: the failure stands
    let (code, stdout, _) = hook(
        "stop",
        &["--judge", script.to_str().unwrap()],
        &input("Stop", &root, "s", true),
    );
    assert_eq!(code, 0);
    let stands = allowed(&serde_json::from_str(&stdout).unwrap()).to_owned();
    assert!(
        stands.starts_with(&stateless("state.damaged"))
            && stands.contains("\ndeep-diff-forge: FAIL STANDS."),
        "{stands}"
    );
    // the next turn: it blocks again
    let again = blocked(&stop_from(&root, "s", &script)).to_owned();
    assert!(again.starts_with(&stateless("state.damaged")), "{again}");
    assert_eq!(
        tree_digest(&sessions()),
        before,
        "the state directory was written"
    );
    // the ledger records what was judged, and that the commits were not
    let ledger = std::fs::read_to_string(root.join(".git/deep-diff-forge/ledger.jsonl")).unwrap();
    assert!(
        ledger.contains("\"unit\":\"stop\",\"outcome\":\"unjudged\",\"class\":\"state.damaged\"")
            || ledger.contains("\"class\":\"state.damaged\""),
        "{ledger}"
    );
}

/// F12.d (design B6, probe c4b): a stop that finds the session's lock held by another hook
/// judges the working tree without the state, and writes none.
#[test]
fn a_held_session_lock_still_judges_the_tree_once() {
    let scratch = Scratch::new("hook-stateless-lock");
    let root = repository(&scratch);
    let (script, log, _) = failing_judge(&scratch);
    start(&root, "s");
    silence_a_lint(&root);
    let state = states_in(&sessions()).remove(0);
    let lock = state.with_extension("lock");
    let go = scratch.join("go");
    let mut holder = hold_lock(&lock, &go);
    let before = tree_digest(&sessions());
    let said = blocked(&stop_from(&root, "s", &script)).to_owned();
    std::fs::remove_file(&go).unwrap();
    assert!(holder.wait().unwrap().success());
    assert!(said.starts_with(&stateless("state.lock_busy")), "{said}");
    assert!(said.contains("check_suppressed in \"a.rs\""), "{said}");
    assert_eq!(runs(&log), 1);
    assert_eq!(
        tree_digest(&sessions()),
        before,
        "the state directory was written"
    );
}

/// F12.d, one case per class of a state that cannot be used and per failure of the key file
/// (lane H3): each stop judges the tree, says the class first, writes nothing of the state
/// directory, and says no byte of the key file or text of the state.
#[test]
fn every_state_that_cannot_be_used_still_judges_the_tree() {
    use std::os::unix::fs::PermissionsExt as _;
    type Spoil = Box<dyn Fn(&Path, &Path)>;
    let key_bytes: Vec<u8> = (0..32_u8).map(|n| b'Q' + n % 9).collect();
    let key_text = String::from_utf8_lossy(&key_bytes).into_owned();
    let cases: Vec<(&str, &str, Spoil)> = vec![
        (
            "damaged",
            "state.damaged",
            Box::new(|state: &Path, _: &Path| std::fs::write(state, "not json").unwrap()),
        ),
        (
            "newer",
            "state.newer",
            Box::new(|state: &Path, _: &Path| {
                let mut value: Value =
                    serde_json::from_str(&std::fs::read_to_string(state).unwrap()).unwrap();
                value["schema"] = json!("deep-diff-forge.hook-state.v999999");
                std::fs::write(state, value.to_string()).unwrap();
            }),
        ),
        (
            "older",
            "state.older",
            Box::new(|state: &Path, _: &Path| {
                let mut value: Value =
                    serde_json::from_str(&std::fs::read_to_string(state).unwrap()).unwrap();
                value["schema"] = json!("deep-diff-forge.hook-state.v9");
                std::fs::write(state, value.to_string()).unwrap();
            }),
        ),
        (
            "unreadable",
            "state.damaged",
            Box::new(|state: &Path, _: &Path| {
                std::fs::set_permissions(state, std::fs::Permissions::from_mode(0o000)).unwrap();
            }),
        ),
        (
            "lists",
            "state.lists_damaged",
            Box::new(|state: &Path, _: &Path| {
                std::fs::write(state.with_extension("tips"), "not a list\n").unwrap();
            }),
        ),
        ("key a link", "state.damaged", {
            let bytes = key_bytes.clone();
            Box::new(move |_: &Path, dir: &Path| {
                let target = dir.parent().unwrap().join("a-key");
                std::fs::write(&target, &bytes).unwrap();
                std::fs::remove_file(dir.join("key")).unwrap();
                std::os::unix::fs::symlink(&target, dir.join("key")).unwrap();
            })
        }),
        ("key too short", "state.damaged", {
            let bytes = key_bytes.clone();
            Box::new(move |_: &Path, dir: &Path| {
                std::fs::write(dir.join("key"), &bytes[..5]).unwrap();
            })
        }),
    ];
    for (case, class, spoil) in cases {
        let scratch = Scratch::new("hook-stateless-class");
        let root = repository(&scratch);
        let (script, log, _) = failing_judge(&scratch);
        start(&root, "s");
        // (a commit gives the session a list of judged commits to spoil)
        silence_a_lint(&root);
        let state = states_in(&sessions()).remove(0);
        spoil(&state, &sessions());
        if case == "unreadable" && std::fs::read(&state).is_ok() {
            continue; // (run as root: nothing is unreadable)
        }
        start_says_what_stops_do(&root, case, class);
        let before = tree_digest_lenient(&sessions());
        let said = blocked(&stop_from(&root, "s", &script)).to_owned();
        assert!(said.starts_with(&stateless(class)), "{case}: {said}");
        // ... and the block says nothing a stop with no state cannot keep
        assert!(
            !said.contains("does not block again")
                && said.contains("with no state to remember it, a working tree that still fails or is held blocks again at the next turn's stop"),
            "{case}: {said}"
        );
        assert!(
            said.contains("check_suppressed in \"a.rs\""),
            "{case}: {said}"
        );
        assert!(
            !said.contains(&key_text) && !said.contains(&key_text[..5]),
            "{case}: {said}"
        );
        assert!(runs(&log) >= 1, "{case}");
        assert_eq!(
            tree_digest_lenient(&sessions()),
            before,
            "{case}: the state directory was written"
        );
        let _ = std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o600));
    }
}

/// C2 (the 0.11.0 tree review): the start of a session whose state, or whose lists of
/// commits, cannot be used (`class`) says what its stops then do, never that they will not be
/// judged; advice stops only when the state itself cannot be used.
fn start_says_what_stops_do(root: &Path, case: &str, class: &str) {
    let started = start(root, "s");
    let what = if class == "state.lists_damaged" {
        "deep-diff-forge: this session's lists of commits cannot be used, so each stop judges the working tree without the session's memory"
    } else {
        "deep-diff-forge: this session's state cannot be used, so no advice will be given and each stop judges the working tree without the session's memory"
    };
    assert!(
        started.starts_with(what) && !started.contains("will not be judged"),
        "{case}: {started}"
    );
}

/// [`tree_digest`] where a file may be unreadable: its bytes count as none.
fn tree_digest_lenient(dir: &Path) -> Vec<(PathBuf, Vec<u8>, u64)> {
    let mut out = Vec::new();
    let mut walk = vec![dir.to_path_buf()];
    while let Some(at) = walk.pop() {
        for entry in std::fs::read_dir(&at).unwrap() {
            let path = entry.unwrap().path();
            let found = std::fs::symlink_metadata(&path).unwrap();
            if found.is_dir() {
                walk.push(path);
            } else {
                let bytes = std::fs::read(&path).unwrap_or_default();
                out.push((path, bytes, found.len()));
            }
        }
    }
    out.sort();
    out
}

/// Closing round, ruling 3: with the project's ledger, a stop whose state cannot be used judges
/// the frontier's commits too (the session's own among them, which it cannot tell apart, so none
/// is marked), records each in the ledger, and blocks on a failing one; the ledger keeps its
/// verdict, so the next stops do not judge it again.
#[test]
fn a_damaged_state_after_a_failing_commit_still_judges_the_commit() {
    let scratch = Scratch::new("hook-stateless-commit");
    let root = repository(&scratch);
    let (script, log) = reading_judge(&scratch);
    start(&root, "s");
    secret(&root, "k.rs");
    let key = commit_all(&root, "a key");
    let state = states_in(&sessions()).remove(0);
    std::fs::write(&state, "{}").unwrap();
    let before = tree_digest(&sessions());
    let first = blocked(&stop_from(&root, "s", &script)).to_owned();
    assert!(
        first.starts_with(&stateless("state.damaged"))
            && first.contains("so were the commits the project's ledger has no verdict for")
            && first.contains(&format!("Commit {key}: FAIL\n")),
        "{first}"
    );
    assert!(!first.contains(NOT_OURS), "{first}");
    let judged = runs(&log);
    for _ in 0..2 {
        let said = allowed(&stop_from(&root, "s", &script)).to_owned();
        assert!(said.starts_with(&stateless("state.damaged")), "{said}");
    }
    assert_eq!(runs(&log), judged, "the commit was judged again");
    assert_eq!(
        tree_digest(&sessions()),
        before,
        "the state directory was written"
    );
    let ledger = std::fs::read_to_string(root.join(".git/deep-diff-forge/ledger.jsonl")).unwrap();
    let full = git(&root, &["rev-parse", "HEAD"]);
    assert!(
        ledger.lines().any(
            |line| line.contains(&format!("\"unit\":\"commit:{}\"", full.trim()))
                && line.contains("\"outcome\":\"fail\"")
        ),
        "{ledger}"
    );
}

/// Closing round, ruling 4: a session's start that writes the project's baseline says so: the
/// project is recorded from this commit, and commits before it are not judged by the hooks.
/// Removing the ledger between sessions is then visible at the next start.
#[test]
fn a_start_that_records_the_project_says_so() {
    const FIRST: &str = "- This start is the first record of this project in its ledger: the project is recorded from commit ";
    let scratch = Scratch::new("hook-first-record");
    let root = repository(&scratch);
    let said = start(&root, "a");
    assert!(said.contains(FIRST), "{said}");
    assert!(
        said.contains(
            "commits that were in the repository before it are not judged by these hooks"
        ),
        "{said}"
    );
    assert!(!start(&root, "b").contains(FIRST));
    silence_a_lint(&root);
    commit_all(&root, "work of a session that ended");
    std::fs::remove_file(root.join(".git/deep-diff-forge/ledger.jsonl")).unwrap();
    let head = short(&root, "HEAD");
    let said = start(&root, "c");
    assert!(said.contains(&format!("{FIRST}{}", &head[..7])), "{said}");
}

/// Makes the stand-in judge whose log is `log` take bound mode: its `check --help` names
/// `--repo`, as forge-warden's does from its 0.3.0 line (the released 0.2.1 does not).
fn takes_bound(log: &Path) {
    std::fs::write(
        log.with_file_name("judge.help"),
        "      --repo <DIR>     Bound mode (ADR 0012, authority mode only)\n",
    )
    .unwrap();
}

/// The argument lines of the stand-in judge's runs, in order.
fn args_of(log: &Path) -> Vec<String> {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| line.strip_prefix("ARGS "))
        .map(str::to_owned)
        .collect()
}

/// The bound-mode arguments a run for the commit `id` with its parent `parent` carries.
fn bound_args(root: &Path, parent: &str, id: &str) -> String {
    let common = std::fs::canonicalize(root.join(".git")).unwrap();
    format!("--repo {} --base {parent} --head {id}", common.display())
}

/// S3.25.b, the acceptance: a commit with one parent is judged in bound mode (`--repo` with
/// the repository's Git directory, `--base` its parent, `--head` the commit), so that a policy
/// that requires verified claims can pass it; the uncommitted change is judged as before. A
/// verdict whose claims name another pair is no verified verdict on the commit (UNJUDGED); a
/// judge that refuses bound mode for a commit has it judged as before, and that is said.
#[test]
fn a_commit_is_judged_in_bound_mode_and_its_claims_name_it() {
    let scratch = Scratch::new("hook-bound-mode");
    let root = repository(&scratch);
    let (script, log) = reading_judge(&scratch);
    takes_bound(&log);
    start(&root, "s");
    std::fs::write(root.join("b.rs"), "fn b() {}\n").unwrap();
    commit_all(&root, "b");
    let (parent, id) = (
        git(&root, &["rev-parse", "HEAD~1"]).trim().to_owned(),
        git(&root, &["rev-parse", "HEAD"]).trim().to_owned(),
    );
    std::fs::write(root.join("u.rs"), "fn u() {}\n").unwrap();
    assert_eq!(stop_from(&root, "s", &script), Value::Null);
    let args = args_of(&log);
    assert_eq!(args.len(), 2, "{args:?}");
    assert!(
        !args[0].contains("--repo"),
        "the uncommitted change in bound mode: {args:?}"
    );
    assert!(
        args[1].contains(&bound_args(&root, &parent, &id)),
        "the commit is not judged in bound mode: {args:?}"
    );
    // a verdict that claims another pair is no verified verdict on this commit
    std::fs::write(log.with_file_name("judge.help.liar"), "").unwrap();
    std::fs::write(root.join("c.rs"), "fn c() {}\n").unwrap();
    let liar = commit_all(&root, "c");
    let said = allowed(&stop_from(&root, "s", &script)).to_owned();
    assert!(
        said.contains(&format!(
            "{liar} (no verified verdict on this commit: the judge was asked about commit {liar} \
             against its parent"
        )),
        "{said}"
    );
    // a judge that refuses bound mode for a commit: judged without --repo, and that is said
    std::fs::remove_file(log.with_file_name("judge.help.liar")).unwrap();
    std::fs::write(log.with_file_name("judge.help.refuse"), "").unwrap();
    secret(&root, "k.rs");
    let key = commit_all(&root, "a key");
    let reason = blocked(&stop_from(&root, "s", &script)).to_owned();
    assert!(
        reason.contains(&format!("Commit {key}: FAIL\n"))
            && reason.contains("the judge refused bound mode for this commit (exit 4"),
        "{reason}"
    );
    let args = args_of(&log);
    let last = &args[args.len() - 2..];
    assert!(
        last[0].contains("--repo") && !last[1].contains("--repo"),
        "{last:?}"
    );
}

/// S3.25.b, ruling 1: a judge whose usage (`check --help`) names no `--repo`, as the released
/// forge-warden 0.2.1's, is started as before: no bound argument for a commit either.
#[test]
fn a_judge_whose_usage_names_no_repo_is_started_as_before() {
    let scratch = Scratch::new("hook-bound-unknown");
    let root = repository(&scratch);
    let (script, log) = reading_judge(&scratch);
    std::fs::write(
        log.with_file_name("judge.help"),
        "      --base <REV>     the claimed base (unverified)\n      --head <REV>     the claimed head (unverified)\n",
    )
    .unwrap();
    start(&root, "s");
    std::fs::write(root.join("b.rs"), "fn b() {}\n").unwrap();
    commit_all(&root, "b");
    assert_eq!(stop_from(&root, "s", &script), Value::Null);
    let args = args_of(&log);
    assert_eq!(args.len(), 1, "{args:?}");
    assert!(
        !args[0].contains("--repo") && !args[0].contains("--base"),
        "{args:?}"
    );
}

/// S3.25.b, ruling 3: what bound mode cannot verify is judged as before: a root commit (no
/// parent) and a merge commit (it brings in commits the pair does not name), without `--repo`;
/// the commit the merge brings in has one parent and is bound.
#[test]
fn a_root_commit_and_a_merge_are_judged_as_before() {
    let scratch = Scratch::new("hook-bound-root-merge");
    let root = repository(&scratch);
    let (script, log) = reading_judge(&scratch);
    takes_bound(&log);
    start(&root, "s");
    git(&root, &["checkout", "--quiet", "-b", "side"]);
    std::fs::write(root.join("side.rs"), "fn side() {}\n").unwrap();
    let side = commit_all(&root, "side");
    git(&root, &["checkout", "--quiet", "-"]);
    std::fs::write(root.join("main.rs"), "fn main() {}\n").unwrap();
    commit_all(&root, "main");
    git(&root, &["merge", "--quiet", "--no-ff", "--no-edit", "side"]);
    let merge = short(&root, "HEAD");
    git(&root, &["checkout", "--quiet", "--orphan", "fresh"]);
    std::fs::write(root.join("only.rs"), "fn only() {}\n").unwrap();
    let orphan = commit_all(&root, "a root commit");
    assert_eq!(stop_from(&root, "s", &script), Value::Null);
    let args = args_of(&log);
    let run_of = |id: &str| -> Vec<&String> {
        args.iter()
            .filter(|a| a.contains(&format!("--head {id}")))
            .collect()
    };
    let full = |id: &str| git(&root, &["rev-parse", id]).trim().to_owned();
    assert_eq!(
        run_of(&full(&side)).len(),
        1,
        "the side commit is not bound: {args:?}"
    );
    assert!(
        run_of(&full(&merge)).is_empty(),
        "the merge is bound: {args:?}"
    );
    assert!(
        run_of(&full(&orphan)).is_empty(),
        "the root commit is bound: {args:?}"
    );
    let bound = args.iter().filter(|a| a.contains("--repo")).count();
    assert_eq!(bound, 2, "{args:?}");
}

/// S3.25.b, ruling 3: a commit the project's frontier gives a stop, and one a stateless stop
/// judges, go the same way as a session's own: in bound mode.
#[test]
fn frontier_and_stateless_commits_are_judged_in_bound_mode() {
    let scratch = Scratch::new("hook-bound-frontier");
    let root = repository(&scratch);
    let (script, log) = reading_judge(&scratch);
    takes_bound(&log);
    start(&root, "a");
    std::fs::write(root.join("left.rs"), "fn left() {}\n").unwrap();
    let left = commit_all(&root, "left by a session that ended");
    start(&root, "b");
    assert_eq!(stop_from(&root, "b", &script), Value::Null);
    let full = |id: &str| git(&root, &["rev-parse", id]).trim().to_owned();
    assert!(
        args_of(&log)
            .iter()
            .any(|a| a.contains(&format!("--head {}", full(&left))) && a.contains("--repo")),
        "{:?}",
        args_of(&log)
    );
    // the stateless stop: a commit, then the state damaged
    std::fs::write(root.join("mine.rs"), "fn mine() {}\n").unwrap();
    let mine = commit_all(&root, "the session's own");
    for state in states_in(&sessions()) {
        std::fs::write(state, "{}").unwrap();
    }
    let said = allowed(&stop_from(&root, "b", &script)).to_owned();
    assert!(said.contains("could not be used (state.damaged)"), "{said}");
    assert!(
        args_of(&log)
            .iter()
            .any(|a| a.contains(&format!("--head {}", full(&mine))) && a.contains("--repo")),
        "{:?}",
        args_of(&log)
    );
}

/// S3.17a.l: when the advice after a tool call is off for a reason told once in the session, the
/// model is told as well as the user: it was told nothing, and advice that stops without a word
/// reads as a change with nothing to say.
#[test]
fn advice_off_is_told_to_the_model() {
    let scratch = Scratch::new("hook-advice-off");
    let root = repository(&scratch);
    let event = input("PostToolUse", &root, "s1", false);
    // a lasting reason (F6: a file name that is not UTF-8 no longer turns the advice off)
    git(&root, &["update-index", "--skip-worktree", "a.rs"]);
    let (code, stdout, stderr) = hook("post-tool", &[], &event);
    assert_eq!((code, stderr.as_str()), (0, ""), "{stdout}");
    let out: Value = serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("{e}: {stdout:?}"));
    let user = out["systemMessage"].as_str().unwrap_or_default();
    assert!(user.contains("the hook did not run: "), "{stdout}");
    assert_eq!(
        out["hookSpecificOutput"]["hookEventName"], "PostToolUse",
        "{stdout}"
    );
    let model = out["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap_or_else(|| panic!("the model is told nothing: {stdout}"));
    assert!(
        model.contains("no advice was given after this tool call")
            && model.contains("says nothing about it"),
        "{model}"
    );
    assert!(agent_facing_faults(model).is_empty(), "{model}");
    // told once: the next tool call says nothing
    assert_eq!(hook("post-tool", &[], &event).1, "");
}

/// A v3 verdict of a policy that requires verified claims (`[claims] require_verified`, which
/// `forge-warden init` writes from 0.3.0) on a patch the hook gives with no `--repo`, `--base`
/// or `--head`: every rule passes and the change is held for the note `claims_unverified`.
fn unverified_hold() -> Value {
    let mut document = verdict_v3("hold", &["pass"], false);
    document["notes"] = json!([{
        "code": "claims_unverified",
        "detail": "the patch was supplied without --repo, --base and --head; its claims cannot be verified"
    }]);
    document
}

/// S3.25.a: under a policy that requires verified claims a stop here can at best hold. The gate
/// called that chain ready (the probe is held, so the policy seemed to protect itself); both
/// `claims_verified` and `policy_protects_itself`, read from the verdict, say why it is not
/// ready, and so does a session's start.
#[test]
fn doctor_gate_says_stops_hold_when_the_policy_requires_verified_claims() {
    let (scratch, root, home, script, _log, exit_file) = gate_fixture("doctor-gate-claims");
    let judge_arg = ["--judge", script.to_str().unwrap()];
    let claims = |doc: &Value| -> (bool, String) {
        let ok = doc["checks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == "claims_verified")
            .unwrap_or_else(|| panic!("no link claims_verified: {doc:#}"))["ok"]
            .as_bool()
            .unwrap();
        (ok, detail(doc, "claims_verified"))
    };
    std::fs::create_dir(root.join(".claude")).unwrap();
    std::fs::write(
        root.join(".claude/settings.local.json"),
        stop_hook_settings(&script),
    )
    .unwrap();
    // a judge that holds the probe for a rule: the claims are not in question
    let (code, doc, stderr) = doctor_gate(&root, &home, &judge_arg);
    assert!(claims(&doc).0, "{doc:#}");
    assert_eq!(code, 0, "{stderr}\n{doc:#}");
    // one that holds it for claims it cannot verify
    let (again, _, _) = judge(&scratch, &unverified_hold());
    assert_eq!(again, script);
    std::fs::write(&exit_file, "8").unwrap();
    let (code, doc, stderr) = doctor_gate(&root, &home, &judge_arg);
    let (ok, said) = claims(&doc);
    assert!(
        !ok && said.starts_with("stops here will hold, never pass: ")
            && said.contains("An uncommitted or staged change can never be verified, so it can at best hold: commit first."),
        "{said}"
    );
    assert!(
        !doc["checks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|check| check["name"] == "policy_protects_itself")
            .unwrap()["ok"]
            .as_bool()
            .unwrap(),
        "{doc:#}"
    );
    assert_eq!((code, doc["ok"].as_bool()), (7, Some(false)), "{stderr}");
    // the start of a session tells the agent: judged, and held
    let (_, stdout, _) = hook(
        "session-start",
        &judge_arg,
        &input("SessionStart", &root, "s1", false),
    );
    assert!(
        stdout.contains("- Stops will be judged. Some links are weak: policy_protects_itself: ")
            && stdout.contains("claims_verified: stops here will hold, never pass: "),
        "{stdout}"
    );
}

/// S3.25.a: a change held only for claims the judge cannot verify is said at every stop, and
/// never passes; the stop names the note.
#[test]
fn a_hold_for_unverified_claims_is_said_at_every_stop_and_never_passes() {
    let scratch = Scratch::new("hook-claims-hold");
    let root = repository(&scratch);
    silence_a_lint(&root);
    let (script, _, exit_file) = judge(&scratch, &unverified_hold());
    std::fs::write(&exit_file, "8").unwrap();
    for active in [false, true, false] {
        let (code, stdout, stderr) = hook(
            "stop",
            &["--judge", script.to_str().unwrap()],
            &input("Stop", &root, "s1", active),
        );
        assert_eq!((code, stderr.as_str()), (0, ""), "{stdout}");
        let out: Value =
            serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("{e}: {stdout:?}"));
        let said = allowed(&out);
        assert!(
            said.contains("HOLD") && said.contains("claims_unverified"),
            "{said}"
        );
    }
}

/// The records of the project's ledger (F11), each line as JSON; none when it is not there.
fn ledger(root: &Path) -> Vec<Value> {
    std::fs::read_to_string(root.join(".git/deep-diff-forge/ledger.jsonl"))
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap_or_else(|e| panic!("{e}: {line}")))
        .collect()
}

/// The ledger's records of the stops of `session`.
fn stops_of(root: &Path, session: &str) -> Vec<Value> {
    let key = &deep_diff_forge_core::sha256_hex(session.as_bytes())[..16];
    ledger(root)
        .into_iter()
        .filter(|record| record["kind"] == "stop" && record["session"] == key)
        .collect()
}

/// F11.a: every outcome of a stop is a record of the project's ledger, under the Git common
/// directory where it outlives the session's state: a pass, a fail that blocks, the same fail
/// standing at the next stop, a hold, an unjudged change, a commit. A record holds no line of
/// the patch and none of the judge's words, and the file is the user's alone (mode 0600).
#[test]
fn every_stop_outcome_is_in_the_ledger() {
    use std::os::unix::fs::PermissionsExt as _;
    let scratch = Scratch::new("agent-hook-ledger-outcomes");
    let root = repository(&scratch);
    let (judge, _, exit_file) = judge(
        &scratch,
        &verdict("fail", "no-silent-suppression", "check_suppressed", "a.rs"),
    );
    silence_a_lint(&root);
    let outcome = |session: &str, code: &str, judge: &Path| -> Vec<Value> {
        std::fs::write(&exit_file, code).unwrap();
        start(&root, session);
        let _ = stop_from(&root, session, judge);
        stops_of(&root, session)
    };
    let unit = |records: &[Value], at: usize| {
        let r = records.get(at).unwrap_or_else(|| {
            panic!("the ledger has no record {at} of this session: {records:?}")
        });
        (
            r["outcome"].clone(),
            r["class"].clone(),
            r["blocked_turns"].clone(),
        )
    };

    let pass = outcome("pass", "0", &judge);
    assert_eq!(pass.len(), 1, "{pass:?}");
    assert_eq!(unit(&pass, 0), (json!("pass"), json!("pass"), json!(0)));
    let tree = pass[0]["unit"].as_str().unwrap();
    assert!(
        tree.starts_with("tree:") && tree.ends_with(":uncommitted") && tree.len() == 5 + 32 + 12,
        "{tree}"
    );
    assert_eq!(pass[0]["worktree"], "m");
    assert_eq!(pass[0]["seals"]["patch_sha256"], Value::Null, "{pass:?}");
    assert!(pass[0]["seals"]["policy_sha256"].is_string(), "{pass:?}");

    let fail = outcome("fail", "7", &judge);
    assert_eq!(unit(&fail, 0), (json!("fail"), json!("fail.new"), json!(1)));
    // the same failure at the next stop of the session stands
    let _ = stop_from(&root, "fail", &judge);
    let stands = stops_of(&root, "fail");
    assert_eq!(stands.len(), 2, "{stands:?}");
    assert_eq!(
        unit(&stands, 1),
        (json!("fail_stands"), json!("fail.stands"), json!(0))
    );
    assert_eq!(stands[0]["unit"], stands[1]["unit"], "one change, one name");

    let hold = outcome("hold", "8", &judge);
    assert_eq!(
        unit(&hold, 0),
        (json!("hold"), json!("hold.rule"), json!(0))
    );

    let unjudged = outcome("unjudged", "0", &scratch.join("no-such-judge"));
    assert_eq!(
        unit(&unjudged, 0),
        (json!("unjudged"), json!("chain.judge_missing"), json!(0))
    );

    // a commit of the session is recorded by its id, with the seals of its verdict
    std::fs::write(&exit_file, "7").unwrap();
    start(&root, "commit");
    git(&root, &["commit", "--quiet", "-am", "the failing change"]);
    let id = git(&root, &["rev-parse", "HEAD"]).trim().to_owned();
    let _ = stop_from(&root, "commit", &judge);
    let committed = stops_of(&root, "commit");
    let record = committed
        .iter()
        .find(|r| r["unit"] == format!("commit:{id}"))
        .unwrap_or_else(|| panic!("no record of the commit: {committed:?}"));
    assert_eq!(record["outcome"], "fail", "{record}");
    assert!(record["seals"]["patch_sha256"].is_string(), "{record}");
    assert!(record["seals"]["verdict_sha256"].is_string(), "{record}");

    let file = root.join(".git/deep-diff-forge/ledger.jsonl");
    let text = std::fs::read_to_string(&file).unwrap();
    for words in [
        "#[allow(dead_code)]",
        "dead_code",
        "A silenced check",
        "no-silent-suppression",
    ] {
        assert!(!text.contains(words), "the ledger holds {words:?}: {text}");
    }
    let mode = std::fs::metadata(&file).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "{mode:o}");
}

/// The stop's output with no ledger in the way, and with `spoil` done to the ledger in `own`,
/// at fresh sessions on the same failing change: the decision of each, and the reason or the
/// message of each.
fn stop_beside(root: &Path, judge: &Path, case: &str, spoil: impl FnOnce(&Path)) -> (Value, Value) {
    // (what an earlier case left at the ledger's name goes first: the control has a ledger)
    let _ = std::fs::remove_file(root.join(".git/deep-diff-forge/ledger.jsonl"));
    let control = stop_from(root, &format!("{case}-control"), judge);
    spoil(&root.join(".git/deep-diff-forge"));
    let spoiled = stop_from(root, &format!("{case}-spoiled"), judge);
    (control, spoiled)
}

/// F11, the coordinator's condition 2: a ledger that cannot be written changes no decision and
/// is never followed or written over. A ledger that is a link out of the repository, a FIFO, a
/// file of a newer build, a file another process holds the lock of, and a ledger directory that
/// cannot be written: each stop blocks as the stop beside it with a ledger does, says that the
/// ledger was not written (except where the stop could not judge at all, as before), and leaves
/// what stands at the ledger's name as it was. A held lock costs at most a fraction of a second.
#[test]
fn a_ledger_that_cannot_be_written_changes_no_decision() {
    use std::os::unix::fs::PermissionsExt as _;
    let scratch = Scratch::new("agent-hook-ledger-spoiled");
    let root = repository(&scratch);
    let (judge, _, _) = failing_judge(&scratch);
    silence_a_lint(&root);
    let file = |own: &Path| own.join("ledger.jsonl");
    let same_block = |(control, spoiled): (Value, Value), why: &str| {
        let (control, spoiled) = (blocked(&control), blocked(&spoiled));
        // (release 0.11.1, L1c: a ledger replaced while the project's policy record stands
        // makes the accepted policy unreadable, which the failing unit's words then say; the
        // decision, the block, is the same)
        let spoiled = spoiled
            .lines()
            .filter(|line| {
                !line.starts_with("This unit was judged under one of its two policies: this project's policy record and its ledger disagree")
                    && !line.starts_with("This unit was judged under one of its two policies: this project's ledger cannot be read whole")
            })
            .collect::<Vec<_>>()
            .join("\n");
        let spoiled = spoiled.as_str();
        assert!(spoiled.starts_with(control), "{control}\n---\n{spoiled}");
        let rest = &spoiled[control.len()..];
        assert!(
            rest.contains("could not write the project's ledger") && rest.contains(why),
            "{rest}"
        );
    };

    let outside = scratch.join("outside.txt");
    std::fs::write(&outside, "not the repository's\n").unwrap();
    same_block(
        stop_beside(&root, &judge, "link", |own| {
            std::fs::remove_file(file(own)).unwrap();
            std::os::unix::fs::symlink(&outside, file(own)).unwrap();
        }),
        "could not be opened",
    );
    assert_eq!(
        std::fs::read_to_string(&outside).unwrap(),
        "not the repository's\n"
    );

    same_block(
        stop_beside(&root, &judge, "fifo", |own| {
            std::fs::remove_file(file(own)).unwrap();
            let made = Command::new("mkfifo").arg(file(own)).status().unwrap();
            assert!(made.success());
        }),
        "not a regular file",
    );

    let newer = "{\"v\":2,\"kind\":\"stop\",\"unit\":\"later\"}\n";
    same_block(
        stop_beside(&root, &judge, "newer", |own| {
            std::fs::remove_file(file(own)).unwrap();
            std::fs::write(file(own), newer).unwrap();
        }),
        "newer build",
    );
    assert_eq!(
        std::fs::read_to_string(file(&root.join(".git/deep-diff-forge"))).unwrap(),
        newer
    );

    let mut holder = None;
    let started = std::cell::Cell::new(None);
    let go = scratch.join("go-ledger");
    same_block(
        stop_beside(&root, &judge, "locked", |own| {
            std::fs::remove_file(file(own)).unwrap();
            // (made by the holder, as `flock(1)` made it: a regular file, held)
            let mut child = hold_lock(&file(own), &go);
            assert!(child.try_wait().unwrap().is_none(), "the lock holder ended");
            holder = Some(child);
            started.set(Some(std::time::Instant::now()));
        }),
        "held its lock",
    );
    let took = started.get().unwrap().elapsed();
    if let Some(mut child) = holder {
        std::fs::remove_file(&go).unwrap();
        assert!(child.wait().unwrap().success());
    }
    assert!(took < std::time::Duration::from_secs(20), "{took:?}");

    // a ledger directory that cannot be written: the stop cannot make its scratch there and is
    // unjudged as before; nothing is made
    let own = root.join(".git/deep-diff-forge");
    std::fs::remove_file(file(&own)).unwrap();
    std::fs::set_permissions(&own, std::fs::Permissions::from_mode(0o500)).unwrap();
    let decision = stop_from(&root, "read-only", &judge);
    std::fs::set_permissions(&own, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(decision.get("decision").is_none(), "{decision}");
    assert!(
        !file(&own).exists(),
        "a ledger was made in a directory that cannot be written"
    );
}

/// `agent verify-branch RANGE --json` in `root`: its exit code, its document and its stderr.
fn verify_branch(root: &Path, range: &str) -> (i32, Value, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .current_dir(root)
        .envs(ISOLATE)
        .env("HOME", home())
        .args(["agent", "verify-branch", range, "--json"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let doc = serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&out.stdout)));
    (
        out.status.code().unwrap(),
        doc,
        String::from_utf8(out.stderr).unwrap(),
    )
}

/// The outcome `agent verify-branch` gives each commit of its range, oldest first.
fn verified(doc: &Value) -> Vec<(String, bool)> {
    doc["commits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            (
                c["outcome"].as_str().unwrap().to_owned(),
                c["cleared"] == true,
            )
        })
        .collect()
}

/// F11.b: `agent verify-branch BASE..HEAD` reads the project's ledger and refuses (exit 7,
/// `branch_unverified`, after its document) every commit of the range that no stop passed: one
/// a stop left unjudged, one that failed, one no stop recorded. A commit unjudged at one stop
/// and passed at a later one is verified; a clearance record (a convention) clears a commit; a
/// ledger with a record of a newer build is refused whole and left as it is.
#[test]
fn verify_branch_refuses_an_unjudged_unit() {
    let scratch = Scratch::new("agent-hook-verify-branch");
    let root = repository(&scratch);
    let base = git(&root, &["rev-parse", "HEAD"]).trim().to_owned();
    let (judge, _, exit_file) = failing_judge(&scratch);
    let range = format!("{base}..HEAD");
    let refused = |code: i32, stderr: &str, n: usize, m: usize| {
        assert_eq!(code, 7, "{stderr}");
        let line =
            format!("refused: {n} of {m} commits are not verified by the ledger (verify-branch)");
        assert!(stderr.lines().any(|l| l == line), "{stderr}");
    };

    // a commit a stop could not judge (no judge): unjudged, refused
    start(&root, "u");
    std::fs::write(root.join("c1.rs"), "fn c1() {}\n").unwrap();
    git(&root, &["add", "c1.rs"]);
    git(&root, &["commit", "--quiet", "-m", "c1"]);
    let _ = stop_from(&root, "u", &scratch.join("no-such-judge"));
    let (code, doc, stderr) = verify_branch(&root, &range);
    refused(code, &stderr, 1, 1);
    assert_eq!(verified(&doc), [("unjudged".to_owned(), false)], "{doc}");
    assert_eq!(doc["verified"], false);
    assert!(
        doc["cannot_know"]
            .as_str()
            .unwrap()
            .contains("proves nothing"),
        "{doc}"
    );

    // the next stop judges it and it passes: verified
    std::fs::write(&exit_file, "0").unwrap();
    let _ = stop_from(&root, "u", &judge);
    let (code, doc, stderr) = verify_branch(&root, &range);
    assert_eq!((code, stderr.as_str()), (0, ""), "{doc}");
    assert_eq!(verified(&doc), [("pass".to_owned(), false)], "{doc}");

    // a commit that fails, and one no stop recorded
    std::fs::write(&exit_file, "7").unwrap();
    start(&root, "f");
    silence_a_lint(&root);
    git(&root, &["commit", "--quiet", "-am", "c2"]);
    let _ = stop_from(&root, "f", &judge);
    git(&root, &["commit", "--quiet", "--allow-empty", "-m", "c3"]);
    let (code, doc, stderr) = verify_branch(&root, &range);
    refused(code, &stderr, 2, 3);
    let outcomes = verified(&doc);
    assert_eq!(outcomes[0].0, "pass", "{doc}");
    assert_eq!(outcomes[1].0, "fail", "{doc}");
    assert_eq!(outcomes[2].0, "no_record", "{doc}");

    // a clearance record, written by a person, clears both
    let file = root.join(".git/deep-diff-forge/ledger.jsonl");
    let mut text = std::fs::read_to_string(&file).unwrap();
    for c in ["HEAD~1", "HEAD"] {
        let id = git(&root, &["rev-parse", c]).trim().to_owned();
        text.push_str("{\"v\":1,\"kind\":\"clearance\",\"unit\":\"commit:");
        text.push_str(&id);
        text.push_str("\"}\n");
    }
    std::fs::write(&file, &text).unwrap();
    let (code, doc, stderr) = verify_branch(&root, &range);
    assert_eq!((code, stderr.as_str()), (0, ""), "{doc}");
    let cleared: Vec<bool> = verified(&doc).into_iter().map(|(_, c)| c).collect();
    assert_eq!(cleared, [false, true, true], "{doc}");

    // a record of a newer build: refused whole, and the file is as it was
    text.push_str("{\"v\":2,\"kind\":\"stop\"}\n");
    std::fs::write(&file, &text).unwrap();
    let (code, doc, stderr) = verify_branch(&root, &range);
    assert_eq!(code, 7, "{stderr}");
    assert!(
        stderr.contains("refused: the range or the ledger could not be read (verify-branch)"),
        "{stderr}"
    );
    assert!(
        doc["unverified"].as_str().unwrap().contains("newer build"),
        "{doc}"
    );
    assert_eq!(std::fs::read_to_string(&file).unwrap(), text);
}

/// A0110-M1 (the 0.11.0 tree review): a stop record that is damaged (no `outcome`, an outcome
/// this build does not write, no `unit`) read as no record, so a branch whose commit a stop had
/// passed still verified beside it. `agent verify-branch` refuses such a ledger whole, naming
/// the line, as it refuses one with a record of a newer build; the genuine pass verifies, and a
/// valid hold beside it (its class a hold's) is a hold, as before. The ledger is left as it is.
#[test]
fn verify_branch_refuses_a_ledger_with_a_damaged_stop_record() {
    let scratch = Scratch::new("agent-hook-verify-branch-damaged");
    let root = repository(&scratch);
    let base = git(&root, &["rev-parse", "HEAD"]).trim().to_owned();
    let (judge, _, exit_file) = failing_judge(&scratch);
    std::fs::write(&exit_file, "0").unwrap();
    start(&root, "d");
    std::fs::write(root.join("c1.rs"), "fn c1() {}\n").unwrap();
    git(&root, &["add", "c1.rs"]);
    git(&root, &["commit", "--quiet", "-m", "c1"]);
    let head = git(&root, &["rev-parse", "HEAD"]).trim().to_owned();
    let _ = stop_from(&root, "d", &judge);
    let range = format!("{base}..HEAD");
    let (code, doc, stderr) = verify_branch(&root, &range);
    assert_eq!((code, stderr.as_str()), (0, ""), "{doc}");

    let file = root.join(".git/deep-diff-forge/ledger.jsonl");
    let genuine = std::fs::read_to_string(&file).unwrap();
    let lines = genuine.lines().count();
    let passed: Value = genuine
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .find(|r| r["unit"] == format!("commit:{head}").as_str() && r["outcome"] == "pass")
        .unwrap_or_else(|| panic!("no pass of {head}: {genuine}"));
    let with = |edit: &dyn Fn(&mut serde_json::Map<String, Value>)| {
        let mut record = passed.as_object().unwrap().clone();
        edit(&mut record);
        format!("{genuine}{}\n", Value::Object(record))
    };
    let damaged: [(&str, String); 3] = [
        (
            "missing-outcome",
            with(&|r| {
                r.remove("outcome");
            }),
        ),
        (
            "unknown-outcome",
            with(&|r| {
                r.insert("outcome".into(), "held".into());
            }),
        ),
        (
            "missing-unit",
            with(&|r| {
                r.insert("outcome".into(), "fail".into());
                r.insert("class".into(), "fail.new".into());
                r.insert("blocked_turns".into(), 1.into());
                r.remove("unit");
            }),
        ),
    ];
    for (name, text) in &damaged {
        std::fs::write(&file, text).unwrap();
        let (code, doc, stderr) = verify_branch(&root, &range);
        assert_eq!(code, 7, "{name}: {stderr} {doc}");
        assert!(
            stderr.contains("refused: the range or the ledger could not be read (verify-branch)"),
            "{name}: {stderr}"
        );
        let why = doc["unverified"].as_str().unwrap_or_default();
        assert!(
            why.contains(&format!("line {} of the ledger", lines + 1))
                && why.contains("is not a record of this build"),
            "{name}: {doc}"
        );
        assert_eq!(doc["verified"], false, "{name}: {doc}");
        assert_eq!(doc["commits"], json!([]), "{name}: {doc}");
        assert_eq!(&std::fs::read_to_string(&file).unwrap(), text, "{name}");
    }

    // a valid hold beside the pass: a hold, not a refusal of the ledger
    std::fs::write(
        &file,
        with(&|r| {
            r.insert("outcome".into(), "hold".into());
            r.insert("class".into(), "hold.claims_unverified".into());
        }),
    )
    .unwrap();
    let (code, doc, stderr) = verify_branch(&root, &range);
    assert_eq!(code, 7, "{stderr}");
    assert!(
        stderr.contains("refused: 1 of 1 commits are not verified by the ledger (verify-branch)"),
        "{stderr}"
    );
    assert_eq!(verified(&doc), [("hold".to_owned(), false)], "{doc}");
    assert_eq!(doc["unverified"], Value::Null, "{doc}");
}

/// A stop of `session` from `root`, with the client's `stop_hook_active` as `active`: true is
/// a stop that follows a block in the same turn.
fn stop_as(root: &Path, session: &str, judge: &Path, active: bool) -> Value {
    let (code, stdout, stderr) = hook(
        "stop",
        &["--judge", judge.to_str().unwrap()],
        &input("Stop", root, session, active),
    );
    assert_eq!((code, stderr.as_str()), (0, ""), "{stdout}");
    if stdout.is_empty() {
        Value::Null
    } else {
        serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("{e}: {stdout:?}"))
    }
}

/// The blocked turns of the ledger's records of `session` whose class is `class`, in order; and
/// whether every one of them has the outcome `outcome`.
fn turns_of(root: &Path, session: &str, class: &str, outcome: &str) -> Vec<u64> {
    let records: Vec<Value> = stops_of(root, session)
        .into_iter()
        .filter(|record| record["class"] == class)
        .collect();
    for record in &records {
        assert_eq!(record["outcome"], outcome, "{record}");
    }
    records
        .iter()
        .map(|record| record["blocked_turns"].as_u64().unwrap())
        .collect()
}

/// F12.a (owner decision OD-J): a file of the working tree this account cannot read made every
/// stop UNJUDGED, so the stop was let through at no cost. It holds the first stop once, naming
/// the escaped path and nothing of Git's words; the stop that follows the block, and the first
/// stop of a later turn (a new hook process reading the state the session kept: what a
/// restarted client meets), let it through as before, UNJUDGED; the ledger records each, never
/// a pass. `agent check` says what the stop holds for. Made readable, the file is judged.
/// (S4.1 planted-breach row "unreadable path"; C3: no pass where no judge ran.)
#[test]
fn a_path_made_unreadable_in_the_session_holds_the_stop_once() {
    use std::os::unix::fs::PermissionsExt as _;
    let scratch = Scratch::new("hook-f12-unreadable");
    let root = repository(&scratch);
    let (script, log, _) = failing_judge(&scratch);
    start(&root, "s");
    silence_a_lint(&root);
    let secret = root.join("secret.rs");
    std::fs::write(&secret, "fn s() {}\n").unwrap();
    std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o000)).unwrap();
    // (whoever reads every file, as root does, has no such file)
    if std::fs::read(&secret).is_ok() {
        return;
    }
    let first = stop_as(&root, "s", &script, false);
    let reason = blocked(&first);
    assert!(
        reason.starts_with("deep-diff-forge: this change cannot pass as it is (")
            && reason.contains("a file in it cannot be read: \"secret.rs\". If this session made it so, undo it; if not, tell a person."),
        "{reason}"
    );
    assert!(
        !reason.contains("git add") && !reason.contains("fatal"),
        "{reason}"
    );
    for active in [true, false, false] {
        let said = allowed(&stop_as(&root, "s", &script, active)).to_owned();
        assert!(
            said.starts_with("deep-diff-forge: UNJUDGED. ")
                && said.contains("the change could not be read from Git"),
            "{said}"
        );
    }
    assert!(!log.exists(), "the judge was run without a patch");
    assert_eq!(
        turns_of(&root, "s", "work.unreadable", "unjudged"),
        [1, 0, 0, 0]
    );
    assert!(
        !stops_of(&root, "s").iter().any(|r| r["outcome"] == "pass"),
        "{:?}",
        stops_of(&root, "s")
    );
    // what agent check says of it is the stop's sentence
    let out = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .envs(ISOLATE)
        .env("HOME", home())
        .current_dir(&root)
        .args(["agent", "check", "--json", "--judge"])
        .arg(&script)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let doc: Value = serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&out.stdout)));
    let units = doc["units"].as_array().unwrap();
    assert!(
        units.iter().any(|unit| unit["outcome"] == "unjudged"
            && unit["reason"]
                .as_str()
                .is_some_and(|r| r.contains("a file in it cannot be read: \"secret.rs\""))),
        "{doc:#}"
    );
    // readable again: judged, and its failure blocks by its own lines
    std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(blocked(&stop_as(&root, "s", &script, false)).contains("check_suppressed in \"a.rs\""));
}

/// F12.a, the guard and the marks: paths marked skip-worktree made every stop UNJUDGED. New at
/// a stop that follows a block, they do not hold it (the client says the agent was just held:
/// holding again there could keep it from stopping); the first stop of the next turn holds
/// once; then they are let through, UNJUDGED, while the marks stay.
#[test]
fn skip_worktree_marks_hold_the_first_stop_of_a_turn_once() {
    let scratch = Scratch::new("hook-f12-skip");
    let root = repository(&scratch);
    let (script, log, _) = failing_judge(&scratch);
    start(&root, "s");
    silence_a_lint(&root);
    git(&root, &["update-index", "--skip-worktree", "a.rs"]);
    let said = allowed(&stop_as(&root, "s", &script, true)).to_owned();
    assert!(
        said.starts_with("deep-diff-forge: UNJUDGED. ")
            && said.contains("the first stop of the next turn holds for it"),
        "{said}"
    );
    let reason = blocked(&stop_as(&root, "s", &script, false)).to_owned();
    assert!(
        reason.contains("the index marks paths skip-worktree, so the working tree is not the whole change. If this session made it so, undo it; if not, tell a person."),
        "{reason}"
    );
    for active in [true, false] {
        let said = allowed(&stop_as(&root, "s", &script, active)).to_owned();
        assert!(said.contains("skip-worktree"), "{said}");
    }
    assert!(!log.exists(), "the judge was run without a patch");
    assert_eq!(
        turns_of(&root, "s", "work.skip_worktree", "unjudged"),
        [0, 1, 0, 0]
    );
}

/// F12.a, the coordinator's bound: a cause whose state keeps changing (a file another program
/// writes, which this account cannot read) holds again at a turn's first stop when it changed,
/// at most three times in a session; then each stop says that it has held that many times and
/// lets it through, UNJUDGED, recorded. A stop that follows a block is never held for it.
#[test]
fn a_cause_that_keeps_changing_holds_at_most_three_times_in_a_session() {
    use std::io::Write as _;
    use std::os::unix::fs::PermissionsExt as _;
    let scratch = Scratch::new("hook-f12-bound");
    let root = repository(&scratch);
    let (script, _, _) = failing_judge(&scratch);
    start(&root, "s");
    let log = root.join("service.log");
    std::fs::write(&log, "x").unwrap();
    // written to, never read: as a log another program keeps
    std::fs::set_permissions(&log, std::fs::Permissions::from_mode(0o200)).unwrap();
    if std::fs::read(&log).is_ok() {
        return;
    }
    let grow = || {
        let mut file = std::fs::OpenOptions::new().append(true).open(&log).unwrap();
        file.write_all(b"x").unwrap();
    };
    let bound =
        "it has held 3 stops of this session, the most one cause holds, so it is let through";
    blocked(&stop_as(&root, "s", &script, false));
    grow();
    allowed(&stop_as(&root, "s", &script, true));
    blocked(&stop_as(&root, "s", &script, false));
    grow();
    blocked(&stop_as(&root, "s", &script, false));
    for _ in 0..2 {
        grow();
        let said = allowed(&stop_as(&root, "s", &script, false)).to_owned();
        assert!(
            said.starts_with("deep-diff-forge: UNJUDGED. ") && said.contains(bound),
            "{said}"
        );
    }
    assert_eq!(
        turns_of(&root, "s", "work.unreadable", "unjudged"),
        [1, 0, 1, 1, 0, 0]
    );
    std::fs::set_permissions(&log, std::fs::Permissions::from_mode(0o644)).unwrap();
}

/// Ruling 2 of F12: with a state that cannot be saved nothing is remembered, so "once" is the
/// client's turn. A failure blocks the first stop of each turn and stands at the stop that
/// follows the block, recorded `fail.unsaved_continuing` with no blocked turn (the model's row
/// A18, untested until now); a cause that holds once does the same, never at a stop that
/// follows a block.
#[test]
fn fail_unsaved_continuing_is_recorded_and_once_is_per_turn_without_a_state() {
    use std::os::unix::fs::PermissionsExt as _;
    let scratch = Scratch::new("hook-f12-unsaved");
    let root = repository(&scratch);
    let (script, _, _) = failing_judge(&scratch);
    start(&root, "s");
    silence_a_lint(&root);
    let sessions = sessions();
    std::fs::set_permissions(&sessions, std::fs::Permissions::from_mode(0o500)).unwrap();
    if std::fs::write(sessions.join("probe"), "x").is_ok() {
        std::fs::set_permissions(&sessions, std::fs::Permissions::from_mode(0o700)).unwrap();
        return;
    }
    blocked(&stop_as(&root, "s", &script, false));
    let said = allowed(&stop_as(&root, "s", &script, true)).to_owned();
    assert!(
        said.starts_with("deep-diff-forge: FAIL STANDS. ")
            && said.contains("cannot keep its own record"),
        "{said}"
    );
    assert_eq!(
        turns_of(&root, "s", "fail.unsaved_continuing", "fail_stands"),
        [0]
    );
    // a cause that holds once, with nothing kept: once per turn
    git(&root, &["checkout", "--quiet", "--", "a.rs"]);
    git(&root, &["update-index", "--skip-worktree", "a.rs"]);
    for _ in 0..2 {
        blocked(&stop_as(&root, "s", &script, false));
        allowed(&stop_as(&root, "s", &script, true));
    }
    assert_eq!(
        turns_of(&root, "s", "work.skip_worktree", "unjudged"),
        [1, 0, 1, 0]
    );
    std::fs::set_permissions(&sessions, std::fs::Permissions::from_mode(0o700)).unwrap();
}

/// F12.a, the coordinator's condition 2: only a regular file is asked whether it can be read.
/// A FIFO in the tree is never opened (an open of a FIFO waits for a writer), and a link to a
/// file outside the repository that cannot be read is not followed: only the regular file is
/// named, and the stop takes its usual time.
#[test]
fn only_a_regular_file_is_named_as_unreadable_and_a_fifo_is_never_opened() {
    use std::os::unix::fs::PermissionsExt as _;
    let scratch = Scratch::new("hook-f12-fifo");
    let root = repository(&scratch);
    let (script, _, _) = failing_judge(&scratch);
    start(&root, "s");
    let outside = scratch.join("outside.rs");
    std::fs::write(&outside, "x\n").unwrap();
    std::fs::set_permissions(&outside, std::fs::Permissions::from_mode(0o000)).unwrap();
    std::os::unix::fs::symlink(&outside, root.join("link.rs")).unwrap();
    assert!(
        Command::new("mkfifo")
            .arg(root.join("pipe"))
            .status()
            .unwrap()
            .success()
    );
    let secret = root.join("secret.rs");
    std::fs::write(&secret, "x\n").unwrap();
    std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::read(&secret).is_ok() {
        return;
    }
    let began = std::time::Instant::now();
    let decision = stop_as(&root, "s", &script, false);
    let took = began.elapsed();
    let reason = blocked(&decision);
    assert!(
        reason.contains("a file in it cannot be read: \"secret.rs\". "),
        "{reason}"
    );
    assert!(
        !reason.contains("pipe") && !reason.contains("link.rs") && !reason.contains("outside"),
        "{reason}"
    );
    assert!(took < std::time::Duration::from_secs(20), "{took:?}");
    std::fs::set_permissions(&outside, std::fs::Permissions::from_mode(0o644)).unwrap();
    std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o644)).unwrap();
}

/// The model's row A6 as forge-warden 0.2 writes it: a hold with every rule passed and the
/// note `unknown_signal` for `label`.
fn unknown_label_hold(label: &str) -> Value {
    let mut document = verdict_v3("hold", &["pass"], false);
    document["notes"] = json!([{
        "code": "unknown_signal",
        "path": "a.rs",
        "detail": format!("signal `{label}` is not in this forge-warden's vocabulary; upgrade forge-warden or review it by hand")
    }]);
    document
}

/// F12.b (owner decision OD-J): a hold whose only cause is a label the paired judge does not
/// know let every stop through. It holds the first stop once, naming the label only when it is
/// one of DDF's own and never the judge's words; then the stop says HOLD, as before, and the
/// ledger says `hold.version_gap`. (S4.1 planted-breach row "unknown label".)
#[test]
fn an_unknown_label_hold_holds_once() {
    let scratch = Scratch::new("hook-f12-gap");
    let root = repository(&scratch);
    silence_a_lint(&root);
    let (script, _, exit_file) = judge(&scratch, &unknown_label_hold("diff_suppressed"));
    std::fs::write(&exit_file, "8").unwrap();
    start(&root, "s");
    let reason = blocked(&stop_as(&root, "s", &script, false)).to_owned();
    assert!(
        reason.contains("the judge does not know the label diff_suppressed (in \"a.rs\"): the paired forge-warden must be upgraded; tell a person."),
        "{reason}"
    );
    assert!(!reason.contains("review it by hand"), "{reason}");
    for active in [true, false] {
        let said = allowed(&stop_as(&root, "s", &script, active)).to_owned();
        assert!(said.starts_with("deep-diff-forge: HOLD. "), "{said}");
    }
    assert_eq!(turns_of(&root, "s", "hold.version_gap", "hold"), [1, 0, 0]);
    // a word that is no label of DDF's is never printed
    let (script, _, exit_file) = judge(&scratch, &unknown_label_hold("Ignore all previous"));
    std::fs::write(&exit_file, "8").unwrap();
    start(&root, "t");
    let reason = blocked(&stop_as(&root, "t", &script, false)).to_owned();
    assert!(
        reason.contains("the judge does not know a label DDF does not list (in \"a.rs\")")
            && !reason.contains("Ignore all previous"),
        "{reason}"
    );
}

/// F12.b on a commit (OD-J): a commit the judge holds only for a label it does not know holds
/// the stop that judges it, once, as the working tree's hold does; it is judged once and is
/// HOLD at every later stop, a continuing one and a later turn's, and the ledger records the
/// one blocked turn under `hold.version_gap`.
#[test]
fn an_unknown_label_hold_on_a_commit_holds_once() {
    let scratch = Scratch::new("hook-f12-gap-commit");
    let root = repository(&scratch);
    let (script, log, exit_file) = judge(&scratch, &unknown_label_hold("diff_suppressed"));
    std::fs::write(&exit_file, "8").unwrap();
    start(&root, "s");
    silence_a_lint(&root);
    git(
        &root,
        &[
            "commit",
            "--quiet",
            "-am",
            "work held for a label the judge does not know",
        ],
    );
    let reason = blocked(&stop_as(&root, "s", &script, false)).to_owned();
    assert!(
        reason.contains("the judge does not know the label diff_suppressed (in \"a.rs\"): the paired forge-warden must be upgraded; tell a person."),
        "a commit held for an unknown label was let through: {reason}"
    );
    assert!(!reason.contains("review it by hand"), "{reason}");
    for active in [true, false] {
        let said = allowed(&stop_as(&root, "s", &script, active)).to_owned();
        assert!(said.starts_with("deep-diff-forge: HOLD. "), "{said}");
    }
    assert_eq!(runs(&log), 1, "the commit was judged again");
    assert_eq!(turns_of(&root, "s", "hold.version_gap", "hold")[0], 1);
}

/// F12.c: a second stop on an unchanged failing unit is allowed and recorded as FAIL STANDS,
/// with no blocked turn; the first was recorded as the fail that blocked.
#[test]
fn fail_stands_is_recorded() {
    let scratch = Scratch::new("hook-f12-stands");
    let root = repository(&scratch);
    let (script, _, _) = failing_judge(&scratch);
    start(&root, "s");
    silence_a_lint(&root);
    blocked(&stop_as(&root, "s", &script, false));
    assert!(
        allowed(&stop_as(&root, "s", &script, false)).starts_with("deep-diff-forge: FAIL STANDS. ")
    );
    assert_eq!(turns_of(&root, "s", "fail.new", "fail"), [1]);
    assert_eq!(turns_of(&root, "s", "fail.stands", "fail_stands"), [0]);
}

/// The classes the project's ledger records for a stop of `session`, in order.
fn classes_of(root: &Path, session: &str) -> Vec<String> {
    stops_of(root, session)
        .iter()
        .map(|record| {
            record["class"]
                .as_str()
                .unwrap_or_else(|| panic!("a record with no class: {record}"))
                .to_owned()
        })
        .collect()
}

/// Lane H2 (the design's F7-W3a and F7-W3b): every reason class a stop reaches through the
/// binary with a stand-in judge is the class of a ledger record, where the ledger wrote
/// `"class": null` for the unjudged, the holds with no rule and the stops that judged nothing.
/// One scenario a class: the judge missing, inside the repository, refusing (exit 4), unable to
/// read its environment (exit 3), giving another code, or a document that is no verdict; the
/// base with no policy; a hold for claims, for notes or for an unknown label; a file that cannot
/// be read and skip-worktree marks; unmerged paths; a repository with no commit, an orphan
/// branch, a session begun before the first commit; a list of commits beside the state that
/// was cut. (Classes that need the stop's clock or a Git that fails or is slow are listed in
/// the lane's hand-back; a state that is not read is recorded since F12.d, by the tests of the
/// stateless stop.)
#[test]
#[expect(
    clippy::too_many_lines,
    reason = "one scenario a class, each a few lines, read against one ledger helper"
)]
fn every_reachable_class_is_in_the_ledger() {
    let fresh = |name: &str| {
        let scratch = Scratch::new(name);
        let root = repository(&scratch);
        (scratch, root)
    };
    let with_exit = |scratch: &Scratch, document: &Value, code: &str| {
        let (script, _, exit_file) = judge(scratch, document);
        std::fs::write(&exit_file, code).unwrap();
        script
    };
    let fail = verdict("fail", "no-silent-suppression", "check_suppressed", "a.rs");
    let stop_classes = |root: &Path, script: &Path| {
        start(root, "s");
        silence_a_lint(root);
        let _ = stop_as(root, "s", script, false);
        classes_of(root, "s")
    };
    let mut reached: Vec<String> = Vec::new();
    // the judge and its answers
    for (code, class) in [
        ("4", "chain.judge_refused"),
        ("3", "chain.judge_environment"),
        ("9", "chain.judge_exit"),
    ] {
        let (scratch, root) = fresh("hook-class-exit");
        let script = with_exit(&scratch, &fail, code);
        let classes = stop_classes(&root, &script);
        assert_eq!(classes, [class], "exit {code}");
        reached.extend(classes);
    }
    let (scratch, root) = fresh("hook-class-missing");
    let classes = stop_classes(&root, &scratch.join("no-such-judge"));
    assert_eq!(classes, ["chain.judge_missing"]);
    reached.extend(classes);
    let (scratch, root) = fresh("hook-class-not-a-verdict");
    let script = scratch.join("says-nothing");
    write_program(&script, "#!/bin/sh\nprintf '{}'\nexit 0\n");
    let classes = stop_classes(&root, &script);
    assert_eq!(classes, ["chain.not_a_verdict"]);
    reached.extend(classes);
    let (scratch, root) = fresh("hook-class-inside");
    let inside = root.join("judge");
    write_program(
        &inside,
        &judge_script(&scratch.join("log"), &scratch.join("exit"), &fail),
    );
    let classes = stop_classes(&root, &inside);
    assert_eq!(classes, ["chain.judge_inside"]);
    reached.extend(classes);
    // the policy
    let (scratch, root) = fresh("hook-class-policy");
    git(&root, &["rm", "--quiet", "warden.toml"]);
    git(&root, &["commit", "--quiet", "-m", "no policy"]);
    let script = with_exit(&scratch, &fail, "7");
    let classes = stop_classes(&root, &script);
    assert_eq!(classes, ["chain.policy_missing"]);
    reached.extend(classes);
    // holds with no rule held
    for (code, class) in [
        ("claims_unverified", "hold.claims_unverified"),
        ("other_note", "hold.notes"),
        ("unknown_signal", "hold.version_gap"),
    ] {
        let (scratch, root) = fresh("hook-class-hold");
        let mut document = verdict_v3("hold", &["pass"], false);
        document["notes"] = json!([{"code": code, "path": "a.rs",
            "detail": "signal `diff_suppressed` is not in this forge-warden's vocabulary"}]);
        let script = with_exit(&scratch, &document, "8");
        let classes = stop_classes(&root, &script);
        assert_eq!(classes, [class], "{code}");
        reached.extend(classes);
    }
    // causes in the working tree that hold once
    let (scratch, root) = fresh("hook-class-skip");
    let script = with_exit(&scratch, &fail, "7");
    start(&root, "s");
    silence_a_lint(&root);
    git(&root, &["update-index", "--skip-worktree", "a.rs"]);
    let _ = stop_as(&root, "s", &script, false);
    let classes = classes_of(&root, "s");
    assert_eq!(classes, ["work.skip_worktree"]);
    reached.extend(classes);
    let (scratch, root) = fresh("hook-class-unreadable");
    let script = with_exit(&scratch, &fail, "7");
    start(&root, "s");
    let secret = root.join("secret.rs");
    std::fs::write(&secret, "x\n").unwrap();
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o000)).unwrap();
    }
    if std::fs::read(&secret).is_err() {
        let _ = stop_as(&root, "s", &script, false);
        let classes = classes_of(&root, "s");
        assert_eq!(classes, ["work.unreadable"]);
        reached.extend(classes);
    }
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o644)).unwrap();
    }
    // unmerged paths
    let (scratch, root) = fresh("hook-class-unmerged");
    let script = with_exit(&scratch, &fail, "7");
    start(&root, "s");
    git(&root, &["checkout", "--quiet", "-b", "other"]);
    std::fs::write(root.join("a.rs"), "fn a() { 1 }\n").unwrap();
    git(&root, &["commit", "--quiet", "-am", "theirs"]);
    git(&root, &["checkout", "--quiet", "main"]);
    std::fs::write(root.join("a.rs"), "fn a() { 2 }\n").unwrap();
    git(&root, &["commit", "--quiet", "-am", "ours"]);
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
    assert!(!merged.status.success(), "the merge should conflict");
    let _ = stop_as(&root, "s", &script, false);
    let classes = classes_of(&root, "s");
    assert!(
        classes.contains(&"place.unmerged".to_owned()),
        "{classes:?}"
    );
    reached.extend(classes);
    // where there is nothing to measure from
    let scratch = Scratch::new("hook-class-no-commit");
    home_in(&scratch);
    let root = scratch.join("repo");
    std::fs::create_dir(&root).unwrap();
    git(&root, &["init", "--quiet"]);
    std::fs::write(root.join("a.rs"), "fn a() {}\n").unwrap();
    let script = with_exit(&scratch, &fail, "7");
    start(&root, "s");
    let _ = stop_as(&root, "s", &script, false);
    let classes = classes_of(&root, "s");
    assert_eq!(classes, ["place.no_commit_yet"]);
    reached.extend(classes);
    // ... and once a commit is made in a session that began with none
    git(&root, &["add", "--all"]);
    git(&root, &["commit", "--quiet", "-m", "first"]);
    let _ = stop_as(&root, "s", &script, false);
    let classes = classes_of(&root, "s");
    assert_eq!(
        classes.last().map(String::as_str),
        Some("place.before_first_commit")
    );
    reached.extend(classes);
    let (scratch, root) = fresh("hook-class-orphan");
    let script = with_exit(&scratch, &fail, "7");
    git(&root, &["checkout", "--quiet", "--orphan", "fresh"]);
    let classes = stop_classes(&root, &script);
    assert_eq!(classes, ["place.orphan"]);
    reached.extend(classes);
    // a list beside the state that is not the one it names
    let (scratch, root) = fresh("hook-class-lists");
    let script = with_exit(&scratch, &fail, "7");
    start(&root, "s");
    let tips: Vec<PathBuf> = std::fs::read_dir(sessions())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|kind| kind == "tips"))
        .collect();
    std::fs::write(&tips[0], "").unwrap();
    silence_a_lint(&root);
    let _ = stop_as(&root, "s", &script, false);
    let classes = classes_of(&root, "s");
    // F12.d: the tree is judged without the state, and the commits are recorded as not judged
    assert_eq!(classes, ["fail.new", "state.lists_damaged"]);
    reached.extend(classes);
    // the ledger writes no class it does not know, and no null
    reached.sort();
    reached.dedup();
    assert!(reached.len() >= 18, "{reached:?}");
}

/// Every file under `dir`, with its bytes, for a search of text.
fn files_under(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut found = Vec::new();
    let mut walk = vec![dir.to_path_buf()];
    while let Some(at) = walk.pop() {
        for entry in std::fs::read_dir(&at).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk.push(path);
            } else if let Ok(bytes) = std::fs::read(&path) {
                found.push((path, bytes));
            }
        }
    }
    found
}

/// The design's V8, lane H3's acceptance test: in a session whose patch has a line with 32
/// random hex digits, which the judge also writes on its stderr and in a note, no file of the
/// state directory holds the line, any eight characters of it, the judge's words, or a plain
/// SHA-256 anyone could compute from a guess: of (path, label, line) as a finding is known by,
/// of a rule, of (rule, signal, path), of what the judge said, of a signal or of a path. The
/// project's ledger is searched for the same.
#[test]
#[expect(
    clippy::too_many_lines,
    reason = "one session and one search of every file it leaves, with every unkeyed form"
)]
fn no_state_file_confirms_a_line_of_the_patch() {
    use deep_diff_forge_core::sha256_hex;
    let scratch = Scratch::new("hook-keyed-history");
    let root = repository(&scratch);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let hex = sha256_hex(format!("{nanos}-{}", std::process::id()).as_bytes())[..32].to_owned();
    let line = format!("#[allow(dead_code)] // {hex}");
    let marker = format!("judge-words-{}", &hex[..12]);
    let mut document = verdict_v3("fail", &["fail"], false);
    let detail = format!("{marker}: the line `{line}` is suspect");
    document["notes"] = json!([{"code": "quoted", "path": "a.rs", "detail": detail}]);
    let (judge_path, log, exit_file) = (
        scratch.join("judge"),
        scratch.join("judge.log"),
        scratch.join("judge.exit"),
    );
    std::fs::write(&exit_file, "7").unwrap();
    let body = judge_script(&log, &exit_file, &document).replacen(
        "#!/bin/sh\n",
        &format!("#!/bin/sh\nprintf '%s\\n' '{marker} saw {hex}' >&2\n"),
        1,
    );
    write_program(&judge_path, &body);
    start(&root, "s");
    std::fs::write(
        root.join("a.rs"),
        format!("fn a() {{}}\n{line}\nfn b() {{}}\n"),
    )
    .unwrap();
    let advice = hook("post-tool", &[], &input("PostToolUse", &root, "s", false)).1;
    assert!(advice.contains("check_suppressed"), "{advice}");
    blocked(&stop_as(&root, "s", &judge_path, false));
    let _ = stop_as(&root, "s", &judge_path, true);
    // (what has blocked is remembered while the change is not committed: the files are read
    // now, and again after the commit has its answer)
    let while_uncommitted = files_under(&sessions());
    assert!(
        the_state()["blocked"]
            .as_array()
            .is_some_and(|lines| lines.len() >= 2),
        "{}",
        the_state()
    );
    git(
        &root,
        &["commit", "--quiet", "-am", "the change with the line"],
    );
    let _ = stop_as(&root, "s", &judge_path, false);
    let _ = stop_as(&root, "s", &judge_path, false);
    // what a guess could be held against
    let mut forbidden: Vec<(String, String)> = vec![
        ("the line".to_owned(), line.clone()),
        ("the judge's words".to_owned(), marker.clone()),
        ("the note".to_owned(), detail.clone()),
    ];
    for at in 0..=line.len() - 8 {
        if line.is_char_boundary(at) && line.is_char_boundary(at + 8) {
            forbidden.push((
                format!("eight characters at {at}"),
                line[at..at + 8].to_owned(),
            ));
        }
    }
    let mut unkeyed = |what: &str, words: &str| {
        let digest = sha256_hex(words.as_bytes());
        forbidden.push((format!("SHA-256 of {what}"), digest.clone()));
        forbidden.push((format!("SHA-256 of {what}, cut"), digest[..24].to_owned()));
    };
    unkeyed("the finding", &format!("a.rs\0check_suppressed\0{line}"));
    unkeyed(
        "the note said",
        &format!("\0said\0note quoted at \"a.rs\": {detail}"),
    );
    unkeyed(
        "the note said (path plain)",
        &format!("\0said\0note quoted at a.rs: {detail}"),
    );
    for rule in document["rules"].as_array().unwrap() {
        let id = rule["id"].as_str().unwrap();
        unkeyed(&format!("the rule {id}"), id);
        for violation in rule["violations"].as_array().unwrap() {
            let signal = violation["signal"].as_str().unwrap_or_default();
            let path = violation["path"].as_str().unwrap_or_default();
            unkeyed(
                &format!("{id}, {signal}, {path}"),
                &format!("{id}\0{signal}\0{path}"),
            );
            unkeyed(&format!("the signal {signal}"), signal);
            unkeyed(&format!("the path {path}"), path);
        }
    }
    unkeyed("the path a.rs", "a.rs");
    unkeyed("the signal check_suppressed", "check_suppressed");
    let state_files = files_under(&sessions());
    assert!(state_files.len() >= 4, "{state_files:?}");
    let ledger = root.join(".git/deep-diff-forge/ledger.jsonl");
    let ledger_file = (ledger.clone(), std::fs::read(&ledger).unwrap_or_default());
    for (path, bytes) in (state_files.iter())
        .chain(&while_uncommitted)
        .chain(std::iter::once(&ledger_file))
    {
        let text = String::from_utf8_lossy(bytes);
        for (what, words) in &forbidden {
            assert!(
                !text.contains(words.as_str()),
                "{} holds {what}",
                path.display()
            );
        }
    }
    // the state says which key it was kept under, and the key is no part of any state
    let key = std::fs::read(sessions().join("key")).unwrap();
    assert_eq!(key.len(), 32);
    let key_hex = key.iter().fold(String::new(), |mut hex, b| {
        use std::fmt::Write as _;
        let _ = write!(hex, "{b:02x}");
        hex
    });
    for (path, bytes) in &state_files {
        if path.file_name().is_some_and(|name| name != "key") {
            let text = String::from_utf8_lossy(bytes);
            assert!(!text.contains(&key_hex), "{} holds the key", path.display());
        }
    }
    assert_eq!(the_state()["schema"], "deep-diff-forge.hook-state.v13");
}

/// Lane H3, ruling 1: what emptying the keyed members costs (a migration, or a key that was
/// replaced or lost), and that it never turns a stop that held or blocked into one that does
/// not: a finding already said is said once more (`reported`), a failure that blocked blocks
/// once more and then stands (`blocked`), and a cause that holds once holds again, its count
/// begun anew (the once lines of `blocked`). Each after a new key; no stop that held lets
/// through.
#[test]
fn a_new_key_says_and_blocks_again_and_never_less() {
    let scratch = Scratch::new("hook-key-replaced");
    let root = repository(&scratch);
    let (script, _, _) = failing_judge(&scratch);
    start(&root, "s");
    silence_a_lint(&root);
    let advice = || hook("post-tool", &[], &input("PostToolUse", &root, "s", false)).1;
    let replace_key = |byte: u8| std::fs::write(sessions().join("key"), [byte; 32]).unwrap();
    assert!(advice().contains("check_suppressed"));
    assert_eq!(advice(), "");
    blocked(&stop_as(&root, "s", &script, false));
    assert!(
        allowed(&stop_as(&root, "s", &script, false)).starts_with("deep-diff-forge: FAIL STANDS. ")
    );
    replace_key(1);
    // the finding is said once more, and then not
    assert!(
        advice().contains("check_suppressed"),
        "a new key forgot nothing"
    );
    assert_eq!(advice(), "");
    // the failure blocks once more, and then stands
    blocked(&stop_as(&root, "s", &script, false));
    assert!(
        allowed(&stop_as(&root, "s", &script, false)).starts_with("deep-diff-forge: FAIL STANDS. ")
    );
    // a cause that holds once: held, then not; a new key: held again
    git(&root, &["checkout", "--quiet", "--", "a.rs"]);
    git(&root, &["update-index", "--skip-worktree", "a.rs"]);
    blocked(&stop_as(&root, "s", &script, false));
    allowed(&stop_as(&root, "s", &script, false));
    replace_key(2);
    blocked(&stop_as(&root, "s", &script, false));
    allowed(&stop_as(&root, "s", &script, false));
    assert_eq!(the_state()["key_id"].as_str().map(str::len), Some(16));
}

/// Lane H3, ruling 5, through the binary: a key file that is no key (here a link to one) ends
/// every hook as a state it cannot use does (`state.damaged`): the stop is UNJUDGED and did
/// not run, nothing is judged or written, and no byte of a key is said.
#[test]
fn a_key_file_that_is_no_key_judges_nothing_and_says_no_key() {
    let scratch = Scratch::new("hook-key-link");
    let root = repository(&scratch);
    let (script, log, _) = failing_judge(&scratch);
    start(&root, "s");
    let key = sessions().join("key");
    let real = std::fs::read(&key).unwrap();
    let elsewhere = scratch.join("a-key-elsewhere");
    std::fs::write(&elsewhere, &real).unwrap();
    std::fs::remove_file(&key).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &key).unwrap();
    silence_a_lint(&root);
    let state = states_in(&sessions()).remove(0);
    let before = std::fs::read(&state).unwrap();
    // F12.d: the tree is judged without the state, and its failure blocks
    let said = blocked(&stop_as(&root, "s", &script, false)).to_owned();
    assert!(
        said.starts_with(&stateless("state.damaged"))
            && said.contains("Why: the hooks' key file ")
            && said.contains("cannot be opened"),
        "{said}"
    );
    let hex = real.iter().fold(String::new(), |mut hex, b| {
        use std::fmt::Write as _;
        let _ = write!(hex, "{b:02x}");
        hex
    });
    assert!(!said.contains(&hex) && !said.contains(&*String::from_utf8_lossy(&real)));
    assert_eq!(runs(&log), 1);
    assert_eq!(std::fs::read(&state).unwrap(), before);
}

/// Whether a unit test that its library's test binary names `path::function` (as
/// `module::tests::name`) is in some crate of the workspace: the longest leading part of `path`
/// that is a source file of a crate (`src/a/b.rs` or `src/a/b/mod.rs`; `src/lib.rs` or
/// `src/main.rs` for none) holds `mod <m> {` for each module of `path` left and `fn <function>()`.
fn a_unit_test_exists(crates: &Path, path: &str, function: &str) -> bool {
    let modules: Vec<&str> = path.split("::").collect();
    std::fs::read_dir(crates).unwrap().any(|entry| {
        let src = entry.unwrap().path().join("src");
        (0..=modules.len()).rev().any(|file_part| {
            let stem = modules[..file_part].join("/");
            let files = if file_part == 0 {
                ["lib.rs".to_owned(), "main.rs".to_owned()]
            } else {
                [format!("{stem}.rs"), format!("{stem}/mod.rs")]
            };
            files.iter().any(|file| {
                std::fs::read_to_string(src.join(file)).is_ok_and(|text| {
                    modules[file_part..]
                        .iter()
                        .all(|module| text.contains(&format!("mod {module} {{")))
                        && text.contains(&format!("fn {function}()"))
                })
            })
        })
    })
}

/// The list of load-sensitive tests (`tests/LOAD_SENSITIVE`) names tests that exist, each once,
/// so a renamed or removed test leaves no stale name for a run that skips the list: a test
/// function of this directory by its name, a unit test of a library by its path in it
/// (`module::tests::name`).
#[test]
fn the_load_sensitive_list_names_tests_that_exist() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let tests = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let list = std::fs::read_to_string(tests.join("LOAD_SENSITIVE")).unwrap();
    let mut sources = String::new();
    for entry in std::fs::read_dir(&tests).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|extension| extension == "rs") {
            sources.push_str(&std::fs::read_to_string(&path).unwrap());
        }
    }
    let names: Vec<&str> = list
        .lines()
        .filter_map(|line| line.split('#').next().map(str::trim))
        .filter(|name| !name.is_empty())
        .collect();
    assert!(!names.is_empty(), "LOAD_SENSITIVE names no test");
    let unknown: Vec<&&str> = names
        .iter()
        .filter(|name| match name.rsplit_once("::") {
            None => !sources.contains(&format!("fn {name}()")),
            Some((path, function)) => !a_unit_test_exists(&crates, path, function),
        })
        .collect();
    assert!(
        unknown.is_empty(),
        "LOAD_SENSITIVE names tests that are not here: {unknown:?}"
    );
    // the reader of unit tests' paths, on names that are not there
    for planted in [
        "evidence::tests::no_such_test",
        "no_such_module::tests::the_import_rule_is_linear_on_long_lines",
        "evidence::no_such_tests::the_import_rule_is_linear_on_long_lines",
    ] {
        let (path, function) = planted.rsplit_once("::").unwrap();
        assert!(
            !a_unit_test_exists(&crates, path, function),
            "{planted} was read as a test"
        );
    }
    let mut once = names.clone();
    once.sort_unstable();
    once.dedup();
    assert_eq!(once.len(), names.len(), "LOAD_SENSITIVE names a test twice");
}

// ---- Release 0.11.1, P5c: the session's lock is taken wherever the state is written ----

/// Where a `.git` stands above the hooks' state directory: the layouts a to d of P5c's design
/// (`wf3/P5c-state-directory-design.md`, measured on 0.11.0).
#[derive(Clone, Copy, Debug)]
enum Above {
    /// None (the control).
    Nothing,
    /// The home directory is a Git repository with a commit (a dotfiles repository).
    Repository,
    /// An empty `.git` directory in the home directory.
    EmptyGitDirectory,
    /// The home directory is a linked working tree of another repository (`.git` is a file).
    GitFile,
}

/// Whether the row `above` of a test is no control under `scratch`: [`Above::Nothing`] is a
/// control only where no `.git` stands above the temporary directory (review of P5c, L4). Such
/// a row is said and left out, not passed.
fn not_a_control(scratch: &Scratch, above: Above) -> bool {
    let Above::Nothing = above else {
        return false;
    };
    let Some(git) = git_above(&scratch.join("control")) else {
        return false;
    };
    eprintln!(
        "note: the control row (nothing above the state directory) is left out: a .git stands at {} above the temporary directory",
        git.display()
    );
    true
}

/// Make `home` the home of this thread's hooks.
fn home_is(home: &Path) {
    HOME.with(|slot| *slot.borrow_mut() = Some(home.to_path_buf()));
    name_what_the_hooks_met();
}

/// A home `name` in `scratch` with `above` in it and no state directory yet, made the home of
/// this thread's hooks.
fn fresh_home(scratch: &Scratch, name: &str, above: Above) -> PathBuf {
    let home = scratch.join(name);
    let dotfiles = |repository: &Path| {
        git(repository, &["init", "--quiet"]);
        std::fs::write(repository.join(".bashrc"), "# dotfiles\n").unwrap();
        git(repository, &["add", ".bashrc"]);
        git(repository, &["commit", "--quiet", "-m", "dotfiles"]);
    };
    match above {
        Above::Nothing => std::fs::create_dir(&home).unwrap(),
        Above::Repository => {
            std::fs::create_dir(&home).unwrap();
            dotfiles(&home);
        }
        Above::EmptyGitDirectory => std::fs::create_dir_all(home.join(".git")).unwrap(),
        Above::GitFile => {
            let dots = scratch.join(format!("{name}-dots"));
            std::fs::create_dir(&dots).unwrap();
            dotfiles(&dots);
            git(
                &dots,
                &[
                    "worktree",
                    "add",
                    "--quiet",
                    "-b",
                    name,
                    home.to_str().unwrap(),
                ],
            );
            assert!(home.join(".git").is_file());
        }
    }
    home_is(&home);
    home
}

/// `session-<the first 32 hex digits of the SHA-256 of the session's id>`: the stem of the
/// session's files.
fn stem(session: &str) -> String {
    format!(
        "session-{}",
        &deep_diff_forge_core::sha256_hex(session.as_bytes())[..32]
    )
}

/// The id a state gives the hooks' key it was written with: the first 16 hex digits of the
/// SHA-256 of `ddf-hook-key`, a zero byte and the key.
fn key_id(key: &[u8]) -> String {
    let mut bytes = b"ddf-hook-key\0".to_vec();
    bytes.extend_from_slice(key);
    deep_diff_forge_core::sha256_hex(&bytes)[..16].to_owned()
}

/// The names in `dir`, sorted; none when `dir` is not there (so that an assertion about them
/// says what it found instead of failing on the listing).
fn names_in(dir: &Path) -> Vec<String> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(error) => panic!("{}: {error}", dir.display()),
    };
    let mut names: Vec<String> = entries
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// Whether another process holds an exclusive `flock` on `path` now. (`flock(2)` from this
/// process: portable to macOS, which has no `/proc/locks`.)
fn locked_by_another(path: &Path) -> bool {
    let file = std::fs::File::open(path).unwrap();
    match nix::fcntl::Flock::lock(file, nix::fcntl::FlockArg::LockExclusiveNonblock) {
        Ok(_taken) => false,
        Err((_, nix::errno::Errno::EWOULDBLOCK)) => true,
        Err((_, other)) => panic!("flock {}: {other}", path.display()),
    }
}

/// Wait, at most twenty seconds, until `done`.
fn wait_until(what: &str, done: impl Fn() -> bool) {
    let asked = std::time::Instant::now();
    while !done() {
        assert!(
            asked.elapsed() < std::time::Duration::from_secs(20),
            "{what} did not happen in twenty seconds"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

/// Release 0.11.1, P5c (rows C3, C5, C7; test T1). A hook that ran while the hooks' state
/// directory was not there took no lock when a directory above it held a `.git` (a home
/// directory that is a Git repository, an empty `.git`, the `.git` file of a linked working
/// tree), and the state was written all the same. Two first stops of a session both judged
/// and both wrote, and the client's continuing stop blocked again in 28 of 46 measured races:
/// each stop drew a key, the state named the one that lost, and the next hook emptied what had
/// blocked. With any of them above it every hook behaves as with none (the control, first).
#[test]
fn a_git_directory_above_the_state_directory_changes_nothing_of_the_lock() {
    for above in [
        Above::Nothing,
        Above::Repository,
        Above::EmptyGitDirectory,
        Above::GitFile,
    ] {
        let scratch = Scratch::new(&format!("hook-git-above-{above:?}"));
        if not_a_control(&scratch, above) {
            continue;
        }
        let root = repository(&scratch);
        let (script, log) = reading_judge(&scratch);
        // the account's first hook is a start: it leaves its lock file beside the state
        fresh_home(&scratch, "start-home", above);
        start(&root, "s");
        let (state, lock) = (
            sessions().join(format!("{}.json", stem("s"))),
            sessions().join(format!("{}.lock", stem("s"))),
        );
        assert!(
            state.is_file() && lock.is_file(),
            "{above:?}: the first start took no lock: {:?}",
            names_in(&sessions())
        );
        // a stop holds the session's lock while its judge runs
        secret(&root, "k.rs");
        std::fs::write(scratch.join("judge.sleep"), "2\n").unwrap();
        let start_home = home();
        let held = std::thread::scope(|scope| {
            let stop = scope.spawn(|| {
                home_is(&start_home);
                stop_from(&root, "s", &script)
            });
            wait_until("the stop's judging", || runs(&log) == 1);
            let held = locked_by_another(&lock);
            blocked(&stop.join().unwrap());
            held
        });
        assert!(
            held,
            "{above:?}: the stop did not hold the session's lock while it judged"
        );
        std::fs::remove_file(scratch.join("judge.sleep")).unwrap();
        // an advice hook under a held lock says nothing and writes nothing
        silence_a_lint(&root);
        let before = std::fs::read(&state).unwrap();
        let go = scratch.join("go");
        let mut holder = hold_lock(&lock, &go);
        let said = hook("post-tool", &[], &input("PostToolUse", &root, "s", false)).1;
        std::fs::remove_file(&go).unwrap();
        assert!(holder.wait().unwrap().success());
        assert!(
            said.is_empty() && std::fs::read(&state).unwrap() == before,
            "{above:?}: an advice hook under a held lock said or wrote: {said}"
        );
        // (and with the lock let go it says and writes: the control of the line above)
        let said = hook("post-tool", &[], &input("PostToolUse", &root, "s", false)).1;
        assert!(
            said.contains("check_suppressed") && std::fs::read(&state).unwrap() != before,
            "{above:?}: {said}"
        );

        two_first_stops_take_turns(&scratch, &root, &script, above);
    }
}

/// Two first stops of a session at once, in a fresh home with `above` in it: one judges with
/// the state and the other finds the lock held (both block, as a failing tree blocks every
/// stop); the client's continuing stop is then let through; and the state names the hooks' key
/// (test T1, [`a_git_directory_above_the_state_directory_changes_nothing_of_the_lock`]).
fn two_first_stops_take_turns(scratch: &Scratch, root: &Path, script: &Path, above: Above) {
    fresh_home(scratch, "race-home", above);
    let race_home = home();
    let both = two_first_stops(scratch, root, script, "first", || home_is(&race_home));
    let blocks = both
        .iter()
        .filter(|said| said["decision"] == "block")
        .count();
    let waited = both
        .iter()
        .filter(|said| {
            said["reason"]
                .as_str()
                .is_some_and(|text| text.starts_with(&stateless("state.lock_busy")))
        })
        .count();
    assert_eq!(
        (blocks, waited),
        (2, 1),
        "{above:?}: two first stops of a session ran at once: {both:?}"
    );
    let (code, stdout, stderr) = hook(
        "stop",
        &["--judge", script.to_str().unwrap()],
        &input("Stop", root, "first", true),
    );
    assert_eq!((code, stderr.as_str()), (0, ""), "{stdout}");
    let after: Value = if stdout.is_empty() {
        Value::Null
    } else {
        serde_json::from_str(&stdout).unwrap()
    };
    assert_ne!(
        after["decision"], "block",
        "{above:?}: the client's continuing stop blocked again: {after}"
    );
    let key = std::fs::read(sessions().join("key")).unwrap();
    let state: Value = serde_json::from_slice(
        &std::fs::read(sessions().join(format!("{}.json", stem("first")))).unwrap(),
    )
    .unwrap();
    assert_eq!(
        state["key_id"].as_str(),
        Some(key_id(&key).as_str()),
        "{above:?}: the state names another key than the hooks' key file"
    );
}

/// Release 0.11.1, P5c row C4 (test T2): wherever a hook writes a session's state it holds the
/// session's lock. A hook that took the lock leaves its lock file while the state stands; one
/// that took none leaves none. So after each hook (a session's first hook, an advice hook, in
/// a fresh home; then a start, an advice hook and a stop), in every layout and with
/// `XDG_STATE_HOME` outside a home that is a repository, the state is there with its lock file
/// beside it, and every stop holds the lock while its judge runs.
#[test]
fn wherever_a_hook_writes_the_state_it_holds_the_lock() {
    let scratch = Scratch::new("hook-lock-wherever");
    let root = repository(&scratch);
    let (script, log) = reading_judge(&scratch);
    silence_a_lint(&root);
    secret(&root, "k.rs");
    std::fs::write(scratch.join("judge.sleep"), "1\n").unwrap();
    let layouts = [
        ("a", Above::Nothing, false),
        ("b", Above::Repository, false),
        ("c", Above::EmptyGitDirectory, false),
        ("d", Above::GitFile, false),
        ("g", Above::Repository, true),
    ];
    for (name, above, outside) in layouts {
        if not_a_control(&scratch, above) {
            continue;
        }
        let home = fresh_home(&scratch, &format!("home-{name}"), above);
        let xdg = scratch.join(format!("state-{name}"));
        let dir = if outside {
            xdg.join("deep-diff-forge/hooks")
        } else {
            sessions()
        };
        let session = format!("t-{name}");
        let shape = |command: &mut Command| {
            if outside {
                command.env("XDG_STATE_HOME", &xdg);
            }
        };
        let run = |event: &str, extra: &[&str], kind: &str| {
            let (code, stdout, stderr) =
                hook_in(event, extra, &input(kind, &root, &session, false), shape);
            assert_eq!((code, stderr.as_str()), (0, ""), "{stdout}");
        };
        let kept_under_the_lock = |after: &str| {
            let names = names_in(&dir);
            let stem = stem(&session);
            assert!(
                names.contains(&format!("{stem}.json")) && names.contains(&format!("{stem}.lock")),
                "{name}: after {after} the state is not there with its lock file beside it: {names:?}"
            );
        };
        run("post-tool", &[], "PostToolUse");
        kept_under_the_lock("the session's first hook, an advice hook");
        run("session-start", &[], "SessionStart");
        kept_under_the_lock("a start");
        run("post-tool", &[], "PostToolUse");
        kept_under_the_lock("an advice hook");
        let judged = runs(&log);
        let lock = dir.join(format!("{}.lock", stem(&session)));
        let held = std::thread::scope(|scope| {
            let stop = scope.spawn(|| {
                home_is(&home);
                run("stop", &["--judge", script.to_str().unwrap()], "Stop");
            });
            wait_until("the stop's judging", || runs(&log) > judged);
            let held = locked_by_another(&lock);
            stop.join().unwrap();
            held
        });
        assert!(
            held,
            "{name}: the stop did not hold the lock while it judged"
        );
        kept_under_the_lock("a stop");
    }
}

/// Release 0.11.1, P5c rows C5 and C6 (test T3): a state directory inside the working tree of
/// the session's project is refused, as 0.11.0 refuses it (the home directory is the project,
/// or lies inside it): the start and an advice hook say so, every stop is UNJUDGED and never
/// blocks, the judge is never called, and when the hooks have run no file of theirs is in the
/// project and Git sees what it saw before. (The lock may leave empty directories there: Git
/// records none.)
#[test]
fn a_state_directory_inside_the_project_is_refused_and_leaves_no_file_there() {
    for case in ["home-is-the-project", "home-inside-the-project"] {
        let scratch = Scratch::new(&format!("hook-state-in-project-{case}"));
        let root = repository(&scratch);
        let (script, log) = reading_judge(&scratch);
        let home = if case == "home-is-the-project" {
            root.clone()
        } else {
            root.join("users/u")
        };
        std::fs::create_dir_all(&home).unwrap();
        home_is(&home);
        secret(&root, "k.rs");
        let status = || {
            git(
                &root,
                &[
                    "status",
                    "--porcelain",
                    "--untracked-files=all",
                    "--ignored",
                ],
            )
        };
        let before = status();
        let refused = "lies inside the project's working tree";
        let (code, started, stderr) = hook(
            "session-start",
            &[],
            &input("SessionStart", &root, "s", false),
        );
        assert_eq!((code, stderr.as_str()), (0, ""), "{started}");
        assert!(
            started.starts_with("deep-diff-forge: the stops of this session will not be judged and no advice will be given: the state directory ")
                && started.contains(refused)
                && started.contains(": point XDG_STATE_HOME (or HOME) outside it. That is not a pass."),
            "{case}: {started}"
        );
        let advice =
            system_message(&hook("post-tool", &[], &input("PostToolUse", &root, "s", false)).1);
        assert!(
            advice.starts_with("deep-diff-forge: the hook did not run: the state directory ")
                && advice.contains(refused),
            "{case}: {advice}"
        );
        assert_eq!(
            hook("post-tool", &[], &input("PostToolUse", &root, "s", false)).1,
            "",
            "{case}: said twice"
        );
        for _ in 0..2 {
            let stop = stop_from(&root, "s", &script);
            let said = allowed(&stop);
            assert!(
                said.starts_with(
                    "deep-diff-forge: UNJUDGED. This stop was not judged: the state directory "
                ) && said.contains(refused),
                "{case}: {said}"
            );
        }
        assert_eq!(runs(&log), 0, "{case}: the judge was called");
        let left = files_left_under(&home.join(".local"));
        assert!(
            left.is_empty(),
            "{case}: the hooks left files in the project: {left:?}"
        );
        assert_eq!(status(), before, "{case}");
    }
}

/// The session-start hook takes the session's lock for its reading and writing (SECURITY.md;
/// release 0.11.1, an audit found no test of it: every test contended stops or advice hooks).
/// Another process holds the lock of a session while its start runs: the start writes nothing
/// into the state directory, says that it recorded nothing and why, and a start after the lock
/// is let go records the session as usual.
#[test]
fn a_session_start_under_a_held_lock_records_nothing_and_says_so() {
    let scratch = Scratch::new("hook-start-lock-held");
    let root = repository(&scratch);
    // (another session's start makes the state directory)
    start(&root, "other");
    let listing = || -> Vec<(String, Vec<u8>)> {
        names_in(&sessions())
            .into_iter()
            .map(|name| {
                let bytes = std::fs::read(sessions().join(&name)).unwrap();
                (name, bytes)
            })
            .collect()
    };
    let lock = sessions().join(format!("{}.lock", stem("s")));
    let go = scratch.join("go");
    let mut holder = hold_lock(&lock, &go);
    let before = listing();
    let (code, said, stderr) = hook(
        "session-start",
        &[],
        &input("SessionStart", &root, "s", false),
    );
    let after = listing();
    std::fs::remove_file(&go).unwrap();
    assert!(holder.wait().unwrap().success());
    assert_eq!((code, stderr.as_str()), (0, ""), "{said}");
    assert!(
        said.contains("- The start of this session could not be recorded (another hook of this session was running, so this start recorded nothing): a later hook takes the commit HEAD names then, so commits made before it are not judged."),
        "{said}"
    );
    assert_eq!(
        after, before,
        "the start wrote into the state directory while another hook held the session's lock"
    );
    // the lock let go, a start records the session
    start(&root, "s");
    assert!(sessions().join(format!("{}.json", stem("s"))).is_file());
}

/// Review of P5c, V1: while a stop holds the session's lock and its judge runs, another process
/// moves the state directory away and puts a fresh directory of mode 0700 at its name. The stop
/// writes nothing into the fresh directory (its writes went by the directory's name, so its
/// state went there while its lock stayed in the one moved away), and says that its state could
/// not be saved and why. First, the positive control: a stop alone writes the session's state at
/// its name.
#[test]
fn a_state_directory_replaced_while_a_stop_holds_the_lock_gets_nothing_written() {
    stop_with_its_state_directory_replaced("hook-state-dir-replaced", false);
}

/// Review of P5c round 2, M1: as the test above, with a symbolic link to another directory put
/// at the state directory's name in place of a fresh directory: nothing is written into the
/// link's target.
#[test]
fn a_state_directory_replaced_by_a_link_while_a_stop_holds_the_lock_gets_nothing_written() {
    stop_with_its_state_directory_replaced("hook-state-dir-linked", true);
}

/// The two tests above: a fresh directory (or, with `link`, a link to the directory
/// `elsewhere`) put at the state directory's name while a stop judges.
fn stop_with_its_state_directory_replaced(name: &str, link: bool) {
    use std::os::unix::fs::PermissionsExt as _;
    let scratch = Scratch::new(name);
    let root = repository(&scratch);
    let (script, log) = reading_judge(&scratch);
    start(&root, "s");
    secret(&root, "k.rs");
    let state = sessions().join(format!("{}.json", stem("s")));
    let before = std::fs::read(&state).unwrap();
    blocked(&stop_from(&root, "s", &script));
    assert_ne!(
        std::fs::read(&state).unwrap(),
        before,
        "(control) a stop alone wrote no state at its name"
    );
    // (a commit, so that the next stop has its verdict to write)
    secret(&root, "k2.rs");
    commit_all(&root, "a commit of this session");
    std::fs::write(scratch.join("judge.sleep"), "2\n").unwrap();
    let (dir, held) = (sessions(), sessions().with_file_name("hooks-held"));
    let elsewhere = scratch.join("elsewhere");
    std::fs::create_dir(&elsewhere).unwrap();
    std::fs::set_permissions(&elsewhere, std::fs::Permissions::from_mode(0o700)).unwrap();
    let judged = runs(&log);
    let stop_home = home();
    let said = std::thread::scope(|scope| {
        let stop = scope.spawn(|| {
            home_is(&stop_home);
            stop_from(&root, "s", &script)
        });
        wait_until("the stop's judging", || runs(&log) > judged);
        std::fs::rename(&dir, &held).unwrap();
        if link {
            std::os::unix::fs::symlink(&elsewhere, &dir).unwrap();
        } else {
            std::fs::create_dir(&dir).unwrap();
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        stop.join().unwrap()
    });
    std::fs::remove_file(scratch.join("judge.sleep")).unwrap();
    assert_eq!(
        (names_in(&dir), files_left_under(&elsewhere)),
        (Vec::<String>::new(), Vec::<PathBuf>::new()),
        "the stop wrote into what was put at the state directory's name while it held the lock in the one moved away: {said}"
    );
    assert!(
        stop_words(&said).contains("is no longer the directory the session's lock was taken in (another process moved it away or put another in its place while this hook ran)"),
        "{said}"
    );
    assert!(names_in(&held).contains(&format!("{}.lock", stem("s"))));
}

/// Review of P5c round 2, M1: an advice hook reads the session's state in its directory, and
/// takes the session's lock only to write. Between the two (here: while its first Git call
/// waits) the directory is moved away and a symbolic link to another directory is put at its
/// name. The lock may follow the link, as in 0.11.0; the state is not written through it: the
/// link's target stays empty, and the hook says the state directory is not a directory of its
/// own, as 0.11.0 said. First, the positive control: an advice hook alone writes the state.
#[test]
fn an_advice_hook_writes_no_state_through_a_link_put_at_the_state_directory() {
    use std::os::unix::fs::PermissionsExt as _;
    let scratch = Scratch::new("hook-advice-link");
    let root = repository(&scratch);
    start(&root, "s");
    let state = sessions().join(format!("{}.json", stem("s")));
    let before = std::fs::read(&state).unwrap();
    silence_a_lint(&root);
    let (code, _, stderr) = hook("post-tool", &[], &input("PostToolUse", &root, "s", false));
    assert_eq!((code, stderr.as_str()), (0, ""));
    assert_ne!(
        std::fs::read(&state).unwrap(),
        before,
        "(control) an advice hook alone wrote no state"
    );
    // (another finding, so that the next advice hook has something to write)
    std::fs::write(
        root.join("b.rs"),
        "fn c() {}\n#[allow(unused)]\nfn d() {}\n",
    )
    .unwrap();
    let (armed, waiting) = (scratch.join("armed"), scratch.join("waiting"));
    let first = git_with(
        &scratch,
        "git-waits",
        &format!(
            "if [ -e '{armed}' ] && [ ! -e '{waiting}' ]; then : > '{waiting}'; while [ -e '{armed}' ]; do sleep 0.05; done; fi",
            armed = armed.display(),
            waiting = waiting.display()
        ),
    );
    std::fs::write(&armed, "").unwrap();
    let (dir, elsewhere) = (sessions(), scratch.join("elsewhere"));
    std::fs::create_dir(&elsewhere).unwrap();
    std::fs::set_permissions(&elsewhere, std::fs::Permissions::from_mode(0o700)).unwrap();
    let advice_home = home();
    let said = std::thread::scope(|scope| {
        let advice = scope.spawn(|| {
            home_is(&advice_home);
            let mut dirs = vec![first.clone()];
            dirs.extend(std::env::split_paths(
                &std::env::var_os("PATH").unwrap_or_default(),
            ));
            hook_in(
                "post-tool",
                &[],
                &input("PostToolUse", &root, "s", false),
                |command| {
                    command.env("PATH", std::env::join_paths(dirs).unwrap());
                },
            )
        });
        wait_until("the advice hook's first Git call", || waiting.exists());
        std::fs::rename(&dir, dir.with_file_name("hooks-held")).unwrap();
        std::os::unix::fs::symlink(&elsewhere, &dir).unwrap();
        std::fs::remove_file(&armed).unwrap();
        advice.join().unwrap()
    });
    assert_eq!(
        files_left_under(&elsewhere),
        Vec::<PathBuf>::new(),
        "the advice hook wrote through a link put at the state directory's name: {said:?}"
    );
    assert!(
        said.1
            .contains("is not a directory of its own (a symbolic link or a file stands there)"),
        "{said:?}"
    );
}

/// Review of P5c round 2, L1: a session's lock file this user may write and not read (mode
/// 0200) is still the lock: the hook looks at it to check it is the one at its name, and does
/// not open it again to read. A stop takes the lock and blocks for a failure, as in 0.11.0;
/// the re-check that opened the file to read gave every stop `state.lock_busy` (UNJUDGED in the
/// session's memory, no block).
#[test]
fn a_lock_file_that_cannot_be_read_is_still_the_lock() {
    use std::os::unix::fs::PermissionsExt as _;
    let scratch = Scratch::new("hook-lock-0200");
    let root = repository(&scratch);
    let (script, _log) = reading_judge(&scratch);
    start(&root, "s");
    let lock = sessions().join(format!("{}.lock", stem("s")));
    std::fs::set_permissions(&lock, std::fs::Permissions::from_mode(0o200)).unwrap();
    secret(&root, "k.rs");
    let said = stop_from(&root, "s", &script);
    let reason = blocked(&said);
    assert!(
        !reason.starts_with(&stateless("state.lock_busy")),
        "a lock file that cannot be read was taken for another hook's: {reason}"
    );
}

/// Review of P5c, L1 and L2: where the session's lock file cannot be opened (a directory stands
/// at its name), no hook writes the session's state without the lock (0.11.0 wrote it), the
/// start says why it recorded nothing, and a stop says the cause first, with how to read what
/// follows it, and not that the session-start hook must be installed (it is, and it ran).
#[test]
fn a_lock_that_cannot_be_taken_keeps_no_state_and_a_stop_says_its_cause_first() {
    let scratch = Scratch::new("hook-lock-unopenable");
    let root = repository(&scratch);
    let (script, _log) = reading_judge(&scratch);
    let lock = format!("{}.lock", stem("s"));
    std::fs::create_dir_all(sessions().join(&lock)).unwrap();
    let (code, started, stderr) = hook(
        "session-start",
        &[],
        &input("SessionStart", &root, "s", false),
    );
    assert_eq!((code, stderr.as_str()), (0, ""), "{started}");
    let unlocked = |hook: &str| {
        assert_eq!(
            names_in(&sessions()),
            std::slice::from_ref(&lock),
            "{hook} wrote a state without the session's lock"
        );
    };
    unlocked("the start");
    assert!(
        started.contains("- The start of this session could not be recorded (its state could not be saved: the session's lock file "),
        "{started}"
    );
    secret(&root, "k.rs");
    let said = stop_from(&root, "s", &script);
    unlocked("the stop");
    let reason = blocked(&said);
    assert!(
        reason.starts_with("deep-diff-forge: this session's lock cannot be taken where its state is kept (the session's lock file ")
            && reason.contains("Read what follows with that: \"when this session's hooks first ran\" means this stop; a commit marked \"not this session's\" may be this session's own work"),
        "{reason}"
    );
    assert!(
        !reason.contains("the session-start hook must be installed")
            && !reason.contains("The hook could not keep its state ("),
        "{reason}"
    );
    // a session whose start was recorded, and whose lock file cannot be opened from then on:
    // its start kept a state, and the stop says so no other way
    start(&root, "r");
    let lock = sessions().join(format!("{}.lock", stem("r")));
    std::fs::remove_file(&lock).unwrap();
    std::fs::create_dir(&lock).unwrap();
    secret(&root, "k2.rs");
    let said = stop_from(&root, "r", &script);
    let reason = blocked(&said);
    assert!(
        reason.starts_with("deep-diff-forge: this session's lock cannot be taken where its state is kept (the session's lock file ")
            && reason.contains("so this stop keeps no state: until the cause is repaired")
            && !reason.contains("its start included"),
        "{reason}"
    );
}

// ---- Release 0.11.1, P5d: the hooks' state directory keeps itself out of every working tree ----

/// Where the hooks' state directory lies in another working tree of the session's project
/// (design D7 of P5d): the home directory, which holds the state, is that tree.
#[derive(Clone, Copy, Debug)]
enum OtherTree {
    /// The home directory is the main working tree; the project is a linked one.
    HomeMain,
    /// The home directory is a linked working tree; the project is the main one.
    HomeLinked,
    /// The home directory is a dotfiles repository whose linked working tree, outside the home
    /// directory, is the project.
    Dotfiles,
}

/// A layout of [`OtherTree`]: the home directory (the tree the session enters), and the
/// project the client names.
struct Laid {
    home: PathBuf,
    project: PathBuf,
}

/// Lay `case` out in `scratch` (made the home of this thread's hooks), as the design's probes
/// laid it (`probe_d7.py`): every tree with the files and the policy of [`repository`].
fn lay_other_tree(scratch: &Scratch, case: OtherTree) -> Laid {
    let main = repository(scratch);
    let laid = match case {
        OtherTree::HomeMain | OtherTree::HomeLinked => {
            let linked = scratch.join("linked");
            git(
                &main,
                &[
                    "worktree",
                    "add",
                    "--quiet",
                    "-b",
                    "side",
                    linked.to_str().unwrap(),
                ],
            );
            if let OtherTree::HomeMain = case {
                Laid {
                    home: main,
                    project: linked,
                }
            } else {
                Laid {
                    home: linked,
                    project: main,
                }
            }
        }
        OtherTree::Dotfiles => {
            std::fs::write(main.join(".bashrc"), "# dotfiles\n").unwrap();
            git(&main, &["add", ".bashrc"]);
            git(&main, &["commit", "--quiet", "-m", "dotfiles"]);
            let project = scratch.join("work/proj");
            std::fs::create_dir(scratch.join("work")).unwrap();
            git(
                &main,
                &[
                    "worktree",
                    "add",
                    "--quiet",
                    "-b",
                    "work",
                    project.to_str().unwrap(),
                ],
            );
            Laid {
                home: main,
                project,
            }
        }
    };
    home_is(&laid.home);
    laid
}

/// Run the hook `event` of `session` from `cwd` with the client naming the project of `laid`
/// (`CLAUDE_PROJECT_DIR`); a stop is given `judge`. A start must record the session, and a stop
/// gives its decision (`Null` when it says nothing).
fn hook_of(laid: &Laid, event: &str, cwd: &Path, session: &str, judge: &Path) -> Value {
    let kind = match event {
        "session-start" => "SessionStart",
        "post-tool" => "PostToolUse",
        _ => "Stop",
    };
    let extra: &[&str] = if event == "stop" {
        &["--judge", judge.to_str().unwrap()]
    } else {
        &[]
    };
    let (code, stdout, stderr) = hook_in(event, extra, &input(kind, cwd, session, false), |c| {
        c.env("CLAUDE_PROJECT_DIR", &laid.project);
    });
    assert_eq!((code, stderr.as_str()), (0, ""), "{event}: {stdout}");
    assert!(
        !stdout.contains("- The start of this session could not be recorded ("),
        "{event}: {stdout}"
    );
    if event != "stop" || stdout.is_empty() {
        Value::Null
    } else {
        serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("{e}: {stdout:?}"))
    }
}

/// The files of the patches the reading judge was shown that lie in the hooks' state
/// directory of a home directory that is the tree's top.
fn hooks_files_shown(log: &Path) -> Vec<String> {
    shown(log)
        .iter()
        .flat_map(|names| names.split(','))
        .filter(|name| name.starts_with(".local/state/deep-diff-forge/"))
        .map(str::to_owned)
        .collect()
}

/// Release 0.11.1, P5d (rows R2, R4; test T1). Where the home directory, which holds the hooks'
/// state, is another working tree of the session's project, a stop made from that tree gave the
/// judge the hooks' key, the session's state, its lock file and its tips as the agent's change
/// (0.11.0 blocked on the key's random bytes, or held, with no change of the agent's). With
/// the directory's `.gitignore` no patch the judge is given names a file of the hooks; a secret
/// the agent then writes into that tree is judged and blocks, as anywhere. (Asserted on the
/// patches' files, not on a real judge's verdict, which depends on the key's bytes.)
#[test]
fn the_hooks_files_in_another_working_tree_of_the_project_are_not_the_agents_change() {
    for case in [
        OtherTree::HomeMain,
        OtherTree::HomeLinked,
        OtherTree::Dotfiles,
    ] {
        let scratch = Scratch::new(&format!("hook-other-tree-{case:?}"));
        let laid = lay_other_tree(&scratch, case);
        let (script, log) = reading_judge(&scratch);
        hook_of(&laid, "session-start", &laid.project, "s", &script);
        hook_of(&laid, "post-tool", &laid.home, "s", &script);
        assert!(
            sessions().join("key").is_file() && laid.home.join(".git").symlink_metadata().is_ok(),
            "{case:?}: (control) the hooks did not keep their state in the home directory's working tree"
        );
        let first = hook_of(&laid, "stop", &laid.home, "s", &script);
        assert_ne!(first["decision"], "block", "{case:?}: {first}");
        assert_eq!(
            hooks_files_shown(&log),
            Vec::<String>::new(),
            "{case:?}: a judged patch named the hooks' files (no change of the agent's): {:?}",
            shown(&log)
        );
        // the agent's own change in that tree is judged as before
        secret(&laid.home, "leak.rs");
        hook_of(&laid, "post-tool", &laid.home, "s", &script);
        let after = runs(&log);
        blocked(&hook_of(&laid, "stop", &laid.home, "s", &script));
        assert!(
            runs(&log) > after
                && shown(&log)
                    .iter()
                    .any(|names| names.split(',').any(|name| name == "leak.rs")),
            "{case:?}: the secret in the home directory's tree was not judged: {:?}",
            shown(&log)
        );
        assert_eq!(
            hooks_files_shown(&log),
            Vec::<String>::new(),
            "{case:?}: a judged patch named the hooks' files beside the agent's change: {:?}",
            shown(&log)
        );
    }
}

/// Release 0.11.1, P5d (rows R2, R4; test T3; P5c's layout b): in a home directory that is a
/// dotfiles repository, its `git status` lists no file of the hooks, whichever hook was the
/// account's first (a start, an advice hook, a stop: each writes the state) and after a stop.
/// Before, it listed the key, the states and the lists, and `git add --all` committed the key
/// beside the states it keys. (The control: a file of another program in the same place is
/// listed.)
#[test]
fn a_home_repository_does_not_list_the_hooks_files() {
    let scratch = Scratch::new("hook-home-repository-ignores");
    let root = repository(&scratch);
    let (script, _) = reading_judge(&scratch);
    silence_a_lint(&root);
    for (row, first) in [
        ("a start", "session-start"),
        ("an advice hook", "post-tool"),
        ("a stop", "stop"),
    ] {
        let home = fresh_home(&scratch, &format!("home-{first}"), Above::Repository);
        let listed = || {
            git(&home, &["status", "--porcelain", "--untracked-files=all"])
                .lines()
                .filter(|line| line.contains(".local/"))
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        let kind = match first {
            "session-start" => "SessionStart",
            "post-tool" => "PostToolUse",
            _ => "Stop",
        };
        for (event, kind) in [(first, kind), ("stop", "Stop")] {
            let judged = ["--judge", script.to_str().unwrap()];
            let extra: &[&str] = if event == "stop" { &judged } else { &[] };
            let (code, stdout, stderr) = hook(event, extra, &input(kind, &root, row, false));
            assert_eq!((code, stderr.as_str()), (0, ""), "{row}: {stdout}");
            assert!(
                names_in(&sessions()).contains(&format!("{}.json", stem(row))),
                "{row}: {event} kept no state: {:?}",
                names_in(&sessions())
            );
            assert_eq!(
                listed(),
                Vec::<String>::new(),
                "{row}: after {event} the home repository lists the hooks' files"
            );
        }
        let other = home.join(".local/state/another-program/x");
        std::fs::create_dir_all(other.parent().unwrap()).unwrap();
        std::fs::write(&other, "x\n").unwrap();
        assert_eq!(
            listed(),
            ["?? .local/state/another-program/x".to_owned()],
            "{row}: (control) the home repository does not list a file there"
        );
    }
}

/// Release 0.11.1, P5d (row R5; test T5), the stated limit asserted so that a change of it is
/// seen: in the layout of T1 (the home directory the main working tree of the project), a file
/// the agent puts into the hooks' state directory there is ignored as the hooks' files are,
/// and is not judged while it is not staged (as a file under any `.gitignore` that ignores
/// itself, a limit SECURITY.md states). Once the repository's index lists it (`git add -f`) it
/// is in the next commit, and the working tree's patch holds it as the working tree has it
/// (HR-M-2): staged harmless and then given a secret in the working tree only, a stop blocks.
/// (A stop also judges the index by itself, HR4-H-1: the staged version here is harmless so
/// that only the working tree's patch can block.)
#[test]
fn a_file_put_into_the_hooks_directory_is_judged_once_staged() {
    let scratch = Scratch::new("hook-other-tree-planted");
    let laid = lay_other_tree(&scratch, OtherTree::HomeMain);
    let (script, log) = reading_judge(&scratch);
    hook_of(&laid, "session-start", &laid.project, "s", &script);
    hook_of(&laid, "post-tool", &laid.home, "s", &script);
    let planted = ".local/state/deep-diff-forge/hooks/leak.rs";
    secret(&laid.home, planted);
    let unstaged = hook_of(&laid, "stop", &laid.home, "s", &script);
    assert_ne!(
        unstaged["decision"],
        "block",
        "the stated limit changed: an unstaged file in the hooks' directory was judged: {unstaged} {:?}",
        shown(&log)
    );
    std::fs::write(laid.home.join(planted), "fn harmless() {}\n").unwrap();
    git(&laid.home, &["add", "-f", planted]);
    secret(&laid.home, planted);
    let staged = hook_of(&laid, "stop", &laid.home, "s", &script);
    assert!(
        staged["decision"] == "block"
            && shown(&log)
                .iter()
                .any(|names| names.split(',').any(|name| name == planted)),
        "a file the index lists in the hooks' directory was not judged as the working tree has it: {staged} {:?}",
        shown(&log)
    );
}

/// Release 0.11.1, P5d (rows R2, R4; test T6; N109): a state directory in the home directory's
/// tree as an earlier build leaves it (the key and another session's state and lists, no
/// `.gitignore`: the tree's `git status` lists them) gets the file at the first state this
/// build keeps there; a stop from that tree then judges the agent's change and none of the
/// directory's files, another session's included.
#[test]
fn another_sessions_files_in_the_tree_are_not_judged_after_a_kept_state() {
    let scratch = Scratch::new("hook-other-tree-earlier");
    let laid = lay_other_tree(&scratch, OtherTree::HomeMain);
    let (script, log) = reading_judge(&scratch);
    hook_of(&laid, "session-start", &laid.project, "other", &script);
    // (as an earlier build leaves it; the control below says whether it is)
    let _ = std::fs::remove_file(sessions().join(".gitignore"));
    let listed = git(
        &laid.home,
        &["status", "--porcelain", "--untracked-files=all"],
    );
    assert!(
        listed.contains(".local/state/deep-diff-forge/hooks/key")
            && listed.contains(&format!("{}.json", stem("other"))),
        "(control) the directory is not as an earlier build leaves it: {listed}"
    );
    hook_of(&laid, "session-start", &laid.project, "s", &script);
    std::fs::write(laid.home.join("note.txt"), "a harmless change\n").unwrap();
    hook_of(&laid, "post-tool", &laid.home, "s", &script);
    hook_of(&laid, "stop", &laid.home, "s", &script);
    assert!(
        shown(&log)
            .iter()
            .any(|names| names.split(',').any(|name| name == "note.txt")),
        "(control) the agent's change was not judged: {:?}",
        shown(&log)
    );
    assert_eq!(
        hooks_files_shown(&log),
        Vec::<String>::new(),
        "a judged patch named the hooks' files, another session's included: {:?}",
        shown(&log)
    );
}
