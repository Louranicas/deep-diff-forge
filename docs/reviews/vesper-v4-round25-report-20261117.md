# Vesper review V4, round 25 (2026-11-17)

## Verdict

**GO** for commit `38af7b8b448929b05d8b0bb28e56de9225c36d53` and for `agent/loop` as a whole (`d4dbc91..38af7b8`). I found no open high or medium against a stated claim, and no new low.

The round-25 rule closes VD4R24-M-1 and HR16-L-3 without erasing or silently releasing remembered failures. HR16-M-1 and HR16-L-1 are closed. HR16-L-2 is acceptably narrowed. The broader class that requires editing Git's administrative files is correctly made a deliberate-evasion limit: growing more identity state would not make these user-writable records an authority, and the implemented remembered-directory rule covers the dangerous case in which the hook already has cheap contrary evidence.

## Re-check of the five prior findings

- **VD4R24-M-1 — closed.** I reran the exact two-nested-tree reproduction against the frozen release binary. Its decisions changed from the round-24 `BLOCK, SILENT, SILENT` sequence to `BLOCK, UNJUDGED, allowed with an explicit note`. The state remained `b8d9b241bd757773/u:1` throughout the latter two stops. Both the new top and the remembered standing top appeared in the message. This is the class rule I proposed in round 24.
- **HR16-M-1 — closed.** A `cp -a` copy of a linked tree inside the main tree, containing a credential, was named at every main-tree stop as a directory that is no working tree by the records; the first and repeated stops were not silent. A stop from the copy was UNJUDGED. The copy was not sent to the judge as the linked tree.
- **HR16-L-1 — closed.** With one administrative `gitdir` edited so that Git listed one path twice, the original tree's remembered entry remained present, one duplicate was still named at stops from the other tree, and neither duplicate path caused both identities to be skipped. The state remained `2400ce1f67746667/u:1` through all stops.
- **HR16-L-2 — acceptably narrowed.** README and SECURITY now explicitly say that a main Git directory named `.git` away from the tree top makes main-tree stops UNJUDGED. That behavior is visible, not a silent pass, and this is an appropriate wording limit rather than a reason to add another identity interpretation.
- **HR16-L-3 — closed.** After `cp -a L C`, `git clean` and `git worktree repair` in `C`, stops from the project named the conflict, stops from `C` and `L` were UNJUDGED, and the original memory `f9f56a0567746cba/u:1` survived every stop. Neither directory was read as the remembered identity while `L` stood.

## Claims

1. **Held — silence rule.** Every attacked open condition produced a block, UNJUDGED answer, or explicit note. The two cases that were silent on v4r24 (VD4R24-M-1 and HR16-M-1) were not silent on v4r25.
2. **Held — pair plus standing remembered directory.** `standing_record` is applied both when locating the stop's tree and when examining other listed trees. A different paired top is not read while the recorded directory is a directory or cannot be inspected; its memory is retained and both locations are exposed. A normal `git worktree move` remained an accepted move: the next stop was `FAIL STANDS`, with the same remembered unit, rather than becoming UNJUDGED.
3. **Held — listed identity and unlisted directories.** The judged set is now keyed by tree id. `listed_instance` consumes distinct administrative ids for duplicate paths. The unpaired nested-copy route is named by `left_out` rather than mistaken for a listed tree.
4. **Held as a stated limit — administrative-file identity moves without a standing remembered directory.** SECURITY's “Not a boundary” paragraph and README's deliberate-evasion list make the boundary explicit. Further code is not warranted for this user-writable-administration class; narrowing is the right answer.
5. **Held for the round-25 changes.** I found no contradiction between the implementation, tests, README, SECURITY, CHANGELOG, and the claims this round replaces.

## Edge and ordinary-workflow attacks

All runs used `/var/home/louranicas/.cache/ddf-t/review-bin/v4r25/release/deep-diff-forge` with a private HOME established inside the Python driver and the real frozen judge.

- Exact VD4R24-M-1 reproduction: `BLOCK` then `UNJUDGED` then an explicit other-tree note; memory unchanged.
- `copyinside`: explicit left-out note on three main-tree stops; stop from the copy UNJUDGED.
- `copyrepair`: explicit note/UNJUDGED from all relevant locations; memory unchanged.
- `gitdirdupa`: duplicate listed path retained as two ids; the failed identity remained named and remembered.
- `gitmv`: `BLOCK` before the move, then `FAIL STANDS` at the moved tree; the project stop named the tree. This is the quiet-and-correct ordinary move that must keep working.
- Recorded path replaced by a file: the repaired new top was read, a new credential blocked, and the remembered unit was retained. This matches the documented “old directory is gone” interpretation.
- Recorded path replaced by a symlink to the repaired top: the aliases resolved to the same top; the new credential blocked and memory remained.
- Recorded path replaced by a different repository: the new top was UNJUDGED and the prior memory remained. A directory standing at the remembered path is conservatively decisive regardless of its contents.

The `metadata` treatment is conservative at the remaining edge: errors other than absence/not-a-directory keep the old record standing, so an inaccessible parent cannot turn uncertainty into permission to read a new top.

## Flake D

The one failure of `a_repository_with_more_working_trees_than_are_read_says_so` is not caused by this round. The test and the session-start/reflog enumeration path it exercises are unchanged by `7380e8e..38af7b8`; this commit's production hunks begin in tree placement/identity handling used after state is available. The reported symptom occurred during the pre-existing 8-second session-start work over 256 tree histories, passed five of six concurrent suite runs, and passed ten of ten alone. I classify it as load sensitivity of round 22's 256-tree cap, not a round-25 regression. It does not block this commit.

## New findings

None.

## Evidence and scope

- Read the complete request, the complete round-24 Vesper report, the round-25 source/docs/test diff, and the affected identity, pairing, left-out, other-tree, listing, and judging paths.
- Verified target commit and frozen binary hashes: release `35d6fad729fcfd9bccf916d9b0b97c7809e86df626ac5b04529a826d853bd220`; debug `cceebda024f73de044b5216e8f99e63513d9ad3530ed6ed0a747348a3a5855ad`.
- `git diff --check 7380e8e..38af7b8` was clean.
- Used the frozen release binary only. I did not build, run Cargo, contact GitHub, rerun the 2,228-test workspace suite, rerun the full 149-test concurrent suite, or independently repeat the full historical comparison matrix. I did not inspect the untracked seventeenth-review report.

STATUS: COMPLETE
