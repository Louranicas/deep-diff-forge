//! Tree-sitter syntax highlighting.
//!
//! Highlighting reuses the in-tree tree-sitter: it loads the grammar's own
//! `highlights.scm` query (via `tree_sitter_rust::HIGHLIGHTS_QUERY`) through the
//! existing `tree_sitter::Query` API — no extra dependency — and maps captures
//! to a small, renderer-neutral palette ([`HighlightClass`]).
//!
//! **Security.** The ANSI renderer routes every byte of attacker-controlled
//! source through [`deep_diff_forge_core::display_safe`], so the only raw escape
//! sequences in the output are the fixed SGR colour codes from
//! [`HighlightClass::ansi_sgr`]. Highlighting therefore cannot become a
//! terminal-injection vector, even on a malicious source file.

use crate::language::Language;
use std::fmt::Write as _;
use tree_sitter::{Query, QueryCursor, StreamingIterator as _};

/// A renderer-neutral highlight class. Mapping to concrete colours lives in
/// [`HighlightClass::ansi_sgr`] (terminal) and can be reused by other renderers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HighlightClass {
    /// Language keyword (`fn`, `let`, `match`, …).
    Keyword,
    /// Function / method / macro name.
    Function,
    /// Type name.
    Type,
    /// String / char literal, escape sequence.
    StringLit,
    /// Comment.
    Comment,
    /// Numeric literal.
    Number,
    /// Named constant.
    Constant,
    /// Attribute (`#[…]`).
    Attribute,
    /// Operator / punctuation.
    Operator,
    /// Variable / property / parameter.
    Variable,
    /// Unclassified (rendered without colour).
    Plain,
}

impl HighlightClass {
    /// Stable lowercase label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Keyword => "keyword",
            Self::Function => "function",
            Self::Type => "type",
            Self::StringLit => "string",
            Self::Comment => "comment",
            Self::Number => "number",
            Self::Constant => "constant",
            Self::Attribute => "attribute",
            Self::Operator => "operator",
            Self::Variable => "variable",
            Self::Plain => "plain",
        }
    }

    /// The ANSI SGR foreground parameter for this class. This is a fixed constant
    /// — never attacker-controlled — which is what keeps the ANSI renderer safe.
    #[must_use]
    pub fn ansi_sgr(self) -> &'static str {
        match self {
            Self::Keyword => "35",                                   // magenta
            Self::Function => "34",                                  // blue
            Self::Type => "36",                                      // cyan
            Self::StringLit => "32",                                 // green
            Self::Comment => "90",                                   // bright black
            Self::Number | Self::Constant | Self::Attribute => "33", // yellow
            Self::Operator => "37",                                  // white
            Self::Variable | Self::Plain => "39",                    // default fg
        }
    }

    /// Map a tree-sitter highlight capture name (e.g. `function.macro`) to a
    /// class by its leading segment.
    #[must_use]
    fn from_capture(name: &str) -> Self {
        match name.split('.').next().unwrap_or(name) {
            "keyword" => Self::Keyword,
            "function" | "constructor" => Self::Function,
            "type" => Self::Type,
            "string" | "escape" | "char" => Self::StringLit,
            "comment" => Self::Comment,
            "number" | "float" | "integer" => Self::Number,
            "constant" => Self::Constant,
            "attribute" => Self::Attribute,
            "operator" | "punctuation" => Self::Operator,
            "variable" | "property" | "label" => Self::Variable,
            _ => Self::Plain,
        }
    }
}

/// A highlighted source span: a byte range and its class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HighlightSpan {
    /// Inclusive start byte offset into the source.
    pub start: usize,
    /// Exclusive end byte offset into the source.
    pub end: usize,
    /// Highlight class for this span.
    pub class: HighlightClass,
}

/// Compute non-overlapping, sorted highlight spans for `source` in `language`.
///
/// Returns an empty vector for an unsupported language or unparsable input
/// (highlighting is best-effort and never an error).
#[must_use]
pub fn highlight(language: Language, source: &str) -> Vec<HighlightSpan> {
    if language != Language::Rust {
        return Vec::new();
    }
    highlight_rust(source)
}

fn highlight_rust(source: &str) -> Vec<HighlightSpan> {
    let language: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&language).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };
    let Ok(query) = Query::new(&language, tree_sitter_rust::HIGHLIGHTS_QUERY) else {
        return Vec::new();
    };
    let names = query.capture_names();
    let mut raw: Vec<HighlightSpan> = Vec::new();
    let mut cursor = QueryCursor::new();
    let mut caps = cursor.captures(&query, tree.root_node(), source.as_bytes());
    while let Some((m, idx)) = caps.next() {
        let cap = m.captures[*idx];
        let Some(name) = names.get(cap.index as usize) else {
            continue;
        };
        let class = HighlightClass::from_capture(name);
        if class == HighlightClass::Plain {
            continue;
        }
        raw.push(HighlightSpan {
            start: cap.node.start_byte(),
            end: cap.node.end_byte(),
            class,
        });
    }
    flatten(raw)
}

/// Resolve overlapping captures into non-overlapping, sorted spans.
///
/// Sort by `(start asc, length asc)` then greedily emit, skipping anything that
/// overlaps an already-emitted span — so the most-specific (shortest) capture at
/// a given start position wins. Gaps are left for the renderer to fill plainly.
fn flatten(mut raw: Vec<HighlightSpan>) -> Vec<HighlightSpan> {
    raw.sort_by_key(|s| (s.start, s.end.saturating_sub(s.start)));
    let mut out: Vec<HighlightSpan> = Vec::with_capacity(raw.len());
    let mut cursor = 0usize;
    for span in raw {
        if span.start >= cursor && span.end > span.start {
            out.push(span);
            cursor = span.end;
        }
    }
    out
}

/// Render `source` to an ANSI-coloured string using `spans`.
///
/// Every byte of `source` is passed through [`deep_diff_forge_core::display_safe`]
/// (so embedded control sequences are neutralised); the only raw escapes emitted
/// are the fixed SGR codes wrapping each span. `spans` must be the non-overlapping,
/// sorted output of [`highlight`].
#[must_use]
pub fn to_ansi(source: &str, spans: &[HighlightSpan]) -> String {
    use deep_diff_forge_core::display_safe;
    let mut out = String::with_capacity(source.len() + spans.len() * 12);
    let mut cursor = 0usize;
    for span in spans {
        if span.start > cursor {
            if let Some(text) = source.get(cursor..span.start) {
                out.push_str(&display_safe(text));
            }
        }
        if let Some(text) = source.get(span.start..span.end) {
            let _ = write!(
                out,
                "\u{1b}[{}m{}\u{1b}[0m",
                span.class.ansi_sgr(),
                display_safe(text)
            );
        }
        cursor = span.end.max(cursor);
    }
    if let Some(rest) = source.get(cursor..) {
        out.push_str(&display_safe(rest));
    }
    out
}

/// Highlight `source` to an ANSI string for `language`.
///
/// Unsupported / unparsable input is returned terminal-safe but uncoloured, so a
/// caller can always print the result without a fallback branch.
#[must_use]
pub fn highlight_to_ansi(language: Language, source: &str) -> String {
    let spans = highlight(language, source);
    if spans.is_empty() {
        return deep_diff_forge_core::display_safe(source).into_owned();
    }
    to_ansi(source, &spans)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_capture_maps_leading_segment() {
        assert_eq!(
            HighlightClass::from_capture("keyword"),
            HighlightClass::Keyword
        );
        assert_eq!(
            HighlightClass::from_capture("function.macro"),
            HighlightClass::Function
        );
        assert_eq!(
            HighlightClass::from_capture("type.builtin"),
            HighlightClass::Type
        );
        assert_eq!(
            HighlightClass::from_capture("string"),
            HighlightClass::StringLit
        );
        assert_eq!(
            HighlightClass::from_capture("comment.documentation"),
            HighlightClass::Comment
        );
        assert_eq!(
            HighlightClass::from_capture("nonsense"),
            HighlightClass::Plain
        );
    }

    #[test]
    fn ansi_sgr_is_a_fixed_numeric_code() {
        for class in [
            HighlightClass::Keyword,
            HighlightClass::Function,
            HighlightClass::Type,
            HighlightClass::StringLit,
            HighlightClass::Comment,
            HighlightClass::Plain,
        ] {
            assert!(class.ansi_sgr().chars().all(|c| c.is_ascii_digit()));
        }
    }

    #[test]
    fn highlights_rust_keywords_and_strings() {
        let spans = highlight(Language::Rust, "fn main() { let s = \"hi\"; }");
        assert!(!spans.is_empty());
        // `fn` should be a keyword span at byte 0.
        assert!(
            spans
                .iter()
                .any(|s| s.start == 0 && s.class == HighlightClass::Keyword)
        );
        // a string literal should be classified.
        assert!(spans.iter().any(|s| s.class == HighlightClass::StringLit));
    }

    #[test]
    fn spans_are_sorted_and_non_overlapping() {
        let spans = highlight(Language::Rust, "fn f<T: Copy>(x: T) -> T { x }");
        let mut last_end = 0;
        for s in &spans {
            assert!(s.start >= last_end, "spans must not overlap");
            assert!(s.end > s.start);
            last_end = s.end;
        }
    }

    #[test]
    fn unsupported_language_yields_no_spans() {
        assert!(highlight(Language::Unsupported, "fn main() {}").is_empty());
    }

    #[test]
    fn to_ansi_neutralizes_source_escape_sequences() {
        // A string literal containing a raw ESC must not leak it to the terminal:
        // only our SGR codes are raw ESC; the source ESC is escaped to \x1b.
        let source = "fn main() { let s = \"\u{1b}[2Jpwn\"; }";
        let out = highlight_to_ansi(Language::Rust, source);
        // The source's ESC (inside the string) is rendered as the literal \x1b.
        assert!(out.contains("\\x1b[2Jpwn"));
        // Every raw ESC in the output is immediately a `[<digits>m` SGR opener or
        // the `[0m` reset — never a bare attacker sequence.
        for (i, _) in out.match_indices('\u{1b}') {
            let tail = &out[i + 1..];
            assert!(tail.starts_with('['), "raw ESC must introduce an SGR code");
        }
    }

    #[test]
    fn to_ansi_round_trips_plain_text_content() {
        let source = "fn answer() -> u32 { 42 }";
        let out = highlight_to_ansi(Language::Rust, source);
        // The identifiers/keywords survive (colour codes added around them).
        assert!(out.contains("answer"));
        assert!(out.contains("42"));
    }

    #[test]
    fn flatten_drops_overlaps_keeping_specific() {
        let raw = vec![
            HighlightSpan {
                start: 0,
                end: 10,
                class: HighlightClass::Type,
            },
            HighlightSpan {
                start: 0,
                end: 3,
                class: HighlightClass::Keyword,
            },
            HighlightSpan {
                start: 5,
                end: 8,
                class: HighlightClass::Function,
            },
        ];
        let flat = flatten(raw);
        // (0,3 keyword) wins at 0 (shorter), then (5,8) — (0,10) is dropped.
        assert_eq!(flat.len(), 2);
        assert_eq!(
            flat[0],
            HighlightSpan {
                start: 0,
                end: 3,
                class: HighlightClass::Keyword
            }
        );
        assert_eq!(flat[1].start, 5);
    }

    #[test]
    fn empty_source_is_empty() {
        assert!(highlight(Language::Rust, "").is_empty());
        assert_eq!(highlight_to_ansi(Language::Rust, ""), "");
    }
}
