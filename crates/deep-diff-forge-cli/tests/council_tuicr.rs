// Review council: DDF→tuicr export through the actual CLI with fake providers, a fake tuicr and one opt-in real-tuicr round trip.
// Design/status: docs/REVIEW_COUNCIL.md#tuicr-export
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Comparisons%2FArchitecture%20Lessons%20and%20Interoperability
//! Admission, identity, anchoring, idempotency and refusal controls for `council export-tuicr`, and
//! the decision-journal controls for `council import-tuicr` (dispositions, conflicts, tombstones,
//! stale revisions, tamper and truncation). The real-tuicr tests run only with `DDF_TUICR_IT=1`,
//! against an isolated XDG data/config store.
#![expect(
    clippy::disallowed_methods,
    reason = "integration test: drives real processes, the environment and wall-clock time"
)]

use deep_diff_forge_review::Profile;
use std::io::Write as _;
use std::os::unix::fs::PermissionsExt;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
const SOURCE: &str = "fn a() {}\nfn b() {}\nfn c() {}\nfn d() {}\n";
/// Notes for the fixture run: two summaries plus three findings.
const NOTES: usize = 5;

struct Fixture {
    root: PathBuf,
    repository: PathBuf,
    run: PathBuf,
    bin: PathBuf,
    fake_tuicr: PathBuf,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn git(repository: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args([
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
        ])
        .args(args)
        .current_dir(repository)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}");
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}
fn executable(path: &Path, text: &str) {
    fs::write(path, text).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}
impl Fixture {
    /// Prepare a run and execute both blind reviews with fake providers that emit cited findings.
    fn new() -> Self {
        Self::with_repository(|_| {})
    }
    /// As [`Fixture::new`], with `setup` applied to the committed repository before the run is prepared.
    fn with_repository(setup: impl FnOnce(&Path)) -> Self {
        let root = std::env::temp_dir().join(format!(
            "ddf-tuicr-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let repository = root.join("repo");
        fs::create_dir(&repository).unwrap();
        fs::write(repository.join("x.rs"), SOURCE).unwrap();
        git(&repository, &["init", "-q"]);
        git(&repository, &["add", "x.rs"]);
        git(&repository, &["commit", "-qm", "fixture"]);
        setup(&repository);
        let analysis = root.join("analysis.json");
        let snapshot =
            deep_diff_forge_core::changed_source_snapshot_id([("x.rs", Some(SOURCE.as_bytes()))]);
        fs::write(
            &analysis,
            serde_json::to_vec(&serde_json::json!({
                "schema": "deep-diff-forge.analysis.v1",
                "review_id": "0".repeat(64),
                "candidate_snapshot": snapshot,
                "coverage": [{"path": "x.rs", "coverage": "analyzed"}],
                "review": {"files": [{"path": "x.rs", "status": "modified"}]}
            }))
            .unwrap(),
        )
        .unwrap();
        let decision = root.join("decision.md");
        fs::write(&decision, "Assess the API boundary.\nKeep dissent.\n").unwrap();
        let mut spec = deep_diff_forge_review::cli::example();
        spec.profile = Profile::Roster;
        spec.reviewers.truncate(2);
        spec.repository = repository.display().to_string();
        spec.analysis_file = analysis.display().to_string();
        spec.decision_file = decision.display().to_string();
        spec.timeout_seconds = 10;
        let request = root.join("request.json");
        fs::write(&request, serde_json::to_vec(&spec).unwrap()).unwrap();
        let bin = root.join("fake-bin");
        fs::create_dir(&bin).unwrap();
        for name in ["codex", "claude"] {
            executable(&bin.join(name), FAKE_PROVIDER);
        }
        let fake_tuicr = root.join("fake-tuicr");
        fs::create_dir(&fake_tuicr).unwrap();
        executable(&fake_tuicr.join("tuicr"), FAKE_TUICR);
        let f = Self {
            run: root.join("run"),
            root,
            repository,
            bin,
            fake_tuicr,
        };
        let run = f.run.to_str().unwrap().to_owned();
        let prepared = f.council(&["prepare", request.to_str().unwrap(), &run]);
        assert!(prepared.status.success(), "{}", stderr(&prepared));
        for id in ["correctness-codex", "security-claude"] {
            let r = f.council(&["review", &run, id, "independent", "--execute"]);
            assert!(r.status.success(), "{}", stderr(&r));
        }
        f
    }
    fn council(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
            .arg("council")
            .args(args)
            .env(
                "PATH",
                format!("{}:{}", self.bin.display(), std::env::var("PATH").unwrap()),
            )
            .output()
            .unwrap()
    }
    fn tuicr_log(&self) -> PathBuf {
        self.root.join("tuicr-log.jsonl")
    }
    /// `export-tuicr` with the fake tuicr first on PATH, in the given fake mode.
    fn export(&self, session: &Path, flags: &[&str], mode: &str) -> Output {
        Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
            .args(["council", "export-tuicr", self.run.to_str().unwrap()])
            .args(["--session", session.to_str().unwrap()])
            .args(flags)
            .env("DDF_FAKE_TUICR_LOG", self.tuicr_log())
            .env("DDF_FAKE_TUICR_MODE", mode)
            .env("DDF_FAKE_TUICR_SESSION", session)
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    self.fake_tuicr.display(),
                    std::env::var("PATH").unwrap()
                ),
            )
            .output()
            .unwrap()
    }
    /// A minimal tuicr 1.3 local session over the fixture repository; `edit` adjusts fields.
    fn session(&self, name: &str, edit: impl FnOnce(&mut serde_json::Value)) -> PathBuf {
        let mut value = session_json(
            &self.repository,
            &git(&self.repository, &["rev-parse", "HEAD"]),
        );
        edit(&mut value);
        let path = self.root.join(name);
        fs::write(&path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
        path
    }
    fn ledger_files(&self) -> Vec<String> {
        let dir = self
            .run
            .join("exports/tuicr/00000000-0000-4000-8000-000000000001");
        let mut names: Vec<String> = fs::read_dir(dir)
            .map(|d| {
                d.map(|e| e.unwrap().file_name().into_string().unwrap())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }
    fn tuicr_calls(&self) -> Vec<serde_json::Value> {
        fs::read_to_string(self.tuicr_log())
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }
}
/// Fields from tuicr's `ReviewSession`/`FileReview` (tuicr 0.27.0 `src/model/review.rs`).
fn session_json(repository: &Path, head: &str) -> serde_json::Value {
    serde_json::json!({
        "id": "00000000-0000-4000-8000-000000000001",
        "version": "1.3",
        "repo_path": repository,
        "branch_name": null,
        "base_commit": head,
        "diff_source": "working_tree",
        "commit_range": null,
        "pr_session_key": null,
        "created_at": "2026-10-25T00:00:00Z",
        "updated_at": "2026-10-25T00:00:00Z",
        "review_comments": [],
        "files": {"x.rs": {
            "path": "x.rs", "reviewed": false, "status": "modified",
            "file_comments": [], "line_comments": {}, "reviewed_hunks": [], "content_hash": 0
        }},
        "session_notes": null
    })
}
fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}
fn json(o: &Output) -> serde_json::Value {
    assert!(o.status.success(), "{}", stderr(o));
    serde_json::from_slice(&o.stdout).unwrap()
}
fn refused(o: &Output, needle: &str) {
    assert_eq!(o.status.code(), Some(4), "{}", stderr(o));
    assert!(
        stderr(o).contains(needle),
        "expected {needle:?} in {}",
        stderr(o)
    );
}
fn note<'a>(result: &'a serde_json::Value, key: &str) -> &'a serde_json::Value {
    result["notes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["key"] == key)
        .unwrap_or_else(|| panic!("missing note {key}"))
}

const FAKE_PROVIDER: &str = r"#!/usr/bin/python3
import sys,json,os
prompt=sys.stdin.read()
parts=prompt.splitlines()[0].split()
reviewer=parts[3];run=parts[6].rstrip('.');phase=parts[8].rstrip('.')
if reviewer=='correctness-codex':
    findings=[{'id':'correctness-codex:range','severity':'high','claim':'Range claim','citations':[{'artifact_id':'candidate:x.rs','start_line':2,'end_line':3},{'artifact_id':'decision','start_line':1,'end_line':1}],'falsifier':'Run the range test','requested_checks':['tests']},
              {'id':'correctness-codex:decision','severity':'low','claim':'Decision claim','citations':[{'artifact_id':'decision','start_line':1,'end_line':2}],'falsifier':'Reread the decision','requested_checks':[]}]
else:
    findings=[{'id':'security-claude:line','severity':'medium','claim':'Line claim','citations':[{'artifact_id':'candidate:x.rs','start_line':4,'end_line':4}],'falsifier':'Fuzz the parser','requested_checks':['fuzz']}]
report={'run_id':run,'reviewer_id':reviewer,'round':phase,'verdict':'changes_requested','summary':'SUMMARY '+reviewer,'findings':findings,'responses':[]}
if os.path.basename(sys.argv[0])=='codex':
    sys.stderr.write('workdir: /w\nmodel: gpt-fake-codex\n')
    with open(sys.argv[sys.argv.index('--output-last-message')+1],'w') as file:json.dump(report,file)
    print(json.dumps(report))
else:
    print(json.dumps({'is_error':False,'structured_output':report,'modelUsage':{'claude-fake-opus':{'inputTokens':1}}}))
";
/// Records argv/stdin; `review add` appends to the session file like tuicr's store does.
const FAKE_TUICR: &str = r"#!/usr/bin/python3
import json,os,sys
log=os.environ['DDF_FAKE_TUICR_LOG'];mode=os.environ.get('DDF_FAKE_TUICR_MODE','normal')
args=sys.argv[1:];data=sys.stdin.read()
with open(log,'a') as f:f.write(json.dumps({'argv':args,'stdin':data})+'\n')
if args[:3]==['review','list','--repo'] and len(args)==4:
    listed=[] if mode=='unlisted' else [{'path':os.environ['DDF_FAKE_TUICR_SESSION'],'active':mode=='active'}]
    print(json.dumps(listed));sys.exit(0)
if len(args)!=6 or args[:3]!=['review','add','--session'] or args[4:]!=['--input','-']:sys.exit(64)
if mode=='fail':sys.stderr.write('boom: storage locked\n');sys.exit(1)
p=json.loads(data);t=p['target'];session=json.load(open(args[3]))
cid='fake-%d'%sum(1 for _ in open(log))
comment={'id':cid,'content':p['content'].strip(),'author':p['username'],'comment_type':p['type'],'lifecycle_state':'local_draft','side':t.get('side')}
if t['type']=='review':
    session['review_comments'].append(comment);where=(None,None,None)
else:
    start=t.get('start_line',t.get('line'));end=t.get('end_line',t.get('line'))
    session['files'][t['file']]['line_comments'].setdefault(str(end),[]).append(comment);where=(t['file'],start,end)
if mode!='diverge':json.dump(session,open(args[3],'w'))
print(json.dumps({'id':cid,'location':'x','path':where[0],'start_line':where[1],'end_line':where[2],'side':t.get('side') if where[0] else None,'comment_type':p['type'],'author':p['username'],'lifecycle_state':'local_draft','created_at':'now','content':comment['content']}))
";

#[test]
fn dry_run_plans_new_side_anchors_with_provenance_and_writes_nothing() {
    let f = Fixture::new();
    let session = f.session("session.json", |_| {});
    let before = fs::read(&session).unwrap();
    let result = json(&f.export(&session, &[], "normal"));
    assert_eq!(result["executed"], false);
    assert_eq!(result["session_revision"], "working_tree");
    assert_eq!(result["notes"].as_array().unwrap().len(), NOTES);
    let range = note(
        &result,
        "correctness-codex/independent/finding/correctness-codex:range",
    );
    assert_eq!(
        range["target"],
        serde_json::json!({"type":"line_range","file":"x.rs","start_line":2,"end_line":3,"side":"new"})
    );
    assert_eq!(
        range["author"],
        "council:correctness-codex (gpt-fake-codex)"
    );
    assert_eq!(range["status"], "planned");
    let line = note(
        &result,
        "security-claude/independent/finding/security-claude:line",
    );
    assert_eq!(
        line["target"],
        serde_json::json!({"type":"line","file":"x.rs","line":4,"side":"new"})
    );
    assert_eq!(line["author"], "council:security-claude (claude-fake-opus)");
    // A finding citing only non-file artifacts is a review-level note, not a downgrade.
    let decision = note(
        &result,
        "correctness-codex/independent/finding/correctness-codex:decision",
    );
    assert_eq!(decision["target"], serde_json::json!({"type":"review"}));
    assert!(decision["downgraded"].is_null());
    let run_id = result["run_id"].as_str().unwrap();
    for n in result["notes"].as_array().unwrap() {
        let content = n["content"].as_str().unwrap();
        for needle in [
            &format!("run={run_id}") as &str,
            &format!("reviewer={}", n["reviewer_id"].as_str().unwrap()),
            "round=independent",
            "finding=",
            "severity=",
            "observed model: ",
            "Model claim, not verified fact",
        ] {
            assert!(content.contains(needle), "{needle} missing from {content}");
        }
    }
    assert!(
        range["content"]
            .as_str()
            .unwrap()
            .contains("finding=correctness-codex:range")
    );
    assert!(range["content"].as_str().unwrap().contains("severity=HIGH"));
    assert_eq!(fs::read(&session).unwrap(), before);
    assert!(!f.run.join("exports").exists());
    assert!(!f.tuicr_log().exists(), "dry run must not invoke tuicr");
}

#[test]
fn execute_writes_each_note_once_and_rerun_is_a_noop() {
    let f = Fixture::new();
    let session = f.session("session.json", |_| {});
    let first = json(&f.export(&session, &["--execute"], "normal"));
    assert_eq!(first["executed"], true);
    let notes = first["notes"].as_array().unwrap();
    assert!(
        notes
            .iter()
            .all(|n| n["status"] == "created" && n["tuicr_comment_id"].is_string())
    );
    // Each note leaves a create-new intent marker and a create-new completion record.
    let ledger = f.ledger_files();
    let records = |ext: &str| {
        ledger
            .iter()
            .filter(|n| Path::new(n).extension().is_some_and(|e| e == ext))
            .count()
    };
    assert_eq!((records("json"), records("pending")), (NOTES, NOTES));
    let calls = f.tuicr_calls();
    assert_eq!(calls[0]["argv"][1], "list");
    let adds: Vec<_> = calls.iter().filter(|c| c["argv"][1] == "add").collect();
    assert_eq!(adds.len(), NOTES);
    // Only list/add with fixed argv; nothing marks reviewed, resolves, deletes or submits.
    assert!(
        calls
            .iter()
            .all(|c| c["argv"][0] == "review" && (c["argv"][1] == "list" || c["argv"][1] == "add"))
    );
    for add in adds {
        let payload: serde_json::Value =
            serde_json::from_str(add["stdin"].as_str().unwrap()).unwrap();
        assert_eq!(payload["type"], "note");
        if payload["target"]["type"] != "review" {
            assert_eq!(payload["target"]["side"], "new");
        }
    }
    let stored: serde_json::Value = serde_json::from_slice(&fs::read(&session).unwrap()).unwrap();
    assert_eq!(stored["review_comments"].as_array().unwrap().len(), 3);
    assert_eq!(
        stored["files"]["x.rs"]["line_comments"]["3"][0]["side"],
        "new"
    );
    assert_eq!(stored["files"]["x.rs"]["reviewed"], false);

    let second = json(&f.export(&session, &["--execute"], "normal"));
    assert!(
        second["notes"]
            .as_array()
            .unwrap()
            .iter()
            .all(|n| n["status"] == "already_exported")
    );
    let adds_after = f
        .tuicr_calls()
        .iter()
        .filter(|c| c["argv"][1] == "add")
        .count();
    assert_eq!(adds_after, NOTES, "rerun must not add comments");
    assert_eq!(f.ledger_files().len(), 2 * NOTES);
    let dry = json(&f.export(&session, &[], "normal"));
    assert!(
        dry["notes"]
            .as_array()
            .unwrap()
            .iter()
            .all(|n| n["status"] == "already_exported")
    );
}

#[test]
fn tampered_reports_are_refused_and_manual_imports_are_skipped() {
    let f = Fixture::new();
    let session = f.session("session.json", |_| {});
    let path = f.run.join("reports/correctness-codex-independent.json");
    let original = fs::read(&path).unwrap();
    let mut report: serde_json::Value = serde_json::from_slice(&original).unwrap();
    report["findings"][0]["claim"] = "forged claim".into();
    fs::write(&path, serde_json::to_vec(&report).unwrap()).unwrap();
    refused(
        &f.export(&session, &["--execute"], "normal"),
        "not admitted",
    );
    assert!(!f.tuicr_log().exists());
    fs::write(&path, &original).unwrap();
    // Without a controller qualification the report is a manual import: visible, never exported.
    fs::remove_file(
        f.run
            .join("jobs/review-security-claude-independent/qualification.json"),
    )
    .unwrap();
    let result = json(&f.export(&session, &[], "normal"));
    assert_eq!(
        result["skipped_reports"][0]["report"],
        "reports/security-claude-independent.json"
    );
    assert_eq!(result["notes"].as_array().unwrap().len(), 3);
    fs::write(
        f.run
            .join("jobs/review-correctness-codex-independent/stderr.log"),
        "model: forged\n",
    )
    .unwrap();
    refused(
        &f.export(&session, &[], "normal"),
        "transcript does not match",
    );
}

/// A session-field edit for one refusal case.
type Edit = Box<dyn FnOnce(&mut serde_json::Value)>;
#[test]
fn foreign_forge_or_unknown_sessions_are_refused() {
    let f = Fixture::new();
    let other = f.root.join("other");
    fs::create_dir(&other).unwrap();
    let cases: [(&str, Edit, &str); 5] = [
        (
            "other.json",
            Box::new(move |s| s["repo_path"] = other.display().to_string().into()),
            "repository mismatch",
        ),
        (
            "pr.json",
            Box::new(|s| s["pr_session_key"] = serde_json::json!({"number": 1})),
            "forge session",
        ),
        (
            "pull.json",
            Box::new(|s| s["diff_source"] = "pull_request".into()),
            "forge session",
        ),
        (
            "old.json",
            Box::new(|s| s["version"] = "1.2".into()),
            "unsupported session",
        ),
        (
            "new.json",
            Box::new(|s| s["diff_source"] = "something_new".into()),
            "unsupported session",
        ),
    ];
    for (name, edit, needle) in cases {
        let session = f.session(name, edit);
        refused(&f.export(&session, &["--execute"], "normal"), needle);
    }
    assert!(!f.tuicr_log().exists());
    assert!(!f.run.join("exports").exists());
}

#[test]
fn changed_session_bytes_refuse_unless_review_level_is_explicit() {
    let f = Fixture::new();
    let session = f.session("session.json", |_| {});
    fs::write(f.repository.join("x.rs"), format!("{SOURCE}fn e() {{}}\n")).unwrap();
    refused(&f.export(&session, &["--execute"], "normal"), "unanchored");
    assert!(!f.tuicr_log().exists());
    let result = json(&f.export(&session, &["--allow-review-level"], "normal"));
    for key in [
        "correctness-codex/independent/finding/correctness-codex:range",
        "security-claude/independent/finding/security-claude:line",
    ] {
        let n = note(&result, key);
        assert_eq!(n["target"], serde_json::json!({"type":"review"}));
        assert!(
            n["downgraded"]
                .as_str()
                .unwrap()
                .contains("differ from the sealed snapshot")
        );
        assert!(
            n["content"]
                .as_str()
                .unwrap()
                .contains("downgraded to review level")
        );
    }
    // A file the session does not list cannot carry a line note either.
    fs::write(f.repository.join("x.rs"), SOURCE).unwrap();
    let unlisted = f.session("unlisted.json", |s| s["files"] = serde_json::json!({}));
    refused(
        &f.export(&unlisted, &[], "normal"),
        "does not list this file",
    );
}

#[test]
fn session_bytes_follow_the_diff_source_revision() {
    let f = Fixture::new();
    let head = git(&f.repository, &["rev-parse", "HEAD"]);
    // Unstaged edit: the working tree differs, the index and HEAD still equal the sealed snapshot.
    fs::write(f.repository.join("x.rs"), "fn changed() {}\n").unwrap();
    let working = f.session("working.json", |_| {});
    refused(&f.export(&working, &[], "normal"), "unanchored");
    let staged = f.session("staged.json", |s| s["diff_source"] = "staged".into());
    assert_eq!(
        json(&f.export(&staged, &[], "normal"))["session_revision"],
        "index"
    );
    let commits = f.session("commits.json", |s| {
        s["diff_source"] = "commit_range".into();
        s["commit_range"] = serde_json::json!([head]);
    });
    assert_eq!(
        json(&f.export(&commits, &[], "normal"))["session_revision"],
        format!("commit:{head}")
    );
    let reordered = f.session("reordered.json", |s| {
        s["diff_source"] = "commit_range".into();
        s["commit_range"] = serde_json::json!([head, "f".repeat(40)]);
    });
    refused(&f.export(&reordered, &[], "normal"), "newest commit");
    git(&f.repository, &["add", "x.rs"]);
    refused(&f.export(&staged, &[], "normal"), "unanchored");
    git(&f.repository, &["commit", "-qm", "change"]);
    let newer = git(&f.repository, &["rev-parse", "HEAD"]);
    let later = f.session("later.json", |s| {
        s["diff_source"] = "commit_range".into();
        s["base_commit"] = newer.clone().into();
        s["commit_range"] = serde_json::json!([newer]);
    });
    refused(&f.export(&later, &[], "normal"), "unanchored");
}

#[test]
fn tuicr_failure_is_typed_keeps_stderr_and_leaves_no_ledger() {
    let f = Fixture::new();
    let session = f.session("session.json", |_| {});
    let failed = f.export(&session, &["--execute"], "fail");
    refused(&failed, "tuicr failed");
    assert!(stderr(&failed).contains("boom: storage locked"));
    assert!(f.ledger_files().is_empty(), "{:?}", f.ledger_files());
    let result = json(&f.export(&session, &["--execute"], "normal"));
    assert!(
        result["notes"]
            .as_array()
            .unwrap()
            .iter()
            .all(|n| n["status"] == "created")
    );
}

#[test]
fn active_sessions_accept_verified_exports() {
    // tuicr is designed for collaborative tools to add to a session open in the TUI.
    let f = Fixture::new();
    let session = f.session("session.json", |_| {});
    let out = json(&f.export(&session, &["--execute"], "active"));
    assert_eq!(out["executed"], true);
    assert!(
        out["notes"]
            .as_array()
            .unwrap()
            .iter()
            .all(|n| n["status"] == "created" && n["tuicr_comment_id"].is_string())
    );
}

#[test]
fn unstored_diverged_or_missing_tuicr_is_refused() {
    let f = Fixture::new();
    let session = f.session("session.json", |_| {});
    refused(
        &f.export(&session, &["--execute"], "unlisted"),
        "not in store",
    );
    assert!(f.tuicr_calls().iter().all(|c| c["argv"][1] == "list"));
    let diverged = f.export(&session, &["--execute"], "diverge");
    refused(&diverged, "diverged");
    // The intent marker survives, so a rerun refuses instead of risking a duplicate.
    assert_eq!(f.ledger_files().len(), 1);
    assert!(f.ledger_files()[0].ends_with(".pending"));
    refused(&f.export(&session, &["--execute"], "normal"), "interrupted");
    let missing = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .args([
            "council",
            "export-tuicr",
            f.run.to_str().unwrap(),
            "--session",
        ])
        .arg(&session)
        .arg("--execute")
        .env("PATH", "/usr/bin:/bin")
        .output()
        .unwrap();
    refused(&missing, "tuicr unavailable");
    refused(
        &f.export(&session, &["--execute", "--execute"], "normal"),
        "usage",
    );
}

/// The first absolute `tuicr` on PATH, as the adapter resolves it.
fn real_tuicr() -> PathBuf {
    std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|d| d.join("tuicr"))
        .find(|p| p.is_file())
        .expect("DDF_TUICR_IT=1 requires tuicr on PATH")
}

/// Real tuicr and the CLI confined to a private HOME and XDG data/config/state/cache.
struct Isolated {
    f: Fixture,
    tuicr: PathBuf,
    home: PathBuf,
    data: PathBuf,
}
impl Isolated {
    fn new() -> Self {
        let f = Fixture::new();
        let (home, data) = (f.root.join("home"), f.root.join("data"));
        for dir in [&home, &data, &f.root.join("config")] {
            fs::create_dir(dir).unwrap();
        }
        Self {
            tuicr: real_tuicr(),
            f,
            home,
            data,
        }
    }
    /// tuicr resolves its store with `directories::ProjectDirs` (XDG on Linux) and its config likewise.
    fn isolate<'c>(&self, c: &'c mut Command) -> &'c mut Command {
        c.env("HOME", &self.home)
            .env("XDG_DATA_HOME", &self.data)
            .env("XDG_CONFIG_HOME", self.f.root.join("config"))
            .env("XDG_STATE_HOME", self.f.root.join("state"))
            .env("XDG_CACHE_HOME", self.f.root.join("cache"))
    }
    fn tuicr(&self, args: &[&str], input: &str) -> serde_json::Value {
        let mut c = Command::new(&self.tuicr);
        c.args(args).current_dir(&self.f.repository);
        let mut child = self
            .isolate(&mut c)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success(), "tuicr {args:?}: {}", stderr(&out));
        serde_json::from_slice(&out.stdout).unwrap()
    }
    fn comments(&self, session: &Path) -> Vec<serde_json::Value> {
        let listed = self.tuicr(
            &["review", "comments", "--session", session.to_str().unwrap()],
            "",
        );
        listed.as_array().unwrap().clone()
    }
    fn export(&self, session: &Path, flags: &[&str]) -> Output {
        let mut c = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"));
        c.args(["council", "export-tuicr", self.f.run.to_str().unwrap()])
            .arg("--session")
            .arg(session)
            .args(flags);
        self.isolate(&mut c).output().unwrap()
    }
    /// tuicr has no non-interactive session creation; `review add` on a 1.3 file saves it into the store.
    fn stored_session(&self) -> PathBuf {
        let seed = self.f.session("seed.json", |_| {});
        self.tuicr(
            &["review", "add", "--session", seed.to_str().unwrap(), "--input", "-"],
            r#"{"type":"note","username":"fixture","content":"disposable fixture session","target":{"type":"review"}}"#,
        );
        fs::remove_file(&seed).unwrap();
        let listed = self.tuicr(
            &[
                "review",
                "list",
                "--repo",
                self.f.repository.to_str().unwrap(),
            ],
            "",
        );
        let stored = PathBuf::from(listed[0]["path"].as_str().unwrap());
        assert!(
            stored.starts_with(self.data.join("tuicr/reviews")),
            "store not isolated: {}",
            stored.display()
        );
        assert!(!self.home.join(".local/share/tuicr").exists());
        stored
    }
}

#[test]
fn real_tuicr_round_trip_in_an_isolated_store() {
    if std::env::var_os("DDF_TUICR_IT").is_none() {
        // Written to the stderr handle, not `eprintln!`, so the harness does not capture it.
        let _ = writeln!(
            std::io::stderr(),
            "UNMEASURED: real tuicr round trip skipped; set DDF_TUICR_IT=1 with tuicr on PATH"
        );
        return;
    }
    let it = Isolated::new();
    let version = Command::new(&it.tuicr).arg("--version").output().unwrap();
    println!(
        "tuicr: {} {}",
        it.tuicr.display(),
        String::from_utf8_lossy(&version.stdout).trim()
    );
    let stored = it.stored_session();
    println!("isolated session: {}", stored.display());

    let first = json(&it.export(&stored, &["--execute"]));
    for n in first["notes"].as_array().unwrap() {
        println!(
            "created {} -> {} {}",
            n["key"], n["tuicr_comment_id"], n["target"]
        );
        assert_eq!(n["status"], "created");
    }
    let comments = it.comments(&stored);
    assert_eq!(comments.len(), NOTES + 1);
    let located: Vec<_> = comments
        .iter()
        .map(|c| c["location"].as_str().unwrap())
        .collect();
    println!("tuicr locations: {located:?}");
    assert!(located.contains(&"x.rs:2-3") && located.contains(&"x.rs:4"));
    for c in &comments {
        assert_eq!(c["lifecycle_state"], "local_draft");
        if c["path"].is_string() {
            assert_eq!(c["side"], "new");
        }
    }
    assert!(
        comments
            .iter()
            .any(|c| c["author"] == "council:correctness-codex (gpt-fake-codex)")
    );
    let session: serde_json::Value = serde_json::from_slice(&fs::read(&stored).unwrap()).unwrap();
    assert_eq!(session["files"]["x.rs"]["reviewed"], false);
    assert!(session["pr_session_key"].is_null());

    let again = json(&it.export(&stored, &["--execute"]));
    for n in again["notes"].as_array().unwrap() {
        assert_eq!(n["status"], "already_exported");
    }
    assert_eq!(it.comments(&stored).len(), NOTES + 1);
    println!("rerun: no-op ({} comments)", NOTES + 1);

    fs::write(it.f.repository.join("x.rs"), "fn changed() {}\n").unwrap();
    let mismatch = it.export(&stored, &["--execute"]);
    refused(&mismatch, "unanchored");
    println!("mismatched revision: {}", stderr(&mismatch).trim());
    assert_eq!(it.comments(&stored).len(), NOTES + 1);
}

// ---- Decision journal: `council import-tuicr` ----

const RANGE_KEY: &str = "correctness-codex/independent/finding/correctness-codex:range";
const LINE_KEY: &str = "security-claude/independent/finding/security-claude:line";
impl Fixture {
    fn import(&self, session: &Path, flags: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
            .args(["council", "import-tuicr", self.run.to_str().unwrap()])
            .args(["--session", session.to_str().unwrap()])
            .args(flags)
            .output()
            .unwrap()
    }
    fn journal(&self) -> PathBuf {
        self.run.join("decisions.jsonl")
    }
    fn events(&self) -> Vec<serde_json::Value> {
        fs::read_to_string(self.journal())
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }
    /// A session holding the five exported council notes, written through the fake tuicr.
    fn exported(&self) -> PathBuf {
        let session = self.session("session.json", |_| {});
        json(&self.export(&session, &["--execute"], "normal"));
        session
    }
    /// The tuicr comment id the export ledger recorded for `key`.
    fn exported_id(&self, session: &Path, key: &str) -> String {
        let out = json(&self.export(session, &[], "normal"));
        note(&out, key)["tuicr_comment_id"]
            .as_str()
            .unwrap()
            .to_owned()
    }
}
/// Rewrite a session file in place, as tuicr does when a human edits it.
fn edit_session(session: &Path, edit: impl FnOnce(&mut serde_json::Value)) {
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(session).unwrap()).unwrap();
    edit(&mut value);
    fs::write(session, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
}
fn human(id: &str, author: &str, kind: &str, content: &str) -> serde_json::Value {
    serde_json::json!({
        "id": id, "content": content, "comment_type": kind, "author": author,
        "created_at": "2026-10-25T00:00:00Z", "line_context": null, "side": "new",
        "line_range": null, "lifecycle_state": "local_draft",
        "remote_review_id": null, "remote_comment_id": null, "commit_id": null
    })
}
/// The comment `id`, review level or line level, for an in-place human edit.
fn comment_mut<'a>(session: &'a mut serde_json::Value, id: &str) -> &'a mut serde_json::Value {
    if session["review_comments"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["id"] == id)
    {
        return session["review_comments"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|c| c["id"] == id)
            .unwrap();
    }
    session["files"]
        .as_object_mut()
        .unwrap()
        .values_mut()
        .flat_map(|f| f["line_comments"].as_object_mut().unwrap().values_mut())
        .flat_map(|list| list.as_array_mut().unwrap().iter_mut())
        .find(|c| c["id"] == id)
        .unwrap_or_else(|| panic!("no comment {id}"))
}
fn kinds(events: &[serde_json::Value]) -> Vec<&str> {
    events.iter().map(|e| e["kind"].as_str().unwrap()).collect()
}

/// Human comments: an anchored issue, a tagged praise, an unlinked note and a bogus tag.
fn add_dispositions(session: &Path) {
    edit_session(session, |s| {
        // Overlaps the range note (x.rs:2-3, new side): linked by anchor.
        let mut issue = human("h-issue", "Luke", "issue", "Off by one at the boundary.");
        issue["line_range"] = serde_json::json!({"start": 3, "end": 3});
        s["files"]["x.rs"]["line_comments"]["3"]
            .as_array_mut()
            .unwrap()
            .push(issue);
        s["files"]["x.rs"]["content_hash"] = 4242.into();
        let reviews = s["review_comments"].as_array_mut().unwrap();
        // Review level, linked by an explicit tag only.
        reviews.push(human(
            "h-praise",
            "Luke",
            "praise",
            &format!("Good catch [ddf:{LINE_KEY}]"),
        ));
        reviews.push(human("h-note", "Ana", "note", "Unrelated note."));
        // A tag naming a key the export never wrote links nothing.
        reviews.push(human(
            "h-bogus",
            "Ana",
            "note",
            "[ddf:nobody/independent/finding/x]",
        ));
    });
}

#[test]
fn import_journals_human_dispositions_with_actor_and_revision_and_rerun_is_a_noop() {
    let f = Fixture::new();
    let session = f.exported();
    let head = git(&f.repository, &["rev-parse", "HEAD"]);
    let range_id = f.exported_id(&session, RANGE_KEY);
    let line_id = f.exported_id(&session, LINE_KEY);
    add_dispositions(&session);
    let before = fs::read(&session).unwrap();
    let dry = json(&f.import(&session, &[]));
    assert_eq!(dry["executed"], false);
    assert!(!f.journal().exists(), "dry run must not write");
    assert_eq!(
        fs::read(&session).unwrap(),
        before,
        "import never writes tuicr"
    );

    let out = json(&f.import(&session, &["--execute"]));
    assert_eq!(
        out["events"], dry["events"],
        "dry run plans exactly what is appended"
    );
    let events = f.events();
    assert_eq!(events, out["events"].as_array().unwrap().clone());
    assert_eq!(
        kinds(&events),
        [
            "disposition",
            "comment_added",
            "comment_added",
            "disposition"
        ]
    );
    let by_id = |id: &str| {
        events
            .iter()
            .find(|e| e["comment_id"] == id)
            .unwrap_or_else(|| panic!("no event for {id}"))
    };
    let issue = by_id("h-issue");
    assert_eq!(
        issue["actor"],
        serde_json::json!({"name": "Luke", "basis": "tuicr_author"})
    );
    assert_eq!(issue["comment_type"], "issue");
    assert_eq!(issue["revision"]["base_commit"], head.as_str());
    assert_eq!(issue["revision"]["diff_source"], "working_tree");
    assert_eq!(issue["session_id"], "00000000-0000-4000-8000-000000000001");
    assert_eq!(issue["content_hash"], 4242);
    assert_eq!(issue["ddf_authored"], false);
    assert_eq!(
        issue["links"],
        serde_json::json!([{"key": RANGE_KEY, "tuicr_comment_id": range_id, "basis": "anchor"}])
    );
    assert_eq!(
        issue["anchor"],
        serde_json::json!({"type": "lines", "file": "x.rs", "side": "new", "start_line": 3, "end_line": 3})
    );
    assert_eq!(
        by_id("h-praise")["links"],
        serde_json::json!([{"key": LINE_KEY, "tuicr_comment_id": line_id, "basis": "tag"}])
    );
    assert_eq!(by_id("h-note")["actor"]["name"], "Ana");
    assert_eq!(by_id("h-bogus")["links"], serde_json::json!([]));
    // The council's own notes are not human decisions.
    assert!(events.iter().all(|e| e["ddf_authored"] == false));
    // Chained from genesis.
    assert_eq!(events[0]["prev"], "0".repeat(64));
    for pair in events.windows(2) {
        assert_eq!(pair[1]["prev"], pair[0]["hash"]);
    }

    let journal = fs::read(f.journal()).unwrap();
    let again = json(&f.import(&session, &["--execute"]));
    assert_eq!(again["events"], serde_json::json!([]));
    assert_eq!(again["journal_events"], 4);
    assert_eq!(
        fs::read(f.journal()).unwrap(),
        journal,
        "rerun appends nothing"
    );
    assert_eq!(
        fs::read_dir(f.run.join("decisions.checkpoints"))
            .unwrap()
            .count(),
        1
    );
}

#[test]
fn stale_revision_or_bytes_refuse_the_disposition() {
    let f = Fixture::new();
    let session = f.exported();
    edit_session(&session, |s| {
        let mut issue = human("h-1", "Luke", "issue", "Boundary.");
        issue["line_range"] = serde_json::json!({"start": 2, "end": 3});
        s["files"]["x.rs"]["line_comments"]["3"]
            .as_array_mut()
            .unwrap()
            .push(issue);
    });
    // A session on another base revision.
    let stale = f.root.join("stale.json");
    fs::copy(&session, &stale).unwrap();
    edit_session(&stale, |s| s["base_commit"] = "f".repeat(40).into());
    refused(&f.import(&stale, &["--execute"]), "stale revision");
    edit_session(&stale, |s| s["base_commit"] = "pristine:abc:0".into());
    refused(&f.import(&stale, &["--execute"]), "not a full commit id");
    // The working tree moved after the run: the line disposition is on bytes the run never saw.
    fs::write(f.repository.join("x.rs"), format!("{SOURCE}fn e() {{}}\n")).unwrap();
    refused(&f.import(&session, &["--execute"]), "stale file");
    assert!(!f.journal().exists(), "a refused import records nothing");
    // A review-level disposition is refused too, because the session scope changed.
    let review = f.root.join("review.json");
    fs::copy(&session, &review).unwrap();
    edit_session(&review, |s| {
        s["files"]["x.rs"]["line_comments"]["3"]
            .as_array_mut()
            .unwrap()
            .retain(|c| c["id"] != "h-1");
        s["review_comments"].as_array_mut().unwrap().push(human(
            "h-2",
            "Luke",
            "note",
            &format!("[ddf:{RANGE_KEY}]"),
        ));
    });
    refused(&f.import(&review, &[]), "stale file");
    // Positive control: the same session on the sealed bytes is accepted.
    fs::write(f.repository.join("x.rs"), SOURCE).unwrap();
    let ok = json(&f.import(&session, &["--execute"]));
    assert_eq!(kinds(ok["events"].as_array().unwrap()), ["disposition"]);
    // A new commit moves HEAD past the run: the stored session's base no longer matches the run.
    fs::write(f.repository.join("x.rs"), "fn changed() {}\n").unwrap();
    git(&f.repository, &["commit", "-qam", "later"]);
    let newer = git(&f.repository, &["rev-parse", "HEAD"]);
    edit_session(&session, |s| s["base_commit"] = newer.into());
    refused(&f.import(&session, &["--execute"]), "stale revision");
    assert_eq!(f.events().len(), 1);
}

#[test]
fn reviewed_marks_need_the_journaled_content_hash() {
    let f = Fixture::new();
    let session = f.exported();
    edit_session(&session, |s| {
        s["files"]["x.rs"]["reviewed"] = true.into();
        s["files"]["x.rs"]["content_hash"] = serde_json::Value::Null;
    });
    refused(&f.import(&session, &["--execute"]), "no tuicr content_hash");
    edit_session(&session, |s| {
        s["files"]["x.rs"]["content_hash"] = 111.into();
    });
    let first = json(&f.import(&session, &["--execute"]));
    let mark = &first["events"][0];
    assert_eq!(mark["kind"], "file_review");
    assert_eq!(mark["reviewed"], true);
    assert_eq!(mark["content_hash"], 111);
    assert_eq!(
        mark["anchor"],
        serde_json::json!({"type": "file", "file": "x.rs"})
    );
    assert_eq!(mark["actor"]["basis"], "unrecorded");
    // A further mark under a different tuicr diff hash is refused.
    edit_session(&session, |s| {
        s["files"]["x.rs"]["reviewed_hunks"] = serde_json::json!(["h1"]);
        s["files"]["x.rs"]["content_hash"] = 222.into();
    });
    refused(&f.import(&session, &["--execute"]), "stale review mark");
    // Positive control: the same mark under the journaled hash is accepted and supersedes the first.
    edit_session(&session, |s| {
        s["files"]["x.rs"]["content_hash"] = 111.into();
    });
    let second = json(&f.import(&session, &["--execute"]));
    assert_eq!(
        second["events"][0]["reviewed_hunks"],
        serde_json::json!(["h1"])
    );
    assert_eq!(second["events"][0]["supersedes"], mark["hash"]);
    // A reviewed mark on bytes the run did not seal is a stale disposition.
    edit_session(&session, |s| {
        s["files"]["x.rs"]["reviewed_hunks"] = serde_json::json!([]);
    });
    fs::write(f.repository.join("x.rs"), "fn changed() {}\n").unwrap();
    refused(&f.import(&session, &["--execute"]), "stale file");
    assert_eq!(f.events().len(), 2);
}

#[test]
fn tampered_or_truncated_journals_are_refused() {
    let f = Fixture::new();
    let session = f.exported();
    edit_session(&session, |s| {
        let reviews = s["review_comments"].as_array_mut().unwrap();
        reviews.push(human("h-1", "Luke", "note", "first"));
        reviews.push(human("h-2", "Luke", "note", "second"));
    });
    json(&f.import(&session, &["--execute"]));
    edit_session(&session, |s| {
        s["review_comments"]
            .as_array_mut()
            .unwrap()
            .push(human("h-3", "Luke", "note", "third"));
    });
    json(&f.import(&session, &["--execute"]));
    let good = fs::read_to_string(f.journal()).unwrap();
    assert_eq!(good.lines().count(), 3);
    let cases: [(&str, String, &str); 5] = [
        (
            "edited content",
            good.replacen("second", "SECOND", 1),
            "modified or reordered",
        ),
        (
            "reattributed actor",
            good.replacen("\"Luke\"", "\"Mallory\"", 1),
            "modified or reordered",
        ),
        (
            "dropped last line",
            good.split_inclusive('\n').take(2).collect(),
            "truncated or rewritten",
        ),
        (
            "partial last line",
            good[..good.len() - 10].to_owned(),
            "incomplete",
        ),
        (
            "reordered lines",
            {
                let l: Vec<&str> = good.lines().collect();
                format!("{}\n{}\n{}\n", l[1], l[0], l[2])
            },
            "modified or reordered",
        ),
    ];
    for (name, text, needle) in cases {
        fs::write(f.journal(), text).unwrap();
        let out = f.import(&session, &[]);
        assert_eq!(out.status.code(), Some(4), "{name}");
        assert!(stderr(&out).contains(needle), "{name}: {}", stderr(&out));
    }
    fs::remove_file(f.journal()).unwrap();
    refused(&f.import(&session, &[]), "truncated or rewritten");
    // Positive control: the untouched journal verifies and plans nothing.
    fs::write(f.journal(), &good).unwrap();
    assert_eq!(
        json(&f.import(&session, &["--execute"]))["events"],
        serde_json::json!([])
    );
}

#[test]
fn edits_of_council_notes_are_conflicts_and_human_edits_are_revisions() {
    let f = Fixture::new();
    let session = f.exported();
    let range_id = f.exported_id(&session, RANGE_KEY);
    let line_id = f.exported_id(&session, LINE_KEY);
    edit_session(&session, |s| {
        s["review_comments"].as_array_mut().unwrap().push(human(
            "h-1",
            "Luke",
            "suggestion",
            "Use a half-open range.",
        ));
    });
    json(&f.import(&session, &["--execute"]));
    let journaled = fs::read(f.journal()).unwrap();
    let added = f.events()[0]["hash"].clone();
    edit_session(&session, |s| {
        comment_mut(s, &range_id)["content"] = "edited by a human in the TUI".into();
        let c = comment_mut(s, "h-1");
        c["content"] = format!("Use a half-open range; see RFC. [ddf:{LINE_KEY}]").into();
        c["lifecycle_state"] = "submitted".into();
    });
    let out = f.import(&session, &["--execute"]);
    assert_eq!(
        out.status.code(),
        Some(7),
        "a conflict is raised, not absorbed"
    );
    let result: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(result["conflicts"], 1);
    let events = f.events();
    assert!(
        fs::read(f.journal()).unwrap().starts_with(&journaled),
        "prior events are never rewritten"
    );
    assert_eq!(
        kinds(&events),
        ["comment_added", "revised", "lifecycle", "revised"]
    );
    let conflict = &events[3];
    assert_eq!(conflict["comment_id"], range_id.as_str());
    assert_eq!(conflict["ddf_authored"], true);
    assert!(
        conflict["conflict"]
            .as_str()
            .unwrap()
            .contains("edited outside DDF")
    );
    assert_eq!(conflict["actor"]["basis"], "unrecorded");
    assert_eq!(conflict["content"], "edited by a human in the TUI");
    let revised = &events[1];
    assert_eq!(revised["comment_id"], "h-1");
    assert!(revised["conflict"].is_null());
    assert_eq!(revised["supersedes"], added);
    assert_eq!(revised["actor"]["name"], "Luke");
    assert_eq!(
        revised["links"],
        serde_json::json!([{"key": LINE_KEY, "tuicr_comment_id": line_id, "basis": "tag"}]),
        "a revision carries the links of its new content"
    );
    let lifecycle = &events[2];
    assert_eq!(lifecycle["previous_lifecycle_state"], "local_draft");
    assert_eq!(lifecycle["lifecycle_state"], "submitted");
    assert_eq!(lifecycle["supersedes"], revised["hash"]);
    // Re-running neither repeats the conflict nor treats the edited note as new.
    let again = f.import(&session, &["--execute"]);
    assert_eq!(again.status.code(), Some(0), "{}", stderr(&again));
    assert_eq!(f.events().len(), 4);
    // A lifecycle-only change of an untouched council note is a transition, not a revision.
    edit_session(&session, |s| {
        comment_mut(s, &line_id)["lifecycle_state"] = "pushed_draft".into();
    });
    let pushed = json(&f.import(&session, &["--execute"]));
    assert_eq!(kinds(pushed["events"].as_array().unwrap()), ["lifecycle"]);
    assert_eq!(pushed["events"][0]["ddf_authored"], true);
    assert_eq!(
        json(&f.import(&session, &["--execute"]))["events"],
        serde_json::json!([])
    );
    // A lifecycle-only change of a journaled human comment supersedes its latest event.
    edit_session(&session, |s| {
        comment_mut(s, "h-1")["lifecycle_state"] = "pushed_draft".into();
    });
    let moved = json(&f.import(&session, &["--execute"]));
    assert_eq!(kinds(moved["events"].as_array().unwrap()), ["lifecycle"]);
    let moved = &moved["events"][0];
    assert_eq!(moved["comment_id"], "h-1");
    assert_eq!(moved["previous_lifecycle_state"], "submitted");
    assert_eq!(moved["supersedes"], lifecycle["hash"]);
}

#[test]
fn deleted_comments_become_tombstones_and_can_be_restored() {
    let f = Fixture::new();
    let session = f.exported();
    let line_id = f.exported_id(&session, LINE_KEY);
    edit_session(&session, |s| {
        let reviews = s["review_comments"].as_array_mut().unwrap();
        reviews.push(human(
            "h-1",
            "Luke",
            "issue",
            &format!("Drop this. [ddf:{LINE_KEY}]"),
        ));
        // A later comment by someone else: h-1's tombstone must still name h-1's author.
        reviews.push(human("h-2", "Ana", "note", "Unrelated."));
    });
    json(&f.import(&session, &["--execute"]));
    let added = f.events()[0].clone();
    assert_eq!(added["comment_id"], "h-1");
    let original = fs::read(&session).unwrap();
    edit_session(&session, |s| {
        s["review_comments"]
            .as_array_mut()
            .unwrap()
            .retain(|c| c["id"] != "h-1");
        for list in s["files"]["x.rs"]["line_comments"]
            .as_object_mut()
            .unwrap()
            .values_mut()
        {
            list.as_array_mut()
                .unwrap()
                .retain(|c| c["id"] != line_id.as_str());
        }
    });
    let out = json(&f.import(&session, &["--execute"]));
    let events = out["events"].as_array().unwrap();
    assert_eq!(kinds(events), ["deleted", "deleted"]);
    let human_tomb = events.iter().find(|e| e["comment_id"] == "h-1").unwrap();
    assert_eq!(human_tomb["supersedes"], added["hash"]);
    assert_eq!(human_tomb["actor"]["name"], "Luke");
    let note_tomb = events
        .iter()
        .find(|e| e["comment_id"] == line_id.as_str())
        .unwrap();
    assert_eq!(note_tomb["ddf_authored"], true);
    assert!(note_tomb["content"].is_null());
    assert_eq!(
        json(&f.import(&session, &["--execute"]))["events"],
        serde_json::json!([])
    );
    fs::write(&session, original).unwrap();
    let restored = json(&f.import(&session, &["--execute"]));
    let back = restored["events"].as_array().unwrap();
    assert_eq!(kinds(back), ["restored", "restored"]);
    let human_back = back.iter().find(|e| e["comment_id"] == "h-1").unwrap();
    assert_eq!(human_back["supersedes"], human_tomb["hash"]);
    assert_eq!(human_back["actor"]["name"], "Luke");
    assert_eq!(
        human_back["links"],
        serde_json::json!([{"key": LINE_KEY, "tuicr_comment_id": line_id, "basis": "tag"}])
    );
    let note_back = back
        .iter()
        .find(|e| e["comment_id"] == line_id.as_str())
        .unwrap();
    assert_eq!(note_back["supersedes"], note_tomb["hash"]);
    assert_eq!(note_back["links"], serde_json::json!([]));
    assert_eq!(f.events().len(), 6);
}

#[test]
fn reviewed_hunks_alone_are_a_positive_mark_that_pins_the_content_hash() {
    let f = Fixture::new();
    let session = f.exported();
    edit_session(&session, |s| {
        s["files"]["x.rs"]["reviewed_hunks"] = serde_json::json!(["h1"]);
        s["files"]["x.rs"]["content_hash"] = serde_json::Value::Null;
    });
    refused(&f.import(&session, &["--execute"]), "no tuicr content_hash");
    edit_session(&session, |s| {
        s["files"]["x.rs"]["content_hash"] = 111.into();
    });
    let first = json(&f.import(&session, &["--execute"]));
    let mark = &first["events"][0];
    assert_eq!(mark["reviewed"], false);
    assert_eq!(mark["reviewed_hunks"], serde_json::json!(["h1"]));
    assert_eq!(mark["content_hash"], 111);
    // The hunk mark pinned 111: a whole-file mark under another tuicr diff hash is refused.
    edit_session(&session, |s| {
        s["files"]["x.rs"]["reviewed"] = true.into();
        s["files"]["x.rs"]["content_hash"] = 222.into();
    });
    refused(&f.import(&session, &["--execute"]), "stale review mark");
    // Clearing every mark is not positive, so it needs no content hash.
    edit_session(&session, |s| {
        s["files"]["x.rs"]["reviewed"] = false.into();
        s["files"]["x.rs"]["reviewed_hunks"] = serde_json::json!([]);
        s["files"]["x.rs"]["content_hash"] = serde_json::Value::Null;
    });
    let cleared = json(&f.import(&session, &["--execute"]));
    assert_eq!(
        kinds(cleared["events"].as_array().unwrap()),
        ["file_review"]
    );
    assert_eq!(cleared["events"][0]["reviewed"], false);
    assert_eq!(cleared["events"][0]["supersedes"], mark["hash"]);
}

#[test]
fn dispositions_need_the_file_on_both_sides_or_neither() {
    // gone.rs is committed, then deleted before the run: inventoried, absent from the snapshot.
    let f = Fixture::with_repository(|repository| {
        fs::write(repository.join("gone.rs"), "fn gone() {}\n").unwrap();
        git(repository, &["add", "gone.rs"]);
        git(repository, &["commit", "-qm", "gone"]);
        fs::remove_file(repository.join("gone.rs")).unwrap();
    });
    let deleted = f.session("deleted.json", |s| {
        s["files"]["gone.rs"] = serde_json::json!({
            "path": "gone.rs", "reviewed": false, "status": "deleted",
            "file_comments": [human("h-gone", "Luke", "note", "Good riddance.")],
            "line_comments": {}, "reviewed_hunks": [], "content_hash": 0
        });
    });
    // Absent on both sides: a disposition on the deleted file is on the run's revision.
    let both_absent = json(&f.import(&deleted, &[]));
    assert_eq!(
        kinds(both_absent["events"].as_array().unwrap()),
        ["comment_added"]
    );
    assert_eq!(
        both_absent["events"][0]["anchor"],
        serde_json::json!({"type": "file", "file": "gone.rs"})
    );
    // Recreated after the run: the session reviews bytes the run never sealed.
    fs::write(f.repository.join("gone.rs"), "fn gone() {}\n").unwrap();
    refused(&f.import(&deleted, &[]), "absent from the sealed snapshot");
    fs::remove_file(f.repository.join("gone.rs")).unwrap();
    // Sealed, then deleted from the working tree: the session cannot show the sealed bytes.
    let sealed = f.session("sealed.json", |s| {
        s["files"]["x.rs"]["file_comments"] =
            serde_json::json!([human("h-x", "Luke", "note", "Keep this.")]);
    });
    fs::remove_file(f.repository.join("x.rs")).unwrap();
    refused(&f.import(&sealed, &[]), "stale file");
    // Positive control: the sealed bytes restored.
    fs::write(f.repository.join("x.rs"), SOURCE).unwrap();
    assert_eq!(
        kinds(json(&f.import(&sealed, &[]))["events"].as_array().unwrap()),
        ["comment_added"]
    );
    assert!(!f.journal().exists(), "dry runs write nothing");
}

#[test]
fn export_ledger_records_must_belong_to_the_run_and_session_and_be_named_by_key() {
    let f = Fixture::new();
    let session = f.exported();
    let dir = f
        .run
        .join("exports/tuicr/00000000-0000-4000-8000-000000000001");
    let name = f
        .ledger_files()
        .into_iter()
        .find(|n| Path::new(n).extension().is_some_and(|e| e == "json"))
        .unwrap();
    let record = dir.join(&name);
    let original = fs::read(&record).unwrap();
    // Each identity field is checked on its own.
    for field in ["run_id", "session_id"] {
        let mut value: serde_json::Value = serde_json::from_slice(&original).unwrap();
        value[field] = "f".repeat(64).into();
        fs::write(&record, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
        refused(
            &f.import(&session, &[]),
            "does not belong to this run and session",
        );
    }
    fs::write(&record, &original).unwrap();
    // A faithful record under a name that is not the hash of its key.
    let misnamed = dir.join(format!("{}.json", "0".repeat(64)));
    fs::copy(&record, &misnamed).unwrap();
    refused(
        &f.import(&session, &[]),
        "does not belong to this run and session",
    );
    fs::remove_file(&misnamed).unwrap();
    assert!(json(&f.import(&session, &[]))["events"].is_array());
}

#[test]
fn sealed_inventory_is_read_up_to_its_cap_and_no_further() {
    const CAP: usize = 8 * 1024 * 1024;
    let f = Fixture::new();
    let session = f.session("session.json", |_| {});
    let inventory = f.run.join("source-inventory.json");
    // Trailing JSON whitespace changes neither the inventory nor its fingerprint.
    let mut padded = fs::read(&inventory).unwrap();
    padded.resize(CAP, b' ');
    fs::write(&inventory, &padded).unwrap();
    assert!(json(&f.import(&session, &[]))["events"].is_array());
    padded.push(b' ');
    fs::write(&inventory, &padded).unwrap();
    refused(&f.import(&session, &[]), "acquisition budget");
}

#[test]
fn interrupted_exports_held_locks_and_unsafe_sessions_refuse_import() {
    let f = Fixture::new();
    let session = f.exported();
    let ledger = f
        .run
        .join("exports/tuicr/00000000-0000-4000-8000-000000000001");
    let marker = ledger.join(format!("{}.pending", "a".repeat(64)));
    fs::write(&marker, "key").unwrap();
    refused(&f.import(&session, &[]), "interrupted export");
    fs::remove_file(&marker).unwrap();
    let lock = f.run.join("decisions.lock");
    fs::write(&lock, "1").unwrap();
    refused(&f.import(&session, &["--execute"]), "locked");
    assert!(
        json(&f.import(&session, &[]))["events"].is_array(),
        "a dry run takes no lock"
    );
    fs::remove_file(&lock).unwrap();
    let escaping = f.root.join("escaping.json");
    fs::copy(&session, &escaping).unwrap();
    edit_session(&escaping, |s| {
        s["files"]["../x.rs"] = s["files"]["x.rs"].clone();
    });
    refused(&f.import(&escaping, &[]), "unsafe session");
    refused(&f.import(&session, &["--execute", "--execute"]), "usage");
    assert!(!f.journal().exists());
}

#[test]
fn real_tuicr_human_comment_is_imported_as_a_disposition() {
    if std::env::var_os("DDF_TUICR_IT").is_none() {
        let _ = writeln!(
            std::io::stderr(),
            "UNMEASURED: real tuicr import skipped; set DDF_TUICR_IT=1 with tuicr on PATH"
        );
        return;
    }
    let it = Isolated::new();
    let stored = it.stored_session();
    json(&it.export(&stored, &["--execute"]));
    let added = it.tuicr(
        &[
            "review",
            "add",
            "--session",
            stored.to_str().unwrap(),
            "--type",
            "issue",
            "--target-file",
            "x.rs",
            "--line",
            "3",
            "--username",
            "Human Reviewer",
            "Off by one here.",
        ],
        "",
    );
    println!("tuicr added {} at {}", added["id"], added["location"]);
    let import = |flags: &[&str]| {
        let mut c = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"));
        c.args(["council", "import-tuicr", it.f.run.to_str().unwrap()])
            .arg("--session")
            .arg(&stored)
            .args(flags);
        it.isolate(&mut c).output().unwrap()
    };
    let out = json(&import(&["--execute"]));
    let events = out["events"].as_array().unwrap();
    for e in events {
        println!(
            "journaled {} {} by {}",
            e["kind"], e["comment_id"], e["actor"]["name"]
        );
    }
    // The fixture note that seeded the session is a human comment too.
    let disposition = events
        .iter()
        .find(|e| e["comment_id"] == added["id"])
        .expect("the tuicr comment is journaled");
    assert_eq!(disposition["kind"], "disposition");
    assert_eq!(disposition["actor"]["name"], "Human Reviewer");
    assert_eq!(disposition["comment_type"], "issue");
    assert_eq!(disposition["links"][0]["key"], RANGE_KEY);
    assert_eq!(disposition["links"][0]["basis"], "anchor");
    assert_eq!(
        disposition["revision"]["base_commit"],
        git(&it.f.repository, &["rev-parse", "HEAD"]).as_str()
    );
    assert!(events.iter().all(|e| e["ddf_authored"] == false));
    assert_eq!(
        json(&import(&["--execute"]))["events"],
        serde_json::json!([])
    );
    println!("rerun: no-op");
    fs::write(it.f.repository.join("x.rs"), "fn changed() {}\n").unwrap();
    it.tuicr(
        &[
            "review",
            "add",
            "--session",
            stored.to_str().unwrap(),
            "--target-file",
            "x.rs",
            "--line",
            "1",
            "--username",
            "Human Reviewer",
            "Later.",
        ],
        "",
    );
    let stale = import(&["--execute"]);
    refused(&stale, "stale file");
    println!("stale disposition: {}", stderr(&stale).trim());
}
