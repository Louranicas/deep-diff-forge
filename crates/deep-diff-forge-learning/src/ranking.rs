//! Ranking learning unit.
//!
//! Learns how much each review signal predicts *realized risk* — a hunk that
//! caused a fix or correlated with a test failure — and turns that into risk
//! weights and a "needs human first" decision. Weights are learned by
//! correlation: a signal earns weight in proportion to how often files carrying
//! it turned out to be risky. The graph crate stays the authority on *what* the
//! signals are; this unit only learns *how much they matter*.

use serde::{Deserialize, Serialize};

use crate::util::clamp_f64;

/// One observed review with its signals and realized outcome.
///
/// The boolean fields are the spec's enumerated review signals (Ranking
/// Learning inputs); modelling each as its own two-variant enum would obscure
/// the one-to-one mapping to the specification without adding safety, so the
/// `struct_excessive_bools` lint is allowed here deliberately.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RankingObservation {
    /// Reviewer opened this file first.
    pub reviewed_first: bool,
    /// Reviewer returned to it after first viewing.
    pub revisited: bool,
    /// Approved with no change requested.
    pub approved_without_changes: bool,
    /// The change ultimately required a follow-up fix.
    pub caused_fix: bool,
    /// The change correlated with a test failure.
    pub correlated_test_failure: bool,
    /// The change touched a public API surface.
    pub public_api_touch: bool,
    /// How many downstream dependents the change reaches.
    pub dependency_fanout: u32,
}

impl RankingObservation {
    /// A benign observation: no risky signals, nothing realized.
    #[must_use]
    pub fn benign() -> Self {
        Self {
            reviewed_first: false,
            revisited: false,
            approved_without_changes: true,
            caused_fix: false,
            correlated_test_failure: false,
            public_api_touch: false,
            dependency_fanout: 0,
        }
    }

    /// Whether this observation realized risk (caused a fix or a test failure).
    /// This is the supervision signal the weights are fit against.
    #[must_use]
    pub fn realized_risk(&self) -> bool {
        self.caused_fix || self.correlated_test_failure
    }

    /// Whether the change had wide blast radius (fan-out above `threshold`).
    #[must_use]
    pub fn wide_fanout(&self, threshold: u32) -> bool {
        self.dependency_fanout > threshold
    }
}

/// Fan-out above which a change is treated as "wide" for weighting.
const WIDE_FANOUT: u32 = 5;

/// Learned per-signal risk weights, each in `0.0..=1.0`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RankingWeights {
    /// Weight for revisited files.
    pub revisited: f64,
    /// Weight for public-API touches.
    pub public_api_touch: f64,
    /// Weight for wide dependency fan-out.
    pub wide_fanout: f64,
    /// Number of observations the weights were fit from.
    pub samples: usize,
}

impl RankingWeights {
    /// Neutral priors used before any data: every signal is mildly risky.
    #[must_use]
    pub fn prior() -> Self {
        Self {
            revisited: 0.5,
            public_api_touch: 0.5,
            wide_fanout: 0.5,
            samples: 0,
        }
    }

    /// A weighted risk score for `obs` in `0.0..=1.0`.
    ///
    /// The score is the average of the weights for the signals present, so it
    /// stays bounded and interpretable regardless of how many signals fire.
    #[must_use]
    pub fn score(&self, obs: &RankingObservation) -> f64 {
        let mut total = 0.0;
        let mut n = 0.0;
        if obs.revisited {
            total += self.revisited;
            n += 1.0;
        }
        if obs.public_api_touch {
            total += self.public_api_touch;
            n += 1.0;
        }
        if obs.wide_fanout(WIDE_FANOUT) {
            total += self.wide_fanout;
            n += 1.0;
        }
        if n == 0.0 { 0.0 } else { total / n }
    }

    /// Whether `obs` should be surfaced to a human first, under `threshold`.
    #[must_use]
    pub fn needs_human_first(&self, obs: &RankingObservation, threshold: f64) -> bool {
        self.score(obs) >= threshold
    }
}

/// Fit [`RankingWeights`] from observations.
///
/// For each signal, the weight is the fraction of observations carrying that
/// signal that realized risk — i.e. `P(risk | signal)`. A signal that never
/// appears keeps its neutral prior so the absence of evidence is not read as
/// "safe".
#[must_use]
pub fn fit_weights(observations: &[RankingObservation]) -> RankingWeights {
    if observations.is_empty() {
        return RankingWeights::prior();
    }
    let prior = RankingWeights::prior();
    RankingWeights {
        revisited: conditional_risk(observations, prior.revisited, |o| o.revisited),
        public_api_touch: conditional_risk(observations, prior.public_api_touch, |o| {
            o.public_api_touch
        }),
        wide_fanout: conditional_risk(observations, prior.wide_fanout, |o| {
            o.wide_fanout(WIDE_FANOUT)
        }),
        samples: observations.len(),
    }
}

/// `P(realized_risk | signal)` over `observations`, or `prior` if the signal
/// never appears.
fn conditional_risk(
    observations: &[RankingObservation],
    prior: f64,
    signal: impl Fn(&RankingObservation) -> bool,
) -> f64 {
    let with_signal: Vec<&RankingObservation> = observations.iter().filter(|o| signal(o)).collect();
    if with_signal.is_empty() {
        return prior;
    }
    let risky = with_signal.iter().filter(|o| o.realized_risk()).count();
    #[allow(clippy::cast_precision_loss)]
    let rate = risky as f64 / with_signal.len() as f64;
    clamp_f64(rate, 0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn risky_public() -> RankingObservation {
        RankingObservation {
            reviewed_first: true,
            revisited: false,
            approved_without_changes: false,
            caused_fix: true,
            correlated_test_failure: false,
            public_api_touch: true,
            dependency_fanout: 0,
        }
    }

    fn safe_public() -> RankingObservation {
        RankingObservation {
            public_api_touch: true,
            ..RankingObservation::benign()
        }
    }

    #[test]
    fn benign_realizes_no_risk() {
        assert!(!RankingObservation::benign().realized_risk());
    }

    #[test]
    fn caused_fix_realizes_risk() {
        let mut o = RankingObservation::benign();
        o.caused_fix = true;
        assert!(o.realized_risk());
    }

    #[test]
    fn test_failure_realizes_risk() {
        let mut o = RankingObservation::benign();
        o.correlated_test_failure = true;
        assert!(o.realized_risk());
    }

    #[test]
    fn wide_fanout_predicate() {
        let mut o = RankingObservation::benign();
        o.dependency_fanout = 10;
        assert!(o.wide_fanout(5));
        assert!(!o.wide_fanout(20));
    }

    #[test]
    fn empty_observations_keep_priors() {
        let w = fit_weights(&[]);
        assert_eq!(w, RankingWeights::prior());
        assert_eq!(w.samples, 0);
    }

    #[test]
    fn signal_that_never_appears_keeps_prior() {
        // Observations with no public-API touch leave that weight at the prior.
        let obs = vec![RankingObservation::benign(); 10];
        let w = fit_weights(&obs);
        assert!((w.public_api_touch - 0.5).abs() < 1e-9);
    }

    #[test]
    fn always_risky_signal_learns_high_weight() {
        let obs = vec![risky_public(); 8];
        let w = fit_weights(&obs);
        assert!((w.public_api_touch - 1.0).abs() < 1e-9);
    }

    #[test]
    fn never_risky_signal_learns_low_weight() {
        let obs = vec![safe_public(); 8];
        let w = fit_weights(&obs);
        assert!((w.public_api_touch - 0.0).abs() < 1e-9);
    }

    #[test]
    fn mixed_signal_learns_proportion() {
        let mut obs = vec![risky_public(); 3];
        obs.extend(vec![safe_public(); 1]);
        let w = fit_weights(&obs);
        // 3 risky of 4 public-API touches.
        assert!((w.public_api_touch - 0.75).abs() < 1e-9);
    }

    #[test]
    fn samples_count_recorded() {
        let obs = vec![RankingObservation::benign(); 7];
        assert_eq!(fit_weights(&obs).samples, 7);
    }

    #[test]
    fn score_zero_when_no_signal_present() {
        let w = RankingWeights::prior();
        assert!((w.score(&RankingObservation::benign()) - 0.0).abs() < 1e-9);
    }

    #[test]
    fn score_averages_present_signals() {
        let w = RankingWeights {
            revisited: 0.2,
            public_api_touch: 0.8,
            wide_fanout: 0.5,
            samples: 1,
        };
        let mut o = RankingObservation::benign();
        o.revisited = true;
        o.public_api_touch = true;
        // average of 0.2 and 0.8
        assert!((w.score(&o) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn needs_human_first_respects_threshold() {
        let w = fit_weights(&[risky_public(); 8]);
        // A change carrying the learned-risky public-API signal is surfaced.
        assert!(w.needs_human_first(&risky_public(), 0.5));
        // A change carrying no risk signal at all scores 0 and is not surfaced —
        // note `safe_public` still touches a public API, which the weights
        // learned is risky, so it is (correctly) flagged; only a signal-free
        // observation is below threshold.
        assert!(!w.needs_human_first(&RankingObservation::benign(), 0.5));
        assert!(w.needs_human_first(&safe_public(), 0.5));
    }

    #[test]
    fn fit_is_deterministic() {
        let obs = vec![risky_public(), safe_public(), risky_public()];
        assert_eq!(fit_weights(&obs), fit_weights(&obs));
    }

    #[test]
    fn weights_are_bounded() {
        let obs = vec![risky_public(); 100];
        let w = fit_weights(&obs);
        assert!(w.public_api_touch >= 0.0 && w.public_api_touch <= 1.0);
        assert!(w.revisited >= 0.0 && w.revisited <= 1.0);
        assert!(w.wide_fanout >= 0.0 && w.wide_fanout <= 1.0);
    }
}
