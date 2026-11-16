# Thirteenth independent review of the agent hooks branch (`agent/loop`, commit `67202b1`)

Date: 2026-11-16, 12:20 to 12:33 AEDT by the host's clock (`date`). Reviewer: an independent
Claude agent that took no part in the work.
Brief: `~/.local/state/habitat-claude/ddf-gap-probes-20261113/ddf-hooks-independent-review-13-brief.md`
(extends the eleventh brief, read whole). Under review: `d4dbc91..67202b1`, mainly the rework
`4b134bd..67202b1`.

## Verdict: GO-WITH-FIXES (neither fix blocks a merge on its own)

Every finding of round 20 and of the twelfth review is closed on the frozen binary with the
real judge: VD4R20-M-1 in the layout reported, VD4R20-L-1, and HR12-L-1 to L-5. The
twelfth review's two matrices (`nr` 84 cells, `mi` 60 cells) and 25 of its other scripts
give the same decision at every step as on `v4r20` (169 of 169 files; the only differences
are commit ids). I found no stop that prints nothing while an ORDINARY WORKFLOW leaves
something open, nothing forgotten that the judge's own reading would still see, and no
unchanged failure that blocks twice.

Two lows; no high and no medium.

- **HR13-L-1** (low; DELIBERATE EVASION in a rare layout; older, `v4r20` the same).
  VD4R20-M-1 is still open for a linked tree that lies at or under one of the 17 names
  (`repo.git/svn`, `repo.git/logs/feat`, `repo.git/hooks/feat`; `git worktree add` makes
  them without complaint): with its `.git` file and administrative directory both removed,
  the next stop from it is SILENT with a second key there, and the failure that blocked
  there is forgotten. Claim 2 ("no tree a session works in is silent because its records
  were removed") does not hold for that layout, and README's "each holds Git's own files
  only" is not true of it. Not stated as a limit.
- **HR13-L-2** (low; ORDINARY WORKFLOW, rare; new in this commit; noise, never a pass).
  Directories Git itself makes or documents in the Git directory that are not among the
  17 now make a stop UNJUDGED where `v4r20` was right to be quiet: `.git/NOTES_MERGE_WORKTREE`
  (`git notes merge` checks a conflict out there and tells the user "Fix conflicts in
  .git/NOTES_MERGE_WORKTREE") and `common` (listed in `gitrepository-layout`, which the
  request names as the source of the list). The reason is cut before Git's words can be
  read ("…: fatal: this operation must b…"). And two sentences are wider than the code:
  "nothing remembered of any tree is forgotten at that stop" (README) / "nothing
  remembered of any tree is forgotten at it" (SECURITY): at such a stop a tree the
  judge's reader finds repaired IS forgotten (rightly); the claim meant "because it is not
  listed", which CHANGELOG says correctly.

What I would do before a merge: one sentence for HR13-L-1 in README beside the list (a
tree made under one of these names is taken for Git's own when both its records are gone),
or, cheaper in behaviour, the fix sketched under the finding; add `common` and
`NOTES_MERGE_WORKTREE` to the list (and `subtree-cache`, which `git subtree` writes), or
narrow the request's "as of its documentation"; and the CHANGELOG's wording ("because a
tree is no longer listed") in README and SECURITY.

## How this was done

- Source from `git archive HEAD` (`/var/home/louranicas/.cache/ddf-t/hooks-review-13/src`);
  `git diff 4b134bd..HEAD -- crates/` in `…/hooks-review-13/notes/r21.diff`. No `cargo`, no
  build, no `toolbox`, `podman` or `distrobox`, nothing at GitHub, no network. The only
  file written in the worktree is this report.
- Binary: `/var/home/louranicas/.cache/ddf-t/review-bin/v4r21/release/deep-diff-forge`,
  SHA-256 `9bc2666879f0c2ecb1504a60c99c597326f7f5765666f4c01d0dcbb96e80b204` (checked; debug
  `a26239da…` checked, not run). For "is it new": `…/v4r20/release/…` (`193674a9…`) with a
  base repository reset to the commit that admits it (`d13/base-r20`, `ea6c0be`). Judge:
  the real forge-warden (`…/target-fw/release/forge-warden`) on the host. Git 2.56.1.
- Base repository: `…/hooks-review-13/d13/base-repo`, a `--no-local` clone of
  `hooks-verify21/d11/base-repo` (`cc312d8`, admits v4r21). Case trees under
  `…/hooks-review-13/d13/cases/`; private HOME `…/hooks-review-13/d13/home`, set inside the
  driver (`lib11.py`).
- Scripts and outputs: `/var/home/louranicas/.cache/ddf-t/hooks-review-13/notes/s/` (the
  twelfth review's scripts copied with their paths changed, and Vesper's
  `missing_git_entry.py` repointed; new: `w13a.py`, `w13b.py`, `w13c.py`, `run-b.sh`,
  `bcmp.py`); outputs in `notes/s/out/` and `notes/s/out/m/`.
- At most two measuring processes at a time (the two lanes of `run-m.sh` and `run-b.sh`,
  each waiting above Tctl 80 °C or Composite 70 °C; neither had to wait). Tctl stayed
  between 64 and 77 °C, Composite under 46 °C. I stopped no process.

## Section 0, item 1: the closed findings' reproductions on this build

| finding | script (output in `notes/s/out/`) | result on v4r21 |
|---|---|---|
| VD4R20-M-1 | `missing_git_entry.py` (`missing_git_entry.txt`) | second stop `allow:UNJUDGED` with "fatal: this operation must be run in a work tree"; memory `…/u:1` kept; second key not put to the judge. **Closed** in the reported layout (but see HR13-L-1) |
| VD4R20-M-1, further | `w13b.py vesperx` | stops 2, 3 from `feature` UNJUDGED, memory kept; stop 4 from `main` silent and forgets `feature` (no longer a tree: as stated); stop 5 from `feature` UNJUDGED again, never silent |
| HR12-L-4 | `w12b.py locked` | UNJUDGED with "Permission denied"; the tree named "The session's directory is under the working tree … whether it is in that tree could not be found out", "A failure there … stands". **Closed** |
| HR12-L-4, by a link | `w13b.py linklock` (cwd = a symlink to L, then `locked/sub`) | the same: named as under L, stands. Held |
| HR12-L-3 | `cut.py` | "…says so at leng…)." (149 + "…"). **Closed** |
| HR11-M-1 | `el2.py`, `el.py barefail` | UNJUDGED both layouts; next stop with Git working BLOCKS. Held |
| HR11-L-2 | `au.py` | step 4 FAIL STANDS. Held |
| HR11-L-5 | `gone.py` | BLOCK, then FAIL STANDS. Held |
| HR12-L-1, L-2 | README and SECURITY diff | both stated, and true of `w12b never` (SILENT) and the twelfth review's `repairedslow` reading |
| VD4R20-L-1 = HR12-L-5 | `~/.cache/ddf-t/hooks-round21/mutant-table-r21.txt` | 69 rows KILLED (one row on two lines), 7 EQUIVALENT, 76 in all; the `NAME_MAX + 2 -> - 2` row only among the equivalents; the assertion in `hook.rs` labelled "a definition check, not a kill"; CHANGELOG 69/7. **Closed** |

## Section 0, item 2: which directories are Git's own

`w13a.py` (outputs `w13a-*.txt`), a bare repository with `main` inside it, the session
begun in `main`, a key in a linked tree made at the named path, then its `.git` file and
its administrative directory removed and a second key written:

| tree at | stop 1 | stop 2 (records removed) | stop 3 (`main`) | stop 4 (it again) | v4r20 |
|---|---|---|---|---|---|
| `repo.git/logs/feat` | BLOCK | **SILENT, memory gone** | SILENT | SILENT | same |
| `repo.git/svn` | BLOCK | **SILENT, memory gone** | SILENT | SILENT | same |
| `repo.git/hooks/feat` | BLOCK | **SILENT, memory gone** | SILENT | SILENT | — |
| `repo.git/worktrees/feat` | Git made the tree its own administrative directory (`HEAD`, `index` judged as files) | not meaningful | | | — |

That is HR13-L-1. Held: a `.git` entry present under one of the 17 names (the entry is
looked at with `symlink_metadata`; one that cannot be looked at is "not known absent");
a symlink inside the common directory that leads out (canonical `cwd` is not under it:
`Failed`, UNJUDGED); a symlink from outside into it (canonicalised: as the target);
spellings with `..` or a trailing slash (canonicalised; `common` is canonical from
`repository_dirs`); `GIT_DIR`, `GIT_WORK_TREE`, `GIT_COMMON_DIR` and five more in the hook's
environment (`el.py env`, the same decisions as on v4r20: `ENV_REMOVED` takes them out). The
project's own `.git`, `.git/hooks`, `.git/refs/heads`, `.git/objects`, `.git/logs`
(`w13a-lfs.txt`): the project judged, no "could not be found out". A submodule's
`.git/modules/sub`, `…/hooks`, `…/refs` (`w12a modules`): Git answers there, as before.

Not new and not a finding of this round: `GIT_CEILING_DIRECTORIES` and
`GIT_DISCOVERY_ACROSS_FILESYSTEM` are not in `ENV_REMOVED`. A ceiling inside a linked tree,
or a session directory on another file system under a linked tree (a mount, by reading also
a btrfs subvolume), makes Git say "not a git repository", which is `Elsewhere::Nothing`: the
linked tree is not judged from there and is named "the session's directory is not in it"
when `git status` shows a change. Suspected only (not run on this build); it is the
twelfth review's "nested repository inside a linked tree" row by another way.

## Section 0, item 3: a stop that cannot place its directory

`w13b.py forget` (`w13b-forget.txt`): the project R and linked trees L and M; failures
block in all three; then R and L repaired and M removed with `git worktree remove --force`,
and a stop from `R/.git/lfs/tmp`:

- stop 4 (cannot place its directory): UNJUDGED; R and L forgotten (the judge's own
  reading found nothing: right); M's memory KEPT (not listed, and the stop could not place
  its directory: as claimed). Not said anywhere (there is nothing to say of a removed tree).
- stop 5, the same keys back in R and L, from `.git/lfs/tmp`: BLOCK on R; L named "not
  judged".
- stop 6 from R: FAIL STANDS, M forgotten now (a stop that places its directory).
- stop 7 from L: BLOCK (L's key had been forgotten at the repair, so it blocks again: right).

Nothing kept forever: what a placeless stop keeps is forgotten at the next stop that can
place its directory. Nothing kept made an ordinary session block or stand wrongly. The
literal sentence of README and SECURITY is wider than this (HR13-L-2, third part).

## Section 0, item 4: ordinary workflows newly UNJUDGED

| stop made from | v4r21 | v4r20 | |
|---|---|---|---|
| `.git/lfs/tmp` (`w13a-lfs.txt`) | UNJUDGED, the project judged (FAIL STANDS) | quiet | as README states |
| `.git/rebase-merge` during `git rebase -i` stopped at an edit (`w13a-rebase.txt`) | the project judged (FAIL STANDS) | — | held |
| `.git/NOTES_MERGE_WORKTREE` during a `git notes merge` conflict (`w13c.txt`) | **UNJUDGED** | SILENT (`w13c-v4r20.txt`) | HR13-L-2 |
| `.git/common` (`w13c.txt`) | **UNJUDGED** | SILENT | HR13-L-2 |
| a plain directory in a bare repository (`w13b vesperx` step 5, the round's test) | UNJUDGED | — | as stated |

None blocks; each is UNJUDGED once per stop made from there. The words a person could act on
("this operation must be run in a work tree") are cut by the reason's 300 characters when the
path is long: "…: fatal: this operation must b…". A stop made from inside a rebase's state
directory, from `.git/hooks` and from the submodule's directory is unchanged.

## Section 0, item 5: the sentences (D, E)

True where said: the no-verdict line's "…" within 150 (README 752); HR12-L-1 and HR12-L-2
in README and SECURITY (measured figures as the twelfth review gave them); "a plain directory
made in a bare repository, and what other programs keep in a Git directory, such as `lfs`"
UNJUDGED; "also when the directory cannot be resolved, under a directory that cannot be
searched"; CHANGELOG's 69/7 and its list of 17 names (identical to `GITS_OWN_SUBTREES`).

Not true, or wider than the code:

- README: "each holds Git's own files only, while a working tree can be anywhere else in the
  Git directory": a tree can be made under them (HR13-L-1).
- README "nothing remembered of any tree is forgotten at that stop" and SECURITY "nothing
  remembered of any tree is forgotten at it": a repaired tree is forgotten there by the
  judge's reading (`w13b-forget.txt` stop 4). CHANGELOG's "forgets nothing because a tree is
  no longer listed" is the accurate form.
- The request's "The list of Git's own subtrees is Git's as of its current documentation
  (`gitrepository-layout`)": that page also lists `common`; and `NOTES_MERGE_WORKTREE` is
  documented in `git-notes(1)`.

## Section 0, item 6: the eleventh brief's sections 1 to 7, briefly

- Matrices `nr` (12 modes by 7 changes) and `mi` (sizes 1, 200, 256, 257, 600 by two halves by
  six changes): 144 of 144 files with the same decision at every step as the twelfth
  review's outputs on v4r20 (`mcmp.py`; `out/m/`).
- `a1`, `a3`, `par`, `cp`, `cn`, `cn2`, `mi7 600`, `ot 31 clean`, `ak 0 17`, `el bare|ingit|env|sub`,
  `w12a ingitfail|nestfail|baredmin|modules|gone|gonegit`, `w12b never|skipwt|failread|repaired|gitdir`,
  `wt2 256`: 25 files, the same decisions as on v4r20 (`bcmp.py`; `out/b-*.txt`; `wt2` differs
  in commit ids only; `cn2`, `cp`, `ot` compared by hand: the same text but the commit).
- Sections 2, 5, 6 (`agent setup`, `agent doctor --gate`, `schema signals`, `explain signal`):
  not changed by this commit (the diff touches `hook.rs`, `hook_run.rs`, `agent_hook.rs` and
  the docs only); not gone through again.
- 7, the tests: `a_directory_is_gits_own_by_where_it_lies_not_by_what_is_missing` fails
  without the structural check (the `feature`, `plain` and `objects/wt` rows);
  `a_tree_whose_two_records_were_removed_is_unjudged_and_keeps_what_blocked_there` fails
  without `unfound.is_none()` in `other_trees` (its memory assertion) and without the
  structural check (UNJUDGED); `a_directory_that_cannot_be_resolved_is_still_under_its_tree`
  fails on the old `.ok().flatten()` (it is skipped, not failed, when run as root: stated in
  the test); the no-verdict test checks the bound, the mark and characters (not bytes). None
  has a tree made under one of the 17 names (HR13-L-1) or a Git directory Git writes that is
  not in the list (HR13-L-2).

## Findings reproduced with the real judge

### HR13-L-1 (low; DELIBERATE EVASION in a rare layout; older): a tree under one of Git's own names, both records removed, is still "Git's own"

`among_gits_own_files` (`hook_run.rs` 2140): a directory under `common` whose first
component is one of `GITS_OWN_SUBTREES`, with no `.git` entry between, is Git's own; on
`Found::Failed` that is `Elsewhere::Nothing`, so `unfound` is `None` and `other_trees` sets
`listed`, and the tree, no longer listed, is forgotten. `git worktree add` places a linked
tree at any of those paths.

```
python3 -I /var/home/louranicas/.cache/ddf-t/hooks-review-13/notes/s/w13a.py top     # out/w13a-top.txt
   worktree list: … .git (bare) ; … .git/main cc312d8 [main] ; … .git/svn cc312d8 [feat]
-- 1 tree at repo.git/svn; key; cwd = it (BLOCK)                 => BLOCK   judge saw: ['lib/first.rs']  mem: 6718ed25a0739fab/u:1
   git in it now: rc=128  fatal: this operation must be run in a work tree
-- 2 its .git file and admin dir removed; a second key; cwd = it => SILENT  judge saw: []  mem: -
-- 3 cwd = main (project)                                        => SILENT  mem: -
-- 4 cwd = it again                                              => SILENT  mem: -
```

The same with `logs/feat` (`w13a-sub.txt`) and `hooks/feat` (`w13a-hooksx.txt`); v4r20 the
same (`w13a-*-v4r20.txt`). How likely: it needs a tree named like a directory Git makes in
the Git directory (`svn`, `logs`, `remotes`, `modules`, `branches`, …; `hooks`, `info`,
`objects`, `refs` exist already, so only a tree below them) AND both records removed by
hand; an agent does neither by mistake. How harmful: as VD4R20-M-1 (a silent stop with
credentials in the directory, and the failure that blocked there forgotten). Cheap: state
it beside the list in README; or, in behaviour, treat `Failed` + structural like `Failed`
for memory (do not set `listed` at that stop) and name a remembered tree that is no longer
listed, so a failure that blocked there is never forgotten in silence (the second key would
still not be judged, which the statement would then cover).

### HR13-L-2 (low; ORDINARY WORKFLOW, rare; new): Git's own directories missing from the list, and two sentences wider than the code

```
python3 -I /var/home/louranicas/.cache/ddf-t/hooks-review-13/notes/s/w13c.py          # out/w13c.txt
   notes merge rc=1: … Automatic notes merge failed. Fix conflicts in .git/NOTES_MERGE_WORKTREE and commit the result with 'git notes merge --commit', or abort
-- 1 cwd = project (clean)                                       => SILENT
-- 2 cwd = .git/NOTES_MERGE_WORKTREE (resolving notes there)     => allow:UNJUDGED
        |   not judged: whether the session's directory is another working tree of this repository could not be found out: Git could not say which repository "…/.git/NOTES_MERGE_WORKTREE" is in: `git rev-parse` failed: fatal: this operation must b…
-- 3 cwd = .git/common (gitrepository-layout)                    => allow:UNJUDGED
```

On v4r20 steps 2 and 3 are SILENT (`w13c-v4r20.txt`), which was right: the project was
judged and clean. Never a pass and never a block; the harm is an UNJUDGED line at each stop
made from there, whose actionable words are cut. Cheap: add `common`,
`NOTES_MERGE_WORKTREE` (and `subtree-cache`, which `git subtree` writes under the Git
directory) to `GITS_OWN_SUBTREES`; put Git's words before the path in the reason, or cut
the path. The sentence part: README "nothing remembered of any tree is forgotten at that
stop" and SECURITY "nothing remembered of any tree is forgotten at it" against
`w13b-forget.txt` stop 4 (R and L, repaired, forgotten there by the judge's reading); use
the CHANGELOG's "because a tree is no longer listed".

## Suspected only (not reproduced)

- `GIT_CEILING_DIRECTORIES` inside a linked tree, and a session directory on another file
  system under a linked tree, make Git say "not a git repository" (`Elsewhere::Nothing`):
  the tree is not judged from there and is named as one "the session's directory is not in"
  (older; section 0 item 2).
- `same_form` takes `..` out by name before following links; an absolute `cwd` with `..`
  after a symbolic link, under a directory that cannot be searched, could be placed under
  the wrong tree. Claude Code gives the directory from the process (no `..`), so not run.

## What I did not look at

`verdict_v2.rs`, `vocabulary.rs`, `agent setup`, `agent doctor --gate`, `schema signals`,
`explain signal` (unchanged by this commit); the time matrices (`tm.py`, `st4.py`,
`worst.py`), `wipe.py`, `km.py`, `lm.py`, `bare2.py`; more than 2,048 failing lines; a full
disk; `core.autocrlf` and `core.fileMode=false`; Vesper's round 21 report (it appeared in the
worktree, untracked, while I worked; I did not read it); the mutation run (no `cargo`).
