# Ninth independent review of the integrity signals: commit `4391c1e` (2026-11-15)

Reviewer: Claude, independent (no part in the work). Brief:
`~/.local/state/habitat-claude/ddf-gap-probes-20261113/ddf-independent-review-9-brief.md`.
Binaries: the frozen `v3r16` release build (SHA-256 `155700c0…`), `v3r15`, `v3r14`, `v3r13`
and the published v0.8.3 binary. I did not run `cargo` and did not build. Scratch:
`/tmp/claude-ddf-indep-review-9/` (notes `NOTES.md`, tools `tools/`, findings `f/`, result
files `results/`); the bulk sets were under `~/.cache/ddf-t/indep-review-9/` and are removed
(the generators and their seeds are in `tools/`). `git apply` runs are in a scratch repository
(`~/.cache/ddf-t/indep-review-9/git/r1`, Git 2.56.0); `rustc` 1.99.1 and `rustfmt` through the
toolbox, single files, real HOME.

## Verdict: GO-WITH-FIXES

No high finding. Nothing I made is lower than the published v0.8.3 binary or than round 14
(about 130,000 valid patches of my own making, three generators, every language; figures
below). Time is linear in every mode I measured. The three things the eighth review broke
are closed as the request says, and Vesper's case is closed.

Two medium findings stand, each reproduced:

- one way past the joins that is not in the list of remaining ways and that contradicts a
  sentence of the README (IR9-M-1; needs a writer and `git apply --unidiff-zero`);
- one edge form that an honest line does end on, in a real `git diff -U0` of code that
  `rustfmt --check` passes (IR9-M-2; claim 4 of the request is broken as it is written; the
  shape is rare, I found none in real code).

Both have small fixes or can be stated. The rest is low: statements that are narrower than
the binary (memory, one sentence about `--unidiff-zero`, the cost of joins on a `-U0` patch,
one remainder named in one of its two forms), spellings still unread, and gaps in the fuzz
target.

## New findings

### IR9-M-1 (medium; needs a writer who sets out to, and `git apply --unidiff-zero`): hunks at old position 0 are each "the top" and are joined to nothing, though the applier stacks them

`placed_at_top` calls every hunk of an existing file whose old position is 0 the top, and a
hunk whose old position is 1 and that holds an old line. `hinted_joins` skips a top hunk in
every family (`if before[index].is_some() || top[index] { continue; }`), and the join Git
must make asks for `!top[index]`. So two top hunks of one section are never read together,
whatever their numbers say. With `git apply --unidiff-zero` they do stand together:
old position 0 means "at the beginning" there too, so each such hunk is put at line 1, the
one written later ABOVE the one written earlier, and all of them above a hunk at old
position 1.

Reproduction, `f/top/` (base `base-m.rs`, a `main` that calls `boom(None)`):

- `twotop.patch`: `@@ -0,0 +3,2 @@` with `+    ()` / `+}`, then `@@ -0,0 +1,2 @@` with
  `+pub fn boom(opt: Option<u8>) -> u8 {` / `+    opt.unwrap`. The new numbers are the true
  places of the lines in the result.
  `git apply --unidiff-zero -v`: "Hunk #1 succeeded at 1 (offset -2 lines). Applied patch
  m.rs cleanly." (plain `git apply`: refused; `twotop-apply.txt`). Result
  `twotop-result.rs`: lines 2 and 3 are `opt.unwrap` and `()`; `rustc` exit 0; the program
  panics ("called `Option::unwrap()` on a `None` value", exit 101).
  v0.8.3 `m.rs 2`, v3r14 2, **v3r15 5 `panic_path_added`**, **v3r16 2, no item**. The same
  four lines as one hunk (`twotop-ctl.patch`): 5.
- `topone.patch`: `@@ -0,0 +1,2 @@` (the function head and `opt.unwrap`), then
  `@@ -1,1 +3,3 @@` with `+    ()`, `+}` and the context line `fn main() {`. **Every number
  of this patch is true.** `git apply --unidiff-zero -v`: "Applied patch m.rs cleanly", no
  offset; the result is the same file. v0.8.3 2, v3r14 2, v3r15 5, **v3r16 2**.
- `threetop.patch` (three hunks at `-0,0`, written last to first): applied, same file;
  v3r16 2. `topthen.patch` (`-0,0` then `@@ -1,0 +3,2 @@`, which is not a top): 5 in v3r14 to
  v3r16, by the new numbers. So the gap is exactly "the follower is itself a top".

What it is: not lower than v0.8.3 or round 14 (neither joins these). Round 15 found the
first two only through the bare `.unwrap` edge that this round rightly removed. A top hunk
is also read from the start of a file ONLY (`fragment: changed && !top`), so behind a
`-0,0` hunk that ends on `/*`, a `-1,1` hunk whose added line is
`// */ pub fn f() { spawn } /*` is a line comment to every reading (not built into a patch
by me; v0.8.3 misses it too; the same root).

What the documents say: the README lists "a hunk whose old position is 1 and that holds
old lines is read as the top … though `--unidiff-zero` … puts it lower when its old lines
are lower in the file". Here the old lines are at line 1 of the old file; nothing in the
list covers a second hunk at the top. And the README says "by the numbers alone … the
numbers join whatever they name" and "`git apply --unidiff-zero` … puts a hunk with no old
line at its new position": for old position 0 it puts it at line 1 whatever the new
number is (the run above; the eighth review noted the fact for one hunk).

Found by a check I recommend keeping (`tools/whole.py`): 20,000 changes cut into touching
hunks with honest numbers against the same change as one hunk. 757 cuts miss a signal the
whole reports; 756 are this class (617 with two hunks at `-0,0`, 139 with `-0,0` then
`-1,L`); 1 is a false item of the whole reading. Apart from this class the joins find every
honest cut.

Fix: in the reading by the numbers alone, the hunks of an existing file at old position 0
are one run in REVERSE order of the patch, and a hunk at old position 1 that holds old
lines goes after the last of them (it is then no top for that reading: read it as a
fragment too). Or declare it: a sentence in the README's writer list, a corpus entry, and
the `--unidiff-zero` sentence corrected for old position 0. Add two hunks at `-0,0` to
`rust_hunk_places` either way.

### IR9-M-2 (medium by the brief's rule; an ordinary diff can do this, rarely): `assert!(` and `true` on two lines is what rustfmt writes for an assertion that can fail

The request, claim 4: "An edge item is never reported on a line an honest diff can end
with." The form `assert!(true` (and `debug_assert!(true`, `assert_eq!(true, true`) is read
at the end of a side as a trivial assertion. rustfmt breaks a long condition before its
operator and leaves the first operand on a line of its own:

```rust
    assert!(
        true
            && "a string literal that is long enough to pass the width of a line all by itself, yes"
                .len() > limit
    );
```

Reproduction, `f/edge/`: `asserttrue-old.rs` (`debug_assert!(cfg!(debug_assertions) && …)`)
and `asserttrue-new.rs` (`assert!(true && …)`), both pass `rustfmt --edition 2021 --check`
(exit 0), the new one compiles. `asserttrue-u0.patch` is `git diff -U0` between them, one
hunk, `@@ -2,2 +2,2 @@`, whose new side is `    assert!(` and `        true`.
v0.8.3 `g.rs 2`, v3r14 2, **v3r15 and v3r16 5, `trivial_assert` new 2 (basis `hunk`)**. The
assertion is not trivial (it fails when the string is not longer than `limit`). The same
change at `-U3` (`asserttrue-u3.patch`): no item in any version.

How rare: the edit has to change the lines `assert!(` and `true` and leave the operator
line alone. Real code does write assertions that begin with `true` and go on
(`assert!(true == likely(true))`, `assert!(true > false)`, `assert!(true.ok_or(…).is_ok())`:
10 places in 32,296 files, the standard library's tests among them); none of them is
broken after `true`, so on my sets, as on the implementer's, the form fires on nothing.

Why the measurement did not see it: the set is every line as a ONE-line hunk, and this form
stands on two lines (in formatted code `assert!(` always ends its line). No one-line hunk
can end on it. The honest-line question was asked of lines, not of line pairs.

Fix: drop the two assertion forms from `waiting` (a cut between `true` and its `,` or `)`
behind two false numbers becomes one more declared miss; `assert!(true,` and
`assert!(true)` shown whole are still read by `trivial_assert`), or keep them and say in
the README and the CHANGELOG that this one form can be the end of an honest line, with the
example. I would drop them: it is the one kept form whose next token decides whether it is
the construct at all. For the other kept forms I found no honest continuation, by
reasoning and by search (below).

### Low findings

- **IR9-L-1 (an ordinary `git diff -U0` can do this, rarely; since round 14; statement).**
  A hunk of added lines is read as going on from the last earlier hunk that ends its file
  (what plain `git apply` does with a `-U0` patch). In the file the two are far apart, and a
  construct can form across them that is in neither place. `f/u0join/u0.patch`:
  `-debug_assert!(` / `+assert!(` at line 10 and `+        true,` at line 41 (an argument of
  some other call): `trivial_assert` in v3r14 to v3r16, nothing in v0.8.3 and v3r13.
  `u0-handler.js.patch`: a changed `} catch (e) {` at line 10 and an added hunk at line 41
  that begins with `}` (the usual shape of an inserted function): `error_swallowed`. The
  README says both readings are made; its list of what the further readings cost names the
  comment, string and released readings and "the end of a side", not the joins. Measured
  with `tools/joinfp.py` (each hunk's labels in the patch against its labels as a section
  of its own) on the `-U0` history of this repository (579 commits) and of forge-warden
  (531): 1,397 Rust sections of two or more hunks, 10,333 hunks; the joins add no label
  and take none. State it as a cost with that figure.
- **IR9-L-2 (needs a writer; in v0.8.3 too): spellings of the released paths still unread.**
  `f/spell/q.rs` compiles and runs ("ran true true true true true true true 1 true"):
  `<p::Command>::new` (after `use std::process as p`), `<(Command)>::new`,
  `<Command<>>::new`, `<std::net::TcpStream>::connect`, `<(TcpStream)>::connect`,
  `<self::UdpSocket>::bind`, `<<Command as T>::Me>::new`. `spell.patch` (each as an added
  line of a fragment): 2 in every version for all seven; only `<Command>::new` is 6. The
  rule reads the name "alone in angle brackets"; a path, a parenthesis or an empty list
  beside it is the same type to the compiler. I found no false item of the two new
  spellings (`Vec<Command>::new`, `Vec::<Command>::new`, a function `build<Command>(…)`
  are not read).
- **IR9-L-3 (statement): memory is more than 2.0 times the release on shapes of mine.**
  Touching hunks with honest numbers and many context lines are one text, and the views of
  that text hold a line twice (old side and new). `f/cost/make.py ctx-touch 43000` (43,000
  hunks of 21 short lines): v0.8.3 106 MB, v3r14 282, **v3r16 279 MB, 2.63 times**;
  `ctx-touch-60 43000` (context lines of 60 bytes, a 54 MB patch, inside both budgets):
  197 MB against **450 MB, 2.28 times** and above the three figures the request states
  (262, 317, 226 MB). Linear (20,000 hunks 132 MB, 40,000 260 MB), and the same in round
  14: a statement to correct ("up to 2.7 times, about 450 MB at the budgets"), not a
  regression. Figures in `f/cost/memory.txt`.
- **IR9-L-4 (needs a writer; statement): the declared decoy has a mirror form the README
  does not name.** Declared: a third hunk with lines, written before the two, "says falsely
  that it ends before the same line". The same works when the third hunk says it BEGINS at
  that line and is written between the two: the first hunk is followed by the first hunk
  of the patch that says it begins there. `f/mirror/mirror.patch` (base `base-f.rs`):
  hunk 1 ends on the context line `debug_assert!(`, hunk 2 is `@@ -4,2 +4,2 @@` changing
  `let z = 3;` (true place: line 7), hunk 3 is `@@ -4,2 +4,2 @@` changing `a < b,` to
  `true,`, both its numbers true. `git apply --check -v`: "Hunk #2 succeeded at 7 (offset 3
  lines)", rc 0; the result holds `debug_assert!(true, …)`. Every version: 2, no item. With
  the decoy's true numbers (`mirror-ctl.patch`): 5 in v3r14 to v3r16. The doc comment of
  `hinted_joins` covers it ("the first of the patch is taken by the first hunk … that says
  it begins there"); the README sentence and the corpus have the one form only.
- **IR9-L-5 (documents).** (a) The README's `--unidiff-zero` sentence, as in IR9-M-1.
  (b) `CHANGELOG.md` line 1039, in the round 14 entry's list of fuzz rules: "a construct cut
  beside its word is found whatever the numbers and the order (7)" stands unmarked (lines
  625 and 906 quote the sentence as withdrawn; this one does not). (c) `evidence.rs` line
  6990, a test comment: "hunk stands in at most three texts, so no bound on joins is
  needed" (seven now). (d) Claim 4 holds for lines of code; a line of prose inside a block
  comment or a string over lines, read from code because the hunk does not show the
  opener, is reported as any construct in it is: `f/edge/prose.patch`
  (` * The fast path is still a todo!` as a one-line hunk): `stub_added` in v3r15 and
  v3r16. That is the standing limit of an unknown start, here through an edge form; none
  in 12.5 million lines (below). Say "a line of code".
- **IR9-L-6 (tests): what `rust_hunk_places` still cannot generate.** Two hunks at old
  position 0, or a top hunk after another hunk, in an existing file (IR9-M-1). Rule 9 takes
  honest lines one at a time: no honest form over two lines (IR9-M-2). Rules 6 and 10 give
  the place a hunk ENDS before to a decoy, never the place a hunk begins at (IR9-L-4).
  No rule compares a file cut into touching hunks with honest numbers against the same
  change as one hunk for every cut point of any text (the graph tests do it for 265 cuts
  of 28 constructs; `tools/whole.py` found IR9-M-1 in its first run). Nothing in the gate
  applies a patch with Git, plain or with `--unidiff-zero`, and compares where the hunks
  went with the texts read (the seventh review said so too). Nothing holds a memory figure.
  By reading, each rule of this round has a unit test that names it (`what_a_side_ends_on…`,
  `what_an_honest_line_ends_on_is_no_item`, `what_a_side_begins_with_is_no_item`,
  `the_released_paths_are_read_in_two_more_spellings`, `a_later_hint_takes_no_earlier_join_away`,
  `every_join_of_round_14_is_a_text…`, the counting test in `views.rs`); I ran none and
  planted nothing.

## The claims of the request

1. **Monotone in the full answer; round 14's joins all made; the list keeps v0.8.3's
   items.** Held. `Hints::Lined` is round 14's rule (I compared it with the round 14
   source, step by step: the same candidates, the same order, the same `followed` state;
   the ring check cannot fire there, since a hunk with no old line is no candidate).
   v3r14 to v3r16 in the full answer (`PLAIN=1 cmp8.py`): 0 lower on 30,000 join patches of
   mine (15,871 with a gain), 17,880 mutants of the committer's 6,002 patches, 34,000
   patches of the eighth review's generator under new seeds, and 65,560 earlier mutants
   (of those a quarter were refused by both binaries: my first generator wrote hunks that
   change nothing; I count them as not tested).
2. **Never lower than v0.8.3.** Held: the same sets, with views checked against the rank
   (`cmp8.py --views`): score, signals, listed items at the bound, outline labels,
   refusals: 0 lower. The rules and matchers of the release's signals outside Rust are
   textually the release's (`token_in`, `imports_module`, `looks_like_secret` unchanged;
   every rule of the release's table is in today's or in `LEGACY_RUST_RULES`), so the mark
   `released` outside Rust marks exactly what the release reported.
3. **No header number decides what is found, except old position 0 or 1; a hint takes no
   join of round 14 away.** Held as written. What old position 0 or 1 decides is more than
   the documents say under `--unidiff-zero` (IR9-M-1).
4. **An edge item is never reported on a line an honest diff can end with.** **Broken as
   written** by IR9-M-2 (one form, rustfmt's own layout, rare). For every other kept form
   I found no honest continuation: rustfmt keeps `#[allow(` together, never breaks a path
   after `::`, rewrites `.unwrap::<>()` to `.unwrap()`; `todo!` and `unimplemented!` are
   followed by a bracket everywhere in code (the 28 other places in 32,296 files are
   comments and strings); the old-side forms have no honest ending. The `cfg_attr` list that ends on
   `ignore` (`Skips::end`, not in the request's list of forms) is a true skip when rustfmt
   writes `ignore` on a line of its own.
5. **Cost linear in time in every mode; memory linear, up to 2.0 times.** Time: held (rank,
   outline, `--hunks` by path with four shebangs in turn, by one address 92,000 hunks into a
   run of 400,000, `--cluster`): `f/cost/`. Memory: linear; the factor is 2.6 and the peak
   450 MB on shapes of mine (IR9-L-3).
6. **The corpus says what the binary does.** Held: `v3r16 eval --corpus fixtures/eval` on an
   archive of the commit: 51 cases, 163/183, 163/230, declared 90, the constants of
   `crates/deep-diff-forge-cli/tests/eval.rs`.

Earlier findings rerun on `v3r16`: Vesper's two patches 7 (v3r15 11); `decoy2` 5; `triple`
5; `q.patch` 9; `alone`, `value`, `inside-u0` 2 as declared. The view by path of a file
with no extension and four shebangs in turn: 12,500 / 25,000 / 50,000 hunks 0.28 / 0.57 /
0.74 s (v3r15 9.3 / 36.8 / 86.1 s; v0.8.3 0.20 / 0.43 / 0.54 s).

## Views

`--outline`, `--hunks --path P` and `--hunks --address A` for every address agree on every
hunk's labels (5,002 patches, 0 mismatches); several addresses at once, reversed, shuffled
and half of them (1,500 patches with two or more hunks): 0; `--cluster --parallel 4` prints
the rank's lines (2,999 patches). I read `views.rs`: `Scans` asks for the joined hunks and
the kinds of a file once (`OnceCell`), `hunk_signals` is called only for one joined hunk
shown alone, and I found no other call made once for each hunk that walks its file. The
counter `file_walks()` is a thread-local that is only added to and read by tests: it
cannot change an answer, and two threads have two counters.

## The declared limits and the questions F and G

- The list in the README is accurate as far as I ran it (the corpus's known misses are
  checked by `eval`; `alone`, `value`, `inside-u0`, the decoy with lines, the two-section
  entry). Marks: right, with the eighth review's corrections taken. Not complete:
  IR9-M-1 (a second hunk at the top under `--unidiff-zero`), IR9-L-4 (the mirror of the
  decoy), IR9-L-1 (the cost of the joins on a `-U0` patch, on the ordinary side),
  IR9-L-2 (the bracketed spellings beyond the name alone).
- **The pairing option not built** (count `.unwrap` at the end of a side only when another
  side of the file begins with `(`): not required. It is one more reading whose only
  purpose is a cut behind two false numbers, and IR9-M-1 and IR9-L-4 show that a writer
  has other ways that cost him nothing more.
- **The two generated patches lower than v3r15 through the join rule:** they do not
  matter. Round 15's join there was the defect IR8-M-1 named; in 34,000 patches of the same
  generator under new seeds I found no such case (every v3r15 to v3r16 loss, 2,052, is in
  a Rust file and is the narrowed edge), and I looked at one of the implementer's two
  (`g7-03836`: random lines of a Python pool, no construct a reader would ask for).
- **The two-section dependency, the memory statement:** the first is acceptable as stated
  (not attacked again); the second needs the larger figures.

## False positives: what I checked

- Edge items on real code not in the implementer's set, every line as a one-line hunk
  (`tools/linescan.py`: numbers apart so that nothing joins, every item of basis `hunk`
  printed): the standard library's sources (3,072 files, 1,509,754 lines): **0**; a second
  crate registry (21,342 files), the Turso 0.8.3 sources and the Rust of other projects on
  this machine (30,026 files, 11,032,482 lines after removing identical files): **0**. The
  tool reports the planted forms (`todo!`, `process::`, `assert!(true`).
- Forms over several lines, by a search of the text of 32,296 distinct files (those sets
  and the 603 crates; `tools/multiline.py`): `assert!(` … `true` and then a line that is
  not `,` or `)`: 0; a path name and `::` at a line end: 0; `todo!` or `#[allow` followed
  by anything but its bracket: only in comments and strings.
- Joins on `-U0` history: 0 labels added or taken in 10,333 Rust hunks (IR9-L-1).

## What I did not look at

- No `cargo`: no test run, no plant, no fuzzing.
- `rpc`, MCP and the daemon (the same `views.rs`; read, not run). Editions before 2021,
  frontmatter, Unicode; the Rust scanner and the released view themselves; the further
  starts; `test_likely`: only in bulk against v0.8.3.
- The two-section forms; the raw-string records; the 265-cut enumeration (the graph test
  holds its counts; not rerun).
- The committer's list of 6,002 patches itself (the request's figures stand unchecked by
  me; I used the list as seed for 83,440 mutants).
- A two-line-hunk scan with the binary (I searched the text for the two-line forms
  instead).
- No command was refused by the permission layer.
