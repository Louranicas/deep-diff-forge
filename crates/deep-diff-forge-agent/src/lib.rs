//! Agent annotation protocol for Deep-Diff-Forge (L5).
//!
//! Annotations are first-class but **not trusted by default** (Vision §4): every
//! claim is classified by grounding (does it carry evidence?) and by source
//! (human / agent / system), untrusted bodies are sanitized, and anchors are
//! validated against the real review. Approval state belongs to the reviewer,
//! not the agent — this crate never auto-resolves an annotation.

mod anchor;
mod store;

pub use anchor::{anchor_path, validate_anchor};
pub use store::AnnotationStore;

use deep_diff_forge_core::AgentAnnotation;

/// Maximum sanitized annotation body length, in characters.
pub const MAX_BODY_LEN: usize = 4096;

/// How well an annotation's claim is backed by evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroundingLevel {
    /// Has evidence and is asserted grounded.
    Grounded,
    /// Has evidence but is not asserted grounded (claim awaits confirmation).
    PartiallyGrounded,
    /// No evidence — cannot be trusted as grounded regardless of any flag.
    Ungrounded,
}

impl GroundingLevel {
    /// Stable snake-case label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Grounded => "grounded",
            Self::PartiallyGrounded => "partially_grounded",
            Self::Ungrounded => "ungrounded",
        }
    }
}

/// Who authored an annotation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnnotationSource {
    /// A human reviewer.
    Human,
    /// An AI agent.
    Agent,
    /// The engine itself.
    System,
}

impl AnnotationSource {
    /// Stable snake-case label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Human => "human",
            Self::Agent => "agent",
            Self::System => "system",
        }
    }
}

/// Classify an annotation's grounding from its evidence and grounded flag.
///
/// Evidence is necessary: an annotation with no evidence is always
/// [`GroundingLevel::Ungrounded`], even if its `grounded` flag is set.
#[must_use]
pub fn grounding_of(annotation: &AgentAnnotation) -> GroundingLevel {
    if annotation.provenance.evidence.is_empty() {
        GroundingLevel::Ungrounded
    } else if annotation.grounded {
        GroundingLevel::Grounded
    } else {
        GroundingLevel::PartiallyGrounded
    }
}

/// Infer the source of an annotation from its provenance agent string.
#[must_use]
pub fn source_of(annotation: &AgentAnnotation) -> AnnotationSource {
    let agent = annotation.provenance.agent.to_ascii_lowercase();
    if agent.contains("human") || agent.contains("reviewer") {
        AnnotationSource::Human
    } else if agent.contains("system") || agent.contains("deep-diff-forge") {
        AnnotationSource::System
    } else {
        AnnotationSource::Agent
    }
}

/// Sanitize an untrusted annotation body: drop control characters (keeping
/// newlines and tabs), trim surrounding whitespace, and cap the length.
#[must_use]
pub fn sanitize_body(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .filter(|c| *c == '\n' || *c == '\t' || !c.is_control())
        .take(MAX_BODY_LEN)
        .collect();
    cleaned.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use deep_diff_forge_core::{AnnotationAnchor, AnnotationProvenance};

    fn annotation(agent: &str, evidence: &[&str], grounded: bool) -> AgentAnnotation {
        AgentAnnotation {
            id: "a1".into(),
            anchor: AnnotationAnchor::File {
                path: "x.rs".into(),
            },
            body: "note".into(),
            provenance: AnnotationProvenance {
                agent: agent.into(),
                model: None,
                evidence: evidence.iter().map(|e| (*e).to_string()).collect(),
            },
            grounded,
        }
    }

    #[test]
    fn no_evidence_is_ungrounded_even_if_flagged() {
        let a = annotation("claude", &[], true);
        assert_eq!(grounding_of(&a), GroundingLevel::Ungrounded);
    }

    #[test]
    fn evidence_and_flag_is_grounded() {
        let a = annotation("claude", &["src/x.rs:10"], true);
        assert_eq!(grounding_of(&a), GroundingLevel::Grounded);
    }

    #[test]
    fn evidence_without_flag_is_partial() {
        let a = annotation("claude", &["cmd output"], false);
        assert_eq!(grounding_of(&a), GroundingLevel::PartiallyGrounded);
    }

    #[test]
    fn human_source_detected() {
        assert_eq!(
            source_of(&annotation("human:luke", &[], false)),
            AnnotationSource::Human
        );
        assert_eq!(
            source_of(&annotation("reviewer", &[], false)),
            AnnotationSource::Human
        );
    }

    #[test]
    fn system_source_detected() {
        assert_eq!(
            source_of(&annotation("system", &[], false)),
            AnnotationSource::System
        );
        assert_eq!(
            source_of(&annotation("deep-diff-forge", &[], false)),
            AnnotationSource::System
        );
    }

    #[test]
    fn agent_source_is_default() {
        assert_eq!(
            source_of(&annotation("claude-code", &[], false)),
            AnnotationSource::Agent
        );
        assert_eq!(
            source_of(&annotation("gpt", &[], false)),
            AnnotationSource::Agent
        );
    }

    #[test]
    fn source_detection_is_case_insensitive() {
        assert_eq!(
            source_of(&annotation("HUMAN", &[], false)),
            AnnotationSource::Human
        );
    }

    #[test]
    fn sanitize_drops_control_chars() {
        assert_eq!(sanitize_body("a\u{07}b\u{00}c"), "abc");
    }

    #[test]
    fn sanitize_keeps_newlines_and_tabs() {
        assert_eq!(sanitize_body("a\nb\tc"), "a\nb\tc");
    }

    #[test]
    fn sanitize_trims_whitespace() {
        assert_eq!(sanitize_body("   hello   "), "hello");
    }

    #[test]
    fn sanitize_caps_length() {
        let long = "x".repeat(MAX_BODY_LEN + 100);
        assert_eq!(sanitize_body(&long).len(), MAX_BODY_LEN);
    }

    #[test]
    fn sanitize_preserves_unicode() {
        assert_eq!(sanitize_body("café→"), "café→");
    }

    #[test]
    fn sanitize_empty_stays_empty() {
        assert_eq!(sanitize_body(""), "");
        assert_eq!(sanitize_body("   "), "");
    }

    #[test]
    fn grounding_labels_are_stable() {
        assert_eq!(GroundingLevel::Grounded.label(), "grounded");
        assert_eq!(
            GroundingLevel::PartiallyGrounded.label(),
            "partially_grounded"
        );
        assert_eq!(GroundingLevel::Ungrounded.label(), "ungrounded");
    }

    #[test]
    fn source_labels_are_stable() {
        assert_eq!(AnnotationSource::Human.label(), "human");
        assert_eq!(AnnotationSource::Agent.label(), "agent");
        assert_eq!(AnnotationSource::System.label(), "system");
    }

    #[test]
    fn max_body_len_is_exposed() {
        assert_eq!(MAX_BODY_LEN, 4096);
    }
}
