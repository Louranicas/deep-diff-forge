//! Composable Unix-filter pipeline for Deep-Diff-Forge (L3).
//!
//! A pipeline is an ordered list of [`ChainStage`]s, each transforming a
//! [`PipelineData`] envelope into the next. Stages are explicit about the input
//! they accept and fail with a typed [`PipelineError`] rather than panicking,
//! so the chain is safe to drive from strict Bash (`set -euo pipefail`).
//!
//! The L3 stage set is [`IngestStage`] (raw patch text → review model) and
//! [`RenderStage`] (review model → rendered text in one of several modes,
//! including a JSONL event stream). Patch truth is never mutated by a stage.

mod jsonl;
mod stages;

pub use jsonl::jsonl_events;
pub use stages::{IngestStage, RenderMode, RenderStage};

use deep_diff_forge_core::ReviewFile;

/// The value flowing between pipeline stages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PipelineData {
    /// Raw patch text (pipeline source).
    Patch(String),
    /// Parsed review model.
    Review(Vec<ReviewFile>),
    /// Rendered output text (pipeline sink).
    Rendered(String),
}

/// Typed, non-panicking pipeline errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PipelineError {
    /// A stage received a data variant it does not accept.
    UnexpectedInput {
        /// Stage that rejected the input.
        stage: &'static str,
        /// Human description of what the stage expected.
        expected: &'static str,
    },
    /// Patch parsing failed inside an ingest stage.
    Parse(String),
}

impl std::fmt::Display for PipelineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnexpectedInput { stage, expected } => {
                write!(f, "stage {stage} expected {expected}")
            }
            Self::Parse(msg) => write!(f, "patch parse failed: {msg}"),
        }
    }
}

impl std::error::Error for PipelineError {}

/// A single composable pipeline stage.
pub trait ChainStage {
    /// Stable stage name (also used in diagnostics).
    fn name(&self) -> &'static str;
    /// Transform the input envelope into the next one.
    ///
    /// # Errors
    ///
    /// Returns [`PipelineError`] when the stage receives an input variant it
    /// does not accept, or when an underlying operation (e.g. patch parsing)
    /// fails.
    fn run(&self, input: PipelineData) -> Result<PipelineData, PipelineError>;
}

/// An ordered chain of stages.
#[derive(Default)]
pub struct Pipeline {
    stages: Vec<Box<dyn ChainStage>>,
}

impl Pipeline {
    /// Create an empty pipeline.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a stage (builder style).
    #[must_use]
    pub fn with(mut self, stage: Box<dyn ChainStage>) -> Self {
        self.stages.push(stage);
        self
    }

    /// Number of stages in the chain.
    #[must_use]
    pub fn len(&self) -> usize {
        self.stages.len()
    }

    /// Whether the chain has no stages.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.stages.is_empty()
    }

    /// Run the chain, folding `initial` through every stage in order.
    ///
    /// # Errors
    ///
    /// Returns the first stage error encountered; remaining stages do not run.
    pub fn run(&self, initial: PipelineData) -> Result<PipelineData, PipelineError> {
        let mut data = initial;
        for stage in &self.stages {
            data = stage.run(data)?;
        }
        Ok(data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PATCH: &str = "--- a/x\n+++ b/x\n@@ -1,1 +1,1 @@\n-a\n+b\n";

    #[test]
    fn empty_pipeline_returns_input() {
        let p = Pipeline::new();
        assert!(p.is_empty());
        let out = p.run(PipelineData::Patch("x".into())).unwrap();
        assert_eq!(out, PipelineData::Patch("x".into()));
    }

    #[test]
    fn len_counts_stages() {
        let p = Pipeline::new()
            .with(Box::new(IngestStage))
            .with(Box::new(RenderStage::new(RenderMode::Json)));
        assert_eq!(p.len(), 2);
        assert!(!p.is_empty());
    }

    #[test]
    fn ingest_then_render_json_produces_document() {
        let p = Pipeline::new()
            .with(Box::new(IngestStage))
            .with(Box::new(RenderStage::new(RenderMode::Json)));
        let out = p.run(PipelineData::Patch(PATCH.into())).unwrap();
        match out {
            PipelineData::Rendered(s) => {
                assert!(s.contains("deep-diff-forge.review.v0"));
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn ingest_then_render_jsonl_streams_one_event() {
        let p = Pipeline::new()
            .with(Box::new(IngestStage))
            .with(Box::new(RenderStage::new(RenderMode::Jsonl)));
        let out = p.run(PipelineData::Patch(PATCH.into())).unwrap();
        match out {
            PipelineData::Rendered(s) => {
                assert_eq!(s.lines().count(), 1);
                assert!(s.contains("\"event\":\"diff.file\""));
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn parse_failure_surfaces_as_pipeline_error() {
        let p = Pipeline::new().with(Box::new(IngestStage));
        let err = p
            .run(PipelineData::Patch("--- a/x\n+++ b/x\n+stray\n".into()))
            .unwrap_err();
        assert!(matches!(err, PipelineError::Parse(_)));
    }

    #[test]
    fn render_rejects_raw_patch_input() {
        let stage = RenderStage::new(RenderMode::Json);
        let err = stage.run(PipelineData::Patch("x".into())).unwrap_err();
        assert!(matches!(err, PipelineError::UnexpectedInput { .. }));
    }

    #[test]
    fn ingest_rejects_already_review_input() {
        let err = IngestStage.run(PipelineData::Review(vec![])).unwrap_err();
        assert!(matches!(err, PipelineError::UnexpectedInput { .. }));
    }

    #[test]
    fn error_display_is_descriptive() {
        let e = PipelineError::UnexpectedInput {
            stage: "render",
            expected: "review model",
        };
        assert!(e.to_string().contains("render"));
        assert!(e.to_string().contains("review model"));
    }

    #[test]
    fn stage_names_are_stable() {
        assert_eq!(IngestStage.name(), "ingest");
        assert_eq!(RenderStage::new(RenderMode::Json).name(), "render");
    }

    #[test]
    fn first_stage_error_stops_the_chain() {
        // Render before ingest: first stage gets Patch and must reject it.
        let p = Pipeline::new()
            .with(Box::new(RenderStage::new(RenderMode::Json)))
            .with(Box::new(IngestStage));
        let err = p.run(PipelineData::Patch(PATCH.into())).unwrap_err();
        assert!(matches!(err, PipelineError::UnexpectedInput { .. }));
    }

    #[test]
    fn default_pipeline_is_empty() {
        assert!(Pipeline::default().is_empty());
    }

    #[test]
    fn ingest_then_inline_renders_text() {
        let p = Pipeline::new()
            .with(Box::new(IngestStage))
            .with(Box::new(RenderStage::new(RenderMode::Inline)));
        let out = p.run(PipelineData::Patch(PATCH.into())).unwrap();
        assert!(matches!(out, PipelineData::Rendered(s) if s.contains("modified  x")));
    }

    #[test]
    fn pipeline_data_variants_compare_by_value() {
        assert_eq!(
            PipelineData::Patch("a".into()),
            PipelineData::Patch("a".into())
        );
        assert_ne!(
            PipelineData::Patch("a".into()),
            PipelineData::Rendered("a".into())
        );
    }

    #[test]
    fn parse_error_message_is_preserved() {
        let err = IngestStage
            .run(PipelineData::Patch("--- a/x\n+++ b/x\n+stray\n".into()))
            .unwrap_err();
        assert!(err.to_string().contains("outside any hunk"));
    }
}
