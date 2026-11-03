// DDF integration: every user-named input file (`--findings`, source files, patch files, the eval
// corpus manifest) is opened here, so none can stall the CLI.
// Design/status: docs/API_AND_IPC.md
//! Open a user-named input file for reading, refusing anything but a regular file.
//!
//! A plain `open` of a FIFO blocks until a writer appears, so `--findings FIFO` hung for ever
//! (bug hunt 2026-11-01, DDF-HUNT-CLI-8). The file is opened with `O_NONBLOCK`, which returns at
//! once for a FIFO, and its type is then read from the open descriptor (`fstat`), not from the
//! path, so the file checked is the file read. A regular file reads the same with the flag set.

use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::Path;

/// Why [`open_regular`] gave no file.
#[derive(Debug)]
pub(crate) enum Refusal {
    /// The open or the `fstat` failed.
    Io(io::Error),
    /// The path names a FIFO, a directory, a device or a socket.
    NotRegular,
}

/// Open `path` for reading if, once open, it is a regular file.
pub(crate) fn open_regular(path: &Path) -> Result<File, Refusal> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_NONBLOCK)
        .open(path)
        .map_err(Refusal::Io)?;
    if file.metadata().map_err(Refusal::Io)?.is_file() {
        Ok(file)
    } else {
        Err(Refusal::NotRegular)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read as _;

    #[test]
    fn a_regular_file_opens_and_reads_whole() {
        let path = std::env::temp_dir().join(format!("ddf-regular-{}", std::process::id()));
        std::fs::write(&path, b"abc").unwrap();
        let mut text = String::new();
        open_regular(&path)
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        assert_eq!(text, "abc");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn a_directory_is_not_regular_and_a_missing_path_is_an_io_error() {
        assert!(matches!(
            open_regular(&std::env::temp_dir()),
            Err(Refusal::NotRegular)
        ));
        assert!(matches!(
            open_regular(Path::new("/nonexistent-ddf-regular")),
            Err(Refusal::Io(_))
        ));
    }
}
