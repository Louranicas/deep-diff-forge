# Vesper V3 round 5 review (2026-11-13)

Reviewed: committed snapshot `8128c77..a530cb3` on `agent/integrity-signals`. The unrelated uncommitted fuzz/docs work in the worktree was excluded.

## Verdict

**NO-GO.** The exact round-four probes now leave the intended evidence, but both fixes retain straightforward variants that reproduce the same silent losses. The whole-new-file proof is not sound in this branch because its parser still accepts multiple sections for one path; Git applies those sections together. The targeted macro fallback is suppressed by the outer macro itself when the hidden assertion starts on the same row.

The stated future v0.8.4 duplicate-path refusal would close VD3R5-H-1 if it is rebased, verified, and made an invariant of every entry point. It is not present in `a530cb3`, so this commit cannot rely on it for a current merge verdict.

## Round-four findings

- **VD3R4-H-1: not closed in this commit.** The original forged line-1 hunk of an existing file is fixed: `/tmp/vesper-v3r4-offset.patch` now reports `panic_path_added` on line 3 with `syntax_fragment` basis. A single `ReviewFile` marked Added with one all-added hunk is still not proof that those bytes are the final whole file when another section names the same path. VD3R5-H-1 restores production-code withdrawal under default `git apply`.
- **VD3R4-M-1: not closed.** The original forwarding-macro probe is improved: `/tmp/vesper-v3r4-macro.patch` now reports `syntax_unchecked` at line 5. Moving `assert!(` onto the outer macro's row makes condition (2) treat that row as accounted for by the unrelated `outer!` construct; neither `trivial_assert` nor `syntax_unchecked` remains. That is VD3R5-M-1.

## New findings

### VD3R5-H-1 — A later section for the same new path invalidates the whole-file proof

`creates_whole_file` reasons only about one parsed `ReviewFile`:

```rust
file.status == FileStatus::Added && file.patch_twin.hunks.len() == 1
```

It does not know that another section in the same patch names the path. The first section can therefore look like the complete new file and earn test-only withdrawal, while a later section changes the final file so the attribute text is not an attribute at all.

Reproduction: `/tmp/vesper-v3r5-duplicate-path.patch` first creates `src/duplicate.rs` as:

```rust
#[cfg(test)] // */
fn production() {
    load().unwrap();
}
```

A second section for the same path prepends `/*`. Default Git accepted both sections:

```text
git apply --check --verbose: success
git apply --verbose:
  Applied patch src/duplicate.rs cleanly.
  Applied patch src/duplicate.rs cleanly.
```

The resulting file is:

```rust
/*
#[cfg(test)] // */
fn production() {
    load().unwrap();
}
```

The `cfg` text is inside the block comment, so `production` is production code. DDF emitted two ranked entries:

```text
src/duplicate.rs  added     score 4  [new_file]
src/duplicate.rs  modified  score 2  []
```

There is no `panic_path_added` or uncertainty item. The first section withdrew the panic path as test-only; the later section contains no added panic spelling to restore it.

This is the same integrity failure as round four and works with ordinary Git defaults, so it is High. A dependency on an unmerged parser release is not a guard in this commit.

Fix: either refuse duplicate paths before any evidence/ranking API processes them, or make the evidence plan know the complete review and permit whole-file withdrawal only when the path occurs exactly once. The latter is useful defense in depth even after the parser refusal, because `file_evidence` can also be called on individually constructed `ReviewFile` values. Add this exact two-section patch as a regression at the parser boundary and at the rank/evidence boundary; the accepted outcomes are parse refusal or a standing `panic_path_added`, never silent withdrawal.

### VD3R5-M-1 — An outer macro on the same row masks the hidden multiline assertion

The new unread-row fallback refuses to mark a row when the parser reports any attribute or macro invocation starting there:

```rust
!new.scan.constructs.iter().any(|read| {
    read.row == tree_row && !matches!(read.kind, ConstructKind::MethodCall { .. })
})
```

That construct need not be the assertion `opens_across_lines` found. If the outer macro and inner assertion share the row, tree-sitter reports `outer!` at that row and treats its token tree as unreadable; the unrelated outer construct suppresses the fallback.

Reproduction: `/tmp/vesper-v3r5-same-row-macro.patch` adds:

```rust
macro_rules! outer { ($($tt:tt)*) => { $($tt)* }; }
outer! { assert!(
    true
); }
fn main() {}
```

This forwarding expansion contains the same `assert!(true)` as the round-four probe. Observed rank output was only:

```text
tests/same_row_macro.rs  score 3  [test_only, new_file]
```

There was no `trivial_assert` and no `syntax_unchecked`. This is a one-line formatting change to the fixed reproduction and silently loses a weight-3 signal, so I rate it Medium.

Fix: condition (2) should suppress fallback only when a relevant construct at the same source position accounts for the opener, not when any macro/attribute begins on the row. Compare the opener's byte column and kind/name, or explicitly distinguish the outer macro node whose token tree owns the unreadable region from an inner opener found lexically within it. Add same-row cases for `outer! { assert!(` and `outer! { #[cfg_attr(`. The already-declared bracket/brace delimiters and split method/stub calls should remain documented or gain their own targeted fallbacks.

## Additional verification

- Re-read the complete committed diff `8128c77..a530cb3`; only the committed graph logic, syntax documentation, tests, corpus, and documentation were included. I did not read the modified worktree copies as evidence for the commit.
- Prior regressions still pass on the supplied debug binary:
  - aliased `rstest` and renamed `tokio` report `panic_path_added`;
  - the harmless parser-budget prefix leaves `src/zzz.rs` with `panic_path_added` and `syntax_fragment` basis;
  - same-row and unreadable-old assertion cases report `trivial_assert` with `syntax_fragment` basis.
- Direct prebuilt tests passed: syntax 107/107, graph 132/132, cluster 43/43, daemon 225 passed with 1 ignored.
- `deep-diff-forge eval` passed at the committed figures: 42 cases; evidence precision 53/56, recall 53/81, findings 33/33, ranking 12/15, declared 34.
- The historical `d4dbc91~40..d4dbc91` patch produced one `syntax_unchecked`, at a multiline `assert_eq!` in `crates/deep-diff-forge-agent/src/findings.rs`; one item across that history does not make the signal unusably noisy.
- Release resource checks remained below the requested limits:

| Input | Elapsed | Peak RSS |
|---|---:|---:|
| 64,109,679-byte node-heavy patch | 1.26 s | 129,808 KiB |
| 13,988,975-byte many-hunks patch | 1.48 s | 179,336 KiB |
| 59,999,513-byte, ~1-million-line near-match patch | 2.50 s | 312,148 KiB |

- The two new committed tests are regression-sensitive against `8128c77`: the existing-file-header test previously withdrew the panic path, and the next-line forwarding-macro test previously emitted no item. They do not cover either reproduction above.

## Not examined

- I did not inspect or rely on the uncommitted fuzz target, seeds, README, changelog, or testing-strategy changes present in the worktree.
- I did not inspect the separate v0.8.4 work or verify that every parser/daemon/MCP entry point will refuse duplicate paths after the future rebase.
- I did not exhaustively test every Git application option, reverse application, path stripping, quoted/non-UTF-8 paths, CRLF/BOM combinations, binary/text mixtures, file mode, rename/copy, or submodule/symlink form. The default two-section application is sufficient for VD3R5-H-1.
- I did not fuzz every unread-row syntax error, delimiter, same-row construct combination, raw identifier, or macro nesting depth.
- I did not run Cargo, Clippy, Miri, sanitizers, or fuzzers. The hive slot was free, but the supplied binaries and current prebuilt test executables covered the focused committed snapshot without touching the uncommitted work.
- I did not modify repository files, refs, remotes, index, configuration, or the existing uncommitted changes apart from this new report. All reproductions were under `/tmp`.

STATUS: COMPLETE
