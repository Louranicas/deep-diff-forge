// DDF integration: `deploy doctor`, the operator's health check (production readiness, 2026-10-30).
// Design/status: docs/OPERATIONS_AND_DAEMON.md#deploy-doctor
// Module index: docs/ARCHITECTURE.md#crates-as-built-030
//! `deploy doctor`: check that this machine can run deep-diff-forge's daemon and Git inputs, and
//! say what to do when it cannot. Every check reads; none changes anything. The daemon checks
//! come from `deep_diff_forge_daemon::inspect`, which applies the daemon's own start rules, and
//! Git is asked for its version the way an acquisition runs it (bounded, same environment).
//!
//! Exit 0 when no check fails (warnings included), 6 when one does: every failing check is one a
//! `daemon start` or a daemon client would also fail on.

use deep_diff_forge_core::exit;
use deep_diff_forge_daemon::{
    DirState, Inspection, LockState, SocketLocation, SocketState, Trouble,
};
use serde_json::{Value, json};

/// How long `deploy doctor` lets Git take to report its version.
const GIT_LIMIT: std::time::Duration = std::time::Duration::from_secs(10);

/// A check's outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum State {
    Ok,
    /// Works, with a limit worth knowing (no runtime directory, a stale socket, no Git).
    Warn,
    /// A daemon start or client would fail here.
    Fail,
}

impl State {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Warn => "warn",
            Self::Fail => "fail",
        }
    }
}

/// One check: its name, outcome and what it found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Check {
    pub(crate) name: &'static str,
    pub(crate) state: State,
    pub(crate) detail: String,
}

fn check(name: &'static str, state: State, detail: impl Into<String>) -> Check {
    Check {
        name,
        state,
        detail: detail.into(),
    }
}

/// The bytes this system's `sun_path` has for a Unix socket's path and its NUL: 108 on Linux,
/// 104 on macOS, read from the type the system's socket calls take.
const SUN_PATH: usize = std::mem::size_of::<nix::libc::sockaddr_un>()
    - std::mem::offset_of!(nix::libc::sockaddr_un, sun_path);

/// Why `path` cannot be a socket's path on this system, or `None` when it can: a path longer
/// than `sun_path` holds with its NUL can be neither bound nor reached (S3.8.c). The one place
/// that decides it: `daemon path` and `daemon start` refuse such a path with these words, and
/// `deploy doctor` fails its `socket` check with them. The limit was Linux's 107 on every
/// system: on macOS a path of 104 to 107 bytes was printed and then could not be bound, and
/// the refusal said "at most 107" (the macOS CI run of 0.11.0, row 19 of its reading).
pub(crate) fn socket_path_refusal(path: &std::path::Path) -> Option<String> {
    refusal_within(path.as_os_str().len(), SUN_PATH)
}

/// [`socket_path_refusal`] where `sun_path` has `room` bytes.
fn refusal_within(length: usize, room: usize) -> Option<String> {
    let most = room.saturating_sub(1);
    (length > most)
        .then(|| format!("the socket path is {length} bytes; a Unix socket path is at most {most}"))
}

/// What the doctor looked at, gathered before any judgement so [`checks`] is pure.
pub(crate) struct Findings {
    pub(crate) version: &'static str,
    /// The socket location and what is there, or why no location could be resolved.
    pub(crate) daemon: Result<(SocketLocation, Inspection), String>,
    /// Git's version line, or why Git could not be run.
    pub(crate) git: Result<String, String>,
}

/// Judge the findings.
pub(crate) fn checks(findings: &Findings) -> Vec<Check> {
    let mut out = vec![check(
        "binary",
        State::Ok,
        format!("deep-diff-forge {}", findings.version),
    )];
    match &findings.daemon {
        Err(why) => out.push(check(
            "socket",
            State::Warn,
            format!("{why}: pass --socket PATH to every daemon command"),
        )),
        Ok((location, found)) => {
            out.push(match socket_path_refusal(location.path()) {
                Some(refusal) => check(
                    "socket",
                    State::Fail,
                    format!("{refusal}: daemon path and daemon start refuse it"),
                ),
                None => check("socket", State::Ok, location.path().display().to_string()),
            });
            out.push(dir_check(found.dir));
            out.push(daemon_check(&found.socket, findings.version));
            out.push(lock_check(found.lock));
        }
    }
    out.push(match &findings.git {
        Ok(line) => check("git", State::Ok, line.clone()),
        Err(why) => check(
            "git",
            State::Warn,
            format!("{why}: the --git-* inputs and review.change are unavailable"),
        ),
    });
    out
}

fn dir_check(dir: DirState) -> Check {
    match dir {
        DirState::Absent => check(
            "socket_dir",
            State::Ok,
            "absent; the first daemon start creates it owner-private",
        ),
        DirState::Private => check("socket_dir", State::Ok, "owner-private"),
        DirState::Refused(err) => check(
            "socket_dir",
            State::Fail,
            format!("{err}; a daemon start refuses it"),
        ),
    }
}

fn daemon_check(socket: &SocketState, version: &str) -> Check {
    match socket {
        SocketState::Absent => check("daemon", State::Ok, "not running"),
        SocketState::Live {
            pid,
            version: running,
            trouble,
        } => {
            let pid = pid.map_or_else(|| "unknown".to_owned(), |pid| pid.to_string());
            match running.as_deref() {
                Some(running) if running == version => match trouble {
                    Some(t) if t.any() => check(
                        "daemon",
                        State::Warn,
                        format!("running, pid {pid}; since it started: {}", trouble_text(*t)),
                    ),
                    _ => check("daemon", State::Ok, format!("running, pid {pid}")),
                },
                running => check(
                    "daemon",
                    State::Warn,
                    format!(
                        "running, pid {pid}, version {}, but this binary is {version}: restart it",
                        running.unwrap_or("unknown")
                    ),
                ),
            }
        }
        SocketState::Stale => check(
            "daemon",
            State::Warn,
            "not running; a stale socket is left, and the next start replaces it",
        ),
        SocketState::Foreign(why) => check(
            "daemon",
            State::Fail,
            format!("this user cannot use the daemon at the socket: {why}"),
        ),
        SocketState::NotASocket => check(
            "daemon",
            State::Fail,
            "a file that is not a socket is at the socket path; a start refuses to delete it",
        ),
        SocketState::Unreachable(why) => check("daemon", State::Fail, why.clone()),
    }
}

/// The nonzero counts of `trouble`, for a reader: `2 contained panics, 1 busy refusal`.
fn trouble_text(trouble: Trouble) -> String {
    let parts: Vec<String> = [
        (trouble.panics, "contained panic", "contained panics"),
        (trouble.busy, "busy refusal", "busy refusals"),
        (
            trouble.session_resets,
            "session table reset",
            "session table resets",
        ),
    ]
    .into_iter()
    .filter(|(n, _, _)| *n > 0)
    .map(|(n, one, many)| format!("{n} {}", if n == 1 { one } else { many }))
    .collect();
    format!(
        "{} (deep-diff-forge daemon metrics shows more)",
        parts.join(", ")
    )
}

fn lock_check(lock: LockState) -> Check {
    match lock {
        LockState::Absent => check(
            "start_lock",
            State::Ok,
            "absent; the first daemon start creates it",
        ),
        LockState::Usable => check("start_lock", State::Ok, "a regular file owned by this user"),
        LockState::Refused(why) => check(
            "start_lock",
            State::Fail,
            format!("{why}; a daemon start refuses it"),
        ),
    }
}

/// The exit code for these checks: 6 when one failed, else 0.
pub(crate) fn exit_code(checks: &[Check]) -> i32 {
    if checks.iter().any(|c| c.state == State::Fail) {
        exit::DAEMON_OR_TERMINAL
    } else {
        exit::SUCCESS
    }
}

/// The checks as a `deep-diff-forge.deploy-doctor.v0` document.
pub(crate) fn document(version: &str, checks: &[Check]) -> Value {
    json!({
        "schema": "deep-diff-forge.deploy-doctor.v0",
        "version": version,
        "healthy": exit_code(checks) == exit::SUCCESS,
        "checks": checks
            .iter()
            .map(|c| json!({"name": c.name, "state": c.state.as_str(), "detail": c.detail}))
            .collect::<Vec<_>>(),
    })
}

/// The checks as aligned text, one per line.
pub(crate) fn text(checks: &[Check]) -> String {
    use std::fmt::Write as _;
    let mut out = String::from("deep-diff-forge deploy doctor\n");
    for c in checks {
        let _ = writeln!(out, "  {:<4} {:<10} {}", c.state.as_str(), c.name, c.detail);
    }
    out
}

/// Look at this machine: the daemon location (`explicit`, or the default) and Git.
pub(crate) fn gather(explicit: Option<&std::path::Path>) -> Findings {
    let daemon = SocketLocation::resolve(explicit)
        .map(|location| {
            let found = deep_diff_forge_daemon::inspect(&location);
            (location, found)
        })
        .map_err(|err| err.to_string());
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("/"));
    Findings {
        version: env!("CARGO_PKG_VERSION"),
        daemon,
        git: deep_diff_forge_patch::git_version(&cwd, &git_options())
            .map_err(|err| err.to_string()),
    }
}

/// Git as an acquisition runs it, but given [`GIT_LIMIT`] to answer.
fn git_options() -> deep_diff_forge_patch::GitAcquireOptions {
    deep_diff_forge_patch::GitAcquireOptions {
        time_limit: GIT_LIMIT,
        ..deep_diff_forge_patch::GitAcquireOptions::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deep_diff_forge_daemon::SocketError;

    fn at(dir: DirState, socket: SocketState, lock: LockState) -> Findings {
        Findings {
            version: "9.9.9",
            daemon: Ok((
                SocketLocation::at("/run/user/1/deep-diff-forge/deep-diff-forge.sock"),
                Inspection { dir, socket, lock },
            )),
            git: Ok("git version 2.51.0".to_owned()),
        }
    }

    fn states(findings: &Findings) -> Vec<(&'static str, State)> {
        checks(findings).iter().map(|c| (c.name, c.state)).collect()
    }

    #[test]
    fn git_gets_ten_seconds_and_otherwise_runs_as_an_acquisition() {
        let options = git_options();
        assert_eq!(options.time_limit, std::time::Duration::from_secs(10));
        let default = deep_diff_forge_patch::GitAcquireOptions::default();
        assert_eq!(
            (options.git_program, options.env, options.byte_budget),
            (default.git_program, default.env, default.byte_budget)
        );
    }

    #[test]
    fn a_fresh_machine_is_healthy() {
        let found = at(DirState::Absent, SocketState::Absent, LockState::Absent);
        let all = checks(&found);
        assert_eq!(
            states(&found),
            [
                ("binary", State::Ok),
                ("socket", State::Ok),
                ("socket_dir", State::Ok),
                ("daemon", State::Ok),
                ("start_lock", State::Ok),
                ("git", State::Ok),
            ]
        );
        assert_eq!(exit_code(&all), exit::SUCCESS);
        assert_eq!(all[0].detail, "deep-diff-forge 9.9.9");
        assert_eq!(all[5].detail, "git version 2.51.0");
    }

    #[test]
    fn a_running_daemon_of_this_version_is_ok_and_another_version_warns() {
        let same = at(
            DirState::Private,
            SocketState::Live {
                pid: Some(42),
                version: Some("9.9.9".into()),
                trouble: Some(Trouble::default()),
            },
            LockState::Usable,
        );
        let daemon = &checks(&same)[3];
        assert_eq!(
            (daemon.state, daemon.detail.as_str()),
            (State::Ok, "running, pid 42")
        );
        let older = at(
            DirState::Private,
            SocketState::Live {
                pid: None,
                version: Some("0.3.0".into()),
                trouble: None,
            },
            LockState::Usable,
        );
        let daemon = &checks(&older)[3];
        assert_eq!(daemon.state, State::Warn);
        assert_eq!(
            daemon.detail,
            "running, pid unknown, version 0.3.0, but this binary is 9.9.9: restart it"
        );
        assert_eq!(exit_code(&checks(&older)), exit::SUCCESS);
    }

    #[test]
    fn a_daemon_with_trouble_warns_with_the_counts() {
        let live = |trouble| {
            at(
                DirState::Private,
                SocketState::Live {
                    pid: Some(7),
                    version: Some("9.9.9".into()),
                    trouble,
                },
                LockState::Usable,
            )
        };
        let warned = &checks(&live(Some(Trouble {
            panics: 2,
            busy: 1,
            session_resets: 0,
        })))[3];
        assert_eq!(warned.state, State::Warn);
        assert_eq!(
            warned.detail,
            "running, pid 7; since it started: 2 contained panics, 1 busy refusal (deep-diff-forge daemon metrics shows more)"
        );
        let one_reset = &checks(&live(Some(Trouble {
            panics: 0,
            busy: 0,
            session_resets: 1,
        })))[3];
        assert!(
            one_reset.detail.contains("1 session table reset ("),
            "{}",
            one_reset.detail
        );
        let resets = &checks(&live(Some(Trouble {
            panics: 1,
            busy: 3,
            session_resets: 2,
        })))[3];
        assert!(
            resets
                .detail
                .contains("1 contained panic, 3 busy refusals, 2 session table resets"),
            "{}",
            resets.detail
        );
        // No metrics (an older daemon) or clean metrics: ok.
        assert_eq!(checks(&live(None))[3].state, State::Ok);
        assert_eq!(checks(&live(Some(Trouble::default())))[3].state, State::Ok);
        assert_eq!(
            exit_code(&checks(&live(Some(Trouble {
                panics: 9,
                busy: 9,
                session_resets: 9
            })))),
            exit::SUCCESS
        );
    }

    #[test]
    fn each_thing_a_start_would_refuse_fails_with_exit_6() {
        for (dir, socket, lock, failing) in [
            (
                DirState::Refused(SocketError::TooPermissive),
                SocketState::Absent,
                LockState::Absent,
                "socket_dir",
            ),
            (
                DirState::Private,
                SocketState::Foreign("the peer runs as uid 7, not uid 1".into()),
                LockState::Usable,
                "daemon",
            ),
            (
                DirState::Private,
                SocketState::NotASocket,
                LockState::Absent,
                "daemon",
            ),
            (
                DirState::Private,
                SocketState::Unreachable("Connection reset by peer".into()),
                LockState::Absent,
                "daemon",
            ),
            (
                DirState::Private,
                SocketState::Absent,
                LockState::Refused("the start lock is a symlink"),
                "start_lock",
            ),
        ] {
            let all = checks(&at(dir, socket, lock));
            let failed: Vec<_> = all
                .iter()
                .filter(|c| c.state == State::Fail)
                .map(|c| c.name)
                .collect();
            assert_eq!(failed, [failing]);
            assert_eq!(exit_code(&all), exit::DAEMON_OR_TERMINAL);
        }
    }

    #[test]
    fn a_stale_socket_or_missing_git_only_warns() {
        let mut found = at(DirState::Private, SocketState::Stale, LockState::Usable);
        found.git = Err("could not run git version: No such file or directory".into());
        let all = checks(&found);
        assert_eq!(all[3].state, State::Warn);
        assert_eq!(all[5].state, State::Warn);
        assert!(
            all[5]
                .detail
                .ends_with("the --git-* inputs and review.change are unavailable")
        );
        assert_eq!(exit_code(&all), exit::SUCCESS);
    }

    #[test]
    fn no_runtime_directory_warns_and_skips_the_daemon_checks() {
        let found = Findings {
            version: "9.9.9",
            daemon: Err("no secure runtime directory".into()),
            git: Ok("git version 2.51.0".into()),
        };
        assert_eq!(
            states(&found),
            [
                ("binary", State::Ok),
                ("socket", State::Warn),
                ("git", State::Ok)
            ]
        );
        assert_eq!(exit_code(&checks(&found)), exit::SUCCESS);
    }

    #[test]
    fn the_document_names_its_schema_and_health() {
        let all = checks(&at(
            DirState::Private,
            SocketState::NotASocket,
            LockState::Absent,
        ));
        let doc = document("9.9.9", &all);
        assert_eq!(doc["schema"], "deep-diff-forge.deploy-doctor.v0");
        assert_eq!(doc["version"], "9.9.9");
        assert_eq!(doc["healthy"], false);
        assert_eq!(doc["checks"][3]["state"], "fail");
        assert_eq!(doc["checks"].as_array().unwrap().len(), all.len());
        let healthy = document(
            "9.9.9",
            &checks(&at(
                DirState::Absent,
                SocketState::Absent,
                LockState::Absent,
            )),
        );
        assert_eq!(healthy["healthy"], true);
        assert!(text(&all).starts_with(
            "deep-diff-forge deploy doctor\n  ok   binary     deep-diff-forge 9.9.9\n"
        ));
    }
    /// Package P3 of 0.11.1 (row 19 of the reading of the macOS CI run of 0.11.0): the longest
    /// socket path is the system's, `sun_path` less its NUL, and the refusal names that number.
    /// With 104 bytes of `sun_path` (macOS) a path of 104 bytes is refused "at most 103"; with
    /// 108 (Linux) one of 107 is taken and one of 108 refused. Here the number is this system's:
    /// 107 on Linux, 103 on macOS. A limit of 107 written for every system is red here on
    /// Linux too, by the case of 104 bytes.
    #[test]
    fn the_longest_socket_path_is_the_system_s() {
        assert_eq!(refusal_within(103, 104), None);
        assert_eq!(
            refusal_within(104, 104).as_deref(),
            Some("the socket path is 104 bytes; a Unix socket path is at most 103")
        );
        assert_eq!(refusal_within(107, 108), None);
        assert_eq!(
            refusal_within(108, 108).as_deref(),
            Some("the socket path is 108 bytes; a Unix socket path is at most 107")
        );
        let most = if cfg!(any(target_os = "linux", target_os = "android")) {
            107
        } else if cfg!(target_vendor = "apple") {
            103
        } else {
            SUN_PATH - 1
        };
        assert_eq!(SUN_PATH - 1, most);
        let path = |bytes: usize| std::path::PathBuf::from(format!("/{}", "s".repeat(bytes - 1)));
        assert_eq!(socket_path_refusal(&path(most)), None);
        let refusal = socket_path_refusal(&path(most + 1)).unwrap();
        assert!(refusal.ends_with(&format!("at most {most}")), "{refusal}");
        // the doctor fails its socket check with the same words, and exits 6
        let mut found = at(DirState::Absent, SocketState::Absent, LockState::Absent);
        if let Ok((location, _)) = &mut found.daemon {
            *location = SocketLocation::at(path(most + 1));
        }
        let all = checks(&found);
        assert_eq!(all[1].name, "socket");
        assert_eq!(all[1].state, State::Fail);
        assert_eq!(
            all[1].detail,
            format!("{refusal}: daemon path and daemon start refuse it")
        );
        assert_eq!(exit_code(&all), exit::DAEMON_OR_TERMINAL);
        if let Ok((location, _)) = &mut found.daemon {
            *location = SocketLocation::at(path(most));
        }
        assert_eq!(checks(&found)[1].state, State::Ok);
    }
}
