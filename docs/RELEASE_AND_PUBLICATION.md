# Release And Publication Plan

This document defines how Deep-Diff-Forge moves from commit to public artifact.

## Release Channels

| Channel | Purpose |
| --- | --- |
| `main` | Current development branch. |
| GitHub Releases | Primary binary release and project visibility. |
| GitLab mirror | Secondary remote and redundancy. |
| crates.io | Rust library and CLI distribution. |
| cargo-binstall | Fast binary installation. |
| package managers | Later, after CLI stabilizes. |

## Versioning

Use semantic versioning:

```text
0.1.0: model and CLI bootstrap
0.2.0: patch parser and projections
0.3.0: pager-compatible CLI
0.4.0: syntax layer
0.5.0: TUI review
0.6.0: daemon and cache
1.0.0: stable CLI, model, and daemon API
```

## Pre-Release Checklist

- [ ] `cargo fmt --all --check`
- [ ] `CARGO_TARGET_DIR=target cargo check --workspace --locked`
- [ ] `CARGO_TARGET_DIR=target cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `CARGO_TARGET_DIR=target cargo test --workspace --locked`
- [ ] Corpus regression snapshots pass
- [ ] CLI smoke passes
- [ ] Daemon smoke passes if daemon is included
- [ ] Docs updated
- [ ] `CHANGELOG.md` updated
- [ ] Release receipt created

## Tag And Push

```bash
version=0.1.0
git tag -a "v${version}" -m "Deep-Diff-Forge v${version}"
git push github main "v${version}"
git push gitlab main "v${version}"
```

GitLab publication is conditional on a valid GitLab project and credentials.

## Artifact Layout

```text
dist/
  deep-diff-forge-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz
  deep-diff-forge-vX.Y.Z-x86_64-apple-darwin.tar.gz
  deep-diff-forge-vX.Y.Z-aarch64-apple-darwin.tar.gz
  deep-diff-forge-vX.Y.Z-x86_64-pc-windows-msvc.zip
  checksums.txt
  checksums.txt.sig
```

## GitHub Actions Plan

Required workflows:

- `ci.yml`: fmt, check, clippy, tests, corpus snapshots
- `release.yml`: tagged build matrix and release upload
- `docs.yml`: build docs when docs site exists
- `security.yml`: cargo audit, dependency review, supply-chain checks

## Publication Receipts

Each release writes:

```text
reports/releases/vX.Y.Z/
  source.txt
  remotes.txt
  checks.txt
  tests.txt
  corpus.txt
  package.txt
  checksums.txt
  publish.txt
```

## Mirror Policy

GitHub is primary today because the repo exists there:

```text
https://github.com/Louranicas/deep-diff-forge
```

GitLab mirror is configured locally as:

```text
git@gitlab.com:Louranicas/deep-diff-forge.git
```

Current blocker:

```text
GitLab project not found or local machine lacks permission.
```

Once fixed:

```bash
git push -u gitlab main
```

## Deployment Link

- Framework: [Codebase Deployment Framework](DEPLOYMENT_FRAMEWORK.md)
