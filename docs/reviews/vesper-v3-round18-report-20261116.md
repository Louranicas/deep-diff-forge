# Vesper V3 round 18 review — `af74308`

## Verdict

**NO-GO.** I found one Medium in the same end-of-side inference class this commit claims to close, `VD3R18-M-1`: the new `#[ignore(…)]` handling reports `test_skipped` as soon as it sees `#[ignore(` or a conditional `ignore(`, without seeing the closing delimiters. A real `git diff -U0` can therefore end its side on that opening parenthesis inside a macro token tree which compiles and skips no test. The round-18 binary alone scores both forms as skips.

I also found one Low in the bracketed-path keyword class, `VD3R18-L-1`: rustc 1.99.1 supports expression operands after `become` and after `do yeet`, but the hard-coded keyword list omits `become` and `yeet`, so bracketed released paths after them are not read. This behavior is not lower than v0.8.3 or round 14; the completeness claim is new.

Because this review is under the round cap, I recommend using the one remaining build round to close `VD3R18-M-1` by requiring the complete attribute/list before reporting it, or explicitly narrow the no-edge claim and declare the incomplete-prefix macro false positive. Do not add another special end hint. `VD3R18-L-1` is a small finite-set fix, or the keyword claim can be narrowed to the seven spellings actually implemented.

## Disposition of the prior findings

- **VD3R17-M-1: closed.** The exact frozen `macro-token-tree.patch` falls from round 17's score 5 with `panic_path_added` to round 18's score 2 with no signal. `waiting` and the `end` argument are removed, and the whole-text macro form remains a hard negative.
- **IR10-M-1: the positive case is closed, but its fix opens VD3R18-M-1.** On `ignparen.patch`, round 18 reports `test_skipped` for the complete `#[ignore(not_ready)]` and `#[cfg_attr(all(), ignore(flaky))]`; v0.8.3, round 14 and round 17 do not. The new implementation also reports the incomplete opening-parenthesis prefixes, contrary to the “whole text only (never at an edge)” claim.
- **IR10-L-1: acceptably declared.** `newtwotop.patch` remains a declared miss (score 4, `new_file` only) and `newtop-natural.patch` remains the declared false positive (score 7, `new_file,panic_path_added`). README, CHANGELOG, the corpus manifest and code comments identify the created-file/old-position-zero family and its writer prerequisite.
- **IR10-L-2: its reported spellings are fixed, but the keyword class is not complete.** Round 18's hunk output labels all five hunks of `kw.patch` `process_spawn`, including `return`, `match`, `in` and labelled `break` plus the control. Source tests cover `yield`, `if` and `while`. `VD3R18-L-1` gives the two omitted current-rustc expression prefixes.
- **IR10-L-3: closed.** `range-u0.patch` falls from round 17's score 5 with `panic_path_added` to round 18's score 2 with no signal; the whole-line range false positive remains declared as inherited from round 14.
- **IR10-L-4: its stale round-16 sentences are corrected.** The current documents describe the new rule and the view-line count accurately. Their new no-edge/whole-text statements are broken by `VD3R18-M-1`, rather than the old wording remaining.

## Findings

### VD3R18-M-1 — Medium — `#[ignore(` and conditional `ignore(` are still inferred at a side's end

**Claims quoted:**

1. “**The rule now, in one sentence: a hunk side is read as text that ends there; nothing is inferred from how it ends.**” (`CHANGELOG.md`, round 18).
2. “`#[ignore(`, `#![ignore(` and `ignore(` as an attribute of its own in a `cfg_attr` list are read as `test_skipped` **on the whole text**, at any spacing the reader handles (**never at an edge**).” (same entry).
3. “In round 18 no form is read at the end of a side” (`docs/TESTING_GOLD_STANDARD.md`).

**Class:** end-of-side inference / incomplete construct. This is the same class as VD3R17-M-1 and IR10-L-3, not a new class. The class is not closed because the newly added `ignore(…)` form recognizes its opening parenthesis without its closing tokens. The general fix is to report the parenthesized form only after its matching `)` and containing `]`/outer list have been seen. If that is not feasible within the remaining round, narrow the claim and declare the macro-token-tree false positive; do not add an end-specific exception.

**Reproduction:** `/tmp/vesper-v3r18/repro` is a scratch Git repository. Both files contain a macro which accepts and discards arbitrary tokens. The committed version says `allow(`; the worktree says `ignore(`, while the unchanged following lines close the tokens. Both worktree files compile with rustc 1.99.1 and run successfully, and there is no test to skip.

`git diff -U0` is:

```diff
diff --git a/cfg.rs b/cfg.rs
@@ -5 +5 @@ fn main() {
-        #[cfg_attr(all(), allow(
+        #[cfg_attr(all(), ignore(
diff --git a/direct.rs b/direct.rs
@@ -5 +5 @@ fn main() {
-        #[allow(
+        #[ignore(
```

Frozen results:

```text
v0.8.3: cfg.rs 2 [], direct.rs 2 []
round 14: cfg.rs 2 [], direct.rs 2 []
round 17: cfg.rs 2 [], direct.rs 2 []
round 18: cfg.rs 5 [test_skipped], direct.rs 5 [test_skipped]
```

Round 18's `--hunks` output places `"evidence":["test_skipped"]` on each one-line hunk. `toolbox run -c fedora-toolbox-44 rustc --edition 2021 direct.rs` and the same command for `cfg.rs` both succeeded; the resulting programs exited 0. Toolbox ran with the real HOME.

The source cause is direct: `ignored` accepts `What::Mark(b'(')` as an ending token, and `Skips::follow` calls `skip` when the next token after a candidate `ignore` is `(`. Neither path requires the parenthesis or attribute to close. The unit table even marks `"a, cfg_attr(b, ignore ("` as a skip, while `no_form_is_read_at_the_end_of_a_side` does not include either opening-parenthesis form among its 52 cases.

**New against the last release:** yes. Published v0.8.3, round 14 and round 17 all produce no signal on both patches; only round 18 reports the false skips. The positive complete-parenthesis behavior is also new in round 18, but these prefixes do not establish it.

### VD3R18-L-1 — Low — the expression-prefix keyword list omits `become` and `yeet`

**Claim quoted:** “Bracketed released paths after a keyword that begins an expression: `return`, `match`, `in`, `break` with or without a label, `yield`, `if`, `while” (request/CHANGELOG), and README's “where its `<` follows … a keyword after which an expression begins”.

**Class:** bracketed released-path keyword completeness, the same class as IR10-L-2. It should be closed once for rustc 1.99.1 by adding the remaining expression-prefix keywords, or the claim should be narrowed to the seven enumerated spellings. The code fix is a small `Word`/`begins_expression` extension with no state or schema change, so under the convergence rule I favor fixing it in the already-required round.

**Reproduction:** rustc 1.99.1 accepts and runs both of these with the named unstable features enabled (`RUSTC_BOOTSTRAP=1`, necessary because this rustc binary is on the stable channel; the implemented list already includes feature-gated `yield`):

```rust
#![feature(explicit_tail_calls)]
fn run<S: AsRef<OsStr>>(program: S) -> Command {
    become <Command>::new(program)
}

#![feature(yeet_expr)]
fn make_error() -> Result<(), Command> {
    do yeet <Command>::new("true");
}
```

The complete programs are `/tmp/vesper-v3r18/become.rs` and `/tmp/vesper-v3r18/yeet.rs`; each compiled with the frozen review compiler and exited 0. A `-U0` patch changing the implemented control `return <Command>::new(program)` to `become <Command>::new(program)`, and a patch adding the `do yeet` line, produce no `process_spawn` on round 18. The source maps both names to `Word::Other`, so the `<` is rejected as following an ordinary name.

**New against the last release:** the missing signals are not new: v0.8.3, round 14, round 17 and round 18 all miss them. The claim that the expression-prefix keyword class is handled is new in this round. Severity is Low because both forms require unstable features and the fix or narrowing is small.

## Claims

1. **No form is read at the end of a hunk side: BROKEN** by VD3R18-M-1. The prior `.unwrap::`/`.expect::` edge forms are gone, but the newly added opening-parenthesis forms are read on one-line sides.
2. **Monotone against round 14 except the declared bare `cfg_attr` `ignore`: HELD on the cases I ran, not exhaustively remeasured.** The only decreases I observed are the declared edge removals; VD3R18-M-1 is an addition. `cfgu0.patch` is exactly the declared round-14 loss.
3. **Never lower than v0.8.3 except the second section for one path: HELD on the cases I ran, not exhaustively remeasured.** None of my reproductions loses a v0.8.3 signal.
4. **Complete `#[ignore(…)]` spellings are skips and `#[ignore[x]]` is not: HELD for the focused positive and source controls.** The implementation's incomplete-prefix behavior is outside that positive claim and breaks Claims 1 and 6.
5. **Cost and memory linear at the stated factors: NOT INDEPENDENTLY TESTED.** The changed readers remain one forward token pass with bounded windows/state by inspection; I did not repeat the 51-input timing or large memory runs.
6. **Corpus and documents say what the binary does; declared limits complete: BROKEN** by both findings. VD3R18-M-1 contradicts “whole text only / never at an edge”; VD3R18-L-1 is absent from the bracketed-path limit/keyword list. The IR10-L-1 and bare-`ignore` declarations themselves are accurate.

## Other checks

- Verified HEAD `af74308fc539afa243a801425b79ceca737a70af`, branch `agent/integrity-signals`, and a clean worktree before writing this report.
- Verified frozen SHA-256: release `22d4d7e0c4fe160be7761672bd387828e117124b0aa831c49994605df7ca43f5`; debug `1fd2687ddd4bc228c7845c90bff28dfc1c3e15aef64f84b726e11a998f3ccac9`; round 17 release `3d843af0355ad5edc2cd1636779c9247c4a0ea1e56b9f76a48015777c98a79fb`.
- Read the full round-18 CHANGELOG entry, the round-17 Vesper report, the tenth independent report, the complete `aa4acb7..af74308` production changes in `rust_tokens.rs`/`evidence.rs`, the changed tests, README declarations, testing document and corpus manifest.
- `git diff --check aa4acb7..af74308` reports one trailing-space line in the committed independent-review fixture; it is reproduction data, not production whitespace.
- The exact round-17 macro and range reproductions now have no signal. The complete `ignparen.patch` gains `test_skipped`; the `kw.patch` hunks each carry `process_spawn`; the two old-position-zero created-file cases match their declarations.

## What I did not test

- The hive slot was HELD (`hive slot show` began with `{`), so I ran no Cargo command, build, workspace test, gate, mutation plant or fuzz campaign. I did not rerun the 6,059-patch comparison, 51-input cost set, 265-cut generator, 43-seed plain harness or the pending `evidence_scan` fuzz campaign.
- I used the supplied frozen binaries. The only compiler checks were the small reproductions above with rustc 1.99.1 inside `fedora-toolbox-44`, always with the real host HOME.
- I did not exhaust every macro grammar, unstable Rust expression feature, raw-identifier spelling, nested `cfg_attr` depth, non-Rust language, or the whole branch history `d4dbc91..af74308` outside the integrity-signal path and its declarations.
- I did not access GitHub, mutate a remote or ref, or kill any process.

STATUS: COMPLETE
