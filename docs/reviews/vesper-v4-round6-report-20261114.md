# Vesper review: V4 round 6 (2026-11-14)

## Verdict

**NO-GO.** The remaining point of VD4R4-M-2 is closed: a killable blocking read is now bounded by the parent and fails visibly. The new helper retains an independent medium resource problem, however: it collects every regular directory entry before truncating the candidate list to eight, so a hostile ranker directory can consume memory without a deterministic bound inside the eight-second window.

## Round 4 disposition

### VD4R4-M-2 — Closed

I reran the round-five delayed-read reproduction against the supplied debug binary. The preload fixture delayed the helper's read of a small regular candidate by ten seconds. The Stop hook killed and reaped the helper and returned explicit UNJUDGED after **8.03 seconds**, rather than waiting 10.90 seconds as `42ac26e` did. No `seal-rankers` child remained.

I repeated the same condition through `agent doctor --gate`. Its chain failed with the helper's eight-second timeout reason, `policy_protects_itself` was not checked, and the command returned after **8.02 seconds**. Thus both consumers use the bounded child path.

The child-output boundary is otherwise coherent by inspection and exercise:

- the parent starts this binary by its canonical path and passes only `--ranker-dir`;
- quote and newline characters in file names are JSON-escaped and recovered as the same paths;
- the helper returns this binary plus at most seven directory entries, in name order;
- a later path swap either makes forge-warden refuse the file or makes its pinned digest differ from the helper's digest;
- malformed, oversized, timed-out or absent helper output becomes UNJUDGED / a failed gate, not a pass.

The documented uninterruptible-kernel-I/O case remains a stated platform limit, not a regression in this fix.

## New findings

### VD4R6-M-1 — Candidate truncation happens after an unbounded directory collection

`rankers` reads the entire `--ranker-dir`, converts every successful entry to a `PathBuf`, calls `is_file`, and `collect`s every regular path. Only after that allocation does it sort the vector, append it, and truncate the combined list to `RANKERS_TRIED` (eight).

I reproduced the growth with empty regular files, so hashing cost and file contents were not factors:

| Directory entries | Helper result | Elapsed | Maximum RSS |
|---:|---:|---:|---:|
| 100,000 | 8 candidates | 1.01 s | 19,728 KiB |
| 500,000 | 8 candidates | 1.79 s | 69,640 KiB |

The complete Stop path over the 500,000-entry directory likewise returned only the normal UNJUDGED judge refusal, but peaked at **69,644 KiB** in **1.92 seconds**. The allocation is approximately linear in the number and length of entry paths. On this filesystem, far more entries can be accumulated before the eight-second parent deadline, reaching hundreds of MiB; the time limit kills a slow child but is not a memory limit and cannot undo system memory pressure or an OOM kill.

Impact: a ranker directory under change-controlled or otherwise hostile storage can make a Stop hook consume unbounded memory even though only seven of its files can ever be offered. This reintroduces the resource-exhaustion class that moving hashing into a bounded helper is meant to contain.

Recommended fix: select the lexicographically first `RANKERS_TRIED - 1` regular directory paths with bounded storage while enumerating—for example, a max-heap of at most seven `PathBuf`s, replacing its largest member when a smaller path is found. Then sort only those seven. Alternatively, impose a documented entry-scan cap and fail visibly when it is crossed. Keep enumeration inside the supervised child for the wall-clock bound.

The 500,000-file scratch fixture was removed after measurement; it contained only empty files created for this review.

## Other checks performed

- Read `42ac26e..08332e0`; `git diff --check 42ac26e..08332e0` passed.
- Ran the helper directly with quoted and newline-containing names, a link, and more than eight regular entries; its JSON remained valid and its result length was eight.
- Inspected `core::process::run_bounded`: timeout kills and reaps the direct helper and detaches pipe readers, so a descendant holding a pipe cannot extend this deadline. The seal helper itself creates no descendants.
- Checked helper output caps, non-JSON/empty output, nonzero-exit behavior, and the parent path/digest handoff by inspection. The production helper cannot normally emit a complete list and then exit nonzero; a nonzero helper with no complete list is rejected.
- Checked the hive slot immediately before each Cargo invocation; it was free.
- In `fedora-toolbox-44`, `the_helper_that_hashes_rankers_is_ended_at_the_deadline` and the integration test `the_seal_helper_lists_each_ranker_with_its_digest_or_its_refusal` passed.

## Not looked at / limitations

- I did not mount a genuinely uninterruptible hard network filesystem; that stated limit cannot be reproduced safely here.
- I did not run a multi-million-entry test because 500,000 entries already established the linear allocation and a 69 MiB peak. The extrapolation above is from the measured slope and enumeration rate.
- I did not run a scheduler-controlled path-swap race or the full real-forge-warden matrix.
- I did not run the full workspace test suite.
- I did not review changes outside `42ac26e..08332e0`.

STATUS: COMPLETE
