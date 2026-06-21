# Deployment Evidence — L0 → L4 (Patch + Projection + Pipeline + Semantic)

## L4 Semantic (continuation, first heavy-dependency wave)

`claim | warrant | evidence`

- `deep-diff-forge-syntax` crate shipped | `[VBR]` | tree-sitter language
  detection, budgeted parse (byte + node budgets → explicit `FallbackReason`),
  top-level symbol extraction, `enclosing_symbol`. Time-budget enforcement is
  honestly deferred (not reported as a fallback).
- First external deps introduced under policy | `[VBE]` | `tree-sitter 0.25` +
  `tree-sitter-rust 0.24` fetched and C-compiled (cc) cleanly; `cargo deny
  check` → `advisories ok, bans ok, licenses ok, sources ok`. Internal path
  deps carry `version = "0.1.0"` (publishable-workspace pattern).
- CLI: `semantic <path> [--json]` | `[VBE]` | emits `deep-diff-forge.semantic.v0`;
  on `crates/deep-diff-forge-core/src/deploy.rs` it correctly identified
  MaturityLevel/GateState/GateResult/DeploymentStatus (enum/impl/struct) + the
  tests module with line ranges.
- Shared `core::json_escape` | `[VBR]` | one canonical RFC-8259 escaper added to
  core (was the 4th place needing it).
- Gate green | `[VBE]` | `just gate-feature` exit 0; **276 tests passed, 0
  failed**; `deep-diff-forge-syntax` carries **51 tests** (>50 bar).
- `deploy status` now reports `L4 (Semantic)` | `[VBE]`.

Honest-degraded: full patch↔symbol join (mapping a hunk to its enclosing symbol
in a diff) needs file bytes the bare `--stdin-patch` lacks — it is the Git-input
wave's job; `enclosing_symbol` is the ready building block. The `semantic`
command proves the layer live on whole files.

## Deployment-Spine Hardening (closes gap-analysis P0s, zero-touch)

`claim | warrant | evidence`

- Typed deployment vocabulary in `core` | `[VBR]` | `deploy.rs`:
  `MaturityLevel`, `GateState`, `GateResult`, `DeploymentStatus` (gap analysis
  §3 "add receipt structs in core").
- `deep-diff-forge deploy status [--json]` | `[VBE]` | emits
  `deep-diff-forge.deployment-status.v0` with maturity + gate stack + external
  observers (gap analysis §2/§9 "deploy status --json").
- GitHub Actions CI | `[VBR]` | `.github/workflows/ci.yml` runs the full gate
  ladder + contracts + `deploy status` + a cargo-deny supply-chain job (gap
  analysis §4). Honest caveat: cannot be executed locally (no runner); YAML uses
  only static `run:` commands (no untrusted-input interpolation).
- Supply-chain policy | `[VBR]` | `deny.toml` (licenses/advisories/bans/sources)
  guards the future tree-sitter/TUI/async waves before heavy deps arrive (gap
  analysis §6 / P1, "MSRV + cargo-deny + cargo-audit before parser expansion";
  MSRV is `rust-version = 1.85` in the workspace manifest).
- Gate green | `[VBE]` | `just gate-feature` exit 0; **213 tests passed, 0
  failed**.

## L3 Pipeline (continuation, zero-touch)

`claim | warrant | evidence`

- `deep-diff-forge-pipeline` crate shipped | `[VBR]` | `ChainStage` trait +
  `Pipeline` runner + `PipelineData`/`PipelineError` envelope; `IngestStage`
  (raw patch → model) and `RenderStage` (model → json/jsonl/inline/side-by-side).
- `--jsonl` streaming wired through the REAL runner | `[VBR]` | CLI
  `run_jsonl_pipeline` builds `Pipeline::new().with(IngestStage).with(RenderStage::jsonl())`
  — the pipeline is load-bearing in the binary, not just unit-tested.
- Gate green | `[VBE]` | `just gate-feature` exit 0; **194 tests passed, 0
  failed**; pipeline crate carries **53 tests** (>50 bar).
- Proven live | `[VBE]` | `git diff HEAD | deep-diff-forge --stdin-patch --jsonl`
  emitted one valid `diff.file` JSON event per file with correct +/- counts.
- Strict-Bash safe | `[VBR]` | stages return typed `PipelineError`, never panic;
  malformed input → exit 4 with clean stdout.

The L2 record follows.

---

# Deployment Evidence — L0 → L2 (Patch + Projection Spine)

## L2 Projection (continuation, zero-touch)

`claim | warrant | evidence`

- `deep-diff-forge-projection` crate shipped | `[VBR]` | inline + side-by-side
  renderer-neutral row builders + text renderers; reads the model, never mutates
  patch truth.
- CLI extended | `[VBR]` | `--stdin-patch --layout inline|side-by-side`;
  unknown layout exits 2.
- Gate green | `[VBE]` | `just gate-feature` exit 0; **138 tests passed, 0
  failed** workspace-wide; projection crate carries **53 tests** (>50 bar).
- Proven live | `[VBE]` | both layouts rendered `fixtures/patch/basic.patch`
  with aligned line numbers/gutter; `--layout zigzag` → exit 2.
- Cross-surface consistency fix | `[VBR]` | status labels unified to snake_case
  (`binary_changed`, `type_changed`) across JSON and projection (was
  `{:?}.to_lowercase()` → `binarychanged`).

The L1 record follows.

---

# Deployment Evidence — L0 → L1 Patch Spine

This file is the sealed evidence record for the deployment that took
Deep-Diff-Forge from **L0 Bootstrap** to **L1 Patch** maturity, per the
[Codebase Deployment Framework](docs/DEPLOYMENT_FRAMEWORK.md). Findings lead;
every claim carries a warrant (`[VBR]` verified by reading, `[VBE]` verified by
execution) per [Agentic Rust Coder V4](docs/AGENTIC_RUST_CODER_V4.md).

## What shipped

The keystone the framework specified across eight docs but had never coded: the
canonical patch parser. Patch truth is upstream of every other feature
(Module Structure Plan, design rule 2), so this is the correct L1 wave.

- New crate `deep-diff-forge-patch` (`crates/deep-diff-forge-patch/`):
  - `parse` / `parse_with` — unified + Git-format patch → stable `core` model.
  - `render_unified` — apply-able patch back from the model (model-stable).
  - `to_json` — `deep-diff-forge.review.v0` JSON projection (hand-rolled, zero deps).
  - Typed, non-panicking `PatchParseError`; byte-budget trust-boundary guard.
- CLI contract: `deep-diff-forge --stdin-patch [--json]`
  (exit 3 = input read failure, exit 4 = patch parse failure).
- Fixture corpus at `fixtures/patch/` (basic, new-file, delete, rename, binary,
  no-newline).

## Gate evidence (`just gate-feature`)

`claim | warrant | evidence`

- Format clean | `[VBE]` | `cargo fmt --all --check` exit 0.
- Compile clean | `[VBE]` | `cargo check --workspace` Finished.
- Lint clean | `[VBE]` | `cargo clippy --workspace --all-targets -- -D warnings` exit 0.
- Pedantic clean | `[VBE]` | `cargo clippy ... -W clippy::pedantic` exit 0 (no suppressions).
- Tests pass | `[VBE]` | `cargo test --workspace --locked` — **85 passed, 0 failed**:
  - `deep-diff-forge-patch` lib: 61
  - `deep-diff-forge-patch` integration (`tests/fixtures.rs`): 9
  - `deep-diff-forge-patch` integration (`tests/round_trip.rs`): 7
  - `deep-diff-forge-cli` integration (`tests/stdin_patch.rs`): 8
  - `deep-diff-forge-core`, `deep-diff-forge-cli` unit: 0 (still bootstrap)
- Contract probes pass | `[VBE]` | `--self-test`, `doctor`, and the four
  `*-contract` probes all run with exit 0.
- Whole gate green | `[VBE]` | `just gate-feature` exit 0.

The `deep-diff-forge-patch` crate carries **77 meaningful tests** (61 + 9 + 7),
clearing the Testing Gold Standard's 50-test floor for a production crate.
`core` and `cli` remain explicitly bootstrap (smoke-only) and are not yet
release-eligible.

## Proof on real, imperfect input (not just fixtures)

`claim | warrant | evidence`

- Parses this repo's own live `git diff` | `[VBE]` |
  `git diff HEAD | deep-diff-forge --stdin-patch` reported 3 files changed with
  correct +/- counts, including the new CLI code under deployment.
- Emits valid anchored JSON with correct escaping | `[VBE]` | `--stdin-patch
  --json` produced `review.v0` with per-line `old_line`/`new_line` anchors and
  a backslash-escaped `"../deep-diff-forge-core"` path.
- Exit-code contract holds, stdout stays clean on error | `[VBE]` | a stray
  body line produced `EXIT=4` with the diagnostic on stderr and empty stdout.
- Parses a freshly generated real `git diff` | `[VBE]` | round-trip against a
  throwaway git repo parsed to `modified f.txt (+1 -1, 1 hunks)`.

## Honest-degraded notes

- **No habitat engine seal.** WFE (:8142), LCM (:8200), and the no-mistakes
  membrane are habitat services, not dependencies of this standalone repo (the
  framework's "adopt the pattern, not the dependency" rule). The authoritative
  seal here is the green `gate-feature` plus the repo's own
  `reports/deployments/` receipt — reported as-is, not dressed as an LCM
  convergence receipt.
- **Render is model-stable, not byte-identical.** It does not reconstruct the
  `\ No newline at end of file` marker (the model does not anchor it; emitting
  it in the header would break apply-ability). Round-trip equality is asserted
  for content; the marker limitation is documented and tested explicitly.
- **JSON is hand-rolled.** Zero external dependencies were added (serde is not
  in the offline cache and the workspace is intentionally dependency-free at
  L1). serde is the planned upgrade when the projection crate lands.
- **GitLab mirror remains blocked** (pre-existing; see
  `docs/RELEASE_AND_PUBLICATION.md`). GitHub is the live remote.

## What remains (next waves)

- L1 hardening: parser fuzz target (`fuzz/`), word/region strategies.
- L0 gap-analysis P0s still open: typed `DeploymentReceipt`/`GateResult` in
  `core`, `deep-diff-forge deploy status --json`, GitHub Actions CI.
- L2 Projection: inline/side-by-side/stacked renderers consuming this model.

## Deployment Link

- Framework: [Codebase Deployment Framework](docs/DEPLOYMENT_FRAMEWORK.md)
