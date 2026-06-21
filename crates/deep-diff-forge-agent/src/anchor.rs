use deep_diff_forge_core::{AnnotationAnchor, ReviewFile};

/// The file path an anchor refers to.
#[must_use]
pub fn anchor_path(anchor: &AnnotationAnchor) -> &str {
    match anchor {
        AnnotationAnchor::File { path }
        | AnnotationAnchor::Hunk { path, .. }
        | AnnotationAnchor::SemanticSpan { path, .. } => path,
    }
}

/// Validate that an anchor refers to something that actually exists in the
/// review: the file must be present, and for hunk/span anchors the referenced
/// id must exist within that file. Untrusted anchors that do not resolve are
/// rejected (returns `false`) rather than silently accepted.
#[must_use]
pub fn validate_anchor(anchor: &AnnotationAnchor, files: &[ReviewFile]) -> bool {
    let path = anchor_path(anchor);
    let Some(file) = files.iter().find(|f| f.path == path) else {
        return false;
    };
    match anchor {
        AnnotationAnchor::File { .. } => true,
        AnnotationAnchor::Hunk { hunk_id, .. } => {
            file.patch_twin.hunks.iter().any(|h| h.id == *hunk_id)
        }
        AnnotationAnchor::SemanticSpan { span_id, .. } => file
            .semantic_twin
            .as_ref()
            .is_some_and(|t| t.spans.iter().any(|s| s.id == *span_id)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deep_diff_forge_core::HunkId;
    use deep_diff_forge_patch::parse;

    fn review() -> Vec<ReviewFile> {
        parse("--- a/src/x.rs\n+++ b/src/x.rs\n@@ -1,1 +1,1 @@\n-a\n+b\n").unwrap()
    }

    #[test]
    fn anchor_path_extracts_file() {
        let a = AnnotationAnchor::File {
            path: "p.rs".into(),
        };
        assert_eq!(anchor_path(&a), "p.rs");
    }

    #[test]
    fn anchor_path_extracts_hunk() {
        let a = AnnotationAnchor::Hunk {
            path: "p.rs".into(),
            hunk_id: HunkId(0),
        };
        assert_eq!(anchor_path(&a), "p.rs");
    }

    #[test]
    fn file_anchor_validates_when_present() {
        let a = AnnotationAnchor::File {
            path: "src/x.rs".into(),
        };
        assert!(validate_anchor(&a, &review()));
    }

    #[test]
    fn file_anchor_rejected_when_absent() {
        let a = AnnotationAnchor::File {
            path: "src/missing.rs".into(),
        };
        assert!(!validate_anchor(&a, &review()));
    }

    #[test]
    fn hunk_anchor_validates_existing_hunk() {
        let a = AnnotationAnchor::Hunk {
            path: "src/x.rs".into(),
            hunk_id: HunkId(0),
        };
        assert!(validate_anchor(&a, &review()));
    }

    #[test]
    fn hunk_anchor_rejected_for_missing_hunk_id() {
        let a = AnnotationAnchor::Hunk {
            path: "src/x.rs".into(),
            hunk_id: HunkId(999),
        };
        assert!(!validate_anchor(&a, &review()));
    }

    #[test]
    fn hunk_anchor_rejected_for_wrong_file() {
        let a = AnnotationAnchor::Hunk {
            path: "src/other.rs".into(),
            hunk_id: HunkId(0),
        };
        assert!(!validate_anchor(&a, &review()));
    }

    #[test]
    fn span_anchor_rejected_when_no_semantic_twin() {
        use deep_diff_forge_core::SemanticSpanId;
        let a = AnnotationAnchor::SemanticSpan {
            path: "src/x.rs".into(),
            span_id: SemanticSpanId(0),
        };
        // The patch parser produces no semantic twin, so any span anchor fails.
        assert!(!validate_anchor(&a, &review()));
    }

    #[test]
    fn validation_against_empty_review_is_false() {
        let a = AnnotationAnchor::File {
            path: "x.rs".into(),
        };
        assert!(!validate_anchor(&a, &[]));
    }
}
