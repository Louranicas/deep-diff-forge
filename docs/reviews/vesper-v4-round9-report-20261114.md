# Vesper review: V4 round 9 (2026-11-14)

Reviewed: committed snapshot `40f5554..1c7f8c1` on `agent/loop`, using the frozen round-9 binaries named in the request. I also reconciled the dispositions from the incremental V4 reviews over `d4dbc91..1c7f8c1`. I wrote only this report in the repository.

## Verdict

**GO.** VD4R7-L-1 is closed for both sources of ranker candidates. A non-UTF-8 directory entry takes no place, and a non-UTF-8 path for this binary now takes no place either; in the latter case all eight places become available to representable directory candidates. I found no new High, Medium or Low finding.

No finding from V4 rounds 1-8 remains open. My verdict for `agent/loop` as a whole, `d4dbc91..1c7f8c1`, is **GO**.

## VD4R7-L-1 disposition: closed

`rankers` now filters the canonicalized `current_exe()` path on `to_str().is_some()` before collecting it. The remaining capacity is computed after that filter. Directory candidates still pass through `offerable`, which checks representability before `is_file` and before insertion into the bounded heap.

I copied the frozen `1c7f8c1` release binary to a Linux path ending in raw byte `0xff`, created UTF-8 regular files `b1` through `b9`, and invoked the copy as the helper. It returned exactly eight entries, `b1` through `b8`; every `path` was a JSON string, there was no null entry, and `b9` was the expected ninth candidate beyond the limit.

I also reran the directory-side reproduction with the normally named frozen binary, a directory entry named `a<0xff>`, and `b1` through `b8`. It returned this binary followed by `b1` through `b7`, all as string paths. The unrepresentable directory entry consumed no place.

Finally, I ran a complete Stop hook from the non-UTF-8 binary with no `--ranker-dir`. It remained explicitly fail-visible:

```text
deep-diff-forge: UNJUDGED. This change (changes since HEAD) was not judged: this binary's own path could not be found, so no ranker could be offered. That is not a pass.
```

The wording groups an unrepresentable path with a missing path, but it is clear about the operational result: no candidate was offered, the change is UNJUDGED, and the result is not a pass. I do not consider that an open integrity issue.

## Branch-level disposition

The incremental review chain now closes every finding I recorded:

- V4 round 1: `VD4-H-1`, `VD4-M-1`, and `VD4-M-2` are closed by sealed verdicts and pinned judge identity, bounded/no-follow state handling, and isolated patch acquisition.
- V4 rounds 2-3: `VD4R2-H-1`, `VD4R2-M-1`, `VD4R2-M-2`, `VD4R3-H-1`, and `VD4R3-M-1` are closed by exact hook-command validation, sparse-tree refusal, and whole-schema verdict validation.
- V4 rounds 4-6: `VD4R4-M-1`, `VD4R4-M-2`, and `VD4R6-M-1` are closed by optional-member validation, killable bounded hashing, and constant-storage directory selection.
- V4 rounds 7-9: `VD4R7-L-1` is now closed for both directory entries and the helper's own executable.

The branch continues to fail visibly: missing/unusable judge, policy, ranker, helper timeout, or unsealed/invalid verdict is UNJUDGED rather than PASS; a policy failure blocks once; ranker candidates are hashed under size and process deadlines; directory storage is constant in the number of entries.

## New findings

None.

## Tests and checks

The hive slot reported `free` immediately before Cargo. Cargo ran in `fedora-toolbox-44` with the review-owned target directory `/tmp/vesper-v4r9-target`.

- All 18 `deep-diff-forge-cli --test agent_hook` integration tests passed, including both non-UTF-8 candidate-source regressions.
- Frozen-release-binary reproductions covered the non-UTF-8 own path with nine fallbacks, the non-UTF-8 directory entry, and the complete no-fallback Stop result.
- `git diff --check 40f5554..1c7f8c1` passed.
- `git diff --check d4dbc91..1c7f8c1` passed for the complete reviewed branch range.

## Not looked at / limitations

- I did not reread all 8,548 inserted lines in `d4dbc91..1c7f8c1` from scratch this round; the branch-level verdict relies on the nine incremental V4 reviews and their reproduced rechecks.
- I did not rerun the 500,000-entry memory measurement, blocking-read interposer, every historical malformed-verdict probe, or the complete real-forge-warden matrix in this round. Those paths were exercised in the rounds that introduced and closed their findings.
- I did not run the full workspace suite, Clippy, Miri, fuzzers, or the complete local CI gate.
- I did not run a live Claude Code session or test macOS/Windows. The raw-byte filename reproductions are Linux-specific.
- I did not review changes outside `40f5554..1c7f8c1` anew, or modify repository refs, remotes, index, configuration, or existing files.

STATUS: COMPLETE
