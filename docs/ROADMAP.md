# Roadmap

## Status 2026-10-28 (0.3.0)

This section is current. The phase lists below are the original plan and are kept unchanged;
the [2026-10-26 status](#status-2026-10-26) is kept as a dated record. Status is read from the
code at `2f03f40` plus this release's version bump, and from `deep-diff-forge --help`.

| Phase | State | What exists / what remains |
| --- | --- | --- |
| 0. Vocabulary and contracts | Done | Core model, exit-code registry, `--self-test`, `doctor`, `claude-code-contract`, `chain-contract`, `cluster-contract`, `loom-contract`. |
| 1. Patch baseline | Mostly done | Parser, renderer, side-by-side and inline projections, `--stdin-patch` with `--json`, `--jsonl`, `--rank`, `--cluster`, `--layout`. Remaining: `--plain` and `--exit-code` do not exist (refused as unknown options); the pipeline has ingest and render stages only. |
| 2. Pager and terminal review | Partial | Done: stdin patch, Git selections (`--git-working-tree`, `--git-staged`, `--git-range`), Rust syntax highlighting, keyboard and mouse TUI. Remaining: file-pair and directory-pair diff (`deep-diff-forge <old> <new>` is refused), Git external-diff mode, automatic width-based layout and a stacked layout. |
| 3. Structural diff | Partial | Done: tree-sitter with the Rust grammar only, byte and node budgets and a cooperative time budget, token-level structural diff with reformat-only and best-effort moved-block detection (`structural <old> <new>`). Remaining: more languages, structural spans tied to hunks. |
| 4. Review intelligence graph | Partial | Done: deterministic file-level risk ranking with generated/vendored suppression. Remaining: hunk-level ranking, symbols, tests, owners, commands in the graph. |
| 5. Agent collaboration | Partial | Done: grounding and source classification, sanitising, anchor validation (library); the TUI shows the engine's own grounded notes. Done (2026-10-30): verified agent findings, the path for external agent findings into the CLI (`findings verify`), the daemon (`findings.verify`, protocol 2), `rpc`, MCP (`verify_findings`), the Python SDK and the cockpit (`review --findings`), each checked against patch truth (anchor, line, freshness, size, duplicates) and never grounded. Remaining: a persistent findings store; agent review requests. |
| 6. App surfaces | Partial | Done: TUI. Remaining: a TUI test harness beyond the headless `review --probe` frame; desktop, web, IDE and forge adapters. |
| 7. Daemon | Partial | Done: Unix-socket JSON-RPC 2.0, protocol 1, sessions, `engine.methods`, 0700 directory and 0600 socket. Done since 0.3.0: a peer-credential check on both ends of the socket, a start lock, requests in parallel, and (2026-10-30) `daemon.metrics`, doctor warnings from it, and `--log-format json`. Remaining: AST and line-index cache, subscriptions. |
| 8. Cluster and loom | Cluster partial, loom not built | Done: `deep-diff-forge-cluster` with contiguous shards, join policies and a receipt, run by `--cluster`. Remaining: semantic and other dimensions; the `cluster` subcommand; all of loom. |
| 9. Deployment spine | Done | CI (`ci.yml`, 10 required checks; weekly `campaign.yml`), `release.yml` (3 targets, sha256, SLSA attestation on tags), `deploy status|release|doctor`, hostile daemon soak smoke, GitHub releases v0.1.0 and v0.2.0, GitLab `main` fast-forwarded from GitHub `main` after each merge. |
| 10. Learning loop | Partial | Done: local-only strategy receipts (`learn record --stdin`), scoring and trust (`learn status`), planner, ranking and annotation learners and promotion rules (library only). Remaining: nothing emits receipts automatically; no corpus manifest, benchmark receipts, or wiring of learned weights into ranking. See [LEARNING_LOOP.md](LEARNING_LOOP.md). |

Release and publication:

- **0.4.0 is tagged** (`v0.4.0`, 2026-10-30, owner decision). Any later `v*` tag is the owner's
  decision. Published GitHub releases are v0.1.0, v0.2.0, v0.3.0 and v0.4.0; v0.2.1 was never tagged.
- **crates.io is on hold** by owner decision until the codebase is production-hardened.
  `release.yml` publishes only when the repository variable `DDF_CRATES_IO` is `open`; a
  configured token does not lift the hold.

Production readiness (2026-10-28 verdict): ready for its declared scope, a local single-user
review tool (CLI, daemon, `rpc`, `mcp`, SDKs) on Linux and macOS, released as GitHub binaries.
Not claimed: crates.io, a multi-user or shared-host daemon.

Residual risks (open):

1. ~~One engine mutex serializes daemon requests~~: closed. Requests run in parallel under a 128 MiB cap on the request bytes in flight, first come, first served.
2. ~~No peer-credential check on the socket~~: closed. The daemon drops a client of another user (`SO_PEERCRED`/`getpeereid`), and the CLI and SDKs refuse a daemon of another user.
3. ~~Git subprocesses have no time limit~~: closed. One deadline bounds every Git call of an acquisition, 120 s for the CLI and 30 s for `review.change` (`GitAcquireOptions::time_limit`).
4. ~~macOS tests run on schedule and dispatch only, not on pull requests~~: closed. They run on every pull request and are a required check.
5. ~~Two `daemon start`s can race in the window between probing and binding~~: closed. A start holds an exclusive lock on `<socket>.lock` from before the probe until the daemon exits.
6. No TUI test harness beyond the headless probe.
7. The council trial is not designed yet.

Fixed on 2026-10-28 (PRs #21 to #23): a text change hidden after a `GIT binary patch` body (now a
strict base85 grammar, else exit 4); gate containers run as the caller's uid and gid instead of
`--user=0:0`; quadratic metadata parsing (1M lines: 139 s to 0.20 s, measured); macOS
`SO_RCVTIMEO` `EINVAL`; a second `daemon start` orphaning the first (now `AddrInUse` when a live
daemon answers); responses over the 80 MiB line cap; an invalid learning receipt exiting 3 (now
4); `daemon --socket PATH health` misparsing; `--foreground` accepted outside `daemon start`.

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
- **Exemption oracle (v11, stage C, PR #5, `39005c5`):** only `#[cfg(test)]`
  may hide code from the diff-scoped mutation gate. Allow-listed code is checked
  by MIR, a direct build-script report and a fail-closed dep-info allow-list,
  under a stop rule ([r18–r22](REVIEW_COUNCIL.md#cfg-test-only-r18)).
- **CI platform:** every job runs on `ubuntu-24.04`, the image the Podman step
  was measured on, and the Node 20 actions moved to their Node 24 releases
  (stage D). Since 2026-10-27, CI runs on `ubuntu-26.04` (item 3 below).
- **Review interop:** Git acquisition (P1a), tuicr export (P1b) and the
  decision journal (P1c).

Remaining, in order:

1. **CLI contract, daemon parity, SDK and MCP**, as four slices, each gated like
   a council round:
   - **A. CLI contract v1 (implemented):** `envelope -- <command…>` gives every
     command one `ddf.result.v1` envelope without changing its own output.
     Also `schema`, `explain`, and one exit-code registry by family, guarded
     against README drift.
   - **B. Daemon parity (implemented: protocol 1):** `diff.compute` matches
     the CLI's review document, with a parity test; there is method discovery
     (`engine.methods`) and protocol negotiation. The remaining planned methods
     (subscriptions, projection, annotations, cache) stay planned.
   - **C. SDK (implemented):** the `deep-diff-forge-sdk` Rust facade, with
     signatures pinned by a compile-checked test (`cargo-semver-checks` waits for
     a published baseline); a stdlib Python client over the socket and over
     `deep-diff-forge rpc`; and `rpc` itself.
   - **D. MCP server (implemented):** `deep-diff-forge mcp` over stdio, dual-era
     (2026-07-28 and the legacy handshake). It has four read-only tools, with
     `review_patch` tested for parity with the CLI. No write tool exists; any
     future one must be opt-in.

   A comes first, then B and D together, then C. All four are implemented.
2. **GitLab `main` divergence:** done 2026-10-27. It was reconciled by merge
   (PR #18, merge commit `85a0e5d`); GitLab fast-forwarded `e689f9c..85a0e5d`.
   The 2026-06-23 logs are under `docs/history/2026-06-23-review-cockpit/`.
   The rule now: GitLab `main` is pushed only as a fast-forward of GitHub
   `main`, after a PR merges.
3. **Ubuntu 26:** done 2026-10-27. CI moved to `ubuntu-26.04` after a measured
   trial: rootless Podman 5.7.0 needed the same AppArmor user-namespace step as
   24.04, and all 12 sandbox probes were refused. The Linux release binary is
   still built on `ubuntu-24.04` for glibc compatibility.
4. **crates.io:** on hold until the codebase is production-ready and the
   repository documentation is current.

Open work after the anchor list: see the vault note [Strategy/Remaining Work 2026-10-28](obsidian://open?vault=deep-diff-forge.vault&file=Strategy%2FRemaining%20Work%202026-10-28) (owner's local vault).

## Deployment Link

- Framework: [Codebase Deployment Framework](DEPLOYMENT_FRAMEWORK.md)


## Review integration route

[Implementation status and delivery gates](RECOMMENDATION_INTEGRATION.md#remaining-module-routes) ↔ [Module vault note](obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fdocs--ROADMAP) (owner's local vault). This route records planned and evaluation-dependent work without claiming it is implemented.
