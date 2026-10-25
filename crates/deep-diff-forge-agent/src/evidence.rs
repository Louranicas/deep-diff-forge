// DDF integration: R14, R15, R16, R17, R52. Separate asserted citations from captured artifact integrity, trusted evaluator support and revision freshness. Do not resolve duplicate IDs together.
// Design/status: docs/RECOMMENDATION_INTEGRATION.md#evidence
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fdeep-diff-forge-agent--evidence
//! Revision-bound evidence checks (R14–R18, R52).
//! Implementation and vault backlinks: `docs/RECOMMENDATION_INTEGRATION.md#evidence`.
//! Artifact integrity proves which bytes were inspected, not that a claim is true.
use deep_diff_forge_core::{AgentAnnotation, AnnotationAnchor, ReviewFile, sha256_hex};
use std::collections::BTreeMap;

/// An immutable review identity, derived from a captured input artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewRevision(String);
impl ReviewRevision {
    /// Seal raw patch input before parsing. Patch-only scope: source context is
    /// not bound. Full-source consumers must use `from_review_id` instead.
    #[must_use]
    pub fn from_patch(bytes: &[u8]) -> Self {
        Self(sha256_hex(bytes))
    }
    /// Bind to the shared service's review ID, which also seals source snapshots.
    /// IDs are supplied by the trusted capture controller, never by agent claims.
    #[must_use]
    pub fn from_review_id(id: &str) -> Option<Self> {
        if id.len() == 64
            && id
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            Some(Self(id.to_owned()))
        } else {
            None
        }
    }
    /// Content identity suitable for persistence alongside an anchor.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
/// An anchor whose document-local counters cannot be reused on another revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundAnchor {
    pub revision: ReviewRevision,
    pub anchor: AnnotationAnchor,
}
impl BoundAnchor {
    /// Validate both revision identity and membership; no fuzzy reattachment.
    #[must_use]
    pub fn validates(&self, current: &ReviewRevision, files: &[ReviewFile]) -> bool {
        &self.revision == current && crate::validate_anchor(&self.anchor, files)
    }
}
/// Evidence lifecycle. Linked bytes alone are not semantic support.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvidenceState {
    Missing,
    Linked,
    Supported {
        evaluator: String,
        rationale: String,
    },
    Contradicted {
        evaluator: String,
        rationale: String,
    },
    Stale,
}
#[derive(Debug, Clone)]
struct Artifact {
    digest: String,
    revision: ReviewRevision,
    state: EvidenceState,
}
/// Local registry. Populate only from captured bytes and trusted evaluator actions.
/// It does not read paths or execute commands supplied by annotations.
#[derive(Debug, Default)]
pub struct EvidenceRegistry {
    artifacts: BTreeMap<String, Artifact>,
}
impl EvidenceRegistry {
    /// Capture bytes once; duplicate IDs are rejected even if the bytes match.
    pub fn capture(&mut self, id: String, revision: ReviewRevision, bytes: &[u8]) -> bool {
        if id.is_empty() || self.artifacts.contains_key(&id) {
            return false;
        }
        self.artifacts.insert(
            id,
            Artifact {
                digest: sha256_hex(bytes),
                revision,
                state: EvidenceState::Linked,
            },
        );
        true
    }
    /// Verify a content-addressed artifact independently of any support judgment.
    #[must_use]
    pub fn matches(&self, id: &str, bytes: &[u8]) -> bool {
        self.artifacts
            .get(id)
            .is_some_and(|a| a.digest == sha256_hex(bytes))
    }
    /// Record an explicit evaluator's support/contradiction of these evidence bytes.
    /// This must be called by a trusted controller, never deserialized from agent flags.
    pub fn assess(
        &mut self,
        id: &str,
        current: &ReviewRevision,
        evaluator: &str,
        rationale: &str,
        supports: bool,
    ) -> bool {
        let Some(a) = self.artifacts.get_mut(id) else {
            return false;
        };
        if &a.revision != current || evaluator.trim().is_empty() || rationale.trim().is_empty() {
            return false;
        }
        a.state = if supports {
            EvidenceState::Supported {
                evaluator: evaluator.into(),
                rationale: rationale.into(),
            }
        } else {
            EvidenceState::Contradicted {
                evaluator: evaluator.into(),
                rationale: rationale.into(),
            }
        };
        true
    }
    /// Freshness is checked on every access, including after a source revision changes.
    #[must_use]
    pub fn state(&self, id: &str, current: &ReviewRevision) -> EvidenceState {
        self.artifacts.get(id).map_or(EvidenceState::Missing, |a| {
            if &a.revision == current {
                a.state.clone()
            } else {
                EvidenceState::Stale
            }
        })
    }
    /// Expose one status per cited artifact, without accepting an annotation's flag.
    #[must_use]
    pub fn inspect(
        &self,
        annotation: &AgentAnnotation,
        current: &ReviewRevision,
    ) -> Vec<EvidenceState> {
        annotation
            .provenance
            .evidence
            .iter()
            .map(|id| self.state(id, current))
            .collect()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn artifact_linking_support_and_freshness_are_separate() {
        let old = ReviewRevision::from_patch(b"old");
        let new = ReviewRevision::from_patch(b"new");
        let mut registry = EvidenceRegistry::default();
        assert!(registry.capture("e".into(), old.clone(), b"test output"));
        assert!(!registry.capture("e".into(), old.clone(), b"forged"));
        assert!(registry.matches("e", b"test output"));
        assert!(!registry.matches("e", b"forged"));
        assert_eq!(registry.state("e", &old), EvidenceState::Linked);
        assert!(!registry.assess("e", &new, "reviewer", "checked", true));
        assert!(registry.assess(
            "e",
            &old,
            "reviewer",
            "test passes for captured input",
            true
        ));
        assert!(matches!(
            registry.state("e", &old),
            EvidenceState::Supported { .. }
        ));
        assert_eq!(registry.state("e", &new), EvidenceState::Stale);
        assert_eq!(registry.state("fabricated", &old), EvidenceState::Missing);
    }
    #[test]
    fn reused_hunk_counter_does_not_validate_a_stale_anchor() {
        let patch = "--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n";
        let files = deep_diff_forge_patch::parse(patch).unwrap();
        let revision = ReviewRevision::from_patch(patch.as_bytes());
        let anchor = BoundAnchor {
            revision: revision.clone(),
            anchor: AnnotationAnchor::Hunk {
                path: "x".into(),
                hunk_id: deep_diff_forge_core::HunkId(0),
            },
        };
        assert!(anchor.validates(&revision, &files));
        assert!(!anchor.validates(&ReviewRevision::from_patch(b"different"), &files));
    }
}
