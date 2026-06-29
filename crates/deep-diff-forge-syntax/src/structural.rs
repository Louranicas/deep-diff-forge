//! Token-level structural (AST-leaf) diffing.
//!
//! Unlike a line-oriented diff, this compares the *token streams* produced by
//! tree-sitter — the grammar's leaf nodes, with whitespace and layout dropped.
//! Two consequences follow directly:
//!
//! - **Reformat-aware.** A change that only reflows whitespace/newlines yields
//!   identical token streams, so it reports zero structural change
//!   ([`StructuralDiff::reformat_only`]).
//! - **Moved-block aware (best-effort).** A *contiguous* run of removed tokens
//!   that reappears verbatim as a contiguous run of added tokens is reclassified
//!   as [`ChangeKind::Moved`]. This fires for context-free identical runs; it does
//!   not reconstruct moves that flat-token LCS matches in place through shared
//!   punctuation (that needs the full tree, as difftastic does).
//!
//! This is an honest token/leaf-level structural diff (LCS over the token
//! sequence, the approach in difftastic's `lcs_diff`), not the optimal
//! tree-edit-distance graph diff. It never touches patch truth: it operates on
//! source bytes and only describes them.

use crate::language::Language;

/// Above this token-pair product the quadratic LCS is skipped and a coarse
/// whole-file remove+add is reported with [`StructuralDiff::degraded`] set —
/// so a pathological input degrades gracefully instead of exhausting memory.
const LCS_CELL_BUDGET: usize = 4_000_000;

/// One lexical token from a source file (a tree-sitter leaf).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    /// The token text (no surrounding whitespace).
    pub text: String,
    /// 1-based source line where the token starts.
    pub line: u32,
}

/// How a token changed between the two sides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    /// Present, unchanged, on both sides.
    Unchanged,
    /// Added on the new side.
    Added,
    /// Removed from the old side.
    Removed,
    /// Part of a block that moved (same tokens, different position).
    Moved,
}

impl ChangeKind {
    /// Stable lowercase label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Unchanged => "unchanged",
            Self::Added => "added",
            Self::Removed => "removed",
            Self::Moved => "moved",
        }
    }
}

/// One token in the structural diff, with its classification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenChange {
    /// What happened to the token.
    pub kind: ChangeKind,
    /// The token text.
    pub text: String,
    /// 1-based line on the old side, if present there.
    pub old_line: Option<u32>,
    /// 1-based line on the new side, if present there.
    pub new_line: Option<u32>,
}

/// The result of structurally diffing two sources.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructuralDiff {
    /// Language used to tokenize (must match on both sides).
    pub language: Language,
    /// Per-token classification, in a merged old→new order.
    pub changes: Vec<TokenChange>,
    /// Count of unchanged tokens.
    pub unchanged: usize,
    /// Count of added tokens.
    pub added: usize,
    /// Count of removed tokens.
    pub removed: usize,
    /// Count of tokens belonging to a moved block.
    pub moved: usize,
    /// True when the token streams are identical (only formatting differs).
    pub reformat_only: bool,
    /// True when the LCS was skipped for size and a coarse diff was produced.
    pub degraded: bool,
}

/// Structurally diff `old_source` against `new_source` for `language`.
///
/// For an unsupported language both sides tokenize to empty and the diff is an
/// empty, `reformat_only` result (callers should fall back to a line diff).
#[must_use]
pub fn structural_diff(language: Language, old_source: &str, new_source: &str) -> StructuralDiff {
    let old = tokenize(language, old_source);
    let new = tokenize(language, new_source);
    diff_tokens(language, &old, &new)
}

/// Tokenize `source` into its tree-sitter leaf tokens (source order).
#[must_use]
pub fn tokenize(language: Language, source: &str) -> Vec<Token> {
    if language != Language::Rust {
        return Vec::new();
    }
    // Guard against unbounded parse + allocation cost on adversarially large
    // inputs; mirrors the byte_budget applied in analyze_language().
    if source.len() > crate::analyze::DEFAULT_BYTE_BUDGET {
        return Vec::new();
    }
    let ts_language: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&ts_language).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };

    let mut tokens = Vec::new();
    // Iterative pre-order leaf walk (no recursion → no stack overflow on deep
    // trees). Children are pushed reversed so popping yields source order.
    let mut stack = vec![tree.root_node()];
    while let Some(node) = stack.pop() {
        if node.child_count() == 0 {
            if let Some(text) = source.get(node.start_byte()..node.end_byte()) {
                if !text.trim().is_empty() {
                    tokens.push(Token {
                        text: text.to_string(),
                        #[allow(clippy::cast_possible_truncation)]
                        line: node.start_position().row as u32 + 1,
                    });
                }
            }
            continue;
        }
        let mut cursor = node.walk();
        let children: Vec<_> = node.children(&mut cursor).collect();
        for child in children.into_iter().rev() {
            stack.push(child);
        }
    }
    tokens
}

fn diff_tokens(language: Language, old: &[Token], new: &[Token]) -> StructuralDiff {
    // Graceful degradation: a huge pair would make the quadratic LCS table blow
    // up, so report a coarse whole-file change instead.
    if old.len().saturating_mul(new.len()) > LCS_CELL_BUDGET {
        return coarse_diff(language, old, new);
    }

    let ops = lcs_ops(old, new);
    let mut changes = mark_moves(&ops, old, new);

    let (mut unchanged, mut added, mut removed, mut moved) = (0, 0, 0, 0);
    for c in &changes {
        match c.kind {
            ChangeKind::Unchanged => unchanged += 1,
            ChangeKind::Added => added += 1,
            ChangeKind::Removed => removed += 1,
            ChangeKind::Moved => moved += 1,
        }
    }
    // A reformat-only edit changes no tokens at all.
    let reformat_only = added == 0 && removed == 0 && moved == 0;
    // Drop the (large) per-token vector when nothing changed structurally —
    // the counts already say "reformat only".
    if reformat_only {
        changes.clear();
    }

    StructuralDiff {
        language,
        changes,
        unchanged,
        added,
        removed,
        moved,
        reformat_only,
        degraded: false,
    }
}

/// One LCS operation, in merged order.
enum Op {
    Equal(usize, usize),
    Delete(usize),
    Insert(usize),
}

/// Longest-common-subsequence diff over token text. Classic O(n·m) DP +
/// backtrack; bounded by [`LCS_CELL_BUDGET`] at the call site.
fn lcs_ops(old: &[Token], new: &[Token]) -> Vec<Op> {
    let n = old.len();
    let m = new.len();
    // table[i][j] = LCS length of old[i..] and new[j..]; (n+1)·(m+1) cells.
    let width = m + 1;
    let mut table = vec![0u32; (n + 1) * width];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            let idx = i * width + j;
            table[idx] = if old[i].text == new[j].text {
                table[(i + 1) * width + (j + 1)] + 1
            } else {
                table[(i + 1) * width + j].max(table[i * width + (j + 1)])
            };
        }
    }

    let mut ops = Vec::with_capacity(n + m);
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if old[i].text == new[j].text {
            ops.push(Op::Equal(i, j));
            i += 1;
            j += 1;
        } else if table[(i + 1) * width + j] >= table[i * width + (j + 1)] {
            ops.push(Op::Delete(i));
            i += 1;
        } else {
            ops.push(Op::Insert(j));
            j += 1;
        }
    }
    while i < n {
        ops.push(Op::Delete(i));
        i += 1;
    }
    while j < m {
        ops.push(Op::Insert(j));
        j += 1;
    }
    ops
}

/// Minimum token run length to be considered a "moved block" (short identical
/// runs like a lone `}` are noise, not moves).
const MIN_MOVE_RUN: usize = 3;

/// Convert LCS ops to [`TokenChange`]s, reclassifying matching delete/insert
/// runs as moves.
fn mark_moves(ops: &[Op], old: &[Token], new: &[Token]) -> Vec<TokenChange> {
    // Collect contiguous delete and insert runs (index ranges into `ops`).
    let mut delete_runs: Vec<(usize, usize)> = Vec::new();
    let mut insert_runs: Vec<(usize, usize)> = Vec::new();
    let mut k = 0;
    while k < ops.len() {
        match ops[k] {
            Op::Delete(_) => {
                let start = k;
                while k < ops.len() && matches!(ops[k], Op::Delete(_)) {
                    k += 1;
                }
                delete_runs.push((start, k));
            }
            Op::Insert(_) => {
                let start = k;
                while k < ops.len() && matches!(ops[k], Op::Insert(_)) {
                    k += 1;
                }
                insert_runs.push((start, k));
            }
            Op::Equal(_, _) => k += 1,
        }
    }

    let run_text = |run: (usize, usize)| -> Vec<&str> {
        ops[run.0..run.1]
            .iter()
            .filter_map(|op| match op {
                Op::Delete(i) => Some(old[*i].text.as_str()),
                Op::Insert(j) => Some(new[*j].text.as_str()),
                Op::Equal(_, _) => None,
            })
            .collect()
    };

    // A delete run and an insert run with identical token text (length >=
    // MIN_MOVE_RUN) are a moved block. Each insert run is matched at most once.
    let mut moved_ops: std::collections::HashSet<usize> = std::collections::HashSet::new();
    let mut used_inserts: Vec<bool> = vec![false; insert_runs.len()];
    for d in &delete_runs {
        let dt = run_text(*d);
        if dt.len() < MIN_MOVE_RUN {
            continue;
        }
        for (ii, ins) in insert_runs.iter().enumerate() {
            if used_inserts[ii] {
                continue;
            }
            if run_text(*ins) == dt {
                used_inserts[ii] = true;
                for idx in d.0..d.1 {
                    moved_ops.insert(idx);
                }
                for idx in ins.0..ins.1 {
                    moved_ops.insert(idx);
                }
                break;
            }
        }
    }

    ops.iter()
        .enumerate()
        .map(|(idx, op)| match op {
            Op::Equal(i, j) => TokenChange {
                kind: ChangeKind::Unchanged,
                text: old[*i].text.clone(),
                old_line: Some(old[*i].line),
                new_line: Some(new[*j].line),
            },
            Op::Delete(i) => TokenChange {
                kind: if moved_ops.contains(&idx) {
                    ChangeKind::Moved
                } else {
                    ChangeKind::Removed
                },
                text: old[*i].text.clone(),
                old_line: Some(old[*i].line),
                new_line: None,
            },
            Op::Insert(j) => TokenChange {
                kind: if moved_ops.contains(&idx) {
                    ChangeKind::Moved
                } else {
                    ChangeKind::Added
                },
                text: new[*j].text.clone(),
                old_line: None,
                new_line: Some(new[*j].line),
            },
        })
        .collect()
}

/// Coarse fallback: report every old token removed and every new token added.
fn coarse_diff(language: Language, old: &[Token], new: &[Token]) -> StructuralDiff {
    let mut changes = Vec::with_capacity(old.len() + new.len());
    for t in old {
        changes.push(TokenChange {
            kind: ChangeKind::Removed,
            text: t.text.clone(),
            old_line: Some(t.line),
            new_line: None,
        });
    }
    for t in new {
        changes.push(TokenChange {
            kind: ChangeKind::Added,
            text: t.text.clone(),
            old_line: None,
            new_line: Some(t.line),
        });
    }
    StructuralDiff {
        language,
        changes,
        unchanged: 0,
        added: new.len(),
        removed: old.len(),
        moved: 0,
        reformat_only: false,
        degraded: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenize_drops_whitespace() {
        let toks = tokenize(Language::Rust, "fn  main ( )  {\n}\n");
        let texts: Vec<&str> = toks.iter().map(|t| t.text.as_str()).collect();
        assert_eq!(texts, vec!["fn", "main", "(", ")", "{", "}"]);
    }

    #[test]
    fn tokenize_unsupported_is_empty() {
        assert!(tokenize(Language::Unsupported, "fn main() {}").is_empty());
    }

    #[test]
    fn identical_sources_are_all_unchanged() {
        let src = "fn main() { let x = 1; }";
        let d = structural_diff(Language::Rust, src, src);
        assert!(d.reformat_only);
        assert_eq!(d.added, 0);
        assert_eq!(d.removed, 0);
        assert!(d.unchanged > 0);
    }

    #[test]
    fn reformatting_only_is_detected() {
        // Same tokens, wildly different whitespace/layout.
        let old = "fn main(){let x=1;}";
        let new = "fn   main ( ) {\n    let x = 1 ;\n}\n";
        let d = structural_diff(Language::Rust, old, new);
        assert!(
            d.reformat_only,
            "pure reformat must report no structural change"
        );
        assert_eq!(d.added + d.removed + d.moved, 0);
    }

    #[test]
    fn added_token_is_classified() {
        let old = "fn f() { a(); }";
        let new = "fn f() { a(); b(); }";
        let d = structural_diff(Language::Rust, old, new);
        assert!(!d.reformat_only);
        assert!(d.added > 0);
        assert!(
            d.changes
                .iter()
                .any(|c| c.kind == ChangeKind::Added && c.text == "b")
        );
    }

    #[test]
    fn removed_token_is_classified() {
        let old = "fn f() { a(); b(); }";
        let new = "fn f() { a(); }";
        let d = structural_diff(Language::Rust, old, new);
        assert!(d.removed > 0);
        assert!(
            d.changes
                .iter()
                .any(|c| c.kind == ChangeKind::Removed && c.text == "b")
        );
    }

    #[test]
    fn renamed_identifier_is_remove_plus_add_not_reformat() {
        let old = "fn alpha() {}";
        let new = "fn bravo() {}";
        let d = structural_diff(Language::Rust, old, new);
        assert!(!d.reformat_only);
        assert!(
            d.changes
                .iter()
                .any(|c| c.kind == ChangeKind::Removed && c.text == "alpha")
        );
        assert!(
            d.changes
                .iter()
                .any(|c| c.kind == ChangeKind::Added && c.text == "bravo")
        );
    }

    #[test]
    fn move_detection_reclassifies_matching_runs() {
        // Deterministic unit test of the move algorithm: a contiguous deleted run
        // whose token text matches a contiguous inserted run (length >=
        // MIN_MOVE_RUN) is reclassified Moved, not Removed+Added.
        let old = vec![
            Token {
                text: "keep".into(),
                line: 1,
            },
            Token {
                text: "p".into(),
                line: 2,
            },
            Token {
                text: "q".into(),
                line: 2,
            },
            Token {
                text: "r".into(),
                line: 2,
            },
        ];
        let new = vec![
            Token {
                text: "p".into(),
                line: 1,
            },
            Token {
                text: "q".into(),
                line: 1,
            },
            Token {
                text: "r".into(),
                line: 1,
            },
            Token {
                text: "keep".into(),
                line: 2,
            },
        ];
        let ops = vec![
            Op::Insert(0),
            Op::Insert(1),
            Op::Insert(2),
            Op::Equal(0, 3),
            Op::Delete(1),
            Op::Delete(2),
            Op::Delete(3),
        ];
        let changes = mark_moves(&ops, &old, &new);
        let moved = changes
            .iter()
            .filter(|c| c.kind == ChangeKind::Moved)
            .count();
        assert_eq!(moved, 6, "the matching p/q/r runs (3 + 3) should be Moved");
        assert!(
            changes
                .iter()
                .any(|c| c.kind == ChangeKind::Unchanged && c.text == "keep")
        );
    }

    #[test]
    fn reordering_is_at_least_not_reformat_only() {
        // Reordering distinct statements is a real change (not pure reformatting).
        // Whether it surfaces as Moved vs Removed+Added depends on token overlap
        // with the surrounding context — flat-token move detection is best-effort
        // for contiguous, context-free identical runs (see the unit test above).
        let old = "fn a() { alpha(); } fn b() { bravo(); }";
        let new = "fn b() { bravo(); } fn a() { alpha(); }";
        let d = structural_diff(Language::Rust, old, new);
        assert!(!d.reformat_only);
    }

    #[test]
    fn short_runs_are_not_moves() {
        // A single `}` reappearing is below MIN_MOVE_RUN and must not be a move.
        let old = "fn a() {}";
        let new = "fn a() {} fn b() {}";
        let d = structural_diff(Language::Rust, old, new);
        assert_eq!(d.moved, 0);
        assert!(d.added > 0);
    }

    #[test]
    fn counts_are_consistent_with_changes() {
        let old = "fn f() { a(); b(); }";
        let new = "fn f() { a(); c(); }";
        let d = structural_diff(Language::Rust, old, new);
        let total = d
            .changes
            .iter()
            .filter(|c| c.kind != ChangeKind::Unchanged)
            .count();
        assert_eq!(total, d.added + d.removed + d.moved);
    }

    #[test]
    fn change_kind_labels() {
        assert_eq!(ChangeKind::Added.label(), "added");
        assert_eq!(ChangeKind::Moved.label(), "moved");
    }

    #[test]
    fn empty_to_nonempty_is_all_added() {
        let d = structural_diff(Language::Rust, "", "fn main() {}");
        assert!(d.added > 0);
        assert_eq!(d.removed, 0);
        assert!(!d.reformat_only);
    }
}
