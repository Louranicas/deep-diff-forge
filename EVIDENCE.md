# Deployment Evidence — L0 → L9 (… + Daemon + Release + Learning)

## Status 2026-10-31 (0.5.0)

**Kind:** append-only evidence log. Each section is dated or session-tagged and records what was measured then. Sections are not rewritten. Later sections correct earlier ones.

**Current at 0.5.0:**

- The newest evidence is at the end of this file: [v0.5.0 release readiness (2026-10-31)](#v050-release-readiness-2026-10-31); before it, [v0.4.0 release readiness (2026-10-30)](#v040-release-readiness-2026-10-30), then [daemon metrics overhead](#daemon-metrics-overhead-2026-10-30) and [findings verification by address](#findings-verification-by-address-2026-10-30). The fuzz numbers in [v0.3.0 release readiness (2026-10-29)](#v030-release-readiness-2026-10-29) supersede the 2026-10-28 ones, which predate the last parser change.
- Published GitHub releases are `v0.1.0`, `v0.2.0`, `v0.3.0` (2026-10-29), `v0.4.0` (2026-10-30) and `v0.5.0` (2026-10-31), each by owner decision. `v0.2.1` was never tagged.
- crates.io is on hold by owner decision until the codebase is production-hardened. This replaces the older "token-gated" wording in the L8 and L9 sections. A configured token alone publishes nothing: the publish job in `.github/workflows/release.yml` runs only on a `v*` tag **and** when the repository variable `DDF_CRATES_IO` is `open`. `deep-diff-forge deploy release` reports `crates.io blocked` (measured with the binary).
- The 2026-10-26 section ends with "Nothing is tagged or published". That refers to the council stack only. The `v0.1.0` and `v0.2.0` GitHub releases already existed.
- Test counts in older sections (568, 765, 794) are counts at that time and are not current.

Current docs: [release and publication](docs/RELEASE_AND_PUBLICATION.md), [review council](docs/REVIEW_COUNCIL.md), [deployment gap analysis](docs/DEPLOYMENT_GAP_ANALYSIS.md), [changelog](CHANGELOG.md).

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

## Security + Quality Hardening Round 3 (S1009000 — third-pass bug hunt)

Gate: **935 tests / 0 failed** (was 933). Hunter: forge-security-architect. 0 Critical/High — convergence seal: **PASS**.

| Finding | Severity | Fix | Regression test |
|---|---|---|---|
| FINDING-R3-1: run_lane worker-panic silent swallow | LOW | `scheduler.rs:85` `unwrap_or_default()` → `resume_unwind(payload)` — honest abort over fabricated partial coverage | Structural test: `run_lane_parallel_returns_input_order` still holds; panic propagation design-verified |
| FINDING-R3-2: SYSTEM_AGENT_ID ingestion guard doc-only | INFO | `store.rs`: `add_untrusted()` entry point downgrades `SYSTEM_AGENT_ID` → empty agent id before ingestion; `add()` reserved for trusted internal callers | `add_untrusted_downgrades_system_agent_id_to_agent` + `add_untrusted_accepts_normal_agent_annotations_unchanged` |
| FINDING-R3-3: RUSTSEC-2026-0002 (lru) undated accept | INFO | `deny.toml`: scoped existing comment with 2026-09-01 mandatory review date | Supply-chain governance note |

Also closed (INFO doc-only, committed separately):
- FINDING-3: FNV-1a "non-reversible" overclaim → "opaque" / "brute-force impractical for typical paths" + HMAC note
- FINDING-7: `ensure_runtime_dir` ancestor security assumption now explicitly documented

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

<a id="review-council-and-ci-2026-10-26"></a>

## Review council and CI (2026-10-26)

Measured by the operator unless labelled otherwise. Model reports are inputs to
triage, not evidence. Design and per-round history:
[`docs/REVIEW_COUNCIL.md`](docs/REVIEW_COUNCIL.md).

### main CI

- **Green at run [40274253094](https://github.com/Louranicas/deep-diff-forge/actions/runs/40274253094)**
  `[measured]`, on the stage B merge (`b50b3f0`): secret scan (9 s), declared
  Rust 1.88 compatibility (30 s), deploy gate-feature (2 m 0 s), supply chain
  audit (5 m 25 s), and review controller mutation and fuzz evidence
  (32 m 7 s). This is the first green `main` since 2026-06.
- The evidence job builds the gate image on the GitHub-hosted `ubuntu-latest`
  runner and runs the **real-Podman planted-breach suite** there `[measured]`.
  Getting there took three fixes, each found in a hosted run's logs:
  `DDF_COUNCIL_PODMAN` unset; the sandbox running as the image's uid 1000
  instead of the runner's 1001 (`e075e00`); and a probe-verdict parsing race
  (run 40268250417, fixed in `110c2ad`).
- Run annotations `[measured]` warn that Node 20 actions are deprecated
  (`actions/checkout` is forced onto Node 24) and that `ubuntu-latest` migrates
  to Ubuntu 26 from 2026-10-19. Neither fails the run; both are open.

### Council runs r12–r17

Each run reviewed a pinned worktree with gate image `:20261026a` (r12–r16),
`:20261026b` (r17) or `:20261026c` (`8f4ea620`, r18). Source: the vault triage
notes `Reviews/Council r12–r17 Triage 2026-10-26`.

| Run | ID | Reviewed | Reports | Gates | Mutation | Notes |
| --- | --- | --- | --- | --- | --- | --- |
| r12 | `8e3b0abb` | `bec1a49..049955a` | 5/6 independent | 8/8 | passed | `architecture-claude` rejected for a fabricated citation (lines 0–0); challenge round not started. First real oracle run: 16/16 mutated functions in production spans, 7 hidden functions all tests |
| r13 | `5fc1d260` | `bec1a49..c0c0d6a` | 12/12 | 8/8 | 82: 81 caught, 1 unviable, 0 missed | 1,175 tests; fuzz about 10 M runs; oracle: 20 hidden functions, MIR unchanged, control moved |
| r14 | `278bbc34` | `c0c0d6a..4ccaf10` | 12/12 | 8/8 | 76/76 caught | 1,177 tests; oracle v3: 20 hidden functions, 5 builds, reproducible |
| r15 | `43066928` | `4ccaf10..1ff04ff` | 12/12 | 8/8 | 76/76 caught | First mutation job with `--emit=link,mir`: 15 m 41 s against 13 m 49 s in r14 for the same 76 mutants |
| r16 | `2a569cea` | `1ff04ff..9d38f43` | 12/12 | 8/8 | 8/8 caught | Oracle v5: 1 hidden function, 17 builds, all five configurations built |
| r17 | `8a35a3d1` | `1ff04ff..a2b362e` | 12/12 | 7/8 | refused before any job | The oracle could not list the scope: cargo-mutants rejected the sealed diff ("hunks not in order or overlap"), exposing the touching-hunks defect latent since r10 |

| r18 | `f2baaa43` | `a2b362e..e45aad0` | 11/12 | 7/8 | refused before its job | The oracle **falsely refused** DDF's own `#[cfg(test)]` code: rustc's incremental reuse after an edit-and-restore duplicated allocations in unchanged MIR (measured). The Codex falsifier challenge was refused by the provider's cybersecurity safeguard |
| r19 | `4946f232` | `e45aad0..d3498a1` | 12/12 | 8/8 | 12/12 caught | Oracle v8: 2 hidden functions passed in 10 builds |
| r20 | `0fd3cac8` | `d3498a1..36d5e7c` | 12/12 | 8/8 | 7/7 caught | Consensus: the v9 dep-info check failed open (measured: every build script skipped) |
| r21 | `b3aa0f3c` | `36d5e7c..1f9dbc0` | 12/12 | 7/7 evidence | no mutants (no Rust in scope changed: unmeasured, not passed) | No HIGH findings; consensus MEDIUMs fixed in r22 |

Every finding the councils reached consensus on was fixed on the following
round's branch and re-reviewed through r21; r22's MEDIUM fixes merged under
the oracle's stop rule (see `docs/REVIEW_COUNCIL.md`, r21–r22).

### Exemption oracle (v7, `e45aad0`)

- **Strict exactness** `[measured]`: a bare top-level `#[test]`, a
  feature-conditional `use m::test` and `#[cfg(any(test))]` are refused with
  0 builds; a genuine `#[cfg(test)]` test passes.
- **Attack catalogue** `[measured]`: every route is refused; all but the
  build-script `cfg(test)` route are refused with 0 builds, and that one is
  caught by the build check.
- **DDF's own review package** `[measured]`: 38 hidden functions, all under
  `#[cfg(test)]`, pass in 10 builds and 100 s, with controls moving in both the
  gate and release configurations.
- **Mutation rehearsal** `[measured]`: 27/27 caught.
- **Real Podman** on `:20261026c` (`8f4ea620`) `[measured]`: the planted-breach
  suite passes. The suite fails unless each of its 12 probes reports a refusal
  exactly once.
- **Residual** `[inferred]`: a build script or proc macro that keys
  `cfg(test)` or its output to a configuration other than gate and release, or
  that detects the oracle. Path-qualified test attributes (`#[tokio::test]`),
  bare `#[test]` outside `#[cfg(test)]`, and `unsafe fn` in scope are refused
  by design.

### Exemption oracle (v11, `e658a0c`)

- **DDF's own review package** `[measured]`: 40 hidden functions, all under
  `#[cfg(test)]`, pass in 12 builds and 127 s in the gate image. Every non-test
  unit, including every registry build script, has a dep-info rule naming its
  own output, with all inputs present and within its allow-list.
- **Incremental history** `[measured]`: with incremental reuse, a rebuild after
  the control's edit-and-restore differed from the first build; without it,
  equal. v8 onwards removes incremental state before each build and treats drift
  as inconclusive.
- **Real Podman** `[measured]`:
  - a build script's `cfg(test)` is refused from cargo's own report in 2 builds;
  - a binary `include!`ing a `#[cfg(test)]` module file is refused by the MIR
    comparison;
  - the same through `/src` was not refused by v8 and is refused by v9 onwards
    from dep-info.
- **Stop rule** `[inferred]`: run-time behaviour of build scripts and proc
  macros (reading, copying, symlinks, generating code, detecting the job) is
  one residual class. For path packages it is in the reviewed diff; for
  dependencies, the supply-chain gate governs it.

### Staged merges

`release/stage-a` merged as `88e1cc0` (PR #3) and `release/stage-b` as
`b50b3f0` (PR #4) and `release/stage-c` as `39005c5` (PR #5), all as merge
commits, each after all five required checks passed. Since 2026-10-26, `main` is
protected by ruleset 25868121: pull requests only, every required check, an
up-to-date branch, no force push, and no bypass.
Nothing is tagged or published; crates.io publication is on hold until the
codebase is production-ready.

## Release dry runs and fuzzing (2026-10-28)

Measured by the operator unless labelled otherwise. Code changes: PRs #21–#23
(see [`CHANGELOG.md`](CHANGELOG.md)).

### Release dry runs

A `workflow_dispatch` run of `.github/workflows/release.yml` on a branch is a
dry run. It runs the gates, the three builds, a binary smoke test and checksum
verification. It publishes, releases and attests nothing; those steps run only
on `v*` tags (`release.yml`, header comment and job conditions).

| Run | Commit | Result | Jobs that ran |
| --- | --- | --- | --- |
| 40634103910 | `24e3d53` (PR #22 merge) | green `[measured]` | macOS tests; builds for `x86_64-unknown-linux-gnu`, `aarch64-apple-darwin`, `x86_64-apple-darwin`; smoke on Linux and arm64 macOS; sha256 checksums |
| 40643652548 | `2f03f40` (PR #23 merge) | green `[measured]` | the same set |

- Publish, upload and attestation steps were **skipped by design**: they require
  a tag ref. They are not evidence of a working publish path.
- `x86_64-apple-darwin` is built but not smoked, because `macos-latest` is arm64
  (`smoke: false` in the build matrix).
- Earlier dry runs on the same day failed on macOS and led to fixes. Run
  40626071938 failed `client_rejects_response_over_cap`, fixed in `26654c8`.
  Run 40618280385, the first macOS test run, found the `/proc/self` gate-uid
  fallback, fixed in `7a2f5ae`. See [`NOTES.md`](NOTES.md).

### Fuzzing

Local campaigns at `332f8fd`, seeded with every fixture and regression input
`[measured, operator workstation]`:

| Target | Duration | Runs | Crashes |
| --- | --- | --- | --- |
| `patch_parser` | 300 s | 5,457,367 (about 5.46 M) | 0 |
| `review_json` | 300 s | 4,159,390 (about 4.16 M) | 0 |

CI runs 10 s per target, which is a smoke test, not a campaign. The CI fuzz
smoke for `patch_parser` (run 40634223913) found the first binary-body grammar
fix unstable under render → parse → render. That finding led to `332f8fd` and
the regression `fixtures/patch/regressions/binary-metadata-after-body.patch`.
The other five targets (`daemon_protocol`, `agent_annotation`,
`council_report`, `stdio_mcp`, `stdio_rpc`) had no long campaign on this date
`[unmeasured]`.

### Metadata parsing

The parser used to scan all metadata lines for `GIT binary patch` on every line,
which made parsing quadratic. With an `in_binary_body` flag (`679f7eb`), 1 M
`index 0` metadata lines (8 MB, inside every budget) parse in **0.20 s instead
of 139 s**. 100 k lines take 0.02 s and 200 k lines 0.04 s
`[measured, release build, operator workstation]`.

## v0.3.0 release readiness (2026-10-29)

Measured on 2026-10-29 for the owner's `v0.3.0` tag decision.

- **Fuzzing, all 8 targets, 300 s each, at `main` `8f957aa`.** This is after the last parser change (PR #41). Settings match CI: `cargo +nightly-2026-10-25 fuzz run`, seeded from `fuzz/seeds/`, `-max_len=65536` (`council_report` 4096). Every target exited 0 with no crash artifact.

  | Target | Runs |
  | --- | ---: |
  | `patch_parser` | 6,471,335 |
  | `review_json` | 4,972,954 |
  | `daemon_protocol` | 5,692,847 |
  | `agent_annotation` | 72,808,741 |
  | `council_report` | 51,881,182 |
  | `stdio_mcp` | 1,062,028 |
  | `stdio_rpc` | 1,844,151 |
  | `structural_soundness` | 1,449,217 |

- **Release dry run.** `release.yml` (`workflow_dispatch`) run 40855042953 at `478db26` passed. That covers the supply-chain gate, every quality job including macOS tests, and the builds for `x86_64-unknown-linux-gnu`, `aarch64-apple-darwin` and `x86_64-apple-darwin`. The `crates.io publish (owner-gated)` job was skipped, because `DDF_CRATES_IO` is unset.
- **Tests and gates.** 1,451 tests (passed plus ignored). Pedantic clippy is clean. `scripts/ci_local.sh` reports steps failed: 0. All nine required checks are green on PRs #40, #41 and #42, including diff mutation with no missed mutant.
- **Daemon soak (PR #40).** It sends a request at the 80 MiB cap, which is served, and one byte over, which is refused. With all 8 connection slots held, a ninth connection is closed unanswered, and the daemon serves again afterwards. With 32 concurrent clients: 504 served, 296 closed at the limit, max RSS 7,532 KiB.
- **Unchanged limits.** These are listed in the README's *Project status*: one engine serializes daemon requests, the socket is not checked for peer credentials, macOS tests run weekly and on demand, two `daemon start` commands can race, there is no cockpit key-sequence harness, and syntax features cover Rust only.

## Daemon peer credentials and start lock (2026-10-30)

Measured on 2026-10-30 on Fedora 44 (Linux 7.2), with the debug build of this change.

- **Racing starts.** Eight `daemon start --foreground --socket PATH` processes were started at once over a stale socket (bound and closed). One daemon served `daemon health`; the other seven exited 6 with `another daemon is running or starting on this socket; refusing to replace it`. `daemon stop` removed the socket; `d.sock.lock` (mode `0600`) stayed, as designed.
- **A client of another user.** The daemon ran as uid 1000 with its directory loosened to `0711` and its socket to `0666`, so another user could reach it. A client run as uid 1 inside `podman unshare` (subordinate uid 524288 outside) connected and was reset without an answer. The daemon logged `refused a connection: the peer runs as uid 524288, not uid 1000`. `daemon health` as uid 1000 was still answered.
- **A daemon of another user.** The daemon ran as uid 1 inside `podman unshare`, with the same loosened modes. `deep-diff-forge daemon health` as uid 1000 exited 6 with `the peer runs as uid 524288, not uid 1000`, and the Python `SocketClient` raised `TransportError` with the same uids before sending anything. That daemon also refused both clients from its side.
- **Not measured here:** macOS (`getpeereid`), which the dispatched macOS CI job covers through the unit tests.

## Daemon requests in parallel (2026-10-30)

Measured on 2026-10-30, on a 16-CPU Fedora host, release builds. "Before" is `main` `a863ebd`
(one `Mutex<Engine>`); "after" is branch `claude/daemon-parallel-20261030`.

- **Throughput and latency.** Six concurrent clients each sent four `diff.rank` requests for
  `git diff v0.2.0 v0.3.0` (2,752,920 bytes) to one daemon; `daemon.health` was sent while they
  ran. Two runs each, same results within 1%:

  | | Before | After |
  | --- | ---: | ---: |
  | 24 requests, wall time | 1.03 s | 0.42 s |
  | Requests per second | 23.4 | 56.8 |
  | One request alone | 0.053 s | 0.053 s |
  | `daemon.health` while they ran | 205 ms | 0.2 ms |

  The gain is 2.4×, not 6×: each request also moves 2.75 MB of JSON each way through a Python
  client, which this measures too.
- **10-minute soak with leak checks** (`daemon_soak.py --seconds 600 --require-growth-check`,
  four concurrent clients doing session and rank work every iteration): before, 594 iterations,
  RSS growth 0 KiB, fd growth 0, peak RSS 7,744 KiB; after, 594 iterations, RSS growth 68 KiB,
  fd growth 0, peak RSS 7,876 KiB. Both PASS (limits: 32 MiB, 4 fds).
- **Tests.** `admission.rs` (6): light requests run together, a request exactly filling the
  budget starts, one over the budget runs alone, one that does not fit waits until room is
  freed, a waiting heavy request is not overtaken by a later light one, and a panicking request
  returns its weight. `serve.rs`: a running request (`test.hold`) does not block
  `daemon.health` (before, it would have waited on the mutex); a panic holding the session table
  empties it and no id is reused; a panic holding nothing keeps every session.

## v0.4.0 release readiness (2026-10-30)

Measured on 2026-10-30 for the owner's `v0.4.0` tag (Luke's instruction of 2026-10-30: "Tag
0.4.0 after a release dry run"). It ships what the production-readiness assessment
([docs/PRODUCTION_READINESS.md](docs/PRODUCTION_READINESS.md), 83/100) said would raise the score.

| Change | PR | Evidence |
| --- | --- | --- |
| Peer-credential check on both ends; a start lock stops two `daemon start`s racing | #44 | [Daemon peer credentials and start lock](#daemon-peer-credentials-and-start-lock-2026-10-30) |
| macOS on every pull request (required check 10 of ruleset 25868121); weekly `campaign.yml` (600 s leak-checked soak, an hour of fuzzing per target); soak leak checks | #46 | Campaign run 40989103096 (dispatched on `main` `50abb6e`, fuzz 120 s per target): soak and all 8 fuzz jobs green |
| `deploy doctor` | #47 | CI 10/10 at `217ed0f`; 6 missed mutants found by CI, fixed, verified locally |
| Daemon requests in parallel under a 128 MiB in-flight budget | #48 | [Daemon requests in parallel](#daemon-requests-in-parallel-2026-10-30); CI 10/10 at `bc7f4cf`, macOS included |

Repository settings changed the same day: Dependabot alerts and security updates on, secret
push protection on; 72 branches already merged into `main` deleted from both remotes (restore
list kept outside the repository).

One local gate run of PR #48, after `main` was merged in, reported a failed test. Seven later
full runs did not reproduce it, and its output was not kept; the gate script now keeps the
test log. It is recorded here as an unexplained, unreproduced failure.

### v0.4.0 dry run, tag and published release (2026-10-30)

- **Dry run** 41000435240 (`workflow_dispatch` on `8223710`): every quality gate including
  macOS, all three builds (x86_64 Linux, x86_64 and aarch64 macOS), crates.io skipped as held.
- **Merge.** PR #49 merged as `ff9e079`, whose tree (`79a2c07`) is identical to the dry-run
  commit's, so the tag names the exact tree the dry run built. `just gate-release` passed on
  `ff9e079` and left the tree clean.
- **Tag** `v0.4.0` (annotated, on `ff9e079`) pushed to GitHub and GitLab.
- **Release run** 41004772365 (`push` of `v0.4.0`): success. The GitHub release `v0.4.0` is
  published and marked Latest, with three binaries, their `.sha256` files and `sbom.spdx.json`.
- **Verified after download:** `sha256sum -c` OK for the x86_64 Linux archive;
  `deep-diff-forge --version` prints `deep-diff-forge 0.4.0`; `gh attestation verify` passes
  with one SLSA v1 provenance signed by `release.yml@refs/tags/v0.4.0`.

## Daemon metrics overhead (2026-10-30)

Plan item N5: recording metrics on the parallel request path must cost under 1%. Measured on a
16-CPU host with release builds. "Without" is branch `claude/verified-findings-20261030` (no
metrics); "with" is `claude/daemon-metrics-20261030`. The workload is six clients sending four
`diff.rank` requests each for a 2,752,920-byte patch, as in [Daemon requests in parallel](#daemon-requests-in-parallel-2026-10-30).
Two alternating runs each:

| | Without metrics | With metrics |
| --- | ---: | ---: |
| Requests per second | 62.79, 62.53 | 62.91, 62.85 |
| One request alone | 0.052 s | 0.052 s |
| `daemon.health` under load | 0.2 ms | 0.3 ms |

The throughput difference (+0.4%) is within run-to-run noise: no overhead is measurable at this
resolution.

## Findings verification by address (2026-10-30)

A benchmark of the findings verifier (`deep-diff-forge bench --quick`, release build, 16-CPU host)
found that each address-anchored finding hashed every hunk of the review again. The workloads:

- `findings_by_address`: 2,000 findings, five per hunk, against a generated 400-hunk patch;
- `findings_verify`: 400 findings anchored by path and line, against a 200-hunk patch.

| Workload | Before | After (one address table per call) |
| --- | ---: | ---: |
| `findings_by_address` | 3,560 ms | 33 ms |
| `findings_verify` | 9.0 ms | 4.6 ms |

The cost had grown with findings times hunks. Through the daemon, one `findings.verify` request
of 10,000 findings against a large patch could hold a worker for minutes. It is now linear in
findings, with one hash per hunk per call.


## v0.5.0 release readiness (2026-10-31)

Measured on 2026-10-31 for the owner's `v0.5.0` tag (Luke's instruction of 2026-10-31: "update
tag"). 0.5.0, not 0.4.1: the graph crate's API changed (`RankedFile` is `#[non_exhaustive]`, built
only by `RankedFile::scored`, and `RiskSignal` gains `Code`). It ships the plan in
[docs/PLAN_AGENT_EVIDENCE_AND_OBSERVABILITY.md](docs/PLAN_AGENT_EVIDENCE_AND_OBSERVABILITY.md).

| Change | PR | Evidence |
| --- | --- | --- |
| Code evidence in risk ranking (nine lexical signals; `evidence_scan` fuzz target) | #51 | CI 10/10 |
| Verified agent findings (`findings verify`, daemon `findings.verify`, MCP, Python SDK; `findings_verify` fuzz target) | #52 | CI 10/10; [findings verification by address](#findings-verification-by-address-2026-10-30) |
| Daemon metrics (protocol 2) and JSON logs; `deploy doctor` reads the metrics | #53 | CI 10/10; [daemon metrics overhead](#daemon-metrics-overhead-2026-10-30) (no overhead measurable) |
| `bench` and `eval`, the PR bench comparison and the weekly campaign receipt | #54 | CI 11/11 (the bench job skipped: its merge base had no `bench`); a review workflow found 10 defects, fixed before merge |
| DDF-EVID-001: code inside string interpolation is scanned as code; the scanner errs toward code; JS `child_process` imports | #55 | CI 11/11, macOS included; the PR's bench comparison, the first real one, found 0 regressions in 7 workloads (largest ratio 1.10x) |

The release PR also makes the `bench` comparison a required check (renamed `bench (base vs
head)`), so ruleset 25868121 requires eleven checks. Before this, the job had made one real
comparison with 0 false failures. One run is a small sample (measured), so a false failure on
a noisy runner remains possible (inferred); a rerun of the job is the first response.

### v0.5.0 dry runs, tag and published release (2026-10-31)

- **Dry run 1** 41129704309 (`workflow_dispatch` on `e1369a7`): failed. macOS tests:
  `json_logs_are_one_object_per_line` panicked on a BrokenPipe from its own write. The daemon
  had dropped a connection over its limit of 8, as designed, while a slow runner was still
  computing earlier answers. The PR's own macOS run had passed. The test was fixed in
  `5b75570` (DDF-TEST-002): a failed write skips that attempt, and each attempt waits up to
  3 s for the log line. Measured locally, 12 copies pinned to one CPU pass 12 of 12.
- **Dry run 2** 41137004116 (`workflow_dispatch` on `5b75570`): every quality gate including
  macOS, all three builds; crates.io skipped as held.
- **Merge.** PR #56 merged as `36bd5f3`, whose tree (`d521c19`) is identical to dry run 2's
  commit. `just gate-release` passed on `36bd5f3` and left the tree clean.
- **Tag** `v0.5.0` (annotated, unsigned, on `36bd5f3`) pushed to GitHub and GitLab.
- **Release run** 41143246901 (`push` of `v0.5.0`): success. The GitHub release `v0.5.0` is
  published (2026-10-31T00:02:49Z) and is the repository's latest release, with three
  binaries, their `.sha256` files and `sbom.spdx.json`.
- **Verified after download:** `sha256sum -c` OK for the x86_64 Linux archive;
  `deep-diff-forge --version` prints `deep-diff-forge 0.5.0`; `gh attestation verify` passes
  with one SLSA v1 provenance signed by `release.yml@refs/tags/v0.5.0`.
- **Ruleset** 25868121 requires eleven checks from 2026-10-31: `bench (base vs head)` was
  added after the merge (`gh api`; the previous ruleset JSON is kept outside the repository).

### v0.5.0 weekly campaign at its declared durations (2026-10-31)

Campaign run 41143303217 (`workflow_dispatch` of `campaign.yml` on `36bd5f3`, the `v0.5.0`
commit, default inputs): success. It is the first run at the declared durations; the earlier
run 40989103096 fuzzed 120 s per target.

- **Daemon soak (long):** 600 s, `--require-growth-check`, verdict PASS.
- **Bench (full):** every workload, 15 repetitions, release build; the receipt is kept as the
  run's `bench-receipt` artifact.
- **Fuzz:** each of the 10 targets in `fuzz/Cargo.toml` for 3,601 s, 0 crashes:

| Target | Runs |
| --- | ---: |
| `stdio_rpc` | 6,487,749 |
| `stdio_mcp` | 6,347,810 |
| `evidence_scan` | 5,552,269 |
| `agent_annotation` | 558,141,705 |
| `findings_verify` | 7,735,397 |
| `daemon_protocol` | 14,719,136 |
| `patch_parser` | 21,686,009 |
| `structural_soundness` | 4,660,599 |
| `council_report` | 236,879,308 |
| `review_json` | 14,008,696 |

## Release integrity settings (2026-10-31)

Repository settings, measured with `gh api` on 2026-10-31:

- **Environment `release`:** deployment policy `custom_branch_policies`, with one policy of
  type `tag` named `v*`, so only a `v*` tag can run a job that names it. The `publish` job
  names it. No required reviewers (owner decision pending: an approval step would stop
  unattended releases).
- **Ruleset 26252036 "Release tags are immutable":** target `tag`, `refs/tags/v*`, rules
  `deletion`, `update` and `non_fast_forward`, enforcement active, no bypass actors.
  Unmeasured: a live refusal. A probe that pushes, moves and deletes a throwaway tag needs a
  force push, which this session's permissions do not allow.
- **Not done:** signed tags. They need the owner's signing key; `v0.1.0` to `v0.5.0` are
  annotated and unsigned.

## Whole-crate mutation of graph and agent (2026-10-31)

The 2026-10-31 readiness reconciliation asked for a whole-crate mutation pass of the crates
that 0.5.0 grew. Measured: cargo-mutants 27.1.0 on `main` `36bd5f3` (`v0.5.0`),
`-p deep-diff-forge-graph -p deep-diff-forge-agent --test-workspace true --timeout 300 -j 6`,
with `CARGO_TARGET_DIR` unset.

- **651 mutants in 71 min:** 593 caught, 18 missed, 0 timeouts, 40 unviable.
- **Missed.** 12 in `agent`: `ReviewRevision::from_review_id` (5), `ReviewRevision::as_str`
  (2), `EvidenceRegistry::assess` (`||` to `&&`), `EvidenceRegistry::inspect` (to `vec![]`),
  the span-id comparison in `validate_anchor`, and `AnnotationStore::is_empty` (2). 6 in
  `graph`: `+=` replaced by `*=` or `-=` in `score_path_and_shape`. Tests compared scores
  with each other, not exact sums.
- **Pinned.** Tests that pin each one were added on `claude/mutation-pins-20261031`. A
  re-run over those functions (`-F 'from_review_id|as_str|assess|inspect|validate_anchor|is_empty|score_path_and_shape'`)
  tested 61 mutants: 56 caught, 5 unviable, 0 missed.
- **Ledger.** Recorded as Turso event 106 (kind `mutation`).


## Whole-crate mutation of core, patch, daemon and syntax (2026-11-01)

The readiness gap "no whole-crate mutation run since council history (120 missed, recorded)"
names the Turso ledger's totals. Measured from the ledger (`mutation_outcomes`): those misses
were 94 in `review` (council gate rehearsals r7 to r15, diff-scoped), 25 in `daemon` (campaign
`mutants/daemon-rerun`) and 2 in `syntax` (campaign `mutants/syntax`), all on older code. This
pass re-ran the crates that matter most for patch truth and the trust boundary, on the code of
`main` `9527f64`.

**Method.** cargo-mutants 27.1.0, `-p <crate> -j 6 --timeout-multiplier 3`, `CARGO_TARGET_DIR`
unset, on an export of `9527f64`: each crate against its own tests (crate scope). The survivors
were then re-run with `--iterate --test-workspace true --timeout 300` (workspace scope, as the
`diff-mutation` check runs). After the pins, the same commands ran on this branch.

| Crate | Mutants | Before: caught / missed / timeouts / unviable | After: caught / missed / timeouts / unviable |
| --- | ---: | --- | --- |
| `core` | 196 | 166 / 22 / 0 / 8 | 185 / 3 / 0 / 8 |
| `patch` | 637 | 500 / 82 / 3 / 52 | 581 / 4 / 0 / 52, then 3 missed (below) |
| `daemon` | 565 | 478 / 16 / 13 / 58 | not re-run whole (below) |
| `syntax` | 435 | 355 / 29 / 20 / 31 | 397 / 3 / 3 / 32; 0 timeouts at 300 s (below) |

- **Workspace scope, before.** `core`: 22 re-run, 20 caught, 2 missed. `patch`: 85 re-run, 58
  caught, 25 missed, 2 timeouts. `daemon`: 29 re-run, 27 caught, 1 missed, 1 timeout. Of the
  catches, 2 (`core`), 40 (`patch`) and 2 (`daemon`) failed only
  `graph`'s `the_import_rule_is_linear_on_long_lines`, whose 10 s deadline the loaded machine
  (load average 40 to 68) exceeded; those catches are spurious, and the `patch` workspace
  verdicts are not evidence. Alone the test took 2.4 s.
- **Pinned.** 51 tests. `core`: hunk counts, body bytes, `body_sha256` and `address` against
  `sha256sum` of the documented bytes; `json_escape` keeps a space. `patch`: Git C-quoted path
  decoding and quoting (every escape, octal, NUL, round trip); raw-record parsing (mode width,
  similarity score, the score's status, the section separator, bytes before the first block);
  every block identity check (old and new `index` IDs, abbreviated IDs, header end, copy,
  rename and mode-change lines); an appended stream's ranges; `GitSelection::label`; a failing
  Git's stderr cut at exactly 8 KiB; a repository path over 1 KiB (Linux only); a nested
  repository named in `omitted_untracked`; the exact `review.v0` document for two files; parser
  refusals (an empty hunk, a truncated side, contradictory rename paths) and statuses.
  `daemon`: a socket line ends at its newline while the peer stays open. `syntax`: every
  highlight label, colour and capture family; empty spans; the byte, node and cell budgets at
  exactly their limit (2 MiB, 200,000 nodes, a 2,000 by 2,000 line table, gaps of 4 M cells);
  exact-LCS minimality; `union` symbols.
- **Watchdogs.** Loop mutants (`i += 1` to `*=`, a cursor moved back) hang. The tests that run
  `decode_quoted_prefix`, `structural_diff`, `mark_moves`, the exhaustive `lcs_ops` and
  `line_anchored_ops` checks and `analyze` now run under a 10 s deadline. `syntax`'s 20
  timeouts fell to 3 at the automatic 20 s limit, where the hung threads slow the rest of the
  suite; `-F 'lcs_ops|mark_moves' --timeout 300` tested 122: 119 caught, 1 missed (equivalent,
  below), 0 timeouts.
- **`patch` after.** The 4th miss, `omitted_untracked`, was pinned after the run:
  `-F omitted_untracked` tested 2, 1 caught, 1 unviable.
- **`daemon` after.** The added test targets the workspace-scope timeout
  `read_capped_line_deadline_noting` (`||` to `&&`). `-F read_capped_line_deadline_noting`
  tested 14: 12 caught, 1 unviable, 1 timeout, also at `--timeout 300`. The new test fails in
  5 s, but `inspect::tests::a_running_daemon_is_live_and_its_lock_usable` then runs past 300 s.
  Open.
- **Equivalent or not compiled here (12: 9 in `core`, `patch` and `syntax`, the after-run
  misses; 3 in `daemon`, workspace scope).** `core` `seal.rs` `compress`, `^` to `|` at 113:32,
  120:33 and 120:49: the two terms of SHA-256's Ch share no set bit, and either XOR in Maj can
  be an OR (checked over all inputs). `patch` `git_raw.rs:64` (2): the
  `#[cfg(not(unix))]` `to_os_string`, compiled on neither Linux nor macOS. `patch`
  `parser.rs:568:61`: the `new file mode`/`deleted file mode` clause after a rename is
  unreachable, since the `InconsistentHeader` check returns first. `syntax`
  `structural.rs:386:70` and `484:70` (`j + 1` to `j * 1` in backtracking): on a mismatch the
  cell is the maximum of its two neighbours, so both comparisons choose alike. `syntax`
  `highlight.rs:272:23` (`>` to `>=`): at equality the gap is empty. `daemon`
  `serve.rs:630:21` and `security.rs:157:21` (`NotFound` guard to `true`, inferred): for any
  other `lstat` error `create_dir_all` fails on the same path with the same kind. `daemon`
  `serve.rs:883:25` (`Interrupted` guard to `false`, inferred from the standard library's
  source): `UnixListener::accept` retries `EINTR` itself. In the workspace-scope run
  `serve.rs:630:21` was the 1 miss; `security.rs:157:21` and `serve.rs:883:25` are the 2
  catches that failed only `graph`'s timing test.
- **`review`, crate scope only.** 1,247 mutants in 12 min: 440 caught, 727 missed, 2 timeouts
  (`mutation_scope.rs:412`, `code_lines`), 78 unviable. The misses are in `execution.rs` (200),
  `storage.rs` (147), `journal.rs` (108), `tuicr.rs` (91), `cli.rs` (86), `mutation_scope.rs`
  (40), `sandbox.rs` (39), `policy.rs` (14) and `model.rs` (2). The crate's behaviour is tested
  mostly from `cli` (council, sandbox and journal suites), so this count is not its coverage.
- **Not done.** `review` in workspace scope (its 729 survivors at about 3 min each, several
  hours here).
  `cli`, `tui`, `learning`, `pipeline`, `projection`, `cluster` and `sdk` were not run. The
  ledger was not written: this pass is recorded here only.

## Review crate: sandbox mutation (2026-11-01)

A whole-file mutation pass over the rootless-Podman gate sandbox,
`crates/deep-diff-forge-review/src/sandbox.rs`, at workspace scope, the scope CI's diff
mutation uses. Measured: cargo-mutants 27.1.0 on `main` `7774957`, with `CARGO_TARGET_DIR`
unset:

```
cargo mutants -p deep-diff-forge-review --file crates/deep-diff-forge-review/src/sandbox.rs \
  --test-workspace true -j 6 --timeout 300 -o <out>
```

- **Before: 41 mutants in 9 min:** 28 caught, 3 missed, 0 timeouts, 10 unviable. That run is
  contaminated: 9 of its mutant logs end in "Disk quota exceeded" on the shared `/tmp`. Those 9
  are 8 false "unviable" (`control`'s `* with +` and `* with /` at 95:36, `>` to `<` and `==` at
  111:32, its three `Ok(Some(..))` replacements at 79:9, and `from_env`'s `delete !` at 59:13)
  and one false "caught" (`&&` to `||` in `from_env`). A clean re-run on the same code plus only the `cpus` test (below)
  gave 36 caught, 3 missed, 2 unviable. So `main` had 5 real survivors: `from_env` `&&` to `||`,
  `control`'s 64 KiB output bound (`* with +`), `cpus` returning 0 or 1, and `>` to `>=` in
  the control deadline.
- **Killed.** `cli/tests/council.rs`: `gates_launch_the_resolved_image_id_...` now asserts
  `--cpus=<host CPUs − 1, min 1>` in argv and receipt (it only checked the prefix). It kills
  `cpus -> 0` on any host but `cpus -> 1` only with at least 3 available CPUs: on 1 or 2 the
  bound is 1 either way (measured here on 16).
  `an_empty_runtime_or_image_is_refused_as_configuration` refuses a blank `DDF_COUNCIL_PODMAN`
  and an empty `DDF_COUNCIL_GATE_IMAGE` by name, and its recording fake runtime shows it was
  never invoked (before, an empty image reached `image inspect` and failed there with another
  message). `sandbox.rs` unit test
  `control_output_is_kept_whole_up_to_64_kib_and_no_further`: 60,000 bytes arrive whole and
  200,000 are cut at 65,536. No production code changed.
- **After.** A targeted run (`-F 'from_env|control|cpus'`, `--timeout 900`) tested 15 mutants:
  14 caught, 1 unviable, 0 missed. Each new test failed on its mutant, except `>` to `>=`
  (below). The full command again, at load average 60 to 240 from parallel mutation campaigns
  on this 16-CPU host: 41 mutants in 49 min, 21 caught, 0 missed, 18 timeouts, 2 unviable.
  17 of the 18 timeouts were caught in the clean re-run or the targeted run; the 18th, 111:32
  `>=`, is the argued-equivalent mutant and has no genuine kill. Their logs stop in other
  crates' tests at 300 s (the `council` suite took up to 254 s in this run, against at most
  40 s in the clean one, and 122 s for the `==` deadline mutant, which waits on its control
  deadline by design). A clean full run with 0 timeouts is still to be measured on an idle host.
- **Load flakes.** In the targeted run `vendoring_output_is_sealed_...` failed (no
  `qualification-error.txt`) on 3 of 15 mutants, and `qualification_cannot_restore_pass_...` on
  one, none of which touch what they check. Cause, from the code: both ran under the fixture's
  2 s job budget, and a job that times out is never qualified, so it gets no marker. Both now
  take `long_budget`, like the other cases whose refusal must not come from the clock. The
  `cpus` kills hold regardless: their own assertion (council.rs `--cpus`) failed too.
- **Final.** The full command with `--timeout 900`, on this branch's last commit, at load
  average 56 to 165: 41 mutants in 78 min, 38 caught, 1 missed, 0 timeouts, 2 unviable. The one
  missed is 111:32 `>=`, the equivalent below. `cpus -> 1` failed only on the `--cpus` assertion.
  `cpus -> 0` failed on it too, plus 5 other council cases. Those still use the 2 s budget, so
  read them as load and not as kills. The vendoring and marker cases did not fail on any mutant
  except one that panics every gate.
- **Equivalent.** `start.elapsed() > CONTROL_DEADLINE` to `>=` (line 111). The two differ
  only when a monotonic nanosecond reading equals 30 s exactly. A 20 ms poll that misses that
  reading fires on the next one under either operator, so the deadline moves by at most one
  poll and no test can tell them apart. The targeted run's "caught" for it was false: the
  failing tests were the load flakes above, not a deadline test.
- **Unviable (2).** `from_env` to `Ok(Default::default())` and `command` to
  `Default::default()`: neither `Sandbox` nor `Command` implements `Default`.

## Review crate: mutation_scope mutation (2026-11-01)

`crates/deep-diff-forge-review/src/mutation_scope.rs` decides which lines the council's
diff-scoped mutation gate mutates, so a gap in its tests could weaken the gate without anyone
noticing. Measured: cargo-mutants 27.1.0 on `main` `7774957`, with `CARGO_TARGET_DIR` unset:
`cargo mutants -p deep-diff-forge-review --file crates/deep-diff-forge-review/src/mutation_scope.rs --test-workspace true -j 6 --timeout 300`.

- **Before, 271 mutants:** 267 caught, 0 missed, 1 timeout, 3 unviable. One of the caught
  mutants (`verified` returning `Ok(vec![])`) was lost when `/tmp` ran out of quota, so it was
  re-run alone with `-F` (caught).
- **Crate scope compared with workspace scope.** The 2026-10-31 whole-crate run (crate scope)
  missed 40 mutants in this file: `require_base`, `verified`, `hex`, `base_blobs`, `capture`
  and `capture_from`. All 40 are caught at workspace scope, by the end-to-end `mutation_base`
  tests in `cli/tests/council.rs`. CI's diff mutation also runs at workspace scope.
- **Timeout.** `byte == b'r'` replaced by `!=` in `code_lines` (line 412) makes the scan count
  `#` runs at every byte. That is a semantic break, and an existing test already caught it:
  `code_lines_skip_comments_and_attributes_across_lines` failed for this mutant in the before
  run (its log shows the failure ahead of `*** result: Timeout`). The 1 MiB `#` run in
  `char_literals_and_lifetimes_are_told_apart` then takes hours, so libtest could not exit and the
  outcome was reported as a timeout. The scan now runs on its own thread under a 30 s wall-clock
  deadline. The deadline does not detect this mutant; it only lets the test binary finish, so
  the existing failure is reported as caught. It would be the only detector for a purely
  quadratic, behaviour-preserving change, and no mutant in this run was one. The bound is 15x
  or more above normal (the whole review lib-test binary takes about 1.7 s), so it does not fail
  on a busy host the way `graph`'s 10 s bound did. Inferred: the caught classification also
  depends on how long the workspace's earlier test binaries take, because the review lib binary
  runs after them inside the same 300 s mutant timeout. Measured: this mutant's test phase took
  184 s under the first after run's 120 s deadline, and 102 s under the 30 s deadline.
- **After, 271 mutants in 46 min (twice):** 268 caught, 0 missed, 0 timeouts, 3 unviable, both
  with the 120 s deadline and with the 30 s one. The line-412 mutant is caught: its review
  lib-test binary reports both tests failed and `finished in 30.00s`.
- **Equivalents:** none. The 3 unviable mutants turn `<<` into `>>` in `LINE_CELL_BUDGET`,
  `TOTAL_CELL_BUDGET` and `LINE_CAP`. `LINE_CELL_BUDGET` and `TOTAL_CELL_BUDGET` fail the
  tests' `const` assertions (`error[E0080]`). `LINE_CAP` becomes 0, and `LINE_CAP - 1` in the
  tests fails the `arithmetic_overflow` lint at compile time.
- **Measurement note.** Four attempts at the after run, and one run limited with `-F`, were
  stopped and thrown away. Each had `TMPDIR` on `/var/mnt/storage`, moved there to avoid the
  `/tmp` quota. On that disk, the `council` integration binary had not finished when the 300 s
  limit ran out, where it had taken 34 s on tmpfs. Mutants whose tests had already failed were
  therefore reported as timeouts. Build copies and
  the tests' temporary directories belong on tmpfs. Six build copies took about 14 GiB of the
  38.5 GiB per-user `/tmp` quota.

## Review crate: policy and model mutation (2026-11-01)

The whole-crate baseline of `deep-diff-forge-review` (crate scope, cargo-mutants 27.1.0,
started 2026-10-31T13:49:52Z on a tree byte-identical to `main` `9527f64`, whose review crate
is unchanged through `7774957`; `env -u CARGO_TARGET_DIR cargo mutants -p deep-diff-forge-review
-j 6 --timeout-multiplier 3`) listed 727 missed mutants; no Turso event records it, so this
paragraph is its durable reference. Of those, 16 were in the council's trust boundary: 14 in
`policy.rs` (`validate_request`'s model argument checks; `validate_report`'s unknown-reviewer check, blank/over-budget summary, claim
and rationale, finding-id prefix/length/uniqueness, citation and count budgets, duplicate
responses; `reports_expected`) and 2 in `model.rs` (`Persona::lens` replaced by `""` or
`"xyzzy"`). Measured: cargo-mutants 27.1.0 on `main` `7774957`, with `CARGO_TARGET_DIR` unset
and `TMPDIR` on the storage disk (the `/tmp` quota was exhausted by concurrent runs),
`cargo mutants -p deep-diff-forge-review --file crates/deep-diff-forge-review/src/<policy|model>.rs --test-workspace true -j 6 --timeout 300`.

- **Before (workspace scope, host load 60–176):** `policy.rs` 87 mutants: 49 caught, 0
  missed, 37 timeouts, 1 unviable. `model.rs` 15: 2 caught, 0 missed, 12 timeouts, 1
  unviable. The timeouts are the host's, not hangs: every timed-out mutant was killed while
  `cli`'s `council` test binary (200–250 s on its own under that load) was running, before
  Cargo reached the review crate's tests that fail on it. Workspace scope at this load cannot
  separate a missed mutant from a timeout, so the crate-scope baseline is the survivor list.
- **Pinned.** Nine contract tests in `crates/deep-diff-forge-review/tests/contracts.rs`
  (`claude/mut-policy-20261101`, `432a0c7`). Each asserts the refusal by its reason, so an
  earlier check cannot satisfy it: a pinned model is one bounded, non-option token; a report
  from outside the roster is refused; summary, claim, falsifier and rationale are refused blank
  and one byte over budget and admitted at it; finding IDs are prefixed, at most 164 bytes and
  unique; 1–16 citations; 128 findings and 1,536 responses are budgeted separately;
  `reports_expected` is two per reviewer for 2 and 3 reviewers (3 separates `n * 2` from
  `n + 2`, which 2 cannot); each persona has its own nonempty lens (checked on
  `Persona::lens`, not on the prompt that embeds it).
- **Targeted check (load ~40):** the same two files with
  `-j 4 --timeout 300 -- --lib --test contracts` (crate scope, only the review crate's unit
  and contract tests): 102 mutants, 100 caught, 2 unviable, 0 missed, 0 timeouts in 10 min (re-run after the test fixes: same counts in 2 min at lower load).
  All 16 baseline survivors are caught.
- **After (workspace scope, same command):** `policy.rs` 12 caught, 74 timeouts, 1 unviable;
  `model.rs` 3 caught, 11 timeouts, 1 unviable; 0 missed. A `--iterate` re-run of the
  timeouts at load 40–65 caught 40 + 4 more and timed out 34 + 7, each again inside `cli`'s
  `council` binary. A further `--iterate` with `--timeout 900` and nothing else changed
  caught all of them (34 + 7, 0 timeouts). Combined: `policy.rs` 86 caught, 1 unviable;
  `model.rs` 14 caught, 1 unviable; **0 missed, 0 timeouts at 900 s**. At the specified
  300 s, workspace scope with `-j 6` on this host is not measured clean: the timeout is
  shorter than the workspace suite runs under contention.
- **Equivalents.** None. The 2 unviable mutants (`assess` and `Gate::required` replaced by
  `Default::default()`) do not compile: `Assessment` and `Gate` have no `Default`.
- **Observed, not retried:** under load, `cli`'s
  `vendoring_output_is_sealed_and_planted_links_or_fifos_are_never_mounted` and
  `qualification_cannot_restore_pass_over_a_refusal_marker` failed in several mutant runs of
  `check_execution`; whether they fail on the mutant or on load is not separated.
