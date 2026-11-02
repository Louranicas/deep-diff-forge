# Vision

> **Target design, not a description of the code.** This is where the project is aiming. For what
> the code does, see the next section and [ARCHITECTURE.md](ARCHITECTURE.md#crates-as-built-030).
>
> **Status 2026-11-01.** Shipped: interactive terminal mode (`deep-diff-forge-tui`), agent mode
> read and verify paths (`cli/src/mcp.rs`, `agent::findings`), file-pair input
> (`cli/src/file_pair.rs`, unreleased). Planned, unowned: pager mode as `GIT_EXTERNAL_DIFF` or
> difftool, desktop and web renderers. Withdrawn: the loom (see [loom](DEEP_DIFF_FORGE_LOOM.md)).

## What exists today (0.3.0, updated 2026-11-01)

Against the four product modes below:

- **Pager mode:** partial. `deep-diff-forge <old> <new>` reviews two regular files (2026-10-31,
  unreleased); there is no `GIT_EXTERNAL_DIFF` or difftool mode, no directory pair, and Git is
  the only VCS read (`--git-working-tree`, `--git-staged`, `--git-range`). Patches on stdin are
  reviewed with `--stdin-patch`.
- **Interactive terminal mode:** built as `deep-diff-forge review` (patch on stdin): ranked file
  sidebar, inline or side-by-side diff, folding, wrapping, themes, mouse scroll and click,
  command palette.
- **Desktop/web embeddable mode:** no desktop or web renderer. Programs embed the engine through
  the Rust `deep-diff-forge-sdk` crate or the stdlib Python client in `sdk/python`.
- **Agent mode:** read-only. Agents get the review document, ranking, outline and hunks over
  `mcp` (eight tools, among them `review_outline`, `review_hunks` and `review_change`), `rpc`, or
  the daemon. Since 0.5.0 an agent's findings are checked against patch truth
  (`findings verify`, `findings.verify`, MCP `verify_findings`) and shown in the cockpit as
  agent notes (`review --findings`), never grounded; DDF checks where a finding points, never
  what it claims.

Deep-Diff-Forge should become the leading diff engine by merging four currently separate worlds:

1. **Patch fidelity**
   The engine must remain compatible with Git, patch files, code review systems, terminals, and automation. If a view cannot be applied, it cannot be the source of truth.

2. **Semantic understanding**
   Structural diffing must explain intent across formatting, moved expressions, renamed symbols, changed control flow, and syntax-aware units. It must not pretend syntax is available when parsing fails.

3. **Reviewer ergonomics**
   The primary user is no longer only the code author. The primary user is a reviewer supervising human and AI-generated changes across many files. The UI must optimize triage, confidence, and fast decisions.

4. **Agent collaboration**
   AI annotations must be first-class but not trusted by default. Agent notes should attach to exact hunks, symbols, commands, tests, and decisions, with provenance and review status.

## North Star

Deep-Diff-Forge is the review cockpit for modern code changes.

It should answer:

- What changed?
- Why does it matter?
- Can this patch be applied safely?
- Which regions deserve review first?
- Which tests, files, symbols, owners, and runtime behaviors are implicated?
- Which AI-generated claims are grounded in evidence?

## Design Posture

Deep-Diff-Forge will be conservative at the boundary and ambitious inside it.

- Conservative: patch truth, exact file paths, line anchors, binary handling, fallback behavior, exit codes, and Git compatibility.
- Ambitious: semantic matching, risk-ranked review, synchronized AI annotations, moved-code detection, adaptive layout, and multi-modal review surfaces.

## Baseline Product Modes

- **Pager mode:** drop-in terminal command for Git, Mercurial, Jujutsu, Fossil, and plain file comparisons.
- **Interactive terminal mode:** keyboard and mouse navigation, file sidebar, toggles, and review stream.
- **Desktop/web embeddable mode:** renderer-independent core model that can power GPUI, TUI, web, or IDE adapters.
- **Agent mode:** structured API for agents to attach claims, ask for review, and receive exact feedback against hunks or symbols.

## Differentiation Thesis

Existing tools optimize for one layer:

- Classic `diff`: patch truth.
- `delta` and `diff-so-fancy`: readable terminal rendering.
- `difftastic`: structural understanding.
- `hunk`: review UI plus AI workflow.
- `lumen`: interactive viewer ergonomics.

Deep-Diff-Forge wins by making these layers cooperate instead of choosing one.

## Deployment Link

- Framework: [Codebase Deployment Framework](DEPLOYMENT_FRAMEWORK.md)


## Review integration route

[Implementation status and delivery gates](RECOMMENDATION_INTEGRATION.md#remaining-module-routes) ↔ [Module vault note](obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fdocs--VISION) (owner's local vault). This route records planned and evaluation-dependent work without claiming it is implemented.
