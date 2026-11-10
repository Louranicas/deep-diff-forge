# Production Readiness

**Kind:** dated assessment, append-only. A later assessment is a new section; earlier sections are
kept as written. Evidence labels: **measured** (run for this assessment), **inferred** (read from
code or docs, not run), **unmeasured** (no evidence either way).

Related: [deployment gap analysis](DEPLOYMENT_GAP_ANALYSIS.md) ·
[operations and daemon](OPERATIONS_AND_DAEMON.md) ·
[release and publication](RELEASE_AND_PUBLICATION.md) · [security policy](../SECURITY.md) ·
[evidence](../EVIDENCE.md) · [roadmap](ROADMAP.md) · [deployment framework](DEPLOYMENT_FRAMEWORK.md).

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

- **[Unmeasured]** Push protection, alert counts and GitHub security settings exist only as self-reported `gh api` results. No CodeQL/code-scanning workflow exists. Global/system Git filters remain trusted (`PROTECTED_SCOPES` and the module's "Configuration trust" section in `git_acquire.rs`; kept by decision on 2026-10-31, see SECURITY.md, and repository `config.worktree` filters are now turned off too), and third-party native/unsafe code is outside the first-party guarantee.

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
| No TUI key-sequence harness | **Codex is right; the self-assessment was wrong.** `tui/src/ui.rs` drives real keys through `map_key` and renders through `TestBackend` (from line 367). Only the live crossterm loop is untested. | The gap is withdrawn. Later on 2026-10-31 the live loop was tested too: scripted input through the production loop in `tui/src/run.rs`, and bare `review` in a pseudo-terminal (`cli/tests/review_tty.rs`). |
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

## Addendum 2026-11-01 (whole-crate mutation)

Not a rescore. The facet-1 gap "no whole-crate mutation run since council history (120 missed,
recorded)" is narrowed, **measured** in [EVIDENCE](../EVIDENCE.md#whole-crate-mutation-of-core-patch-daemon-and-syntax-2026-11-01):
`graph` and `agent` were run on 2026-10-31, and `core`, `patch`, `daemon` and `syntax` now.
After 51 new tests, the misses left in `core`, `patch` and `syntax` are equivalent mutants or
code compiled only off Unix. `daemon`'s after state is workspace scope only (3 equivalents, 2 of
them caught only spuriously); its crate-scope misses after the change are **unmeasured**. Still
open: one `daemon` loop mutant reported as a timeout, `review` in
workspace scope (727 crate-scope misses, unclassified), and `cli`, `tui`, `learning`,
`pipeline`, `projection`, `cluster` and `sdk` (**unmeasured**). The 120 historical misses stay
in the ledger as history.

## Follow-up 2026-10-31: the platform matrix (no rescore)

Answers the accepted point "'Linux and macOS' is broad" and item 3 above. Not a new assessment:
the score stays as reconciled until the next one.

- **Stated** (measured, read): the scope is four targets, named in README.md#supported-platforms,
  `SECURITY.md` and `RELEASE_AND_PUBLICATION.md`; `cli/tests/crate_edges.rs` holds the release
  matrix and the target triples those docs name to the same list, and checks that each doc
  names the glibc floor and states musl as unsupported. musl, Windows, other architectures and the BSDs are
  stated as unsupported.
- **glibc floor 2.34** (measured on the v0.5.0 x86_64 binary: highest required version
  `GLIBC_2.34`; ran on AlmaLinux 9, Ubuntu 22.04 and Debian 12; refused on Debian 11). On glibc
  2.34 to 2.38 the loader prints one stderr warning per run (measured). Enforced by
  `scripts/check_glibc_floor.sh` in the release smoke.
- **arm64 Linux**: a release target built on `ubuntu-22.04-arm` and a `Linux arm64 tests` CI job
  on the same image, which gates every release build through `quality`. On `ubuntu-24.04-arm` the
  binary needed `GLIBC_2.39`, over the floor (measured in PR #79); on 22.04-arm
  `scripts/check_glibc_floor.sh` passes the 2.34 floor (measured in CI).
- **x86_64 macOS** runs in the release smoke under Rosetta 2 (since the 2026-10-31 release
  hardening); it has no test job.
- **launchd**: the `macOS launchd agent` job loads the shipped plist into a hosted Mac's launchd
  and checks health, crash restart, no restart after `daemon stop`, and bootout (pending its
  first CI run; the flow ran only against a fake `launchctl` on Linux). Skipped in release runs
  until it has a record on hosted runners.

## Follow-up 2026-10-31 (unreleased): the refused commands

Codex's facet-6 gap, "`chain`, `cluster`, `loom` and direct `<old> <new>` remain refused despite
published contracts", is closed by the change that adds this note, after v0.5.0 (measured: `cli/tests/file_pair.rs`,
`cli/tests/contract.rs`). `<old> <new>` reviews two regular files through `git diff --no-index`
with the acquisition's hardening; directories are refused. `chain`, `cluster` and `loom` were
withdrawn, not built: each is refused with exit 2 and names its replacement, `--help` lists them
under "WITHDRAWN", and the three contracts list only modes a test runs. The score above is not
changed by this note.

## Follow-up 2026-10-31 (unreleased): the evaluation corpus (not a rescore)

The Codex finding that the gate "is only 11 cases" and "excludes one declared miss from recall"
is addressed; no score above is changed by it.

- **[Measured]** `fixtures/eval` has 34 cases: 14 ranking (3 hard negatives), 17 evidence and 3
  findings. Every origin is classified: a CVE, GHSA or RUSTSEC shape with its identifier, a
  real-world shape, one of this project's findings, or `synthetic:` with its reason.
- **[Measured]** Declared misses are counted, not excluded. The 11 declared (3 ranking pairs,
  7 missed evidence items, 1 false positive) count as wrong, so the gate reports ranking 12/15,
  precision 21/22, recall 21/28 and findings 33/33, and its floors are those values. Before,
  the one declared miss was left out of recall, which read 7/7.
- **[Measured]** The manifest cannot drift silently: a file no case names, a symlink, a
  repeated id, a missing origin, a field the case's kind does not read or a ranking pair that is
  not two distinct paths is refused (exit 4), and a declared miss the engine now gets right
  fails (exit 7).
- **[Inferred]** It is still a corpus written by the rules' author, with cases shaped after real
  advisories rather than taken from them. It is a regression gate and a list of known limits,
  not evidence of review quality on real-world code; a held-out corpus remains open.

## Follow-up 2026-11-01: release authority, secret scanning and the Mac (no rescore)

Owner decisions of 2026-11-01 against the reconciled list above. Not a new assessment.

- **[Measured]** Release authority, item 1: the `release` environment now has the owner as
  its required reviewer and a deployment policy of `v*` tags only (`gh api
  repos/Louranicas/deep-diff-forge/environments`); the `v*` tag ruleset (no deletion, update or
  bypass) and the fail-closed crates.io job are on `main` (CHANGELOG, Unreleased). Only the
  crates.io `publish` job runs in that environment. Still open: signed tags, and crates.io
  trusted publishing (an account-level step).
- **[Measured]** Code scanning and secret patterns, item 2: CodeQL runs (`codeql.yml`; not a
  required check). Push protection and provider patterns are on; non-provider patterns and
  validity checks stay `disabled` for this user-owned repository although the API accepts the
  change (GitHub documents validity checks for organisation repositories with Secret
  Protection). gitleaks, in the required `secrets` job and the pre-push hook, is the
  compensating control. This part of item 2 is closed as unavailable, not done.
- **[Measured]** Items 4 and 5 moved: `daemon stop` drains, a systemd user unit and a launchd
  agent ship in `contrib/`, and the Python client is a package (`sdk/python/pyproject.toml`,
  not on PyPI); all unreleased.
- **On hold:** running the launchd agent on a physical Mac. No real Mac is available; the
  `macOS launchd agent` CI job on a hosted runner is the only macOS launchd evidence. It is an
  open item for future maintainers ([roadmap](ROADMAP.md#open-items-for-maintainers)).

## Assessment 2026-11-02 (`main` `1eaba47`, Claude's self-assessment)

The same seven facets and weights. The scope is unchanged: a local, single-user review engine.
The score was written before reading the independent rescore below.

**What changed since the reconciled 86 (2026-10-31):**

- The full-stack bug hunt (#82, #83). 53 findings were confirmed; 49 were fixed with
  failing-first tests and diff mutation; 3 are deferred (listed under known limitations below);
  1 could not be reproduced. The most serious fixed findings:
  - a submodule clean filter ran during working-tree acquisition;
  - a 619 KB request could abort the daemon;
  - the pre-push secret hook failed open;
  - the release build job held write and OIDC grants, and one failed platform could publish a
    partial release.
- Release authority: an approval-gated `release` environment, a `v*` tag ruleset and a
  fail-closed crates.io job.
- Platforms: arm64 Linux in CI and as a release target; the x86_64 macOS archive runs under
  Rosetta 2 in the release smoke.
- Operability: `daemon stop` drains, and a systemd user unit and a launchd agent ship.
- Interfaces: the Python client has package metadata.

| # | Facet | Weight | Reconciled 10-31 | Self 11-02 |
| --- | --- | ---: | ---: | ---: |
| 1 | Correctness and patch-truth integrity | 20 | 19 | 19 |
| 2 | Security and trust boundary | 15 | 13 | 14 |
| 3 | Supply chain and release integrity | 15 | 13 | 14 |
| 4 | Reliability and operability | 15 | 13 | 14 |
| 5 | Platform and portability | 10 | 7 | 8 |
| 6 | Interfaces and integration contracts | 15 | 13 | 14 |
| 7 | Documentation, governance and evidence | 10 | 8 | 8 |
| | **Total** | **100** | **86** | **91** |

## Independent rescore 2026-11-02 (Codex, `main` `1eaba47`)

Codex (`codex exec -s read-only`) read the tree at `1eaba47` and was given the measured facts it
could not check itself (test counts, CI results and repository settings). It was not shown the
self-assessment.

| # | Facet | Weight | Codex | Reason (Codex's, shortened) |
| --- | --- | ---: | ---: | --- |
| 1 | Correctness and patch-truth integrity | 20 | 19 | Strong parser and oracle testing, mutation-gated fixes. Against: one patch-model ambiguity; incomplete mutation and evaluation evidence. |
| 2 | Security and trust boundary | 15 | 14 | Hostile-input bounds, same-UID daemon, sandbox, CodeQL and secret controls. Against: CodeQL optional; the user's global Git filters stay trusted. |
| 3 | Supply chain and release integrity | 15 | 13 | Pinned Actions, audits, SBOM, provenance, immutable tags, split release jobs. Against: unsigned tags; the GitHub Release is not behind the approval. |
| 4 | Reliability and operability | 15 | 13 | Bounded parallel daemon, drain, metrics, soak, service units. Against: no control path at saturation; a signal stop does not drain. |
| 5 | Platform and portability | 10 | 8 | Four targets, arm64 Linux CI and smoke. Against: minimum OS versions are not executed. |
| 6 | Interfaces and integration contracts | 15 | 14 | Shared tested contracts. Against: SDK distribution; the old/new-path form. |
| 7 | Documentation, governance and evidence | 10 | 8 | Extensive and candid. Against: recent PR evidence not recorded; packaging statements drift. |
| | **Total** | **100** | **89** | |

## Reconciliation 2026-11-02 (Claude, after the independent rescore)

Each of Codex's 18 gaps was checked against the tree, and the ones that can be fixed in this
change were fixed.

| Gap (Codex's numbering) | Verdict | Action |
| --- | --- | --- |
| 17: tracked, stale Python build output | **Accepted, and worse than stated.** Reproduced: `pip install ./sdk/python` from `1eaba47` installed the stale copy, the client from before the bug-hunt fixes. | Fixed in this change (CHANGELOG, Fixed); a test pins the `.gitignore` entries. |
| 16: `pyproject.toml` installs from `@v0.5.0`, which has no package metadata | **Accepted.** | The comment names `@main`, as the README already does. |
| 6: CodeQL is not required | **Accepted.** | Required since 2026-11-02 (thirteen checks). |
| 18: `EVIDENCE.md` does not record #82 to #84 | **Accepted.** | Recorded ([EVIDENCE](../EVIDENCE.md#bug-hunt-merges-docs-sync-and-required-codeql-2026-11-02)). |
| 12: no complete arm64 or launchd CI evidence | **Partly disputed.** Both jobs passed on #83 and #84 (measured, now in EVIDENCE). They are still not required, and no release dry run exists for this commit. | The dry run comes with the next release. |
| 10: the GitHub Release is not behind the approval | **Accepted.** Only crates.io `publish` names the `release` environment. | Open, for the owner: gating the release job puts every release on hold for the owner's approval. |
| 9, 11: unsigned tags; a long-lived crates.io token | **Accepted.** | Open, for the owner (a signing key; trusted publishing is an account step). crates.io is on hold. |
| 13: no control path at saturation; a signal stop does not drain | **Accepted.** SIGTERM has no handler (`OPERATIONS_AND_DAEMON.md`, launchd notes); the systemd unit drains through `daemon stop`. | Open: DDF-HUNT-DAEMON-7 below. |
| 2: `diff --git a/x b/y` with no rename header | **Accepted.** | Open: DDF-HUNT-PATCH-6 below. |
| 1: the tree is not a released version | **Accepted.** | The next release. |
| 3, 4, 5, 7, 8, 14, 15 (eval corpus, whole-crate mutation, campaign schedule, secret patterns, global Git filters, node-exporter path, minimum OS versions) | **Accepted** as stated; each is already recorded as open or residual above. | None now. |

### Reconciled score: 89 / 100

| # | Facet | Weight | Self | Codex | Reconciled |
| --- | --- | ---: | ---: | ---: | ---: |
| 1 | Correctness and patch-truth integrity | 20 | 19 | 19 | 19 |
| 2 | Security and trust boundary | 15 | 14 | 14 | 14 |
| 3 | Supply chain and release integrity | 15 | 14 | 13 | 13 |
| 4 | Reliability and operability | 15 | 14 | 13 | 13 |
| 5 | Platform and portability | 10 | 8 | 8 | 8 |
| 6 | Interfaces and integration contracts | 15 | 14 | 14 | 14 |
| 7 | Documentation, governance and evidence | 10 | 8 | 8 | 8 |
| | **Total** | **100** | **91** | **89** | **89** |

Facets 3 and 4 take Codex's score: the release approval covers only crates.io, and the daemon
has no control path at saturation. The fixes in this change are not scored here; they count at
the next rescore.

### Known limitations (deferred by the 2026-11-01 bug hunt, open on `main`)

| ID | Area | Limitation | Why deferred |
| --- | --- | --- | --- |
| DDF-HUNT-PATCH-6 | patch | A `diff --git a/x b/y` section with no rename header is reviewed as `modified y`, while `git apply` deletes `x` and creates `y`. The model has no old-path field for this form. | Needs a patch-model decision: represent it, or refuse it. |
| DDF-HUNT-DAEMON-7 | daemon | Eight idle connections fill the daemon's connection limit, and then `daemon.shutdown` and `health` cannot get in. Only a signal stops it then. SIGTERM, which `launchctl bootout` also sends, does not drain and leaves the socket file behind (the systemd unit stops through `daemon stop`, which drains; [operations](OPERATIONS_AND_DAEMON.md)). A same-user process can cause this. | Needs a control-channel design. |
| DDF-HUNT-CLI-8 | cli | `findings verify --findings FIFO` (and `review --findings`) waits forever for a writer. | Low severity: the caller chose the path. |

### What would raise the score, in order of impact

1. **Release the hardened tree** (facets 3 and 7): a four-target dry run, then a tag. Owner
   decisions: signed tags, and the GitHub Release job behind the `release` approval.
2. **Close DDF-HUNT-PATCH-6** (facet 1): refuse or represent the old/new-path form. Add a
   held-out real-world evaluation corpus.
3. **A daemon control path at saturation** (DDF-HUNT-DAEMON-7) and a drain on supervisor
   signals (facet 4).
4. **Whole-crate mutation** of `review`, `daemon` (crate scope) and the unmeasured crates, with
   survivors classified (facet 1).
5. **Record the first scheduled campaign**, and run the node-exporter path and the minimum OS
   versions (facets 4 and 5).

## Owner decisions and the signed v0.6.0 tag (2026-11-03)

Added after the 89/100 reconciliation; it does not rescore.

- **[Owner decision]** Release tags are signed. **[Measured]** `v0.6.0` (tag object
  `d165dd9`, commit `571006b`) carries an SSH signature, and `git tag -v` with
  `.github/allowed_signers` reports a good signature from ED25519 key
  `SHA256:h4S5C3lX4poSYOx8ii8yezKBlp5pQf2jy168XgwWR8I`. `just release-tag` now signs and
  verifies through `scripts/sign_release_tag.sh`, tested in `cli/tests/repo_scripts.rs`.
- **[Owner decision]** The GitHub Release job stays outside the `release` approval. Releases
  are run by an agent (Claude Code) after the owner approves each tag; the approval covers
  only crates.io. Codex's facet-3 gap "the GitHub Release is not behind the approval" is
  accepted, not open.
- **[Owner decision]** crates.io stays on hold until the owner reopens it, so trusted
  publishing is deferred with it.
- **[Measured]** GitHub reports the v0.6.0 tag signature as `verified: false`,
  `reason: unknown_key`: the signing key is not registered on the account. Registering it
  as a signing key on GitHub and GitLab is an account action for the owner. The
  `allowed_signers` check does not depend on it.
- **[Measured, correction 2026-11-03]** The owner has since registered the key on GitHub as a
  signing key. GitHub now reports the v0.6.0 tag object `d165dd9` as `verified: true`,
  `reason: valid`, so the `unknown_key` line above is superseded. GitLab: not registered.

### DDF-HUNT-CLI-8 fixed (2026-11-03)

**[Measured]** The deferred limitation "`findings verify --findings FIFO` (and `review --findings`)
waits forever for a writer" (Known limitations above) is fixed. The same hang was also found in
`eval`, when the corpus's `MANIFEST.json` is a FIFO. All user-named input files now open through
`crates/deep-diff-forge-cli/src/regular.rs` (`O_NONBLOCK` plus `fstat` on the open descriptor), so
a non-regular file is refused at once. The regression tests `a_fifo_as_the_findings_file_is_refused_not_waited_on`
(`cli/tests/findings.rs`) and `a_fifo_manifest_is_refused_not_waited_on` (`cli/tests/eval.rs`)
fail on the old code, where they hit their 10-second deadline, and pass on the new.

### DDF-HUNT-DAEMON-7 fixed (2026-11-03)

Supersedes the DDF-HUNT-DAEMON-7 rows above (known limitations and gap 13); they stay as
history. **[Measured]**
- Two control slots beyond the eight data slots answer `daemon.health`, `daemon.status`,
  `daemon.metrics` and `daemon.shutdown`. Each holds one request of at most 4 KiB, read within
  5 s. Test: `serve::tests::a_daemon_at_its_connection_limit_still_answers_health_and_stops`.
  It fails with the control slots set to 0.
- SIGTERM and SIGINT stop the daemon as `daemon stop` does: no new connections, the socket
  removed, a drain, exit 0. Test: `sigterm_and_sigint_stop_the_daemon_and_remove_its_socket`
  (`cli/tests/daemon.rs`). It fails on the previous code, where the signal killed the process.
- Residual: a same-user process that keeps reconnecting can still hold the data and control
  slots. A signal from the same user, which `systemctl stop` and `launchctl bootout` send,
  always stops the daemon cleanly.

### DDF-HUNT-PATCH-6 fixed, a fuzz crash fixed, scheduled runs confirmed (2026-11-03)

**[Measured]**
- **DDF-HUNT-PATCH-6** (Known limitations above, now history). Git was asked directly: for
  `diff --git a/x b/y` with `--- a/x`/`+++ b/y` and no rename header, `git apply` reports
  `x => y`, deletes `x` and creates `y`. DDF now records the rename headers Git would write, so
  the status is `renamed` and the old path is kept. Test:
  `differing_names_without_a_rename_header_are_the_rename_git_applies` (`patch/tests/git_oracle.rs`)
  applies the original and the rendered patch and checks the tree. It fails on the old parser.
  Without `---`/`+++` (a mode change only), Git refuses the section ("lacks filename
  information") and DDF keeps its earlier reading of the `diff --git` line, unchanged.
- **DDF-FUZZ-PATCH-8**. The first scheduled fuzz campaign (run 41456383570, 2026-11-02, on
  `571006b`, the v0.6.0 commit) found a `patch_parser` crash. A quoted `diff --git` half
  squashed `//` while an unquoted one kept it, so parse and render drifted. Both now keep the
  slashes, as `git_header_name` does. The input is in `fixtures/patch/regressions/` and the test
  `a_doubled_slash_in_a_diff_git_name_survives_quoting` fails on the old parser. The defect is in
  v0.6.0. Its impact: a name with `//` in the `diff --git` line of a patch that has no `---`/`+++`
  could be reviewed under a path with one slash fewer than Git's.
- **Scheduled runs do fire.** The earlier observation that they never ran was taken too early.
  On 2026-11-02 `codeql` (12:30 UTC), `ci` (12:36) and `campaign` (13:49) ran from their crons,
  about eight hours after the scheduled times, as GitHub allows under load. `ci` and `codeql`
  passed. `campaign` failed on the crash above.
- **GitLab signing key (correction, 2026-11-03).** The release signing key is now registered on
  GitLab too, with usage type Signing, so the "GitLab: not registered" lines above are
  superseded. GitLab's tag signature API reports only `signature_type: SSH` for v0.6.0, so a
  "Verified" status there is unmeasured.

## Assessment 2026-11-03: adversarial circle of experts

Assessed after v0.6.0, on the release candidate branch (`fix-deferred-bugs` at `ff58ccb`, then
PR #92). The circle had five experts, each scoring all seven facets independently from the
code, without seeing the others' scores:

- four Codex personas (`codex exec -s read-only`): Mara Ilic (Git internals), Ivo Navarro
  (offensive security), Sena Okafor (site reliability) and Okon Adeyemi (release governance);
- one Claude persona: Rhea Lindqvist (agent integration).

A sixth agent, the chair (Claude), then tried to refute each of the 17 claims that would change
a score. Verdicts: 12 confirmed, 5 partly confirmed, 0 refuted.

| # | Facet | Weight | Mara | Ivo | Sena | Okon | Rhea | Mean | Reconciled |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | Correctness and patch-truth integrity | 20 | 16 | 19 | 19 | 18 | 19 | 18.2 | 19 |
| 2 | Security and trust boundary | 15 | 14 | 12 | 14 | 13 | 13 | 13.2 | 13 |
| 3 | Supply chain and release integrity | 15 | 13 | 10 | 12 | 10 | 13 | 11.6 | 12 |
| 4 | Reliability and operability | 15 | 14 | 13 | 12 | 14 | 13 | 13.2 | 13 |
| 5 | Platform and portability | 10 | 8 | 8 | 7 | 8 | 8 | 7.8 | 8 |
| 6 | Interfaces and integration contracts | 15 | 12 | 13 | 14 | 14 | 12 | 13.0 | 13 |
| 7 | Documentation, governance and evidence | 10 | 8 | 8 | 8 | 5 | 8 | 7.4 | 8 |
| | **Total** | **100** | **85** | **83** | **86** | **82** | **86** | **84.4** | **86** |

**Reconciled: 86 / 100.** The previous reconciled score was 89. The lower score does not mean the
code got worse. A sharper, adversarial reading found real defects that earlier assessments missed:

- the first scheduled fuzz campaign found a crash in the shipped v0.6.0 (DDF-FUZZ-PATCH-8);
- release authority rested on procedure, not on an enforced control;
- the parser accepted move headers that Git refuses;
- the accept loop had a cleanup gap.

The reconciled column credits the fixes below, all merged or in the follow-up PR, and keeps the
deductions for what remains open.

### Fixed in response (each claim checked first, each fix test-proven)

| Claim | Finding | Fix |
| --- | --- | --- |
| Mara 1 | An implied move (PATCH-6) to or from a path Git refuses was accepted, and DDF then refused its own rendering | Refused at parse (git oracle test) |
| C1 | A lone `rename from/to` or `copy from/to` header was accepted; Git refuses it | Refused unless `---`/`+++` names the other side (oracle test) |
| C2 | `100644 -> 100664` was reported as a mode change; Git canonicalises it away | `canon_mode` as Git does; a mode change Git reads as no change is refused |
| C4 | Any pushed `v*` tag published; signing was only checked locally | The supply-chain gate verifies the tag against `main`'s `allowed_signers` before building |
| C5 | `eval --corpus` read files without limit | 8 MiB per file, 64 MiB per corpus |
| C6 | SBOM checksums hashed `Cargo.toml` | The `.crate` SHA-256 recorded in Cargo.lock |
| C7, C11, C12, C13 | Stale or incomplete SECURITY/EVIDENCE statements | Dated corrections |
| C8 | A fatal `accept()` error skipped cleanup and drain | It runs the shutdown epilogue; transient errors are retried |
| C10 | Soak receipts held no daemon metrics | The soak records `daemon metrics` at the start and the end |
| C15 | `renamed` gave no structured old path; the old path's hazards were ignored | `old_path` in review.v0, JSONL and text; its hazards count |
| Rhea 2 | `modified` became `renamed` for the same input, filed only as a fix | `### Changed` entry, and the v0 compatibility rule written down |
| macOS CI | At the connection limit, macOS's EINVAL read as "no daemon" | Mapped to "no answer", exit 6 |

### Open, with owner decisions noted

| Claim | Open item | Status |
| --- | --- | --- |
| C3 | Up to eight maximum-size request lines buffered before admission (about 1.25 GiB worst case) | Design: account for bytes while reading |
| C9 | No execution deadline or cancellation after the client gives up | Design: a cooperative deadline in dispatch |
| C14 | Release archives are not byte-reproducible | Design: normalised tar/gzip, `SOURCE_DATE_EPOCH`, pinned images |
| C16 | "Not a regular file" exits 3 in some commands and 4 in others | Needs an exit-code policy decision; it changes caller behaviour |
| C17 | README headline readiness for four targets, while minimum OS floors are not executed | Wording |
| Ivo 3 | Global, system or command-scope Git filters still run on hostile repositories | Accepted trust boundary (same UID) |
| Release authority | The GitHub Release job is outside the approval environment; the signing key has no passphrase and shares a UID with the forge tokens | **Owner decisions 2026-11-03: accepted.** The tag-signature check (C4) now enforces the key half. |
| v0.6.0 | The shipped release contains DDF-FUZZ-PATCH-8 | Recommendation: release 0.7.0 (the `renamed` value change makes it more than a patch release) |

### What would raise the score

1. Release 0.7.0 with these fixes (facets 1, 3 and 7).
2. Byte accounting before admission, and dispatch deadlines (facets 2 and 4).
3. Reproducible archives and per-target SBOMs (facet 3).
4. One exit code for "not a regular file", and a transient flag on RPC error 8 (facet 6).
5. Run the minimum OS floors, or narrow the README claim (facet 5).

### C14 addressed (2026-11-03)

**[Measured]** Release archives are packed by `scripts/package_release.py`, so two packs of the
same inputs are byte-identical (`a_release_archive_is_the_same_bytes_from_the_same_inputs`). The
control `plain_tar_czf_is_not_reproducible` shows that the old `tar czf` packs differed.
**[Unmeasured]** The script has not yet run on the macOS runners; the next release dry run is
its first. Open: the macOS image floats (`macos-latest`), and the binaries themselves have not
been rebuilt independently and compared.

### C3 and C9 fixed (2026-11-03)

**[Measured]** These two open items from the circle assessment above are fixed. The "Open" rows
for them stay as history.
- **C3:** request lines held across connections share a 160 MiB read budget (`ReadBudget`,
  `admission.rs`). Tests: `request_lines_being_read_share_one_byte_budget` (socket) and
  `concurrent_readers_never_hold_more_than_the_budget` (peak at most the limit). The socket test
  fails with the reservation disabled.
- **C9:** socket requests stop at the client's 30 s deadline, checked between stages
  (`deadline.rs`). Test: `a_request_past_its_deadline_is_stopped_and_releases_its_weight`. It
  fails with the check disabled.

### macOS runner pinned (2026-11-03)

The open item above ("the macOS image floats") is closed for the major version. `ci.yml` and
`release.yml` run on `macos-26`, the image `macos-latest` named on 2026-11-03 (measured from the
release dry run 41589040037 logs: `Image: macos-26-arm64`, version `20261026.0455.2`). The test
`no_workflow_runs_on_a_floating_runner_label` rejects any `-latest` label, and a planted
`macos-latest` made it fail (measured). Still open: GitHub updates the image's patch level weekly,
so reproducing a macOS binary bit for bit needs the same image version. No rescore.

## Assessment 2026-11-04 (`main` `7c0910b`, v0.7.0 and test-only fixes; Claude)

**Score: 80 / 100**, down from the reconciled 86. Four reviewers each read one layer (patch
parser, daemon and SDKs, CLI contracts, supply chain); every important claim was then checked
against the published v0.7.0 binary before it was reported.

**The blocker (measured):** `@@-1,1 +1,1 @@`, a hunk header with no space after `@@`, was
accepted. Git does not read it as a hunk header, and reads the lines after it as a new file. In
a scratch repository DDF reported one modified file (`x`, 3 hunks, exit 0), while `git apply`
passed `--check` and changed `x` **and** rewrote a second file DDF never showed. The parser
trimmed whitespace after `@@`; Git requires exactly `@@ -`. The fuzz target compared DDF only
with itself, skipped non-UTF-8 input, and the Git-oracle tests had no header cases.

| Facet | Weight | Before | 2026-11-04 |
| --- | ---: | ---: | ---: |
| Correctness and patch-truth integrity | 20 | 19 | 15 |
| Security and trust boundary | 15 | 13 | 13 |
| Supply chain and release integrity | 15 | 12 | 13 |
| Reliability and operability | 15 | 13 | 12 |
| Platform and portability | 10 | 8 | 7 |
| Interfaces and integration contracts | 15 | 13 | 12 |
| Documentation, governance and evidence | 10 | 8 | 8 |
| **Total** | **100** | **86** | **80** |

### Fixed in response (each claim checked first, each fix test-proven)

**[Measured]** Every fix below has a test that fails without it.

- **#100, the parser (item 1).**
  - `parse_hunk_header` is a literal port of Git's grammar (`@@ -` RANGE ` +` RANGE ` @@`).
  - A hunk may open only where Git's would: after `+++`, a Git extended header, the previous
    hunk or its `\` marker (`MisplacedHunk`).
  - Refused as Git refuses or ignores them: a hunk with no `+` or `-` line
    (`HunkWithoutChange`), and a section Git reads no patch from (`SectionWithoutPatch`).
  - A header line that is not UTF-8 is refused (`LossyHeader`, exit 3), instead of being
    reviewed under a substituted name.
  - `tests/git_differential.rs` is a differential oracle: whatever DDF accepts, `git apply
    --numstat -z` must read as the same files and counts. It covers more than 150 one-character
    mutants of a hunk header in two placements. All six of its tests fail on v0.7.0's parser.
  - The fuzz target decodes raw bytes and is seeded from every patch fixture.
- **#101, the daemon and child processes (items 3 and 4).**
  - A request's read budget goes back once its answer is ready.
  - Each response has 30 s from its first byte. It is written non-blocking, with the deadline
    checked before every write and each wait for space bounded by `poll`. A blocking write
    cannot give that bound, because `SO_SNDTIMEO` restarts for each buffer a call waits for.
    Chunked blocking writes passed on Linux and failed on macOS CI; a 4 KB send buffer now
    models macOS on Linux.
  - `review.change` checks its deadline between its two Git stages.
  - `core::process::run_bounded` gives `tuicr` and the council's Git calls a deadline and
    capped output. `clippy.toml` disallows the unbounded waits (`Command::output`,
    `Command::status`, `Child::wait`, `Child::wait_with_output`).
  - `findings verify --require-findings` exits 7 on a findings file with no findings. Non-UTF-8
    input exits 3 everywhere.
  - Silent writes cannot compile: the `Sent` and `Written` proof types have no `Default`.
  - Diff mutation on the PR: 0 missed, 0 timeouts.
- **Release builds from one codegen unit (#101).** The bench gate caught `parse_seal` 1.27x
  slower on a branch that changed no parser code. Identical binaries measured 1.00x; built with
  one codegen unit each, base and head were within 1.01x. The default 16-unit split had made the
  shipped parser slower by chance. `release_binaries_are_built_from_one_codegen_unit` keeps the
  setting.
- **The fuzz campaign (item 2):** run 41825957934 on `main` `6358dc4` passed (measured). All ten targets ran for
  3,600 s each, alongside the long daemon soak and the full bench. `patch_parser` ran
  14,413,643 inputs (about 4,000 a second) to 2,328 coverage edges, from raw bytes and the
  seeded fixtures, with no crash. The last scheduled run (2026-11-02) had failed on that target;
  that crash is a regression fixture now.

### Open

- **Owner decisions (item 5).**
  - Turn on Immutable Releases (an agent's attempt was refused, correctly, as a shared-settings
    change).
  - Decide whether to yank crates.io `0.2.0`.
  - Tag 0.7.1; each tag needs the owner's word.
- **Test hygiene (new, measured).** Tests build scratch directories from
  `std::env::temp_dir()` directly, at about 100 call sites, many with no guard that removes
  them. One clean run of the daemon and core tests left 37 directories behind.
- **From the assessment, unchanged.**
  - The council's outputs carry no schema version.
  - A `release.yml` edited on a tagged branch would bypass the tag-signature check
    (same-owner risk).
  - The SIGTERM wake-up depends on reconnecting by socket path, and a second signal during the
    drain is ignored.

No rescore until 0.7.1 ships and an independent rescore reads it.

## Assessment 2026-11-07 (`v0.8.0`, `dd1aa89`; independent rescore)

**Score: 80 / 100**, level with v0.7.0 but for different reasons. A fresh assessor, given the
rubric above and a read-only checkout of the tag but not the previous assessor's conclusions,
read the code, ran the suite (1,929 passed, 0 failed) and drove the released binary. It also
ran a random line-mutation differential against `git apply` (60,000 mutants of `fixtures/`).
Its high finding was then reproduced by hand against Git 2.56 before it was acted on.

| Facet | Weight | 2026-11-04 (v0.7.0) | 2026-11-07 (v0.8.0) |
| --- | ---: | ---: | ---: |
| Correctness and patch-truth integrity | 20 | | 15 |
| Security and trust boundary | 15 | | 12 |
| Supply chain and release integrity | 15 | | 13 |
| Reliability and operability | 15 | | 13 |
| Platform and portability | 10 | | 8 |
| Interfaces and integration contracts | 15 | | 12 |
| Documentation, governance and evidence | 10 | | 7 |
| **Total** | 100 | **80** | **80** |

The 2026-11-04 section scored by a different facet split, so its column is left blank rather
than mapped.

**Up (measured):** the hunk-header smuggle is closed; SIGTERM drains and removes the socket; the
control slots answer while every data slot is held; idle connections drop at 30 s; the tag
signature, the SLSA attestation and the checksums verify; Immutable Releases is on; a full test
run leaves no scratch directory behind.

**The new high finding (F1, measured, fixed on `main` after v0.8.0):** Git ends a `diff --git`
section's extended header at the first line it does not read as one, and at a last line with no
line feed; DDF read on. `rename to .github/workflows/pwn.yml`, a line of text, then
`rename to docs/harmless.md` was reviewed as a docs move (exit 0, CLI, `rpc` and MCP alike)
while `git apply` installed the workflow. The 0.8.0 differential oracle mutated only hunk
headers, so this sibling of the 0.7.0 blocker went uncaught. Two related readings (F2: a
`---`/`+++` pair after the header ended; F3: `+++ /dev/null` without `deleted file mode`)
are fixed with it, and the oracle now breaks every extended-header position
(`no_extended_header_mutant_reads_differently_from_git`).

**Open:**

- crates.io `0.2.0` is still installable and not yanked (owner decision; F4).
- A same-user process holding ten idle connections blocks `health` and `stop` until they time
  out (F5; the accepted same-user boundary).
- The status lines shipped inside a release archive are written before the tag, so they say
  "not yet tagged" (F6). The post-release docs PR corrects `main` only.
- `Linux arm64 tests` and `macOS launchd agent` are not required checks, and `launchd` is
  skipped on release runs (F7).
- The differential is not in the fuzz harness.

The low-severity items left open on 2026-11-04 are fixed on `main`: the council's results name
their schema; a release is built only from a signed tag on `main`; a signal stops the daemon
without its socket path, and a second signal ends the drain. A rescore follows 0.8.1.

## Assessment 2026-11-09 (`v0.8.1`, `d290379`; independent rescore)

**Score: 80 / 100**, level with v0.8.0. F1 to F3 are fixed in the release: their repros exit 4.
But two new high findings of the same kind turned up, both present since v0.8.0. As before, the
assessor was a fresh agent with the rubric and a read-only checkout of the tag, but not the
previous conclusions. It read the code, ran `cargo test -p deep-diff-forge-patch` (249 passed)
and drove the released binary. It wrote 191 probes and ran about 183,000 random line mutants,
each compared against `git apply --numstat` and `--summary`. Each finding below was reproduced
by hand against Git 2.56 with a real apply before it was acted on.

| Facet | Weight | 2026-11-07 (v0.8.0) | 2026-11-09 (v0.8.1) |
| --- | ---: | ---: | ---: |
| Correctness and patch-truth integrity | 20 | 15 | 14 |
| Security and trust boundary | 15 | 12 | 11 |
| Supply chain and release integrity | 15 | 13 | 14 |
| Reliability and operability | 15 | 13 | 14 |
| Platform and portability | 10 | 8 | 8 |
| Interfaces and integration contracts | 15 | 12 | 13 |
| Documentation, governance and evidence | 10 | 7 | 6 |
| **Total** | 100 | **80** | **80** |

**Up (measured):**

- A release is built only from a signed tag whose commit is on `main`.
- A signal stops the daemon after its socket path is gone, a second signal ends the drain, and
  an idle daemon uses no CPU.
- Council results name their schema.

**The new findings (measured; fixed on `main` after v0.8.1):**

- **N1 (high).** DDF never read `copy to`, so a copy was reviewed under the `diff --git` or
  `+++` name. `diff --git a/README.md b/docs/README-copy.md` with
  `copy to .github/workflows/pwn.yml` and a hunk was reviewed as a docs copy (exit 0, CLI, `rpc`
  and MCP) while `git apply` wrote the workflow with the hunk's content.
- **N2 (high).** `diff --git a/.github/workflows/ci.yml …` with `--- README` / `+++ README`:
  Git reads a name that `-p1` leaves nothing of as no name, and changed the workflow. DDF
  reviewed `README`.
- **N3 (medium).** Repeated mode headers, and an `index` line's mode: Git keeps the last, and
  made a script executable that DDF, reading the first, showed unchanged.

The oracle compared numstat only, never modes or copies, and its mutants never wrote `copy to`
or a name without a `/`. It now compares `--summary` too and inserts each of those
(`no_duplicated_or_copy_header_mutant_reads_differently_from_git`). Its first run on the fix
found two further readings Git refuses, which are refused now as well: a second `---`/`+++`
pair before a Git section's first hunk, and a new or deleted file whose `+++` or `---` differs
from the `diff --git` name.

**Down:** the README called 0.8.1 production-ready and its differential complete over "every
extended-header position"; it compared numstat only. Both statements are corrected.

**Open:**

- crates.io `0.2.0` is still installable and not yanked (owner decision; F4).
- Ten idle same-user connections block `health` and `stop` until they time out (F5).
- The status lines inside a release archive still say "not yet tagged" (F6).
- `Linux arm64 tests` and `macOS launchd agent` are not required checks (F7).
- The differential is not in the fuzz harness.
- **Low:** DDF accepts some inputs Git refuses, which it reviews as changes that never apply:
  - empty or patch-free input (exit 0, no files);
  - binary bodies it does not decode;
  - a `diff --git` line with two different names and no other name, which is the form
    `git diff --no-index` writes for the file-pair mode.

  A traditional `@@ -0,0` hunk on `--- a/x` creates a file to Git and is labelled `modified`;
  its content is shown in full.

## Assessment 2026-11-09 (`main` `af5c3d1`, the untagged 0.8.2; independent rescore before the tag)

**Score: 80 / 100**, level with v0.8.1. A fresh assessor scored the code to be tagged as v0.8.2, with the rubric and a read-only checkout, and read the previous section only after forming a view. N1 to N3 are fixed: the copy is read as Git reads it, and N2 and N3 exit 4. But one new high finding and one medium of the same kind turned up, so the tag waited for their fix. The facets are unchanged from v0.8.1 (14, 11, 14, 14, 8, 13, 6). There is no release yet, so supply chain was scored on the v0.8.1 pipeline and the green CI on `af5c3d1`.

**Method (measured):** 34 hand probes, a random differential of 36,000 cases over six seeds (one or two sections, up to four random extended headers of every kind, names with directories, quotes, UTF-8, CR, tab and `a//b`, CRLF and an unterminated last line), each compared against `git apply --numstat -z` and `--summary` with Git 2.56. Workspace tests: 1,939 passed. Both findings were reproduced by hand against a real `git apply` before they were fixed.

**The findings (fixed before the tag):**

- **F1 (high).** Git ends a section's extended header at `Binary files … differ` or `GIT binary patch`; DDF read on. `diff --git a/.github/workflows/ci.yml …`, an `index` line, `Binary files … differ`, then `copy from docs/logo.png` / `copy to docs/logo-copy.png` was reviewed as a binary copy of a logo (exit 0, CLI and MCP), while `git apply` rewrote the workflow from the `index` blob. The blob only has to be in the object database, for example from any fetched branch. The rename form was already in v0.8.1. The N1 fix moved the copy form's reviewed path off the workflow entirely. Mode lines after the binary line showed a mode change Git never makes.
- **F2 (medium).** Git reads an unquoted move name only up to a carriage return. `rename to build.rs\rdocs/notes.md` created `build.rs`; DDF showed the whole line with a control hazard but no build-script signal.

**Fix:**
- A binary line ends the header. Any header, `---`/`+++` or hunk after it is refused.
- These are refused too:
  - `GIT binary patch` with trailing text;
  - `Binary files` without `differ`;
  - a binary patch with no `literal` or `delta` line. The oracle's first run on the fix found this one.
- An unquoted move name with CR, vertical tab or form feed is refused.
- The differential inserts binary lines and control-character names at every position, plus `no_header_after_a_binary_line_or_control_in_a_move_name_is_read`. Both tests fail on `af5c3d1`.
- The refusal for contradictory headers now names its causes (the assessor's L-e).

**Open (unchanged unless noted):**

- F4 (crates.io 0.2.0 not yanked).
- F5: twelve idle connections make `health` and `stop` fail in about 3 ms with exit 6.
- F6: the release-archive status lines.
- F7: the arm64 and launchd checks are not required.
- The differential is not in the fuzz harness.
- **Lows:**
  - empty input exits 0;
  - binary bodies are not decoded;
  - a nameless `diff --git`;
  - a traditional `@@ -0,0` is labelled modified;
  - `Binary files` straight after `diff --git` shows a file Git reads no patch for;
  - `Files a/x and b/x differ` is binary to Git and shown as modified;
  - `rename from dir//x` shows the unsquashed old path;
  - a binary copy or rename keeps the `binary_changed` status, so whether the old path survives shows only in its metadata.
- **Regressions:** none on input Git writes. 85 of 36,000 cases v0.8.1 read correctly are now refused on purpose: repeated or `index` modes, and nameless `---`/`+++` names, forms no Git writes.

## Assessment 2026-11-11 (`v0.8.2`, `fb5ffe1`; independent rescore)

**Score: 70 / 100**, ten below the last three. A fresh assessor scored the released v0.8.2 with the rubric, a read-only checkout of the tag and the released x86_64 Linux binary. It was told not to read this document, `EVIDENCE.md` or the 0.8.x changelog until its scores were written. It found one new high finding and two mediums in patch truth, each reproduced by hand with a real `git apply`, and two mediums outside it. The coordinating session then reproduced F3, F2 and F1 again on the released binary before acting.

| Facet | Weight | 2026-11-09 (v0.8.1) | 2026-11-11 (v0.8.2) |
| --- | ---: | ---: | ---: |
| Correctness and patch-truth integrity | 20 | 14 | 12 |
| Security and trust boundary | 15 | 11 | 10 |
| Supply chain and release integrity | 15 | 14 | 13 |
| Reliability and operability | 15 | 14 | 11 |
| Platform and portability | 10 | 8 | 6 |
| Interfaces and integration contracts | 15 | 13 | 10 |
| Documentation, governance and evidence | 10 | 6 | 8 |
| **Total** | 100 | **80** | **70** |

The assessor is a different one each time, so part of the drop is probably a stricter reading of the same rubric (inferred, not measured): platform (only x86_64 Linux was run), interfaces (the JSON has no resolved mode or type field) and reliability (a mail-format patch refused) lost points for things earlier assessors saw and scored higher.

**Method (measured):** the parser read in full; 135 hand probes; a random differential of 96,000 cases over twelve seeds (one to three sections, 26 section kinds, 42 names, 15 mutation operators), of which 28,173 were accepted by both DDF and Git and 21,742 agreed after a real apply of the tree DDF's reading implies; a systematic sweep of 70 header lines at every position of 14 base patches (10,780 cases). No crash, timeout or invalid JSON in 106,780 cases. Workspace tests: 1,940 passed, 0 failed. Release: signed tag, immutable, checksum and attestation verified.

**The findings:**

- **F3 (high), fixed on `main`.** A `diff --git` line with no extended header is no header to Git, so a binary line straight after it is no patch; but the line's names stay in Git's patch for the next section that brings none. `diff --git a/first b/first`, `Binary files … differ`, then a section for `second` with an `index` line and a hunk: DDF showed a text change to `second`; Git wrote it to `first` and left `second` alone, exit 0. The previous assessment listed "`Binary files` straight after `diff --git` shows a file Git reads no patch for" as a low; it was the first half of this.
- **F2 (medium), fixed on `main`.** `new file mode 100644` followed by `new mode 100755` or `120000`: Git keeps the later mode and creates an executable or a symlink; DDF showed a plain new file. `--outline` and `--rank` did flag the mode; the JSON carried only the two raw lines, and inline output showed no mode at all. The N3 fix in 0.8.2 refused a repeated mode header and an `index` mode, not this pair.
- **F1 (medium), open.** A `GIT binary patch` body that passes DDF's line grammar but does not inflate: Git prints "corrupt binary patch", applies the sections before it, drops that one and every later one, and exits 0. DDF shows all of them as changed. Nothing unreviewed lands; reviewed changes are silently missing. The previous assessment listed "binary bodies are not decoded" as a low.
- **F4 (medium, reliability), open.** A mail-format patch whose commit message has a line beginning `-` or `+` (a bullet) is refused with exit 4; Git applies it.
- **F5 (medium, display), open.** `--layout inline` shows no modes, no symlink type and no rename source.
- **Lows:** empty input exits 0; a non-UTF-8 header line exits 3 ("may be transient") though it is deterministic; `copy from d//e` shows the unsquashed old path; twenty idle connections reset a `health` call; `daemon start` did not return within 180 s once when its output was captured (one observation, cause not investigated); the README inside the tag says "not yet tagged" (F6).

**Fix (F3 and F2):**
- A binary line straight after `diff --git` is refused.
- `new file mode` or `deleted file mode` beside `old mode` or `new mode` is refused, in either order.
- The differential gains the findings, their controls and 115 generated two-section cases (a bare `diff --git` line, one of 23 lines, then each of five sections), and its summary comparison now includes the mode a file is created or deleted with. The existing generated cases were single sections and passed on the v0.8.2 parser; the new tests fail on it.

**Open:** F1, F4 and F5 above; F4 to F7 of the earlier assessments (crates.io 0.2.0, idle connections, archive status lines, arm64 and launchd checks not required); the differential is not in the fuzz harness. 727 of the assessor's generated cases are the known class where Git refuses a section it can name no file for and DDF shows the `b/` name.

**Pattern:** this is the fourth assessment in a row to find a new high in how the parser reads Git's headers, each in a shape the tests of the time did not generate. Refusing one more shape per release does not end that; the conformance table and the fail-closed default planned for 0.9.0 are what would.

