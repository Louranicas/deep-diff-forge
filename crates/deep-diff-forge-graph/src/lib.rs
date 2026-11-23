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

mod diagnostics;
pub mod evidence;
pub mod lexical;
mod rank;
mod risk;
mod rust_tokens;
#[cfg(test)]
mod unplaced;
mod vocabulary;

pub use diagnostics::{Diagnostic, Diagnostics, Judge, code_of, diagnostics};
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

/// The identity of the scorer this crate was built from: the SHA-256 of every file under `src/`
/// but `lib.rs` (the rule tables, the secret prefixes and their byte classes, the readers, the
/// path rules and weights, the order and the vocabulary), taken at build time by `build.rs`. It
/// moves on any edit of those files, a comment included; `rank.v0` carries it as
/// `tool.scorer_sha256`, since `tool.version` alone does not tell two scorers apart.
#[must_use]
pub fn scorer_sha256() -> &'static str {
    include_str!(concat!(env!("OUT_DIR"), "/scorer_sha256"))
}

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
            "{{\n  \"schema\": \"deep-diff-forge.rank.v0\",\n  \"input_sha256\": {},\n  \"tool\": {{\"name\": {}, \"version\": {}, \"scorer_sha256\": {}}},\n  \"ranked\": [{body}]\n}}\n",
            json_escape(&self.seal.input_sha256),
            json_escape(InputSeal::TOOL_NAME),
            json_escape(&self.seal.tool_version),
            json_escape(scorer_sha256()),
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

    /// The covered sources, as `build.rs` frames them: every file under `src/` but `lib.rs`.
    fn scorer_sources() -> Vec<(String, String)> {
        fn walk(dir: &std::path::Path, prefix: &str, out: &mut Vec<(String, String)>) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let entry = entry.unwrap();
                let name = format!("{prefix}{}", entry.file_name().to_string_lossy());
                if entry.file_type().unwrap().is_dir() {
                    walk(&entry.path(), &format!("{name}/"), out);
                } else if entry.path().extension().is_some_and(|ext| ext == "rs")
                    && name != "lib.rs"
                {
                    let text = std::fs::read_to_string(entry.path()).unwrap();
                    out.push((name, text.replace("\r\n", "\n")));
                }
            }
        }
        let mut sources = Vec::new();
        walk(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            "",
            &mut sources,
        );
        sources.sort();
        sources
    }

    fn digest_of(sources: &[(String, String)]) -> String {
        let mut input = Vec::new();
        for (name, text) in sources {
            input.extend_from_slice(format!("{name}\0{}\0", text.len()).as_bytes());
            input.extend_from_slice(text.as_bytes());
        }
        deep_diff_forge_core::sha256_hex(&input)
    }

    /// The build-time digest is the SHA-256 (core's, checked against NIST vectors) of exactly the
    /// covered sources: a file left out, or a wrong digest, fails here.
    #[test]
    fn scorer_identity_is_the_digest_of_every_source_but_lib_rs() {
        let sources = scorer_sources();
        assert!(sources.len() >= 7, "{}", sources.len());
        assert_eq!(
            scorer_sha256(),
            digest_of(&sources),
            "the build-time scorer digest is not the digest of src/ without lib.rs"
        );
    }

    /// Every table and reader that decides a label is in a covered file, and an edit of the line
    /// that defines it moves the identity (the readers too, not only the tables: F2a can change a
    /// reading with no table change).
    #[test]
    fn scorer_identity_moves_with_every_table() {
        let sources = scorer_sources();
        let base = digest_of(&sources);
        for definition in [
            "const RULES: &[Rule]",
            "const LEGACY_RUST_RULES: &[Rule]",
            "const RELEASED_RULES: &[Rule]",
            "const SECRET_PREFIXES: [SecretPrefix;",
            // The byte classes of the secret prefixes.
            "const fn upper_digit(b: u8) -> bool",
            "const fn alnum(b: u8) -> bool",
            "const fn slack(b: u8) -> bool",
            "const fn word(b: u8) -> bool",
            "const fn gitlab(b: u8) -> bool",
            "pub const MAX_EVIDENCE_PER_FILE: usize",
            "fn trivial_assert<",
            "const WORDS: [(&[u8], Word);",
            "const READ: [EvidenceSignal;",
            "pub const LARGE_CHANGE_LINES: usize",
            "pub fn vocabulary() -> Vec<SignalFacts>",
        ] {
            let at = sources
                .iter()
                .position(|(_, text)| text.contains(definition))
                .unwrap_or_else(|| panic!("{definition:?} is in no covered source"));
            let mut edited = sources.clone();
            edited[at].1 = edited[at]
                .1
                .replacen(definition, &format!("{definition} "), 1);
            assert_ne!(
                digest_of(&edited),
                base,
                "{definition:?} does not move the identity"
            );
        }
    }

    #[test]
    fn thresholds_are_exposed() {
        assert_eq!(LARGE_CHANGE_LINES, 80);
        assert_eq!(MANY_HUNKS, 5);
    }
}
