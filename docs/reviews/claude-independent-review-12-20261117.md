# Twelfth independent review of the integrity signals (signals round 19, commit `f819a56`)

Reviewer: a fresh Claude agent (Opus 5.5) that took no part in the work. Scope: `af74308..f819a56`
(round 19) and the whole feature `d4dbc91..f819a56`. Frozen binaries only (no `cargo`):
`review-bin/v3r19/release` (SHA-256 `54b93d22…` checked), `v3r18`, `v3r14`, published v0.8.3.
Scratch, notes and every reproduction: `/var/home/louranicas/.cache/ddf-t/indep-review-12/`
(`notes/NOTES.md`, `notes/f/`).

## Verdict: GO-WITH-FIXES (documents only; nothing high or medium)

No high or medium finding against a claim. Three lows, all fixed by narrowing or correcting
documents (no code): one ordinary (IR12-L-1: the declared rustfmt miss also happens with default
`git diff`, which the request and the coordinator's ruling say it does not), one writer's
(IR12-L-2: an exception to the whole-attribute rule that the README does not state), and wording
(IR12-L-3). Every closed finding of round 18 stays closed on this build. Under the round cap I accept
narrowing for all three. This becomes GO after a docs-only diff that I re-read.

## Section 0: closed findings rerun on v3r19

| finding | patch | v0.8.3 | r14 | r18 | r19 |
| --- | --- | --- | --- | --- | --- |
| VD3R18-M-1 | Vesper `ignparen-u0.patch` (both files) | none | none | `test_skipped` | **none** (closed) |
| VD3R18-M-1 | Vesper `ignparen-u3.patch` | none | none | skip | skip (whole text in a macro; as designed, see limits) |
| VD3R18-L-1 | `become-u0/u3`, `yeet-u0/u3` | none | none | none | **`process_spawn`** (closed) |
| IR11-L-2 | `mutref-u0/u3` | none | none | none | **`process_spawn`** at both lines (closed) |
| IR11-L-1 | `vecturbo-u0` | none | none | spawn | spawn (declared writer's false positive; `-U3` none) |
| IR10/IR11 | `endparen.patch` | none | none | skip | **none** (closed) |
| IR10 | `ignparen.patch` (whole forms) | none | none | 2 skips | 2 skips (kept) |

## Findings

### IR12-L-1 (low, ordinary): the rustfmt `ignore` / `ignore = "…"` miss is not limited to `-U0`; default `git diff` loses it too

- **Claims it makes false.** The request, claim 2: "Monotone against round 14 … except the stated
  attribute cases at a `-U0` side end", and "With default context the `)]` is shown and the skip is
  read". The coordinator's ruling, reason 4: "Default `git diff` (3 lines of context) shows the `)]`".
  README (ordinary limits): "rustfmt writes … `ignore = "reason"` alone on the line above `)]`, so an
  ordinary `git diff -U0` of two rustfmt-clean files ends a hunk there". The head of `rust_tokens` and
  the CHANGELOG say the same of `-U0` only.
- **What happens.** A hunk with context ends on the `ignore` line, before `)]`, when the last changed
  line is exactly as many lines above `ignore` as the context width. A change to the skip's
  *condition* is a change to the skip, by the design (IR-M-3). So: rustfmt's vertical layout
  (`#[cfg_attr(`, `any(`, four targets, `),`, `ignore = "…"`, `)]`). Change the second and third
  targets (`"macos"` to `"ios"`, `"wasm32"` to `"wasm64"`; still rustfmt-clean), then run plain
  `git diff`. The hunk `@@ -2,8 +2,8 @@` shows `#[cfg_attr(` through `ignore = "slow on these
  platforms"` and stops there.
- **Reproduced** (`notes/f/u3/`):
  - `four-u3.patch` (default context): r14 and r18 report `test_skipped` (new line 5, hunk); r19 reports nothing; v0.8.3 reports nothing.
  - `fourbare-u3.patch` (the same with a bare `ignore`): r14 reports a skip; r18 and r19 report nothing. This has been so since round 17, which declared it only at `-U0`.
  - `mid-u1.patch` (`git diff -U1`, three targets: rustfmt keeps `any(…)` on one line): r14 reports 2 skips, r18 reports 1, r19 reports 0.
  - `git apply --check` passes on `four-u3.patch`. The new file compiles and its test runs (`four-new.rs`).
- **How common.** Rare. Over the history of ten real repositories that no review had used
  (`/var/mnt/storage/repos`: nushell, zellij, atuin, just, bottom, yazi, herdr, television, bacon,
  tuicr; 5,288 commits as `git show -U0`, `-U1` and `-U3`, 15,864 patches), r19 loses **nothing**
  against r14. 486 of those patches carry `test_skipped`, the same on r14, r18 and r19. Nushell writes
  this layout (`#[cfg_attr(`, `target_os = "macos",`, `ignore = "…"`, `)]`) in five tests, and a
  condition change there at `-U1` is this case.
- **Class.** It is the declared class of ruling 1 and VD3R18-M-1: an attribute the side does not hold
  whole. What is wrong is only the stated scope (`-U0`) and the premise of the coordinator's reason 4.
  The trade-off stands: reading the open list again would bring back the false skip of VD3R18-M-1.
- **Narrowing: yes, docs only.** Say that the miss happens wherever a hunk ends between the
  `ignore` and the `)]`, at any context width: at `-U0` on the `ignore` line, at `-U1` after a changed
  condition line, and with default context after a change two or three lines above it in rustfmt's
  vertical layout. Correct "with default context the `)]` is shown" in the request record, or mark it
  wrong. Optionally add `four-u3.patch` to the corpus as a `known_miss`.
- **New against the release?** No. v0.8.3 reports nothing on either patch.

### IR12-L-2 (low, writer's): a list opened past `CFG_ATTR_LISTS` is a skip, with no `ignore` at all and not whole; the README states no exception

- **Claim it makes false.** README: "A skip is read only where the side holds its attribute whole,
  from `#` to its `]`" (and "Since round 19 none of them is read where the side ends before the
  `]`"). The exception is stated only in the CHANGELOG and in the `Skips` doc comment ("A list opened
  past [`CFG_ATTR_LISTS`] is the one exception … taken to hold a skip").
- **Reproduced** (`notes/f/deep/`). `deep17.rs` has 17 nested `#[cfg_attr(a, ` inside
  `discard! { … }` and no `ignore` anywhere. It compiles with rustc 1.99.1 and its one test runs.
  Its `git diff -U0` (`deep17-u0.patch`) and default `git diff` (`deep17-u3.patch`) both give
  `test_skipped` on r14, r18 and r19, and nothing on v0.8.3.
- **Class.** The bound on followed lists. It needs a writer: no code nests `#[cfg_attr(` inside a
  list. It is not new in this round (r14 does the same).
- **Narrowing: yes.** Add one clause to the README's writer's limits and to the whole-attribute
  sentence: a text that opens more than 16 `#[cfg_attr(` lists inside one another is taken to hold a
  skip. No code.

### IR12-L-3 (low, wording)

- **The request's cost line is inverted.** It says "54.29 s against v3r18's 54.63 s". In
  `round19/cost/cost-final.txt` the first column is v3r18 (`cost19.sh` runs `$R18` first) and
  sums to 54.29 s; round 19 sums to 54.63 s. The CHANGELOG ("54.29 s in all for round 18 and
  54.63 s now") is right. Correct the request record.
- **"nor … `mut` after `&raw`".** The README and the doc comment on `begins_expression` say so,
  but the reader takes `mut` whatever comes before it. `&raw mut <Command>::new("true")` is read
  as a `process_spawn` (`notes/f/probe/`). It does not compile (rustc 1.99.1, E0745, checked), so no
  honest diff is affected. Say "`mut` (only `&mut` compiles there)", or leave it as a limit.

## Claims of the request

1. Skip read only when the side holds the attribute whole: **held**, with the undeclared exception of IR12-L-2.
   - At a `-U0` side end, none of these is read on r19: `#[ignore(`, `#[ignore =`, `#[ignore = "…"`, `#[ignore = ']'`, `#[ignore = "]"`, `#[ignore('a]`, `#[cfg_attr(miri, ignore)`, `cfg_attr(…, ignore = "x")`, 60-argument `#[ignore(…)]`.
   - All of these are still read: `#[ignore]`, `# [ ignore ]`, `#[r#ignore]`, `#[ignore = "slow"]`, `#[ignore(']')]`, `#![cfg_attr(miri, ignore)]`, a comment between `)` and `]`, nested and trailing `cfg_attr`, raw `r#cfg_attr`/`r#ignore`, `ignore = "a)]"`, `ignore(flaky)`, and 59 arguments, which fit the bound.
   - The views (`--outline`, `--hunks`) agree with the rank.
2. Monotone against r14 except the stated cases: **held on every patch I generated in bulk.** 47,864 patches: 97 lower, every one `test_skipped`. All 97 are in generated sets, and each is a list or attribute that the side does not hold whole: the jgen sets, and the realdiff generator's truncated `cfg_attr` blocks. None is in the real histories. **Broken in scope** by IR12-L-1: the same kind happens at default context, not only at `-U0`.
3. Never lower than v0.8.3: **held.** Nothing lower on the 47,864 patches. No refusal differs on any pair.
4. Keyword list as measured: **held on what I checked.** The table is consistent with what the code reads, and a name before the `<` is not read. I compiled only `&raw mut <Command>::new` (E0745); I did not re-derive the 677 programs. IR12-L-3 covers the wording.
5. Cost and memory: **held on my shapes.** 2 MB of `#[ignore(` on one line or one per line, `#[cfg_attr(a, ignore(x` per line, and 60 nested brackets per attribute: r19 is within r18's time and memory on each.
6. Corpus and documents: **corpus held** (r19 `eval`: 54 cases, 177/201, 177/259, 33/33, 12/15, 109 declared). The 176 to 173 cut count is consistent: the three attribute cuts behind two false numbers (replacement hunks) are found by r14 and r18 and not by r19, and are found with one true number. Documents: see IR12-L-1 and IR12-L-3.

## Declared limits, my view

- The rustfmt `ignore = "…"` miss (coordinator's ruling): it is ordinary, and the decision to accept it stands. Its stated reach is too small (IR12-L-1).
- IR11-L-1 (side beginning at `<Command>` under an unseen `Vec::`): correctly a writer's false positive. Reading it is the right direction.
- The 64-token bound: exact on the binary (59 arguments read, 60 not).
- **Limit, one line:** a whole attribute inside a token tree that a macro discards is a skip. Vesper's own `ignparen-u3.patch` is reported. This follows the stated design ("read like any other"), but the README's example sentence ("compiles and skips nothing") could read as if it were not reported. It is a writer's case.

## Bulk comparison (as run)

The 47,864 patches:
- The `-U0`/`-U1`/`-U3` history of the ten real repositories above (15,864).
- `realdiff.py` seeds 301 and 302 on `scan-a.lst`: real `git diff --no-index` at three widths (16,000).
- `jgen.py` seed 51 (10,000) and seed 52, honest numbers (6,000).

Every binary was run on every patch (`notes/multi.py`). There were no exits and no timeouts, and
the plain and strict comparisons are equal:
- r14 to r19: 97 lower.
- v0.8.3 to r19: 0 lower.
- r18 to r19: 23 lower, all in generated sets.

Outputs: `notes/cmp-*.txt`, `notes/cmps-*.txt`. The generated sets were removed after the
comparison; `notes/hist12.sh` and `notes/bulk12.sh` regenerate them.

## Weak tests

- No test or corpus case holds the default-context or `-U1` form of the ordinary miss. Only `-U0` and the case with its `)]` are held.
- As the implementer states, no fuzz rule tells round 18's reading of the parenthesis from round 19's.

## What I did not look at

- I did not run `cargo` (tests, clippy, gate, fuzz).
- I did not run the full keyword derivation.
- I did not cover languages other than Rust.
- I did not re-measure the 51 cost inputs or the memory shapes of the earlier rounds.
- I did not re-run the eleventh review's 58,198 patches: they had been removed, and I generated new sets.
