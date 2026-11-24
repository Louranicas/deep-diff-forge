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

/// S3.34.c, the missed mutant of `texts_found` (the field `released` deleted from the hit it
/// keeps, so the hit keeps the reading's own `false`): outside Rust a hunk read alone is release
/// 0.8.3's reading, so what it finds of a signal 0.8.3 had is a released finding, and a ranked
/// file's bounded list holds those first. A Python, a JavaScript and a shell file each spawning a
/// process, one hunk each: the item is marked released; a Python file whose only finding is a
/// signal 0.8.3 did not have (`test_skipped`) is not.
#[test]
fn a_finding_outside_rust_of_a_signal_release_had_is_marked_released() {
    use deep_diff_forge_graph::{file_evidence, file_evidence_released};
    let marks = |path: &str, line: &str| {
        let patch = format!(
            "diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n@@ -1,2 +1,3 @@\n keep = 1\n+{line}\n other = 2\n"
        );
        let files = parse(&patch).unwrap();
        let items = file_evidence(&files[0]);
        let signals: Vec<String> = items.iter().map(|e| format!("{:?}", e.signal)).collect();
        (signals, file_evidence_released(&files[0]))
    };
    for (path, line) in [
        ("tools/run.py", "subprocess.run([\"ls\"])"),
        ("web/run.js", "require(\"child_process\").exec(\"ls\")"),
        ("scripts/run.sh", "eval \"$cmd\""),
    ] {
        let (signals, released) = marks(path, line);
        assert!(!signals.is_empty(), "{path}: nothing found in {line:?}");
        assert!(
            released.iter().all(|&r| r),
            "{path}: a finding of a signal release 0.8.3 had, in a hunk read alone, is not marked released: {signals:?} {released:?}"
        );
    }
    let (signals, released) = marks("tests/test_a.py", "@pytest.mark.skip");
    assert_eq!(signals, ["TestSkipped"], "the near miss found otherwise");
    assert_eq!(
        released,
        [false],
        "a signal release 0.8.3 did not have was marked released"
    );
}
