//! Strategy scoring and trust.
//!
//! The scorer turns a flat list of [`StrategyReceipt`]s into a per-strategy
//! [`StrategyScore`], and a [`TrustPolicy`] decides whether a strategy has
//! earned the right to become a learned default. The policy encodes the spec's
//! qualitative bar — *fast enough, stable across reruns, helpful to reviewers,
//! not hiding patch truth* — as explicit, inspectable thresholds.
//!
//! Note on "not hiding patch truth": a receipt cannot alter patch content, so
//! the scorer treats a high *revisit* rate as the observable proxy for a
//! strategy whose output misled the reviewer into a second look.

use serde::{Deserialize, Serialize};

use crate::receipt::{ReviewOutcome, Strategy, StrategyReceipt};

/// Aggregate outcome statistics for one strategy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StrategyScore {
    /// Which strategy this scores.
    pub strategy: Strategy,
    /// Number of receipts observed for this strategy.
    pub samples: usize,
    /// Fraction of *decided* receipts that were accepted (`0.0..=1.0`).
    pub acceptance_rate: f64,
    /// Fraction of receipts that used a fallback (`0.0..=1.0`).
    pub fallback_rate: f64,
    /// Mean wall-clock cost in milliseconds.
    pub mean_elapsed_ms: f64,
    /// Fraction of receipts the reviewer revisited (`0.0..=1.0`).
    pub revisit_rate: f64,
    /// Fraction of receipts that were "helpful" (accepted, not revisited).
    pub helpful_rate: f64,
    /// Fraction of receipts served from a warm cache (`0.0..=1.0`).
    pub cache_hit_rate: f64,
}

impl StrategyScore {
    /// An empty score for `strategy` (no samples observed).
    #[must_use]
    pub fn empty(strategy: Strategy) -> Self {
        Self {
            strategy,
            samples: 0,
            acceptance_rate: 0.0,
            fallback_rate: 0.0,
            mean_elapsed_ms: 0.0,
            revisit_rate: 0.0,
            helpful_rate: 0.0,
            cache_hit_rate: 0.0,
        }
    }

    /// Whether this strategy has earned the right to be a learned default under
    /// `policy`. A score with too few samples never earns trust — the loop must
    /// not promote on thin evidence.
    #[must_use]
    pub fn earns_trust(&self, policy: &TrustPolicy) -> bool {
        self.samples >= policy.min_samples
            && self.acceptance_rate >= policy.min_acceptance_rate
            && self.helpful_rate >= policy.min_helpful_rate
            && self.fallback_rate <= policy.max_fallback_rate
            && self.revisit_rate <= policy.max_revisit_rate
            && self.mean_elapsed_ms <= policy.max_mean_elapsed_ms
    }
}

/// Thresholds a strategy must clear to be trusted as a learned default.
///
/// These are deliberately conservative defaults. They are a *policy*, not a law
/// of the engine — a deployment may tighten or loosen them, and every promotion
/// records which thresholds it cleared (see [`crate::promote`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrustPolicy {
    /// Minimum receipts before a strategy can be trusted.
    pub min_samples: usize,
    /// Minimum acceptance rate.
    pub min_acceptance_rate: f64,
    /// Minimum helpful rate.
    pub min_helpful_rate: f64,
    /// Maximum tolerated fallback rate.
    pub max_fallback_rate: f64,
    /// Maximum tolerated revisit rate.
    pub max_revisit_rate: f64,
    /// Maximum tolerated mean elapsed time, in milliseconds.
    pub max_mean_elapsed_ms: f64,
}

impl Default for TrustPolicy {
    fn default() -> Self {
        Self {
            min_samples: 20,
            min_acceptance_rate: 0.70,
            min_helpful_rate: 0.50,
            max_fallback_rate: 0.30,
            max_revisit_rate: 0.25,
            max_mean_elapsed_ms: 250.0,
        }
    }
}

/// Score every strategy seen in `receipts`.
///
/// Returns one [`StrategyScore`] per [`Strategy`] variant, in
/// [`Strategy::all`] order, so the output shape is stable regardless of which
/// strategies appear in the data (absent strategies score as
/// [`StrategyScore::empty`]).
#[must_use]
pub fn score_strategies(receipts: &[StrategyReceipt]) -> Vec<StrategyScore> {
    Strategy::all()
        .into_iter()
        .map(|strategy| score_one(strategy, receipts))
        .collect()
}

/// Score a single `strategy` over `receipts`.
#[must_use]
pub fn score_one(strategy: Strategy, receipts: &[StrategyReceipt]) -> StrategyScore {
    let mine: Vec<&StrategyReceipt> = receipts.iter().filter(|r| r.strategy == strategy).collect();
    let samples = mine.len();
    if samples == 0 {
        return StrategyScore::empty(strategy);
    }
    #[allow(clippy::cast_precision_loss)]
    let n = samples as f64;

    let decided = mine.iter().filter(|r| r.outcome.is_decided()).count();
    let accepted = mine
        .iter()
        .filter(|r| r.outcome == ReviewOutcome::Accepted)
        .count();
    let fallbacks = mine.iter().filter(|r| r.used_fallback()).count();
    let revisits = mine.iter().filter(|r| r.revisited).count();
    let helpful = mine.iter().filter(|r| r.is_helpful()).count();
    let hits = mine.iter().filter(|r| r.cache.is_hit()).count();
    // saturating_add: two near-u64::MAX receipts from a poisoned store would
    // wrap (release) or panic (debug) with plain `.sum()`; saturate instead so
    // mean_elapsed_ms stays large-but-finite rather than wrapping to near-zero.
    let total_ms: u64 = mine
        .iter()
        .map(|r| r.elapsed_ms)
        .fold(0u64, u64::saturating_add);

    #[allow(clippy::cast_precision_loss)]
    let acceptance_rate = if decided == 0 {
        0.0
    } else {
        accepted as f64 / decided as f64
    };
    #[allow(clippy::cast_precision_loss)]
    StrategyScore {
        strategy,
        samples,
        acceptance_rate,
        fallback_rate: fallbacks as f64 / n,
        mean_elapsed_ms: total_ms as f64 / n,
        revisit_rate: revisits as f64 / n,
        helpful_rate: helpful as f64 / n,
        cache_hit_rate: hits as f64 / n,
    }
}

/// The most helpful trusted strategy in `scores`, if any clears `policy`.
///
/// Ties break toward the lower-cost strategy (smaller `mean_elapsed_ms`), then
/// by [`Strategy::all`] order, so the result is deterministic.
#[must_use]
pub fn best_trusted<'a>(
    scores: &'a [StrategyScore],
    policy: &TrustPolicy,
) -> Option<&'a StrategyScore> {
    scores
        .iter()
        .filter(|s| s.earns_trust(policy))
        .max_by(|a, b| {
            a.helpful_rate
                .total_cmp(&b.helpful_rate)
                .then(b.mean_elapsed_ms.total_cmp(&a.mean_elapsed_ms))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::receipt::{CacheState, FallbackReason, ReviewOutcome};

    fn r(strategy: Strategy, outcome: ReviewOutcome, revisited: bool) -> StrategyReceipt {
        StrategyReceipt::new("h", "rust", "v", strategy).with_outcome(outcome, revisited)
    }

    fn many(
        strategy: Strategy,
        outcome: ReviewOutcome,
        revisited: bool,
        n: usize,
    ) -> Vec<StrategyReceipt> {
        (0..n).map(|_| r(strategy, outcome, revisited)).collect()
    }

    #[test]
    fn empty_input_scores_all_strategies_empty() {
        let scores = score_strategies(&[]);
        assert_eq!(scores.len(), 3);
        assert!(scores.iter().all(|s| s.samples == 0));
    }

    #[test]
    fn scores_are_in_strategy_all_order() {
        let scores = score_strategies(&[]);
        assert_eq!(scores[0].strategy, Strategy::Patch);
        assert_eq!(scores[1].strategy, Strategy::Syntax);
        assert_eq!(scores[2].strategy, Strategy::Hybrid);
    }

    #[test]
    fn acceptance_rate_over_decided_only() {
        let mut rs = many(Strategy::Syntax, ReviewOutcome::Accepted, false, 3);
        rs.extend(many(Strategy::Syntax, ReviewOutcome::Rejected, false, 1));
        rs.extend(many(Strategy::Syntax, ReviewOutcome::Skipped, false, 2)); // not decided
        let s = score_one(Strategy::Syntax, &rs);
        assert_eq!(s.samples, 6);
        // 3 accepted / 4 decided = 0.75
        assert!((s.acceptance_rate - 0.75).abs() < 1e-9);
    }

    #[test]
    fn helpful_rate_counts_accepted_not_revisited() {
        let mut rs = many(Strategy::Patch, ReviewOutcome::Accepted, false, 3);
        rs.extend(many(Strategy::Patch, ReviewOutcome::Accepted, true, 1)); // revisited
        let s = score_one(Strategy::Patch, &rs);
        // 3 helpful / 4 samples
        assert!((s.helpful_rate - 0.75).abs() < 1e-9);
    }

    #[test]
    fn fallback_rate_computed() {
        let mut rs = many(Strategy::Hybrid, ReviewOutcome::Accepted, false, 2);
        rs.push(
            r(Strategy::Hybrid, ReviewOutcome::Accepted, false)
                .with_fallback(Some(FallbackReason::NodeBudget)),
        );
        let s = score_one(Strategy::Hybrid, &rs);
        // 1 of 3
        assert!((s.fallback_rate - 1.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn mean_elapsed_is_average() {
        let rs = vec![
            r(Strategy::Patch, ReviewOutcome::Accepted, false).with_elapsed_ms(10),
            r(Strategy::Patch, ReviewOutcome::Accepted, false).with_elapsed_ms(20),
        ];
        let s = score_one(Strategy::Patch, &rs);
        assert!((s.mean_elapsed_ms - 15.0).abs() < 1e-9);
    }

    #[test]
    fn cache_hit_rate_computed() {
        let rs = vec![
            r(Strategy::Syntax, ReviewOutcome::Accepted, false).with_cache(CacheState::Hit),
            r(Strategy::Syntax, ReviewOutcome::Accepted, false).with_cache(CacheState::Cold),
        ];
        let s = score_one(Strategy::Syntax, &rs);
        assert!((s.cache_hit_rate - 0.5).abs() < 1e-9);
    }

    #[test]
    fn receipts_for_other_strategies_are_ignored() {
        let mut rs = many(Strategy::Patch, ReviewOutcome::Accepted, false, 5);
        rs.extend(many(Strategy::Syntax, ReviewOutcome::Rejected, false, 5));
        let s = score_one(Strategy::Patch, &rs);
        assert_eq!(s.samples, 5);
        assert!((s.acceptance_rate - 1.0).abs() < 1e-9);
    }

    #[test]
    fn default_policy_is_conservative() {
        let p = TrustPolicy::default();
        assert_eq!(p.min_samples, 20);
        assert!(p.min_acceptance_rate > 0.5);
    }

    #[test]
    fn thin_evidence_never_earns_trust() {
        let rs = many(Strategy::Syntax, ReviewOutcome::Accepted, false, 5);
        let s = score_one(Strategy::Syntax, &rs);
        assert!(!s.earns_trust(&TrustPolicy::default())); // < 20 samples
    }

    #[test]
    fn strong_strategy_earns_trust() {
        let rs = many(Strategy::Syntax, ReviewOutcome::Accepted, false, 25);
        let s = score_one(Strategy::Syntax, &rs);
        assert!(s.earns_trust(&TrustPolicy::default()));
    }

    #[test]
    fn high_fallback_rate_blocks_trust() {
        let mut rs = many(Strategy::Hybrid, ReviewOutcome::Accepted, false, 25);
        for rr in &mut rs[..10] {
            rr.fallback = Some(FallbackReason::ByteBudget);
        }
        let s = score_one(Strategy::Hybrid, &rs);
        assert!(s.fallback_rate > 0.30);
        assert!(!s.earns_trust(&TrustPolicy::default()));
    }

    #[test]
    fn slow_strategy_blocks_trust() {
        let rs: Vec<_> = (0..25)
            .map(|_| r(Strategy::Hybrid, ReviewOutcome::Accepted, false).with_elapsed_ms(500))
            .collect();
        let s = score_one(Strategy::Hybrid, &rs);
        assert!(!s.earns_trust(&TrustPolicy::default()));
    }

    #[test]
    fn high_revisit_rate_blocks_trust() {
        let rs = many(Strategy::Syntax, ReviewOutcome::Accepted, true, 25);
        let s = score_one(Strategy::Syntax, &rs);
        assert!(s.revisit_rate > 0.25);
        assert!(!s.earns_trust(&TrustPolicy::default()));
    }

    #[test]
    fn best_trusted_none_when_no_data() {
        let scores = score_strategies(&[]);
        assert!(best_trusted(&scores, &TrustPolicy::default()).is_none());
    }

    #[test]
    fn best_trusted_picks_highest_helpful() {
        let mut rs = many(Strategy::Patch, ReviewOutcome::Accepted, false, 25);
        // Syntax: half helpful (revisited), so lower helpful_rate than Patch.
        rs.extend(many(Strategy::Syntax, ReviewOutcome::Accepted, true, 25));
        let scores = score_strategies(&rs);
        let best = best_trusted(&scores, &TrustPolicy::default());
        // Only Patch clears the revisit threshold.
        assert_eq!(best.map(|s| s.strategy), Some(Strategy::Patch));
    }

    #[test]
    fn best_trusted_breaks_ties_toward_cheaper() {
        let fast: Vec<_> = (0..25)
            .map(|_| r(Strategy::Patch, ReviewOutcome::Accepted, false).with_elapsed_ms(5))
            .collect();
        let slow: Vec<_> = (0..25)
            .map(|_| r(Strategy::Syntax, ReviewOutcome::Accepted, false).with_elapsed_ms(50))
            .collect();
        let mut rs = fast;
        rs.extend(slow);
        let scores = score_strategies(&rs);
        // Equal helpful_rate (1.0); tie breaks to the lower mean_elapsed_ms.
        let best = best_trusted(&scores, &TrustPolicy::default());
        assert_eq!(best.map(|s| s.strategy), Some(Strategy::Patch));
    }
}
