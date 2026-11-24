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
  of `release.yml` runs in the `release` environment, which deploys only from `v*` tags and,
  since 2026-11-01, waits for the owner's approval; it is not used: 0.9.0's crates were
  published from the owner's machine on 2026-11-23, and from 0.10.0 publishing is a step of the
  release.
- **Secret scanning.** GitHub secret scanning with push protection and provider patterns is on.
  Non-provider patterns and validity checks are not available for this user-owned repository, so
  gitleaks (the `secret scan` check and the versioned pre-push hook) is the compensating control
  ([SECURITY.md](../../SECURITY.md#supply-chain-and-releases)).
- **Local replica.** `scripts/ci_local.sh` (`just ci`) runs the CI gate jobs locally: 35 steps
  from 0.10.0 (32 before). It prints the commit it gates and writes nothing into the checkout: the
  SBOM step generates into the gate's own scratch directory and compares, so `sbom.spdx.json` is
  never rewritten, also when the step is stopped. Three steps run only there; they are listed as
  `LOCAL_ONLY` in `crates/deep-diff-forge-cli/tests/ci_replica.rs`, since GitHub's workflow is a
  mirror and is not changed for them (every step is a `ci.yml` step or a member of the list, and
  every member is a step): the **release-script self-test** (`scripts/test_release_local.sh`: a
  release build and its verify in a throwaway clone, with planted failures; it runs
  `scripts/check_glibc_floor.sh` on the release binary, and the test step runs it on the test
  binary), the **comparison corpus manifest** (`sha256sum -c` of the corpus's `MANIFEST.sha256`
  where the corpus is on the machine, `DDF_COMPARE_CORPUS`; elsewhere the step passes and says
  `skipped: no corpus here`) and the **contract matrix**. `scripts/release_local.sh` writes the
  checks run on the artefact and the gate's result (from the log `DDF_GATE_LOG` names) into the
  release receipt, and beside it `RELEASE-MANIFEST.json` (what the binary reads and writes, read
  from the binary itself; [the release document](../RELEASE_AND_PUBLICATION.md)), which the
  self-test holds (a manifest without a member, or signed in the namespace of `SHA256SUMS`,
  fails verify).
- **Contract matrix.** `scripts/contract_matrix.sh [--require-real] <binary>` plays the contract
  between the stop hook and forge-warden, and between this build and the published ones, one
  scratch repository and home per row, run the client's way (no configuration of the user or
  the system). The stand-in judge is `scripts/contract-matrix/judge.py`, answering with
  forge-warden's own v2 and v3 documents. The real rows run once for each forge-warden given:
  `FW_BIN` (the gate's judge) and each path of `FW_BIN_OLDER` (colon-separated); a row's id
  carries the judge's name, `fw-recorded[005a1959]`. Every binary is accepted by its SHA-256 in
  `scripts/contract-matrix/binaries.tsv`, never by its version: the forge-wardens `005a1959`
  (hub main, Train B15, the gate's judge from 0.11.0), `db4c580e` (Train B14c) and `0.2.1-build`
  (a build of the release commit of forge-warden 0.2.1, not the published file), and the
  published deep-diff-forge 0.9.0 and 0.10.0 (`DDF_RANKER_090`, `DDF_RANKER_0100`). Without
  `FW_BIN` the real rows are skipped by name; with `--require-real` a missing binary is red, and
  so is every forge-warden `binaries.tsv` records that was not given (row `fw-given[<name>]`).
  The gate runs the stand-in rows always and passes `--require-real` when `FW_BIN` is set.
  `fw-unknown-labels-hold` builds a stand-in ranker with `rustc`. The rows, and what each needs:

  | Row | Needs | What it holds |
  | --- | --- | --- |
  | `v2-pass`, `v2-hold`, `v2-fail`, `v3-pass`, `v3-hold`, `v3-fail` | stand-in | each verdict is read as what it says; a FAIL blocks |
  | `unread-fail-holds-once-then-stands` | stand-in | a FAIL whose document cannot be read (exit 7) holds once, then stands |
  | `exit-4-is-unjudged`, `a-hanging-judge-is-unjudged` | stand-in | a refusal and a hang are UNJUDGED, never a pass |
  | `require-verified-claims-holds-never-passes` | stand-in | the uncommitted change holds for claims it cannot verify; `agent doctor --gate` names `claims_verified` weak |
  | `the-judged-patch-is-make-patch-bytes` | stand-in | the judge is given the bytes of forge-warden's recorded `make_patch.sh` |
  | `verify-branch-fail`, `verify-branch-refuse`, `verify-branch-pass` | stand-in | `agent verify-branch` refuses a commit that failed or was not judged, and verifies one that passed |
  | `verify-branch-every-outcome` | stand-in | one branch with a pass, a FAIL that stands, a HOLD, an UNJUDGED and a commit no stop saw: four of five refused (exit 7) |
  | `control-a-fail-let-through-is-red` | stand-in | the known-bad control: a FAIL expected to pass the stop must come out red |
  | `a-first-stop-is-never-silent` | stand-in | a stop that is its session's first hook says the session's start was not recorded, and runs no judge |
  | `stateless-stop-damaged`, `-older`, `-newer`, `-lists_damaged`, `-lock_busy`, `-no_directory` | stand-in | each way a state cannot be used whose stop judges the tree without it: the failing tree blocks and the class is named |
  | `stray-bytes-in-a-source-file-are-read-in-a-git-mode` | this build | `--git-range` reads a source file Git calls binary for a stray NUL beside a byte that is not UTF-8: `check_suppressed` and `diff_suppressed` (the published 0.10.0 gave `binary_change` alone) |
  | `diagnostics-paged-and-put-together-rpc`, `-mcp` | this build | `diff.diagnostics` (at 4,096 bytes) and MCP `diagnose_patch` (at 12,364) page the document and the pages make it again, with its SHA-256 on every page |
  | `state-written-by-<0.9.0, 0.10.0>-is-carried-over-by-this-build` | the published binary | its v10 session is written again as v13 by this build's stop; the failure blocks once more, then stands; the commit judged is not judged again |
  | `state-written-by-this-build-is-refused-and-kept-by-<0.9.0, 0.10.0>` | the published binary | a v13 session: the published stop is UNJUDGED ("cannot be read as a state") and its files are kept byte for byte |
  | `states-v11-to-v13-are-refused-and-kept-by-<0.9.0, 0.10.0>` | the published binary | the same for the v11, v12 and v13 states of `tests/fixtures/hook-state/` |
  | `fw-recorded` | each forge-warden | the binary is the one recorded, with the verdict schema recorded |
  | `fw-judges-the-stop` | each forge-warden | the stop reads the real verdict (it holds under the starter policy) |
  | `fw-a-commit-passes-in-bound-mode` | each forge-warden | a one-parent commit passes; with `005a1959` and `db4c580e` (whose usage names `--repo`) it is judged in bound mode under a policy that requires verified claims; with `0.2.1-build` it is never bound, and passes because its starter policy (v0) requires none |
  | `fw-unknown-labels-hold` | each forge-warden, `rustc` | with `db4c580e` and `0.2.1-build` a ranking that carries one of the six labels they do not know (`stub_added`) holds as unknown; with `005a1959` it holds by a rule of the policy (`no-placeholders`); with every judge a made-up label holds as unknown |
  | `fw-reads-the-rank` | each forge-warden | the judge reads this build's rank document (with `scorer_sha256`) without a rank refusal |
  | `fw-empty-patch-this-ranker`, `-ranker-0.9.0`, `-ranker-0.10.0` | each forge-warden, the published binaries | an empty patch: `005a1959` passes `--allow-empty` to its ranker and holds (`empty_ranking`) with this build and 0.10.0, and gets "the ranker exited 2" from 0.9.0; `db4c580e` and `0.2.1-build` get "the ranker exited 4" from this build and 0.10.0, and hold with 0.9.0 |
  | `fw-an-older-ranker-is-admitted-0.9.0`, `-0.10.0` | each forge-warden, the published binaries | a policy that admits only a published ranker: the stop finds it in `--ranker-dir`, gives it to the judge as `--ranker`, and the commit passes |
  | `fw-the-frontier-judges-a-commit-left-behind` | each forge-warden | a commit left by a session that never stopped blocks the next session's stop with the real judge's FAIL, marked as not this session's |
  | `fw-agent-check-equals-the-stop` | each forge-warden | `agent check --session` after a stop says it equals the stop in its units and inputs, and each unit has the stop's outcome |
  | `fw-given[<name>]` | `--require-real` | every forge-warden `binaries.tsv` records was given |

  Rows that would only repeat a test of `tests/agent_hook.rs` on its stand-in are left to the
  test (this build on every state fixture: `every_schema_since_the_last_release_is_read`); the
  first-stop and stateless rows repeat theirs the client's way, side by side.

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
