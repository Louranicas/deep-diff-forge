use crate::evidence::{Evidence, EvidenceSignal, MAX_EVIDENCE_PER_FILE};
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
    /// [`MAX_EVIDENCE_PER_FILE`] items, in the file's order (hunk, line, signal). When the
    /// file has more, these are first what release 0.8.3 listed ([`Evidence::released`]),
    /// then an item for every signal that has none yet, then the others as they come (the
    /// function `listed` of this module says why).
    pub evidence: Vec<Evidence>,
    /// More evidence was found than `evidence` holds (the score counts all of it).
    pub evidence_truncated: bool,
}

impl RankedFile {
    /// Assess `file` and place it. The answer is the file's own, whatever else is in the
    /// review.
    #[must_use]
    pub fn scored(file: &ReviewFile) -> Self {
        let assessment = assess_file(file);
        let (evidence, evidence_truncated) = listed(assessment.evidence);
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

/// The items of a file that its bounded list shows, and whether some are left out.
/// `evidence` is every item of the file, in the file's order (hunk, line, signal), and the
/// answer is in that order too. The score and the signals of a file come from every item;
/// only the list is bounded.
///
/// A file with more than [`MAX_EVIDENCE_PER_FILE`] items shows that many. They are taken in
/// three steps, each while there is room:
///
/// 1. The items release 0.8.3 reported ([`Evidence::released`]), in the order it listed
///    them. That release printed the first 32 of its items, so every item it printed is
///    printed. (Until 2026-11-15 the list was the first 32 of all items, and an item only
///    this version finds, on an earlier line, pushed out one that release showed: on this
///    repository's history a file that spawns processes at five places listed four of them
///    after the panic paths of its tests took the rest of the room. Review of the same
///    change on the released line, VLF3-L-1.)
/// 2. For each signal of the file with no item in the list yet, its first item, the heavier
///    signal first ([`EvidenceSignal::weight`], then the order of [`EvidenceSignal::ALL`]).
///    A reader who is told a file carries a signal is shown a place to look for it: forty
///    panic paths do not leave out the one stub at the end of the file.
/// 3. The other items as they come.
///
/// Step 2 is not "the heavier signal's items first": the weights are close (most are 3),
/// one signal's items would take all the room, and a signal with one item would still have
/// no place in the list.
fn listed(mut evidence: Vec<Evidence>) -> (Vec<Evidence>, bool) {
    if evidence.len() <= MAX_EVIDENCE_PER_FILE {
        return (evidence, false);
    }
    let mut keep = vec![false; evidence.len()];
    let mut room = MAX_EVIDENCE_PER_FILE;
    let mut take = |keep: &mut [bool], at: usize| {
        if room > 0 && !keep[at] {
            keep[at] = true;
            room -= 1;
        }
    };
    for (at, item) in evidence.iter().enumerate() {
        if item.released {
            take(&mut keep, at);
        }
    }
    let mut unplaced: Vec<EvidenceSignal> = EvidenceSignal::ALL
        .into_iter()
        .filter(|signal| {
            !evidence
                .iter()
                .zip(&keep)
                .any(|(item, kept)| *kept && item.signal == *signal)
        })
        .collect();
    // A stable sort: signals of one weight stay in the order of `ALL`.
    unplaced.sort_by_key(|signal| std::cmp::Reverse(signal.weight()));
    for signal in unplaced {
        if let Some(at) = evidence.iter().position(|item| item.signal == signal) {
            take(&mut keep, at);
        }
    }
    for at in 0..evidence.len() {
        take(&mut keep, at);
    }
    let mut keep = keep.into_iter();
    evidence.retain(|_| keep.next() == Some(true));
    (evidence, true)
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

    /// A file the patch creates with these lines.
    fn created(lines: &[String]) -> ReviewFile {
        use std::fmt::Write as _;
        let mut text = format!(
            "diff --git a/src/x.rs b/src/x.rs\nnew file mode 100644\n--- /dev/null\n+++ b/src/x.rs\n@@ -0,0 +1,{} @@\n",
            lines.len()
        );
        for line in lines {
            let _ = writeln!(text, "+{line}");
        }
        deep_diff_forge_patch::parse(&text).unwrap().remove(0)
    }

    fn listed_items(file: &ReviewFile) -> Vec<(EvidenceSignal, u32)> {
        RankedFile::scored(file)
            .evidence
            .iter()
            .map(|item| (item.signal, item.line))
            .collect()
    }

    /// An item release 0.8.3 did not report, on line 1, and 32 it listed below it: all 32
    /// are listed. The one above is in the signals and the score, and the list says it is
    /// cut short. Two such items: a signal that release did not have, and a signal it had
    /// in code its scanner took for a comment (review of the released line's fix, VLF3-L-1,
    /// `vesper-lexfix3-cap.patch`; until 2026-11-15 the list was line 1 and 31 spawns).
    #[test]
    fn what_release_0_8_3_listed_is_not_pushed_out_of_the_list() {
        use EvidenceSignal as S;
        for (first, signal) in [
            ("x.unwrap();", S::PanicPathAdded),
            ("/* /* */ // */ reqwest::blocking::get(u);", S::NetworkFetch),
        ] {
            let mut lines = vec![first.to_owned()];
            lines.extend((0..32).map(|i| format!("std::process::Command::new(\"p{i}\");")));
            let file = created(&lines);
            let spawns: Vec<(S, u32)> = (2..=33).map(|line| (S::ProcessSpawn, line)).collect();
            assert_eq!(listed_items(&file), spawns, "{first}");
            let ranked = RankedFile::scored(&file);
            assert!(ranked.evidence_truncated, "{first}");
            assert!(
                ranked.signals.contains(&RiskSignal::Code(signal)),
                "{first}"
            );
            assert_eq!(
                ranked.score,
                RankedFile::scored(&created(&lines[1..])).score + signal.weight(),
                "{first}"
            );
        }
    }

    /// Forty panic paths and one stub on the last line: the list shows the stub. Before
    /// the first item of a signal with no place yet, nothing else of the new items is taken;
    /// an item release 0.8.3 listed comes before both. The list stays in the file's order.
    #[test]
    fn every_signal_of_a_file_has_a_place_in_its_list() {
        use EvidenceSignal as S;
        let mut lines: Vec<String> = (0..40).map(|i| format!("a{i}.unwrap();")).collect();
        lines.push("todo!();".to_owned());
        let mut expected: Vec<(S, u32)> = (1..=31).map(|line| (S::PanicPathAdded, line)).collect();
        expected.push((S::StubAdded, 41));
        assert_eq!(listed_items(&created(&lines)), expected);
        // and with an item of the release below both
        lines.push("unsafe { f() }".to_owned());
        expected.remove(30);
        expected.push((S::UnsafeAdded, 42));
        assert_eq!(listed_items(&created(&lines)), expected);
    }

    /// When the signals with no place are more than the room, the heavier comes first, not
    /// the earlier in the file or in the list of signals: 31 items of the release leave one
    /// place, and it goes to the stub on the last line (weight 3), not to the suppressed
    /// lint on line 1 (weight 2; spelled with a space, which that release did not read).
    #[test]
    fn the_heavier_signal_takes_the_last_place() {
        use EvidenceSignal as S;
        let mut lines = vec!["# [allow(dead_code)]".to_owned()];
        lines.extend((0..31).map(|i| format!("std::process::Command::new(\"p{i}\");")));
        lines.push("todo!();".to_owned());
        let file = created(&lines);
        let ranked = RankedFile::scored(&file);
        assert!(
            ranked
                .signals
                .contains(&RiskSignal::Code(S::CheckSuppressed))
        );
        let listed = listed_items(&file);
        assert_eq!(listed.len(), 32);
        assert_eq!(listed[0], (S::ProcessSpawn, 2));
        assert_eq!(listed[31], (S::StubAdded, 33));
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
