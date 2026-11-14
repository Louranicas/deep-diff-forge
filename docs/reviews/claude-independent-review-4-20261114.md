# Fourth independent review of the integrity signals: commit 9c20ea0 (2026-11-14)

Reviewer: a Claude subagent that took no part in the work. Commit `9c20ea0` on `2824a8b`,
read from `git archive`. Binaries: the frozen `v3r11` (this commit) and `v3r10` (the commit
before). No `cargo` was run. Scratch and reproductions: `/tmp/claude-ddf-indep-review-4/`.

## Verdict: NO-GO

One high finding, reproduced with a control and compiler evidence: a single ordinary-looking
added line makes this commit report none of the five signals for the rest of the hunk, where
the commit before reported all of them. It is not in the declared list.

The review is incomplete. A batch of further probes was refused by the session's permission
layer and I did not rerun it in another form (see "Not done").

## IR4-H-1 (high, reproduced): a misread character literal blanks the rest of the hunk

**What happens.** `t/h2.rs` is a 29-line new file. Its first line is a constant holding two
character literals and a string. The rest holds one `unwrap`, one `todo!`, two ignored tests
(one by `cfg_attr`) and two trivial assertions.

```
./both.sh t/h2.patch
== v3r10   panic_path_added 4, stub_added 8, test_skipped 14, trivial_assert 16,
           test_skipped 20, trivial_assert 22      (all syntax_fragment)
== v3r11   (nothing)

./both.sh t/h2ctl.patch        # the same file without the constant
== v3r10   the same six items, one line up
== v3r11   the same six items, lexical
```

`rustc --edition 2021` compiles `h2.rs` with 0 errors; the program panics at `h2.rs:4`
("called `Option::unwrap()` on a `None` value", exit 101); the test build reports
`0 passed; 2 ignored`.

A one-line form (`t/a1.rs`, the literals and an `unwrap` on the same line) loses the item on
that line only: `v3r10` reports `panic_path_added` on line 2, `v3r11` reports nothing.

**Cause, from the source.** The code view (`lexical.rs`, `Scan::in_code`, the Rust branch at
about line 685) takes `'` for a character literal only when the closing quote is two bytes
on, or three after a backslash. A literal written with a longer escape is left as code, and
its closing quote is then tried as an opening one. With one byte and another quote after it,
those three bytes are blanked as a literal, and the scanner is one quote out of step: the
content of the next literal is read as code. If that content is a double quote, a string
opens that the source does not have. For Rust an open string is not restored at the end of
the line (`finish`), and if the text then shows a block-comment opener to the scanner,
`in_block_comment` is carried by every reading (`Reading`, `step` line 508) until a closer
appears in the hunk.

`lexical.rs` is unchanged by this feature (`git diff d4dbc91 9c20ea0` shows nothing for it),
so the defect is older than this commit. Until this commit the parser read the raw text and
still found the constructs. Now the code view is the reader's only input, so the commit turns
an old weakness of one-line rules into the loss of all five signals. That is a regression
against `v3r10` and claim 2 does not hold.

**Also seen, not part of this commit.** In my first probe (`t/h1.rs`) `unsafe_added` on line 9
is missing on BOTH binaries, and present on both in its control. So the released one-line
rules lose evidence the same way. That belongs to the owners of the released code; I only
report it. (`h1.rs` is not compiler evidence for the panic: my probe dereferenced an empty
slice and the program died of that, exit 139. `h2.rs` replaces it.)

**Fix, smallest first.** Read a Rust character literal whole: after `'\`, look for the
closing quote within the longest escape (`'\u{10FFFF}'`); otherwise take one UTF-8 character
and require the quote after it. Then a closing quote is never offered as an opener. Add the
case to the corpus. The cost in false positives should be none: the change only stops code
from being blanked.

## Claims

| Claim | Result |
| --- | --- |
| 1. One answer, cost linear in the input | Held on what I measured (below). |
| 2. A construct with its tokens on changed lines is found | **Broken**: IR4-H-1. |
| 3. Nothing is withdrawn | Held by construction (one reader, read in the source); not tested further. |
| 4. The corpus says what the binary does | The numbers match (46 cases, precision 81/95, recall 81/125, declared 61, exit 0). The corpus has no case for IR4-H-1. |

## Cost

User seconds to rank a new file of one unit repeated, 1 MiB then 4 MiB, on one line and one
unit per line. All grow about four times for four times the input.

| Unit | one line | many lines |
| --- | --- | --- |
| `#[cfg_attr(` | 0.03 / 0.13 | 0.09 / 0.37 |
| `x.unwrap::<` | 0.02 / 0.10 | 0.08 / 0.34 |
| `Option::<` | 0.02 / 0.08 | 0.08 / 0.36 |
| `>::unwrap` | 0.02 / 0.10 | 0.09 / 0.39 |
| `#[test] fn a(){` | 0.02 / 0.09 | 0.06 / 0.27 |
| `assert!(` | 0.02 / 0.07 | 0.09 / 0.38 |
| `(((` | 0.02 / 0.08 | 0.19 / not measured |
| `<<<>` | 0.02 / 0.08 | 0.15 / 0.63 |

The `(((` many-lines cell at 4 MiB is not a measurement: the patch has 1,048,577 lines and
is refused by the 1,000,000-line budget. I did not measure memory, the outline, hunks or
cluster views, or the daemon.

## False positives on real history (rank output, items on Rust files)

| History | Commits compared | Rust file diffs | Items v3r10 / v3r11 | Lost | Gained |
| --- | --- | --- | --- | --- | --- |
| This branch, last 150, first-parent diffs | 150 | 424 | 672 / 621 | 51 | 0 |
| forge-warden hub `main`, last 150 | 149 (1 empty) | 221 | 307 / 291 | 18 | 2 |

Every lost item is `syntax_unchecked`, the signal the commit removes. The two gained items
are real `.unwrap()` calls (`doors/mod.rs:312`, `z3_authority.rs:372` at `dcce82c33`). I did
not establish why `v3r10` left them out. Limit: rank prints at most 32 items a file, so this
compares what is printed, not everything found.

## The hint `test_likely`

Read in the source only. Its consumers are the three writers (`cli/src/main.rs:1858`,
`daemon/src/handler.rs:1149`, `tui/src/notes.rs:228`); `rank.rs` and `risk.rs` do not name
it. I found no path to a signal, a score or an order. Not tested with patches.

## Leftovers

`syntax_unchecked`, `syntax_fragment`, `FragmentPlan` and the budgets are named only in the
changelog, the review files and the corpus origins. The remaining tree-sitter mentions are
about the syntax crate's own uses (highlighting, `semantic`). I did not read
`docs/API_AND_IPC.md` or the schemas line by line.

## Weak tests

- The fuzz target's respacing rule runs only on text with no `"`, `'`, `/` or `#` byte (its
  own header says so). It cannot see anything that depends on the code view, and it never
  respaces an attribute, so `test_skipped` and `test_removed` are outside it.
- `rust_char_literals_are_not_strings` covers the two short forms and a lifetime. No test
  holds a literal with a longer escape or a non-ASCII character, and nothing compares the
  code view with a real lexer.
- I did not check whether each guard of the reader has a test that fails without it.

## Declared regressions

Not examined one by one. One observation from the source: the module says the nested-comment
case costs "to the end of that line", while block-comment state is carried over lines by
every reading. The declared reach may be too small. Suspected, not reproduced.

## Not done

- The probes that would say which spellings trigger IR4-H-1 (other escapes, a non-ASCII
  character, formatted spacing) and how far the nested-comment regression reaches were
  refused by the permission layer. Whether `rustfmt` spacing prevents the misreading is
  therefore unknown.
- Suspected from reading, none reproduced: the `Result`/`Option` function-as-value rule for
  a bracketed owner type (`panic_path`, the `opens` branch); a path with a leading `::` in
  `test_attribute`; a word directly after a byte-order mark (the tokeniser takes every
  non-ASCII byte as part of a name); generic arguments longer than `TURBOFISH_TOKENS`.
- Brief items 5 and 7 are only partly covered; the views, the daemon and memory were not
  measured.
