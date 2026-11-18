# Vesper V3 round 3 review — fixes at `2afeb06`

Reviewed: `cacd43b..2afeb06` on `agent/integrity-signals`

## Verdict: NO-GO

The release time and memory fixes hold under the round-two inputs and a new million-line lexical probe. The message-only span false positives and default-parameter callback miss are also fixed. Two safety-evidence blockers remain: the test-attribute allowlist still trusts unresolved names that Rust can alias to production procedural macros, and an attacker can spend the review parser budget with harmless hunks so that a later parser-only panic path disappears completely. The old-side comparison also has two reproducible ways to suppress a newly trivial assertion.

## Round 2 findings

| Finding | Status | Result |
|---|---|---|
| `VD3R2-H-1` | **Not closed** | Unknown suffixes and `#[my_framework::test]` are fixed, and raw `#[r#test]` is recognized. `VD3R3-H-1` shows that the exact allowlist is still not proof: Rust attribute macros can be imported under an allowlisted name, and dependency crates can be renamed to an allowlisted path. |
| `VD3R2-H-2` | **Closed** | The prior 64,109,679-byte node-heavy input now completes in 2.48 seconds at 146,248 KiB RSS. The 60 MiB `}` input completes in 1.21 seconds at 249,400 KiB RSS. The early hunk-size check removes the reported amplification. The new aggregate-budget correctness failure is separate `VD3R3-H-2`. |
| `VD3R2-M-1` | **Closed for the reported message-only changes** | Changing only a `todo!` or `assert!(true, …)` message now emits no evidence, while `assert!(computed)` to `assert!(true)` still reports. The old-side correspondence introduces separate `VD3R3-M-1`. |
| `VD3R2-L-1` | **Closed** | All five lines in the round-two callback probe are now `error_swallowed`, including default-valued parameters, nested destructuring and comments. Regex, template-literal and ordinary-string delimiters in new probes did not confuse `after_closing`. |

The declared `#[cfg(test)]` helper/`impl` false positive remains as documented and is not a new finding.

## New findings

### VD3R3-H-1 — Allowlisted attribute spellings are not proof of test semantics

Rust resolves procedural attributes through imports and crate names. A production procedural macro can therefore be imported under an allowlisted bare name, and a dependency can be renamed to an allowlisted crate path. DDF does not resolve either fact, but withdraws `panic_path_added` based only on the resulting spelling.

A concrete valid pattern is importing `tracing::instrument` as `rstest`. `instrument` leaves the function in production; the local alias does not turn it into a test. DDF nevertheless trusts `#[rstest]`. Similarly, Cargo can rename an arbitrary procedural-macro package to `tokio`, making `#[tokio::test]` unrelated to Tokio's test harness.

Reproduction (`/tmp/vesper-ddf-r3-attributes.patch`):

```diff
diff --git a/src/aliased.rs b/src/aliased.rs
new file mode 100644
--- /dev/null
+++ b/src/aliased.rs
@@ -0,0 +1,5 @@
+use tracing::instrument as rstest;
+#[rstest]
+pub fn production() {
+    load().unwrap();
+}
diff --git a/src/renamed_crate.rs b/src/renamed_crate.rs
new file mode 100644
--- /dev/null
+++ b/src/renamed_crate.rs
@@ -0,0 +1,4 @@
+#[tokio::test]
+pub fn production() {
+    load().unwrap();
+}
diff --git a/src/control.rs b/src/control.rs
new file mode 100644
--- /dev/null
+++ b/src/control.rs
@@ -0,0 +1,3 @@
+pub fn production() {
+    load().unwrap();
+}
```

Command:

```bash
/var/home/louranicas/.cache/ddf-t/target-integrity/release/deep-diff-forge \
  --stdin-patch --rank --json < /tmp/vesper-ddf-r3-attributes.patch
```

Observed:

```text
src/control.rs       panic_path_added line 2 basis syntax_fragment
src/aliased.rs       no evidence
src/renamed_crate.rs no evidence
```

The source comment already acknowledges the shadowing problem. A short spelling allowlist reduces accidental false contexts but cannot prove resolution, which is the standard required before withdrawing evidence.

Fix: restrict withdrawal to compiler-owned test semantics that cannot be rebound, or resolve attributes against the crate graph/imports. Framework spellings that are not resolved should retain the panic item. If retaining noise for framework test functions is undesirable, mark the item as uncertain/test-likely rather than deleting it. Add an imported-alias production control for every bare external name and a renamed-crate control for every external path.

### VD3R3-H-2 — Harmless early hunks can spend the parser budget and erase later evidence

`FragmentPlan` allocates its 2 MiB strictly in review order. Parser-only spellings do not merely fall back to an item labelled `lexical`: when the lexical rule does not recognize the spelling, there is no item at all. An attacker can choose paths/order so harmless Rust files spend nearly the entire budget, then put a spaced panic path later.

The reproduction consists of 33 new Rust files whose one-line hunks contain 62,000 harmless `a` bytes, one 51,048-byte filler hunk, and then `src/zzz.rs`:

```diff
diff --git a/src/zzz.rs b/src/zzz.rs
new file mode 100644
--- /dev/null
+++ b/src/zzz.rs
@@ -0,0 +1,3 @@
+pub fn load() {
+    value.unwrap ();
+}
```

Generation and command:

```bash
head -c 62000 /dev/zero | tr '\0' a > /tmp/vesper-ddf-r3-filler.txt
head -c 51048 /dev/zero | tr '\0' a > /tmp/vesper-ddf-r3-filler-tail.txt
: > /tmp/vesper-ddf-r3-budget-first.patch
for i in $(seq -w 1 33); do
  printf 'diff --git a/src/a%s.rs b/src/a%s.rs\nnew file mode 100644\n--- /dev/null\n+++ b/src/a%s.rs\n@@ -0,0 +1 @@\n+' \
    "$i" "$i" "$i" >> /tmp/vesper-ddf-r3-budget-first.patch
  sed 's/$/;/' /tmp/vesper-ddf-r3-filler.txt \
    >> /tmp/vesper-ddf-r3-budget-first.patch
done
printf 'diff --git a/src/a34.rs b/src/a34.rs\nnew file mode 100644\n--- /dev/null\n+++ b/src/a34.rs\n@@ -0,0 +1 @@\n+' \
  >> /tmp/vesper-ddf-r3-budget-first.patch
sed 's/$/;/' /tmp/vesper-ddf-r3-filler-tail.txt \
  >> /tmp/vesper-ddf-r3-budget-first.patch
printf 'diff --git a/src/zzz.rs b/src/zzz.rs\nnew file mode 100644\n--- /dev/null\n+++ b/src/zzz.rs\n@@ -0,0 +1,3 @@\n+pub fn load() {\n+    value.unwrap ();\n+}\n' \
  >> /tmp/vesper-ddf-r3-budget-first.patch
wc -c /tmp/vesper-ddf-r3-budget-first.patch
/var/home/louranicas/.cache/ddf-t/target-integrity/release/deep-diff-forge \
  --stdin-patch --rank --json < /tmp/vesper-ddf-r3-budget-first.patch
```

Observed for the 2,100,799-byte combined patch:

```json
{"path":"src/zzz.rs","score":4,"signals":["new_file"],"evidence":null}
```

The same `zzz.rs` diff alone is:

```text
score 7; new_file, panic_path_added;
panic_path_added line 2 basis syntax_fragment
```

This is exploitable in an ordinary sorted patch by naming harmless fillers before the target. It also changes the target's score by three. The outline and hunk views agree with the rank, but they agree on the missing item; no “parser budget exhausted” signal tells the reviewer why.

Fix: an attacker-controlled prefix must not silently disable the only detector for later constructs. Options include a deterministic fair allocation across files/hunks, a cheap near-match pass that prioritizes parser-only spellings, or a general `syntax_unchecked` risk/evidence signal for hunks denied parsing. The strongest fix is to make lower-rung rules cover parser spelling variations such as whitespace before call parentheses, so budget exhaustion downgrades basis rather than erasing evidence. Add a regression where fillers consume all but fewer bytes than the target's cost.

### VD3R3-M-1 — Old-side comparison can mistake another construct or unreadable text for prior evidence

The comparison asks whether any old construct with the same signal starts on the same hunk line. That is not a stable identity for a construct. Two assertions can start on one source line: an existing trivial assertion then causes a different assertion becoming trivial to be treated as already trivial.

Minimal reproduction (`/tmp/vesper-ddf-r3-same-row.patch`):

```diff
diff --git a/tests/same_row.rs b/tests/same_row.rs
--- a/tests/same_row.rs
+++ b/tests/same_row.rs
@@ -1,4 +1,4 @@
 fn check() {
     assert!(true); assert!(
-        computes()
+        true
     );
```

Observed: no evidence. Moving the first assertion to its own context line makes the changed second assertion report `trivial_assert` on line 4 with `syntax_fragment` basis.

There is a second, attacker-controlled suppression. If the old side exceeds the 64 KiB fragment limit while the new side is small, `fragment_cost` admits the hunk but `scan_side(old)` returns `None`; the code then emits nothing because it has “no proof.” A 66,000-byte old expression replaced by `true` reproduces this:

```text
tests/control.rs        trivial_assert line 3 basis syntax_fragment
tests/old_unreadable.rs no evidence
```

The combined patch is only 66,372 bytes (`/tmp/vesper-ddf-r3-old-unreadable.patch`).

Fix: pair old/new constructs structurally, including their ordinal/span or syntax-tree position, rather than by signal and start row. When a new risky construct spans an added row but the old side cannot be read, retain conservative evidence instead of suppressing it; uncertainty must not be attacker-selectable. Add both the same-row two-assertion case and an over-budget old-side case.

## Other verification

- Directly ran the current prebuilt test executables: syntax 105/105, graph 124/124, cluster 43/43, and daemon views 19/19 passed.
- `deep-diff-forge eval` passed at the committed figures: 40 cases, evidence precision 44/46, recall 44/71, findings 33/33, ranking 12/15, 32 declared items.
- Clustered ranking content was identical at parallelism 1, 2, 4 and 8 after excluding the receipt fields that intentionally record worker count. The normalized SHA-256 was `25e165d0f4fe114321c10f7707c6f02cbd6e945c00bb467f6f29961d4e78c8fc` in all four runs.
- Outline and hunk views used the same plan and agreed with rank for the post-budget target.
- Release resource probes:

| Input | Elapsed | Peak RSS | Result |
|---|---:|---:|---|
| Round-two 64,109,679-byte node-heavy patch | 2.48 s | 146,248 KiB | exit 0 |
| Round-two 62,914,671-byte closing-brace patch | 1.21 s | 249,400 KiB | exit 0 |
| 59,999,513-byte, 999,995-line near-match patch | 2.34 s | 311,688 KiB | exit 0 |

None exceeded five seconds or 1 GiB.

## Not examined

- I did not exhaustively enumerate every procedural-attribute import/re-export arrangement or prove which compiler-owned full paths can never be rebound.
- I did not fuzz structural old/new construct correspondence or every JavaScript regular-expression ambiguity.
- I did not test every distribution of the 2 MiB plan across renames, copies, binary files, or every daemon/RPC/MCP entry point.
- I did not run Cargo, Clippy or fuzzers; the build slot was free, but current test executables were already available and were run directly.
- I did not modify repository files, refs, remotes, or worktree content other than this report.

STATUS: COMPLETE
