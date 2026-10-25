// DDF integration: R08, R10, R11. Never turn a failed analysis into formatting-only or behavioral safety; parser time cancellation is cooperative, node limits are post-parse.
// Design/status: docs/RECOMMENDATION_INTEGRATION.md#syntax-availability
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fdeep-diff-forge-syntax--structural
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

/// Above this token-pair product (of the part between the common prefix and
/// suffix) the quadratic LCS is skipped and a coarse
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
    /// Why structural analysis is unavailable; never interpreted as unchanged.
    pub unavailable: Option<deep_diff_forge_core::FallbackReason>,
}

/// Structurally diff `old_source` against `new_source` for `language`.
///
/// Unsupported, oversized or invalid sources produce an explicit unavailable
/// result. Such a result never asserts that a change is formatting only.
#[must_use]
pub fn structural_diff(language: Language, old_source: &str, new_source: &str) -> StructuralDiff {
    match (
        tokenize_checked(language, old_source),
        tokenize_checked(language, new_source),
    ) {
        (Ok(old), Ok(new)) => diff_tokens(language, &old, &new),
        (Err(reason), _) | (_, Err(reason)) => StructuralDiff {
            language,
            changes: Vec::new(),
            unchanged: 0,
            added: 0,
            removed: 0,
            moved: 0,
            reformat_only: false,
            degraded: true,
            unavailable: Some(reason),
        },
    }
}

/// Tokenize `source` into its tree-sitter leaf tokens (source order).
#[must_use]
pub fn tokenize(language: Language, source: &str) -> Vec<Token> {
    tokenize_checked(language, source).unwrap_or_default()
}

/// Tokenize with explicit failure information. Use this for correctness decisions.
/// The compatibility `tokenize` helper alone cannot distinguish empty from failed.
///
/// # Errors
/// Returns a typed fallback for unsupported, oversized, erroneous or over-budget trees.
pub fn tokenize_checked(
    language: Language,
    source: &str,
) -> Result<Vec<Token>, deep_diff_forge_core::FallbackReason> {
    use deep_diff_forge_core::FallbackReason;
    if language != Language::Rust {
        return Err(FallbackReason::UnsupportedLanguage);
    }
    // Guard against unbounded parse + allocation cost on adversarially large
    // inputs; mirrors the byte_budget applied in analyze_language().
    if source.len() > crate::analyze::DEFAULT_BYTE_BUDGET {
        return Err(FallbackReason::ByteBudgetExceeded);
    }
    let ts_language: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&ts_language).is_err() {
        return Err(FallbackReason::UnsupportedLanguage);
    }
    let Some(tree) = crate::parse_bounded(&mut parser, source) else {
        return Err(FallbackReason::TimeBudgetExceeded);
    };
    if tree.root_node().has_error() {
        return Err(FallbackReason::ParseErrorsExceeded);
    }

    let mut tokens = Vec::new();
    // Iterative pre-order leaf walk (no recursion → no stack overflow on deep
    // trees). Children are pushed reversed so popping yields source order.
    let mut stack = vec![tree.root_node()];
    let mut visited = 0usize;
    while let Some(node) = stack.pop() {
        visited += 1;
        if visited > 200_000 {
            return Err(FallbackReason::NodeBudgetExceeded);
        }
        if node.child_count() == 0 {
            if let Some(text) = source.get(node.start_byte()..node.end_byte())
                && !text.trim().is_empty()
            {
                tokens.push(Token {
                    text: text.to_string(),
                    line: u32::try_from(node.start_position().row + 1).unwrap_or(u32::MAX),
                });
            }
            continue;
        }
        let mut cursor = node.walk();
        let children: Vec<_> = node.children(&mut cursor).collect();
        for child in children.into_iter().rev() {
            stack.push(child);
        }
    }
    Ok(tokens)
}

fn diff_tokens(language: Language, old: &[Token], new: &[Token]) -> StructuralDiff {
    // Graceful degradation: a huge changed middle would make the quadratic LCS
    // table blow up, so report a coarse whole-file change instead.
    let (prefix, suffix) = common_affixes(old, new);
    let middle = (old.len() - prefix - suffix).saturating_mul(new.len() - prefix - suffix);
    if middle > LCS_CELL_BUDGET {
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
        unavailable: None,
    }
}

/// One LCS operation, in merged order.
enum Op {
    Equal(usize, usize),
    Delete(usize),
    Insert(usize),
}

/// Lengths of the longest common token prefix and, of what remains, the longest common suffix.
/// A common prefix or suffix always belongs to some longest common subsequence, so only the
/// middle needs the quadratic table; this keeps a small edit to a large file within budget.
fn common_affixes(old: &[Token], new: &[Token]) -> (usize, usize) {
    let prefix = old
        .iter()
        .zip(new)
        .take_while(|(a, b)| a.text == b.text)
        .count();
    let suffix = old[prefix..]
        .iter()
        .rev()
        .zip(new[prefix..].iter().rev())
        .take_while(|(a, b)| a.text == b.text)
        .count();
    (prefix, suffix)
}

/// Longest-common-subsequence diff over token text. Classic O(n·m) DP +
/// backtrack over the part between the common prefix and suffix; bounded by
/// [`LCS_CELL_BUDGET`] at the call site.
fn lcs_ops(old: &[Token], new: &[Token]) -> Vec<Op> {
    let (prefix, suffix) = common_affixes(old, new);
    let n = old.len() - prefix - suffix;
    let m = new.len() - prefix - suffix;
    let (old_mid, new_mid) = (&old[prefix..prefix + n], &new[prefix..prefix + m]);
    // table[i][j] = LCS length of old_mid[i..] and new_mid[j..]; (n+1)·(m+1) cells.
    let width = m + 1;
    let mut table = vec![0u32; (n + 1) * width];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            let idx = i * width + j;
            table[idx] = if old_mid[i].text == new_mid[j].text {
                table[(i + 1) * width + (j + 1)] + 1
            } else {
                table[(i + 1) * width + j].max(table[i * width + (j + 1)])
            };
        }
    }

    let mut ops = Vec::with_capacity(old.len() + new.len());
    ops.extend((0..prefix).map(|k| Op::Equal(k, k)));
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if old_mid[i].text == new_mid[j].text {
            ops.push(Op::Equal(prefix + i, prefix + j));
            i += 1;
            j += 1;
        } else if table[(i + 1) * width + j] >= table[i * width + (j + 1)] {
            ops.push(Op::Delete(prefix + i));
            i += 1;
        } else {
            ops.push(Op::Insert(prefix + j));
            j += 1;
        }
    }
    while i < n {
        ops.push(Op::Delete(prefix + i));
        i += 1;
    }
    while j < m {
        ops.push(Op::Insert(prefix + j));
        j += 1;
    }
    ops.extend((0..suffix).map(|k| Op::Equal(prefix + n + k, prefix + m + k)));
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
        unavailable: None,
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

    /// Independent forward-DP reference for the length of the longest common subsequence.
    fn reference_lcs_len(a: &[&str], b: &[&str]) -> usize {
        let mut prev = vec![0usize; b.len() + 1];
        for x in a {
            let mut row = vec![0usize; b.len() + 1];
            for (j, y) in b.iter().enumerate() {
                row[j + 1] = if x == y {
                    prev[j] + 1
                } else {
                    row[j].max(prev[j + 1])
                };
            }
            prev = row;
        }
        prev[b.len()]
    }

    /// Every sequence of length 0..=`max_len` over `alphabet`.
    fn all_sequences(alphabet: &[&'static str], max_len: usize) -> Vec<Vec<&'static str>> {
        let mut all = vec![Vec::new()];
        let mut frontier = vec![Vec::new()];
        for _ in 0..max_len {
            frontier = frontier
                .iter()
                .flat_map(|seq: &Vec<&'static str>| {
                    alphabet.iter().map(move |t| {
                        let mut next = seq.clone();
                        next.push(*t);
                        next
                    })
                })
                .collect();
            all.extend(frontier.iter().cloned());
        }
        all
    }

    // Mutation testing found that corrupting the DP's diagonal (match) cell survived every
    // fixture test: greedy backtracking hid it. Check optimality and edit-script validity
    // exhaustively on small inputs instead.
    #[test]
    fn lcs_ops_is_a_valid_optimal_edit_script() {
        let sequences = all_sequences(&["a", "b", "c"], 5);
        let tokens = |seq: &[&str]| -> Vec<Token> {
            seq.iter()
                .map(|t| Token {
                    text: (*t).to_owned(),
                    line: 1,
                })
                .collect()
        };
        for old in &sequences {
            for new in &sequences {
                let ops = lcs_ops(&tokens(old), &tokens(new));
                let (mut next_old, mut next_new, mut equal) = (0, 0, 0);
                for op in &ops {
                    match *op {
                        Op::Equal(i, j) => {
                            assert_eq!((i, j), (next_old, next_new), "{old:?} -> {new:?}");
                            assert_eq!(old[i], new[j], "{old:?} -> {new:?}");
                            next_old += 1;
                            next_new += 1;
                            equal += 1;
                        }
                        Op::Delete(i) => {
                            assert_eq!(i, next_old, "{old:?} -> {new:?}");
                            next_old += 1;
                        }
                        Op::Insert(j) => {
                            assert_eq!(j, next_new, "{old:?} -> {new:?}");
                            next_new += 1;
                        }
                    }
                }
                assert_eq!((next_old, next_new), (old.len(), new.len()));
                assert_eq!(
                    equal,
                    reference_lcs_len(old, new),
                    "not a longest common subsequence: {old:?} -> {new:?}"
                );
            }
        }
    }

    // Council r6/r7: every real DDF source file above ~2,000 tokens degraded to partial_lcs,
    // however small the edit, because the budget applied to the whole-file product.
    #[test]
    fn a_small_edit_to_a_file_beyond_the_cell_budget_is_still_analyzed() {
        // 800 one-line functions; `name(i)` names the i-th.
        let source = |name: &dyn Fn(usize) -> String| -> String {
            let mut text = String::new();
            for i in 0..800 {
                text.push_str("fn ");
                text.push_str(&name(i));
                text.push_str("() {}\n");
            }
            text
        };
        let old = source(&|i| format!("f{i}"));
        let new = source(&|i| {
            if i == 400 {
                "renamed".into()
            } else {
                format!("f{i}")
            }
        });
        let tokens = tokenize(Language::Rust, &old).len();
        assert!(tokens * tokens > LCS_CELL_BUDGET, "{tokens} tokens");
        let d = structural_diff(Language::Rust, &old, &new);
        assert!(!d.degraded);
        assert_eq!((d.added, d.removed, d.moved), (1, 1, 0));
        assert_eq!(d.unchanged, tokens - 1);
        let changed: Vec<_> = d
            .changes
            .iter()
            .filter(|c| c.kind != ChangeKind::Unchanged)
            .map(|c| (c.kind, c.text.as_str(), c.old_line, c.new_line))
            .collect();
        assert_eq!(
            changed,
            [
                (ChangeKind::Removed, "f400", Some(401), None),
                (ChangeKind::Added, "renamed", None, Some(401)),
            ]
        );
        // Two edits 300 functions apart: a 1,800-token middle on each side (3.24 M cells) is
        // analysed exactly, so the budget is applied to the middle on both sides, not the files.
        let two_edits = source(&|i| match i {
            400 | 700 => format!("e{i}"),
            _ => format!("f{i}"),
        });
        let d = structural_diff(Language::Rust, &old, &two_edits);
        assert!(!d.degraded);
        assert_eq!((d.added, d.removed), (2, 2));
        // A changed middle that is itself beyond the budget still degrades.
        let rewritten = source(&|i| format!("g{i}"));
        assert!(structural_diff(Language::Rust, &old, &rewritten).degraded);
    }

    #[test]
    fn moved_blocks_are_counted_in_the_summary() {
        let old = "fn a() { alpha(); beta(); gamma(); } fn b() { one(); } fn c() { two(); }";
        let new = "fn b() { one(); } fn c() { two(); } fn a() { alpha(); beta(); gamma(); }";
        let d = structural_diff(Language::Rust, old, new);
        let moved = d
            .changes
            .iter()
            .filter(|c| c.kind == ChangeKind::Moved)
            .count();
        assert!(moved > 0);
        assert_eq!(d.moved, moved);
    }

    #[test]
    fn tokenize_unsupported_is_empty() {
        assert_eq!(tokenize(Language::Unsupported, "fn main() {}").len(), 0);
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

#[cfg(test)]
mod failure_regressions {
    use super::*;
    #[test]
    fn oversize_invalid_and_unsupported_never_report_formatting_only() {
        for (language, old, new) in [
            (
                Language::Rust,
                "x".repeat(2 * 1024 * 1024 + 1),
                "y".repeat(2 * 1024 * 1024 + 1),
            ),
            (Language::Rust, "fn {".into(), "fn }".into()),
            (Language::Unsupported, "old".into(), "new".into()),
        ] {
            let diff = structural_diff(language, &old, &new);
            assert!(!diff.reformat_only);
            assert!(diff.degraded);
            assert!(diff.unavailable.is_some());
        }
    }
}
