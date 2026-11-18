# Vesper V4 round 12 review (2026-11-14)

Reviewed: committed snapshot `ffa0f8e..f9d1c9d` on `agent/loop`, using the frozen round-12 release binary and the real `forge-warden` named in the request. I wrote only this report in the repository. Concurrent round-13 edits appeared in the worktree during the review, so the reported source tests were run from a separate `git archive` of exact commit `f9d1c9d` rather than from the changing worktree.

## Verdict

**NO-GO.** VD4R11-H-1 is closed, and the commit-based design closes the third independent review's two Highs and most of its Medium/Low findings. Real-judge runs caught the original commit-then-revert credential, an untracked-file stash, and an older stash entry. The commit and patch acquisition tests pass.

I found one new Medium ordinary-workflow defect outside the three fourth-review findings already assigned to round 13. The implementation intends not to block twice when an uncommitted failure is committed unchanged, but it remembers and compares only the judge's failure shape—not the patch digest that the state documentation says is present. After one uncommitted credential blocks, the agent can materially change that patch, leave one credential under the same rule/signal/path/item count, and commit it. The new commit is judged FAIL but is immediately classified as standing; it never blocks as a commit.

Per the request, I did not spend time rediscovering or assign duplicate IDs to the fourth independent review's three known failures: staged-only content is not judged; enough standing failures can consume the twelve displayed units and omit what blocks; and a no-verdict commit can stop later commits being judged. Those already make this snapshot NO-GO independently. My verdict on `agent/loop` as a whole is NO-GO.

## VD4R11-H-1

### Closed

I reproduced the original shape with the real judge in `/tmp/vesper-v4r12/commit-revert-2`: after session start, one commit added the example credential and the next commit reverted it, leaving a clean tree and an empty net change from the session base. The round-12 stop blocked and named the original commit `3975720374e0` as FAIL under `no-secrets`.

By reading, `session_commits` enumerates commits reachable from the session's current local sources but not from its recorded starting tips. `commits_beyond` returns them parents before children with their first parent, and `judge_commit` gives the judge `commit_patch(first_parent, commit)` rather than a ref-tip net patch. A later revert therefore cannot erase the earlier unit.

## Third independent review findings

### HR3-H-1 — closed

`named_commits` now includes the whole stash reflog, and the revision walk reaches all parents of each stash commit. The third-parent root commit made by `git stash -u` is consequently judged from nothing, and older stash entries remain sources through the reflog.

I reproduced both with the real judge. A stash of untracked `lib/new.rs` blocked on commit `45a4b36b2193`; after two later stashes, the older tracked credential stash blocked on commit `1282671e7b26`. Both appeared in the same stop and named their own paths.

### HR3-H-2 — closed within the new documented bound

The 4,096-ref name-ordered list is gone. Initial state records commit IDs, not ref names, in a separately sealed tips file. `named_commits` reads all relevant refs and reflogs, deduplicates IDs, and refuses visibly above one million commits rather than silently truncating by ref name. The integration test with 5,000 branches/tags, a later stash, and a late-named branch passed.

The one-million-ID/192-MiB bounds are large and fail UNJUDGED; I did not construct their maximum case.

### HR3-M-1 — closed

This is the same architectural closure as VD4R11-H-1: each commit is judged against its first parent. Both the real commit/revert reproduction and the integration case where a second commit merely removes the credential pass through the judge as separate patches.

### HR3-M-2 — closed

There is no four-patch or sixteen-tip selection and no ref-name priority. Remote-tracking-only commits are deliberately excluded from current sources; current local branches, tags, stash, worktree `HEAD`s, and selected reflogs feed one revision walk. Commits are processed oldest first until the shared stop deadline, and the remainder is counted for later stops. The 21-branch integration case passed and judged each commit once.

### HR3-M-3 — closed for repaired uncommitted failures

`remember` retains only blocked uncommitted failure identities present at the current successfully read stop. A stop that finds the failure gone forgets it, so reintroducing it blocks again. The corresponding integration test passed.

The distinct uncommitted-to-commit transition is broken when the patch changes without changing its failure shape; that is VD4R12-M-1 below.

### HR3-M-4 — closed

Repository identity is now the Git common directory's inode plus its recorded top-level path; the mount-time device number is gone. Moved/replaced projects produce UNJUDGED with the new-session remedy. The clone-at-path and changed-identity tests passed. Inode reuse/collision after replacement remains an explicitly documented limitation; I did not simulate a remount or reboot.

### HR3-L-1 — closed

An uncommitted failure identity no longer includes the collection of tree/ref labels around it, and a commit is remembered by its immutable ID. Switching branches or adding another judged unit therefore does not by itself make the same failure block again. Ref names no longer reach the identity or message.

### HR3-L-2 — closed

When the 32-entry said-once set is full, the stop emits a bounded “More things were left out or not judged” notice once. Its integration regression passed.

### HR3-L-3 — wording closed; remaining visibility is stated and accepted

A nested linked worktree is now identified as “another working tree of this project (a linked worktree),” and the message accurately says its commits are among session commits while its uncommitted change is judged only from inside that tree. The integration regression passed.

The advice hook still covers only the project's primary tree, and a linked tree the session is not in is not judged. README and SECURITY now state both. I treat that residual as the declared scope, not a new finding.

### HR3-L-4 — closed

No ref name is decoded or shown. `for-each-ref` output contains only object types and IDs, is read lossily for fixed ASCII fields, and the current tests cover whole ID collection. A non-UTF-8 ref name therefore cannot terminate recording merely through its name. I did not recreate that exact byte-name repository.

### HR3-L-5 — closed

Remote-tracking-only commits are not current session sources, and messages name a commit only by twelve hexadecimal ID digits. A hostile ref name cannot reach the model or consume a block. A fetched commit becomes in scope when a local source/history names it, as documented.

### HR3-L-6 — closed for the reported detached/worktree cases; remote-only push remains stated

The sources include every present worktree `HEAD`, each accessible worktree's `HEAD` reflog, and the stash reflog. Integration tests passed for a detached commit after leaving it, a commit made in another linked tree, and one that linked tree later left. A commit pushed directly by URL that ends up named only remotely remains outside the hook's sources and is explicitly listed as a limit.

### HR3-L-7 — closed

Current README/SECURITY describe the 4-MiB state file, the separate 65-MiB tips file, counted unnamed entries, `/clear` as a new session, the inode-only identity, local source set, and time-bounded oldest-first commit processing. Historical changelog statements are dated history.

## New finding

### VD4R12-M-1 — ordinary workflow: a changed patch inherits an uncommitted failure's “already blocked” status

The intended exception is narrow: if an uncommitted patch has already blocked and is then committed unchanged, the same content need not hold a second stop merely because its unit changed from tree to commit. The state and function comments say the remembered key is `FAILURE|PATCH`.

The implementation does not contain the patch component:

- `unit_failure` hashes only `failure_identity(rules, said)`—rule ID, signal, path, item count, and judge side-notes.
- `uncommitted_failures` stores that failure-only digest in `State.blocked`.
- `commit_unit` compares the newly judged commit's same failure-only digest with `state.blocked` and changes `Holds::Now` to `Holds::Never` on a match.
- The stop decides before `remember` observes that the working tree is now clean and removes the old blocked entry. The commit is then persisted as `ID:f`, so every later stop treats it as standing too.

I reproduced this with the frozen round-12 release binary and the real judge in `/tmp/vesper-v4r12/repro-blocked-digest.sh`:

1. An uncommitted `lib/a.rs` added one example AWS credential. The stop blocked under `no-secrets`.
2. Before another stop, I replaced it with a different example credential, added a new function, and committed the materially different patch as `e2c274f27a2d…`.
3. The next stop judged that commit FAIL but returned an allowed `systemMessage`: `FAIL STANDS` / `Commit e2c274f27a2d: FAIL, which stands`.
4. State then contained the commit as `e2c274f…:f` and no blocked uncommitted entry. Thus no later stop can give this commit its required first block.

This is ordinary attempted repair: after a block, an agent edits the implicated file and commits without an intermediate stop. It is Medium rather than High because the first block already explained the same failure class and the later stop visibly says FAIL STANDS; it is not silent. Nevertheless, it breaks the round's central guarantee that each failing commit holds the stop once, and the changed credential/content was never the patch that caused the earlier block.

Recommended fix: carry the SHA-256 of the judged patch alongside the failure digest for blocked uncommitted units, and suppress a new commit's block only when both match. The canonical uncommitted patch and the later commit patch are already intended to be byte-identical for an unchanged tree. Add a regression with the same failure shape but changed bytes, alongside the unchanged-patch control. Because the state schema currently claims a composite value but stores the old form, either change the schema or strictly validate/migrate the new entry form.

I found no new High or Low finding.

## Known fourth-review findings excluded from this pass

As requested, I treated these as already found and awaiting round 13, not as Vesper findings:

- staged/index-only content is not part of the uncommitted patch;
- after twelve standing failures, message truncation can omit the unit that actually blocks;
- a commit with no verdict can stop judging later session commits.

I read enough surrounding control flow to avoid mistaking their consequences for a new issue, but I did not reproduce or independently scope their fixes.

## Other checks

- HEAD was `f9d1c9d2f52a6dd7cdecf80445dc17611f02102f` on `agent/loop`.
- Frozen binary SHA-256 values: debug `3ac6300ca4652cbd2f234acb00aecbae4b2f1ceb53ea27f6d98f28128eb8213d`; release `c8fcdc84515b4e936e73b6f34e24b65088dc1a27002e4951945deff09a1146b9`. Real judge SHA-256: `55bf9fe6d19d7f7b66ef7cc7f3056ff9ab7ba7dc10b0e310d59a2aafd33874c9`.
- `git diff --check ffa0f8e..f9d1c9d` passed.
- The hive slot reported `free` immediately before every Cargo invocation. The toolbox launcher used the real HOME. Inside `fedora-toolbox-44`, HOME was `/tmp/vesper-v4r12-cargo-home`, the existing Rust toolchain directories were named explicitly, and `CARGO_TARGET_DIR=/tmp/vesper-v4r12-f9-target`.
- From `/tmp/vesper-v4r12-f9d1c9d`, an exact archive of `f9d1c9d`, `cargo test -p deep-diff-forge-patch --test judge_patch`: 18 passed. The first parallel run transiently failed `only_the_working_tree_and_the_ignore_rules_shape_the_patch` because its before/after loose-object count changed by one; the isolated retry, the full serialized suite, and a second normal parallel suite all passed. I could not reproduce a product failure and treat this as a verification caveat rather than a finding.
- From the same exact archive, `cargo test -p deep-diff-forge-cli --test agent_hook`: 62 passed.
- My host hook scripts set private HOME as their first operational environment step; no toolbox, podman, or distrobox launcher was invoked with HOME/XDG changed.

## Not examined / limitations

- I did not reproduce the fourth independent review's three known findings, review its uncommitted report file, or anticipate the round-13 implementation beyond avoiding duplicate findings.
- I did not run the full workspace suite, Clippy, Miri, sanitizers, mutation testing, a live Claude Code session, or the complete local gate.
- I did not test macOS, SHA-256 Git repositories, shallow/partial clones, network file systems, a reboot/remount, inode reuse, Git LFS, or two PID namespaces.
- I did not force the one-million-commit, 192-MiB listing, 256-worktree, 4,096-judged-entry eviction, 45-second deadline, or 65-MiB tips-file boundaries.
- I did not independently exercise every rebase/autosquash/cherry-pick/merge/reset workflow. I inspected the shared revision-walk/first-parent implementation and relied on the passing pull/rebase, commit/revert, detached-HEAD, linked-worktree, branch, tag, and stash integration cases plus my real-judge reproductions.
- I did not re-audit unchanged round-1-to-round-11 verdict sealing, ranker sealing, process isolation, setup/doctor, schema/explain, daemon, MCP, TUI, or SDK behavior beyond the paths compiled and exercised by the focused tests.
- I did not modify repository refs, remotes, index, configuration, or tracked files other than this report. Concurrent tracked round-13 edits and the pre-existing untracked fourth-review report remained untouched and were not reviewed.

STATUS: COMPLETE
