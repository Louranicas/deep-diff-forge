// Review council: Public API boundary; separate typed review claims from controller-owned execution and human authority.
// Design/status: docs/REVIEW_COUNCIL.md#council-boundary
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fcouncil--crates--deep-diff-forge-review--src--lib
//! Review roster and adversarial council, with controller-owned verification.
//! Design: `docs/REVIEW_COUNCIL.md`. [Vault](obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FReview%20Council).
pub mod cli;
mod execution;
mod model;
mod policy;
mod storage;
pub use execution::{run_gate, run_reviewer};
pub use model::*;
pub use policy::{assess, report_schema, validate_report, validate_request};
use std::{error::Error, fmt};
pub use storage::{load_manifest, prepare, source_fingerprint};
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
pub(crate) fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}
