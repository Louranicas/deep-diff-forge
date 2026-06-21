//! Annotation learning unit.
//!
//! Scores how much to trust an agent's annotations based on their track record:
//! how often they carried evidence, were accepted, were contradicted by tests
//! or source, or were overridden by a reviewer. The cardinal rule from the
//! spec — *agent annotations are untrusted until grounded* — is enforced
//! structurally: a source with no grounded, accepted evidence can never rise
//! above [`TrustTier::Untrusted`], no matter how many annotations it emits.

use serde::{Deserialize, Serialize};

use crate::util::clamp_f64;

/// One observed annotation from an agent source.
///
/// The boolean fields are the spec's enumerated Annotation Learning inputs
/// (accepted / rejected / contradicted / overridden); they map one-to-one to
/// the specification, so the `struct_excessive_bools` lint is allowed here
/// deliberately rather than splintering the contract into enums.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnnotationObservation {
    /// How many evidence spans the annotation cited (0 = ungrounded claim).
    pub evidence_spans: u32,
    /// The reviewer accepted the annotation.
    pub accepted: bool,
    /// The reviewer rejected the annotation.
    pub rejected: bool,
    /// The annotation was contradicted by tests or source.
    pub contradicted: bool,
    /// The reviewer manually overrode the annotation.
    pub reviewer_override: bool,
}

impl AnnotationObservation {
    /// An ungrounded, unresolved observation (the default an agent starts at).
    #[must_use]
    pub fn ungrounded() -> Self {
        Self {
            evidence_spans: 0,
            accepted: false,
            rejected: false,
            contradicted: false,
            reviewer_override: false,
        }
    }

    /// Whether the annotation was grounded in at least one evidence span.
    #[must_use]
    pub fn is_grounded(&self) -> bool {
        self.evidence_spans > 0
    }

    /// Whether this is a "clean win": grounded, accepted, not contradicted, not
    /// overridden. This is the only observation that builds trust.
    #[must_use]
    pub fn is_clean_win(&self) -> bool {
        self.is_grounded() && self.accepted && !self.contradicted && !self.reviewer_override
    }
}

/// How much an annotation source is trusted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustTier {
    /// Default — display ungrounded, never auto-acted-upon.
    Untrusted,
    /// Some grounded wins; display normally.
    Provisional,
    /// Consistent grounded wins; display prominently.
    Trusted,
}

impl TrustTier {
    /// Stable lowercase label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Untrusted => "untrusted",
            Self::Provisional => "provisional",
            Self::Trusted => "trusted",
        }
    }

    /// Display prominence implied by the tier, `0..=100`.
    #[must_use]
    pub fn display_prominence(self) -> u8 {
        match self {
            Self::Untrusted => 10,
            Self::Provisional => 50,
            Self::Trusted => 90,
        }
    }
}

/// The learned trust profile for one annotation source.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AnnotationTrust {
    /// Quality score in `0.0..=1.0` (grounded-win rate, penalized by harm).
    pub source_quality: f64,
    /// Discrete trust tier derived from the quality and grounding history.
    pub tier: TrustTier,
    /// Display prominence, `0..=100`.
    pub display_prominence: u8,
    /// Observations the profile was fit from.
    pub samples: usize,
}

impl AnnotationTrust {
    /// The starting profile for an unseen source: untrusted, no evidence.
    #[must_use]
    pub fn unseen() -> Self {
        Self {
            source_quality: 0.0,
            tier: TrustTier::Untrusted,
            display_prominence: TrustTier::Untrusted.display_prominence(),
            samples: 0,
        }
    }
}

/// Minimum clean wins required to leave [`TrustTier::Untrusted`].
const MIN_GROUNDED_WINS: usize = 3;
/// Quality at or above which a source becomes [`TrustTier::Trusted`].
const TRUSTED_QUALITY: f64 = 0.80;
/// Quality at or above which a source becomes [`TrustTier::Provisional`].
const PROVISIONAL_QUALITY: f64 = 0.50;

/// Fit an [`AnnotationTrust`] profile for one source from its observations.
///
/// Quality is the clean-win rate minus a penalty for contradictions (the most
/// harmful outcome — a confidently wrong annotation). The tier gate is the
/// safety invariant: regardless of quality, a source with fewer than
/// [`MIN_GROUNDED_WINS`] clean wins stays [`TrustTier::Untrusted`].
#[must_use]
pub fn fit_trust(observations: &[AnnotationObservation]) -> AnnotationTrust {
    if observations.is_empty() {
        return AnnotationTrust::unseen();
    }
    let n = observations.len();
    let clean_wins = observations.iter().filter(|o| o.is_clean_win()).count();
    let contradictions = observations.iter().filter(|o| o.contradicted).count();

    #[allow(clippy::cast_precision_loss)]
    let win_rate = clean_wins as f64 / n as f64;
    #[allow(clippy::cast_precision_loss)]
    let harm_rate = contradictions as f64 / n as f64;
    // A contradiction costs double a missing win — being confidently wrong is
    // worse than being silent.
    let source_quality = clamp_f64(win_rate - harm_rate, 0.0, 1.0);

    let tier = if clean_wins < MIN_GROUNDED_WINS {
        TrustTier::Untrusted
    } else if source_quality >= TRUSTED_QUALITY {
        TrustTier::Trusted
    } else if source_quality >= PROVISIONAL_QUALITY {
        TrustTier::Provisional
    } else {
        TrustTier::Untrusted
    };

    AnnotationTrust {
        source_quality,
        tier,
        display_prominence: tier.display_prominence(),
        samples: n,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clean_win() -> AnnotationObservation {
        AnnotationObservation {
            evidence_spans: 2,
            accepted: true,
            rejected: false,
            contradicted: false,
            reviewer_override: false,
        }
    }

    fn ungrounded_accept() -> AnnotationObservation {
        AnnotationObservation {
            evidence_spans: 0,
            accepted: true,
            ..AnnotationObservation::ungrounded()
        }
    }

    fn contradiction() -> AnnotationObservation {
        AnnotationObservation {
            evidence_spans: 1,
            accepted: false,
            rejected: true,
            contradicted: true,
            reviewer_override: false,
        }
    }

    #[test]
    fn ungrounded_is_not_grounded() {
        assert!(!AnnotationObservation::ungrounded().is_grounded());
    }

    #[test]
    fn grounded_requires_evidence_span() {
        assert!(clean_win().is_grounded());
        assert!(!ungrounded_accept().is_grounded());
    }

    #[test]
    fn clean_win_requires_grounding_and_acceptance() {
        assert!(clean_win().is_clean_win());
        assert!(!ungrounded_accept().is_clean_win()); // accepted but ungrounded
        assert!(!contradiction().is_clean_win());
    }

    #[test]
    fn override_disqualifies_clean_win() {
        let mut o = clean_win();
        o.reviewer_override = true;
        assert!(!o.is_clean_win());
    }

    #[test]
    fn tier_labels_and_prominence_ordered() {
        assert_eq!(TrustTier::Untrusted.label(), "untrusted");
        assert!(
            TrustTier::Untrusted.display_prominence() < TrustTier::Provisional.display_prominence()
        );
        assert!(
            TrustTier::Provisional.display_prominence() < TrustTier::Trusted.display_prominence()
        );
    }

    #[test]
    fn unseen_source_is_untrusted() {
        let t = AnnotationTrust::unseen();
        assert_eq!(t.tier, TrustTier::Untrusted);
        assert_eq!(t.samples, 0);
    }

    #[test]
    fn empty_observations_yield_unseen() {
        assert_eq!(fit_trust(&[]), AnnotationTrust::unseen());
    }

    #[test]
    fn ungrounded_acceptances_never_build_trust() {
        // The safety invariant: a source can be accepted many times, but if it
        // never grounds its claims it stays Untrusted.
        let obs = vec![ungrounded_accept(); 50];
        let t = fit_trust(&obs);
        assert_eq!(t.tier, TrustTier::Untrusted);
        assert!(t.source_quality < f64::EPSILON);
    }

    #[test]
    fn too_few_wins_stays_untrusted_even_if_perfect() {
        let obs = vec![clean_win(); 2]; // below MIN_GROUNDED_WINS
        let t = fit_trust(&obs);
        assert_eq!(t.tier, TrustTier::Untrusted);
    }

    #[test]
    fn consistent_grounded_wins_become_trusted() {
        let obs = vec![clean_win(); 10];
        let t = fit_trust(&obs);
        assert_eq!(t.tier, TrustTier::Trusted);
        assert!((t.source_quality - 1.0).abs() < 1e-9);
        assert_eq!(t.display_prominence, 90);
    }

    #[test]
    fn mixed_record_is_provisional() {
        let mut obs = vec![clean_win(); 6];
        obs.extend(vec![ungrounded_accept(); 4]); // dilute the win rate to 0.6
        let t = fit_trust(&obs);
        assert_eq!(t.tier, TrustTier::Provisional);
    }

    #[test]
    fn contradictions_penalize_quality() {
        let mut obs = vec![clean_win(); 5];
        obs.extend(vec![contradiction(); 5]);
        let t = fit_trust(&obs);
        // win_rate 0.5 - harm_rate 0.5 = 0.0
        assert!(t.source_quality < f64::EPSILON);
        assert_eq!(t.tier, TrustTier::Untrusted);
    }

    #[test]
    fn quality_is_bounded() {
        let obs = vec![contradiction(); 10];
        let t = fit_trust(&obs);
        assert!(t.source_quality >= 0.0 && t.source_quality <= 1.0);
    }

    #[test]
    fn samples_recorded() {
        let obs = vec![clean_win(); 4];
        assert_eq!(fit_trust(&obs).samples, 4);
    }

    #[test]
    fn fit_is_deterministic() {
        let obs = vec![clean_win(), contradiction(), clean_win()];
        assert_eq!(fit_trust(&obs), fit_trust(&obs));
    }
}
