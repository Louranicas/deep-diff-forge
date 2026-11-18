# Vesper review: V4 round 3 (2026-11-14)

## Verdict

**NO-GO.** The skip-worktree and patch-error fixes close VD4R2-M-1 and the newly disclosed silent-failure path, but the exact-command check still accepts a command that the shell runs as another program, and the verdict reader still accepts a materially incomplete and contradictory document as PASS.

## Findings

### VD4R3-H-1 — A shell assignment can stand in for the checked executable

`plain_words` admits `=` in every plain word, and `not_this_chain` canonicalizes the first decoded word as a path. A POSIX shell does not necessarily execute that word: at command position, a word such as `A=ddf` is a variable assignment, and the following word (`agent`) is the program.

I reproduced this against the supplied debug binary in `/tmp/vesper-v4r2-doctor`:

1. I made the repository entry `A=ddf` a symlink to the supplied `deep-diff-forge` binary.
2. I registered the command `A=ddf agent hook stop --client claude --judge /tmp/vesper-v4r3-hold-judge --policy warden.toml`.
3. The stand-in judge returned a sealed HOLD, so the policy-protection probe was affirmative. `agent doctor --gate ... --json` exited 0; both `chain` and `hooks_registered` were `ok: true` (`/tmp/vesper-v4r3-assignment-gate-pass.json`).
4. Running that exact command through `/bin/sh`, with a recording `agent` on `PATH`, printed `different-program:hook stop ...` and exited 0. Deep Diff Forge and its judge were never run (`/tmp/vesper-v4r3-assignment-runtime-pass.out`).

This is not only a later-mutation race: the gate and the shell disagree about the unchanged command at the instant it is checked. A separately reproduced variant used a repository-writable `ddf-link` which canonicalized to this binary during the gate and was then redirected to `/usr/bin/true`; the registered command subsequently exited 0 with no output. Thus canonicalizing an arbitrary spelling does not establish that the shell will execute this immutable program.

Impact: `doctor --gate` can certify a chain whose Stop hook does not invoke Deep Diff Forge at all. That defeats the high-level purpose of the gate and leaves a stop unjudged without even the hook's UNJUDGED notice.

Recommended fix: require the decoded program word itself to be the absolute canonical `current_exe` spelling emitted by `agent setup --hooks`, rather than canonicalizing a relative or linked spelling and comparing its target. In particular, reject assignment words and relative paths. This also closes the requested repository-link and different-working-directory cases.

### VD4R3-M-1 — The “whole v2” check accepts an incomplete, contradictory verdict as PASS

`not_a_verdict` checks only a shallow subset of the versioned document. It does not require, among other schema members, `seals.patch_facts_sha256` or `seals.git`; it accepts empty `ranked_by` and `summary` objects and an empty `rules` list; and it checks neither the allowed values nor the relationship of `exit.name` and `exit.action` to the verdict. Nested rules, violations, waiver notes and notes likewise receive only partial type checks.

I reproduced the consequence with `/tmp/vesper-v4r3-invalid-judge`. It exited 0 and emitted a correctly policy/patch-sealed document with:

- `evaluated_on: "not-a-date"`;
- no `patch_facts_sha256` and no `git`;
- `ranked_by: {}`, `summary: {}`, and `rules: []`;
- `verdict: "pass"` and `exit.code: 0`, but `exit.name: "policy_fail"` and `exit.action: "change_input"`.

The supplied hook exited 0 and emitted zero bytes (`/tmp/vesper-v4r3-invalid-doc.out`): it treated that non-v2 document as PASS. The intentionally unchecked date and ranker identity are not necessary to the reproduction; missing required members, empty required structures, and a contradictory exit tuple suffice.

Impact: a malformed or version-incompatible judge response can silently become PASS instead of UNJUDGED. This is the same trust-boundary failure as VD4R2-M-2, despite the added top-level checks.

Recommended fix: validate the complete `forge-warden.verdict.v2` contract with a typed representation or the authoritative schema, including every required nested member, enums, minimum cardinalities and the verdict/exit tuple. Forward-compatible unknown members can remain accepted if that is a deliberate local compatibility rule; they do not require accepting absent or malformed members known to v2.

## Round 2 disposition

| Round 2 finding | Status | Evidence |
|---|---|---|
| VD4R2-H-1 | **Not closed** | The former substring acceptance is fixed, but VD4R3-H-1 shows the replacement parser still certifies a command the shell runs as another program. |
| VD4R2-M-1 | **Closed** | `git ls-files -t -z --sparse` is bounded and inspected before patch construction. The existing hand-marked fixture listed `S hidden.txt`; Stop returned an explicit UNJUDGED message naming skip-worktree (`/tmp/vesper-v4r3-sparse-stop.out`). Ordinary tracked entries were not refused. Sparse-index directory entries are preserved by `--sparse` and carry the same `S` tag. |
| VD4R2-M-2 | **Not closed** | Duplicate-key rejection and process/verdict/code agreement are improvements, but VD4R3-M-1 demonstrates that a document which is not a whole v2 verdict is still accepted as PASS. |

The additional patch-error fix is effective in the exercised path. With `.git/info/exclude` unreadable, Stop returned UNJUDGED with `Permission denied`, rather than returning no output as “nothing to judge” (`/tmp/vesper-v4r3-exclude-failure.out`). The source now maps only `JudgePatchError::NoBase` and a genuinely empty patch to `Ok(None)`; other patch construction failures reach UNJUDGED or the explicit “hook did not run; nothing was judged” message.

## Checks performed

- Read `f9d7b1a..045ef60`, including the command parser/gate, verdict reader, duplicate-key scanner, patch error mapping, and skip-worktree detection.
- `git diff --check f9d7b1a..045ef60` passed.
- Reproduced both findings and the skip-worktree/unreadable-exclude controls with the supplied `045ef60` debug binary.
- Checked the hive build slot immediately before attempting Cargo; it was free. `cargo` is not installed on this host (`cargo: command not found`), so I did not rebuild or run Cargo tests.

## Not looked at / limitations

- I did not empirically build a multi-million-path index or a partial clone. By inspection, timeout or the 64 MiB listing cap is a failure rather than “nothing to judge”; PostToolUse reports the generic hook-failure system message, while Stop reports UNJUDGED.
- I did not repeat the full real-forge-warden pass/hold/fail matrix. The available forge-warden refused the ad-hoc invocation because this host has no Git allowlist enrolled; the request records that matrix as already run by the author.
- I did not test managed settings or plugin-provided hooks, which the request explicitly excludes from the gate's current discovery set.
- I did not review changes outside `f9d7b1a..045ef60`.

STATUS: COMPLETE
