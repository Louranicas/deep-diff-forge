# Feature Deployability Matrix

Every baseline and pioneer feature must have a command path, model path, test path, and release gate.

## Baseline Features

| Feature | CLI/Bash surface | Model/API surface | Test gate | Deployable when |
| --- | --- | --- | --- | --- |
| Review-first interactive UI | `deep-diff-forge review --git` | `ReviewDocument`, projections | TUI smoke + snapshot | Keyboard and mouse review works without daemon. |
| Multi-file review stream + sidebar | `review --git`, `--json` | file list, status, ranked stream | multi-file fixture | Changed files can be navigated and exported. |
| Inline agent / AI annotations | `annotation add`, JSONL API | `AgentAnnotation` | grounded/ungrounded fixtures | Annotations anchor to file/hunk/span and never mutate patch truth. |
| Responsive auto split/stack layout | `--layout auto|split|stack|inline` | projection config | terminal width snapshots | Same diff renders correctly at narrow and wide widths. |
| Mouse support inside viewer | `review --mouse` | TUI event model | terminal integration smoke | Scroll/select/collapse work in supported terminals. |
| Runtime view toggles | `--toggle <name>=<value>`, TUI keys | session toggles | projection toggle tests | Toggles rerender without recomputing patch truth. |
| Syntax highlighting | `--syntax on|off` | token spans | language fixtures | Supported files highlight and unsupported files degrade. |
| Structural diffing | `--semantic on|off` | `SemanticTwin` | syntax fixtures | Semantic spans synchronize to patch hunks. |
| Pager-compatible mode | default pipe/TTY behavior | inline projection | Git external diff tests | Works with Git diff/difftool and plain shell pipes. |

## Pioneer Features

| Feature | CLI/Bash surface | Model/API surface | Test gate | Deployable when |
| --- | --- | --- | --- | --- |
| Semantic Patch Twin | `deep-diff-forge --git --semantic --json` | `PatchTwin` + `SemanticTwin` + anchor map | patch/semantic sync fixtures | Patch output remains apply-able and semantic anchors survive toggles. |
| Review Intelligence Graph | `deep-diff-forge rank --git --json` | review graph nodes and ranked stream | ranking fixtures | Risk ranking is deterministic and explainable. |
| Adaptive Diff Planner | `deep-diff-forge plan --git --json` | `PlannerDecision` and fallback reasons | planner budget fixtures | Strategy choices and fallback reasons are visible and bounded. |

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
