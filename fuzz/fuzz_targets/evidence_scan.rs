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
// same creation two to five times over in one patch, each file has the hunks that fit in what
// the files before it left, the rank reads at most 256 KiB in the carried reading in all, and
// each file's hunks have the labels its share gives them.
//
// Input: the first byte picks a path (and so a language and rule scope), the rest is the body
// of one hunk; every line becomes a changed or context line, so the scanner sees comments and
// strings opening and closing across lines.

use deep_diff_forge_core::PatchLineKind;
use deep_diff_forge_graph::lexical::{
    Language, RustLexerState, ScanState, code_view, has_token, rust_lexer_view,
};
use deep_diff_forge_graph::{
    CarriedShares, Evidence, EvidenceSignal, HunkLabels, MAX_EVIDENCE_PER_FILE, RankedFile, Side,
    assess_file_in_patch, carried_bytes_on_this_thread, file_evidence, file_evidence_released,
    hunk_signals,
};
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

fuzz_target!(|data: &[u8]| {
    let Some((&pick, rest)) = data.split_first() else {
        return;
    };
    if rest.len() > 64 * 1024 {
        return;
    }
    let path = PATHS[usize::from(pick) % PATHS.len()];
    let text = String::from_utf8_lossy(rest);
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
            let hunk = file
                .patch_twin
                .hunks
                .iter()
                .find(|h| h.id == item.hunk)
                .expect("evidence names a hunk of its file");
            assert_eq!(item.address, hunk.address(&file.path));
            let on_side = hunk.lines.iter().any(|l| match item.side {
                Side::New => l.new_line == Some(item.line),
                Side::Old => l.old_line == Some(item.line),
            });
            assert!(on_side, "evidence line {} is not in its hunk", item.line);
            assert!(hunk_signals(file, hunk).contains(&item.signal));
        }
        for signal in EvidenceSignal::ALL {
            let in_evidence = ranked.evidence.iter().any(|e| e.signal == signal);
            let in_signals = ranked.signals.iter().any(|s| s.label() == signal.label());
            assert!(!in_evidence || in_signals, "{} found but not scored", signal.label());
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
        shared_in_patch(&patch, copies);
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

/// `patch`, one creation, `copies` times over in one patch (each copy at its own path): each
/// file has the hunks, from its first, that fit in what the files before it left of 256 KiB,
/// up to the first with a line over 512 bytes, and takes them when they are two or more; the
/// rank reads at most 256 KiB in the carried reading; every hunk has the labels of its share.
fn shared_in_patch(patch: &str, copies: usize) {
    let mut text = String::new();
    for copy in 0..copies {
        text.push_str(&patch.replace("+++ b/src/cut.rs", &format!("+++ b/src/cut{copy}.rs")));
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
