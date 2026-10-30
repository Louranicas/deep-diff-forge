// DDF integration: read-only inspection of a daemon location for `deploy doctor` (2026-10-30).
// Design/status: docs/OPERATIONS_AND_DAEMON.md#deploy-doctor
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fcrates--deep-diff-forge-daemon--src--inspect
//! What is at a daemon location, read without changing anything: the socket, its start lock and
//! the directory they live in. Each answer applies the same rule the daemon applies when it
//! starts (`serve.rs`), so a refusal here is a start or a connect that would be refused.
//!
//! Inspection never takes the start lock: holding it, even briefly, would make a concurrent
//! `daemon start` fail.

use crate::security::{SocketError, validate_private_dir};
use crate::serve::{SocketLocation, request};
use serde_json::Value;
use std::os::unix::fs::{FileTypeExt as _, MetadataExt as _};
use std::path::Path;

/// What is at the socket path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SocketState {
    /// Nothing: no daemon runs here, and a start binds a fresh socket.
    Absent,
    /// A daemon of this user answered `daemon.health`.
    Live {
        /// Its process id, as it reports it.
        pid: Option<u64>,
        /// Its version, as it reports it.
        version: Option<String>,
    },
    /// A socket nothing answers on, left by a daemon that is gone; the next start replaces it.
    Stale,
    /// A daemon answered, but this user may not use it (it runs as another user, or the socket's
    /// mode refuses the connect).
    Foreign(String),
    /// Something other than a socket is at the path; a start refuses to delete it.
    NotASocket,
    /// The path or the daemon could not be read.
    Unreachable(String),
}

/// The start lock beside the socket (`<socket>.lock`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockState {
    /// Not created yet; the first start creates it.
    Absent,
    /// A regular file owned by this user: a start can take it.
    Usable,
    /// A start would refuse it, for the reason given.
    Refused(&'static str),
}

/// The directory that holds the socket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirState {
    /// Not created yet; a start creates it owner-private.
    Absent,
    /// Owner-private, owned by this user, not a symlink.
    Private,
    /// A start would refuse it.
    Refused(SocketError),
}

/// Everything [`inspect`] found at one location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inspection {
    pub dir: DirState,
    pub socket: SocketState,
    pub lock: LockState,
}

/// Inspect a daemon location without changing it.
#[must_use]
pub fn inspect(location: &SocketLocation) -> Inspection {
    let path = location.path();
    Inspection {
        dir: path.parent().map_or(DirState::Absent, dir_state),
        socket: socket_state(location),
        lock: location.start_lock_path().map_or(
            LockState::Refused("the socket path has no file name"),
            |lock| lock_state(&lock),
        ),
    }
}

fn dir_state(dir: &Path) -> DirState {
    match std::fs::symlink_metadata(dir) {
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => DirState::Absent,
        _ => match validate_private_dir(dir) {
            Ok(()) => DirState::Private,
            Err(err) => DirState::Refused(err),
        },
    }
}

fn socket_state(location: &SocketLocation) -> SocketState {
    match std::fs::symlink_metadata(location.path()) {
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return SocketState::Absent,
        Err(err) => return SocketState::Unreachable(err.to_string()),
        Ok(metadata) if !metadata.file_type().is_socket() => return SocketState::NotASocket,
        Ok(_) => {}
    }
    match request(
        location,
        r#"{"jsonrpc":"2.0","id":1,"method":"daemon.health"}"#,
    ) {
        Ok(line) => {
            let result = serde_json::from_str::<Value>(&line)
                .ok()
                .and_then(|reply| reply.get("result").cloned());
            match result {
                Some(health) => SocketState::Live {
                    pid: health.get("pid").and_then(Value::as_u64),
                    version: health
                        .get("version")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                },
                None => SocketState::Unreachable(format!(
                    "the daemon answered daemon.health without a result: {line}"
                )),
            }
        }
        Err(err) if err.kind() == std::io::ErrorKind::ConnectionRefused => SocketState::Stale,
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => {
            SocketState::Foreign(err.to_string())
        }
        Err(err) => SocketState::Unreachable(err.to_string()),
    }
}

/// The rule `take_start_lock` applies: opened without following a symlink, a regular file,
/// owned by the effective user.
fn lock_state(lock: &Path) -> LockState {
    match std::fs::symlink_metadata(lock) {
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => LockState::Absent,
        Err(_) => LockState::Refused("the start lock cannot be read"),
        Ok(metadata) if metadata.file_type().is_symlink() => {
            LockState::Refused("the start lock is a symlink")
        }
        Ok(metadata) if !metadata.is_file() => {
            LockState::Refused("the start lock is not a regular file")
        }
        Ok(metadata) if metadata.uid() != rustix::process::geteuid().as_raw() => {
            LockState::Refused("the start lock is owned by another user")
        }
        Ok(_) => LockState::Usable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::serve::run_server;
    use std::os::unix::fs::PermissionsExt as _;
    use std::path::PathBuf;

    struct Dir(PathBuf);

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A daemon serving `location` in this process until `daemon.shutdown`.
    fn serve(location: &SocketLocation) -> std::thread::JoinHandle<std::io::Result<()>> {
        let at = SocketLocation::at(location.path());
        let server = std::thread::spawn(move || run_server(&at));
        for _ in 0..500 {
            if request(location, r#"{"id":1,"method":"daemon.health"}"#).is_ok() {
                return server;
            }
            std::thread::yield_now();
            std::thread::park_timeout(std::time::Duration::from_millis(10));
        }
        panic!("the test daemon never answered");
    }

    fn private_dir(name: &str) -> Dir {
        let dir = std::env::temp_dir().join(format!("ddf-inspect-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        Dir(dir)
    }

    #[test]
    fn a_fresh_location_has_nothing_yet() {
        let dir = private_dir("fresh");
        let location = SocketLocation::at(dir.0.join("sub").join("d.sock"));
        assert_eq!(
            inspect(&location),
            Inspection {
                dir: DirState::Absent,
                socket: SocketState::Absent,
                lock: LockState::Absent,
            }
        );
    }

    #[test]
    fn a_running_daemon_is_live_and_its_lock_usable() {
        let dir = private_dir("live");
        let location = SocketLocation::at(dir.0.join("d.sock"));
        let server = serve(&location);
        let found = inspect(&location);
        assert_eq!(found.dir, DirState::Private);
        assert_eq!(found.lock, LockState::Usable);
        assert_eq!(
            found.socket,
            SocketState::Live {
                pid: Some(u64::from(std::process::id())),
                version: Some(env!("CARGO_PKG_VERSION").to_owned()),
            }
        );
        request(&location, r#"{"id":2,"method":"daemon.shutdown"}"#).unwrap();
        server.join().unwrap().unwrap();
    }

    #[test]
    fn a_socket_nothing_answers_on_is_stale() {
        let dir = private_dir("stale");
        let path = dir.0.join("d.sock");
        drop(std::os::unix::net::UnixListener::bind(&path).unwrap());
        assert_eq!(socket_state(&SocketLocation::at(&path)), SocketState::Stale);
    }

    #[test]
    fn a_file_at_the_socket_path_is_not_a_socket() {
        let dir = private_dir("file");
        let path = dir.0.join("d.sock");
        std::fs::write(&path, b"").unwrap();
        assert_eq!(
            socket_state(&SocketLocation::at(&path)),
            SocketState::NotASocket
        );
    }

    #[test]
    fn a_socket_this_user_may_not_connect_to_is_foreign() {
        let dir = private_dir("foreign");
        let path = dir.0.join("d.sock");
        let _listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
        // Root bypasses the mode, so the check only means something for an ordinary user.
        if rustix::process::geteuid().is_root() {
            return;
        }
        assert!(matches!(
            socket_state(&SocketLocation::at(&path)),
            SocketState::Foreign(_)
        ));
    }

    #[test]
    fn a_start_lock_a_start_would_refuse_is_refused() {
        let dir = private_dir("lock");
        let link = dir.0.join("link.lock");
        std::os::unix::fs::symlink(dir.0.join("elsewhere"), &link).unwrap();
        assert_eq!(
            lock_state(&link),
            LockState::Refused("the start lock is a symlink")
        );
        let not_file = dir.0.join("dir.lock");
        std::fs::create_dir(&not_file).unwrap();
        assert_eq!(
            lock_state(&not_file),
            LockState::Refused("the start lock is not a regular file")
        );
        let usable = dir.0.join("ok.lock");
        std::fs::write(&usable, b"").unwrap();
        assert_eq!(lock_state(&usable), LockState::Usable);
    }

    #[test]
    fn a_shared_directory_is_refused() {
        let dir = private_dir("shared");
        std::fs::set_permissions(&dir.0, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            dir_state(&dir.0),
            DirState::Refused(SocketError::TooPermissive)
        );
    }
}
