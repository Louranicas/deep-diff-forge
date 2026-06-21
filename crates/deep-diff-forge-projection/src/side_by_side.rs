use deep_diff_forge_core::{PatchLine, PatchLineKind, ReviewFile, display_safe};
use std::fmt::Write as _;

/// One cell of a side-by-side row (one side, old or new).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SideCell {
    /// Line number on this side.
    pub line: u32,
    /// `' '` context, `'+'` added, `'-'` removed.
    pub marker: char,
    /// Cell text.
    pub text: String,
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
            marker: ' ',
            text: line.text.clone(),
        }),
        right: Some(SideCell {
            line: line.new_line.unwrap_or(0),
            marker: ' ',
            text: line.text.clone(),
        }),
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
    }
}

fn fmt_cell(cell: Option<&SideCell>, width: usize) -> String {
    match cell {
        Some(c) => {
            let body = truncate_pad(&c.text, width);
            format!("{:>4} {} {body}", c.line, c.marker)
        }
        None => format!("{:>4}   {}", "", truncate_pad("", width)),
    }
}

fn truncate_pad(text: &str, width: usize) -> String {
    // Neutralise terminal escapes BEFORE width math, so the visible escaped form
    // is what gets measured, truncated, and padded (an attacker cannot use a
    // zero-width control sequence to desync column alignment).
    let safe = display_safe(text);
    let count = safe.chars().count();
    if count > width {
        safe.chars().take(width).collect()
    } else {
        format!("{safe}{}", " ".repeat(width - count))
    }
}

/// Render every file as two aligned columns with the given per-column text width.
#[must_use]
pub fn render_side_by_side(files: &[ReviewFile], width: usize) -> String {
    let mut out = String::new();
    for file in files {
        let status = crate::status_label(file.status);
        let _ = writeln!(out, "{status}  {}", display_safe(&file.path));
        for row in side_rows(file) {
            let _ = writeln!(
                out,
                "{} | {}",
                fmt_cell(row.left.as_ref(), width),
                fmt_cell(row.right.as_ref(), width)
            );
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
        assert!(side_rows(&files[0]).is_empty());
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
        let s = fmt_cell(None, 4);
        assert!(s.contains("    "));
    }

    #[test]
    fn fmt_cell_some_shows_marker_and_text() {
        let c = SideCell {
            line: 3,
            marker: '+',
            text: "hi".to_string(),
        };
        let s = fmt_cell(Some(&c), 4);
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
}
