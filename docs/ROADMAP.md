# Roadmap

## Phase 0: Vocabulary and Contracts

- Define patch truth model.
- Define semantic twin model.
- Define review graph primitives.
- Define planner strategy and fallback reasons.
- Define CLI shape.
- Define API and IPC contracts.
- Define socket locations and daemon lifecycle.
- Define performance budgets and fallback taxonomy.
- Define Claude Code, Bash, JSON, JSONL, and exit-code contracts.
- Define chain, cluster, dimensional execution, and loom assimilation contracts.
- Ship bootstrap smoke commands: `--self-test`, `doctor`, `claude-code-contract`, `chain-contract`, `cluster-contract`, and `loom-contract`.

## Phase 1: Patch Baseline

- Parse unified patches.
- Render unified patches.
- Build side-by-side row projection.
- Preserve metadata: modes, renames, binary markers, no-newline lines.
- Add JSON output.
- Add Bash-safe `--stdin-patch`, `--json`, `--jsonl`, `--plain`, and `--exit-code`.
- Add `deep-diff-forge-pipeline` with ingest, plan, rank, and render stage contracts.

## Phase 2: Pager and Terminal Review

- Add CLI input modes for file pair, directory pair, stdin patch, and Git external diff.
- Add syntax highlighting.
- Add keyboard and mouse TUI navigation.
- Add responsive side-by-side/stacked/inline projections.

## Phase 3: Structural Diff

- Add tree-sitter registry.
- Add syntax fallback budget: bytes, parse errors, node count, elapsed time.
- Add structural spans synchronized to patch hunks.
- Add moved-block and reformat-aware matching.

## Phase 4: Review Intelligence Graph

- Rank files/hunks by risk.
- Attach symbols, tests, owners, comments, commands, and agent notes.
- Add generated/vendor detection.
- Add review state and decision export.

## Phase 5: Agent Collaboration

- Add annotation API.
- Add provenance model.
- Add agent review requests.
- Add grounded/ungrounded claim display.
- Add command/test evidence linking.

## Phase 6: App Surfaces

- TUI production hardening.
- Desktop/web renderer model.
- IDE integration adapters.
- Forge integration.

## Phase 7: Shared Cache Daemon

- Add optional Unix domain socket daemon.
- Add JSON-RPC session API.
- Add AST and line-index cache.
- Add multi-client subscriptions.
- Add secure socket ownership checks.

## Phase 8: Cluster And Loom

- Add `deep-diff-forge-cluster`.
- Add dimensional sharding and join policies.
- Add local parallel execution receipts.
- Add `deep-diff-forge-loom`.
- Add loom plan, fixture, gate, and receipt commands.
- Add loom-assisted feature assimilation workflow.

## Phase 9: Deployment Spine

- Add CI workflows.
- Add release packaging.
- Add deployment receipt generation.
- Add daemon smoke tests.
- Add GitHub release automation.
- Add GitLab mirror documentation and push gate.

## Phase 10: Learning Loop

- Add corpus manifest format.
- Add regression snapshot runner.
- Add benchmark receipts.
- Add planner outcome receipts.
- Add review graph ranking evaluation.
- Add annotation grounding evaluation.

## Status (2026-10-26)

The phases above are the original plan; this section records where the review
council stack stands. Measured evidence is in
[`EVIDENCE.md`](../EVIDENCE.md#review-council-and-ci-2026-10-26).

Done:

- **Staged merges:** stage A (PR #3, `88e1cc0`) and stage B (PR #4,
  `b50b3f0`) are merged into `main`.
- **Green CI:** `main` passes all five jobs at run 40274253094, including the
  real-Podman sandbox suite on a GitHub-hosted runner (Phase 9's CI workflow).
- **Gate sandbox:** the eight council gates run only in a rootless Podman
  container, as the operator's own uid
  ([gate sandbox](REVIEW_COUNCIL.md#gate-sandbox)).
- **Exemption oracle:** only `#[cfg(test)]` may hide code from the
  diff-scoped mutation gate ([r18](REVIEW_COUNCIL.md#cfg-test-only-r18)).
- **Review interop:** Git acquisition (P1a), tuicr export (P1b) and the
  decision journal (P1c).

Remaining, in order:

1. **Council r18 triage** (run `f2baaa43`), then merge stage C (PR #5).
2. **CLI contract, daemon parity, SDK and MCP**, as four slices, each gated like
   a council round:
   - **A. CLI contract v1:** one `ddf.result.v1` envelope, uniform `--json`,
     a documented exit-code table, `ddf schema` and `ddf explain`.
   - **B. Daemon parity:** the planned JSON-RPC methods and a versioned
     protocol.
   - **C. SDK:** a semver-checked Rust facade crate, a thin Python client over
     the daemon socket, and `ddf rpc` over stdio.
   - **D. MCP server:** `ddf mcp` over stdio, read-only by default, with each
     write tool opt-in.

   A comes first, then B and D together, then C. None of this is implemented.
3. **GitLab `main` divergence:** reconcile it before any GitLab push of `main`.
4. **CI platform migrations:** Node 20 actions are deprecated, and the
   `ubuntu-latest` label moves to Ubuntu 26; the rootless-Podman step was
   measured on Ubuntu 24.04 only.
5. **crates.io:** on hold until the codebase is production-ready and the
   repository documentation is current.

## Deployment Link

- Framework: [Codebase Deployment Framework](DEPLOYMENT_FRAMEWORK.md)


## Review integration route

[Implementation status and delivery gates](RECOMMENDATION_INTEGRATION.md#remaining-module-routes) ↔ [Module vault note](obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fdocs--ROADMAP). This route records planned and evaluation-dependent work without claiming it is implemented.
