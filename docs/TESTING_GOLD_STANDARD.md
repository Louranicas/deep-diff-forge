# Testing Gold Standard

Deep-Diff-Forge treats tests as executable design evidence. A module is not
deployment-ready because it has many tests; it is deployment-ready when its
tests prove real behavior, failure modes, invariants, and integration contracts.

This standard is intentionally stricter than the current bootstrap repo state.
The current implementation has only smoke behavior. Production modules must
meet this standard before they can be marked deployable.

## Non-Negotiables

- Every production module must have at least 50 meaningful tests before release
  eligibility.
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

## The 50-Test Minimum

The minimum applies per production module or crate. A module may exceed 50
tests; it may not ship below 50 unless it is explicitly marked bootstrap,
experimental, or unused by release artifacts.

Recommended distribution:

| Test class | Minimum | Purpose |
| --- | ---: | --- |
| Unit invariants | 10 | IDs, ranges, parser fragments, pure strategy rules. |
| Fixture tests | 10 | Real patch, syntax, projection, graph, or annotation examples. |
| Negative tests | 8 | Malformed input, unsupported states, budget exhaustion. |
| Integration tests | 8 | Crate boundary, CLI, filesystem, daemon, or pipeline behavior. |
| Property or fuzz seeds | 5 | Generalized parser, codec, ordering, or round-trip behavior. |
| Regression tests | 5 | Bugs, exemplar lessons, corpus failures, release incidents. |
| Resource tests | 4 | Memory, time, file size, cache, parallelism, socket ownership. |

These numbers are floors, not quotas. A module should add the tests demanded by
its risk profile even if it already exceeds 50.

## Module Test Targets

| Module or crate | Required focus |
| --- | --- |
| `deep-diff-forge-core` | IDs, ranges, stable equality, receipt schemas, enum serialization once serde lands. |
| `deep-diff-forge-patch` | Unified patch parsing, Git headers, binary markers, no-newline metadata, render round trips. |
| `deep-diff-forge-projection` | Inline, side-by-side, stacked, JSON, JSONL, width, windowing, stable row IDs. |
| `deep-diff-forge-pipeline` | stdin opt-in, stdout/stderr split, JSONL streaming, manifest stage order, pipe failures. |
| `deep-diff-forge-git` | worktree status, index/tree pairs, external diff invocation, attributes, ignored files. |
| `deep-diff-forge-syntax` | language detection, parser budgets, tree lowering, moved nodes, renames, reformat-only spans. |
| `deep-diff-forge-planner` | strategy selection, generated suppression, binary fallback, budget profiles, explanations. |
| `deep-diff-forge-graph` | graph node/edge creation, ranking determinism, risk reasons, ownership/test links. |
| `deep-diff-forge-agent` | grounded vs ungrounded annotations, evidence links, sanitization, approval records. |
| `deep-diff-forge-tui` | layout state, keyboard/mouse commands, sidebar navigation, toggles, viewport stability. |
| `deep-diff-forge-cluster` | deterministic sharding, lane failures, join policies, replay manifests, resource receipts. |
| `deep-diff-forge-loom` | intake, boundary map, fixture synthesis, gate receipts, license/provenance fields. |
| `deep-diff-forge-daemon` | socket path validation, ownership, payload caps, session lifecycle, health RPC. |
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
| L7 Daemon | UDS/named-pipe lifecycle and health RPC. |
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
L1: patch crate must meet 50-test minimum before patch release
L2+: every production crate or module must meet 50-test minimum before release
```

Until the product-native deployment gate exists, `just gate-feature` is the
local enforcement path. Future enforcement should move into:

```text
deep-diff-forge deploy gate --mode feature
deep-diff-forge deploy test-audit --json
```

## Deployment Link

- Framework: [Codebase Deployment Framework](DEPLOYMENT_FRAMEWORK.md)
