//! Release 0.11.1, package L1c: the project's policy record (`.git/deep-diff-forge/policy.jsonl`),
//! the policy name bound on first use, the owner's command `agent policy-accept` (a local
//! convention, not authentication) and one snapshot at the append (a stale `pass` is never
//! recorded), with `agent verify-branch` counting a pass as a stop's frontier counts it.
//!
//! The hooks run as Claude Code runs them, against real repositories, with a stand-in judge whose
//! answer depends on the policy it is given (as in `two_policies.rs`): each line
//! `rule ID SEVERITY WORD` holds or fails a patch that adds a line with `WORD`, or for
//! `path:NAME` one that changes `NAME`; `@refuse` refuses the ranker (exit 4). `lint FILE`
//! reads a policy, and refuses one with a line `@invalid` (exit 4). While the file `hold` stands
//! in the judge's directory, every check run writes `waiting` there and waits for `go`: a stop
//! paused after its read, on a file and not on time.
#![cfg(unix)]
#![expect(
    clippy::disallowed_methods,
    reason = "integration test: drives real processes, the environment and wall-clock time"
)]

use deep_diff_forge_testkit::Scratch;
use serde_json::{Value, json};
use std::io::{BufRead as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

const ISOLATE: [(&str, &str); 2] = [
    ("GIT_CONFIG_GLOBAL", "/dev/null"),
    ("GIT_CONFIG_NOSYSTEM", "1"),
];

/// A key fails, an edit of the policy file is held.
const STRICT: &str = "rule no-secrets fail AKIA\nrule ci-is-owned hold path:warden.toml\n";
/// STRICT and a rule against the word TIGHT: a tightening.
const TIGHT: &str =
    "rule no-secrets fail AKIA\nrule ci-is-owned hold path:warden.toml\nrule tight fail TIGHT\n";
/// No rule against keys, no protection.
const LOOSE: &str = "rule no-todo hold NEVER-WRITTEN-ANYWHERE\n";
const KEY: &str = "const K: &str = \"AKIAQ7X2P4M9R3T6W8YZ\";\n";

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
            "-c",
            "protocol.file.allow=always",
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

/// The stand-in judge (see the head of this file).
const JUDGE: &str = r#"#!/bin/sh
if [ "$1" = check ] && [ "$2" = --help ]; then echo 'usage: forge-warden check --policy P --patch F --ranker R --json'; exit 0; fi
if [ "$1" = lint ]; then
  if grep -q '^@invalid' "$2"; then echo 'forge-warden: policy is not valid TOML of the forge-warden.policy.v0 shape' >&2; exit 4; fi
  echo '{"schema":"forge-warden.lint.v1"}'; exit 0
fi
policy=; patch=; ranker=
while [ $# -gt 0 ]; do
  case "$1" in
    --policy) policy="$2";;
    --patch) patch="$2";;
    --ranker) ranker="$2";;
  esac
  shift
done
if [ -e '@DIR@/hold' ] && ! grep -q '^+# deep-diff-forge' "$patch"; then
  : > '@DIR@/waiting'
  n=0; while [ ! -e '@DIR@/go' ] && [ "$n" -lt 2400 ]; do sleep 0.05; n=$((n+1)); done
fi
sum() { sha256sum < "$1" | cut -d' ' -f1; }
p="$(sum "$policy")"; f="$(sum "$patch")"; r="$(sum "$ranker")"
printf 'RUN %s %s\n' "$p" "$f" >> '@DIR@/judge.log'
if grep -q '^@refuse' "$policy"; then echo '{"exit":4,"name":"invalid_input","message":"this policy does not admit the ranker","next":"change_input"}' >&2; exit 4; fi
added="$(grep '^+' "$patch" | grep -v -e '^+++ ' -e '^+rule ' -e '^+@')"
rules=; passed=0; held=0; failed=0
while read -r kind id severity word; do
  [ "$kind" = rule ] || continue
  case "$word" in
    path:*) grep -q -e "^+++ b/${word#path:}\$" -e "^--- a/${word#path:}\$" "$patch" && hit=1 || hit=;;
    *) printf '%s\n' "$added" | grep -qF -- "$word" && hit=1 || hit=;;
  esac
  if [ -n "$hit" ]; then
    outcome="$severity"; violations="[{\"signal\":\"$id\",\"path\":\"p\",\"message\":\"m\"}]"
  else
    outcome=pass; violations='[]'
  fi
  case "$outcome" in pass) passed=$((passed+1));; hold) held=$((held+1));; fail) failed=$((failed+1));; esac
  rules="$rules${rules:+,}{\"id\":\"$id\",\"kind\":\"forbid_signal\",\"severity\":\"$severity\",\"outcome\":\"$outcome\",\"why\":\"w\",\"violations\":$violations}"
done < "$policy"
if [ "$failed" -gt 0 ]; then verdict=fail; code=7; name=policy_fail; action=change_input
elif [ "$held" -gt 0 ]; then verdict=hold; code=8; name=policy_hold; action=review_evidence
else verdict=pass; code=0; name=pass; action=proceed; fi
seals="{\"policy_sha256\":\"$p\",\"rank_sha256\":\"$f\",\"patch_sha256\":\"$f\",\"patch_facts_sha256\":null,\"provenance\":\"produced\",\"ranked_by\":{\"version\":\"stand-in\",\"sha256\":\"$r\"},\"git\":{\"path\":\"/usr/bin/git\",\"version\":\"git version 2.56.0\",\"sha256\":\"$p\",\"admission\":\"approved\",\"allowlist_sha256\":\"$f\"}}"
printf '{"schema":"forge-warden.verdict.v2","tool":{"name":"forge-warden","version":"0.2.0"},"evaluated_on":"2026-11-14","seals":%s,"verdict":"%s","exit":{"code":%s,"name":"%s","action":"%s"},"summary":{"files":1,"rules":%s,"passed":%s,"waived":0,"held":%s,"failed":%s},"rules":[%s],"expired_waivers":[],"unused_waivers":[],"notes":[]}\n' \
  "$seals" "$verdict" "$code" "$name" "$action" "$((passed+held+failed))" "$passed" "$held" "$failed" "$rules"
exit "$code"
"#;

/// Write an executable script through a child process (ETXTBSY: see `two_policies.rs`).
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

fn sha(text: &str) -> String {
    deep_diff_forge_core::sha256_hex(text.as_bytes())
}

/// What a stop says, whether it blocked or not, on one line.
fn words(decision: &Value) -> String {
    let said = decision["reason"]
        .as_str()
        .or_else(|| decision["systemMessage"].as_str())
        .unwrap_or_default();
    said.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn blocks(decision: &Value) -> bool {
    decision["decision"] == "block"
}

/// One project of a test: its repository, the home of its hooks and the stand-in judge.
struct Project {
    _scratch: Scratch,
    root: PathBuf,
    home: PathBuf,
    dir: PathBuf,
    judge: PathBuf,
}

/// A repository whose first commit has the policy file `warden.toml` with `policy`.
fn project(name: &str, policy: &str) -> Project {
    let scratch = Scratch::new(&format!("policy-record-{name}"));
    let home = scratch.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let root = scratch.join("repo");
    std::fs::create_dir(&root).unwrap();
    git(&root, &["init", "--quiet"]);
    std::fs::write(root.join("a.rs"), "fn a() {}\n").unwrap();
    std::fs::write(root.join("warden.toml"), policy).unwrap();
    git(&root, &["add", "--all"]);
    git(&root, &["commit", "--quiet", "-m", "base"]);
    let dir = scratch.join("judge-dir");
    std::fs::create_dir(&dir).unwrap();
    let judge = scratch.join("judge");
    write_program(&judge, &JUDGE.replace("@DIR@", &dir.display().to_string()));
    Project {
        _scratch: scratch,
        root,
        home,
        dir,
        judge,
    }
}

impl Project {
    fn commit(&self, files: &[(&str, &str)], message: &str) -> String {
        for (path, text) in files {
            std::fs::write(self.root.join(path), text).unwrap();
        }
        git(&self.root, &["add", "--all"]);
        git(&self.root, &["commit", "--quiet", "-m", message]);
        self.head()
    }

    fn head(&self) -> String {
        git(&self.root, &["rev-parse", "HEAD"]).trim().to_owned()
    }

    fn own(&self) -> PathBuf {
        self.root.join(".git/deep-diff-forge")
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"));
        command
            .current_dir(&self.root)
            .envs(ISOLATE)
            .env("HOME", &self.home)
            .env_remove("XDG_STATE_HOME")
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("CLAUDE_PROJECT_DIR");
        command
    }

    /// The hook `event` with `input`, given `policy` as its policy name (the default when `None`).
    fn hook_with(&self, event: &str, input: &Value, policy: Option<&str>) -> String {
        let mut command = self.command();
        command.args(["agent", "hook", event, "--client", "claude", "--judge"]);
        command.arg(&self.judge);
        if let Some(policy) = policy {
            command.args(["--policy", policy]);
        }
        let mut child = command
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
        assert_eq!(out.status.code(), Some(0), "{event}");
        String::from_utf8(out.stdout).unwrap()
    }

    fn start_with(&self, session: &str, policy: Option<&str>) -> String {
        let input = json!({"hook_event_name": "SessionStart", "session_id": session,
            "cwd": self.root, "source": "startup"});
        let said = self.hook_with("session-start", &input, policy);
        let doc: Value = serde_json::from_str(&said).unwrap_or(Value::String(said.clone()));
        let text = doc["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .map_or(said, str::to_owned);
        text.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    fn start(&self, session: &str) -> String {
        self.start_with(session, None)
    }

    fn stop_with(&self, session: &str, policy: Option<&str>) -> Value {
        let input = json!({"hook_event_name": "Stop", "session_id": session,
            "cwd": self.root, "stop_hook_active": false});
        let said = self.hook_with("stop", &input, policy);
        if said.is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&said).unwrap_or_else(|e| panic!("{e}: {said}"))
        }
    }

    fn stop(&self, session: &str) -> Value {
        self.stop_with(session, None)
    }

    fn ledger(&self) -> Vec<Value> {
        std::fs::read_to_string(self.own().join("ledger.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap_or_else(|e| panic!("{e}: {line}")))
            .collect()
    }

    fn record_text(&self) -> String {
        std::fs::read_to_string(self.own().join("policy.jsonl")).unwrap_or_default()
    }

    /// [`Project::records_of`] for a ledger that holds a line that is not text (read lossy, such
    /// lines passed over).
    fn records_of_bytes(&self, id: &str) -> Vec<(String, String, Option<String>)> {
        let unit = format!("commit:{id}");
        let bytes = std::fs::read(self.own().join("ledger.jsonl")).unwrap_or_default();
        String::from_utf8_lossy(&bytes)
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .filter(|r| r["kind"] == "stop" && r["unit"] == unit.as_str())
            .map(|r| {
                (
                    r["outcome"].as_str().unwrap().to_owned(),
                    r["class"].as_str().unwrap().to_owned(),
                    r["seals"]["policy_sha256"].as_str().map(str::to_owned),
                )
            })
            .collect()
    }

    /// The stop records of the commit `id`: (outcome, class, policy digest).
    fn records_of(&self, id: &str) -> Vec<(String, String, Option<String>)> {
        let unit = format!("commit:{id}");
        self.ledger()
            .into_iter()
            .filter(|r| r["kind"] == "stop" && r["unit"] == unit.as_str())
            .map(|r| {
                (
                    r["outcome"].as_str().unwrap().to_owned(),
                    r["class"].as_str().unwrap().to_owned(),
                    r["seals"]["policy_sha256"].as_str().map(str::to_owned),
                )
            })
            .collect()
    }

    fn verify(&self, range: &str) -> (i32, Value) {
        let out = self
            .command()
            .args(["agent", "verify-branch", range, "--json"])
            .stdin(Stdio::null())
            .output()
            .unwrap();
        let doc = serde_json::from_slice(&out.stdout)
            .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&out.stdout)));
        (out.status.code().unwrap(), doc)
    }

    /// The doctor's link `policy_accepted`: whether it is ready, and its words.
    fn accepted_link(&self) -> (bool, String) {
        let out = self
            .command()
            .args(["agent", "doctor", "--gate", "--json", "--judge"])
            .arg(&self.judge)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        let doc: Value = serde_json::from_slice(&out.stdout)
            .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&out.stdout)));
        let link = doc["checks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == "policy_accepted")
            .cloned()
            .unwrap_or_else(|| panic!("no link policy_accepted: {doc}"));
        (
            link["ok"].as_bool().unwrap(),
            link["detail"].as_str().unwrap().to_owned(),
        )
    }

    /// `agent policy-accept --commit COMMIT [EXTRA] --judge J --yes-from-pipe`, `typed` on its
    /// standard input: its exit code, standard output and standard error.
    fn accept(&self, commit: &str, extra: &[&str], typed: &str) -> (i32, String, String) {
        self.accept_as(commit, extra, typed, true)
    }

    /// [`Project::accept`], with `--yes-from-pipe` only where `pipe` says so.
    fn accept_as(
        &self,
        commit: &str,
        extra: &[&str],
        typed: &str,
        pipe: bool,
    ) -> (i32, String, String) {
        let mut command = self.command();
        command
            .args(["agent", "policy-accept", "--commit", commit])
            .args(extra)
            .arg("--judge")
            .arg(&self.judge);
        if pipe {
            command.arg("--yes-from-pipe");
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let _ = child.stdin.take().unwrap().write_all(typed.as_bytes());
        let out = child.wait_with_output().unwrap();
        (
            out.status.code().unwrap(),
            String::from_utf8(out.stdout).unwrap(),
            String::from_utf8(out.stderr).unwrap(),
        )
    }
}

/// R3 (T-22): the first hook binds the name it is given, once, and says so at the start; a hook
/// given another name passes nothing and names both names; `agent verify-branch` then exits 7.
/// The control: the bound name blocks the key.
#[test]
fn the_first_hook_binds_the_name_and_another_name_passes_nothing() {
    let p = project("bind", STRICT);
    std::fs::write(p.root.join("loose.toml"), LOOSE).unwrap();
    git(&p.root, &["add", "--all"]);
    git(
        &p.root,
        &["commit", "--quiet", "-m", "a lenient file beside it"],
    );
    let base = p.head();
    let said = p.start("k0");
    assert!(
        said.contains("- This start bound the policy name \"warden.toml\" to this project, on first use, in its policy record (a file of the repository's Git directory that any process of this user can write): stops given another policy name pass nothing."),
        "{said}"
    );
    let record = p.record_text();
    assert_eq!(record.lines().count(), 1, "{record}");
    let binding: Value = serde_json::from_str(record.lines().next().unwrap()).unwrap();
    assert_eq!(
        (
            &binding["kind"],
            &binding["policy"],
            &binding["baseline_head"],
            &binding["policy_sha256"]
        ),
        (
            &json!("binding"),
            &json!("warden.toml"),
            &json!(base),
            &json!(sha(STRICT))
        ),
        "{binding}"
    );
    // a second start binds nothing and says nothing of it
    let again = p.start("k1");
    assert!(!again.contains("This start bound"), "{again}");
    assert_eq!(p.record_text(), record);

    // the attack: a session whose hooks are given the lenient file commits a key
    let said = p.start_with("k2", Some("loose.toml"));
    assert!(
        said.contains("these hooks are given the policy name \"loose.toml\"; this project's policy record binds \"warden.toml\" (a file any process of this user can write): no change passes until the two names match"),
        "{said}"
    );
    let key = p.commit(&[("k.md", KEY)], "a key");
    let decision = p.stop_with("k2", Some("loose.toml"));
    assert!(
        words(&decision).contains("these hooks are given the policy name \"loose.toml\"; this project's policy record binds \"warden.toml\""),
        "{decision}"
    );
    let records = p.records_of(&key);
    assert!(!records.is_empty(), "{:?}", p.ledger());
    assert!(
        records
            .iter()
            .all(|(outcome, class, _)| outcome == "unjudged" && class == "chain.policy_missing"),
        "{records:?}"
    );
    assert_eq!(p.verify(&format!("{base}..HEAD")).0, 7);
    // the control: the bound name fails the same key
    p.start("k3");
    let decision = p.stop("k3");
    assert!(blocks(&decision), "{decision}");
}

/// The repository of the acceptance tests: the first commit has TIGHT, the second makes it STRICT
/// (the policy the ledger first records), and a session's start binds `warden.toml`. Returns the
/// project, the TIGHT commit and the STRICT commit.
fn tightenable(name: &str) -> (Project, String, String) {
    let p = project(name, TIGHT);
    let tight = p.head();
    let strict = p.commit(
        &[("warden.toml", STRICT)],
        "the policy the ledger first records",
    );
    p.start("k0");
    (p, tight, strict)
}

/// R4: an acceptance through the command, typed in full, makes the policy at `C` the accepted one
/// (the doctor says so), and units are judged under it; nothing is verified by it (R11). Before
/// it, the start names the bound name and the accepted policy as the ledger's first record's.
#[test]
fn an_acceptance_puts_a_policy_into_effect_and_verifies_nothing() {
    let (p, tight, strict) = tightenable("accept");
    let (ok, detail) = p.accepted_link();
    assert!(ok, "{detail}");
    assert!(
        detail.contains(&format!(
            "\"warden.toml\" is bound; the accepted policy is the one at commit {} (by the project's first record)",
            &strict[..7]
        )) && detail.contains("a file any process running as this user can write: a local convention, not authentication"),
        "{detail}"
    );
    // a commit that STRICT passes and TIGHT fails, made before the acceptance, stops and passes
    p.start("k1");
    let early = p.commit(&[("e.md", "TIGHT word\n")], "under STRICT");
    assert!(!blocks(&p.stop("k1")));
    let (code, out, err) = p.accept(&tight, &[], &format!("{}\n", sha(TIGHT)));
    assert_eq!(code, 0, "{out}{err}");
    assert!(
        out.contains("is now this project's accepted policy"),
        "{out}"
    );
    assert!(out.contains("A policy acceptance record is a local convention, not authentication: any process running as this user can run this command or write the record it appends. The judge on the pull request, under the target branch's policy, is the authority."), "{out}");
    assert!(
        out.contains("+rule tight fail TIGHT"),
        "the whole diff: {out}"
    );
    let (ok, detail) = p.accepted_link();
    assert!(
        ok && detail.contains(&format!(
            "the accepted policy is the one at commit {} (by an acceptance record, a local convention, of commit {})",
            &tight[..7],
            &tight[..7]
        )),
        "{detail}"
    );
    // a later commit with TIGHT now fails under the accepted policy (half 2)
    p.start("k2");
    let late = p.commit(&[("l.md", "TIGHT again\n")], "under TIGHT");
    let decision = p.stop("k2");
    assert!(blocks(&decision), "{decision}");
    assert!(p.records_of(&late).iter().any(|r| r.0 == "fail"));
    // R11: the earlier pass, sealed to STRICT (accepted then), keeps its commit verified; the
    // acceptance verified nothing and judged nothing again by itself
    assert!(
        p.records_of(&early).iter().all(|r| r.0 == "pass"),
        "{:?}",
        p.records_of(&early)
    );
    let (code, doc) = p.verify(&format!("{strict}..{early}"));
    assert_eq!(code, 0, "{doc}");
}

/// T-3, the compare-and-swap: a record whose `from_sha256` is not the accepted digest at its
/// place, one whose commit is not an ancestor of HEAD, and one whose `to_sha256` is not the bytes
/// at its commit are ignored by every reader, and the doctor names each with why. A record
/// written by hand that does apply takes effect: the stated limit (a local convention).
#[test]
fn an_acceptance_that_does_not_apply_is_ignored_and_named() {
    let (p, tight, strict) = tightenable("cas");
    let side = {
        git(&p.root, &["checkout", "--quiet", "-b", "side"]);
        let side = p.commit(&[("warden.toml", LOOSE)], "side: loose");
        git(&p.root, &["checkout", "--quiet", "main"]);
        side
    };
    let line = |from: &str, to: &str, commit: &str| {
        json!({"v": 1, "kind": "acceptance", "policy": "warden.toml", "from_sha256": from,
            "to_sha256": to, "commit": commit})
        .to_string()
    };
    let mut text = p.record_text();
    text.push_str(&line(&sha(LOOSE), &sha(TIGHT), &tight));
    text.push('\n');
    text.push_str(&line(&sha(STRICT), &sha(LOOSE), &side));
    text.push('\n');
    text.push_str(&line(&sha(STRICT), &sha(LOOSE), &tight));
    text.push('\n');
    std::fs::write(p.own().join("policy.jsonl"), &text).unwrap();
    let (ok, detail) = p.accepted_link();
    assert!(!ok, "{detail}");
    for why in [
        // (in this tree's history and not from the policy reached: a disagreement, ruling 10 F2)
        "the acceptance record on line 2 is ignored: it is in this tree's history and does not follow from the accepted policy reached before it",
        &format!(
            "the acceptance record on line 3 does not apply here and is harmless: its commit {} is not an ancestor of HEAD",
            &side[..12]
        ),
        &format!(
            "the acceptance record on line 4 is ignored: the file \"warden.toml\" at its commit {} is not the bytes its to_sha256 names",
            &tight[..12]
        ),
    ] {
        assert!(detail.contains(why), "{why}: {detail}");
    }
    assert!(
        detail.contains(&format!("the one at commit {}", &strict[..7])),
        "{detail}"
    );
    // a key is still failed under the accepted policy, never under LOOSE
    p.start("k1");
    p.commit(&[("k.md", KEY)], "a key");
    assert!(blocks(&p.stop("k1")));

    // a record written by hand that applies takes effect (the stated limit)
    text.push_str(&line(&sha(STRICT), &sha(TIGHT), &tight));
    text.push('\n');
    std::fs::write(p.own().join("policy.jsonl"), &text).unwrap();
    let (_, detail) = p.accepted_link();
    assert!(
        detail.contains(&format!(
            "the accepted policy is the one at commit {}",
            &tight[..7]
        )),
        "{detail}"
    );
    let reference = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/reference/agent-hooks.md"),
    )
    .unwrap();
    let flat = reference.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        flat.contains("An acceptance record that a process of this user writes by hand, and that applies, takes effect as one the command appended: the policy record is a local convention, not authentication."),
        "agent-hooks.md must state the limit"
    );
}

/// Every refusal of the command, each beside the valid form of the same case, and nothing
/// written by any refusal: a standard input that is not a terminal without `--yes-from-pipe`; a
/// typed digest that is wrong; a commit that is not an ancestor of HEAD; no file of the name at
/// the commit; a file the judge does not read; no binding yet.
#[test]
fn every_refusal_of_the_command_writes_nothing() {
    let (p, tight, _) = tightenable("refusals");
    let record = p.record_text();
    let typed = format!("{}\n", sha(TIGHT));
    // not a terminal, no option: refused (exit 2); the same with the option is the control below
    let (code, _, err) = p.accept_as(&tight, &[], &typed, false);
    assert_eq!(code, 2, "{err}");
    assert!(err.contains("standard input is not a terminal"), "{err}");
    // a wrong digest
    let (code, _, err) = p.accept(&tight, &[], &format!("{}\n", sha(LOOSE)));
    assert_eq!(code, 4, "{err}");
    assert!(
        err.contains("the digest typed is not the digest of the policy to accept"),
        "{err}"
    );
    let (code, _, err) = p.accept(&tight, &[], "\n");
    assert_eq!(code, 4, "{err}");
    // not an ancestor of HEAD
    git(&p.root, &["checkout", "--quiet", "-b", "side"]);
    let side = p.commit(&[("warden.toml", TIGHT)], "side: tight");
    git(&p.root, &["checkout", "--quiet", "main"]);
    let (code, _, err) = p.accept(&side, &[], &typed);
    assert_eq!(code, 4, "{err}");
    assert!(err.contains("is not an ancestor of HEAD"), "{err}");
    // no file of the name at the commit
    let (code, _, err) = p.accept(&tight, &["--policy", "other.toml"], &typed);
    assert_eq!(code, 4, "{err}");
    assert!(err.contains("has no file \"other.toml\""), "{err}");
    // a file the judge does not read
    let bad = p.commit(
        &[("warden.toml", &format!("{TIGHT}@invalid\n"))],
        "a policy that does not parse",
    );
    let (code, _, err) = p.accept(
        &bad,
        &[],
        &format!("{}\n", sha(&format!("{TIGHT}@invalid\n"))),
    );
    assert_eq!(code, 4, "{err}");
    assert!(err.contains("does not read the policy to accept"), "{err}");
    assert_eq!(p.record_text(), record, "a refusal wrote to the record");
    // the control: the valid form appends one record
    let (code, out, err) = p.accept(&tight, &[], &typed);
    assert_eq!(code, 0, "{out}{err}");
    assert_eq!(p.record_text().lines().count(), record.lines().count() + 1);
    // no binding yet: refused
    let q = project("refusals-unbound", TIGHT);
    let first = q.head();
    let (code, _, err) = q.accept(&first, &[], &typed);
    assert_eq!(code, 4, "{err}");
    assert!(err.contains("no record of the project yet"), "{err}");
    assert!(!q.own().join("policy.jsonl").exists());
}

/// Two commands in a race (R4, the compare-and-swap of the append): each shows the same accepted
/// digest; the first to append applies; the second, whose shown state is gone, is refused and
/// writes nothing. Exactly one applies.
#[test]
fn two_commands_in_a_race_exactly_one_applies() {
    let (p, tight, strict) = tightenable("race-commands");
    let loose = p.commit(&[("warden.toml", LOOSE)], "loose");
    let spawn = |commit: &str| -> (Child, std::io::BufReader<std::process::ChildStdout>) {
        let mut child = p
            .command()
            .args([
                "agent",
                "policy-accept",
                "--commit",
                commit,
                "--yes-from-pipe",
                "--judge",
            ])
            .arg(&p.judge)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut out = std::io::BufReader::new(child.stdout.take().unwrap());
        // shown: the command waits for the digest, holding no lock
        let mut line = String::new();
        while out.read_line(&mut line).unwrap() > 0 {
            if line.contains("is the authority.") {
                break;
            }
        }
        assert!(line.contains("is the authority."), "{line}");
        (child, out)
    };
    let (mut first, _) = spawn(&tight);
    let (mut second, _) = spawn(&loose);
    let finish = |child: &mut Child, typed: &str| {
        child
            .stdin
            .take()
            .unwrap()
            .write_all(typed.as_bytes())
            .unwrap();
        let status = child.wait().unwrap();
        let mut err = String::new();
        std::io::Read::read_to_string(child.stderr.as_mut().unwrap(), &mut err).unwrap();
        (status.code().unwrap(), err)
    };
    let (one, _) = finish(&mut first, &format!("{}\n", sha(TIGHT)));
    let (two, err) = finish(&mut second, &format!("{}\n", sha(LOOSE)));
    assert_eq!((one, two), (0, 4), "{err}");
    assert!(
        err.contains("this project's accepted policy changed since it was shown"),
        "{err}"
    );
    let acceptances = p
        .record_text()
        .lines()
        .filter(|l| l.contains("\"acceptance\""))
        .count();
    assert_eq!(acceptances, 1);
    let (_, detail) = p.accepted_link();
    assert!(
        detail.contains(&format!("the one at commit {}", &tight[..7])),
        "{detail}; strict {strict}"
    );
}

/// Ruling 4, the snapshot at the append, the first order: a stop paused after its read (the judge
/// waits on a file), an acceptance of a tighter policy through the command, the stop resumed. The
/// stop's pass under the old accepted policy is not recorded: its units are UNJUDGED naming the
/// change, the session does not remember the commit as passed, and the next stop fails it under
/// the new policy.
#[test]
fn a_stop_that_judged_under_a_replaced_policy_appends_no_pass() {
    let (p, tight, _) = tightenable("snapshot");
    p.start("k1");
    let m = p.commit(&[("m.md", "TIGHT but STRICT allows it\n")], "M");
    std::fs::write(p.dir.join("hold"), "").unwrap();
    let decision = std::thread::scope(|scope| {
        let stop = scope.spawn(|| p.stop("k1"));
        // the positive control: the judge was reached and waits
        let mut waited = 0;
        while !p.dir.join("waiting").exists() && waited < 1200 {
            std::thread::sleep(std::time::Duration::from_millis(50));
            waited += 1;
        }
        assert!(
            p.dir.join("waiting").exists(),
            "the stop never reached the judge"
        );
        let (code, out, err) = p.accept(&tight, &[], &format!("{}\n", sha(TIGHT)));
        assert_eq!(code, 0, "{out}{err}");
        std::fs::remove_file(p.dir.join("hold")).unwrap();
        std::fs::write(p.dir.join("go"), "").unwrap();
        stop.join().unwrap()
    });
    let records = p.records_of(&m);
    assert!(!records.is_empty(), "{:?}", p.ledger());
    assert!(
        records.iter().all(|r| r.0 != "pass"),
        "a stale pass was recorded: {records:?}"
    );
    assert!(
        records
            .iter()
            .any(|r| r.0 == "unjudged" && r.1 == "chain.policy_missing"),
        "{records:?}"
    );
    let said = words(&decision);
    assert!(
        said.contains("UNJUDGED: this project's accepted policy changed while this stop judged: it was the one at commit")
            && said.contains(&format!("and it is now the one at commit {}. What passed at this stop was judged under a policy that is no longer the accepted one, so it is recorded as UNJUDGED, not as a pass: stop again to have it judged under the policy accepted now", &tight[..7])),
        "{decision}"
    );
    // judged again at the next stop, under the policy accepted now: it fails
    std::fs::remove_file(p.dir.join("go")).unwrap();
    let again = p.stop("k1");
    assert!(blocks(&again), "{again}");
    assert!(
        p.records_of(&m).iter().any(|r| r.0 == "fail"),
        "{:?}",
        p.records_of(&m)
    );
}

/// A stop of `session` paused at its first judge run (the judge waits on a file), `during` run
/// while it waits, then let go: what the stop decided.
fn paused_stop(p: &Project, session: &str, during: impl FnOnce()) -> Value {
    std::fs::write(p.dir.join("hold"), "").unwrap();
    let decision = std::thread::scope(|scope| {
        let stop = scope.spawn(|| p.stop(session));
        let mut waited = 0;
        while !p.dir.join("waiting").exists() && waited < 1200 {
            std::thread::sleep(std::time::Duration::from_millis(50));
            waited += 1;
        }
        assert!(
            p.dir.join("waiting").exists(),
            "the stop never reached the judge"
        );
        during();
        std::fs::remove_file(p.dir.join("hold")).unwrap();
        std::fs::write(p.dir.join("go"), "").unwrap();
        stop.join().unwrap()
    });
    for file in ["waiting", "go"] {
        let _ = std::fs::remove_file(p.dir.join(file));
    }
    decision
}

/// One probe per exit path of the comparison at the append (ruling 4; N-class of 2026-11-23),
/// beside the first order above (the derived snapshot differs): a hand-written record that does
/// not apply changes the file and not the snapshot, so the pass is recorded; the project lock held
/// by another past the stop's wait, the ledger that cannot be read again, and a policy record that
/// cannot be read whole at the re-read each record no pass, and say why.
#[test]
fn every_exit_of_the_comparison_at_the_append_is_probed() {
    let ignored_line = |p: &Project| {
        let mut text = p.record_text();
        text.push_str(
            &json!({"v": 1, "kind": "acceptance", "policy": "warden.toml",
                "from_sha256": sha(LOOSE), "to_sha256": sha(TIGHT), "commit": "f".repeat(40)})
            .to_string(),
        );
        text.push('\n');
        std::fs::write(p.own().join("policy.jsonl"), text).unwrap();
    };
    let passes = |p: &Project, id: &str| p.records_of(id).iter().filter(|r| r.0 == "pass").count();
    // (b) the inputs changed, the snapshot did not: recorded as judged
    let (p, _, _) = tightenable("exit-b");
    p.start("k1");
    let m = p.commit(&[("m.md", "fn m() {}\n")], "M");
    let decision = paused_stop(&p, "k1", || ignored_line(&p));
    assert!(
        !words(&decision).contains("changed while this stop judged"),
        "{decision}"
    );
    assert_eq!(passes(&p, &m), 1, "{:?}", p.records_of(&m));
    // (d) the project lock held by another past the stop's wait
    let (p, _, _) = tightenable("exit-d");
    p.start("k1");
    let m = p.commit(&[("m.md", "fn m() {}\n")], "M");
    let lock_file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(p.own().join("project.lock"))
        .unwrap();
    let held = std::cell::RefCell::new(None);
    let decision = paused_stop(&p, "k1", || {
        let lock = nix::fcntl::Flock::lock(lock_file, nix::fcntl::FlockArg::LockExclusiveNonblock)
            .unwrap_or_else(|_| panic!("the test could not take the project lock"));
        *held.borrow_mut() = Some(lock);
    });
    drop(held);
    assert!(
        words(&decision).contains("UNJUDGED: this project's policy record could not be read again before this stop's records were made"),
        "{decision}"
    );
    assert_eq!(passes(&p, &m), 0, "{:?}", p.records_of(&m));
    // (e) the ledger cannot be read again (a byte that is not text appended while the judge
    // ran): the arm (readable, then not); the records can still be appended, so the count of
    // passes holds the exit
    let (p, _, _) = tightenable("exit-e");
    p.start("k1");
    let m = p.commit(&[("m.md", "fn m() {}\n")], "M");
    let ledger = p.own().join("ledger.jsonl");
    let decision = paused_stop(&p, "k1", || {
        let mut bytes = std::fs::read(&ledger).unwrap();
        bytes.extend_from_slice(b"\xff\n");
        std::fs::write(&ledger, bytes).unwrap();
    });
    assert!(
        words(&decision).contains("UNJUDGED: this project's ledger could not be read again before this stop's records were made"),
        "{decision}"
    );
    assert!(!p.records_of_bytes(&m).is_empty(), "nothing was recorded");
    assert_eq!(
        p.records_of_bytes(&m)
            .iter()
            .filter(|r| r.0 == "pass")
            .count(),
        0,
        "{:?}",
        p.records_of_bytes(&m)
    );
    // (g) the ledger cannot be read at either read (ruling 10, F1): the accepted policy cannot be
    // had, nothing passes, and the words name the ledger
    let (p, _, _) = tightenable("exit-g");
    p.start("k1");
    let m = p.commit(&[("m.md", "fn m() {}\n")], "M");
    let mut bytes = std::fs::read(ledger_of(&p)).unwrap();
    bytes.extend_from_slice(b"\xff\n");
    std::fs::write(ledger_of(&p), bytes).unwrap();
    let decision = p.stop("k1");
    assert!(
        words(&decision).contains("this project's ledger cannot be read whole"),
        "{decision}"
    );
    assert!(!p.records_of_bytes(&m).is_empty(), "nothing was recorded");
    assert!(
        p.records_of_bytes(&m).iter().all(|r| r.0 != "pass"),
        "{:?}",
        p.records_of_bytes(&m)
    );
    // (f) the policy record cannot be read whole at the re-read
    let (p, _, _) = tightenable("exit-f");
    p.start("k1");
    let m = p.commit(&[("m.md", "fn m() {}\n")], "M");
    let decision = paused_stop(&p, "k1", || {
        let mut text = p.record_text();
        text.push_str("{\"v\":1,\"kind\":\"acceptance\"}\n");
        std::fs::write(p.own().join("policy.jsonl"), text).unwrap();
    });
    assert!(
        words(&decision).contains("UNJUDGED: this project's accepted policy changed while this stop judged: it was the one at commit")
            && words(&decision).contains("and it is now one that cannot be read (this project's policy record cannot be read whole"),
        "{decision}"
    );
    assert_eq!(passes(&p, &m), 0, "{:?}", p.records_of(&m));
}

/// Ruling 4, the other order: the command paused between showing and appending (it waits for the
/// digest) while a stop runs whole: the stop is not held up by it (no lock is held while the
/// command waits) and records under the accepted policy of its time; the command then applies.
#[test]
fn a_command_waiting_for_its_digest_holds_no_lock() {
    let (p, tight, strict) = tightenable("command-waits");
    let mut child = p
        .command()
        .args([
            "agent",
            "policy-accept",
            "--commit",
            &tight,
            "--yes-from-pipe",
            "--judge",
        ])
        .arg(&p.judge)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut out = std::io::BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    while out.read_line(&mut line).unwrap() > 0 && !line.contains("is the authority.") {}
    assert!(line.contains("is the authority."), "{line}");
    p.start("k1");
    let m = p.commit(&[("m.md", "fn m() {}\n")], "M");
    let decision = p.stop("k1");
    assert!(
        !words(&decision).contains("could not be read"),
        "{decision}"
    );
    assert!(
        p.records_of(&m)
            .iter()
            .any(|r| r.0 == "pass" && r.2.as_deref() == Some(&sha(STRICT))),
        "{:?}",
        p.records_of(&m)
    );
    child
        .stdin
        .take()
        .unwrap()
        .write_all(format!("{}\n", sha(TIGHT)).as_bytes())
        .unwrap();
    assert_eq!(child.wait().unwrap().code(), Some(0));
    let (_, detail) = p.accepted_link();
    assert!(
        detail.contains(&format!("the one at commit {}", &tight[..7])),
        "{detail}"
    );
    // the pass of its time stays a pass: STRICT was the accepted policy when it was recorded
    assert_eq!(p.verify(&format!("{strict}..{m}")).0, 0);
}

/// Two stops of two sessions and a command at once cannot deadlock: each takes the session lock,
/// then the project lock, then the ledger's own; the command the project lock alone; every wait
/// is bounded. All three end, the ledger holds whole lines, and the record one acceptance.
#[test]
fn two_stops_and_a_command_at_once_all_end() {
    let (p, tight, _) = tightenable("no-deadlock");
    p.start("k1");
    p.start("k2");
    p.commit(&[("m.md", "fn m() {}\n")], "M");
    let (decisions, accepted) = std::thread::scope(|scope| {
        let one = scope.spawn(|| p.stop("k1"));
        let two = scope.spawn(|| p.stop("k2"));
        let command = scope.spawn(|| p.accept(&tight, &[], &format!("{}\n", sha(TIGHT))));
        (
            [one.join().unwrap(), two.join().unwrap()],
            command.join().unwrap(),
        )
    });
    assert_eq!(accepted.0, 0, "{accepted:?}");
    for decision in &decisions {
        assert!(!blocks(decision), "{decision}");
    }
    let text = std::fs::read_to_string(p.own().join("ledger.jsonl")).unwrap();
    assert!(text.ends_with('\n'));
    for line in text.lines() {
        serde_json::from_str::<Value>(line).unwrap_or_else(|e| panic!("{e}: {line}"));
    }
    let record = p.record_text();
    assert_eq!(
        record
            .lines()
            .filter(|l| l.contains("\"acceptance\""))
            .count(),
        1,
        "{record}"
    );
}

/// The record's file under deliberate acts of the same user, stated as what they are: removed
/// (the ledger's first record's policy is accepted again, and a key still fails under half 1 and
/// half 2); a record cut in its middle (passed over as not whole, named by the doctor; stops still
/// judge); a record that is JSON and not whole (the file cannot be read whole: no change passes,
/// and the words name the file); the ledger removed while the record stands (the two disagree: no
/// change passes).
#[test]
fn the_record_under_acts_of_the_same_user() {
    let (p, tight, strict) = tightenable("acts");
    let (code, out, err) = p.accept(&tight, &[], &format!("{}\n", sha(TIGHT)));
    assert_eq!(code, 0, "{out}{err}");
    let whole = p.record_text();
    // cut in the middle of the acceptance: passed over, the first record's policy again
    let cut = &whole[..whole.len() - 20];
    std::fs::write(p.own().join("policy.jsonl"), cut).unwrap();
    let (ok, detail) = p.accepted_link();
    assert!(
        !ok && detail
            .contains("its line 2 is passed over: it is not whole (a write that did not finish)")
            && detail.contains(&format!("the one at commit {}", &strict[..7])),
        "{detail}"
    );
    p.start("k1");
    p.commit(&[("k.md", KEY)], "a key");
    assert!(blocks(&p.stop("k1")));
    // a JSON line that is not a whole record: the file cannot be read whole
    std::fs::write(
        p.own().join("policy.jsonl"),
        format!("{whole}{{\"v\":1,\"kind\":\"acceptance\"}}\n"),
    )
    .unwrap();
    p.start("k2");
    std::fs::write(p.root.join("b.rs"), "fn b() {}\n").unwrap();
    let before = p.ledger().len();
    let decision = p.stop("k2");
    assert!(
        words(&decision)
            .contains("this project's policy record cannot be read whole (the policy record")
            && words(&decision).contains("line 3 is not a record of this build"),
        "{decision}"
    );
    let written: Vec<Value> = p.ledger().into_iter().skip(before).collect();
    assert!(
        written.iter().any(|r| r["kind"] == "stop")
            && written
                .iter()
                .all(|r| r["kind"] != "stop" || r["outcome"] != "pass"),
        "{written:?}"
    );
    // removed: the first record's policy is accepted again
    std::fs::remove_file(p.own().join("policy.jsonl")).unwrap();
    let (ok, detail) = p.accepted_link();
    assert!(
        ok && detail.contains(&format!("the one at commit {}", &strict[..7])),
        "{detail}"
    );
    // the ledger removed while the record stands: the two disagree
    std::fs::write(p.own().join("policy.jsonl"), &whole).unwrap();
    std::fs::remove_file(p.own().join("ledger.jsonl")).unwrap();
    p.start("k3");
    std::fs::write(p.root.join("c.rs"), "fn c() {}\n").unwrap();
    let decision = p.stop("k3");
    assert!(
        words(&decision)
            .contains("this project's policy record and its ledger disagree, so the accepted policy cannot be read"),
        "{decision}"
    );
    assert!(
        p.ledger()
            .iter()
            .all(|r| r["kind"] != "stop" || r["outcome"] != "pass"),
        "{:?}",
        p.ledger()
    );
}

/// Ruling 6, F5: `agent verify-branch` counts a pass as a stop's frontier counts it. A pass sealed
/// to a policy that was never accepted does not verify its commit (and the output says so); once
/// that policy is accepted it does. The published 0.11.0 counts every pass (stated).
#[test]
fn verify_branch_counts_a_pass_only_sealed_to_an_accepted_policy() {
    let (p, tight, strict) = tightenable("verify");
    let m = p.commit(&[("m.md", "fn m() {}\n")], "M");
    // a pass of M sealed to TIGHT, which was never the accepted policy (as 0.11.0 could write)
    let mut ledger = std::fs::read_to_string(p.own().join("ledger.jsonl")).unwrap();
    let start = p
        .ledger()
        .into_iter()
        .find(|r| r["kind"] == "start")
        .unwrap();
    ledger.push_str(
        &json!({"v": 1, "kind": "stop", "session": start["session"], "worktree": null,
            "unit": format!("commit:{m}"), "outcome": "pass", "class": "pass", "blocked_turns": 0,
            "agent_checks": {"runs": 0, "failed": 0}, "ignored_elsewhere": null,
            "seals": {"policy_sha256": sha(TIGHT), "ranker_sha256": "a".repeat(64),
                "patch_sha256": "b".repeat(64), "verdict_sha256": "c".repeat(64)}})
        .to_string(),
    );
    ledger.push('\n');
    std::fs::write(p.own().join("ledger.jsonl"), ledger).unwrap();
    let (code, doc) = p.verify(&format!("{strict}..{m}"));
    assert_eq!(code, 7, "{doc}");
    assert_eq!(doc["commits"][0]["passes_not_accepted"], 1, "{doc}");
    assert!(
        doc["accepted_policy"].as_str().is_some_and(|w| w.contains("A pass counts only when it is sealed to a policy that was this project's accepted policy")),
        "{doc}"
    );
    // accepted: the same record verifies
    let (code, out, err) = p.accept(&tight, &[], &format!("{}\n", sha(TIGHT)));
    assert_eq!(code, 0, "{out}{err}");
    let (code, doc) = p.verify(&format!("{strict}..{m}"));
    assert_eq!(code, 0, "{doc}");
    // an accepted policy that cannot be read: nothing verifies, and the output says why
    std::fs::write(p.own().join("policy.jsonl"), "{\"v\":1}\n").unwrap();
    let (code, doc) = p.verify(&format!("{strict}..{m}"));
    assert_eq!(code, 7, "{doc}");
    assert!(
        doc["unverified"]
            .as_str()
            .is_some_and(|w| w.contains("the project's accepted policy cannot be read")),
        "{doc}"
    );
}

/// The pin update that held a project for ever under L1b alone: the accepted policy refuses this
/// ranker (`@refuse`), so a unit it does not fail is UNJUDGED; once the policy that admits it is
/// accepted through the command, the same work is judged and passes.
#[test]
fn a_pin_update_is_accepted_and_the_project_is_judged_again() {
    let p = project("pin", &format!("{STRICT}@refuse\n"));
    p.start("k0");
    let admits = p.commit(&[("warden.toml", STRICT)], "the pin update");
    p.start("k1");
    std::fs::write(p.root.join("b.rs"), "fn b() {}\n").unwrap();
    let decision = p.stop("k1");
    assert!(words(&decision).contains("UNJUDGED"), "{decision}");
    let (code, out, err) = p.accept(&admits, &[], &format!("{}\n", sha(STRICT)));
    assert_eq!(code, 0, "{out}{err}");
    p.start("k2");
    let decision = p.stop("k2");
    assert!(!words(&decision).contains("UNJUDGED"), "{decision}");
    let trees: Vec<Value> = p
        .ledger()
        .into_iter()
        .filter(|r| {
            r["kind"] == "stop" && r["unit"].as_str().is_some_and(|u| u.starts_with("tree:"))
        })
        .collect();
    assert!(
        trees.last().is_some_and(|r| r["outcome"] == "pass"),
        "{trees:?}"
    );
}

/// Ruling 7 (decision 1): a torn last line of the policy record (a write that a crash cut) never
/// makes a stop UNJUDGED and never applies half a record: the cut acceptance is passed over, the
/// accepted policy stays the one before it, and the doctor names the line.
#[test]
fn a_torn_last_line_neither_blocks_judging_nor_applies() {
    let (p, tight, strict) = tightenable("torn");
    let loose = p.commit(&[("warden.toml", LOOSE)], "loose");
    let whole = p.record_text();
    let cut = json!({"v": 1, "kind": "acceptance", "policy": "warden.toml",
        "from_sha256": sha(STRICT), "to_sha256": sha(LOOSE), "commit": loose})
    .to_string();
    // every cut of the record short of its closing brace, the last line of the file
    for at in [cut.len() - 1, cut.len() / 2, 5] {
        std::fs::write(
            p.own().join("policy.jsonl"),
            format!("{whole}{}", &cut[..at]),
        )
        .unwrap();
        let (ok, detail) = p.accepted_link();
        assert!(
            !ok && detail.contains(
                "its line 2 is passed over: it is not whole (a write that did not finish)"
            ) && detail.contains(&format!(
                "the accepted policy is the one at commit {}",
                &strict[..7]
            )),
            "{at}: {detail}"
        );
    }
    // a stop judges, under the policy before the cut record: the key still fails, nothing is UNJUDGED
    p.start("k1");
    let key = p.commit(&[("k.md", KEY)], "a key");
    let decision = p.stop("k1");
    assert!(blocks(&decision), "{decision}");
    assert!(!words(&decision).contains("UNJUDGED"), "{decision}");
    assert!(
        p.records_of(&key).iter().any(|r| r.0 == "fail"),
        "{:?}",
        p.records_of(&key)
    );
    // the next writer begins on a line of its own, and the cut line stays passed over
    let (code, out, err) = p.accept(&tight, &[], &format!("{}\n", sha(TIGHT)));
    assert_eq!(code, 0, "{out}{err}");
    let (ok, detail) = p.accepted_link();
    assert!(
        !ok && detail.contains(&format!(
            "the accepted policy is the one at commit {}",
            &tight[..7]
        )),
        "{detail}"
    );
}

/// Ruling 8 (N2, R1): the texts of the stateless stop and of the guard notice. With no policy
/// file at the commit the ledger first recorded, a commit the stateless stop cannot judge under
/// that policy names it as that, not as "the accepted policy" (none exists) or "the base commit"
/// (that stop has no session base). The guard notice for an edit of the policy says what is true
/// of both kinds of stop.
#[test]
fn the_stateless_stop_and_the_guard_notice_say_what_is_true() {
    let p = project("n2", STRICT);
    git(&p.root, &["rm", "--quiet", "warden.toml"]);
    git(
        &p.root,
        &[
            "commit",
            "--quiet",
            "-m",
            "no policy file at the first record",
        ],
    );
    p.start("k0");
    p.commit(&[("warden.toml", STRICT)], "P: a person adds the policy");
    p.start("k1");
    // the guard notice (T5), after an edit of the policy
    std::fs::write(p.root.join("warden.toml"), LOOSE).unwrap();
    let input = json!({"hook_event_name": "PostToolUse", "session_id": "k1", "cwd": p.root,
        "tool_name": "Edit"});
    let notice = p.hook_with("post-tool", &input, None);
    let notice = notice.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        notice.contains("where this project's ledger first recorded a commit that has the policy file, committing an edit of it can make this session's stops stricter, never looser, and committing it does not make it the project's accepted policy"),
        "{notice}"
    );
    git(&p.root, &["checkout", "--quiet", "--", "warden.toml"]);
    let benign = p.commit(&[("b.rs", "fn b() {}\n")], "B: benign");
    // the session's state is damaged: the next stop is the stateless one
    let mut keys = 0;
    let mut dirs = vec![p.home.join(".local/state/deep-diff-forge")];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.file_name().is_some_and(|name| name == "key") {
                std::fs::write(&path, "x").unwrap();
                keys += 1;
            }
        }
    }
    assert!(keys > 0, "no key file to damage");
    let decision = p.stop("k1");
    let said = words(&decision);
    assert!(
        said.contains(
            "under the policy at the commit this project's ledger first recorded (commit"
        ) && said.contains("that commit has no policy file \"warden.toml\""),
        "{said}"
    );
    assert!(
        !said.contains("under the accepted policy (commit")
            && !said.contains("the base commit has no policy file"),
        "{said}"
    );
    assert!(
        p.records_of(&benign).iter().all(|r| r.0 != "pass") && !p.records_of(&benign).is_empty(),
        "{:?}",
        p.records_of(&benign)
    );
}

/// Damage the session's state (its key files), so that the next stop is the stateless one.
fn damage(p: &Project) {
    let mut keys = 0;
    let mut dirs = vec![p.home.join(".local/state/deep-diff-forge")];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.file_name().is_some_and(|name| name == "key") {
                std::fs::write(&path, "x").unwrap();
                keys += 1;
            }
        }
    }
    assert!(keys > 0, "no key file to damage");
}

/// Vesper's early read of binary 1 (L1C-EARLY-ADVERSARY): which acceptances apply depends on
/// HEAD, so HEAD moved while the judge ran changes the accepted policy with neither file changed.
/// A looser policy accepted at C, a session at C commits D with a key; the judge pauses; HEAD is
/// reset to before C; the judge resumes: D is recorded UNJUDGED naming the change, never `pass`.
/// The control: HEAD not moved, the pass is kept.
#[test]
fn head_moved_while_the_judge_ran_appends_no_stale_pass() {
    for moved in [true, false] {
        let p = project(if moved { "head-moved" } else { "head-kept" }, STRICT);
        let first = p.head();
        p.start("k0");
        let loose = p.commit(&[("warden.toml", LOOSE)], "C: loose");
        let (code, out, err) = p.accept(&loose, &[], &format!("{}\n", sha(LOOSE)));
        assert_eq!(code, 0, "{out}{err}");
        p.start("k1");
        let key = p.commit(&[("k.md", KEY)], "D: a key");
        let decision = paused_stop(&p, "k1", || {
            if moved {
                git(&p.root, &["reset", "--quiet", "--hard", &first]);
            }
        });
        let records = p.records_of(&key);
        assert!(!records.is_empty(), "{:?}", p.ledger());
        if moved {
            assert!(
                records.iter().all(|r| r.0 != "pass"),
                "a stale pass was recorded after HEAD moved: {records:?}"
            );
            assert!(
                words(&decision).contains(
                    "UNJUDGED: this project's accepted policy changed while this stop judged"
                ),
                "{decision}"
            );
        } else {
            assert!(
                records
                    .iter()
                    .any(|r| r.0 == "pass" && r.2.as_deref() == Some(sha(LOOSE).as_str())),
                "the control lost its pass: {records:?}"
            );
        }
    }
}

/// Ruling 9: the accepted policy is one required policy of every stop. Where an acceptance
/// applies, the stop whose session state cannot be used judges every unit under the same
/// accepted policy as the stop with its state (the same case run both ways, compared by the
/// policy digests each unit was sealed to); and a hook given another name than the bound one
/// passes nothing in either.
#[test]
fn the_stateless_stop_judges_under_the_accepted_policy_too() {
    for damaged in [false, true] {
        let (p, tight, _) = tightenable(if damaged { "r9-damaged" } else { "r9-intact" });
        let (code, out, err) = p.accept(&tight, &[], &format!("{}\n", sha(TIGHT)));
        assert_eq!(code, 0, "{out}{err}");
        p.start("k1");
        let m = p.commit(&[("m.md", "TIGHT only the accepted policy fails\n")], "M");
        if damaged {
            damage(&p);
        }
        let decision = p.stop("k1");
        let records = p.records_of(&m);
        assert!(!records.is_empty(), "{damaged}: {:?}", p.ledger());
        assert!(
            records
                .iter()
                .any(|r| r.2.as_deref() == Some(sha(TIGHT).as_str())),
            "{damaged}: not judged under the accepted policy: {records:?}"
        );
        assert!(
            blocks(&decision) && records.iter().all(|r| r.0 == "fail"),
            "{damaged}: {decision} {records:?}"
        );
    }
    for damaged in [false, true] {
        let p = project(
            if damaged {
                "r9-name-damaged"
            } else {
                "r9-name-intact"
            },
            STRICT,
        );
        std::fs::write(p.root.join("loose.toml"), LOOSE).unwrap();
        git(&p.root, &["add", "--all"]);
        git(
            &p.root,
            &["commit", "--quiet", "-m", "a lenient file beside it"],
        );
        p.start("k0");
        p.start_with("k1", Some("loose.toml"));
        let key = p.commit(&[("k.md", KEY)], "a key");
        if damaged {
            damage(&p);
        }
        let decision = p.stop_with("k1", Some("loose.toml"));
        let records = p.records_of(&key);
        assert!(
            !records.is_empty() && records.iter().all(|r| r.0 != "pass"),
            "{damaged}: a key passed under another name than the bound one: {records:?}"
        );
        assert!(
            words(&decision).contains("these hooks are given the policy name \"loose.toml\"; this project's policy record binds \"warden.toml\""),
            "{damaged}: {decision}"
        );
        // ruling 11, N3: an accepted policy that cannot be had names no commit as its own
        assert!(
            !words(&decision).contains("the accepted policy (commit"),
            "{damaged}: {decision}"
        );
    }
}

fn ledger_of(p: &Project) -> PathBuf {
    p.own().join("ledger.jsonl")
}

/// Ruling 10, F1: a ledger that exists and cannot be read whole (here grown past the 64 MiB its
/// readers read, as a session's own commits and stops can grow it) leaves the accepted policy
/// unknown, never "no accepted policy": a hook given another name passes nothing, an accepted
/// tighter policy is not dropped into a pass, and the doctor's link fails and says what to do.
#[test]
fn a_ledger_grown_past_its_readers_passes_nothing() {
    let pad = |p: &Project| {
        let line = format!(
            "{}\n",
            json!({"v": 1, "kind": "check", "session": "0123456789abcdef", "units": 1, "failed": 0})
        );
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(ledger_of(p))
            .unwrap();
        let block = line.repeat(1 << 14);
        let mut written = 0_usize;
        while written <= 65 << 20 {
            file.write_all(block.as_bytes()).unwrap();
            written += block.len();
        }
    };
    // another name than the bound one
    let p = project("grown-name", STRICT);
    std::fs::write(p.root.join("loose.toml"), LOOSE).unwrap();
    git(&p.root, &["add", "--all"]);
    git(
        &p.root,
        &["commit", "--quiet", "-m", "a lenient file beside it"],
    );
    p.start("k0");
    pad(&p);
    p.start_with("k1", Some("loose.toml"));
    let key = p.commit(&[("k.md", KEY)], "a key");
    let decision = p.stop_with("k1", Some("loose.toml"));
    assert!(
        !blocks(&decision) && words(&decision).contains("UNJUDGED"),
        "{decision}"
    );
    let ledger = std::fs::read(ledger_of(&p)).unwrap();
    let tail =
        String::from_utf8_lossy(&ledger[ledger.len().saturating_sub(1 << 16)..]).into_owned();
    let mine: Vec<Value> = tail
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|r| r["unit"] == format!("commit:{key}").as_str())
        .collect();
    assert!(
        !mine.is_empty() && mine.iter().all(|r| r["outcome"] != "pass"),
        "a key passed under another name than the bound one: {mine:?}"
    );
    // an accepted tighter policy: what only it fails does not pass
    let (p, tight, _) = tightenable("grown-accepted");
    let (code, out, err) = p.accept(&tight, &[], &format!("{}\n", sha(TIGHT)));
    assert_eq!(code, 0, "{out}{err}");
    pad(&p);
    p.start("k1");
    let m = p.commit(&[("m.md", "TIGHT only the accepted policy fails\n")], "M");
    let decision = p.stop("k1");
    let ledger = std::fs::read(ledger_of(&p)).unwrap();
    let tail =
        String::from_utf8_lossy(&ledger[ledger.len().saturating_sub(1 << 16)..]).into_owned();
    let mine: Vec<Value> = tail
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|r| r["unit"] == format!("commit:{m}").as_str())
        .collect();
    assert!(
        !mine.is_empty() && mine.iter().all(|r| r["outcome"] != "pass"),
        "{decision} {mine:?}"
    );
    let (ok, detail) = p.accepted_link();
    assert!(
        !ok && detail.contains("this project's ledger cannot be read whole")
            && detail.contains("to settle it, a person deals with the ledger"),
        "{detail}"
    );
}

/// Ruling 10, F2 and F3: two branches' acceptances. On main, an acceptance made on a feature
/// branch is not of main's history: the doctor names it and stays ready. After the feature branch
/// is merged, its acceptance is in main's history and does not follow from the policy reached:
/// no change passes, naming both commits, until a person accepts again from the policy reached.
#[test]
fn acceptances_of_two_branches_that_meet_in_a_merge_pass_nothing_until_settled() {
    let p = project("merge", STRICT);
    p.start("k0");
    // the feature branch: a looser policy accepted there
    git(&p.root, &["checkout", "--quiet", "-b", "feature"]);
    let loose = p.commit(&[("warden.toml", LOOSE)], "feature: loose");
    let (code, out, err) = p.accept(&loose, &[], &format!("{}\n", sha(LOOSE)));
    assert_eq!(code, 0, "{out}{err}");
    // main: a tighter policy accepted from the first one
    git(&p.root, &["checkout", "--quiet", "main"]);
    let tight = p.commit(&[("warden.toml", TIGHT)], "main: tight");
    let (code, out, err) = p.accept(&tight, &[], &format!("{}\n", sha(TIGHT)));
    assert_eq!(code, 0, "{out}{err}");
    let (ok, detail) = p.accepted_link();
    assert!(
        ok && detail.contains("does not apply here and is harmless")
            && detail.contains(&format!("the one at commit {}", &tight[..7])),
        "{detail}"
    );
    // the control: no merge; a commit only TIGHT fails blocks
    p.start("k1");
    let before = p.commit(&[("m.md", "TIGHT\n")], "M");
    assert!(blocks(&p.stop("k1")));
    assert!(p.records_of(&before).iter().any(|r| r.0 == "fail"));
    // the merge brings the feature's acceptance into main's history
    git(
        &p.root,
        &["merge", "--quiet", "--no-edit", "-X", "theirs", "feature"],
    );
    p.start("k2");
    let after = p.commit(&[("n.md", "TIGHT again\n")], "N");
    let decision = p.stop("k2");
    let records = p.records_of(&after);
    assert!(
        !records.is_empty() && records.iter().all(|r| r.0 != "pass"),
        "{decision} {records:?}"
    );
    assert!(
        words(&decision).contains(&format!(
            "acceptances in this tree's history (of commit {})",
            &tight[..12]
        )),
        "{decision}"
    );
    let (ok, detail) = p.accepted_link();
    assert!(
        !ok && detail.contains("to settle it, a person accepts"),
        "{detail}"
    );
    // settled: the owner accepts the merged policy, from the policy reached
    let merged = p.head();
    let text = std::fs::read_to_string(p.root.join("warden.toml")).unwrap();
    let (code, out, err) = p.accept(&merged, &[], &format!("{}\n", sha(&text)));
    assert_eq!(code, 0, "{out}{err}");
    assert!(
        out.contains(&format!("the file at commit {}", &loose[..12])),
        "{out}"
    );
    let (ok, detail) = p.accepted_link();
    assert!(ok, "{detail}");
}

/// Ruling 7 row 3 and ruling 10 F8: where the ledger's first record names no commit, or that
/// commit has no policy file, an acceptance from no accepted policy supplies the first one, and
/// after it the stop without its state does not pass a key committed beside a loosening (it did
/// before the acceptance: the stated limit). And the shallow refusal of the command, with its
/// control.
#[test]
fn the_first_acceptance_from_none_and_the_shallow_refusal() {
    for (kind, accepted) in [("nofile", false), ("nofile", true), ("nullhead", true)] {
        let p = project(&format!("first-{kind}-{accepted}"), STRICT);
        if kind == "nullhead" {
            // a repository whose first hook ran before its first commit
            std::fs::remove_dir_all(p.root.join(".git")).unwrap();
            git(&p.root, &["init", "--quiet"]);
            p.start("k0");
            git(&p.root, &["add", "a.rs"]);
            git(&p.root, &["commit", "--quiet", "-m", "first"]);
        } else {
            git(&p.root, &["rm", "--quiet", "--cached", "warden.toml"]);
            git(&p.root, &["commit", "--quiet", "-m", "no policy file"]);
            p.start("k0");
        }
        let policy = p.commit(&[("warden.toml", STRICT)], "P: a person adds the policy");
        p.start("k1");
        if accepted {
            let (code, out, err) = p.accept(&policy, &[], &format!("{}\n", sha(STRICT)));
            assert_eq!(code, 0, "{kind}: {out}{err}");
            assert!(
                out.contains("Accepted policy now: none (this project has no accepted policy yet"),
                "{out}"
            );
        }
        p.commit(&[("warden.toml", LOOSE)], "L: loosen");
        let key = p.commit(&[("k.md", KEY)], "K: a key");
        damage(&p);
        p.stop("k1");
        let records = p.records_of(&key);
        if accepted {
            assert!(
                !records.is_empty() && records.iter().all(|r| r.0 != "pass"),
                "{kind}: the first acceptance did not hold: {records:?}"
            );
        } else {
            assert!(
                !records.is_empty() && records.iter().all(|r| r.0 != "pass"),
                "{kind}: {records:?}"
            );
        }
    }
    // the shallow refusal: a depth-1 clone recorded, then HEAD moved across a cut
    let upstream = project("shallow-up", STRICT);
    upstream.commit(&[("b.rs", "fn b() {}\n")], "U1");
    let p = project("shallow-clone", STRICT);
    std::fs::remove_dir_all(&p.root).unwrap();
    let url = format!("file://{}", upstream.root.display());
    git(
        p.root.parent().unwrap(),
        &[
            "clone",
            "--quiet",
            "--depth",
            "1",
            &url,
            p.root.to_str().unwrap(),
        ],
    );
    p.start("k0");
    upstream.commit(&[("c.rs", "fn c() {}\n")], "U2");
    let tight = upstream.commit(&[("warden.toml", TIGHT)], "U3: tight");
    git(
        &p.root,
        &["fetch", "--quiet", "--depth", "1", "origin", "main"],
    );
    git(&p.root, &["reset", "--quiet", "--hard", "FETCH_HEAD"]);
    let (code, _, err) = p.accept(&tight, &[], &format!("{}\n", sha(TIGHT)));
    assert_eq!(code, 4, "{err}");
    assert!(err.contains("this clone is shallow"), "{err}");
    git(&p.root, &["fetch", "--quiet", "--unshallow", "origin"]);
    let (code, out, err) = p.accept(&tight, &[], &format!("{}\n", sha(TIGHT)));
    assert_eq!(code, 0, "{out}{err}");
}

/// Ruling 10, F4 to F6: the command's last line says what an acceptance does to units already
/// judged; a refusal of its arguments carries what the command is not; a stop given another name
/// whose file is missing at the base names both names.
#[test]
fn the_command_and_the_stop_say_what_is_true() {
    let (p, tight, _) = tightenable("texts");
    let (code, out, err) = p.accept(&tight, &[], &format!("{}\n", sha(TIGHT)));
    assert_eq!(code, 0, "{out}{err}");
    assert!(
        out.contains("Units not yet judged are judged under it from the next stop; a commit a stop already passed under an earlier accepted policy keeps that pass, and no commit is verified by this."),
        "{out}"
    );
    let refused = p
        .command()
        .args(["agent", "policy-accept", "--bogus"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let said = String::from_utf8_lossy(&refused.stderr);
    assert_eq!(refused.status.code(), Some(2), "{said}");
    assert!(
        said.contains("any process running as this user can invoke or write the record of, not authentication; the pull request's judge, under the target branch's policy, is the authority."),
        "{said}"
    );
    // a name whose file is missing at the base, and which the record does not bind
    p.start_with("k1", Some("other.toml"));
    p.commit(&[("m.md", "m\n")], "M");
    let decision = p.stop_with("k1", Some("other.toml"));
    let said = words(&decision);
    assert!(
        said.contains("\"other.toml\"") && said.contains("binds \"warden.toml\""),
        "{said}"
    );
}

/// Ruling 11 (N-F2, N1, F3): the owner accepts B on a feature branch, D (a tightening) on main
/// and E on the feature branch again, in either order of D and E in the record. Without a merge,
/// main blocks under D, and each tree's doctor stays ready (the other branch's acceptances are
/// not of its history). With the merge, nothing passes in either order: E, made where D was not
/// in history, does not settle D's disagreement. The command refuses to settle at a commit that
/// lacks D, names D and shows the diff against it, and settles at the merge commit.
#[test]
fn a_disagreement_is_settled_only_where_the_owner_saw_it() {
    const E_POLICY: &str =
        "rule no-todo hold NEVER-WRITTEN-ANYWHERE\nrule e-loose hold NEVER-EITHER\n";
    for (merged, d_first) in [(false, true), (false, false), (true, true), (true, false)] {
        let label = format!("settle-{merged}-{d_first}");
        let p = project(&label, STRICT);
        p.start("k0");
        git(&p.root, &["checkout", "--quiet", "-b", "feature"]);
        let b = p.commit(&[("warden.toml", LOOSE)], "feature: B");
        let (code, out, err) = p.accept(&b, &[], &format!("{}\n", sha(LOOSE)));
        assert_eq!(code, 0, "{out}{err}");
        let accept_on = |branch: &str, policy: &str, message: &str| {
            git(&p.root, &["checkout", "--quiet", branch]);
            let at = p.commit(&[("warden.toml", policy)], message);
            let (code, out, err) = p.accept(&at, &[], &format!("{}\n", sha(policy)));
            assert_eq!(code, 0, "{label} {message}: {out}{err}");
            at
        };
        let (d, _e) = if d_first {
            let d = accept_on("main", TIGHT, "main: D");
            (d, accept_on("feature", E_POLICY, "feature: E"))
        } else {
            let e = accept_on("feature", E_POLICY, "feature: E");
            (accept_on("main", TIGHT, "main: D"), e)
        };
        git(&p.root, &["checkout", "--quiet", "feature"]);
        let (ok, detail) = p.accepted_link();
        assert!(ok, "{label}: the feature tree's doctor: {detail}");
        git(&p.root, &["checkout", "--quiet", "main"]);
        let (ok, detail) = p.accepted_link();
        assert!(ok, "{label}: main's doctor: {detail}");
        if merged {
            git(
                &p.root,
                &["merge", "--quiet", "--no-edit", "-X", "theirs", "feature"],
            );
        }
        p.start("k1");
        let m = p.commit(&[("m.md", "TIGHT only D fails\n")], "M");
        let decision = p.stop("k1");
        let records = p.records_of(&m);
        assert!(!records.is_empty(), "{label}: {:?}", p.ledger());
        if !merged {
            assert!(
                blocks(&decision) && records.iter().any(|r| r.0 == "fail"),
                "{label}: {decision} {records:?}"
            );
            continue;
        }
        assert!(
            records.iter().all(|r| r.0 != "pass"),
            "{label}: the merge settled D's disagreement: {records:?}"
        );
        assert!(
            words(&decision).contains(&format!("(of commit {}", &d[..12])),
            "{label}: {decision}"
        );
        // the command at a commit that lacks D is refused, naming D
        let (code, _, err) = p.accept(&b, &[], &format!("{}\n", sha(LOOSE)));
        assert_eq!(code, 4, "{label}: {err}");
        assert!(
            err.contains(&format!("does not have in its history commit {}", &d[..12])),
            "{label}: {err}"
        );
        // at the merge commit it settles, and says what it settles first
        let merge = p.head();
        let text = std::fs::read_to_string(p.root.join("warden.toml")).unwrap();
        let (code, out, err) = p.accept(&merge, &[], &format!("{}\n", sha(&text)));
        assert_eq!(code, 0, "{label}: {out}{err}");
        assert!(
            out.starts_with("This project's accepted policy cannot be had: acceptances in this tree's history do not follow")
                && out.contains(&format!("disagreeing acceptance: sha256 {} (at commit {})", sha(TIGHT), &d[..12]))
                && out.contains("Policy reached before them: sha256")
                && out.contains(&format!("the disagreeing acceptance at commit {} to policy to accept:", &d[..12]))
                && out.contains("-rule tight fail TIGHT"),
            "{label}: {out}"
        );
        let (ok, detail) = p.accepted_link();
        assert!(ok, "{label}: after the settle: {detail}");
    }
}
