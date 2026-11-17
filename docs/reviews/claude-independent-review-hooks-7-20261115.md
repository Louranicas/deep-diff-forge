# Seventh independent review of the agent hooks branch (`agent/loop`, commit `065c74f`)

Date: 2026-11-15. Reviewer: an independent Claude agent that took no part in the work.
Brief: `~/.local/state/habitat-claude/ddf-gap-probes-20261113/ddf-hooks-independent-review-7-brief.md`.
Under review: `d4dbc91..065c74f`, mainly the rework `c6c4f28..065c74f` ("nothing is given up,
and a failure is forgotten only by a stop that looked").

## Verdict: NO-GO as committed, on one finding with a small fix

By the rule the brief sets ("a new failure in an ordinary workflow that never blocks" is
high), one finding is high, and it is new with this commit:

- **HR7-H-1.** A failure in a linked working tree is remembered by the tree's *path* and is
  forgotten only when a stop finds a tree at that path without it. A tree that is removed
  is never "found without it". When a working tree is made again at the same path (the
  agent throws its worktree away and starts over), a new failure there with the same rule,
  signal, path and count is taken for the old one: every stop says FAIL STANDS, "has
  already blocked a stop of this session", and it never blocks. The build before
  (`c6c4f28`) blocks in the same sequence.

It is not a silent stop (the user sees FAIL STANDS at every stop), it needs a tree made
again at the same path, and the fix is a few lines. Without it I would have said
GO-WITH-FIXES: I did not find a stop that prints nothing while something is open, a commit
that is never asked about again, or a block that repeats with nothing changed. One medium
and six lows follow.

- **HR7-M-1** (older than this commit): when one of two failing things is repaired, the
  other one, unchanged and already told to a person, blocks again. Claim 3 and the block's
  own text say it does not.
- **HR7-L-1**: at a crowded stop the line "and 42 more commits fail." is cut away: 43
  commits that block are neither named nor counted, in a message of 7,975 characters.
- **HR7-L-2** to **HR7-L-6**: below.

Every finding is marked ORDINARY WORKFLOW or DELIBERATE EVASION. None of this round's
findings is a deliberate evasion.

## How this was done

- Source read from `git archive 065c74f` (extracted to `/tmp/claude-ddf-hooks-review-7/src`).
  The worktree was at `065c74f` with one untracked file,
  `docs/reviews/vesper-v4-round15-report-20261115.md`, which I did **not** read: what
  overlaps with it was found independently. No `cargo` was run, nothing was built. No
  `toolbox`, `podman` or `distrobox` call was made. Nothing at GitHub, no network.
- Binary: `/var/home/louranicas/.cache/ddf-t/review-bin/v4r15/release/deep-diff-forge`
  (SHA-256 `8359334d…4af1eb`); for "is it new" the build before, `…/v4r14/release/…`.
  Judge: the real forge-warden 0.2.0, `/var/home/louranicas/.cache/ddf-t/target-fw/release/forge-warden`
  (SHA-256 `55bf9fe6…`), run on the host. Git 2.56.0. Load average 27 to 38 throughout.
- Scratch: `/tmp/claude-ddf-hooks-review-7/`. `base-repo` is a `git clone --no-local` of the
  sixth review's base repository with the ranker digest set to this binary and committed
  (`4aaf86d`); `base-long` the same with two long rule ids. Every case is a fresh clone
  under `cases/`. Helper `h.sh` (the sixth review's, pointed at this build), `lib.sh`,
  scripts `t0.sh` … `t16.sh`, outputs in `out/`, notes in `NOTES.md`. The sixth review's
  scripts were copied to `r6/` with the paths changed and run again on this build.
- Every run had a private `HOME` set inside the helper or the script
  (`/tmp/claude-ddf-hooks-review-7/home*`), `XDG_STATE_HOME` and `CLAUDE_PROJECT_DIR` unset,
  `GIT_CONFIG_GLOBAL=/dev/null`. The real `~/.local/state/deep-diff-forge` was not written.
- Where a verdict had to be missing, the judge is the sixth review's wrapper `tools/flaky`
  (exit 3 for a patch that holds a marker text, otherwise the real judge unchanged);
  `tools/slowjudge` sleeps and then runs the real judge; `tools/slowgit/git` is a Git that is
  slow once (used for one hook in HR7-L-4, to widen a race window). These runs say so.
- Random workflows, two drivers of my own, real judge:
  - `fuzz7.py` (the sixth review's 53 Git operations, ten stops a session, each stop made
    twice). Invariants: A (a key open and the stop silent), B (said with nothing open), and
    three that are new: **N** an expected-block oracle of my own for the project's tree
    (plain `git diff`: path → number of key lines; a block is expected for a keyed commit
    first seen, for a tree set that is other than the one remembered, for a staged set that
    is not within the tree's and other than the one remembered), **O** no block when the
    oracle expects none, **C1** the same stop made again never blocks.
    600 sessions, 6,000 stops and 6,000 repeated stops: **0 violations**.
  - `fuzz7b.py` (two working trees, the session's directory moves between them, keys come
    and go uncommitted and staged in both, a per-tree oracle that updates only for a tree
    that was looked at): 300 sessions, 3,000 stops and 3,000 repeated stops (630 of the
    first stops blocked): **0 violations**.
  - The oracle follows the documented identity (the whole set of rule, path and count), so
    by construction it does not flag HR7-M-1, and neither driver removes a working tree, so
    neither reaches HR7-H-1.
- What Claude Code does with `stop_hook_active`, where it cuts a message and whether two
  hooks of one session overlap is taken from the branch's own statements, not from a live
  session.

## Findings reproduced with the real judge

### HR7-H-1 (high by the brief's rule; ORDINARY WORKFLOW, uncommon; NEW with `065c74f`): a working tree made again at the same path inherits "has already blocked"

`remember` keeps a failure as `TREEID/KEY`, with `TREEID` sixteen digits of the SHA-256 of
the tree's path (`tree_id`), and forgets it in two ways only: a stop that read that tree and
did not find it, or a stop that found the tree in `git worktree list` with an empty
`git status` (`other_trees` → `clean`). A tree that is no longer listed is in neither set,
so its entry stays for the rest of the session. `KEY` holds the label "Uncommitted changes
in "PATH"", so a tree at the same path gives the same key.

`t2c.sh`, `out/t2c.txt` (nothing simulated):

```
-- turn 1: git worktree add ../wt -b fix1; a fixture with a key-like value is written there
   stop (cwd in the tree)                => BLOCK
   stop again                            => allow: FAIL STANDS
-- turn 2: the approach is given up: git worktree remove --force ../wt; git branch -D fix1
   stop (cwd in the project)             => SILENT
-- turn 3: git worktree add ../wt -b fix2 (the same path); ANOTHER key is written into a file of the same name
   stop (cwd in the new tree)            => allow: FAIL STANDS  "Uncommitted changes in "…-wt": FAIL, which stands"
   stop again                            => allow: FAIL STANDS
   blocked in the state: ['c1cd30a7c3e2309d/df226717…a08b:1']
-- control: the same turn 3 in a session that never had the first tree
   stop (cwd in the tree)                => BLOCK
```

The new tree is on another branch and holds another key; the message says "This failure
has already blocked a stop of this session … it is left for a person", which is false, and
the agent is never told. Variants, all reproduced:

- `t2.sh L1` (`out/t2.txt`): the same with `tests-travel-with-code` (a `src/` file changed
  with no test, changed differently in the second tree).
- `t2d.sh` (`out/t2d.txt`): the directory deleted and `git worktree prune`, then
  `git worktree add` at the same path and the same fixture generated again. README: "the
  same failure blocks again if it comes back". It stands.
- `t2.sh L1c`: the second tree at another path blocks (so it is the path, nothing else).
- `t1old.sh` (`out/t1old.txt`): the build before (`v4r14`), same sequence: the second tree
  **blocks**. The regression is this commit's memory by tree path.

```bash
W=/tmp/claude-ddf-hooks-review-7; . $W/h.sh; . $W/lib.sh
R=$(fresh h1x); SESSION=h1x; start $R >/dev/null; T=$R-wt
git -C $R worktree add -q -b fix1 $T; newsecret $T/lib/fixture.rs; s $T; s $T true   # BLOCK; stands
git -C $R worktree remove --force $T; git -C $R branch -q -D fix1; s $R               # silent
git -C $R worktree add -q -b fix2 $T
printf 'pub const LIVE: &str = "AKIAI44QH8DHBEXAMPLE";\n' > $T/lib/fixture.rs; s $T   # FAIL STANDS, no block
```

Why I rate it by the rule and not lower: nothing here sets out to evade; "throw the
worktree away and start over at the same path" on the same task makes the same paths fail
the same rules again, which is exactly when the identity matches. Why it is not worse: it
is said at every stop, and it needs the tree to be made again at the same path.

Cheap fix: at every stop, when the list of working trees can be read, drop the remembered
entries of every `TREEID` that is not the id of a tree `git worktree list` names now (the
list is read anyway; a tree beyond the sixteen is still named, and keeps its entries). Better: know a tree as
the project is known, by the inode of its Git directory with its path, so a tree made anew
is another tree. A test: the sequence above.

### HR7-M-1 (medium; ORDINARY WORKFLOW, and a common one; older than this commit, also in `c6c4f28`): what is left of a failure blocks again when another part of it is repaired

The identity of an uncommitted failure is one digest over every line (rule, signal, path,
count). Repairing one line gives another digest, so the lines that did not change block
again. `t1.sh A` and `B`, `out/t1.txt`:

```
== A
-- stop 1: two secrets (lib/x.rs, lib/y.rs)                      => BLOCK
-- stop 2: again                                                 => allow: FAIL STANDS
-- stop 3: y repaired, x left as told to the person              => BLOCK   (x.rs was not touched)
-- stop 4: again                                                 => allow: FAIL STANDS
== B
-- stop 1: secret + src change without test (two rules)          => BLOCK
-- stop 2: again                                                 => allow: FAIL STANDS
-- stop 3: test added; the secret (told to the person) as it was => BLOCK
-- stop 4                                                        => allow: FAIL STANDS
```

This is what a block asks for: "What the rules name in this session's own work is this
session's to repair; what they name in work it did not make is for a person: tell them …
Then stop again: a failure that has blocked does not block again while it stays as it is."
The agent repairs its own part, has told the person about the other, stops, and is blocked
for the part it was told to leave. The request's claim 3: "an unchanged failure never
blocks twice while the state can be kept, whatever is staged, edited beside it, or looked
at in between". README says only the other direction ("One more item under a rule that
already failed is a new failure and blocks").

It is bounded (one more block for each repair, not one every other turn as HR6-M-1 was),
and a second block that says what is left is defensible. But then the documents and the
block's text must say it. The other way is the one `failure_within` already takes for the
staged change: remember the lines, and block when a line is there that was not remembered.
The same mechanism in the staged unit: `t3.sh S5` (`out/t3.txt`): a staged key blocks as
"Staged changes" at stop 3; the tree gets the key back at stop 4 (blocks, as documented);
the tree is repaired again at stop 5 and "Staged changes" blocks a second time, although the
index was never touched: while the tree had the failure the staged one was "no unit of its
own" and was forgotten.

```bash
W=/tmp/claude-ddf-hooks-review-7; . $W/h.sh; . $W/lib.sh
R=$(fresh m1x); SESSION=m1x; start $R >/dev/null
newsecret $R/lib/x.rs; newsecret $R/lib/y.rs; s $R; s $R true    # BLOCK; stands
mv $R/lib/y.rs /tmp/claude-ddf-hooks-review-7/out/y.away; s $R   # BLOCK again, for x.rs alone
```

### HR7-L-1 (low; ORDINARY WORKFLOW piece by piece, crowded): the cut for the lines of every stop removes the count of what blocks; "the lines below count every unit" is not true

`trees_text` keeps `TAIL_ROOM` = 1,700 characters for the lines of every stop and gives
what blocks 5,700. The lines can be longer than 1,700: three names on the "stand" line and
one on the "held" line can be working trees' changes, each up to about 220 characters
("Uncommitted changes in "PATH" (rule names)"), beside three commits with no verdict at
150 characters of reason each, the staged hint and the rule-text line. Then `within` cuts
the end of the upper part, and the end of the upper part is the last named blockers and
the count.

`t15b.sh`, `out/t15b.txt`, `out/t15b-3.json` (real judge; the wrapper only for three
marked commits). A project and a linked tree with paths of 127 and 139 characters; an
uncommitted and a staged failure in the project and an uncommitted failure in the linked
tree, all standing; a staged hold in the linked tree; one failing and two held commits;
three commits with no verdict; then 70 fetched commits that fail two rules each, brought
under a local branch:

```
-- stop 3: the crowded stop
   BLOCK chars=7975 | cut mark (upper part): True | cut mark (whole): False
     …
     Commit 9e93e7773078: FAIL (no-secrets-or-credential-like-values-anywhere, tests-travel-with-code…)
     (Not all of the above fits in this message; the lines below count every unit.)
     3 commits of this session have no verdict: …            (543 chars)
     3 held by the judge, for a person: Staged changes in …  (354 chars)
     4 earlier failures stand: Uncommitted changes in …      (620 chars)
     What is staged fails for as long as the index holds it … (158 chars)
     Rule text comes from the policy at the base commit; …   (107 chars)
   commits named as failing now: 27 | counted in "more commits fail": []
```

70 commits block; 27 are named; the line "and 42 more commits fail." (it is there in
`t15.sh`, whose tail is 1,710 characters and whose upper part is 50 short of the cut; here the tail is 1,787) and one named commit are gone. The lines below
count commits with no verdict, holds and failures that *stand*; nothing counts what blocks
now. The message is 7,975 characters: the cut happens with a thousand characters of the
9,000 unused. A working tree's unit with no verdict stands in the same place (it is
appended after what blocks) and would go first (from reading; not produced). README: "the
rest are counted"; SECURITY.md: "A stop's message never loses what blocks".

Weak test: `a_message_is_under_its_bound_whatever_its_parts_add_up_to` says "every part at
its largest at once", but its four tree units are all UNJUDGED, so no tree label is on the
held or the stand line and the tail stays under 1,700.

Cheap fix: make the tail first and give the upper part `DETAIL_CAP - tail` (what
`blocking_text` gets as `room`), so its own count line is inside the room; or cap each name
on the tail's lines.

### HR7-L-2 (low; ORDINARY WORKFLOW in an unusual layout; same root as HR7-H-1): two working trees whose shown path is the same share one identity

`blocked_keys` strips the `TREEID` before a failure is compared, so the tree is told only by
the label inside the key, and the label is `shown_path`: cut to the first and the last 60
characters past 120. `t2.sh L4` (`out/t2.txt`): two linked trees
`…/client-aaaa/shared/…/project-name` and `…/client-bbbb/shared/…/project-name` (138
characters). A key in tree A blocks and stands. A new key in tree B:

```
-- stop 3 (in tree B: a new key in ANOTHER tree)   => allow: FAIL STANDS
   blocked: ['3a32096f9ca68686/4647bc68…c78e:1', '50f2ce25f93ee50a/4647bc68…c78e:1']
```

Two tree ids, one key: it never blocks. Fix with HR7-H-1: compare `TREEID/KEY` whole (the
unit knows its tree), and take the label out of the key.

### HR7-L-3 (low; ORDINARY WORKFLOW): that a failure stands in a tree the stop did not look at is said once, not at every stop

README: "a stop that did not look … forgets nothing, names the tree and says that a failure
stands there"; the rule of the commit: "A stop that did not look says that it stands";
the rule of round 14: what fails and stands is said "at that stop, every time". `t2b.sh`,
`out/t2b.txt`:

```
-- stop 3 (project; …)   [allow] deep-diff-forge: The working tree "…-wt" of this project has changes that are not
                          committed, … A failure there has blocked a stop of this session and stands: it was not looked at at this stop.
-- stop 3b (project again) [allow] deep-diff-forge: 1 thing in this project is not judged: the working tree "…-wt".
```

The sentence is part of a note's full text, and a note is in full once. From the second
such stop on, nothing says that a failure stands. The new test
`a_failure_in_a_working_tree_that_was_not_looked_at_stands` asserts exactly this (the
sentence at round 0, the counting line alone at rounds 1 and 2), so it pins the gap.
Cheap fix: put "(a failure stands there)" into the note's `name`, which is on the counting
line at every stop.

### HR7-L-4 (low; ORDINARY WORKFLOW only where two hooks of a session overlap; the stated limit is not accurate): a late write of an advice hook brings back a failure that a stop forgot

An advice hook reads the whole state, works, and writes the whole state. README states
that it and a stop "can still each write the state the other did not see: … so a commit
is then judged again, a failure blocks again, or an attempt is not counted. Nothing
becomes a pass by it". The other direction is not stated: the advice hook's older
`blocked` list comes back, so a failure that a stop saw repaired and forgot is "one that
has blocked" again, and does not block when it returns.

`t8.sh`, `out/t8.txt` (the advice hook alone runs with `tools/slowgit` first on `PATH`,
which sleeps two seconds once; everything else as it is):

```
== control (no overlap)   stop 3: key repaired => HOLD, blocked: []   stop 4: the key is back => BLOCK
== race                   stop 3: key repaired => HOLD, blocked: []
                          the advice hook has ended … blocked: ['4b213679b0..664e62:1']
                          stop 4: the key is back => allow: FAIL STANDS
```

Not a pass and not silent. Whether the two overlap in Claude Code I did not test (they can
when a tool call of a background task ends during a stop). Cheap fix: an advice hook
writes only what it owns (`reported`, `noticed`) into the state it reads again just
before writing, or takes the lock for that read and write.

### HR7-L-5 (low; ORDINARY WORKFLOW, Claude Code's own layout): a working tree inside the project's tree is "not judged" at every stop, also at the stop that judged it

`t9.sh N1`, `t9b.sh` (`out/t9.txt`, `out/t9b.txt`). A linked tree at
`.claude/worktrees/feat` inside the project (not ignored). With nothing changed anywhere,
every stop of the session says:

```
deep-diff-forge: 1 thing in this project is not judged: the working tree ".claude/worktrees/feat" inside it.
```

also at a stop made from inside that tree, where its change was judged and passed (or
failed, blocked and stands). From the project, the same tree is counted twice ("2 things
in this project are not judged: the working tree ".claude/worktrees/feat" inside it, the
working tree "/…/.claude/worktrees/feat""). The note comes from the project's patch
(`left_out`), which does not know what else the stop judged. An all-pass stop that is not
empty, with a sentence that is not true: it teaches the user to pass over the line that
elsewhere means something. Fix: no `left_out` note for a nested tree that this stop judged,
and none for one that `other_trees` reports.

### HR7-L-6 (low; ORDINARY WORKFLOW; what is left of HR6-M-2): a commit held or failed for something that is not a rule keeps no reason

The judged file keeps rule names (`h:rules`, `f:rules;held`). What the judge says beside
its rules (a note, an expired waiver) is not kept. `t11.sh M1`, `out/t11-M1.txt`: a commit
that changes a submodule entry is held for the note `patch_unverifiable` ("unsupported file
mode 160000 …"); its line is `ce4929bb4c1a:h:`. Stop 1 shows the note. Stops 2 and 3:

```
1 held by the judge, for a person: Commit ce4929bb4c1a.
```

For a commit that fails by an expired waiver the later stops are "Commit …: FAIL, which
stands" with nothing under it (from reading: `f:` with no name). Keep the note's code, or
"(a note of the judge)", with the answer.

### Smaller things

- `rule_names` cuts the names of a unit's rules at 70 characters with "…": on the short
  lines and on the lines of every stop a third rule, or a held rule after two long failing
  ones, is not named (seen in `out/t15b-3.json`: "…, tests-travel-with-code…"). README:
  "each unit these lines name comes with the names of its rules".
- `rule_ids` keeps eight failing and eight held names and 64 characters of each, with no
  word when it left one out.
- "1 more working trees that were not looked at" (`out/t9.txt`).
- Three blocks for one unchanged credential over `git stash -u`, a stop, `git stash pop`
  (`t3.sh S3`): the uncommitted change, the stash commit, and the change that "came back".
  Each step is documented; together they are worth one sentence in README.

## Suspected only (not reproduced)

- With HR7-L-1's cut, a working tree's change that got no verdict (it is appended after
  what blocks) is cut before any blocker, and nothing below counts it. From reading.
- A session's first stop and another stop of it at the same time take no lock (no state
  file yet). One try with different answers for the two (`t7.sh K3`) left the session
  whole. Not pursued.

## The areas of the brief: what was tried and whether it held

0. **This round's rework.**
   - FAILURE MEMORY, new taken for old. Broken: a tree removed and added again at the same
     path (HR7-H-1); two trees with one shown path (HR7-L-2). Held: the same rule and path
     with another secret and the same count stands (`t1.sh C`, stop 5), which is the
     stated identity; a count that changes blocks (`t1.sh C`); a staged failure wider than
     the tree's blocks as its own unit (`t3.sh S6`); another key staged and deleted from
     the tree beside a failure that stands blocks (`S7`); a second tree with the same
     relative path at another path blocks (`t2.sh L1c`); a renamed file blocks again
     (`t14.sh`, the path is in the identity); a new failure made after a block, with
     `stop_hook_active` true, blocks (`t16.sh`). A linked tree repaired and broken again
     between two stops that did not look: it stands, and the stop in between says so, once
     (HR7-L-3); with the tree clean in between it is forgotten and blocks on return (the
     new test, and `r6/t4b.sh`). I think the implementer's choice is right: it is the same
     as in the project's own tree between two stops.
   - FAILURE MEMORY, old taken for new. Held: `git add -A`, one more edit, `add -A`, edit,
     `restore --staged .`, `add` of the file, `add -N`: one block, then FAIL STANDS at seven
     stops (`t3.sh S1`); the sixth review's `t4.sh` and `t4b.sh` on this build (HR6-M-1 is
     closed); `cwd` leaving a linked tree and returning; eighteen linked trees (`t9.sh N2`:
     the two beyond sixteen are counted at every stop, and a failure in one of the trees
     stands over a stop from the project and back). Broken: HR7-M-1 (a part
     repaired), and its staged form (`S5`). As documented: commit of the failing change,
     `commit --amend`, `reset --soft`, stash and pop each block again (`S3`, `S4`).
   - NOTHING IS GIVEN UP. Held. Three commits with no verdict (`t5.sh V1`, wrapper): asked
     at every stop for five counted attempts, then one a stop in turn (`n6 n5 n5`,
     `n6 n6 n5`, `n6 n6 n6`, `n7 …`); when the judge recovers each is judged at its turn and
     **each blocks** (stops 12, 13, 14), then all stand. One such commit and a fresh commit
     at every stop (`V2`): the fresh ones pass, the old one counts on and blocks at the
     first stop after the judge recovers. A stop killed with `kill -9` while the judge ran
     (`t10.sh C1`): nothing written, the next stop judges and blocks, the scratch directory
     is cleared. An unwritable state (`t10.sh RO`): block, FAIL STANDS under
     `stop_hook_active`, block, each with the reason. Shallow edge: `r6/t5b.sh`, judged and
     blocks after `git fetch --unshallow`. What is final: only
     `GitAcquireError::BudgetExceeded` from making the commit's patch, a size over 64 MiB
     of `git diff` output, which a commit cannot change; a time-out is not that error. By
     reading: a commit whose ask ends with under three seconds of the stop left is not
     counted and stays first in line; I found no ordinary way to make one ask take that
     long with this judge (its ranker gives up at about ten seconds).
   - WHAT IS HELD IS NAMED. Held for rules: a change with a key and a silenced check, live
     (`t6.sh H1`): "(held, for a person)" at the block, at FAIL STANDS, after the commit and
     at both later stops; the line is `f:no-secrets;no-silent-suppression`. A held commit
     first judged with 40 failing ones (`t6.sh H2`, the sixth review's `t3b`): its rule is
     on the block and on the line of every later stop. Not held for notes: HR7-L-6. Rule
     names are `[a-z0-9-]` by `verdict_v2` and come from the policy at the base, so they
     cannot carry text; long and many: the smaller things above.
   - THE MESSAGE. No message over 9,000 characters: the sixth review's fullest case gives
     6,907 on this build (`out/r6-t3-b4.txt`), mine 7,975. A blocker lost by a cut:
     HR7-L-1. Text reaching the agent: paths are quoted, escaped and marked as data, rule
     names are constrained, the judge's words are escaped and marked as its own; nothing
     new found.
   - THE LOCK. Held: a second stop waits 3.0 s and says UNJUDGED "another stop of this
     session is running"; the first blocks; a third afterwards says FAIL STANDS
     (`t7.sh K2`). The judge's process does not inherit the lock's descriptor
     (`out/t7-k2-fds.txt`: none), so a stop that is killed lets go. The lock file left by
     every stop does not hold a later one. The advice hook: HR7-L-4.
   - THE TABLE. `r6/t6.sh` on this build (`out/r6-t6.txt`): 21 rows, three stops each:
     silent in exactly the all-pass rows, a fail blocks once and stands, a hold and a
     missing verdict at every stop. No block that repeats.
   - Which commits are the session's; index states: the two drivers above (rebase, squash,
     amend, cherry-pick, merges, fixup, resets, stash of four kinds, detached `HEAD`,
     `update-ref`, `commit-tree`, `add -N`, `rm --cached`, `mv`, commits in a linked tree):
     no key open at a silent stop in 900 sessions. The fourth review's 30 workflows
     after a commit with a key, on this build (`r4/t4.sh`, `out/r4-t4.txt`): a block in 25,
     a named working tree in one, and silence in the four that are stated or hold nothing
     (`stash drop`, an index entry taken back, `reflog expire` with `gc`, a detached tree
     removed). State files: a state of the build before
     (`t7.sh U5`) makes every hook say UNJUDGED with the remedy "remove the file", at the
     stop, at a session-start and after a tool call (the stated choice; it will matter the
     first time a released build changes the schema). SessionStart again as `resume`,
     `compact` and `clear` keeps the base, what blocked and what was judged (`t14.sh`).

1. **A pass that should not be one.** Not changed by this commit (`verdict_v2.rs` is
   untouched). Sampled on this build with the sixth review's `t11`: a judge inside the
   repository is not run (UNJUDGED); a real PASS replayed for another patch is refused
   (UNJUDGED). Nothing new tried.
2. **A block that cannot be escaped, or that is waved through.** None found. The same stop
   made again never blocked in 9,000 repeated stops. HR7-M-1 is one more block for
   each repair, not a trap.
3. **The hook contract** (`t13.sh`): empty input, not JSON, an empty object, no session id,
   another event's name, a relative and a missing `cwd`, a session id of 300,000
   characters, wrong types, an unknown option: exit 0, valid JSON, UNJUDGED with the reason,
   nothing on stderr.
4. **Resources and files.** State, list and lock files 0600 in a 0700 directory; the
   scratch directory under `.git/deep-diff-forge` is gone after every stop, and after a
   killed one at the next. Stops took about 0.3 to 1.1 s (10 s with 70 commits to judge) at
   load 30. Not tried: a full disk.
5. **`agent setup --hooks` and `agent doctor --gate`** (`t12.sh`): with `forge-warden` on
   `PATH`, setup prints the three hooks with absolute paths and time-outs 5, 10 and 60;
   registered as printed, the gate is ready (18 links, exit 0), the start says "stops will
   be judged", and the registered stop command blocks a key. (A note on the sixth review's
   `t10.sh`, which I ran again: it passes `--judge` to `agent setup`, which that command
   does not take, so its steps G1 and G3 register nothing, then and now; `t12.sh` does
   that part as a user would.)
6. **`schema signals`**: 21 labels, the 11 of `EvidenceSignal::label` and the 10 of
   `RiskSignal::label`. `explain signal` not looked at again.
7. **The tests.** For this round's guards the new integration tests would fail with the
   guard removed (the request says 14 of 84 fail on `c6c4f28`; I could not run them).
   Missing or weak: no test removes a working tree (HR7-H-1); none repairs a part of a
   failure (HR7-M-1); the message test never puts a tree's label on a line of every stop
   (HR7-L-1); the linked-tree test pins "said once" (HR7-L-3); no test has two hooks of a
   session write the state across each other.

## Are the stated limits stated accurately?

- "A failure that blocked is forgotten only by a stop that looked at its unit and did not
  find it": true as far as it goes, and too narrow: a unit that no longer exists is never
  looked at (HR7-H-1).
- "An unchanged failure never blocks twice": not true for a part of a failure
  (HR7-M-1); the stash round trip and the staged form are documented piece by piece.
- "A stop that did not look says that it stands": once (HR7-L-3).
- The advice-and-stop limit: one direction is missing (HR7-L-4).
- "every one of the units that block … the rest are counted": HR7-L-1.
- The slow lane, the shallow edge, the lock, the probe, "what is still final": accurate as
  far as I could test.

## What I did not look at

- Vesper's round 15 report (in the worktree, untracked): not read, on purpose.
- A live Claude Code session; macOS; Git other than 2.56; SHA-256 repositories; partial
  clones; Git LFS; a full disk; network file systems; two PID namespaces sharing one
  repository (a hook in a container beside one on the host: `clear_stale` goes by `/proc`).
- Areas 1 and 6 beyond the samples named; `explain signal`; the advice hook's own text.
- `git worktree move` and a project that is itself a linked tree (read, not run).
- The unit and integration tests were read, not run: no `cargo`.
- Housekeeping, for the record: my scratch holds about 1,300 case directories under
  `/tmp/claude-ddf-hooks-review-7/` (`cases/`, `fuzz/`); nothing was removed. Session ids
  of the copied sixth-review scripts are fixed names, so two of my hostile-input runs met
  a session recorded by an earlier script; those two lines of `out/t13.txt` ("judge
  missing file", "judge in repo") are not evidence of anything.
