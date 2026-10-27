//! CLI contract tests for the `daemon` subcommand, including a full
//! start -> health -> stop lifecycle over a real Unix socket.

#![expect(
    clippy::disallowed_methods,
    reason = "integration test: drives real processes, the environment and wall-clock time"
)]

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_deep-diff-forge")
}

fn run(args: &[&str]) -> (i32, String, String) {
    run_with_runtime_dir(args, None)
}

/// Run with an explicit `$XDG_RUNTIME_DIR` (or none), never the caller's ambient value,
/// so results do not depend on the desktop session or sandbox the tests run in.
fn run_with_runtime_dir(
    args: &[&str],
    runtime_dir: Option<&std::path::Path>,
) -> (i32, String, String) {
    let mut command = Command::new(bin());
    command.args(args).env_remove("XDG_RUNTIME_DIR");
    if let Some(dir) = runtime_dir {
        command.env("XDG_RUNTIME_DIR", dir);
    }
    let out = command.output().expect("run");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn temp_socket(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("ddf-clid-{}-{name}/d.sock", std::process::id()))
}

#[test]
fn daemon_path_prints_socket() {
    let runtime = std::env::temp_dir().join(format!("ddf-clid-{}-runtime", std::process::id()));
    let (code, stdout, _) = run_with_runtime_dir(&["daemon", "path"], Some(&runtime));
    assert_eq!(code, 0);
    assert!(stdout.contains("deep-diff-forge.sock"));
    assert!(stdout.starts_with(runtime.to_str().unwrap()));
}

#[test]
fn daemon_path_without_runtime_dir_refuses_tmp_fallback() {
    let (code, stdout, stderr) = run(&["daemon", "path"]);
    assert_eq!(code, 2);
    assert_eq!(stdout, "");
    assert!(stderr.contains("refusing an insecure world-writable /tmp socket"));
}

#[test]
fn daemon_unknown_subcommand_reports_usage_without_runtime_dir() {
    let (code, _, stderr) = run(&["daemon", "frobnicate"]);
    assert_eq!(code, 2);
    assert!(stderr.contains("usage"));
    assert!(!stderr.contains("XDG_RUNTIME_DIR"));
}

#[test]
fn daemon_path_respects_socket_flag() {
    let (code, stdout, _) = run(&["daemon", "path", "--socket", "/tmp/custom.sock"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("/tmp/custom.sock"));
}

#[test]
fn daemon_health_without_daemon_exits_6() {
    let sock = temp_socket("nohealth");
    let _ = std::fs::remove_dir_all(sock.parent().unwrap());
    let (code, _, stderr) = run(&["daemon", "health", "--socket", sock.to_str().unwrap()]);
    assert_eq!(code, 6);
    assert!(stderr.contains("no daemon"));
}

#[test]
fn daemon_unknown_subcommand_exits_2() {
    let (code, _, stderr) = run(&["daemon", "frobnicate"]);
    assert_eq!(code, 2);
    assert!(stderr.contains("usage"));
}

#[test]
fn daemon_full_lifecycle_start_health_stop() {
    let sock = temp_socket("life");
    let _ = std::fs::remove_dir_all(sock.parent().unwrap());

    // Start the daemon as a child process (run_server blocks until shutdown).
    let mut child = Command::new(bin())
        .args([
            "daemon",
            "start",
            "--foreground",
            "--socket",
            sock.to_str().unwrap(),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn daemon");

    // Wait for the socket to appear.
    let mut ready = false;
    for _ in 0..200 {
        if sock.exists() {
            ready = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(ready, "daemon socket never appeared");

    let (hcode, hout, _) = run(&["daemon", "health", "--socket", sock.to_str().unwrap()]);
    assert_eq!(hcode, 0);
    assert!(hout.contains("\"status\":\"ok\""));

    let (scode, sout, _) = run(&["daemon", "stop", "--socket", sock.to_str().unwrap()]);
    assert_eq!(scode, 0);
    assert!(sout.contains("\"stopping\":true"));

    // The server should exit shortly after shutdown.
    for _ in 0..200 {
        if let Ok(Some(_)) = child.try_wait() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(sock.parent().unwrap());
}
