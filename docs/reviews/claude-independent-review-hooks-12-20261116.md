# Twelfth independent review of the agent hooks branch (`agent/loop`, commit `4b134bd`)

Date: 2026-11-16, 09:10 to 09:29 AEDT by the host's clock (`date`). Reviewer: an independent
Claude agent that took no part in the work.
Brief: `~/.local/state/habitat-claude/ddf-gap-probes-20261113/ddf-hooks-independent-review-12-brief.md`
(extends the eleventh brief, read whole). Under review: `d4dbc91..4b134bd`, mainly the rework
`07df120..4b134bd`.

## Verdict: GO-WITH-FIXES (none of the fixes blocks a merge on its own)

Every finding of the eleventh review is closed on the frozen binary with the real judge
(HR11-M-1, HR11-L-1 to L-6; below). In what this round reworked I found no stop that prints
nothing while something it knows of is open, nothing forgotten that the judge's own reading
would still see, and no unchanged failure that blocks twice. The eleventh review's two
matrices (`nr`, 84 cells; `mi`, 60 cells) give the same decision at every step as on
`v4r19` (144 of 144 files).

I found five lows; no high and no medium.

- **HR12-L-1** (low; ORDINARY WORKFLOW with a rare feature; older, `v4r19` the same). A
  linked tree whose only change is one `git status` does not show (a file marked
  assume-unchanged) and in which nothing has blocked yet is never named by a stop made
  from another tree: that stop prints nothing. The new README paragraph describes only the
  tree "where something is remembered", so a reader can infer this but it is not said.
- **HR12-L-2** (low; ORDINARY WORKFLOW in a large repository; new in this commit). Where
  something is remembered of another tree and `git status` there shows nothing, the tree is
  now read with the judge's reader, which hashes the whole tree again (a scratch index
  from `HEAD`, `git add --all`: no stat cache) within the look share (at most 6 seconds).
  Where that takes longer, a tree whose failure was REPAIRED is named at every stop from
  elsewhere with "could not be looked at … A failure there has blocked a stop of this
  session and stands", and each such stop takes about 6 seconds more. `07df120` forgot it
  at once. Conservative (never silent, nothing forgotten), but a false "stands" and a cost
  at every stop.
- **HR12-L-3** (low; the sentence of E is new, the behaviour older). README 741 and
  CHANGELOG 649 say "a reason a message cuts … ends with '…'". The line that counts the
  commits with no verdict cuts each reason at 150 characters with no mark ("… says so at
  lengt).").
- **HR12-L-4** (low; DELIBERATE EVASION or an odd state, not an ordinary workflow; older).
  A session directory under a directory that cannot be searched (mode 000) makes the stop
  UNJUDGED with the operating system's words, and the tree it is in is named "the
  session's directory is not in it": the sentence SECURITY now says is never said of that
  tree ("the tree is never named as one the session is not in").
- **HR12-L-5** (low; tests, not the product). The kill of `NAME_MAX + 2` → `NAME_MAX - 2`
  in `TAIL_LINE` holds only by the assertion that pins the constant to its own formula.
  By behaviour that mutant is equivalent: the longest counting line a stop makes has 781
  characters, and the mutated `TAIL_LINE` is 794 (the original 806). It should be counted
  with the equivalents (7, not 6), and the "70 of 70 killed" read as 69 by behaviour.

What I would do before a merge: one sentence each for HR12-L-1 and HR12-L-2 (README,
"Such a stop looks at that tree with `git status` …"), and the "…" for the no-verdict
line or a narrower sentence (HR12-L-3). HR12-L-4 is one line (`unfound` from the absolute
`cwd` as given when it cannot be resolved). HR12-L-5 is accounting.

## How this was done

- Source read from `git archive HEAD` (`/var/home/louranicas/.cache/ddf-t/hooks-review-12/src`);
  `git diff 07df120..HEAD` saved in `…/hooks-review-12/notes/hook_run.diff`, `hook.diff`.
  No `cargo`, no build, no `toolbox`, `podman` or `distrobox`, nothing at GitHub, no
  network. The only file written in the worktree is this report.
- Binary: `/var/home/louranicas/.cache/ddf-t/review-bin/v4r20/release/deep-diff-forge`
  (SHA-256 `193674a9…2ecd`, checked; the debug build `804fb80a…` checked, not run). For "is
  it new": `…/v4r19/release/…` (`09bbaeaf…`) on a base repository reset to the commit that
  admits it (`d12/base-r19`, `ac20f76`). Judge: the real forge-warden 0.2.0
  (`…/target-fw/release/forge-warden`, SHA-256 `55bf9fe6…`), on the host. Git 2.56.1.
- Scripts and outputs: `/var/home/louranicas/.cache/ddf-t/hooks-review-12/notes/s/`
  (the eleventh review's scripts copied from `hooks-round20/live/rv11` with their paths
  changed; new: `w12a.py`, `w12b.py`, `cut.py`, `mcmp.py`, `run-m.sh`, `tools/hangin`,
  `tools/lj`); outputs in `notes/s/out/` and `notes/s/out/m/`; notes with times in
  `notes/NOTES.md`. Repositories and case trees: `…/hooks-review-12/d12/` (`base-repo`
  admits this binary, `ea6c0be`), the size measurement in `…/hooks-review-12/big/`.
- At most two measuring processes at a time; Tctl stayed between 44 and 77 °C, Composite
  under 45 °C; no wait was needed. I stopped one run of my own (`run-m.sh`: its wait
  function overwrote a loop variable, so cells overwrote each other); its outputs were
  deleted and the run made again. I stopped no other process.

## Section 0, item 1: the eleventh review's reproductions on this build

All with the real judge (outputs in `notes/s/out/`):

- **HR11-M-1 closed.** `el2.py` (`el2.txt`): step 2 (the bare-inside tree pruned while
  away, keys in it) is now `UNJUDGED` with Git's words ("gitfile does not point to a valid
  repositor…"), as the ordinary layout is. `el.py barefail` (`el-barefail.txt`): UNJUDGED,
  and the tree is named "The session's directory is under the working tree … and whether
  it is in that tree could not be found out at this stop"; the next stop, with Git
  working, BLOCKS on the new key.
- **HR11-L-1 closed.** `ak.py 0 17`, `6 17`, `1 20`: BLOCK after 17.4, 17.4 and 20.4 s; no
  run was cut.
- **HR11-L-2 closed.** `au.py` (`au.txt`): step 3 names the tree ("1 thing in this project
  is not judged: the working tree …-L (a failure stands there)"); step 4 is FAIL STANDS.
- **HR11-L-3 closed.** `wt2.py 256` (`wt2-256.txt`, 257 trees): "not judged: not in this
  session, whatever is committed (what is not committed is judged): the commits that were
  there when it began could not be listed (…)." whole at every stop.
- **HR11-L-4 closed.** `mi7.py 600` (`mi7-600.txt`): step 7 no longer has the "given up"
  sentence; step 5 still has it; step 10 stands.
- **HR11-L-5 closed.** `gone.py` (`gone.txt`): the deleted `build/out` in the linked tree:
  the linked tree is judged (BLOCK), then stands.
- **HR11-L-6 closed** in the places reproduced: `ot.py 31 clean`: "this run had 30.0
  seconds, the most one run of the judge has"; README 735 and 632 are reworded; the
  "all that was left" branch is in the code (`hook_run.rs` 1933 to 1940), not run live.

## Section 0, item 2: WHICH TREE (`among_gits_own_files`, `standing_directory`)

`w12a.py` (outputs `w12a-*.txt`) and the eleventh review's `el.py bare|ingit|env|sub`
(`el-*.txt`, the same decisions as in that review). The judge's calls are logged.

| case | result |
|---|---|
| linked tree inside the project's `.git` (`.git/wt-in`); Git fails for it at one stop, a second key written meanwhile | UNJUDGED with Git's words; the tree named "(the session's directory may be in it)" with "a failure … stands"; next stop BLOCKS on the new key, the first stands |
| a linked tree nested inside another; Git fails for the inner one | UNJUDGED; both trees named "may be in it" (true: both contain the directory), nothing forgotten; then each stands |
| bare-inside, `cwd` = `repo.git/worktrees/feature` with a Git that fails there | the project (`main`) judged and blocks; `feature` named as a tree the session is not in (true) |
| `.git/modules/sub`, `…/hooks`, `…/refs` of a submodule | Git answers there: the submodule named as another repository; the project judged |
| `GIT_DIR`, `GIT_WORK_TREE` and six more in the hook's environment | no effect (also on the new reading of another tree: `w12b.py gitdir`) |
| gone `cwd` through a symbolic link to a linked tree | that tree judged (BLOCK) |
| the gone directory comes back | judged as usual (BLOCK on a new key) |
| gone `cwd` whose nearest existing directory is a nested repository inside a linked tree | the nested repository named; the linked tree named as one the session is not in (as for a submodule; older and stated) |
| the whole linked tree deleted while the session was in it | the tree named "(gone)" with its failure standing |
| a relative `cwd` | not walked up; the project judged; never silent |
| gone `cwd` inside `.git/wt-in`, inside a bare-inside tree, inside an administrative directory | the tree it was in judged first; the administrative directory as Git's own |
| `cwd` under a directory of mode 000 | UNJUDGED, but the tree is named "not in it": HR12-L-4 |

Nothing that is a tree was taken for Git's own, and nothing of Git's own for a tree; no
stop was silent while something it knew of was open.

## Section 0, item 3: ANOTHER TREE, read with the judge's reader

`w12b.py` (outputs `w12b-*.txt`): a failure blocks in the linked tree L, a stop is made from
the project's tree R, then from L again.

| what hides it, or what fails | from R | back in L |
|---|---|---|
| assume-unchanged (`au.py`) | named, stands | FAIL STANDS |
| another path marked skip-worktree | "could not be looked at" with the skip-worktree reason, stands | UNJUDGED (stated), then FAIL STANDS once removed |
| a clean filter (this machine's `sed` filter is seen by `git status`) | named | FAIL STANDS |
| Git fails for `add`/`write-tree` in L only | "could not be looked at (… `git add` failed …)" | FAIL STANDS |
| the key's file made unreadable | "could not be looked at" | FAIL STANDS |
| the key removed (repaired) | SILENT, forgotten | the same key again BLOCKS (it was forgotten after a reading that found nothing) |
| `GIT_DIR`/`GIT_WORK_TREE` of another repository in the hook's environment | named ("`git status` there shows none of them; the patch a stop judges by has them") | FAIL STANDS |
| Git in L sleeps 60 s on `add` | after 6.2 s, "could not be looked at (`git add` did not finish …)" | FAIL STANDS |
| the key REPAIRED and Git in L slow on `add` | 6.2 s and "could not be looked at … stands" at every stop: HR12-L-2 | — |
| L never stopped in, key in an assume-unchanged file | SILENT: HR12-L-1 | BLOCK |

Nothing was forgotten that the judge's reading would still see. Kept although gone: only
HR12-L-2. "Could not be looked at" against "nothing there": each named as what it was.
`core.autocrlf` and `core.fileMode=false` I did not run (they go through the same reader;
by reading, a tree with CRLF checkouts is "Found" and named with "stands" while something
is remembered of it: conservative).

## Section 0, item 4: the tests of F and the kills of G (by reading; no test run)

- F. `late_stop` waits for a wall-clock moment counted from the stop's first call of Git
  and measures `left_at_commits` (an upper bound) and `left_at_end` (a lower bound).
  `an_ask_is_written_down_when_it_was_made_and_only_then` checks
  `written_is_what_was_asked` at every stop, also those made again, and asserts the turn
  moves (`b` then `a`); a stop with no ask passes only if it ended with under 3.05 s of its
  judging, so a build that refused an ask with 3.5 s left fails it. The second test's
  first loop asserts nothing asked and nothing written with under 1.85 s; its second loop
  fails a build that asks only one with over 7 s left. Both still hold round 19's claims.
  Their retry budgets (4 and 3+4) keep a residual load sensitivity, as the request says.
- G. Read: `held_for_the_project`, `lines_taken`, `time_is_held_back_only_for_commits_that_still_wait`,
  `what_a_commit_keeps_of_its_reason…`, `notes_and_names_take_no_more_than_their_room`,
  `a_staged_change_is_remembered_as_a_whole…`, `what_blocks_gets_the_room…`: each asserts
  behaviour that the named mutants change, except the `TAIL_LINE` pin (HR12-L-5). The other
  `TAIL_LINE` mutants (`- 80`, `*` → `+`) also fail the behavioural check (646 and 325 are
  under 781). The six equivalents: `O_NOFOLLOW | O_NONBLOCK` with `^` (distinct single
  bits: equal); the seal helper's stderr cap (`run_bounded` truncates stderr and nothing
  reads it: equal); `staged_memory` `&&` → `||` (own and kept are disjoint subsets of the
  index's keys, so the `all` over the index implies the length test; with an empty index
  `whole` needs a marked own change, which an empty index cannot have: equal, by reading);
  `Bench::judge` `<` → `<=` against `COMMIT_FLOOR` (differs at exactly 3 s to the
  nanosecond, in wording only: equal in practice). I agree with all six.
- The new integration tests for A, B and D would fail without their guards: the A test
  asserts UNJUDGED and "may be in it" for a failing Git in `other/`, and "not could not be
  found out" for `objects` and `worktrees/other` with the same failing Git; the D test
  asserts the linked tree's patch is judged from a deleted `build/out`; the B test has the
  four rows of its table. None of them has a tree in which nothing is remembered
  (HR12-L-1) or a read slower than its share (HR12-L-2).

## Section 0, item 5: the sentences of E

True where said, in every case I ran: "(this run had 30.0 seconds, the most one run of the
judge has)"; the reason of a session with more than 256 trees, consequence first and whole;
the "given up" sentence at `mi7` step 5 and not step 7; "…" at the end of a working tree's
reason cut at 300 (`el2.txt`: "valid repositor…"). Not true everywhere it is claimed:
HR12-L-3. Missing where it should be: HR12-L-1 and HR12-L-2 are not stated in the README
paragraph that describes the reading of another tree.

## Section 0, item 6: the eleventh brief's sections 1 to 7, briefly

- Matrices: `nr.py` 12 modes by 7 changes and `mi.py` sizes 1, 200, 256, 257, 600 by
  two halves by six changes: 144 of 144 files with the same decision at every step as the
  eleventh review's outputs on `v4r19` (`mcmp.py`; outputs `out/m/`).
- 1 (`a1.py`), 3 (`a3.py`), 4 (`par.py`), `cp.py`: the same decisions as in the eleventh
  review. The counting line with long names and control characters (`cn.py`, `cn2.py`):
  622 and 639 characters, as before; a tree with a newline in its path is still UNJUDGED
  from inside (older, noted by the eleventh review).
- 2, 5, 6, 7 beyond what is said above: not gone through again.

## Findings reproduced with the real judge

### HR12-L-1 (low; ORDINARY WORKFLOW with a rare feature; older): a hidden change in a tree where nothing is remembered is never named

`other_tree` (`hook_run.rs` 3718 onward): `Ok(false) if !stands => OtherTree::Nothing`. Where
nothing is remembered, `git status` alone decides whether the tree is named, and it does
not show a change in a file marked assume-unchanged (nor one a clean filter or the
line-ending settings hide), all of which the judge's patch has.

```
python3 -I /var/home/louranicas/.cache/ddf-t/hooks-review-12/notes/s/w12b.py never
   status L: ''  status R: ''
-- 1 L: key in an assume-unchanged file, never stopped in; stop from R (silent?) => SILENT
-- 2 again                                                                       => SILENT
-- 3 control: stop from L (BLOCK)                                                => BLOCK   judge saw: ['lib/a.rs']
```

`v4r19` the same (`w12b-never-v4r19.txt`). The tree's change would be "not judged" from R
anyway; what the rule asks is that this is said. Rated low, like HR11-L-2, for the rarity
of the feature. Cheap: say it in README beside "where something of the tree is
remembered" (reading every tree would cost the time of HR12-L-2 at every stop).

### HR12-L-2 (low; ORDINARY WORKFLOW in a large repository; new): the judge's reading of another tree takes the whole look share, and a repaired tree "stands"

`other_tree_read` makes the judge's patch of the other tree. `working_tree`
(`judge_patch.rs` 455 onward) builds a scratch index with `git read-tree BASE` and then
`git add --all`, so every tracked file is hashed again; `git status` uses the stat cache.
It runs in `within(share)` of `LOOK_TIME` (6 s). When that is not enough, the read is
`NotRead` and the tree is named as one that "could not be looked at", with "A failure
there has blocked a stop of this session and stands", at every stop from another tree,
also after the failure was repaired.

```
python3 -I /var/home/louranicas/.cache/ddf-t/hooks-review-12/notes/s/w12b.py repairedslow
-- 1 BLOCK in L                                                            => BLOCK
   key removed in L (repaired); status L: ''
-- 2 stop from R; L's full read is slower than its share (Git add sleeps)  => allow [6.2s]
        | … could not be found out (the change could not be read from Git: `git add` did not finish in the time a hook has and was stopped): such changes are not judged. A failure there has blocked a stop of this session and stands …
-- 3 again                                                                 => allow [6.2s]  … (it could not be looked at) (a failure stands there).
-- 4 the read is fast again: forgotten (SILENT)                            => SILENT [0.4s]
```

On `v4r19` steps 2 to 4 are SILENT in 0.2 s (`w12b-repairedslow-v4r19.txt`). The slowness
is a shim (`tools/hangin`, `git add` sleeps 60 s in L); the size at which it is real I
measured on this machine: a fresh index and `git add --all` of a 10,000-file, 314 MB tree
took 0.9 s with a warm cache (`git status` 0.007 s), so a tree of roughly 1.5 GB warm, less
cold, passes the share. Never silent and nothing forgotten; the harm is a false "stands"
and about 6 seconds at every stop made from another tree while something is remembered
there. Cheap: state it in README; or, where `git status` says nothing and the read runs out
of time, say "could not be read in time" without "stands".

### HR12-L-3 (low; the sentence new, the cut older): a reason cut on the no-verdict line has no "…"

README 741 and CHANGELOG 649: "a reason a message cuts for room ends with '…'".
`no_verdict_line` (`hook.rs` 1636 to 1640) cuts each commit's reason to `REASON_CHARS / 2`
characters and closes it with ")".

```
python3 -I /var/home/louranicas/.cache/ddf-t/hooks-review-12/notes/s/cut.py     # judge tools/lj: no verdict, a long message
1 commit of this session has no verdict: e53aeaa16753 (the judge could not read its inputs or its environment (exit 3): io: the review's stand-in judge gives no verdict for this patch, and says so at lengt).
```

`v4r19` the same (`cut-v4r19.txt`). Cheap: end the cut reason with "…" there too, or say
the claim of the working trees' reasons only.

### HR12-L-4 (low; DELIBERATE EVASION or an odd state; older): a `cwd` that cannot be resolved names its tree "not in it"

`trees_to_judge` sets `unfound = standing_directory(cwd).ok().flatten()`. When
`canonicalize` fails with another error than "not found" (permission), `unfound` is `None`,
so `other_trees` names the tree the directory is in with the sentence SECURITY says is never
said of it.

```
python3 -I /var/home/louranicas/.cache/ddf-t/hooks-review-12/notes/s/w12b.py locked
-- 2 cwd = L/locked/sub, 'locked' mode 000 => allow:UNJUDGED
        |   not judged: whether the session's directory is another working tree of this repository could not be found out: it could not be looked at (Permission denied (os error 13)).
        | The working tree "…-L" of this project has changes that are not committed, and the session's directory is not in it: …
```

The stop is UNJUDGED, so nothing passes; only the sentence is false. `v4r19` the same.
Cheap: when `standing_directory` errs, take the absolute `cwd` as given for `unfound`.

### HR12-L-5 (low; tests): one kill of G holds only by a pinned constant

`what_blocks_gets_the_room_the_lines_below_leave_and_no_more` (`hook.rs` tests, around
3452) asserts `TAIL_LINE == STANDING_NAMED * (NAME_MAX + 2) + 80`, which repeats the
definition. The behavioural check beside it (`count_line(99_999, "held by the judge, for a
person", three names of NAME_MAX)`, 781 characters, fits `TAIL_LINE`) passes for the
mutant `NAME_MAX - 2` (794), and the room of layout 2 is computed from `TAIL_LINE` itself.
So that mutant changes how many characters the upper part gets (12 more) and nothing that
the guarantee "the counting lines are never cut" covers: it is equivalent by behaviour.
`python3 -c` with the line's format gives 781, 806, 794. Harm: none to the product; the
kill count is one too high.

## Suspected only (not reproduced)

- HR12-L-2 with a real tree of a few gigabytes (I used a shim and a measured rate).
- `core.autocrlf=true` checkouts: a remembered tree is "Found" and named with "stands" for
  as long as its files have CRLF, also after a repair (by reading).
- In the pruned bare-inside case (`el2` step 2) the tree's memory is dropped because its
  administrative directory is gone (`remember_within`, `listed`); the README sentence
  "nothing of it is forgotten" is about Git failing for a tree that is still listed. Not
  run: the tree cannot come back as the same tree.

## What I did not look at

`verdict_v2.rs`, `vocabulary.rs`, `schema signals`, `explain signal`, `agent setup`,
`agent doctor --gate` beyond reading one README sentence; the time matrices (`tm.py`,
`st4.py`, `worst.py`) and `wipe.py`, `km.py`, `lm.py`, `bare2.py` (the implementer reports
them unchanged; I ran `nr`, `mi`, `ot`, `ak` only); more than 2,048 failing lines; a full
disk; `core.fileMode=false`; Vesper's round 20 report; the mutation run (no `cargo`).
