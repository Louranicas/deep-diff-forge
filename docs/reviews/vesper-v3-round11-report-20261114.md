# Vesper V3 round 11 review (2026-11-14)

Reviewed: committed snapshot `2824a8b..9c20ea0` on `agent/integrity-signals`, using the frozen round-11 release binary named in the request. I wrote only this report in the repository.

## Verdict

**NO-GO.** The parser rewrite closes VD3R10-M-1, removes IR3-H-1's hostile parser cost, and closes the parser blind spots in IR3-M-2 through IR3-M-5 and IR3-L-4. The supplied hostile parser cases now complete in effectively zero time; the selected-hunk reproduction is local again; the graph tests pass; and the corpus matches the frozen binary.

I found one new Medium resource finding. The token window is bounded, but the result accumulator is not: it retains every non-adjacent duplicate match until the end and conditionally sorts them. An alternating `unwrap`/`todo` line therefore turns a constant two-item answer into an input-sized allocation and an `O(n log n)` comparison sort. A 16 MiB patch peaked at about 97 MiB RSS. This breaks the strict linear-cost claim and the module's stronger statement that memory is only the short window.

## Prior Vesper finding

### VD3R10-M-1 — closed

I reran `/tmp/vesper-v3r10-selected-cost.patch` and selected the cheap hunk by address `e6771adb0ea2d0c6939e1def8c387e877a0ffe31d5b583ea51673eddff03001a`. The frozen round-11 release binary returned exactly that hunk in 0.00 seconds user/real time and about 4.2 MiB RSS. The unselected hostile hunk was not scanned. By reading, selected `hunks` obtains labels with `hunk_signals` for only the shown hunk; the whole-file `Scans` path remains for outline/all-hunks work.

## Third independent review findings

- **IR3-H-1 — closed.** The parser, its square/byte budgets, and its evidence-path clock are deleted. `amp8k.patch` and `q22k-2629.patch`, formerly about 15 and 118 seconds, each ranked in 0.00 seconds in my release runs.
- **IR3-M-1 — not closed as detection, but honestly converted to a declared regression; accepted.** Both `at1.patch` and the former positive control `at1ctl.patch` now produce no `test_skipped`. The corpus declares both misses. This is the stated changed-token boundary rather than a grammar or comparison bug. I accept it as a measured best-effort scanner limitation, not as a claim that these changes are safe.
- **IR3-M-2 — closed.** The five supplied `macro_rules!` cases now report, respectively, `panic_path_added`, `trivial_assert`, `stub_added`, `test_skipped`, and `panic_path_added`, with hunk basis where split over lines.
- **IR3-M-3 — closed for the reported reproduction.** `gc1`, `gc3`, and `gc4` now report their panic, assertion, and skipped-test evidence. The separate compiler/code-view disagreement for a genuinely nested Rust block comment remains declared below.
- **IR3-M-4 — closed.** The supplied assertions containing line/block comments now report `trivial_assert`, including the change to an existing multiline assertion.
- **IR3-M-5 — closed.** The supplied turbofish method calls now report `panic_path_added`, including the existing-file `tf5` case.
- **IR3-L-1 — closed.** This is the same behavior as VD3R10-M-1 above.
- **IR3-L-2 — closed for current behavior.** The current README and API describe the token reader, removed parser values, changed-token boundary, and local selected-hunk scan. Remaining names in tests, changelog, and review documents describe superseded historical behavior. I found no live graph dependency on the syntax crate or current consumer of a removed schema member.
- **IR3-L-3 — not closed, but honestly declared; accepted.** `un1.patch` remains silent and is now a corpus declaration under the changed-token rule.
- **IR3-L-4 — closed.** Both the argument-bearing `#[tokio::test(...)]` and multiline `#[test]` reproductions report `test_removed`.

## New finding

### VD3R11-M-1 — the bounded token window feeds an unbounded match vector and an input-sized sort

`Found::note` merges only when the immediately preceding item has the same `(line, signal)`. Otherwise it appends an `Item`. If any later item is out of signal order, `Found::sorted` sorts the entire raw vector by `(line, signal)` and only then deduplicates it. Thus a single added line alternating two recognized constructs, such as:

```rust
x.unwrap(); todo!(); x.unwrap(); todo!(); // repeated
```

retains one `Item` for every construct even though the externally visible answer has only two evidence items. Because signal order alternates, it also sets `unordered` and runs `sort_by_key` on that vector. This contradicts `rust_tokens.rs`'s statement that "the memory is the window, whatever the text"; the short token window itself is not the problem.

I generated `/tmp/vesper-v3r11-sort-{1,2,4,8,16}m.patch`, each with one added Rust line and the alternating text above, then ranked them with the frozen release binary. Every run returned exactly `stub_added` and `panic_path_added` on line 1. `/usr/bin/time` measured:

| Approximate patch size | User seconds | Peak RSS |
|---:|---:|---:|
| 1 MiB | 0.04 | 10.7 MiB |
| 2 MiB | 0.08 | 18.5 MiB |
| 4 MiB | 0.13 | 33.8 MiB |
| 8 MiB | 0.26 | 49.8 MiB |
| 16 MiB | 0.52 | 96.5 MiB |

The elapsed measurements remain benign and close to linear over this range, but the committed algorithm conditionally performs a comparison sort over `n` raw matches, so the claimed linear worst-case cost does not hold by construction. The memory growth is directly reproduced. It also sits outside the daemon's byte admission accounting: admission is weighted by request/patch bytes and explicitly permits two 64 MiB requests together, while this evidence path can add several times the input size in `Item` storage before deduplication.

Recommended fix: coalesce matches online into the finite signal set for the current hunk line (merging basis/hint there), flush when the line advances, and handle a late `cfg_attr` result without retaining every duplicate construct. The final result should never need a sort proportional to the number of raw matches. Add a regression that repeats alternating constructs on one line and asserts bounded retained items, not only bounded token-window length or elapsed time.

## Claims

1. **One input, one answer, at a cost linear in the input — broken in its cost part.** Determinism and the absence of parser, evidence clock, and evidence budget held by reading and reproduction. VD3R11-M-1 supplies an input-sized comparison sort, so the stated worst-case linear cost is false. The module's window-only memory claim is also false.
2. **Changed-token constructs are found across spacing/lines, with the declared exceptions — held for what I tested.** I reran the independent review's macro bodies, grammar-disagreement cases, commented assertions, attributes, and turbofish calls. I did not exhaust all valid Rust spellings or formally prove the 64/72-token boundaries.
3. **Nothing is withdrawn — held.** There is one token reader and no parser answer or old-side proof that removes an item. The hint only annotates panic evidence.
4. **The corpus says what the binary does — held.** Frozen release `eval --corpus fixtures/eval --json` returned 46 cases, no failures, precision 81/95, recall 81/125, 44 known misses, and 61 declarations.

## Declared regressions

- **Changes that add no token to the construct (`at1`, `at1ctl`, `un1`) — accept as declared.** These are important misses, particularly the conditional skip, but the boundary is simple, documented, included in recall, and no longer presented as parser coverage. I would revisit them as a separate old/new token feature rather than restoring a proof/parser path.
- **A nonstandard owner's function passed as a value (`it.map(X::unwrap)`) — accept as declared.** Reporting every such path without statement/context tracking also reports imports and non-values. Direct calls and standard `Result`/`Option` values remain covered; the omission is explicit in README and corpus history.
- **Plain code after the nested block-comment spelling — accept as declared.** It is a real valid-Rust miss inherited from the released code view, clearly documented and reported to that component's owners. It should be fixed in the Rust code view, but I do not require this token-reader commit to duplicate comment semantics.
- **Spacing-only rewrites inside a multiline skip/assertion — accept.** This is conservative evidence noise, does not hide a construct, and is the natural counterpart of anchoring evidence on any changed token in the construct.

## Other checks

- `git diff --check 2824a8b..9c20ea0` passed. HEAD was `9c20ea097f91d1bd74f6d9504986d1e469aaa6f2` on `agent/integrity-signals`.
- The hive slot reported `free` immediately before Cargo. In `fedora-toolbox-44`, with `CARGO_TARGET_DIR=/tmp/vesper-v3r11-target`, `cargo test -p deep-diff-forge-graph` passed: 168 unit tests, 6 integration tests, and doc tests.
- The frozen release binary SHA-256 was `6f52caf66ad6ba78f55e7802ac6edf67adab81b44e05a1e5f1a1347fcea35245`; debug was `3059674b91f857571cf3694c2f00df3fa5d8ca82be4b0f54ca62f95ed5fccd05`.
- I confirmed `test_likely` is carried as metadata and is not consulted by signal scoring or ordering in the changed graph/view paths.

## Not examined / limitations

- I did not run the full workspace suite, Clippy, Miri, a fuzz campaign, or the complete local gate.
- I did not run a 64 MiB alternating-match patch or concurrent maximum daemon requests; the 1–16 MiB memory measurements are direct, while the maximum/concurrency consequence follows from the source and daemon admission limits.
- I did not rerun all 221 reproduction patches or the 150-commit historical comparison; I used the supplied representative IR3 cases, hostile parser cases, corpus, and focused source reading.
- I did not start a persistent daemon or exercise MCP/TUI/SDK transports live.
- I did not review unrelated language rules or changes outside `2824a8b..9c20ea0`, and I did not modify refs, remotes, index, configuration, or repository files other than this report.

STATUS: COMPLETE
