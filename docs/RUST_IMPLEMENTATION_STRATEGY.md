# Rust Implementation Strategy

Deep-Diff-Forge is a Rust codebase. The architecture should make that choice
matter: predictable binaries, strong data contracts, bounded memory, explicit
errors, and reliable concurrency.

## Rust Principles

- Prefer small crates with narrow ownership.
- Keep stable models in `deep-diff-forge-core`.
- Use explicit error types at crate boundaries.
- Avoid panics outside tests and contract bootstrap assertions.
- Avoid `unsafe` unless isolated, measured, fuzzed, and feature-gated.
- Stream large data instead of collecting whole repos.
- Treat CLI, daemon, TUI, and agent surfaces as projections over the same model.

## Proposed Crate Expansion

| Crate | Phase | Purpose |
| --- | --- | --- |
| `deep-diff-forge-patch` | 1 | Unified patch parser, renderer, row anchors. |
| `deep-diff-forge-projection` | 1 | Inline, side-by-side, stacked, JSON, compact projections. |
| `deep-diff-forge-pipeline` | 1 | Chain stages, stream codecs, manifest runner. |
| `deep-diff-forge-git` | 2 | Git workspace and external diff adapters. |
| `deep-diff-forge-tui` | 2 | Review-first terminal UI and mouse support. |
| `deep-diff-forge-syntax` | 3 | Tree-sitter registry and semantic matching. |
| `deep-diff-forge-planner` | 3 | Adaptive strategy selection and fallbacks. |
| `deep-diff-forge-graph` | 4 | Review Intelligence Graph and ranking. |
| `deep-diff-forge-agent` | 5 | Annotation protocol, provenance, evidence. |
| `deep-diff-forge-cluster` | 6 | Parallel dimensional execution lanes. |
| `deep-diff-forge-loom` | 6 | Assimilation plans, fixtures, gates, receipts. |
| `deep-diff-forge-daemon` | 7 | Optional IPC, shared cache, subscriptions. |

The detailed module tree, dependency graph, crate charters, and contextual code
flows live in `docs/MODULE_STRUCTURE_PLAN.md`.

## Dependency Posture

| Area | Likely dependency | Reason |
| --- | --- | --- |
| CLI parsing | `clap` | Stable command contracts and completions. |
| Serialization | `serde`, `serde_json` | JSON and JSONL contracts. |
| Git | `gix` | Rust-native Git operations. |
| Syntax | `tree-sitter` | Structural diffing. |
| Parallelism | `rayon` | CPU-bound file and syntax lanes. |
| Async IPC | `tokio` | Daemon sockets and subscriptions. |
| TUI | `ratatui`, `crossterm` | Terminal layout, mouse, keyboard. |
| Hashing | `blake3` | Fast content-addressed cache keys. |

Dependencies should be introduced when the owning phase starts, not during
architecture bootstrap.

## Error Model

Every recoverable failure should become a typed fallback record.

```rust
pub enum EngineError {
    Input(InputError),
    Patch(PatchError),
    Syntax(SyntaxError),
    Planner(PlannerError),
    Projection(ProjectionError),
    Ipc(IpcError),
    Contract(ContractError),
}
```

Patch parsing errors can be hard failures when no patch truth can be produced.
Semantic, risk, agent, history, and presentation failures should preserve patch
truth whenever possible.

## Testing Ladder

| Level | Tests |
| --- | --- |
| Unit | IDs, ranges, parser fragments, planner decisions. |
| Fixture | Unified patches, semantic examples, render snapshots. |
| Contract | CLI exit codes, JSON schemas, JSONL events, stdout/stderr split. |
| Corpus | Optional 10TB corpus runs with receipts. |
| Fuzz | Patch parser, syntax lowering, JSONL codecs. |
| Integration | Git workspace, TUI smoke, daemon socket lifecycle. |

## No Hidden Global State

The CLI should pass explicit config, budgets, paths, and caches into engine
objects. The daemon may hold shared state, but it must expose state through
versioned APIs and receipts.

## First Implementation Milestone

1. Add `deep-diff-forge-patch`.
2. Parse a small unified patch into `PatchTwin`.
3. Render the same `PatchTwin` back to an apply-able patch.
4. Emit `deep-diff-forge --stdin-patch --json`.
5. Add chain smoke tests for stdin, stdout, stderr, and exit codes.
6. Add loom fixture for the patch parser baseline.

## Deployment Link

- Framework: [Codebase Deployment Framework](DEPLOYMENT_FRAMEWORK.md)
