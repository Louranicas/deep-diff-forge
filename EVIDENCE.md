# Deployment Evidence — L0 → L9 (… + Daemon + Release + Learning)

## Live feature & capacity verification (S1008452)

`claim | warrant | evidence` — the release binary exercised end-to-end (real
process, exit codes + output asserted; not mocked). **0 product bugs found.** Full
report: `reports/LIVE_TEST_EVIDENCE_S1008452.md`.

- Quality gate | `[VBE]` | `check → clippy -D warnings → pedantic → test` clean;
  **794 tests / 0 failed**; fuzz harness compiles (`cargo check --manifest-path
  fuzz/Cargo.toml --bins` rc 0).
- CLI features + edge cases | `[VBE]` | **48/48** — meta (`--version`/`--help`/
  `--self-test`/`doctor`/`loom-contract`), `deploy status|release [--json]`, patch
  projections (`--json`/`--jsonl`/`--rank`/`--cluster`/`--layout {inline,side-by-side}`),
  `semantic`/`structural` (reformat-aware `"reformat_only": true`)/`highlight`,
  `review --probe`, `learn status [--json]`; bad layout → rc 2.
- L6 determinism | `[VBE]` | 5-file patch: `--parallel serial`==`auto`==`8`
  byte-identical ranked results; `--parallel 8` used 5 workers (clamped to files).
- Error/security contracts | `[VBE]` | malformed/truncated/bad-header patch → rc 4;
  empty → rc 0 `files:[]`; Trojan-Source `U+202E` + terminal `ESC` neutralized in
  body (inline) and path (summary); **C1/DEL escaped in `--json`/`--jsonl`**
  (S1008452 fix, fail-before/pass-after); `… | head` → rc 0 (no SIGPIPE panic).
- Daemon (UDS JSON-RPC) | `[VBE]` | live start, `0600` socket; CLI `health`/`status`;
  **9/9** RPC incl. `diff.plan`, `session.open→snapshot→close`, and wire error codes
  (`-32602`/`-32601`/`4`/`1`); CLI `stop` → exit 0, socket cleaned up. Hostile soak
  (`daemon_soak.py`) PASS.
- Privacy & SBOM | `[VBE]` | `privacy_probe.py` PASS (no source/path leak);
  regenerated SBOM content-identical to committed (100 pkgs / 185 rels; timestamp
  only).
- Honest-degraded | live fuzzing not run (cargo-fuzz absent; harness compiles +
  794 tests + hostile inputs + soak cover robustness); 64 MiB over-budget rejection
  is unit-tested, not re-exercised live.

## Supply-chain & fuzz hardening (S1008443 cont. → main `0d41ef2`)

`claim | warrant | evidence`

- CI/release supply-chain hardened | `[VBR]` | every GitHub Action pinned to a
  commit SHA (CI + release); release emits SLSA build-provenance attestations for
  the binary + sha256 + SBOM; SPDX SBOM (`sbom.spdx.json`) generated from
  `Cargo.lock`/`cargo metadata`, CI-gated and release-uploaded.
- Fuzz harness landed | `[VBR]`/`[VBE]` | `fuzz/` (cargo-fuzz) — `patch_parser`,
  `review_json`, `daemon_protocol`, `agent_annotation`; excluded from the
  publishable workspace (`Cargo.toml exclude = ["fuzz"]`); `cargo check
  --manifest-path fuzz/Cargo.toml --bins` is a CI gate (verified exit 0).
- Security probes wired | `[VBR]` | `scripts/security/{daemon_soak,privacy_probe,
  generate_spdx_sbom}.py` + `mutation_gate.sh`, exposed as `just security-*`.
- Regression tests added | `[VBE]` | JSON-RPC wire error-code guard
  (`protocol.rs`) + empty-input `--json` schema-snapshot / `--jsonl` zero-read
  (`cli` `security.rs`); all three live-confirmed passing.
- Doc accuracy | `[VBR]` | daemon `lib.rs` corrected from "thread-per-connection"
  to the true single-threaded sequential accept loop (matches `serve.rs`).
- Gate green | `[VBE]` | `check → clippy -D warnings → pedantic → test` clean;
  workspace **792 tests, 0 failed**; fuzz harness compiles. Production crate code
  unchanged from `9195c24`. Committed `0d41ef2`; `main` synced to both remotes.

## Zen bias-controlled re-review remediation (S1008443)

`claim | warrant | evidence` — 3 findings from Zen's re-review of `920ca59`, fixed
by FORGE builders (optimizer) then verified by three judges outside the build loop.

- F1 closing-`@@` enforced | `[VBE]` | `parse_hunk_header` requires the closer
  token; `@@ -1,1 +1,1 NOT_A_HUNK` → CLI exit 4 (was 0); valid forms (default
  counts, trailing section text) still accepted. Parser tests `hunk_header_*`.
- F2 explicit-socket fail-closed | `[VBE]` | `bind_explicit` — 0755 parent → daemon
  exit 6, dir stays 755 (was chmodded to 700); regular file / symlink at path →
  refused, victim intact; absent parent → created `0700`. Tests `explicit_bind_*`,
  `socket_location_at_*`, `resolve_explicit_*`.
- F3 bounded LRU sessions | `[VBR]` | `MAX_SESSIONS=64`, evict-before-insert; true
  min-tick evicted; reads refresh recency. Tests `sessions_are_bounded`,
  `session_count_never_exceeds_cap`, `lru_evicts_the_true_least_recently_used`.
- Independent judges | `[VBR]` | forge-security-architect PASS (no Critical/High
  regression, no new fail-open, no `unsafe`/silent-swallow); forge-tester PASS
  (genuine fail-before/pass-after on the core fixes); agent-claim-verifier VERIFIED
  CLEAN 4/4 (conf 0.97).
- Gate green | `[VBE]` | `just gate-release` exit 0 (fmt → check → clippy →
  pedantic → test → contracts → docs); workspace **789 tests, 0 failed** (765 + 24).
  Diff = `parser.rs`, `daemon/serve.rs`, `daemon/handler.rs` only.

## L9 Learning (local-only loop; v0.2.0 crates.io-publishable cut)

`claim | warrant | evidence`

- Learning loop code | `[VBR]` | `deep-diff-forge-learning` (9 modules):
  `receipt` (`StrategyReceipt` — hashes/counts/timings only, no path/source),
  `store` (XDG local-only JSONL, fail-soft, pure `resolve_learning_dir`),
  `score` (per-strategy scores + `TrustPolicy::earns_trust`), `planner`/`ranking`/
  `annotation` (the three spec learning units), `promote` (explainable gate),
  `error`, `util` (FNV-1a redaction). `#![forbid(unsafe_code)]`.
- Privacy by construction | `[VBR]` | `StrategyReceipt` has no path/source field;
  `receipt::tests::json_never_contains_a_path`; `redacted_id` is non-reversible.
- Safety invariants | `[VBR]` | annotation source stays `Untrusted` without
  grounded wins (`annotation::tests::ungrounded_acceptances_never_build_trust`);
  promotion blocks on any failed rule and lists every reason
  (`promote::tests::all_reasons_accumulated_not_short_circuited`).
- CLI surface | `[VBE]` | `learn status [--json]` (`deep-diff-forge.learning.v0`)
  + `learn record --stdin` round-trip live-proven; `deploy status` → `L9
  (Learning)`.
- Broken-pipe fix | `[VBE]` | bulk output routed through `emit`/`emitln!`;
  `… --stdin-patch --json | head -1; echo $?` → **0** (was 101). L1-era open
  finding resolved, std-only, no `unsafe`.
- Publish-readiness | `[VBE]` | `cargo publish --dry-run -p deep-diff-forge-core`
  and `-p deep-diff-forge-learning` both package + verify + reach "Uploading …
  aborting upload due to dry run"; manifests inherit version + internal deps via
  `[workspace.package]` + `[workspace.dependencies]`; `core`/`cli` gained the
  mandatory `description`.
- Gate green | `[VBE]` | `check → clippy -D warnings → pedantic → test → docs`
  clean; **765 tests passed, 0 failed**; `cargo fmt --check` exit 0; `cargo deny
  check` + strict `cargo audit` clean.
- Security hardened (S1008412) | `[VBE]` | 8-dimension STRIDE review + independent
  verify → 17 confirmed findings (4 MED / 12 LOW / 1 INFO; **0 Critical/High**),
  all remediated with fail-before/pass-after tests: terminal-escape injection
  (`core::display_safe` wired at every human renderer; 5 e2e regression tests),
  read-cap DoS guards, daemon panic-isolation + read-timeout + size-bound +
  symlink-reject + no-`/tmp`-fallback, owner-private learning store, fail-closed
  agent trust, workspace-`forbid(unsafe)`, pinned tree-sitter, CI+release
  cargo-audit gate, `SECURITY.md`. Detail: `reports/security/` (+ register JSON).

The one genuine wall is unchanged: **crates.io upload** needs a
`CARGO_REGISTRY_TOKEN` (an irreversible, yank-only act I cannot self-authorize).
The workspace is now dry-run-verified publish-ready; the release workflow publishes
all 12 crates in dependency order once the token is configured. Until then the
target is reported `blocked`, never faked.

## L8 Release (tagged release; crates.io token-gated)

`claim | warrant | evidence`

- Release-automation code | `[VBR]` | `core::release` (`TargetState`,
  `ReleaseTarget`, `ReleasePlan` — independent per-target states) + CLI
  `deploy release [--json]` (`deep-diff-forge.release.v0`).
- Release infrastructure | `[VBR]` | `release.yml` (tag-triggered build matrix →
  binary + sha256 + GitHub Release upload; token-gated crates.io publish job in
  dependency order), `CHANGELOG.md`, dual `LICENSE-MIT` + `LICENSE-APACHE`.
- Tagged release cut | `[VBE]` | `v0.1.0` tagged and pushed to both remotes;
  GitHub Release created with the linux binary + checksum.
- Honest per-target posture | `[VBE]` | `deploy release --json` →
  github/gitlab/github-release `published`, **crates.io `blocked`**,
  `fully_published: false`, `pending: ["crates.io"]`.
- Gate green | `[VBE]` | `just gate-feature` exit 0; **568 tests passed, 0
  failed**; cargo-deny clean. `deploy status` → `L8 (Release)`.

The one genuine wall: **crates.io publication** needs a `CARGO_REGISTRY_TOKEN`
(no token present — `cargo publish` is an irreversible, yank-only act I cannot
self-authorize). The release workflow performs it automatically once the token
is configured as a repository secret; until then the target is reported
`blocked`, never faked.

## L7 Daemon (continuation, std-first UDS)

`claim | warrant | evidence`

- `deep-diff-forge-daemon` crate shipped | `[VBR]` | UDS JSON-RPC 2.0 server,
  std-first (`std::os::unix::net`, thread-free single-accept loop, no `tokio`)
  with `serde_json` for framing. Methods: `engine.initialize`, `daemon.health`,
  `daemon.status`, `daemon.shutdown`, `diff.plan`, `session.open|snapshot|close`.
- Socket security | `[VBR]`/`[VBE]` | runtime dir created `0700` + validated
  (rejects group/world-accessible dirs), socket `0600`, stale sockets replaced;
  asserted by `bind_secure` tests reading back the real modes.
- Real socket round-trip tested, not mocked | `[VBE]` | `UnixStream::pair`
  covers `handle_connection`; a live `run_server` thread + `request` client
  cover the full accept loop incl. `diff.plan` over the socket and graceful
  shutdown. Only the long-running CLI `daemon start` invocation is the boundary.
- CLI: `daemon {path|start [--foreground]|health|status|stop} [--socket]` |
  `[VBE]` | live lifecycle: started → socket appeared → `daemon health` returned
  `{"status":"ok","pid":…,"protocol":0}` → `daemon stop` → `{"stopping":true}`
  → server exited rc=0.
- Gate green | `[VBE]` | `just gate-feature` exit 0; **553 tests passed, 0
  failed**; daemon crate carries **57 tests** (>50). cargo-deny clean
  (serde/serde_json maintained — no new advisory).
- `deploy status` reports `L7 (Daemon)` | `[VBE]`.

With L7, **all engine layers L0-L7 are implemented.** The remaining rungs are
not engineering gaps: L8 Release is credential-gated (crates.io / GitHub
Releases / GitLab — irreversible outward acts), and L9 Learning needs runtime
telemetry from a deployed daemon.

Honest scope: the daemon serves connections sequentially (one accept loop); a
thread-per-connection or `tokio` multi-client upgrade is deferred until a
measured need (the framework's "std first, async only with benefit" rule).
uid-ownership verification is approximated by the `0700` mode check (strict
getuid needs `libc`); documented, not silently skipped.

## L6 Cluster (continuation, near-zero-dep)

`claim | warrant | evidence`

- `deep-diff-forge-cluster` crate shipped | `[VBR]` | bounded parallel scheduler
  on std `thread::scope` (no external dep, no `unsafe`): deterministic contiguous
  sharding, `run_lane`, `run_risk_cluster`, `apply_join` over `JoinPolicy`, and a
  structured `ClusterReceipt`.
- **Determinism under parallelism** (the defining L6 guarantee) | `[VBE]` |
  `--cluster --parallel serial` and `--parallel 8` produced **identical** ranked
  JSON; in-crate tests assert serial == Fixed(n) == Auto for 40–200 files.
- CLI: `--stdin-patch --cluster [--parallel serial|auto|N] [--json]`
  (`deep-diff-forge.cluster.v0` with receipt) | `[VBE]` | live human + JSON.
- Gate green | `[VBE]` | `just gate-feature` exit 0; **491 tests passed, 0
  failed**; cluster crate carries **52 tests** (>50). cargo-deny clean.
- `deploy status` reports `L6 (Cluster)` | `[VBE]`.

Honest scope: lanes run the patch+risk dimensions today (the layers with data
from a bare patch); semantic/history lanes join in once the Git-input wave
feeds file bytes. Replay manifests are noted as future; the receipt ships now.

## L5 Review (three sub-waves)

`claim | warrant | evidence`

- **L5a graph** (`cb017be`) | `[VBR]`/`[VBE]` | `deep-diff-forge-graph` —
  deterministic, explainable risk ranking from patch facts; `--stdin-patch
  --rank [--json]` (`deep-diff-forge.rank.v0`). 54 tests. Live: ranked this
  repo's own diff, `core/src/lib.rs` (public API) first.
- **L5b agent** (`5f85a47`) | `[VBR]` | `deep-diff-forge-agent` — grounding
  classification (evidence necessary; no-evidence ⇒ Ungrounded), source
  inference, body sanitization, anchor validation, reviewer-owned resolution
  (never auto-resolves). 50 tests. Library for the TUI/daemon.
- **L5c TUI** | `[VBE]` | `deep-diff-forge-tui` (ratatui 0.29 + crossterm 0.28)
  — pure tested state model + crossterm key-mapping + ratatui render exercised
  headlessly via `TestBackend`; `review [--probe]`. 50 tests. Live: `review
  --probe` rendered a full ranked sidebar + detail frame.
- Gate green | `[VBE]` | `just gate-feature` exit 0; **437 tests passed, 0
  failed**; each L5 crate ≥50.
- Supply-chain gate caught a real transitive advisory | `[VBE]` | ratatui pulls
  `paste 1.0.15` (unmaintained, RUSTSEC-2024-0436, not a vulnerability); handled
  with a documented scoped `ignore` in `deny.toml`; `cargo deny check` →
  advisories/bans/licenses/sources ok.
- `deploy status` reports `L5 (Review)` | `[VBE]`.

### Known limitation (honest-degraded)

- **SIGPIPE / broken-pipe:** the CLI uses `println!`, which panics (exit 101)
  if a downstream consumer closes the pipe early (e.g. `… | head`). This is
  pre-existing CLI-wide (since L1), not L5-specific, and does not corrupt or
  truncate emitted output. A dedicated hardening pass should reset the SIGPIPE
  disposition or route bulk output through a broken-pipe-tolerant writer; it is
  tracked rather than rushed mid-wave. The interactive `review` TUI is
  unaffected (it owns the terminal).
- The live interactive `review` loop (`run`) needs a real TTY and is the single
  untested boundary; all logic lives in the tested state model + `--probe`.

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

## Security + Quality Hardening (S1009000 — bug hunt seal)

Systematic bug-hunt loop: false-positive verifier → fix → verify → repeat. Gate: **930 tests / 0 failed** (was 927 — 3 new regression tests added). Full `check → clippy -D warnings → pedantic → test` green.

| Finding | Severity | Fix | Regression test |
|---|---|---|---|
| FINDING-001: TUI note-box title escape injection | HIGH | `display_safe()` wraps anchor path at `diffview.rs:470` | `note_box_title_escapes_malicious_anchor_path` |
| FINDING-002: Recursive `count_errors` stack overflow | MEDIUM | Iterative `TreeCursor` DFS at `analyze.rs:110` | `count_errors_iterative_does_not_overflow_on_deeply_nested_source` |
| OPEN-006: No clippy restriction lints | LOW | `[workspace.lints.clippy]` `unwrap_in_result`+`panic_in_result_fn` = deny | Lint enforcement (+ fixed `handler.rs:236` `.expect()` → `?`) |
| OPEN-007: SBOM gate regenerates-not-verifies | LOW | `ci.yml` now diffs regen vs `git show HEAD:sbom.spdx.json`; JSON timestamp strip | Verified end-to-end in-session |
| OPEN-001 (FALSE POSITIVE): DEL/C1 in `--json`/`--jsonl` | — | Already closed in v0.2.0 (S1008452); source-confirmed FP | — |

Verifier result (agent-claim-verifier): FIX-1/2/3 **CONFIRMED** at source + execution. FIX-4 initially PARTIAL (tag-value regex against JSON payload — fixed in-session); final regex `"created":\s*"[^"]*"` verified correct.

## Security + Quality Hardening Round 2 (S1009000 — second-pass bug hunt)

Five additional fixes after the adversarial security audit and OPEN-003/OPEN-002 explorer findings. Gate: **933 tests / 0 failed** (was 932). Full `check → clippy -D warnings → pedantic → test` green.

| Finding | Severity | Fix | Regression test |
|---|---|---|---|
| OPEN-002: Same-UID daemon DoS (head-of-line blocking) | MEDIUM | Thread-per-connection: `serve.rs::run_server` uses `Arc<Mutex<Engine>>` + `std::thread::spawn` per connection; shutdown self-connect wakeup | Existing connection tests pass; behaviour verified |
| OPEN-003: Spaced-path tokenization | LOW | `parser.rs::start_git_file` uses `rfind(" b/")` instead of `split_whitespace()` | `spaced_path_git_header_is_parsed_correctly` + `spaced_path_multiple_spaces_in_filename` |
| FINDING-2: tokenize/highlight bypass byte budget | LOW | `structural.rs::tokenize` + `highlight.rs::highlight_rust` both guard on `DEFAULT_BYTE_BUDGET` before parse | Byte-budget enforcement tested via existing analyze tests |
| FINDING-4: TUI terminal not restored on panic | LOW | `run.rs` adds `TerminalGuard { armed }` with Drop; disarmed on normal exit, fires on panic unwind | No TTY in test environment; design verified by inspection |
| FINDING-5: `--parallel 65535` spawns runaway threads | LOW | `scheduler.rs::resolve_workers` caps `Fixed(n)` at `cpu_count × 4` | `fixed_parallelism_capped_at_cpu_multiple` |

Verifier result: agent-claim-verifier **5/5 CONFIRMED** at source + Fix B tests executed green.

<!-- CODEX_PI_HARNESS_S1008820_BACKLINK_START -->

## Codex Pi Harness backlink (S1008820)

This document is linked into the Codex Pi Harness v5 plan.

- Plan: `ai_docs/CODEX_PI_HARNESS_PLAN_S1008820.md`
- Workspace plan: `meta-plans/PLAN_codex_pi_harness_S1008820.md`
- BIDI map: `ai_docs/CODEX_PI_HARNESS_BIDI_LINK_MAP_S1008820.md`
- Status: plan/gate only; build not armed.

<!-- CODEX_PI_HARNESS_S1008820_BACKLINK_END -->
