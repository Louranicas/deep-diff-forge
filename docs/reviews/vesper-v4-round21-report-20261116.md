# Vesper V4 round 21 review: `67202b1` (2026-11-16)

## Verdict

**NO-GO.** I found one Medium deliberate-evasion defect, `VD4R21-M-1`: a valid linked working tree may be placed below one of the 17 names now treated as structurally Git-owned. Once both of that tree's identity records are removed, the frozen binary again produces a silent stop and forgets the failure that stood there. This is the `VD4R20-M-1` outcome at the boundary the round explicitly asked me to attack, so that finding is not closed.

I also found one Low deliberate-evasion defect, `VD4R21-L-1`: when an unresolved hook `cwd` is relative, the stop preserves memory but still says that the session's directory is not in the linked tree it denotes. Thus the absolute and partially resolved forms of `HR12-L-4` are fixed, but its claimed general rule is not closed.

`VD4R20-L-1`, `HR12-L-1`, `HR12-L-2`, `HR12-L-3` and `HR12-L-5` are closed. `VD4R20-M-1` and `HR12-L-4` are not closed.

The reviewed worktree was clean at `67202b174431ca88352ed12e781e0754d666ba63`, parent `4b134bd91943ff3b4155fbfc4a4806c50e404eea`. The frozen release binary SHA-256 is `9bc2666879f0c2ecb1504a60c99c597326f7f5765666f4c01d0dcbb96e80b204`; debug is `a26239da0d860c547af1413b9f162ae8488fbaa7641c21422511e5a6883fa835`. The real judge used for live checks is `55bf9fe6d19d7f7b66ef7cc7f3056ff9ab7ba7dc10b0e310d59a2aafd33874c9`.

## Findings

### VD4R21-M-1 — deliberate evasion: a linked tree below a trusted Git-subtree name becomes silent and is forgotten when both identity records are removed

`GITS_OWN_SUBTREES` includes `hooks`, `modules` and 15 other top-level names at `hook_run.rs:2112-2130`. `among_gits_own_files` returns true for every descendant of one of those names when `symlink_metadata(<ancestor>/.git)` reports `NotFound` at each step (`hook_run.rs:2140-2165`). At `hook_run.rs:2066`, a failed Git lookup with that result becomes `Elsewhere::Nothing`. Since that produces no `unfound` path, `other_trees` is allowed to write the now-shorter linked-tree listing at `hook_run.rs:3647-3652`, erasing the missing tree's remembered failure.

Git permits a linked worktree at `repo.git/hooks/feature`. Its `.git` file distinguishes it while present, but after both identity records are removed it is indistinguishable to this heuristic from an ordinary directory below `hooks`. Reproduction:

1. Clone a bare repository and add `main` plus the linked tree `hooks/feature` inside it; start the session in `main`.
2. Add a credential in `hooks/feature`; a stop from that tree blocks and remembers one failure.
3. Remove `hooks/feature/.git` and `repo.git/worktrees/feature`, leave its reviewed files in place, and add a second credential.
4. Confirm `git -C hooks/feature rev-parse --show-toplevel` exits 128 with `fatal: this operation must be run in a work tree`.
5. Stop again from `hooks/feature`.

Frozen-v4r21 result:

```text
first BLOCK memory e4f12733bee9e9fe/u:1
git_rc 128 git_stderr fatal: this operation must be run in a work tree
second SILENT memory -
<silent>
```

The second credential was not put to the judge, and the first failure was forgotten. This breaks claims 1, 2 and 3. It also falsifies the source and README premise that each listed subtree holds Git's own files only: those are conventional Git-owned locations, not locations Git reserves against a worktree path.

Reproduction: `/var/home/louranicas/.cache/ddf-t/vesper-scratch/v4r21-reserved-subtree.py`, SHA-256 `b47a1f87ee1e502a655ce1d2c352259a33779e65ee728c666e865966b4bde7c6`. It runs the frozen release binary and real judge sequentially with a private HOME set inside the driver.

The regression test added in this commit checks `objects/wt` only while its `.git` entry remains. It should also remove both records from a real linked tree under each accepted structural class, at least `hooks/feature`, and require UNJUDGED, retained memory and no judge run for the new content. A safe fix must not use membership in these conventional subtrees as conclusive proof after all worktree identity has disappeared; alternatively, the hook must reject or separately remember such a session tree before relying on that classification.

### VD4R21-L-1 — deliberate evasion: an unresolved relative `cwd` is still described as outside the linked tree it denotes

On `Elsewhere::Failed`, `trees_to_judge` keeps an unresolved relative path as written (`hook_run.rs:3914-3919`). `other_trees` then tests that relative path with `cwd.starts_with(&tree.path)` where `tree.path` is absolute (`hook_run.rs:3719`). That comparison cannot match, so the linked tree is described with “the session's directory is not in it.”

I began a session in the main tree, blocked a credential in a linked tree, changed the driver's real current directory to the main tree, made `linked/locked` unsearchable, and supplied the equivalent relative `cwd` `../<linked>/locked/sub`. The frozen binary returned UNJUDGED and retained memory, but said:

```text
The working tree "...-linked" of this project has changes that are not committed, and the session's directory is not in it ... A failure there ... stands.
```

This is narrower than the original ordinary absolute-path report: it requires a relative client value plus an unresolved component, it does not make the stop silent, and memory stays intact. It nevertheless breaks claim 4 and the unqualified README/SECURITY statement. Resolve the relative path against the hook process's actual current directory before applying `same_form`, or avoid the definitive “not in it” statement when the relative input cannot be anchored reliably.

Reproduction: `/var/home/louranicas/.cache/ddf-t/vesper-scratch/v4r21-relative-locked.py`, SHA-256 `707617581cafb7922db9c35176e70d1af1d185cec0fb3e0161fb27e25c286f67`. It runs the frozen release binary and real judge sequentially with a private HOME set inside the driver.

## Requested finding status

- **VD4R20-M-1 — not closed.** The ordinary `repo.git/feature` reproduction is fixed, but `VD4R21-M-1` reaches the same silent stop and forgotten memory with the valid tree path `repo.git/hooks/feature`.
- **VD4R20-L-1 — closed.** The table now reports 69 behaviorally killed and 7 equivalent mutants. The `TAIL_LINE` equality is explicitly a definition check, and the `+2 -> -2` mutant is listed as equivalent with the 806/794/781 measurements.
- **HR12-L-1 — closed.** README and SECURITY now state that a never-blocked other tree whose only changes `git status` hides is not named.
- **HR12-L-2 — closed.** README and SECURITY state the bounded full-read behavior, repeated “could not be looked at” note, standing failure and per-stop delay.
- **HR12-L-3 — closed.** `no_verdict_line` keeps the ellipsis inside the 150-character reason bound and its focused test covers ASCII and multibyte boundaries.
- **HR12-L-4 — not closed.** Absolute unresolved paths and absolute paths through a resolvable symlink prefix now produce “may be in it”; `VD4R21-L-1` retains the false “not in it” statement for an unresolved relative path.
- **HR12-L-5 — closed.** Same accounting correction as `VD4R20-L-1`.

## Claims

1. **Broken.** `VD4R21-M-1` is silent while unjudged credentials and a remembered failure are open.
2. **Broken.** Location under a conventional Git subtree is not proof that no working tree exists there; removing both records from `hooks/feature` makes that session tree silent.
3. **Broken.** Git cannot place the session directory in `VD4R21-M-1`, yet the structural shortcut prevents `unfound` from being set and the shortened listing forgets the tree.
4. **Broken.** `VD4R21-L-1` names an unresolved relative session directory as outside the linked tree it denotes.
5. **Held within this round's reviewed scope.** I found no change to the judge-reader-only forgetting rule, gone absolute-directory handling, bounded asks, one block per failing line, own-tree-first ordering, single-writer state or bounded messages beyond the classification defects above. I did not rerun the full historical corpus.
6. **Held for D and E.** The assume-unchanged and bounded-read limitations are now stated, and the 69/7 mutation sentence matches the supplied table. The separate structural-subtree sentences are false under `VD4R21-M-1`.

## Verification performed

- Read the request, round 20 report, complete twelfth independent review, production/test diff in `hook.rs`, `hook_run.rs` and `agent_hook.rs`, changed README/SECURITY/CHANGELOG text, and the full corrected mutation table.
- Confirmed branch, exact HEAD/parent, clean pre-report worktree, changed-file scope and `git diff --check`.
- Hashed both frozen binaries and the real judge.
- Reproduced sequentially on the frozen release binary with the real judge:
  - a linked tree at `repo.git/hooks/feature`, followed by removal of both identity records: BLOCK then SILENT, with remembered state erased (`VD4R21-M-1`);
  - an unsearchable linked-tree directory supplied by a relative `cwd`: UNJUDGED and memory retained, but the linked tree falsely described as outside the session (`VD4R21-L-1`).
- Inspected the focused tests for the structural classification, missing-tree memory, unresolved absolute path and ellipsis boundary. The structural test stops before removing the `.git` entry from its subtree-resident simulated tree.
- All live runs used one measuring driver at a time, private HOME values set inside the drivers, and stayed below the owner's stop limits (observed Tctl 69.1–74.0 °C; Composite 44.9 °C). No process was killed.

## Not examined

- I did not build, run Cargo, run the workspace/gate, or execute a mutation campaign; frozen binaries and supplied artifacts were used as required.
- I did not rerun the full `verify5`–`verify10` or eleventh-review corpus, all five repeated integration suites, the load-sensitive tests, or all 76 mutant plants.
- I did not repeat the reserved-subtree attack under all 17 names, with an unreadable `.git` entry, or under a real submodule administrative directory; the `hooks/feature` case is sufficient to disprove the classifier's universal claim.
- I did not separately execute the directory-returns, foreign-repository, rebase, submodule, LFS/annex, sparse-checkout, clean-filter, CRLF or file-mode matrices.
- I did not re-review the pre-round-20 branch, daemon/TUI behavior, or any remote forge. Nothing was accessed at GitHub.

STATUS: COMPLETE
