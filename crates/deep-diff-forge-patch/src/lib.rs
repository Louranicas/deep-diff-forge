//! Canonical patch layer for Deep-Diff-Forge.
//!
//! This crate is the L1 keystone of the deployment maturity ladder: patch truth
//! is upstream of every other feature (Module Structure Plan, design rule 2).
//! It parses unified and Git-format patches into the stable
//! [`deep_diff_forge_core`] model, renders an apply-able patch back from that
//! model, and projects the model into a stable JSON document for Claude Code,
//! CI, and Bash consumers.
//!
//! # Trust boundary
//!
//! Patch input is untrusted. Parsing is panic-free on malformed input, returns
//! typed [`PatchParseError`] values instead of unwrapping, and bounds input
//! size with an explicit byte budget so a pathological input degrades instead
//! of exhausting memory.

mod parser;
mod render;

#[cfg(feature = "json")]
mod json;

pub use parser::{ParseOptions, parse, parse_with};
pub use render::render_unified;

#[cfg(feature = "json")]
pub use json::to_json;

/// Default maximum input size accepted by the parser, in bytes.
///
/// Inputs larger than this are rejected with [`PatchParseError::BudgetExceeded`]
/// rather than parsed, preserving the trust-boundary guarantee that a
/// pathological input cannot exhaust memory by accident.
pub const DEFAULT_BYTE_BUDGET: usize = 64 * 1024 * 1024;

/// Typed, non-panicking errors produced while parsing a patch.
///
/// Patch parsing is allowed to be a hard failure: when no apply-able patch
/// truth can be produced from the input, the engine must report the failure
/// rather than fabricate a diff. Every variant carries enough context for a
/// caller to act without scraping prose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatchParseError {
    /// The input exceeded the configured byte budget and was not parsed.
    BudgetExceeded {
        /// Configured limit in bytes.
        limit_bytes: usize,
        /// Observed input size in bytes.
        actual_bytes: usize,
    },
    /// A hunk header (`@@ -a,b +c,d @@`) could not be parsed.
    MalformedHunkHeader {
        /// 1-based line number of the offending header.
        line_number: usize,
        /// Raw text of the offending header.
        text: String,
    },
    /// A diff body line appeared before any hunk header opened a hunk.
    BodyLineOutsideHunk {
        /// 1-based line number of the offending body line.
        line_number: usize,
        /// Raw text of the offending body line.
        text: String,
    },
}

impl std::fmt::Display for PatchParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BudgetExceeded {
                limit_bytes,
                actual_bytes,
            } => write!(
                f,
                "patch input is {actual_bytes} bytes, exceeding the {limit_bytes} byte budget"
            ),
            Self::MalformedHunkHeader { line_number, text } => {
                write!(f, "malformed hunk header at line {line_number}: {text:?}")
            }
            Self::BodyLineOutsideHunk { line_number, text } => {
                write!(
                    f,
                    "diff body line outside any hunk at line {line_number}: {text:?}"
                )
            }
        }
    }
}

impl std::error::Error for PatchParseError {}
