# Vesper V3 round 2 review — fixes at `cacd43b`

Reviewed: `f2a366d..cacd43b` on `agent/integrity-signals`

## Verdict: NO-GO

The literal round-one false-`cfg_attr` reproduction and the original multiline/catch reproductions are fixed. Two release blockers remain, however. An arbitrary procedural-attribute path ending in `::test` is still accepted as proof and can suppress production panic evidence, and an admitted release-build workload made of many just-under-budget Rust fragments still exceeds the 10-second production limit. The span fix also reports pre-existing stubs/assertions when only their message changes.

## Round 1 findings

| Finding | Status | Result |
|---|---|---|
| `VD3-H-1` | **Not closed** | `#[cfg_attr(any(), test)]`, `#[allow(test)]`, an ordinary `mod tests`, and the other original shaping controls now retain production panic evidence. `VD3R2-H-1` finds a remaining unsafe proof rule: any path ending in `::test` withdraws evidence, even when it is an unrelated custom procedural macro. |
| `VD3-H-2` | **Not closed** | The original 60 MiB all-`a` line completes in 2.01 seconds on the release binary, so that exact debug-build result is closed. `VD3R2-H-2` gives a different 61.1 MiB release input that remains alive after 15 seconds, beyond forge-warden's 10-second limit. |
| `VD3-M-1` | **Closed for the reported cases** | The Rust multiline assertion is now `trivial_assert` on its added `true` row, and the next-line JavaScript empty block is `error_swallowed`. Python and TypeScript multiline assertions remain declared misses. The general span implementation introduces the separate false positive `VD3R2-M-1`. |
| `VD3-L-1` | **Closed as scoped** | `Result::unwrap`, empty Promise `.catch`, and `test.skipIf` now fire. The remaining enumerated variants are committed declared misses. `VD3R2-L-1` is a new callback spelling missed by the new implementation. |

I agree with retaining `test_skipped` for `#[cfg_attr(condition, ignore)]`, including `any()`: unlike panic withdrawal, that decision preserves rather than removes safety evidence. The false-condition noise is acceptable and is not a finding.

## New findings

### VD3R2-H-1 — An arbitrary attribute path ending in `::test` still hides production panic evidence

`opens_test_code` treats every function attribute whose compact path ends in `::test` as proof of test code. Rust permits dependencies to export procedural attributes with arbitrary paths and behavior; a custom `#[my_framework::test]` need not be the built-in test harness or remove the function from production. The parser nevertheless withdraws the method-call item.

The reverse normalization is also incomplete: Rust accepts the built-in attribute as `#[r#test]`, but DDF does not recognize the raw identifier and reports the test's `.unwrap()` as production evidence. I verified `#[r#test]` is accepted by Rust 1.88 with `rustc --test --emit=metadata`.

Reproduction excerpt from `/tmp/vesper-ddf-r2-probes.patch`:

```diff
diff --git a/src/custom_test_attr.rs b/src/custom_test_attr.rs
new file mode 100644
--- /dev/null
+++ b/src/custom_test_attr.rs
@@ -0,0 +1,4 @@
+#[my_framework::test]
+pub fn production() {
+    load().unwrap();
+}
diff --git a/src/raw_test_attr.rs b/src/raw_test_attr.rs
new file mode 100644
--- /dev/null
+++ b/src/raw_test_attr.rs
@@ -0,0 +1,4 @@
+#[r#test]
+fn actual_test() {
+    load().unwrap();
+}
```

Command:

```bash
/var/home/louranicas/.cache/ddf-t/target-integrity/release/deep-diff-forge \
  --stdin-patch --rank --json < /tmp/vesper-ddf-r2-probes.patch
```

Observed:

```text
src/custom_test_attr.rs  no evidence
src/raw_test_attr.rs     panic_path_added line 3 basis syntax_fragment
```

The same probe showed that direct `#[cfg(test)]` on a helper function or `impl` remains reported, although the hunk proves those items are test-configuration-only. That is conservative noise, not the blocker; the blocker is the custom attribute removing evidence from production code.

Fix: do not use the suffix of an unresolved attribute path as proof. Normalize raw identifiers and trust only semantics that are actually known: at minimum the built-in `test`/`r#test`, plus a deliberately enumerated set of framework attributes if that policy is acceptable. Unknown `foo::test` must retain panic evidence. Add a custom-path negative and a raw-built-in positive beside the existing `cfg_attr` tests.

### VD3R2-H-2 — Many sub-budget Rust fragments exceed the release timeout

The new benchmark covers one 4 MiB line. It does not cover aggregate parser work across many hunks. A 64,109,679-byte patch containing 1,000 Rust files, each with one approximately 64,002-byte expression line, was still running after a 15-second outer timeout on the supplied release binary. The input is below the 64 MiB patch cap and has far fewer than the one-million-line cap.

Reproduction:

```bash
yes 'a+' | tr -d '\n' | head -c 64000 \
  > /tmp/vesper-ddf-r2-node-unit.txt
: > /tmp/vesper-ddf-r2-node-heavy.patch
for i in $(seq 1 1000); do
  printf 'diff --git a/src/n%s.rs b/src/n%s.rs\n' "$i" "$i" \
    >> /tmp/vesper-ddf-r2-node-heavy.patch
  printf 'new file mode 100644\n--- /dev/null\n+++ b/src/n%s.rs\n' "$i" \
    >> /tmp/vesper-ddf-r2-node-heavy.patch
  printf '@@ -0,0 +1 @@\n+' >> /tmp/vesper-ddf-r2-node-heavy.patch
  cat /tmp/vesper-ddf-r2-node-unit.txt >> /tmp/vesper-ddf-r2-node-heavy.patch
  printf 'a;\n' >> /tmp/vesper-ddf-r2-node-heavy.patch
done

ls -l /tmp/vesper-ddf-r2-node-heavy.patch
/usr/bin/time -f 'elapsed=%e maxrss_kb=%M exit=%x' \
  timeout 15s \
  /var/home/louranicas/.cache/ddf-t/target-integrity/release/deep-diff-forge \
  --stdin-patch --rank --json \
  < /tmp/vesper-ddf-r2-node-heavy.patch > /dev/null
```

Observed:

```text
size:       64,109,679 bytes
elapsed:    15.01 seconds
max RSS:    146,456 KiB
exit:       124 (timeout)
```

A second admitted hostile shape exposes memory amplification before the fragment byte check. One 60 MiB line of `}` completed in 1.77 seconds but reached 924,576 KiB RSS. `fragment_evidence` counts braces, joins the body, and constructs balancing wrappers before `scan_fragment` rejects the source as over 64 KiB.

```text
input:      62,914,671 bytes
elapsed:    1.77 seconds
max RSS:    924,576 KiB
exit:       0
```

For comparison, the original 60 MiB all-`a` input completed in 2.01 seconds at 371,992 KiB RSS on this release binary.

Fix: impose a deterministic aggregate fragment budget per review (bytes and/or nodes attempted), falling back to lexical evidence after it is exhausted. Reject over-64-KiB hunks before joining the body or allocating brace-balancing wrappers. Extend the benchmark with repeated just-under-budget, node-heavy fragments; a single long line does not exercise this path. The production gate should also assert a peak-memory bound for admitted inputs.

### VD3R2-M-1 — The span rule reports constructs that were already risky before the change

The implementation equates “an added row lies anywhere in this construct” with “the risky construct was added.” That is unsound for a diff signal. Changing only a `todo!` message reports `stub_added` even though both old and new sides are stubs. Changing only a message on an existing `assert!(true, ...)` likewise reports `trivial_assert`. The evidence is anchored to the changed message row, not the row that introduced the construct—because no row did.

Reproduction excerpt:

```diff
diff --git a/src/todo_message.rs b/src/todo_message.rs
--- a/src/todo_message.rs
+++ b/src/todo_message.rs
@@ -1,5 +1,5 @@
 fn later() {
     todo!(
-        "old message"
+        "new message"
     )
 }
diff --git a/tests/assert_message.rs b/tests/assert_message.rs
--- a/tests/assert_message.rs
+++ b/tests/assert_message.rs
@@ -1,6 +1,6 @@
 fn check() {
     assert!(
         true,
-        "old message"
+        "new message"
     );
 }
```

Observed with the supplied release binary:

```text
src/todo_message.rs     stub_added line 3 basis syntax_fragment
tests/assert_message.rs trivial_assert line 4 basis syntax_fragment
```

The new test `an_added_line_inside_a_construct_that_spans_lines_is_evidence` enshrines this problem: its `todo!(` and closing `)` are context, and it adds only the string `"later"`; the old side was already `todo!()`.

Fix: compare the old and new construct classifications. Emit an “added”/weakening signal only when the new construct has the risky property and the old corresponding construct did not. Anchor it to an added row that caused that semantic transition. Keep the intended `assert!(computed)` to `assert!(true)` regression, where the classification genuinely changes.

### VD3R2-L-1 — Empty Promise handlers with default-valued parameters evade the new rule

`empty_callback` finds the first `)` rather than balancing delimiters. A default expression containing a call therefore ends parameter scanning early, and a genuinely empty rejection handler is missed.

Reproduction:

```diff
diff --git a/web/callbacks.ts b/web/callbacks.ts
new file mode 100644
--- /dev/null
+++ b/web/callbacks.ts
@@ -0,0 +1,3 @@
+run().catch((error = fallback()) => {});
+run().catch(function (error = fallback()) {});
+run().catch(async (error = fallback()) => {});
```

Observed: no evidence on any of the three lines. Controls with nested destructuring and comments were correctly reported.

Fix: use the JavaScript/TypeScript parser, or at least a bounded balanced-delimiter scan that respects the already-blanked code view, rather than `find(')')`/`split_once(')')`.

## Verification and tests

- The supplied release binary reports the original false `cfg_attr` production `.unwrap()` and all the other round-one context controls correctly.
- The original Rust multiline assertion and next-line JavaScript `catch` now report correctly. The Python and TypeScript multiline cases remain quiet as declared.
- `deep-diff-forge eval` completed successfully with the committed figures: 39 cases, precision 40/41, recall 40/67, findings 33/33, ranking 12/15, 31 declared items.
- Inspection shows the substantive new syntax/graph/corpus tests exercise behavior that failed on `f2a366d`, as established by the round-one probes. The workload-list/checksum test also necessarily fails before the new workload exists. I did not mechanically transplant the tests onto `f2a366d`.
- I attempted the focused Cargo test with the build slot free. This environment has no `cc` linker, so Cargo stopped while linking dependency build scripts; that is an environment limitation, not a test failure. The prebuilt release binary was used for all behavioral reproductions.
- The changed eval floor remains exactly the measured result, so adding declared misses does not create immediate regression slack while the manifest totals remain fixed.

## Not examined

- I did not run the full Cargo suite, Clippy, fuzzers, or the benchmark harness because the local toolchain could not link.
- I did not exhaustively enumerate attribute-procedural-macro ecosystems, every JavaScript parameter grammar, or every combination of inner `cfg(test)` placement.
- I did not wait for the 1,000-fragment hostile input to finish after 15 seconds; forge-warden would already have terminated it at 10 seconds.
- I did not test cluster/daemon consumers, every eval case semantically, or peak RSS under an external cgroup limit.
- I did not modify repository files, refs, remotes, or worktree content other than this report.

STATUS: COMPLETE
