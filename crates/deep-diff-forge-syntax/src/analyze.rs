// DDF integration: R08, R10, R11. Never turn a failed analysis into formatting-only or behavioral safety; parser time cancellation is cooperative, node limits are post-parse.
// Design/status: docs/RECOMMENDATION_INTEGRATION.md#syntax-availability
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fdeep-diff-forge-syntax--analyze
use crate::Symbol;
use crate::language::{Language, detect_language};
use deep_diff_forge_core::{FallbackReason, ParseStatus};
use tree_sitter::Node;

/// Default byte budget: refuse to parse inputs larger than this.
pub const DEFAULT_BYTE_BUDGET: usize = 2 * 1024 * 1024;
/// Default node budget: discard semantics if the tree exceeds this.
pub const DEFAULT_NODE_BUDGET: usize = 200_000;

/// Budgets controlling a semantic analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyntaxOptions {
    /// Maximum source size in bytes (pre-parse guard).
    pub byte_budget: usize,
    /// Maximum tree node count (post-parse cap).
    pub node_budget: usize,
}

impl Default for SyntaxOptions {
    fn default() -> Self {
        Self {
            byte_budget: DEFAULT_BYTE_BUDGET,
            node_budget: DEFAULT_NODE_BUDGET,
        }
    }
}

/// The result of analyzing one source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticAnalysis {
    /// Detected language.
    pub language: Language,
    /// Parse status (parsed / parsed-with-errors / fallback reason).
    pub parse_status: ParseStatus,
    /// Top-level symbols (empty on any fallback).
    pub symbols: Vec<Symbol>,
}

impl SemanticAnalysis {
    fn fallback(language: Language, reason: FallbackReason) -> Self {
        Self {
            language,
            parse_status: ParseStatus::Fallback { reason },
            symbols: Vec::new(),
        }
    }
}

/// Analyze a file by path (language detected from the extension) and source.
#[must_use]
pub fn analyze(path: &str, source: &str, options: SyntaxOptions) -> SemanticAnalysis {
    analyze_language(detect_language(path), source, options)
}

/// Analyze source with an explicit language.
#[must_use]
pub fn analyze_language(
    language: Language,
    source: &str,
    options: SyntaxOptions,
) -> SemanticAnalysis {
    if !language.is_supported() {
        return SemanticAnalysis::fallback(language, FallbackReason::UnsupportedLanguage);
    }
    if source.len() > options.byte_budget {
        return SemanticAnalysis::fallback(language, FallbackReason::ByteBudgetExceeded);
    }

    let mut parser = tree_sitter::Parser::new();
    if parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .is_err()
    {
        return SemanticAnalysis::fallback(language, FallbackReason::UnsupportedLanguage);
    }
    let Some(tree) = crate::parse_bounded(&mut parser, source) else {
        return SemanticAnalysis::fallback(language, FallbackReason::TimeBudgetExceeded);
    };

    let root = tree.root_node();
    if root.descendant_count() > options.node_budget {
        return SemanticAnalysis::fallback(language, FallbackReason::NodeBudgetExceeded);
    }

    let errors = count_errors(root);
    let symbols = extract_symbols(root, source);
    let parse_status = if errors > 0 {
        ParseStatus::ParsedWithErrors { errors }
    } else {
        ParseStatus::Parsed
    };

    SemanticAnalysis {
        language,
        parse_status,
        symbols,
    }
}

/// Find the smallest symbol whose line span contains `line`.
#[must_use]
pub fn enclosing_symbol(symbols: &[Symbol], line: u32) -> Option<&Symbol> {
    symbols
        .iter()
        .filter(|s| s.contains_line(line))
        .min_by_key(|s| s.line_count())
}

fn count_errors(root: Node) -> u32 {
    // Iterative DFS via TreeCursor — avoids stack overflow on deeply-nested trees.
    // A narrow tree ~40k levels deep (well within the 200k node-budget) would
    // overflow the stack with tail-recursive descent; this mirrors the pattern
    // already used in structural.rs.
    let mut count = 0u32;
    let mut cursor = root.walk();
    loop {
        let node = cursor.node();
        if node.is_error() || node.is_missing() {
            count = count.saturating_add(1);
        }
        if cursor.goto_first_child() {
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                return count;
            }
        }
    }
}

fn extract_symbols(root: Node, source: &str) -> Vec<Symbol> {
    let mut symbols = Vec::new();
    let mut cursor = root.walk();
    for child in root.named_children(&mut cursor) {
        let Some(kind) = friendly_kind(child.kind()) else {
            continue;
        };
        let name = symbol_name(child, source).unwrap_or_else(|| kind.to_string());
        symbols.push(Symbol {
            name,
            kind: kind.to_string(),
            start_byte: to_u64(child.start_byte()),
            end_byte: to_u64(child.end_byte()),
            start_line: to_line(child.start_position().row),
            end_line: to_line(child.end_position().row),
        });
    }
    symbols
}

fn friendly_kind(kind: &str) -> Option<&'static str> {
    match kind {
        "function_item" | "function_signature_item" => Some("function"),
        "struct_item" => Some("struct"),
        "enum_item" => Some("enum"),
        "trait_item" => Some("trait"),
        "impl_item" => Some("impl"),
        "mod_item" => Some("module"),
        "const_item" => Some("const"),
        "static_item" => Some("static"),
        "type_item" => Some("type"),
        "union_item" => Some("union"),
        "macro_definition" => Some("macro"),
        _ => None,
    }
}

fn symbol_name(node: Node, source: &str) -> Option<String> {
    if let Some(name) = node.child_by_field_name("name") {
        return node_text(name, source);
    }
    if node.kind() == "impl_item"
        && let Some(ty) = node.child_by_field_name("type")
    {
        return node_text(ty, source);
    }
    None
}

fn node_text(node: Node, source: &str) -> Option<String> {
    source
        .get(node.start_byte()..node.end_byte())
        .map(ToString::to_string)
}

fn to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

fn to_line(row: usize) -> u32 {
    u32::try_from(row).unwrap_or(u32::MAX).saturating_add(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run `work` on its own thread and fail if it takes more than ten seconds: these analyses
    /// take milliseconds, so a tree walk that never ends fails its test instead of hanging the
    /// suite (and a mutation run reports it as caught, not as a timeout).
    fn within_deadline<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> T {
        let (done, wait) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = done.send(work());
        });
        wait.recv_timeout(std::time::Duration::from_secs(10))
            .expect("an analysis finished within ten seconds")
    }

    // The tests call these deadline-bound wrappers, which shadow the functions under test.

    fn analyze(path: &str, source: &str, options: SyntaxOptions) -> SemanticAnalysis {
        let (path, source) = (path.to_owned(), source.to_owned());
        within_deadline(move || super::analyze(&path, &source, options))
    }

    fn analyze_language(
        language: Language,
        source: &str,
        options: SyntaxOptions,
    ) -> SemanticAnalysis {
        let source = source.to_owned();
        within_deadline(move || super::analyze_language(language, &source, options))
    }

    const SAMPLE: &str = "\
fn alpha() {
    let x = 1;
}

struct Point {
    x: i32,
    y: i32,
}

enum Shape {
    Circle,
    Square,
}

impl Point {
    fn origin() -> Self {
        Point { x: 0, y: 0 }
    }
}
";

    fn analyze_rs(source: &str) -> SemanticAnalysis {
        analyze("x.rs", source, SyntaxOptions::default())
    }

    #[test]
    fn parses_clean_rust_as_parsed() {
        assert_eq!(analyze_rs("fn main() {}").parse_status, ParseStatus::Parsed);
    }

    #[test]
    fn detects_language_rust() {
        assert_eq!(analyze_rs("fn main() {}").language, Language::Rust);
    }

    #[test]
    fn extracts_a_function_symbol() {
        let a = analyze_rs("fn alpha() {}");
        assert_eq!(a.symbols.len(), 1);
        assert_eq!(a.symbols[0].kind, "function");
        assert_eq!(a.symbols[0].name, "alpha");
    }

    #[test]
    fn extracts_multiple_top_level_items() {
        let a = analyze_rs(SAMPLE);
        let kinds: Vec<&str> = a.symbols.iter().map(|s| s.kind.as_str()).collect();
        assert!(kinds.contains(&"function"));
        assert!(kinds.contains(&"struct"));
        assert!(kinds.contains(&"enum"));
        assert!(kinds.contains(&"impl"));
    }

    #[test]
    fn struct_symbol_has_name() {
        let a = analyze_rs(SAMPLE);
        assert!(
            a.symbols
                .iter()
                .any(|s| s.kind == "struct" && s.name == "Point")
        );
    }

    #[test]
    fn enum_symbol_has_name() {
        let a = analyze_rs(SAMPLE);
        assert!(
            a.symbols
                .iter()
                .any(|s| s.kind == "enum" && s.name == "Shape")
        );
    }

    #[test]
    fn impl_symbol_uses_type_name() {
        let a = analyze_rs(SAMPLE);
        assert!(
            a.symbols
                .iter()
                .any(|s| s.kind == "impl" && s.name == "Point")
        );
    }

    #[test]
    fn impl_method_is_not_top_level_symbol() {
        // `origin` lives inside the impl; only the impl is top-level.
        let a = analyze_rs(SAMPLE);
        assert!(!a.symbols.iter().any(|s| s.name == "origin"));
    }

    #[test]
    fn symbol_line_ranges_are_one_based() {
        let a = analyze_rs("fn alpha() {}\n");
        assert_eq!(a.symbols[0].start_line, 1);
    }

    #[test]
    fn second_item_starts_after_first() {
        let a = analyze_rs("fn a() {}\nfn b() {}\n");
        assert_eq!(a.symbols.len(), 2);
        assert!(a.symbols[1].start_line > a.symbols[0].start_line);
    }

    #[test]
    fn trait_and_const_and_type_are_extracted() {
        let src = "trait T {}\nconst C: u8 = 1;\ntype Alias = u8;\n";
        let a = analyze_rs(src);
        let kinds: Vec<&str> = a.symbols.iter().map(|s| s.kind.as_str()).collect();
        assert!(kinds.contains(&"trait"));
        assert!(kinds.contains(&"const"));
        assert!(kinds.contains(&"type"));
    }

    #[test]
    fn module_and_static_and_macro_are_extracted() {
        let src = "mod m {}\nstatic S: u8 = 1;\nmacro_rules! mac { () => {}; }\n";
        let a = analyze_rs(src);
        let kinds: Vec<&str> = a.symbols.iter().map(|s| s.kind.as_str()).collect();
        assert!(kinds.contains(&"module"));
        assert!(kinds.contains(&"static"));
        assert!(kinds.contains(&"macro"));
    }

    #[test]
    fn broken_syntax_reports_parsed_with_errors() {
        let a = analyze_rs("fn broken( {");
        assert!(matches!(a.parse_status, ParseStatus::ParsedWithErrors { errors } if errors > 0));
    }

    #[test]
    fn empty_source_is_parsed_with_no_symbols() {
        let a = analyze_rs("");
        assert_eq!(a.parse_status, ParseStatus::Parsed);
        assert_eq!(a.symbols.len(), 0);
    }

    #[test]
    fn unsupported_language_falls_back() {
        let a = analyze("notes.txt", "hello", SyntaxOptions::default());
        assert_eq!(a.language, Language::Unsupported);
        assert_eq!(
            a.parse_status,
            ParseStatus::Fallback {
                reason: FallbackReason::UnsupportedLanguage
            }
        );
        assert_eq!(a.symbols.len(), 0);
    }

    #[test]
    fn byte_budget_forces_fallback() {
        let opts = SyntaxOptions {
            byte_budget: 4,
            ..SyntaxOptions::default()
        };
        let a = analyze("x.rs", "fn main() {}", opts);
        assert_eq!(
            a.parse_status,
            ParseStatus::Fallback {
                reason: FallbackReason::ByteBudgetExceeded
            }
        );
    }

    #[test]
    fn node_budget_forces_fallback() {
        let opts = SyntaxOptions {
            node_budget: 1,
            ..SyntaxOptions::default()
        };
        let a = analyze("x.rs", SAMPLE, opts);
        assert_eq!(
            a.parse_status,
            ParseStatus::Fallback {
                reason: FallbackReason::NodeBudgetExceeded
            }
        );
        assert_eq!(a.symbols.len(), 0);
    }

    #[test]
    fn fallback_yields_no_symbols() {
        let a = analyze("notes.txt", "x", SyntaxOptions::default());
        assert_eq!(a.symbols.len(), 0);
    }

    #[test]
    fn enclosing_symbol_finds_containing_item() {
        let a = analyze_rs(SAMPLE);
        // Point struct spans lines 5..8; line 6 is inside it.
        let s = enclosing_symbol(&a.symbols, 6).unwrap();
        assert_eq!(s.name, "Point");
        assert_eq!(s.kind, "struct");
    }

    #[test]
    fn enclosing_symbol_none_when_between_items() {
        let a = analyze_rs("fn a() {}\n\n\nfn b() {}\n");
        // line 2 is a blank line between the two functions.
        assert!(enclosing_symbol(&a.symbols, 2).is_none());
    }

    #[test]
    fn enclosing_symbol_picks_innermost_by_line_count() {
        let outer = Symbol {
            name: "outer".into(),
            kind: "impl".into(),
            start_byte: 0,
            end_byte: 0,
            start_line: 1,
            end_line: 20,
        };
        let inner = Symbol {
            name: "inner".into(),
            kind: "function".into(),
            start_byte: 0,
            end_byte: 0,
            start_line: 5,
            end_line: 8,
        };
        let syms = vec![outer, inner];
        assert_eq!(enclosing_symbol(&syms, 6).unwrap().name, "inner");
    }

    #[test]
    fn default_options_are_generous() {
        let o = SyntaxOptions::default();
        assert_eq!(o.byte_budget, DEFAULT_BYTE_BUDGET);
        assert_eq!(o.node_budget, DEFAULT_NODE_BUDGET);
    }

    #[test]
    fn symbol_byte_ranges_are_populated() {
        let a = analyze_rs("fn alpha() {}\n");
        assert!(a.symbols[0].end_byte > a.symbols[0].start_byte);
    }

    #[test]
    fn analyze_language_bypasses_detection() {
        let a = analyze_language(Language::Rust, "fn z() {}", SyntaxOptions::default());
        assert_eq!(a.symbols.len(), 1);
        assert_eq!(a.symbols[0].name, "z");
    }

    #[test]
    fn analyze_language_unsupported_falls_back() {
        let a = analyze_language(Language::Unsupported, "anything", SyntaxOptions::default());
        assert!(matches!(
            a.parse_status,
            ParseStatus::Fallback {
                reason: FallbackReason::UnsupportedLanguage
            }
        ));
    }

    #[test]
    fn errors_counted_are_positive_for_multiple_breaks() {
        let a = analyze_rs("fn a( { struct B");
        match a.parse_status {
            ParseStatus::ParsedWithErrors { errors } => assert!(errors >= 1),
            other => panic!("expected parsed-with-errors, got {other:?}"),
        }
    }

    #[test]
    fn unicode_source_parses_and_spans_are_valid() {
        let a = analyze_rs("fn greet() {\n    let s = \"café→\";\n}\n");
        assert_eq!(a.parse_status, ParseStatus::Parsed);
        assert_eq!(a.symbols.len(), 1);
    }

    #[test]
    fn many_functions_all_extracted() {
        use std::fmt::Write as _;
        let mut src = String::new();
        for i in 0..30 {
            let _ = writeln!(src, "fn f{i}() {{}}");
        }
        let a = analyze_rs(&src);
        assert_eq!(a.symbols.len(), 30);
    }

    #[test]
    fn count_errors_iterative_does_not_overflow_on_deeply_nested_source() {
        // Regression: FINDING-002 — the former recursive count_errors would stack-
        // overflow on ASTs whose DEPTH exceeded ~40k levels (a narrowly nested file
        // well inside the 200k node-budget). The iterative TreeCursor rewrite makes
        // the traversal O(1) stack regardless of depth. We use a moderately nested
        // Rust expression (800 nested closures, enough to exercise multi-level
        // recursion in the old code without needing a 40k-deep test fixture).
        let depth = 800_usize;
        let mut src = String::from("fn f() { ");
        for _ in 0..depth {
            src.push_str("(|| ");
        }
        src.push('1');
        for _ in 0..depth {
            src.push_str(")()");
        }
        src.push_str("; }");
        // Must complete without panicking/aborting.
        let a = analyze_rs(&src);
        // Broken syntax is acceptable; what matters is we return, not abort.
        let _ = a.parse_status;
    }

    #[test]
    fn count_errors_zero_for_clean_parse() {
        let a = analyze_rs("fn clean() -> u32 { 42 }");
        assert_eq!(a.parse_status, ParseStatus::Parsed);
    }

    // Pins for the 2026-10-31 whole-crate mutation run.

    #[test]
    fn the_default_byte_budget_is_two_mebibytes() {
        assert_eq!(DEFAULT_BYTE_BUDGET, 2 << 20);
    }

    /// Both budgets are inclusive: a source of exactly `byte_budget` bytes, or a tree of exactly
    /// `node_budget` nodes, is analysed; one less of either falls back.
    #[test]
    fn the_budgets_admit_exactly_their_size() {
        let source = "fn main() {}";
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .unwrap();
        let nodes = parser
            .parse(source, None)
            .unwrap()
            .root_node()
            .descendant_count();
        let status = |byte_budget, node_budget| {
            analyze(
                "x.rs",
                source,
                SyntaxOptions {
                    byte_budget,
                    node_budget,
                },
            )
            .parse_status
        };
        assert_eq!(status(source.len(), nodes), ParseStatus::Parsed);
        assert_eq!(
            status(source.len() - 1, nodes),
            ParseStatus::Fallback {
                reason: FallbackReason::ByteBudgetExceeded
            }
        );
        assert_eq!(
            status(source.len(), nodes - 1),
            ParseStatus::Fallback {
                reason: FallbackReason::NodeBudgetExceeded
            }
        );
    }

    #[test]
    fn a_union_is_a_union_symbol() {
        let a = analyze_rs("union U {\n    a: u32,\n}\n");
        assert_eq!(a.symbols.len(), 1);
        assert_eq!(
            (a.symbols[0].name.as_str(), a.symbols[0].kind.as_str()),
            ("U", "union")
        );
    }
}
