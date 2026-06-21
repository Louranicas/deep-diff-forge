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
