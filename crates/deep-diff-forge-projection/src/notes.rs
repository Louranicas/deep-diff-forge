//! What a projected row's text cannot show, as `\` note rows, and the gutter width that keeps a
//! file's rows aligned. Shared by the inline and side-by-side layouts, each judging a pair by the
//! cells it prints, so neither shows a changed line as an unchanged-looking pair without a note
//! (look-alike letters excepted: see [`renders_alike`]).
use crate::{cell_width, fit_cells};
use deep_diff_forge_core::{PatchLine, ReviewFile, display_safe};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthChar;

/// Note after a line that was not UTF-8 in the patch: its text is the lossy decoding.
pub const LOSSY: &str = "\\ non-UTF-8 line, shown lossily";
/// Note after a line without a terminating newline (Git's own wording).
pub const NO_NEWLINE: &str = "\\ No newline at end of file";
/// Note after a removed and an added line whose bytes differ but whose display is the same.
pub const RENDERS_ALIKE: &str = "\\ differs from its pair only in bytes shown alike";

/// The flag notes of one line, in display order.
pub fn line_notes(lossy: bool, no_newline: bool) -> impl Iterator<Item = &'static str> {
    [(lossy, LOSSY), (no_newline, NO_NEWLINE)]
        .into_iter()
        .filter_map(|(set, note)| set.then_some(note))
}

/// Terminal tab stops, which the inline layout's raw tabs advance to.
const TAB_STOP: usize = 8;

/// Whether two lines with different bytes look the same once `show` has turned each into the
/// cells a layout prints. `show` applies the layout's own escaping, tab model and cut; on top of
/// it, a Unicode space separator is as blank as that many spaces (a no-break space against a
/// space), and trailing blanks print as nothing (a trailing space, or a side-by-side cell's
/// padding). So a tab against the spaces it advances to, a literal `\x1b` against a raw `ESC`
/// (which escaping writes as `\x1b`) and two lines that differ only past a cut all render alike.
/// Letters that merely look alike (confusables such as Cyrillic `а` against Latin `a`) are not
/// detected: they are different glyphs, not the same cells.
pub fn renders_alike(old: &str, new: &str, show: impl Fn(&str) -> String) -> bool {
    old != new && seen(&show(old)) == seen(&show(new))
}

/// What a reader sees of printed cells: space separators as blanks of their width, without the
/// trailing blanks.
fn seen(cells: &str) -> String {
    let mut out = String::new();
    for c in cells.chars() {
        if is_space_separator(c) {
            out.push_str(&" ".repeat(UnicodeWidthChar::width(c).unwrap_or(1)));
        } else {
            out.push(c);
        }
    }
    out.truncate(out.trim_end_matches(' ').len());
    out
}

/// Unicode's `Space_Separator` (`Zs`) characters, which `display_safe` passes through.
fn is_space_separator(c: char) -> bool {
    matches!(
        c,
        ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}' | '\u{202f}' | '\u{205f}' | '\u{3000}'
    )
}

/// The inline layout's printed text: terminal-safe, with each tab expanded to the next 8-column
/// tab stop counted from `column`, the screen column the text starts at.
pub fn inline_shown(text: &str, column: usize) -> String {
    let mut out = String::new();
    let mut at = column;
    for grapheme in display_safe(text).graphemes(true) {
        if grapheme == "\t" {
            let advance = TAB_STOP - at % TAB_STOP;
            out.push_str(&" ".repeat(advance));
            at += advance;
        } else {
            out.push_str(grapheme);
            at += cell_width(grapheme);
        }
    }
    out
}

/// The side-by-side layout's printed cell: terminal-safe, tabs as four spaces, cut and padded
/// to `width` cells.
pub fn side_shown(text: &str, width: usize) -> String {
    fit_cells(&display_safe(text), width)
}

/// The width of a file's line-number fields: at least 4, and wide enough for its largest line
/// number, so a hunk that crosses 9999 keeps every row's columns where they were.
pub fn gutter_width(file: &ReviewFile) -> usize {
    file.patch_twin
        .hunks
        .iter()
        .flat_map(|hunk| &hunk.lines)
        .flat_map(|line: &PatchLine| [line.old_line, line.new_line])
        .flatten()
        .max()
        .map_or(4, |n| n.to_string().len().max(4))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_follow_the_flags_in_order() {
        assert_eq!(line_notes(false, false).count(), 0);
        assert_eq!(line_notes(true, false).collect::<Vec<_>>(), [LOSSY]);
        assert_eq!(line_notes(false, true).collect::<Vec<_>>(), [NO_NEWLINE]);
        assert_eq!(
            line_notes(true, true).collect::<Vec<_>>(),
            [LOSSY, NO_NEWLINE]
        );
    }

    #[test]
    fn only_different_bytes_with_the_same_display_render_alike() {
        let side = |t: &str| side_shown(t, 60);
        assert!(renders_alike("\techo hi", "    echo hi", side));
        assert!(renders_alike("a\\x1bb", "a\u{1b}b", side));
        assert!(!renders_alike("same", "same", side));
        assert!(!renders_alike("\techo", "   echo", side));
        assert!(!renders_alike("a", "b", side));
        // Blank cells: separators by their width, trailing blanks not at all.
        assert!(renders_alike("a b", "a\u{2003}b", side));
        assert!(renders_alike("a  b", "a\u{3000}b", side));
        assert!(!renders_alike("a b", "a\u{3000}b", side));
        assert!(renders_alike("x", "x \u{a0}", side));
        assert!(!renders_alike("x", "x.", side));
        assert!(
            !renders_alike("\u{430}", "a", side),
            "confusables are not detected"
        );
    }

    #[test]
    fn inline_tabs_advance_to_the_next_eight_column_stop() {
        assert_eq!(inline_shown("\tx", 14), "  x");
        assert_eq!(inline_shown("\tx", 16), "        x");
        assert_eq!(inline_shown("\tx", 0), "        x");
        assert_eq!(inline_shown("a\tx", 14), "a x");
        assert_eq!(inline_shown("\t\tx", 14), format!("{}x", " ".repeat(10)));
        assert_eq!(inline_shown("ab\tx", 14), "ab        x");
        assert_eq!(inline_shown("界\tx", 13), "界 x");
        assert_eq!(inline_shown("\u{1b}\tx", 0), "\\x1b    x");
        assert_eq!(side_shown("\tx", 6), "    x ");
    }

    #[test]
    fn the_gutter_fits_the_largest_line_number() {
        let parse = |text: &str| deep_diff_forge_patch::parse(text).unwrap();
        let small = parse("--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n");
        assert_eq!(gutter_width(&small[0]), 4);
        let edge = parse("--- a/x\n+++ b/x\n@@ -9999 +9999 @@\n-a\n+b\n");
        assert_eq!(gutter_width(&edge[0]), 4);
        let over = parse("--- a/x\n+++ b/x\n@@ -9999,2 +9999,1 @@\n-a\n-b\n+c\n");
        assert_eq!(gutter_width(&over[0]), 5, "the old side reaches 10000");
        let new_side = parse("--- a/x\n+++ b/x\n@@ -1 +99999,2 @@\n-a\n+b\n+c\n");
        assert_eq!(gutter_width(&new_side[0]), 6, "the new side reaches 100000");
        let binary = parse("diff --git a/p.png b/p.png\nBinary files a/p.png and b/p.png differ\n");
        assert_eq!(gutter_width(&binary[0]), 4);
    }
}
