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

/// The subcommand is the positional, wherever `--socket PATH` stands: before, `daemon --socket
/// PATH health` took PATH as the subcommand and failed with a usage error.
#[test]
fn daemon_subcommand_follows_or_precedes_the_socket_option() {
    let sock = temp_socket("order");
    let _ = std::fs::remove_dir_all(sock.parent().unwrap());
    let path = sock.to_str().unwrap();
    let (code, stdout, stderr) = run(&["daemon", "--socket", path, "path"]);
    assert_eq!((code, stdout.trim()), (0, path), "{stderr}");
    let (code, _, stderr) = run(&["daemon", "--socket", path, "health"]);
    assert_eq!(code, 6, "no daemon, not a usage error: {stderr}");
}

/// `start` always serves in the foreground; the flag that says so is refused anywhere else.
#[test]
fn foreground_belongs_to_start_alone() {
    for sub in ["path", "health", "status", "stop"] {
        let (code, stdout, stderr) =
            run(&["daemon", sub, "--foreground", "--socket", "/tmp/x.sock"]);
        assert_eq!((code, stdout.as_str()), (2, ""), "{sub}: {stderr}");
        assert!(stderr.contains("--foreground applies only to"), "{stderr}");
    }
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

/// A socket this user may not connect to is reported as unusable, not as absent. A mode-000
/// socket refuses the connect with EACCES (root bypasses modes); a daemon of another user is
/// refused the same way.
#[test]
fn a_daemon_this_user_may_not_use_is_not_reported_absent() {
    let sock = temp_socket("unusable");
    let _ = std::fs::remove_dir_all(sock.parent().unwrap());
    std::fs::create_dir_all(sock.parent().unwrap()).unwrap();
    if std::os::unix::fs::MetadataExt::uid(&std::fs::metadata(sock.parent().unwrap()).unwrap()) == 0
    {
        return;
    }
    let listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();
    std::fs::set_permissions(&sock, std::os::unix::fs::PermissionsExt::from_mode(0o000)).unwrap();
    let (code, _, stderr) = run(&["daemon", "health", "--socket", sock.to_str().unwrap()]);
    assert_eq!(code, 6);
    assert!(stderr.contains("cannot use the daemon at"), "{stderr}");
    drop(listener);
    let _ = std::fs::remove_dir_all(sock.parent().unwrap());
}

/// A second `daemon start` on a running daemon's socket exits 6 and leaves that daemon serving.
#[test]
fn a_second_start_is_refused_and_the_first_keeps_serving() {
    let sock = temp_socket("second-start");
    let _ = std::fs::remove_dir_all(sock.parent().unwrap());
    let path = sock.to_str().unwrap();
    let mut first = Command::new(bin())
        .args(["daemon", "start", "--foreground", "--socket", path])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn daemon");
    let mut ready = false;
    for _ in 0..200 {
        if run(&["daemon", "health", "--socket", path]).0 == 0 {
            ready = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(ready, "the first daemon never answered");

    let (code, _, stderr) = run(&["daemon", "start", "--foreground", "--socket", path]);
    assert_eq!(code, 6, "{stderr}");
    assert!(
        stderr.contains("another daemon is running or starting on this socket"),
        "{stderr}"
    );
    let (code, stdout, _) = run(&["daemon", "health", "--socket", path]);
    assert_eq!(code, 0);
    assert!(stdout.contains("\"status\":\"ok\""));

    assert_eq!(run(&["daemon", "stop", "--socket", path]).0, 0);
    for _ in 0..200 {
        if let Ok(Some(_)) = first.try_wait() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let _ = first.kill();
    let _ = first.wait();
    let _ = std::fs::remove_dir_all(sock.parent().unwrap());
}
