use crate::evidence::{Evidence, MAX_EVIDENCE_PER_FILE};
use crate::risk::{RiskSignal, assess_file};
use deep_diff_forge_core::{FileStatus, ReviewFile};

/// A file placed in the ranked review stream. Built only by [`RankedFile::scored`], so its
/// score, signals and evidence always come from one assessment.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct RankedFile {
    /// File path.
    pub path: String,
    /// File status.
    pub status: FileStatus,
    /// Risk score (higher = review first).
    pub score: u32,
    /// Contributing signals.
    pub signals: Vec<RiskSignal>,
    /// The code evidence behind the [`RiskSignal::Code`] signals, at most
    /// [`MAX_EVIDENCE_PER_FILE`] items, in the order they stand in the file. When more were
    /// found, the list holds first the items the reading every release up to 0.8.3 had finds
    /// ([`Evidence::released`]), as many as fit, and then, in the room that is left, items only
    /// a further reading finds: with 32 or more released items it is exactly the list 0.8.3
    /// printed, and it never holds fewer of them.
    pub evidence: Vec<Evidence>,
    /// More evidence was found than `evidence` holds (the score counts all of it).
    pub evidence_truncated: bool,
}

impl RankedFile {
    /// Assess `file` and place it.
    #[must_use]
    pub fn scored(file: &ReviewFile) -> Self {
        let assessment = assess_file(file);
        let evidence_truncated = assessment.evidence.len() > MAX_EVIDENCE_PER_FILE;
        let evidence = listed(assessment.evidence);
        Self {
            path: file.path.clone(),
            status: file.status,
            score: assessment.score,
            signals: assessment.signals,
            evidence,
            evidence_truncated,
        }
    }
}

/// The items a ranked file lists: all of them when they fit the bound, in the order they stand
/// in the file. Otherwise released items first ([`Evidence::released`]), as many as fit, then
/// items only a further reading finds in the room that is left; the items kept stay in the
/// order of the file. A further reading can add an item to the list but never takes the place
/// of one 0.8.3 listed. (The score and the signals count every item, listed or not.)
fn listed(mut evidence: Vec<Evidence>) -> Vec<Evidence> {
    if evidence.len() <= MAX_EVIDENCE_PER_FILE {
        return evidence;
    }
    let released = evidence.iter().filter(|item| item.released).count();
    let mut room_released = released.min(MAX_EVIDENCE_PER_FILE);
    let mut room_further = MAX_EVIDENCE_PER_FILE - room_released;
    evidence.retain(|item| {
        let room = if item.released {
            &mut room_released
        } else {
            &mut room_further
        };
        let keep = *room > 0;
        *room -= usize::from(keep);
        keep
    });
    evidence
}

/// Rank the review by descending risk score, with a deterministic path
/// tie-break, so the same review always produces the same order.
#[must_use]
pub fn rank(files: &[ReviewFile]) -> Vec<RankedFile> {
    let mut ranked: Vec<RankedFile> = files.iter().map(RankedFile::scored).collect();
    sort_ranked(&mut ranked);
    ranked
}

/// The ranking order: descending score, then path.
pub fn sort_ranked(ranked: &mut [RankedFile]) {
    ranked.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.path.cmp(&b.path)));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file with `n` added lines that each carry one `unsafe` (one evidence item a line).
    fn unsafe_lines(n: usize) -> ReviewFile {
        use std::fmt::Write as _;
        let mut text = format!("--- a/src/x.rs\n+++ b/src/x.rs\n@@ -0,0 +1,{n} @@\n");
        for i in 0..n {
            let _ = writeln!(text, "+unsafe {{ f{i}() }}");
        }
        deep_diff_forge_patch::parse(&text).unwrap().remove(0)
    }

    /// A new Rust file: `first` lines with a fetch behind a nested block comment, which only
    /// the lexer reading finds, `spawns` lines with a process spawn, which every reading
    /// finds, and `last` more lines of the first kind.
    fn hidden_then_plain(first: usize, spawns: usize, last: usize) -> ReviewFile {
        use std::fmt::Write as _;
        let total = first + spawns + last;
        let mut text = format!("--- /dev/null\n+++ b/src/cap.rs\n@@ -0,0 +1,{total} @@\n");
        for i in 0..total {
            let _ = if (first..first + spawns).contains(&i) {
                writeln!(text, "+std::process::Command::new(\"sh\"); // {i}")
            } else {
                writeln!(
                    text,
                    "+/* /* */ // */ let _ = reqwest::blocking::get(u{i});"
                )
            };
        }
        deep_diff_forge_patch::parse(&text).unwrap().remove(0)
    }

    /// At the bound an item only a further reading finds never takes the place of an item 0.8.3
    /// listed. A fetch on line 1 that only the lexer reading finds, then 32 spawns that 0.8.3
    /// finds: 0.8.3 listed the spawns on lines 2 to 33 (measured on the published binary), and
    /// so does this list; the fetch counts in the score and the signals and is not listed.
    /// With room left, the items of the further reading are listed in their places in the file.
    #[test]
    fn at_the_bound_the_items_0_8_3_listed_are_listed() {
        use crate::evidence::EvidenceSignal;
        let lines = |ranked: &RankedFile| -> Vec<u32> {
            ranked.evidence.iter().map(|item| item.line).collect()
        };
        let ranked = RankedFile::scored(&hidden_then_plain(1, 32, 0));
        assert_eq!(lines(&ranked), (2..=33).collect::<Vec<_>>());
        assert!(ranked.evidence.iter().all(|item| item.released));
        assert!(ranked.evidence_truncated);
        for signal in [EvidenceSignal::NetworkFetch, EvidenceSignal::ProcessSpawn] {
            assert!(
                ranked.signals.contains(&RiskSignal::Code(signal)),
                "{signal:?}"
            );
        }
        let without_the_fetch = RankedFile::scored(&hidden_then_plain(0, 32, 0));
        assert!(ranked.score > without_the_fetch.score);
        assert_eq!(
            lines(&without_the_fetch),
            lines(&ranked).iter().map(|l| l - 1).collect::<Vec<_>>()
        );
        // More released items than the bound: the first 32 of them, as 0.8.3 listed.
        let ranked = RankedFile::scored(&hidden_then_plain(2, 40, 3));
        assert_eq!(lines(&ranked), (3..=34).collect::<Vec<_>>());
        // All of it fits: every item, in the order of the file.
        let ranked = RankedFile::scored(&hidden_then_plain(1, 31, 0));
        assert_eq!(lines(&ranked), (1..=32).collect::<Vec<_>>());
        assert!(!ranked.evidence[0].released && !ranked.evidence_truncated);
        // Room for one more: every released item, and the first of the others, in file order.
        let ranked = RankedFile::scored(&hidden_then_plain(1, 31, 2));
        assert_eq!(lines(&ranked), (1..=32).collect::<Vec<_>>());
        assert!(ranked.evidence_truncated);
        assert_eq!(
            ranked.evidence.iter().filter(|item| item.released).count(),
            31
        );
    }

    #[test]
    fn evidence_is_capped_at_exactly_the_limit() {
        for (lines, kept, truncated) in [(31, 31, false), (32, 32, false), (33, 32, true)] {
            let ranked = RankedFile::scored(&unsafe_lines(lines));
            assert_eq!(
                (ranked.evidence.len(), ranked.evidence_truncated),
                (kept, truncated),
                "{lines}"
            );
            // The score counts the signal once, however many lines carry it.
            assert_eq!(ranked.score, RankedFile::scored(&unsafe_lines(1)).score);
        }
    }
    use deep_diff_forge_patch::parse;

    const MULTI: &str = "\
diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,1 +1,1 @@
-a
+b
diff --git a/tests/it.rs b/tests/it.rs
--- a/tests/it.rs
+++ b/tests/it.rs
@@ -1,1 +1,1 @@
-a
+b
diff --git a/src/generated/g.rs b/src/generated/g.rs
--- a/src/generated/g.rs
+++ b/src/generated/g.rs
@@ -1,1 +1,1 @@
-a
+b
";

    fn ranked(input: &str) -> Vec<RankedFile> {
        rank(&parse(input).unwrap())
    }

    #[test]
    fn ranks_public_api_first() {
        let r = ranked(MULTI);
        assert_eq!(r[0].path, "src/lib.rs");
    }

    #[test]
    fn generated_file_ranks_with_tests_below_source() {
        let r = ranked(MULTI);
        let generated = r.iter().find(|f| f.path == "src/generated/g.rs").unwrap();
        let test = r.iter().find(|f| f.path == "tests/it.rs").unwrap();
        assert_eq!((generated.score, test.score), (1, 1));
        assert!(r[0].score > generated.score);
    }

    #[test]
    fn ranking_is_descending_by_score() {
        let r = ranked(MULTI);
        for pair in r.windows(2) {
            assert!(pair[0].score >= pair[1].score);
        }
    }

    #[test]
    fn ranking_is_deterministic() {
        assert_eq!(ranked(MULTI), ranked(MULTI));
    }

    #[test]
    fn tie_break_is_path_ascending() {
        // Two equivalent plain-source files: tie broken by path.
        let input = "\
--- a/src/zeta.rs
+++ b/src/zeta.rs
@@ -1,1 +1,1 @@
-a
+b
--- a/src/alpha.rs
+++ b/src/alpha.rs
@@ -1,1 +1,1 @@
-a
+b
";
        let r = ranked(input);
        assert_eq!(r[0].score, r[1].score);
        assert_eq!(r[0].path, "src/alpha.rs");
        assert_eq!(r[1].path, "src/zeta.rs");
    }

    #[test]
    fn empty_review_ranks_empty() {
        assert_eq!(rank(&[]).len(), 0);
    }

    #[test]
    fn ranked_file_carries_status() {
        let r = ranked("--- a/src/x.rs\n+++ b/src/x.rs\n@@ -1,1 +1,1 @@\n-a\n+b\n");
        assert_eq!(r[0].status, FileStatus::Modified);
    }

    #[test]
    fn ranked_file_carries_signals() {
        let r = ranked("--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@\n-a\n+b\n");
        assert!(r[0].signals.contains(&RiskSignal::PublicApiSurface));
    }

    #[test]
    fn all_files_appear_in_ranking() {
        let r = ranked(MULTI);
        assert_eq!(r.len(), 3);
    }

    #[test]
    fn new_file_outranks_plain_modify_same_path_depth() {
        let input = "\
diff --git a/src/added.rs b/src/added.rs
new file mode 100644
--- /dev/null
+++ b/src/added.rs
@@ -0,0 +1,1 @@
+x
diff --git a/src/edited.rs b/src/edited.rs
--- a/src/edited.rs
+++ b/src/edited.rs
@@ -1,1 +1,1 @@
-a
+b
";
        let r = ranked(input);
        assert_eq!(r[0].path, "src/added.rs");
    }
}
