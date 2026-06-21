//! Integration tests: rank reviews built from the on-disk fixture corpus.

use deep_diff_forge_graph::{RiskSignal, rank};
use deep_diff_forge_patch::parse;

fn load(name: &str) -> String {
    let path = format!("{}/../../fixtures/patch/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"))
}

#[test]
fn basic_fixture_ranks_its_single_file() {
    let r = rank(&parse(&load("basic.patch")).unwrap());
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].path, "src/lib.rs");
    assert!(r[0].signals.contains(&RiskSignal::PublicApiSurface));
}

#[test]
fn new_file_fixture_flags_new_file() {
    let r = rank(&parse(&load("new_file.patch")).unwrap());
    assert!(r[0].signals.contains(&RiskSignal::NewFile));
}

#[test]
fn delete_fixture_flags_deletion() {
    let r = rank(&parse(&load("delete_file.patch")).unwrap());
    assert!(r[0].signals.contains(&RiskSignal::DeletedFile));
}

#[test]
fn binary_fixture_flags_binary() {
    let r = rank(&parse(&load("binary.patch")).unwrap());
    assert!(r[0].signals.contains(&RiskSignal::BinaryChange));
}

#[test]
fn lib_rs_outranks_all_other_fixtures_combined() {
    let mut all = String::new();
    for name in ["basic.patch", "new_file.patch", "binary.patch"] {
        all.push_str(&load(name));
    }
    let r = rank(&parse(&all).unwrap());
    // basic.patch touches src/lib.rs (public API) -> ranks first.
    assert_eq!(r[0].path, "src/lib.rs");
}

#[test]
fn ranking_a_real_combined_review_is_deterministic() {
    let mut all = String::new();
    for name in [
        "basic.patch",
        "new_file.patch",
        "delete_file.patch",
        "binary.patch",
    ] {
        all.push_str(&load(name));
    }
    let files = parse(&all).unwrap();
    assert_eq!(rank(&files), rank(&files));
}
