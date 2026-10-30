// DDF integration: R14, R15, R16, R17, R52. Separate asserted citations from captured artifact integrity, trusted evaluator support and revision freshness. Do not resolve duplicate IDs together.
// Design/status: docs/RECOMMENDATION_INTEGRATION.md#evidence
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fdeep-diff-forge-tui--notes
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
use deep_diff_forge_graph::{EvidenceSignal, RankedFile, RiskSignal};

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
        out.extend(evidence_notes(file, entry));
        if let Some(note) = semantic_note(file) {
            out.push(note);
        }
    }
    out
}

/// The first path that `files` names more than once, if any. Notes, ranking and the cockpit's
/// content are keyed by path, so a review that names a path twice (two patches concatenated)
/// would show one section's content and notes for both; callers refuse it instead (bug hunt
/// 2026-10-28, H4-03).
#[must_use]
pub fn duplicate_path(files: &[ReviewFile]) -> Option<&str> {
    let mut seen = std::collections::HashSet::new();
    files
        .iter()
        .map(|f| f.path.as_str())
        .find(|path| !seen.insert(*path))
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
    // Count each kind, in order of first appearance: naming every span after the first one's
    // kind called three added nodes and a removal "4 semantic change(s): node added" (bug hunt
    // 2026-10-28, H4-10).
    let mut kinds: Vec<(SemanticChangeKind, usize)> = Vec::new();
    for span in &twin.spans {
        match kinds.iter_mut().find(|(kind, _)| *kind == span.kind) {
            Some((_, count)) => *count += 1,
            None => kinds.push((span.kind, 1)),
        }
    }
    let breakdown: Vec<String> = kinds
        .iter()
        .map(|(kind, count)| format!("{count} {}", semantic_phrase(*kind)))
        .collect();
    let body = format!(
        "{} semantic change(s): {} (tree-sitter, {}).",
        twin.spans.len(),
        breakdown.join(", "),
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
        RiskSignal::GeneratedOrVendored => {
            "Generated/vendored path — lower priority, but its author chose the path."
        }
        RiskSignal::HazardousText => {
            "Hidden or control characters, or a traversing path — what you see may not be what applies."
        }
        RiskSignal::SymlinkChange => "Symlink — check where it points.",
        RiskSignal::ExecutableBit => "Becomes executable — check what runs it.",
        RiskSignal::Code(signal) => code_phrase(signal),
    }
}

fn code_phrase(signal: EvidenceSignal) -> &'static str {
    match signal {
        EvidenceSignal::UnsafeAdded => "`unsafe` added.",
        EvidenceSignal::ProcessSpawn => "Starts a process.",
        EvidenceSignal::NetworkFetch => "Makes a network call or download.",
        EvidenceSignal::DynamicEval => "Builds and runs code from data.",
        EvidenceSignal::CiPrivilege => "CI workflow gains a privilege.",
        EvidenceSignal::BuildHook => "Runs at build or install time.",
        EvidenceSignal::SecretLike => "Looks like a credential.",
        EvidenceSignal::CheckSuppressed => "A lint, type or coverage check is switched off.",
        EvidenceSignal::TestRemoved => "A test is removed.",
    }
}

/// One system note per evidence item, on its hunk, naming the line (lexical evidence: the note
/// says what was found, not that the change is wrong).
fn evidence_notes(file: &ReviewFile, ranked: &RankedFile) -> Vec<AgentAnnotation> {
    ranked
        .evidence
        .iter()
        .map(|item| AgentAnnotation {
            id: format!(
                "ddf/sys/evidence:{}:{}:{}:{}",
                file.path,
                item.address,
                item.side.label(),
                item.signal.label()
            ),
            anchor: AnnotationAnchor::Hunk {
                path: file.path.clone(),
                hunk_id: item.hunk,
            },
            body: format!(
                "{} ({} line {})",
                code_phrase(item.signal),
                item.side.label(),
                item.line
            ),
            provenance: AnnotationProvenance {
                agent: SYSTEM_AGENT_ID.to_string(),
                model: None,
                evidence: vec![
                    item.signal.label().to_string(),
                    format!("basis:{}", deep_diff_forge_graph::EVIDENCE_BASIS),
                ],
            },
            grounded: true,
        })
        .collect()
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
    fn duplicate_paths_are_found() {
        assert_eq!(
            duplicate_path(&parse(&format!("{LIB}{PLAIN}")).unwrap()),
            None
        );
        assert_eq!(
            duplicate_path(&parse(&format!("{PLAIN}{LIB}{LIB}")).unwrap()),
            Some("src/lib.rs")
        );
        assert_eq!(duplicate_path(&[]), None);
    }

    #[test]
    fn public_api_file_gets_a_risk_note() {
        let a = annos(LIB);
        assert_eq!(a.len(), 1);
        assert!(a[0].body.contains("Public API"));
    }

    #[test]
    fn risk_note_cites_asserted_risk_signals() {
        let a = annos(LIB);
        assert!(a[0].grounded);
        assert!(
            a[0].provenance
                .evidence
                .iter()
                .any(|e| e == "public_api_surface")
        );
        assert_eq!(grounding_of(&a[0]), GroundingLevel::PartiallyGrounded);
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
        assert_eq!(annos(PLAIN).len(), 0);
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
        assert_eq!(missing.len(), 0);
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
        assert_ne!(a.len(), 0);
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

    /// Mixed kinds are each counted, not all named after the first (bug hunt H4-10).
    #[test]
    fn a_semantic_note_counts_each_kind() {
        let mut files = parse(LIB).unwrap();
        let hunk_id = files[0].patch_twin.hunks[0].id;
        let span = |id, kind| SemanticSpan {
            id: SemanticSpanId(id),
            hunk_id,
            kind,
            old_range: None,
            new_range: None,
        };
        files[0].semantic_twin = Some(SemanticTwin {
            language: "rust".to_string(),
            parse_status: ParseStatus::Parsed,
            spans: vec![
                span(1, SemanticChangeKind::AddedNode),
                span(2, SemanticChangeKind::RemovedNode),
                span(3, SemanticChangeKind::AddedNode),
                span(4, SemanticChangeKind::AddedNode),
            ],
        });
        let a = engine_annotations(&files, &rank(&files));
        let sem = a.iter().find(|x| x.id.starts_with("ddf/sys/sem:")).unwrap();
        assert_eq!(
            sem.body,
            "4 semantic change(s): 3 node added, 1 node removed (tree-sitter, rust)."
        );
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
            RiskSignal::HazardousText,
            RiskSignal::SymlinkChange,
            RiskSignal::ExecutableBit,
        ];
        for s in signals {
            assert_ne!(signal_phrase(s).len(), 0);
        }
        let distinct: std::collections::BTreeSet<_> =
            signals.iter().map(|s| signal_phrase(*s)).collect();
        assert_eq!(
            distinct.len(),
            signals.len(),
            "each signal has its own phrase"
        );
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
            assert_ne!(semantic_phrase(k).len(), 0);
        }
    }
}
