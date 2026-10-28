# Testing Gold Standard

Deep-Diff-Forge treats tests as executable design evidence. A module is not
deployment-ready because it has many tests; it is deployment-ready when its
tests prove real behavior, failure modes, invariants, and integration contracts.

This standard is intentionally strict. Production modules must meet it before
they can be marked deployable.

## Status 2026-10-28 (0.3.0)

The sentence "the current implementation has only smoke behavior" (bootstrap
era) no longer holds. How the standard is enforced today:

- **Every pull request** runs the CI jobs in `.github/workflows/ci.yml`:
  `cargo test --workspace --locked`, fmt, clippy and pedantic with
  `-D warnings`, rustdoc `-D warnings`, contract probes, a 10 s hostile daemon
  soak, privacy probes and the Python SDK against the built binary
  (`deploy gate-feature`); the declared-MSRV check on Rust 1.88.0; and
  supply-chain checks.
- **Diff-scoped mutation** is a required check: `diff-mutation`, four shards,
  runs [`scripts/diff_mutation.sh`](../scripts/diff_mutation.sh), which mutates
  only the Rust lines under `crates/` changed since the merge base
  (`cargo mutants --no-config --in-diff`, 300 s timeout) and fails on any missed
  mutant or timeout. `--no-config` ignores a candidate's own mutants config.
- **Fuzz smoke:** each of the eight targets in `fuzz/Cargo.toml`
  (`patch_parser`, `review_json`, `daemon_protocol`, `agent_annotation`,
  `council_report`, `stdio_mcp`, `stdio_rpc`, `structural_soundness`; `stdio_invariants.rs` is a shared
  module) runs for 10 s per pull request; CI fails if a declared target is not
  in the run list. Crash inputs become fixtures under
  `fixtures/patch/regressions/`.
- **Sandbox refusal controls:** the rootless-Podman planted-breach suite
  (`crates/deep-diff-forge-cli/tests/council_sandbox.rs`, 12 probes) runs on
  every pull request.
- **Locally:** `just ci` ([`scripts/ci_local.sh`](../scripts/ci_local.sh))
  replays the `secrets`, `msrv`, `gate` and `supply-chain` jobs step for step.
- **Gaps:** macOS tests run weekly and on manual runs, not on pull requests.
  The TUI's interactive terminal path has no test harness; unit tests cover its
  state, tree and rendering, and the headless `review --probe` frame is tested
  end to end. `deploy gate` (below) is not implemented.

## Non-Negotiables

- Every change to production code must leave no surviving mutant in the code it
  touches (diff-scoped `cargo mutants --in-diff`, 0 missed and 0 timeouts), and a
  fix comes with a test shown to fail on the old code. Mutation, not a test count,
  is the coverage oracle (anchor B-09).
- Every crate must include integration tests when it exposes behavior across a
  crate, CLI, filesystem, socket, corpus, or process boundary.
- Tests must align with documented behavior and code invariants.
- Tests must not be written merely to satisfy implementation details.
- Tests must not assert on accidental formatting unless formatting is the
  behavior under test.
- Test failures must lead to a behavior decision: fix code, fix spec, or mark
  the test invalid with evidence.
- Never engage in test fitting.

## Meaningful Test Definition

A test is meaningful when it proves at least one of these:

- a documented contract
- a public API invariant
- a parser or renderer round trip
- an error or fallback boundary
- a security or permission boundary
- a resource budget boundary
- a concurrency or ordering guarantee
- a CLI stdout/stderr/exit-code contract
- a JSON or JSONL schema contract
- a regression from an exemplar, fixture, corpus, or previous bug

A test is not meaningful when it only:

- repeats the implementation line by line
- checks a private helper with no observable contract
- asserts on fixture text created solely to make the test pass
- mocks away the actual boundary being claimed
- accepts any output that contains a vague substring
- changes every time the implementation is refactored without behavior change

## Coverage Oracle: Mutation, Not Counts

There is no per-crate test count. A count floor (formerly 50 per crate) rewarded
volume over strength and was never enforced: the project's antipattern catalogue
lists "test count literals as oracle". Instead, every diff's production code is
mutated (`cargo mutants --no-config --in-diff`) and each surviving mutant is closed
by a test that fails on it, or the code is shown to be unreachable. A crate with few
tests and no survivors is covered; a crate with many tests and survivors is not.

The test classes a module usually needs, as a checklist rather than a quota:

| Test class | Typical | Purpose |
| --- | ---: | --- |
| Unit invariants | 10 | IDs, ranges, parser fragments, pure strategy rules. |
| Fixture tests | 10 | Real patch, syntax, projection, graph, or annotation examples. |
| Negative tests | 8 | Malformed input, unsupported states, budget exhaustion. |
| Integration tests | 8 | Crate boundary, CLI, filesystem, daemon, or pipeline behavior. |
| Property or fuzz seeds | 5 | Generalized parser, codec, ordering, or round-trip behavior. |
| Regression tests | 5 | Bugs, exemplar lessons, corpus failures, release incidents. |
| Resource tests | 4 | Memory, time, file size, cache, parallelism, socket ownership. |

The numbers are typical, not required. A module adds the tests its risk profile
and its surviving mutants demand.

## Module Test Targets

| Module or crate | Required focus |
| --- | --- |
| `deep-diff-forge-core` | IDs, ranges, stable equality, receipt schemas, enum serialization once serde lands. |
| `deep-diff-forge-patch` | Unified patch parsing, Git headers, binary markers, no-newline metadata, render round trips. |
| `deep-diff-forge-projection` | Inline, side-by-side, stacked, JSON, JSONL, width, windowing, stable row IDs. |
| `deep-diff-forge-pipeline` | stdin opt-in, stdout/stderr split, JSONL streaming, manifest stage order, pipe failures. |
| Git acquisition (in `deep-diff-forge-patch`, `git_acquire.rs`; there is no `-git` crate) | worktree status, index/tree pairs, external diff invocation, attributes, ignored files. |
| `deep-diff-forge-syntax` | language detection, parser budgets, tree lowering, moved nodes, renames, reformat-only spans. |
| Planner (in `deep-diff-forge-learning` and the patch strategy code; there is no `-planner` crate) | strategy selection, generated suppression, binary fallback, budget profiles, explanations. |
| `deep-diff-forge-graph` | graph node/edge creation, ranking determinism, risk reasons, ownership/test links. |
| `deep-diff-forge-agent` | grounded vs ungrounded annotations, evidence links, sanitization, approval records. |
| `deep-diff-forge-tui` | layout state, keyboard/mouse commands, sidebar navigation, toggles, viewport stability. |
| `deep-diff-forge-cluster` | deterministic sharding, lane failures, join policies, replay manifests, resource receipts. |
| `deep-diff-forge-loom` (not implemented; `loom` is refused) | intake, boundary map, fixture synthesis, gate receipts, license/provenance fields. |
| `deep-diff-forge-learning` | receipt validation, privacy (no source, patch text or paths), store bounds and permissions, promotion/demotion. |
| `deep-diff-forge-review` | council admission, receipt qualification, sandbox argv, tuicr import/export, sealed evidence. |
| `deep-diff-forge-sdk` | client framing, error mapping, parity with the daemon and `rpc`. |
| `deep-diff-forge-daemon` | socket path validation, ownership, stale versus live socket, payload caps, read deadline, connection cap, session lifecycle, health RPC. |
| `deep-diff-forge-cli` | argument parsing, exit codes, stdout/stderr, contract commands, no-TTY behavior. |

## Integration Tests

Integration tests must cover user-visible and machine-visible boundaries.

Required integration surfaces by maturity:

| Maturity | Required integration tests |
| --- | --- |
| L0 Bootstrap | CLI smoke and contract commands. |
| L1 Patch | `--stdin-patch --json`, parser/render fixture round trip. |
| L2 Projection | pager-compatible output, JSON/JSONL schema output, width behavior. |
| L3 Pipeline | chained stdin/stdout commands and failure propagation. |
| L4 Semantic | syntax fallback preserved with patch truth. |
| L5 Review | TUI state smoke and annotation grounding. |
| L6 Cluster | parallel lane replay and deterministic joins. |
| L7 Daemon | Unix-socket lifecycle and health RPC (no named-pipe implementation). |
| L8 Release | package artifact smoke and checksum verification. |
| L9 Learning | receipt replay and promotion/demotion simulation. |

## Anti-Test-Fitting Rules

Test fitting is any change that makes tests green while weakening the behavior
the tests are supposed to prove.

Forbidden patterns:

- relaxing assertions without a spec update
- changing fixtures to match a bug
- over-mocking parsers, filesystems, sockets, or command boundaries
- accepting unordered output when deterministic order is required
- ignoring stderr when stderr is part of the contract
- replacing semantic validation with snapshot-only approval
- adding tests after code only to mirror implementation branches
- deleting regression tests because a refactor made them inconvenient

Required countermeasures:

- Each fixture test states the behavior it proves.
- Each regression test references the bug, receipt, or exemplar source.
- Each snapshot test has at least one semantic assertion.
- Each integration test uses the public command or API boundary.
- Randomized tests record seed values on failure.
- Fuzz-discovered failures become deterministic regression fixtures.

## Top-Tail Practices To Adopt

These practices are drawn from mature diff engines, parser projects, release
systems, and high-reliability CLI tooling:

- **Golden fixtures with semantic assertions:** snapshots are useful, but every
  snapshot suite also asserts stable IDs, counts, fallback reasons, or anchors.
- **Round-trip properties:** parsing and rendering must preserve apply-able
  patch truth.
- **Metamorphic tests:** equivalent changes such as path normalization or line
  ending variants should preserve intended semantics.
- **Negative-first parser tests:** malformed inputs are as important as valid
  inputs.
- **Contract-first CLI tests:** stdout, stderr, exit code, TTY detection, color,
  and JSON modes are explicitly tested.
- **Budget tests:** large files, parser timeouts, node ceilings, and memory
  pressure produce fallback records, not panics.
- **Concurrency determinism tests:** cluster lanes and cache hits cannot change
  observable order unless `as-ready` is explicitly requested.
- **Security boundary tests:** sockets, cache directories, payload caps, and
  annotations are treated as hostile input surfaces.
- **Corpus replay receipts:** large corpus runs produce structured evidence
  instead of only pass/fail summaries.
- **Failure-domain tests:** patch, semantic, graph, projection, daemon, and
  release failures are isolated and reported by domain.

## Test Review Checklist

Before accepting a test:

- What behavior does it prove?
- Which public contract, invariant, fixture, or bug does it trace to?
- Would this test fail for a plausible real bug?
- Does it avoid asserting private implementation details?
- Does it cover success and failure where both matter?
- Does it use the smallest fixture that still proves the behavior?
- Is the assertion stronger than "contains string"?
- Does it preserve patch truth?
- Does it avoid hidden network, daemon, and global filesystem dependencies?
- Is this test still valuable after a refactor?

## Deployment Gate

The deployment framework must enforce this standard progressively:

```text
L0: smoke and contract tests accepted while codebase is bootstrap-only
L1+: every diff to a production crate leaves 0 missed mutants in its scope
```

Until the product-native deployment gate exists, `just gate-feature` is the
quick local path and `just ci` the full local replica of the CI gate jobs.
Future enforcement should move into (not implemented):

```text
deep-diff-forge deploy gate --mode feature
```

### Status (2026-10-26)

Done, for the review-council stack:

- **Mutation as a gate, not a report.** The council's mutation gate runs
  cargo-mutants on the diff's scope in the Podman sandbox. Councils r13–r16
  each finished with 0 missed mutants (for example 76/76 caught in r14 and
  r15), and surviving mutants from each pre-council rehearsal were closed by
  new tests before review (the `test:` commits for the r9–r15 rehearsals' surviving mutants).
- **Exclusions are controlled.** Only `#[cfg(test)]` may hide code from
  mutation; the exemption oracle refuses anything else and proves the hidden
  code leaves non-test MIR unchanged, with positive controls
  ([review council](REVIEW_COUNCIL.md#r18-council-r17-follow-ups--only-cfgtest-every-member-and-a-diff-builder-bug)).
- **Tests that fail against the old code.** Fixes in r12–r18 came with tests
  shown to fail before the fix, such as the `mutation_scope` property tests
  that now assert hunk ordering.
- **CI runs the real thing.** `main` CI runs the real-Podman planted-breach
  suite on a GitHub-hosted runner and is green at run 40274253094.

Remaining: the `deploy gate` command above is still future work. The 50-test
minimum is retired in favour of diff-scoped mutation (anchor B-09, 2026-10-27).
Evidence: [`EVIDENCE.md`](../EVIDENCE.md#review-council-and-ci-2026-10-26).

## Deployment Link

- Framework: [Codebase Deployment Framework](DEPLOYMENT_FRAMEWORK.md)
