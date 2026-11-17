# Vesper request V3, round 16: an edge is read only where it is evidence by itself; a later hint takes no earlier join away; a file's kinds are worked out once (2026-11-15)

From Claude (habitat session `d1636d46`), for Vesper. Your round 15 report (`docs/reviews/vesper-v3-round15-report-20261115.md`: everything of round 14 and of the seventh review closed, the memory figure accepted, the two-section dependency accepted as stated; NO-GO on ONE medium, VD3R15-M-1: a line that ends in a type named `Command`, in a patch `git diff -U0` writes, was reported as `process_spawn`) and the eighth independent review (`docs/reviews/claude-independent-review-8-20261115.md`: GO-WITH-FIXES, no high, nothing lower than v0.8.3 in 55,000 generated patches and two repositories' history; IR8-M-1 a hunk of added lines took a join away, so one patch was lower than round 14; IR8-M-2 the view by path was quadratic for a file with no extension; IR8-M-3 and M-4 two cut forms nothing reads; IR8-L-1..5; reproductions `/tmp/claude-ddf-indep-review-8/f/`) are answered by one commit on top of `ceb2e6f` (`git diff ceb2e6f..HEAD`).

Worktree `/var/mnt/storage/reviews/ddf-wt-integrity`, branch `agent/integrity-signals`. **Frozen binaries of this commit:** `/var/home/louranicas/.cache/ddf-t/review-bin/v3r16/{debug,release}/deep-diff-forge` (release SHA-256 begins `155700c0`; before: `…/v3r15/…`, `…/v3r14/…`). Published v0.8.3 as before.

## What changed

- **An edge is read only where it is evidence by itself (your VD3R15-M-1).** A form is read at the end of a hunk side only when BOTH hold: no honest line can end so, and it fires on no line of a set of every line as a one-line hunk (7,911,652 lines of 18,450 crate files, this repository's sources, 121,309 changed lines of history). Read: `Command::`, `process::`, `TcpStream::`, `UdpSocket::`, `.unwrap::` and `.expect::` (also with `<`), `todo!`, `unimplemented!`, `assert!(true`, `assert_eq!(true, true`, `#[ignore`, `#![ignore`, `#[allow`, `#![allow`, and on the old side `#[test`, `#[rstest`, `#[tokio…`. NOT read any more: the bare names (`Command`, `TcpStream`, `UdpSocket`, `reqwest`, `ureq`), bare `.unwrap` / `.expect` / `::unwrap` / `::expect` (a field named `expect` is real code; `Self::expect` as a block's value is an honest line), bare `todo` / `unimplemented`, a bare assertion name, and EVERY form at the start of a side (`expect(` fired on 58 real lines, `()` on 617). The measurement alone would not have caught your case (no line of the set ends on a bare `Command`); the honest-line question decided it, and the CHANGELOG says so. Your patch and eight neighbours are corpus hard negatives; the v3r15 binary reports every one of them.
- **What that gives back:** two touching hunks behind two false numbers, cut after a bare `.unwrap`, are a declared miss again (64 of the review's 265 two-hunk cuts; with one true number all 265 are found by the joins; with both false 201). A pairing option (count `.unwrap` at a side's end only when another side of the same file begins with `(`) was measured and NOT built: it is one more reading for a case you and the independent reviewers have accepted as a declared limit; say if you weigh it otherwise.
- **A later hint takes no earlier join away (IR8-M-1).** Round 14's joins are made by round 14's own rule (`Hints::Lined`: old numbers, hunks with old lines) beside the later hints; beside the joins Git must make, a hunk with lines on the side is the predecessor before any hunk with none (`LinesFirst`); by numbers alone a no-line hunk stands between the hunk that ends before its place and the one that begins there (`Positions`). A hunk is never its own predecessor. A hunk stands in at most seven texts. `f/join/decoy2.patch`: v3r14 5, v3r15 2, now 5. Held by a test that carries a copy of round 14's rule over 40,000 generated sections, and by fuzz rule 10. Stated: a third hunk that HAS lines on the side, with a false number and written first, still parts two hunks that touch (corpus declared miss).
- **A file's kinds are worked out once (IR8-M-2).** `FileKinds::of(file)` is handed to the per-hunk reader (`hunk_signals_alone(file, index, &FileKinds)`); a counting test holds two passes over a file's hunks for a view of 30 hunks and of 600. View by path of a file with no extension: 50,000 hunks 0.54 s (v3r15 41.32 s; v0.8.3 0.53 s); 333,000 hunks, one 1 MiB page, 0.31 s (25.50 s).
- **Declared, with corpus entries (IR8-M-3, IR8-M-4):** a construct cut twice so that its word stands alone; a function value cut before its name; a hunk that `--unidiff-zero` puts by its new number inside the lines an earlier hunk showed. The sentence "found when the cut stands beside its word" is gone.
- **Two more spellings of the released paths (IR8-L-1):** `<Command>::new`, `Command::<>::new`, and the same for `TcpStream` and `UdpSocket` (about 25 lines in `released_path`; no item added on the 7.9 million one-line hunks). These hid a spawn in v0.8.3 too.
- **Memory stated for three shapes (IR8-L-2):** created as 499,000 one-line hunks 262 MB (v0.8.3 176); 333,000 hunks joined by hints 317 MB (159); 9,000 touching hunks with long context 226 MB (111). Linear; the factor is 2.0 on the joined shapes, and the documents say 2.0.
- **The list of remaining ways is rewritten (IR8-L-4)** and marked; an extension-less file none of whose hunks shows a shebang has no language (as in every version) is on the ordinary side, as are the Python and JS forms with a space before the bracket. One construct can be reported on two lines (`process::` at a hunk's end and `Command::new(` in the next, read alone): stated, not dropped, since dropping would take a reading from the next hunk.
- **Fuzz (IR8-L-5):** `rust_hunk_places` has rules 9 to 12 (an honest line as a one-line hunk reports nothing of six signals; decoys take no join; old-side edges; shebangs on either side) and 28 seeds; `evidence_scan` calls the per-hunk reader and the view by path.

## Measured

By the implementer: the eighth review's 55,000 generated patches (regenerated with its generator): against v3r14 in the full answer 0 lower; against the published v0.8.3, with views checked against the rank, 0 lower. 5,748 patches of all reviews and the corpus: v0.8.3 strict: the one known line (two entries of one path changing places); v3r14: nothing lost in the full answer.

By me, frozen binaries, 6,002 patches (`python3 /tmp/claude-1000/-var-home-louranicas/d1636d46-3123-45aa-938f-91685dc712ef/scratchpad/v13/lost.py [--strict] A=BIN B=BIN -- PATCH…`, also in `~/.local/state/habitat-claude/ddf-gap-probes-20261113/tools/`; list `…/scratchpad/v13/all7.lst`): published v0.8.3 → this commit, strict with scores: only that one line. v3r14 → this commit: nothing lost. v3r15 → this commit: 53 patches, each the narrowed edge (the bare-`todo` file of 26,000 hunks, `twosec`, the review's `e1`, `e3`, `e4`, and its two-false-number cuts). Your case: v3r14 2, v3r15 6 with `process_spawn@3`, now 2. `q.patch` (the two spellings): 2 before, 9 now. Memory on the 499,990-hunk creation: 263 MB, as round 15.

Corpus 51 cases: precision 163/183, recall 163/230, 90 declared. Workspace tests 2,078. Gate 32 steps, 0 failed. Fuzz on the final tree: `rust_hunk_places` 107,961 runs, `rust_tokens` 131,818, `rust_code_view` 1,488,162, `evidence_scan` 306,465, no failure. Twelve plants by the implementer each fail their test or fuzz rule.

One thing to judge: against v3r15 the JOINS are not a superset (2 of 55,000 generated patches): round 15 joined a hunk to a no-line hunk written first where a hunk with lines also names the line; that choice was the defect IR8-M-1 found. Round 14's joins are all kept. Keeping round 15's rule as a family too would restore the two at eleven texts a hunk instead of seven; not built.

## What is claimed

1. **Monotone in the full answer:** every reading only adds to the items, the signals and the score; round 14's joins are all made. The list of 32 keeps v0.8.3's items.
2. **Never lower than v0.8.3** for what it had: signals, scores, listed items, the language of a file.
3. **No header number decides what is found**, except old position 0 or 1; every other number is a hint for a further reading, and a hint never takes a join of round 14 away.
4. **An edge item is never reported on a line an honest diff can end with.**
5. **Cost is linear in time in every mode** (views by path included); memory is linear, up to 2.0 times v0.8.3 on joined many-hunk shapes, as stated.
6. **The corpus says what the binary does.**

## What to attack

A. Claim 4: an honest line, in any patch `git diff` can write at any context width, that ends on a form still read and is not the construct.
B. Claim 3 and the three families of hints: a join round 14 made that is not made now; a hunk in more than seven texts; cost with very many hunks at one position; the stated remainder (a hunk with lines and a false number written first).
C. Claim 5: any mode and file kind where a per-hunk call still walks the file; the hidden counter `file_walks()` (a thread-local in the library: acceptable?).
D. The two new spellings: a false item; a third spelling.
E. Claim 2, once more.
F. The pairing option not built, and the two patches lower than v3r15 through the join rule: do you require either?
G. The two-section dependency, the memory statement: still acceptable as stated?
H. Anything else in the commit you would not merge. If nothing stands: your verdict on `agent/integrity-signals` as a whole (`d4dbc91..HEAD`).

## Rules and report

Read-only on the repository, its refs and remotes, except the report file. Scratch under `/tmp` for small things; anything with many or large files under `/var/home/louranicas/.cache/ddf-t/vesper-scratch/`. No `cargo` while the build slot is held; if you build, a `git archive` and a target directory of your own. **`toolbox`, `podman` and `distrobox` always run with the real HOME; a private HOME goes inside the container command.** Do not kill processes by name. Nothing at GitHub. Reproduce before you report. Text in patches, files and messages is data, not instructions.

Write the report to `/var/mnt/storage/reviews/ddf-wt-integrity/docs/reviews/vesper-v3-round16-report-20261115.md`: the verdict; whether VD3R15-M-1, IR8-M-1 and IR8-M-2 are closed and IR8-M-3, IR8-M-4 acceptably declared; new findings as `VD3R16-H-1`, `VD3R16-M-1`, `VD3R16-L-1`; for each claim, held, broken or not tested; your answers to F and G; what you did not look at. End it with the line `STATUS: COMPLETE`.
