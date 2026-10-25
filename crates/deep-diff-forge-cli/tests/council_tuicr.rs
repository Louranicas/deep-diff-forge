// Review council: DDF→tuicr export through the actual CLI with fake providers, a fake tuicr and one opt-in real-tuicr round trip.
// Design/status: docs/REVIEW_COUNCIL.md#tuicr-export
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Comparisons%2FArchitecture%20Lessons%20and%20Interoperability
//! Admission, identity, anchoring, idempotency and refusal controls for `council export-tuicr`.
//! The real-tuicr test runs only with `DDF_TUICR_IT=1`, against an isolated XDG data/config store.
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
