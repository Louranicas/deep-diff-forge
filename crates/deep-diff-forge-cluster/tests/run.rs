//! Integration tests: cluster real reviews, asserting determinism + receipts.

use deep_diff_forge_cluster::{run_lane, run_risk_cluster};
use deep_diff_forge_core::{ExecutionDimension, JoinPolicy, Parallelism};
use deep_diff_forge_patch::parse;

fn load(name: &str) -> String {
    let path = format!("{}/../../fixtures/patch/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"))
}

fn combined() -> Vec<deep_diff_forge_core::ReviewFile> {
    let mut all = String::new();
    for name in [
        "basic.patch",
        "new_file.patch",
        "delete_file.patch",
        "binary.patch",
    ] {
        all.push_str(&load(name));
    }
    parse(&all).unwrap()
}

#[test]
fn fixture_cluster_ranks_public_api_first() {
    let run = run_risk_cluster(
        &combined(),
        Parallelism::Auto,
        JoinPolicy::RankedReviewOrder,
    );
    assert_eq!(run.ranked[0].path, "src/lib.rs");
}

#[test]
fn fixture_cluster_is_deterministic_serial_vs_parallel() {
    let files = combined();
    let serial = run_risk_cluster(&files, Parallelism::Serial, JoinPolicy::RankedReviewOrder);
    let p3 = run_risk_cluster(&files, Parallelism::Fixed(3), JoinPolicy::RankedReviewOrder);
    assert_eq!(serial.ranked, p3.ranked);
}

#[test]
fn fixture_cluster_receipt_counts_files() {
    let run = run_risk_cluster(
        &combined(),
        Parallelism::Serial,
        JoinPolicy::RankedReviewOrder,
    );
    assert_eq!(run.receipt.file_count, 4);
}

#[test]
fn fixture_cluster_receipt_lists_patch_and_risk() {
    let run = run_risk_cluster(
        &combined(),
        Parallelism::Serial,
        JoinPolicy::RankedReviewOrder,
    );
    assert!(run.receipt.dimensions.contains(&ExecutionDimension::Patch));
    assert!(run.receipt.dimensions.contains(&ExecutionDimension::Risk));
}

#[test]
fn input_order_join_preserves_fixture_order() {
    let run = run_risk_cluster(
        &combined(),
        Parallelism::Fixed(2),
        JoinPolicy::DeterministicInputOrder,
    );
    let paths: Vec<&str> = run.ranked.iter().map(|r| r.path.as_str()).collect();
    assert_eq!(
        paths,
        vec!["src/lib.rs", "hello.txt", "gone.txt", "logo.png"]
    );
}

#[test]
fn large_synthetic_cluster_determinism() {
    use std::fmt::Write as _;
    let mut s = String::new();
    for i in 0..200 {
        let _ = write!(
            s,
            "--- a/src/f{i}.rs\n+++ b/src/f{i}.rs\n@@ -1,1 +1,1 @@\n-a\n+b\n"
        );
    }
    let files = parse(&s).unwrap();
    let serial = run_risk_cluster(
        &files,
        Parallelism::Serial,
        JoinPolicy::DeterministicInputOrder,
    );
    let p8 = run_risk_cluster(
        &files,
        Parallelism::Fixed(8),
        JoinPolicy::DeterministicInputOrder,
    );
    assert_eq!(serial.ranked, p8.ranked);
    assert_eq!(p8.ranked.len(), 200);
}

#[test]
fn run_lane_over_fixtures_returns_paths_in_order() {
    let files = combined();
    let out = run_lane(&files, Parallelism::Fixed(4), |_, f| f.path.clone());
    assert_eq!(out, vec!["src/lib.rs", "hello.txt", "gone.txt", "logo.png"]);
}

#[test]
fn ranked_join_is_descending_over_fixtures() {
    let run = run_risk_cluster(
        &combined(),
        Parallelism::Auto,
        JoinPolicy::RankedReviewOrder,
    );
    for pair in run.ranked.windows(2) {
        assert!(pair[0].score >= pair[1].score);
    }
}

#[test]
fn auto_and_serial_agree_on_fixtures() {
    let files = combined();
    let auto = run_risk_cluster(&files, Parallelism::Auto, JoinPolicy::RankedReviewOrder);
    let serial = run_risk_cluster(&files, Parallelism::Serial, JoinPolicy::RankedReviewOrder);
    assert_eq!(auto.ranked, serial.ranked);
}

#[test]
fn empty_cluster_has_empty_receipt() {
    let run = run_risk_cluster(&[], Parallelism::Auto, JoinPolicy::RankedReviewOrder);
    assert_eq!(run.receipt.file_count, 0);
    assert!(run.ranked.is_empty());
}
