# Eleventh independent review of the integrity signals (commit af74308), 2026-11-16

Reviewer: a Claude agent with no part in the work (started 2026-11-16 12:12 UTC, report
written 2026-11-16 12:33 UTC). Brief:
`~/.local/state/habitat-claude/ddf-gap-probes-20261113/ddf-independent-review-11-brief.md`.
Commit under review: `af74308` (branch `agent/integrity-signals`), read from a `git archive`
(`aa4acb7..af74308` read whole for `rust_tokens.rs` and `evidence.rs`). Binaries, frozen,
not built: this commit `review-bin/v3r18/release` (SHA-256 `22d4d7e0c4fe160b…`, checked),
round 17 `v3r17` (`3d843af0…`), round 14 `v3r14`, round 16 `v3r16` and 15 `v3r15` for one
bisection, and release v0.8.3 (`4cafd0ad…`). No `cargo` was run; `rustc` 1.99.1 and Git 2.56.1
in the toolbox and on the host. Notes, tools and every reproduction:
`/var/home/louranicas/.cache/ddf-t/indep-review-11/notes/` (`NOTES.md`; patches under `f/`;
tools under `tools/` and `tools10/`; results under `../out/`; compiled programs under `../rs/`).

## Verdict: GO-WITH-FIXES

Nothing high, nothing medium. Two lows that need a writer who sets out to, one low in the
documents, and some weak spots in the tests. Every comparison I ran agrees with claims 2 and 3:
nothing lower than round 14 beyond the declared bare `ignore`, nothing lower than v0.8.3
beyond the second section for one path, and nothing lower than round 17 beyond `.unwrap::` /
`.expect::` at the end of a side. The end of a side holds the one rule on every set I made
and on 70,924 cuts of real lines. The rule's own sentence also claims the BEGINNING of a side,
and there one form is still read (IR11-L-1).

## Findings

### IR11-L-1 (low; needs a writer who sets out to): a side that begins with `<Command>::new(` is read as a spawn, though what the side does not show can make it `Vec::<Command>::new()`

Claims made false:

- head of `rust_tokens.rs` (new in round 18): "**A hunk side is read as text that ends
  there; nothing is inferred from how it ends**, and nothing from how it begins."
- README (round 18): "A construct of which a side shows only the beginning is not read, and
  neither is one of which it shows only the end." and, of the bracketed spelling,
  "`Vec::<Command>::new()` is another path and is not [read]".

`released_path` opens a bracketed path when the `<` "follows no name and no `:`". At the
first token of a side nothing is before the `<`, and `opens` takes that as open
(`open.checked_sub(1).is_none_or(…)`). The unseen line above can end with `::`:

- `rs/begin/new.rs`: `let v = Vec::` and on the next line `<Command>::new();`. rustc 1.99.1
  compiles it (one unused-import warning); it prints `0` and starts no process.
- `f/begin/vecturbo-u0.patch`: the real `git diff -U0` of `<Child>::new();` changed to
  `<Command>::new();` on that second line (scratch repository under `../gw/`;
  `git apply --unidiff-zero --check -v` passes and the result is `new.rs` byte for byte).
- `python3 -P -E notes/tools/r.py f/begin/vecturbo-u0.patch V083 R14 R17 R18`:
  v0.8.3 and round 14 `score=2 sig=[]`; round 17 and round 18
  `score=6 sig=['process_spawn'] ev=[('process_spawn', 'new', 4, 'lexical')]`. The same with
  `-U3` (`vecturbo-u3.patch`): no item in any version. Bisected: rounds 14 and 15 none, rounds
  16, 17 and 18 report it (it came with the bracketed spellings).

This is one false item, new against the release, not new in round 18. Class: a form read at
an edge of a side. This is the beginning edge, which the round's sentence now claims. It
needs a writer, because no formatter breaks a line between `::` and `<`. I ran
`tools/cutscan.py`: 4,000 real lines (the tenth review's `scan-a` list) with a known word, cut
between every two tokens, each part put as the last line or the first line of a side. Of
70,924 cuts, none reports a signal that the whole line does not (`out/cutscan-7.txt`). A
control with `Vec::<Command>::new()` and `make::<Command>::new(x)` finds both
(`rs/cutctl/`).

Recommendation: narrow the sentence rather than grow the code. For example: "nothing is
inferred from how a side ends; at its beginning a `<` that nothing precedes opens a path,
and a turbofish whose `::` stands on the line above is then read as the bracketed spelling
(a writer's false item)". Declare it with the other writer's limits. Making `opens` false at
a side's first token would lose `let c =` / `<Command>::new(…)` written over two lines in a
`-U0` hunk, which rounds 16 to 18 report.

### IR11-L-2 (low; needs a writer who sets out to): `&mut <Command>::new(…)` spawns and is not read; the keyword list leaves out `mut`

Claim made false (README, round 18): "A bracketed spelling is read where its `<` follows a
mark … or a keyword after which an expression begins (`return`, `match`, `in`, `break` with
or without its label, `yield`, `if`, `while` …); after any other name the `<` is a comparison
or a generic list and is not read." After `&mut` an expression begins and the `<` is neither
a comparison nor a generic list. The doc of `Word::begins_expression` gives the same list as
complete.

- `rs/kw/mutref.rs`: `let ok = run(&mut <Command>::new("touch").arg(…/SPAWNED1));` and
  `let c = &mut <Command>::new("touch");`. rustc 1.99.1 compiles it cleanly. Run, it
  created `rs/kw/SPAWNED1` and printed `true true`.
- `f/kw/mutref-u0.patch`, `-u3.patch` (real `git diff`): v0.8.3, rounds 14, 17 and 18 all
  `score=2 sig=[]`.

Not lower than any version (none reads it). Class: the keyword list (IR10-L-2's class). This
is the second finding in that class, so the class rule applies. Either one rule closes it:
a `<` opens a path after any reserved keyword that cannot end an expression or a path, which
is every keyword except `self`, `Self`, `super`, `crate`, `true`, `false` and `await`. Or the
claim is narrowed to "after these seven keywords". Adding `mut` alone is under 20 lines with
no new state. `&raw const` / `&raw mut` were not tested. Narrowing is acceptable here: the
bracketed spelling itself is a writer's.

### IR11-L-3 (low; documents): two sentences that round 18 made false

- README, `rank.v0` paragraph: "A further reading only adds: … everything the joins and the
  views of an earlier round of this branch found. That is not true of every change between
  two rounds, and the two that took something back say so in the changelog: round 16 …;
  round 15's hints …; and round 17 …". Round 18 also took something back: `.unwrap::` and
  `.expect::` at the end of a side, read in rounds 15 to 17. On my sets 224 patches are lower
  than round 17 for that reason alone (below). The list should name round 18, and "the two"
  now introduces three, four with round 18.
- CHANGELOG, round 18 entry: "`#[ignore(`, `#![ignore(` and `ignore(` … are read as
  `test_skipped` on the whole text, … (never at an edge)". A side that ends on `#[ignore(` is
  a skip (`f/q/endparen.patch`: round 18 `test_skipped` on line 3, round 17 none). That is
  right by the README ("With the token that ends the doubt each construct is read anywhere,
  at the end of a side too (… `#[ignore(` …)"), but the parenthesis says the opposite.
- A smaller one: "peak memory within 1 MiB on every input". On my run of the 51 inputs,
  five were 2 MiB higher (`items-frag`, `items-new`, `items60-new`, `depthcode-2048k`,
  `rhash-newfile-oneline`). Every input was 1 to 2 MiB higher, a fixed amount (a larger
  binary). The CHANGELOG's own second run says "within 2 MiB". Say 2 MiB.

## The claims of the request

1. **No form is read at the end of a hunk side**: held for the end. By reading:
   `waiting`, `Found::edge` and `end` are gone. Every rule (`panic_path`, `trivial_assert`,
   `stub`, `ignored`, `test_attribute`, `released_path`, `allowed`, `Skips`) fires only on
   tokens it sees. None fires on the absence of a following token except the standard
   function as a value (`Result::unwrap`), which every continuation keeps. On my sets every
   item lost against round 17 is an edge `.unwrap::` / `.expect::`. On 70,924 real-line cuts,
   no cut adds a signal. **Broken for the beginning**, which the head's sentence also claims
   (IR11-L-1). The start readings of `evidence` (from inside a string, from a raw string) are
   chosen from what the side carries over any line end, its last included. That is a choice
   of reading, declared with its false items, not a construct read at the edge. Still, "nothing
   is inferred from how it ends" reads more broadly than the code does. A clause would help:
   "(the further starts are chosen from what the side carries over its line ends, the last
   one too)".
2. **Monotone against round 14 except the bare `ignore`**: held. 58,198 patches (below),
   plain and strict. 185 lower: 182 where a side, or a joined text, ends on `ignore` in an
   open `cfg_attr` list, and 3 with two sections for one path. No refusal differs.
3. **Never lower than v0.8.3 except the second section for one path**: held. 277 lower,
   all with a path in two sections.
4. **`#[ignore(…)]` in every listed spelling is a skip; `#[ignore[x]]` is not read**: held.
   Compiled with `rustc --test -A ill_formed_attribute_input` and run (`rs/ign/`), and
   ranked (`f/ign/`, `f/q/`):
   - Ignored by rustc and read by DDF: `#[r#ignore]`, `#[r#cfg_attr(all(), r#ignore)]`,
     `#[cfg_attr(all(), allow(unused), ignore(y))]`, `#[ignore("why")]` and
     `#[ignore(a = "b", c)]`.
   - Read by DDF: `#[ignore/**/(x)]`, `#[ignore ( "x" )]`, `#![cfg_attr(all(), ignore(x))]`,
     `#[cfg_attr(all(), r#ignore(x))]` and `#[cfg_attr(all(),ignore(x),)]`.
   - Hard errors, so no spelling is missed: `#[ignore{x}]`, `#[ignore[x]]`,
     `#[cfg_attr(all(), ignore{x})]`, `#[cfg_attr(all(), ignore[x])]` ("wrong meta list
     delimiters"), `#[ignore = x]` ("attribute value must be a literal") and
     `#[core::prelude::v1::ignore]`.
   - No false skip: `my::ignore(x)`, `foo(ignore(x))`, `ignore::case(x)` in a list, and
     `cfg_attr(ignore, …)`. A derive helper attribute named `ignore` (`rs/helper/`, a
     proc-macro crate built with rustc) is an ambiguity error (E0659) at its use, so
     `#[ignore(…)]` on a field is no honest false skip.
   - Joins: `#[ignore` / `(not_ready)]` and `#[cfg_attr(all(), ignore` / `(flaky))]` as two
     `-U0` hunks with true numbers are found (`f/q/joinparen.patch`, `joincfgparen.patch`).
     The second was round 14's by its edge and is round 18's by the join.
5. **Cost and memory as stated**: held.
   - The 51 inputs (`out/cost51.txt`, best of three, one process at a time): 49.30 s for
     round 17 and 49.67 s for round 18 (+0.75 %). Memory as in IR11-L-3.
   - The inputs that looked slower, rerun alternating: `items-new` 0.83 / 0.82 s,
     `items60-frag` 4.33 / 4.32 s, `pairs-330k` 3.02 / 2.97 s (best of nine), `shebangs-100k`
     0.75 / 0.74 s. They were noise; other agents held the load at 6 to 11.
   - My own shapes: 200,000 lines of `ignore(…)` lists, 200,000 lines of `if <Command>…` /
     `break 'l <TcpStream>…`, and one line of 600,000 `return <Command>::new(x);`. Round 18
     costs up to 28 % more time and 26 % more memory on them than round 17, because it finds
     an item on every line where round 17 finds none. The same shapes with forms both find
     (`ctl-*`) cost the same: 0.69 / 0.69 s, 0.83 / 0.83 s and 0.52 / 0.51 s, within 1 MiB.
     So the cost is in the answer, not in the reading.
6. **The corpus and the documents say what the binary does; the limits are complete**:
   - Corpus: held. `eval --corpus fixtures/eval` on the archive gives 53 cases, precision
     171/194, recall 171/252, findings 33/33 and 107 declared, as stated.
   - Documents: broken in three sentences (IR11-L-3).
   - Limits: incomplete by IR11-L-1 and IR11-L-2.

Bulk sets (`out/bulk.jsonl`, all four binaries on each patch). Generators are the tenth
review's (`notes/tools10/`), with new seeds:

| Set | Patches |
| --- | ---: |
| `realdiff.py`: real `git diff --no-index` patches at `-U0`, `-U1` and `-U3` of real files (the `scan-b` list) with rustfmt-shaped construct edits, seeds 201 and 202 | 20,000 |
| `jgen.py` 41 | 20,000 |
| `jgen.py` 42, honest | 10,000 |
| `gen.py` 941 (Rust) | 8,000 |
| History at `-U0`, `-U1` and `-U3` of four Rust repositories no earlier review used (`the_maintenance_engine_v2`, `herdr-engineering-engine-v5`, `devops_engine_v2`, `the_code_synthor_v7`; read-only) | 198 |
| **Total** | **58,198** |

Against round 17, 224 patches are lower, all `panic_path_added`. In 170 a side ends on
`.unwrap::` / `.expect::`; in 54 a joined text does. Two of them are also lower than round
14, by the bare `ignore`. No loss in any comparison is on a real `git diff` or a history
patch. The generated sets are deleted; the seeds regenerate them.

## The declared limits

- **The bare `ignore` at a side's end (ordinary)**: true of the binary. `cfgu0.patch`:
  round 14 skip, rounds 17 and 18 and v0.8.3 none. The bulk losses against round 14 are this
  one. Marking it ordinary is right. It needs a `-U0` hunk whose `)]` is unchanged; `-U1` and
  `-U3` show it. When the next hunk has a true number, the join finds the `ignore` with `(`,
  `)` or `,` after it.
- **IR10-L-1 (created file, later hunk at `-0,0`; writer)**: true. `newtwotop.patch` is
  nothing in any version. `newtop-natural.patch` is the false item in rounds 14 to 18. Both
  are declared, in the corpus and in a test.
- **IR9-M-1 and the two-false-numbers miss**: the join code is unchanged in this round (the
  `evidence.rs` diff is documents and tests). The bulk's R17-only items are exactly that miss
  for `.unwrap::`. Right as a writer's limit.
- **Missing from the list**: IR11-L-1 and IR11-L-2. A `-U0` hunk that changes the first line
  of a turbofish that some writer broke over lines (`.unwrap::<` and the arguments below) is
  covered by the general ordinary limit ("a construct of which the hunk shows only a part is
  not read"). rustfmt keeps a turbofish on one line (checked, `rs/fmt/`), so a hunk ends on
  `.unwrap::<` only by hand.

## Weak tests

- Fuzz rule 16 cuts only the LAST line of a side. It has no mirror for the first line, which
  the head's sentence also claims, and `what_a_side_begins_with_is_no_item` has no bracketed
  spelling. That is how IR11-L-1 stands since round 16.
- `#[ignore(…)]` and `ignore(…)` in a list are not among the 28 constructs of the 265-cut
  count. Their join is held only by the fuzz target's joined constructs, which were not run
  here. My two patches above show it on the binary.
- No negative test for `#[ignore{x}]` or `cfg_attr(…, ignore{x})`. Harmless: both are hard
  errors.
- No test or plant for a keyword the list leaves out (IR11-L-2). The plants cover each
  listed keyword, a label, `else` and `await`.
- Rule 16 catches a re-read edge form only where its list has a whole line that is no
  construct (`value.unwrap:: NOT_A_CALL`). For the other listed forms the whole line is the
  construct, and the subset property holds even with the plant.
- `evidence_scan` was not fuzzed on the final tree, by the implementer's own note (the build
  slot was held). I could not run it either.

## What I did not look at

- No fuzz target was run and no plant was rebuilt (no `cargo`). I read the implementer's
  controls in `round18/ctl/` and did not rerun them.
- The daemon's `views.rs` (`--hunks`, `--outline`) was not compared with `--rank`; it is
  unchanged in this round.
- Languages other than Rust only as far as `gen.py` reaches.
- The `-U0` history of this repository and forge-warden: the tenth review ran them on round
  17, and the code paths that differ are the four in `rust_tokens.rs`.
- `&raw const` / `&raw mut` before a bracketed path.
- No command was refused by the permission layer. One tool call was refused: writing a
  report draft into my notes directory (a rule against report files from subagents). I did
  not retry it; this file is the one the brief names.
