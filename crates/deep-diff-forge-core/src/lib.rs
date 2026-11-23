// DDF integration: R02, R03, R04, R05, R06, R07, R44, R46. Preserve source bytes and exact original input independently of normalized display; use Git as the application oracle.
// Design/status: docs/RECOMMENDATION_INTEGRATION.md#patch-truth
// Module index: docs/RECOMMENDATION_INTEGRATION.md#patch-truth
mod deploy;
pub mod exit;
pub mod git;
pub mod process;
mod release;
mod seal;
mod util;

pub use deploy::{DeploymentStatus, GateResult, GateState, MaturityLevel};
pub use release::{ReleasePlan, ReleaseTarget, TargetState};
pub use seal::{InputSeal, Sha256, sha256, sha256_hex};
pub use util::{
    display_safe, display_safe_bounded, is_unsafe_in_untrusted_text, json_escape, json_line_safe,
    path_hazards, text_hazards,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewDocument {
    pub files: Vec<ReviewFile>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionPlan {
    pub dimensions: Vec<ExecutionDimension>,
    pub lanes: Vec<ExecutionLane>,
    pub join_policy: JoinPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExecutionDimension {
    Patch,
    Semantic,
    Risk,
    Agent,
    Runtime,
    Storage,
    History,
    Presentation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionLane {
    pub id: String,
    pub dimension: ExecutionDimension,
    pub parallelism: Parallelism,
    pub input_contract: StreamContract,
    pub output_contract: StreamContract,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Parallelism {
    Serial,
    Auto,
    Fixed(u16),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamContract {
    Human,
    Json,
    JsonLines,
    Compact,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinPolicy {
    DeterministicInputOrder,
    RankedReviewOrder,
    AsReadyWithStableIds,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoomPlan {
    pub name: String,
    pub phases: Vec<LoomPhase>,
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoomPhase {
    Intake,
    BoundaryMap,
    WeavePlan,
    FixtureSynthesis,
    RustCrateStub,
    Gate,
    Receipt,
    Assimilation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewFile {
    pub path: String,
    pub status: FileStatus,
    pub patch_twin: PatchTwin,
    pub semantic_twin: Option<SemanticTwin>,
    pub planner: PlannerDecision,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
    /// `copy from`/`copy to`: a new path made from an existing one, which stays.
    Copied,
    /// The object type changed (a regular file, a symlink or a submodule became another).
    TypeChanged,
    /// Only the mode changed (the executable bit), with no content change.
    ModeChanged,
    BinaryChanged,
    Unknown,
}

impl FileStatus {
    /// Canonical snake-case label, identical across every output surface
    /// (JSON, JSONL, projection, rank).
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Added => "added",
            Self::Modified => "modified",
            Self::Deleted => "deleted",
            Self::Renamed => "renamed",
            Self::Copied => "copied",
            Self::TypeChanged => "type_changed",
            Self::ModeChanged => "mode_changed",
            Self::BinaryChanged => "binary_changed",
            Self::Unknown => "unknown",
        }
    }
}

#[cfg(test)]
mod file_status_tests {
    use super::{FileStatus, SemanticChangeKind};

    /// Semantic regions sort by kind in this order; the JSON regions of every review depend on it.
    #[test]
    fn semantic_kinds_rank_in_their_published_order() {
        use SemanticChangeKind::{
            AddedNode, ModifiedNode, MovedNode, ReformattedOnly, RemovedNode, RenamedSymbol,
        };
        let ranks = [
            AddedNode,
            RemovedNode,
            ModifiedNode,
            MovedNode,
            ReformattedOnly,
            RenamedSymbol,
        ]
        .map(SemanticChangeKind::rank);
        assert_eq!(ranks, [0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn labels_are_snake_case() {
        assert_eq!(FileStatus::BinaryChanged.label(), "binary_changed");
        assert_eq!(FileStatus::TypeChanged.label(), "type_changed");
        assert_eq!(FileStatus::ModeChanged.label(), "mode_changed");
        assert_eq!(FileStatus::Copied.label(), "copied");
        assert_eq!(FileStatus::Renamed.label(), "renamed");
        assert_eq!(FileStatus::Deleted.label(), "deleted");
        assert_eq!(FileStatus::Modified.label(), "modified");
        assert_eq!(FileStatus::Added.label(), "added");
        assert_eq!(FileStatus::Unknown.label(), "unknown");
    }

    /// The wire names are a contract: these are the bytes consumers have parsed since v0.2.0, when
    /// they came from `Debug`. A rename of a variant must not move them.
    #[test]
    fn wire_names_are_the_published_names() {
        use super::{FallbackReason as F, SemanticChangeKind as K};
        let reasons = [
            (F::UnsupportedLanguage, "UnsupportedLanguage"),
            (F::ParseErrorsExceeded, "ParseErrorsExceeded"),
            (F::ByteBudgetExceeded, "ByteBudgetExceeded"),
            (F::NodeBudgetExceeded, "NodeBudgetExceeded"),
            (F::TimeBudgetExceeded, "TimeBudgetExceeded"),
            (F::BinaryInput, "BinaryInput"),
            (F::InvalidPatch, "InvalidPatch"),
        ];
        for (reason, name) in reasons {
            assert_eq!(reason.wire_name(), name);
        }
        let kinds = [
            (K::AddedNode, "AddedNode"),
            (K::RemovedNode, "RemovedNode"),
            (K::ModifiedNode, "ModifiedNode"),
            (K::MovedNode, "MovedNode"),
            (K::ReformattedOnly, "ReformattedOnly"),
            (K::RenamedSymbol, "RenamedSymbol"),
        ];
        for (kind, name) in kinds {
            assert_eq!(kind.wire_name(), name);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchTwin {
    /// Encoded Git binary payload, including literal/delta headers and separators.
    pub binary_body: Vec<String>,
    pub hunks: Vec<PatchHunk>,
    pub metadata: Vec<String>,
    /// Patch lines of this file that were not UTF-8 and were decoded lossily (U+FFFD). 0 means
    /// every line is the exact input text; the input seal always covers the exact bytes.
    pub lossy_lines: u32,
    /// Git first gave this file's change as binary, and the hunks are its text read again with
    /// `--text`: why Git called the file binary. Only a Git acquisition sets it; a patch read
    /// from input never does.
    pub diff_suppressed: Option<DiffSuppression>,
}

/// A recorded header line that makes its section binary to Git: `GIT binary patch`,
/// `Binary files … differ`, or `Files … differ` (the parser's reading, kept here once so the
/// ranking reads it too).
#[must_use]
pub fn is_binary_marker(meta: &str) -> bool {
    meta.starts_with("GIT binary patch")
        || meta.starts_with("Binary files ")
        || (meta.starts_with("Files ") && meta.ends_with(" differ"))
}

impl PatchTwin {
    /// The section's content is a binary blob to Git: a marker among its headers. The status
    /// says what Git does to the path (a binary creation is `added`, as `git apply --summary`
    /// has it); this stays true for it (S3.4).
    #[must_use]
    pub fn is_binary(&self) -> bool {
        self.metadata.iter().any(|m| is_binary_marker(m))
    }
}

/// Why Git gave a text file's change as binary ([`PatchTwin::diff_suppressed`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffSuppression {
    /// The path's `diff` attribute is unset (`-diff`, the `binary` macro) or names a diff
    /// driver, from any attributes file Git reads. Also given when `git check-attr` fails for a
    /// reason other than the time limit (its 64 KiB output cap included) or the path cannot be
    /// passed to it as an argument: the cause is then not known, and the label is kept.
    Attribute,
    /// No `diff` attribute applies: Git's own content check (a NUL byte) called it binary.
    Content,
    /// No `diff` attribute applies and the file is UTF-16 or UTF-32 (a byte order mark, or one
    /// NUL in every pair): Git's content check called it binary for the NULs of its encoding.
    Encoding,
    /// No attribute label covers the file and DDF could not read it again within its limits:
    /// `git check-attr` was cut off by the time limit, or the second reading was cut off by the
    /// byte budget or the time limit, failed, or gave an answer that could not be used.
    Unread,
}

impl DiffSuppression {
    /// Stable label for machine output.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Attribute => "attribute",
            Self::Content => "content",
            Self::Encoding => "encoding",
            Self::Unread => "unread",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchHunk {
    pub id: HunkId,
    pub old_start: Option<u32>,
    pub new_start: Option<u32>,
    /// The section heading Git writes after the closing `@@` (often the enclosing function), or
    /// `None` when the header has none. Display only: it never changes what the hunk applies.
    pub section: Option<String>,
    pub lines: Vec<PatchLine>,
}

impl PatchHunk {
    /// Lines on the old side (context and removed) and on the new side (context and added).
    #[must_use]
    pub fn counts(&self) -> (u32, u32) {
        let mut old = 0u32;
        let mut new = 0u32;
        for line in &self.lines {
            match line.kind {
                PatchLineKind::Context => {
                    old += 1;
                    new += 1;
                }
                PatchLineKind::Removed => old += 1,
                PatchLineKind::Added => new += 1,
            }
        }
        (old, new)
    }

    /// Where plain `git apply` anchors the hunk, which its lines decide and its header's line
    /// does not: a hunk at old line 0 or 1 at the start of the file, one with no trailing context
    /// at its end, one with both on the whole file. `None` for any other hunk, which Git places
    /// where its context and removed lines are found nearest the line its header names.
    #[must_use]
    pub fn git_anchor(&self) -> Option<&'static str> {
        let start = self.old_start.unwrap_or(0) <= 1;
        let end = self
            .lines
            .last()
            .is_none_or(|l| l.kind != PatchLineKind::Context);
        match (start, end) {
            (true, true) => Some("Git places this hunk on the whole file"),
            (true, false) => Some("Git places this hunk at the start of the file"),
            (false, true) => Some("Git places this hunk at the end of the file"),
            (false, false) => None,
        }
    }

    /// SHA-256 of the hunk's body: each line as its prefix (` `, `-`, `+`), text and LF, with
    /// `\ No newline at end of file` after a line that lacks one. It depends on the body alone,
    /// so the same change keeps it when other files or hunks move; use it to match a hunk across
    /// revisions, never as proof that two reviews saw the same patch.
    #[must_use]
    pub fn body_sha256(&self) -> String {
        crate::sha256_hex(self.body_bytes().as_bytes())
    }

    /// The hunk's address within one review: SHA-256 over `ddf-hunk-v1`, the file path, the
    /// header's ranges and the body, NUL-separated. Any change to the path, the position or a
    /// byte of the body changes it, and a different patch never shares it except by containing
    /// the same hunk at the same place. It names a hunk's content and place, not its occurrence:
    /// a patch that repeats a section of one path with a byte-identical hunk at the same ranges
    /// (a concatenated `a->b`, `b->a`, `a->b` series) gives both copies the same address.
    /// Selecting by it returns every copy, and a finding anchored to it resolves to the first in
    /// review order; the copies' content is identical, so either reading is true of both.
    #[must_use]
    pub fn address(&self, path: &str) -> String {
        let (old_count, new_count) = self.counts();
        let header = format!(
            "{},{}\0{},{}",
            self.old_start.unwrap_or(0),
            old_count,
            self.new_start.unwrap_or(0),
            new_count
        );
        let mut bytes = String::with_capacity(path.len() + header.len() + 32);
        bytes.push_str("ddf-hunk-v1\0");
        bytes.push_str(path);
        bytes.push('\0');
        bytes.push_str(&header);
        bytes.push('\0');
        bytes.push_str(&self.body_bytes());
        crate::sha256_hex(bytes.as_bytes())
    }

    fn body_bytes(&self) -> String {
        let mut body = String::new();
        for line in &self.lines {
            body.push(match line.kind {
                PatchLineKind::Context => ' ',
                PatchLineKind::Removed => '-',
                PatchLineKind::Added => '+',
            });
            body.push_str(&line.text);
            body.push('\n');
            if line.no_newline {
                body.push_str("\\ No newline at end of file\n");
            }
        }
        body
    }
}

#[cfg(test)]
mod hunk_tests {
    use super::{HunkId, PatchHunk, PatchLine, PatchLineKind};

    fn line(kind: PatchLineKind, text: &str, no_newline: bool) -> PatchLine {
        PatchLine {
            no_newline,
            lossy: false,
            kind,
            old_line: None,
            new_line: None,
            text: text.to_string(),
        }
    }

    /// `@@ -3,2 +4,3 @@`: one context line, one removed, two added, the last without a newline.
    fn hunk() -> PatchHunk {
        PatchHunk {
            id: HunkId(0),
            old_start: Some(3),
            new_start: Some(4),
            section: None,
            lines: vec![
                line(PatchLineKind::Context, "a", false),
                line(PatchLineKind::Removed, "b", false),
                line(PatchLineKind::Added, "c", false),
                line(PatchLineKind::Added, "d", true),
            ],
        }
    }

    #[test]
    fn counts_give_context_to_both_sides_and_each_change_to_its_own() {
        assert_eq!(hunk().counts(), (2, 3));
        let only_removed = PatchHunk {
            lines: vec![
                line(PatchLineKind::Removed, "x", false),
                line(PatchLineKind::Removed, "y", false),
            ],
            ..hunk()
        };
        assert_eq!(only_removed.counts(), (2, 0));
    }

    #[test]
    fn git_anchor_follows_the_start_line_and_the_trailing_context() {
        let end = "Git places this hunk at the end of the file";
        assert_eq!(hunk().git_anchor(), Some(end));
        let mut lines = hunk().lines;
        lines.push(line(PatchLineKind::Context, "e", false));
        let inner = PatchHunk { lines, ..hunk() };
        assert_eq!(inner.git_anchor(), None);
        for (start, said) in [(0, "on the whole file"), (1, "on the whole file")] {
            let h = PatchHunk {
                old_start: Some(start),
                ..hunk()
            };
            assert_eq!(
                h.git_anchor(),
                Some(&*format!("Git places this hunk {said}"))
            );
        }
        let first = PatchHunk {
            old_start: Some(1),
            ..inner
        };
        assert_eq!(
            first.git_anchor(),
            Some("Git places this hunk at the start of the file")
        );
    }

    // The expected digests are `sha256sum` over the bytes the doc comments define, written out
    // with `printf`, so they pin the wire form rather than the code that builds it.
    #[test]
    fn body_bytes_and_digest_follow_the_documented_form() {
        let hunk = hunk();
        assert_eq!(
            hunk.body_bytes(),
            " a\n-b\n+c\n+d\n\\ No newline at end of file\n"
        );
        assert_eq!(
            hunk.body_sha256(),
            "27d7c55254a8fc3cd3f20e1274028609394697033b3b1b857ef8abb5c23aa42d"
        );
    }

    #[test]
    fn address_hashes_tag_path_ranges_and_body() {
        // printf 'ddf-hunk-v1\0src/x.rs\0003,2\0004,3\0 a\n-b\n+c\n+d\n\\ No newline at end of file\n'
        assert_eq!(
            hunk().address("src/x.rs"),
            "fc17192b5bb10221803e8adbb35c2bb322e0b37c664e1f7855c5584ffd983edf"
        );
        assert_ne!(hunk().address("src/y.rs"), hunk().address("src/x.rs"));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HunkId(pub u64);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchLine {
    /// This source line has no terminating LF on its corresponding side(s).
    pub no_newline: bool,
    /// This line was not UTF-8 in the patch and `text` is its lossy decoding (U+FFFD); the input
    /// seal covers the exact bytes. Counted per file in `PatchTwin::lossy_lines`.
    pub lossy: bool,
    pub kind: PatchLineKind,
    pub old_line: Option<u32>,
    pub new_line: Option<u32>,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatchLineKind {
    Context,
    Added,
    Removed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticTwin {
    pub language: String,
    pub parse_status: ParseStatus,
    pub spans: Vec<SemanticSpan>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseStatus {
    Parsed,
    ParsedWithErrors { errors: u32 },
    Fallback { reason: FallbackReason },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticSpan {
    pub id: SemanticSpanId,
    pub hunk_id: HunkId,
    pub kind: SemanticChangeKind,
    pub old_range: Option<TextRange>,
    pub new_range: Option<TextRange>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SemanticSpanId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemanticChangeKind {
    AddedNode,
    RemovedNode,
    ModifiedNode,
    MovedNode,
    ReformattedOnly,
    RenamedSymbol,
}

impl SemanticChangeKind {
    /// The position of this kind in sorted output. Written out, not `self as u8`, so reordering
    /// or inserting a variant cannot silently reorder output that callers compare (anchor D-02);
    /// a new variant fails to compile until it is given a rank.
    #[must_use]
    pub const fn rank(self) -> u8 {
        match self {
            Self::AddedNode => 0,
            Self::RemovedNode => 1,
            Self::ModifiedNode => 2,
            Self::MovedNode => 3,
            Self::ReformattedOnly => 4,
            Self::RenamedSymbol => 5,
        }
    }

    /// The name this kind has in every output (review JSON, analysis JSON). Written out, not taken
    /// from `Debug`, so renaming a variant cannot silently change bytes that consumers parse; a new
    /// variant fails to compile until it is named here.
    #[must_use]
    pub const fn wire_name(self) -> &'static str {
        match self {
            Self::AddedNode => "AddedNode",
            Self::RemovedNode => "RemovedNode",
            Self::ModifiedNode => "ModifiedNode",
            Self::MovedNode => "MovedNode",
            Self::ReformattedOnly => "ReformattedOnly",
            Self::RenamedSymbol => "RenamedSymbol",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextRange {
    pub start_byte: u64,
    pub end_byte: u64,
    pub start_line: u32,
    pub end_line: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannerDecision {
    pub strategy: DiffStrategy,
    pub fallback: Option<FallbackReason>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffStrategy {
    Line,
    Word,
    Syntax,
    MovedBlock,
    Binary,
    GeneratedSuppressed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FallbackReason {
    UnsupportedLanguage,
    ParseErrorsExceeded,
    ByteBudgetExceeded,
    NodeBudgetExceeded,
    TimeBudgetExceeded,
    BinaryInput,
    InvalidPatch,
}

impl FallbackReason {
    /// The name this reason has in every output (review and structural JSON, `fallback:` text).
    /// Written out, not taken from `Debug`, so renaming a variant cannot silently change bytes that
    /// consumers parse; a new variant fails to compile until it is named here.
    #[must_use]
    pub const fn wire_name(&self) -> &'static str {
        match self {
            Self::UnsupportedLanguage => "UnsupportedLanguage",
            Self::ParseErrorsExceeded => "ParseErrorsExceeded",
            Self::ByteBudgetExceeded => "ByteBudgetExceeded",
            Self::NodeBudgetExceeded => "NodeBudgetExceeded",
            Self::TimeBudgetExceeded => "TimeBudgetExceeded",
            Self::BinaryInput => "BinaryInput",
            Self::InvalidPatch => "InvalidPatch",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentAnnotation {
    pub id: String,
    pub anchor: AnnotationAnchor,
    pub body: String,
    pub provenance: AnnotationProvenance,
    pub grounded: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnnotationAnchor {
    File {
        path: String,
    },
    Hunk {
        path: String,
        hunk_id: HunkId,
    },
    SemanticSpan {
        path: String,
        span_id: SemanticSpanId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnnotationProvenance {
    pub agent: String,
    pub model: Option<String>,
    pub evidence: Vec<String>,
}

impl ReviewDocument {
    #[must_use]
    pub fn empty() -> Self {
        Self { files: Vec::new() }
    }
}

mod snapshot;
pub use snapshot::changed_source_snapshot_id;
