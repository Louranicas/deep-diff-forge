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
