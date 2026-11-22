# Twelfth independent review, re-read of the round 19 closing diff (`f819a56..7639dd4`)

Reviewer: the same Claude agent that wrote `claude-independent-review-12-20261117.md`. Rule 5 of
the convergence rule: this is the re-read of a docs-only diff, with no change to code. Binary: the
frozen v3r19 release build (`54b93d22…`). The coordinator states that the `7639dd4` build is
byte-identical to it. I did not rebuild it (no `cargo`). Notes and patches are in
`/var/home/louranicas/.cache/ddf-t/indep-review-12/` (`notes/NOTES.md`, `notes/f/`).

## Verdict: GO for `agent/integrity-signals` at `7639dd4` (`d4dbc91..7639dd4`)

There is no open high or medium finding against any claim. Every finding of the two round 19
reviews is closed by narrowing, and each narrowed sentence is true of the frozen binary. One new
low remains in an example clause (IR12R-L-1 below). The general sentence above that clause is
right, so it does not block GO; it can be corrected in passing at integration.

## The diff checked

- **Code.** `git diff -U0 f819a56 7639dd4 -- 'crates/*/src/*.rs'` changes only comment and
  doc-comment lines. I found no line that is not a comment.
- **Corpus.** I ran `deep-diff-forge eval --corpus fixtures/eval` from a `git archive 7639dd4`.
  The result is exactly as stated: 55 cases, precision 177/203, recall 177/260, findings 33/33,
  ranking 12/15, 112 declared. On the frozen binary the new case
  `evidence/independent-review-12-20261117` behaves as its manifest declares:

  | file | v0.8.3 | r14 | r18 | r19 | declared as |
  | --- | --- | --- | --- | --- | --- |
  | `tests/four.rs` (`four-u3.patch`) | none | skip | skip | none | known miss |
  | `src/deep.rs` (`deep17-u0`) | none | skip | skip | skip | known false positive |
  | `src/mutmacro.rs` | none | none | none | spawn | known false positive |

  "Rounds 14 to 19" for the nested list and "new against round 18" for `mut` both hold.
- **Floors.** The floors in `eval.rs` are consistent with the figures: 177/203 = 871,921 ppm and
  177/260 = 680,769 ppm. The declared counts are 112 and 83.
- **Whitespace.** `git diff --check` reports trailing whitespace in one place, line 20 of the new
  corpus patch. That is the patch's own content: the line ends `#[cfg_attr(a, ` as Git wrote it.
  It is not a defect, provided the gate exempts fixtures, as it did for this commit (32 PASS,
  implementer).

## Findings

| finding | status | how it is closed |
| --- | --- | --- |
| IR12-L-1 | **closed** | The sentence in README, `rust_tokens` and CHANGELOG, "wherever a hunk ends between the `ignore` and the `)]`, at any context width (plain `git diff` included)", is true of the binary and wide enough. The coordinator's earlier premise is corrected in the CHANGELOG addendum. `four-u3` is a known miss in the corpus. |
| IR12-L-2 | **closed** | The README, the head of `rust_tokens`, the docs of `ignored` and `Skips` and TESTING all qualify the whole-attribute rule "for the first 16 `#[cfg_attr(` lists followed at once". They declare the seventeenth list as a writer's false item with no `ignore` and no `]`. `deep17-u0` is a known false positive in the corpus. |
| IR12-L-3 | **closed** | The false "nor … `mut` after `&raw`" is gone from the README and from `begins_expression`. The rule is stated as lexical, with `&raw mut` and `mut` in a discarding macro declared as writer's. The inverted cost line was in the request record, which stays as it was asked; the CHANGELOG has the right figures. |
| VD3R19-M-1 | **closed** (the same narrowing as IR12-L-2) | Vesper quoted four claims: the README, `ignored`, TESTING and the test name. Each is now qualified, except the test name `no_form_is_read_at_the_end_of_a_side`. Its exception is stated in the head comment, and it is declared beside it in the README's writer's limits. I accept that narrowing. |
| VD3R19-L-1 | **closed** (the same narrowing as IR12-L-3) | The README and the doc comment both say the rule is lexical: any `mut` straight before the `<`, with `&mut` as the measured case. `mut-macro` is a known false positive in the corpus. `raw-mut` is declared in the documents only; it does not compile, so no case can hold it. |

### IR12R-L-1 (low, wording, ordinary): an example clause says "two or three lines above"; only three is true

- **The text.** README, ordinary limits: "and the default three lines after a change two or three
  lines above the `ignore` in rustfmt's vertical layout of a condition". The CHANGELOG addendum
  says the same in other words: "Plain `git diff` of a change to two targets of rustfmt's vertical
  `any(…)` ends its hunk on the `ignore` line".
- **What is true.** With three lines of context, the hunk ends on the `ignore` line only when the
  **last changed line is exactly three lines above the `ignore`**. If the last change is two lines
  above, the `)]` is shown and the skip is read.
- **Reproduction.** In the same rustfmt layout, I changed targets 2, 3 and 4 (the last change is
  two lines above `ignore`), giving `notes/f/u3/two-above-u3.patch`. On r19 that patch reports
  `test_skipped`, as r14 and r18 do.
- **Same kind of looseness.** The CHANGELOG's "two targets" holds only for the second and third of
  four. The README's "`-U1` after a changed line of the condition" holds only when that line stands
  directly above the `ignore`.
- **Weight.** The general rule sentence before these examples is exact, so no claim is too narrow.
  The example is merely imprecise. Suggested wording: "the default three lines when the last
  changed line is three lines above the `ignore`".

## Is any sentence now too wide or too narrow?

- **Too narrow:** none found. The whole-attribute rule, the keyword rule and the rustfmt miss are
  each stated at their true reach, and I checked each on the binary.
- **Too wide:** only the example of IR12R-L-1.
- **Unchanged and correct:** the README sentence "Since round 19 none of them is read where the
  side ends before the `]` (but for that seventeenth list)" is true of the binary. My earlier
  probes still hold: the unclosed forms at a `-U0` end give no skip, and the bound is exact at 59
  arguments (read) and 60 (not read).

## What I did not look at

- No `cargo`.
- I did not confirm that the binary built from `7639dd4` is byte-identical. I relied on the
  coordinator's statement and on the source diff being comment-only.
- I did not re-run the bulk comparison: the binary is unchanged.
- I did not read Vesper's re-read.

STATUS: COMPLETE
