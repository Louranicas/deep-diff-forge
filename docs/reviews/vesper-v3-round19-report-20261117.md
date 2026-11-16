# Vesper V3 round 19 review — `f819a56`

## Verdict

**NO-GO as presently claimed.** The focused fixes for my round-18 reproductions and the eleventh independent review's reported cases work, but the new absolute whole-attribute rule retains a deliberate overflow exception: the seventeenth open `#[cfg_attr(` immediately reports `test_skipped`, even when the side does not hold a closing `]` and no `ignore` appears at all. That is `VD3R19-M-1`, in the same incomplete-construct/end-of-side class the last build round was meant to close.

I also found one Low in the keyword-context class: round 19 treats every `mut` immediately before `<Command>` as the measured `&mut` case. It therefore reports `process_spawn` after `&raw mut` despite the documents explicitly saying that case is not read, and in a compiling discard-macro token tree which starts no process.

Because this was the line's last build round, I recommend the promised docs-only narrowing, not another build: qualify the whole-attribute rule with the sixteen-list overflow exception and declare its writer-only false positive; qualify the keyword rule as a lexical immediate-keyword rule and declare the `mut` context false positive. I would re-read that docs-only narrowing as the convergence rule permits. I do not recommend merging `d4dbc91..f819a56` with the present absolute claims.

## Prior finding dispositions

- **VD3R18-M-1: the two reported forms are fixed, but the class is not closed.** On my exact one-line `#[ignore(` and `#[cfg_attr(all(), ignore(` patches, round 18 scores 4 with `test_skipped`; round 19 scores 1 with no evidence, as do v0.8.3 and round 14. The complete forms in the new corpus still score as skips. `VD3R19-M-1` is the remaining same-class exception.
- **VD3R18-L-1: closed for the reported cases.** The frozen round-19 binary reports `process_spawn` after `become` and `do yeet`; v0.8.3, round 14 and round 18 do not.
- **IR11-L-1: acceptably declared.** `vecturbo-u0.patch` remains a round-19 `process_spawn` false positive, while v0.8.3 and round 14 report none. README, the head of `rust_tokens`, CHANGELOG and the corpus now identify the unseen `Vec::` continuation and classify it as writer-only.
- **IR11-L-2: closed for `&mut`, but the keyword-context class has VD3R19-L-1.** Both `mutref-u0.patch` and `mutref-u3.patch` gain the two expected spawn items only in round 19.
- **IR11-L-3: closed.** README names rounds 18 and 19 among the rounds that took findings back; the stale round-18/round-17 claims are corrected in the current round's entry; the memory statement now says within 2 MiB. Historical request/report text is appropriately unchanged.

## Findings

### VD3R19-M-1 — Medium — the seventeenth open `cfg_attr` reports a skip before the attribute is whole

**Claims quoted:**

1. README: “**A skip is read only where the side holds its attribute whole, from `#` to its `]`**”.
2. `rust_tokens.rs`, `ignored`: “**The attribute is read only when the side's text holds it whole**”.
3. `docs/TESTING_GOLD_STANDARD.md`: “In round 19 a skip is read only where the side holds its attribute whole”.
4. The test/property name `no_form_is_read_at_the_end_of_a_side` and the request's Claim 1: no form is read at the end of a side.

**Class:** incomplete construct / end-of-side inference, the same class as VD3R18-M-1 and VD3R17-M-1. It is not closed by one rule because `Skips::read` explicitly bypasses that rule when `self.open.len() >= CFG_ATTR_LISTS`: it calls `Self::note` at the opening parenthesis, without an `ignore`, a closing `)`, or the attribute's `]`.

**Reproduction on the frozen binaries:** `/tmp/vesper-v3r19/depth-old.rs` and `depth-new.rs` are complete Rust programs. A discard macro accepts arbitrary tokens. The old form has seventeen nested `#[allow_attr(a, ` openers; the new form has seventeen nested `#[cfg_attr(a, ` openers; a later unchanged line closes every delimiter. Both compile with rustc 1.99.1 and run with exit 0. Neither contains `ignore`, defines a test, nor skips one. Their real `diff -U0` side ends on the seventeen openers before any closer.

| Binary | Result for `tests/x.rs` |
| --- | --- |
| published v0.8.3 | score 1, `test_only`, no evidence |
| round 14 | score 4, `test_only,test_skipped` |
| round 18 | score 4, `test_only,test_skipped` |
| round 19 | score 4, `test_only,test_skipped`, line 4 |

The production unit test pins the same behavior directly: `nest(CFG_ATTR_LISTS)` has no signal and `nest(CFG_ATTR_LISTS + 1)` has `TestSkipped`. The round-19 CHANGELOG and source comments mention the exception (“taken to hold a skip”), but they do not narrow the four absolute claims above or list it as an ordinary/writer false positive. The corpus also has no declaration for it.

**New against the last release:** yes: v0.8.3 reports no skip. It is not newly introduced by round 19; round 14 and round 18 already report it. What is new is the round-19 class-closing claim that every skip requires a whole attribute.

**Severity and narrowing:** Medium because `test_skipped` is a machine-consumed integrity signal and the reproduction is a compiling change with no skipped test. It needs a writer to put deeply nested attribute-shaped tokens inside a macro; ordinary Rust does not nest attributes this way. Under the round cap, narrowing is the right answer: state everywhere that the whole-attribute rule holds only for the first sixteen concurrently followed `#[cfg_attr` lists, and that opening another is conservatively reported as a writer-only false positive even without `ignore` or `]`. Add it to the corpus's declared false positives; do not open round 20.

### VD3R19-L-1 — Low — `mut` opens a bracketed path outside the measured `&mut` context

**Claims quoted:**

1. README: the measured list contains “`mut` (**of `&mut`**)” and “nor `const` or **`mut` after `&raw`**”.
2. `Word::begins_expression`: “`mut` (of `&mut`)” and “nor `const` and `mut` after `&raw`”.
3. Request Claim 4: “The keyword list is exactly the measured one”.

**Class:** bracketed released-path keyword context, the same class as IR11-L-2 and VD3R18-L-1. The finite keyword list is correct, but the implementation tests only the immediately preceding word; `Word::Mut.begins_expression()` does not require the preceding `&` or reject `&raw mut`.

**Reproduction on the frozen binaries:** `/tmp/vesper-v3r19/raw-mut.patch` adds `let p = &raw mut <Command>::new("true");`. Round 19 reports score 6 and `process_spawn`; v0.8.3, round 14 and round 18 report score 2 with no signal. `/tmp/vesper-v3r19/mut-macro.patch` changes a discard macro body to `discard! { mut <Command>::new("true") }`; round 19 again reports the spawn and all three earlier binaries do not. The complete macro program `/tmp/vesper-v3r19/mut-macro.rs` compiles with rustc 1.99.1 and exits 0; `Command` is unused and no process starts.

**New against the last release:** yes, and new against round 18.

**Severity and narrowing:** Low. The compiling reproduction needs a macro that accepts and discards arbitrary tokens; the `&raw mut` spelling is rejected as taking a raw address of a temporary, exactly as the documents say. The context check would be under twenty lines, but this is the last build round. Narrowing is therefore right: say the lexical reader opens the path after an immediately preceding `mut`, with `&mut` the measured positive, and declare other macro/raw contexts as writer false positives.

## Claims

1. **Whole attribute / no end inference: BROKEN.** The focused direct and conditional `ignore(` cases are fixed and the 64-token direct-attribute bound is declared, but VD3R19-M-1 remains an explicit exception before `]`.
2. **Monotone against round 14 except the stated attribute cases: HELD on the supplied 6,110-patch artifacts and my focused comparisons, not independently regenerated.** The 15 plain losses are the six bare-`ignore` cases and nine newly declared incomplete-attribute cases; strict adds the pre-existing bounded-list item. My new depth reproduction is already a round-14 false positive, not a new loss.
3. **Never lower than v0.8.3 except the second section for one path: HELD on the supplied 6,110-patch artifacts and my focused comparisons, not independently regenerated.** The sole listed loss is `two-sections-create`, the known 0.8.4 integration dependency.
4. **Exact measured keyword rule: BROKEN by VD3R19-L-1.** The measured positive set (`&mut`, `become`, `do yeet` included) is implemented, and ordinary names before `<` remain controls, but `mut` is applied outside its measured grammar context.
5. **Cost and memory linear at the stated factors: NOT INDEPENDENTLY TESTED.** By inspection the changes retain bounded lookahead and one-pass list tracking. I read the 51-input, memory, fuzz and gate records but did not repeat them.
6. **Corpus/documents match the binary and declared limits are complete: BROKEN.** The frozen `eval` run itself matches the stated 54 cases, precision 177/201, recall 177/259, findings 33/33, ranking 12/15 and 109 declarations. The two findings above are not declared in that corpus, and the absolute prose does not describe the overflow behavior.

## Other checks

- Verified HEAD `f819a5645724054b4719657d8c44f2d1269dea2b`, the requested delta `af74308..HEAD`, and frozen SHA-256 values: release `54b93d22fe88a43e519bb2e33276fcf41b60ed8e6e164af1283492b520dcf877`, debug `f3867db1d72fbe580855eb8c63a83a7d85c3a977a3a6304d1f6ab46867123190`, round-18 release `22d4d7e0c4fe160be7761672bd387828e117124b0aa831c49994605df7ca43f5`, round-14 release `11ae63e4e560bef15c60f1a87a3a493c5fb40f9f966500c2ed025ec20b4b707b`.
- Read the full round-19 CHANGELOG entry first, both prior reports, every production/test/document/corpus change in the commit, the focused controls, and the comparison summaries. `git diff --check af74308..HEAD` is clean.
- Ran the frozen round-19 `eval` against the committed corpus: exit 0 with exactly the documented metrics and no failures.
- The worktree already held untracked `docs/reviews/claude-independent-review-12-20261117.md` at intake. I did not read, change, stage or remove it.

## What I did not test

- I did not build or run Cargo, the workspace tests, gate, mutation plants or fuzz targets. I used the frozen binaries and read their supplied logs/artifacts as required.
- I did not regenerate the 6,110-patch set, the 265 cuts, 177,957 fragments, 677/1,354 keyword compilations, or 51-input cost suite. I checked their recorded classifications and independently reproduced the changed high-risk cases.
- I did not exhaust every possible attribute delimiter, macro token tree, keyword grammar position, non-Rust language or the entire branch outside the integrity-signal path.
- I did not access GitHub, mutate refs/remotes, kill processes, or alter any file other than this report.

STATUS: COMPLETE
