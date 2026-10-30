# Plan: Code Evidence, Verified Findings, and Observability

**Kind:** plan, append-only after acceptance. Versions are dated sections. A later revision is a new
section; earlier ones are kept. Evidence labels: **measured**, **inferred**, **unmeasured**.

Related: [production readiness](PRODUCTION_READINESS.md) · [roadmap](ROADMAP.md) ·
[API and IPC](API_AND_IPC.md) · [operations](OPERATIONS_AND_DAEMON.md) ·
[learning loop](LEARNING_LOOP.md) · [testing](TESTING_GOLD_STANDARD.md).

## 1.0 — 2026-10-30 (main `ff9e079`, v0.4.0)

### Sources reviewed

- **Code (primary).** These are the 14 crates on `main` after v0.4.0.
  - Read in full for this plan:
    - `graph/src/risk.rs` (12 risk signals, path and metadata only);
    - `agent/src/{lib,anchor,evidence,store}.rs` (grounding, sanitising, anchor validation, revision binding; library only);
    - `core/src/lib.rs` (`AgentAnnotation`, `AnnotationAnchor`, `PatchHunk::address`, a content-addressed SHA-256);
    - `daemon/src/{handler,serve,admission,inspect}.rs`;
    - `cli/src/{main,mcp,doctor}.rs`;
    - `tui/src/notes.rs`.
- **Docs.**
  - The roadmap. Phase 5 has no path for external agent annotations, and Phase 10 has "no corpus manifest, benchmark receipts".
  - [Operations](OPERATIONS_AND_DAEMON.md#observability): it says "Target design, not implemented".
  - `PIONEER_FEATURE_SPECS.md`.
  - The vault's *Expert Circle Agent-First DDF 2026-10-28* ranked plan. Tiers 0 and 1 are shipped. Of Tier 2, #12 (syntax-evidence risk signals) is not. Of Tier 3, #13 (opt-in annotation export, anchored and sealed) is not.
- **Probe (measured, `ff9e079`).** I passed a hunk that adds `unsafe { std::process::Command::new("curl"); }` to `src/lib.rs`. It ranks with one signal, `public_api_surface`, and a score of 7. That is the same as any edit to `lib.rs`. The ranking cannot see what the code does.

### The two most impactful additions, and why

The test for each candidate was whether it makes the correct review cheaper for an agent to get right without making an incomplete review look complete. That is the circle's one sentence.

| Candidate | Agent value | Fits patch truth | Cost | Verdict |
| --- | --- | --- | --- | --- |
| **A. Code-evidence risk signals** with addresses | High. Agents triage by rank. Today rank sees paths, not code, so a dangerous hunk in an ordinary file is invisible. | Yes. It reads only the hunk lines DDF already holds, and each signal points at a hunk address and line, so an agent can check it. | M | **Chosen** |
| **B. Verified agent findings** (intake, anchor check, seal) | High. Multi-agent review (Claude, Codex) produces findings, and nothing checks that they point at real, current lines. | Yes. Findings are checked against the sealed review. A finding never changes patch truth. | M | **Chosen** |
| Hunk-to-symbol join (Rust) | Medium. The `@@` heading already names the function for most hunks (38 of 39 in the circle's sample). | Needs the post-image source, so it only works with Git or `analyze` inputs. | M–L | Deferred: after A, reusing A's language layer |
| `<old> <new>` file-pair diff | Medium, and mostly for humans | Yes | M | Deferred |
| `chain` / `cluster` / `loom` subcommands | Low for agents today. Agents already chain through `envelope` and JSON. | Yes | L | Deferred |
| Interdiff (review a patch's revision 2 against revision 1) | High later | Yes | L | Deferred (the circle deferred "deltas") |
| More tree-sitter grammars | Medium | Yes | Each grammar is a new dependency | Deferred. A's lexical layer covers six languages with no new dependency. |

A and B fit together. A makes DDF say *where the danger is*, with an address. B lets an agent say *what it found*, and makes DDF check where the finding points.

---

### Feature A: code-evidence risk signals

**Goal.** Each file's rank carries machine-checkable code evidence: which signal, in which hunk (address), at which line. Evidence comes from added and removed lines of the patch alone, so it works for every input: stdin, Git, MCP, daemon, `rpc`.

**Signals.** This is a closed `EvidenceSignal` enum in a new `graph/src/evidence.rs`.

| Signal | Lines | Languages and paths | Matches (outside comments and strings) | Weight |
| --- | --- | --- | --- | --- |
| `unsafe_added` | added | `.rs` | the `unsafe` keyword | 4 |
| `process_spawn` | added | rs, py, js/ts, go, sh | `Command::new`, `std::process::`; `subprocess.`, `os.system(`, `os.popen(`, `os.exec`; `child_process`, `execSync(`, `spawnSync(`; `exec.Command(`; in shell, `` ` `` or `$(` | 4 |
| `network_fetch` | added | rs, py, js/ts, go, sh, CI | `reqwest::`, `TcpStream::connect`, `UdpSocket::bind`; `requests.`, `urllib.request`, `socket.`; `fetch(`, `XMLHttpRequest`; `http.Get(`, `net.Dial(`; `curl `, `wget ` | 3 |
| `dynamic_eval` | added | py, js/ts, sh | `eval(`, `exec(`, `new Function(`, `compile(`; shell `eval ` | 4 |
| `ci_privilege` | added | `.github/workflows/*`, `.gitlab-ci.yml` | `pull_request_target`, `permissions: write-all`, `contents: write`, `id-token: write`, `secrets: inherit` | 4 |
| `build_hook` | added | `build.rs` (any line), `package.json` (`"preinstall"`, `"install"`, `"postinstall"`), `setup.py`, `Makefile` | code that runs at build or install time | 3 |
| `secret_like` | added | any text | `-----BEGIN` … `PRIVATE KEY-----`, `AKIA` plus 16 upper-case alphanumerics, `ghp_` / `gho_` / `ghs_` plus 36 alphanumerics, `xox[abp]-` | 5 |
| `check_suppressed` | added | rs, py, js/ts, go | `#[allow(`, `#![allow(`, `#[expect(`; `# noqa`, `# type: ignore`; `eslint-disable`, `@ts-ignore`, `@ts-nocheck`; `//nolint` | 2 |
| `test_removed` | removed | rs, py, js/ts, go | `#[test]`, `#[tokio::test]`; `def test_`; `it(`, `test(`, `describe(`; `func Test` | 3 |

- **The lexical layer.** `graph/src/lexical.rs` is a single-line scanner per language family: C-like (rs, js/ts, go), hash (py, sh, yaml) and none.
  - It removes `//` and `#` comments, `/* … */` comments that open and close on the line, and quoted strings (with escapes). What is left are code bytes, which are matched with word boundaries.
  - It is **line-lexical evidence**: a multi-line block comment or raw string can hide a token, or produce one. The JSON and the docs say so (`"basis": "lexical"`). It is never presented as parsing.
  - `secret_like` deliberately scans strings as well, and comments too, since a secret can sit in either.
- **Scoring.** Each signal kind adds its weight once per file. Evidence weight is **not** removed for generated, vendored or test paths: a path cannot hide code (the circle's D2 principle).
  - For a test path, the weight is halved, rounded up, and `test_removed` is kept at full weight.
  - The `RiskSignal` enum gains one variant per evidence kind, so `signals` lists them, and the order of `rank` stays deterministic.
- **Output (additive; schemas stay `v0`, as the API doc's additive rule allows).**
  - `rank.v0`: each ranked file gains `"evidence": [{"signal", "address", "side": "new"|"old", "line": N, "basis": "lexical"}]`. At most 32 entries per file, then `"evidence_truncated": true`.
  - `review-outline.v0`: each hunk gains `"evidence": [signal labels]`.
  - The same appears in MCP `rank_patch` and `review_outline`, daemon `diff.rank`, `review.outline`, `session.snapshot` and `diff.plan`, and `rpc`. The parity tests (`cli/tests/daemon_parity.rs`, `mcp.rs`) show these are one document.
  - TUI: each evidence signal becomes a system note on its hunk ("unsafe added (line 12)").
- **Tests.**
  - Per signal: one positive, and negatives in a comment, in a string and on a context line.
  - Path invariance: the same hunk under `src/generated/` keeps its evidence.
  - Removed versus added lines.
  - A cap test.
  - Ordering determinism.
  - The circle probe: `unsafe` plus `Command` in `src/lib.rs` must outrank a README edit and a large docs edit.
- **Fuzz.** A new `evidence_scan` target: arbitrary bytes as a hunk line in each language must not panic, and every evidence `line` must lie inside its hunk.
- **Mutation.** Diff-scoped in CI, with 0 missed.

### Feature B: verified agent findings

**Goal.** An agent's findings are checked against patch truth before anyone reads them. The check covers address, line, freshness, size and duplicates. What survives is sealed and shown, and nothing else is. DDF never judges whether a claim is true: it checks *where* a finding points, not *what* it says. Output says `"claim_verified": false` every time.

- **Input.** Findings are JSONL. Each line is a `deep-diff-forge.finding.v0` object:
  - required: `agent` (string), `severity` (`info|low|medium|high|critical`), `title` (string, at most 200 characters) and `body` (string);
  - an anchor: `address` (a hunk address), or `path` alone (a file-level finding);
  - optional: `side` (`new|old`) and `line` (an integer inside the hunk on that side);
  - optional: `review` (the `input_sha256` the agent reviewed), `id`, `model`, `evidence` (at most 16 strings).
- **Verdict per finding.**
  - **Accepted** (normalised): `path`, `address`, `side`, `line`, and the line's own `kind` and `text`. The text is cut to 200 bytes and escaped with `display_safe`. The body is passed through `sanitize_body` (at most 4,096 characters).
  - **Refused**, with one stable reason code:
    - `malformed` (not JSON, wrong types, or unknown fields);
    - `stale_review` (`review` ≠ the sealed `input_sha256`);
    - `unknown_path`, `unknown_address`, or `line_outside_hunk`;
    - `duplicate` (same anchor, same title, same agent);
    - `too_long`;
    - `limit` (more than 10,000 findings).
- **Output.** `deep-diff-forge.findings.v0`, sealed:
  - `input_sha256` of the patch, `findings_sha256` of the findings bytes, and `tool`;
  - `accepted[]` and `refused[]` (`index`, `reason`, and an excerpt cut to 120 bytes);
  - `counts`, and `"claim_verified": false`.
- **Surfaces.**
  - CLI: `deep-diff-forge findings verify --findings FILE [--require-all] [--json]`. The patch comes from stdin or a `--git-*` selection.
    - Exit codes: 0; 7 with `--require-all` when any finding is refused; 4 when the findings file is unreadable or over 8 MiB; 2 on a usage error.
  - MCP: `verify_findings`, read-only (`readOnlyHint: true`), taking a `patch` and `findings` (an array).
  - Daemon: `findings.verify` (`Work::Request`). It is protocol 2: `PROTOCOL_VERSION` goes up, and older clients still negotiate 0 and 1.
  - `rpc`: the same method.
  - Python SDK: `verify_findings(patch, findings)`.
  - TUI: `review --findings FILE` shows accepted findings as agent notes (`Agent` source, `PartiallyGrounded` at most), and the status line shows how many were refused.
- **Code location.** A new `agent/src/findings.rs` holds parsing and verification as a pure library over `&[ReviewFile]` and the seal. The CLI, MCP, daemon and TUI only adapt input and output.
- **Tests.** One per refusal reason, plus:
  - a test that findings change nothing in the review output: the review document's bytes are the same with and without them;
  - prompt-injection bodies, which are sanitised and keep no control or bidi characters;
  - 10,001 findings, which hits `limit`;
  - parity across CLI, MCP and daemon.
- **Fuzz.** A new `findings_verify` target: arbitrary findings bytes against a fixture patch. It must not panic, output must always be JSON, every accepted anchor must resolve, and accepted plus refused must equal the number of input lines.

---

### System M: monitoring, evaluation and benchmarking

All three parts share one rule: **a number is a receipt.** Each has a schema, the version and build profile, a host fingerprint (CPU count, OS, architecture), and the inputs' SHA-256. Numbers from different machines are never compared as if they were the same.

**M1. Runtime monitoring (daemon).**

- **`daemon.metrics`** (protocol 2): one document, `deep-diff-forge.daemon-metrics.v0`, containing:
  - **per method:** requests, errors by RPC code, and latency as fixed log₂ microsecond buckets, from which p50, p95 and p99 are estimated and labelled as estimates;
  - **admission:** admitted, waited, total wait in microseconds, busy refusals, and peak in-flight bytes and peak running;
  - **connections:** accepted, refused at the limit, refused as another user's peer, read timeouts and accept errors;
  - **engine:** contained panics, session table resets, sessions opened, closed and evicted, and peak retained bytes;
  - **uptime.**
- **Recording cost.** Everything is lock-free (`AtomicU64`) in a `metrics.rs` owned by the `Engine`. Recording costs one or two atomic adds per event. The overhead target is under 1% on the parallel benchmark (measured, with M2).
- **Commands.** `deep-diff-forge daemon metrics [--json] [--socket PATH]`.
- **`deploy doctor`.** When a daemon is live, it reads `daemon.metrics` and warns on contained panics (above 0), busy refusals (above 0) or session resets (above 0), each with its count.
- **Structured logs.** `daemon start --log-format json` writes one JSON object per line (`ts_ms`, `level`, `event`, fields). The default stays the current text. Events covered: connection refused, read timeout, contained panic, busy refusal, accept error, start and stop.

**M2. Benchmarking.**

- **The command.** `deep-diff-forge bench [--json] [--quick] [--workload NAME]...`.
  - Workloads are deterministic, generated from a fixed seed with no files: `parse_seal`, `rank`, `evidence_scan`, `outline_page`, `findings_verify`, `structural_rust` and `daemon_parallel` (an in-process server over a socket pair, using M1's counters).
  - Each runs a warm-up, then R repetitions (5 with `--quick`, 15 otherwise), and reports min, median and p95 in nanoseconds, plus bytes per second.
  - The receipt is `deep-diff-forge.bench.v0`. The clock is read through one named clock door.
- **Comparison.** `scripts/bench_compare.py BASE HEAD [--max-regression 0.25] [--noise-floor-ns 200000]` compares each workload's median. It fails when head is slower than base by more than the ratio *and* the noise floor. Unmatched workloads are reported and do not fail.
- **CI.** A `bench (base vs head)` job runs on pull requests: it builds the merge base and the head in release mode, and alternates their runs three times on the same runner, so runner speed cancels out.
  - It is **informational at first:** not required, and its noise is recorded.
  - It becomes required by a later decision once two weeks of runs show a false-failure rate under 2% (a measured criterion, not a feeling).
- **Weekly campaign.** It adds a `bench` job that keeps the receipt as an artifact.

**M3. Evaluation.**

- **The corpus.** `fixtures/eval/` is a corpus with a manifest, the "corpus manifest" Phase 10 of the roadmap asks for. Each case is a patch plus labels:
  - `ranking/`: `must_outrank` pairs, for example the `unsafe` hunk above a README edit and above a large docs change;
  - `evidence/`: the signals expected at each address and line, including negatives (a keyword in a comment, in a string, or on a context line);
  - `findings/`: the expected accepted or refused verdict per line.
- **The command.** `deep-diff-forge eval [--corpus DIR] [--json]` prints `deep-diff-forge.eval.v0`:
  - ranking pair accuracy;
  - evidence precision and recall per signal;
  - findings verdict accuracy;
  - the corpus's SHA-256.
- **The gate.** `cli/tests/eval.rs` runs the committed corpus and enforces floors: evidence precision of 1.0 on the negatives, recall of at least 0.95, ranking pair accuracy of 1.0 and findings accuracy of 1.0.
  - A change that lowers a metric fails CI.
  - A change that raises it updates the floor in the same PR, so the floors only ever ratchet up.
- **The learning loop.** Eval receipts are the "benchmark receipts" the [learning loop](LEARNING_LOOP.md) lacks: `learn record` accepts a `deep-diff-forge.eval.v0` receipt as evidence (Phase 10).

---

### Delivery (each a PR through the 10 required checks)

| # | PR | Contents | Acceptance |
| --- | --- | --- | --- |
| 1 | A: evidence | `lexical.rs`, `evidence.rs`, the risk variants and weights, evidence in rank and outline (CLI, daemon, MCP, `rpc`), TUI notes, `evidence_scan` fuzz target, docs | the circle probe ranks first; negatives clean; parity tests; 0 missed mutants; fuzz 300 s clean |
| 2 | B: findings | `agent/src/findings.rs`, `findings verify`, MCP `verify_findings`, daemon `findings.verify` (protocol 2), Python SDK, `review --findings`, `findings_verify` fuzz target, docs | one test per refusal reason; seal and parity; review bytes unchanged; fuzz 300 s clean |
| 3 | M1: monitoring | `daemon/src/metrics.rs`, `daemon.metrics`, `daemon metrics`, doctor warnings, `--log-format json` | counters checked by tests that drive each event; overhead measured |
| 4 | M2 + M3: bench and eval | `bench`, `eval`, `fixtures/eval/`, `bench_compare.py` and its tests, CI `bench` job (informational), campaign job, `cli/tests/eval.rs` floors | eval floors hold; the compare script's own tests catch a planted regression |

---

### Gap analysis (of this plan, against the code and docs)

| # | Gap | Consequence if left | Recommendation |
| --- | --- | --- | --- |
| G1 | Six languages are detected by **extension** only. A shebang script with no extension gets no rules. | Shell evidence is missed in `scripts/run` | Detect `#!/bin/sh`, `bash`, `python` and `node` from the first line of a new file's hunk at line 1. Otherwise no language: report nothing rather than guess. |
| G2 | The lexical layer **cannot see a multi-line context**: a hunk that opens a `/*` on one line hides the next. | False positives inside block comments; false negatives inside multi-line strings | Track `/* */` state across the lines **within a hunk**, in order (state resets at each hunk). Document what remains: a hunk that starts inside a comment. |
| G3 | **Weights are new tuning knobs.** Raising one reorders every rank. | Silent rank changes | Weights live in one `const` table. The eval corpus's `must_outrank` pairs are the regression test, and a weight change must keep eval accuracy at 1.0. |
| G4 | **Hunk addresses are content hashes**: an agent that anchored to a hunk before a rebase gets `unknown_address`. | Lost findings | That is correct and fails closed. Also add `path` plus `line` (new side) as an anchor, re-resolved against the current review, and say which anchor was used (`anchor: "address"` or `"line"`). |
| G5 | **Protocol bump to 2.** An old client asking for protocol 1 must not see the new methods. | Compatibility break | `METHODS` `since: 2` for `findings.verify` and `daemon.metrics`, with a negotiation test for 0, 1 and 2. |
| G6 | **Metrics are per process and lost on restart.** | No history | That is by design (no store). Doctor reports uptime beside the counts. History comes from the campaign's receipts. |
| G7 | **The bench CI job doubles build time** on PRs. | Slower PRs | Run it in parallel with the existing jobs (it does not block them). Use `--quick`, and skip it for docs-only diffs with a path check inside the job, so it still reports. |
| G8 | **`findings verify` needs the same patch bytes** the agent saw. With `--git-*`, the working tree may have moved. | `stale_review` surprises | Return the current `input_sha256` in every refusal, so the agent can re-fetch. Document "verify against the bytes you reviewed". |
| G9 | **The eval corpus is written by the same author as the rules.** | It tests the rules' own idea of risk | Include real hunks from this repository's history (for example the `GIT binary patch` hiding case and the Trojan Source case), each with its commit. Mark synthetic cases `"origin": "synthetic"`. |
| G10 | **No doc owner for the new schemas.** | Drift | `API_AND_IPC.md` gains a section per schema. A contract test checks that each schema name the code emits appears in the docs. |

### Non-anthropocentric gap analysis

This reads the plan from the side of the things that use DDF and are not people: LLM agents, CI runners, the daemon process, the fuzzer and mutation tester, the corpus, and the repository itself.

| # | Who | What they need that a human-centred plan misses | Recommendation |
| --- | --- | --- | --- |
| N1 | **An LLM agent (token budget)** | Evidence repeated on every file and hunk grows the response. An agent pays for every byte. | The evidence cap is 32 per file. The outline carries only labels, and the location is in rank. `--max-bytes` still bounds every page. |
| N2 | **An LLM agent (determinism)** | Agents compare runs. Unordered evidence, or a changing excerpt, reads as a change. | Evidence is sorted by (hunk order, side, line, signal), and excerpts are deterministic. A determinism test runs the same input 3 times and compares the bytes. |
| N3 | **An adversarial agent** | A finding's body is a prompt-injection channel to the *next* agent that reads the review. A flood of findings can bury the real ones. | Sanitised bodies, bounded counts and `duplicate`. The output marks every finding's `source: "agent"` and `claim_verified: false`, so a consuming agent can't mistake it for engine output. Findings are never mixed into `review.v0`. |
| N4 | **An agent optimising against DDF (Goodhart)** | Once agents are scored on "evidence found", they learn to avoid tokens rather than risk. | Evidence is labelled `basis: lexical`, and documented as triage, not a verdict. The eval corpus includes evasions (string-concatenated `Com`+`mand`) as **documented misses**, so the limit stays visible. |
| N5 | **The daemon process** | Metrics recording on the request path must not add lock contention, now that requests run in parallel. | Atomics only, no lock, and a benchmark (M2 `daemon_parallel`) shows overhead under 1%. |
| N6 | **CI runners** | Absolute benchmark numbers vary 2–3× across runners. A fixed threshold either always fails or never does. | Base and head run on the same runner, alternating. Fails need both a ratio and a noise floor. The job is informational until its false-failure rate is measured. |
| N7 | **The fuzzer** | New byte-facing parsers (lexical scanner, findings JSONL) are only as safe as their fuzz targets. | Two new targets, added to `fuzz/Cargo.toml`, so the CI smoke and the weekly campaign include them automatically (`fuzz_target.sh --list`). |
| N8 | **The mutation tester** | Lexical tables of tokens produce many equivalent mutants (a removed table entry that no test names). | One test per table entry, generated from the same table, so every entry has a positive case. Equivalent exclusions need a written reason. |
| N9 | **The corpus** | A corpus nobody can re-derive rots. | `fixtures/eval/MANIFEST.json` lists every case with its SHA-256, its origin and its labels. The eval receipt carries the manifest's SHA-256. |
| N10 | **The repository** | Every new dependency is a supply-chain edge (cargo-deny, audit, SBOM). | No new crates: the lexical layer, the histograms and the JSONL handling use std and `serde_json`, which is already a dependency. |
| N11 | **Downstream machines reading schemas** | Silent shape changes break parsers. | Only additive fields go into `v0` schemas. New documents get new names (`findings.v0`, `daemon-metrics.v0`, `bench.v0`, `eval.v0`). `engine.methods` lists the new methods with `since: 2`. |

### Recommendations integrated (plan revision 1.1, same day)

The delivery table above stands. These are added to its PRs:

- **PR 1 (A):** G1 (shebang detection), G2 (block-comment state within a hunk), G3 (one weight table, eval-guarded), N1 (cap of 32, labels in the outline), N2 (sorted, with a determinism test), N4 (`basis: lexical`, documented misses), N8 (one generated test per token table entry), N7 (the `evidence_scan` fuzz target).
- **PR 2 (B):** G4 (a `path` plus `line` anchor, with `anchor` named), G5 (protocol 2 with a negotiation test), G8 (the current `input_sha256` in refusals), N3 (`source: "agent"`, `claim_verified: false`, never mixed into `review.v0`), N7 (the `findings_verify` fuzz target).
- **PR 3 (M1):** N5 (atomics only, overhead measured), G6 (uptime beside counts).
- **PR 4 (M2 + M3):** G7 (parallel, `--quick`, docs-only skip), N6 (same runner, alternating, ratio and noise floor, informational first), G9 and N9 (manifest, real cases with commits, synthetic marked), G10 (a schema-to-docs contract test).
- **Every PR:** N10 (no new dependencies) and N11 (additive `v0` fields, new names for new documents).

### Out of scope (and why)

- **An LLM inside DDF, patch repair, and a patch-writing MCP tool:** the circle's "never" list.
- **A persistent findings store, and signed receipts:** the circle deferred both. B prints sealed output to stdout, and a store comes later.
- **A metrics endpoint over TCP, and Prometheus:** the daemon stays on a Unix socket, single user. `daemon.metrics` is the interface.
- **A claim that evidence is complete:** the lexical layer finds tokens. It doesn't prove a change is safe, and every document says so.
