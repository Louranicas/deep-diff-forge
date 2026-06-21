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
}

impl std::fmt::Display for SocketError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let msg = match self {
            Self::Missing => "runtime directory is missing",
            Self::NotADirectory => "runtime path is not a directory",
            Self::TooPermissive => "runtime directory is accessible by group or others",
        };
        f.write_str(msg)
    }
}

impl std::error::Error for SocketError {}

/// The runtime base directory: `$XDG_RUNTIME_DIR` if set, else a per-process
/// fallback under `/tmp` (matching the CLI `doctor` output).
#[must_use]
pub fn runtime_base() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR").map_or_else(
        || PathBuf::from("/tmp/deep-diff-forge-runtime"),
        PathBuf::from,
    )
}

/// The daemon's private runtime directory (`<base>/deep-diff-forge`).
#[must_use]
pub fn runtime_dir() -> PathBuf {
    runtime_base().join("deep-diff-forge")
}

/// The default socket path (`<runtime-dir>/deep-diff-forge.sock`).
#[must_use]
pub fn default_socket_path() -> PathBuf {
    runtime_dir().join("deep-diff-forge.sock")
}

/// Validate that `dir` is an existing, owner-private directory.
///
/// # Errors
///
/// Returns [`SocketError`] when the directory is missing, is not a directory,
/// or is accessible by group/others.
pub fn validate_private_dir(dir: &Path) -> Result<(), SocketError> {
    let metadata = std::fs::metadata(dir).map_err(|_| SocketError::Missing)?;
    if !metadata.is_dir() {
        return Err(SocketError::NotADirectory);
    }
    if metadata.permissions().mode() & 0o077 != 0 {
        return Err(SocketError::TooPermissive);
    }
    Ok(())
}

/// Create (if needed) the daemon runtime directory with owner-private mode and
/// validate it.
///
/// # Errors
///
/// Returns an I/O error if the directory cannot be created or secured, or a
/// validation failure surfaced as `PermissionDenied`.
pub fn ensure_runtime_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(SECURE_DIR_MODE))?;
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
    fn default_socket_path_ends_with_sock() {
        assert!(
            default_socket_path()
                .to_string_lossy()
                .ends_with("deep-diff-forge.sock")
        );
    }

    #[test]
    fn runtime_dir_is_under_base() {
        assert!(runtime_dir().starts_with(runtime_base()));
    }

    #[test]
    fn runtime_dir_named_deep_diff_forge() {
        assert_eq!(runtime_dir().file_name().unwrap(), "deep-diff-forge");
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
    fn secure_modes_are_owner_only() {
        assert_eq!(SECURE_DIR_MODE & 0o077, 0);
        assert_eq!(SECURE_SOCKET_MODE & 0o077, 0);
    }

    #[test]
    fn socket_error_display_is_descriptive() {
        assert!(SocketError::TooPermissive.to_string().contains("group"));
        assert!(SocketError::Missing.to_string().contains("missing"));
    }
}
