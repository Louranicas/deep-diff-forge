# Feature Deployability Matrix

Every baseline and pioneer feature must have a command path, model path, test path, and release gate.

This is a target matrix: a surface marked *(not implemented)* was checked against the binary on
2026-10-27 (anchor E-02) and is refused today. `deep-diff-forge --help` lists what exists.

## Status 2026-10-28 (0.3.0)

Rechecked against the 0.3.0 debug binary. Every surface marked *(not implemented)* below exits 2
(unknown command or unknown option). Corrections in this pass: the review cockpit reads its patch
from stdin and has no Git flags of its own; `--syntax`, `--semantic` and the pioneer commands
`rank`, `plan` and `--git` do not exist; the "Today" column names what does. `split` is accepted
as an alias of `side-by-side` (`layout_from_str` in `crates/deep-diff-forge-projection/src/lib.rs`).

## Baseline Features

| Feature | CLI/Bash surface (target) | Today | Model/API surface | Test gate | Deployable when |
| --- | --- | --- | --- | --- | --- |
| Review-first interactive UI | `deep-diff-forge review` (`--git` *(not implemented)*) | `git diff \| deep-diff-forge review [--side] [--palette \| --cmd NAME]`; headless `review --probe [--cols N] [--rows N]`; `just review [ref]` | `ReviewDocument`, projections | TUI unit tests + `--probe` frame | Keyboard and mouse review works without daemon. |
| Multi-file review stream + sidebar | `review`, `--json` | `review` sidebar; `--stdin-patch --json` or `--git-working-tree --json` (and `--git-staged`, `--git-range A..B`) | file list, status, ranked stream | multi-file fixture | Changed files can be navigated and exported. |
| Inline agent / AI annotations | `annotation add` *(not implemented)*, JSONL API | no CLI command; `deep-diff-forge-agent` crate API | `AgentAnnotation` | grounded/ungrounded fixtures | Annotations anchor to file/hunk/span and never mutate patch truth. |
| Responsive auto split/stack layout | `--layout auto\|stack` *(not implemented)* | `--stdin-patch --layout inline\|side-by-side` (`split` = `side-by-side`); `review --side` | projection config | terminal width snapshots | Same diff renders correctly at narrow and wide widths. |
| Mouse support inside viewer | `review --mouse` *(not implemented)* | none | TUI event model | terminal integration smoke | Scroll/select/collapse work in supported terminals. |
| Runtime view toggles | `--toggle <name>=<value>` *(not implemented)* | TUI keys only | session toggles | projection toggle tests | Toggles rerender without recomputing patch truth. |
| Syntax highlighting | `--syntax on\|off` *(not implemented)* | `highlight <path> [--color\|--no-color]` | token spans | language fixtures | Supported files highlight and unsupported files degrade. |
| Structural diffing | `--semantic on\|off` *(not implemented)* | `semantic <path> [--json]`, `structural <old> <new> [--json]` | `SemanticTwin` | syntax fixtures | Semantic spans synchronize to patch hunks. |
| Pager-compatible mode | default pipe/TTY behavior | `git diff \| deep-diff-forge --stdin-patch --layout inline` (without a flag, `--stdin-patch` prints a per-file summary); no Git external-diff integration | inline projection | Git external diff tests | Works with Git diff/difftool and plain shell pipes. |

## Pioneer Features

| Feature | CLI/Bash surface (target) | Today | Model/API surface | Test gate | Deployable when |
| --- | --- | --- | --- | --- | --- |
| Semantic Patch Twin | `deep-diff-forge --git --semantic --json` *(not implemented)* | `--stdin-patch --json` (patch twin, `deep-diff-forge.review.v0`); `semantic <path>`; `analyze <patch> <base-dir> <candidate-dir>` | `PatchTwin` + `SemanticTwin` + anchor map | patch/semantic sync fixtures | Patch output remains apply-able and semantic anchors survive toggles. |
| Review Intelligence Graph | `deep-diff-forge rank --git --json` *(not implemented)* | `--stdin-patch --rank [--json]` or `--git-range A..B --rank` (`deep-diff-forge.rank.v0`); MCP `rank_patch` | review graph nodes and ranked stream | ranking fixtures | Risk ranking is deterministic and explainable. |
| Adaptive Diff Planner | `deep-diff-forge plan --git --json` *(not implemented)* | per-file `strategy` in `--stdin-patch --json`; JSON-RPC `diff.plan` (daemon or `deep-diff-forge rpc`) returns the ranked file list | `PlannerDecision` and fallback reasons | planner budget fixtures | Strategy choices and fallback reasons are visible and bounded. |

## Acceptance Criteria Per Feature

A feature is not deployable until it has:

- documented CLI flags
- stable model fields
- human output
- JSON or JSONL output
- at least one fixture
- at least one regression snapshot
- failure mode documented
- receipt in release gate

## Minimal First Implementation Order

1. Pager-compatible patch parser and inline renderer.
2. Side-by-side projection.
3. JSON `ReviewDocument` export.
4. Git workspace input.
5. Baseline `review` TUI.
6. Syntax highlighting.
7. Semantic twin.
8. Planner budgets.
9. Review graph ranking.
10. Agent annotations.

## Deployment Link

- Framework: [Codebase Deployment Framework](DEPLOYMENT_FRAMEWORK.md)
