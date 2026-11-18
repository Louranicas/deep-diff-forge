# Independent adversarial review of `agent/integrity-signals` at `f6f99d2`

Date: 2026-11-14. Reviewer: an independent Claude agent that had not seen this code. Scope:
`d4dbc91..f6f99d2`. Read-only on the repository; this file is the only one added.

Binaries used (prebuilt, of the head): `~/.cache/ddf-t/target-integrity/release/deep-diff-forge`
(`B` below) and the `debug` one. All scratch files are under `/tmp/claude-ddf-indep-review/`.
`rustc` is 1.99.1 in `fedora-toolbox-44`, run three times on files of a few lines with the build
slot `free`; no `cargo` was run.

## Verdict: NO-GO

Claim 1 (evidence is withdrawn only on proof) is broken twice, each shown with `git apply` and a
compiled, panicking binary. Claim 4 is broken on time (27 s for a 2 MB patch) and on determinism
(the same patch scores differently under CPU contention). Claim 2 is broken by placement inside
a macro body. Claims 3 and the worker-count part of 4 held.

---

## Findings

### IR-H-1 (high). A proved test construct withdraws every panic path on its line, production ones included

`fragment_evidence` pushes `(line index, PanicPathAdded)` to `withdrawn` for a construct that is
`in_test`, and `hunk_matches` then sets `basis = None` for that *line*
(`Parsed::has(&parsed.withdrawn, at, signal)`, evidence.rs about line 1067). A production panic
path on the same line survives only if the parser names it as a `MethodCall` (it then lands in
`found`, which wins). The token rules find two kinds the parser never names: the function form
(`Result::unwrap(x)`) and a call inside a macro's arguments (`vec![x.unwrap()]`,
`println!("{}", x.unwrap())`). Those are withdrawn with the test construct. Round 7 fixed this
granularity for the hint (`test_likely`); the withdrawal still goes by line. Round 4 listed
"same-line item layouts" as not examined.

Patch `/tmp/claude-ddf-indep-review/ts/a6.patch`:

```
diff --git a/src/a6.rs b/src/a6.rs
new file mode 100644
index 0000000..1111111
--- /dev/null
+++ b/src/a6.rs
@@ -0,0 +1,3 @@
+#[cfg(test)]
+fn t() { a().unwrap(); } pub fn production() -> u32 { Result::unwrap(crate::b()) }
+pub fn production2() -> u32 { vec![crate::b().unwrap()][0] } #[cfg(test)] fn t2() { a().unwrap(); }
```

Command: `$B --stdin-patch --rank --json < a6.patch`

Observed: `{"path": "src/a6.rs", "status": "added", "score": 4, "signals": ["new_file"]}`

Expected: `panic_path_added` at lines 2 and 3 (score 7). Controls: the same production line
alone in a new file gives `panic_path_added`, basis `lexical` (`ts/a2.patch`, `ts/a5.patch`);
with a plain `b().unwrap()` in `production` on the shared line it is reported
(`ts/a4.patch`).

Live code: in `/tmp/claude-ddf-indep-review/rustc/p2` (a scratch Git repository) `git apply
--check` and `git apply` accept the patch; `src/main.rs` declares `mod a6;` and
`pub fn b() -> Result<u32, ()> { Err(()) }`; `rustc --edition 2024 -o pbin src/main.rs` builds
with no diagnostic (not a test build), and `./pbin a6` and `./pbin a6b` exit 101 with
`panicked at src/a6.rs:2:55` and `src/a6.rs:3:47`: `called Result::unwrap() on an Err value`.
`production` and `production2` are top-level functions that only share a line with the
`#[cfg(test)]` functions.

Same root cause, lower harm: `test_likely` is true for a line whose named call is in a test and
whose production call is inside a macro (`c2/case-hint_mixed_line.patch`:
`#[test] fn check() { a().unwrap(); } pub fn production() -> u32 { vec![b().unwrap()][0] }`
gives `"test_likely": true`).

Fix: withdraw by construct, not by line. A token-rule hit on a line may be withdrawn only when
everything on that row that is not whitespace or a comment lies inside a proved item (compare
the row's byte span with the proved items' byte ranges), which is what `test_rows` already
guarantees for interior rows. The same rule for the hint.

### IR-H-2 (high). The grammar's shebang rule hides a `/*` that rustc reads: `#[cfg(test)]` in a comment is taken for proof

tree-sitter-rust 0.24.2 reads the first line as a shebang when it matches
`/#![\r\f\t\v ]*([^\[\n].*)?\n/` (grammar.js:1650): `#!`, blanks, then any character but `[`.
rustc decides differently: after `#!` it skips whitespace *and comments*, and if the next token
is `[` the line is an inner attribute, not a shebang. So `#! /**/ [allow(unused)] /*` is one
ignored line for the parser, and for rustc an attribute followed by an opened block comment.
The tree has no error, the file is whole and new, the path is named once, the attribute is
exact and first: every stated condition holds, and the code is in the program. This is round
5's `/*` … `#[cfg(test)] // */` moved into the one line the two readers disagree about.

Patch `/tmp/claude-ddf-indep-review/ts/s2.patch`:

```
diff --git a/src/s2.rs b/src/s2.rs
new file mode 100644
index 0000000..1111111
--- /dev/null
+++ b/src/s2.rs
@@ -0,0 +1,5 @@
+#! /**/ [allow(unused)] /*
+#[cfg(test)] // */
+pub mod t {
+    pub fn production() -> u32 { crate::b().unwrap() }
+}
```

Command: `$B --stdin-patch --rank --json < s2.patch`

Observed: `{"path": "src/s2.rs", "status": "added", "score": 4, "signals": ["new_file"]}`

Expected: `panic_path_added` at line 4. Control (`ts/s2ctl.patch`, the file as rustc reads it,
`#![allow(unused)]` then the module): score 9, `check_suppressed` and `panic_path_added`.

Live code: same scratch repository; `git apply` accepts it; `mod s2;` in `main.rs`; the build
has no diagnostic; `./pbin s2` exits 101, `panicked at src/s2.rs:4:45`. For rustc line 1 is
`#![allow(unused)]` and `/*`, line 2 is comment text up to `*/`, and `pub mod t` has no
attribute.

Fix: no proof for a tree whose root holds a `shebang` node. More generally, give proof only when
every child of `source_file` is an item, an attribute or a comment: the grammar also accepts
expression statements at the top level, which rustc never does (see IR-L-1).

### IR-H-3 (high). A 2 MB patch ranks in 27 s; a 3 MB patch of harmless lines in 10.8 s (`--outline`: 21.6 s)

Two independent causes.

(a) Text that is all error recovery parses at about 13 µs per byte here (one hunk of 20 KB:
0.32 s; 50 KB: 0.68 s), so the 2 MiB review budget is worth about 27 s of it. The only other
bound is the clock (1 s + 5 ms per KiB for each parse, up to three wrappings and two sides for
each hunk).

Generator `/tmp/claude-ddf-indep-review/perf/gen4.py`; `python3 gen4.py 32 'a?b:' >
ternary32.patch` writes 32 new files `src/fN.rs`, each

```
@@ -0,0 +1,2 @@
+// todo
+a?b:a?b:a?b:…        (one line, 64,988 bytes)
```

2,083,298 bytes, 224 lines. `/usr/bin/time $B --stdin-patch --rank --json < ternary32.patch`:
**27.04 s** wall, 26.93 s user, 24 MB (load average 4 of 16 cores; 26.84 s on an earlier run).
Units `dyn A+` and `}{)(][` take 23.3 s and 23.6 s.

(b) `FragmentScan::readable` is a linear search of the unreadable ranges, called once for
every row: in `scan_side` for each wrapping, and again in the search for `parsed.unread`. Rows
times ranges is quadratic in the hunk. `python3 gen2.py 32 8300 22900 'm!{}' > slow.patch`
writes 32 new files, each `// todo`, 8,300 lines `m!{}` and 22,900 empty lines (3,062,946
bytes, 998,592 lines; nothing in it is evidence). Three runs of `--rank --json`: **10.89, 10.84,
10.82 s** (load 1.8). `--outline` on the same patch: **21.59 s** (the views scan every hunk a
second time for its labels). Scaling confirms the cause: 6,000 macros and no empty lines 1.85 s;
6,000 and 12,500: 5.2 s; 6,000 and 25,000: 8.6 s.

A third, smaller one: `opens_across_lines` runs `unclosed` to the end of the line for every
`assert!(` on it (`perf/a32.patch`, 32 lines of 8,191 nested `assert!(`: 4.19 s; under the
bound, on the same curve).

Expected by claim 4: under 5 s. The changelog names a consumer limit of 10 s.

Fix: (a) a budget in parser work, not bytes: stop a review's parsing after a fixed number of
tree-sitter progress callbacks (a count, so it is the same on every machine), or cut the byte
budget to what the slowest text allows; (b) sort and merge `unreadable` once and answer by
binary search or one sweep; compute a hunk's evidence once for rank and views; (c) one pass for
`unclosed`.

### IR-M-1 (medium). The same patch gives a different score under CPU load: the parse deadline is wall-clock time

`parse_within` cancels on `Instant::elapsed`. A hunk whose parse takes a few tenths of a second
on an idle machine passes the deadline on a busy or slower one, the parser's answer becomes
`None`, and what only the parser found is replaced by `syntax_unchecked`.

Patch `/tmp/claude-ddf-indep-review/perf/nd2.patch` (29,756 bytes): a new file `tests/n.rs`
with

```
+fn t() {
+    assert!(
+        true
+    );
+}
+a?b:a?b:…            (7,400 times on one line)
```

`$B --stdin-patch --rank --json < nd2.patch`, three runs alone:
`"score": 6, "signals": ["test_only", "new_file", "trivial_assert"]`.

Eight copies started together on one CPU
(`for i in 1..8: taskset -c 5 $B --stdin-patch --rank --json < nd2.patch > nd2-busy-$i.json &`):
`"score": 4, "signals": ["test_only", "new_file", "syntax_unchecked"]`.

Expected by claim 4: one output for one input. `nd.patch` (the filler alone) flips between
score 4 and score 5 the same way. An author can size the filler so the flip happens on the
judge's machine and not on their own.

Fix: the same as IR-H-3 (a): bound the parse by a count of work, and keep the clock only as a
failure that is reported as an error, never as a different answer.

### IR-M-2 (medium). Inside a macro body, a construct the token rules do not spell leaves no item and no mark

`parsed.unread` marks only what `opens_across_lines` knows: `assert…!(` or `#[cfg_attr(` with
the parenthesis on the opener's line. Everything else the parser alone finds is silent on an
unreadable row. Each of these is a hunk of an existing `src/x.rs` (one context line `// ctx`
before and after; files `c2/case-*.patch`, made by `c2/probe.py c2/rust1.txt`):

| Added lines inside `outer! { … }` | Observed | Outside a macro body |
| --- | --- | --- |
| `assert!` / `(true);` (two lines) | no evidence | `trivial_assert` |
| `assert` / `!(true);` | no evidence | `trivial_assert` |
| `todo` / `!();` | no evidence | `stub_added` |
| `let v = x.unwrap` / `();` | no evidence | `panic_path_added` |
| `let v = x.` / `unwrap();` | no evidence | `panic_path_added` |
| `#` / `[ignore]`, `#[` / `ignore]` | no evidence | `test_skipped` |
| `assert!{true};`, `assert![true];` (one line) | no evidence | `trivial_assert` |
| `todo![];`, `unimplemented!{};` (one line) | no evidence | `stub_added` |

Example (`c2/case-bang_newline_paren.patch`), `$B --stdin-patch --rank --json`: score 2, no
`evidence` member. The control `assert!(` / `true` / `);` in the same place gives
`syntax_unchecked` (`c2/case-ctl_macro_assert_multi.patch`). Expected by claim 2: an item or
`syntax_unchecked`. The same hunk with the parser switched off by the budget does carry the
mark, so reading the hunk says less than not reading it. `proptest! { … }` and any
`macro_rules!` body are such places. None of these is a `known_miss` in the manifest.

Fix: on an added row the parser could not read, if the row holds one of `PARSER_WORDS` in its
code view and no token rule reported that word's signal on the row, mark the row
`syntax_unchecked`.

### IR-M-3 (medium). A multi-line `#[cfg_attr(…, ignore)]` whose condition is changed is not evidence

The old-side pairing drops a construct when the old side has a construct at the same place with
the same signal. For a conditional skip the condition is the content.

Patch `/tmp/claude-ddf-indep-review/c2/cfgattr.patch`:

```
diff --git a/tests/slow.rs b/tests/slow.rs
--- a/tests/slow.rs
+++ b/tests/slow.rs
@@ -1,7 +1,7 @@
 #[test]
 #[cfg_attr(
-    miri,
+    all(),
     ignore
 )]
 fn slow() {
     assert_eq!(f(), 1);
```

Observed: `{"path": "tests/slow.rs", "status": "modified", "score": 1, "signals": ["test_only"]}`.
The test was skipped under Miri and is now skipped always. The same change on one line
(`c2/cfgattr1.patch`, `-#[cfg_attr(miri, ignore)]` / `+#[cfg_attr(all(), ignore)]`) gives
`test_skipped`, score 4. Expected: `test_skipped` in both.

Fix: for an attribute, "carried it before" must also compare the attribute's words (or its
text with whitespace removed); for a macro call, its arguments.

### IR-L-1 (low, latent). A frontmatter block does the same as IR-H-2 once rustc accepts it

`ts/c3.patch` creates

```
---x
;#[cfg(test)] /*
---
// */
mod t {
    pub fn production() -> u32 { super::b().unwrap() }
}
```

DDF: score 4, no `panic_path_added` (the grammar reads `---x;` as an expression statement at
the top level, with no error). rustc 1.99.1 lexes lines 1 to 3 as a frontmatter and refuses it
only by feature gate (`error[E0658]: frontmatters are experimental`), so today this needs
nightly or `cargo -Zscript`. The fix suggested under IR-H-2 (only items, attributes and comments
at the top level) closes it.

### IR-L-2 (low). False positives and undeclared respellings of the token rules

Reproduced with `c2/probe.py` on `c2/fp.txt`, `c2/rust1.txt`, `c2/multi.txt`; none is declared
in `MANIFEST.json`.

- False positives with the parser's basis: `self.expect(Token::LBrace)?;` (any hand-written
  parser) and `mock.expect().times(1)` are `panic_path_added`, `syntax_fragment`;
  `#[cfg_attr(feature = "serde", serde(skip, ignore, default))]` on a struct is `test_skipped`,
  `syntax_fragment` (`signal_of` accepts `ignore` anywhere after `cfg_attr`); Python
  `unittest.skipper(x)` is `test_skipped`.
- Misses: Rust `it.map(Result::unwrap)`, `it.map(Option::unwrap)`, `<Option<u32>>::unwrap(x)`,
  `Result::<u32, E>::unwrap(r)` (the changelog names the function form as read);
  `#[cfg(any())]` on a test. Python `raise unittest.SkipTest("no")`, `@mark.skip` after
  `from pytest import mark`, `self.assertTrue(True, "m")`, `assert(True)`. JS
  `test.skip.each([...])(…)`, `it.concurrent.skip(…)`, `expect( true ).toBe( true )`,
  `assert.ok( true )`, `expect(true).toBeTruthy()`, and a `catch (e) {` whose block holds only
  a comment line. Go `t.Skip ("x")`, `if nil != err {}`.

### IR-L-3 (low). A spaced `allow` attribute is not `check_suppressed`

Seen in the IR-H-2 control and then probed (`c2/allow.txt`): `#![allow(unused)]` is
`check_suppressed`; `#! [allow(unused)]`, `#! /**/ [allow(unused)]` and `# [allow(unused)]` give
nothing. The rule is older than this branch; the branch made the integrity tokens independent of
spacing and left this one exact.

---

## The six claims

1. **Withdrawn only on proof: broken.** IR-H-1 and IR-H-2, both compiled and run. No other
   signal is removed by the parser: by reading, `withdrawn` only ever holds `PanicPathAdded`.
2. **Silence is not selectable: broken by placement** (IR-M-2). It held for size and order: a
   60-file, 4 MB review gave 32 files `trivial_assert` and 28 `syntax_unchecked`
   (`det/big.patch`); over the node budget is marked too.
3. **`test_likely` is a hint only: held.** By reading, no score, signal or sort reads it; the
   same hunk under `#[test]` and under `#[inline]` scores 5 and 5. The hint itself can be wrong
   by line (IR-H-1, last paragraph).
4. **Determinism and bounds: broken.** Time: IR-H-3 (27 s, 10.8 s, 21.6 s). Same output:
   IR-M-1. Held: `--cluster --parallel 1, 2, 8, 16` identical and equal to `--rank` and
   `--outline` scores on a review over the budget (`det/`); under 100 MB in every run; no panic in 1,500
   seeded token-soup patches on the debug binary, three modes each (`fuzz/soup.py 1 1500`).
5. **Token rules: hold for the README table as written**, with the false positives and
   undeclared misses of IR-L-2.
6. **The tests: several do not test the claim.** See below.

## Weak tests

- `fuzz/fuzz_targets/fragment_scan.rs`, invariant 2: in a new file any `panic_path_added` may
  go (`may_go = … || (PanicPathAdded && new_file)`). The one automated guard of claim 1 cannot
  see a wrong withdrawal; IR-H-1 and IR-H-2 both satisfy it. It also drops `\r` and stops at
  48 KiB, so it never meets a budget.
- `a_panic_path_under_anything_less_than_that_proof_stands`: every production call is a plain
  method call on a line of its own. A withdrawal by row, by line or by file passes it. No test
  has a token-only hit beside a proved construct.
- `a_hunk_with_very_many_items_is_read_in_one_pass`: no deadline (it relies on not finishing),
  and neither of its hunks is a parsed hunk with many unreadable ranges, so the quadratic of
  IR-H-3 (b) is not exercised. The bench workload `rank_many_fragments` uses the fast `a+a+…`
  text only.
- `a_path_named_by_two_sections_is_no_whole_file` returns early when `parse` refuses two
  sections of one path, and `a_hunk_that_only_says_it_starts_the_file_proves_no_test_code`
  guards its third case with `if let Ok`. Both run today; after the rebase onto the 0.8.4 parser
  the first becomes empty with no failure.
- The cluster path: `cluster.rs` fixtures are `-a` / `+b` lines, and
  `cluster_scores_match_direct_scoring` compares with the single-file `score_file`. Putting
  `RankedFile::scored(file)` back in `run_risk_cluster` passes every committed test. The claim
  "the same for any worker count" held when I ran it by hand, and nothing committed checks it
  with a spent budget or a path named twice.
- Nothing tests that evidence is independent of the clock; `an_exhausted_deadline_cancels_the_parse`
  tests only that cancelling works.

## Suspected, not reproduced

- Other places where tree-sitter-rust 0.24.2 and rustc lex differently with no error node. I
  tried nested block comments (`/*/* */`, `/*!/* */`), a bare CR in a line comment, and raw C
  strings; all three agree with rustc. I did not run a differential fuzz of the two lexers,
  which is the only way to close this class.
- A patch that sets `--cfg test` for a normal build (`.cargo/config.toml` rustflags, a
  `build.rs` printing `cargo:rustc-cfg=test`) together with a whole new file. The code comment
  on `opens_test_code` declares it; the README sentence "withdrawn only where the patch proves
  the code is compiled for tests alone" does not. Not built.
- `hunks` pages: each page request plans and scores the whole review again, so a paged read of
  `slow.patch` should cost the 10 s once per page. Not measured.

## Not looked at

- The patch parser's agreement with Git beyond eight second-section spellings (`//`, quoted
  octal, `./`, traditional headers, rename from, copy from: the plan's path count held, or Git
  refused the patch). The G1/G2 work for 0.8.4 is not on this branch.
- The daemon, RPC and MCP entry points (only the CLI was run), the TUI, `bench`, Windows and
  macOS, case-folding file systems.
- `lexical.rs` (`code_view`) beyond its use here; the Python, JS and Go rules beyond the probes
  listed; `eval` scoring logic (I ran `eval`; it passes with its declared items).
- The full test suite, Clippy, Miri and the fuzzers were not run (no `cargo`).
- Memory near the 64 MiB input limit, and inputs made of very many small hunks.
