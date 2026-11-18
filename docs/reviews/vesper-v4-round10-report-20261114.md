# Vesper review: V4 round 10 (2026-11-14)

Reviewed: committed snapshot `1c7f8c1..69e2fe8` on `agent/loop`, using the frozen round-10 release binary named in the request and the real forge-warden where a verdict was needed. I wrote only this report in the repository.

## Verdict

**NO-GO.** HR-H-2 and every independent-review Medium and Low finding are closed. The ordinary forms of HR-H-1 are closed: after a recorded start, `cwd` in a nested or unrelated repository no longer chooses what is judged.

HR-H-1 is not fully closed. I found a path-preserving repository-replacement bypass, `VD4R10-H-1`: move the original project directory, clone the same base back at its recorded pathname, and stop with `cwd` in the moved original. The hook accepts the clean replacement as the recorded project, ignores the changed original because its Git common directory differs, and allows the stop with no output. No change was judged and no message names that fact.

## Independent-review findings

- **HR-H-1 — not closed.** Nested repositories, gone directories, unrelated `cwd` values and an entered linked worktree are handled, but `VD4R10-H-1` preserves the recorded pathname while changing the repository at it.
- **HR-H-2 — closed.** The first hook stores the full commit id. Advice and stops compare the current tree with that fixed base, and `Bench` reads the policy from the base commit. I committed a credential after `SessionStart`; the frozen binary and real judge blocked it under `no-secrets`, even though the current tree was clean. The integration test also covers a committed policy deletion followed by a credential.
- **HR-M-1 — closed.** Git failures, a refused/gone recorded project, an orphan base and an unreadable base take explicit trouble/UNJUDGED paths. Only Git's `LC_ALL=C` no-repository result is silent for an unrecorded session.
- **HR-M-2 — closed.** The real index is listed before scratch staging; indexed paths left out by ignore processing are force-added by literal NUL-delimited pathspec. The ignored-index integration and patch tests passed.
- **HR-M-3 — closed.** Notes and expired waivers are carried from the sealed verdict, bounded and escaped as the judge's words. A fail with no failed rule says that the matter is for a person rather than giving an impossible repair instruction.
- **HR-M-4 — closed.** `agent doctor --gate` makes the working-tree patch and checks the effective settings chain, `disableAllHooks`, exact command, hook type and timeout. The gate tests for skip-worktree, unreadable input and settings all passed.
- **HR-M-5 — closed.** Hook argument refusal emits a visible message and returns normally; a Stop usage error is explicitly UNJUDGED rather than process exit 2.
- **HR-L-1 — closed.** One 45-second deadline is shared by all Stop work inside the installed 60-second timeout, and a later hook removes dead-process scratch directories where `/proc` is available.
- **HR-L-2 — closed.** Advice failures are deduplicated by a bounded per-session set; Stop still repeats the reason.
- **HR-L-3 — closed.** `SessionStart` uses the gate links and promises judgment only when the chain is ready; otherwise it names the missing links.
- **HR-L-4 — closed.** The repeated-failure identity includes rule, signal, path and evidence-item count; a changed count blocks again. Failure to persist state is reported with the decision.
- **HR-L-5 — closed.** Scratch staging disables repository, user and system configuration and user/system attributes. Only the user's absolute `core.excludesFile` value is deliberately carried over; the hostile user-filter test passed without running the filter or changing the patch.
- **HR-L-6 — closed.** The vocabulary now derives `hazardous_text` basis and sides from the scorer/rule facts. All vocabulary fact tests passed.

## New finding

### VD4R10-H-1 — a replacement repository at the recorded pathname silently displaces the session's actual project

Session state identifies the project only by its top-level pathname and base commit. On every hook, `locate` resolves `CLAUDE_PROJECT_DIR` first and accepts it when its top equals the stored string (`hook_run.rs:638-683`). It does not verify that the repository at that path is the one recorded at session start.

`judge_trees` separately considers `cwd`, but `other_tree` adds it only when its current Git common directory equals the accepted project's common directory (`hook_run.rs:1231-1251`). A moved original repository and a replacement clone have different common-directory paths, so the original is treated as an unrelated repository and silently ignored.

Reproduction, using frozen release binary SHA-256 `18ad57e3e35e7c421d0dc5e3cbcad748068064599e7c0fe1016c791b2915574e`:

1. Clone the supplied base repository to `/tmp/.../project`.
2. Run `SessionStart` with one fresh session id and `CLAUDE_PROJECT_DIR=/tmp/.../project`; state records base `c278f8b...` and that pathname.
3. Add `AKIAIOSFODNN7EXAMPLE` to `lib/a.rs` without committing.
4. Move the whole repository to `/tmp/.../project-moved`.
5. Clone `project-moved` back to the original `/tmp/.../project` pathname. The replacement clone is clean and has the recorded base.
6. Run Stop with the same session id, `CLAUDE_PROJECT_DIR` still naming `/tmp/.../project`, and hook-input `cwd` naming `/tmp/.../project-moved`.

Observed:

```text
stop output bytes: 0
moved original:      M lib/a.rs
replacement clone:  clean
exit:                0
```

The judge was never sought because the accepted replacement tree equalled the base. This contradicts the documented promise that the recorded project is judged wherever the session has gone, and it recreates HR-H-1's silent allowance with a changed project.

Impact is High: a started session can leave a credential or any other change in the repository it is actively in, while a clean lookalike at the recorded pathname causes Stop to look exactly like a pass. This is distinct from the declared “another branch/another linked worktree” limitation: `cwd` names the moved original project from the same session, and the replacement exists only to satisfy the pathname lookup.

Recommended fix: record and verify repository identity as well as the pathname—on Unix, the Git common directory's device/inode identity is a conservative session-local discriminator. A mismatch must at least produce visible UNJUDGED, never accept a clean replacement. When `cwd` resolves to the original recorded identity after a move, it can be judged there; otherwise say that the project was replaced or moved and could not be identified. Add exactly the move-and-reclone reproduction as an integration test.

I found no new Medium or Low finding.

## Other attacks and checks

- A committed credential after a recorded start was blocked by the real judge under the policy at the recorded base.
- Source review confirmed that reset/rebase/checkout does not move the stored base; a missing base becomes UNJUDGED. Work committed only on another branch without touching a judged tree remains the explicitly documented PR-judge limitation.
- A resumed/compact/clear `SessionStart` with the same session id reuses the stored project/base. Session ids are hashed before choosing a state filename. Different project paths under one reused id conflict visibly rather than silently replacing state.
- A symlink or renamed policy in the working tree cannot change the bytes read by `file_at_commit`; `--policy NAME` selects that named path at the recorded base.
- `git diff --check 1c7f8c1..69e2fe8` passed.

## Tests

The hive slot reported `free` immediately before each Cargo invocation. Cargo ran in `fedora-toolbox-44` with `CARGO_TARGET_DIR=/tmp/vesper-v4r10-target`.

- `deep-diff-forge-cli --test agent_hook`: 34 passed.
- `deep-diff-forge-patch --test judge_patch`: 9 passed.
- `deep-diff-forge-graph vocabulary`: 7 passed.
- Frozen-binary reproductions covered the committed-credential block with the real judge and `VD4R10-H-1`'s silent path-preserving replacement.

## Not looked at / limitations

- I did not run the full workspace suite, Clippy, Miri, fuzzers, or the complete local gate.
- I did not run a live Claude Code session; hook inputs and environment were supplied exactly to the frozen binary.
- I did not rerun every malformed-verdict, non-UTF-8 ranker, blocked-read or 500,000-entry probe from rounds 1-9; those paths were covered by the earlier incremental reviews and their regression tests.
- I did not independently measure the full 45-second deadline under eight slow rankers or on a filesystem without `/proc`.
- I did not test macOS/Windows, submodule-policy layouts, or every possible concurrent pair of hooks sharing one session id.
- I did not review changes outside `1c7f8c1..69e2fe8` or modify repository refs, remotes, index, configuration, or files other than this report.

STATUS: COMPLETE
