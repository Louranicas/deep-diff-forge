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
pub use util::{RedactedId, redacted_id};

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
                StrategyReceipt::new(crate::redacted_id("h"), "rust", "v", strategy)
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
        let scratch = deep_diff_forge_testkit::Scratch::new("learn-report");
        let dir = scratch.join("absent");
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

    /// SECURITY.md (learning store): "Learning data is not uploaded by the project." A small
    /// negative surface: the crate that holds the learning data has no way of its own to the
    /// network. Its normal dependencies are exactly `serde`, `serde_json` and `rustix`, `rustix`
    /// with only its `process` feature (not `net`), and it has no target-specific or build
    /// dependencies; its code outside the test modules names no socket or HTTP client and spawns
    /// no program. A change that adds any of these turns this red, so its author must say why it
    /// is not an upload. Not proven here: that `serde`, `serde_json` or `rustix`'s `process`
    /// module cannot reach a network (they are not read); that no other crate sends what this
    /// crate stores (the CLI's learn commands are held by the CLI's `tests/learn.rs`; the TUI,
    /// which reads the store, is not held). A copy the user makes or a command the user runs is
    /// not an upload by the project.
    #[test]
    fn the_learning_crate_has_no_way_of_its_own_to_the_network() {
        // A line reader over this manifest's own layout, not a TOML parser.
        let manifest = include_str!("../Cargo.toml");
        let mut section = "";
        let mut dependency_sections = Vec::new();
        let mut dependencies = Vec::new();
        for line in manifest.lines().map(str::trim) {
            if line.starts_with('[') {
                section = line;
                if line.contains("dependencies") {
                    dependency_sections.push(line);
                }
            } else if section == "[dependencies]" && !line.is_empty() && !line.starts_with('#') {
                dependencies.push(line);
            }
        }
        assert_eq!(
            dependency_sections,
            ["[dependencies]", "[dev-dependencies]"],
            "the learning crate's dependency sections"
        );
        let names: Vec<&str> = dependencies
            .iter()
            .map(|line| line.split(['=', '.', ' ']).next().unwrap_or_default())
            .collect();
        assert_eq!(
            names,
            ["serde", "serde_json", "rustix"],
            "the learning crate's dependencies"
        );
        assert!(
            dependencies.contains(&r#"rustix = { version = "1.1", features = ["process"] }"#),
            "rustix with a feature other than `process`: {dependencies:?}"
        );
        let mut files = vec![std::path::PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src"
        ))];
        let mut read = 0;
        while let Some(path) = files.pop() {
            if path.is_dir() {
                files.extend(
                    std::fs::read_dir(&path)
                        .unwrap()
                        .map(|entry| entry.unwrap().path()),
                );
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            let code = text
                .split_once("\n#[cfg(test)]\nmod tests {")
                .map_or(text.as_str(), |(code, _)| code);
            for name in [
                "std::net",
                "TcpStream",
                "TcpListener",
                "UdpSocket",
                "UnixStream",
                "UnixListener",
                "UnixDatagram",
                "rustix::net",
                "Command",
                "reqwest",
                "ureq",
                "hyper",
                "http://",
                "https://",
            ] {
                assert!(
                    !code.contains(name),
                    "{} names `{name}` outside its tests",
                    path.display()
                );
            }
            read += 1;
        }
        assert!(read >= 10, "only {read} source files were read");
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
