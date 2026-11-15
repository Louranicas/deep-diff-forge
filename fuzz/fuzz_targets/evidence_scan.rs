#![no_main]
// Code evidence (plan 2026-10-30, Feature A): for any patch, ranking never panics, every evidence
// item names a hunk of its own file by address and a line inside that hunk on its side, the
// per-hunk labels agree with the file's evidence, and the cap holds.
//
//
// And the same lines as hunks of three lines with unseen lines between them (eighth independent
// review, IR8-M-2: nothing but the views called `hunk_signals_alone`, and no target held what
// a view of hunks by path costs). A hunk that is read alone has, from the reader of one hunk,
// the labels the scan of its file gives it, and that reader makes no pass over the file. The
// view of the file's hunks by path shows for each hunk the labels the outline shows, and goes
// through every hunk of the file a fixed number of times (two), however many hunks there are.
//
// Input: the first byte picks a path (and so a language and rule scope), the rest is the body
// of one hunk; every line becomes a changed or context line, so the scanner sees comments and
// strings opening and closing across lines.

use deep_diff_forge_daemon::views::{MAX_MAX_BYTES, Page, Selection, hunks, outline};
use deep_diff_forge_graph::{
    EvidenceSignal, FileKinds, MAX_EVIDENCE_PER_FILE, RankedFile, Side, file_evidence, file_walks,
    hunk_signals, hunk_signals_alone, joined_hunks,
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
        let evidence = file_evidence(file);
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
    let patch = format!("--- a/{path}\n+++ b/{path}\n@@ -1,{old} +1,{new} @@\n{body}");
    let Ok(files) = deep_diff_forge_patch::parse(&patch) else {
        return;
    };
    for file in &files {
        let ranked = RankedFile::scored(file);
        assert!(ranked.evidence.len() <= MAX_EVIDENCE_PER_FILE);
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
    }
});
