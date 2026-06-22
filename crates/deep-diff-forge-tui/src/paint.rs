//! Terminal-safe syntax painting: source text → coloured ratatui [`Span`]s.
//!
//! This is the one place where the [`deep_diff_forge_syntax`] highlight spans
//! meet the renderer. The security invariant is strict: a coloured span is only
//! ever built from text that [`display_safe`] left **byte-for-byte unchanged**.
//! If a line contains any terminal-unsafe control character or a Trojan-Source
//! bidi/invisible codepoint, the entire line is emitted as one escaped,
//! unstyled span — colouring never gets the chance to reintroduce a raw escape.

use crate::theme::Palette;
use deep_diff_forge_core::display_safe;
use deep_diff_forge_syntax::{Language, highlight};
use ratatui::style::Style;
use ratatui::text::Span;
use std::borrow::Cow;

/// A single, fully-escaped span for arbitrary untrusted text (paths, labels).
#[must_use]
pub fn safe_span(raw: &str, style: Style) -> Span<'static> {
    Span::styled(display_safe(raw).into_owned(), style)
}

/// Convert one source line into themed, terminal-safe spans over `base`.
///
/// `base` carries the row's background tint (e.g. the added/removed gutter
/// colour); each highlighted token overrides only the foreground, so the tint
/// survives across the whole line.
#[must_use]
pub fn themed_spans(
    language: Language,
    raw: &str,
    palette: &Palette,
    base: Style,
) -> Vec<Span<'static>> {
    match display_safe(raw) {
        // Unsafe content: never colour it — show the escaped form, unstyled.
        Cow::Owned(escaped) => vec![Span::styled(escaped, base)],
        // Clean content: safe to colour from the raw (== safe) text.
        Cow::Borrowed(safe) => color_clean(language, safe, palette, base),
    }
}

fn color_clean(
    language: Language,
    text: &str,
    palette: &Palette,
    base: Style,
) -> Vec<Span<'static>> {
    let spans = highlight(language, text);
    if spans.is_empty() {
        return vec![Span::styled(text.to_string(), base)];
    }
    let mut out = Vec::with_capacity(spans.len() * 2 + 1);
    let mut cursor = 0usize;
    for hs in spans {
        // Defensive against unsorted/overlapping/out-of-range spans: skip rather
        // than panic. `str::get` returns None on a non-char boundary.
        if hs.start < cursor || hs.start > hs.end || hs.end > text.len() {
            continue;
        }
        if hs.start > cursor {
            if let Some(gap) = text.get(cursor..hs.start) {
                out.push(Span::styled(gap.to_string(), base));
            }
        }
        if let Some(tok) = text.get(hs.start..hs.end) {
            let color = palette.class_color(hs.class);
            out.push(Span::styled(tok.to_string(), base.fg(color)));
            cursor = hs.end;
        }
    }
    if cursor < text.len() {
        if let Some(rest) = text.get(cursor..) {
            out.push(Span::styled(rest.to_string(), base));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::ThemeKind;

    fn palette() -> Palette {
        ThemeKind::Dark.palette()
    }

    fn joined(spans: &[Span<'static>]) -> String {
        spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn clean_text_round_trips_content() {
        let p = palette();
        let spans = themed_spans(Language::Rust, "let x = 1;", &p, Style::default());
        assert_eq!(joined(&spans), "let x = 1;");
    }

    #[test]
    fn keyword_gets_its_own_coloured_span() {
        let p = palette();
        let spans = themed_spans(Language::Rust, "fn main() {}", &p, Style::default());
        // "fn" is a keyword; there must be a span whose fg is the keyword colour.
        let kw = p.class_color(deep_diff_forge_syntax::HighlightClass::Keyword);
        assert!(spans.iter().any(|s| s.style.fg == Some(kw)));
    }

    #[test]
    fn raw_escape_never_survives() {
        let p = palette();
        let evil = "let x = \u{1b}[2J;";
        let spans = themed_spans(Language::Rust, evil, &p, Style::default());
        let out = joined(&spans);
        assert!(!out.contains('\u{1b}'), "raw ESC leaked through painting");
        assert!(out.contains("\\x1b"), "escaped form should be shown");
    }

    #[test]
    fn unsafe_line_is_a_single_unstyled_span() {
        let p = palette();
        let spans = themed_spans(Language::Rust, "a\u{1b}b", &p, Style::default());
        assert_eq!(spans.len(), 1);
    }

    #[test]
    fn unsupported_language_is_plain_single_span() {
        let p = palette();
        let spans = themed_spans(Language::Unsupported, "let x = 1;", &p, Style::default());
        assert_eq!(spans.len(), 1);
        assert_eq!(joined(&spans), "let x = 1;");
    }

    #[test]
    fn base_background_is_preserved_on_tokens() {
        let p = palette();
        let base = Style::default().bg(p.added_bg);
        let spans = themed_spans(Language::Rust, "fn f() {}", &p, base);
        assert!(spans.iter().all(|s| s.style.bg == Some(p.added_bg)));
    }

    #[test]
    fn empty_line_yields_one_span() {
        let p = palette();
        let spans = themed_spans(Language::Rust, "", &p, Style::default());
        assert_eq!(joined(&spans), "");
    }

    #[test]
    fn safe_span_escapes_paths() {
        let s = safe_span("a/\u{1b}b.rs", Style::default());
        assert!(!s.content.contains('\u{1b}'));
    }

    #[test]
    fn content_is_fully_reconstructable_with_unicode() {
        let p = palette();
        let src = "let s = \"café→\"; // 注释";
        let spans = themed_spans(Language::Rust, src, &p, Style::default());
        assert_eq!(joined(&spans), src);
    }
}
