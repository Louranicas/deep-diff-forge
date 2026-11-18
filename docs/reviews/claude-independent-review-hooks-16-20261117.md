# Sixteenth independent review of the agent hooks branch (`agent/loop`, commit `7380e8e`)

Date: 2026-11-16 13:20 to 13:37 UTC (2026-11-17 00:20 to 00:37 AEDT) by the host's clock
(`date`). Vesper's round-24 report appeared in the worktree during this review; I did not read
it, so this review is not anchored on it.
Reviewer: an independent Claude agent that took no part in the work. Brief:
`~/.local/state/habitat-claude/ddf-gap-probes-20261113/ddf-hooks-independent-review-16-brief.md`
(extends the eleventh brief, read whole; machine rules `machine-rules-20261115.md` read first).
Under review: `d4dbc91..7380e8e`, mainly the rework `c9a2ece..7380e8e`. Weighed by the
convergence rule the round-24 request carries.

## Verdict: NO-GO (one medium, fixable in a few lines)

Every finding round 24 answers is closed on the frozen binary with the real judge:
VD4R23-M-1, VD4R23-L-1, HR15-L-1, HR15-L-2 and the ceiling note (`R/../H`). The pair rule
holds against every redirect I could make with `core.worktree`, an edited `commondir`, the
`gitdir` record of a moved or repaired tree, relative records, `/home` (a link on this host)
on either side of the comparison, a link put in later above a tree, and a hanging Git
(the stop stays inside its six seconds for other trees and forgets nothing). State and
ceiling hold. The fifteenth review's corpus gives the same decision at every step as on
`v4r23`, except the steps this round means to change (see "Section 8").

What keeps it from GO is one older hole that this round touched and kept, in an ordinary
(if uncommon) workflow:

- **HR16-M-1** (medium; ORDINARY WORKFLOW; older, kept by this round's new `Unpaired` arm in
  `left_out`). A copy of a linked tree made INSIDE the main tree (`cp -a ../feature
  ./feature-copy`) with a credential in it: every stop from the project is SILENT. A nested
  clone at the same place is named ("a Git repository of its own … left out and not
  judged"); the copy is not, because `left_out` takes any directory Git places in this
  repository, now also an unpaired one, for a working tree "named by the rule for working
  trees", and that rule names only listed trees. It breaks the README's rule for what a stop
  names ("nothing that is known in the project stays outside what was judged … a nested
  repository that was left out") and the doc comment of the new arm.

Three lows:

- **HR16-L-1** (low; DELIBERATE EVASION; older, narrowed by this round). Two listed entries
  with one path (tree I's administrative `gitdir` edited to name tree O's `.git`): a stop
  from O is SILENT while I's failure stands and a second key sits in I. A listed tree is
  skipped as "judged" by its PATH (`listed_others`: `judged.contains(&tree.path)`), so I's
  entry is never seen from O.
- **HR16-L-2** (low; ORDINARY WORKFLOW, uncommon; NEW in this round). A main Git directory
  named `.git` that is not in the tree's top (`git clone --separate-git-dir=X/.git`, or a
  `.git` that is a symbolic link to such a directory): every stop of a session begun in
  the main tree is UNJUDGED (on `v4r23`: BLOCK, then FAIL STANDS). README's parenthesis
  "(its Git directory made apart with `--separate-git-dir`)" promises that such a session
  is judged.
- **HR16-L-3** (low, or a limit to state; ORDINARY WORKFLOW, uncommon; older). `cp -a L C`,
  `git clean` in C, then `git worktree repair` in C: Git's records now name C, L's memory is
  forgotten at the first stop (C read clean under L's id), stops from the project and from C
  are SILENT with two keys in L, and a stop from L is UNJUDGED for good (on `v4r23` the same
  silence, but a stop from L blocked).

## How this was done

- Source: `git archive 7380e8e` into `/var/home/louranicas/.cache/ddf-t/hooks-review-16/src`;
  `git diff c9a2ece..7380e8e` in `…/hooks-review-16/notes/r24.diff` and `r24-docs.diff`.
  Read: the request, Vesper's round-23 report, the fifteenth review, the whole production
  diff, `pair_of`, `placed`, `names_the_tree`, `unpaired_here`, `elsewhere`, `left_out`,
  `other_trees`, `listed_others`, `listed_instance`, `linked_instances`, `same_form`,
  `through_links`, `ceiling_without`, `state_read`, `Session::keep`, `repository_dirs`, the
  new tests and the pair unit table. No `cargo`, no build, no `toolbox`, `podman` or
  `distrobox`, nothing at GitHub. The only file written in the worktree is this report.
- Binaries: `…/review-bin/v4r24/release/deep-diff-forge`, SHA-256
  `0d42de401c9c246cf99bb404115e81f37f485e70d0b2d0a20f63b62b997180ec` (debug `dc89eff2…`
  hashed, not run); for "is it new" `…/v4r23/release/…` (`92d740a5…`). Judge: the real
  forge-warden (`…/target-fw/release/forge-warden`, `55bf9fe6…`) on the host.
- Bases: `…/hooks-review-16/d16/base-repo` (a `--no-local` clone of
  `hooks-round24/d11/base-repo`, `4f5fe7c`, admits v4r24); `d16/base-all` (my commit
  `b34f7ea` admits v4r22, v4r23 and v4r24, for the `v4r23` runs and `w15schema`). Every
  case tree under `d16/cases/`; a private HOME under `d16/` set inside every driver.
- Scripts and outputs: `/var/home/louranicas/.cache/ddf-t/hooks-review-16/notes/` (`s/`: my
  `w16*.py` and copies of the fifteenth's and Vesper's drivers, repointed; `rv11/`, `s14/`:
  the earlier corpora, repointed; `tools/treegit/git`: a Git that hangs only on the pair
  question in linked trees); outputs `notes/out/<script>-<scenario>-<build>.txt`; hashes in
  `notes/out/script-hashes.txt`; a dated log in `notes/NOTES.md`. I stopped no process.
- One slip against the machine rules, recorded: one `rm -rf` of my own copied
  `__pycache__` ran inside a compound command (my directory, nothing else touched).

## Section 0, item 1: the closed findings' reproductions on this build

`notes/run1.sh`, outputs `notes/out/*-v4r24.txt` and `*-v4r23.txt`.

| finding | script | v4r24 | v4r23 |
|---|---|---|---|
| VD4R23-M-1 | `v4r23-core-worktree-nested.py` | BLOCK; UNJUDGED "Git names …/outer as the working tree of a Git directory whose records name …/outer/inner as that tree", memory kept; from R "could not be found out", memory kept. **Closed** | BLOCK, warning with memory erased, SILENT |
| VD4R23-L-1 | `v4r23-v9-clean.py` | SILENT, schema v10, `trees` present. **Closed** | schema v9, no `trees` |
| HR15-L-1 | `w15cw.py copy` | steps 2 to 5 UNJUDGED or "could not be found out", memory kept at every step. **Closed** | memory erased at 2, SILENT at 3 and 5 |
| HR15-L-2 | `w15copy.py clean` | from C UNJUDGED, memory kept; from L FAIL STANDS (no second block). **Closed** | from L BLOCK a second time |
| HR15-L-2 | `w15copy.py keep` | from C UNJUDGED (was FAIL STANDS): as the round means | FAIL STANDS from C |
| ceiling note | `w15ceil.py dotdot` | from `H/notes/today` SILENT (H stays hidden). **Closed** | "another repository, H" |
| schema | `w15schema.py up`, `unlistedfirst` (`base-all`) | v4r22 v9 read, written v10; records removed UNJUDGED and named; v4r22 refuses v10. `unlistedfirst`: SILENT and forgotten, the stated limit | (as the fifteenth found) |

## Section 0, item 2: the pair rule

`w16pair.py`, `w16link.py` (outputs `notes/out/w16pair-*`, `w16link-*`), each on both builds:

| case | v4r24 | v4r23 |
|---|---|---|
| L's `commondir` edited to name another repository X | from L UNJUDGED ("records name no working tree that can be read"), from R "could not be looked at", memory kept | from L "another repository", from R "has changes" |
| I's administrative `gitdir` edited to name O's `.git` (Git lists O twice) | from R "could not be looked at" (twice), from I UNJUDGED, **from O SILENT** (HR16-L-1) | SILENT at every stop from R and O, memory erased |
| main `.git` a symbolic link to `store/proj/.git` | UNJUDGED at every stop (HR16-L-2) | BLOCK, FAIL STANDS |
| `--relative-paths` tree; and `mv` then `git worktree repair` | BLOCK, FAIL STANDS; moved: UNJUDGED until repair, then FAIL STANDS | the same, but FAIL STANDS straight after `mv` |
| `git worktree move` | FAIL STANDS from the new path | the same |
| tree added through `/home/…` (a link on this host), absolute and relative records; repair run from a `/home` path | Git records the resolved path; BLOCK, FAIL STANDS from either spelling | the same |
| the tree's parent moved and a link left at the old name (Git lists the path through the link) | FAIL STANDS from either spelling; the same tree is also named "has changes … the session's directory is not in it" (older, cosmetic) | the same |
| `cp -a L C`, `git clean`, `git worktree repair` in C | HR16-L-3 | the same silence; from L BLOCK |
| a copy of L inside R with a key | HR16-M-1 (SILENT from R) | SILENT from R |
| control: a nested clone inside R with a key | "a Git repository of its own … not judged", every stop | the same |

`GIT_DIR`, `GIT_WORK_TREE`, `GIT_COMMON_DIR`, `GIT_INDEX_FILE` are taken out for every Git
call (`deep-diff-forge-core/src/git.rs`; source). `repository_dirs` canonicalises all three
directories, `pair_of` compares canonical paths with the `gitdir` record resolved by
`same_form` (links followed), so case, trailing slashes, `/./` and links on either side reduce
to one form; `includeIf` that sets `core.worktree` is the `core.worktree` case. A tree id reused
is the stated birth-time limit.

## Section 0, item 3: the interpretations

- `--separate-git-dir` with a directory not named `.git` (`w16pair.py sep|seplinked`): a
  session begun in the main tree is judged as on `v4r23`; a session begun in a linked tree is
  UNJUDGED at a stop from the main tree, as stated. Right as a narrowing: Git keeps no record
  of the main tree's top there. Every stop of such a repository also names "the working tree
  `store/x.git` … is gone or is no working tree any more … (`git worktree prune` forgets it)"
  (Git lists the Git directory as the main tree): older, the same on `v4r23`, and the advice
  is wrong (prune does not remove a main tree). Wording, not counted.
- Named `X/.git` (untested by the implementer): HR16-L-2. The cheap rule that keeps the
  narrowing: for a main Git directory named `.git` whose parent is not Git's top, believe the
  top when it is the project's own top and its `.git` (file or link) names that Git
  directory, as for `Pair::Unrecorded`; a copy elsewhere still fails the "project's own top"
  test.
- A tree moved with `mv` and not repaired: UNJUDGED, then FAIL STANDS after repair (no second
  block). Right as stated. The reason does not say what to do; README does (`git worktree
  repair`). A remedy in the words would help a person.
- An unpaired answer about another repository: UNJUDGED instead of the "another repository"
  note (`commondir` case). Both are said; nothing is silent.

## Section 0, item 4: the added Git calls

`w16time.py 20 100` and `20 0.4` (a Git wrapper that sleeps only on the pair question run in a
linked tree; `notes/tools/treegit/git`), 20 linked trees, a failure standing in one, a new key
in another: the stop took 6.4 s and 6.5 s, said how many trees were not reached and that a
failure stands in one of them, forgot nothing; unarmed, the next stop named both trees and the
stop from the tree gave FAIL STANDS. Held.

## Section 0, item 5: the ceiling

`w16ceil.py` (both builds the same, and Git agrees): `X/..` with `X` a link into the tree
(Git resolves the link first): dropped, the tree judged; `::L/sub` (after an empty entry):
dropped; `::R/../H`: Git finds H after an empty entry (as written, no normalising), and the
hooks name H as another repository, as Git sees it; an entry written through `/home`: dropped.
The fifteenth's `alias`, `ceilrel` I did not rerun (unchanged code paths). Held.

## Section 0, item 6: state

`w16state.py` (v4r24): a v9 state with a remembered linked failure, then a stop with a missing
judge: UNJUDGED, written as v10 with the tree recorded, the next real stop FAIL STANDS; the
state directory read-only: the stop says the state could not be kept, the file stays v9 and
whole, and once writable it is written as v10 with nothing lost or doubled; a v9 state, then a
stop from a copy (UNJUDGED): written as v10, memory kept, then the records removed: named from
R, UNJUDGED from L. Held.

## Section 0, item 7: the sentences

True where said, checked against runs: README's pair paragraph for `core.worktree`, the copy
(`cp -a` elsewhere), the moved tree, the linked-session separate-git-dir case; SECURITY's
"believed only as a pair"; the CHANGELOG round entry, whose tests exist
(`nested_trees_one_pointed_at_the_other_are_unjudged_and_keep_what_blocked`,
`a_tree_pointed_at_a_copy_inside_itself_is_unjudged_and_keeps_what_blocked`,
`a_copy_of_a_linked_tree_is_not_judged_as_that_tree`,
`a_copy_of_a_project_whose_git_directory_was_made_apart_is_not_judged_as_it`,
`a_clean_state_of_the_schema_before_is_written_anew_at_the_first_stop`,
`a_ceiling_written_through_the_project_still_hides_its_repository`, the pair unit table).
The submodule Git-directory UNJUDGED is now stated in README, SECURITY and CHANGELOG.

Wider than the code:

- README "A nested repository in the project's tree … is left out of the patch and named" and
  "nothing that is known in the project stays outside what was judged": HR16-M-1.
- The doc comment of `left_out`'s new arm ("a tree of this repository Git names with another's
  Git directory: named by the rule for working trees, not as a repository of its own"): no
  rule names an unlisted copy.
- README "(its Git directory made apart with `--separate-git-dir`)": not for one named `.git`
  (HR16-L-2). SECURITY says only "the main tree's top as `git worktree list` gives it", which
  for a separate Git directory not named `.git` is the Git directory itself: read literally it
  says such a session is UNJUDGED, which the code does not do. One clause would fix it.
- SECURITY "nothing of a tree's identity is taken from a path, as … Git records it": the skip
  in `listed_others` is by path (HR16-L-1).

## Section 8: the eleventh brief's sections 1 to 7, briefly

- The fifteenth's corpus on v4r24 (`notes/run16c.sh`, one HOME per lane; compared with the
  fifteenth's `v4r23` outputs by `s14/bcmp.py` and `s14/mcmp.py`; no output holds "was not
  found" or a traceback): the `nr` and `mi` matrices 144 of 144 files the same at every step;
  the batch 32 of 39 the same by `bcmp`, `wt2 256` differing in commit ids only, and the six
  files with no decision column compared by hand after taking out paths, ids and times:
  `cp`, `missing_git_entry`, `ot 31 clean`, `w14home2`, `w14trouble` the same. `cn2` (a linked
  tree whose path holds a newline and an injected sentence) changed: `v4r23` looked at it
  ("has changes that are not committed"), `v4r24` says "could not be found out (git
  rev-parse did not name the repository's directories)", because the new pair question
  splits Git's answer by lines. Still said at every stop, the injected text still escaped
  (`\x0a`); a tree path with a newline is not an ordinary workflow. Not counted.
- Sections 1 (a pass that should not be one), 2 (blocks that fire forever or are waved
  through), 3 (the input contract), 5 (`agent setup`, `agent doctor --gate`), 6 (`schema
  signals`, `explain signal`): no code of theirs changed in this round except `gate` gaining
  the `Unpaired` arm (doctor not ready, with the reason, where a stop would be UNJUDGED: the
  two agree). Not rerun beyond the corpus.
- Section 4: the time runs above; two stops at once not rerun.
- Section 7, the tests: read, not run. The new tests use real Git and the reading judge and
  would fail without their guards (the implementer's control says each fails on `c9a2ece`).
  Missing: a copy of a linked tree INSIDE the main tree (HR16-M-1), two listed entries with
  one path (HR16-L-1), a separate Git directory named `.git` or a `.git` link (HR16-L-2),
  `git worktree repair` run in a copy (HR16-L-3).

## Findings reproduced with the real judge

Every command below is a driver in `/var/home/louranicas/.cache/ddf-t/hooks-review-16/notes/s/`
run with `DDF=…/review-bin/<build>/release/deep-diff-forge`, `BASE=…/d16/base-repo` (v4r24) or
`…/d16/base-all` (v4r23), via `notes/run2.sh` or `notes/run3.sh`.

### HR16-M-1 (medium; ORDINARY WORKFLOW; older, kept by this round): a copy of a linked tree inside the main tree is silent

Claim: README "nothing that is known in the project stays outside what was judged. The last
clause is the rule for what a stop names beside its verdict: a nested repository that was left
out, …" and "A nested repository in the project's tree (a scratch `git init`, a checkout) is
left out of the patch and named"; the round-14 rule (request claim 1). Class: a copy of a
linked tree (HR15-L-2's class).

```
python3 -I w16pair.py copyinside          # out/w16pair-copyinside-v4r24.txt
   cp -a L R/feature-copy; a key in R/feature-copy/lib/incopy.rs
   git status in R: '?? feature-copy/'   (Git sees an embedded repository and does not descend)
-- 1 cwd = R                => SILENT
-- 2 cwd = R again          => SILENT
-- 3 cwd = R/feature-copy   => allow:UNJUDGED ("Git names …/feature-copy as the working tree of a
                               Git directory whose records name …-feature as that tree")
-- 4 cwd = R                => SILENT
python3 -I w16pair.py nestedclone         # control: a nested clone at the same place
-- 1 cwd = R                => allow "The directory "vendor-x" … is a Git repository of its own …
                               left out and not judged."   (and counted at every stop after)
```

v4r23 (`out/w16pair-copyinside-v4r23.txt`): SILENT from R as well, and from the copy BLOCK
(it judged the copy as L). The cause is `left_out` (`hook_run.rs`, about line 3205): a directory
left out of the patch is not named when Git places it in this repository, and this round added
`Found::Unpaired(unpaired) => unpaired.id == stopping.work.place.id` beside `Found::Tree`, with
the comment that such a directory is "named by the rule for working trees"; but that rule
(`other_trees`) walks `git worktree list`, and a copy is not listed. Why medium and not high:
the credential cannot reach the project's history unjudged. A commit made from the copy goes
to L's branch and BLOCKs at the next stop (`w16pair.py copyinsidecommit`:
`Commit d348ed050cd2: FAIL`), and `git add -A` in R stages only a gitlink (HOLD,
`patch_unverifiable`). But by the eleventh brief's rule an ordinary workflow that ends in a
silent stop with unjudged work in the project is high; a reader who weighs it by the letter
should read it so. How likely: copying a worktree with `cp -a` is a known agent mistake on
this machine; copying it into the project directory is the same mistake with a relative
destination. Cheap fix (a line and a test): in `left_out`, an unpaired directory is not a
working tree; name it (best as "a copy of the working tree … of this project, not judged").

### HR16-L-1 (low; DELIBERATE EVASION; older, narrowed by this round): two listed entries with one path

Claim: the round-14 rule (request claim 1); SECURITY "nothing of a tree's identity is taken
from a path, as a message shows it or as Git records it". Class: path taken for identity.

```
python3 -I w16pair.py gitdirdupa          # out/w16pair-gitdirdupa-v4r24.txt (and gitdirdupz)
-- 1 key in I; cwd = I                     => BLOCK   mem: ed4ce959188914a2/u:1
   R/.git/worktrees/<I>/gitdir := O/.git;  git worktree list: R, O, O
-- 2 second key in I; cwd = R              => allow "… O … could not be found out (Git names O … with
                                              the Git directory …, which is not this tree's)"  (kept)
-- 3 cwd = O                               => SILENT  mem: ed4ce959188914a2/u:1
-- 4 cwd = R again                         => allow "2 things … not judged: O (… a failure stands
                                              there), O (… a failure stands there)"
-- 5 cwd = I                               => allow:UNJUDGED (Git names I … whose records name O)
```

v4r23: SILENT at steps 2, 3 and 4, memory erased. At step 3 both listed entries carry the path
O, which is the judged tree, so `listed_others` drops both (`!judged.contains(&tree.path)`), and
I's id appears at no entry: its failure is neither looked at nor said. Memory is kept. How
likely: it needs a hand edit of Git's administrative file. Cheap fix: drop a listed entry as
judged only when its id (`listed_instance`) is the judged tree's, or say every remembered,
listed id no entry carried.

### HR16-L-2 (low; ORDINARY WORKFLOW, uncommon; new): a main Git directory named `.git` elsewhere

Claim: README "Where a repository records no top level of its main tree (its Git directory
made apart with `--separate-git-dir`), the main tree's top is the project's when the session
began in the main tree". Class: the pair rule's main-tree side.

```
python3 -I w16pair.py sepdot              # git clone --separate-git-dir=<store>/x/.git
-- 1 key in R; cwd = R        => allow:UNJUDGED "This stop was not judged: Git names R as the working
                                 tree of a Git directory whose records name <store>/x as that tree …"
-- 2 cwd = R again            => the same
-- 3 key in L; cwd = L        => BLOCK, and "This session's start was not recorded …" (the project is
                                 recorded only now, from L)
python3 -I w16pair.py dotgitlink          # R/.git moved to <store>/proj/.git, a link left at R/.git
-- 1, 2 cwd = R               => UNJUDGED at both
```

v4r23: BLOCK, then FAIL STANDS in both. Nothing is silent and nothing passes, but a session in
such a repository is never judged and its failures never block, and the start it should have
recorded is recorded later from elsewhere. `pair_of` takes a main Git directory named `.git`
to be the main tree's, at its parent. Cheap fix: see "the interpretations" above, or narrow
README to "a Git directory not named `.git`".

### HR16-L-3 (low, or a limit to state; ORDINARY WORKFLOW, uncommon; older): `git worktree repair` run in a copy

Claim: SECURITY/README "A linked tree a failure is remembered of stays remembered while its
directory stands"; round-14 rule. Class: a copy of a linked tree.

```
python3 -I w16pair.py copyrepair          # out/w16pair-copyrepair-v4r24.txt
-- 1 key in L; cwd = L                     => BLOCK   mem: b53ff0c83afc1da2/u:1
   cp -a L C; git -C C clean -fdq; git worktree repair (in C): "gitdir incorrect"; list: R, C
-- 2 second key in L; cwd = R              => SILENT  mem: -
-- 3 cwd = C                               => SILENT
-- 4 cwd = R                               => SILENT
-- 5 cwd = L                               => allow:UNJUDGED (records name C)
```

After the repair Git itself names C as the tree, so reading C clean under L's id is what
`git worktree move` would also give; the hooks keep the tree's top in the v10 state and could
see that the recorded directory still stands with a `.git` naming the same administrative
directory (then: name it, forget nothing). v4r23 was silent at 2 to 4 too, and at 5 it blocked;
v4r24 is UNJUDGED there, so L's two keys never block while it stands. I would state it as a
limit ("`git worktree repair` run in a copy makes the copy the tree; the directory copied is
then no tree of the project") rather than grow the code, unless the fix for HR16-M-1 already
touches the same place.

## Suspected only (not reproduced)

- An `includeIf "gitdir:…"` in the user's global configuration that sets `core.worktree`
  reduces to the `core.worktree` cases (held); not run.
- A tree that cannot be looked at (`chmod 000`): by reading, `names_the_tree` returns Git's
  words and the tree is "could not be looked at", nothing forgotten; not run.

## What I did not look at

`verdict_v2.rs`, `vocabulary.rs`, `agent setup`, `schema signals`, `explain signal`
(unchanged by this round); two stops at once; a full disk; the mutation run (no `cargo`);
`w15ceil.py alias|ceilrel` and the btrfs subvolume cases (unchanged code); the debug binary
(hashed, not run); an exhaustive search for a second ordinary path to two listed entries with
one path (I found none; Git refuses `worktree add` at a registered path and `add -f` deletes
the old administrative directory).
