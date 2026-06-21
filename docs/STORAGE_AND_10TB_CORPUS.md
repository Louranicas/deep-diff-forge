# Storage And 10TB Corpus Policy

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

