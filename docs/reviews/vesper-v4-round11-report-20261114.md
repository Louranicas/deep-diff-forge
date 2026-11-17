# Vesper review: V4 round 11 (2026-11-14)

Reviewed: committed snapshot `69e2fe8..ffa0f8e` on `agent/loop`, using the frozen round-11 release binary named in the request. Every hook/binary reproduction used the private home `/tmp/vesper-v4r11-home.qbztIc`; the review clone admitted the frozen ranker digest in its committed base policy. I wrote only this report in the repository.

## Verdict

**NO-GO.** VD4R10-H-1 is closed. The concrete HR2-H-1 branch-switch and stale-index cases are closed, and HR2-M-1 through HR2-M-4 and HR2-L-1 through HR2-L-3 are closed as written.

I found one new High ordinary-workflow gap in the broader HR2-H-1 promise. A ref tip is represented by only the cumulative patch from the session base (or its merge base) to the tip. If an intermediate session commit introduces a credential and a later ordinary `git revert` removes it, the cumulative tip tree equals the base. The implementation discards the empty patch, Stop is silent, and the judge is never invoked, even though the current branch still reaches and retains the credential-bearing commit. This directly contradicts both the stated “every commit” contract and the hook's named purpose: guarding a credential the agent commits, for which reverting the tree does not remove the credential from Git history.

## VD4R10-H-1 — closed

The project state now records the device and inode of the Git common directory. `locate` accepts a repository only when that identity matches, and it can find the original after a move through the hook input's `cwd`.

I reran the exact move-and-clone-back shape with the frozen binary: start in `/tmp/vesper-v4r11/moved`, modify `lib/a.rs`, move the repository to `moved-original`, clone a clean repository back at `moved`, and Stop with `cwd` in `moved-original`. The hook followed and attempted to judge the modified original and said:

```text
The project this session began in is no longer at "/tmp/vesper-v4r11/moved":
it was found, and judged, at "/tmp/vesper-v4r11/moved-original".
```

My host-side real-judge run was UNJUDGED because that environment has no enrolled Git allowlist, but it was visible and referred to the modified original; the clean replacement did not displace it. That is the required closure of the silent identity bypass.

## Second independent review findings

- **HR2-H-1 — the reported cases are closed, but the general finding is not fully closed.** A commit on a branch the session leaves is now named under `refs/heads/feature`; the stale-index case produces separate named working-tree and `HEAD` answers. The implementation also covers `HEAD`, branches, remote-tracking refs, recursively peeled tags, and stash tips, with explicit UNJUDGED outcomes beyond its bounds. `VD4R11-H-1` shows that a tip's single cumulative patch is not equivalent to judging every newly reachable commit.
- **HR2-M-1 — closed.** State schema v2 bounds each list, stores a failure as its SHA-256 plus item count, refuses malformed/oversized existing state instead of treating it as absent, and refuses to write a state it could not read back. The largest configured lists remain below the 4 MiB read cap by construction and test.
- **HR2-M-2 — closed.** State records whether SessionStart established the base. I reproduced a session whose first hook was PostToolUse after a commit: PostToolUse was silent, but the following Stop explicitly said that the session start was not recorded and that earlier committed work was not judged.
- **HR2-M-3 — closed.** I reproduced both ordinary visible cases. Stop from a second repository named it and said nothing there was judged; an untracked nested repository with no commit was left out and named instead of making the project patch fail. The implementation separately asks Git for dirty submodules and names their uncommitted work.
- **HR2-M-4 — closed.** The state retains up to 256 failure digests for the session, so the same failure becomes `FAIL STANDS` across later turns instead of being cleared after one stand. A changed identity blocks again. Messages describe the range as including commits pulled or made by others and distinguish this session's repair duty from work for a person.
- **HR2-L-1 — closed.** The gate now checks the state directory and both SessionStart and PostToolUse registrations, in addition to the stop registration and timeout.
- **HR2-L-2 — closed.** State-access advice is passed through the bounded once-per-session notice set where state can be retained; a Stop still says the reason. An unreadable existing state is trouble and is never overwritten as a fresh session.
- **HR2-L-3 — closed.** The frozen SessionStart output named the recorded project path (`/tmp/vesper-v4r11/revert3`) as well as the base.

## New finding

### VD4R11-H-1 — ordinary workflow: a later revert makes a reachable credential commit disappear from the one cumulative ref patch

The ref discovery correctly notices a current tip with commits beyond the recorded tips (`session_commits`, `hook_run.rs:1587-1637`). But `commit_patches` then makes only one patch for that tip, from `merge_base(session_base, tip)` to the tip (`hook_run.rs:1647-1679`). When those two trees are equal, it maps the patch to `None` and omits the ref altogether (`hook_run.rs:1672,1680-1686`). `commit_patch` itself is a tree-to-tree diff (`judge_patch.rs:677-706`); it carries no intermediate commit history.

That optimization is sound for a net working-state rule, but not for the documented rule that every session commit retained by a ref is judged. It is particularly unsafe for `no-secrets`: the policy says a credential committed once is compromised forever, even after a revert.

Reproduction with frozen release SHA-256 `e5ca574a64fe348a73e0b444f66d3eb01689609521c0acc85dc29c85dd272f84`:

1. Clone the supplied base and commit the frozen ranker's digest, producing base `1a4114af91cf2176d133a7b21d25b08ca2bc3ca6`.
2. Run SessionStart for a fresh session.
3. Add `AKIAIOSFODNN7EXAMPLE` to `lib/a.rs` and commit it. PostToolUse reports `secret_like` as advice.
4. Run ordinary `git revert --no-edit HEAD`. The next PostToolUse says nothing.
5. Run Stop.

Observed:

```text
Stop exit:                 0
Stop output bytes:         0
working tree:              clean
base..HEAD diff:           empty
credential commit:         60ff892190c7690bfbdc4799639e0e34b9a58698
branch containing commit:  main
git show <commit>:lib/a.rs: contains the credential
```

This is an **ordinary workflow**, not deliberate evasion. Reverting a bad commit is the standard response to discovering it after commit. The advice did notice the secret before the revert, but advice never blocks; the contract says an ordinary workflow must not end in a silent Stop with unjudged work. The current branch still reaches the commit, so this is not the stated limit for a reset-away commit that no ref names.

The same class applies to any sequence whose intermediate commits violate policy but whose final tree cancels those changes, including a secret commit followed by a hand-written cleanup commit or a merged branch that introduces and later reverts it.

Recommended fix: enumerate commits newly reachable from each current tip and not reachable from the recorded tips, under explicit commit/patch bounds, and judge each commit's change rather than only the net tip tree. Merge semantics need an explicit conservative rule. If the history exceeds those bounds, name the ref as UNJUDGED. As a minimum safety fix, a tip with new reachable commits and an empty cumulative patch must be visible as UNJUDGED rather than discarded; that closes the silence but does not satisfy the “every commit judged” promise. Add the exact commit-plus-revert case with a judge-run count assertion.

I found no new Medium or Low finding.

## Stated limits and bounds

- The four-patch and sixteen-tip bounds are visible: excess work becomes named UNJUDGED. Deliberately adding refs can therefore avoid a block but cannot manufacture a silent pass; that is consistent with the stated non-boundary model.
- A commit genuinely made unreachable by reset and ref deletion remains a stated limit. VD4R11-H-1 is different because the active branch retains the commit in its ancestry.
- Another repository, a dirty submodule, and a nested repository are named rather than judged. That is an accepted ordinary-workflow limitation because it is visible.
- Device/inode reuse after deletion, linked worktrees the session never enters, editable hook state/settings, Git's English no-repository message, and `/proc` cleanup remain documented boundary limits. I found no contradiction beyond VD4R11-H-1.

## Other checks and tests

- `git diff --check 69e2fe8..ffa0f8e` passed. HEAD was `ffa0f8ea2ba2dcda4a3f4ea8f97513f4d5f2a480` on `agent/loop`.
- Frozen hashes: release `e5ca574a64fe348a73e0b444f66d3eb01689609521c0acc85dc29c85dd272f84`; debug `c335f75c4c83fe1766370e8398d69d0720b87b1e218a59315fe7faa3e1c8fbb5`.
- The branch-switch, stale-index, move-and-reclone, unrecorded-start, foreign-repository, nested-repository, and commit-plus-revert probes all used the private HOME and frozen release binary.
- The host-side real judge SHA-256 was `55bf9fe6d19d7f7b66ef7cc7f3056ff9ab7ba7dc10b0e310d59a2aafd33874c9`. On host it returned explicit UNJUDGED for nonempty patches because no Git allowlist was enrolled. This did not affect VD4R11-H-1: its defect is that the hook discarded the empty cumulative patch and never invoked any judge.
- The hive slot reported `free` before both attempted toolbox checks. I did not run Cargo: the shared `fedora-toolbox-44` container refused even `true` (`unable to find user` initially, then `crun: kill: Operation not permitted`). I did not mutate or restart that shared container.

## Not looked at / limitations

- I did not run the Cargo suites, Clippy, Miri, fuzzers, or the complete local gate because the required toolbox was unavailable.
- I did not obtain a sealed pass/fail from the real judge in the toolbox. The request's recorded real-judge runs cover the original HR2-H-1 cases; my routing probes obtained explicit host-side UNJUDGED outcomes where a patch existed.
- I did not run a live Claude Code session; I supplied the documented hook JSON and environment directly.
- I did not exhaust partial/shallow clones, alternates, replace refs, SHA-256 repositories, network filesystems, simultaneous hooks, or every merge topology.
- I did not independently time four real judge patches or a repository with 4,096 refs. I inspected the shared deadline and bounded ref/patch paths and read their tests.
- I did not rerun every malformed-verdict, ranker-seal, hostile configuration/attribute, or million-entry directory probe from rounds 1-10.
- I did not review changes outside `69e2fe8..ffa0f8e` or modify repository refs, remotes, index, configuration, or repository files other than this report.

STATUS: COMPLETE
