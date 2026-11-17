# Vesper V3 round 15 review — `ceb2e6f`

## Verdict

**NO-GO.** The round closes the requested round-14 and independent-review defects, and the stated 499,990-hunk memory figure is accurate. One new medium finding stands: the new end-of-hunk heuristic reports `process_spawn` for an ordinary, valid `git diff -U0` whose changed line merely ends in a type named `Command`, even when the file contains no process spawn.

Reviewed exact commit `ceb2e6fc08fd4c8823cdff075363d19566dc2aab`. The frozen release binary was SHA-256 `44ba828ceeb05525717fb1976dd6bcdf31ee7c56fbc4b7db14b5f6b0b191b87b`.

## Finding

### VD3R15-M-1 — a bare `Command` at a hunk edge is falsely a process spawn

`waiting` treats a side whose last token is the bare word `Command` as `process_spawn` (`crates/deep-diff-forge-graph/src/rust_tokens.rs:861-865`). This is not enough of the released path `Command::new`: `Command` may be any user type and may occur at a valid line boundary for unrelated syntax.

I compiled this valid file:

```rust
struct Command;

pub fn command() -> Command
{
    Command
}
```

The patch was produced by Git, not handwritten:

```diff
@@ -3 +3 @@ struct Command;
-fn command() -> Command
+pub fn command() -> Command
```

On that patch, v3r14 reports score 7 with only `public_api_surface`. The frozen v3r15 binary reports score 11 with `public_api_surface` **and `process_spawn`**, placing `process_spawn` evidence on line 3 with basis `hunk`. There is no process-spawning operation anywhere in the file.

This invalidates the selection rationale that the built edge forms did not fire on real diffs. It is also materially different from a declared miss: it adds a weight-4 signal and can change ranking or policy behavior on an ordinary zero-context diff. Add this case to the measured corpus and either remove the bare-name edge form or narrow it to syntax that is sufficient evidence. The same rationale should be rechecked for the bare `TcpStream`, `UdpSocket`, `reqwest`, and `ureq` forms beside it.

## Requested closures

- **VD3R14-M-1: closed.** The claim now distinguishes the monotone full answer from the bounded list. The bounded-list tests exercise the release floor, and the one strict r14-to-r15 loss is the stated `bound-lang.patch` trade: a later-round fetch yields list space to the release's secret.
- **IR7-H-1: closed.** `FileKind::readings` keeps the release reading and adds every distinct language named by the first old/new-side line of every hunk. `shebang-lower.patch` remains score 9, and the out-of-order `ooo.patch` rises from 2 to 9. The source unites readings rather than allowing a new reading to withdraw one.
- **IR7-M-1: closed.** `html.patch` now reports the two panic-path items and the process spawn; the first four record-increasing raw-string hash counts are read.
- **IR7-M-2: closed.** Both `pq.patch` and `qp.patch` now find the cut panic path. Source inspection confirms old and new positional hints are each taken with and without the must-join relation, while the hunk-alone reading remains.
- **IR7-M-3: closed.** `sp1`, `sp3`, `sp4`, and `sp7` all recover their process/network signals. The token reader accepts whitespace, comments, and line ends between the released path segments without claiming aliases or globs.
- **IR7-H-2: acceptably stated for this branch.** The current two-section behavior is pinned and the dependency on the release-line parser refusal is explicit. This is enough while the branch cannot merge first; it is not permission to merge this branch without that rebase/dependency.

## Claims

1. **Held:** readings are united, so the full items/signals/score only add; the bounded list prioritizes items marked as v0.8.3's. The evidence tests include exact release-mark and bounded-list properties.
2. **Held on the available comparison set:** strict v0.8.3-to-v3r15 comparison over 5,699 available patches found no actual loss or lower score. Its sole printed loss was the disclosed two-section ranking reorder: v0.8.3 ranked the `new_file` entry first, while v3r15 ranked a new score-6 `process_spawn` entry first and retained the score-4 `new_file` entry second. Sixty-six paths from `all5.lst` had been removed from `/tmp`, so I could not independently repeat all 5,765.
3. **Held:** source inspection and the false-number/shebang/join reproductions found header numbers only establishing the old-position 0/1 top case or adding readings; no reading is withdrawn. VD3R15-M-1 is an edge-selection defect, not a header-number dependency.
4. **Held for the reviewed implementation and probes:** readings remain separate and united by findings, there is no clock/budget, and the join construction/readers are linear in the represented input. The fresh evidence suite's cost tests passed. Memory is discussed below.
5. **Held literally:** the nine eval-corpus integration tests passed, including manifest drift, floors, and declared-miss checks. VD3R15-M-1 is a new valid Git-produced case missing from that corpus, so the corpus should be extended before making the broader edge false-positive statement again.

## F — memory

The statement is accurate. On `/tmp/claude-ddf-lexfix-review-3/mem/created-open-499k.patch` (21,622,760 bytes), the frozen release binary completed `--rank` in 1.58 seconds with maximum RSS 263,060 KiB, matching the reported 264 MB.

I would **not require the streaming rewrite before this branch merges**. The case is deliberately extreme (499,990 hunks), the cost is linear and disclosed, and this round reduces the v3r14 measurement rather than regressing it. The roughly 12:1 RSS/input ratio is still worth tracking and bounding in deployment; it should not be represented as constant-memory processing.

## G — two-section dependency

Stating and pinning the dependency is sufficient for review of this branch, provided the merge ordering is enforced: the branch must be rebased onto the release-line refusal and its pin updated before merge. Without that prerequisite, the branch is not independently merge-ready.

## Verification performed

- Read `87d1afd..ceb2e6f`, including evidence/ranking, joins, language readings, Rust token edges, tests, fuzz-target changes, docs, and the imported independent review.
- Reproduced all named H/M patches: shebang lower/out-of-order, raw-string HTML, both positional joins, two-section edge, four spaced released paths, and honest/false cut pairs.
- Ran a fresh `git archive ceb2e6f` with a private target directory. `cargo test -p deep-diff-forge-graph --locked evidence::tests` passed: 100/100. `cargo test -p deep-diff-forge-cli --locked --test eval` passed: 9/9. The toolbox launcher retained the real HOME.
- Compared the 5,699 still-present entries of `all5.lst`: r14-to-r15 plain had 0 losses and 37 gains; strict had only the stated `bound-lang.patch` list trade. v0.8.3-to-v3r15 strict had only the inspected two-section rank reorder described above and 2,404 gains.
- Reproduced the 499,990-hunk memory case and created/compiled the VD3R15-M-1 source before obtaining its patch with `git diff -U0`.
- `git diff --check 87d1afd..ceb2e6f` only reported trailing spaces contained in the imported patch fixture; I did not treat fixture payload bytes as production whitespace defects.

## Not independently repeated

I did not rerun the full workspace gate, clippy, the four long fuzz campaigns, the 177,957-fragment/history/603-crate measurements, or the complete 5,765-patch comparison (66 listed scratch paths no longer existed). I did not re-prove Git's placement algorithm from Git source, and I did not test the explicitly declared non-Rust spaced-form misses, aliases/globs, editions before 2021, frontmatter, or Unicode-after-18 cases.

STATUS: COMPLETE
