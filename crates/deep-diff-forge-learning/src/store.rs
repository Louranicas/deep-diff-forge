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
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use crate::error::LearningError;
use crate::receipt::StrategyReceipt;

/// Subdirectory under the resolved state dir.
const LEARNING_SUBDIR: &str = "deep-diff-forge/learning";
/// Receipts file name, relative to the learning dir.
const RECEIPTS_FILE: &str = "receipts/strategy.jsonl";

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
    if let Some(base) = xdg_state_home {
        if !base.is_empty() {
            return Ok(PathBuf::from(base).join(LEARNING_SUBDIR));
        }
    }
    if let Some(home) = home {
        if !home.is_empty() {
            return Ok(PathBuf::from(home)
                .join(".local/state")
                .join(LEARNING_SUBDIR));
        }
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
    let path = receipts_path(dir);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
        // The privacy contract ("local-only … prefer hashes, counts, timings")
        // is load-bearing: enforce owner-private (0o700) directories rather than
        // inheriting the process umask. Failing to secure them is an error, not
        // silently accepted — the store must not exist world-readable.
        secure_dir(dir)?;
        secure_dir(parent)?;
    }
    let mut line = receipt.to_json()?;
    line.push('\n');
    let mut opts = fs::OpenOptions::new();
    opts.create(true).append(true);
    // Create the JSONL owner-read/write only (0o600). `mode` applies on create;
    // an existing file keeps the mode set when it was first created.
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        opts.mode(0o600);
    }
    let mut file = opts.open(&path)?;
    file.write_all(line.as_bytes())?;
    Ok(())
}

/// Tighten `dir` to owner-only (`0o700`) on Unix. No-op on other platforms.
///
/// # Errors
/// Returns an I/O error if the permissions cannot be set.
fn secure_dir(dir: &Path) -> Result<(), LearningError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(not(unix))]
    {
        let _ = dir; // perms model differs; rely on the user profile directory.
    }
    Ok(())
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
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file = fs::File::open(&path)?;
    let reader = BufReader::new(file);
    let mut receipts = Vec::new();
    for line in reader.lines() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
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
    if !path.exists() {
        return Ok(0);
    }
    let file = fs::File::open(&path)?;
    let reader = BufReader::new(file);
    let mut n = 0;
    for line in reader.lines() {
        if !line?.trim().is_empty() {
            n += 1;
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
        StrategyReceipt::new("hash", "rust", "v", s)
            .with_cache(CacheState::Hit)
            .with_outcome(outcome, false)
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
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let r = receipt(Strategy::Patch, ReviewOutcome::Accepted);
        let body = format!("\n{}\n\n", r.to_json().unwrap());
        fs::write(&path, body).unwrap();
        assert_eq!(load_receipts(&dir).expect("load"), vec![r]);
    }

    #[test]
    fn load_surfaces_corruption_rather_than_dropping() {
        let dir = temp_dir();
        let _g = Scratch(dir.clone());
        let path = receipts_path(&dir);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "this is not json\n").unwrap();
        assert!(load_receipts(&dir).is_err());
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
