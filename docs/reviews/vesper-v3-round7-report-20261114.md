# Vesper V3 round 7 review (2026-11-14)

Reviewed: committed snapshot `ccc5e9c..a355bc2` on `agent/integrity-signals`. The untracked round-seven request was treated as the request, not as reviewed code.

## Verdict

**GO WITH A LOW-RISK FOLLOW-UP.** The new member is additive, never changes the evidence signal or score, and is consistent across the machine surfaces. I found one Low precision defect in the hint: its row-only range can mark a production panic path after a test item on the same row, and the parser-visible-only implementation misses an attributed item inside a macro body. Neither case withdraws, demotes, or hides the standing `panic_path_added` signal.

## Finding

### VD3R7-L-1 — `test_likely` is attached by row, not by the attributed item

`FragmentScan::test_spelled` stores inclusive row ranges. `fragment_evidence` turns every added hunk line whose row falls in one of those ranges into a likely line, and `file_evidence_in` puts `test_likely` on the line's `panic_path_added` item. This loses column/item identity at an item's first and last row.

I reproduced a false hint with `/tmp/vesper-v3r7-same-row-after.patch`:

```rust
#[test] fn check() {} fn production() { b().unwrap(); }
```

The test function contains no panic path. The only `.unwrap()` is in the following production function, but both functions occupy row 2. The supplied `a355bc2` debug binary emitted:

```json
{
  "signal": "panic_path_added",
  "side": "new",
  "line": 2,
  "basis": "syntax_fragment",
  "test_likely": true
}
```

The score remained 5 and `signals` remained `["panic_path_added"]`, so this is misleading annotation rather than lost integrity evidence.

The opposite edge is also reproducible with `/tmp/vesper-v3r7-macro-body.patch`:

```rust
outer! {
    #[test]
    fn check() {
        f().unwrap();
    }
}
```

The hunk plainly spells a test attribute around the panic path, but tree-sitter exposes the outer invocation's token tree as unreadable and the walk deliberately does not descend into it. The item therefore has no `test_spelled` range and the reported `panic_path_added` has no `test_likely` member. Nested functions, attributed `impl` blocks, closures inside an attributed function, and calls inside a macro invocation within an attributed function did receive the hint in `/tmp/vesper-v3r7-hint-shapes.patch`; the limitation is specifically an attributed item hidden inside another macro's token tree.

Impact is Low: the signal, score, order, judge input and TUI evidence item remain present. The member is explicitly documented as a non-proof hint. Still, the same-row case labels an entirely production-only panic path “likely test code,” reducing the hint's usefulness in exactly the adversarial presentation it is meant to qualify.

Recommended fix: retain byte/column spans for test-spelled items and associate a panic construct with the enclosing item span instead of any overlapping row. If items inside macro token trees are intended to receive the hint, add a conservative lexical fallback for the attribute/item pairing there; otherwise document that parser-unread macro bodies receive no hint. Add the production-function-on-the-same-row probe as a regression.

## Claims

1. **Nothing is withdrawn and no score changes: held.** `test_spelled` is separate from `test_rows`/`in_test`; only the latter reaches `Parsed::withdrawn`. Risk scoring still keys only on distinct `EvidenceSignal` values. Matched marked/unmarked probes (`#[test]` versus `#[inline]`) both scored 5 with the same sole signal, `panic_path_added`; only the optional member differed. The eval corpus projects evidence to `(path, signal, side, line)` and ignores the hint.
2. **Only `panic_path_added`, new side, can carry the mark: held.** `file_evidence_in` gates the boolean with `signal == PanicPathAdded && side == New` before consulting the sorted likely-line list. I found no other constructor of `Evidence` that sets it true.
3. **CLI and daemon `rank.v0` agree: held.** A marked, multi-shape patch produced value-identical CLI and `rpc`/daemon rank documents (`/tmp/vesper-v3r7-cli.sorted` and `/tmp/vesper-v3r7-rpc-result.sorted`). MCP `rank_patch` calls daemon `diff.rank`; its bounded summary clones complete ranked entries, so retained entries keep the member. The Rust SDK returns the shared `RankedFile`/`Evidence` types, and the Python SDK returns the daemon document without narrowing its evidence shape. Both CLI and daemon omit the member when false.
4. **The member is documented and presented as a hint: held.** README and `docs/API_AND_IPC.md` explicitly say the member is a hint, never proof, and changes neither signal nor score. The TUI says: “Under a test attribute: likely test code, which the patch does not prove.” VD3R7-L-1 concerns the precision of when that statement is attached, not an unsafe consumer action.

## Other adversarial checks

- Nested functions, an attributed `impl`, a closure body, and a macro invocation inside an attributed function were marked; the immediately following function on another row was not.
- A real production item can deliberately carry `#[my_framework::test]` and earn the hint, as declared, but its signal and score stand.
- Outline and hunk views continue to expose distinct signal labels rather than evidence-item metadata; they do not drop or demote marked items. Cluster lanes rank the same shared `Evidence` values. TUI creates the same grounded note and only appends the qualified hint sentence.
- Evidence truncation is still based on item count and occurs after score calculation; the added boolean does not alter which evidence items are selected.
- MCP's `rank-summary.v0` copies retained daemon rank entries rather than rewriting their evidence objects.

## Tests

The hive slot was free before each Cargo invocation. Cargo ran inside `fedora-toolbox-44` against a `git archive` of `a355bc2` under `/tmp`.

- Syntax: 108 unit + 10 integration tests passed.
- Graph: 134 unit + 6 integration tests passed.
- TUI: 226 tests passed.
- CLI `rank_evidence`: 5 tests passed.
- CLI `daemon_parity`: 4 tests passed.
- `git diff --check ccc5e9c..a355bc2` passed.
- Live supplied-binary checks reproduced VD3R7-L-1, verified score/signal invariance, and verified CLI/daemon equality on marked evidence.

## Not examined / limitations

- I did not rerun the fuzz campaign or full 32-step local CI.
- I did not test every Rust attribute grammar extension, procedural macro expansion, or malformed tree-sitter recovery shape.
- I did not start the graphical TUI interactively; I inspected its note construction and ran its test suite.
- I did not review changes outside `ccc5e9c..a355bc2`, modify repository refs/index/configuration/remotes, or alter the untracked request file. Only this report was added.

STATUS: COMPLETE
