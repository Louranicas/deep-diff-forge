use crate::notes::{RENDERS_ALIKE, gutter_width, inline_shown, line_notes, renders_alike};
use deep_diff_forge_core::{PatchLine, PatchLineKind, ReviewFile, display_safe};
use std::fmt::Write as _;

/// One inline display row: old/new line numbers, a marker, and the text.
///
/// Read its fields; build it with [`inline_rows`]. Outside this crate it cannot be built by a
/// struct literal, so a flag added later is not a breaking change:
///
/// ```compile_fail
/// let _ = deep_diff_forge_projection::InlineRow {
///     old_line: None,
///     new_line: None,
///     marker: ' ',
///     text: String::new(),
///     lossy: false,
///     no_newline: false,
/// };
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct InlineRow {
    /// Old-side line number, if the row exists on the old side.
    pub old_line: Option<u32>,
    /// New-side line number, if the row exists on the new side.
    pub new_line: Option<u32>,
    /// `' '` context, `'+'` added, `'-'` removed.
    pub marker: char,
    /// Row text (without the leading marker).
    pub text: String,
    /// The line was not UTF-8 in the patch; `text` is its lossy decoding.
    pub lossy: bool,
    /// The line has no terminating newline.
    pub no_newline: bool,
}

/// Build the inline display rows for one file's hunks (body lines only).
#[must_use]
pub fn inline_rows(file: &ReviewFile) -> Vec<InlineRow> {
    let mut rows = Vec::new();
    for hunk in &file.patch_twin.hunks {
        for line in &hunk.lines {
            rows.push(InlineRow {
                old_line: line.old_line,
                new_line: line.new_line,
                marker: marker_of(line.kind),
                text: line.text.clone(),
                lossy: line.lossy,
                no_newline: line.no_newline,
            });
        }
    }
    rows
}

fn marker_of(kind: PatchLineKind) -> char {
    match kind {
        PatchLineKind::Context => ' ',
        PatchLineKind::Added => '+',
        PatchLineKind::Removed => '-',
    }
}

fn num(value: Option<u32>, width: usize) -> String {
    value.map_or_else(|| " ".repeat(width), |n| format!("{n:>width$}"))
}

/// For each line of a hunk, the removed line it pairs with: the k-th added line of a change
/// block pairs with the k-th removed line, as the side-by-side layout zips them.
fn pairs(lines: &[PatchLine]) -> Vec<Option<usize>> {
    let mut paired = vec![None; lines.len()];
    // The current block's removed lines, and the next one an added line pairs with.
    let mut removed = Vec::new();
    let mut next = 0;
    let mut previous = PatchLineKind::Context;
    for (i, line) in lines.iter().enumerate() {
        match line.kind {
            PatchLineKind::Removed => {
                if previous != PatchLineKind::Removed {
                    removed.clear();
                }
                removed.push(i);
            }
            PatchLineKind::Added => {
                if previous != PatchLineKind::Added {
                    next = 0;
                }
                paired[i] = removed.get(next).copied();
                next += 1;
            }
            PatchLineKind::Context => removed.clear(),
        }
        previous = line.kind;
    }
    paired
}

/// Render every file inline: a status/path header, per-hunk separators, and
/// `old new marker text` rows, each followed by a `\` note for what its text cannot show (a
/// lossy line, a missing final newline, an added line that shows exactly as its removed pair).
/// Tabs are printed raw, so "shows exactly as" assumes the terminal's default 8-column tab stops.
/// The number fields are as wide as the file's largest line number. Patch truth is read-only.
#[must_use]
pub fn render_inline(files: &[ReviewFile]) -> String {
    let mut out = String::new();
    for file in files {
        let status = file.status.label();
        let width = gutter_width(file);
        // `  old new m ` precedes the text: the screen column its raw tabs advance from.
        let column = 2 * width + 6;
        // Paths and line text are attacker-controlled; neutralise terminal
        // escapes before they reach a reviewer's terminal.
        let _ = writeln!(out, "{status}  {}", display_safe(&file.path));
        for hunk in &file.patch_twin.hunks {
            let old_start = hunk.old_start.unwrap_or(0);
            let new_start = hunk.new_start.unwrap_or(0);
            let _ = writeln!(out, "  @@ -{old_start} +{new_start} @@");
            let paired = pairs(&hunk.lines);
            for (line, pair) in hunk.lines.iter().zip(paired) {
                let _ = writeln!(
                    out,
                    "  {} {} {} {}",
                    num(line.old_line, width),
                    num(line.new_line, width),
                    marker_of(line.kind),
                    display_safe(&line.text)
                );
                let alike = pair.is_some_and(|r| {
                    renders_alike(&hunk.lines[r].text, &line.text, |t| inline_shown(t, column))
                });
                let notes = line_notes(line.lossy, line.no_newline);
                for note in notes.chain(alike.then_some(RENDERS_ALIKE)) {
                    let _ = writeln!(out, "  {} {note}", " ".repeat(2 * width + 1));
                }
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

    fn rows(input: &str) -> Vec<InlineRow> {
        let files = parse(input).unwrap();
        inline_rows(&files[0])
    }

    #[test]
    fn inline_rows_count_matches_lines() {
        assert_eq!(rows(BASIC).len(), 4);
    }

    #[test]
    fn context_row_has_both_line_numbers() {
        let r = rows(BASIC);
        assert_eq!(r[0].marker, ' ');
        assert_eq!(r[0].old_line, Some(1));
        assert_eq!(r[0].new_line, Some(1));
    }

    #[test]
    fn removed_row_has_only_old_number() {
        let r = rows(BASIC);
        let removed = r.iter().find(|x| x.marker == '-').unwrap();
        assert_eq!(removed.old_line, Some(2));
        assert_eq!(removed.new_line, None);
    }

    #[test]
    fn added_row_has_only_new_number() {
        let r = rows(BASIC);
        let added = r.iter().find(|x| x.marker == '+').unwrap();
        assert_eq!(added.old_line, None);
        assert_eq!(added.new_line, Some(2));
    }

    #[test]
    fn row_text_preserves_content() {
        let r = rows(BASIC);
        assert_eq!(r[0].text, "a");
    }

    #[test]
    fn empty_file_has_no_rows() {
        let files =
            parse("diff --git a/p.png b/p.png\nBinary files a/p.png and b/p.png differ\n").unwrap();
        assert_eq!(inline_rows(&files[0]).len(), 0);
    }

    #[test]
    fn render_includes_status_header() {
        let files = parse(BASIC).unwrap();
        let out = render_inline(&files);
        assert!(out.contains("modified  x"));
    }

    #[test]
    fn render_includes_hunk_separator() {
        let files = parse(BASIC).unwrap();
        let out = render_inline(&files);
        assert!(out.contains("@@ -1 +1 @@"));
    }

    #[test]
    fn render_marks_added_and_removed() {
        let files = parse(BASIC).unwrap();
        let out = render_inline(&files);
        assert!(out.contains("- b"));
        assert!(out.contains("+ B"));
    }

    #[test]
    fn render_empty_is_empty_string() {
        assert_eq!(render_inline(&[]), "");
    }

    #[test]
    fn num_pads_to_width_four() {
        assert_eq!(num(Some(7), 4), "   7");
        assert_eq!(num(Some(1234), 4), "1234");
        assert_eq!(num(Some(10000), 5), "10000");
        assert_eq!(num(Some(7), 5), "    7");
    }

    #[test]
    fn num_none_is_blank_width_four() {
        assert_eq!(num(None, 4), "    ");
        assert_eq!(num(None, 6), "      ");
    }

    #[test]
    fn marker_of_maps_kinds() {
        assert_eq!(marker_of(PatchLineKind::Context), ' ');
        assert_eq!(marker_of(PatchLineKind::Added), '+');
        assert_eq!(marker_of(PatchLineKind::Removed), '-');
    }

    #[test]
    fn multi_file_render_has_two_headers() {
        let input = "--- a/a\n+++ b/a\n@@ -1,1 +1,1 @@\n-a\n+A\n--- a/b\n+++ b/b\n@@ -1,1 +1,1 @@\n-b\n+B\n";
        let files = parse(input).unwrap();
        let out = render_inline(&files);
        assert_eq!(out.matches("modified  ").count(), 2);
    }

    #[test]
    fn multi_hunk_rows_concatenate() {
        let input = "--- a/x\n+++ b/x\n@@ -1,1 +1,1 @@\n-a\n+A\n@@ -9,1 +9,1 @@\n-b\n+B\n";
        let files = parse(input).unwrap();
        assert_eq!(inline_rows(&files[0]).len(), 4);
    }

    #[test]
    fn added_only_rows_have_no_old_number() {
        let input = "--- /dev/null\n+++ b/n\n@@ -0,0 +1,2 @@\n+one\n+two\n";
        let files = parse(input).unwrap();
        let r = inline_rows(&files[0]);
        assert!(r.iter().all(|row| row.old_line.is_none()));
    }

    #[test]
    fn blank_context_row_has_empty_text() {
        let input = "--- a/x\n+++ b/x\n@@ -1,2 +1,2 @@\n \n-b\n+B\n";
        let files = parse(input).unwrap();
        let r = inline_rows(&files[0]);
        assert_eq!(r[0].text, "");
        assert_eq!(r[0].marker, ' ');
    }

    #[test]
    fn render_row_columns_are_present() {
        let files = parse(BASIC).unwrap();
        let out = render_inline(&files);
        // context line 1/1 should render both numbers.
        assert!(out.contains("   1    1   a"));
    }

    /// Bug hunt 2026-11-01: a change the row text cannot show gets a note. Before, an
    /// end-of-file-newline-only change and a change between two non-UTF-8 bytes printed as two
    /// identical `-`/`+` rows.
    #[test]
    fn changes_the_text_cannot_show_get_a_note() {
        let nonl = "--- a/x.sh\n+++ b/x.sh\n@@ -1 +1 @@\n-rm -rf /tmp/cache\n\\ No newline at end of file\n+rm -rf /tmp/cache\n";
        let out = render_inline(&parse(nonl).unwrap());
        assert_eq!(
            out,
            "modified  x.sh\n  @@ -1 +1 @@\n     1      - rm -rf /tmp/cache\n            \\ No newline at end of file\n          1 + rm -rf /tmp/cache\n"
        );
        let lossy = b"--- a/x.txt\n+++ b/x.txt\n@@ -1 +1 @@\n-key=\xff\n+key=\xfe\n";
        let decoded = deep_diff_forge_patch::decode(lossy);
        let files = deep_diff_forge_patch::parse_decoded(
            &decoded,
            deep_diff_forge_patch::ParseOptions::default(),
        )
        .unwrap();
        let out = render_inline(&files);
        assert_eq!(out.matches(crate::notes::LOSSY).count(), 2, "{out}");
        let r = inline_rows(&files[0]);
        assert!(r.iter().all(|row| row.lossy && !row.no_newline));
        assert!(rows(nonl)[0].no_newline && !rows(nonl)[1].no_newline);
    }

    /// An added line that shows exactly as its removed pair is noted: a literal `\x1b` against
    /// a raw `ESC`, which escaping writes as `\x1b`. Unpaired and context lines are never noted.
    #[test]
    fn an_added_line_shown_exactly_as_its_pair_is_noted() {
        let input = "--- a/x\n+++ b/x\n@@ -1,4 +1,6 @@\n-a\\x1bb\n-same\n+a\u{1b}b\n+same2\n+a\u{1b}b\n a\\x1bb\n+lone\n-x\n+y\n";
        let out = render_inline(&parse(input).unwrap());
        let noted: Vec<&str> = out.lines().collect();
        assert_eq!(out.matches(crate::notes::RENDERS_ALIKE).count(), 1, "{out}");
        let at = noted
            .iter()
            .position(|l| l.contains(crate::notes::RENDERS_ALIKE))
            .unwrap();
        assert!(noted[at - 1].contains("+ a\\x1bb"), "{out}");
        assert_eq!(
            pairs(&parse(input).unwrap()[0].patch_twin.hunks[0].lines),
            [
                None,
                None,
                Some(0),
                Some(1),
                None,
                None,
                None,
                None,
                Some(7)
            ]
        );
    }

    /// Review 2026-11-01: inline prints tabs raw, so a pair looks alike by 8-column terminal
    /// tab stops from the text's column, not by the side-by-side layout's four spaces. Before,
    /// `\techo` against two spaces (both `  echo` at column 14) had no note, and `\techo`
    /// against four spaces, which a reader can tell apart, was noted. A trailing space and a
    /// no-break space against a space are invisible inline too.
    #[test]
    fn inline_notes_a_pair_by_what_the_terminal_shows() {
        let noted = |old: &str, new: &str| {
            let input = format!("--- a/x\n+++ b/x\n@@ -1 +1 @@\n-{old}\n+{new}\n");
            render_inline(&parse(&input).unwrap()).contains(crate::notes::RENDERS_ALIKE)
        };
        assert!(noted("\techo", "  echo"));
        assert!(!noted("\techo", "    echo"));
        assert!(noted("foo", "foo "));
        assert!(noted("foo", "foo\t"));
        assert!(noted("a b", "a\u{a0}b"));
        assert!(!noted("a b", "a  b"));
        assert!(!noted("a", "b"));
        // Past line 9999 the text starts at column 16, where a tab is eight spaces wide.
        let at = |old: &str, new: &str| {
            let input = format!("--- a/x\n+++ b/x\n@@ -10000 +10000 @@\n-{old}\n+{new}\n");
            render_inline(&parse(&input).unwrap()).contains(crate::notes::RENDERS_ALIKE)
        };
        assert!(at("\techo", "        echo"));
        assert!(!at("\techo", "  echo"));
    }

    /// Bug hunt 2026-11-01: every row of a file keeps its columns when line numbers reach
    /// five digits. Before, `{:>4}` widened only the rows at 10000 and later.
    #[test]
    fn rows_stay_aligned_across_line_ten_thousand() {
        let input = "--- a/x\n+++ b/x\n@@ -9998,4 +9998,4 @@\n a\n b\n-c\n+C\n d\n";
        let out = render_inline(&parse(input).unwrap());
        let body: Vec<&str> = out.lines().skip(2).collect();
        assert_eq!(
            body,
            [
                "   9998  9998   a",
                "   9999  9999   b",
                "  10000       - c",
                "        10000 + C",
                "  10001 10001   d",
            ]
        );
    }
}
