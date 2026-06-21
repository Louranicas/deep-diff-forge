//! Deployment status vocabulary.
//!
//! These types let machines (CI, habitat panels, agents) consume deployment
//! state instead of scraping prose — the gap analysis's "machine interfaces
//! outrank prose" policy. They are vocabulary only: serialization lives in the
//! CLI projection, never here.

/// Deployment maturity level (L0..L9 from the deployment framework).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MaturityLevel {
    /// Docs, core vocabulary, CLI smoke commands.
    L0,
    /// Patch parser, renderer, JSON output, fixtures.
    L1,
    /// Inline/side-by-side/stacked/pager output.
    L2,
    /// Chain stages and strict Bash contracts.
    L3,
    /// Tree-sitter syntax and semantic fallback.
    L4,
    /// TUI, graph ranking, agent annotations.
    L5,
    /// Parallel dimensional lanes and corpus receipts.
    L6,
    /// UDS daemon, shared cache, health, subscriptions.
    L7,
    /// Signed assets, crates, CI, no-mistakes gate.
    L8,
    /// Corpus-driven promotion and SLO-backed defaults.
    L9,
}

impl MaturityLevel {
    /// Short token, e.g. `"L3"`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::L0 => "L0",
            Self::L1 => "L1",
            Self::L2 => "L2",
            Self::L3 => "L3",
            Self::L4 => "L4",
            Self::L5 => "L5",
            Self::L6 => "L6",
            Self::L7 => "L7",
            Self::L8 => "L8",
            Self::L9 => "L9",
        }
    }

    /// Human name, e.g. `"Pipeline"`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::L0 => "Bootstrap",
            Self::L1 => "Patch",
            Self::L2 => "Projection",
            Self::L3 => "Pipeline",
            Self::L4 => "Semantic",
            Self::L5 => "Review",
            Self::L6 => "Cluster",
            Self::L7 => "Daemon",
            Self::L8 => "Release",
            Self::L9 => "Learning",
        }
    }
}

/// State of a single gate in a deployment run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateState {
    /// The gate was not executed in this run.
    NotRun,
    /// The gate passed.
    Pass,
    /// The gate produced a non-blocking warning.
    Warn,
    /// The gate failed.
    Fail,
    /// The gate is blocked by an unmet precondition.
    Blocked,
}

impl GateState {
    /// Stable lower-case token for machine output.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotRun => "not-run",
            Self::Pass => "pass",
            Self::Warn => "warn",
            Self::Fail => "fail",
            Self::Blocked => "blocked",
        }
    }

    /// Whether this state blocks deployment progress.
    #[must_use]
    pub fn is_blocking(self) -> bool {
        matches!(self, Self::Fail | Self::Blocked)
    }
}

/// One named gate and its state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateResult {
    /// Gate name (e.g. `"compile"`).
    pub name: String,
    /// Gate state.
    pub state: GateState,
}

impl GateResult {
    /// Construct a gate result.
    #[must_use]
    pub fn new(name: impl Into<String>, state: GateState) -> Self {
        Self {
            name: name.into(),
            state,
        }
    }
}

/// A machine-readable deployment status snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeploymentStatus {
    /// Repository identifier.
    pub repo: String,
    /// Declared maturity level.
    pub maturity: MaturityLevel,
    /// Gate results in stack order.
    pub gates: Vec<GateResult>,
}

impl DeploymentStatus {
    /// Construct a status with no gates.
    #[must_use]
    pub fn new(repo: impl Into<String>, maturity: MaturityLevel) -> Self {
        Self {
            repo: repo.into(),
            maturity,
            gates: Vec::new(),
        }
    }

    /// Append a gate (builder style).
    #[must_use]
    pub fn with_gate(mut self, name: impl Into<String>, state: GateState) -> Self {
        self.gates.push(GateResult::new(name, state));
        self
    }

    /// Look up a gate's state by name.
    #[must_use]
    pub fn gate_state(&self, name: &str) -> Option<GateState> {
        self.gates.iter().find(|g| g.name == name).map(|g| g.state)
    }

    /// Whether any gate is in a blocking state.
    #[must_use]
    pub fn has_blocking_gate(&self) -> bool {
        self.gates.iter().any(|g| g.state.is_blocking())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maturity_tokens_are_stable() {
        assert_eq!(MaturityLevel::L0.as_str(), "L0");
        assert_eq!(MaturityLevel::L3.as_str(), "L3");
        assert_eq!(MaturityLevel::L9.as_str(), "L9");
    }

    #[test]
    fn maturity_names_match_ladder() {
        assert_eq!(MaturityLevel::L1.name(), "Patch");
        assert_eq!(MaturityLevel::L2.name(), "Projection");
        assert_eq!(MaturityLevel::L3.name(), "Pipeline");
        assert_eq!(MaturityLevel::L4.name(), "Semantic");
    }

    #[test]
    fn maturity_is_ordered() {
        assert!(MaturityLevel::L0 < MaturityLevel::L1);
        assert!(MaturityLevel::L3 < MaturityLevel::L4);
        assert!(MaturityLevel::L9 > MaturityLevel::L0);
    }

    #[test]
    fn gate_state_tokens_are_stable() {
        assert_eq!(GateState::NotRun.as_str(), "not-run");
        assert_eq!(GateState::Pass.as_str(), "pass");
        assert_eq!(GateState::Warn.as_str(), "warn");
        assert_eq!(GateState::Fail.as_str(), "fail");
        assert_eq!(GateState::Blocked.as_str(), "blocked");
    }

    #[test]
    fn fail_and_blocked_are_blocking() {
        assert!(GateState::Fail.is_blocking());
        assert!(GateState::Blocked.is_blocking());
    }

    #[test]
    fn pass_warn_notrun_are_not_blocking() {
        assert!(!GateState::Pass.is_blocking());
        assert!(!GateState::Warn.is_blocking());
        assert!(!GateState::NotRun.is_blocking());
    }

    #[test]
    fn gate_result_constructs_from_str() {
        let g = GateResult::new("compile", GateState::Pass);
        assert_eq!(g.name, "compile");
        assert_eq!(g.state, GateState::Pass);
    }

    #[test]
    fn status_starts_with_no_gates() {
        let s = DeploymentStatus::new("deep-diff-forge", MaturityLevel::L3);
        assert!(s.gates.is_empty());
        assert_eq!(s.repo, "deep-diff-forge");
        assert_eq!(s.maturity, MaturityLevel::L3);
    }

    #[test]
    fn with_gate_appends_in_order() {
        let s = DeploymentStatus::new("r", MaturityLevel::L0)
            .with_gate("identity", GateState::Pass)
            .with_gate("compile", GateState::NotRun);
        assert_eq!(s.gates.len(), 2);
        assert_eq!(s.gates[0].name, "identity");
        assert_eq!(s.gates[1].name, "compile");
    }

    #[test]
    fn gate_state_lookup_finds_gate() {
        let s = DeploymentStatus::new("r", MaturityLevel::L0).with_gate("test", GateState::Pass);
        assert_eq!(s.gate_state("test"), Some(GateState::Pass));
        assert_eq!(s.gate_state("missing"), None);
    }

    #[test]
    fn has_blocking_gate_detects_fail() {
        let s = DeploymentStatus::new("r", MaturityLevel::L0)
            .with_gate("compile", GateState::Pass)
            .with_gate("test", GateState::Fail);
        assert!(s.has_blocking_gate());
    }

    #[test]
    fn has_blocking_gate_false_when_all_clear() {
        let s = DeploymentStatus::new("r", MaturityLevel::L0)
            .with_gate("compile", GateState::Pass)
            .with_gate("test", GateState::Warn);
        assert!(!s.has_blocking_gate());
    }

    #[test]
    fn status_equality_is_structural() {
        let a = DeploymentStatus::new("r", MaturityLevel::L1).with_gate("x", GateState::Pass);
        let b = DeploymentStatus::new("r", MaturityLevel::L1).with_gate("x", GateState::Pass);
        assert_eq!(a, b);
    }
}
