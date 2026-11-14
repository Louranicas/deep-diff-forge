#![no_main]
// Code evidence (plan 2026-10-30, Feature A): for any patch, ranking never panics, every evidence
// item names a hunk of its own file by address and a line inside that hunk on its side, the
// per-hunk labels agree with the file's evidence, and the cap holds.
//
// And no answer is lower than 0.8.3's: in a Rust file, a rule token that the reading of 0.8.3
// shows on a changed line is evidence, whatever the lexer reading makes of that line. The
// reading of 0.8.3 is `lexical::code_view`, followed down each side of a hunk as the scan
// follows it; three token rules are derived here again from that view alone.
//
// Input: the first byte picks a path (and so a language and rule scope), the rest is the body
// of one hunk; every line becomes a changed or context line, so the scanner sees comments and
// strings opening and closing across lines.

use deep_diff_forge_core::PatchLineKind;
use deep_diff_forge_graph::lexical::{Language, ScanState, code_view, has_token};
use deep_diff_forge_graph::{
    EvidenceSignal, MAX_EVIDENCE_PER_FILE, RankedFile, Side, file_evidence, hunk_signals,
};
use libfuzzer_sys::fuzz_target;

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
    let patch = format!("--- a/{path}\n+++ b/{path}\n@@ -1,{old} +1,{new} @@\n{body}");
    let Ok(files) = deep_diff_forge_patch::parse(&patch) else {
        return;
    };
    for file in &files {
        let ranked = RankedFile::scored(file);
        assert!(ranked.evidence.len() <= MAX_EVIDENCE_PER_FILE);
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
