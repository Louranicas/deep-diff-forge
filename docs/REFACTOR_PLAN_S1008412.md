# Refactor & Feature Plan (S1008412)

Addresses Zen's review feedback + the two ❌ rows in the README feature comparison,
all in Rust, grounded in exemplar codebases (`difftastic`, `tree-sitter-rust`
`highlights.scm`) on `/mnt/storage-10tb/repos`.

## Priorities (execute in order, gate-green each)

### P1 — Patch parser: reject malformed/truncated hunks (Zen #1, core invariant)
**Problem.** `parser.rs::close_hunk` pushes a hunk regardless of remaining
`rem_old`/`rem_new`, so a hunk header (`@@ -a,b +c,d @@`) that declares more lines
than it provides is **silently accepted** (truncation). Likewise a `+` after the
new-count is exhausted (while old-count remains) over-fills a hunk. This weakens the
"patch truth" invariant: an internally-incoherent patch must not parse as truth.

**Fix.** Make the hunk's declared counts *exact*:
1. `consume_body`: reject a `+` when `rem_new == 0`, a `-` when `rem_old == 0`, a
   context line when either is `0` → new `PatchParseError::HunkLineCountMismatch`.
2. Closing a hunk (new header, new file, or EOF) with `rem_old != 0 || rem_new != 0`
   → `PatchParseError::TruncatedHunk { line_number, remaining_old, remaining_new }`.
   This requires `close_hunk`/`flush`/`finish` to return `Result` and propagate.
3. Tests: truncated hunk → error; over-long hunk (extra +/-) → error; exact hunk →
   ok; the `\ No newline` marker after exhaustion still ok; migrate existing tests
   that relied on lenient truncation (each must now be well-formed).

### P2 — Daemon socket path: type-driven, not ad-hoc (Zen #2)
**Problem.** Socket resolution is scattered imperative `Option`/`PathBuf` handling
(`security.rs` free functions + the CLI's nested `if let`); Zen dislikes the
explicit path juggling. Invalid states (unvalidated path, group-readable dir) are
representable.
**Fix.** Introduce a `SocketLocation` newtype (parse-don't-validate):
- `SocketLocation::resolve(explicit: Option<&Path>) -> Result<SocketLocation, SocketError>`
  — the ONE entry point: explicit `--socket` override, else `$XDG_RUNTIME_DIR`, else
  a typed `SocketError::NoRuntimeDir` (no `/tmp` fallback).
- It owns `path()` + `bind()` (ensure owner-private dir, symlink-reject, 0o600
  socket) + `connect()`. The CLI calls `SocketLocation::resolve(...)?` once and uses
  the value; the scattered `Option` logic disappears. Invalid location = unconstructable.
- Keep the existing pure resolvers as private internals; the public surface is the type.

### P3 — Syntax highlighting (flip ❌→✅ honestly)
**Approach.** Reuse the in-tree tree-sitter; load `tree_sitter_rust::HIGHLIGHTS_QUERY`
into a `tree_sitter::Query`, run a `QueryCursor` over the parsed tree, map capture
names (`keyword`, `function`, `string`, `comment`, `type`, …) to a small ANSI palette.
- New `syntax::highlight` module: `highlight_to_spans(source, language) -> Vec<HighlightSpan>`
  (byte-range + highlight class) — pure, testable without ANSI.
- A renderer-neutral `HighlightClass` enum + an `ansi` helper that wraps a span in
  SGR codes. **Security:** highlighting emits only OUR controlled SGR; attacker source
  text inside spans is still routed through `core::display_safe` first, so token
  content cannot inject escapes. Gate highlighting behind a `--color`/TTY check.
- Wire into `semantic --color` and the inline projection / TUI (opt-in). Zero new deps.

### P4 — Structural / AST-level diffing (flip ❌→✅ honestly)
**Approach (ref: difftastic `src/diff/lcs_diff.rs`).** A token-level structural diff,
not a full tree-edit-distance:
- New crate `deep-diff-forge-structural` (or `syntax::structural`): tokenize old+new via
  tree-sitter leaf nodes (named tokens, whitespace-insensitive ⇒ **reformat-aware**),
  run an LCS/Myers diff over the token sequences → `StructuralChange` (matched / added /
  removed) with byte ranges.
- **Moved-block detection:** a removed run that equals an added run elsewhere is marked
  `Moved` rather than remove+add.
- CLI `structural <old> <new> [--json]` + schema `deep-diff-forge.structural.v0`.
- Honest scope note: token/leaf-level structural diff (reformat-aware + moved-block),
  not difftastic's optimal graph diff — the comparison row + README footnote say so.

## Verification
Each P gate-green (check→clippy→pedantic→test→docs). Final: an independent `zen`
review + a security seal (highlighting must not break the terminal-injection
guarantee; structural diff must never mutate patch truth). Update the README feature
comparison to reflect only what genuinely ships.

## Status — DONE (gate-green, 754 tests)

- **P1 ✅** Parser enforces exact hunk counts: `TruncatedHunk` +
  `HunkLineCountMismatch`. One lenient test was a malformed patch — corrected.
- **P2 ✅** `SocketLocation` type; CLI resolves once; no `/tmp` fallback; symlink
  + ownership gates retained.
- **P3 ✅** `highlight` command + `syntax::highlight` (grammar `highlights.scm`,
  zero new deps). Injection-safe (verified: hostile file → 0 raw ESC).
- **P4 ✅** `structural` command + `syntax::structural` (token LCS, reformat-aware,
  best-effort moves, bounded). Honest scope: not difftastic's tree-edit-distance.
- README comparison rows flipped ❌→✅ for highlighting + structural (footnoted).
