//! Bounded parallel dimensional execution for Deep-Diff-Forge (L6).
//!
//! A cluster is a scheduler, not a different engine: it splits the review into
//! deterministic contiguous shards, runs lanes across bounded std threads
//! ([`run_lane`]), and rejoins by an explicit [`JoinPolicy`]. The defining
//! guarantee is **determinism under parallelism** — the output is identical for
//! any worker count — backed by a structured [`ClusterReceipt`]. No external
//! dependency and no `unsafe`; remote/distributed execution is out of scope
//! until local receipts and replay are boring and testable.
//!
//! [`JoinPolicy`]: deep_diff_forge_core::JoinPolicy

mod cluster;
mod scheduler;

pub use cluster::{
    ClusterReceipt, ClusterRun, apply_join, dimension_label, join_label, parallelism_label,
    run_risk_cluster,
};
pub use scheduler::{contiguous_chunks, resolve_workers, run_lane};

#[cfg(test)]
mod tests {
    use super::*;
    use deep_diff_forge_core::{JoinPolicy, Parallelism};
    use deep_diff_forge_patch::parse;

    #[test]
    fn facade_run_risk_cluster_works() {
        let files = parse("--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@\n-a\n+b\n").unwrap();
        let run = run_risk_cluster(&files, Parallelism::Auto, JoinPolicy::RankedReviewOrder);
        assert_eq!(run.ranked.len(), 1);
        assert_eq!(run.receipt.file_count, 1);
    }

    #[test]
    fn facade_run_lane_works() {
        let files = parse("--- a/x.rs\n+++ b/x.rs\n@@ -1,1 +1,1 @@\n-a\n+b\n").unwrap();
        let out = run_lane(&files, Parallelism::Serial, |i, _| i);
        assert_eq!(out, vec![0]);
    }

    #[test]
    fn facade_helpers_exported() {
        assert_eq!(resolve_workers(Parallelism::Serial, 5), 1);
        assert_eq!(contiguous_chunks(4, 2).len(), 2);
        assert_eq!(
            join_label(JoinPolicy::RankedReviewOrder),
            "ranked-review-order"
        );
    }
}
