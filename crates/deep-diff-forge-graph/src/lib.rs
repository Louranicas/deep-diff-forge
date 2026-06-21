//! Review Intelligence Graph for Deep-Diff-Forge (L5).
//!
//! Ranks a review by likely impact rather than file order, using deterministic,
//! explainable signals derived from the patch model (public-API surface, change
//! size, hunk count, new/deleted/binary, generated/vendored suppression,
//! test-only de-prioritization). Ranking never mutates patch truth and is
//! reproducible: the same review always yields the same order.
//!
//! Today the signals are computed from patch facts alone; richer signals
//! (control-flow edits, dependency fan-out, symbol-level risk) arrive once the
//! semantic and Git-input waves feed this layer.

mod rank;
mod risk;

pub use rank::{RankedFile, rank};
pub use risk::{LARGE_CHANGE_LINES, MANY_HUNKS, RiskSignal, change_counts, score_file};

#[cfg(test)]
mod tests {
    use super::*;
    use deep_diff_forge_patch::parse;

    #[test]
    fn public_api_facade_reexports_work() {
        let files = parse("--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@\n-a\n+b\n").unwrap();
        let ranked = rank(&files);
        assert_eq!(ranked.len(), 1);
        assert!(ranked[0].signals.contains(&RiskSignal::PublicApiSurface));
    }

    #[test]
    fn score_file_reexport_works() {
        let files = parse("--- a/x.rs\n+++ b/x.rs\n@@ -1,1 +1,1 @@\n-a\n+b\n").unwrap();
        let (score, _) = score_file(&files[0]);
        assert!(score >= 1);
    }

    #[test]
    fn change_counts_reexport_works() {
        let files = parse("--- a/x\n+++ b/x\n@@ -1,1 +1,1 @@\n-a\n+b\n").unwrap();
        assert_eq!(change_counts(&files[0]), (1, 1));
    }

    #[test]
    fn thresholds_are_exposed() {
        assert_eq!(LARGE_CHANGE_LINES, 80);
        assert_eq!(MANY_HUNKS, 5);
    }
}
