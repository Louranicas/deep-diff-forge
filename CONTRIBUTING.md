# Contributing to Deep-Diff-Forge

**You are invited.** Deep-Diff-Forge is open to everyone: first-time contributors and
veterans, people and AI-assisted workflows, Rust experts and people who have never written
Rust. You don't need permission to start or prior involvement, and no contribution is too small.
Bug reports, a sharper sentence in the docs, a failing test that proves a bug, a new
tree-sitter language, or a review of someone else's pull request all move the project forward.

This guide covers how to get started, what we ask of every change, and how a change
reaches `main`.

## Ways to contribute

| You have… | Start here |
| --- | --- |
| Found a bug | [Open a bug report](https://github.com/Louranicas/deep-diff-forge/issues/new?template=bug_report.yml). A patch that reproduces it is the best report there is. |
| An idea | [Open a feature request](https://github.com/Louranicas/deep-diff-forge/issues/new?template=feature_request.yml) and describe the problem before the solution. |
| An hour | Pick an issue labelled [`good first issue`](https://github.com/Louranicas/deep-diff-forge/labels/good%20first%20issue) or [`help wanted`](https://github.com/Louranicas/deep-diff-forge/labels/help%20wanted), or one of the open problems below. |
| A security finding | **Don't open a public issue.** Follow [SECURITY.md](SECURITY.md) and use a private advisory. |
| Docs or graphics to improve | Every `.md` file is fair game. The code is the source of truth, so check claims against it. |

### Open problems worth owning

Each of these is real, scoped, and listed as a known limit in the
[production-readiness notes](SECURITY.md):

- **A TUI test harness.** The cockpit is tested through `review --probe` frames; a harness that
  drives key sequences would catch interaction bugs.
- **Peer credentials on the daemon socket.** Check `SO_PEERCRED` on Linux and `getpeereid` on
  macOS, in addition to the `0700` directory and `0600` socket.
- **A lock file for `daemon start`.** Two starts racing between the stale-socket probe and the bind
  can still collide.
- **More languages.** Syntax features (`semantic`, `highlight`, `structural`) speak Rust today.
  The registry in `crates/deep-diff-forge-syntax` is built to take more grammars.
- **File-pair input.** `deep-diff-forge <old> <new>` is refused today; see
  [`docs/ROADMAP.md`](docs/ROADMAP.md).

## Getting set up

You need Rust **1.88 or newer** (edition 2024) and Git. [`just`](https://github.com/casey/just)
is optional and runs the same commands.

```bash
git clone https://github.com/<you>/deep-diff-forge.git
cd deep-diff-forge
cargo build -p deep-diff-forge-cli
cargo test --workspace --locked
git diff | target/debug/deep-diff-forge review     # see your own change in the cockpit
```

Start with the [README](README.md), then [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) for
how the 14 crates fit together.

## The quality bar

Deep-Diff-Forge reviews untrusted code, so it holds itself to a strict bar. The aim is to
make it hard to write bad code: the gates catch problems so reviewers don't have to. Every
pull request must pass:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings -W clippy::pedantic
cargo test --workspace --locked
```

`just gate-feature` runs those plus the contract probes. `scripts/ci_local.sh` (or `just ci`)
replicates every CI gate job on your machine, including the supply-chain checks and the SBOM.

What the gates enforce, and what we ask in review:

- **Patch truth is sacred.** Nothing may rewrite the apply-able patch. Enrichment layers read the
  model; they never write it.
- **Refuse, don't repair.** Malformed input fails with a documented exit code. It is never quietly
  normalised into something plausible.
- **No `unsafe`.** It is forbidden workspace-wide. Production code has no `unwrap` or `expect`,
  and no unexplained lint suppression.
- **A test that fails on the old code.** A fix comes with a test that fails without the fix. A
  new behaviour comes with tests of its refusals as well as its successes.
- **Mutation-tested diffs.** CI runs `cargo-mutants` on the lines your pull request changes (four
  shards), and a surviving mutant fails the check. Run `scripts/diff_mutation.sh origin/main`
  locally to see survivors before CI does.
- **Fuzzed parsers.** A parser change must survive the fuzz targets in [`fuzz/`](fuzz/). CI fuzzes
  each target briefly; run longer locally (`cargo +nightly fuzz run patch_parser`).
- **Docs move with code.** Several tests read the README (exit codes, caps, daemon methods). If
  your change alters a documented contract, update the doc in the same pull request.
- **A new dependency is a reviewed decision.** `cargo deny` and `cargo audit` gate licences,
  sources and advisories, and the committed SBOM must be regenerated
  (`python3 scripts/security/generate_spdx_sbom.py`).

## How a change reaches `main`

1. Fork the repository and create a branch from `main`.
2. Make focused commits. Messages follow [Conventional Commits](https://www.conventionalcommits.org/)
   and describe the behaviour, for example
   `fix(daemon): a second start never replaces a live daemon's socket`.
3. Open a pull request and fill in the template. Draft pull requests are welcome for early
   feedback.
4. `main` is protected. Merging needs a pull request, a branch that is up to date with `main`,
   and **nine required checks**:
   - the gate;
   - supply chain;
   - review-controller evidence;
   - secret scan;
   - Rust 1.88 compatibility;
   - four diff-mutation shards.

   Nobody can bypass them, including the maintainer.
5. Add an entry under `## [Unreleased]` in [CHANGELOG.md](CHANGELOG.md) when users would notice
   the change.

Changes to sockets or platform code should also pass the macOS job, which runs on schedule and
on demand rather than on every pull request. Say so in your PR and a maintainer will run it.

## Licence

Deep-Diff-Forge is dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your
option. Unless you explicitly state otherwise, any contribution you intentionally submit for
inclusion is dual-licensed under the same terms, without any additional terms or conditions.

## Conduct

Everyone who participates agrees to the [Code of Conduct](CODE_OF_CONDUCT.md). In short: be kind,
assume good intent, and critique the code, not the person.

Thank you for being here.
