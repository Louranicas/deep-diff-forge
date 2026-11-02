// DDF integration: R31, R32. Measure displayed cells and preserve grapheme boundaries after terminal-safety escaping; keep remaining chrome/wrap geometry visible as work.
// Design/status: docs/RECOMMENDATION_INTEGRATION.md#terminal-geometry
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fdeep-diff-forge-projection--width
//! Shared terminal-cell geometry (R31), not UTF-8 bytes or Unicode scalar counts.
//! See `docs/RECOMMENDATION_INTEGRATION.md#terminal-geometry` and its vault backlink.
use std::borrow::Cow;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;
/// Cell width of already terminal-safe text.
#[must_use]
pub fn cell_width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}
/// Expand each tab to four spaces, the one tab rule every renderer shares, so geometry is
/// independent of terminal stops. A renderer must not hand a raw tab to a terminal cell: ratatui
/// drops control characters, so an unexpanded tab vanishes and an indentation change with it.
#[must_use]
pub fn expand_tabs(text: &str) -> Cow<'_, str> {
    if text.contains('\t') {
        Cow::Owned(text.replace('\t', "    "))
    } else {
        Cow::Borrowed(text)
    }
}
/// The longest prefix of already-safe text that fits in `width` cells, without splitting a
/// grapheme and without padding. Tabs are expanded first ([`expand_tabs`]).
#[must_use]
pub fn clip_cells(text: &str, width: usize) -> String {
    let mut out = String::new();
    let mut used = 0;
    for grapheme in expand_tabs(text).graphemes(true) {
        let cells = cell_width(grapheme);
        if used + cells > width {
            break;
        }
        out.push_str(grapheme);
        used += cells;
    }
    out
}
/// Fit already-safe text to exactly `width` cells without splitting a grapheme.
/// Tabs are expanded to four spaces so geometry is independent of terminal stops.
#[must_use]
pub fn fit_cells(text: &str, width: usize) -> String {
    let mut out = clip_cells(text, width);
    let used = cell_width(&out);
    out.push_str(&" ".repeat(width - used));
    out
}
/// Hard-wrap safe text in terminal cells. Oversized graphemes become `?` when
/// the viewport cannot hold them; the patch model itself is never changed.
#[must_use]
pub fn wrap_cells(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let expanded = expand_tabs(text);
    let mut result = Vec::new();
    let mut line = String::new();
    let mut used = 0;
    for grapheme in expanded.graphemes(true) {
        let cells = cell_width(grapheme);
        if used + cells > width && !line.is_empty() {
            result.push(std::mem::take(&mut line));
            used = 0;
        }
        if cells > width {
            line.push('?');
            used += 1;
        } else {
            line.push_str(grapheme);
            used += cells;
        }
    }
    if !line.is_empty() || result.is_empty() {
        result.push(line);
    }
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cjk_combining_emoji_and_tabs_fit_cells() {
        for text in [
            "界".repeat(60),
            "e\u{301}".repeat(80),
            "👨‍👩‍👦".repeat(40),
            "a\tb".into(),
        ] {
            assert_eq!(cell_width(&fit_cells(&text, 60)), 60);
            assert!(
                wrap_cells(&text, 17)
                    .iter()
                    .all(|line| cell_width(line) <= 17)
            );
        }
        assert_eq!(fit_cells("界x", 1), " ");
        assert_eq!(clip_cells("界x", 1), "");
        assert_eq!(clip_cells("界x", 2), "界");
        assert_eq!(clip_cells("a\tb", 3), "a  ");
        assert_eq!(clip_cells("abc", 9), "abc");
        assert_eq!(wrap_cells("界x", 1), vec!["?", "x"]);
    }
    #[test]
    fn expand_tabs_borrows_tab_free_text() {
        assert!(matches!(expand_tabs("a b"), Cow::Borrowed("a b")));
        assert_eq!(expand_tabs("\tif a:\t"), "    if a:    ");
    }
}
