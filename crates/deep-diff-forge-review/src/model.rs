// Review council: Typed providers, personas, phases, findings, citations, execution outcomes and final qualification.
// Design/status: docs/REVIEW_COUNCIL.md#typed-protocol
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fcouncil--crates--deep-diff-forge-review--src--model
//! Typed protocol. Model verdicts remain claims; only measured controller receipts satisfy gates.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Codex,
    ClaudeCode,
}
impl Provider {
    #[must_use]
    pub fn family(self) -> &'static str {
        match self {
            Self::Codex => "openai",
            Self::ClaudeCode => "anthropic",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Persona {
    General,
    Correctness,
    Security,
    Architecture,
    Performance,
    Falsifier,
    Subtractor,
}
impl Persona {
    #[must_use]
    pub fn lens(self) -> &'static str {
        match self {
            Self::General => {
                "Trace intent against actual code. Check correctness, boundaries, tests and operational behavior."
            }
            Self::Correctness => {
                "Find reachable invariant violations, state errors, Unicode/overflow failures and compatibility regressions. Supply a minimal counterexample."
            }
            Self::Security => {
                "Challenge trust boundaries, hostile inputs, authority, injection, credential exposure and resource exhaustion. Trace a concrete path."
            }
            Self::Architecture => {
                "Attack the premise. Compare smaller alternatives, ownership and dependency direction. Identify irreversible commitments and migration cost."
            }
            Self::Performance => {
                "Challenge asymptotic cost, allocation amplification, lock contention and latency claims. Specify a reproducible measurement and its limits."
            }
            Self::Falsifier => {
                "Try to refute the proposed design and its tests. Find vacuous assertions, missing negative controls, surviving mutants and untested crash points."
            }
            Self::Subtractor => {
                "Identify machinery that can be removed without losing required behavior. Challenge apparatus-to-product ratio and duplicated policy authority."
            }
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reviewer {
    pub id: String,
    pub provider: Provider,
    pub model: Option<String>,
    pub persona: Persona,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Profile {
    Roster,
    ImportantDecision,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub repository: String,
    pub analysis_file: String,
    pub decision_file: String,
    pub context_files: Vec<String>,
    pub builder_family: String,
    pub profile: Profile,
    pub reviewers: Vec<Reviewer>,
    pub mutation_package: String,
    pub fuzz_targets: Vec<String>,
    pub timeout_seconds: u64,
    pub fuzz_seconds: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Artifact {
    pub id: String,
    pub label: String,
    pub sha256: String,
    pub text: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema: String,
    pub run_id: String,
    pub review_id: String,
    pub source_tree: String,
    pub request: Request,
    pub artifacts: Vec<Artifact>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Round {
    Independent,
    Challenge,
}
impl Round {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Independent => "independent",
            Self::Challenge => "challenge",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Low,
    Medium,
    High,
    Critical,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    NoBlockers,
    ChangesRequested,
    Abstain,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Citation {
    pub artifact_id: String,
    pub start_line: usize,
    pub end_line: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Finding {
    pub id: String,
    pub severity: Severity,
    pub claim: String,
    pub citations: Vec<Citation>,
    pub falsifier: String,
    pub requested_checks: Vec<Gate>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Challenge {
    pub finding_id: String,
    pub disposition: String,
    pub rationale: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub run_id: String,
    pub reviewer_id: String,
    pub round: Round,
    pub verdict: Verdict,
    pub summary: String,
    pub findings: Vec<Finding>,
    pub responses: Vec<Challenge>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Gate {
    Format,
    Check,
    Clippy,
    Pedantic,
    Tests,
    Antipatterns,
    Fuzz,
    Mutation,
}
impl Gate {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Format => "format",
            Self::Check => "check",
            Self::Clippy => "clippy",
            Self::Pedantic => "pedantic",
            Self::Tests => "tests",
            Self::Antipatterns => "antipatterns",
            Self::Fuzz => "fuzz",
            Self::Mutation => "mutation",
        }
    }
    #[must_use]
    pub fn required(profile: Profile) -> Vec<Self> {
        let mut gates = vec![
            Self::Format,
            Self::Check,
            Self::Clippy,
            Self::Pedantic,
            Self::Tests,
            Self::Antipatterns,
        ];
        if profile == Profile::ImportantDecision {
            gates.extend([Self::Fuzz, Self::Mutation]);
        }
        gates
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Passed,
    Failed,
    TimedOut,
    OutputLimit,
    Unavailable,
    Stale,
    Inconclusive,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Execution {
    pub run_id: String,
    pub job: String,
    pub source_tree_before: String,
    pub source_tree_after: String,
    pub outcome: Outcome,
    pub exit_code: Option<i32>,
    pub elapsed_ms: u64,
    pub argv: Vec<String>,
    pub stdout_sha256: String,
    pub stderr_sha256: String,
    /// Local ID of the gate image the job was launched from; absent for provider jobs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_id: Option<String>,
}
#[derive(Debug, Clone, Serialize)]
pub struct Assessment {
    pub run_id: String,
    pub state: String,
    pub reasons: Vec<String>,
    pub reports_received: usize,
    pub reports_expected: usize,
    pub provider_families: Vec<String>,
    pub decision_owner: String,
}

/// Controller-written final qualification, separate from subprocess exit status.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Qualification {
    pub outcome: Outcome,
    pub report_sha256: Option<String>,
    pub witness_sha256: Option<String>,
}
