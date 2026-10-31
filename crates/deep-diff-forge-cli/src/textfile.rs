// DDF integration: `daemon metrics --out PATH`, a file for node_exporter's textfile collector.
// Design/status: docs/OPERATIONS_AND_DAEMON.md#exporting-metrics-to-prometheus
//! Replace a file whole, so a reader such as `node_exporter`'s textfile collector sees the old
//! contents or the new, never half of either.

use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::{self, Write as _};
use std::path::Path;

/// The mode of the written file: `0644`, because the textfile collector runs as its own user and
/// must read it, and the daemon metrics hold counts, the version and method names, nothing secret.
pub(crate) const MODE: u32 = 0o644;

fn refuse(path: &Path, why: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("{} {why}; refusing to replace it", path.display()),
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
    let name = path
        .file_name()
        .ok_or_else(|| refuse(path, "names no file"))?;
    // A path that cannot be examined (absent, or under a missing directory) fails below, when the
    // temporary file cannot be created beside it.
    if let Ok(found) = fs::symlink_metadata(path) {
        if found.file_type().is_symlink() {
            return Err(refuse(path, "is a symlink"));
        }
        if !found.is_file() {
            return Err(refuse(path, "is not a regular file"));
        }
    }
    let mut temporary = OsString::from(".");
    temporary.push(name);
    temporary.push(format!(".{}.tmp", std::process::id()));
    let temporary = path.with_file_name(temporary);
    // `create_new` refuses an existing file or symlink at the temporary name, so nothing is
    // written through a link planted there.
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(MODE);
    }
    let mut file = options.open(&temporary)?;
    let written = (|| {
        // The umask may have narrowed the mode; set it exactly.
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
    fn a_planted_temporary_is_not_written_through_or_removed() {
        let dir = scratch("planted");
        let path = dir.join("ddf.prom");
        let planted = dir.join(format!(".ddf.prom.{}.tmp", std::process::id()));
        fs::write(&planted, b"not ours\n").unwrap();
        let err = write_atomically(&path, b"new\n").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(&planted).unwrap(), b"not ours\n");
        assert!(!path.exists());
        fs::remove_dir_all(&dir).unwrap();
    }
}
