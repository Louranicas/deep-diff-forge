# Architecture

The workspace has **15 narrow crates** (**14 published** plus the dev-only, unpublished
`testkit` crate) with one-way dependencies. `core` is pure vocabulary, with no I/O and no
parsing. Every published crate except `learning` depends on `core`, and patch truth is upstream
of everything.

```mermaid
flowchart LR
  subgraph surfaces["Surfaces"]
    cli["cli"]
    tui["tui"]
    sdk["sdk"]
  end
  cli --> tui & pipeline & cluster & daemon & review & learning & agent & syntax & projection & graph & patch
  tui --> agent & cluster & daemon & learning & syntax & projection & graph & patch
  sdk --> graph & patch
  sdk -- "cfg(unix)" --> daemon
  pipeline --> syntax & projection & graph & patch
  cluster --> graph
  daemon --> agent & graph & patch
  core(("core"))
  patch --> core
```

<sub>Drawn from <a href="../crate-edges.txt"><code>docs/crate-edges.txt</code></a>, which a test checks against <code>cargo metadata</code>. Normal edges into <code>core</code> are left out except one: every published crate except <code>learning</code> has one. Dev-only edges and the dev-only <code>testkit</code> crate are omitted from the diagram.</sub>

| Crate | Role at `v0.11.0` |
| --- | --- |
| `deep-diff-forge-core` | Stable model (`ReviewFile`, hunks, anchors), `InputSeal` (FIPS 180-4 SHA-256 & RFC 2104 HMAC-SHA-256), exit-code & exit-7 refusal registry, terminal/JSON safety, release and deployment types. |
| `deep-diff-forge-patch` | 42-shape Git header conformance parser, `SealedReview`, `InputGuards`, `--raw -z` Git acquisition (`diff_suppressed`), `<old> <new>` `diff_files`, scratch-dir `judge_patch`, renderer, `review.v0` JSON. |
| `deep-diff-forge-projection` | Renderer-neutral inline and side-by-side projections, `\` note rows, grapheme cell-width measurement. |
| `deep-diff-forge-pipeline` | Composable Unix-filter stages (`--jsonl`) and the shared source-pair `analyze` service. |
| `deep-diff-forge-syntax` | Budgeted tree-sitter Rust parser (byte, node, and wall-clock caps), symbols, terminal-safe ANSI highlight, token-level `structural_diff`. |
| `deep-diff-forge-graph` | Dual-reader (`rust_tokens` + `lexical`) 27-signal risk ranking (`rank.v0` + build-time `tool.scorer_sha256`) and `diagnostics.v0` (`<label>@line`, `<label>@file`, repairs). Depends on `core` only (no `syntax` edge). |
| `deep-diff-forge-agent` | Annotation provenance, grounding, sanitization, anchor validation, and `findings.v0` verification. |
| `deep-diff-forge-cluster` | Bounded `std::thread::scope` parallel `patch` and `risk` lanes, deterministic `ranked-review-order` join, and receipts. |
| `deep-diff-forge-learning` | Local-only redacted strategy receipts (`learn record\|status`), `TrustPolicy`, and library-level learners/promotion gates. |
| `deep-diff-forge-daemon` | Optional Unix-socket JSON-RPC 2.0 server and shared dispatcher (Engine Protocol 3, 17 methods, 128 MiB FIFO admission, 64 LRU sessions, `cache_entries: 0`, byte-exact paging, Prometheus/JSON metrics). |
| `deep-diff-forge-tui` | Interactive review cockpit (`ratatui`/`crossterm`), mouse + keyboard, verified `--findings`, tested headlessly (`--probe`). |
| `deep-diff-forge-review` | Adversarial review council (6 slots, 2 rounds, 8 rootless Podman gates), `tuicr` interop, append-only hash-chained `decisions.jsonl` recovery (council exit 3). |
| `deep-diff-forge-sdk` | Stable Rust facade (`review`, `rank`, `diagnostics`, `exit`) and Unix daemon `Client` (`ProtocolTooOld`, `UnknownMembers`). |
| `deep-diff-forge-cli` | The `deep-diff-forge` binary: CLI review/gate flags, `rpc`, dual-era 9-tool `mcp`, `envelope`/`schema`/`explain`, `agent` hooks (state v13, HMAC-SHA-256, ledger), `eval` (73 cases), `bench`, `council`, `deploy`. |
| `deep-diff-forge-testkit` | Dev-only, unpublished workspace test helper (`Scratch` owner-private `0700` temp directories). |

Deeper: [`docs/ARCHITECTURE.md`](../ARCHITECTURE.md) and [`docs/SCHEMATICS.md`](../SCHEMATICS.md).

See also: [`ARCHITECTURE`](../ARCHITECTURE.md) · [`SCHEMATICS`](../SCHEMATICS.md).
