// DDF review route: DDF-R28. Status and remaining work are recorded in the linked ledger; this comment does not claim delivery.
// Design/status: docs/RECOMMENDATION_INTEGRATION.md#remaining-module-routes
// Module index: docs/RECOMMENDATION_INTEGRATION.md#remaining-module-routes
use std::ffi::OsString;
use std::os::unix::fs::MetadataExt as _;
use std::os::unix::fs::PermissionsExt as _;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

/// Required mode for the daemon's runtime directory (owner-only).
pub const SECURE_DIR_MODE: u32 = 0o700;
/// Required mode for the daemon socket (owner read/write only).
pub const SECURE_SOCKET_MODE: u32 = 0o600;

/// Reason a socket location is rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketError {
    /// The directory does not exist.
    Missing,
    /// The path exists but is not a directory.
    NotADirectory,
    /// The directory grants access to group or others (mode `& 0o077 != 0`).
    TooPermissive,
    /// The directory is not owned by the effective user running the daemon.
    WrongOwner,
    /// The path is a symlink — refused, to defeat symlink-swap attacks on the
    /// socket directory.
    Symlink,
    /// No secure runtime directory is available (`$XDG_RUNTIME_DIR` is unset and
    /// there is deliberately no world-writable `/tmp` fallback) and no explicit
    /// socket path was given.
    NoRuntimeDir,
}

impl std::fmt::Display for SocketError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let msg = match self {
            Self::Missing => "runtime directory is missing",
            Self::NotADirectory => "runtime path is not a directory",
            Self::TooPermissive => "runtime directory is accessible by group or others",
            Self::WrongOwner => "runtime directory is not owned by the effective user",
            Self::Symlink => "runtime path is a symlink",
            Self::NoRuntimeDir => {
                "no secure runtime directory ($XDG_RUNTIME_DIR unset or relative; no /tmp fallback)"
            }
        };
        f.write_str(msg)
    }
}

impl std::error::Error for SocketError {}

/// Pure resolver for the runtime base directory from the `XDG_RUNTIME_DIR` value.
///
/// Returns `None` when the variable is unset, empty or relative (the XDG Base
/// Directory spec makes a relative value invalid; one would put the socket under
/// the working directory). There is deliberately
/// **no world-writable `/tmp` fallback**: `$XDG_RUNTIME_DIR` (typically
/// `/run/user/<uid>`) is a per-user, kernel-managed, owner-only directory, while
/// a predictable `/tmp` path invites symlink/TOCTOU squatting. When it is
/// absent the daemon fails closed and the operator passes an explicit
/// `--socket PATH` (or sets `XDG_RUNTIME_DIR`). Factored out for env-free tests.
#[must_use]
pub fn runtime_base_from(xdg_runtime_dir: Option<OsString>) -> Option<PathBuf> {
    // An empty path is not absolute, so one check refuses both.
    xdg_runtime_dir
        .filter(|v| Path::new(v).is_absolute())
        .map(PathBuf::from)
}

/// The runtime base directory from the live environment, or `None` if
/// `$XDG_RUNTIME_DIR` is unset, empty or relative (no insecure fallback).
#[must_use]
#[expect(
    clippy::disallowed_methods,
    reason = "the runtime-directory door: XDG_RUNTIME_DIR locates the socket"
)]
pub fn runtime_base() -> Option<PathBuf> {
    runtime_base_from(std::env::var_os("XDG_RUNTIME_DIR"))
}

/// The daemon's private runtime directory (`<base>/deep-diff-forge`), or `None`.
#[must_use]
pub fn runtime_dir() -> Option<PathBuf> {
    runtime_base().map(|b| b.join("deep-diff-forge"))
}

/// The default socket path (`<runtime-dir>/deep-diff-forge.sock`), or `None` when
/// no secure runtime directory is available.
#[must_use]
pub fn default_socket_path() -> Option<PathBuf> {
    runtime_dir().map(|d| d.join("deep-diff-forge.sock"))
}

/// Validate that `dir` is an existing, owner-private, non-symlink directory.
///
/// Symlinks are rejected outright (via `symlink_metadata`) so an attacker cannot
/// swap the socket directory for a link they control. Ownership is compared
/// directly with the process's effective user, including when running as root.
///
/// # Errors
///
/// Returns [`SocketError`] when the path is a symlink, is missing, is not a
/// directory, or is accessible by group/others.
pub fn validate_private_dir(dir: &Path) -> Result<(), SocketError> {
    // lstat first: reject the path itself being a symlink.
    let link_meta = std::fs::symlink_metadata(dir).map_err(|_| SocketError::Missing)?;
    if link_meta.file_type().is_symlink() {
        return Err(SocketError::Symlink);
    }
    let metadata = std::fs::metadata(dir).map_err(|_| SocketError::Missing)?;
    if !metadata.is_dir() {
        return Err(SocketError::NotADirectory);
    }
    if metadata.uid() != rustix::process::geteuid().as_raw() {
        return Err(SocketError::WrongOwner);
    }
    if metadata.permissions().mode() & 0o077 != 0 {
        return Err(SocketError::TooPermissive);
    }
    Ok(())
}

/// Create (if needed) the daemon runtime directory with owner-private mode and
/// validate it.
///
/// Ownership is validated against the process's effective user after securing
/// the opened directory handle.
///
/// ## Ancestor security
///
/// Only the leaf (`<XDG_RUNTIME_DIR>/deep-diff-forge`) is explicitly secured here.
/// The parent (`$XDG_RUNTIME_DIR`, typically `/run/user/<uid>`) is a kernel-managed,
/// per-user directory that systemd creates with mode `0700` and transitions to
/// owner-only at login; the daemon relies on the kernel/init system to maintain that
/// invariant for the parent. If `$XDG_RUNTIME_DIR` is passed as an explicit `--socket`
/// parent by the operator, the operator is responsible for that directory's security.
///
/// # Errors
///
/// Returns an I/O error if the directory cannot be created or secured, or a
/// validation failure surfaced as `PermissionDenied`.
pub fn ensure_runtime_dir(dir: &Path) -> std::io::Result<()> {
    // Never chmod through a pre-existing symlink. Apart from changing an
    // attacker-chosen target, doing the lstat after chmod would make the
    // validation far too late to be useful.
    match std::fs::symlink_metadata(dir) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    SocketError::Symlink,
                ));
            }
            if !metadata.is_dir() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::NotADirectory,
                    SocketError::NotADirectory,
                ));
            }
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir_all(dir)?;
        }
        Err(err) => return Err(err),
    }
    // Chmod the opened directory object, not the path, and verify that the
    // lstat/open pair identifies the same inode. This closes the symlink-swap
    // window between validation and permission tightening.
    let path_metadata = std::fs::symlink_metadata(dir)?;
    let directory = std::fs::File::open(dir)?;
    let opened_metadata = directory.metadata()?;
    if (path_metadata.dev(), path_metadata.ino()) != (opened_metadata.dev(), opened_metadata.ino())
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "runtime directory changed while it was being secured",
        ));
    }
    directory.set_permissions(std::fs::Permissions::from_mode(SECURE_DIR_MODE))?;
    validate_private_dir(dir)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::PermissionDenied, e))
}

/// The effective user ID of the process at the other end of a connected Unix socket, as the kernel
/// recorded it when the connection was made (`SO_PEERCRED` on Linux, `getpeereid` on macOS and the
/// BSDs). Other platforms fail closed with `Unsupported`.
///
/// # Errors
///
/// Returns the OS error when the credentials cannot be read, or `Unsupported` on a platform with
/// no peer-credential call.
pub fn peer_euid(stream: &UnixStream) -> std::io::Result<u32> {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        Ok(rustix::net::sockopt::socket_peercred(stream)?.uid.as_raw())
    }
    #[cfg(any(
        target_vendor = "apple",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly"
    ))]
    {
        Ok(nix::unistd::getpeereid(stream)?.0.as_raw())
    }
    #[cfg(not(any(
        target_os = "linux",
        target_os = "android",
        target_vendor = "apple",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly"
    )))]
    {
        let _ = stream;
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "this platform cannot read a Unix socket peer's credentials",
        ))
    }
}

/// Refuse a connection whose peer is not `expected` (the caller passes its own effective user ID).
/// The daemon checks every client it accepts, and the client checks the daemon it reached, so
/// neither relies on the `0700` directory and `0600` socket alone: a socket another user made
/// reachable, or a `--socket` path naming another user's daemon, is refused at connect.
///
/// # Errors
///
/// Returns `PermissionDenied` naming both user IDs when they differ, or the error of
/// [`peer_euid`].
pub fn require_peer_user(stream: &UnixStream, expected: u32) -> std::io::Result<()> {
    let peer = peer_euid(stream)?;
    if peer == expected {
        Ok(())
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!("the peer runs as uid {peer}, not uid {expected}"),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deep_diff_forge_testkit::Scratch;

    fn private_scratch(name: &str, mode: u32) -> Scratch {
        let dir = Scratch::new(&format!("sec-{name}"));
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(mode)).expect("chmod");
        dir
    }

    /// SECURITY.md (daemon): "The trust boundary is the OS account." The positive half: a peer
    /// of the same effective user is admitted. The outcome is asserted, not unwrapped, so a
    /// comparison that refuses everyone fails here by name (U1, lane T3b).
    #[test]
    fn a_peer_is_the_user_that_connected() {
        let me = rustix::process::geteuid().as_raw();
        let (ours, theirs) = UnixStream::pair().expect("socket pair");
        assert_eq!(peer_euid(&ours).expect("peer credentials"), me);
        assert_eq!(peer_euid(&theirs).expect("peer credentials"), me);
        let admitted = require_peer_user(&ours, me);
        assert!(
            admitted.is_ok(),
            "a peer of the daemon's own user was refused: {admitted:?}"
        );
    }

    /// The negative half: a peer whose kernel-reported uid is not the expected one is refused
    /// with `PermissionDenied` naming both. Another real user cannot be had in a test, so the
    /// expected uid is moved instead; that `peer_euid` reads the peer's credentials, not this
    /// process's, is not held here (it needs a second account: a stated limit).
    #[test]
    fn a_peer_of_another_user_is_refused() {
        let me = rustix::process::geteuid().as_raw();
        let (ours, _theirs) = UnixStream::pair().expect("socket pair");
        let other = me.wrapping_add(1);
        let refused = require_peer_user(&ours, other);
        assert!(
            refused
                .as_ref()
                .is_err_and(|err| err.kind() == std::io::ErrorKind::PermissionDenied),
            "a peer of uid {me} was admitted where uid {other} was expected: {refused:?}"
        );
        assert_eq!(
            refused.map_err(|err| err.to_string()),
            Err(format!("the peer runs as uid {me}, not uid {other}"))
        );
    }

    #[test]
    fn runtime_base_uses_xdg_runtime_dir() {
        let base = runtime_base_from(Some(OsString::from("/run/user/1000"))).expect("base");
        assert_eq!(base, PathBuf::from("/run/user/1000"));
    }

    #[test]
    fn runtime_base_none_without_xdg() {
        assert_eq!(runtime_base_from(None), None);
        assert_eq!(runtime_base_from(Some(OsString::new())), None);
        // Relative values are invalid: the socket would land under the working directory.
        assert_eq!(runtime_base_from(Some(OsString::from("run"))), None);
        assert_eq!(runtime_base_from(Some(OsString::from("./run/user"))), None);
    }

    #[test]
    fn socket_path_derives_from_xdg_base() {
        // Exercise the full chain with an explicit base (no env, no /tmp).
        let dir = runtime_base_from(Some(OsString::from("/run/user/1000")))
            .map(|b| b.join("deep-diff-forge"))
            .expect("dir");
        let sock = dir.join("deep-diff-forge.sock");
        assert!(sock.to_string_lossy().ends_with("deep-diff-forge.sock"));
        assert_eq!(dir.file_name().unwrap(), "deep-diff-forge");
        assert!(sock.starts_with("/run/user/1000"));
    }

    #[test]
    fn private_dir_0700_validates() {
        let dir = private_scratch("ok", 0o700);
        assert_eq!(validate_private_dir(dir.path()), Ok(()));
    }

    #[test]
    fn world_readable_dir_is_too_permissive() {
        let dir = private_scratch("perm", 0o755);
        assert_eq!(
            validate_private_dir(dir.path()),
            Err(SocketError::TooPermissive)
        );
    }

    #[test]
    fn group_writable_dir_is_too_permissive() {
        let dir = private_scratch("grp", 0o770);
        assert_eq!(
            validate_private_dir(dir.path()),
            Err(SocketError::TooPermissive)
        );
    }

    #[test]
    fn missing_dir_is_missing() {
        let scratch = Scratch::new("sec-absent");
        let dir = scratch.join("absent");
        assert_eq!(validate_private_dir(&dir), Err(SocketError::Missing));
    }

    #[test]
    fn file_is_not_a_directory() {
        let scratch = Scratch::new("sec-file");
        let path = scratch.write("file", b"x");
        assert_eq!(validate_private_dir(&path), Err(SocketError::NotADirectory));
    }

    #[test]
    fn symlink_is_rejected() {
        // A symlink (even one pointing at a valid owner-private dir) is refused.
        let target = private_scratch("symtarget", 0o700);
        let links = Scratch::new("sec-symlink");
        let link = links.join("link");
        std::os::unix::fs::symlink(target.path(), &link).expect("symlink");
        assert_eq!(validate_private_dir(&link), Err(SocketError::Symlink));
    }

    #[test]
    fn ensure_runtime_dir_creates_and_secures() {
        let scratch = Scratch::new("sec-ensure");
        let dir = scratch.join("absent").join("deep-diff-forge");
        ensure_runtime_dir(&dir).expect("ensure");
        let mode = std::fs::metadata(&dir).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, SECURE_DIR_MODE);
    }

    #[test]
    fn ensure_runtime_dir_rejects_symlink_without_chmodding_target() {
        let target = private_scratch("ensure-sym-target", 0o755);
        let links = Scratch::new("sec-ensure-symlink");
        let link = links.join("link");
        std::os::unix::fs::symlink(target.path(), &link).expect("symlink");

        let refused = ensure_runtime_dir(&link);
        assert!(
            refused
                .as_ref()
                .is_err_and(|err| err.kind() == std::io::ErrorKind::PermissionDenied),
            "a symlinked runtime directory was not refused: {refused:?}"
        );
        let mode = std::fs::metadata(target.path())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o755, "target permissions must be untouched");
    }

    // The live wrappers must resolve exactly what the pure resolver yields for
    // the current environment, whether `XDG_RUNTIME_DIR` is set or not. Reads
    // the environment only; nothing in this crate mutates it.
    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "test: compares the resolver with the live environment"
    )]
    fn live_resolvers_match_the_environment() {
        let expected = runtime_base_from(std::env::var_os("XDG_RUNTIME_DIR"));
        assert_eq!(runtime_base(), expected);
        let expected_dir = expected.map(|base| base.join("deep-diff-forge"));
        assert_eq!(runtime_dir(), expected_dir);
        assert_eq!(
            default_socket_path(),
            expected_dir.map(|dir| dir.join("deep-diff-forge.sock"))
        );
    }

    #[test]
    fn ensure_runtime_dir_propagates_non_missing_errors_without_creating() {
        // A regular file used as a parent makes lstat fail with ENOTDIR, not
        // ENOENT; that must be reported, not treated as "absent, create it".
        let scratch = Scratch::new("sec-notdir");
        let file = scratch.write("file", b"x");
        let err = ensure_runtime_dir(&file.join("deep-diff-forge")).expect_err("ENOTDIR");
        assert_eq!(err.kind(), std::io::ErrorKind::NotADirectory);
        assert!(std::fs::metadata(&file).unwrap().is_file());
    }

    #[test]
    fn secure_modes_are_owner_only() {
        assert_eq!(SECURE_DIR_MODE & 0o077, 0);
        assert_eq!(SECURE_SOCKET_MODE & 0o077, 0);
    }

    /// SECURITY.md (daemon): "Its managed runtime directory ... is owned by the effective user
    /// with mode `0700`; its socket is mode `0600`". The numbers themselves, not only "owner
    /// only": `0o600` for the directory or `0o700` for the socket would pass the test above.
    #[test]
    fn the_documented_modes_are_0700_and_0600() {
        assert_eq!(
            (SECURE_DIR_MODE, SECURE_SOCKET_MODE),
            (0o700, 0o600),
            "SECURITY.md documents the runtime directory at 0700 and the socket at 0600"
        );
    }

    /// The managed runtime directory is the daemon's own: one that exists with a looser mode is
    /// made `0700` on the opened directory, and ends owned by the effective user. Before lane
    /// T3b only a freshly created directory was checked, against the constant itself.
    #[test]
    fn an_existing_managed_directory_is_made_0700_and_owned_by_the_effective_user() {
        let scratch = Scratch::new("sec-managed");
        let dir = scratch.join("deep-diff-forge");
        std::fs::create_dir(&dir).expect("mkdir");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        let ensured = ensure_runtime_dir(&dir);
        assert!(
            ensured.is_ok(),
            "the managed runtime directory at 0755 was not secured: {ensured:?}"
        );
        let metadata = std::fs::symlink_metadata(&dir).expect("lstat");
        assert_eq!(
            (
                metadata.permissions().mode() & 0o777,
                metadata.uid() == rustix::process::geteuid().as_raw()
            ),
            (0o700, true),
            "the managed runtime directory is not 0700 and owned by the effective user"
        );
    }

    /// SECURITY.md (daemon): "Symlinks, wrong-owner directories, permissive directories and
    /// inode swaps fail closed." Wrong owner: `/` belongs to root on Linux and macOS, and the
    /// owner is checked before the mode, so another user's directory is refused as such (with the
    /// owner check gone it would read as merely too permissive). As root, or where `/` is this
    /// user's, there is no other user's directory to hand: the test says so and checks nothing.
    #[test]
    fn a_directory_of_another_user_is_refused_as_wrong_owner() {
        let root = std::fs::symlink_metadata("/").expect("lstat /");
        if root.uid() == rustix::process::geteuid().as_raw() {
            eprintln!("skipped: `/` is owned by this test's own user (root?); no other owner here");
            return;
        }
        assert_eq!(
            validate_private_dir(Path::new("/")),
            Err(SocketError::WrongOwner),
            "a directory owned by another user was not refused as wrong-owner"
        );
    }

    /// Inode swaps: `ensure_runtime_dir` chmods the directory it opened, and only after checking
    /// that it is the one the path named at its `lstat`. The path `flip/deep-diff-forge` is reached
    /// through a link that a second thread flips, by `rename`, between `real/` (where it is a
    /// directory) and `trap/` (where it is a link to `victim`, mode 0755). However the race falls,
    /// on correct code `victim` is never chmodded: a link seen by the first `lstat` is refused,
    /// and one that appears after it is caught by the identity check. Counted, not timed: the
    /// rounds and the refusals are printed, and red is the victim's mode, named.
    #[test]
    fn a_runtime_directory_swapped_after_its_check_is_never_secured_through_the_swap() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let scratch = Scratch::new("sec-swap");
        let victim = scratch.join("victim");
        std::fs::create_dir(&victim).expect("mkdir victim");
        std::fs::set_permissions(&victim, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        for side in ["real", "trap"] {
            std::fs::create_dir(scratch.join(side)).expect("mkdir side");
        }
        std::fs::create_dir(scratch.join("real").join("deep-diff-forge")).expect("mkdir real");
        std::os::unix::fs::symlink(&victim, scratch.join("trap").join("deep-diff-forge"))
            .expect("link trap");
        let flip = scratch.join("flip");
        std::os::unix::fs::symlink(scratch.join("real"), &flip).expect("link flip");
        let target = flip.join("deep-diff-forge");
        let stop = AtomicBool::new(false);
        let (rounds, refused, victim_mode) = std::thread::scope(|scope| {
            scope.spawn(|| {
                let staged = scratch.join("flip.next");
                let mut to_trap = true;
                while !stop.load(Ordering::Acquire) {
                    let side = scratch.join(if to_trap { "trap" } else { "real" });
                    let _ = std::fs::remove_file(&staged);
                    std::os::unix::fs::symlink(&side, &staged).expect("stage link");
                    std::fs::rename(&staged, &flip).expect("flip link");
                    to_trap = !to_trap;
                }
            });
            let mut refused = 0_u32;
            let mut rounds = 0_u32;
            let mut victim_mode = 0o755;
            while rounds < 20_000 {
                rounds += 1;
                if ensure_runtime_dir(&target).is_err() {
                    refused += 1;
                }
                victim_mode = std::fs::metadata(&victim)
                    .expect("stat victim")
                    .permissions()
                    .mode()
                    & 0o777;
                if victim_mode != 0o755 {
                    break;
                }
            }
            stop.store(true, Ordering::Release);
            (rounds, refused, victim_mode)
        });
        eprintln!("swap race: {rounds} rounds, {refused} refused");
        assert_eq!(
            victim_mode, 0o755,
            "a directory swapped in after the check was chmodded through the swap (round {rounds})"
        );
    }

    #[test]
    fn socket_error_display_is_descriptive() {
        assert!(SocketError::TooPermissive.to_string().contains("group"));
        assert!(SocketError::Missing.to_string().contains("missing"));
        assert!(SocketError::Symlink.to_string().contains("symlink"));
        assert!(SocketError::WrongOwner.to_string().contains("owned"));
    }
}
