# Vesper V3 round 12 review (2026-11-14)

Reviewed: committed snapshot `7004510..5810cb5` on `agent/integrity-signals`, using the frozen round-12 debug and release binaries named in the request. I wrote only this report in the repository.

## Verdict

**NO-GO.** IR4-H-1 and the released nested-block-comment failure are closed. The new Rust scanner follows the lexer closely, keeps its exact reading, remains bounded in carried state, passes the full graph suite, and restores every signal in the supplied character-literal and nested-comment reproductions. VD3R11-M-1 is also closed by its preceding commit.

I do not accept two of the four declared fragment limits as the final behavior. An existing-file hunk can begin inside a block comment; reading it only as code lets an added `// */ process_spawn /*` line change real compiled behavior while hiding the spawn. A conservative second reading as one open block comment closes that gap with bounded state and linear work. Separately, the first physical line of a Rust file is not an uncertain fragment: `rustc` deliberately strips a shebang, but the scanner reads a `/*` in it as a real opener and hides later compiled code. The frozen binary misses the supplied spawn in a source that `rustc 1.99.1` accepts.

I found one stale module-level limitation statement as a Low documentation defect. Therefore my verdict on the whole `d4dbc91..5810cb5` branch remains NO-GO pending the two Medium fixes.

## Prior findings

### IR4-H-1 — closed

I reran `/tmp/claude-ddf-indep-review-4/t/h1.patch` and `h2.patch` and the released `charlit-hide.patch`. Round 11 reported no evidence. The frozen round-12 release binary recovered all expected items: `unsafe_added`, `panic_path_added`, two `stub_added`, two `test_skipped`, and two `trivial_assert` items in the main character-literal reproduction; the shorter second reproduction likewise recovered all six expected integrity items.

By reading, `lifetime_or_char`, `quoted`, and `char_text` now pass over the complete lexer token, including long hex/Unicode escapes, and distinguish lifetimes and labels. The compiler-exact reading is retained explicitly when the bounded reading set would otherwise evict it. The lexer-oracle fuzz target includes the original failure as a seed and classifies compiler-code bytes independently of the view.

### Released nested-block-comment failure — closed

The frozen round-12 release binary reports both `network_fetch` and `process_spawn` in the supplied `hide.patch`; round 11 reports neither. `rust::Scan::comment` counts nested `/*` and `*/` across lines and blanks through the matching outer closer. The focused and full graph suites include and pass the nested-comment regression.

This closure is for a scan whose starting state is known. VD3R12-M-1 below concerns a different ambiguity: an existing-file hunk whose omitted prefix may already have opened a comment.

### VD3R11-M-1 — closed by `7004510`

`Found` now stores one bounded signal slot per `(line, signal)` answer, with a `BTreeMap` only for late earlier-line results. It no longer retains every raw alternating match or sorts that match list. The graph cost regression passed in the full suite. I did not repeat the round-11 1–16 MiB RSS series.

## New findings

### VD3R12-M-1 — an existing-file hunk is not conservatively read as possibly starting inside a block comment

The scanner resets every hunk to `Open::Nothing`. That is exact for a new file and for a side beginning at physical line 1, but not for an existing-file fragment. The supplied `inside-existing-comment.patch` is a direct case:

```rust
+   // */ pub fn evil(u: &str) { std::process::Command::new("sh").arg("-c").arg(u).status().ok(); } /*
```

In the actual file this line begins inside an existing block comment. Therefore `//` is comment text, `*/` closes the comment, the function is compiled, and the final `/*` reopens the surrounding prose comment. Starting the hunk as code instead treats `//` as a line comment and hides the process spawn. The frozen round-12 binary reports no evidence. The same ambiguity occurs in the later hunk of `two-hunk-comment.patch`; it also reports no spawn.

This is not merely an opener deliberately added in the same hunk or an unbounded language-semantic problem. It is the ordinary missing-prefix state of any existing-file diff hunk, and the source module says uncertainty should err toward visible code. The requested alternative is bounded: for each existing-file side whose hunk begins after line 1, include a reading starting at `Open::Comment(1)` and union its view with the code-start reading. Nested depth above one does not need a separate initial state: the first visible `*/` may close only one real level, but treating it as closing the hypothetical comment exposes code earlier, conservatively. State remains a fixed finite set and work remains a constant multiple of input length.

This can create visible false positives after `*/` text in a string near the start of a fragment. I accept that trade: it is review evidence, the item is visible, and it follows the component's stated preference for a false positive over hidden executable code. Limit the extra reading to genuine fragments rather than new files or sides known to start at line 1.

### VD3R12-M-2 — a real Rust shebang is known file context, but its `/*` hides compiled code

The supplied `shebang.patch` begins:

```rust
#!/usr/bin/env -S cargo run /*
fn main() { std::process::Command::new("sh").status().ok(); }
// */
```

I compiled its source successfully with `rustc 1.99.1 --edition=2021 --emit=metadata`. Rust's front end strips the first-line shebang, so the process call is compiled. The frozen round-12 binary instead feeds the shebang to the token lexer, takes `/*` as an unterminated block-comment opener, and reports no `process_spawn` on line 2. The fuzz oracle uses `Cursor` with `FrontmatterAllowed::No` directly and therefore models the token lexer after front-end preprocessing, not the compiler's treatment of a complete source file; it cannot reject this case.

Unlike an arbitrary hunk-start string state, this context is available and deterministic: the patch identifies a new file and its physical first line. Apply the compiler's shebang rule before the Rust code view (while preserving `#![...]` inner attributes), or otherwise ensure the shebang cannot carry lexical state. Add a compiler-backed regression asserting that a signal on line 2 remains visible when line 1 contains comment/string openers in a compiler-stripped shebang.

### VD3R12-L-1 — `rust_tokens.rs` still says the code view does not nest Rust block comments

The module-level “What it cannot see” list in `crates/deep-diff-forge-graph/src/rust_tokens.rs` says the view “ends a Rust block comment at the first `*/`” and can misread `/* /* */`. That is the defect this commit fixes and contradicts the current implementation, README, changelog, tests, and reproduced behavior. Rewrite it to describe the remaining hunk-start ambiguity rather than the closed lexer defect.

## Claims

1. **Whole-file Rust view preserves every byte the token lexer calls code — held within the stated edition and lexer scope.** The scanner structure follows `rustc_lexer`, the exact reading cannot be evicted, the focused lexer tests passed, and the supplied compiler-valid character/literal cases no longer hide code. I did not find a counterexample in tokenization from a known code start. VD3R12-M-2 is the important boundary the oracle omits: compiler front-end shebang removal happens before that lexer.
2. **Other language views are byte-identical — held.** The shared scanner changes are Rust removal plus an early Rust-only dispatch; the added state member is default and unused by other languages. The supplied old/new code-view comparator found no changed view among 7,770 Python, JavaScript/TypeScript, Go, shell, and YAML lines from 593 reproduction patches.
3. **Cost is linear and memory bounded — held for the implementation inspected and tests run.** Rust carries at most four readings, scans each line a constant number of times, advances through nested comments/raw-string candidates without rescanning consumed bytes, and stores output proportional to the current line. Both two-megabyte hostile-line tests passed. The full graph suite's two-megabyte aggregate cost test completed in about 61 seconds in a contended environment; I found no input-dependent nested rescan or unbounded reading accumulation.
4. **Round 11 claims stand with this input — held apart from the newly rejected declared limits.** There is one deterministic token answer, no parser/clock/budget/withdrawal path, the result accumulator fix remains in place, and the corpus matches the frozen binary. `eval --corpus fixtures/eval --json` returned 47 cases, no failures, precision 97/111, recall 97/139, findings 33/33, ranking 12/15, 42 known misses, and 59 declarations.

## Answer to B: hunk-start limits

- **Block-comment start (two-hunk comment and inside-existing-comment): not accepted.** Add the second `Open::Comment(1)` reading for genuine existing-file fragments. Its false positives are conservative and bounded.
- **Opened string in a prior hunk: accepted as a declared limit for this round.** A complete fix has several possible starting literal kinds and raw delimiters and would spend scarce readings or create much broader noise. The construction requires a deliberately coordinated opener in omitted context; keep it measured and documented rather than claiming safety.
- **Shebang: not accepted.** It is exact first-line compiler context, not unknowable fragment state, and has a narrow fix as described in VD3R12-M-2.
- **Pre-2021 `cr`, frontmatter, and post-Unicode-18 characters:** accepted as explicit compatibility limits. The claimed edition floor and table version are stated. Frontmatter should be revisited when the supported compiler/Cargo-script contract includes it.

## Other checks

- HEAD was `5810cb53554fe6e488418843436989759b7b7a76` on `agent/integrity-signals`.
- Frozen SHA-256 values matched the request: debug `e8be11f50f0f61e349627a839aeaed832cab44892bdf15c49089185874065e57`; release `fb438ddcbfebe1be750a8eede178f627e22b446d71cad19c7b0edacfdb20476a`.
- The hive slot reported `free` immediately before Cargo. With the launcher using the real HOME and `CARGO_TARGET_DIR=/tmp/vesper-v3r12-target`, `cargo test -p deep-diff-forge-graph --locked` passed 182 unit tests, 6 integration tests, and doc tests. The focused lexical selection passed 37 tests.
- The four supplied open constructions all remained silent in both round 11 and round 12, as documented. The two closed released failures changed from silent to the expected evidence.
- `git diff --check 7004510..5810cb5` reports trailing whitespace on all nine lines of `fuzz/seeds/rust_code_view/byte_order_mark_and_crlf`; inspection shows these are intentional CRLF bytes in the named lexer seed. I did not treat that fixture encoding as a defect.

## Not examined / limitations

- I did not run a fresh libFuzzer campaign, independently reproduce the claimed 180 million generated inputs, or mutate the Unicode tables. I inspected the oracle and its seed-only exhaustive character check.
- I did not run the full workspace suite, Clippy, Miri, sanitizers, the complete local gate, or tests on non-Linux platforms.
- I did not rerun all 221 patches through the CLI myself. I used the supplied old/new view comparator over 593 patches, inspected its differences, and directly reran the character, nested-comment, and four open-limit patches.
- I did not repeat the 150-commit historical comparison or independently assess every newly found/lost `test_likely` annotation.
- I did not exhaust every Rust edition error-recovery tokenization, compiler frontmatter variant, or future Unicode assignment. My compiler probe used rustc 1.99.1 and edition 2021.
- I did not modify refs, remotes, index, configuration, or repository files other than this report.

STATUS: COMPLETE
