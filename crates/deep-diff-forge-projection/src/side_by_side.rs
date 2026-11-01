// DDF integration: R31, R32. Measure displayed cells and preserve grapheme boundaries after terminal-safety escaping; keep remaining chrome/wrap geometry visible as work.
// Design/status: docs/RECOMMENDATION_INTEGRATION.md#terminal-geometry
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fdeep-diff-forge-projection--side_by_side
use crate::notes::{LOSSY, NO_NEWLINE, RENDERS_ALIKE, gutter_width, renders_alike, side_shown};
use deep_diff_forge_core::{PatchLine, PatchLineKind, ReviewFile, display_safe};
use std::fmt::Write as _;

/// One cell of a side-by-side row (one side, old or new).
///
/// Read its fields; build it with [`side_rows`]. Outside this crate it cannot be built by a
/// struct literal, so a flag added later is not a breaking change:
///
/// ```compile_fail
/// let _ = deep_diff_forge_projection::SideCell {
///     line: 1,
///     marker: ' ',
///     text: String::new(),
///     lossy: false,
///     no_newline: false,
/// };
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SideCell {
    /// Line number on this side.
    pub line: u32,
    /// `' '` context, `'+'` added, `'-'` removed.
    pub marker: char,
    /// Cell text.
    pub text: String,
    /// The line was not UTF-8 in the patch; `text` is its lossy decoding.
    pub lossy: bool,
    /// The line has no terminating newline.
    pub no_newline: bool,
}

/// One side-by-side row: an optional old-side cell and an optional new-side cell.
///
/// A context line fills both sides; a removed line fills only `left`; an added
/// line fills only `right`. Within a change block, removed and added lines are
/// zipped so the surviving change is visible across the gutter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SideRow {
    /// Old-side cell, if present.
    pub left: Option<SideCell>,
    /// New-side cell, if present.
    pub right: Option<SideCell>,
}

/// Build aligned side-by-side rows for one file's hunks.
#[must_use]
pub fn side_rows(file: &ReviewFile) -> Vec<SideRow> {
    let mut rows = Vec::new();
    for hunk in &file.patch_twin.hunks {
        let lines = &hunk.lines;
        let mut i = 0;
        while i < lines.len() {
            if lines[i].kind == PatchLineKind::Context {
                rows.push(context_row(&lines[i]));
                i += 1;
                continue;
            }
            // Gather a change block: removed run, then added run (git order).
            let mut removed = Vec::new();
            while i < lines.len() && lines[i].kind == PatchLineKind::Removed {
                removed.push(&lines[i]);
                i += 1;
            }
            let mut added = Vec::new();
            while i < lines.len() && lines[i].kind == PatchLineKind::Added {
                added.push(&lines[i]);
                i += 1;
            }
            let pairs = removed.len().max(added.len());
            for k in 0..pairs {
                rows.push(SideRow {
                    left: removed.get(k).map(|l| cell(l, '-')),
                    right: added.get(k).map(|l| cell(l, '+')),
                });
            }
        }
    }
    rows
}

fn context_row(line: &PatchLine) -> SideRow {
    SideRow {
        left: Some(SideCell {
            line: line.old_line.unwrap_or(0),
            ..cell(line, ' ')
        }),
        right: Some(cell(line, ' ')),
    }
}

fn cell(line: &PatchLine, marker: char) -> SideCell {
    let number = match marker {
        '-' => line.old_line,
        _ => line.new_line,
    };
    SideCell {
        line: number.unwrap_or(0),
        marker,
        text: line.text.clone(),
        lossy: line.lossy,
        no_newline: line.no_newline,
    }
}

/// One cell: the line number in a `gutter`-wide field, the marker, and the text in `width` cells.
fn fmt_cell(cell: Option<&SideCell>, gutter: usize, width: usize) -> String {
    match cell {
        Some(c) => {
            let body = truncate_pad(&c.text, width);
            format!("{:>gutter$} {} {body}", c.line, c.marker)
        }
        None => format!("{:>gutter$}   {}", "", truncate_pad("", width)),
    }
}

/// A note cell: blank numbers, then the note from the marker column on, fitted to the cell.
fn fmt_note(note: Option<&str>, gutter: usize, width: usize) -> String {
    format!(
        "{:>gutter$} {}",
        "",
        crate::fit_cells(note.unwrap_or(""), width + 2)
    )
}

/// The note rows after `row`: each flag that either side carries, on the side(s) carrying it,
/// then a note on both sides when a removed and an added line differ only in bytes shown alike
/// in their `width`-cell cells.
fn note_rows(row: &SideRow, width: usize) -> Vec<(Option<&'static str>, Option<&'static str>)> {
    let (left, right) = (row.left.as_ref(), row.right.as_ref());
    let has = |cell: Option<&SideCell>, flag: fn(&SideCell) -> bool| cell.is_some_and(flag);
    let mut notes = Vec::new();
    for (note, flag) in [
        (LOSSY, (|c: &SideCell| c.lossy) as fn(&SideCell) -> bool),
        (NO_NEWLINE, |c: &SideCell| c.no_newline),
    ] {
        let (l, r) = (has(left, flag), has(right, flag));
        if l || r {
            notes.push((l.then_some(note), r.then_some(note)));
        }
    }
    if let (Some(l), Some(r)) = (left, right)
        && renders_alike(&l.text, &r.text, |t| side_shown(t, width))
    {
        notes.push((Some(RENDERS_ALIKE), Some(RENDERS_ALIKE)));
    }
    notes
}

fn truncate_pad(text: &str, width: usize) -> String {
    // Neutralise terminal escapes BEFORE width math, so the visible escaped form
    // is what gets measured, truncated, and padded (an attacker cannot use a
    // zero-width control sequence to desync column alignment).
    // One function prints the cell and decides whether a pair looks alike, so they cannot drift.
    side_shown(text, width)
}

/// Render every file as two aligned columns with the given per-column text width. The number
/// fields are as wide as the file's largest line number, so the ` | ` gutter sits in one column
/// on every row, and a row is followed by a `\` note for what its cells cannot show: a lossy
/// line, a missing final newline, or a pair whose printed cells look the same (a tab is shown as
/// four spaces, padding hides a trailing space, and the cut hides a change past `width`).
#[must_use]
pub fn render_side_by_side(files: &[ReviewFile], width: usize) -> String {
    let mut out = String::new();
    for file in files {
        let status = file.status.label();
        let gutter = gutter_width(file);
        let _ = writeln!(out, "{status}  {}", display_safe(&file.path));
        for row in side_rows(file) {
            let _ = writeln!(
                out,
                "{} | {}",
                fmt_cell(row.left.as_ref(), gutter, width),
                fmt_cell(row.right.as_ref(), gutter, width)
            );
            for (left, right) in note_rows(&row, width) {
                let _ = writeln!(
                    out,
                    "{} | {}",
                    fmt_note(left, gutter, width),
                    fmt_note(right, gutter, width)
                );
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use deep_diff_forge_patch::parse;

    const BASIC: &str = "--- a/x\n+++ b/x\n@@ -1,3 +1,3 @@\n a\n-b\n+B\n c\n";

    fn rows(input: &str) -> Vec<SideRow> {
        let files = parse(input).unwrap();
        side_rows(&files[0])
    }

    #[test]
    fn context_fills_both_sides() {
        let r = rows(BASIC);
        assert!(r[0].left.is_some());
        assert!(r[0].right.is_some());
        assert_eq!(r[0].left.as_ref().unwrap().marker, ' ');
    }

    #[test]
    fn change_block_pairs_removed_with_added() {
        let r = rows(BASIC);
        // a (ctx), [b/-, B/+] paired, c (ctx) => 3 rows.
        assert_eq!(r.len(), 3);
        let changed = &r[1];
        assert_eq!(changed.left.as_ref().unwrap().marker, '-');
        assert_eq!(changed.right.as_ref().unwrap().marker, '+');
    }

    #[test]
    fn removed_only_block_has_empty_right() {
        let input = "--- a/x\n+++ b/x\n@@ -1,2 +1,1 @@\n keep\n-gone\n";
        let r = rows(input);
        let changed = r
            .iter()
            .find(|x| x.left.as_ref().is_some_and(|c| c.marker == '-'))
            .unwrap();
        assert!(changed.right.is_none());
    }

    #[test]
    fn added_only_block_has_empty_left() {
        let input = "--- /dev/null\n+++ b/n\n@@ -0,0 +1,2 @@\n+one\n+two\n";
        let r = rows(input);
        assert!(r.iter().all(|x| x.left.is_none()));
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn uneven_block_pads_shorter_side() {
        // two removed, one added => 2 rows; second row has no right.
        let input = "--- a/x\n+++ b/x\n@@ -1,2 +1,1 @@\n-a\n-b\n+C\n";
        let r = rows(input);
        assert_eq!(r.len(), 2);
        assert!(r[0].right.is_some());
        assert!(r[1].right.is_none());
    }

    #[test]
    fn left_cell_uses_old_line_number() {
        let r = rows(BASIC);
        assert_eq!(r[1].left.as_ref().unwrap().line, 2);
    }

    #[test]
    fn right_cell_uses_new_line_number() {
        let r = rows(BASIC);
        assert_eq!(r[1].right.as_ref().unwrap().line, 2);
    }

    #[test]
    fn context_left_uses_old_right_uses_new() {
        let input = "--- a/x\n+++ b/x\n@@ -5,1 +9,1 @@\n same\n";
        let r = rows(input);
        assert_eq!(r[0].left.as_ref().unwrap().line, 5);
        assert_eq!(r[0].right.as_ref().unwrap().line, 9);
    }

    #[test]
    fn binary_file_has_no_rows() {
        let files =
            parse("diff --git a/p.png b/p.png\nBinary files a/p.png and b/p.png differ\n").unwrap();
        assert_eq!(side_rows(&files[0]).len(), 0);
    }

    #[test]
    fn render_contains_gutter() {
        let files = parse(BASIC).unwrap();
        let out = render_side_by_side(&files, 20);
        assert!(out.contains(" | "));
    }

    #[test]
    fn render_contains_status_header() {
        let files = parse(BASIC).unwrap();
        let out = render_side_by_side(&files, 20);
        assert!(out.contains("modified  x"));
    }

    #[test]
    fn truncate_pad_pads_short_text() {
        assert_eq!(truncate_pad("hi", 5), "hi   ");
    }

    #[test]
    fn truncate_pad_truncates_long_text() {
        assert_eq!(truncate_pad("hello world", 5), "hello");
    }

    #[test]
    fn truncate_pad_handles_unicode_width() {
        // 3 chars, pad to 5 => 2 trailing spaces.
        assert_eq!(truncate_pad("a→b", 5), "a→b  ");
    }

    #[test]
    fn fmt_cell_none_is_blank_columns() {
        let s = fmt_cell(None, 4, 4);
        assert_eq!(s, " ".repeat(11));
        assert_eq!(fmt_cell(None, 6, 4), " ".repeat(13));
    }

    #[test]
    fn fmt_cell_some_shows_marker_and_text() {
        let c = SideCell {
            line: 3,
            marker: '+',
            text: "hi".to_string(),
            lossy: false,
            no_newline: false,
        };
        assert_eq!(fmt_cell(Some(&c), 5, 4), "    3 + hi  ");
        let s = fmt_cell(Some(&c), 4, 4);
        assert!(s.contains('+'));
        assert!(s.contains("hi"));
        assert!(s.contains('3'));
    }

    #[test]
    fn render_empty_is_empty() {
        assert_eq!(render_side_by_side(&[], 20), "");
    }

    #[test]
    fn multi_hunk_rows_accumulate() {
        let input = "--- a/x\n+++ b/x\n@@ -1,1 +1,1 @@\n-a\n+A\n@@ -9,1 +9,1 @@\n-b\n+B\n";
        let r = rows(input);
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn render_width_controls_column_size() {
        let files = parse(BASIC).unwrap();
        let narrow = render_side_by_side(&files, 5);
        let wide = render_side_by_side(&files, 40);
        assert!(wide.len() > narrow.len());
    }

    #[test]
    fn paired_change_row_count_is_max_of_sides() {
        let input = "--- a/x\n+++ b/x\n@@ -1,1 +1,3 @@\n-a\n+A\n+B\n+C\n";
        let r = rows(input);
        assert_eq!(r.len(), 3);
        assert!(r[0].left.is_some());
        assert!(r[1].left.is_none());
        assert!(r[2].left.is_none());
    }

    /// Bug hunt 2026-11-01: a change the cells cannot show gets a note row. Before, an
    /// end-of-file-newline-only change, a change between two non-UTF-8 bytes and a Makefile tab
    /// turned into four spaces each printed as one row whose two sides looked the same.
    #[test]
    fn changes_the_cells_cannot_show_get_a_note_row() {
        let nonl = "--- a/x.sh\n+++ b/x.sh\n@@ -1 +1 @@\n-rm -rf /tmp/cache\n\\ No newline at end of file\n+rm -rf /tmp/cache\n";
        let out = render_side_by_side(&parse(nonl).unwrap(), 20);
        assert_eq!(
            out,
            "modified  x.sh\n   1 - rm -rf /tmp/cache    |    1 + rm -rf /tmp/cache   \n     \\ No newline at end of |                            \n"
        );
        let tab =
            "--- a/Makefile\n+++ b/Makefile\n@@ -1,2 +1,2 @@\n all:\n-\techo hi\n+    echo hi\n";
        let out = render_side_by_side(&parse(tab).unwrap(), 60);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 4, "{out}");
        assert_eq!(lines[3].matches(RENDERS_ALIKE).count(), 2, "{out}");
        assert_eq!(lines[3].find(" | "), lines[2].find(" | "));
        let lossy = b"--- a/x.txt\n+++ b/x.txt\n@@ -1 +1 @@\n-key=\xff\n+key=\xfe\n";
        let decoded = deep_diff_forge_patch::decode(lossy);
        let files = deep_diff_forge_patch::parse_decoded(
            &decoded,
            deep_diff_forge_patch::ParseOptions::default(),
        )
        .unwrap();
        let out = render_side_by_side(&files, 60);
        assert_eq!(
            out.lines().nth(2).unwrap().matches(LOSSY).count(),
            2,
            "{out}"
        );
        // Context with no final newline is noted on both sides; a plain change is not noted.
        let ctx = "--- a/x\n+++ b/x\n@@ -1,2 +1,2 @@\n-a\n+b\n c\n\\ No newline at end of file\n";
        let out = render_side_by_side(&parse(ctx).unwrap(), 30);
        assert_eq!(out.lines().count(), 4, "{out}");
        assert_eq!(out.lines().nth(3).unwrap().matches(NO_NEWLINE).count(), 2);
        // Only the side carrying the flag gets the note.
        let added = "--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n\\ No newline at end of file\n";
        let out = render_side_by_side(&parse(added).unwrap(), 30);
        let note = out.lines().nth(2).unwrap();
        let (left, right) = note.split_once(" | ").unwrap();
        assert!(
            !left.contains(NO_NEWLINE) && right.contains(NO_NEWLINE),
            "{out}"
        );
        assert_eq!(note_rows(&rows(BASIC)[1], 60).len(), 0);
    }

    /// Review 2026-11-01: the cells as printed decide whether a pair looks alike. Before, a
    /// trailing space (padding hides it), a no-break space against a space and a change past
    /// the cell width (the cut hides it) each printed one row whose two sides were the same.
    #[test]
    fn a_pair_whose_printed_cells_match_is_noted() {
        let long = "x".repeat(100);
        let input = format!(
            "--- a/x\n+++ b/x\n@@ -1,3 +1,3 @@\n-foo\n-a b\n-{long}A\n+foo \n+a\u{a0}b\n+{long}B\n"
        );
        let out = render_side_by_side(&parse(&input).unwrap(), 60);
        let lines: Vec<&str> = out.lines().skip(1).collect();
        assert_eq!(lines.len(), 6, "{out}");
        for note in lines.iter().skip(1).step_by(2) {
            assert_eq!(note.matches(RENDERS_ALIKE).count(), 2, "{out}");
        }
        // Wider than the cell only on one side, a wider space, or a visible difference: no note.
        for (old, new) in [
            ("a b", "a\u{3000}b"),
            ("a  b", "a b"),
            ("foo", " foo"),
            (long.as_str(), "x"),
        ] {
            let input = format!("--- a/x\n+++ b/x\n@@ -1 +1 @@\n-{old}\n+{new}\n");
            let out = render_side_by_side(&parse(&input).unwrap(), 60);
            assert_eq!(out.lines().count(), 2, "{old:?} {new:?}: {out}");
        }
        let wide = "--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a  b\n+a\u{3000}b\n";
        let out = render_side_by_side(&parse(wide).unwrap(), 60);
        assert_eq!(out.matches(RENDERS_ALIKE).count(), 2, "{out}");
        // Cut at two cells the pair looks alike and is noted; at three it differs and is not.
        let narrow = render_side_by_side(&parse(&input_pair("abc", "abd")).unwrap(), 2);
        assert_eq!(narrow.lines().count(), 3, "the note row follows: {narrow}");
        let wider = render_side_by_side(&parse(&input_pair("abc", "abd")).unwrap(), 3);
        assert_eq!(wider.lines().count(), 2, "{wider}");
    }

    fn input_pair(old: &str, new: &str) -> String {
        format!("--- a/x\n+++ b/x\n@@ -1 +1 @@\n-{old}\n+{new}\n")
    }

    /// Bug hunt 2026-11-01: the gutter stays in one column when line numbers reach five
    /// digits. Before, `{:>4}` pushed the rows at 10000 and later one column right.
    #[test]
    fn the_gutter_stays_aligned_across_line_ten_thousand() {
        let input = "--- a/x\n+++ b/x\n@@ -9998,4 +9998,4 @@\n a\n b\n-c\n+C\n d\n";
        let out = render_side_by_side(&parse(input).unwrap(), 10);
        let body: Vec<&str> = out.lines().skip(1).collect();
        assert_eq!(body.len(), 4);
        assert_eq!(body[0], " 9998   a          |  9998   a         ");
        assert_eq!(body[2], "10000 - c          | 10000 + C         ");
        assert!(body.iter().all(|row| row.find(" | ") == Some(18)), "{out}");
    }
}
