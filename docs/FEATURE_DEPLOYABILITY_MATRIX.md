# Feature Deployability Matrix

Every baseline and pioneer feature must have a command path, model path, test path, and release gate.

This is a target matrix: a surface marked *(not implemented)* was checked against the binary on
2026-10-27 (anchor E-02) and is refused today. `deep-diff-forge --help` lists what exists.

> **Status 2026-11-25 (`v0.11.0`).** The "Today (`v0.11.0`)" column is what ships on `main` at
> tag `v0.11.0`; a surface marked *(not implemented)* is planned and unowned. The `rank`
> subcommand (2026-10-31; `--rank` serves it) and the `chain`, `cluster`, and `loom` subcommands
> (2026-10-31; all refused with exit 2) are withdrawn. The `launchd` agent on physical Mac
> hardware remains on hold (owner decision 2026-11-01; see
> [ROADMAP.md](ROADMAP.md#open-items-for-maintainers)).

## Status 2026-10-28 (0.3.0, historical check)

Rechecked against the 0.3.0 debug binary. Every surface marked *(not implemented)* below exits 2
(unknown command or unknown option). Corrections in that pass: the review cockpit reads its patch
from stdin and has no Git flags of its own; `--syntax`, `--semantic` and the pioneer commands
`rank`, `plan` and `--git` do not exist; the "Today" column names what does. `split` is accepted
as an alias of `side-by-side` (`layout_from_str` in `crates/deep-diff-forge-projection/src/lib.rs`).

## Baseline Features

| Feature | CLI/Bash surface (target) | Today (`v0.11.0`) | Model/API surface | Test gate | Deployable when |
| --- | --- | --- | --- | --- | --- |
| Review-first interactive UI | `deep-diff-forge review` (`--git` *(not implemented)*) | `git diff \| deep-diff-forge review [--side] [--findings FILE] [--palette \| --cmd NAME]`; headless `review --probe [--cols N] [--rows N]`; `just review [ref]` | `ReviewDocument`, projections | TUI live-loop tests (`tui/src/run.rs`) + PTY (`cli/tests/review_tty.rs`) + `--probe` | Keyboard and mouse review works without daemon. |
| Multi-file review stream + sidebar | `review`, `--json` | `review` sidebar; `--stdin-patch`, `--git-working-tree`, `--git-staged`, `--git-range A..B`, `<old> <new>` with `--json`, `--jsonl`, `--outline`, `--hunks` | file list, status, ranked stream, paged views | multi-file & paging fixtures | Changed files can be navigated and exported. |
| Inline agent / AI annotations | `annotation add` *(not implemented)*, JSONL API | `findings verify --findings FILE [--require-findings] [--require-all]` (`finding.v0` -> sealed `findings.v0`), `review --findings FILE`, daemon `findings.verify`, MCP `verify_findings`, `deep-diff-forge-agent` crate API | `AgentAnnotation`, `FindingReport` | grounded/ungrounded fixtures + `findings_verify` fuzzer | Annotations anchor to file/hunk/span and never mutate patch truth. |
| Responsive auto split/stack layout | `--layout auto\|stack` *(not implemented)* | `--layout inline\|side-by-side` (`split` = `side-by-side`); `review --side` and runtime toggle `s`/`Tab`/`t` | projection config | terminal width snapshots | Same diff renders correctly at narrow and wide widths. |
| Mouse support inside viewer | `review --mouse` *(not implemented)* | on by default in `review`: wheel scroll, file-tree and menu clicks, pane focus (no flag) | TUI event model | terminal integration smoke | Scroll/select/collapse work in supported terminals. |
| Runtime view toggles | `--toggle <name>=<value>` *(not implemented)* | TUI keys (`s`/`Tab`/`t`, `z`, `w`, `n`, `v`/`Space`, `T`, `:`, `?`) | session toggles | projection toggle tests | Toggles rerender without recomputing patch truth. |
| Syntax highlighting | `--syntax on\|off` *(not implemented)* | `highlight <path> [--color\|--no-color]` (Rust only; terminal-escape sanitized) | token spans | language fixtures | Supported files highlight and unsupported files degrade. |
| Structural diffing | `--semantic on\|off` *(not implemented)* | `semantic <path> [--json]`, `structural <old> <new> [--json]` (Rust only; budgeted) | `SemanticTwin`, `StructuralDiff` | syntax fixtures + `structural_soundness` fuzzer | Semantic spans synchronize to patch hunks. |
| Pager-compatible mode | default pipe/TTY behavior | `git diff \| deep-diff-forge --stdin-patch --layout inline\|side-by-side`; `deep-diff-forge [--] <old> <new>` for two regular files; no directory pairs or Git external-diff/difftool mode | inline/side-by-side projection | file-pair & projection tests | Works with Git diff/difftool and plain shell pipes. |

## Pioneer Features

| Feature | CLI/Bash surface (target) | Today (`v0.11.0`) | Model/API surface | Test gate | Deployable when |
| --- | --- | --- | --- | --- | --- |
| Semantic Patch Twin | `deep-diff-forge --git --semantic --json` *(not implemented)* | `--stdin-patch --json` (`deep-diff-forge.review.v0`); `semantic <path>`; `analyze <patch> <base-dir> <candidate-dir>` (`analysis.v1`) | `PatchTwin` + `SemanticTwin` + anchor map | patch/semantic sync fixtures | Patch output remains apply-able and semantic anchors survive toggles. |
| Review Intelligence Graph | `deep-diff-forge rank --git --json` *(withdrawn 2026-10-31)* | `--rank [--json] [--fail-on LIST]`, `--diagnostics [--json] [--fail-on LIST]` across `--stdin-patch`, `--git-*`, `<old> <new>`; `schema signals`, `explain signal`; daemon `diff.rank` / `diff.diagnostics`; MCP `rank_patch` / `diagnose_patch` | 27-signal additive scorer (`rank.v0` with `scorer_sha256`, `diagnostics.v0`) | ranking & 73-case eval corpus + `evidence_scan`/`rust_*` fuzzers | Risk ranking is deterministic and explainable. |
| Adaptive Diff Planner | `deep-diff-forge plan --git --json` *(not implemented)* | per-file `strategy` (`Line`/`Binary`) in `review.v0`; JSON-RPC `diff.plan` returns ranked list; library-only `PlannerLearner` in `deep-diff-forge-learning` | `DiffStrategy`, `PlannerLearner` (unwired) | planner budget fixtures | Strategy choices and fallback reasons are visible and bounded. |

## Shipped Surfaces Matrix (`v0.11.0`)

Every shipped surface in `v0.11.0`, its exact deployability status, platform/sandbox requirements, test gate, and reference documentation:

| Surface | Status (`v0.11.0`) | Command / API Surface | Platform & Sandbox Requirements | Test & Contract Gate | Reference |
| --- | --- | --- | --- | --- | --- |
| **Strict 42-shape Git parser & acquisition** | Shipped | `--stdin-patch [--allow-empty] [--require-files] [--require-hunks]`, `--git-working-tree`, `--git-staged`, `--git-range A..B`, `<old> <new>` | Local `git` binary for `--git-*` & `<old> <new>`; stdin requires no external tool | `patch/tests/conformance.rs` (29 rows, 42 shapes), `git_differential.rs`, `patch_parser` / `judge_patch` / `git_raw_stream` fuzzers | [`CONFORMANCE.md`](../crates/deep-diff-forge-patch/CONFORMANCE.md), [`commands.md`](reference/commands.md) |
| **27-signal risk & dual-reader evidence** | Shipped | `--rank [--json] [--fail-on LIST]`, `--cluster [--parallel ...]`, `schema signals`, `explain signal` | Pure Rust (zero external tools) | `cli/tests/rank_evidence.rs`, `fail_on.rs`, `eval.rs` (73 cases), `long_lines_of_every_trigger_rank_in_bounded_time`, `evidence_scan`/`rust_*` fuzzers | [`risk-ranking.md`](reference/risk-ranking.md) |
| **Diagnostics engine (`diagnostics.v0`)** | Shipped | `--diagnostics [--json] [--fail-on LIST]`, `schema diagnostics`, daemon `diff.diagnostics`, MCP `diagnose_patch` & `review_change(view="diagnostics")`, Rust/Python SDKs | Pure Rust; daemon/SDK paging requires protocol 3 | `cli/tests/diagnostics.rs`, `daemon_parity.rs`, `contract.rs`, `ci_local.sh` `diagnostics contract` step | [`diagnostics.md`](reference/diagnostics.md) |
| **Machine contract & unified exit 7** | Shipped | `envelope -- <cmd>`, `schema [result\|exit-codes\|signals\|diagnostics]`, `explain`, 11 exit-7 refusal classes | Pure Rust | `cli/tests/contract.rs`, `sealed_output.rs`, `empty_input.rs` | [`output-and-exit-codes.md`](reference/output-and-exit-codes.md) |
| **Daemon (protocol 3, 17 RPC methods) & `rpc`** | Shipped | `daemon {path\|start\|health\|status\|metrics\|stop}`, `rpc [--allow-repo PATH]`, systemd/launchd units | Unix domain sockets (Linux & macOS; no Windows named pipes); `$XDG_RUNTIME_DIR` or `--socket`; `launchd` on physical Mac on hold | `cli/tests/daemon.rs`, `daemon_parity.rs`, `protocol_compat.rs`, `rpc.rs`, `service_units.rs`, `daemon_soak.py`, `daemon_protocol`/`stdio_rpc` fuzzers | [`mcp-sdk-daemon.md`](reference/mcp-sdk-daemon.md), [`OPERATIONS_AND_DAEMON.md`](OPERATIONS_AND_DAEMON.md) |
| **Read-only MCP server (9 tools)** | Shipped | `mcp [--allow-repo PATH]` over stdio (2026-07-28 + legacy 2025-*) | Stdio JSON-RPC; local `git` only for `review_change` under `--allow-repo` | `cli/tests/mcp.rs`, `stdio_mcp` fuzzer | [`mcp-sdk-daemon.md`](reference/mcp-sdk-daemon.md), [`API_AND_IPC.md`](API_AND_IPC.md) |
| **Rust & Python SDKs** | Shipped | `deep-diff-forge-sdk` (crates.io); `sdk/python/` (`StdioClient`, `SocketClient`) | Rust MSRV 1.88 (Unix-only for `Client`); Python 3 stdlib + `deep-diff-forge` binary | `sdk/tests/api.rs`, `sdk/python/test_deep_diff_forge.py`, `cli/tests/python_sdk.rs` | [`mcp-sdk-daemon.md`](reference/mcp-sdk-daemon.md), [`sdk/python/README.md`](../sdk/python/README.md) |
| **Agent hooks (v13), HMAC history & ledger** | Shipped | `agent {setup\|doctor\|check\|verify-branch\|hook}` (`session-start`, `post-tool`, `stop`) | Local `git`; optional external `forge-warden` binary for judged stops; `.git/deep-diff-forge/` writable | `cli/tests/agent.rs`, `agent_hook.rs`, `agent_check.rs`, `scripts/contract_matrix.sh`, `hook_input`/`verdict_reader`/`judge_patch` fuzzers | [`agent-hooks.md`](reference/agent-hooks.md) |
| **Evaluation, benchmarking & learning** | Shipped (`learn` CLI partial) | `eval [--json]`, `bench [--quick] [--json]`, `learn {record\|status}` | `$XDG_STATE_HOME/deep-diff-forge/learning/` (`0700`/`0600`) for `learn` | `cli/tests/eval.rs`, `bench.rs`, `learn.rs`, `privacy_probe.py`, `bench_compare.py` | [`evaluation.md`](reference/evaluation.md), [`LEARNING_LOOP.md`](LEARNING_LOOP.md) |
| **Adversarial Review Council & journal** | Shipped | `council {example\|prepare\|review\|gate\|status\|import\|export-tuicr\|import-tuicr}` | Unix; rootless Podman + `DDF_COUNCIL_GATE_IMAGE` for 8 gates; local Claude Code/Codex CLIs for live rounds; optional `tuicr` 0.27.0 | `cli/tests/council.rs`, `council_sandbox.rs` (12 probes), `council_tuicr.rs`, `review/tests/contracts.rs`, `council_report`/`journal_recover` fuzzers | [`REVIEW_COUNCIL.md`](REVIEW_COUNCIL.md), [`quality-gates.md`](reference/quality-gates.md) |
| **Withdrawn commands** | Withdrawn | `chain`, `cluster`, `loom`, `rank` -> exit 2 | N/A | `cli/tests/help.rs`, `contract.rs` | [`commands.md`](reference/commands.md), [`DEEP_DIFF_FORGE_LOOM.md`](DEEP_DIFF_FORGE_LOOM.md) |

Platform and distribution posture at `v0.11.0` ([`supported-platforms.md`](reference/supported-platforms.md), [`RELEASE_AND_PUBLICATION.md`](RELEASE_AND_PUBLICATION.md)):
- **Linux x86_64 (`x86_64-unknown-linux-gnu`):** primary release binary target (glibc floor 2.34, checked by `scripts/check_glibc_floor.sh`), built twice reproducibly locally (`scripts/release_local.sh`), published with signed `SHA256SUMS` and `RELEASE-MANIFEST.json` on GitLab, Gitea, and Forgejo, and mirrored on GitHub.
- **Linux arm64 & macOS (arm64/x86_64):** CI-tested source targets; crates published on crates.io (14 published crates from `v0.9.0` onward; `deep-diff-forge-testkit` is dev-only and unpublished).
- **Windows / multi-user daemon:** unsupported.

## Acceptance Criteria Per Feature

A feature is not deployable until it has:

- documented CLI flags
- stable model fields
- human output
- JSON or JSONL output
- at least one fixture
- at least one regression snapshot
- failure mode documented
- receipt in release gate

## Minimal First Implementation Order

1. Pager-compatible patch parser and inline renderer.
2. Side-by-side projection.
3. JSON `ReviewDocument` export.
4. Git workspace input.
5. Baseline `review` TUI.
6. Syntax highlighting.
7. Semantic twin.
8. Planner budgets.
9. Review graph ranking.
10. Agent annotations.

## Deployment Link

- Framework: [Codebase Deployment Framework](DEPLOYMENT_FRAMEWORK.md)
