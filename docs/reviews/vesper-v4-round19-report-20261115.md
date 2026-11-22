# Vesper review V4, round 19 — 2026-11-15

## Verdict

**GO** on `07df120` and on `agent/loop` as a whole (`d4dbc91..07df120`).

VD4R18-L-1 and HR10-M-1, HR10-M-2 and HR10-M-3 are closed. The unavailable-baseline state is persistent and conservative: it still judges the current tree, never adopts a later baseline, and never treats a session commit as already present. Failed reads preserve memory. Exact equality with the index makes a remembered whole-index mark cover the staged failure while that index is untouched. Git, rather than path containment, identifies the tree containing the stop.

I found no new High, Medium or Low finding. Reviewed exact commit `07df12074d68d4c3246888604b0ea316ac79b799`; frozen release SHA-256 `09bbaeaff51f8e60749c039d8d600f4ac6dd55a052b8f0b86bf5eb7516fd9b4e` and debug SHA-256 `0768ef5f3d92b529f796e1062dd83abef9d307c0dc44e947c60e80d9fa484106`.

## Requested closures

- **VD4R18-L-1: closed.** `record` retains project identity, base and an explicit `no_baseline` reason when `named_commits` refuses the over-256-worktree selection. `Session::tips` thereafter returns `Tips::Unavailable` without relisting. `judge_units` still judges the current working tree and emits an UNJUDGED commit unit. In a fresh frozen-binary/real-judge run with the main tree plus 256 linked trees, an uncommitted credential in the main tree blocked. After I committed it, the session's commits remained UNJUDGED. I then removed two trees, taking the count below the limit; another stop remained UNJUDGED for the original baseline reason. It neither acquired a later baseline nor passed the session commit as one that had been there.
- **HR10-M-1: closed.** `Reading::{Nothing, Found, NotRead}` carries the distinction through the uncommitted tree and staged paths. Only `Nothing` forgets; `NotRead` becomes an explicit UNJUDGED unit and preserves remembered lines. In the supplied real-judge unreadable-file sequence, the staged credential blocked once, stood unchanged, survived the unread stop, and stood on both later readable stops with the index untouched.
- **HR10-M-2: closed.** `seen` compares the exact whole-index mark with the remembered staged mark before subtracting tree lines. In the independent review's `km.py c1 300` sequence, the initial staged failure blocked once. Restoring 100 failing lines in the tree blocked as the tree's change. Repairing 50 and then the remaining 50 while never touching the index produced FAIL STANDS/HOLD, never a second staged block. The index tree remained byte-for-byte the same as at the first stop.
- **HR10-M-3: closed.** `elsewhere` canonicalizes `cwd` and asks Git which working tree contains it before considering whether it is under the common directory. The frozen-binary run with `repo.git/main` and `repo.git/feature` judged a credential from inside `feature`: BLOCK, then FAIL STANDS. Stops from `main` named the standing failure rather than pretending it had been judged there. The exact integration test additionally covers a linked tree nested under another tree's `.claude/worktrees`, and directories that are actually Git administration remain no tree.

## New findings

None. There is no `VD4R19-H-1`, `VD4R19-M-1` or `VD4R19-L-1`.

I considered the remaining `Found::Failed` fallback for a directory beneath the common Git directory. It is needed because Git rejects queries from actual administrative directories; valid working trees nested there are now found before that fallback. Corrupting a linked tree's Git administrative data can still make Git unable to identify it, but that is deliberate repository corruption rather than an ordinary round-19 regression, and I did not classify it as a finding.

## Claims

1. **Held.** The silence invariant remains explicit in `judge_units` and its outcome assembly: every non-pass, unavailable commit baseline, standing failure, unread unit or unjudged project contributes a unit or note. The over-limit live run was BLOCK/UNJUDGED, the unread run said what stood, and the exact integration silence test passed.
2. **Held.** Commit asks remain bounded and persisted only when run. The scheduling code reserves time for asks still owed after each run, caps the initial owed set, and retains later turns. The focused tests for “asked and only then,” the zero/seven-second boundary, a hanging pair followed by a failing third commit, and eventual in-turn submission passed. Two deadline-sensitive tests failed only when all 112 integration tests ran concurrently on this loaded host; each passed when rerun alone from the same exact archive (`106.25s` and `98.70s`). Their failure messages expressly showed the test's synthetic stop arriving at commits with too little wall time, not an ask recorded without running or an unbounded product stop.
3. **Held.** `Reading::NotRead` preserves rather than clears memory. A whole staged mark covers the exact current index independently of how the working-tree failure overlaps it; a changed index does not receive that equality. The live unread and 300-file sequences, plus the focused changed-index and large-failure tests, behaved accordingly. I found no unchanged failure that blocked twice without an earlier explicit memory-loss condition.
4. **Held.** The working tree containing `cwd`, as Git identifies it, is constructed and judged first. The recorded project tree follows only when distinct. Actual Git administrative directories add no false tree. Judge, Git and status calls continue to receive sub-deadlines derived from the single stop deadline. The bare-inside, nested-tree, current-tree ordering, slow second-tree and hanging-commit tests passed; I observed no run exceeding the product's deadline.
5. **Held.** The persistent `Tips::Unavailable` state separates unknown commit attribution from current uncommitted work. The live 257-tree run blocked on the uncommitted credential, kept the later commit UNJUDGED, and did not change its answer when the count fell below 256.
6. **Held.** Session state still has one lock/writer. The state stores bounded line identities/aggregate marks and bounded noticed/reported sets; messages reserve whole counting lines and condense excess notes. The exact concurrent-first-stop and bounded-message integration tests passed. I found no partial counting line or unbounded per-stop message introduced by this commit.

## Attack results

- **A — unavailable baseline:** no bypass found. A session commit cannot pass as pre-existing; falling below the cap does not create a baseline mid-session. Uncommitted work is still judged.
- **B — three-valued reads:** no conflation found in the tree/index path. `NotRead` propagates through `uncommitted`, `tree_units` and `seen`; `Nothing` alone clears. A changed failure after an unread stop is compared with preserved memory on the next readable stop.
- **C — untouched/changed index:** no repeat block with an untouched index in the 300-file reproduction. Exact mark equality is deliberately lost when the index changes, allowing genuinely new staged content to block.
- **D — `elsewhere`:** actual directories in the common Git directory added no tree; linked trees inside a bare repository and inside another tree's administrative-looking directory were identified and judged; foreign nested repositories and submodules remain named and excluded as separate repositories by design.
- **E — time:** no product deadline breach or owed-ask starvation found. The full concurrent test run exposed host-load sensitivity in two synthetic wall-clock tests, and both passed alone. The exact tests for a clean project tree donating its time and a changed second tree receiving its reserved time passed.
- **F — branch-wide:** no other defect found that I would withhold from merge.

## Verification performed

- Read the request, round-18 report, tenth independent review, exact `5c387a3..07df120` source/test/docs delta and the branch-wide state, tree-identity, scheduling, memory and message paths implicated by the claims.
- Verified clean exact HEAD, both frozen binary hashes and clean `git diff --check 5c387a3..07df120`.
- Ran the supplied `verify10.sh` against the frozen release binary and real forge-warden with private HOME exported inside the script: bare-repository trees and unread staged memory held.
- Ran the independent review's `km.py c1 300` against a policy admitting the round-19 binary: all untouched-index repeat stops stood rather than blocked.
- Ran a fresh 257-worktree start/commit/count-fall reproduction under cache-backed scratch with the frozen release binary and real judge.
- Made an exact `git archive 07df120` under `/var/home/louranicas/.cache/ddf-t/vesper-scratch` with its own target directory. I checked the hive slot immediately before every Cargo invocation; it was free. Toolbox retained the real HOME. The complete `agent_hook` run produced 110 passes and two load-sensitive failures; both failed tests then passed in isolated exact reruns, making all 112 tests pass across the run and reruns.
- Did not kill processes by name or modify repository content other than this report.

## Not independently examined

I did not rerun the complete workspace suite, CLI unit suite, clippy, the 32-step local gate, mutation testing, release packaging, or every historical round-5-to-round-9 script. I did not test a filesystem without birth time, forced inode reuse, NFS/FUSE/no-`flock`, non-UTF-8 worktree names, state/disk exhaustion, every Git version/configuration, a submodule specifically nested inside a bare repository's linked tree as a separate live combination, or deliberate corruption of Git administrative files. I did not reproduce every deadline edge with the production forge-warden; the focused exact-commit integration tests supplied those controls.

STATUS: COMPLETE
