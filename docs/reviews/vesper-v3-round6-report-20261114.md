# Vesper V3 round 6 review (2026-11-14)

Reviewed: committed snapshot `a530cb3..ccc5e9c` on `agent/integrity-signals`. The pre-existing uncommitted worktree changes were excluded; Cargo tests used a `git archive` of `ccc5e9c` under `/tmp`.

## Verdict

**GO.** Both round-five findings are closed. I found no new High, Medium, or Low finding in the committed delta.

## Round-five findings

- **VD3R5-H-1: closed.** `FragmentPlan::of` counts paths across the complete review before assigning `Reading::WholeFile`; a path named by two parsed sections receives only `Reading::Hunk`. Only `WholeFile` reaches the parser's withdrawal path, and `fragment_evidence` retains the earlier line-1/all-added/error-free conditions. The original `/tmp/vesper-v3r5-duplicate-path.patch` now ranks the created section with `panic_path_added` (score 7), while the later section remains separate. Rank, outline, and cluster modes agree.
- **VD3R5-M-1: closed.** `opens_across_lines` now returns the opener's byte column, and the fallback is suppressed only by a non-method construct at that same row and column. The original `/tmp/vesper-v3r5-same-row-macro.patch` now reports `syntax_unchecked` and scores 4. The byte-coordinate comparison remains aligned through tabs, multibyte text, and the code view because the lexical view blanks in place one byte at a time and tree-sitter also reports byte columns.

## Adversarial checks

### Whole-review path identity

- Re-ran the exact duplicate-path reproduction through `--rank`, `--outline`, `--cluster --parallel 1`, and `--cluster --parallel 4`; all retained `panic_path_added` for the created section.
- Read the parser's path resolution: quoted names are decoded, repeated slashes are squashed, default `-p1` stripping is applied, and rename/copy destinations become `ReviewFile::path`, so the ordinary spelling variants converge before `FragmentPlan` counts them.
- Tried to evade the count by creating `src/duplicate.rs` in one section and then modifying it while renaming it to `src/moved.rs` in another (`/tmp/vesper-v3r6-rename-source.patch`). DDF did give the sections different paths, but Git refused both `--check` and application because the rename source does not exist in the preimage (`src/duplicate.rs: No such file or directory`). A later ordinary modification of the created file is the applicable Git form, and that resolves to the same counted path.
- A later deletion and creation at the same path, or a mode/binary section for it, also shares the resolved path. A rename or copy whose destination is the created path shares that destination; one whose source is the newly created path has the same preimage failure as the reproduced attack.

### Entry points and planning

- Production rank paths call `deep_diff_forge_graph::rank`, including CLI rank, daemon handlers, MCP/SDK ranking, pipeline, and TUI callers.
- Daemon outline and hunk views build one plan from all `review.files()` before pagination or selection.
- `run_risk_cluster` builds one plan before launching lanes and passes the indexed reading to each worker. The exact cluster test and live `--parallel 1`/`--parallel 4` probes agreed.
- The public single-file convenience functions still plan their argument as a one-file review, as declared. I found no workspace user-facing path that feeds one section of a larger review through those conveniences; current non-test uses of the individual APIs are fuzz targets. The public SDK exposes whole-review `rank`.
- The plan counts all file paths before spending the parser budget, so a duplicate section beyond that budget still prevents `WholeFile` for the earlier section.

### Opener positions

- Tabs and multibyte characters before an opener preserve byte offsets in `code_view`; parser columns are byte columns too.
- Multiple same-row openers are compared independently by their returned columns. The regression also covers an unrelated outer macro and a preceding macro on the same row.
- Qualified paths are measured from their path start (`std::assert!`), matching the macro-invocation node. Spellings for which the lightweight path scan and parser disagree, such as raw-identifier-like prefixes or whitespace around `::`, err toward `syntax_unchecked` rather than suppressing it.
- The previously declared exclusions (other delimiters and split one-line lexical forms) remain exclusions; I did not find an undeclared silent suppression in the new column comparison.

### Fuzz target

- `fragment_scan` builds successfully at the reviewed commit.
- Its two shapes exercise the parser-on/parser-off monotonicity rule for a single new file and a mixed change, determinism, bounds, and basis/side invariants. It deliberately does not model a second section, so it is not the regression for VD3R5-H-1; the dedicated graph unit test is.
- The target's allowance for withdrawing `panic_path_added` in its new-file shape is intentionally broader than proof of semantic test-only code. It is useful as a “parser removes no other signal” invariant, not as a complete oracle for whether each panic withdrawal was justified.

## Tests

The hive slot was free before Cargo. Cargo ran inside `fedora-toolbox-44` against an archived `ccc5e9c` source tree, not the dirty worktree, using the supplied target directory.

- `deep-diff-forge-graph`: 133 unit + 6 integration tests passed.
- `deep-diff-forge-cluster`: 43 unit + 10 integration tests passed.
- `deep-diff-forge-daemon`: 225 passed, 1 ignored.
- Targeted round-five and worker-independence regressions passed.
- `cargo check --manifest-path fuzz/Cargo.toml --bin fragment_scan` passed.
- `git diff --check a530cb3..ccc5e9c` passed.

## New findings

None.

## Not examined / limitations

- I did not run the 150-second fuzz campaign or independently reproduce its historical control crash; I inspected and compiled the target and relied on the request's run counts.
- I did not test case-folding or Unicode-normalizing filesystems. This Linux filesystem is case-sensitive; those platform equivalences cannot be honestly reproduced here.
- I did not exhaust every hostile patch-header combination already covered by the patch parser's wider suite, nor rerun the large resource fixtures from round five.
- I did not review or test the worktree's uncommitted modifications. I did not modify repository refs, index, configuration, remotes, or existing files; only this report was added.

STATUS: COMPLETE
