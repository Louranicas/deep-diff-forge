//! The strategy receipt: one durable record per planner decision.
//!
//! A receipt is the atom of the learning loop. Every time the engine picks a
//! review strategy for a file, it can emit a receipt describing *what* it chose,
//! *how it performed*, and *how the reviewer reacted* — but never *what the code
//! was*. The struct deliberately cannot hold a path or source text; the file is
//! identified only by a redacted [`crate::util::redacted_id`] token.

use serde::{Deserialize, Serialize};

use crate::error::LearningError;

/// The review strategy the planner selected for a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Strategy {
    /// Patch-truth only — no semantic parse.
    Patch,
    /// Tree-sitter structural analysis layered on patch truth.
    Syntax,
    /// Patch truth plus a budget-limited semantic pass.
    Hybrid,
}

impl Strategy {
    /// Stable lowercase label, e.g. `"syntax"`.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Patch => "patch",
            Self::Syntax => "syntax",
            Self::Hybrid => "hybrid",
        }
    }

    /// Every strategy, in a stable order — useful for scoring and reporting.
    #[must_use]
    pub fn all() -> [Strategy; 3] {
        [Self::Patch, Self::Syntax, Self::Hybrid]
    }
}

/// Why a semantic strategy fell back to a cheaper one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FallbackReason {
    /// File exceeded the pre-parse byte budget.
    ByteBudget,
    /// Parsed tree exceeded the node budget.
    NodeBudget,
    /// Parse produced too many error nodes.
    ParseErrors,
    /// Parse exceeded the time budget.
    TimeBudget,
    /// No grammar registered for the language.
    Unsupported,
}

impl FallbackReason {
    /// Stable lowercase label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::ByteBudget => "byte_budget",
            Self::NodeBudget => "node_budget",
            Self::ParseErrors => "parse_errors",
            Self::TimeBudget => "time_budget",
            Self::Unsupported => "unsupported",
        }
    }
}

/// Cache disposition for the strategy run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheState {
    /// Served from a warm cache.
    Hit,
    /// Present in cache but stale; recomputed.
    Miss,
    /// No cache entry; computed cold.
    Cold,
    /// Cache deliberately bypassed.
    Bypass,
}

impl CacheState {
    /// Stable lowercase label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Hit => "hit",
            Self::Miss => "miss",
            Self::Cold => "cold",
            Self::Bypass => "bypass",
        }
    }

    /// Whether this disposition counts as a cache hit for hit-rate scoring.
    #[must_use]
    pub fn is_hit(self) -> bool {
        matches!(self, Self::Hit)
    }
}

/// How the reviewer (human or agent) resolved the file this receipt describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewOutcome {
    /// Approved as-is.
    Accepted,
    /// Rejected / changes requested.
    Rejected,
    /// Skipped without a decision.
    Skipped,
    /// Returned to after first viewing.
    Revisited,
    /// No decision recorded yet.
    Pending,
}

impl ReviewOutcome {
    /// Stable lowercase label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Rejected => "rejected",
            Self::Skipped => "skipped",
            Self::Revisited => "revisited",
            Self::Pending => "pending",
        }
    }

    /// Whether the reviewer reached a decision (accepted or rejected).
    #[must_use]
    pub fn is_decided(self) -> bool {
        matches!(self, Self::Accepted | Self::Rejected)
    }
}

/// A single planner-decision receipt.
///
/// Privacy contract: this struct carries a redacted `file_hash`, a `language`
/// token, a `parser_version`, and counts/timings — never a path or source text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StrategyReceipt {
    /// Non-reversible file identity ([`crate::util::redacted_id`]).
    pub file_hash: String,
    /// Language token, e.g. `"rust"`.
    pub language: String,
    /// Parser/grammar version that produced any semantic data.
    pub parser_version: String,
    /// Strategy the planner selected.
    pub strategy: Strategy,
    /// Fallback reason, if the strategy fell back.
    pub fallback: Option<FallbackReason>,
    /// Wall-clock cost of the strategy run, in milliseconds.
    pub elapsed_ms: u64,
    /// File size in bytes.
    pub bytes: u64,
    /// Semantic node count (0 if no parse).
    pub nodes: u64,
    /// Cache disposition.
    pub cache: CacheState,
    /// How the reviewer resolved the file.
    pub outcome: ReviewOutcome,
    /// Whether the reviewer revisited this file after first viewing.
    pub revisited: bool,
}

impl StrategyReceipt {
    /// Construct a receipt for `strategy` on a file identified by `file_hash`.
    ///
    /// Defaults are the "nothing-observed-yet" values: no fallback, cold cache,
    /// pending outcome, not revisited. Use the `with_*` setters to refine.
    #[must_use]
    pub fn new(
        file_hash: impl Into<String>,
        language: impl Into<String>,
        parser_version: impl Into<String>,
        strategy: Strategy,
    ) -> Self {
        Self {
            file_hash: file_hash.into(),
            language: language.into(),
            parser_version: parser_version.into(),
            strategy,
            fallback: None,
            elapsed_ms: 0,
            bytes: 0,
            nodes: 0,
            cache: CacheState::Cold,
            outcome: ReviewOutcome::Pending,
            revisited: false,
        }
    }

    /// Record the fallback reason (or clear it with `None`).
    #[must_use]
    pub fn with_fallback(mut self, fallback: Option<FallbackReason>) -> Self {
        self.fallback = fallback;
        self
    }

    /// Record the run cost in milliseconds.
    #[must_use]
    pub fn with_elapsed_ms(mut self, elapsed_ms: u64) -> Self {
        self.elapsed_ms = elapsed_ms;
        self
    }

    /// Record file size and node count.
    #[must_use]
    pub fn with_size(mut self, bytes: u64, nodes: u64) -> Self {
        self.bytes = bytes;
        self.nodes = nodes;
        self
    }

    /// Record the cache disposition.
    #[must_use]
    pub fn with_cache(mut self, cache: CacheState) -> Self {
        self.cache = cache;
        self
    }

    /// Record the reviewer outcome and whether the file was revisited.
    #[must_use]
    pub fn with_outcome(mut self, outcome: ReviewOutcome, revisited: bool) -> Self {
        self.outcome = outcome;
        self.revisited = revisited;
        self
    }

    /// Whether this run used a fallback strategy.
    #[must_use]
    pub fn used_fallback(&self) -> bool {
        self.fallback.is_some()
    }

    /// Whether this run was "helpful": the reviewer accepted the file without
    /// having to revisit it. This is the positive signal the scorer rewards —
    /// it never implies the patch truth was altered (the receipt cannot change
    /// patch content; it only describes review ergonomics).
    #[must_use]
    pub fn is_helpful(&self) -> bool {
        self.outcome == ReviewOutcome::Accepted && !self.revisited
    }

    /// Serialize to a single compact JSON line (for JSONL storage).
    ///
    /// # Errors
    /// Returns [`LearningError::Serialize`] if serialization fails (it does not
    /// for this plain struct, but the result is surfaced rather than panicked).
    pub fn to_json(&self) -> Result<String, LearningError> {
        serde_json::to_string(self).map_err(|e| LearningError::Serialize(e.to_string()))
    }

    /// Parse a receipt from one JSON object.
    ///
    /// # Errors
    /// Returns [`LearningError::Deserialize`] if `line` is not a valid receipt.
    pub fn from_json(line: &str) -> Result<Self, LearningError> {
        serde_json::from_str(line).map_err(|e| LearningError::Deserialize(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> StrategyReceipt {
        StrategyReceipt::new(
            "deadbeefdeadbeef",
            "rust",
            "tree-sitter-rust 0.24",
            Strategy::Syntax,
        )
        .with_elapsed_ms(12)
        .with_size(18_420, 2_840)
        .with_cache(CacheState::Hit)
        .with_outcome(ReviewOutcome::Accepted, false)
    }

    #[test]
    fn strategy_labels_are_stable() {
        assert_eq!(Strategy::Patch.label(), "patch");
        assert_eq!(Strategy::Syntax.label(), "syntax");
        assert_eq!(Strategy::Hybrid.label(), "hybrid");
    }

    #[test]
    fn strategy_all_has_three_distinct() {
        let all = Strategy::all();
        assert_eq!(all.len(), 3);
        assert_ne!(all[0], all[1]);
        assert_ne!(all[1], all[2]);
    }

    #[test]
    fn fallback_labels_are_stable() {
        assert_eq!(FallbackReason::ByteBudget.label(), "byte_budget");
        assert_eq!(FallbackReason::NodeBudget.label(), "node_budget");
        assert_eq!(FallbackReason::ParseErrors.label(), "parse_errors");
        assert_eq!(FallbackReason::TimeBudget.label(), "time_budget");
        assert_eq!(FallbackReason::Unsupported.label(), "unsupported");
    }

    #[test]
    fn cache_hit_predicate() {
        assert!(CacheState::Hit.is_hit());
        assert!(!CacheState::Miss.is_hit());
        assert!(!CacheState::Cold.is_hit());
        assert!(!CacheState::Bypass.is_hit());
    }

    #[test]
    fn cache_labels_are_stable() {
        assert_eq!(CacheState::Hit.label(), "hit");
        assert_eq!(CacheState::Bypass.label(), "bypass");
    }

    #[test]
    fn outcome_decided_predicate() {
        assert!(ReviewOutcome::Accepted.is_decided());
        assert!(ReviewOutcome::Rejected.is_decided());
        assert!(!ReviewOutcome::Skipped.is_decided());
        assert!(!ReviewOutcome::Revisited.is_decided());
        assert!(!ReviewOutcome::Pending.is_decided());
    }

    #[test]
    fn outcome_labels_are_stable() {
        assert_eq!(ReviewOutcome::Accepted.label(), "accepted");
        assert_eq!(ReviewOutcome::Pending.label(), "pending");
    }

    #[test]
    fn new_receipt_has_neutral_defaults() {
        let r = StrategyReceipt::new("h", "rust", "v", Strategy::Patch);
        assert_eq!(r.fallback, None);
        assert_eq!(r.elapsed_ms, 0);
        assert_eq!(r.cache, CacheState::Cold);
        assert_eq!(r.outcome, ReviewOutcome::Pending);
        assert!(!r.revisited);
    }

    #[test]
    fn builders_set_fields() {
        let r = sample();
        assert_eq!(r.elapsed_ms, 12);
        assert_eq!(r.bytes, 18_420);
        assert_eq!(r.nodes, 2_840);
        assert_eq!(r.cache, CacheState::Hit);
        assert_eq!(r.outcome, ReviewOutcome::Accepted);
    }

    #[test]
    fn used_fallback_predicate() {
        let r = sample();
        assert!(!r.used_fallback());
        let r = r.with_fallback(Some(FallbackReason::NodeBudget));
        assert!(r.used_fallback());
    }

    #[test]
    fn helpful_requires_accepted_and_not_revisited() {
        assert!(sample().is_helpful());
        assert!(
            !sample()
                .with_outcome(ReviewOutcome::Accepted, true)
                .is_helpful()
        );
        assert!(
            !sample()
                .with_outcome(ReviewOutcome::Rejected, false)
                .is_helpful()
        );
        assert!(
            !sample()
                .with_outcome(ReviewOutcome::Skipped, false)
                .is_helpful()
        );
    }

    #[test]
    fn json_round_trip_is_lossless() {
        let r = sample().with_fallback(Some(FallbackReason::ByteBudget));
        let json = r.to_json().expect("serialize");
        let back = StrategyReceipt::from_json(&json).expect("deserialize");
        assert_eq!(r, back);
    }

    #[test]
    fn json_is_single_line() {
        let json = sample().to_json().expect("serialize");
        assert!(!json.contains('\n'));
    }

    #[test]
    fn json_never_contains_a_path() {
        // The struct cannot hold a path; prove the serialized form is clean even
        // when the language/version carry slashes.
        let r = StrategyReceipt::new(
            crate::util::redacted_id("secret/path/file.rs"),
            "rust",
            "v",
            Strategy::Patch,
        );
        let json = r.to_json().expect("serialize");
        assert!(!json.contains("secret"));
        assert!(!json.contains("path/file"));
    }

    #[test]
    fn from_json_rejects_garbage() {
        assert!(StrategyReceipt::from_json("not json").is_err());
        assert!(StrategyReceipt::from_json("{}").is_err());
    }

    #[test]
    fn from_json_rejects_unknown_strategy() {
        let bad = r#"{"file_hash":"h","language":"rust","parser_version":"v","strategy":"telepathy","fallback":null,"elapsed_ms":0,"bytes":0,"nodes":0,"cache":"cold","outcome":"pending","revisited":false}"#;
        assert!(StrategyReceipt::from_json(bad).is_err());
    }

    #[test]
    fn fallback_serializes_as_snake_case() {
        let r = sample().with_fallback(Some(FallbackReason::ParseErrors));
        let json = r.to_json().expect("serialize");
        assert!(json.contains("parse_errors"));
    }
}
