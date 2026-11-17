# Vesper V4 round 20 review: `4b134bd` (2026-11-16)

## Verdict

**NO-GO.** I found one Medium deliberate-evasion defect, `VD4R20-M-1`, which leaves HR11-M-1 open under a neighboring form of the same mistaken classification. I also found one Low ordinary-workflow issue in the mutation accounting, `VD4R20-L-1`.

HR11-L-1 through HR11-L-6 are closed. The exact HR11-M-1 reproductions with a present but broken `.git` pointer and with a one-command Git failure are fixed, but the claimed general rule is not: absence of every `.git` entry is treated as proof that an arbitrary directory under the common directory is Git's own.

The reviewed worktree was clean at `4b134bd91943ff3b4155fbfc4a4806c50e404eea`, parent `07df12074d68d4c3246888604b0ea316ac79b799`. The frozen release binary SHA-256 is `193674a9c5a3b6db8adeb73d5baa87debb7c26984060a4ecbc4b281fa2f42ecd`; debug is `804fb80a7ad82c8ffdfebf3851f7486994d82a6361c39ac1ff3b0d668d1a8a07`. The real judge used for live checks is `55bf9fe6d19d7f7b66ef7cc7f3056ff9ab7ba7dc10b0e310d59a2aafd33874c9`.

## Findings

### VD4R20-M-1 — deliberate evasion: deleting both identity records makes a live linked-tree directory “Git's own”, producing a silent stop and forgetting its failure

`among_gits_own_files` at `hook_run.rs:2096` treats a canonical directory as known to be Git's own when it is below the common directory and no directory from it up to the common directory has a `.git` entry. At `hook_run.rs:2058`, any failed `find` with that result becomes `Elsewhere::Nothing`.

The absence of a `.git` entry does not prove that the directory is administrative. In the supported bare-inside layout (`repo.git/main`, `repo.git/feature`), it can be the still-present directory in which the session was working after its two identity records have been removed:

1. Create the bare repository and its `main` and `feature` linked trees inside it; begin the session in `main`.
2. Put a credential in `feature`; a stop from `feature` blocks and remembers one uncommitted failure.
3. Remove `feature/.git` and `repo.git/worktrees/feature`, leaving `feature` and its reviewed files in place; add a second credential there.
4. `git -C feature rev-parse --show-toplevel` exits 128 with `fatal: this operation must be run in a work tree`.
5. Make another stop from `feature`.

Frozen-v4r20 result:

```text
first BLOCK memory ee3ddea3025b9866/u:1
git_rc 128 git_stderr fatal: this operation must be run in a work tree
second SILENT memory -
<silent>
```

The second credential was not put to the judge. The earlier failure was forgotten because the administrative directory no longer identified the linked tree. This breaks round 14's silence rule and claims 2 and 6's documented statement that an unplaceable session tree is UNJUDGED with Git's words and nothing is forgotten. It is the HR11-M-1 outcome again, reached by removing the `.git` entry as well as its target.

Reproduction: `/var/home/louranicas/.cache/ddf-t/vesper-scratch/v4r20-live/missing_git_entry.py`, SHA-256 `d71f687c152a348d593e9bb8d3fe7f4de689b5672bf7ed7f8dc1b04582eeeab9`. It runs the frozen release binary and real judge with a private HOME set inside the driver environment. It creates and damages only its own scratch clone.

The safe rule is not to infer that an arbitrary descendant of a bare common directory is administrative merely because `.git` is absent. On `Found::Failed`, ambiguous plain directories should remain `Elsewhere::Failed`; `Elsewhere::Nothing` can be limited to paths structurally known to be Git administration (the common directory itself and recognized administrative subtrees). A regression should remove both the linked tree's `.git` file and its administrative entry while its directory remains, then require UNJUDGED, retained memory, and no claim that the directory is Git's own.

### VD4R20-L-1 — ordinary workflow: one claimed mutant kill pins an internal formula although the mutant still satisfies the behavioral bound

The mutation table calls `TAIL_LINE = STANDING_NAMED * (NAME_MAX + 2) + 80` changed to `NAME_MAX - 2` killed by `what_blocks_gets_the_room_the_lines_below_leave_and_no_more`. The killing assertion is the direct equality:

```rust
assert_eq!(TAIL_LINE, STANDING_NAMED * (NAME_MAX + 2) + 80);
```

With `NAME_MAX = 240` and three names, the production constant is 806 and the mutant is 794. The adjacent test's actual longest `count_line(99_999, "held by the judge, for a person", three 240-character names)` is 781 characters. Therefore its behavioral safety assertion, `longest <= TAIL_LINE`, passes under the mutant. The equality pins the chosen conservative formula; it does not demonstrate a message overflow, a lost count, or another contract failure.

This mutant should be reported with the equivalent/non-behavioral cases unless a behavioral edge is added that fails at 794. On the supplied evidence the accurate summary is 69 behaviorally killed and 7 equivalent/non-behavioral, not 70 and 6. This does not affect runtime correctness, but the mutation evidence is part of the round's claimed assurance.

## Prior findings

- **HR11-M-1 — not closed.** Its exact broken-pointer and one-command-failure cases are fixed and reproduced, but VD4R20-M-1 reaches the same silent/forgotten result when the `.git` entry itself is gone.
- **HR11-L-1 — closed.** `asks_kept` collects the last parseable answer per commit before testing the two waiting lanes. The new unit test covers no-verdict followed by pass/fail/hold/final answers.
- **HR11-L-2 — closed.** A remembered other tree with clean `git status` is read through `uncommitted_by`; only `Reading::Nothing` forgets it. My assume-unchanged five-step reproduction kept memory and returned `FAIL STANDS` without a second block.
- **HR11-L-3 — closed.** The consequence now precedes the long cause, and a truncated reason ends in `…` rather than a misleading full stop.
- **HR11-L-4 — closed.** `gone: gone && !all` suppresses the “may block once more” sentence at the stop from which the whole index is remembered; the focused test follows the reported 600-line sequence.
- **HR11-L-5 — closed for its reported ordinary workflow.** A gone absolute directory walks to the nearest existing ancestor. My live deleted-`build/out` reproduction blocked on the linked tree and then stood. Relative nonexistent `cwd` values are deliberately not walked; I did not treat malformed/relative client input as the original finding.
- **HR11-L-6 — closed.** A stopped run now names the 30-second maximum, its apportioned time, or all remaining time as applicable; README's linked-tree figure and gate/list wording match the code.

## Claims

1. **Broken.** Round 14's silence rule fails in VD4R20-M-1: the stop is silent while two credentials remain in the session directory and its remembered failure is open.
2. **Broken.** A directory Git cannot place can still be taken for Git's own when its `.git` entry is absent; it is not UNJUDGED and its tree memory is forgotten.
3. **Held for the reviewed and reproduced paths.** The judge's own reader, not clean `git status`, decides whether a remembered tree is empty. Assume-unchanged is reproduced. Skip-worktree and filters are covered by the committed tests/recorded checks; I did not separately run `core.autocrlf` or `core.fileMode=false`.
4. **Held for the stated ordinary workflow.** A gone absolute directory under a linked tree is resolved through its nearest existing ancestor and the linked tree is judged. The source also handles a symlinked existing ancestor by canonicalization. A relative missing `cwd` is explicitly not walked and was not tested as a supported client path.
5. **Held within this round's reviewed scope.** I found no regression in bounded asks, one block per failure while state/tree remain available, own-tree-first ordering, commits unavailable while uncommitted work is judged, or bounded single-writer messages. The supplied `verify5`–`verify10` runs all return zero, and the focused live cases preserve the prior decisions. VD4R20-M-1 is outside the qualification that the tree can still be read/identified.
6. **Held for the sentences in item E.** The reordered/cut reason, conditional digest sentence and all stopped-run time forms are true where emitted, and the README/SECURITY edits match them. The separate directory-classification sentences are false under VD4R20-M-1 and must change with its fix.

## Attacks A–F

- **A — broken.** Present `.git` entries, broken pointers, linked administrative directories, Git environment overrides and one-command failure are handled conservatively. Removing the `.git` entry as well as its administrative peer defeats the absence-based shortcut (VD4R20-M-1).
- **B — held in the tested case.** Assume-unchanged is no longer forgotten; source inspection shows the remembered-tree re-read shares the judge patch builder and absolute deadline. I did not separately execute sparse checkout, clean-filter, CRLF or mode-only matrices.
- **C — held for the reported case.** Gone absolute directories under a linked tree are judged correctly. I did not run a relative-`cwd` client, and a nonexistent relative path is explicitly not walked. A symlinked existing ancestor follows canonicalization; a directory that returns is resolved normally at the next stop.
- **D — held by source design; not rerun as a compiled test.** The stand-in Git waits for a wall-clock moment measured from the stop's first Git call, the test measures remaining time, retries only boundedly when load consumed the scenario's time, and asserts ask/write correspondence on qualifying stops. I did not build or run Cargo.
- **E — one disclosed kill is not behavioral.** The `TAIL_LINE` `+2 → -2` mutant is killed only by the formula equality while the actual longest-line bound still passes (VD4R20-L-1). The remaining sampled mutant tests assert observable parsing, message bounds, memory, time or locking behavior; I found no second constant-only kill.
- **F — not mergeable as submitted.** VD4R20-M-1 is a silent unjudged change and memory loss; it requires a fix and re-check.

## Verification performed

- Read the request, the complete eleventh independent review, the production diff in `hook.rs`, `hook_run.rs` and `judge_patch.rs`, the relevant integration/unit tests, all changed README/SECURITY/CHANGELOG claims, and the full mutant table.
- Confirmed branch, exact HEAD/parent, clean worktree, changed-file scope and `git diff --check`.
- Hashed the frozen debug/release binaries and real judge.
- Reproduced on the frozen release binary with the real judge:
  - broken administrative pointer in a bare-inside tree: UNJUDGED with Git's words;
  - assume-unchanged: remembered failure named from another tree and `FAIL STANDS` on return;
  - deleted linked-tree subdirectory: BLOCK then `FAIL STANDS`;
  - missing `.git` plus missing administrative entry: SILENT and memory erased (VD4R20-M-1).
- Recomputed the `TAIL_LINE` production/mutant reserves and actual longest line: 806, 794 and 781 characters.
- All live runs were sequential (one measuring driver at a time), used private HOME values only inside their hook-driver environments, and stayed below the owner's limits (observed Tctl 52.4–70.1 °C; Composite 43.8 °C). No process was killed.

## Not examined

- I did not build, run Cargo, run the workspace/gate, or execute the mutation campaign; frozen binaries and recorded test/mutation artifacts were used as required.
- I did not rerun the full `verify5`–`verify10` corpus, all 161 eleventh-review files, the load-sensitive timing tests, or every 70 planted mutant. I read their recorded results and reran the three finding paths plus my new attack.
- I did not separately execute `core.autocrlf`, `core.fileMode=false`, clean-filter, sparse-checkout, symlinked-gone-directory, relative-`cwd`, or directory-returns matrices.
- I did not re-review `verdict_v2.rs`, the entire pre-round-20 branch, daemon/TUI behavior, or any remote forge.

STATUS: COMPLETE
