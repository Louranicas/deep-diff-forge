// Review council: Public API boundary; separate typed review claims from controller-owned execution and human authority.
// Design/status: docs/REVIEW_COUNCIL.md#council-boundary
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fcouncil--crates--deep-diff-forge-review--src--lib
//! Review roster and adversarial council, with controller-owned verification.
//! Design: `docs/REVIEW_COUNCIL.md`. [Vault](obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FReview%20Council).
pub mod cli;
mod execution;
mod journal;
mod model;
mod mutation_scope;
mod policy;
mod sandbox;
mod storage;
mod tuicr;
pub use execution::{run_gate, run_reviewer};
pub use journal::{
    Actor, ActorBasis, Anchor, DecisionEvent, EventKind, ImportOptions, JOURNAL_FILE, Lifecycle,
    Link, LinkBasis, SessionRevision, TuicrImport, TuicrImportError, import_tuicr,
};
pub use model::*;
pub use policy::{assess, report_schema, validate_report, validate_request};
use std::{error::Error, fmt};
pub use storage::{SealedManifest, load_manifest, prepare, source_fingerprint};
pub use tuicr::{
    ExportOptions, NoteStatus, PlannedNote, SESSION_VERSION, SkippedReport, Target, TuicrExport,
    TuicrExportError, export_tuicr,
};
#[derive(Debug)]
pub struct ReviewFailure(pub String);
impl fmt::Display for ReviewFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl Error for ReviewFailure {}
impl From<std::io::Error> for ReviewFailure {
    fn from(e: std::io::Error) -> Self {
        Self(e.to_string())
    }
}
impl From<serde_json::Error> for ReviewFailure {
    fn from(e: serde_json::Error) -> Self {
        Self(e.to_string())
    }
}
pub(crate) type Result<T> = std::result::Result<T, ReviewFailure>;
pub(crate) fn require(ok: bool, message: impl Into<String>) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(ReviewFailure(message.into()))
    }
}
/// `value` as one path segment, refused unless it is an [`identifier`]. Every id joined into a
/// run-directory path goes through this, so the join itself cannot name a parent, a root or a
/// separator, whatever validated the id earlier (anchor C-09).
pub(crate) fn segment(value: &str) -> Result<&str> {
    require(identifier(value), "id is not a safe path segment")?;
    Ok(value)
}
pub(crate) fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

#[cfg(test)]
mod segment_tests {
    use super::segment;
    use std::path::Path;

    /// Anchor C-09: an id becomes a path segment only through `segment`, which refuses anything
    /// that could leave the run directory, whether or not the id was validated where it was parsed.
    #[test]
    fn only_identifiers_become_path_segments() {
        for bad in [
            "",
            "..",
            ".",
            "../escape",
            "a/b",
            "/abs",
            "a\\b",
            "x\0y",
            "é",
            &"a".repeat(81),
        ] {
            assert!(segment(bad).is_err(), "{bad:?}");
        }
        for good in [
            "review-security-claude-independent",
            "council_report",
            "a",
            &"a".repeat(80),
        ] {
            assert_eq!(segment(good).unwrap(), good);
        }
        let run = Path::new("/run");
        assert!(crate::storage::job_dir(run, "../reports").is_err());
        // Before, report_path joined any id: "../../x" named a file outside the run directory.
        assert!(crate::storage::report_path(run, "../../x", crate::Round::Independent).is_err());
        assert_eq!(
            crate::storage::job_dir(run, "gate-tests").unwrap(),
            Path::new("/run/jobs/gate-tests")
        );
    }
}
