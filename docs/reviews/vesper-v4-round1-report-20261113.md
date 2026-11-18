# Vesper V4 round 1 review — agent hooks and judge patch

Reviewed: `d4dbc91..aef898f` (`1cde0ff`, `aef898f`) on `agent/loop`

## Verdict: NO-GO

The hook decisions, base-commit policy read, ordinary patch equivalence, message escaping, loop guard and signal vocabulary behave as claimed in the tested cases. The default hook setup nevertheless trusts the first file named `forge-warden` on `PATH`, and exit 0 is accepted as PASS without any verdict document. A repository-writable PATH entry can therefore make a failing stop pass silently. State storage also follows a symlink out of the Git common directory and reads attacker-sized files without a cap, while repository-local attributes can change the bytes judged despite the documented invariance.

I reviewed the committed snapshots with `git show aef898f:PATH` and `git diff d4dbc91..aef898f`; I did not review the uncommitted `agent doctor --gate` work. I did not run Cargo because hive reported the `lodestar` build lane occupied.

## Findings

### VD4-H-1 — A program merely named `forge-warden` can silently pass a failing stop

The setup-generated hook command names no `--judge`, so `judge_program` searches `PATH` and accepts the first regular file named `forge-warden`. It performs no identity, ownership or location check. `judged(Some(0), …)` then returns `Pass` even when stdout is empty or not a `forge-warden.verdict.v2` document.

A repository or toolchain that can place a file in an earlier PATH directory can therefore replace the judge. This is not only a denial of judgment: the fake program's exit 0 produces no hook output, exactly the successful-stop behavior.

Reproduction:

```sh
mkdir -p /tmp/vesper-v4/bin
printf '%s\n' '#!/bin/sh' 'exit 0' > /tmp/vesper-v4/bin/forge-warden
chmod 755 /tmp/vesper-v4/bin/forge-warden

# /tmp/vesper-v4/repo has a committed warden.toml and a dirty code.rs.
printf '%s' \
  '{"hook_event_name":"Stop","session_id":"fake-pass","cwd":"/tmp/vesper-v4/repo","stop_hook_active":false}' \
  | PATH="/tmp/vesper-v4/bin:$PATH" \
    /var/home/louranicas/.cache/ddf-t/target-agentloop/release/deep-diff-forge \
      agent hook stop --client claude \
  > /tmp/vesper-v4/fake-pass.out
```

Observed:

```text
exit:         0
stdout bytes: 0
```

For the same repository and hook input, a stand-in judge that printed this document and exited 7 produced `{"decision":"block",…}`:

```json
{"schema":"forge-warden.verdict.v2","rules":[{"id":"must-fail","outcome":"fail","why":"probe","violations":[{"signal":"unsafe_added","path":"code.rs"}]}]}
```

Fix: setup should resolve and print an absolute, operator-admitted judge path rather than leaving the security decision to the hook's future PATH. Refuse judge paths inside the reviewed repository or another repository-writable location. A PASS must also require a valid, schema-checked verdict document whose aggregate outcome is pass; exit 0 with absent, malformed or contradictory output must be UNJUDGED. A schema check alone does not authenticate a PATH replacement, so both controls are needed.

### VD4-M-1 — The state store follows directory symlinks and reads state without a size bound

`Place::own` is `<common-dir>/deep-diff-forge`, but `Scratch::new`, `load` and `save` use ordinary pathname operations. They never reject a symlink at that path. A symlink causes scratch activity and the persistent session file to occur in the link target, contrary to the claimed location and write boundary.

Reproduction:

```sh
mv /tmp/vesper-v4/repo/.git/deep-diff-forge \
   /tmp/vesper-v4/repo/.git/deep-diff-forge.before-symlink
mkdir -p /tmp/vesper-v4/state-target
ln -s /tmp/vesper-v4/state-target \
      /tmp/vesper-v4/repo/.git/deep-diff-forge

printf '%s' \
  '{"hook_event_name":"PostToolUse","session_id":"symlink-probe","cwd":"/tmp/vesper-v4/repo"}' \
  | /var/home/louranicas/.cache/ddf-t/target-agentloop/release/deep-diff-forge \
      agent hook post-tool --client claude
find /tmp/vesper-v4/state-target -maxdepth 1 -type f -printf '%f %s bytes\n'
```

Observed:

```text
session-680dd6b23f636b9382fe3c51f0b304a5.json 140 bytes
```

The hook returned ordinary advice while writing that state outside the common directory.

`load` also calls `read_to_string` with no cap before parsing. Replacing another session's state with a 512 MiB sparse, invalid JSON file and running `post-tool` completed, but consumed over half a gigabyte merely to fall back to empty state:

```sh
session_hash=$(printf '%s' huge-state | sha256sum | cut -c1-32)
truncate -s 536870912 \
  "/tmp/vesper-v4/state-target/session-${session_hash}.json"
/usr/bin/time -f 'elapsed=%e maxrss_kb=%M exit=%x' \
  sh -c 'printf "%s" "$1" | "$2" agent hook post-tool --client claude >/tmp/vesper-v4/huge-state.out' sh \
  '{"hook_event_name":"PostToolUse","session_id":"huge-state","cwd":"/tmp/vesper-v4/repo"}' \
  /var/home/louranicas/.cache/ddf-t/target-agentloop/release/deep-diff-forge
```

Observed:

```text
elapsed=0.35 maxrss_kb=528340 exit=0
```

Fix: open/create the state root relative to a verified common-directory handle, reject symlinks and non-directories, and use no-follow/atomic APIs that do not reopen attacker-substitutable path components. Bound state reads to a small documented maximum before allocation and treat oversized state as empty with a bounded diagnostic. Use restrictive directory/file modes and a securely created temporary file rather than a predictable pathname alone.

### VD4-M-2 — `.git/info/attributes` changes the exact patch bytes

The final diff is isolated from attributes, but the scratch index is populated with `git add --all` in the reviewed work tree. Built-in clean conversions still consult working-tree, info and configuration attributes. External filters are neutralized; text/EOL and working-tree-encoding behavior is not.

I committed `file.txt` as `base\n`, changed its working-tree bytes to `changed\r\n`, and captured the judge's `--patch` input twice. The only environmental change between captures was this uncommitted Git-internal file:

```text
.git/info/attributes: *.txt text eol=lf
```

Observed:

```text
without info attributes:
  sha256 5bc8daa5a7740abc439d361110d92cb2527b93cb4047b937102e8b6caa52e05c
  added bytes: changed\r\n

with info attributes:
  sha256 1e46976bc06d48c1aedd6c9fa9c7bec0da7b5c503319d31066d95200813115d3
  added bytes: changed\n
```

The patch object IDs changed as well. Thus repository-local state that is not in the change changes the bytes given to the judge, contrary to claim 5 and the integration test's stated invariant.

Fix: define one canonical staging policy. If environment-independent bytes are required, exclude `.git/info/attributes`, global attributes and configuration-driven normalization while building the scratch tree, and deliberately apply only the committed/worktree `.gitattributes` semantics the contract chooses. If exact “what this local Git would commit” semantics are required instead, narrow the invariant and surface the attribute/configuration provenance because claims 4 and 5 cannot both mean their current broad wording. Add CRLF, `core.autocrlf`, `.git/info/attributes` and `working-tree-encoding` regression cases.

## Claim assessment

| # | Assessment | Evidence |
|---:|---|---|
| 1 | **Held** | End-to-end `post-tool` advice exited 0, contained `hookSpecificOutput.additionalContext`, and had no `decision`. A hostile newline/ESC path was rendered as visible `\\x0a`/`\\x1b`; the decoded message had no raw control characters. |
| 2 | **Broken** | Ordinary fail/hold/unjudged and repeat-loop behavior worked, but VD4-H-1 turns “the judge is not forge-warden” into silent PASS rather than UNJUDGED. Sequential identical fail tests blocked once and then emitted `FAIL STANDS` without a second block. |
| 3 | **Held** | With committed policy name `attr-probe` and working-tree policy name `loosened-in-working-tree`, the file passed to the judge contained the committed `attr-probe` policy. |
| 4 | **Held in tested cases** | The committed integration test passed byte-for-byte equivalence and covered tracked edits/deletion, untracked and ignored files, executable mode and binary content. I did not cover every exotic Git state listed in the request. |
| 5 | **Broken** | VD4-M-1 crosses the intended state/scratch location through a symlink. VD4-M-2 shows repository-local attributes change patch bytes. The committed fsmonitor/hook/filter hardening otherwise disables those program-execution paths in the inspected Git runner. |
| 6 | **Held in tested cases** | Messages contain labels, locations and remedy text, not changed line bodies. Newline, ESC and quotation in paths were escaped and paths are capped. Rule text uses `display_safe`; advice and rule bodies are capped below the client's 10,000-character limit. |
| 7 | **Broken** | VD4-M-1 shows the state file need not live under the Git common directory and may impose attacker-selected allocation. The session ID itself is hashed and does not directly choose a pathname; malformed state falls back rather than producing PASS. |
| 8 | **Held for valid hook invocations** | Runtime input, Git, state and judge failures tested here returned exit 0 and either no decision or UNJUDGED. The fake judge issue is a false PASS, assessed under claim 2, not a nonzero hook exit. |

## Other verification

- Ran the prebuilt committed test executables directly: `agent_hook` 9/9, `judge_patch` 5/5, `contract` 19/19 and graph 113/113 passed.
- `schema signals` returned 21 unique labels. The graph tests tie weights and sides to the scorer/rule table, and the contract tests confirmed README/vocabulary agreement. I inspected every `explain signal` accepted/substitute line and found no unsafe remedy instruction.
- The scoring diff replaces numeric literals with `RiskSignal::weight()` values of the same numbers; I found no ranking behavior change.
- A missing explicit judge produced an `UNJUDGED` system message. A failing judge blocked, and the same failure with `stop_hook_active: true` was reported to the user without blocking again.
- A path containing a newline and ESC produced a 617-byte advice message with no raw control bytes.

## Not examined

- I did not run Cargo, Clippy or fuzzers because the build lane was occupied.
- I did not run the live forge-warden script inside `fedora-toolbox-44`; stand-in judges and the supplied DDF binary were sufficient for the findings above.
- I did not exhaustively test submodules, nested repositories, sparse checkout, linked worktrees, merge state, detached HEAD, unborn HEAD, every global-excludes arrangement, or every non-UTF-8 path.
- I did not reproduce a client-enforced kill at 5/60 seconds, orphaned Git children, or stale scratch accumulation after SIGKILL.
- I did not audit the uncommitted `agent doctor --gate` changes visible in the worktree.
- I did not modify repository files, refs, remotes or existing worktree changes other than this report.

STATUS: COMPLETE
