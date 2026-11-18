# Eighth independent review of the integrity signals: commit `ceb2e6f` (2026-11-15)

Reviewer: Claude, independent (no part in the work). Brief:
`~/.local/state/habitat-claude/ddf-gap-probes-20261113/ddf-independent-review-8-brief.md`.
Binaries: the frozen `v3r15` release build (SHA-256 `44ba828c…`), `v3r14`, `v3r13`, and the
published v0.8.3 binary. I did not run `cargo` and did not build. Scratch:
`/tmp/claude-ddf-indep-review-8/` (notes `NOTES.md`, tools `tools/`, findings `f/`) and, for
bulk, `~/.cache/ddf-t/indep-review-8/`. `git apply` runs are in scratch repositories;
`rustc` 1.99 through the toolbox, single files.

## Verdict: GO-WITH-FIXES (do not merge `ceb2e6f` as it stands)

No high finding. I found nothing lower than the published v0.8.3 binary by any measure I
could take (score, signals, listed items at the bound, outline labels, refusals), in any
language, on 55,000 generated patches, 3,378 history patches and the committer's list.
I found no item that an ordinary `git diff` gains or loses against round 14.

Four medium findings stand, each reproduced. Two of them break a claim of the request as
it is written (claim 3, "no header number decides what is found", and claim 4, "cost linear
in time"); one of those two is also an answer LOWER than round 14, where the request says
nothing is lost. The other two are ways past the new edge rule and the joins that the list
of remaining ways does not have. All four have narrow fixes. The fixes change `evidence.rs`,
`rust_tokens.rs` and `views.rs`, so they need another read.

## New findings

### IR8-M-1 (medium; needs a writer who sets out to): the old number of one hunk takes a join away from two other hunks. Lower than round 14; claim 3 broken

`hinted_joins` keeps, for each position, ONE hunk that "ends before" it
(`ends_before.entry(start + lines.max(1)).or_insert(index)`). Since this commit a hunk with
no line on the side is also entered there (`lines.max(1)`), first come first kept. So a hunk
of added lines only, written earlier in the patch, takes the place of the hunk that truly
ends before that position, and the hunk that follows is joined to the wrong one. A second
form: a hunk with no old line that is written BEFORE the hunk it follows finds itself in
the table (`head(first, earlier) != index` is false) and gives up instead of taking the
next candidate. Round 14 entered only hunks with old lines.

Reproduction, `f/join/` (base file `repo/f.rs`):

- `decoy2.patch`: four hunks. Hunk 1 changes the last line (it must match the end of the
  file); hunk 2 is `@@ -4,0 +60,1 @@` with one added comment line (plain Git puts it at the
  end, after hunk 1, whatever its numbers); hunks 3 and 4 touch (`@@ -3,2 +3,2 @@` ends on
  the context line `debug_assert!(`, `@@ -5,3 +30,3 @@` changes `a < b,` to `true,`), their
  OLD numbers honest, the new number of the second false.
- `git apply --check -v decoy2.patch`: applies ("Hunk #2 succeeded at 11", "Hunk #4 succeeded
  at 5"). The result (`decoy2-result.rs`) compiles (`rustc` exit 0) and holds
  `debug_assert!(true, "a must be below b")`.
- `v3r14`: `f.rs 5 trivial_assert new 30 (hunk)`. **`v3r15`: `f.rs 2`, no item.** v0.8.3: 2.
- `decoy2-other-number.patch` is the same patch with ONE number changed, the old position of
  the hunk of added lines (`-4,0` to `-20,0`), a number plain `git apply` never reads: both
  patches give the identical file (same md5). `v3r15` finds the assertion with `-20` and not
  with `-4`. So a header number that is neither 0 nor 1 decides what is found.
- In bulk: 5 of 55,000 generated patches are lower in `v3r15` than in `v3r14` in the full
  answer (`PLAIN=1 tools/cmp8.py R14=… R15=…`; the five are kept in `f/join/gen-lower/`). I
  traced two by hand to this cause; the headers of the other three have the same shape (a
  hunk with no old line at the position of a join round 14 made). The request says
  "v3r14 → this commit: plain 0 lost".

What it is and is not: not lower than v0.8.3 (which joins nothing). It gives a writer no
power the declared way does not (make both numbers of the second hunk false). The doc
comment of `joined_texts` does name "a hunk whose position says falsely that it follows
another, before two that truly touch"; the request's list and the CHANGELOG do not, and
round 14 read these patches. The same table has a second gap that is not a regression
(`f/join/triple.patch`): three hunks whose old numbers all touch, the middle one of added
lines only: the third is not joined to the second on the old side, since the first already
holds the position (`v3r14` the same).

Fix: keep the candidates for a position in the order of the patch and take the first that
is not the hunk itself and is not yet followed (a hunk with lines before one without); a
hunk with no line that has been joined can then stand for the position in its turn, which
also closes `triple.patch`. Add the case to rule 6 of `rust_hunk_places` (it writes two
hunks only).

### IR8-M-2 (medium; an ordinary diff can do this only when very large, a writer can make one request cost minutes): a view of hunks by path or address is quadratic for a file with no known extension. Claim 4 broken

`hunk_signals_alone` calls `FileKind::readings(file)` for every hunk it is asked about, and
since this commit `readings` walks EVERY hunk of the file for a shebang when the name gives
no language (v0.8.3 and round 14 looked at the first hunk only). So `--hunks --path P`, and
`--hunks` with several addresses, cost (hunks shown) × (hunks of the file) for any file
whose extension is not in the table (`.md`, `.txt`, `.json`, `.toml`, `.lock`, `.c`, a
script with no extension, …). A file with an extension is untouched. Rank and outline are
not affected (they scan the file once).

Reproduction: `f/quad/repro.sh` (writes `bin/tool` and `bin/tool.sh` with N hunks of
`ctx / -a / +b / ctx`), measured in `f/quad/measured.txt`, one 16 MiB page:

| hunks | v0.8.3 | v3r14 | v3r15, `bin/tool.sh` | v3r15, `bin/tool` |
|---|---|---|---|---|
| 12,500 | 0.20 s | 0.21 s | 0.22 s | 4.17 s |
| 25,000 | 0.41 s | 0.42 s | 0.45 s | 16.24 s |
| 50,000 | 0.56 s | 0.56 s | 0.58 s | 41.42 s |

And `data/table` in 333,000 hunks of `-a / +b`, ONE page of 1 MiB (2,768 hunks):
v0.8.3 0.24 s, `v3r14` 0.28 s, **`v3r15` 25.89 s**; a page of 64 KiB (175 hunks) 1.98 s
against 0.21 s. Reading that file page by page costs about fifty minutes where it cost
half a minute. The daemon, `rpc` and MCP serve the same views.

Fix: read the kinds of a file once per view (beside `joined` in `Scans`) and pass them to
`hunk_signals_alone`, or keep them with the file. No test or fuzz target holds the cost of
`hunk_signals_alone` (it is called from `views.rs` only).

### IR8-M-3 (medium; needs a writer who sets out to): a cut beside the word that no edge reads

The CHANGELOG says: "So the construct is found when the cut stands beside its word,
whatever the numbers and the order of the hunks". Two forms where it is not:

**(a) The word alone in a hunk of its own.** The edge forms need the word with a neighbour
(`.unwrap` at the end, `unwrap(` at the start). Cut on BOTH sides, the word is neither.
`f/edge/alone.patch` (base `f/edge/repo3/w.rs`, which has two blank lines): three hunks,
`+    let v = opt.` / blank; `+    unwrap` / blank; `+    ();`, the second and third with
false numbers. `git apply --check -v`: rc 0 ("Hunk #2 succeeded at 5", "Hunk #3 succeeded
at 7"); result `alone-result.rs`: `rustc` 0 errors, the program panics (exit 101).
`v3r15`: `w.rs 2`, no item. Enumerated (`f/edge/cuts3.py`, 28 constructs, every pair of
cuts, 1,273 patches): 28 forms are missed in three hunks though each of their two cuts
alone is found: `.unwrap`, `.expect`, `::unwrap` (also with generic arguments), `#[ignore]`,
`#[ignore = …]`, `#![ignore]`, `#[allow(`, `process` / `::` / `Command`, and on the old side
`#[test]`, `#[rstest]`, `#[tokio::test]` (patches in `f/edge/cuts3/`).

**(b) The function passed as a value.** `Option::unwrap` and `Result::unwrap` as values are
read inside a hunk, and at the end of a side (`::unwrap`), but not cut before the word:
`Option::` at the end is no form and `unwrap` at the start needs `(`.
`f/edge/value.patch`: `+    let f: fn(Option<u8>) -> u8 = Option::` / blank, then with false
numbers `+    unwrap;`. `git apply`: rc 0; `value-result.rs` compiles and panics
("called `Option::unwrap()` on a `None` value"); `v3r15`: nothing. With honest numbers
(`value-honest.patch`) the join finds it.

The two-hunk enumeration (`f/edge/cuts.py`, 265 cuts): 23 missed; 11 are declared forms (an
assertion after its bracket, inside a `cfg_attr` list, `#[` then `allow(`), 2 are cuts
inside the generic arguments of a method call, and 10 are the value form (b), written
`Option::unwrap` and `Result::<u8, E>::unwrap`. With honest numbers all 265 are found by the join
(`cuts-honest.py`): the joins hold.

Fix or state: a side that is nothing but `unwrap` or `expect` is as rare in real code as
the built forms, and so is one that ends on `Option::` or `Result::`: in the 8.05 million
lines of the 603 crates no line is only `unwrap` or `expect` and none ends on `Option::` or
`Result::` (30 lines are only `ignore`, arguments of a `cfg_attr` list written over lines,
and 2 only `test`; those two words would have to be measured as the others were).
Otherwise the sentence in the CHANGELOG and the list of remaining ways must say "cut on
one side of its word, with its neighbour on the same side of the cut".

### IR8-M-4 (medium; needs a writer who sets out to, and `git apply --unidiff-zero`): a hunk of added lines put INSIDE an earlier hunk

The joins model hunks that follow one another. With `--unidiff-zero` Git puts a hunk with no
old line at its new position, and that position can be inside the lines of an earlier hunk
(nothing in an empty preimage has to match). `f/overlap/inside-u0.patch`:
`@@ -1,3 +1,4 @@` (two context lines, `+    let v = opt.unwrap`, one context line of code)
and `@@ -3,0 +4,1 @@` with `+    ();`. `git apply --unidiff-zero`: applied; the result
(`inside-u0-result.rs`) has `let v = opt.unwrap` and `();` on lines 3 and 4, compiles and
panics. No version reports it: the first side does not END on the word (its context line
follows), the second begins with `(`, and the join by numbers reads the second hunk after
the first's last line. (Plain `git apply` puts the second hunk at the end of the file,
rc 0, and the result does not compile. Plain Git also refuses a later hunk whose context is
a line of an earlier hunk, `f/overlap/overlap.patch`: so this needs the flag.)

The documents take that applier into account ("once for an applier that goes by the numbers
alone") and declare one limit that rests on it (`@@ -1,L` placed lower). This is a second
one and is not declared. Fix: also read a hunk of added lines at the place its new number
names inside another hunk's new lines; or declare it.

### Low findings

- **IR8-L-1 (needs a writer): spellings of the released paths that are still unread, in
  v0.8.3 too.** `<Command>::new("true")`, `Command::<>::new("true")`,
  `<TcpStream>::connect(a)`, `TcpStream::<>::connect(a)` compile and run (`f/paths/q.rs`,
  `rustc` exit 0, "ran"); with `Command` and `TcpStream` imported above the hunk no version
  reports them (`f/paths/q.patch`: 2, 2, 2). The reader passes over generic arguments for
  `unwrap` and not for these paths. The statement "no alias or glob import is modelled" does
  not cover it.
- **IR8-L-2 (statement): the memory figure is one shape's.** The request states 264 MB
  against 176 MB (1.5 times). I measured that (263 MB against 176 MB; rank; linear from
  200,000 to 499,990 hunks). Other shapes cost twice the release: 333,000 hunks joined by
  hints, old numbers one way and new numbers the other (`worst-333000.patch`, written by
  `f/cost/make.py`; figures in `f/cost/measured.txt`): v0.8.3 160 MB, `v3r14` 334 MB, **`v3r15` 319 MB**, 3.5 s
  against 0.8 s; a file of touching hunks with long context lines (51 MB): 112, 226,
  **226 MB**. Linear in every shape I tried; the constant is 2.0, not 1.5. Outline at a
  16 MiB page is 359 MB and hunks 537 MB in every version (the page, not the scan).
- **IR8-L-3 (cost of the edges; an ordinary diff can do this, rarely): what the built forms
  report falsely, and one construct twice.** A changed last line of a side that ends
  `| Kind::Command` (a pattern broken before `|`) is `process_spawn`; one that ends
  `self.done + self.todo` is `stub_added`; a changed first line `unwrap(x);` is a panic
  path (`edge/e1`, `e3`, `e4`). I scanned every line of the 603 crates (8.05 million lines,
  `edge/scan_ends.py`): 1 line ends on a built form (`*ureq`), 58 begin with a call of an
  own function named `expect` (`indexmap`'s test helper among them), 24 with `ignore =` (20
  of them true: the argument of a `cfg_attr` over lines). So the stated cost holds. A
  construct cut and also read whole is two items: `process::` at the end of one hunk and
  `Command::new(` in the next is `process_spawn` on both lines (`edge/e6`); a removed
  `#[tokio::` / `test]` is `test_removed` in both hunks (`f/fuzzgap/old-edge.patch`).
  Harmless to the score.
- **IR8-L-4 (documents): the list of remaining ways.** Missing on the "needs a writer"
  side: IR8-M-1, M-3, M-4 and L-1 above. Missing on the "ordinary diff" side: a script with
  no extension whose hunks do not show line 1 has no language at all, so an added
  `curl … | sh` and `eval` are not reported (`f/lang/noshebang.patch`: 2 in every version;
  an ordinary `git diff -U3` of line 20 of such a script). And `eval (source)`,
  `os.system (command)` with a space before the bracket (`f/lang/eval-space.patch`) are
  listed under "needs a writer": code that no formatter has touched writes them; I would
  list them under ordinary.
- **IR8-L-5 (tests): what `rust_hunk_places` cannot generate.** Rule 6 writes two hunks,
  never a third that names the same position (IR8-M-1 lives there; `f/join/decoy2.patch`
  is an input it would not catch). Rule 7 cuts a construct once, on added lines, from a list
  of 13 forms: never twice (`f/edge/alone.patch`), never on the old side (if the old-side
  branch of `waiting` were deleted, no rule of the target would fail:
  `f/fuzzgap/old-edge.patch`), never the value form. Rule 8 has one shebang, on a context
  line, of two languages: never on the old side only, never two languages
  (`f/fuzzgap/two-shebangs-old-side.patch`), never with more than 32 items. No target holds
  a cost, and none calls `hunk_signals_alone` (IR8-M-2).

## The claims of the request

1. **Monotone in the full answer; the list keeps v0.8.3's items.** Against v0.8.3: held on
   everything I ran. Against round 14: **broken** by IR8-M-1 (5 of 55,000 generated patches;
   none on history).
2. **Never lower than v0.8.3.** Held. 55,000 generated patches (`tools/gen.py`: Python, JS
   and TS, Go, shell, `Dockerfile`, `Makefile`, YAML and CI workflows, `package.json`,
   `build.rs`, `setup.py`, vendored and test paths, Rust, and files with no extension with
   one, two or no shebang on old, new and context lines; honest and false numbers; one to
   five hunks; shuffled hunks; two sections for one path; creations and deletions; files
   of up to 300 changed lines, for the bound): score, signals, listed items by address, side and
   line, outline labels and refusals compared (`tools/cmp8.py --views`): 0 lower. History:
   579 commits of this repository at `-U3`, `-U0`, `-U1` and `-U10` and 531 commits of
   forge-warden at `-U3` and `-U0`: 0 lower. The committer's list (5,699 of its 5,765 files
   still exist), rank only, with the committer's `lost.py --strict`: one line, the patch
   `two-sections-create.patch`, which is the two entries of one path changing places as
   the request says (I read both answers); no refusal differs. Round 14 to this commit on
   that list: plain 0 lost, strict 1 (`bound-lang.patch`, the declared trade).
3. **No header number decides what is found but old position 0 or 1.** **Broken** by
   IR8-M-1 as written (one old number of a third hunk). The language rule held: I found no
   number that changes the language readings, and no second language that takes anything
   away (the readings are united; the mark of the release is set under the release's kind
   only; path kinds are the same under every language).
4. **Rounds 11 to 14 stand; cost linear in time; memory as stated.** Time: **broken** by
   IR8-M-2 for views by path. Rank and outline are linear on every shape I tried. Memory:
   linear, the figure stated is accurate for its shape and low for others (IR8-L-2). "No
   view is a mixture", "one answer, no clock": not tested.
5. **The corpus says what the binary does.** Held: `v3r15 eval --corpus fixtures/eval` on an
   archive of the commit prints 50 cases, 158/179, 158/213, declared 79, which are the
   constants of `crates/deep-diff-forge-cli/tests/eval.rs` (882,681 and 741,784 ppm, 79).

The seventh review's findings, rerun on `v3r15`: `shebang-lower` 9, `ooo` 9, `html` three
items, `sp1`, `sp3`, `sp4`, `sp7` found, `cutpair` found, `twosec` found at the edge, `pq`
and `qp` found, `bound-lang` lists the secret. Closed as the request says.

## Views against the rank

Outline, `--hunks`, `--hunks --path P` for every path and `--hunks --address A` for every
address agree on every hunk's labels (`tools/viewcheck.py`, 4,000 generated patches, 0
differences); the outline's score and labels agree with the rank (55,000); `--cluster
--parallel 4` prints the rank's lines (900). `rpc` and MCP not run.

## Git, as I found it (2.56.0)

Every sentence about `git apply` that I checked holds (`git/`, repository `git/r1`):
`@@ -4,3 +1,4 @@` and `@@ -4,3 +9,4 @@` on a script's first three lines go to line 1 in
silence or with "succeeded at 1 (offset -8 lines)"; `@@ -1,0 +2,1 @@` is refused by plain
Git and put after line 1 by `--unidiff-zero`; `@@ -2,0 +3,1 @@` then `@@ -3,1 +4,1 @@` is
refused plain and applied together with `--unidiff-zero`; `@@ -0,0 +5,1 @@` on a file with
lines is refused plain and put at line 1 with the flag; `@@ -1,2 +5,3 @@` whose lines are
at 5 is refused plain and applied at 5 with `--unidiff-zero` and with `-C0` (the declared
limit is accurate); a hunk with no trailing context line must match the end. One thing the
documents do not say and that costs nothing: on an EMPTY existing file a hunk of added
lines applies at line 1 whatever both numbers are (`@@ -7,0 +9,2 @@`); the code reads it as
a fragment, which loses nothing the top reading has.

The two-section dependency: a modification followed by a rename of the same file does not
continue it (Git gives the rename the ORIGINAL file; `f/twosec/modren.patch`), and Git
refuses every other spelling of the path I tried (`./a.rs`, `/a.rs`, `x/../a.rs`, `a.rs/`).
I found no two-section form that the statement does not cover. Stating and pinning it is
enough for a branch that does not merge before the rebase, provided the release track's
refusal compares paths as Git does.

## The declared limits, one by one

- A construct shown in part, cut away from its word (assertion after `(`, `cfg_attr` list,
  `#[` then `allow(`): accurate; ordinary is right. The sentence "found when the cut stands
  beside its word" is not accurate (IR8-M-3).
- A change that only removes tokens; a name a macro builds or an alias binds; generic
  arguments over 64 tokens; a lint inside `cfg_attr` or written `expect(…)`; `Err(_) => {}`
  spaced: accurate as far as I looked; not attacked further.
- A zero-hash raw string closed after a backslash; a raw string past four records: by
  reading the rule I found no way to make it miss below five records (the closing quote
  always sets a record, and the first four are read). Five patches of my own (`f/raw/`: a
  side that begins inside a raw string whose text holds lower records, with repeats, the
  code after it on a changed line) are all found; round 14 misses four of them. Marked
  rightly.
- A string start with nothing carried over a line end; two or more comments deep: not
  attacked again; marked as deliberate, which earlier measurements support.
- Two hunks that touch while neither number says so, cut away from the word: accurate, but
  not the whole class (IR8-M-1: the numbers say so and another hunk takes the place;
  IR8-M-3: cut at the word; IR8-M-4: inside another hunk).
- A second section for one path: accurate.
- `@@ -1,L` placed lower by `--unidiff-zero` or `-C0`: accurate (run above).
- The other languages' one-space forms: accurate; I would mark `eval (x)` ordinary
  (IR8-L-4). The line-end forms (`exec.` and `Command(` on two lines) fall under "one
  line a rule", listed as ordinary: right.
- At the bound, a newer signal with no item in the list: accurate.

**The option not taken** (a reading that hands each line to the rules as the lexer makes
it, so that no view of a joined run is held): I would not require it before a merge. The
cost is a constant of two on memory that is linear and bounded by the input budgets
(319 MB at the worst I found), and the rewrite touches the plumbing of every reading in a
branch where each such change has cost a review round. State the factor of two instead of
one shape's 1.5.

## False positives of the round

Every item `v3r15` reports and `v3r14` does not, on per-commit patches of this repository
(579 commits, `-U3`, `-U0`, `-U1`, `-U10`) and of forge-warden (531 commits, `-U3`, `-U0`):
**none**, at the level of listed items and of every hunk's labels (`tools/itemgain.py`,
`tools/gains.py`); nothing lost either. The tools do see differences (v0.8.3 to `v3r15` at
`-U3`: 2,755 items gained, 0 lost). The edge figure by another method: see IR8-L-3. The
stated cost of the language rule (a text file whose hunk begins on a shebang-looking line):
in the trees I looked at, such a line below line 1 of a file with no known extension occurs
3 times in 38,696 lines of this repository (2 in `docs/MAKING_SKILLS.md`) and 0 times in
1.73 million lines of the crates' other files; a hunk that begins on it makes the whole
file a script, so every `curl`, `wget` and `eval` on a changed line of any of its hunks is
an item.

## What I did not look at

- No `cargo`: no plant in any test, so "would a unit test fail if the rule were removed" is
  answered only by reading (the tests exist for each new rule; I ran none).
- `rpc`, MCP and the daemon; `--cluster` only for equality with the rank.
- The Rust scanner itself, the released view (`legacy_rust.rs`), the further starts and
  the hint `test_likely`: earlier reviews; I only ran them in bulk against v0.8.3.
- Editions before 2021, frontmatter, Unicode.
- On the committer's list of 5,765: 66 of its files no longer exist, and on the 5,699 that
  do I compared the rank only (results `all5-lost-strict.txt`, `all5-lost-r14-plain.txt`,
  `all5-lost-r14-strict.txt` in my scratch). My own comparison with the views did not
  finish on that list in 25 minutes (it holds patches made to be slow, and my tool first
  hung on a patch that names one path many times); the views were compared on the
  generated and the history patches. Lines of `all5-v083-r15.txt` that read
  `B:HUNKS-vs-OUTLINE` are my tool reading one 16 MiB page of a longer view, not a
  difference.
- Fuzzing: none run by me.
