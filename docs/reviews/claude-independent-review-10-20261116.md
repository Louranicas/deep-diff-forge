# Tenth independent review of the integrity signals (commit aa4acb7), 2026-11-16

Reviewer: a Claude agent with no part in the work (started 2026-11-16 03:04 UTC, report
written 2026-11-16 03:21 UTC). Brief: `~/.local/state/habitat-claude/ddf-gap-probes-20261113/ddf-independent-review-10-brief.md`.
Commit under review: `aa4acb7` (branch `agent/integrity-signals`), read from a `git archive`.
Binaries (frozen, not built): this commit `review-bin/v3r17/release` (SHA-256 `3d843af0…`,
checked), round 16 `v3r16`, round 14 `v3r14`, release v0.8.3. No `cargo` was run. Notes,
tools and every reproduction: `/var/home/louranicas/.cache/ddf-t/indep-review-10/notes/`
(`NOTES.md`; patches under `f/`; tools under `tools/`; results under `../out/`).

## Verdict: GO-WITH-FIXES

Nothing high. One medium that needs a writer who sets out to (a test switched off by a form
the round's own documents say the compiler refuses), four lows, and weak spots in the tests.
Every comparison I ran agrees with the round's claims 1 and 2: nothing lower than round 14
beyond the declared `ignore`, nothing lower than v0.8.3 beyond the known second section for
one path. No false item on an honest diff was found: 19.1 million lines of real Rust as
one-line hunks give no edge item, and the one form left at an edge is sound except after a
range (`..unwrap::`), which needs a writer.

## Findings

### IR10-M-1 (medium; needs a writer who sets out to): `#[ignore(…)]` switches a test off and is read by no version; the documents say the compiler refuses it

The CHANGELOG entry ("`#[ignore(…)]` is refused by the compiler and is not a form of the
attribute") and the doc comment of `waiting` in `rust_tokens.rs` (line 805: "`#[ignore(…)]`
is not a form of the attribute: the compiler refuses it") are false. rustc 1.99.1 reports
`#[ignore(reason)]` through the lint `ill_formed_attribute_input`: deny by default, part of
`future_incompatible`, and its own message says "this was previously accepted by the
compiler". A lint can be allowed. With it allowed the test compiles cleanly and does not run:

- `f/ign/a.rs`: no allow: `error: valid forms for the attribute are …` (the lint, at deny).
- `f/ign/b.rs`: `#![allow(ill_formed_attribute_input)]`: compiled with `--test`, run:
  `test skipped ... ignored`.
- `f/ign/c.rs`, `rustc --edition 2021 --test -A ill_formed_attribute_input` (the flag Cargo's
  `[lints.rust]` table passes; Cargo itself was not run): `#[ignore(not_ready)]`,
  `#[ignore()]` and `#[cfg_attr(all(), ignore(x))]`: `3 ignored`. (`#[ignore[x]]` is a hard
  error: "wrong meta list delimiters".)
- `f/ign/ignparen.patch`: a real `git diff` (scratch repository `repo/ign`; plain
  `git apply --check` applies it) that adds `[lints.rust] ill_formed_attribute_input =
  "allow"` to `Cargo.toml` and puts `#[ignore(not_ready)]` on one test and
  `#[cfg_attr(all(), ignore(flaky))]` on another. The result compiled as above: 2 ignored.
  `python3 notes/tools/r.py f/ign/ignparen.patch`: v0.8.3, round 14, round 16 and round 17
  all give `tests/it.rs score=1 sig=['test_only']`, no `test_skipped`; `Cargo.toml` only
  `config_or_lockfile`.

Not lower than v0.8.3 (it reads none of it). Severity: it is the construct the signal is
for, two short lines, compile-clean, with the lint written where no Rust rule reads it; and
the documents give its impossibility as a fact. Fix: read `#[ignore(` (and `ignore(` as an
argument of a `cfg_attr` list after the condition) as a skip, or declare it; correct both
sentences.

### IR10-L-1 (low; needs a writer): a created file in several hunks at old position 0 is joined in the order written, the reverse of the only applier that takes it (IR9-M-1's family, not declared for created files)

`placed_at_top` says: "In a section that creates a file the first hunk is [the top],
whatever its header says, and no other", and the head of `rust_tokens.rs` says a file
created as several hunks is one text. `git apply` (Git 2.56.1 here) refuses a created file
with two hunks at `-0,0`; `git apply --unidiff-zero` applies it and stacks the later hunk
ABOVE the earlier one, as for an existing file.

- `f/top/newtwotop.patch` (new file; `@@ -0,0 +3,3 @@` with `()`, `}`, `fn main`, then
  `@@ -0,0 +1,2 @@` with `pub fn boom(…) {` and `opt.unwrap`): `--unidiff-zero` gives
  "Hunk #1 succeeded at 1 (offset -2 lines). Applied patch m.rs cleanly", the result
  (`f/top/newtwotop-result.rs`) holds `opt.unwrap` / `()`, compiles and panics (exit 101).
  Every version scores 4 (`new_file`), no `panic_path_added`.
- `f/top/newtop-natural.patch` (the two in the other order): Git's result has no call
  (`()` above `boom`), and rounds 14 to 17 report `panic_path_added` (score 7): a join Git
  does not make.

The README's declaration names "two or more hunks at old position 0 of an existing file";
the fuzz (rules 1 and 14) never writes a created file this way. Fix: declare it with
IR9-M-1, or read each `-0,0` hunk of a created file after the first as a top of its own too.

### IR10-L-2 (low; needs a writer): `<Command>::new` after a keyword is not read, and is not among the declared spellings

`released_path` takes a `<` that follows any name for a type's generic list, and a keyword
is a name to the token reader. So `return <Command>::new("true").status()`,
`match <Command>::new(…)…`, `for _ in <Command>::new(…)…`, `break 'l <Command>::new(…)` are
not read (after `=`, `(`, `&` they are). The same holds for every bracketed spelling of this
round (`return <std::net::TcpStream>::connect(a)`).

- `f/spell/kw-u3.patch` (real `git diff`, scratch repository `repo/kw`; plain `git apply`
  applies it; the result compiles with no warning and runs, spawning `true` twice:
  `Ok(true) true`): every version scores 2, no `process_spawn`. `f/spell/kw-u0.patch` is
  the same at `-U0` (`--unidiff-zero` applies it). `f/spell/kw.patch` has all four keywords;
  only the control line after `=` is read (rounds 16 and 17).

v0.8.3 reads no bracketed spelling, so nothing is lower. Fix: let a keyword before the `<`
open a path, or declare the case.

### IR10-L-3 (low; needs a writer): the one form left at an edge is not always a method: `..unwrap::`

The edge rule reads `.unwrap::` because "every continuation is a call of a method named
`unwrap`". After a range the `.` is the second of `..`, and `unwrap` is a free function:
`0..unwrap::<u8>()` compiles and calls no method (`f/edge/range.rs`, runs: `0..0 1..3 3`).

- `f/edge/range-u0.patch` (real `git diff -U0`, scratch repository `repo/range`; file
  `let r = 1..unwrap::` / `<u8>();` with a free `fn unwrap`; compiles, prints `0..4`, holds
  no panic path): rounds 16 and 17 report `panic_path_added` with basis `hunk` at the edge;
  v0.8.3 and round 14 do not.
- `f/edge/range-u3.patch` (the same at `-U3`): rounds 14 to 17 report it (the rule for the
  whole reads `..unwrap::<u8>(` as a method call); so the false item at the edge is the rule
  for the whole's own false item, which round 14 already had.

rustfmt writes the call on one line, so an honest diff does not end there. Fix: the `.`
before the name must not follow another `.` (in `waiting` and in `panic_path`).

### IR10-L-4 (low; documents): sentences left from round 16 that are now false

- Head of `rust_tokens.rs`, lines 69–76: "What the first half ends on is read as the
  beginning of the construct ([`waiting`]) where it is evidence by itself: the word with the
  token that makes it the construct (`todo!`, `#[ignore`, `Command::`), which no line of
  honest code ends on. Not … the name of an assertion without its `true`". Round 17 reads
  none of those three and dropped the "no honest line" argument; the module head is the list
  of declared limits the brief asks about, and it states the round 16 rule.
- Head of `evidence.rs`, lines 20–21: "what the first half ends on is read only where it is
  evidence by itself, see `waiting`": the round 16 rule.
- `docs/TESTING_GOLD_STANDARD.md` line 63–64: "Since round 16 the edge rule cuts only where
  what a side ends on is evidence by itself (`todo!`, `#[ignore`, `Command::`)": false since
  round 17 (the sentence before it was given "so it was said then", this one was not).
- `placed_at_top`'s "and no other" (IR10-L-1) and the `#[ignore(…)]` sentences (IR10-M-1).

## The request's claims

1. **Monotone against round 14, except the bare `ignore` of a `cfg_attr` list at a side's
   end: HELD** on every set I made. 40,000 real `git diff --no-index` patches of real files
   (`tools/realdiff.py`, seeds 101–104: rustfmt-shaped constructs over one and several lines,
   removals and changes, at `-U0`, `-U1`, `-U3`): 0 lost. The ninth review's generators with
   new seeds (`jgen.py` 31 and 32 honest, `gen.py` 931 rust and 932 mixed; 46,000): 228
   patches lower, 48 of them a second section for one path and the other 180 each a
   `test_skipped` whose side ends on `ignore` in an open list (checked by script: every one).
   The `-U0`, `-U1`, `-U3` history of `ddf-hunt-main` and `forge-warden` (1,701 patches): 0
   lost, 0 gained. `all7.lst` (6,000 on disk): 0 lost.
2. **Never lower than v0.8.3, except the second section for one path: HELD.** realdiff
   40,000: 0; history 1,701: 0; generated 46,000: 371 lower, all a second section for one
   path; `all7.lst`: 1 (`two-sections-create`); the views (`cmp8.py --views`, 20,000 of the
   sets above: score, signals, items, outline labels, the views against the ranking): lower
   0.
3. **An edge form is read only where every continuation is the construct, and never where a
   sentence can end so: HELD with one exception** (IR10-L-3, after `..`). A field with
   generic arguments is a hard parse error even under `#[cfg(any())]` (`f/edge/field.rs`,
   `field2.rs`); inside a macro's token tree it compiles, which the rule excludes in words.
   Every line of 54,244 listed Rust files (the ninth review's lists: Rust's standard library
   and 30,026 files of real crates; and the Cargo registry and the repositories under
   `reviews/`; 19.1 million lines, files deduplicated by content)
   as a one-line hunk: 0 items with basis `hunk` (`out/linescan-a.r17.txt`,
   `linescan-b.r17.txt`).
4. **No header number decides what is found, except old position 0 or 1 (declared): HELD
   as declared for existing files**; the declaration does not cover a created file
   (IR10-L-1).
5. **Cost linear, memory at the stated factors: HELD on my shapes.** `--rank`, `--outline`,
   `--hunks`, `--cluster --parallel 4` (`out/mem1.txt`, `mem2.txt`; peak memory against
   v0.8.3): a 40 MB context line 1.25 (1.00 in `--hunks`); a 20 MB line changed 1.17; a 40 MB
   created line 1.00; a 40 MB context line that opens a comment and a string (several
   readings) 1.25; 2,000 touching hunks with 10 KB context lines 1.97; 40,000 touching hunks
   whose lines hold `*/` and an odd quote (every further reading made) 2.62, in all four
   modes; 200 files 1.03. Time on that last-but-one shape is 3.5 times v0.8.3 (no time
   factor is claimed). The 51 inputs of round 16 (`cost17.sh`, R16 against R17, one
   process at a time): all 51 present; peak memory the same in every input (the largest difference 9 against 10 MiB on a small one); user and system seconds 53.60 in all for round 16 and 53.12 for round 17, measured beside other agents' test runs, so single inputs moved both ways by up to 60 % (`manyhunks-touch` 2.82 against 4.58 s, `items60-frag` 7.06 against 5.64 s); the slowest `items60-frag`, 5.64 s and 404 MiB (`out/cost51-r17.txt`). Nothing points at a change in cost.
6. **The corpus and the documents say what the binary does: the corpus HELD** (`eval` on a
   `git archive` of the commit: 52 cases, precision 166/188, recall 166/244, 103 declared, as
   stated); **the documents BROKEN in places** (IR10-M-1, IR10-L-4).

## The declared limits

- **The bare `ignore` of a `cfg_attr` list at a side's end** (the coordinator's ruling).
  True of the frozen binary; the README marks it *ordinary*, rightly. But the CHANGELOG and
  the request frame it as "three of this round's own patches, none of the 6,002", and that
  reads as rare. It is what rustfmt and `git diff -U0` make together: `f/cfg/cfgu0.patch`
  (both sides rustfmt-clean; a `#[should_panic(` attribute over three lines replaced by
  `#[cfg_attr(any(…), ignore)]`, which rustfmt writes with `ignore` alone on a line above
  `)]`; the `)]` is common, so the `-U0` hunk ends on `    ignore`): rounds 14 and 16 report
  `test_skipped`, round 17 and v0.8.3 nothing. The false item the ruling avoids,
  `#[cfg_attr(…, ignore::case)]`, needs an attribute macro under a crate or module named
  `ignore`. I would revisit the ruling: it trades an ordinary true skip for a writer's false
  one. Either way the CHANGELOG should say an ordinary `-U0` of rustfmt output shows it.
- **Hunks at old position 0 (IR9-M-1).** True: `twotop`, `topone`, `threetop` score 2 in
  every version, `topthen` and `twotop-ctl` 5; marked *writer*, rightly; incomplete for a
  created file (IR10-L-1).
- **The four bracket spellings.** True (`spell.patch` of the ninth review: `<(Command)>`,
  `<(TcpStream)>`, `<Command<>>`, `<<Command as T>::Me>` unread, the three others read);
  marked *writer*, rightly; incomplete (IR10-L-2). The eight-name bound is stated.
- **Far joins in `-U0`.** Stated as *ordinary* with the ninth review's figure; not
  re-measured.
- **The mirror decoy.** True (`mirror.patch` 2, its control 5 in rounds 14 and 17); *writer*.
- **The head of `rust_tokens.rs`** states round 16's edge rule (IR10-L-4).

## Weak tests (by reading; no test or plant was run)

- The guards of the one edge form are not held. The positive set (336 continuations) holds
  that `.unwrap::` is read; no negative test ends a side on another method's name with
  `::` (`.collect::`, `.parse::<`), on a path call (`X::unwrap::`, which goes on as a value
  `X::unwrap::<u8>`), or on `..unwrap::`. A plant that reads any name there, or drops the
  check of the `.` before it, passes every test I can see; none of the twelve plants
  (`/tmp/claude-ddf-round17/ctl/`) touches them.
- `a_scan_holds_one_reading_of_one_text_at_a_time` counts the lines of `Views` only. Memory
  held elsewhere per text (a clone of the code view, a cache of readings) is not counted:
  the test holds the count, not the memory.
- `rust_hunk_places` rule 9 now asserts the `cfg_attr` loss as correct; the fuzz cannot
  write a created file in several `-0,0` hunks under `--unidiff-zero` order (IR10-L-1), a
  range before the edge form (IR10-L-3), a keyword before a bracketed path (IR10-L-2), or an
  `#[ignore(` (IR10-M-1).

## The dogfood result

forge-warden fails `no-hidden-text` on three seeds of `rust_hunk_places`
(`a_spawn_cut_after_its_path_separator_the_second_half_first`, renamed away;
`a_call_cut_after_its_methods_path_separator_the_second_half_first`, its new name;
`a_lined_hunk_after_two_that_touch_claims_to_end_there_the_second_first`). They are
libFuzzer inputs: choice bytes (`06 09 50 c8 01 1e`, `01 09 03 01 01 07`) and a short line,
with no NUL, so Git diffs them as text. Nothing in them is hidden text a reader would be
misled by: no bidirectional control, no zero-width character, no code. Across the commit's
own change outside the seeds there is no bidirectional or zero-width character; the whole
feature (`d4dbc91..aa4acb7`) has four U+200E marks, all in the fixture
`fixtures/eval/evidence/independent-review-5-20261114.patch`, on purpose. I recommend a
`.gitattributes` line `fuzz/seeds/** -diff` (or `binary`): the seeds then show as binary in
every diff and stop failing the gate for bytes nobody reads.

## What I did not look at

- No `cargo`: no unit test, plant or fuzz target was run; claims about them are by reading.
  The `ill_formed_attribute_input` allowance in `Cargo.toml` was simulated with the rustc
  flag Cargo passes for it.
- Languages other than Rust only through `gen.py` mixed (8,000) and the views comparison.
- `crates/deep-diff-forge-daemon/src/views.rs` and `lexical.rs` were not read line by line;
  the views were compared by output only.
- The 265-cut and 336-continuation counts were not re-derived; times measured beside other
  agents' test runs are noisy (memory is not).

STATUS: COMPLETE
