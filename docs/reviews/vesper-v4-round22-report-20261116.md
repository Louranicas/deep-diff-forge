# Vesper V4 round 22 review: `8959ca8` (2026-11-16)

## Verdict

**NO-GO.** I found one Medium deliberate-evasion defect, `VD4R22-M-1`. The new conservative `Found::Failed` handling fixes the reported existing-directory cases, but a missing relative `cwd` returns `Elsewhere::Nothing` before the new relative-path anchoring is reached. With an intact linked tree this falsely says the session is not in that tree and does not judge it. With the linked tree's two identity records removed, the frozen binary again gives a silent stop and forgets the failure that stood there.

The exact `repo.git/hooks/feature` and locked-relative reproductions from round 21 are fixed. Nevertheless, `VD4R20-M-1`, `VD4R21-M-1`, `VD4R21-L-1`, and `HR12-L-4` are not closed as their general rules: the new trigger reaches the same silent/forgotten result and the same relative-path misclassification. `HR13-L-1` and `HR13-L-2` are closed.

The reviewed worktree was clean before this report at `8959ca8138950c21702b6ec0f74c0dd43e7aa6cf`, parent `67202b174431ca88352ed12e781e0754d666ba63`. The frozen release binary SHA-256 is `4709a2d02f0725b9c518e173ffaebed19fbe58232c6922cbca3a1dbfea33c58c`; debug is `c0f80fddefe18d9dd54c3e7f2cebbeb8d37a9198764c1daad8d9786e0a5f95fc`. The real judge used in the live checks is `55bf9fe6d19d7f7b66ef7cc7f3056ff9ab7ba7dc10b0e310d59a2aafd33874c9`.

## Finding

### VD4R22-M-1 — deliberate evasion: a missing relative `cwd` bypasses `Unfound`, allowing a silent stop and forgotten tree memory

`elsewhere()` calls `standing_directory(Path::new(cwd))` before any relative-path anchoring (`hook_run.rs:2075-2079`). `standing_directory()` deliberately does not walk upward from a relative path: the first `NotFound` or `NotADirectory` returns `Ok(None)` (`hook_run.rs:2121-2138`). `elsewhere()` turns that into `Elsewhere::Nothing`.

The new anchoring is only in `Unfound::of()` (`hook_run.rs:3864-3876`), and `Unfound::of()` is called only after `Elsewhere::Failed`. It is therefore unreachable for a relative path whose named directory is missing. `unfound` remains `None`, so `other_trees()` is allowed to accept a shortened administrative listing and forget a no-longer-listed tree (`hook_run.rs:3617-3629`).

I reproduced two forms with the frozen release binary and real judge, changing the driver's process directory to the main tree before passing a relative `cwd` beneath the linked tree:

1. With an intact linked tree, block a credential there, remove only the empty `build/gone` directory named by `cwd`, and stop with `../<linked>/build/gone`. The stop does not judge the linked tree and falsely says:

   ```text
   The working tree "...-linked" of this project has changes that are not committed, and the session's directory is not in it: they were not judged ... A failure there ... stands.
   ```

   This is an ordinary precursor: a tool can remove the directory it was in, and relative hook input is within the claimed behavior. It is the remaining `VD4R21-L-1`/`HR12-L-4` misclassification, now reached by `NotFound` rather than `PermissionDenied`.

2. In a fresh session, after the first linked-tree credential blocks, also remove `linked/.git` and its `.git/worktrees/<id>` administrative directory while leaving the reviewed files in place. Stop from the same missing relative path. The result is:

   ```text
   unlisted first BLOCK memory b3d0d6aa464412df/u:1
   unlisted relative ../vesper-r22-relative-gone-unlisted-420629-420629-linked/build/gone
   unlisted second SILENT memory -
   <silent>
   ```

   That composition is deliberate evasion or damaged Git administration, but it has the round-20/21 Medium impact: unjudged credentials remain, a prior blocking failure is erased, and the stop is silent. It breaks the round-14 silence rule and claims 2 through 4.

Reproduction: `/var/home/louranicas/.cache/ddf-t/vesper-scratch/v4r22-relative-gone.py`, SHA-256 `2d4e27a88c978c2e972a9d101e7b646679808214b4713b8504421b68115e57f0`. It runs one hook process at a time with the private HOME set inside the driver environment and changes only scratch clones.

Anchor a relative `cwd` against the hook process's current directory before the first call to `standing_directory`, then walk an absent path to its nearest existing ancestor just as for an absolute path. If the process directory cannot be obtained, use the conservative `Elsewhere::Failed`/`Unfound::Anywhere` path. A regression should cover both the intact linked tree and removal of both identity records, requiring the former to judge the linked tree and the latter to be UNJUDGED with memory retained.

## Requested finding status

- **VD4R20-M-1 — not closed as a class.** Its exact absolute, existing plain-tree reproduction is fixed, but `VD4R22-M-1` again produces SILENT and erases the same tree memory after both identity records disappear.
- **VD4R21-M-1 — not closed as a class.** Removing the structural Git-subtree shortcut fixes the exact `repo.git/hooks/feature` trigger. Adding a missing relative `cwd` bypasses the new `Failed` arm and restores the same outcome without relying on a reserved subtree.
- **VD4R21-L-1 — not closed.** The locked relative path is now anchored and correctly described as possibly inside the linked tree. A missing relative path still returns before anchoring and is falsely described as outside it.
- **HR12-L-4 — not closed.** Absolute and inaccessible-relative forms are fixed, but the stated rule for an unresolved path is still false for relative `NotFound`.
- **HR13-L-1 — closed.** The structural subtree list and shortcut are removed; the exact `hooks`, `svn`, and `logs` class now goes through the conservative failure path when Git returns a non-no-repository failure.
- **HR13-L-2 — closed.** `unplaced()` puts Git's words first, the incomplete structural list is gone, and README/SECURITY now describe the actual no-listing and judge-clean cases rather than claiming every conventional Git directory is classified structurally.

## `GIT_DISCOVERY_ACROSS_FILESYSTEM`

Leaving `GIT_DISCOVERY_ACROSS_FILESYSTEM` unchanged is reasonable and does not itself block this round. Clearing `GIT_CEILING_DIRECTORIES` is necessary because a caller's arbitrary ceiling is not a repository boundary. In contrast, the default mount boundary is Git's normal discovery and trust boundary, while a caller that intentionally works across it can opt in with `GIT_DISCOVERY_ACROSS_FILESYSTEM=1`. The mount limitation is now stated.

This means the universal wording must retain that qualification: with the variable unset, a directory on a separately mounted filesystem beneath a linked tree is intentionally `NotARepository` to Git and the hook cannot infer the enclosing tree. I did not create a mount to live-test that declared limit.

## Claims

1. **Broken.** The second `VD4R22-M-1` reproduction is silent with an unjudged credential and a remembered failure open before the stop.
2. **Broken.** Every actual `Found::Failed` path I inspected is now conservative, including failures below the Git common directory. But a missing relative session directory returns before Git and is still taken as adding nothing; the deliberate composition is silent rather than UNJUDGED.
3. **Broken.** The absent relative path leaves `unfound == None`; the shortened linked-tree listing then forgets the missing tree and its blocking memory.
4. **Broken.** An unresolved relative `cwd` ending in a missing component is named as outside the linked tree it is under. The new link/`..` resolver is not reached.
5. **Held in source and supplied focused evidence.** Every hook Git acquisition is made through `Deadline::git()`, which supplies an empty `GIT_CEILING_DIRECTORIES`; inherited `GIT_DIR`, `GIT_WORK_TREE`, and `GIT_COMMON_DIR` are removed by the shared launcher. The focused ceiling test and frozen verification artifacts pass. I did not independently recreate its live fixture.
6. **Broken only where it incorporates round-20 claim 4 and this round's directory sentences.** The judge-reader-only forgetting rule, bounded asks, one block per retained failing line, own-tree-first order, unavailable-commit handling, single-writer state, bounded messages, and Git-words-first sentence remain supported by this diff and the supplied regression runs. The unqualified gone/relative-directory rule is broken by `VD4R22-M-1`.

## Verification performed

- Read the request, round-21 report, complete thirteenth independent review, CHANGELOG/README/SECURITY changes, the production diff, and the new integration/unit tests.
- Confirmed branch, exact HEAD/parent, pre-report clean worktree, changed-file scope, and clean `git diff --check`.
- Hashed the frozen debug/release binaries and real judge.
- Audited `find`, `elsewhere`, `standing_directory`, `Unfound::of`, `through_links`, `trees_to_judge`, `other_trees`, the shared Git environment removal, and every hook use of `Deadline::git()` relevant to discovery.
- Reproduced the two round-21 cases on the frozen release binary: the reserved-subtree case is BLOCK then UNJUDGED with memory retained; the locked relative case is UNJUDGED, retains memory, and says the linked tree may contain the session directory.
- Reproduced `VD4R22-M-1` twice: the intact linked tree is falsely named as outside the session and not judged; after both identity records are removed, the stop is SILENT and memory becomes empty.
- Checked ordinary Git behavior with inherited `core.worktree`, `core.bare`, and `include.path` entries supplied through `GIT_CONFIG_COUNT`; none changed `rev-parse --show-toplevel` in the scratch repository. Source inspection confirms `safe.directory` refusal takes the conservative `Found::Failed` route.
- Reviewed the coordinator's frozen-binary records for `verify5` through `verify10`, the eight selected eleventh-review drivers, and the focused round-22 reproductions; every recorded driver exited zero. I did not treat those coordinator runs as independent execution.

## Not examined

- I did not build, run Cargo, run the workspace/gate, or execute the full integration suite; the frozen binaries and recorded artifacts were used as required.
- I did not independently rerun the full historical `verify5`–`verify10` and eleventh-review corpora, the five repeated `agent_hook` suites, timing/load tests, or mutation campaign.
- I did not create a cross-filesystem mount, alter ownership for a live `safe.directory` refusal, or execute full rebase, submodule, notes-merge, LFS/annex, symlink-loop, directory-returns, or foreign-repository matrices.
- I did not re-review the pre-round-20 branch, daemon/TUI behavior, or any remote forge. Nothing was accessed at GitHub.

STATUS: COMPLETE
