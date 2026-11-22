# Eleventh independent review of the agent hooks branch (`agent/loop`, commit `07df120`)

Date: 2026-11-15, 11:51 to 12:27 AEDT by the host's clock (`date`). Reviewer: an independent
Claude agent that took no part in the work.
Brief: `~/.local/state/habitat-claude/ddf-gap-probes-20261113/ddf-hooks-independent-review-11-brief.md`.
Under review: `d4dbc91..07df120`, mainly the rework `5c387a3..07df120` ("not read" is not
"nothing there"; staged stands while the index is untouched).

## Verdict: GO-WITH-FIXES

The four findings this commit answers are closed on the frozen binary with the real judge:
HR10-M-1 (a stop that could not read forgets nothing), HR10-M-2 (a large staged failure
stands whatever the tree does), HR10-M-3 (trees inside a bare repository are judged from
inside) and VD4R18-L-1 (a session that begins with more than 256 working trees judges its
tree). HR10-L-1, L-2 and L-3 are closed too, L-2 with one exception (HR11-L-1 below).

In what this commit reworked I found no new or changed failure that is taken for one that
stands (outside the stated identity of rule, signal, path and number), and no unchanged
failure that blocks twice without a stop having said so. I ran 84 cells of "the read fails
at one stop, something changes meanwhile", 60 cells of "the index changed by other means",
and the tenth review's 45-cell matrix; they held.

I found one medium and five lows. Two of them are stops that print nothing while something
is open. Both are older than this commit, both need an unusual precondition, and both sit
in a read that still has two answers where this commit says there are three.

- **HR11-M-1** (medium; ORDINARY WORKFLOW in one layout, after an unusual event; older,
  `v4r18` does the same). In a bare repository with its trees inside it, `elsewhere()`
  still takes EVERY failure of Git for a directory under the common directory to mean
  "Git's own files". A working tree there whose `.git` file names an administrative
  directory that is gone (pruned while its directory was away): a stop made from inside
  it, with credentials in it, prints nothing; advice prints nothing. In the ordinary layout
  the same state is UNJUDGED with the reason. With a Git that fails at one stop for another
  cause the stop is not silent, but it is not UNJUDGED either: it names the tree with the
  sentence HR10-M-3 was about ("the session's directory is not in it … a stop made from
  inside it judges them").
- **HR11-L-1** (low; ORDINARY WORKFLOW with a slow judge; new code of this commit).
  `asks_kept` reads every line of the append-only file of judged commits, also the
  no-verdict lines of a commit that has since passed. In a session where a commit once had
  no verdict, 8 or 13 seconds are held back for asks at every later stop although no commit
  waits: from a linked tree a run of 17 seconds on the stop's own change is cut at 13.6
  ("the rest is kept for what is judged after it") and the change is UNJUDGED. README says
  3 seconds are held back when no commit waits.
- **HR11-L-2** (low; ORDINARY WORKFLOW with `git update-index --assume-unchanged`; older).
  A stop made from another tree takes `git status` saying "clean" for "nothing uncommitted
  there" and forgets what has blocked in that tree. A failure in a file marked
  assume-unchanged is in the judge's patch and not in `git status`: after it has blocked in
  a linked tree, a stop from the project's tree is SILENT (the tree is not named, nothing
  "stands there"), and back in the linked tree the unchanged failure blocks a second time.
  No stop says anything of memory.
- **HR11-L-3** (low; ORDINARY WORKFLOW at scale; this commit's sentence). The reason of
  "The commits of this session: UNJUDGED" in a session that began with more than 256
  working trees is cut at 300 characters and closed with a full stop, so every stop of
  such a session reads "… so the session's own cannot be told from them: they are not."
  The words that carry the point ("judged in this session, whatever is committed; what is
  not committed is") are never shown.
- **HR11-L-4** (low; ORDINARY WORKFLOW, rare; this commit's sentence). The sentence for a
  digest that is given up ("… and the index no longer holds exactly that … may block once
  more when the working tree is repaired again") is also said at a stop where the index
  was not touched and only the tree was repaired: at the very stop that is the block it
  warns of, after which no such block can come.
- **HR11-L-5** (low; ORDINARY WORKFLOW; older). When the session's directory is a
  directory of a linked tree that was deleted (`cd build`, then the directory is removed),
  the tree the session is in is not judged, at any stop, and is named with "the session's
  directory is not in it".

Three wordings that are not accurate are listed as HR11-L-6.

What I would fix before a merge: HR11-M-1 (one condition in `elsewhere`, one test) and
HR11-L-1 (the last line of each commit, one unit test). HR11-L-3 is one constant or one
sentence. The others can follow.

## How this was done

- Source read from `git archive 07df120`
  (`/var/home/louranicas/.cache/ddf-t/hooks-review-11/src`). No `cargo`, no build, no
  `toolbox`, `podman` or `distrobox` call, nothing at GitHub, no network. The only file
  written in the worktree is this report.
- Binary: `/var/home/louranicas/.cache/ddf-t/review-bin/v4r19/release/deep-diff-forge`
  (SHA-256 `09bbaeaf…9b4e`, checked); for "is it new": `…/v4r18/release/…` (`6cd99e47…`).
  Judge: the real forge-warden 0.2.0 (`…/target-fw/release/forge-warden`, SHA-256
  `55bf9fe6…`), on the host. Git 2.56.0. The load average was 17 to 43 while I worked.
- Scripts, notes and outputs: `/tmp/claude-ddf-hooks-review-11/` (`NOTES.md`, `lib11.py`,
  `out/`, `r9/`). Repositories: `/var/home/louranicas/.cache/ddf-t/hooks-review-11/`
  (`base-repo` admits this build's ranker, commit `ac20f76`; `base-both` admits this build
  and `v4r18`, `d0dc949`; `base-long`, `a067e3f`; `home`, `home9`, `home-ig`: private
  `HOME`s set inside the drivers). The case directories are removed; a script must be run
  again to look at one (`wt3.py` takes the case directory of a `wt2.py` run as its
  argument, and `wt2.py` leaves it in place).
- Where a verdict had to be missing or slow the judge is a wrapper that runs the real
  judge for everything else: the tenth review's `tools/oj` and the eighth's `tools/tj`.
  A Git that fails is a shim in front of `/usr/bin/git`: `tools/failgit` (fails every call
  that has a given word) and `tools/failin` (the same, only for calls made from under one
  directory); a slow Git is the ninth review's `tools/hanggit` and `tools/untilgit`. Runs
  that use them say so.
- I killed no process. For about two seconds `par.py` made the shared state directory
  read-only while two of the time runs were going; neither output has "could not keep its
  state". I did not read Vesper's round 19 report (it appeared in the worktree, untracked,
  while I worked).

## Findings reproduced with the real judge

### HR11-M-1 (medium; ORDINARY WORKFLOW in one layout, after an unusual event; older): under the common directory, every failure of Git is "Git's own files"

`elsewhere` (`hook_run.rs` 2008 to 2040) asks Git, as this commit says. What it does with a
failure is still decided by the path:

```rust
// in no working tree, and inside the Git directory: Git's own files
Found::Failed(_) if cwd.starts_with(&place.common) => Elsewhere::Nothing,
```

`find` (281 to 313) answers `Found::Failed` for everything that is not Git's "not a git
repository (or any": a directory in no working tree, a Git that ran out of time, a
repository Git refuses for its owner, a `.git` file that names nothing. The comment, the
commit message and README 725 to 728 say "only a directory that is in no working tree and
inside the Git directory is one of Git's own"; the code does not ask whether the directory
is in no working tree, only whether Git failed. In the layout `repo.git/main`,
`repo.git/feature` every working tree is under the common directory, so there every
failure of Git for the tree a stop is in is read as "nothing here".

`el2.py` (`out/el2.txt`; no shim, the judge is `tools/oj`, which only logs):

```
-- 1 bare-inside; cwd = feature with a key (BLOCK)                         => BLOCK     judge saw: ['lib/linked.rs']
   git in feature/ now: fatal: gitfile does not point to a valid repository: …git/feature/.git
   git worktree list: ….git (bare) | ….git/main ac20f76 [main]
-- 2 feature/ was pruned (its files and the keys are still there); cwd = feature  => SILENT   judge saw: []
   post-tool from feature/: (nothing)
-- 1 CONTROL ordinary layout; cwd = linked tree with a key (BLOCK)         => BLOCK     judge saw: ['lib/linked.rs']
-- 2 CONTROL the linked tree was pruned; cwd = it                          => allow:UNJUDGED
        |   not judged: whether the session's directory is another working tree of this repository could not be found out: Git could not say which repository "…-L" is in: `git rev-parse` failed: fatal: gitfile does not point to a valid repository: …
```

Between 1 and 2 the tree's directory was moved away, `git worktree prune` was run, and the
directory was moved back; a second credential was then written into it. The same state
comes of deleting `repo.git/worktrees/feature` by hand, which is an honest mistake to make
in this layout (two directories are called `feature`). The directory is no working tree of
the repository any more, so nothing in it can be judged; what the rule asks is that this is
said. In the ordinary layout it is.

`el.py barefail` (`out/el-barefail.txt`; `tools/failin` makes `git rev-parse` fail in
`feature/` at one stop, nothing else changed):

```
-- bare-inside; rev-parse FAILS in feature/ at this stop; cwd = feature (key there) => allow   judge saw: []
        | deep-diff-forge: The working tree "….git/feature" of this project has changes that are not committed, and the session's directory is not in it: they were not judged (a stop made from inside it judges them; its commits are judged from anywhere).
-- the next stop, Git works (must block)                                   => BLOCK     judge saw: ['lib/linked.rs']
-- CONTROL ordinary layout; rev-parse FAILS in the linked tree; cwd = it   => allow:UNJUDGED
        |   not judged: whether the session's directory is another working tree of this repository could not be found out: …
```

So in this layout a stop whose own tree could not be found is not UNJUDGED, its first line
is not UNJUDGED, and it says of the tree the session is in that the session is not in it:
the false sentence of HR10-M-3, now only when Git fails.

On `v4r18` step 2 of `el2.py` is SILENT too (`out/el2-v4r18.txt`; there step 1 is HR10-M-3
itself). So this is not new with this commit; it is what is left of HR10-M-3 in the
function this commit rewrote for this layout.

Rating. A stop that prints nothing while credentials lie in the directory the session is
in is the one thing the rule forbids. I rate it medium and not high because the tree must
first lose its administrative directory, which no ordinary Git command does while the
tree is there, and because it takes this one layout. If the owner counts a pruned tree's
directory as outside the project, it is low; the second reproduction (not UNJUDGED, a
false sentence) stands either way.

Cheap fix: under the common directory, `Elsewhere::Nothing` only when Git's own words are
"this operation must be run in a work tree" (read under `LC_ALL=C`, as `NO_REPOSITORY`
is); every other failure is `Elsewhere::Failed`, as it is outside the common directory. A
test: bare-inside, a tree whose `.git` file names a directory that is gone, a stop from
inside it: UNJUDGED with the reason. The new test
`working_trees_inside_a_bare_repository_are_judged_from_inside` asserts
`!said.contains("could not be found out")` for three of Git's own directories, which holds
for a failing Git as well.

```bash
W=/tmp/claude-ddf-hooks-review-11; python3 -I $W/el2.py; python3 -I $W/el.py barefail
```

### HR11-L-1 (low; ORDINARY WORKFLOW with a slow judge; new code of this commit): time is held back for commits that no longer wait

`asks_kept` (`hook_run.rs` 2393 to 2406) says whether a commit waits for its turn and
whether one with fewer attempts waits by `known.iter().any(…)` over every line of the
session's file of judged commits. That file is append-only: `sorted_commits` (3027) takes
the LAST line of each commit, `asks_kept` takes any line. A commit that had no verdict at
one stop and passed at the next leaves its `n1@…` line there for the rest of the session
(5 seconds more held back); one that had six such stops leaves lines on both sides of five
attempts (10 seconds more).

`ak.py STALE SLEEP` (`out/ak-*.txt`; the stop is made from a linked tree that has an
uncommitted and a staged change; the project's tree has a change; `tools/oj` sleeps before
it runs the real judge on the linked tree's uncommitted change; before that stop two stops
were SILENT, so every commit of the session has a pass):

| before the stop | the judge takes | result |
|---|---|---|
| the session's one commit passed at once | 17 s | BLOCK after 17.8 s (by the rule the run may take 24 s) |
| the same | 20 s | BLOCK after 20.6 s |
| the commit had no verdict at 1 stop, then passed | 17 s | BLOCK after 17.8 s |
| the same | 20 s | the run is stopped at 18.7 s ("this run had 18.7 of the 34.7 seconds …: the rest is kept for what is judged after it"): the uncommitted change is UNJUDGED |
| the commit had no verdict at 6 stops, then passed | 17 s | the run is stopped at 13.6 s: "Uncommitted changes … UNJUDGED … this run had 13.6 of the 34.6 seconds that were left of the stop's judging: the rest is kept for what is judged after it"; the stop ends at 14.4 s |

In the last row nothing is judged after it that needs the 21 seconds: the staged change
took 0.1 s, the project's tree 0.1 s, and no commit was asked about. README 735 says what
is held back is "3 seconds when no commit of the session waits for a verdict; 5 more when
one waits for its turn, 5 more when one with fewer attempts waits". That is HR10-L-2's
second row for every session in which a commit ever had no verdict (one time-out of the
judge is enough for the first 5 seconds). The unit test (7716 to 7729) has no file in
which a later line of the same commit follows a no-verdict line.

Cheap fix: take the last line of each commit, as `sorted_commits` does. A test: the lines
`n7@4`, then `p` for the same id, give three seconds.

```bash
W=/tmp/claude-ddf-hooks-review-11; python3 -I $W/ak.py 0 17; python3 -I $W/ak.py 6 17; python3 -I $W/ak.py 0 20; python3 -I $W/ak.py 1 20
```

### HR11-L-2 (low; ORDINARY WORKFLOW with assume-unchanged; older): `git status` "clean" is taken for "nothing there", by a reader that is not the one that judges

`other_trees` (`hook_run.rs` 3575 to 3576) looks at a tree the stop is not in with
`git status --porcelain`, and `Ok(false) => out.clean.push(id)`; `remember_within` (4367)
then forgets everything remembered of that tree, and no note names it. The judge's patch is
made another way (a scratch index from `HEAD`, `git add --all`), and sees what `git status`
does not: a change in a file marked `git update-index --assume-unchanged` (what people use
for a local configuration file they do not mean to commit).

`au.py` (`out/au.txt`; nothing simulated):

```
   git status --porcelain in the linked tree: ''
-- 1 a key in a file marked assume-unchanged; stop from the linked tree   => BLOCK              mem: 7b83…/u:1
-- 2 again (stands)                                                       => allow:FAIL STANDS  mem: 7b83…/u:1
-- 3 a stop from the project's tree: `git status` there says clean        => SILENT             mem: -
-- 4 from the linked tree again; NOTHING changed                          => BLOCK              mem: 7b83…/u:1
-- 5 again                                                                => allow:FAIL STANDS
```

Step 3 breaks README 515 to 517 ("A stop that did not look (the session's directory was
not in that linked working tree) forgets nothing, names the tree and says at every such
stop that a failure stands there") and the round 14 rule: a failure that has blocked and
is not repaired stands in the project, and the stop prints nothing. Step 4 breaks claim 3
(an unchanged failure blocks twice; no stop said its memory was lost). `v4r18` does the
same (`out/au-v4r18.txt`). It is this commit's own rule seen in the one place that does
not read with the hook's own reader: "only 'nothing there' forgets", and here "nothing
there" is another program's answer to another question.

It is low because the feature is rare and the second block is conservative. The silent
stop is the part I would not leave.

Cheap fix: a tree of which something is remembered is not forgotten on `git status` alone:
either it is named ("a failure stands there") until a stop made from inside it has looked,
or it is read with `uncommitted()` before it is forgotten (only trees where something has
blocked, so it is rare). A test: the five steps above; step 3 must name the tree and step
4 must not block.

```bash
python3 -I /tmp/claude-ddf-hooks-review-11/au.py
```

### HR11-L-3 (low; ORDINARY WORKFLOW at scale; this commit's sentence): the reason of a session with no list of commits is cut to "they are not."

`judge_units` (`hook_run.rs` 3905 to 3908) gives the reason "the commits that were there
when this session began could not be listed (DETAIL), so the session's own cannot be told
from them: they are not judged in this session, whatever is committed; what is not
committed is". DETAIL is 165 characters for more than 256 working trees, the whole 372.
`unit_text`
(`hook.rs` 1248 to 1250) shows the first 300 characters of a reason and adds a full stop.
The cut falls after "they are not":

```
The commits of this session: UNJUDGED
  not judged: the commits that were there when this session began could not be listed (the repository has more working trees than a hook reads the history of (256 at most; `git worktree prune` takes away the entries of trees whose directories are gone)), so the session's own cannot be told from them: they are not.
```

That is every stop of such a session (`wt2.py`, `out/wt2-256.txt`, steps A1 to A9;
`wt3.py`). What the reader is not told is the new fact of this round: that the commits are
not judged in this session whatever is committed, and that what is not committed is. The
same cut ends other reasons in the middle of a word ("git rev-parse did not name the
repo.", `out/cn2.txt`); there is no mark that says a reason was cut.

Cheap: put the consequence before the detail, or give this reason its room; and end a cut
reason with "…" and not with a full stop.

```bash
python3 -I /tmp/claude-ddf-hooks-review-11/wt2.py 256     # 257 working trees on disk, about a minute
```

### HR11-L-4 (low; ORDINARY WORKFLOW, rare; this commit's sentence): the sentence for a digest that is given up, at a stop where the index was not touched

`staged_memory` (`hook_run.rs` 4271 to 4272) sets `gone` whenever a mark is remembered that
is neither what the index holds nor the staged change's own lines at this stop.
`memory_notes` (4175 to 4181) words that as "… and the index no longer holds exactly that:
which of the lines that are still staged have blocked before is not known any more. What is
staged there may block once more when the working tree is repaired again, although those
lines have not changed."

`mi7.py 600` (`out/mi7-600.txt`): 600 failing files staged and repaired in the tree; the
keys are back in 300 files of the tree; one more failing file is staged (step 5: BLOCK,
and both sentences, which are true there); then the tree is repaired and the index is not
touched:

```
-- 7 the tree repaired; INDEX UNTOUCHED since 5 (True): the block step 5 warned of => BLOCK   mem: m/s:1(mark)
        | Staged changes in "…": FAIL
        | A failure here has blocked before and is not what it was. …
        | Of what is staged in 1 working tree, the hook remembered what had blocked as a whole, … and the index no longer holds exactly that: … What is staged there may block once more when the working tree is repaired again, although those lines have not changed.
-- 9 keys back in 100 files of the tree (documented block as the tree's)  => BLOCK
-- 10 the tree repaired again; INDEX UNTOUCHED since 5 (True)             => allow:HOLD  (Staged changes: FAIL, which stands)
```

At step 7 the index holds what it held at step 5; the block the sentence warns of is this
stop's; and from this stop on the mark is exactly what the index holds, so no such block
can come (step 10 stands). The sentence is HR10-L-1's kind: said where it is not true of
the stop. The block itself was announced at step 5, as the rule asks.

Cheap: do not say it at a stop whose mark for the staged change becomes what the index
holds (the memory is then at its best), or word it by what happened ("the staged change's
own failure is not what it was").

```bash
python3 -I /tmp/claude-ddf-hooks-review-11/mi7.py 600
```

### HR11-L-5 (low; ORDINARY WORKFLOW; older): a session's directory that was deleted inside a linked tree

`elsewhere` answers `Nothing` for a `cwd` that does not exist (2011 to 2019). When that
directory was inside a linked tree, the tree the session is in is not judged:

`gone.py` (`out/gone.txt`; `v4r18` the same, `out/gone-v4r18.txt`):

```
-- cwd = linked/build/out, which was deleted; a key is in the linked tree => allow  judge saw: []
        | deep-diff-forge: The working tree "…-L" of this project has changes that are not committed, and the session's directory is not in it: they were not judged (a stop made from inside it judges them; its commits are judged from anywhere).
-- again                                                              => allow  judge saw: []
-- cwd = the linked tree's top (control: must block)                  => BLOCK  judge saw: ['lib/linked.rs']
```

It is said at every stop, so it is low; but the sentence is not true of that session, and
a credential in the tree is not put to the judge for as long as the session's directory
stays the deleted one (an agent that ran `cargo clean` or removed a build directory it was
in). README 338 to 340 states that a stop from a directory that is gone still judges the
project; it does not say that the tree the directory was in is left out.

Cheap: ask Git about the nearest ancestor of `cwd` that exists.

```bash
python3 -I /tmp/claude-ddf-hooks-review-11/gone.py
```

### HR11-L-6 (low): three wordings

- README 400 to 401: "A run that was cut is written down as asked and cut, with the time
  it had." A run that had all that was left of the stop (three seconds or more) and was
  cut by the stop's end is written with no time: `n0@6:the judge was stopped before it
  gave a verdict` (`out/st4-TA.txt`; that run had 10.8 s). The time is written only when
  the run had less than what was left, or less than three seconds (`hook_run.rs` 1913 to
  1927).
- README 735: "From a linked tree with nothing else to judge: 32 seconds." No run has more
  than 30 (README 741). `ot.py 31 clean` (`out/ot-31-clean.txt`): the run is stopped at
  30.0 s and the reason says "this run had 30.0 of the 34.9 seconds that were left of the
  stop's judging: the rest is kept for what is judged after it"; 3 of those 4.9 seconds
  are kept, 1.9 are the 30-second bound.
- README 632 to 633, of the gate's `commits` link: "(where they cannot, no session is
  recorded and every stop is UNJUDGED)". With more than 256 working trees a session is
  recorded now and what is not committed is judged; the gate's own detail says so
  correctly ("the commits of a session here would be UNJUDGED at every stop (what is not
  committed is judged)").

## What this commit answers: closed

All on `v4r19` with the real judge; the tenth review's scripts with their paths changed.

- **HR10-M-1 closed.** `wipe.py 1`, `1 sparse`, `1 slowgit` (`out/wipe-1*.txt`): the stops
  that cannot read say UNJUDGED and "what has blocked of it before stands", the memory is
  kept (`m/s:1` throughout), and the stop after is FAIL STANDS.
- **HR10-M-2 closed.** `km.py c1 300 100 50` and `c1 257 100 50` (`out/km-c1-*.txt`): step
  5 is FAIL STANDS; `c1 200` as before. `km.py leave 300` and `200`: lines that left the
  index and came back with a stop in between still BLOCK.
- **HR10-M-3 closed.** `bare2.py` (`out/bare2.txt`): from `feature/` the judge sees
  `lib/linked.rs`, the stop BLOCKS, then stands; staged, then committed: BLOCK for the
  commit. What is left of it is HR11-M-1.
- **HR10-L-1 closed** in its wording (`km.py c2 300`, step 5: "Of what is staged in 1
  working tree, the hook remembered what had blocked as a whole … (a change of more than
  256 failing lines, or a session of more than 2048) …"). HR11-L-4 is another stop at
  which it is said.
- **HR10-L-2 closed** for a session with no no-verdict line in its file: `ot.py 17 clean`
  BLOCK after 17.3 s; `ot.py 12 clean staged` BLOCK after 24.3 s; `ot.py 40 dirty`: the
  first run is stopped at 28.9 s and `lib/main.rs` is then judged and blocks (29.1 s).
  Not closed for the others: HR11-L-1.
- **HR10-L-3 closed.** `st4.py TA` (`out/st4-TA.txt`; the judge hangs on the commit in its
  turn and on one with fewer attempts): two commits never asked about are submitted at
  +36.0 s and BLOCK. `st4.py TAC` (it hangs on the first never-asked commit too): the runs
  get 30.0, 5.8 and 5 seconds, the second never-asked commit is asked first of its kind at
  the next stop and BLOCKS.
- **VD4R18-L-1 closed.** Below, "more than 256 working trees".

## The areas of the brief: what was tried and whether it held

0. **This round's rework.**
   - THREE-VALUED READS. By reading: `Reading<T>` is carried from `uncommitted` through
     `judged_patch`, `tree_units` and `seen`; `Looked.tree` and `Looked.index` are `None`
     exactly when nothing is known, and `remember_within` touches a tag only for `Some`.
     The places where one answer still stands for two: `elsewhere`'s `Found::Failed` under
     the common directory (HR11-M-1); `other_trees`' `Ok(false)` from `git status`
     (HR11-L-2); `has_uncommitted_changes` before the time is divided (time only; what it
     cannot say counts as a change); `elsewhere` for a `cwd` that is gone (HR11-L-5).
     By running, `nr.py MODE CHANGE` (`out/nr-*.txt`, 12 modes by 7 changes): a staged
     failure and a tree failure have blocked and stand; the read fails at two stops; a
     change is made meanwhile; the read works again. The modes that really stop the read:
     an untracked file of mode 000, a path marked skip-worktree, a Git that fails in
     `ls-files --others`, in `ls-files --stage` (the index alone is not read: "what is
     staged could not be read from Git … what has blocked of it before stands"), in
     `write-tree`; the scratch directory under the Git directory made read-only; the tree
     moved away and back; `HEAD` taken away and put back. In all of them the memory is
     unchanged through the unread stops, and each change made meanwhile (a new failing
     file staged; a new failing file in the tree; a second credential in the staged file;
     a second one in the tree's file; the tree's failing file staged and repaired) BLOCKS
     at the first stop that reads; no change, and a file that left the index and came back
     as it was, stand. An `index.lock` and a Git that fails in `status` or `worktree list`
     do not stop the read: there the change blocks at once (my script marks those cells
     "DIFFERENT"; that is its expectation, not a defect). With two trees (`nrl.py`, the
     unread tree the linked one or the project's, 6 cells): the same. Held.
   - A MARK EQUAL TO THE INDEX. `mi.py N HALF CHANGE` (`out/mi-*.txt`; N = 1, 200, 256,
     257, 600; with the staged change all its own, and with half of it in the tree again).
     The same bytes under another path, the same bytes under one more path, a second
     credential in one staged file (each made with `git update-index --cacheinfo`): BLOCK
     at every size. The index emptied and put back with `git read-tree`, no stop between:
     stands. Other credentials at the same paths: stands (the stated identity). `git
     reset` and `git add` of half: stands up to 256; with 257 and 600 and nothing in the
     tree it BLOCKS as a whole with "not what it was" (the stated rule for a digest: "any
     change in it, the repair of one part too"). Where the tree was then repaired with the
     index untouched and the stop blocked (eight cells, all 257 or 600 with half in the
     tree), a stop had said so before, at the stop of the change. I found no index changed
     to something that fails anew that stands. By reading: a mark reaches `s` or `k` only
     from lines that were the staged change's own at a stop that judged them
     (`staged_memory`, `Unit::short`), so `all_told` cannot hold for a line that has not
     blocked. The tenth review's `lm` matrix: 42 of 45 files have the same decisions as
     its outputs on `v4r18` (`lmcmp.py`); the other three are that review's outputs of a
     script that had crashed, and on this build the stash round trip blocks three times at
     1, 256, 257 and 600, as stated.
   - MORE THAN 256 WORKING TREES (`wt2.py 256`, `wt3.py`; 257 trees made on disk). The
     start says the chain is not ready for `commits` and why; the state has `no_baseline`
     and no tips file. Nothing changed: UNJUDGED for the commits, never silent. A
     credential in the tree: BLOCK, then stands. Committed: "The commits of this session:
     UNJUDGED" at every stop; nothing is ever written to the file of judged commits. From
     a linked tree: its own change BLOCKS. Trees removed until 198 are left: still
     UNJUDGED, no list is taken, two more commits (one with a credential) are not judged
     and not called "there". `agent doctor --gate`: exit 7 with the `commits` link not
     ready at 257; ready at 198. A second session begun at 198: its own commit with a
     credential BLOCKS; the first session's commits are "there" for it (each session
     judges its own). A session whose first hook is a stop or an advice hook at 258: the
     same, with the note that its start was not recorded. A v9 state read by `v4r18` and
     a v8 state read by this build: UNJUDGED ("not a state of this version of the hooks"),
     nothing recorded over it, at the stop, in advice and at a start. Held; the wording is
     HR11-L-3. I found no commit made by such a session that reads as judged or as one
     that was there.
   - WHICH TREE (`el.py`, `od.py`; the judge's calls logged). Bare with trees inside: a
     third tree nested in `main/.claude/worktrees/x` is judged first, then `main`; from
     `repo.git`, `repo.git/hooks`, `repo.git/worktrees/feature` and a plain directory in
     the bare repository: the project is judged and the other trees are named; the
     project `feature` and the stop from `main`: `main` first. A session begun in
     `repo.git` itself: UNJUDGED ("this operation must be run in a work tree"). A linked
     tree inside the project's `.git` (`.git/wt-in`), from its top, from deep inside and
     through a symbolic link: judged first. `.git/hooks`, a tree's administrative
     directory, a `cwd` that does not exist in the project: the project. `GIT_DIR`,
     `GIT_WORK_TREE`, `GIT_INDEX_FILE`, `GIT_COMMON_DIR`, `GIT_OBJECT_DIRECTORY`,
     `GIT_CEILING_DIRECTORIES`, `GIT_NAMESPACE` set to another repository in the hook's
     environment: no effect on what is judged. A submodule of a tree inside a bare
     repository: named as another repository, the tree it is in named as not judged (as
     stated for submodules). `od.py linked`, `projlinked`, `nested`, `bare`, `submodule`:
     the order of the tenth review. Broken: HR11-M-1, HR11-L-5. One more thing, older and
     no ordinary workflow: a working tree whose path has a newline is UNJUDGED from inside
     (`git rev-parse` is read line by line) and is told "(a stop made from inside it
     judges them)" (`out/cn2.txt`).
   - TIME. `ot.py`, `st4.py` as above. `tm.py` (the turn at its edges, `out/tm-*.txt`):
     7.9 s left: both asks and the probe are made and written; 3.1 s: the first is made
     and written, the second is not made, nothing is written for it, "not asked at this
     stop"; 2.9 s: nothing made, nothing written; 3.1, 8.2 and 13.2 s with a judge that
     hangs: the runs get what `run_time` says (13.2: 8.0 s and then 4.9 s) and each is
     written. `worst.py` (`out/worst.txt`; the judge hangs on every patch, the stop is in
     a linked tree, both trees have an uncommitted and a staged change, three commits
     wait): 13.7 s, 5.0 s, then the project's uncommitted change takes all that is left
     (the stated "under 3 seconds: all that is left"), its staged change and the commits
     get nothing, nothing is written for the three commits, and the stop ends at 35.02 s
     with its scratch directory empty. No stop I ran took longer than 41.04 s. Every ask
     that was written had been made, and nothing was written for one that was not. Two
     things to say of the claims: "no run of the judge takes the time of an ask owed after
     it" (the commit message and claim 2 of the request) is true of asks about commits and
     not of a run on a working tree's change, which README's table states; and in
     `worst.py` the commits are said to be unlisted because "`git for-each-ref` did not
     finish in the time a hook has", when it had no time at all. "3 seconds when no commit
     waits": HR11-L-1 is the session where none waits and more is held back. The other
     direction (a commit waits and the file does not say so) I did not find: a commit
     never asked about has its one ask counted, and everything else that waits has a line.
     A missing or changed file of judged commits makes the whole stop UNJUDGED, the tree
     too (older; stated in README 665 to 667).
   - THE MEMORY SENTENCES. "what has blocked of it before stands": at every stop that
     could not read while something of the staged change was remembered, and not
     otherwise. "Of what is staged … the index no longer holds exactly that": at the stops
     where a digest was given up because the index changed (`km.py c2 300` step 5, `mi.py`
     step 5); also at HR11-L-4's stop. "… is remembered as a whole from now on": at the
     stop where the staged change's own failure became a digest while the index holds
     more (`mi.py 600 1 dup` step 5). I found no block of an unchanged staged failure that
     no sentence had announced. The sentences for more than 2,048 lines: not reached.
   - WHAT HELD BEFORE, on this build (`r9/`, `r9cmp.py`). The eighth and ninth reviews'
     cases: tree identity (`t1.sh`: move, `mv` and repair, `mv` without, remove and add,
     lock, gc, touched files, restores), relative paths (`t2.sh`), the project a linked
     tree, bare with trees beside it, many trees (`t3.sh`), line identity (`t4.sh`,
     `t7.sh`), `t9.sh`, `t10.sh`, the round 14 table (`r6/t6.sh`: SILENT in exactly the
     all-pass rows), nine trees of 256 failing files (`m1.py`), the turn from the wrapper's
     log (`t5.py mix`, 30 stops, no violation), eight first hooks at once (`t6.sh par`):
     the decision and first words of every stop are the same as in the tenth review's
     outputs on `v4r18` (12 of 12 files). The crowded stop (`t15b.sh`): 7,052 characters,
     its counting lines whole. The counting line with long names (`cn.py`: seven linked
     trees whose directory names have 150 to 177 characters, one with terminal escapes,
     one with a newline): 622 and 639
     characters, three names each with "(a failure stands there)", "and 4 more (among
     them 3 working trees where a failure stands that has blocked)", no control character
     in the message.
   - STATED LIMITS. A key replaced by another in the same file: README 502 to 513, and a
     line of its own in the messages; accurate (`mi.py … othercontent` stands at every
     size). A branch moved back onto an old failing commit: README 364 to 367 ("resets to
     it: it was there before, and it is not judged at a stop") and the range words of
     every message; accurate; read only. The stash round trip: README 595 to 598; three
     blocks, as run. No birth time with reused inode numbers: README 536 to 545; accurate
     to the code's comment; not testable here. The list is never taken later: README 720
     to 723 and SECURITY 482 to 486; accurate (`wt2.py`, 198 trees left). Not accurate:
     the three places of HR11-L-6.
   - `CLAUDE_PROJECT_DIR` wrong (`cp.py`): as in the tenth review, never silent. Which
     commits are the session's, the staged unit against index states beyond the above,
     the policy from the base: not gone through again.
1. **A pass that should not be one** (`a1.py`). A judge that exits 0 with no document;
   one that replays a real pass for another patch; a document cut short; a key given
   twice; a pass document with exit 0 over a real fail; the real judge copied into the
   repository and named by `--judge`; no `--judge` and a stand-in `forge-warden` in the
   repository's `bin` on `PATH`: UNJUDGED every time. Nothing else tried.
2. **A block that cannot be escaped, or one that is waved through.** HR11-L-2 is a block
   that comes a second time; it ends at the next stop. With a state directory that cannot
   be written a failure blocks at every stop and `stop_hook_active` ends it with FAIL
   STANDS (`par.py`). Nothing waved through.
3. **The hook input and output contract** (`a3.py`). Empty input, text that is no JSON, an
   array, a missing `session_id` or `cwd`, a wrong event name, a `cwd` that is no string,
   invalid UTF-8, 33 MiB, an unknown option: exit 0, valid JSON, "UNJUDGED. The stop hook
   did not run: …". A session id with slashes and a NUL, a key given twice: judged. A path
   with a newline and an instruction in it: escaped (`\x0a`) in the block and in advice.
   Every output of every run was parsed as JSON by the driver.
4. **Resources and files** (`par.py`). A second stop of a session while the first runs:
   UNJUDGED after 3.0 s ("another hook of this session is running"); the first one's
   block is remembered. State files 0600 in a directory 0700. The scratch directory under
   the Git directory was empty after every stop I looked at, also after runs that were
   cut. A state directory that cannot be written: said with the decision. A full disk: not
   tried (a read-only scratch directory makes the stop UNJUDGED and forgets nothing).
5. **`agent setup --hooks`, `agent doctor --gate`.** The settings `agent setup --hooks
   --json` prints, written to the project's `.claude/settings.json`: the gate exits 0 with
   every link ready; without them 7. `--judge` is no option of `agent setup` (the judge is
   found on `PATH` there). The gate in a repository with 257 trees: above. Not tested
   further.
6. **`schema signals`, `explain signal`.** 21 labels; `explain signal` answers for each and
   refuses an unknown one. Not checked against the signal tables.
7. **The tests.** By reading; I ran no test (no `cargo`).
   - `a_stop_that_could_not_read_the_tree_forgets_nothing_of_it` holds the forgetting. It
     has no change made during the unread stop, so it does not hold that such a change
     blocks afterwards (it does: `nr.py`).
   - `working_trees_inside_a_bare_repository_are_judged_from_inside` holds HR10-M-3. For
     Git's own directories it asserts only that "could not be found out" is not said,
     which a failing Git satisfies too (HR11-M-1).
   - The unit test of `asks_kept` (7716 to 7729) has no later line of the same commit
     (HR11-L-1).
   - `a_repository_with_more_working_trees_than_are_read_says_so` makes its trees with
     `--no-checkout`, makes no commit in the session and removes no tree: that a commit of
     such a session is UNJUDGED at every stop, and that the list is not taken when the
     trees become fewer, is held by no test (it holds: `wt2.py`).
   - Nothing holds what `git status` "clean" does to a tree where a failure has blocked
     (HR11-L-2), or a `cwd` that is gone inside a linked tree (HR11-L-5).
   - `giving_up_what_was_remembered_of_a_staged_change_says_what_happened` (by its name
     and the request) holds that the sentence is said; nothing holds that it is not said
     at a stop where the index was not touched (HR11-L-4).

## Suspected only (not reproduced)

- HR11-M-1 by other causes of a failing Git in that layout (a tree another user owns,
  which Git refuses; a Git that runs out of time): by reading; I ran the pruned tree and a
  shim.
- HR11-L-2 by other things `git status` does not show and the judge's patch does (a file
  changed only by a clean filter or by `core.autocrlf`, a mode with `core.fileMode=false`):
  by reading; I ran assume-unchanged.
- HR11-L-1 for the project's main tree alone (28 of 41 seconds where 30 are the bound): by
  reading; I ran it from a linked tree.

## What I did not look at

`verdict_v2.rs`; `judge_patch.rs` beyond `named_commits`, `worktrees`,
`has_uncommitted_changes` and how the ignore file is carried over (the user's default
ignore file is honoured: `ig.py`); attributes, filters, CRLF, symlinks, binary files; more
than 2,048 failing lines live; a file system with no birth time or no `flock`; a full
disk; the eighth review's `t6.sh kill` and `t6k.sh` (they pick a process by name), its
`t11.sh` and `t5.py slow`; the ninth review's `w1.py`, `e1.py`, `e2.py`, `u1.py`, `st1.py`,
`b1.py`; the tenth review's `xb.py` and `st3.py` (their ground is covered by `wt2.py` and
`st4.py`); a branch moved back onto an old failing commit; areas 1, 5 and 6 beyond what is
said above; what Claude Code itself does with `stop_hook_active` or with two hooks at
once; the mutation run; Vesper's round 19 report.
