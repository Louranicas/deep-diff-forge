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

/// Run as [`run`] does, under `umask 077` (set by `sh`, which then execs the binary).
fn run_under_umask_077(args: &[&str]) -> (i32, String, String) {
    let out = Command::new("sh")
        .args(["-c", "umask 077 && exec \"$0\" \"$@\"", bin()])
        .args(args)
        .env_remove("XDG_RUNTIME_DIR")
        .output()
        .expect("run under umask 077");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// How many 10 ms steps a test waits for a daemon to start or to exit: 10 s. Generous, so a
/// loaded machine (a mutation run, a parallel build) is not read as a failure; a daemon that
/// never starts or never stops still fails the test. Before, 2 s.
const WAIT_STEPS: usize = 1000;

/// A spawned daemon, killed and reaped when the test ends, also when an assertion fails first. A
/// `Child` is not killed on drop: before, a failed assertion left its daemon serving, and one from
/// a mutation run was found still running 31 hours later (2026-10-30).
struct Daemon(std::process::Child);

impl Daemon {
    fn spawn(socket: &str) -> Self {
        Self(
            Command::new(bin())
                .args(["daemon", "start", "--foreground", "--socket", socket])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("spawn daemon"),
        )
    }

    /// Wait up to 10 s for the daemon to exit on its own.
    fn exited(&mut self) -> bool {
        for _ in 0..WAIT_STEPS {
            if let Ok(Some(_)) = self.0.try_wait() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
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
    let mut child = Daemon::spawn(sock.to_str().unwrap());

    // Wait for the socket to appear.
    let mut ready = false;
    for _ in 0..WAIT_STEPS {
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

    // The server exits shortly after shutdown.
    assert!(child.exited(), "the daemon kept running after daemon stop");
    drop(child);
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

/// Bug hunt 2026-11-01: a daemon at its connection limit closes a further connection unanswered,
/// and the CLI says so (exit 6) instead of saying there is no daemon; once a slot is free it is
/// served again. Before, `daemon health` and `daemon stop` said "no daemon at" while it ran.
#[test]
fn a_daemon_at_its_connection_limit_is_not_reported_absent() {
    use std::io::{BufRead as _, Write as _};
    let sock = temp_socket("at-limit");
    let _ = std::fs::remove_dir_all(sock.parent().unwrap());
    let path = sock.to_str().unwrap();
    let mut daemon = Daemon::spawn(path);
    let mut ready = false;
    for _ in 0..WAIT_STEPS {
        if run(&["daemon", "health", "--socket", path]).0 == 0 {
            ready = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(ready, "the daemon never answered");
    // Eight connections, each served once, so every worker slot is held.
    let mut holders = Vec::new();
    for _ in 0..8 {
        let mut held = std::os::unix::net::UnixStream::connect(&sock).unwrap();
        held.set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        writeln!(held, r#"{{"method":"daemon.health"}}"#).unwrap();
        let mut line = String::new();
        std::io::BufReader::new(held.try_clone().unwrap())
            .read_line(&mut line)
            .unwrap();
        assert!(line.contains("\"status\":\"ok\""), "{line}");
        holders.push(held);
    }
    for sub in ["health", "stop"] {
        let (code, stdout, stderr) = run(&["daemon", sub, "--socket", path]);
        assert_eq!((code, stdout.as_str()), (6, ""), "{sub}: {stderr}");
        assert!(
            stderr.contains("no answer from the daemon at")
                && stderr.contains("(it may be at its connection limit; retry)"),
            "{sub}: {stderr}"
        );
        assert!(!stderr.contains("no daemon at"), "{sub}: {stderr}");
    }
    assert!(daemon.0.try_wait().unwrap().is_none(), "the daemon stopped");
    drop(holders);
    let mut served = false;
    for _ in 0..WAIT_STEPS {
        if run(&["daemon", "health", "--socket", path]).0 == 0 {
            served = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        served,
        "the daemon did not serve again once the slots were free"
    );
    assert_eq!(run(&["daemon", "stop", "--socket", path]).0, 0);
    assert!(daemon.exited(), "the daemon kept running after daemon stop");
    drop(daemon);
    let _ = std::fs::remove_dir_all(sock.parent().unwrap());
}

/// A second `daemon start` on a running daemon's socket exits 6 and leaves that daemon serving.
#[test]
fn a_second_start_is_refused_and_the_first_keeps_serving() {
    let sock = temp_socket("second-start");
    let _ = std::fs::remove_dir_all(sock.parent().unwrap());
    let path = sock.to_str().unwrap();
    let mut first = Daemon::spawn(path);
    let mut ready = false;
    for _ in 0..WAIT_STEPS {
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
    assert!(first.exited(), "the daemon kept running after daemon stop");
    drop(first);
    let _ = std::fs::remove_dir_all(sock.parent().unwrap());
}

/// `daemon metrics` reads a live daemon's counters: the requests it served are there, by method.
#[test]
fn daemon_metrics_reports_what_the_daemon_served() {
    let sock = temp_socket("metrics");
    let _ = std::fs::remove_dir_all(sock.parent().unwrap());
    let path = sock.to_str().unwrap();
    let mut daemon = Daemon::spawn(path);
    let mut ready = false;
    for _ in 0..WAIT_STEPS {
        if run(&["daemon", "health", "--socket", path]).0 == 0 {
            ready = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(ready, "the daemon never answered");
    assert_eq!(run(&["daemon", "status", "--socket", path]).0, 0);
    let (code, stdout, stderr) = run(&["daemon", "metrics", "--socket", path]);
    assert_eq!(code, 0, "{stderr}");
    let reply: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let m = &reply["result"];
    assert_eq!(m["schema"], "deep-diff-forge.daemon-metrics.v0");
    assert!(m["methods"]["daemon.health"]["requests"].as_u64().unwrap() >= 1);
    assert_eq!(m["methods"]["daemon.status"]["requests"], 1);
    assert!(m["connections"]["accepted"].as_u64().unwrap() >= 3);
    // deploy doctor reads the same metrics: a clean daemon is ok.
    let (code, stdout, _) = run(&["deploy", "doctor", "--socket", path]);
    assert_eq!(code, 0, "{stdout}");
    assert!(stdout.contains("ok   daemon     running, pid "), "{stdout}");
    assert_eq!(run(&["daemon", "stop", "--socket", path]).0, 0);
    assert!(daemon.exited(), "the daemon kept running after daemon stop");
    drop(daemon);
    let _ = std::fs::remove_dir_all(sock.parent().unwrap());
}

#[test]
fn log_format_belongs_to_start_and_takes_text_or_json() {
    let (code, _, stderr) = run(&["daemon", "health", "--log-format", "json"]);
    assert_eq!(code, 2);
    assert!(
        stderr.contains("--log-format applies only to `daemon start`"),
        "{stderr}"
    );
    let (code, _, stderr) = run(&[
        "daemon",
        "start",
        "--log-format",
        "xml",
        "--socket",
        "/dev/null/d.sock",
    ]);
    assert_eq!(code, 2);
    assert!(
        stderr.contains("--log-format takes text or json"),
        "{stderr}"
    );
    for format in ["text", "json"] {
        let (code, _, _) = run(&[
            "daemon",
            "start",
            "--log-format",
            format,
            "--socket",
            "/dev/null/d.sock",
        ]);
        assert_eq!(
            code, 6,
            "{format}: admitted, then the unbindable socket ends the start"
        );
    }
}

/// `--log-format json`: a client that hangs up before reading its answer makes the daemon's write
/// fail, and the daemon logs that as one `connection_error` JSON object on stderr.
#[test]
fn json_logs_are_one_object_per_line() {
    use std::io::{Read as _, Write as _};
    let sock = temp_socket("jsonlog");
    let _ = std::fs::remove_dir_all(sock.parent().unwrap());
    std::fs::create_dir_all(sock.parent().unwrap()).unwrap();
    std::fs::set_permissions(
        sock.parent().unwrap(),
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .unwrap();
    let log = sock.with_file_name("stderr.log");
    let path = sock.to_str().unwrap();
    let child = Command::new(bin())
        .args(["daemon", "start", "--log-format", "json", "--socket", path])
        .stdout(Stdio::null())
        .stderr(std::fs::File::create(&log).unwrap())
        .spawn()
        .expect("spawn daemon");
    let mut daemon = Daemon(child);
    let mut ready = false;
    for _ in 0..WAIT_STEPS {
        if run(&["daemon", "health", "--socket", path]).0 == 0 {
            ready = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(ready, "the daemon never answered");
    // A large answer to a request whose client is already gone: the write fails. Retried, since
    // a fast daemon can answer before the client has closed. Each attempt waits up to 3 s for
    // the log line before the next is sent, so a slow daemon is not buried in more work; one
    // that drops the connection (at its connection limit) fails this client's own write, and
    // that attempt is skipped, not a failure (seen on macOS).
    let body = (0..20_000).fold(String::new(), |mut body, i| {
        use std::fmt::Write as _;
        let _ = writeln!(body, "+line {i}");
        body
    });
    let request = serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "diff.compute",
        "params": {"patch": format!("--- a/x\n+++ b/x\n@@ -0,0 +1,20000 @@\n{body}")},
    })
    .to_string();
    let mut text = String::new();
    'attempts: for _ in 0..10 {
        let mut client = std::os::unix::net::UnixStream::connect(path).unwrap();
        let sent = client
            .write_all(request.as_bytes())
            .and_then(|()| client.write_all(b"\n"));
        drop(client);
        if let Err(err) = sent {
            use std::io::ErrorKind::{BrokenPipe, ConnectionReset};
            assert!(matches!(err.kind(), BrokenPipe | ConnectionReset), "{err}");
        }
        for _ in 0..60 {
            std::thread::sleep(Duration::from_millis(50));
            text.clear();
            std::fs::File::open(&log)
                .unwrap()
                .read_to_string(&mut text)
                .unwrap();
            if text.contains("connection_error") {
                break 'attempts;
            }
        }
    }
    let line = text
        .lines()
        .find(|l| l.contains("connection_error"))
        .unwrap_or_else(|| panic!("no connection_error logged: {text}"));
    let event: serde_json::Value = serde_json::from_str(line).unwrap();
    assert_eq!(
        (event["event"].as_str(), event["level"].as_str()),
        (Some("connection_error"), Some("warn"))
    );
    assert!(
        event["message"]
            .as_str()
            .unwrap()
            .starts_with("connection error: ")
    );
    assert!(event["ts_ms"].as_u64().is_some());
    assert_eq!(run(&["daemon", "stop", "--socket", path]).0, 0);
    assert!(daemon.exited());
    drop(daemon);
    let _ = std::fs::remove_dir_all(sock.parent().unwrap());
}

/// `daemon stop` returns only once the daemon has exited: with a large request still being
/// answered, a `daemon start` right after `stop` binds the same socket instead of finding the
/// old daemon in the way (exit 6), and the request in progress is still answered in full.
#[test]
fn stop_returns_after_the_daemon_has_exited() {
    use std::io::{BufRead as _, BufReader, Write as _};
    let sock = temp_socket("stopwait");
    let dir = sock.parent().unwrap().to_path_buf();
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::set_permissions(&dir, std::os::unix::fs::PermissionsExt::from_mode(0o700)).unwrap();
    let path = sock.to_str().unwrap().to_owned();
    let mut first = Daemon::spawn(&path);
    let ready = (0..WAIT_STEPS).any(|_| {
        let up = run(&["daemon", "health", "--socket", &path]).0 == 0;
        if !up {
            std::thread::sleep(Duration::from_millis(10));
        }
        up
    });
    assert!(ready, "the daemon never answered");
    let body = (0..200_000).fold(String::new(), |mut body, i| {
        use std::fmt::Write as _;
        let _ = writeln!(body, "+line {i}");
        body
    });
    let request = serde_json::json!({
        "jsonrpc": "2.0", "id": 9, "method": "diff.compute",
        "params": {"patch": format!("--- a/x\n+++ b/x\n@@ -0,0 +1,200000 @@\n{body}")},
    })
    .to_string();
    let mut client = std::os::unix::net::UnixStream::connect(&sock).unwrap();
    client.write_all(request.as_bytes()).unwrap();
    client.write_all(b"\n").unwrap();
    let reader = std::thread::spawn({
        let client = client.try_clone().unwrap();
        move || {
            let mut answer = String::new();
            BufReader::new(client)
                .read_line(&mut answer)
                .map(|_| answer)
        }
    });
    // The large request is in progress once the daemon has admitted its bytes.
    let admitted = (0..WAIT_STEPS).any(|_| {
        let (_, metrics, _) = run(&["daemon", "metrics", "--socket", &path]);
        let metrics: serde_json::Value = serde_json::from_str(&metrics).unwrap_or_default();
        let big = metrics["result"]["admission"]["peak_in_flight_bytes"].as_u64() > Some(1 << 20);
        if !big {
            std::thread::sleep(Duration::from_millis(10));
        }
        big
    });
    let (stopped, _, stop_err) = run(&["daemon", "stop", "--socket", &path]);
    let mut second = Daemon::spawn(&path);
    let second_up = (0..WAIT_STEPS).any(|_| {
        if let Ok(Some(_)) = second.0.try_wait() {
            return false;
        }
        let up = run(&["daemon", "health", "--socket", &path]).0 == 0;
        if !up {
            std::thread::sleep(Duration::from_millis(10));
        }
        up
    });
    let second_alive = second.0.try_wait().is_ok_and(|status| status.is_none());
    let _ = run(&["daemon", "stop", "--socket", &path]);
    let answer = reader.join().unwrap().unwrap();
    assert!(admitted, "the large request was never admitted");
    assert_eq!(stopped, 0, "{stop_err}");
    assert!(first.exited(), "the first daemon had not exited");
    assert!(
        second_up && second_alive,
        "a start right after `stop` did not run: the old daemon was still in the way"
    );
    let answer: serde_json::Value = serde_json::from_str(&answer).unwrap();
    assert_eq!(answer["id"], 9);
    assert!(
        answer.get("result").is_some(),
        "the request in progress was not answered"
    );
    assert!(second.exited());
    let _ = std::fs::remove_dir_all(&dir);
}

/// `--format` belongs to `daemon metrics` and takes json or prometheus; `--out` needs the
/// Prometheus text. Refusals exit 2 before any daemon is asked.
#[test]
fn metrics_format_and_out_are_admitted_only_where_they_apply() {
    for (args, refusal) in [
        (
            &["daemon", "health", "--format", "prometheus"][..],
            "--format applies only to `daemon metrics`",
        ),
        (
            &["daemon", "metrics", "--format", "openmetrics"][..],
            "--format takes json or prometheus, not openmetrics",
        ),
        (
            &["daemon", "metrics", "--out", "x.prom"][..],
            "--out applies only to `daemon metrics --format prometheus`",
        ),
        (
            &["daemon", "metrics", "--format", "json", "--out", "x.prom"][..],
            "--out applies only to `daemon metrics --format prometheus`",
        ),
        (
            &["daemon", "status", "--out", "x.prom"][..],
            "--out applies only to `daemon metrics --format prometheus`",
        ),
    ] {
        let (code, stdout, stderr) = run(args);
        assert_eq!(code, 2, "{args:?}: {stderr}");
        assert_eq!(stdout, "", "{args:?}");
        assert!(stderr.contains(refusal), "{args:?}: {stderr}");
    }
    // No daemon: exit 6, and the textfile is left as it was.
    let sock = temp_socket("prom-absent");
    let dir = sock.parent().unwrap();
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir).unwrap();
    let out = dir.join("ddf.prom");
    std::fs::write(&out, "ddf_daemon_uptime_seconds 1\n").unwrap();
    let (code, stdout, stderr) = run(&[
        "daemon",
        "metrics",
        "--format",
        "prometheus",
        "--out",
        out.to_str().unwrap(),
        "--socket",
        sock.to_str().unwrap(),
    ]);
    assert_eq!((code, stdout.as_str()), (6, ""), "{stderr}");
    assert!(stderr.contains("no daemon at"), "{stderr}");
    assert_eq!(
        std::fs::read_to_string(&out).unwrap(),
        "ddf_daemon_uptime_seconds 1\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// `daemon metrics --format prometheus` renders the same counters as the JSON answer, on stdout
/// or, with `--out`, as a whole `0644` file and no temporary left beside it; a symlink at the
/// path is refused (exit 3) and its target kept.
#[test]
fn prometheus_metrics_match_the_json_and_write_a_textfile() {
    use std::os::unix::fs::PermissionsExt as _;
    let sock = temp_socket("prometheus");
    let dir = sock.parent().unwrap().to_path_buf();
    let _ = std::fs::remove_dir_all(&dir);
    let path = sock.to_str().unwrap();
    let mut daemon = Daemon::spawn(path);
    let mut ready = false;
    for _ in 0..WAIT_STEPS {
        if run(&["daemon", "health", "--socket", path]).0 == 0 {
            ready = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(ready, "the daemon never answered");
    assert_eq!(run(&["daemon", "status", "--socket", path]).0, 0);
    let (code, json, _) = run(&["daemon", "metrics", "--format", "json", "--socket", path]);
    assert_eq!(code, 0);
    let reply: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(
        reply["result"]["schema"],
        "deep-diff-forge.daemon-metrics.v0"
    );
    assert_eq!(reply["result"]["methods"]["daemon.status"]["requests"], 1);

    let (code, text, stderr) = run(&[
        "daemon",
        "metrics",
        "--format",
        "prometheus",
        "--socket",
        path,
    ]);
    assert_eq!(code, 0, "{stderr}");
    assert!(
        text.contains("\nddf_daemon_requests_total{method=\"daemon.status\"} 1\n"),
        "{text}"
    );
    // The JSON request above is counted too: one daemon.metrics before this one.
    assert!(
        text.contains("\nddf_daemon_requests_total{method=\"daemon.metrics\"} 1\n"),
        "{text}"
    );
    assert!(text.contains("# TYPE ddf_daemon_request_duration_seconds histogram\n"));
    assert!(text.contains(&format!(
        "ddf_daemon_build_info{{version=\"{}\"}} 1\n",
        env!("CARGO_PKG_VERSION")
    )));
    assert!(text.starts_with("# HELP ") && text.ends_with('\n') && !text.contains("# EOF"));

    // Under umask 077, as a cron or systemd job may run: the file is still 0644, which only the
    // explicit set_permissions gives (the inherited 022 alone turns the create mode into 0644).
    let out = dir.join("ddf.prom");
    let (code, stdout, stderr) = run_under_umask_077(&[
        "daemon",
        "metrics",
        "--format",
        "prometheus",
        "--out",
        out.to_str().unwrap(),
        "--socket",
        path,
    ]);
    assert_eq!((code, stdout.as_str()), (0, ""), "{stderr}");
    let written = std::fs::read_to_string(&out).unwrap();
    assert!(
        written.contains("\nddf_daemon_requests_total{method=\"daemon.metrics\"} 2\n"),
        "{written}"
    );
    assert_eq!(
        std::fs::metadata(&out).unwrap().permissions().mode() & 0o777,
        0o644
    );
    // Beside the daemon's own socket and lock, only the textfile: no temporary is left.
    let others: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| !name.starts_with("d.sock"))
        .collect();
    assert_eq!(others, ["ddf.prom"]);

    let target = dir.join("target");
    std::fs::write(&target, "keep\n").unwrap();
    let link = dir.join("link.prom");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let (code, _, stderr) = run(&[
        "daemon",
        "metrics",
        "--format",
        "prometheus",
        "--out",
        link.to_str().unwrap(),
        "--socket",
        path,
    ]);
    assert_eq!(code, 3, "{stderr}");
    assert!(
        stderr.contains("is a symlink; refusing to replace it"),
        "{stderr}"
    );
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "keep\n");

    assert_eq!(run(&["daemon", "stop", "--socket", path]).0, 0);
    assert!(daemon.exited(), "the daemon kept running after daemon stop");
    drop(daemon);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A daemon that answers no metrics document (an RPC error, as a protocol 1 daemon gives, or a
/// result of another shape) exits 6 with the reason, and no text is written.
#[test]
fn prometheus_metrics_refuse_an_answer_that_is_not_a_metrics_document() {
    use std::io::{BufRead as _, Write as _};
    for (answer, reason) in [
        (
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"method not found"}}"#,
            "the daemon refused daemon.metrics: method not found",
        ),
        (
            r#"{"jsonrpc":"2.0","id":1,"result":{"schema":"deep-diff-forge.daemon-status.v0"}}"#,
            "not a deep-diff-forge.daemon-metrics.v0 document",
        ),
    ] {
        let sock = temp_socket("prom-refused");
        let dir = sock.parent().unwrap();
        let _ = std::fs::remove_dir_all(dir);
        std::fs::create_dir_all(dir).unwrap();
        let listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        let fake = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            std::io::BufReader::new(&stream)
                .read_line(&mut line)
                .unwrap();
            assert!(line.contains("daemon.metrics"), "{line}");
            writeln!(&stream, "{answer}").unwrap();
        });
        let out = dir.join("ddf.prom");
        let (code, stdout, stderr) = run(&[
            "daemon",
            "metrics",
            "--format",
            "prometheus",
            "--out",
            out.to_str().unwrap(),
            "--socket",
            sock.to_str().unwrap(),
        ]);
        // Assert before joining: a CLI that never connects (one that refuses the options) must
        // fail this test, not leave it waiting on the fake's accept.
        assert_eq!((code, stdout.as_str()), (6, ""), "{stderr}");
        assert!(stderr.contains(reason), "{stderr}");
        fake.join().unwrap();
        assert!(!out.exists());
        let _ = std::fs::remove_dir_all(dir);
    }
}
