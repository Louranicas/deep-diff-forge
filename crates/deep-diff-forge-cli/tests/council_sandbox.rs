// Review council: Planted-breach controls for the real rootless Podman gate sandbox (opt-in, never a silent pass).
// Design/status: docs/REVIEW_COUNCIL.md#gate-sandbox
//! A candidate crate whose own `#[test]` tries to read agent credentials, reach the network, forge
//! controller evidence, write the image root and its own source, select a toolchain with a planted
//! `rust-toolchain.toml`, escape its cgroup bounds and see host sockets/PIDs. Each refusal must carry the
//! specific errno that only the control produces, and the probes with a plausible false refusal have a
//! positive control. Further runs plant a symlinked mutation witness, a symlink in the vendoring output
//! and candidate toolchain configuration. Runs only with `DDF_COUNCIL_SANDBOX_IT=1` plus
//! `DDF_COUNCIL_PODMAN` / `DDF_COUNCIL_GATE_IMAGE`; otherwise it prints an `UNMEASURED` line and returns.
use deep_diff_forge_review::Profile;
use std::os::unix::fs::MetadataExt;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

/// Every probe the planted test reports; each must appear exactly once, as `refused`.
const PROBES: [&str; 12] = [
    "read codex-auth",
    "read claude-credentials",
    "connect 1.1.1.1:443",
    "write run-jobs",
    "write /etc/ddf-probe",
    "write /opt/ddf-probe-writable",
    "write /src/src/lib.rs",
    "exists /run/host",
    "exists session-bus",
    "pids",
    "cgroup bounds",
    "toolchain file",
];

/// The real home of the current uid from the password database, never `$HOME`.
fn real_home(uid: u32) -> String {
    fs::read_to_string("/etc/passwd")
        .unwrap()
        .lines()
        .map(|line| line.split(':').collect::<Vec<_>>())
        .find(|fields| fields.len() >= 6 && fields[2] == uid.to_string())
        .map(|fields| fields[5].to_owned())
        .expect("current uid has a password-database entry")
}

/// Planted breaches: each probe prints one `BREACH` or `refused` line, never file contents.
const PROBE_CRATE: &str = r#"//! Planted breaches: each probe prints one `BREACH` or `refused` line, never file contents.
/// The mutation gate's target.
pub fn add(a: u64, b: u64) -> u64 {
    a + b
}
#[cfg(test)]
mod tests {
    use std::io::{ErrorKind, Write as _};
    use std::path::Path;
    use std::time::Duration;
    const ENOENT: i32 = 2;
    const EROFS: i32 = 30;
    const ENETUNREACH: i32 = 101;
    fn verdict(breached: bool, probe: &str, detail: &str) {
        // Written to the fd directly so libtest output capture cannot hide it.
        let mut out = std::io::stdout().lock();
        let word = if breached { "BREACH" } else { "refused" };
        writeln!(out, "{word} {probe}: {detail}").unwrap();
        out.flush().unwrap();
    }
    #[test]
    fn adds() {
        assert_eq!(super::add(2, 3), 5);
        assert_eq!(super::add(7, 0), 7);
    }
    /// Mutation-gate probe: when the snapshot carries `plant-witness-symlink`, move cargo-mutants'
    /// output directory aside and leave a symlink in its place, so the final witness is reached
    /// through a directory symlink that a leaf-only check would follow.
    #[test]
    fn plant_witness_symlink() {
        let out = Path::new("/work/mutation-results/mutants.out");
        if !Path::new("/src/plant-witness-symlink").exists() {
            return;
        }
        if std::fs::symlink_metadata(out).is_ok_and(|m| m.is_dir()) {
            let aside = Path::new("/work/mutation-results/aside");
            std::fs::rename(out, aside).unwrap();
            std::os::unix::fs::symlink(aside, out).unwrap();
        }
    }
    #[test]
    fn planted_breaches() {
        // The probes are measured in the tests gate; under cargo-mutants they would only repeat.
        if Path::new("/work/mutation-results").exists() {
            return;
        }
        for (probe, path) in [
            ("read codex-auth", "@HOME@/.codex/auth.json"),
            ("read claude-credentials", "@HOME@/.claude/.credentials.json"),
        ] {
            match std::fs::File::open(path) {
                Ok(_) => verdict(true, probe, "opened for reading"),
                Err(e) => verdict(e.kind() != ErrorKind::NotFound, probe, &e.to_string()),
            }
        }
        // No route at all: the namespace has only loopback. A timeout or reset would mean packets left.
        let address = "1.1.1.1:443".parse().unwrap();
        match std::net::TcpStream::connect_timeout(&address, Duration::from_secs(3)) {
            Ok(_) => verdict(true, "connect 1.1.1.1:443", "connected"),
            Err(e) => verdict(e.raw_os_error() != Some(ENETUNREACH), "connect 1.1.1.1:443", &e.to_string()),
        }
        // /opt/ddf-probe-writable is owned by the gate user, and /src/src/lib.rs by the operator mapped
        // through keep-id: only a read-only mount turns those writes into EROFS.
        for (probe, path, expected) in [
            ("write run-jobs", "@FORGED@", ENOENT),
            ("write /etc/ddf-probe", "/etc/ddf-probe", EROFS),
            ("write /opt/ddf-probe-writable", "/opt/ddf-probe-writable/probe", EROFS),
            ("write /src/src/lib.rs", "/src/src/lib.rs", EROFS),
        ] {
            match std::fs::OpenOptions::new().write(true).create(true).truncate(false).open(path) {
                Ok(_) => verdict(true, probe, "opened for writing"),
                Err(e) => verdict(e.raw_os_error() != Some(expected), probe, &e.to_string()),
            }
        }
        for (probe, path) in [("exists /run/host", "/run/host"), ("exists session-bus", "/run/user/@UID@/bus")] {
            let present = Path::new(path).exists();
            verdict(present, probe, if present { "present" } else { "absent" });
        }
        let pids = std::fs::read_dir("/proc")
            .unwrap()
            .filter(|e| e.as_ref().unwrap().file_name().to_string_lossy().bytes().all(|b| b.is_ascii_digit()))
            .count();
        verdict(pids > 32, "pids", &format!("{pids} visible in /proc"));
        let cgroup = |file: &str| {
            std::fs::read_to_string(format!("/sys/fs/cgroup/{file}")).unwrap_or_default().trim().to_owned()
        };
        let (memory, swap, cpu, tasks) = (cgroup("memory.max"), cgroup("memory.swap.max"), cgroup("cpu.max"), cgroup("pids.max"));
        let bounded = memory == "34359738368" && swap == "0" && tasks == "8192" && !cpu.is_empty() && !cpu.starts_with("max");
        verdict(!bounded, "cgroup bounds", &format!("memory.max={memory} memory.swap.max={swap} cpu.max={cpu} pids.max={tasks}"));
        // A planted toolchain file must not select the toolchain of a top-level gate command. rustup
        // exports RUSTUP_TOOLCHAIN to everything under `cargo`, so judge with the environment the
        // container started with (PID 1 is --init). Positive control: without RUSTUP_TOOLCHAIN that
        // same environment lets the file select nightly.
        let initial = std::fs::read("/proc/1/environ").unwrap_or_default();
        let initial = initial
            .split(|b| *b == 0)
            .filter_map(|entry| {
                let entry = String::from_utf8_lossy(entry);
                let (key, value) = entry.split_once('=')?;
                Some((key.to_owned(), value.to_owned()))
            })
            .collect::<Vec<_>>();
        let dir = Path::new("/work/tmp/toolchain-probe");
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("rust-toolchain.toml"), "[toolchain]\nchannel = \"nightly\"\n").unwrap();
        let version = |pinned: bool| {
            let output = std::process::Command::new("/opt/cargo/bin/rustc")
                .arg("--version")
                .current_dir(dir)
                .env_clear()
                .envs(initial.iter().filter(|(key, _)| pinned || key != "RUSTUP_TOOLCHAIN").cloned())
                .output()
                .unwrap();
            String::from_utf8_lossy(&output.stdout).trim().to_owned()
        };
        let (pinned, unpinned) = (version(true), version(false));
        verdict(
            !pinned.starts_with("rustc 1.99.1") || !unpinned.contains("nightly"),
            "toolchain file",
            &format!("container environment: {pinned}; positive control without RUSTUP_TOOLCHAIN: {unpinned}"),
        );
    }
}
"#;

fn git(repository: &Path, args: &[&str]) {
    assert!(
        Command::new("git")
            .args(args)
            .current_dir(repository)
            .status()
            .unwrap()
            .success()
    );
}

/// Relative to the run directory: a controller-evidence path the planted test tries to forge.
const FORGED: &str = "jobs/forged-by-gate.txt";

/// A committed probe crate (plus `extra` files) and its analysis/decision/request under a fresh root;
/// returns (root, request).
fn fixture(home: &str, uid: u32, label: &str, extra: &[(&str, &str)]) -> (PathBuf, PathBuf) {
    let root = std::env::temp_dir().join(format!(
        "ddf-council-sandbox-{}-{label}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir(&root).unwrap();
    let root = fs::canonicalize(&root).unwrap();
    let forged = root.join("run").join(FORGED);
    let repository = root.join("repo");
    fs::create_dir_all(repository.join("src")).unwrap();
    fs::write(
        repository.join("Cargo.toml"),
        "[package]\nname = \"ddf-sandbox-probe\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[workspace]\n",
    )
    .unwrap();
    fs::write(
        repository.join("Cargo.lock"),
        "# This file is automatically @generated by Cargo.\n# It is not intended for manual editing.\nversion = 4\n\n[[package]]\nname = \"ddf-sandbox-probe\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    let lib = PROBE_CRATE
        .replace("@HOME@", home)
        .replace("@UID@", &uid.to_string())
        .replace("@FORGED@", &forged.display().to_string());
    fs::write(repository.join("src/lib.rs"), &lib).unwrap();
    for (path, contents) in extra {
        fs::write(repository.join(path), contents).unwrap();
    }
    git(&repository, &["init", "-q"]);
    git(&repository, &["add", "."]);
    git(
        &repository,
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-qm",
            "planted-breach probe",
        ],
    );

    let snapshot =
        deep_diff_forge_core::changed_source_snapshot_id([("src/lib.rs", Some(lib.as_bytes()))]);
    let analysis = root.join("analysis.json");
    fs::write(
        &analysis,
        serde_json::to_vec(&serde_json::json!({
            "schema": "deep-diff-forge.analysis.v1",
            "review_id": "0".repeat(64),
            "candidate_snapshot": snapshot,
            "coverage": [{"path": "src/lib.rs", "coverage": "analyzed"}],
            "review": {"files": [{"path": "src/lib.rs", "status": "added"}]},
        }))
        .unwrap(),
    )
    .unwrap();
    let decision = root.join("decision.md");
    fs::write(
        &decision,
        "Measure the gate sandbox against planted breaches.\n",
    )
    .unwrap();
    let mut spec = deep_diff_forge_review::cli::example();
    spec.profile = Profile::Roster;
    spec.reviewers.truncate(2);
    spec.repository = repository.display().to_string();
    spec.analysis_file = analysis.display().to_string();
    spec.decision_file = decision.display().to_string();
    spec.timeout_seconds = 600;
    spec.mutation_package = "ddf-sandbox-probe".into();
    let request = root.join("request.json");
    fs::write(&request, serde_json::to_vec(&spec).unwrap()).unwrap();
    (root, request)
}

fn council(args: &[&str], runtime: Option<&str>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"));
    command.arg("council").args(args);
    if let Some(runtime) = runtime {
        command.env("DDF_COUNCIL_PODMAN", runtime);
    }
    command.output().unwrap()
}
fn prepare(request: &Path, run: &Path) {
    let prepared = council(
        &["prepare", request.to_str().unwrap(), run.to_str().unwrap()],
        None,
    );
    assert!(
        prepared.status.success(),
        "{}",
        String::from_utf8_lossy(&prepared.stderr)
    );
}
fn gate(run: &Path, gate: &str, runtime: Option<&str>) -> Output {
    council(&["gate", run.to_str().unwrap(), gate, "--execute"], runtime)
}
fn json(path: &Path) -> serde_json::Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}
/// The operator's runtime argv, as the controller splits `DDF_COUNCIL_PODMAN`.
fn runtime() -> Vec<String> {
    std::env::var("DDF_COUNCIL_PODMAN")
        .unwrap()
        .split_whitespace()
        .map(str::to_owned)
        .collect()
}
fn runtime_output(args: &[&str]) -> Output {
    let runtime = runtime();
    Command::new(&runtime[0])
        .args(&runtime[1..])
        .args(args)
        .output()
        .unwrap()
}

/// Wraps the real runtime: after the vendoring job's container has exited, plant a symlink to a host
/// directory (the fixture root, never the real home, in case it were relabelled) inside its output,
/// as a hostile vendoring step could have.
const PLANT_VENDOR_SYMLINK: &str = r"import os,re,subprocess,sys
args=sys.argv[1:]
code=subprocess.run(args).returncode
if 'run' in args and any(re.match(r'--name=.*-sandbox-vendor-',a) for a in args):
    work=next(a.split('=',1)[1].split(':')[0] for a in args if a.startswith('--volume=') and a.endswith(':/work:Z'))
    os.makedirs(os.path.join(work,'vendor'),exist_ok=True)
    os.symlink(sys.argv[0].rsplit('/',1)[0],os.path.join(work,'vendor','escape'))
sys.exit(code)
";

#[test]
fn real_sandbox_refuses_planted_breaches() {
    if std::env::var("DDF_COUNCIL_SANDBOX_IT").as_deref() != Ok("1") {
        eprintln!(
            "UNMEASURED: set DDF_COUNCIL_SANDBOX_IT=1 with DDF_COUNCIL_PODMAN and DDF_COUNCIL_GATE_IMAGE to run the real-Podman planted-breach controls; nothing was measured"
        );
        return;
    }
    for required in ["DDF_COUNCIL_PODMAN", "DDF_COUNCIL_GATE_IMAGE"] {
        assert!(
            std::env::var(required).is_ok_and(|v| !v.is_empty()),
            "{required} must be set when DDF_COUNCIL_SANDBOX_IT=1"
        );
    }
    let image = std::env::var("DDF_COUNCIL_GATE_IMAGE").unwrap();
    let uid = fs::metadata("/proc/self").unwrap().uid();
    let home = real_home(uid);
    // A probe against a missing or unreadable file would be a vacuous refusal.
    for credential in [".codex/auth.json", ".claude/.credentials.json"] {
        let path = Path::new(&home).join(credential);
        assert!(
            fs::File::open(&path).is_ok(),
            "{} is not readable on the host; the probe would be vacuous",
            path.display()
        );
    }
    // Positive control for the read-only-root probe: without --read-only the gate user can write there.
    let writable = runtime_output(&[
        "run",
        "--rm",
        "--pull=never",
        "--userns=keep-id",
        "--network=none",
        &image,
        "sh",
        "-c",
        "touch /opt/ddf-probe-writable/probe",
    ]);
    assert!(
        writable.status.success(),
        "positive control failed, so the read-only-root probe would be vacuous: {}",
        String::from_utf8_lossy(&writable.stderr)
    );
    let inspected = runtime_output(&["image", "inspect", "--format", "{{.Id}}", &image]);
    let image_id = String::from_utf8_lossy(&inspected.stdout).trim().to_owned();
    breach_probes(&home, uid, &image_id);
    // Also the oracle's positive control: an honest crate reaches the mutation job slot.
    witness_symlink_probe(&home, uid);
    vendor_symlink_probe(&home, uid);
    toolchain_config_probe(&home, uid);
    exemption_oracle_probe(&home, uid);
}

/// A build script that sets `cfg(test)` ships the probe's test module in the normal build, where
/// cargo-mutants still skips it. The exemption oracle must refuse before the mutation job exists,
/// with the build script run offline inside its own sandbox job.
fn exemption_oracle_probe(home: &str, uid: u32) {
    let build = "fn main() {\n    println!(\"cargo:rustc-cfg=test\");\n}\n";
    let (root, request) = fixture(home, uid, "exemption", &[("build.rs", build)]);
    let run = root.join("run");
    prepare(&request, &run);
    let mutation = gate(&run, "mutation", None);
    let stderr = String::from_utf8_lossy(&mutation.stderr);
    assert_eq!(mutation.status.code(), Some(4), "{stderr}");
    assert!(
        stderr.contains("candidate code cannot exempt itself from the mutation gate"),
        "{stderr}"
    );
    let witness = json(&run.join("jobs/sandbox-exemption-oracle/work/exemption-oracle.json"));
    assert_eq!(witness["state"], "refused", "{witness}");
    assert!(
        witness["refused"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["function"].as_str().unwrap().starts_with("tests::")),
        "{witness}"
    );
    assert!(!run.join("jobs/gate-mutation").exists());
    let _ = fs::remove_dir_all(&root);
}

/// Breach probes in the tests gate, then a mutation gate on the same run (the positive control for
/// [`witness_symlink_probe`]).
fn breach_probes(home: &str, uid: u32, image_id: &str) {
    let (root, request) = fixture(home, uid, "probes", &[]);
    let run = root.join("run");
    let forged = run.join(FORGED);
    prepare(&request, &run);
    let tests = gate(&run, "tests", None);
    let job = run.join("jobs/gate-tests");
    let log = fs::read_to_string(job.join("stdout.log")).unwrap_or_default();
    let lines = log
        .lines()
        .filter(|l| l.starts_with("BREACH ") || l.starts_with("refused "))
        .collect::<Vec<_>>();
    for line in &lines {
        eprintln!("PROBE {line}");
    }
    assert_eq!(
        tests.status.code(),
        Some(0),
        "tests gate did not qualify, so the probes are unmeasured: {}\n{log}",
        String::from_utf8_lossy(&tests.stderr)
    );
    let receipts: serde_json::Value = serde_json::from_slice(&tests.stdout).unwrap();
    assert_eq!(receipts[0]["outcome"], "passed");
    // Launched by the resolved image ID, recorded in the receipt.
    assert_eq!(receipts[0]["image_id"], image_id);
    assert!(
        lines.iter().all(|l| !l.starts_with("BREACH ")),
        "{lines:#?}"
    );
    for probe in PROBES {
        assert_eq!(
            lines
                .iter()
                .filter(|l| l.starts_with(&format!("refused {probe}:")))
                .count(),
            1,
            "{probe} not reported exactly once as refused: {lines:#?}"
        );
    }
    assert_eq!(lines.len(), PROBES.len());
    assert!(!forged.exists(), "gate forged {}", forged.display());
    let environment = json(&job.join("environment.json"));
    assert_eq!(environment["network"], "none");
    assert_eq!(environment["host_home_mounted"], false);
    assert_eq!(environment["image_id"], image_id);
    assert_eq!(
        environment["read_only_mounts"],
        serde_json::json!(["/src", "/cargo-home", "/vendor"])
    );
    let name = environment["container_name"].as_str().unwrap();
    assert_eq!(
        runtime_output(&["container", "exists", name]).status.code(),
        Some(1)
    );
    // The seal was recorded and every gate mounted the sealed copy.
    let seal = json(&run.join("jobs/sandbox-vendor/vendor-seal.json"));
    assert_eq!(environment["vendor_sha256"], seal["sha256"]);
    let mutation = gate(&run, "mutation", None);
    assert_eq!(
        mutation.status.code(),
        Some(0),
        "positive control: an unplanted mutation gate must qualify: {}",
        String::from_utf8_lossy(&mutation.stderr)
    );
    assert!(json(&run.join("jobs/gate-mutation/qualification.json"))["witness_sha256"].is_string());
    let _ = fs::remove_dir_all(&root);
}

/// A witness reached through a container-written directory symlink is refused, not followed.
fn witness_symlink_probe(home: &str, uid: u32) {
    let (root, request) = fixture(home, uid, "witness", &[("plant-witness-symlink", "")]);
    let run = root.join("run");
    prepare(&request, &run);
    let mutation = gate(&run, "mutation", None);
    let job = run.join("jobs/gate-mutation");
    assert!(
        fs::symlink_metadata(job.join("work/mutation-results/mutants.out"))
            .unwrap()
            .file_type()
            .is_symlink(),
        "the plant did not happen, so the probe is unmeasured"
    );
    assert_eq!(
        mutation.status.code(),
        Some(7),
        "{}",
        String::from_utf8_lossy(&mutation.stderr)
    );
    let receipts: serde_json::Value = serde_json::from_slice(&mutation.stdout).unwrap();
    assert_eq!(receipts[0]["outcome"], "inconclusive");
    let refusal = fs::read_to_string(job.join("qualification-error.txt")).unwrap();
    assert!(refusal.contains("mutation witness refused"), "{refusal}");
    let _ = fs::remove_dir_all(&root);
}

/// A symlink planted in the vendoring output is refused; nothing is sealed or mounted.
fn vendor_symlink_probe(home: &str, uid: u32) {
    let (root, request) = fixture(home, uid, "vendor", &[]);
    let run = root.join("run");
    prepare(&request, &run);
    let wrapper = root.join("plant-vendor-symlink.py");
    fs::write(&wrapper, PLANT_VENDOR_SYMLINK).unwrap();
    let planting = format!("python3 {} {}", wrapper.display(), runtime().join(" "));
    let check = gate(&run, "check", Some(&planting));
    assert_eq!(check.status.code(), Some(4));
    assert!(
        String::from_utf8_lossy(&check.stderr).contains("dependency vendoring did not pass"),
        "{}",
        String::from_utf8_lossy(&check.stderr)
    );
    assert!(
        fs::symlink_metadata(run.join("jobs/sandbox-vendor/work/vendor/escape"))
            .unwrap()
            .file_type()
            .is_symlink(),
        "the plant did not happen, so the probe is unmeasured"
    );
    let refusal =
        fs::read_to_string(run.join("jobs/sandbox-vendor/qualification-error.txt")).unwrap();
    assert!(refusal.contains("symlink or special file"), "{refusal}");
    assert!(!run.join("vendor").exists());
    assert!(!run.join("jobs/gate-check").exists());
    let _ = fs::remove_dir_all(&root);
}

/// Candidate toolchain configuration is refused before any job (vendoring included) exists.
fn toolchain_config_probe(home: &str, uid: u32) {
    for planted in ["rust-toolchain.toml", ".cargo/config.toml"] {
        let (root, request) = fixture(home, uid, "toolchain", &[]);
        let repository = root.join("repo");
        fs::create_dir_all(repository.join(".cargo")).unwrap();
        fs::write(
            repository.join(planted),
            "[toolchain]\npath = \"/work/evil\"\n[build]\nrustc-wrapper = \"/src/evil\"\n",
        )
        .unwrap();
        git(&repository, &["add", planted]);
        git(
            &repository,
            &[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "-qm",
                "plant toolchain configuration",
            ],
        );
        let run = root.join("run");
        prepare(&request, &run);
        let refused = gate(&run, "check", None);
        assert_eq!(refused.status.code(), Some(4), "{planted}");
        assert!(
            String::from_utf8_lossy(&refused.stderr)
                .contains(&format!("candidate toolchain configuration {planted}")),
            "{}",
            String::from_utf8_lossy(&refused.stderr)
        );
        assert_eq!(fs::read_dir(run.join("jobs")).unwrap().count(), 0);
        let _ = fs::remove_dir_all(&root);
    }
}
