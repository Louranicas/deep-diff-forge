//! Integration tests: analyze real Rust source files from the workspace.

use deep_diff_forge_core::ParseStatus;
use deep_diff_forge_syntax::{Language, SyntaxOptions, analyze, enclosing_symbol};

fn read(rel: &str) -> String {
    let path = format!("{}/{rel}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"))
}

#[test]
fn analyzes_own_lib_rs() {
    let src = read("src/lib.rs");
    let a = analyze("src/lib.rs", &src, SyntaxOptions::default());
    assert_eq!(a.language, Language::Rust);
    assert_eq!(a.parse_status, ParseStatus::Parsed);
}

#[test]
fn own_lib_rs_has_symbols() {
    let src = read("src/lib.rs");
    let a = analyze("src/lib.rs", &src, SyntaxOptions::default());
    assert!(!a.symbols.is_empty());
    assert!(
        a.symbols
            .iter()
            .any(|s| s.kind == "struct" && s.name == "Symbol")
    );
}

#[test]
fn analyzes_a_large_real_parser_file() {
    let src = read("../deep-diff-forge-patch/src/parser.rs");
    let a = analyze("parser.rs", &src, SyntaxOptions::default());
    assert_eq!(a.parse_status, ParseStatus::Parsed);
    // The real parser file defines several top-level items.
    assert!(a.symbols.len() >= 3);
}

#[test]
fn real_file_symbols_have_monotonic_lines() {
    let src = read("src/analyze.rs");
    let a = analyze("analyze.rs", &src, SyntaxOptions::default());
    for pair in a.symbols.windows(2) {
        assert!(pair[1].start_line >= pair[0].start_line);
    }
}

#[test]
fn enclosing_symbol_resolves_a_real_line() {
    let src = read("src/language.rs");
    let a = analyze("language.rs", &src, SyntaxOptions::default());
    // Some line inside the file should resolve to an enclosing item.
    let mid = a.symbols.first().map_or(1, |s| s.start_line + 1);
    assert!(enclosing_symbol(&a.symbols, mid).is_some());
}

#[test]
fn every_crate_src_file_parses_clean() {
    for rel in ["src/lib.rs", "src/language.rs", "src/analyze.rs"] {
        let src = read(rel);
        let a = analyze(rel, &src, SyntaxOptions::default());
        assert_eq!(
            a.parse_status,
            ParseStatus::Parsed,
            "{rel} did not parse clean"
        );
    }
}

#[test]
fn non_rust_path_falls_back_even_with_rust_content() {
    // Detection is by extension: a .txt file is unsupported regardless of bytes.
    let a = analyze("snippet.txt", "fn main() {}", SyntaxOptions::default());
    assert_eq!(a.language, Language::Unsupported);
    assert!(a.symbols.is_empty());
}

#[test]
fn tight_node_budget_falls_back_on_real_file() {
    let src = read("src/analyze.rs");
    let opts = SyntaxOptions {
        node_budget: 10,
        ..SyntaxOptions::default()
    };
    let a = analyze("analyze.rs", &src, opts);
    assert!(matches!(a.parse_status, ParseStatus::Fallback { .. }));
}

#[test]
fn generous_budget_parses_real_file() {
    let src = read("src/analyze.rs");
    let a = analyze("analyze.rs", &src, SyntaxOptions::default());
    assert!(matches!(
        a.parse_status,
        ParseStatus::Parsed | ParseStatus::ParsedWithErrors { .. }
    ));
}

#[test]
fn symbols_have_nonempty_names() {
    let src = read("src/lib.rs");
    let a = analyze("src/lib.rs", &src, SyntaxOptions::default());
    assert!(a.symbols.iter().all(|s| !s.name.is_empty()));
}
