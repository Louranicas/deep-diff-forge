# Pioneer Feature Specifications

This document converts the three above-baseline features into implementation-ready specs.

> **Target design, not a description of the code.** None of the commands in the `CLI` blocks
> below exist at `v0.11.0`; each is refused as an unknown command (exit 2). The `rank`
> subcommand was withdrawn on 2026-10-31 (`--rank` serves it). The next section lists what
> ships at `v0.11.0`.
>
> **Status 2026-11-25 (`v0.11.0`).** Shipped: the semantic twin through `analyze` (`pipeline::review`),
> Rust only; file-level 27-signal risk ranking with dual-reader code evidence (`graph::{risk,evidence,rust_tokens,lexical}`)
> and structured `diagnostics.v0` (`graph::diagnostics`). Planned, unowned: hunk, symbol, test
> and owner nodes; the adaptive diff planner and its budget profiles (`PlannerLearner` exists in
> `deep-diff-forge-learning` as an unwired library type only). Withdrawn: the `rank` subcommand
> (2026-10-31; use `--rank`). Nothing here is on hold.

## What exists today (v0.11.0)

| Feature | Built | Commands & surfaces that exist |
| --- | --- | --- |
| Semantic patch twin | `core::ReviewFile` carries a `PatchTwin` and an optional `SemanticTwin`. The parser never fills the semantic twin; only `analyze` does, after checking the patch against base and candidate directories (`crates/deep-diff-forge-pipeline/src/review.rs`). Rust is the only tree-sitter grammar (`DEFAULT_BYTE_BUDGET = 2 MiB`, `DEFAULT_NODE_BUDGET = 200_000`). | `semantic <path> [--json]`, `structural <old> <new> [--json]`, `analyze <patch> <base-dir> <candidate-dir>` |
| Review intelligence graph | File-level deterministic additive scoring over **27 signals** (`crates/deep-diff-forge-graph/src/{risk,evidence,rust_tokens,lexical,vocabulary,diagnostics}.rs`): 13 `RiskSignal` path/shape facts (`public_api_surface`, `new_file`, `deleted_file`, `binary_change`, `diff_suppressed`, `large_change`, `many_hunks`, `config_or_lockfile`, `test_only`, `generated_or_vendored`, `hazardous_text`, `symlink_change`, `executable_bit`) plus 14 `EvidenceSignal` code-evidence signals (`unsafe_added`, `process_spawn`, `network_fetch`, `dynamic_eval`, `ci_privilege`, `build_hook`, `secret_like`, `check_suppressed`, `test_removed`, and the 5 integrity signals `test_skipped`, `trivial_assert`, `stub_added`, `error_swallowed`, `panic_path_added`), each evidence item (up to 32 per file) anchored to a hunk address and line; canonical `rank.v0` with `tool.scorer_sha256`; and `deep-diff-forge.diagnostics.v0` (`<label>@line` / `<label>@file` with repair ids). No hunk, symbol, test or owner graph nodes. | `--rank [--json] [--fail-on LIST]`, `--diagnostics [--json] [--fail-on LIST]` (on `--stdin-patch`, `--git-*`, `<old> <new>`), `schema signals\|diagnostics`, `explain signal`, daemon `diff.plan`/`diff.rank`/`diff.diagnostics` (protocol 3), MCP `rank_patch`/`diagnose_patch`/`review_change`, Rust & Python SDKs |
| Adaptive diff planner | Not built in the diff engine. The parser sets `DiffStrategy::Binary` for binary files and `DiffStrategy::Line` for everything else (`crates/deep-diff-forge-patch/src/parser.rs`). No budget profiles on CLI/daemon (`PlannerLearner` exists in `deep-diff-forge-learning` as library-only code, unwired). | none (`diff.plan` returns the ranked file list) |

## 1. Semantic Patch Twin

### Purpose

Preserve apply-able patch truth while adding syntax-level understanding.

### Inputs

- canonical patch twin
- old/new file bytes
- language guess
- parser budget
- tree-sitter parse output when supported

### Outputs

- `PatchTwin`: apply-able hunk structure
- `SemanticTwin`: syntax spans and parse status
- `AnchorMap`: file, hunk, line, byte, and syntax-node links

### CLI

Target commands, not built.

```bash
deep-diff-forge --git --semantic --json
deep-diff-forge old.rs new.rs --semantic --layout split
deep-diff-forge explain-span <span-id> --json
```

### Failure Modes

| Failure | Behavior |
| --- | --- |
| Unsupported language | Keep patch twin, semantic fallback reason. |
| Parser errors | Use parse error budget, then fallback. |
| Node budget exceeded | Stop semantic analysis, keep patch twin. |
| Anchor mismatch | Mark semantic span invalid, never alter patch truth. |

### Tests

- reformat-only change keeps patch lines and marks semantic reformat
- moved function links old and new spans
- malformed syntax keeps patch twin
- unsupported language exports fallback reason

## 2. Review Intelligence Graph

### Purpose

Rank review work by likely impact, not file order.

### Nodes

- file
- hunk
- symbol
- test
- owner
- risk signal
- command evidence
- agent annotation
- reviewer decision

### Edges

- file contains hunk
- hunk touches symbol
- symbol is covered by test
- hunk has risk signal
- annotation claims about hunk
- command produced evidence
- reviewer decision resolves annotation

### CLI

Target commands, not built.

```bash
deep-diff-forge rank --git
deep-diff-forge rank --git --json
deep-diff-forge graph --git --json
deep-diff-forge why-first <hunk-id>
```

### Ranking Signals

- public API edit
- control-flow edit
- dependency fan-out
- security-sensitive path
- generated/vendor suppression
- test distance
- ownership
- prior fallback
- ungrounded agent claim

### Tests

- public API hunk outranks comment-only edit
- generated file suppresses below source edit
- ungrounded agent claim raises review priority
- ranking explanation is deterministic

## 3. Adaptive Diff Planner

### Purpose

Choose the cheapest truthful diff strategy per file and region.

### Strategies

- binary metadata
- generated summary
- line diff
- word diff
- syntax diff
- region syntax diff
- moved-block diff

### CLI

Target commands, not built.

```bash
deep-diff-forge plan --git
deep-diff-forge plan --git --json
deep-diff-forge --git --budget fast
deep-diff-forge --git --budget deep
```

### Budget Profiles

| Profile | Use |
| --- | --- |
| `fast` | Claude Code quick review and CI preflight. |
| `balanced` | Default human review. |
| `deep` | Expensive semantic review. |
| `forensic` | Offline corpus analysis and benchmark runs. |

### Fallback Requirements

Every fallback records:

- attempted strategy
- fallback strategy
- reason
- elapsed time
- byte count
- node count if available
- whether patch truth remained complete

### Tests

- large file selects line or summary instead of syntax
- binary file selects binary metadata
- small Rust file selects syntax
- parser failure falls back to line diff with reason
- `fast` and `deep` profiles produce explainable differences

## Deployment Link

- Framework: [Codebase Deployment Framework](DEPLOYMENT_FRAMEWORK.md)


## Review integration route

[Implementation status and delivery gates](RECOMMENDATION_INTEGRATION.md#remaining-module-routes). This route records planned and evaluation-dependent work without claiming it is implemented.
