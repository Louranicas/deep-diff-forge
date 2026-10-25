// Review council: Actual CLI process lifecycle with fake providers and tools, plus timeout/owned-child cleanup and zero-work refusals.
// Design/status: docs/REVIEW_COUNCIL.md#cli-controls
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fcouncil--crates--deep-diff-forge-cli--tests--council
//! Actual CLI/driver lifecycle against deterministic fake providers; no model/API calls.
use deep_diff_forge_review::{Profile, Request};
use std::os::unix::fs::PermissionsExt;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
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
        for (name, script) in [("codex", FAKE), ("claude", FAKE), ("podman", FAKE_RUNTIME)] {
            let p = bin.join(name);
            fs::write(&p, script).unwrap();
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
    /// The controller process. Providers still see `DDF_FAKE_MODE`; sandboxed gates never do.
    fn command(&self, args: &[&str], mode: &str) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"));
        command
            .arg("council")
            .args(args)
            .env("DDF_FAKE_MODE", mode)
            .env("CARGO_TARGET_DIR", self.root.join("normal-target"))
            .env("DDF_COUNCIL_PODMAN", self.bin.join("podman"))
            .env("DDF_COUNCIL_GATE_IMAGE", "fake-gate")
            .env(
                "PATH",
                format!("{}:{}", self.bin.display(), std::env::var("PATH").unwrap()),
            );
        command
    }
    fn call(&self, args: &[&str], mode: &str) -> Output {
        self.command(args, mode).output().unwrap()
    }
    /// Install the fake `cargo` with its mode baked in: the sandbox has no ambient environment.
    fn install_gate(&self, mode: &str) {
        let cargo = self.bin.join("cargo");
        fs::write(&cargo, FAKE_GATE.replace("@MODE@", mode)).unwrap();
        fs::set_permissions(&cargo, fs::Permissions::from_mode(0o700)).unwrap();
    }
    fn gate(&self, gate: &str) -> Output {
        self.gate_with(gate, "normal")
    }
    /// A gate run under a planted runtime fault; a controller that hangs fails the test, never blocks it.
    fn gate_with(&self, gate: &str, runtime: &str) -> Output {
        let mut child = self
            .command(
                &["gate", self.run.to_str().unwrap(), gate, "--execute"],
                "normal",
            )
            .env("DDF_FAKE_RUNTIME", runtime)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let start = Instant::now();
        while child.try_wait().unwrap().is_none() {
            if start.elapsed() > Duration::from_secs(150) {
                let _ = child.kill();
                panic!("controller hung on {gate} under {runtime}");
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        child.wait_with_output().unwrap()
    }
    /// Every `rm` / `container exists` the fake runtime answered.
    fn control_log(&self) -> Vec<Vec<String>> {
        fs::read_to_string(self.bin.join("runtime-control.log"))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
    /// Commit an extra file into the fixture repository, outside the analysed candidate.
    fn commit(&self, path: &str, contents: &str) {
        let file = self.repository.join(path);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, contents).unwrap();
        for args in [
            vec!["add", path],
            vec![
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "-qm",
                "extra file",
            ],
        ] {
            assert!(
                Command::new("git")
                    .args(args)
                    .current_dir(&self.repository)
                    .status()
                    .unwrap()
                    .success()
            );
        }
    }
    /// Every runtime invocation the fake recorded for one job's `/work` mount.
    fn runtime_argv(&self, job: &str) -> Vec<Vec<String>> {
        fs::read_to_string(
            self.run
                .join("jobs")
                .join(job)
                .join("work/runtime-argv.json"),
        )
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
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

/// Fake rootless runtime: answers `image inspect` with [`FAKE_IMAGE_ID`] and `rm`/`container exists`
/// (logging both to `fake-bin/runtime-control.log`), and for `run` maps `/work`, `/src`, `/cargo-home`
/// and `/vendor` to their `--volume` host paths, then runs the gate argv with ONLY the `--env` values
/// plus a PATH to the fake tools (no inherited environment), propagating the exit code. Each `run` argv
/// is appended as one JSON line to `<work>/runtime-argv.json`. `DDF_FAKE_RUNTIME` (inherited from the
/// controller, never passed into the gate) plants runtime faults: `hang_inspect`, `bad_id`,
/// `orphan_inspect` (answers, then leaves a descendant holding the output pipe), `linger`
/// (the container never goes away) and `hang_exists`; `mode:job` limits a removal fault to containers
/// whose name contains `job`. A name listed in `fake-bin/occupied-names` is
/// refused as in use, like a container another invocation still holds.
const FAKE_RUNTIME: &str = r"#!/usr/bin/python3
import json,os,re,shutil,subprocess,sys,time
args=sys.argv[1:]
here=os.path.dirname(os.path.abspath(__file__))
mode,_,only=os.environ.get('DDF_FAKE_RUNTIME','normal').partition(':')
image_id='0123456789abcdef'*4
def control():
    with open(os.path.join(here,'runtime-control.log'),'a') as log:log.write(json.dumps(args)+'\n')
if args[:2]==['image','inspect']:
    if mode=='hang_inspect':time.sleep(120)
    if args[-1]!='fake-gate':sys.exit(125)
    if mode=='orphan_inspect' and os.fork()==0:
        # A descendant that keeps the output pipe open after the runtime itself exits.
        with open(os.path.join(here,'orphan.pid'),'w') as out:out.write(str(os.getpid()))
        time.sleep(120);os._exit(0)
    print('not-an-image-id' if mode=='bad_id' else 'sha256:'+image_id);sys.exit(0)
if args[:1]==['rm']:control();sys.exit(0)
if args[:2]==['container','exists']:
    control()
    if only not in args[-1]:sys.exit(1)
    if mode=='hang_exists':time.sleep(120)
    sys.exit(0 if mode=='linger' else 1)
assert args[0]=='run',args
mounts={};env={};workdir=None;name=None;i=1
while args[i].startswith('--'):
    flag,_,value=args[i].partition('=')
    if flag=='--volume':
        host,target,_=value.split(':');mounts[target]=host
    elif flag=='--env':
        key,_,val=value.partition('=');env[key]=val
    elif flag=='--workdir':workdir=value
    elif flag=='--name':name=value
    i+=1
image=args[i];inner=args[i+1:]
assert image==image_id,image
occupied=os.path.join(here,'occupied-names')
if os.path.exists(occupied) and name in open(occupied).read().split():
    print('container name in use: '+name,file=sys.stderr);sys.exit(125)
with open(os.path.join(mounts['/work'],'runtime-argv.json'),'a') as log:log.write(json.dumps(sys.argv)+'\n')
def host(text):
    return re.sub(r'(^|=)(/work|/vendor|/src|/cargo-home)(?=/|$)',lambda m:m.group(1)+mounts[m.group(2)],text)
env={key:host(val) for key,val in env.items()}
env['PATH']=here+':/usr/bin:/bin'
exe=shutil.which(inner[0],path=env['PATH'])
sys.exit(subprocess.run([exe]+[host(a) for a in inner[1:]],cwd=host(workdir),env=env).returncode)
";
/// The image ID the fake runtime resolves `fake-gate` to; gates must launch this, never the tag.
const FAKE_IMAGE_ID: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
/// Fake `cargo` inside the fake sandbox. Modes plant what a hostile candidate (or its tools) could
/// leave in the job's writable `/work`: `vendor_symlink`, `vendor_root_symlink`, `vendor_fifo` and
/// `vendor_dirs` (more empty directories than the seal's entry budget) in
/// the vendoring output; `fifo` and `symlinked_dir` at the mutation witness.
const FAKE_GATE: &str = r"#!/usr/bin/python3
import json,os,pathlib,sys
mode='@MODE@'
if sys.argv[1:2]==['vendor']:
    out=pathlib.Path(sys.argv[-1])
    if mode=='vendor_root_symlink':out.symlink_to('/etc');sys.exit(0)
    crate=out/'crate-1.0.0';crate.mkdir(parents=True);(crate/'lib.rs').write_text('pub fn f() {}\n')
    if mode=='vendor_symlink':(crate/'escape').symlink_to('/etc')
    if mode=='vendor_fifo':os.mkfifo(crate/'pipe')
    if mode=='vendor_dirs':
        for i in range(100_001):(crate/str(i)).mkdir()
    sys.exit(0)
if 'mutants' in sys.argv:
    assert 'CARGO_TARGET_DIR' not in os.environ,'mutation must not share a cargo target dir'
    output=pathlib.Path(sys.argv[sys.argv.index('--output')+1]);parent=output/'mutants.out'
    if mode=='symlinked_dir':
        real=output/'elsewhere';real.mkdir(parents=True);parent.symlink_to(real)
    else:parent.mkdir(parents=True)
    data={'total_mutants':1,'caught':1,'unviable':0,'missed':0,'timeout':0,'outcomes':[{'scenario':'Baseline','summary':'Success'},{'scenario':{'Mutant':{}},'summary':'CaughtMutant'}]}
    if mode=='empty':data.update(total_mutants=0,caught=0,outcomes=[])
    if mode=='caption_only':data.pop('outcomes')
    if mode=='survivor':data['missed']=1
    if mode=='fifo':os.mkfifo(parent/'outcomes.json')
    else:(parent/'outcomes.json').write_text(json.dumps(data))
elif 'fuzz' in sys.argv:
    print('Done '+('0' if mode=='empty' else '100')+' runs in 1 second(s)',file=sys.stderr)
else:
    seen={'cwd':os.getcwd(),'x.rs':pathlib.Path('x.rs').read_text(),'env':dict(os.environ)}
    print('OBSERVED '+json.dumps(seen))
    for passed in ([18446744073709551615]*2 if mode=='overflow' else [0 if mode=='empty' else 1]):
        print('test result: ok. '+str(passed)+' passed; 0 failed; 0 ignored; 0 measured; 0 filtered out')
";
#[test]
fn empty_mutation_fuzz_or_test_runs_are_inconclusive_even_at_exit_zero() {
    for (gate, mode) in [
        ("mutation", "empty"),
        ("mutation", "caption_only"),
        ("mutation", "survivor"),
        ("fuzz", "empty"),
        ("tests", "empty"),
        // Two maximal candidate captions: an overflowing sum is unmeasured, not a panic or a wrap.
        ("tests", "overflow"),
    ] {
        let f = Fixture::new();
        f.prepare();
        f.install_gate(mode);
        let result = f.gate(gate);
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
fn a_mutation_base_scopes_mutation_to_a_sealed_diff_and_a_changed_diff_is_refused() {
    for tamper in [false, true] {
        let f = Fixture::new();
        let head = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&f.repository)
            .output()
            .unwrap();
        let head = String::from_utf8(head.stdout).unwrap().trim().to_owned();
        let mut spec: serde_json::Value =
            serde_json::from_slice(&fs::read(&f.request).unwrap()).unwrap();
        spec["mutation_base"] = head.into();
        fs::write(&f.request, serde_json::to_vec(&spec).unwrap()).unwrap();
        f.commit("y.rs", "fn changed() {}\n");
        f.prepare();
        f.install_gate("valid");
        let sealed = fs::read_to_string(f.run.join("mutation.diff")).unwrap();
        assert!(
            sealed.contains("+++ b/y.rs") && sealed.contains("+fn changed() {}"),
            "{sealed}"
        );
        if tamper {
            fs::write(f.run.join("mutation.diff"), "").unwrap();
            let result = f.gate("mutation");
            assert_eq!(result.status.code(), Some(4));
            assert!(
                String::from_utf8_lossy(&result.stderr)
                    .contains("sealed mutation diff does not match the manifest"),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            assert_eq!(fs::read_dir(f.run.join("jobs")).unwrap().count(), 0);
            continue;
        }
        let result = f.gate("mutation");
        assert_eq!(
            result.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let argv = f.runtime_argv("gate-mutation").concat();
        assert!(
            argv.windows(2)
                .any(|w| w == ["--in-diff", "/work/mutation.diff"])
        );
        assert_eq!(
            fs::read_to_string(f.run.join("jobs/gate-mutation/work/mutation.diff")).unwrap(),
            sealed
        );
    }
}

#[test]
fn mutation_jobs_own_a_build_cache_and_cannot_contaminate_normal_tests() {
    let f = Fixture::new();
    f.prepare();
    f.install_gate("valid");
    let result = f.gate("mutation");
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
    assert_eq!(env["cargo_target_dir"], serde_json::Value::Null);
    assert_eq!(env["network"], "none");
    assert_eq!(env["host_home_mounted"], false);
    assert!(
        !env["environment"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v.as_str().unwrap().starts_with("CARGO_TARGET_DIR=")),
        "{env}"
    );
    // The caller's own target dir never reaches the sandbox, and vendoring ran first.
    let normal = f.root.join("normal-target").display().to_string();
    let argv = f.runtime_argv("gate-mutation").concat();
    assert!(argv.iter().any(|a| a == "--network=none"));
    // Without a mutation base the whole package is mutated, each mutant against the workspace suite.
    assert!(!argv.iter().any(|a| a == "--in-diff"));
    assert!(argv.windows(2).any(|w| w == ["--timeout", "300"]));
    assert!(
        argv.iter()
            .all(|a| !a.starts_with("--env=CARGO_TARGET_DIR") && !a.contains(&normal))
    );
    // Gates mount the controller's sealed copy of the vendoring output, never the job's own /work.
    let sealed = fs::canonicalize(f.run.join("vendor")).unwrap();
    assert!(sealed.join("crate-1.0.0/lib.rs").is_file());
    assert!(
        argv.iter()
            .any(|a| *a == format!("--volume={}:/vendor:ro,z", sealed.display())),
        "{argv:?}"
    );
    assert!(argv.iter().all(|a| !a.contains("sandbox-vendor")));
    let seal: serde_json::Value = serde_json::from_slice(
        &fs::read(f.run.join("jobs/sandbox-vendor/vendor-seal.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(env["vendor_sha256"], seal["sha256"]);
    assert_eq!(seal["files"], 1);
    assert!(
        f.run
            .join("jobs/gate-mutation/work/mutation-results/mutants.out/outcomes.json")
            .is_file()
    );
    let qualification: serde_json::Value = serde_json::from_slice(
        &fs::read(f.run.join("jobs/gate-mutation/qualification.json")).unwrap(),
    )
    .unwrap();
    assert!(qualification["witness_sha256"].is_string());
}

#[test]
fn qualification_cannot_restore_pass_over_a_refusal_marker() {
    let f = Fixture::new();
    f.prepare();
    f.install_gate("empty");
    assert_eq!(f.gate("tests").status.code(), Some(7));
    let job = f.run.join("jobs/gate-tests");
    assert!(job.join("qualification-error.txt").is_file());
    let gate_reason = |output: &Output| {
        let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        result["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r.as_str().unwrap().starts_with("gate-tests:"))
    };
    let before = f.status();
    assert_eq!(before.status.code(), Some(7));
    assert!(gate_reason(&before));
    // A same-uid forger rewrites the controller's qualification; the exit-0 receipt is untouched.
    fs::write(
        job.join("qualification.json"),
        br#"{"outcome":"passed","report_sha256":null,"witness_sha256":null}"#,
    )
    .unwrap();
    let after = f.status();
    assert_eq!(after.status.code(), Some(7));
    assert!(
        gate_reason(&after),
        "{}",
        String::from_utf8_lossy(&after.stdout)
    );
}

#[test]
fn ignored_candidate_files_are_refused_at_prepare() {
    let f = Fixture::new();
    assert!(
        Command::new("git")
            .args(["rm", "-q", "--cached", "x.rs"])
            .current_dir(&f.repository)
            .status()
            .unwrap()
            .success()
    );
    fs::write(f.repository.join(".gitignore"), "x.rs\n").unwrap();
    let result = f.call(
        &[
            "prepare",
            f.request.to_str().unwrap(),
            f.run.to_str().unwrap(),
        ],
        "normal",
    );
    assert_eq!(result.status.code(), Some(4));
    assert!(
        String::from_utf8_lossy(&result.stderr)
            .contains("candidate file is outside the fingerprinted source inventory"),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(!f.run.exists());
}

#[test]
fn gates_run_on_the_sealed_snapshot_and_refuse_a_tampered_one() {
    let f = Fixture::new();
    f.prepare();
    f.install_gate("valid");
    fs::write(f.run.join("source/x.rs"), "fn forged() {}\n").unwrap();
    let result = f.gate("tests");
    assert_eq!(result.status.code(), Some(4));
    assert!(
        String::from_utf8_lossy(&result.stderr).contains("gate snapshot does not match"),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    // Refused at admission: not even the vendoring job got a slot.
    assert_eq!(fs::read_dir(f.run.join("jobs")).unwrap().count(), 0);

    let f = Fixture::new();
    f.prepare();
    f.install_gate("valid");
    let result = f.gate("tests");
    assert_eq!(
        result.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let job = fs::canonicalize(f.run.join("jobs/gate-tests")).unwrap();
    let log = fs::read_to_string(job.join("stdout.log")).unwrap();
    let seen: serde_json::Value = serde_json::from_str(
        log.lines()
            .find_map(|l| l.strip_prefix("OBSERVED "))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(seen["cwd"], job.join("src").display().to_string());
    assert_eq!(seen["x.rs"], "fn b() {}\n");
    // Only sandbox-issued environment reaches the gate, with container paths under its own /work.
    let env = seen["env"].as_object().unwrap();
    assert!(env.keys().all(|k| !k.starts_with("DDF_")), "{env:?}");
    assert_eq!(env["HOME"], job.join("work/home").display().to_string());
    assert_eq!(
        env["CARGO_TARGET_DIR"],
        job.join("work/target").display().to_string()
    );
    assert_eq!(env["CARGO_NET_OFFLINE"], "true");
    assert_eq!(
        env["CARGO_HOME"],
        job.join("cargo-home").display().to_string()
    );
    assert_eq!(env["RUSTUP_TOOLCHAIN"], "1.99.1");
}

#[test]
fn antipattern_gate_runs_the_controllers_pinned_checker() {
    let f = Fixture::new();
    let planted = f.repository.join("scripts/security");
    fs::create_dir_all(&planted).unwrap();
    fs::write(
        planted.join("review_policy_check.py"),
        "print('{\"state\":\"passed\",\"workspace_members_checked\":1}')\n",
    )
    .unwrap();
    for args in [
        vec!["add", "scripts"],
        vec![
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-qm",
            "plant a self-grading checker",
        ],
    ] {
        assert!(
            Command::new("git")
                .args(args)
                .current_dir(&f.repository)
                .status()
                .unwrap()
                .success()
        );
    }
    f.prepare();
    let result = f.gate("antipatterns");
    assert_eq!(
        result.status.code(),
        Some(7),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let receipts: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_ne!(receipts[0]["outcome"], "passed");
    let job = f.run.join("jobs/gate-antipatterns");
    assert!(
        !fs::read_to_string(job.join("stdout.log"))
            .unwrap()
            .contains("\"workspace_members_checked\":1")
    );
    assert_eq!(
        fs::read_to_string(job.join("work/review_policy_check.py")).unwrap(),
        include_str!("../../deep-diff-forge-review/policy/review_policy_check.py")
    );
    let argv = f.runtime_argv("gate-antipatterns").concat();
    assert!(argv.iter().any(|a| a == "/work/review_policy_check.py"));
    assert!(argv.iter().all(|a| !a.contains("scripts/security")));
    assert_eq!(f.status().status.code(), Some(7));
}

#[test]
fn gates_refuse_without_a_sandbox() {
    for image in [None, Some("absent-image")] {
        let f = Fixture::new();
        f.prepare();
        f.install_gate("valid");
        let mut command = f.command(
            &["gate", f.run.to_str().unwrap(), "tests", "--execute"],
            "normal",
        );
        match image {
            None => command.env_remove("DDF_COUNCIL_GATE_IMAGE"),
            Some(image) => command.env("DDF_COUNCIL_GATE_IMAGE", image),
        };
        let result = command.output().unwrap();
        assert_eq!(result.status.code(), Some(4), "{image:?}");
        assert!(
            String::from_utf8_lossy(&result.stderr).contains("gate sandbox unavailable"),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(fs::read_dir(f.run.join("jobs")).unwrap().count(), 0);
    }
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

fn receipts(output: &Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|_| {
        panic!(
            "no receipts: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )
    })
}

#[test]
fn candidate_toolchain_configuration_is_refused_before_any_job_slot() {
    for planted in [
        ".cargo/config.toml",
        ".cargo/config",
        "rust-toolchain.toml",
        "rust-toolchain",
        "fuzz/.cargo/config.toml",
    ] {
        let f = Fixture::new();
        f.commit(
            planted,
            "[build]\nrustc-wrapper = \"/work/src/wrapper\"\n[toolchain]\npath = \"/work/evil\"\n",
        );
        f.prepare();
        f.install_gate("valid");
        for gate in ["tests", "antipatterns"] {
            let result = f.gate(gate);
            assert_eq!(result.status.code(), Some(4), "{planted}/{gate}");
            assert!(
                String::from_utf8_lossy(&result.stderr)
                    .contains(&format!("candidate toolchain configuration {planted}")),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
        assert_eq!(fs::read_dir(f.run.join("jobs")).unwrap().count(), 0);
        assert!(!f.run.join("controller.lock").exists());
    }
}

#[test]
fn sealed_paths_under_the_fuzz_artifact_mountpoint_are_refused_before_any_job_slot() {
    for planted in ["fuzz/artifacts/council_report/x.rs", "fuzz/artifacts"] {
        let f = Fixture::new();
        f.commit(planted, "pub fn sealed() {}\n");
        f.prepare();
        f.install_gate("valid");
        for gate in ["fuzz", "tests"] {
            let result = f.gate(gate);
            assert_eq!(result.status.code(), Some(4), "{planted}/{gate}");
            assert!(
                String::from_utf8_lossy(&result.stderr).contains(&format!(
                    "snapshot contains {planted} under the fuzz artifact mountpoint"
                )),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
        assert_eq!(fs::read_dir(f.run.join("jobs")).unwrap().count(), 0);
        assert!(!f.run.join("controller.lock").exists());
    }
}

#[test]
fn deleted_toolchain_configuration_selects_nothing_and_is_not_refused() {
    let f = Fixture::new();
    f.commit(
        ".cargo/config.toml",
        "[build]\nrustc-wrapper = \"/work/wrapper\"\n",
    );
    // Tracked but deleted in the workspace: inventoried, absent from the snapshot.
    fs::remove_file(f.repository.join(".cargo/config.toml")).unwrap();
    f.prepare();
    f.install_gate("valid");
    let result = f.gate("tests");
    assert_eq!(
        result.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn vendoring_output_is_sealed_and_planted_links_or_fifos_are_never_mounted() {
    for mode in [
        "vendor_symlink",
        "vendor_root_symlink",
        "vendor_fifo",
        // Empty directories count against the entry budget, like files.
        "vendor_dirs",
    ] {
        let f = Fixture::new();
        f.prepare();
        f.install_gate(mode);
        let result = f.gate("tests");
        assert_eq!(
            result.status.code(),
            Some(4),
            "{mode}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(
            String::from_utf8_lossy(&result.stderr).contains("dependency vendoring did not pass"),
            "{mode}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let vendor = f.run.join("jobs/sandbox-vendor");
        let refusal = fs::read_to_string(vendor.join("qualification-error.txt")).unwrap();
        assert!(
            refusal.contains("vendored output refused"),
            "{mode}: {refusal}"
        );
        assert!(!vendor.join("vendor-seal.json").exists());
        assert!(!f.run.join("vendor").exists(), "{mode}");
        assert!(!f.run.join("jobs/gate-tests").exists(), "{mode}");
        // A later gate is refused too, rather than mounting what the vendoring job left behind.
        assert_eq!(f.gate("check").status.code(), Some(4), "{mode}");
        assert!(!f.run.join("jobs/gate-check").exists(), "{mode}");
    }

    // A sealed copy changed after vendoring is re-measured and refused before the next gate's slot.
    let f = Fixture::new();
    f.prepare();
    f.install_gate("valid");
    assert_eq!(f.gate("tests").status.code(), Some(0));
    fs::write(f.run.join("vendor/crate-1.0.0/added.rs"), "// planted\n").unwrap();
    let result = f.gate("check");
    assert_eq!(result.status.code(), Some(4));
    assert!(
        String::from_utf8_lossy(&result.stderr).contains("sealed vendor directory changed"),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(!f.run.join("jobs/gate-check").exists());
}

#[test]
fn a_fifo_or_symlinked_directory_mutation_witness_is_refused_without_hanging() {
    for mode in ["fifo", "symlinked_dir"] {
        let f = Fixture::new();
        f.prepare();
        f.install_gate(mode);
        let result = f.gate("mutation");
        assert_eq!(
            result.status.code(),
            Some(7),
            "{mode}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(receipts(&result)[0]["outcome"], "inconclusive", "{mode}");
        let job = f.run.join("jobs/gate-mutation");
        let refusal = fs::read_to_string(job.join("qualification-error.txt")).unwrap();
        assert!(
            refusal.contains("mutation witness refused"),
            "{mode}: {refusal}"
        );
        let qualification: serde_json::Value =
            serde_json::from_slice(&fs::read(job.join("qualification.json")).unwrap()).unwrap();
        assert_eq!(qualification["witness_sha256"], serde_json::Value::Null);
        assert_eq!(f.status().status.code(), Some(7), "{mode}");
    }
}

#[test]
fn a_container_that_is_not_provably_removed_never_qualifies() {
    for runtime in ["linger:gate-tests", "hang_exists:gate-tests"] {
        let f = Fixture::new();
        f.prepare();
        f.install_gate("valid");
        let result = f.gate_with("tests", runtime);
        assert_eq!(
            result.status.code(),
            Some(7),
            "{runtime}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(receipts(&result)[0]["outcome"], "inconclusive", "{runtime}");
        let job = f.run.join("jobs/gate-tests");
        let refusal = fs::read_to_string(job.join("qualification-error.txt")).unwrap();
        assert!(
            refusal.contains("not provably removed"),
            "{runtime}: {refusal}"
        );
        // The name that was proved (or not) gone is the name the container ran under.
        let environment: serde_json::Value =
            serde_json::from_slice(&fs::read(job.join("environment.json")).unwrap()).unwrap();
        let name = environment["container_name"].as_str().unwrap();
        assert!(
            f.control_log()
                .iter()
                .any(|c| c[..2] == ["container", "exists"] && c[2] == name)
        );
        assert_eq!(f.status().status.code(), Some(7), "{runtime}");
    }
    // The vendoring job is held to the same rule: nothing it left is sealed or mounted.
    let f = Fixture::new();
    f.prepare();
    f.install_gate("valid");
    let result = f.gate_with("tests", "linger:sandbox-vendor");
    assert_eq!(result.status.code(), Some(4));
    assert!(!f.run.join("jobs/sandbox-vendor/vendor-seal.json").exists());
    assert!(!f.run.join("vendor").exists());
}

#[test]
fn an_unresolvable_or_hanging_image_lookup_is_refused_before_any_job_slot() {
    for runtime in ["bad_id", "hang_inspect"] {
        let f = Fixture::new();
        f.prepare();
        f.install_gate("valid");
        let start = Instant::now();
        let result = f.gate_with("tests", runtime);
        assert_eq!(result.status.code(), Some(4), "{runtime}");
        assert!(
            String::from_utf8_lossy(&result.stderr).contains("gate sandbox unavailable"),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(start.elapsed() < Duration::from_secs(90), "{runtime}");
        assert_eq!(fs::read_dir(f.run.join("jobs")).unwrap().count(), 0);
    }
}

#[test]
fn a_control_command_descendant_is_killed_when_the_runtime_exits() {
    let f = Fixture::new();
    f.prepare();
    f.install_gate("valid");
    let start = Instant::now();
    let result = f.gate_with("tests", "orphan_inspect");
    assert_eq!(
        result.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    // Without the group kill, the held pipe would stall the lookup until its 30 s deadline.
    assert!(start.elapsed() < Duration::from_secs(25));
    let pid = fs::read_to_string(f.bin.join("orphan.pid")).unwrap();
    assert!(!Path::new(&format!("/proc/{}", pid.trim())).exists());
}

#[test]
fn gates_launch_the_resolved_image_id_with_read_only_inputs_and_bounded_resources() {
    let f = Fixture::new();
    f.prepare();
    f.install_gate("valid");
    let result = f.gate("tests");
    assert_eq!(
        result.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(receipts(&result)[0]["image_id"], FAKE_IMAGE_ID);
    let job = fs::canonicalize(f.run.join("jobs/gate-tests")).unwrap();
    let execution: serde_json::Value =
        serde_json::from_slice(&fs::read(job.join("execution.json")).unwrap()).unwrap();
    assert_eq!(execution["image_id"], FAKE_IMAGE_ID);
    let environment: serde_json::Value =
        serde_json::from_slice(&fs::read(job.join("environment.json")).unwrap()).unwrap();
    assert_eq!(environment["image_id"], FAKE_IMAGE_ID);
    assert_eq!(environment["image"], "fake-gate");
    let argv = f.runtime_argv("gate-tests").concat();
    // The mutable tag is resolved once at admission and never launched.
    assert!(argv.iter().any(|a| a == FAKE_IMAGE_ID));
    assert!(argv.iter().all(|a| a != "fake-gate"));
    for expected in [
        format!("--volume={}:/src:ro,Z", job.join("src").display()),
        format!(
            "--volume={}:/cargo-home:ro,Z",
            job.join("cargo-home").display()
        ),
        format!("--volume={}:/work:Z", job.join("work").display()),
        "--workdir=/src".into(),
        "--env=RUSTUP_TOOLCHAIN=1.99.1".into(),
        "--env=CARGO_HOME=/cargo-home".into(),
        "--memory=32g".into(),
        "--memory-swap=32g".into(),
    ] {
        assert!(argv.contains(&expected), "missing {expected}: {argv:?}");
    }
    assert!(argv.iter().any(|a| a.starts_with("--cpus=")));
    // The vendoring job alone has network and a writable Cargo home; it too runs on a read-only source.
    let vendor = f.runtime_argv("sandbox-vendor").concat();
    assert!(vendor.iter().all(|a| a != "--network=none"));
    assert!(
        vendor
            .iter()
            .any(|a| a == "--env=CARGO_HOME=/work/cargo-home")
    );
    assert!(vendor.iter().any(|a| a.ends_with("/src:/src:ro,Z")));
    assert!(vendor.iter().any(|a| a == "--env=RUSTUP_TOOLCHAIN=1.99.1"));
}

#[test]
fn fuzz_jobs_get_a_scratch_artifact_mount_over_the_read_only_source() {
    let f = Fixture::new();
    f.commit("fuzz/Cargo.toml", "[package]\nname = \"fixture-fuzz\"\n");
    f.prepare();
    f.install_gate("valid");
    let result = f.gate("fuzz");
    assert_eq!(
        result.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let job = f.run.join("jobs/gate-fuzz");
    // The mountpoint exists in the controller's copy but is outside the fingerprinted inventory.
    assert!(job.join("src/fuzz/artifacts").is_dir());
    let argv = f.runtime_argv("gate-fuzz").concat();
    assert!(
        argv.iter()
            .any(|a| a == "--tmpfs=/src/fuzz/artifacts:rw,nosuid,nodev,noexec,size=64m,U")
    );
}

#[test]
fn a_changed_workspace_or_held_lock_is_refused_before_any_job_slot() {
    let f = Fixture::new();
    f.prepare();
    f.install_gate("valid");
    fs::write(f.repository.join("x.rs"), "fn changed() {}\n").unwrap();
    let result = f.gate("tests");
    assert_eq!(result.status.code(), Some(4));
    assert!(
        String::from_utf8_lossy(&result.stderr).contains("workspace changed"),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(fs::read_dir(f.run.join("jobs")).unwrap().count(), 0);
    assert!(!f.run.join("controller.lock").exists());

    let f = Fixture::new();
    f.prepare();
    f.install_gate("valid");
    fs::write(f.run.join("controller.lock"), "another controller\n").unwrap();
    assert_eq!(f.gate("tests").status.code(), Some(4));
    assert_eq!(fs::read_dir(f.run.join("jobs")).unwrap().count(), 0);
    // The other controller's lock is not ours to remove.
    assert!(f.run.join("controller.lock").exists());
}

#[test]
fn container_names_carry_a_per_invocation_nonce() {
    let f = Fixture::new();
    f.prepare();
    f.install_gate("valid");
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(f.run.join("manifest.json")).unwrap()).unwrap();
    let prefix = format!(
        "ddf-council-{}-gate-tests",
        &manifest["run_id"].as_str().unwrap()[..16]
    );
    // Another invocation with the same run ID still holds the name the controller used to derive.
    fs::write(f.bin.join("occupied-names"), format!("{prefix}\n")).unwrap();
    let result = f.gate("tests");
    assert_eq!(
        result.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let environment: serde_json::Value =
        serde_json::from_slice(&fs::read(f.run.join("jobs/gate-tests/environment.json")).unwrap())
            .unwrap();
    let name = environment["container_name"].as_str().unwrap();
    let nonce = name.strip_prefix(&format!("{prefix}-")).unwrap();
    assert!(
        nonce.len() == 16 && nonce.bytes().all(|b| b.is_ascii_hexdigit()),
        "{name}"
    );
    // The other invocation's container is never removed in our name.
    let log = f.control_log();
    assert!(log.iter().all(|c| c.last().unwrap() != &prefix), "{log:?}");
    assert!(log.iter().any(|c| c.last().unwrap() == name));
}
