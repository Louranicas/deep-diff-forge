# Vesper V4 round 15 review (2026-11-15)

## Verdict

**NO-GO** on `065c74f` and on `agent/loop` as a whole (`d4dbc91..065c74f`).

The reported round-14 retry cap is fixed, the reported HR6 failure-memory workflow is fixed, and fail-plus-hold rules are retained and named. Three ordinary cases still break the new claims: fast-lane no-verdict commits can starve every slow-lane commit forever; a filesystem path is not a stable worktree-instance identity; and two concurrent stops which are both the session's first hook bypass the lock and both block/write concurrently.

I found no new High or Low finding.

## Prior findings

- **VD4R14-M-1: closed in its reported form.** After five counted temporary failures, the credential commit is submitted again. In my direct derivative of the round-14 reproduction, its sixth call reached the real judge and blocked. It is no longer discarded merely because its count reached five. VD4R15-M-1 is a separate starvation path before the slow-lane ask is reached.
- **HR6-M-1: closed in its reported staged/edit/cwd-not-looked form.** Remembered failures are retained when their tree was not read, and the staged failure contained by the tree failure is not treated as a second unit. The focused tests and source paths hold the reported repair. VD4R15-M-2 shows that the new association is still based on a mutable/reusable display path rather than a stable worktree instance.
- **HR6-M-2: closed.** A fail-and-hold verdict retains the held rule names, marks them as held for a person, repeats them on standing lines, and excludes them from failure identity. The real-document unit coverage and message/state code agree.

## New findings

### VD4R15-M-1 — ordinary workflow: three fast-lane no-verdict commits can starve a slow-lane failing commit forever

`sorted_commits` orders all ranks through five before the one selected slow-lane commit. `judge_commits` asks those fast-lane commits first. Once three asks fail, it probes the chain; if the probe also gives no verdict, `waits` is set and the selected slow-lane commit is skipped. Because no verdict proved the chain working, the three fast-lane attempts do not count. They therefore remain ahead of the slow lane and repeat the same result at every stop.

I reproduced this with the frozen release binary and real judge behind a small wrapper:

1. A credential commit receives five temporary no-verdict answers while harmless commits pass, leaving it at `n5`. A sixth submission would reach the real judge and fail.
2. Three later harmless commits receive structured temporary no-verdict answers; the generated probe does too.
3. At each of four further stops, those same three commits are asked and the probe fails. The credential call count remains exactly five at every stop. Its patch is never submitted again.
4. Every stop allows and names four unjudged commits, including the credential as `no verdict in 5 attempts; it is asked about again in its turn`. The stated turn never arrives.

Artifacts: `/tmp/vesper-v4r15-fastlane-starve/`; `repro.sh`, `calls`, `starve1.out` through `starve4.out`, and the judged state under `home/`.

This is the same security-relevant outcome as round 14—an available credential FAIL never blocks—although it remains loudly `UNJUDGED`, so Medium remains appropriate. Reserve and perform the selected slow-lane ask before fast-lane failures can terminate the phase. If “one slow-lane commit per stop” is the price chosen for never giving up, that one ask must actually be reserved at every stop.

### VD4R15-M-2 — ordinary workflow: path hashing confuses a moved tree and a replacement tree

The state prefixes a failure with `tree_id(top)`, which is only the first 16 hex digits of the path hash. The failure key for a `WhileNew` unit also hashes `unit.what`, whose label contains that same path. Consequently:

- **Move:** the same physical linked worktree, with the same unchanged credential failure, gets a new tree ID and failure key after `git worktree move`; it blocks a second time.
- **Replacement:** after a failing linked worktree is force-removed/pruned and a different linked worktree is created at the same path, an identical new credential failure gets the old tree ID and key; it is reported as `FAIL STANDS` and never blocks for the new worktree.

Both are reproduced with the frozen binary and real judge under `/tmp/vesper-v4r15-tree-reuse/`. `repro.sh` shows `first_block=1 replacement_block=0 replacement_stands=1`. `distinct.sh` creates another physical tree and blocks there; `move-distinct.sh` then moves that unchanged tree and it blocks again.

The association needs a stable worktree-instance identity, not its current display path. The failure key should distinguish staged from uncommitted units without embedding the path already supplied by that worktree identity. A new worktree generation at a reused path must not inherit the old generation's failures, while a moved existing worktree must keep them.

### VD4R15-M-3 — ordinary workflow: two concurrent first stops bypass the lock and both block/write

`stop_turn` returns `Unlocked` whenever the session state file does not exist. A stop is allowed to be the session's first hook, so two concurrent first stops both observe no state and both run. This contradicts “one stop of a session at a time” and also permits the same failure to block twice before either write becomes visible.

I reproduced this with two simultaneous first-hook stops, the frozen binary, and a judge wrapper that sleeps three seconds. Both judge invocations started before either ended; total elapsed time was three seconds rather than serialized time; and both outputs were `decision: block` for the same unchanged credential failure:

```
elapsed=3s starts_before_first_end=2 one_block=1 two_block=1
```

Artifacts: `/tmp/vesper-v4r15-first-race/` (`repro.sh`, `calls`, `one.out`, `two.out`).

The session's deterministic lock file should be taken by every stop whenever the state directory is available, including a stop that will create the initial state. The absence of prior state does not mean two initial stops have nothing to overwrite; both are about to create that state and its side files.

## Claims

1. **Round 14 silence rule: held in the reviewed and tested cases.** Every reproduced non-pass remained named; I found no silent known non-pass.
2. **Nothing is given up: broken by VD4R15-M-1.** The slow-lane commit remains in state and text but is never submitted while three non-counting fast-lane asks repeatedly stop the phase.
3. **One block per failure: broken by VD4R15-M-2 and VD4R15-M-3.** Moving one unchanged worktree blocks its failure twice; replacing a tree at the same path suppresses a new block; concurrent first stops both block the same failure. The ordinary staged/tree shuffle case from HR6-M-1 is held.
4. **Bounded and one stop at a time: partly held, partly broken.** Message finishing enforces the 9,000-character cap, judging retains one stop deadline, and a lock serializes stops once state exists. VD4R15-M-3 breaks the unqualified concurrency claim before the first state file exists.

## Verification

- Reviewed exact HEAD `065c74fcbc37bceb9d6ebf28d6904db22c5d1994`; the worktree was clean before the report.
- `git diff --check c6c4f28 065c74f`: clean.
- Frozen SHA-256: debug `1921585e4ff15acb662ddb798b418874b9837dccf87ec88be3d6f2bb7b65e826`; release `8359334db6010203fe1c3bb4d96f430d8a74e2653127dcbfe028a8351c4af1eb`.
- Reproductions used private HOME values exported inside their scripts. No toolbox, Podman or Distrobox launcher received a changed HOME.
- Checked the hive slot immediately before Cargo; it reported free. Ran `cargo test -p deep-diff-forge-cli --test agent_hook` in `fedora-toolbox-44` with a private HOME only inside the container command: **84 passed, 0 failed**.

## Not looked at

- I did not run the full 2,126-test workspace suite, CLI unit suite, complete 32-step gate, or all prior real-judge workflows.
- I did not exhaustively test crashes between every side-file/state write, unwritable-state variants of the slow lane, every stash/amend/rename sequence, or every worktree administrative transition.
- I did not independently stress the 9,000-character cap across every combination of blocker, held rule, note and judge text; I reviewed its final enforcement and relied on focused tests.
- I did not test stale locks on filesystems without working `flock`, advice-hook/stop races, other operating systems, packaging, or unrelated branches.

STATUS: COMPLETE
