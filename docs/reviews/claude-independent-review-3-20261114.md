# Third independent review of the integrity signals: commit `2824a8b` (on `1228c55`)

Date: 2026-11-14. Reviewer: an independent Claude agent that took no part in the work. Scope:
`git diff 1228c55..2824a8b`, read from `git archive 2824a8b`
(`/tmp/claude-ddf-indep-review-3/src/`), not from the working tree. Read-only on the
repository; this file is the only one written. No `cargo` was run. `rustc` 1.99.1
(`fedora-toolbox-44`) was run on single files of a few lines.

## Binaries

| Name here | File | SHA-256 (first 16) |
|---|---|---|
| `NEW` | `~/.cache/ddf-t/review-bin/v3r10/release/deep-diff-forge` | `29b3e9a6e6829982` |
| `NEW-debug` | `~/.cache/ddf-t/review-bin/v3r10/debug/deep-diff-forge` | `90a22d516c24b156` |
| `OLD` | `~/.cache/ddf-t/review-bin/v3r9/release/deep-diff-forge` | `67bfb97197cab545` |

`OLD` has the hash the second review called `FIX2` ("uncommitted work after `bebebef`"). I
used it as the brief names it, the build before this commit, and only for comparison.

Everything below is under `/tmp/claude-ddf-indep-review-3/`: patches in `p/` and `perf/`,
outputs in `out/`, compiled cases in `rustc/`, tools in `tools/`. Every quoted output is from
`NEW` unless it says otherwise. Times are user CPU seconds on this machine (16 cores, load
2 to 4 while measuring); the request's own figures reproduce here (`hb22k` 1.77 s,
`tern128x1k` 1.84 s, `mix_hb7_tern94` 2.66 / 2.79 / 2.77 s).

## Verdict: NO-GO

One high finding and five medium ones.

- Claim 3 (bounded by counts, worst 2.8 s) is broken by a factor of about forty: an 8 KB
  patch takes 15 to 17 s in each of rank, outline and hunks, a 22 KB patch 118 s, all inside
  both budgets. The square budget was sized on one text; another text costs about sixty times
  as much per byte squared.
- Claim 4 (a parser's word in a parsed hunk is an item or `syntax_unchecked`, with one
  declared exception) is broken in three more places, each compiled and run.
- The new "whole text" comparison of attributes (the fix for IR2-M-1) removes whitespace
  first, and whitespace is not always free: a failing test becomes an ignored one with no
  evidence.

Claims 1, 2 and 5 held as far as I tested them. The deletion did what it says: every
reproduction of the second review and of round 9 now carries its item
(`out/old-repros.txt`).

---

## Findings I reproduced

### IR3-H-1 (high). Text inside both budgets costs 15 s at 8 KB and 118 s at 22 KB

The square budget assumes the worst quadratic text costs about 3.76e-9 s × bytes². A hunk
that is `// todo` and then one line of `&)` repeated costs about 2.4e-7 s × bytes² for a rank
(two wrappings are parsed, each about 1.2e-7): sixty times the assumed constant.

| Patch | Size | `--rank` | `--outline` | `--hunks` |
|---|---|---|---|---|
| `perf/amp4k.patch` | 4,198 B | 4.07 s | | |
| `perf/amp8k.patch` | 8,102 B | 17.51 s | 15.61 s | 15.52 s |
| `perf/q22k-2629.patch` (one hunk of 22,000 B) | 22,102 B | 118.3 s | | |
| `perf/amp128x1k.patch` (128 hunks of 1 KB) | 141 KB | 30.20 s | | |

Command: `/usr/bin/time -f '%U' NEW --stdin-patch --rank --json < perf/amp8k.patch` (and
`--outline`, `--hunks`). Each output shows the hunk was parsed (no `syntax_unchecked`; score 4,
`new_file`). Growth is on the square: 512 B 0.063 s, 1 KB 0.247 s, 2 KB 0.989 s, 4 KB 4.02 s,
8 KB 15.5 s, 22 KB 118.3 s (`tools/shape1.py SIZE '&)'`).

Controls:

- The same 4 KB with `// tada` for `// todo` (no parser word): 0.001 s. The time is the parser's.
- With `{` as a second line (only the function wrapping is tried): 2.0 s; balanced, or with
  `}`: 4.0 s. So both wrappings are slow, and the doc's "for each wrapping" applies in full.
- `OLD` on `amp4k`: 6.28 s. The weakness is older than this commit; what is new is the claim
  and the budget sized on it.
- `NEW-debug` on 1 KB: 1.01 s.

It is not one freak text. A search of every two-token unit over 35 tokens at 4 KiB
(`tools/pairs.py 4096 2`, result `out/pairs2-4096.txt`) found 52 units over 1 s and 32 over
3.3 s, among them `,*`, `&)`, `>*`, `+-`, `->*`, `)&`, `-,`, `*)`; the request's `#!` costs
0.07 s at that size. Three-token units of the same tokens were no worse
(`out/triples-1024.txt`).

The linear budget is off too, by less: 128 hunks of 1 KB of `a,` rank in 5.00 s and of `a?` in
3.45 s (`perf/ac128x1k.patch`, `perf/aq128x1k.patch`), against the stated 1.8 s for "the
slowest linear text" (`a?b:`, 14 microseconds a byte; these are 26 to 38).

Expected by claim 3: at most what the budgets allow, "worst found 2.8 s". A consumer with a
10 s limit is passed by an 8 KB patch.

On a fix: the budget is a count multiplied by a constant that was measured on one text, and
no test would notice another text (see "Weak tests"). A smaller square budget set from `&)`
would be about (2.8 KiB)², which reads almost nothing of real code. The cost that grows is
error recovery, so the charge may have to follow something the parse itself reports, or
the parse may need a bound that gives "not checked" for the hunk in a way that does not
depend on the machine (a count of the parser's own steps, if the tree-sitter version in use
offers one; I did not check). I tested neither.

### IR3-M-1 (medium). An attribute's "whole text" is compared without whitespace, and a newline inside it is not free

`ConstructKind::Attribute.text` is the attribute with all whitespace removed. A newline ends
a line comment, so moving one changes what is code and what is comment while the text stays
the same. Patch `p/at1.patch`:

```
 #[test]
 #[cfg_attr(
     all(
-        // heavy: only on CI
-        feature = "ci",
+        // heavy: only on CI feature = "ci",
         target_os = "linux",
     ),
     ignore
 )]
 fn checks_the_ledger() {
```

`NEW --stdin-patch --rank --json < p/at1.patch`: `src/at1.rs score 2 []`, no evidence.
Expected `test_skipped`: the condition went from `all(feature = "ci", target_os = "linux")`
to `all(target_os = "linux")`. Compiled with `rustc --test` and run (`out/at1-rustc.log`):
before, `test checks_the_ledger ... FAILED`; after, `test checks_the_ledger ... ignored`,
`test result: ok`. That is the change the signal exists for.

Control `p/at1ctl.patch` (the `feature = "ci",` line deleted outright): `test_skipped`, line
13, `syntax_fragment`. So the fix for IR2-M-1 works until the text can be kept equal.

Two more of the same kind: `p/at2.patch` (`// x` / `all(miri), // all(),` becomes
`// x all(miri), //` / `all(),`: no evidence) and `p/at3.patch` (a space inside a string:
`target_os = "lin ux"` becomes `"linux"`: no evidence). The old side has to hold a line
comment or such a string already, which the author of a patch does not choose; a multi-line
`cfg_attr` with a comment on one of its conditions is ordinary code.

Fix: compare the attribute's tokens (the leaf nodes of its tree, comments left out, string
literals kept whole), or its text with only the whitespace between tokens removed.

### IR3-M-2 (medium). IR-M-2 is open in a `macro_rules!` body: a split construct leaves no item and no mark

`fragment.rs` marks rows unreadable for a `token_tree` whose parent is `macro_invocation`.
The body of a `macro_rules!` definition is a `token_tree` under `macro_rule`: the parser
names no construct there and the rows count as read, so `unread_words` never looks at them.
The first review named this place ("`proptest! { … }` and any `macro_rules!` body"); the fix
covered invocations only.

| Patch | New file holds | `NEW` | Run |
|---|---|---|---|
| `p/mr1.patch` | `$x.unwrap` / `()` in a `macro_rules!` arm, used by `production` | score 4, `new_file` | `rustc/mr1`: exit 101, panicked |
| `p/mr2.patch` | `assert!(` / `true` / `);` in an arm, used by a test | score 4, `new_file` | `rustc/mr2`: `test t ... ok` |
| `p/mr3.patch` | `todo` / `!()` in an arm | score 4, `new_file` | |
| `p/mr4.patch` | `#[` / `ignore]` in an arm that makes a test | score 4, `new_file` | |
| `p/mr5.patch` | `Option::` / `unwrap($x)` in an arm | score 4, `new_file` | |

Control `p/mr1ctl.patch` (the same two lines inside `vec![…]`): `syntax_unchecked`, line 2.
`OLD` gives the same five results, so this is not a regression; it is outside claim 4's one
declared exception. Logs: `out/rustc-cases.log`.

Fix: treat every `token_tree` that is not under an attribute as unreadable (or the whole
`macro_definition`).

### IR3-M-3 (medium). Where the grammar reads a comment and the compiler reads code, a construct over lines leaves nothing

The deletion removed what the grammar/compiler differences could withdraw. They can still
hide: a construct that only the parser finds (one written over lines) is silent when the
grammar takes its lines for a comment, and nothing marks the hunk, because for the grammar
the rows are read and hold no word. Using IR2-H-2's `įĪ` (the scanner reads it as `/*`
inside a block comment), patch `p/gc1.patch`:

```
pub fn production(x: Option<u32>) -> u32 {
    /* įĪ */ let v = x
        .unwrap
        (); // */
    v
}
```

`NEW`: `src/gc1.rs score 4 ['new_file']`, no evidence. Compiled (`--edition 2024`) and run:
exit 101, `panicked at rustc/gc1.rs:3:10`. Control `p/gc1ctl.patch` (`xy` for `įĪ`): score 7,
`panic_path_added` line 3. The same hides an assertion (`p/gc3.patch`: `/* įĪ */ assert!(` /
`true` / `); // */`) and a skip (`p/gc4.patch`: `/* įĪ */ #[test] #[` / `ignore] // */`):
both score 4, no evidence.

The request lists what is not fixed (a call in a macro body after `/* /* */ // */`; the hint
on the IR2 texts). This is neither: plain code, no macro, and the token rules' code view is
right about it. The lexer's code view and the grammar disagree on these lines, and that
disagreement is visible to the program.

Fix: on an added line, a parser word that the code view shows as code and that lies inside a
comment or string node of the tree is unread: mark the hunk `syntax_unchecked`.

### IR3-M-4 (medium). A comment inside an assertion written over lines makes it "not trivial"

`MacroCall.arguments` is the text of the token tree without whitespace, comments included.
`signal_of` wants `true` or `true,…`. Patch `p/ca1.patch`:

```
#[test]
fn checks() {
    assert!(
        true // always
    );
}
```

`NEW`: `src/ca1.rs score 4 ['new_file']`, no evidence; `rustc/ca1`: `test checks ... ok`.
The arguments read `true//always`. No mark either: the word `assert` is the construct's own
name, so it counts as read. As a change to an existing assertion (`p/ca5.patch`:
`-        compute() == 4` / `+        // compute() == 4` / `+        true` under an unchanged
`assert!(`): score 2, no evidence. Also `p/ca2.patch` (`true /* always */, "msg"`) and
`p/ca6.patch` (`assert_eq!(` / `true, // compute(),` / `true`). On one line the token rule
finds it, because the code view blanks the comment (`p/ca3.patch`: `trivial_assert`,
`lexical`); `p/ca4.patch` (`true, // …` first) is found. `OLD`: the same.

Fix: build `arguments` from the token tree's tokens, comments left out.

### IR3-M-5 (medium). A method call with a turbofish is no construct

`construct` takes a `call_expression` whose function is a `field_expression`. With generic
arguments the function is a `generic_function` around it, and the token rule wants `(`
straight after the name. Patch `p/tf4.patch`:

```
fn b() -> Option<u32> { None }
pub fn production() -> u32 {
    let v = b().expect::<'static>("no");
    v
}
```

`NEW`: `src/tf4.rs score 4 ['new_file']`, no evidence, no mark. `rustc --edition 2024`
builds it with one warning (`late_bound_lifetime_arguments`, a future-compatibility lint, so
a build with warnings denied refuses it) and it exits 101, `panicked at rustc/tf4.rs:3:17`.
In a hunk of an existing file (`p/tf5.patch`, `::<'_>`): score 2, no evidence.

`b().unwrap::<>()` builds with no warning and panics (`rustc/t1`). There the grammar reports
an error, so the result is `syntax_unchecked` and not `panic_path_added` (`p/tf1.patch`,
`p/tf2.patch`, `p/tf3.patch`): marked, at weight 1 for a certain panic path. Control
`p/tfctl.patch` (`.unwrap()`): `panic_path_added`. `OLD`: the same.

Fix: look through `generic_function` in `construct`, and let `calls_method` pass over
`::<…>` between the name and the parenthesis.

### IR3-L-1 (low). Selecting one hunk now costs its whole file

`Scans::labels` scans the file of the hunk it is asked about. `perf/sel.patch` is one file
with a three-line hunk and a 4 KB hunk of the text of IR3-H-1.
`--hunks --address <the small hunk>`: `OLD` 0.00 s, `NEW` 4.00 s, same answer. On ordinary
files it is nothing; it is the request's question D, answered.

### IR3-L-2 (low). What the deletion left in the text

- `README.md` 986 to 989: "…and the rule that the Rust parser may add evidence but remove
  only a panic path in a file the patch creates whole." The rule is now "removes nothing".
- `CHANGELOG.md`, `[Unreleased]` / `Added`, third entry: "`panic_path_added` … added outside
  test code; inside a test function or test module that the hunk shows, it is not reported."
  The later Testing entries tell the history; the Added entry is what a reader of the release
  takes as the behaviour.
- `evidence.rs` 1484: "Up to three wrappings are tried". At most two are: the bare body
  needs `unopened == 0` and the `match` wrapping needs `unopened > 0`. The request and the
  budget's doc repeat "three".
- `score_file_in` (`risk.rs` 85) has no caller left; `hunk_signals` is called only by a fuzz
  target and `hunk_signals_in` only by a test, while their docs still say they are what the
  views show.
- `cli/src/bench.rs`: after this commit `rank_many_fragments` parses nothing (its hunks hold
  no parser word and are over 22 KiB, as its new doc says), so the workload that was added
  to watch the parser's cost no longer reaches the parser. I did not check whether another
  workload does.

### IR3-L-3 (low). A construct on unchanged lines that the change brings to life is silent

`p/un1.patch` deletes the `/*` and `*/` lines around an unchanged `let v = x.unwrap();` (and
the `unwrap_or(0)` line that stood in for it): score 2, no evidence. The token rules read
added lines only, by design, and the parser skips a construct whose lines are all unchanged
without asking the old side, where this one is comment text. The old side is already parsed
for constructs that reach an added line; the same question would answer this.

### IR3-L-4 (low). `test_removed` still reads only the bare attributes

`p/tr1.patch` removes a test under `#[tokio::test(flavor = "multi_thread")]`: score 2, no
evidence. `p/tr2.patch` removes one under `#[` / `test` / `]`: the same. The README names
the three bare spellings, so this is the documented rule; the new test is called
`a_removed_test_attribute_is_read_whatever_its_spelling`, and the tails `[test]`,
`[tokio::test]`, `[rstest]` end at the bracket.

---

## Suspicions I did not reproduce

- The fuzz target runs `scan_fragment` on up to 48 KiB of any text with no budget: by
  IR3-H-1's constant a single input can take minutes. I did not run the fuzzer.
- `#[expect(lint)]` suppresses a lint as `#[allow(lint)]` does and is not `check_suppressed`
  (`p/fp4.patch` line 1 gives nothing). It is outside this commit and I did not check the
  rule's history.

---

## The five claims

| Claim | Result | Basis |
|---|---|---|
| 1. Monotone | **Held** | By reading: `hunk_matches` takes each line's basis from the token rules and only raises it with the parser's answer; `Parsed` has no member that removes. The second review's `n2`, `n3`, `n3b`, `n1b`, `n1c` and round 9's file all carry their items now (`out/old-repros.txt`). The old-side comparison can hide a changed attribute (IR3-M-1), which is a parser-only item, not a token-rule item. |
| 2. One input, one answer | **Held, by reading** | No clock in `fragment.rs`, `evidence.rs`, `risk.rs`, `rank.rs` outside tests. Not tested under load. |
| 3. Bounded by counts | **Broken** | IR3-H-1. The part "the views cost what the rank costs" held on every patch I timed. |
| 4. A parser's word is accounted for | **Broken** | IR3-M-2, IR3-M-3, IR3-M-5; IR3-M-4 by the claim's intent. |
| 5. The corpus says what the binary does | **Held** | `NEW eval --corpus fixtures/eval` from the archive: exit 0, 45 cases, precision 77/90, recall 77/119, declared 58 (`out/eval.txt`). |

Also held: `test_likely` changes no score (`p/h1.patch` against `p/h2.patch`: both 7);
outline, hunks (paged at 4 KiB and at 16 MiB) and rank agree on the labels of all 5,101 hunks
of 149 real commits (`tools/views.py hist`: 0 mismatches); `--cluster --parallel 1|3|8` prints
the rank's order on five patches. The new path constructs gave nothing on `use`, a
visibility path, a type path, a macro path or a module path (`p/fp1.patch`); a path in a
pattern and `Kind::expect` as a constant are reported, as the request says (`p/fp2.patch`,
`p/fp4.patch`). Very many small parsed hunks are cheap: 26,000 one-line hunks rank in 0.80 s
(`perf/tiny26k.patch`).

## Weak tests

- **Nothing tests claim 3.** `the_plan_stops_at_the_square_budget` and
  `many_small_hunks_all_fit` check the plan's arithmetic on hunks that are one comment
  ("it costs the parser nothing but the reading"). No test parses hostile text inside the
  budgets under a bound, so the constant behind (22 KiB)² is not held by anything, and
  IR3-H-1 passes the whole gate. The bench workload for many fragments no longer parses
  either (IR3-L-2).
- `a_hunks_labels_come_from_the_one_scan_of_its_file` checks that the labels are right, not
  that a file is scanned once; scanning twice again would pass it.
- `a_conditional_skip_whose_condition_changed_is_evidence` has no attribute with a comment
  or a string that holds whitespace (IR3-M-1).
- The fuzz rule is claim 1 only. It cannot see a parser-only item that goes missing, which
  is all five medium findings.
- The commit names four plant controls. For the other new tests I read the code and could
  not run it; `the_cfg_attr_lists_of_a_line_have_an_allowance` is under a deadline and its
  doc says what the reverted code does, which is the right shape.

## The cost on real history

This repository's last 150 first-parent commits (`hist/`, 2,087 files, 880 Rust files, 2,974
Rust hunks), `NEW` against `OLD` (`tools/hist.py`, `hist2.py`, `hist3.py`, `histdiff.py`;
`out/histdiff.txt`):

| | `OLD` | `NEW` |
|---|---|---|
| Files with `panic_path_added` | 298 | 327 (37% of Rust files) |
| …of them on a test path | 145 | 145 |
| …of them with every listed item `test_likely` | 161 | 179 |
| Hunks with `panic_path_added` | 397 | 426 |
| `panic_path_added` items listed (32 a file at most) | 1,822 | 2,076 |
| …of them `test_likely` | 1,292 | 1,358 |
| Files with `syntax_unchecked` | 37 | 51 |
| Hunks with `syntax_unchecked` | 55 | 68 (41 new, 28 gone) |
| Commits with a `syntax_unchecked` file | 12 | 22 of 150 |
| Sum of scores | 7,812 | 7,913 |

- Of the 195 panic-path items `NEW` lists and `OLD` does not, 126 carry `test_likely`; the
  other 69 are test code in hunks the parser no longer reads (smaller budgets), so they have
  no hint. I found none in production code and none from the new path constructs.
- The budget's doc says a real review "spends little of this budget". Here 15% of commits
  have at least one file the parser did not read, up from 8%: a new Rust file over 22 KiB is
  never parsed, and commit `cb39f13` (186 KB) went from 0 unread files to 5.
- All 150 patches rank in 0.93 s together.

## What I did not look at

- Vesper's round 10 report, which appeared in the worktree while I worked: not opened.
- The TUI, the MCP server, the SDK and the daemon's other methods beyond a search for
  readers of `test_likely`, `basis` and the removed members (none reads a removed member).
- Memory beyond the peak RSS of the timed runs (under 16 MB each).
- Python, JavaScript, Go and shell rules; `lexical.rs`.
- Other Rust repositories for false positives: only this repository's history.
- The fuzzer, the unit tests and the mutation state: nothing was built or run.
- Why tree-sitter is slow on these texts.
