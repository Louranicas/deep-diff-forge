//! Review Intelligence Graph for Deep-Diff-Forge (L5).
//!
//! Ranks a review by likely impact rather than file order, using deterministic,
//! explainable signals derived from the patch model (public-API surface, change
//! size, hunk count, new/deleted/binary, generated/vendored suppression,
//! test-only de-prioritization). Ranking never mutates patch truth and is
//! reproducible: the same review always yields the same order.
//!
//! Signals come from patch facts (path, status, size, modes, hazards) and from code evidence:
//! what the changed lines do, found lexically and pointed at by hunk address and line
//! ([`evidence`]). Ranking reads only the patch, so it works for every input.

pub mod evidence;
pub mod lexical;
mod rank;
mod risk;
mod rust_tokens;
#[cfg(test)]
mod unplaced;
mod vocabulary;

pub use evidence::{
    Basis, EVIDENCE_BASIS, Evidence, EvidenceSignal, FileKinds, HunkLabels, MAX_EVIDENCE_PER_FILE,
    Side, file_evidence, file_evidence_released, file_walks, hunk_signals, hunk_signals_alone,
    joined_hunks, signal_sides, view_lines_peak,
};
#[doc(hidden)]
pub use evidence::{CarriedShares, carried_bytes_on_this_thread, hunk_signals_in_patch};
pub use rank::{RankedFile, rank, sort_ranked};
#[doc(hidden)]
pub use risk::assess_file_in_patch;
pub use risk::{
    Assessment, LARGE_CHANGE_LINES, MANY_HUNKS, RiskSignal, assess_file, change_counts, score_file,
};
pub use vocabulary::{
    NativeLint, NotAccepted, PATH_BASIS, PATH_SIGNALS, Remedy, SignalClass, SignalFacts,
    SignalKind, facts, remedy, signal_named, vocabulary,
};

/// The `deep-diff-forge.rank.v0` document, rendered by one function for every surface (S3.15):
/// [`RankDocument::to_json`] is the bytes `--stdin-patch --rank --json` prints, newline included,
/// and the daemon's `diff.rank` (so `rpc` and MCP's `rank_patch`) carries the same bytes, its
/// value form being those bytes parsed.
#[derive(Debug, Clone, Copy)]
pub struct RankDocument<'a> {
    /// The seal over the exact input bytes the ranking was computed from.
    pub seal: &'a deep_diff_forge_core::InputSeal,
    /// The files in review-risk order ([`rank`]).
    pub ranked: &'a [RankedFile],
}

impl RankDocument<'_> {
    /// The canonical bytes: schema first, two-space indent, one ranked entry a line.
    #[must_use]
    pub fn to_json(&self) -> String {
        use deep_diff_forge_core::{InputSeal, json_escape};
        let entries: Vec<String> = self.ranked.iter().map(|rf| entry_json(rf, true)).collect();
        let body = if entries.is_empty() {
            String::new()
        } else {
            format!("\n    {}\n  ", entries.join(",\n    "))
        };
        format!(
            "{{\n  \"schema\": \"deep-diff-forge.rank.v0\",\n  \"input_sha256\": {},\n  \"tool\": {{\"name\": {}, \"version\": {}}},\n  \"ranked\": [{body}]\n}}\n",
            json_escape(&self.seal.input_sha256),
            json_escape(InputSeal::TOOL_NAME),
            json_escape(&self.seal.tool_version),
        )
    }
}

/// The ranked entries as one JSON array (`diff.plan`, `session.snapshot`): the document's entries.
#[must_use]
pub fn ranked_array_json(ranked: &[RankedFile]) -> String {
    let entries: Vec<String> = ranked.iter().map(|rf| entry_json(rf, true)).collect();
    format!("[{}]", entries.join(", "))
}

/// One ranked entry as `rank.v0` writes it; `full` false leaves out the optional members
/// (`evidence`, `evidence_truncated`, `diff_suppressed`), as `cluster.v0`'s list has none.
#[must_use]
pub fn entry_json(rf: &RankedFile, full: bool) -> String {
    use deep_diff_forge_core::json_escape;
    use std::fmt::Write as _;
    let signals: Vec<String> = rf.signals.iter().map(|s| json_escape(s.label())).collect();
    let mut out = format!(
        "{{\"path\": {}, \"status\": {}, \"score\": {}, \"signals\": [{}]",
        json_escape(&rf.path),
        json_escape(rf.status.label()),
        rf.score,
        signals.join(", ")
    );
    // Each optional member is present only when found or true, as `hazards` is.
    if full && !rf.evidence.is_empty() {
        let items: Vec<String> = rf
            .evidence
            .iter()
            .map(|e| {
                format!(
                    "{{\"signal\": {}, \"address\": {}, \"side\": {}, \"line\": {}, \"basis\": {}{}}}",
                    json_escape(e.signal.label()),
                    json_escape(&e.address),
                    json_escape(e.side.label()),
                    e.line,
                    json_escape(e.basis.label()),
                    if e.test_likely {
                        ", \"test_likely\": true"
                    } else {
                        ""
                    }
                )
            })
            .collect();
        let _ = write!(out, ", \"evidence\": [{}]", items.join(", "));
    }
    if full && rf.evidence_truncated {
        out.push_str(", \"evidence_truncated\": true");
    }
    if let (true, Some(cause)) = (full, rf.diff_suppressed) {
        let _ = write!(out, ", \"diff_suppressed\": {}", json_escape(cause.label()));
    }
    out.push('}');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use deep_diff_forge_patch::parse;

    #[test]
    fn public_api_facade_reexports_work() {
        let files = parse("--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@\n-a\n+b\n").unwrap();
        let ranked = rank(&files);
        assert_eq!(ranked.len(), 1);
        assert!(ranked[0].signals.contains(&RiskSignal::PublicApiSurface));
    }

    #[test]
    fn score_file_reexport_works() {
        let files = parse("--- a/x.rs\n+++ b/x.rs\n@@ -1,1 +1,1 @@\n-a\n+b\n").unwrap();
        let (score, _) = score_file(&files[0]);
        assert!(score >= 1);
    }

    #[test]
    fn change_counts_reexport_works() {
        let files = parse("--- a/x\n+++ b/x\n@@ -1,1 +1,1 @@\n-a\n+b\n").unwrap();
        assert_eq!(change_counts(&files[0]), (1, 1));
    }

    #[test]
    fn thresholds_are_exposed() {
        assert_eq!(LARGE_CHANGE_LINES, 80);
        assert_eq!(MANY_HUNKS, 5);
    }
}
