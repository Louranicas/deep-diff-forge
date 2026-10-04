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

## Gate Contract (`--stdin-patch`)

A script that turns a review document into a pass/fail verdict must not read
"nothing to review" as "nothing wrong". Two guards make that impossible; pass
both:

```bash
git diff | deep-diff-forge --stdin-patch --rank --json --require-files --require-hunks
```

| Guard | Refuses when | stderr line |
| --- | --- | --- |
| `--require-files` | the well-formed patch describes 0 files (empty or mis-piped input) | `refused: 0 files in input (--require-files)` |
| `--require-hunks` | the patch has 0 hunks or 0 added+removed lines across all files (header-only `diff --git a/x b/x`, rename-only, context-only hunks) | `refused: 0 hunks in input (--require-hunks)` |

Rules:

- A refusal exits **7**, writes one line to stderr, and leaves stdout empty.
- `--require-files` alone passes a header-only or rename-only diff (one file,
  zero hunks); only `--require-hunks` closes that gap.
- When both guards would fail, the files guard reports first.
- A parse failure is still exit 4; the guards never mask it.
- Without either flag the default is unchanged: nothing to review → exit 0.

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
  "input_sha256": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
  "tool": {"name": "deep-diff-forge", "version": "0.2.1"},
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
