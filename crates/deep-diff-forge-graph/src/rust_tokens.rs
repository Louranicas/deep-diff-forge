// DDF integration: agent batteries, the integrity signals of a Rust hunk read as tokens.
// Design/status: docs/reference/risk-ranking.md
//! The tokens of one side of a Rust hunk, read once and in order, and the constructs the
//! integrity signals are about, found among them.
//!
//! A side of a hunk is its unchanged lines and its added lines (the new side) or its removed
//! lines (the old side), each as its code view: comments and strings blank, raw identifier
//! prefixes blank. A view is one whole reading of the side from a stated start, and a side can
//! have several (from code; from inside a block comment, a string or a raw string; from the
//! top of a file): the reader reads each on its own and what it
//! finds in any is found. No view is a mixture of two readings. The lines are those of a text,
//! which is one hunk or several that Git must put one straight after the other (a file created
//! as several hunks is one text). This module cuts a view's text into tokens and looks at each
//! token once. A token
//! is an identifier (ASCII letters, digits, `_` and any byte that is not ASCII) or one byte of
//! punctuation, and it knows the hunk line and the byte column it stands at. White space and
//! line ends only separate tokens, so a construct reads the same on one line and over ten.
//!
//! There is no grammar here. Until 2026-11-14 a parser read each hunk on top of rules that read
//! one line, and ten rounds of review found the same three things again and again: a text that
//! the grammar and the compiler read differently; a text on which the parser is slow, where no
//! budget of bytes holds (8 KB of `&)` took 15 s; third independent review, IR3-H-1); and the
//! places the parser is blind in (another macro's body, a `macro_rules!` arm, under an error),
//! where a construct left no trace. A reader of tokens has no tree to get wrong, costs what its
//! input is long, and reads a macro's body like any other tokens.
//!
//! Each construct is found from one word (`unwrap`, `assert`, `todo`, `#`, …) by reading a
//! bounded number of the tokens before and after it ([`TURBOFISH_TOKENS`]). The one construct
//! with no bound of its own, a `#[cfg_attr(…)]` list, is followed token by token as the tokens
//! pass ([`Skips`]), never from each list's own start. Nothing recurses and nothing is sorted.
//! The tokens are held only in a short window, and what is found is kept as the answer (one
//! place for each line and signal, [`Found`]) and not as a list of matches. So the cost is the
//! length of the side, and the memory is the window and the answer, whatever the text.
//!
//! What it cannot see, by its design:
//! - The code view decides what is a comment or a string. Read from where a file starts it
//!   agrees with the compiler's lexer (editions 2021 and 2024; a fuzz target holds it to that
//!   lexer), and a text that Git must put at the top of its file is read that way and no
//!   other. So a patch made with full context (`git diff -U<large>`), every hunk of which
//!   reaches the top of its file, has no unknown start. What the view cannot know is what the
//!   lines above any other text left open. A side of such a text that the change touches is
//!   read from code and also from the further starts the module `evidence` gives it
//!   (`text_hits`): from inside one block comment; from inside a string, and from inside a
//!   raw string, when its reading from code carries anything over a line end; from the top
//!   of a file, when its first line is a shebang. (Every hunk is read once more as release
//!   0.8.3 read it, by that release's rules; this reader does not read that view.) What remains (the figures are from 177,957 fragments cut from 13,779 real
//!   crate files, 25,208 more with their true starts known, and this repository's history of
//!   481 commits):
//!   - A side that began inside a string and that, read from code, carries nothing over a
//!     line end: the last line of the string with code after it and quotes that balance
//!     (`"; call(); // "`). Nothing then says the side began in a string. It takes a writer
//!     who sets out to: of the fragments with known starts none loses an item. Reading every
//!     fragment from inside a string would close this for 457 false items on the fragments
//!     and 212 on the history (3.8% of its items), and is not done.
//!   - A side that began two or more block comments deep is read as one deep, and from its
//!     first `*/` on that reading is wrong: the text it shows too early can open a line
//!     comment or a string of its own and hide code the compiler reads later (`// */ //`,
//!     and on the next line `// */ call(); /*`). None of the fragments with known starts
//!     begins two deep. (Opened by an earlier hunk of a created file, the comment is in the
//!     same text, and nothing is unknown.)
//!   - A side that began inside a raw string opened with no `#` whose closing quote follows
//!     a backslash is read with the string ending in the wrong place. So is one that began
//!     inside a raw string whose own text has quotes with one, two, three and four `#` after
//!     them and that was opened with more: the side is read for the first four counts that
//!     its quotes reach (`evidence::quote_hash_records`), and no real side measured has
//!     more. (Until 2026-11-15 only the first count was read, and `href="#top"` in a page
//!     kept in `r##"…"##` lost every item after the string: seventh independent review,
//!     IR7-M-1, the plain output of `git diff`.)
//!   - Two hunks that touch while neither position of the second says so: Git finds a hunk
//!     by its text wherever its header says it is, no join reads the two together, and a
//!     construct cut in two by them is read in two halves. **A hunk side is read as text
//!     that ends there; nothing is inferred from how it ends.** So such a construct is found
//!     only when one half holds it whole; else it is a
//!     declared miss, which takes a writer who sets out to (one true number of the second
//!     hunk joins the two, and the construct is found). Rounds 15 to 17 read some forms at
//!     the end of a side as the beginning of the construct, each on an argument (no honest
//!     line ends so; every continuation the grammar allows is the construct), and grammar
//!     broke each: a path that goes on
//!     (`const X: u8 = Command::` and `VERSION;`), an assertion whose condition goes on
//!     (`assert!(` and `true` and `&& …`, rustfmt's layout), a sentence that ends on
//!     `todo!`, a range (`0..unwrap::<u8>()`, a free function) and the token tree of a
//!     macro, which takes any tokens (`discard! { value.unwrap:: NOT_A_CALL }` compiles; round
//!     17 of Vesper's review, VD3R17-M-1, and the tenth independent review, IR10-L-3). Since
//!     round 18 no form is read there. Round 18 still read a skip's attribute from the token
//!     after its `ignore` (`#[ignore(`; round 18 of Vesper's review, VD3R18-M-1): since round
//!     19 a skip is read only where the side holds its attribute whole, `#` to `]`, but for
//!     a seventeenth `cfg_attr` list nested in 16 ([`CFG_ATTR_LISTS`]), taken to hold one.
//!   - The beginning of a side is read as the first tokens of its text, and a side's first
//!     token can go on from a line the side does not show. So a bracketed released path at
//!     the very start of a side, `<Command>::new()`, is read as the path even where an
//!     unseen `Vec::` on the line above makes it a type argument: a false `process_spawn`
//!     (eleventh independent review, IR11-L-1, `vecturbo-u0.patch`). It takes a writer (no
//!     formatter breaks a line between `::` and `<`), and reading it is the direction that
//!     detects: not reading a `<` that nothing shown precedes would lose `let c =` and
//!     `<Command>::new(…)` written over two lines.
//!   - A `cfg_attr` list that the side leaves open, on its `ignore` or after it (`ignore`,
//!     `ignore = "…"`, `ignore(…)`, up to its `)]`), is no skip, by the same rule: `ignore`
//!     can go on as `ignore::case`, and in a macro that takes any tokens anything can
//!     follow. This is *ordinary*: rustfmt writes `ignore` or `ignore = "…"` alone above
//!     `)]`, and wherever a hunk ends between the `ignore` and the `)]`, at any context width
//!     (plain `git diff` included), the skip is not read (`-U0`: `cfgu0.patch`,
//!     `cfgeqnew-u0.patch`; three lines of context: the twelfth review's `four-u3.patch`,
//!     IR12-L-1). Rounds 13 to 16 read the bare `ignore` there as a skip and rounds 13 to 18
//!     the one with a reason: these are the places where a later round reports less than
//!     round 14 did (release 0.8.3 reports nothing there).
//!   - The further readings cost false items. From inside a comment: the text after a `*/`
//!     in a string near the start of a fragment (1 on the fragments, 8 on the history). From
//!     inside a string: the text of the side's own strings (60 and 24). From inside a raw
//!     string: none measured. The reading of release 0.8.3, which only that release's own
//!     rules read (the reader of tokens does not): text of strings and comments that its
//!     scanner took for code (2 and 9; and 148 on 603 crates read as created files, where
//!     every other reading is exact).
//!   - Editions before 2021 read three forms otherwise, and there a string can be taken to
//!     end at another place than the compiler ends it: `cr"…"` (the name `cr` and an ordinary
//!     string), `c"…"` (the name `c` and a string: the same bytes are blank), and `'r#name`
//!     (`'r`, `#` and what follows, so `'r#r"\"` holds a raw string).
//!   - Frontmatter (`---` blocks at the top of a file, which rustc 1.99.1 refuses as
//!     experimental) is read as Rust, and so is a character that Unicode assigned after
//!     version 18.0.0.
//! - A name a macro builds, or one bound to another name (`use Option::unwrap as take;`), is
//!   not the word.
//! - A construct longer than its bound is not read (an `#[ignore…]` of more than
//!   [`TURBOFISH_TOKENS`] tokens from `[` to `]` too; a `cfg_attr` list has no bound), and
//!   neither is one the hunk shows only a part of: a hunk side is read as text that ends
//!   there; nothing is inferred from how it ends.
//! - The paths release 0.8.3 keyed a signal on are read as tokens (`Command :: new`,
//!   `process::` and `Command` on two lines); a path through a `use` alias or a glob import
//!   that never writes those names is not, here as in that release.
//! - A change that only takes tokens away adds no token: an assertion or a conditional skip
//!   that becomes one because a line was removed has no token on an added line.

use crate::evidence::{Basis, EvidenceSignal};
use deep_diff_forge_core::{PatchLine, PatchLineKind};

/// The most tokens generic arguments written with `::<` may be, from their `<` to their `>`,
/// both counted, for the reader to pass over them: `x.unwrap::<…>()`, `X::unwrap::<…>(x)`,
/// `Result::<…>::unwrap`. Longer ones are not passed over, and the construct is not read. It is
/// the bound on every construct here but a `cfg_attr` list: no other shape is as long.
pub(crate) const TURBOFISH_TOKENS: usize = 64;

/// How many `#[cfg_attr(` lists may be open inside one another and still each be followed. One
/// opened past that is taken to hold a skip: what is not read is no reason to drop evidence.
/// Code does not nest them at all (the lists a `cfg_attr` applies are written without `#[`);
/// the limit keeps a text that is nothing but openers from holding a list for each.
pub(crate) const CFG_ATTR_LISTS: usize = 16;

/// How many tokens after the current one are in hand before it is looked at.
const AHEAD: usize = TURBOFISH_TOKENS + 8;

/// How many tokens before the current one are kept.
const BEHIND: usize = TURBOFISH_TOKENS + 8;

/// How many tokens are let pass before the window drops those behind it.
const SLIDE: usize = 1024;

/// One line of one side of a hunk.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SideLine<'a> {
    /// Its index in the lines of the text.
    pub index: usize,
    /// Its code view.
    pub code: &'a str,
    /// Whether the change made it: an added line of the new side, a removed line of the old.
    pub changed: bool,
}

/// One signal the reader found: at a line of the hunk, with the rung it stands on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Item {
    /// The index, in the lines of the text, of the first changed line that holds a token of the
    /// construct.
    pub line: usize,
    pub signal: EvidenceSignal,
    /// [`Basis::Lexical`] when the construct's tokens are on one line, [`Basis::Hunk`] when
    /// they are on several.
    pub basis: Basis,
    /// For a panic path: every one found on this line stands inside the braces of an item whose
    /// attribute spells a test. A hint, never proof ([`TestStretch`]).
    pub test_likely: bool,
    /// For a panic path, its forms on this line: [`BY_CALL`] (`unwrap`, `expect`), [`BY_MACRO`]
    /// (`panic!`, `unreachable!`), or both; 0 for every other signal. The case of a diagnostic
    /// (since 0.11.0, F2a, ruling 6): its repairs differ.
    pub forms: u8,
}

/// [`Item::forms`]: a call that panics on failure.
pub(crate) const BY_CALL: u8 = 1;
/// [`Item::forms`]: a macro that panics where it is reached.
pub(crate) const BY_MACRO: u8 = 2;

/// What the reader found on the two sides of a hunk, each sorted by line and then by signal,
/// one item for each pair of them.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct HunkItems {
    /// On added lines.
    pub new: Vec<Item>,
    /// On removed lines.
    pub old: Vec<Item>,
}

/// Read the sides of `hunk` in one reading of it. `views` holds, for each line, its code view on
/// the old and on the new side (`None` on the side the line is not on); `old` and `new` say
/// which sides this reading covers. A Rust hunk can have two readings (a side read from code,
/// and read as if it began inside a block comment): each is read here on its own, whole, and
/// what they find is united by the caller. No view is ever a mixture of two readings.
pub(crate) fn read_hunk(
    lines: &[&PatchLine],
    views: &[(Option<String>, Option<String>)],
    old: bool,
    new: bool,
) -> HunkItems {
    let side = |new: bool| {
        lines
            .iter()
            .zip(views)
            .enumerate()
            .filter_map(move |(index, (line, view))| {
                let (other, changed, code) = if new {
                    (PatchLineKind::Removed, PatchLineKind::Added, &view.1)
                } else {
                    (PatchLineKind::Added, PatchLineKind::Removed, &view.0)
                };
                (line.kind != other).then(|| SideLine {
                    index,
                    code: code.as_deref().unwrap_or(line.text.as_str()),
                    changed: line.kind == changed,
                })
            })
    };
    // A side with no changed line has nothing to report, and is not read; nor is a side that
    // `views` does not cover.
    let has = |kind: PatchLineKind| lines.iter().any(|line| line.kind == kind);
    HunkItems {
        new: if new && has(PatchLineKind::Added) {
            read_new(side(true))
        } else {
            Vec::new()
        },
        old: if old && has(PatchLineKind::Removed) {
            read_old(side(false))
        } else {
            Vec::new()
        },
    }
}

/// The words the reader knows. Every other identifier is [`Word::Other`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Word {
    Unwrap,
    Expect,
    Assert,
    DebugAssert,
    AssertEq,
    DebugAssertEq,
    AssertNe,
    DebugAssertNe,
    Todo,
    Unimplemented,
    Panic,
    Unreachable,
    Ignore,
    CfgAttr,
    Cfg,
    Any,
    All,
    Not,
    Test,
    Rstest,
    Tokio,
    True,
    False,
    Nan,
    Result,
    Option,
    Command,
    New,
    Process,
    Reqwest,
    Ureq,
    TcpStream,
    Connect,
    UdpSocket,
    Bind,
    Allow,
    Return,
    Match,
    In,
    Break,
    Yield,
    If,
    While,
    Mut,
    Become,
    Do,
    Yeet,
    Let,
    Underscore,
    Other,
}

impl Word {
    /// A keyword of rustc 1.99.1 after which an expression can begin and a `<` opens a path in
    /// angle brackets: `return`, `match`, `in` (of `for … in`), `break` (its label too: `break
    /// 'l <Command>::new(…)`), `yield`, `if`, `while`, `mut` (measured as `&mut`) and `become`;
    /// and `yeet` after `do` (read in [`released_path`]). The list is measured: each strict and
    /// reserved keyword, and `yeet` after `do`, was compiled before `<Command>::new("true")` in
    /// every position that fits it, and these compile (`yield`, `become`, `do yeet` with a
    /// nightly feature; IR10-L-2, IR11-L-2, VD3R18-L-1). `else` is not among them, nor `const`
    /// after `&raw` (no address of a temporary). The rule is lexical, the one word before the
    /// `<`: any `mut` opens it, so `&raw mut <Command>::new(…)` (refused, E0745) and `mut <…>`
    /// in a macro's discarded tokens are a writer's false items (VD3R19-L-1, IR12-L-3).
    const fn begins_expression(self) -> bool {
        matches!(
            self,
            Self::Return
                | Self::Match
                | Self::In
                | Self::Break
                | Self::Yield
                | Self::If
                | Self::While
                | Self::Mut
                | Self::Become
        )
    }

    /// The word `name` is, whole: a longer name (`unwrap_or`, `ignored`) is another word.
    fn of(name: &[u8]) -> Self {
        WORDS
            .iter()
            .find(|(word, _)| *word == name)
            .map_or(Self::Other, |(_, word)| *word)
    }
}

/// The words the reader knows ([`Word::of`]).
const WORDS: [(&[u8], Word); 49] = [
    (b"unwrap", Word::Unwrap),
    (b"expect", Word::Expect),
    (b"assert", Word::Assert),
    (b"debug_assert", Word::DebugAssert),
    (b"assert_eq", Word::AssertEq),
    (b"debug_assert_eq", Word::DebugAssertEq),
    (b"assert_ne", Word::AssertNe),
    (b"debug_assert_ne", Word::DebugAssertNe),
    (b"todo", Word::Todo),
    (b"unimplemented", Word::Unimplemented),
    (b"panic", Word::Panic),
    (b"unreachable", Word::Unreachable),
    (b"ignore", Word::Ignore),
    (b"cfg_attr", Word::CfgAttr),
    (b"cfg", Word::Cfg),
    (b"any", Word::Any),
    (b"all", Word::All),
    (b"not", Word::Not),
    (b"test", Word::Test),
    (b"rstest", Word::Rstest),
    (b"tokio", Word::Tokio),
    (b"true", Word::True),
    (b"false", Word::False),
    (b"NAN", Word::Nan),
    (b"Result", Word::Result),
    (b"Option", Word::Option),
    (b"Command", Word::Command),
    (b"new", Word::New),
    (b"process", Word::Process),
    (b"reqwest", Word::Reqwest),
    (b"ureq", Word::Ureq),
    (b"TcpStream", Word::TcpStream),
    (b"connect", Word::Connect),
    (b"UdpSocket", Word::UdpSocket),
    (b"bind", Word::Bind),
    (b"allow", Word::Allow),
    (b"return", Word::Return),
    (b"match", Word::Match),
    (b"in", Word::In),
    (b"break", Word::Break),
    (b"yield", Word::Yield),
    (b"if", Word::If),
    (b"while", Word::While),
    (b"mut", Word::Mut),
    (b"become", Word::Become),
    (b"do", Word::Do),
    (b"yeet", Word::Yeet),
    (b"let", Word::Let),
    (b"_", Word::Underscore),
];

/// The names of [`WORDS`], for the test that ranks a long line of each.
#[cfg(test)]
pub(crate) fn word_names() -> impl Iterator<Item = &'static [u8]> {
    WORDS.iter().map(|(name, _)| *name)
}

/// What a token is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum What {
    /// An identifier, a keyword or a number.
    Name(Word),
    /// One byte of punctuation.
    Mark(u8),
}

/// One token and where it stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Token<'a> {
    what: What,
    /// Its bytes in the code view: an assertion's operands are compared by them (since 0.11.0,
    /// F2a).
    text: &'a str,
    /// The index of its line in `hunk.lines`.
    line: usize,
    /// The byte column it starts at.
    column: usize,
    /// Whether its line is one the change made.
    changed: bool,
}

/// Whether `byte` can be part of an identifier: an ASCII letter or digit, `_`, or a byte of a
/// character that is not ASCII.
const fn names(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || !byte.is_ascii()
}

// The compiler's white space, the six ASCII characters and U+0085, U+200E, U+200F, U+2028 and
// U+2029, separates tokens: read as part of a name, a left-to-right mark between `unwrap` and
// its parenthesis would make another word of it. The one definition is the code view's.
use crate::lexical::rust_white as white;

/// The tokens of a side, line after line.
struct Tokens<'a, I> {
    lines: I,
    line: Option<SideLine<'a>>,
    /// The byte of `line` the next token is looked for at.
    at: usize,
}

impl<'a, I: Iterator<Item = SideLine<'a>>> Iterator for Tokens<'a, I> {
    type Item = Token<'a>;

    fn next(&mut self) -> Option<Token<'a>> {
        loop {
            let Some(line) = self.line else {
                self.line = Some(self.lines.next()?);
                self.at = 0;
                continue;
            };
            let bytes = line.code.as_bytes();
            loop {
                let space = white(bytes.get(self.at..).unwrap_or_default());
                if space == 0 {
                    break;
                }
                self.at += space;
            }
            let Some(&first) = bytes.get(self.at) else {
                self.line = None;
                continue;
            };
            let column = self.at;
            let what = if names(first) {
                while bytes.get(self.at).is_some_and(|byte| {
                    names(*byte) && (byte.is_ascii() || white(&bytes[self.at..]) == 0)
                }) {
                    self.at += 1;
                }
                What::Name(Word::of(&bytes[column..self.at]))
            } else {
                self.at += 1;
                What::Mark(first)
            };
            return Some(Token {
                what,
                text: line.code.get(column..self.at).unwrap_or_default(),
                line: line.index,
                column,
                changed: line.changed,
            });
        }
    }
}

/// The tokens around the current one: up to [`BEHIND`] before it and [`AHEAD`] after it, taken
/// from the side as the reader moves on. A token is cut once and held for a while; the side is
/// never held whole (a side of 60 MiB of `}` is as many tokens).
struct Window<'a, I> {
    source: Tokens<'a, I>,
    held: Vec<Token<'a>>,
    /// The current token's place in `held`.
    at: usize,
}

impl<'a, I: Iterator<Item = SideLine<'a>>> Window<'a, I> {
    fn over(lines: I) -> Self {
        let mut window = Self {
            source: Tokens {
                lines,
                line: None,
                at: 0,
            },
            held: Vec::with_capacity(SLIDE + BEHIND + AHEAD + 1),
            at: 0,
        };
        window.fill();
        window
    }

    fn fill(&mut self) {
        while self.held.len() <= self.at + AHEAD {
            match self.source.next() {
                Some(token) => self.held.push(token),
                None => break,
            }
        }
    }

    /// Go on to the next token.
    fn step(&mut self) {
        self.at += 1;
        if self.at >= SLIDE + BEHIND {
            self.held.drain(..self.at - BEHIND);
            self.at = BEHIND;
        }
        self.fill();
    }

    /// The token at place `at` of the window, if it holds one there.
    fn token(&self, at: usize) -> Option<Token<'a>> {
        self.held.get(at).copied()
    }

    fn here(&self) -> Option<Token<'a>> {
        self.token(self.at)
    }

    /// Whether the token at `at` is the punctuation `mark`.
    fn marks(&self, at: usize, mark: u8) -> bool {
        self.token(at)
            .is_some_and(|token| token.what == What::Mark(mark))
    }

    /// The word at `at`, when a name stands there.
    fn word(&self, at: usize) -> Option<Word> {
        match self.token(at)?.what {
            What::Name(word) => Some(word),
            What::Mark(_) => None,
        }
    }

    /// Whether the tokens at `at` and `at + 1` are `first` and `second` written with nothing
    /// between them: `::` and `->` are one token each to the compiler, and `: :` is not `::`.
    fn pair(&self, at: usize, first: u8, second: u8) -> bool {
        match (self.token(at), self.token(at + 1)) {
            (Some(a), Some(b)) => {
                a.what == What::Mark(first)
                    && b.what == What::Mark(second)
                    && a.line == b.line
                    && a.column + 1 == b.column
            }
            _ => false,
        }
    }

    /// Whether `::` stands at `at` and `at + 1`.
    fn path(&self, at: usize) -> bool {
        self.pair(at, b':', b':')
    }

    /// Whether the `>` at `at` is the end of an arrow (`->`), and so closes nothing.
    fn arrow(&self, at: usize) -> bool {
        at.checked_sub(1)
            .is_some_and(|before| self.pair(before, b'-', b'>'))
    }

    /// The place of the `>` that closes the `<` at `open`, when it is within
    /// [`TURBOFISH_TOKENS`]. Brackets inside are passed over whole.
    fn closes(&self, open: usize) -> Option<usize> {
        let (mut angles, mut nested) = (0usize, 0usize);
        for at in open..open + TURBOFISH_TOKENS {
            let What::Mark(mark) = self.token(at)?.what else {
                continue;
            };
            match mark {
                b'(' | b'[' | b'{' => nested += 1,
                b')' | b']' | b'}' => nested = nested.checked_sub(1)?,
                b'<' if nested == 0 => angles += 1,
                b'>' if nested == 0 && !self.arrow(at) => {
                    angles -= 1;
                    if angles == 0 {
                        return Some(at);
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// The place of the `<` that the `>` at `close` closes, when it is within
    /// [`TURBOFISH_TOKENS`]: [`Self::closes`] read backwards.
    fn opens(&self, close: usize) -> Option<usize> {
        let (mut angles, mut nested) = (0usize, 0usize);
        for back in 0..TURBOFISH_TOKENS {
            let at = close.checked_sub(back)?;
            let What::Mark(mark) = self.token(at)?.what else {
                continue;
            };
            match mark {
                b')' | b']' | b'}' => nested += 1,
                b'(' | b'[' | b'{' => nested = nested.checked_sub(1)?,
                b'>' if nested == 0 && !self.arrow(at) => angles += 1,
                b'<' if nested == 0 => {
                    angles = angles.checked_sub(1)?;
                    if angles == 0 {
                        return Some(at);
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// The place of the closer that matches the bracket at `open`, when it is within
    /// [`TURBOFISH_TOKENS`] tokens of it, both counted. Brackets inside are passed over whole.
    fn matching(&self, open: usize) -> Option<usize> {
        let What::Mark(bracket) = self.token(open)?.what else {
            return None;
        };
        let close = closer(bracket)?;
        let mut depth = 0usize;
        for at in open..open + TURBOFISH_TOKENS {
            match self.token(at)?.what {
                What::Mark(b'(' | b'[' | b'{') => depth += 1,
                What::Mark(mark @ (b')' | b']' | b'}')) => {
                    depth -= 1;
                    if depth == 0 {
                        return (mark == close).then_some(at);
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// Whether the tokens from `a.0` to `a.1` are those from `b.0` to `b.1`, one by one, and
    /// none is a quote (the code view blanks the text of a string or a character, so `"a"` and
    /// `"b"` would be the same), the name `NAN` (`f64::NAN == f64::NAN` is false), or a `(`, `{`
    /// or `!`: a call, a block or a macro can answer otherwise each time, and the same call on
    /// both sides is how a test checks that code is deterministic (`assert_eq!(rank(&files),
    /// rank(&files))`: seven such lines in this project's history, none of which cannot fail).
    fn same(&self, a: (usize, usize), b: (usize, usize)) -> bool {
        a.1 - a.0 == b.1 - b.0
            && (0..=a.1 - a.0).all(|k| match (self.token(a.0 + k), self.token(b.0 + k)) {
                (Some(x), Some(y)) => {
                    x.text == y.text
                        && !matches!(
                            x.what,
                            What::Mark(b'"' | b'\'' | b'(' | b'{' | b'!') | What::Name(Word::Nan)
                        )
                }
                _ => false,
            })
    }

    /// The first changed line among the tokens from `first` to `last`, and the rung a construct
    /// made of them stands on: `lexical` on one line, `hunk` over several.
    fn extent(&self, first: usize, last: usize) -> (Option<usize>, Basis) {
        let changed = self.held[first..=last]
            .iter()
            .find(|token| token.changed)
            .map(|token| token.line);
        let basis = if self.held[first].line == self.held[last].line {
            Basis::Lexical
        } else {
            Basis::Hunk
        };
        (changed, basis)
    }
}

/// The matching closer of an opening bracket.
const fn closer(mark: u8) -> Option<u8> {
    match mark {
        b'(' => Some(b')'),
        b'[' => Some(b']'),
        b'{' => Some(b'}'),
        _ => None,
    }
}

/// A call that panics on failure, when the current token is `unwrap` or `expect`: the places
/// of its first and last token.
///
/// - After `.`, then generic arguments or none, then `(`: a method call. With the generic
///   arguments it was no construct to the parser (third independent review, IR3-M-5:
///   `b().expect::<'static>("no")` and `b().unwrap::<>()` compile and panic).
/// - After `::`, then generic arguments or none, then `(`: a call by a path, whoever owns it
///   (`X::unwrap(x)`, `<Option<u32>>::unwrap(x)`, `a::b::unwrap::<u8>(x)`).
/// - After `Result::` or `Option::` (also `Result::<u32, E>::`), not called: the standard
///   function used as a value (`.map(Result::unwrap)`). A path of the patch's own
///   (`it.map(X::unwrap)`) is not read here: nothing in the tokens tells such a value from an
///   import, a type or a longer path.
///
/// After a range the `.` is the second of `..`, and `0..unwrap::<u8>()` calls a free function
/// named `unwrap`: it is read as a method call all the same, as round 14 read it (a writer's
/// false item: rustfmt writes the call on one line, and a function of that name is no
/// honest code's; tenth independent review, IR10-L-3).
fn panic_path<'a, I: Iterator<Item = SideLine<'a>>>(
    window: &Window<'a, I>,
) -> Option<(usize, usize)> {
    let at = window.at;
    let method = at.checked_sub(1).is_some_and(|dot| window.marks(dot, b'.'));
    let pathed = at.checked_sub(2).filter(|colons| window.path(*colons));
    let first = if method { at - 1 } else { pathed? };
    let mut next = at + 1;
    let mut passed = true;
    if window.path(next) && window.marks(next + 2, b'<') {
        match window.closes(next + 2) {
            Some(close) => next = close + 1,
            None => passed = false,
        }
    }
    if passed && window.marks(next, b'(') {
        return Some((first, next));
    }
    if method {
        return None;
    }
    let before = first.checked_sub(1)?;
    let standard = |at: usize| matches!(window.word(at), Some(Word::Result | Word::Option));
    if standard(before) {
        return Some((before, at));
    }
    // `Result::<u32, E>::unwrap`
    if window.marks(before, b'>') && !window.arrow(before) {
        let open = window.opens(before)?;
        let owner = open.checked_sub(3)?;
        if window.path(open - 2) && standard(owner) {
            return Some((owner, at));
        }
    }
    None
}

/// An assertion that cannot fail, when the current token is `assert` or `debug_assert`
/// (`trues` 1) or `assert_eq` or `debug_assert_eq` (`trues` 2): `!`, an opening bracket of any
/// of the three kinds, `true` (twice with a comma between for the `_eq` forms), then a comma
/// or the bracket's closer.
fn trivial_assert<'a, I: Iterator<Item = SideLine<'a>>>(
    window: &Window<'a, I>,
    trues: usize,
) -> Option<(usize, usize)> {
    let at = window.at;
    if !window.marks(at + 1, b'!') {
        return None;
    }
    let What::Mark(open) = window.token(at + 2)?.what else {
        return None;
    };
    let close = closer(open)?;
    let mut next = at + 3;
    for nth in 0..trues {
        if nth > 0 {
            if !window.marks(next, b',') {
                return None;
            }
            next += 1;
        }
        if window.word(next) != Some(Word::True) {
            return None;
        }
        next += 1;
    }
    (window.marks(next, b',') || window.marks(next, close)).then_some((at, next))
}

/// What an assertion asks of its operands: `assert!` and `debug_assert!` that the one is true,
/// the `_eq` forms that the two are equal, the `_ne` forms that they differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Asks {
    True,
    Equal,
    Unequal,
}

/// An operand's value as far as its tokens alone give it ([`operand`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Value {
    Bool(bool),
    /// An integer literal.
    Int(i128),
    /// Anything else; for one run of tokens with none of the reading's operators in it, the
    /// places of its first and last token, which [`Window::same`] compares.
    Unknown(Option<(usize, usize)>),
}

/// The comparisons, which hold or not by how two values are ordered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Compare {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl Compare {
    fn holds(self, order: std::cmp::Ordering) -> bool {
        use std::cmp::Ordering::{Equal, Greater, Less};
        match self {
            Self::Eq => order == Equal,
            Self::Ne => order != Equal,
            Self::Lt => order == Less,
            Self::Le => order != Greater,
            Self::Gt => order == Greater,
            Self::Ge => order != Less,
        }
    }
}

/// The operators [`operand`] knows, and an open parenthesis on its stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Operator {
    Open,
    Or,
    And,
    Compare(Compare),
    Not,
}

impl Operator {
    /// How tightly it binds, as in Rust: `!`, then the comparisons, `&&`, `||`.
    const fn binds(self) -> u8 {
        match self {
            Self::Open => 0,
            Self::Or => 1,
            Self::And => 2,
            Self::Compare(_) => 3,
            Self::Not => 4,
        }
    }

    /// `left self right` ([`Self::Not`] takes `left` alone): `||` is true where either side is
    /// (`x || true` too: whatever `x` is, the condition is not false), `&&` false where either
    /// is; a comparison of two literals, or of two runs of the same tokens (`x == x`).
    fn apply<'a, I: Iterator<Item = SideLine<'a>>>(
        self,
        window: &Window<'a, I>,
        left: Value,
        right: Value,
    ) -> Value {
        use Value::{Bool, Int, Unknown};
        match (self, left, right) {
            (Self::Not, Bool(value), _) => Bool(!value),
            (Self::Or, Bool(true), _) | (Self::Or, _, Bool(true)) => Bool(true),
            (Self::Or, Bool(false), Bool(false)) | (Self::And, Bool(true), Bool(true)) => {
                Bool(self == Self::And)
            }
            (Self::And, Bool(false), _) | (Self::And, _, Bool(false)) => Bool(false),
            (Self::Compare(compare), Bool(a), Bool(b)) => Bool(compare.holds(a.cmp(&b))),
            (Self::Compare(compare), Int(a), Int(b)) => Bool(compare.holds(a.cmp(&b))),
            (Self::Compare(compare), Unknown(Some(a)), Unknown(Some(b))) if window.same(a, b) => {
                Bool(compare.holds(std::cmp::Ordering::Equal))
            }
            _ => Unknown(None),
        }
    }
}

/// The operator at `at` and how many tokens it is: `&&`, `||` and the comparisons.
fn operator<'a, I: Iterator<Item = SideLine<'a>>>(
    window: &Window<'a, I>,
    at: usize,
) -> Option<(Operator, usize)> {
    use Compare as C;
    let pairs = [
        (b'&', b'&', Operator::And),
        (b'|', b'|', Operator::Or),
        (b'=', b'=', Operator::Compare(C::Eq)),
        (b'!', b'=', Operator::Compare(C::Ne)),
        (b'<', b'=', Operator::Compare(C::Le)),
        (b'>', b'=', Operator::Compare(C::Ge)),
    ];
    if let Some((_, _, operator)) = pairs.iter().find(|(a, b, _)| window.pair(at, *a, *b)) {
        return Some((*operator, 2));
    }
    if window.marks(at, b'<') {
        return Some((Operator::Compare(C::Lt), 1));
    }
    (window.marks(at, b'>') && !window.arrow(at)).then_some((Operator::Compare(C::Gt), 1))
}

/// The integer literal at `at`, a `-` before one too, and the place after it: decimal, `0x`,
/// `0o` or `0b`, with `_` and an integer type's suffix or none.
fn literal<'a, I: Iterator<Item = SideLine<'a>>>(
    window: &Window<'a, I>,
    at: usize,
) -> Option<(i128, usize)> {
    const SUFFIXES: [&str; 12] = [
        "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128", "isize",
    ];
    let (negative, at) = if window.marks(at, b'-') {
        (true, at + 1)
    } else {
        (false, at)
    };
    let text = window.token(at)?.text;
    if !text.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    let text = SUFFIXES
        .iter()
        .find_map(|suffix| text.strip_suffix(suffix))
        .unwrap_or(text)
        .replace('_', "");
    let (digits, radix) = match text.get(..2) {
        Some("0x") => (&text[2..], 16),
        Some("0o") => (&text[2..], 8),
        Some("0b") => (&text[2..], 2),
        _ => (text.as_str(), 10),
    };
    let value = i128::from_str_radix(digits, radix).ok()?;
    Some((if negative { -value } else { value }, at + 1))
}

/// The end of a run of tokens from `at`: up to an operator of [`operator`] or a `)` outside
/// its own brackets, or `to`. Generic arguments after `::` are passed over whole. `None` for a
/// `]` or `}` that closes nothing of it, or a lone `=`.
fn run<'a, I: Iterator<Item = SideLine<'a>>>(
    window: &Window<'a, I>,
    mut at: usize,
    to: usize,
) -> Option<usize> {
    let mut depth = 0usize;
    while at < to {
        if depth == 0 && (operator(window, at).is_some() || window.marks(at, b')')) {
            break;
        }
        match window.token(at)?.what {
            What::Mark(b'(' | b'[' | b'{') => depth += 1,
            What::Mark(b')' | b']' | b'}') => depth = depth.checked_sub(1)?,
            What::Mark(b'=') if depth == 0 => return None,
            What::Mark(b':') if window.path(at) && window.marks(at + 2, b'<') => {
                at = window.closes(at + 2)?;
            }
            _ => {}
        }
        at += 1;
    }
    (depth == 0).then_some(at)
}

/// The value of the operand whose tokens are from `from` to before `to`, read once from the
/// left with two stacks (nothing recurses): `!`, parentheses, `&&`, `||`, the six
/// comparisons, `true`, `false`, integer literals and runs of other tokens ([`run`]). `None`
/// for a shape it does not read.
fn operand<'a, I: Iterator<Item = SideLine<'a>>>(
    window: &Window<'a, I>,
    from: usize,
    to: usize,
) -> Option<Value> {
    let (mut values, mut operators) = (Vec::<Value>::new(), Vec::<Operator>::new());
    let reduce = |values: &mut Vec<Value>, operator: Operator| {
        let right = values.pop()?;
        let left = values.pop()?;
        values.push(operator.apply(window, left, right));
        Some(())
    };
    // a value is whole: the prefixes before it apply
    let push = |values: &mut Vec<Value>, operators: &mut Vec<Operator>, mut value: Value| {
        while operators.last() == Some(&Operator::Not) {
            operators.pop();
            value = Operator::Not.apply(window, value, value);
        }
        values.push(value);
    };
    let (mut at, mut value_next) = (from, true);
    while at < to {
        if value_next {
            let ends =
                |at: usize| at >= to || window.marks(at, b')') || operator(window, at).is_some();
            let (value, end) = match window.token(at)?.what {
                What::Mark(b'!' | b'(') => {
                    let open = window.marks(at, b'(');
                    operators.push(if open { Operator::Open } else { Operator::Not });
                    at += 1;
                    continue;
                }
                What::Name(word @ (Word::True | Word::False)) if ends(at + 1) => {
                    (Value::Bool(word == Word::True), at + 1)
                }
                _ => match literal(window, at).filter(|(_, end)| ends(*end)) {
                    Some((value, end)) => (Value::Int(value), end),
                    None => {
                        let end = run(window, at, to).filter(|end| *end > at)?;
                        (Value::Unknown(Some((at, end - 1))), end)
                    }
                },
            };
            push(&mut values, &mut operators, value);
            (at, value_next) = (end, false);
        } else if window.marks(at, b')') {
            loop {
                match operators.pop()? {
                    Operator::Open => break,
                    operator => reduce(&mut values, operator)?,
                }
            }
            let value = values.pop()?;
            push(&mut values, &mut operators, value);
            at += 1;
        } else {
            let (operator, length) = operator(window, at)?;
            while let Some(&top) = operators.last()
                && top != Operator::Open
                && top.binds() >= operator.binds()
            {
                operators.pop();
                reduce(&mut values, top)?;
            }
            operators.push(operator);
            (at, value_next) = (at + length, true);
        }
    }
    if value_next {
        return None;
    }
    while let Some(operator) = operators.pop() {
        if operator == Operator::Open {
            return None;
        }
        reduce(&mut values, operator)?;
    }
    (values.len() == 1).then(|| values[0])
}

/// An assertion that cannot fail, read as a class (since 0.11.0, F2a), when the current token
/// is an assertion's word: `!`, an opening bracket, then its operands up to the first comma
/// outside brackets each (one, or two for the `_eq` and `_ne` forms), within [`AHEAD`] tokens.
/// `assert!` whose operand is true by its tokens alone (`!false`, `(true)`, `1 == 1`,
/// `true || x`, `x == x`); `assert_eq!` whose operands are equal literals or the same tokens
/// (`x, x`; `v.0, v.0`); `assert_ne!` whose operands are unequal literals (`1, 2`). Not read
/// ([`Window::same`]): the same call, block or macro on both sides (`f(2), f(2)`, which can
/// answer otherwise each time), `NAN`, and two strings or characters, which the code view
/// blanks; nor what holds only by the type (`v.is_ok() || v.is_err()`, `matches!(v, _)`). The construct is from the word to the comma or bracket after the last
/// operand, so a change of the message alone is none.
fn cannot_fail<'a, I: Iterator<Item = SideLine<'a>>>(
    window: &Window<'a, I>,
    asks: Asks,
) -> Option<(usize, usize)> {
    let at = window.at;
    if !window.marks(at + 1, b'!') {
        return None;
    }
    let What::Mark(open) = window.token(at + 2)?.what else {
        return None;
    };
    let close = closer(open)?;
    let wanted = if asks == Asks::True { 1 } else { 2 };
    let (mut spans, mut found) = ([(0, 0); 2], 0);
    let (mut start, mut next, mut depth) = (at + 3, at + 3, 0usize);
    while found < wanted {
        if next > at + AHEAD {
            return None;
        }
        match window.token(next)?.what {
            What::Mark(b'(' | b'[' | b'{') => depth += 1,
            What::Mark(mark @ (b')' | b']' | b'}')) if depth == 0 => {
                if mark != close || found + 1 != wanted {
                    return None;
                }
                (spans[found], found) = ((start, next), found + 1);
            }
            What::Mark(b')' | b']' | b'}') => depth -= 1,
            What::Mark(b',') if depth == 0 => {
                (spans[found], found, start) = ((start, next), found + 1, next + 1);
            }
            What::Mark(b':') if window.path(next) && window.marks(next + 2, b'<') => {
                next = window.closes(next + 2)?;
            }
            _ => {}
        }
        next += 1;
    }
    let value = |(from, to): (usize, usize)| operand(window, from, to);
    let holds = match asks {
        Asks::True => value(spans[0])?,
        Asks::Equal | Asks::Unequal => {
            let compare = if asks == Asks::Equal {
                Compare::Eq
            } else {
                Compare::Ne
            };
            Operator::Compare(compare).apply(window, value(spans[0])?, value(spans[1])?)
        }
    };
    (holds == Value::Bool(true)).then_some((at, spans[wanted - 1].1))
}

/// The calls of the standard library (and `killpg`) that answer with a `Result` whose error is
/// a failure, and that code drops with `let _ =`: the last call of a dropped statement must be
/// one of these for the drop to be read ([`dropped`]).
const FALLIBLE: [&str; 29] = [
    "remove_file",
    "remove_dir",
    "remove_dir_all",
    "create_dir",
    "create_dir_all",
    "rename",
    "copy",
    "hard_link",
    "set_permissions",
    "write_all",
    "flush",
    "sync_all",
    "sync_data",
    "set_len",
    "read_to_end",
    "read_to_string",
    "read_exact",
    "send",
    "try_send",
    "recv",
    "join",
    "kill",
    "killpg",
    "wait",
    "try_wait",
    "shutdown",
    "set_nonblocking",
    "set_read_timeout",
    "set_write_timeout",
];

/// A result dropped by a statement (since 0.11.0, F2a), when the current token is `let` or
/// `_`: `let _ =` (or `let _: Type =`), or `_ =` where a statement begins (after `;`, a brace or
/// nothing the side shows), then an expression with no `?` in it whose last call, outside
/// brackets, is one of [`FALLIBLE`] by name (a method or a path: `child.kill()`,
/// `std::fs::remove_file(p)`), then `;`, within [`AHEAD`] tokens. The reader cannot know a
/// call's type, so it reads only names whose standard function answers with a `Result`: on
/// this project's history most other calls dropped this way were a `String` written with
/// `write!`/`writeln!` (no error exists), a value computed for its effect in a test, or a
/// function of the project (of 450 such lines a third dropped a `Result`). Not read: a call of
/// another name (a function of the patch's own, `write!(…)`), a name that only begins with `_`
/// (`let _e =`), a pattern (`let (_, _) =`), `.ok();`, `drop(…)`, an expression that ends
/// otherwise (`.await`, a block), and `connect`, `bind`, `status`, `output` and `spawn`, names
/// that other crates' calls with no `Result` share and that the paths of `network_fetch` and
/// `process_spawn` already read. A method of one of the names read that answers with no
/// `Result` (`Path::join`, `[…].join(…)`) is a false item.
fn dropped<'a, I: Iterator<Item = SideLine<'a>>>(window: &Window<'a, I>) -> Option<(usize, usize)> {
    let at = window.at;
    let begins = |before: usize| b";{}".iter().any(|m| window.marks(before, *m));
    let mut next = match window.word(at)? {
        Word::Let if window.word(at + 1) == Some(Word::Underscore) => at + 2,
        Word::Underscore if at.checked_sub(1).is_none_or(begins) => at + 1,
        _ => return None,
    };
    let (mut depth, mut angles, mut open) = (0usize, 0usize, None);
    if window.marks(next, b':') && !window.path(next) {
        // the type, up to the `=` outside its brackets
        next += 1;
        while !(depth == 0 && angles == 0 && window.marks(next, b'=')) {
            match window.token(next)?.what {
                What::Mark(b'(' | b'[') => depth += 1,
                What::Mark(b')' | b']') => depth = depth.checked_sub(1)?,
                What::Mark(b'<') => angles += 1,
                What::Mark(b'>') if !window.arrow(next) => angles = angles.checked_sub(1)?,
                What::Mark(b';' | b'{' | b'}') => return None,
                _ => {}
            }
            next = (next < at + AHEAD).then_some(next + 1)?;
        }
    }
    if !window.marks(next, b'=') || window.pair(next, b'=', b'=') || window.pair(next, b'=', b'>') {
        return None;
    }
    for end in next + 1..=at + AHEAD {
        match window.token(end)?.what {
            What::Mark(b'?') => return None,
            What::Mark(b'(') if depth == 0 => {
                open = Some(end);
                depth = 1;
            }
            What::Mark(b'(' | b'[' | b'{') => depth += 1,
            What::Mark(b')' | b']' | b'}') => depth = depth.checked_sub(1)?,
            What::Mark(b';') if depth == 0 => {
                // the `(` that the last `)` closes follows the name of what is called
                let called = open
                    .and_then(|open| window.token(open.checked_sub(1)?))
                    .is_some_and(|name| {
                        matches!(name.what, What::Name(_)) && FALLIBLE.contains(&name.text)
                    });
                return (called && window.marks(end - 1, b')')).then_some((at, end));
            }
            _ => {}
        }
    }
    None
}

/// A placeholder, when the current token is `todo` or `unimplemented`: `!` and an opening
/// bracket. The same shape is a panic path when the token is `panic` or `unreachable` (since
/// 0.11.0, F2a): `panic!(…)` and `unreachable!(…)` stop the program where they are reached.
fn stub<'a, I: Iterator<Item = SideLine<'a>>>(window: &Window<'a, I>) -> Option<(usize, usize)> {
    let at = window.at;
    let opened = matches!(window.token(at + 2)?.what, What::Mark(mark) if closer(mark).is_some());
    (window.marks(at + 1, b'!') && opened).then_some((at, at + 2))
}

/// `#[ignore]`, `#[ignore = "…"]`, `#[ignore(…)]` and the inner forms `#![ignore]` (inside a
/// test's body it switches the test off as the outer one does; compiled and run), when the
/// current token is `#`.
///
/// `#[ignore(…)]` is not a form the attribute documents, and rustc 1.99.1 reports it through
/// the lint `ill_formed_attribute_input` ("this was previously accepted by the compiler"):
/// deny by default, and a lint can be allowed (`[lints.rust] ill_formed_attribute_input =
/// "allow"` in `Cargo.toml`, a line no Rust rule reads). With it allowed the test compiles
/// and does not run (tenth independent review, IR10-M-1: `ignparen.patch`, two tests ignored;
/// until round 18 the documents here said the compiler refuses it, which is false).
///
/// A `#` straight after a name is read like any other: up to edition 2018 `x#[ignore]` is
/// the tokens `x`, `#`, `[`, `ignore`, `]` to the compiler too, and a macro can drop the `x`
/// (compiled and run on edition 2018: the test does not run; from edition 2021 it is an error).
///
/// The attribute is read only when the side's text holds it whole, from its `#` to the `]`
/// that closes its `[` ([`TURBOFISH_TOKENS`] tokens at most from `[` to `]`; a `cfg_attr` list
/// past [`CFG_ATTR_LISTS`] is the one exception, see [`Skips`]): in a macro that takes any
/// tokens `#[ignore(` goes on as anything (`discard! { #[ignore(` and `NOT_A_SKIP )] }`
/// compiles and skips no test; VD3R18-M-1, a `git diff -U0` that ends on the parenthesis).
/// The construct, for where it is evidence, is still `#[ignore` and the token after it.
fn ignored<'a, I: Iterator<Item = SideLine<'a>>>(window: &Window<'a, I>) -> Option<(usize, usize)> {
    let at = window.at;
    let open = if window.marks(at + 1, b'!') {
        at + 2
    } else {
        at + 1
    };
    let ends = window.marks(open + 2, b']')
        || window.marks(open + 2, b'=')
        || window.marks(open + 2, b'(');
    (window.marks(open, b'[')
        && window.word(open + 1) == Some(Word::Ignore)
        && ends
        && window.matching(open).is_some())
    .then_some((at, open + 2))
}

/// A test's attribute, when the current token is `#`: `#[test]`, `#[tokio::test]` or
/// `#[rstest]`, closed there or followed by arguments (`#[tokio::test(flavor = "…")]`; third
/// independent review, IR3-L-4: only the bare three were read).
fn test_attribute<'a, I: Iterator<Item = SideLine<'a>>>(
    window: &Window<'a, I>,
    at: usize,
) -> Option<(usize, usize)> {
    if !window.marks(at + 1, b'[') {
        return None;
    }
    let end = match window.word(at + 2)? {
        Word::Test | Word::Rstest => at + 3,
        Word::Tokio if window.path(at + 3) && window.word(at + 5) == Some(Word::Test) => at + 6,
        _ => return None,
    };
    (window.marks(end, b']') || window.marks(end, b'(')).then_some((at, end))
}

/// What a `cfg` predicate is in a build of the tests: true, false, or true on some platforms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Holds {
    Yes,
    No,
    Depends,
}

impl Holds {
    /// `self` and `other` taken together by `any` (`or`) or by `all`.
    fn join(self, other: Self, or: bool) -> Self {
        let (wins, loses) = if or {
            (Self::Yes, Self::No)
        } else {
            (Self::No, Self::Yes)
        };
        if self == wins || other == wins {
            wins
        } else if self == loses && other == loses {
            loses
        } else {
            Self::Depends
        }
    }
}

/// Whether the predicate of `#[cfg(…)]` from `from` to before `to` switches a test off: it is
/// false in every build of the tests (`any()`, `all(any())`, `not(all())`, `not(test)`), or,
/// not true in every one, it names a predicate outside the platform's (`FALSE`,
/// `feature = "…"`, a name of the build's own). The platform's are `unix`, `windows`, every `target_…`, `panic`, `miri` and
/// `debug_assertions`, and `test`, which holds; `any`, `all` and `not` combine them, read with
/// one stack. `None` for tokens a predicate does not have.
fn switched_off<'a, I: Iterator<Item = SideLine<'a>>>(
    window: &Window<'a, I>,
    from: usize,
    to: usize,
) -> Option<bool> {
    const PLATFORM: [&str; 5] = ["unix", "windows", "panic", "miri", "debug_assertions"];
    // the lists open: `any` (or), `all` or `not`, and what their arguments give so far
    let mut lists: Vec<(Option<Word>, Option<Holds>)> = vec![(None, None)];
    let (mut at, mut unlisted) = (from, false);
    while at < to {
        let token = window.token(at)?;
        let value = match token.what {
            What::Mark(b',') => None,
            What::Mark(b')') => {
                let (word, held) = lists.pop()?;
                Some(match (word?, held) {
                    (Word::Any, held) => held.unwrap_or(Holds::No),
                    (Word::All, held) => held.unwrap_or(Holds::Yes),
                    (_, Some(Holds::Yes)) => Holds::No,
                    (_, Some(Holds::No)) => Holds::Yes,
                    (_, held) => held?,
                })
            }
            What::Name(word @ (Word::Any | Word::All | Word::Not))
                if window.marks(at + 1, b'(') =>
            {
                lists.push((Some(word), None));
                at += 1;
                None
            }
            What::Name(_) if window.marks(at + 1, b'(') => {
                // a predicate with arguments of its own (`version(…)`): not the platform's
                unlisted = true;
                at = window.matching(at + 1)?;
                Some(Holds::Depends)
            }
            What::Name(word) => {
                let name = token.text;
                // `key = "…"`: the string is blank in the code view, its quotes are two tokens
                if window.marks(at + 1, b'=') && window.marks(at + 2, b'"') {
                    at = (at + 3..to).find(|q| window.marks(*q, b'"'))?;
                }
                if word == Word::Test {
                    Some(Holds::Yes)
                } else {
                    unlisted |= !(PLATFORM.contains(&name) || name.starts_with("target_"));
                    Some(Holds::Depends)
                }
            }
            What::Mark(_) => return None,
        };
        if let Some(value) = value {
            let (word, held) = lists.last_mut()?;
            *held = Some(match (*word, *held) {
                (_, None) => value,
                (Some(Word::Any), Some(was)) => was.join(value, true),
                (Some(Word::All), Some(was)) => was.join(value, false),
                // a second argument of `not`, or of the predicate itself
                _ => return None,
            });
        }
        at += 1;
    }
    match lists[..] {
        [(None, Some(held))] => Some(held == Holds::No || (held == Holds::Depends && unlisted)),
        _ => None,
    }
}

/// A test compiled out (since 0.11.0, F2a), when the current token is `#`: among the whole
/// outer attributes that follow one another from here, within [`AHEAD`] tokens, a test's
/// attribute ([`test_attribute`]) and `#[cfg(…)]` whose predicate switches it off
/// ([`switched_off`]), in either order: `#[cfg(any())] #[test]`. The item is on the first
/// changed line of the two attributes (a test added under such a `cfg` is one too), with the
/// rung of the two. Not read: a `cfg` on a module of tests (`#[cfg(any())] mod tests`), a
/// test attribute that `cfg_attr` applies, an inner `#![cfg(…)]` over a file of tests.
fn compiled_out<'a, I: Iterator<Item = SideLine<'a>>>(window: &Window<'a, I>) -> Option<Item> {
    let (mut at, mut test, mut off) = (window.at, None, None);
    while window.marks(at, b'#') && window.marks(at + 1, b'[') && at < window.at + AHEAD {
        let close = window.matching(at + 1)?;
        if test_attribute(window, at).is_some() {
            test = test.or(Some((at, close)));
        }
        let whole = window.matching(at + 3) == Some(close - 1);
        if off.is_none()
            && window.word(at + 2) == Some(Word::Cfg)
            && whole
            && switched_off(window, at + 4, close - 1) == Some(true)
        {
            off = Some((at, close));
        }
        if let (Some(test), Some(off)) = (test, off) {
            let (first, rung) = window.extent(test.0, test.1);
            let (other, basis) = window.extent(off.0, off.1);
            let line = match (first, other) {
                (Some(a), Some(b)) => a.min(b),
                (a, b) => a.or(b)?,
            };
            let apart = window.held[test.0].line != window.held[off.0].line;
            return Some(Item {
                line,
                signal: EvidenceSignal::TestSkipped,
                basis: if apart { Basis::Hunk } else { rung.max(basis) },
                test_likely: false,
                forms: 0,
            });
        }
        at = close + 1;
    }
    None
}

/// How many names before the last one are passed over in `<a::b::Name>::function`
/// ([`released_path`]).
const OWNER_NAMES: usize = 8;

/// A path that release 0.8.3 keyed `process_spawn` or `network_fetch` on, when the current
/// token is the name it starts from: `Command::new` and `process::Command` (from `Command`),
/// `reqwest::` and `ureq::`, `TcpStream::connect`, `UdpSocket::bind`. That release looked for
/// each as one string on one line, and so does its rule here still; white space, a comment
/// or a line end between the names hid the path from it (`std::process :: Command :: new`,
/// or `Command` and `::new` on two lines: both compile and run; seventh independent review,
/// IR7-M-3). Read as tokens the path is the same names and `::` whatever stands between
/// them. Nothing more is modelled than the release keyed on: no `use` alias, no glob import.
///
/// The name and the function it is asked for are one path to the compiler in two more
/// spellings, and both are read: with an empty list of generic arguments between them
/// (`Command::<>::new`), and with the name alone in angle brackets (`<Command>::new`). Each
/// compiles and runs (eighth independent review, IR8-L-1: four lines that no version
/// reported). `Vec::<Command>::new` and a type `Foo<Command>` are other things: there the
/// `<` follows `::` or a name, and the path is not read.
///
/// In the angle brackets the name may be the last name of a path
/// (`<std::net::TcpStream>::connect`, `<self::UdpSocket>::bind`, `<p::Command>::new` after
/// `use std::process as p`; round 16 of Vesper's review, VD3R16-L-1, and the ninth
/// independent review, IR9-L-2): the release's rule keys on the last name too
/// (`tokio::net::TcpStream::connect` is its item), and the names before it are passed over
/// backwards, [`OWNER_NAMES`] at most, in the tokens the window already holds. Other
/// spellings of the same type are NOT read and are declared: in parentheses
/// (`<(Command)>::new`), with an empty list of generic arguments inside the brackets
/// (`<Command<>>::new`), and as an associated type (`<<Command as T>::Me>::new`).
///
/// The `<` opens such a path where it follows no name and no `:` (`= <Command>::new`,
/// `(<Command>::new`), and where it follows a keyword after which an expression begins
/// ([`Word::begins_expression`]: `return <Command>::new(…)`, `match`, `for _ in`, `break` with
/// or without its label, `yield`, `if`, `while`, `&mut`, `become`, and `do yeet`; tenth
/// independent review, IR10-L-2). After any other name it is a comparison or a type's generic
/// list (`a <Command> ::new(x)`).
///
/// A side's first token can go on from a line the side does not show, so a `<` at the very
/// start of a side follows nothing the reader sees and opens a path: when an unseen `Vec::`
/// stands above it, `<Command>::new()` there is a type argument and the item is false
/// (eleventh independent review, IR11-L-1, `vecturbo-u0.patch`: a writer's, since no
/// formatter breaks a line between `::` and `<`; reading it is the direction that detects).
fn released_path<'a, I: Iterator<Item = SideLine<'a>>>(
    window: &Window<'a, I>,
) -> Option<(EvidenceSignal, (usize, usize))> {
    use EvidenceSignal as S;
    let at = window.at;
    // the place of `name`, when the path goes on to it at `from`
    let to = |from: usize, name: Word| {
        (window.path(from) && window.word(from + 2) == Some(name)).then_some(from + 2)
    };
    // the name at `at` stands in angle brackets that open a path, alone or as the last name
    // of a path (`<std::net::TcpStream>`, `<::std::net::TcpStream>`)
    let bracketed = || {
        // `break 'l` before the `<`: the label's name, its quote, the keyword
        let labelled = |name: usize| {
            name.checked_sub(2).is_some_and(|keyword| {
                window.marks(keyword + 1, b'\'') && window.word(keyword) == Some(Word::Break)
            })
        };
        // `do yeet` before the `<`
        let yeeted = |name: usize| {
            window.word(name) == Some(Word::Yeet)
                && name
                    .checked_sub(1)
                    .is_some_and(|keyword| window.word(keyword) == Some(Word::Do))
        };
        let opens = |open: usize| {
            window.marks(open, b'<')
                && open
                    .checked_sub(1)
                    .is_none_or(|before| match window.word(before) {
                        Some(word) => {
                            word.begins_expression() || labelled(before) || yeeted(before)
                        }
                        None => !window.marks(before, b':'),
                    })
        };
        let mut first = at;
        for _ in 0..OWNER_NAMES {
            match first.checked_sub(3) {
                Some(owner) if window.path(owner + 1) && window.word(owner).is_some() => {
                    first = owner;
                }
                _ => break,
            }
        }
        if first
            .checked_sub(2)
            .is_some_and(|colons| window.path(colons))
        {
            first -= 2;
        }
        window.marks(at + 1, b'>') && first.checked_sub(1).is_some_and(opens)
    };
    let then = |name: Word| {
        to(at + 1, name)
            .or_else(|| {
                (window.path(at + 1) && window.marks(at + 3, b'<') && window.marks(at + 4, b'>'))
                    .then(|| to(at + 5, name))
                    .flatten()
            })
            .or_else(|| bracketed().then(|| to(at + 2, name)).flatten())
    };
    match window.word(at)? {
        Word::Command => {
            let owner = at.checked_sub(3).filter(|owner| {
                window.path(owner + 1) && window.word(*owner) == Some(Word::Process)
            });
            let called = then(Word::New);
            (owner.is_some() || called.is_some())
                .then(|| (S::ProcessSpawn, (owner.unwrap_or(at), called.unwrap_or(at))))
        }
        Word::Reqwest | Word::Ureq => window
            .path(at + 1)
            .then_some((S::NetworkFetch, (at, at + 2))),
        Word::TcpStream => then(Word::Connect).map(|end| (S::NetworkFetch, (at, end))),
        Word::UdpSocket => then(Word::Bind).map(|end| (S::NetworkFetch, (at, end))),
        _ => None,
    }
}

/// `#[allow(` and `#![allow(`, when the current token is `#`: what release 0.8.3 keyed
/// `check_suppressed` on, with anything between the tokens (a comment, a line end); and since
/// 0.11.0 `#[expect(` and `#![expect(`, which switch the lint off as `allow` does (and report
/// only when it would not have fired).
fn allowed<'a, I: Iterator<Item = SideLine<'a>>>(window: &Window<'a, I>) -> Option<(usize, usize)> {
    let at = window.at;
    let open = if window.marks(at + 1, b'!') {
        at + 2
    } else {
        at + 1
    };
    (window.marks(open, b'[')
        && matches!(window.word(open + 1), Some(Word::Allow | Word::Expect))
        && window.marks(open + 2, b'('))
    .then_some((at, open + 2))
}

/// The signals the reader gives, in the order of [`EvidenceSignal`], which is the order its
/// items come out in for one line.
const READ: [EvidenceSignal; 9] = [
    EvidenceSignal::ProcessSpawn,
    EvidenceSignal::NetworkFetch,
    EvidenceSignal::CheckSuppressed,
    EvidenceSignal::TestRemoved,
    EvidenceSignal::TestSkipped,
    EvidenceSignal::TrivialAssert,
    EvidenceSignal::StubAdded,
    EvidenceSignal::ErrorSwallowed,
    EvidenceSignal::PanicPathAdded,
];

/// Whether the reader looks for `signal` on the side of a hunk that `old_side` names: on the
/// old side for a removed test ([`read_old`]), on the new side for the others it gives
/// ([`read_new`]). For the vocabulary's sides of a signal (`crate::evidence::signal_sides`).
pub(crate) fn reads(signal: EvidenceSignal, old_side: bool) -> bool {
    READ.contains(&signal) && ((signal == EvidenceSignal::TestRemoved) == old_side)
}

/// What one line holds: for each of [`READ`], the basis and the hint of the constructs of that
/// signal found on it so far, taken together.
type OnLine = [Option<(Basis, bool, u8)>; READ.len()];

/// What the reader found, as it finds it: one entry for each line that holds something, and in
/// it one place for each signal. A construct found again on a line joins what is there.
///
/// So what is kept is the answer and never the matches: before, every match that did not
/// directly follow one like it was kept to the end and the whole list was sorted, and a line
/// of `x.unwrap(); todo!();` repeated over 16 MiB held 97 MiB for an answer of two items
/// (review V3 round 11, VD3R11-M-1). Nothing is sorted here. Constructs mostly come in the
/// order of the lines they are reported on. Two kinds can come late, for a line before the
/// last one reported on: a `cfg_attr` list, which is noted where it ends for the line its
/// first changed token is on; and a path call that reaches back over generic arguments
/// (`Result::<[u8; { x.unwrap() }]>::unwrap` over two lines is an item for its first line,
/// found after the `x.unwrap()` of its second: fifth independent review, IR5-L-3). Any such
/// item goes into `late`, a map by line, which is joined with `lines` at the end, both being
/// in order.
#[derive(Default)]
struct Found {
    /// The lines in the order they were first reported on, which is their order in the hunk.
    lines: Vec<(usize, OnLine)>,
    /// Items for a line before the last of `lines`.
    late: std::collections::BTreeMap<usize, OnLine>,
}

impl Found {
    fn note(&mut self, item: Item) {
        // Only the reader's own signals are ever noted.
        let Some(place) = READ.iter().position(|signal| *signal == item.signal) else {
            return;
        };
        let last = self.lines.last().map(|(line, _)| *line);
        let on_line = match last {
            Some(line) if line > item.line => self.late.entry(item.line).or_default(),
            Some(line) if line == item.line => {
                let at = self.lines.len() - 1;
                &mut self.lines[at].1
            }
            _ => {
                self.lines.push((item.line, OnLine::default()));
                let at = self.lines.len() - 1;
                &mut self.lines[at].1
            }
        };
        on_line[place] = Some(match on_line[place] {
            Some((basis, likely, forms)) => (
                basis.max(item.basis),
                likely && item.test_likely,
                forms | item.forms,
            ),
            None => (item.basis, item.test_likely, item.forms),
        });
    }

    /// Note `signal` for the construct `window` holds from `first` to `last`, when one of its
    /// tokens is on a changed line.
    fn construct<'a, I: Iterator<Item = SideLine<'a>>>(
        &mut self,
        window: &Window<'a, I>,
        signal: EvidenceSignal,
        (first, last): (usize, usize),
        (test_likely, forms): (bool, u8),
    ) {
        let (changed, basis) = window.extent(first, last);
        if let Some(line) = changed {
            self.note(Item {
                line,
                signal,
                basis,
                test_likely,
                forms,
            });
        }
    }

    /// The items by line and then by signal, one for each pair: `lines` and `late` are each in
    /// line order and are walked together once, so this costs the answer's length.
    fn sorted(self) -> Vec<Item> {
        fn join(into: &mut OnLine, from: OnLine) {
            for (place, more) in from.into_iter().enumerate() {
                into[place] = match (into[place], more) {
                    (Some((basis, likely, forms)), Some((other, also, more))) => {
                        Some((basis.max(other), likely && also, forms | more))
                    }
                    (was, more) => was.or(more),
                };
            }
        }
        let mut out = Vec::new();
        let mut push = |line: usize, on_line: OnLine| {
            for (signal, held) in READ.into_iter().zip(on_line) {
                if let Some((basis, test_likely, forms)) = held {
                    out.push(Item {
                        line,
                        signal,
                        basis,
                        test_likely,
                        forms,
                    });
                }
            }
        };
        let mut late = self.late.into_iter().peekable();
        for (line, mut on_line) in self.lines {
            while let Some((before, held)) = late.next_if(|(before, _)| *before < line) {
                push(before, held);
            }
            if let Some((_, held)) = late.next_if(|(same, _)| *same == line) {
                join(&mut on_line, held);
            }
            push(line, on_line);
        }
        for (line, held) in late {
            push(line, held);
        }
        out
    }
}

/// One `#[cfg_attr(…)]` list that is open: where the reader is in it.
#[derive(Debug, Clone, Copy)]
struct List {
    /// The hunk line its `#` is on.
    line: usize,
    /// The first changed line that holds one of its tokens so far.
    changed: Option<usize>,
    /// Brackets open inside the current list, other than a followed `cfg_attr(`.
    depth: usize,
    /// How many `cfg_attr(` lists that it applies are open around the current one. Each was
    /// entered from an argument after its list's condition, so that is all there is to
    /// remember of them.
    levels: usize,
    /// Which argument of the current list this is: 0 is the condition.
    argument: usize,
    /// The current argument was a followed `cfg_attr(…)`, now closed: it is not `ignore`,
    /// whatever stands between it and the next comma.
    spent: bool,
    /// How many tokens the current argument has so far outside its own brackets (2 for more).
    tokens: u8,
    /// The first of them, when it is a name.
    head: Option<Word>,
    /// The argument is `ignore` so far, and the next token says whether it is an attribute of
    /// its own: the first changed line of the list up to that `ignore`, and the line the
    /// `ignore` is on.
    candidate: Option<(Option<usize>, usize)>,
    /// The argument is `allow` or `expect` so far, and a `(` next makes it the lint attribute:
    /// as `candidate`.
    lint: Option<(Option<usize>, usize)>,
    /// The suppression it holds so far, as `skip` (since 0.11.0, F2b).
    suppression: Option<(usize, Basis)>,
    /// The skip it holds so far: the first changed line of the list up to an `ignore` that is
    /// an attribute of its own, and the rung. It is noted only when the list closes and the
    /// attribute's `]` follows (a list past [`CFG_ATTR_LISTS`] is not followed at all).
    skip: Option<(usize, Basis)>,
    /// Its closing parenthesis has been read.
    closed: bool,
}

impl List {
    fn begin_argument(&mut self) {
        self.tokens = 0;
        self.head = None;
    }

    fn count_token(&mut self) {
        self.tokens = self.tokens.saturating_add(1).min(2);
    }
}

/// The conditional skips of a side: `#[cfg_attr(condition, …)]` that applies `ignore`, as an
/// attribute of its own (`ignore`, `ignore = "…"`) and not as another attribute's argument
/// (`cfg_attr(feature = "serde", serde(skip, ignore))` skips no test; independent review,
/// IR-L-2), also through a `cfg_attr` that a `cfg_attr` applies (`cfg_attr(all(),
/// cfg_attr(all(), ignore))`; second independent review, IR2-M-2). The condition is no
/// attribute, whatever it names.
///
/// Each open list is followed token by token as the tokens pass, the innermost one only, so a
/// token is read once however many lists are open: read from each list's own start, a line that
/// is nothing but `#[cfg_attr(` cost the square of its length (880 KB took 32 s). At most
/// [`CFG_ATTR_LISTS`] are followed at once.
///
/// The construct is the list from its `#` to the `ignore`, so its condition is inside: a
/// change to the condition is a change to the skip (`miri` to `all()`: the test now runs
/// nowhere; independent review, IR-M-3).
///
/// An `ignore` is the attribute when a comma, the list's closing parenthesis, `=` or `(`
/// follows it (`cfg_attr(all(), ignore(flaky))` switches the test off where the lint
/// `ill_formed_attribute_input` is allowed: see [`ignored`]). The skip is read only when the
/// side's text holds the attribute whole, to the list's `)` and the `]` after it (so an
/// `ignore(` to its matching `)`): `ignore` can go on as `ignore::case`, and in a macro
/// `cfg_attr(all(), ignore(` goes on as anything (VD3R18-M-1). The one exception: a list opened
/// past [`CFG_ATTR_LISTS`] is not followed and is taken to hold a skip, with no `ignore` and no
/// `]` (a writer's false item; round 19 of Vesper's review, VD3R19-M-1; IR12-L-2).
///
/// By the same rule, since 0.11.0 (F2b), the conditional suppressions: an argument after a
/// condition that is `allow(` or `expect(` (`cfg_attr(test, allow(dead_code))`, nested too)
/// switches lints off where the condition holds, and is noted as `check_suppressed` when the
/// attribute is whole; `serde(allow(…))`, another attribute's argument, is not. A list past
/// [`CFG_ATTR_LISTS`] is taken to hold no suppression: an allowance nested that deep is not
/// read (taking it, like the skip, would add a writer's false item, VD3R19-M-1).
#[derive(Default)]
struct Skips {
    open: Vec<List>,
}

impl Skips {
    /// The skip of `list`, whose `ignore` is on line `ignore_line`, when a token of it up to
    /// there is on a changed line (`changed`, the first such line).
    fn skip(list: &List, changed: Option<usize>, ignore_line: usize) -> Option<(usize, Basis)> {
        let basis = if list.line == ignore_line {
            Basis::Lexical
        } else {
            Basis::Hunk
        };
        changed.map(|line| (line, basis))
    }

    /// Note a skip, or a suppression.
    fn note(found: &mut Found, signal: EvidenceSignal, (line, basis): (usize, Basis)) {
        found.note(Item {
            line,
            signal,
            basis,
            test_likely: false,
            forms: 0,
        });
    }

    /// Read the current token of `window`.
    fn read<'a, I: Iterator<Item = SideLine<'a>>>(
        &mut self,
        window: &Window<'a, I>,
        found: &mut Found,
    ) {
        let Some(token) = window.here() else { return };
        let closed = self.open.last_mut().is_some_and(|list| {
            Self::follow(list, token);
            list.closed
        });
        if closed && let Some(inner) = self.open.pop() {
            // the attribute is whole: its list closed here, and its `]` follows
            if window.marks(window.at + 1, b']') {
                if let Some(skip) = inner.skip {
                    Self::note(found, EvidenceSignal::TestSkipped, skip);
                }
                if let Some(suppression) = inner.suppression {
                    Self::note(found, EvidenceSignal::CheckSuppressed, suppression);
                }
            }
            if let Some(around) = self.open.last_mut() {
                // the inner list's `(` was a bracket of the list around it, and so were its
                // tokens
                around.depth = around.depth.saturating_sub(1);
                around.changed = around.changed.or(inner.changed);
            }
        }
        if token.what != What::Mark(b'(') {
            return;
        }
        // `#[cfg_attr(` or `#![cfg_attr(` ends here
        let at = window.at;
        let Some(bracket) = at.checked_sub(2) else {
            return;
        };
        if window.word(at - 1) != Some(Word::CfgAttr) || !window.marks(bracket, b'[') {
            return;
        }
        let hash = match bracket.checked_sub(1) {
            Some(before) if window.marks(before, b'#') => before,
            Some(before) if window.marks(before, b'!') => match before.checked_sub(1) {
                Some(hash) if window.marks(hash, b'#') => hash,
                _ => return,
            },
            _ => return,
        };
        let (changed, _) = window.extent(hash, at);
        let list = List {
            line: window.held[hash].line,
            changed,
            depth: 0,
            levels: 0,
            argument: 0,
            spent: false,
            tokens: 0,
            head: None,
            candidate: None,
            lint: None,
            suppression: None,
            skip: None,
            closed: false,
        };
        if self.open.len() >= CFG_ATTR_LISTS {
            // not followed: taken to hold a skip (and no suppression: a list that deep is
            // a declared limit of that reading)
            if let Some(skip) = Self::skip(&list, changed, token.line) {
                Self::note(found, EvidenceSignal::TestSkipped, skip);
            }
            return;
        }
        self.open.push(list);
    }

    /// One token of the innermost open list.
    fn follow(list: &mut List, token: Token<'_>) {
        if let Some((changed, line)) = list.candidate.take()
            && matches!(token.what, What::Mark(b',' | b')' | b'=' | b'('))
            && let Some((first, basis)) = Self::skip(list, changed, line)
        {
            // every `ignore` of the list has the same first changed line: it is set once
            list.skip = Some(
                list.skip
                    .map_or((first, basis), |(at, was)| (at, was.max(basis))),
            );
        }
        if let Some((changed, line)) = list.lint.take()
            && token.what == What::Mark(b'(')
            && let Some((first, basis)) = Self::skip(list, changed, line)
        {
            list.suppression = Some(
                list.suppression
                    .map_or((first, basis), |(at, was)| (at, was.max(basis))),
            );
        }
        if token.changed && list.changed.is_none() {
            list.changed = Some(token.line);
        }
        match token.what {
            What::Name(_) | What::Mark(_) if list.depth > 0 => match token.what {
                What::Mark(b'(' | b'[' | b'{') => list.depth += 1,
                What::Mark(b')' | b']' | b'}') => list.depth -= 1,
                _ => {}
            },
            What::Name(word) => {
                if list.tokens == 0 {
                    list.head = Some(word);
                    if list.argument > 0 && !list.spent {
                        match word {
                            Word::Ignore => list.candidate = Some((list.changed, token.line)),
                            Word::Allow | Word::Expect => {
                                list.lint = Some((list.changed, token.line));
                            }
                            _ => {}
                        }
                    }
                }
                list.count_token();
            }
            What::Mark(b'(')
                if list.argument > 0
                    && !list.spent
                    && list.tokens == 1
                    && list.head == Some(Word::CfgAttr) =>
            {
                // a `cfg_attr(` that this list applies: its arguments are read by the same rule
                list.levels += 1;
                list.argument = 0;
                list.begin_argument();
            }
            What::Mark(b'(' | b'[' | b'{') => {
                list.count_token();
                list.depth += 1;
            }
            What::Mark(b',') => {
                list.argument += 1;
                list.spent = false;
                list.begin_argument();
            }
            What::Mark(b')') if list.levels == 0 => list.closed = true,
            What::Mark(b')') => {
                // back in the list around it, in an argument after its condition
                list.levels -= 1;
                list.argument = 1;
                list.spent = true;
                list.begin_argument();
            }
            What::Mark(_) => list.count_token(),
        }
    }
}

/// Where the reader is in an attribute, for the hint.
#[derive(Debug, Clone, Copy)]
struct Attribute {
    /// Brackets open, the attribute's own `[` counted.
    depth: usize,
    /// Still in the path: nothing but names and `::` so far.
    in_path: bool,
    /// The last name of the path, when it has one.
    last: Option<Word>,
    /// The path holds a `::`.
    colons: bool,
    /// The path is the one name `cfg`.
    cfg: bool,
    /// How much of `(`, `test`, `)` has followed the path, in that order with nothing else.
    arguments: Arguments,
}

/// How far an attribute's arguments are `(test)` and nothing more.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Arguments {
    /// None yet.
    None,
    /// `(`
    Opened,
    /// `(test`
    Test,
    /// `(test)`
    Closed,
    /// Anything else.
    Other,
}

impl Attribute {
    /// Whether it spells a test, once it is closed.
    fn spells_test(&self) -> bool {
        matches!(self.last, Some(Word::Test | Word::Rstest))
            || (self.cfg && self.arguments == Arguments::Closed)
    }

    /// One token of the attribute, after its `#`. `false` when the token closes it.
    fn read(&mut self, token: Token<'_>) -> bool {
        let top = self.depth == 1;
        match token.what {
            // the attribute's own `[`
            What::Mark(b'[') if self.depth == 0 => self.depth = 1,
            What::Mark(mark @ (b'(' | b'[' | b'{')) => {
                self.arguments = if top && self.arguments == Arguments::None && mark == b'(' {
                    Arguments::Opened
                } else {
                    Arguments::Other
                };
                self.in_path = false;
                self.depth += 1;
            }
            What::Mark(mark @ (b')' | b']' | b'}')) => {
                self.depth = self.depth.saturating_sub(1);
                if self.depth == 0 {
                    return false;
                }
                self.arguments =
                    if self.depth == 1 && self.arguments == Arguments::Test && mark == b')' {
                        Arguments::Closed
                    } else {
                        Arguments::Other
                    };
            }
            What::Name(word) if top && self.in_path => {
                self.cfg = !self.colons && self.last.is_none() && word == Word::Cfg;
                self.last = Some(word);
            }
            // a path's `::`: `a::cfg(test)` and `::cfg(test)` are not `cfg(test)`
            What::Mark(b':') if top && self.in_path => {
                self.colons = true;
                self.cfg = false;
            }
            What::Name(Word::Test) if self.depth == 2 && self.arguments == Arguments::Opened => {
                self.arguments = Arguments::Test;
            }
            _ => {
                self.in_path &= !top;
                self.arguments = Arguments::Other;
            }
        }
        true
    }
}

/// The stretches of a side that stand inside the braces of an item whose attribute spells a
/// test: the last name of its path is `test` (`#[test]`, `#[tokio::test]`) or `rstest`, or it
/// is exactly `#[cfg(test)]`. Other attributes may stand between it and the item. A panic path
/// found in such a stretch says so ([`Item::test_likely`]).
///
/// It is a hint for a reader and never proof, and nothing but that one member comes of it:
/// every one of these names but `cfg` can be bound by an import to a macro that leaves the
/// function in the program, an attribute macro around the item may drop the `cfg`, a hunk can
/// begin under a comment that ends on the attribute's own line, and braces are only counted
/// here, in the code view's reading of what is code.
#[derive(Default)]
struct TestStretch {
    /// Braces open.
    depth: usize,
    /// The depth the outermost stretch is open at.
    inside: Option<usize>,
    /// A test's attribute has been read and its item has not begun.
    pending: bool,
    /// Parentheses and square brackets open since then: a `;` or a `{` inside them is not the
    /// item's.
    nested: usize,
    attribute: Option<Attribute>,
}

impl TestStretch {
    /// Whether the token just read stands in a stretch.
    const fn holds(&self) -> bool {
        self.inside.is_some()
    }

    /// Read the current token of `window`.
    fn read<'a, I: Iterator<Item = SideLine<'a>>>(&mut self, window: &Window<'a, I>) {
        let Some(token) = window.here() else { return };
        if let Some(attribute) = &mut self.attribute {
            if !attribute.read(token) {
                if attribute.spells_test() {
                    self.pending = true;
                    self.nested = 0;
                }
                self.attribute = None;
            }
            return;
        }
        let What::Mark(mark) = token.what else { return };
        match mark {
            // an outer attribute: an inner one (`#![…]`) belongs to what is around it
            b'#' if window.marks(window.at + 1, b'[') => {
                self.attribute = Some(Attribute {
                    depth: 0,
                    in_path: true,
                    last: None,
                    colons: false,
                    cfg: false,
                    arguments: Arguments::None,
                });
            }
            b'(' | b'[' => self.nested += 1,
            b')' | b']' => {
                // closed without having been opened since the attribute: the attribute stood
                // inside these brackets, on something that is no item
                self.pending &= self.nested > 0;
                self.nested = self.nested.saturating_sub(1);
            }
            b';' if self.nested == 0 => self.pending = false,
            b'{' => {
                self.depth += 1;
                if self.pending && self.nested == 0 {
                    self.pending = false;
                    if self.inside.is_none() {
                        self.inside = Some(self.depth);
                    }
                }
            }
            b'}' => {
                self.pending = false;
                if self.inside == Some(self.depth) {
                    self.inside = None;
                }
                self.depth = self.depth.saturating_sub(1);
            }
            _ => {}
        }
    }
}

/// The signals of the new side of a hunk, on its added lines: `lines` are its unchanged and
/// added lines in order. Sorted by line and then by signal, one item for each pair.
///
/// A construct is evidence when at least one of its tokens is on an added line, and it is
/// reported on the first such line. So an assertion whose condition became `true` is one, on
/// the line of the `true`; a conditional skip whose condition changed is one; and an assertion
/// that was trivial before and only got another message is none, since a string is blank and
/// the tokens of `assert!(true,` did not change.
///
/// The signals release 0.8.3 keyed on a path or on `#[allow(` are read here too
/// ([`released_path`], [`allowed`]), beside its own rules for them, which read one line.
///
/// A hunk side is read as text that ends there; nothing is inferred from how it ends. A
/// construct of which the side shows only the beginning is not read (see the head of this
/// module: rounds 15 to 17 read some forms there, and round 18 a skip's attribute up to its
/// `(`, and grammar broke each).
pub(crate) fn read_new<'a>(lines: impl Iterator<Item = SideLine<'a>>) -> Vec<Item> {
    use EvidenceSignal as S;
    let mut window = Window::over(lines);
    let mut found = Found::default();
    let mut skips = Skips::default();
    let mut tests = TestStretch::default();
    while let Some(token) = window.here() {
        tests.read(&window);
        skips.read(&window, &mut found);
        if token.what == What::Mark(b'#')
            && let Some(item) = compiled_out(&window)
        {
            found.note(item);
        }
        let construct = match token.what {
            What::Name(Word::Unwrap | Word::Expect) => {
                panic_path(&window).map(|extent| (S::PanicPathAdded, extent))
            }
            // the reading of release 0.9.0 stays one reading (`trivial_assert`), and the class
            // is another (`cannot_fail`, since 0.11.0)
            What::Name(Word::Assert | Word::DebugAssert) => trivial_assert(&window, 1)
                .or_else(|| cannot_fail(&window, Asks::True))
                .map(|extent| (S::TrivialAssert, extent)),
            What::Name(Word::AssertEq | Word::DebugAssertEq) => trivial_assert(&window, 2)
                .or_else(|| cannot_fail(&window, Asks::Equal))
                .map(|extent| (S::TrivialAssert, extent)),
            What::Name(Word::AssertNe | Word::DebugAssertNe) => {
                cannot_fail(&window, Asks::Unequal).map(|extent| (S::TrivialAssert, extent))
            }
            What::Name(Word::Todo | Word::Unimplemented) => {
                stub(&window).map(|extent| (S::StubAdded, extent))
            }
            What::Name(Word::Panic | Word::Unreachable) => {
                stub(&window).map(|extent| (S::PanicPathAdded, extent))
            }
            What::Name(Word::Let | Word::Underscore) => {
                dropped(&window).map(|extent| (S::ErrorSwallowed, extent))
            }
            What::Mark(b'#') => ignored(&window)
                .map(|extent| (S::TestSkipped, extent))
                .or_else(|| allowed(&window).map(|extent| (S::CheckSuppressed, extent))),
            What::Name(
                Word::Command | Word::Reqwest | Word::Ureq | Word::TcpStream | Word::UdpSocket,
            ) => released_path(&window),
            _ => None,
        };
        if let Some((signal, extent)) = construct {
            let likely = signal == S::PanicPathAdded && tests.holds();
            let forms = match token.what {
                _ if signal != S::PanicPathAdded => 0,
                What::Name(Word::Panic | Word::Unreachable) => BY_MACRO,
                _ => BY_CALL,
            };
            found.construct(&window, signal, extent, (likely, forms));
        }
        window.step();
    }
    found.sorted()
}

/// The signals of the old side of a hunk, on its removed lines: `lines` are its unchanged and
/// removed lines in order. A test's attribute with at least one token on a removed line is a
/// removed test, on the first such line. As on the new side, nothing is inferred from how the
/// side ends (`#[rstest` goes on as `#[rstest::fixture]`, `#[tokio::` as `#[tokio::main]`).
pub(crate) fn read_old<'a>(lines: impl Iterator<Item = SideLine<'a>>) -> Vec<Item> {
    let mut window = Window::over(lines);
    let mut found = Found::default();
    while let Some(token) = window.here() {
        if token.what == What::Mark(b'#')
            && let Some(extent) = test_attribute(&window, window.at)
        {
            found.construct(&window, EvidenceSignal::TestRemoved, extent, (false, 0));
        }
        window.step();
    }
    found.sorted()
}

#[cfg(test)]
mod tests {
    use super::*;
    use EvidenceSignal as S;

    /// Review V3 round 11, VD3R11-M-1: what the reader keeps is its answer, not its matches.
    /// A line of `x.unwrap(); todo!();` repeated held one item for each construct to the end,
    /// and they were sorted: 16 MiB of it took 97 MiB for an answer of two items. Here 200,000
    /// constructs on one line, and as many again on a second, leave two lines of two signals
    /// each; a conditional skip that ends late for an earlier line is one late entry, however
    /// many such lists there are; and the answer is in order.
    #[test]
    fn what_is_kept_is_the_answer_and_not_the_matches() {
        assert!(
            READ.is_sorted(),
            "items of one line come out in READ's order"
        );
        let mut found = Found::default();
        let item = |line, signal, basis, test_likely| Item {
            line,
            signal,
            basis,
            test_likely,
            forms: u8::from(signal == S::PanicPathAdded),
        };
        for line in [3usize, 7] {
            for round in 0..100_000 {
                found.note(item(line, S::PanicPathAdded, Basis::Lexical, round != 5));
                found.note(item(line, S::StubAdded, Basis::Lexical, false));
            }
        }
        // lists that end on line 7 and were first changed on lines 5 and 1, many times over
        for _ in 0..50_000 {
            found.note(item(5, S::TestSkipped, Basis::Hunk, false));
            found.note(item(1, S::TestSkipped, Basis::Lexical, false));
            found.note(item(3, S::TestSkipped, Basis::Hunk, false));
        }
        found.note(item(9, S::TrivialAssert, Basis::Hunk, false));
        assert_eq!((found.lines.len(), found.late.len()), (3, 3));
        assert_eq!(
            found.sorted(),
            [
                item(1, S::TestSkipped, Basis::Lexical, false),
                item(3, S::TestSkipped, Basis::Hunk, false),
                item(3, S::StubAdded, Basis::Lexical, false),
                // one of the panic paths on the line was outside a test: no hint for the line
                item(3, S::PanicPathAdded, Basis::Lexical, false),
                item(5, S::TestSkipped, Basis::Hunk, false),
                item(7, S::StubAdded, Basis::Lexical, false),
                item(7, S::PanicPathAdded, Basis::Lexical, false),
                item(9, S::TrivialAssert, Basis::Hunk, false),
            ]
        );
        // and through the reader itself: one added line of alternating constructs
        let text = "x.unwrap(); todo!(); ".repeat(50_000);
        let lines = [SideLine {
            index: 0,
            code: &text,
            changed: true,
        }];
        assert_eq!(
            read_new(lines.into_iter()),
            [
                item(0, S::StubAdded, Basis::Lexical, false),
                item(0, S::PanicPathAdded, Basis::Lexical, false),
            ]
        );
    }

    /// A side from lines written as in a patch: `+` or `-` first for a line the change made, a
    /// space for an unchanged one.
    fn side<'a>(lines: &[&'a str]) -> Vec<(usize, &'a str, bool)> {
        lines
            .iter()
            .enumerate()
            .map(|(index, line)| (index, &line[1..], !line.starts_with(' ')))
            .collect()
    }

    fn lines_of<'a>(side: &'a [(usize, &'a str, bool)]) -> impl Iterator<Item = SideLine<'a>> {
        side.iter().map(|(index, code, changed)| SideLine {
            index: *index,
            code,
            changed: *changed,
        })
    }

    /// One item as the tests name it: its signal, its basis and the index of its line.
    type Found3 = (EvidenceSignal, Basis, usize);

    /// What the new side `lines` gives.
    fn new(lines: &[&str]) -> Vec<Found3> {
        read_new(lines_of(&side(lines)))
            .iter()
            .map(|item| (item.signal, item.basis, item.line))
            .collect()
    }

    /// The signals of the new side `lines`.
    fn signals(lines: &[&str]) -> Vec<EvidenceSignal> {
        new(lines).iter().map(|item| item.0).collect()
    }

    /// What the old side `lines` gives.
    fn old(lines: &[&str]) -> Vec<Found3> {
        read_old(lines_of(&side(lines)))
            .iter()
            .map(|item| (item.signal, item.basis, item.line))
            .collect()
    }

    /// The hints of the panic paths of the new side `lines`: `(line index, test_likely)`.
    fn hints(lines: &[&str]) -> Vec<(usize, bool)> {
        read_new(lines_of(&side(lines)))
            .iter()
            .filter(|item| item.signal == S::PanicPathAdded)
            .map(|item| (item.line, item.test_likely))
            .collect()
    }

    fn tokens<'a>(lines: &[&'a str]) -> Vec<Token<'a>> {
        let side: Vec<SideLine<'a>> = side(lines)
            .into_iter()
            .map(|(index, code, changed)| SideLine {
                index,
                code,
                changed,
            })
            .collect();
        Tokens {
            lines: side.into_iter(),
            line: None,
            at: 0,
        }
        .collect()
    }

    #[test]
    fn a_side_is_cut_into_names_and_single_marks_with_their_places() {
        let found = tokens(&["+  x.unwrap_or(1)", " ", "+\t#![a2b::  _c]"]);
        let places: Vec<(What, usize, usize, bool)> = found
            .iter()
            .map(|token| (token.what, token.line, token.column, token.changed))
            .collect();
        let name = What::Name(Word::Other);
        assert_eq!(
            places,
            [
                (name, 0, 2, true),
                (What::Mark(b'.'), 0, 3, true),
                (name, 0, 4, true),
                (What::Mark(b'('), 0, 13, true),
                (name, 0, 14, true),
                (What::Mark(b')'), 0, 15, true),
                (What::Mark(b'#'), 2, 1, true),
                (What::Mark(b'!'), 2, 2, true),
                (What::Mark(b'['), 2, 3, true),
                (name, 2, 4, true),
                (What::Mark(b':'), 2, 7, true),
                (What::Mark(b':'), 2, 8, true),
                (name, 2, 11, true),
                (What::Mark(b']'), 2, 13, true),
            ]
        );
        // an unchanged line's tokens say so, and an empty side has none
        assert!(tokens(&[" a b"]).iter().all(|token| !token.changed));
        assert_eq!(tokens(&[]), []);
        assert_eq!(tokens(&["+", "+   ", " \t"]), []);
    }

    /// A word is a whole name: a longer one is another word, and so is one with a character
    /// that is not ASCII in it.
    #[test]
    fn a_word_is_the_whole_name() {
        let words = |line: &str| -> Vec<Word> {
            tokens(&[line])
                .iter()
                .filter_map(|token| match token.what {
                    What::Name(word) => Some(word),
                    What::Mark(_) => None,
                })
                .collect()
        };
        assert_eq!(
            words("+unwrap expect assert debug_assert assert_eq debug_assert_eq todo"),
            [
                Word::Unwrap,
                Word::Expect,
                Word::Assert,
                Word::DebugAssert,
                Word::AssertEq,
                Word::DebugAssertEq,
                Word::Todo
            ]
        );
        assert_eq!(
            words("+unimplemented ignore cfg_attr cfg test rstest tokio true Result Option"),
            [
                Word::Unimplemented,
                Word::Ignore,
                Word::CfgAttr,
                Word::Cfg,
                Word::Test,
                Word::Rstest,
                Word::Tokio,
                Word::True,
                Word::Result,
                Word::Option
            ]
        );
        assert_eq!(
            words("+assert_ne debug_assert_ne false NAN"),
            [Word::AssertNe, Word::DebugAssertNe, Word::False, Word::Nan]
        );
        assert_eq!(
            words("+unwrap_or unwrapé éunwrap 1unwrap ignored Test result _todo todo_ assert_nee"),
            [Word::Other; 10]
        );
    }

    /// The compiler takes eleven characters for white space, five of them not ASCII. Read as
    /// part of a name, a left-to-right mark after `unwrap` made another word of it, and the
    /// call (which compiles) was not found.
    #[test]
    fn the_compilers_white_space_separates_tokens() {
        for space in [
            "\t", "\u{b}", "\u{c}", "\r", " ", "\u{85}", "\u{200e}", "\u{200f}", "\u{2028}",
            "\u{2029}",
        ] {
            let line = format!("+x{space}.{space}unwrap{space}({space})");
            assert_eq!(signals(&[&line]), [S::PanicPathAdded], "{space:?}");
            let line = format!("+assert{space}!{space}({space}true{space})");
            assert_eq!(signals(&[&line]), [S::TrivialAssert], "{space:?}");
        }
        // other characters that are not ASCII are part of the name they stand in
        for joined in ["\u{a0}", "\u{200b}", "\u{feff}", "é"] {
            let line = format!("+x.unwrap{joined}()");
            assert_eq!(signals(&[&line]), [], "{joined:?}");
        }
    }

    /// The window holds a few tokens around the current one and drops the rest as it goes.
    /// A construct is found wherever it stands in the side: before the first slide, on it,
    /// and with its tokens on both sides of it.
    #[test]
    fn a_construct_is_found_wherever_the_window_slides() {
        for before in (0..3 * (SLIDE + BEHIND))
            .step_by(7)
            .chain(SLIDE..SLIDE + 2 * BEHIND + AHEAD)
        {
            let line = format!(
                "+{}x.unwrap::<u8>() #[ignore] assert!(true)",
                "a ".repeat(before)
            );
            assert_eq!(
                signals(&[&line]),
                [S::TestSkipped, S::TrivialAssert, S::PanicPathAdded],
                "{before} tokens before"
            );
        }
        // and the window never holds the side: a million tokens pass through a few
        let line = format!("+{}", "} ".repeat(1_000_000));
        let side = side(&[&line]);
        let mut window = Window::over(lines_of(&side));
        let mut most = 0;
        while window.here().is_some() {
            most = most.max(window.held.len());
            window.step();
        }
        assert!(most <= SLIDE + BEHIND + AHEAD + 1, "{most}");
    }

    /// Third independent review, IR3-M-5, and review V3 rounds 3, 8 and 9: a call of `unwrap`
    /// or `expect`, as a method or by a path, whatever its spacing, with generic arguments or
    /// without, on one line or over several.
    #[test]
    fn a_panic_path_is_a_method_call_a_path_call_or_the_standard_function_as_a_value() {
        for line in [
            "+x.unwrap()",
            "+x . unwrap ( )",
            "+x.expect(\"\")",
            "+b().unwrap::<>()",
            "+b().expect::<'static>(\"\")",
            "+b().unwrap :: < Vec<Vec<u8>> > ()",
            "+b().unwrap::<fn(u8) -> u8>()",
            "+b().unwrap::<{ N > 1 }>()",
            "+b().unwrap::<[u8; 4]>()",
            "+X::unwrap(x)",
            "+<Option<u32>>::unwrap(x)",
            "+Result::<u32, E>::unwrap(r)",
            "+a::b::unwrap::<u8>(x)",
            "+self::expect()",
            "+Wrapper :: expect (x, \"\")",
            "+$x.unwrap()",
            "+it.map(Result::unwrap)",
            "+it.map(Option :: expect)",
            "+let f = Option::<u32>::unwrap;",
            "+let f = Result::<fn() -> u8, E>::expect;",
            "+let f = std::option::Option::unwrap;",
            "+1..unwrap()",
        ] {
            assert_eq!(
                new(&[line]),
                [(S::PanicPathAdded, Basis::Lexical, 0)],
                "{line}"
            );
        }
        for line in [
            "+x.unwrap_or(1)",
            "+x.expect_err(\"\")",
            "+unwrap(x)",
            "+x.unwrap",
            "+x.unwrap;",
            "+x.unwrap::<u8>",
            "+fn unwrap(self) {}",
            "+a::unwrap::b(x)",
            "+X::unwrap!(x)",
            "+use a::expect;",
            "+use b::{c, unwrap};",
            "+fn f(a: a::unwrap) {}",
            "+pub(in a::unwrap) fn f() {}",
            "+let v = X::unwrap { a: 1 };",
            "+it.map(Result::unwrap_or_default)",
            "+it.map(MyOption::unwrap)",
            "+it.map(Results::unwrap)",
            // not `::`: the two colons are apart
            "+X: :unwrap(x)",
            "+X::unwrap: :<u8>(x)",
            "+it.map(Option: :unwrap)",
            // generic arguments that do not close, or close a bracket they did not open
            "+x.unwrap::<u8()",
            "+x.unwrap::<u8)>()",
            "+let f = Option::<u32)>::unwrap;",
        ] {
            assert_eq!(new(&[line]), [], "{line}");
        }
        // Declared: a function of the patch's own type passed as a value is not read. The
        // parser named any path used as a value (review V3 round 9); nothing in the tokens
        // tells this one from the imports, types and longer paths above.
        assert_eq!(new(&["+it.map(X::unwrap)"]), []);
    }

    /// Generic arguments longer than the bound are not passed over, forwards or backwards, and
    /// the bound is exact.
    #[test]
    fn generic_arguments_are_passed_over_up_to_the_bound() {
        // `<`, names and `>`: `fill + 2` tokens
        let call = |fill: usize| format!("+x.unwrap::<{}>()", "a ".repeat(fill));
        assert_eq!(signals(&[&call(TURBOFISH_TOKENS - 2)]), [S::PanicPathAdded]);
        assert_eq!(signals(&[&call(TURBOFISH_TOKENS - 1)]), []);
        let value = |fill: usize| format!("+let f = Option::<{}>::unwrap;", "a ".repeat(fill));
        assert_eq!(
            signals(&[&value(TURBOFISH_TOKENS - 2)]),
            [S::PanicPathAdded]
        );
        assert_eq!(signals(&[&value(TURBOFISH_TOKENS - 1)]), []);
        // nested ones count as they are, and an arrow closes nothing
        assert_eq!(
            signals(&["+x.unwrap::<A<B<C>>, fn(D) -> E<F>>()"]),
            [S::PanicPathAdded]
        );
        assert_eq!(signals(&["+x.unwrap::<A<B<C>>()"]), []);
        assert_eq!(signals(&["+x.unwrap::<fn() ->()"]), []);
    }

    /// A construct is evidence when one of its tokens is on a changed line, and it is reported
    /// on the first such line; its basis says whether its tokens are on one line.
    #[test]
    fn a_construct_is_evidence_where_its_first_changed_token_is() {
        let cases: &[(&[&str], &[Found3])] = &[
            // all on unchanged lines: it was there before
            (&[" x.unwrap();", "+let a = 1;"], &[]),
            (&[" assert!(", "     true", " );", "+// x"], &[]),
            // one line
            (&["+x.unwrap();"], &[(S::PanicPathAdded, Basis::Lexical, 0)]),
            // over lines: on the first changed line that holds one of its tokens
            (
                &["+x", "+    .unwrap", "+    ();"],
                &[(S::PanicPathAdded, Basis::Hunk, 1)],
            ),
            (
                &[" x", "     .unwrap", "+    ();"],
                &[(S::PanicPathAdded, Basis::Hunk, 2)],
            ),
            (
                &[" x.", "+    unwrap", "     ();"],
                &[(S::PanicPathAdded, Basis::Hunk, 1)],
            ),
            // the receiver is not a token of the call
            (&["+x", "     .unwrap()"], &[]),
            // review V3 round 1, VD3-M-1: the condition became `true`
            (
                &[" assert!(", "+    true", " );"],
                &[(S::TrivialAssert, Basis::Hunk, 1)],
            ),
            // review V3 round 2, VD3R2-M-1: only what follows the construct changed
            (&[" assert!(", "     true,", "+    \"   \"", " );"], &[]),
            (&[" todo!(", "+    \"   \"", " )"], &[]),
            // the owner of a standard function used as a value is a token of it
            (
                &["+it.map(Result::", "     unwrap)"],
                &[(S::PanicPathAdded, Basis::Hunk, 0)],
            ),
            // but the owner of a call by a path is not
            (
                &["+X::", "     unwrap(x)"],
                &[(S::PanicPathAdded, Basis::Hunk, 0)],
            ),
            (&["+X", "     ::unwrap(x)"], &[]),
        ];
        for (lines, expected) in cases {
            assert_eq!(new(lines), *expected, "{lines:?}");
        }
    }

    /// Review V3 round 3, VD3R3-H-2, the independent review, IR-M-2, and the third, IR3-M-4:
    /// `assert!(true)` and its forms whatever the spacing, the bracket and the lines, with a
    /// comment inside (blank in the code view), and nothing that only begins like one.
    #[test]
    fn a_trivial_assertion_is_read_token_by_token() {
        for lines in [
            &["+assert!(true);"][..],
            &["+assert ! ( true ) ;"],
            &["+assert!(true, \"\");"],
            &["+debug_assert!(true);"],
            &["+assert_eq!(true, true);"],
            &["+assert_eq ! (true , true , \"\");"],
            &["+debug_assert_eq!(true,true);"],
            &["+assert!{true};"],
            &["+assert![true];"],
            &["+assert_eq!{true, true};"],
            &["+std::assert!(true);"],
            &["+$crate::assert!(true);"],
            &["+assert", "+!", "+(", "+true", "+)"],
            // IR3-M-4: the comments are blank
            &["+assert!(", "+    true          ", "+);"],
            &["+assert!(true             , \"\");"],
            &[
                "+assert_eq!(",
                "+    true,               ",
                "+    true",
                "+);",
            ],
        ] {
            assert_eq!(signals(lines), [S::TrivialAssert], "{lines:?}");
        }
        for line in [
            "+assert!(true_enough());",
            "+assert!(true == x);",
            "+assert!(true && x);",
            "+assert!(true",
            "+assert!(true]",
            "+assert!{true)",
            "+assert(true);",
            "+assert_eq!(true);",
            "+assert_eq!(true, x);",
            "+assert_eq!(true, true",
            "+assert_ne!(true, true);",
            "+my_assert!(true);",
            "+assert!true;",
            "+it.map(assert);",
        ] {
            assert_eq!(signals(&[line]), [], "{line}");
        }
    }

    /// F2a.a: an assertion that cannot fail is read as a class, beside the reading of 0.9.0
    /// (`assert!(true)`, `assert_eq!(true, true)`): a condition true by its tokens alone, two
    /// equal literals or the same tokens for the `_eq` forms, two unequal literals for the
    /// `_ne` forms. One that fails on every run (`assert_eq!(1, 2)`, `assert_ne!(x, x)`,
    /// `assert!(false)`) or can pass and fail is none.
    #[test]
    fn an_assertion_of_equal_operands_cannot_fail() {
        for lines in [
            &["+assert!(!false);"][..],
            &["+assert!((true));"],
            &["+assert!(1 == 1);"],
            &["+assert!(true || x);"],
            &["+assert!(x || true);"],
            &["+debug_assert!(!(1 > 2) && 0x10 >= 16, \"\");"],
            &["+assert!(x == x);"],
            &["+assert!(a.len <= a.len);"],
            &["+assert_eq!(x, x);"],
            &["+assert_eq!(v.0[1], v.0[1]);"],
            &["+assert_eq!(1, 1_u8);"],
            &["+assert_eq!(-1, -0b1, \"\");"],
            &["+debug_assert_eq!(Map::<u8, u8>::MAX, Map::<u8, u8>::MAX);"],
            &["+assert_eq!(x.0, x.0,);"],
            &["+assert_ne!(1, 2);"],
            &["+debug_assert_ne!(true, false, \"\");"],
            &["+assert_ne!{0, 1}"],
            &["+assert_eq!(", "+    value,", "+    value", "+);"],
        ] {
            assert_eq!(signals(lines), [S::TrivialAssert], "{lines:?}");
        }
        for line in [
            "+assert_eq!(1, 2);",
            "+assert_eq!(x, y);",
            "+assert_eq!(x, x.0);",
            "+assert_ne!(x, x);",
            "+assert_ne!(1, 0x1);",
            "+assert_ne!(1, x);",
            "+assert!(false);",
            "+assert!(1 == 2);",
            "+assert!(x || false);",
            "+assert!(x == y);",
            "+assert!(x != x);",
            "+assert!(true && x);",
            "+assert!(!x);",
            "+assert!(1 << 2 == 4);",
            "+assert!(x = x);",
            "+assert_eq!(\"a\", \"b\");",
            "+assert_eq!('a', 'b');",
            "+assert_eq!(f64::NAN, f64::NAN);",
            "+assert_eq!(f(2), f(2));",
            "+assert_eq!(rank(&files), rank(&files));",
            "+assert!(a.len() <= a.len());",
            "+assert_eq!(vec![x], vec![x]);",
            "+assert_eq!({ x }, { x });",
            "+assert!(f64::NAN == f64::NAN);",
            "+assert_eq!(x, x",
            "+assert_eq!(x, x];",
            "+assert_eq!(x);",
            "+assert_ne(1, 2);",
        ] {
            assert_eq!(signals(&[line]), [], "{line}");
        }
        // a change of the message alone is none: the construct ends at the comma
        assert_eq!(signals(&[" assert_eq!(x, x,", "+    \"now\");"]), []);
        // operands longer than the window are not read
        let long = format!("+assert_eq!({0}, {0});", ["x"; 40].join(" + "));
        assert_eq!(signals(&[&long]), []);
    }

    /// F2a.b: a statement that drops what a call returns, `let _ = call();`, is a swallowed
    /// error where the call returns a `Result`; the reader cannot know a type, and reads the
    /// calls of the standard library that do by name.
    #[test]
    fn a_dropped_result_is_a_swallowed_error() {
        for lines in [
            &["+let _ = std::fs::remove_file(\"x\");"][..],
            &["+    let _ = tx.send(1);"],
            &["+let _ : Result<(), Box<dyn Iterator<Item = u8>>> = rx.recv();"],
            &["+let _ = handle.join();"],
            &["+let _ = std::io::copy(&mut pipe, &mut std::io::sink());"],
            &["+{ _ = child.kill(); }"],
            &["+_ = child.wait();"],
            &[
                "+let _ = File::create(p)",
                "+    .and_then(|mut f| f.write_all(b))",
                "+    .flush();",
            ],
            &["+let", "+_", "+=", "+fs::remove_dir_all(p)", "+;"],
        ] {
            assert_eq!(signals(lines), [S::ErrorSwallowed], "{lines:?}");
        }
        for line in [
            "+let _ = x;",
            "+let _ = f();",
            "+let _ = a.b(c)(d);",
            "+let _ = writeln!(out, \"x\");",
            "+let _ = write!(out, \"x\");",
            "+let _ = std::fs::remove_file(p).is_ok();",
            "+let _ = std::fs::remove_file;",
            "+let _ = remove_file!(p);",
            "+let _e = child.kill();",
            "+let _x = child.kill();",
            "+let (_, _) = child.kill();",
            "+let _ = child.kill()?;",
            "+let _ = kill(x?);",
            "+let _ = (child.kill(), b);",
            "+let _ = child.wait().await;",
            "+let _ = { child.kill() };",
            "+let _ = child.kill()",
            "+child.kill().ok();",
            "+drop(child.kill());",
            "+x = _ == child.kill();",
            "+let _ == child.kill();",
            "+if let _ = child.kill() {}",
            "+match x { _ => child.kill(), };",
        ] {
            assert_eq!(signals(&[line]), [], "{line}");
        }
    }

    /// F2a.c: `panic!` and `unreachable!` are panic paths, read as a stub is (the word, `!`, an
    /// opening bracket), with the hint of a test's braces as every panic path has it.
    #[test]
    fn panic_and_unreachable_are_panic_paths() {
        for lines in [
            &["+panic!(\"x\");"][..],
            &["+unreachable!();"],
            &["+    _ => panic!(\"{x:?}\"),"],
            &["+std::panic!{};"],
            &["+core::unreachable![]"],
            &["+$crate::panic!(\"x\")"],
            &["+panic", "+!", "+(", "+)"],
        ] {
            assert_eq!(signals(lines), [S::PanicPathAdded], "{lines:?}");
        }
        for line in [
            "+std::panic::catch_unwind(f);",
            "+#[should_panic]",
            "+panic!",
            "+panic!;",
            "+unreachable_unchecked();",
            "+my_panic!();",
            "+panic(x);",
            "+let panic = 1;",
        ] {
            assert_eq!(signals(&[line]), [], "{line}");
        }
        assert_eq!(
            hints(&[
                "+#[test]",
                "+fn t() { unreachable!(); }",
                "+fn u() { panic!(); }"
            ]),
            [(1, true), (2, false)]
        );
    }

    /// F2a.d: a test compiled out by `#[cfg(…)]` whose predicate is false in every build of the
    /// tests or names anything but the platform's is a skipped test, whichever of the two
    /// attributes comes first and whichever line the change made.
    #[test]
    fn a_test_compiled_out_by_an_empty_any_is_skipped() {
        let skipped = |lines: &[&str]| -> Vec<(EvidenceSignal, usize)> {
            new(lines).iter().map(|item| (item.0, item.2)).collect()
        };
        for (lines, line) in [
            (&["+#[cfg(any())]", "+#[test]", "+fn e() {}"][..], 0),
            (&["+#[cfg(all(any()))]", "+#[test]"], 0),
            (&["+#[cfg(not(all()))]", "+#[test]"], 0),
            (&["+#[cfg(FALSE)]", "+#[test]"], 0),
            (&["+#[cfg(not(test))]", "+#[test]"], 0),
            (&["+#[cfg(feature = \"slow\")]", "+#[test]"], 0),
            (&["+#[cfg(all(unix, any()))]", "+#[test]"], 0),
            (&["+#[cfg(any(unix, version(\"1.80\")))]", "+#[test]"], 0),
            (&[" #[test]", "+#[cfg(any())]", " fn e() {}"], 1),
            (&[" #[cfg(any())]", "+#[tokio::test]"], 1),
            (&[" #[cfg(any())]", " #[allow(x)]", "+#[test]"], 2),
            (&["+#[cfg(any())] #[should_panic] #[rstest(a)]"], 0),
            (&["+#[test]", "+#[cfg", "+(", "+any", "+(", "+)", "+)]"], 0),
        ] {
            assert_eq!(skipped(lines), [(S::TestSkipped, line)], "{lines:?}");
        }
        for lines in [
            &["+#[cfg(test)]", "+#[test]"][..],
            &["+#[cfg(any(unix))]", "+#[test]"],
            &["+#[cfg(all(target_os = \"linux\", not(miri)))]", "+#[test]"],
            &["+#[cfg(any(test, FALSE))]", "+#[test]"],
            &["+#[cfg(any())]", "+fn e() {}"],
            &["+#[cfg(any())]", "+mod tests {}"],
            &["+#[cfg(any())]", "+fn e() {}", "+#[test]"],
            &["+#![cfg(any())]", "+#[test]"],
            &["+#[cfg(any()]", "+#[test]"],
            &["+#[cfg(not(unix, windows))]", "+#[test]"],
            &[" #[cfg(any())]", " #[test]", "+fn e() {}"],
        ] {
            assert_eq!(skipped(lines), [], "{lines:?}");
        }
    }

    #[test]
    fn a_stub_is_the_macro_and_its_opening_bracket() {
        for lines in [
            &["+todo!()"][..],
            &["+todo ! ( )"],
            &["+unimplemented!(\"\")"],
            &["+todo![]"],
            &["+unimplemented!{}"],
            &["+std::todo!()"],
            &["+todo", "+!()"],
            &["+todo!", "+()"],
        ] {
            assert_eq!(signals(lines), [S::StubAdded], "{lines:?}");
        }
        for line in [
            "+my_todo!(x)",
            "+retodo ! ()",
            "+todo()",
            "+todo!",
            "+todo!;",
            "+let todo = 1;",
        ] {
            assert_eq!(signals(&[line]), [], "{line}");
        }
    }

    #[test]
    fn an_ignore_attribute_is_read_whatever_its_spacing() {
        for lines in [
            &["+#[ignore]"][..],
            &["+# [ ignore ]"],
            &["+#[ignore = \"\"]"],
            &["+#![ignore]"],
            &["+#", "+[ignore]"],
            &["+#[", "+ignore]"],
            &["+#[ignore", "+]"],
            // `#[ignore(…)]`: a lint the compiler denies by default and a crate can allow,
            // and the test is then ignored (tenth independent review, IR10-M-1)
            &["+#[ignore(not_ready)]"],
            &["+#[ignore()]"],
            &["+# [ ignore ( x ) ]"],
            &["+#![ignore(x)]"],
            &["+#[ignore", "+(x)]"],
            // a `#` after a name is a `#`: up to edition 2018 the compiler reads `x#[ignore]`
            // as these tokens too
            &["+x#[ignore]"],
            &["+x #[ignore]"],
        ] {
            assert_eq!(signals(lines), [S::TestSkipped], "{lines:?}");
        }
        for line in [
            "+#[ignored]",
            "+#[ignore_me]",
            "+[ignore]",
            "+#[ignore[x]]",
            "+#[ignore::x(y)]",
            "+#[ignore",
            "+#[doc = ignore]",
            "+#!![ignore]",
            // not whole: the side does not hold the `]` (round 18 of Vesper's review,
            // VD3R18-M-1: in a macro that takes any tokens `#[ignore(` goes on as anything)
            "+#[ignore(",
            "+#![ignore(",
            "+#[ignore(x)",
            "+#[ignore(x]",
            "+#[ignore =",
            "+#[ignore = \"\"",
        ] {
            assert_eq!(signals(&[line]), [], "{line}");
        }
        // whole within the bound, from `[` to `]`, and not past it
        let arguments = |count: usize| format!("+#[ignore({})]", "x ".repeat(count));
        assert_eq!(
            signals(&[&arguments(TURBOFISH_TOKENS - 5)]),
            [S::TestSkipped]
        );
        assert_eq!(signals(&[&arguments(TURBOFISH_TOKENS - 4)]), []);
        // an `ignore` that is another attribute's argument is no skip (the attribute is
        // read for what it is)
        assert_eq!(signals(&["+#[allow(ignore)]"]), [S::CheckSuppressed]);
    }

    /// The cases of the conditional skip's tables that apply `allow(…)` after a condition, in a
    /// whole attribute: since 0.11.0 (F2b) each is a suppression as well.
    const SUPPRESSING: [&str; 6] = [
        "all(a, b), allow(x), ignore)]",
        "ignore, allow(x))]",
        "miri, allow(ignore))]",
        "a, allow(x), cfg_attr(b, inline, ignore))]",
        "all(), cfg_attr(ignore, allow(x)))]",
        "cfg_attr(a, ignore), allow(x))]",
    ];

    /// `#[cfg_attr(condition, allow(…))]` and `expect(…)` switch lints off where the
    /// condition holds, also through a `cfg_attr` that a `cfg_attr` applies, whatever the
    /// spacing and over lines, inner or outer (with a raw identifier in the evidence tests): a suppression on the
    /// first changed line of the list, when the attribute is whole. Not: the condition,
    /// another attribute's argument, a longer name, a path, no list, a list the side leaves open.
    #[test]
    fn a_conditional_allow_is_a_suppression() {
        for line in [
            "+#[cfg_attr(all(), allow(dead_code))]",
            "+#[cfg_attr(test, expect(unused))]",
            "+#![cfg_attr(not(test), allow(clippy::all))]",
            "+# [ cfg_attr ( miri , allow ( x ) ) ]",
            "+#[cfg_attr(all(), cfg_attr(test, expect(dead_code)))]",
            "+#[cfg_attr(a, inline, allow(x, reason = \"r\"))]",
            "+#[cfg_attr(feature = \"x\", allow(x))] fn f() {}",
        ] {
            assert_eq!(signals(&[line]), [S::CheckSuppressed], "{line}");
            if line.contains('"') {
                continue;
            }
            // the same over lines: every token on a line of its own
            let spread: Vec<String> = tokens(&[line])
                .iter()
                .map(|token| {
                    let rest = &line[1 + token.column..];
                    let end = rest
                        .bytes()
                        .position(|byte| !names(byte))
                        .map_or(rest.len(), |end| end.max(1));
                    format!("+{}", &rest[..end])
                })
                .collect();
            let spread: Vec<&str> = spread.iter().map(String::as_str).collect();
            let items = new(&spread);
            assert_eq!(items.len(), 1, "{line}, a token a line");
            assert_eq!(
                (items[0].0, items[0].1),
                (S::CheckSuppressed, Basis::Hunk),
                "{line}, a token a line"
            );
        }
        for line in [
            "+#[cfg_attr(feature = \"serde\", serde(allow(x)))]",
            "+#[cfg_attr(allow(x), inline)]",
            "+#[cfg_attr(test, allow)]",
            "+#[cfg_attr(test, allowed(x))]",
            "+#[cfg_attr(test, allow::x(y))]",
            "+#[cfg_attr(test, allow(x)",
            "+#[cfg_attr(test, allow(x))",
            "+#[cfg_attr(test, inline) allow(x)]",
            "+#[serde(allow(x))]",
        ] {
            assert_eq!(signals(&[line]), [], "{line}");
        }
    }

    /// The conditional skip, argument by argument. Independent review IR-L-2 (an `ignore`
    /// that is another attribute's), second independent review IR2-M-2 (a `cfg_attr` that a
    /// `cfg_attr` applies). These are the cases the line rule's `cfg_attr_skips` was held to,
    /// each now a whole attribute.
    #[test]
    fn a_conditional_skip_applies_ignore_as_an_attribute_of_its_own() {
        for (arguments, skips) in [
            ("miri, ignore)]", true),
            ("miri, ignore = \"\")]", true),
            ("all(a, b), allow(x), ignore)]", true),
            // the side ends on the `ignore`
            ("miri, ignore", false),
            ("ignore, allow(x))]", false),
            ("miri, allow(ignore))]", false),
            ("feature = \"\", serde(skip, ignore, default))]", false),
            ("miri, ignored)]", false),
            ("miri)]", false),
            ("miri, ignore x)]", false),
            ("miri, x ignore)]", false),
            // `ignore(…)` as an attribute of its own: the lint `ill_formed_attribute_input`,
            // which a crate can allow (IR10-M-1)
            ("miri, ignore(x))]", true),
            ("all(), ignore(flaky))]", true),
            ("a, cfg_attr(b, ignore(y)))]", true),
            // not whole: the list, or the `ignore(` in it, is still open where the side ends,
            // or no `]` follows its `)` (VD3R18-M-1)
            ("a, cfg_attr(b, ignore (", false),
            ("all(), ignore(", false),
            ("all(), ignore(flaky)", false),
            ("all(), ignore(flaky))", false),
            ("miri, ignore)", false),
            ("miri, ignore,", false),
            ("miri, ignore = \"\"", false),
            ("miri, ignore)) ]", false),
            ("miri, ignore[x])]", false),
            ("miri, serde(ignore(x)))]", false),
            ("miri, ignore::x(y))]", false),
            // IR2-M-2
            ("all(), cfg_attr(all(), ignore))]", true),
            ("all(), cfg_attr ( all() , ignore = \"\" ) )]", true),
            ("a, allow(x), cfg_attr(b, inline, ignore))]", true),
            ("a, cfg_attr(b, cfg_attr(c, ignore)))]", true),
            ("a, cfg_attr(b, cfg_attr(c, inline)), ignore)]", true),
            ("a, cfg_attr(b, ignore)", false),
            ("a, cfg_attr(b, ignore))]", true),
            ("a, cfg_attr(b, ignore ,", false),
            ("a, cfg_attr(b, ignore =", false),
            // an `ignore` that the side ends on is no skip: it can go on as `ignore::case`
            ("a, cfg_attr(b, ignore", false),
            ("a, cfg_attr(b, ignore::case))]", false),
            // and only that: the nested condition, the place of the condition, another
            // attribute's list, a longer name, and what follows a nested list that has closed
            ("all(), cfg_attr(ignore, allow(x)))]", false),
            ("cfg_attr(a, ignore), allow(x))]", false),
            ("a, serde(cfg_attr(b, ignore)))]", false),
            ("a, my_cfg_attr(b, ignore))]", false),
            ("a, cfg_attr(b, inline) ignore)]", false),
            (
                "a, cfg_attr(b, cfg_attr(c, inline)))] fn ignore() {}",
                false,
            ),
            ("a, cfg_attr(b, inline))] fn f(x: u8, ignore: u8) {}", false),
        ] {
            // since 0.11.0 (F2b) an `allow(…)` after a condition of a whole attribute is a
            // suppression as well
            let suppresses = SUPPRESSING.contains(&arguments);
            let expected: &[EvidenceSignal] = match (suppresses, skips) {
                (true, true) => &[S::CheckSuppressed, S::TestSkipped],
                (true, false) => &[S::CheckSuppressed],
                (false, true) => &[S::TestSkipped],
                (false, false) => &[],
            };
            for opener in ["#[cfg_attr(", "# [ cfg_attr (", "#![cfg_attr("] {
                let line = format!("+{opener}{arguments}");
                assert_eq!(signals(&[&line]), expected, "{line}");
                // the same over lines: every token on a line of its own
                let spread: Vec<String> = tokens(&[&line])
                    .iter()
                    .map(|token| {
                        let rest = &line[1 + token.column..];
                        let end = rest
                            .bytes()
                            .position(|byte| !names(byte))
                            .map_or(rest.len(), |end| end.max(1));
                        format!("+{}", &rest[..end])
                    })
                    .collect();
                let spread: Vec<&str> = spread.iter().map(String::as_str).collect();
                assert_eq!(signals(&spread), expected, "{line}, a token a line");
            }
        }
        // not an attribute's opener
        for line in [
            "+cfg_attr(miri, ignore)",
            "+[cfg_attr(miri, ignore)]",
            "+#[my::cfg_attr(miri, ignore)]",
            "+#[cfg_attr[miri, ignore]]",
            "+#!![cfg_attr(miri, ignore)]",
        ] {
            assert_eq!(signals(&[line]), [], "{line}");
        }
    }

    /// Independent review, IR-M-3, the second, IR2-M-1, and the third, IR3-M-1: the construct
    /// is the list from its `#` to its `ignore`, so a token changed anywhere in the condition
    /// is a change to the skip, on that token's line, and a token changed after the `ignore`
    /// is not.
    #[test]
    fn a_conditional_skip_is_evidence_where_its_first_changed_token_is() {
        let skip = |line: usize, basis: Basis| vec![(S::TestSkipped, basis, line)];
        assert_eq!(
            new(&["+#[cfg_attr(miri, ignore)]"]),
            skip(0, Basis::Lexical)
        );
        assert_eq!(
            new(&[" #[cfg_attr(", "+    all(),", "     ignore", " )]"]),
            skip(1, Basis::Hunk)
        );
        assert_eq!(
            new(&[
                " #[cfg_attr(",
                "     all(",
                "+        unix,",
                "     ),",
                "+    ignore",
                " )]"
            ]),
            skip(2, Basis::Hunk)
        );
        assert_eq!(
            new(&[" #[cfg_attr(", "     miri,", "+    ignore = \"\"", " )]"]),
            skip(2, Basis::Hunk)
        );
        // nothing of it changed: a line added after the `ignore`, or beside the attribute
        assert_eq!(
            new(&[
                " #[cfg_attr(",
                "     miri,",
                "     ignore",
                "+    , inline",
                " )]"
            ]),
            []
        );
        assert_eq!(
            new(&[" #[cfg_attr(miri, ignore)]", "+#[inline]", " fn slow() {}"]),
            []
        );
        // a list inside a list (in what another attribute holds): each is followed, and the
        // outer one goes on where the inner one closed
        assert_eq!(
            new(&["+#[cfg_attr(a, b(#[cfg_attr(c, d)]), ignore)]"]),
            skip(0, Basis::Lexical)
        );
        assert_eq!(
            new(&["+#[cfg_attr(a, b(#[cfg_attr(c, ignore)]))]"]),
            skip(0, Basis::Lexical)
        );
        assert_eq!(new(&["+#[cfg_attr(a, b(#[cfg_attr(c, d)], ignore))]"]), []);
        // a changed token inside the inner list is a changed token of the outer one
        assert_eq!(
            new(&[
                " #[cfg_attr(a, b(#[cfg_attr(",
                "+    c,",
                "     d)]),",
                "     ignore)]"
            ]),
            skip(1, Basis::Hunk)
        );
    }

    /// Found 2026-11-14: each `cfg_attr` list of a line was read from its own start, so a line
    /// that is nothing but `#[cfg_attr(` cost the square of its length (880 KB took 32 s to
    /// rank). The lists are followed as the tokens pass, [`CFG_ATTR_LISTS`] of them at most,
    /// and one opened past that is taken to hold a skip.
    #[test]
    fn at_most_a_few_cfg_attr_lists_are_followed_at_once() {
        let nest = |depth: usize| format!("+{}", "#[cfg_attr(".repeat(depth));
        assert_eq!(signals(&[&nest(CFG_ATTR_LISTS)]), []);
        assert_eq!(signals(&[&nest(CFG_ATTR_LISTS + 1)]), [S::TestSkipped]);
        assert_eq!(signals(&[&nest(200_000)]), [S::TestSkipped]);
        // lists side by side are each closed before the next opens: none is past the limit
        let beside = "#[cfg_attr(a, inline)] ".repeat(10_000);
        assert_eq!(signals(&[&format!("+{beside}")]), []);
        assert_eq!(
            signals(&[&format!("+{beside}#[cfg_attr(a, ignore)]")]),
            [S::TestSkipped]
        );
        // the lists a `cfg_attr` applies are one list to the reader, however deep
        let deep = format!(
            "+#[cfg_attr(a, {}ignore{}]",
            "cfg_attr(a, ".repeat(200_000),
            ")".repeat(200_001)
        );
        assert_eq!(signals(&[&deep]), [S::TestSkipped]);
        // and one that the side leaves open, on its `ignore` or after it, is none
        let deep = format!("+#[cfg_attr(a, {}ignore", "cfg_attr(a, ".repeat(200_000));
        assert_eq!(signals(&[&deep]), []);
        let deep = format!("+#[cfg_attr(a, {}ignore)", "cfg_attr(a, ".repeat(200_000));
        assert_eq!(signals(&[&deep]), []);
        let deep = format!(
            "+#[cfg_attr(a, {}inline{}] fn f(x: u8, ignore: u8) {{}}",
            "cfg_attr(a, ".repeat(200_000),
            ")".repeat(200_001)
        );
        assert_eq!(signals(&[&deep]), []);
    }

    /// Review V3 round 9, VD3R9-M-1, and the third independent review, IR3-L-4: a removed
    /// test attribute, whatever its spacing and its lines, bare or with arguments.
    #[test]
    fn a_removed_test_attribute_is_read_on_the_old_side() {
        for lines in [
            &["-#[test]"][..],
            &["-# [ test ]"],
            &["-#[tokio::test]"],
            &["-#[ tokio :: test ]"],
            &["-#[rstest]"],
            // IR3-L-4: before, only the bare three
            &["-#[tokio::test(flavor = \"\")]"],
            &["-#[test(x)]"],
            &["-#[rstest(a, b)]"],
            &["-#[", "-  test", "-]"],
            &["-x#[test]"],
        ] {
            assert_eq!(
                old(lines).iter().map(|item| item.0).collect::<Vec<_>>(),
                [S::TestRemoved],
                "{lines:?}"
            );
            // on the new side it is no evidence of anything
            let added: Vec<String> = lines
                .iter()
                .map(|line| format!("+{}", &line[1..]))
                .collect();
            let added: Vec<&str> = added.iter().map(String::as_str).collect();
            assert_eq!(new(&added), [], "{lines:?}");
        }
        for line in [
            "-#[testing]",
            "-#[test_case(1)]",
            "-#[cfg(test)]",
            "-#![test]",
            "-#[tokio: :test]",
            "-#[tokio::main]",
            "-#[other::test]",
            "-#[test = 1]",
            "-[test]",
        ] {
            assert_eq!(old(&[line]), [], "{line}");
        }
        // on the first removed line that holds one of its tokens, and only when one does
        assert_eq!(
            old(&[" #[", "-  test", " ]"]),
            [(S::TestRemoved, Basis::Hunk, 1)]
        );
        assert_eq!(old(&[" #[test]", "-fn t() {}"]), []);
        assert_eq!(old(&["-#[test]", "-#[test]"]).len(), 2);
    }

    /// The hint: a panic path inside the braces of an item whose attribute spells a test says
    /// so. Review V3 round 7, VD3R7-L-1, and the independent review, IR-H-1: a function that
    /// only shares a line with a test is not one, and a line with a call of each has no hint.
    #[test]
    fn a_panic_path_inside_an_item_that_spells_a_test_carries_the_hint() {
        for attribute in [
            "#[test]",
            "#[tokio::test]",
            "#[tokio::test(flavor = \"\")]",
            "#[rstest]",
            "#[my_framework::test]",
            "#[cfg(test)]",
            "#[ cfg( test ) ]",
            "#[::core::prelude::v1::test]",
            "#[rstest::rstest]",
            "#[tokio::testing::test]",
        ] {
            let line = format!("+{attribute}");
            assert_eq!(
                hints(&["+fn real() {}", &line, "+fn a() { b().unwrap(); }"]),
                [(2, true)],
                "{attribute}"
            );
        }
        for attribute in [
            "#[inline]",
            "#[cfg(not(test))]",
            "#[cfg_attr(test, allow(x))]",
            "#[testing]",
            "#[doc = \"\"]",
            "#[doc = test]",
            "#[cfg_attr(any(), test)]",
            "#[allow(test)]",
            "#[doc(alias = \"\", test)]",
            "#[my::tests]",
            "#[cfg(any(test, feature = \"\"))]",
            "#[cfg_attr(any(), cfg(test))]",
            "#[cfg(test = \"\")]",
            "#[cfg(test,)]",
            "#[cfg((test))]",
            "#[cfg(test) x]",
            "#[a::cfg(test)]",
            "#[::cfg(test)]",
            "#[test::x]",
            "#![test]",
        ] {
            let line = format!("+{attribute}");
            assert_eq!(
                hints(&[&line, "+fn a() { b().unwrap(); }"]),
                [(1, false)],
                "{attribute}"
            );
        }
    }

    /// The hint follows the braces: review V3 round 7, VD3R7-L-1, and the independent review,
    /// IR-H-1 (a function that only shares a line with a test is not one, and a line with a
    /// call of each has no hint).
    #[test]
    fn the_hint_is_for_what_stands_inside_the_items_braces() {
        type Hints<'a> = &'a [(usize, bool)];
        let cases: &[(&[&str], Hints)] = &[
            // other attributes may stand between, and the item's own brackets are passed
            (
                &[
                    "+#[cfg(test)]",
                    "+#[inline]",
                    "+impl Thing {",
                    "+    fn f(a: [u8; 2]) -> u8 { b().unwrap() }",
                    "+}",
                ],
                &[(3, true)],
            ),
            // nested: a function in a test module, and the function after the module
            (
                &[
                    "+#[cfg(test)]",
                    "+mod t {",
                    "+    fn h() { b().unwrap(); }",
                    "+}",
                    "+fn real() { c().unwrap(); }",
                ],
                &[(2, true), (4, false)],
            ),
            // an unchanged attribute counts: the hunk shows it
            (
                &[" #[test]", " fn a() {", "+    b().unwrap();", " }"],
                &[(2, true)],
            ),
            // a module that is only named `tests` spells nothing
            (
                &["+mod tests {", "+    fn p() { b().unwrap(); }", "+}"],
                &[(1, false)],
            ),
            // an item with no body takes the attribute with it
            (
                &["+#[cfg(test)]", "+mod t;", "+fn real() { c().unwrap(); }"],
                &[(2, false)],
            ),
            (
                &[
                    "+#[cfg(test)]",
                    "+use a::b;",
                    "+fn real() { c().unwrap(); }",
                ],
                &[(2, false)],
            ),
            // VD3R7-L-1 and IR-H-1
            (
                &["+#[test] fn check() {} fn production() { b().unwrap(); }"],
                &[(0, false)],
            ),
            (
                &["+#[test] fn check() { a().unwrap(); } fn production() { b().unwrap(); }"],
                &[(0, false)],
            ),
            (&["+#[test] fn check() { a().unwrap(); }"], &[(0, true)]),
            (
                &[
                    "+#[cfg(test)]",
                    "+fn t() { a().unwrap(); } pub fn p() -> u32 { Result::unwrap(b()) }",
                ],
                &[(1, false)],
            ),
            // an attribute on a parameter is not an item's
            (
                &["+fn real(#[cfg(test)] x: u8) { c().unwrap(); }"],
                &[(0, false)],
            ),
            // inside another macro's body the tokens are the same tokens
            (
                &[
                    "+outer! {",
                    "+    #[test]",
                    "+    fn check() {",
                    "+        f().unwrap();",
                    "+    }",
                    "+}",
                ],
                &[(3, true)],
            ),
        ];
        for (lines, expected) in cases {
            assert_eq!(hints(lines), *expected, "{lines:?}");
        }
        // only a panic path carries it
        let other = read_new(lines_of(&side(&["+#[test]", "+fn a() { todo!(); }"])));
        assert_eq!(other.len(), 1);
        assert!(!other[0].test_likely);
    }

    /// The items come sorted by line and then by signal, one for each pair, also when a list
    /// that ends late is noted for an early line.
    #[test]
    fn items_are_sorted_and_single() {
        let found = new(&[
            " #[cfg_attr(",
            "+    a,",
            "+    b(todo!(), x.unwrap(), y.unwrap(), assert!(true)),",
            "+    ignore)]",
            "+x.unwrap(); todo!();",
        ]);
        assert_eq!(
            found,
            [
                (S::TestSkipped, Basis::Hunk, 1),
                (S::TrivialAssert, Basis::Lexical, 2),
                (S::StubAdded, Basis::Lexical, 2),
                (S::PanicPathAdded, Basis::Lexical, 2),
                (S::StubAdded, Basis::Lexical, 4),
                (S::PanicPathAdded, Basis::Lexical, 4),
            ]
        );
        // the higher basis of two constructs on one line stands
        assert_eq!(
            new(&["+x.unwrap(); y.", "+    unwrap(); z.unwrap();"]),
            [
                (S::PanicPathAdded, Basis::Hunk, 0),
                (S::PanicPathAdded, Basis::Lexical, 1)
            ]
        );
    }

    /// `read_hunk` gives each side its own lines, and reads no side the change left alone.
    #[test]
    fn each_side_of_a_hunk_is_read_from_its_own_lines() {
        let hunk_of = |body: &str| {
            let old = body.lines().filter(|line| !line.starts_with('+')).count();
            let new = body.lines().filter(|line| !line.starts_with('-')).count();
            let patch = format!("--- a/x.rs\n+++ b/x.rs\n@@ -1,{old} +1,{new} @@\n{body}");
            deep_diff_forge_patch::parse(&patch).unwrap().remove(0)
        };
        let read = |body: &str| {
            let file = hunk_of(body);
            let hunk = &file.patch_twin.hunks[0];
            let views: Vec<(Option<String>, Option<String>)> = hunk
                .lines
                .iter()
                .map(|line| {
                    let text = Some(line.text.clone());
                    match line.kind {
                        PatchLineKind::Context => (text.clone(), text),
                        PatchLineKind::Added => (None, text),
                        PatchLineKind::Removed => (text, None),
                    }
                })
                .collect();
            read_hunk(&hunk.lines.iter().collect::<Vec<_>>(), &views, true, true)
        };
        // the removed line between the dot and the name is not on the new side
        let found = read(" x.\n-#[test]\n+unwrap()\n");
        assert_eq!(
            found.new,
            [Item {
                line: 2,
                signal: S::PanicPathAdded,
                basis: Basis::Hunk,
                test_likely: false,
                forms: BY_CALL
            }]
        );
        assert_eq!(
            found.old,
            [Item {
                line: 1,
                signal: S::TestRemoved,
                basis: Basis::Lexical,
                test_likely: false,
                forms: 0
            }]
        );
        // a side with no line of the change is not read: nothing is evidence on it anyway
        assert_eq!(read(" #[\n+test]\n").old, []);
        assert_eq!(read(" x.\n-unwrap()\n").new, []);
    }

    /// The tokens of `line` (written without a sign), each as its text.
    fn token_texts(line: &str) -> Vec<String> {
        let signed = format!("+{line}");
        tokens(&[&signed])
            .iter()
            .map(|token| {
                let rest = &signed[1 + token.column..];
                let end = rest
                    .bytes()
                    .position(|byte| !names(byte))
                    .map_or(rest.len(), |end| end.max(1));
                rest[..end].to_owned()
            })
            .collect()
    }

    /// The paths release 0.8.3 keyed `process_spawn` and `network_fetch` on, and its
    /// `#[allow(`, are the same tokens whatever stands between them (seventh independent
    /// review, IR7-M-3: `sp1`, `sp3`, `sp4`, `sp7`, each a created file that compiles and
    /// that release 0.8.3 and round 14 said nothing about).
    #[test]
    fn the_paths_of_the_released_signals_are_read_as_tokens() {
        for (lines, signal, basis) in [
            // `sp1`, `sp4`: white space inside the path
            (
                &["+pub fn f() { let _ = std::process :: Command :: new(\"sh\").spawn(); }"][..],
                S::ProcessSpawn,
                Basis::Lexical,
            ),
            (
                &["+pub fn f() { let _ = std::net::TcpStream :: connect(\"1.2.3.4:80\"); }"],
                S::NetworkFetch,
                Basis::Lexical,
            ),
            // `sp3`: a line end after each `::`
            (
                &[
                    "+pub fn f() { let _ = std::process::",
                    "+Command::",
                    "+new(\"sh\").spawn(); }",
                ],
                S::ProcessSpawn,
                Basis::Hunk,
            ),
            // `sp7`: the name on one line and `::new` on the next (the import is a glob;
            // what is read is `Command :: new`, as that release's rule keys on it)
            (
                &["+pub fn f() { let _ = Command", "+::new(\"sh\").spawn(); }"],
                S::ProcessSpawn,
                Basis::Hunk,
            ),
            (
                &["+fn f() -> process  ::  Command {"],
                S::ProcessSpawn,
                Basis::Lexical,
            ),
            (
                &["+let r = reqwest", "+    ::blocking::get(u);"],
                S::NetworkFetch,
                Basis::Hunk,
            ),
            (
                &["+let r = ureq :: get(u);"],
                S::NetworkFetch,
                Basis::Lexical,
            ),
            (
                &["+let s = UdpSocket", "+    :: bind(a);"],
                S::NetworkFetch,
                Basis::Hunk,
            ),
            (
                &["+#", "+[allow(dead_code)]"],
                S::CheckSuppressed,
                Basis::Hunk,
            ),
            (
                &["+#! [ allow (unused) ]"],
                S::CheckSuppressed,
                Basis::Lexical,
            ),
        ] {
            assert_eq!(new(lines), [(signal, basis, 0)], "{lines:?}");
        }
        // the first changed line of the path, when its first name is on an unchanged one
        assert_eq!(
            new(&[" let c = Command", "+    ::new(\"sh\");"]),
            [(S::ProcessSpawn, Basis::Hunk, 1)]
        );
        assert_eq!(
            new(&[" let c = Command", "     ::new(\"sh\");", "+f();"]),
            []
        );
        // other names, and `: :`, which is not `::`
        for line in [
            "+MyCommand :: new(x);",
            "+Command :: new_with(x);",
            "+subprocess :: Command;",
            "+command :: new(x);",
            "+Command : : new(x);",
            "+TcpStream :: bind(x);",
            "+reqwest_like :: get(u);",
            "+# [ allowed (x) ]",
        ] {
            assert_eq!(new(&[line]), [], "{line}");
        }
    }

    /// The same paths with an empty list of generic arguments, and with the name alone in
    /// angle brackets (eighth independent review, IR8-L-1: the review's `q.rs`, which
    /// compiles and runs, and of which no version reported a line).
    #[test]
    fn the_released_paths_are_read_in_two_more_spellings() {
        for (line, signal) in [
            (
                "+    let _ = <Command>::new(\"true\").status();",
                S::ProcessSpawn,
            ),
            (
                "+    let _ = Command::<>::new(\"true\").status();",
                S::ProcessSpawn,
            ),
            (
                "+    let _ = <TcpStream>::connect(\"127.0.0.1:9\");",
                S::NetworkFetch,
            ),
            (
                "+    let _ = TcpStream::<>::connect(\"127.0.0.1:9\");",
                S::NetworkFetch,
            ),
            ("+    let _ = < UdpSocket > :: bind(a);", S::NetworkFetch),
            ("+    let _ = UdpSocket :: < > :: bind(a);", S::NetworkFetch),
            ("+    spawn(<Command>::new);", S::ProcessSpawn),
        ] {
            assert_eq!(new(&[line]), [(signal, Basis::Lexical, 0)], "{line}");
        }
        assert_eq!(
            new(&["+    let _ = <Command", "+    >::new(\"true\");"]),
            [(S::ProcessSpawn, Basis::Hunk, 0)]
        );
        // the name as the last name of a path in the brackets (round 16 of Vesper's
        // review, VD3R16-L-1: its `q.rs`; ninth independent review, IR9-L-2: `q.rs`, each
        // compiled and run by the reviewer)
        for (line, signal) in [
            (
                "+    let _ = <std::net::TcpStream>::connect(\"127.0.0.1:9\");",
                S::NetworkFetch,
            ),
            (
                "+    let _ = <std::net::UdpSocket>::bind(\"127.0.0.1:0\");",
                S::NetworkFetch,
            ),
            ("+    let _ = <self::UdpSocket>::bind(a);", S::NetworkFetch),
            ("+    let _ = <p::Command>::new(\"true\");", S::ProcessSpawn),
            (
                "+    let _ = < :: std :: net :: TcpStream > :: connect(a);",
                S::NetworkFetch,
            ),
            (
                "+    let _ = <a::b::c::d::e::f::g::h::Command>::new(x);",
                S::ProcessSpawn,
            ),
        ] {
            assert_eq!(new(&[line]), [(signal, Basis::Lexical, 0)], "{line}");
        }
        // `process::Command` is the release's key by itself, in brackets as anywhere
        assert_eq!(
            new(&["+    let _ = <std::process::Command>::new(\"true\");"]),
            [(S::ProcessSpawn, Basis::Lexical, 0)]
        );
        // the spellings that are declared and not read, and paths that are other things
        for line in [
            "+    let _ = <(Command)>::new(\"true\");",
            "+    let _ = <Command<>>::new(\"true\");",
            "+    let _ = <(TcpStream)>::connect(a);",
            "+    let _ = <<Command as T>::Me>::new(\"true\");",
            "+    let v = Vec::<std::net::TcpStream>::new();",
            "+    let m: HashMap<String, net::TcpStream> = HashMap::new();",
            "+    let b = a <net::UdpSocket>::bind(x);",
            "+    let d = <net::TcpStream as Default>::connect();",
            "+    let e = <a::b::c::d::e::f::g::h::i::Command>::new(x);",
            "+    let f = <net::TcpStreamer>::connect(x);",
        ] {
            assert_eq!(new(&[line]), [], "{line}");
        }
        // and what is another thing: the name as a generic argument of another path or
        // type, generic arguments that are not empty, a comparison
        for line in [
            "+    let v = Vec::<Command>::new();",
            "+    let v: Vec<Command> = Vec::new();",
            "+    let m = HashMap::<String, TcpStream>::new();",
            "+    let c = Command::<u8>::new(x);",
            "+    let b = a <Command> ::new(x);",
            "+    let d = <Command as Default>::default();",
            "+    let e = <MyCommand>::new(x);",
            "+    let f = <Command>::new_with(x);",
            "+    let g = <TcpStream>::bind(x);",
        ] {
            assert_eq!(new(&[line]), [], "{line}");
        }
    }

    /// The bracketed spellings after a keyword after which an expression begins, where the
    /// `<` cannot be a comparison (tenth independent review, IR10-L-2: its `kw.patch`,
    /// compiled and run, spawning `true`); after any other name it is not read.
    #[test]
    fn a_bracketed_path_after_a_keyword_that_begins_an_expression_is_read() {
        for (line, signal) in [
            (
                "+    return <Command>::new(\"true\").status();",
                S::ProcessSpawn,
            ),
            (
                "+    match <Command>::new(\"true\").status() {",
                S::ProcessSpawn,
            ),
            (
                "+    for _ in <Command>::new(\"true\").status().ok() {}",
                S::ProcessSpawn,
            ),
            (
                "+    'l: loop { break 'l <Command>::new(\"true\").status(); }",
                S::ProcessSpawn,
            ),
            (
                "+    loop { break <TcpStream>::connect(a); }",
                S::NetworkFetch,
            ),
            ("+    if <UdpSocket>::bind(a).is_ok() {", S::NetworkFetch),
            (
                "+    while <Command>::new(x).status().is_err() {}",
                S::ProcessSpawn,
            ),
            ("+    yield <Command>::new(x);", S::ProcessSpawn),
            (
                "+    return <std::net::TcpStream>::connect(a);",
                S::NetworkFetch,
            ),
            // eleventh independent review, IR11-L-2 (`mutref-u0.patch`, compiled and run: it
            // spawned `touch`), and round 18 of Vesper's review, VD3R18-L-1 (`become.rs`,
            // `yeet.rs`, compiled with their nightly features)
            (
                "+    let ok = run(&mut <Command>::new(\"touch\").arg(p));",
                S::ProcessSpawn,
            ),
            (
                "+    let c = &mut <Command>::new(\"touch\");",
                S::ProcessSpawn,
            ),
            ("+    become <Command>::new(program)", S::ProcessSpawn),
            ("+    do yeet <Command>::new(\"true\");", S::ProcessSpawn),
            ("+    do yeet <TcpStream>::connect(a);", S::NetworkFetch),
        ] {
            assert_eq!(new(&[line]), [(signal, Basis::Lexical, 0)], "{line}");
        }
        // the keyword and its label on the line before: the path is the name and what
        // follows it, on the second line
        assert_eq!(
            new(&["+    break 'outer", "+        <Command>::new(x);"]),
            [(S::ProcessSpawn, Basis::Lexical, 1)]
        );
        // the keyword on the line before
        assert_eq!(
            new(&["+    do yeet", "+        <Command>::new(x);"]),
            [(S::ProcessSpawn, Basis::Lexical, 1)]
        );
        // a label alone, a postfix keyword, a longer name, `yeet` without `do`, a name
        // before `yeet`: a comparison or a generic list
        for line in [
            "+    let b = 'l <Command> ::new(x);",
            "+    let b = x.await <Command> ::new(x);",
            "+    let b = returned <Command> ::new(x);",
            "+    let b = mutable <Command> ::new(x);",
            "+    let b = becomes <Command> ::new(x);",
            "+    let b = yeet <Command> ::new(x);",
            "+    let b = undo yeet <Command> ::new(x);",
        ] {
            assert_eq!(new(&[line]), [], "{line}");
        }
    }

    /// Every form that some round read at the end of a hunk side, and that no round reads
    /// there now: a hunk side is read as text that ends there; nothing is inferred from how
    /// it ends. From round 13 to round 16 a `cfg_attr` list left open on its `ignore`; in
    /// round 15 a bare name of a panic path, a stub, an assertion or a released path, and
    /// what follows them up to the next doubt; in rounds 15 and 16 an assertion up to its
    /// `true`, an attribute up to its name, a released path up to its `::`, and on the old
    /// side a test's attribute up to its name; from round 15 to round 17 a method's name with
    /// `::` after it (round 17 of Vesper's review, VD3R17-M-1: in a macro that takes any
    /// tokens `value.unwrap::` goes on as `NOT_A_CALL`; tenth independent review, IR10-L-3:
    /// after a range `unwrap` is a free function); and from round 13 to round 18 a skip's
    /// attribute that the side does not hold whole, to its `]` (round 18 of Vesper's review,
    /// VD3R18-M-1: in such a macro `#[ignore(` and `cfg_attr(all(), ignore(` go on as
    /// anything; the parenthesised forms were read in round 18 only).
    const FORMS_ONCE_READ_AT_AN_EDGE: [&str; 63] = [
        // rounds 13 to 16
        "#[cfg_attr(miri, ignore",
        "#![cfg_attr(miri, ignore",
        "#[cfg_attr(a, cfg_attr(b, ignore",
        // round 15
        "let v = opt.unwrap",
        "let v = opt.expect",
        "let v = X::unwrap",
        "let v = X::expect",
        "let v = X::unwrap::",
        "let v = X::expect::<",
        "todo",
        "unimplemented",
        "assert",
        "assert!",
        "debug_assert",
        "assert_eq",
        "debug_assert_eq!",
        "assert_eq!(true",
        "assert_eq!(true,",
        "let c = Command",
        "let s = TcpStream",
        "let u = UdpSocket",
        "use reqwest",
        "use ureq",
        // rounds 15 and 16
        "todo!",
        "unimplemented!",
        "assert!(true",
        "debug_assert![true",
        "assert_eq!(true, true",
        "debug_assert_eq!{true, true",
        "#[ignore",
        "#![ignore",
        "#[allow",
        "#![allow",
        "let c = Command::",
        "let p = process::",
        "let s = TcpStream::",
        "let u = UdpSocket::",
        "#[test",
        "#![test",
        "#[rstest",
        "#[tokio",
        "#[tokio::",
        "#[tokio::test",
        // rounds 15 to 17
        "let v = opt.unwrap::",
        "let v = opt.unwrap::<",
        "let v = opt.expect::",
        "let v = opt.expect::<",
        "let v = make()?.expect :: <",
        "x.y().unwrap ::",
        // the reviewers' shapes of round 17
        "        value.unwrap::",
        "    let r = 0..unwrap::",
        "        value.expect::<",
        // rounds 13 to 18: a skip's attribute up to the token after its `ignore`
        "#[ignore =",
        "#![ignore =",
        "#[cfg_attr(miri, ignore,",
        "#[cfg_attr(miri, ignore =",
        "#[cfg_attr(miri, ignore)",
        "#[cfg_attr(a, cfg_attr(b, ignore)",
        // round 18: with its opening parenthesis (Vesper's `direct.rs` and `cfg.rs`)
        "#[ignore(",
        "#![ignore(",
        "        #[ignore(",
        "#[cfg_attr(miri, ignore(",
        "        #[cfg_attr(all(), ignore(",
    ];

    /// No form is read at the end of a side (round 18, the coordinator's ruling on round 17
    /// of Vesper's review, VD3R17-M-1, and the tenth independent review, IR10-L-3). Every form
    /// of [`FORMS_ONCE_READ_AT_AN_EDGE`], at the end of a new side and of an old side, on one
    /// changed line after context or after a changed line, with a space between every two
    /// tokens, and over lines with a token on each, inside a macro's braces as outside: no
    /// item.
    #[test]
    fn no_form_is_read_at_the_end_of_a_side() {
        let mut cases = 0;
        for form in FORMS_ONCE_READ_AT_AN_EDGE {
            let tokens = token_texts(form);
            let spaced = tokens.join(" ");
            for (sign, read) in [('+', new as fn(&[&str]) -> Vec<Found3>), ('-', old)] {
                for shown in [form, spaced.as_str()] {
                    let line = format!("{sign}{shown}");
                    for before in [" fn f() {", " discard! {", &format!("{sign}fn f() {{")] {
                        assert_eq!(read(&[before, &line]), [], "{sign} {before} | {shown}");
                    }
                    assert_eq!(read(&[&line]), [], "{sign} {shown}");
                }
                let spread: Vec<String> = tokens
                    .iter()
                    .map(|token| format!("{sign}{token}"))
                    .collect();
                let spread: Vec<&str> = spread.iter().map(String::as_str).collect();
                assert_eq!(read(&spread), [], "{sign} {form}, a token a line");
                cases += 1;
            }
        }
        assert_eq!(cases, 2 * FORMS_ONCE_READ_AT_AN_EDGE.len());
        // the reviewers' patches as `git diff -U0` wrote them: Vesper's `macro-token-tree.patch`
        // (`discard! { value.unwrap:: NOT_A_CALL }` compiles and calls nothing) and the tenth
        // review's `range-u0.patch` (`let r = 1..unwrap::` and `<u8>();`, a free function)
        assert_eq!(new(&["+        value.unwrap::"]), []);
        assert_eq!(new(&["+    let r = 0..unwrap::"]), []);
        // Vesper's two patches of round 18 (VD3R18-M-1), as `git diff -U0` wrote them, and
        // the same text with its `)]`, which is the whole attribute and a skip
        for (shown, close) in [
            ("+        #[ignore(", "         )]"),
            ("+        #[cfg_attr(all(), ignore(", "         ))]"),
        ] {
            assert_eq!(new(&[" fn main() {", shown]), [], "{shown}");
            assert_eq!(
                new(&[" discard! {", shown, "             NOT_A_SKIP", close]),
                [(S::TestSkipped, Basis::Lexical, 1)],
                "{shown}"
            );
        }
        // and the macro's text whole: `.unwrap::` going on as a name is no call either
        assert_eq!(
            new(&[
                " discard! {",
                "+        value.unwrap::",
                "         NOT_A_CALL",
                " }"
            ]),
            []
        );
    }

    /// A method call with generic arguments is read by the rule for the whole wherever its
    /// lines are cut, when the side shows it whole, and not when the side ends before its
    /// call: the tokens shown, then a continuation, as one text, are the construct; the
    /// tokens shown alone are nothing (round 17 read them as an edge form). Generated here:
    /// four beginnings, generic arguments of several shapes and a call of several shapes,
    /// written on the next line or lines, each line with any white space.
    #[test]
    fn a_method_call_with_generic_arguments_is_read_over_any_lines_and_not_before_its_call() {
        const SHOWN: [&str; 4] = [
            "let v = opt.unwrap::",
            "let v = opt . unwrap ::",
            "let v = make()?.expect::",
            "x.y().unwrap::",
        ];
        const GENERICS: [&str; 7] = [
            "<>",
            "<u8>",
            "< 'static >",
            "<Vec<u8>>",
            "<[u8; 4]>",
            "<(u8, fn(u8) -> u8)>",
            "<Result<Vec<u8>, Box<dyn std::error::Error>>>",
        ];
        const CALLS: [&str; 4] = ["()", "(\"why\")", "( msg )", "(&format!(\"{x}\"))"];
        let mut cases = 0;
        for shown in SHOWN {
            for generics in GENERICS {
                for call in CALLS {
                    // the form with `<` shown is the form and the first token of its generics
                    let (open, rest) = generics.split_at(1);
                    for (first, second) in [
                        (shown.to_owned(), format!("{generics}{call};")),
                        (format!("{shown}{open}"), format!("{rest}{call};")),
                        (format!("{shown} {open}"), format!("    {rest} {call} ;")),
                    ] {
                        let shown_line = format!("+{first}");
                        // with the continuation unseen, nothing
                        assert_eq!(new(&[" fn f() {", &shown_line]), [], "{first}");
                        // and the rule for the whole fires on the two as one text, the
                        // continuation on one line or on two
                        let next_line = format!("+{second}");
                        assert_eq!(
                            signals(&[&shown_line, &next_line]),
                            [S::PanicPathAdded],
                            "{first} | {second}"
                        );
                        let (head, tail) = second.split_at(second.find('>').map_or(1, |at| at + 1));
                        let (head, tail) = (format!("+{head}"), format!("+{tail}"));
                        assert_eq!(
                            signals(&[&shown_line, &head, &tail]),
                            [S::PanicPathAdded],
                            "{first} | {second}"
                        );
                        cases += 1;
                    }
                }
            }
        }
        assert_eq!(cases, 4 * 7 * 4 * 3);
    }

    /// What a side can end on with a continuation that is NOT the construct is no item,
    /// though round 16 read each of these (each line is the form and, after `|`, an honest
    /// continuation; the form at the end of a side gives nothing, and the two lines as one
    /// text give nothing either):
    ///
    /// - round 16 of Vesper's review, VD3R16-M-1: `const X: u8 = Command::` and `VERSION;`
    ///   on the next line, which compiles, and whose `git diff -U0` ends on `Command::`;
    /// - ninth independent review, IR9-M-2: rustfmt's own layout of a long condition that
    ///   begins with `true`;
    /// - an attribute that is a path (`#[rstest::fixture]`, `#[tokio::main]`);
    /// - IR9-L-5: `todo!` as the end of a sentence in a comment the hunk does not open.
    #[test]
    fn what_a_side_ends_on_with_another_continuation_is_no_item() {
        for (shown, goes_on) in [
            ("const X: u8 = Command::", "VERSION;"),
            ("let id = std::process::", "id();"),
            ("let v = TcpStream::", "VERSION;"),
            ("let v = UdpSocket ::", "DEFAULT_PORT;"),
            ("    assert!(true", "        && long_condition(a) > limit);"),
            ("    debug_assert![true", "        == likely(true)];"),
            ("    assert_eq!(true, true", "        && x);"),
            ("#[ignore", "::case]"),
            ("#![ignore", "::all]"),
            ("#[allow", "::this]"),
            ("#![allow", "::that]"),
        ] {
            let (shown, goes_on) = (format!("+{shown}"), format!("+{goes_on}"));
            assert_eq!(new(&[" fn f() {", &shown]), [], "{shown}");
            assert_eq!(signals(&[&shown, &goes_on]), [], "{shown} {goes_on}");
        }
        // prose: the end of a sentence, and no bracket ever follows
        for shown in [
            " * The fast path is still a todo!",
            "// not yet: unimplemented !",
        ] {
            let line = format!("+{shown}");
            assert_eq!(new(&[" fn f() {", &line]), [], "{shown}");
        }
        // the old side: nothing is read at its end
        for (shown, goes_on) in [
            ("#[test", "::case]"),
            ("#[rstest", "::fixture]"),
            ("#[tokio", "::main]"),
            ("#[tokio::", "main]"),
            ("#[tokio::test", "::x]"),
        ] {
            let (shown, goes_on) = (format!("-{shown}"), format!("-{goes_on}"));
            assert_eq!(old(&[" fn f() {}", &shown]), [], "{shown}");
            assert_eq!(old(&[&shown, &goes_on]), [], "{shown} {goes_on}");
        }
        // with the token that ends the doubt, each is a construct to the rule for the
        // whole, at the end of a side as anywhere, and no form at an edge is needed
        for (shown, signal) in [
            ("    assert!(true,", S::TrivialAssert),
            ("    assert_eq!(true, true,", S::TrivialAssert),
            ("#[ignore = \"\"]", S::TestSkipped),
            ("#[ignore(x)]", S::TestSkipped),
            ("#[allow(", S::CheckSuppressed),
            ("    todo!(", S::StubAdded),
            ("    unimplemented![", S::StubAdded),
        ] {
            let line = format!("+{shown}");
            assert_eq!(
                new(&[" fn f() {", &line]),
                [(signal, Basis::Lexical, 1)],
                "{shown}"
            );
        }
        for shown in ["#[test]", "#[rstest(", "#[tokio::test("] {
            let line = format!("-{shown}");
            assert_eq!(
                old(&[" fn f() {}", &line]),
                [(S::TestRemoved, Basis::Lexical, 1)]
            );
        }
    }

    /// What honest code ends a line with is no item, and neither is what says too little.
    /// Each line here is one a `git diff -U0` can end a hunk on. Round 15 reported the first
    /// group (round 15 of Vesper's review, VD3R15-M-1: `pub fn command() -> Command` with
    /// its brace on the next line, in a file that spawns nothing, was a `process_spawn`).
    #[test]
    fn what_an_honest_line_ends_on_is_no_item() {
        for end in [
            // a type, a crate, a field or a variable of the name, and no more of the path
            "pub fn command() -> Command",
            "fn dial(addr: &str) -> io::Result<()> where S: From<TcpStream",
            "    socket: UdpSocket",
            "use reqwest",
            "extern crate ureq",
            "    let n = self.expect",
            "    let on = opts.unwrap",
            "        Self::expect",
            "        Option::<u8>::Some as fn(u8) -> Option<u8>; X::unwrap",
            "    let rest = todo",
            "    let later = unimplemented",
            // every assertion begins so; and the second `true` is not there
            "    assert",
            "    assert!",
            "    debug_assert_eq!",
            "    assert_eq!(true",
            // what real code ends lines with
            "    assert!(",
            "    assert_eq!(a,",
            "#",
            "#[",
            "#[cfg_attr(miri,",
            "    x.unwrap_or",
            "    let p = self.process",
            "    assert!(truth",
            "    assert!(true ==",
            "#[ignored",
        ] {
            let line = format!("+{end}");
            assert_eq!(new(&[" fn f() {", &line]), [], "{end}");
        }
    }

    /// The beginning of a side is not read as the end of a construct. Round 15 read these,
    /// and the first three begin lines of honest code (a call of a function named `expect`
    /// or `test`, an assignment to a variable named `ignore`); the others do not show the
    /// head that makes them the construct.
    #[test]
    fn what_a_side_begins_with_is_no_item() {
        for start in [
            "expect(&value, 3);",
            "ignore = true;",
            "unwrap();",
            "unwrap::<u8>();",
            "ignore]",
            "[ignore]",
            "![ignore]",
            "[allow(dead_code)]",
            "(a, b)",
            "!(x)",
            "allow(x);",
        ] {
            let line = format!("+{start}");
            assert_eq!(new(&[&line, " fn g() {}"]), [], "{start}");
        }
        // `::Command::new` is the path itself from its second name on, at the beginning of
        // a side as anywhere; `::Command` alone is nothing
        assert_eq!(
            new(&["+::Command::new(\"sh\")", " fn g() {}"]),
            [(S::ProcessSpawn, Basis::Lexical, 0)]
        );
        assert_eq!(new(&["+::Command", " fn g() {}"]), []);
        for start in [
            "test(&[], 1);",
            "test]",
            "[test]",
            "rstest]",
            "tokio::test]",
        ] {
            let line = format!("-{start}");
            assert_eq!(old(&[&line, " fn t() {}"]), [], "{start}");
        }
    }
}
