# Tenth independent review of the agent hooks branch (`agent/loop`, commit `5c387a3`)

Date: 2026-11-15, 09:44 to 10:10 AEDT by the host's clock (`date`). Reviewer: an independent
Claude agent that took no part in the work.
Brief: `~/.local/state/habitat-claude/ddf-gap-probes-20261113/ddf-hooks-independent-review-10-brief.md`.
Under review: `d4dbc91..5c387a3`, mainly the rework `f604d58..5c387a3` ("the tree a stop is in
comes first; a mark is how a failure is remembered, not what it is compared by").

## Verdict: GO-WITH-FIXES

I found no stop that prints nothing while something is open, and no new or changed failure
that is taken for one that stands, outside the stated identity (rule, signal, path,
number). The two findings of the round before are closed: HR9-M-1 (staging part of a
failure of 257 or 600 lines stands at every step) and VD4R17-L-1 (the linked tree is
submitted first). HR9-L-2 and HR9-L-3 are closed too.

I found three mediums and three lows. All are ORDINARY WORKFLOW; none is a deliberate
evasion. Two of the mediums are again in what a stop REMEMBERS (an unchanged staged failure
blocks a second time and no stop said that its memory was lost), one of them a regression
of this commit. The third is a repository layout in which the tree a stop is in is not
judged at all.

- **HR10-M-1** (medium; ORDINARY WORKFLOW; older than this commit, `v4r17` does the same).
  A stop that could not READ the working tree (an untracked file that cannot be read, a
  path marked skip-worktree, a Git that is slow at that one stop) forgets everything the
  session remembers of that tree's staged change. The staged failure, untouched in the
  index and already told, blocks a second time at the next stop that can read the tree. No
  stop says anything about memory. One failing line is enough; no mark is involved.
- **HR10-M-2** (medium; ORDINARY WORKFLOW, rare; **a regression of `5c387a3`**: `v4r17`
  stands). A staged failure of more than 256 lines that has blocked as the staged change's
  own, then is in the working tree again in part, then is repaired there again in part:
  the stop blocks for "Staged changes", with the index untouched since the first block and
  only repairs made in the tree. No stop said that memory could not be kept. With 200 lines
  the same sequence stands.
- **HR10-M-3** (medium; ORDINARY WORKFLOW in one repository layout; older than this
  commit). In a bare repository whose working trees lie inside it (`repo.git/main`,
  `repo.git/feature`), a stop made from a linked tree that is not the session's project
  does not judge that tree, first or at all. It names it with a sentence that is false
  there ("the session's directory is not in it … a stop made from inside it judges them").
  A credential in the tree the stop is in is never submitted while it is uncommitted or
  staged. Its commit is judged.
- **HR10-L-1** (low; ORDINARY WORKFLOW; this commit's third memory sentence). The sentence
  "of what is staged in 1 working tree it could not keep …" is reached by a run of the
  hooks with ONE working tree and 300 failing lines (`git reset -- FILE` of one staged
  file). README says it needs more than 2,048 failing lines and is held by unit tests
  only; the sentence itself begins "This session has more failing lines than the hook
  keeps one by one (2048)", which is not true there.
- **HR10-L-2** (low; ORDINARY WORKFLOW with a slow judge; a regression of `5c387a3`). When
  the stop is in a linked tree, the run of the judge on that tree's change gets 15 of the
  35 seconds (10 and then 5 when the tree also has a staged change), because time is held
  back for the project's tree before it has been read. A run of 17 seconds is stopped at
  15, the stop's own tree is UNJUDGED, and the stop ends after 15.1 seconds with 20 unused.
  `v4r17` judged it (BLOCK after 17.3 seconds).
- **HR10-L-3** (low; ORDINARY WORKFLOW only with a judge that hangs on two commits; older
  than this commit). With one commit asked about in its turn and one with fewer attempts,
  both of which use up their run, a commit that was never asked about is never submitted,
  at any stop, although each stop comes to its commits with all 41 seconds. README's bold
  property ("submitted at least once in every N stops", while 8 seconds are left) does not
  hold for it, and every stop says "they are judged at the next stop".

What I would fix before a merge: HR10-M-1 (one condition in `seen` or `uncommitted`, one
test), HR10-M-2 (one comparison against the mark of what the index holds, one test), and
HR10-M-3 (one condition in `elsewhere`, one test). HR10-L-1 needs the sentence's first
clause and one README paragraph corrected. HR10-L-2 and HR10-L-3 can follow.

## How this was done

- Source read from `git archive 5c387a3` (under my scratch directory on disk). No `cargo`, no
  build, no `toolbox`, `podman` or `distrobox` call, nothing at GitHub, no network. The only
  file written in the worktree is this report.
- Binary: `/var/home/louranicas/.cache/ddf-t/review-bin/v4r18/release/deep-diff-forge`
  (SHA-256 `6cd99e47…ef22`); for "is it new", `…/v4r17/release/…` with the ninth review's
  base repository. Judge: the real forge-warden 0.2.0 (`…/target-fw/release/forge-warden`,
  SHA-256 `55bf9fe6…`), on the host. Git 2.56.0. Load average 3 to 6 while I worked.
- Scripts, notes and outputs: `/tmp/claude-ddf-hooks-review-10/` (`NOTES.md`, `lib10.py`,
  `out/`). Every repository and case tree: `/var/home/louranicas/.cache/ddf-t/hooks-review-10/`
  (`base-repo` admits this build's ranker, commit `b215679`; `base-both` admits this build
  and `v4r17`; `base-long`; `home`, a private `HOME` set inside the driver). I removed my
  case directories when their output was saved, so a script must be run again to look at
  one. `/tmp` had 442,000 inodes free throughout.
- Where a verdict had to be missing or slow the judge is a wrapper that runs the real judge
  for everything else: `tools/oj` (mine: logs each submission by the first path of the
  patch after `--patch`; sleeps or gives no verdict for patches that hold a chosen word)
  and the eighth review's `tools/tj`. A slow Git is the ninth review's shim in front of
  `/usr/bin/git` (`tools/hanggit`, `tools/untilgit`). Runs that use them say so.
- I killed no process. I did not read Vesper's round 18 report (it appeared in the worktree,
  untracked, while I worked).

## Findings reproduced with the real judge

### HR10-M-1 (medium; ORDINARY WORKFLOW; older): a stop that could not read the tree forgets the staged change, which then blocks a second time

`uncommitted` (`hook_run.rs` 2428) returns `tree: Err(reason), staged: Ok(None),
index_same: false` when the tree's change cannot be read ("what stops the tree from being
read stops the index too, and is said once", 2459). `tree_units` (3397) turns
`staged: Ok(None)` into "no verdict on the index: `None`", and `seen` (3469) reads `None` as
"the index holds no failing line": `index_whole = Some(empty)`, so `Looked.index` is
`Some(empty)`. `remember_within` (3945 onwards) then keeps nothing under `s` and nothing
under `k`. The stop did not look at the index; it remembers it as clean.

`wipe.py 1` (`out/wipe-1.txt`; one key staged, then repaired in the tree and not staged
again; nothing simulated):

```
-- 1 1 failing file staged, repaired in the tree (repair not staged)   => BLOCK              mem: m/s:1
        | Staged changes in "…": FAIL
-- 2 again                                                             => allow:FAIL STANDS  mem: m/s:1
        | Staged changes in "…": FAIL, which stands
-- 3 an untracked file that cannot be read is in the tree (mode 000)   => allow:UNJUDGED     mem: -
-- 4 again                                                             => allow:UNJUDGED     mem: -
-- 5 that is undone; the index is as it was at step 1 (True)           => BLOCK              mem: m/s:1
        | Staged changes in "…": FAIL
-- 6 again                                                             => allow:FAIL STANDS  mem: m/s:1
```

The same with a path marked skip-worktree for one stop (`wipe.py 1 sparse`,
`out/wipe-1-sparse.txt`), and with a Git that is slow at one stop and nothing else changed
(`wipe.py 1 slowgit`, `out/wipe-1-slowgit.txt`: `tools/hanggit` hangs in
`ls-files --others`; that stop takes 41.0 s and says "the change could not be read from
Git: `git ls-files` did not finish in the time a hook has"; the next stop, fast, BLOCKS for
the staged change again). On `v4r17` the same (`out/wipe-1-v4r17.txt`), so this is not new
with this commit; it is inside this commit's claim 3 and the sentence in the doc comment of
`remember` ("a tree this stop did not look at, and a change that got no verdict, keep what
is remembered").

What it breaks: claim 3 ("an unchanged failure blocks twice only where a stop said its
memory could not be kept"); README 516 to 517 ("it then stands for as long as the index
holds it, whatever the working tree does meanwhile"). It is conservative: an extra block,
no failure missed. Who meets it: a session with a staged failure that stands, at whose
stop the tree cannot be read once: a root-owned or mode-000 file left by a tool, a slow
`git ls-files` on a loaded machine or a network file system. The kept lines (`k`) go the
same way.

Cheap fix: a tree that could not be read says nothing of its index: `Looked.index` must be
`None` then (make `uncommitted` return the reason for `staged` too, or pass "the tree was
not read" to `seen`). A test: a staged failure that has blocked; a stop with an unreadable
untracked file; the file removed; the next stop must not block.

```bash
W=/tmp/claude-ddf-hooks-review-10; python3 -I $W/wipe.py 1; python3 -I $W/wipe.py 1 sparse; python3 -I $W/wipe.py 1 slowgit
```

### HR10-M-2 (medium; ORDINARY WORKFLOW, rare; a regression of this commit): past 256 staged lines, a tree that fails again in part and is repaired again in part makes the untouched staged failure block again

Before this commit a staged failure of more than 256 lines was not cut (`failure_part`
returned all of it), so "the staged change's own" was the whole index whatever the tree
had. Now it is cut line by line (`seen`, 3473 to 3475), which is what closes HR9-M-1. What
is remembered did not follow: when the tree has some of the lines again, the mark of the
former whole is kept under `k` (`remember_within` 3984, `mark == held`), and the smaller
own failure under `s` as lines. `beyond` (`hook.rs` 1114) lets a mark cover a failure only
when the failure is exactly what the mark stands for. So when the tree is repaired again
in part, the lines that become the staged change's own again are covered by nothing: they
are in the mark, and the mark says nothing of a part.

`km.py c1 300 100 50` (`out/km-c1-300.txt`; the index holds 300 key files from before
step 1 and is never touched; `git write-tree` is the same at the end):

```
-- 1 300 failing files staged, all repaired in the tree (repair not staged)  => BLOCK              mem: m/s:1(mark)
-- 2 again                                                                   => allow:HOLD         mem: m/s:1(mark)
        | Staged changes in "…": FAIL, which stands
-- 3 the keys are back in 100 files of the tree (documented: blocks as the tree's)
                                                                             => BLOCK              mem: m/k:1(mark) m/s:200 m/u:100
-- 4 again                                                                   => allow:FAIL STANDS  mem: m/k:1(mark) m/s:200 m/u:100
-- 5 50 of those 100 repaired again in the tree; INDEX UNTOUCHED             => BLOCK              mem: m/k:1(mark) m/s:250 m/u:50
        | Staged changes in "…": FAIL
        | A failure here has blocked before and is not what it was. It was remembered by a digest of all its lines (…)
        | Uncommitted changes in "…": FAIL, which stands
-- 6 again                                                                   => allow:FAIL STANDS
-- 7 the rest repaired again in the tree; INDEX UNTOUCHED                    => allow:HOLD         mem: m/k:250 m/s:1(mark)
   index tree unchanged since step 1: True
```

At step 5 the agent has only repaired files in its working tree. Every line the block
lists was put to it at step 1 as the staged change's own and has been in the index ever
since. Controls: `km.py c1 200 100 50` (`out/km-c1-200.txt`): step 5 is FAIL STANDS;
`km.py c1 257 100 50`: step 5 BLOCKS; the build before
(`DDF=…/v4r17/… BASE=/tmp/claude-ddf-hooks-review-9/base-repo python3 -I km.py c1 300 100 50`,
`out/km-c1-300-v4r17.txt`): step 5 is FAIL STANDS. So it came with this commit.

What it breaks: README 516 to 517 ("it then stands for as long as the index holds it,
whatever the working tree does meanwhile"); README 555 to 561, which says of the digest of
a staged change "there, and only there [where a stop said 'could not keep'], what is staged
may block once more when the tree is repaired again"; claim 3. One can read the digest
sentence ("any change in it … blocks again as a whole") as covering it, since the staged
change's OWN failure grew from 200 to 250 lines; the block does say "not what it was". But
what grew back had blocked as this unit's own, the index did not change, and the same
history under 257 lines stands. It is conservative (an extra block). It needs more than 256
failing lines staged and a tree that fails again in part, so it is rare.

The property test lets it through by design: `an_unchanged_failure_blocks_twice_only_where_a_stop_said_so`
(`hook_run.rs` 6725 to 6734) accepts any block of a staged change that its exact model
calls not new, whenever a mark is remembered for it and its own lines differ from the last
stop's (`beyond_model`). So "compares every stop with an exact model" holds only where no
mark is involved.

Cheap fix: a mark remembered for the staged change (`s` or `k`) that equals the mark of
what the index holds at this stop (`remember_within` computes it as `held`) says that every
line the index holds has blocked for the staged change; then no line of the staged
change's own is new, whatever part of the index it is. `seen` has both verdicts and can
pass that on. A test: the sequence above, step 5 must not block; and the property test
without the `beyond_model` exception for this case.

```bash
W=/tmp/claude-ddf-hooks-review-10; python3 -I $W/km.py c1 300 100 50; python3 -I $W/km.py c1 200 100 50
```

### HR10-M-3 (medium; ORDINARY WORKFLOW in one layout; older): in a bare repository with its working trees inside it, the tree a stop is in is not judged

`elsewhere` (`hook_run.rs` 1891) answers "nothing more to judge" when the session's
directory is the project's top **or lies under the Git common directory**
(`cwd.starts_with(&place.common)`, 1897; the comment: "a place inside the Git directory,
where no working tree is"). In the layout `git clone --bare URL repo.git; cd repo.git;
git worktree add main; git worktree add feature` the common directory is `repo.git`, and
every working tree is under it. So for a session whose project is `repo.git/main`, a stop
made from `repo.git/feature` (or from `repo.git/main/.claude/worktrees/x`, which is under
`repo.git` too) judges `main` only.

`bare2.py` (`out/bare2.txt`; the judge is `tools/oj`, which only logs and runs the real
judge; `main/` is clean, a key is in `feature/`, every stop is made FROM `feature/`):

```
git -C feature rev-parse --show-toplevel: …git/feature | --git-common-dir: ….git
-- 1 main/ clean; a key in feature/, the stop made FROM feature/      => allow  judge saw: []
        | deep-diff-forge: The working tree "….git/feature" of this project has changes that are not committed, and the session's directory is not in it: they were not judged (a stop made from inside it judges them; its commits are judged from anywhere).
-- 2 again, from feature/lib                                          => allow  judge saw: []
        | deep-diff-forge: 1 thing in this project is not judged: the working tree "….git/feature".
-- 3 the key staged in feature/                                       => allow  judge saw: []
-- 4 the key committed in feature/                                    => BLOCK  judge saw: ['lib/linked.rs']
        | Commit 1c304d6cb429: FAIL
```

Control, the same with the trees beside the bare repository (`od.py bare`, last row,
`out/od-bare.txt`): the judge sees `lib/linked.rs`, then `lib/main.rs`, and the stop blocks
naming both. On `v4r17` the same four rows (`out/bare2-v4r17.txt`): not new with this
commit, but it is claim 4 ("the tree a stop is in is judged first").

Rating. No stop is silent: the tree is named as not judged at every stop, and its commits
are judged. But the sentence is false there (the session's directory IS in it, and a stop
made from inside it does not judge it), so the agent is given a remedy that cannot work,
and uncommitted and staged work in the tree the agent is in is never put to the judge. By
the letter of the rule ("a new failure in an ordinary workflow that never blocks" is high)
one can argue high for a session that does not commit. I rate it medium because it is
said at every stop, the commit blocks, and it takes this one layout; if the owner counts
that layout as an ordinary one, it is the first thing to fix.

Cheap fix: do not decide by the path. Ask `find(cwd)` first; a directory under the common
directory is "nowhere" only when Git says it is in no working tree. A test: a bare
repository, two trees added inside it, a session begun in one, a stop from the other with
a credential there: it must block and the tree must be first.

```bash
W=/tmp/claude-ddf-hooks-review-10; python3 -I $W/bare2.py; python3 -I $W/od.py bare
```

### HR10-L-1 (low; ORDINARY WORKFLOW; this commit): the third memory sentence is reached with one tree, and its first clause is not true there

`remember_within` (3963 to 3990) gives up a mark remembered for a staged change when it is
neither what the index holds nor what is the staged change's own, and names the unit in
`given_up`; `memory_notes` (3875) words it as "This session has more failing lines than
the hook keeps one by one (2048): of what is staged in 1 working tree it could not keep …".
That needs no 2,048 lines: a staged failure of more than 256 lines has a mark of its own.

`km.py c2 300` (`out/km-c2-300.txt`):

```
-- 1 300 failing files staged, all repaired in the tree                => BLOCK              mem: m/s:1(mark)
-- 3 the keys are back in all files of the tree (documented block)     => BLOCK              mem: m/k:1(mark) m/u:1(mark)
-- 4 again                                                             => allow:FAIL STANDS
-- 5 ONE file unstaged (git reset -- file); tree unchanged             => allow:FAIL STANDS  mem: m/u:1(mark)
        | This session has more failing lines than the hook keeps one by one (2048): of what is staged in 1 working tree it could not keep which lines had already blocked while the working tree's own failure has them too. When that working tree is repaired again, what is staged there may block once more, although the index has not changed.
-- 7 the tree repaired again; index untouched since 5                  => BLOCK              mem: m/s:1(mark)
```

The session has 300 failing lines, not more than 2,048. With 200 lines (`km.py c2 200`)
step 5 keeps the other 199 lines and step 7 stands, so unstaging one file of 300 costs the
memory of the other 299; that is said, as the rule asks, but with the wrong cause. README
565 to 570 says that with 256 working trees or fewer these sentences are not reached by a
run of the hooks ("So these sentences are held by unit tests, not by a run of the hooks");
the round 18 request says the same. For this one it is not so, and a run of the hooks can
hold it.

Cheap: word the sentence by its cause (the staged failure was remembered by a digest, and
the index no longer holds exactly that), correct the README paragraph, and add the run
above as an integration test.

```bash
W=/tmp/claude-ddf-hooks-review-10; python3 -I $W/km.py c2 300; python3 -I $W/km.py c2 200
```

### HR10-L-2 (low; ORDINARY WORKFLOW with a slow judge; a regression of this commit): the stop's own linked tree gets less of the stop than before

`judge_units` (3617) counts two patches for a tree that has not been read yet
(`later = (count - at - 1) * 2`), and `run_time` keeps five seconds for each run owed. From
a linked tree the judging has 35 seconds; the first run is owed 0 + 2 + 2 runs after it
and gets 35 - 20 = 15 seconds (10 when the linked tree has a staged change too, whose own
run then gets 5). Before this commit both trees were read first and the count was exact:
25 seconds. The time kept for the project's tree is not given back when that tree turns
out to have nothing to judge.

`ot.py` (`out/ot-*.txt`; the stop is made from the linked tree, the project's tree is
clean; `tools/oj` sleeps before it runs the real judge on the linked tree's patch):

| the judge takes | linked tree has | `v4r18` | `v4r17` |
|---|---|---|---|
| 17 s | one change | UNJUDGED ("the judge was stopped before it gave a verdict"); the stop ends at 15.1 s | BLOCK at 17.3 s |
| 12 s each | a change and a staged change | both UNJUDGED (stopped at 10 s and after 5 s more); 15.1 s | BLOCK at 24.3 s |

This is the cost of the answer to VD4R17-L-1, and it is the symptom that finding was
about (the stop's own tree UNJUDGED at every such stop), now for a judge that takes 15 to
25 seconds where it was for a slow run on the main tree. The real judge took 0.09 s, 0.39 s
and 1.41 s on patches of 1, 8 and 32 MiB here, so it needs a much slower machine or judge.
The order itself holds: `ot.py 40 dirty` (first run stopped at 15 s, then `lib/main.rs` is
judged and blocks) and `ot.py noverdict dirty` (first tree UNJUDGED, the second still read
and judged).

Cheap: when the first tree's runs are done, nothing is owed any more for a second tree
that has no patch; or look whether the project's tree has anything to judge (one
`git status`) before the time is divided.

```bash
W=/tmp/claude-ddf-hooks-review-10; python3 -I $W/ot.py 17 clean; python3 -I $W/ot.py 12 clean staged
```

### HR10-L-3 (low; ORDINARY WORKFLOW only with a judge that hangs on two commits; older): a commit never asked about that is never asked

`sorted_commits` (2828) puts the commit whose turn it is first, then one commit of those
with fewer attempts, then the commits never asked about. The first run gets what is left
less five seconds (at most 30), the second all that is left (`run_time` with nothing
owed). When both use up their run, nothing is left for a commit that was never asked
about; and since no patch gets a verdict at such a stop and there is no time for the
probe, the second commit's attempts never count, so it never joins the first in the turn
queue, where they would alternate and leave 11 seconds.

`st3.py` (`out/st3.txt`; `tools/oj` gives no verdict for commit T at five stops that
count, then sleeps 100 s on T and on a second commit A; everything else is the real
judge; the tree is clean; C is a commit with a credential made afterwards):

```
-- A committed; clean tree                         => allow [41.0s] judge was run for: ['lib/ct.rs', 'lib/ca.rs'] | judged file: {'T': 'n6@6…', 'A': 'n0@6…'}
-- C (a credential) committed; stop 1 after it     => allow [41.0s] judge was run for: ['lib/ct.rs', 'lib/ca.rs'] | {'T': 'n7@7…', 'A': 'n0@7…'}
        | 1 commit of this session: UNJUDGED
        |   not judged: they were not judged yet: one stop judges as many commits as its time allows, the oldest first; they are judged at the next stop.
-- C (a credential) committed; stop 2 after it     => allow [41.0s] judge was run for: ['lib/ct.rs', 'lib/ca.rs'] | {'T': 'n8@8…', 'A': 'n0@8…'}
-- C (a credential) committed; stop 3 after it     => allow [41.0s] judge was run for: ['lib/ct.rs', 'lib/ca.rs'] | {'T': 'n9@9…', 'A': 'n0@9…'}
```

C is never submitted; by reading, that goes on for as long as the tree is clean (A stays
at `n0`). Each stop comes to its commits with the whole 41 seconds, so the precondition of
README 391 to 400 holds (8 seconds left), and the bold sentence there ("Every commit
without a verdict is submitted to the judge at least once in every N stops") does not hold
for C. It is never silent (UNJUDGED at every stop), but "they are judged at the next stop"
is not true, and a commit with a credential is never put to the judge. It needs a judge
that hangs on two different commits, which I could only make with a wrapper; hence low.

Cheap: when a commit was never asked about, the second owed run leaves `COMMIT_FLOOR` for
it (it is owed too); or an ask the judge's own time ended counts as an attempt whatever
the chain did, so both commits come into the turn queue.

```bash
W=/tmp/claude-ddf-hooks-review-10; python3 -I $W/st3.py 3     # about four minutes
```

## The areas of the brief: what was tried and whether it held

0. **This round's rework.**
   - LINES AND MARKS (`lm.py SCENARIO N`, `out/lm-*.txt`; N = 1, 200, 256, 257, 600; one
     failing file a line). Staged one, half, all but one, all, one harmless edit, none,
     the other half: FAIL STANDS at every step, for every N (HR9-M-1 is closed; the ninth
     review's `p1.py 257`, `p2.py 257`, `c1.py` rerun: stand). A part committed: BLOCK for
     the commit (documented); for 257 and 600 the same stop also says of the rest "not
     what it was" (it changed; the same stop). A part stashed (`git stash -u -- paths`)
     and popped: three blocks, as README says. One STAGED file repaired in the tree:
     BLOCK as "Staged changes" (documented), and for 257 and 600 the tree's failure blocks
     with "not what it was" (stated: the repair of one part). One UNSTAGED file repaired:
     stands up to 256, blocks as a whole past it (stated). One new failing file, unstaged
     and then staged: BLOCK each time; "New at this stop" and "Stands beside it" up to 256
     remembered lines (also from 256 to 257, HR9-L-3 (b) is closed), "not what it was" where
     the memory is a mark. A failing file replaced by another path, unstaged and staged:
     BLOCK for every N. A second key in a failing file: BLOCK for every N. A key replaced
     by another in the same file: stands for every N (the stated identity). I found no
     changed failure that stands. "Not what it was" appeared only where the unit's memory
     held a mark. Broken: HR10-M-2 (staged, past 256).
   - THE STAGED KEPT MARK (`km.py`). Lines that left the index and came back, with a stop
     that looked in between (`km.py leave 300` and `200`): BLOCK. A whole staged failure of
     300 that is in the tree again and repaired again: stands (the kept mark). A part:
     HR10-M-2. The third sentence where it should not be, in its wording: HR10-L-1 (reached
     with one tree). Missing where it should be: not found; HR10-M-1 is a loss of another
     kind (no mark; the stop did not look). The sentences for more than 2,048 failing
     lines: not reached (read only).
   - THE ORDER (`od.py`, `ot.py`, the judge's calls logged by `tools/oj`). From a linked
     tree's top and from deep inside it: `lib/linked.rs` then `lib/main.rs`. From deep in
     the main tree, from outside every tree, from the Git directory, from the linked
     tree's administrative directory: the project only, the other tree named. The project
     itself a linked tree, the stop from the main tree and from a second linked tree: the
     stop's tree first, then the project. A linked tree nested in the project
     (`.claude/worktrees/w1`): first. From a submodule: the project is judged, the
     submodule named twice as not judged (as stated). A first tree with no verdict: the
     second is still read and judged. Broken: HR10-M-3 (trees inside a bare repository);
     HR10-L-2 (the time).
   - ASKS (`tm.py`, the ninth review's driver on this build, `out/tm-*.txt`). 7.9 s left:
     both asks and the probe submitted, both written (`n8@8`, `n2@8`). 3.1 s: the first
     submitted and written (`n8@8`), the second not submitted, nothing written for it,
     "not asked at this stop" (HR9-L-2 is closed). 2.9 s: nothing submitted, nothing
     written. 3.1 s with a judge that hangs: the first is cut by the stop's end and
     written (`n8@8`, "the judge was stopped"); 8.2 s with a judge that hangs: both
     submitted, both written, the second with its turn used and no attempt counted
     (`n1@8`). The log and the judged file agree in all five. Starving: HR10-L-3, which is
     not at these edges.
   - MORE THAN 256 WORKING TREES (`wt.py`, 257 linked trees made on disk and removed
     again; `out/wt-*.txt`). A session that began with one tree: its tree judged (a key
     BLOCKS, then stands), "The commits of this session: UNJUDGED" with the reason at
     every stop, also after the key is committed; from a linked tree the stop's own tree
     blocks; 241 trees counted as not looked at. A session that BEGINS there: the start
     says stops will not be judged; every stop is UNJUDGED with the reason, with a key in
     the tree and with the key committed; advice says the same. Never silent, as stated.
     My answer to the question: judge the tree's own change there too. It needs the base
     (`HEAD` at the start) and the policy, not the list of commits; the session would then
     be what the first case already is. As it stands a credential in the tree is not put
     to the judge in a repository that merely has many trees. Two words to mend: the reason
     begins "invalid Git selection:", and "`git worktree prune` removes the ones that are
     gone" does nothing for 257 trees that exist.
   - WHAT HELD BEFORE, on this build. The eighth and ninth reviews' scripts copied with
     their paths changed (`r9/`, outputs `r9/out/`): tree identity (`t1.sh` move, mv and
     repair, mv without, remove and add, lock, gc, touched files, restores), relative
     paths (`t2.sh`), project a linked tree, bare with trees beside it, many trees
     (`t3.sh`), line identity (`t4.sh`, `t7.sh`), `t9.sh`, `t10.sh`, the round 14 table
     (`r6/t6.sh`): the decision and first words of every stop are the same as in the ninth
     review's outputs on `v4r17` (compared after taking out paths and ids; silent in
     exactly the all-pass rows). Nine trees of 256 failing files (`m1.py`): two become
     marks at the ninth stop, tree 1 stands when revisited, a renamed file and another
     rule on one path BLOCK with the sentence. The crowded stop (`t15b.sh`): 7,052
     characters at most, counting lines whole. The lock with eight first hooks at once
     (`t6.sh par`). The turn from the wrapper's log (`t5.py mix`, 30 stops): no violation.
     `h1.py`, `k1.py`, `o1.py`: HR9-L-3 (a), (b) and VD4R17-L-1 closed.
   - STATED LIMITS. A key replaced by another in the same file: stated in README 497 to
     508 and in a line of its own in every message where a working tree's failure stands
     (checked under FAIL STANDS, a HOLD head and a block); accurate. The stash round trip:
     three blocks, as stated. No birth time with reused inode numbers: both directions are
     now in README 524 to 539; accurate to the code's comment; not testable here. A branch
     moved back onto an old failing commit: read only (the range sentence says it). The
     three memory sentences held by unit tests only: not accurate for the third
     (HR10-L-1).
   - STATE FILES (`xb.py`, `out/xb-*.txt`). A state marked v7: UNJUDGED at the stop and in
     advice, nothing recorded over it. A state written by this build (with a `k` mark)
     read by `v4r17` and back: no crash, no pass, the new failing file blocks in both. A
     state written by `v4r17` (a marked staged failure with part of it in the tree) read
     by this build: one more block at the first stop ("not what it was"), which is
     HR10-M-2's cut seen from the other side; the CHANGELOG warns about going back, not
     about this.
   - `CLAUDE_PROJECT_DIR` WRONG (`cp.py`, `out/cp.txt`): missing, not a repository, a
     subdirectory, relative, empty: the project is found from `cwd` and the key blocks.
     Another repository: named as not judged at every stop (stated). Changed between
     start and stop, or set only at the stop: UNJUDGED, "this session began in … and the
     project found now is …". Never silent.
   - Which commits are the session's, the staged unit against index states, the policy
     from the base: not gone through again beyond what the reruns pass through.
1. **A pass that should not be one.** Not gone through again.
2. **A block that cannot be escaped, or one that is waved through.** HR10-M-1 and
   HR10-M-2 are blocks that come a second time; both end at the next stop. Nothing waved
   through.
3. **The hook input and output contract.** Every output of every run was parsed as JSON by
   the driver; none failed. Nothing else tried.
4. **Resources and files.** The scratch directory under the Git directory was empty after
   every stop I looked at, also after runs of the judge that were cut. State files are
   0600 in a 0700 directory. Longest stops: 41.0 s (a hanging judge; a hanging
   `ls-files`). Nothing of mine was written outside my two scratch directories.
5. **`agent setup --hooks`, `agent doctor --gate`.** `doctor --gate` run once in the base
   repository (every link read as expected). Not tested.
6. **`schema signals`, `explain signal`.** Run once each. Not tested.
7. **The tests.** By reading; I ran no test (no `cargo`).
   - `an_unchanged_failure_blocks_twice_only_where_a_stop_said_so`: the `beyond_model`
     exception (6725 to 6734) accepts HR10-M-2. Its stops never have the hook's own case
     "the tree could not be read, so the index was not read, and the index is not empty"
     (it passes an index verdict or "no verdict" whenever the index is not the tree), so
     HR10-M-1 passes it.
   - No test in `agent_hook.rs` makes a stop that cannot read the tree while a staged
     failure stands, and none has a bare repository with its trees inside it (the one
     bare case, 5746, has the tree beside it).
   - Nothing holds the time the stop's own linked tree gets (HR10-L-2), or that a commit
     never asked about is asked while two others use up their runs (HR10-L-3).
   - The third memory sentence has no run of the hooks, and can have one (HR10-L-1).

## Suspected only (not reproduced)

- HR10-M-1 by other causes of an unreadable tree (`HEAD` that cannot be read, a sparse
  checkout made during the session): the same code path by reading; I ran the unreadable
  file, one skip-worktree path and the slow `ls-files`.
- HR10-M-3 for `main/.claude/worktrees/x` in that layout: by reading (it is under the
  common directory too); I ran `feature/` beside `main/`.
- HR10-L-3 "for ever": three stops run; the rest by reading.
- A stop from a linked tree when the project's tree cannot be read
  (`Elsewhere::Failed`): by reading, said as UNJUDGED; not run.

## What I did not look at

Areas 1, 3, 5 and 6 of the brief beyond what is said above; `verdict_v2.rs`;
`judge_patch.rs`; attributes, filters, CRLF, symlinks, binary files; more than 2,048
failing lines live; a file system with no birth time or no `flock`; a full disk or a state
directory that cannot be written; two stops at once beyond the eighth review's parallel
advice hooks; the eighth review's `t6.sh kill` (it picks a process by name), its `t11.sh`
and `t5.py slow`; the ninth review's `w1.py` (which commits are the session's), `e1.py`
and `e2.py` (longest stops, trees not reached), `u1.py`, `st1.py`, `b1.py`; a branch moved
back onto an old failing commit; what Claude Code itself does with `stop_hook_active` or
with two hooks at once; Vesper's round 18 report.
