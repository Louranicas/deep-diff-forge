# Feature Baseline

This baseline is non-negotiable for the first complete product cut. It is a requirement list,
not a description of the code; the next section says where the code stands.

> **Status 2026-11-25 (`v0.11.0`).** Each row in the baseline table below is labelled Built,
> Partial, or Not built; Not built means planned and unowned. Withdrawn commands (`chain`,
> `cluster`, `loom`, and the `rank` subcommand) are recorded explicitly under
> [Withdrawn commands](#withdrawn-commands).

## Status at v0.11.0 (2026-11-25)

### Original Baseline Capabilities

| Capability | State | Evidence |
| --- | --- | --- |
| Review-first interactive UI | Built | `deep-diff-forge review` (`--side`, `--probe`, `--palette`/`--cmd`, `--findings`); `crates/deep-diff-forge-tui` with live-loop tests (`tui/src/run.rs`) and PTY integration (`cli/tests/review_tty.rs`). |
| Multi-file review stream + sidebar | Partial | Ranked file tree with status, counts, and reviewed flag; one file's diff at a time, no continuous stream. Machine equivalents: `--outline` and `--hunks` (and daemon/MCP `review.outline`, `review.hunks`, `review.change`). |
| Inline agent / AI annotations | Partial | Engine-generated notes with grounding labels (`engine_annotations`). External agent findings are verified against patch truth (`findings verify`, daemon `findings.verify`, MCP `verify_findings`, `deep-diff-forge-agent`) and shown as ungrounded agent notes (`review --findings`, `claim_verified: false`), anchored to hunks and lines, not to symbols or commands. |
| Responsive auto split/stack layout | Not built | Inline and side-by-side (`split` alias) are toggled explicitly (`--layout inline\|side-by-side`, `review --side`, `s`/`Tab`/`t`); no width-based switch and no stacked layout. |
| Mouse support inside the viewer | Partial | On by default in `review`: wheel scroll, menu-bar clicks, tree-row select, and pane focus (`map_mouse`); no pane resize or context actions. |
| Runtime view toggles | Partial | Layout (`s`/`Tab`/`t`), context fold (`z`), wrap (`w`), notes (`n`), reviewed mark (`v`/`Space`), theme (`T`), palette (`:`), help (`?`). No syntax/word/whitespace/moves/generated toggles. |
| Syntax highlighting | Partial | Tree-sitter for Rust only, terminal-injection-safe (`highlight <path> [--color\|--no-color]`, `crates/deep-diff-forge-syntax/src/highlight.rs`); other languages degrade explicitly without a fallback highlighter. |
| Structural diffing | Partial | `structural <old> <new> [--json]`: token-level, reformat-only and best-effort move detection, byte/node/time budgeted with explicit degraded state; Rust only. |
| Pager-compatible mode | Partial | `--stdin-patch --layout inline\|side-by-side`, Git selections (`--git-working-tree`, `--git-staged`, `--git-range`), and two-regular-file comparison (`deep-diff-forge [--] <old> <new>`); no directory pairs, no `GIT_EXTERNAL_DIFF` or `git difftool` mode. |

### Shipped Capabilities Through v0.11.0 (v0.1.0–v0.11.0)

Beyond the initial baseline table, `v0.11.0` ships the following source-verified capabilities across the 15-crate workspace (14 published crates + dev-only `deep-diff-forge-testkit`):

| Capability | Shipped | Source & Reference |
| --- | --- | --- |
| **Strict 42-shape Git parser & `judge_patch`** | 0.1.0–0.11.0 | `crates/deep-diff-forge-patch/src/{parser,judge_patch,git_raw,git_acquire}.rs`, [`CONFORMANCE.md`](../crates/deep-diff-forge-patch/CONFORMANCE.md). Accepts only the 42 header shapes Git's own diff writes (`HeaderNotWrittenByGit`), refuses `-U0` unanchored hunks (`HunkPlacedElsewhere`), unwriteable paths/symlinks (`GitCannotWrite`), non-UTF-8 headers (`LossyHeader`), and empty/prose `--stdin-patch` without `--allow-empty` (exit 4 `no_patch_in_input`). Git selections re-read binary-marked text files with `--text` (including stray-byte lossy code-evidence reading in 0.11.0, closing [DDF-2026-006](advisories/DDF-2026-006.md)). |
| **27-signal dual-reader risk & evidence engine** | 0.3.0–0.11.0 | `crates/deep-diff-forge-graph/src/{risk,evidence,rust_tokens,lexical,vocabulary}.rs`, [`risk-ranking.md`](reference/risk-ranking.md). 13 `RiskSignal` + 14 `EvidenceSignal` (including `diff_suppressed` and the 5 integrity signals `test_skipped`, `trivial_assert`, `stub_added`, `error_swallowed`, `panic_path_added`). Dual-reader Rust hunk scanning (`rust_tokens` + 0.8.3 lexical floor), multi-section same-path merging, linear-time long-word & secret-prefix guards ([DDF-2026-003](advisories/DDF-2026-003.md)), quoted CI privilege parsing ([DDF-2026-005](advisories/DDF-2026-005.md)), GitLab & fine-grained GitHub tokens ([DDF-2026-007](advisories/DDF-2026-007.md)), canonical `rank.v0` bytes with `tool.scorer_sha256`, `schema signals`, and `explain signal`. |
| **Diagnostics engine (`diagnostics.v0`) & `--fail-on`** | 0.11.0 | `crates/deep-diff-forge-graph/src/diagnostics.rs`, [`diagnostics.md`](reference/diagnostics.md). Emits `deep-diff-forge.diagnostics.v0` (`<label>@line` and `<label>@file` codes pinned in `fixtures/diagnostics/codes.v0.txt` with structured repair ids). Exposed via `--diagnostics [--json] [--fail-on LIST]`, `--rank --fail-on LIST`, `schema diagnostics`, daemon `diff.diagnostics`, MCP `diagnose_patch` / `review_change(view="diagnostics")`, and both SDKs. |
| **Machine contract & unified exit-7 registry** | 0.3.0–0.11.0 | `crates/deep-diff-forge-core/src/exit.rs`, `crates/deep-diff-forge-cli/src/{contract,reason}.rs`, [`output-and-exit-codes.md`](reference/output-and-exit-codes.md). `envelope` (`ddf.result.v1`), `schema [result\|exit-codes\|signals\|diagnostics]`, `explain`, deterministic `input_sha256` sealing, hunk `address`/`body_sha256`, 11 exit-7 refusal reason classes (`refused: <text> (<tag>)`), and closed-stdout verdict-refusal preservation ([DDF-2026-010](advisories/DDF-2026-010.md), [DDF-2026-011](advisories/DDF-2026-011.md)). |
| **Optional daemon (protocol 3, 17 RPC methods) & `rpc`** | 0.3.0–0.11.0 | `crates/deep-diff-forge-daemon/src/*.rs`, `crates/deep-diff-forge-cli/src/rpc.rs`, [`mcp-sdk-daemon.md`](reference/mcp-sdk-daemon.md). Owner-private Unix socket (`0700`/`0600`), kernel peer-credential checks (`SO_PEERCRED`/`getpeereid`), `<socket>.lock`, 80 MiB line cap, max 8 connections, 128 MiB in-flight admission cap, `--allow-repo`, Prometheus textfile export, systemd/launchd service units, and stdio `deep-diff-forge rpc`. |
| **Read-only MCP server (9 tools)** | 0.3.0–0.11.0 | `crates/deep-diff-forge-cli/src/mcp.rs`, [`mcp-sdk-daemon.md`](reference/mcp-sdk-daemon.md). Dual-era stdio MCP (`2026-07-28` + legacy `2025-*`) serving 9 `readOnlyHint: true` tools (`review_patch`, `rank_patch`, `review_outline`, `review_hunks`, `diagnose_patch`, `review_change`, `explain_exit_code`, `verify_findings`, `exit_code_registry`) with unified `\u` hidden-character escaping and parse refusals that never echo patch text. |
| **Rust & Python SDKs** | 0.3.0–0.11.0 | `crates/deep-diff-forge-sdk/`, `sdk/python/deep_diff_forge.py`. Rust library facade (`review`, `rank`, `diagnostics`, Unix `Client` with `ProtocolTooOld`) and stdlib-only Python package (`StdioClient`, `SocketClient`, `diagnostics_patch`, `ProtocolTooOld`). |
| **Agent hooks (state v13), HMAC history & project ledger** | 0.9.0–0.11.0 | `crates/deep-diff-forge-cli/src/{hook,hook_run,ledger,verdict_v2,agent_check,doctor}.rs`, `crates/deep-diff-forge-core/src/seal.rs`, [`agent-hooks.md`](reference/agent-hooks.md). `agent {setup,doctor,check,verify-branch,hook}` with state schema `deep-diff-forge.hook-state.v13`, RFC 2104 HMAC-SHA-256 keyed history, post-tool advice from `diagnostics.v0`, stateless fallback stop judging, single-parent commit bound-mode judging with forge-warden (verdict v2/v3), and `.git/deep-diff-forge/` project frontier ledger. |
| **Evaluation, benchmarking & local learning** | 0.4.0–0.11.0 | `crates/deep-diff-forge-learning/`, `crates/deep-diff-forge-cli/src/{eval,bench}.rs`, [`evaluation.md`](reference/evaluation.md). `eval` (`deep-diff-forge.eval.v0`, 73-case corpus + 155 declared misses), `bench` (`deep-diff-forge.bench.v0`, 9 workloads + `scripts/bench_compare.py`), and local-only `learn {record,status}` (`$XDG_STATE_HOME/deep-diff-forge/learning/`). |
| **Adversarial Review Council & decision journal** | 0.3.0–0.11.0 | `crates/deep-diff-forge-review/src/*.rs`, [`REVIEW_COUNCIL.md`](REVIEW_COUNCIL.md). `council {example,prepare,review,gate,status,import,export-tuicr,import-tuicr}`: 6 slots, 2 rounds, 8 rootless Podman gates, `mutation_scope.rs`, exemption oracle, tuicr export/import, append-only hash-chained `decisions.jsonl` with exit code 3 (`recorded_incomplete`) and [DDF-2026-004](advisories/DDF-2026-004.md) journal fault recovery. |

### Withdrawn Commands

- `deep-diff-forge chain` -> refused with exit 2 (compose single commands in the shell; `chain-contract` lists them).
- `deep-diff-forge cluster` -> refused with exit 2 (use the `--cluster` mode of `--stdin-patch`, a Git selection, or `<old> <new>`).
- `deep-diff-forge loom` -> refused with exit 2 (withdrawn with no replacement; `loom-contract` records the withdrawal).
- `deep-diff-forge rank` -> withdrawn on 2026-10-31 and refused with exit 2 (`--rank` serves it).

Compatibility at `v0.11.0`: Git and traditional unified patches are parsed under the 42-shape conformance rules and rendered back (`render_unified`); machine JSON/JSONL outputs (`review.v0`, `rank.v0`, `diagnostics.v0`, `review-outline.v0`, `review-hunks.v0`, `cluster.v0`, `findings.v0`, `ddf.result.v1`) are deterministic and sealed; exit codes and exit-7 refusal classes come from one registry (`deep-diff-forge schema exit-codes`). Directory comparison is not built: `analyze <patch> <base-dir> <candidate-dir>` verifies that the patch transforms base into candidate before attaching syntax analysis (`deep-diff-forge.analysis.v1`).

## Matrix Coverage

| Capability | Requirement |
| --- | --- |
| Review-first interactive UI | File review flow is the default, not an afterthought. |
| Multi-file review stream + sidebar | Stream view, file tree/sidebar, changed-file status, and jump navigation. |
| Inline agent / AI annotations | Agent notes anchored to files, hunks, lines, symbols, and commands with provenance. |
| Responsive auto split/stack layout | Side-by-side on wide surfaces, stacked or inline on narrow surfaces. |
| Mouse support inside the viewer | Scroll, select, open, collapse, resize, and context actions. |
| Runtime view toggles | Toggle syntax/line/word/semantic, context size, whitespace, comments, moves, generated files, and annotations. |
| Syntax highlighting | Tree-sitter-based where possible, fallback highlighter for plain text. |
| Structural diffing | Syntax-aware matching with conservative fallback and explicit parse status. |
| Pager-compatible mode | Works as `GIT_EXTERNAL_DIFF`, `git difftool`, `LESS`/pager stream, and plain CLI. |

## Compatibility Requirements

- Produce and consume unified patches.
- Preserve exact old/new file labels, modes, renames, binary markers, and no-newline metadata.
- Support directory comparisons.
- Support unsupported and malformed files through text fallback.
- Support machine-readable JSON output.
- Keep exit-code semantics explicit and scriptable.

## Performance Requirements

- Large changes must degrade, not hang.
- Every expensive diff strategy gets a byte/node/time budget.
- The planner records fallback reasons.
- Generated and vendored files must be detectable and suppressible.
- Interactive UI should target stable frame budgets, with progressive loading for large review sets.

## Trust Requirements

- AI annotations are never silently merged into the diff model.
- Every AI claim has source spans or is marked ungrounded.
- Approval state belongs to the reviewer, not the agent.
- Patch truth remains auditable without AI features enabled.

## Deployability Requirements

- Every feature has a CLI path.
- Every feature has a JSON or JSONL path for Claude Code.
- Every feature has Bash-safe exit-code behavior.
- Every feature has a fixture and regression receipt before release.
- Every feature degrades to patch truth when semantic or agent layers fail.

## Deployment Link

- Framework: [Codebase Deployment Framework](DEPLOYMENT_FRAMEWORK.md)
