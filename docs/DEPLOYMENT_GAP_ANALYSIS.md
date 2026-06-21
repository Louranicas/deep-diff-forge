# Deployment Gap Analysis

This analysis reviews the current Deep-Diff-Forge deployment framework against
the repository as it exists now. It has two passes:

1. A codebase-centered deployment gap analysis.
2. A non-anthropocentric gap analysis that treats machines, ecosystems,
   supply chains, energy, failure modes, and non-human consumers as first-class
   deployment stakeholders.

The codebase remains the primary source of truth.

## Current State

Repository maturity:

```text
deployment maturity: L0 Bootstrap
implemented crates: deep-diff-forge-core, deep-diff-forge-cli
implemented behavior: core vocabulary, CLI smoke commands, contract probes
planned behavior: patch parser, projection, pipeline, git, syntax, graph,
  agent, TUI, cluster, loom, daemon, release automation
```

Validated today:

- Rust workspace compiles.
- Formatting passes.
- Bootstrap contract probes run.
- Documentation graph is connected through the deployment framework.

Not yet implemented:

- patch parser
- fixture corpus
- JSON/JSONL schemas in code
- pipeline stages
- release packaging
- daemon runtime
- CI workflows
- deployment receipt writer

## Resolution Status (L3)

Several P0/P1 gaps are now closed by the L1–L3 deployment waves:

- **P0 receipt schema in code** — RESOLVED: `MaturityLevel`/`GateState`/
  `GateResult`/`DeploymentStatus` in `deep-diff-forge-core::deploy`.
- **P0 `deploy status` command** — RESOLVED: `deep-diff-forge deploy status
  [--json]` emits `deep-diff-forge.deployment-status.v0`.
- **P0 CI workflow** — RESOLVED: `.github/workflows/ci.yml` (gate ladder +
  contracts + `deploy status` + cargo-deny).
- **P0 fixture gate / real tests** — RESOLVED: `fixtures/patch/` + 213
  workspace tests; patch/projection/pipeline crates each exceed the 50-test bar.
- **P1 supply-chain policy** — PARTIAL: `deny.toml` + MSRV `1.85` landed;
  `cargo audit`/`cargo vet` deferred to the dependency-introduction (L4+) wave.

The remaining gaps below stay open pending the heavy-dependency waves
(tree-sitter/TUI/async) and outward release acts (crates.io / GitHub Releases),
which are deliberately gated.

## Executive Gaps

| Priority | Gap | Impact | Recommendation |
| --- | --- | --- | --- |
| P0 | Deployment framework is mostly manual. | Receipts and gates depend on operator discipline. | Add `deep-diff-forge deploy doctor` and `deep-diff-forge deploy receipt` commands. |
| P0 | No executable fixture gate exists. | Framework cannot prove patch truth or projection stability. | Implement `deep-diff-forge-patch` plus first patch fixtures. |
| P0 | Current bootstrap crates have no real tests. | The codebase cannot yet demonstrate the required 50 meaningful tests per production module. | Treat current crates as L0 bootstrap and block production graduation until `TESTING_GOLD_STANDARD.md` is met. |
| P0 | No CI workflow exists. | GitHub push is not independently verified. | Add `ci.yml` with fmt, check, clippy, tests, and contract probes. |
| P0 | No machine-readable deployment schema exists in code. | Habitat/factory and CI cannot consume receipts reliably. | Add receipt structs in `core` and JSON output in CLI. |
| P1 | No release artifact automation. | Release path is documented but not reproducible. | Add release manifest and build script after patch/projection spine lands. |
| P1 | No supply-chain policy beyond docs. | Dependency growth could become opaque. | Add MSRV, audit, deny, license, and dependency-review gates. |
| P1 | Zellij/habitat integration is observational only. | External orchestration cannot assert readiness. | Define a stable `deployment.status.v0` JSON shape. |
| P1 | Daemon security is planned but untested. | Future UDS service could ship unsafe permissions. | Add socket permission test fixtures before daemon implementation. |
| P2 | No benchmark receipt format. | Performance claims cannot be tracked. | Add benchmark schema and initial no-op baseline. |
| P2 | No docs freshness check. | Bidirectional doc graph can drift. | Add a docs-link check command or script. |

Current mitigation:

- The repo-local `justfile` now provides executable `status`, `gate-docs`,
  `gate-bootstrap`, `gate-feature`, `contracts`, `doctor`, and
  `receipt-bootstrap` recipes.
- This reduces manual deployment drift at L0, but it does not replace the
  longer-term recommendation to add product-native `deep-diff-forge deploy`
  commands and typed receipt schemas.

## Codebase Deployment Gap Analysis

### 1. Source Of Truth Gap

The framework correctly states that Rust code and tests outrank docs. The
current repo, however, is still dominated by docs. That is acceptable for L0,
but it creates a short-term truth gap: planned contracts outnumber executable
contracts.

Risk:

- Future contributors may treat planned behavior as already implemented.
- Deployment maturity can be overstated.
- External orchestrators may call commands that are only future plans.

Recommendation:

- Add an explicit `implemented`, `planned`, and `blocked` table to every
  user-facing command family.
- Keep bootstrap contract commands, but have each print `status=planned` for
  unimplemented surfaces.
- Add a generated `reports/current-capabilities.json` once JSON output exists.

### 2. Gate Automation Gap

The framework defines gates, but no single command runs the gate stack and
writes a receipt.

Risk:

- Operators may skip gates.
- CI and habitat services cannot compare receipt fields.
- Validation output remains scattered across terminal logs.

Recommendation:

Add a deployment crate or CLI module:

```text
deep-diff-forge deploy doctor
deep-diff-forge deploy gate --mode docs
deep-diff-forge deploy gate --mode bootstrap
deep-diff-forge deploy receipt --out reports/deployments/<ts>
deep-diff-forge deploy status --json
```

Initial implementation can be a thin orchestrator around existing commands.

### 3. Receipt Schema Gap

The deployment framework includes a sample `summary.json`, but the schema does
not exist in Rust.

Risk:

- Receipt fields drift across docs, CI, habitat, and manual runs.
- JSON consumers cannot rely on stable names.
- Deployment history becomes hard to compare.

Recommendation:

- Add `DeploymentReceipt`, `GateStatus`, `GateResult`, and `ExternalObservation`
  to `deep-diff-forge-core`.
- Add a `schema` string to every receipt.
- Emit `not-run`, `pass`, `warn`, `fail`, and `blocked` states explicitly.
- Keep raw command logs beside structured summaries.

### 4. CI Gap

No GitHub Actions workflow exists yet.

Risk:

- GitHub `main` can accept commits that only passed locally.
- Release confidence depends on the current workstation.
- GitLab mirror readiness remains invisible.

Recommendation:

Add `.github/workflows/ci.yml`:

```yaml
name: ci
on:
  pull_request:
  push:
    branches: [main]
jobs:
  rust:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo fmt --all --check
      - run: CARGO_TARGET_DIR=target cargo check --workspace --locked
      - run: CARGO_TARGET_DIR=target cargo clippy --workspace --all-targets -- -D warnings
      - run: CARGO_TARGET_DIR=target cargo test --workspace --locked
      - run: CARGO_TARGET_DIR=target cargo run -p deep-diff-forge-cli -- --self-test
      - run: CARGO_TARGET_DIR=target cargo run -p deep-diff-forge-cli -- doctor
      - run: CARGO_TARGET_DIR=target cargo run -p deep-diff-forge-cli -- claude-code-contract
      - run: CARGO_TARGET_DIR=target cargo run -p deep-diff-forge-cli -- chain-contract
      - run: CARGO_TARGET_DIR=target cargo run -p deep-diff-forge-cli -- cluster-contract
      - run: CARGO_TARGET_DIR=target cargo run -p deep-diff-forge-cli -- loom-contract
```

### 5. Fixture Gap

The framework lists fixture classes, but the repository does not yet contain a
`fixtures/` tree.

Risk:

- Patch truth cannot be regression-tested.
- Projection output cannot be compared.
- Semantic fallback behavior remains theoretical.

Recommendation:

Create:

```text
fixtures/
  patch/basic.patch
  patch/rename.patch
  patch/no-newline.patch
  patch/binary.patch
  expected/review-basic.json
```

Tie the first L1 milestone to parser round-trip tests before adding syntax or
TUI behavior.

### 6. Supply-Chain Gap

The framework references release quality but does not define dependency policy.
Difftastic demonstrates the importance of MSRV, conservative upgrades, parser
dependency posture, and package metadata.

Risk:

- Heavy tree-sitter parser dependencies may arrive without policy.
- License compatibility may be assumed instead of checked.
- Transitive dependency vulnerabilities can reach release artifacts.

Recommendation:

- Add a documented MSRV before first public crate release.
- Add `cargo deny` for licenses, advisories, bans, and duplicate versions.
- Add `cargo audit` or equivalent security check in CI.
- Add parser dependency groups behind features.
- Add `cargo vet` later if the dependency graph becomes broad.

### 7. Packaging Gap

Release packaging is planned but not implemented.

Risk:

- Users cannot reproduce install instructions.
- Binary names, archive names, and checksums can drift.
- crates.io packaging may include or omit unintended files.

Recommendation:

- Add package metadata to every crate as it becomes public.
- Add `dist/` artifact naming policy to release automation.
- Add checksum generation before GitHub Releases.
- Delay crates.io publishing until the patch parser and CLI JSON output exist.

### 8. Runtime Gap

Daemon behavior is specified, but no daemon exists.

Risk:

- Habitat service row could create premature expectations.
- Socket security is untested.
- Runtime smoke cannot run.

Recommendation:

- Keep daemon out of required gates until the daemon crate exists.
- Add a `daemon planned` status in contract output.
- Implement socket path validation as a pure function before binding sockets.
- Add tests for Linux fallback, macOS temp path, and Windows named-pipe naming.

### 9. Zellij/Habitat Integration Gap

The framework references Zellij and habitat services but does not define a
machine-readable status contract for those external observers.

Risk:

- Panels and service probes scrape prose.
- External service degradation may be confused with Deep-Diff-Forge failure.
- Gate-only and deploy-dev modes cannot distinguish internal vs external risk.

Recommendation:

Define:

```json
{
  "schema": "deep-diff-forge.deployment-status.v0",
  "repo": "deep-diff-forge",
  "maturity": "L0",
  "commit": "unknown",
  "gates": {},
  "external_observers": {
    "zellij": "observed",
    "habitat": "optional"
  }
}
```

This should be emitted by `deep-diff-forge deploy status --json`.

### 10. Rollback Gap

Rollback rules are described, but rollback cannot be tested until release
artifacts and daemon state exist.

Risk:

- First production rollback will exercise untested logic.
- Cache compatibility issues may be discovered too late.

Recommendation:

- Add rollback fixture receipts before release.
- Make cache schema version part of every future cache key.
- Add `deep-diff-forge cache status --json` before cache mutation commands.

## Non-Anthropocentric Gap Analysis

This pass evaluates deployment from perspectives that are not centered on a
human reviewer or operator. The stakeholders are machines, automated agents,
build systems, filesystems, CPUs, memory, networks, energy budgets, package
ecosystems, and future maintainers who may never interact with the original
authors.

### 1. Machine Consumer Gap

The framework is readable by humans, but machines need schemas, stable fields,
and predictable state transitions.

Non-human risk:

- CI systems cannot infer nuanced Markdown.
- Habitat panels may misclassify states if they scrape strings.
- Future agents may hallucinate state from prose.

Recommendation:

- Define JSON schemas for deployment status, deployment receipt, contract
  result, gate result, benchmark result, and daemon health.
- Keep Markdown as explanation, not the machine interface.
- Add schema version fields everywhere.

### 2. Filesystem And Storage Gap

The framework assumes local filesystem behavior but does not model disk
pressure, inode pressure, readonly home caches, or 10TB corpus absence as
first-class states.

Observed context:

- The local environment has read-only home-cache edge cases.
- The 10TB disk is optional.
- `target/` must stay repo-local.

Non-human risk:

- Build cache writes fail in global cache locations.
- Corpus jobs consume excessive disk space.
- Receipt directories grow without retention policy.

Recommendation:

- Add disk preflight checks for target, cache, state, and receipt paths.
- Add retention policy for reports and corpus outputs.
- Add explicit `readonly`, `low-space`, `missing`, and `permission-denied`
  states in deployment receipts.

### 3. CPU And Energy Gap

Clustered execution is planned, but energy and thermal cost are not modeled.

Non-human risk:

- Parallel syntax analysis can saturate CPUs unnecessarily.
- CI runners may throttle or fail under aggressive defaults.
- Laptop workflows may waste battery on deep analysis when fast patch truth
  would be enough.

Recommendation:

- Add budget profiles that include CPU and energy posture:
  `fast`, `balanced`, `deep`, `corpus`, `battery`, `ci`.
- Default cluster parallelism should be conservative until benchmarks exist.
- Receipts should record elapsed time, worker count, and peak memory.

### 4. Memory Pressure Gap

The framework says large data should stream, but there is no measurable memory
budget yet.

Non-human risk:

- Large diffs or syntax trees can exhaust memory.
- Daemon caches can grow without pressure handling.
- JSON output can accidentally buffer whole repositories.

Recommendation:

- Add memory budgets to planner and cluster receipts.
- Require streaming APIs for JSONL and projections.
- Add cache high-water marks and eviction receipts.
- Add synthetic large-file fixtures early.

### 5. Network And Mirror Gap

GitHub push works; GitLab is configured but blocked. The framework treats this
correctly as conditional, but network dependency states need clearer modeling.

Non-human risk:

- Release automation may mark partial publication as success.
- Mirror drift may remain invisible.
- crates.io publish may occur when binary release failed.

Recommendation:

- Model publication target states independently:
  `not-configured`, `blocked`, `skipped`, `published`, `failed`.
- Require release receipts to list every intended target.
- Do not publish crates until binary smoke and checksums pass.

### 6. Package Ecosystem Gap

Deployment should respect downstream package managers and distro builders, not
only direct users.

Non-human risk:

- Missing MSRV hurts distro packaging.
- Build scripts may require network or vendored parser surprises.
- Feature flags may not produce reproducible dependency sets.

Recommendation:

- Set MSRV before public crate publication.
- Keep build scripts deterministic and network-free.
- Document default features and minimal features.
- Add `cargo package --list` checks.

### 7. Time And Clock Gap

Receipts use timestamps, but clock skew and reproducibility are not modeled.

Non-human risk:

- Distributed CI or habitat services may produce misleading ordering.
- Replay systems may sort receipts incorrectly.

Recommendation:

- Include monotonic run IDs or sequence IDs alongside wall-clock timestamps.
- Use UTC timestamps.
- Record host and CI provider identifiers where available.
- Avoid relying on local timezone for deployment logic.

### 8. Failure Ecology Gap

The framework has blocking rules, but it does not yet classify cascading
failures between parser, projection, daemon, cache, habitat, and release.

Non-human risk:

- One subsystem failure can hide another.
- Recovery may mutate state in the wrong order.

Recommendation:

- Add failure domains:
  `input`, `patch`, `semantic`, `projection`, `pipeline`, `cluster`,
  `daemon`, `cache`, `release`, `external-observer`.
- Receipts should record domain and blast radius.
- Rollback should be domain-specific.

### 9. Ecosystem And License Gap

The project draws from exemplars and planned parser ecosystems, but adoption
boundaries need machine-readable capture.

Non-human risk:

- License obligations are missed.
- Exemplar-derived behavior is not traceable.
- Generated fixtures may embed source with unclear rights.

Recommendation:

- Add loom receipt fields for license notes and adoption boundaries.
- Add fixture provenance metadata.
- Add dependency license gate before parser expansion.

### 10. Accessibility For Non-Human Tools

The framework uses Mermaid and Markdown tables, which are useful to humans but
not sufficient for tools.

Non-human risk:

- Search, CI, and agents cannot reliably navigate diagrams.
- Tables are hard to diff semantically.

Recommendation:

- Mirror important tables as future TOML/JSON manifests:
  `.deep-diff-forge/deployment.toml`,
  `.deep-diff-forge/gates.toml`,
  `.deep-diff-forge/docs-map.toml`.
- Generate docs from manifests once contracts settle.

## Recommended Implementation Plan

### Immediate: L0 Hardening

1. Add `DeploymentReceipt` and `GateResult` types to `deep-diff-forge-core`.
2. Add `deep-diff-forge deploy status --json`.
3. Add GitHub Actions CI for fmt, check, clippy, tests, and contract probes.
4. Add docs-link check for the Markdown graph.
5. Add `reports/` and `.deep-diff-forge/` path policy to `.gitignore` or docs.

### Next: L1 Patch Gate

1. Add `deep-diff-forge-patch`.
2. Add first patch fixtures.
3. Add parser round-trip tests.
4. Add `--stdin-patch --json`.
5. Make patch truth the first executable deployment gate beyond smoke commands.

### Then: Machine-Readable Deployment

1. Add JSON schemas for deployment status and receipts.
2. Add deployment receipt writer.
3. Add habitat/factory observer status output.
4. Add disk, CPU, memory, and network preflight fields.
5. Add benchmark and fixture receipts.

### Later: Release And Runtime

1. Add release packaging automation.
2. Add daemon socket path validator and tests.
3. Add cache schema versioning.
4. Add release target matrix and mirror state tracking.
5. Add rollback receipt fixtures.

## Recommended Policy Updates

Add these policies to the deployment framework:

- **Machine interfaces outrank prose for automation.**
- **External observer health is advisory unless explicitly required.**
- **Disk, memory, CPU, and network are deployment stakeholders.**
- **Every planned command must declare implemented vs planned status.**
- **Every release target has an independent state.**
- **Every large corpus run records resource consumption.**
- **Every exemplar-derived feature records provenance and license notes.**

## Deployment Link

- Framework: [Codebase Deployment Framework](DEPLOYMENT_FRAMEWORK.md)
