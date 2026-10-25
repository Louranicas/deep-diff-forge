// Review council: Actual CLI process lifecycle with fake providers and tools, plus timeout/owned-child cleanup and zero-work refusals.
// Design/status: docs/REVIEW_COUNCIL.md#cli-controls
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fcouncil--crates--deep-diff-forge-cli--tests--council
//! Actual CLI/driver lifecycle against deterministic fake providers; no model/API calls.
use deep_diff_forge_review::{Profile, Request};
use std::os::unix::fs::PermissionsExt;
use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    root: PathBuf,
    repository: PathBuf,
    run: PathBuf,
    request: PathBuf,
    bin: PathBuf,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "ddf-council-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let repository = root.join("repo");
        fs::create_dir(&repository).unwrap();
        fs::write(repository.join("x.rs"), "fn b() {}\n").unwrap();
        for args in [
            vec!["init", "-q"],
            vec!["add", "x.rs"],
            vec![
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "-qm",
                "fixture",
            ],
        ] {
            assert!(
                Command::new("git")
                    .args(args)
                    .current_dir(&repository)
                    .status()
                    .unwrap()
                    .success()
            );
        }
        let analysis = root.join("analysis.json");
        fs::write(
            &analysis,
            include_str!("../../../fixtures/council/analysis.json"),
        )
        .unwrap();
        let decision = root.join("decision.md");
        fs::write(
            &decision,
            "Assess the captured API boundary. Keep dissent and request falsification.\n",
        )
        .unwrap();
        let mut spec = deep_diff_forge_review::cli::example();
        spec.profile = Profile::Roster;
        spec.reviewers.truncate(2);
        spec.repository = repository.display().to_string();
        spec.analysis_file = analysis.display().to_string();
        spec.decision_file = decision.display().to_string();
        spec.timeout_seconds = 2;
        let request = root.join("request.json");
        fs::write(&request, serde_json::to_vec(&spec).unwrap()).unwrap();
        let bin = root.join("fake-bin");
        fs::create_dir(&bin).unwrap();
        for name in ["codex", "claude"] {
            let p = bin.join(name);
            fs::write(&p, FAKE).unwrap();
            fs::set_permissions(&p, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let run = root.join("run");
        Self {
            root,
            repository,
            run,
            request,
            bin,
        }
    }
    fn call(&self, args: &[&str], mode: &str) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"));
        command
            .arg("council")
            .args(args)
            .env("DDF_FAKE_MODE", mode)
            .env("CARGO_TARGET_DIR", self.root.join("normal-target"))
            .env(
                "PATH",
                format!("{}:{}", self.bin.display(), std::env::var("PATH").unwrap()),
            );
        command.output().unwrap()
    }
    fn prepare(&self) {
        let result = self.call(
            &[
                "prepare",
                self.request.to_str().unwrap(),
                self.run.to_str().unwrap(),
            ],
            "normal",
        );
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    fn review(&self, id: &str, round: &str, mode: &str) -> Output {
        self.call(
            &["review", self.run.to_str().unwrap(), id, round, "--execute"],
            mode,
        )
    }
    fn status(&self) -> Output {
        self.call(&["status", self.run.to_str().unwrap()], "normal")
    }
}
const FAKE: &str = r"#!/usr/bin/python3
import sys,json,os,time
prompt=sys.stdin.read()
parts=prompt.splitlines()[0].split()
reviewer=parts[3];run=parts[6].rstrip('.');phase=parts[8].rstrip('.')
mode=os.environ.get('DDF_FAKE_MODE','normal')
if mode=='timeout':
    import subprocess
    child=subprocess.Popen(['/bin/sleep','30'])
    print(json.dumps({'owned_child':child.pid}),flush=True)
    time.sleep(30)
if mode=='output_limit':
    sys.stdout.write('x'*(10*1024*1024));sys.stdout.flush();time.sleep(30)
if mode=='malformed': print('not JSON');sys.exit(0)
report={'run_id':run,'reviewer_id':reviewer,'round':phase,'verdict':'no_blockers','summary':'REVIEW_SUMMARY::'+reviewer+'::'+phase,'findings':[],'responses':[]}
if mode=='stale': report['run_id']='unrelated'
if phase=='independent' and 'REVIEW_SUMMARY::' in prompt: sys.exit(9)
if os.path.basename(sys.argv[0])=='codex':
    path=sys.argv[sys.argv.index('--output-last-message')+1]
    with open(path,'w') as file:json.dump(report,file)
    print(json.dumps(report))
else:
    print(json.dumps({'is_error':mode=='denied','structured_output':None if mode=='denied' else report}))
";
#[test]
fn two_providers_are_blind_then_cross_review_and_quality_remains_required() {
    let f = Fixture::new();
    f.prepare();
    assert_eq!(
        f.review("correctness-codex", "challenge", "normal")
            .status
            .code(),
        Some(4)
    );
    for round in ["independent", "challenge"] {
        for id in ["correctness-codex", "security-claude"] {
            let r = f.review(id, round, "normal");
            assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
        }
    }
    let status = f.status();
    assert_eq!(status.status.code(), Some(7));
    let result: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(result["reports_received"], 4);
    assert_eq!(result["state"], "hold");
    let prompt = fs::read_to_string(
        f.run
            .join("jobs/review-correctness-codex-challenge/stdin.txt"),
    )
    .unwrap();
    assert!(prompt.contains("REVIEW_SUMMARY::security-claude::independent"));
    assert_eq!(
        f.review("correctness-codex", "independent", "normal")
            .status
            .code(),
        Some(4)
    );
    let argv: serde_json::Value = serde_json::from_slice(
        &fs::read(
            f.run
                .join("jobs/review-correctness-codex-independent/execution.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let argv = argv["argv"].as_array().unwrap();
    for expected in ["read-only", "hooks", "model_reasoning_effort=\"high\""] {
        assert!(argv.iter().any(|v| v == expected), "missing {expected}");
    }
    assert_eq!(
        fs::metadata(&f.run).unwrap().permissions().mode() & 0o777,
        0o700
    );
}
#[test]
fn exit_zero_with_malformed_stale_or_error_output_never_qualifies() {
    for (reviewer, mode) in [
        ("correctness-codex", "malformed"),
        ("correctness-codex", "stale"),
        ("security-claude", "denied"),
    ] {
        let f = Fixture::new();
        f.prepare();
        let r = f.review(reviewer, "independent", mode);
        assert_eq!(r.status.code(), Some(7));
        let receipt: serde_json::Value = serde_json::from_slice(&r.stdout).unwrap();
        assert_eq!(receipt["outcome"], "failed");
        assert_eq!(f.status().status.code(), Some(7));
    }
}
#[test]
fn changed_candidate_manifest_or_accepted_report_cannot_reuse_results() {
    let f = Fixture::new();
    f.prepare();
    assert!(
        f.review("correctness-codex", "independent", "normal")
            .status
            .success()
    );
    let path = f.run.join("reports/correctness-codex-independent.json");
    let mut report: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    report["summary"] = "tampered".into();
    fs::write(&path, serde_json::to_vec(&report).unwrap()).unwrap();
    assert_eq!(f.status().status.code(), Some(4));
    let f = Fixture::new();
    f.prepare();
    fs::write(f.repository.join("x.rs"), "fn changed() {}\n").unwrap();
    assert_eq!(f.status().status.code(), Some(7));
    assert_eq!(
        f.review("correctness-codex", "independent", "normal")
            .status
            .code(),
        Some(4)
    );
    let f = Fixture::new();
    fs::write(f.repository.join("x.rs"), "fn unrelated() {}\n").unwrap();
    assert_eq!(
        f.call(
            &[
                "prepare",
                f.request.to_str().unwrap(),
                f.run.to_str().unwrap()
            ],
            "normal"
        )
        .status
        .code(),
        Some(4)
    );
}
#[test]
fn timeouts_are_recorded_and_unexecuted_requests_are_refused() {
    let f = Fixture::new();
    f.prepare();
    assert_eq!(
        f.call(
            &[
                "review",
                f.run.to_str().unwrap(),
                "correctness-codex",
                "independent"
            ],
            "normal"
        )
        .status
        .code(),
        Some(4)
    );
    let r = f.review("correctness-codex", "independent", "timeout");
    assert_eq!(r.status.code(), Some(7));
    let receipt: serde_json::Value = serde_json::from_slice(&r.stdout).unwrap();
    assert_eq!(receipt["outcome"], "timed_out");
    assert!(!f.run.join("controller.lock").exists());
    #[cfg(target_os = "linux")]
    {
        let line = fs::read_to_string(
            f.run
                .join("jobs/review-correctness-codex-independent/stdout.log"),
        )
        .unwrap();
        let pid = serde_json::from_str::<serde_json::Value>(&line).unwrap()["owned_child"]
            .as_u64()
            .unwrap();
        if let Ok(stat) = fs::read_to_string(format!("/proc/{pid}/stat")) {
            assert_eq!(stat.split_whitespace().nth(2), Some("Z"));
        }
    }
}
#[test]
fn subtree_and_symlink_source_capture_are_refused() {
    let f = Fixture::new();
    let mut request: Request = serde_json::from_slice(&fs::read(&f.request).unwrap()).unwrap();
    let sub = f.repository.join("sub");
    fs::create_dir(&sub).unwrap();
    request.repository = sub.display().to_string();
    fs::write(&f.request, serde_json::to_vec(&request).unwrap()).unwrap();
    assert_eq!(
        f.call(
            &[
                "prepare",
                f.request.to_str().unwrap(),
                f.run.to_str().unwrap()
            ],
            "normal"
        )
        .status
        .code(),
        Some(4)
    );
    let f = Fixture::new();
    fs::remove_file(f.repository.join("x.rs")).unwrap();
    std::os::unix::fs::symlink(f.root.join("decision.md"), f.repository.join("x.rs")).unwrap();
    assert_eq!(
        f.call(
            &[
                "prepare",
                f.request.to_str().unwrap(),
                f.run.to_str().unwrap()
            ],
            "normal"
        )
        .status
        .code(),
        Some(4)
    );
}

#[test]
fn oversized_transcripts_fail_with_explicit_incomplete_diagnostics() {
    let f = Fixture::new();
    f.prepare();
    let output = f.review("correctness-codex", "independent", "output_limit");
    assert_eq!(output.status.code(), Some(7));
    let receipt: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(receipt["outcome"], "output_limit");
    let job = f.run.join("jobs/review-correctness-codex-independent");
    let diagnostic: serde_json::Value =
        serde_json::from_slice(&fs::read(job.join("transcript-limit.json")).unwrap()).unwrap();
    assert_eq!(diagnostic["transcript_complete"], false);
    assert!(fs::metadata(job.join("stdout.log")).unwrap().len() <= 8 * 1024 * 1024);
}

const FAKE_GATE: &str = r"#!/usr/bin/python3
import json,os,pathlib,sys
mode=os.environ.get('DDF_FAKE_MODE','empty')
if 'mutants' in sys.argv:
    assert os.environ.get('CARGO_TARGET_DIR','').endswith('/jobs/gate-mutation/mutation-target')
    parent=pathlib.Path(sys.argv[sys.argv.index('--output')+1])/'mutants.out';parent.mkdir(parents=True)
    data={'total_mutants':1,'caught':1,'unviable':0,'missed':0,'timeout':0,'outcomes':[{'scenario':'Baseline','summary':'Success'},{'scenario':{'Mutant':{}},'summary':'CaughtMutant'}]}
    if mode=='empty':data.update(total_mutants=0,caught=0,outcomes=[])
    if mode=='caption_only':data.pop('outcomes')
    if mode=='survivor':data['missed']=1
    (parent/'outcomes.json').write_text(json.dumps(data))
elif 'fuzz' in sys.argv:
    print('Done '+('0' if mode=='empty' else '100')+' runs in 1 second(s)',file=sys.stderr)
else:
    print('test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out')
";
#[test]
fn empty_mutation_fuzz_or_test_runs_are_inconclusive_even_at_exit_zero() {
    for (gate, mode) in [
        ("mutation", "empty"),
        ("mutation", "caption_only"),
        ("mutation", "survivor"),
        ("fuzz", "empty"),
        ("tests", "empty"),
    ] {
        let f = Fixture::new();
        f.prepare();
        let cargo = f.bin.join("cargo");
        fs::write(&cargo, FAKE_GATE).unwrap();
        fs::set_permissions(&cargo, fs::Permissions::from_mode(0o700)).unwrap();
        let result = f.call(&["gate", f.run.to_str().unwrap(), gate, "--execute"], mode);
        assert_eq!(
            result.status.code(),
            Some(7),
            "{gate}/{mode}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let receipts: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(receipts[0]["outcome"], "inconclusive");
        assert_eq!(f.status().status.code(), Some(7));
    }
}

#[test]
fn mutation_jobs_own_a_build_cache_and_cannot_contaminate_normal_tests() {
    let f = Fixture::new();
    f.prepare();
    let cargo = f.bin.join("cargo");
    fs::write(&cargo, FAKE_GATE).unwrap();
    fs::set_permissions(&cargo, fs::Permissions::from_mode(0o700)).unwrap();
    let result = f.call(
        &["gate", f.run.to_str().unwrap(), "mutation", "--execute"],
        "valid",
    );
    assert_eq!(
        result.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let env: serde_json::Value = serde_json::from_slice(
        &fs::read(f.run.join("jobs/gate-mutation/environment.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(env["normal_target_inherited"], false);
    assert_eq!(
        env["mutation_target_dir"],
        f.run
            .join("jobs/gate-mutation/mutation-target")
            .display()
            .to_string()
    );
}

#[test]
fn relative_run_paths_resolve_before_provider_working_directory_changes() {
    let f = Fixture::new();
    f.prepare();
    let result = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .args([
            "council",
            "review",
            "run",
            "correctness-codex",
            "independent",
            "--execute",
        ])
        .current_dir(&f.root)
        .env(
            "PATH",
            format!("{}:{}", f.bin.display(), std::env::var("PATH").unwrap()),
        )
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}
