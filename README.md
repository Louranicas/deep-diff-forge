# Deep-Diff-Forge

**A next-generation review engine for code changes — patch truth and semantic
intent, together.**

Deep-Diff-Forge is a Rust CLI (and optional daemon) that treats a diff not as a
blob of `+`/`-` lines but as a *review*: it preserves the exact, apply-able patch
while layering syntax-aware understanding, risk ranking, agent annotations, an
interactive terminal cockpit, and bounded parallel execution on top — every
layer a projection over one stable model, none of them ever allowed to corrupt
the patch.

> **Maturity: L9 (Learning).** All engine layers L0–L8 are implemented, plus the
> L9 local-only learning loop (`learn status|record`). 12 crates, 754 tests, zero
> `unsafe`, supply-chain-gated, dual MIT/Apache-2.0 licensed. The workspace is
> **crates.io-publish-ready** (`cargo publish --dry-run` is clean across all
> crates); the upload itself is **token-gated** — the release workflow publishes
> automatically when `CARGO_REGISTRY_TOKEN` is configured. See
> [`EVIDENCE.md`](EVIDENCE.md) and
> [`CHANGELOG.md`](CHANGELOG.md).

---

## Table of contents

- [Why Deep-Diff-Forge](#why-deep-diff-forge)
- [The first principle](#the-first-principle)
- [Three pioneer features](#three-pioneer-features)
- [Feature comparison](#feature-comparison)
- [Install & build](#install--build)
- [Quick start](#quick-start)
- [Command reference](#command-reference)
- [Output formats & schemas](#output-formats--schemas)
- [Exit codes](#exit-codes)
- [The optional daemon](#the-optional-daemon)
- [Risk ranking signals](#risk-ranking-signals)
- [Architecture](#architecture)
- [The deployment framework](#the-deployment-framework)
- [Building, testing, and quality gates](#building-testing-and-quality-gates)
- [Project status](#project-status)
- [License](#license)

---

## Why Deep-Diff-Forge

Existing tools each optimize one layer of the review problem:

| Tool | Optimizes | Limit |
| --- | --- | --- |
| classic `diff` | patch truth, exit codes, automation | no semantic or review intelligence |
| `delta`, `diff-so-fancy` | readable terminal rendering | line-oriented; pretty is not understanding |
| `difftastic` | structural (syntax-tree) diffing | output is not apply-able as a patch |
| `hunk` | review-first UI + AI workflow | broad desktop surface, not a composable core |
| `lumen` | interactive viewer ergonomics | viewer, not an engine |

Deep-Diff-Forge wins by making these layers **cooperate** instead of choosing
one: a conservative, apply-able core with ambitious, clearly-separated
enrichment on top. It is built for humans reviewing AI-generated changes across
many files — and for the agents, scripts, and CI that increasingly drive review.

It is **Bash-first and Claude-Code-first**: every action has a deterministic
command, machine-readable output (`--json` / `--jsonl`), stable exit codes, and
works as a Unix filter with no daemon required.

## The first principle

> **A diff engine must preserve patch truth while exposing semantic intent.**

Patch truth (the exact text that can be applied) is sacred and *separable* from
every enrichment layer. Semantic analysis, risk ranking, and AI annotations may
be absent, partial, or wrong — they can never mutate the apply-able patch. This
single invariant is enforced everywhere: the parser, the projections, the
ranking, the annotation layer, and the cluster scheduler all read the model;
none rewrite it.

## Three pioneer features

1. **Semantic Patch Twin** — every change carries two synchronized
   representations: an apply-able *patch twin* and a syntax *semantic twin*,
   joined by stable anchors. Switch views without losing line anchors, comments,
   or applicability.

2. **Review Intelligence Graph** — a deterministic, explainable risk ranking
   that orders the review stream by likely impact (public-API surface, change
   size, new/deleted/binary, generated-file suppression, test de-prioritization)
   rather than raw file order. (`--rank`)

3. **Adaptive Diff Planner** — per-file/per-region strategy selection with
   explained, budgeted, conservative fallback. (Strategy vocabulary is modeled
   today; semantic strategy selection grows as the Git-input wave feeds file
   bytes.)

---

## Feature comparison

Deep-Diff-Forge is a review **engine**, not a syntax-highlighting pager — so it
brings capabilities the rendering-focused tools don't, and (honestly) doesn't yet
do everything they do. The table reflects default, out-of-the-box behavior.

| Capability | deep-diff-forge | [`delta`][delta] | [`difftastic`][difft] | [`diff-so-fancy`][dsf] | `diff` |
| --- | :---: | :---: | :---: | :---: | :---: |
| Review-first interactive UI | ✅ | ❌ | ❌ | ❌ | ❌ |
| Multi-file review stream + sidebar | ✅ | ❌ | ❌ | ❌ | ❌ |
| Inline agent / AI annotations | ✅ | ❌ | ❌ | ❌ | ❌ |
| Runtime view toggles (inline ↔ side-by-side) | ✅ | ❌ | ❌ | ❌ | ❌ |
| Deterministic risk ranking | ✅ | ❌ | ❌ | ❌ | ❌ |
| Machine-readable JSON / JSONL output | ✅ | ❌ | ❌ <sup>1</sup> | ❌ | ❌ |
| Optional shared-cache daemon (UDS) | ✅ | ❌ | ❌ | ❌ | ❌ |
| Local, private learning loop | ✅ | ❌ | ❌ | ❌ | ❌ |
| Apply-able patch output preserved | ✅ | ✅ <sup>2</sup> | ❌ | ✅ <sup>2</sup> | ✅ |
| Syntax highlighting | ✅ <sup>3</sup> | ✅ | ✅ | ❌ | ❌ |
| Structural / AST-level diffing | ✅ <sup>4</sup> | ❌ | ✅ | ❌ | ❌ |
| Pager / Unix-filter friendly | ✅ | ✅ | ✅ | ✅ | ✅ |

<sup>1</sup> difftastic has an experimental JSON display; deep-diff-forge ships a
stable, schema-versioned `--json` plus streaming `--jsonl`.
<sup>2</sup> `delta` / `diff-so-fancy` re-render an underlying Git diff that
remains apply-able; they do not alter patch truth.
<sup>3</sup> Tree-sitter syntax highlighting (`highlight`), reusing the grammar's
own queries and **terminal-injection-safe** (attacker source can't smuggle
escapes). Rust today; more languages as grammars are added.
<sup>4</sup> Token/leaf-level structural diff (`structural`): **reformat-aware**
(layout-only changes report zero structural change) with best-effort moved-block
detection. difftastic's optimal tree-edit-distance graph diff is deeper.

> Deep-Diff-Forge is optimized for reviewing a whole changeset **interactively**
> and for **agent-collaborative, machine-readable** review. Comparisons are
> best-effort against each tool's default behavior — corrections welcome via PR.

[delta]: https://github.com/dandavison/delta
[difft]: https://github.com/Wilfred/difftastic
[dsf]: https://github.com/so-fancy/diff-so-fancy

---

## Install & build

Requires Rust **1.85+** (edition 2024). The build is pinned to a repo-local
target directory.

```bash
git clone https://github.com/Louranicas/deep-diff-forge.git
cd deep-diff-forge

# Build the release binary (repo-local target/)
CARGO_TARGET_DIR=target cargo build --release -p deep-diff-forge-cli

# The binary:
target/release/deep-diff-forge --version
```

If you have [`just`](https://github.com/casey/just):

```bash
just gate-feature      # full quality gate: fmt, check, clippy, pedantic, test, contracts
just contracts         # run the bootstrap contract probes
just status            # repo identity + metadata
```

Optional convenience aliases:

```bash
alias ddf='deep-diff-forge'
git config --global alias.review '!git diff | deep-diff-forge --stdin-patch --rank'
```

## Quick start

```bash
# Human-readable review summary of your working tree
git diff | deep-diff-forge --stdin-patch

# Risk-ranked review: what to look at first
git diff | deep-diff-forge --stdin-patch --rank

# Side-by-side view
git diff | deep-diff-forge --stdin-patch --layout side-by-side

# Machine-readable review document for an agent / CI
git diff | deep-diff-forge --stdin-patch --json

# Symbols of a source file (tree-sitter)
deep-diff-forge semantic src/lib.rs

# Interactive review cockpit
git diff | deep-diff-forge review
```

---

## Command reference

The primary input today is a unified/Git patch on **stdin** (`--stdin-patch`).
Pipe `git diff`, a `.patch` file, or any unified diff into it.

### `--stdin-patch` — review a patch

```bash
deep-diff-forge --stdin-patch [MODE]
```

| Mode (flag) | Output |
| --- | --- |
| *(none)* | Human review summary: one line per file with `+adds -dels`, hunk count, status. |
| `--json` | One complete `deep-diff-forge.review.v0` JSON document (files, hunks, line anchors, metadata, summary). |
| `--jsonl` | One JSON event per file (`{"event":"diff.file",…}`), newline-delimited — streamed through the real pipeline runner. |
| `--rank` | Risk-ranked review stream (highest-impact first). Add `--json` for `deep-diff-forge.rank.v0`. |
| `--cluster [--parallel serial\|auto\|N]` | Same ranking, computed via bounded parallel lanes with a deterministic join + a receipt. Add `--json` for `deep-diff-forge.cluster.v0`. |
| `--layout inline` | Inline projection with old/new line numbers and markers. |
| `--layout side-by-side` | Two-column old-vs-new projection with a gutter. |

Examples:

```bash
git diff HEAD~3 | deep-diff-forge --stdin-patch
git diff | deep-diff-forge --stdin-patch --json   > review.json
git diff | deep-diff-forge --stdin-patch --jsonl  | while read -r ev; do echo "$ev"; done
git diff | deep-diff-forge --stdin-patch --rank --json
git diff | deep-diff-forge --stdin-patch --cluster --parallel 4 --json
```

### `semantic <path>` — tree-sitter symbols

Parse a source file and report its top-level symbols (functions, structs, enums,
traits, impls, modules, consts, …) with line ranges and a parse status.

```bash
deep-diff-forge semantic crates/deep-diff-forge-core/src/lib.rs
deep-diff-forge semantic src/lib.rs --json     # deep-diff-forge.semantic.v0
```

Supported language today: **Rust** (extensible via the tree-sitter registry).
Unsupported extensions degrade with an explicit `fallback:UnsupportedLanguage`,
never a guess. Parsing is byte- and node-budgeted; a malformed file reports
`parsed_with_errors:N` rather than failing.

### `highlight <path>` — syntax highlighting

Print a source file with tree-sitter syntax highlighting, reusing the grammar's
own `highlights.scm` queries.

```bash
deep-diff-forge highlight src/lib.rs               # colour when stdout is a TTY
deep-diff-forge highlight src/lib.rs --color       # force ANSI
deep-diff-forge highlight src/lib.rs --no-color     # plain (still sanitised)
```

Colour is automatic on a terminal and off when piped. Either way the output is
**terminal-injection-safe**: source text is routed through the control-char
sanitiser, so the only raw escapes are fixed SGR colour codes — a hostile file
cannot hijack your terminal.

### `structural <old> <new>` — token-level structural diff

Diff two files by their tree-sitter **token streams** instead of raw lines, so
the result is **reformat-aware**: a change that only reflows whitespace reports
zero structural change.

```bash
deep-diff-forge structural old.rs new.rs            # human summary
deep-diff-forge structural old.rs new.rs --json     # deep-diff-forge.structural.v0
```

Reports added / removed / unchanged tokens, with best-effort moved-block
detection. This is a token/leaf-level diff (not difftastic's optimal
tree-edit-distance), and it never touches patch truth — it only describes source.

### `review [--probe]` — interactive review cockpit

```bash
git diff | deep-diff-forge review            # launch the TUI (needs a terminal)
git diff | deep-diff-forge review --probe    # render one frame headlessly (CI/agents, no TTY)
```

The TUI shows a risk-ranked file sidebar and a detail pane. Keys:

| Key | Action |
| --- | --- |
| `j` / `↓` | next file |
| `k` / `↑` | previous file |
| `g` / `Home` · `G` / `End` | first / last file |
| `t` / `Tab` | toggle inline ↔ side-by-side layout |
| `Ctrl-d` / `PageDown` · `Ctrl-u` / `PageUp` | scroll detail down / up |
| `q` / `Esc` | quit |

`--probe` renders a single frame to stdout via a headless backend — useful for
snapshots, CI, and agents that cannot attach a terminal.

### `deploy status` — machine-readable deployment state

```bash
deep-diff-forge deploy status            # human
deep-diff-forge deploy status --json     # deep-diff-forge.deployment-status.v0
```

Reports the declared maturity level, the gate stack, and external-observer
posture so CI and orchestration can consume deployment state instead of scraping
prose.

### `daemon` — optional UDS JSON-RPC service

```bash
deep-diff-forge daemon path                      # print the socket path
deep-diff-forge daemon start --foreground        # serve (owner-private UDS)
deep-diff-forge daemon health                     # query a running daemon
deep-diff-forge daemon status
deep-diff-forge daemon stop
# all accept: --socket <PATH>
```

See [The optional daemon](#the-optional-daemon).

### `learn` — local-only learning loop (L9)

Deep-Diff-Forge improves through measured review outcomes. The loop records a
local-only receipt per planner decision (hashes, counts, timings — never a path
or source line), scores each strategy, and gates whether a learned default may
be promoted. Nothing is uploaded; nothing mutates patch truth.

```bash
deep-diff-forge learn status                 # store path, receipt count, scores, trust verdict
deep-diff-forge learn status --json          # deep-diff-forge.learning.v0
echo '<receipt-json>' | deep-diff-forge learn record --stdin   # feed one receipt (agents/CI)
```

Receipts live under `$XDG_STATE_HOME/deep-diff-forge/learning/` (falling back to
`~/.local/state/...`). A fresh machine with no store reports zero receipts and no
trusted default — never an error.

### Diagnostics & contracts

```bash
deep-diff-forge --help
deep-diff-forge --version
deep-diff-forge --self-test            # core model smoke check
deep-diff-forge doctor                 # runtime/cache/state/socket paths
deep-diff-forge claude-code-contract   # agent-facing output guarantees
deep-diff-forge chain-contract         # Unix-filter chaining guarantees
deep-diff-forge cluster-contract       # parallel execution guarantees
deep-diff-forge loom-contract          # assimilation-pipeline guarantees
```

---

## Output formats & schemas

Every machine mode emits a versioned schema string so consumers can rely on
stable fields. `--json` is one complete document; `--jsonl` is one event per
line. Primary output goes to **stdout**; diagnostics to **stderr**.

| Command | Schema |
| --- | --- |
| `--stdin-patch --json` | `deep-diff-forge.review.v0` |
| `--stdin-patch --jsonl` | line events: `{"event":"diff.file",…}` |
| `--stdin-patch --rank --json` | `deep-diff-forge.rank.v0` |
| `--stdin-patch --cluster --json` | `deep-diff-forge.cluster.v0` |
| `semantic --json` | `deep-diff-forge.semantic.v0` |
| `structural --json` | `deep-diff-forge.structural.v0` |
| `deploy status --json` | `deep-diff-forge.deployment-status.v0` |
| `learn status --json` | `deep-diff-forge.learning.v0` |
| `daemon …` | JSON-RPC 2.0 |

Example — `--rank --json`:

```json
{
  "schema": "deep-diff-forge.rank.v0",
  "ranked": [
    {"path": "src/lib.rs", "status": "modified", "score": 7, "signals": ["public_api_surface"]},
    {"path": "tests/it.rs", "status": "modified", "score": 1, "signals": ["test_only"]}
  ]
}
```

Example — `--cluster --json` (note the receipt):

```json
{
  "schema": "deep-diff-forge.cluster.v0",
  "receipt": {"dimensions": ["patch", "risk"], "parallelism": "fixed:4", "workers": 4, "join_policy": "ranked-review-order", "file_count": 2},
  "ranked": [ /* … */ ]
}
```

JSON strings are RFC-8259 escaped; UTF-8 passes through. File statuses use a
single canonical snake-case spelling (`added`, `modified`, `deleted`,
`renamed`, `type_changed`, `binary_changed`, `unknown`) across every surface.

## Exit codes

| Code | Meaning |
| --- | --- |
| 0 | Success. |
| 2 | CLI usage / argument error. |
| 3 | Input (stdin or file) read failure. |
| 4 | Patch parse failure. |
| 6 | Daemon / interactive-terminal failure. |

Diagnostics never pollute stdout: on error, stdout stays empty and the message
goes to stderr.

## The optional daemon

The daemon accelerates repeated review and multi-client workflows. It is
**never required** for one-shot CLI correctness — every command works without
it. It is **std-first** (no async runtime): a `UnixListener` JSON-RPC 2.0 server
over an owner-private Unix domain socket.

**Security:** the runtime directory is created `0700` (and rejected if group- or
world-accessible), the socket is `0600`, and stale sockets are replaced on bind.

**Default socket:** `$XDG_RUNTIME_DIR/deep-diff-forge/deep-diff-forge.sock`. There
is no world-writable `/tmp` fallback: if `$XDG_RUNTIME_DIR` is unset the daemon
fails closed and you pass an explicit `--socket PATH`. (`doctor` reports the
resolved socket, or `<unavailable: set XDG_RUNTIME_DIR or pass --socket>`.)

**JSON-RPC methods:** `engine.initialize`, `daemon.health`, `daemon.status`,
`daemon.shutdown`, `diff.plan`, `session.open`, `session.snapshot`,
`session.close`.

```bash
# Terminal 1
deep-diff-forge daemon start --foreground

# Terminal 2
deep-diff-forge daemon health
# {"id":1,"jsonrpc":"2.0","result":{"status":"ok","pid":…,"protocol":0,"sessions":0,…}}
deep-diff-forge daemon stop
```

## Risk ranking signals

`--rank` / `--cluster` compute a deterministic, explainable score per file.
Higher = review first. Signals:

| Signal | Effect |
| --- | --- |
| `public_api_surface` | `lib.rs` / `mod.rs` / `…/api/…` — strong boost |
| `large_change` | ≥ 80 changed lines |
| `many_hunks` | ≥ 5 hunks |
| `new_file` / `deleted_file` | added / removed file |
| `binary_change` | binary file (no reviewable text) |
| `config_or_lockfile` | `Cargo.toml`, `*.lock`, `*.yaml`, … |
| `test_only` | de-prioritized below equivalent source |
| `generated_or_vendored` | `vendor/`, `node_modules/`, `target/`, `*.min.js`, … — suppressed to 0 |

Ranking is reproducible (a path tie-break makes the order stable) and, under
`--cluster`, **identical for any worker count** — parallelism never changes the
result.

---

## Architecture

Eleven narrow crates with strictly acyclic, inward dependency flow. `core` is
pure vocabulary (no I/O, no parsing); every feature crate depends on `core`,
never the reverse. Patch truth is upstream of everything.

| Crate | Role |
| --- | --- |
| `deep-diff-forge-core` | Stable model: IDs, patch/semantic twins, planner & graph vocabulary, deployment types, `json_escape`. |
| `deep-diff-forge-patch` | Unified/Git patch parser, apply-able renderer, `review.v0` JSON. |
| `deep-diff-forge-projection` | Renderer-neutral inline & side-by-side projections. |
| `deep-diff-forge-pipeline` | Composable Unix-filter stages (`ChainStage`, ingest/render), JSONL. |
| `deep-diff-forge-syntax` | Tree-sitter language detection, budgeted parse, symbol extraction. |
| `deep-diff-forge-graph` | Review Intelligence Graph — deterministic risk ranking. |
| `deep-diff-forge-agent` | Annotation provenance, grounding classification, sanitization, anchor validation. |
| `deep-diff-forge-tui` | Review-first terminal UI (ratatui), tested headlessly. |
| `deep-diff-forge-cluster` | Bounded parallel dimensional execution + deterministic joins + receipts. |
| `deep-diff-forge-learning` | L9 local-only learning loop: strategy receipts, scoring, gated promotion. |
| `deep-diff-forge-daemon` | Optional UDS JSON-RPC service (std-first). |
| `deep-diff-forge-cli` | Thin command entry point over the above. |

### Maturity ladder

The codebase advances through declared maturity levels; each is gated and sealed.

```
L0 Bootstrap → L1 Patch → L2 Projection → L3 Pipeline → L4 Semantic
   → L5 Review → L6 Cluster → L7 Daemon → L8 Release → [L9 Learning]
```

**L0–L9 are shipped; `v0.2.0` adds the L9 learning loop and the first
crates.io-publishable cut.** The workspace passes `cargo publish --dry-run`
across every crate; the crates.io upload is token-gated (the release workflow
publishes when `CARGO_REGISTRY_TOKEN` is configured). The learning loop is
local-only and runs without any deployed daemon.

## The deployment framework

Deep-Diff-Forge is developed against an explicit, receipt-backed deployment
framework — the codebase is the source of truth, and docs are binding only until
code implements them, after which code wins or the gate fails.

- **[`docs/DEPLOYMENT_FRAMEWORK.md`](docs/DEPLOYMENT_FRAMEWORK.md)** — the
  governing document: source-of-truth order, deployment modes, the 11-gate
  stack, receipts, maturity ladder, and a bidirectional map to every other doc.
- **[`docs/DEPLOYMENT_GAP_ANALYSIS.md`](docs/DEPLOYMENT_GAP_ANALYSIS.md)** —
  codebase + non-anthropocentric gap passes.
- **[`docs/MODULE_STRUCTURE_PLAN.md`](docs/MODULE_STRUCTURE_PLAN.md)** — the
  crate/module/dependency plan.
- **[`docs/TESTING_GOLD_STANDARD.md`](docs/TESTING_GOLD_STANDARD.md)** — the
  50-meaningful-tests rule and anti-test-fitting discipline.
- **[`docs/AGENTIC_RUST_CODER_V4.md`](docs/AGENTIC_RUST_CODER_V4.md)** — the
  evidence-labelled implementation standard.
- See the framework's documentation map for the full set (architecture, specs,
  API/IPC, chaining/clustering, loom, performance, release, operations, …).

## Building, testing, and quality gates

The mandatory gate (zero tolerance at every stage):

```bash
just gate-feature
# = cargo fmt --check
#   cargo check --workspace
#   cargo clippy --workspace --all-targets -- -D warnings
#   cargo clippy --workspace --all-targets -- -D warnings -W clippy::pedantic
#   cargo test --workspace --locked
#   bootstrap contract probes
```

Standards enforced across the tree: **754 tests** (every production crate ≥ 50
meaningful tests), **zero `unsafe`** (compiler-forbidden workspace-wide via
`[workspace.lints]`), no production `unwrap`/`expect`, pedantic clippy clean with
no unexplained suppressions, and a `cargo-deny` ([`deny.toml`](deny.toml)) +
strict `cargo-audit` supply-chain gate (advisories, licenses, bans, sources). CI
mirrors the gate in [`.github/workflows/ci.yml`](.github/workflows/ci.yml).

The engine is designed to be run on **untrusted input** (an attacker controls the
whole diff); it has been adversarially hardened against terminal-escape injection,
memory-exhaustion DoS, and the daemon's transport/filesystem surface. See
[`SECURITY.md`](SECURITY.md) for the threat model and disclosure policy.

Durable engineering lessons are recorded in [`NOTES.md`](NOTES.md).

## Project status

This is an actively-built engine at **L9 maturity**. Everything documented above
is implemented, gated, and live-proven. Honest current limitations:

- Full **patch↔symbol join** (mapping a hunk to its enclosing semantic symbol in
  a diff) awaits a Git-input layer that supplies file bytes; `semantic <file>`
  proves the engine on whole files today, and `enclosing_symbol` is the ready
  building block.
- The daemon serves connections **sequentially**; a thread-per-connection /
  async upgrade is deferred until a measured need.
- **L9 Learning**: the learning loop records and scores receipts and gates
  promotion; wiring the engine's hot path to *emit* receipts automatically (vs.
  the explicit `learn record`) lands as live signal accrues.
- **Release**: `v0.2.0` is publish-ready (clean `cargo publish --dry-run` across
  the workspace); the **crates.io** upload is gated on a registry token.
- Time-budget enforcement in the semantic layer is deferred (and never reported
  as a fallback).

## License

Licensed under either of **MIT** or **Apache-2.0** at your option.

---

- **GitHub:** https://github.com/Louranicas/deep-diff-forge
- **Deployment framework:** [`docs/DEPLOYMENT_FRAMEWORK.md`](docs/DEPLOYMENT_FRAMEWORK.md)
