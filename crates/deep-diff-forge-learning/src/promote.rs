//! Promotion gate.
//!
//! The learning loop never silently changes behaviour. A learned candidate
//! default is promoted only if it clears every rule from the spec:
//!
//! - corpus regression is unchanged or intentionally updated
//! - benchmark regression is within budget
//! - the fallback rate does not worsen materially
//! - the ranking improvement is supported by receipts
//! - a human can inspect the change
//!
//! [`evaluate_promotion`] returns a [`PromotionDecision`] that lists *every*
//! reason a candidate was blocked, so the outcome is always explainable — the
//! spec's "learned behavior must be explainable through receipts".

use serde::{Deserialize, Serialize};

/// A proposed change to a learned default, with the evidence for promoting it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PromotionCandidate {
    /// Human-readable name of the default being changed.
    pub name: String,
    /// Net change in corpus regressions vs the current default. `0` = unchanged,
    /// negative = fewer regressions, positive = new regressions.
    pub corpus_regression_delta: i64,
    /// Whether any new corpus regressions were intentionally accepted (e.g. a
    /// deliberately updated expected output).
    pub regressions_intentional: bool,
    /// Benchmark regression vs the current default, as a fraction (`0.05` = 5%
    /// slower). Negative means faster.
    pub benchmark_regression: f64,
    /// Change in fallback rate vs the current default (`0.02` = 2 points worse).
    pub fallback_rate_delta: f64,
    /// Whether the ranking/quality improvement is backed by receipts.
    pub ranking_improvement_supported: bool,
    /// Number of receipts supporting the candidate.
    pub supporting_receipts: usize,
    /// Whether the change is human-inspectable (diffable, documented).
    pub human_inspectable: bool,
}

impl PromotionCandidate {
    /// A neutral candidate that, with defaults, would pass every rule — a base
    /// to tweak in tests and callers.
    #[must_use]
    pub fn clean(name: impl Into<String>, supporting_receipts: usize) -> Self {
        Self {
            name: name.into(),
            corpus_regression_delta: 0,
            regressions_intentional: false,
            benchmark_regression: 0.0,
            fallback_rate_delta: 0.0,
            ranking_improvement_supported: true,
            supporting_receipts,
            human_inspectable: true,
        }
    }
}

/// Thresholds the promotion gate enforces.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PromotionPolicy {
    /// Maximum tolerated benchmark regression (fraction).
    pub max_benchmark_regression: f64,
    /// Maximum tolerated worsening of the fallback rate.
    pub max_fallback_rate_delta: f64,
    /// Minimum supporting receipts.
    pub min_supporting_receipts: usize,
}

impl Default for PromotionPolicy {
    fn default() -> Self {
        Self {
            max_benchmark_regression: 0.05,
            max_fallback_rate_delta: 0.01,
            min_supporting_receipts: 50,
        }
    }
}

/// The outcome of evaluating a candidate against a policy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PromotionDecision {
    /// Name of the evaluated candidate.
    pub name: String,
    /// Whether the candidate may be promoted.
    pub promote: bool,
    /// Every reason the candidate was blocked (empty iff `promote`).
    pub blocking_reasons: Vec<String>,
}

impl PromotionDecision {
    /// A short human summary of the decision.
    #[must_use]
    pub fn summary(&self) -> String {
        if self.promote {
            format!("PROMOTE {}", self.name)
        } else {
            format!(
                "HOLD {} ({} reason(s))",
                self.name,
                self.blocking_reasons.len()
            )
        }
    }
}

/// Evaluate `candidate` against `policy`, collecting every blocking reason.
///
/// The decision is "promote" only if `blocking_reasons` is empty — a single
/// failed rule holds the candidate. Reasons are accumulated rather than
/// short-circuited so a caller sees the full picture in one pass.
#[must_use]
pub fn evaluate_promotion(
    candidate: &PromotionCandidate,
    policy: &PromotionPolicy,
) -> PromotionDecision {
    let mut reasons = Vec::new();

    if candidate.corpus_regression_delta > 0 && !candidate.regressions_intentional {
        reasons.push(format!(
            "introduces {} unintended corpus regression(s)",
            candidate.corpus_regression_delta
        ));
    }
    if candidate.benchmark_regression > policy.max_benchmark_regression {
        reasons.push(format!(
            "benchmark regression {:.1}% exceeds budget {:.1}%",
            candidate.benchmark_regression * 100.0,
            policy.max_benchmark_regression * 100.0
        ));
    }
    if candidate.fallback_rate_delta > policy.max_fallback_rate_delta {
        reasons.push(format!(
            "fallback rate worsens by {:.1} points (budget {:.1})",
            candidate.fallback_rate_delta * 100.0,
            policy.max_fallback_rate_delta * 100.0
        ));
    }
    if !candidate.ranking_improvement_supported {
        reasons.push("ranking improvement is not supported by receipts".to_string());
    }
    if candidate.supporting_receipts < policy.min_supporting_receipts {
        reasons.push(format!(
            "only {} supporting receipt(s); need {}",
            candidate.supporting_receipts, policy.min_supporting_receipts
        ));
    }
    if !candidate.human_inspectable {
        reasons.push("change is not human-inspectable".to_string());
    }

    PromotionDecision {
        name: candidate.name.clone(),
        promote: reasons.is_empty(),
        blocking_reasons: reasons,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_policy_is_conservative() {
        let p = PromotionPolicy::default();
        assert!((p.max_benchmark_regression - 0.05).abs() < 1e-9);
        assert_eq!(p.min_supporting_receipts, 50);
    }

    #[test]
    fn clean_candidate_with_enough_receipts_promotes() {
        let c = PromotionCandidate::clean("planner.default", 100);
        let d = evaluate_promotion(&c, &PromotionPolicy::default());
        assert!(d.promote);
        assert!(d.blocking_reasons.is_empty());
    }

    #[test]
    fn thin_evidence_blocks() {
        let c = PromotionCandidate::clean("planner.default", 10);
        let d = evaluate_promotion(&c, &PromotionPolicy::default());
        assert!(!d.promote);
        assert!(
            d.blocking_reasons
                .iter()
                .any(|r| r.contains("supporting receipt"))
        );
    }

    #[test]
    fn unintended_regression_blocks() {
        let mut c = PromotionCandidate::clean("x", 100);
        c.corpus_regression_delta = 2;
        let d = evaluate_promotion(&c, &PromotionPolicy::default());
        assert!(!d.promote);
        assert!(
            d.blocking_reasons
                .iter()
                .any(|r| r.contains("corpus regression"))
        );
    }

    #[test]
    fn intentional_regression_is_allowed() {
        let mut c = PromotionCandidate::clean("x", 100);
        c.corpus_regression_delta = 2;
        c.regressions_intentional = true;
        let d = evaluate_promotion(&c, &PromotionPolicy::default());
        assert!(d.promote);
    }

    #[test]
    fn fewer_regressions_never_block() {
        let mut c = PromotionCandidate::clean("x", 100);
        c.corpus_regression_delta = -3; // improvement
        let d = evaluate_promotion(&c, &PromotionPolicy::default());
        assert!(d.promote);
    }

    #[test]
    fn benchmark_over_budget_blocks() {
        let mut c = PromotionCandidate::clean("x", 100);
        c.benchmark_regression = 0.10;
        let d = evaluate_promotion(&c, &PromotionPolicy::default());
        assert!(!d.promote);
        assert!(d.blocking_reasons.iter().any(|r| r.contains("benchmark")));
    }

    #[test]
    fn benchmark_improvement_is_fine() {
        let mut c = PromotionCandidate::clean("x", 100);
        c.benchmark_regression = -0.20; // faster
        assert!(evaluate_promotion(&c, &PromotionPolicy::default()).promote);
    }

    #[test]
    fn worse_fallback_rate_blocks() {
        let mut c = PromotionCandidate::clean("x", 100);
        c.fallback_rate_delta = 0.05;
        let d = evaluate_promotion(&c, &PromotionPolicy::default());
        assert!(!d.promote);
        assert!(
            d.blocking_reasons
                .iter()
                .any(|r| r.contains("fallback rate"))
        );
    }

    #[test]
    fn unsupported_ranking_blocks() {
        let mut c = PromotionCandidate::clean("x", 100);
        c.ranking_improvement_supported = false;
        let d = evaluate_promotion(&c, &PromotionPolicy::default());
        assert!(!d.promote);
        assert!(
            d.blocking_reasons
                .iter()
                .any(|r| r.contains("not supported"))
        );
    }

    #[test]
    fn non_inspectable_blocks() {
        let mut c = PromotionCandidate::clean("x", 100);
        c.human_inspectable = false;
        let d = evaluate_promotion(&c, &PromotionPolicy::default());
        assert!(!d.promote);
        assert!(
            d.blocking_reasons
                .iter()
                .any(|r| r.contains("human-inspectable"))
        );
    }

    #[test]
    fn all_reasons_accumulated_not_short_circuited() {
        let c = PromotionCandidate {
            name: "bad".to_string(),
            corpus_regression_delta: 1,
            regressions_intentional: false,
            benchmark_regression: 0.5,
            fallback_rate_delta: 0.5,
            ranking_improvement_supported: false,
            supporting_receipts: 0,
            human_inspectable: false,
        };
        let d = evaluate_promotion(&c, &PromotionPolicy::default());
        assert!(!d.promote);
        // All six rules fire.
        assert_eq!(d.blocking_reasons.len(), 6);
    }

    #[test]
    fn summary_reflects_decision() {
        let promote = evaluate_promotion(
            &PromotionCandidate::clean("good", 100),
            &PromotionPolicy::default(),
        );
        assert!(promote.summary().starts_with("PROMOTE"));
        let hold = evaluate_promotion(
            &PromotionCandidate::clean("thin", 1),
            &PromotionPolicy::default(),
        );
        assert!(hold.summary().starts_with("HOLD"));
    }

    #[test]
    fn decision_round_trips_through_json() {
        let d = evaluate_promotion(
            &PromotionCandidate::clean("good", 100),
            &PromotionPolicy::default(),
        );
        let json = serde_json::to_string(&d).expect("serialize");
        let back: PromotionDecision = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(d, back);
    }
}
