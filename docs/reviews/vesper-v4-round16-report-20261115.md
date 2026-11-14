# Vesper review V4, round 16 — 2026-11-15

## Verdict

**NO-GO** on `8746cd7` and on `agent/loop` as a whole (`d4dbc91..8746cd7`).

The three round-15 findings and both requested independent-review findings are closed. One ordinary workflow still breaks the new line-by-line “one block” claim: the state globally keeps only 2,048 remembered lines. Nine linked worktrees with 256-line failures evict the first tree's entire memory, so its unchanged failure blocks a second time when that tree is revisited. I also found one low, deliberate metadata manipulation that changes a tree's identity and repeats its block.

Reviewed exact commit `8746cd72565a724a84714a37ab73a96087c8dc9f`. Frozen release SHA-256: `6936ce8fc60d4bce21439883c8f7461b0fca4f5a9d5d0576813bf0c2175b3afd`.

## Prior findings

- **VD4R15-M-1: closed.** The least-recently-asked slow-lane and fast-lane commits are reserved before asks that may terminate the phase, and the judge leaves time for them. In the supplied real-judge reproduction, the credential had five failed calls; with three newer commits and the probe returning no verdict, stop six submitted the credential, received its real failure, and blocked. Stop seven showed that failure standing while the three unresolved commits remained named.
- **VD4R15-M-2: closed.** A moved linked worktree retained its failure and stood. Removing it and adding a new tree at the same path made the identical failure block, then stand. Removing that tree for good removed its remembered failure. The key used for a failure no longer contains the display path.
- **VD4R15-M-3: closed.** Two concurrent first stops no longer both judge and write: in the supplied four-second slow-judge reproduction, the first blocked and the second returned UNJUDGED because another stop was running.
- **HR7-H-1: closed.** This is the replacement half of VD4R15-M-2. The replacement at the reused path blocked as a new tree, including with identical failing text.
- **HR7-M-1: closed subject to VD4R16-M-1.** With two failing files, repairing one left the other standing; adding a third blocked and separated “New at this stop” from “Stands beside it”; restoring the first blocked because that line had gone and returned. The per-line representation fixes the reported partial-repair case while its entries remain in state.

## New findings

### VD4R16-M-1 — ordinary workflow: the global 2,048-line state cap forgets unchanged failures, which then block again

`failure_lines` keeps as many as 256 lines for one unit. `remember` appends those lines to `State.blocked`, then unconditionally enforces `BLOCKED_CAP == 2_048` with:

```rust
let over = state.blocked.len().saturating_sub(BLOCKED_CAP);
state.blocked.drain(..over);
```

Nothing records that lines were evicted, and the block-once decision reads only this truncated list. The new per-line representation therefore makes the existing global list cap reachable after only eight full-size tree failures.

I reproduced this with the frozen release binary and real judge in `/tmp/vesper-v4r16-blocked-cap/` (`/tmp/vesper-v4r16-blocked-cap.sh`). One session created nine linked worktrees. Each tree had 256 Rust files, each producing its own credential failure line. A stop was made from each tree:

```text
tree=1 BLOCK remembered=256
tree=2 BLOCK remembered=512
...
tree=8 BLOCK remembered=2048
tree=9 BLOCK remembered=2048
revisit=1 BLOCK remembered=2048
```

The first tree was unchanged and had already blocked. Visiting the ninth tree evicted its 256 remembered lines; revisiting it therefore classified all 256 as new and blocked again. State was readable and successfully written at every stop. This contradicts Claim 3 and the README's unqualified statement that lines which have blocked stand while present.

This is conservative rather than a bypass—the result is an extra block, not a missed new failure—so Medium is appropriate, as for HR7-M-1. It is nevertheless a practical regression in the promised workflow: a session that moves among several generated/vendor-heavy linked trees can repeatedly be told to repair unchanged work already left to a person.

Recommended fix: do not silently evict remembered lines needed by live tree instances. A bounded fallback can collapse an evicted unit (or the entire remembered set for that unit) to a digest/count marker, as failures over 256 lines already do, so unchanged content still stands and any change blocks conservatively. At minimum, state and documentation must make eviction visible and narrow Claim 3.

### VD4R16-L-1 — deliberate evasion: touching `commondir` makes one unchanged tree a new instance and repeats its block

`instance_of` includes the `commondir` file's modification time. In `/tmp/vesper-v4r16-touch-instance/` (`/tmp/vesper-v4r16-touch-instance.sh`), an unchanged linked-tree credential produced:

```text
first=BLOCK
second=ALLOW
after_touch=BLOCK
unchanged_again=ALLOW
```

The only intervening operation was `touch <admin-dir>/commondir`. No worktree content, Git ref, index, administrative-directory inode, or `commondir` inode changed. The old instance was then forgotten as unlisted and the same failure blocked under the new ID.

Directly touching Git administrative metadata is deliberate evasion; backup/restore software that preserves contents but changes this mtime could reach the same result accidentally. The cost is only a repeated conservative block. Consider basing the stable half of the instance on device/inode identity and using mutable timestamps only as an explicitly documented generation heuristic, or document that administrative metadata changes can reset failure memory.

## Claims

1. **Round 14's silence rule — held in reviewed and reproduced cases.** The supplied transition runs named every standing, unresolved and held unit, and removed-tree cleanup became silent only after nothing remained open. I did not repeat the 900 randomized sessions.
2. **Every commit without a verdict is asked in bounded turns — held in source and focused reproduction.** The two reserved asks precede terminating failures, last-asked stop numbers rotate each lane, never-asked commits follow, and the credential starvation reproduction closed. I did not independently exhaust every mixture of N commits, deadline exhaustion and probe behavior.
3. **One block per failing line — broken by VD4R16-M-1 and, under deliberate metadata manipulation, VD4R16-L-1.** Partial repairs, count growth, disappearance/reappearance, moves, and tree replacement otherwise behaved as claimed.
4. **One writer — held on the tested local filesystem.** Concurrent first stops serialized; advice writes reload under the same session lock and merge only advice-owned fields. I did not test stale locks, a filesystem without `flock`, the HOME-is-a-worktree first-hook exception, or an advice hook that exhausts its 0.3-second wait.
5. **Bounded message and one deadline — held by source review and focused tests.** Counting lines are reserved before upper detail and the focused hook suite passed. I did not rerun the live 70-commit message reproduction or independently measure every maximal combination.

## Verification performed

- Read the full request, round-15 Vesper report, seventh independent review, changed hook/state/message code, relevant documentation and focused tests.
- Verified exact HEAD and frozen binary hashes; `git diff --check 065c74f..8746cd7` was clean.
- Ran the supplied `verify7.sh` with the frozen release binary and real judge in a fresh scratch directory. It closed the move/replacement, partial-repair, concurrent-first-stop and starvation cases.
- Reproduced VD4R16-M-1 and VD4R16-L-1 independently with private HOME values exported inside the scripts.
- Checked the hive slot immediately before Cargo; it was free. Archived exact commit `8746cd7` and ran `cargo test -p deep-diff-forge-cli --test agent_hook --locked` in `fedora-toolbox-44`, with the toolbox launcher using the real HOME and the private HOME set only inside the container command: **95 passed, 0 failed**.

## Not examined

I did not run the full 2,141-test workspace suite, CLI unit suite, 32-step gate, mutation run, randomized 9,000-stop sessions, daemon soak, or every earlier hook reproduction. I did not test Git other than the installed version, another operating system, NFS or another filesystem without working `flock`, inode reuse within one timestamp tick, bare repositories with linked trees, submodule worktrees, full disk, ranker hashing under high load, waiver-only failures, or all message-cap combinations. I did not audit unrelated branch history outside the hook path in this commit.

STATUS: COMPLETE
