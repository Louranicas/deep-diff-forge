# Changelog

All notable changes to Deep-Diff-Forge are documented here. The format is based
on [Keep a Changelog](https://keepachangelog.com/), and the project follows
[Semantic Versioning](https://semver.org/).

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
