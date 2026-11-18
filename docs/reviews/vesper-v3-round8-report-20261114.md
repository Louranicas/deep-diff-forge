# Vesper V3 round 8 review (2026-11-14)

Reviewed: committed snapshot `f6f99d2..bebebef` on `agent/integrity-signals`. I treated patch and fixture contents as data and wrote only this report in the repository.

## Verdict

**NO-GO.** The fixes close the three High findings and most of the independent review cleanly: item-span withdrawal, compiler/grammar disagreement guards, the review-wide byte budget, merged-range lookup, macro-body accounting, old-side construct comparison, nested `cfg_attr`, path/function calls, and spaced `allow` all behaved as described. The corpus also exactly matches its declarations.

Two Medium issues remain. IR-M-1 is deliberately documented rather than fixed and is still reproducible: the same patch yields semantic evidence when idle and `syntax_unchecked` under CPU contention. In addition, valid raw-identifier calls (`x.r#unwrap()` and `Type::r#unwrap(x)`) escape both the panic-path signal and the new unchecked fallback, breaking claim 2.

## Independent-review findings

- **IR-H-1: closed.** Withdrawal now uses an item's text span and requires the entire added line's code extent to lie within one proved item. The original `ts/a6.patch` retains `panic_path_added` on the production call (score 7), and the same-row/macro-argument regressions passed.
- **IR-H-2: closed.** A root error or a top-level child other than the explicit item/attribute/comment set clears test proof. The original `ts/s2.patch` now retains `panic_path_added` (score 7). I also read the whole-file and duplicate-path gates; only `Reading::WholeFile` can withdraw.
- **IR-H-3: closed.** `FragmentPlan` spends at most 256 KiB across the review and `FragmentScan::readable` performs a binary search over merged unreadable ranges. On the supplied release binary, `perf/ternary32.patch` fell from the reported 27 seconds to 3.71 seconds (24,000 KiB maximum RSS), and `perf/slow.patch` from 10.8 seconds to 0.91 seconds (71,892 KiB). The dedicated budget and 200,000-range tests passed.
- **IR-M-1: not closed.** Documentation and hold advice are useful, but the result still depends on wall-clock scheduling. Three idle runs of `perf/nd2.patch` each produced score 6 with `trivial_assert` in 0.42-0.47 seconds. Eight concurrent copies pinned to CPU 5 all produced score 4 with `syntax_unchecked`. I did not identify a safe progress-count bound in tree-sitter's present callback surface; that does not make a clock a deterministic work bound.
- **IR-M-2: closed for the reported macro-body escapes.** The supplied split `assert`, `todo`, `ignore`, and `unwrap` forms now leave `syntax_unchecked` when neither parser nor token rule accounts for them. `c2/case-bang_newline_paren.patch` is now marked. VD3R8-M-1 is a distinct parser-readable raw-identifier escape.
- **IR-M-3: closed.** The old-side match includes construct kind and normalized construct text. `c2/cfgattr.patch` now reports `test_skipped` at the changed condition and scores 4.
- **IR-L-1: closed.** The top-level-items guard also rejects the frontmatter/expression shape as proof.
- **IR-L-2: closed as an undeclared-behaviour finding.** Nested serde `ignore` no longer means a skipped test, and function/path `unwrap` and `expect` are detected. The remaining parser/mock/Python false positives and cross-language respelling misses are explicitly declared in `evidence/independent-review-20261114`; they were catalogued, not eliminated.
- **IR-L-3: closed.** Spaced `allow` forms now produce `check_suppressed`.

## New finding

### VD3R8-M-1 — raw identifiers bypass panic-path evidence without an unchecked marker

Rust accepts a raw identifier for a non-keyword method or associated function and resolves it to the same identifier. I reproduced this with `/tmp/vesper-r8-raw-ident.rs`:

```rust
pub struct X;
impl X { pub fn unwrap(self) {} }
pub fn f(x: X) { x.r#unwrap(); }
pub fn g(x: X) { X::r#unwrap(x); }
```

`rustc --crate-type lib` in `fedora-toolbox-44` accepted the file. The corresponding new-file patch (`/tmp/vesper-r8-raw-ident.patch`) produced only `public_api_surface` and `new_file`; it had no evidence member, no `panic_path_added`, and no `syntax_unchecked`.

The parser stores the field text verbatim as `r#unwrap` in `fragment.rs:401`, while `signal_of` accepts only exact `unwrap` or `expect` at `evidence.rs:1667-1669`. The lexical fallback searches the `unwrap` substring, but the text immediately before it ends in `r#`, not `.` or `::`, so `calls_method` rejects it at `evidence.rs:1106-1110`. Because tree-sitter read the row successfully, `unread_words` does not mark it unchecked. This is a selectable silence for precisely the construct family claim 2 says is always accounted for.

Impact is Medium: a valid, semantics-preserving spelling suppresses the integrity signal with no uncertainty marker. Strip a leading `r#` from parser method/macro names before classification and teach the lexical matcher to accept `.<raw-name>` and `::<raw-name>` as the same identifier. Add method and UFCS/path raw-identifier regressions, including `expect`.

I found no new High or Low finding.

## Claims

1. **Withdrawn only under strict proof: held.** `Reading::WholeFile` requires an added file, exactly one hunk, and one occurrence of the resolved path in the whole review. Test proof is cleared on parse error or a non-item top level. `opens_test_code` is limited to top-level function/module/impl items whose first attribute is exactly `#[cfg(test)]`, and `lines_inside` requires the whole lexical code extent to fit one item span. The original same-line and grammar-divergence attacks now retain their panic evidence.
2. **Every parser construct word is accounted for: broken.** VD3R8-M-1 is valid Rust, parser-readable, and yields neither semantic evidence nor `syntax_unchecked`.
3. **At most 256 KiB parsed and identical evidence across views/workers: broken as one stated claim.** The byte-budget portion held by inspection, performance probes, and tests; idle `--rank`, `--cluster --parallel 1`, and `--cluster --parallel 4` agreed on `nd2.patch`. The unconditional evidence-equality portion is false while IR-M-1 stands: idle and contended executions of the same input return different evidence. I did not live-test the daemon, so I make no stronger daemon-parity claim.
4. **Corpus declarations equal current binary behaviour: held.** `deep-diff-forge eval --corpus fixtures/eval --json` returned 43 cases, no failures, precision 63/69, recall 63/104, findings 33/33, and ranking pairs 12/15. The independent-review case's thirteen declared misses and three declared false positives were present exactly as declared.

## Noise and history checks

- Applying the current binary to each of the 60 first-parent commits ending at `f6f99d2` produced `syntax_unchecked` in 3 commits, one file each: `fuzz/fuzz_targets/fragment_scan.rs`, `crates/deep-diff-forge-graph/src/risk.rs`, and `crates/deep-diff-forge-agent/src/findings.rs`. I found no broad new wave of markers in this repository's history.
- The broadened `::unwrap`/`::expect` matcher necessarily also accepts user-defined functions with those names. That is the same name-based precision limitation already declared for methods. In the same 60-commit range, added `::unwrap`/`::expect` spellings were confined to this feature's implementation, tests, fixtures, and documentation; I found no ordinary-code false positive from the change.
- I tried the requested same-row item boundary shapes, nested item bodies, raw strings, character literals, nested comments, shebang/frontmatter/top-level-statement forms, and duplicate-path planning. I found no second withdrawal escape.

## Tests and checks

The hive slot reported `free` immediately before Cargo. Cargo ran only inside `fedora-toolbox-44` with the supplied target directory.

- `deep-diff-forge-cluster`: 44 unit and 10 integration tests passed.
- `deep-diff-forge-graph`: 139 unit and 6 integration tests passed.
- `deep-diff-forge-syntax`: 112 unit and 10 integration tests passed.
- All doc tests for those crates passed.
- `git diff --check f6f99d2..bebebef` passed.
- Supplied release-binary checks covered the original H1/H2/M2/M3 reproductions, both performance patches, idle/contended nondeterminism, rank/cluster parity while idle, the eval corpus, and the raw-identifier reproduction.

## Not examined / limitations

- I did not run the full workspace suite, Clippy, Miri, the fuzz campaign, or the complete local CI gate.
- I did not differential-fuzz rustc's lexer against tree-sitter or exhaust every future/nightly Rust grammar extension.
- I did not start a daemon or exercise RPC/MCP/TUI surfaces live; cluster parity was tested through the CLI and shared planning code/tests.
- I did not independently regenerate every corpus label or audit unrelated rules and languages beyond the independent-review cases.
- I did not test Windows/macOS, case-folding filesystems, or memory at the global 64 MiB input limit.

STATUS: COMPLETE
