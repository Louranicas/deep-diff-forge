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
- [ ] `CARGO_TARGET_DIR=target cargo clippy --workspace --all-targets --locked -- -D warnings`
- [ ] `CARGO_TARGET_DIR=target cargo clippy --workspace --all-targets --locked -- -D warnings -W clippy::pedantic`
- [ ] `CARGO_TARGET_DIR=target cargo test --workspace --locked`
- [ ] `cargo check --locked --manifest-path fuzz/Cargo.toml --bins`
- [ ] `cargo deny check` and `cargo deny --manifest-path fuzz/Cargo.toml --locked check`
- [ ] `cargo audit --deny warnings` for both `Cargo.lock` and `fuzz/Cargo.lock`
- [ ] `python3 scripts/security/daemon_soak.py`
- [ ] `python3 scripts/security/privacy_probe.py`
- [ ] Regenerated `sbom.spdx.json` matches the committed dependency graph
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

## GitHub Actions

Implemented workflows:

- `ci.yml`: formatting, compilation, strict and pedantic lint, tests, docs,
  contracts, fuzz-harness compilation, daemon/privacy probes, root and fuzz
  audits, policy checks, and SPDX SBOM drift detection.
- `release.yml`: tag/version validation, repeated audit and policy gates, locked
  build, SHA-256 generation, GitHub build-provenance attestations, release
  upload, and crates.io publication.

Both workflows pin third-party Actions to full commit SHAs and use explicit
least-privilege permissions. Release publication is tag-only; the tag must
exactly match the workspace version. See [`SECURITY.md`](../SECURITY.md) for the
threat model and residual account-level controls.

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

GitHub is the primary remote:

```text
https://github.com/Louranicas/deep-diff-forge
```

GitLab mirror is live under the authenticated namespace (`lukeomahoney`, not the
GitHub username `Louranicas`), created via push-to-create and private by default:

```text
git@gitlab.com:lukeomahoney/deep-diff-forge.git
https://gitlab.com/lukeomahoney/deep-diff-forge
```

Both remotes are pushed in lock-step:

```bash
git push github main
git push gitlab main
```

Note: the GitLab namespace differs from GitHub. The earlier "project not found"
blocker was a namespace mismatch (`Louranicas` is the GitHub user; the GitLab
account is `lukeomahoney`) — resolved by repointing the remote.

## Deployment Link

- Framework: [Codebase Deployment Framework](DEPLOYMENT_FRAMEWORK.md)
