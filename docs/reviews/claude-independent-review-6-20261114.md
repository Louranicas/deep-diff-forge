# Sixth independent review of the integrity signals: commit `8f290e3` (2026-11-14)

Reviewer: a Claude agent that took no part in the work. Brief:
`~/.local/state/habitat-claude/ddf-gap-probes-20261113/ddf-independent-review-6-brief.md`.
Reviewed: `5810cb5..8f290e3` on `agent/integrity-signals` (source read from a `git archive` of
`8f290e3`), with the frozen binaries `~/.cache/ddf-t/review-bin/v3r13/{release,debug}`
(release SHA-256 begins `0d7b9cf4acd15591`), `v3r12`, `v3r11`, `v3r10` and the published
v0.8.3. I ran no `cargo`. `rustc` 1.99.1 ran single files in `fedora-toolbox-44` with the real
HOME. The only file I wrote in the worktree is this report. Everything else is under
`/tmp/claude-ddf-indep-review-6/` (called `S/` below). No command was refused by the
permission layer.

Vesper's round 13 report appeared in the worktree while I was writing. I read it after my
findings were reproduced. Its one finding (VD3R13-M-1) is my IR6-L-1; it accepts two things I
report as high (the line-number rule, and the conditional string reading).

## Verdict

**NO-GO.**

The change of model does what it says for the cases it was written for: IR5-H-1, IR5-M-1,
IR5-M-2, VD3R12-M-1 and VD3R12-M-2 are closed in their reported forms, no view is a mixture,
and a further view only adds items. The lexing of each view held against everything I threw
at it. What is wrong is **which views a side gets**, and that is decided from things the patch
writer controls or that ordinary diffs do not have:

- the test for "top of the file" believes the new-side number of the hunk header, which plain
  `git apply` does not use to place a hunk (IR6-H-1);
- the test for "began inside a string" is the end state of the side, so an ordinary `git diff`
  hunk that sits between two multi-line strings loses every item, including `#[ignore]` on a
  test, that rounds 10 to 12 reported (IR6-H-2);
- hunk boundaries are believed, and plain `git apply` accepts a created file written as
  several hunks (IR6-H-3; this one is also in the released v0.8.3).

## New findings

### IR6-H-1: a hunk header that says `+1` switches the new readings off and the shebang rule on; plain `git apply` places the hunk by its old side

`rust_hunk_views` (`evidence.rs:803`) takes a side for the top of its file when the number of
its first line is 1 (`at_top`). For the new side that number comes from the `+N` of the hunk
header. The CHANGELOG, the README and the request say this is "where `git apply` puts such a
hunk unless it is run with `--unidiff-zero` or `-C0`". That is not what Git does. Git decides
that a hunk must match at the beginning of the file from its OLD position (0 or 1), and uses
the new position as nothing more than the place where its search for the context starts
(that is my memory of `apply.c`; the runs below are the evidence). So `@@ -9,4 +1,5 @@` is
applied at line 9, with no option given, and the parser of this branch (and, by reading, of
the 0.8.4 work in `ddf-wt-084`) accepts any pair of numbers.

Three reproductions, all with `git apply` 2.56.0 and no option:

1. `S/lie/lie1.patch` (repository `S/lie/repo`): an existing block comment, a line added
   inside it.

   ```
   @@ -9,4 +1,5 @@
    pub fn h() {}
    pub fn i() {}
    pub fn j() {}
   +// */ pub fn evil() { std::process::Command::new("sh")…status().ok(); } /*
    */
   ```
   `git apply --check -v`: `Hunk #1 succeeded at 9 (offset 8 lines).` The file compiles and
   `evil()` runs (`S/rc.sh`). `v3r13`: score 7, no item. The same patch with `+9`
   (`S/lie/honest1.patch`): `v3r13` reports `process_spawn` on line 12.
2. `S/lie/lie2.patch`: the commit's own unit-test case `two-hunk-comment.patch` with ONE
   number changed in the second header (`@@ -9,3 +10,4 @@` to `@@ -9,3 +1,4 @@`).
   `git apply`: `Hunk #2 succeeded at 10 (offset 9 lines).`, the result is byte for byte the
   file the test describes, it compiles and `evil()` prints `EVIL-RAN`. `v3r13`: no item
   (with `+10`: `process_spawn`).
3. `S/lie/lie3.patch` (repository `S/lie/repo7`): **new in this commit, found by every
   earlier binary.** With `+1` the side gets `RustStart::FileStart`, and the shebang rule
   blanks a first line that starts with `#!` and is not an inner attribute. Inside a macro
   call such a line is code:

   ```
   @@ -6,1 +1,2 @@
   +#! std::process::Command::new("sh").arg("-c").arg("echo EVIL-RAN").status().ok();
    } }
   ```
   (the file has `macro_rules! run { (# ! $($t:tt)*) => { $($t)* } }` and
   `pub fn evil() { run! {` above it). `git apply`: `Hunk #1 succeeded at 6 (offset 5
   lines).`; rustc compiles it and it prints `EVIL-RAN`. v0.8.3, `v3r10`, `v3r11`, `v3r12`:
   `process_spawn`, score 11. `v3r13`: no item, score 7. With `+6`
   (`S/lie/honest3.patch`) `v3r13` reports it.

Command for each: `python3 S/ev.py FILE.patch v083 r12 r13`.

Why high: the number is the patch writer's word, the tool Git ships applies the patch where
the context is, and one digit undoes the round's fix for VD3R12-M-1 and for the string start,
and opens a hole (3) that no earlier version had. Round 4 of Vesper's review met the same
fact for the withdrawal ("a hunk that only says it starts the file") and the answer then was
not to believe the header.

Cheapest sound fix: a side is the top of its file only when Git must place it there: the
hunk's `old_start` is 0 or 1 (for both sides), never the new number alone. Better, in the
parser: refuse or mark as a hazard a hunk whose new start is not the old start plus the
lines the earlier hunks of the section added less those they removed; `git diff` never
writes one. No unit test and no fuzz property holds this: the only test with different
numbers on the two sides is `@@ -1,2 +4,2 @@`, the harmless direction.

### IR6-H-2: an ordinary diff hunk between two multi-line strings loses every item; rounds 10 to 12 found them

The reading from inside a string is made only when the side, read from code, ENDS inside a
string. A side that begins inside a string and also ends inside one (the context above
closes one multi-line string, the context below opens the next) ends, read from code, in
code. No string reading is made, and from the first quote on the code reading has code for
text. This is inside the limit the CHANGELOG declares ("another string opens on it and stays
open"). I report it as a finding because of what the declaration does not say: it is the
normal shape of a hunk in a snapshot or expect-test file, it needs no intent, and it is a
regression against the three rounds before.

`S/t/snap2.patch`, made by `diff -U3` from `S/t/snap2/old.rs` and `new.rs` (two tests in the
`check(input, r#"…"#)` style; the change adds `#[ignore]` above the second, which fails):

```
@@ -15,6 +15,7 @@
 "#);
 }
 
+#[ignore]
 #[test]
 fn renders_a_large_value() {
     check(538, r#"
```
`rustc --test`: before, `1 passed; 1 failed`; after, `1 passed; 0 failed; 1 ignored`.
`python3 S/ev.py S/t/snap2.patch v083 r10 r11 r12 r13`:
`r10`, `r11`, `r12`: `test_skipped` on line 18, score 4. `r13`: no item, score 1.

Same cause, other signals:
- `S/h/oldstr-both.patch`: a `#[test]` removed between two strings. `r12`: `test_removed`;
  `r13`: nothing.
- `S/t/strcmt.patch` (`diff -U3`): a function added after the closing line of a multi-line
  string, holding the string `"/*"`. Read from code the side ends in a block comment, not in
  a string. `r12`: `process_spawn`; `r13`: nothing. rustc compiles it and it prints
  `EVIL-RAN`. The CHANGELOG's wording "ends in code" does not cover an end inside a comment.

Measured (`S/fragtruth.py`, results `S/fragtruth.txt`, rows `S/fragtruth.jsonl`): 25,208
fragments of 40 lines cut from 2,500 real crate files (seeded sample of the cargo registry),
each as a hunk below line 1, against the truth (the same lines as a created file with the
true opener before them; the true state at the first line from the fifth review's lexer
harness).

| true start | fragments | true items | `v3r13` misses | `v3r12` misses | `v3r13` adds |
|---|---|---|---|---|---|
| code | 24,651 | 15,625 | 0 | 0 | 0 |
| block comment, depth 1 | 234 | 23 | 0 | 0 | 35 (code text in comments; `v3r12` the same) |
| ordinary string | 23 | 16 | 0 | 0 | 0 |
| raw string | 300 | 81 | **20** | 0 | 0 |
| block comment, depth 2 or more | 0 | | | | |

All 20 misses are fragments that begin in one raw string and end in another
(`winnow` `examples/arithmetic/test_parser_lexer.rs` in two versions of the crate, 8 each;
`document-features` `lib.rs`, 2; `pulldown-cmark` `build.rs`, 2); `v3r12` found every one. A
quarter of the true items in fragments that begin inside a raw string are lost. Raw strings
are thirteen times as common a start as ordinary strings, and the tests that show this
reading working use an ordinary string.

Cheapest sound fix: make the reading from inside a string for every fragment that holds a
quote (the implementer measured its cost: 481 false items against 107,720, where the
condition now loses 93 real ones and I lose 20 of 15,745), and let an item only that reading
finds say so (a basis or a member), so a consumer can weigh it. Two cheaper widenings that
catch `snap2` and its kin but not a deliberate writer: make the reading when the leading
unchanged lines alone, read from code, end inside a string; and add a reading from code that
starts at the first changed line. The test
`a_side_that_begins_inside_a_string_is_read_that_way_when_its_quotes_say_so` asserts the
miss (`found(&even) == []`): it pins the defect.

### IR6-H-3: hunk boundaries are the patch writer's; plain `git apply` accepts a created file in several hunks (released v0.8.3 too)

Each hunk is read alone. `git diff` never cuts two neighbouring lines into two hunks, but
`git apply` accepts it wherever the second hunk has no context and must match at the end:
after every line of a file the patch creates (second header `@@ -K,0 +K+1,M @@`), and at the
end of an existing file. The parser here accepts both (a creation section with `-2,0` too).

1. **A construct cut in two.** `S/lie/split4.patch` (created file), `S/lie/split2.patch`
   (end of an existing file, repository `S/lie/repo3`):

   ```
   @@ -0,0 +1,2 @@
   +pub fn a() {}
   +pub fn evil(x: Option<u32>) -> u32 { x.unwrap
   @@ -2,0 +3,1 @@
   +() }
   ```
   `git apply --check`: accepted, applied cleanly. v0.8.3, `r10`, `r11`, `r12`, `r13`: no
   item. As one hunk (`S/lie/joined1.patch` shape): `panic_path_added`, basis `hunk`. Every
   construct the reader of tokens follows over lines can be cut this way.
2. **The declared fragment limits reach a created file.** `S/lie/newfile-depth2.patch`: hunk
   1 ends with `/* /*`, hunk 2 is ` */ // */ pub fn evil() { …Command::new("sh")… }`.
   `git apply --check` accepts; the file compiles and prints `EVIL-RAN`. `r13`: no item,
   score 4; the same lines as one hunk (`S/lie/newfile-depth2-joined.patch`):
   `process_spawn`, score 8. With one comment level
   (`S/lie/newfile-depth1.patch`) `r13` finds it, unless the second header says `+1`
   (`S/lie/newfile-depth1-lie.patch`, IR6-H-1; `git apply`: `Hunk #2 succeeded at 3`).
3. **Released code.** `S/lie/newfile-depth1.patch` on v0.8.3 and `v3r12`: no item. A process
   spawn in a created file is hidden from the released binary by a patch that plain
   `git apply` accepts. This belongs with the nested-comment finding for the release track;
   it is not on a public remote and should not go to one before the owner decides.

Overlapping hunks I could not use: Git refuses a later hunk whose context touches lines an
earlier hunk patched (`S/lie/split1.patch`, refused), so inside an existing file only the end
is open.

Cheapest sound fix, in the parser: refuse a creation section whose hunks are not `-0,0` one
after another with no gap, or better join them; and join (or refuse) a hunk that starts on
the new side exactly where the hunk before it ends. Then a created file is one side read
from line 1, which is the case the lexer oracle proves.

### IR6-L-1: "shows what follows as code earlier, never later" is false (Vesper's VD3R13-M-1, found independently)

`S/t/depth2.patch` (`diff -U3`, two hunks, existing file): `/* /*` added in the first hunk,
` */ // */ pub fn evil() {…}` in the second. Read from depth one, the first `*/` ends the
comment and `//` then hides the rest of the line: the function is shown never. rustc
compiles it and it prints `EVIL-RAN`. `r13`: no item. The sentence is in `RustStart::Comment`,
the head of `rust_tokens.rs`, the README and the CHANGELOG.

### IR6-L-2: the views of a hunk are all held at once

A hunk's readings are built before any is read (`hunk_views` returns all of them). For a
fragment with three readings (`S/cost/`, `S/cost/cost.sh`):

| input | size | `v3r12` | `v3r13` |
|---|---|---|---|
| 999,000 one-byte lines, below line 1 | 3.0 MB | 1.27 s, 159 MiB | 1.41 s, **389 MiB** |
| the same as a created file | 3.0 MB | 1.28 s, 159 MiB | 1.10 s, 235 MiB |
| 999,000 lines with an item in each view, below line 1 | 60.9 MB | 3.63 s, 329 MiB | 5.80 s, 627 MiB |
| the same as a created file | 60.9 MB | 3.63 s, 329 MiB | 3.46 s, 382 MiB |
| one 30 MB line, below line 1 | 30.4 MB | 1.23 s, 120 MiB | 1.67 s, 149 MiB |
| 330,000 hunks, each a fragment | 20.6 MB | 1.84 s, 175 MiB | 2.51 s, 175 MiB |

(User plus system CPU seconds and peak resident memory, release builds, machine load 30 to
40.) Linear, and within "three readings" for time (at most 1.6 times). Memory is up to 2.4
times, and 48% more for a created file with one reading. "Rank in the time they took before" holds
for the implementer's inputs, not for these. Reading one view at a time and dropping it
would bring memory back.

### IR6-L-3: what the fuzz targets cannot see

- `rust_tokens` rules 1 to 4 build every hunk as `@@ -1,N +1,M @@`, so arbitrary text never
  reaches the readings from inside a comment or a string at the evidence level. Rule 5 puts
  its hunk below line 1 but with complete literals, so the string reading is never chosen
  there. Nothing holds `rust_hunk_views` (which views, for which side) but unit tests.
- No property varies the header numbers (IR6-H-1), the hunk boundaries (IR6-H-3), or cuts a
  hunk out of a whole file and compares its evidence with the file's (that property would
  have failed on IR6-H-2 at the first snapshot test). The three inputs I built by hand for
  these classes (`lie1`, `split4`, `snap2`) each show the code wrong.
- `rust_code_view` holds each view to its stated start; it does not ask whether the starts
  chosen include the true one.

Not a crash risk as far as I could tell: 30,000 random hunks at and below line 1 through
`--rank`, `--outline` and `--hunks` of the debug binary gave no panic (`S/smoke/smoke.py`).

### IR6-L-4 (older than the branch, noted in passing): `check_suppressed` reads one raw line

`#[/**/allow(dead_code)]`, `#[` and `allow(dead_code)]` on two lines,
`#[cfg_attr(all(), allow(dead_code))]` and `#[expect(dead_code)]` each compile under
`#![deny(warnings)]` and leave no item, in v0.8.3, `r12` and `r13` alike (`S/h/allow-*.patch`,
`.rs`). The five signals of the token reader read across lines and comments; this one does
not.

## The claims

1. **No view is a mixture: held.** By reading (`rust::side`, `rust_hunk_views`,
   `view_matches`, `unite`) and by the earlier sets: of 264 earlier reproduction patches 15
   change, all as the request says (`S/repro-r12-r13.txt`).
2. **Monotone in views: held.** `unite` keeps every item of the first reading. Every item
   `v3r13` loses against `v3r12` comes from the code reading itself being exact now, not
   from a further view.
3. **Rounds 11 and 12 stand.** One answer: held (same bytes twice; `--cluster` with 1 and 8
   workers differs only in the worker count it prints). Other languages identical: held on
   the fifth review's 30 patches (6 each of Python, JavaScript, Go, shell and YAML;
   `S/lang-r12-r13.txt`) and on every non-Rust section of 483 per-commit patches of this
   repository (86 Python, 93 YAML, 39 shell sections among them; all 19 patches that differ
   differ in `.rs` files only; `S/hist-r12-r13.txt`). Not tested: TypeScript, and Go and
   JavaScript beyond those 12. Cost linear: held, see IR6-L-2 for memory.
4. **The corpus says what the binary does: held.** `v3r13 eval --corpus fixtures/eval`: 48
   cases, 116/132, 116/162, 65 declared, exit 0.

Closed, as tested with the frozen binaries: IR5-H-1 (`union-hide`, `skip-hide`, `mod-union`,
`union-onequote` found; `union-fp` loses its four false items), IR5-M-1 (`kraw`, `kraw-rest`
found; `rawlt`, `rawlt-rest`, `craw2` unchanged and declared), IR5-M-2 (`lrm-raw`, four
`ws-*-raw` found), VD3R12-M-1 at depth one, VD3R12-M-2.

The shebang rule against rustc 1.99.1, nine forms the unit test does not have (`S/sb/`:
U+200E and U+0085 before `[`, a plain comment then a doc comment, `[` on line 5, a byte
order mark before an attribute and before a shebang, a space before `#!`, a nested comment
before `[`, U+00A0 before `[`): the binary and the compiler agree on all nine. The rule is
right; what is wrong is when it is applied (IR6-H-1, case 3).

Each true start of a hunk, with rustc evidence (`S/t/*.patch`, `S/rc.sh NAME`; the first row
is the two filed constructions in `…/finding-nested-comment-20261114/open-not-fixed/`):

| the hunk begins inside | case | `v3r13` |
|---|---|---|
| a block comment | `two-hunk-comment`, `inside-existing-comment` | found |
| a doc block comment | `docblock` | found |
| a comment two deep, closers written plainly | `depth2plain` | found |
| a comment two deep, `*/ // */` | `depth2` | **missed** (declared; IR6-L-1) |
| an ordinary string, side ends in a string | `strcont` (line continuations) | found |
| an ordinary string, side ends elsewhere | `strcode`, `strcmt`, `snap2` | **missed** (declared; IR6-H-2) |
| a raw string | `rawplain`, `rawquote` | found |
| a raw string whose last byte is `\` | `rawbs` | **missed** (declared) |

## The declared limits: how easy, how honest, what would fix them

- **Began inside a string, read from code does not end in one.** Deliberate: one line,
  `"; pub fn evil() { … } // "` (`strcode`). By accident: yes, routinely, in any file of
  multi-line strings with short code between them (IR6-H-2); once in this repository's own
  history (`json.rs:517`, commit `0b1c2b2`, where a string starting with `\"` puts the code
  reading back in step). Fix: under IR6-H-2.
- **Began inside a raw string, read as an ordinary one.** By accident it mostly works,
  because either the code reading or the string reading is in step after the closing quote;
  it fails with the limit above (all 20 measured misses are raw strings). Deliberate: a raw
  string whose text ends in a backslash (`rawbs`: `C:\last\"#; pub fn evil() {…}`), one
  line. Fix: a second string start that takes no escapes (it ends at the first quote), next
  to the ordinary one.
- **Two or more comments deep.** By accident: no (0 of 25,208 real fragments start two
  deep). Deliberate: one more `/*` than the fix for VD3R12-M-1 expects, in an existing file
  (`depth2`) or, through IR6-H-3, in a created one. A fixed further depth moves the
  construction one level up. Fix: a reading for each depth up to the number of `*/` the
  side holds, capped at a small number, and an item that says "not read" beyond the cap; or
  true prefix state when the patch comes from a Git selection. And correct the sentence
  (IR6-L-1).
- **Editions before 2021 (`cr"\"`, `'r#r"\"`).** By accident: hardly; these forms are rare
  in any edition. Deliberate: needs a crate on 2015 or 2018. Fix: do not model the edition;
  report the three forms themselves as an item when they stand on a changed line.
- **Frontmatter, Unicode after 18.** Not looked at beyond reading.
- **"Line 1 is the top of the file."** Wrong as stated: IR6-H-1.

## Measured change in false and lost items against `v3r12`

Per-commit patches of this repository, every commit that touches a `.rs` file (483, the
commit under review among them), each Rust hunk alone so that the CLI's cap of 32 items a
file does not hide anything (`S/perhunk.py`, `S/hist-perhunk.txt`, `.jsonl`): 6,040 hunks, 22
differ.

- Gained: 12 items, all text of strings. 9 from the reading inside a comment (strings that
  hold patch text with `*/`: `2824a8b` 1, `bebebef` 1, `a530cb3` 4, `8128c77` 2, and 1 in
  `8f290e3` itself), 3 from the reading inside a string (`rank.rs:67` in three commits).
  **The implementer's figure of 8 false items for the comment reading on 481 commits is
  confirmed** (my ninth is in the commit under review).
- Lost: 36, of which 35 are text of strings (help text, patch text, a probe program in a raw
  string: I checked the true state with the lexer harness) and 1 is real code
  (`json.rs:517`). Confirmed.
- 2 panic paths change their hint. Confirmed.

603 crates as created files (`S/reg-r12-r13.txt`): 20 differ; every difference I opened is
string or raw-string text no longer reported, or real code after a multi-line string now
found (`portable-atomic` `unsafe` on lines 1936 and 1966). Note for whoever repeats this:
`--rank --json` prints at most 32 items a file, so some "new" items in a large file are
only the window moving.

On fragments: the table under IR6-H-2. In my sample the two further readings added no
false item in 24,651 fragments that start in code (the implementer's 37 in 177,957 would
predict about 5; my sample is smaller and such items cluster in few files).

## Weak tests

- View choice by line number: only `-1 +4` is tested; nothing for `-N +1` (IR6-H-1).
- The string condition: the test pins the miss as the expected answer (IR6-H-2).
- No test or seed has a creation section with two hunks that cut a construct (IR6-H-3);
  the existing two-hunk creation case is about the withdrawal that no longer exists.
- Two lines of `view_matches` cannot be held by any test because removing them changes cost
  only: the skip of a blank line in a further reading, and the filter that keeps raw-line
  rules to the first reading. That is fine; say so in a comment so a mutation run does not
  count them as misses.
- The unit tests for uniting (`findings_of_two_readings_are_united`) and for the three plant
  controls the commit names are good: by reading, each fails without its rule.

## What I did not look at

- I ran no unit test, fuzz target, mutation run, Clippy or gate (no `cargo`).
- The scanner's token rules against the lexer beyond the cases above: the fifth review and
  the fuzz target cover them; I reran the fifth review's edition cases and nine shebang
  forms only.
- `git apply` with `-C`, `--unidiff-zero`, `--3way`, `git am`, GNU `patch` and other
  appliers; only plain `git apply` 2.56.0. GNU `patch` places hunks more loosely still.
- Two sections for one path on this branch (refused by 0.8.4, which this branch is not
  rebased on). I did not run a 0.8.4 binary; that its parser checks no hunk positions is
  from reading `ddf-wt-084`.
- TypeScript, and other languages beyond the sets named under claim 3.
- The daemon, MCP and review CLI paths beyond `--outline`, `--hunks` and `--cluster` on one
  case (they agree with the rank).
- `test_likely` as an attack surface: it is a hint and reaches no score.
- Frontmatter and characters after Unicode 18.

## Reproduction index

All under `/tmp/claude-ddf-indep-review-6/`: `ev.py FILE.patch [v083 r10 r11 r12 r13]`
(answers), `mk.sh NAME` (patch from `t/NAME/old.rs` and `new.rs`), `rc.sh NAME` (compile and
run with rustc), `lie/` (IR6-H-1 and IR6-H-3, with the scratch repositories `git apply` ran
in), `t/` and `h/` (IR6-H-2, the starts, IR6-L-1, IR6-L-4), `fragtruth.*`, `perhunk.py`,
`hist-*`, `reg-*`, `lang-*`, `repro-*`, `cost/`, `sb/`, `smoke/`.
