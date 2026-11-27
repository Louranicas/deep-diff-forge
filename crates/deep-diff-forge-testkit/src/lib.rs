// Module index: docs/ARCHITECTURE.md#crates-as-built-030
//! Test-only helpers for the workspace (DDF-TEST-TMPLEAK-001).
//!
//! A test that needs a file or directory on disk takes a [`Scratch`]: a fresh, owner-private
//! directory under the system temp dir, removed with everything in it when the guard drops,
//! whether the test passes or panics. This is the only place the workspace reads
//! `std::env::temp_dir()`: `clippy.toml` disallows it everywhere else, and
//! `crates/deep-diff-forge-cli/tests/contract.rs` scans every source file for it, because
//! integration tests opt out of `disallowed_methods` file by file.
//!
//! Before this, about 100 call sites built `temp_dir()/ddf-<name>-<pid>` by hand and many never
//! removed it: one clean run of the daemon and core tests left 37 directories behind.
//!
//! A test that needs another process to hold a lock the product takes calls [`hold_lock`]: one
//! holder for Linux and macOS, used by the hook and council tests.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

/// A fresh, owner-private (0700) directory that is removed, contents and all, when dropped.
///
/// Name it after the test (`Scratch::new("eval-fifo")`); the directory is
/// `<temp>/ddf-<label>-<pid>-<n>`, unique within the process and across concurrent processes.
#[must_use = "a Scratch removes its directory when dropped: bind it to a named variable"]
#[derive(Debug)]
pub struct Scratch(PathBuf);

impl Scratch {
    /// Creates the directory.
    ///
    /// # Panics
    ///
    /// If `label` is empty or holds anything but ASCII letters, digits, `-` and `_` (so it cannot
    /// name a path outside the temp dir), or if the directory cannot be created.
    pub fn new(label: &str) -> Self {
        assert!(
            !label.is_empty()
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
            "a Scratch label is ASCII letters, digits, `-` and `_`: {label:?}"
        );
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = temp_root().join(format!("ddf-{label}-{}-{n}", std::process::id()));
        // Left by an earlier process that had this pid and was killed before its guards ran.
        remove_tree(&path).unwrap_or_else(|e| panic!("clear stale {}: {e}", path.display()));
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        builder
            .create(&path)
            .unwrap_or_else(|e| panic!("create {}: {e}", path.display()));
        Self(path)
    }

    /// The directory itself.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.0
    }

    /// A path inside the directory (nothing is created).
    #[must_use]
    pub fn join(&self, relative: impl AsRef<Path>) -> PathBuf {
        self.0.join(relative)
    }

    /// Writes `contents` to `name` inside the directory, creating parent directories, and
    /// returns its path.
    ///
    /// # Panics
    ///
    /// If a parent directory cannot be created or the file cannot be written.
    pub fn write(&self, name: impl AsRef<Path>, contents: impl AsRef<[u8]>) -> PathBuf {
        let path = self.0.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .unwrap_or_else(|e| panic!("create {}: {e}", parent.display()));
        }
        std::fs::write(&path, contents).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
        path
    }
}

impl AsRef<Path> for Scratch {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    /// Removes the directory. A failure panics, so a leak fails the test instead of piling up,
    /// except while the test is already panicking (a second panic would abort the run).
    fn drop(&mut self) {
        if let Err(e) = remove_tree(&self.0)
            && !std::thread::panicking()
        {
            panic!(
                "scratch directory {} was not removed: {e}",
                self.0.display()
            );
        }
    }
}

/// The holder [`hold_lock`] runs: an exclusive `flock(2)` on `argv[1]` (opened for appending, so
/// made when it is not there), then `argv[2]` made to say so, then a wait while `argv[3]` exists.
const HOLD: &str = "import fcntl, os, sys, time
f = open(sys.argv[1], 'a')
fcntl.flock(f, fcntl.LOCK_EX)
open(sys.argv[2], 'w').close()
while os.path.exists(sys.argv[3]):
    time.sleep(0.02)
";

/// Holds an exclusive `flock(2)` lock on `path` (made when it is not there) from another process
/// until `go` is removed, and returns that process once the lock is held. It then exits 0; it
/// also ends when the directory that holds `go` is removed.
///
/// The lock is the one the product takes (`flock(2)`, `LOCK_EX`), taken by `python3`'s
/// `fcntl.flock`, which Linux and macOS both have. `flock(1)` is util-linux's: macOS has none,
/// and the 0.11.0 macOS CI run failed five tests at its spawn. The holder is the one process
/// with `path` open (its pid is the child's).
///
/// # Panics
///
/// If `go` cannot be written, `python3` cannot be started, or the holder ends or has not taken
/// the lock within 20 s.
#[must_use = "the holder keeps the lock until `go` is removed: wait for it after that"]
#[expect(
    clippy::disallowed_methods,
    reason = "a test helper: starts the lock holder and polls, under a 20 s bound, for its lock"
)]
pub fn hold_lock(path: &Path, go: &Path) -> std::process::Child {
    std::fs::write(go, "").unwrap_or_else(|e| panic!("write {}: {e}", go.display()));
    let held = go.with_extension("held");
    let mut child = std::process::Command::new("python3")
        .args(["-I", "-c", HOLD])
        .arg(path)
        .arg(&held)
        .arg(go)
        .stdin(std::process::Stdio::null())
        .spawn()
        .unwrap_or_else(|e| panic!("start python3 to hold {}: {e}", path.display()));
    let asked = std::time::Instant::now();
    while !held.exists() {
        if let Ok(Some(status)) = child.try_wait() {
            panic!("the holder of {} ended ({status})", path.display());
        }
        assert!(
            asked.elapsed() < std::time::Duration::from_secs(20),
            "no lock on {} after 20 s",
            path.display()
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    child
}

/// The system temp dir (`$TMPDIR`, else `/tmp`): the one reviewed door to it.
#[expect(
    clippy::disallowed_methods,
    reason = "the testkit is the workspace's single door to the temp dir (DDF-TEST-TMPLEAK-001)"
)]
fn temp_root() -> PathBuf {
    std::env::temp_dir()
}

/// Removes `path` and everything under it; absent is success. A test may leave a directory it
/// made read-only (to provoke a refusal), so on failure every directory under `path` is made
/// owner-writable and the removal is tried once more. Symlinks are removed, never followed.
fn remove_tree(path: &Path) -> std::io::Result<()> {
    if std::fs::remove_dir_all(path).is_ok() {
        return Ok(());
    }
    make_owner_writable(path);
    match std::fs::remove_dir_all(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

fn make_owner_writable(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let Ok(meta) = std::fs::symlink_metadata(path) else {
            return;
        };
        if !meta.is_dir() {
            return;
        }
        let mode = meta.permissions().mode() | 0o700;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
        if let Ok(entries) = std::fs::read_dir(path) {
            for entry in entries.flatten() {
                make_owner_writable(&entry.path());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scratch_is_an_empty_private_directory_that_is_gone_after_drop() {
        let scratch = Scratch::new("testkit-basic");
        let path = scratch.path().to_owned();
        assert!(path.is_dir());
        assert_eq!(std::fs::read_dir(&path).unwrap().count(), 0);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700);
        }
        let file = scratch.write("a/b/c.txt", "x");
        assert_eq!(std::fs::read(&file).unwrap(), b"x");
        drop(scratch);
        assert!(!path.exists(), "{} survived its guard", path.display());
    }

    #[test]
    fn a_scratch_is_removed_when_its_test_panics() {
        let seen = std::sync::Mutex::new(None);
        let result = std::panic::catch_unwind(|| {
            let scratch = Scratch::new("testkit-panic");
            scratch.write("f", "x");
            *seen.lock().unwrap() = Some(scratch.path().to_owned());
            panic!("the test fails");
        });
        assert!(result.is_err());
        let path = seen.into_inner().unwrap().unwrap();
        assert!(
            !path.exists(),
            "{} survived a panicking test",
            path.display()
        );
    }

    #[test]
    fn two_scratches_with_one_label_never_share_a_directory() {
        let (a, b) = (Scratch::new("testkit-same"), Scratch::new("testkit-same"));
        assert_ne!(a.path(), b.path());
    }

    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "checks the door against the temp dir it stands for"
    )]
    fn a_scratch_lives_directly_in_the_system_temp_dir() {
        let scratch = Scratch::new("testkit-where");
        assert!(scratch.path().is_absolute());
        assert_eq!(
            scratch.path().parent(),
            Some(std::env::temp_dir().as_path())
        );
    }

    #[test]
    fn an_absent_path_is_already_removed() {
        let scratch = Scratch::new("testkit-absent");
        assert!(remove_tree(&scratch.join("never-made")).is_ok());
    }

    /// A leak fails the test: when the directory cannot be removed, the drop panics. Here the
    /// directory is swapped for a regular file, which `remove_dir_all` refuses.
    #[test]
    fn a_directory_that_cannot_be_removed_fails_the_test() {
        let scratch = Scratch::new("testkit-stuck");
        let path = scratch.path().to_owned();
        std::fs::remove_dir(&path).unwrap();
        std::fs::write(&path, "not a directory").unwrap();
        let dropped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(scratch)));
        std::fs::remove_file(&path).unwrap();
        let message = *dropped
            .expect_err("a scratch that was not removed must fail the test")
            .downcast::<String>()
            .unwrap();
        assert!(message.contains("was not removed"), "{message}");
        assert!(
            remove_tree(&path).is_ok(),
            "removing a missing path is success"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_directory_the_test_made_read_only_is_still_removed() {
        use std::os::unix::fs::PermissionsExt as _;
        let scratch = Scratch::new("testkit-readonly");
        let locked = scratch.join("locked");
        std::fs::create_dir(&locked).unwrap();
        std::fs::write(locked.join("f"), "x").unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o500)).unwrap();
        let path = scratch.path().to_owned();
        drop(scratch);
        assert!(!path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_inside_is_removed_and_its_target_is_left_alone() {
        let outside = Scratch::new("testkit-target");
        let kept = outside.write("keep", "x");
        let scratch = Scratch::new("testkit-link");
        std::os::unix::fs::symlink(outside.path(), scratch.join("link")).unwrap();
        drop(scratch);
        assert_eq!(std::fs::read(&kept).unwrap(), b"x");
    }

    #[test]
    fn a_stale_directory_from_an_earlier_process_is_replaced() {
        let n = NEXT.load(Ordering::Relaxed);
        let stale = temp_root().join(format!("ddf-testkit-stale-{}-{n}", std::process::id()));
        let _ = std::fs::create_dir_all(stale.join("old"));
        // Another test may take `n` first; then this one only checks it got a fresh directory.
        let scratch = Scratch::new("testkit-stale");
        assert_eq!(std::fs::read_dir(scratch.path()).unwrap().count(), 0);
        let _ = remove_tree(&stale);
    }

    /// The holder takes the lock another process's `flock(2)` meets: a try of `LOCK_EX |
    /// LOCK_NB` from a second process is refused while `go` stands and taken once it is gone.
    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "test: runs a second process that tries the lock, and reaps the holder"
    )]
    fn a_held_lock_is_refused_to_another_process_until_go_is_removed() {
        let scratch = Scratch::new("testkit-lock");
        let (lock, go) = (scratch.join("x.lock"), scratch.join("go"));
        // exit 0: taken; 3: another holds it
        let try_lock = || {
            std::process::Command::new("python3")
                .args([
                    "-I",
                    "-c",
                    "import fcntl, sys\nf = open(sys.argv[1], 'a')\ntry:\n    fcntl.flock(f, fcntl.LOCK_EX | fcntl.LOCK_NB)\nexcept BlockingIOError:\n    sys.exit(3)\n",
                ])
                .arg(&lock)
                .status()
                .unwrap()
                .code()
        };
        let mut holder = hold_lock(&lock, &go);
        assert!(lock.is_file(), "the lock file was not made");
        assert_eq!(try_lock(), Some(3), "the holder does not hold the lock");
        std::fs::remove_file(&go).unwrap();
        assert!(holder.wait().unwrap().success());
        assert_eq!(try_lock(), Some(0), "the lock outlived its holder");
    }

    #[test]
    fn a_label_that_could_leave_the_temp_dir_is_refused() {
        for label in ["", "../x", "a/b", "a b", ".", "é"] {
            let refused = std::panic::catch_unwind(|| Scratch::new(label)).is_err();
            assert!(refused, "label {label:?} was accepted");
        }
    }
}
