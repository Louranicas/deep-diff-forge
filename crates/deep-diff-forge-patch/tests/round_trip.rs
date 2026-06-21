//! Round-trip property tests: parsing the rendered output of a parsed patch
//! must reproduce the same model (`parse(render(parse(x))) == parse(x)`), the
//! core patch-truth invariant of the deployment framework.

use deep_diff_forge_patch::{parse, render_unified};

fn load(name: &str) -> String {
    let path = format!("{}/../../fixtures/patch/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"))
}

fn assert_model_stable(name: &str) {
    let src = load(name);
    let first = parse(&src).unwrap();
    let rendered = render_unified(&first);
    let second = parse(&rendered).unwrap();
    assert_eq!(first, second, "round-trip changed the model for {name}");
}

#[test]
fn basic_round_trips() {
    assert_model_stable("basic.patch");
}

#[test]
fn new_file_round_trips() {
    assert_model_stable("new_file.patch");
}

#[test]
fn delete_file_round_trips() {
    assert_model_stable("delete_file.patch");
}

#[test]
fn binary_round_trips() {
    assert_model_stable("binary.patch");
}

#[test]
fn rename_round_trips() {
    assert_model_stable("rename.patch");
}

#[test]
fn no_newline_hunks_round_trip_even_though_marker_is_dropped() {
    // The render intentionally does not reconstruct the `\ No newline` marker
    // (it would break apply-ability), so strict equality is not expected; the
    // hunk content must still round-trip.
    let src = load("no_newline.patch");
    let first = parse(&src).unwrap();
    let second = parse(&render_unified(&first)).unwrap();
    assert_eq!(first[0].patch_twin.hunks, second[0].patch_twin.hunks);
    assert_eq!(first[0].path, second[0].path);
}

#[test]
fn double_render_is_idempotent() {
    let first = parse(&load("basic.patch")).unwrap();
    let once = render_unified(&first);
    let twice = render_unified(&parse(&once).unwrap());
    assert_eq!(once, twice);
}
