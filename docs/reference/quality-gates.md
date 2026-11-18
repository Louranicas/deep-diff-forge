# Quality gates

Deep-Diff-Forge reviews untrusted code, so it is built to make bad code hard to write. Up to
0.8.3, `main` changed only through a pull request, on a branch that is up to date, after
**thirteen required checks** that nobody could bypass. From 0.8.4 nothing runs at GitHub: `main`
moves on GitLab, and the release commit passes the local gate (`scripts/ci_local.sh`, the steps of
the `secrets`, `msrv`, `gate` and `supply-chain` jobs), run by the owner's release script in a
fresh clone of that commit before the tag; `just release-tag` itself runs only `gate-release`,
a subset of it (the procedure is in [Tag And Push](../RELEASE_AND_PUBLICATION.md#tag-and-push)); the macOS, diff-mutation, bench and
CodeQL checks below do not run for it.

| Required check | What it proves |
| --- | --- |
| `deploy gate-feature` | fmt, check, clippy `-D warnings`, pedantic, all tests, docs, contract probes, fuzz-harness compile, hostile-daemon soak, learning-privacy probes, Python SDK against the built binary |
| `diff mutation` × 4 shards | `cargo-mutants` on the lines the PR changes; one surviving mutant fails it |
| `review controller mutation and fuzz evidence` | scoped mutation of the council's admission code; a fuzz smoke for every target; 12 planted-breach probes against the rootless Podman sandbox |
| `supply chain audit` | `cargo-deny` (bans, licences, sources, advisories), strict `cargo-audit`, and the committed SPDX SBOM regenerated and compared |
| `declared Rust 1.88 compatibility` | the MSRV still builds |
| `macOS tests` | the whole suite on macOS |
| `secret scan (gitleaks)` | a pinned, checksum-verified gitleaks over the full history |
| `bench (base vs head)` | the merge base and the head benchmarked on one runner; a workload over 25% and 0.2 ms slower fails |
| `codeql (rust)`, `codeql (actions)` | CodeQL security-extended queries over the Rust code and the workflows (required since 2026-11-02) |

Across the tree:
- **No `unsafe`.** It is forbidden in every crate.
- **No production `unwrap` or `expect`.** No suppression goes unexplained.
- **Pinned Actions.** Every GitHub Action is pinned to a commit SHA.
- **Platforms.** Jobs on macOS and on arm64 Linux run the whole suite on every pull request,
  every push to `main`, weekly, on manual runs and in every release run (where both gate the
  build), and outside release runs a macOS job loads the launchd agent into a real launchd
  ([supported platforms](../../README.md#supported-platforms)).
- **Campaigns.** Each week `campaign.yml` runs a 10-minute hostile daemon soak with real review
  work and leak checks (RSS and open-fd growth), and an hour of fuzzing for every fuzz target.
- **Releases.** Each release up to v0.8.3 built four targets (three up to v0.5.0) and ran every one (the
  x86_64 macOS binary under Rosetta 2), held the Linux ones to the glibc floor, verified checksums, and attached SLSA provenance and the SBOM. From 0.8.4 a release is built
  on the owner's machine for Linux x86_64 only, twice from fresh exports that must give one SHA-256,
  with the same smoke, glibc check, SBOM and checksums, and `SHA256SUMS` signed by the owner; it
  has no build-provenance attestation ([local release](../RELEASE_AND_PUBLICATION.md#local-release-from-084)). GitHub release tags
  are immutable (a tag ruleset), and CodeQL analyses the code and workflows. The crates.io job
  (on hold) runs in the `release` environment, which deploys only from `v*` tags and, since
  2026-11-01, waits for the owner's approval.
- **Secret scanning.** GitHub secret scanning with push protection and provider patterns is on.
  Non-provider patterns and validity checks are not available for this user-owned repository, so
  gitleaks (the `secret scan` check and the versioned pre-push hook) is the compensating control
  ([SECURITY.md](../../SECURITY.md#supply-chain-and-releases)).
- **Local replica.** `scripts/ci_local.sh` (`just ci`) runs the CI gate jobs locally.

The engine is hardened against:
- terminal-escape injection;
- Trojan-Source bidi text;
- memory exhaustion;
- hidden-content patches;
- attacks on the daemon's transport and filesystem.

Fifteen fuzz targets cover the parser, review JSON, daemon protocol, agent annotations, council
reports, the MCP and `rpc` stdio servers, the soundness of the structural diff's
"formatting only" verdict, agent-findings verification, the code-evidence scanner, the
reader of a Rust hunk's tokens (every item on a line the change made, and the same signals
however a construct is spread over lines), where a Rust hunk stands and how it starts (the
same change written in other ways Git accepts loses no item), the Rust code views against
the compiler's own lexer (no byte of a token that is code is blank), the JSON a client gives
the agent hooks (`hook_input`: an input the hook cannot read is refused with the hooks' own
"did not run" message, UNJUDGED for a stop, never a pass), and the patch the hooks make of a
working tree (`judge_patch`, with real Git: the parser reads exactly the paths the edits
changed, or refuses the patch). See
[`SECURITY.md`](../../SECURITY.md) for the threat model.

# Review council and evidence gates

[`docs/REVIEW_COUNCIL.md`](../REVIEW_COUNCIL.md) holds the full design, threat model and
per-round history. This section is a map, not a copy.

- **The council.** `deep-diff-forge council prepare|review|gate|status` runs six reviewer
  slots over a sealed snapshot, split between Claude Code and Codex. The slots are correctness,
  security, architecture, falsifier, performance and subtractor. They review once
  independently, then in a challenge round. A report is admitted only if its citations resolve
  to real bytes in the snapshot. Missing evidence is `HOLD` with exit 7, never an empty pass.
  Nothing is approved automatically.
- **Eight evidence gates** run only inside a [rootless Podman sandbox](../REVIEW_COUNCIL.md#gate-sandbox):
  format, check, clippy, pedantic, tests, antipatterns, fuzz and mutation. The sandbox:
  - uses the operator-admitted image, by ID;
  - runs with `--network=none` (only dependency vendoring is networked);
  - mounts the root and the source read-only;
  - drops all capabilities and sets `no-new-privileges` and `--userns=keep-id`;
  - runs as `--user=<uid>:<gid>` from the operator's effective IDs (`geteuid`/`getegid`), so a
    gate never runs as root.

  Twelve planted-breach probes run in CI.
- **Diff-scoped mutation.** cargo-mutants runs `--in-diff` against a scope built from the
  sealed base and candidate blobs, with `--no-config`, so a candidate cannot shrink its own
  scope.
- **Exemption oracle.** [`exemption_oracle.py`](../../crates/deep-diff-forge-review/policy/exemption_oracle.py)
  refuses code hidden from mutation by anything other than `#[cfg(test)]`, and checks that
  `#[cfg(test)]` code leaves non-test MIR unchanged ([r18](../REVIEW_COUNCIL.md#cfg-test-only-r18)).
- **tuicr and the decision journal.** `council export-tuicr` files admitted findings into a
  local [tuicr](../REVIEW_COUNCIL.md#tuicr-export) session. `council import-tuicr` appends
  human dispositions to an append-only [`decisions.jsonl`](../REVIEW_COUNCIL.md#decision-journal).

See also: [`TESTING_GOLD_STANDARD`](../TESTING_GOLD_STANDARD.md) · [`REVIEW_COUNCIL`](../REVIEW_COUNCIL.md) · [`DEPLOYMENT_FRAMEWORK`](../DEPLOYMENT_FRAMEWORK.md).
