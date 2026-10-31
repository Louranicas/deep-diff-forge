// DDF integration: `daemon metrics --out PATH`, a file for node_exporter's textfile collector.
// Design/status: docs/OPERATIONS_AND_DAEMON.md#exporting-metrics-to-prometheus
//! Replace a file whole, so a reader such as `node_exporter`'s textfile collector sees the old
//! contents or the new, never half of either.

use std::ffi::{OsStr, OsString};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

/// The mode of the written file: `0644`, because the textfile collector runs as its own user and
/// must read it, and the daemon metrics hold counts, the version and method names, nothing secret.
pub(crate) const MODE: u32 = 0o644;

/// How many temporary names one write tries. A run killed between creating its temporary and the
/// rename leaves the file behind, and a later run may get the same PID; the next name serves it.
const TEMPORARY_ATTEMPTS: u32 = 8;

/// A refusal, worded to follow the path the caller names (`PATH: is a symlink; …`).
fn refuse(why: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("{why}; refusing to replace it"),
    )
}

/// Write `contents` to `path` through a temporary file in the same directory, then rename it
/// over `path`. The temporary name starts with `.` and does not end `.prom`, so the collector
/// never reads it. A symlink or anything but a regular file at `path` is refused.
///
/// # Errors
///
/// The refusal, or the I/O error that stopped the write; `path` is left as it was.
pub(crate) fn write_atomically(path: &Path, contents: &[u8]) -> io::Result<()> {
    let name = path.file_name().ok_or_else(|| refuse("names no file"))?;
    // A path that cannot be examined (absent, or under a missing directory) fails below, when the
    // temporary file cannot be created beside it.
    if let Ok(found) = fs::symlink_metadata(path) {
        if found.file_type().is_symlink() {
            return Err(refuse("is a symlink"));
        }
        if !found.is_file() {
            return Err(refuse("is not a regular file"));
        }
    }
    let (temporary, mut file) = create_temporary(path, name)?;
    let written = (|| {
        // The umask may have narrowed the mode; set it exactly (tests/daemon.rs writes under
        // umask 077).
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            file.set_permissions(fs::Permissions::from_mode(MODE))?;
        }
        file.write_all(contents)?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    written
}

/// The temporary name of try `attempt`: `.NAME.PID.ATTEMPT.tmp`, beside `path`.
fn temporary_name(path: &Path, name: &OsStr, attempt: u32) -> PathBuf {
    let mut temporary = OsString::from(".");
    temporary.push(name);
    temporary.push(format!(".{}.{attempt}.tmp", std::process::id()));
    path.with_file_name(temporary)
}

/// Create the temporary file, trying the next name while one is taken. `create_new` refuses an
/// existing file or symlink, so nothing is written through a link planted there, and a file left
/// by a killed run is neither reused nor removed.
fn create_temporary(path: &Path, name: &OsStr) -> io::Result<(PathBuf, File)> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(MODE);
    }
    for attempt in 0..TEMPORARY_ATTEMPTS {
        let temporary = temporary_name(path, name, attempt);
        match options.open(&temporary) {
            Ok(file) => return Ok((temporary, file)),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {}
            Err(err) => return Err(err),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!(
            "all {TEMPORARY_ATTEMPTS} temporary names beside it are taken; stale `.*.tmp` files \
             there can be deleted"
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ddf-textfile-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn entries(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn a_new_file_is_written_world_readable_and_no_temporary_is_left() {
        let dir = scratch("new");
        let path = dir.join("ddf.prom");
        write_atomically(&path, b"a 1\n").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"a 1\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, MODE);
        }
        assert_eq!(entries(&dir), ["ddf.prom"]);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_existing_file_is_replaced_not_rewritten_in_place() {
        let dir = scratch("replace");
        let path = dir.join("ddf.prom");
        fs::write(&path, b"old old old\n").unwrap();
        // A reader holding the old file keeps the old contents: the new file is a new inode.
        let held = dir.join("held");
        fs::hard_link(&path, &held).unwrap();
        write_atomically(&path, b"new\n").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"new\n");
        assert_eq!(fs::read(&held).unwrap(), b"old old old\n");
        assert_eq!(entries(&dir), ["ddf.prom", "held"]);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_or_a_directory_at_the_path_is_refused_and_left_alone() {
        let dir = scratch("refuse");
        let target = dir.join("target");
        fs::write(&target, b"keep\n").unwrap();
        let link = dir.join("ddf.prom");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let err = write_atomically(&link, b"new\n").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert!(err.to_string().contains("is a symlink"), "{err}");
        assert_eq!(fs::read(&target).unwrap(), b"keep\n");
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        let sub = dir.join("sub");
        fs::create_dir(&sub).unwrap();
        let err = write_atomically(&sub, b"new\n").unwrap_err();
        assert!(err.to_string().contains("is not a regular file"), "{err}");
        assert_eq!(entries(&dir), ["ddf.prom", "sub", "target"]);
        let err = write_atomically(Path::new("/"), b"new\n").unwrap_err();
        assert!(err.to_string().contains("names no file"), "{err}");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_path_that_cannot_be_examined_or_written_is_an_error() {
        let dir = scratch("unreachable");
        let file = dir.join("file");
        fs::write(&file, b"x").unwrap();
        // A path under a regular file, or in a missing directory: the temporary cannot be made.
        assert!(write_atomically(&file.join("ddf.prom"), b"new\n").is_err());
        let err = write_atomically(&dir.join("missing/ddf.prom"), b"new\n").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        assert_eq!(entries(&dir), ["file"]);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_taken_temporary_name_is_skipped_not_written_through_or_removed() {
        let dir = scratch("planted");
        let path = dir.join("ddf.prom");
        let name = path.file_name().unwrap();
        // What a run killed before its rename leaves behind, under this PID.
        let planted = temporary_name(&path, name, 0);
        fs::write(&planted, b"not ours\n").unwrap();
        write_atomically(&path, b"new\n").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"new\n");
        assert_eq!(fs::read(&planted).unwrap(), b"not ours\n");
        let planted_name = format!(".ddf.prom.{}.0.tmp", std::process::id());
        assert_eq!(entries(&dir), [planted_name, "ddf.prom".to_owned()]);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn every_temporary_name_taken_is_an_error_naming_the_cleanup() {
        let dir = scratch("exhausted");
        let path = dir.join("ddf.prom");
        let name = path.file_name().unwrap();
        for attempt in 0..TEMPORARY_ATTEMPTS {
            fs::write(temporary_name(&path, name, attempt), b"not ours\n").unwrap();
        }
        let err = write_atomically(&path, b"new\n").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert!(err.to_string().contains("all 8 temporary names"), "{err}");
        assert!(!path.exists());
        assert_eq!(entries(&dir).len(), 8);
        for attempt in 0..TEMPORARY_ATTEMPTS {
            assert_eq!(
                fs::read(temporary_name(&path, name, attempt)).unwrap(),
                b"not ours\n"
            );
        }
        fs::remove_dir_all(&dir).unwrap();
    }
}
