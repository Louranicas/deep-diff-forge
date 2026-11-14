# Ninth independent review of the agent hooks branch (`agent/loop`, commit `f604d58`)

Date: 2026-11-15 (08:12 to 08:47 AEDT, by the clock). Reviewer: an independent Claude agent that took no part in the work.
Brief: `~/.local/state/habitat-claude/ddf-gap-probes-20261113/ddf-hooks-independent-review-9-brief.md`.
Under review: `d4dbc91..f604d58`, mainly the rework `8746cd7..f604d58` ("nothing remembered is
dropped in silence; a tree is known by when it was made; the session's own change comes first").

## Verdict: GO-WITH-FIXES

I found no stop that prints nothing while something is open, and no new or changed failure
that is taken for one that stands, outside the identity the documents state (rule, signal,
path, number). The digest marks noticed every change I made to a marked failure. The tree
identity by birth time held in every transition I could make on this host.

I found one medium and three lows of my own, all ORDINARY WORKFLOW, all in the direction
"an unchanged failure blocks again" or "a sentence is not accurate". None is a deliberate
evasion. One low of Vesper's round 17 report I corroborate in part.

- **HR9-M-1** (medium; ORDINARY WORKFLOW in a large change; not new with `f604d58`, the
  build before does the same). When the uncommitted failure of a tree has more than 256
  lines (it is then known by its digest), a plain `git add` of some of the failing files
  makes the stop block again, as "Staged changes", for files that have not changed and have
  already blocked; every further partial `git add` blocks once more. With 256 lines the same
  sequence stands. README line 513 says "Staging, and editing other files, does not make a
  failure block again"; claim 3 says an unchanged failure blocks twice only where a stop
  said its memory could not be kept. No stop said anything about memory.
- **HR9-L-1** (low; ORDINARY WORKFLOW, crowded; this commit's `within_bound`). When the
  session has more than 2,048 failing lines, the lines kept for a staged change (`k`) of a
  tree the stop is not looking at are let go, and no stop says so. The staged failure then
  blocks a second time when the tree is repaired again, and that block does not say that it
  is not new. README line 537 says of this shortening "nothing is lost by that"; the commit
  says "nothing remembered is dropped in silence".
- **HR9-L-2** (low; ORDINARY WORKFLOW on a slow stop). With a little over three seconds of
  judging left, the first owed ask is submitted to the judge and answered ("no verdict"),
  and the stop then writes nothing and says of that commit "not asked at this stop for lack
  of time (it keeps its turn)". It was asked and it was answered.
- **HR9-L-3** (low; ORDINARY WORKFLOW; two sentences missing where they belong). (a) The new
  sentence that says what "stands" means for an uncommitted change is said only when the
  message is headed FAIL STANDS; under a HOLD head (a held commit beside it) the same
  standing failure is "FAIL, which stands" with no such sentence, and in a block it is
  "Stands beside it … not new". (b) The new sentence that a block "is not all new" is not
  said when a failure of up to 256 remembered lines grows past 256: the block lists the
  whole failure as "FAIL".
- **VD4R17-L-1** (Vesper's finding, round 17; low; ORDINARY WORKFLOW). I read the order in
  the source before I read Vesper's report and did not rate it; that was a miss of mine. I
  corroborate the order: a stop made from a linked tree submits the project's main tree's
  patch first (`o1.py`). I did not reproduce the starvation that follows from it.

What I would fix before a merge: HR9-M-1 (a few lines in `seen` and one test) and
VD4R17-L-1 (the order of two trees). HR9-L-1 needs either a mark for the kept lines or one
honest sentence in README and in the block. The other lows can follow.

## Three mistakes of mine, and one thing about this machine

- **A file written outside my scratch directory.** At 08:30:39 I wrote my script `st1.py`
  to `/var/home/louranicas/st1.py`. The command began `cd SCRATCH && ( … ) &`, so the `cd`
  ran in the background list and the heredoc after it wrote in the tool's default
  directory. I found it at 08:45 (`find -maxdepth 1 -newer`), moved it into my scratch
  directory with `mv`, and checked that nothing else of mine is in the home directory. It
  ran once from there; it writes only under my scratch directory.
- **I removed my own `cases/` directory while one of my background runs was using it**
  (`e1.py worktreelist`). That run was invalid; I ran it again and use only the second.
- **I tipped `/tmp` over its inode limit three times** (about 08:17 to 08:20, 08:21 to
  08:22, 08:43 to 08:44). `/tmp` is a tmpfs with 1,048,576 inodes and had about 6,000 free
  when I began; my runs never held more than about 5,000, but that was enough. While it
  was full no command of mine could start ("disk full", ENOSPC), and anything else on this
  machine that created a file under `/tmp` in those minutes may have failed the same way.
  After the first time I ran one experiment at a time and removed my own case directories
  after each (plain `rm -rf` of paths under my scratch directory; nothing else was removed
  or killed). Lost to it: the first run of `m1.py`, the first run of `tm.py 8.2 20`, and
  the last (control) row of the saved rerun of `w1.py`; each was run again or had been
  complete before.
- **The machine:** the inodes of `/tmp` are held by earlier work, not by this review
  (`du --inodes`): `/tmp/claude-1000` 355 thousand, `/tmp/claude-ddf-hooks-review-7` 113
  thousand, `-review-6` 68 thousand, `/tmp/claude-ddf-lexfix` 49 thousand,
  `/tmp/claude-ddf-hooks-round14` 42 thousand, `/tmp/claude-ddf-hooks-review` 34 thousand,
  and about twenty more of 10 to 27 thousand. At 08:45 about 4,700 were free. The next
  reviewer, the next gate and the next fuzz run will hit this. It is the owner's to clear.

## How this was done

- Source read from `git archive f604d58` (extracted under my scratch directory, removed
  again at the end to give the inodes back). No `cargo`, no build, no `toolbox`, `podman` or
  `distrobox` call, nothing at GitHub, no network. The only file written in the worktree is
  this report.
- Binary: `/var/home/louranicas/.cache/ddf-t/review-bin/v4r17/release/deep-diff-forge`
  (SHA-256 `fbb014f6…fc70`); for "is it new", the build before, `…/v4r16/release/…`.
  Judge: the real forge-warden 0.2.0 (`…/target-fw/release/forge-warden`, SHA-256
  `55bf9fe6…`), on the host. Git 2.56.0. Load average about 4 when I began (read once).
- Scratch: `/tmp/claude-ddf-hooks-review-9/` (tmpfs). `base-repo` is a clone of the eighth
  review's base with this build's ranker digest committed (`edbf4cb`); `base-long` the same
  with long rule names. Driver `lib9.py` (Python; every hook run has a private `HOME` set
  in it, `XDG_STATE_HOME` and `CLAUDE_PROJECT_DIR` unset, `GIT_CONFIG_GLOBAL=/dev/null`);
  the eighth review's scripts copied with their paths changed (`h.sh`, `t*.sh`, `t5.py`,
  `r6/t6.sh`); my scripts `m1.py`, `m2.py`, `p1.py`, `p2.py`, `c1.py`, `k1.py`, `h1.py`,
  `o1.py`, `tm.py`, `e1.py`, `e2.py`, `b1.py`, `b2.py`, `st1.py`, `u1.py`, `w1.py`,
  `i1.sh`, `t20.sh`; outputs in `out/`; notes in `NOTES.md`. Case directories are removed
  after each run, so a script must be run again to look at one.
- Where a verdict had to be missing or slow the judge is a wrapper that says so in its
  header: `tools/tj` (the eighth review's: logs each submission by the argument after
  `--patch`, gives no verdict for marked patches, can sleep, else runs the real judge
  unchanged). A slow Git is a shim in front of `/usr/bin/git`: `tools/untilgit` (sleeps in
  one chosen call until a wall-clock time), `tools/hanggit` (sleeps in chosen
  subcommands), `tools/slowstatus` (the eighth review's). Runs that use them say so.
- I read Vesper's round 17 report (`docs/reviews/vesper-v4-round17-report-20261115.md`,
  written while I worked) only after my own findings were reproduced.

## Findings reproduced with the real judge

### HR9-M-1 (medium; ORDINARY WORKFLOW in a large change; also on `8746cd7`): staging part of a failure of more than 256 lines blocks again

`seen` (`hook_run.rs` 3419) decides which of the index's failing lines are the staged
change's own: the ones the uncommitted change's failure does not have
(`hook::lines_beyond(&failed, &cover)`, line 3455). `cover` is `failure_lines` of the
tree's verdict, and `failure_lines` returns one mark when there are more than 256 lines
(`hook.rs` 1014). A mark covers no single line (`lines_beyond`, `hook.rs` 1088: either the
whole failure is the mark, or every line is beyond it). So with a tree failure of 257 lines
and an index that holds a part of it, every staged line is called the staged change's own,
the unit `TREE/s` has lines that were never remembered under that tag, and the stop blocks.
The comment over this code says what goes wrong, for the one case it guards (no verdict on
the tree, lines 3442 to 3446: "called its own, lines that have blocked as the tree's would
block a second time"); with a verdict on the tree and a native mark it is not guarded.

`p1.py 257`, `out/p1-257.txt` (nothing simulated; 257 files with a key-like value):

```
-- 1 257 failing files, nothing staged             => BLOCK          m/u: 1 (mark)
-- 2 again                                         => allow: FAIL STANDS
-- 3 `git add lib/g/k1.rs` (nothing else changed)  => BLOCK          m/u: 1 (mark)  m/s: 1
      | Staged changes in "…": FAIL
      | Uncommitted changes in "…": FAIL, which stands
      | What is staged fails for as long as the index holds it: stage the repaired file (`git add`) or take the old content out of the index (`git restore --staged`).
-- 4 again                                         => allow: FAIL STANDS
-- 5 two more files staged                         => BLOCK  "Staged changes …: FAIL, new at this stop" / "Stands beside it …"
-- 6 git add -A                                    => allow: FAIL STANDS
```

Control `p1.py 256`, `out/p1-256.txt`: steps 3 and 5 are FAIL STANDS. The same with an
everyday rule, `p2.py 257` (`out/p2-257.txt`): 257 new files under `src/` with no test
change (`tests-travel-with-code`, one line a path) block once and stand; `git add` of two of
them blocks again; `p2.py 256` stands. `c1.py` step 4 (`out/c1.txt`) is the same block one
stop after the corner the commit states as UNJUDGED. On the build before
(`DDF=…/v4r16/… BASE=/tmp/claude-ddf-hooks-review-8/base-repo python3 -I p1.py 257`) steps
3 and 5 block too, so this came with the line identity of `8746cd7` and is not a regression
of this commit; this commit's claim 3 and its README paragraph on digests cover it.

What it breaks: README 513 ("Staging, and editing other files, does not make a failure
block again"), README 531 to 534 ("it stands while it is exactly what it was"), claim 3.
The hint it prints is not true of the case either: nothing was repaired. It is
conservative: an extra block, said, no failure missed. Who meets it: an agent with one
large change (a rename over 257 files with no test change; a vendored directory) that
stages in parts, which is how such a change is usually committed. Each part is one more
blocked stop for files it only staged.

Cheap fix: in `seen`, take `cover` from the tree verdict's lines before they are folded
into a mark (the verdict has them at that stop; only what is remembered needs the mark).
A test: 257 failing files, a stop, `git add` of one, a stop that must not block. The
property test cannot see this: its index is only ever all of the tree, empty, or the tree
before one repair (see "The tests").

```bash
W=/tmp/claude-ddf-hooks-review-9; python3 -I $W/p1.py 257; python3 -I $W/p1.py 256; python3 -I $W/p2.py 257
```

### HR9-L-1 (low; ORDINARY WORKFLOW, crowded; this commit): the lines kept for a staged change are let go in silence, and the staged failure then blocks a second time with no word that it is not new

`within_bound` (`hook_run.rs` 3650) puts a unit into its short form when the list has more
than 2,048 entries: its own lines become one mark and "the lines kept for it are let go".
A unit whose own lines are none or one has no mark at all (`short_form`, 3677 to 3686):
the kept lines simply go. For a tree the stop is not looking at, nothing is said
(`Kept::short` names only units the stop looked at). The kept lines are what makes a
staged failure stand "for as long as the index holds it, whatever the working tree does
meanwhile" (README 512; seventh review, HR7-M-1).

`m2.py 7`, `out/m2-7.txt`. Tree A: 250 key files staged, then repaired in the tree and not
staged again:

```
-- 1  A: staged, tree repaired                      => BLOCK "Staged changes …: FAIL"        A/s: 250
-- 2  A: the tree has the keys back                 => BLOCK as the tree's (documented)      A/u: 250  A/k: 250
-- 2c A: tree repaired again, index untouched       => allow; "Staged changes …: FAIL, which stands"
-- 2d A: keys back in the tree                      => BLOCK as the tree's (documented)      A/u: 250  A/k: 250
B1 … B6: six other trees of 250 failing files       => BLOCK each; 2,000 entries
B7: a seventh (2,250 entries before the bound)      => BLOCK; no word about memory;          A/u: 250, A/k gone
-- 3  A: tree repaired again, index untouched since step 1
                                                    => BLOCK "Staged changes …: FAIL"  (no "not what it was", no "Stands beside it")
```

Control `m2.py 2` (two other trees, 1,000 entries; `out/m2-2.txt`): step 3 is
"Staged changes …: FAIL, which stands". So the same staged content, untouched in the index
since step 1, blocks at step 1 and again at step 3, and the only stop that could have said
that memory was given up (B7) said nothing. The request for round 17 states the choice
("When a staged change is shortened to a mark, its kept lines are let go; a later repair of
the tree then blocks"); README does not, and says the opposite in general ("nothing is
lost by that", 537; "those, and only those, may block once more", 541). When the staged
change has two or more own lines the block at least says "A failure here has blocked
before and is not what it was" (a mark is there); with none or one own line, as here, it
says nothing.

It needs more than 2,048 failing lines in the session and the staged-then-repaired
history, so it is rare. It is the last piece of VD4R16-M-1 rather than something new in
kind.

Cheap fix: keep one mark for the kept lines too (a `k` mark: the digest and count of the
lines kept), so that the same index content stands; or count such a unit among the ones
the stop names, and say in README that the kept lines of a staged change are given up
with the rest.

```bash
W=/tmp/claude-ddf-hooks-review-9; python3 -I $W/m2.py 7; python3 -I $W/m2.py 2
```

### HR9-L-2 (low; ORDINARY WORKFLOW on a slow stop): with about three seconds left an owed ask is made and answered, and the stop says it was not asked

`judge_commits` (`hook_run.rs` 3028 to 3035): after an ask that got no verdict, when less
than `COMMIT_FLOOR` is left, "the stop's time ended the asking: not an attempt, and no turn
is used", and `not_asked` words it. That arm does not look at whether the judge was run or
whether it answered. `tm.py 3.1` (`out/tm-3.1.txt`; no linked tree, so the judging has 41
seconds; the Git shim sleeps in `rev-list --topo-order`, the last Git call before the
commits, until 37.9 seconds into the stop; the judge is `tools/tj`, which answers "no
verdict" at once for the two marked commits):

```
   the commits were reached at +37.904 s (judging left then: 3.096 s of 41)
   submitted: +38.015 TK-a  (judging left: 2.985)
   message: 2 commits of this session have no verdict: 0b97… (not asked at this stop for lack of time (it keeps its turn); before: no verdict in 7 attempts; …), c86b… (not asked at this stop …)
   before: TK-a n7@7, TK-b n1@7      after: TK-a n7@7, TK-b n1@7
```

`TK-a` was submitted and the judge answered (exit 3) within 0.1 second; nothing was
written and the message says it was not asked. "Nothing is written for an ask that was not
made" holds; this is the other direction, which no sentence covers (the doc comment speaks
of a run "the stop's end cut short"; this run was not cut). The cost: the turn is not
used, so a session whose stops all reach their commits in this window asks the judge about
the same commit every time. That is inside the stated precondition (under 8 seconds), so
it is a matter of wording and of one comparison: a run the judge finished is an ask that
was made.

The other edges held, from the wrapper's log against the judged file:

| judging left at the commits | judge | first owed ask | second owed ask | written |
|---|---|---|---|---|
| 7.896 s (`out/tm-7.9.txt`) | answers at once | submitted | submitted (and the probe) | `n8@8`, `n2@8` |
| 8.196 s (`out/tm-8.2-hang.txt`) | hangs | submitted, stopped after 3.1 s | submitted with 4.97 s, cut by the stop's end | first `n8@8`; second nothing, "not asked" (stated) |
| 3.096 s (`out/tm-3.1.txt`) | answers at once | submitted and answered | not submitted | nothing (this finding) |
| 2.896 s (`out/tm-2.9.txt`) | n/a | not submitted | not submitted | nothing |

```bash
W=/tmp/claude-ddf-hooks-review-9; python3 -I $W/tm.py 3.1; python3 -I $W/tm.py 7.9; python3 -I $W/tm.py 2.9; python3 -I $W/tm.py 8.2 20
```

### HR9-L-3 (low; ORDINARY WORKFLOW; wording): two of the new sentences are missing where they belong

(a) `STANDS_BY_LINE` is added only in the branch that makes the head "FAIL STANDS"
(`hook.rs` 1865 to 1879). `h1.py`, `out/h1.txt`: a key in `lib/cfg.rs` blocks; the agent
replaces it by another key in another item of the same file; the next stop:

```
== alone:       => "FAIL STANDS. … For a change that is not committed, that means: … Whether they are the same lines of those files the hook does not know."
== heldcommit:  (the session also has a commit the judge holds)
                => "HOLD. The judge wants a person to look at this change (…)."
                   Uncommitted changes in "…": FAIL, which stands          (the sentence is not there)
```

The same is true under an UNJUDGED head and in a block ("Stands beside it, as said before
(it has blocked a stop and is left for a person; not new)"), by reading. README 502 says
"The message that says a failure stands says this of a change that is not committed"; one
of four forms does. This is the answer to HR8-L-4, and it is partial.

(b) `BY_MARK` is said when what is remembered is a mark (`hook.rs` 1541 to 1547). When what
is remembered is up to 256 single lines and the failure has grown past 256, what is
remembered has no mark, the new failure is one mark, and the block is a plain "FAIL".
`k1.py`, `out/k1.txt`: 257 failing files block; one repaired (256) blocks with the
sentence; one put back and one new (258) blocks with no sentence and no "New at this
stop", although 256 of the 258 lines were said at the stop before. README 533 says the
block comes "with a sentence that says the block is not all new".

Cheap: say `STANDS_BY_LINE` once in the detail whenever a working tree's failure stands,
whatever the head; say `BY_MARK` when the unit's failure is a mark and anything is
remembered under its tag.

```bash
W=/tmp/claude-ddf-hooks-review-9; python3 -I $W/h1.py; python3 -I $W/k1.py
```

### VD4R17-L-1 (Vesper, round 17): the project's main tree is judged before the linked tree the stop is in

`judge_units` (`hook_run.rs` 3527 to 3541) puts `place.top` first and the tree of `cwd`
second, and reads both with Git before either is judged. `o1.py`, `out/o1.txt`: a key in
the main tree and one in a linked tree, the stop made from the linked tree, the wrapper's
log: `TK-main` at +0.197 s, `TK-linked` at +0.258 s. README 664 and the commit say "the
working tree the stop is in" comes first. I corroborate the order only; the consequence
(the stop's own tree UNJUDGED when the main tree's run is slow) is Vesper's reproduction,
not mine.

## The areas of the brief: what was tried and whether it held

0. **This round's rework.**
   - THE MEMORY. `m1.py` (`out/m1.txt`): nine linked trees of 256 failing files; trees 1
     and 2 become marks at the ninth stop (2,304 entries to 1,794); tree 1 revisited
     stands. A marked tree with one file renamed (the same count, another path): BLOCK with
     the "not what it was" sentence. A marked tree in which one path now fails another rule
     (the same number of lines): BLOCK with the sentence. A marked tree with one key
     replaced by another in the same file: stands (the stated identity). A marked change
     staged whole (`git add -A`), then one harmless edit, then unstaged: stands each time;
     committed: blocks as the commit (documented). From the project: "8 things … 3 named
     (a failure stands there), and 5 more (among them 5 working trees where a failure
     stands that has blocked)", which is right. A native mark (257 files) with one file
     repaired: BLOCK with the sentence (`k1.py`). The corner stated as UNJUDGED (`c1.py`,
     `out/c1.txt`): the tree's patch gets no verdict, its memory is a mark, the index holds
     one of its files: both units UNJUDGED with the stated reason at every such stop, never
     silent; when the judge answers again the staged file blocks (HR9-M-1). The order of
     shortening against the tree the session is in: by reading, a looked-at unit is
     shortened only when every other is short, and the trees a stop looks at (at most two,
     at most 512 entries each) cannot reach that with 256 working trees or fewer. Broken:
     HR9-M-1, HR9-L-1.
     Past 2,048 failing CHANGES: **not reached.** A tree has two changes, so it needs more
     than 1,024 linked trees (about 10,000 inodes on a `/tmp` that had 5,000 free). I read
     the code instead (`within_bound` 3704 to 3723, `memory_notes`, `stop` 4365 to 4372):
     the dropped units are named in `Kept::lost`, the sentence is made at that stop and is
     appended to whatever the stop says, a silent stop included. One thing to know about
     that sentence and the one for a looked-at change: neither can be reached in a
     repository of 256 working trees or fewer, and with more than 256 the hook refuses to
     list the session's commits at all (`WORKTREES_CAP`, `judge_patch.rs` 1060 and 1124:
     "the repository has more working trees than a hook reads the history of"), which
     README does not state. So both sentences describe states that only a repository the
     hooks already call UNJUDGED at every stop can be in. They are held by unit tests only,
     as the request says.
   - THE TREE INSTANCE. On this host every file system has a birth time (`stat -c %w`:
     tmpfs, btrfs, ext4 on `/var/mnt/storage`, `/boot` and `/var/mnt/ssd-data`, ntfs3), so
     the fallback was judged by reading only. tmpfs does not reuse inode numbers
     (`t20.sh`: 0 of 20,000 `mkdir`/`rmdir`; six `worktree add`/`remove` rounds gave six
     numbers), and its birth times are about a millisecond apart, so "remove and add within
     the clock's grain with a reused inode" cannot be made here. Held, with the real judge
     (`out/r8-t1.txt`, `out/r8-t2.txt`, `out/r8-t3.txt`, `out/b1-*.txt`): `worktree move`;
     `mv` and `repair`; `mv` with no repair; remove and add at the same path with the same
     text (blocks, then stands); `lock`; `gc` and `prune`; `commondir`, `gitdir` and `HEAD`
     touched (stands: VD4R16-L-1 is closed here); the tree's directory put back from
     `cp -a`; the whole project put back from a copy (UNJUDGED at every stop, with the
     remedy); relative paths by flag and by configuration, and `repair --relative-paths`
     over a repository with one absolute and one relative tree (stands in each tree, "a
     failure stands there" from the project: HR8-M-1 is closed); a `gitdir` file with
     trailing spaces (Git itself calls that tree prunable; the hook still knows it and says
     that a failure stands there); CRLF in `gitdir` and in the tree's `.git`; the tree's
     directory replaced by a symbolic link to where it was moved, `cwd` by the link and by
     the real path; the project a linked tree; a bare repository with two trees. A tree
     whose name is not UTF-8, reached through a link with a UTF-8 name: UNJUDGED at every
     stop with its reason (said; and the same stop also says "the session's directory is
     not in it", which is not true there). The administrative directory itself put back
     from `cp -a` (`b1.py cpadmin`): a new inode and birth time, so a new tree and one more
     block, which is the conservative direction; README speaks of a restore only for file
     systems with no birth time. Not tried: a tree on another file system (by reading, the
     identity is read under the repository's `worktrees`, so it does not matter), a file
     system with no birth time, a restore that keeps birth times (Linux has no call that
     sets one).
     One statement to complete. The code comment over `instance_words` says that with no
     birth time, whole-second times and reused inode numbers "a tree removed and made again
     within one second can be taken for the old one". That is the direction the brief calls
     high (a new failure that never blocks), on ext2, ext3 and ext4 with 128-byte inodes,
     which have whole-second times and reuse the lowest free inode. README 518 to 526 names
     those file systems and states only the harmless direction (a `touch` or a restore
     makes the tree new). I could not test it; it should be said where README names them.
   - THE ORDER AND THE TIME. The edges at 7.9, 3.1, 2.9 and 8.2 seconds: the table under
     HR9-L-2. Longest stops (`e1.py`, `out/e1-*.txt`): every Git call hangs, 45.01 s and
     UNJUDGED with the reason; `git status` hangs in every other tree and the judge hangs
     on the tree's patch, 31.06 s; the judge hangs, 25.07 s; `git worktree list` hangs,
     41.00 s, a BLOCK for the tree's own new line, the commits UNJUDGED with the reason,
     and "the other working trees … could not be listed". No stop over 45.01 s. A failure
     that stands in trees the six seconds did not reach (`e2.py`, `out/e2.txt`): "3 working
     trees … were not reached … Failures that have blocked a stop of this session stand in
     2 of them", at the next stop on the counting line, 6.2 s, and both stand on return.
     The eighth review's sixteen slow trees (`t8.sh x 2.5`): BLOCK in 6.2 s with "15
     working trees … were not reached" (HR8-L-2 is closed). The stated choice, a turn
     written with no attempt (`u1.py`): three commits whose patch Git cannot make are
     written `n0@N` at every stop, named at every stop, cost 0.2 s together, and a new
     commit and seven others are asked about before the second and third of them; no
     starvation and nothing hidden, by this run and by reading. The eighth review's turn
     counts (`t5.py`, `out/r8-t5-mix.txt`, `out/r8-t5-down.txt`): 0 violations in 30 and 16
     stops. Broken: HR9-L-2 at its edge; the order of the two judged trees (VD4R17-L-1).
   - THE MESSAGE. The counting lines were right wherever I counted them (`m1.py`, `e2.py`,
     `b1.py mixed`). The eighth review's crowded stop on this build (`t15b.sh`,
     `out/r8-t15b.txt`): 7,020 characters, 26 commits named and "and 44 more commits fail",
     no cut mark. No message over 9,000 characters seen. A tree whose name holds a quote,
     line breaks, an instruction and a bidi control (`b1.py inject`): escaped in the unit's
     name and on the counting line (`\"\x0a…\x0a\u{202e}`); the name is cut at 140
     characters there, which loses its closing quote. The two memory sentences: not seen
     live (see above). Missing where they belong: HR9-L-3.
   - WHAT HELD LAST TIME, on this build (`run-r8.sh`, `out/r8-*.txt`): the identity
     transitions; line identity through rename, counts, two rules on one path, staged
     against uncommitted against committed (`t4.sh`, `t7.sh`); the lock with 8 and 24 first
     hooks at once and a stop killed while its judge sleeps (`t6.sh par`, `t6k.sh`, the
     process id from `$!`); the round 14 table (`r6/t6.sh`, `out/r8-r6.txt`): silent in
     exactly the all-pass rows. I did not run the eighth review's `t6.sh kill`: it picks a
     process by name.
   - STATED LIMITS. A key replaced by another in the same file: stated in "A failure
     blocks once" (README 494 to 503) and in the FAIL STANDS head, accurately; not in the
     other forms of the message (HR9-L-3). A branch moved back onto an old failing commit:
     stated in the start text and in every range sentence, accurately; I agree with the
     eighth review that it should stay a stated limit. The stash round trip
     (`w1.py`, `out/w1.txt`): BLOCK as the uncommitted change, BLOCK as the stash's commit,
     BLOCK when it is back, then FAIL STANDS: three times, as README 545 says.
   - WHICH COMMITS ARE THE SESSION'S (`w1.py`): amend, revert, rebase, squash merge with
     the branch deleted, fixup with autosquash, cherry-pick with the source deleted, a true
     merge, `reset --soft`, `reset --hard`, a detached `HEAD` left again, a commit kept only
     by `update-ref` with the reflog expired, a commit kept only by a tag, a commit made in
     a linked tree and judged from the project, a commit fetched during the session and
     then merged: each BLOCK, then FAIL STANDS, none silent. A commit made on a detached
     `HEAD` in a linked tree that is then removed: silent, and it is the stated limit (no
     ref, no `HEAD` and no reflog leads to it any more).
   - STATE FILES (`st1.py`, `out/st1.txt`): a state of schema v7: the stop is UNJUDGED and
     the advice and start hooks say so, nothing is recorded over it; a judged file cut in
     half or missing and a tips file emptied: UNJUDGED with the remedy; a judged file with
     a line added: read up to its count; 2,049 remembered lines planted: refused; `resume`,
     `compact` and `clear` starts keep the base; a second session in the same checkout has
     its own memory and says that its start was not recorded. Held.
1. **A pass that should not be one.** Not gone through again, apart from what the cases
   pass through.
2. **A block that cannot be escaped, or one that is waved through.** HR9-M-1 and HR9-L-1
   are blocks that come again; both end. Nothing waved through.
3. **The hook input and output contract** (`i1.sh`): empty input, not JSON, no session id,
   a `cwd` that does not exist, another event's name, 33 MiB of input, an unknown flag:
   each is UNJUDGED or "did not run" with its reason, exit 0, valid JSON. A first hook
   that is a stop with `stop_hook_active` true and a key in the tree: BLOCK. Held.
4. **Resources and files.** Time: above. `/tmp` out of inodes while my hooks ran: not
   examined (the first stops of the lost `m1.py` run answered "allow" with a message I did
   not keep).
5. **`agent setup --hooks`, `agent doctor --gate`.** Run once each in a scratch clone;
   read only. Not tested.
6. **`schema signals`, `explain signal`.** Not looked at.
7. **The tests.** By reading; I ran no test and no control (no `cargo`).
   - `an_unchanged_failure_blocks_twice_only_where_a_stop_said_so` (`hook_run.rs` 6307):
     its index is only ever a copy of the tree, empty, or the tree before one repair, so a
     part of an unchanged failure is never staged (HR9-M-1 passes it). It calls a place
     unchanged only when tree and index are both as the last stop saw them, so "the tree
     is repaired again and the index is as it was" after a shortening asserts nothing
     (HR9-L-1 passes it). It holds less than claim 3 and README 512 to 513 say.
   - No test in `agent_hook.rs` stages part of a failure of more than 256 lines.
   - The two memory sentences and the fallback with no birth time are held by unit tests
     only (stated in the request); the sentences cannot be reached by an integration test
     in a repository the hooks list the commits of.
   - Nothing holds the wording of an ask that was made and answered with under three
     seconds left (HR9-L-2), or the two sentences of HR9-L-3 under other heads.

## Suspected only (not reproduced)

- On a file system with no birth time, whole-second times and reused inode numbers, a tree
  removed and made again within one second is the old tree to the hook, and the same
  failing line in it never blocks (from the code's own comment; no such file system here).
- A stop whose commits are always reached with three to eight seconds left asks about the
  same commit at every stop (HR9-L-2 and the stated precondition together).
- The consequence of VD4R17-L-1 (I corroborated the order only).

## What I did not look at

Areas 1, 5 and 6 of the brief beyond what is said above; `verdict_v2.rs`; `judge_patch.rs`
beyond the change to `nested` (attributes, filters, sparse checkout, submodules, CRLF);
more than 2,048 failing changes and more than 256 working trees, live; a file system with
no birth time, with reused inode numbers, or with no `flock`; a tree on another file
system; a full disk or a state directory that cannot be written; two stops at once
(`verify7.sh` is the committer's); the eighth review's slow turn driver (`t5.py slow`) and
its `t11.sh`; `CLAUDE_PROJECT_DIR` set wrongly; what Claude Code itself does with
`stop_hook_active`, a message over 10,000 characters or two hooks at once (taken from the
branch's own statements); a random driver over sessions.
