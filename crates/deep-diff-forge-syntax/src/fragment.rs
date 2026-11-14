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
//! The budgets are counts (bytes before the parse, nodes after it) and no clock is asked, so the
//! same fragment always gives the same answer, whatever else the machine is doing.

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
    /// `cfg_attr`, `miri`, `ignore`. The contents of string literals are not words, and neither
    /// is what is nested in a list of its arguments, with one exception: see the private
    /// `attribute_words`. `text` is the whole attribute as it is written, with all whitespace
    /// removed: two attributes are the same attribute only when their texts are equal, since
    /// the words leave out what is nested and what is a string (`all(miri)` against `all()`,
    /// `target_os = "plan9"` against `"linux"`; second independent review, IR2-M-1).
    Attribute { words: Vec<String>, text: String },
    /// A macro invocation: the last segment of its path, and its arguments with all whitespace
    /// removed, cut at [`MACRO_ARGUMENT_CAP`] bytes.
    MacroCall { name: String, arguments: String },
    /// A method call `receiver.name(…)`, or a function named by a path: called
    /// (`Type::name(…)`, `<Option<u32>>::name(x)`, `Type::<T>::name(x)`) or used as a value
    /// (`it.map(Type::name)`). `name` is the last segment of the path, and the construct is
    /// placed there. A path in a `use` declaration, the path of a macro or of an attribute,
    /// the path inside a visibility (`pub(in a::b)`) and a path that only leads to a longer
    /// one are not constructs, and neither is a definition (`fn name`) or a name called with
    /// no path (`name(x)`).
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
    /// The row (0-based line of the fragment) it starts on; for a method call or a path, the
    /// row of the name.
    pub row: u32,
    /// The column (in bytes) it starts at on `row`. With the row it says which construct of a
    /// line this is, when a line holds several.
    pub column: u32,
    /// The last row it reaches: an attribute or a macro invocation can span several rows, and a
    /// change to any of them changes the construct. For a method call or a path, equal to
    /// `row`.
    pub last_row: u32,
    /// The place just past the part the parser read as this construct itself: the end of an
    /// attribute, the end of a macro's path (before its `!`: its arguments are not read), the
    /// end of a method's or a path's name. A word between the construct's start and here is
    /// its own.
    pub end: Place,
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
    /// The items (function, module, `impl`) that carry an attribute which *spells* a test:
    /// `#[test]`, `#[rstest]`, any path ending in `::test` (`#[tokio::test]`), or
    /// `#[cfg(test)]`, wherever the item stands in the fragment; each from the start of its
    /// first attribute to its end. This is a hint for a reader and never proof: every one of
    /// these names but `cfg` can be bound to something else, and a fragment from the middle of
    /// a file can sit under an open comment. Nothing may be withdrawn on it, and nothing in
    /// this scan is proof of test code: the scanner once offered one (a top-level
    /// `#[cfg(test)]` item of a file read whole), and five reviews each found another text
    /// that the grammar and the compiler read differently (a `//*` inside a block comment, a
    /// character the scanner cuts to its low byte, a `cr"…"` that is a raw string only from
    /// edition 2021; second independent review, IR2-H-1 to IR2-H-4). That class has no end, so
    /// the proof is gone. An item inside another macro's body is not read and gets no hint.
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

    /// Whether everything from `first` up to `last` lies inside one item whose attribute
    /// spells a test (see [`Self::spelled_items`]): a hint, never proof.
    #[must_use]
    pub fn spells_test(&self, first: Place, last: Place) -> bool {
        self.spelled_items
            .iter()
            .any(|(start, end)| *start <= first && last <= *end)
    }

    /// Whether `place` is inside a construct's own part: a method's or a path's name, a
    /// macro's path, or an attribute.
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
/// fragment is over [`FRAGMENT_BYTE_BUDGET`] or its tree is over [`FRAGMENT_NODE_BUDGET`]: the
/// caller then has no rung-2 answer at all.
///
/// The answer is a function of `source` alone. No clock cuts this parse: a deadline made one
/// patch give the parser's item on an idle machine and "not checked" on a loaded one
/// (independent review, IR-M-1; still so after it was only documented, review V3 round 8).
/// What bounds the work is bytes. Measured without a clock on 56 shapes of hostile text at 8,
/// 16, 32 and 60 KiB (release build): the slowest of them, text that is all error recovery
/// (`a?b:a?b:…`), costs about 14 microseconds a byte and grows in step with its length;
/// ordinary Rust costs about 0.3. One text is worse: `// todo`, a newline and `#!#!#!…` costs
/// about 3.76e-9 s times the square of its bytes (16 KiB 1.0 s, 64 KiB 14.8 s; second
/// independent review, IR2-H-5). So the byte budget here does not bound a caller's time by
/// itself: a caller that scans many fragments charges each the square of its bytes against a
/// budget of its own (the graph crate's `REVIEW_SQUARE_BUDGET`). A caller that needs a
/// deadline sets one around the whole operation, and then gets an error and not a different
/// answer.
#[must_use]
pub fn scan_fragment(language: Language, source: &str) -> Option<FragmentScan> {
    if !language.is_supported() || source.len() > FRAGMENT_BYTE_BUDGET {
        return None;
    }
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .ok()?;
    let tree = parser.parse(source, None)?;
    let root = tree.root_node();
    if root.descendant_count() > FRAGMENT_NODE_BUDGET {
        return None;
    }
    let mut scan = walk(root, source);
    scan.unreadable = merged(std::mem::take(&mut scan.unreadable));
    Some(scan)
}

fn place_of(point: tree_sitter::Point) -> Place {
    (
        u32::try_from(point.row).unwrap_or(u32::MAX),
        u32::try_from(point.column).unwrap_or(u32::MAX),
    )
}

/// The stretch an item covers with its attributes: from the first attribute of the run before
/// it (`run`, in source order) to its end.
fn item_span(node: Node, run: &[Node]) -> Span {
    let first = run.first().copied().unwrap_or(node);
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

/// What the walk knows about the children of one node while it is among them.
struct Level<'tree> {
    /// An error sits at or above the parent.
    unread: bool,
    /// A `use` declaration sits at or above the parent: a path there names what is imported
    /// and is no value.
    in_use: bool,
    /// An item whose attribute spells a test sits at or above the parent.
    test_spelled: bool,
    /// The parent's kind.
    kind: &'static str,
    /// The attributes directly before the current child, in source order: the run that
    /// comments do not break and anything else does.
    run: Vec<Node<'tree>>,
}

/// One walk of the tree, without recursion (a fragment can nest as deeply as its node budget).
/// `above` holds a [`Level`] for each ancestor of the current node.
///
/// The walk keeps the parent's kind and the attributes before the current node itself. Asking
/// the tree (`Node::parent`, `Node::prev_sibling`) costs the node's depth each time, because
/// tree-sitter finds a parent by walking down from the root: 4,700 functions nested in one
/// another took 2.9 s that way, on the square of the depth (found while the parse clock was
/// being removed, 2026-11-14).
fn walk<'tree>(root: Node<'tree>, source: &str) -> FragmentScan {
    let mut scan = FragmentScan::default();
    let mut above: Vec<Level<'tree>> = Vec::new();
    let mut cursor = root.walk();
    loop {
        let node = cursor.node();
        let kind = node.kind();
        let (error_above, use_above, spelled_above, parent) =
            above.last().map_or((false, false, false, ""), |level| {
                (level.unread, level.in_use, level.test_spelled, level.kind)
            });
        let run: &[Node<'tree>] = above.last().map_or(&[], |level| level.run.as_slice());
        let error_here = node.is_error() || node.is_missing();
        let macro_body = kind == "token_tree" && parent == "macro_invocation";
        if error_here || macro_body {
            scan.unreadable.push(rows_of(node));
        }
        let unread = error_above || error_here;
        let item = matches!(kind, "function_item" | "mod_item" | "impl_item");
        let spells = item && spells_test(run, source);
        if spells {
            scan.spelled_items.push(item_span(node, run));
        }
        let test_spelled = spelled_above || spells;
        let in_use = use_above || kind == "use_declaration";
        if !unread && let Some(read) = construct(node, source, parent, in_use) {
            // A method call is where its name is: a chain often starts lines above it, and a
            // path can be broken after its `::`.
            let named_at = match read {
                ConstructKind::MethodCall { .. } if kind == "call_expression" => node
                    .child_by_field_name("function")
                    .and_then(|function| function.child_by_field_name("field")),
                ConstructKind::MethodCall { .. } => node.child_by_field_name("name"),
                ConstructKind::Attribute { .. } | ConstructKind::MacroCall { .. } => None,
            };
            let placed = named_at.unwrap_or(node);
            let row = row_of(placed);
            // Its own part ends with the name, the macro's path, or the attribute.
            let own = match read {
                ConstructKind::MacroCall { .. } => node.child_by_field_name("macro"),
                ConstructKind::MethodCall { .. } | ConstructKind::Attribute { .. } => None,
            }
            .unwrap_or(placed);
            scan.constructs.push(Construct {
                kind: read,
                row,
                end: place_of(own.end_position()),
                column: u32::try_from(placed.start_position().column).unwrap_or(u32::MAX),
                last_row: if named_at.is_some() {
                    row
                } else {
                    rows_of(node).1
                },
                test_spelled,
            });
        }
        // The run before the next sibling: an attribute joins it, a comment leaves it as it
        // is, anything else ends it.
        if let Some(level) = above.last_mut() {
            match kind {
                "attribute_item" => level.run.push(node),
                "line_comment" | "block_comment" => {}
                _ => level.run.clear(),
            }
        }
        // Nothing under an attribute or a macro body is a construct of its own.
        let closed = macro_body || kind == "attribute_item";
        if !closed && cursor.goto_first_child() {
            above.push(Level {
                unread,
                in_use,
                test_spelled,
                kind,
                run: Vec::new(),
            });
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

/// The construct `node` is, if it is one. `parent` is the kind of the node above it and
/// `in_use` says that a `use` declaration is above it.
fn construct(node: Node, source: &str, parent: &str, in_use: bool) -> Option<ConstructKind> {
    match node.kind() {
        "attribute_item" | "inner_attribute_item" => Some(ConstructKind::Attribute {
            words: attribute_words(node, source),
            text: text(node, source)
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect(),
        }),
        "macro_invocation" => {
            let path = text(node.child_by_field_name("macro")?, source);
            let name = plain(path.rsplit("::").next().unwrap_or(path).trim()).to_owned();
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
            let name = plain(text(function.child_by_field_name("field")?, source)).to_owned();
            Some(ConstructKind::MethodCall { name })
        }
        // A path in an expression: the function of a call (`X::unwrap(x)`), with or without
        // generic arguments, or a value (`it.map(Result::unwrap)`). Before, only the token
        // rules read these, on one line: `X::` and `r#unwrap(x)` on the next left neither an
        // item nor a mark (review V3 round 9, VD3R9-M-1). A type's path is another kind of
        // node. Not a value: what a `use` imports, the path of a macro or of an attribute, the
        // path a visibility names, and a path that is only the front of a longer one. A path
        // in a pattern is read as a value: it errs toward a construct.
        "scoped_identifier" => {
            let leads = matches!(
                parent,
                "macro_invocation"
                    | "attribute"
                    | "scoped_identifier"
                    | "scoped_type_identifier"
                    | "visibility_modifier"
            );
            if in_use || leads {
                return None;
            }
            let name = plain(text(node.child_by_field_name("name")?, source)).to_owned();
            Some(ConstructKind::MethodCall { name })
        }
        _ => None,
    }
}

/// An attribute's words: the identifiers of its path, then the identifiers that stand directly
/// in its argument list. What is nested deeper belongs to something else:
/// `#[cfg_attr(feature = "serde", serde(skip, ignore))]` gives `cfg_attr`, `feature`, `serde`,
/// and the `ignore` there is serde's, not a skipped test (review, IR-L-2).
///
/// One nested list is followed, and only that one: in a `cfg_attr`, an argument after the
/// condition that is itself `cfg_attr(…)` applies attributes too, so what stands directly in
/// that list after its own condition is added, after the words of the list around it:
/// `#[cfg_attr(all(), cfg_attr(all(), ignore))]` gives `cfg_attr`, `all`, `cfg_attr`, `ignore`
/// (second independent review, IR2-M-2: it gave no `ignore`, and the test did not run).
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
    let Some(arguments) = attribute.child_by_field_name("arguments") else {
        return words;
    };
    let conditional = words.len() == 1 && words[0] == "cfg_attr";
    // The lists still to read, without recursion: the attribute's own, then each nested
    // `cfg_attr(…)` found on the way. Of a nested list the condition is left out.
    let mut lists = vec![(arguments, false)];
    while let Some((list, nested)) = lists.pop() {
        let mut walker = list.walk();
        // which argument of the list, counted by its commas, and whether the last word was a
        // `cfg_attr` that a list may follow
        let (mut argument, mut applies) = (0usize, false);
        for child in list.children(&mut walker) {
            match child.kind() {
                "," => {
                    argument += 1;
                    applies = false;
                }
                "identifier" => {
                    let word = plain(text(child, source));
                    applies = conditional && argument > 0 && word == "cfg_attr";
                    if !nested || argument > 0 {
                        words.push(word.to_owned());
                    }
                }
                "token_tree" => {
                    if applies {
                        lists.push((child, true));
                    }
                    applies = false;
                }
                "line_comment" | "block_comment" => {}
                _ => applies = false,
            }
        }
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
            words.push(plain(text(current, source)).to_owned());
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

/// Whether an item with the attributes `run` before it has one that spells a test: its path
/// ends in `test` (`#[test]`, `#[tokio::test]`) or is `rstest`, or it is exactly
/// `#[cfg(test)]`. Each segment of the path is read as the identifier it is (`#[r#test]`,
/// `#[r#tokio::r#test]`, `#[r#cfg(test)]`; review V3 round 9).
///
/// A hint for a reader, never proof. Every one of these names but `cfg` can be bound by an
/// import to a macro that leaves the function in the program (`use tracing::instrument as
/// rstest;`, a dependency renamed to `tokio`, `use some::attribute as test;`: compiled and run
/// on Rust 1.99, 2026-11-13), an attribute macro before or around a `#[cfg(test)]` may drop
/// it, and a fragment can begin under a comment that ends on the attribute's own line.
fn spells_test(run: &[Node], source: &str) -> bool {
    run.iter().any(|attribute| {
        attribute_parts(*attribute, source).is_some_and(|(path, arguments)| {
            let last = path.rsplit("::").next().unwrap_or(&path);
            matches!(plain(last), "test" | "rstest")
                || (plain(&path) == "cfg" && arguments.as_deref() == Some("(test)"))
        })
    })
}

/// An identifier without its raw prefix: `r#unwrap` is the identifier `unwrap` to the
/// compiler, wherever a keyword is not meant (review V3 round 8, VD3R8-M-1).
fn plain(name: &str) -> &str {
    name.strip_prefix("r#").unwrap_or(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan(source: &str) -> FragmentScan {
        scan_fragment(Language::Rust, source).expect("a small Rust fragment is scanned")
    }

    /// Each construct's kind and row. An attribute's `text` is left out (emptied), so that a
    /// test can name an attribute by its words: [`attribute`].
    fn kinds(source: &str) -> Vec<(ConstructKind, u32)> {
        scan(source)
            .constructs
            .into_iter()
            .map(|c| {
                let kind = match c.kind {
                    ConstructKind::Attribute { words, .. } => ConstructKind::Attribute {
                        words,
                        text: String::new(),
                    },
                    other => other,
                };
                (kind, c.row)
            })
            .collect()
    }

    fn attribute(words: &[&str]) -> ConstructKind {
        ConstructKind::Attribute {
            words: words.iter().map(|w| (*w).to_owned()).collect(),
            text: String::new(),
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
    fn a_whole_test_function_is_read() {
        let source = "#[test]\n#[ignore = \"slow\"]\nfn a() {\n    assert!( true );\n    let x = f().unwrap();\n}\n";
        assert_eq!(
            kinds(source),
            [
                (attribute(&["test"]), 0),
                (attribute(&["ignore"]), 1),
                (macro_call("assert", "true"), 3),
                (method("unwrap"), 4),
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
        assert!(found.readable(0) && found.readable(1));
        // the macro's own row holds a macro body
        assert!(!found.readable(2));
    }

    #[test]
    fn a_method_call_is_placed_on_the_row_of_its_name() {
        let source = "    let v = f()\n        .g()\n        .expect(\"set\");\n";
        assert_eq!(kinds(source), [(method("expect"), 2), (method("g"), 1)]);
    }

    #[test]
    fn a_string_a_comment_and_a_longer_name_are_not_the_construct() {
        let source = "    let s = \"x.unwrap() assert!(true) #[ignore]\"; // y.unwrap()\n    z.unwrap_or(1);\n";
        assert_eq!(kinds(source), [(method("unwrap_or"), 1)]);
    }

    #[test]
    fn an_attribute_lists_its_identifiers_and_not_its_strings() {
        assert_eq!(
            kinds("#[cfg_attr(miri, ignore)]\nfn a() {}\n"),
            [(attribute(&["cfg_attr", "miri", "ignore"]), 0)]
        );
        assert_eq!(
            kinds("#[doc = \"ignore\"]\nfn a() {}\n"),
            [(attribute(&["doc"]), 0)]
        );
    }

    /// The hint: a construct inside an item whose attribute spells a test says so, wherever
    /// the item stands, and nothing more is claimed for it. There is no proof of test code any
    /// more (second independent review, IR2-H-1 to IR2-H-4).
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
            "#[ cfg( test ) ]",
            // review V3 round 9: each segment of the path is the identifier it is. Before,
            // `r#cfg` was not `cfg`.
            "#[r#cfg(test)]",
            "#[r#tokio::r#test]",
            "#[::core::prelude::v1::test]",
            "#[std::prelude::v1::test]",
            "#[tokio::test(flavor = \"multi_thread\")]",
            "#[async_std::test]",
            "#[actix_rt::test]",
            "#[test_log::test]",
            "#[rstest::rstest]",
            "#[tokio::testing::test]",
        ] {
            let source = format!("fn real() {{}}\n{attribute}\nfn a() {{ b().unwrap(); }}\n");
            assert_eq!(hinted(&source), [true], "{attribute}");
        }
        for attribute in [
            "#[inline]",
            "#[cfg(not(test))]",
            "#[cfg_attr(test, allow(x))]",
            "#[testing]",
            "#[doc = \"test\"]",
            "#[cfg_attr(any(), test)]",
            "#[cfg_attr(feature = \"x\", test)]",
            "#[allow(test)]",
            "#[doc(alias = \"x\", test)]",
            "#[my::tests]",
            "#[cfg(any(test, feature = \"x\"))]",
            "#[cfg_attr(any(), cfg(test))]",
            "#[cfg(test = \"x\")]",
            "#[a::cfg(test)]",
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
        // a module that is only named `tests` spells nothing
        assert_eq!(
            hinted("mod tests {\n    pub fn production() { load().unwrap(); }\n}\n"),
            [false]
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

    /// Asking the tree for a node's parent or the sibling before it costs the node's depth, so
    /// items nested in one another were scanned on the square of the depth: 4,700 nested
    /// functions took 2.9 s (release). The walk keeps both itself. Here 3,000 modules nested
    /// in one another, each under a test attribute, are scanned within the bound, and the
    /// answer about each is unchanged: every one spells a test.
    #[test]
    fn items_nested_in_one_another_are_scanned_in_one_pass() {
        const DEPTH: usize = 3_000;
        let source = format!(
            "{}fn f() {{ a().unwrap(); }}{}",
            "#[test] mod m {".repeat(DEPTH),
            "}".repeat(DEPTH)
        );
        assert!(source.len() <= FRAGMENT_BYTE_BUDGET);
        let (done, wait) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = done.send(scan_fragment(Language::Rust, &source));
        });
        let found = wait
            .recv_timeout(std::time::Duration::from_secs(3))
            .expect("3,000 nested modules were scanned within three seconds")
            .expect("the fragment is within its budgets");
        assert_eq!(found.spelled_items.len(), DEPTH);
        let call = found
            .constructs
            .iter()
            .find(|c| matches!(&c.kind, ConstructKind::MethodCall { name } if name == "unwrap"))
            .expect("the call is read");
        assert!(call.test_spelled);
    }

    /// Review V3 round 8, VD3R8-M-1: a raw identifier is the identifier.
    #[test]
    fn raw_identifiers_are_named_without_their_prefix() {
        let found =
            kinds("fn f(x: X) {\n    x.r#unwrap();\n    r#todo!();\n}\n#[r#ignore]\nfn t() {}\n");
        let names: Vec<ConstructKind> = found.into_iter().map(|(kind, _)| kind).collect();
        assert_eq!(
            names,
            [
                method("unwrap"),
                macro_call("todo", ""),
                attribute(&["ignore"])
            ]
        );
    }

    /// Review V3 round 9, VD3R9-M-1, and the second independent review, IR2-M-2: a function
    /// named by a path is a construct like a method call, whether it is called or passed, and
    /// it is placed at its name. Before, the scanner named only `receiver.name(…)`: a path was
    /// left to the token rules, which read one line, and `X::` with `r#unwrap(x)` on the next
    /// line was read by nothing.
    #[test]
    fn a_path_call_and_a_path_value_are_constructs() {
        let named = |source: &str| -> Vec<(String, u32, u32)> {
            scan(source)
                .constructs
                .into_iter()
                .filter_map(|c| match c.kind {
                    ConstructKind::MethodCall { name } => Some((name, c.row, c.column)),
                    _ => None,
                })
                .collect()
        };
        let one = |name: &str, row: u32, column: u32| vec![(name.to_owned(), row, column)];
        for (source, column) in [
            ("fn f() { X::unwrap(x); }", 12),
            ("fn f() { <Option<u32>>::unwrap(x); }", 24),
            ("fn f() { Result::<u32, E>::unwrap(r); }", 27),
            ("fn f() { <X as T>::unwrap(x); }", 19),
            ("fn f() { a::b::unwrap::<u8>(x); }", 15),
            ("fn f() { self::unwrap(); }", 15),
            ("fn f() { let g = Option::<u32>::unwrap; }", 32),
            ("fn f() { X::r#unwrap(x); }", 12),
        ] {
            assert_eq!(named(source), one("unwrap", 0, column), "{source}");
        }
        // passed as a value: the call it is passed to is a construct too, and comes first
        assert_eq!(
            named("fn f() { it.map(Result::unwrap); }"),
            [("map".to_owned(), 0, 12), ("unwrap".to_owned(), 0, 24)]
        );
        // over two lines the construct is on the line of the name, and its own part is the name
        let broken = scan("fn f(x: X) {\n    X::\n        r#unwrap(x);\n}\n");
        assert_eq!(broken.constructs.len(), 1);
        let call = &broken.constructs[0];
        assert_eq!(call.kind, method("unwrap"));
        assert_eq!(
            (call.row, call.column, call.last_row, call.end),
            (2, 8, 2, (2, 16))
        );
        assert!(broken.names((2, 10)) && !broken.names((1, 4)));
        assert_eq!(
            named("fn f(it: I) {\n    let _ = it.map(Result::\n        expect);\n}\n"),
            [("map".to_owned(), 1, 15), ("expect".to_owned(), 2, 8)]
        );
        // a path that only leads to a longer one names the longer one's end
        assert_eq!(named("fn f() { a::unwrap::b(x); }"), one("b", 0, 20));
        // not values: an import, a macro's path, an attribute's, a visibility's, a type, a
        // definition, a field, and a name with no path
        for source in [
            "use a::expect;\nuse a::{b, unwrap};\nuse a::unwrap as c;\nuse a::unwrap::*;\n",
            "fn f() { X::unwrap!(x); }",
            "#![a::unwrap]\n#[b::expect]\nfn f() {}\n",
            "fn f() {\n    #![a::unwrap]\n}\n",
            "pub(in a::unwrap) fn f() {}",
            "fn f(a: a::unwrap, b: x::expect<u8>) -> q::unwrap { loop {} }",
            "impl X { fn unwrap(self) {} }\nfn expect() {}\n",
            "fn f() { let h = x.unwrap; }",
            "fn f() { unwrap(x); let g = unwrap; }",
            "fn f() { let v = X::unwrap { a: 1 }; }",
        ] {
            assert_eq!(named(source), [], "{source}");
        }
        // inside an item whose attribute spells a test it carries the hint, like a method call
        let hinted =
            scan("#[test]\nfn a() { Result::unwrap(b()); }\nfn c() { Result::unwrap(d()); }\n");
        let hints: Vec<bool> = hinted
            .constructs
            .iter()
            .filter(|c| c.kind == method("unwrap"))
            .map(|c| c.test_spelled)
            .collect();
        assert_eq!(hints, [true, false]);
    }

    /// Second independent review, IR2-M-2: `#[cfg_attr(all(), cfg_attr(all(), ignore))]`
    /// switches the test off and gave no `ignore` among its words. A `cfg_attr` that a
    /// `cfg_attr` applies is followed, and nothing else that is nested is.
    #[test]
    fn a_cfg_attr_applied_by_a_cfg_attr_gives_the_words_it_applies() {
        for (source, words) in [
            (
                "#[cfg_attr(all(), cfg_attr(all(), ignore))]",
                &["cfg_attr", "all", "cfg_attr", "ignore"][..],
            ),
            (
                "#[cfg_attr(a, cfg_attr(b, cfg_attr(c, ignore = \"x\")))]",
                &["cfg_attr", "a", "cfg_attr", "cfg_attr", "ignore"][..],
            ),
            (
                "#[cfg_attr(a, allow(x), cfg_attr /* c */ (b, inline, ignore))]",
                &["cfg_attr", "a", "allow", "cfg_attr", "inline", "ignore"][..],
            ),
            (
                "#[r#cfg_attr(a, r#cfg_attr(b, r#ignore))]",
                &["cfg_attr", "a", "cfg_attr", "ignore"][..],
            ),
            // the nested condition is no attribute, whatever it is called
            (
                "#[cfg_attr(all(), cfg_attr(ignore, allow(x)))]",
                &["cfg_attr", "all", "cfg_attr", "allow"][..],
            ),
            // in the place of the condition nothing is applied
            (
                "#[cfg_attr(cfg_attr(a, ignore), allow(x))]",
                &["cfg_attr", "cfg_attr", "allow"][..],
            ),
            // another attribute's list is that attribute's
            (
                "#[cfg_attr(a, serde(cfg_attr(b, ignore)))]",
                &["cfg_attr", "a", "serde"][..],
            ),
            (
                "#[other(a, cfg_attr(b, ignore))]",
                &["other", "a", "cfg_attr"][..],
            ),
        ] {
            assert_eq!(
                kinds(&format!("{source}\nfn t() {{}}\n"))[0].0,
                attribute(words),
                "{source}"
            );
        }
        // without recursion: a chain as deep as a fragment allows
        let depth = 4_000;
        let deep = format!(
            "#[cfg_attr(a, {}ignore{})]\nfn t() {{}}\n",
            "cfg_attr(a, ".repeat(depth),
            ")".repeat(depth)
        );
        assert!(deep.len() <= FRAGMENT_BYTE_BUDGET);
        let ConstructKind::Attribute { words, .. } = &scan(&deep).constructs[0].kind else {
            panic!("an attribute");
        };
        assert_eq!(words.len(), depth + 3);
        assert_eq!(words.last().map(String::as_str), Some("ignore"));
    }

    /// Second independent review, IR2-M-1: the words of an attribute leave out what is nested
    /// in a list and what is a string, so two attributes with the same words can be different
    /// attributes. The construct carries the attribute's whole text, without its whitespace.
    #[test]
    fn an_attribute_carries_its_whole_text_without_whitespace() {
        let attributes = |source: &str| -> Vec<ConstructKind> {
            scan(source)
                .constructs
                .into_iter()
                .map(|c| c.kind)
                .collect()
        };
        assert_eq!(
            attributes(
                "#[cfg_attr(\n    all( miri ),\n    ignore = \"very slow\"\n)]\nfn a() {\n    #![allow(x)]\n}\n"
            ),
            [
                ConstructKind::Attribute {
                    words: vec!["cfg_attr".into(), "all".into(), "ignore".into()],
                    text: "#[cfg_attr(all(miri),ignore=\"veryslow\")]".into(),
                },
                ConstructKind::Attribute {
                    words: vec!["allow".into(), "x".into()],
                    text: "#![allow(x)]".into(),
                },
            ]
        );
        for (before, after) in [
            ("all(miri)", "all()"),
            ("not(any())", "not(miri)"),
            ("target_os = \"plan9\"", "target_os = \"linux\""),
            ("feature = \"never-enabled\"", "feature = \"default\""),
            ("any(miri)", "any(miri, unix, windows)"),
        ] {
            let was = attributes(&format!("#[cfg_attr({before}, ignore)]\nfn a() {{}}\n"));
            let is = attributes(&format!("#[cfg_attr({after}, ignore)]\nfn a() {{}}\n"));
            let words = |kind: &ConstructKind| match kind {
                ConstructKind::Attribute { words, .. } => words.clone(),
                _ => Vec::new(),
            };
            assert_eq!(
                words(&was[0]),
                words(&is[0]),
                "{before}: the words are the same"
            );
            assert_ne!(was, is, "{before}: the attributes are not");
        }
        // only the spacing differs: the same attribute
        assert_eq!(
            attributes("#[cfg_attr(miri, ignore)]\nfn a() {}\n"),
            attributes("#[ cfg_attr(\n    miri,\n    ignore\n) ]\nfn a() {}\n")
        );
    }

    /// Independent review IR-M-1, review V3 round 8: the scan is a function of its input, so
    /// nothing in this file may ask a clock or parse under the crate's deadline (the other
    /// readers of this crate, which only show code, still do). A scan that depends on time
    /// cannot be shown by running it; this holds the source to it.
    #[test]
    fn the_fragment_scan_asks_no_clock() {
        let source = include_str!("fragment.rs");
        let code = source
            .split_once("#[cfg(test)]\nmod tests {")
            .expect("the tests module is the end of the file")
            .0;
        for word in [
            "parse_bounded",
            "parse_within",
            "Instant",
            "elapsed",
            "progress_callback",
        ] {
            assert!(!code.contains(word), "the fragment scanner names `{word}`");
        }
    }
}
