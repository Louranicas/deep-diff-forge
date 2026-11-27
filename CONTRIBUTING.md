# Contributing to Deep-Diff-Forge

**You are invited.** Deep-Diff-Forge is open to everyone: first-time contributors and
veterans, people and AI-assisted workflows, Rust experts and people who have never written
Rust. You don't need permission to start or prior involvement, and no contribution is too small.
Bug reports, a sharper sentence in the docs, a failing test that proves a bug, a new
tree-sitter language, or a review of someone else's merge request all move the project forward.

This guide covers how to get started, what we ask of every change, and how a change
reaches `main`.

## Ways to contribute

| You have… | Start here |
| --- | --- |
| Found a bug | [Open a bug report](https://gitlab.com/lukeomahoney/deep-diff-forge/-/issues/new). A patch that reproduces it is the best report there is. |
| An idea | [Open a feature request](https://gitlab.com/lukeomahoney/deep-diff-forge/-/issues/new) and describe the problem before the solution. |
| An hour | Pick an [open issue on GitLab](https://gitlab.com/lukeomahoney/deep-diff-forge/-/issues), or one of the open problems below. |
| A security finding | **Don't open a public issue.** Follow [SECURITY.md](SECURITY.md) and open a confidential issue on GitLab (https://gitlab.com/lukeomahoney/deep-diff-forge/-/issues/new, tick *This issue is confidential*). |
| Docs or graphics to improve | Every `.md` file is fair game. The code is the source of truth, so check claims against it. |

### Open problems worth owning

Each of these is real, scoped, and listed as a known limit in the
[security policy](SECURITY.md#residual-risks-and-non-goals) or the
[roadmap](docs/ROADMAP.md#open-items-for-maintainers):

- **More languages.** Syntax features (`semantic`, `highlight`, `structural`) speak Rust today.
  The registry in `crates/deep-diff-forge-syntax` is built to take more grammars.
- **Directory-pair input.** `deep-diff-forge <old> <new>` compares two regular files; comparing
  two trees (nested repositories, links and special files inside them) is open; see
  [`docs/ROADMAP.md`](docs/ROADMAP.md).
- **The launchd agent on a physical Mac (on hold since 2026-11-01).** The maintainer has no real
  Mac; CI loads the agent on a hosted macOS runner only (`macOS launchd agent`). If you have a
  Mac, install `contrib/launchd/dev.deep-diff-forge.daemon.plist` as
  [operations](docs/OPERATIONS_AND_DAEMON.md#running-as-a-service) describes, use it across
  login, logout and sleep, run `git diff | deep-diff-forge review` in a real terminal, and report
  what happened in an issue.

## Getting set up

You need Rust **1.88 or newer** (edition 2024) and Git. [`just`](https://github.com/casey/just)
is optional and runs the same commands.

```bash
git clone https://gitlab.com/<you>/deep-diff-forge.git    # your fork on GitLab
cd deep-diff-forge
cargo build -p deep-diff-forge-cli
cargo test --workspace --locked
git diff | target/debug/deep-diff-forge review     # see your own change in the cockpit
```

Start with the [README](README.md), then [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) for
how the 14 crates fit together (plus `deep-diff-forge-testkit`, test-only and never published).

## The quality bar

Deep-Diff-Forge reviews untrusted code, so it holds itself to a strict bar. The aim is to
make it hard to write bad code: the gates catch problems so reviewers don't have to. Every
merge request must pass:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings -W clippy::pedantic
cargo test --workspace --locked
RUSTDOCFLAGS="-D warnings -D rustdoc::broken_intra_doc_links" cargo doc --workspace --no-deps --locked
```

`just gate-feature` runs formatting, compilation, clippy, pedantic clippy, workspace tests and
the contract probes; `just docs` runs the rustdoc check above, and `just gate-release` runs both.
`scripts/ci_local.sh` (or `just ci`) runs all 36 steps of the local gate (replicating the
`secrets`, `msrv`, `gate` and `supply-chain` CI jobs on pinned Rust 1.99.1 plus Rust 1.88.0 for
MSRV, along with the four local-only steps including the diagnostics contract and the
forge-warden contract matrix); it does not replay the Podman sandbox, diff-mutation or bench
jobs. `just install-hooks` installs the versioned gitleaks pre-push hook
(`scripts/hooks/pre-push`), which refuses a push that adds a secret.

What the gates enforce, and what we ask in review:

- **Patch truth is sacred.** Nothing may rewrite the apply-able patch. Enrichment layers read the
  model; they never write it.
- **Refuse, don't repair.** Malformed input fails with a documented exit code. It is never quietly
  normalised into something plausible.
- **No `unsafe`.** It is forbidden workspace-wide. Production code has no `unwrap` or `expect`,
  and no unexplained lint suppression.
- **A test that fails on the old code.** A fix comes with a test that fails without the fix. A
  new behaviour comes with tests of its refusals as well as its successes.
- **Mutation-tested diffs.** `scripts/diff_mutation.sh origin/main` runs `cargo-mutants` on the
  lines your change touches, and a surviving mutant fails it; run it before asking for review
  (the GitHub mirror's CI runs it in four shards on its pull requests).
- **Fuzzed parsers.** A parser, scanner, hook, journal or stdio-protocol change must survive the
  18 fuzz targets in [`fuzz/`](fuzz/) (`scripts/fuzz_target.sh --list`). The GitHub mirror's CI
  fuzzes each target briefly; run longer locally with `DDF_FUZZ_CORPUS_ROOT=<dir> scripts/fuzz_target.sh patch_parser 60`
  (or `just fuzz-campaign` across all 18 targets).
- **Docs move with code.** Contract tests read `README.md` (strictly under 400 lines, links and
  platform table), `docs/SCHEMATICS.md` (`crate-edges` block), `SECURITY.md`,
  `crates/deep-diff-forge-patch/CONFORMANCE.md` and [`docs/reference/`](docs/reference/) (exit
  codes, exit-7 refusal reasons, signal tables, diagnostics codes, hook reason classes,
  evaluation figures, MCP/daemon caps and methods). If your change alters a documented contract
  or algorithm, update the reference docs (and [`docs/ALGORITHMS_AND_HEURISTICS.md`](docs/ALGORITHMS_AND_HEURISTICS.md))
  in the same merge request.
- **A new dependency is a reviewed decision.** `cargo deny` and `cargo audit` gate licences,
  sources and advisories, and the committed SBOM must be regenerated
  (`python3 scripts/security/generate_spdx_sbom.py`).

## How a change reaches `main`

GitLab, <https://gitlab.com/lukeomahoney/deep-diff-forge>, is the place of contribution and of
release. GitHub, <https://github.com/Louranicas/deep-diff-forge>, is a mirror: it takes pull
requests only to bring its `main` up to date with GitLab's, so a contribution opened there is not
merged there.

1. Fork the repository on GitLab and create a branch from `main`.
2. Make focused commits. Messages follow [Conventional Commits](https://www.conventionalcommits.org/)
   and describe the behaviour, for example
   `fix(daemon): a second start never replaces a live daemon's socket`.
3. Open a merge request on GitLab against `main`. Draft merge requests are welcome for early
   feedback.
4. GitLab runs no CI for this project. Before a merge the maintainer runs the local gate,
   `scripts/ci_local.sh`, on the merge request's head commit; it must end with
   `steps failed: 0`. The maintainer merges.
5. Add an entry under `## [Unreleased]` in [CHANGELOG.md](CHANGELOG.md) when users would notice
   the change.

On the GitHub mirror, `main` is protected by a branch ruleset whose required checks (the gate,
supply chain, review-controller evidence, secret scan, Rust 1.88 compatibility, macOS tests, the
bench comparison, four diff-mutation shards and CodeQL) run on the mirror's pull requests, with
`Linux arm64 tests` and `macOS launchd agent` as jobs that are not required. A merge request on
GitLab is tested on Linux x86_64 only, by the local gate
([supported platforms](README.md#supported-platforms)).

## Licence

Deep-Diff-Forge is dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your
option. Unless you explicitly state otherwise, any contribution you intentionally submit for
inclusion is dual-licensed under the same terms, without any additional terms or conditions.

## Conduct

Everyone who participates agrees to the [Code of Conduct](CODE_OF_CONDUCT.md). In short: be kind,
assume good intent, and critique the code, not the person.

Thank you for being here.
