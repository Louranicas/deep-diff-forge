# Claude Code, Bash, And CLI Optimization

Deep-Diff-Forge is optimized first for Claude Code, Bash, and CLI workflows. GUI and IDE surfaces are projections over the same contracts, not separate product truths.

## Design Priority

1. **Bash first:** every important action has a deterministic command.
2. **Claude Code friendly:** every agent-facing command has stable output, exit codes, and documented contracts.
3. **Human readable by default:** interactive review is pleasant in a terminal.
4. **Machine readable on demand:** JSON/JSONL output exists for all non-interactive flows.
5. **No daemon required:** CLI and library modes work without long-running services.

## Bootstrap Commands

Current deployability smoke commands:

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

These commands exist before the full engine so CI, Claude Code, and Bash scripts have a stable handshake.

## Future Command Contract

```bash
# file pair
deep-diff-forge old.rs new.rs

# stdin patch
git diff | deep-diff-forge --stdin-patch

# git workspace
deep-diff-forge --git

# interactive review
deep-diff-forge review --git

# agent-safe JSON
deep-diff-forge --git --json

# progressive JSONL stream
deep-diff-forge --git --jsonl --progress

# explain planner decisions
deep-diff-forge plan --git --json

# validate daemon and cache
deep-diff-forge doctor

# chainable local pipeline
deep-diff-forge ingest --git --jsonl | deep-diff-forge plan --stdin --jsonl
deep-diff-forge chain --manifest .deep-diff-forge/chain.toml

# clustered dimensional execution
deep-diff-forge cluster --git --dimensions patch,semantic,risk --parallel auto --jsonl

# loom assimilation
deep-diff-forge loom plan --source /mnt/storage-10tb/repos/hunk --feature "review stream"
deep-diff-forge loom gate --plan docs/loom/review-stream.json

# daemon lifecycle
deep-diff-forge daemon start
deep-diff-forge daemon status --json
deep-diff-forge daemon stop
```

## Exit Codes

| Code | Meaning |
| --- | --- |
| 0 | Success. |
| 1 | Diff found when `--exit-code` is requested. |
| 2 | CLI usage or argument error. |
| 3 | Input read or decode failure. |
| 4 | Patch parse failure. |
| 5 | Git workspace failure. |
| 6 | Daemon or IPC failure. |
| 7 | Contract violation. |
| 8 | Internal recoverable fallback reached hard boundary. |
| 101 | Panic or unrecoverable internal bug. |

## Output Modes

| Mode | Flag | Audience |
| --- | --- | --- |
| Human terminal | default | Developer review. |
| ANSI pager | `--pager` or stdout TTY detection | Git pager use. |
| Plain text | `--no-color --plain` | Logs and minimal terminals. |
| JSON | `--json` | Claude Code, CI, scripts. |
| JSONL | `--jsonl --progress` | Streaming agent workflows. |
| TOON-like compact | `--compact` | Token-efficient agent handoff. |

## Claude Code Agent Contract

Claude Code should be able to run:

```bash
deep-diff-forge claude-code-contract
```

Required guarantees:

- Contract commands do not require a TTY.
- Contract commands do not start the daemon implicitly.
- Future JSON schemas are versioned.
- Every file, hunk, semantic span, and annotation has a stable id.
- AI annotations are explicitly marked as human, agent-grounded, agent-ungrounded, or system-generated.
- Patch truth remains available even when semantic analysis fails.
- Chain, cluster, and loom contracts are inspectable before their full engines ship.
- Clustered execution joins by stable IDs and deterministic order unless ranking is explicitly requested.
- Loom outputs record exemplar lessons, crate ownership, fixtures, gates, and receipts.

## Bash Integration Patterns

### Git External Diff

```bash
git config --global diff.deep-diff-forge.command 'deep-diff-forge'
git config --global diff.tool deep-diff-forge
```

### Pager Alias

```bash
alias ddf='deep-diff-forge'
alias ddfg='deep-diff-forge --git'
alias ddfr='deep-diff-forge review --git'
```

### CI Gate

```bash
set -euo pipefail
CARGO_TARGET_DIR=target cargo check --workspace --locked
CARGO_TARGET_DIR=target cargo test --workspace --locked
deep-diff-forge --self-test
deep-diff-forge doctor
```

### Agent Review Handoff

```bash
deep-diff-forge --git --json > /tmp/deep-diff-forge-review.json
deep-diff-forge plan --git --json > /tmp/deep-diff-forge-plan.json
```

## Shell Safety Rules

- Never require shell aliases for correctness.
- Never write outside explicit XDG paths unless configured.
- Never mutate Git state unless the command name says so.
- Never start daemon implicitly from read-only commands.
- Never emit secrets in diagnostics.
- Stream large outputs; do not buffer unbounded diffs in memory.

## CLI Performance Defaults

| Situation | Default behavior |
| --- | --- |
| stdout is TTY | Human ANSI output. |
| stdout is pipe | Plain or JSON-compatible output depending on flag. |
| large diff | Progressive file-first output. |
| huge generated file | Suppress with summary unless user expands. |
| parser budget exceeded | Patch truth plus semantic fallback reason. |
| daemon unavailable | Run in-process unless daemon explicitly requested. |
