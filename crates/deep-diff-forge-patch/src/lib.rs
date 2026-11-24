// DDF integration: R02, R03, R04, R05, R06, R07, R44, R46. Preserve source bytes and exact original input independently of normalized display; use Git as the application oracle.
// Design/status: docs/RECOMMENDATION_INTEGRATION.md#patch-truth
// Module index: docs/RECOMMENDATION_INTEGRATION.md#patch-truth
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

mod decode;
mod git_acquire;
mod git_raw;
mod guards;
mod judge_patch;
mod parser;
mod paths;
mod render;
mod sealed;

#[cfg(feature = "json")]
mod json;

pub use decode::{DecodedPatch, decode};
pub use git_acquire::{
    GitAcquireOptions, GitAcquisition, GitSelection, RepositoryDirs, acquire as acquire_git,
    diff_files, git_version, repository_dirs,
};
pub use git_raw::{
    GitAcquireError, GitChange, GitChangeKind, GitChangeOrigin, GitPath, StructuredDiff,
};
pub use guards::{InputGuards, has_reviewable_hunks};
pub use judge_patch::{
    CommitAndParent, HeadState, JudgePatch, JudgePatchError, NAMED_COMMITS_CAP, Named, Opens,
    RefTips, Worktree, commit_patch, commits_beyond, dirty_submodules, file_at_commit,
    has_commits_beyond, has_uncommitted_changes, head_contains, head_state, judge_patch,
    judge_patch_probed, merge_base, named_commits, ref_tips, staged_patch, worktrees,
};
pub use parser::{ParseOptions, parse, parse_decoded, parse_with};
pub use render::{old_path, render_unified};
pub use sealed::{ReviewError, SealedReview};

#[cfg(feature = "json")]
pub use json::to_json;
#[cfg(feature = "json")]
pub(crate) use json::to_json_sealed;

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
///
/// `#[non_exhaustive]` since 0.8.0: a new refusal is a new variant, and matching code outside
/// this crate keeps a wildcard arm (or uses [`PatchParseError::exit_code`]), so adding one is
/// not a breaking change.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PatchParseError {
    /// EOF marker is malformed, duplicated or not immediately after a content line, or a later
    /// line adds to the side of the file it ended (Git would join that line onto the marked one).
    InvalidEofMarker { line_number: usize },
    /// Combined multi-parent or mixed text/binary sections need a different model, and the
    /// `rename old`/`rename new` headers no current Git writes are not translated.
    UnsupportedFormat { line_number: usize },
    /// A quoted Git path was malformed or was not UTF-8, a rename or copy names a path
    /// `git apply` refuses (absolute, or with a `.`, `..` or `.git` component), or a file
    /// section names no path at all, as with a `diff --git i/x w/y` mode change that has no
    /// `---`/`+++`, or a move header whose other side no header names (`copy from x` alone)
    /// (Git: "lacks filename information"); the line is the section's first.
    InvalidPath { line_number: usize },
    /// A mode header (`old mode`, `new mode`, `new file mode`, `deleted file mode`) is not one
    /// to six octal digits; `git apply` refuses it as an invalid mode. Also a section whose only
    /// content is a mode header that names no change once Git canonicalises it (`100644` to
    /// `100664`, or a lone `old mode`), which `git apply` refuses as "patch with only garbage".
    InvalidMode { line_number: usize },
    /// A file header out of place: an extended header after the file's `---`, `+++` or first
    /// hunk, a `+++` that does not follow a `---` at once, or a `---` with no `+++` after it.
    /// Git ignores such lines, so taking them as headers changed what the review showed.
    MisplacedHeader { line_number: usize },
    /// The headers contradict each other: `new file mode` with an old path that is not
    /// `/dev/null`, `deleted file mode` with a new path that is not, a repeated mode header (or an
    /// `index` mode beside one), or a section that is both a rename and a copy.
    InconsistentHeader { line_number: usize },
    /// The patch ends inside a hunk line that has no line feed, which Git refuses as corrupt.
    UnterminatedLine { line_number: usize },
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
    /// A hunk header in a place `git apply` does not read one as part of the file: not right
    /// after the file's `+++` (or a Git section's extended headers) and not right after the
    /// previous hunk's last line. Git ends the file's hunks at any other line, so a hunk after a
    /// blank or stray line is "patch fragment without header" to Git, and taking it as this
    /// file's would review lines Git refuses or applies elsewhere.
    MisplacedHunk {
        /// 1-based line number of the hunk header.
        line_number: usize,
    },
    /// A hunk that adds and removes nothing (only context lines, or none). `git apply` refuses
    /// it as a corrupt patch.
    HunkWithoutChange {
        /// 1-based line number where the hunk ended.
        line_number: usize,
    },
    /// A file section from which `git apply` reads no patch: a `diff --git` line with no
    /// extended header or hunk after it (Git: not a header), or a traditional `---`/`+++` pair
    /// with no hunk (Git: "No valid patches"). A Git section of `---`/`+++` alone is refused
    /// too, since the model could not render it back. So is a Git section with no hunk, no
    /// binary marker and no change in its headers, such as an `index` line alone (Git: "patch
    /// with only garbage"). The line is the section's first.
    SectionWithoutPatch {
        /// 1-based line number where the section began.
        line_number: usize,
    },
    /// One path is written or removed by more than one file section, in a way `git apply` does
    /// not carry out as the sequence the review would show. Git applies a patch as a set: every
    /// removal first, every creation after, and a rename or copy reads its source as it was
    /// before the patch. So "modify `x`", then "delete `x`" leaves `x` in place with the
    /// modified content. Two shapes are read, because Git can apply them in order when their
    /// hunks apply: a change in place after a section that wrote the path, and `new file` after
    /// `deleted file` for one path (a change of type). A single section that renames a path to
    /// itself removes and writes it, and is refused too (`same_section`).
    PathInSeveralSections {
        /// 1-based line number where the later of the two sections began (the section itself,
        /// with `same_section`).
        line_number: usize,
        /// The path, as the review would show it.
        path: String,
        /// The section at `line_number` renames `path` to itself, so it removes and writes it.
        same_section: bool,
    },
    /// A `diff --git` section whose header lines are not a shape Git's own diff writes: a
    /// combination or an order of `old mode`, `new mode`, `new file mode`, `deleted file mode`,
    /// `similarity index`, `rename`, `copy`, `index`, `---`/`+++` and binary lines that no Git
    /// writes, a line not in the form Git writes it (a mode other than `100644`, `100755`,
    /// `120000` or `160000`, an `index` id that is not hex, a score that is not a percentage),
    /// a `Files … differ` line, or a binary marker ending in a carriage return. The shapes Git
    /// writes are listed in the crate's `CONFORMANCE.md`; `similarity index`, `dissimilarity
    /// index` and `index` lines may be left out of them. Five rescores in a row found a header
    /// shape Git reads otherwise than the review did; DDF now reads no other header shape.
    HeaderNotWrittenByGit {
        /// 1-based line number of the header line refused.
        line_number: usize,
        /// Which header it is (`similarity index`, `---/+++`, …).
        header: &'static str,
        /// Why it is refused.
        reason: &'static str,
    },
    /// A hunk plain `git apply` places by an anchor its header does not determine: one with no
    /// context that does not start at line 0 or 1 (Git places it at the end of the file), or one
    /// after a hunk with no trailing context. `git diff -U0` writes such hunks; `git apply
    /// --unidiff-zero` reads them, plain `git apply` does not place them where they say.
    HunkPlacedElsewhere {
        /// 1-based line number of the hunk's header.
        line_number: usize,
        /// Why Git places it elsewhere.
        reason: &'static str,
    },
    /// A patch `git apply` fails to write after it has begun: a symlink whose target is empty,
    /// holds a NUL, is longer than 4,095 bytes or is in a binary body, or one path that is both
    /// a file and the directory of another. Git has then already removed what the patch deletes.
    GitCannotWrite {
        /// 1-based line number where the section began.
        line_number: usize,
        /// What Git cannot write.
        reason: &'static str,
    },
    /// A header line (a path, mode or hunk header the parser reads meaning from) that is not
    /// UTF-8. Decoded lossily, the value it names would be a substitute: `caf\xe9` and
    /// `caf\xe8` both read `caf\u{fffd}`, so two files would share one reviewed path.
    LossyHeader {
        /// 1-based line number of the header line.
        line_number: usize,
    },
    /// A line that begins with `@@` is not a hunk header in exactly Git's grammar
    /// (`@@ -a[,b] +c[,d] @@`, single spaces, digits only). Git either refuses it as corrupt or
    /// does not read it as a hunk at all, ending the file there.
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

impl PatchParseError {
    /// The engine exit code for this refusal ([`deep_diff_forge_core::exit`]), decided here once
    /// for every surface that reports one. A header that is not UTF-8 is an encoding failure
    /// (`io`, 3), as the exit-code registry names non-UTF-8 input and as a Git acquisition
    /// refuses a non-UTF-8 path; every other refusal is `invalid_input` (4). The match names
    /// every variant, so a new refusal does not compile until its code is chosen.
    #[must_use]
    pub const fn exit_code(&self) -> i32 {
        use deep_diff_forge_core::exit;
        match self {
            Self::LossyHeader { .. } => exit::IO,
            Self::InvalidEofMarker { .. }
            | Self::UnsupportedFormat { .. }
            | Self::InvalidPath { .. }
            | Self::InvalidMode { .. }
            | Self::MisplacedHeader { .. }
            | Self::InconsistentHeader { .. }
            | Self::UnterminatedLine { .. }
            | Self::BudgetExceeded { .. }
            | Self::LineBudgetExceeded { .. }
            | Self::MisplacedHunk { .. }
            | Self::HunkWithoutChange { .. }
            | Self::SectionWithoutPatch { .. }
            | Self::PathInSeveralSections { .. }
            | Self::HeaderNotWrittenByGit { .. }
            | Self::HunkPlacedElsewhere { .. }
            | Self::GitCannotWrite { .. }
            | Self::MalformedHunkHeader { .. }
            | Self::BodyLineOutsideHunk { .. }
            | Self::TruncatedHunk { .. }
            | Self::HunkLineCountMismatch { .. } => exit::INVALID_INPUT,
        }
    }
}

impl PatchParseError {
    /// The message of a refusal for what plain `git apply` would place or write otherwise.
    fn fmt_git_outcome(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::HunkPlacedElsewhere {
                line_number,
                reason,
            } => write!(
                f,
                "the hunk at line {line_number} would be placed elsewhere by git apply: {reason} (git diff -U0 output; apply it with --unidiff-zero)"
            ),
            Self::GitCannotWrite {
                line_number,
                reason,
            } => write!(
                f,
                "the file section beginning at line {line_number} cannot be written by git apply, which would fail after removing files: {reason}"
            ),
            _ => Ok(()),
        }
    }

    /// The message of a refusal for a path that one section, or two, both remove and write.
    fn fmt_path_reuse(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PathInSeveralSections {
                line_number,
                path,
                same_section: true,
            } => write!(
                f,
                "the file section beginning at line {line_number} renames {} to itself, removing and writing one path in one section, which no tool writes",
                Excerpt(path)
            ),
            Self::PathInSeveralSections {
                line_number, path, ..
            } => write!(
                f,
                "the file section beginning at line {line_number} writes or removes {}, which an earlier section also writes or removes; git apply does not apply such sections one after the other",
                Excerpt(path)
            ),
            _ => Ok(()),
        }
    }
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
                write!(
                    f,
                    "invalid, non-UTF-8 or unsafe Git path at line {line_number}"
                )
            }
            Self::InvalidMode { line_number } => {
                write!(f, "invalid file mode at line {line_number}")
            }
            Self::MisplacedHeader { line_number } => write!(
                f,
                "file header out of place at line {line_number} (after the file's headers or first hunk, or a --- without +++)"
            ),
            Self::InconsistentHeader { line_number } => write!(
                f,
                "contradictory file headers ending at line {line_number} (a new or deleted file without /dev/null, /dev/null on both sides, a repeated mode, or a move that is both a rename and a copy)"
            ),
            Self::UnterminatedLine { line_number } => write!(
                f,
                "the patch ends inside hunk line {line_number}, which has no line feed"
            ),
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
            Self::MisplacedHunk { line_number } => write!(
                f,
                "hunk header at line {line_number} does not follow its file's +++ header or the previous hunk, so git apply does not read it as part of the file"
            ),
            Self::HunkWithoutChange { line_number } => write!(
                f,
                "hunk ending at line {line_number} adds and removes nothing (git apply refuses it as corrupt)"
            ),
            Self::SectionWithoutPatch { line_number } => write!(
                f,
                "the file section beginning at line {line_number} holds nothing git apply reads as a patch (no hunk, no binary marker, and no rename, copy, creation, deletion or mode change in its headers)"
            ),
            Self::PathInSeveralSections { .. } => self.fmt_path_reuse(f),
            Self::HeaderNotWrittenByGit {
                line_number,
                header,
                reason,
            } => write!(
                f,
                "header line {line_number} ({header}) is not one Git's diff writes there: {reason}"
            ),
            Self::HunkPlacedElsewhere { .. } | Self::GitCannotWrite { .. } => {
                self.fmt_git_outcome(f)
            }
            Self::LossyHeader { line_number } => write!(
                f,
                "header line {line_number} is not UTF-8, so the path or value it names cannot be reviewed exactly"
            ),
            Self::MalformedHunkHeader { line_number, text } => write!(
                f,
                "malformed hunk header at line {line_number}: {}",
                Excerpt(text)
            ),
            Self::BodyLineOutsideHunk { line_number, text } => write!(
                f,
                "diff body line outside any hunk at line {line_number}: {}",
                Excerpt(text)
            ),
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
                "hunk body line at line {line_number} exceeds the count declared by its header: {}",
                Excerpt(text)
            ),
        }
    }
}

impl PatchParseError {
    /// The refusal as a surface that a model reads renders it (MCP, the daemon's
    /// `diff.diagnostics`, the SDKs): what the engine found, at which line of the patch, and the
    /// engine's own explanation, with no text of the input. [`Display`](std::fmt::Display) is
    /// the command line's and the protocol-2 methods' released message and stays as it is: it
    /// quotes the failing line. Here a variant that quotes a line gives the rule the line broke
    /// instead, and the one variant whose message names a path ([`Self::PathInSeveralSections`])
    /// shows it as the diagnostics text form shows a path: quoted, `\` and `"` escaped, every
    /// control, bidi and invisible character escaped, and cut after [`ERROR_EXCERPT_BYTES`]
    /// rendered bytes with the number of bytes left out. Every other variant's message holds
    /// only numbers and the engine's fixed words, and reads as `Display` does.
    #[must_use]
    pub const fn without_input(&self) -> WithoutInput<'_> {
        WithoutInput(self)
    }
}

/// A [`PatchParseError`] rendered with no text of the input ([`PatchParseError::without_input`]).
#[derive(Debug, Clone, Copy)]
pub struct WithoutInput<'a>(&'a PatchParseError);

impl std::fmt::Display for WithoutInput<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            PatchParseError::MalformedHunkHeader { line_number, .. } => write!(
                f,
                "malformed hunk header at line {line_number}: a line that begins with @@ is not a hunk header in Git's grammar (@@ -a[,b] +c[,d] @@, single spaces, digits only)"
            ),
            PatchParseError::BodyLineOutsideHunk { line_number, .. } => write!(
                f,
                "diff body line outside any hunk at line {line_number}: a line that begins with + or - stands where no hunk header has opened a hunk"
            ),
            PatchParseError::HunkLineCountMismatch { line_number, .. } => write!(
                f,
                "hunk body line at line {line_number} exceeds the count declared by its header: the hunk already holds the old and new lines its @@ header declared"
            ),
            PatchParseError::PathInSeveralSections {
                line_number,
                path,
                same_section: true,
            } => write!(
                f,
                "the file section beginning at line {line_number} renames {} to itself, removing and writing one path in one section, which no tool writes",
                SafePath(path)
            ),
            PatchParseError::PathInSeveralSections {
                line_number, path, ..
            } => write!(
                f,
                "the file section beginning at line {line_number} writes or removes {}, which an earlier section also writes or removes; git apply does not apply such sections one after the other",
                SafePath(path)
            ),
            // The match names every variant, so a new one does not compile until it is decided
            // whether its message holds text of the input.
            PatchParseError::InvalidEofMarker { .. }
            | PatchParseError::UnsupportedFormat { .. }
            | PatchParseError::InvalidPath { .. }
            | PatchParseError::InvalidMode { .. }
            | PatchParseError::MisplacedHeader { .. }
            | PatchParseError::InconsistentHeader { .. }
            | PatchParseError::UnterminatedLine { .. }
            | PatchParseError::BudgetExceeded { .. }
            | PatchParseError::LineBudgetExceeded { .. }
            | PatchParseError::MisplacedHunk { .. }
            | PatchParseError::HunkWithoutChange { .. }
            | PatchParseError::SectionWithoutPatch { .. }
            | PatchParseError::HeaderNotWrittenByGit { .. }
            | PatchParseError::HunkPlacedElsewhere { .. }
            | PatchParseError::GitCannotWrite { .. }
            | PatchParseError::LossyHeader { .. }
            | PatchParseError::TruncatedHunk { .. } => std::fmt::Display::fmt(self.0, f),
        }
    }
}

/// A path as [`WithoutInput`] shows it: quoted, `\` and `"` escaped, each control, bidi and
/// invisible character written as the core crate's `display_safe` writes it, and cut after
/// [`ERROR_EXCERPT_BYTES`] rendered bytes (never inside an escape) with the bytes left out
/// counted after the closing quote.
struct SafePath<'a>(&'a str);

impl std::fmt::Display for SafePath<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut shown = String::new();
        let mut consumed = 0;
        for c in self.0.chars() {
            let mut one = [0; 4];
            let rendered = match c {
                '\\' => std::borrow::Cow::Borrowed("\\\\"),
                '"' => std::borrow::Cow::Borrowed("\\\""),
                _ => deep_diff_forge_core::display_safe(c.encode_utf8(&mut one)),
            };
            if shown.len() + rendered.len() > ERROR_EXCERPT_BYTES {
                break;
            }
            shown.push_str(&rendered);
            consumed += c.len_utf8();
        }
        write!(f, "\"{shown}\"")?;
        if consumed < self.0.len() {
            write!(f, "… (+{} bytes)", self.0.len() - consumed)?;
        }
        Ok(())
    }
}

/// Bytes of an offending line a parse error quotes; the rest is counted, not echoed.
pub const ERROR_EXCERPT_BYTES: usize = 120;

/// An offending input line as a parse error quotes it: escaped, and cut after
/// [`ERROR_EXCERPT_BYTES`] with the number of bytes left out. The line was written by whoever
/// wrote the patch, and the message reaches terminals and agents' contexts: before, a 2 MB line
/// made a 2,000,074-byte error.
struct Excerpt<'a>(&'a str);

impl std::fmt::Display for Excerpt<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = self.0;
        if text.len() <= ERROR_EXCERPT_BYTES {
            return write!(f, "{text:?}");
        }
        // The last char boundary at or before the limit (a char is at most 4 bytes).
        let end = (ERROR_EXCERPT_BYTES.saturating_sub(3)..=ERROR_EXCERPT_BYTES)
            .rev()
            .find(|&i| text.is_char_boundary(i))
            .unwrap_or(0);
        write!(f, "{:?}… (+{} bytes)", &text[..end], text.len() - end)
    }
}

impl std::error::Error for PatchParseError {}

/// Immutable capture of original patch truth, independent of normalized rendering.
/// See `docs/RECOMMENDATION_INTEGRATION.md#patch-truth` (R02–R07).
#[derive(Debug, Clone)]
pub struct CapturedPatch {
    raw: String,
    /// The original bytes, kept only when they are not UTF-8 (then `raw` is their lossy decoding).
    bytes: Option<Vec<u8>>,
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
        Ok(Self {
            raw,
            bytes: None,
            digest,
            files,
        })
    }
    /// Parse and seal bytes that may not be UTF-8: a non-UTF-8 line is decoded lossily and its
    /// file reports it in `PatchTwin::lossy_lines`, while the digest is of the exact bytes.
    ///
    /// # Errors
    /// Returns the parser's typed error without retaining an invalid artifact.
    pub fn capture_bytes(bytes: Vec<u8>) -> Result<Self, PatchParseError> {
        match String::from_utf8(bytes) {
            Ok(raw) => Self::capture(raw),
            Err(error) => {
                let bytes = error.into_bytes();
                let decoded = decode(&bytes);
                let files = parse_decoded(&decoded, ParseOptions::default())?;
                let raw = decoded.text().to_owned();
                let digest = deep_diff_forge_core::sha256_hex(&bytes);
                Ok(Self {
                    raw,
                    bytes: Some(bytes),
                    digest,
                    files,
                })
            }
        }
    }
    /// The text the patch was parsed from: the original input byte for byte (including cosmetic
    /// headings and final LF state) when [`Self::is_exact`], else its lossy decoding.
    #[must_use]
    pub fn original(&self) -> &str {
        &self.raw
    }
    /// The original input bytes, always exact.
    #[must_use]
    pub fn original_bytes(&self) -> &[u8] {
        self.bytes.as_deref().unwrap_or(self.raw.as_bytes())
    }
    /// Whether [`Self::original`] is the exact input (every line was UTF-8).
    #[must_use]
    pub fn is_exact(&self) -> bool {
        self.bytes.is_none()
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

#[cfg(test)]
mod tests {
    use super::*;

    const MARKER: &str = "zq7MARKER";

    /// Every variant's name. The match names every variant, so a new refusal does not compile
    /// here until it is given a name, and the coverage check below then asks for its case.
    fn variant(error: &PatchParseError) -> &'static str {
        match error {
            PatchParseError::InvalidEofMarker { .. } => "InvalidEofMarker",
            PatchParseError::UnsupportedFormat { .. } => "UnsupportedFormat",
            PatchParseError::InvalidPath { .. } => "InvalidPath",
            PatchParseError::InvalidMode { .. } => "InvalidMode",
            PatchParseError::MisplacedHeader { .. } => "MisplacedHeader",
            PatchParseError::InconsistentHeader { .. } => "InconsistentHeader",
            PatchParseError::UnterminatedLine { .. } => "UnterminatedLine",
            PatchParseError::BudgetExceeded { .. } => "BudgetExceeded",
            PatchParseError::LineBudgetExceeded { .. } => "LineBudgetExceeded",
            PatchParseError::MisplacedHunk { .. } => "MisplacedHunk",
            PatchParseError::HunkWithoutChange { .. } => "HunkWithoutChange",
            PatchParseError::SectionWithoutPatch { .. } => "SectionWithoutPatch",
            PatchParseError::PathInSeveralSections { .. } => "PathInSeveralSections",
            PatchParseError::HeaderNotWrittenByGit { .. } => "HeaderNotWrittenByGit",
            PatchParseError::HunkPlacedElsewhere { .. } => "HunkPlacedElsewhere",
            PatchParseError::GitCannotWrite { .. } => "GitCannotWrite",
            PatchParseError::LossyHeader { .. } => "LossyHeader",
            PatchParseError::MalformedHunkHeader { .. } => "MalformedHunkHeader",
            PatchParseError::BodyLineOutsideHunk { .. } => "BodyLineOutsideHunk",
            PatchParseError::TruncatedHunk { .. } => "TruncatedHunk",
            PatchParseError::HunkLineCountMismatch { .. } => "HunkLineCountMismatch",
        }
    }

    /// The variants whose released message quotes a line of the input.
    const QUOTING: [&str; 3] = [
        "MalformedHunkHeader",
        "BodyLineOutsideHunk",
        "HunkLineCountMismatch",
    ];

    /// The parse failure of `patch` on the one path the daemon, the CLI and the SDKs share.
    fn refusal(patch: &[u8]) -> PatchParseError {
        let guards = InputGuards {
            files: false,
            hunks: false,
        };
        match SealedReview::new(patch, guards, "t") {
            Err(ReviewError::Parse(error)) => error,
            Err(other) => panic!("refused, not a parse failure: {other}"),
            Ok(_) => panic!("parsed: {}", String::from_utf8_lossy(patch)),
        }
    }

    /// One real patch per variant, each with the marker on the line the parser fails at, or in
    /// a header the section names (N75: one probe per way out of `without_input`).
    fn cases() -> Vec<PatchParseError> {
        let m = MARKER;
        let path = format!("src/{m}\u{202e}\u{200b}\u{feff}.rs");
        let section = |from: &str, to: &str| {
            format!(
                "diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n@@ -1 +1 @@\n-{from}\n+{to}\n"
            )
        };
        let text = [
            format!(
                "--- a/{m}\n+++ b/{m}\n@@ -1 +1 @@\n-a\n\\ No newline at end of file\n\\ No newline at end of file\n"
            ),
            format!(
                "diff --git a/{m}.bin b/{m}.bin\nindex 1111111..2222222 100644\nGIT binary patch\nliteral 8\nD~~~~~\n\n"
            ),
            format!("diff --git a/x b/{m}\ncopy from x\n"),
            format!("diff --git a/{m} b/{m}\nold mode 100644\nnew mode 100664\n"),
            format!("diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\nrename to {m}\n"),
            format!(
                "diff --git a/{m} b/{m}\ndeleted file mode 100644\n--- /dev/null\n+++ /dev/null\n@@ -1 +0,0 @@\n-a\n"
            ),
            format!("--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+{m}"),
            format!("--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n{m}\n@@ -5 +5 @@\n-c\n+d\n"),
            format!("--- a/x\n+++ b/x\n@@ -1 +1 @@\n {m}\n"),
            format!("--- a/{m}\n+++ b/{m}\n--- a/y\n+++ b/y\n@@ -1 +1 @@\n-a\n+b\n"),
            format!(
                "{}diff --git a/{path} b/{path}\ndeleted file mode 100644\nindex 2222222..0000000\n--- a/{path}\n+++ /dev/null\n@@ -1 +0,0 @@\n-{m}\n",
                section("a", "b")
            ),
            format!(
                "diff --git a/{path} b/{path}\nsimilarity index 100%\nrename from {path}\nrename to {path}\n"
            ),
            format!("diff --git a/{m} b/{m}\nold mode 100664\nnew mode 100744\n"),
            format!("--- a/x\n+++ b/x\n@@ -3 +3 @@\n-a\n+{m}\n"),
            format!(
                "diff --git a/d b/d\nnew file mode 100644\n--- /dev/null\n+++ b/d\n@@ -0,0 +1 @@\n+a\ndiff --git a/d/{m} b/d/{m}\nnew file mode 100644\n--- /dev/null\n+++ b/d/{m}\n@@ -0,0 +1 @@\n+b\n"
            ),
            format!("--- a/x\n+++ b/x\n@@ -1,{m} +1 @@\n-a\n+b\n"),
            format!("--- a/x\n+++ b/x\n+{m}\n"),
            format!("--- a/x\n+++ b/x\n@@ -1,2 +1,3 @@\n-{m}\n+b\n"),
            format!("--- a/x\n+++ b/x\n@@ -1,1 +0,0 @@\n+{m}\n"),
        ];
        let mut cases: Vec<PatchParseError> = text.iter().map(|p| refusal(p.as_bytes())).collect();
        let mut lossy = format!("--- a/{m}").into_bytes();
        lossy.extend_from_slice(b"\xe9\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n");
        cases.push(refusal(&lossy));
        let big = format!("--- a/{m}\n+++ b/{m}\n@@ -1 +1 @@\n-a\n+b\n");
        cases.push(parse_with(&big, ParseOptions { byte_budget: 8 }).unwrap_err());
        let long = format!("{m}\n").repeat(DEFAULT_LINE_BUDGET + 1);
        let options = ParseOptions {
            byte_budget: long.len(),
        };
        cases.push(parse_with(&long, options).unwrap_err());
        cases
    }

    /// N75: every variant, from a real patch, rendered without the input: no byte of the line it
    /// failed at (the marker), no raw hidden character, the same line number; a path only inside
    /// its escaped quote; and every variant that quotes nothing reads exactly as released.
    #[test]
    fn every_parse_error_renders_without_the_input() {
        let cases = cases();
        let mut covered: Vec<&str> = cases.iter().map(variant).collect();
        covered.sort_unstable();
        covered.dedup();
        assert_eq!(covered.len(), 21, "a variant has no case: {covered:?}");
        for error in &cases {
            let name = variant(error);
            let released = error.to_string();
            let safe = error.without_input().to_string();
            assert!(
                !safe.contains(deep_diff_forge_core::is_unsafe_in_untrusted_text),
                "{name}: a raw hidden character in {safe:?}"
            );
            let head = released
                .split_once(": ")
                .map_or(&*released, |(head, _)| head);
            assert!(
                safe.starts_with(head),
                "{name}: the rendering does not say what and where as released does: {safe:?} / {released:?}"
            );
            if name == "PathInSeveralSections" {
                let escaped = format!("\"src/{MARKER}\\u{{202e}}\\u{{200b}}\\u{{feff}}.rs\"");
                assert_eq!(
                    safe.matches(MARKER).count(),
                    safe.matches(&escaped).count(),
                    "{name}: patch text outside the escaped path: {safe:?}"
                );
                assert_eq!(safe.matches(&escaped).count(), 1, "{safe:?}");
            } else {
                assert!(
                    !safe.contains(MARKER),
                    "{name}: the rendering holds text of the input: {safe:?}"
                );
            }
            if QUOTING.contains(&name) {
                assert!(released.contains(MARKER), "{name}: {released:?}");
                assert!(safe.len() > head.len() + 2, "{name}: no reason: {safe:?}");
            } else if name != "PathInSeveralSections" {
                assert_eq!(safe, released, "{name}: changed where nothing is quoted");
            }
        }
    }

    /// The two messages the brief names, word for word.
    #[test]
    fn a_rendering_without_the_input_keeps_the_engines_explanation() {
        let truncated = refusal(b"--- a/x\n+++ b/x\n@@ -1,2 +1,3 @@\n-a\n+b\n");
        assert_eq!(
            truncated.without_input().to_string(),
            "truncated hunk ending at line 5: 1 old / 2 new line(s) declared but missing"
        );
        let header =
            refusal(format!("--- a/x\n+++ b/x\n@@ -1,{MARKER} +1 @@\n-a\n+b\n").as_bytes());
        assert_eq!(
            header.without_input().to_string(),
            "malformed hunk header at line 3: a line that begins with @@ is not a hunk header in Git's grammar (@@ -a[,b] +c[,d] @@, single spaces, digits only)"
        );
    }

    /// A long path is cut after `ERROR_EXCERPT_BYTES` rendered bytes, never inside an escape, and
    /// the bytes left out are counted after the closing quote.
    #[test]
    fn a_long_path_is_bounded_and_counted() {
        let path = "\u{202e}".repeat(400);
        let shown = SafePath(&path).to_string();
        let (quoted, rest) = shown.rsplit_once('"').unwrap();
        assert!(quoted.len() <= ERROR_EXCERPT_BYTES + 1, "{shown}");
        assert!(
            quoted.ends_with("\\u{202e}"),
            "cut inside an escape: {shown}"
        );
        let kept = quoted.matches("\\u{202e}").count();
        assert_eq!(rest, format!("… (+{} bytes)", 3 * (400 - kept)));
        assert_eq!(SafePath("a\"b\\c").to_string(), "\"a\\\"b\\\\c\"");
    }
}
