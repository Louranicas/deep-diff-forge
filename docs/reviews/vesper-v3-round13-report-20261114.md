# Vesper V3 round 13 review (2026-11-14)

Reviewed: exact committed snapshot `5810cb5..8f290e3` on `agent/integrity-signals`, using the frozen round-13 debug/release binaries named in the request. I wrote only this report in the repository; source tests used a `git archive` of the exact commit and reproductions were under `/tmp`.

## Verdict

**NO-GO.** VD3R12-M-1, VD3R12-M-2, VD3R12-L-1, and IR5-H-1 are closed in their reported forms. IR5-M-1 and IR5-M-2 are also fixed. Whole views remove the byte-union defect cleanly, the requested depth-one COMMENT view recovers the existing-comment constructions, the conditional STRING view recovers the supplied honest-string case, and the compiler-style shebang path recovers the hidden spawn.

I found one new Medium, VD3R13-M-1. The declared “comment depth two or more is read as one” limit is not only a source of early exposure/false positives as the README and module head say. Once a depth-one COMMENT view closes early, comment text it wrongly exposes can create lexical state that hides real code after the actual later closer. A small compiler-valid patch at true comment depth two hides a `process_spawn` from every round-13 view. This is a deliberate evasion rather than an ordinary edit, and the depth-two condition is declared, so I rate it Medium rather than High. The stated consequence—“shows code earlier, never later”—is nevertheless false and was the basis on which the deeper-start case was accepted.

I found no new High or Low finding. The whole `d4dbc91..8f290e3` branch remains NO-GO pending VD3R13-M-1.

## Required closures

### VD3R12-M-1 — closed at comment depth one

For every changed side beginning below line 1, `rust_hunk_views` now creates a whole `RustStart::Comment` reading in addition to the code reading. Each side carries its own start and lines. Findings—not bytes—are united after each view's token reader and line rules have independently run.

The frozen round-13 release binary recovers `process_spawn` on line 12 of `two-hunk-comment.patch`, where round 12 is silent. The new corpus/regressions cover both a hunk beginning inside an existing comment and a later hunk whose omitted prefix opened the comment. This closes my requested one-open-comment construction.

It does not safely generalize to an unknown deeper comment state; that is VD3R13-M-1.

### VD3R12-M-2 — closed

At a side whose first file line is line 1, `RustStart::FileStart` applies the compiler's shebang decision before lexical scanning. It strips an initial BOM for the decision, distinguishes `#![…]` inner attributes by the first non-whitespace/non-ordinary-comment token, follows nested comments while finding that token, and treats doc comments as tokens as the compiler does.

The frozen round-13 binary reports `process_spawn` on line 2 of the supplied `shebang.patch`; round 12 reports nothing. The focused tests cover comments, doc comments, a bracket on a later line, BOM, and comment/string openers in a true shebang. I found no disagreement with the stated rustc 1.99.1 rule.

### VD3R12-L-1 — closed

The head of `rust_tokens.rs` now describes nested Rust block comments as lexer-followed state and lists the remaining hunk-start ambiguities. The stale sentence saying the view ends at the first `*/` is gone.

The replacement sentence for depth greater than one overclaims the consequence; that is part of VD3R13-M-1, not a reopening of the old stale statement.

### IR5-H-1 — closed

Rust no longer forks/resumes and unions code bytes. `rust_side` produces one exact whole view from one stated start. `view_matches` runs the token reader and applicable line rules separately for each view. `unite` merges only hits with the same hunk position and signal, keeps the highest basis, and preserves the CODE view's `test_likely` answer when CODE found the item. No token stream contains bytes from two readings.

Frozen-binary checks:

- `union-hide.patch`: round 13 reports `panic_path_added`, `stub_added`, `test_skipped`, and `trivial_assert`; round 12 reports none.
- `skip-hide.patch`: round 13 reports `test_skipped`; round 12 reports none.
- `mod-union.patch`: round 13 reports `panic_path_added`; round 12 reports none.

The corresponding graph regressions passed. The former false positives from comment/string text are separately declared in the corpus.

## Other fifth-review findings

### IR5-M-1 — closed for `k#`; the raw-lifetime compatibility limit is stated

The unsupported `k#name` arm is removed. `kraw-rest.patch` changes from no items in round 12 to all six expected round-13 items, including both `process_spawn` instances, `unsafe_added`, `test_skipped`, `trivial_assert`, and `panic_path_added`.

The pre-2021 raw-lifetime case remains silent (`rawlt-rest.patch`) and is now explicitly listed as an edition compatibility limit. That is consistent with the branch's stated 2021/2024 lexer target.

### IR5-M-2 — closed

`without_raw_prefixes`, the token reader, and the shebang rule use the same `rust_white` definition, including U+0085, U+200E, U+200F, U+2028, and U+2029. `lrm-raw.patch` now reports its two panic paths and skipped test; round 12 reports none. The focused tests cover all five non-ASCII whitespace characters and negative neighboring characters.

### IR5-L-1 and IR5-L-3 — closed

The scanner mutation gaps named by the fifth review have direct unit controls, and the commit reports all 51 listed mutations caught. The `Found` comment now accurately names both late `cfg_attr` completion and a path call reaching back across generic arguments. The relevant tests passed.

## New finding

### VD3R13-M-1 — a depth-one COMMENT view can hide real code after the next comment closer

The branch declares that a side which truly begins two or more block comments deep is read as one deep. README and `rust_tokens.rs` characterize this as conservative: it “shows code earlier, never later.” That conclusion does not follow once the early-exposed comment text is lexed as code. It can open a line comment, string, or other carried state and thereby hide code that the true deeper reading exposes after a later `*/`.

Reproduction: `/tmp/vesper-v3r13-nested-depth.patch` and its complete source `/tmp/vesper-v3r13-nested-depth.rs`.

The real source has two open block comments before the hunk:

```rust
/* outer
/* inner
// */ //
// */ pub fn evil() { std::process::Command::new("true").status().ok(); } /*
*/
fn main() { evil(); }
```

`rustc 1.99.1 --edition=2021 --emit=metadata` accepts it. In the compiler's reading, the first visible `*/` closes the inner comment; the leading `//` and trailing `//` remain comment text. On the next line the second `*/` closes the outer comment, so `pub fn evil` and its process spawn are compiled; the final `/*` is closed by the following line.

Round 13 has only these applicable views:

- CODE treats both visible lines as `//` line comments and sees no spawn.
- COMMENT starts at depth one. On the first line its hypothetical comment closes at the first `*/`; the trailing `//` then becomes a real line comment in that view. The view begins the next line in code, so its leading `//` hides the second closer and the compiled function.
- STRING is not selected because the CODE reading ends in code.

Both frozen round-12 and round-13 release binaries report no integrity item for the patch. Thus no view is a byte mixture and every view is exact from its stated start, but the set of starts contains no view that exposes the real construct.

This requires deliberately shaped text inside nested comments, and the unknown deeper start is declared; it is not an ordinary false negative. It is still a general hiding primitive for all token integrity constructs, and the documentation promises the opposite consequence. The new fuzz properties cannot see it: the code-view oracle checks a view against its stated start, not whether the chosen starts include a reading that preserves actual code under unknown omitted prefix state. The token fuzz rule likewise inserts a balanced comment between known tokens; it does not generate a fragment beginning at unknown comment depth with adversarial closer text.

Recommended resolution: at minimum, state honestly that depth greater than one can hide later code and add this compiler-valid case to the corpus. If the branch intends conservative security rather than a declared evasion, one extra fixed depth is not a sound general fix: the same construction moves to the next depth. Prefer true prefix state for Git-backed selections. For bare patches, a linear evidence-specific resynchronization design would need to preserve separate findings from plausible closer states without merging their bytes or allowing early-exposed text to suppress all later starts; its cost and state bound should be explicit and fuzzed. Until one of those policies is chosen, I would not merge the “never later” claim.

## Claims

1. **No view is a mixture — held.** `rust_side` scans every side whole from exactly one `RustStart`; `view_matches` consumes one `Views`; `unite` combines `Hit` records only. I reproduced the three IR5-H-1 cases now found. VD3R13-M-1 is an insufficient-start-set defect, not mixed bytes.
2. **Monotone in views — held.** `unite` is an ordered union by hunk position and signal with maximum basis. Adding a view cannot delete an existing item. CODE's `test_likely` answer wins when CODE found the item; otherwise all finding views must agree on the hint. The focused union/order/hint regressions passed.
3. **Rounds 11 and 12 stand — held for the stated implementation properties.** There is one deterministic answer per input, no parser, clock, withdrawal, or syntax-unchecked path; other languages still use the untouched shared scanner path; and Rust performs at most three linear scans of a side. The exact graph cost regressions passed. The prior assumption that a depth-one comment start is always conservative is broken separately by VD3R13-M-1.
4. **The corpus says what the binary does — held.** The frozen release binary returned 48 cases, no failures, evidence precision 116/132, recall 116/162, findings 33/33, ranking 12/15, and 46 known misses. The independent-review-5 declarations include the four stated new-case misses and two known false positives.

## Answer to B: which view policy I accept

- **CODE and FileStart:** accepted. Each side uses its own line numbers; a true line-1 side gets the compiler-style shebang path, otherwise CODE. A patch that lies about line numbers necessarily controls the evidence input and remains a trust boundary.
- **COMMENT at depth one:** accepted and it closes VD3R12-M-1, but not as a sound stand-in for arbitrary comment depth. The current depth-two declaration must not say “never later”; VD3R13-M-1 shows later code can be hidden.
- **Conditional STRING:** accepted for this round as a measured precision/recall trade, not as a security-complete reading. It recovers the supplied `honest-string.patch` panic path (round 12 is silent) while avoiding the measured 481 false items of applying STRING to every fragment. The remaining 93 real fragment items, the supplied `two-hunk-string.patch`, and raw-string starts are explicit limits. Wording should cover every case where CODE does not end in a string/raw string, not imply that only a final code state can miss.
- **Old/new sides, renames, and several hunks:** accepted. Each side is read from its own lines and change flag; each hunk is independent, so a prior hunk cannot contaminate a later one. That independence also means omitted true cross-hunk state needs the alternative starts and remains subject to their declared limits.
- **Top-of-file/shebang:** accepted. The hunk-visible no-token case chooses the reading that exposes at least as much as an unseen later inner-attribute bracket could; I found no compiled construct it hides.

## Fuzz and resource assessment

The new fuzz properties are useful for lexer conformance of each stated view, shebang preprocessing, ending-state classification, and stability of token findings around balanced comments/strings. They do not prove that the view-selection heuristic includes the true omitted-prefix state, do not compare end-to-end findings against a complete-file compiler oracle, and do not cover arbitrary initial comment depth; VD3R13-M-1 is a concrete blind spot.

The implementation makes at most three whole readings, tokenizes each once, and unions bounded per-line/signal answers in order. I found no input-dependent rescan or cross-view byte storage. The exact graph suite's two-megabyte hostile tests passed. I did not rerun the 661/331-second fuzz campaigns or the daemon soak. The reported soak flake also occurs with the untouched round-12 binary and the daemon is outside this diff; I see no evidence tying it to this commit.

## Verification

- HEAD: `8f290e3241591520548f11d8bf960af2689d9cb4`.
- Frozen SHA-256: debug `6a9a00dcd324aea2869fb668874ed16a0781cc740b88b3cc0a6274b2b66bfd2f`; release `0d7b9cf4acd155913292acd550939badab388abd3f3a74fe3cbe2c47f57434ec`.
- `git diff --check 5810cb5..8f290e3` reports five trailing-space lines in patch/fuzz fixtures. Inspection shows they are literal test payload lines, not source-formatting defects.
- Source tests ran from `/tmp/vesper-v3r13-8f290e3`, an exact `git archive` of `8f290e3`.
- The hive slot reported `free` immediately before Cargo. Toolbox was launched with the real HOME; only the command inside `fedora-toolbox-44` received the private HOME and the existing Rust toolchain paths. `CARGO_TARGET_DIR=/tmp/vesper-v3r13-target`.
- `cargo test -p deep-diff-forge-graph --locked`: 197 unit tests, 6 integration tests, and doc tests passed.
- Frozen-binary corpus evaluation: 48 cases, no failures, metrics matching the committed manifest.

## Not examined / limitations

- I did not rerun the full 177,957-fragment measurement, 481-commit history comparison, 226 earlier reproduction patches, or every one of the fifth review's 38 patches. I directly compared the two frozen releases on ten representative prior/open constructions and the new nested-depth reproduction.
- I did not run a fresh libFuzzer campaign, independently validate millions of generated cases, or mutation-test the scanner. I inspected the properties and ran their ordinary graph regressions.
- I did not run the full workspace suite, Clippy, Miri, sanitizers, the complete local gate, daemon soak, or tests on non-Linux platforms.
- I did not independently compare rustc editions 2015 through 2024 beyond compiling the new reproduction with rustc 1.99.1 edition 2021. I relied on the committed compiler comparison for the shebang and edition table.
- I did not retest other languages with millions of random lines. By reading, their shared-scanner path is unchanged and Rust dispatches before it.
- I did not force a multi-gigabyte review or very many hunks; the focused hostile-size tests passed.
- I did not modify refs, remotes, index, configuration, or repository files other than this report.

STATUS: COMPLETE
