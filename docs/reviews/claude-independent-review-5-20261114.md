# Fifth independent review of the integrity signals: commit 5810cb5 (2026-11-14)

Reviewer: a Claude subagent that took no part in the work. Commit `5810cb5` on `7004510`,
read from `git archive`. Binaries: the frozen `v3r12` (this commit), `v3r11`, `v3r10` and the
published v0.8.3 release binary. No `cargo` was run. `rustc 1.99.1` was run on single files
in the toolbox, for compiler evidence and for a small harness around `lexical.rs` (old and
new copies side by side). Scratch and every reproduction: `/tmp/claude-ddf-indep-review-5/`.
I read Vesper's round 12 report only after my findings were reproduced; none of the three
below is in it.

Helper used in every reproduction (prints score, file signals and evidence items for each
named binary): `python3 /tmp/claude-ddf-indep-review-5/ev.py FILE.patch v083 r10 r11 r12`.

## Verdict: NO-GO

The new scanner does what the commit says at the level of the lexer: I could not find Rust
text that compiles on edition 2021 or 2024 and whose code it blanks when read from the
first line of a file. The verdict comes from three things that the lexer oracle cannot see:

- **IR5-H-1 (high):** after any string that runs over a line end, the view is the union of
  two readings that are out of step for the rest of the hunk, so later string and comment
  text is shown as code. The token reader reads that union, and a comment that holds two
  quotes between a construct's tokens hides all five integrity signals. `v3r10` found them.
  Not declared. In real code 6.9% of files already hold such a string.
- **IR5-M-1 (medium):** on editions 2015 and 2018 two more texts read differently from the
  compiler, `k#r"\"` and `'r#r"\"`. Neither is declared (only `cr` is). One line of either
  blanks the rest of the hunk, where v0.8.3 and `v3r11` reported everything. The `k#` rule
  is right in no edition that rustc 1.99.1 accepts.
- **IR5-M-2 (medium):** `r#unwrap`, `r#ignore`, `r#assert` written after one of the five
  white space characters past ASCII (U+200E and its kin) are not read as the names they
  are. `v3r10` found them.

## IR5-H-1 (high, reproduced): after a string over a line end, a comment with two quotes hides any integrity construct

**What happens.** `a/union-hide.rs` is a 24-line new file. Lines 1 and 2 are a constant
whose string runs over the line end. Then come an `unwrap`, a `todo!`, an ignored test and a
trivial assertion, each with the comment `/*""*/` between two of its tokens.
`a/union-ctl.rs` is the same file with the constant written on one line.

```
python3 ev.py a/union-hide.patch v083 r10 r11 r12
r10  score 18  panic_path_added 5, stub_added 9, test_skipped 15   (syntax_fragment)
r11  score 9   (no item)
r12  score 9   (no item)

python3 ev.py a/union-ctl.patch v083 r10 r11 r12
r10  score 21  the same three and trivial_assert 22
r11  score 21  panic_path_added 5, stub_added 9, test_skipped 15, trivial_assert 22
r12  score 21  the same four
```

`rustc --edition 2021` and `--edition 2024` compile `union-hide.rs`; the test build prints
`tests::skipped ... ignored`, `tests::checks_nothing ... ok`, `1 passed; 0 failed; 1
ignored`; `rustc/union_driver_2021` prints `first(&[]) panicked: true; later() panicked:
true`.

A second form needs no comment (`a/skip-hide.rs`): a closing bracket inside a string of a
`cfg_attr` condition.

```
#[cfg_attr(not(feature = "never)"), ignore)]      after a two-line string above it
r10  test_skipped 7     r11  nothing     r12  nothing
#[cfg_attr(not(feature = "never"), ignore)]       (a/skip-ctl.rs)
r10, r11, r12  test_skipped 7
```

`rustc --test` on `skip-hide.rs`: `tests::the_check_that_matters ... ignored`.

It is not a matter of new files: `a/mod-union.patch` is a hunk of an existing file with the
two-line string in its unchanged lines and `*v.first().unwrap/*""*/()` added; `v3r10`
reports `panic_path_added` on line 8, `v3r11` and `v3r12` nothing. Its control
(`a/mod-union-ctl.patch`, the hunk without the string's lines) is found by all three.

**Cause.** A line that leaves a literal open makes the next line be read twice, as code and
as resuming the literal, and `union_views` keeps a byte that either reading keeps. After the
string closes, the reading that did not resume takes the closing quote for an opener. From
then on the two readings are complements: one is inside a string exactly where the other is
in code. Every later line with an even number of quotes keeps them that way, so they never
meet again, to the end of the hunk. The harness shows it (`h/harness view rust
a/union-hide.rs`):

```
L "    *v.first().unwrap/*\"\"*/()"
V "    *v.first().unwrap  \"\"  ()"
L "    #[cfg_attr(not(feature = \"never)\"), ignore)]"
V "    #[cfg_attr(not(feature = \"never)\"), ignore)]"
```

The compiler's reading (the exact one) is in the union, so no code byte is blank and claim 1
of round 12 holds. What the union adds is tokens: two `"` marks between `unwrap` and `(`,
so `panic_path` finds no call; between `todo` and `!`; between `[` and `ignore`; between
`(` and `true`; and a `)` that closes the `cfg_attr` list before its `ignore`
(`Skips::follow`). A comment with one quote works once: the two readings meet again on that
line, so only the first construct is lost (`a/union-onequote.patch`: `v3r12` reports lines
9, 15 and 22 and not line 5).

**Why the fuzz target cannot see it.** `rust_code_view` checks PRECISION only "up to the
first line on which a literal runs over the line end". After that line it checks only that
code is kept. The reader's own fuzz target feeds it views, not the question whether a view
shows what is not code.

**How common the precondition is.** `h/harness multi` over the 18,676 Rust files of the
local crate sources: 1,283 files (6.9%) have at least one line read more than one way, and
576,750 of 7,589,723 lines (7.6%). In this repository: 31 of 157 files, 11.8% of lines. In
those stretches no added string is needed. An honest author is unlikely to put a quote
between the tokens of a construct or an unbalanced bracket in a `cfg_attr` string: on the
18,588 real files no file-level integrity signal that `v3r10` gives is lost by `v3r12`
(`sig-r10-r12.txt`). So this is a way to hide, not a common accident.

**The same cause costs precision, also in the released binary** (`a/union-fp.patch`): after
a two-line string, a comment `// the old code said "x.unwrap()" here` and a string that
names `Command::new`, `todo!()` and `unsafe { }` give `v3r12` four items and score 23; with
the string on one line, no item and score 9. v0.8.3 gives the two security items of the
four. `v3r11` and `v3r12` agree, so the commit did not change it.

**Declared?** No. `lexical.rs` and the changelog say that after a string over a line end
"what either reading finds code is code". Neither says that a construct can be broken by
it. Round 11's claim 2 ("found however its tokens are spaced or split") does not hold in
such a stretch, and round 12's claim 4 says round 11's claims stand.

**Fix, cheapest sound.** Unite findings, not views. The reader reads the exact view; where
more readings exist it reads the union as well; an item either finds is an item. For a file
the patch creates, and for any side that starts at line 1, the exact view is the compiler's
and that alone closes this finding. Tests: the four constructs of `union-hide.rs`, the
`cfg_attr` of `skip-hide.rs`, and `mod-union.patch`.

## IR5-M-1 (medium, reproduced): `k#` and `'r#` read differently on editions 2015 and 2018; one line blanks the rest of the hunk

**What happens.** `a/kraw-rest.rs` (11 lines): line 2 is `m!(k#r"\"); m!(" /* ");`, then a
process spawn, an `unsafe` block, an ignored test, a trivial assertion and an `unwrap`.

```
python3 ev.py a/kraw-rest.patch v083 r11 r12
v083  score 12  process_spawn 4, unsafe_added 5, process_spawn 11
r11   score 21  those and test_skipped 9, trivial_assert 10, panic_path_added 11
r12   score 4   (no item)
python3 ev.py a/kraw-rest-ctl.patch ...     (k r"\" with a space)
r12   score 21  the six items
```

`rustc --edition 2015` and `--edition 2018` compile `kraw-rest.rs` and the program prints
`spawned: true`. Edition 2021 refuses it: `error: prefix k is unknown`. The same with a raw
lifetime, `m!('r#r"\"); m!(" /* ");` (`a/rawlt-rest.rs`): editions 2015 and 2018 compile and
run it; v0.8.3 reports `process_spawn`, `v3r11` three items, `v3r12` nothing. One-line forms:
`a/kraw.patch`, `a/rawlt.patch` (the spawn on the same line is lost by `v3r12` only).

**Cause.** `rust::Scan::token` takes `k#name` as one name (the lexer's
`ForcedKeywordIdent`, which is in `ra-ap-rustc_lexer` 0.184.0) and `lifetime_or_char` takes
`'r#name` as one raw lifetime. Then the `r` is part of a name, the quote opens an ordinary
string, `\"` is an escape and the string runs to the next quote. rustc 1.99.1 on editions
2015 and 2018 reads `k`, `#`, `r"\"` and `'r`, `#`, `r"\"`: a raw string that ends at the
second quote. After it the view is one quote out of step, and ` /* ` in the next string
opens a comment that every reading carries.

**How this compares with what is declared.** Only `cr"…\"…"` is declared as the pre-2021
difference. That one is in v0.8.3 too and has the same strength (`a/craw2.rs`: compiles on
2018, a spawn between its two lines is hidden from v0.8.3, `v3r11` and `v3r12`). So these two
give an author on an old edition nothing `cr` did not give. They are still a change for the
worse in a commit meant for the released line: v0.8.3 reads both right on 2015 and 2018.
For `'r#` it is a trade (`a/rawlt21.rs`: on edition 2021 v0.8.3 and `v3r11` miss a spawn that
`v3r12` finds). For `k#` it is a loss only: no edition of rustc 1.99.1 reads `k#name` as
one token, the fuzz oracle does because it is a newer lexer than the compiler, and no unit
test holds the rule (see "Weak tests"). The README's known limits name no edition; the
changelog says the rules were "checked with rustc 1.99.1 in editions 2015 to 2024", which
is not true of this one.

**Fix.** Drop the `k` arm until a released compiler has it (and teach the oracle that).
For `'r#name`, `cr` and `c`, where editions disagree about where a literal ends on the
line, read the line both ways and keep what either keeps, as is done at a line start. Or
declare all three in the README with the `cr` case.

## IR5-M-2 (medium, reproduced): a raw identifier after white space past ASCII is not read as its name

**What happens.** `a/lrm-raw.rs`: `*v.first().<U+200E>r#unwrap()`,
`*Option::<U+200E>r#unwrap(v.first())`, `#[<U+200E>r#ignore]` and
`<U+200E>r#assert!(false);` inside the ignored test. `a/lrm-raw-ctl.rs` is the same without
the mark.

```
python3 ev.py a/lrm-raw.patch r10 r12
r10  score 13  new_file, hazardous_text, test_skipped, panic_path_added   items 2, 5, 10
r12  score 7   new_file, hazardous_text                                   no item
python3 ev.py a/lrm-raw-ctl.patch r10 r11 r12       all three: items 2, 5, 10
```

The same with U+0085, U+200F, U+2028 and U+2029 (`a/ws-nel-raw`, `ws-rlm-raw`, `ws-ls-raw`,
`ws-ps-raw`). With a vertical tab or a form feed the items are found. `rustc --edition 2021`
compiles `lrm-raw.rs`; the program prints `first(&[]) panicked: true`; the test build prints
`tests::skipped ... ignored`.

**Cause.** `without_raw_prefixes` (`evidence.rs`) blanks `r#` only when the byte before it
is not a name byte, and it counts every byte past ASCII as a name byte. The token reader
knows these five characters as white space (`rust_tokens::white`); this function does not.
So `r`, `#`, `unwrap` stay three tokens and `unwrap` has no `.` or `::` before it.

**What is left.** The file keeps `hazardous_text` (+3) for the invisible character, so a
reader is pointed at the file. The integrity items and their 6 points are lost. Not
declared; `v3r10` found them.

**Fix.** Let the scanner blank the prefix where it reads a raw identifier (it knows), or
make `in_name` pass over what `white` passes over. Test: the four lines above for each of
the five characters.

## Low

- **IR5-L-1, the unit tests do not hold a third of the rules I removed.** See "Weak tests".
- **IR5-L-2, a stale statement** (the same as Vesper's VD3R12-L-1): the head of
  `rust_tokens.rs` still says the code view "ends a Rust block comment at the first `*/`".
- **IR5-L-3, a small one in `7004510`:** the comment on `Found` says the only item that can
  come late is a `cfg_attr` list. A path call that reaches back
  (`Result::<[u8; {x.unwrap()}]>::unwrap` over two lines, `a/late2.patch`) can too. The map
  handles any late item, so the answer is right (items on lines 2 and 3); only the sentence
  is not.

## The four claims of round 12

1. **Read from a file's first line, no code byte is blank (edition 2021 and later): held,
   as far as I could attack it.** I compared `rust::Scan` with `ra-ap-rustc_lexer` 0.184.0
   function by function and found no difference. A battery of 59 marked lines (`t/ta.rs`,
   `t/tb.rs`, `t/tc.rs`: every literal form and prefix, escapes, line continuations,
   lifetimes and labels, numbers and suffixes next to quotes, raw identifiers, doc and
   nested comments, `/*/`, `//*`, names and white space past ASCII, emoji, strings, raw
   strings and comments over lines) compiles on editions 2021 and 2024 (`tc.rs`, the
   `#"x"#` forms, on 2021 only, as it must) and `v3r12` reports the marker on all 59
   (v0.8.3 and `v3r11` miss the nested comment line). A BOM, CRLF line ends and a NUL byte in a comment
   (`a/bomcrlf.rs`, `a/nul.rs`): compiled, and found. A 20 MB line: read. The stated
   exception is incomplete (IR5-M-1), and "no code byte is blank" is not "the rules see the
   code" (IR5-H-1, IR5-M-2).
2. **The other languages are byte-identical: held.** The diff of the shared scanner is the
   Rust branches and nothing else. Old and new `code_view` side by side on 3.6 million
   random lines in each of Python, JavaScript, Go, shell and YAML: no view and no carried
   state differs (`h/harness fuzz`). `v3r11` against `v3r12` on 30 random patches of those
   languages (9,000 hunks; scores, signals and the listed items): no difference.
3. **Cost is linear: held.** Two megabytes of each of 29 units, as one line and as many
   lines, rank in 0.03 to 0.09 s and 10 to 15 MiB on both binaries. Half a million lines
   that each leave a literal open: 0.36 s, 81 MiB, the same on both. A 20 MB line: 0.6 s,
   82 MiB. `r` and a million `#`: 0.03 s. The debug binary on twelve of them: no panic.
   Table: output of `cost.sh` (`cost.lst`).
4. **Round 11's claims stand with this input: one of them does not, and did not before.**
   Round 11's claim 2 is broken by IR5-H-1 and IR5-M-2, on `v3r11` as on `v3r12`. One
   answer: `--outline`, `--hunks` and `--cluster --parallel 4` agree with `--rank` on the
   patches I tried. The corpus: `eval --corpus fixtures/eval` on the archive gives 47 cases,
   precision 97/111, recall 97/139, declared 59, as stated.

## What the fuzz property cannot see: three inputs by hand

1. `a/union-hide.rs`: the view shows comment text as code after a literal over a line end.
   PRECISION is not checked there. The view is wrong for its readers (IR5-H-1).
2. `a/kraw-rest.rs`: the oracle is a lexer, not a compiler of an edition. It agrees with
   the view; rustc 2018 does not (IR5-M-1). The same holds for the shebang, which the
   compiler strips before the lexer (declared), and would for frontmatter (rustc 1.99.1
   refuses it as experimental: `a/front.rs`).
3. `a/lrm-raw.rs`: the view is right and the function applied to it afterwards is not
   (IR5-M-2). The property stops at `code_view`. A fourth, tried and right:
   `a/bomcrlf.rs` (the oracle does not strip a BOM or normalise CRLF; the view copes).

Also outside the property: the table check runs only from the seed `every_character`, the
local gate only compiles the fuzz crate, and the branches are pushed where no CI runs. So
on this branch the oracle has run only by hand.

## The constructions declared as not closed

Each still hides a `process_spawn` on v0.8.3 and on `v3r12` (rerun from `open-not-fixed/`).

| Construction | How easy on purpose | By accident | Cheapest sound fix |
| --- | --- | --- | --- |
| `/*` added in one hunk, `// */ code` in a later one | one line each | not seen: an honest closing line has no `//` before the `*/` | a second reading of a side that starts after line 1, begun as `Comment(1)`; it joins the plain reading at the first `*/` both see |
| `// */ code /*` inside an existing comment | one line | no | the same reading |
| a string opened in an earlier hunk, `// "; code` later | one line each | yes, see below | the true state from the file, when the review comes from a Git selection; for a bare patch, a declared limit |
| `#!… /*` as first line | one line, any edition | no (a real shebang holds no `/*`) | apply the compiler's shebang rule to a side that starts at line 1: the line carries no state |

**The string case is the one an honest author can hit.** `a/honest-string.patch`: a hunk
whose three unchanged lines are inside a multi-line SQL string, and whose changed line
closes the string and changes `.ok()` to `.unwrap()`. `v3r10` reports `panic_path_added` on
line 15; v0.8.3, `v3r11` and `v3r12` report nothing. With the string's opening line in the
hunk (`a/honest-string-ctl.patch`) all find it. The loss is the closing line only. How often:
of 2,648 hunks of modified Rust files in the last 400 commits of this repository, 17 (0.64%)
truly start inside a string (15 ordinary, 2 raw) and none inside a block comment; in none of
the 17 does the answer of `v3r12` change when the hunk is given a first line that opens what
is really open (`frag.py`, `frag.out`).

**My answer to question B.** Fix the shebang and add the comment reading: both are sound,
small and cost almost nothing in precision. But fix IR5-H-1 first, since a second reading
united byte by byte is exactly what IR5-H-1 uses. The string case cannot be closed the same
way: a reading begun inside a string never joins the plain one, and every string of every
later hunk would read as code. Declare it, and say that the closing line of a long string is
where it bites.

## The declared regressions of round 11

- A change that adds no token (a term deleted from a `cfg_attr` condition, comments taken
  from around code): accept as declared for now. It is a real weakening with no trace, so it
  should not stay: the old side can be read for a list that still stands on the new side.
- A function as a value only for `Result` and `Option`: accept.
- A call after `/* /* */ // */`: closed by this commit. The four nested-comment patches of
  the second review are found; 13 of 226 earlier patches change, as stated (my rerun:
  `repro-r11-r12.txt`; four found, nine lose a wrong `test_likely`).
- Any rewritten line inside a multi-line skip or assertion is reported: accept.

## Measured change in answers, `v3r11` against `v3r12`

- 18,588 real Rust files as whole new files (603 crates, 75,828 listed items; rank lists
  at most 32 for a file, scores and signals count all): no file differs in score, signals
  or listed items (`reg-r11-r12.txt`).
- The last 150 commits of this repository (278 items; 12 patches refused by both): no
  difference (`hist-r11-r12.txt`).
- `v3r10` against `v3r12` on the 18,588 files, file-level signals: `syntax_unchecked` is gone
  from 5,798 files, and `test_skipped` is found in 7 files where the parser missed it (all
  seven are real multi-line `cfg_attr(…, ignore)`). Nothing else.

So no new false positive and no lost item on real code. The false positives of IR5-H-1's
cause are old.

## Weak tests

I removed or changed 28 rules of the scanner one at a time and ran the unit tests of
`lexical.rs` (`rustc --test` on the file; `mut/mut.py`, `mut/mut.out`; 36 tests pass
unchanged). 17 changes fail a test. 11 pass all of them:

- no `k#` arm; no raw identifier arm; no raw lifetime;
- a lifetime does not start with a digit; nor with an emoji; an emoji does not go on in a
  name;
- `b'` read as a lifetime or character, not as a byte literal;
- `c"` not a string (the view is the same either way: equivalent);
- a raw string with more than 255 `#` takes a suffix;
- `r#` and a character that is no quote passes one byte, not one character;
- `0_` does not go on as digits.

Most of these the fuzz target would catch, and it is the real test. But the diff-scoped
mutation check that the project requires runs the unit tests, and the fuzz target is not
run by `ci_local.sh`. The three name tables are held by 27 characters in a unit test and
otherwise by the seed.

## Commit `9c20ea0..7004510`

Read and probed (`a/late.patch`: a `cfg_attr` list whose item comes late for line 1, with
other items on lines 1 to 5, and a second list): ten items, in line and signal order, none
twice, the same as `v3r11`. Old and new sides of a hunk keep separate states
(`a/sides.patch`: a comment opened on a removed line does not blank added lines, and the
reverse). I found nothing wrong but the sentence in IR5-L-3.

## Not looked at

- No fuzz run of my own (no `cargo`); I did not check the three Unicode tables against
  `unicode-ident`, nor against the Unicode version of rustc 1.99.1.
- The daemon, MCP and Git-selection paths; only `--stdin-patch` was run.
- The old side of a hunk that starts inside a literal; hunks of a renamed or copied file.
- Other Rust repositories: only this one has history I could read. The crate sources stood
  in as whole files.
- Rust reached without the `.rs` extension (`#[path = "x.txt"]`, `include!`): no rule reads
  it. Not this commit's matter.
- `agent/loop`, the non-Rust rules, and anything outside `d4dbc91..5810cb5`.

## Reproduction files

All under `/tmp/claude-ddf-indep-review-5/`: `a/` (sources and patches of every finding and
control), `rustc/` (the compiled programs), `t/` (the battery of valid lines), `h/` (the
harness: `main.rs`, the two copies of `lexical.rs`, the binary), `cost/` and `cost.sh`,
`mut/`, `frag.py` and `frag.out`, `reg.lst` and the three comparison outputs, `mk.py`
(source to new-file patch), `ev.py`, `cmp.py`, `sig.py`.
