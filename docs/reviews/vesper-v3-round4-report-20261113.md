# Vesper V3 round 4 review (2026-11-13)

## Verdict

**NO-GO.** `8128c77` fixes the exact round-three attribute aliases, parser-budget prefix, and old-side pairing reproductions, but the replacement proof for test-only code still trusts a hunk's claimed line number. Git itself can apply such a hunk elsewhere with supported permissive options. That lets a production panic path be withdrawn. I also reproduced the declared macro-body gap as a silent miss in all three review views.

## Round-three findings

- **VD3R3-H-1: not closed.** Removing the test-attribute allowlist is correct: the original aliased `rstest` and renamed `tokio` reproductions now both report `panic_path_added` with `syntax_fragment` basis. Exact `#[cfg(test)]` plus first-attribute/top-level/error-free restrictions are materially safer, but `from_first_line` is not proof that the hunk is actually applied at the first line. VD3R4-H-1 bypasses the replacement rule.
- **VD3R3-H-2: closed.** The original 2,100,799-byte harmless-prefix reproduction now reports `src/zzz.rs` at score 7 with `panic_path_added` and `syntax_fragment` basis. Harmless hunks no longer consume parser budget. Parser-word hunks that do consume it leave `syntax_unchecked`, and the spaced lexical rules retain the same-line spellings. Cluster content was identical at parallelism 1, 2, 4, and 8 after removing the receipt's worker fields (normalized SHA-256 `75efcf0c116d3fe28aac807b41328f958f37d45fa700eed9f331272c1ab516f3`). VD3R4-M-1 is a distinct partial-parse gap, not a recurrence of the first-fit-prefix defect.
- **VD3R3-M-1: closed.** Both original reproductions now report `trivial_assert` with `syntax_fragment` basis: the second assertion on a shared row is no longer paired with the first, and an unreadable old side no longer suppresses the new item. The row/column/signal comparison is conservative when the old side cannot be scanned.

## New findings

### VD3R4-H-1 — A forged line-1 hunk can be applied later and suppress a production panic path

`fragment_evidence` treats this as proof that the fragment begins the file:

```rust
let from_first_line =
    new.offset == 0 && hunk.new_start == Some(1) && hunk.old_start.is_none_or(|old| old <= 1);
```

The adjacent comment says Git applies such a hunk only at the beginning. That is true for default `git apply` in my reproduction, but false for supported application modes explicitly named in the request.

Reproduction: `/tmp/vesper-v3r4-offset.patch` claims `@@ -1,4 +1,4 @@` and presents this fragment:

```diff
 #[cfg(test)] // */
 fn production() {
-    safe();
+    load().unwrap();
 }
```

The actual file has `/*` immediately above that context. Rust therefore sees the `#[cfg(test)] // ` text inside the block comment; `*/` closes the comment, and `production` is production code. DDF parses the isolated hunk as a top-level exact `#[cfg(test)]` function and returns score 2 with no signals or evidence.

Application results against the actual file:

```text
git apply --check ...       -> rejected at production.rs:1
git apply --unidiff-zero    -> Hunk #1 succeeded at 2 (offset 1 line)
git apply -C0               -> Hunk #1 succeeded at 4 (offset 3 lines) in the expanded probe
```

After either permissive application, the file contains the production `load().unwrap()` that DDF suppressed. `--recount` alone rejected the probe; that does not repair the accepted modes.

The patch header is attacker-controlled metadata, not proof of application position. Without the base file or a guaranteed no-offset application policy, a modified-file hunk cannot prove it begins the file. Restrict withdrawal to complete new-file content, or make the caller provide verified application/base context and reject any offset/fuzz/recount result before allowing withdrawal. Add a regression that runs the accepted `-C0`/`--unidiff-zero` application and analyzes the same patch.

### VD3R4-M-1 — A parsed outer macro silently hides a multiline parser-word construct

`syntax_unchecked` is added only when `fragment_evidence` returns `None`. A successful tree-sitter scan returns `Some(Parsed)` even when the relevant parser word lies inside an unreadable outer macro token tree, so the fallback says the hunk was checked while neither rung actually reads the construct.

Reproduction: `/tmp/vesper-v3r4-macro.patch` adds a valid forwarding macro and invokes it with a multiline trivial assertion:

```rust
macro_rules! outer {
    ($($tt:tt)*) => { $($tt)* };
}
outer! {
    assert!(
        true
    );
}
```

The forwarding expansion contains `assert!(true)`, but the syntax scan marks the outer token tree unreadable and the line-based lexical rule cannot assemble the three lines. Observed rank output for `tests/macro.rs` was only score 3 with `test_only` and `new_file`: no `trivial_assert` and no `syntax_unchecked`. Outline and hunk views likewise had neither label.

This is an easy, silent loss of a weight-3 integrity signal, though it requires a nested macro and multiline spelling, so I rate it Medium rather than High. A targeted fallback need not label every macro body: when a parser word occurs on an unreadable row and no syntax or lexical item accounts for it, emit `syntax_unchecked` on the first relevant added line. Add the forwarding-macro case as a declared miss or regression.

## Additional verification

- Re-read the complete `2afeb06..8128c77` diff, concentrating on `fragment.rs`, `evidence.rs`, risk/vocabulary propagation, notes, API documentation, and the new tests.
- Direct prebuilt test executables passed: syntax 107/107, graph 130/130, cluster 43/43, and daemon 225 passed with 1 ignored.
- `deep-diff-forge eval` passed at the committed figures: 41 cases; evidence precision 50/53, recall 50/77, findings 33/33, ranking 12/15, declared 33.
- Release resource checks remained below the requested limits:

| Input | Elapsed | Peak RSS |
|---|---:|---:|
| 64,109,679-byte node-heavy patch | 1.26 s | 129,700 KiB |
| 13,988,975-byte many-hunks patch | 1.45 s | 179,352 KiB |
| 59,999,513-byte, ~1-million-line near-match patch | 2.48 s | 312,016 KiB |

- The new tests are regression-sensitive by inspection: the test-attribute and exact-`cfg` expectations contradict `2afeb06`; column and `SyntaxUnchecked` assertions require fields/variants absent there; the same-row and unreadable-old cases reproduce old failures; and the 150,000-item test exercises the old quadratic path. I did not rebuild and execute the suite at `2afeb06` because this review was read-only.
- The worktree remained clean apart from this report.

## Not examined

- I did not exhaustively compare every patch implementation or fuzz policy. GNU `patch(1)` was unavailable in the environment; the finding is already reproduced with two `git apply` modes.
- I did not fuzz tree-sitter/rustc disagreements across all raw strings, nested comments, BOMs, shebangs, new Rust syntax, generic/`where`-clause `impl`s, or same-line item layouts.
- I did not exhaustively fuzz old/new wrapper selection, tabs, Unicode columns, or construct correspondence.
- I did not test every daemon/RPC/MCP pagination and filtering combination, every 2 MiB allocation distribution, or every file ordering.
- I did not run Cargo, Clippy, Miri, sanitizers, or fuzzers; the supplied binaries and current prebuilt test executables were sufficient for the focused re-check.

STATUS: COMPLETE
