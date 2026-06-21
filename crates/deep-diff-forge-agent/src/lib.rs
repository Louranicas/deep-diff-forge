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

/// The engine's exact, namespaced reserved id for its own (`System`)
/// annotations. The slash makes it implausible as an organic agent name, and
/// ingestion of untrusted annotations must reject this id from external authors.
pub const SYSTEM_AGENT_ID: &str = "deep-diff-forge/system";

/// Classify the source of an annotation. **Fails closed.**
///
/// The `provenance.agent` field is self-asserted and fully attacker-controlled,
/// so trust is never inferred from it: every externally-authored annotation is
/// classified [`AnnotationSource::Agent`] (untrusted). A previous version
/// substring-matched the label (`contains("human")` / `contains("system")`),
/// which let an adversarial annotation labelled e.g. `"human-helper"` escalate
/// itself to `Human`. That inference is removed.
///
/// `Human` is never derived from the wire (a human reviewer's trust arrives
/// through the reviewer's own actions, not a label), and only the exact reserved
/// [`SYSTEM_AGENT_ID`] maps to `System`. The real trust authority is
/// [`grounding_of`] — evidence, which an attacker cannot fabricate by relabelling.
#[must_use]
pub fn source_of(annotation: &AgentAnnotation) -> AnnotationSource {
    if annotation.provenance.agent == SYSTEM_AGENT_ID {
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
    fn source_fails_closed_to_agent() {
        // Everything self-reported is untrusted Agent — including labels that
        // previously escalated to Human/System.
        for label in [
            "claude-code",
            "gpt",
            "human:luke",
            "reviewer",
            "system",
            "deep-diff-forge",
            "HUMAN",
        ] {
            assert_eq!(
                source_of(&annotation(label, &[], false)),
                AnnotationSource::Agent,
                "label {label:?} must classify as untrusted Agent"
            );
        }
    }

    #[test]
    fn only_exact_reserved_id_is_system() {
        assert_eq!(
            source_of(&annotation(SYSTEM_AGENT_ID, &[], false)),
            AnnotationSource::System
        );
        // Near-misses do not escalate (no substring / prefix match).
        assert_eq!(
            source_of(&annotation("deep-diff-forge/system-but-evil", &[], false)),
            AnnotationSource::Agent
        );
        assert_eq!(
            source_of(&annotation("x deep-diff-forge/system", &[], false)),
            AnnotationSource::Agent
        );
    }

    #[test]
    fn adversarial_label_cannot_escalate_to_human() {
        // The headline fix: an attacker-chosen agent string can never become
        // Human, regardless of what trust-implying word it embeds.
        for evil in ["human-helper-bot", "trusted-reviewer", "SYSTEM-override"] {
            assert_ne!(
                source_of(&annotation(evil, &[], false)),
                AnnotationSource::Human
            );
        }
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
