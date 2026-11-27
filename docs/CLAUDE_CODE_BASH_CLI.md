# Claude Code, Bash, And CLI Optimization

Deep-Diff-Forge is optimized first for Claude Code, Bash, and CLI workflows.
GUI and IDE surfaces are projections over the same contracts, not separate
product truths.

## Status (v0.11.0)

Verified against `crates/deep-diff-forge-cli/src/{main,args,agent,agent_check,hook,hook_run,ledger,mcp,reason,fail_on}.rs`
and `crates/deep-diff-forge-core/src/exit.rs` at `v0.11.0`:

- **Shipped:** all review/ranking/diagnostics/views CLI flags (`--diagnostics`,
  `--fail-on LIST`, `--outline`, `--hunks`, `--rank`, `--cluster`, `--jsonl`,
  `--allow-empty`, `<old> <new>`), the built-in MCP server with **9 read-only
  tools**, `deep-diff-forge rpc` (protocol 3), `findings verify`, `eval`,
  `bench`, and the complete Claude Code agent integration
  (`agent setup --hooks`, `agent doctor --gate`, `agent check`,
  `agent verify-branch`, `agent hook`, hook state `deep-diff-forge.hook-state.v13`
  and the project outcome ledger under `.git/deep-diff-forge/ledger.jsonl`).
- **Withdrawn** (exit `2` with replacement): `chain`, `cluster`, `loom`.
- **Not built:** commands listed under [Not built](#not-built-planned-unless-marked-withdrawn).

## Design Priority

1. **Bash first:** every important action has a deterministic command.
2. **Claude Code friendly:** every agent-facing command has stable output,
   exit codes, exit-7 reason classes, and documented contracts.
3. **Human readable by default:** interactive review (`review`) and text
   summaries stay clean in a terminal.
4. **Machine readable on demand:** versioned JSON/JSONL, diagnostics, and 9
   read-only MCP tools serve non-interactive agent workflows.
5. **No daemon required:** CLI, `rpc`, `mcp` and hooks work without
   long-running services. No command starts the daemon implicitly.

## Bootstrap Commands

These take no arguments and need no TTY or daemon:

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

`doctor` reports runtime, cache and state directories and the resolved daemon
socket without contacting a daemon.

## Command Contract

### Shipped

```bash
# stdin patch: summary, JSON document, ranking, diagnostics, views, JSONL, layout
git diff | deep-diff-forge --stdin-patch
git diff | deep-diff-forge --stdin-patch --json
git diff | deep-diff-forge --stdin-patch --rank --json
git diff | deep-diff-forge --stdin-patch --diagnostics --json
git diff | deep-diff-forge --stdin-patch --outline --max-bytes 16384
git diff | deep-diff-forge --stdin-patch --hunks --path src/lib.rs
git diff | deep-diff-forge --stdin-patch --jsonl
git diff | deep-diff-forge --stdin-patch --layout side-by-side
# empty/prose stdin exits 4 (`no_patch_in_input`) unless --allow-empty is passed:
git diff | deep-diff-forge --stdin-patch --rank --json --allow-empty

# structured Git input (runs git itself; sees untracked files and C-quoted paths)
deep-diff-forge --git-working-tree --include-untracked --json
deep-diff-forge --git-staged --rank --fail-on security,integrity
deep-diff-forge --git-range main..HEAD --repo ../other --diagnostics --json
deep-diff-forge --git-working-tree --acquisition-json

# two regular files through hardened `git diff --no-index`, sealed (`--` if a path is a command name)
deep-diff-forge old.rs new.rs --json
deep-diff-forge -- doctor notes.txt --rank

# gates: input guards (exit 7) and signal/label gates (exit 7 after printing document)
git diff | deep-diff-forge --stdin-patch --rank --json --require-files --require-hunks
deep-diff-forge --git-range main..HEAD --diagnostics --json --fail-on security,integrity
deep-diff-forge findings verify --findings found.jsonl --require-findings --require-all < change.patch

# Claude Code / coding-agent setup, parity checks, on-demand judging and branch verification
deep-diff-forge agent setup --client claude --allow-repo "$PWD" --hooks
deep-diff-forge agent setup --client codex --allow-repo "$PWD"
deep-diff-forge agent doctor --gate --json
deep-diff-forge agent check --require-pass --json
deep-diff-forge agent check --session <SESSION_ID> --json
deep-diff-forge agent verify-branch main..HEAD --json

# file-level tools and corpus/benchmark checks
deep-diff-forge semantic src/lib.rs --json
deep-diff-forge structural old.rs new.rs --json
deep-diff-forge highlight src/lib.rs --no-color
deep-diff-forge analyze change.patch base/ candidate/
deep-diff-forge eval --json
deep-diff-forge bench --quick --json

# interactive review, and one headless frame for agents and CI
git diff | deep-diff-forge review [--findings found.jsonl]
git diff | deep-diff-forge review --probe --cols 120 --rows 40

# bounded local parallelism (patch + risk lanes)
git diff | deep-diff-forge --stdin-patch --cluster --parallel auto --json

# machine contract & signal explanations
deep-diff-forge envelope -- --stdin-patch --json < change.patch
deep-diff-forge schema [result|exit-codes|signals|diagnostics]
deep-diff-forge explain 7 --json
deep-diff-forge explain signal unsafe_added --audience agent --json

# agent and SDK transports
deep-diff-forge mcp [--allow-repo PATH]...    # 9 read-only MCP tools over stdio
deep-diff-forge rpc [--allow-repo PATH]...    # daemon JSON-RPC (protocol 3) over stdio

# daemon lifecycle (start runs in the foreground; use a supervisor in production)
deep-diff-forge daemon start [--allow-repo PATH]... [--log-format json]
deep-diff-forge daemon status
deep-diff-forge daemon metrics --format prometheus --out DIR/deep-diff-forge.prom
deep-diff-forge daemon stop                   # drains up to 30 s; waits up to 35 s for exit

# local learning loop
deep-diff-forge learn status --json
```

`daemon health|status|stop` print the daemon's JSON-RPC response line; they
take no `--json` flag.

### Not built (planned unless marked withdrawn)

Each of these exits `2`:

```bash
deep-diff-forge --git                                                # today: --git-working-tree
deep-diff-forge review --git                                         # today: git diff | deep-diff-forge review
deep-diff-forge --git --jsonl --progress
deep-diff-forge plan --git --json
deep-diff-forge ingest --git --jsonl | deep-diff-forge plan --stdin --jsonl
deep-diff-forge chain --manifest .deep-diff-forge/chain.toml         # withdrawn: join commands with the shell
deep-diff-forge cluster --git --parallel auto --jsonl                # withdrawn: --git-working-tree --cluster
deep-diff-forge loom plan --source <repo> --feature "review stream"  # withdrawn
deep-diff-forge daemon status --json
```

## Claude Code Hooks, State v13 & Project Ledger

For Claude Code, `deep-diff-forge agent setup --client claude --hooks` prints
both the MCP registration and the `.claude/settings.json` /
`~/.claude/settings.json` `hooks` block (it never writes files itself). Optional
flags `--judge PATH`, `--policy NAME` and `--ranker-dir DIR` pin the external
judge (`forge-warden`), policy file name (`warden.toml` by default) and older
ranker directory into the printed hook commands:

```bash
deep-diff-forge agent setup --client claude --allow-repo "$PWD" --hooks
deep-diff-forge agent doctor --gate    # verifies MCP/CLI/daemon parity + judge/policy/hook wiring
```

### Hook Lifecycle (`agent hook`)

| Claude Code event | Command | Timeout | Role |
| --- | --- | ---: | --- |
| `SessionStart` | `agent hook session-start --client claude …` | 10 s | Fixes the session project (inode of Git dir + path), base commit (`HEAD`, or prior session base on `resume`/`compact`/`clear`), baseline commit list, and runs the `agent doctor --gate` readiness check. |
| `PostToolUse` | `agent hook post-tool --client claude [--policy NAME]` | 5 s | **Advice only, never blocks.** Diffs the working tree since session base and surfaces new `diagnostics.v0` items with `audience == "agent"` in classes `security` and `integrity` (omitting `shape` and `floor_only` items), up to 20 items / ~5,600 bytes per turn, once per session. Also flags edits to guard files (policy file, `.gitattributes`, CI workflows, lint/Cargo configs, `.claude/settings*.json`; up to 8 per advice) before items. |
| `Stop` | `agent hook stop --client claude …` | 60 s (45 s internal budget, 30 s per judge run) | Evaluates units in order: uncommitted tree, staged change (if distinct), linked worktree changes, session commits (oldest first), and unjudged project frontier commits via external `forge-warden` (`forge-warden.verdict.v2` / `.v3`). Prints nothing only when every unit passes. **Only a `fail` blocks the stop** (and each failing line blocks once; repeats report `FAIL STANDS`). Missing judge/policy/ranker reports `UNJUDGED`, never a pass. |

### Session State (`deep-diff-forge.hook-state.v13`)

- **Location:** `$XDG_STATE_HOME/deep-diff-forge/hooks/` (else
  `~/.local/state/deep-diff-forge/hooks/`), mode `0700`.
- **Schema & migrations (`hook_run.rs`):** writes `deep-diff-forge.hook-state.v13`
  (`STATE_OWN = 13`) and automatically migrates states from `STATE_FLOOR = 10`
  (0.9.0/0.10.0) through `v11` → `v12` → `v13`. Older schemas (`< 10`) or newer
  schemas (`> 13`) fail closed into stateless fallback judging (`state.older` /
  `state.newer`) so a stop still judges the working tree.
- **Keyed history:** since `v12`, digests stored in hook state (`reported`,
  `noticed`, `blocked`, plus `guards` added in `v13`) are keyed with HMAC-SHA-256
  (RFC 2104, `deep-diff-forge-core::seal`).

### Project Outcome Ledger & Branch Verification

- **Location:** `<git-common-dir>/deep-diff-forge/ledger.jsonl` (typically
  `.git/deep-diff-forge/ledger.jsonl`, mode `0600`, append-only via `O_APPEND`
  under `flock`, never following symlinks, alongside `ledger.salt` for salted
  uncommitted tree digests).
- **Privacy & integrity:** records carry schema `v: 1`, `kind` (`stop`, `check`,
  `baseline`, `start`, `clearance`), hashed session key (16 hex chars), unit
  identifiers, outcomes, reason-class codes (50 structured codes in `reason.rs`
  across `pass`, `fail.*`, `hold.*`, `work.*`, `chain.*`, `time.*`, `state.*`,
  `place.*`, `input.*`), and SHA-256 seals — **never patch lines or judge prose**.
- **On-demand & branch checks:**
  - `deep-diff-forge agent check [--session ID | --base REV] [--range A..B] [--require-pass] [--json]`
    runs the stop's judge/policy on demand and records a `check` entry in the
    ledger when `--session` is given.
  - `deep-diff-forge agent verify-branch BASE..HEAD [--json]` checks every commit
    in `BASE..HEAD` against the ledger and exits `7` (`branch_unverified`) if
    any commit lacks a `pass` or `clearance` record. (The ledger is local
    evidence, not a server-side trust boundary — the PR judge remains the
    boundary.)

See [`docs/reference/agent-hooks.md`](reference/agent-hooks.md) for the
complete hook specification and all 50 reason classes.

## Built-in MCP Server (All 9 Read-Only Tools)

`deep-diff-forge mcp [--allow-repo PATH]...` speaks dual-era MCP (`2026-07-28`
modern `_meta` + `server/discover`, and legacy `2025-11-25` / `2025-06-18` /
`2025-03-26` `initialize`) over stdio. All **9 tools** are read-only
(`readOnlyHint: true`), declare `outputSchema`, escape all invisible/bidi/control
characters via `\uXXXX` (`safe_text`), and report parse failures without echoing
patch text (`without_input_in_errors()`):

| Tool | Purpose |
| --- | --- |
| `review_outline` | **Start here.** Risk-ordered file & hunk outline without line text (`review-outline.v0`, cursor-paged within `max_bytes`, default 32 KiB). |
| `review_hunks` | Verbatim hunks by `addresses` or `paths` (or all), cursor-paged within `max_bytes`. |
| `diagnose_patch` | Cursor-paged `deep-diff-forge.diagnostics.v0` (`max_bytes` 12,364–32 MiB, default 32 KiB) locating every ranking label with repair IDs and `page.document_sha256`. |
| `review_change` | Read `staged`, `working_tree` (optional `include_untracked`) or `range` straight from Git under `--allow-repo` roots, returning `outline`, `hunks` or `diagnostics` plus `acquisition`. |
| `review_patch` | Full `review.v0` up to 256 KiB (else `review-summary.v0`), optionally filtered by `paths` or `detail: "summary"`. |
| `rank_patch` | Sealed `rank.v0` (canonical bytes escaped in text copy, value in `structuredContent`) up to 256 KiB, else `rank-summary.v0`. |
| `verify_findings` | Ground-truth anchor/line/freshness verification of `finding.v0` items -> `findings.v0` (`claim_verified: false`). |
| `explain_exit_code` | Explain one CLI exit code (`engine` or `council`); code `7` includes all 11 exit-7 refusal classes. |
| `exit_code_registry` | Full exit-code and exit-7 refusal registry. |

`deep-diff-forge agent doctor` checks live in-process parity across CLI, daemon
and MCP (`mcp_handshake`, `review_parity`, `rank_parity`, `diagnostics_parity`,
`views`).

## Exit Codes

From `crates/deep-diff-forge-core/src/exit.rs` (`schema exit-codes`,
[`reference/output-and-exit-codes.md`](reference/output-and-exit-codes.md)):

| Code | Name | Meaning | Next |
| --- | --- | --- | --- |
| `0` | `success` | Success. | `proceed` |
| `2` | `usage` | CLI usage / argument error. | `fix_invocation` |
| `3` | `io` | I/O or encoding failure: reading input (stdin, a file, Git, learning store), non-UTF-8 input, or writing output. | `check_environment` |
| `4` | `invalid_input` | Parse/validation error, `--stdin-patch` input with no patch (`no_patch_in_input`, unless `--allow-empty`), over 64 MiB budget, invalid receipt, non-regular file path, or refused Git acquisition / file pair. | `change_input` |
| `6` | `daemon_or_terminal` | Daemon / interactive-terminal failure (`review` without TTY, `daemon status` with no daemon, `deploy doctor` failure). | `check_environment` |
| `7` | `input_contract_refused` | Contract or gate refusal; `stderr` prints `refused: <text> (<tag>)` naming one of the 11 registered classes (`require_files`, `require_hunks`, `require_findings`, `require_all`, `eval_failed`, `chain_not_ready`, `fail_on_label`, `fail_on_truncated`, `require_pass_no_unit`, `require_pass_not_pass`, `branch_unverified`). Input refusals leave `stdout` empty; verdict refusals print their document first and preserve exit `7` even if stdout closes. | `change_input` (refined per class) |
| `101` | `panic` | Internal error (a panic). | `report_bug` |

`council` subcommands use their own family (`0` completed, `3` recorded
incomplete, `4` refused, `7` hold). Codes `1`, `5` and `8` are never emitted.

## Output Modes

| Mode | Flag | Status |
| --- | --- | --- |
| Human summary | default | shipped (plain text) |
| Inline or side-by-side projection | `--layout inline\|side-by-side` | shipped |
| Ranked list | `--rank [--json] [--fail-on LIST]` | shipped (`rank.v0`) |
| Diagnostics | `--diagnostics [--json] [--fail-on LIST]` | shipped (`diagnostics.v0` or derived text) |
| Budgeted outline & hunks | `--outline`, `--hunks [--address A \| --path P]` | shipped (`review-outline.v0`, `review-hunks.v0`) |
| JSON | `--json` | shipped (`review.v0`, `rank.v0`, `diagnostics.v0`, `cluster.v0`) |
| JSONL | `--jsonl` | shipped (one `diff.file` event per file) |
| Syntax colour | `highlight --color\|--no-color` | shipped for `highlight` only |
| Machine envelope | `envelope -- <command…>` | shipped (`ddf.result.v1`) |
| ANSI pager / `--plain` / `--progress` / `--compact` | `--pager`, `--plain`, `--progress`, `--compact` | planned (exit `2`) |

Conflicting output flags exit `2` before any input is read
(`validate_review_options_or_exit` in `main.rs`): combining two output modes
(`--outline`, `--hunks`, `--cluster`, `--rank`, `--diagnostics`, `--jsonl`,
`--layout`, `--acquisition-json`, and `--json` except as the format of `--rank`,
`--diagnostics`, `--cluster`, `--outline` or `--hunks`), combining `--address`
with `--path`, passing view options (`--address`, `--path`, `--max-bytes`,
`--cursor`) without `--outline`/`--hunks`, passing `--parallel` without
`--cluster`, or passing `--fail-on` without `--rank` or `--diagnostics`.
Single-valued options given twice exit `2` (`args::admit`); `--address`,
`--path`, `--allow-repo` and `--workload` repeat. `--help` / `-h` prints
command help anywhere before `--` except as a valued option's argument.

## Claude Code Agent Contract

```bash
deep-diff-forge claude-code-contract
```

Guarantees that hold in `v0.11.0`:

- Contract and machine commands require no TTY.
- No command starts the daemon implicitly.
- JSON documents carry a versioned `schema`, an `input_sha256` seal over exact
  input bytes, and `tool` (`name`, `version`, plus `scorer_sha256` in `rank.v0`
  and `diagnostics.v0`).
- Patch truth remains available even when semantic analysis fails
  (`summary.semantic_fallbacks`).
- Every hunk carries deterministic `address` (`ddf-hunk-v1`) and `body_sha256`.
- `--cluster` joins deterministically regardless of worker count.
- `deep-diff-forge mcp` exposes 9 read-only tools with invisible-char escaping
  and zero patch-text echo on parse errors.
- `findings verify` / `verify_findings` verifies anchors/lines/freshness and
  marks accepted findings `source: "agent"` (`claim_verified: false`).
- `agent hook` keeps session state in `hook-state.v13` and appends outcomes to
  `.git/deep-diff-forge/ledger.jsonl`.

## Bash Integration Patterns

### Git External Diff (planned)

7-arg and 9-arg `GIT_EXTERNAL_DIFF` and `git difftool` invocations are not
implemented. (Direct two-file comparison `deep-diff-forge <old> <new>` is
shipped for regular files.) Use a Git alias over `--stdin-patch` or the
structured `--git-*` selections for Git workflows.

### Aliases

```bash
alias ddf='deep-diff-forge'
alias ddfg='deep-diff-forge --git-working-tree --include-untracked'
alias ddfd='deep-diff-forge --git-working-tree --include-untracked --diagnostics'
alias ddfr='git diff | deep-diff-forge review'
git config --global alias.review '!git diff | deep-diff-forge --stdin-patch --rank'
```

### CI & Branch Gate

```bash
set -euo pipefail
CARGO_TARGET_DIR=target cargo check --workspace --locked
CARGO_TARGET_DIR=target cargo test --workspace --locked
deep-diff-forge --self-test
deep-diff-forge doctor
deep-diff-forge agent doctor --json
deep-diff-forge --git-range "origin/main..HEAD" --diagnostics --json \
  --require-files --require-hunks --fail-on security,integrity > review-diagnostics.json
```

### Agent Review Handoff

```bash
out="$(mktemp -d)"
deep-diff-forge --git-working-tree --include-untracked --outline > "$out/outline.json"
deep-diff-forge --git-working-tree --include-untracked --rank --json > "$out/rank.json"
deep-diff-forge --git-working-tree --include-untracked --diagnostics --json > "$out/diagnostics.json"
```

## Shell Safety Rules

- Never require shell aliases for correctness.
- Never write outside explicit XDG state/runtime paths or the repository's
  `<git-common-dir>/deep-diff-forge/` ledger directory.
- Never mutate Git state (`--git-*` runs `git` with a fixed argv and never
  touches the index).
- Never start the daemon implicitly from read-only commands.
- Never emit secrets in diagnostics or ledger records.
- Bound memory: input caps at 64 MiB (exit `4` above).

## CLI Performance Defaults

| Situation | Behaviour in v0.11.0 |
| --- | --- |
| stdout is a TTY or a pipe | Review output is the same plain text either way. Only `highlight` adds ANSI colour (when stdout is a TTY or `--color` is passed). |
| Large diff | Read whole up to 64 MiB, then output; above the budget, exit `4`. Use `--outline` / `--hunks` with `--max-bytes` and `--cursor` to bound agent context. |
| Generated or vendored file | Base weight `1` with `generated_or_vendored` and no positive path bonuses, while retaining new/deleted/binary and code evidence signals (`risk.rs`): a patch author cannot hide risky code behind a vendored path name. |
| Semantic analysis fails | Patch truth plus a counted `semantic_fallbacks`. |
| Daemon unavailable | CLI never requires the daemon; `daemon health\|status\|metrics\|stop` exit `6`. |

## Deployment Link

- Framework: [Codebase Deployment Framework](DEPLOYMENT_FRAMEWORK.md)
