#![no_main]
// Code evidence (plan 2026-10-30, Feature A): for any patch, ranking never panics, every evidence
// item names a hunk of its own file by address and a line inside that hunk on its side, the
// per-hunk labels agree with the file's evidence, and the cap holds. At the cap the list holds
// first the items the reading every release up to 0.8.3 had finds, in their order, so an item
// only a further reading finds never takes the place of one that 0.8.3 listed.
//
// And no answer is lower than 0.8.3's: in a Rust file, a rule token that the reading of 0.8.3
// shows on a changed line is evidence, whatever the lexer reading makes of that line. The
// reading of 0.8.3 is `lexical::code_view`, followed down each side of a hunk as the scan
// follows it; three token rules are derived here again from that view alone.
//
// And the hunks of a creation are one text: the same lines as a Rust file the patch creates, cut
// into hunks at lines the input chooses, have as evidence every `unsafe` and `Command::new`
// that the lexer reading (`lexical::rust_lexer_view`) shows when it reads the lines as one
// file from its first, whatever a hunk read alone would show, in the hunks read together: from
// the first, up to 256 KiB of the new side with no line longer than 512 bytes.
//
// And the created files of a patch share one carried reading of 256 KiB in patch order: the
// same creation two to five times over in one patch, at paths taken from `PATHS` in turn from
// the one the first byte picks (so in several languages, Rust and JavaScript among them), each
// file has the hunks that fit in what the files before it left, the rank reads at most 256 KiB
// in the carried reading in all, and each file's hunks have the labels its share gives them.
// The copies can pass the bound only when their new sides, after the input's lossy decoding,
// add up to more than 256 KiB, and only through copies in a language (a copy has at most
// about 64 KiB of valid UTF-8, so four valid copies pass it only at that size, five at less;
// two can when invalid bytes are decoded to three bytes each). Two seeds
// pass it from the first run (`shared_bound_py`, `shared_bound_ts`: Python, TypeScript, Go,
// shell, YAML and Rust in turn, made so that four copies fit and the fifth is cut part-way): a
// budget not charged for any one of Python, JavaScript, Go, shell or YAML fails on them.
//
//
// And the same lines as hunks of three lines with unseen lines between them (eighth independent
// review, IR8-M-2: nothing but the views called `hunk_signals_alone`, and no target held what
// a view of hunks by path costs). A hunk that is read alone has, from the reader of one hunk,
// the labels the scan of its file gives it, and that reader makes no pass over the file. The
// view of the file's hunks by path shows for each hunk the labels the outline shows, and goes
// through every hunk of the file a fixed number of times (two), however many hunks there are.
// And a scan holds one reading of one text at a time: the lines of the readings alive at one
// moment are never more than the lines of the file.
//
// Input: the first byte picks a path (and so a language and rule scope), the rest is the body
// of one hunk; every line becomes a changed or context line, so the scanner sees comments and
// strings opening and closing across lines.

use deep_diff_forge_core::PatchLineKind;
use deep_diff_forge_daemon::views::{MAX_MAX_BYTES, Page, Selection, hunks, outline};
use deep_diff_forge_graph::lexical::{
    Language, RustLexerState, ScanState, code_view, has_token, rust_lexer_view,
};
use deep_diff_forge_graph::{
    CarriedShares, Evidence, EvidenceSignal, FileKinds, HunkLabels, MAX_EVIDENCE_PER_FILE,
    RankedFile, Side, assess_file_in_patch, carried_bytes_on_this_thread, file_evidence,
    file_evidence_released, file_walks, hunk_signals, hunk_signals_alone, joined_hunks,
    view_lines_peak,
};
use deep_diff_forge_patch::{InputGuards, SealedReview};
use libfuzzer_sys::fuzz_target;
use std::fmt::Write as _;

const PATHS: &[&str] = &[
    "src/lib.rs",
    "x.py",
    "x.ts",
    "x.go",
    "x.sh",
    ".github/workflows/ci.yml",
    "build.rs",
    "package.json",
    "src/generated/x.rs",
    "tests/it.rs",
    "bin/tool",
    "notes.txt",
];

/// The lines as hunks of three (an added line, a removed line, a line of context), ten lines
/// apart: read one at a time and through the views.
fn hunks_alone_and_by_path(name: &str, text: &str) {
    let mut patch = format!("--- a/{name}\n+++ b/{name}\n");
    let lines: Vec<String> = text
        .lines()
        .map(|line| line.chars().filter(|c| *c != '\r').collect())
        .collect();
    for (k, rows) in lines.chunks(3).enumerate() {
        let (old, new) = (rows.len() - 1, 1 + rows.len() / 3);
        let at = 1 + 10 * k;
        let _ = writeln!(patch, "@@ -{at},{old} +{at},{new} @@");
        for (sign, line) in ['+', '-', ' '].into_iter().zip(rows) {
            patch.push(sign);
            patch.push_str(line);
            patch.push('\n');
        }
    }
    let guards = InputGuards {
        files: false,
        hunks: false,
    };
    let Ok(review) = SealedReview::new(patch.as_bytes(), guards, "0.0.0-fuzz") else {
        return;
    };
    for file in review.files() {
        // what a scan holds: one reading of one text at a time, so never more lines than
        // the file has, however many texts its hunks stand in (ninth independent review,
        // IR9-L-6: nothing held a memory figure)
        let _ = view_lines_peak();
        let evidence = file_evidence(file);
        let lines: usize = file.patch_twin.hunks.iter().map(|h| h.lines.len()).sum();
        let held = view_lines_peak();
        assert!(
            held <= lines as u64,
            "a scan of {lines} lines held {held} lines of readings at one time"
        );
        let joined = joined_hunks(file);
        let kinds = FileKinds::of(file);
        let before = file_walks();
        for (index, hunk) in file.patch_twin.hunks.iter().enumerate() {
            if joined[index] {
                continue;
            }
            let scanned: Vec<EvidenceSignal> = EvidenceSignal::ALL
                .into_iter()
                .filter(|signal| {
                    evidence
                        .iter()
                        .any(|item| item.hunk == hunk.id && item.signal == *signal)
                })
                .collect();
            assert_eq!(
                hunk_signals_alone(file, index, &kinds),
                Some(scanned),
                "a hunk read alone differs from the scan of its file"
            );
        }
        assert_eq!(
            file_walks(),
            before,
            "reading a hunk alone went through its file"
        );
    }
    if review.files().len() != 1 {
        return;
    }
    let page = Page {
        max_bytes: MAX_MAX_BYTES,
        cursor: None,
    };
    let Ok(listed) = outline(&review, &page) else {
        return;
    };
    let file = &review.files()[0];
    let before = file_walks();
    let Ok(shown) = hunks(&review, &Selection::Paths(vec![file.path.clone()]), &page) else {
        return;
    };
    assert!(
        file_walks() - before <= 2,
        "a view of {} hunks by path went through the file {} times",
        file.patch_twin.hunks.len(),
        file_walks() - before
    );
    let (Some(listed), Some(shown)) = (
        listed["files"][0]["hunks"].as_array(),
        shown["hunks"].as_array(),
    ) else {
        return;
    };
    // a page holds what fits: the hunks shown are the first of the file, in its order
    for (by_path, in_outline) in shown.iter().zip(listed) {
        assert_eq!(by_path["address"], in_outline["address"]);
        assert_eq!(
            by_path["evidence"], in_outline["evidence"],
            "the labels of a hunk by path are not the outline's"
        );
    }
}

fuzz_target!(|data: &[u8]| {
    let Some((&pick, rest)) = data.split_first() else {
        return;
    };
    if rest.len() > 64 * 1024 {
        return;
    }
    let path = PATHS[usize::from(pick) % PATHS.len()];
    let text = String::from_utf8_lossy(rest);
    hunks_alone_and_by_path(path, &text);
    let mut body = String::new();
    let (mut old, mut new) = (0u32, 0u32);
    for (i, line) in text.lines().enumerate() {
        let line: String = line.chars().filter(|c| *c != '\r').collect();
        let sign = match i % 3 {
            0 => '+',
            1 => '-',
            _ => ' ',
        };
        body.push(sign);
        body.push_str(&line);
        body.push('\n');
        if sign != '+' {
            old += 1;
        }
        if sign != '-' {
            new += 1;
        }
    }
    if old == 0 && new == 0 {
        return;
    }
    one_text_in_hunks(pick, &text);
    let patch = format!("--- a/{path}\n+++ b/{path}\n@@ -1,{old} +1,{new} @@\n{body}");
    let Ok(files) = deep_diff_forge_patch::parse(&patch) else {
        return;
    };
    for file in &files {
        let ranked = RankedFile::scored(file);
        assert!(ranked.evidence.len() <= MAX_EVIDENCE_PER_FILE);
        let all = file_evidence(file);
        assert_eq!(ranked.evidence.len(), all.len().min(MAX_EVIDENCE_PER_FILE));
        // Which items the reading of 0.8.3 finds is said beside the list, item for item.
        let marks = file_evidence_released(file);
        assert_eq!(marks.len(), all.len());
        let of_0_8_3: Vec<&Evidence> =
            all.iter().zip(&marks).filter(|(_, m)| **m).map(|(e, _)| e).collect();
        let released = || of_0_8_3.iter().copied();
        let listed = || ranked.evidence.iter().filter(|e| of_0_8_3.contains(e));
        assert_eq!(listed().count(), released().count().min(MAX_EVIDENCE_PER_FILE));
        assert!(
            listed().zip(released()).all(|(listed, found)| listed == found),
            "the released items listed are not the first of the file"
        );
        for item in &ranked.evidence {
            let (index, hunk) = file
                .patch_twin
                .hunks
                .iter()
                .enumerate()
                .find(|(_, h)| h.id == item.hunk)
                .expect("evidence names a hunk of its file");
            assert_eq!(item.address, hunk.address(&file.path));
            let on_side = hunk.lines.iter().any(|l| match item.side {
                Side::New => l.new_line == Some(item.line),
                Side::Old => l.old_line == Some(item.line),
            });
            assert!(on_side, "evidence line {} is not in its hunk", item.line);
            assert!(hunk_signals(file, index).is_some_and(|found| found.contains(&item.signal)));
        }
        for signal in EvidenceSignal::ALL {
            let in_evidence = ranked.evidence.iter().any(|e| e.signal == signal);
            let in_signals = ranked.signals.iter().any(|s| s.label() == signal.label());
            assert!(
                !in_evidence || in_signals,
                "{} found but not scored",
                signal.label()
            );
        }
        if Language::of(path, None) == Some(Language::Rust) {
            released_reading_is_found(file);
        }
    }
});

/// The lines of `text` as one stretch of a Rust file, cut into hunks of one to four lines
/// (`pick` and the line lengths choose): every item the lexer reading has for the whole text
/// is evidence of the patch, which creates the file (the hunks of a file the patch creates
/// are read as one text, up to the bound; the hunks of a file that exists are read each on its
/// own). The bound is where it is said to be: within 256 KiB, the hunks before the first with a
/// line longer than 512 bytes are read together.
fn one_text_in_hunks(pick: u8, text: &str) {
    let lines: Vec<String> = text
        .lines()
        .map(|line| line.chars().filter(|c| *c != '\r').collect())
        .collect();
    if lines.is_empty() {
        return;
    }
    let mut patch = String::from("--- /dev/null\n+++ b/src/cut.rs\n");
    let mut at = 0;
    while at < lines.len() {
        let take = (1 + (usize::from(pick) + lines[at].len()) % 4).min(lines.len() - at);
        let _ = writeln!(patch, "@@ -{at},0 +{},{take} @@", at + 1);
        for line in &lines[at..at + take] {
            patch.push('+');
            patch.push_str(line);
            patch.push('\n');
        }
        at += take;
    }
    let Ok(files) = deep_diff_forge_patch::parse(&patch) else {
        return;
    };
    let [file] = files.as_slice() else {
        return;
    };
    let hunks = &file.patch_twin.hunks;
    let bytes: usize = hunks
        .iter()
        .flat_map(|hunk| &hunk.lines)
        .map(|line| line.text.len() + 1)
        .sum();
    let first_long = hunks
        .iter()
        .position(|hunk| hunk.lines.iter().any(|line| line.text.len() > 512))
        .unwrap_or(hunks.len());
    let together = HunkLabels::new(file).read_together();
    if bytes <= 256 * 1024 {
        assert_eq!(together, if first_long > 1 { first_long } else { 0 });
    }
    let copies = 1 + usize::from(pick >> 4) % 5;
    if copies > 1 {
        shared_in_patch(pick, &patch, copies);
    }
    // A hunk past the bound is read alone; the first is read from the top of the file either way.
    let evidence = file_evidence(file);
    let mut state = RustLexerState::default();
    let added = hunks.iter().take(together.max(1)).flat_map(|hunk| &hunk.lines);
    for line in added {
        let view = rust_lexer_view(&line.text, &mut state);
        let Some(number) = line.new_line else {
            continue;
        };
        for (token, signal) in [
            ("unsafe", EvidenceSignal::UnsafeAdded),
            ("Command::new", EvidenceSignal::ProcessSpawn),
        ] {
            assert!(
                !has_token(&view, token)
                    || evidence
                        .iter()
                        .any(|e| e.signal == signal && e.side == Side::New && e.line == number),
                "read as one text, line {number} has `{token}`; cut into hunks it is not evidence: \
                 {:?}",
                line.text
            );
        }
    }
}

/// A rule token and the signal it is evidence of.
type Rules = &'static [(&'static str, EvidenceSignal)];

/// What three Rust token rules find in the reading of 0.8.3 alone is in the file's evidence.
fn released_reading_is_found(file: &deep_diff_forge_core::ReviewFile) {
    let evidence = file_evidence(file);
    for hunk in &file.patch_twin.hunks {
        let (mut old, mut new) = (ScanState::default(), ScanState::default());
        for line in &hunk.lines {
            let old_view = (line.kind != PatchLineKind::Added)
                .then(|| code_view(Language::Rust, &line.text, &mut old));
            let new_view = (line.kind != PatchLineKind::Removed)
                .then(|| code_view(Language::Rust, &line.text, &mut new));
            let (side, number, view, rules): (_, _, _, Rules) = match line.kind {
                PatchLineKind::Context => continue,
                PatchLineKind::Added => (
                    Side::New,
                    line.new_line,
                    new_view,
                    &[
                        ("unsafe", EvidenceSignal::UnsafeAdded),
                        ("Command::new", EvidenceSignal::ProcessSpawn),
                    ],
                ),
                PatchLineKind::Removed => (
                    Side::Old,
                    line.old_line,
                    old_view,
                    &[("#[test]", EvidenceSignal::TestRemoved)],
                ),
            };
            let (Some(number), Some(view)) = (number, view) else {
                continue;
            };
            for (token, signal) in rules {
                assert!(
                    !has_token(&view, token)
                        || evidence.iter().any(|e| {
                            e.hunk == hunk.id
                                && e.signal == *signal
                                && e.side == side
                                && e.line == number
                        }),
                    "0.8.3 reads `{token}` on line {number} ({side:?}) and it is not evidence: {:?}",
                    line.text
                );
            }
        }
    }
}

/// `patch`, one creation, `copies` times over in one patch, copy `k` at `c{k}/` and the path
/// of `PATHS` `k` places after the one `pick` picks (so in the languages of those paths): each
/// file has the hunks, from its first, that fit in what the files before it left of 256 KiB,
/// up to the first with a line over 512 bytes, and takes them when they are two or more and
/// the file is read as one text at all; the rank reads at most 256 KiB in the carried reading;
/// every hunk has the labels of its share.
fn shared_in_patch(pick: u8, patch: &str, copies: usize) {
    let mut text = String::new();
    for copy in 0..copies {
        let at = PATHS[(usize::from(pick) + copy) % PATHS.len()];
        text.push_str(&patch.replace("+++ b/src/cut.rs", &format!("+++ b/c{copy}/{at}")));
    }
    let Ok(files) = deep_diff_forge_patch::parse(&text) else {
        return;
    };
    let shares = CarriedShares::of(&files);
    let mut left = 256 * 1024;
    for (index, file) in files.iter().enumerate() {
        let (mut fit, mut bytes) = (0, 0);
        'hunks: for hunk in &file.patch_twin.hunks {
            let mut more = 0;
            for line in &hunk.lines {
                more += line.text.len() + 1;
                if line.text.len() > 512 || bytes + more > left {
                    break 'hunks;
                }
            }
            fit += 1;
            bytes += more;
        }
        let own = HunkLabels::new(file).read_together();
        let share = shares.read_together(index);
        if own == 0 {
            assert_eq!(share, 0, "file {index}");
            continue;
        }
        if fit >= 2 {
            left -= bytes;
            assert_eq!(share, fit, "file {index}");
        } else {
            assert_eq!(share, 0, "file {index}");
        }
    }
    let before = carried_bytes_on_this_thread();
    let _ = deep_diff_forge_graph::rank(&files);
    assert!(carried_bytes_on_this_thread() - before <= 256 * 1024);
    for (index, file) in files.iter().enumerate() {
        let mut found = std::collections::HashMap::new();
        for item in assess_file_in_patch(file, &shares, index).evidence {
            found
                .entry(item.hunk)
                .or_insert_with(Vec::new)
                .push(item.signal);
        }
        let mut labels = HunkLabels::in_patch(file, &shares, index, &[]);
        for (h, hunk) in file.patch_twin.hunks.iter().enumerate() {
            let of_evidence: Vec<EvidenceSignal> = EvidenceSignal::ALL
                .into_iter()
                .filter(|signal| found.get(&hunk.id).is_some_and(|s| s.contains(signal)))
                .collect();
            assert_eq!(labels.of(h), of_evidence, "file {index} hunk {h}");
        }
    }
}
