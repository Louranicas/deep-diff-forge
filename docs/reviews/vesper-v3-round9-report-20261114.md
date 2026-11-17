# Vesper V3 round 9 review (2026-11-14)

Reviewed: committed snapshot `bebebef..1228c55` on `agent/integrity-signals`, using the frozen round-9 binaries named in the request. I wrote only this report in the repository.

## Verdict

**NO-GO.** IR-M-1 is closed, and the exact same-line method/UFCS raw-identifier cases of VD3R8-M-1 are closed. The count-bounded parser was deterministic under the original contention reproduction, the hostile-shape timings stayed linear in the tested range, and the rewritten item walk preserved the attribute boundaries I attacked.

I found one new Medium gap in the broader raw-identifier claim: valid multiline path calls and function values such as `X::` / `r#unwrap(x)` and `Result::` / `r#unwrap` are parser-readable but produce neither `panic_path_added` nor `syntax_unchecked`. A removed `#[r#test]` also produces no `test_removed`. The implementation normalizes raw prefixes within each line, but the lexical rules do not carry a path across lines, and the fragment parser classifies only field-expression method calls.

## Prior findings

### IR-M-1 — closed

`scan_fragment` calls `Parser::parse` directly and no longer calls `parse_bounded`, installs a progress callback, or consults a clock. Its byte and post-parse node limits remain 64 KiB and 50,000 nodes per fragment; `FragmentPlan` retains the 256 KiB review budget. Structural analysis and highlighting still use the timed parser, but neither supplies integrity evidence.

I reran the original `perf/nd2.patch` reproduction with the frozen release binary:

- three runs pinned to CPU 5 while otherwise idle;
- eight copies started together on CPU 5.

All eleven output files were byte-identical (SHA-256 `b42e86583cc899994dc71258350e33a162501a0ab7a1656e72f9ced90918b108`) and all scored 6 with `trivial_assert`, never `syntax_unchecked`. `--rank`, `--cluster --parallel 1`, `--cluster --parallel 4`, and `--outline` also agreed on path, score and signals.

The daemon's request deadline is checked between stages; it does not cut `scan_fragment`. A request that exhausts the outer deadline returns `RESOURCE_EXHAUSTED` after the bounded stage rather than returning different evidence.

### VD3R8-M-1 — closed as originally reported

The original `/tmp/vesper-r8-raw-ident.patch` now reports `panic_path_added` for both `x.r#unwrap()` and `X::r#unwrap(x)`, at lines 3 and 4 respectively. The frozen binary scored the file 12 with the expected signal. Same-line raw `expect`, macro names, `ignore`, `cfg_attr` and `allow` are covered by the new parser/token normalization and passed their tests.

VD3R9-M-1 below is a distinct multiline and exact-token gap exposed by the expanded claim that every Rust rule reads raw identifiers as their identifiers.

## New finding

### VD3R9-M-1 — multiline raw path calls and removed raw test attributes are silent

I created `/tmp/vesper-v3r9-raw-multiline.patch` and compiled its added Rust source successfully with `rustc`. It contains:

```rust
pub fn method(x: X) {
    x.
        r#unwrap();
}
pub fn path(x: X) {
    X::
        r#unwrap(x);
}
pub fn passed(it: impl Iterator<Item = X>) {
    let _ = it.map(X::
        r#unwrap);
}
pub fn standard(it: impl Iterator<Item = Result<u8, ()>>) {
    let _ = it.map(Result::
        r#unwrap);
}
```

The frozen release binary reported only the method call at line 5. The path call at line 9, user-defined function value at line 13, and standard `Result::r#unwrap` function value at line 17 produced no evidence and no `syntax_unchecked`.

The cause has two parts:

- `without_raw_prefixes` preserves columns by replacing `r#` with two spaces, but it operates independently on each line. On the line containing `unwrap`, `calls_method` cannot see the `.` or `::` from the preceding line (`evidence.rs:1120-1142`).
- The syntax scanner only constructs `MethodCall` when a call expression's function is a `field_expression` (`fragment.rs:433-439`). Scoped/path calls and paths passed as values are not constructs, even though tree-sitter read them successfully. Because the row is readable, `unread_words` does not add `syntax_unchecked`.

The same patch removes a valid `#[r#test]` attribute from a test file. `rustc --test` recognizes `#[r#test]`, but DDF reported only `test_only`; it did not report `test_removed`. The raw-prefix blanking turns it into `#[  test]`, while the old-side rule still requires one of the exact tokens `#[test]`, `#[tokio::test]`, or `#[rstest]` (`evidence.rs:519-525`).

One conservative mismatch confirms the normalization is not shared everywhere: `#[r#cfg(test)]` is accepted by rustc, but `opens_test_code` compares the unnormalized attribute path to exact `cfg` (`fragment.rs:546-550`). A whole new file under that attribute kept `panic_path_added` rather than withdrawing it. That false positive is safe by itself, but it contradicts the every-rule claim and shares the same cause.

Impact is Medium. Splitting a valid path at a newline is semantics-preserving and suppresses the panic-path integrity signal with no uncertainty marker; removing a raw-spelled test attribute is likewise silent. Recommended fix: classify scoped/qualified call targets and path values in the syntax scan, or give the lexical matcher bounded cross-line path state. Make exact attribute rules raw-aware without relying on inserted spaces, and normalize the path returned by `attribute_parts` before `cfg` proof/hint comparisons. Add regressions for multiline raw and ordinary `::unwrap`/`::expect`, passed standard functions, removed `#[r#test]`/qualified raw test attributes, and `#[r#cfg(test)]`.

I found no new High or Low finding.

## Claims

1. **One input, one answer: held.** The evidence parser has no clock path. Eleven idle/contended runs were byte-identical; rank and cluster worker counts agreed. Daemon deadlines fail the request after a stage rather than changing the parser's answer.
2. **Count-bounded work and measured parser cost: held for the attacked shapes.** I reran all 56 supplied shapes at 8, 16, 32 and 60 KiB. The slowest 60 KiB cases were 0.79 seconds user (`a?b:`), 0.78 seconds (`ternary lines` and `a?b:` inside a function), 0.69 seconds (`dyn A+`), and 0.67 seconds (`}{)(][`); growth tracked input length. `perf/ternary32.patch` took 3.57 seconds real / 3.54 user with 23,616 KiB maximum RSS, and `perf/slow.patch` took 0.80 real / 0.76 user with 72,128 KiB. I found no under-4-MiB review over five seconds.
3. **Every Rust rule reads a raw identifier as its identifier: broken.** VD3R9-M-1 covers silent multiline path calls and removed raw test attributes; raw `cfg` proof is also not normalized.
4. **Round-8 claims 1, 2 and 4 stand: broken as a combined claim.** Withdrawal remains restricted to strict proof, and the 44-case corpus exactly matches the binary. The accounting subclaim is broken by the parser-readable multiline `unwrap` rows that leave neither evidence nor `syntax_unchecked`.

## Walk and corpus checks

- The new `Level` state preserves the old attribute-run boundaries in the tested cases. Comments between attributes did not break the run; only an exact first `#[cfg(test)]` proved test-only code; a leading `#[allow]` prevented proof; an intervening macro broke the run; and a nested test item after a doc comment received only `test_likely`.
- The nested-item scan now carries parent kind and preceding attributes without repeated parent/previous-sibling walks. The 3,000-level regression passed.
- `deep-diff-forge eval --corpus fixtures/eval --json` returned 44 cases and no failures: precision 69/75, recall 69/110, findings 33/33, and ranking pairs 12/15.

## Tests and checks

The hive slot reported `free` immediately before Cargo. Cargo ran in `fedora-toolbox-44` with the review-owned target directory `/tmp/vesper-v3r9-target`.

- `deep-diff-forge-cluster`: 44 unit and 10 integration tests passed.
- `deep-diff-forge-graph`: 140 unit and 6 integration tests passed.
- `deep-diff-forge-syntax`: 115 unit and 10 integration tests passed.
- All doc tests for those crates passed.
- `git diff --check bebebef..1228c55` passed.
- Frozen-binary checks covered the original nondeterminism and raw-identifier reproductions, rank/cluster/outline parity, all supplied timing shapes, both large performance patches, the eval corpus, the attribute-walk cases, and VD3R9-M-1.

## Not examined / limitations

- I did not run the full workspace suite, Clippy, Miri, the fuzz campaign, or the complete local CI gate.
- I did not start a daemon or exercise RPC/MCP/TUI transports live; I inspected their shared evidence/planning paths and daemon deadline behavior.
- I did not prove a formal worst-case complexity bound for tree-sitter or exhaust all possible 64 KiB token sequences; claim 2 remains an empirical bound as stated.
- I did not test Windows/macOS or every Rust grammar/nightly extension.
- I did not review changes outside `bebebef..1228c55` or modify repository refs, remotes, index, configuration, or existing files.

STATUS: COMPLETE
