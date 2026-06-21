//! Integration tests over the on-disk fixture corpus at `fixtures/patch/`.
//!
//! These exercise the public crate boundary (`parse`/`render_unified`/`to_json`)
//! against real patch files, per the deployment framework's Gate 5 (Fixture)
//! and the Testing Gold Standard's integration-coverage requirement.

use deep_diff_forge_core::{FileStatus, PatchLineKind};
use deep_diff_forge_patch::{parse, render_unified};

fn load(name: &str) -> String {
    let path = format!("{}/../../fixtures/patch/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"))
}

#[test]
fn basic_fixture_is_one_modified_file() {
    let files = parse(&load("basic.patch")).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, "src/lib.rs");
    assert_eq!(files[0].status, FileStatus::Modified);
}

#[test]
fn basic_fixture_has_one_hunk_with_an_edit() {
    let files = parse(&load("basic.patch")).unwrap();
    let hunk = &files[0].patch_twin.hunks[0];
    let adds = hunk
        .lines
        .iter()
        .filter(|l| l.kind == PatchLineKind::Added)
        .count();
    let dels = hunk
        .lines
        .iter()
        .filter(|l| l.kind == PatchLineKind::Removed)
        .count();
    assert_eq!((adds, dels), (1, 1));
}

#[test]
fn new_file_fixture_is_added() {
    let files = parse(&load("new_file.patch")).unwrap();
    assert_eq!(files[0].status, FileStatus::Added);
    assert_eq!(files[0].path, "hello.txt");
}

#[test]
fn delete_file_fixture_is_deleted() {
    let files = parse(&load("delete_file.patch")).unwrap();
    assert_eq!(files[0].status, FileStatus::Deleted);
    assert_eq!(files[0].path, "gone.txt");
}

#[test]
fn rename_fixture_is_renamed_to_new_path() {
    let files = parse(&load("rename.patch")).unwrap();
    assert_eq!(files[0].status, FileStatus::Renamed);
    assert_eq!(files[0].path, "new/name.rs");
}

#[test]
fn binary_fixture_is_binary_changed_with_no_hunks() {
    let files = parse(&load("binary.patch")).unwrap();
    assert_eq!(files[0].status, FileStatus::BinaryChanged);
    assert!(files[0].patch_twin.hunks.is_empty());
}

#[test]
fn no_newline_fixture_captures_marker_in_metadata() {
    let files = parse(&load("no_newline.patch")).unwrap();
    assert!(
        files[0]
            .patch_twin
            .metadata
            .iter()
            .any(|m| m.starts_with("\\ No newline"))
    );
}

#[test]
fn every_fixture_parses_without_error() {
    for name in [
        "basic.patch",
        "new_file.patch",
        "delete_file.patch",
        "rename.patch",
        "binary.patch",
        "no_newline.patch",
    ] {
        assert!(parse(&load(name)).is_ok(), "fixture {name} failed to parse");
    }
}

#[test]
fn rendered_basic_is_apply_able_shaped() {
    let files = parse(&load("basic.patch")).unwrap();
    let rendered = render_unified(&files);
    assert!(rendered.contains("--- a/src/lib.rs"));
    assert!(rendered.contains("+++ b/src/lib.rs"));
    assert!(rendered.contains("@@ -1,4 +1,4 @@"));
}
