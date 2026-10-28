# Bash API Contracts

Deep-Diff-Forge must be easy to drive from strict Bash scripts.

## Status 2026-10-28 (0.3.0)

Everything under "Shipped" was checked against the 0.3.0 binary and
`crates/deep-diff-forge-cli/src/main.rs`. The earlier "Claude Code invocation
examples" used commands and flags that do not exist (`--git`, `--budget`,
`--semantic`, `plan`, `why-first`, `ingest`, `rank`, `render`, `cluster`,
`loom plan`); they now sit under [Planned](#planned-not-implemented), next to the
shipped command that does the job today.

## Contract Rules (shipped)

- Commands work under `set -euo pipefail`; a failure is a non-zero exit.
- stdout carries the primary output; stderr carries diagnostics. On an error,
  stdout stays empty.
- No command prompts, except `review` (the interactive cockpit, which exits 6
  without a terminal; use `review --probe` headless).
- Exit codes come from one registry: `deep-diff-forge schema exit-codes`.
- `--json` prints one JSON document; `--jsonl` prints one JSON event per line.
- Input is read whole, under a 64 MiB byte budget (`DEFAULT_BYTE_BUDGET` in
  `crates/deep-diff-forge-patch/src/lib.rs`), and output is written when the
  run completes. There is no progressive streaming yet.
- Bulk output tolerates a closed pipe: `deep-diff-forge … | head` exits cleanly.

## Exit Codes (shipped)

| Code | Name | Meaning |
| --- | --- | --- |
| 0 | `success` | Success. |
| 2 | `usage` | CLI usage / argument error (including an unknown command or option). |
| 3 | `io` | I/O or encoding failure: reading input (stdin, a file, Git, the learning store), non-UTF-8 input, or writing output. |
| 4 | `invalid_input` | A patch or analysis input that fails to parse or validate, input over the byte budget, an invalid receipt, or a refused Git acquisition. |
| 6 | `daemon_or_terminal` | Daemon / interactive-terminal failure. |
| 7 | `input_contract_refused` | Input contract refused (see the gate contract below). |
| 101 | `panic` | Internal error (a panic). |

`council` subcommands use their own family (0 completed, 4 refused, 7 hold).
`deep-diff-forge explain <code> [--family engine|council] [--json]` names one
code.

## Stable Bootstrap Contract

These commands take no arguments and need no TTY or daemon:

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

The four `*-contract` commands print guarantees as text. `chain-contract`,
`cluster-contract` and `loom-contract` describe engines that are mostly planned:
see [CHAINING_AND_CLUSTERING.md](CHAINING_AND_CLUSTERING.md).

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
- Text that is not a patch at all (for example `echo garbage`) parses to 0 files
  and exits 0 unless `--require-files` is passed (measured on 0.3.0).

The same guards apply to the structured Git selections
(`--git-working-tree`, `--git-staged`, `--git-range A..B`) and to the daemon's
`diff.compute` (`require_files`, `require_hunks`).

## Machine Envelope (shipped)

To get any command's result as one JSON object:

```bash
deep-diff-forge envelope -- --stdin-patch --json < change.patch
# {"command":["--stdin-patch","--json"],"exit_code":0,"family":"engine","ok":true,"outcome":"success","schema":"ddf.result.v1","stderr":"","stdout":{…},…}
deep-diff-forge schema result        # JSON Schema of ddf.result.v1
deep-diff-forge schema exit-codes    # ddf.exit-codes.v1 registry
```

`envelope` exits with the wrapped command's code.

## Justfile Runner Contract

The repo-local `justfile` provides shortcuts for humans, agents, CI operators
and Zellij panes. The recipes wrap documented commands; they are not a separate
product API.

```bash
just status
just gate-docs          # fmt + check
just gate-bootstrap     # fmt + check + contracts
just gate-feature       # fmt + check + clippy + pedantic + test + contracts
just ci                 # local replica of the CI gate jobs (scripts/ci_local.sh)
just contracts
just doctor
just receipt-bootstrap
```

Rules:

- `CARGO_TARGET_DIR` is pinned to repo-local `target`.
- Habitat and Zellij recipes are read-only and advisory.
- Generated receipts go under `reports/` and are ignored by Git.
- Product behaviour still belongs to `deep-diff-forge`, not `just`.

## JSON Shapes

### Review document (shipped, `--stdin-patch --json`)

Measured on 0.3.0 for a one-line change:

```json
{
  "schema": "deep-diff-forge.review.v0",
  "input_sha256": "6369d5c655af20b18347f87a6f5a73da2096dd8c15d842332c62e9201ad06c52",
  "tool": {"name": "deep-diff-forge", "version": "0.3.0"},
  "files": [
    {
      "path": "x.rs",
      "status": "modified",
      "strategy": "line",
      "hunks": [
        {
          "id": 0,
          "old_start": 1,
          "new_start": 1,
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

### Rank document (shipped, `--stdin-patch --rank --json`)

```json
{
  "schema": "deep-diff-forge.rank.v0",
  "input_sha256": "6369d5c655af20b18347f87a6f5a73da2096dd8c15d842332c62e9201ad06c52",
  "tool": {"name": "deep-diff-forge", "version": "0.3.0"},
  "ranked": [
    {"path": "x.rs", "status": "modified", "score": 2, "signals": []}
  ]
}
```

### JSONL event (shipped, `--stdin-patch --jsonl`)

One line per file:

```json
{"event":"diff.file","path":"x.rs","status":"modified","additions":1,"deletions":1,"hunks":1}
```

Other shipped schemas: `deep-diff-forge.cluster.v0` (`--cluster --json`),
`deep-diff-forge.semantic.v0` (`semantic --json`),
`deep-diff-forge.structural.v0` (`structural --json`),
`deep-diff-forge.learning.v0` (`learn status --json`),
`deep-diff-forge.git-acquisition.v0` (`--acquisition-json`),
`deep-diff-forge.analysis.v1` (`analyze`), `ddf.result.v1` (`envelope`),
`ddf.exit-codes.v1` (`schema exit-codes`).

### Planner decision (planned, not implemented)

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

No command emits `plan.v0` today. The review document's per-file `strategy`
field carries the chosen strategy.

### Progress event (planned, not implemented)

```json
{"event":"diff.file.updated","file":"src/lib.rs","patch":"ready","semantic":"ready"}
```

## Shell Completion (planned, not implemented)

Completion files would be generated into `completions/deep-diff-forge.{bash,zsh,fish}`.
No `completions/` directory exists yet. No shell completion may be required for
command correctness.

## Invocation Examples

### Shipped

```bash
# Machine-readable review of the working tree, untracked files included
deep-diff-forge --git-working-tree --include-untracked --json

# Risk-ranked review of a commit range, as a gate
deep-diff-forge --git-range main..HEAD --rank --json --require-files --require-hunks

# Symbols of one file, and a reformat-aware comparison of two versions
deep-diff-forge semantic src/lib.rs --json
deep-diff-forge structural old.rs new.rs --json

# Bounded local parallelism (patch + risk lanes, deterministic join)
git diff | deep-diff-forge --stdin-patch --cluster --parallel 4 --json

# One daemon-protocol request without a daemon
printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"engine.methods"}' | deep-diff-forge rpc
```

`--parallel` takes `serial`, `auto` (the default) or a worker count from 1 (anything else exits 2); a fixed count is
capped at four times the CPU count.

### Planned (not implemented)

Each of these exits 2 today:

```bash
deep-diff-forge --git --json --budget fast
deep-diff-forge src/lib.rs src/lib.rs.new --semantic --budget deep --json
deep-diff-forge plan --git --json
deep-diff-forge why-first hunk:42 --json
deep-diff-forge ingest --git --jsonl \
  | deep-diff-forge plan --stdin --jsonl \
  | deep-diff-forge rank --stdin --json \
  | deep-diff-forge render --stdin --plain
deep-diff-forge cluster --git --dimensions patch,semantic,risk --parallel 4 --json
deep-diff-forge loom plan --source <repo> --feature "syntax fallback"
```

## Deployment Link

- Framework: [Codebase Deployment Framework](DEPLOYMENT_FRAMEWORK.md)
