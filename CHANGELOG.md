# Changelog

All notable changes to Deep-Diff-Forge are documented here. The format is based
on [Keep a Changelog](https://keepachangelog.com/), and the project follows
[Semantic Versioning](https://semver.org/).

## [0.2.0] - 2026-06-22

The **L9 (Learning)** layer plus the first crates.io-publishable cut — 12 crates,
765 tests, zero `unsafe`, supply-chain-gated. `cargo publish --dry-run` is clean
across the workspace; the crates.io upload itself remains token-gated.

### Added

- **Syntax highlighting** — `highlight <path> [--color|--no-color]` (+
  `syntax::highlight`): tree-sitter highlighting reusing the grammar's own
  `highlights.scm`, mapped to an ANSI palette. Colour auto-detects a TTY.
  Terminal-injection-safe: source text is routed through `display_safe`, so the
  only raw escapes are fixed SGR codes.
- **Structural diff** — `structural <old> <new> [--json]` (+
  `syntax::structural`, schema `deep-diff-forge.structural.v0`): a token/leaf-level
  (AST) diff over tree-sitter token streams. **Reformat-aware** (layout-only edits
  report zero structural change) with best-effort moved-block detection; never
  touches patch truth. Bounded LCS with graceful degradation on huge inputs.
- **Learning (L9)** — `deep-diff-forge-learning`: the local-only learning loop.
  Per-decision [`StrategyReceipt`]s (hashes/counts/timings — never a path or
  source line) are appended as JSONL under `$XDG_STATE_HOME/deep-diff-forge/
  learning/`, scored per strategy, and gated for promotion. Three learning units
  — planner (strategy + budget), ranking (risk weights), annotation (agent trust
  tiers, *untrusted until grounded*) — plus an explainable promotion gate. CLI
  `learn {status|record --stdin} [--json]`. 135 tests; never uploads, never
  mutates patch truth.

### Changed

- **Daemon socket handling** (Zen review) — replaced scattered `Option<PathBuf>`
  resolution with a parse-don't-validate `SocketLocation` type: one
  `SocketLocation::resolve(explicit)` entry point (explicit `--socket` → else
  `$XDG_RUNTIME_DIR` → else a typed `NoRuntimeDir`, no `/tmp` fallback), with
  `bind`/`connect`/`path` as methods. A "no location" state is unrepresentable
  past construction, and the CLI's nested `if let` collapses to one call.
- **Publishable-workspace manifests** — versions and internal deps are now
  inherited via `[workspace.package]` + `[workspace.dependencies]`; every crate
  carries `description`, `keywords`, `categories`, and `homepage`. `core` and
  `cli` gained the `description` that `cargo publish` requires.

### Fixed

- **Patch truth: malformed/truncated hunks are now rejected** (Zen review) — the
  parser enforces each hunk's declared `@@ -a,b +c,d @@` counts exactly. A hunk
  that ends (at a new header, a new file, or EOF) before its counts are satisfied
  is a `TruncatedHunk` error; a body line that over-fills a side is a
  `HunkLineCountMismatch`. An internally-incoherent patch no longer parses as
  apply-able truth.
- **Broken-pipe handling** — bulk CLI output (`--stdin-patch`, `--rank`,
  `--cluster`, `--jsonl`, `semantic`, `review --probe`, `learn`) is routed
  through a broken-pipe-tolerant writer, so `deep-diff-forge … | head` exits `0`
  instead of panicking with `EPIPE` (exit 101). Resolves the L1-era open finding;
  std-only, no `unsafe`.

### Security

Pre-publication adversarial hardening (S1008412): an 8-dimension STRIDE review
with independent verification produced a CVSS-scored register of 17 confirmed
findings (4 MEDIUM, 12 LOW, 1 INFO; **no Critical/High**), all remediated with
fail-before/pass-after regression tests. A later final-hardening review fleet
(5 reviewers + adversarial verify) added the items below. See `SECURITY.md`.

- **Trojan Source defence (CVE-2021-42574)** — `display_safe` now also escapes
  bidirectional/invisible Unicode (`U+202A–202E`, `U+2066–2069`, directional
  marks, zero-width, BOM) to a visible `\u{XXXX}`, so attacker source cannot
  display differently than it reads — on-mission for a review tool.
- **Cross-platform contract made honest** — the UDS daemon is Unix-only; it now
  emits a clear `compile_error!` on non-Unix instead of a cryptic `std::os::unix`
  failure, and the release matrix dropped its (un-buildable) Windows lane so CI
  and code agree before the irreversible publish (Linux + macOS).
- **Broken-pipe fix completed** — the remaining `deploy`/`doctor`/`--help`/
  contract commands (notably the `--json` outputs CI/agents pipe to `head`/`jq`)
  now route through the broken-pipe-tolerant writer; `… | head` exits `0`, not 101.
- **`structural` on a non-Rust file** now errors explicitly instead of silently
  reporting "formatting only"; **`doctor`** resolves both its runtime-dir and
  socket lines through the daemon's `var_os` resolver (consistent on non-UTF-8 env).
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

A second bias-controlled re-review (S1008443) raised three further findings, each
fixed and independently verified (security / tester / claim-verifier judges,
outside the build loop):

- **Mandatory hunk-header closer** — `parse_hunk_header` now requires the closing
  `@@` token; a header with trailing garbage and no closer (`@@ -1,1 +1,1
  NOT_A_HUNK`) is rejected (exit 4) instead of normalizing into a valid model.
  Completes the patch-truth hardening alongside the truncated-hunk fix.
- **Explicit `--socket` no longer mutates caller state** — `SocketLocation` carries
  provenance; an explicit `--socket` path binds via a fail-closed `bind_explicit`
  that **creates** an absent parent at `0700` but **never chmods a pre-existing
  parent** (a world-readable parent fails closed rather than being silently
  tightened to `0700`) and removes the socket path **only if it is verifiably a
  socket** (never a regular file, directory, or symlink). The engine-owned
  `$XDG_RUNTIME_DIR` path is unchanged, and `daemon start --socket /new/path` still
  works (the absent parent is created).
- **Bounded daemon sessions** — review sessions are capped at `MAX_SESSIONS` (64)
  with LRU eviction (least-recently-used evicted before insert; reads refresh
  recency), hard-bounding daemon memory regardless of how many sessions a client
  opens without closing.

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
