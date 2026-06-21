# Vision

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

