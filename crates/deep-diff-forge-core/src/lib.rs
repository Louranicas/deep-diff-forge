mod deploy;
mod release;
mod util;

pub use deploy::{DeploymentStatus, GateResult, GateState, MaturityLevel};
pub use release::{ReleasePlan, ReleaseTarget, TargetState};
pub use util::json_escape;

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
    use super::FileStatus;

    #[test]
    fn labels_are_snake_case() {
        assert_eq!(FileStatus::BinaryChanged.label(), "binary_changed");
        assert_eq!(FileStatus::TypeChanged.label(), "type_changed");
        assert_eq!(FileStatus::Modified.label(), "modified");
        assert_eq!(FileStatus::Added.label(), "added");
        assert_eq!(FileStatus::Unknown.label(), "unknown");
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchTwin {
    pub hunks: Vec<PatchHunk>,
    pub metadata: Vec<String>,
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
