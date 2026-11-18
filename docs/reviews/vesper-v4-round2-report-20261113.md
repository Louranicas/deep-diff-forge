# Vesper V4 round 2 review — hook fixes and `agent doctor --gate`

Reviewed: `aef898f..f9d7b1a` on `agent/loop`

## Verdict: NO-GO

The three V4 round-one findings are closed in their original forms. The sealed-document check, judge-path pinning, bounded/no-follow state store, and scratch Git repository all withstand the original reproductions.

I would not merge `agent doctor --gate` as a readiness gate yet. It returns success for a Stop command that does not invoke DDF at all, merely because a shell comment contains `agent hook stop`. Scratch staging also misrepresents a sparse checkout by reporting every intentionally absent tracked file as deleted. Finally, the verdict check accepts a schema-invalid and internally contradictory document as PASS; this is limited by the trust placed in the pinned judge, but contradicts the stated goal that accidental malformed output becomes UNJUDGED.

## Round-one findings

- **VD4-H-1: closed.** The original first-on-PATH program that exits 0 and prints nothing now produces `UNJUDGED … the judge exited 0 (pass) but that is not a verdict: it printed no verdict document`, not silent PASS. `agent setup --hooks` writes the canonical judge path into SessionStart and Stop commands. Relative/empty PATH entries are ignored, and canonical containment checks cover the work-tree top and linked-worktree common directory. The shallow document validation issue is new VD4R2-M-2 rather than the original unsealed-exit exploit.
- **VD4-M-1: closed.** A symlink at `<common-dir>/deep-diff-forge` is refused with `is not a directory of its own`; no file appeared in its target. A 512 MiB state file was rejected before reading and replaced atomically after the hook ran: 0.05 s, 5,412 KiB peak RSS, exit 0, final state 140 bytes. The descriptor is no-follow/nonblocking, must be a regular file, and is size-checked before allocation.
- **VD4-M-2: closed.** Repeating the CRLF capture with and without `.git/info/attributes` produced byte-identical patches, SHA-256 `564a55922a0863c8b5779fa871986b4e08ebc933e0233ba2a34b275fa2965435`; the added line remained `changed\r\n`. The scratch repository prevents repository-local attributes/configuration from shaping staged bytes while retaining the documented working-tree `.gitattributes` and ignore semantics.

## New findings

### VD4R2-H-1 — `doctor --gate` reports a non-hook command as registered and exits 0

`registers_stop_hook` accepts any string containing `agent hook stop`. It does not establish that the shell command invokes DDF, names the current binary, names the admitted judge, or even reaches those words as executable tokens.

I placed this in the only Claude settings file visible to a scratch repository:

```json
{
  "hooks": {
    "Stop": [{
      "hooks": [{
        "type": "command",
        "command": "true # agent hook stop"
      }]
    }]
  }
}
```

The shell runs `true`; the remainder is a comment. DDF is never invoked. I used a stand-in judge that returned an accepted sealed fail on the policy probe solely to isolate the registration check, then ran:

```sh
HOME=/tmp/vesper-v4r2-doctor-home \
  deep-diff-forge agent doctor --gate \
    --judge /tmp/vesper-v4r2-sealed-judge --json
```

Observed:

```text
exit 0
"ok": true
chain:                  ok, exit 7
policy_protects_itself: ok
hooks_registered:       ok, /tmp/vesper-v4r2-doctor-home/.claude/settings.json
```

Thus the command advertised as checking “every link a judged stop depends on” can certify a setup with no stop hook. This is not just failure to authenticate a repository setting: an ordinary comment, echo, wrapper command, or different DDF/judge path produces a false ready result. Because `--gate` exposes exit 0 specifically for automation/readiness, I rate it High.

Fix: compare against a command shape that actually invokes the canonical current DDF binary with `agent hook stop --client claude` and the canonical judge selected by the gate. The safest contract is the exact shell-quoted command emitted by `agent setup --hooks`, with only explicitly supported additions; otherwise use a shell parser with a deliberately narrow accepted grammar and reject comments, pipelines, conditionals, substitutions, wrappers, and extra commands. Add negative tests for `true # agent hook stop`, `echo agent hook stop`, another binary, another judge, and a command containing the phrase only in an argument.

### VD4R2-M-1 — Scratch staging turns sparse-checkout omissions into deletions

The scratch repository runs `git read-tree <base>` into a fresh full index and then `git add --all` against the sparse working tree. It does not import the real index's skip-worktree bits or sparse patterns. Files intentionally absent from disk are consequently staged as deletions.

Reproduction in `/tmp/vesper-v4r2-sparse`:

1. Commit `warden.toml`, `kept.txt`, and `hidden.txt`.
2. Enable non-cone sparse checkout containing only `warden.toml` and `kept.txt`.
3. Change only `kept.txt`.
4. Run the Stop hook with a sealed capture judge.

The real repository remained:

```text
 M kept.txt
```

The judge patch was:

```diff
diff --git a/hidden.txt b/hidden.txt
deleted file mode 100644
--- a/hidden.txt
+++ /dev/null
@@ -1 +0,0 @@
-hidden base
diff --git a/kept.txt b/kept.txt
...
-kept base
+kept changed
```

This can falsely fail or hold a clean sparse worktree, and a large sparse repository can inflate the patch until acquisition becomes UNJUDGED. The original repository's index and status remained untouched, so this is patch truth rather than write-boundary corruption.

Fix: either detect sparse checkout/skip-worktree entries and refuse with an explicit UNJUDGED reason, or reconstruct the scratch index with the original index's skip-worktree state before `git add --all`. Do not copy staged content from the real index; only preserve which base paths are intentionally absent. Add cone and non-cone sparse tests where an included file changes and excluded base files must remain unchanged in the resulting tree.

### VD4R2-M-2 — A schema-invalid, internally contradictory “verdict” is accepted as PASS

`not_a_verdict` checks only `schema`, top-level `verdict`, three seal members, and the process exit. It does not validate the closed `forge-warden.verdict.v2` schema or even the document's required `exit` member. Rule objects are also read permissively with missing/wrong members becoming empty strings and lists.

I used an admitted stand-in that hashed the exact policy and patch, exited 0, and printed:

```json
{
  "schema": "forge-warden.verdict.v2",
  "verdict": "pass",
  "exit": {"code": 7, "name": "policy_fail", "action": "change_input"},
  "seals": {
    "policy_sha256": "<exact>",
    "patch_sha256": "<exact>",
    "provenance": "produced"
  },
  "rules": [{"id":"probe","outcome":"pass","why":"probe","violations":[]}]
}
```

It also omitted required `tool`, `evaluated_on`, `summary`, waiver, note, rank, ranked-by, and Git-seal members. The Stop hook exited 0 with **zero stdout bytes**, accepting PASS despite the document itself saying exit 7/policy failure and being invalid under the named v2 schema.

A deliberately malicious selected judge can forge the checked seals, as the change documents. This finding is narrower: malformed or contradictory output from an otherwise selected judge is treated as a verdict rather than the promised accidental-output UNJUDGED path. Replay protection is likewise only policy/patch scoped; the code ignores the document's evaluated date and ranked-by identity.

Fix: deserialize a strict typed v2 document (or validate the shipped v2 schema) and enforce its cross-field invariants: process exit, `verdict`, and document `exit` must agree; required objects and member types must be present; seal digests must have the schema's lowercase-hex shape; and the ranked-by identity/date policy should be explicitly checked or explicitly documented as unchecked. Reject duplicate keys before or during strict parsing. Add the contradictory-exit and missing-required-members cases to all exits 0/7/8.

## Other verification

- Direct prebuilt test executables passed: `agent_hook` 14/14, `judge_patch` 6/6, contract 19/19, and graph 113/113.
- The fake first-PATH judge, state-root symlink, huge state, and local-attributes round-one probes were rerun on the release binary as described above.
- Setup JSON named the discovered judge both in top-level `judge` and the SessionStart/Stop `--judge` arguments.
- The sparse probe confirmed the source repository's index, refs, object store, and status were not modified by patch construction.
- The new tests are regression-sensitive by inspection: the sealed-document and state tests fail or do not compile against the old interfaces; the local-attributes test reproduces VD4-M-2; `agent doctor --gate` and its checks do not exist at `aef898f`; the large-state and fake-PATH cases reproduce the old behavior.

## Not examined

- I did not exhaustively test submodules, untracked nested repositories, SHA-256 repositories, linked-worktree excludes, file/directory type changes, sparse-index performance, or every `.gitignore` negation.
- I did not exercise disk-full, rename failure, simultaneous hooks for one session, PID reuse after a killed hook, or directory-swap races beyond the static symlink and special-file logic.
- I did not fuzz JSON duplicate-key behavior, every malformed verdict member, 4 MiB output boundaries, or every valid hold/fail shape without matching rules.
- I did not test every Claude Code settings merge/matcher form or shell dialect; the no-op command is sufficient to establish the false-ready result.
- I did not run Cargo, Clippy, Miri, sanitizers, or fuzzers. The hive slot was free, but the supplied release binary and current prebuilt test executables covered the focused review without rebuilding.
- I did not modify repository files, refs, remotes, index, configuration, or existing worktree content apart from this report. All reproductions were under `/tmp`.

STATUS: COMPLETE
