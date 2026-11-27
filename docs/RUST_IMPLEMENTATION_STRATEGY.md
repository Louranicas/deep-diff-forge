# Rust Implementation Strategy

Deep-Diff-Forge is a Rust codebase. The architecture should make that choice
matter: predictable binaries, strong data contracts, bounded memory, explicit
errors, and reliable concurrency.

> **Target design, not a description of the code**, except this first section. Crates and
> dependencies named below may not exist.
>
> **Status 2026-11-25 (`v0.11.0`).** Shipped: everything in the next section (**15 workspace
> crates** under `crates/`: 14 published crates + dev-only, unpublished `deep-diff-forge-testkit`).
> Planned, unowned: the `git` and `planner` crates and the dependencies the later sections name
> but the code does not use. Withdrawn 2026-10-31: the `loom` crate and its milestone step.

## What exists today (v0.11.0)

- **Toolchain & profiles (`Cargo.toml`):** Rust edition `2024`, Cargo `resolver = "2"`, MSRV
  `1.88` (`rust-version = "1.88"`, verified by `cargo +1.88.0 check --workspace --all-targets --locked`);
  the remaining CI jobs and `scripts/ci_local.sh` pin `1.99.1` (`.github/workflows/ci.yml`).
  `[profile.release]` sets `codegen-units = 1` for reproducible release binaries and stable
  benchmark performance; `[profile.dev.package.deep-diff-forge-core]` sets `opt-level = 3` so
  pure-Rust SHA-256 and HMAC-SHA-256 stay fast in debug builds (for example during stop-hook
  ranker sealing).
- **Unsafe, lints & effect doors (`Cargo.toml`, `clippy.toml`):** `[workspace.lints.rust]`
  enforces `unsafe_code = "forbid"` across every workspace crate (and `fuzz/Cargo.toml`) so no
  crate can override it locally. `[workspace.lints.clippy]` denies `unwrap_in_result`,
  `panic_in_result_fn`, `allow_attributes`, and `allow_attributes_without_reason` (suppressions
  must use `#[expect(..., reason = "...")]`). `clippy.toml` enforces effect doors (anchor B-04)
  via `disallowed-methods` for `std::process::Command::new`, `std::env::var`, `std::env::var_os`,
  `std::time::Instant::now`, `std::time::SystemTime::now`, `std::thread::sleep`,
  `std::env::temp_dir` (requiring `deep_diff_forge_testkit::Scratch`), and unbounded child waits
  (`Command::output`, `Command::status`, `Child::wait`, `Child::wait_with_output`, requiring
  `deep_diff_forge_core::process::run_bounded`).
- **Supply-chain policy (`deny.toml`):** `cargo-deny` enforces `advisories` v2 (`yanked = "deny"`,
  `unmaintained = "all"`, `ignore = []`), `licenses` v2 (permissive allow-list compatible with
  `MIT OR Apache-2.0`), `bans` (`multiple-versions = "warn"`, `wildcards = "deny"`,
  `allow-wildcard-paths = true` exclusively for dev-only `deep-diff-forge-testkit`), and
  `sources` (`unknown-registry = "deny"`, `unknown-git = "deny"`), paired with strict
  `cargo-audit --deny warnings` and SPDX SBOM verification (`sbom.spdx.json`).
- **Workspace crates (15 total: 14 published + `testkit`):** exact dependency DAG enforced in
  `docs/crate-edges.txt` and [ARCHITECTURE.md](ARCHITECTURE.md) (note: `graph` depends only on
  `core` in normal dependencies and does not depend on `syntax`). `git`, `planner`, and `loom`
  from the target table below do not exist; `learning`, `review`, `sdk`, and `testkit` exist and
  are not in it:
  - `deep-diff-forge-core`: pure domain vocabulary (`ReviewFile`, hunks, anchors, `InputSeal`),
    pure-Rust SHA-256 and RFC 2104 HMAC-SHA-256 (`seal.rs`), unified exit-code registry (`exit.rs`),
    bounded child execution (`process.rs`), deployment/snapshot types.
  - `deep-diff-forge-patch`: strict 42-shape Git header parser (`parser.rs`, `CONFORMANCE.md`),
    `SealedReview`, `InputGuards`, `git_acquire.rs`, `git_raw.rs`, `judge_patch.rs`, `render.rs`,
    hand-rolled `review.v0` JSON (`json` default feature).
  - `deep-diff-forge-projection`: inline and side-by-side (`split`) projections, note rows, display width.
  - `deep-diff-forge-pipeline`: `IngestStage`, `RenderStage` (`--jsonl`), source-pair `analyze` (`analysis.v1`).
  - `deep-diff-forge-syntax`: Rust-only tree-sitter (`DEFAULT_BYTE_BUDGET = 2 MiB`, `DEFAULT_NODE_BUDGET = 200_000`),
    symbols (`semantic.v0`), terminal-safe ANSI highlighting, token-level `structural_diff`.
  - `deep-diff-forge-graph`: 27-signal risk and dual-reader evidence engine (`risk.rs`, `evidence.rs`,
    `rust_tokens.rs`, `lexical.rs`, `lexical/legacy_rust.rs`, `vocabulary.rs`), canonical `rank.v0`
    with `scorer_sha256`, and `diagnostics.v0` (`diagnostics.rs`).
  - `deep-diff-forge-agent`: annotation provenance, grounding, sanitization, anchor validation,
    and verified agent findings (`finding.v0` -> `findings.v0`).
  - `deep-diff-forge-cluster`: bounded `std::thread::scope` parallel `patch` + `risk` lanes with
    deterministic `ranked-review-order` join and receipt (`cluster.v0`).
  - `deep-diff-forge-learning`: local-only strategy receipt store (`$XDG_STATE_HOME/deep-diff-forge/learning/`),
    `TrustPolicy` (`learn record|status`), and library-only learners (`PlannerLearner`,
    `RankingWeights`, `AnnotationTrust`, `PromotionPolicy`).
  - `deep-diff-forge-daemon`: owner-private Unix-socket JSON-RPC 2.0 server (`PROTOCOL_VERSION = 3`,
    17 methods, max 8 connections, 128 MiB in-flight cap, 80 MiB line cap, paging, metrics, peer credentials).
  - `deep-diff-forge-tui`: ratatui/crossterm interactive review cockpit (`review`, `--probe`, `--side`, `--findings`).
  - `deep-diff-forge-review`: adversarial review council (6 slots, 2 rounds, 8 rootless Podman
    gates, `mutation_scope.rs`, `tuicr.rs` interop, `journal.rs` hash-chained decision journal
    with exit 3 `recorded_incomplete` and DDF-2026-004 recovery).
  - `deep-diff-forge-sdk`: stable Rust facade (`review`, `rank`, `diagnostics`, Unix `Client` with `ProtocolTooOld`).
  - `deep-diff-forge-cli`: `deep-diff-forge` binary (all CLI modes, 9-tool `mcp`, `rpc`, `envelope`,
    `schema`, `explain`, `agent {setup,doctor,check,verify-branch,hook}` with state v13 & `.git/deep-diff-forge/`
    ledger, `eval`, `bench`, `learn`, `council`, `deploy`, withdrawn command refusals).
  - `deep-diff-forge-testkit` (`publish = false`, dev-only): self-removing `Scratch` directories
    and shared test helpers; never a normal dependency.
- **External dependencies actually used** (verified from `crates/*/Cargo.toml` at `v0.11.0`):
  `nix` (`core` since 0.10.0, `review`, `cli`, and `daemon` off Linux/Android for `getpeereid`),
  `serde` (`daemon`, `learning`, `review`), `serde_json` (`agent`, `daemon`, `learning`, `review`,
  `cli`, `sdk`), `rustix` (`daemon`, `learning`), `signal-hook` (`daemon`), `tree-sitter` and
  `tree-sitter-rust` (`syntax`, pinned), `ratatui` and `crossterm` (`tui`; `use-dev-tty` on macOS),
  `unicode-width` and `unicode-segmentation` (`projection`). `patch`, `graph`, `cluster`, and
  `pipeline` declare no direct external dependencies (each reaches `nix` transitively through
  `core`); only dev-only `deep-diff-forge-testkit` has zero external dependencies (`deny.toml`).
- **Not used:** `clap` (arguments are parsed by `crates/deep-diff-forge-cli/src/args.rs`),
  `gix` (Git input runs the `git` program), `rayon` (`cluster` uses `std::thread::scope`),
  `tokio` (the daemon uses `std::os::unix::net` and threads), `blake3` (seals and HMAC history
  use pure-Rust SHA-256 and RFC 2104 HMAC-SHA-256 in `crates/deep-diff-forge-core/src/seal.rs`).
- **Errors:** each crate has its own error type (for example `PatchParseError`, `ReviewError`,
  `LearningError`, `RpcError`); there is no shared `EngineError`. The CLI maps them to the
  exit-code registry (`deep-diff-forge schema exit-codes`, including the 11 unified exit-7
  refusal classes and council exit code 3 `recorded_incomplete`).
- **Testing:** unit, fixture, doc-contract, conformance, integration, contract-matrix
  (`scripts/contract_matrix.sh`), and fuzz levels exist, plus a labelled evaluation corpus
  (`fixtures/eval/MANIFEST.json`, scored by `eval`: 11 cases in 0.5.0, 34 in 0.6.0, 35 in 0.8.4,
  56 in 0.9.0/0.10.0, **73 cases** in `v0.11.0` with **155 total declared items** [130 `known_miss`
  + 25 `known_false_positive`, including 46 non-Rust misses declared on 2026-11-24]). **18**
  libFuzzer binary targets are declared in `fuzz/Cargo.toml` (`scripts/fuzz_target.sh --list`):
  `patch_parser`, `review_json`, `daemon_protocol`, `agent_annotation`, `council_report`,
  `stdio_mcp`, `stdio_rpc`, `structural_soundness`, `findings_verify`, `evidence_scan`,
  `rust_tokens`, `rust_code_view`, `rust_hunk_places`, `hook_input`, `judge_patch`,
  `verdict_reader`, `journal_recover`, and `git_raw_stream` (plus shared helper
  `stdio_invariants.rs`, giving 19 `.rs` files in `fuzz/fuzz_targets/`). CI runs diff-scoped
  `cargo mutants --no-config --in-diff` on each pull request's changed lines.
- **First milestone below:** steps 1 to 5 are done; step 6 (loom fixture) is withdrawn with
  the loom (2026-10-31).

## Rust Principles

- Prefer small crates with narrow ownership.
- Keep stable models in `deep-diff-forge-core`.
- Use explicit error types at crate boundaries.
- Avoid panics outside tests and contract bootstrap assertions.
- Avoid `unsafe` unless isolated, measured, fuzzed, and feature-gated.
- Stream large data instead of collecting whole repos.
- Treat CLI, daemon, TUI, and agent surfaces as projections over the same model.
- Use evidence-labelled implementation and reporting from
  [Agentic Rust Coder V4](AGENTIC_RUST_CODER_V4.md).

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

Target table, written before the code; see [What exists today](#what-exists-today-v0110) for the
dependencies the workspace actually uses.

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
| Corpus | Optional corpus runs on the operator workstation's 10TB disk, with receipts. |
| Fuzz | Patch parser, syntax lowering, JSONL codecs. |
| Integration | Git workspace, TUI smoke, daemon socket lifecycle. |

Coverage is judged by diff-scoped mutation, not by a test count: a change leaves no
surviving mutant in the production code it touches. The detailed standard is
defined in `docs/TESTING_GOLD_STANDARD.md`.

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
