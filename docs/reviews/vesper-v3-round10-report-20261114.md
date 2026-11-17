# Vesper V3 round 10 review (2026-11-14)

Reviewed: committed snapshot `1228c55..2824a8b` on `agent/integrity-signals`, using the frozen round-10 release binary named in the request. I wrote only this report in the repository.

## Verdict

**NO-GO.** VD3R9-M-1 is closed. The unsafe test-code withdrawal is gone, the four independent-review proof failures are closed by that deletion, the square budget bounded every supplied hostile case I ran, the rank/outline duplicate scan is gone, and the corpus matches the frozen binary.

I found one new Medium regression in the view cache: asking `--hunks --address` for one cheap hunk now scans every hunk in that hunk's file. A cheap one-line response took 1.77 seconds because an unselected 22 KiB hostile hunk in the same file consumed the square budget; the round-9 implementation returned the same selected hunk in 0.00 seconds.

## Prior Vesper finding

### VD3R9-M-1 — closed

I reran `/tmp/vesper-v3r9-raw-multiline.patch`. The frozen binary reported all four multiline path forms at lines 5, 9, 13 and 17 as `panic_path_added` with `syntax_fragment` basis, including the user-defined and standard paths passed as values. The removed raw-spelled attribute now produced `test_removed` at old line 1. The file signals and scores were otherwise as expected.

The implementation classifies readable `scoped_identifier` nodes as path constructs while excluding `use`, macro, attribute and type-path contexts. The old-side raw and spaced attribute matching also covers `#[r#test]`.

## Independent review 2 findings

- **IR2-H-1 — closed by deletion.** The `//*` grammar/compiler disagreement still affects the hint, but it cannot withdraw the production panic path. The original `h/n1.patch` now reports `panic_path_added`.
- **IR2-H-2 — closed by deletion.** The scanner's low-byte Unicode disagreement can no longer remove evidence. The original `h/n2.patch` reports the panic path; `test_likely` remains only a hint.
- **IR2-H-3 — closed by deletion.** The non-nesting lexical code view is no longer used to prove that evidence may be removed. The original `h/n3.patch` reports the panic path. The distinct macro-body nested-comment miss remains declared, as discussed under IR2-M-2.
- **IR2-H-4 — closed by deletion.** Edition-dependent `cr"..."` parsing can no longer withdraw evidence. The original `h/n4.patch` reports the panic path; its hint is non-authoritative.
- **IR2-H-5 — closed.** The plan charges both sides by bytes and by the sum of their squares; oversized hostile hunks become `syntax_unchecked`. Outline now obtains scores and hunk labels from one uncapped file assessment. The supplied hostile runs stayed within 1.84 seconds user for rank and 1.76 for outline in my runs, apart from the purpose-built combined-budget case I did not rerun. The new targeted-selection regression is VD3R10-M-1 below.
- **IR2-M-1 — closed.** Nested and string-valued `cfg_attr` condition changes now produce `test_skipped`; I reran `case-m3_nested_condition.patch`, `case-m3_string_condition.patch`, and `case-m3_oneline_nested.patch` successfully. Whole whitespace-stripped attribute text, not only its words, participates in old/new equality.
- **IR2-M-2 — not fully closed, with the remainder declared.** Raw identifiers, multiline path calls/values, nested `cfg_attr`, and the plain-code nested-comment case are now accounted for. A call after the nested-comment spelling inside another macro body is still silent (`case-macro_body_nested_comment_unwrap.patch` produced no evidence), exactly as the request and corpus declare.
- **IR2-L-1 — not closed, but its safety consequence is deleted and the behavior is declared.** `test_likely` can still be true on production code in IR2-H-1, H-2 and H-4. It no longer changes a signal, score, ordering or withdrawal; the docs describe it as an untrusted reader hint.

## New finding

### VD3R10-M-1 — selecting one hunk scans hostile unselected hunks in the same file

`hunks()` creates `Scans` for the review and asks `scans.labels(f, hunk)` for each selected unit (`views.rs:473-492`). `labels()` calls `file(index)`, whose lazy initializer runs `assess_file_in` across the entire file (`views.rs:169-193`). That is appropriate for `outline` and `Selection::All`, which need file scores or every hunk's labels, but it is unnecessary for a selection naming one address.

I built `/tmp/vesper-v3r10-selected-cost.patch` with two hunks in one file:

- selected hunk: one added line, `fn cheap() {}`;
- unselected hunk: the supplied 22 KiB `// todo` / `#!#!...` quadratic text.

The selected address was `e6771adb0ea2d0c6939e1def8c387e877a0ffe31d5b583ea51673eddff03001a`. With the frozen round-10 release binary:

```text
deep-diff-forge --stdin-patch --hunks --json --address <address>
round10 selected real=1.77 user=1.76 rss=11392 KiB
```

The response contained exactly the one cheap line and no evidence. The frozen round-9 binary returned that same selected address from the same patch in 0.00 seconds. Thus an agent drilling into one already-known hunk can pay the entire file/review parser budget for unrelated, unreturned hunks. The maximum is bounded, so this is not IR2-H-5's unbounded denial of service, but it is a material latency regression on the endpoint whose selection is intended to narrow work.

Recommended fix: retain the one-file assessment for outline/all selection, but for address/path paging compute labels only for selected hunks under the already-built review plan (and cache each requested hunk once). Add a performance regression with a cheap selected address and a hostile unselected hunk in the same file.

I found no new High or Low finding.

## Claims

1. **Monotone — held in the paths inspected and reproduced.** `Parsed` has only additive `found`, `unread` and hint state; `hunk_matches` can raise a basis or add an item but has no removal branch. The four former proof exploits retain their lexical/parser evidence. I did not run a fuzz campaign.
2. **One input, one answer — held.** The fragment evidence path calls the non-clocked parser and the plan is a deterministic function of review order and counts. I found no deadline or progress callback reintroduced into that path.
3. **Bounded by counts — held as a bound; targeted-selection efficiency is broken.** The linear and square charges are saturating and reject before parsing once either review budget would be exceeded. Supplied release timings included: `hb16x16k` 1.70 seconds user rank / 1.10 outline; `hb32x8k` 1.84 / 1.75; `hb4x64k` 0.00 / 0.01; one 22 KiB hostile hunk 1.80 / 1.71; `tern34x3800` 1.91 / 1.92; and the 880 KiB repeated `cfg_attr` case 0.04 / 0.05. Outline was not slower than rank in these runs. VD3R10-M-1 shows that a narrowly selected hunk unnecessarily pays the bounded whole-file cost.
4. **A parser word is accounted for — held with the declared exception.** The prior multiline path cases are items; budgeted-out hunks receive `syntax_unchecked`. The known nested-comment call inside another macro body remains silent and declared.
5. **Corpus honesty — held.** `eval --corpus fixtures/eval --json` returned 45 cases, no failures, precision 77/90, recall 77/119, findings 33/33, ranking 12/15, 42 known misses and 58 declarations.

## Other attacks and checks

- The path construct does not report the `unwrap`/`expect` method definitions themselves. A valid enum-pattern path `E::unwrap`/`E::expect` is reported; that is the explicitly declared “any `Type::unwrap` or `Type::expect` path” false positive, not a new finding.
- The four former whole-file proof reproductions all retained `panic_path_added`; three carried the fallible `test_likely` hint, without score suppression.
- `git diff --check 1228c55..2824a8b` passed.
- The hive slot reported `free` immediately before Cargo. In `fedora-toolbox-44`, with `CARGO_TARGET_DIR=/tmp/vesper-v3r10-target`, tests passed for `deep-diff-forge-daemon` (226 passed, 1 ignored), `deep-diff-forge-graph` (147 unit and 6 integration), and `deep-diff-forge-syntax` (114 unit and 10 integration), plus their doc tests.

## Not examined / limitations

- I did not run the full workspace suite, Clippy, Miri, a fuzz campaign, or the complete local gate.
- I did not rerun all 300 historical commits or every independent-review performance shape; I used the supplied representative and budget-bound cases.
- I did not start a persistent daemon or exercise RPC/MCP/TUI transports live. I exercised the frozen CLI view paths and inspected their shared daemon implementation and consumers.
- I did not formally prove a worst-case tree-sitter bound for every byte string within the square budget.
- I did not test non-Linux platforms or every Rust edition/nightly grammar extension.
- I did not review changes outside `1228c55..2824a8b` or modify repository refs, remotes, index, configuration, or files other than this report.

STATUS: COMPLETE
