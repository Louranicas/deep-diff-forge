use crate::{AnnotationSource, GroundingLevel, grounding_of, source_of};
use deep_diff_forge_core::AgentAnnotation;

/// An in-memory collection of annotations with reviewer-owned resolution state.
///
/// Resolution is explicit: the store never auto-resolves an annotation, and
/// only the caller (a human reviewer, in practice) marks one resolved.
#[derive(Debug, Default, Clone)]
pub struct AnnotationStore {
    annotations: Vec<AgentAnnotation>,
    resolved: Vec<String>,
}

impl AnnotationStore {
    /// Create an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an annotation.
    pub fn add(&mut self, annotation: AgentAnnotation) {
        self.annotations.push(annotation);
    }

    /// Number of annotations.
    #[must_use]
    pub fn len(&self) -> usize {
        self.annotations.len()
    }

    /// Whether the store has no annotations.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.annotations.is_empty()
    }

    /// All annotations in insertion order.
    #[must_use]
    pub fn list(&self) -> &[AgentAnnotation] {
        &self.annotations
    }

    /// Mark an annotation resolved. Returns `true` if it was found and newly
    /// resolved, `false` if unknown or already resolved.
    pub fn resolve(&mut self, id: &str) -> bool {
        let known = self.annotations.iter().any(|a| a.id == id);
        if !known || self.is_resolved(id) {
            return false;
        }
        self.resolved.push(id.to_string());
        true
    }

    /// Whether an annotation id is resolved.
    #[must_use]
    pub fn is_resolved(&self, id: &str) -> bool {
        self.resolved.iter().any(|r| r == id)
    }

    /// Annotations not yet resolved.
    #[must_use]
    pub fn unresolved(&self) -> Vec<&AgentAnnotation> {
        self.annotations
            .iter()
            .filter(|a| !self.is_resolved(&a.id))
            .collect()
    }

    /// Annotations at a given grounding level.
    #[must_use]
    pub fn by_grounding(&self, level: GroundingLevel) -> Vec<&AgentAnnotation> {
        self.annotations
            .iter()
            .filter(|a| grounding_of(a) == level)
            .collect()
    }

    /// Annotations from a given source.
    #[must_use]
    pub fn by_source(&self, source: AnnotationSource) -> Vec<&AgentAnnotation> {
        self.annotations
            .iter()
            .filter(|a| source_of(a) == source)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deep_diff_forge_core::{AnnotationAnchor, AnnotationProvenance};

    fn ann(id: &str, agent: &str, evidence: &[&str], grounded: bool) -> AgentAnnotation {
        AgentAnnotation {
            id: id.into(),
            anchor: AnnotationAnchor::File {
                path: "x.rs".into(),
            },
            body: "b".into(),
            provenance: AnnotationProvenance {
                agent: agent.into(),
                model: None,
                evidence: evidence.iter().map(|e| (*e).to_string()).collect(),
            },
            grounded,
        }
    }

    #[test]
    fn new_store_is_empty() {
        let s = AnnotationStore::new();
        assert!(s.is_empty());
        assert_eq!(s.len(), 0);
    }

    #[test]
    fn add_increases_len() {
        let mut s = AnnotationStore::new();
        s.add(ann("a", "claude", &[], false));
        assert_eq!(s.len(), 1);
        assert!(!s.is_empty());
    }

    #[test]
    fn list_returns_in_insertion_order() {
        let mut s = AnnotationStore::new();
        s.add(ann("a", "claude", &[], false));
        s.add(ann("b", "claude", &[], false));
        let ids: Vec<&str> = s.list().iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b"]);
    }

    #[test]
    fn resolve_marks_known_annotation() {
        let mut s = AnnotationStore::new();
        s.add(ann("a", "claude", &[], false));
        assert!(s.resolve("a"));
        assert!(s.is_resolved("a"));
    }

    #[test]
    fn resolve_unknown_is_false() {
        let mut s = AnnotationStore::new();
        assert!(!s.resolve("nope"));
    }

    #[test]
    fn double_resolve_is_false_second_time() {
        let mut s = AnnotationStore::new();
        s.add(ann("a", "claude", &[], false));
        assert!(s.resolve("a"));
        assert!(!s.resolve("a"));
    }

    #[test]
    fn unresolved_excludes_resolved() {
        let mut s = AnnotationStore::new();
        s.add(ann("a", "claude", &[], false));
        s.add(ann("b", "claude", &[], false));
        s.resolve("a");
        let ids: Vec<&str> = s.unresolved().iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, vec!["b"]);
    }

    #[test]
    fn store_does_not_auto_resolve() {
        let mut s = AnnotationStore::new();
        s.add(ann("a", "claude", &["e"], true));
        assert!(!s.is_resolved("a"));
        assert_eq!(s.unresolved().len(), 1);
    }

    #[test]
    fn by_grounding_filters_grounded() {
        let mut s = AnnotationStore::new();
        s.add(ann("a", "claude", &["e"], true));
        s.add(ann("b", "claude", &[], true));
        let grounded = s.by_grounding(GroundingLevel::Grounded);
        assert_eq!(grounded.len(), 1);
        assert_eq!(grounded[0].id, "a");
    }

    #[test]
    fn by_grounding_filters_ungrounded() {
        let mut s = AnnotationStore::new();
        s.add(ann("a", "claude", &["e"], true));
        s.add(ann("b", "claude", &[], true));
        assert_eq!(s.by_grounding(GroundingLevel::Ungrounded).len(), 1);
    }

    #[test]
    fn by_source_filters_human() {
        let mut s = AnnotationStore::new();
        s.add(ann("a", "human:luke", &[], false));
        s.add(ann("b", "claude", &[], false));
        let human = s.by_source(AnnotationSource::Human);
        assert_eq!(human.len(), 1);
        assert_eq!(human[0].id, "a");
    }

    #[test]
    fn by_source_filters_agent() {
        let mut s = AnnotationStore::new();
        s.add(ann("a", "human", &[], false));
        s.add(ann("b", "claude", &[], false));
        assert_eq!(s.by_source(AnnotationSource::Agent).len(), 1);
    }

    #[test]
    fn store_is_cloneable() {
        let mut s = AnnotationStore::new();
        s.add(ann("a", "claude", &[], false));
        let c = s.clone();
        assert_eq!(c.len(), 1);
    }

    #[test]
    fn empty_store_filters_are_empty() {
        let s = AnnotationStore::new();
        assert!(s.by_grounding(GroundingLevel::Grounded).is_empty());
        assert!(s.by_source(AnnotationSource::Agent).is_empty());
        assert!(s.unresolved().is_empty());
    }

    #[test]
    fn resolve_one_of_many() {
        let mut s = AnnotationStore::new();
        for id in ["a", "b", "c"] {
            s.add(ann(id, "claude", &[], false));
        }
        s.resolve("b");
        assert!(s.is_resolved("b"));
        assert!(!s.is_resolved("a"));
        assert_eq!(s.unresolved().len(), 2);
    }
}
