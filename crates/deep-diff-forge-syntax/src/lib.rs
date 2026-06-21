//! Tree-sitter semantic layer for Deep-Diff-Forge (L4).
//!
//! Given a file's language and source bytes, this crate parses with tree-sitter
//! under explicit budgets and extracts top-level symbols, reporting an explicit
//! [`ParseStatus`](deep_diff_forge_core::ParseStatus) (parsed,
//! parsed-with-errors, or a fallback reason). It never
//! pretends syntax is available when parsing fails, and it never mutates patch
//! truth (it has no access to it).
//!
//! Budgets enforced today: byte budget (pre-parse guard against pathological
//! inputs) and node budget (post-parse cap). Time-budget enforcement is
//! deferred until the parser progress-callback API is wired, and is therefore
//! never reported as a fallback reason here.

mod analyze;
mod language;

pub use analyze::{SemanticAnalysis, SyntaxOptions, analyze, analyze_language, enclosing_symbol};
pub use language::{Language, detect_language};

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
