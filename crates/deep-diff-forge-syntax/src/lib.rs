// DDF integration: R08, R10, R11. Never turn a failed analysis into formatting-only or behavioral safety; parser time cancellation is cooperative, node limits are post-parse.
// Design/status: docs/RECOMMENDATION_INTEGRATION.md#syntax-availability
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fdeep-diff-forge-syntax--lib
//! Tree-sitter semantic layer for Deep-Diff-Forge (L4).
//!
//! Given a file's language and source bytes, this crate parses with tree-sitter
//! under explicit budgets and extracts top-level symbols, reporting an explicit
//! [`ParseStatus`](deep_diff_forge_core::ParseStatus) (parsed,
//! parsed-with-errors, or a fallback reason). It never
//! pretends syntax is available when parsing fails, and it never mutates patch
//! truth (it has no access to it).
//!
//! Source bytes are bounded before parsing. A cooperative progress callback
//! cancels long parses; node count is capped after parsing. This is not process
//! isolation or a hard real-time guarantee (R10).

/// Wall-clock backstop for one parse (anchor A-04): a floor plus an allowance per KiB of source.
/// Ordinary Rust parses in linear time, about 0.29 ms per KiB in a release build (a 1.9 MB file in
/// 0.55 s), so the allowance leaves about seventeen times that at every size: ordinary code gets the
/// same result whatever the machine's load. The backstop exists for hostile input, where
/// tree-sitter's error recovery is super-linear (a 455 KiB token soup ran for 125 s; it is now cut
/// at about 3 s). Counting progress callbacks is no substitute: that input made 19 calls per KiB,
/// fewer than ordinary code (27.5), because each recovery step is slow. The previous flat 250 ms
/// made one 220 KB file complete in 9 and time out in 55 of 64 parallel runs.
pub(crate) const BACKSTOP_FLOOR: std::time::Duration = std::time::Duration::from_secs(1);
/// The backstop's allowance per KiB of source (see [`BACKSTOP_FLOOR`]).
pub(crate) const BACKSTOP_PER_KIB: std::time::Duration = std::time::Duration::from_millis(5);

/// The parse deadline for `bytes` of source.
pub(crate) fn backstop_for(bytes: usize) -> std::time::Duration {
    let kib = u32::try_from(bytes / 1024).unwrap_or(u32::MAX);
    BACKSTOP_FLOOR.saturating_add(BACKSTOP_PER_KIB.saturating_mul(kib))
}

/// Cooperative parser deadline; checked by tree-sitter's progress callback.
/// The source byte guard and post-parse node cap remain independently necessary.
fn parse_bounded(parser: &mut tree_sitter::Parser, source: &str) -> Option<tree_sitter::Tree> {
    parse_within(parser, source, backstop_for(source.len()))
}

/// [`parse_bounded`] with an explicit deadline, so the cancellation path can be tested.
fn parse_within(
    parser: &mut tree_sitter::Parser,
    source: &str,
    deadline: std::time::Duration,
) -> Option<tree_sitter::Tree> {
    let start = std::time::Instant::now();
    // `checked_sub` is `None` exactly when more than `deadline` has elapsed.
    let mut cancel = |_: &tree_sitter::ParseState| deadline.checked_sub(start.elapsed()).is_none();
    let options = tree_sitter::ParseOptions::new().progress_callback(&mut cancel);
    parser.parse_with_options(
        &mut |offset, _| &source.as_bytes()[offset..],
        None,
        Some(options),
    )
}

mod analyze;
mod highlight;
mod language;
mod structural;

pub use analyze::{SemanticAnalysis, SyntaxOptions, analyze, analyze_language, enclosing_symbol};
pub use highlight::{HighlightClass, HighlightSpan, highlight, highlight_to_ansi, to_ansi};
pub use language::{Language, detect_language};
pub use structural::{
    ChangeKind, StructuralDiff, Token, TokenChange, structural_diff, tokenize, tokenize_checked,
};

/// A top-level named item discovered in a source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    /// Item name (for `impl` blocks, the implemented type).
    pub name: String,
    /// Friendly item kind, e.g. `"function"`, `"struct"`, `"impl"`.
    pub kind: String,
    /// Byte offset of the item start.
    pub start_byte: u64,
    /// Byte offset of the item end.
    pub end_byte: u64,
    /// 1-based start line.
    pub start_line: u32,
    /// 1-based end line.
    pub end_line: u32,
}

impl Symbol {
    /// Whether `line` falls within this symbol's line span (inclusive).
    #[must_use]
    pub fn contains_line(&self, line: u32) -> bool {
        self.start_line <= line && line <= self.end_line
    }

    /// Number of lines the symbol spans (inclusive).
    #[must_use]
    pub fn line_count(&self) -> u32 {
        self.end_line
            .saturating_sub(self.start_line)
            .saturating_add(1)
    }
}

#[cfg(test)]
mod tests {
    fn rust_parser() -> tree_sitter::Parser {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .unwrap();
        parser
    }

    /// The cancellation path is reachable: an exhausted deadline stops the parse at the first
    /// progress check (`TimeBudgetExceeded` had no test).
    #[test]
    fn an_exhausted_deadline_cancels_the_parse() {
        let source = "fn f() { let x = 1; }\n".repeat(2000);
        assert!(parse_within(&mut rust_parser(), &source, std::time::Duration::ZERO).is_none());
        assert!(parse_bounded(&mut rust_parser(), &source).is_some());
    }

    /// The deadline grows with the source: a floor, plus a fixed allowance per KiB.
    #[test]
    fn the_backstop_scales_with_source_size() {
        assert_eq!(backstop_for(0), std::time::Duration::from_secs(1));
        assert_eq!(backstop_for(1023), std::time::Duration::from_secs(1));
        assert_eq!(backstop_for(1024), std::time::Duration::from_millis(1005));
        assert_eq!(
            backstop_for(2 * 1024 * 1024),
            std::time::Duration::from_millis(11_240)
        );
    }

    use super::*;

    fn sym(start: u32, end: u32) -> Symbol {
        Symbol {
            name: "x".into(),
            kind: "function".into(),
            start_byte: 0,
            end_byte: 0,
            start_line: start,
            end_line: end,
        }
    }

    #[test]
    fn contains_line_inclusive_bounds() {
        let s = sym(3, 7);
        assert!(s.contains_line(3));
        assert!(s.contains_line(7));
        assert!(s.contains_line(5));
    }

    #[test]
    fn contains_line_rejects_outside() {
        let s = sym(3, 7);
        assert!(!s.contains_line(2));
        assert!(!s.contains_line(8));
    }

    #[test]
    fn line_count_is_inclusive() {
        assert_eq!(sym(3, 7).line_count(), 5);
        assert_eq!(sym(4, 4).line_count(), 1);
    }
}
