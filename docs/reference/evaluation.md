# Benchmarks and evaluation

```bash
deep-diff-forge bench --quick                 # time the hot paths on generated inputs
deep-diff-forge bench --json > after.json     # a deep-diff-forge.bench.v0 receipt
scripts/bench_compare.py --base before*.json --head after*.json   # same machine only
deep-diff-forge eval                          # score the labelled corpus in fixtures/eval
```

- **`bench`** runs nine workloads on inputs generated from fixed parameters, so every run
  measures the same bytes:
  - `parse_seal`, `rank_with_evidence` and `outline_page` on a 200-file patch;
  - `rank_long_line`: one added line of 4 MiB made of near-matches of the evidence rules'
    words, the costliest input per byte to rank;
  - `rank_many_fragments`: 64 hunks of about 60 KB, each a sum of thirty thousand terms, so
    as many tokens as source of that size gets. It was the costliest input per byte for the
    parser that once read Rust hunks; the reader of tokens reads each of these hunks whole,
    and the workload holds that cost;
  - `findings_verify` and `findings_by_address`;
  - `structural_rust`;
  - `daemon_parallel` (four clients through the daemon's own request path).

  Each workload warms up once, then runs 15 repetitions (5 with `--quick`). The receipt names
  the build profile and the host, and each workload's input SHA-256, minimum, median, p95 and
  throughput.
- **`scripts/bench_compare.py`** compares receipts from one machine. It takes the best median of
  each side's runs, and fails a workload that is over 25% *and* over 0.2 ms slower. A workload
  whose input SHA-256 changed is reported as not compared, never as ok or slow. It refuses
  receipts whose profile, CPU count, OS or architecture differ (two machines that agree on all
  four are not told apart), and exits 2 on a malformed receipt.
- **On every pull request,** a required `bench` job builds the merge base and the head on
  one runner and alternates them three times. The weekly campaign keeps a full run's receipt.
- **`eval`** scores the engine against `fixtures/eval/MANIFEST.json`, where every case has its
  file's SHA-256, its origin (a CVE, GHSA or RUSTSEC shape, a real-world shape, one of this
  project's findings, or `synthetic:` with its reason) and its labels:
  - ranking pairs (`must_outrank`), including hard negatives, where a risky-looking file must
    *not* outrank;
  - code evidence (expected items; any other item is a false positive);
  - each finding's expected verdict.

  A limit the engine has today is declared (`known_miss` for a ranking pair or an evidence item,
  `known_false_positive` for an evidence item). It is counted as wrong in its metric and listed
  in the receipt's `declared` (`known_misses` counts only the evidence `known_miss` items), but it does not fail the run; it fails once the engine gets it
  right, so the label has to move. The manifest is refused (exit 4) when a file no longer matches
  its hash, a file in the corpus is named by no case or is a symlink, a case has no origin, a
  repeated id or a field its kind does not read, or a ranking pair is not two distinct paths. `eval` prints `deep-diff-forge.eval.v0` and exits 7 when a case
  fails, with `refused: N cases failed (eval)` on stderr after the receipt; a reader that closes
  stdout early still gets exit 7.

  **Measured 2026-11-18 on the integrated tree (release 0.8.4 with the integrity signals and the agent hooks), 56 cases** (14 ranking, 39 evidence, 3 findings; 112 declared misses). The two items of a second section for one path (IR7-H-2) went from declared misses to expected when the sections of one path that Git applies in order were read as one file (precision 179 / 205 on 55 cases); the integrated review's two false items of that reading (CI1) were then added as one case of two declared false positives, so precision is 179 / 207. **Measured again for 0.10.0** (same 56 cases; 105 declared): the parser refuses hunks plain `git apply` places elsewhere than their header says, so 18 sections of nine evidence cases moved with their 16 expectations to `fixtures/eval-refused-by-parser/`, where a test holds each refused, and 29 gained a blank line of context with their items unchanged: precision 170 / 195, recall 170 / 247. **Measured again 2026-11-22, 58 cases** (41 evidence; 106 declared): a case of four privilege grants written with quotes, which 0.9.0 missed, with three near misses, and a case of the seventeen Python assertions 0.9.0 labelled `trivial_assert` though they can fail (hard negatives) beside four ended forms and one declared miss (`assert True or ready`): precision 178 / 203, recall 178 / 256. **Measured again 2026-11-23, 59 cases** (42 evidence; 106 declared): `evidence/secret-forge-tokens`, eight token forms releases 0.5.0 to 0.9.0 missed (six GitLab prefixes, the routable GitLab form and a fine-grained GitHub token) with five near misses, moved precision from 178 / 203 to 186 / 211 and recall from 178 / 256 to 186 / 264.
  Three evidence cases were added for the integrity signals, one of them a declared miss, and
  one case of twelve declared misses: ways an agent makes a gate pass without doing the work
  (a tautological assertion, a deleted assertion, an early `return` in a test, an error
  replaced by a default, a special-cased input, an edited snapshot, a mocked subject), which
  no rule sees yet, and four cases from an adversarial review of these signals (nineteen items
  its fixes now catch, eight declared misses and one declared false positive; a second false
  positive, a panic path in a `#[test]` function, is declared because a test attribute is not
  proof of test code), and one case from an independent review (ten items its fixes now
  catch, thirteen declared misses, most of them respellings in Python, JavaScript and Go,
  where no parser backs the token rules, and three declared false positives: a method named
  `expect` that is not `Result`'s, twice, and a Python name that begins with
  `unittest.skip`), and one case for raw identifiers (`x.r#unwrap()`; six items), and one
  case from a second independent review and the ninth round of the first (eight items its
  fixes now catch, one declared miss and one declared false positive). Seven false
  positives are declared since 2026-11-14 that were not reported before: six panic paths in
  `#[cfg(test)]` code of a file the patch creates (the parser no longer withdraws them: see
  `panic_path_added` under *Risk ranking*; each carries `test_likely`), and a conditional
  skip of which only the reason changed. That is why precision fell from 69 / 75. The last
  case is from a third independent review, after which the parser was taken out of the
  evidence path (five items found by reading the tokens of a hunk across its lines; three
  declared misses, all changes that add no token to the construct). With that change three
  constructs inside a macro's body went from declared misses to found, two items the parser
  found became declared misses (a function of the patch's own type passed as a value, and a
  call after `/* /* */ // */`), and a conditional skip of which only the spacing changed
  became a declared false positive, so precision went from 77 / 90 to 81 / 95 and recall
  from 77 / 119 to 81 / 125. Then the code view was made to read Rust as the compiler's lexer
  does: the two calls after `/* /* */ // */` are found again, and a new case holds what a
  nested block comment and a character literal with a long escape had hidden from every Rust
  rule, the released ones included (fourteen items, all found), so precision was 97 / 111 and
  recall 97 / 139. Last, a Rust hunk became a set of whole views whose findings are united,
  and a case from the fifth independent review and round 12 of the first holds what that
  closes (nineteen items: constructs with a comment between their tokens after a string over
  lines, raw identifiers after white space past ASCII, a line inside an existing block
  comment, a shebang line, `k#r"\"`, the line that closes a string opened above its hunk),
  what it costs (two declared false positives, one for each further reading) and what stays
  open (four declared misses: a text only editions before 2021 read as the compiler does, and
  a string opened by an earlier hunk), so precision was 116 / 132 and recall 116 / 162. Then
  nothing about a hunk's place or start was taken on the patch writer's word any more, and a
  case from the sixth independent review, round 13 of the first and a review of the port of
  the Rust scanner to the released line holds what that closes (seventeen items: hunks whose
  header says `+1`, a file created as several hunks, two hunks of added lines that are read
  together and each on its own, a hunk between two strings over lines,
  a string whose text opens more comments than it closes, and the full-context forms of two
  constructions a bare fragment cannot show), what it costs (three declared false positives,
  two of them items release 0.8.3 reports in the text of a string, kept so that nothing that
  release reports is lost) and what stays open (eight declared misses: a fragment that
  begins in a string and shows no sign of it, one that begins two comments deep, two hunks
  that touch behind a false position, and four spellings of a suppressed lint); one declared
  miss of the case before is found, so precision was 134 / 153 and recall 134 / 187. Then no
  number of a hunk header decided what is found any more and the edges of a hunk side were
  read, and a case from the seventh independent review and round 14 of the first holds what
  that closes (twenty items: a script under false header numbers, a raw string whose text
  holds `"#`, hunks joined on the hint of either number, a call cut by hunks that nothing
  joins, the released signals' paths with white space and line ends in them, a second
  section for one path), what it costs (two declared false positives: a call of a function
  named `expect` at the start of a hunk, and a text file read as a script) and what stays
  open (five declared misses: a second section that goes on in a string the first opened, an
  assertion cut after its bracket, a raw string past four counts of `#`, a spawn through an
  alias, a Python call with a space before its dot); three declared misses of the case
  before are found, so precision was 158 / 179 and recall 158 / 213. Then the end of a hunk
  side was read only where it is evidence by itself and its beginning not at all (a return
  type named `Command` at the end of a line had been a spawn): the case gained two expected
  items and nine hard negatives, lines of honest code that end or begin with a word a
  construct is known by, and four items went back to declared misses (a call cut after
  `.unwrap` by hunks that nothing joins, and its forms), so precision was 156 / 176 and recall
  156 / 215. Then the end of a side was read at one form only (round 17) and at none (round
  18: a hunk side is read as text that ends there), and two cases from the ninth and tenth
  independent reviews and rounds 16 and 17 of the first were added (released paths in angle
  brackets, also after a keyword; `#[ignore(…)]`; hard negatives where a form at the end of
  a side goes on as something else, in a macro's braces and after a range; declared misses
  and false positives of hunks at old position 0, of spellings and of far joins), so
  precision was 171 / 194 and recall 171 / 252. Then a skip was read only where the side
  holds its attribute whole (but for a seventeenth nested `cfg_attr` list) and the keywords
  before a bracketed path became the ones rustc compiles there (round 19), and a case from the eleventh independent review and round 18 of
  the first was added (six expected spawns after `&mut`, `become` and `do yeet`; two hard
  negatives, `#[ignore(` at the end of a side in a macro's braces; a declared miss,
  `ignore = "…"` above an unchanged `)]`; a declared false positive, a side that begins at
  `<Command>::new();` under an unseen `Vec::`), so precision was 177 / 201 and recall
  177 / 259. Then the two reviews of round 19 were answered by declaring, with no code: a case
  from both (a declared miss, `ignore = "…"` above `)]` where plain `git diff` ends a hunk; two
  declared false positives, both a writer's: a seventeenth nested `cfg_attr` list with no
  `ignore`, and `mut <Command>::new` in a macro that discards it), so precision was 177 / 203
  and recall 177 / 260; with the sections of one path read as one file (2026-11-18) they are
  179 / 205 and 179 / 260. Declared
  misses lower recall on purpose until rules
  catch them; the
  2026-10-31 measurement on 34 cases was precision 21 / 22 and recall 21 / 28:
  The case `evidence/rust-code-view-20261114`, added in 0.8.4 with seven expected items, is in this corpus with fourteen (the same patch, read by the integrity signals as well); the 0.8.4 measurement on 35 cases was precision 28 / 29 and recall 28 / 35, and before it, on 34 cases (2026-10-31), 21 / 22 and 21 / 28.
  On 61 cases (0.11.0, 2026-11-23: `evidence/cargo-lints-suppressed` and
  `evidence/rust-allow-in-comment-and-string` added for the readings of F2b) precision is
  195 / 220 and recall 195 / 272, with 105 declared misses; the floors stay.

  | Metric | Right / total | Floor |
  | --- | ---: | ---: |
  | ranking pairs | 12 / 15 | 80.0% |
  | evidence precision | 195 / 220 | 88.2% |
  | evidence recall | 195 / 272 | 70.5% |
  | findings verdicts | 33 / 33 | 100% |

  The receipt (`--json`) also splits evidence precision and recall by signal (`by_signal`) and by
  the language of the item's file (`by_language`, the engine's own language reading, `none` where
  no language applies); each adds up to the two evidence rows above, and the text summary prints
  the four metrics only. Measured 2026-11-23 on 61 cases:

  | Signal | Precision | Recall | Cases |
  | --- | ---: | ---: | ---: |
  | `assertions_removed` | 0 / 0 (-) | 0 / 1 (0.0%) | 1 |
  | `build_hook` | 1 / 1 (100.0%) | 1 / 1 (100.0%) | 1 |
  | `check_suppressed` | 14 / 14 (100.0%) | 14 / 15 (93.3%) | 7 |
  | `ci_privilege` | 5 / 5 (100.0%) | 5 / 5 (100.0%) | 2 |
  | `dynamic_eval` | 7 / 7 (100.0%) | 7 / 11 (63.6%) | 7 |
  | `error_defaulted` | 0 / 0 (-) | 0 / 2 (0.0%) | 1 |
  | `error_swallowed` | 12 / 12 (100.0%) | 12 / 15 (80.0%) | 6 |
  | `expectations_changed` | 0 / 0 (-) | 0 / 1 (0.0%) | 1 |
  | `input_special_cased` | 0 / 0 (-) | 0 / 1 (0.0%) | 1 |
  | `module_mocked` | 0 / 0 (-) | 0 / 1 (0.0%) | 1 |
  | `network_fetch` | 17 / 19 (89.5%) | 17 / 22 (77.3%) | 12 |
  | `panic_path_added` | 43 / 58 (74.1%) | 43 / 55 (78.2%) | 18 |
  | `process_spawn` | 38 / 41 (92.7%) | 38 / 51 (74.5%) | 12 |
  | `secret_like` | 9 / 9 (100.0%) | 9 / 9 (100.0%) | 2 |
  | `stub_added` | 6 / 6 (100.0%) | 6 / 9 (66.7%) | 8 |
  | `test_removed` | 6 / 6 (100.0%) | 6 / 6 (100.0%) | 4 |
  | `test_skipped` | 15 / 19 (78.9%) | 15 / 30 (50.0%) | 14 |
  | `trivial_assert` | 14 / 14 (100.0%) | 14 / 29 (48.3%) | 13 |
  | `unsafe_added` | 8 / 9 (88.9%) | 8 / 8 (100.0%) | 5 |

  | Language | Precision | Recall | Cases |
  | --- | ---: | ---: | ---: |
  | `go` | 4 / 4 (100.0%) | 4 / 8 (50.0%) | 5 |
  | `javascript` | 13 / 13 (100.0%) | 13 / 23 (56.5%) | 10 |
  | `none` | 17 / 18 (94.4%) | 17 / 21 (81.0%) | 5 |
  | `python` | 12 / 13 (92.3%) | 12 / 29 (41.4%) | 14 |
  | `rust` | 137 / 159 (86.2%) | 137 / 178 (77.0%) | 23 |
  | `shell` | 6 / 7 (85.7%) | 6 / 7 (85.7%) | 6 |
  | `yaml` | 6 / 6 (100.0%) | 6 / 6 (100.0%) | 2 |

  A case whose patch the parser refuses is listed in the receipt's `refused` with the parser's
  reason and fails the run (exit 7); its items are not scored, and every other case is still
  scored and the receipt printed (before 0.10.0 the first such case ended the run with exit 4 and
  no figures). The test `the_figures_in_the_documents_are_the_measured_ones` fails when the four
  rows of the table above, or the evaluation gate's sentence in `docs/TESTING_GOLD_STANDARD.md`,
  differ from what the run measures.

  `crates/deep-diff-forge-cli/tests/eval.rs` holds these floors (the measured values) and fails
  on any undeclared miss. The corpus is written by the rules' author and is small: it is a
  regression gate and an honest list of known limits, not a measure of review quality on
  real-world code. The declared misses are a removed authorization check, a GitHub Actions
  script injection and a lockfile registry swap (ranking), and aliased imports, `window.fetch`,
  a fullwidth `ｅval`, a triple-quoted f-string, bash `/dev/tcp`, an assembled call and a
  JavaScript placeholder that exists only as a message string (evidence), plus one false
  positive (a heredoc read as code).

See also: [`PERFORMANCE_AND_NOVELTY`](../PERFORMANCE_AND_NOVELTY.md) · [`PLAN_AGENT_EVIDENCE_AND_OBSERVABILITY`](../PLAN_AGENT_EVIDENCE_AND_OBSERVABILITY.md).
