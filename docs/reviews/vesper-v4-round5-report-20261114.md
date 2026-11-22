# Vesper review: V4 round 5 (2026-11-14)

## Verdict

**NO-GO.** VD4R4-M-1 is closed. VD4R4-M-2 is substantially improved on size and memory, but is not closed: the advertised eight-second hashing deadline cannot interrupt a blocking file read and can be exceeded arbitrarily. I found no separate round-5 finding requiring a new `VD4R5-*` ID.

## Round 4 disposition

### VD4R4-M-1 — Closed

The validator now checks each known optional member from the reproduction in its schema-defined form. I reran the stand-in judge against both supplied binaries:

- the original document with `claims.base: "NOT-HEX"` now returns explicit UNJUDGED, naming malformed `seals.claims`;
- after making that claim valid while retaining `evidence: "not-an-array"`, `evidence_truncated: false`, and `waived_by: -1`, both builds return explicit UNJUDGED, naming malformed rule violations.

By inspection against forge-warden 0.2.0's `verdict.v2.schema.json`, the new code also checks citation address, side and positive line, optional claim `head`, and the other known nested members. The deliberately forward-compatible handling of unknown members, new lowercase rule kinds and new lowercase note codes is unchanged.

### VD4R4-M-2 — Not closed: a blocking read defeats the deadline

The size and memory parts are fixed. With the supplied debug binary, a 256 MiB + 1 byte sparse ranker was rejected before its contents were read; the whole Stop invocation completed in **0.86 seconds** at **9,956 KiB maximum RSS**, with an explicit UNJUDGED reason naming the file and the 256 MiB limit. The new streaming SHA-256 has constant-sized buffering, and its state/padding logic agrees with the existing one-shot implementation at the exercised block and padding boundaries.

The time bound is not hard, however. `file_sha256` checks `now() >= deadline` before calling synchronous `File::read`, then cannot check again until that call returns. `O_NONBLOCK` is ignored for ordinary regular files and does not make a slow FUSE/NFS read cancellable. Directory enumeration and `is_file` metadata lookup in `rankers` are synchronous for the same reason.

I reproduced the direct read case with a preload interposer that delayed exactly one `read` of a small regular candidate by ten seconds, modelling a stalled backing filesystem. The configured limit was eight seconds, but the supplied debug Stop hook returned only after **10.90 seconds**. It then emitted the expected UNJUDGED deadline reason. Raising that one blocking delay can carry the hook past Claude's 60-second hook timeout, at which point the client can discard the process before the fail-visible message is emitted.

This is a continuation of VD4R4-M-2 rather than a new finding: the resource-bounding fix does not yet bound wall-clock time in the condition it claims to handle.

Recommended fix: perform ranker enumeration/open/hash work in a separately supervised process that can be killed and reaped at the deadline (or another genuinely cancellable I/O boundary). Checking `Instant` around blocking filesystem calls and setting `O_NONBLOCK` on a regular file is not sufficient. Preserve the new per-file cap and streaming hash inside that boundary.

## Other checks performed

- Read `6d7db10..42ac26e`; `git diff --check 6d7db10..42ac26e` passed.
- Compared every validator branch with the forge-warden 0.2.0 schema, including claims, citations, waiver notes, notes, summary integers, exit tuples and Git admission.
- Reviewed the streaming SHA-256 update/finalization logic around 55/56/63/64-byte padding and block boundaries. The `u64` wrapping byte count gives SHA-256's length modulo 2^64 bits; a stream over 2^32 bytes does not encounter the former 32-bit boundary.
- Checked the profile change: package `opt-level = 3` does not disable overflow checks or debug assertions in the dev profile. I found no behavior relying on unoptimized core code.
- Checked the hive slot immediately before each Cargo invocation; it was free.
- In `fedora-toolbox-44`, the streaming-hash comparison test, the optional-schema-member test, and the ranker size/time unit test all passed.

## Not looked at / limitations

- I did not run a real FUSE or remote NFS server; the blocking-read result uses a controlled libc interposer so the delay is deterministic and attributable to the one ranker read.
- I did not empirically hash more than 2^32 bytes; that boundary was reviewed from the `u64` length arithmetic.
- I did not run a scheduler-controlled shrink/grow/symlink race. By inspection, a changed file is refused by forge-warden or produces a digest mismatch unless the later bytes are again identical.
- I did not rerun the full real-forge-warden pass/hold/fail matrix or the full workspace test suite.
- I did not revisit the stated canonical-path replacement and non-UTF-8 limitations from round 4.
- I did not review changes outside `6d7db10..42ac26e`.

STATUS: COMPLETE
