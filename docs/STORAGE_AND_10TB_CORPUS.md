# Storage And 10TB Corpus Policy

> **Status 2026-11-25 (`v0.11.0`).** Shipped: none of the `[storage]` configuration, large cache,
> or 10TB corpus mirror is built in the engine; the safety rules ("Corpus Ingestion Rules",
> "Destructive Boundary") remain current operator policy. Planned, unowned: the `[storage]`
> configuration, large cache, and corpus mirror. Nothing is withdrawn or on hold.

## Status 2026-11-25 (v0.11.0)

**Kind:** operator policy for the workstation's optional storage disk. The safety rules ("Corpus Ingestion Rules", "Destructive Boundary") are current. The paths, config keys, and large cache below are not built.

**At `v0.11.0`** (verified in code and with the binary):

- The engine reads no `[storage]` configuration. `large_cache_dir` and `large_corpus_dir` appear in no crate. The engine writes nothing to this disk, and it has no corpus mirror or benchmark archive.
- The engine writes no diff or AST cache (`daemon.health` reports `cache_entries: 0`). `deep-diff-forge doctor` prints `cache_dir`, `state_dir` (the learning store's resolved base; a relative `XDG_STATE_HOME` is ignored), and the daemon socket path. Persistent local state at `v0.11.0` consists only of the local-only learning store (`$XDG_STATE_HOME/deep-diff-forge/learning/`, `learn record`), agent hook session state (`deep-diff-forge.hook-state.v13`), and the per-repository project frontier ledger under `.git/deep-diff-forge/`.
- Repository test fixtures live under `fixtures/` (`corpus/`, `council/`, `diagnostics/`, `eval/`, `eval-refused-by-parser/`, `patch/`, `protocol/`). An optional external comparison corpus (`DDF_COMPARE_CORPUS`) is checked only if present on disk during `scripts/ci_local.sh`.
- On the operator workstation the `STORAGE-10TB` disk is mounted at `/var/mnt/storage`, not `/mnt/storage-10tb` (measured with `findmnt` on 2026-10-28). Every absolute path in this document is an operator-workstation path and is not referenced by the repository code.

Current docs: [operations and daemon](OPERATIONS_AND_DAEMON.md), [learning loop](LEARNING_LOOP.md).

The local 10TB drive is mounted as:

```text
/mnt/storage-10tb
```

Observed identity:

```text
label: STORAGE-10TB
filesystem: ext4
mount: /mnt/storage-10tb
```

Deep-Diff-Forge may use this disk as a corpus and archive source, but not as a required runtime dependency.

## Roles

| Role | Location | Required? |
| --- | --- | --- |
| Small fixtures | `fixtures/` inside repo | Yes |
| Large corpus mirror | `/mnt/storage-10tb/deep-diff-forge-corpus/` | No |
| Benchmark archives | `/mnt/storage-10tb/deep-diff-forge-benchmarks/` | No |
| Release receipts archive | `/mnt/storage-10tb/deep-diff-forge-receipts/` | No |
| Local cache | `$XDG_CACHE_HOME/deep-diff-forge/` | No, rebuildable |

## Corpus Sources On 10TB

Useful exemplar sources already present:

- `/mnt/storage-10tb/repos/difftastic`
- `/mnt/storage-10tb/repos/hunk`
- `/mnt/storage-10tb/no-mistakes`
- `/mnt/storage-10tb/devops_engine_v2`
- `/mnt/storage-10tb/claude-optimized-deployment`
- `/mnt/storage-10tb/the_maintenance_engine`
- `/mnt/storage-10tb/the_code_synthor_v7`

These should be sampled through manifests, not blindly copied into the repo.

## Corpus Ingestion Rules

1. Read-only by default.
2. Generate a manifest before copying.
3. Copy only minimal fixtures needed for tests.
4. Store large source snapshots outside Git.
5. Record source path, commit hash when available, file hash, and license note.
6. Never include private or sensitive content in public fixtures.

## Manifest Format

```text
corpus_id: difftastic-sample-rust-001
source_path: /mnt/storage-10tb/repos/difftastic/sample_files/rust_1.rs
source_repo: /mnt/storage-10tb/repos/difftastic
source_commit: unknown
sha256: <hash>
license: verify-before-publication
purpose: structural diff regression
copied_to: fixtures/corpus/rust/reformat_001_before.rs
```

## Cache Policy

Cache data is rebuildable and can be pruned.

Default local cache:

```text
~/.cache/deep-diff-forge/
```

Large optional cache:

```text
/mnt/storage-10tb/deep-diff-forge-cache/
```

The engine must not write to the 10TB disk unless explicitly configured:

```toml
[storage]
large_cache_dir = "/mnt/storage-10tb/deep-diff-forge-cache"
large_corpus_dir = "/mnt/storage-10tb/deep-diff-forge-corpus"
```

## Destructive Boundary

Deep-Diff-Forge must never format, repartition, delete broad directories, or prune unrelated data on `/mnt/storage-10tb`.

Allowed operations:

- read source files
- write under explicitly configured `deep-diff-forge-*` directories
- prune only engine-owned cache directories

Forbidden operations:

- deleting arbitrary 10TB paths
- modifying exemplar repos in place
- running recursive cleanup outside engine-owned directories
- assuming the 10TB disk exists on other machines

## Deployment Link

- Framework: [Codebase Deployment Framework](DEPLOYMENT_FRAMEWORK.md)
