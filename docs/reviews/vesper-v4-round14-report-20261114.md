# Vesper V4 round 14 review (2026-11-14)

## Verdict

**NO-GO** on `c6c4f28` and on `agent/loop` as a whole (`d4dbc91..c6c4f28`).

The new invariant is a substantial improvement: I did not find a silent stop with a known non-pass, and the one-failure retry cases from round 13 and HR5 are repaired. One ordinary retry-exhaustion path still permanently prevents a real failing verdict from being obtained, however. The stop continues to say `UNJUDGED`, so this is not the old silent-pass bug, but the hook never blocks for the failure even after the judge has recovered.

## Prior findings

- **VD4R13-M-1: closed in its reported one-transient-failure form.** A commit whose first judging gets no verdict is named, retried, and can then block. The focused test suite covers this, and I repeated the broader mechanism with the frozen binary.
- **HR5-H-1: closed.** A commit with no verdict remains a unit and is named at every later stop; later stops are not silent merely because the first judging produced no verdict.
- The other HR5 findings named by this request appeared addressed in the reviewed paths and focused tests. I found no regression in staged identity, persistent holds/failures, intent-to-add handling, bounded messages, or the documented gate/start cases.

## New finding

### VD4R14-M-1 — ordinary workflow: five patch-specific temporary errors permanently suppress a subsequently available FAIL

`ATTEMPTS` is five. Once a saved `NoVerdict` reaches that count, `sorted_commits` leaves the commit as a visible unit but does not put it in `ask` again (`hook_run.rs`, `ATTEMPTS` and `sorted_commits`). An attempt counts whenever *some* patch or the probe got a verdict at that stop, not when the affected patch's failure was shown to be permanent.

That distinction is insufficient for a judge/ranker failure which affects one patch while other patches work. I reproduced this with the frozen release binary and the real judge:

1. Start a session, commit a credential patch, and then make one harmless commit before each of six stops.
2. A judge wrapper returns the ordinary structured temporary I/O error for the credential patch on its first five calls and delegates every other call to the real judge. Each harmless commit passes, so the chain is considered working and each credential error consumes an attempt.
3. Stops 1 through 5 all allow and say the credential commit is `UNJUDGED`. After stop 5 the judged file records `n5`.
4. At stop 6 a fresh harmless commit is judged, but the credential commit is not submitted. The wrapper's credential-call count remains five. The stop again allows and says: `could not be judged in 5 attempts`.
5. A seventh stop likewise leaves the credential-call count at five.
6. Running the real judge directly on the exact credential commit patch at that point returns `policy_fail` (exit 7), rule `no-secrets`, for `lib/credential.rs`.

Thus recovery after the fifth patch-specific failure cannot be observed. A commit that really fails is permanently prevented from reaching the failing verdict and never blocks a stop. Continuous `UNJUDGED` output keeps claim 1 intact, but it does not satisfy claim 2 or the requested attack-B property.

The attempt cap should not make a non-final environmental result operationally final. A bounded design could slow retries after five attempts, require an explicit human/session action to abandon them, or retain a low-frequency probe/retry; it should not make retry impossible while the session lives.

Reproduction artifacts are under `/tmp/vesper-v4r14-attempts/`. The decisive outputs are `stop1.out` through `stop7.out`, `credential-count3` (still `5`), the saved judged file under `home3`, and `credential.patch`.

## Claims

1. **Silent only when everything known has passed: held in the reviewed and reproduced cases.** The exhausted commit remains named at every stop. I found no silent known non-pass.
2. **Nothing is final that an outage caused: broken by VD4R14-M-1.** The stored outcome remains labelled `UNJUDGED`, but five temporary per-patch errors make the no-retry decision permanent; a later real FAIL is never obtained.
3. **Bounded resources: held for the paths inspected and focused tests.** The message builder has a single character budget, state/list readers remain bounded, and judging uses the stop deadline. I did not independently stress every maximum-size state and repository shape.
4. **Rounds 10–13 stand: held within this round's scope.** I found no new defect in unit selection, base-policy use, per-commit patches, or sealed-verdict handling. VD4R14-M-1 concerns when an unsealed/no-verdict patch is ever resubmitted.

## Persistent-line noise (question D)

A held, standing-fail, or unjudged unit should remain visible at every stop. Silence otherwise becomes indistinguishable from an all-pass state, and an agent can lose the only live warning across turns. The current bounded approach is directionally right: show actionable current blockers first, compact stable units to identity/outcome lines, and aggregate overflow with an explicit count. I would not make a still-true unit disappear merely because it was reported before.

## Checks performed

- Verified `HEAD` is `c6c4f282de1188583abf60f3e5d63a5e2b4d698c`; worktree was clean before the report.
- Reviewed `651adae..c6c4f28`, the request, the fifth independent review, retry/state selection, message construction, staged/uncommitted identity, notes, and relevant documentation/tests.
- Verified frozen binary hashes: debug `52352311b66855cf5f970eb6adf0e14b5ae05a1900a9fdc90fd2298ecbd3925e`; release `ecb8605c3c0d56a59eaee649e57c0aade137b8e4940ba602f9e31a06a1aed467`.
- Reproduced VD4R14-M-1 with the frozen release binary and real judge `55bf9fe6d19d7f7b66ef7cc7f3056ff9ab7ba7dc10b0e310d59a2aafd33874c9`.
- Checked the hive slot before Cargo; it was free. Ran `cargo test -p deep-diff-forge-cli --test agent_hook` in `fedora-toolbox-44`, with the launcher using the real HOME and a private HOME only inside the container command: **78 passed, 0 failed**.

## Not looked at

- I did not rerun the full 2,116-test workspace suite or the complete local gate.
- I did not exhaustively repeat all 30 prior real-judge workflows; I relied on source review, their committed regressions, the focused 78-test suite, and the new adversarial workflow above.
- I did not independently stress sparse indexes, every filesystem failure/crash point, all message-cap combinations, or repositories at every stated global bound.

STATUS: COMPLETE
