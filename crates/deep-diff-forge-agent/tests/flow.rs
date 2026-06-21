//! Integration tests: annotate a real review end to end.

use deep_diff_forge_agent::{
    AnnotationSource, AnnotationStore, GroundingLevel, SYSTEM_AGENT_ID, anchor_path, grounding_of,
    sanitize_body, source_of, validate_anchor,
};
use deep_diff_forge_core::{
    AgentAnnotation, AnnotationAnchor, AnnotationProvenance, HunkId, ReviewFile,
};
use deep_diff_forge_patch::parse;

fn review() -> Vec<ReviewFile> {
    parse("--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@\n-a\n+b\n").unwrap()
}

fn make(
    id: &str,
    anchor: AnnotationAnchor,
    agent: &str,
    evidence: &[&str],
    grounded: bool,
) -> AgentAnnotation {
    AgentAnnotation {
        id: id.into(),
        anchor,
        body: "review note".into(),
        provenance: AnnotationProvenance {
            agent: agent.into(),
            model: Some("opus".into()),
            evidence: evidence.iter().map(|e| (*e).to_string()).collect(),
        },
        grounded,
    }
}

#[test]
fn grounded_agent_annotation_against_real_hunk() {
    let files = review();
    let a = make(
        "g1",
        AnnotationAnchor::Hunk {
            path: "src/lib.rs".into(),
            hunk_id: HunkId(0),
        },
        "claude-code",
        &["src/lib.rs:1 changed a->b"],
        true,
    );
    assert!(validate_anchor(&a.anchor, &files));
    assert_eq!(grounding_of(&a), GroundingLevel::Grounded);
    assert_eq!(source_of(&a), AnnotationSource::Agent);
}

#[test]
fn ungrounded_claim_is_flagged_even_anchored() {
    let files = review();
    let a = make(
        "u1",
        AnnotationAnchor::File {
            path: "src/lib.rs".into(),
        },
        "claude-code",
        &[],
        true,
    );
    assert!(validate_anchor(&a.anchor, &files));
    assert_eq!(grounding_of(&a), GroundingLevel::Ungrounded);
}

#[test]
fn anchor_to_missing_file_is_rejected() {
    let files = review();
    let a = make(
        "m1",
        AnnotationAnchor::File {
            path: "ghost.rs".into(),
        },
        "claude",
        &[],
        false,
    );
    assert!(!validate_anchor(&a.anchor, &files));
}

#[test]
fn store_separates_grounded_from_ungrounded() {
    let mut store = AnnotationStore::new();
    store.add(make(
        "g",
        AnnotationAnchor::File {
            path: "src/lib.rs".into(),
        },
        "claude",
        &["e"],
        true,
    ));
    store.add(make(
        "u",
        AnnotationAnchor::File {
            path: "src/lib.rs".into(),
        },
        "claude",
        &[],
        true,
    ));
    assert_eq!(store.by_grounding(GroundingLevel::Grounded).len(), 1);
    assert_eq!(store.by_grounding(GroundingLevel::Ungrounded).len(), 1);
}

#[test]
fn reviewer_resolves_an_annotation() {
    let mut store = AnnotationStore::new();
    store.add(make(
        "a",
        AnnotationAnchor::File {
            path: "src/lib.rs".into(),
        },
        "claude",
        &["e"],
        true,
    ));
    assert_eq!(store.unresolved().len(), 1);
    assert!(store.resolve("a"));
    assert!(store.unresolved().is_empty());
}

#[test]
fn human_and_agent_annotations_coexist() {
    let mut store = AnnotationStore::new();
    // A self-asserted "human:luke" label is fail-closed to untrusted Agent; only
    // the exact reserved engine id is System.
    store.add(make(
        "h",
        AnnotationAnchor::File {
            path: "src/lib.rs".into(),
        },
        "human:luke",
        &[],
        false,
    ));
    store.add(make(
        "a",
        AnnotationAnchor::File {
            path: "src/lib.rs".into(),
        },
        "claude",
        &["e"],
        true,
    ));
    store.add(make(
        "s",
        AnnotationAnchor::File {
            path: "src/lib.rs".into(),
        },
        SYSTEM_AGENT_ID,
        &[],
        false,
    ));
    assert_eq!(store.by_source(AnnotationSource::Human).len(), 0);
    assert_eq!(store.by_source(AnnotationSource::Agent).len(), 2);
    assert_eq!(store.by_source(AnnotationSource::System).len(), 1);
}

#[test]
fn untrusted_body_is_sanitized_before_storage() {
    let dirty = "  inject\u{07}ed\u{00} note  ";
    let clean = sanitize_body(dirty);
    assert_eq!(clean, "injected note");
}

#[test]
fn anchor_path_round_trips_through_validation() {
    let files = review();
    let anchor = AnnotationAnchor::Hunk {
        path: "src/lib.rs".into(),
        hunk_id: HunkId(0),
    };
    assert_eq!(anchor_path(&anchor), "src/lib.rs");
    assert!(validate_anchor(&anchor, &files));
}

#[test]
fn stale_hunk_anchor_after_reparse_is_rejected() {
    let files = review();
    let anchor = AnnotationAnchor::Hunk {
        path: "src/lib.rs".into(),
        hunk_id: HunkId(42),
    };
    assert!(!validate_anchor(&anchor, &files));
}

#[test]
fn full_triage_flow() {
    let files = review();
    let mut store = AnnotationStore::new();
    for (id, ev, grounded) in [
        ("a", vec!["e"], true),
        ("b", vec![], true),
        ("c", vec!["e"], false),
    ] {
        let a = make(
            id,
            AnnotationAnchor::File {
                path: "src/lib.rs".into(),
            },
            "claude",
            &ev,
            grounded,
        );
        assert!(validate_anchor(&a.anchor, &files));
        store.add(a);
    }
    assert_eq!(store.by_grounding(GroundingLevel::Grounded).len(), 1);
    assert_eq!(
        store.by_grounding(GroundingLevel::PartiallyGrounded).len(),
        1
    );
    assert_eq!(store.by_grounding(GroundingLevel::Ungrounded).len(), 1);
    store.resolve("a");
    assert_eq!(store.unresolved().len(), 2);
}
