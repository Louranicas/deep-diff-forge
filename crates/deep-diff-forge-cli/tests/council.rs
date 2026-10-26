// Review council: Actual CLI process lifecycle with fake providers and tools, plus timeout/owned-child cleanup and zero-work refusals.
// Design/status: docs/REVIEW_COUNCIL.md#cli-controls
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fcouncil--crates--deep-diff-forge-cli--tests--council
//! Actual CLI/driver lifecycle against deterministic fake providers; no model/API calls.
use deep_diff_forge_review::{Profile, Request};
use std::os::unix::fs::PermissionsExt;
use std::{
    fs,
    path::PathBuf,
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
        for name in [
            "cargo",
            "ddf-cargo-mutants-noexclude",
            "ddf-cargo-mutants-strict",
        ] {
            let tool = self.bin.join(name);
            fs::write(&tool, FAKE_GATE.replace("@MODE@", mode)).unwrap();
            fs::set_permissions(&tool, fs::Permissions::from_mode(0o700)).unwrap();
        }
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
    if mode=='orphan_inspect':
        pidfile=os.path.join(here,'orphan.pid')
        if os.fork()==0:
            # A descendant that keeps the output pipe open after the runtime itself exits.
            with open(pidfile+'.tmp','w') as out:out.write(str(os.getpid()))
            os.rename(pidfile+'.tmp',pidfile)
            time.sleep(120);os._exit(0)
        # Exit only once the descendant is recorded, so the controller cannot kill it first.
        while not os.path.exists(pidfile):time.sleep(0.01)
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
/// `vendor_dirs` (more empty directories than the seal's entry budget) in the vendoring output;
/// `fifo` and `symlinked_dir` at the mutation witness. For the exemption oracle (installed again
/// as `ddf-cargo-mutants-noexclude` and `ddf-cargo-mutants-strict`): `exempt`/`exempt_txt` hide
/// `hidden` with something other than the allow-list; the rest hide it with `#[cfg(test)]`, and
/// `hidden_test` keeps it out of non-test units, `shadowed` compiles it in (a build script setting
/// `cfg(test)`), `restores` puts the neutralised file back during the build,
/// `release_ships` only in release, `neutral_fails`/`harness_fails` stop the non-test unit or
/// only the harness building when it is neutralised, `mir_blind` emits MIR no change can alter,
/// and `unreproducible` differs on every build.
const FAKE_GATE: &str = r"#!/usr/bin/python3
import json,os,pathlib,sys
mode='@MODE@'
if '--list' in sys.argv:
    # Exemption oracle listings over `ORACLE_LIB`: `shipped` (lines 1-3) is always mutated; the
    # no-exclusion build also names `hidden` (lines 4-6, or 1-3 of the `.txt` module); the strict
    # build names it too when something other than `#[cfg(test)]`/`#[test]` hid it (`exempt*`).
    def fn(name,first,file='src/lib.rs'):
        span={'start':{'line':first,'column':1},'end':{'line':first+2,'column':2}}
        body={'start':{'line':first+1,'column':5},'end':{'line':first+1,'column':6}}
        return {'file':file,'genre':'FnValue','span':body,'function':{'function_name':name,'span':span}}
    tool=os.path.basename(sys.argv[0]);rows=[fn('shipped',1)]
    hidden=fn('hidden',1,'src/hidden.txt') if mode=='exempt_txt' else fn('hidden',4)
    allow_listed={'hidden_test','shadowed','release_ships','neutral_fails','harness_fails','mir_blind','unreproducible','restores'}
    if 'noexclude' in tool and (mode in allow_listed or mode.startswith('exempt')):rows.append(hidden)
    if 'strict' in tool and mode.startswith('exempt'):rows.append(hidden)
    print(json.dumps(rows));sys.exit(0)
if sys.argv[1:2]==['metadata']:
    root=os.path.join(os.getcwd(),'src','lib.rs')
    package={'name':'deep-diff-forge-review','manifest_path':os.path.join(os.getcwd(),'Cargo.toml'),'targets':[{'name':'deep-diff-forge-review','kind':['rlib','cdylib'],'src_path':root}]}
    print(json.dumps({'packages':[package]}));sys.exit(0)
if sys.argv[1:2]==['build'] and '--keep-going' in sys.argv:
    ORACLE_SOURCE='pub fn shipped() -> u32 {\n    1\n}\npub fn hidden() -> u32 {\n    2\n}\n'
    # The oracle's builds. Non-test MIR stands in as the source text the unit compiles; `--tests`
    # (the gate configuration) also builds a harness.
    assert 'CARGO_TARGET_DIR' not in os.environ and 'CARGO_INCREMENTAL' not in os.environ,'not the gate environment'
    assert os.environ.get('CARGO_ENCODED_RUSTFLAGS')=='--emit=link,mir' and os.environ.get('INSTA_UPDATE')=='no'
    gate='--tests' in sys.argv
    assert not gate or sys.argv[sys.argv.index('--profile')+1]=='test','the gate builds the test profile'
    lines=pathlib.Path('src/lib.rs').read_text().split('\n')
    neutralised=lines[4]!='    2'
    # `restores`: a build script that puts the neutralised file back during the build.
    if mode=='restores' and neutralised:pathlib.Path('src/lib.rs').write_text(ORACLE_SOURCE)
    shipped,hidden='\n'.join(lines[:3]),'\n'.join(lines[3:])
    ships=mode=='shadowed' or (mode=='release_ships' and '--release' in sys.argv)
    nontest=shipped+'\n'+hidden if ships else shipped
    if mode=='mir_blind':nontest='constant'
    if mode=='unreproducible':import time;nontest+='\n'+str(time.time_ns())
    deps=pathlib.Path(os.getcwd(),'target','release' if '--release' in sys.argv else 'debug','deps');deps.mkdir(parents=True,exist_ok=True)
    def artifact(test,name,mir):
        (deps/name).write_text('binary')
        stem=name.split('.')[0].removeprefix('lib') if '.' in name else name
        (deps/(stem+'.mir')).write_text(mir)
        print(json.dumps({'reason':'compiler-artifact','manifest_path':os.path.join(os.getcwd(),'Cargo.toml'),'target':{'name':'deep-diff-forge-review','kind':['rlib','cdylib']},'profile':{'test':test},'filenames':[str(deps/name)]}))
    if not (mode=='neutral_fails' and neutralised):artifact(False,'libdeep_diff_forge_review-0f.rlib',nontest)
    harness_breaks=mode=='harness_fails' and neutralised
    if gate and not harness_breaks:artifact(True,'deep_diff_forge_review-7e',shipped+'\n'+hidden)
    sys.exit(101 if (mode=='neutral_fails' and neutralised) or (gate and harness_breaks) else 0)
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
                .any(|w| w == ["--in-diff", "/cargo-home/mutation.diff"])
        );
        assert_eq!(
            fs::read_to_string(f.run.join("jobs/gate-mutation/cargo-home/mutation.diff")).unwrap(),
            sealed
        );
    }
}

/// A fixture prepared with `mutation_base` set to the commit before `setup` runs.
fn prepare_with_mutation_base(f: &Fixture, setup: impl Fn(&Fixture)) -> Output {
    let head = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&f.repository)
        .output()
        .unwrap();
    let mut spec: serde_json::Value =
        serde_json::from_slice(&fs::read(&f.request).unwrap()).unwrap();
    spec["mutation_base"] = String::from_utf8(head.stdout).unwrap().trim().into();
    fs::write(&f.request, serde_json::to_vec(&spec).unwrap()).unwrap();
    setup(f);
    f.call(
        &[
            "prepare",
            f.request.to_str().unwrap(),
            f.run.to_str().unwrap(),
        ],
        "normal",
    )
}

#[test]
fn index_flags_attributes_and_untracked_files_cannot_shrink_the_mutation_scope() {
    let f = Fixture::new();
    f.commit("hidden.rs", "pub fn hidden() {}\n");
    f.commit("binary.rs", "pub fn binary() {}\n");
    f.commit(
        "multi.rs",
        "pub fn first() {}\npub fn second() {}\npub fn third() {}\n",
    );
    f.commit("sub/inner.rs", "pub fn inner() {}\n");
    f.commit("sub/changed.rs", "pub fn before() {}\n");
    let result = prepare_with_mutation_base(&f, |f| {
        // Each of these hid a change from `git diff <base>` of the working tree.
        f.commit(".gitattributes", "binary.rs -diff\n");
        fs::write(f.repository.join("binary.rs"), "pub fn binary() { 1; }\n").unwrap();
        fs::write(f.repository.join("hidden.rs"), "pub fn hidden() { 2; }\n").unwrap();
        assert!(
            Command::new("git")
                .args(["update-index", "--assume-unchanged", "hidden.rs"])
                .current_dir(&f.repository)
                .status()
                .unwrap()
                .success()
        );
        fs::write(f.repository.join("untracked.rs"), "pub fn untracked() {}\n").unwrap();
        fs::write(
            f.repository.join("multi.rs"),
            "pub fn first() {}\npub fn changed() {}\npub fn third() {}\n",
        )
        .unwrap();
        fs::write(f.repository.join("sub/changed.rs"), "pub fn after() {}\n").unwrap();
    });
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let sealed = fs::read_to_string(f.run.join("mutation.diff")).unwrap();
    for added in [
        "+pub fn binary() { 1; }",
        "+pub fn hidden() { 2; }",
        "+pub fn untracked() {}",
    ] {
        assert!(sealed.contains(added), "{added} missing from {sealed}");
    }
    // Precise as well as complete: an unchanged line of a changed base file is not in scope.
    assert!(
        sealed.contains(
            "+++ b/multi.rs\n@@ -2,1 +2,1 @@\n-pub fn second() {}\n+pub fn changed() {}\n"
        ),
        "{sealed}"
    );
    assert!(!sealed.contains("+pub fn first()"), "{sealed}");
    // An unchanged file is not in scope at all, nested or not, and a nested change is exact.
    assert!(
        !sealed.contains("x.rs") && !sealed.contains("sub/inner.rs"),
        "{sealed}"
    );
    assert!(
        sealed.contains(
            "+++ b/sub/changed.rs\n@@ -1,1 +1,1 @@\n-pub fn before() {}\n+pub fn after() {}\n"
        ),
        "{sealed}"
    );
    // Every inventoried file is in scope, since `#[path]` can make any file a module.
    assert!(sealed.contains("+++ b/.gitattributes"), "{sealed}");
}

#[test]
fn analysis_admission_binds_the_review_id_and_refuses_unusable_analyses() {
    type Edit = dyn Fn(&mut serde_json::Value, &mut serde_json::Value);
    fn important(_: &mut serde_json::Value, spec: &mut serde_json::Value) {
        spec["profile"] = "important_decision".into();
        spec["reviewers"] =
            serde_json::to_value(deep_diff_forge_review::cli::example().reviewers).unwrap();
    }
    let prepare = |edit: &Edit| -> (Fixture, Output) {
        let f = Fixture::new();
        let path = f.root.join("analysis.json");
        let mut analysis: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let mut spec: serde_json::Value =
            serde_json::from_slice(&fs::read(&f.request).unwrap()).unwrap();
        edit(&mut analysis, &mut spec);
        fs::write(&path, serde_json::to_vec(&analysis).unwrap()).unwrap();
        fs::write(&f.request, serde_json::to_vec(&spec).unwrap()).unwrap();
        let result = f.call(
            &[
                "prepare",
                f.request.to_str().unwrap(),
                f.run.to_str().unwrap(),
            ],
            "normal",
        );
        (f, result)
    };
    // Admitted: the sealed manifest carries the analysis's own review ID.
    let (f, result) = prepare(&|_, _| {});
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(f.run.join("manifest.json")).unwrap()).unwrap();
    let analysis: serde_json::Value =
        serde_json::from_slice(&fs::read(f.root.join("analysis.json")).unwrap()).unwrap();
    assert_eq!(manifest["review_id"], analysis["review_id"]);
    // The important profile admits the same analysis when every file is analysed.
    let (_, result) = prepare(&important);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let refusals: [(&str, &Edit); 4] = [
        ("invalid review_id", &|a, _| a["review_id"] = "abc".into()),
        ("empty or mismatched analysis coverage", &|a, _| {
            a["coverage"] = serde_json::json!([]);
        }),
        ("coverage paths do not uniquely match files", &|a, _| {
            a["coverage"][0]["path"] = "other.rs".into();
        }),
        (
            "important decision has missing/partial/unavailable source analysis",
            &|a, spec| {
                important(a, spec);
                a["coverage"][0]["coverage"] = "partial_lcs".into();
            },
        ),
    ];
    for (message, edit) in refusals {
        let (f, result) = prepare(edit);
        assert!(!result.status.success(), "{message}");
        assert!(
            String::from_utf8_lossy(&result.stderr).contains(message),
            "{message}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(!f.run.exists(), "{message}");
    }
}

/// Two functions for the fake exemption oracle; `hidden.txt` is a non-`.rs` module.
const ORACLE_LIB: &str =
    "pub fn shipped() -> u32 {\n    1\n}\npub fn hidden() -> u32 {\n    2\n}\n";

fn oracle_fixture(mode: &str) -> Fixture {
    let f = Fixture::new();
    f.commit("src/lib.rs", ORACLE_LIB);
    f.commit("src/hidden.txt", "pub fn hidden() -> u32 {\n    3\n}\n");
    f.prepare();
    f.install_gate(mode);
    f
}

fn oracle_witness(f: &Fixture) -> serde_json::Value {
    serde_json::from_slice(
        &fs::read(
            f.run
                .join("jobs/sandbox-exemption-oracle/work/exemption-oracle.json"),
        )
        .unwrap(),
    )
    .unwrap()
}

#[test]
fn exclusions_other_than_cfg_test_are_refused_without_building() {
    for mode in ["exempt", "exempt_txt"] {
        // The strict listing still names `hidden`: something other than `#[cfg(test)]` or a bare
        // `#[test]` hid it, so no build can clear it (council r16).
        let f = oracle_fixture(mode);
        let result = f.gate("mutation");
        let stderr = String::from_utf8_lossy(&result.stderr);
        assert_eq!(result.status.code(), Some(4), "{mode}: {stderr}");
        assert!(
            stderr.contains("candidate code cannot exempt itself from the mutation gate")
                && stderr.contains("hidden"),
            "{stderr}"
        );
        let witness = oracle_witness(&f);
        assert_eq!(witness["state"], "refused", "{witness}");
        assert_eq!(witness["refused"][0]["function"], "hidden");
        assert_eq!(witness["builds"], 0, "{witness}");
        assert!(!f.run.join("jobs/gate-mutation").exists());
        assert_eq!(f.gate("mutation").status.code(), Some(4));
        let argv = f.runtime_argv("sandbox-exemption-oracle").concat();
        assert!(
            argv.iter().any(|a| a == "/work/exemption_oracle.py"),
            "{argv:?}"
        );
        assert!(argv.iter().any(|a| a == "--network=none"), "{argv:?}");
    }
}

#[test]
fn allow_listed_test_code_passes() {
    let f = oracle_fixture("hidden_test");
    let result = f.gate("mutation");
    assert_eq!(
        result.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let witness = oracle_witness(&f);
    assert_eq!(witness["state"], "passed", "{witness}");
    assert_eq!(witness["allow_listed"], 1);
    assert_eq!(witness["control"]["mir_changed"], true);
}

#[test]
fn allow_listed_code_compiled_into_shipped_units_is_refused() {
    // A shadowed `#[test]`, a build script setting cfg(test), or either only in release.
    for (mode, configuration) in [("shadowed", "gate"), ("release_ships", "release")] {
        let f = oracle_fixture(mode);
        let result = f.gate("mutation");
        assert_eq!(
            result.status.code(),
            Some(4),
            "{mode}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let witness = oracle_witness(&f);
        assert_eq!(witness["state"], "refused", "{witness}");
        assert!(
            witness["shipping_configurations"]
                .as_array()
                .unwrap()
                .iter()
                .any(|c| c == configuration),
            "{witness}"
        );
    }
}

#[test]
fn a_non_test_unit_that_stops_building_counts_as_shipping() {
    let f = oracle_fixture("neutral_fails");
    assert_eq!(f.gate("mutation").status.code(), Some(4));
    assert_eq!(oracle_witness(&f)["state"], "refused");
}

#[test]
fn a_test_harness_that_stops_building_is_still_test_code() {
    // A neutralised test-only `const fn` used in a test constant breaks only the harness.
    let f = oracle_fixture("harness_fails");
    let result = f.gate("mutation");
    assert_eq!(
        result.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(oracle_witness(&f)["state"], "passed");
}

#[test]
fn a_build_that_does_not_reproduce_is_inconclusive_and_refused() {
    let f = oracle_fixture("unreproducible");
    assert_eq!(f.gate("mutation").status.code(), Some(4));
    let witness = oracle_witness(&f);
    assert_eq!(witness["state"], "inconclusive", "{witness}");
    assert_eq!(witness["reproducible"], false);
    assert!(!f.run.join("jobs/gate-mutation").exists());
}

#[test]
fn a_build_that_restores_a_neutralised_file_is_inconclusive() {
    // Unchanged MIR proves nothing if the build put the original body back (council r17).
    let f = oracle_fixture("restores");
    assert_eq!(f.gate("mutation").status.code(), Some(4));
    let witness = oracle_witness(&f);
    assert_eq!(witness["state"], "inconclusive", "{witness}");
    assert!(
        witness["reasons"][0]
            .as_str()
            .unwrap()
            .contains("changed during the build"),
        "{witness}"
    );
}

#[test]
fn the_mutation_job_emits_mir_like_the_oracle() {
    // The flag the oracle needs is in both jobs' environment, so it tells a build script nothing.
    let f = oracle_fixture("hidden_test");
    assert_eq!(f.gate("mutation").status.code(), Some(0));
    for job in ["sandbox-exemption-oracle", "gate-mutation"] {
        let argv = f.runtime_argv(job).concat();
        assert!(
            argv.iter()
                .any(|a| a == "--env=CARGO_ENCODED_RUSTFLAGS=--emit=link,mir"),
            "{job}: {argv:?}"
        );
    }
}

#[test]
fn a_comparison_the_positive_control_cannot_move_is_inconclusive_and_refused() {
    let f = oracle_fixture("mir_blind");
    let result = f.gate("mutation");
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert_eq!(result.status.code(), Some(4), "{stderr}");
    assert!(stderr.contains("inconclusive"), "{stderr}");
    assert_eq!(oracle_witness(&f)["control"]["mir_changed"], false);
    assert!(!f.run.join("jobs/gate-mutation").exists());
}

/// Stands in for the oracle inside the fake sandbox: writes `@WITNESS@` and exits `@CODE@`.
const FAKE_ORACLE: &str = r"#!/usr/bin/python3
import os,sys
if 'exemption_oracle.py' not in ' '.join(sys.argv):os.execv('/usr/bin/python3',['/usr/bin/python3']+sys.argv[1:])
open(sys.argv[sys.argv.index('--witness')+1],'w').write('@WITNESS@')
sys.exit(@CODE@)
";

#[test]
fn the_controller_checks_the_oracle_witness_as_well_as_its_exit() {
    // A zero exit cannot pass a vacuous witness, and a passing witness cannot pass a failed exit.
    for (witness, code) in [
        (
            r#"{"state":"passed","hidden_functions":1,"control":{"mir_changed":false},"reproducible":true}"#,
            0,
        ),
        (
            r#"{"state":"passed","hidden_functions":1,"control":{"mir_changed":true},"reproducible":false}"#,
            0,
        ),
        (r#"{"state":"passed","hidden_functions":0}"#, 1),
        (
            r#"{"state":"passed","hidden_functions":1,"control":{"mir_changed":true},"reproducible":true,"configurations":{"gate":"built","release":"does not build"}}"#,
            0,
        ),
    ] {
        let f = Fixture::new();
        f.prepare();
        f.install_gate("valid");
        let oracle = f.bin.join("python3");
        fs::write(
            &oracle,
            FAKE_ORACLE
                .replace("@WITNESS@", witness)
                .replace("@CODE@", &code.to_string()),
        )
        .unwrap();
        fs::set_permissions(&oracle, fs::Permissions::from_mode(0o700)).unwrap();
        let result = f.gate("mutation");
        assert_eq!(
            result.status.code(),
            Some(4),
            "{witness}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(!f.run.join("jobs/gate-mutation").exists(), "{witness}");
    }
}

fn git(f: &Fixture, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(&f.repository)
        .output()
        .unwrap();
    assert!(output.status.success(), "{args:?}");
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

#[test]
fn replacement_refs_cannot_substitute_a_base_blob() {
    let f = Fixture::new();
    f.commit("swapped.rs", "pub fn original() {}\n");
    let result = prepare_with_mutation_base(&f, |f| {
        fs::write(f.repository.join("swapped.rs"), "pub fn changed() {}\n").unwrap();
        // Make the base's blob read back as the candidate's bytes, hiding the change.
        let base_blob = git(f, &["rev-parse", "HEAD:swapped.rs"]);
        let candidate_blob = git(f, &["hash-object", "-w", "swapped.rs"]);
        git(f, &["replace", &base_blob, &candidate_blob]);
    });
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let sealed = fs::read_to_string(f.run.join("mutation.diff")).unwrap();
    assert!(sealed.contains("+pub fn changed() {}"), "{sealed}");
}

#[test]
fn a_forged_base_object_is_refused_before_any_run_exists() {
    let f = Fixture::new();
    f.commit("forged.rs", "pub fn real() {}\n");
    let result = prepare_with_mutation_base(&f, |f| {
        // The candidate changes the file, so its base blob must be read; then the base blob's
        // loose object is overwritten with the candidate's bytes under the genuine ID, which would
        // make the change vanish. `cat-file` serves them; only re-hashing on read notices.
        fs::write(f.repository.join("forged.rs"), "pub fn forged() {}\n").unwrap();
        let oid = git(f, &["rev-parse", "HEAD:forged.rs"]);
        let object = f
            .repository
            .join(format!(".git/objects/{}/{}", &oid[..2], &oid[2..]));
        let mut permissions = fs::metadata(&object).unwrap().permissions();
        permissions.set_mode(0o644);
        fs::set_permissions(&object, permissions).unwrap();
        let forged = Command::new("python3")
            .args([
                "-I",
                "-c",
                "import sys,zlib;b=b'pub fn forged() {}\\n';sys.stdout.buffer.write(zlib.compress(b'blob %d\\0'%len(b)+b))",
            ])
            .output()
            .unwrap();
        fs::write(&object, forged.stdout).unwrap();
    });
    assert_eq!(result.status.code(), Some(4));
    assert!(
        String::from_utf8_lossy(&result.stderr).contains("a base object does not hash to its ID"),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(!f.run.exists());
}

#[test]
fn a_binary_base_file_turned_into_rust_is_in_scope() {
    let f = Fixture::new();
    // The base holds a non-UTF-8 blob at the path the candidate turns into Rust.
    fs::write(f.repository.join("module.rs"), [0xff, b'\n']).unwrap();
    git(&f, &["add", "module.rs"]);
    git(
        &f,
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-qm",
            "binary base",
        ],
    );
    let result = prepare_with_mutation_base(&f, |f| {
        fs::write(
            f.repository.join("module.rs"),
            "pub fn now_rust() -> bool { false }\n",
        )
        .unwrap();
    });
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let sealed = fs::read_to_string(f.run.join("mutation.diff")).unwrap();
    assert!(
        sealed.contains("+++ b/module.rs\n@@ -0,0 +1,1 @@\n+pub fn now_rust() -> bool { false }"),
        "{sealed}"
    );
}

#[test]
fn a_submodule_replaced_by_a_file_diffs_against_empty() {
    let f = Fixture::new();
    // A gitlink in the base tree: its ID names a commit, not a blob.
    let head = git(&f, &["rev-parse", "HEAD"]);
    git(
        &f,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("160000,{head},vendored"),
        ],
    );
    git(
        &f,
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-qm",
            "gitlink",
        ],
    );
    let result = prepare_with_mutation_base(&f, |f| {
        git(f, &["rm", "-q", "--cached", "vendored"]);
        fs::write(f.repository.join("vendored"), "pub fn vendored() {}\n").unwrap();
    });
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let sealed = fs::read_to_string(f.run.join("mutation.diff")).unwrap();
    assert!(
        sealed.contains("+++ b/vendored\n@@ -0,0 +1,1 @@\n+pub fn vendored() {}"),
        "{sealed}"
    );
}

#[test]
fn path_characters_that_could_confuse_a_diff_parser_are_refused() {
    for (name, refused) in [
        ("non\u{a0}breaking.rs", true),
        ("\"quoted.rs", true),
        // Some diff parsers end a name at a space (Git appends a tab after such names).
        ("plain space.rs", true),
        ("plain-name.rs", false),
    ] {
        let f = Fixture::new();
        let result = prepare_with_mutation_base(&f, |f| {
            fs::write(f.repository.join(name), "pub fn named() {}\n").unwrap();
        });
        if refused {
            assert_eq!(result.status.code(), Some(4), "{name}");
            assert!(
                String::from_utf8_lossy(&result.stderr).contains("mutation scope path contains"),
                "{name}: {}",
                String::from_utf8_lossy(&result.stderr)
            );
        } else {
            assert!(
                result.status.success(),
                "{name}: {}",
                String::from_utf8_lossy(&result.stderr)
            );
            let sealed = fs::read_to_string(f.run.join("mutation.diff")).unwrap();
            assert!(sealed.contains(&format!("+++ b/{name}\n")), "{sealed}");
        }
    }
}

#[test]
fn a_changed_binary_file_needs_no_header_safe_name() {
    // A changed non-UTF-8 file is out of scope and gets no header (council r15).
    let f = Fixture::new();
    f.commit("assets/My Logo.png", "old");
    let result = prepare_with_mutation_base(&f, |f| {
        fs::write(
            f.repository.join("assets/My Logo.png"),
            [0xff_u8, 0xfe, 0x00],
        )
        .unwrap();
        fs::write(f.repository.join("changed.rs"), "pub fn changed() {}\n").unwrap();
    });
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn only_changed_files_need_header_safe_names() {
    // An unchanged file with a space in its name gets no header, so it cannot block the diff scope
    // (council r13); a control character still breaks the line-oriented hashing of every path.
    for (name, refused) in [("docs/Design Notes.md", false), ("tab\tname.md", true)] {
        let f = Fixture::new();
        f.commit(name, "unchanged\n");
        let result = prepare_with_mutation_base(&f, |f| {
            fs::write(f.repository.join("changed.rs"), "pub fn changed() {}\n").unwrap();
        });
        assert_eq!(
            result.status.success(),
            !refused,
            "{name}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
}

#[test]
fn a_symlink_replaced_by_a_file_with_its_target_bytes_is_in_scope() {
    let f = Fixture::new();
    // A symlink's blob holds its target; a regular file with those bytes hashes the same.
    std::os::unix::fs::symlink("target.rs", f.repository.join("link.rs")).unwrap();
    git(&f, &["add", "link.rs"]);
    git(
        &f,
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-qm",
            "symlink",
        ],
    );
    let result = prepare_with_mutation_base(&f, |f| {
        fs::remove_file(f.repository.join("link.rs")).unwrap();
        fs::write(f.repository.join("link.rs"), "target.rs").unwrap();
    });
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let sealed = fs::read_to_string(f.run.join("mutation.diff")).unwrap();
    assert!(
        sealed.contains("+++ b/link.rs\n@@ -0,0 +1,1 @@\n+target.rs"),
        "{sealed}"
    );
}

#[test]
fn binary_files_are_out_of_scope_and_control_characters_in_paths_are_refused() {
    let f = Fixture::new();
    let result = prepare_with_mutation_base(&f, |f| {
        fs::write(f.repository.join("asset.bin"), [0xff, 0xfe, b'\n']).unwrap();
        fs::write(f.repository.join("text.rs"), "pub fn text() {}\n").unwrap();
    });
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let sealed = fs::read(f.run.join("mutation.diff")).unwrap();
    let sealed = String::from_utf8(sealed).expect("the sealed scope is UTF-8");
    assert!(
        !sealed.contains("asset.bin") && sealed.contains("+++ b/text.rs"),
        "{sealed}"
    );
    // A name with a newline could forge a header line; it is refused, not escaped.
    let f = Fixture::new();
    let result = prepare_with_mutation_base(&f, |f| {
        fs::write(
            f.repository.join("evil\n@@ -1,1 +1,1 @@.rs"),
            "pub fn evil() {}\n",
        )
        .unwrap();
    });
    assert_eq!(result.status.code(), Some(4));
    assert!(
        String::from_utf8_lossy(&result.stderr).contains("mutation scope path contains"),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    // A refusal after the run directory was created leaves nothing behind.
    assert!(!f.run.exists());
}

#[test]
fn a_mutation_base_that_is_not_an_ancestor_of_head_is_refused() {
    let f = Fixture::new();
    let result = prepare_with_mutation_base(&f, |f| {
        // Rewrite history so the recorded base is no longer an ancestor of HEAD.
        assert!(
            Command::new("git")
                .args([
                    "-c",
                    "user.name=Fixture",
                    "-c",
                    "user.email=fixture@example.invalid",
                    "commit",
                    "-q",
                    "--amend",
                    "-m",
                    "rewritten",
                ])
                .current_dir(&f.repository)
                .status()
                .unwrap()
                .success()
        );
    });
    assert_eq!(result.status.code(), Some(4));
    assert!(
        String::from_utf8_lossy(&result.stderr)
            .contains("must be a commit that is an ancestor of HEAD"),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(!f.run.exists());
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
    // The candidate's `.cargo/mutants.toml` is never read.
    assert!(argv.iter().any(|a| a == "--no-config"));
    // The job runs as the operator, not as the image's USER (a uid-1001 runner could not read
    // the private source copy otherwise).
    let me = fs::metadata("/proc/self").unwrap();
    let user = format!(
        "--user={}:{}",
        std::os::unix::fs::MetadataExt::uid(&me),
        std::os::unix::fs::MetadataExt::gid(&me)
    );
    assert!(argv.contains(&user), "{argv:?}");
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
    for (planted, deleted) in [
        ("fuzz/artifacts/council_report/x.rs", false),
        ("fuzz/artifacts", false),
        // Deleted but inventoried: the controller would create the mountpoint over the deletion.
        ("fuzz/artifacts", true),
    ] {
        let f = Fixture::new();
        f.commit(planted, "pub fn sealed() {}\n");
        if deleted {
            f.commit("fuzz/keep.rs", "pub fn kept() {}\n");
            fs::remove_file(f.repository.join(planted)).unwrap();
        }
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
        if mode == "vendor_dirs" {
            // Creating 100,001 directories takes about 3 s on a loaded host, beyond the fixture's
            // 2 s job timeout; the case must be refused by the entry budget, not the clock.
            let mut spec: serde_json::Value =
                serde_json::from_slice(&fs::read(&f.request).unwrap()).unwrap();
            spec["timeout_seconds"] = 120.into();
            fs::write(&f.request, serde_json::to_vec(&spec).unwrap()).unwrap();
        }
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
    // SIGKILL is immediate, but the orphan's /proc entry lasts until its new parent reaps it.
    let proc = PathBuf::from(format!("/proc/{}", pid.trim()));
    let deadline = Instant::now() + Duration::from_secs(10);
    while proc.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!proc.exists(), "descendant {} survived", pid.trim());
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
