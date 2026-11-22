# Seventeenth independent review of the agent hooks branch (`agent/loop`, commit `38af7b8`)

Date: 2026-11-16 17:50 to 18:05 UTC (2026-11-17 04:50 to 05:05 AEDT) by the host's clock.
Reviewer: an independent Claude agent that took no part in the work. Brief:
`~/.local/state/habitat-claude/ddf-gap-probes-20261113/ddf-hooks-independent-review-17-brief.md`
(extends the eleventh and sixteenth briefs, both read whole). Under review: `d4dbc91..38af7b8`,
mainly the round `7380e8e..38af7b8`. Weighed by the convergence rule the round-25 request
carries. Vesper's round-25 report was not read (none was in the worktree when this review began).

## Verdict: NO-GO (one medium, new in this round, in an ordinary workflow)

Every finding round 25 answers is closed on the frozen binary with the real judge: VD4R24-M-1,
HR16-M-1, HR16-L-1, HR16-L-3, and HR16-L-2 is narrowed with sentences that are exact. The
round-23 and round-24 closed findings stay closed. What keeps it from GO is that ruling 1 (an
identity does not move while its remembered directory stands) takes **any** directory at the
remembered path as the tree standing, so an ordinary `git worktree move` followed by anything
new at the old path (a fresh linked tree, a clone, a build directory) makes the moved tree
UNJUDGED for the rest of the session, and the remedy its message gives does not work:

- **HR17-M-1** (medium; ORDINARY WORKFLOW; NEW in `38af7b8`). `git worktree move L L2` of a tree
  a failure stands in, then `git worktree add L …` (or a clone or a build directory at `L`):
  every stop from `L2` is UNJUDGED, a new key in `L2` never blocks, and `git worktree repair`
  "in the directory that stands" changes nothing. On `v4r24`: FAIL STANDS, then the new key
  BLOCKs.

Three lows:

- **HR17-L-1** (low; ORDINARY WORKFLOW, uncommon; older). `git worktree repair` run in a copy
  before anything blocked in the original: the original is no tree by the records, and a key
  in it is SILENT at every stop from the project. This round's new hint in every unpaired
  reason ("`git worktree repair` in a tree moved with `mv` records where it is") is also shown
  for copies.
- **HR17-L-2** (low; ORDINARY WORKFLOW; older). A tree moved with plain `mv`, then the advice of
  the "gone" note this round reworded (`git worktree prune`), or Git's own advice at the old
  path (`git worktree add -f`): what blocked is forgotten and every stop from the project is
  SILENT with the blocked key and a new one in the moved directory.
- **HR17-L-3** (low; test, not product; older). The flake in
  `a_repository_with_more_working_trees_than_are_read_says_so` is the session start's 8 s
  budget over 256 trees under load: reproduced on both `v4r25` and `v4r24`, not this round's.

Under the convergence rule: HR17-M-1 is against a claim and is fixed, best by about ten lines
(below) rather than by narrowing, because the class needs no attacker. The three lows are
wording and stated limits, each under 20 lines.

## How this was done

- Source: `git archive 38af7b8` into `/var/home/louranicas/.cache/ddf-t/hooks-review-17/src`;
  the round's diff in `…/hooks-review-17/notes/r25.diff` and `r25-docs.diff`. Read: the
  round-25 request, Vesper's round-24 report, the sixteenth review, the whole production diff
  (`standing_record`, `moved_here`, `moved_words`, `elsewhere`, `left_out`, `listed_instance`
  with `taken`, `listed_others`, `other_trees`, `judge_units`' `out.tops` and `judged`,
  `recorded_trees`, `trees_to_judge`, `Unfound::words`), the new tests and their names, and the
  changed README, SECURITY and CHANGELOG text. No `cargo`, no build, no `toolbox`, `podman` or
  `distrobox`, nothing at GitHub. I stopped no process. The only file written in the worktree
  is this report.
- Binaries (hashes checked): `…/review-bin/v4r25/release/deep-diff-forge` `35d6fad7…`; for "is
  it new" `…/v4r24/release/…` `0d42de40…`; debug `cceebda0…` hashed, not run. Judge: the real
  forge-warden `…/target-fw/release/forge-warden` `55bf9fe6…` on the host. Git 2.56.1.
- Bases: `…/hooks-review-17/d17/base-repo` (a `--no-local` clone of
  `hooks-round25/d11/base-repo`, admits v4r25) and `d17/base-v24` (a clone of the sixteenth's
  `d16/base-repo`, admits v4r24). Case trees under `d17/cases/`; a private HOME under `d17/`
  set inside every driver (`lib11.py` reads `RHOME`).
- Scripts: `/var/home/louranicas/.cache/ddf-t/hooks-review-17/notes/` (`run1.sh` to `run4.sh`;
  `s/w17.py` my scenarios; `s/w17many.py` the start timing; copies of the sixteenth's and
  Vesper's drivers, repointed). Outputs: `notes/out/<script>-<scenario>-<build>.txt`; hashes
  in `notes/out/script-hashes.txt`; a log in `notes/NOTES.md`. No output holds a traceback or
  an unintended "was not found" (the one hit is `w16state badjudge`, whose judge is missing on
  purpose).

## Section 0, item 1: the closed findings on this build

`run1.sh` and `run4.sh` (both builds where marked):

| finding | script | v4r25 | v4r24 |
|---|---|---|---|
| VD4R24-M-1 | `v4r24-gitdir-duplicate.py` | BLOCK; UNJUDGED "Git names …/outer as the working tree that this session's memory has at …/outer/inner, which still stands"; from main named; memory kept at every step. **Closed** | BLOCK, SILENT, SILENT, memory erased |
| HR16-M-1 | `w16pair copyinside` | the copy named at every stop from R ("… is no working tree of this project by the repository's records …"); UNJUDGED from it. **Closed** | SILENT from R |
| HR16-M-1 | `w16pair copyinsidecommit` | named; `git add -A` HOLD; commit from the copy BLOCK | SILENT at 1 |
| HR16-L-1 | `w16pair gitdirdupa`, `gitdirdupz` | from O "1 thing … not judged" (I's tree named), memory kept. **Closed** | SILENT from O |
| HR16-L-3 | `w16pair copyrepair` | from R named (both paths), from C and L UNJUDGED, memory kept. **Closed** | SILENT at 2 to 4, memory erased |
| HR16-L-2 | `w16pair sepdot`, `dotgitlink`, `w17 sepdotlinked2` | UNJUDGED at every stop from the main tree, with words; stops from L judged. As now stated | the same |
| controls | `w16pair sep`, `moved`, `movedrepair`, `gitmv`, `nestedclone`; `w17 dotgitlinkname` (`.git` a link to `proj.git`) | the same decisions as v4r24 at every step | |
| round 23/24 | `v4r23-core-worktree-nested`, `v4r23-v9-clean`, `w15cw copy`, `w15copy clean|keep`, `w15ceil dotdot`, `w16link homeadd|homerel|homerepair|laterlink`, `w16state badjudge|unpaired` | held (UNJUDGED or named, memory kept; v10 written; ceiling hidden; links on either side read as one tree) | (as the sixteenth found) |

## Section 0, item 2: ruling 1 in ordinary workflows

`run2.sh` with `s/w17.py` on both builds.

| case | v4r25 | v4r24 |
|---|---|---|
| `git worktree move L L2`, then `git worktree add L -b feature2` (`moveadd`) | from L2 UNJUDGED at every stop; a NEW key in L2 UNJUDGED; repair in L and in L2 change nothing (HR17-M-1) | FAIL STANDS; the new key BLOCK |
| the same with a clone at L (`moveclone`), a build directory `L/target/debug` (`movemkdir`) | the same (HR17-M-1) | FAIL STANDS; BLOCK |
| two trees swapped with three `git worktree move` (`swap`) | UNJUDGED from either; named from R | FAIL STANDS |
| L replaced by a file (`movefile`), by a link to L2 (`movelinknew`) | read at L2; the new key BLOCK | |
| L replaced by a link elsewhere (`movelinkother`), an unreadable directory (`moveunread`) | UNJUDGED, named from R (as ruling 1 says) | |
| copy, `rm -rf L`, repair in the copy (`removerepair`); repair, then `rm -rf L` (`repairremove`) | FAIL STANDS from the copy once L is gone; a new key BLOCK; UNJUDGED while L stands | |
| memory in L, a copy inside R repaired there (`copyinrepaired`) | named from R, UNJUDGED from C and L, memory kept | SILENT, memory erased |
| repair in a copy BEFORE anything blocked, then a key in L (`repairfirst`, `repairfirstin`) | SILENT from R at every stop; UNJUDGED from L (HR17-L-1) | the same |
| `mv L L2`, then `git worktree prune` (`mvprune`); `mv L L.bak`, then `git worktree add -f L` (`mvaddf`) | memory erased, SILENT from R and L with two keys in the moved directory (HR17-L-2) | the same |
| a commit of the new key in `moveadd` (`moveaddcommit`) | BLOCK on the commit; removing the new tree at L lifts the UNJUDGED | BLOCK |

## Section 0, item 3: rulings 2 and 3

A copy of a linked tree inside the main tree, inside another linked tree (`copyinsidelinked`:
v4r25 names it from O, v4r24 was SILENT), with relative records (`relcopyinside`: Git does not
see a repository and the copy's key is in the patch: BLOCK), a nested clone (named): nothing
unlisted is taken for a listed tree. Two listed entries with one path are two trees in both
orders of the administrative directories (`gitdirdupa`, `gitdirdupz`). `taken` cannot misname
an ordinary tree: an id is taken only by an earlier entry with the same path, and the fallback
through the tree's own `.git` file ignores `taken`. `judged` is now the ids of the trees read
(the project and the linked tree the session is in); the placeholder of an unplaced directory
has a path id no listed tree has, so nothing changed for it. Held.

## Section 0, item 4: the flake

`run3.sh`, `s/w17many.py`: a repository built as the test builds it (255 linked trees,
`--detach --no-checkout`, a 140-character directory name, plus the main tree: "as many as are
read"), then N session starts at once, each with its own HOME, machine load about 10 to 13 from
other work:

| starts at once | v4r25 | v4r24 |
|---|---|---|
| 1 | 3.45 s, recorded | 3.47 s, recorded |
| 8 | 3.0 s | 3.0 s |
| 16 | 3.6 s | 3.6 s |
| 32 | 5.0 s | 5.0 s |
| 48 | all recorded (`v4r25` only) | |
| 64 | all 64 at 8.07 s: "the commits … could not be listed: `git rev-list` did not finish in the time a hook has and was stopped" | the same, 8.06 s |

The test's "before" session expects the wording of a session that has its list; under the
suite's load its start ran past the 8 s budget and got the wording of one that has none, which
is the hook doing what it says. Round 25 touches no code of the start or of listing commits.
**Not this round's**; a load sensitivity of the start budget over 256 trees' reflogs, present
on `v4r24` identically (HR17-L-3).

## Section 0, item 5: the sentences

True where said, checked against runs: CHANGELOG round entry (each named test exists and its
case reproduces); README and SECURITY on the copy inside the project, on two entries with one
path, on a main Git directory named `.git` not at the top (exact: every stop from the main
tree UNJUDGED, stops from linked trees judged, `.git` a link to a directory named otherwise is
judged); the prune note (Git lists a separate main Git directory as the main tree and prune
does not change it); the `mv` remedy (repair after `mv` gives FAIL STANDS).

Not true, or narrower than the hooks' blindness:

- "until `git worktree repair` is run in the directory that stands or that directory is
  removed" (README, SECURITY, CHANGELOG, and the stop's own words): false where the directory
  that stands is another tree, a clone or a build directory (HR17-M-1).
- "A working tree is known by what it is, not by its path: a linked tree that is moved (`git
  worktree move` …) keeps what has blocked in it" (README): it keeps the memory, but whatever
  stands at its old path now decides that it is never judged (HR17-M-1).
- "This is what a copy of a tree that `git worktree repair` was run in …, or Git's own files
  edited by hand, give" (README): an ordinary move gives it too (HR17-M-1).
- SECURITY "Not a boundary" names only hand edits of `gitdir`, `commondir`, `core.worktree`.
  The hooks are also blind, with no hand edit, to a directory orphaned by Git's own commands:
  `git worktree repair` in a copy before anything blocked (HR17-L-1), `mv` then `prune` or
  `add -f` (HR17-L-2). The limit is narrower than the blindness; one sentence each closes it.

## Section 0, item 6: the eleventh brief's sections 1 to 7, briefly

This round's code is confined to where a tree is (`elsewhere`, `left_out`, `listed_others`,
`other_trees`, `out.tops`); the judge call, verdict validation, the patch, state schema,
`agent setup`, `agent doctor --gate`, `schema signals` and `explain signal` are unchanged. On
this build: every hook output in my runs is valid JSON (no `NOTJSON`); BLOCK, FAIL STANDS,
UNJUDGED and SILENT where expected in the controls above; no stop past its deadline (all under
0.5 s except the start timing); a missing judge is UNJUDGED and a v9 state is written as v10
with nothing lost (`w16state`). Section 7, the tests: read, not run. The five new integration
tests and the unit table hold each finding's case. The unit table's row "stands | not read"
**asserts the HR17-M-1 behaviour**; no test moves a tree with `git worktree move` and puts
something at its old path. The flaky test is load-sensitive by construction (a wall-clock
budget on 256 trees under the suite's parallel load).

## Findings reproduced with the real judge

Every command below runs a driver in `/var/home/louranicas/.cache/ddf-t/hooks-review-17/notes/s/`
with `DDF=…/review-bin/<build>/release/deep-diff-forge` and `BASE=…/d17/base-repo` (v4r25) or
`…/d17/base-v24` (v4r24), through `notes/run2.sh` (`VS=v4r25` for one build).

### HR17-M-1 (medium; ORDINARY WORKFLOW; new in `38af7b8`): a tree moved with `git worktree move` is never judged again once anything stands at its old path

Claims made false: README "until `git worktree repair` is run in the directory that stands or
that directory is removed" (the same in SECURITY, the CHANGELOG and the stop's reason); README
"A working tree is known by what it is, not by its path: a linked tree that is moved (`git
worktree move`, …) keeps what has blocked in it"; README "This is what a copy of a tree that
`git worktree repair` was run in …, or Git's own files edited by hand, give". By the eleventh
brief's rule a new failure in an ordinary workflow that never blocks reads high; I rate it
medium because a commit of it still blocks and every stop says UNJUDGED (the precedent of
HR16-M-1). Class: ruling 1's test for "the remembered directory stands" (new in this round).

```
./run2.sh moveadd          # out/w17-moveadd-v4r25.txt, out/w17-moveadd-v4r24.txt
-- 1 key in L; cwd = L                                   v4r25 BLOCK      v4r24 BLOCK
   git worktree move L L2; git worktree add L -b feature2    state trees: {id: L}
-- 2 cwd = L2      v4r25 allow:UNJUDGED ("Git names L2 as the working tree that this session's
                   memory has at L, which still stands … until `git worktree repair` is run in
                   the directory that stands or it is removed")       v4r24 FAIL STANDS
-- 3 a NEW key in L2; cwd = L2          v4r25 UNJUDGED                 v4r24 BLOCK
-- 4 cwd = L2 again                     v4r25 UNJUDGED                 v4r24 FAIL STANDS
-- 5 cwd = R                            v4r25 named                    v4r24 named
   git worktree repair in L (rc 0); git worktree repair in L2 (rc 0)
-- 6 cwd = L2                           v4r25 UNJUDGED                 v4r24 FAIL STANDS
   state trees after: {id: L}  (v4r24: {id: L2})
./run2.sh moveclone movemkdir swap   # a clone at L; `L/target/debug`; two trees swapped: the same
./run2.sh moveaddcommit              # the new key committed in L2: BLOCK (both builds); after
                                     # `git worktree remove --force L`: FAIL STANDS from L2
```

At steps 2 to 4 the fresh tree at L is also named "the session's directory may be in it" and
not looked at from L2. How likely: an agent that was blocked in a tree and sets it aside with
`git worktree move` to start fresh at the same path, or a build tool that recreates its output
directory at the old path, does this without any intent; one stop between the move and the
new directory would have recorded L2 and avoided it, but a turn usually does both. Cause:
`standing_record` (`hook_run.rs` about line 2612) takes "a directory stands at the recorded
path" for "the tree stands there". Cheap fix (about ten lines, no state): the recorded
directory stands as that tree only when its `.git` names this tree's administrative directory
(`instance_of` of the directory its `.git` file points to is `id`), or when that cannot be read.
VD4R24-M-1 (`inner/.git` still names the inner tree's directory), HR16-L-3 (the original's
`.git` names it) and `moveunread` keep their UNJUDGED; `moveadd`, `moveclone`, `movemkdir`,
`swap` and `movelinkother` read the tree at its new top. The `.git` file of the tree then
belongs in the "Not a boundary" list of Git's files edited by hand. Narrowing instead is
possible (state that any directory at a moved tree's old path keeps it UNJUDGED until that
directory is removed, and drop "repair … in the directory that stands" from the remedy), but
it narrows an ordinary workflow that was right on v4r24.

### HR17-L-1 (low; ORDINARY WORKFLOW, uncommon; older): repair in a copy before anything blocked orphans the original

Claim: the round-14 rule (request claim 1); README "This is what a copy of a tree that `git
worktree repair` was run in … give" (said only of a copy repaired after something blocked).
Class: a copy of a linked tree (HR15-L-2, HR16-L-3).

```
./run2.sh repairfirst repairfirstin   # out/w17-repairfirst*-v4r25.txt and -v4r24.txt
   cp -a L C (beside R, or inside R); git worktree repair in C: "gitdir incorrect"; list: R, C
-- 1 a key in L; cwd = R    SILENT   (both builds)
-- 2 cwd = L                UNJUDGED
-- 3 cwd = R                SILENT
-- 4 cwd = C                SILENT (C is clean)
```

Git's records name C as the tree; nothing records L, so no stop from the project can know it.
This round's hint, appended to every unpaired reason ("`git worktree repair` in a tree moved
with `mv` records where it is"), is shown for copies too (`w15copy`, `copyinside`), where
following it makes this case. Fix: words. The hint conditional on the case ("if this directory
was moved with `mv` …; if it is a copy, remove it"), and one sentence in README and SECURITY:
`git worktree repair` run in a copy makes the copy the tree; the directory copied is then no
tree of the project and nothing in it is judged unless a stop is made from it.

### HR17-L-2 (low; ORDINARY WORKFLOW; older): `mv`, then `prune` (the gone note's advice) or `add -f` (Git's advice)

Claim: SECURITY "What blocked in a tree is not forgotten while its directory stands"; the
round-14 rule. Class: a tree whose registration Git ends while its moved directory stands.

```
./run2.sh mvprune            # out/w17-mvprune-v4r25.txt, -v4r24.txt
-- 1 key in L; cwd = L        BLOCK
   mv L L2
-- 2 cwd = R                  "The working tree L … is gone or is no working tree any more …
                              (`git worktree prune` forgets a linked tree that is gone; …). A
                              failure there has blocked a stop of this session and stands"
   git worktree prune ("gitdir file points to non-existent location"); a second key in L2
-- 3 cwd = R                  SILENT, memory erased     (v4r24 the same)
-- 4 cwd = R again            SILENT
-- 5 cwd = L2                 UNJUDGED ("gitfile does not point to a valid repository")
./run2.sh mvaddf             # mv L L.bak; git worktree add -f L (Git reuses the directory name)
-- 2 cwd = L, 3 and 4 cwd = R SILENT, memory erased; 5 cwd = L.bak UNJUDGED (both builds)
```

The note this round reworded still advises prune for a tree a failure stands in, without
saying that a tree moved with `mv` is put right with `git worktree repair` and that prune
forgets what blocked there. Nothing from L2 can be committed after prune (its `.git` names
nothing), so the keys cannot reach the history unjudged. Fix: words in that note when a failure
stands (moved: repair in the new place; prune forgets what blocked) and a stated limit beside
"a tree made anew at the path of a removed one".

### HR17-L-3 (low; test; older): the flaky test is load-sensitive

See section 0, item 4. Not this round's. A cheap fix: run this test alone (a serial guard), or
accept either wording for "before" when the start says its commits could not be listed.

## Suspected only (not reproduced)

- With the HR17-M-1 fix as proposed, deleting the inner tree's `.git` file in VD4R24-M-1 would
  make its directory "not standing" again; that is a hand edit of Git's files and belongs in
  the stated limit's list.
- A tree known only by its path (`x…` ids, when its administrative directory cannot be read) is
  never in `State::trees`, so ruling 1 never protects it; I found no ordinary way to give a
  tree with a remembered failure such an id.

## What I did not look at

`verdict_v2.rs`, `vocabulary.rs`, `agent setup`, `schema signals`, `explain signal` (unchanged);
two stops at once; a full disk; the mutation run and the test suite (no `cargo`); the debug
binary (hashed, not run); the fifteenth's and sixteenth's full corpora (`nr`, `mi` matrices,
batch), which the implementer's `compare25.out` reports unchanged; `w15schema`; a hanging Git
during the new `standing_record` (it asks no Git, only `metadata`).
