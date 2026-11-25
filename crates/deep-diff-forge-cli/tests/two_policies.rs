//! Release 0.11.1, package L1b: every unit of a stop is judged under two policies, the one at
//! the session's base (as 0.11.0 judged) and the project's accepted policy (the policy file at
//! the commit the project's ledger first recorded), the stricter answer standing. A policy edit
//! that is merely committed judges nothing, in the session that makes it or after it.
//!
//! The hooks run as Claude Code runs them, against real repositories, with a stand-in judge
//! whose answer depends on the policy it is given: each line `rule ID SEVERITY WORD` of the
//! policy holds (`hold`) or fails (`fail`) a patch that adds a line with `WORD` (a line of a
//! policy file, `rule …`, is not read for it), or, for `path:NAME`, a patch that changes the
//! file `NAME`; a line `@refuse` makes it refuse the
//! ranker it is offered (exit 4, as a policy that does not admit this build does); `@hang`
//! makes it use all the time it is given. Every run is logged with the digests of its policy
//! and patch, so a test can tell which policy judged what.
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

/// The policy every project of these tests starts with: a key fails, an edit of the policy
/// file is held (it protects itself, as forge-warden's starter policy does in `ci-is-owned`).
const STRICT: &str = "rule no-secrets fail AKIA\nrule ci-is-owned hold path:warden.toml\n";
/// A loosened policy, as a left commit writes it: no rule against keys, no protection.
const LOOSE: &str = "rule no-todo hold NEVER-WRITTEN-ANYWHERE\n";
/// A line with a key, as the judge's rule `no-secrets` reads it.
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

/// A project of one test: its repository, the home of its hooks, the stand-in judge and the
/// judge's log.
struct Project {
    _scratch: Scratch,
    root: PathBuf,
    home: PathBuf,
    judge: PathBuf,
    log: PathBuf,
}

/// A repository with one commit (`a.rs` and, when given, the policy file `warden.toml`), a home
/// for its hooks and the stand-in judge.
fn project(name: &str, policy: Option<&str>) -> Project {
    let scratch = Scratch::new(&format!("two-policies-{name}"));
    let home = scratch.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let root = scratch.join("repo");
    std::fs::create_dir(&root).unwrap();
    git(&root, &["init", "--quiet"]);
    std::fs::write(root.join("a.rs"), "fn a() {}\n").unwrap();
    if let Some(policy) = policy {
        std::fs::write(root.join("warden.toml"), policy).unwrap();
    }
    git(&root, &["add", "--all"]);
    git(&root, &["commit", "--quiet", "-m", "base"]);
    let (judge, log) = (scratch.join("judge"), scratch.join("judge.log"));
    write_program(&judge, &JUDGE.replace("@LOG@", &log.display().to_string()));
    Project {
        _scratch: scratch,
        root,
        home,
        judge,
        log,
    }
}

/// The stand-in judge (see the head of this file). Its document is a whole
/// `forge-warden.verdict.v2` sealed to its inputs, one rule a policy line, the summary counting
/// them, with the outcome and exit that belong together.
const JUDGE: &str = r#"#!/bin/sh
if [ "$1" = check ] && [ "$2" = --help ]; then echo 'usage: forge-warden check --policy P --patch F --ranker R --json'; exit 0; fi
policy=; patch=; ranker=
while [ $# -gt 0 ]; do
  case "$1" in
    --policy) policy="$2";;
    --patch) patch="$2";;
    --ranker) ranker="$2";;
  esac
  shift
done
sum() { sha256sum < "$1" | cut -d' ' -f1; }
p="$(sum "$policy")"; f="$(sum "$patch")"; r="$(sum "$ranker")"
printf 'RUN %s %s %s\n' "$p" "$f" "$(date +%s.%N)" >> '@LOG@'
if grep -q '^@refuse' "$policy"; then echo '{"exit":4,"name":"invalid_input","message":"this policy does not admit the ranker","next":"change_input"}' >&2; exit 4; fi
if grep -q '^@hang' "$policy"; then sleep 600; fi
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

/// Write an executable script through a child process: a file this process had open for
/// writing can still be busy when another test thread forks (ETXTBSY).
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

impl Project {
    /// Write `files` (a path and its text; `None` removes the file) and commit them.
    fn commit(&self, files: &[(&str, Option<&str>)], message: &str) -> String {
        for (path, text) in files {
            let path = self.root.join(path);
            match text {
                Some(text) => {
                    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                    std::fs::write(&path, text).unwrap();
                }
                None => std::fs::remove_file(&path).unwrap(),
            }
        }
        git(&self.root, &["add", "--all"]);
        git(&self.root, &["commit", "--quiet", "-m", message]);
        self.head()
    }

    fn head(&self) -> String {
        git(&self.root, &["rev-parse", "HEAD"]).trim().to_owned()
    }

    /// Run `deep-diff-forge agent hook <event> --client claude --judge <judge>` with `input`.
    fn hook(&self, event: &str, input: &Value) -> String {
        let mut child = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
            .envs(ISOLATE)
            .env("HOME", &self.home)
            .env_remove("XDG_STATE_HOME")
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("CLAUDE_PROJECT_DIR")
            .args(["agent", "hook", event, "--client", "claude", "--judge"])
            .arg(&self.judge)
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
        assert_eq!(
            (
                out.status.code(),
                String::from_utf8_lossy(&out.stderr).as_ref()
            ),
            (Some(0), ""),
            "{event}"
        );
        String::from_utf8(out.stdout).unwrap()
    }

    /// The start of `session` with `source` (`startup`, `resume`, `compact`, `clear`): what the
    /// agent is told.
    fn start(&self, session: &str, source: &str) -> String {
        let input = json!({"hook_event_name": "SessionStart", "session_id": session,
            "cwd": self.root, "source": source});
        let said = self.hook("session-start", &input);
        let doc: Value = serde_json::from_str(&said).unwrap_or(Value::String(said.clone()));
        doc["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .map_or(said, str::to_owned)
    }

    /// What a stop of `session` decides (`Null`: it said nothing).
    fn stop(&self, session: &str) -> Value {
        let input = json!({"hook_event_name": "Stop", "session_id": session,
            "cwd": self.root, "stop_hook_active": false});
        let said = self.hook("stop", &input);
        if said.is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&said).unwrap_or_else(|e| panic!("{e}: {said}"))
        }
    }

    /// The project's ledger, a record a line.
    fn ledger(&self) -> Vec<Value> {
        std::fs::read_to_string(self.root.join(".git/deep-diff-forge/ledger.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap_or_else(|e| panic!("{e}: {line}")))
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

    /// `agent verify-branch RANGE --json`: its exit code and its document.
    fn verify(&self, range: &str) -> (i32, Value) {
        let out = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
            .current_dir(&self.root)
            .envs(ISOLATE)
            .env("HOME", &self.home)
            .args(["agent", "verify-branch", range, "--json"])
            .stdin(Stdio::null())
            .output()
            .unwrap();
        let doc = serde_json::from_slice(&out.stdout)
            .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&out.stdout)));
        (out.status.code().unwrap(), doc)
    }

    /// The runs of the judge so far: the digests of the policy and of the patch of each.
    fn runs(&self) -> Vec<(String, String)> {
        std::fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .filter_map(|line| {
                let mut words = line.strip_prefix("RUN ")?.split(' ');
                Some((words.next()?.to_owned(), words.next()?.to_owned()))
            })
            .collect()
    }
}

/// The SHA-256 of `text`, as the judge logs and the ledger seals a policy.
fn sha(text: &str) -> String {
    deep_diff_forge_core::sha256_hex(text.as_bytes())
}

/// What a stop says, whether it blocked (its reason) or let the agent stop (its message).
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

/// The reviewer's case of the 0.11.0 tree review, from every source of a fresh session: a
/// session K0 leaves a commit L that deletes the rule against keys and the policy's protection
/// of its own file, and adds a key; L never had a stop. In 0.11.0 a session that began at L
/// with `startup` judged L under L's own policy: L passed and `agent verify-branch` verified it.
/// Now L is judged under the accepted policy too (the one at the commit the ledger first
/// recorded): it fails, whatever the source (R1, T-1). With `startup` the next session's own
/// commit with a key, and its own uncommitted key, fail as well (scenario 6, T-4), and a chain
/// of left commits that splits the loosening from the key fails at the key (T-2).
#[test]
fn a_left_loosening_judges_neither_itself_nor_the_next_session() {
    for source in ["startup", "resume", "compact", "clear"] {
        let p = project(&format!("left-{source}"), Some(STRICT));
        let base = p.head();
        p.start("k0", "startup");
        let left = p.commit(
            &[("warden.toml", Some(LOOSE)), ("notes.md", Some(KEY))],
            "L: loosen and a key",
        );
        p.start("k1", source);
        let decision = p.stop("k1");
        assert!(blocks(&decision), "{source}: {decision}");
        let records = p.records_of(&left);
        assert!(
            !records.is_empty() && records.iter().all(|(outcome, _, _)| outcome == "fail"),
            "{source}: {records:?}"
        );
        let (code, doc) = p.verify(&format!("{base}..HEAD"));
        assert_eq!(code, 7, "{source}: {doc}");
        if source == "startup" {
            // L was judged under both policies, the stricter standing, and said so (T8)
            let policies: std::collections::BTreeSet<_> =
                records.iter().filter_map(|r| r.2.clone()).collect();
            assert_eq!(
                policies,
                [sha(STRICT), sha(LOOSE)].into_iter().collect(),
                "{records:?}"
            );
            assert!(
                words(&decision).contains(&format!(
                    "judged under two policies: at the session base (commit {}) and the accepted one (commit {}); the stricter stands",
                    &left[..7],
                    &base[..7]
                )),
                "{decision}"
            );
        }
    }

    // scenario 6: L only loosens; the next session's own commit and tree carry keys
    let p = project("scenario-6", Some(STRICT));
    let base = p.head();
    p.start("k0", "startup");
    p.commit(&[("warden.toml", Some(LOOSE))], "L: loosen only");
    let said = p.start("k1", "startup");
    let own = p.commit(&[("deploy.md", Some(KEY))], "M: a key");
    std::fs::write(p.root.join("local.md"), KEY).unwrap();
    let decision = p.stop("k1");
    assert!(blocks(&decision), "{decision}");
    assert!(
        p.records_of(&own)
            .iter()
            .all(|(outcome, _, _)| outcome == "fail"),
        "{:?}",
        p.records_of(&own)
    );
    let trees: Vec<Value> = (p.ledger().into_iter())
        .filter(|r| r["unit"].as_str().is_some_and(|u| u.starts_with("tree:")))
        .collect();
    assert!(
        !trees.is_empty() && trees.iter().all(|r| r["outcome"] == "fail"),
        "{trees:?}"
    );
    assert_eq!(p.verify(&format!("{base}..HEAD")).0, 7);
    // the start said that the stops judge under both, and how a policy is not put into effect
    let said = said.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        said.contains("is not the accepted policy of this project (that is the one at commit")
            && said.contains("each change is judged under both. Committing an edit of the policy does not make it accepted.")
            && said.contains("so it is not authentication. The pull-request judge under the target branch's policy is the authority."),
        "{said}"
    );
    assert!(
        !said.contains("\"kind\"") && !said.contains("clearance"),
        "{said}"
    );

    // a chain: L1 drops the protection, L2 drops the rule and adds a key, L3 is innocent
    let p = project("chain", Some(STRICT));
    p.start("k0", "startup");
    p.commit(
        &[("warden.toml", Some("rule no-secrets fail AKIA\n"))],
        "L1: drop the protection",
    );
    let key = p.commit(
        &[("warden.toml", Some(LOOSE)), ("k.md", Some(KEY))],
        "L2: drop the rule, a key",
    );
    let innocent = p.commit(&[("b.rs", Some("fn b() {}\n"))], "L3");
    p.start("k1", "startup");
    assert!(blocks(&p.stop("k1")));
    assert!(
        p.records_of(&key).iter().all(|r| r.0 == "fail"),
        "{:?}",
        p.records_of(&key)
    );
    assert!(p.records_of(&innocent).iter().all(|r| r.0 == "pass"));
}

/// The invariant (the first critic's p1 and p1b, T-13 and T-14): half 1 is the policy at the
/// session's base, as in 0.11.0, so a session that commits its own loosening and then a key is
/// failed whatever its ledger is: kept, removed, not a regular file, over the 64 MiB a reader
/// reads (each of which leaves no accepted policy: R6 a). And a policy that does not protect its
/// own file is not loosened by a stop's `pass` of the edit (p1b).
#[test]
fn an_own_loosening_then_a_key_fails_whatever_the_ledger() {
    for ledger in ["kept", "removed", "a-directory", "over-64-MiB"] {
        let p = project(&format!("p1-{ledger}"), Some(STRICT));
        p.start("k1", "startup");
        p.commit(&[("warden.toml", Some(LOOSE))], "A: loosen (own)");
        let key = p.commit(&[("notes.md", Some(KEY))], "B: a key (own)");
        let file = p.root.join(".git/deep-diff-forge/ledger.jsonl");
        match ledger {
            "removed" => std::fs::remove_file(&file).unwrap(),
            "a-directory" => {
                std::fs::remove_file(&file).unwrap();
                std::fs::create_dir(&file).unwrap();
            }
            "over-64-MiB" => {
                let f = std::fs::OpenOptions::new()
                    .append(true)
                    .open(&file)
                    .unwrap();
                f.set_len((64 << 20) + 1).unwrap();
            }
            _ => {}
        }
        let decision = p.stop("k1");
        assert!(blocks(&decision), "{ledger}: {decision}");
        let said = words(&decision);
        assert!(
            said.contains(&format!("Commit {}", &key[..12])),
            "{ledger}: {said}"
        );
    }

    // p1b: the project's policy does not protect itself; the session drops the rule, then
    // commits a key: half 1 fails the key. The edit's own pass puts nothing into effect for
    // the next session either.
    let unprotected = "rule no-secrets fail AKIA\n";
    let p = project("p1b", Some(unprotected));
    p.start("k1", "startup");
    let edit = p.commit(&[("warden.toml", Some(LOOSE))], "A: drop the rule (own)");
    let key = p.commit(&[("notes.md", Some(KEY))], "B: a key (own)");
    assert!(blocks(&p.stop("k1")));
    assert!(p.records_of(&edit).iter().all(|r| r.0 == "pass"));
    assert!(p.records_of(&key).iter().all(|r| r.0 == "fail"));
    p.start("k2", "startup");
    let later = p.commit(&[("later.md", Some(KEY))], "C: a key (the next session's)");
    assert!(blocks(&p.stop("k2")));
    assert!(
        p.records_of(&later).iter().all(|r| r.0 == "fail"),
        "{:?}",
        p.records_of(&later)
    );
}

/// Two policies that give a unit answers of the same strictness: the rules of both are named,
/// and the judge was run under each (the cache is keyed by the policy too: T-15). A rule a
/// commit tightened before the session began judges that session (half 1: T-5).
#[test]
fn two_policies_that_tie_name_the_rules_of_both() {
    let p = project("tie", Some(STRICT));
    p.start("k0", "startup");
    let tightened = "rule no-todo fail TODO\nrule ci-is-owned hold path:warden.toml\n";
    p.commit(
        &[("warden.toml", Some(tightened))],
        "T: no-secrets out, no-todo in",
    );
    p.start("k1", "startup");
    std::fs::write(p.root.join("both.md"), format!("// TODO\n{KEY}")).unwrap();
    let decision = p.stop("k1");
    let said = words(&decision);
    assert!(blocks(&decision), "{decision}");
    assert!(
        said.contains("no-todo") && said.contains("no-secrets"),
        "{said}"
    );
    // the tree's patch was judged under both policies, once each
    let tree: Vec<(String, String)> = p.runs();
    let patches: std::collections::BTreeMap<&str, Vec<&str>> = tree.iter().fold(
        std::collections::BTreeMap::default(),
        |mut by, (policy, patch)| {
            by.entry(patch.as_str()).or_default().push(policy.as_str());
            by
        },
    );
    assert!(
        patches.values().any(|policies| {
            policies.contains(&sha(STRICT).as_str()) && policies.contains(&sha(tightened).as_str())
        }),
        "{tree:?}"
    );
    // the rule the session's base tightened judges its own commit
    let todo = p.commit(&[("t.md", Some("// TODO\n"))], "M: a todo");
    std::fs::remove_file(p.root.join("both.md")).unwrap();
    let decision = p.stop("k1");
    assert!(blocks(&decision), "{decision}");
    assert!(
        p.records_of(&todo).iter().all(|r| r.0 == "fail"),
        "{:?}",
        p.records_of(&todo)
    );
}

/// Ruling 3 of release 0.11.1 (and T-11): a policy that gives a unit no verdict makes it
/// UNJUDGED in either order, with words that name the policy; every record of the unit is
/// `unjudged` and none `pass`, so `agent verify-branch` exits 7. The accepted policy refuses
/// this build's ranker (a pin update the project has not accepted) in one order; the session's
/// base refuses it in the other.
#[test]
fn a_policy_that_gives_no_verdict_makes_the_unit_unjudged_in_either_order() {
    let refusing = format!("@refuse\n{STRICT}");
    for (name, first, then, half) in [
        (
            "accepted-refuses",
            refusing.as_str(),
            STRICT,
            "under the accepted policy (commit ",
        ),
        (
            "base-refuses",
            STRICT,
            refusing.as_str(),
            "under the policy at the session base (commit ",
        ),
    ] {
        let p = project(name, Some(first));
        let base = p.head();
        p.start("k0", "startup");
        // the left commit changes the policy file (held under the protection where it judges)
        let left = p.commit(&[("warden.toml", Some(then))], "L: the policy changes");
        p.start("k1", "startup");
        let innocent = p.commit(&[("b.rs", Some("fn b() {}\n"))], "X: innocent");
        let decision = p.stop("k1");
        let said = words(&decision);
        assert!(!blocks(&decision), "{name}: {decision}");
        assert!(
            said.contains("UNJUDGED") && said.contains(half),
            "{name}: {said}"
        );
        let records = p.records_of(&innocent);
        assert!(
            !records.is_empty() && records.iter().all(|r| r.0 == "unjudged"),
            "{name}: {records:?}"
        );
        assert!(
            p.ledger().iter().all(|r| r["outcome"] != "pass"),
            "{name}: {:?}",
            p.ledger()
        );
        let (code, doc) = p.verify(&format!("{base}..HEAD"));
        assert_eq!(code, 7, "{name}: {doc}");
        assert!(p.records_of(&left).iter().all(|r| r.0 != "pass"), "{name}");
    }
}

/// R6 a and b: where the ledger names no accepted policy the stops judge under the policy at
/// the session's base alone, as 0.11.0 did, never at a unit's parent and never UNJUDGED for it;
/// the start says so and why, and quotes no ledger line (T3, T-7, T-16, T-20).
#[test]
fn no_accepted_policy_judges_under_the_session_base_alone_and_says_so() {
    // the commit the ledger first recorded has no policy file; a person adds one (P); an
    // innocent change after it is judged, and a key fails
    let p = project("no-file", None);
    p.start("k0", "startup");
    p.commit(&[("warden.toml", Some(STRICT))], "P: the policy");
    let said = p.start("k1", "startup");
    let said = said.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        said.contains("- This project's ledger names no accepted policy (the commit of its first record has no file \"warden.toml\"): changes are judged under the policy at commit"),
        "{said}"
    );
    assert!(
        !said.contains("\"kind\"") && !said.contains("clearance"),
        "{said}"
    );
    let innocent = p.commit(&[("b.rs", Some("fn b() {}\n"))], "X: innocent");
    // (P, which the person left, is held under its own protection: the stop is a HOLD)
    let decision = p.stop("k1");
    assert!(!words(&decision).contains("UNJUDGED"), "{decision}");
    let records = p.records_of(&innocent);
    assert!(records.len() == 1 && records[0].0 == "pass", "{records:?}");
    std::fs::write(p.root.join("k.md"), KEY).unwrap();
    assert!(blocks(&p.stop("k1")));

    // a project first recorded before its first commit: the baseline's head is null
    let scratch = Scratch::new("two-policies-null-head");
    let home = scratch.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let root = scratch.join("repo");
    std::fs::create_dir(&root).unwrap();
    git(&root, &["init", "--quiet"]);
    let (judge, log) = (scratch.join("judge"), scratch.join("judge.log"));
    write_program(&judge, &JUDGE.replace("@LOG@", &log.display().to_string()));
    let p = Project {
        _scratch: scratch,
        root,
        home,
        judge,
        log,
    };
    p.start("k0", "startup");
    let baseline = p.ledger();
    assert!(
        baseline
            .iter()
            .any(|r| r["kind"] == "baseline" && r["head"].is_null()),
        "{baseline:?}"
    );
    p.commit(
        &[("warden.toml", Some(STRICT)), ("a.rs", Some("fn a() {}\n"))],
        "R: first",
    );
    let said = p.start("k1", "startup");
    let said = said.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        said.contains(
            "- This project's ledger names no accepted policy (its first record names no commit)"
        ),
        "{said}"
    );
    std::fs::write(p.root.join("k.md"), KEY).unwrap();
    assert!(blocks(&p.stop("k1")));

    // a ledger that cannot be read
    let p = project("unreadable", Some(STRICT));
    p.start("k0", "startup");
    let file = p.root.join(".git/deep-diff-forge/ledger.jsonl");
    std::fs::remove_file(&file).unwrap();
    std::fs::create_dir(&file).unwrap();
    let said = p.start("k1", "startup");
    assert!(
        said.split_whitespace().collect::<Vec<_>>().join(" ").contains(
            "- This project's ledger names no accepted policy (it could not be read): changes are judged under the policy at commit"
        ),
        "{said}"
    );
    std::fs::write(p.root.join("k.md"), KEY).unwrap();
    let decision = p.stop("k1");
    assert!(blocks(&decision), "{decision}");
    assert!(
        words(&decision).contains("No accepted policy could be read from it, so they were judged under the policy at the commit this session began from alone."),
        "{decision}"
    );
}

/// R6 c: an accepted policy that exists and cannot be read lets no unit pass, and never falls
/// back to `HEAD`: the commit the ledger first recorded is gone from the repository
/// (`place.base_gone`, T-16), or a shallow clone does not have the history back to it
/// (`place.shallow_parent`, T-23). What the policy at the session's base fails still fails.
#[test]
fn an_accepted_policy_that_cannot_be_read_lets_no_unit_pass() {
    // pruned: the history is replaced by an unrelated root with a loosened policy
    let p = project("pruned", Some(STRICT));
    let first = p.head();
    p.start("k0", "startup");
    git(&p.root, &["checkout", "--quiet", "--orphan", "other"]);
    p.commit(
        &[("warden.toml", Some(LOOSE)), ("w.rs", Some("fn w() {}\n"))],
        "W: unrelated root",
    );
    git(&p.root, &["branch", "-D", "main"]);
    git(&p.root, &["reflog", "expire", "--expire=now", "--all"]);
    git(&p.root, &["gc", "--quiet", "--prune=now"]);
    let gone = Command::new("git")
        .current_dir(&p.root)
        .envs(ISOLATE)
        .args(["cat-file", "-e", &first])
        .status()
        .unwrap();
    assert!(!gone.success(), "the first record's commit is still there");
    p.start("k1", "startup");
    std::fs::write(p.root.join("c.rs"), "fn c() {}\n").unwrap();
    let decision = p.stop("k1");
    let said = words(&decision);
    assert!(!blocks(&decision), "{decision}");
    assert!(
        said.contains(&format!(
            "the commit this project's ledger first recorded, {}, is no longer in this repository, so the accepted policy cannot be read (`place.base_gone`)",
            &first[..12]
        )),
        "{said}"
    );
    let stops: Vec<Value> = (p.ledger().into_iter())
        .filter(|r| r["kind"] == "stop")
        .collect();
    assert!(
        !stops.is_empty()
            && stops
                .iter()
                .all(|r| r["outcome"] != "pass" && r["class"] == "place.base_gone"),
        "{stops:?}"
    );
    // a key still fails under the policy at the session's base? (that base is W: loosened, so
    // it does not; and the key is not a pass either)
    std::fs::write(p.root.join("k.md"), KEY).unwrap();
    let decision = p.stop("k1");
    assert!(
        !blocks(&decision) && words(&decision).contains("UNJUDGED"),
        "{decision}"
    );

    // shallow: a depth-1 clone records its baseline, a depth-1 fetch moves HEAD across it
    let upstream = project("shallow-upstream", Some(STRICT));
    upstream.commit(&[("b.rs", Some("fn b() {}\n"))], "U1");
    let scratch = Scratch::new("two-policies-shallow");
    let home = scratch.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let root = scratch.join("clone");
    let url = format!("file://{}", upstream.root.display());
    git(
        &scratch.join(""),
        &[
            "clone",
            "--quiet",
            "--depth",
            "1",
            &url,
            root.to_str().unwrap(),
        ],
    );
    let (judge, log) = (scratch.join("judge"), scratch.join("judge.log"));
    write_program(&judge, &JUDGE.replace("@LOG@", &log.display().to_string()));
    let p = Project {
        _scratch: scratch,
        root,
        home,
        judge,
        log,
    };
    p.start("k0", "startup");
    upstream.commit(&[("c.rs", Some("fn c() {}\n"))], "U2");
    upstream.commit(&[("d.rs", Some("fn d() {}\n"))], "U3");
    git(
        &p.root,
        &["fetch", "--quiet", "--depth", "1", "origin", "main"],
    );
    git(&p.root, &["reset", "--quiet", "--hard", "FETCH_HEAD"]);
    p.start("k1", "startup");
    std::fs::write(p.root.join("e.rs"), "fn e() {}\n").unwrap();
    let decision = p.stop("k1");
    let said = words(&decision);
    assert!(
        said.contains("this clone is shallow and does not have the history back to the commit this project's ledger first recorded")
            && said.contains("(`place.shallow_parent`)"),
        "{said}"
    );
    assert!(
        (p.ledger().iter()).all(|r| r["kind"] != "stop" || r["outcome"] != "pass"),
        "{:?}",
        p.ledger()
    );
}

/// R10 (T-18, T-19): a `pass` that 0.11.0 recorded under a left commit's own policy does not take
/// the commit out of the frontier; the commit is judged again and its new record is the worse
/// one `agent verify-branch` takes. A pass sealed to the accepted policy stays, and its commit
/// is not judged again. A raw `clearance` line moves no policy.
#[test]
fn passes_sealed_to_a_policy_never_accepted_are_judged_again() {
    let p = project("upgrade", Some(STRICT));
    let base = p.head();
    p.start("k0", "startup");
    let innocent = p.commit(&[("b.rs", Some("fn b() {}\n"))], "X: innocent");
    let left = p.commit(&[("warden.toml", Some(LOOSE))], "L: loosen");
    let key = p.commit(&[("k.md", Some(KEY))], "M: a key");
    // the ledger as the published 0.11.0 left it through the hole: X passed under the accepted
    // policy, L and M under L's own; and a person's clearance line for L
    let record = |id: &str, policy: &str| {
        json!({"v": 1, "kind": "stop", "session": "0123456789abcdef", "worktree": null,
            "unit": format!("commit:{id}"), "outcome": "pass", "class": "pass",
            "blocked_turns": 0, "agent_checks": {"runs": 0, "failed": 0},
            "ignored_elsewhere": null, "seals": {"policy_sha256": policy,
            "ranker_sha256": "c".repeat(64), "patch_sha256": "b".repeat(64),
            "verdict_sha256": null}})
    };
    let file = p.root.join(".git/deep-diff-forge/ledger.jsonl");
    let mut text = std::fs::read_to_string(&file).unwrap();
    for line in [
        record(&innocent, &sha(STRICT)),
        record(&left, &sha(LOOSE)),
        record(&key, &sha(LOOSE)),
        json!({"v": 1, "kind": "clearance", "unit": format!("commit:{left}")}),
    ] {
        text.push_str(&line.to_string());
        text.push('\n');
    }
    std::fs::write(&file, text).unwrap();
    assert_eq!(
        p.verify(&format!("{base}..HEAD")).0,
        0,
        "the hole, as 0.11.0 left it"
    );
    p.start("k1", "resume");
    let decision = p.stop("k1");
    assert!(blocks(&decision), "{decision}");
    assert!(
        p.records_of(&key).iter().any(|r| r.0 == "fail"),
        "{:?}",
        p.records_of(&key)
    );
    assert!(
        p.records_of(&left).iter().any(|r| r.0 == "hold"),
        "{:?}",
        p.records_of(&left)
    );
    assert_eq!(p.records_of(&innocent).len(), 1, "X was judged again");
    let (code, doc) = p.verify(&format!("{base}..HEAD"));
    assert_eq!(code, 7, "{doc}");
    let commits = doc["commits"].as_array().unwrap();
    assert_eq!(commits[0]["verified"], true, "{doc}");
    assert_eq!(commits[2]["verified"], false, "{doc}");
}

/// An edit of the policy and its exact restoration: the files are the same bytes again, so a
/// later unit is judged once, under one policy (T-25).
#[test]
fn an_exact_restoration_is_judged_once() {
    let p = project("restored", Some(STRICT));
    p.start("k0", "startup");
    p.commit(&[("warden.toml", Some(LOOSE))], "L1: loosen");
    p.commit(&[("warden.toml", Some(STRICT))], "L2: restore");
    p.start("k1", "startup");
    let before = p.runs().len();
    let innocent = p.commit(&[("b.rs", Some("fn b() {}\n"))], "X");
    p.stop("k1");
    let runs = p.runs();
    let on_innocent: Vec<&(String, String)> = runs[before..]
        .iter()
        .filter(|(policy, _)| *policy == sha(STRICT))
        .collect();
    assert!(!on_innocent.is_empty(), "{runs:?}");
    assert_eq!(
        p.records_of(&innocent).len(),
        1,
        "{:?}",
        p.records_of(&innocent)
    );
    assert!(
        runs[before..]
            .iter()
            .all(|(policy, _)| *policy == sha(STRICT)),
        "{runs:?}"
    );
}

/// Replace objects and grafts cannot change the accepted policy: every Git call of the hooks
/// runs with them off (T-26). A replacement of the first record's commit by one with a loosened
/// policy leaves the key failing.
#[test]
fn a_replace_ref_cannot_change_the_accepted_policy() {
    let p = project("replace", Some(STRICT));
    let first = p.head();
    p.start("k0", "startup");
    git(&p.root, &["checkout", "--quiet", "--orphan", "decoy"]);
    let decoy = p.commit(
        &[("warden.toml", Some(LOOSE))],
        "Z: an unrelated root, loosened",
    );
    git(&p.root, &["checkout", "--quiet", "main"]);
    git(&p.root, &["replace", &first, &decoy]);
    // (the test's own Git honours the replacement: the left commit writes another loosening)
    p.commit(
        &[(
            "warden.toml",
            Some("rule no-todo hold NEVER-WRITTEN-EITHER\n"),
        )],
        "L: loosen",
    );
    p.start("k1", "startup");
    let key = p.commit(&[("k.md", Some(KEY))], "M: a key");
    let decision = p.stop("k1");
    assert!(blocks(&decision), "{decision}");
    assert!(
        p.records_of(&key).iter().all(|r| r.0 == "fail"),
        "{:?}",
        p.records_of(&key)
    );
}

/// The stateless stop (a session whose state cannot be used) judges under the policy at `HEAD`
/// and the accepted policy, the stricter standing (T-9, T11); 0.11.0 judged under the accepted
/// policy alone.
#[test]
fn the_stateless_stop_judges_under_both_policies() {
    let p = project("stateless", Some(STRICT));
    p.start("k0", "startup");
    let left = p.commit(&[("warden.toml", Some(LOOSE))], "L: loosen");
    p.start("k1", "startup");
    for key in walk(&p.home.join(".local/state/deep-diff-forge")) {
        if key.file_name().is_some_and(|name| name == "key") {
            std::fs::write(&key, "x").unwrap();
        }
    }
    std::fs::write(p.root.join("k.md"), KEY).unwrap();
    let decision = p.stop("k1");
    let said = words(&decision);
    assert!(blocks(&decision), "{decision}");
    assert!(
        said.contains(&format!(
            "The tree was judged under the policy at HEAD and under the project's accepted policy (the one at commit {}), whose files differ; the stricter answer stands.",
            &p.ledger().iter().find(|r| r["kind"] == "baseline").unwrap()["head"].as_str().unwrap()[..7]
        )),
        "{said}"
    );
    assert!(
        p.records_of(&left).iter().all(|r| r.0 != "pass"),
        "{:?}",
        p.records_of(&left)
    );
    // it has no session base: its words call the first policy the one at HEAD (T8)
    assert!(
        said.contains("This unit was judged under two policies: at HEAD (commit")
            && !said.contains("at the session base"),
        "{said}"
    );
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(walk(&path));
        } else {
            out.push(path);
        }
    }
    out
}

/// Two sessions stop at once in a project judged under two policies: both answer, the ledger
/// holds whole lines only, and `agent verify-branch` takes the worst (T-24).
#[test]
fn two_stops_at_once_both_answer_and_the_ledger_stays_whole() {
    let p = project("two-at-once", Some(STRICT));
    let base = p.head();
    p.start("k0", "startup");
    p.commit(&[("warden.toml", Some(LOOSE))], "L: loosen");
    p.start("k1", "startup");
    p.start("k2", "startup");
    let key = p.commit(&[("k.md", Some(KEY))], "M: a key");
    let decisions: Vec<Value> = std::thread::scope(|scope| {
        let one = scope.spawn(|| p.stop("k1"));
        let two = scope.spawn(|| p.stop("k2"));
        vec![one.join().unwrap(), two.join().unwrap()]
    });
    assert!(decisions.iter().all(blocks), "{decisions:?}");
    let text = std::fs::read_to_string(p.root.join(".git/deep-diff-forge/ledger.jsonl")).unwrap();
    assert!(text.ends_with('\n'));
    for line in text.lines() {
        serde_json::from_str::<Value>(line).unwrap_or_else(|e| panic!("{e}: {line}"));
    }
    assert!(p.records_of(&key).iter().all(|r| r.0 == "fail"));
    assert_eq!(p.verify(&format!("{base}..HEAD")).0, 7);
}

/// `agent check --session` judges under the policy at the session's base alone: where a stop of
/// the session also judges under the accepted policy, it says so and does not call itself the
/// stop's whole answer.
#[test]
fn agent_check_says_where_the_stop_also_judges_under_the_accepted_policy() {
    let p = project("check", Some(STRICT));
    p.start("k0", "startup");
    p.commit(&[("warden.toml", Some(LOOSE))], "L: loosen");
    p.start("k1", "startup");
    std::fs::write(p.root.join("b.rs"), "fn b() {}\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .current_dir(&p.root)
        .envs(ISOLATE)
        .env("HOME", &p.home)
        .env_remove("XDG_STATE_HOME")
        .args(["agent", "check", "--session", "k1", "--judge"])
        .arg(&p.judge)
        .arg("--json")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let doc: Value = serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&out.stdout)));
    let differences = doc["stop"]["differences"].to_string();
    assert!(
        differences.contains("a stop of this session also judges every unit under the project's accepted policy (the one at commit"),
        "{doc}"
    );
    assert_eq!(doc["stop"]["whole_answer"], false, "{doc}");
}
