// DDF integration: R08, R10, R11. Never turn a failed analysis into formatting-only or behavioral safety; parser time cancellation is cooperative, node limits are post-parse.
// Design/status: docs/RECOMMENDATION_INTEGRATION.md#syntax-availability
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fdeep-diff-forge-syntax--highlight
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
use std::cell::RefCell;
use std::fmt::Write as _;
use tree_sitter::{Query, QueryCursor, StreamingIterator as _};

enum QueryState {
    Untried,
    Unavailable,
    Ready(Query),
}

thread_local! {
    /// The compiled Rust highlight query, cached per thread.
    ///
    /// `Query::new` parses and compiles the grammar's entire `highlights.scm`.
    /// The review TUI calls [`highlight`] once per rendered source line and
    /// rebuilds the whole diff document every frame, so recompiling the query on
    /// every call made rendering pathologically slow (seconds per keypress on a
    /// multi-file diff). Compiling once and reusing the immutable query removes
    /// that cost; `Unavailable` means compilation failed and highlighting is skipped.
    static RUST_QUERY: RefCell<QueryState> = const { RefCell::new(QueryState::Untried) };
}

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
    // Guard against unbounded parse cost; mirrors the byte_budget in analyze().
    if source.len() > crate::analyze::DEFAULT_BYTE_BUDGET {
        return Vec::new();
    }
    let language: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&language).is_err() {
        return Vec::new();
    }
    let Some(tree) = crate::parse_bounded(&mut parser, source) else {
        return Vec::new();
    };
    // The highlight query's cost grows with nesting depth, not size: 240,010 nodes laid out flat
    // highlight in 0.14 s, nested 60,000 deep in 20 s (release build, bug hunt H4-04). Bound
    // both, as `semantic` bounds nodes, and fall back to plain text past either.
    if exceeds_bounds(
        &tree,
        crate::analyze::DEFAULT_NODE_BUDGET,
        MAX_HIGHLIGHT_DEPTH,
    ) {
        return Vec::new();
    }
    RUST_QUERY.with(|cell| {
        let mut slot = cell.borrow_mut();
        // Compile the highlight query at most once per thread, then reuse it.
        if matches!(*slot, QueryState::Untried) {
            *slot = match Query::new(&language, tree_sitter_rust::HIGHLIGHTS_QUERY) {
                Ok(query) => QueryState::Ready(query),
                Err(_) => QueryState::Unavailable,
            };
        }
        let QueryState::Ready(query) = &*slot else {
            return Vec::new();
        };
        let names = query.capture_names();
        let mut raw: Vec<HighlightSpan> = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut caps = cursor.captures(query, tree.root_node(), source.as_bytes());
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
    })
}

/// The deepest syntax tree `highlight` queries. Real code nests a few dozen levels; 8,000
/// levels highlight in about 25 ms, and the cost climbs steeply past that.
const MAX_HIGHLIGHT_DEPTH: usize = 4096;

/// Whether `tree` has more than `node_budget` nodes or nests more than `depth_limit` levels.
fn exceeds_bounds(tree: &tree_sitter::Tree, node_budget: usize, depth_limit: usize) -> bool {
    tree.root_node().descendant_count() > node_budget || deeper_than(tree, depth_limit)
}

/// Whether any node of `tree` lies more than `limit` levels below the root. An explicit stack
/// visits each node once and stops at the first one past the limit.
fn deeper_than(tree: &tree_sitter::Tree, limit: usize) -> bool {
    let mut stack = vec![(tree.root_node(), 0usize)];
    while let Some((node, depth)) = stack.pop() {
        if depth > limit {
            return true;
        }
        let mut cursor = node.walk();
        stack.extend(node.children(&mut cursor).map(|child| (child, depth + 1)));
    }
    false
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

/// [`deep_diff_forge_core::display_safe`] for each line of `text`, keeping the line
/// feeds themselves. A line feed only moves to the next line, so it is safe to print,
/// and escaping it (as `\x0a`) printed a whole source file on one line. Every other
/// control character, including a carriage return, is still escaped.
#[must_use]
pub fn display_safe_lines(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&deep_diff_forge_core::display_safe(line));
    }
    out
}

/// Render `source` to an ANSI-coloured string using `spans`.
///
/// Every line of `source` is passed through [`deep_diff_forge_core::display_safe`]
/// (so embedded control sequences are neutralised); the only raw bytes it adds are
/// the line feeds between lines and the fixed SGR codes wrapping each span. `spans`
/// must be the non-overlapping, sorted output of [`highlight`].
#[must_use]
pub fn to_ansi(source: &str, spans: &[HighlightSpan]) -> String {
    let mut out = String::with_capacity(source.len() + spans.len() * 12);
    let mut cursor = 0usize;
    for span in spans {
        if span.start > cursor
            && let Some(text) = source.get(cursor..span.start)
        {
            out.push_str(&display_safe_lines(text));
        }
        if let Some(text) = source.get(span.start..span.end) {
            let _ = write!(
                out,
                "\u{1b}[{}m{}\u{1b}[0m",
                span.class.ansi_sgr(),
                display_safe_lines(text)
            );
        }
        cursor = span.end.max(cursor);
    }
    if let Some(rest) = source.get(cursor..) {
        out.push_str(&display_safe_lines(rest));
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
        return display_safe_lines(source);
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
        assert_ne!(spans.len(), 0);
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
        assert_eq!(highlight(Language::Unsupported, "fn main() {}").len(), 0);
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

    /// Line feeds print as line breaks, coloured or not; every other control character is
    /// still escaped. Before, each line feed printed as the text `\x0a`, so a file came out
    /// on one line.
    #[test]
    fn line_feeds_stay_line_breaks_and_other_controls_are_escaped() {
        let source = "fn a() {}\n// \r\u{7}\nfn b() {}\n";
        for out in [
            highlight_to_ansi(Language::Rust, source),
            highlight_to_ansi(Language::Unsupported, source),
        ] {
            assert_eq!(out.matches('\n').count(), 3, "{out:?}");
            assert!(!out.contains("\\x0a"), "{out:?}");
            assert!(out.contains("\\x0d\\x07"), "{out:?}");
            assert!(!out.contains('\r') && !out.contains('\u{7}'), "{out:?}");
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

    fn parse(source: &str) -> tree_sitter::Tree {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .unwrap();
        parser.parse(source, None).unwrap()
    }

    /// `source_file > function_item > block > "{"` is three levels below the root.
    #[test]
    fn depth_is_measured_exactly() {
        let tree = parse("fn f(){}");
        assert!(!deeper_than(&tree, 3));
        assert!(deeper_than(&tree, 2));
        assert!(!deeper_than(&parse(""), 0));
        // The node budget is a strict bound too.
        let nodes = tree.root_node().descendant_count();
        assert!(!exceeds_bounds(&tree, nodes, 3));
        assert!(exceeds_bounds(&tree, nodes - 1, 3));
        assert!(exceeds_bounds(&tree, nodes, 2));
    }

    /// Bug hunt 2026-10-28, H4-04: a 60,000-deep nest took 20 s (release) to highlight. Past
    /// the depth or node bound it is plain text; ordinary nesting is still highlighted.
    #[test]
    fn deep_or_huge_trees_fall_back_to_plain_text() {
        let nest = |n: usize| format!("fn f(){{{}{}}}", "{".repeat(n), "}".repeat(n));
        assert_ne!(highlight(Language::Rust, &nest(1000)).len(), 0);
        assert_eq!(
            highlight(Language::Rust, &nest(MAX_HIGHLIGHT_DEPTH)).len(),
            0
        );
        assert_eq!(highlight(Language::Rust, &nest(60_000)).len(), 0);
        let flat = format!("fn f(){{{}}}", "{}".repeat(60_000));
        assert_eq!(highlight(Language::Rust, &flat).len(), 0);
        let flat_small = format!("fn f(){{{}}}", "{}".repeat(1_000));
        assert_ne!(highlight(Language::Rust, &flat_small).len(), 0);
    }

    #[test]
    fn empty_source_is_empty() {
        assert_eq!(highlight(Language::Rust, "").len(), 0);
        assert_eq!(highlight_to_ansi(Language::Rust, ""), "");
    }

    // Pins for the 2026-10-31 whole-crate mutation run.

    #[test]
    fn every_class_has_its_label_and_colour() {
        use HighlightClass as H;
        for (class, label, sgr) in [
            (H::Keyword, "keyword", "35"),
            (H::Function, "function", "34"),
            (H::Type, "type", "36"),
            (H::StringLit, "string", "32"),
            (H::Comment, "comment", "90"),
            (H::Number, "number", "33"),
            (H::Constant, "constant", "33"),
            (H::Attribute, "attribute", "33"),
            (H::Operator, "operator", "37"),
            (H::Variable, "variable", "39"),
            (H::Plain, "plain", "39"),
        ] {
            assert_eq!((class.label(), class.ansi_sgr()), (label, sgr));
        }
    }

    #[test]
    fn every_capture_family_maps_to_its_class() {
        use HighlightClass as H;
        for (capture, class) in [
            ("constructor", H::Function),
            ("escape", H::StringLit),
            ("char", H::StringLit),
            ("number", H::Number),
            ("float", H::Number),
            ("integer", H::Number),
            ("constant.builtin", H::Constant),
            ("attribute", H::Attribute),
            ("operator", H::Operator),
            ("punctuation.bracket", H::Operator),
            ("variable.parameter", H::Variable),
            ("property", H::Variable),
            ("label", H::Variable),
            ("tag", H::Plain),
        ] {
            assert_eq!(HighlightClass::from_capture(capture), class, "{capture}");
        }
    }

    #[test]
    fn flatten_drops_empty_spans() {
        let span = |start, end| HighlightSpan {
            start,
            end,
            class: HighlightClass::Keyword,
        };
        assert_eq!(flatten(vec![span(0, 0), span(0, 3)]), [span(0, 3)]);
    }

    /// The byte budget is inclusive: a source of exactly `DEFAULT_BYTE_BUDGET` bytes is
    /// highlighted, one byte more is not.
    #[test]
    fn the_byte_budget_admits_exactly_its_size() {
        let at_budget = format!("//{}", "a".repeat(crate::analyze::DEFAULT_BYTE_BUDGET - 2));
        assert_eq!(highlight(Language::Rust, &at_budget).len(), 1);
        assert_eq!(highlight(Language::Rust, &format!("{at_budget}a")).len(), 0);
    }
}
