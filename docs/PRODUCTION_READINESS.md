# Production Readiness

**Kind:** dated assessment, append-only. A later assessment is a new section; earlier sections are
kept as written. Evidence labels: **measured** (run for this assessment), **inferred** (read from
code or docs, not run), **unmeasured** (no evidence either way).

Related: [deployment gap analysis](DEPLOYMENT_GAP_ANALYSIS.md) ·
[operations and daemon](OPERATIONS_AND_DAEMON.md) ·
[release and publication](RELEASE_AND_PUBLICATION.md) · [security policy](../SECURITY.md) ·
[evidence](../EVIDENCE.md) · [roadmap](ROADMAP.md).

## Assessment 2026-10-30 (`main` `a4bc06f`, workspace 0.3.0 + unreleased)

**Score: 83 / 100.** DDF is production-ready for the job it states: a local, single-user review
engine on Linux and macOS, driven through the CLI, the MCP server, the daemon and the Rust and
Python SDKs. It is not ready for:

- a daemon shared by many concurrent clients;
- Windows;
- syntax-aware review of languages other than Rust;
- the `chain`, `cluster`, `loom` and `<old> <new>` commands, which are still refused.

### Scope reviewed

- **Code:** 14 crates, about 52,000 lines of Rust under `crates/`.
- **Docs:** every `docs/*.md` file, plus `README`, `SECURITY`, `CHANGELOG`, `CONTRIBUTING` and
  `EVIDENCE`.
- **Workflows:** `ci.yml` and `release.yml`.
- **Repository settings:** the GitHub ruleset and security settings.
- **Records:** the Turso ledger (`ddf.db`: events, memories, findings, mutation runs) and the vault.

### Evidence gathered for this assessment (measured unless marked)

| Check | Result |
| --- | --- |
| `scripts/ci_local.sh` on `a4bc06f` (a copy of every CI step that runs locally) | 30 / 30 steps passed, `steps failed: 0` |
| `cargo test --workspace --locked` | 1,456 passed, 0 failed, 4 ignored |
| Ignored tests | 2 real-tuicr round trips, 1 real-Podman sandbox, 1 test that runs only as a spawned child process; each needs an opt-in environment |
| Python SDK tests (`sdk/python`) | 11 / 11 passed |
| GitHub CI on `main` `a4bc06f`, after the PR #44 merge | success |
| macOS job, `workflow_dispatch` on `876f323` (the PR #44 head) | success |
| Release binary, x86_64 Linux | 8.0 MB; `deep-diff-forge 0.3.0` |
| `--stdin-patch --json` on `git diff v0.2.0 v0.3.0` (58,140 lines) | exit 0; 0.11 s wall time; 26 MB peak RSS |
| Daemon lifecycle | `start`, `health` (`status: ok`, protocol 1) and `stop` all worked. A second `start` on the same socket exited 6 ("another daemon is running or starting on this socket"). After `stop`, the socket was removed and `<socket>.lock` stayed, as designed. |
| Production `unwrap`/`expect` outside `#[cfg(test)]` | 1 hit, a `todo!()` in a doc example in `patch/src/sealed.rs`; none in shipped code |
| `#[allow(...)]` attributes | 0; lints are silenced only by `#[expect(.., reason)]` |
| Ruleset 25868121 | active; 9 required checks; deletion, non-fast-forward and pull-request rules; no bypass actors |
| Secret scanning | enabled, 0 alerts. Push protection is disabled. |
| Dependabot | `dependabot.yml` covers both lock files, but no Dependabot PR has ever opened. Dependabot alerts and security updates are disabled at the repository level. |
| Code scanning | no analysis configured |
| Remote branches | 75 on each remote; 73 of the 75 on `origin` are already merged into `main` |
| Turso ledger | 79 events (66 checkpoints, 13 ingests). Over all recorded rounds: 5,479 mutants tested, 5,078 caught, 120 missed, 252 unviable. The diff-scoped gate runs since council r13 report 0 missed. |
| Weekly CI schedule | added 2026-10-27 (`c4f1f45`); it has not fired yet, and the first run is due Monday 2026-11-02 |

### Facets

Each facet has a weight; the weights sum to 100. The score is the share of that weight that the
evidence supports.

| # | Facet | Weight | Score |
| --- | --- | ---: | ---: |
| 1 | Correctness and patch-truth integrity | 20 | 18 |
| 2 | Security and trust boundary | 15 | 13 |
| 3 | Supply chain and release integrity | 15 | 13 |
| 4 | Reliability and operability | 15 | 11 |
| 5 | Platform and portability | 10 | 7 |
| 6 | Interfaces and integration contracts | 15 | 12 |
| 7 | Documentation, governance and evidence | 10 | 9 |
| | **Total** | **100** | **83** |

#### 1. Correctness and patch-truth integrity: 18 / 20

**Strengths**

- The parser refuses a patch it cannot represent exactly rather than repairing it, with Git as
  the oracle (`deep-diff-forge-patch`: 5,300 lines and 167 tests, plus `tests/git_oracle.rs`).
- 1,456 tests pass.
- Four required CI shards run diff-scoped mutation testing.
- 8 fuzz targets each ran 300 s on `8f957aa` after the last parser change
  ([evidence](../EVIDENCE.md#v030-release-readiness-2026-10-29)).
- Machine outputs are sealed. `--require-files` and `--require-hunks` stop a gate from reading
  an empty diff as green.
- Large input is handled well: 58,140 lines took 0.11 s (measured).

**Gaps**

- CI fuzzing runs 10 s per target, a smoke test. The longest recorded campaign is 300 s per
  target (measured, local). No continuous fuzzing service runs.
- Across council history, 120 mutants were missed. The diff gate catches new code but has not
  been re-run over whole crates.

#### 2. Security and trust boundary: 13 / 15

**Strengths**

- `unsafe_code = "forbid"` across the workspace.
- Effect doors: spawning a process, reading the environment, reading a clock and sleeping each
  need a reviewed `#[expect(clippy::disallowed_methods, reason)]`.
- Since PR #44, the daemon checks peer credentials on both ends (`SO_PEERCRED` / `getpeereid`),
  verified cross-uid in both directions on 2026-10-30.
- The start lock opens with `O_NOFOLLOW` and refuses a lock file that is not a regular file
  owned by the effective user.
- The MCP server is read-only.
- Council gates run in a rootless Podman sandbox with no network and no capabilities.
- Git subprocesses have time limits. The receipt privacy probe runs in CI.
- Secret scanning is on, with 0 alerts.

**Gaps**

- The peer-credential check and the start lock are on `main` only. Every published binary up
  to `v0.3.0` lacks them, so users of the released binaries keep the two 0.3.0 residual risks
  until the next tag.
- Push protection and non-provider secret patterns are disabled.
- No static analysis (code scanning) is configured.

#### 3. Supply chain and release integrity: 13 / 15

**Strengths**

- `cargo-deny` and strict `cargo audit` gate both lock files.
- The committed SPDX SBOM is regenerated and compared on every run.
- gitleaks is pinned and checksum-verified.
- Release assets carry sha256 checksums and SLSA build provenance (`attest-build-provenance`),
  and publishing happens only on a pushed `v*` tag.
- crates.io publishing needs `DDF_CRATES_IO=open`; a token alone publishes nothing.
- Ruleset 25868121 has no bypass.

**Gaps**

- Dependabot has never opened a PR, and its alerts are off. Advisories are caught only when CI
  runs: on pull requests and pushes to `main`, and weekly once the schedule starts firing.
- The last release dry run (`478db26`, 2026-10-29) predates PR #44, which added a `nix`
  dependency on non-Linux targets. Run a dry run before the next tag.
- crates.io is held by owner decision (not a defect).

#### 4. Reliability and operability: 11 / 15

**Strengths**

- The daemon has bounded concurrency, deadlines and an aggregate memory cap.
- Responses over 80 MiB are refused with `RESOURCE_EXHAUSTED`.
- A start lock prevents orphaned daemons (measured: 8 racing starts gave 1 daemon and 7 exits
  of 6).
- `daemon health` and `daemon status` report pid, protocol, sessions and retained bytes.
- A hostile soak smoke test runs in CI, and the stable exit-code registry is documented
  (`explain`).

**Gaps**

- One engine behind a `Mutex<Engine>` (`daemon/src/serve.rs:186`) serves every request, so
  requests run one at a time.
- The CI soak runs 10 s, so its long-run behaviour is unmeasured.
- Logs are plain stderr lines, with no structured logging or metrics endpoint.
- `deploy doctor` and `deploy receipt` do not exist.

#### 5. Platform and portability: 7 / 10

**Strengths**

- Linux is the primary target, tested on every PR on `ubuntu-26.04`.
- A Rust 1.88 MSRV check is a required check.
- Release builds cover x86_64 Linux and both x86_64 and aarch64 macOS.
- The macOS job passed on the PR #44 head.

**Gaps**

- macOS tests do not run on pull requests, only on manual dispatch and the weekly schedule,
  which has not fired yet. A macOS regression can merge: it happened twice during PR #44 and
  was caught only by a manual dispatch.
- There is no Windows support, by design: Unix sockets and `flock`.
- Syntax features (tree-sitter) cover Rust only.

#### 6. Interfaces and integration contracts: 12 / 15

**Strengths**

- One engine serves the CLI, daemon JSON-RPC, MCP and both SDKs, and parity tests prove it
  (`cli/tests/daemon_parity.rs`).
- Schemas are versioned, and `--self-test` plus the four contract commands run in CI.
- `sdk/tests/api.rs` pins public signatures.
- The Python SDK is tested against the built binary.

**Gaps**

- The `chain`, `cluster`, `loom` and `<old> <new>` commands are refused ("NOT YET
  IMPLEMENTED"), although their contracts are published.
- The bind API changed (now `BoundListener`), so the next release is a semver-breaking `0.4.0`.
- No harness tests the TUI cockpit's key sequences.

#### 7. Documentation, governance and evidence: 9 / 10

**Strengths**

- Every doc carries a dated status block.
- Evidence is labelled, and `EVIDENCE.md` records commands and run IDs.
- CONTRIBUTING, Code of Conduct, issue and PR templates, and a security policy with supported
  versions are all in place.
- The Turso ledger and the vault keep append-only history.
- This assessment found two stale lines and fixed them:
  - `docs/DEPLOYMENT_FRAMEWORK.md` omitted `v0.3.0` from its list of releases;
  - `SECURITY.md` said that PRs #21–#23 were in no release, but they shipped in `v0.3.0`.

**Gaps**

- 73 merged branches remain on each remote.
- About 30 design docs mix shipped behaviour and design intent, so a reader must rely on each
  status block to tell which is which.

### What would raise the score, in order of impact

1. **Tag `0.4.0`** after a release dry run on current `main`. This ships the peer-credential
   check and the start lock to binary users (+2, facets 2 and 3).
2. **Run the macOS job on pull requests** that touch `daemon/`, `patch/src/git*` or
   `cli/src/main.rs`, using a path filter (+2, facet 5).
3. **Enable Dependabot alerts and security updates, and secret push protection** (+2, facets 2
   and 3).
4. **Serve the daemon with an engine per connection, or a pool**, with the existing bounds
   kept, and run a soak of 10 minutes or more as a scheduled job (+3, facet 4).
5. **Add `deploy doctor`** to check the socket, lock, permissions, Git and toolchain (+1,
   facet 4).
6. **Delete the 73 merged remote branches** on both remotes (+1, facet 7).
7. **Schedule a longer fuzz campaign** (for example 1 h per target, weekly) and record it in
   `EVIDENCE.md` (+1, facet 1).

## Assessment 2026-10-31 (`v0.5.0`, Claude's self-assessment)

**Score: 91 / 100** (was 83 on 2026-10-30). The declared scope is unchanged: a local,
single-user review engine on Linux and macOS, driven through the CLI, MCP, the daemon and the
Rust and Python SDKs. It is still not ready for a daemon shared by many users, for Windows, for
syntax-aware review outside Rust, or for the refused `chain`, `cluster`, `loom` and
`<old> <new>` commands. An independent rescore by Codex follows this section.

### What changed since 2026-10-30

All seven actions the 2026-10-30 assessment listed are done. Six shipped in v0.4.0
([EVIDENCE](../EVIDENCE.md#v040-release-readiness-2026-10-30)): the tag, macOS on every pull
request, Dependabot alerts and push protection, the parallel daemon with a weekly 600 s soak,
`deploy doctor`, and the deletion of merged branches. The seventh, a weekly hour of fuzzing per
target, runs in `campaign.yml`. v0.5.0 adds the work of
[the plan](PLAN_AGENT_EVIDENCE_AND_OBSERVABILITY.md): code evidence, verified agent findings,
daemon metrics and JSON logs, `bench` and `eval`, and the DDF-EVID-001 fix
([EVIDENCE](../EVIDENCE.md#v050-release-readiness-2026-10-31)).

### Evidence gathered for this assessment (measured unless marked)

| Check | Result |
| --- | --- |
| `scripts/ci_local.sh` on the release head `5b75570` | `steps failed: 0` |
| `cargo test --workspace --locked` | 1,592 passed, 0 failed, 4 ignored (the same 4 opt-in tests as before) |
| Python SDK tests against the release binary | 12 / 12 passed |
| PR #56 CI and release dry run | 11 / 11 checks green at `5b75570`. Dry run 41137004116 passed: every quality gate including macOS, and all three builds. A first dry run (41129704309, at `e1369a7`) failed on a timing-dependent macOS test, which is now fixed (DDF-TEST-002). |
| Release binary, x86_64 Linux | 8.4 MB; `deep-diff-forge 0.5.0` |
| `--stdin-patch --json` on `git diff v0.2.0` to the release head (71,107 lines) | exit 0; 0.15 s wall time; 36 MB peak RSS |
| Daemon lifecycle (owner-private socket directory) | `health` reports protocol 2 and version 0.5.0; a second `start` exits 6; `daemon metrics` returns admission, connection, engine and per-method counters; `deploy doctor` passes all four checks; `stop` removes the socket and keeps the lock. A socket directory that is group-accessible is refused at start, as designed. |
| `#[allow(...)]` attributes | 0 in code (5 matches, all inside strings and tests of the `check_suppressed` signal); 70 `#[expect(.., reason)]` |
| Rust under `crates/` | about 61,000 lines (52,000 on 2026-10-30) |
| Fuzz targets | 10 (8 on 2026-10-30); every one runs 10 s on each pull request and an hour weekly |
| Weekly campaign | one dispatched run (40989103096, before #51): soak, bench and fuzz all passed. The schedule's first run is due 2026-11-02 (unmeasured). |
| Ruleset 25868121 | active; 11 required checks (`bench (base vs head)` added 2026-10-31); no bypass actors |
| Security settings (`gh api`) | secret scanning on, push protection on, Dependabot security updates on, 0 open Dependabot alerts, 0 open secret alerts. Non-provider patterns and validity checks are off. No code scanning analysis exists. |
| Dependabot version PRs | none has ever opened (`dependabot.yml` covers both lock files and Actions, 7-day cooldown) |
| Branches | 3 on each remote: `main` and two unmerged branches not written by Claude (`release/stage-e`, 2026-10-26; `docs/vault-highways-s1009380`, 2026-07-14), kept for their owners |
| Turso ledger | 99 events (86 checkpoints, 13 ingests). Mutation totals over 54 recorded runs are unchanged at 5,479 tested and 120 missed: the diff-scoped CI shards do not write to the ledger. |

### Facets

| # | Facet | Weight | 2026-10-30 | 2026-10-31 |
| --- | --- | ---: | ---: | ---: |
| 1 | Correctness and patch-truth integrity | 20 | 18 | 19 |
| 2 | Security and trust boundary | 15 | 13 | 14 |
| 3 | Supply chain and release integrity | 15 | 13 | 14 |
| 4 | Reliability and operability | 15 | 11 | 14 |
| 5 | Platform and portability | 10 | 7 | 8 |
| 6 | Interfaces and integration contracts | 15 | 12 | 13 |
| 7 | Documentation, governance and evidence | 10 | 9 | 9 |
| | **Total** | **100** | **83** | **91** |

**1. Correctness, 19 / 20.** Gained: ten fuzz targets, with an hour each weekly; `eval` holds
floors for ranking, evidence and findings verdicts over an 11-case labelled corpus; the
`bench` comparison is a required check, so a slowdown over 25% and 0.2 ms blocks a merge.
Gaps: no whole-crate mutation run since council history (120 missed, recorded); the weekly
fuzz schedule has not fired on its own yet; the lexical scanner is a reviewer's lens with
documented misses (triple-quoted f-strings, multi-line interpolation).

**2. Security, 14 / 15.** Gained: the peer-credential check and start lock are in released
binaries; push protection is on; DDF-EVID-001 closed (quoting no longer hides code); agent
text drops bidi and invisible characters. Gaps: no static analysis (code scanning); secret
scanning's non-provider patterns are off.

**3. Supply chain, 14 / 15.** Gained: Dependabot alerts and security updates are on; the
release dry run built the exact tree that is tagged (`v0.5.0` on `36bd5f3`, tree `d521c19`). Gaps: no Dependabot version PR has ever
opened, so whether version updates work is unmeasured; crates.io is held by owner decision
(not a defect).

**4. Reliability, 14 / 15.** Gained: requests run in parallel under a 128 MiB in-flight
budget; a weekly 600 s leak-checked soak; `deploy doctor`; lock-free metrics with latency
percentiles; JSON logs. Gaps: metrics are pull-only (no export to a monitoring system); the
scheduled soak has one dispatched run, none scheduled yet.

**5. Platform, 8 / 10.** Gained: macOS runs the whole suite on every pull request as a
required check. Gaps: no Windows (by design: Unix sockets and `flock`); tree-sitter syntax
covers Rust only.

**6. Interfaces, 13 / 15.** Gained: `findings.verify` gives one verdict through the CLI,
daemon, `rpc`, MCP and the Python SDK; daemon protocol 2 keeps the method lists of protocols
0 and 1 for old clients. Gaps: `chain`, `cluster`, `loom` and `<old> <new>` are still refused
although their contracts are published; the TUI's key-to-action mapping has 22 unit tests,
but no harness drives whole key sequences against a rendered cockpit (inferred).

**7. Documentation, 9 / 10.** Gained: the merged branches are gone; every release doc names
v0.5.0; EVIDENCE records each PR's CI and the release. Gap: about 30 design docs still mix
shipped behaviour and design intent, held apart only by their status blocks.

### What would raise the score, in order of impact

1. **Add code scanning** (CodeQL for Rust and the Actions workflows) and turn on non-provider
   secret patterns (+1, facet 2).
2. **Let the weekly campaign fire** on 2026-11-02 and record its soak, fuzz and bench results
   in EVIDENCE (+1, facets 1 and 4).
3. **Run a whole-crate mutation pass** of `graph` and `agent`, the crates this release grew,
   and record it in the ledger (+1, facet 1).
4. **Prove Dependabot version updates** (a first version PR, or a recorded reason none is
   due) (+1, facet 3).
5. **A TUI key-sequence harness** over a rendered `TestBackend` cockpit (+1, facet 6).

## Independent rescore 2026-10-31 (Codex, `v0.5.0`)

**Reviewer:** Codex (`codex exec`, model `gpt-daybreak-blue-latest`, reasoning effort high,
read-only sandbox, hooks disabled), on a worktree of `main` at `36bd5f3` (the `v0.5.0` commit)
with the self-assessment above present but uncommitted. It was asked to score independently, on
the same facets and weights, and not to anchor on either earlier score. Its tool had no `cargo`
and no built binary, so it did not run tests or the CLI. Its report follows unedited, with
headings moved down a level.

#### Score: 81 / 100

| Facet | Weight | My score | Self-assessment |
|---|---:|---:|---:|
| Correctness and patch-truth integrity | 20 | 18 | 19 |
| Security and trust boundary | 15 | 13 | 14 |
| Supply chain and release integrity | 15 | 12 | 14 |
| Reliability and operability | 15 | 12 | 14 |
| Platform and portability | 10 | 7 | 8 |
| Interfaces and integration contracts | 15 | 12 | 13 |
| Documentation, governance and evidence | 10 | 7 | 9 |
| **Total** | **100** | **81** | **91** |

`git rev-parse HEAD` measured `36bd5f3`; `v0.5.0` points there and its tree matches release head `5b75570`. `cargo metadata --offline --locked` could not run because `cargo` is absent, and no built binary exists under `target/`; tests and CLI behavior are therefore unmeasured by this review.

##### 1. Correctness and patch-truth integrity — 18 / 20

- **[Measured/read]** Patch truth has a real Git oracle: reconstructed patches are applied to exact base bytes, including CRLF, missing newline, quoted Unicode paths and binary data (`crates/deep-diff-forge-patch/tests/git_oracle.rs:25-75`). Seals bind parsed files to exact input bytes (`crates/deep-diff-forge-patch/src/sealed.rs:55-64`).
- **[Measured/read]** CI runs workspace tests, four differential-mutation shards and ten-second fuzzing of every target (`.github/workflows/ci.yml:120-155`, `252-279`, `179-189`). The weekly workflow specifies one hour per fuzz target (`.github/workflows/campaign.yml:99-124`).
- **[Measured/read]** The evaluation gate is only 11 cases. It excludes one declared miss from recall rather than failing on it (`fixtures/eval/MANIFEST.json:125-138`; `crates/deep-diff-forge-cli/tests/eval.rs:15-21`).

**Gaps**

- **[Unmeasured]** No scheduled hour-long campaign has run; the recorded dispatch used only 120 seconds per target (`EVIDENCE.md:715`). The 1,592-pass claim was not rerun here. Historical mutation evidence still contains 120 missed mutants, and the tiny eval corpus is not credible evidence for near-perfect review quality.

##### 2. Security and trust boundary — 13 / 15

- **[Measured/read]** First-party unsafe code is forbidden and lint suppressions require reasons (`Cargo.toml:34-52`). Git acquisition disables hooks, transports, external diff, textconv and repository-local content filters (`crates/deep-diff-forge-core/src/git.rs:11-47`; `crates/deep-diff-forge-patch/src/git_acquire.rs:191-286`).
- **[Inferred]** Daemon isolation is solid for the stated boundary: owner-private directory/socket modes, effective-UID peer checks and fail-closed unsupported platforms (`crates/deep-diff-forge-daemon/src/security.rs:10-13`, `90-117`, `180-239`).
- **[Measured/read]** The security policy honestly trusts the same OS account and rejects multi-tenant/network use (`SECURITY.md:75-84`). That makes “local, single-user” honest; it is not a sandbox against compromised same-UID processes.

**Gaps**

- **[Unmeasured]** Push protection, alert counts and GitHub security settings exist only as self-reported `gh api` results. No CodeQL/code-scanning workflow exists. Global/system Git filters remain trusted (`git_acquire.rs:225-230`; kept by decision on 2026-10-31, see SECURITY.md, and repository `config.worktree` filters are now turned off too), and third-party native/unsafe code is outside the first-party guarantee.

##### 3. Supply chain and release integrity — 12 / 15

- **[Measured/read]** Actions are SHA-pinned with read-only default permissions; both lockfiles receive cargo-deny/audit checks, and the committed SPDX SBOM is regenerated and compared (`.github/workflows/ci.yml:18-20`, `319-360`).
- **[Measured/read]** Release builds depend on quality and supply-chain jobs, smoke runnable targets, verify checksums, and attest archives and the SBOM (`.github/workflows/release.yml:70-173`). `git rev-parse 5b75570^{tree} 36bd5f3^{tree}` measured the same tree, `d521c19…`.
- **[Measured]** `git cat-file -p v0.5.0` shows an annotated but unsigned tag. The release recipe likewise uses `git tag -a`, not a signed tag (`justfile:126-148`).

**Gaps**

- **[Inferred/unmeasured]** Repository code contains no protected release environment or signed-tag enforcement; external tag rules are unmeasured. Any pushed `v*` tag matching the workspace version triggers publication. crates.io still uses a long-lived token rather than trusted publishing, and an opened gate with a missing token exits successfully without publishing (`release.yml:175-225`). Actual v0.5.0 artifact and attestation verification is absent from `EVIDENCE.md`.

##### 4. Reliability and operability — 12 / 15

- **[Inferred]** Requests run concurrently under FIFO admission, a 128 MiB in-flight weight and a 30-second wait bound (`crates/deep-diff-forge-daemon/src/admission.rs:18-24`, `119-174`). Request/response caps, connection limits, deadlines and panic containment are implemented (`serve.rs:169-249`, `389-420`).
- **[Measured/read]** The hostile soak exercises cap boundaries, connection floods, concurrent clients, real review work, RSS and FD growth (`scripts/security/daemon_soak.py:116-237`, `240-271`).
- **[Measured/read]** Metrics are process-local and pull-only; JSON logs have only warning-level refusal/error events. No service unit ships and shutdown has no drain phase (`docs/OPERATIONS_AND_DAEMON.md:22-25`, `79-83`, `213-237`, `297-327`).

**Gaps**

- **[Unmeasured]** The scheduled 600-second soak has never fired. The only long evidence is local/manual. There is no durable telemetry, export, graceful drain, cancellation, packaged supervisor integration or recovery test across process restart.

##### 5. Platform and portability — 7 / 10

- **[Measured/read]** CI tests Ubuntu with Rust 1.88 and runs the whole workspace on `macos-latest` for every trigger (`.github/workflows/ci.yml:52-85`).
- **[Measured/read]** Releases build x86_64 GNU/Linux plus x86_64 and aarch64 macOS; only Linux x86_64 and arm64 macOS are executed (`.github/workflows/release.yml:82-120`).
- **[Measured/read]** The daemon intentionally fails compilation off Unix (`crates/deep-diff-forge-daemon/src/lib.rs:20-29`).

**Gaps**

- **[Inferred]** “Linux and macOS” is too broad. The binary matrix is Ubuntu-era glibc on x86_64 Linux; there is no Linux arm64, musl or distro compatibility matrix. The x86_64 macOS artifact is cross-built but never smoked. Windows exclusion is honest.

##### 6. Interfaces and integration contracts — 12 / 15

- **[Measured/read]** CLI/daemon parity tests cover success, refusal and parse-error behavior across several patch forms (`crates/deep-diff-forge-cli/tests/daemon_parity.rs:66-124`). Protocol method metadata and version-filtered discovery are centralized (`crates/deep-diff-forge-daemon/src/handler.rs:395-467`, `595-625`).
- **[Measured/read]** Contrary to the self-assessment, a TUI harness already drives real key sequences through mapping, state and `TestBackend` rendering (`crates/deep-diff-forge-tui/src/ui.rs:367-375`, `511-613`). Only the live terminal loop is untested (`run.rs:43-48`).
- **[Measured]** `sdk/python` contains only a module and tests—no `pyproject.toml` or installer. Release archives contain only the binary, README and licences (`docs/RELEASE_AND_PUBLICATION.md:94-107`); the Rust SDK is not on crates.io.

**Gaps**

- **[Measured/read]** `chain`, `cluster`, `loom` and direct `<old> <new>` remain refused despite published contracts (`crates/deep-diff-forge-cli/src/main.rs:1962-1966`). Calling the SDKs production-ready overstates their distribution readiness.

##### 7. Documentation, governance and evidence — 7 / 10

- **[Measured/read]** Security reporting, supported versions, operational limits, contribution templates and extensive evidence records exist (`SECURITY.md:23-56`; `.github/ISSUE_TEMPLATE/`; `EVIDENCE.md`).
- **[Measured]** Documentation contradicts itself: the changelog calls the bench check both informational and required (`CHANGELOG.md:19-20`, `82-85`); the release plan says both ten and eleven required checks (`docs/RELEASE_AND_PUBLICATION.md:79-81`, `131-133`); the roadmap still calls bench informational and stops the release list at v0.4.0 (`docs/ROADMAP.md:21`).
- **[Measured]** `EVIDENCE.md` ends with pre-tag v0.5 readiness, not the actual tag workflow, downloaded checksums or attestation verification (`EVIDENCE.md:776-794`). `git status --short` also shows the self-assessment itself is uncommitted.

**Gaps**

- **[Measured/inferred]** The repository has over 10,000 lines of design documentation across `docs/`; 24 documents contain planned/future/not-implemented material. Status blocks help, but current contradictions show they are not sufficient. `git branch -r` also measured four real branches per remote, not the self-assessment’s claimed three.

#### Disagreements with the self-assessment

- **The 91/100 score overvalues configured checks as proven operation.** The hour-long fuzz campaign and scheduled soak have never fired. Run `40989103096` used 120-second fuzzing and `EVIDENCE.md:715` does not report a bench job.
- **Supply chain is not 14/15.** `v0.5.0` is unsigned; release authorization relies on an external tag push, with no repository-enforced release environment or signature (`justfile:146-148`; `.github/workflows/release.yml:6-12`). The crates.io job can succeed while publishing nothing when its token is absent (`release.yml:191-199`).
- **“EVIDENCE records … the release” is false for v0.5.0.** It records PR readiness and the required-check change, then ends (`EVIDENCE.md:776-794`). There is no v0.5 tag-run ID or post-download attestation verification comparable to v0.4.0’s record at `EVIDENCE.md:727-739`.
- **The branch count is wrong.** `git branch -r` measured four branches on each remote: `main`, `release/stage-e`, `docs/vault-highways-s1009380`, and `claude/release-0.5.0`.
- **Platform scope is only partly honest.** Single-user/local and no-Windows are explicit and accurate. Generic “Linux and macOS” hides that only x86_64 GNU/Linux and arm64 macOS are executed; x86_64 macOS is merely cross-built (`release.yml:82-120`).
- **The claimed TUI gap is stale.** Real key sequences already drive the mapped action and rendered cockpit (`ui.rs:367-375`, `511-613`). The remaining gap is the live crossterm event loop, not a missing `TestBackend` key-sequence harness.
- **Documentation is not 9/10.** Current release, check-count and bench-status contradictions are directly visible in `CHANGELOG.md`, `ROADMAP.md` and `RELEASE_AND_PUBLICATION.md`.

#### Top 5 actions that would raise the score

1. Protect release authority: signed tags, protected tag rules, an approval-gated release environment, crates.io trusted publishing, and fail if publication is enabled without credentials.
2. Let the scheduled campaign run at its declared durations; then run whole-crate mutation on `graph` and `agent` and record exact artifacts/results in `EVIDENCE.md`.
3. Add CodeQL/code scanning and independently verify repository security settings, non-provider secret detection and push protection.
4. Package and publish the Python and Rust SDKs through supported channels—or narrow the production-ready claim to the binary interfaces.
5. Define and test an exact platform matrix: supported Linux architecture/glibc floor, Linux arm64 if claimed, and execution of the x86_64 macOS artifact; reconcile the contradictory release documentation.

## Reconciliation 2026-10-31 (Claude, after the independent rescore)

Codex scored 81, the self-assessment 91. Each disagreement was checked against the repository
and GitHub (measured). Codex's tool could not run `cargo`, the binary or `gh`, so some of its
"unmeasured" points were measured here, and one hour-long campaign ran after its review.

| Point | Verdict | Action |
| --- | --- | --- |
| The CHANGELOG called `bench` informational and required; `RELEASE_AND_PUBLICATION` said ten and eleven checks; `ROADMAP` called `bench` informational | **Accepted.** All three were stale lines from the release PR. | Fixed in this change. |
| EVIDENCE did not record the v0.5.0 tag run or verify its assets | **Accepted**: the release run was still in progress when Codex reviewed. | Recorded: run 41143246901, `sha256sum -c` OK, `--version` 0.5.0, one SLSA v1 provenance from `release.yml@refs/tags/v0.5.0` ([EVIDENCE](../EVIDENCE.md#v050-dry-runs-tag-and-published-release-2026-10-31)). |
| Four branches per remote, not three | **Accepted** at review time: `claude/release-0.5.0` was not yet deleted. | Deleted from both remotes; 3 remain (`main` and the two unmerged branches). |
| The scheduled hour-long fuzz and 600 s soak never ran | **Accepted** at review time. | Campaign run 41143303217 on the `v0.5.0` commit: 10 targets × 3,601 s with 0 crashes, soak PASS, full bench passed ([EVIDENCE](../EVIDENCE.md#v050-weekly-campaign-at-its-declared-durations-2026-10-31)). The cron schedule itself is still unexercised (first run 2026-11-02). |
| No TUI key-sequence harness | **Codex is right; the self-assessment was wrong.** `tui/src/ui.rs` drives real keys through `map_key` and renders through `TestBackend` (from line 367). Only the live crossterm loop is untested. | The gap is withdrawn. |
| `v0.5.0` is unsigned; no protected release environment or tag rule; any `v*` tag that matches the version publishes | **Accepted** (`git cat-file -p v0.5.0` has no signature; `release.yml` has no `environment:`). v0.1.0 to v0.4.0 are unsigned too. | Open: signed tags, a tag ruleset and a release environment with approval. |
| An opened crates.io gate without a token exits successfully | **Accepted as described**; the gate is closed (`DDF_CRATES_IO` unset), so nothing publishes today. | Open, for when crates.io opens. |
| Push protection and alert counts are self-reported | **Disputed.** They were read from the GitHub API on 2026-10-31 (`security_and_analysis`: push protection enabled; 0 open Dependabot and secret alerts), which is a measurement, not a claim. Codex could not run `gh`. | None. |
| Global and system Git filters stay trusted | **Accepted** as a residual risk: repository-local filters are refused, while the user's own Git configuration is inside the same-account trust boundary (`SECURITY.md`). | Documented here. |
| "Linux and macOS" is broad: only x86_64 GNU/Linux and arm64 macOS run in CI; x86_64 macOS is cross-built and never run; no Linux arm64 or musl | **Accepted.** | Open: run the x86_64 macOS archive in the release smoke, and state the Linux target (x86_64, glibc) where the scope is declared. |
| The Python SDK has no package; the Rust SDK is not on crates.io | **Accepted.** The Python SDK is a single module used from a checkout, and crates.io is held by owner decision. | Open: package it, or name SDK use as from-source in the scope. |
| The eval corpus (11 cases) is small | **Accepted.** It is a floor against regressions, not a measure of review quality. | None now. |

### Reconciled score: 86 / 100

| # | Facet | Weight | Self | Codex | Reconciled |
| --- | --- | ---: | ---: | ---: | ---: |
| 1 | Correctness and patch-truth integrity | 20 | 19 | 18 | 19 |
| 2 | Security and trust boundary | 15 | 14 | 13 | 13 |
| 3 | Supply chain and release integrity | 15 | 14 | 12 | 13 |
| 4 | Reliability and operability | 15 | 14 | 12 | 13 |
| 5 | Platform and portability | 10 | 8 | 7 | 7 |
| 6 | Interfaces and integration contracts | 15 | 13 | 12 | 13 |
| 7 | Documentation, governance and evidence | 10 | 9 | 7 | 8 |
| | **Total** | **100** | **91** | **81** | **86** |

Facet 1 keeps 19 because the campaign Codex found missing has now run at full length. Facets
2 to 5 and 7 take Codex's accepted points. Facet 6 keeps 13: the TUI gap is withdrawn, and the
SDK distribution gap replaces it.

### What would raise the score, in order of impact

1. **Release authority** (+2, facet 3): signed tags, a tag ruleset for `v*`, a `release`
   environment with required approval; and a crates.io job that fails, rather than passing
   silently, when the gate is open but the token is missing.
2. **Code scanning** (CodeQL for Rust and the workflows) and non-provider secret patterns (+1,
   facet 2).
3. **The platform matrix stated and run** (+1, facet 5): run the x86_64 macOS archive in the
   release smoke, and declare x86_64 GNU/Linux (glibc) as the Linux target.
4. **Operability** (+1, facet 4): a graceful drain on `daemon stop`, and a shipped user
   service unit (systemd, launchd).
5. **A whole-crate mutation pass** of `graph` and `agent`, recorded in the ledger, and a
   Python SDK package (+1, facets 1 and 6).
