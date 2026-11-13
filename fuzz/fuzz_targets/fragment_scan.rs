#![no_main]
// The fragment scanner (agent batteries, the evidence ladder's rung 2) and what the evidence
// scan does with its answer. Four reviews of the integrity signals each found a way to make the
// parser's answer remove evidence it had no proof against, and an independent review found
// that the first version of this target could not see a wrong removal (any panic path of a new
// file was allowed to go). The rule it holds now:
//
//   the parser may add an item or raise its basis; it may remove only `panic_path_added`, only
//   in a file the patch creates whole, and only from a line whose every `unwrap` and `expect`
//   in code lies inside one item the scanner proved to be test code. A file whose first line
//   begins `#!` without being an inner attribute proves nothing.
//
// Checked for any text, as a Rust fragment and as the body of a patch:
// 1. `scan_fragment` never panics, gives the same answer twice, and its rows are in order and
//    inside the fragment.
// 2. Every item the token rules find alone (the parser switched off) is still there with the
//    parser on, except `syntax_unchecked` (the mark of the parser being off) and a
//    `panic_path_added` under the rule above ([`may_withdraw`], which reads the scanner's
//    spans and the lexer's code view itself and none of the evidence scan's own reasoning).
// 3. `syntax_fragment` is the basis of new-side items only, and with the parser off no item has
//    it.
//
// Input: the first byte picks the shape (a new file, or a change with added, removed and
// unchanged lines) and a path; the rest is the text.

use deep_diff_forge_graph::{
    Basis, EvidenceSignal, Reading, Side, file_evidence, file_evidence_in,
};
use deep_diff_forge_graph::lexical::{self, ScanState};
use deep_diff_forge_syntax::{Language, Span, scan_fragment};
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
    for (start, end) in scan.test_items.iter().chain(&scan.spelled_items) {
        assert!(start < end && end.0 <= rows, "an item's stretch is out of order or outside");
    }
    for construct in &scan.constructs {
        assert!(
            (construct.row, construct.column) <= construct.end,
            "a construct's own part ends before it starts"
        );
    }
}

/// Whether a panic path on line `line` (from 1) of a new file whose text is `lines` may be
/// withdrawn: the file's first line is no shebang, and every `unwrap` and `expect` in the
/// line's code lies inside one of `items`.
fn may_withdraw(lines: &[&str], items: &[Span], line: u32) -> bool {
    let first = lines.first().copied().unwrap_or("");
    if first.starts_with("#!") && !first.starts_with("#![") {
        return false;
    }
    let Some(row) = line.checked_sub(1) else {
        return false;
    };
    let mut state = ScanState::default();
    let mut code = String::new();
    for text in lines.iter().take(row as usize + 1) {
        code = lexical::code_view(lexical::Language::Rust, text, &mut state);
    }
    let words: Vec<u32> = ["unwrap", "expect"]
        .iter()
        .flat_map(|word| code.match_indices(word).map(|(at, _)| at))
        .map(|at| u32::try_from(at).unwrap_or(u32::MAX))
        .collect();
    !words.is_empty()
        && items.iter().any(|(start, end)| {
            words
                .iter()
                .all(|column| *start <= (row, *column) && (row, *column) < *end)
        })
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
    let lines: Vec<&str> = text.lines().collect();
    let proved = scan_fragment(Language::Rust, &lines.join("\n"))
        .map(|scan| scan.test_items)
        .unwrap_or_default();
    for file in &files {
        let parser_off = vec![Reading::Off; file.patch_twin.hunks.len()];
        let tokens_only = file_evidence_in(file, &parser_off);
        let with_parser = file_evidence(file);
        for item in &tokens_only {
            assert!(
                item.basis != Basis::SyntaxFragment,
                "an item has the parser's basis with the parser off"
            );
            let may_go = item.signal == EvidenceSignal::SyntaxUnchecked
                || (item.signal == EvidenceSignal::PanicPathAdded
                    && new_file
                    && may_withdraw(&lines, &proved, item.line));
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
