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
// And hunks that touch are one text: the same lines as a Rust file the patch creates, cut
// into hunks at lines the input chooses, have as evidence every `unsafe` and `Command::new`
// that the lexer reading (`lexical::rust_lexer_view`) shows when it reads the lines as one
// file from its first, whatever a hunk read alone would show. The same holds at the end of a
// file that exists: a hunk that shows a line and adds lines after it, then hunks of added
// lines only whose old positions are any numbers above 1 (plain `git apply` appends each).
//
// Input: the first byte picks a path (and so a language and rule scope), the rest is the body
// of one hunk; every line becomes a changed or context line, so the scanner sees comments and
// strings opening and closing across lines.

use deep_diff_forge_core::PatchLineKind;
use deep_diff_forge_graph::lexical::{
    Language, RustLexerState, ScanState, code_view, has_token, rust_lexer_view,
};
use deep_diff_forge_graph::{
    EvidenceSignal, MAX_EVIDENCE_PER_FILE, RankedFile, Side, file_evidence, hunk_signals,
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
    one_text_in_hunks(pick, &text, true);
    one_text_in_hunks(pick, &text, false);
    let patch = format!("--- a/{path}\n+++ b/{path}\n@@ -1,{old} +1,{new} @@\n{body}");
    let Ok(files) = deep_diff_forge_patch::parse(&patch) else {
        return;
    };
    for file in &files {
        let ranked = RankedFile::scored(file);
        assert!(ranked.evidence.len() <= MAX_EVIDENCE_PER_FILE);
        let all = file_evidence(file);
        assert_eq!(ranked.evidence.len(), all.len().min(MAX_EVIDENCE_PER_FILE));
        let released = || all.iter().filter(|e| e.released);
        let listed = || ranked.evidence.iter().filter(|e| e.released);
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
/// is evidence of the patch. `created`: the patch creates the file. Otherwise the stretch is
/// the end of a file that exists: the first hunk shows one line and adds after it, and each
/// later hunk only adds, at an old position that is any number above 1.
fn one_text_in_hunks(pick: u8, text: &str, created: bool) {
    let lines: Vec<String> = text
        .lines()
        .map(|line| line.chars().filter(|c| *c != '\r').collect())
        .collect();
    if lines.is_empty() {
        return;
    }
    let mut patch = String::from(if created {
        "--- /dev/null\n+++ b/src/cut.rs\n"
    } else {
        "--- a/src/cut.rs\n+++ b/src/cut.rs\n"
    });
    let mut at = 0;
    while at < lines.len() {
        let take = (1 + (usize::from(pick) + lines[at].len()) % 4).min(lines.len() - at);
        if created {
            let _ = writeln!(patch, "@@ -{at},0 +{},{take} @@", at + 1);
        } else if at == 0 {
            let _ = writeln!(patch, "@@ -7,1 +7,{} @@\n x", take + 1);
        } else {
            let anywhere = 2 + (usize::from(pick) + lines[at].len()) % 60;
            let _ = writeln!(patch, "@@ -{anywhere},0 +{},{take} @@", at + 8);
        }
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
    let evidence = file_evidence(file);
    let mut state = RustLexerState::default();
    let added = file.patch_twin.hunks.iter().flat_map(|hunk| &hunk.lines);
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
                "read as one text, line {number} has `{token}`; cut into hunks it is not evidence \
                 (created: {created}): {:?}",
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
