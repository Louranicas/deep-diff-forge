# Vesper V4 round 23 review: `c9a2ece` (2026-11-16)

## Verdict

**NO-GO.** I found one Medium deliberate-evasion defect, `VD4R23-M-1`: the new check accepts a Git top merely because it contains the session directory. With two genuine nested linked trees, `core.worktree` can redirect the inner tree to the outer one. The hook then judges the clean outer tree under the inner tree's identity, erases the inner tree's remembered failure, and the following stop is silent while both credentials remain in the valid, listed inner tree.

I also found one Low ordinary-workflow migration defect, `VD4R23-L-1`: a clean v9 state is read but not written back as v10 because `Session::keep()` compares only the parsed semantic state. It remains byte-for-byte v9 until some state member changes.

The exact round-22 relative/gone reproduction is fixed. `VD4R22-M-1`, `HR14-L-1`, `HR14-L-2`, and `HR14-L-3` are closed. The exact HR14-L-4 case (redirecting a sibling linked tree to the main tree) is fixed, but HR14-L-4 is not closed as a general rule because `VD4R23-M-1` reaches its silent/forgotten outcome with a redirected ancestor. Claim 6's v9 rewrite is also not met.

The reviewed worktree was clean before this report at `c9a2ece4eb5fb1d6f9ace01fefd164179398b92b`, parent `8959ca8138950c21702b6ec0f74c0dd43e7aa6cf`. The frozen release binary SHA-256 is `92d740a58ad68452428e787370a2700763b53faa907b595f9c86d5729390f2ea`; debug is `07fe685399755d435a1f95aca5cc3582bb5f5cd2d371576e10ae9502ccd6c8d2`. The real judge used in live checks is `55bf9fe6d19d7f7b66ef7cc7f3056ff9ab7ba7dc10b0e310d59a2aafd33874c9`.

## Findings

### VD4R23-M-1 — deliberate evasion: a redirected inner tree is accepted as its outer tree, judged clean under the wrong identity, and forgotten

The new invariant at `hook_run.rs:2291-2303` rejects `Found::Tree` only when `cwd` is not below `found.top`. Containment is necessary, but it does not prove that Git has named the same working-tree instance. Git permits one linked working tree inside another. If the inner tree's per-worktree `core.worktree` names the outer tree, Git reports the genuine outer top while retaining the inner tree's administrative `git_dir`; the outer top contains the inner `cwd`, so the check accepts it as `Elsewhere::Linked`.

`trees_to_judge()` then pairs `linked.top` (the outer directory) with `tree_instance(&linked)` (the inner administrative directory's identity) at `hook_run.rs:4370-4374`. A clean reading of the outer tree therefore replaces the remembered lines of the inner tree with nothing. The later `pointed_elsewhere()` safeguard notices that looking at the inner path names the outer tree and emits an UNJUDGED note, but it is too late: the own-tree reading has already been recorded as a clean reading of the inner id. The note even says the prior failure stands at this stop while the state written after it has already removed that failure. The next clean-project stop has no memory and is silent.

Reproduction with the frozen release binary and real judge:

1. Create main tree `R`, linked tree `O`, and a second linked tree `I` at `O/inner`; Git accepts all three and lists both linked trees.
2. Begin the session in `R`, put a credential in `I`, and stop from `I`: BLOCK, with one inner-tree failure remembered.
3. Enable per-worktree configuration and set `I`'s `core.worktree` to `O`. Put `/inner/` in the repository's administrative `info/exclude`, so the valid outer tree is clean while the inner tree retains its credential. Git in `I` now reports `O` as its top and a clean status.
4. Add a second credential in `I` and stop from `I`, then stop from `R`.

Frozen-v4r23 result:

```text
first BLOCK memory 7e46ceab22187f03/u:1
top .../outer
status ''
second allow:... memory -
deep-diff-forge: Whether the working tree ".../outer/inner" ... could not be found out
  (Git names ".../outer" as the working tree there, which is not this tree) ...
  A failure there has blocked a stop of this session and stands ...
third SILENT memory -
<silent>
```

Both credentials remain in `I`, whose `.git` file and administrative directory still exist and which `git worktree list` still names. This breaks the round-14 silence rule, the “Git answer” invariant, and the remembered-tree invariant. It is stronger than HR14-L-4's reported sibling-to-main redirect: no record is removed and the inner directory remains a valid working tree throughout.

Reproduction: `/var/home/louranicas/.cache/ddf-t/vesper-scratch/v4r23-core-worktree-nested.py`, SHA-256 `d7507a2b6fbd0e013c3762d4b51c9844cecb46ad4227cdb727570e393460c37d`. It runs sequentially with a private HOME set inside the driver and changes only scratch trees.

The safe invariant is a pair, not containment alone: Git's canonical top and its `git_dir` must identify the same main/linked tree in the repository's administrative listing. For a linked result, require the reported top to be the top recorded by that same administrative directory/id. Any mismatched pair should be `Elsewhere::Failed`, retain memory, and judge neither path as that tree. A regression should use two genuine nested linked trees and require UNJUDGED plus retained inner-tree memory after the redirect.

### VD4R23-L-1 — ordinary workflow: a clean v9 state is not written back as v10

`state_from()` deliberately parses v9 into the same in-memory `State` as v10 with an empty `trees` map. `Session` retains only that parsed state, not the schema read. `Session::keep()` returns without writing whenever `kept == state` and no list changed (`hook_run.rs:1103-1112`). Therefore a clean hook that reads v9 and changes nothing cannot distinguish it from an already-v10 state and leaves the file at v9.

I created an ordinary clean session state, changed only its schema marker to v9 and removed the v10-only `trees` member, then ran a clean stop through the frozen binary:

```text
stop SILENT
schema deep-diff-forge.hook-state.v9
trees_member False
```

Reproduction: `/var/home/louranicas/.cache/ddf-t/vesper-scratch/v4r23-v9-clean.py`, SHA-256 `4b7b8f114e82a79a14ae44b530395be4538609e99e1703baa9a23982d0e63252`.

This is Low: the state is accepted, and the first semantic state change does rewrite it. I also simulated a v9 state with a remembered linked-tree failure; a stop from the main tree named that dirty tree, populated `trees`, and wrote v10, after which removing both identity records remained UNJUDGED with memory retained. The defect is the unqualified migration promise, not rejection or immediate memory loss. Preserve a `needs_upgrade` bit from `load/state_from` and make `keep()` write when it is set, or narrow the claim to “written as v10 the next time the hook otherwise writes the state.”

## Requested finding status

- **VD4R22-M-1 — closed.** My original two-case driver now returns `FAIL STANDS` for the intact relative/gone linked tree and UNJUDGED with memory retained after both identity records are removed. The relative path is anchored and walked before placement.
- **HR14-L-1 — closed.** A ceiling that hides the enclosing home repository again leaves session start, advice, and stop silent with no state; a ceiling at a linked tree's top is removed once the project is known, so the linked credential BLOCKs and then stands.
- **HR14-L-2 — closed.** With both linked-tree records removed, stops from that still-standing directory are UNJUDGED and stops elsewhere name it; memory remains. The v10 map's parsing and size checks are conservative. Its documented non-UTF-8/16-KiB/256-KiB omissions remain deliberate limits, not hidden claims.
- **HR14-L-3 — closed.** A live btrfs subvolume inside a linked tree is UNJUDGED with Git's boundary words and the linked tree identified as possibly containing the session directory; stopping from the linked top judges both credentials. The narrowed rule is discussed below.
- **HR14-L-4 — not closed as a class.** The exact linked-to-main redirect is UNJUDGED and retains memory, both from that tree and later from the main tree. `VD4R23-M-1` bypasses containment with a redirected outer ancestor and reaches silent/forgotten again.

## View of the narrowed boundary rule

Narrowing the special boundary treatment to a **linked** tree lying across the named boundary is correct. The main project tree is judged at every stop regardless of where its `cwd` lies, so a boundary inside the main tree does not hide the project's patch. Treating every boundary message as uncertainty would make essentially every ordinary outside-repository directory below `/var` UNJUDGED rather than Git's normal “no repository.”

For a linked tree, the project does not otherwise judge its patch; the named mount/subvolume point at or below that tree is therefore the useful distinction. The live btrfs reproduction confirms the intended branch. An unlisted remembered tree is independently caught by its recorded directory after placement, so it does not depend on the current linked list's boundary test. I did not create a privileged mount or test a boundary whose mount-point text cannot be decoded; the latter falls back to resolved `cwd`, conservatively matching any linked tree that contains it.

## Claims

1. **Broken.** The third step of `VD4R23-M-1` is silent while the valid inner tree still contains two unjudged credentials.
2. **Held for the paths tested and inspected.** `standing_directory()` is now the single first resolver, anchors relative paths, walks missing/file components, and returns `Unknown` conservatively on lookup failure. The exact missing-relative and file-shaped paths are covered by the new implementation/tests; I independently reran the former.
3. **Broken.** “Contains `cwd`” does not establish that Git's top and administrative directory describe one tree. The mismatched outer-top/inner-admin answer is believed in `VD4R23-M-1`.
4. **Held in the live ceiling cases and source audit.** Project discovery keeps the caller's ceiling; project-scoped calls remove absolute entries at or under the resolved main/linked tops. I found no hook Git call that bypasses the project-scoped deadline after placement.
5. **Broken.** The inner tree's directory stands, remains listed, and is not clean, yet the outer tree's clean reading is tagged with the inner id and erases its failure. The next stop is silent.
6. **Broken narrowly.** v9 is accepted, every non-v9/v10 schema is refused, and an eventual write uses v10. A clean v9 state is not written back at that hook, contrary to the claim.
7. **Broken by claims 1, 3, 5, and 6.** The replacement fixes otherwise hold in the reproduced cases; the new source and documentation sentences about a Git answer and immediate v9 rewrite are wider than the implementation.

## Verification performed

- Read the request, round-22 report, complete fourteenth independent review, CHANGELOG/README/SECURITY changes, the full production diff, and the new integration/unit tests.
- Confirmed branch, exact HEAD/parent, clean pre-report worktree, changed-file scope, and clean `git diff --check`.
- Hashed both frozen binaries and the real judge.
- Audited `Deadline::with_project`, ceiling filtering, `find`/boundary parsing, `standing_directory`, `elsewhere`, linked-instance pairing, other-tree reading, unlisted-tree retention, v10 parsing/serialization/caps, recording after a stop, and the save/no-change path.
- Reproduced sequentially on the frozen release binary with the real judge:
  - both cases of `VD4R22-M-1`: fixed as described above;
  - HR14-L-1 hidden-home and linked-ceiling cases: fixed;
  - HR14-L-2 with both records removed: fixed, memory retained from inside and elsewhere;
  - HR14-L-3 with a real unprivileged btrfs subvolume: fixed, then judged from the linked top;
  - the exact HR14-L-4 sibling-to-main redirect: fixed, memory retained;
  - `VD4R23-M-1`: BLOCK, then a warning while memory is erased, then SILENT;
  - v9 with a blocked linked tree: changed to v10 with its tree recorded; clean v9: remained v9 (`VD4R23-L-1`).
- Reviewed the coordinator's frozen-binary records for `verify5` through `verify10`, eight selected eleventh-review drivers, the round-21/22 reproductions, and the fourteenth-review scripts; I did not count those coordinator runs as independent execution.

## Not examined

- I did not build, run Cargo, run the workspace/gate, or execute the full integration suite; frozen binaries and supplied artifacts were used as required.
- I did not independently rerun the full historical matrices, timing/load-sensitive tests, five repeated `agent_hook` suites, or mutation campaign.
- I did not stress the v10 map to its 16-KiB-per-path or 256-KiB-total limits, create non-UTF-8 tree paths, corrupt/truncate the state on disk, reuse an instance id, or replace a recorded tree with a symlink. I inspected their parser and focused tests.
- I did not create a privileged mount, make a cross-filesystem boundary inside the main tree, exhaust linked-tree enumeration, or test localized/non-UTF-8 mount-point output.
- I did not execute full rebase, notes-merge, LFS/annex, or every submodule Git-directory workflow; I inspected the changed logic and the supplied submodule run.
- I did not re-review the pre-round-20 branch, daemon/TUI behavior, or any remote forge. Nothing was accessed at GitHub.

STATUS: COMPLETE
