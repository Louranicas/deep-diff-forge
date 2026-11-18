# Vesper review: V4 round 7 (2026-11-14)

## Verdict

**NO-GO.** VD4R6-M-1 is closed: the selected set now has constant-size storage, and the 500,000-entry reproduction stays near baseline memory. I found one new low-severity availability defect: a non-UTF-8 regular name can consume one of the seven fallback slots even though the helper cannot transmit that path to the parent, suppressing a later usable ranker.

## Round 6 disposition

### VD4R6-M-1 — Closed

I recreated the 500,000-empty-file directory from round 6 and reran both paths with the supplied debug binary:

| Path | Result | Elapsed | Maximum RSS |
|---|---:|---:|---:|
| `agent hook seal-rankers` | 8 candidates | 1.98 s | 9,076 KiB |
| complete Stop hook | normal explicit UNJUDGED | 2.17 s | 9,864 KiB |

Round 6 measured 69,640 KiB in the helper and 69,644 KiB through Stop for the same entry count. The new heap holds at most seven directory `PathBuf`s, and the measured RSS no longer scales with the directory collection.

By inspection, the heap returns the same bytewise path order as sort-then-truncate for regular files. A descending input can still make `wanted`/`is_file` run for every entry because every new name changes the kept set, but storage remains constant and enumeration remains inside the eight-second supervised child. That is consistent with the documented time bound and is not a recurrence of the finding.

## New findings

### VD4R7-L-1 — A non-UTF-8 name consumes a fallback slot and is then discarded

`first_by_name` treats every regular path as eligible. Later, `seal_rankers` converts the selected path with `to_str`; a non-UTF-8 path becomes JSON `null`. `sealed_rankers_by` discards any entry whose `path` is not a string. Selection is not backfilled after that discard.

I reproduced this with one regular file named `a<0xff>` followed by seven valid UTF-8 executable files `b1` through `b7`:

- the helper returned this binary, `path: null`, and only `b1` through `b6`;
- the parent discarded the null path;
- a logging exit-4 judge was called seven times: this binary and `b1` through `b6`;
- `b7` was never offered, although it would be the seventh offerable fallback after skipping the path the IPC cannot represent.

With seven early non-UTF-8 files, the parent called the judge only for this binary; a later valid `z-valid-ranker` was absent. If the policy rejects this binary but admits the omitted later fallback, the stop becomes UNJUDGED instead of receiving that fallback's verdict.

Impact: this is fail-visible and requires a configured ranker directory, so I rate it low. It nevertheless defeats the fallback purpose for a directory containing an unrepresentable early name, and Git repositories can contain such Unix path bytes.

Recommended fix: make representability part of `wanted`, before a path enters the bounded heap—for example, require `path.to_str().is_some() && path.is_file()`. Then the heap selects the first seven regular paths the helper can actually serialize and the parent can actually offer. Alternatively, define a lossless path encoding for the helper IPC.

## Other checks performed

- Read `08332e0..7defe1f`; `git diff --check 08332e0..7defe1f` passed.
- Exercised quote/newline names, links, directories, more than eight entries, case-sensitive byte ordering, and the non-UTF-8 crowd-out described above.
- Checked link/race behavior by inspection: a selected link is hashed by the helper but forge-warden refuses its final symlink, and a changed regular path yields a pinned-digest mismatch unless the bytes are again identical.
- Checked the hive slot immediately before each Cargo invocation; it was free.
- In `fedora-toolbox-44`, `the_first_files_by_name_are_chosen_without_holding_the_directory` and `the_seal_helper_lists_each_ranker_with_its_digest_or_its_refusal` passed.

## Not looked at / limitations

- I did not instrument the exact `stat` count of a real directory returned in worst-case descending order; the worst-case call count follows directly from the heap branch, while the memory property does not depend on it.
- I did not run a scheduler-controlled entry-vanish race. A selected entry that vanishes is reported as refused and still occupies its slot, matching the prior first-seven-candidate behavior.
- I did not rerun the blocking-read interposer, full real-forge-warden matrix, or full workspace test suite; those paths were unchanged by `7defe1f`.
- I did not review changes outside `08332e0..7defe1f`.

The 500,000-file and non-UTF-8 scratch directories were removed after measurement; they contained only files created for this review.

STATUS: COMPLETE
