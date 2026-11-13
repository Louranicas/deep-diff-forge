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

/// A place in a fragment: its row and its byte column, both from 0. Places compare in source
/// order.
pub type Place = (u32, u32);

/// A stretch of a fragment: from a place up to, not including, another.
pub type Span = (Place, Place);

/// One construct the parser read with no error above it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Construct {
    pub kind: ConstructKind,
    /// The row (0-based line of the fragment) it starts on; for a method call, the row of the
    /// method's name.
    pub row: u32,
    /// The column (in bytes) it starts at on `row`. With the row it says which construct of a
    /// line this is, when a line holds several.
    pub column: u32,
    /// The last row it reaches: an attribute or a macro invocation can span several rows, and a
    /// change to any of them changes the construct. For a method call, equal to `row`.
    pub last_row: u32,
    /// The place just past the part the parser read as this construct itself: the end of an
    /// attribute, the end of a macro's path (before its `!`: its arguments are not read), the
    /// end of a method's name. A word between the construct's start and here is its own.
    pub end: Place,
    /// Whether it sits inside an item that carries `#[cfg(test)]` in a way that is proof **if
    /// the fragment is the beginning of its file**: a module, a function or an `impl` at the
    /// top level whose first attribute is exactly `#[cfg(test)]`, in a tree with no error (the
    /// reasons are on the private `opens_test_code`). A fragment from
    /// the middle of a file proves nothing, because what stands above it can make the attribute
    /// text a comment, a string or the input of a macro; the caller knows where the fragment
    /// begins (not from a hunk's header, which is the patch writer's word: only a file the
    /// patch creates whole is known) and must not use this otherwise. False says only that there is no proof, not that
    /// the code is production code.
    pub in_test: bool,
    /// Whether an item around it carries an attribute that *spells* a test (`#[test]`, a path
    /// ending in `::test`, `#[rstest]`, `#[cfg(test)]`). A hint for a reader, never proof: see
    /// [`FragmentScan::spelled_items`]. It follows the items the construct is inside, so a
    /// function that only shares a row with a test function is not taken for it.
    pub test_spelled: bool,
}

/// What the parser could and could not read in a fragment.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FragmentScan {
    /// Constructs in source order.
    pub constructs: Vec<Construct>,
    /// Inclusive row ranges the parse says nothing reliable about: error nodes and the insides
    /// of macro invocations. Sorted, and merged so that no two overlap or touch.
    pub unreadable: Vec<(u32, u32)>,
    /// The items proved to be test code **if the fragment is its whole file**: each from the
    /// start of its first attribute to its end. Proof is by the stretch an item covers, not by
    /// rows or lines: a function that only shares a line with a test item is outside it, and
    /// so is a call the parser names no construct for (review, IR-H-1: by line, a production
    /// call inside a macro's arguments was withdrawn with the test call beside it). See
    /// [`Construct::in_test`] for the conditions.
    pub test_items: Vec<Span>,
    /// The items (function, module, `impl`) that carry an attribute which *spells* a test:
    /// `#[test]`, `#[rstest]`, any path ending in `::test` (`#[tokio::test]`), or
    /// `#[cfg(test)]`, wherever the item stands in the fragment; each from the start of its
    /// first attribute to its end. This is a hint for a reader and never proof: every one of
    /// these names but `cfg` can be bound to something else, and a fragment from the middle of
    /// a file can sit under an open comment. Nothing may be withdrawn on it; for what is proof
    /// see [`Self::test_items`]. An item inside another macro's body is not read and gets no
    /// hint.
    pub spelled_items: Vec<Span>,
}

impl FragmentScan {
    /// Whether the parser read `row` with no error and no macro body on it.
    #[must_use]
    pub fn readable(&self, row: u32) -> bool {
        // The ranges are sorted and disjoint: the first that ends at or after `row` is the only
        // one that can hold it. A search of every range for every row made a hunk of 8,000
        // macro lines cost a third of a second (review, IR-H-3).
        let at = self.unreadable.partition_point(|(_, last)| *last < row);
        self.unreadable
            .get(at)
            .is_none_or(|(first, _)| *first > row)
    }

    /// Whether everything from `first` up to `last` lies inside one item that is test code
    /// (see [`Self::test_items`]).
    #[must_use]
    pub fn inside_test(&self, first: Place, last: Place) -> bool {
        self.test_items
            .iter()
            .any(|(start, end)| *start <= first && last <= *end)
    }

    /// Whether everything from `first` up to `last` lies inside one item whose attribute
    /// spells a test (see [`Self::spelled_items`]): a hint, never proof.
    #[must_use]
    pub fn spells_test(&self, first: Place, last: Place) -> bool {
        self.spelled_items
            .iter()
            .any(|(start, end)| *start <= first && last <= *end)
    }

    /// Whether `place` is inside a construct's own part: its name, its path or its attribute.
    #[must_use]
    pub fn names(&self, place: Place) -> bool {
        self.constructs
            .iter()
            .any(|read| (read.row, read.column) <= place && place < read.end)
    }

    /// How many of the `rows` rows from `first` on the parser could not read.
    #[must_use]
    pub fn unreadable_in(&self, first: u32, rows: u32) -> u32 {
        let end = first.saturating_add(rows);
        self.unreadable
            .iter()
            .map(|(from, to)| {
                let from = (*from).max(first);
                let to = to.saturating_add(1).min(end);
                to.saturating_sub(from)
            })
            .sum()
    }

    /// How many rows of `0..rows` the parser could not read.
    #[must_use]
    pub fn unreadable_rows(&self, rows: u32) -> u32 {
        self.unreadable_in(0, rows)
    }
}

/// Sort `ranges` and merge those that overlap or touch.
fn merged(mut ranges: Vec<(u32, u32)>) -> Vec<(u32, u32)> {
    ranges.sort_unstable();
    let mut out: Vec<(u32, u32)> = Vec::with_capacity(ranges.len());
    for (first, last) in ranges {
        match out.last_mut() {
            Some(open) if first <= open.1.saturating_add(1) => open.1 = open.1.max(last),
            _ => out.push((first, last)),
        }
    }
    out
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
    let mut scan = walk(root, source);
    scan.unreadable = merged(std::mem::take(&mut scan.unreadable));
    // A tree with an error anywhere is not a reading of the file that the compiler shares, so
    // it proves nothing about what is test code. Neither is one whose top level holds anything
    // but items, attributes and comments: the grammar accepts there what the compiler never
    // does (a shebang line that the compiler reads as an attribute and an opened comment,
    // an expression), and each is a place where the two readers of one file part ways.
    if root.has_error() || !top_level_is_items(root) {
        scan.test_items.clear();
        for construct in &mut scan.constructs {
            construct.in_test = false;
        }
    }
    Some(scan)
}

/// Whether every child of the file's root is an item, an attribute or a comment.
///
/// tree-sitter-rust reads a first line `#! /**/ [allow(unused)] /*` as a shebang, one ignored
/// line; the compiler skips the comment after `#!`, finds `[`, reads an inner attribute and then
/// an opened block comment. A `#[cfg(test)] // */` on the next line is then comment text for
/// the compiler and an attribute for the grammar, with no error in the tree (review, IR-H-2).
/// So a shebang proves nothing here, and neither does any top-level statement.
fn top_level_is_items(root: Node) -> bool {
    let mut walker = root.walk();
    root.children(&mut walker).all(|child| {
        matches!(
            child.kind(),
            "function_item"
                | "mod_item"
                | "impl_item"
                | "struct_item"
                | "enum_item"
                | "union_item"
                | "trait_item"
                | "type_item"
                | "const_item"
                | "static_item"
                | "use_declaration"
                | "extern_crate_declaration"
                | "foreign_mod_item"
                | "macro_definition"
                | "macro_invocation"
                | "attribute_item"
                | "inner_attribute_item"
                | "line_comment"
                | "block_comment"
        )
    })
}

fn place_of(point: tree_sitter::Point) -> Place {
    (
        u32::try_from(point.row).unwrap_or(u32::MAX),
        u32::try_from(point.column).unwrap_or(u32::MAX),
    )
}

/// The stretch an item covers with its attributes: from the first attribute or comment of the
/// run before it to its end.
fn item_span(node: Node) -> Span {
    let mut first = node;
    let mut before = node.prev_sibling();
    while let Some(sibling) = before {
        match sibling.kind() {
            "attribute_item" => first = sibling,
            "line_comment" | "block_comment" => {}
            _ => break,
        }
        before = sibling.prev_sibling();
    }
    (
        place_of(first.start_position()),
        place_of(node.end_position()),
    )
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
    let mut above: Vec<(bool, bool, bool)> = Vec::new();
    let mut cursor = root.walk();
    loop {
        let node = cursor.node();
        let (error_above, test_above, spelled_above) =
            above.last().copied().unwrap_or((false, false, false));
        let error_here = node.is_error() || node.is_missing();
        let macro_body = node.kind() == "token_tree"
            && node
                .parent()
                .is_some_and(|parent| parent.kind() == "macro_invocation");
        if error_here || macro_body {
            scan.unreadable.push(rows_of(node));
        }
        let unread = error_above || error_here;
        let opens_test = opens_test_code(node, source);
        if opens_test {
            scan.test_items.push(item_span(node));
        }
        let spells = spells_test(node, source);
        if spells {
            scan.spelled_items.push(item_span(node));
        }
        let test_spelled = spelled_above || spells;
        let in_test = test_above || opens_test;
        if !unread && let Some(kind) = construct(node, source) {
            // A method call is where its name is: a chain often starts lines above it.
            let named_at = match kind {
                ConstructKind::MethodCall { .. } => node
                    .child_by_field_name("function")
                    .and_then(|function| function.child_by_field_name("field")),
                ConstructKind::Attribute { .. } | ConstructKind::MacroCall { .. } => None,
            };
            let placed = named_at.unwrap_or(node);
            let row = row_of(placed);
            // Its own part ends with the method's name, the macro's path, or the attribute.
            let own = match kind {
                ConstructKind::MacroCall { .. } => node.child_by_field_name("macro"),
                ConstructKind::MethodCall { .. } | ConstructKind::Attribute { .. } => None,
            }
            .unwrap_or(placed);
            scan.constructs.push(Construct {
                kind,
                row,
                end: place_of(own.end_position()),
                column: u32::try_from(placed.start_position().column).unwrap_or(u32::MAX),
                last_row: if named_at.is_some() {
                    row
                } else {
                    rows_of(node).1
                },
                in_test,
                test_spelled,
            });
        }
        // Nothing under an attribute or a macro body is a construct of its own.
        let closed = macro_body || node.kind() == "attribute_item";
        if !closed && cursor.goto_first_child() {
            above.push((unread, in_test, test_spelled));
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
            words: attribute_words(node, source),
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

/// An attribute's words: the identifiers of its path, then the identifiers that stand directly
/// in its argument list. What is nested deeper belongs to something else:
/// `#[cfg_attr(feature = "serde", serde(skip, ignore))]` gives `cfg_attr`, `feature`, `serde`,
/// and the `ignore` there is serde's, not a skipped test (review, IR-L-2).
fn attribute_words(item: Node, source: &str) -> Vec<String> {
    let mut walker = item.walk();
    let Some(attribute) = item
        .children(&mut walker)
        .find(|child| child.kind() == "attribute")
    else {
        return identifiers(item, source);
    };
    let mut words = attribute
        .named_child(0)
        .map(|path| identifiers(path, source))
        .unwrap_or_default();
    if let Some(arguments) = attribute.child_by_field_name("arguments") {
        let mut walker = arguments.walk();
        words.extend(
            arguments
                .children(&mut walker)
                .filter(|child| child.kind() == "identifier")
                .map(|child| text(child, source).to_owned()),
        );
    }
    words
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

/// Whether `node` is an item compiled only for tests, as far as a file read from its first line
/// can prove it: a module, a function or an `impl` at the top level of the file whose first
/// attribute is exactly `#[cfg(test)]`.
///
/// The answer withdraws evidence, so it has to be proof. Each condition closes a way to make
/// production code look like test code (review V3: VD3-H-1, VD3R2-H-1, VD3R3-H-1):
///
/// - **`cfg`, and nothing else.** The compiler reserves that name ("name `cfg` is reserved in
///   attribute namespace"), so no import and no crate can make `#[cfg(test)]` mean anything
///   else. Every other attribute can be rebound: `use tracing::instrument as rstest;` makes
///   `#[rstest]` leave the function in the program, a dependency renamed to `tokio` makes
///   `#[tokio::test]` whatever that crate says, and `use some::attribute as test;` does the
///   same to `#[test]` itself (compiled and run on Rust 1.99, 2026-11-13). So `#[test]` proves
///   nothing here. `#[cfg(not(test))]`, `#[cfg(any(test, …))]` and `#[cfg_attr(…, cfg(test))]`
///   are not it either.
/// - **The first attribute.** An attribute macro written before it receives the item with the
///   `cfg` still on it and may drop it.
/// - **At the top level of the file.** An attribute macro on an enclosing item, or a macro whose
///   body the item is in, receives the item as tokens and may do the same.
/// - **In a tree with no error, read from the first line** (the caller's part, and
///   [`scan_fragment`]'s): otherwise a comment or a string opened above the fragment can hold
///   the attribute text and end before the item.
///
/// What is left unproved: a build that sets `--cfg test` itself, and a file that another file
/// includes inside a macro. Neither is in the hunk, and the first is a build setting a review
/// of the build files sees.
fn opens_test_code(node: Node, source: &str) -> bool {
    if !matches!(node.kind(), "function_item" | "mod_item" | "impl_item")
        || node
            .parent()
            .is_none_or(|parent| parent.kind() != "source_file")
    {
        return false;
    }
    let mut first = None;
    let mut before = node.prev_sibling();
    while let Some(sibling) = before {
        match sibling.kind() {
            "attribute_item" => first = Some(sibling),
            "line_comment" | "block_comment" => {}
            _ => break,
        }
        before = sibling.prev_sibling();
    }
    first
        .and_then(|attribute| attribute_parts(attribute, source))
        .is_some_and(|(path, arguments)| path == "cfg" && arguments.as_deref() == Some("(test)"))
}

/// Whether `node` is an item with an attribute that spells a test: its path ends in `test`
/// (`#[test]`, `#[tokio::test]`, `#[r#test]`) or is `rstest`, or it is exactly `#[cfg(test)]`.
/// A hint for a reader, never proof: [`opens_test_code`] says what proof takes.
fn spells_test(node: Node, source: &str) -> bool {
    if !matches!(node.kind(), "function_item" | "mod_item" | "impl_item") {
        return false;
    }
    let mut before = node.prev_sibling();
    while let Some(sibling) = before {
        match sibling.kind() {
            "attribute_item" => {
                let spelled = attribute_parts(sibling, source).is_some_and(|(path, arguments)| {
                    let last = path.rsplit("::").next().unwrap_or(&path);
                    let last = last.strip_prefix("r#").unwrap_or(last);
                    matches!(last, "test" | "rstest")
                        || (path == "cfg" && arguments.as_deref() == Some("(test)"))
                });
                if spelled {
                    return true;
                }
            }
            "line_comment" | "block_comment" => {}
            _ => return false,
        }
        before = sibling.prev_sibling();
    }
    false
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
    fn a_whole_test_function_is_read_and_a_test_attribute_alone_proves_nothing() {
        let source = "#[test]\n#[ignore = \"slow\"]\nfn a() {\n    assert!( true );\n    let x = f().unwrap();\n}\n";
        assert_eq!(
            kinds(source),
            [
                (attribute(&["test"]), 0, false),
                (attribute(&["ignore"]), 1, false),
                (macro_call("assert", "true"), 3, false),
                (method("unwrap"), 4, false),
            ]
        );
    }

    #[test]
    fn a_construct_says_at_which_column_it_starts() {
        let found = scan("    assert!(a()); assert!(true);\n    b().unwrap(); c().unwrap();\n");
        let places: Vec<(u32, u32)> = found.constructs.iter().map(|c| (c.row, c.column)).collect();
        assert_eq!(places, [(0, 4), (0, 18), (1, 8), (1, 22)]);
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
    fn only_an_exact_first_cfg_test_at_the_top_level_marks_what_is_inside_it() {
        let module = "#[cfg(test)]\nmod checks {\n    fn helper() { a().unwrap(); }\n}\n";
        assert_eq!(kinds(module)[1], (method("unwrap"), 2, true));
        let spaced = "#[ cfg( test ) ]\nmod checks {\n    fn helper() { a().unwrap(); }\n}\n";
        assert_eq!(kinds(spaced)[1], (method("unwrap"), 2, true));
        // on a function and on an impl it is the same proof, with other attributes after it
        let function = "#[cfg(test)]\n#[allow(dead_code)]\nfn helper() { a().unwrap(); }\n";
        assert_eq!(kinds(function)[2], (method("unwrap"), 2, true));
        let implementation = "use x::Thing;\n/// For tests.\n#[cfg(test)]\nimpl Thing {\n    fn make() -> Self { a().unwrap() }\n}\n";
        assert_eq!(kinds(implementation)[1], (method("unwrap"), 4, true));
        // a test function inside it stays test code, and so does a closure inside that
        let nested =
            "#[cfg(test)]\nmod t {\n    #[test]\n    fn a() { run(|| b().unwrap()); }\n}\n";
        assert_eq!(kinds(nested).last().unwrap(), &(method("unwrap"), 3, true));
        // the rows wholly inside the item are test code, a macro's body included; the rows the
        // item shares with what is before and after it are not claimed
        // the item is a stretch, from its first attribute to its end: what lies in it is test
        // code, a macro's body included, and what only shares a row with it is not
        let found = scan(nested);
        assert_eq!(found.test_items, [((0, 0), (4, 1))]);
        assert!(found.inside_test((3, 4), (3, 36)) && found.inside_test((0, 0), (4, 1)));
        assert!(!found.inside_test((3, 4), (4, 2)) && !found.inside_test((5, 0), (5, 1)));
        let shared = scan("#[cfg(test)]\nfn t() { a().unwrap(); } pub fn p() { b().unwrap(); }\n");
        assert_eq!(shared.test_items, [((0, 0), (1, 24))]);
        assert!(shared.inside_test((1, 0), (1, 24)));
        assert!(
            !shared.inside_test((1, 0), (1, 52)),
            "the row holds another function"
        );
        let broken = scan("#[cfg(test)]\nmod t {\n    fn a() {}\n}\nfn broken( {\n");
        assert_eq!(broken.test_items, []);
        let unproved = [
            // a cfg that only mentions test among other things
            "#[cfg(not(test))]\nmod real {\n    fn f() { a().unwrap(); }\n}\n",
            // the item after the one under cfg(test) is not under it
            "#[cfg(test)]\nfn helper() {}\nfn real() { a().unwrap(); }\n",
            // an attribute before it may be a macro that drops it
            "#[strip]\n#[cfg(test)]\nfn production() { a().unwrap(); }\n",
            // so may an attribute on the item around it, or a macro whose body it is in
            "#[strip]\nmod outer {\n    #[cfg(test)]\n    fn production() { a().unwrap(); }\n}\n",
            "mod outer {\n    #[cfg(test)]\n    fn production() { a().unwrap(); }\n}\n",
            "fn wrapper() {\n    #[cfg(test)]\n    fn production() { a().unwrap(); }\n}\n",
            // a tree with an error anywhere is no reading of the file
            "let = ;\n#[cfg(test)]\nfn production() { a().unwrap(); }\n",
            "#[cfg(test)]\nfn production() { a().unwrap(); }\nfn broken( {\n",
        ];
        for source in unproved {
            let found = kinds(source);
            let call = found
                .iter()
                .find(|(kind, _, _)| *kind == method("unwrap"))
                .unwrap_or_else(|| panic!("no call read: {source}"));
            assert!(!call.2, "believed to be test code: {source}");
        }
    }

    /// The hint beside the proof: a construct inside an item whose attribute spells a test
    /// says so, wherever the item stands, and nothing more is claimed for it.
    #[test]
    fn a_construct_inside_an_item_that_spells_a_test_carries_the_hint() {
        let hinted = |source: &str| -> Vec<bool> {
            scan(source)
                .constructs
                .iter()
                .filter(|c| c.kind == method("unwrap"))
                .map(|c| c.test_spelled)
                .collect()
        };
        for attribute in [
            "#[test]",
            "#[tokio::test]",
            "#[tokio::test(flavor = \"x\")]",
            "#[rstest]",
            "#[r#test]",
            "#[my_framework::test]",
            "#[cfg(test)]",
        ] {
            let source = format!("fn real() {{}}\n{attribute}\nfn a() {{ b().unwrap(); }}\n");
            assert_eq!(hinted(&source), [true], "{attribute}");
            // a hint is not the proof: only the exact `cfg(test)` is, and then only for a
            // fragment that is its whole file, which the caller knows
            assert_eq!(
                scan(&source).test_items.is_empty(),
                attribute != "#[cfg(test)]",
                "{attribute}"
            );
        }
        for attribute in [
            "#[inline]",
            "#[cfg(not(test))]",
            "#[cfg_attr(test, allow(x))]",
            "#[testing]",
            "#[doc = \"test\"]",
        ] {
            let source = format!("{attribute}\nfn a() {{ b().unwrap(); }}\n");
            assert_eq!(hinted(&source), [false], "{attribute}");
        }
        // nested: a closure in a test, a function in a test module, a method of a test impl
        assert_eq!(
            hinted("#[test]\nfn a() { run(|| b().unwrap()); }\n"),
            [true]
        );
        assert_eq!(
            hinted(
                "#[cfg(test)]\nmod t {\n    fn h() { b().unwrap(); }\n}\nfn real() { c().unwrap(); }\n"
            ),
            [true, false]
        );
        // Review V3 round 7, VD3R7-L-1: a function that only shares a row with a test is not
        // one. Before, the hint went by rows and this production call carried it.
        assert_eq!(
            hinted("#[test] fn check() {} fn production() { b().unwrap(); }\n"),
            [false]
        );
        assert_eq!(
            hinted("#[test] fn check() { a().unwrap(); } fn production() { b().unwrap(); }\n"),
            [true, false]
        );
        // the rows wholly inside such an item, for what the parser names no construct for
        let module =
            "fn real() {}\n#[cfg(test)]\nmod tests {\n    fn helper() { a().unwrap(); }\n}\n";
        let found = scan(module);
        assert_eq!(found.spelled_items, [((1, 0), (4, 1))]);
        assert!(found.spells_test((3, 4), (3, 33)) && !found.spells_test((0, 0), (0, 12)));
        // one row, two functions: the test is a stretch of it and the row as a whole is not
        let shared = scan("#[test] fn a() {} fn b() {}\n");
        assert_eq!(shared.spelled_items, [((0, 0), (0, 17))]);
        assert!(shared.spells_test((0, 0), (0, 17)) && !shared.spells_test((0, 0), (0, 27)));
    }

    /// Review, IR-H-2 and IR-L-1: the grammar and the compiler part ways on a first line that
    /// starts `#!`, and the grammar takes a statement at the top level. Either way the tree has
    /// no error and the `#[cfg(test)]` under it is not one for the compiler. Only a top level of
    /// items, attributes and comments proves anything.
    #[test]
    fn a_top_level_the_compiler_would_read_otherwise_proves_no_test_code() {
        for source in [
            // for the compiler: `#![allow(unused)]`, then a comment that ends on line 2
            "#! /**/ [allow(unused)] /*\n#[cfg(test)] // */\npub mod t {\n    pub fn production() -> u32 { b().unwrap() }\n}\n",
            "#!/usr/bin/env run\n#[cfg(test)]\nmod t {\n    fn f() { b().unwrap(); }\n}\n",
            // an expression where only items may stand
            "x;\n#[cfg(test)]\nmod t {\n    fn f() { b().unwrap(); }\n}\n",
            "---x\n;#[cfg(test)] /*\n---\n// */\nmod t {\n    pub fn production() -> u32 { b().unwrap() }\n}\n",
        ] {
            let found = scan(source);
            assert_eq!(found.test_items, [], "{source}");
            assert!(found.constructs.iter().all(|c| !c.in_test), "{source}");
        }
        // the control: an inner attribute and a comment first are an ordinary file
        let plain = scan(
            "#![allow(unused)]\n// a file\nuse a::b;\n#[cfg(test)]\nmod t {\n    fn f() { b().unwrap(); }\n}\n",
        );
        assert_eq!(plain.test_items, [((3, 0), (6, 1))]);
    }

    /// The part of a construct that is its own, and the rows the parser could not read: sorted,
    /// merged, and counted without visiting every row.
    #[test]
    fn a_construct_has_its_own_part_and_unreadable_rows_are_merged() {
        let found = scan(
            "    std::assert!(a); b.unwrap(); #[x]\n    outer! {\n        inner!(1);\n    }\n",
        );
        // `std::assert`, `unwrap` and `outer` are named; what follows each is not
        assert!(found.names((0, 4)) && found.names((0, 14)) && !found.names((0, 15)));
        assert!(found.names((0, 23)) && !found.names((0, 29)));
        assert!(
            found.names((1, 4)) && !found.names((2, 8)),
            "inside a macro's body"
        );
        assert_eq!(found.unreadable, [(0, 3)]);
        assert_eq!(
            merged(vec![(5, 6), (0, 1), (2, 2), (6, 9), (20, 21)]),
            [(0, 2), (5, 9), (20, 21)]
        );
        assert_eq!(
            (found.unreadable_in(0, 4), found.unreadable_in(2, 10)),
            (4, 2)
        );
        assert_eq!(found.unreadable_rows(2), 2);
        let none = scan("fn a() {}\nfn b() {}\n");
        assert!(none.readable(0) && none.readable(1) && none.unreadable_rows(2) == 0);
    }

    /// Review, IR-L-2: an attribute's words are its path and what stands directly in its own
    /// argument list.
    #[test]
    fn an_attribute_lists_only_the_words_of_its_own_argument_list() {
        assert_eq!(
            kinds("#[cfg_attr(feature = \"serde\", serde(skip, ignore, default))]\nstruct A;\n")[0]
                .0,
            attribute(&["cfg_attr", "feature", "serde"])
        );
        assert_eq!(
            kinds("#[cfg_attr(all(a, b), ignore = \"slow\")]\nfn a() {}\n")[0].0,
            attribute(&["cfg_attr", "all", "ignore"])
        );
        assert_eq!(
            kinds("#[tokio::test(flavor = \"x\")]\nasync fn a() {}\n")[0].0,
            attribute(&["tokio", "test", "flavor"])
        );
    }

    /// Review V3 round 3, VD3R3-H-1. Every attribute name but `cfg` can be bound by an import
    /// to a macro that leaves the function in the program, `test` included, so none of them is
    /// proof that code is test code.
    #[test]
    fn a_test_attribute_of_any_spelling_proves_no_test_code() {
        for attribute in [
            "#[test]",
            "#[r#test]",
            "#[::core::prelude::v1::test]",
            "#[std::prelude::v1::test]",
            "#[tokio::test]",
            "#[tokio::test(flavor = \"multi_thread\")]",
            "#[r#tokio::r#test]",
            "#[async_std::test]",
            "#[actix_rt::test]",
            "#[test_log::test]",
            "#[rstest]",
            "#[rstest::rstest]",
        ] {
            let source = format!("{attribute}\npub fn production() {{ b().unwrap(); }}\n");
            assert_eq!(
                kinds(&source)[1],
                (method("unwrap"), 1, false),
                "{attribute}"
            );
        }
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

    /// Independent review, IR-H-3 (b): `readable` searched every range for every row, so a
    /// hunk of macro lines with blank lines between them cost the square of its length (3 MiB
    /// ranked in 10.8 s), and no test had a scan with many ranges. Here 600,000 rows are
    /// looked up among 200,000 ranges: at once by a binary search, and not within the half
    /// minute by a search of them all (a planted linear search was stopped by this bound).
    #[test]
    fn a_row_is_found_among_many_ranges_without_reading_them_all() {
        const RANGES: u32 = 200_000;
        let found = FragmentScan {
            unreadable: (0..RANGES).map(|i| (i * 3, i * 3 + 1)).collect(),
            ..FragmentScan::default()
        };
        let (done, wait) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let rows = RANGES * 3;
            let unread = (0..rows).filter(|row| !found.readable(*row)).count();
            let _ = done.send((
                unread,
                found.unreadable_rows(rows),
                found.readable(2),
                found.readable(rows),
            ));
        });
        let answer = wait
            .recv_timeout(std::time::Duration::from_secs(30))
            .expect("600,000 rows were looked up within half a minute");
        assert_eq!(answer, (400_000, 400_000, true, true));
    }
}
