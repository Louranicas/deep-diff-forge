// Review council: Rootless Podman sandbox for quality gates; candidate code never runs with the controller's authority.
// Design/status: docs/REVIEW_COUNCIL.md#gate-sandbox
//! Gate commands execute inside a rootless Podman container: no network, read-only root, all capabilities
//! dropped, `no-new-privileges`, a keep-id user namespace and no host home mounts. Only the job's own work
//! directory is writable, so a gate cannot read agent credentials or write controller evidence. Podman's
//! client can exit while conmon keeps a container running, so the controller removes it by name on refusal.
use crate::{Result, ReviewFailure, require};
use std::{
    path::Path,
    process::{Command, Stdio},
};

/// Container-side root of a job's private work directory.
pub(crate) const WORK: &str = "/work";
const VENDOR: &str = "/vendor";

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
    /// Resolve the image to its local ID so each receipt names the exact environment that ran.
    pub(crate) fn image_id(&self) -> Result<String> {
        let output = self
            .command()
            .args(["image", "inspect", "--format", "{{.Id}}", &self.image])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()?;
        let id = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        require(
            output.status.success() && !id.is_empty(),
            "gate sandbox unavailable: image is not present locally (never pulled implicitly)",
        )?;
        Ok(id)
    }
    /// Remove a container that may outlive its killed client; failure means it already exited.
    pub(crate) fn remove(&self, name: &str) {
        let _ = self
            .command()
            .args(["rm", "--force", "--time", "0", name])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
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
                // Measured: `passthrough` drops both streams when they are redirected to files;
                // `none` still delivers them over the attach and keeps no second copy in logs.
                "--log-driver=none",
            ]
            .map(str::to_owned),
        );
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
        if let Some(vendor) = spec.vendor {
            argv.push(format!("--volume={}:{VENDOR}:ro,z", vendor.display()));
        }
        for (key, value) in env(spec) {
            argv.push(format!("--env={key}={value}"));
        }
        argv.push(format!("--workdir={WORK}/src"));
        argv.push(self.image.clone());
        argv.extend(spec.inner.iter().cloned());
        argv
    }
    pub(crate) fn record(&self, spec: &Spec<'_>, image_id: &str) -> serde_json::Value {
        serde_json::json!({
            "sandbox": "podman-rootless",
            "image": self.image,
            "image_id": image_id,
            "network": if spec.network { "default" } else { "none" },
            "root_filesystem": "read-only",
            "capabilities": "all dropped",
            "no_new_privileges": true,
            "userns": "keep-id",
            "host_home_mounted": false,
            "writable_mounts": [WORK],
            "read_only_mounts": spec.vendor.map(|_| VENDOR),
            "environment": env(spec).iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>(),
            "cargo_target_dir": cargo_target_dir(spec),
            "normal_target_inherited": false,
        })
    }
}

pub(crate) struct Spec<'a> {
    pub(crate) name: &'a str,
    pub(crate) work: &'a Path,
    pub(crate) vendor: Option<&'a Path>,
    pub(crate) inner: &'a [String],
    pub(crate) timeout_seconds: u64,
    pub(crate) network: bool,
    /// cargo-mutants copies the tree per mutant; a shared target dir would cross-contaminate (AP-32).
    pub(crate) mutation: bool,
}
fn cargo_target_dir(spec: &Spec<'_>) -> Option<String> {
    (!spec.mutation).then(|| format!("{WORK}/target"))
}
fn env(spec: &Spec<'_>) -> Vec<(&'static str, String)> {
    let mut env = vec![
        ("HOME", format!("{WORK}/home")),
        ("CARGO_HOME", format!("{WORK}/cargo-home")),
        ("TMPDIR", format!("{WORK}/tmp")),
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
/// Container name: unique per run and job, so a stale container can be found and removed.
pub(crate) fn container_name(run_id: &str, job: &str) -> String {
    format!("ddf-council-{}-{job}", &run_id[..run_id.len().min(16)])
}
