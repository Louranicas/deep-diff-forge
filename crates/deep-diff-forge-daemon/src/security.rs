use std::ffi::OsString;
use std::os::unix::fs::MetadataExt as _;
use std::os::unix::fs::PermissionsExt as _;
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
                "no secure runtime directory ($XDG_RUNTIME_DIR unset; no /tmp fallback)"
            }
        };
        f.write_str(msg)
    }
}

impl std::error::Error for SocketError {}

/// Pure resolver for the runtime base directory from the `XDG_RUNTIME_DIR` value.
///
/// Returns `None` when the variable is unset or empty. There is deliberately
/// **no world-writable `/tmp` fallback**: `$XDG_RUNTIME_DIR` (typically
/// `/run/user/<uid>`) is a per-user, kernel-managed, owner-only directory, while
/// a predictable `/tmp` path invites symlink/TOCTOU squatting. When it is
/// absent the daemon fails closed and the operator passes an explicit
/// `--socket PATH` (or sets `XDG_RUNTIME_DIR`). Factored out for env-free tests.
#[must_use]
pub fn runtime_base_from(xdg_runtime_dir: Option<OsString>) -> Option<PathBuf> {
    xdg_runtime_dir.filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// The runtime base directory from the live environment, or `None` if
/// `$XDG_RUNTIME_DIR` is unset/empty (no insecure fallback).
#[must_use]
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

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str, mode: u32) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ddf-sec-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(mode)).expect("chmod");
        dir
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
        let dir = temp_dir("ok", 0o700);
        assert_eq!(validate_private_dir(&dir), Ok(()));
    }

    #[test]
    fn world_readable_dir_is_too_permissive() {
        let dir = temp_dir("perm", 0o755);
        assert_eq!(validate_private_dir(&dir), Err(SocketError::TooPermissive));
    }

    #[test]
    fn group_writable_dir_is_too_permissive() {
        let dir = temp_dir("grp", 0o770);
        assert_eq!(validate_private_dir(&dir), Err(SocketError::TooPermissive));
    }

    #[test]
    fn missing_dir_is_missing() {
        let dir = std::env::temp_dir().join(format!("ddf-sec-absent-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(validate_private_dir(&dir), Err(SocketError::Missing));
    }

    #[test]
    fn file_is_not_a_directory() {
        let path = std::env::temp_dir().join(format!("ddf-sec-file-{}", std::process::id()));
        std::fs::write(&path, b"x").expect("write");
        assert_eq!(validate_private_dir(&path), Err(SocketError::NotADirectory));
    }

    #[test]
    fn symlink_is_rejected() {
        // A symlink (even one pointing at a valid owner-private dir) is refused.
        let target = temp_dir("symtarget", 0o700);
        let link = std::env::temp_dir().join(format!("ddf-sec-symlink-{}", std::process::id()));
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(&target, &link).expect("symlink");
        assert_eq!(validate_private_dir(&link), Err(SocketError::Symlink));
        let _ = std::fs::remove_file(&link);
    }

    #[test]
    fn ensure_runtime_dir_creates_and_secures() {
        let dir = std::env::temp_dir().join(format!(
            "ddf-sec-ensure-{}/deep-diff-forge",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
        ensure_runtime_dir(&dir).expect("ensure");
        let mode = std::fs::metadata(&dir).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, SECURE_DIR_MODE);
    }

    #[test]
    fn ensure_runtime_dir_rejects_symlink_without_chmodding_target() {
        let target = temp_dir("ensure-sym-target", 0o755);
        let link =
            std::env::temp_dir().join(format!("ddf-sec-ensure-symlink-{}", std::process::id()));
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(&target, &link).expect("symlink");

        let err = ensure_runtime_dir(&link).expect_err("symlink must be rejected");
        assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
        let mode = std::fs::metadata(&target).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755, "target permissions must be untouched");
        let _ = std::fs::remove_file(&link);
    }

    #[test]
    fn secure_modes_are_owner_only() {
        assert_eq!(SECURE_DIR_MODE & 0o077, 0);
        assert_eq!(SECURE_SOCKET_MODE & 0o077, 0);
    }

    #[test]
    fn socket_error_display_is_descriptive() {
        assert!(SocketError::TooPermissive.to_string().contains("group"));
        assert!(SocketError::Missing.to_string().contains("missing"));
        assert!(SocketError::Symlink.to_string().contains("symlink"));
        assert!(SocketError::WrongOwner.to_string().contains("owned"));
    }
}
