//! Planner learning unit.
//!
//! Maps the observable shape of a file (size, language, prior fallback history,
//! generated/vendor classification, semantic usefulness) to a recommended
//! review strategy and budget. The recommendation is *advisory*: it is derived
//! deterministically from accumulated receipts plus a few hard rules, and a
//! caller is always free to override it. Nothing here mutates patch truth.

use serde::{Deserialize, Serialize};

use crate::receipt::{FallbackReason, Strategy, StrategyReceipt};
use crate::score::{TrustPolicy, best_trusted, score_strategies};

/// What the planner can observe about a file before choosing a strategy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannerObservation {
    /// File size in bytes.
    pub file_bytes: u64,
    /// Language token, e.g. `"rust"`.
    pub language: String,
    /// Diff size in changed lines.
    pub diff_lines: u64,
    /// Whether the file is generated or vendored (low review value).
    pub generated_or_vendor: bool,
    /// The fallback reason from a previous run of this file, if any.
    pub previous_fallback: Option<FallbackReason>,
    /// How useful the semantic pass was last time, `0..=100`.
    pub semantic_usefulness: u8,
}

impl PlannerObservation {
    /// A plain observation for `language` at `file_bytes` with no prior history.
    #[must_use]
    pub fn new(language: impl Into<String>, file_bytes: u64, diff_lines: u64) -> Self {
        Self {
            file_bytes,
            language: language.into(),
            diff_lines,
            generated_or_vendor: false,
            previous_fallback: None,
            semantic_usefulness: 50,
        }
    }
}

/// How much budget to grant the semantic pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetProfile {
    /// Minimal budget — bail to patch truth quickly.
    Lean,
    /// The default budget envelope.
    Standard,
    /// Extra budget for files where semantics clearly help.
    Generous,
}

impl BudgetProfile {
    /// Stable lowercase label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Lean => "lean",
            Self::Standard => "standard",
            Self::Generous => "generous",
        }
    }

    /// Node budget implied by this profile.
    #[must_use]
    pub fn node_budget(self) -> u64 {
        match self {
            Self::Lean => 5_000,
            Self::Standard => 20_000,
            Self::Generous => 80_000,
        }
    }
}

/// The planner's recommendation for a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannerRecommendation {
    /// Strategy to attempt.
    pub preferred_strategy: Strategy,
    /// Budget envelope for any semantic pass.
    pub budget_profile: BudgetProfile,
    /// Node count above which to fall back to patch truth.
    pub fallback_threshold_nodes: u64,
    /// Cache priority, `0..=100` (higher = keep warm longer).
    pub cache_priority: u8,
}

/// Threshold above which a file is "large" for budgeting purposes.
const LARGE_FILE_BYTES: u64 = 256 * 1024;
/// Semantic-usefulness score at or above which generosity is warranted.
const USEFUL_SEMANTIC: u8 = 70;

/// A planner that recommends strategies, optionally informed by past receipts.
///
/// Construct with [`PlannerLearner::from_receipts`] to let the trusted-strategy
/// signal steer recommendations, or [`PlannerLearner::cold`] for the rule-only
/// planner used before any data exists.
#[derive(Debug, Clone)]
pub struct PlannerLearner {
    /// The strategy the receipts show to be most trustworthy, if any.
    trusted_default: Option<Strategy>,
}

impl PlannerLearner {
    /// A planner with no learned signal — recommendations come from the hard
    /// rules alone.
    #[must_use]
    pub fn cold() -> Self {
        Self {
            trusted_default: None,
        }
    }

    /// Build a planner from accumulated receipts under the default trust policy.
    #[must_use]
    pub fn from_receipts(receipts: &[StrategyReceipt]) -> Self {
        Self::from_receipts_with_policy(receipts, &TrustPolicy::default())
    }

    /// Build a planner from receipts under an explicit `policy`.
    #[must_use]
    pub fn from_receipts_with_policy(receipts: &[StrategyReceipt], policy: &TrustPolicy) -> Self {
        let scores = score_strategies(receipts);
        let trusted_default = best_trusted(&scores, policy).map(|s| s.strategy);
        Self { trusted_default }
    }

    /// The learned trusted default strategy, if the receipts earned one.
    #[must_use]
    pub fn trusted_default(&self) -> Option<Strategy> {
        self.trusted_default
    }

    /// Recommend a strategy and budget for `obs`.
    ///
    /// Hard rules (always applied, regardless of learned signal):
    /// - Generated/vendor files get [`Strategy::Patch`] with a [`BudgetProfile::Lean`]
    ///   budget and low cache priority — semantic analysis there is wasted work.
    /// - A file whose previous run hit the byte or node budget starts at
    ///   [`Strategy::Patch`] rather than re-paying for a fallback.
    ///
    /// Otherwise the learned trusted default (if any) wins, falling back to a
    /// size/usefulness heuristic.
    #[must_use]
    pub fn recommend(&self, obs: &PlannerObservation) -> PlannerRecommendation {
        if obs.generated_or_vendor {
            return PlannerRecommendation {
                preferred_strategy: Strategy::Patch,
                budget_profile: BudgetProfile::Lean,
                fallback_threshold_nodes: BudgetProfile::Lean.node_budget(),
                cache_priority: 5,
            };
        }

        let budgeted_out = matches!(
            obs.previous_fallback,
            Some(
                FallbackReason::ByteBudget
                    | FallbackReason::NodeBudget
                    | FallbackReason::TimeBudget
            )
        );
        if budgeted_out {
            return PlannerRecommendation {
                preferred_strategy: Strategy::Patch,
                budget_profile: BudgetProfile::Lean,
                fallback_threshold_nodes: BudgetProfile::Lean.node_budget(),
                cache_priority: 20,
            };
        }

        let budget = if obs.file_bytes >= LARGE_FILE_BYTES {
            // Large file: only spend generously if semantics clearly paid off.
            if obs.semantic_usefulness >= USEFUL_SEMANTIC {
                BudgetProfile::Standard
            } else {
                BudgetProfile::Lean
            }
        } else if obs.semantic_usefulness >= USEFUL_SEMANTIC {
            BudgetProfile::Generous
        } else {
            BudgetProfile::Standard
        };

        let heuristic = if obs.semantic_usefulness >= USEFUL_SEMANTIC {
            Strategy::Syntax
        } else {
            Strategy::Hybrid
        };
        let preferred = self.trusted_default.unwrap_or(heuristic);

        // A large changed surface is worth keeping warm.
        let cache_priority = u8::try_from((obs.diff_lines / 10).min(90))
            .unwrap_or(90)
            .max(30);

        PlannerRecommendation {
            preferred_strategy: preferred,
            budget_profile: budget,
            fallback_threshold_nodes: budget.node_budget(),
            cache_priority,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::receipt::ReviewOutcome;

    fn trusted_syntax_receipts() -> Vec<StrategyReceipt> {
        (0..30)
            .map(|_| {
                StrategyReceipt::new("h", "rust", "v", Strategy::Syntax)
                    .with_outcome(ReviewOutcome::Accepted, false)
                    .with_elapsed_ms(10)
            })
            .collect()
    }

    #[test]
    fn observation_constructor_defaults() {
        let o = PlannerObservation::new("rust", 1000, 40);
        assert_eq!(o.semantic_usefulness, 50);
        assert!(!o.generated_or_vendor);
        assert_eq!(o.previous_fallback, None);
    }

    #[test]
    fn budget_profile_labels_and_budgets() {
        assert_eq!(BudgetProfile::Lean.label(), "lean");
        assert!(BudgetProfile::Lean.node_budget() < BudgetProfile::Standard.node_budget());
        assert!(BudgetProfile::Standard.node_budget() < BudgetProfile::Generous.node_budget());
    }

    #[test]
    fn cold_planner_has_no_trusted_default() {
        assert_eq!(PlannerLearner::cold().trusted_default(), None);
    }

    #[test]
    fn generated_files_always_get_lean_patch() {
        let planner = PlannerLearner::from_receipts(&trusted_syntax_receipts());
        let mut obs = PlannerObservation::new("rust", 1000, 40);
        obs.generated_or_vendor = true;
        obs.semantic_usefulness = 100;
        let rec = planner.recommend(&obs);
        // Hard rule overrides even a trusted Syntax default.
        assert_eq!(rec.preferred_strategy, Strategy::Patch);
        assert_eq!(rec.budget_profile, BudgetProfile::Lean);
        assert!(rec.cache_priority <= 10);
    }

    #[test]
    fn previous_budget_fallback_starts_at_patch() {
        let planner = PlannerLearner::cold();
        let mut obs = PlannerObservation::new("rust", 1000, 40);
        obs.previous_fallback = Some(FallbackReason::NodeBudget);
        let rec = planner.recommend(&obs);
        assert_eq!(rec.preferred_strategy, Strategy::Patch);
        assert_eq!(rec.budget_profile, BudgetProfile::Lean);
    }

    #[test]
    fn previous_unsupported_fallback_does_not_force_patch() {
        // Unsupported is not a budget exhaustion; it should not trigger the
        // budgeted-out rule.
        let planner = PlannerLearner::cold();
        let mut obs = PlannerObservation::new("rust", 1000, 40);
        obs.previous_fallback = Some(FallbackReason::Unsupported);
        obs.semantic_usefulness = 80;
        let rec = planner.recommend(&obs);
        assert_ne!(rec.budget_profile, BudgetProfile::Lean);
    }

    #[test]
    fn trusted_default_steers_preferred_strategy() {
        let planner = PlannerLearner::from_receipts(&trusted_syntax_receipts());
        assert_eq!(planner.trusted_default(), Some(Strategy::Syntax));
        let obs = PlannerObservation::new("rust", 1000, 40);
        assert_eq!(planner.recommend(&obs).preferred_strategy, Strategy::Syntax);
    }

    #[test]
    fn large_useless_file_gets_lean_budget() {
        let planner = PlannerLearner::cold();
        let mut obs = PlannerObservation::new("rust", 1024 * 1024, 40);
        obs.semantic_usefulness = 10;
        assert_eq!(planner.recommend(&obs).budget_profile, BudgetProfile::Lean);
    }

    #[test]
    fn large_useful_file_gets_standard_budget() {
        let planner = PlannerLearner::cold();
        let mut obs = PlannerObservation::new("rust", 1024 * 1024, 40);
        obs.semantic_usefulness = 90;
        assert_eq!(
            planner.recommend(&obs).budget_profile,
            BudgetProfile::Standard
        );
    }

    #[test]
    fn small_useful_file_gets_generous_budget() {
        let planner = PlannerLearner::cold();
        let mut obs = PlannerObservation::new("rust", 1000, 40);
        obs.semantic_usefulness = 90;
        assert_eq!(
            planner.recommend(&obs).budget_profile,
            BudgetProfile::Generous
        );
    }

    #[test]
    fn cold_planner_heuristic_picks_syntax_when_useful() {
        let planner = PlannerLearner::cold();
        let mut obs = PlannerObservation::new("rust", 1000, 40);
        obs.semantic_usefulness = 90;
        assert_eq!(planner.recommend(&obs).preferred_strategy, Strategy::Syntax);
    }

    #[test]
    fn cold_planner_heuristic_picks_hybrid_when_unclear() {
        let planner = PlannerLearner::cold();
        let mut obs = PlannerObservation::new("rust", 1000, 40);
        obs.semantic_usefulness = 30;
        assert_eq!(planner.recommend(&obs).preferred_strategy, Strategy::Hybrid);
    }

    #[test]
    fn fallback_threshold_matches_budget() {
        let planner = PlannerLearner::cold();
        let obs = PlannerObservation::new("rust", 1000, 40);
        let rec = planner.recommend(&obs);
        assert_eq!(
            rec.fallback_threshold_nodes,
            rec.budget_profile.node_budget()
        );
    }

    #[test]
    fn cache_priority_is_bounded() {
        let planner = PlannerLearner::cold();
        let obs = PlannerObservation::new("rust", 1000, 100_000);
        let rec = planner.recommend(&obs);
        assert!(rec.cache_priority >= 30 && rec.cache_priority <= 90);
    }

    #[test]
    fn thin_receipts_yield_cold_planner() {
        let few: Vec<_> = (0..3)
            .map(|_| {
                StrategyReceipt::new("h", "rust", "v", Strategy::Syntax)
                    .with_outcome(ReviewOutcome::Accepted, false)
            })
            .collect();
        let planner = PlannerLearner::from_receipts(&few);
        assert_eq!(planner.trusted_default(), None);
    }

    #[test]
    fn recommendation_is_deterministic() {
        let planner = PlannerLearner::from_receipts(&trusted_syntax_receipts());
        let obs = PlannerObservation::new("rust", 4096, 60);
        assert_eq!(planner.recommend(&obs), planner.recommend(&obs));
    }
}
