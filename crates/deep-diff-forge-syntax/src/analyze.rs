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
    let Some(tree) = parser.parse(source, None) else {
        return SemanticAnalysis::fallback(language, FallbackReason::ParseErrorsExceeded);
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

fn count_errors(node: Node) -> u32 {
    let mut errors = u32::from(node.is_error() || node.is_missing());
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        errors = errors.saturating_add(count_errors(child));
    }
    errors
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
    if node.kind() == "impl_item" {
        if let Some(ty) = node.child_by_field_name("type") {
            return node_text(ty, source);
        }
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
        assert!(a.symbols.is_empty());
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
        assert!(a.symbols.is_empty());
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
        assert!(a.symbols.is_empty());
    }

    #[test]
    fn fallback_yields_no_symbols() {
        let a = analyze("notes.txt", "x", SyntaxOptions::default());
        assert!(a.symbols.is_empty());
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
}
