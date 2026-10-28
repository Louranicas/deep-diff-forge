# Performance And Novelty Design

This document records the top-tail design choices that make Deep-Diff-Forge more than a composite of existing tools.

> **Target design, not a description of the code.** The planner, the shared AST cache daemon,
> progressive rendering and region strategies below are not built. The next section is what
> 0.3.0 has.

## What exists today (0.3.0)

Budgets in the code (constants, read from source):

| Budget | Where | Value |
| --- | --- | --- |
| Patch bytes | `deep-diff-forge-patch` `DEFAULT_BYTE_BUDGET` | 64 MiB, else `BudgetExceeded` (exit 4) |
| Patch lines | `deep-diff-forge-patch` `DEFAULT_LINE_BUDGET` | 1,000,000 physical lines |
| Syntax source bytes | `deep-diff-forge-syntax` `DEFAULT_BYTE_BUDGET` | 2 MiB, else fallback |
| Syntax nodes | `deep-diff-forge-syntax` `DEFAULT_NODE_BUDGET` | 200,000, checked after parsing |
| Syntax time | `deep-diff-forge-syntax` `backstop_for` | 1 s plus 5 ms per KiB, cooperative cancel |
| Daemon request line | `deep-diff-forge-daemon` `MAX_REQUEST_BYTES` | 80 MiB; a larger response is refused with `RESOURCE_EXHAUSTED` (8) |
| Daemon sessions | `deep-diff-forge-daemon` `MAX_SESSIONS`, `MAX_RETAINED_SESSION_BYTES` | 64 sessions, 128 MiB retained |
| MCP response | `deep-diff-forge-cli` `REVIEW_FULL_CAP` | 256 KiB, then a summary document |

Measured (2026-10-28, PR #21 to #23 work): metadata parsing of a 1M-line patch went from 139 s
to 0.20 s after a quadratic step was removed. The syntax crate's notes record ordinary Rust at
about 0.29 ms per KiB in a release build, and a 455 KiB hostile input cut at about 3 s instead
of running 125 s (`crates/deep-diff-forge-syntax/src/lib.rs`).

Not built: the adaptive planner (the parser sets `Binary` or `Line` only), region strategies,
word diff, progressive rendering, any AST or line-index cache, arena allocation. Generated and
vendored files are detected and scored to zero in ranking (`graph`), not summarised. Parallel
work exists only as `--cluster [--parallel N]`, which scores files on `std::thread::scope`
lanes and returns the same order for any worker count.

## Performance Target

Deep-Diff-Forge should feel immediate on ordinary changes and remain controlled on pathological changes.

| Case | Target behavior |
| --- | --- |
| Small file pair | Complete patch and semantic diff in one synchronous pass. |
| Medium repo diff | Show patch-first review stream quickly, fill semantic spans progressively. |
| Large generated file | Detect, suppress or summarize, never freeze the review. |
| Parser failure | Fall back to text diff with explicit reason. |
| Huge syntax graph | Stop by node/time budget, preserve patch twin, mark semantic fallback. |
| Repeated review | Reuse AST and line-index cache across sessions. |

## Budget Model

Every expensive operation receives a budget.

| Budget | Default intent |
| --- | --- |
| Byte budget | Prevent parsing huge files by accident. |
| Node budget | Prevent syntax graph explosion. |
| Time budget | Preserve interactivity. |
| Memory budget | Prevent cache and graph overgrowth. |
| File count budget | Switch to progressive mode for huge review sets. |

Budgets are part of planner input and fallback output.

## Adaptive Strategy Ladder

```mermaid
flowchart TB
    Start[File candidate]
    Binary{Binary or unsupported file kind?}
    Generated{Generated or vendored?}
    Small{Within syntax budget?}
    Parsed{Parser clean enough?}
    Syntax[Syntax strategy]
    Region[Region syntax strategy]
    Line[Line strategy]
    Summary[Suppressed summary]
    BinaryOut[Binary metadata strategy]

    Start --> Binary
    Binary -- yes --> BinaryOut
    Binary -- no --> Generated
    Generated -- yes --> Summary
    Generated -- no --> Small
    Small -- yes --> Parsed
    Small -- no --> Region
    Parsed -- yes --> Syntax
    Parsed -- no --> Line
    Region --> Line
```

## Progressive Rendering

The engine emits usable partial results:

1. File list and status.
2. Patch twins.
3. Side-by-side rows.
4. Syntax highlighting.
5. Semantic spans.
6. Review graph risk ranking.
7. Agent annotations and evidence links.

This lets the UI become useful before the deepest analysis completes.

## Novel Features Beyond Exemplars

### 1. Semantic Patch Twin

The engine maintains synchronized patch and semantic models. This enables:

- patch application and semantic review in one surface
- comments that survive view toggles
- exact line anchors plus syntax-node anchors
- graceful fallback without losing review state

### 2. Review Intelligence Graph

The graph ranks review work by expected value. Signals include:

- public API changes
- control-flow changes
- dependency fan-out
- test touch distance
- generated or vendored code
- ownership hints
- ungrounded AI claims
- previous review decisions

### 3. Adaptive Diff Planner

The planner chooses the cheapest truthful strategy per file and region. It can mix:

- line diff
- word diff
- syntax diff
- moved-block detection
- reformat-only classification
- binary metadata
- generated-file suppression

### 4. Grounded Agent Layer

Agent annotations carry provenance and evidence. The display distinguishes:

- grounded claim
- partially grounded claim
- ungrounded claim
- contradicted claim
- reviewer-approved note

### 5. Shared AST Cache Daemon

Not built: the 0.3.0 daemon holds review sessions only and reports `cache_entries: 0`. The
target: the optional daemon caches syntax and line metadata across clients. This is the reason Unix sockets are included. It turns repeated review, IDE integration, and agent workflows into cache hits rather than repeated parses.

## Implementation Techniques

| Technique | Use |
| --- | --- |
| Arena allocation | Fast syntax graph construction and disposal. |
| Content fingerprints | Cache invalidation and duplicate detection. |
| Parallel file planning | Independent per-file strategy selection. |
| Incremental projection | UI updates before all semantic passes finish. |
| Stable IDs | Comments and annotations survive rerenders. |
| Conservative fallback | Never hide changes because a parser failed. |
| Snapshot journals | Reproducible review and agent sessions. |

## Regression Culture

Borrowing from Difftastic, the repo should grow a sample corpus immediately.

Minimum fixture classes:

- pure line edits
- reformat-only changes
- moved function
- renamed symbol
- generated file
- binary file
- malformed syntax
- parser unsupported language
- large file budget fallback
- patch metadata: rename, mode change, deletion, no-newline
- AI annotation with valid evidence
- AI annotation with missing evidence

## Deployment Link

- Framework: [Codebase Deployment Framework](DEPLOYMENT_FRAMEWORK.md)


## Review integration route

[Implementation status and delivery gates](RECOMMENDATION_INTEGRATION.md#remaining-module-routes) ↔ [Module vault note](obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fdocs--PERFORMANCE_AND_NOVELTY) (owner's local vault). This route records planned and evaluation-dependent work without claiming it is implemented.
