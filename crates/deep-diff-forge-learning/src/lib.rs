//! L9 learning loop for Deep-Diff-Forge.
//!
//! Deep-Diff-Forge improves through *measured review outcomes*, not vague
//! preference. This crate is the measurement-and-promotion half of that loop:
//!
//! 1. **Observe** — emit a [`StrategyReceipt`] per planner decision and append
//!    it to a local-only [`store`].
//! 2. **Score** — aggregate receipts into per-strategy [`StrategyScore`]s and a
//!    trust verdict ([`score`]).
//! 3. **Learn** — three units turn observations into recommendations: the
//!    [`planner`] (strategy + budget), [`ranking`] (risk weights), and
//!    [`annotation`] (agent trust tiers).
//! 4. **Gate** — [`promote`] decides whether a learned default may become the
//!    new default, and lists every reason it may not.
//!
//! Invariants the type system and module boundaries enforce:
//!
//! - **Local-only & private.** Receipts carry hashes, counts, and timings —
//!   never a path or source line. Nothing is uploaded.
//! - **Never mutates patch truth.** This crate observes and recommends; it has
//!   no path to change diff content.
//! - **Explainable & gated.** No default is promoted without clearing every
//!   rule, and every block is reported as a human-readable reason.
//! - **Fail-soft.** A missing store is "no data yet", not an error — reviews
//!   run unchanged on a fresh machine.

#![forbid(unsafe_code)]

pub mod annotation;
pub mod error;
pub mod planner;
pub mod promote;
pub mod ranking;
pub mod receipt;
pub mod score;
pub mod store;
pub mod util;

pub use annotation::{AnnotationObservation, AnnotationTrust, TrustTier, fit_trust};
pub use error::LearningError;
pub use planner::{BudgetProfile, PlannerLearner, PlannerObservation, PlannerRecommendation};
pub use promote::{PromotionCandidate, PromotionDecision, PromotionPolicy, evaluate_promotion};
pub use ranking::{RankingObservation, RankingWeights, fit_weights};
pub use receipt::{CacheState, FallbackReason, ReviewOutcome, Strategy, StrategyReceipt};
pub use score::{StrategyScore, TrustPolicy, best_trusted, score_one, score_strategies};
pub use util::redacted_id;

use std::path::Path;

/// A read-only summary of the learning state: how many receipts exist, how each
/// strategy scores, which strategy (if any) has earned trust, and the planner
/// the receipts imply.
#[derive(Debug, Clone)]
pub struct LearningReport {
    /// Total receipts observed.
    pub total_receipts: usize,
    /// Per-strategy scores in [`Strategy::all`] order.
    pub scores: Vec<StrategyScore>,
    /// The trusted default strategy, if any earned it.
    pub trusted_default: Option<Strategy>,
    /// The trust policy used to compute the verdict.
    pub policy: TrustPolicy,
}

impl LearningReport {
    /// Build a report from already-loaded receipts under the default policy.
    #[must_use]
    pub fn from_receipts(receipts: &[StrategyReceipt]) -> Self {
        Self::from_receipts_with_policy(receipts, TrustPolicy::default())
    }

    /// Build a report from receipts under an explicit `policy`.
    #[must_use]
    pub fn from_receipts_with_policy(receipts: &[StrategyReceipt], policy: TrustPolicy) -> Self {
        let scores = score_strategies(receipts);
        let trusted_default = best_trusted(&scores, &policy).map(|s| s.strategy);
        Self {
            total_receipts: receipts.len(),
            scores,
            trusted_default,
            policy,
        }
    }

    /// Build a report by loading receipts from the store under `dir`.
    ///
    /// # Errors
    /// Returns an error if the store exists but cannot be read or parsed.
    pub fn from_dir(dir: &Path) -> Result<Self, LearningError> {
        let receipts = store::load_receipts(dir)?;
        Ok(Self::from_receipts(&receipts))
    }

    /// A [`PlannerLearner`] consistent with this report's trusted default.
    #[must_use]
    pub fn planner(&self, receipts: &[StrategyReceipt]) -> PlannerLearner {
        PlannerLearner::from_receipts_with_policy(receipts, &self.policy)
    }
}

/// Record a receipt to the production learning store (fail-soft convenience).
///
/// Resolves the real learning directory and appends `receipt`. Intended for the
/// engine's hot path: a caller that does not care about the outcome can ignore
/// the `Result` and a failed write never affects the review.
///
/// # Errors
/// Returns an error if the state directory cannot be resolved or the append
/// fails.
pub fn record_receipt(receipt: &StrategyReceipt) -> Result<(), LearningError> {
    let dir = store::learning_dir()?;
    store::append_receipt(&dir, receipt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::receipt::ReviewOutcome;

    fn accepted(strategy: Strategy, n: usize) -> Vec<StrategyReceipt> {
        (0..n)
            .map(|_| {
                StrategyReceipt::new("h", "rust", "v", strategy)
                    .with_outcome(ReviewOutcome::Accepted, false)
                    .with_elapsed_ms(8)
            })
            .collect()
    }

    #[test]
    fn empty_report_has_no_trusted_default() {
        let report = LearningReport::from_receipts(&[]);
        assert_eq!(report.total_receipts, 0);
        assert_eq!(report.trusted_default, None);
        assert_eq!(report.scores.len(), 3);
    }

    #[test]
    fn report_counts_receipts() {
        let report = LearningReport::from_receipts(&accepted(Strategy::Patch, 5));
        assert_eq!(report.total_receipts, 5);
    }

    #[test]
    fn strong_strategy_becomes_trusted_default() {
        let report = LearningReport::from_receipts(&accepted(Strategy::Syntax, 30));
        assert_eq!(report.trusted_default, Some(Strategy::Syntax));
    }

    #[test]
    fn report_planner_matches_trusted_default() {
        let receipts = accepted(Strategy::Syntax, 30);
        let report = LearningReport::from_receipts(&receipts);
        let planner = report.planner(&receipts);
        assert_eq!(planner.trusted_default(), Some(Strategy::Syntax));
    }

    #[test]
    fn from_dir_on_missing_store_is_empty() {
        let dir = std::env::temp_dir().join(format!("ddf-learn-report-{}", std::process::id()));
        let report = LearningReport::from_dir(&dir).expect("report");
        assert_eq!(report.total_receipts, 0);
    }

    #[test]
    fn custom_policy_changes_trust_threshold() {
        // 10 receipts: below the default min_samples (20) but above a custom 5.
        let receipts = accepted(Strategy::Patch, 10);
        let strict = LearningReport::from_receipts(&receipts);
        assert_eq!(strict.trusted_default, None);

        let lenient_policy = TrustPolicy {
            min_samples: 5,
            ..TrustPolicy::default()
        };
        let lenient = LearningReport::from_receipts_with_policy(&receipts, lenient_policy);
        assert_eq!(lenient.trusted_default, Some(Strategy::Patch));
    }

    #[test]
    fn public_api_is_reexported() {
        // Compile-time proof the headline types are reachable from the crate
        // root (a regression guard for the re-export surface).
        let _ = redacted_id("x");
        let _ = TrustPolicy::default();
        let _ = PromotionPolicy::default();
        let _ = RankingWeights::prior();
        let _ = AnnotationTrust::unseen();
        let _ = BudgetProfile::Standard;
    }
}
