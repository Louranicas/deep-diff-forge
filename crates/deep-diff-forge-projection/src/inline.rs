use deep_diff_forge_core::{PatchLineKind, ReviewFile, display_safe};
use std::fmt::Write as _;

/// One inline display row: old/new line numbers, a marker, and the text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineRow {
    /// Old-side line number, if the row exists on the old side.
    pub old_line: Option<u32>,
    /// New-side line number, if the row exists on the new side.
    pub new_line: Option<u32>,
    /// `' '` context, `'+'` added, `'-'` removed.
    pub marker: char,
    /// Row text (without the leading marker).
    pub text: String,
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

fn num(value: Option<u32>) -> String {
    value.map_or_else(|| "    ".to_string(), |n| format!("{n:>4}"))
}

/// Render every file inline: a status/path header, per-hunk separators, and
/// `old new marker text` rows. Patch truth is read-only here.
#[must_use]
pub fn render_inline(files: &[ReviewFile]) -> String {
    let mut out = String::new();
    for file in files {
        let status = crate::status_label(file.status);
        // Paths and line text are attacker-controlled; neutralise terminal
        // escapes before they reach a reviewer's terminal.
        let _ = writeln!(out, "{status}  {}", display_safe(&file.path));
        for hunk in &file.patch_twin.hunks {
            let old_start = hunk.old_start.unwrap_or(0);
            let new_start = hunk.new_start.unwrap_or(0);
            let _ = writeln!(out, "  @@ -{old_start} +{new_start} @@");
            for line in &hunk.lines {
                let _ = writeln!(
                    out,
                    "  {} {} {} {}",
                    num(line.old_line),
                    num(line.new_line),
                    marker_of(line.kind),
                    display_safe(&line.text)
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
        assert!(inline_rows(&files[0]).is_empty());
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
        assert_eq!(num(Some(7)), "   7");
        assert_eq!(num(Some(1234)), "1234");
    }

    #[test]
    fn num_none_is_blank_width_four() {
        assert_eq!(num(None), "    ");
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
}
