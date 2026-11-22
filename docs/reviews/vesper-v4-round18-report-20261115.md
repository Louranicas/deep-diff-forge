# Vesper V4 round 18 review — `5c387a3`

## Verdict

**NO-GO.** VD4R17-L-1 and HR9-M-1 are closed. The ordering fix works in the live linked-tree control, and failures larger than 256 lines now stand through partial staging because the two current verdicts are compared line by line rather than through the remembered mark.

One ordinary-workflow Low stands. When a session begins with more than 256 working trees, failure to enumerate the commit baseline prevents the project and base from being recorded at all. Consequently even the current tree's independently judgeable uncommitted credential is never offered to the judge. The stop is loudly UNJUDGED, so this is not silence or a High, but I would not discard the current-tree judgment merely because commit attribution is unavailable.

Reviewed exact commit `5c387a3fbe162c79664515c6cb2ca3ed523b5cb2`. Frozen release SHA-256: `6cd99e47718deee39bbce527180a872a754824b45f47761eb4fec4c3b231ef22`.

## Requested closures

- **VD4R17-L-1: closed.** `judge_units` now constructs the linked tree containing `cwd` before the recorded project tree and, importantly, does not call `uncommitted` for the second tree until the first tree's units have been judged. In the frozen-binary/real-judge reproduction, a stop in the linked tree submitted `lib/linked.rs` before `lib/main.rs`; a stop in the main tree submitted `lib/main.rs`. The focused slow test also passed: the linked tree blocks while the slow main tree is UNJUDGED.
- **HR9-M-1: closed.** `seen` derives `tree_whole` and `index_whole` from the two verdicts at this stop, before either is collapsed to `failure_lines`. `lines_beyond` and `failure_part` likewise use all verdict lines. In the live 257-file sequence, the first stop blocked; staging one file, about one hundred more, all files, and then unstaging all produced FAIL STANDS; adding one genuinely new failing file blocked.

## New finding

### VD4R18-L-1 — ordinary workflow at scale: a session begun with 257 worktrees does not judge its current tree

README lines 692-698 and the new integration test intentionally make every stop wholly UNJUDGED when the session begins with more than 256 worktrees, “whatever is in its tree.” The reason for the cap is sound: `named_commits` refuses to establish the pre-session commit set when it would have to read more than 256 worktree HEAD reflogs (`crates/deep-diff-forge-patch/src/judge_patch.rs:1058-1060,1123-1127`). What does not follow is that the current working-tree patch must also be abandoned.

`record` already obtains the current `HEAD` as the base before it asks for the named-commit baseline (`crates/deep-diff-forge-cli/src/hook_run.rs:1047-1068`). When baseline enumeration fails, however, it returns `Err` and discards the otherwise valid project identity and base. `locate` therefore records no project, and every later stop repeats the same start failure instead of reaching `judge_units`.

I reproduced the stated behavior live under `/var/home/louranicas/.cache/ddf-t/vesper-scratch/v4r18-many-trees` with 256 `--no-checkout` linked worktrees plus the main tree. After session start, I added a credential to `lib/credential.rs` in the main tree. Session start said all stops would be unjudged, and the stop returned only:

```text
deep-diff-forge: UNJUDGED. This stop was not judged: the commits that are there as the session begins could not be listed ... the repository has more working trees than a hook reads the history of ...
```

The current-tree credential was not offered to the real judge. By contrast, the test's session that began at 256 and then acquired a 257th tree judges and blocks on the identical current-tree class while saying only the session's commits are UNJUDGED. The latter separation is the right result in both cases: project identity, current `HEAD`, policy, and current tree are available; only the pre-session commit set is not.

Recommended fix: record the project identity and base together with a persistent “commit baseline unavailable” state when initial tip enumeration exceeds this bound. Continue to judge the current tree (and the linked current tree, if applicable), while making the session's commits UNJUDGED at every stop. This likely requires an explicit state representation/schema change rather than treating an empty tip list as a valid baseline.

Severity is Low because the threshold is unusual and every affected start/stop is explicit rather than silent. It is nevertheless an ordinary supported Git workflow at scale, and it is also avoidable loss of the hook's most immediate protection.

## Claims

1. **Held.** I found no stop that printed nothing with an open unit. The live overflow case is explicit UNJUDGED; the linked-tree and large-mark cases block or say FAIL STANDS. The focused unit and integration suites include the silence invariant.
2. **Held within the stated commit-enumeration bound.** The source records an ask exactly when `Judge::ran` is true; a run that ends at the stop deadline is still written, while a judge not started for lack of time is not. The 3.1/under-3/over-7-second focused tests passed. In a repository that already exceeds 256 trees, commits cannot be identified and are explicitly UNJUDGED rather than asked; VD4R18-L-1 concerns the separable current-tree change.
3. **Held.** Current verdicts are compared in full regardless of the remembered form. The live 257-line staging sequence held. Mark equality includes all line keys and counts; a changed mark blocks as a whole, and the staged kept-mark reconciliation only retains a mark while it equals the staged change's own lines or the whole index. The exact-model/unit tests covering sizes 200, 256, 257 and 600 passed. I found no unchanged repeat without the corresponding memory-loss sentence.
4. **Held.** The linked current tree is read and judged before the project's tree, and the latter is read lazily. Deep paths and symlinked paths canonicalize to their containing tree; a project that is itself a linked tree is its own first unit; a foreign nested repository/submodule is deliberately named and not judged. All operations remain under the one stop deadline. VD4R18-L-1 happens before a project is recorded and is not an ordering failure.
5. **Held in source and exercised tests.** The session lock remains the one writer. Message construction still reserves whole counting/standing lines and bounds the result. The new memory sentences are emitted at the transition that loses precision; paths and judge text continue through the data-safe renderers.

## D — more than 256 working trees

**No: I would judge the tree's own uncommitted and staged changes.** I agree that the session's commits must remain UNJUDGED when their pre-session baseline cannot be enumerated. I do not agree that this makes the recorded project identity, base commit, policy, and current-tree diff unknowable. The behavior for a session that crosses the threshold after starting already demonstrates the safe split. VD4R18-L-1 records this as a Low rather than accepting the all-or-nothing start state.

## Verification performed

- Read the request, round-17 report, ninth independent review, exact production/test/docs delta, and the branch-wide state/identity/message paths implicated by it.
- Verified exact HEAD and clean worktree before writing the report; verified both frozen binary hashes and a clean `git diff --check f604d58..5c387a3`.
- Ran `verify9.sh` with the frozen release binary and real forge-warden under cache-backed scratch. Linked-tree submission order was `linked.rs`, then `main.rs`; the 257-line staged sequence behaved as claimed.
- Made a fresh `git archive 5c387a3` under `/var/home/louranicas/.cache/ddf-t/vesper-scratch` with its own Cargo target. After checking the hive slot immediately before each Cargo command, 70 hook-focused CLI unit tests passed and all 106 `agent_hook` integration tests passed. Toolbox retained the real HOME.
- Reproduced the 257-worktree initial-session case live with the frozen release binary and real judge. All many-file scratch stayed under the requested cache directory.
- Did not kill or clean up any process by name.

## Not independently examined

I did not rerun the full 2,162-test workspace suite, clippy, the 32-step local gate, mutation controls, or release packaging. I did not independently run the full round-14-to-round-17 historical scripts, the live 25-second slow-order wrapper, or every 7.9/3.1/2.9-second wall-clock reproduction; the exact focused tests for the slow order and timing edges passed. I did not test a filesystem without birth time, inode reuse, NFS/FUSE/no-`flock`, non-UTF-8 worktree names, state/disk exhaustion, more than 2,048 live failing units, every mark transition live, submodule worktrees, hostile control characters across every message field, or Git variants outside the installed version.

STATUS: COMPLETE
