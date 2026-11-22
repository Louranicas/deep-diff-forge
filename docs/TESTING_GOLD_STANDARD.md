# Testing Gold Standard

Deep-Diff-Forge treats tests as executable design evidence. A module is not
deployment-ready because it has many tests; it is deployment-ready when its
tests prove real behavior, failure modes, invariants, and integration contracts.

This standard is intentionally strict. Production modules must meet it before
they can be marked deployable.

## Status 2026-10-31 (0.5.0 and later)

The sentence "the current implementation has only smoke behavior" (bootstrap
era) no longer holds. How the standard is enforced today:

- **Every pull request** runs the CI jobs in `.github/workflows/ci.yml`:
  `cargo test --workspace --locked`, fmt, clippy and pedantic with
  `-D warnings`, rustdoc `-D warnings`, contract probes, the soak and
  bench-comparison regression tests, a 10 s hostile daemon soak, privacy probes
  and the Python SDK against the built binary (`deploy gate-feature`); the
  workspace tests on macOS (`macos`); the declared-MSRV check on Rust 1.88.0;
  and supply-chain checks.
- **Diff-scoped mutation** is a required check: `diff-mutation`, four shards,
  runs [`scripts/diff_mutation.sh`](../scripts/diff_mutation.sh), which mutates
  only the Rust lines under `crates/` changed since the merge base
  (`cargo mutants --no-config --in-diff`, 300 s timeout) and fails on any missed
  mutant or timeout. `--no-config` ignores a candidate's own mutants config.
- **Fuzz smoke:** every target in `fuzz/Cargo.toml`
  (`patch_parser`, `review_json`, `daemon_protocol`, `agent_annotation`,
  `council_report`, `stdio_mcp`, `stdio_rpc`, `structural_soundness`,
  `findings_verify`, `evidence_scan`, `rust_tokens`, `rust_code_view`, `rust_hunk_places`,
  `hook_input`, `judge_patch`: fifteen; `stdio_invariants.rs` is a shared module)
  runs for 10 s per pull request; CI fails if a declared target is not in the
  run list. Crash inputs become fixtures under
  `fixtures/patch/regressions/`. `rust_code_view` has an oracle: the compiler's own lexer
  (`ra-ap-rustc_lexer`, a dependency of the fuzz crate only). For any text read as a Rust
  file from its first line, no byte of a token that is code may be blank in either view that
  follows that lexer: the side read whole from code (`rust_views`), in which every byte of a
  comment and of a literal's text must be blank too, and the line-by-line reading of release
  0.8.4 (`rust_lexer_view`), in which they must be where that view has one reading. The
  reading of 0.8.3 (`code_view`) is kept as it was and is not held to the lexer. The other starts a side of a
  hunk is read from are held to the same reading: from inside a block comment the views are
  those of `/*` and the text, from inside a string those of `"` and the text, and from line 1
  of a file a shebang line by the compiler's rule is blank. `rust_tokens` holds, among its
  rules, that a comment between two tokens of a construct and complete strings on the lines
  before it do not change the reader's signals. `rust_hunk_places` holds where a hunk is and
  how it starts: it builds a whole Rust file whose every line is in a known state, takes the
  evidence of the file read whole, and requires that the same change written in other ways
  Git accepts loses no item (the file created as several hunks, with honest or false
  headers; a hunk cut out with the lines above and below unseen, whatever the cut left open;
  the same hunk with another new position in its header), that nothing the scanner and
  the Rust rules of release 0.8.3 find on a changed line is missing, and that no join takes
  a reading away (what each hunk gives as a patch of its own is among what its file gives
  for it). Nine of its first ten seeds fail on the sources of the commit before it
  (`8f290e3`); the tenth holds there and fails on round 14 as it was first built, which
  read a hunk Git must put after another only as its continuation. Since round 15 it also
  holds that a construct only a join reads is found when either number of the headers says
  the hunks touch (with or without an old line in the first, in both orders), that a
  construct cut beside the word it is known by is found (so it was said then; since round
  17 only after a method's name and `::`) whatever the numbers and the order
  of the hunks, and that every item of a script's hunk with honest numbers is found under
  false ones and with the hunk written second; the fragment rules also run with removed
  lines, and a raw string's text may hold quotes. Of its 18 seeds the 8 new ones are those
  rules': on the sources of `87d1afd`, each rule run alone, the join rule and the edge
  rule fail on every seed, the script rule on its two seeds, and the raw-string seed fails
  the older rules. In round 16 the edge rule cut only where what a side ended on was
  taken for evidence by itself (`todo!`, `#[ignore`, `Command::`; so it was said then:
  since round 18 nothing is read at the end of a side), and a ninth rule holds the other
  direction: a line of honest code as a one-line hunk (a return type named `Command`, a
  field named `expect`, a variable named `todo`, a call of a function named `expect`)
  reports none of the signals whose constructs its last or first word begins. Its 20 seeds
  hold, and that rule fails on every one of them on the sources of `ceb2e6f`. After the
  eighth independent review it has three more rules (28 seeds): no hunk of added lines
  that names the place where two hunks with old lines touch parts those two (up to three
  such hunks, written before, between or after, with or without a hunk that ends the file
  before them; this rule fails on ten of the seeds on the sources of `ceb2e6f`, its own
  four among them); a removed test's attribute cut after `#[test`, `#[tokio::` and the like
  is reported (with the old side's forms deleted no rule used to fail); and a shebang on a
  removed first line only, or on an added one only, names a script's language, and a
  second hunk that begins with another language's shebang takes nothing away.
  `evidence_scan` also writes its lines as hunks of three with unseen lines between them
  and holds that a hunk read alone has the labels the scan of its file gives it, with no
  pass over the file, and that the view of the file's hunks by path shows the outline's
  labels and goes through the file's hunks at most twice (no target called the reader of
  one hunk before, and nothing held what a view by path costs). In round 17 the end of a
  side went under one rule (a form is read there only where every continuation the
  grammar allows is the construct: a method's name with `::` after it), and
  `rust_hunk_places` (38 seeds) follows it: its edge rule cuts only after that form; its
  rule for honest ends has the lines that round 16 still reported (a constant
  `Command::` whose name stands on the next line, a sentence of a comment that ends on
  `todo!`, a `cfg_attr` list left open on `ignore`), honest forms over TWO lines (`assert!(`
  and `true`, as rustfmt writes a long condition) and removed lines (`#[rstest`, which
  goes on as `::fixture]`), and fails on every seed on the sources of `4391c1e`; a removed
  test's attribute cut in two is found by the join when the old numbers say the hunks
  touch; a change cut inside into touching hunks with true numbers loses no item of the
  same change as one hunk (the ninth independent review's `whole.py`); two hunks at old
  position 0 of an existing file, which `git apply --unidiff-zero` stacks and the reader
  joins to nothing (declared), lose nothing each has alone; and a hunk with lines that
  falsely claims the place where two hunks touch parts nothing when it is written after
  them. `evidence_scan` also holds what a scan keeps in memory: the readings alive at one
  moment never hold more lines than the file has (a unit test in the graph crate holds
  the exact figure: one reading of the longest text; it counts the lines of the views a
  reading holds, not memory, and the memory figures of
  [`reference/risk-ranking.md`](reference/risk-ranking.md) are measured, not held by
  it). In round 18 no form is read at the end of a side (a hunk side is read as text that
  ends there; nothing is inferred from how it ends: in a macro that takes any tokens
  `.unwrap::` goes on as anything, and after a range `unwrap` is a free function), and
  `rust_hunk_places` follows: its edge rule (7) is turned round, a method call with
  generic arguments cut after its `::` or `<` is not found when the numbers say nothing
  and is found by the join when the old number is true; its rule for honest ends (9) has
  the reviewers' shapes (`value.unwrap::` and `value.expect::<` in a macro's braces,
  `0..unwrap::`); a new rule (16) puts a line as a side's last line whole and cut between
  any two of its tokens outside every string and comment of every reading (every cut of a
  line from a list of every form some round read at an edge, written out whole, and one
  cut of the input's own first line) and requires that the cut line reports nothing of
  the reader's eight signals that the whole line does not; and its joined constructs have
  `ignore(…)` in a `cfg_attr` list. `rust_tokens` has `#[ignore(…)]` and `ignore(…)` in a
  `cfg_attr` list among the constructs of its rule 5. The graph crate's tests hold that no
  form of the 52 that rounds 13 to 17 read at an edge gives an item there, on either side,
  spaced or a token a line, in a macro's braces or not. In round 19 a skip is read only
  where the side holds its attribute whole, from `#` to its `]`, for the first 16
  `cfg_attr` lists followed at once (a seventeenth nested one is taken to hold a skip with
  no `ignore` and no `]`, a writer's false item: VD3R19-M-1, IR12-L-2) (round 18 still read
  `#[ignore(` from its parenthesis, and in a macro that takes any tokens it goes on as
  anything: round 18 of Vesper's review, VD3R18-M-1), and the list of forms has 63 (the
  skip's attributes that rounds 13 to 18 read before their `]`, Vesper's two one-line
  sides among them); a test holds the bound of the whole attribute (`#[ignore(…)]` of up
  to 64 tokens from `[` to `]`). The keywords after which a `<` opens a bracketed path
  are the ones rustc 1.99.1 compiles before `<Command>::new("true")`, and a test holds
  each added one (`&mut`, `become`, `do yeet`) and a name before the `<` as controls; the
  rule is lexical (any `mut` straight before the `<`, so `&raw mut` and `mut` in a macro's
  discarded tokens are a writer's false items: VD3R19-L-1, IR12-L-3).
  The fuzz targets are unchanged in this round.
  In 0.9.0 two targets were added for the agent hooks. `hook_input` reads any byte string
  up to 256 KiB (the hook itself reads up to 32 MiB) as the JSON a client gives
  `agent hook <event>`, through the CLI's own `hook` module, for each of the three events:
  UTF-8 text is read only when this target's own reading finds an object that names the
  event, a session and a directory (and then with the same values), and all other text is
  refused with the hooks' own message: a JSON object with `systemMessage` alone, saying
  nothing was judged, and for a stop UNJUDGED and "not a pass". Bytes that are not UTF-8
  are refused before that reading, where the target cannot reach; for them it checks only
  the form of the message, and a CLI test
  (`bytes_that_are_not_utf8_are_refused_by_every_event`) runs the hook on such bytes
  (integrated review of 0.9.0, VI1-L-3 and CI5). `judge_patch` edits a working tree with real Git (one
  base repository per process, put back before every input; files written, deleted or made
  executable, any bytes in names and contents, Git's own files left out) and calls
  `judge_patch`: the patch is read as the hooks read it (`SealedReview` with no guards) or
  refused with an error, and when it is read, its paths are exactly the paths the edits
  changed, each once. Five minutes each on 2026-11-18, no crash: `hook_input` 4,046,034
  runs, `judge_patch` 8,102 (about 26 a second: each runs Git several times). Seeds:
  `fuzz/seeds/hook_input` (4), `fuzz/seeds/judge_patch` (5).
- **Cost of one long line (0.9.0):** one rule closes the class "cost quadratic in the length
  of one line" (FINDING-5, the secret rule, the empty-handler rule): no check walks from a
  match to the end of its line, or of its run, more than once for the line. One test holds
  the class (`long_lines_of_every_trigger_rank_in_bounded_time` in the graph crate): it
  builds its list from the tables in the code (every rule's tokens, modules, spaced heads and
  tails and secret prefixes, the JavaScript operand keywords, the Rust token reader's words,
  the shell's list words, the empty-handler rule's head words, and every ASCII punctuation
  byte in every language; 397 triggers on 2026-11-18), and ranks two lines of 2 MiB for
  each, the trigger repeated and one word ending in it, inside a bound of thread CPU time
  (10 s in a debug build, 2 s in a release build). The gate runs all of them: on 16 threads
  67 s in a debug build, the slowest line 3.2 s; in a release build 6 s, the slowest 0.22 s.
  A new token, keyword or word is in the list without a new test. Readings that changed for
  the class are compared with the reading they replaced, kept for the tests, on seeded
  generated lines (the keyword test, the secret rule and the empty-handler rule: the same
  answer on every line).
- **Sandbox refusal controls:** the rootless-Podman planted-breach suite
  (`crates/deep-diff-forge-cli/tests/council_sandbox.rs`, 12 probes) runs on
  every pull request.
- **Locally:** `just ci` ([`scripts/ci_local.sh`](../scripts/ci_local.sh))
  replays the `secrets`, `msrv`, `gate` and `supply-chain` jobs step for step,
  on the toolchains CI pins (1.99.1, and 1.88.0 for the MSRV step).
- **Gate scripts fail closed** (`cli/tests/repo_scripts.rs`): the pre-push
  secret hook refuses a push whose commits it cannot resolve;
  `diff_mutation.sh` finds changed Rust files by NUL-separated name (not by
  diff header text) and runs at the repository root from any directory;
  `fuzz_target.sh` lists targets with a TOML parser and fuzzes a new private
  corpus each run, under a root it makes if missing.
- **Evaluation gate** (`cli/tests/eval.rs`, every PR): the labelled corpus in
  `fixtures/eval` (59 cases, measured 2026-11-23 on the integrated tree) must score at least its floors,
  which are the measured values: ranking pairs 12/15, evidence precision 186/211,
  evidence recall 186/264, findings 33/33 (0.10.0; sections the parser refuses are in
  `fixtures/eval-refused-by-parser/`, held refused by a test). Any undeclared miss fails. The 106
  declared misses (`known_miss`, `known_false_positive`) count as wrong in their
  metric rather than being excluded, and each fails once the engine gets it
  right. Planted controls prove a wrong label fails with exit 7, a changed file,
  an orphan file, a repeated id, a missing origin or a misspelt field with exit 4,
  a declared miss the engine now gets right with exit 7, and the `ci-privilege`
  and `generated-spawn` ranking cases each fail when their signal is removed.
- **Bench on pull requests** (`bench` job, required since 2026-10-31): base and head on one
  runner, alternating three times; `scripts/bench_compare.py` fails a workload over 25%
  and over 0.2 ms slower, and never compares timings taken on different inputs. A local
  `bench --quick` run found a quadratic cost in address-anchored findings (3,560 ms to
  33 ms once fixed; EVIDENCE.md).
- **Weekly campaign** (`.github/workflows/campaign.yml`): a 10-minute hostile
  daemon soak with real review work that fails on RSS growth over 32 MiB or
  open-fd growth over 4 between early and late samples, and an hour of fuzzing
  for every target in `fuzz/Cargo.toml` (`scripts/fuzz_target.sh`, the same
  script as CI's 10 s smoke). A crash keeps its input as an artifact.
- **TUI live loop** (2026-10-31): `run.rs` tests drive the production loop and
  terminal restore with scripted input against a `TestBackend` (frames drawn,
  burst coalescing, resize, failing and ending input, panic and failed-restore
  paths), and `cli/tests/review_tty.rs` runs bare `review` in a pseudo-terminal.
- **Gaps:** `deploy gate` (below) is not implemented.

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
| `deep-diff-forge-loom` (withdrawn 2026-10-31; `loom` is refused with exit 2) | none: no crate exists. A change that adopts an exemplar's lesson lands as a reviewed change with its own tests (`loom-contract`). |
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
