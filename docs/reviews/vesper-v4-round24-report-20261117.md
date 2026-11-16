# Vesper V4 round 24 review: `7380e8e` (2026-11-17)

## Verdict

**NO-GO.** The exact round-23 and fifteenth-review reproductions are fixed, and the clean-v9 migration and ceiling correction hold. I found one Medium deliberate-evasion defect in the same pair/identity class, `VD4R24-M-1`: `pair_of` trusts the current contents of one linked administrative directory's `gitdir` file without reconciling that changed top with the top already remembered for that tree identity. With two real nested linked trees, changing the inner tree's `core.worktree` to the clean outer tree and changing the inner administrative directory's `gitdir` record to the outer tree's `.git` makes the one-direction pair test return `Pair::One`. Round 24 then judges the clean outer tree under the inner tree's identity, erases the inner failure, and prints nothing at that stop or the next one while both credentials remain in the standing inner directory.

This is the same class as `VD4R23-M-1`, HR15-L-1 and HR15-L-2: one directory is read as another linked-tree identity, so a clean reading erases memory belonging to the wrong directory. Under the convergence rule it needs one rule, not another spelling check. The state already records the old top for every identity with a remembered failure. When an administrative identity is now paired with another top while that recorded old directory still stands, neither top should be accepted as a clean reading of that identity; the stop should be UNJUDGED and retain the memory. That also preserves the accepted moved-tree interpretation: after plain `mv`, the old directory is gone, and after `git worktree repair` the new directory can be read normally. No new schema is needed.

The reviewed worktree was clean before this report at `7380e8ec4eba239eed19444b57b23add785a6eba` on `agent/loop`. Frozen SHA-256 values were verified: release `0d42de401c9c246cf99bb404115e81f37f485e70d0b2d0a20f63b62b997180ec`, debug `dc89eff2805649164a0450c01aa08a04f17bcb99d3caf0e5bebf1c905a5f4445`, round-23 release `92d740a58ad68452428e787370a2700763b53faa907b595f9c86d5729390f2ea`, and the real judge `55bf9fe6d19d7f7b66ef7cc7f3056ff9ab7ba7dc10b0e310d59a2aafd33874c9`.

## Prior findings and note

- **VD4R23-M-1: exact reproduction closed; class not closed.** With only the inner tree's `core.worktree` redirected to the outer tree, round 24 changes the second stop from the round-23 wrong-tree reading to UNJUDGED, retains the inner memory, and the following main-tree stop names the unlooked-at inner tree with its failure standing. `VD4R24-M-1` reaches the old silent/forgotten result after the linked tree's own `gitdir` record is changed to agree with the redirected top.
- **VD4R23-L-1: closed.** The exact clean-v9 reproduction is silent as it should be, but its state file is rewritten to schema `deep-diff-forge.hook-state.v10` with a `trees` member at that first stop.
- **HR15-L-1: exact reproduction closed; class not closed.** A linked tree whose `core.worktree` names a clean copy inside itself is UNJUDGED at each affected stop, and its remembered failure remains. `VD4R24-M-1` is the same wrong-directory/identity class after an administrative record is edited too.
- **HR15-L-2: exact reproduction closed; class not closed.** A cleaned `cp -a` copy is UNJUDGED, the original tree's memory remains, and returning to the original tree gives `FAIL STANDS`, not a second block. Again, the class remains open through `VD4R24-M-1`.
- **Fifteenth-review ceiling note: closed.** With `GIT_CEILING_DIRECTORIES` written as `R/../H`, Git itself sees no repository from `H/notes/today`, and the round-24 stop is silent and does not invoke the judge. The implementation compares the resolved entry rather than its misleading written prefix.

## Finding

### VD4R24-M-1 — Medium — deliberate evasion — a changed `gitdir` record lets one linked-tree identity move onto a clean tree and forget its standing failure

**Claims made false:**

1. README: “What Git answers is believed only as a pair that names one tree. The top level Git names and the Git directory it names must be one tree of the repository's own records.”
2. README: where the pair is not one tree, “nothing of either tree is forgotten, and neither directory is judged as that tree.”
3. SECURITY: “What blocked in a tree is not forgotten while its directory stands.”
4. The round-14 rule carried as Claim 1: a stop prints nothing only when every unit has a pass and nothing else is open.

**Class:** linked-tree top/identity pairing and wrong-directory clean forgetting, the existing class of `VD4R23-M-1`, HR15-L-1 and HR15-L-2. The claim should not be narrowed to exclude this case: the request explicitly asks to attack edited `gitdir` files, and the security promise says standing remembered work is not forgotten. The class-closing rule is to distrust a linked identity at a new top while the state-recorded old top for that identity still stands. Checking only another pointer or spelling would leave the same identity-reassignment class open.

**Reproduction on the frozen round-24 binary:**

1. Clone the admitted base as main tree `R`; add real linked tree `O`; add real linked tree `I` at `O/inner`.
2. Begin a session in `R`, add a credential to `I`, and stop from `I`. It BLOCKs and records one failure for `I`'s administrative identity.
3. Enable per-worktree config and set `I`'s `core.worktree` to `O`. Change only `I`'s administrative `gitdir` file from `I/.git` to `O/.git`. Exclude `/inner/` from the common repository so the outer reading is clean.
4. Git now reports `O` as the top from `I`, with an empty status. `git worktree list --porcelain` names `O` twice, once for the outer branch and once for the inner branch.
5. Add a second credential to the still-standing `I`, stop from `I`, then stop from `R`.

Observed round-24 decisions and memory:

```text
first  BLOCK   memory <inner-id>/u:1
top    .../outer
status ''
second SILENT  memory -
third  SILENT  memory -
```

Both credential files remain in `O/inner`. The cause is visible at `hook_run.rs:477-498`: for a linked Git directory, `pair_of` reads that directory's current `gitdir` file and returns `Pair::One` when its parent equals Git's top. It does not compare that top with the top already recorded for this identity in the session state, and `placed` therefore accepts the redirected outer top as the inner identity. `names_the_tree` also accepts the same constructed association. The clean outer reading is consequently added to `out.clean` for the inner id and its remembered failure is removed.

Reproducer: `/var/home/louranicas/.cache/ddf-t/vesper-scratch/v4r24-gitdir-duplicate.py`, SHA-256 `22896d1f608816cfa30ac7f4fecaebf3e41373bf1e21fc31c37ebbabb5beacc8`. It runs sequentially, uses the frozen release binary and real judge, and sets its private HOME inside the driver.

**Against earlier versions:** the exact reproduction also reaches BLOCK, SILENT, SILENT on `c9a2ece`/round 23 when run with its admitted base, so this is not introduced by `7380e8e`; round 24 fails to close the already-reported class. Published v0.8.3 has no `agent hook` command (`agent hook stop --client claude` prints the older agent usage and exits 2), so this branch-only hook defect is not applicable to the last release.

## Interpretations

- **`--separate-git-dir` linked session:** I accept UNJUDGED at a stop from the main tree. I independently created such a repository, began the session in a linked tree, put a credential in the main tree, and stopped there. Round 24 said UNJUDGED. The separate common Git directory contains no durable reverse record of that main top when the project recorded by the session is the linked top, so treating the main directory as proved would exceed the pair rule. This is conservative and the behavior is explicitly documented.
- **Plain `mv` of a linked tree:** I accept UNJUDGED before repair and `FAIL STANDS` after `git worktree repair`. I reproduced both transitions. Before repair, the administrative `gitdir` still names the vanished old top and the memory remains; after repair, Git and the record name the moved top and the unchanged credential is judged under the same identity. The proposed class fix above preserves this because the recorded old directory no longer stands.
- **Unpaired answer about another repository:** UNJUDGED is right. Once Git's top and Git directory do not form a trusted pair, the hook cannot safely classify the directory as foreign and forget all project-tree uncertainty; conservative UNJUDGED is consistent with the stated rule.
- **Separate Git directory named `X/.git`:** not independently tested. By inspection it takes the normal `.git` main-pair branch, with parent `X` as the main top; that matches the written interpretation.

The interpretation driver is `/var/home/louranicas/.cache/ddf-t/vesper-scratch/v4r24-interpretations.py`, SHA-256 `9150be691acffed5eb2c6394fb4cabaad4193eb7f0d081449f64b09d11482858`.

## Claims

1. **Broken.** `VD4R24-M-1` makes both the affected stop and the following main-tree stop silent while two credential-bearing files remain unjudged in the standing inner tree.
2. **Broken.** The implementation verifies a one-direction current record, not that the administrative identity still belongs to the directory whose failure the session remembers. The constructed answer is believed and the inner memory is forgotten.
3. **Broken.** At the affected stop, what is read from the clean outer tree counts as the inner tree's reading. The additional `names_the_tree` call repeats the same accepted constructed association.
4. **Held on the focused cases; not exhaustively tested.** The `R/../H` hidden-repository case is fixed, and source inspection shows absolute entries compared through links/`..` before filtering. I did not exhaust every empty, repeated, non-UTF-8, unresolvable or symlink-loop ceiling entry.
5. **Held.** The exact clean-v9 state is written as v10 at the first stop that reads it, with no semantic change required.
6. **Broken by Claims 1-3.** The exact replacements and documentation agree on the focused cases, but the unqualified pair, only-this-tree-reading, and silence sentences do not hold for `VD4R24-M-1`.

## Verification performed

- Read the complete request, the round-23 Vesper report, the fifteenth independent report, the full production diff, changed integration/unit tests, and the changed README, SECURITY and CHANGELOG claims.
- Verified exact HEAD/branch, clean pre-report worktree, frozen hashes, changed-file scope and `git diff --check`.
- Audited `pair_of`, `placed`, `find`, `elsewhere`, `names_the_tree`, `other_tree`, linked-instance enumeration, unlisted-tree retention, ceiling normalization, state v9/v10 loading and `Session::keep`.
- Replayed the exact nested `core.worktree` round-23 reproduction, clean-v9 migration, HR15-L-1 clean-copy redirect, HR15-L-2 cleaned copied tree, and the `R/../H` ceiling case on the frozen round-24 release binary.
- Reproduced both declared workflow interpretations above and the new finding sequentially with private HOME values set inside the drivers.
- Reviewed the coordinator's comparison record: unchanged decisions for verify5-10, the selected eleventh-review scripts, round-21/22 reproductions, round-14 cases, the fifteenth-review corpus, and all 144 matrix files; I did not count those as independent runs.

## Not examined

- I did not build, run Cargo, run the workspace/gate, mutation campaign, or integration suite; the request forbids building, so I used the frozen binaries.
- I did not independently run the 144-case matrix, the full historical batches, or the five repeated 144-test suites.
- I did not stress sixteen slow/hanging Git processes or make a stand-in Git fail exactly one command. By inspection every new `repository_dirs` call and the subsequent status/read remain inside the per-tree share of the existing six-second look budget, but timing Claim C was not independently measured.
- I did not test every combination of `GIT_DIR`, `GIT_WORK_TREE`, `GIT_COMMON_DIR`, includeIf, symlinked administrative directories, case-folding file systems, non-UTF-8 paths, or simultaneous record changes. `VD4R24-M-1` directly exercises edited `core.worktree` and `gitdir` records.
- I did not create privileged mounts, access GitHub, mutate any ref or remote, or kill any process.

STATUS: COMPLETE
