# Vesper V3 round 19 docs-only re-read — `7639dd4`

## Verdict

**GO** for `agent/integrity-signals` at `7639dd4` (`d4dbc91..7639dd4`). The closing diff narrows the disputed claims to the frozen round-19 binary's actual behavior, records all three review cases in the corpus, and introduces no new open high or medium. This satisfies convergence rule 5.

## Finding dispositions

- **VD3R19-M-1: acceptably narrowed and declared.** README, the head and relevant item docs in `rust_tokens.rs`, `evidence.rs`, the testing standard and the CHANGELOG now qualify the whole-attribute rule to the first 16 simultaneously followed `#[cfg_attr(` lists. They state that a seventeenth opener is not followed and is conservatively taken as `test_skipped` without `ignore` or `]`, and classify the macro-token-tree reproduction as a writer's false item. `src/deep.rs` records it as `known_false_positive` at the same line the frozen binary reports.
- **VD3R19-L-1: acceptably narrowed and declared.** The keyword claim now says the implementation is lexical: it reads the one word immediately before `<`, so every immediately preceding `mut` opens the bracketed path; `&mut` is the measured positive. The false statement about `mut` after `&raw` is gone. Both `&raw mut` and a discarding macro's `mut <Command>::new` are declared writer cases, and `src/mutmacro.rs` records the compiling macro case as `known_false_positive` at the line the frozen binary reports.
- **IR12-L-1: acceptably declared.** The documents no longer confine the rustfmt miss to `-U0`: they state that it occurs whenever a hunk ends between `ignore`/`ignore = "…"` and `)]`, at any context width, including plain `git diff`. `tests/four.rs` records the plain-diff example as an ordinary `known_miss`.
- **IR12-L-2 and IR12-L-3:** closed by the same two declarations as VD3R19-M-1 and VD3R19-L-1.

The earlier round-19 dispositions remain sound: VD3R18-M-1's focused incomplete forms are fixed, with the bounded seventeenth-list exception now explicit; VD3R18-L-1's `become` and `do yeet` positives remain closed; IR11-L-1 remains acceptably declared; IR11-L-2's measured `&mut` positive remains closed with the broader lexical behavior now stated; IR11-L-3 remains closed.

## Checks

- Read the complete request, convergence rule, prior Vesper report and every requested `f819a56..7639dd4` production-comment, document, test-floor, manifest and corpus change. I did not read the other reviewer's report.
- Verified `HEAD=7639dd417f2faee50bf0cc503231041b0d98bfb9`, and the frozen release binary SHA-256 `54b93d22fe88a43e519bb2e33276fcf41b60ed8e6e164af1283492b520dcf877`.
- Confirmed the production Rust delta changes doc comments only. Executable test changes update corpus floors and expected summary counts; no production behavior changed.
- Ran the frozen binary's `eval --corpus fixtures/eval` from a `git archive 7639dd4`: exit 0, 55 cases, evidence precision 177/203, evidence recall 177/260, findings 33/33, ranking 12/15, declared 112.
- Ran the new corpus patch through published v0.8.3, round 14, round 18 and round 19 binaries. `tests/four.rs` is a skip only in rounds 14/18; `src/deep.rs` is a skip in rounds 14/18/19 but not v0.8.3; `src/mutmacro.rs` is a spawn only in round 19. These are exactly the three declarations.
- The manifest hash and labels passed the frozen evaluator. `git diff --check` reports only the deliberate trailing space inside the embedded adversarial patch line that ends on the seventeenth opener, not whitespace in production source or prose.

## New findings

None.

## Not independently checked

- Per request, I ran no Cargo command, build, workspace tests, clippy, gate, fuzz target or remote operation, and accessed nothing at GitHub.
- I did not regenerate the 15,864-patch comparison, the earlier 6,110-patch comparison, keyword compilation matrix, cost suite or memory measurements.
- The worktree already contained untracked `docs/reviews/claude-independent-review-12-reread-20261117.md`; I did not read, modify, stage or remove it.

STATUS: COMPLETE
