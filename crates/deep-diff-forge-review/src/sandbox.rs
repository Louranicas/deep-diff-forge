// Review council: Rootless Podman sandbox for quality gates; candidate code never runs with the controller's authority.
// Design/status: docs/REVIEW_COUNCIL.md#gate-sandbox
//! Gate commands execute inside a rootless Podman container: no network, read-only root, all capabilities
//! dropped, `no-new-privileges`, a keep-id user namespace and no host home mounts. The candidate source,
//! the vendored dependencies and the Cargo home are mounted read-only; only the job's own scratch
//! directory is writable, so a gate cannot read agent credentials, write controller evidence or change
//! the bytes it is measured on. Podman's client can exit while conmon keeps a container running, so the
//! controller removes it by name and proves it gone before reading anything the container could write.
use crate::{Result, ReviewFailure, require};
use nix::{
    sys::signal::{Signal, killpg},
    unistd::Pid,
};
use std::io::Read as _;
use std::os::unix::process::CommandExt as _;
use std::{
    path::Path,
    process::{Command, ExitStatus, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

/// Container-side root of a job's private, writable scratch directory.
pub(crate) const WORK: &str = "/work";
/// Container-side read-only mount of the job's copy of the sealed snapshot; gates run here.
const SOURCE: &str = "/src";
const VENDOR: &str = "/vendor";
/// Container-side read-only mount of the job's controller-written Cargo home.
pub(crate) const CARGO_HOME: &str = "/cargo-home";
/// The fuzz gate's tmpfs, relative to the source root. cargo-fuzz creates `fuzz/artifacts/<target>`
/// even when `-artifact_prefix` points elsewhere. Admission refuses any sealed path under it.
pub(crate) const FUZZ_ARTIFACTS: &str = "fuzz/artifacts";
/// The image's stable toolchain (review-policy/sandbox/Containerfile). Set for every job so a
/// `rust-toolchain(.toml)` file can never select a toolchain; an explicit `+nightly` still wins.
const STABLE_TOOLCHAIN: &str = "1.99.1";
/// Bound on each runtime control command (`image inspect`, `rm`, `container exists`).
const CONTROL_DEADLINE: Duration = Duration::from_secs(30);

pub(crate) struct Sandbox {
    runtime: Vec<String>,
    image: String,
}
impl Sandbox {
    /// The operator selects the runtime command and gate image; neither comes from a candidate or model.
    pub(crate) fn from_env() -> Result<Self> {
        let image = std::env::var("DDF_COUNCIL_GATE_IMAGE").map_err(|_| {
            ReviewFailure("gate sandbox unavailable: DDF_COUNCIL_GATE_IMAGE is not set".into())
        })?;
        let runtime = std::env::var("DDF_COUNCIL_PODMAN")
            .unwrap_or_else(|_| "podman".into())
            .split_whitespace()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        require(
            !runtime.is_empty() && !image.is_empty(),
            "gate sandbox unavailable: empty runtime or image",
        )?;
        Ok(Self { runtime, image })
    }
    fn command(&self) -> Command {
        let mut command = Command::new(&self.runtime[0]);
        command.args(&self.runtime[1..]);
        command
    }
    /// Run one runtime control command under [`CONTROL_DEADLINE`]; `None` means it did not finish.
    fn control(&self, args: &[&str]) -> Result<Option<(ExitStatus, Vec<u8>)>> {
        let mut child = self
            .command()
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| ReviewFailure("missing runtime output stream".into()))?;
        // A detached reader, so a descendant that keeps the pipe open cannot outlast the deadline.
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = stdout.take(64 * 1024).read_to_end(&mut bytes);
            let _ = sender.send(bytes);
        });
        let start = Instant::now();
        loop {
            if let Some(status) = child.try_wait()? {
                // A descendant may still hold the pipe; the group outlives its reaped leader.
                if let Ok(group) = i32::try_from(child.id()) {
                    let _ = killpg(Pid::from_raw(group), Signal::SIGKILL);
                }
                let remaining = CONTROL_DEADLINE.saturating_sub(start.elapsed());
                return Ok(receiver
                    .recv_timeout(remaining)
                    .ok()
                    .map(|bytes| (status, bytes)));
            }
            if start.elapsed() > CONTROL_DEADLINE {
                if let Ok(group) = i32::try_from(child.id()) {
                    let _ = killpg(Pid::from_raw(group), Signal::SIGKILL);
                }
                let _ = child.kill();
                let _ = child.wait();
                return Ok(None);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    /// Resolve the image to its local ID. Jobs launch that ID, never the mutable tag, so a re-pointed
    /// tag cannot change what runs after admission, and each receipt names the exact environment.
    pub(crate) fn image_id(&self) -> Result<String> {
        let output = self.control(&["image", "inspect", "--format", "{{.Id}}", &self.image])?;
        let id = output
            .filter(|(status, _)| status.success())
            .map(|(_, stdout)| String::from_utf8_lossy(&stdout).trim().to_owned())
            .unwrap_or_default();
        let id = id.strip_prefix("sha256:").unwrap_or(&id);
        require(
            id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit()),
            "gate sandbox unavailable: image is not present locally (never pulled implicitly) or did not resolve to an image ID in time",
        )?;
        Ok(id.to_owned())
    }
    /// Remove a container that may outlive its killed client, then prove it is gone: `container exists`
    /// must answer 1 (no such container). Any other answer, or no answer in time, is a refusal.
    pub(crate) fn remove(&self, name: &str) -> Result<()> {
        // `--rm` has usually removed it already; this step's own status is not evidence either way.
        let _ = self.control(&["rm", "--force", "--time", "0", name])?;
        let exists = self.control(&["container", "exists", name])?;
        require(
            exists.is_some_and(|(status, _)| status.code() == Some(1)),
            "gate container was not provably removed",
        )
    }
    /// Wrap a fixed gate argv. `network` is only true for the controller's dependency vendoring step.
    pub(crate) fn argv(&self, spec: &Spec<'_>) -> Vec<String> {
        let mut argv = self.runtime.clone();
        argv.extend(
            [
                "run",
                "--rm",
                "--pull=never",
                "--read-only",
                "--cap-drop=all",
                "--security-opt=no-new-privileges",
                "--userns=keep-id",
                "--init",
                "--pids-limit=8192",
                "--memory=32g",
                // Equal to --memory, so the container gets no swap on top of its memory bound.
                "--memory-swap=32g",
                // Measured: `passthrough` drops both streams when they are redirected to files;
                // `none` still delivers them over the attach and keeps no second copy in logs.
                "--log-driver=none",
            ]
            .map(str::to_owned),
        );
        argv.push(format!("--cpus={}", cpus()));
        argv.push(format!("--name={}", spec.name));
        // conmon's own deadline backs up the controller's; the controller still removes by name.
        argv.push(format!(
            "--timeout={}",
            spec.timeout_seconds.saturating_add(60)
        ));
        if !spec.network {
            argv.push("--network=none".into());
        }
        argv.push(format!("--volume={}:{WORK}:Z", spec.work.display()));
        argv.push(format!("--volume={}:{SOURCE}:ro,Z", spec.source.display()));
        if let Some(home) = spec.cargo_home {
            argv.push(format!("--volume={}:{CARGO_HOME}:ro,Z", home.display()));
        }
        if let Some((vendor, _)) = spec.vendor {
            argv.push(format!("--volume={}:{VENDOR}:ro,z", vendor.display()));
        }
        if spec.fuzz {
            // The controller pre-creates the mountpoint in its source copy; `U` makes it the job's.
            argv.push(format!(
                "--tmpfs={SOURCE}/{FUZZ_ARTIFACTS}:rw,nosuid,nodev,noexec,size=64m,U"
            ));
        }
        for (key, value) in env(spec) {
            argv.push(format!("--env={key}={value}"));
        }
        argv.push(format!("--workdir={SOURCE}"));
        argv.push(spec.image_id.to_owned());
        argv.extend(spec.inner.iter().cloned());
        argv
    }
    pub(crate) fn record(&self, spec: &Spec<'_>) -> serde_json::Value {
        let mut read_only = vec![SOURCE];
        read_only.extend(spec.cargo_home.map(|_| CARGO_HOME));
        read_only.extend(spec.vendor.map(|_| VENDOR));
        serde_json::json!({
            "sandbox": "podman-rootless",
            "image": self.image,
            "image_id": spec.image_id,
            "container_name": spec.name,
            "network": if spec.network { "default" } else { "none" },
            "root_filesystem": "read-only",
            "capabilities": "all dropped",
            "no_new_privileges": true,
            "userns": "keep-id",
            "host_home_mounted": false,
            "writable_mounts": [WORK],
            "read_only_mounts": read_only,
            "tmpfs_mounts": if spec.fuzz { vec![format!("{SOURCE}/{FUZZ_ARTIFACTS}")] } else { Vec::new() },
            "vendor_sha256": spec.vendor.map(|(_, digest)| digest),
            "memory": "32g",
            "memory_swap": "32g",
            "cpus": cpus(),
            "environment": env(spec).iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>(),
            "cargo_target_dir": cargo_target_dir(spec),
            "normal_target_inherited": false,
        })
    }
}

pub(crate) struct Spec<'a> {
    pub(crate) name: &'a str,
    pub(crate) image_id: &'a str,
    /// Writable scratch, mounted at `/work`.
    pub(crate) work: &'a Path,
    /// The job's copy of the sealed snapshot, mounted read-only at `/src`.
    pub(crate) source: &'a Path,
    /// Controller-written Cargo home holding only the vendored-source config, mounted read-only.
    pub(crate) cargo_home: Option<&'a Path>,
    /// The run's sealed vendor directory and its recorded digest.
    pub(crate) vendor: Option<(&'a Path, &'a str)>,
    pub(crate) inner: &'a [String],
    pub(crate) timeout_seconds: u64,
    pub(crate) network: bool,
    /// cargo-mutants copies the tree per mutant; a shared target dir would cross-contaminate (AP-32).
    pub(crate) mutation: bool,
    pub(crate) fuzz: bool,
}
/// All host CPUs but one, so a gate cannot starve the controller that supervises it.
fn cpus() -> usize {
    std::thread::available_parallelism()
        .map_or(1, std::num::NonZero::get)
        .saturating_sub(1)
        .max(1)
}
fn cargo_target_dir(spec: &Spec<'_>) -> Option<String> {
    (!spec.mutation).then(|| format!("{WORK}/target"))
}
fn env(spec: &Spec<'_>) -> Vec<(&'static str, String)> {
    let cargo_home = match spec.cargo_home {
        Some(_) => CARGO_HOME.to_owned(),
        // Only the vendoring job (which downloads) and the antipatterns gate (no Cargo) use scratch.
        None => format!("{WORK}/cargo-home"),
    };
    let mut env = vec![
        ("HOME", format!("{WORK}/home")),
        ("CARGO_HOME", cargo_home),
        ("TMPDIR", format!("{WORK}/tmp")),
        ("RUSTUP_TOOLCHAIN", STABLE_TOOLCHAIN.into()),
    ];
    if !spec.network {
        env.push(("CARGO_NET_OFFLINE", "true".into()));
    }
    if let Some(target) = cargo_target_dir(spec) {
        env.push(("CARGO_TARGET_DIR", target));
    }
    env
}
/// Cargo configuration that resolves every crate from the run's read-only vendor directory.
pub(crate) const VENDORED_CONFIG: &str = "[source.crates-io]\nreplace-with = \"vendored-sources\"\n\n[source.vendored-sources]\ndirectory = \"/vendor\"\n";
/// Container name: run, job and a per-invocation nonce, so a container left by another invocation
/// with the same run ID is never confused with, or removed as, this one.
pub(crate) fn container_name(run_id: &str, job: &str) -> Result<String> {
    let mut nonce = [0_u8; 8];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut nonce)?;
    let nonce = u64::from_ne_bytes(nonce);
    Ok(format!(
        "ddf-council-{}-{job}-{nonce:016x}",
        &run_id[..run_id.len().min(16)]
    ))
}
