# Fourteenth independent review of the agent hooks branch (`agent/loop`, commit `8959ca8`)

Date: 2026-11-16, 14:30 to 14:45 AEDT by the host's clock (`date`). Reviewer: an independent
Claude agent that took no part in the work.
Brief: `~/.local/state/habitat-claude/ddf-gap-probes-20261113/ddf-hooks-independent-review-14-brief.md`
(extends the eleventh brief, read whole). Under review: `d4dbc91..8959ca8`, mainly the rework
`67202b1..8959ca8`.

## Verdict: GO-WITH-FIXES

Every finding round 22 answers is closed on the frozen binary with the real judge:
VD4R21-M-1 = HR13-L-1 (`hooks/feature`, `logs/feat`, `svn`), VD4R21-L-1, VD4R20-M-1 in its
reported layout, HR12-L-4, HR13-L-2 (Git's words first; `NOTES_MERGE_WORKTREE` and `common`
UNJUDGED as now stated), and the two suspicions of the thirteenth review (a ceiling inside a
linked tree; `..` after a link). The twelfth and thirteenth reviews' matrices (`nr`, `mi`: 144
files) and 25 other scripts give the same decision at every step as on `v4r21`, except the one
step the round means to change (a stop from inside a Git directory is now UNJUDGED). I found no
stop that prints nothing while an ORDINARY WORKFLOW leaves something open in the project, and
no unchanged failure that blocks twice.

Four lows; no high and no medium.

- **HR14-L-1** (low; ORDINARY WORKFLOW in a rare configuration; NEW in this commit; noise and
  time, never a pass, never a block). Emptying `GIT_CEILING_DIRECTORIES` for every Git call of
  the hooks also un-hides a repository the user hides with a ceiling (a home-directory
  repository). A session that starts below it, in a directory that is in no repository to the
  user's Git, now takes that repository for its project: "deep-diff-forge is watching the
  changes of this session in <home>", every stop UNJUDGED ("no policy file") while the home
  repository has a change, and every tool call pays a read of the whole home (1.0 to 1.3 s
  with 20,000 untracked files, where `v4r21` took 0.00 s; the advice limit is 4 s). From a
  session with a real project, a stop made from a hidden directory now says "The session's
  directory is in another repository, <home>". Not stated.
- **HR14-L-2** (low; DELIBERATE EVASION; older, `v4r21` the same). VD4R20-M-1's outcome in the
  ordinary layout: a linked tree beside the project (not in the Git directory), both records
  removed, or its `.git` file removed and then `git worktree prune`, which the stop's own
  message advises. Git places the directory in no repository (it does not fail), so round 22's
  rule does not reach it: the next stop from inside it is SILENT with a second key there, and
  the failure that blocked there is forgotten. SECURITY's new sentence says such a stop is
  UNJUDGED "wherever its directory lies".
- **HR14-L-3** (low; ORDINARY WORKFLOW, rare; a stated limit stated too narrowly; older). The
  file system limit reads "a directory on another file system mounted inside a linked tree";
  a btrfs subvolume made inside a linked tree with `btrfs subvolume create` (no mount, no
  privilege; btrfs is Fedora's default) is the same to Git: the tree is not judged from there
  and is named "the session's directory is not in it", which is false. With
  `GIT_DISCOVERY_ACROSS_FILESYSTEM=1` it is judged and blocks.
- **HR14-L-4** (low; DELIBERATE EVASION, implausible by accident; older). `core.worktree` in a
  linked tree's own configuration (`extensions.worktreeConfig`, `git config --worktree`)
  pointed at the project's top: from the tree, Git says it is the project; the next stop is
  SILENT with two keys in the tree, and what blocked there is forgotten (the judge's reader
  reads the project through the redirect).

What I would do before a merge: HR14-L-1 is a regression of this commit and cheap to remove
(empty only the ceiling entries that lie at or under the top of one of the project's trees,
which the hooks know from the administrative directories; or keep the session's own ceiling
for the start's choice of project and for a `Foreign` answer in `elsewhere`). The other three
want a sentence each, or the cheap fixes under them.

## How this was done

- Source from `git archive HEAD` (`/var/home/louranicas/.cache/ddf-t/hooks-review-14/src`);
  `git diff 67202b1..HEAD -- crates/` in `…/hooks-review-14/notes/r22.diff`. No `cargo`, no
  build, no `toolbox`, `podman` or `distrobox`, nothing at GitHub, no network. The only file
  written in the worktree is this report.
- Binary: `/var/home/louranicas/.cache/ddf-t/review-bin/v4r22/release/deep-diff-forge`,
  SHA-256 `4709a2d02f0725b9c518e173ffaebed19fbe58232c6922cbca3a1dbfea33c58c` (checked; debug
  `c0f80fdd…` checked, not run). For "is it new": `…/v4r21/release/…` (`9bc26668…`) with a base
  reset to the commit that admits it (`d14/base-r21`, `cc312d8`). Judge: the real forge-warden
  (`…/target-fw/release/forge-warden`) on the host.
- Base: `…/hooks-review-14/d14/base-repo`, a `--no-local` clone of
  `hooks-verify22/d11/base-repo` (`389e1d1`, admits v4r22). Case trees under
  `…/hooks-review-14/d14/cases/`; private HOME `…/hooks-review-14/d14/home`, set inside the
  driver (`lib11.py`). One btrfs subvolume made by `w14vol.py` was emptied and removed after
  its run.
- Scripts and outputs: `/var/home/louranicas/.cache/ddf-t/hooks-review-14/notes/s/` (the
  thirteenth review's scripts with their paths changed; Vesper's two round 21 reproductions
  and `missing_git_entry.py` repointed; new: `w14ext.py`, `w14prune.py`, `w14home.py`,
  `w14home2.py`, `w14home3.py`, `w14vol.py`, `w14cw.py`, `w14ceil.py`, `w14git.py`,
  `w14trouble.py`); outputs in `notes/s/out/` and `notes/s/out/m/`; SHA-256 of every script in
  `out/script-hashes.txt`.
- At most two measuring lanes at a time (`run-b.sh`, then `run-m.sh`, each waiting above Tctl
  80 °C or Composite 70 °C; neither had to wait; Tctl 58 °C, Composite 46 °C when checked). I
  stopped no process.

## Section 0, item 1: the closed findings' reproductions on this build

| finding | script (output in `notes/s/out/`) | result on v4r22 |
|---|---|---|
| VD4R21-M-1 | `v4r21-reserved-subtree.py` (`.txt`) | first BLOCK; second `allow:UNJUDGED` "`git rev-parse` failed: fatal: this operation must be run in a work tree. Whether … it is inside the Git directory …"; memory `…/u:1` kept. **Closed** |
| VD4R21-L-1 | `v4r21-relative-locked.py` | UNJUDGED; the linked tree "The session's directory is under the working tree … whether it is in that tree could not be found out"; memory kept. **Closed** |
| HR13-L-1 | `w13a.py top|sub|hooksx` | stop 2 (records removed) UNJUDGED, memory kept, second key not judged; stop 3 from `main` SILENT and forgets (the tree is no longer listed: as stated); stop 4 from it UNJUDGED again. **Closed** |
| HR13-L-2 | `w13c.py` | `.git/NOTES_MERGE_WORKTREE` and `.git/common` UNJUDGED with Git's words whole and "it is inside the Git directory"; README now names that consequence. **Closed** (as stated) |
| HR13-L-2, sentences | README, SECURITY diff | "forgets nothing because a tree is no longer listed (a tree the judge's own reading finds clean is forgotten …)". **Closed** |
| claim 3 | `w13b.py forget` | stop 4 from `.git/lfs/tmp`: R and L (repaired) forgotten by the judge's reading, M (removed) kept; stops 5 to 7 as on v4r21. Held |
| VD4R20-M-1 | `missing_git_entry.py` | UNJUDGED, memory kept. **Closed** in its layout (see HR14-L-2 for the other) |
| HR12-L-4 | `w12b.py locked` | UNJUDGED with "Permission denied"; the tree "may be in it", "stands". **Closed** |
| ceiling (13th, suspected) | `w14ceil.py linked` | ceiling = the linked tree's top, stop from `L/sub/deep`: BLOCK on L, then FAIL STANDS; v4r21: not judged, "the session's directory is not in it". **Closed**; `w14ceil.py project` (session begun under a ceiling at the project) BLOCK; `gate` unaffected by the ceiling |

## Section 0, item 2: any path to silence or forgetting at a stop Git cannot place

Read: `find`, `elsewhere`, `unplaced`, `standing_directory`, `Unfound`, `through_links`,
`trees_to_judge`, `other_trees` at `8959ca8`. Every `Found::Failed` is `Elsewhere::Failed`, and
`Elsewhere::Failed` always sets `unfound`, so `other_trees` never sets `listed` at such a stop
and every tree the directory may be in is named, not looked at, not forgotten. The paths to
`Elsewhere::Nothing` that remain are: `standing_directory` returning `Ok(None)` (only for a
relative path that does not exist: an absolute one always reaches `/`); `cwd == place.top`;
`Found::Tree` of the project; `Found::Gone` (the canonical directory is not a directory: a
`cwd` that names a file, or a race); and `Found::NotARepository`. The last is where the lows
are: Git *places* the directory, in no repository, when a linked tree beside the project lost
both records (HR14-L-2) and when a file system boundary lies between the directory and its
tree (HR14-L-3). `GIT_DIR`, `GIT_WORK_TREE`, `GIT_COMMON_DIR` stay removed by the shared list;
`core.worktree` in a tree's own configuration redirects the answer (HR14-L-4); a `.git` that
is an empty directory is passed over by Git (the tree above is found), a `.git` file with
garbage or naming a missing directory is `Failed` (UNJUDGED; `w14ext.py admin`); Git failing
on one command only for the project (`w14trouble.py`, a project path of about 290 characters): UNJUDGED,
Git's words readable (the path is shortened in the middle). `safe.directory` and `includeIf`
are the user's global configuration, which the hooks do not override: not tested further.

## Section 0, item 3: where the session's directory is

`Unfound::of` anchors a relative path at the hook's own directory; `through_links` steps a
`..` up from where the part before it leads, and gives `Anywhere` when that part cannot be
resolved (a loop, a directory that cannot be searched); `..` past `/` stays at `/`. Vesper's
`relative-locked` and the round's `..`-after-a-link test cover the cases a process can produce
(Claude Code gives the process's directory: absolute, no `..`, no loop). A `cwd` under another
repository is `Foreign`, as before. The stated mount limit is narrower than what Git does
(HR14-L-3). On `GIT_DISCOVERY_ACROSS_FILESYSTEM`: leaving it alone keeps a false "the
session's directory is not in it" for a directory across a boundary inside a linked tree.
Reading Git's "Stopping at filesystem boundary" as a failure would be wrong: on this machine
`/var` is a mount point, so every "not a git repository" there carries those words
(`w14ext.py`, step 2). Git names the mount point it stopped at ("or any parent up to mount
point X"); when X lies at or under a tree of this project the stop can ask again with the
variable set, or call it `Failed`. Setting it for every call is the simpler fix, with the same
side effect as HR14-L-1 across mounts.

## Section 0, item 4: ordinary workflows from inside a Git directory

`w14git.py dotgit|hooks|lfs|rebase|am|submodule` (outputs `w14git-*.txt`), each: the project
clean twice, then a key in the project, then again, then a stop from the project.

| stop made from | clean | key | again | from R |
|---|---|---|---|---|
| `.git`, `.git/hooks`, `.git/lfs/tmp` | UNJUDGED (one line) | BLOCK on the project + one UNJUDGED line | UNJUDGED + FAIL, which stands | FAIL STANDS |
| `.git/rebase-merge` (`rebase -i` stopped at an edit), `.git/rebase-apply` (`git am` conflict) | the same | the same | the same | the same |
| `.git/modules/sub` (a submodule's Git directory) | HOLD (the gitlink commit) + "in another repository … sub" | BLOCK | HOLD, FAIL stands | as before |

The post-tool hook says nothing from any of them. Nothing blocks that did not, nothing blocks
twice, and the project is judged at every stop; the cost is one UNJUDGED line per stop made
from there, whose words are whole. Two remarks, not findings: the line reads
`Uncommitted changes in ".../.git/hooks": UNJUDGED`, which names a Git directory as a place
with uncommitted changes; and a stop from the common directory itself (`.git`, or `repo.git`
in the bare layout, where an agent runs `git worktree add`) could stay quiet safely, since no
working tree can have its top at the common directory.

## Section 0, item 5: the ceiling

Every Git call in `hook_run.rs` takes its options from `Deadline::git()` (`deadline.git()`,
`end.git()`, `stopping.deadline.git()`: every call site a search finds), and every runner in
`deep-diff-forge-patch` keeps the caller's `env`; `doctor.rs` (`agent doctor` without
`--gate`) still honours a ceiling, which is right for it. The judge and the ranker helper are
not Git. No call misses it. What the change does beyond its purpose is HR14-L-1.

## Section 0, item 6: the sentences

True where said: README's paragraph on directories Git cannot place (the reason's words, the
tree "may be in it", "forgets nothing because a tree is no longer listed"), its ceiling
sentence, SECURITY's matching sentences, the CHANGELOG round entry (its tests exist:
`a_tree_under_a_name_git_uses_whose_records_were_removed_is_unjudged`,
`gits_words_are_never_cut_away`, `a_relative_directory_that_cannot_be_resolved_is_anchored`,
`a_parent_step_after_a_link_is_taken_where_the_link_leads`,
`a_ceiling_in_the_environment_does_not_hide_the_tree`).

Wider than the code, or missing:

- SECURITY: "each time a tree whose `.git` file and administrative directory had both been
  removed … was a silent stop … Such a stop is UNJUDGED with Git's words first, wherever its
  directory lies": not for a tree outside the Git directory (HR14-L-2).
- README: "a directory on another file system mounted inside a linked tree": also a btrfs
  subvolume, with no mount (HR14-L-3).
- README, SECURITY, CHANGELOG: nothing says that with the ceiling gone a repository the user
  hid is found (HR14-L-1).

## Section 0, item 7: the eleventh brief's sections 1 to 7, briefly

- Matrices `nr` (12 modes by 7 changes) and `mi` (sizes 1, 200, 256, 257, 600 by two halves
  by six changes): 144 of 144 files with the same decision at every step as the thirteenth
  review's outputs on v4r21 (`mcmp.py`, `out/m/`).
- `a1`, `a3`, `par`, `cp`, `cn`, `cn2`, `mi7 600`, `ot 31 clean`, `ak 0 17`,
  `el bare|ingit|env|sub`, `w12a ingitfail|nestfail|baredmin|modules|gone|gonegit`,
  `w12b never|skipwt|failread|repaired|gitdir`, `wt2 256`: 25 files (`bcmp.py`, `out/b-*.txt`);
  the same decisions as on v4r21 except `w12a gonegit` step 3 (a deleted directory under
  `repo.git/worktrees/feature`: UNJUDGED, was quiet; the round's meant consequence; `main`
  judged, `feature` named); `wt2` differs in commit ids only; `cn2`, `cp`, `ot` compared by
  hand: the same text but the commit and, in `cn2`, Git's words now before the path (a path
  with a newline and an injected sentence stays escaped as `\x0a`).
- Sections 2, 5, 6 (`agent setup`, `agent doctor --gate`, `schema signals`, `explain signal`):
  not changed by this commit; `agent doctor --gate` run once under a ceiling (`w14ceil.py gate`):
  it fails only for the missing registration, as it should.
- 7, the tests: read, not run (no `cargo`). The integration tests use real Git and the
  reading judge; the ceiling test sets the variable on the hook's own command, so it fails if
  `NO_CEILING` is dropped from `Deadline::git()` (the implementer's control says so). No test
  has a linked tree outside the Git directory with both records removed (HR14-L-2), a
  ceiling that hides a repository the session is not in (HR14-L-1), a subvolume or mount
  (HR14-L-3) or `core.worktree` (HR14-L-4).

## Findings reproduced with the real judge

### HR14-L-1 (low; ORDINARY WORKFLOW, rare configuration; NEW): the ceiling is emptied also where it hid a repository on purpose

`Deadline::git()` puts `GIT_CEILING_DIRECTORIES=` into every Git call of the hooks, also the
`find` that chooses the session's project at its start and the `find` of `elsewhere` whose
`Foreign` answer is said at every stop. A user who hides a home-directory repository with a
ceiling (a documented use of the variable) and registers the hooks for every project
(`~/.claude/settings.json`, which `agent setup --hooks` offers) gets:

```
python3 -I /var/home/louranicas/.cache/ddf-t/hooks-review-14/notes/s/w14home.py noproject    # out/w14home-noproject-v4r22.txt
   git in H/notes/today with the user's ceiling: rc=128 fatal: not a git repository (or any of the parent directories): .git
   session-start rc=0 out='deep-diff-forge is watching the changes of this session in "…/w14home-noproject-…-H". …'
-- 1 stop, session in H/notes/today (no repository to the user)  => allow:UNJUDGED
        | deep-diff-forge: UNJUDGED. This change (…) was not judged: the base commit has no policy file "warden.toml". That is not a pass.
   (v4r21, out/w14home-noproject-v4r21.txt: start says nothing, the stop is SILENT, no state written)

python3 -I …/w14home3.py 20000     # out/w14home3-v4r22.txt, -v4r21.txt; 20,000 untracked files in H
   v4r22: start 0.73s; post-tool 1.28s, 1.02s, 1.16s; stop allow 0.74s UNJUDGED
   v4r21: start 0.00s; post-tool 0.00s ×3; stop SILENT 0.00s

python3 -I …/w14home.py outside    # a session with a real project; a stop from H/notes/today
   v4r22: allow "The session's directory is in another repository, "…-H", which is not this session's project …"
   v4r21: SILENT
```

Never a pass and never a block; the harm is a session attached to the wrong repository, an
UNJUDGED line at every stop while the home repository has any change (a home repository
nearly always has one), and up to the advice limit (4 s) on every tool call in a real home.
Cheap: empty only the ceiling entries that are at or under the top of a tree of this project
(the hooks read those tops from the administrative directories), or keep the session's
ceiling for the start's `find` and treat a `Foreign` answer that only the emptied ceiling
allowed as `NotARepository`; and say in README which ceiling is ignored.

### HR14-L-2 (low; DELIBERATE EVASION; older): a linked tree beside the project with both records removed is silent from inside and forgotten

```
python3 -I /var/home/louranicas/.cache/ddf-t/hooks-review-14/notes/s/w14ext.py both    # out/w14ext-both-v4r22.txt
-- 1 key in L; cwd = L                                   => BLOCK   mem: 43d60cee97908b58/u:1
   git in L: rc=128  fatal: not a git repository (or any parent up to mount point /var)
                     Stopping at filesystem boundary (GIT_DISCOVERY_ACROSS_FILESYSTEM not set).
-- 2 records removed (both); a second key; cwd = L       => SILENT  mem: -
-- 3 cwd = L again                                       => SILENT  mem: -
-- 4 cwd = R                                             => SILENT  mem: -
python3 -I …/w14prune.py                                  # out/w14prune.txt
-- 1 key in L; cwd = L                                   => BLOCK   mem: 369699798caa77ee/u:1
-- 2 L/.git removed; cwd = L                             => allow "… is gone or is no working tree any more … (`git worktree prune` forgets it). A failure there … stands"
   git worktree prune
-- 3 after `git worktree prune`; a second key; cwd = L   => SILENT  mem: -
```

v4r21 the same (`out/w14ext-both-v4r21.txt`). Only the `.git` file removed: the tree is
named "(gone)" with its failure standing (`w14ext.py file`); only the administrative
directory removed: Git fails ("gitfile does not point to a valid repository"), UNJUDGED with
memory kept (`w14ext.py admin`). The code is consistent with its definition (Git places the
directory: in no repository), and README says that what is remembered of a tree Git no
longer lists is forgotten. But the round's sentences read wider: SECURITY "Such a stop is
UNJUDGED with Git's words first, wherever its directory lies", README "No stop is silent
because of where its directory lies"; in the most common layout of linked trees the outcome
of VD4R20-M-1 (silent, credentials in the directory, the failure that blocked there
forgotten) stands. How likely: it needs the `.git` file removed by hand; the second step is
what the stop itself advises. How harmful: less than it looks: the directory is in no
repository any more, so nothing there reaches a commit without a new tree, which is judged
anew. Cheap: at the stop that forgets a remembered failure because its tree is no longer
listed, say so once ("what blocked in the working tree X is forgotten: Git no longer lists
it"); and narrow the SECURITY sentence to trees inside the Git directory, or state this case.

### HR14-L-3 (low; ORDINARY WORKFLOW, rare; a stated limit too narrow; older): a btrfs subvolume inside a linked tree

```
python3 -I /var/home/louranicas/.cache/ddf-t/hooks-review-14/notes/s/w14vol.py           # out/w14vol.txt
   subvolume create: rc=0 Create subvolume '…-feature/vol'      (unprivileged; st_dev L=64, L/vol=140)
   git in L/vol/sub: rc=128 fatal: not a git repository (or any parent up to mount point …-feature) / Stopping at filesystem boundary …
-- 1 key in L; cwd = L/vol/sub          => allow "The working tree "…-feature" of this project has changes that are not committed, and the session's directory is not in it: they were not judged …"
-- 2 a key in the subvolume too         => allow "1 thing in this project is not judged: the working tree …-feature"
-- 3 cwd = L                             => BLOCK (lib/first.rs, vol/sub/k.rs)
-- 4 a second session with GIT_DISCOVERY_ACROSS_FILESYSTEM=1; cwd = L/vol/sub => BLOCK
```

Never silent (the tree is named as not judged), nothing forgotten; but "the session's
directory is not in it" is false, and README's limit says "mounted", which a subvolume made
with one unprivileged command is not. Cheap: as in section 0 item 3, or add "or a btrfs
subvolume made inside it" to the limit.

### HR14-L-4 (low; DELIBERATE EVASION, implausible by accident; older): `core.worktree` in a linked tree's own configuration

```
python3 -I /var/home/louranicas/.cache/ddf-t/hooks-review-14/notes/s/w14cw.py            # out/w14cw.txt
-- 1 key in L; cwd = L                                  => BLOCK   mem: cf25c5ddd30a8750/u:1
   git config extensions.worktreeConfig true; git -C L config --worktree core.worktree R
   git in L now: <R> | status: ''
-- 2 L's core.worktree = R; a second key; cwd = L       => SILENT  mem: -
-- 3 cwd = R                                            => SILENT  mem: -
```

v4r21 the same (`out/w14cw-v4r21.txt`). The tree's two keys are not judged and what blocked
there is forgotten, because the judge's own reader of L reads R. Nothing in L can be
committed while the redirect stands, so the harm is the silence. Cheap: `find` knows the
tree's own Git directory (`git_dir`, under `worktrees/`) and the hooks read the path each
administrative directory records; a top that is not the directory its `gitdir` file names
is `Failed`.

## Suspected only (not reproduced)

- A `cwd` that names a file inside a linked tree is `Found::Gone` after `canonicalize`
  succeeds (`find` checks `is_dir`), so `Elsewhere::Nothing`: the tree is named "not in it".
  Claude Code gives a directory; not run.
- The same shortcut through the ceiling across a slow network mount (`/net`, `/afs`), which
  is the other documented use of `GIT_CEILING_DIRECTORIES`: discovery walks up into it now,
  bounded by the hook's deadline. Not run.

## What I did not look at

`verdict_v2.rs`, `vocabulary.rs`, `agent setup`, `schema signals`, `explain signal` (unchanged
by this commit); the time matrices (`tm.py`, `st4.py`, `worst.py`), `wipe.py`, `km.py`,
`lm.py`, `bare2.py`; more than 2,048 failing lines; a full disk; `core.autocrlf` and
`core.fileMode=false`; a real mount (only a btrfs subvolume); `safe.directory` and `includeIf`
in a global configuration; the mutation run (no `cargo`); Vesper's round 22 report (it appeared in the worktree,
untracked, while I worked; I did not read it).
