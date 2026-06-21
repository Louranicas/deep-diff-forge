# Changelog

All notable changes to Deep-Diff-Forge are documented here. The format is based
on [Keep a Changelog](https://keepachangelog.com/), and the project follows
[Semantic Versioning](https://semver.org/).

## [0.2.0] - 2026-06-22

The **L9 (Learning)** layer plus the first crates.io-publishable cut — 12 crates,
703 tests, zero `unsafe`, supply-chain-gated. `cargo publish --dry-run` is clean
across the workspace; the crates.io upload itself remains token-gated.

### Added

- **Learning (L9)** — `deep-diff-forge-learning`: the local-only learning loop.
  Per-decision [`StrategyReceipt`]s (hashes/counts/timings — never a path or
  source line) are appended as JSONL under `$XDG_STATE_HOME/deep-diff-forge/
  learning/`, scored per strategy, and gated for promotion. Three learning units
  — planner (strategy + budget), ranking (risk weights), annotation (agent trust
  tiers, *untrusted until grounded*) — plus an explainable promotion gate. CLI
  `learn {status|record --stdin} [--json]`. 135 tests; never uploads, never
  mutates patch truth.

### Changed

- **Publishable-workspace manifests** — versions and internal deps are now
  inherited via `[workspace.package]` + `[workspace.dependencies]`; every crate
  carries `description`, `keywords`, `categories`, and `homepage`. `core` and
  `cli` gained the `description` that `cargo publish` requires.

### Fixed

- **Broken-pipe handling** — bulk CLI output (`--stdin-patch`, `--rank`,
  `--cluster`, `--jsonl`, `semantic`, `review --probe`, `learn`) is routed
  through a broken-pipe-tolerant writer, so `deep-diff-forge … | head` exits `0`
  instead of panicking with `EPIPE` (exit 101). Resolves the L1-era open finding;
  std-only, no `unsafe`.

### Security

Pre-publication adversarial hardening (S1008412): an 8-dimension STRIDE review
with independent verification produced a CVSS-scored register of 17 confirmed
findings (4 MEDIUM, 12 LOW, 1 INFO; **no Critical/High**), all remediated with
fail-before/pass-after regression tests. See `SECURITY.md`.

- **Terminal/ANSI-escape injection** — attacker-controlled diff bodies, file
  paths, and symbol names are now sanitized by `core::display_safe` (control
  chars escaped to a visible `\xHH`) at every human-render boundary; `json_escape`
  also neutralises `DEL` + the C1 block. A reviewer rendering a hostile patch can
  no longer have their terminal hijacked (screen clobber, scrollback forgery,
  OSC-52 clipboard write, title/hyperlink spoof).
- **Memory-exhaustion DoS** — stdin, source-file, and daemon request reads are
  hard-capped at the byte budget instead of buffering unbounded input.
- **Daemon robustness** — per-connection panic/error isolation (`catch_unwind`,
  no fatal `?`), a read timeout (slowloris), a size-bounded request line, symlink
  rejection, and **no world-writable `/tmp` socket fallback** (fails closed
  without `$XDG_RUNTIME_DIR`).
- **Fail-closed trust** — agent annotation `source` is no longer inferred from an
  attacker-controlled label; learning-store files are owner-private (`0700`/`0600`).
- **Supply chain** — `unsafe_code = "forbid"` is now compiler-enforced
  workspace-wide; `tree-sitter` is pinned to exact versions; a strict `cargo
  audit` gate runs in CI and gates the irreversible crates.io publish;
  least-privilege workflow permissions; Dependabot; `SECURITY.md`.

[0.2.0]: https://github.com/Louranicas/deep-diff-forge/releases/tag/v0.2.0

## [0.1.0] - 2026-06-22

First tagged release. The complete diff/review engine (layers L1–L7,
Patch→Daemon) at deployment maturity **L8 (Release)** — 11 crates, 568 tests,
zero `unsafe`, supply-chain-gated.

### Added

- **Patch (L1)** — `deep-diff-forge-patch`: unified/Git patch parser, apply-able
  renderer, and `deep-diff-forge.review.v0` JSON. CLI `--stdin-patch [--json]`.
- **Projection (L2)** — `deep-diff-forge-projection`: renderer-neutral inline and
  side-by-side views. CLI `--layout inline|side-by-side`.
- **Pipeline (L3)** — `deep-diff-forge-pipeline`: composable Unix-filter stages
  (`ChainStage`) and JSONL streaming. CLI `--jsonl`.
- **Semantic (L4)** — `deep-diff-forge-syntax`: tree-sitter language detection,
  budgeted parsing, and symbol extraction (Rust). CLI `semantic <path> [--json]`.
- **Review (L5)** — `deep-diff-forge-graph` (Review Intelligence Graph,
  deterministic risk ranking; CLI `--rank`), `deep-diff-forge-agent` (annotation
  provenance, grounding, sanitization), and `deep-diff-forge-tui` (review TUI;
  CLI `review [--probe]`).
- **Cluster (L6)** — `deep-diff-forge-cluster`: bounded parallel dimensional
  execution with deterministic joins and receipts. CLI `--cluster [--parallel]`.
- **Daemon (L7)** — `deep-diff-forge-daemon`: optional UDS JSON-RPC service
  (std-first, owner-private sockets). CLI `daemon {path|start|health|status|stop}`.
- **Deployment surface** — `deploy status` and `deploy release` machine-readable
  reports (`deployment-status.v0`, `release.v0`); CI workflow; `deny.toml`
  supply-chain policy; release workflow (`release.yml`).

### Engineering

- Strictly acyclic 11-crate workspace; `core` is pure vocabulary.
- Patch truth is preserved across every layer (never mutated by enrichment).
- Each production crate carries at least 50 meaningful tests.
- Pedantic clippy clean; no production `unwrap`/`expect`; no `unsafe`.

### Not yet released

- **crates.io publication** is pending a registry token (the release workflow
  publishes automatically when `CARGO_REGISTRY_TOKEN` is configured).

[0.1.0]: https://github.com/Louranicas/deep-diff-forge/releases/tag/v0.1.0
