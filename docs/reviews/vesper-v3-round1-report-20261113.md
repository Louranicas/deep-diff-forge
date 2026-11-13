# Vesper V3 round 1 review — integrity signals and fragment parser

Reviewed: `d4dbc91..f2a366d` (`f2a366dc32f3ffa82ec41151eaf88e49898053e9`) on `agent/integrity-signals`

## Verdict: NO-GO

Two failures block this from the production consumer described in the request. A valid false `cfg_attr` can make a production panic path disappear, and an admitted patch below the stated 64 MiB input cap takes 17.26 seconds, so forge-warden's 10-second limit will terminate it. Multiline versions of constructs in the signal table also evade detection.

I used the supplied prebuilt binary. I did not run Cargo because `hive slot show` reported an occupied lane.

## Findings

### VD3-H-1 — A false `cfg_attr` suppresses a production panic path

The Rust fragment parser flattens all identifiers in a preceding attribute and treats the function as a test when the final identifier is `test` or `rstest`. It does not preserve or evaluate `cfg_attr` structure. Therefore `#[cfg_attr(any(), test)]` marks the following function as test code even though `any()` with no operands is false and Rust does not apply `#[test]`. `confirm_by_parse` then withdraws the lexical `panic_path_added` item from production code.

This is attacker-controlled hunk shaping of exactly the withdrawal permitted by claim 2. The converse is visible too: `#[cfg_attr(any(), ignore)]` emits `test_skipped` even though its condition is false and the test still runs.

Reproduction (`/tmp/vesper-ddf-context-min.patch`):

```diff
diff --git a/src/false_cfg_attr.rs b/src/false_cfg_attr.rs
new file mode 100644
--- /dev/null
+++ b/src/false_cfg_attr.rs
@@ -0,0 +1,4 @@
+#[cfg_attr(any(), test)]
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
diff --git a/tests/false_ignore.rs b/tests/false_ignore.rs
new file mode 100644
--- /dev/null
+++ b/tests/false_ignore.rs
@@ -0,0 +1,3 @@
+#[cfg_attr(any(), ignore)]
+#[test]
+fn still_runs() { assert!(compute()); }
```

Command:

```bash
/var/home/louranicas/.cache/ddf-t/target-integrity/debug/deep-diff-forge \
  --stdin-patch --rank --json < /tmp/vesper-ddf-context-min.patch
```

Observed relevant result:

```text
src/control.rs          panic_path_added line 2 basis syntax_fragment
src/false_cfg_attr.rs   no evidence
tests/false_ignore.rs   test_skipped line 1 basis syntax_fragment
```

I separately tested the other requested context-shaping cases. A closed `#[cfg(test)] mod`, comment/string text, a neighbouring real `#[test]`, and a macro containing test-looking text did not suppress a later production `.unwrap()`.

Fix: preserve the attribute AST and recognize only an actual direct `#[test]`/`#[rstest]` attribute for withdrawal. Do not treat identifiers nested inside `cfg_attr` as active attributes unless the condition can be proved active under the relevant configuration. If activity is unknown, retain the safety evidence. Apply the same condition-aware rule to `ignore`, and add false-condition regression tests for both directions.

### VD3-H-2 — An admitted 60 MiB line exceeds forge-warden's 10-second limit

A patch under the stated 64 MiB production input limit, containing one very long added line, completed in 17.26 seconds. Forge-warden's 10-second wrapper will kill this input. The run returned normally only because I gave it a 20-second outer timeout.

Reproduction:

```bash
{
  printf 'diff --git a/src/long.rs b/src/long.rs\n'
  printf 'new file mode 100644\n--- /dev/null\n+++ b/src/long.rs\n'
  printf '@@ -0,0 +1 @@\n+'
  head -c 62914560 /dev/zero | tr '\0' a
  printf '\n'
} > /tmp/vesper-ddf-longline.patch

ls -l /tmp/vesper-ddf-longline.patch
/usr/bin/time -f 'elapsed=%e maxrss_kb=%M exit=%x' \
  timeout 20s \
  /var/home/louranicas/.cache/ddf-t/target-integrity/debug/deep-diff-forge \
  --stdin-patch --rank --json \
  < /tmp/vesper-ddf-longline.patch \
  > /tmp/vesper-ddf-longline.json
```

Observed:

```text
size:       62,914,668 bytes
elapsed:    17.26 seconds
max RSS:    375,804 KiB
exit:       0
```

The build slot was occupied during this run, so this is not an idle-machine microbenchmark. That does not rescue the production claim: the consumer has the same fixed wall-clock limit and may also run on a loaded host, while this result exceeds it by 72.6%.

Fix: bound work by admitted bytes as well as parser nodes. Avoid rescanning a giant line independently for every lexical rule/signal, and add an end-to-end worst-case benchmark at the actual production cap under the production timeout. If the implementation cannot meet that bound, lower and enforce the admitted input cap coherently at the consumer and CLI boundary.

### VD3-M-1 — Added tokens inside existing multiline constructs evade the advertised signals

The scanner keys lexical rules to the added line, while the Rust fragment confirmation accepts a construct only when its start row is added. Consequently, changing only the argument/body inside an existing multiline construct is missed. This affects the exact constructs named in the signal table, not merely semantic analogues. A valid JavaScript `catch` with its brace on the next line is also missed because the hunk rule requires `catch` and `{` on one line.

Reproduction (`/tmp/vesper-ddf-dodges.patch`):

```diff
diff --git a/tests/multiline.rs b/tests/multiline.rs
--- a/tests/multiline.rs
+++ b/tests/multiline.rs
@@ -1,4 +1,4 @@
 fn checks() {
     assert!(
-        computes_answer()
+        true
     );
diff --git a/tests/test_multiline.py b/tests/test_multiline.py
--- a/tests/test_multiline.py
+++ b/tests/test_multiline.py
@@ -1,4 +1,4 @@
 def test_answer():
     assert (
-        computes_answer()
+        True
     )
diff --git a/web/multiline.test.ts b/web/multiline.test.ts
--- a/web/multiline.test.ts
+++ b/web/multiline.test.ts
@@ -1,5 +1,5 @@
 test("answer", () => {
   expect(
-    computesAnswer()
+    true
   ).toBe(true);
 });
diff --git a/web/a.ts b/web/a.ts
--- a/web/a.ts
+++ b/web/a.ts
@@ -1,2 +1,3 @@
 try { work(); }
-catch (e) { report(e); }
+catch
+{ }
```

Command:

```bash
/var/home/louranicas/.cache/ddf-t/target-integrity/debug/deep-diff-forge \
  --stdin-patch --rank --json < /tmp/vesper-ddf-dodges.patch
```

Observed: all four files have no evidence. Controls in the same probe did detect `core::todo!()` as `stub_added` with `syntax_fragment` basis and a two-line Python `except (ValueError):` / `pass` as `error_swallowed` with `hunk` basis.

Fix: consider a syntax construct changed when any semantically relevant row in its span is added, then anchor the evidence to the added token/argument row. For languages without syntax confirmation, add bounded multiline state for the table's assertion forms and for a `catch` followed by its opening brace/body. Add one regression for each layout above.

### VD3-L-1 — Common equivalent spellings provide additional undeclared dodges

The following variants are not in the request's declared-evasion list and produced no evidence:

```text
test_skipped:       Go `tt.Skip()` where the `*testing.T` receiver is named `tt`;
                    TypeScript `test.skipIf(condition)(...)`
trivial_assert:     Python `assert 1`, `assert not False`, `self.assertEqual(1, 1)`
stub_added:         a Python function whose body is `...`
error_swallowed:    JavaScript `run().catch(() => {})`
panic_path_added:   Rust `Result::unwrap(read())`
```

Reproduction is `/tmp/vesper-ddf-variants.patch`, run with the same supplied binary command. Its relevant files (`go/other_receiver_test.go`, `web/skipif.test.ts`, `tests/constants.py`, `a.py`, `web/promise.ts`, and `src/ufcs.rs`) all returned no evidence.

These are broader semantic forms than the README's deliberately literal table, so I rate this low rather than treating each as a broken table promise. They nevertheless allow the same review-risk changes without using any already-declared miss.

Fix: either declare these as intentional token-rule limits in the evasion corpus or cover the high-confidence forms. In particular, bind Go's `*testing.T` parameter name before matching `Skip`, recognize Python ellipsis bodies, and use syntax to recognize constant-truth assertions and empty callback handlers. Keep the Rust panic description limited to method calls if UFCS is intentionally out of scope.

## Claim assessment

| # | Assessment | Evidence |
|---:|---|---|
| 1 | **Broken** | VD3-M-1 misses table-listed multiline assertions and an empty `catch`. Comment/string-only probes for all five signals did remain quiet. |
| 2 | **Broken** | VD3-H-1 makes the parser withdraw a production panic path under a false `cfg_attr`. The requested closed-module, neighbour, comment/string, and macro controls behaved correctly. |
| 3 | **Held in tested cases** | An over-64-KiB Rust hunk and an item inside a macro retained `panic_path_added` with `lexical` basis; a readable control used `syntax_fragment`. Invalid UTF-8 did not panic and still found the real construct. |
| 4 | **Held in tested cases** | Three runs of the same context patch were byte-identical (`sha256 6b61329c70d8255b8e234c2e876c1ed3a3225516f48a60e16ca6701cb0861918`). CRLF changed content addresses as expected, but paths, scores, signals, lines, and bases matched LF. Inspection confirmed count-based parser budgets and deterministic ordering. |
| 5 | **Broken** | VD3-H-2 takes 17.26 seconds for a valid patch below the stated production cap. |
| 6 | **Held in tested cases** | Every emitted item inspected pointed to the added construct's new-side line. VD3-M-1 concerns missing items, not mispointed emitted items. |

## Other review work

- Scoring inspection found each evidence signal added at most once per file. Test paths halve the weight with ceiling, except `test_removed`, `test_skipped`, and `trivial_assert`, which correctly retain full weight as test-weakening signals.
- The `rank.v0` changes I inspected are additive members/enum values (`basis` and the five new signals); I found no document-name or structural compatibility break.
- A real-history probe, `git diff --binary d4dbc91~40..d4dbc91`, produced a 1.4 MiB patch and completed in 1.08 seconds at 18,276 KiB RSS. I sampled the 123 new-signal items (all `panic_path_added`); sampled calls were real `unwrap`/`expect` sites. Macro-contained examples remained lexical as documented. I found no definite real-code false positive in that sample.
- A patch containing only the five signal spellings in Rust, Python, JavaScript, and Go comments/strings produced no evidence.

## Not examined

- I did not run the Cargo test suite, Clippy, fuzzers, or benchmarks because the build lane was occupied, per the round rules.
- I did not exhaustively judge all 123 real-history items, every language grammar edge, every 64 MiB hostile-input shape, cluster/daemon surfaces, or non-`rank.v0` consumers.
- I did not test every parser budget boundary at exactly 64 KiB and 50,000 nodes, every invalid UTF-8 position, or every tree-sitter recovery ambiguity.
- I did not modify repository files, refs, remotes, or worktree content other than this report.

STATUS: COMPLETE
