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
