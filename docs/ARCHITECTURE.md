# Architecture

> **Target design, not a description of the code** (anchor E-01, 2026-10-27). Crates, commands and caches named here may not exist. The crates and edges that do exist are generated in [Current crates and edges](SCHEMATICS.md#current-crates-and-edges) from `docs/crate-edges.txt`, which a test holds equal to `cargo metadata`.

Deep-Diff-Forge is organized around a layered core.

## Platform scope (decision D-05, 2026-10-27)

Deep-Diff-Forge targets Linux and macOS. Windows is not a goal: the daemon uses Unix-domain
sockets and refuses to compile elsewhere (`deep-diff-forge-daemon/src/lib.rs`, `compile_error!`),
and the CLI and TUI depend on it unconditionally. The crates are not split to make a Windows build
possible; library crates that happen to build on Windows (`patch`, `learning`) are not tested or
released there.

## Source headers (decision E-05, 2026-10-27)

Some files open with `// DDF integration`, `// Design/status` and `// Module vault note` lines that
link to a design section and to the maintainer's local Obsidian vault (65 of 120 Rust files on
2026-10-27; all 79 links resolved). They are optional and not enforced: CI cannot see the vault, so
a scan could check only that a header is present, which would push headers onto files with no note
to link. Add one when a vault note exists for the file; never add a link to a note that does not.

## Layers

1. **Input Layer**
   Reads Git state, patch files, file pairs, directories, stdin streams, and future forge APIs.

2. **Patch Truth Layer**
   Produces canonical apply-able patch data:
   paths, modes, renames, hunk headers, old/new line numbers, additions, removals, context, and metadata.

3. **Semantic Layer**
   Parses supported languages, builds syntax trees, computes structural matches, detects moved or reformatted code, and emits semantic spans.

4. **Planner Layer**
   Chooses strategies per file and region:
   line, word, syntax, moved-block, binary, generated-file, or fallback.

5. **Review Graph Layer**
   Connects files, hunks, symbols, tests, risk signals, ownership, comments, agent notes, and commands.

6. **Projection Layer**
   Converts the core model into side-by-side, inline, stacked, JSON, TUI, desktop, web, and agent API views.

7. **Pipeline Layer**
   Runs chainable command stages: ingest, plan, rank, annotate, render, chain, and cluster.

8. **Loom Layer**
   Assimilates exemplar lessons and feature proposals into Rust crate plans, fixtures, gates, and receipts.

## Core Principle

Patch truth and semantic truth are separate but synchronized.

The patch twin answers: "What exact text can be applied?"

The semantic twin answers: "What code meaning changed?"

The projection layer joins them through stable anchors:

- file identity
- old/new byte ranges
- old/new line ranges
- syntax node identities
- hunk ids
- semantic span ids

## Crate Plan

Initial:

- `deep-diff-forge-core`: shared model, planner enums, patch/semantic/review graph vocabulary.
- `deep-diff-forge-cli`: CLI entry point and smoke surface.

Planned:

- `deep-diff-forge-git`: Git/gix integration.
- `deep-diff-forge-patch`: robust unified patch parser and renderer.
- `deep-diff-forge-syntax`: tree-sitter registry and semantic matcher.
- `deep-diff-forge-tui`: interactive terminal UI.
- `deep-diff-forge-agent`: annotation, provenance, and review API.
- `deep-diff-forge-ui-model`: renderer-neutral layout and interaction model.
- `deep-diff-forge-pipeline`: chain stages, stream codecs, and manifest runner.
- `deep-diff-forge-cluster`: dimensional lane sharding, parallelism, and deterministic joins.
- `deep-diff-forge-loom`: controlled assimilation plans, fixtures, gates, and receipts.
- `deep-diff-forge-daemon`: optional local IPC daemon for shared cache and multi-client review sessions.

## Schematics

Detailed diagrams and API maps live in:

- `docs/SCHEMATICS.md`
- `docs/API_AND_IPC.md`
- `docs/CHAINING_AND_CLUSTERING.md`
- `docs/DIMENSIONAL_EXECUTION_MODEL.md`
- `docs/DEEP_DIFF_FORGE_LOOM.md`
- `docs/RUST_IMPLEMENTATION_STRATEGY.md`
- `docs/MODULE_STRUCTURE_PLAN.md`
- `docs/PERFORMANCE_AND_NOVELTY.md`

## Non-Goals

- Do not replace Git.
- Do not make non-apply-able semantic output the canonical patch.
- Do not trust AI annotations without provenance.
- Do not require a desktop app for the engine to be useful.

## Deployment Link

- Framework: [Codebase Deployment Framework](DEPLOYMENT_FRAMEWORK.md)


## Review integration route

[Implementation status and delivery gates](RECOMMENDATION_INTEGRATION.md#remaining-module-routes) ↔ [Module vault note](obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fdocs--ARCHITECTURE). This route records planned and evaluation-dependent work without claiming it is implemented.


## Captured multi-provider review controller

`deep-diff-forge-review` depends inward on core vocabulary and the canonical changed-source digest. It admits shared-service analysis artifacts and emits review/verification artifacts through the CLI. Existing daemon and TUI routes are not yet migrated. [Full implemented workflow, interfaces and limits](REVIEW_COUNCIL.md) ↔ [Vault](obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FReview%20Council).
