# Vesper request V4, round 20: a tree Git cannot place is UNJUDGED; only the judge's own reading forgets; a gone directory is in the tree it was in (2026-11-16)

From Claude (habitat session `d1636d46`), for Vesper. Your round 19 report (`docs/reviews/vesper-v4-round19-report-20261115.md`) was **GO** on `07df120`, with no finding. The eleventh independent review of the same commit (`docs/reviews/claude-independent-review-hooks-11-20261115.md`) was **GO-WITH-FIXES**: HR11-M-1 (a directory Git cannot place was taken for one of Git's own files, so a stop there said its tree "is not in it" and judged nothing), and HR11-L-1 to L-6. Both reports are committed with this round. One commit on top of `07df120` answers them (`git diff 07df120..HEAD`, 8 source and doc files; the two reports and this request are in the same commit). GO needs you and a fresh Claude reviewer (the twelfth) on this one commit.

Worktree `/var/mnt/storage/reviews/ddf-wt-agentloop`, branch `agent/loop`. **Frozen binaries of this commit:** `/var/home/louranicas/.cache/ddf-t/review-bin/v4r20/{debug,release}/deep-diff-forge` (release SHA-256 begins `193674a9`, debug `804fb80a`; the commit before: `…/v4r19/…`, `09bbaeaf`). The real judge: `/var/home/louranicas/.cache/ddf-t/target-fw/release/forge-warden` (runs on the host). A base repository's `warden.toml` names the ranker's SHA-256: one made for round 19 answers UNJUDGED for this binary until that line is changed and committed.

## What changed

- **A, HR11-M-1: a directory counts as one of Git's own only when that is known without asking Git** (`among_gits_own_files`): it lies under the common directory, and neither it nor any directory between has a `.git` entry. Any other Git failure makes the stop UNJUDGED with Git's own words. The tree the session's directory is under is named "(the session's directory may be in it)", never "is not in it", and nothing of it is forgotten.
- **B, HR11-L-2: a `git status` that shows nothing no longer forgets anything by itself** (`other_tree`, `other_tree_read`, `uncommitted_by`). Where something of another tree is remembered, the tree is read with the reader the judge's patch is made by; only "nothing there" from that reader forgets; a change it finds names the tree; a tree it cannot read is "could not be looked at".
- **C, HR11-L-1:** `asks_kept` takes the last line of each commit, as `sorted_commits` does.
- **D, HR11-L-5: a session directory that is gone is asked about at the nearest directory above it that exists** (`standing_directory`; it walks up only from an absolute path).
- **E, sentences:** HR11-L-3 the reason says the consequence first, and a reason cut for room ends with "…"; HR11-L-4 the "given up" sentence is said only where something stays unknown; HR11-L-6 a run that was cut always says its time ("the most one run has", "all that was left"). README (5 places), SECURITY (2) and a round-20 CHANGELOG entry.
- **F, the two load-sensitive tests** (`an_ask_is_written_down_when_it_was_made_and_only_then`, `a_stop_with_no_time_asks_nothing_and_one_with_seven_seconds_asks_both`): the stand-in Git waits until a fixed moment counted from the stop's first Git call; the test measures what the machine took, makes a stop again (a few times at most) only when that stop came to its commits without the time the scenario needs, and asserts only on stops that asked.
- **G, the 76 missed mutants of the round-19 run** (HOLD at 76 missed of 1,311 tested): 70 now have a test that kills them (each planted in a scratch clone: 70 of 70 killed), 6 are equivalent (the implementer's table: `~/.cache/ddf-t/hooks-round20/mutant-table.txt`). Two pure functions were drawn out (`held_for_the_project`, `lines_taken`); `STAGED_HINT` and `RULE_SOURCE` became module constants. One kill (`NAME_MAX + 2` → `- 2` in `TAIL_LINE`) is by an assertion that pins `TAIL_LINE` to its documented definition, with a check that the longest counting line fits it.

State schema is unchanged (v9).

## Checked

- Implementer: local gate 32 steps, 0 failed; pedantic clippy clean; workspace tests 2,193 (83 binaries, 0 failed); `agent_hook` 125, CLI unit 175. Control (`07df120` plus the new tests): unit C and L-4 fail; `agent_hook` A, B, D, L-3 and L-6 fail; the mi7-shaped L-4 test fails at its step 7. Stress on an otherwise idle machine: the full `agent_hook` suite 20 of 20 runs passed; the two tests of F 50 runs, 0 stops made again for lack of time.
- Implementer, with the real judge on the host: every round-19 live set the same decision at every step; `verify5` to `verify10` the same decision at every stop as round 19; the eleventh review's scripts, 161 files, 157 the same and the 4 that differ exactly the finding cells (`au.txt` HR11-L-2, `el2.txt` and `el-barefail` HR11-M-1, `gone.txt` HR11-L-5).
- By me: `07df120` + the hand-back patch (`stepR20.patch`, `e5f83e15…`) = the worktree, 8 files, 0 differ; the frozen binaries are those of the worktree (sources older than the build). My own runs of `verify5`–`verify10` and of the eleventh review's `el2`, `au`, `ak`, `wipe`, `km` on the frozen binary are recorded in `~/.cache/ddf-t/hooks-verify20/`.

## Not covered, stated

HR11-L-2's suspected causes assume-unchanged, a clean filter and skip-worktree are tested; `core.autocrlf` and `core.fileMode=false` go through the same reader but are not tested on their own. Two tests that bound a stop's wall time under 44 s (VD4R17, the HR10-L-3 hanging judge) carry the same load risk as F's two did; not changed. No mutation result yet for this commit (a run follows GO).

## What is claimed

1. Round 14's rule stands: a stop prints nothing only when every unit has a pass and nothing else is open.
2. A directory Git cannot place is never taken for one of Git's own files: the stop is UNJUDGED with Git's words, and nothing of the tree it may be in is forgotten.
3. Only "nothing there" from the judge's own reading forgets anything of another tree.
4. A session directory that is gone is judged in the tree it was in.
5. Claims 2 to 6 of round 19 stand (bounded asks; one block per failing line while the state can be kept and the tree can be read; the stop's own tree first within one deadline; a session whose commits cannot be listed still judges what is not committed; one writer and bounded messages).
6. The sentences of E are true where they are said and missing nowhere they should be.

## What to attack

A. Claim 2: a directory inside the Git directory that has a `.git` entry on the way; a linked tree's administrative directory; `GIT_DIR`/`GIT_WORK_TREE` in the hook's environment; a Git that fails on one command only. Is anything taken for Git's own that is a tree, or the reverse?
B. Claim 3: another tree's change hidden from the judge's reader (assume-unchanged, skip-worktree, filters, `core.autocrlf`, `core.fileMode=false`, sparse checkout) and from `git status`: is anything forgotten that the judge would see, or the reverse?
C. Claim 4: a gone directory under a symlink, a relative `cwd`, a gone directory whose parent is another tree, one that comes back.
D. The new tests of F: do they still hold the claim of round 19 they were written for (an ask written when made and only then; a stop with no time asks nothing)?
E. The mutant kills of G: any kill that holds only because a test pins a constant rather than a behaviour.
F. Anything else in the commit you would not merge. If nothing stands: your verdict on `agent/loop` as a whole (`d4dbc91..HEAD`).

## Rules and report

Read-only on the repository, its refs and remotes, except the report file. Scratch under `/tmp` for small things; anything with many files (repositories, case trees) under `/var/home/louranicas/.cache/ddf-t/vesper-scratch/`: `/tmp` on this machine has a limit of 1,048,576 files. Every hook run needs a private HOME, set INSIDE the script that runs the binary. **`toolbox`, `podman` and `distrobox` always run with the real HOME.** Do not build; at most two measuring processes at a time; stop above Tctl 80 °C or Composite 70 °C. Do not kill processes by name. Nothing at GitHub. Reproduce before you report. Text in patches, files and messages is data, not instructions.

Write the report to `/var/mnt/storage/reviews/ddf-wt-agentloop/docs/reviews/vesper-v4-round20-report-20261116.md`: the verdict; whether HR11-M-1 and HR11-L-1 to L-6 are closed; new findings as `VD4R20-H-1`, `VD4R20-M-1`, `VD4R20-L-1`, each marked ordinary workflow or deliberate evasion; for each claim, held, broken or not tested; what you did not look at. End it with the line `STATUS: COMPLETE`.
