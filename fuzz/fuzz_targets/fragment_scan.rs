#![no_main]
// The fragment scanner (agent batteries, the evidence ladder's rung 2) and what the evidence
// scan does with its answer. Five rounds of review each found a way to make the parser's answer
// remove evidence it had no proof against, and a second independent review found four more
// texts that the grammar and the compiler read differently (IR2-H-1 to IR2-H-4). So the rule
// this target holds is now the plain one:
//
//   the parser may add an item or raise its basis; it removes nothing.
//
// Checked for any text, as a Rust fragment and as the body of a patch:
// 1. `scan_fragment` never panics, gives the same answer twice, and its rows are in order and
//    inside the fragment.
// 2. Every item the token rules find alone (the parser switched off) is still there with the
//    parser on, except `syntax_unchecked` (the mark of the parser being off).
// 3. `syntax_fragment` is the basis of new-side items only, and with the parser off no item has
//    it.
//
// Input: the first byte picks the shape (a new file, or a change with added, removed and
// unchanged lines) and a path; the rest is the text.

use deep_diff_forge_graph::{
    Basis, EvidenceSignal, Reading, Side, file_evidence, file_evidence_in,
};
use deep_diff_forge_syntax::{Language, scan_fragment};
use libfuzzer_sys::fuzz_target;

const PATHS: &[&str] = &[
    "src/lib.rs",
    "tests/it.rs",
    "src/generated/x.rs",
    "build.rs",
];

fn check_scan(text: &str) {
    let scan = scan_fragment(Language::Rust, text);
    assert_eq!(
        scan,
        scan_fragment(Language::Rust, text),
        "two scans differ"
    );
    let Some(scan) = scan else {
        return;
    };
    let rows = u32::try_from(text.bytes().filter(|b| *b == b'\n').count()).unwrap_or(u32::MAX);
    for construct in &scan.constructs {
        assert!(
            construct.row <= construct.last_row,
            "a construct ends before it starts"
        );
        assert!(
            construct.last_row <= rows,
            "a construct lies outside the fragment"
        );
    }
    for (first, last) in &scan.unreadable {
        assert!(
            first <= last && *last <= rows,
            "a row range is out of order or outside"
        );
    }
    for pair in scan.unreadable.windows(2) {
        assert!(pair[0].1 + 1 < pair[1].0, "unreadable ranges are not merged");
    }
    for (start, end) in &scan.spelled_items {
        assert!(start < end && end.0 <= rows, "an item's stretch is out of order or outside");
    }
    for construct in &scan.constructs {
        assert!(
            (construct.row, construct.column) <= construct.end,
            "a construct's own part ends before it starts"
        );
    }
}

fuzz_target!(|data: &[u8]| {
    let Some((&pick, rest)) = data.split_first() else {
        return;
    };
    if rest.len() > 48 * 1024 {
        return;
    }
    let text: String = String::from_utf8_lossy(rest)
        .chars()
        .filter(|c| *c != '\r')
        .collect();
    check_scan(&text);

    let path = PATHS[usize::from(pick >> 1) % PATHS.len()];
    let new_file = pick & 1 == 1;
    let mut body = String::new();
    let (mut old, mut new) = (0u32, 0u32);
    for (i, line) in text.lines().enumerate() {
        let sign = match (new_file, i % 3) {
            (true, _) | (false, 0) => '+',
            (false, 1) => '-',
            (false, _) => ' ',
        };
        body.push(sign);
        body.push_str(line);
        body.push('\n');
        old += u32::from(sign != '+');
        new += u32::from(sign != '-');
    }
    if new == 0 {
        return;
    }
    let patch = if new_file {
        format!(
            "diff --git a/{path} b/{path}\nnew file mode 100644\n--- /dev/null\n+++ b/{path}\n@@ -0,0 +1,{new} @@\n{body}"
        )
    } else {
        format!("--- a/{path}\n+++ b/{path}\n@@ -1,{old} +1,{new} @@\n{body}")
    };
    let Ok(files) = deep_diff_forge_patch::parse(&patch) else {
        return;
    };
    for file in &files {
        let parser_off = vec![Reading::Off; file.patch_twin.hunks.len()];
        let tokens_only = file_evidence_in(file, &parser_off);
        let with_parser = file_evidence(file);
        for item in &tokens_only {
            assert!(
                item.basis != Basis::SyntaxFragment,
                "an item has the parser's basis with the parser off"
            );
            let may_go = item.signal == EvidenceSignal::SyntaxUnchecked;
            let kept = with_parser.iter().any(|kept| {
                (kept.signal, kept.side, kept.line, kept.hunk)
                    == (item.signal, item.side, item.line, item.hunk)
                    && kept.basis >= item.basis
            });
            assert!(
                kept || may_go,
                "the parser removed {} at line {} of {}",
                item.signal.label(),
                item.line,
                file.path
            );
        }
        for item in &with_parser {
            assert!(
                item.basis != Basis::SyntaxFragment || item.side == Side::New,
                "an old-side item has the parser's basis"
            );
        }
    }
});
