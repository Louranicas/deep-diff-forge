// DDF integration: R02, R03, R04, R05, R06, R07, R44, R46. Preserve source bytes and exact original input independently of normalized display; use Git as the application oracle.
// Design/status: docs/RECOMMENDATION_INTEGRATION.md#patch-truth
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fdeep-diff-forge-core--lib
mod deploy;
pub mod exit;
pub mod git;
mod release;
mod seal;
mod util;

pub use deploy::{DeploymentStatus, GateResult, GateState, MaturityLevel};
pub use release::{ReleasePlan, ReleaseTarget, TargetState};
pub use seal::{InputSeal, sha256, sha256_hex};
pub use util::{display_safe, json_escape};

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
    TypeChanged,
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
            Self::TypeChanged => "type_changed",
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchHunk {
    pub id: HunkId,
    pub old_start: Option<u32>,
    pub new_start: Option<u32>,
    pub lines: Vec<PatchLine>,
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
