# Vesper review: V4 round 8 (2026-11-14)

Reviewed: committed snapshot `7defe1f..40f5554` on `agent/loop`. I wrote only this report in the repository.

## Verdict

**NO-GO.** The fix works for entries read from `--ranker-dir`, but VD4R7-L-1 is not fully closed. The helper's own executable is inserted before the new `offerable` predicate and still consumes one of the eight places when its path is not UTF-8. The helper serializes that path as JSON `null`, the parent drops it, and only seven UTF-8 directory fallbacks are available; the eighth offerable fallback is still suppressed.

This remains Low severity and fail-visible, but it is the same availability defect in a special candidate source explicitly named by this round's attack list.

## VD4R7-L-1 disposition: not closed

The directory case is fixed correctly. `rankers` passes `offerable` into `first_by_name`, and `offerable` checks `to_str().is_some()` before `is_file()`. The supplied Linux regression passed and returned `b1` through `b7`; the non-UTF-8 directory entry took no place and incurred no `stat`.

The own-binary path bypasses that predicate:

- `rankers` inserts canonicalized `current_exe()` directly at `hook_run.rs:508-512`.
- Its presence reduces the directory capacity with `RANKERS_TRIED.saturating_sub(list.len())` at lines 516-519.
- `seal_rankers` still converts every selected path with `to_str()` at line 703, producing JSON `null` for this binary.
- `sealed_rankers_by` drops a non-string path at line 758.

I copied the supplied `40f5554` release binary to a Linux path ending in the raw byte `0xff`, created UTF-8 regular files `b1` through `b8`, and invoked that copy as:

```text
agent hook seal-rankers --ranker-dir /tmp/vesper-v4r8-own-name/rankers
```

The helper returned eight entries: one `{"path":null,"sha256":…}` for itself and string paths `b1` through `b7`. It returned seven offerable paths; `b8` was absent. This is the same crowd-out as round 7, now caused by the candidate inserted outside the fixed filter.

Without `--ranker-dir`, I also ran a complete Stop hook from the non-UTF-8 binary against a changed Git repository. It returned:

```text
deep-diff-forge: UNJUDGED. This change (changes since HEAD) was not judged: this binary's own path could not be found, so no ranker could be offered. That is not a pass.
```

Thus the no-fallback form is safely fail-visible and never becomes a pass, as requested. With a ranker directory, however, the parent can offer only seven fallbacks and cannot reach an eighth ranker that might be the first one admitted by policy.

Recommended fix: make representability part of the own-executable insertion too. Add the canonicalized executable only when `to_str().is_some()` (it is already known to be the running regular file), then let `RANKERS_TRIED.saturating_sub(list.len())` admit eight directory candidates when it cannot be serialized. Preserve the current explicit UNJUDGED result when no offerable candidate exists. Add a Linux integration test that launches a copied binary from a non-UTF-8 path with eight UTF-8 fallbacks; the present regression launches from Cargo's ordinary UTF-8 binary path, so it cannot cover this branch.

## Neighbouring cases

- A NUL byte cannot occur in a filesystem path component.
- Quotes, backslashes, control characters and newlines in valid UTF-8 names are escaped by `serde_json` and decoded back to the same string by the parent; I found no lossy transformation for representable names.
- At most eight paths are emitted. Ordinary platform path/component limits keep this well below the helper's 256 KiB stdout cap; I found no separate parent path-length rejection that should be part of selection.
- A selected entry that vanishes, becomes non-regular, exceeds 256 MiB or misses the hashing deadline remains present with a refusal reason. That consumes its place by the documented design and is not silently discarded.
- The constant-storage selection and eight-second helper supervision are unchanged. I found no recurrence of VD4R6-M-1 or the hashing-deadline issue.

## New findings

None. The surviving defect is VD4R7-L-1 rather than a new round-8 finding.

## Tests and checks

The hive slot reported `free` immediately before each Cargo invocation. Cargo ran only in `fedora-toolbox-44` using the supplied target directory.

- The targeted `a_name_the_helper_cannot_write_takes_no_place` regression passed.
- All 17 `deep-diff-forge-cli --test agent_hook` integration tests passed.
- The supplied release binary reproduced the non-UTF-8 own-executable crowd-out and the complete Stop hook's explicit UNJUDGED/no-pass behavior.
- `git diff --check 7defe1f..40f5554` passed.

## Not looked at / limitations

- I did not rerun the 500,000-entry memory measurement, the blocking-read interposer, or the full workspace/CI suite; the bounded heap and child deadline were unchanged.
- I did not construct a complete sealed forge-warden verdict where only omitted `b8` is admitted; the helper's emitted candidate list directly reproduces the prerequisite omission.
- I did not test non-UTF-8 executable paths on macOS or Windows. The reproduction and proposed regression are Linux-specific, matching the filesystem capability used by the round-7 test.
- I did not review changes outside `7defe1f..40f5554`, modify refs, remotes, index, configuration, or existing repository files.

STATUS: COMPLETE
