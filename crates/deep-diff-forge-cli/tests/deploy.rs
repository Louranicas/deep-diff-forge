//! CLI contract tests for `deploy status`, `deploy release` and `deploy doctor`.

#![expect(
    clippy::disallowed_methods,
    reason = "integration test: drives real processes, the environment and wall-clock time"
)]

use std::process::Command;

fn run(args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .args(args)
        .output()
        .expect("run binary");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn status_json_declares_schema_and_maturity() {
    let (code, stdout, _) = run(&["deploy", "status", "--json"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("\"schema\": \"deep-diff-forge.deployment-status.v0\""));
    assert!(stdout.contains("\"maturity\": \"L9\""));
    assert!(stdout.contains("\"repo\": \"deep-diff-forge\""));
}

#[test]
fn status_json_lists_gate_stack() {
    let (code, stdout, _) = run(&["deploy", "status", "--json"]);
    assert_eq!(code, 0);
    for gate in [
        "identity", "format", "compile", "lint", "test", "fixture", "contract",
    ] {
        assert!(stdout.contains(gate), "missing gate {gate}");
    }
    assert!(stdout.contains("\"state\": \"not-run\""));
    // Anchor E-06: the snapshot says it ran nothing, so `not-run` cannot be read as a verdict.
    assert!(stdout.contains("\"gates_run\": false"));
}

#[test]
fn status_human_shows_maturity_name() {
    let (code, stdout, _) = run(&["deploy", "status"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("L9 (Learning)"));
    assert!(stdout.contains("not run by this command; run them with: just ci"));
}

#[test]
fn status_json_includes_external_observers() {
    let (_, stdout, _) = run(&["deploy", "status", "--json"]);
    assert!(stdout.contains("\"external_observers\""));
    assert!(stdout.contains("\"habitat\": \"optional\""));
}

#[test]
fn release_json_declares_schema_and_version() {
    let (code, stdout, _) = run(&["deploy", "release", "--json"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("\"schema\": \"deep-diff-forge.release.v0\""));
    assert!(stdout.contains("\"version\": \"0.3.0\""));
}

#[test]
fn release_json_reports_crates_io_blocked() {
    let (code, stdout, _) = run(&["deploy", "release", "--json"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("crates.io"));
    assert!(stdout.contains("\"state\": \"blocked\""));
    assert!(stdout.contains("\"pending\": [\"github-release\", \"crates.io\"]"));
    assert!(stdout.contains("\"fully_published\": false"));
}

#[test]
fn release_human_lists_targets_and_pending() {
    let (code, stdout, _) = run(&["deploy", "release"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("github"));
    assert!(stdout.contains("gitlab"));
    assert!(stdout.contains("crates.io"));
    assert!(stdout.contains("pending: github-release, crates.io"));
}

#[test]
fn deploy_without_subcommand_exits_two() {
    let (code, _, stderr) = run(&["deploy"]);
    assert_eq!(code, 2);
    assert!(stderr.contains("usage"));
}

#[test]
fn deploy_unknown_subcommand_exits_two() {
    let (code, _, stderr) = run(&["deploy", "frobnicate"]);
    assert_eq!(code, 2);
    assert!(stderr.contains("usage"));
}

/// A private directory under the system temp dir, removed when the test ends.
struct Dir(std::path::PathBuf);

impl Dir {
    fn new(name: &str) -> Self {
        use std::os::unix::fs::PermissionsExt as _;
        let path = std::env::temp_dir().join(format!("ddf-doctor-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        Self(path)
    }

    fn socket(&self) -> String {
        self.0.join("d.sock").to_str().unwrap().to_owned()
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The state `deploy doctor --json` reports for one check.
fn doctor_state(socket: &str, name: &str) -> (i32, String) {
    let (code, stdout, stderr) = run(&["deploy", "doctor", "--json", "--socket", socket]);
    let doc: serde_json::Value =
        serde_json::from_str(&stdout).unwrap_or_else(|_| panic!("{stderr}"));
    assert_eq!(doc["schema"], "deep-diff-forge.deploy-doctor.v0");
    assert_eq!(doc["healthy"], code == 0);
    let state = doc["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == name)
        .unwrap_or_else(|| panic!("no {name} check: {stdout}"))["state"]
        .as_str()
        .unwrap()
        .to_owned();
    (code, state)
}

#[test]
fn deploy_doctor_sees_a_running_daemon() {
    let dir = Dir::new("live");
    let socket = dir.socket();
    let mut daemon = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .args(["daemon", "start", "--foreground", "--socket", &socket])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn daemon");
    let mut ready = false;
    for _ in 0..200 {
        if run(&["daemon", "health", "--socket", &socket]).0 == 0 {
            ready = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let found = ready.then(|| run(&["deploy", "doctor", "--socket", &socket]));
    let _ = daemon.kill();
    let _ = daemon.wait();
    let (code, stdout, _) = found.expect("the daemon never answered");
    assert_eq!(code, 0, "{stdout}");
    assert!(stdout.contains("ok   daemon     running, pid "), "{stdout}");
    assert!(
        stdout.contains("ok   start_lock a regular file owned by this user"),
        "{stdout}"
    );
}

#[test]
fn deploy_doctor_fails_with_exit_6_on_what_a_start_refuses() {
    use std::os::unix::fs::PermissionsExt as _;
    let shared = Dir::new("shared");
    std::fs::set_permissions(&shared.0, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(
        doctor_state(&shared.socket(), "socket_dir"),
        (6, "fail".into())
    );

    let occupied = Dir::new("occupied");
    std::fs::write(occupied.socket(), b"not a socket").unwrap();
    assert_eq!(
        doctor_state(&occupied.socket(), "daemon"),
        (6, "fail".into())
    );

    let stale = Dir::new("stale");
    drop(std::os::unix::net::UnixListener::bind(stale.socket()).unwrap());
    assert_eq!(doctor_state(&stale.socket(), "daemon"), (0, "warn".into()));
}

#[test]
fn only_deploy_doctor_takes_a_socket() {
    for sub in ["status", "release"] {
        let (code, _, stderr) = run(&["deploy", sub, "--socket", "/x/d.sock"]);
        assert_eq!(code, 2, "{sub}");
        assert!(stderr.contains("--socket is accepted only with deploy doctor"));
    }
}
