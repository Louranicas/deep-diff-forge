# Changelog

All notable changes to Deep-Diff-Forge are documented here. The format is based
on [Keep a Changelog](https://keepachangelog.com/), and the project follows
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Fixed (expert circle, 2026-10-28)

A seven-seat expert circle reviewed DDF as pioneer coding agents would use it: Claude Code, Codex,
Zen (pi) and four specialist personas. It found these defects, each reproduced with the 0.3.0
binary and each fixed with a test that fails on the old code.

- **A rewritten comment is never "formatting only".** In tree-sitter-rust a plain comment's text
  is no leaf token, so `structural` and `analyze` reported `// SAFETY: the pointer is valid` →
  `// SAFETY: trust me` as `reformat_only: true` with no change. A comment is now one token,
  compared byte for byte. A new fuzz target, `structural_soundness`, checks that
  `reformat_only` is claimed only when the sources differ in whitespace alone (1,077,956 runs,
  0 crashes, measured locally).
- **A path can lower a file's risk but never zero it.** `generated/`, `vendor/`, `dist/` and
  similar paths used to score 0, so a new `src/generated/x.rs` that spawns a process ranked below
  a one-line README edit. The patch author chooses the path. Such files now keep the base weight
  and their new, deleted or binary signal, without the source, size or API bonuses.
- **`highlight` prints line breaks.** It escaped each line feed as the text `\x0a`, so a file
  printed on one line. Every other control character is still escaped.
- **Parse errors quote at most 120 bytes of the offending line** and count the rest. A 2 MB
  line made a 2,000,074-byte error, in a terminal or an agent's context.
- **Every command answers `--help`** (exit 0) from one table that `--help` also prints. Before,
  only `review` did; the others exited 2, and `council --help` exited 4.

### Security (bug hunt, 2026-10-28)

A team of five Sonnet bug hunters covered the whole stack: patch and core, daemon/rpc/MCP/SDKs,
CLI and council, analysis and TUI, CI/scripts/supply chain. Each finding was re-checked by an
independent false-positive checker before a fix.
- **Repository filters never run.** A clean or process filter defined in a repository's own
  configuration and named in its `.gitattributes` ran during working-tree acquisition
  (`--git-working-tree`, `review.change`), so reviewing a repository someone handed you ran their
  code. Every filter the local configuration defines is turned off for each Git call; the user's
  global filters, such as Git LFS, stay on.
- **`review.change` refuses foreign Git directories.** A `.git` file inside an allowed root could
  point at a repository outside it. The top level, the Git directory and the common directory must
  all be under an allowed root, and an object store with alternates is refused.
- **The Git time limit holds** even when a child Git started keeps its pipes open.
- **The daemon refuses params a method does not take**, so a misspelt guard such as
  `require_file` no longer passes an empty review. MCP enforces its schema enums.
- `highlight` keeps line breaks without colour too (the earlier fix covered only colour).

### Added (agent-first core)

The expert circle's Tier 1: an agent learns where to look before it reads anything large, and
never has to paste a patch.

- **Hunk addresses in `review.v0`** (additive): `address` (SHA-256 of `ddf-hunk-v1`, path,
  ranges and body: exactly this hunk of this patch), `body_sha256` (the body alone: it survives a
  move), `section` (the `@@` heading, display only, never re-rendered) and `old_lines` /
  `new_lines`.
- **Budgeted, sealed views.**
  - `review.outline` lists files in review-risk order with their hunks' addresses, ranges,
    headings and counts, and no line text.
  - `review.hunks` returns chosen hunks verbatim, by address or by path.
  - Both keep every answer within `max_bytes`, continue a long hunk at a line (never inside one),
    and page with a cursor bound to the patch and the selection.
  - They are available on the socket, over `rpc`, as the MCP tools `review_outline` and
    `review_hunks` (where `max_bytes` counts both copies of the answer), and on the CLI as
    `--outline` and `--hunks [--address A]... [--path P]... [--max-bytes N] [--cursor C]` for
    piped patches and Git selections alike.
  - Parity tests hold the CLI equal to the daemon page for page.
- **`review.change` and MCP `review_change`: review straight from Git.**
  - Selections are staged, working tree or range. The answer is the outline or hunks of the
    change, sealed over Git's exact bytes, with an `acquisition` member.
  - It reads only repositories whose canonical path and Git top level are under a root the
    operator allowed with `--allow-repo PATH`, on `mcp`, `rpc` and `daemon start`. Without one it
    reads nothing.
  - Tests cover an outside repository, an escaping symlink and a subdirectory of a larger
    repository.
- **Git subprocesses have a time limit** (closes a 2026-10-28 residual risk). One deadline bounds
  every Git call of an acquisition, 120 s for the CLI and 30 s for `review.change`. A Git still
  running is killed and the acquisition fails with `TimedOut` (exit 3) instead of hanging on a
  stalled filesystem or hook.

### Added

- **`diff.rank`**, a daemon method on the socket, `rpc` and MCP: the sealed
  `deep-diff-forge.rank.v0` document that `--stdin-patch --rank --json` prints, with the same
  input guards. A parity test holds it equal to the CLI on every fixture. The Python SDK gains
  `rank_patch`.
- **MCP tools declare an `outputSchema`,** and a test checks every tool's results against it.

### Changed

- **MCP `rank_patch` returns the sealed `rank.v0` object** (through `diff.rank`), and its
  summary keeps the seal. It used to return `diff.plan`'s bare array, which the MCP tools
  specification does not allow as `structuredContent`, and which could not say which bytes it
  ranked. `rank_patch` also takes `require_files` and `require_hunks`.

## [0.3.0] - 2026-10-28

The workspace version is 0.3.0; the `v0.3.0` tag, which publishes the release, is the owner's
decision. 0.3.0 is minor rather than patch because the library APIs changed incompatibly since
0.2.1 (see *Changed*). 0.2.1 was never tagged, so this release follows v0.2.0.

### Added (documentation and community)

- **The README is rewritten from the code.**
  - Three graphics: a terminal session generated from the binary's real output
    (`scripts/docs/render_terminal_svg.py`), an architecture diagram that adapts to light and
    dark themes, and a refreshed banner (14 crates, 1,354 tests).
  - A Mermaid crate graph drawn from `docs/crate-edges.txt`.
  - The risk-ranking weights from `risk.rs`.
  - The nine required checks.
  - The production-readiness verdict and its known limits.
  - A documentation map.
- **An open call for contributors:** `CONTRIBUTING.md` (setup, the quality bar, open problems
  worth owning), `CODE_OF_CONDUCT.md` (Contributor Covenant 2.1), issue forms for bugs and
  features, and a pull request template.
- **Every document under `docs/`, plus `SECURITY.md`, `EVIDENCE.md` and `NOTES.md`,**
  is checked against the code at 0.3.0:
  - target designs say what exists today;
  - ledgers and history carry a dated status block, and no past entry is rewritten;
  - surfaces that are refused today are labelled.

### Fixed (found while checking the docs against the code)

- **`review --cmd` refuses an unknown command name** (exit 2, naming the commands there are).
  Before, a typo fell through to `rank` and exited 0. A `--cols` or `--rows` value that is not a
  number is refused too; before, it silently became the default.
- **`--parallel` refuses anything but `serial`, `auto` or a count from 1** (exit 2). Before, an
  unreadable count ran as `auto`, and `0` was reported as `fixed:0` while one worker ran.
- **`analyze` with the wrong number of paths, or an option, exits 2** (fix the command line).
  Before, a missing path exited 4 (change the input).
- **`deploy release` no longer claims a GitHub release for an untagged version.**
  `github-release` is `blocked` until the owner tags, so `pending` lists it with crates.io. It
  used to report `published` for 0.2.1, which was never tagged.
- **A tag release runs the macOS tests.** `release.yml` calls `ci.yml` with the tag push's
  event, which the macOS job did not accept, so a tagged release would have shipped macOS
  binaries tested only by an earlier dry run.
- **`just release-tag` pushes the tag to `origin`.** It used to push to a remote named
  `github` that no checkout has. Its comment now describes the crates.io hold, not a token.
- `--help` no longer says learned scores set engine defaults; nothing reads them yet.
  `chain-contract` no longer promises a schema version on every JSONL record; each record names
  its event.

### Added

- **Every exit code names what to do next.** The registry gives each code an
  `action` (`proceed`, `fix_invocation`, `check_environment`, `change_input`,
  `review_evidence`, `report_bug`) and `may_be_transient`; `schema exit-codes`,
  `explain --json` and the MCP exit-code tools report both, and the README
  tables carry a checked Next column. A new code cannot be added without one.
- **A release dry run.** `release.yml` runs on `workflow_dispatch`: the quality
  and supply-chain gates and all three builds, with each executable binary
  smoke-tested and every archive's checksum verified, while releasing and
  attesting stay limited to tag refs. Tag releases get the same smoke and
  checksum checks before anything is published.
- **macOS tests.** A `macos` CI job runs the workspace tests on `macos-latest`
  weekly and on manual runs, including a dispatched release dry run.
- **Deployment resource requirements** in `docs/OPERATIONS_AND_DAEMON.md`: the
  measured memory of maximum-size requests and the `/tmp` inode limit.
- **The review JSON marks the line that was decoded lossily** (`"lossy": true`
  on the line), beside the per-file `lossy_lines` count, so a consumer need not
  guess from U+FFFD. Measured with a JSON-only rebuild sweep over 1,127 real
  commits: every other text file rebuilds to its exact blob.

### Added (slice C: SDKs)

- **Rust: the `deep-diff-forge-sdk` crate.** `review` (with `InputGuards`)
  returns a `Review` whose `to_json` is the CLI's review document byte for byte;
  `rank` orders it by risk; `exit` re-exports the registry; `Client` calls a
  running daemon. `Error` maps to the CLI exit codes. The public signatures are
  pinned by a compile-checked test, and a live in-process daemon test proves the
  client and the facade agree.
- **`deep-diff-forge rpc`:** the daemon's JSON-RPC over stdio, with no daemon.
  It uses the socket server's own line handler and request cap, and stops on
  `daemon.shutdown`.
- **Python client** (`sdk/python`, standard library only): `StdioClient` and
  `SocketClient`. Its tests check CLI parity over both transports and run in CI.

### Added (slice D: MCP server)

- `deep-diff-forge mcp`: a read-only, dual-era MCP server over stdio. It
  implements the 2026-07-28 specification (per-request `_meta`,
  `server/discover`, `resultType`, `UnsupportedProtocolVersion` -32022) and keeps
  the `initialize` handshake for 2025-11-25 and earlier clients. Its tools are
  `review_patch` (the daemon's `diff.compute`, the same JSON value as the CLI's
  document, tested over real stdio), `rank_patch`, `explain_exit_code` and
  `exit_code_registry`. All are annotated read-only. Domain refusals are tool
  errors (`isError`); malformed calls are protocol errors. Request lines are
  capped and stdout carries only protocol messages.

### Added (slice B: daemon parity, protocol 1)

- **Daemon `diff.compute`** returns the same `deep-diff-forge.review.v0` document
  as `--stdin-patch --json` for the same bytes, under the same input contracts
  (`require_files`, `require_hunks`). A parity test proves this across fixtures
  and guard combinations. The input-contract predicate moved to
  `deep_diff_forge_patch::InputGuards`, shared by both surfaces.
- **`engine.methods`** lists every method with its params, result and protocol
  version. `engine.initialize` reports the method names and refuses a protocol
  version it does not speak (domain error 2). Domain error 7 is an input-contract
  refusal, matching CLI exit 7.

### Added (slice A: CLI contract v1)

- `deep-diff-forge envelope -- <command…>` wraps any command's result in one
  `ddf.result.v1` JSON object: exit code, registry outcome, stdout (parsed when it
  is one JSON value), stderr, and any truncation. It runs the command as a child
  of the same binary with stdin passed through, leaves the command's own output
  unchanged, and exits with its code.
- `deep-diff-forge schema [result|exit-codes]` prints the envelope's JSON Schema
  (2020-12) or the exit-code registry. `deep-diff-forge explain <code> [--family
  engine|council] [--json]` names one code.
- One exit-code registry, by command family (`engine`, `council`), owned by
  `deep_diff_forge_core::exit` so the daemon and SDK slices can share it. Every
  exit site uses its constants, and a source-scan test forbids numeric exit
  literals. The engine meanings now say what the codes are used for: 3 is any I/O
  or encoding failure, 4 is any invalid input, and 101 is a panic (council sa1:
  the old "patch parse" label misnamed receipt and analysis errors). The README
  documents both tables, and a test fails if they drift from the registry.
- `envelope` keeps the command's exit code when its own reader closes the pipe;
  whitespace-only stdout stays text; the schema lists every emitted field as
  required and enumerates `outcome`.
- `--help` no longer says crates.io is blocked only by a missing token; it is on
  hold until the codebase is production-ready.

### Changed

- **The crates.io hold is a condition, not a missing secret.** The publish job
  runs only when the repository variable `DDF_CRATES_IO` is `open`; before, it
  ran on every tag and skipped only because no token was configured.
- **`rank_patch` is bounded like `review_patch`.** Above 256 KiB it returns a
  `deep-diff-forge.rank-summary.v0` with the highest-ranked files that fit and
  `omitted_files`.
- **Fallback reasons and semantic kinds have written-out wire names**
  (`FallbackReason::wire_name`, `SemanticChangeKind::wire_name`) instead of
  `Debug`; the bytes are unchanged.

- **CI runs on Ubuntu 26** (Luke's decision, a measured trial, 2026-10-27).
  Every CI job and the release workflow's supply-chain and publish jobs run on
  `ubuntu-26.04`. The trial measured rootless Podman 5.7.0 on Ubuntu 26.04.1
  (kernel 7.0.0): the AppArmor user-namespace restriction starts on, as on
  24.04; the existing guarded step lifts it; and all 12 planted-breach probes
  were refused. The Linux release binary is still built on `ubuntu-24.04`, so
  its glibc requirement keeps it running on older distributions.
- **GitHub and GitLab `main` share one history again** (Luke's decision,
  2026-10-27). GitLab `main` carried three commits from 2026-06-23 (review
  cockpit hardening) that GitHub never received; their code had already
  reached GitHub by another path. A merge joins the histories (the code is
  unchanged), so GitLab `main` fast-forwards with no force push. The two
  collaboration logs they added live under `docs/history/2026-06-23-review-cockpit/`.
- **One parse → guard → seal path** (anchor C-05; library API).
  `deep_diff_forge_patch::SealedReview::new` decodes, parses, applies the input
  contracts and seals the same bytes; the CLI's `--stdin-patch`, the daemon's
  `diff.compute` and the SDK's `review` all use it, and Git selections seal
  through `StructuredDiff::sealed_review` from the same capture.
  `to_json_sealed` is no longer public, so a seal can no longer be paired with
  files parsed from other bytes. Every output is byte-identical.
- **The design docs say they are target design** (anchor E-01).
  `SCHEMATICS.md`, `ARCHITECTURE.md` and `MODULE_STRUCTURE_PLAN.md` open with
  a target-design banner, and `SCHEMATICS.md` carries a generated "current
  crates and edges" block rendered from `docs/crate-edges.txt`; a test fails
  when the block is stale (`DDF_BLESS_SCHEMATICS=1` regenerates it).
- **`deploy status` says it ran nothing** (anchor E-06). Its gates are a
  static `not-run` list; the JSON now carries `"gates_run": false` so a caller
  cannot read `not-run` as a verdict, and the human output points at `just ci`.
- **Council fields that take a closed set are enums** (anchor C-02; library
  API). `Request.builder_family` (`BuilderFamily`), a challenge's
  `disposition` (`Disposition`) and `Assessment.state` (`AssessmentState`)
  were strings checked after parsing; an invalid value is now refused when it
  is read. The JSON is unchanged, and a test holds the reviewer-facing
  report schema's enum lists equal to the Rust enums.
- **The council grades only a verified manifest** (anchor C-01; library API).
  `load_manifest` returns a `SealedManifest`, which nothing else can construct,
  and `assess` takes it, so an in-memory `Manifest` that skipped the seal,
  request and artifact-digest checks can no longer be assessed. It reads as
  the `Manifest` it wraps.
- **`StructuralDiff` has one state, not three flags and an `Option`**
  (anchor C-06; library API). `state: StructuralState` is `Unavailable(reason)`,
  `Degraded`, `LineAnchored` or `Minimal { reformat_only }`, so combinations
  such as "reformat-only and unavailable" can no longer be built. The
  `reformat_only()`, `degraded()`, `minimal()` and `unavailable()` accessors
  report what the fields did, and every JSON output is byte-identical.
- **A learning receipt cannot carry a path** (anchor C-07; library API).
  `StrategyReceipt.file_hash` is a `RedactedId`, which only `redacted_id()`
  makes and which deserializing checks (16–64 lowercase hex characters), so a
  path is refused when a receipt is built or read, not only when it is stored.
  On disk the field is the same bare string. A load error now names its store
  line for malformed JSON too.
- **`InputGuards` has no `Default`** (anchor C-04; library API). "No guards"
  is written out as `InputGuards { files: false, hunks: false }`, so
  `..Default::default()` can no longer silently drop a contract a caller asked
  for; the SDK's `Guards` already worked this way. A `compile_fail` doctest
  holds it.
- **The test-count floor is retired** (anchor B-09). The 50-tests-per-crate
  rule was never enforced, and `just test-audit` always exited 0 (and read 0
  without `rg`); the project's own antipattern catalogue lists count literals
  as a false oracle. The Testing Gold Standard and the deployment docs now
  name diff-scoped mutation (0 missed in the changed code) as the coverage
  oracle, and the `test-audit` recipe is removed.
- README exit-code table gains code 7 and the recommended gate command now
  reads `--require-files --require-hunks`. The workspace has 1,192 passing tests
  (1 ignored), measured 2026-10-26 on the stage D tree; the 991 baseline is
  historical.
- The exemption control evolved through councils r10–r18: a skip-attribute
  lexer, then oracle spans (r12), differential MIR compilation (r13), the
  gate's own build (r14), proven test code (r15), five configurations (r16),
  an allow-list (r17) and `#[cfg(test)]` only (r18). Councils r18–r21 then
  hardened the evidence:
  - incremental state is removed before each build, and drift of the
    unmodified tree is inconclusive (r19–r20);
  - path dependencies are observed (r19);
  - a build script's `cfg(test)` is refused from cargo's own report (r19);
  - each unit's dep-info is held to an allow-list and fails closed (r20–r22);
  - the controller requires exactly the gate and release configurations and a
    clean outcome (r19–r20).

  A stop rule bounds further detectors (r21). Each step answered a council
  finding; `docs/REVIEW_COUNCIL.md` keeps the history.
- The skip-attribute lexer (about 580 lines) is retired (r13); the oracle is
  the single control. `code_lines` widening follows nested comments, attribute
  brackets, plain/byte/raw strings and character literals.
- The mutation job carries `CARGO_ENCODED_RUSTFLAGS=--emit=link,mir`, the same
  flag as the oracle's builds, so the two are indistinguishable by flags.
- CI runs every job on `ubuntu-24.04`, where the rootless-Podman step was
  measured; `ubuntu-latest` moves to Ubuntu 26, which will be its own measured
  change. `actions/checkout` (v4 → v7.0.1) and `softprops/action-gh-release`
  (v2 → v3.0.3) move to their Node 24 releases, pinned by SHA.
- Structural diff trims the common token prefix and suffix before the LCS and
  anchors oversize diffs on unchanged lines.
- CI: every toolchain pinned (stable 1.99.1, `nightly-2026-10-25`); the
  evidence job's mutation timeout raised to 300 s; the real-Podman sandbox
  suite runs on GitHub-hosted runners with canary (non-secret) credential
  files and prints job diagnostics on failure; pedantic clippy covers the
  no-default-features build and the fuzz crate; gitleaks 8.30.1 (checksum
  pinned) scans full history. `.gitignore` covers `__pycache__/`.

### Fixed

- **Patch parsing is linear in metadata lines.** Every non-body line rescanned
  all of the file's metadata, so 1M `index` lines (8 MB) took 139 s and held the
  daemon's engine past every client deadline; it now takes 0.20 s.
- **A refused learning receipt exits 4** (change the input), not 3 (a
  retryable I/O failure), as the README documents.
- **A second `daemon start` never replaces a live daemon's socket.** It
  exits 6 instead of orphaning the running daemon.
- **`daemon --socket PATH health` works.** Commands take their positionals
  from argument admission, so an option's value is never read as the
  subcommand. `--foreground` belongs to `daemon start` alone (which always
  serves in the foreground) and is refused elsewhere.
- **The daemon never sends a response over its 80 MiB line cap**; it answers
  `RESOURCE_EXHAUSTED` (8) for the same id instead of a line the client
  refuses. The README now says one engine serves the eight connections.
- **The daemon client reads a complete response on macOS** when the daemon
  closes first, and gates never fall back to running as root where `/proc` is
  missing (found by the first macOS test runs).
- **No trust policy trusts a strategy with no receipts**, even with
  `min_samples: 0`.
- **The Python `SocketClient` raises `TransportError`** for a missing socket, a
  refused connection or a timeout, as documented, not a bare `OSError`.
- **The README's daemon section** lists all ten methods and protocol 1, under a
  test.

- **Antipattern rows record what enforces them** (anchor B-10). A catalogue
  row may carry a DDF-owned `ddf_enforcement` claim beside its verbatim import
  labels; the policy checker accepts it only when every piece of evidence
  resolves against parsed configuration (a workspace lint at `deny`, a path
  in `clippy.toml`'s `disallowed-methods`), and its report lists the enforced
  rows. AP-17 (unreviewed lint suppression, B-03) and AP-06 (policy coupled to
  clock or host, B-04, with its scope stated) are the first two.
- **Every pull request is mutation-tested on its changed lines** (anchor
  B-01). A new four-shard `diff mutation` job runs `scripts/diff_mutation.sh`,
  which mutates the Rust lines the pull request changes since its merge base
  (`cargo mutants --in-diff`) and fails on any missed mutant or timeout. A
  change with no Rust lines under `crates/` passes and says so. Before, CI
  mutated two functions of the review crate, and "0 missed on every diff" was
  held by habit.
- **The local CI replica and pre-push hook are versioned** (anchor B-07).
  `scripts/ci_local.sh` replicates every step of the CI gate jobs and fails
  closed; each step is labelled with the CI step it replicates, and
  `tests/ci_replica.rs` fails when a CI step has no replica step or a replica
  step names no CI step. `just ci` runs it (before, `just ci` was the feature
  gate alone, without docs, deny, audit, SBOM, MSRV, soak, probes, the SDK test
  or the fuzz compile). The gitleaks pre-push hook is `scripts/hooks/pre-push`;
  `just install-hooks` installs it and `just doctor` reports whether it is
  installed.
- **CI runs weekly** (anchor B-11). A `schedule:` trigger (Monday 04:23 UTC)
  runs the full workflow, so a new RustSec advisory or a yanked dependency
  fails within a week even when nothing is merged.
- **The crate graph is a committed table** (anchor B-05).
  `docs/crate-edges.txt` lists every edge between workspace crates (normal,
  dev and target-specific kept apart), and `tests/crate_edges.rs` compares it
  with `cargo metadata`: a new dependency between workspace crates fails until
  the table is changed in a reviewed diff, and a row no `Cargo.toml` declares
  fails as stale.
- **Effects go through named doors** (anchor B-04). A new `clippy.toml`
  disallows `Command::new`, `env::var`/`var_os`, `Instant::now`,
  `SystemTime::now` and `thread::sleep`. The 21 functions that legitimately
  spawn, read the environment or read a clock each carry
  `#[expect(clippy::disallowed_methods, reason = "<door>")]` naming the door,
  and the 20 integration-test files that drive real processes opt in once at
  the top. A new effect anywhere else no longer passes clippy.
- **MCP `review_patch` returns a bounded summary for large reviews** (anchor
  A-07; Luke's decision). The full document is returned while it is at most
  256 KiB; a larger one comes back as `deep-diff-forge.review-summary.v0` (the
  same seal and totals, per-file counts for up to 1,000 files, `reason`,
  `omitted_files`). `paths` returns chosen files in full and `detail: "summary"`
  asks for the summary. A 30 MB patch now answers in 181 KB instead of one
  178 MB line.
- **Review council** (`council prepare|status|review|gate`) — six default
  reviewer slots across Claude Code and Codex, independent then challenge
  rounds, controller admission that rejects fabricated citations, and `HOLD`
  (exit 7) for missing evidence. See `docs/REVIEW_COUNCIL.md`.
- **Gate sandbox** — the eight evidence gates (format, check, clippy, pedantic,
  tests, antipatterns, fuzz, mutation) run on a sealed snapshot in a rootless
  Podman container: image by ID, `--network=none` except dependency vendoring,
  read-only root and source, all capabilities dropped, `--userns=keep-id`. A
  real-Podman planted-breach suite (12 probes) guards it.
- **Diff-scoped mutation gate** — cargo-mutants 27.1.0 `--no-config --in-diff`
  against a scope diff built from verified base and candidate blobs
  (`mutation_scope`, `mutation_base`), 300 s per mutant, 4 jobs.
- **Exemption oracle** (`policy/exemption_oracle.py`, schema v7) — run before
  the mutation job; only `#[cfg(test)]` may hide code from mutation, and that
  code must leave every workspace member's non-test MIR unchanged in the gate
  and release builds. Anything else hidden is refused with no builds.
- **Git acquisition** (P1a) — `--git-working-tree [--include-untracked]`,
  `--git-staged`, `--git-range A..B`, `--repo`, `--acquisition-json`
  (`deep-diff-forge.git-acquisition.v0`).
- **`council import-tuicr`** decision journal (P1c, DDF-R18) — appends tuicr
  dispositions to an append-only `<run>/decisions.jsonl`; dry run by default.

- **`council export-tuicr`** (DDF-R54) — files controller-admitted council
  findings into a local tuicr 1.3 session as local draft notes. Dry run by
  default; `--execute` writes through `tuicr review add` with fixed argv.
  Refuses tampered reports, forge sessions, a different repository and, unless
  `--allow-review-level` is given, findings whose cited bytes in the session's
  revision differ from the run's sealed snapshot. A create-new ledger in the
  run makes re-runs no-ops. See `docs/REVIEW_COUNCIL.md#tuicr-export`.
- **Input sealing** — every input-derived machine document (`review.v0`,
  `rank.v0`, `cluster.v0`, `semantic.v0`) now carries two additive top-level
  members directly after `schema`: `input_sha256` (lowercase hex SHA-256 over
  the exact bytes read, before any parsing or normalisation) and `tool`
  (`{"name": "deep-diff-forge", "version": …}`). An orchestrator can tie a
  verdict to exact input bytes and an exact tool build. Schema names are
  unchanged (`.v0`): the fields are purely additive. SHA-256 is implemented in
  `deep-diff-forge-core` (`sha256`, `sha256_hex`, `InputSeal`) in pure safe Rust
  with no new dependency, tested against the NIST vectors;
  `deep-diff-forge-patch` gains `to_json_sealed`.
- **`--require-files`** for `--stdin-patch` — refuse a patch that parses to 0
  files with the new **exit code 7**, an empty stdout, and
  `refused: 0 files in input (--require-files)` on stderr. Gates should always
  pass it so an empty or mis-piped input is never read as a clean review.
  Default behaviour without the flag (0 files → exit 0) is unchanged.
- **`--require-hunks`** for `--stdin-patch` — closes the gap `--require-files`
  leaves open: a header-only (`diff --git a/x b/x`) or rename-only diff parses
  to one file with 0 hunks and passed the files guard with exit 0. The new
  guard refuses a patch with 0 hunks or 0 added+removed lines across all files
  (header-only, rename-only, context-only hunks) with the same **exit code 7**,
  an empty stdout, and `refused: 0 hunks in input (--require-hunks)` on stderr.
  A gate should pass **both** flags; when both would fail, the files guard
  reports first. Default behaviour without the flag is unchanged.
- Determinism contract test: `--rank --json --require-files` is byte-identical
  across runs with an identical `input_sha256`.
- **Stdio server fuzz targets** — `stdio_mcp` and `stdio_rpc` compile the
  CLI's `mcp.rs` and `rpc.rs` into the fuzz crate and drive their stdio
  loops in process, at the real cap or a small one. They assert one JSON-RPC
  2.0 object per output line, exactly one of `result`/`error`, and at most one
  answer per input line. CI runs a 10 s instrumented smoke from
  `fuzz/seeds/`.

- **`learn` is tested end to end** (anchor A-08, from Parking). Recording a
  receipt, reading the count back, the default `status` subcommand, and the
  refusals (a path in place of the redacted id, a self-contradicting receipt,
  an unknown subcommand, `record` without `--stdin`) now run against the
  binary in an isolated `XDG_STATE_HOME`. Before, replacing `learn_cmd` with a
  no-op survived mutation.
- **A self-contradicting learning receipt is refused** (anchor C-11). A
  receipt whose outcome is `revisited` but whose `revisited` flag is false is
  refused on append and on load, so scoring cannot count the revisit in one
  rate and not the other.
- **Contradicted facts corrected** (anchor E-02). `--help` no longer lists
  shipped commands (`review`, `daemon start`, the `--git-*` family) as future
  modes; its "not yet implemented" list is checked to be refused. The
  crates.io publish loop in `release.yml` now includes `sdk`, and a test keeps
  it naming every workspace crate after its dependencies. The API doc states
  protocol 1, the release doc describes per-archive SHA-256 files and
  attestations (no signed `checksums.txt`), and the feature matrix marks the
  options that do not exist.
- **An unmeasured test no longer shows as passed** (anchor B-08). The
  real-Podman sandbox test and the two real-tuicr tests printed `UNMEASURED`
  and returned, so `cargo test` reported them `ok`. They are now `#[ignore]`d
  with the command that runs them, and an explicit `--ignored` run without
  their environment fails; CI runs the sandbox test with `--ignored`.
- **Lint suppressions carry a reason and cannot go stale** (anchor B-03). The
  nine workspace `#[allow]` sites (and two in the fuzz targets) are now
  `#[expect(.., reason = "..")]`, and clippy's `allow_attributes` and
  `allow_attributes_without_reason` are denied in the workspace and fuzz
  manifests: a bare `#[allow]` no longer builds, and an expectation whose lint
  stops firing fails as unfulfilled.
- **Code-literal scans cover every source file** (anchor B-02). The exit-code
  scan read a hard-coded list of five files and stopped at the first
  `#[cfg(test)]`, which hid the production code after a mid-file test item
  (`core/src/lib.rs` from line 128, `daemon/src/serve.rs` from 181). It now
  walks every `crates/*/src` file, skips each test item on its own, and also
  catches bare `exit(N)`, `ExitCode::from(N)` and `code: N`. A new scan refuses
  JSON-RPC error-code literals outside their named constants; the MCP server's
  six `-32600` and the SDK's daemon codes `4` and `7` now use the constants.
- **One status vocabulary** (anchor D-01). The JSON, JSONL and projection label
  tables are replaced by `FileStatus::label`, and the human summary no longer
  prints `{:?}` (it printed `binarychanged` and `typechanged` where every other
  surface prints `binary_changed` and `type_changed`).
- **Syntax analysis no longer depends on machine load** (anchor A-04). The
  flat 250 ms parse deadline made one 220 KB Rust file come out complete in 9
  and timed out in 55 of 64 parallel runs, and timed out every 1 MB file even
  on an idle machine. The deadline is now a backstop of 1 s plus 5 ms per KiB,
  about seventeen times the measured linear parse rate at every size: under a
  load average of 33–39, 64 parallel runs of 220 KB, 1 MB and 1.9 MB inputs
  gave one result each. Hostile input is still cut (a 455 KiB token soup that
  ran for 125 s now stops at about 3 s). Counting tree-sitter progress
  callbacks was measured and rejected: that input made fewer callbacks per KiB
  than ordinary code.
- **The daemon recovers fully from a contained panic** (anchor A-03). It used
  to keep answering through the poisoned engine lock while each connection loop
  read the poison as "not running", so after one panic every connection got a
  single answer and was dropped. The sessions the panic may have left
  inconsistent are now cleared (`Engine::reset_sessions`; the pid, start time
  and session-id counter are kept) and the poison is cleared, so the
  connection and later ones are served as before.
- **Every fuzz target runs in CI** (anchor B-06). `patch_parser`,
  `review_json`, `daemon_protocol` and `agent_annotation` only ever compiled;
  the instrumented smoke now runs all seven targets for 10 s each and refuses a
  `fuzz/Cargo.toml` target that the step does not run. New seeds for
  `daemon_protocol` and `agent_annotation`; Dependabot now watches `/fuzz`.
- **Unknown options are refused, not ignored** (anchor A-01). Every entry point
  admits its arguments against a closed set (`cli/src/args.rs`) and exits 2,
  naming the option, with an empty stdout. Before, a misspelt guard such as
  `--require-file` was silently skipped, so a gate passed every empty patch
  with exit 0. Commands that take no arguments (`doctor`, `--version`, the
  `*-contract` printers) and `schema` now refuse extra ones too. A test ties
  every option in `--help` to an invocation that admits it.
- **A non-UTF-8 line no longer refuses the whole review** (anchor C-12).
  Real repositories commit non-UTF-8 content (Latin-1 sources, fuzz corpora):
  2 of 33 HEE v5 commits, and DDF's own merge #11 (binary fuzz seeds), failed
  with exit 3. Now `patch::decode` keeps valid lines byte-exact and decodes an
  invalid line lossily (U+FFFD); the parser marks only that line's file
  (`PatchTwin::lossy_lines`, shown as `"lossy_lines": N` in the review JSON and
  JSONL, and in the human summary), and every seal covers the exact input
  bytes (`seal_of` takes bytes; `CapturedPatch::capture_bytes`,
  `original_bytes`, `is_exact`). Output for UTF-8 input is byte-identical. A
  non-UTF-8 *path* is still refused, because a path is an identity.
  `GitAcquireError::NonUtf8Patch` is removed.
- **The review JSON records which side lacks a final newline** (anchor A-06).
  A line carries `"no_newline": true` when the marker follows it; its `kind`
  names the side (removed: old, added: new, context: both). Before, patches
  that differed only in which side lacked the newline produced identical JSON,
  although git applies one and refuses the other. Other output is unchanged.
- **The learning store validates records on load** (anchor A-05). A hand-edited
  line holding a path (or a path-like token) is refused, naming its line; before,
  only `append_receipt` applied the privacy checks.
- Non-Unix `path_from_bytes` (`deep-diff-forge-patch`) reuses valid UTF-8
  without a copy, which clears `clippy::needless_pass_by_value` in a
  Windows-target pedantic build.
- **main's red CI** (failing since 2026-06, run 37532439196): a floating
  `stable` toolchain deprecated `fetch_update` under `-D warnings`; `cargo deny`
  was called without being installed; the SBOM `documentNamespace` embedded
  wall-clock time. The namespace is now content-addressed and `created` comes
  from `SOURCE_DATE_EPOCH` or the last `Cargo.lock` commit, so regenerations are
  byte-identical. main CI is green at run 40274253094.
- **Sandbox worked only for uid 1000**: gate containers ran as the image's
  `USER 1000`, so an operator with another uid (1001 on GitHub runners) could
  not read the private source copy. Containers now run as
  `--user=<uid>:<gid>` read from `/proc/self`.
- **Probe verdict race**: parallel probe tests could write a verdict mid-line
  after libtest's `test name ... `, losing it (main run 40268250417). Verdicts
  now start with a newline and are parsed anywhere in a line.
- **`mutation_scope` emitted touching hunks** (latent since r10): a widened
  pure insertion could start on the line the previous hunk ended on, and
  cargo-mutants rejected the diff. Regions now merge on touching; property
  tests assert header ordering. Surfaced by council r17.
- The oracle removes its build trees on exit (they filled `/tmp`).
- `council_report` fuzz target builds again after the `mutation_diff_sha256`
  field.

### Security

- **No text change can hide behind a binary patch body.** After
  `GIT binary patch`, every later line was taken as binary data, so a crafted
  patch was reviewed as one binary file while `git apply` also changed a text
  file. The body now takes only the lines of git's binary grammar; such a patch
  is refused (exit 4).
- **Ids become paths only as checked segments** (anchor C-09). Every reviewer,
  job, session and fuzz-target id joined into a council run directory now goes
  through one check (`segment`, `job_dir`, `report_path`) at the join, so a
  path cannot leave the run directory even where the id was not validated
  first; before, the joins relied on `identifier()` having been called
  earlier.
- **MCP marks patch text as untrusted for the consuming model** (anchor E-07).
  The server instructions and the `review_patch` / `rank_patch` descriptions
  say that paths and line text come from the patch, whose authors wrote them,
  and are data to review, never instructions. Reviewed code and comments are
  an injection channel into a model's context; nothing said so before.
- **One hardened Git door** (anchor A-02). Git acquisition (`patch`) and
  council storage (`review`) now apply the same set from
  `deep_diff_forge_core::git`: no optional locks, replacement refs, fsmonitor,
  hooks, external diff or transports (`protocol.allow=never`,
  `GIT_NO_LAZY_FETCH`), and every inherited variable that would point Git at
  another repository removed. Measured before: `--git-range` in a partial clone
  ran `git fetch origin` against the promisor remote (a network effect from a
  read-only command); council storage honoured an inherited `GIT_DIR`, so
  `rev-parse HEAD` named another repository while the top-level check passed.
  A partial clone's missing blob is now an error, not a fetch.
- Council gates never fall back to the host: a missing gate image or a failed
  dependency-vendoring job is a refusal. Vendored dependencies are sealed and
  re-measured before each gate; base Git objects are verified on read.
- The mutation gate ignores the candidate's cargo-mutants config, and code
  hidden from mutation by anything other than `#[cfg(test)]` is refused.

## [0.2.1] - 2026-09-04

### Security

- Hardened the Unix daemon against symlink/inode-swap attacks, slowloris and
  stalled-reader clients, unbounded worker creation, oversized responses, and
  multi-session memory exhaustion. JSON-RPC fields are now shape-validated.
- Added a structural patch line budget and strict size/line/count, ownership,
  permission, and pseudonymous-identifier validation to the learning store.
- Upgraded Ratatui/Crossterm so the dependency graph uses patched `lru` 0.18.4
  and removed all RustSec advisory waivers.
- Restricted release publication to version-matching tags, stopped persisting
  checkout credentials, pinned the audit tool, and made the publish version
  derive from the workspace manifest. The separate fuzz dependency graph now
  passes the same advisory, license, source, and wildcard-dependency policy;
  CI/release concurrency and dependency-update cooldowns are explicit.

[0.2.1]: https://github.com/Louranicas/deep-diff-forge/releases/tag/v0.2.1

## [0.2.0] - 2026-06-23

The **L9 (Learning)** layer plus the first crates.io-publishable cut, a
supply-chain & fuzz hardening pass, and the interactive **review cockpit** (ranked
tree, multi-file diff, inline agent notes, command palette, colour themes, mouse)
— 12 crates, 927 tests, zero `unsafe`, supply-chain-gated. `cargo publish
--dry-run` is clean across the workspace; the crates.io upload itself remains
token-gated.

### Added

- **Interactive review cockpit** — `review [--probe] [--side] [--palette | --cmd
  NAME]` (+ `deep-diff-forge-tui`, ratatui): a review-first terminal UI with a
  risk-ranked file tree, a multi-file diff pane, and the engine's own findings as
  inline grounded agent notes. Runtime view toggles for layout (inline ↔
  side-by-side), fold, line-wrap, and notes; a per-file **reviewed flag** (`v` /
  `Space`, auto-advancing, with a `viewed:n/total` progress indicator) for triage;
  three colour themes (`T`: dark · midnight · mono); a top menu bar and a `?`
  keybinding help card; and full **mouse** support (scroll, click-to-focus,
  sidebar selection, menu clicks). State is pure and headless-testable
  (`render_to_lines`); `--probe` renders one frame with no TTY for CI/agents.
- **Command palette** — `:` in-app, or headless via `review --palette` / `--cmd
  NAME`, runs any engine capability against the loaded review and shows it in a
  result panel: `rank`, `outline` (tree-sitter symbols of the changed regions),
  `cluster`, `summary`, `notes`, `review` (the `review.v0` JSON), live `daemon`
  health/status, `learning` scores, and the `maturity` ladder. In-process and
  read-only; never mutates patch truth.
- **`deploy release`** — `deploy release [--json]` (schema
  `deep-diff-forge.release.v0`): a declared per-target publication snapshot
  (GitHub, GitLab, GitHub release, crates.io) with a `pending` list; `crates.io`
  reports `blocked` until a registry token is configured.
- **Syntax highlighting** — `highlight <path> [--color|--no-color]` (+
  `syntax::highlight`): tree-sitter highlighting reusing the grammar's own
  `highlights.scm`, mapped to an ANSI palette. Colour auto-detects a TTY.
  Terminal-injection-safe: source text is routed through `display_safe`, so the
  only raw escapes are fixed SGR codes.
- **Structural diff** — `structural <old> <new> [--json]` (+
  `syntax::structural`, schema `deep-diff-forge.structural.v0`): a token/leaf-level
  (AST) diff over tree-sitter token streams. **Reformat-aware** (layout-only edits
  report zero structural change) with best-effort moved-block detection; never
  touches patch truth. Bounded LCS with graceful degradation on huge inputs.
- **Learning (L9)** — `deep-diff-forge-learning`: the local-only learning loop.
  Per-decision [`StrategyReceipt`]s (hashes/counts/timings — never a path or
  source line) are appended as JSONL under `$XDG_STATE_HOME/deep-diff-forge/
  learning/`, scored per strategy, and gated for promotion. Three learning units
  — planner (strategy + budget), ranking (risk weights), annotation (agent trust
  tiers, *untrusted until grounded*) — plus an explainable promotion gate. CLI
  `learn {status|record --stdin} [--json]`. 135 tests; never uploads, never
  mutates patch truth.

### Changed

- **Daemon socket handling** (Zen review) — replaced scattered `Option<PathBuf>`
  resolution with a parse-don't-validate `SocketLocation` type: one
  `SocketLocation::resolve(explicit)` entry point (explicit `--socket` → else
  `$XDG_RUNTIME_DIR` → else a typed `NoRuntimeDir`, no `/tmp` fallback), with
  `bind`/`connect`/`path` as methods. A "no location" state is unrepresentable
  past construction, and the CLI's nested `if let` collapses to one call.
- **Publishable-workspace manifests** — versions and internal deps are now
  inherited via `[workspace.package]` + `[workspace.dependencies]`; every crate
  carries `description`, `keywords`, `categories`, and `homepage`. `core` and
  `cli` gained the `description` that `cargo publish` requires.

### Fixed

- **Patch truth: malformed/truncated hunks are now rejected** (Zen review) — the
  parser enforces each hunk's declared `@@ -a,b +c,d @@` counts exactly. A hunk
  that ends (at a new header, a new file, or EOF) before its counts are satisfied
  is a `TruncatedHunk` error; a body line that over-fills a side is a
  `HunkLineCountMismatch`. An internally-incoherent patch no longer parses as
  apply-able truth.
- **Broken-pipe handling** — bulk CLI output (`--stdin-patch`, `--rank`,
  `--cluster`, `--jsonl`, `semantic`, `review --probe`, `learn`) is routed
  through a broken-pipe-tolerant writer, so `deep-diff-forge … | head` exits `0`
  instead of panicking with `EPIPE` (exit 101). Resolves the L1-era open finding;
  std-only, no `unsafe`.

### Security

Pre-publication adversarial hardening (S1008412): an 8-dimension STRIDE review
with independent verification produced a CVSS-scored register of 17 confirmed
findings (4 MEDIUM, 12 LOW, 1 INFO; **no Critical/High**), all remediated with
fail-before/pass-after regression tests. A later final-hardening review fleet
(5 reviewers + adversarial verify) added the items below. See `SECURITY.md`.

- **Trojan Source defence (CVE-2021-42574)** — `display_safe` now also escapes
  bidirectional/invisible Unicode (`U+202A–202E`, `U+2066–2069`, directional
  marks, zero-width, BOM) to a visible `\u{XXXX}`, so attacker source cannot
  display differently than it reads — on-mission for a review tool.
- **Cross-platform contract made honest** — the UDS daemon is Unix-only; it now
  emits a clear `compile_error!` on non-Unix instead of a cryptic `std::os::unix`
  failure, and the release matrix dropped its (un-buildable) Windows lane so CI
  and code agree before the irreversible publish (Linux + macOS).
- **Broken-pipe fix completed** — the remaining `deploy`/`doctor`/`--help`/
  contract commands (notably the `--json` outputs CI/agents pipe to `head`/`jq`)
  now route through the broken-pipe-tolerant writer; `… | head` exits `0`, not 101.
- **`structural` on a non-Rust file** now errors explicitly instead of silently
  reporting "formatting only"; **`doctor`** resolves both its runtime-dir and
  socket lines through the daemon's `var_os` resolver (consistent on non-UTF-8 env).
- **Terminal/ANSI-escape injection** — attacker-controlled diff bodies, file
  paths, and symbol names are now sanitized by `core::display_safe` (control
  chars escaped to a visible `\xHH`) at every human-render boundary; `json_escape`
  also neutralises `DEL` + the C1 block. A reviewer rendering a hostile patch can
  no longer have their terminal hijacked (screen clobber, scrollback forgery,
  OSC-52 clipboard write, title/hyperlink spoof).
- **Memory-exhaustion DoS** — stdin, source-file, and daemon request reads are
  hard-capped at the byte budget instead of buffering unbounded input.
- **Daemon robustness** — per-connection panic/error isolation (`catch_unwind`,
  no fatal `?`), a read timeout (slowloris), a size-bounded request line, symlink
  rejection, and **no world-writable `/tmp` socket fallback** (fails closed
  without `$XDG_RUNTIME_DIR`).
- **Fail-closed trust** — agent annotation `source` is no longer inferred from an
  attacker-controlled label; learning-store files are owner-private (`0700`/`0600`).
- **Supply chain** — `unsafe_code = "forbid"` is now compiler-enforced
  workspace-wide; `tree-sitter` is pinned to exact versions; a strict `cargo
  audit` gate runs in CI and gates the irreversible crates.io publish;
  least-privilege workflow permissions; Dependabot; `SECURITY.md`.

A second bias-controlled re-review (S1008443) raised three further findings, each
fixed and independently verified (security / tester / claim-verifier judges,
outside the build loop):

- **Mandatory hunk-header closer** — `parse_hunk_header` now requires the closing
  `@@` token; a header with trailing garbage and no closer (`@@ -1,1 +1,1
  NOT_A_HUNK`) is rejected (exit 4) instead of normalizing into a valid model.
  Completes the patch-truth hardening alongside the truncated-hunk fix.
- **Explicit `--socket` no longer mutates caller state** — `SocketLocation` carries
  provenance; an explicit `--socket` path binds via a fail-closed `bind_explicit`
  that **creates** an absent parent at `0700` but **never chmods a pre-existing
  parent** (a world-readable parent fails closed rather than being silently
  tightened to `0700`) and removes the socket path **only if it is verifiably a
  socket** (never a regular file, directory, or symlink). The engine-owned
  `$XDG_RUNTIME_DIR` path is unchanged, and `daemon start --socket /new/path` still
  works (the absent parent is created).
- **Bounded daemon sessions** — review sessions are capped at `MAX_SESSIONS` (64)
  with LRU eviction (least-recently-used evicted before insert; reads refresh
  recency), hard-bounding daemon memory regardless of how many sessions a client
  opens without closing.

A supply-chain & fuzz hardening pass then raised the release/CI posture:

- **GitHub Actions pinned to commit SHAs** across CI and release (tag-hijack
  defence); the release workflow now emits **SLSA build-provenance attestations**
  for the binary, its checksum, and the SBOM.
- **SPDX SBOM** (`sbom.spdx.json`) generated from `Cargo.lock`/`cargo metadata`,
  gated in CI and uploaded with each release.
- **Fuzz harness** (`fuzz/`, cargo-fuzz) — four targets (`patch_parser`,
  `review_json`, `daemon_protocol`, `agent_annotation`), excluded from the
  publishable workspace; a compile gate keeps it building in CI.
- **CI security probes** — a hostile-daemon soak, a learning-privacy probe, and a
  mutation-gate inventory, exposed as `just security-*` recipes.
- Added a JSON-RPC wire error-code regression test and empty-input
  `--json`/`--jsonl` stability tests. Corrected the daemon's `lib.rs` doc from
  "thread-per-connection" to the true single-threaded sequential accept loop.

[0.2.0]: https://github.com/Louranicas/deep-diff-forge/releases/tag/v0.2.0

## [0.1.0] - 2026-06-22

First tagged release. The complete diff/review engine (layers L1–L7,
Patch→Daemon) at deployment maturity **L8 (Release)** — 11 crates, 568 tests,
zero `unsafe`, supply-chain-gated.

### Added

- **Patch (L1)** — `deep-diff-forge-patch`: unified/Git patch parser, apply-able
  renderer, and `deep-diff-forge.review.v0` JSON. CLI `--stdin-patch [--json]`.
- **Projection (L2)** — `deep-diff-forge-projection`: renderer-neutral inline and
  side-by-side views. CLI `--layout inline|side-by-side`.
- **Pipeline (L3)** — `deep-diff-forge-pipeline`: composable Unix-filter stages
  (`ChainStage`) and JSONL streaming. CLI `--jsonl`.
- **Semantic (L4)** — `deep-diff-forge-syntax`: tree-sitter language detection,
  budgeted parsing, and symbol extraction (Rust). CLI `semantic <path> [--json]`.
- **Review (L5)** — `deep-diff-forge-graph` (Review Intelligence Graph,
  deterministic risk ranking; CLI `--rank`), `deep-diff-forge-agent` (annotation
  provenance, grounding, sanitization), and `deep-diff-forge-tui` (review TUI;
  CLI `review [--probe]`).
- **Cluster (L6)** — `deep-diff-forge-cluster`: bounded parallel dimensional
  execution with deterministic joins and receipts. CLI `--cluster [--parallel]`.
- **Daemon (L7)** — `deep-diff-forge-daemon`: optional UDS JSON-RPC service
  (std-first, owner-private sockets). CLI `daemon {path|start|health|status|stop}`.
- **Deployment surface** — `deploy status` and `deploy release` machine-readable
  reports (`deployment-status.v0`, `release.v0`); CI workflow; `deny.toml`
  supply-chain policy; release workflow (`release.yml`).

### Engineering

- Strictly acyclic 11-crate workspace; `core` is pure vocabulary.
- Patch truth is preserved across every layer (never mutated by enrichment).
- Each production crate carries at least 50 meaningful tests.
- Pedantic clippy clean; no production `unwrap`/`expect`; no `unsafe`.

### Not yet released

- **crates.io publication** is pending a registry token (the release workflow
  publishes automatically when `CARGO_REGISTRY_TOKEN` is configured).

[0.2.0]: https://github.com/Louranicas/deep-diff-forge/releases/tag/v0.2.0
[0.1.0]: https://github.com/Louranicas/deep-diff-forge/releases/tag/v0.1.0
