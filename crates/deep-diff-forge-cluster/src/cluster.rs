use crate::scheduler::{resolve_workers, run_lane};
use deep_diff_forge_core::{ExecutionDimension, JoinPolicy, Parallelism, ReviewFile};
use deep_diff_forge_graph::{RankedFile, score_file};

/// A structured record of a cluster run (the framework's dimensional receipt).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClusterReceipt {
    /// Dimensions executed.
    pub dimensions: Vec<ExecutionDimension>,
    /// Requested parallelism.
    pub parallelism: Parallelism,
    /// Join policy applied to the output.
    pub join_policy: JoinPolicy,
    /// Number of files processed.
    pub file_count: usize,
    /// Concrete worker count used.
    pub worker_count: usize,
}

/// The result of a cluster run: joined output plus its receipt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClusterRun {
    /// Ranked/joined files.
    pub ranked: Vec<RankedFile>,
    /// Receipt describing the run.
    pub receipt: ClusterReceipt,
}

/// Run the patch+risk dimensions over the review with bounded parallelism, then
/// join deterministically. The result is identical for any worker count.
#[must_use]
pub fn run_risk_cluster(
    files: &[ReviewFile],
    parallelism: Parallelism,
    join: JoinPolicy,
) -> ClusterRun {
    let scored: Vec<RankedFile> = run_lane(files, parallelism, |_, file| {
        let (score, signals) = score_file(file);
        RankedFile {
            path: file.path.clone(),
            status: file.status,
            score,
            signals,
        }
    });
    let ranked = apply_join(scored, join);
    let receipt = ClusterReceipt {
        dimensions: vec![ExecutionDimension::Patch, ExecutionDimension::Risk],
        parallelism,
        join_policy: join,
        file_count: files.len(),
        worker_count: resolve_workers(parallelism, files.len()),
    };
    ClusterRun { ranked, receipt }
}

/// Apply a join policy to lane results.
///
/// `DeterministicInputOrder` and `AsReadyWithStableIds` both preserve input
/// order (in this batch scheduler, as-ready collapses to deterministic order);
/// `RankedReviewOrder` sorts by descending score with a path tie-break.
#[must_use]
pub fn apply_join(mut results: Vec<RankedFile>, policy: JoinPolicy) -> Vec<RankedFile> {
    match policy {
        JoinPolicy::DeterministicInputOrder | JoinPolicy::AsReadyWithStableIds => results,
        JoinPolicy::RankedReviewOrder => {
            results.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.path.cmp(&b.path)));
            results
        }
    }
}

/// Human label for a parallelism setting.
#[must_use]
pub fn parallelism_label(parallelism: Parallelism) -> String {
    match parallelism {
        Parallelism::Serial => "serial".to_string(),
        Parallelism::Auto => "auto".to_string(),
        Parallelism::Fixed(n) => format!("fixed:{n}"),
    }
}

/// Stable label for a join policy.
#[must_use]
pub fn join_label(policy: JoinPolicy) -> &'static str {
    match policy {
        JoinPolicy::DeterministicInputOrder => "deterministic-input-order",
        JoinPolicy::RankedReviewOrder => "ranked-review-order",
        JoinPolicy::AsReadyWithStableIds => "as-ready-with-stable-ids",
    }
}

/// Stable label for an execution dimension.
#[must_use]
pub fn dimension_label(dimension: ExecutionDimension) -> &'static str {
    match dimension {
        ExecutionDimension::Patch => "patch",
        ExecutionDimension::Semantic => "semantic",
        ExecutionDimension::Risk => "risk",
        ExecutionDimension::Agent => "agent",
        ExecutionDimension::Runtime => "runtime",
        ExecutionDimension::Storage => "storage",
        ExecutionDimension::History => "history",
        ExecutionDimension::Presentation => "presentation",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deep_diff_forge_patch::parse;

    const MULTI: &str = "\
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,1 +1,1 @@
-a
+b
--- a/src/other.rs
+++ b/src/other.rs
@@ -1,1 +1,1 @@
-a
+b
--- a/tests/it.rs
+++ b/tests/it.rs
@@ -1,1 +1,1 @@
-a
+b
";

    fn review() -> Vec<ReviewFile> {
        parse(MULTI).unwrap()
    }

    fn many(n: usize) -> Vec<ReviewFile> {
        use std::fmt::Write as _;
        let mut s = String::new();
        for i in 0..n {
            let _ = write!(
                s,
                "--- a/src/f{i}.rs\n+++ b/src/f{i}.rs\n@@ -1,1 +1,1 @@\n-a\n+b\n"
            );
        }
        parse(&s).unwrap()
    }

    #[test]
    fn cluster_ranks_public_api_first() {
        let run = run_risk_cluster(&review(), Parallelism::Auto, JoinPolicy::RankedReviewOrder);
        assert_eq!(run.ranked[0].path, "src/lib.rs");
    }

    #[test]
    fn cluster_result_is_identical_across_parallelism() {
        let files = many(40);
        let serial = run_risk_cluster(&files, Parallelism::Serial, JoinPolicy::RankedReviewOrder);
        let p4 = run_risk_cluster(&files, Parallelism::Fixed(4), JoinPolicy::RankedReviewOrder);
        let auto = run_risk_cluster(&files, Parallelism::Auto, JoinPolicy::RankedReviewOrder);
        assert_eq!(serial.ranked, p4.ranked);
        assert_eq!(serial.ranked, auto.ranked);
    }

    #[test]
    fn input_order_join_matches_serial_score_order() {
        let files = many(10);
        let run = run_risk_cluster(
            &files,
            Parallelism::Fixed(3),
            JoinPolicy::DeterministicInputOrder,
        );
        let paths: Vec<&str> = run.ranked.iter().map(|r| r.path.as_str()).collect();
        let expected: Vec<String> = (0..10).map(|i| format!("src/f{i}.rs")).collect();
        assert_eq!(
            paths,
            expected.iter().map(String::as_str).collect::<Vec<_>>()
        );
    }

    #[test]
    fn ranked_join_orders_by_score_desc() {
        let run = run_risk_cluster(
            &review(),
            Parallelism::Serial,
            JoinPolicy::RankedReviewOrder,
        );
        for pair in run.ranked.windows(2) {
            assert!(pair[0].score >= pair[1].score);
        }
    }

    #[test]
    fn as_ready_collapses_to_input_order_in_batch() {
        let files = many(8);
        let a = run_risk_cluster(
            &files,
            Parallelism::Fixed(3),
            JoinPolicy::AsReadyWithStableIds,
        );
        let b = run_risk_cluster(
            &files,
            Parallelism::Fixed(3),
            JoinPolicy::DeterministicInputOrder,
        );
        assert_eq!(a.ranked, b.ranked);
    }

    #[test]
    fn receipt_records_dimensions() {
        let run = run_risk_cluster(
            &review(),
            Parallelism::Serial,
            JoinPolicy::RankedReviewOrder,
        );
        assert_eq!(
            run.receipt.dimensions,
            vec![ExecutionDimension::Patch, ExecutionDimension::Risk]
        );
    }

    #[test]
    fn receipt_records_file_count() {
        let run = run_risk_cluster(
            &review(),
            Parallelism::Serial,
            JoinPolicy::RankedReviewOrder,
        );
        assert_eq!(run.receipt.file_count, 3);
    }

    #[test]
    fn receipt_records_worker_count() {
        let run = run_risk_cluster(
            &review(),
            Parallelism::Fixed(2),
            JoinPolicy::RankedReviewOrder,
        );
        assert_eq!(run.receipt.worker_count, 2);
    }

    #[test]
    fn receipt_records_join_policy() {
        let run = run_risk_cluster(
            &review(),
            Parallelism::Serial,
            JoinPolicy::RankedReviewOrder,
        );
        assert_eq!(run.receipt.join_policy, JoinPolicy::RankedReviewOrder);
    }

    #[test]
    fn empty_review_clusters_empty() {
        let run = run_risk_cluster(&[], Parallelism::Auto, JoinPolicy::RankedReviewOrder);
        assert!(run.ranked.is_empty());
        assert_eq!(run.receipt.file_count, 0);
    }

    #[test]
    fn apply_join_input_order_is_identity() {
        let run = run_risk_cluster(
            &review(),
            Parallelism::Serial,
            JoinPolicy::DeterministicInputOrder,
        );
        let again = apply_join(run.ranked.clone(), JoinPolicy::DeterministicInputOrder);
        assert_eq!(run.ranked, again);
    }

    #[test]
    fn parallelism_labels() {
        assert_eq!(parallelism_label(Parallelism::Serial), "serial");
        assert_eq!(parallelism_label(Parallelism::Auto), "auto");
        assert_eq!(parallelism_label(Parallelism::Fixed(4)), "fixed:4");
    }

    #[test]
    fn join_labels() {
        assert_eq!(
            join_label(JoinPolicy::DeterministicInputOrder),
            "deterministic-input-order"
        );
        assert_eq!(
            join_label(JoinPolicy::RankedReviewOrder),
            "ranked-review-order"
        );
        assert_eq!(
            join_label(JoinPolicy::AsReadyWithStableIds),
            "as-ready-with-stable-ids"
        );
    }

    #[test]
    fn dimension_labels_cover_all() {
        assert_eq!(dimension_label(ExecutionDimension::Patch), "patch");
        assert_eq!(dimension_label(ExecutionDimension::Risk), "risk");
        assert_eq!(dimension_label(ExecutionDimension::Semantic), "semantic");
        assert_eq!(
            dimension_label(ExecutionDimension::Presentation),
            "presentation"
        );
    }

    #[test]
    fn large_cluster_preserves_all_files() {
        let files = many(100);
        let run = run_risk_cluster(
            &files,
            Parallelism::Fixed(8),
            JoinPolicy::DeterministicInputOrder,
        );
        assert_eq!(run.ranked.len(), 100);
    }

    #[test]
    fn cluster_scores_match_direct_scoring() {
        let files = review();
        let run = run_risk_cluster(
            &files,
            Parallelism::Fixed(2),
            JoinPolicy::DeterministicInputOrder,
        );
        for (file, ranked) in files.iter().zip(run.ranked.iter()) {
            assert_eq!(ranked.score, score_file(file).0);
        }
    }

    #[test]
    fn fixed_one_matches_serial() {
        let files = many(20);
        let one = run_risk_cluster(
            &files,
            Parallelism::Fixed(1),
            JoinPolicy::DeterministicInputOrder,
        );
        let serial = run_risk_cluster(
            &files,
            Parallelism::Serial,
            JoinPolicy::DeterministicInputOrder,
        );
        assert_eq!(one.ranked, serial.ranked);
    }

    #[test]
    fn receipt_records_parallelism_setting() {
        let run = run_risk_cluster(
            &review(),
            Parallelism::Fixed(3),
            JoinPolicy::RankedReviewOrder,
        );
        assert_eq!(run.receipt.parallelism, Parallelism::Fixed(3));
    }

    #[test]
    fn auto_worker_count_is_at_least_one() {
        let run = run_risk_cluster(&review(), Parallelism::Auto, JoinPolicy::RankedReviewOrder);
        assert!(run.receipt.worker_count >= 1);
    }

    #[test]
    fn apply_join_ranked_sorts_descending() {
        let files = many(5);
        let run = run_risk_cluster(
            &files,
            Parallelism::Serial,
            JoinPolicy::DeterministicInputOrder,
        );
        let ranked = apply_join(run.ranked.clone(), JoinPolicy::RankedReviewOrder);
        for pair in ranked.windows(2) {
            assert!(pair[0].score >= pair[1].score);
        }
    }

    #[test]
    fn large_ranked_cluster_is_deterministic() {
        let files = many(120);
        let a = run_risk_cluster(&files, Parallelism::Fixed(3), JoinPolicy::RankedReviewOrder);
        let b = run_risk_cluster(&files, Parallelism::Fixed(9), JoinPolicy::RankedReviewOrder);
        assert_eq!(a.ranked, b.ranked);
    }

    #[test]
    fn dimension_labels_cover_remaining_variants() {
        assert_eq!(dimension_label(ExecutionDimension::Agent), "agent");
        assert_eq!(dimension_label(ExecutionDimension::Runtime), "runtime");
        assert_eq!(dimension_label(ExecutionDimension::Storage), "storage");
        assert_eq!(dimension_label(ExecutionDimension::History), "history");
    }

    #[test]
    fn cluster_run_is_cloneable_and_eq() {
        let run = run_risk_cluster(
            &review(),
            Parallelism::Serial,
            JoinPolicy::RankedReviewOrder,
        );
        assert_eq!(run.clone(), run);
    }
}
