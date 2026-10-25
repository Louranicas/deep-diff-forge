// DDF integration: R02, R03, R04, R05, R06, R07, R44, R46. Preserve source bytes and exact original input independently of normalized display; use Git as the application oracle.
// Design/status: docs/RECOMMENDATION_INTEGRATION.md#patch-truth
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fdeep-diff-forge-patch--lib
//! Canonical patch layer for Deep-Diff-Forge.
//!
//! This crate is the L1 keystone of the deployment maturity ladder: patch truth
//! is upstream of every other feature (Module Structure Plan, design rule 2).
//! It parses unified and Git-format patches into the stable
//! [`deep_diff_forge_core`] model, renders an apply-able patch back from that
//! model, and projects the model into a stable JSON document for Claude Code,
//! CI, and Bash consumers.
//!
//! Native Git input goes through [`acquire_git`]: paths, statuses, modes and
//! blob IDs come from Git's NUL-delimited `--raw -z` records, each paired with
//! the exact patch block it describes ([`StructuredDiff`]); the patch bytes
//! stay the authoritative artifact.
//!
//! # Trust boundary
//!
//! Patch input is untrusted. Parsing is panic-free on malformed input, returns
//! typed [`PatchParseError`] values instead of unwrapping, and bounds input
//! size with an explicit byte budget so a pathological input degrades instead
//! of exhausting memory.

mod git_acquire;
mod git_raw;
mod parser;
mod paths;
mod render;

#[cfg(feature = "json")]
mod json;

pub use git_acquire::{GitAcquireOptions, GitAcquisition, GitSelection, acquire as acquire_git};
pub use git_raw::{
    GitAcquireError, GitChange, GitChangeKind, GitChangeOrigin, GitPath, StructuredDiff,
};
pub use parser::{ParseOptions, parse, parse_with};
pub use render::render_unified;

#[cfg(feature = "json")]
pub use json::{to_json, to_json_sealed};

/// Default maximum input size accepted by the parser, in bytes.
///
/// Inputs larger than this are rejected with [`PatchParseError::BudgetExceeded`]
/// rather than parsed, preserving the trust-boundary guarantee that a
/// pathological input cannot exhaust memory by accident.
pub const DEFAULT_BYTE_BUDGET: usize = 64 * 1024 * 1024;

/// Maximum number of physical lines accepted in one patch.
///
/// A byte budget alone is insufficient: a patch made of millions of tiny lines
/// expands into substantially larger per-line model objects. This structural
/// budget bounds that amplification before any model object for the excess
/// line is allocated.
pub const DEFAULT_LINE_BUDGET: usize = 1_000_000;

/// Typed, non-panicking errors produced while parsing a patch.
///
/// Patch parsing is allowed to be a hard failure: when no apply-able patch
/// truth can be produced from the input, the engine must report the failure
/// rather than fabricate a diff. Every variant carries enough context for a
/// caller to act without scraping prose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatchParseError {
    /// EOF marker is malformed, duplicated or not immediately after a content line.
    InvalidEofMarker { line_number: usize },
    /// Combined multi-parent or mixed text/binary sections need a different model.
    UnsupportedFormat { line_number: usize },
    /// A quoted Git path was malformed or was not UTF-8.
    InvalidPath { line_number: usize },
    /// The input exceeded the configured byte budget and was not parsed.
    BudgetExceeded {
        /// Configured limit in bytes.
        limit_bytes: usize,
        /// Observed input size in bytes.
        actual_bytes: usize,
    },
    /// The patch contains more physical lines than the structural budget.
    LineBudgetExceeded {
        /// Configured maximum line count.
        limit_lines: usize,
        /// First observed count over the limit.
        actual_lines: usize,
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
    /// A hunk ended (at a new header, a new file, or end of input) before its
    /// declared old/new line counts were satisfied — the hunk is truncated, so
    /// no apply-able patch truth can be produced from it.
    TruncatedHunk {
        /// 1-based line number where the hunk's content ended.
        line_number: usize,
        /// Old-side lines still expected by the header count.
        remaining_old: u32,
        /// New-side lines still expected by the header count.
        remaining_new: u32,
    },
    /// A hunk body line would exceed the side count declared by its header
    /// (e.g. an added line after the new-count is already satisfied). The hunk
    /// does not match its own `@@ -a,b +c,d @@` contract.
    HunkLineCountMismatch {
        /// 1-based line number of the offending body line.
        line_number: usize,
        /// Raw text of the offending body line.
        text: String,
    },
}

impl std::fmt::Display for PatchParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidEofMarker { line_number } => {
                write!(f, "invalid EOF marker at line {line_number}")
            }
            Self::UnsupportedFormat { line_number } => {
                write!(f, "unsupported patch format at line {line_number}")
            }
            Self::InvalidPath { line_number } => {
                write!(f, "invalid or non-UTF-8 Git path at line {line_number}")
            }
            Self::BudgetExceeded {
                limit_bytes,
                actual_bytes,
            } => write!(
                f,
                "patch input is {actual_bytes} bytes, exceeding the {limit_bytes} byte budget"
            ),
            Self::LineBudgetExceeded {
                limit_lines,
                actual_lines,
            } => write!(
                f,
                "patch contains at least {actual_lines} lines, exceeding the {limit_lines}-line budget"
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
            Self::TruncatedHunk {
                line_number,
                remaining_old,
                remaining_new,
            } => write!(
                f,
                "truncated hunk ending at line {line_number}: {remaining_old} old / {remaining_new} new line(s) declared but missing"
            ),
            Self::HunkLineCountMismatch { line_number, text } => write!(
                f,
                "hunk body line at line {line_number} exceeds the count declared by its header: {text:?}"
            ),
        }
    }
}

impl std::error::Error for PatchParseError {}

/// Immutable capture of original patch truth, independent of normalized rendering.
/// See `docs/RECOMMENDATION_INTEGRATION.md#patch-truth` (R02–R07).
#[derive(Debug, Clone)]
pub struct CapturedPatch {
    raw: String,
    digest: String,
    files: Vec<deep_diff_forge_core::ReviewFile>,
}
impl CapturedPatch {
    /// Parse and seal the same bytes. No mutable access to the captured model.
    ///
    /// # Errors
    /// Returns the parser's typed error without retaining an invalid artifact.
    pub fn capture(raw: String) -> Result<Self, PatchParseError> {
        let files = parse(&raw)?;
        let digest = deep_diff_forge_core::sha256_hex(raw.as_bytes());
        Ok(Self { raw, digest, files })
    }
    /// Original input, byte for byte (including cosmetic headings and final LF state).
    #[must_use]
    pub fn original(&self) -> &str {
        &self.raw
    }
    /// SHA-256 of the original bytes.
    #[must_use]
    pub fn digest(&self) -> &str {
        &self.digest
    }
    /// Immutable parsed patch view; enrich a separate copy for presentation.
    #[must_use]
    pub fn files(&self) -> &[deep_diff_forge_core::ReviewFile] {
        &self.files
    }
}
