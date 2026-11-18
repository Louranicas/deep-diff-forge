# Vesper review V4, round 17 — 2026-11-15

## Verdict

**NO-GO** on `f604d58` and on `agent/loop` as a whole (`d4dbc91..f604d58`).

VD4R16-M-1, VD4R16-L-1 and HR8-M-1 are closed on the tested host and workflows. The round-16 nine-tree case now retains the first tree's unchanged failure; changing it blocks as a whole. Mutable Git administrative-file times no longer change a tree's identity on this birth-time filesystem. Relative-path linked trees retain one identity from inside and outside.

One ordinary-workflow low stands. The new order still judges the project's main working tree before the linked working tree in which the stop was made. With both dirty, the main-tree patch is submitted first. A slow main-tree run can leave only about five seconds for the linked tree; a linked-tree verdict that needs longer is then UNJUDGED at every such stop. That breaks Claim 4 and the README/changelog statement that “the working tree the stop is in” comes first.

Reviewed exact commit `f604d58da58bbfb70b9dc4d5fa2925c900dd0356`. Frozen release SHA-256: `fbb014f6812224f3e335bab6e9e37819370eb3bab4808b47beb9ab3ee99efc70`.

## Prior findings

- **VD4R16-M-1: closed.** `within_bound` first replaces large units with one digest/count mark, preferring trees this stop did not look at. The mark covers every owned failing-line identity and count: an exact failure stands; any change makes the whole unit new. A looked-at unit is shortened only after all other reducible units. If more than 2,048 one-mark changes still do not fit, whole unlooked units are dropped oldest first and `memory_notes` says at that stop how many were lost and that an unchanged failure may block again. In the supplied real-judge reproduction, nine linked trees with 256 failing files each blocked once; the first and second stood unchanged when revisited, and adding one failure to the first blocked.
- **VD4R16-L-1: closed on this filesystem.** `instance_of` uses the administrative directory's inode plus birth time. Touching `commondir` and `gitdir` no longer changed the identity: the unchanged failure stood. The stated fallback to `commondir` inode/mtime where birth time is absent remains inherently mutable; it is documented and I could not test such a filesystem live.
- **HR8-M-1: closed.** Git pointer files are read as bounded bytes, CRLF is handled, non-UTF-8 path bytes are accepted, and relative paths are resolved from the directory containing the pointer, not the hook's current directory. More importantly, existence/forgetting follows identities of administrative directories under `worktrees`, not a successful path match. The supplied `--relative-paths` sequence blocked once in the linked tree, said from the project that a failure stood there, and returned to FAIL STANDS on each revisit.

## New finding

### VD4R17-L-1 — ordinary workflow: a stop in a linked worktree judges the main worktree first, so the stop's own change can still be starved

Claim 4 and the new README/changelog order say the working tree the stop is in is read and judged first. `judge_units` instead always constructs `trees` with `place.top` first, then appends `Elsewhere::Linked`:

```rust
let mut trees = vec![(
    place.top.clone(),
    tree_instance(place),
    uncommitted(stopping, &place.top, "work", &mut out.beside)?,
)];
// ...
Elsewhere::Linked(linked) => trees.push((
    linked.top.clone(),
    tree_instance(&linked),
    uncommitted(stopping, &linked.top, "work-2", &mut out.beside)?,
)),
// ...
for (top, id, found) in trees {
    tree_units(...);
}
```

When the session began in the main tree but its current directory is a linked worktree, `place.top` is the main tree and `linked.top` is the tree the stop is in. The loop therefore submits the wrong tree first.

I reproduced this in `/tmp/vesper-v4r17-own-order/`; scripts are `/tmp/vesper-v4r17-own-order.sh` and `/tmp/vesper-v4r17-slow-order.sh`. They use the frozen release binary, a private HOME exported inside each script, and the real forge-warden behind a logging/delay wrapper.

With one credential in `lib/main.rs` of the main tree and another in `lib/linked.rs` of the linked tree, a stop whose `cwd` was the linked tree logged:

```text
RUN warden.toml,
RUN lib/main.rs,
RUN lib/linked.rs,
```

The first line is the chain/probe preparation. The two actual changes are unambiguous: `main.rs` was submitted before `linked.rs`. Both real verdicts failed, so this first control blocked and named both.

For the timing consequence, the same wrapper delayed the main-tree patch by 30 seconds and the linked-tree patch by 6 seconds before invoking the real judge. The hook's bounded runner stopped the main run after its allocation, then reached the linked tree with only its smaller reserved share:

```text
elapsed=25
MAIN   1794691012.090548935
LINKED 1794691031.896167364

allow
deep-diff-forge: UNJUDGED. Not everything of this change ... was judged.
Uncommitted changes in ".../repo": UNJUDGED
  not judged: the judge was stopped before it gave a verdict.
Uncommitted changes in ".../linked": UNJUDGED
  not judged: the judge was stopped before it gave a verdict.
```

If the linked tree had been first, its six-second run fit the roughly twenty seconds allocated to the first of two working-tree patches. In the implemented order it receives roughly five seconds after the main run consumes its allocation, so it times out. This can recur at every stop while both changes and timings remain. The output is not silent and the stop stays within its deadline, which keeps this Low, as HR8-L-2 was; but the claimed priority and its practical guarantee are false in the common workflow of a session moving into a linked worktree while the main tree is dirty.

Recommended fix: order the at-most-two judged trees by the current/session directory. When `Elsewhere::Linked` is the tree containing `cwd`, put it before `place.top` (and make the focused ordering test use a stop from the linked tree with both trees dirty). The main tree remains a judged unit, just second.

## Claims

1. **Round 14's silence rule: held in the inspected and reproduced cases.** Relative-tree and nine-tree controls named standing or unjudged work at outside stops. Memory shortening/loss produces an explicit note in the same stop. The slow-order reproduction was UNJUDGED, not silent. I did not rerun the complete historical outcome table or randomized session driver.
2. **Every commit without a verdict is asked in bounded turns: held by source and focused tests; timing edges not live-reproduced.** `OWED_ASKS_NEED` is eight seconds. With eight seconds, `run_time` gives the first reserved ask three and leaves five for the second; below eight but at least three, the first gets all remaining time and the second runs only if three seconds remain; below three, `judge_commits` records neither ask. `Judge::ran` prevents a no-time/non-start from being counted, and `not_asked` says it kept its turn. A commit whose patch/ranker cannot be offered despite time is deliberately rotated by updating its last stop without incrementing attempts; because it moves behind the other least-recently-asked commits, I found no starvation from that choice.
3. **One block per failing-line identity while memory is kept: held, subject to the stated identity and disclosed loss.** The nine-tree case stands unchanged, an added line blocks, marks include every line key and count, and a mark mismatch makes the whole failure new. A SHA-256-prefix collision could make a changed mark stand but requires an infeasible deliberate digest collision (84 bits for aggregate marks; 96 bits for individual line keys), not an ordinary workflow. Replacement content under the same rule/signal/path/count still stands by design; README, SECURITY.md and FAIL STANDS now say that the hook does not know whether it is the same file line. The staged/mark ambiguity becomes UNJUDGED rather than guessing. I did not find an unchanged repeat without an earlier memory-loss sentence.
4. **The stop's own change first and one deadline: broken by VD4R17-L-1.** The single deadline and the separate six-second look phase held; my slow reproduction ended in about 25 seconds and product code bounds every judge/status run. Submodules and unjudged other trees now come last and are counted. But when the stop is in a linked tree, the main tree is still the first judged tree and can starve the actual current tree.
5. **One writer and bounded messages: held in source and the exercised outputs.** This commit does not weaken the session lock. Memory notes reserve their room before other notes; note counting preserves the number of unnamed standing trees; paths, rule fields and judge words use escaped/data-marked renderers. The largest exercised nine-tree message was 5,610 characters and retained its counting/standing line. I did not rerun the 70-commit near-cap case or inject every control character.

## Requested attack areas

### A. Memory and changed failures

No break found beyond the explicit identity/mark limits. A looked-at change is the last candidate for shortening and is named if shortened. An unlooked change becomes a mark and remains exact for unchanged failures. Whole-unit loss happens only after the mark count itself exceeds the cap and is announced at that stop. The staged corner refuses to attribute new staged lines when an unjudged uncommitted failure is known only by a nonmatching mark.

### B. Tree identity and path forms

The supplied relative-path and mutable-time cases held. Code reads path bytes without requiring UTF-8, strips one CR before interpreting a pointer line, resolves from the pointer's directory and normalizes/canonicalizes both forms. A trailing space remains a path byte rather than being silently trimmed, matching the conservative behavior for a malformed pointer. Mixed relative/absolute repairs held in the supplied test history. I did not have a filesystem without birth time, force inode reuse, restore an administrative directory through a filesystem snapshot that preserves inode/birth time, or create a live non-UTF-8 worktree name during this run.

### C. Commit-turn timing edges

Held by direct source/test review at 7.9, 3.1 and 2.9 seconds: at 7.9 the first reserved ask runs and the second depends on three seconds remaining; at 3.1 the first runs with all 3.1 seconds; at 2.9 neither starts or changes state. The stated “turn written with no attempt” case is for an ask that cannot be run despite available time, not one missed for time, and moves it behind peers rather than pinning it first.

### D. Deadline and slow trees

The six-second look phase uses a per-tree half/even-share deadline, prioritizes trees where failures stand, and counts both late and beyond-sixteen trees with their standing totals. No one slow looked-at tree can consume the whole stop or erase the count for later ones. The judged-tree ordering defect is VD4R17-L-1.

### E. Messages and hostile text

I found the shortening/loss sentences at their transition points and not on ordinary below-cap stops. Standing counts survive the condensed note line. Paths are quoted and control-escaped; rule names/explanations, signals and judge words go through `display_safe`; judge words are explicitly labelled data, not instructions. No new injection route found.

### F. Anything else / branch-wide verdict

No second new finding. The single standing low makes the branch-wide verdict NO-GO until the linked-current-tree ordering is corrected and covered. The earlier hook findings reviewed in rounds 14–16 otherwise remain closed in this commit.

## Verification performed

- Read the complete request, round-16 report, eighth independent review, exact source delta, changed tests, README, SECURITY.md and changelog.
- Verified exact HEAD, clean worktree before the report, frozen debug/release hashes, and a clean `git diff --check 8746cd7..f604d58`.
- Ran the supplied round-17 real-judge reproduction on the frozen release binary: relative-path identity, mutable administrative times, and nine linked trees with 256 failing files each all held.
- Built two independent linked-current-tree reproductions with the frozen release binary and real judge behind a wrapper: one established actual patch submission order; the other established repeatable starvation within the bounded deadline.
- Inspected the 100 focused hook tests, including memory properties, time allocation, relative paths, standing counts and main-tree-before-other-tree coverage.

I checked the hive slot before attempting Cargo and it reported `free`. I then attempted to extract an exact `git archive f604d58` under `/tmp`, but the active mutation workload had exhausted the tmpfs inode limit (`df -i /tmp`: 1,048,576/1,048,576; `tar`: `No space left on device`). I did not disturb or kill any mutation process or remove its files. The failed partial archive was only this review's scratch; the environment did not support trashing directories on the tmpfs. I therefore did not run Cargo.

## Not examined

I did not run the focused Cargo test executable, the full 2,154-test workspace suite, CLI unit suite, local gate, mutation run, or the 90-session/5,000-stop property independently. I did not test a filesystem without birth time, inode reuse, snapshot restoration of administrative metadata, NFS/FUSE/no-`flock`, a live non-UTF-8 worktree name, full disk/state-write failure, stale locks, every Git version/configuration, submodule worktrees, all 2,048-change message transitions as integration runs, the 70-commit message cap, every 7.9/3.1/2.9-second live edge, SDK/client timeout behavior, setup/doctor/schema commands, release packaging, or unrelated branch code.

STATUS: COMPLETE
