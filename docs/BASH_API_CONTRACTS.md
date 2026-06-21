# Bash API Contracts

Deep-Diff-Forge must be easy to drive from strict Bash scripts.

## Contract Rules

- All commands work under `set -euo pipefail`.
- Use stdout for primary output.
- Use stderr for diagnostics.
- Do not prompt unless command is explicitly interactive.
- Exit codes are stable.
- `--json` is single JSON document.
- `--jsonl` is newline-delimited event stream.
- Large outputs stream progressively.

## Stable Bootstrap Contract

These commands are available now:

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

## Justfile Runner Contract

The repo-local `justfile` provides deployment shortcuts for humans, agents, CI
operators, and Zellij panes. These recipes wrap documented commands; they do
not create a separate product API.

```bash
just status
just gate-docs
just gate-bootstrap
just gate-feature
just test-audit
just contracts
just doctor
just receipt-bootstrap
```

Rules:

- `CARGO_TARGET_DIR` is pinned to repo-local `target`.
- Habitat and Zellij recipes are read-only and advisory.
- Generated receipts go under `reports/` and are ignored by Git.
- Product behavior still belongs to `deep-diff-forge`, not `just`.

## Planned JSON Shapes

### Review Document

```json
{
  "schema": "deep-diff-forge.review.v0",
  "files": [],
  "summary": {
    "files_changed": 0,
    "additions": 0,
    "deletions": 0,
    "semantic_fallbacks": 0
  }
}
```

### Planner Decision

```json
{
  "schema": "deep-diff-forge.plan.v0",
  "file": "src/lib.rs",
  "strategy": "syntax",
  "fallback": null,
  "budget": "balanced",
  "explanation": ["small supported Rust file", "parser budget available"]
}
```

### JSONL Progress Event

```json
{"event":"diff.file.updated","file":"src/lib.rs","patch":"ready","semantic":"ready"}
```

## Shell Completion Plan

Completion files should be generated into:

```text
completions/deep-diff-forge.bash
completions/deep-diff-forge.zsh
completions/deep-diff-forge.fish
```

No shell completion may be required for command correctness.

## Claude Code Invocation Examples

```bash
# Fast machine-readable workspace review
deep-diff-forge --git --json --budget fast

# Deep semantic review for selected files
deep-diff-forge src/lib.rs src/lib.rs.new --semantic --budget deep --json

# Get only strategy decisions
deep-diff-forge plan --git --json

# Ask why a hunk is ranked first
deep-diff-forge why-first hunk:42 --json

# Compose as Unix filters
deep-diff-forge ingest --git --jsonl \
  | deep-diff-forge plan --stdin --jsonl \
  | deep-diff-forge rank --stdin --json \
  | deep-diff-forge render --stdin --plain

# Run bounded local parallelism
deep-diff-forge cluster --git --dimensions patch,semantic,risk --parallel 4 --json

# Produce a loom assimilation plan
deep-diff-forge loom plan --source /mnt/storage-10tb/repos/difftastic --feature "syntax fallback"
```

## Deployment Link

- Framework: [Codebase Deployment Framework](DEPLOYMENT_FRAMEWORK.md)
