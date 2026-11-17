# Vesper review: V4 round 4 (2026-11-14)

## Verdict

**NO-GO.** Both V4 round 3 findings are closed, including the shell-command ambiguity and the invalid document used in that round. Two narrower problems remain in the new code: known optional members of the v2 schema are still not validated, and hashing a candidate ranker is outside every byte, memory and time bound.

## Round 3 disposition

| Round 3 finding | Status | Evidence |
|---|---|---|
| VD4R3-H-1 | **Closed** | I reran the assignment reproduction with the supplied `6d7db10` debug binary. `hooks_registered` is now false and identifies `A=ddf` as not naming this program by its own absolute path. I also copied the binary and judge beneath a canonical path containing spaces and a literal apostrophe, generated the hook with `agent setup --hooks`, and ran `doctor --gate`: `chain`, `policy_protects_itself`, and `hooks_registered` were all true. The emitted `'/path/it'\''s …'` form is decoded to the exact words a POSIX shell runs. Relative and linked program spellings are rejected by the same exact comparison. |
| VD4R3-M-1 | **Closed** | The round 3 stand-in document now produces an explicit UNJUDGED response (`evaluated_on` is not a date) instead of a zero-byte PASS. The new validator checks the required decision-bearing members, enums, exit tuple, nonempty rules, and summary counts used in that reproduction. |

Replacing the binary or judge at its canonical pathname after the gate remains a check/use limit: the later shell invocation runs what then occupies that path. I did not count that separately because the request identifies replacement after the gate as a stated limit, and the fix eliminates the gate's prior disagreement with an unchanged shell command.

## New findings

### VD4R4-M-1 — Known v2 violation and claim members remain unchecked

`verdict_v2::rule_defect` validates only a violation's `message` plus optional string `path` and `signal`. It does not validate the schema-defined `evidence`, `evidence_truncated`, or `waived_by` members. `seals_defect` similarly requires `claims` to be an object with `verified: false`, but does not validate the schema-defined `base` or `head` commit patterns.

I reproduced this end to end with `/tmp/vesper-v4r4-invalid-nested-judge`. It emitted an otherwise sealed PASS with one `waived` rule and these known members:

- `claims.base: "NOT-HEX"` (not 4–64 lowercase hexadecimal digits);
- `violations[0].evidence: "not-an-array"`;
- `violations[0].evidence_truncated: false` (the schema permits only `true` when present);
- `violations[0].waived_by: -1` (the schema requires a nonnegative integer).

Both supplied debug and release hooks exited 0 and emitted zero bytes, accepting the schema-invalid waiver as PASS. This does not depend on the intentionally open treatment of unknown future members, rule kinds, or note codes: every malformed member above is already named and constrained by `verdict.v2.schema.json`.

Impact: the hook still treats a malformed v2 waiver/evidence record as a whole authority verdict and silently allows the stop. This is narrower than VD4R3-M-1 because the outcome and exit tuple are now coherent, but it leaves the advertised member-by-member trust-boundary check incomplete.

Recommended fix: validate every present, known v2 member: claim commit patterns; evidence as an array of citations with SHA-256 address, `new|old` side, and line at least 1; `evidence_truncated` as exactly `true`; and `waived_by` as a nonnegative integer. Unknown members can remain accepted under the documented forward-compatibility policy.

### VD4R4-M-2 — Ranker sealing reads each candidate wholly, with no resource bound

`hook_run::file_sha256` uses `std::fs::read(path)` before the bounded judge process starts. Candidate size, aggregate bytes, memory, and elapsed time are therefore outside `JUDGE_LIMITS` and the hook's own Git/process deadlines. Up to seven `--ranker-dir` entries can be reached after the built-in ranker is refused with exit 4.

I reproduced the cost with a rejecting stand-in judge and one 256 MiB sparse regular file in `--ranker-dir`. The supplied debug hook took **28.30 seconds** and reached **271,880 KiB maximum RSS** before returning UNJUDGED. One more same-sized candidate is enough to consume nearly the entire 60-second Claude Stop-hook allowance on this machine; additional or larger candidates can make the client discard the hook before it can emit its explicit UNJUDGED notice. This is a regression in the new sealing path: the bytes are read into one `Vec` before forge-warden can apply its own executable-size and child-process bounds.

Impact: a large candidate in a configured ranker directory can cause high transient memory use or make a Stop hook disappear at the client timeout, defeating the fail-visible behavior this work package otherwise establishes.

Recommended fix: open and hash rankers through a bounded, streaming path, impose a per-file and aggregate byte budget, and include hashing in an elapsed-time budget that leaves time for the judge and for emitting UNJUDGED. A refused or over-budget candidate should produce the existing explicit unreadable/unusable-ranker reason rather than consuming the client timeout.

## Other checks performed

- Read `045ef60..6d7db10`; `git diff --check 045ef60..6d7db10` passed.
- Reproduced both round 3 findings against the supplied debug binary and the remaining nested-document acceptance against both supplied binaries.
- Exercised setup and gate with canonical executable and judge paths containing spaces and an apostrophe.
- Inspected the forge-warden 0.2 ranker door: it pins one opened descriptor, refuses a final symlink, hashes that descriptor, and runs `/proc/self/fd/...`. Together with the new digest comparison, ordinary path/link swaps become a mismatch or a refusal; I found no digest substitution from that race by inspection.
- Checked the hive slot immediately before Cargo; it was free. In `fedora-toolbox-44`, `cargo test -p deep-diff-forge-cli verdict_v2` passed (3 tests), and `cargo test -p deep-diff-forge-cli --test agent_hook` passed (15 tests).

## Not looked at / limitations

- I did not rerun the full real-forge-warden pass/hold/fail/policy-change matrix recorded by the author.
- I did not run the full workspace test suite, fuzzing, or platform-specific tests outside Linux.
- I did not construct a scheduler-controlled ranker swap race; the race assessment above is source inspection plus the digest relationships, not a timing proof.
- I did not test a non-UTF-8 canonical executable path. The gate uses `Path::to_str`, so such an installation cannot be certified; setup already serializes paths lossily. I treated this as a portability limitation rather than a merge-blocking regression for the requested Linux path cases.
- I did not review changes outside `045ef60..6d7db10`.

STATUS: COMPLETE
