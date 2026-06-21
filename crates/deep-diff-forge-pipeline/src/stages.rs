use crate::{ChainStage, PipelineData, PipelineError, jsonl_events};
use deep_diff_forge_projection::{Layout, ProjectionOptions, render};

/// Ingest stage: raw patch text → parsed review model.
#[derive(Debug, Clone, Copy, Default)]
pub struct IngestStage;

impl ChainStage for IngestStage {
    fn name(&self) -> &'static str {
        "ingest"
    }

    fn run(&self, input: PipelineData) -> Result<PipelineData, PipelineError> {
        match input {
            PipelineData::Patch(text) => deep_diff_forge_patch::parse(&text)
                .map(PipelineData::Review)
                .map_err(|e| PipelineError::Parse(e.to_string())),
            _ => Err(PipelineError::UnexpectedInput {
                stage: "ingest",
                expected: "raw patch text",
            }),
        }
    }
}

/// Output mode for [`RenderStage`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderMode {
    /// One complete `deep-diff-forge.review.v0` JSON document.
    Json,
    /// One JSON event per file, newline-delimited.
    Jsonl,
    /// Inline projection.
    Inline,
    /// Side-by-side projection at the default width.
    SideBySide,
}

/// Render stage: review model → rendered text in the selected mode.
#[derive(Debug, Clone, Copy)]
pub struct RenderStage {
    mode: RenderMode,
}

impl RenderStage {
    /// Create a render stage with the given mode.
    #[must_use]
    pub fn new(mode: RenderMode) -> Self {
        Self { mode }
    }

    /// Convenience constructor for the JSONL streaming mode.
    #[must_use]
    pub fn jsonl() -> Self {
        Self {
            mode: RenderMode::Jsonl,
        }
    }
}

impl ChainStage for RenderStage {
    fn name(&self) -> &'static str {
        "render"
    }

    fn run(&self, input: PipelineData) -> Result<PipelineData, PipelineError> {
        let PipelineData::Review(files) = input else {
            return Err(PipelineError::UnexpectedInput {
                stage: "render",
                expected: "review model",
            });
        };
        let text = match self.mode {
            RenderMode::Json => deep_diff_forge_patch::to_json(&files),
            RenderMode::Jsonl => jsonl_events(&files),
            RenderMode::Inline => render(
                &files,
                ProjectionOptions {
                    layout: Layout::Inline,
                    ..ProjectionOptions::default()
                },
            ),
            RenderMode::SideBySide => render(
                &files,
                ProjectionOptions {
                    layout: Layout::SideBySide,
                    ..ProjectionOptions::default()
                },
            ),
        };
        Ok(PipelineData::Rendered(text))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PATCH: &str = "--- a/x\n+++ b/x\n@@ -1,2 +1,2 @@\n a\n-b\n+B\n";

    fn ingest(text: &str) -> PipelineData {
        IngestStage
            .run(PipelineData::Patch(text.to_string()))
            .unwrap()
    }

    #[test]
    fn ingest_produces_review_model() {
        let out = ingest(PATCH);
        assert!(matches!(out, PipelineData::Review(_)));
    }

    #[test]
    fn ingest_parses_expected_file_count() {
        let PipelineData::Review(files) = ingest(PATCH) else {
            panic!("expected review");
        };
        assert_eq!(files.len(), 1);
    }

    #[test]
    fn ingest_empty_patch_is_empty_review() {
        let PipelineData::Review(files) = ingest("") else {
            panic!("expected review");
        };
        assert!(files.is_empty());
    }

    #[test]
    fn ingest_rejects_rendered_input() {
        let err = IngestStage
            .run(PipelineData::Rendered("x".into()))
            .unwrap_err();
        assert!(matches!(
            err,
            PipelineError::UnexpectedInput {
                stage: "ingest",
                ..
            }
        ));
    }

    #[test]
    fn ingest_malformed_is_parse_error() {
        let err = IngestStage
            .run(PipelineData::Patch("--- a/x\n+++ b/x\n+stray\n".into()))
            .unwrap_err();
        assert!(matches!(err, PipelineError::Parse(_)));
    }

    #[test]
    fn render_json_mode_emits_schema() {
        let out = RenderStage::new(RenderMode::Json)
            .run(ingest(PATCH))
            .unwrap();
        let PipelineData::Rendered(s) = out else {
            panic!("expected rendered");
        };
        assert!(s.contains("deep-diff-forge.review.v0"));
    }

    #[test]
    fn render_jsonl_mode_one_line_per_file() {
        let out = RenderStage::jsonl().run(ingest(PATCH)).unwrap();
        let PipelineData::Rendered(s) = out else {
            panic!("expected rendered");
        };
        assert_eq!(s.lines().count(), 1);
    }

    #[test]
    fn render_inline_mode_has_header() {
        let out = RenderStage::new(RenderMode::Inline)
            .run(ingest(PATCH))
            .unwrap();
        let PipelineData::Rendered(s) = out else {
            panic!("expected rendered");
        };
        assert!(s.contains("modified  x"));
    }

    #[test]
    fn render_side_by_side_mode_has_gutter() {
        let out = RenderStage::new(RenderMode::SideBySide)
            .run(ingest(PATCH))
            .unwrap();
        let PipelineData::Rendered(s) = out else {
            panic!("expected rendered");
        };
        assert!(s.contains(" | "));
    }

    #[test]
    fn render_rejects_patch_input() {
        let err = RenderStage::new(RenderMode::Json)
            .run(PipelineData::Patch("x".into()))
            .unwrap_err();
        assert!(matches!(
            err,
            PipelineError::UnexpectedInput {
                stage: "render",
                ..
            }
        ));
    }

    #[test]
    fn render_rejects_rendered_input() {
        let err = RenderStage::jsonl()
            .run(PipelineData::Rendered("x".into()))
            .unwrap_err();
        assert!(matches!(err, PipelineError::UnexpectedInput { .. }));
    }

    #[test]
    fn render_mode_is_copy_constructible() {
        let a = RenderStage::new(RenderMode::Json);
        let b = a;
        assert_eq!(a.name(), b.name());
    }

    #[test]
    fn jsonl_convenience_matches_explicit_mode() {
        let a = RenderStage::jsonl().run(ingest(PATCH)).unwrap();
        let b = RenderStage::new(RenderMode::Jsonl)
            .run(ingest(PATCH))
            .unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn ingest_stage_name() {
        assert_eq!(IngestStage.name(), "ingest");
    }

    #[test]
    fn render_stage_name() {
        assert_eq!(RenderStage::jsonl().name(), "render");
    }
}
