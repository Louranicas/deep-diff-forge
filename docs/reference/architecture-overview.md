# Architecture

There are 14 narrow crates with one-way dependencies. `core` is pure vocabulary, with no I/O
and no parsing. Every crate except `learning` depends on `core`, and patch truth is upstream
of everything.

```mermaid
flowchart LR
  subgraph surfaces[" "]
    cli[cli]
    tui[tui]
    sdk[sdk]
  end
  cli --> tui & pipeline & cluster & daemon & review & learning & agent & syntax & projection & graph & patch
  tui --> agent & cluster & daemon & learning & syntax & projection & graph & patch
  sdk --> graph & patch
  sdk -. "cfg(unix)" .-> daemon
  pipeline --> syntax & projection & graph & patch
  cluster --> graph
  daemon --> agent & graph & patch
  core((core))
  patch --> core
```

<sub>Drawn from <a href="../crate-edges.txt"><code>docs/crate-edges.txt</code></a>, which a test checks against <code>cargo metadata</code>. Edges into <code>core</code> are left out except one: every crate but <code>learning</code> has one. Dev-only edges are omitted.</sub>

| Crate | Role |
| --- | --- |
| `deep-diff-forge-core` | The stable model: IDs, patch and semantic twins, planner and graph vocabulary, the exit-code registry, release and deployment types. |
| `deep-diff-forge-patch` | Strict unified/Git patch parser, `SealedReview`, Git acquisition, apply-able renderer, `review.v0` JSON. |
| `deep-diff-forge-projection` | Renderer-neutral inline and side-by-side projections. |
| `deep-diff-forge-pipeline` | Composable Unix-filter stages and JSONL. |
| `deep-diff-forge-syntax` | Tree-sitter detection, budgeted parsing, symbols, highlighting, token-level structural diff. |
| `deep-diff-forge-graph` | The review intelligence graph: deterministic risk ranking and lexical code evidence. |
| `deep-diff-forge-agent` | Annotation provenance, grounding, sanitisation, anchor validation, agent-findings verification. |
| `deep-diff-forge-cluster` | Bounded parallel lanes, deterministic joins, receipts. |
| `deep-diff-forge-learning` | Local-only learning loop: receipts, scoring, gated promotion. |
| `deep-diff-forge-daemon` | The optional Unix-socket JSON-RPC service and its shared request handler. |
| `deep-diff-forge-tui` | The review cockpit (ratatui), tested headlessly. |
| `deep-diff-forge-review` | The review council: captured rosters, challenge rounds, sandboxed evidence gates. |
| `deep-diff-forge-sdk` | The stable Rust facade and daemon client. |
| `deep-diff-forge-cli` | The `deep-diff-forge` binary, plus the MCP, `rpc` and `envelope` surfaces. |

Deeper: [`docs/ARCHITECTURE.md`](../ARCHITECTURE.md) and [`docs/SCHEMATICS.md`](../SCHEMATICS.md).

See also: [`ARCHITECTURE`](../ARCHITECTURE.md) · [`SCHEMATICS`](../SCHEMATICS.md).
