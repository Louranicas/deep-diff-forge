// DDF integration: R08, R10, R11. Never turn a failed analysis into formatting-only or behavioral safety; parser time cancellation is cooperative, node limits are post-parse.
// Design/status: docs/RECOMMENDATION_INTEGRATION.md#syntax-availability
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fdeep-diff-forge-syntax--structural
//! Token-level structural (AST-leaf) diffing.
//!
//! Unlike a line-oriented diff, this compares the *token streams* produced by
//! tree-sitter — the grammar's leaf nodes, with whitespace and layout dropped. A
//! comment or a string literal is one token of its exact text (whitespace inside a
//! literal is its value).
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
//!
//! **Bounded work.** The common token prefix and suffix are matched first, and
//! the exact LCS runs on the changed middle when that fits [`LCS_CELL_BUDGET`].
//! A larger middle is anchored on unchanged lines (an LCS over lines, each
//! identified by its token texts) and the token LCS runs on each gap between
//! anchors: a valid but not necessarily minimal script
//! ([`StructuralDiff::minimal`] is false). Only when the line table, or the gaps'
//! cells together, are over budget does the file degrade to a coarse whole-file
//! change ([`StructuralDiff::degraded`]).

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
    /// How the script was produced. One state replaces three flags and an `Option` whose
    /// contradictory combinations (degraded and minimal, reformat-only and unavailable) could be
    /// built before (anchor C-06).
    pub state: StructuralState,
}

/// How a [`StructuralDiff`] was produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StructuralState {
    /// No structural analysis: unsupported, oversized or invalid input. Never read as unchanged.
    Unavailable(deep_diff_forge_core::FallbackReason),
    /// The changed middle was too large for any token table: one coarse whole-file change.
    Degraded,
    /// Anchored on unchanged lines: a valid script, not necessarily minimal, with real edits.
    LineAnchored,
    /// A minimal (longest-common-subsequence) script; `reformat_only` when it has no edits, so
    /// the token streams are identical and only formatting differs.
    Minimal {
        /// No token was added, removed or moved.
        reformat_only: bool,
    },
}

impl StructuralDiff {
    /// True when the token streams are identical (only formatting differs).
    #[must_use]
    pub fn reformat_only(&self) -> bool {
        matches!(
            self.state,
            StructuralState::Minimal {
                reformat_only: true
            }
        )
    }

    /// True when no full token script was produced (coarse, or unavailable).
    #[must_use]
    pub fn degraded(&self) -> bool {
        matches!(
            self.state,
            StructuralState::Degraded | StructuralState::Unavailable(_)
        )
    }

    /// True when `changes` is a minimal (longest-common-subsequence) script.
    #[must_use]
    pub fn minimal(&self) -> bool {
        matches!(self.state, StructuralState::Minimal { .. })
    }

    /// Why structural analysis is unavailable, if it is.
    #[must_use]
    pub fn unavailable(&self) -> Option<&deep_diff_forge_core::FallbackReason> {
        match &self.state {
            StructuralState::Unavailable(reason) => Some(reason),
            _ => None,
        }
    }
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
            state: StructuralState::Unavailable(reason),
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
        // A comment is one token holding its exact bytes. In tree-sitter-rust a plain comment's
        // text is no leaf (only its `//` or `/*` is), so a rewritten comment, such as a changed
        // `// SAFETY:` justification, used to change no token and was called reformat-only.
        // A string literal is one token too: its whitespace-only content was dropped with the
        // layout and a raw string's delimiters are no leaf, so `""` to `" "`, a tab to two
        // spaces and `f(a)` to `f(r"a")` changed no token (bug hunt 2026-11-01).
        if node.child_count() == 0 || is_whole_token(node.kind()) {
            if let Some(text) = source.get(node.start_byte()..node.end_byte())
                && !text.trim().is_empty()
            {
                tokens.push(Token {
                    text: if is_comment(node.kind()) {
                        comment_token(text)
                    } else {
                        text.to_string()
                    },
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

/// A comment's token: its text with each line's trailing whitespace, CR included, removed.
/// A line comment's node ends with its line feed, so CRLF conversion or a trailing space
/// would otherwise read as a rewritten comment (bug hunt 2026-10-28, H4-05); any change to
/// the words still changes the token.
fn comment_token(text: &str) -> String {
    text.lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
}

/// Whether a tree-sitter node kind is a comment, whose whole text is one token.
fn is_comment(kind: &str) -> bool {
    matches!(kind, "line_comment" | "block_comment")
}

/// Whether a node's whole text is one token: a comment, or a string literal (plain, byte, C or
/// raw), whose exact bytes, whitespace included, are its value.
fn is_whole_token(kind: &str) -> bool {
    is_comment(kind) || matches!(kind, "string_literal" | "raw_string_literal")
}

fn diff_tokens(language: Language, old: &[Token], new: &[Token]) -> StructuralDiff {
    // Graceful degradation: a huge changed middle would make the quadratic LCS
    // table blow up, so report a coarse whole-file change instead.
    // A middle too large for one token table is anchored on unchanged lines first; only when a
    // gap between anchors is itself too large does the whole file degrade.
    let (prefix, suffix) = common_affixes(old, new);
    let middle = (old.len() - prefix - suffix).saturating_mul(new.len() - prefix - suffix);
    let (ops, minimal) = if middle <= LCS_CELL_BUDGET {
        (lcs_ops(old, new), true)
    } else if let Some(ops) = line_anchored_ops(old, new) {
        (ops, false)
    } else {
        return coarse_diff(language, old, new);
    };
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
        // An empty edit script is minimal whichever path produced it.
        state: if reformat_only {
            StructuralState::Minimal {
                reformat_only: true,
            }
        } else if minimal {
            StructuralState::Minimal {
                reformat_only: false,
            }
        } else {
            StructuralState::LineAnchored
        },
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

/// Token index ranges of each source line in `tokens` (tokens arrive in source order).
fn line_runs(tokens: &[Token]) -> Vec<std::ops::Range<usize>> {
    let mut runs: Vec<std::ops::Range<usize>> = Vec::new();
    for (k, token) in tokens.iter().enumerate() {
        match runs.last_mut() {
            Some(run) if tokens[run.start].line == token.line => run.end = k + 1,
            _ => runs.push(k..k + 1),
        }
    }
    runs
}

/// The ID of each line's token texts in `ids`, assigning new IDs in first-seen order.
fn intern_lines<'a>(
    ids: &mut std::collections::HashMap<Vec<&'a str>, usize>,
    side: &'a [Token],
    runs: &[std::ops::Range<usize>],
) -> Vec<usize> {
    runs.iter()
        .map(|run| {
            let key = side[run.clone()].iter().map(|t| t.text.as_str()).collect();
            let next = ids.len();
            *ids.entry(key).or_insert(next)
        })
        .collect()
}

/// Two-level diff for a changed middle beyond [`LCS_CELL_BUDGET`]: an LCS over whole lines (a
/// line's identity is its token texts, so layout inside a line is ignored) picks unchanged lines
/// as anchors, then a token LCS runs on each gap between anchors. The script is valid but, unlike
/// [`lcs_ops`], not guaranteed minimal. `None` when the line table, or the gaps' cells together,
/// are over budget, so the work stays within twice the budget however many gaps there are.
fn line_anchored_ops(old: &[Token], new: &[Token]) -> Option<Vec<Op>> {
    let (prefix, suffix) = common_affixes(old, new);
    let (old_end, new_end) = (old.len() - suffix, new.len() - suffix);
    let old_lines = line_runs(&old[prefix..old_end]);
    let new_lines = line_runs(&new[prefix..new_end]);
    let (n, m) = (old_lines.len(), new_lines.len());
    if n.saturating_mul(m) > LCS_CELL_BUDGET {
        return None;
    }
    // Intern each line's token texts, so the table compares integers.
    let mut ids = std::collections::HashMap::new();
    let old_ids = intern_lines(&mut ids, &old[prefix..old_end], &old_lines);
    let new_ids = intern_lines(&mut ids, &new[prefix..new_end], &new_lines);
    let width = m + 1;
    let mut table = vec![0u32; (n + 1) * width];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            table[i * width + j] = if old_ids[i] == new_ids[j] {
                table[(i + 1) * width + (j + 1)] + 1
            } else {
                table[(i + 1) * width + j].max(table[i * width + (j + 1)])
            };
        }
    }
    let mut ops = Vec::with_capacity(old.len() + new.len());
    ops.extend((0..prefix).map(|k| Op::Equal(k, k)));
    // Token cursors (absolute indices) up to which ops have been emitted, and gap cells spent.
    let (mut old_at, mut new_at) = (prefix, prefix);
    let mut spent = 0;
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if old_ids[i] == new_ids[j] {
            let (old_line, new_line) = (prefix + old_lines[i].start, prefix + new_lines[j].start);
            gap_ops(
                old,
                new,
                (old_at, new_at),
                (old_line, new_line),
                &mut spent,
                &mut ops,
            )?;
            let len = old_lines[i].len();
            ops.extend((0..len).map(|k| Op::Equal(old_line + k, new_line + k)));
            (old_at, new_at) = (old_line + len, new_line + len);
            i += 1;
            j += 1;
        } else if table[(i + 1) * width + j] >= table[i * width + (j + 1)] {
            i += 1;
        } else {
            j += 1;
        }
    }
    gap_ops(
        old,
        new,
        (old_at, new_at),
        (old_end, new_end),
        &mut spent,
        &mut ops,
    )?;
    ops.extend((0..suffix).map(|k| Op::Equal(old_end + k, new_end + k)));
    Some(ops)
}

/// Append the token LCS of `old[from.0..to.0]` against `new[from.1..to.1]` with absolute indices,
/// adding its cells to `spent`; `None` once the gaps together exceed [`LCS_CELL_BUDGET`].
fn gap_ops(
    old: &[Token],
    new: &[Token],
    from: (usize, usize),
    to: (usize, usize),
    spent: &mut usize,
    ops: &mut Vec<Op>,
) -> Option<()> {
    *spent = spent.saturating_add((to.0 - from.0).saturating_mul(to.1 - from.1));
    if *spent > LCS_CELL_BUDGET {
        return None;
    }
    ops.extend(
        lcs_ops(&old[from.0..to.0], &new[from.1..to.1])
            .into_iter()
            .map(|op| match op {
                Op::Equal(i, j) => Op::Equal(from.0 + i, from.1 + j),
                Op::Delete(i) => Op::Delete(from.0 + i),
                Op::Insert(j) => Op::Insert(from.1 + j),
            }),
    );
    Some(())
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
    // MIN_MOVE_RUN) are a moved block. Each insert run is matched at most once,
    // to the earliest unused one. Insert runs are indexed by text once, so the
    // search is linear in the runs rather than a scan per delete run.
    let mut unused: std::collections::HashMap<
        Vec<&str>,
        std::collections::VecDeque<(usize, usize)>,
    > = std::collections::HashMap::new();
    for ins in &insert_runs {
        let text = run_text(*ins);
        if text.len() >= MIN_MOVE_RUN {
            unused.entry(text).or_default().push_back(*ins);
        }
    }
    let mut moved_ops: std::collections::HashSet<usize> = std::collections::HashSet::new();
    // Only runs of at least MIN_MOVE_RUN are indexed, so a shorter delete run finds nothing.
    for d in &delete_runs {
        let dt = run_text(*d);
        if let Some(ins) = unused
            .get_mut(&dt)
            .and_then(std::collections::VecDeque::pop_front)
        {
            moved_ops.extend(d.0..d.1);
            moved_ops.extend(ins.0..ins.1);
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
        state: StructuralState::Degraded,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run `work` on its own thread and fail if it takes more than ten seconds. The slowest
    /// diff here takes a few seconds in a debug build, so an edit-script loop whose cursor stops
    /// advancing fails its test instead of hanging the suite (and a mutation run reports it as
    /// caught, not as a timeout).
    fn within_deadline<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> T {
        let (done, wait) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = done.send(work());
        });
        wait.recv_timeout(std::time::Duration::from_secs(10))
            .expect("a diff finished within ten seconds")
    }

    // The tests call these deadline-bound wrappers, which shadow the functions under test; the
    // exhaustive `lcs_ops` and `line_anchored_ops` tests run their loops under one deadline.

    fn structural_diff(language: Language, old: &str, new: &str) -> StructuralDiff {
        let (old, new) = (old.to_owned(), new.to_owned());
        within_deadline(move || super::structural_diff(language, &old, &new))
    }

    fn mark_moves(ops: &[Op], old: &[Token], new: &[Token]) -> Vec<TokenChange> {
        let ops: Vec<Op> = ops
            .iter()
            .map(|op| match *op {
                Op::Equal(i, j) => Op::Equal(i, j),
                Op::Delete(i) => Op::Delete(i),
                Op::Insert(j) => Op::Insert(j),
            })
            .collect();
        let (old, new) = (old.to_vec(), new.to_vec());
        within_deadline(move || super::mark_moves(&ops, &old, &new))
    }

    /// Anchor C-06: each state reports the flags the JSON outputs carry, exactly as the four
    /// separate fields did on the paths that built them.
    #[test]
    fn each_state_reports_the_flags_its_path_used_to_set() {
        use deep_diff_forge_core::FallbackReason;
        let flags = |state: StructuralState| {
            let diff = StructuralDiff {
                language: Language::Rust,
                changes: Vec::new(),
                unchanged: 0,
                added: 0,
                removed: 0,
                moved: 0,
                state,
            };
            (
                diff.reformat_only(),
                diff.degraded(),
                diff.minimal(),
                diff.unavailable().cloned(),
            )
        };
        let reason = FallbackReason::UnsupportedLanguage;
        assert_eq!(
            flags(StructuralState::Unavailable(reason.clone())),
            (false, true, false, Some(reason))
        );
        assert_eq!(flags(StructuralState::Degraded), (false, true, false, None));
        assert_eq!(
            flags(StructuralState::LineAnchored),
            (false, false, false, None)
        );
        assert_eq!(
            flags(StructuralState::Minimal {
                reformat_only: false
            }),
            (false, false, true, None)
        );
        assert_eq!(
            flags(StructuralState::Minimal {
                reformat_only: true
            }),
            (true, false, true, None)
        );
    }

    /// A comment whose only change is its line ending or trailing spaces is reformat-only; a
    /// changed word is not.
    #[test]
    fn comment_line_endings_and_trailing_spaces_are_formatting() {
        let base = "// hi\nfn a() {} /* x\n y */\n/// doc\nfn b() {}\n";
        for variant in [
            base.replace('\n', "\r\n"),
            base.replace("// hi\n", "// hi  \n"),
            base.replace("/* x\n", "/* x \t\n"),
        ] {
            assert!(
                structural_diff(Language::Rust, base, &variant).reformat_only(),
                "{variant:?}"
            );
        }
        let reworded = base.replace("// hi", "// bye");
        assert!(!structural_diff(Language::Rust, base, &reworded).reformat_only());
        assert_eq!(comment_token("// a \r\n"), "// a");
        assert_eq!(comment_token("/* a  \r\n b */"), "/* a\n b */");
    }

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
        within_deadline(|| {
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
        });
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
        assert!(!d.degraded());
        assert!(d.minimal());
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
        assert!(!d.degraded());
        assert!(d.minimal(), "within budget, so the exact token LCS");
        assert_eq!((d.added, d.removed), (2, 2));
        // Edits at both ends leave a middle of ~4,700 tokens (21.9 M cells): anchored on the
        // unchanged lines between them, it is still analysed exactly.
        let both_ends = source(&|i| match i {
            10 | 790 => format!("e{i}"),
            _ => format!("f{i}"),
        });
        let d = structural_diff(Language::Rust, &old, &both_ends);
        assert!(!d.degraded());
        assert!(!d.minimal(), "anchored, so not claimed minimal");
        assert_eq!((d.added, d.removed, d.moved), (2, 2, 0));
        // A rewritten block of 160 functions near the end (a 960-token gap, 0.92 M cells) plus an
        // edit near the start: the gap is measured on its own, so the file is still analysed.
        let block = source(&|i| match i {
            10 | 600..760 => format!("e{i}"),
            _ => format!("f{i}"),
        });
        let d = structural_diff(Language::Rust, &old, &block);
        assert!(!d.degraded());
        assert_eq!((d.added, d.removed), (161, 161));
        // A changed middle that is itself beyond the budget still degrades.
        let rewritten = source(&|i| format!("g{i}"));
        assert!(structural_diff(Language::Rust, &old, &rewritten).degraded());
    }

    // 2,100 lines a side (4.41 M line-table cells): even with only two edits, the line table
    // itself is over budget, so the file degrades rather than allocating it.
    #[test]
    fn a_line_table_beyond_the_budget_degrades() {
        let source = |edited: bool| -> String {
            let mut text = String::new();
            for i in 0..2100 {
                let name = if edited && (i == 5 || i == 2095) {
                    "e"
                } else {
                    "f"
                };
                text.push_str("fn ");
                text.push_str(name);
                text.push_str(&i.to_string());
                text.push_str("() {}\n");
            }
            text
        };
        let d = structural_diff(Language::Rust, &source(false), &source(true));
        assert!(d.degraded());
        assert!(d.unavailable().is_none());
    }

    // Each rewritten block is a gap well inside the budget; together they are not. The work bound
    // is on the gaps' total, so eight blocks degrade where four are analysed.
    #[test]
    fn the_gaps_together_stay_within_the_cell_budget() {
        let source = |blocks: usize| -> String {
            let mut text = String::new();
            for i in 0..1600 {
                // A 150-function block starts every 200 functions.
                let rewritten = i % 200 < 150 && i / 200 < blocks;
                text.push_str(if rewritten { "fn g" } else { "fn f" });
                text.push_str(&i.to_string());
                text.push_str("() {}\n");
            }
            text
        };
        let old = source(0);
        let four = structural_diff(Language::Rust, &old, &source(4));
        assert!(!four.degraded() && !four.minimal());
        assert_eq!((four.added, four.removed), (600, 600));
        assert!(structural_diff(Language::Rust, &old, &source(8)).degraded());
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

    // The anchored path gives up minimality, never validity: check the edit script exhaustively,
    // with each side grouped into lines differently.
    #[test]
    fn line_anchored_ops_is_always_a_valid_edit_script() {
        let sequences = all_sequences(&["a", "b", "c"], 5);
        let tokens = |seq: &[&str], per_line: usize| -> Vec<Token> {
            seq.iter()
                .enumerate()
                .map(|(k, t)| Token {
                    text: (*t).to_owned(),
                    line: u32::try_from(k / per_line).unwrap(),
                })
                .collect()
        };
        // One deadline per grouping keeps each well inside it (the three take a few seconds
        // together in a debug build).
        for (old_per_line, new_per_line) in [(1, 1), (2, 2), (1, 3)] {
            let sequences = sequences.clone();
            within_deadline(move || {
                for old in &sequences {
                    for new in &sequences {
                        let (o, n) = (tokens(old, old_per_line), tokens(new, new_per_line));
                        let ops = line_anchored_ops(&o, &n).unwrap();
                        let (mut next_old, mut next_new) = (0, 0);
                        for op in &ops {
                            match *op {
                                Op::Equal(i, j) => {
                                    assert_eq!((i, j), (next_old, next_new), "{old:?} -> {new:?}");
                                    assert_eq!(old[i], new[j], "{old:?} -> {new:?}");
                                    next_old += 1;
                                    next_new += 1;
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
                        // One token per line: the line LCS is the token LCS, so anchoring is optimal.
                        if (old_per_line, new_per_line) == (1, 1) {
                            let equal = ops.iter().filter(|op| matches!(op, Op::Equal(..))).count();
                            assert_eq!(equal, reference_lcs_len(old, new), "{old:?} -> {new:?}");
                        }
                        if old == new && old_per_line == new_per_line {
                            assert!(ops.iter().all(|op| matches!(op, Op::Equal(..))));
                        }
                    }
                }
            });
        }
    }

    #[test]
    fn affixes_and_line_runs_are_exact() {
        let tokens = |spec: &[(&str, u32)]| -> Vec<Token> {
            spec.iter()
                .map(|(text, line)| Token {
                    text: (*text).to_owned(),
                    line: *line,
                })
                .collect()
        };
        let old = tokens(&[("a", 1), ("b", 1), ("x", 2), ("c", 3), ("d", 3), ("e", 3)]);
        let new = tokens(&[("a", 1), ("b", 1), ("y", 2), ("z", 2), ("d", 3), ("e", 3)]);
        assert_eq!(common_affixes(&old, &new), (2, 2));
        assert_eq!(common_affixes(&old, &old), (6, 0));
        assert_eq!(common_affixes(&old, &[]), (0, 0));
        assert_eq!(line_runs(&old), [0..2, 2..3, 3..6]);
        assert_eq!(line_runs(&new), [0..2, 2..4, 4..6]);
        assert_eq!(line_runs(&[]), Vec::<std::ops::Range<usize>>::new());
    }

    #[test]
    fn tokenize_unsupported_is_empty() {
        assert_eq!(tokenize(Language::Unsupported, "fn main() {}").len(), 0);
    }

    #[test]
    fn identical_sources_are_all_unchanged() {
        let src = "fn main() { let x = 1; }";
        let d = structural_diff(Language::Rust, src, src);
        assert!(d.reformat_only());
        assert_eq!(d.added, 0);
        assert_eq!(d.removed, 0);
        assert!(d.unchanged > 0);
    }

    /// A rewritten comment is a change, not formatting: every kind of comment counts with its
    /// exact bytes. Before, a changed `// SAFETY:` justification was reported reformat-only.
    #[test]
    fn a_rewritten_comment_is_never_reformat_only() {
        for (old, new) in [
            (
                "// SAFETY: the pointer is valid\nfn a() {}\n",
                "// SAFETY: trust me\nfn a() {}\n",
            ),
            (
                "/// Returns the sum.\nfn a() {}\n",
                "/// Returns the product.\nfn a() {}\n",
            ),
            ("/* keep */ fn a() {}\n", "/* drop */ fn a() {}\n"),
            ("fn a() {} // x\n", "fn a() {} //  x\n"),
        ] {
            let d = structural_diff(Language::Rust, old, new);
            assert!(!d.reformat_only(), "{old:?} -> {new:?}");
            assert_eq!((d.added, d.removed), (1, 1), "{old:?} -> {new:?}");
        }
        // Moving an unchanged comment to its own line is still formatting only.
        let d = structural_diff(
            Language::Rust,
            "fn a() {} // note\n",
            "fn a() {}\n// note\n",
        );
        assert!(d.reformat_only());
    }

    #[test]
    fn reformatting_only_is_detected() {
        // Same tokens, wildly different whitespace/layout.
        let old = "fn main(){let x=1;}";
        let new = "fn   main ( ) {\n    let x = 1 ;\n}\n";
        let d = structural_diff(Language::Rust, old, new);
        assert!(
            d.reformat_only(),
            "pure reformat must report no structural change"
        );
        assert_eq!(d.added + d.removed + d.moved, 0);
    }

    /// Bug hunt 2026-11-01: a string literal of only whitespace had no token, so `""` to `" "`,
    /// or a tab to two spaces, was called reformat-only.
    #[test]
    fn whitespace_inside_a_string_literal_is_a_change() {
        for (old, new) in [
            (
                "fn s() -> &'static str { \"\" }",
                "fn s() -> &'static str { \" \" }",
            ),
            ("const S: &str = \"\t\";", "const S: &str = \"  \";"),
            ("const B: &[u8] = b\"\t\";", "const B: &[u8] = b\"  \";"),
            ("const S: &str = r\" \";", "const S: &str = r\"   \";"),
            ("const S: &str = r\"a\";", "const S: &str = r#\"a\"#;"),
            ("fn f() { g(a) }", "fn f() { g(r\"a\") }"),
        ] {
            let d = structural_diff(Language::Rust, old, new);
            assert!(!d.reformat_only(), "{old:?} -> {new:?}");
            assert_eq!((d.added, d.removed), (1, 1), "{old:?} -> {new:?}");
        }
        assert_eq!(
            tokenize(Language::Rust, "fn f() { x(\" a \", r#\"b\"#) }")
                .iter()
                .map(|t| t.text.as_str())
                .collect::<Vec<_>>(),
            [
                "fn", "f", "(", ")", "{", "x", "(", "\" a \"", ",", "r#\"b\"#", ")", "}"
            ]
        );
        // Reflowing the code around a literal is still formatting only.
        let d = structural_diff(
            Language::Rust,
            "fn s()->&'static str{\" \"}",
            "fn s() -> &'static str {\n    \" \"\n}\n",
        );
        assert!(d.reformat_only());
    }

    #[test]
    fn added_token_is_classified() {
        let old = "fn f() { a(); }";
        let new = "fn f() { a(); b(); }";
        let d = structural_diff(Language::Rust, old, new);
        assert!(!d.reformat_only());
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
        assert!(!d.reformat_only());
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
        assert!(!d.reformat_only());
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
        assert!(!d.reformat_only());
    }

    // Pins for the 2026-10-31 whole-crate mutation run: each budget is inclusive.

    #[test]
    fn tokenizing_admits_exactly_the_byte_budget() {
        use deep_diff_forge_core::FallbackReason;
        let at_budget = format!("//{}", "a".repeat(crate::analyze::DEFAULT_BYTE_BUDGET - 2));
        assert_eq!(
            tokenize_checked(Language::Rust, &at_budget).unwrap().len(),
            1
        );
        assert_eq!(
            tokenize_checked(Language::Rust, &format!("{at_budget}a")),
            Err(FallbackReason::ByteBudgetExceeded)
        );
    }

    /// A tree of exactly 200,000 nodes is tokenized and one statement more is refused. Without
    /// comments every node is visited once, so tree-sitter's `descendant_count` is the count.
    #[test]
    fn tokenizing_admits_exactly_the_node_budget() {
        use deep_diff_forge_core::FallbackReason;
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .unwrap();
        let mut nodes = |source: &str| {
            parser
                .parse(source, None)
                .unwrap()
                .root_node()
                .descendant_count()
        };
        // `a;` adds three nodes and `-a;` five, so some mix of them reaches any count.
        let source = |plain: usize, negated: usize| {
            format!("fn f() {{{}{}}}", "a;".repeat(plain), "-a;".repeat(negated))
        };
        let (base, plain, negated) = (nodes(&source(0, 0)), 3, 5);
        assert_eq!(nodes(&source(1, 0)) - base, plain);
        assert_eq!(nodes(&source(0, 1)) - base, negated);
        let left = 200_000 - base;
        let negated_count = (0..plain)
            .find(|k| (left - k * negated) % plain == 0)
            .unwrap();
        let exact = source((left - negated_count * negated) / plain, negated_count);
        assert_eq!(nodes(&exact), 200_000);
        assert!(tokenize_checked(Language::Rust, &exact).is_ok());
        let over = exact.replacen('{', "{a;", 1);
        assert_eq!(
            tokenize_checked(Language::Rust, &over),
            Err(FallbackReason::NodeBudgetExceeded)
        );
    }

    /// One line per function; `edited(i)` says whether the i-th is renamed.
    fn functions(count: usize, edited: impl Fn(usize) -> bool) -> String {
        let mut text = String::new();
        for i in 0..count {
            text.push_str(if edited(i) { "fn e" } else { "fn f" });
            text.push_str(&i.to_string());
            text.push_str("() {}\n");
        }
        text
    }

    /// Edits on the first and last of 2,000 lines leave a middle of 2,000 partial or whole lines
    /// a side: a line table of exactly the 4 M-cell budget, which is analysed.
    #[test]
    fn a_line_table_of_exactly_the_budget_is_analysed() {
        let old = functions(2000, |_| false);
        let new = functions(2000, |i| i == 0 || i == 1999);
        let d = structural_diff(Language::Rust, &old, &new);
        assert!(!d.degraded() && !d.minimal());
        assert_eq!((d.added, d.removed), (2, 2));
    }

    /// Lines inserted near the start and at the end (gaps of 0 cells, and no common suffix) put
    /// the diff on the anchored path; a block of 400 rewritten five-token lines between whole
    /// unchanged lines is then a 2,000 by 2,000 gap, exactly the budget, so the file is still
    /// analysed.
    #[test]
    fn gaps_totalling_exactly_the_budget_are_analysed() {
        use std::fmt::Write as _;
        let source = |rewritten: &str, inserted: bool| -> String {
            let mut text = functions(10, |_| false);
            if inserted {
                text.push_str("struct Inserted;\n");
            }
            text.push_str(&functions(300, |_| false).replace("fn f", "fn h"));
            for i in 0..400 {
                let _ = writeln!(text, "type {rewritten}{i} = u8;");
            }
            text.push_str(&functions(10, |_| false).replace("fn f", "fn t"));
            if inserted {
                text.push_str("struct End;\n");
            }
            text
        };
        let d = structural_diff(Language::Rust, &source("A", false), &source("B", true));
        assert!(!d.degraded() && !d.minimal());
        assert_eq!((d.added, d.removed), (3 + 400 + 3, 400));
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
            assert!(!diff.reformat_only());
            assert!(diff.degraded());
            assert!(diff.unavailable().is_some());
        }
    }
}
