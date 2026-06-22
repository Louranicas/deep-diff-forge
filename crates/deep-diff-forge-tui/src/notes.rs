//! Engine-authored inline annotations and anchored lookups.
//!
//! A review tool is more useful when it explains *why* a file deserves
//! attention, in-place, next to the change. [`engine_annotations`] turns the
//! engine's own findings — risk signals from [`deep_diff_forge_graph`] and
//! semantic-change spans from the core model — into [`AgentAnnotation`]s
//! authored by the reserved [`SYSTEM_AGENT_ID`]. They are *grounded* (every
//! note carries the evidence that produced it), so the UI can render them with
//! the same trust posture as any other annotation, and a human or external
//! agent's notes layer on top without special-casing.
//!
//! These functions never mutate the review model and never fabricate content
//! that the engine did not derive.

use deep_diff_forge_agent::SYSTEM_AGENT_ID;
use deep_diff_forge_core::{
    AgentAnnotation, AnnotationAnchor, AnnotationProvenance, HunkId, ReviewFile, SemanticChangeKind,
};
use deep_diff_forge_graph::{RankedFile, RiskSignal};

/// Build the engine's own inline notes for a ranked review.
///
/// One risk note per file that carries any [`RiskSignal`], plus one semantic
/// note per file whose semantic twin found change spans. Files with neither
/// produce no note, so the annotation stream stays signal-dense.
#[must_use]
pub fn engine_annotations(files: &[ReviewFile], ranked: &[RankedFile]) -> Vec<AgentAnnotation> {
    let mut out = Vec::new();
    for entry in ranked {
        let Some(file) = files.iter().find(|f| f.path == entry.path) else {
            continue;
        };
        if !entry.signals.is_empty() {
            out.push(risk_note(file, &entry.signals));
        }
        if let Some(note) = semantic_note(file) {
            out.push(note);
        }
    }
    out
}

/// File-anchored annotations for `path` (notes that apply to the whole file).
#[must_use]
pub fn file_annotations<'a>(
    annotations: &'a [AgentAnnotation],
    path: &str,
) -> Vec<&'a AgentAnnotation> {
    annotations
        .iter()
        .filter(|a| matches!(&a.anchor, AnnotationAnchor::File { path: p } if p == path))
        .collect()
}

/// Annotations anchored to a specific hunk of `path`.
#[must_use]
pub fn hunk_annotations<'a>(
    annotations: &'a [AgentAnnotation],
    path: &str,
    hunk: HunkId,
) -> Vec<&'a AgentAnnotation> {
    annotations
        .iter()
        .filter(|a| {
            matches!(
                &a.anchor,
                AnnotationAnchor::Hunk { path: p, hunk_id } if p == path && *hunk_id == hunk
            )
        })
        .collect()
}

fn risk_note(file: &ReviewFile, signals: &[RiskSignal]) -> AgentAnnotation {
    let evidence: Vec<String> = signals.iter().map(|s| s.label().to_string()).collect();
    let body = signals
        .iter()
        .map(|s| signal_phrase(*s))
        .collect::<Vec<_>>()
        .join(" ");
    AgentAnnotation {
        id: format!("ddf/sys/risk:{}", file.path),
        anchor: first_anchor(file),
        body,
        provenance: AnnotationProvenance {
            agent: SYSTEM_AGENT_ID.to_string(),
            model: None,
            evidence,
        },
        grounded: true,
    }
}

fn semantic_note(file: &ReviewFile) -> Option<AgentAnnotation> {
    let twin = file.semantic_twin.as_ref()?;
    let first = twin.spans.first()?;
    let body = format!(
        "{} semantic change(s): {} (tree-sitter, {}).",
        twin.spans.len(),
        semantic_phrase(first.kind),
        twin.language
    );
    Some(AgentAnnotation {
        id: format!("ddf/sys/sem:{}", file.path),
        anchor: AnnotationAnchor::Hunk {
            path: file.path.clone(),
            hunk_id: first.hunk_id,
        },
        body,
        provenance: AnnotationProvenance {
            agent: SYSTEM_AGENT_ID.to_string(),
            model: None,
            evidence: vec![
                format!("language:{}", twin.language),
                format!("spans:{}", twin.spans.len()),
            ],
        },
        grounded: true,
    })
}

fn first_anchor(file: &ReviewFile) -> AnnotationAnchor {
    file.patch_twin.hunks.first().map_or_else(
        || AnnotationAnchor::File {
            path: file.path.clone(),
        },
        |h| AnnotationAnchor::Hunk {
            path: file.path.clone(),
            hunk_id: h.id,
        },
    )
}

fn signal_phrase(signal: RiskSignal) -> &'static str {
    match signal {
        RiskSignal::PublicApiSurface => "Public API surface — review first.",
        RiskSignal::NewFile => "New file.",
        RiskSignal::DeletedFile => "File deleted — check for lost behaviour.",
        RiskSignal::BinaryChange => "Binary change — no reviewable text.",
        RiskSignal::LargeChange => "Large change by line count.",
        RiskSignal::ManyHunks => "Many hunks — scattered edits.",
        RiskSignal::ConfigOrLockfile => "Config/lockfile change.",
        RiskSignal::TestOnly => "Test-only change (lower priority).",
        RiskSignal::GeneratedOrVendored => "Generated/vendored — usually skip.",
    }
}

fn semantic_phrase(kind: SemanticChangeKind) -> &'static str {
    match kind {
        SemanticChangeKind::AddedNode => "node added",
        SemanticChangeKind::RemovedNode => "node removed",
        SemanticChangeKind::ModifiedNode => "node modified",
        SemanticChangeKind::MovedNode => "node moved",
        SemanticChangeKind::ReformattedOnly => "reformatted only",
        SemanticChangeKind::RenamedSymbol => "symbol renamed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deep_diff_forge_agent::{AnnotationSource, GroundingLevel, grounding_of, source_of};
    use deep_diff_forge_core::{ParseStatus, SemanticSpan, SemanticSpanId, SemanticTwin};
    use deep_diff_forge_graph::rank;
    use deep_diff_forge_patch::parse;

    // lib.rs is a public-API surface, so it earns a risk signal.
    const LIB: &str = "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@\n-a\n+b\n";
    // A plain source file with a tiny change earns no signal.
    const PLAIN: &str = "--- a/src/util.rs\n+++ b/src/util.rs\n@@ -1,1 +1,1 @@\n-a\n+b\n";

    fn annos(src: &str) -> Vec<AgentAnnotation> {
        let files = parse(src).unwrap();
        let ranked = rank(&files);
        engine_annotations(&files, &ranked)
    }

    #[test]
    fn public_api_file_gets_a_risk_note() {
        let a = annos(LIB);
        assert_eq!(a.len(), 1);
        assert!(a[0].body.contains("Public API"));
    }

    #[test]
    fn risk_note_is_grounded_with_evidence() {
        let a = annos(LIB);
        assert!(a[0].grounded);
        assert!(
            a[0].provenance
                .evidence
                .iter()
                .any(|e| e == "public_api_surface")
        );
        assert_eq!(grounding_of(&a[0]), GroundingLevel::Grounded);
    }

    #[test]
    fn risk_note_is_system_authored() {
        let a = annos(LIB);
        assert_eq!(a[0].provenance.agent, SYSTEM_AGENT_ID);
        assert_eq!(source_of(&a[0]), AnnotationSource::System);
    }

    #[test]
    fn risk_note_anchors_to_the_first_hunk() {
        let files = parse(LIB).unwrap();
        let ranked = rank(&files);
        let a = engine_annotations(&files, &ranked);
        let hunk_id = files[0].patch_twin.hunks[0].id;
        assert!(matches!(
            &a[0].anchor,
            AnnotationAnchor::Hunk { path, hunk_id: h } if path == "src/lib.rs" && *h == hunk_id
        ));
    }

    #[test]
    fn plain_file_gets_no_note() {
        assert!(annos(PLAIN).is_empty());
    }

    #[test]
    fn note_id_is_stable_and_namespaced() {
        let a = annos(LIB);
        assert_eq!(a[0].id, "ddf/sys/risk:src/lib.rs");
    }

    #[test]
    fn hunk_lookup_finds_the_note() {
        let files = parse(LIB).unwrap();
        let ranked = rank(&files);
        let a = engine_annotations(&files, &ranked);
        let hunk_id = files[0].patch_twin.hunks[0].id;
        let found = hunk_annotations(&a, "src/lib.rs", hunk_id);
        assert_eq!(found.len(), 1);
        let missing = hunk_annotations(&a, "src/lib.rs", HunkId(999));
        assert!(missing.is_empty());
    }

    #[test]
    fn file_lookup_matches_file_anchor_only() {
        // A binary file has no hunks, so its risk note is File-anchored.
        let files =
            parse("diff --git a/lib.so b/lib.so\nBinary files a/lib.so and b/lib.so differ\n")
                .unwrap();
        let ranked = rank(&files);
        let a = engine_annotations(&files, &ranked);
        // Binary change is a signal, so there is a note, and it is file-anchored.
        assert!(!a.is_empty());
        let by_file = file_annotations(&a, "lib.so");
        assert_eq!(
            by_file.len(),
            a.iter()
                .filter(|x| matches!(x.anchor, AnnotationAnchor::File { .. }))
                .count()
        );
    }

    #[test]
    fn semantic_twin_produces_a_semantic_note() {
        let mut files = parse(LIB).unwrap();
        let hunk_id = files[0].patch_twin.hunks[0].id;
        files[0].semantic_twin = Some(SemanticTwin {
            language: "rust".to_string(),
            parse_status: ParseStatus::Parsed,
            spans: vec![SemanticSpan {
                id: SemanticSpanId(1),
                hunk_id,
                kind: SemanticChangeKind::ModifiedNode,
                old_range: None,
                new_range: None,
            }],
        });
        let ranked = rank(&files);
        let a = engine_annotations(&files, &ranked);
        let sem: Vec<_> = a
            .iter()
            .filter(|x| x.id.starts_with("ddf/sys/sem:"))
            .collect();
        assert_eq!(sem.len(), 1);
        assert!(sem[0].body.contains("node modified"));
        assert!(sem[0].body.contains("semantic change"));
    }

    #[test]
    fn no_semantic_twin_yields_no_semantic_note() {
        let a = annos(LIB);
        assert!(a.iter().all(|x| !x.id.starts_with("ddf/sys/sem:")));
    }

    #[test]
    fn every_signal_has_a_nonempty_phrase() {
        let signals = [
            RiskSignal::PublicApiSurface,
            RiskSignal::NewFile,
            RiskSignal::DeletedFile,
            RiskSignal::BinaryChange,
            RiskSignal::LargeChange,
            RiskSignal::ManyHunks,
            RiskSignal::ConfigOrLockfile,
            RiskSignal::TestOnly,
            RiskSignal::GeneratedOrVendored,
        ];
        for s in signals {
            assert!(!signal_phrase(s).is_empty());
        }
    }

    #[test]
    fn every_semantic_kind_has_a_nonempty_phrase() {
        let kinds = [
            SemanticChangeKind::AddedNode,
            SemanticChangeKind::RemovedNode,
            SemanticChangeKind::ModifiedNode,
            SemanticChangeKind::MovedNode,
            SemanticChangeKind::ReformattedOnly,
            SemanticChangeKind::RenamedSymbol,
        ];
        for k in kinds {
            assert!(!semantic_phrase(k).is_empty());
        }
    }
}
