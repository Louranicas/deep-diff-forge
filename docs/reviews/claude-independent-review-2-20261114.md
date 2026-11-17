# Second independent review of the integrity signals: commit `bebebef` (on `f6f99d2`)

Date: 2026-11-14. Reviewer: an independent Claude agent that took no part in the work. Scope:
`git diff f6f99d2..bebebef`. Read-only on the repository; this file is the only one written.

## Binaries, and which output came from which

The binary named in the brief (`~/.cache/ddf-t/target-integrity/release/deep-diff-forge`) was
rebuilt from later, uncommitted work while this review ran (the coordinator said so at about
11:00 AEDT). Every reproduction was therefore run again on frozen copies, and **every output
quoted below is from a frozen copy**:

| Name here | File | SHA-256 | What it is |
|---|---|---|---|
| `NEW` | `/tmp/claude-ddf-indep-review-2/bin/ddf-new` | `67a147f0…c02acea` | commit `bebebef` (built 10:18) |
| `FIX2` | `/tmp/claude-ddf-indep-review-2/bin/ddf-fix2` | `67bfb971…d124a59` | uncommitted work after `bebebef` (built 10:57) |

Outputs: `/tmp/claude-ddf-indep-review-2/out-new/` and `out-fix2/` (`rerun.sh new|fix2` makes
them). Source of `bebebef` was read from `git archive bebebef` (`src-bebebef/`), not from the
working tree. `rustc` is 1.99.1 in `fedora-toolbox-44`, run on single files of a few lines;
no `cargo` was run. Each high finding was applied with `git apply` in a scratch repository,
built as an ordinary (non-test) binary and run (`case.sh NAME EDITION ENTRY`; logs
`out-new/case-NAME.log`).

## Verdict: NO-GO

Claim 1 (a panic path is withdrawn only for code proved to be test code) is broken in four
independent ways, each compiled and run to a panic. They share one root: the proof rests on
three readers of the file agreeing with the compiler (tree-sitter-rust's scanner, the code
view of `lexical.rs`, and an assumed edition), and "the tree has no error and its top level is
items" does not establish that. Claim 3 is broken on time (a 257 KB patch ranks in 15 s, its
outline takes 31 s). Claim 2 is broken by spelling and by the code view. Claim 4 held.
The uncommitted work after `bebebef` (`FIX2`) closes one medium finding, leaves all four high
ones open and makes the time worse (one 65 KB hunk: 14.8 s).

---

## New findings in `bebebef`

### IR2-H-1 (high). `//*` inside a block comment: tree-sitter does not count the nested opener, rustc does

`tree-sitter-rust-0.24.2/src/scanner.c`, `process_left_forward_slash`: after a `/` inside a
block comment, any character but `*` returns the state to `Continuing`, a second `/` included.
So in `//*` the second slash and the star are not read as `/*`. rustc reads them as a nested
opener (`rustc_lexer`: `'/' if self.first() == '*'`). For the grammar `/* //* */` is a whole
comment; for rustc it is still open at depth 1.

Patch `/tmp/claude-ddf-indep-review-2/h/n2.patch` (creates `src/n2.rs`):

```
/* //* */ #[cfg(test)] // */
pub mod t {
    pub fn production() -> u32 { crate::b().unwrap() }
}
```

Command: `NEW --stdin-patch --rank --json < h/n2.patch`

Observed: `src/n2.rs score 4 ["new_file"]`, no evidence.

Expected: `panic_path_added` at line 3. For rustc the whole first line is one comment and
`pub mod t` has no attribute: `git apply` accepts the patch, `rustc --edition 2024` builds it
with no diagnostic, and the binary exits 101, `panicked at src/n2.rs:3:45` (`out-new/case-n2.log`).
The tree has no error and its top level is a comment, an attribute, a comment and a module,
so every stated condition holds. Control `h/n2ctl.patch` (`/* /* */ #[cfg(test)] // */`, which
both readers nest alike): score 7, `panic_path_added` line 3.

This is round 5's `/*` … `#[cfg(test)] // */` and the first review's IR-H-2 again, in a third
place where the two lexers part. The top-level check does not help: nothing here is at the top
level but items and comments.

### IR2-H-2 (high). The scanner reads a character by its low byte: `Ī` is `*` and `į` is `/` inside a block comment

Same file, `process_block_comment`: `char first = (char)lexer->lookahead;` and the three state
functions take `char current`. A code point is cut to eight bits, so U+012A (`Ī`) compares
equal to `*` (0x2A) and U+012F (`į`) to `/` (0x2F). Inside a block comment `įĪ` opens a nested
comment and `Īį` closes one, for the grammar only.

(a) Hiding code from the grammar. Patch `h/n3.patch` (creates `src/n3.rs`):

```
macro_rules! live { (#[cfg(test)] $($rest:tt)*) => { $($rest)* }; }
/* įĪ */ live! { // */
#[cfg(test)]
pub mod t {
    pub fn production() -> u32 { crate::b().unwrap() }
}
/* įĪ */ } // */
```

`NEW --stdin-patch --rank --json < h/n3.patch`: `src/n3.rs score 4 ["new_file"]`, no evidence.
For the grammar lines 2 and 7 are each one comment (`įĪ` took the depth to 2), and the module
stands at the top level under `#[cfg(test)]`. For rustc `/* įĪ */` is a comment and
`live! {` … `}` is a macro call around the module, which drops the `cfg`. Built and run:
exit 101, `panicked at src/n3.rs:5:45` (`out-new/case-n3.log`). Control `h/n3ctl.patch` (`xy`
for `įĪ`): score 7, `panic_path_added` line 5.

(b) Showing the grammar an attribute that is comment text. Patch `h/n3b.patch`:

```
/* /* Īį */ #[cfg(test)] // */
pub mod t {
    pub fn production() -> u32 { crate::b().unwrap() }
}
```

`NEW`: `src/n3b.rs score 4 ["new_file"]`. Built and run: exit 101, `panicked at
src/n3b.rs:3:45`. Control `h/n3bctl.patch` (`xy` for `Īį`): score 7, `panic_path_added`.

### IR2-H-3 (high). The code view does not nest block comments, so "all the code of the line lies in the item" is wrong (IR-H-1 open in a variant)

The fix for IR-H-1 measures a line's code by `code_extent` on the lexer's code view.
`lexical.rs` (`Scan::step`) ends a block comment at the first `*/`. rustc and the grammar nest.
After `/* /* */` the code view is in code while the compiler is still in the comment, and a
`"` or a `//` there blanks the rest of the line in the view. The production function after it
is then outside the "code extent", the line counts as wholly inside the test item, the token
rule's item is withdrawn, and the parser's own item for the production call is dropped as well
(`fragment_evidence`: `if signal == S::PanicPathAdded && test_lines.binary_search(&index).is_ok() { continue; }`).
That last part is new in this commit: at `f6f99d2` a call the parser named on such a line was
kept (by reading: `found` won over `withdrawn`; no binary of `f6f99d2` was at hand to run).

Patch `h/n1b.patch` (creates `src/n1b.rs`):

```
#[cfg(test)]
fn t() { a().unwrap(); /* /* */ " */ } pub fn production() -> u32 { crate::b().unwrap() }
/* /* */ } */
```

`NEW --stdin-patch --rank --json < h/n1b.patch`: `src/n1b.rs score 4 ["new_file"]`, no
evidence. Expected `panic_path_added` at line 2: `production` is an ordinary top-level
function with a plain method call. Built (`--edition 2024`) and run: exit 101, `panicked at
src/n1b.rs:2:80` (`out-new/case-n1b.log`). Line 3 is a comment for the compiler; in the code
view it holds a `}`, which balances the braces so that the hunk is read unwrapped (without it
the hunk is wrapped, `new.offset` is 1 and nothing is withdrawn: `h/n1.patch`, score 7).

`h/n1c.patch` is the same with `//` for the `"` and the function form
(`Result::unwrap(crate::b())`): score 4, no evidence; exit 101, `panicked at src/n1c.rs:2:70`.
Control `h/n1ctl.patch` (a plain `/* */` in `t`): score 7, `panic_path_added` line 2.

The same blind spot of the view breaks claim 2 (IR2-M-2, last two rows) and gives the hint
wrongly (IR2-L-1).

### IR2-H-4 (high). `cr"\"` is a raw C string only from edition 2021: in a 2015 or 2018 crate the grammar and the code view read another file than rustc

The grammar's scanner (`scan_raw_string_start`: `b` or `c`, then `r`) and `rust_raw_hashes` in
`lexical.rs` both read `cr"…"` as a raw string. rustc does so only in edition 2021 and later.
In editions 2015 and 2018 `cr"\"` is the identifier `cr` and the start of an ordinary string
in which `\"` is an escaped quote, so the string runs on to the next `"`. The edition is in
`Cargo.toml` (default 2015 when the key is absent; `rustc` without `--edition` is 2015), not
in the hunk.

Patch `h/n4.patch` (creates `src/n4.rs`):

```
macro_rules! live { ($a:ident $b:literal $($rest:tt)*) => { $($rest)* }; }
live! { cr"\" }
#[cfg(test)] pub mod t { // "
    pub fn production() -> u32 { crate::b().unwrap() }
}
```

`NEW --stdin-patch --rank --json < h/n4.patch`: `src/n4.rs score 4 ["new_file"]`, no evidence.
For the grammar: a macro call, then a module under `#[cfg(test)]` with a line comment. For
rustc 2018: `live! { cr "\" }⏎#[cfg(test)] pub mod t { // " pub fn production() … }`, one macro
call whose string swallows the attribute and the module's opening; the macro emits the
function. `rustc --edition 2018`: no diagnostic, exit 101, `panicked at src/n4.rs:4:45`; the
same with no `--edition` flag (2015). With `--edition 2021` the file is what the grammar reads
(`error: no rules expected cr"\"`), which shows the two readings (`out-new/case-n4.log`).

### IR2-H-5 (high). A 257 KB patch ranks in 15 s and its outline takes 31 s (IR-H-3 open in a variant)

Text `#!#!#!…` after a first line is slow for the grammar when it is parsed unwrapped, and the
cost grows with the square of its length: one hunk of 8 KB 0.24 s, 16 KB 0.85 to 1.07 s. The
parse clock (1 s and 5 ms a KiB) cuts each parse at about 1.1 to 1.3 s, so the cost of a review
is the number of hunks times about a second, and the byte budget does not bound it: 256 KiB
buys sixteen 16 KB hunks.

Generator `/tmp/claude-ddf-indep-review-2/perf/gen_hb.py`; `python3 gen_hb.py 16 16000 >
hb16x16k.patch` writes 16 new files `src/fN.rs`, each

```
@@ -0,0 +1,2 @@
+// todo
+#!#!#!…        (one line, 15,990 bytes)
```

257,666 bytes. `/usr/bin/time NEW --stdin-patch --rank --json < perf/hb16x16k.patch`:
**14.50 s** wall, 14.33 s user, 9 MB (load 15 of 16 cores), **15.43 s** on a quiet host
(load 3). `--outline`: **30.90 s**. `hb15x17k.patch` (256,560 bytes): 16.04 s and 30.90 s.
`hb32x8k.patch` (259,362 bytes): 7.95 s and 15.25 s. Output: every file `score 4
["new_file"]`, no `syntax_unchecked`, since the wrapped reading answers. Controls: the same
16 KB hunk with an opening brace before the line, so that only the wrapped reading is tried
(`hb1_openbrace.patch`): 0.00 s; one hunk alone (`hb1_balanced.patch`): 0.85 s
(`out-new/perf.txt`).

Also, the views still read every hunk twice (rank, then labels): the first review's
`perf/ternary32.patch` (2 MB) ranks in 3.7 s as the request says, and its `--outline` takes
**7.3 to 7.5 s** (`--hunks` 4.8 s). So "one review costs the parser at most 256 KiB" does not
hold for the views.

Expected by claim 3 and the brief: under 5 s for a patch under 4 MiB.

### IR2-M-1 (medium). IR-M-3 is closed as written and open beside it: a condition changed inside a list, or by a string, is still "the same attribute"

The old side is now compared by the attribute's `words`, and `attribute_words` keeps only the
identifiers that stand directly in the argument list. What is nested (`all(miri)`) and what is
a string (`target_os = "plan9"`) is not in the words.

Cases `/tmp/claude-ddf-indep-review-2/c/m3.txt`, made into patches by `probe.py c/m3.txt
tests/slow.rs` (`c/case-m3_*.patch`; each is `#[cfg_attr(` / condition / `ignore` / `)]` with
only the condition line changed). `NEW --stdin-patch --rank --json`:

| Condition before → after | Observed |
|---|---|
| `miri,` → `all(),` (the first review's case) | `test_skipped` (closed) |
| `all(miri),` → `all(),` | score 1, no evidence |
| `not(any()),` → `not(miri),` | score 1, no evidence |
| `target_os = "plan9",` → `target_os = "linux",` | score 1, no evidence |
| `feature = "never-enabled",` → `feature = "default",` | score 1, no evidence |
| `any(miri),` → `any(miri, unix, windows),` | score 1, no evidence |

Expected: `test_skipped` in each (the same change on one line is reported:
`case-m3_oneline_nested.patch`, score 4). Output: `out-new/m3.txt`. Fix: compare the
attribute's text with whitespace and comments removed.

### IR2-M-2 (medium). Claim 2: words that leave neither an item nor `syntax_unchecked`

Cases `c/claim2.txt` (patches `c/case-*.patch`, each a hunk of an existing `src/x.rs`), output
`out-new/claim2.txt`. All compiled and behave as the plain spelling
(`rustc/claim2/raw.rs`, `ign.rs`):

| Added line(s) | `NEW` | Truth |
|---|---|---|
| `let v = x.r#unwrap();` | no evidence | panics (`r#unwrap` is `unwrap`) |
| `let v = x.r#expect("set");` | no evidence | panics |
| `let v = it.map(Option::r#unwrap);` | no evidence | panics |
| `#[r#ignore]` / `fn t() {}` | no evidence | test reported `ignored` |
| `let f = Option::<u32>::unwrap;` | no evidence | panics when called |
| `#[cfg_attr(all(), cfg_attr(all(), ignore))]` | no evidence | test reported `ignored` |
| `outer! {` / `/* /* */ // */ let v = x.unwrap();` / `}` | no evidence | code after the comment |
| `/* /* */ // */ let v = Result::unwrap(x);` (plain code) | no evidence | panics |

The first four are on rows the parser read: `unread_words` looks only at rows it did not, and
the construct it names (`r#unwrap`) has no signal. The last two are IR2-H-3's blind spot.
`FIX2` reports the first four; the other four are as here.

### IR2-L-1 (low). `test_likely` is true on a line that holds a production call

`h/n1.patch`: `NEW` gives `panic_path_added` line 2 with `"test_likely": true`; the line holds
`pub fn production() -> u32 { crate::b().unwrap() }` after the test function (same cause as
IR2-H-3; the hint changes no score).

---

## The first review's findings

| Finding | Status in `bebebef` (`NEW`) |
|---|---|
| IR-H-1 | Closed as written (`ts/a6.patch`: lines 2 and 3 reported). **Open in a variant: IR2-H-3.** Variants that held: production before the attribute on the line, on the closing row of a multi-line test, after non-ASCII text, a tab, char literals holding quotes, a raw string, between two test items (`h/v1`, `v2`, `v3`, `v7`, `v8`, `v10`, `v11`) |
| IR-H-2 | Closed as written (`ts/s2.patch`: line 4 reported). **Open in variants: IR2-H-1, IR2-H-2, IR2-H-4.** Variants that held: `#!` then a newline then `[`, a byte order mark before the shebang (`h/v4`, `v5`) |
| IR-H-3 | (a) `ternary32.patch` 3.7 s for `--rank`, 7.4 s for `--outline`; (b) `slow.patch` 0.9 s, closed; `a32.patch` 0.1 s, closed. **Open in a variant: IR2-H-5** |
| IR-M-1 | Not fixed, as the request says. Not re-run |
| IR-M-2 | Closed as written (all eight rows now `syntax_unchecked`). Other words: IR2-M-2 |
| IR-M-3 | Closed as written. **Open in variants: IR2-M-1** |
| IR-L-1 | Closed (`ts/c3.patch`: line 6 reported) |
| IR-L-2 | The listed false positives and misses are fixed or declared in the corpus case |
| IR-L-3 | Closed for the spaced forms (`#! [allow(`, `# [allow(`). `#! /**/ [allow(unused)]` still gives nothing (`c2/case-allow_comment.patch`, score 2) |

Full re-run: `out-new/first-review-rerun.txt`.

## The four claims

1. **Withdrawal only on proof: broken** (IR2-H-1, IR2-H-2, IR2-H-3, IR2-H-4), each built and
   run. "Nothing else is withdrawn by the parser" held by reading (`withdrawn` holds only
   `PanicPathAdded`).
2. **Every parser word accounted for: broken** (IR2-M-2).
3. **At most 256 KiB for the parser; the same evidence everywhere: broken on cost** (IR2-H-5:
   15 s; the views read twice). Held: `--rank` and `--cluster --parallel 1, 8, 16` gave the
   same score and signals for all 60 files of the first review's over-budget `det/big.patch`.
   The daemon's views were not run.
4. **Corpus honesty: held.** `NEW eval --corpus src-bebebef/fixtures/eval`: exit 0, 43 cases,
   precision 63/69, recall 63/104, as the request says. `eval.rs` fails a declared miss that
   is found and a declared false positive that is not reported (read, lines 393 to 460).

## Cost of the fix

`hist.py bin/ddf-new 150`: each of the last 150 non-merge commits ranked as a patch (291 Rust
files). `panic_path_added`: 315 with the parser's basis, 19 lexical; all 19 lexical ones are
real `.unwrap()` calls inside macro arguments or on rows the parser could not read, none comes
from the new `::unwrap(` form. `syntax_unchecked`: 2 files, both a multi-line `assert!(` /
`assert_eq!(` (`hist/new.txt`). So the new rules cost little on this history. Not compared
with `f6f99d2` (no binary of it).

## The tests the commit adds

None was run (no `cargo`); all of this is by reading.

- `fuzz_targets/fragment_scan.rs`, `may_withdraw`: its oracle is the same code view and the
  same scanner spans the implementation uses. It allows the withdrawal in IR2-H-1, IR2-H-2
  and IR2-H-4 (the spans are the scanner's) and in IR2-H-3 (in the code view the line's only
  `unwrap` is the test's). The guard of claim 1 cannot see a wrong withdrawal that comes from
  either reader; only the compiler, or a lexer written to the compiler's rules, can be its
  oracle. It would fail if withdrawal went back to "by line" (the author's plant control).
- `a_production_call_on_the_line_of_a_proved_test_stands`: fails if the fix is reverted. No
  line where the code view and the compiler differ.
- `a_file_the_compiler_reads_otherwise_proves_no_test_code`: fails if `top_level_is_items` is
  removed. Two shapes, both at the top level of the first lines; nothing inside a comment, a
  macro body or a string.
- `a_conditional_skip_whose_condition_changed_is_evidence`: fails if the pairing goes back to
  the signal. One changed condition, an identifier that stands directly in the list.
- `a_word_of_the_parsers_that_nothing_read_is_unchecked`: fails without `unread_words`. Rows
  the parser did not read only.
- `the_cluster_scores_under_the_plan_of_the_whole_review`: sound; it fails if the cluster
  scores a file under its own plan (it first asserts the two plans differ).
- The 600,000-row `readable` test: sound by arithmetic; its bound is a wall clock.
- No test holds a review at the budget to a time: the request's figure rests on one text
  (`a?b:`), and `#!#!` is four times slower.

## Uncommitted work after `bebebef`

`FIX2` (`git diff bebebef` in the worktree at about 10:57: no parse clock in the fragment
scan, raw identifiers read as the identifier, the walk keeps the parent and the attribute
run itself). All outputs: `out-fix2/`.

- **IR2-H-1, IR2-H-2, IR2-H-3, IR2-H-4: all still open.** `n2`, `n3`, `n3b`, `n1b`, `n1c`,
  `n4`: score 4, no evidence, as on `NEW` (`out-fix2/h-cases.txt`).
- **IR2-M-1: still open** (`out-fix2/m3.txt`, the same five rows).
- **IR2-M-2: the four raw-identifier rows are fixed by the newer build.** Still open:
  `Option::<u32>::unwrap` as a value, the nested `cfg_attr`, and the two nested-comment rows.
- **IR2-U-H-1 (high, new in the uncommitted work). With no clock the quadratic text is not
  cut.** `perf/hb1x64k.patch` (65,103 bytes, one new file: `// todo` and one line of `#!`
  repeated): `FIX2 --stdin-patch --rank --json`: **14.79 s** wall, 14.74 s user (`NEW`:
  1.32 s, the clock). `perf/hb4x64k.patch` (260,412 bytes, four such files):
  **59.96 s** for `--rank --json`, **121.45 s** for `--outline`, 22 MB (`out-fix2/perf.txt`).
  The comment added to `scan_fragment` says 56 shapes were measured and the slowest
  costs 14 microseconds a byte "in step with its length"; this one costs about 230 at 64 KiB
  and grows with the square. `hb16x16k.patch`: 15.70 s (`--outline` 30.93 s), as on `NEW`.
  IR-M-1 is closed by it (no clock is asked), at this price.
- The walk without `parent` and `prev_sibling`: read, not attacked further. The run of
  attributes is cleared by anything but an attribute or a comment, as before.

## Suspected, not reproduced

- Other places where the grammar's lexer and rustc part with no error node. Read in
  `scanner.c` and `grammar.js` and judged alike, not run: raw strings (the hash count is a
  `uint8_t`, and rustc refuses more than 255), char literals against lifetimes, the shebang
  rule against `strip_shebang`. A differential run of the two lexers is the only way to close
  the class; three reviews have now each found another member.
- The parse clock and `#!` text (`NEW`): on a loaded machine the unwrapped reading of a
  smaller hunk is cut and the wrapped one answers, so a whole-file proof could be lost or kept
  by load. This direction only loses a withdrawal. Not run.

## Not looked at

- The patch parser's agreement with Git (what "creates the file whole" rests on), the daemon,
  RPC, MCP and TUI entry points, `--hunks` paging, Windows and macOS.
- The Python, JavaScript and Go rules; `unwrap_err` and `expect_err` (no rule; not in the
  README's list either).
- Memory near the input limit; many small hunks other than the ones above.
- Every test, Clippy, Miri and the fuzzers: none run. The report of Vesper's round 8, which
  appeared in the worktree during this review, was not read.
