# Vesper review V3, round 14 — 2026-11-15

## Verdict

**NO-GO.** The scanner, placement and joining changes close the four findings this round set out to answer. One medium finding remains in the claimed public-output monotonicity and its supporting measurement: the 32-item list necessarily drops some items shown by round 13 when it reserves places for later v0.8.3 items. Scores and file signals do not regress in the reproductions, and the v0.8.3 compatibility claim held.

Reviewed commit `87d1afd7e0e8f337ea9172370799ee5ce174f1fe` against `8f290e3` in the clean `agent/integrity-signals` worktree. Frozen release SHA-256 was `11ae63e4e560bef15c60f1a87a3a493c5fb40f9f966500c2ed025ec20b4b707b`.

## Prior findings

- **VD3R13-M-1: closed as a declared limit.** A fragment beginning two or more block comments deep remains a miss, but the false “never later” assertion is gone. README, changelog, corpus prose and code comments state the limit and the one-level reading accurately. The full-context implementation and its focused tests support the stated general answer: when the hunk reaches the file top, the start state is known.
- **IR6-H-1: closed.** `placed_at_top` never consults `new_start`. The three supplied false-`+1` patches (`lie1`, `lie2`, `lie3`) all recovered `process_spawn` with the frozen binary. The intentional `h/asym.patch` change went the other way: round 13 reported the spawn and round 14 did not, matching the corrected old-side placement rule.
- **IR6-H-2: closed.** `snap2.patch` recovered `test_skipped@18`; `strcmt.patch` recovered `process_spawn@7`. The further string starts are additive and are triggered by any state carried over a line end, including a block comment.
- **IR6-H-3: closed.** `split4.patch` recovered the cross-hunk `panic_path_added`; `newfile-depth2-joined.patch` retained its spawn. The rank, outline and an address-selected hunk agreed on the joined call's label.

## New finding

### VD3R14-M-1 — the bounded public answer is not monotone from round 13, contrary to Claim 1 and the stated strict measurement

The new list priority correctly keeps v0.8.3 items, but a 32-item output cannot in general keep both every item round 13 listed and every later v0.8.3 item. `listed` reserves places for all `Evidence::released` items before retaining the remaining items in file order. Consequently it displaces round-13 evidence at the cap.

Reproduction:

```text
patch: /tmp/claude-ddf-indep-review-6/hist/098-d239165.patch
file:  crates/deep-diff-forge-cli/tests/repo_scripts.rs

v3r13 listed: panic_path_added new line 330
v3r14 listed: no line 330; process_spawn new line 368 occupies that place
v0.8.3 listed: process_spawn new line 368
```

Both round 13 and round 14 have score 10, the same file signals, `evidence_truncated: true`, and 32 listed items. Thus this is not a lost internal signal or a lower score; it is a lost public evidence location. On all 621 patches under `/tmp/claude-ddf-indep-review-6`, the request's own strict comparator reported:

```text
r13->r14: patches 621, with a lost item 14, with a gained item 34,
           refused by both 23, refusal differs 0
```

One loss is the declared `h/asym.patch` placement correction. The other 13 are cap displacement in history patches, including repeated losses in `repo_scripts.rs`, `git_acquire.rs`, `council_tuicr.rs`, and `council.rs`. This contradicts “Round 13's answers are a subset” and the measured claim that strict round 13 to round 14 has only the asymmetric-top loss.

The v0.8.3 goal itself held on the same set:

```text
v083->r14: patches 621, with a lost item 0, with a gained item 380,
            refused by both 23, refusal differs 0
```

Recommended resolution: narrow Claim 1 explicitly to the uncapped evidence set, scores and signals; state that the bounded list can displace round-13 items to preserve v0.8.3; correct the measured paragraph; and add a cap regression that demonstrates this unavoidable tradeoff. If public listed-item monotonicity from both versions is required, the fixed bound or the representation must change.

## Claims

1. **Monotone — broken as written.** Source inspection and focused cases support that readings union their findings and that joins take no uncapped item away. The public bounded answer is not a round-13 superset, as VD3R14-M-1 shows. The asymmetric-top loss is otherwise reproduced and intentional.
2. **Never lower than v0.8.3 — held for the tested surface.** Strict comparison over 621 supplied patches found no lost v0.8.3 item, no changed refusal, and 380 patches with gains. The implementation marks the legacy Rust reading and takes released items first at the bound. I did not exhaust all 3,040 inputs claimed by the request.
3. **Placement is Git's — held within the declared plain-`git apply` model.** The supplied false-new-number and split-hunk cases match Git's old-side placement. I also confirmed that plain Git refuses an old-position-1 hunk whose context exists only lower, while `-C0` applies it lower; that exception is declared. This was not an exhaustive proof over every accepted hunk form.
4. **Rounds 11 to 13 stand; cost remains linear — held for source inspection and sampled runs.** `texts` builds at most the alone, must-join and positional-join readings; their chains are disjoint within each class. Address selection is one pass, joined membership is cached once per file, and an unjoined selected hunk is scanned alone. Large supplied inputs of about 19–61 MB completed in about 1.5–6.9 CPU seconds; focused graph and daemon tests passed. I did not repeat the 330,000-hunk or ten-input memory measurements.
5. **The corpus says what the binary does — partially held, not exhaustively tested.** Selected positive and declared-negative fixtures matched, and focused tests passed. The request's separate strict-comparison measurement is not truthful for the 621-patch subset, per VD3R14-M-1. I did not independently recount all 49 corpus cases.

## Option not taken

I agree with not adding a blanket patch-order reading. Patch order is not file order, so that reading would add another writer-controlled guess, would not close the adversarial false-position case under reordering, and would make an address-selected hunk depend on unrelated preceding patch sections. The current policy—always alone, plus joins justified by plain Git or suggested by old positions—has a clearer invariant. The remaining false-position pair is better left explicit than “fixed” by a heuristic with a 35% measured cost and no sound placement basis.

## Verification performed

- Read the complete request, prior Vesper report, sixth independent review, changed implementation, tests, README/changelog claims, rank list policy and fuzz target.
- Verified HEAD, frozen-binary hashes and worktree state; reviewed `git diff 8f290e3..87d1afd` and `git diff --check`. The reported trailing spaces are in imported review/fixture data.
- Reproduced the named IR6 placement, fragment-start and joining cases with frozen round-13 and round-14 binaries.
- Compared rank, outline and address-selected hunk evidence on a joined created-file patch.
- Ran strict round-13 and published-v0.8.3 comparisons over 621 supplied patches.
- With the build slot free, archived exact commit `87d1afd` and ran `cargo test -p deep-diff-forge-graph -p deep-diff-forge-daemon --locked` in `fedora-toolbox-44`, keeping the launcher on the real HOME and setting the private HOME only inside the container command. It passed.
- Sampled time and peak RSS on the supplied `manyhunks-*`, `long-*` and `items60-frag` inputs.

## Not examined

I did not run the fuzz targets, the full workspace/gate, the complete 3,040-patch set, the 481-commit and 603-crate measurements, the 330,000-hunk measurements, or another Rust/Git version. I did not audit unrelated history before `8f290e3`, prove every `git apply` placement form, or independently validate every corpus truth label and false-positive count.

STATUS: COMPLETE
