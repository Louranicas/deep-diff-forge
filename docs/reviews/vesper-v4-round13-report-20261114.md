# Vesper V4 round 13 review (2026-11-14)

Reviewed: exact committed snapshot `f9d1c9d..651adae` on `agent/loop`, frozen round-13 debug/release binaries, and the real `forge-warden` named in the request. I wrote only this report in the repository; reproductions and an exact source archive were under `/tmp`.

## Verdict

**NO-GO.** VD4R12-M-1 is closed. HR4-H-1, HR4-H-2, HR4-M-1, HR4-M-3, and HR4-L-1 through L-6 are closed as scoped. HR4-M-2's original permanent starvation is also closed: a no-verdict commit no longer prevents later commits from being judged.

I found one new Medium ordinary-workflow defect in the replacement classification, VD4R13-M-1. A successful probe or verdict for another patch is taken as proof that a no-verdict answer for this commit is permanently patch-specific. That inference is not valid for a transient judge invocation/runtime failure. I reproduced a credential commit getting one explicit UNJUDGED message, being stored as `ID:u`, and then being silently skipped forever, even though the same judge worked immediately afterwards.

I found no new High or Low finding. My verdict on `agent/loop` as a whole is NO-GO because a session commit can be permanently retired without a verdict on an ordinary transient failure.

## VD4R12-M-1 / HR4-L-4

### Closed

`commit_unit` no longer compares a commit's failure identity with failures that previously blocked as uncommitted or staged changes. Every newly answered failing commit has `Holds::Now`, so it blocks independently of the contents of `State.blocked`.

I reproduced the round-12 shape with the frozen release binary and real judge in `/tmp/vesper-v4r13-real/loose`. An uncommitted example credential blocked; I then replaced the patch with changed code and another example credential of the same rule/signal/path/count shape and committed it without an intervening stop. Commit `9a675b06152a…` blocked at the next stop and was named with `no-secrets`. It did not inherit the earlier uncommitted failure's standing status.

The old comment claiming a `FAILURE|PATCH` composite is gone; the current state documentation accurately limits `blocked` to uncommitted and staged failure identities.

## Fourth independent review findings

### HR4-H-1 — closed

`staged_patch` copies the repository index's stage entries into a scratch index, writes its tree in the scratch object store, and diffs that tree against `HEAD`. It does not write the reviewed repository's index or object store. `uncommitted` makes the staged patch a separate unit when it differs from both `HEAD` and the working-tree patch. The same path is used for a linked working tree the session has entered.

I reproduced the reviewer's ordinary case with the frozen release binary and real judge in `/tmp/vesper-v4r13-real/staged`: a new file containing an example credential was staged and then removed from the working tree. The stop blocked and named `Staged changes in "/tmp/vesper-v4r13-real/staged"` and `lib/staged_key.rs` under `no-secrets`.

The integration regression additionally covers a tracked staged credential repaired but not restaged, the new-file/deleted-tree case, the identical fully staged case (one judge run), and a linked worktree. The patch tests cover partial staged/tree divergence and index immutability. Unmerged index entries produce UNJUDGED rather than a fabricated tree, as stated.

### HR4-H-2 — closed

The message is now assembled by consequence: every failure that blocks now first, then every UNJUDGED unit, then HOLD units, then standing failures. A blocking unit is always named; when its full rule text would exceed the cap the message explicitly says the rules are omitted because it is full. HOLD and standing detail yield first, and standing failures collapse to three names plus a count.

The 13-standing-failures regression passed: a new no-verdict commit is named at the head of an UNJUDGED message, and a later failing commit is named with its rule before the standing summary. The first line is selected from the worst new result, so standing failures no longer mask a new UNJUDGED or HOLD.

### HR4-M-1 — closed as the selected documentation correction

The implementation still defines the session's commits as commits reachable from current local sources but not from anything recorded in the repository at session start. Initial tips include remote-tracking refs; current sources do not. Therefore a commit fetched before the session is not judged merely because the session later fast-forwards, merges, rebases onto, or resets to it; one fetched during the session and then brought under a local source is judged.

README, SECURITY, start text, range text, limits, and tests now say this explicitly. A non-fast-forward merge commit newly made by the session remains its own unit and exposes its first-parent change. I found no ordinary workflow in which work created by this session is misclassified as initially present: another pre-existing session's commit is deliberately initial work, while a commit concurrently made after this session's tips were recorded is beyond those tips and is judged when a local source names it.

### HR4-M-2 — original starvation closed; replacement has VD4R13-M-1

The session-long “three no verdicts” behavior is gone. With a patch-specific refusal while the chain is deemed operational, the commit is recorded `:u`, named once, and iteration continues; later commits in the same stop are judged. If the chain as a whole is deemed unable to give verdicts, the current and remaining commits are counted, nothing is remembered for them, and a later stop retries.

The regression with three refused commits followed by a credential passes, as does the broken-chain case that stops after the first commit and a probe and retries everything with a working judge. However, the predicate used to distinguish those cases is too loose; see VD4R13-M-1.

### HR4-M-3 — closed

When saving state fails, the actual decision is still produced and the failure to keep state is appended. Only in that error path, when the client says a stop hook is already active and the decision would block, the hook allows with `FAIL STANDS`, repeats what would have blocked, and explains that it cannot remember. The focused regression passed.

Making the state directory unwritable on purpose is deliberate evasion, not an ordinary workflow. It still causes the first failing stop with `stop_hook_active=false` to block, and subsequent output is explicit rather than a pass. Read-only media, permissions, or a full filesystem are ordinary failures covered by the same behavior.

### HR4-L-1 — closed

Reflogs are read with `git rev-list --walk-reflogs`, so display/signature configuration cannot inject non-ID lines. A missing/stale worktree directory is skipped while collecting sources and is separately named with the `git worktree prune` remedy. The gate now includes a `commits` link. The relevant patch and integration regressions passed.

### HR4-L-2 — closed

IDs listed at the shallow boundary are detected from the Git common directory's `shallow` file. A boundary commit with no available parent is named UNJUDGED with the `git fetch --unshallow` remedy rather than diffed against the empty tree. The shallow-clone regression passed.

### HR4-L-3 — closed

Judged commits moved out of a 4,096-entry state list no longer become tips. They are kept in a separately sealed `session-<hash>.judged` file with schema v4. The state authenticates a prefix by count and SHA-256; extra complete lines left by a crash before the state write are ignored, and a later successful write reconciles them. Within the one-million-source/list-file bounds, an older UNJUDGED commit cannot disappear merely because many later commits got answers.

### HR4-L-5 — closed within the stated bound

At each stop, other usable working trees are checked for staged, unstaged, or untracked changes without refreshing their indexes. One with changes is named once as not judged; a removed tree is named with the prune remedy. Sixteen other usable trees are checked and the seventeenth-and-later limit is explicit in README and SECURITY. The integration regression passed.

### HR4-L-6 — closed

The five reported documentation/code disagreements are reconciled: initially present commits, staged content, message ordering/truncation, no-verdict handling, and uncommitted-to-commit blocking now match the implementation. Historical changelog entries remain dated history.

## New finding

### VD4R13-M-1 — ordinary workflow: a transient judge failure is permanently remembered as a patch-specific refusal

`Judge::judge` sets the stop-wide `chain` flag to true after any patch gets a verdict. If no earlier patch did, `chain_works` runs one policy-protection probe and treats any verdict for that probe as proof that the chain works. In `judge_commits`, any commit that then got `Judged::Unjudged` while at least three seconds remain is recorded as outcome `u` whenever that flag/probe is true. `remember` appends `ID:u` to the judged file. Every later stop treats any known outcome other than `f` as complete and skips the commit.

That proves only that the chain worked for a different invocation. It does not prove this commit's no-verdict answer was caused by immutable patch content. `run_judge` folds invocation timeout, signal/termination, transient I/O/environment failures, malformed or missing output, and many other judge failures into the same `Judged::Unjudged` variant as deterministic patch refusal.

Reproduction: `/tmp/vesper-v4r13-transient/repro2.sh`, frozen release binary. The wrapper delegated to the real `forge-warden` except that the first commit invocation returned one exit-3 `forge-warden.error.v1` saying `transient judge failure`; every following invocation delegated normally.

1. Session start completed. I reset only the wrapper's call counter.
2. Commit `b4a88fac9e03…` added an example AWS credential.
3. Commit `e04ab14451ed…` added harmless code.
4. At the stop, the credential's judge invocation got the one transient error. The protection probe immediately got a real verdict, and the harmless commit then got a real pass.
5. The stop allowed with an explicit UNJUDGED message naming `b4a88fac9e03` and the transient reason. The judged file stored:

   ```text
   b4a88fac9e03fbb47bff2aa4ec427ae5b7e3fab2:u
   e04ab14451edf05444343353f48e24ca56d89471:p
   ```

6. The next stop produced no output and made no judge call. The credential commit will never be retried or block in that session.

This is Medium, not High: the first stop plainly says UNJUDGED and names the commit, so it is not a silent pass. It is nevertheless an ordinary process/environment failure, and the permanent skip contradicts the distinction this round intends: the chain, not the patch, failed on that invocation.

Recommended fix: do not make a generic judge no-verdict permanent from a successful verdict/probe in another invocation. Persist `:u` only for refusals the hook itself can prove deterministic from the commit/repository (for example the known shallow boundary or a patch-acquisition refusal with a stable class), or keep “said once” separate from “has a verdict” and retry the commit on a later stop after continuing to later commits now. Add a regression where a credential commit's judge invocation fails once, the probe and next commit succeed, and the credential blocks on the next stop.

## Verification

- HEAD: `651adaea28e05759d7d7102adf16dd8cead09920`.
- Frozen binary SHA-256: debug `e45df4272cc370793453a467d99b514ad4a00b416a117454835bfe6be2285986`; release `02b953f2ff9978b95703ba9021b7b933a6d9ffced7d72d82772707b253d80874`. Real judge SHA-256: `55bf9fe6d19d7f7b66ef7cc7f3056ff9ab7ba7dc10b0e310d59a2aafd33874c9`.
- `git diff --check f9d1c9d..651adae` passed.
- Source tests ran from `/tmp/vesper-v4r13-651adae`, an exact `git archive` of `651adae`.
- The hive slot reported `free` immediately before each Cargo invocation. Toolbox was launched with the real HOME; only the command inside `fedora-toolbox-44` received `HOME=/tmp/vesper-v4r13-cargo-home`, with explicit existing Rust toolchain paths and `CARGO_TARGET_DIR=/tmp/vesper-v4r13-target`.
- `cargo test -p deep-diff-forge-patch --test judge_patch`: 20 passed.
- `cargo test -p deep-diff-forge-cli --test agent_hook`: 71 passed.

## Not examined / limitations

- I did not rerun all 30 fourth-review workflows or all thirteen third-review cases with the real judge. I read their common paths, ran the focused exact-commit suites, and reproduced staged-only content, the loose failure transition, and the new transient no-verdict case.
- I did not run the full workspace suite, Clippy, Miri, sanitizers, mutation testing, a live Claude Code session, or the complete local gate.
- I did not test macOS, Git older than the installed version, SHA-256 object repositories, reftable, partial clones, Git LFS, network filesystems, overlayfs remounts, a full filesystem, or a real killed/oomed `forge-warden`; the one-shot wrapper deterministically models the control-flow result of such a transient invocation failure.
- I did not force the one-million-commit, 192-MiB ref/reflog listing, 67-MiB list-file, 256-worktree, seventeenth-other-tree, 45-second deadline, or very-large-message boundaries.
- I did not re-audit unchanged round-1-to-round-12 verdict sealing, ranker hashing/isolation, setup, daemon, MCP, TUI, SDK, schema, or signal behavior beyond code reached by the focused tests.
- I did not mutate repository refs, remotes, index, configuration, or tracked files other than this report.

STATUS: COMPLETE
