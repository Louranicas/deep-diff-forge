//! Local-only learning store.
//!
//! All learning data lives under `$XDG_STATE_HOME/deep-diff-forge/learning/`
//! (falling back to `$HOME/.local/state/...`). Receipts are appended as JSONL —
//! one self-describing record per line, append-only, human-inspectable, and
//! trivially recoverable. Nothing is ever uploaded; the store is the whole
//! footprint of the learning loop on a machine.
//!
//! Every function takes an explicit base directory so the store is testable
//! without touching the real state directory; [`learning_dir`] resolves the
//! production location and the `*_default` wrappers use it.

use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read as _, Write};
use std::path::{Path, PathBuf};

use crate::error::LearningError;
use crate::receipt::StrategyReceipt;

/// Subdirectory under the resolved state dir.
const LEARNING_SUBDIR: &str = "deep-diff-forge/learning";
/// Receipts file name, relative to the learning dir.
const RECEIPTS_FILE: &str = "receipts/strategy.jsonl";
/// Maximum on-disk receipt store size accepted or produced by this process.
const MAX_RECEIPTS_BYTES: u64 = 64 * 1024 * 1024;
/// Maximum size of one JSONL record, including its newline.
const MAX_RECEIPT_LINE_BYTES: usize = 1024 * 1024;
/// Maximum number of records materialized by [`load_receipts`].
const MAX_RECEIPTS: usize = 100_000;

/// Pure resolver for the learning directory, given the two relevant env values.
///
/// Order: `$XDG_STATE_HOME/deep-diff-forge/learning`, else
/// `$HOME/.local/state/deep-diff-forge/learning`. Empty values are ignored.
/// Factored out so it is testable without mutating process-global environment
/// (which is `unsafe` under edition 2024 and would couple parallel tests).
///
/// # Errors
/// Returns [`LearningError::NoStateDir`] if neither value is usable.
pub fn resolve_learning_dir(
    xdg_state_home: Option<OsString>,
    home: Option<OsString>,
) -> Result<PathBuf, LearningError> {
    if let Some(base) = xdg_state_home
        && !base.is_empty()
    {
        return Ok(PathBuf::from(base).join(LEARNING_SUBDIR));
    }
    if let Some(home) = home
        && !home.is_empty()
    {
        return Ok(PathBuf::from(home)
            .join(".local/state")
            .join(LEARNING_SUBDIR));
    }
    Err(LearningError::NoStateDir)
}

/// Resolve the production learning directory from the live environment.
///
/// # Errors
/// Returns [`LearningError::NoStateDir`] if neither `$XDG_STATE_HOME` nor
/// `$HOME` is set.
pub fn learning_dir() -> Result<PathBuf, LearningError> {
    resolve_learning_dir(std::env::var_os("XDG_STATE_HOME"), std::env::var_os("HOME"))
}

/// Path to the receipts JSONL file under `dir`.
#[must_use]
pub fn receipts_path(dir: &Path) -> PathBuf {
    dir.join(RECEIPTS_FILE)
}

/// Append one receipt as a JSONL line under `dir`, creating directories as
/// needed. Append-only: existing receipts are never rewritten.
///
/// # Errors
/// Returns an error if the directory cannot be created, the receipt cannot be
/// serialized, or the write fails.
pub fn append_receipt(dir: &Path, receipt: &StrategyReceipt) -> Result<(), LearningError> {
    validate_receipt_for_storage(receipt)?;
    let path = receipts_path(dir);
    if let Some(parent) = path.parent() {
        // The privacy contract ("local-only … prefer hashes, counts, timings")
        // is load-bearing: enforce owner-private (0o700) directories rather than
        // inheriting the process umask. Failing to secure them is an error, not
        // silently accepted — the store must not exist world-readable.
        ensure_secure_dir(dir)?;
        ensure_secure_dir(parent)?;
    }
    let mut line = receipt.to_json()?;
    line.push('\n');
    if line.len() > MAX_RECEIPT_LINE_BYTES {
        return Err(invalid_data(
            "serialized receipt exceeds the per-record budget",
        ));
    }
    let mut file = open_receipts_for_append(&path)?;
    let current_len = file.metadata()?.len();
    if current_len.saturating_add(line.len() as u64) > MAX_RECEIPTS_BYTES {
        return Err(invalid_data(
            "receipt store exceeds the on-disk size budget",
        ));
    }
    file.write_all(line.as_bytes())?;
    Ok(())
}

/// Create `dir` if absent, reject symlinks/non-directories, and tighten the
/// actual directory to owner-only (`0o700`) on Unix.
///
/// # Errors
/// Returns an I/O error if the permissions cannot be set.
fn ensure_secure_dir(dir: &Path) -> Result<(), LearningError> {
    match fs::symlink_metadata(dir) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(invalid_data("learning directory must not be a symlink"));
        }
        Ok(metadata) if !metadata.is_dir() => {
            return Err(invalid_data("learning path is not a directory"));
        }
        Ok(_) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => fs::create_dir_all(dir)?,
        Err(err) => return Err(err.into()),
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
        let path_metadata = fs::symlink_metadata(dir)?;
        if path_metadata.uid() != rustix::process::geteuid().as_raw() {
            return Err(invalid_data(
                "learning directory is not owned by the effective user",
            ));
        }
        let directory = File::open(dir)?;
        let opened_metadata = directory.metadata()?;
        if (path_metadata.dev(), path_metadata.ino())
            != (opened_metadata.dev(), opened_metadata.ino())
        {
            return Err(invalid_data(
                "learning directory changed while it was being secured",
            ));
        }
        directory.set_permissions(fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(not(unix))]
    {
        let _ = dir; // perms model differs; rely on the user profile directory.
    }
    Ok(())
}

fn invalid_data(message: &'static str) -> LearningError {
    std::io::Error::new(std::io::ErrorKind::InvalidData, message).into()
}

fn validate_receipt_for_storage(receipt: &StrategyReceipt) -> Result<(), LearningError> {
    let hash_valid = (16..=64).contains(&receipt.file_hash.len())
        && receipt
            .file_hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    if !hash_valid {
        return Err(invalid_data(
            "file_hash must be a 16-64 character lowercase hexadecimal redacted id",
        ));
    }
    let token_valid = |value: &str, maximum: usize| {
        !value.is_empty()
            && value.len() <= maximum
            && value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'+' | b'@')
            })
    };
    if !token_valid(&receipt.language, 64) {
        return Err(invalid_data("language must be a bounded identifier token"));
    }
    if !token_valid(&receipt.parser_version, 128) {
        return Err(invalid_data(
            "parser_version must be a bounded identifier token",
        ));
    }
    Ok(())
}

/// Validate that a path and opened handle identify the same owner-private
/// regular file. The identity check closes the lstat/open swap window.
fn validate_receipts_file(path_metadata: &fs::Metadata, file: &File) -> Result<(), LearningError> {
    if path_metadata.file_type().is_symlink() || !path_metadata.is_file() {
        return Err(invalid_data(
            "receipt store must be a regular non-symlink file",
        ));
    }
    let file_metadata = file.metadata()?;
    if !file_metadata.is_file() {
        return Err(invalid_data("opened receipt store is not a regular file"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
        if path_metadata.dev() != file_metadata.dev() || path_metadata.ino() != file_metadata.ino()
        {
            return Err(invalid_data(
                "receipt store changed while it was being opened",
            ));
        }
        if file_metadata.permissions().mode() & 0o077 != 0 {
            return Err(invalid_data(
                "receipt store is accessible by group or others",
            ));
        }
        if file_metadata.uid() != rustix::process::geteuid().as_raw() {
            return Err(invalid_data(
                "receipt store is not owned by the effective user",
            ));
        }
    }
    if file_metadata.len() > MAX_RECEIPTS_BYTES {
        return Err(invalid_data(
            "receipt store exceeds the on-disk size budget",
        ));
    }
    Ok(())
}

fn existing_receipts_file(path: &Path) -> Result<Option<(fs::Metadata, File)>, LearningError> {
    let path_metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err.into()),
    };
    if path_metadata.file_type().is_symlink() || !path_metadata.is_file() {
        return Err(invalid_data(
            "receipt store must be a regular non-symlink file",
        ));
    }
    let file = OpenOptions::new().read(true).open(path)?;
    validate_receipts_file(&path_metadata, &file)?;
    Ok(Some((path_metadata, file)))
}

fn open_receipts_for_append(path: &Path) -> Result<File, LearningError> {
    if let Some((path_metadata, _)) = existing_receipts_file(path)? {
        let file = OpenOptions::new().append(true).open(path)?;
        validate_receipts_file(&path_metadata, &file)?;
        return Ok(file);
    }

    let mut options = OpenOptions::new();
    options.create_new(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let file = options.open(path)?;
    let path_metadata = fs::symlink_metadata(path)?;
    validate_receipts_file(&path_metadata, &file)?;
    Ok(file)
}

fn open_receipts_for_read(path: &Path) -> Result<Option<File>, LearningError> {
    existing_receipts_file(path).map(|entry| entry.map(|(_, file)| file))
}

fn read_capped_line(
    reader: &mut BufReader<File>,
    line: &mut Vec<u8>,
) -> Result<usize, LearningError> {
    let read = reader
        .take(MAX_RECEIPT_LINE_BYTES as u64 + 1)
        .read_until(b'\n', line)?;
    if line.len() > MAX_RECEIPT_LINE_BYTES {
        return Err(invalid_data("receipt record exceeds the per-record budget"));
    }
    Ok(read)
}

/// Load all receipts under `dir`.
///
/// A missing store is not an error — it yields an empty vector (the loop is
/// fail-soft: "no data yet" is a normal state). Blank lines are skipped. A
/// single corrupt line aborts with [`LearningError::Deserialize`] rather than
/// silently dropping data, so corruption is visible rather than masked.
///
/// # Errors
/// Returns an error if the file exists but cannot be read, or if a non-blank
/// line fails to parse.
pub fn load_receipts(dir: &Path) -> Result<Vec<StrategyReceipt>, LearningError> {
    let path = receipts_path(dir);
    let Some(file) = open_receipts_for_read(&path)? else {
        return Ok(Vec::new());
    };
    let reader = BufReader::new(file);
    let mut reader = reader;
    let mut receipts = Vec::new();
    let mut line = Vec::new();
    let mut total_read = 0_u64;
    loop {
        line.clear();
        let read = read_capped_line(&mut reader, &mut line)?;
        if read == 0 {
            break;
        }
        total_read = total_read.saturating_add(read as u64);
        if total_read > MAX_RECEIPTS_BYTES {
            return Err(invalid_data(
                "receipt store grew beyond the on-disk size budget while reading",
            ));
        }
        let line = std::str::from_utf8(&line)
            .map_err(|_| invalid_data("receipt store contains invalid UTF-8"))?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if receipts.len() >= MAX_RECEIPTS {
            return Err(invalid_data(
                "receipt store exceeds the record-count budget",
            ));
        }
        receipts.push(StrategyReceipt::from_json(trimmed)?);
    }
    Ok(receipts)
}

/// Count receipts under `dir` without fully materializing them.
///
/// # Errors
/// Returns an error if the file exists but cannot be read.
pub fn count_receipts(dir: &Path) -> Result<usize, LearningError> {
    let path = receipts_path(dir);
    let Some(file) = open_receipts_for_read(&path)? else {
        return Ok(0);
    };
    let mut reader = BufReader::new(file);
    let mut n = 0;
    let mut line = Vec::new();
    let mut total_read = 0_u64;
    loop {
        line.clear();
        let read = read_capped_line(&mut reader, &mut line)?;
        if read == 0 {
            break;
        }
        total_read = total_read.saturating_add(read as u64);
        if total_read > MAX_RECEIPTS_BYTES {
            return Err(invalid_data(
                "receipt store grew beyond the on-disk size budget while reading",
            ));
        }
        let line = std::str::from_utf8(&line)
            .map_err(|_| invalid_data("receipt store contains invalid UTF-8"))?;
        if !line.trim().is_empty() {
            n += 1;
            if n > MAX_RECEIPTS {
                return Err(invalid_data(
                    "receipt store exceeds the record-count budget",
                ));
            }
        }
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::receipt::{CacheState, ReviewOutcome, Strategy};
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    /// Unique scratch dir per test — no `Math.random`/clock; a process-unique
    /// counter keeps parallel tests from colliding.
    fn temp_dir() -> PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        std::env::temp_dir().join(format!("ddf-learn-store-{pid}-{n}"))
    }

    struct Scratch(PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn receipt(s: Strategy, outcome: ReviewOutcome) -> StrategyReceipt {
        StrategyReceipt::new("0123456789abcdef", "rust", "v", s)
            .with_cache(CacheState::Hit)
            .with_outcome(outcome, false)
    }

    fn write_private(path: &Path, body: &[u8]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut options = OpenOptions::new();
        options.create(true).truncate(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        options.open(path).unwrap().write_all(body).unwrap();
    }

    #[test]
    fn receipts_path_joins_subdir() {
        let p = receipts_path(Path::new("/base"));
        assert!(p.ends_with("receipts/strategy.jsonl"));
    }

    #[test]
    fn load_missing_store_is_empty_not_error() {
        let dir = temp_dir();
        let _g = Scratch(dir.clone());
        assert_eq!(load_receipts(&dir).expect("load"), Vec::new());
    }

    #[test]
    fn count_missing_store_is_zero() {
        let dir = temp_dir();
        let _g = Scratch(dir.clone());
        assert_eq!(count_receipts(&dir).expect("count"), 0);
    }

    #[test]
    fn append_then_load_round_trips() {
        let dir = temp_dir();
        let _g = Scratch(dir.clone());
        let r = receipt(Strategy::Syntax, ReviewOutcome::Accepted);
        append_receipt(&dir, &r).expect("append");
        let loaded = load_receipts(&dir).expect("load");
        assert_eq!(loaded, vec![r]);
    }

    #[cfg(unix)]
    #[test]
    fn store_is_owner_private() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = temp_dir();
        let _g = Scratch(dir.clone());
        append_receipt(&dir, &receipt(Strategy::Patch, ReviewOutcome::Accepted)).expect("append");
        let file_mode = fs::metadata(receipts_path(&dir))
            .unwrap()
            .permissions()
            .mode();
        let dir_mode = fs::metadata(&dir).unwrap().permissions().mode();
        let receipts_dir_mode = fs::metadata(receipts_path(&dir).parent().unwrap())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(
            file_mode & 0o777,
            0o600,
            "JSONL must be owner read/write only"
        );
        assert_eq!(
            dir_mode & 0o077,
            0,
            "learning dir must not be group/world accessible"
        );
        assert_eq!(
            receipts_dir_mode & 0o077,
            0,
            "receipts dir must be owner-only"
        );
    }

    #[test]
    fn append_is_additive() {
        let dir = temp_dir();
        let _g = Scratch(dir.clone());
        append_receipt(&dir, &receipt(Strategy::Patch, ReviewOutcome::Accepted)).expect("a");
        append_receipt(&dir, &receipt(Strategy::Syntax, ReviewOutcome::Rejected)).expect("b");
        append_receipt(&dir, &receipt(Strategy::Hybrid, ReviewOutcome::Skipped)).expect("c");
        assert_eq!(load_receipts(&dir).expect("load").len(), 3);
        assert_eq!(count_receipts(&dir).expect("count"), 3);
    }

    #[test]
    fn append_creates_nested_dirs() {
        let dir = temp_dir();
        let _g = Scratch(dir.clone());
        assert!(!receipts_path(&dir).exists());
        append_receipt(&dir, &receipt(Strategy::Patch, ReviewOutcome::Accepted)).expect("append");
        assert!(receipts_path(&dir).exists());
    }

    #[test]
    fn load_skips_blank_lines() {
        let dir = temp_dir();
        let _g = Scratch(dir.clone());
        let path = receipts_path(&dir);
        let r = receipt(Strategy::Patch, ReviewOutcome::Accepted);
        let body = format!("\n{}\n\n", r.to_json().unwrap());
        write_private(&path, body.as_bytes());
        assert_eq!(load_receipts(&dir).expect("load"), vec![r]);
    }

    #[test]
    fn load_surfaces_corruption_rather_than_dropping() {
        let dir = temp_dir();
        let _g = Scratch(dir.clone());
        let path = receipts_path(&dir);
        write_private(&path, b"this is not json\n");
        assert!(load_receipts(&dir).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn append_rejects_symlink_directory_without_chmodding_target() {
        use std::os::unix::fs::PermissionsExt as _;
        let target = temp_dir();
        let link = temp_dir();
        let _target_guard = Scratch(target.clone());
        fs::create_dir_all(&target).unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o755)).unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let result = append_receipt(&link, &receipt(Strategy::Patch, ReviewOutcome::Accepted));
        assert!(result.is_err());
        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o755
        );
        let _ = fs::remove_file(&link);
    }

    #[cfg(unix)]
    #[test]
    fn read_and_append_reject_receipt_file_symlink() {
        let dir = temp_dir();
        let _guard = Scratch(dir.clone());
        let path = receipts_path(&dir);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let victim = dir.join("victim.jsonl");
        write_private(&victim, b"do not touch\n");
        std::os::unix::fs::symlink(&victim, &path).unwrap();

        assert!(load_receipts(&dir).is_err());
        assert!(append_receipt(&dir, &receipt(Strategy::Patch, ReviewOutcome::Accepted)).is_err());
        assert_eq!(fs::read(&victim).unwrap(), b"do not touch\n");
    }

    #[cfg(unix)]
    #[test]
    fn read_rejects_world_readable_receipt_store() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = temp_dir();
        let _guard = Scratch(dir.clone());
        let path = receipts_path(&dir);
        write_private(&path, b"\n");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(load_receipts(&dir).is_err());
    }

    #[test]
    fn read_rejects_oversized_sparse_store() {
        let dir = temp_dir();
        let _guard = Scratch(dir.clone());
        let path = receipts_path(&dir);
        write_private(&path, b"");
        OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(MAX_RECEIPTS_BYTES + 1)
            .unwrap();
        assert!(count_receipts(&dir).is_err());
    }

    #[test]
    fn append_rejects_unredacted_file_identity() {
        let dir = temp_dir();
        let _guard = Scratch(dir.clone());
        let receipt = StrategyReceipt::new("secret/source/path.rs", "rust", "v1", Strategy::Patch);
        assert!(append_receipt(&dir, &receipt).is_err());
        assert!(!receipts_path(&dir).exists());
    }

    #[test]
    fn append_rejects_path_like_metadata_tokens() {
        let dir = temp_dir();
        let _guard = Scratch(dir.clone());
        let receipt =
            StrategyReceipt::new("0123456789abcdef", "../../secret", "v1", Strategy::Patch);
        assert!(append_receipt(&dir, &receipt).is_err());
    }

    #[test]
    fn order_is_preserved() {
        let dir = temp_dir();
        let _g = Scratch(dir.clone());
        append_receipt(&dir, &receipt(Strategy::Patch, ReviewOutcome::Accepted)).unwrap();
        append_receipt(&dir, &receipt(Strategy::Syntax, ReviewOutcome::Accepted)).unwrap();
        let loaded = load_receipts(&dir).expect("load");
        assert_eq!(loaded[0].strategy, Strategy::Patch);
        assert_eq!(loaded[1].strategy, Strategy::Syntax);
    }

    #[test]
    fn resolver_prefers_xdg_state_home() {
        let dir = resolve_learning_dir(
            Some(OsString::from("/tmp/xdg-state-probe")),
            Some(OsString::from("/home/someone")),
        )
        .expect("dir");
        assert!(dir.starts_with("/tmp/xdg-state-probe"));
        assert!(dir.ends_with("deep-diff-forge/learning"));
    }

    #[test]
    fn resolver_falls_back_to_home_local_state() {
        let dir = resolve_learning_dir(None, Some(OsString::from("/home/someone"))).expect("dir");
        assert_eq!(
            dir,
            PathBuf::from("/home/someone/.local/state/deep-diff-forge/learning")
        );
    }

    #[test]
    fn resolver_ignores_empty_xdg() {
        let dir =
            resolve_learning_dir(Some(OsString::new()), Some(OsString::from("/home/someone")))
                .expect("dir");
        assert!(dir.starts_with("/home/someone/.local/state"));
    }

    #[test]
    fn resolver_errors_when_nothing_set() {
        assert!(matches!(
            resolve_learning_dir(None, None),
            Err(LearningError::NoStateDir)
        ));
        assert!(matches!(
            resolve_learning_dir(Some(OsString::new()), Some(OsString::new())),
            Err(LearningError::NoStateDir)
        ));
    }
}
