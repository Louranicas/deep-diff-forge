# Vesper V3 round 17 review — `aa4acb7`

## Verdict

**NO-GO.** I found one Medium ordinary-valid-Rust false positive, `VD3R17-M-1`. The one edge form retained by this round, `.unwrap::` / `.expect::`, is not unconditionally a method call inside a macro token tree: a macro may accept arbitrary tokens. A valid Git-produced hunk can therefore end on `.unwrap::` while its unseen continuation is not a call, yet the frozen round-17 binary reports `panic_path_added`.

The exact `Command::` reproduction of `VD3R16-M-1` and the exact multiline assertion reproduction of `IR9-M-2` are fixed, but their proposed general replacement—“every grammar continuation is the construct”—is still false. `VD3R16-L-1` is closed. `IR9-M-1` is acceptably declared. `IR9-L-1` through `IR9-L-6` are closed for the specific findings they reported; the new macro-token-tree gap is not among their covered cases.

The forge-warden `no-hidden-text` dogfood failure on three fuzz-seed paths should **not** block on substance. Those paths are raw nine-byte libFuzzer corpus inputs whose first six bytes are documented choice bytes; the control bytes are input data, not hidden source text. One reported deletion/addition pair is a byte-identical rename. The correct operational response is a narrow, auditable waiver or policy exclusion for raw fuzz corpus inputs, not changing the seed bytes or weakening the rule globally. If project process requires a green forge-warden receipt, the receipt remains administratively unresolved until that waiver is recorded, but this policy false positive is not an additional code-review NO-GO. The branch is already NO-GO for `VD3R17-M-1`.

Reviewed exact commit `aa4acb7ad3054b5028a5ac2bf38231f1e5e2f054`, parent `4391c1edf6c663e51b24eb7efd14ee8e9cf0aaac`. Frozen release SHA-256: `3d843af0355ad5edc2cd1636779c9247c4a0ea1e56b9f76a48015777c98a79fb`; debug: `f50ebb12ba796b1a39bb005cea78407f0dd38330c96302504e9adab836c6ca3f`.

## Finding

### VD3R17-M-1 — a macro token tree can continue `.unwrap::` with arbitrary tokens, but the hunk edge is reported as a panic path

The rule's own comment acknowledges that arbitrary continuations are allowed “inside the token tree of a macro” (`rust_tokens.rs:775`), but `waiting` does not establish that the edge is outside one. It reports every changed side ending in `.unwrap::`, `.expect::`, `.unwrap::<` or `.expect::<` (`rust_tokens.rs:836-857`). The generated “every continuation” test only enumerates generic arguments followed by calls; it does not enumerate macro token trees.

This complete Rust program is valid because `discard!` accepts and discards arbitrary token trees:

```rust
macro_rules! discard {
    ($($tokens:tt)*) => {};
}

fn main() {
    discard! {
        value.unwrap::
        NOT_A_CALL
    }
}
```

Rust 1.99.1 compiled and ran it successfully. Git produced this zero-context patch after changing `other` to `unwrap`:

```diff
@@ -7 +7 @@ fn main() {
-        value.other::
+        value.unwrap::
```

Frozen results:

- v3r14: score 2, no evidence.
- v3r16: score 5, `panic_path_added` at new line 7, basis `hunk`.
- v3r17: score 5, `panic_path_added` at new line 7, basis `hunk`.

No `unwrap` method is called; the macro expands to nothing. This is not merely an unparsed invalid suffix: the whole file is accepted by the compiler. Macro DSLs and token-generating macros routinely accept token streams that Rust's expression grammar does not, so “every continuation the grammar allows is the construct” cannot justify an edge inference without proving the tokens are outside a macro token tree. The README also says calls inside macro bodies are read and then claims the retained form has only call continuations, so claim 6 is false as written.

Reproduction: `/var/home/louranicas/.cache/ddf-t/vesper-scratch/v3r17-macro-token-tree/macro-token-tree.patch`, SHA-256 `124b7bed3b4d6e19e4dcb3ea1f69f8ba374cd996535c94aca55d544204e46a64`. The source and scratch Git repository are in the same directory.

The conservative fix is to remove the last edge form too and declare that cut as a miss unless a join supplies the continuation. Alternatively, the edge reader would need reliable token-tree context extending above the fragment, which this hunk-local design intentionally does not have. A regression should place `.unwrap::` and `.expect::<` inside a macro accepting `$($tokens:tt)*`, leave a non-call continuation unchanged outside the hunk, compile the result, and require no signal.

## Requested finding status

- **VD3R16-M-1 — exact reproduction closed; substantive edge-rule class not closed.** `Command::` followed by unchanged `VERSION;` now returns to score 2, matching round 14. `VD3R17-M-1` breaks the general rule used to retain the final edge form.
- **VD3R16-L-1 — closed.** My `q.patch` rises from 6 to 9: `<std::net::TcpStream>::connect` and `<self::UdpSocket>::bind` now add `network_fetch`, while the existing process path remains.
- **IR9-M-1 — acceptably declared.** Multiple hunks at old position 0 require a writer and `git apply --unidiff-zero`, are no worse than v0.8.3 or round 14, and are now named in README, corpus and tests with controls. I do not require another join family for this round.
- **IR9-M-2 — exact reproduction closed; substantive edge-rule class not closed.** The `assert!(` / `true` / `&& ...` form is no longer `trivial_assert`. The replacement rule is broken by `VD3R17-M-1`.
- **IR9-L-1 — closed.** The far `-U0` joins are stated as an ordinary false-positive cost with the 10,333-hunk measurement.
- **IR9-L-2 — closed with acceptable declarations.** Qualified names in angle brackets are read up to the stated eight-name walk. The parenthesized owner, `<Command<>>` and associated-type spellings are obscure, below the released behavior already, and explicitly declared; I accept them as remaining limits.
- **IR9-L-3 — closed.** README and CHANGELOG give the 2.19–2.65 release factors and 451 MB peak shape rather than the former 2.0 claim.
- **IR9-L-4 — closed.** The mirror decoy is stated, represented in the corpus and covered with controls.
- **IR9-L-5 — closed for its reported wording defects.** The old-position-zero sentence, round-14 fuzz-rule history, seven-text comment and prose edge are corrected. The new macro-token-tree wording defect is `VD3R17-M-1`.
- **IR9-L-6 — closed for its listed test gaps.** Rules 13–15, two-line honest forms, old-side forms and the live-reading counter address the reported gaps. The new edge property test is incomplete because it generates only ordinary call continuations, not macro token trees.

## Dogfood `no-hidden-text` failure

I read the supplied forge-warden 0.2.0 receipt and inspected the three named paths byte for byte:

- Removed `a_spawn_cut_after_its_path_separator_the_second_half_first` and added `a_call_cut_after_its_methods_path_separator_the_second_half_first` are both the same nine bytes, SHA-256 `05b3dee058e6063f0323f44fb04d5a59ca5c4df8134028c4d370e51c8829d046`; this is a rename reflecting the changed construct.
- Added `a_lined_hunk_after_two_that_touch_claims_to_end_there_the_second_first` is nine bytes, SHA-256 `a05ced495d494bc3c14688b0d92e61a6e97364af3fdd3456159b18d5cf33b4a9`.
- `rust_hunk_places.rs` documents its input as six choice bytes followed by lines. In the first seed the choice prefix is `06 09 50 c8 01 1e`; in the second it is `01 09 03 01 01 07`. The controls flagged by `hazardous_text` are therefore part of the target's compact input encoding.

The policy rationale—text that reads differently from how code runs—does not apply to these binary selector bytes. A path-scoped waiver should be limited to raw corpus files such as `fuzz/seeds/**`, retain review of filenames and binary changes, and leave `no-hidden-text` fully active for source, fixtures intended as text and generated code. On that basis the dogfood failure should not block this commit's technical review.

## Claims

1. **Held in the supplied comparison scope.** The final two-worker comparison records no round-14 loss among the 6,000 surviving measured patches; the only three plain losses are this round's explicitly named `cfg_attr(..., ignore` cases of its own. I did not rerun all 6,033 patches.
2. **Held with the stated prerequisite.** The supplied comparison has only the known two-section `new_file` loss against v0.8.3. As before, the branch must inherit the released-line duplicate-section refusal before merge.
3. **Broken by VD3R17-M-1.** In a macro token tree, the continuation after `.unwrap::` need not be generic arguments and a call.
4. **Held as a declaration.** Old position 0/1 remains the named exception; one hint does not remove a round-14 join in the reviewed source and supplied comparison evidence.
5. **Held on the supplied measurements and source design; not independently remeasured.** The scans retain one `Views` at a time and the recorded time/memory series is linear at the newly stated factors. I found no new per-hunk whole-file walk.
6. **Broken.** The corpus matches the binary for its represented cases, but the documents' universal edge claim is false and the corpus lacks `VD3R17-M-1`.

## Verification performed

- Read the complete request, round-16 report, complete ninth independent review, full round-17 CHANGELOG entry, changed README/testing claims, production changes in `rust_tokens.rs` and `evidence.rs`, relevant unit/fuzz tests, corpus manifest changes and recorded comparison/control artifacts.
- Confirmed branch, exact HEAD/parent, clean pre-report worktree and frozen binary hashes. `git diff --check` reports only the two intentional trailing-space payload lines inside `fixtures/eval/evidence/independent-review-9-20261115.patch`.
- Re-ran the frozen binaries on `command-colon.patch`, `q.patch` and the new macro-token-tree reproduction.
- Compiled and ran the new complete Rust program with Rust 1.99.1 in `fedora-toolbox-44`, using the real HOME. No Cargo command was run.
- Read the forge-warden dogfood receipt and inspected the exact old/new seed bytes, sizes and hashes.
- Nothing was accessed at GitHub, and no process was killed.

## Not independently repeated

- I did not run Cargo, the workspace tests, gate, clippy, mutation plants, four fuzz campaigns, the 6,033-patch comparison, the 51-input cost set or the large memory measurements. I inspected their recorded artifacts and source-level invariants.
- I did not exhaust macro grammars, procedural-macro inputs, every possible continuation after the retained edge forms, or all qualified-path spellings.
- I did not independently apply the old-position-zero corpus with Git, rebuild the 52-case eval receipt, or rerun the 130,000-patch independent-review generators.
- I did not re-review unrelated branch history, daemon/MCP transport behavior, the release-line parser changes, or any remote forge.

STATUS: COMPLETE
