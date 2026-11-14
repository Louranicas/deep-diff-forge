// DDF integration: agent batteries, the integrity signals of a Rust hunk read as tokens.
// Design/status: obsidian://open?vault=deep-diff-forge.vault&file=Strategy%2FDDF%20batteries%20-%20plan%20of%20record%202026-11-13
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
//!     construct cut in two by them is read in two halves. What each half ends or begins
//!     on is read as an edge ([`waiting`], [`completed`]): the construct is reported when
//!     the cut stands after or before the word it is known by. Not when it stands
//!     elsewhere: after the opening bracket of an assertion, inside a `cfg_attr` list
//!     before its `ignore`, between `#[` and `allow(`, or between a comment's opener and
//!     its end (that last is the matter of the starts above).
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
//! - A construct longer than its bound is not read. One the hunk shows only a part of is
//!   read when the part it shows ends or begins on the word the construct is known by
//!   (above), and else not.
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
}

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
/// what they find is united by the caller. No view is ever a mixture of two readings. `edges`
/// says which ends of the sides are read as the edges of constructs.
pub(crate) fn read_hunk(
    lines: &[&PatchLine],
    views: &[(Option<String>, Option<String>)],
    old: bool,
    new: bool,
    edges: Edges,
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
            read_new(side(true), edges)
        } else {
            Vec::new()
        },
        old: if old && has(PatchLineKind::Removed) {
            read_old(side(false), edges)
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
    Todo,
    Unimplemented,
    Ignore,
    CfgAttr,
    Cfg,
    Test,
    Rstest,
    Tokio,
    True,
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
    Other,
}

impl Word {
    const fn panics(self) -> bool {
        matches!(self, Self::Unwrap | Self::Expect)
    }

    const fn stubs(self) -> bool {
        matches!(self, Self::Todo | Self::Unimplemented)
    }

    /// How many `true` make an assertion of this name one that cannot fail, if it is an
    /// assertion.
    const fn asserts(self) -> Option<usize> {
        match self {
            Self::Assert | Self::DebugAssert => Some(1),
            Self::AssertEq | Self::DebugAssertEq => Some(2),
            _ => None,
        }
    }

    /// The word `name` is, whole: a longer name (`unwrap_or`, `ignored`) is another word.
    fn of(name: &[u8]) -> Self {
        match name {
            b"unwrap" => Self::Unwrap,
            b"expect" => Self::Expect,
            b"assert" => Self::Assert,
            b"debug_assert" => Self::DebugAssert,
            b"assert_eq" => Self::AssertEq,
            b"debug_assert_eq" => Self::DebugAssertEq,
            b"todo" => Self::Todo,
            b"unimplemented" => Self::Unimplemented,
            b"ignore" => Self::Ignore,
            b"cfg_attr" => Self::CfgAttr,
            b"cfg" => Self::Cfg,
            b"test" => Self::Test,
            b"rstest" => Self::Rstest,
            b"tokio" => Self::Tokio,
            b"true" => Self::True,
            b"Result" => Self::Result,
            b"Option" => Self::Option,
            b"Command" => Self::Command,
            b"new" => Self::New,
            b"process" => Self::Process,
            b"reqwest" => Self::Reqwest,
            b"ureq" => Self::Ureq,
            b"TcpStream" => Self::TcpStream,
            b"connect" => Self::Connect,
            b"UdpSocket" => Self::UdpSocket,
            b"bind" => Self::Bind,
            b"allow" => Self::Allow,
            _ => Self::Other,
        }
    }
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
struct Token {
    what: What,
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
    type Item = Token;

    fn next(&mut self) -> Option<Token> {
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
    held: Vec<Token>,
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
    fn token(&self, at: usize) -> Option<Token> {
        self.held.get(at).copied()
    }

    fn here(&self) -> Option<Token> {
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

/// A placeholder, when the current token is `todo` or `unimplemented`: `!` and an opening
/// bracket.
fn stub<'a, I: Iterator<Item = SideLine<'a>>>(window: &Window<'a, I>) -> Option<(usize, usize)> {
    let at = window.at;
    let opened = matches!(window.token(at + 2)?.what, What::Mark(mark) if closer(mark).is_some());
    (window.marks(at + 1, b'!') && opened).then_some((at, at + 2))
}

/// `#[ignore]`, `#[ignore = "…"]` and the inner forms `#![ignore]` (inside a test's body it
/// switches the test off as the outer one does; compiled and run), when the current token is
/// `#`. A `#` straight after a name is read like any other: up to edition 2018 `x#[ignore]` is
/// the tokens `x`, `#`, `[`, `ignore`, `]` to the compiler too, and a macro can drop the `x`
/// (compiled and run on edition 2018: the test does not run; from edition 2021 it is an error).
fn ignored<'a, I: Iterator<Item = SideLine<'a>>>(window: &Window<'a, I>) -> Option<(usize, usize)> {
    let at = window.at;
    let open = if window.marks(at + 1, b'!') {
        at + 2
    } else {
        at + 1
    };
    let ends = window.marks(open + 2, b']') || window.marks(open + 2, b'=');
    (window.marks(open, b'[') && window.word(open + 1) == Some(Word::Ignore) && ends)
        .then_some((at, open + 2))
}

/// A test's attribute, when the current token is `#`: `#[test]`, `#[tokio::test]` or
/// `#[rstest]`, closed there or followed by arguments (`#[tokio::test(flavor = "…")]`; third
/// independent review, IR3-L-4: only the bare three were read).
fn test_attribute<'a, I: Iterator<Item = SideLine<'a>>>(
    window: &Window<'a, I>,
) -> Option<(usize, usize)> {
    let at = window.at;
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

/// A path that release 0.8.3 keyed `process_spawn` or `network_fetch` on, when the current
/// token is the name it starts from: `Command::new` and `process::Command` (from `Command`),
/// `reqwest::` and `ureq::`, `TcpStream::connect`, `UdpSocket::bind`. That release looked for
/// each as one string on one line, and so does its rule here still; white space, a comment
/// or a line end between the names hid the path from it (`std::process :: Command :: new`,
/// or `Command` and `::new` on two lines: both compile and run; seventh independent review,
/// IR7-M-3). Read as tokens the path is the same names and `::` whatever stands between
/// them. Nothing more is modelled than the release keyed on: no `use` alias, no glob import.
fn released_path<'a, I: Iterator<Item = SideLine<'a>>>(
    window: &Window<'a, I>,
) -> Option<(EvidenceSignal, (usize, usize))> {
    use EvidenceSignal as S;
    let at = window.at;
    let then = |name: Word| window.path(at + 1) && window.word(at + 3) == Some(name);
    match window.word(at)? {
        Word::Command => {
            let owner = at.checked_sub(3).filter(|owner| {
                window.path(owner + 1) && window.word(*owner) == Some(Word::Process)
            });
            let called = then(Word::New);
            (owner.is_some() || called).then(|| {
                (
                    S::ProcessSpawn,
                    (owner.unwrap_or(at), if called { at + 3 } else { at }),
                )
            })
        }
        Word::Reqwest | Word::Ureq => window
            .path(at + 1)
            .then_some((S::NetworkFetch, (at, at + 2))),
        Word::TcpStream => then(Word::Connect).then_some((S::NetworkFetch, (at, at + 3))),
        Word::UdpSocket => then(Word::Bind).then_some((S::NetworkFetch, (at, at + 3))),
        _ => None,
    }
}

/// `#[allow(` and `#![allow(`, when the current token is `#`: what release 0.8.3 keyed
/// `check_suppressed` on, with anything between the tokens (a comment, a line end).
fn allowed<'a, I: Iterator<Item = SideLine<'a>>>(window: &Window<'a, I>) -> Option<(usize, usize)> {
    let at = window.at;
    let open = if window.marks(at + 1, b'!') {
        at + 2
    } else {
        at + 1
    };
    (window.marks(open, b'[')
        && window.word(open + 1) == Some(Word::Allow)
        && window.marks(open + 2, b'('))
    .then_some((at, open + 2))
}

/// Which edges of a side are read as edges of a construct ([`waiting`], [`completed`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Edges {
    /// The side's first tokens may complete a construct whose head stands before the side.
    /// So for the reading from code, and not for a reading that begins inside a comment or
    /// a string: the first tokens of those are not the first of the side.
    pub start: bool,
    /// The side's last tokens may begin a construct that goes on after the side.
    pub end: bool,
}

/// The constructs that the end of the side leaves waiting: `window` holds the side's last
/// tokens, and they are the beginning of a construct up to and with the word it is known by.
///
/// A hunk shows a part of a file and does not say what stands after its last line. Two hunks
/// that stand together in the file can be written so that nothing says they do (the second
/// with a false position: Git finds it by its text), and a construct cut in two by them is
/// then read in two halves, by no join (seventh independent review: `cutpair.patch`; the
/// same holds for a second section for one path). So the edge of a side is read as what it
/// is, unknown text, as the start of a side already is: a word that a construct is known by,
/// left at the end with the rest missing, is evidence that the construct may stand there.
/// These are the forms that code never shows at the end of a line of its own (measured on
/// 177,957 fragments of real crate files cut at fixed lines, on this repository's history at
/// three context widths, and on 603 crates: none of them stands at the end of any side):
///
/// - `.unwrap`, `.expect`, `::unwrap`, `::expect`, alone or with the `::` or `::<` of generic
///   arguments begun;
/// - `todo`, `unimplemented` and the names of the assertions, alone or with their `!`; and an
///   assertion as far as its `true` (`assert!(true`, `assert_eq!(true, true`);
/// - `#[ignore` and `#![ignore` without their end, and `#[allow` and `#![allow`;
/// - `Command`, `Command::`, `process::`, `TcpStream`, `TcpStream::`, `UdpSocket`,
///   `UdpSocket::`, `reqwest`, `ureq`;
/// - on the old side, `#[test`, `#[rstest`, `#[tokio`, `#[tokio::` and `#[tokio::test`.
///
/// Not read, because real code ends lines so (the same measurement): an assertion cut after
/// its opening bracket (`assert!(` ends a line of every assertion written over several: 279
/// sides), a bare `#` or `#[`, and a `cfg_attr` list left open that does not end on `ignore`
/// (217; one that does is read by [`Skips::end`]).
fn waiting<'a, I: Iterator<Item = SideLine<'a>>>(
    window: &Window<'a, I>,
    found: &mut Found,
    new: bool,
) {
    use EvidenceSignal as S;
    let Some(end) = window.held.len().checked_sub(1) else {
        return;
    };
    // the token `back` places before the last one
    let word = |back: usize| end.checked_sub(back).and_then(|at| window.word(at));
    let mark = |back: usize, mark: u8| {
        end.checked_sub(back)
            .is_some_and(|at| window.marks(at, mark))
    };
    let path = |back: usize| end.checked_sub(back).is_some_and(|at| window.path(at));
    // `#[` or `#![` ends `back` places before the last token: the place of its `#`
    let attribute = |back: usize| {
        if !mark(back, b'[') {
            None
        } else if mark(back + 1, b'#') {
            Some(end - back - 1)
        } else if mark(back + 1, b'!') && mark(back + 2, b'#') {
            Some(end - back - 2)
        } else {
            None
        }
    };
    if !new {
        // a test's attribute as far as it goes
        let hash = match word(0) {
            Some(Word::Test) if path(2) && word(3) == Some(Word::Tokio) => attribute(4),
            Some(Word::Test | Word::Rstest | Word::Tokio) => attribute(1),
            None if path(1) && word(2) == Some(Word::Tokio) => attribute(3),
            _ => None,
        };
        if let Some(hash) = hash.filter(|hash| window.marks(hash + 1, b'[')) {
            found.edge(window, S::TestRemoved, hash, end);
        }
        return;
    }
    // a panic path: the word after `.` or `::`, then nothing, or `::` or `::<`
    let panics = [0, 2, 3].into_iter().find(|back| {
        word(*back).is_some_and(Word::panics)
            && match back {
                0 => true,
                2 => path(1),
                _ => path(2) && mark(0, b'<'),
            }
    });
    if let Some(back) = panics {
        if mark(back + 1, b'.') {
            found.edge(window, S::PanicPathAdded, end - back - 1, end);
        } else if path(back + 2) {
            found.edge(window, S::PanicPathAdded, end - back - 2, end);
        }
    }
    // a stub or an assertion: the name, then nothing or `!`
    for back in [0, 1] {
        if back == 1 && !mark(0, b'!') {
            continue;
        }
        match word(back) {
            Some(name) if name.stubs() => found.edge(window, S::StubAdded, end - back, end),
            Some(name) if name.asserts().is_some() => {
                found.edge(window, S::TrivialAssert, end - back, end);
            }
            _ => {}
        }
    }
    // an assertion as far as its `true`: `assert!(true`, `assert_eq!(true`, `assert_eq!(true,`
    // and `assert_eq!(true, true`
    for back in 3..=5 {
        let Some(trues) = word(back).and_then(Word::asserts) else {
            continue;
        };
        let opened = mark(back - 1, b'!')
            && end
                .checked_sub(back - 2)
                .and_then(|at| window.token(at))
                .is_some_and(
                    |token| matches!(token.what, What::Mark(open) if closer(open).is_some()),
                );
        let rest: Vec<Option<Word>> = (0..back - 2).rev().map(word).collect();
        let truths = match (trues, rest.as_slice()) {
            (_, [Some(Word::True)]) => true,
            (2, [Some(Word::True), None]) => mark(0, b','),
            (2, [Some(Word::True), None, Some(Word::True)]) => mark(1, b','),
            _ => false,
        };
        if opened && truths {
            found.edge(window, S::TrivialAssert, end - back, end);
        }
    }
    // an attribute as far as its name
    match (word(0), attribute(1)) {
        (Some(Word::Ignore), Some(hash)) => found.edge(window, S::TestSkipped, hash, end),
        (Some(Word::Allow), Some(hash)) => found.edge(window, S::CheckSuppressed, hash, end),
        _ => {}
    }
    // a path of release 0.8.3's signals as far as its first name, or that and `::`
    let back = if path(1) { 2 } else { 0 };
    match word(back) {
        Some(Word::Command) => found.edge(window, S::ProcessSpawn, end - back, end),
        Some(Word::Process) if back == 2 => found.edge(window, S::ProcessSpawn, end - back, end),
        Some(Word::TcpStream | Word::UdpSocket) => {
            found.edge(window, S::NetworkFetch, end - back, end);
        }
        Some(Word::Reqwest | Word::Ureq) if back == 0 => {
            found.edge(window, S::NetworkFetch, end, end);
        }
        _ => {}
    }
}

/// The constructs that the beginning of the side completes: `window` stands at the side's
/// first token, and the first tokens are the end of a construct from the word it is known by,
/// with what stands before that word missing. The other half of [`waiting`], for a cut before
/// the word: `x.` and then `unwrap()`, `#[` and then `ignore]`.
///
/// - `unwrap(` and `expect(` (also with generic arguments), with no `.` or `::` before them;
/// - `ignore]`, `ignore =`, and those after `[` or `![`; `[allow(` and `![allow(`;
/// - `::Command` (the `process` before it missing);
/// - on the old side, `test]`, `rstest]`, `tokio::test`, those with arguments, and each after
///   `[`.
///
/// Measured as above: two sides of the 177,957 begin with a call of a function of the
/// crate's own named `expect` or `unwrap`, and are false items when that line is a changed
/// one; none begins in any other of these ways. Not read: a side that begins with `(` (4,279
/// of them: what a call cut after its name leaves is read at the end of the other half),
/// with `!(`, with `allow(`, or with `ignore,` or `ignore)` (a name any argument can have).
fn completed<'a, I: Iterator<Item = SideLine<'a>>>(
    window: &Window<'a, I>,
    found: &mut Found,
    new: bool,
) {
    use EvidenceSignal as S;
    // after `[` or `![`, or after nothing
    let open = if window.marks(0, b'[') {
        Some(1)
    } else if window.marks(0, b'!') && window.marks(1, b'[') {
        Some(2)
    } else {
        None
    };
    let at = open.unwrap_or(0);
    if !new {
        let end = match window.word(at) {
            Some(Word::Test | Word::Rstest) => at + 1,
            Some(Word::Tokio) if window.path(at + 1) && window.word(at + 3) == Some(Word::Test) => {
                at + 4
            }
            _ => return,
        };
        if open != Some(2) && (window.marks(end, b']') || window.marks(end, b'(')) {
            found.edge(window, S::TestRemoved, 0, end);
        }
        return;
    }
    match window.word(at) {
        Some(Word::Ignore) if window.marks(at + 1, b']') || window.marks(at + 1, b'=') => {
            found.edge(window, S::TestSkipped, 0, at + 1);
        }
        Some(Word::Allow) if open.is_some() && window.marks(at + 1, b'(') => {
            found.edge(window, S::CheckSuppressed, 0, at + 1);
        }
        Some(name) if open.is_none() && name.panics() => {
            let generic = window.path(1) && window.marks(3, b'<');
            if window.marks(1, b'(') || generic {
                found.edge(window, S::PanicPathAdded, 0, 1);
            }
        }
        _ => {}
    }
    if window.path(0) && window.word(2) == Some(Word::Command) {
        found.edge(window, S::ProcessSpawn, 0, 2);
    }
}

/// The signals the reader gives, in the order of [`EvidenceSignal`], which is the order its
/// items come out in for one line.
const READ: [EvidenceSignal; 8] = [
    EvidenceSignal::ProcessSpawn,
    EvidenceSignal::NetworkFetch,
    EvidenceSignal::CheckSuppressed,
    EvidenceSignal::TestRemoved,
    EvidenceSignal::TestSkipped,
    EvidenceSignal::TrivialAssert,
    EvidenceSignal::StubAdded,
    EvidenceSignal::PanicPathAdded,
];

/// What one line holds: for each of [`READ`], the basis and the hint of the constructs of that
/// signal found on it so far, taken together.
type OnLine = [Option<(Basis, bool)>; READ.len()];

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
            Some((basis, likely)) => (basis.max(item.basis), likely && item.test_likely),
            None => (item.basis, item.test_likely),
        });
    }

    /// Note `signal` for the construct `window` holds from `first` to `last`, when one of its
    /// tokens is on a changed line.
    fn construct<'a, I: Iterator<Item = SideLine<'a>>>(
        &mut self,
        window: &Window<'a, I>,
        signal: EvidenceSignal,
        (first, last): (usize, usize),
        test_likely: bool,
    ) {
        let (changed, basis) = window.extent(first, last);
        if let Some(line) = changed {
            self.note(Item {
                line,
                signal,
                basis,
                test_likely,
            });
        }
    }

    /// Note `signal` for a part of a construct that stands at an edge of the side, the tokens
    /// from `first` to `last`, when one of them is on a changed line. Its basis is `hunk`:
    /// the rest of the construct is not on this side, and what the item says is that the
    /// hunk shows its beginning or its end.
    fn edge<'a, I: Iterator<Item = SideLine<'a>>>(
        &mut self,
        window: &Window<'a, I>,
        signal: EvidenceSignal,
        first: usize,
        last: usize,
    ) {
        if let (Some(line), _) = window.extent(first, last) {
            self.note(Item {
                line,
                signal,
                basis: Basis::Hunk,
                test_likely: false,
            });
        }
    }

    /// The items by line and then by signal, one for each pair: `lines` and `late` are each in
    /// line order and are walked together once, so this costs the answer's length.
    fn sorted(self) -> Vec<Item> {
        fn join(into: &mut OnLine, from: OnLine) {
            for (place, more) in from.into_iter().enumerate() {
                into[place] = match (into[place], more) {
                    (Some((basis, likely)), Some((other, also))) => {
                        Some((basis.max(other), likely && also))
                    }
                    (was, more) => was.or(more),
                };
            }
        }
        let mut out = Vec::new();
        let mut push = |line: usize, on_line: OnLine| {
            for (signal, held) in READ.into_iter().zip(on_line) {
                if let Some((basis, test_likely)) = held {
                    out.push(Item {
                        line,
                        signal,
                        basis,
                        test_likely,
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
#[derive(Default)]
struct Skips {
    open: Vec<List>,
}

impl Skips {
    /// Note the skip of `list`, whose `ignore` is on line `ignore_line`, when a token of it up to
    /// there is on a changed line (`changed`, the first such line).
    fn skip(found: &mut Found, list: &List, changed: Option<usize>, ignore_line: usize) {
        if let Some(line) = changed {
            found.note(Item {
                line,
                signal: EvidenceSignal::TestSkipped,
                basis: if list.line == ignore_line {
                    Basis::Lexical
                } else {
                    Basis::Hunk
                },
                test_likely: false,
            });
        }
    }

    /// Read the current token of `window`.
    fn read<'a, I: Iterator<Item = SideLine<'a>>>(
        &mut self,
        window: &Window<'a, I>,
        found: &mut Found,
    ) {
        let Some(token) = window.here() else { return };
        let closed = self.open.last_mut().is_some_and(|list| {
            Self::follow(list, token, found);
            list.closed
        });
        if closed
            && let Some(inner) = self.open.pop()
            && let Some(around) = self.open.last_mut()
        {
            // the inner list's `(` was a bracket of the list around it, and so were its tokens
            around.depth = around.depth.saturating_sub(1);
            around.changed = around.changed.or(inner.changed);
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
            closed: false,
        };
        if self.open.len() >= CFG_ATTR_LISTS {
            // not followed: taken to hold a skip
            Self::skip(found, &list, changed, token.line);
            return;
        }
        self.open.push(list);
    }

    /// One token of the innermost open list.
    fn follow(list: &mut List, token: Token, found: &mut Found) {
        if let Some((changed, line)) = list.candidate.take()
            && matches!(token.what, What::Mark(b',' | b')' | b'='))
        {
            Self::skip(found, list, changed, line);
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
                    if word == Word::Ignore && list.argument > 0 && !list.spent {
                        list.candidate = Some((list.changed, token.line));
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

    /// The side has ended. A list that ends on `ignore` is taken to hold the skip: the hunk
    /// shows no more of it, and what is not shown is no reason to drop evidence.
    fn end(&self, found: &mut Found) {
        for list in &self.open {
            if let Some((changed, line)) = list.candidate {
                Self::skip(found, list, changed, line);
            }
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
    fn read(&mut self, token: Token) -> bool {
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
/// ([`released_path`], [`allowed`]), beside its own rules for them, which read one line. And
/// what stands at the edges of the side is read as the part of a construct that it may be
/// ([`waiting`], [`completed`]), where `edges` says so.
pub(crate) fn read_new<'a>(lines: impl Iterator<Item = SideLine<'a>>, edges: Edges) -> Vec<Item> {
    use EvidenceSignal as S;
    let mut window = Window::over(lines);
    let mut found = Found::default();
    let mut skips = Skips::default();
    let mut tests = TestStretch::default();
    if edges.start {
        completed(&window, &mut found, true);
    }
    while let Some(token) = window.here() {
        tests.read(&window);
        skips.read(&window, &mut found);
        let construct = match token.what {
            What::Name(Word::Unwrap | Word::Expect) => {
                panic_path(&window).map(|extent| (S::PanicPathAdded, extent))
            }
            What::Name(Word::Assert | Word::DebugAssert) => {
                trivial_assert(&window, 1).map(|extent| (S::TrivialAssert, extent))
            }
            What::Name(Word::AssertEq | Word::DebugAssertEq) => {
                trivial_assert(&window, 2).map(|extent| (S::TrivialAssert, extent))
            }
            What::Name(Word::Todo | Word::Unimplemented) => {
                stub(&window).map(|extent| (S::StubAdded, extent))
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
            found.construct(&window, signal, extent, likely);
        }
        if edges.end && window.token(window.at + 1).is_none() {
            waiting(&window, &mut found, true);
        }
        window.step();
    }
    skips.end(&mut found);
    found.sorted()
}

/// The signals of the old side of a hunk, on its removed lines: `lines` are its unchanged and
/// removed lines in order. A test's attribute with at least one token on a removed line is a
/// removed test, on the first such line.
pub(crate) fn read_old<'a>(lines: impl Iterator<Item = SideLine<'a>>, edges: Edges) -> Vec<Item> {
    let mut window = Window::over(lines);
    let mut found = Found::default();
    if edges.start {
        completed(&window, &mut found, false);
    }
    while let Some(token) = window.here() {
        if token.what == What::Mark(b'#')
            && let Some(extent) = test_attribute(&window)
        {
            found.construct(&window, EvidenceSignal::TestRemoved, extent, false);
        }
        if edges.end && window.token(window.at + 1).is_none() {
            waiting(&window, &mut found, false);
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
            read_new(lines.into_iter(), NO_EDGES),
            [
                item(0, S::StubAdded, Basis::Lexical, false),
                item(0, S::PanicPathAdded, Basis::Lexical, false),
            ]
        );
    }

    /// The constructs alone: the tests of each construct read a side whose ends are ends.
    const NO_EDGES: Edges = Edges {
        start: false,
        end: false,
    };

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
        read_new(lines_of(&side(lines)), NO_EDGES)
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
        read_old(lines_of(&side(lines)), NO_EDGES)
            .iter()
            .map(|item| (item.signal, item.basis, item.line))
            .collect()
    }

    /// The hints of the panic paths of the new side `lines`: `(line index, test_likely)`.
    fn hints(lines: &[&str]) -> Vec<(usize, bool)> {
        read_new(lines_of(&side(lines)), NO_EDGES)
            .iter()
            .filter(|item| item.signal == S::PanicPathAdded)
            .map(|item| (item.line, item.test_likely))
            .collect()
    }

    fn tokens(lines: &[&str]) -> Vec<Token> {
        let side = side(lines);
        Tokens {
            lines: lines_of(&side),
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
            words("+unwrap_or unwrapé éunwrap 1unwrap ignored Test result _todo todo_ assert_ne"),
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
            "+assert!((true));",
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
            "+#[ignore(x)]",
            "+#[ignore",
            "+#[doc = ignore]",
            "+#!![ignore]",
        ] {
            assert_eq!(signals(&[line]), [], "{line}");
        }
        // an `ignore` that is another attribute's argument is no skip (the attribute is
        // read for what it is)
        assert_eq!(signals(&["+#[allow(ignore)]"]), [S::CheckSuppressed]);
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
            ("miri, ignore", true),
            ("ignore, allow(x))]", false),
            ("miri, allow(ignore))]", false),
            ("feature = \"\", serde(skip, ignore, default))]", false),
            ("miri, ignored)]", false),
            ("miri)]", false),
            ("miri, ignore x)]", false),
            ("miri, x ignore)]", false),
            ("miri, ignore(x))]", false),
            // IR2-M-2
            ("all(), cfg_attr(all(), ignore))]", true),
            ("all(), cfg_attr ( all() , ignore = \"\" ) )]", true),
            ("a, allow(x), cfg_attr(b, inline, ignore))]", true),
            ("a, cfg_attr(b, cfg_attr(c, ignore)))]", true),
            ("a, cfg_attr(b, cfg_attr(c, inline)), ignore)]", true),
            ("a, cfg_attr(b, ignore", true),
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
            let expected: &[EvidenceSignal] = if skips { &[S::TestSkipped] } else { &[] };
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
        let deep = format!("+#[cfg_attr(a, {}ignore", "cfg_attr(a, ".repeat(200_000));
        assert_eq!(signals(&[&deep]), [S::TestSkipped]);
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
        let other = read_new(
            lines_of(&side(&["+#[test]", "+fn a() { todo!(); }"])),
            NO_EDGES,
        );
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
            read_hunk(
                &hunk.lines.iter().collect::<Vec<_>>(),
                &views,
                true,
                true,
                NO_EDGES,
            )
        };
        // the removed line between the dot and the name is not on the new side
        let found = read(" x.\n-#[test]\n+unwrap()\n");
        assert_eq!(
            found.new,
            [Item {
                line: 2,
                signal: S::PanicPathAdded,
                basis: Basis::Hunk,
                test_likely: false
            }]
        );
        assert_eq!(
            found.old,
            [Item {
                line: 1,
                signal: S::TestRemoved,
                basis: Basis::Lexical,
                test_likely: false
            }]
        );
        // a side with no line of the change is not read: nothing is evidence on it anyway
        assert_eq!(read(" #[\n+test]\n").old, []);
        assert_eq!(read(" x.\n-unwrap()\n").new, []);
    }

    /// What the new side `lines` gives when both its ends are read as edges.
    fn edged(lines: &[&str]) -> Vec<Found3> {
        let edges = Edges {
            start: true,
            end: true,
        };
        read_new(lines_of(&side(lines)), edges)
            .iter()
            .map(|item| (item.signal, item.basis, item.line))
            .collect()
    }

    /// The same for the old side.
    fn edged_old(lines: &[&str]) -> Vec<Found3> {
        let edges = Edges {
            start: true,
            end: true,
        };
        read_old(lines_of(&side(lines)), edges)
            .iter()
            .map(|item| (item.signal, item.basis, item.line))
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

    /// The end of a side is unknown text, as its start is: a word that a construct is known
    /// by, left there with the rest missing, is evidence that the construct may stand there.
    /// Two hunks that touch behind positions that do not say so are joined by nothing, and a
    /// second section for one path is another file to this crate (seventh independent
    /// review: `cutpair.patch`, `twosec.patch`); what each half ends on is still read.
    #[test]
    fn what_a_side_ends_on_is_read_as_the_beginning_of_a_construct() {
        for (end, signal) in [
            ("let v = opt.unwrap", S::PanicPathAdded),
            ("let v = opt . expect", S::PanicPathAdded),
            ("let v = X::unwrap", S::PanicPathAdded),
            ("let v = opt.unwrap::", S::PanicPathAdded),
            ("let v = X::expect::<", S::PanicPathAdded),
            ("todo", S::StubAdded),
            ("unimplemented !", S::StubAdded),
            ("assert", S::TrivialAssert),
            ("debug_assert_eq!", S::TrivialAssert),
            ("assert!(true", S::TrivialAssert),
            ("assert_eq![true", S::TrivialAssert),
            ("assert_eq!(true,", S::TrivialAssert),
            ("assert_eq!(true, true", S::TrivialAssert),
            ("#[ignore", S::TestSkipped),
            ("#![ignore", S::TestSkipped),
            ("#[allow", S::CheckSuppressed),
            ("let c = Command", S::ProcessSpawn),
            ("let c = Command::", S::ProcessSpawn),
            ("use std::process::", S::ProcessSpawn),
            ("let s = TcpStream", S::NetworkFetch),
            ("let s = UdpSocket ::", S::NetworkFetch),
            ("let r = reqwest", S::NetworkFetch),
            ("let r = ureq", S::NetworkFetch),
        ] {
            let line = format!("+{end}");
            // at the end of the side it is an item, on the changed line that holds it
            assert_eq!(
                edged(&[" fn f() {", &line]),
                [(signal, Basis::Hunk, 1)],
                "{end}"
            );
            // on an unchanged line it is none, and neither is it before the end
            let unchanged = format!(" {end}");
            assert_eq!(edged(&["+fn f() {", &unchanged]), [], "{end}");
            assert_eq!(edged(&[&line, "+;"]), [], "{end}");
            // and a side whose end is not read as an edge has none
            assert_eq!(new(&[" fn f() {", &line]), [], "{end}");
        }
        // over two lines: on the first changed one
        assert_eq!(
            edged(&["+let v = opt .", " unwrap"]),
            [(S::PanicPathAdded, Basis::Hunk, 0)]
        );
        // what real code ends lines with is no item: an assertion's opening bracket, a bare
        // `#` or `#[`, a longer word, `process` alone, `assert_eq!(true` with another word
        for end in [
            "assert!(",
            "assert_eq!(a,",
            "#",
            "#[",
            "#[cfg_attr(miri,",
            "x.unwrap_or",
            "unwrap",
            "let p = self.process",
            "assert!(truth",
            "assert!(true ==",
            "#[ignored",
        ] {
            let line = format!("+{end}");
            assert_eq!(edged(&[" fn f() {", &line]), [], "{end}");
        }
        // the old side: a test's attribute as far as it goes
        for end in [
            "#[test",
            "#[rstest",
            "#[tokio",
            "#[tokio::",
            "#[tokio::test",
        ] {
            let line = format!("-{end}");
            assert_eq!(
                edged_old(&[" fn f() {}", &line]),
                [(S::TestRemoved, Basis::Hunk, 1)],
                "{end}"
            );
            assert_eq!(old(&[" fn f() {}", &line]), [], "{end}");
        }
        assert_eq!(edged_old(&["-#![test"]), []);
        assert_eq!(edged_old(&["-#[testing"]), []);
    }

    /// The beginning of a side, for a cut before the word: the first tokens are the end of a
    /// construct whose head stands before the side.
    #[test]
    fn what_a_side_begins_with_is_read_as_the_end_of_a_construct() {
        for (start, signal) in [
            ("unwrap();", S::PanicPathAdded),
            ("expect(\"x\");", S::PanicPathAdded),
            ("unwrap::<u8>();", S::PanicPathAdded),
            ("ignore]", S::TestSkipped),
            ("ignore = \"slow\"]", S::TestSkipped),
            ("[ignore]", S::TestSkipped),
            ("![ignore]", S::TestSkipped),
            ("[allow(dead_code)]", S::CheckSuppressed),
            ("![allow(unused)]", S::CheckSuppressed),
            ("::Command::new(\"sh\")", S::ProcessSpawn),
        ] {
            let line = format!("+{start}");
            assert_eq!(
                edged(&[&line, " fn g() {}"]),
                [(signal, Basis::Hunk, 0)],
                "{start}"
            );
            // on an unchanged line, and after another token, it is none
            let unchanged = format!(" {start}");
            assert_eq!(edged(&[&unchanged, "+fn g() {}"]), [], "{start}");
            // and a side whose beginning is not read as an edge has none
            let found = read_new(
                lines_of(&side(&[&line, " fn g() {}"])),
                Edges {
                    start: false,
                    end: true,
                },
            );
            // (`::Command::new` is the path itself from its second name on)
            assert_eq!(
                found.iter().map(|item| item.signal).collect::<Vec<_>>(),
                if start.starts_with("::Command") {
                    vec![S::ProcessSpawn]
                } else {
                    vec![]
                },
                "{start}"
            );
        }
        // what real code begins lines with is no item: a bracket, `!(`, a function named
        // `allow`, an argument named `ignore`, a longer word
        for start in [
            "(a, b)",
            "!(x)",
            "allow(x);",
            "ignore,",
            "ignore)",
            "unwrap_or(1);",
            "unwrap;",
            "::new(x)",
        ] {
            let line = format!("+{start}");
            assert_eq!(edged(&[&line, " fn g() {}"]), [], "{start}");
        }
        // the old side
        for start in [
            "test]",
            "[test]",
            "rstest]",
            "tokio::test]",
            "[tokio::test(flavor = \"x\")]",
        ] {
            let line = format!("-{start}");
            assert_eq!(
                edged_old(&[&line, " fn t() {}"]),
                [(S::TestRemoved, Basis::Hunk, 0)],
                "{start}"
            );
        }
        assert_eq!(edged_old(&["-![test]", " fn t() {}"]), []);
        assert_eq!(edged_old(&["-testing]", " fn t() {}"]), []);
    }
}
