// DDF integration: agent batteries, phase 1 (the evidence ladder, rung 2).
// Design/status: obsidian://open?vault=deep-diff-forge.vault&file=Strategy%2FDDF%20batteries%20-%20beyond%20lexical%20evidence%202026-11-13
//! Constructs in a fragment of source: what a hunk holds, read by the parser instead of by
//! spelling.
//!
//! A hunk is not a file. It can begin in the middle of a function and end before the function
//! does, so the parse of a fragment has parts the parser could not read. [`scan_fragment`]
//! reports the constructs that sit under no error node, and the rows it could not read
//! ([`FragmentScan::unreadable`]): there the caller keeps whatever a lower rung found, and
//! nothing is claimed. The inside of a macro invocation is such a part too, since the parser
//! does not expand macros.
//!
//! The budgets are counts (bytes before the parse, nodes after it), so the same fragment always
//! gives the same answer. The clock is only the crate's backstop against hostile input.

use crate::Language;
use tree_sitter::Node;

/// The largest fragment that is parsed. A hunk larger than this stays on the lower rungs.
pub const FRAGMENT_BYTE_BUDGET: usize = 64 << 10;

/// The most nodes a fragment's tree may have and still be used.
pub const FRAGMENT_NODE_BUDGET: usize = 50_000;

/// The most bytes of a macro's arguments that are kept (whitespace removed first).
pub const MACRO_ARGUMENT_CAP: usize = 48;

/// What a construct is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConstructKind {
    /// An attribute. `words` are its identifiers in order: `#[cfg_attr(miri, ignore)]` gives
    /// `cfg_attr`, `miri`, `ignore`. The contents of string literals are not words.
    Attribute { words: Vec<String> },
    /// A macro invocation: the last segment of its path, and its arguments with all whitespace
    /// removed, cut at [`MACRO_ARGUMENT_CAP`] bytes.
    MacroCall { name: String, arguments: String },
    /// A method call `receiver.name(…)`.
    MethodCall { name: String },
}

/// One construct the parser read with no error above it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Construct {
    pub kind: ConstructKind,
    /// The row (0-based line of the fragment) it starts on; for a method call, the row of the
    /// method's name.
    pub row: u32,
    /// The last row it reaches: an attribute or a macro invocation can span several rows, and a
    /// change to any of them changes the construct. For a method call, equal to `row`.
    pub last_row: u32,
    /// Whether it sits inside test code that the fragment itself proves: a function that
    /// carries a test attribute directly (`#[test]`, `#[tokio::test]`, `#[rstest]`), or a module
    /// that is exactly `#[cfg(test)]`. An attribute that only mentions `test` inside another
    /// one (`#[cfg_attr(any(), test)]`, `#[cfg(not(test))]`) proves nothing and is false, and so
    /// is a module that is merely named `tests`. False says only that the fragment does not
    /// prove it, not that the code is production code.
    pub in_test: bool,
}

/// What the parser could and could not read in a fragment.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FragmentScan {
    /// Constructs in source order.
    pub constructs: Vec<Construct>,
    /// Inclusive row ranges the parse says nothing reliable about: error nodes and the insides
    /// of macro invocations. Ranges may overlap.
    pub unreadable: Vec<(u32, u32)>,
}

impl FragmentScan {
    /// Whether the parser read `row` with no error and no macro body on it.
    #[must_use]
    pub fn readable(&self, row: u32) -> bool {
        !self
            .unreadable
            .iter()
            .any(|(first, last)| (*first..=*last).contains(&row))
    }

    /// How many rows of `0..rows` the parser could not read.
    #[must_use]
    pub fn unreadable_rows(&self, rows: u32) -> u32 {
        let count = (0..rows).filter(|row| !self.readable(*row)).count();
        u32::try_from(count).unwrap_or(u32::MAX)
    }
}

/// Scan `source` as a fragment of `language`. `None` when the language has no grammar, the
/// fragment is over [`FRAGMENT_BYTE_BUDGET`], its tree is over [`FRAGMENT_NODE_BUDGET`], or the
/// parse was cancelled by the crate's backstop: the caller then has no rung-2 answer at all.
#[must_use]
pub fn scan_fragment(language: Language, source: &str) -> Option<FragmentScan> {
    if !language.is_supported() || source.len() > FRAGMENT_BYTE_BUDGET {
        return None;
    }
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .ok()?;
    let tree = crate::parse_bounded(&mut parser, source)?;
    let root = tree.root_node();
    if root.descendant_count() > FRAGMENT_NODE_BUDGET {
        return None;
    }
    Some(walk(root, source))
}

fn row_of(node: Node) -> u32 {
    u32::try_from(node.start_position().row).unwrap_or(u32::MAX)
}

fn rows_of(node: Node) -> (u32, u32) {
    let last = u32::try_from(node.end_position().row).unwrap_or(u32::MAX);
    (row_of(node), last)
}

fn text<'a>(node: Node, source: &'a str) -> &'a str {
    source.get(node.byte_range()).unwrap_or("")
}

/// One walk of the tree, without recursion (a fragment can nest as deeply as its node budget).
/// `above` holds, for each ancestor of the current node, whether an error sits at or above it
/// and whether test code does.
fn walk(root: Node, source: &str) -> FragmentScan {
    let mut scan = FragmentScan::default();
    let mut above: Vec<(bool, bool)> = Vec::new();
    let mut cursor = root.walk();
    loop {
        let node = cursor.node();
        let (error_above, test_above) = above.last().copied().unwrap_or((false, false));
        let error_here = node.is_error() || node.is_missing();
        let macro_body = node.kind() == "token_tree"
            && node
                .parent()
                .is_some_and(|parent| parent.kind() == "macro_invocation");
        if error_here || macro_body {
            scan.unreadable.push(rows_of(node));
        }
        let unread = error_above || error_here;
        let in_test = test_above || opens_test_code(node, source);
        if !unread && let Some(kind) = construct(node, source) {
            // A method call is where its name is: a chain often starts lines above it.
            let named_at = match kind {
                ConstructKind::MethodCall { .. } => node
                    .child_by_field_name("function")
                    .and_then(|function| function.child_by_field_name("field")),
                ConstructKind::Attribute { .. } | ConstructKind::MacroCall { .. } => None,
            };
            let row = row_of(named_at.unwrap_or(node));
            scan.constructs.push(Construct {
                kind,
                row,
                last_row: if named_at.is_some() {
                    row
                } else {
                    rows_of(node).1
                },
                in_test,
            });
        }
        // Nothing under an attribute or a macro body is a construct of its own.
        let closed = macro_body || node.kind() == "attribute_item";
        if !closed && cursor.goto_first_child() {
            above.push((unread, in_test));
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                return scan;
            }
            above.pop();
        }
    }
}

fn construct(node: Node, source: &str) -> Option<ConstructKind> {
    match node.kind() {
        "attribute_item" | "inner_attribute_item" => Some(ConstructKind::Attribute {
            words: identifiers(node, source),
        }),
        "macro_invocation" => {
            let path = text(node.child_by_field_name("macro")?, source);
            let name = path.rsplit("::").next().unwrap_or(path).trim().to_owned();
            let mut walker = node.walk();
            let body = node
                .children(&mut walker)
                .find(|child| child.kind() == "token_tree");
            let inner = body.map_or("", |tree| text(tree, source));
            // Without the delimiters, and without whitespace: `( true )` reads as `true`.
            let inner = inner
                .get(1..inner.len().saturating_sub(1))
                .unwrap_or_default();
            let arguments: String = inner
                .chars()
                .filter(|c| !c.is_whitespace())
                .take_while({
                    let mut bytes = 0;
                    move |c| {
                        bytes += c.len_utf8();
                        bytes <= MACRO_ARGUMENT_CAP
                    }
                })
                .collect();
            Some(ConstructKind::MacroCall { name, arguments })
        }
        "call_expression" => {
            let function = node.child_by_field_name("function")?;
            if function.kind() != "field_expression" {
                return None;
            }
            let name = text(function.child_by_field_name("field")?, source).to_owned();
            Some(ConstructKind::MethodCall { name })
        }
        _ => None,
    }
}

/// The identifiers under `node`, in order, without recursion.
fn identifiers(node: Node, source: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut cursor = node.walk();
    loop {
        let current = cursor.node();
        if current.kind() == "identifier" {
            words.push(text(current, source).to_owned());
        }
        if cursor.goto_first_child() {
            continue;
        }
        loop {
            if cursor.node() == node {
                return words;
            }
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                return words;
            }
        }
    }
}

/// An attribute's own path and its arguments, both with whitespace removed: `#[tokio::test(
/// flavor = "x")]` gives `tokio::test` and `(flavor="x")`; `#[test]` gives `test` and nothing.
/// What an attribute holds inside its arguments is not its path: `#[cfg_attr(any(), test)]` has
/// the path `cfg_attr`.
fn attribute_parts(item: Node, source: &str) -> Option<(String, Option<String>)> {
    let compact = |node: Node| -> String {
        text(node, source)
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect()
    };
    let mut walker = item.walk();
    let attribute = item
        .children(&mut walker)
        .find(|child| child.kind() == "attribute")?;
    let path = attribute.named_child(0)?;
    let arguments = attribute.child_by_field_name("arguments").map(compact);
    Some((compact(path), arguments))
}

/// The attribute paths that make the function under them a test, written without raw-identifier
/// prefixes: the built-in attribute and a short list of test attributes of well-known crates.
/// A path that is not here proves nothing, whatever its last segment is: any crate can export
/// an attribute macro named `test` that leaves the function in the program (review V3 round 2,
/// VD3R2-H-1: before, every path ending in `::test` was believed).
const TEST_ATTRIBUTES: &[&str] = &[
    "test",
    "core::prelude::v1::test",
    "std::prelude::v1::test",
    "rstest",
    "rstest::rstest",
    "tokio::test",
    "async_std::test",
    "actix_rt::test",
    "test_log::test",
];

/// A path as [`TEST_ATTRIBUTES`] writes it: no leading `::`, no `r#` on a segment.
fn plain_path(path: &str) -> String {
    let segments: Vec<&str> = path
        .trim_start_matches("::")
        .split("::")
        .map(|segment| segment.strip_prefix("r#").unwrap_or(segment))
        .collect();
    segments.join("::")
}

/// Whether `node` is a function or a module that the fragment proves to be test code.
///
/// - A function with a test attribute of its own, by its whole path ([`TEST_ATTRIBUTES`]).
/// - A module with exactly `#[cfg(test)]`.
///
/// Nothing weaker counts, because the answer withdraws evidence. `#[cfg_attr(any(), test)]`
/// never applies `#[test]`, `#[cfg(not(test))]` is production code, and a module named `tests`
/// with no `#[cfg(test)]` is compiled into the program (review V3 round 1, VD3-H-1: before, the
/// last identifier anywhere in an attribute decided, so a false `cfg_attr` hid a panic path in
/// production code). Names are not resolved here: a crate that is itself named `tokio`, or an
/// import that shadows the built-in `test`, would be believed. `#[cfg(test)]` on a function or
/// an `impl` is not read; what is under it is reported.
fn opens_test_code(node: Node, source: &str) -> bool {
    let attributes = || {
        let mut found = Vec::new();
        let mut before = node.prev_sibling();
        while let Some(sibling) = before {
            match sibling.kind() {
                "attribute_item" => found.extend(attribute_parts(sibling, source)),
                "line_comment" | "block_comment" => {}
                _ => break,
            }
            before = sibling.prev_sibling();
        }
        found
    };
    match node.kind() {
        "function_item" => attributes()
            .iter()
            .any(|(path, _)| TEST_ATTRIBUTES.contains(&plain_path(path).as_str())),
        "mod_item" => attributes()
            .iter()
            .any(|(path, arguments)| path == "cfg" && arguments.as_deref() == Some("(test)")),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan(source: &str) -> FragmentScan {
        scan_fragment(Language::Rust, source).expect("a small Rust fragment is scanned")
    }

    fn kinds(source: &str) -> Vec<(ConstructKind, u32, bool)> {
        scan(source)
            .constructs
            .into_iter()
            .map(|c| (c.kind, c.row, c.in_test))
            .collect()
    }

    fn attribute(words: &[&str]) -> ConstructKind {
        ConstructKind::Attribute {
            words: words.iter().map(|w| (*w).to_owned()).collect(),
        }
    }

    fn macro_call(name: &str, arguments: &str) -> ConstructKind {
        ConstructKind::MacroCall {
            name: name.to_owned(),
            arguments: arguments.to_owned(),
        }
    }

    fn method(name: &str) -> ConstructKind {
        ConstructKind::MethodCall {
            name: name.to_owned(),
        }
    }

    #[test]
    fn a_whole_test_function_is_read_and_known_to_be_test_code() {
        let source = "#[test]\n#[ignore = \"slow\"]\nfn a() {\n    assert!( true );\n    let x = f().unwrap();\n}\n";
        assert_eq!(
            kinds(source),
            [
                (attribute(&["test"]), 0, false),
                (attribute(&["ignore"]), 1, false),
                (macro_call("assert", "true"), 3, true),
                (method("unwrap"), 4, true),
            ]
        );
    }

    #[test]
    fn statements_from_the_middle_of_a_body_are_read_as_they_stand() {
        let source = "    let x = f().unwrap();\n    let y = g( ).expect(\"no\");\n    todo!()\n";
        let found = scan(source);
        assert_eq!(
            found.constructs.iter().map(|c| &c.kind).collect::<Vec<_>>(),
            [
                &method("unwrap"),
                &method("expect"),
                &macro_call("todo", "")
            ]
        );
        assert!(found.constructs.iter().all(|c| !c.in_test));
        assert!(found.readable(0) && found.readable(1));
        // the macro's own row holds a macro body
        assert!(!found.readable(2));
    }

    #[test]
    fn a_method_call_is_placed_on_the_row_of_its_name() {
        let source = "    let v = f()\n        .g()\n        .expect(\"set\");\n";
        assert_eq!(
            kinds(source),
            [(method("expect"), 2, false), (method("g"), 1, false)]
        );
    }

    #[test]
    fn a_string_a_comment_and_a_longer_name_are_not_the_construct() {
        let source = "    let s = \"x.unwrap() assert!(true) #[ignore]\"; // y.unwrap()\n    z.unwrap_or(1);\n";
        assert_eq!(kinds(source), [(method("unwrap_or"), 1, false)]);
    }

    #[test]
    fn an_attribute_lists_its_identifiers_and_not_its_strings() {
        assert_eq!(
            kinds("#[cfg_attr(miri, ignore)]\nfn a() {}\n"),
            [(attribute(&["cfg_attr", "miri", "ignore"]), 0, false)]
        );
        assert_eq!(
            kinds("#[doc = \"ignore\"]\nfn a() {}\n"),
            [(attribute(&["doc"]), 0, false)]
        );
    }

    #[test]
    fn test_modules_and_test_attributes_mark_what_is_inside_them() {
        let module = "#[cfg(test)]\nmod checks {\n    fn helper() { a().unwrap(); }\n}\n";
        assert_eq!(kinds(module)[1], (method("unwrap"), 2, true));
        let spaced = "#[ cfg( test ) ]\nmod checks {\n    fn helper() { a().unwrap(); }\n}\n";
        assert_eq!(kinds(spaced)[1], (method("unwrap"), 2, true));
        let tokio = "#[tokio::test]\nasync fn a() { b().await.unwrap(); }\n";
        assert_eq!(kinds(tokio)[1], (method("unwrap"), 1, true));
        let flavoured =
            "#[tokio::test(flavor = \"multi_thread\")]\nasync fn a() { b().await.unwrap(); }\n";
        assert_eq!(kinds(flavoured)[1], (method("unwrap"), 1, true));
        let rstest = "#[rstest]\nfn a() { b().unwrap(); }\n";
        assert_eq!(kinds(rstest)[1], (method("unwrap"), 1, true));
        // the built-in attribute written with a raw identifier or a full path is the same one
        for attribute in [
            "#[r#test]",
            "#[::core::prelude::v1::test]",
            "#[r#tokio::r#test]",
            "#[async_std::test]",
        ] {
            let source = format!("{attribute}\nfn a() {{ b().unwrap(); }}\n");
            assert_eq!(
                kinds(&source)[1],
                (method("unwrap"), 1, true),
                "{attribute}"
            );
        }
        // a cfg that only mentions test among other things, and an ordinary module, do not
        let other = "#[cfg(not(test))]\nmod real {\n    fn f() { a().unwrap(); }\n}\n";
        assert_eq!(kinds(other)[1], (method("unwrap"), 2, false));
    }

    /// Review V3 round 1, VD3-H-1. What withdraws evidence must be proved, not mentioned: an
    /// attribute that only names `test` inside another attribute applies no `#[test]`, and a
    /// module named `tests` without `#[cfg(test)]` is compiled into the program.
    #[test]
    fn an_attribute_that_only_mentions_test_proves_no_test_code() {
        for source in [
            "#[cfg_attr(any(), test)]\npub fn production() {\n    load().unwrap();\n}\n",
            "#[cfg_attr(feature = \"x\", test)]\nfn f() {\n    load().unwrap();\n}\n",
            "#[cfg_attr(test, allow(dead_code))]\nfn f() {\n    load().unwrap();\n}\n",
            "#[allow(test)]\nfn f() {\n    load().unwrap();\n}\n",
            "#[doc(alias = \"x\", test)]\nfn f() {\n    load().unwrap();\n}\n",
            "#[my::tests]\nfn f() {\n    load().unwrap();\n}\n",
            // review V3 round 2, VD3R2-H-1: a path that only ends in `test` is not the test
            // attribute of anything known
            "#[my_framework::test]\npub fn production() {\n    load().unwrap();\n}\n",
            "#[tokio::testing::test]\npub fn production() {\n    load().unwrap();\n}\n",
            "#[testing]\nfn f() {\n    load().unwrap();\n}\n",
            "#[cfg(any(test, feature = \"x\"))]\nmod m {\n    fn f() { load().unwrap(); }\n}\n",
            "#[cfg_attr(any(), cfg(test))]\nmod m {\n    fn f() { load().unwrap(); }\n}\n",
            "#[cfg(test = \"x\")]\nmod m {\n    fn f() { load().unwrap(); }\n}\n",
            "mod tests {\n    pub fn production() { load().unwrap(); }\n}\n",
        ] {
            let found = kinds(source);
            let (kind, _, in_test) = found.last().unwrap();
            assert_eq!(*kind, method("unwrap"), "{source}");
            assert!(!in_test, "believed to be test code: {source}");
        }
    }

    /// Review V3 round 1, VD3-M-1: a construct that spans rows says where it ends, so a change
    /// to its second or third row is a change to it.
    #[test]
    fn a_construct_says_which_rows_it_spans() {
        let spans = |source: &str| -> Vec<(u32, u32)> {
            let mut rows: Vec<(u32, u32)> = scan(source)
                .constructs
                .iter()
                .map(|c| (c.row, c.last_row))
                .collect();
            rows.sort_unstable();
            rows
        };
        assert_eq!(
            spans("fn a() {\n    assert!(\n        true\n    );\n}\n"),
            [(1, 3)]
        );
        assert_eq!(
            spans("#[cfg_attr(\n    miri,\n    ignore\n)]\nfn a() {}\n"),
            [(0, 3)]
        );
        assert_eq!(spans("fn a() { todo!() }\n"), [(0, 0)]);
        // a method call is one row: the row of its name, however long the chain is
        assert_eq!(
            spans("fn a() {\n    b()\n        .c()\n        .unwrap();\n}\n"),
            [(2, 2), (3, 3)]
        );
    }

    #[test]
    fn what_the_parser_cannot_read_is_reported_and_holds_no_construct() {
        // a block opened and never closed: the call is under an error
        let open = scan("    if a {\n        b.unwrap();\n");
        assert_eq!(open.constructs, []);
        assert!(open.unreadable_rows(2) > 0);
        // a macro's body is not expanded: what is inside it is not a construct of its own
        let inside = scan("    outer! {\n        assert!(true);\n        x.unwrap();\n    }\n");
        assert_eq!(
            inside
                .constructs
                .iter()
                .map(|c| &c.kind)
                .collect::<Vec<_>>(),
            [&macro_call("outer", "assert!(true);x.unwrap();")]
        );
        assert!(!inside.readable(1) && !inside.readable(2));
    }

    #[test]
    fn a_macro_is_named_by_its_last_segment_and_its_arguments_are_capped() {
        assert_eq!(
            kinds("    std::assert_eq!(true , true);\n")[0].0,
            macro_call("assert_eq", "true,true")
        );
        let long = format!("    m!({});\n", "a".repeat(200));
        let ConstructKind::MacroCall { arguments, .. } = &kinds(&long)[0].0 else {
            panic!("a macro call");
        };
        assert_eq!(arguments.len(), MACRO_ARGUMENT_CAP);
    }

    #[test]
    fn the_budgets_and_the_language_decide_whether_there_is_an_answer() {
        assert_eq!(scan_fragment(Language::Unsupported, "fn a() {}"), None);
        let exactly = format!("fn a() {{}}{}", " ".repeat(FRAGMENT_BYTE_BUDGET - 9));
        assert!(scan_fragment(Language::Rust, &exactly).is_some());
        let over = format!("{exactly} ");
        assert_eq!(scan_fragment(Language::Rust, &over), None);
        let many = "f();\n".repeat(FRAGMENT_NODE_BUDGET / 4);
        assert!(many.len() <= FRAGMENT_BYTE_BUDGET);
        assert_eq!(scan_fragment(Language::Rust, &many), None);
    }

    #[test]
    fn deep_nesting_does_not_overflow_the_stack() {
        let depth = 3000;
        let source = format!("fn f() {{ {}1{} }}", "(".repeat(depth), ")".repeat(depth));
        assert!(scan_fragment(Language::Rust, &source).is_some());
    }
}
