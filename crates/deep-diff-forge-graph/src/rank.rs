use crate::evidence::{CarriedShares, Evidence, EvidenceSignal, MAX_EVIDENCE_PER_FILE, Share};
use crate::risk::{RiskSignal, assess_file_marked};
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
    /// file has more, these are first what release 0.8.3 listed ([`Evidence::released`]: the
    /// items the reading every release up to 0.8.3 had finds, as many as fit, so that with 32
    /// or more of them it is exactly the list 0.8.3 printed, and it never holds fewer of them),
    /// then an item for every signal that has none yet, then the others as they come (the
    /// function `listed` of this module says why).
    pub evidence: Vec<Evidence>,
    /// More evidence was found than `evidence` holds (the score counts all of it).
    pub evidence_truncated: bool,
}

impl RankedFile {
    /// Assess `file`, read as the only file of its patch, and place it. [`rank`] places each
    /// file of a patch read with its share of the patch's carried reading
    /// ([`Self::scored_in_patch`]); that share is the one thing in the answer that depends on
    /// what else the patch holds.
    #[must_use]
    pub fn scored(file: &ReviewFile) -> Self {
        Self::sharing(file, Share::Own)
    }

    /// Not part of the API, and may change: assess `file` read with its share of its patch's
    /// carried reading and place it, `file` being the file at `index` of the files `shares`
    /// was made from. [`rank`] and the cluster's lanes place each file so.
    #[doc(hidden)]
    #[must_use]
    pub fn scored_in_patch(file: &ReviewFile, shares: &CarriedShares, index: usize) -> Self {
        Self::sharing(file, shares.share(index))
    }

    fn sharing(file: &ReviewFile, share: Share) -> Self {
        let (assessment, _) = assess_file_marked(file, share);
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
///
/// `files` are the files of one patch, all of them, in patch order: the files it creates in
/// several hunks share one carried reading in that order ([`CarriedShares`]).
#[must_use]
pub fn rank(files: &[ReviewFile]) -> Vec<RankedFile> {
    let shares = CarriedShares::of(files);
    let mut ranked: Vec<RankedFile> = files
        .iter()
        .enumerate()
        .map(|(index, file)| RankedFile::scored_in_patch(file, &shares, index))
        .collect();
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
        // Of the items listed, which the reading of 0.8.3 finds.
        let released = |file: &ReviewFile, ranked: &RankedFile| -> Vec<bool> {
            let (all, marks) =
                crate::evidence::file_evidence_marked(file, crate::evidence::Share::Own);
            let mark = |item| {
                all.iter()
                    .position(|found| found == item)
                    .map(|at| marks[at])
            };
            ranked
                .evidence
                .iter()
                .map(|item| mark(item).unwrap())
                .collect()
        };
        let file = hidden_then_plain(1, 32, 0);
        let ranked = RankedFile::scored(&file);
        assert_eq!(lines(&ranked), (2..=33).collect::<Vec<_>>());
        assert!(released(&file, &ranked).iter().all(|released| *released));
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
        let file = hidden_then_plain(1, 31, 0);
        let ranked = RankedFile::scored(&file);
        assert_eq!(lines(&ranked), (1..=32).collect::<Vec<_>>());
        assert!(!released(&file, &ranked)[0] && !ranked.evidence_truncated);
        // Room for one more: every released item, and the first of the others, in file order.
        let file = hidden_then_plain(1, 31, 2);
        let ranked = RankedFile::scored(&file);
        assert_eq!(lines(&ranked), (1..=32).collect::<Vec<_>>());
        assert!(ranked.evidence_truncated);
        let marks = released(&file, &ranked);
        assert_eq!(marks.iter().filter(|released| **released).count(), 31);
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
    /// The list of 32 keeps release 0.8.3's items, and for that it gives up items that an
    /// earlier round of this branch listed: it is not a superset of that round's list
    /// (round 14 of Vesper's review, VD3R14-M-1, on this repository's history:
    /// `tests/repo_scripts.rs` in commit `d239165`, where round 13 listed the panic path on
    /// line 330 and this list has the spawn on line 368 in its place, which release 0.8.3
    /// listed). The same shape here: 33 panic paths and then a spawn. The first 32 of all
    /// items, which was round 13's list, are the panic paths of lines 1 to 32. This list has
    /// the spawn, and the panic path of line 32 is the item that made room. Nothing else is
    /// lost by it: the signals and the score come from every item and are the same.
    #[test]
    fn the_list_gives_up_a_later_rounds_item_to_keep_the_releases() {
        use EvidenceSignal as S;
        let mut lines: Vec<String> = (0..33).map(|i| format!("a{i}.unwrap();")).collect();
        lines.push("std::process::Command::new(\"sh\");".to_owned());
        let file = created(&lines);
        let first_32: Vec<(S, u32)> = (1..=32).map(|line| (S::PanicPathAdded, line)).collect();
        let listed = listed_items(&file);
        let mut expected = first_32.clone();
        expected[31] = (S::ProcessSpawn, 34);
        assert_eq!(listed, expected);
        assert!(!listed.contains(&first_32[31]));
        let ranked = RankedFile::scored(&file);
        assert!(ranked.evidence_truncated);
        assert!(
            ranked
                .signals
                .contains(&RiskSignal::Code(S::PanicPathAdded))
        );
        assert!(ranked.signals.contains(&RiskSignal::Code(S::ProcessSpawn)));
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

    /// The six forms of one change to `src/{name}.rs` whose first lines are `first` and whose
    /// last lines are `second`: as one section, and cut between the two into the several
    /// sections of one path that the parser reads (for
    /// [`the_sections_of_one_path_git_applies_in_order_are_one_file`]).
    fn one_path_shapes(name: &str, first: &[&str], second: &[&str]) -> Vec<(&'static str, String)> {
        use std::fmt::Write as _;
        let plus = |lines: &[&str]| -> String {
            lines.iter().fold(String::new(), |mut text, line| {
                let _ = writeln!(text, "+{line}");
                text
            })
        };
        let p = format!("src/{name}.rs");
        let (a, b) = (first.len(), second.len());
        let create = |lines: &[&str]| {
            format!(
                "diff --git a/{p} b/{p}\nnew file mode 100644\n--- /dev/null\n+++ b/{p}\n@@ -0,0 +1,{} @@\n{}",
                lines.len(),
                plus(lines)
            )
        };
        let change = format!("diff --git a/{p} b/{p}\n--- a/{p}\n+++ b/{p}\n");
        let delete = format!(
            "diff --git a/{p} b/{p}\ndeleted file mode 100644\n--- a/{p}\n+++ /dev/null\n@@ -1 +0,0 @@\n-// old\n"
        );
        let whole: Vec<&str> = first.iter().chain(second).copied().collect();
        vec![
            ("one section", create(&whole)),
            (
                "create then change",
                format!(
                    "{}{change}@@ -{a},0 +{},{b} @@\n{}",
                    create(first),
                    a + 1,
                    plus(second)
                ),
            ),
            (
                "create then change, other numbers",
                format!(
                    "{}{change}@@ -9,0 +3,{b} @@\n{}",
                    create(first),
                    plus(second)
                ),
            ),
            (
                "change then change",
                format!(
                    "{change}@@ -1 +1,{} @@\n // old\n{}{change}@@ -{},0 +{},{b} @@\n{}",
                    1 + a,
                    plus(first),
                    1 + a,
                    2 + a,
                    plus(second)
                ),
            ),
            ("delete then create", format!("{delete}{}", create(&whole))),
            (
                "delete, create, change",
                format!(
                    "{delete}{}{change}@@ -{a},0 +{},{b} @@\n{}",
                    create(first),
                    a + 1,
                    plus(second)
                ),
            ),
        ]
    }

    /// IR7-H-2 in the shapes of one path in several sections that the parser reads (a change
    /// in place after a creation, after a change, and after a deletion and a creation): plain
    /// `git apply` 2.56.0 writes the text of the one-section form in each, and the sections
    /// are read as one file too, so each finds what the one-section form finds, on the section
    /// that holds the line. `n`: a call cut as `opt.unwrap` and `();`; `two`: a spawn after
    /// the end of a string that the first section opens. (Until this rule none of the
    /// several-section forms had the item, in any release.)
    #[test]
    fn the_sections_of_one_path_git_applies_in_order_are_one_file() {
        use crate::evidence::EvidenceSignal as S;
        for (name, first, second, signal, at) in [
            (
                "n",
                &[
                    "pub fn f(opt: Option<u32>) -> u32 {",
                    "    let v = opt.unwrap",
                ][..],
                &["    ();", "    v", "}"][..],
                S::PanicPathAdded,
                0,
            ),
            (
                "two",
                &["pub fn help() -> &'static str {", "    let s = \"usage:"][..],
                &[
                    "  --help\"; std::process::Command::new(\"sh\").status().ok(); // \"",
                    "    s }",
                ][..],
                S::ProcessSpawn,
                1,
            ),
        ] {
            for (shape, text) in one_path_shapes(name, first, second) {
                // 0.10.0: the parser refuses the later sections (no context); read as built.
                let files = crate::unplaced::unplaced(&text);
                let ranked = rank(&files);
                let found: Vec<&RankedFile> = ranked
                    .iter()
                    .filter(|r| r.signals.contains(&RiskSignal::Code(signal)))
                    .collect();
                assert_eq!(found.len(), 1, "{name}, {shape}: {ranked:#?}");
                // on the section that holds the line: the first lines are the first section's
                let item = found[0]
                    .evidence
                    .iter()
                    .find(|e| e.signal == signal)
                    .unwrap();
                let holder = files
                    .iter()
                    .find(|f| f.patch_twin.hunks.iter().any(|h| h.id == item.hunk))
                    .unwrap();
                let text_of: Vec<&str> = holder
                    .patch_twin
                    .hunks
                    .iter()
                    .flat_map(|h| h.lines.iter())
                    .map(|l| l.text.as_str())
                    .collect();
                let wanted = if at == 0 {
                    first.last()
                } else {
                    second.first()
                };
                assert!(text_of.contains(wanted.unwrap()), "{name}, {shape}");
            }
        }
    }

    /// A run of sections of one path is read as one file only within the patch's budget: with a
    /// line longer than 512 bytes in it, its sections are read as before, apart.
    #[test]
    fn sections_of_one_path_past_the_budget_are_read_apart() {
        let long = "x".repeat(600);
        let text = format!(
            "diff --git a/src/n.rs b/src/n.rs\nnew file mode 100644\n--- /dev/null\n+++ b/src/n.rs\n@@ -0,0 +1,2 @@\n+// {long}\n+    let v = opt.unwrap\ndiff --git a/src/n.rs b/src/n.rs\n--- a/src/n.rs\n+++ b/src/n.rs\n@@ -2,0 +3,1 @@\n+    ();\n"
        );
        // 0.10.0: the parser refuses the second section (no context); read as built.
        let files = crate::unplaced::unplaced(&text);
        let shares = CarriedShares::of(&files);
        assert!(!shares.joined_across_sections(0) && !shares.joined_across_sections(1));
        assert!(rank(&files).iter().all(|r| r.evidence.is_empty()));
        let short = text.replace(&long, "short");
        let files = crate::unplaced::unplaced(&short);
        let shares = CarriedShares::of(&files);
        assert!(shares.joined_across_sections(0) && shares.joined_across_sections(1));
    }
}
