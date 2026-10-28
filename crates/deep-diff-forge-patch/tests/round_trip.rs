// DDF integration: R02, R03, R04, R05, R06, R07, R44, R46. Preserve source bytes and exact original input independently of normalized display; use Git as the application oracle.
// Design/status: docs/RECOMMENDATION_INTEGRATION.md#patch-truth
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fdeep-diff-forge-patch--tests--round_trip
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
fn binary_literal_delta_round_trips() {
    assert_model_stable("binary_literal_delta.patch");
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
fn no_newline_markers_round_trip() {
    assert_model_stable("no_newline.patch");
    let rendered = render_unified(&parse(&load("no_newline.patch")).unwrap());
    assert!(rendered.contains("\\ No newline at end of file"));
}

#[test]
fn double_render_is_idempotent() {
    let first = parse(&load("basic.patch")).unwrap();
    let once = render_unified(&first);
    let twice = render_unified(&parse(&once).unwrap());
    assert_eq!(once, twice);
}

#[test]
fn discovered_fuzz_inputs_fail_explicitly_or_render_stably() {
    let directory = format!(
        "{}/../../fixtures/patch/regressions",
        env!("CARGO_MANIFEST_DIR")
    );
    for entry in std::fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        let input = std::fs::read_to_string(&path).unwrap();
        if let Ok(files) = parse(&input) {
            let rendered = render_unified(&files);
            let reparsed = parse(&rendered).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            assert_eq!(rendered, render_unified(&reparsed), "{}", path.display());
        }
    }
}
