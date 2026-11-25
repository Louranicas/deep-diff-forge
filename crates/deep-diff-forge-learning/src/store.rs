// DDF review route: DDF-R38. Status and remaining work are recorded in the linked ledger; this comment does not claim delivery.
// Design/status: docs/RECOMMENDATION_INTEGRATION.md#remaining-module-routes
// Module index: docs/RECOMMENDATION_INTEGRATION.md#remaining-module-routes
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
use std::io::{BufRead, BufReader, Read, Seek as _, SeekFrom, Write};
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
/// `$HOME/.local/state/deep-diff-forge/learning`. Empty and relative values are
/// ignored: the XDG Base Directory spec makes a relative path invalid, and one
/// would put the store under the working directory, often the repository under
/// review.
/// Factored out so it is testable without mutating process-global environment
/// (which is `unsafe` under edition 2024 and would couple parallel tests).
///
/// # Errors
/// Returns [`LearningError::NoStateDir`] if neither value is usable.
pub fn resolve_learning_dir(
    xdg_state_home: Option<OsString>,
    home: Option<OsString>,
) -> Result<PathBuf, LearningError> {
    // An empty path is not absolute, so one check refuses both.
    if let Some(base) = xdg_state_home
        && Path::new(&base).is_absolute()
    {
        return Ok(PathBuf::from(base).join(LEARNING_SUBDIR));
    }
    if let Some(home) = home
        && Path::new(&home).is_absolute()
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
/// `$HOME` is set to an absolute path.
#[expect(
    clippy::disallowed_methods,
    reason = "the learning-store door: XDG_STATE_HOME and HOME locate the store"
)]
pub fn learning_dir() -> Result<PathBuf, LearningError> {
    resolve_learning_dir(std::env::var_os("XDG_STATE_HOME"), std::env::var_os("HOME"))
}

/// Path to the receipts JSONL file under `dir`.
#[must_use]
pub fn receipts_path(dir: &Path) -> PathBuf {
    dir.join(RECEIPTS_FILE)
}

/// Append one receipt as a JSONL line under `dir`, creating directories as
/// needed. Append-only: existing receipts are never rewritten. A store whose
/// last byte is not a newline (a torn write, or a hand edit that dropped the
/// final newline) gets one first, so the new receipt is its own line and is
/// never glued onto the last one.
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
    if current_len > 0 {
        // Writes still go to the end (O_APPEND); the seek only reads the last byte.
        let mut last = [0_u8];
        file.seek(SeekFrom::End(-1))?;
        file.read_exact(&mut last)?;
        if last != *b"\n" {
            line.insert(0, '\n');
        }
    }
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
    let token_valid = |value: &str, maximum: usize| {
        !value.is_empty()
            && value.len() <= maximum
            && value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'+' | b'@')
            })
    };
    if !token_valid(&receipt.language, 64) {
        return Err(LearningError::InvalidReceipt(
            "language must be a bounded identifier token",
        ));
    }
    if !token_valid(&receipt.parser_version, 128) {
        return Err(LearningError::InvalidReceipt(
            "parser_version must be a bounded identifier token",
        ));
    }
    // Anchor C-11: a receipt whose outcome is "revisited" but whose revisit flag says it was not
    // contradicts itself; scoring would count the revisit in one rate and not the other.
    if receipt.outcome == crate::ReviewOutcome::Revisited && !receipt.revisited {
        return Err(LearningError::InvalidReceipt(
            "outcome revisited contradicts revisited = false",
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
        let file = OpenOptions::new().read(true).append(true).open(path)?;
        validate_receipts_file(&path_metadata, &file)?;
        return Ok(file);
    }

    let mut options = OpenOptions::new();
    options.create_new(true).read(true).append(true);
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
/// silently dropping data, so corruption is visible rather than masked. Every
/// record must also pass the privacy checks [`append_receipt`] applies and hold
/// only receipt fields, so a hand-edited line holding a path, in a field or in
/// an extra key, is refused, naming its line (anchor A-05).
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
    let mut number = 0_usize;
    loop {
        line.clear();
        let read = read_capped_line(&mut reader, &mut line)?;
        if read == 0 {
            break;
        }
        number += 1;
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
        let receipt = StrategyReceipt::from_stored_json(trimmed)
            .and_then(|receipt| validate_receipt_for_storage(&receipt).map(|()| receipt))
            .map_err(|error| {
                LearningError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("receipt store line {number}: {error}"),
                ))
            })?;
        receipts.push(receipt);
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
    use deep_diff_forge_testkit::Scratch;

    fn receipt(s: Strategy, outcome: ReviewOutcome) -> StrategyReceipt {
        StrategyReceipt::new(crate::redacted_id("x"), "rust", "v", s)
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
        let scratch = Scratch::new("learn-store");
        let dir = scratch.join("store");
        assert_eq!(load_receipts(&dir).expect("load"), Vec::new());
    }

    #[test]
    fn count_missing_store_is_zero() {
        let scratch = Scratch::new("learn-store");
        let dir = scratch.join("store");
        assert_eq!(count_receipts(&dir).expect("count"), 0);
    }

    #[test]
    fn append_then_load_round_trips() {
        let scratch = Scratch::new("learn-store");
        let dir = scratch.join("store");
        let r = receipt(Strategy::Syntax, ReviewOutcome::Accepted);
        append_receipt(&dir, &r).expect("append");
        let loaded = load_receipts(&dir).expect("load");
        assert_eq!(loaded, vec![r]);
    }

    #[cfg(unix)]
    #[test]
    fn store_is_owner_private() {
        use std::os::unix::fs::PermissionsExt as _;
        let scratch = Scratch::new("learn-store");
        let dir = scratch.join("store");
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
        let scratch = Scratch::new("learn-store");
        let dir = scratch.join("store");
        append_receipt(&dir, &receipt(Strategy::Patch, ReviewOutcome::Accepted)).expect("a");
        append_receipt(&dir, &receipt(Strategy::Syntax, ReviewOutcome::Rejected)).expect("b");
        append_receipt(&dir, &receipt(Strategy::Hybrid, ReviewOutcome::Skipped)).expect("c");
        assert_eq!(load_receipts(&dir).expect("load").len(), 3);
        assert_eq!(count_receipts(&dir).expect("count"), 3);
    }

    #[test]
    fn append_creates_nested_dirs() {
        let scratch = Scratch::new("learn-store");
        let dir = scratch.join("store");
        assert!(!receipts_path(&dir).exists());
        append_receipt(&dir, &receipt(Strategy::Patch, ReviewOutcome::Accepted)).expect("append");
        assert!(receipts_path(&dir).exists());
    }

    #[test]
    fn load_skips_blank_lines() {
        let scratch = Scratch::new("learn-store");
        let dir = scratch.join("store");
        let path = receipts_path(&dir);
        let r = receipt(Strategy::Patch, ReviewOutcome::Accepted);
        let body = format!("\n{}\n\n", r.to_json().unwrap());
        write_private(&path, body.as_bytes());
        assert_eq!(load_receipts(&dir).expect("load"), vec![r]);
    }

    #[test]
    fn load_surfaces_corruption_rather_than_dropping() {
        let scratch = Scratch::new("learn-store");
        let dir = scratch.join("store");
        let path = receipts_path(&dir);
        write_private(&path, b"this is not json\n");
        assert!(load_receipts(&dir).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn append_rejects_symlink_directory_without_chmodding_target() {
        use std::os::unix::fs::PermissionsExt as _;
        let scratch = Scratch::new("learn-store");
        let target = scratch.join("target");
        let link = scratch.join("link");
        fs::create_dir_all(&target).unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o755)).unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let result = append_receipt(&link, &receipt(Strategy::Patch, ReviewOutcome::Accepted));
        assert!(result.is_err());
        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o755
        );
    }

    #[cfg(unix)]
    #[test]
    fn read_and_append_reject_receipt_file_symlink() {
        let scratch = Scratch::new("learn-store");
        let dir = scratch.join("store");
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
        let scratch = Scratch::new("learn-store");
        let dir = scratch.join("store");
        let path = receipts_path(&dir);
        write_private(&path, b"\n");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(load_receipts(&dir).is_err());
    }

    #[test]
    fn read_rejects_oversized_sparse_store() {
        let scratch = Scratch::new("learn-store");
        let dir = scratch.join("store");
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

    /// The budgets are part of the store's contract: 1 MiB per record, 64 MiB on disk and
    /// 100,000 records.
    #[test]
    fn the_store_budgets_are_the_documented_ones() {
        assert_eq!(
            (MAX_RECEIPT_LINE_BYTES, MAX_RECEIPTS_BYTES, MAX_RECEIPTS),
            (1_048_576, 67_108_864, 100_000)
        );
    }

    /// The per-record budget is inclusive: a line of exactly `MAX_RECEIPT_LINE_BYTES` (newline
    /// included) is read, one byte more is refused by name, on both read paths.
    #[test]
    fn the_per_record_budget_is_inclusive_on_read() {
        let scratch = Scratch::new("learn-store");
        let dir = scratch.join("store");
        let path = receipts_path(&dir);
        let mut line = vec![b'x'; MAX_RECEIPT_LINE_BYTES - 1];
        line.push(b'\n');
        write_private(&path, &line);
        assert_eq!(count_receipts(&dir).unwrap(), 1);
        line.insert(0, b'x');
        write_private(&path, &line);
        for error in [
            count_receipts(&dir).unwrap_err().to_string(),
            load_receipts(&dir).unwrap_err().to_string(),
        ] {
            assert!(error.contains("per-record budget"), "{error}");
        }
    }

    /// The record-count budget is inclusive: `MAX_RECEIPTS` records load and count, one more is
    /// refused by name. Blank lines are not records.
    #[test]
    fn the_record_count_budget_is_inclusive() {
        let scratch = Scratch::new("learn-store");
        let dir = scratch.join("store");
        let path = receipts_path(&dir);
        let mut one = receipt(Strategy::Patch, ReviewOutcome::Accepted)
            .to_json()
            .unwrap();
        one.push('\n');
        let mut body = one.repeat(MAX_RECEIPTS);
        body.push_str("\n\n");
        write_private(&path, body.as_bytes());
        assert_eq!(count_receipts(&dir).unwrap(), MAX_RECEIPTS);
        assert_eq!(load_receipts(&dir).unwrap().len(), MAX_RECEIPTS);
        body.push_str(&one);
        write_private(&path, body.as_bytes());
        for error in [
            count_receipts(&dir).unwrap_err().to_string(),
            load_receipts(&dir).unwrap_err().to_string(),
        ] {
            assert!(error.contains("record-count budget"), "{error}");
        }
    }

    /// An append that would take the store past its on-disk budget is refused, and the store is
    /// left as it was. A store exactly at the budget is still readable.
    #[test]
    fn append_refuses_to_grow_the_store_past_its_budget() {
        let scratch = Scratch::new("learn-store");
        let dir = scratch.join("store");
        append_receipt(&dir, &receipt(Strategy::Patch, ReviewOutcome::Accepted)).unwrap();
        let path = receipts_path(&dir);
        OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(MAX_RECEIPTS_BYTES)
            .unwrap();
        let error = append_receipt(&dir, &receipt(Strategy::Patch, ReviewOutcome::Accepted))
            .unwrap_err()
            .to_string();
        assert!(error.contains("on-disk size budget"), "{error}");
        assert_eq!(fs::metadata(&path).unwrap().len(), MAX_RECEIPTS_BYTES);
    }

    /// Blank padding: `bytes` bytes of whitespace lines, each within the per-record budget.
    fn blank_padding(bytes: usize) -> Vec<u8> {
        // A budget mutated to a few bytes would make this loop run 64M times; refuse to build.
        const { assert!(MAX_RECEIPT_LINE_BYTES >= 1024) };
        let mut body = Vec::with_capacity(bytes);
        while body.len() < bytes {
            let line = (bytes - body.len()).min(MAX_RECEIPT_LINE_BYTES);
            body.extend(std::iter::repeat_n(b' ', line - 1));
            body.push(b'\n');
        }
        body
    }

    /// The on-disk budget is inclusive on read: a store of exactly `MAX_RECEIPTS_BYTES` is read
    /// whole by both read paths.
    #[test]
    fn a_store_exactly_at_the_size_budget_is_readable() {
        let scratch = Scratch::new("learn-store");
        let dir = scratch.join("store");
        let path = receipts_path(&dir);
        let size = usize::try_from(MAX_RECEIPTS_BYTES).unwrap();
        write_private(&path, &blank_padding(size));
        assert_eq!(fs::metadata(&path).unwrap().len(), MAX_RECEIPTS_BYTES);
        assert_eq!(count_receipts(&dir).unwrap(), 0);
        assert_eq!(load_receipts(&dir).unwrap().len(), 0);
    }

    /// The on-disk budget is inclusive on append: an append that lands exactly on it succeeds.
    #[test]
    fn an_append_that_reaches_the_size_budget_exactly_is_kept() {
        let scratch = Scratch::new("learn-store");
        let dir = scratch.join("store");
        let path = receipts_path(&dir);
        let new = receipt(Strategy::Patch, ReviewOutcome::Accepted);
        let line = new.to_json().unwrap().len() + 1;
        let size = usize::try_from(MAX_RECEIPTS_BYTES).unwrap();
        write_private(&path, &blank_padding(size - line));
        append_receipt(&dir, &new).unwrap();
        assert_eq!(fs::metadata(&path).unwrap().len(), MAX_RECEIPTS_BYTES);
        assert_eq!(load_receipts(&dir).unwrap(), vec![new]);
    }

    /// Anchor C-07: an unredacted identity cannot be built at all; the JSON form of one is refused
    /// when it is read, by name. Before, only the store checked, after construction.
    #[test]
    fn an_unredacted_file_identity_cannot_be_read() {
        let good = StrategyReceipt::new(crate::redacted_id("x"), "rust", "v1", Strategy::Patch)
            .to_json()
            .unwrap();
        for bad in [
            "secret/source/path.rs",
            "0123456789ABCDEF",
            "0123456789abcde",
            "",
        ] {
            let json = good.replace(&crate::redacted_id("x").to_string(), bad);
            let error = StrategyReceipt::from_json(&json).unwrap_err().to_string();
            assert!(error.contains("redacted id"), "{bad}: {error}");
        }
        assert!(StrategyReceipt::from_json(&good).is_ok());
    }

    /// Anchor A-05: a hand-edited store cannot smuggle a path past the privacy contract; before,
    /// only `append_receipt` checked it and `load_receipts` accepted any parseable line.
    #[test]
    fn load_refuses_records_that_append_would_refuse() {
        let scratch = Scratch::new("learn-store");
        let dir = scratch.join("store");
        append_receipt(&dir, &receipt(Strategy::Patch, ReviewOutcome::Accepted)).unwrap();
        assert_eq!(load_receipts(&dir).unwrap().len(), 1);
        let good = fs::read(receipts_path(&dir)).unwrap();
        let unredacted =
            StrategyReceipt::new(crate::redacted_id("x"), "rust", "v1", Strategy::Patch)
                .to_json()
                .unwrap()
                .replace(&crate::redacted_id("x").to_string(), "/home/x/secret.rs");
        let path_language = StrategyReceipt::new(
            crate::redacted_id("x"),
            "../../secret",
            "v1",
            Strategy::Patch,
        )
        .to_json()
        .unwrap();
        let extra_key =
            StrategyReceipt::new(crate::redacted_id("x"), "rust", "v1", Strategy::Patch)
                .to_json()
                .unwrap()
                .replacen(
                    '{',
                    r#"{"path":"secret/src/payments.rs","source":"fn f() {}","#,
                    1,
                );
        for bad in [unredacted, path_language, extra_key] {
            let mut store = good.clone();
            store.extend_from_slice(bad.as_bytes());
            store.push(b'\n');
            write_private(&receipts_path(&dir), &store);
            let error = load_receipts(&dir).unwrap_err().to_string();
            assert!(error.contains("receipt store line 2: "), "{error}");
        }
    }

    /// Anchor C-11: a self-contradicting receipt is refused on append and on load.
    #[test]
    fn a_revisited_outcome_without_the_revisit_flag_is_refused() {
        let scratch = Scratch::new("learn-store");
        let dir = scratch.join("store");
        let contradictory =
            StrategyReceipt::new(crate::redacted_id("x"), "rust", "v1", Strategy::Patch)
                .with_outcome(ReviewOutcome::Revisited, false);
        let error = append_receipt(&dir, &contradictory)
            .unwrap_err()
            .to_string();
        assert!(error.contains("contradicts"), "{error}");
        let consistent = contradictory
            .clone()
            .with_outcome(ReviewOutcome::Revisited, true);
        append_receipt(&dir, &consistent).unwrap();
        let mut store = fs::read(receipts_path(&dir)).unwrap();
        store.extend_from_slice(contradictory.to_json().unwrap().as_bytes());
        store.push(b'\n');
        write_private(&receipts_path(&dir), &store);
        let error = load_receipts(&dir).unwrap_err().to_string();
        assert!(
            error.contains("receipt store line 2: ") && error.contains("contradicts"),
            "{error}"
        );
    }

    /// A store whose last record lost its newline (a torn write, a hand edit) is readable; an
    /// append gives the new receipt its own line rather than gluing it onto the last one, which
    /// made the whole store unreadable while `learn record` reported success.
    #[test]
    fn append_after_a_missing_final_newline_keeps_every_receipt() {
        let scratch = Scratch::new("learn-store");
        let dir = scratch.join("store");
        let first = receipt(Strategy::Patch, ReviewOutcome::Accepted);
        let second = receipt(Strategy::Syntax, ReviewOutcome::Rejected);
        append_receipt(&dir, &first).unwrap();
        let path = receipts_path(&dir);
        let mut store = fs::read(&path).unwrap();
        assert_eq!(store.pop(), Some(b'\n'));
        write_private(&path, &store);
        assert_eq!(load_receipts(&dir).unwrap(), vec![first.clone()]);
        append_receipt(&dir, &second).unwrap();
        assert_eq!(load_receipts(&dir).unwrap(), vec![first, second.clone()]);
        assert_eq!(count_receipts(&dir).unwrap(), 2);
        // A torn tail stays its own (corrupt, visible) line; the receipt after it is whole.
        let torn = fs::read(&path).unwrap().len() - 20;
        OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(torn as u64)
            .unwrap();
        append_receipt(&dir, &second).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3, "{text}");
        assert_eq!(lines[2], second.to_json().unwrap());
        let error = load_receipts(&dir).unwrap_err().to_string();
        assert!(error.contains("receipt store line 2: "), "{error}");
        // A store that already ends in a newline gets no blank line.
        assert!(text.ends_with('\n') && !text.contains("\n\n"));
    }

    #[test]
    fn append_rejects_path_like_metadata_tokens() {
        let scratch = Scratch::new("learn-store");
        let dir = scratch.join("store");
        let receipt = StrategyReceipt::new(
            crate::redacted_id("x"),
            "../../secret",
            "v1",
            Strategy::Patch,
        );
        assert!(append_receipt(&dir, &receipt).is_err());
    }

    #[test]
    fn order_is_preserved() {
        let scratch = Scratch::new("learn-store");
        let dir = scratch.join("store");
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

    /// A relative value is invalid (XDG Base Directory spec): it would put the store under the
    /// working directory, often the repository under review.
    #[test]
    fn resolver_ignores_relative_values() {
        let dir = resolve_learning_dir(
            Some(OsString::from("relstate")),
            Some(OsString::from("/home/someone")),
        )
        .expect("dir");
        assert_eq!(
            dir,
            PathBuf::from("/home/someone/.local/state/deep-diff-forge/learning")
        );
        assert!(matches!(
            resolve_learning_dir(
                Some(OsString::from("./state")),
                Some(OsString::from("home"))
            ),
            Err(LearningError::NoStateDir)
        ));
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

    /// SECURITY.md (learning store): "Directory/file symlinks and inode replacement are
    /// rejected." The file half, deterministically: the store's own steps (the path's metadata
    /// first, the file opened after it) with the file replaced by another inode between them, as a
    /// rename over the path does. `validate_receipts_file` is where every door compares the two
    /// (read, append to an existing store, create), so the comparison is reached here directly
    /// rather than through a race; the other checks it makes (regular file, owner-private, owned
    /// by the user, size) all pass for the replacement, so only the identity check can refuse it.
    #[cfg(unix)]
    #[test]
    fn a_receipt_store_replaced_between_its_check_and_its_open_is_refused() {
        let scratch = Scratch::new("learn-store");
        let path = receipts_path(&scratch.join("store"));
        write_private(&path, b"\n");
        let checked = fs::symlink_metadata(&path).unwrap();
        // The control: the same steps with nothing replaced pass.
        validate_receipts_file(&checked, &File::open(&path).unwrap()).unwrap();
        let replacement = path.with_extension("replacement");
        write_private(&replacement, b"\n");
        fs::rename(&replacement, &path).unwrap();
        let opened = File::open(&path).unwrap();
        let refused = validate_receipts_file(&checked, &opened)
            .map_or_else(|error| error.to_string(), |()| "accepted".to_owned());
        assert!(
            refused.contains("receipt store changed while it was being opened"),
            "a store whose inode was replaced after its check: {refused}"
        );
    }

    /// Flip the symbolic link `link` between `to_a` and `to_b` until `stop` is set: each flip is
    /// one `rename` of a fresh link over `link`, so every path through `link` names an entry under
    /// `to_a` or one under `to_b`, never nothing. Returns the number of flips.
    #[cfg(unix)]
    fn flip_until(
        link: &Path,
        to_a: &Path,
        to_b: &Path,
        stop: &std::sync::atomic::AtomicBool,
    ) -> u64 {
        let fresh = link.with_extension("fresh");
        let mut flips = 0_u64;
        while !stop.load(std::sync::atomic::Ordering::Relaxed) {
            for target in [to_a, to_b] {
                let _ = fs::remove_file(&fresh);
                if std::os::unix::fs::symlink(target, &fresh).is_ok()
                    && fs::rename(&fresh, link).is_ok()
                {
                    flips += 1;
                }
            }
        }
        flips
    }

    /// How many times a race test calls the store's step against the flipping link.
    #[cfg(unix)]
    const RACE_ROUNDS: usize = 20_000;

    /// SECURITY.md (learning store): "Directory/file symlinks and inode replacement are
    /// rejected." The directory half, through a real race (the comparison of the directory's
    /// metadata with the opened directory is inside `ensure_secure_dir`, which has no seam). The
    /// store directory is reached through a link that a second thread flips between `real/`,
    /// where `store` is a directory, and `trap/`, where `store` is a link to `victim`. A check
    /// through `real/` followed by an open through `trap/` opens `victim`; only the identity check
    /// keeps `victim` from being made `0700`. `victim` itself is reachable only through the link
    /// `trap/store`, which the store refuses by its own metadata, so on correct code its mode
    /// never changes, however the race falls: green does not depend on the timing. Red does: it
    /// needs at least one open to land in the window, which is measured in the plant run of
    /// 0.11.1 (lane T2), not promised. The rounds are counted, not timed.
    #[cfg(unix)]
    #[test]
    fn a_directory_replaced_between_its_check_and_its_open_is_not_secured() {
        use std::os::unix::fs::PermissionsExt as _;
        use std::sync::atomic::{AtomicBool, Ordering};
        let scratch = Scratch::new("learn-store");
        let (real, trap, victim) = (
            scratch.join("real"),
            scratch.join("trap"),
            scratch.join("victim"),
        );
        fs::create_dir_all(real.join("store")).unwrap();
        fs::create_dir_all(&trap).unwrap();
        fs::create_dir_all(&victim).unwrap();
        fs::set_permissions(&victim, fs::Permissions::from_mode(0o755)).unwrap();
        std::os::unix::fs::symlink(&victim, trap.join("store")).unwrap();
        let via = scratch.join("via");
        std::os::unix::fs::symlink(&real, &via).unwrap();
        let dir = via.join("store");
        let stop = AtomicBool::new(false);
        let (refused, flips) = std::thread::scope(|scope| {
            let flipper = scope.spawn(|| flip_until(&via, &real, &trap, &stop));
            let mut refused = 0_usize;
            for _ in 0..RACE_ROUNDS {
                if ensure_secure_dir(&dir).is_err_and(|error| {
                    error
                        .to_string()
                        .contains("learning directory changed while it was being secured")
                }) {
                    refused += 1;
                }
                if fs::metadata(&victim).unwrap().permissions().mode() & 0o777 != 0o755 {
                    break;
                }
            }
            stop.store(true, Ordering::Relaxed);
            (refused, flipper.join().unwrap())
        });
        eprintln!(
            "{RACE_ROUNDS} rounds, {flips} flips, {refused} replacements refused by identity"
        );
        assert_eq!(
            fs::metadata(&victim).unwrap().permissions().mode() & 0o777,
            0o755,
            "a directory reached by replacing the store directory after its check was secured \
             ({refused} replacements were refused by identity first)"
        );
    }

    /// SECURITY.md (learning store): "New receipt files use exclusive creation." A store that is
    /// absent when it is checked is created with `create_new`: if anything appears at the path
    /// before the creation (here a link to a file that does not exist), the creation fails rather
    /// than opening or creating what the link names. Through a real race, as above: the path is
    /// reached through a link flipped between `empty/`, where there is no store, and `trap/`,
    /// where the store is a dangling link to `elsewhere`. On correct code `elsewhere` is never
    /// created, however the race falls; with a plain `create` the creation follows the link and
    /// makes it. The rounds are counted, not timed; red needs one creation in the window
    /// (measured in lane T2's plant run, not promised).
    #[cfg(unix)]
    #[test]
    fn a_link_that_appears_at_the_creation_door_is_never_followed() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let scratch = Scratch::new("learn-store");
        let (empty, trap, elsewhere) = (
            scratch.join("empty"),
            scratch.join("trap"),
            scratch.join("elsewhere.jsonl"),
        );
        fs::create_dir_all(&empty).unwrap();
        fs::create_dir_all(&trap).unwrap();
        std::os::unix::fs::symlink(&elsewhere, trap.join("strategy.jsonl")).unwrap();
        let via = scratch.join("via");
        std::os::unix::fs::symlink(&empty, &via).unwrap();
        let path = via.join("strategy.jsonl");
        let stop = AtomicBool::new(false);
        let (exclusive, created, flips) = std::thread::scope(|scope| {
            let flipper = scope.spawn(|| flip_until(&via, &empty, &trap, &stop));
            let (mut exclusive, mut created) = (0_usize, 0_usize);
            for _ in 0..RACE_ROUNDS {
                match open_receipts_for_append(&path) {
                    Ok(_) => created += 1,
                    Err(LearningError::Io(error))
                        if error.kind() == std::io::ErrorKind::AlreadyExists =>
                    {
                        exclusive += 1;
                    }
                    Err(_) => {}
                }
                // A store created in `empty/` is removed, so the next round meets no store there.
                let _ = fs::remove_file(empty.join("strategy.jsonl"));
                if fs::symlink_metadata(&elsewhere).is_ok() {
                    break;
                }
            }
            stop.store(true, Ordering::Relaxed);
            (exclusive, created, flipper.join().unwrap())
        });
        eprintln!(
            "{RACE_ROUNDS} rounds, {flips} flips, {created} stores created, {exclusive} creations refused as existing"
        );
        assert!(
            fs::symlink_metadata(&elsewhere).is_err(),
            "the store's creation followed a link that appeared at its path and created {}",
            elsewhere.display()
        );
    }

    /// SECURITY.md (learning store): "language and parser-version fields accept bounded token
    /// characters rather than paths." The parser version and both bounds, on append and on load:
    /// a path-like parser version is refused by name; a value at its bound (64 bytes for the
    /// language, 128 for the parser version) is kept and one byte more is refused. Before 0.11.1
    /// the parser-version check could be removed with the crate green, and no bound was tested
    /// (U1 audit, 2026-11-26).
    #[test]
    fn the_parser_version_and_both_bounds_hold_on_append_and_on_load() {
        let scratch = Scratch::new("learn-store");
        let dir = scratch.join("store");
        let with = |language: &str, version: &str| {
            StrategyReceipt::new(crate::redacted_id("x"), language, version, Strategy::Patch)
        };
        let refusal = |receipt: &StrategyReceipt| {
            append_receipt(&dir, receipt)
                .map_or_else(|error| error.to_string(), |()| "accepted".to_owned())
        };
        let version = "parser_version must be a bounded identifier token";
        let language = "language must be a bounded identifier token";
        for path in [
            "../../secret",
            "/etc/passwd",
            "src/secret.rs",
            "C:\\secret",
            "two words",
            "",
        ] {
            let error = refusal(&with("rust", path));
            assert!(error.contains(version), "parser version {path:?}: {error}");
        }
        let kept = [
            with("rust", &"v".repeat(128)),
            with(&"r".repeat(64), "v1"),
            with("rust", "tree-sitter-rust_0.24.1+git@abc"),
        ];
        for receipt in &kept {
            assert_eq!(refusal(receipt), "accepted", "{receipt:?}");
        }
        let error = refusal(&with("rust", &"v".repeat(129)));
        assert!(
            error.contains(version),
            "a parser version of 129 bytes: {error}"
        );
        let error = refusal(&with(&"r".repeat(65), "v1"));
        assert!(error.contains(language), "a language of 65 bytes: {error}");
        assert_eq!(load_receipts(&dir).unwrap(), kept.to_vec());
        // The read door applies the same checks to a hand-edited store.
        let good = fs::read(receipts_path(&dir)).unwrap();
        for (bad, why) in [
            (with("rust", "/etc/passwd"), version),
            (with("rust", &"v".repeat(129)), version),
            (with(&"r".repeat(65), "v1"), language),
        ] {
            let mut store = good.clone();
            store.extend_from_slice(bad.to_json().unwrap().as_bytes());
            store.push(b'\n');
            write_private(&receipts_path(&dir), &store);
            let error = load_receipts(&dir).unwrap_err().to_string();
            assert!(
                error.contains("receipt store line 4: ") && error.contains(why),
                "{bad:?}: {error}"
            );
        }
    }

    /// SECURITY.md (scope): "learning metadata supplied at the receipt boundary" is
    /// attacker-controlled. The boundary as `learn record --stdin` crosses it
    /// (`StrategyReceipt::from_json`, then `append_receipt`) and as a hand-edited store does
    /// (`load_receipts`): each hostile receipt is refused with its reason and the store's bytes are
    /// unchanged; an extra key is dropped at the record door and refused at the read door, and in
    /// neither case is its value stored. Before 0.11.1 the only falsifier cited for this sentence
    /// was a fuzz target that asserts no panic and a round trip (U1 audit, 2026-11-26).
    #[test]
    fn hostile_metadata_at_the_receipt_boundary_never_reaches_the_store() {
        let scratch = Scratch::new("learn-store");
        let dir = scratch.join("store");
        let good = r#"{"file_hash":"0123456789abcdef","language":"rust","parser_version":"v1","strategy":"syntax","fallback":null,"elapsed_ms":3,"bytes":10,"nodes":2,"cache":"cold","outcome":"accepted","revisited":false}"#;
        let record = |line: &str| {
            StrategyReceipt::from_json(line)
                .and_then(|receipt| append_receipt(&dir, &receipt))
                .map_or_else(|error| error.to_string(), |()| "recorded".to_owned())
        };
        assert_eq!(record(good), "recorded");
        let stored = fs::read(receipts_path(&dir)).unwrap();
        let hostile = [
            (
                good.replace("0123456789abcdef", "/home/x/secret.rs"),
                "redacted id",
            ),
            (
                good.replace("0123456789abcdef", "0123456789ABCDEF"),
                "redacted id",
            ),
            (
                good.replace(r#""language":"rust""#, r#""language":"../../secret""#),
                "language must be",
            ),
            (
                good.replace(
                    r#""parser_version":"v1""#,
                    r#""parser_version":"/etc/passwd""#,
                ),
                "parser_version must be",
            ),
            (
                good.replace(r#""outcome":"accepted""#, r#""outcome":"revisited""#),
                "contradicts",
            ),
            (
                good.replace(r#""strategy":"syntax""#, r#""strategy":"telepathy""#),
                "deserialize error",
            ),
        ];
        for (line, why) in &hostile {
            let error = record(line);
            assert!(error.contains(why), "recorded {line}: {error}");
            assert_eq!(
                fs::read(receipts_path(&dir)).unwrap(),
                stored,
                "the store changed after refusing {line}"
            );
        }
        let extra = good.replacen('{', r#"{"source":"fn secret() {}","#, 1);
        assert_eq!(record(&extra), "recorded");
        let text = fs::read_to_string(receipts_path(&dir)).unwrap();
        assert!(!text.contains("secret"), "an extra key was stored: {text}");
        // The read door: each hostile line, written into the store by hand, is refused by line.
        let stored = fs::read(receipts_path(&dir)).unwrap();
        for (line, why) in hostile
            .iter()
            .map(|(line, why)| (line.as_str(), *why))
            .chain([(extra.as_str(), "not a receipt field")])
        {
            let mut store = stored.clone();
            store.extend_from_slice(line.as_bytes());
            store.push(b'\n');
            write_private(&receipts_path(&dir), &store);
            let error = load_receipts(&dir).unwrap_err().to_string();
            assert!(
                error.contains("receipt store line 3: ") && error.contains(why),
                "loaded {line}: {error}"
            );
        }
    }
}
