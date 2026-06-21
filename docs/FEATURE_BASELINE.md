# Feature Baseline

This baseline is non-negotiable for the first complete product cut.

## Matrix Coverage

| Capability | Requirement |
| --- | --- |
| Review-first interactive UI | File review flow is the default, not an afterthought. |
| Multi-file review stream + sidebar | Stream view, file tree/sidebar, changed-file status, and jump navigation. |
| Inline agent / AI annotations | Agent notes anchored to files, hunks, lines, symbols, and commands with provenance. |
| Responsive auto split/stack layout | Side-by-side on wide surfaces, stacked or inline on narrow surfaces. |
| Mouse support inside the viewer | Scroll, select, open, collapse, resize, and context actions. |
| Runtime view toggles | Toggle syntax/line/word/semantic, context size, whitespace, comments, moves, generated files, and annotations. |
| Syntax highlighting | Tree-sitter-based where possible, fallback highlighter for plain text. |
| Structural diffing | Syntax-aware matching with conservative fallback and explicit parse status. |
| Pager-compatible mode | Works as `GIT_EXTERNAL_DIFF`, `git difftool`, `LESS`/pager stream, and plain CLI. |

## Compatibility Requirements

- Produce and consume unified patches.
- Preserve exact old/new file labels, modes, renames, binary markers, and no-newline metadata.
- Support directory comparisons.
- Support unsupported and malformed files through text fallback.
- Support machine-readable JSON output.
- Keep exit-code semantics explicit and scriptable.

## Performance Requirements

- Large changes must degrade, not hang.
- Every expensive diff strategy gets a byte/node/time budget.
- The planner records fallback reasons.
- Generated and vendored files must be detectable and suppressible.
- Interactive UI should target stable frame budgets, with progressive loading for large review sets.

## Trust Requirements

- AI annotations are never silently merged into the diff model.
- Every AI claim has source spans or is marked ungrounded.
- Approval state belongs to the reviewer, not the agent.
- Patch truth remains auditable without AI features enabled.

## Deployability Requirements

- Every feature has a CLI path.
- Every feature has a JSON or JSONL path for Claude Code.
- Every feature has Bash-safe exit-code behavior.
- Every feature has a fixture and regression receipt before release.
- Every feature degrades to patch truth when semantic or agent layers fail.
