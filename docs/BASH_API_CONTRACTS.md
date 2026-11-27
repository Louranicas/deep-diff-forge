# Bash API Contracts

Deep-Diff-Forge is designed to be driven cleanly from strict Bash scripts.

## Status (v0.11.0)

Verified against `crates/deep-diff-forge-cli/src/{main,args,fail_on,refusal,reason,contract,agent,agent_check,ledger}.rs`
and `crates/deep-diff-forge-core/src/exit.rs` at `v0.11.0`:

- **Shipped:** `--stdin-patch`, structured Git selections (`--git-working-tree`,
  `--git-staged`, `--git-range A..B`), direct two-file comparison (`<old> <new>`),
  `--diagnostics [--fail-on LIST]`, `--rank [--fail-on LIST]`, `--outline`,
  `--hunks`, `--cluster`, `--jsonl`, `--layout`, `findings verify`,
  `agent {setup,doctor,check,verify-branch,hook}`, `eval`, `bench`, `explain`,
  `schema`, `envelope`, `rpc`, `mcp`, `daemon`, `review`, `council`, `learn`,
  `deploy`, `semantic`, `highlight`, `structural`, `analyze`.
- **Withdrawn** (refused with exit `2` and a replacement): `chain`, `cluster`,
  `loom`.
- **Not built:** commands listed under [Not built](#not-built) below (each exits
  `2`).

## Contract Rules (shipped)

- Commands work under `set -euo pipefail`; failures exit non-zero.
- `stdout` carries primary output; `stderr` carries diagnostics and refusal
  lines. On an error or input refusal, `stdout` stays empty; verdict refusals
  print their JSON/text document first and then exit `7`.
- No command prompts, except `review` (the interactive TUI cockpit, which exits
  `6` without a terminal; use `review --probe` headless).
- Exit codes and exit-7 reason classes come from one registry in the binary:
  `deep-diff-forge schema exit-codes`.
- `--json` prints one JSON document; `--jsonl` prints one JSON event per line.
- Input is read whole under a 64 MiB byte budget (`DEFAULT_BYTE_BUDGET` in
  `crates/deep-diff-forge-patch/src/lib.rs`), and output is written when the run
  completes. There is no progressive streaming yet.
- Every file opened by path (`--findings`, `<old> <new>`, `semantic`,
  `highlight`, `structural`, `analyze`, `eval`) is opened non-blocking and
  refused immediately with exit `4` if it is not a regular file (FIFO,
  directory, device or socket; `regular.rs`). Missing or unreadable paths exit
  `3`.
- Closed pipes: `deep-diff-forge … | head` exits `0` on a passing command, while
  any pending verdict refusal preserves exit `7` and its `stderr` reason line
  even when stdout is closed or pointed at `/dev/full` (`refusal.rs`).
- A diagnostic write failure (`2>/dev/full`) never panics or overwrites the
  command's exit code.
- Closed option sets (`args.rs`): unknown flags, malformed `--flag=value` forms,
  or repeated single-valued options exit `2` before input is read. Repeatable
  valued options are `--address`, `--path`, `--allow-repo` and `--workload`.

## Exit Codes (shipped)

Aligned with `crates/deep-diff-forge-core/src/exit.rs` (`EXIT_CODES`):

| Code | Name | Meaning | Next (`action`) |
| --- | --- | --- | --- |
| `0` | `success` | Success. | `proceed` |
| `2` | `usage` | CLI usage / argument error (unknown command/option, conflicting modes, withdrawn `chain`/`cluster`/`loom`, daemon socket path ≥ 108 bytes). | `fix_invocation` |
| `3` | `io` | I/O or encoding failure: reading input (stdin, a file, Git, the learning store), non-UTF-8 input, or writing output. | `check_environment` |
| `4` | `invalid_input` | Invalid input: a patch or analysis input that fails to parse or validate, `--stdin-patch` input in which Git finds no patch (`no_patch_in_input`, unless `--allow-empty`), input over the 64 MiB byte budget, an invalid receipt, a named path that is not a regular file, or a refused Git acquisition or file pair. | `change_input` |
| `6` | `daemon_or_terminal` | Daemon / interactive-terminal failure (for example `review` without a TTY, `daemon status` with no daemon, or a failing `deploy doctor` check). | `check_environment` |
| `7` | `input_contract_refused` | Input contract or gate verdict refused. `stderr` names the reason class in `refused: <text> (<tag>)` (see [Exit-7 Reason-Class Registry](#exit-7-reason-class-registry-refusals)). | `change_input` (refined per class) |
| `101` | `panic` | Internal error (a panic). | `report_bug` |

`council` subcommands use their own family:

| Council code | Name | Meaning | Next (`action`) |
| --- | --- | --- | --- |
| `0` | `completed` | Completed under contract (`status` exit `0` means ready for human review). | `proceed` |
| `3` | `recorded_incomplete` | I/O failure after the decision was recorded in the journal; rerun the same import to complete it. | `check_environment` |
| `4` | `refused` | Malformed input, stale admission, existing artifact, or protocol/I/O failure. | `change_input` |
| `7` | `hold` | HOLD: failed, missing or inconclusive evidence, or an import conflict. | `review_evidence` |

Only `check_environment` has `may_be_transient: true`.

Inspect any code or signal from scripts:

```bash
deep-diff-forge schema exit-codes
deep-diff-forge explain 7 --json
deep-diff-forge explain 3 --family council --json
deep-diff-forge schema signals
deep-diff-forge explain signal unsafe_added --audience agent --json
```

### Exit-7 Reason-Class Registry (`REFUSALS`)

Every engine exit `7` leaves through `refusal.rs` and prints one `stderr` line
per active refusal in registry order: `refused: <text> (<tag>)`. The text
carries counts, flags, labels and class names only, never untrusted patch text.
The same 11 classes are exposed by `schema exit-codes` (`refusals[]`),
`explain 7 [--json]`, and the MCP tools `explain_exit_code` (code `7`) and
`exit_code_registry`.

| Class | Tag | Commands | Stdout | Next | Since |
| --- | --- | --- | --- | --- | --- |
| `require_files` | `--require-files` | `--stdin-patch`, `--git-*`, `<old> <new>`, `findings verify`; daemon `require_files` | `empty` | `change_input` | 0.3.0 |
| `require_hunks` | `--require-hunks` | `--stdin-patch`, `--git-*`, `<old> <new>`, `findings verify`; daemon `require_hunks` | `empty` | `change_input` | 0.3.0 |
| `require_findings` | `--require-findings` | `findings verify` | `document` | `review_evidence` | 0.8.0 |
| `require_all` | `--require-all` | `findings verify` | `document` | `review_evidence` | 0.5.0 |
| `eval_failed` | `eval` | `eval` | `document` | `review_evidence` | 0.5.0 |
| `chain_not_ready` | `agent doctor` | `agent doctor` | `document` | `check_environment` | 0.9.0 |
| `fail_on_label` | `--fail-on` | `--rank` and `--diagnostics` of `--stdin-patch`, `--git-*`, `<old> <new>` | `document` | `review_evidence` | 0.11.0 |
| `fail_on_truncated` | `--fail-on evidence_truncated` | `--rank` and `--diagnostics` of `--stdin-patch`, `--git-*`, `<old> <new>` | `document` | `review_evidence` | 0.11.0 |
| `require_pass_no_unit` | `--require-pass no_unit` | `agent check` | `document` | `review_evidence` | 0.11.0 |
| `require_pass_not_pass` | `--require-pass not_pass` | `agent check` | `document` | `review_evidence` | 0.11.0 |
| `branch_unverified` | `verify-branch` | `agent verify-branch` | `document` | `review_evidence` | 0.11.0 |

### Withdrawn Subcommands (exit 2)

Each withdrawn command exits `2` with its replacement (`WITHDRAWN` in `main.rs`):

- `deep-diff-forge chain` → join single commands with the shell (`chain-contract` lists them).
- `deep-diff-forge cluster` → use `--cluster` on `--stdin-patch`, `--git-*` or `<old> <new>`.
- `deep-diff-forge loom` → nothing replaces it (`loom-contract` records the withdrawal).

## Stable Bootstrap Contract

These commands take no arguments (extra arguments exit `2`) and need no TTY or
daemon:

```bash
deep-diff-forge --help
deep-diff-forge --version
deep-diff-forge --self-test
deep-diff-forge doctor
deep-diff-forge claude-code-contract
deep-diff-forge chain-contract
deep-diff-forge cluster-contract
deep-diff-forge loom-contract
```

## Gate Contracts

### Input Guards & Empty/Prose Stdin (`--stdin-patch`, `--git-*`, `<old> <new>`)

```bash
git diff | deep-diff-forge --stdin-patch --rank --json --require-files --require-hunks
```

Refusal precedence on `--stdin-patch`:

1. **Parse error → exit `4`** (never masked by guards; includes non-Git headers
   and unanchored `-U0` hunks).
2. **Input guards → exit `7`** (`stdout` stays empty):
   - `--require-files`: `refused: 0 files in input (--require-files)` (evaluated first if both fail).
   - `--require-hunks`: `refused: 0 hunks in input (--require-hunks)` (refuses mode-only, rename-only, empty-file or binary-only changes).
3. **No patch in stdin → exit `4`** (`error: no_patch_in_input: …`): empty stdin,
   whitespace, prose or a commit message with no file sections fails closed with
   exit `4`, matching `git apply`. Pass `--allow-empty` only when an empty
   change is intentionally allowed (`--allow-empty` is accepted by `--stdin-patch`
   only, not `<old> <new>` or `--git-*`).

The `--require-files` and `--require-hunks` guards apply equally to `--git-*`,
`<old> <new>`, `findings verify`, and the daemon/MCP `require_files` /
`require_hunks` parameters.

### Signal / Label Gates (`--fail-on LIST`)

`--fail-on LIST` works with `--rank` or `--diagnostics` on `--stdin-patch`,
`--git-*` and `<old> <new>` (`fail_on.rs`):

- `LIST` is a comma-separated list of signal classes (`security`, `integrity`,
  `shape`) and/or signal labels from `deep-diff-forge schema signals`. Invalid
  tokens exit `2` before input is read.
- Evaluates each ranked file's `signals[]` (never truncated evidence rows) and
  its `evidence_truncated` bit (always checked, regardless of `LIST`).
- Prints the full ranking or diagnostics document to `stdout`, then exits `7`
  with `fail_on_label` and/or `fail_on_truncated` on `stderr`, followed by up to
  32 indented (`"  "`) escaped file lines (plus `  and N more files`).

```bash
deep-diff-forge --git-range main..HEAD --diagnostics --json --fail-on security,integrity
deep-diff-forge --git-staged --rank --json --require-files --require-hunks --fail-on security,check_suppressed
```

### Findings, Evaluation & Agent Gates

- **`findings verify`**: `deep-diff-forge findings verify --findings FILE [--require-all] [--require-findings] [--require-files] [--require-hunks] < change.patch` — exits `7` with `require_findings` (empty findings file) and/or `require_all` (any finding refused or over the 10,000 limit), printing `deep-diff-forge.findings.v0` first.
- **`eval`**: `deep-diff-forge eval [--corpus DIR] [--json]` — scores ranking, code evidence and findings against `fixtures/eval` (73 cases and 155 declared items: 130 `known_miss` + 25 `known_false_positive`); prints `deep-diff-forge.eval.v0` and exits `7` (`eval_failed`) if any case fails.
- **`bench`**: `deep-diff-forge bench [--quick] [--workload NAME]... [--json]` — times hot paths (5 repetitions with `--quick`, 15 default) and prints `deep-diff-forge.bench.v0`.
- **`agent doctor`**: `deep-diff-forge agent doctor [--gate [--judge PATH] [--policy NAME] [--ranker-dir DIR]] [--json]` — checks in-process MCP/CLI/daemon parity (including `diagnostics_parity`) and, with `--gate`, hook/judge chain readiness; exits `7` (`chain_not_ready`) if a gate check fails (exit `101` if surfaces disagree).
- **`agent check`**: `deep-diff-forge agent check [--session ID | --base REV] [--range A..B] [--judge PATH] [--policy NAME] [--ranker-dir DIR] [--require-pass] [--json]` — judges working tree, staged and commit units with the stop hook's judge/policy; `--require-pass` exits `7` (`require_pass_no_unit` or `require_pass_not_pass`).
- **`agent verify-branch`**: `deep-diff-forge agent verify-branch BASE..HEAD [--json]` — verifies every commit in `BASE..HEAD` against `.git/deep-diff-forge/ledger.jsonl`; exits `7` (`branch_unverified`) if any commit lacks a pass or clearance record.

## Machine Envelope (shipped)

To wrap any command's result in one JSON object (`ddf.result.v1`):

```bash
deep-diff-forge envelope -- --stdin-patch --json < change.patch
deep-diff-forge schema result        # JSON Schema (2020-12) of ddf.result.v1
deep-diff-forge schema exit-codes    # ddf.exit-codes.v1 registry (codes + refusals)
deep-diff-forge schema signals       # deep-diff-forge.signals.v0 (27 signals)
deep-diff-forge schema diagnostics   # JSON Schema (2020-12) of deep-diff-forge.diagnostics.v0
```

`envelope` exits with the wrapped command's code, or `128 + N` when signal `N`
ended it (`outcome: "signal"`). Captured `stdout` caps at 16 MiB and `stderr` at
1 MiB (`truncated` lists anything cut). Every word after `--` belongs to the
wrapped command, including `--help`.

## Justfile Runner Contract

The repo-local `justfile` wraps documented commands; it is not a separate
product API:

```bash
just status
just gate-docs          # fmt + check
just gate-bootstrap     # fmt + check + contracts
just gate-feature       # fmt + check + clippy + pedantic + test + contracts
just ci                 # local replica of CI gate jobs (scripts/ci_local.sh)
just contracts
just doctor
just receipt-bootstrap
```

- `CARGO_TARGET_DIR` is pinned to repo-local `target`.
- Generated receipts go under `reports/` and are ignored by Git.

## JSON Shapes

### Review document (shipped, `--stdin-patch --json`)

```json
{
  "schema": "deep-diff-forge.review.v0",
  "input_sha256": "6369d5c655af20b18347f87a6f5a73da2096dd8c15d842332c62e9201ad06c52",
  "tool": {"name": "deep-diff-forge", "version": "0.11.0"},
  "files": [
    {
      "path": "x.rs",
      "status": "modified",
      "strategy": "line",
      "hunks": [
        {
          "id": 0,
          "old_start": 1,
          "old_lines": 1,
          "new_start": 1,
          "new_lines": 1,
          "section": null,
          "address": "<64-hex>",
          "body_sha256": "<64-hex>",
          "lines": [
            {"kind": "removed", "old_line": 1, "new_line": null, "text": "a"},
            {"kind": "added", "old_line": null, "new_line": 1, "text": "b"}
          ]
        }
      ],
      "metadata": []
    }
  ],
  "summary": {
    "files_changed": 1,
    "additions": 1,
    "deletions": 1,
    "semantic_fallbacks": 0
  }
}
```

Files with hidden or traversing characters carry additive `hazards`; renamed or
copied files carry `old_path`.

### Rank document (shipped, `--stdin-patch --rank --json`)

```json
{
  "schema": "deep-diff-forge.rank.v0",
  "input_sha256": "6369d5c655af20b18347f87a6f5a73da2096dd8c15d842332c62e9201ad06c52",
  "tool": {"name": "deep-diff-forge", "version": "0.11.0", "scorer_sha256": "<64-hex>"},
  "ranked": [
    {"path": "x.rs", "status": "modified", "score": 2, "signals": []}
  ]
}
```

When code evidence signals fire, entries include `evidence[]` (up to 32 rows per
file, `evidence_truncated: true` if cut) and Git binary re-reads may include
`diff_suppressed`.

### Diagnostics document (shipped, `--stdin-patch --diagnostics --json`)

Every label of `rank.v0` located as `<label>@line` or `<label>@file` with
structured repair and substitute IDs (no line text from the patch, only paths):

```json
{
  "schema": "deep-diff-forge.diagnostics.v0",
  "input_sha256": "<64-hex>",
  "tool": {"name": "deep-diff-forge", "version": "0.11.0", "scorer_sha256": "<64-hex>"},
  "judge": {"state": "unknown", "ranker_sha256": null},
  "counts": {"files": 1, "diagnostics": 1, "agent": 1, "person": 0, "cut_files": 0},
  "cut": [],
  "diagnostics": [
    {
      "key": "<key>",
      "code": "unsafe_added@line",
      "label": "unsafe_added",
      "class": "security",
      "path": "src/lib.rs",
      "anchor": "line",
      "side": "new",
      "line": 12,
      "occurrence": 0,
      "reading": "current",
      "audience": "agent",
      "repairs": ["unsafe_added:a1"],
      "not_accepted": ["unsafe_added:n1"]
    }
  ],
  "texts": {
    "unsafe_added:a1": "…",
    "unsafe_added:n1": "…"
  }
}
```

Without `--json`, `--diagnostics` renders human-readable text derived directly
from the same canonical document (`diagnostics_text.rs`).

### JSONL event (shipped, `--stdin-patch --jsonl`)

One line per file:

```json
{"event":"diff.file","path":"x.rs","status":"modified","additions":1,"deletions":1,"hunks":1}
```

Other shipped schemas: `deep-diff-forge.review-outline.v0` (`--outline`),
`deep-diff-forge.review-hunks.v0` (`--hunks`), `deep-diff-forge.cluster.v0`
(`--cluster --json`), `deep-diff-forge.findings.v0` (`findings verify`),
`deep-diff-forge.semantic.v0` (`semantic --json`), `deep-diff-forge.structural.v0`
(`structural --json`), `deep-diff-forge.learning.v0` (`learn status --json`),
`deep-diff-forge.git-acquisition.v0` (`--acquisition-json`),
`deep-diff-forge.analysis.v1` (`analyze`), `deep-diff-forge.bench.v0` (`bench --json`),
`deep-diff-forge.eval.v0` (`eval --json`), `deep-diff-forge.daemon-metrics.v0`
(`daemon metrics`), `deep-diff-forge.deploy-doctor.v0` (`deploy doctor --json`),
`deep-diff-forge.agent-{setup,doctor,check,verify-branch}.v0`,
`deep-diff-forge.signals.v0` (`schema signals`),
`deep-diff-forge.signal-explain.v0` (`explain signal <label> --json`),
`deep-diff-forge.hook-state.v13` (hook session state), `ddf.result.v1`
(`envelope`), `ddf.exit-codes.v1` (`schema exit-codes`).

### Planner decision & progress event (planned, not implemented)

No command emits `deep-diff-forge.plan.v0` or `diff.file.updated` progress
events today. The review document's per-file `strategy` field carries the
chosen strategy.

## Shell Completion (planned, not implemented)

No `completions/` directory exists yet. No shell completion is required for
command correctness.

## Invocation Examples

### Shipped

```bash
# Machine-readable review of the working tree, untracked files included
deep-diff-forge --git-working-tree --include-untracked --json

# Risk-ranked review and diagnostics gate on a commit range
deep-diff-forge --git-range main..HEAD --rank --json --require-files --require-hunks
deep-diff-forge --git-range main..HEAD --diagnostics --json --fail-on security,integrity

# Budgeted outline and targeted hunk inspection
deep-diff-forge --git-staged --outline --max-bytes 16384
deep-diff-forge --git-staged --hunks --path src/lib.rs --max-bytes 16384

# Two regular files through hardened `git diff --no-index`
deep-diff-forge old.rs new.rs --rank --json --require-files --require-hunks
deep-diff-forge -- doctor notes.txt --diagnostics --json

# Verify agent findings against a patch
deep-diff-forge findings verify --findings found.jsonl --require-findings --require-all < change.patch

# Symbols of one file, and reformat-aware token comparison
deep-diff-forge semantic src/lib.rs --json
deep-diff-forge structural old.rs new.rs --json

# Bounded local parallelism (patch + risk lanes, deterministic join)
git diff | deep-diff-forge --stdin-patch --cluster --parallel 4 --json

# Agent setup, parity/gate doctor, on-demand check and branch ledger verification
deep-diff-forge agent setup --client claude --hooks --allow-repo "$PWD"
deep-diff-forge agent doctor --gate --json
deep-diff-forge agent check --require-pass --json
deep-diff-forge agent verify-branch main..HEAD --json

# Evaluation corpus and benchmarks
deep-diff-forge eval --json
deep-diff-forge bench --quick --json

# One daemon-protocol (protocol 3) request without a daemon
printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"engine.methods"}' | deep-diff-forge rpc
```

`--parallel` takes `serial`, `auto` (default) or a positive worker count
(anything else exits `2`); fixed counts cap at 4× CPU count.

### Not built

Each of these exits `2`: `plan`, `why-first`, `ingest`, `--git` and `--budget`
are planned and unowned; `chain`, `cluster`, `loom` (and `rank`/`render` as
subcommands) are withdrawn:

```bash
deep-diff-forge --git --json --budget fast
deep-diff-forge plan --git --json
deep-diff-forge why-first hunk:42 --json
deep-diff-forge ingest --git --jsonl \
  | deep-diff-forge plan --stdin --jsonl \
  | deep-diff-forge rank --stdin --json \
  | deep-diff-forge render --stdin --plain
```

## Deployment Link

- Framework: [Codebase Deployment Framework](DEPLOYMENT_FRAMEWORK.md)
