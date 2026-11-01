//! What a projected row's text cannot show, as `\` note rows, and the gutter width that keeps a
//! file's rows aligned. Shared by the inline and side-by-side layouts so neither can show a
//! changed line as an unchanged-looking pair.
use deep_diff_forge_core::{PatchLine, ReviewFile, display_safe};

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

/// Whether two lines with different bytes would show the same text: after terminal-safety
/// escaping and the layouts' tab expansion (a tab against four spaces, or a literal `\x1b`
/// against a raw `ESC`, which escaping writes as `\x1b`).
pub fn renders_alike(old: &str, new: &str) -> bool {
    old != new && shown(old) == shown(new)
}

fn shown(text: &str) -> String {
    display_safe(text).replace('\t', "    ")
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
        assert!(renders_alike("\techo hi", "    echo hi"));
        assert!(renders_alike("a\\x1bb", "a\u{1b}b"));
        assert!(!renders_alike("same", "same"));
        assert!(!renders_alike("\techo", "   echo"));
        assert!(!renders_alike("a", "b"));
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
