# Architecture

Deep-Diff-Forge is organized around a layered core.

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
- `docs/PERFORMANCE_AND_NOVELTY.md`

## Non-Goals

- Do not replace Git.
- Do not make non-apply-able semantic output the canonical patch.
- Do not trust AI annotations without provenance.
- Do not require a desktop app for the engine to be useful.
