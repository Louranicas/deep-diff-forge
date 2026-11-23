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
use std::collections::BTreeMap;
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

fn check_json(home: &Path, cwd: &Path, args: &[&str], path: Option<&Path>) -> Value {
    let mut all = vec!["--json"];
    all.extend_from_slice(args);
    let (code, stdout, stderr) = check(home, cwd, &all, path);
    assert_eq!((code, stderr.as_str()), (0, ""), "{stdout}");
    serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("{e}: {stdout:?}"))
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

/// The user's Claude Code settings in `home`, registering the stop hook `agent setup --hooks`
/// prints for this binary and `judge`.
fn install(home: &Path, judge: &Path) {
    std::fs::create_dir_all(home.join(".claude")).unwrap();
    let stop = format!(
        "'{}' agent hook stop --client claude --judge '{}'",
        this_binary().display(),
        std::fs::canonicalize(judge).unwrap().display()
    );
    let settings = json!({"hooks": {"Stop": [{"hooks": [{"type": "command", "command": stop}]}]}});
    std::fs::write(home.join(".claude/settings.json"), settings.to_string()).unwrap();
}

/// Every file of the hooks' state directory: its name, then its bytes' SHA-256, size, mode,
/// inode, and modification and change times to the nanosecond.
fn state_listing(home: &Path) -> BTreeMap<String, String> {
    use std::os::unix::fs::MetadataExt as _;
    let dir = home.join(".local/state/deep-diff-forge/hooks");
    let mut listing = BTreeMap::new();
    for entry in std::fs::read_dir(&dir).unwrap() {
        let entry = entry.unwrap();
        let meta = entry.metadata().unwrap();
        let bytes = std::fs::read(entry.path()).unwrap_or_default();
        listing.insert(
            entry.file_name().to_string_lossy().into_owned(),
            format!(
                "{} {} {:o} {} {}.{} {}.{}",
                deep_diff_forge_core::sha256_hex(&bytes),
                meta.len(),
                meta.mode(),
                meta.ino(),
                meta.mtime(),
                meta.mtime_nsec(),
                meta.ctime(),
                meta.ctime_nsec()
            ),
        );
    }
    let dir_meta = std::fs::metadata(&dir).unwrap();
    listing.insert(
        ".".to_owned(),
        format!(
            "{}.{} {}.{}",
            dir_meta.mtime(),
            dir_meta.mtime_nsec(),
            dir_meta.ctime(),
            dir_meta.ctime_nsec()
        ),
    );
    listing
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

/// F3.c: with `--session`, the policy is the one at the base the session recorded, and the
/// judge is the one the registered Stop hook names, not one on `PATH`. A committed loosening
/// of the policy and a passing judge on `PATH` leave the check where the stop is: FAIL.
#[test]
fn check_uses_the_session_base_and_the_installed_judge() {
    let scratch = Scratch::new("agent-check-installed");
    let (home, root) = repository(&scratch);
    let installed = scratch.join("installed-judge");
    judge(&installed, "policy", None);
    install(&home, &installed);
    // what PATH holds: a judge that passes everything
    let bin = scratch.join("bin");
    std::fs::create_dir(&bin).unwrap();
    judge(&bin.join("forge-warden"), "pass", None);
    let (code, _, stderr) = hook(
        &home,
        "session-start",
        &[],
        &input("SessionStart", &root, "s1"),
    );
    assert_eq!((code, stderr.as_str()), (0, ""));
    let base = git(&root, &["rev-parse", "HEAD"]).trim().to_owned();
    std::fs::write(root.join("warden.toml"), "# loosened\n").unwrap();
    git(&root, &["commit", "--quiet", "-am", "loosen the policy"]);
    silence_a_lint(&root);

    let (code, stdout, _) = hook(
        &home,
        "stop",
        &["--judge", installed.to_str().unwrap()],
        &input("Stop", &root, "s1"),
    );
    assert_eq!(code, 0);
    assert!(
        stdout.contains("\"decision\":\"block\""),
        "the stop did not block: {stdout}"
    );

    let doc = check_json(&home, &root, &["--session", "s1"], Some(&bin));
    let outcome = &doc["units"][0]["outcome"];
    assert_eq!(
        outcome, "fail",
        "check says {outcome} where the stop blocked: {doc}"
    );
    assert_eq!(doc["base"], json!({"commit": base, "from": "session"}));
    assert_eq!(
        doc["judge"]["program"],
        std::fs::canonicalize(&installed).unwrap().to_str().unwrap()
    );
    assert_eq!(doc["stop"]["same_inputs"], true, "{doc}");
    assert_eq!(doc["units"][0]["rules"][0]["id"], "no-silent-suppression");
    let (_, text, _) = check(&home, &root, &["--session", "s1"], Some(&bin));
    assert!(text.starts_with("The uncommitted change: FAIL\n"), "{text}");
    assert!(
        text.contains("Judged with what a stop of this session judges the uncommitted change with"),
        "{text}"
    );
    assert!(text.contains("This is not the stop's answer"), "{text}");

    // Without a session the base is HEAD, whose policy is the loosened one: the check says
    // which base and judge it used and claims no equality.
    let doc = check_json(&home, &root, &[], Some(&bin));
    assert_eq!(doc["units"][0]["outcome"], "pass", "{doc}");
    assert_eq!(doc["base"]["from"], "HEAD");
    assert_eq!(doc["stop"]["same_inputs"], false);
    let (_, text, _) = check(&home, &root, &[], Some(&bin));
    assert!(!text.contains("Judged with what a stop"), "{text}");
    assert!(text.contains("Not as the stop: no session"), "{text}");

    // A judge named here that is not the Stop hook's is said, and the inputs are not the stop's.
    let doc = check_json(
        &home,
        &root,
        &[
            "--session",
            "s1",
            "--judge",
            bin.join("forge-warden").to_str().unwrap(),
        ],
        None,
    );
    assert_eq!(doc["stop"]["same_inputs"], false, "{doc}");
    assert_eq!(
        doc["stop"]["differences"][0],
        "--judge is not the registered Stop hook's"
    );

    // No registered Stop hook and no --judge: nothing is looked up on PATH.
    std::fs::remove_file(home.join(".claude/settings.json")).unwrap();
    // (since the second part: a check with no judge stages nothing and has no unit)
    let doc = check_json(&home, &root, &["--session", "s1"], Some(&bin));
    assert_eq!(doc["units"], json!([]), "{doc}");
    assert!(
        doc["unjudged"]
            .as_str()
            .is_some_and(|why| why.starts_with("no judge")),
        "{doc}"
    );
}

/// F3.c: the session's state is read and never written: every file of the state directory,
/// and the directory itself, are byte for byte and time for time what they were.
#[test]
fn check_leaves_the_session_state_byte_identical() {
    let scratch = Scratch::new("agent-check-read-only");
    let (home, root) = repository(&scratch);
    let installed = scratch.join("installed-judge");
    judge(&installed, "fail", None);
    install(&home, &installed);
    let (code, _, _) = hook(
        &home,
        "session-start",
        &[],
        &input("SessionStart", &root, "s1"),
    );
    assert_eq!(code, 0);
    silence_a_lint(&root);
    let (_, stdout, _) = hook(
        &home,
        "stop",
        &["--judge", installed.to_str().unwrap()],
        &input("Stop", &root, "s1"),
    );
    assert!(stdout.contains("\"decision\":\"block\""), "{stdout}");
    let before = state_listing(&home);
    assert!(
        before
            .keys()
            .any(|name| Path::new(name).extension().is_some_and(|e| e == "json")),
        "{before:?}"
    );
    // (times are compared to the nanosecond: a write within the same tick still changes ctime)
    std::thread::sleep(Duration::from_millis(20));
    let doc = check_json(&home, &root, &["--session", "s1"], None);
    assert_eq!(doc["units"][0]["outcome"], "fail", "{doc}");
    let _ = check(&home, &root, &["--session", "s1"], None);
    let _ = check(&home, &root, &["--session", "another"], None);
    assert_eq!(
        state_listing(&home),
        before,
        "agent check changed the session's state"
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

/// The index file's SHA-256 and `git status --porcelain=v2` of `root`, in that order: the index
/// is read first, before `git status` can refresh it.
fn index_and_status(root: &Path) -> (String, String) {
    let index = deep_diff_forge_core::sha256_hex(&std::fs::read(root.join(".git/index")).unwrap());
    (index, git(root, &["status", "--porcelain=v2"]))
}

/// Part 1's probe by the coordinator: a check with no judge left an empty `.git/deep-diff-forge/`.
/// A check that cannot judge makes nothing; one that judges removes its scratch and the hooks'
/// directory when it made it and left it empty; neither changes the index or the working tree.
#[test]
fn a_check_leaves_nothing_behind_and_changes_neither_index_nor_tree() {
    let scratch = Scratch::new("agent-check-leaves-nothing");
    let (home, root) = repository(&scratch);
    let fast = scratch.join("judge");
    judge(&fast, "fail", None);
    silence_a_lint(&root);
    std::fs::write(root.join("new.rs"), "fn n() {}\n").unwrap();
    git(&root, &["add", "new.rs"]);
    let _ = index_and_status(&root); // (a first `git status` may refresh the index)
    let before = index_and_status(&root);
    let own = root.join(".git/deep-diff-forge");

    // no hook registered and no --judge: nothing is staged, nothing is made
    let doc = check_json(&home, &root, &[], None);
    assert_eq!(doc["units"], json!([]), "{doc}");
    assert!(
        doc["unjudged"]
            .as_str()
            .is_some_and(|why| why.starts_with("no judge")),
        "{doc}"
    );
    assert!(
        !own.exists(),
        "a check that cannot judge made {}",
        own.display()
    );

    // with a judge: judged, and its scratch and the directory it made are gone again
    let doc = check_json(&home, &root, &["--judge", fast.to_str().unwrap()], None);
    assert_eq!(doc["units"][0]["outcome"], "fail", "{doc}");
    assert!(!own.exists(), "a check left {} behind", own.display());
    assert_eq!(
        index_and_status(&root),
        before,
        "a check changed the index or the working tree"
    );

    // a directory that was there stays, and holds no scratch of the check
    std::fs::create_dir(&own).unwrap();
    let _ = check_json(&home, &root, &["--judge", fast.to_str().unwrap()], None);
    assert!(own.is_dir(), "a check removed a directory it did not make");
    assert_eq!(std::fs::read_dir(&own).unwrap().count(), 0);
}
