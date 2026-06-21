//! Integration tests: drive the full ingest → render chain over fixtures.

use deep_diff_forge_pipeline::{IngestStage, Pipeline, PipelineData, RenderMode, RenderStage};

fn load(name: &str) -> String {
    let path = format!("{}/../../fixtures/patch/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"))
}

fn run(name: &str, mode: RenderMode) -> String {
    let pipeline = Pipeline::new()
        .with(Box::new(IngestStage))
        .with(Box::new(RenderStage::new(mode)));
    match pipeline.run(PipelineData::Patch(load(name))).unwrap() {
        PipelineData::Rendered(s) => s,
        other => panic!("expected rendered, got {other:?}"),
    }
}

#[test]
fn basic_through_json() {
    assert!(run("basic.patch", RenderMode::Json).contains("deep-diff-forge.review.v0"));
}

#[test]
fn basic_through_jsonl_is_one_line() {
    assert_eq!(run("basic.patch", RenderMode::Jsonl).lines().count(), 1);
}

#[test]
fn basic_through_inline_has_header() {
    assert!(run("basic.patch", RenderMode::Inline).contains("modified  src/lib.rs"));
}

#[test]
fn basic_through_side_by_side_has_gutter() {
    assert!(run("basic.patch", RenderMode::SideBySide).contains(" | "));
}

#[test]
fn new_file_jsonl_reports_added() {
    assert!(run("new_file.patch", RenderMode::Jsonl).contains("\"status\":\"added\""));
}

#[test]
fn every_fixture_runs_every_mode() {
    for name in [
        "basic.patch",
        "new_file.patch",
        "delete_file.patch",
        "rename.patch",
        "binary.patch",
        "no_newline.patch",
    ] {
        for mode in [
            RenderMode::Json,
            RenderMode::Jsonl,
            RenderMode::Inline,
            RenderMode::SideBySide,
        ] {
            let _ = run(name, mode);
        }
    }
}

#[test]
fn malformed_patch_errors_in_chain() {
    let pipeline = Pipeline::new().with(Box::new(IngestStage));
    let err = pipeline
        .run(PipelineData::Patch("--- a/x\n+++ b/x\n+stray\n".into()))
        .unwrap_err();
    assert!(format!("{err}").contains("patch parse failed"));
}
