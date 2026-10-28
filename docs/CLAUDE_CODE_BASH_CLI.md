# Claude Code, Bash, And CLI Optimization

Deep-Diff-Forge is optimized first for Claude Code, Bash, and CLI workflows. GUI and IDE surfaces are projections over the same contracts, not separate product truths.

## Status 2026-10-28 (0.3.0)

The command list, exit codes and output modes below were checked against the
0.3.0 binary (`deep-diff-forge --help`, `schema exit-codes`) and
`crates/deep-diff-forge-cli/src/main.rs`. The earlier "Future Command Contract"
mixed shipped and unbuilt commands, the exit-code table was the original design
(codes 1, 5 and 8 are not emitted), and several output flags (`--pager`,
`--plain`, `--compact`, `--progress`) do not exist. Those items are now marked
planned, beside the shipped command that does the job today.

## Design Priority

1. **Bash first:** every important action has a deterministic command.
2. **Claude Code friendly:** every agent-facing command has stable output, exit codes, and documented contracts.
3. **Human readable by default:** interactive review is pleasant in a terminal.
4. **Machine readable on demand:** JSON/JSONL output exists for the non-interactive review flows.
5. **No daemon required:** CLI and library modes work without long-running services. No command starts the daemon implicitly.

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

`doctor` reports the runtime, cache and state directories and the resolved
daemon socket; it does not contact a daemon.

## Command Contract

### Shipped

```bash
# stdin patch: summary, JSON document, ranking, JSONL, side-by-side
git diff | deep-diff-forge --stdin-patch
git diff | deep-diff-forge --stdin-patch --json
git diff | deep-diff-forge --stdin-patch --rank --json
git diff | deep-diff-forge --stdin-patch --jsonl
git diff | deep-diff-forge --stdin-patch --layout side-by-side

# structured Git input (runs git itself; sees untracked files and C-quoted paths)
deep-diff-forge --git-working-tree --include-untracked --json
deep-diff-forge --git-staged --rank
deep-diff-forge --git-range main..HEAD --repo ../other --json
deep-diff-forge --git-working-tree --acquisition-json

# gate: refuse an empty or hunkless patch (exit 7)
git diff | deep-diff-forge --stdin-patch --rank --json --require-files --require-hunks

# file-level tools
deep-diff-forge semantic src/lib.rs --json
deep-diff-forge structural old.rs new.rs --json
deep-diff-forge highlight src/lib.rs --no-color

# patch + base/candidate sources → analysis.v1
deep-diff-forge analyze change.patch base/ candidate/

# interactive review, and one headless frame for agents and CI
git diff | deep-diff-forge review
git diff | deep-diff-forge review --probe --cols 120 --rows 40

# bounded local parallelism (patch + risk lanes)
git diff | deep-diff-forge --stdin-patch --cluster --parallel auto --json

# machine contract
deep-diff-forge envelope -- --stdin-patch --json < change.patch
deep-diff-forge schema exit-codes
deep-diff-forge explain 7 --json

# agent and SDK transports
deep-diff-forge mcp      # read-only MCP server over stdio
deep-diff-forge rpc      # daemon JSON-RPC over stdio, no daemon

# daemon lifecycle (start runs in the foreground; use a second terminal or a supervisor)
deep-diff-forge daemon start
deep-diff-forge daemon status
deep-diff-forge daemon stop

# local learning loop
deep-diff-forge learn status --json
```

`daemon health|status|stop` print the daemon's JSON-RPC response line; they
take no `--json` flag.

### Planned (not implemented)

Each of these exits 2 today:

```bash
deep-diff-forge old.rs new.rs                      # today: structural old.rs new.rs
deep-diff-forge --git                              # today: --git-working-tree
deep-diff-forge review --git                       # today: git diff | deep-diff-forge review
deep-diff-forge --git --jsonl --progress
deep-diff-forge plan --git --json
deep-diff-forge ingest --git --jsonl | deep-diff-forge plan --stdin --jsonl
deep-diff-forge chain --manifest .deep-diff-forge/chain.toml
deep-diff-forge cluster --git --dimensions patch,semantic,risk --parallel auto --jsonl
deep-diff-forge loom plan --source <repo> --feature "review stream"
deep-diff-forge loom gate --plan docs/loom/review-stream.json
deep-diff-forge daemon status --json
```

## Exit Codes

The shipped codes are one registry in the binary
(`crates/deep-diff-forge-core/src/exit.rs`), printed by
`deep-diff-forge schema exit-codes` and documented in the README's
[Exit codes](../README.md#exit-codes).

| Code | Name | Meaning | Next |
| --- | --- | --- | --- |
| 0 | `success` | Success. | `proceed` |
| 2 | `usage` | CLI usage / argument error. | `fix_invocation` |
| 3 | `io` | I/O or encoding failure: reading input (stdin, a file, Git, the learning store), non-UTF-8 input, or writing output. | `check_environment` |
| 4 | `invalid_input` | A patch or analysis input that fails to parse or validate, input over the byte budget, an invalid receipt, or a refused Git acquisition. | `change_input` |
| 6 | `daemon_or_terminal` | Daemon / interactive-terminal failure (for example `review` without a TTY, or `daemon status` with no daemon). | `check_environment` |
| 7 | `input_contract_refused` | `--require-files` refused a patch with 0 files (`refused: 0 files in input (--require-files)`), or `--require-hunks` refused one with 0 hunks or 0 added+removed lines (`refused: 0 hunks in input (--require-hunks)`). stdout is empty. A gate should pass both flags. | `change_input` |
| 101 | `panic` | Internal error (a panic). | `report_bug` |

The `council` subcommands use their own family: 0 completed, 4 refused, 7 hold.

Not emitted (original design only): 1 (diff found with `--exit-code`), 5 (Git
workspace failure; a Git failure is 3 or 4 today), 8 (fallback reached a hard
boundary).

## Output Modes

| Mode | Flag | Status |
| --- | --- | --- |
| Human summary | default | shipped (plain text) |
| Inline or side-by-side projection | `--layout inline\|side-by-side` | shipped |
| Ranked list | `--rank` | shipped |
| JSON | `--json` | shipped (`review.v0`, `rank.v0` with `--rank`, `cluster.v0` with `--cluster`) |
| JSONL | `--jsonl` | shipped (one `diff.file` event per file, written when the run ends) |
| Syntax colour | `highlight --color\|--no-color` | shipped for `highlight` only |
| Machine envelope | `envelope -- <command…>` | shipped (`ddf.result.v1`) |
| ANSI pager | `--pager` | planned |
| Plain text | `--no-color --plain` | planned for review output |
| JSONL progress | `--jsonl --progress` | planned |
| TOON-like compact | `--compact` | planned |

When several output flags are given, `--jsonl` wins (`stdin_patch` handles it
first), then `--cluster`, then `--rank`, then `--json`, then `--layout`
(`emit_review` in `main.rs`).

## Claude Code Agent Contract

Claude Code should be able to run:

```bash
deep-diff-forge claude-code-contract
```

Guarantees that hold today:

- Contract and machine commands do not require a TTY.
- No command starts the daemon implicitly.
- JSON documents carry a versioned `schema` name, an `input_sha256` seal over
  the exact input bytes, and the `tool` name and version.
- Patch truth remains available even when semantic analysis fails
  (`summary.semantic_fallbacks` counts the fallbacks).
- Chain, cluster and loom contracts are inspectable (`*-contract`) before their
  engines ship.
- `--cluster` joins by a deterministic order: identical output for any worker
  count.
- For MCP clients, `deep-diff-forge mcp` exposes the review as read-only tools.

Target guarantees, not yet complete:

- Every file, hunk, semantic span and annotation has a stable id. Today hunks
  carry a per-file `id` and files are keyed by `path`; JSONL events carry no
  `schema` field.
- AI annotations are marked as human, agent-grounded, agent-ungrounded or
  system-generated (the `deep-diff-forge-agent` crate models this; no CLI
  command emits annotations yet).
- Loom outputs record exemplar lessons, crate ownership, fixtures, gates and
  receipts (loom is not implemented).

## Bash Integration Patterns

### Git External Diff (planned)

`GIT_EXTERNAL_DIFF` and `git difftool` invocation are not implemented: the
binary refuses positional file pairs. Use a Git alias over `--stdin-patch` or
the structured Git selections instead.

### Aliases

```bash
alias ddf='deep-diff-forge'
alias ddfg='deep-diff-forge --git-working-tree --include-untracked'
alias ddfr='git diff | deep-diff-forge review'
git config --global alias.review '!git diff | deep-diff-forge --stdin-patch --rank'
```

### CI Gate

```bash
set -euo pipefail
CARGO_TARGET_DIR=target cargo check --workspace --locked
CARGO_TARGET_DIR=target cargo test --workspace --locked
deep-diff-forge --self-test
deep-diff-forge doctor
deep-diff-forge --git-range "origin/main..HEAD" --rank --json --require-files --require-hunks > review-rank.json
```

### Agent Review Handoff

```bash
out="$(mktemp -d)"
deep-diff-forge --git-working-tree --include-untracked --json > "$out/review.json"
deep-diff-forge --git-working-tree --include-untracked --rank --json > "$out/rank.json"
```

## Shell Safety Rules

- Never require shell aliases for correctness.
- Never write outside explicit XDG paths unless configured.
- Never mutate Git state. The Git selections run `git` with a fixed argv and
  never write the index (no `git add -N`).
- Never start the daemon implicitly from read-only commands.
- Never emit secrets in diagnostics.
- Bound memory: input is capped at the 64 MiB patch byte budget (exit 4 above
  it). Progressive streaming of large outputs is planned.

## CLI Performance Defaults

| Situation | Behaviour today |
| --- | --- |
| stdout is a TTY or a pipe | Review output is the same plain text either way. Only `highlight` adds colour, when stdout is a terminal or `--color` is passed. |
| Large diff | Read whole up to 64 MiB, then output; above the budget, exit 4. |
| Generated or vendored file | Scored 0 with the `generated_or_vendored` signal (`risk.rs`); hiding it behind a summary is planned. |
| Semantic analysis fails | Patch truth plus a counted semantic fallback. |
| Daemon unavailable | The CLI never uses the daemon; `daemon health/status/stop` exit 6. |

## Deployment Link

- Framework: [Codebase Deployment Framework](DEPLOYMENT_FRAMEWORK.md)
