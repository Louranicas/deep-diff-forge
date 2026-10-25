// DDF integration: R02, R09, R19, R21. Validate that a patch transforms the captured base into the captured candidate before attaching semantics; export explicit coverage and snapshot scope.
// Design/status: docs/RECOMMENDATION_INTEGRATION.md#shared-service
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fdeep-diff-forge-pipeline--review
//! Shared review service: capture → validate source pairs → syntax → rank → export.
//! R02/R09/R19/R21: `docs/RECOMMENDATION_INTEGRATION.md#shared-service` links to
//! the exact vault note, tests and deferred daemon/TUI consumers. Patch truth is
//! immutable; enrichment lives in a separate model. Syntax is not behavior proof.
use deep_diff_forge_core::{
    DiffStrategy, FileStatus, ParseStatus, PatchLineKind, ReviewFile, SemanticChangeKind,
    SemanticSpan, SemanticSpanId, SemanticTwin, TextRange, json_escape, sha256_hex,
};
use deep_diff_forge_patch::{CapturedPatch, PatchParseError};
use deep_diff_forge_syntax::{ChangeKind, StructuralDiff, detect_language, structural_diff};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write as _,
};

/// Caller-supplied full source bytes for one changed path. `None` means absent,
/// not an empty file. Renames/copies/binary bodies require a richer source adapter.
#[derive(Debug, Clone)]
pub struct SourcePair {
    pub path: String,
    pub old: Option<String>,
    pub new: Option<String>,
}
/// Every changed file gets a coverage entry, including skipped and missing ones.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Coverage {
    Analyzed,
    PartialLcs,
    MissingSource,
    UnsupportedChange,
    Unavailable(String),
}
impl Coverage {
    fn label(&self) -> &str {
        match self {
            Self::Analyzed => "analyzed",
            Self::PartialLcs => "partial_lcs",
            Self::MissingSource => "missing_source",
            Self::UnsupportedChange => "unsupported_change",
            Self::Unavailable(_) => "unavailable",
        }
    }
}
/// One explicit record of the analysis attempted for a file.
#[derive(Debug, Clone)]
pub struct FileAnalysis {
    pub path: String,
    pub coverage: Coverage,
    pub structural: Option<StructuralDiff>,
}
/// Immutable source identity + captured patch + separate enriched view.
#[derive(Debug, Clone)]
pub struct ReviewBundle {
    patch: CapturedPatch,
    id: String,
    base_id: String,
    candidate_id: String,
    files: Vec<ReviewFile>,
    analyses: Vec<FileAnalysis>,
}
/// Acquisition and validation errors are hard failures, never an empty clean diff.
#[derive(Debug)]
pub enum ReviewError {
    Patch(PatchParseError),
    InvalidSources(String),
}
impl std::fmt::Display for ReviewError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Patch(e) => write!(f, "{e}"),
            Self::InvalidSources(s) => write!(f, "source validation: {s}"),
        }
    }
}
impl std::error::Error for ReviewError {}
fn snapshot_id(pairs: &BTreeMap<&str, &SourcePair>, old: bool) -> String {
    deep_diff_forge_core::changed_source_snapshot_id(pairs.iter().map(|(path, pair)| {
        (
            *path,
            if old {
                pair.old.as_deref()
            } else {
                pair.new.as_deref()
            }
            .map(str::as_bytes),
        )
    }))
}
/// Analyze captured inputs without network, subprocesses or agent-executed commands.
/// The changed-source snapshots are scoped to supplied changed paths, not a Git tree.
///
/// # Errors
/// Rejects duplicate/extra sources, over-budget acquisition or patch/source mismatch.
pub fn analyze_review(raw: String, sources: &[SourcePair]) -> Result<ReviewBundle, ReviewError> {
    let patch = CapturedPatch::capture(raw).map_err(ReviewError::Patch)?;
    let paths: BTreeSet<_> = patch.files().iter().map(|f| f.path.as_str()).collect();
    if paths.len() != patch.files().len() {
        return Err(ReviewError::InvalidSources(
            "repeated file sections require a sequential patch adapter".into(),
        ));
    }
    let mut pairs = BTreeMap::new();
    let mut bytes = 0usize;
    for pair in sources {
        bytes = bytes
            .saturating_add(pair.old.as_ref().map_or(0, String::len))
            .saturating_add(pair.new.as_ref().map_or(0, String::len));
        if bytes > deep_diff_forge_patch::DEFAULT_BYTE_BUDGET {
            return Err(ReviewError::InvalidSources(
                "aggregate source byte budget exceeded".into(),
            ));
        }
        if pairs.insert(pair.path.as_str(), pair).is_some() || !paths.contains(pair.path.as_str()) {
            return Err(ReviewError::InvalidSources(format!(
                "duplicate or extra path {:?}",
                pair.path
            )));
        }
    }
    let base_id = snapshot_id(&pairs, true);
    let candidate_id = snapshot_id(&pairs, false);
    let id = sha256_hex(
        format!(
            "ddf.review.v1\0{}\0{base_id}\0{candidate_id}",
            patch.digest()
        )
        .as_bytes(),
    );
    let mut files = patch.files().to_vec();
    let mut analyses = Vec::new();
    let mut span_id = 0u64;
    for file in &mut files {
        let Some(pair) = pairs.get(file.path.as_str()) else {
            analyses.push(FileAnalysis {
                path: file.path.clone(),
                coverage: Coverage::MissingSource,
                structural: None,
            });
            continue;
        };
        if !matches!(
            file.status,
            FileStatus::Added | FileStatus::Deleted | FileStatus::Modified
        ) || file.patch_twin.hunks.is_empty()
            || file
                .patch_twin
                .metadata
                .iter()
                .any(|m| m.starts_with("copy from "))
        {
            analyses.push(FileAnalysis {
                path: file.path.clone(),
                coverage: Coverage::UnsupportedChange,
                structural: None,
            });
            continue;
        }
        let absence_valid = match file.status {
            FileStatus::Added => pair.old.is_none() && pair.new.is_some(),
            FileStatus::Deleted => pair.old.is_some() && pair.new.is_none(),
            _ => pair.old.is_some() && pair.new.is_some(),
        };
        if !absence_valid
            || !patch_matches(
                file,
                pair.old.as_deref().unwrap_or(""),
                pair.new.as_deref().unwrap_or(""),
            )
        {
            return Err(ReviewError::InvalidSources(format!(
                "patch does not transform supplied base into candidate for {:?}",
                file.path
            )));
        }
        analyses.push(enrich_file(file, pair, &mut span_id));
    }
    Ok(ReviewBundle {
        patch,
        id,
        base_id,
        candidate_id,
        files,
        analyses,
    })
}
fn enrich_file(file: &mut ReviewFile, pair: &SourcePair, span_id: &mut u64) -> FileAnalysis {
    let old = pair.old.as_deref().unwrap_or("");
    let new = pair.new.as_deref().unwrap_or("");
    let old_ranges = line_ranges(old);
    let new_ranges = line_ranges(new);
    let diff = structural_diff(detect_language(&file.path), old, new);
    let coverage = if let Some(reason) = &diff.unavailable {
        file.planner.fallback = Some(reason.clone());
        Coverage::Unavailable(format!("{reason:?}"))
    } else if diff.degraded {
        Coverage::PartialLcs
    } else {
        Coverage::Analyzed
    };
    let mut spans = Vec::new();
    let mut old_hunks = BTreeMap::new();
    let mut new_hunks = BTreeMap::new();
    for hunk in &file.patch_twin.hunks {
        for line in &hunk.lines {
            if let Some(n) = line.old_line {
                old_hunks.insert(n, hunk.id);
            }
            if let Some(n) = line.new_line {
                new_hunks.insert(n, hunk.id);
            }
        }
    }
    for change in &diff.changes {
        if change.kind == ChangeKind::Unchanged {
            continue;
        }
        let hunk = change
            .old_line
            .and_then(|n| old_hunks.get(&n))
            .or_else(|| change.new_line.and_then(|n| new_hunks.get(&n)));
        if let Some(hunk_id) = hunk {
            spans.push(SemanticSpan {
                id: SemanticSpanId(*span_id),
                hunk_id: *hunk_id,
                kind: match change.kind {
                    ChangeKind::Added => SemanticChangeKind::AddedNode,
                    ChangeKind::Removed => SemanticChangeKind::RemovedNode,
                    ChangeKind::Moved => SemanticChangeKind::MovedNode,
                    ChangeKind::Unchanged => continue,
                },
                old_range: change
                    .old_line
                    .and_then(|line| old_ranges.get(line.saturating_sub(1) as usize).copied()),
                new_range: change
                    .new_line
                    .and_then(|line| new_ranges.get(line.saturating_sub(1) as usize).copied()),
            });
            *span_id += 1;
        }
    }
    file.semantic_twin = Some(SemanticTwin {
        language: diff.language.name().into(),
        parse_status: diff
            .unavailable
            .clone()
            .map_or(ParseStatus::Parsed, |reason| ParseStatus::Fallback {
                reason,
            }),
        spans,
    });
    if diff.unavailable.is_none() {
        file.planner.strategy = DiffStrategy::Syntax;
    }
    FileAnalysis {
        path: file.path.clone(),
        coverage,
        structural: Some(diff),
    }
}

fn range_json(range: Option<TextRange>) -> String {
    range.map_or_else(
        || "null".into(),
        |r| {
            format!(
                "{{\"start_byte\":{},\"end_byte\":{},\"start_line\":{},\"end_line\":{}}}",
                r.start_byte, r.end_byte, r.start_line, r.end_line
            )
        },
    )
}
fn line_ranges(source: &str) -> Vec<TextRange> {
    let mut offset = 0u64;
    source
        .split_inclusive('\n')
        .enumerate()
        .map(|(index, text)| {
            let line = u32::try_from(index + 1).unwrap_or(u32::MAX);
            let range = TextRange {
                start_byte: offset,
                end_byte: offset + text.len() as u64,
                start_line: line,
                end_line: line,
            };
            offset += text.len() as u64;
            range
        })
        .collect()
}
fn patch_matches(file: &ReviewFile, old: &str, new: &str) -> bool {
    let old_lines: Vec<_> = old.split_inclusive('\n').collect();
    let mut cursor = 0usize;
    let mut output = String::new();
    let mut output_lines = 0usize;
    for hunk in &file.patch_twin.hunks {
        let start = hunk.old_start.unwrap_or(0).saturating_sub(1) as usize;
        // An insertion after line N has old_start=N, old_count=0.
        let old_count = hunk
            .lines
            .iter()
            .filter(|l| l.kind != PatchLineKind::Added)
            .count();
        let start = if old_count == 0 {
            hunk.old_start.unwrap_or(0) as usize
        } else {
            start
        };
        if start < cursor || start > old_lines.len() {
            return false;
        }
        output_lines += start - cursor;
        let new_count = hunk
            .lines
            .iter()
            .filter(|l| l.kind != PatchLineKind::Removed)
            .count();
        let expected_start = if new_count == 0 {
            output_lines
        } else {
            output_lines + 1
        };
        if hunk.new_start.map(|n| n as usize) != Some(expected_start) {
            return false;
        }
        for line in &old_lines[cursor..start] {
            output.push_str(line);
        }
        cursor = start;
        for line in &hunk.lines {
            let body = format!("{}{}", line.text, if line.no_newline { "" } else { "\n" });
            if line.kind != PatchLineKind::Added {
                if old_lines.get(cursor).copied() != Some(body.as_str()) {
                    return false;
                }
                cursor += 1;
            }
            if line.kind != PatchLineKind::Removed {
                output.push_str(&body);
                output_lines += 1;
            }
        }
    }
    for line in &old_lines[cursor..] {
        output.push_str(line);
    }
    output == new
}
impl ReviewBundle {
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }
    #[must_use]
    pub fn files(&self) -> &[ReviewFile] {
        &self.files
    }
    #[must_use]
    pub fn analyses(&self) -> &[FileAnalysis] {
        &self.analyses
    }
    #[must_use]
    pub fn patch(&self) -> &CapturedPatch {
        &self.patch
    }
    /// Canonical machine envelope: explicit coverage and no behavioral-equivalence claim.
    #[must_use]
    pub fn to_json(&self) -> String {
        let mut records = Vec::new();
        for (analysis, file) in self.analyses.iter().zip(&self.files) {
            let mut record = format!(
                "{{\"path\":{},\"coverage\":{}",
                json_escape(&analysis.path),
                json_escape(analysis.coverage.label())
            );
            if let Coverage::Unavailable(reason) = &analysis.coverage {
                let _ = write!(record, ",\"reason\":{}", json_escape(reason));
            }
            if let Some(diff) = &analysis.structural {
                let _ = write!(
                    record,
                    ",\"reformat_only\":{},\"degraded\":{},\"added\":{},\"removed\":{},\"moved\":{}",
                    diff.reformat_only, diff.degraded, diff.added, diff.removed, diff.moved
                );
            }
            if let Some(twin) = file.semantic_twin.as_ref() {
                let spans = twin.spans.iter().map(|s| format!("{{\"id\":{},\"hunk_id\":{},\"kind\":{},\"old_range\":{},\"new_range\":{}}}", s.id.0, s.hunk_id.0, json_escape(&format!("{:?}", s.kind)), range_json(s.old_range), range_json(s.new_range))).collect::<Vec<_>>().join(",");
                let _ = write!(
                    record,
                    ",\"span_precision\":\"source_line\",\"semantic_spans\":[{spans}]"
                );
            }
            record.push('}');
            records.push(record);
        }
        let ranked = deep_diff_forge_graph::rank(&self.files)
            .iter()
            .map(|f| {
                format!(
                    "{{\"path\":{},\"score\":{}}}",
                    json_escape(&f.path),
                    f.score
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "{{\"schema\":\"deep-diff-forge.analysis.v1\",\"review_id\":{},\"patch_sha256\":{},\"base_snapshot\":{},\"candidate_snapshot\":{},\"snapshot_scope\":\"supplied_changed_sources\",\"behavior_verified\":false,\"coverage\":[{}],\"ranked\":[{}],\"review\":{}}}\n",
            json_escape(&self.id),
            json_escape(self.patch.digest()),
            json_escape(&self.base_id),
            json_escape(&self.candidate_id),
            records.join(","),
            ranked,
            deep_diff_forge_patch::to_json(&self.files)
        )
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use deep_diff_forge_core::HunkId;
    const PATCH: &str = "--- a/x.rs\n+++ b/x.rs\n@@ -1 +1 @@\n-fn a() {}\n+fn b() {}\n";
    fn pair() -> SourcePair {
        SourcePair {
            path: "x.rs".into(),
            old: Some("fn a() {}\n".into()),
            new: Some("fn b() {}\n".into()),
        }
    }
    #[test]
    fn verified_pair_is_enriched_without_mutating_patch_truth() {
        let bundle = analyze_review(PATCH.into(), &[pair()]).unwrap();
        assert_eq!(bundle.patch().original(), PATCH);
        assert!(bundle.patch().files()[0].semantic_twin.is_none());
        let twin = bundle.files()[0].semantic_twin.as_ref().unwrap();
        assert_ne!(twin.spans.len(), 0);
        assert!(twin.spans.iter().all(|s| s.hunk_id == HunkId(0)));
        assert_eq!(bundle.analyses()[0].coverage, Coverage::Analyzed);
        assert_eq!(
            bundle.id(),
            analyze_review(PATCH.into(), &[pair()]).unwrap().id()
        );
        assert!(bundle.to_json().contains("\"behavior_verified\":false"));
    }
    #[test]
    fn mismatched_duplicate_extra_or_absent_sides_fail() {
        let mut bad = pair();
        bad.old = Some("wrong\n".into());
        assert!(analyze_review(PATCH.into(), &[bad]).is_err());
        assert!(analyze_review(PATCH.into(), &[pair(), pair()]).is_err());
        let mut bad = pair();
        bad.path = "other.rs".into();
        assert!(analyze_review(PATCH.into(), &[bad]).is_err());
        let mut bad = pair();
        bad.new = None;
        assert!(analyze_review(PATCH.into(), &[bad]).is_err());
    }
    #[test]
    fn skipped_and_unsupported_are_not_clean() {
        let bundle = analyze_review(PATCH.into(), &[]).unwrap();
        assert_eq!(bundle.analyses()[0].coverage, Coverage::MissingSource);
        let patch = PATCH.replace("x.rs", "x.txt");
        let mut source = pair();
        source.path = "x.txt".into();
        let bundle = analyze_review(patch, &[source]).unwrap();
        assert!(matches!(
            bundle.analyses()[0].coverage,
            Coverage::Unavailable(_)
        ));
        assert!(
            !bundle.analyses()[0]
                .structural
                .as_ref()
                .unwrap()
                .reformat_only
        );
    }
    #[test]
    fn insertion_after_a_line_and_eof_marker_validate_exact_bytes() {
        let patch =
            "--- a/x.rs\n+++ b/x.rs\n@@ -1,0 +2 @@\n+fn b() {}\n\\ No newline at end of file\n";
        let source = SourcePair {
            path: "x.rs".into(),
            old: Some("fn a() {}\n".into()),
            new: Some("fn a() {}\nfn b() {}".into()),
        };
        assert!(analyze_review(patch.into(), &[source]).is_ok());
    }
    #[test]
    fn changes_to_unmodified_context_change_snapshot_identity() {
        let patch = "--- a/x.rs\n+++ b/x.rs\n@@ -1 +1 @@\n-fn a() {}\n+fn b() {}\n";
        let original = analyze_review(patch.into(), &[pair()]).unwrap();
        let mut source = pair();
        source.old.as_mut().unwrap().push_str("// context\n");
        source.new.as_mut().unwrap().push_str("// context\n");
        let contextual = analyze_review(patch.into(), &[source]).unwrap();
        assert_ne!(original.id(), contextual.id());
    }
}
