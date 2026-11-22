// DDF integration: line-lexical code view for evidence signals (plan 2026-10-30, Feature A).
// Design/status: docs/PLAN_AGENT_EVIDENCE_AND_OBSERVABILITY.md#feature-a-code-evidence-risk-signals
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fcrates--deep-diff-forge-graph--src--lexical
//! A line-lexical view of source code: what is left of a line once its comments and string
//! contents are blanked. Evidence rules match tokens in that view, so a keyword in a comment or
//! a string is not read as code. Code that runs inside a string is kept: `$(…)`, `${…}` and
//! backticks in a shell double-quoted string, `${…}` in a JS template literal and `{…}` in a
//! one-line Python f- or t-string, so quoting a fetch or a spawn does not hide it.
//!
//! This is lexical, not parsing, and where it can misread it errs toward code: an extra
//! evidence item is a false positive a reviewer sees, a hidden one is a miss nobody does.
//! - Block comments and triple-quoted strings are followed across the lines of one hunk, in
//!   order ([`ScanState`]); a hunk that starts inside one is read as code.
//! - The hunks of a file the patch creates are one text: a later one is read once more from
//!   what the hunks before it left open (`carried_view`), beside the readings that start it
//!   as code. The hunks of a file that exists are read each on its own.
//! - A string still open at the end of a line (outside Rust) is read as code from its quote,
//!   and is also carried to the next line of the hunk. That line is read twice, as code and as
//!   resuming inside the string, and both readings go on down the hunk (up to four at once);
//!   what any reading finds code is code. So the closing quote of a multi-line string cannot
//!   open a phantom string over the code after it. The plain reading, which resumes no
//!   string, is always one of them, so a quote misread as an opener hides no later line that a
//!   scan without carried strings keeps; it can still leave that plain reading in a block
//!   comment or triple string, as any misread quote could before strings were carried.
//! - Rust has three readings, and a rule that reads code is asked of each on its own
//!   (one view at a time): what any of them finds is found, and the bytes of two readings are
//!   never mixed. The first is the shared scanner's, [`code_view`] with [`Language::Rust`],
//!   exactly as every release from 0.5.0 to 0.8.3 had it: it ends a block comment at the first
//!   `*/` and knows a character literal by a closing quote two or three bytes on. The same
//!   scanner is also kept as the module `legacy_rust` ([`rust_legacy_views`]), which the
//!   readings of whole hunk sides use. The second, [`rust_lexer_view`], reads Rust as the
//!   compiler's own lexer does (the `rust` module of this file): token by token, with nested
//!   block comments, character and byte literals with any escape, lifetimes and labels, raw
//!   strings with any number of `#`, raw identifiers, and numbers and literal suffixes (which
//!   decide whether an `r` or a `b` before a quote is a prefix). The third reads with the same
//!   lexer one side of a hunk whole, from a stated start ([`RustStart`]): one exact reading,
//!   nothing read twice and no two readings mixed, so a view never shows a byte its own reading
//!   takes for a comment or a literal. Where the start of a hunk is not known, its side is read
//!   as more than one whole view ([`rust_views`]: from code, from inside a block comment, from
//!   inside a string or a raw string). A fuzz target (`rust_code_view`) holds the lexer's
//!   readings to that lexer: no byte of a token that is code is ever blank. The first reading
//!   stays so that no answer is lower than 0.8.3's. Where a hunk starts inside a string or a
//!   comment opened above it and no start says so, no reading is the compiler's, and each can
//!   find code the others do not.
//! - An interpolation is scanned as code with its own strings, escapes and comments, and ends
//!   at its matching closer on the same line. Shell `case` patterns, ANSI-C `$'…'` strings,
//!   word-start `#` comments, JS regex literals (also on a line that continues an
//!   expression), TypeScript non-null assertions, Python format specs, nested triple quotes,
//!   Rust raw strings (`r#"…"#`, `br"…"`, `cr"…"`) and Go raw strings (backticks, which run no
//!   escapes) are known. YAML quoted scalars are kept as code, but a `#` inside one (or inside
//!   a quoted word of a `run: |` shell line) starts no comment.
//! - A construct the scanner does not know (a heredoc) is read as code. A
//!   triple-quoted f-string's interpolations are blanked with the rest of it, a continuation
//!   line of a multi-line string is read as code, and non-ASCII spellings of a token
//!   (Python's NFKC `ｅval`) are not matched.
//! - JSX text, a regex right after `if (…)`, a `case` inside a shell `$(…)` that a construct
//!   above it confuses, and message text in a shell `${x:-…}` are read approximately.
//!
//! It is a reviewer's lens, not an adversarial boundary: an author who wants to hide a call
//! from a line scanner can (assembling it at run time always works), and no evidence means no
//! token was found, never that a change is safe. Evidence built on it says `basis: lexical`.

/// A language family the scanner knows, chosen from the path (and a shebang for files with no
/// extension).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Language {
    Rust,
    Python,
    /// JavaScript and TypeScript.
    JavaScript,
    Go,
    /// Shell, and the shell-like build files (`Dockerfile`, `Makefile`).
    Shell,
    /// YAML (CI workflows): only `#` comments are blanked; quoted scalars are kept, and a `#`
    /// inside one is not a comment.
    Yaml,
}

impl Language {
    /// The language for `path`, or for a file with no known extension, the one its shebang
    /// (`first_line`) names. `None` when neither says: no rules apply rather than a guess.
    #[must_use]
    pub fn of(path: &str, first_line: Option<&str>) -> Option<Self> {
        let name = path.rsplit('/').next().unwrap_or(path);
        if name == "Dockerfile" || name.starts_with("Dockerfile.") || name == "Makefile" {
            return Some(Self::Shell);
        }
        let by_extension = name.rsplit_once('.').and_then(|(_, ext)| match ext {
            "rs" => Some(Self::Rust),
            "py" | "pyi" => Some(Self::Python),
            "js" | "mjs" | "cjs" | "jsx" | "ts" | "mts" | "cts" | "tsx" => Some(Self::JavaScript),
            "go" => Some(Self::Go),
            "sh" | "bash" | "zsh" | "mk" => Some(Self::Shell),
            "yml" | "yaml" => Some(Self::Yaml),
            _ => None,
        });
        by_extension.or_else(|| first_line.and_then(Self::of_shebang))
    }

    fn of_shebang(line: &str) -> Option<Self> {
        let interpreter = line.strip_prefix("#!")?.trim();
        // `#!/usr/bin/env python3` names the program after `env`.
        let program = interpreter
            .strip_prefix("/usr/bin/env")
            .map_or(interpreter, str::trim_start)
            .split_whitespace()
            .next()?;
        let program = program.rsplit('/').next().unwrap_or(program);
        if matches!(program, "sh" | "bash" | "zsh" | "dash" | "ksh") {
            Some(Self::Shell)
        } else if program.starts_with("python") {
            Some(Self::Python)
        } else if matches!(program, "node" | "deno" | "bun") {
            Some(Self::JavaScript)
        } else {
            None
        }
    }

    /// `//` and `/* */` comments; `#` is code (a Rust attribute, a JS private field).
    const fn c_like(self) -> bool {
        matches!(self, Self::Rust | Self::JavaScript | Self::Go)
    }

    /// Whether `byte` opens a string. A YAML quoted scalar is tracked only so that a `#` in it
    /// starts no comment: its contents are kept.
    const fn opens_string(self, byte: u8) -> bool {
        match self {
            Self::Rust => byte == b'"',
            Self::Python | Self::Shell | Self::Yaml => byte == b'"' || byte == b'\'',
            Self::Go | Self::JavaScript => matches!(byte, b'"' | b'\'' | b'`'),
        }
    }

    /// Whether a string opened by `quote` runs no backslash escapes: a Go raw string
    /// (backticks) and a YAML single-quoted scalar, whose only escape is a doubled quote.
    const fn raw_quote(self, quote: u8) -> bool {
        matches!((self, quote), (Self::Go, b'`') | (Self::Yaml, b'\''))
    }
}

/// What multi-line constructs the previous lines of the same hunk left open: one or more
/// readings of them, each scanned on the next line, which keeps what any of them finds code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ScanState {
    /// The plain reading: each line read from the block comment or triple string the previous
    /// plain reading left open, and from no carried string. It is never dropped, so a line keeps
    /// at least what this reading finds code, whatever the other readings carry.
    plain: Reading,
    /// Up to [`MORE_READINGS`] other readings, distinct from `plain` and from each other: the
    /// lines read as resuming inside a string a reading left open, and what followed them.
    more: [Option<Reading>; MORE_READINGS],
}

/// How many readings besides the plain one a [`ScanState`] carries. Each carried string doubles
/// a reading, so the count is capped; a reading past the cap is dropped, the plain one never.
const MORE_READINGS: usize = 3;

impl ScanState {
    /// The readings, the plain one first.
    fn readings(&self) -> impl Iterator<Item = Reading> + '_ {
        std::iter::once(self.plain).chain(self.more.iter().flatten().copied())
    }

    /// The state after a line: `readings` (the plain one first) without repeats, capped.
    fn from_readings(readings: &[Reading]) -> Self {
        let mut state = Self {
            plain: readings.first().copied().unwrap_or_default(),
            more: [None; MORE_READINGS],
        };
        let mut slots = state.more.iter_mut();
        for (i, reading) in readings.iter().enumerate().skip(1) {
            if readings[..i].contains(reading) {
                continue;
            }
            let Some(slot) = slots.next() else { break };
            *slot = Some(*reading);
        }
        state
    }
}

/// One reading of what the previous line left open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Reading {
    in_block_comment: bool,
    /// The quote character of an open Python triple-quoted string.
    in_triple: Option<u8>,
    /// In JS, the last code byte of the previous line, so a line that starts with `/` after
    /// `(a - b)` is a division and after `=` a regex.
    js_last: Option<u8>,
    /// A string the previous line left open at the top level (a [`Frame::Str`] with `start` 0),
    /// which the next line may resume inside.
    in_string: Option<Frame>,
}

/// One level of what is open on a line, innermost last on [`code_view`]'s stack. An empty stack
/// is plain code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Frame {
    /// A string opened at byte `start` by `quote` (three of them when `triple`): its contents
    /// are blanked (kept, in YAML). An f- or t-string (`fstring`) runs `{…}`; a shell ANSI-C
    /// string (`$'…'`, `ansi`) honours backslashes; a raw string (`raw`: a Rust raw string's
    /// count of `#`, or 0 for a Go raw string or a YAML single-quoted scalar) does not, and
    /// closes only at a quote followed by that many `#`.
    Str {
        quote: u8,
        start: usize,
        triple: bool,
        fstring: bool,
        ansi: bool,
        raw: Option<usize>,
        escaped: bool,
    },
    /// Code inside an interpolation, until `close` brings `depth` back to zero. `open` nests
    /// another level (`(` in `$(`), `None` for a backtick span. `cases` counts open shell `case`
    /// statements, whose pattern `)` is not a closer. In a Python replacement field, `brackets`
    /// counts open `(` and `[`, and `spec` marks the format spec after a top-level `:`, which is
    /// literal text.
    Code {
        open: Option<u8>,
        close: u8,
        depth: u32,
        cases: u32,
        brackets: u32,
        spec: bool,
    },
    /// A JS block comment inside an interpolation, until `*/`.
    Comment,
}

impl Frame {
    /// The interpolation that opens at `rest` inside a string opened by `quote` (an f- or
    /// t-string when `fstring`), and the length of its opener.
    fn interpolation(
        language: Language,
        quote: u8,
        fstring: bool,
        rest: &[u8],
    ) -> Option<(Self, usize)> {
        let code = |open, close, len| {
            Some((
                Self::Code {
                    open,
                    close,
                    depth: 1,
                    cases: 0,
                    brackets: 0,
                    spec: false,
                },
                len,
            ))
        };
        match (language, quote) {
            (Language::Shell, b'"') if rest.starts_with(b"$(") => code(Some(b'('), b')', 2),
            (Language::Shell, b'"') if rest.first() == Some(&b'`') => code(None, b'`', 1),
            // A JS template's `${…}`, and a shell parameter expansion, which nests its own
            // quotes (`"${x:-"'"}"`) and so is scanned the same way.
            (Language::JavaScript, b'`') | (Language::Shell, b'"') if rest.starts_with(b"${") => {
                code(Some(b'{'), b'}', 2)
            }
            (Language::Python, _) if fstring && rest.first() == Some(&b'{') => {
                code(Some(b'{'), b'}', 1)
            }
            _ => None,
        }
    }
}

/// Whether the bytes just before a Python quote are an interpolating prefix: at most two
/// prefix letters, one of them `f` or `t` (a PEP 750 template string runs its `{…}` too), not
/// the end of an identifier (`elif"`).
fn is_interpolating_prefix(before: &[u8]) -> bool {
    let start = before
        .iter()
        .rposition(|b| !b"fFtTrRbBuU".contains(b))
        .map_or(0, |p| p + 1);
    let prefix = &before[start..];
    prefix.len() <= 2
        && prefix
            .iter()
            .any(|b| matches!(b, b'f' | b'F' | b't' | b'T'))
        && (start == 0 || !is_ident(before[start - 1]))
}

/// Whether the shell word `word` starts at byte `i` of `bytes`: a whole shell word, so `$case`,
/// `--ignore-case`, `esac=1` and `esac-tool` are not it.
fn shell_word_at(bytes: &[u8], i: usize, word: &[u8]) -> bool {
    let breaks_before = |b: u8| b.is_ascii_whitespace() || b";&|({`!)".contains(&b);
    let breaks_after = |b: &u8| b.is_ascii_whitespace() || b";&|)".contains(b);
    bytes[i..].starts_with(word)
        && (i == 0 || breaks_before(bytes[i - 1]))
        && bytes.get(i + word.len()).is_none_or(breaks_after)
}

/// Whether `before` (trimmed) ends with the whole word `word`.
fn ends_with_word(before: &[u8], word: &[u8]) -> bool {
    before.ends_with(word)
        && before
            .len()
            .checked_sub(word.len() + 1)
            .is_none_or(|k| !is_ident(before[k]))
}

/// Whether a shell word at byte `i` is in command position, where `case` starts a statement:
/// first on the line or in its substitution, after `;`, `|`, `&`, `(`, a group's `{` (not the
/// `{` of `${`), a backtick or `!`, after a reserved word that starts a command list, or, while
/// a `case` is open (`in_case`), after a pattern's `)`.
fn in_command_position(bytes: &[u8], i: usize, in_case: bool) -> bool {
    let before = bytes[..i].trim_ascii_end();
    match before.last() {
        None | Some(b';' | b'|' | b'&' | b'(' | b'`' | b'!') => true,
        Some(b'{') => before.len() < 2 || before[before.len() - 2] != b'$',
        Some(b')') => in_case,
        Some(_) => SHELL_LIST_WORDS
            .iter()
            .any(|word| ends_with_word(before, word)),
    }
}

/// The shell's reserved words after which a command list starts ([`in_command_position`]);
/// with `case` and `esac`, the words the shell scanner looks for.
pub(crate) const SHELL_LIST_WORDS: [&[u8]; 8] = [
    b"then", b"do", b"else", b"if", b"elif", b"while", b"until", b"time",
];

/// Whether a shell `(` at byte `i` opens a case pattern (`case x in (a) …`) rather than a
/// subshell: it follows the `in` of a case or the `;;` or `;&` that ends an arm.
fn opens_case_pattern(bytes: &[u8], i: usize) -> bool {
    let before = bytes[..i].trim_ascii_end();
    before.ends_with(b";;") || before.ends_with(b";&") || ends_with_word(before, b"in")
}

/// JS words after which an expression starts, so a `/` there opens a regex and a quote right
/// after one opens a string (`return"x"`, `typeof'y'`).
pub(crate) const JS_OPERAND_KEYWORDS: [&[u8]; 14] = [
    b"return",
    b"typeof",
    b"void",
    b"delete",
    b"in",
    b"of",
    b"instanceof",
    b"new",
    b"throw",
    b"case",
    b"do",
    b"else",
    b"yield",
    b"await",
];

/// The length of the longest of [`JS_OPERAND_KEYWORDS`] (`instanceof`).
const JS_OPERAND_KEYWORD_MAX: usize = {
    let (mut most, mut k) = (0, 0);
    while k < JS_OPERAND_KEYWORDS.len() {
        if JS_OPERAND_KEYWORDS[k].len() > most {
            most = JS_OPERAND_KEYWORDS[k].len();
        }
        k += 1;
    }
    most
};

/// The identifier that ends `before`, if any: the reading of release 0.8.3, which walked
/// back over the whole word. Kept for the tests, which hold the bounded reading to it.
#[cfg(test)]
fn trailing_word(before: &[u8]) -> &[u8] {
    let start = before
        .iter()
        .rposition(|b| !is_ident(*b))
        .map_or(0, |p| p + 1);
    &before[start..]
}

/// [`ends_with_operand_keyword`] as release 0.8.3 had it, word for word: the reference the
/// tests hold the bounded reading to, and what the scanner asks inside
/// [`released_word_walk::run`].
#[cfg(test)]
fn released_ends_with_operand_keyword(before: &[u8]) -> bool {
    let word = trailing_word(before);
    JS_OPERAND_KEYWORDS.contains(&word) && !before[..before.len() - word.len()].ends_with(b".")
}

/// Tests only: the scanner with the released keyword reading, so a test can scan the same
/// text both ways and compare what the scanner and the rules make of it.
#[cfg(test)]
pub(crate) mod released_word_walk {
    use std::cell::Cell;

    thread_local! {
        /// `Some(calls)` while [`run`] runs on this thread.
        static CALLS: Cell<Option<u64>> = const { Cell::new(None) };
    }

    /// Whether the scanner on this thread asks the released reading (counted).
    pub(super) fn on() -> bool {
        CALLS.with(|calls| calls.get().map(|n| calls.set(Some(n + 1))).is_some())
    }

    /// `work`, with every keyword question of the scanner on this thread answered by the
    /// released reading; and how many questions it answered.
    pub(crate) fn run<T>(work: impl FnOnce() -> T) -> (T, u64) {
        let outer = CALLS.replace(Some(0));
        let out = work();
        let calls = CALLS.replace(outer).unwrap_or(0);
        (out, calls)
    }

    /// `count` JavaScript lines from `seed` (the same lines on every run): words of up to
    /// `longest` bytes, words that are, end in or hold a keyword the scanner knows (with up to
    /// two more word bytes than the longest keyword before it, after `.`, `$`, a space or a
    /// byte past ASCII), before and after quotes, template literals and their `${…}`, regexes,
    /// divisions, comments and the tokens of the JavaScript rules, joined with no space
    /// between them as often as with one.
    pub(crate) fn hostile_lines(seed: u64, count: usize, longest: usize) -> Vec<String> {
        const MARKS: [&str; 22] = [
            "\"", "'", "`", "/", "/x/g", "${", "}", "/*", "*/", "//", ".", " ", "(", ")", "=", "+",
            "\\", "$", "é", "!", ";", ",",
        ];
        const TOKENS: [&str; 8] = [
            "eval(x)",
            "fetch(u)",
            "execSync(c)",
            "new Function(s)",
            "require('child_process')",
            "it(",
            "catch {}",
            "// eslint-disable",
        ];
        const WORD_BYTES: &[u8] = b"aZ_09q";
        let mut seed = seed | 1;
        let mut next = move |below: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            usize::try_from(seed % (below as u64)).unwrap()
        };
        let keywords = super::JS_OPERAND_KEYWORDS;
        let mut lines = Vec::with_capacity(count);
        for _ in 0..count {
            let mut line = String::new();
            for _ in 0..=next(8) {
                let keyword = std::str::from_utf8(keywords[next(keywords.len())]).unwrap();
                let word = |len: usize, next: &mut dyn FnMut(usize) -> usize| -> String {
                    (0..len)
                        .map(|_| char::from(WORD_BYTES[next(WORD_BYTES.len())]))
                        .collect()
                };
                match next(6) {
                    0 => line.push_str(keyword),
                    1 => {
                        let pad = next(super::JS_OPERAND_KEYWORD_MAX + 3);
                        line.push_str(&word(pad, &mut next));
                        line.push_str(keyword);
                    }
                    2 => {
                        let len = 1 + next(longest);
                        line.push_str(&word(len, &mut next));
                    }
                    3 => line.push_str(TOKENS[next(TOKENS.len())]),
                    _ => line.push_str(MARKS[next(MARKS.len())]),
                }
                if next(2) == 0 {
                    line.push(' ');
                }
            }
            lines.push(line);
        }
        lines
    }
}

/// Whether `before` ends with a [`JS_OPERAND_KEYWORDS`] word that is not a property name
/// (`x.return`).
///
/// Only the last [`JS_OPERAND_KEYWORD_MAX`] + 1 bytes are read: a word longer than every
/// keyword is none of them, whatever its other bytes. The scanner asks this at every byte
/// after a word byte, and until 0.9.0 the whole word was walked back each time, so a line of
/// one word cost the square of its length (FINDING-5: 200,000 letters took 18 s to rank in
/// v0.8.3). The answer is the released one for every `before` (the tests compare the two
/// readings, and the views and evidence they give).
fn ends_with_operand_keyword(before: &[u8]) -> bool {
    #[cfg(test)]
    if released_word_walk::on() {
        return released_ends_with_operand_keyword(before);
    }
    let len = before
        .iter()
        .rev()
        .take(JS_OPERAND_KEYWORD_MAX + 1)
        .take_while(|b| is_ident(**b))
        .count();
    let (rest, word) = before.split_at(before.len() - len);
    len <= JS_OPERAND_KEYWORD_MAX && JS_OPERAND_KEYWORDS.contains(&word) && !rest.ends_with(b".")
}

/// Whether a JS `/` here starts a regex literal rather than a division, as in the language's
/// grammar: the last code byte before it is an operator or an opening bracket (or there is
/// none), or the last word is one after which an expression starts. `a++ /` and `a-- /` are
/// divisions.
fn starts_regex(out: &[u8], previous_line: Option<u8>) -> bool {
    let carried;
    let mut before = out.trim_ascii_end();
    if before.is_empty()
        && let Some(last) = previous_line
    {
        carried = [last];
        before = &carried;
    }
    match before {
        [.., b'+', b'+'] | [.., b'-', b'-'] => false,
        // Nothing before it, or an arrow, whose body can start with a regex.
        [] | [.., b'=', b'>'] => true,
        // A TypeScript non-null assertion (`x! /`, `f()! /`) ends an operand.
        [.., operand, b'!'] if is_ident(*operand) || matches!(operand, b')' | b']') => false,
        // Not `<` or `>`: `</p>` and `<p>/` are JSX tags far more often than `a < /re/`.
        [.., last] if b"(,=:[!&|?{};+-*%~^".contains(last) => true,
        _ => ends_with_operand_keyword(before),
    }
}

/// The number of `#` of a Rust raw string whose opening quote follows `before` (`r"`, `r#"`,
/// `br##"`, the raw C string `cr#"`), or `None` when the quote opens an ordinary string.
fn rust_raw_hashes(before: &[u8]) -> Option<usize> {
    let hashes = before.iter().rev().take_while(|b| **b == b'#').count();
    let prefix = &before[..before.len() - hashes];
    let rest = prefix.strip_suffix(b"r")?;
    // At most one of `b` and `c`: Rust has no `bcr` or `cbr` prefix.
    let rest = rest
        .strip_suffix(b"b")
        .or_else(|| rest.strip_suffix(b"c"))
        .unwrap_or(rest);
    rest.last().is_none_or(|b| !is_ident(*b)).then_some(hashes)
}

/// The length of a JS regex literal at `rest` (which starts with its `/`), up to and including
/// its closing `/`, or `None` when the line holds no closer.
fn regex_len(rest: &[u8]) -> Option<usize> {
    let mut class = false;
    let mut escaped = false;
    rest.iter().enumerate().skip(1).find_map(|(j, &b)| {
        if escaped {
            escaped = false;
            return None;
        }
        match b {
            b'\\' => escaped = true,
            b'[' => class = true,
            b']' => class = false,
            b'/' if !class => return Some(j + 1),
            _ => {}
        }
        None
    })
}

/// The code bytes of `line`: comments and string contents replaced by spaces, the quotes kept,
/// so token positions stay where they were. Code inside an interpolation is scanned as code,
/// with its own strings, escapes and comments, so a closer inside a nested string or after a
/// backslash does not end it. When a desync is possible, the scan errs toward code: a string
/// still open at the end of the line (outside Rust, whose strings span lines) is read as code
/// from its quote on, so a quote the scanner misread cannot hide what follows it. `state`
/// carries an open block comment, triple string or string to the next line. A line that may
/// start inside a carried string is read both as code and as resuming the string, each reading
/// goes on to the next line, and a byte any reading keeps is kept: the closing quote of a
/// multi-line string does not open a phantom string over the code after it. The plain reading,
/// which resumes no string, always goes on, so a quote misread as an opener cannot hide a later
/// line of the hunk that a scan carrying no string would keep.
#[must_use]
pub fn code_view(language: Language, line: &str, state: &mut ScanState) -> String {
    let mut readings = Vec::with_capacity(2 * (1 + MORE_READINGS));
    let mut views = Vec::with_capacity(readings.capacity());
    for mut start in state.readings() {
        let open = start.in_string.take();
        let mut plain = start;
        views.push(scan_line(language, line, &mut plain, None));
        readings.push(plain);
        if let Some(open) = open {
            let mut resumed = start;
            views.push(scan_line(language, line, &mut resumed, Some(open)));
            readings.push(resumed);
        }
    }
    *state = ScanState::from_readings(&readings);
    union_views(line, &views)
}

/// One line's view, starting inside `open` when the previous line left that string open.
fn scan_line(language: Language, line: &str, state: &mut Reading, open: Option<Frame>) -> String {
    let bytes = line.as_bytes();
    let mut scan = Scan {
        language,
        bytes,
        out: Vec::with_capacity(bytes.len()),
        stack: open.into_iter().collect(),
        blank: 0,
        copy: 0,
        no_regex: false,
    };
    for i in 0..bytes.len() {
        if !scan.step(i, state) {
            break;
        }
    }
    scan.finish(state)
}

/// The bytes of `line` that any of `views` (views of it) keeps, the rest blank. Each view keeps
/// or blanks a whole character, so the union is valid UTF-8 wherever every view is.
fn union_views(line: &str, views: &[String]) -> String {
    let kept = |i: usize| {
        views
            .iter()
            .any(|view| view.as_bytes().get(i).is_some_and(|c| *c != b' '))
    };
    let len = views.iter().map(String::len).max().unwrap_or(0);
    let out: Vec<u8> = line
        .bytes()
        .take(len)
        .enumerate()
        .map(|(i, byte)| if kept(i) { byte } else { b' ' })
        .collect();
    String::from_utf8(out).unwrap_or_default()
}

/// [`union_views`] of views given one at a time: which bytes of the line some view so far
/// keeps, one bit for each, so that a view can be dropped as soon as it is made and no two
/// views of a line are alive together.
#[derive(Debug, Default)]
struct Kept {
    bits: Vec<u64>,
    len: usize,
}

impl Kept {
    /// Adds `view`, a view of the line.
    fn add(&mut self, view: &str) {
        self.len = self.len.max(view.len());
        self.bits
            .resize(self.len.div_ceil(64).max(self.bits.len()), 0);
        for (at, byte) in view.bytes().enumerate() {
            if byte != b' ' {
                self.bits[at / 64] |= 1 << (at % 64);
            }
        }
    }

    /// The bytes of `line` that any view added keeps, the rest blank: [`union_views`] of them.
    fn view(self, line: &str) -> String {
        let out: Vec<u8> = line
            .bytes()
            .take(self.len)
            .enumerate()
            .map(|(at, byte)| {
                if self.bits[at / 64] & (1 << (at % 64)) == 0 {
                    b' '
                } else {
                    byte
                }
            })
            .collect();
        String::from_utf8(out).unwrap_or_default()
    }
}

/// [`code_view`], with each reading's view dropped before the next is made and the line
/// scanned once for readings that begin the same: the same view and the same state, at the
/// cost in memory of one view however many readings `state` holds. With the view, how many
/// times the line was scanned (at most twice for each reading `state` holds, as in
/// [`code_view`], which scans exactly twice for each reading that carries a string and once
/// for each that does not).
/// The reading carried from one hunk of a created file to the next is read this way, since it
/// can come to a line with several readings where a hunk read alone comes with one.
fn code_view_one_at_a_time(
    language: Language,
    line: &str,
    state: &mut ScanState,
) -> (String, usize) {
    let mut readings = Vec::with_capacity(2 * (1 + MORE_READINGS));
    let mut kept = Kept::default();
    // Where each scan as code so far began, with what it ended in: readings that differ only
    // in the string they carry begin the same as code, and the line is scanned once for them.
    let mut as_code: Vec<(Reading, Reading)> = Vec::with_capacity(1 + MORE_READINGS);
    let mut scans = 0;
    for mut start in state.readings() {
        let open = start.in_string.take();
        let plain = if let Some(&(_, end)) = as_code.iter().find(|(began, _)| *began == start) {
            end
        } else {
            let mut plain = start;
            kept.add(&scan_line(language, line, &mut plain, None));
            scans += 1;
            as_code.push((start, plain));
            plain
        };
        readings.push(plain);
        if let Some(open) = open {
            let mut resumed = start;
            kept.add(&scan_line(language, line, &mut resumed, Some(open)));
            scans += 1;
            readings.push(resumed);
        }
    }
    *state = ScanState::from_readings(&readings);
    (kept.view(line), scans)
}

/// One line's scan in progress: the view so far, what is open, and how many of the next bytes
/// a multi-byte decision (`/*`, `*/`, `"""`, a char literal, an opener, a regex) blanks or then
/// copies. One byte per step, so no step can stall.
struct Scan<'a> {
    language: Language,
    bytes: &'a [u8],
    out: Vec<u8>,
    stack: Vec<Frame>,
    blank: usize,
    copy: usize,
    /// A regex probe found no closer: the rest of the line has none either way, so later `/`
    /// are read as division (code) without probing again, which keeps a line linear.
    no_regex: bool,
}

impl Scan<'_> {
    /// Scan byte `i`; `false` when the rest of the line is a comment.
    fn step(&mut self, i: usize, state: &mut Reading) -> bool {
        let byte = self.bytes[i];
        if self.blank > 0 {
            self.blank -= 1;
            self.out.push(b' ');
            return true;
        }
        if self.copy > 0 {
            self.copy -= 1;
            self.out.push(byte);
            return true;
        }
        let rest = &self.bytes[i..];
        if state.in_block_comment {
            if rest.starts_with(b"*/") {
                state.in_block_comment = false;
                self.blank = 1;
            }
            self.out.push(b' ');
            return true;
        }
        if let Some(triple) = state.in_triple {
            if rest.starts_with(&[triple; 3]) {
                state.in_triple = None;
                self.copy = 2;
                self.out.push(byte);
            } else {
                self.out.push(b' ');
            }
            return true;
        }
        match self.stack.last().copied() {
            Some(Frame::Comment) => {
                if rest.starts_with(b"*/") {
                    self.stack.pop();
                    self.blank = 1;
                }
                self.out.push(b' ');
                true
            }
            Some(Frame::Code { spec: true, .. }) => {
                // A format spec is literal text in which only `{…}` runs.
                if let Some((frame, _)) = Frame::interpolation(self.language, b'"', true, rest) {
                    self.stack.push(frame);
                    self.out.push(byte);
                } else if byte == b'}' {
                    self.stack.pop();
                    self.out.push(byte);
                } else {
                    self.out.push(b' ');
                }
                true
            }
            Some(str @ Frame::Str { .. }) => {
                self.in_string(str, rest);
                true
            }
            Some(Frame::Code { .. }) | None => self.in_code(i, state),
        }
    }

    /// A byte inside the innermost open string, `frame`.
    fn in_string(&mut self, frame: Frame, rest: &[u8]) {
        let Frame::Str {
            quote,
            start,
            triple,
            fstring,
            ansi,
            raw,
            escaped,
        } = frame
        else {
            return;
        };
        let byte = rest[0];
        let top = self.stack.len() - 1;
        let with_escape = |escaped| Frame::Str {
            quote,
            start,
            triple,
            fstring,
            ansi,
            raw,
            escaped,
        };
        let hashes = raw.unwrap_or(0);
        let closes = byte == quote
            && (!triple || rest.starts_with(&[quote; 3]))
            && rest
                .get(1..=hashes)
                .is_some_and(|tail| tail.iter().all(|b| *b == b'#'));
        if escaped {
            self.stack[top] = with_escape(false);
        } else if closes {
            self.stack.pop();
            self.copy = if triple { 2 } else { hashes };
            self.out.push(byte);
            return;
        } else if fstring && rest.starts_with(b"{{") {
            // A doubled brace is a literal brace, not an interpolation.
            self.blank = 1;
        } else if let Some((code, len)) = Frame::interpolation(self.language, quote, fstring, rest)
        {
            self.stack.push(code);
            self.copy = len - 1;
            self.out.push(byte);
            return;
        } else if byte == b'\\'
            && raw.is_none()
            && !(self.language == Language::Shell && quote == b'\'' && !ansi)
            && !(fstring && rest.get(1) == Some(&b'{'))
        {
            // A backslash escapes the next byte, except in a plain single-quoted shell string
            // and before an f-string's `{`, which Python still runs.
            self.stack[top] = with_escape(true);
        }
        // A YAML quoted scalar is code (a `run:` line); the frame only keeps `#` from a comment.
        self.out.push(if self.language == Language::Yaml {
            byte
        } else {
            b' '
        });
    }

    /// A byte of code: at the top of the line or inside an interpolation. `false` when the rest
    /// of the line is a comment.
    fn in_code(&mut self, i: usize, state: &mut Reading) -> bool {
        let (language, bytes) = (self.language, self.bytes);
        let (byte, rest) = (bytes[i], &bytes[i..]);
        let nested = !self.stack.is_empty();
        if byte == b'\\' && (nested || matches!(language, Language::Shell | Language::JavaScript)) {
            // An escaped byte is never a closer, a quote or the start of a comment.
            self.copy = 1;
            self.out.push(byte);
            return true;
        }
        if language.c_like() && rest.starts_with(b"//") {
            return false;
        }
        if language.c_like() && rest.starts_with(b"/*") {
            if nested {
                self.stack.push(Frame::Comment);
            } else {
                state.in_block_comment = true;
            }
            self.blank = 1;
            self.out.push(b' ');
            return true;
        }
        if language == Language::JavaScript
            && byte == b'/'
            && !self.no_regex
            && starts_regex(&self.out, state.js_last)
        {
            if let Some(len) = regex_len(rest) {
                // A regex literal's text is data: its quotes and brackets open nothing.
                self.blank = len - 2;
                self.copy = 1;
                self.out.push(byte);
                return true;
            }
            self.no_regex = true;
        }
        if !nested && byte == b'#' && is_comment_start(language, bytes, i) {
            return false;
        }
        if language == Language::Python
            && (byte == b'"' || byte == b'\'')
            && rest.starts_with(&[byte; 3])
        {
            if nested {
                self.stack.push(Frame::Str {
                    quote: byte,
                    start: i,
                    triple: true,
                    fstring: false,
                    ansi: false,
                    raw: None,
                    escaped: false,
                });
            } else {
                state.in_triple = Some(byte);
            }
            self.copy = 2;
            self.out.push(byte);
            return true;
        }
        // A Rust char literal (`'"'`, `'\''`) is not a string start; a lifetime (`'a`) has no
        // closing quote two or three bytes on, so it is left as code.
        if language == Language::Rust && byte == b'\'' {
            let close = if rest.get(1) == Some(&b'\\') { 3 } else { 2 };
            if rest.get(close) == Some(&b'\'') {
                self.blank = close;
                self.out.push(b' ');
                return true;
            }
        }
        // In JS a quote right after a word byte opens nothing: no string can start there, and
        // JSX text (`<p>Can't</p>`) puts apostrophes there. Nor in YAML, where a scalar's quote
        // comes first and `it's` is plain text.
        let after_word = i > 0
            && is_ident(bytes[i - 1])
            && match language {
                Language::JavaScript => byte != b'`' && !ends_with_operand_keyword(&bytes[..i]),
                Language::Yaml => true,
                _ => false,
            };
        if language.opens_string(byte) && !after_word {
            self.stack.push(Frame::Str {
                quote: byte,
                start: i,
                triple: false,
                fstring: language == Language::Python && is_interpolating_prefix(&bytes[..i]),
                ansi: language == Language::Shell && byte == b'\'' && i > 0 && bytes[i - 1] == b'$',
                raw: if language.raw_quote(byte) {
                    Some(0)
                } else {
                    (language == Language::Rust && byte == b'"')
                        .then(|| rust_raw_hashes(&bytes[..i]))
                        .flatten()
                },
                escaped: false,
            });
            self.out.push(byte);
            return true;
        }
        self.track_code(i);
        self.out.push(byte);
        true
    }

    /// The bookkeeping of the interpolation byte `i` is code in: nesting, shell `case`
    /// statements and a Python replacement field's brackets and format spec.
    fn track_code(&mut self, i: usize) {
        let (language, bytes) = (self.language, self.bytes);
        let byte = bytes[i];
        let Some(Frame::Code {
            open,
            close,
            depth,
            cases,
            brackets,
            spec,
        }) = self.stack.last_mut()
        else {
            return;
        };
        if language == Language::Shell {
            // `esac` after `|` is a pattern alternative, not the end of the case.
            let ends_case = || !bytes[..i].trim_ascii_end().ends_with(b"|");
            if shell_word_at(bytes, i, b"case") && in_command_position(bytes, i, *cases > 0) {
                *cases += 1;
            } else if shell_word_at(bytes, i, b"esac")
                // With no case open the decrement below is a no-op, so `esac` may be read as
                // if one were.
                && in_command_position(bytes, i, true)
                && ends_case()
            {
                *cases = cases.saturating_sub(1);
            }
        }
        let pattern_paren = language == Language::Shell
            && byte == b'('
            && *cases > 0
            && opens_case_pattern(bytes, i);
        if language == Language::Python {
            match byte {
                b'(' | b'[' => *brackets += 1,
                b')' | b']' => *brackets = brackets.saturating_sub(1),
                // A top-level `:` starts the format spec (a walrus needs parentheses here, so
                // `{x:=10}` is the spec `=10`).
                b':' if *depth == 1 && *brackets == 0 => {
                    *spec = true;
                }
                _ => {}
            }
        }
        if *open == Some(byte) && !pattern_paren {
            *depth += 1;
        } else if byte == *close && !(byte == b')' && *cases > 0) {
            // A `)` while a `case` is open ends a pattern, not the substitution.
            *depth -= 1;
            if *depth == 0 {
                self.stack.pop();
            }
        }
    }

    /// The view, with a string still open at the end of the line read as code from its quote.
    fn finish(mut self, state: &mut Reading) -> String {
        if self.language == Language::JavaScript
            && let Some(&last) = self.out.trim_ascii_end().last()
        {
            state.js_last = Some(last);
        }
        let open_string = self.stack.iter().find_map(|f| match f {
            Frame::Str { start, .. } => Some(*start),
            _ => None,
        });
        if let Some(start) = open_string
            && self.language != Language::Rust
        {
            self.out.truncate(start);
            self.out.extend_from_slice(&self.bytes[start..]);
        }
        // A string open at the top level goes on to the next line; one inside an interpolation
        // ends with the line, as the interpolation does.
        if let [
            Frame::Str {
                quote,
                triple,
                fstring,
                ansi,
                raw,
                ..
            },
        ] = self.stack[..]
        {
            state.in_string = Some(Frame::Str {
                quote,
                start: 0,
                triple,
                fstring,
                ansi,
                raw,
                escaped: false,
            });
        }
        // Only ASCII bytes are replaced, by ASCII, the scan stops only at an ASCII comment
        // start, and a restored tail starts at an ASCII quote, so the view is valid UTF-8
        // wherever the line was.
        String::from_utf8(self.out).unwrap_or_default()
    }
}

/// Whether a `#` at byte `i` starts a comment: always in Python, and in shell and YAML only at
/// the start of a word (`$#`, `${x#y}` and `a#b` are code).
fn is_comment_start(language: Language, bytes: &[u8], i: usize) -> bool {
    let word_start = i == 0 || bytes[i - 1].is_ascii_whitespace();
    match language {
        Language::Python => true,
        Language::Shell => word_start || b";&|(".contains(&bytes[i - 1]),
        Language::Yaml => word_start,
        Language::Rust | Language::JavaScript | Language::Go => false,
    }
}

/// An identifier byte (ASCII letters, digits, `_`).
#[must_use]
pub const fn is_ident(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// A byte that, just before a token, makes it part of something else: an identifier byte, `.`
/// (a method or field: `re.eval(`) or `$` (a shell or template name: `$eval`).
#[must_use]
pub const fn is_boundary_blocker(b: u8) -> bool {
    is_ident(b) || b == b'.' || b == b'$'
}

/// Whether `token` occurs in `text` with no [`is_boundary_blocker`] byte before it, and, when
/// the token ends in an identifier byte, no identifier byte after it.
#[must_use]
pub fn has_token(text: &str, token: &str) -> bool {
    let bytes = text.as_bytes();
    let ends_in_ident = token.as_bytes().last().copied().is_some_and(is_ident);
    text.match_indices(token).any(|(start, _)| {
        let before_ok = start == 0 || !is_boundary_blocker(bytes[start - 1]);
        let after_ok =
            !ends_in_ident || bytes.get(start + token.len()).is_none_or(|b| !is_ident(*b));
        before_ok && after_ok
    })
}

/// A YAML line's code view as YAML reads a key and its value: a quoted scalar is its content
/// (`contents: 'write'` and `"contents": "write"` read `contents: write`), and the blanks
/// around the `:` that ends a key are one space after it (`contents : write`). `None` when it
/// reads as the view does. The bytes of the line say it; nothing is parsed: a quote opens a
/// scalar at the line's start, after a blank, `[`, `{` or `,`, or right after a quoted key's
/// `:` (JSON: `{"contents":"write"}`), and the scalar is read when it closes on the line before
/// a byte that can end one and holds no escape (a `\`, or a doubled `''`: such a scalar is
/// left as written). One pass; no byte is looked at more than twice. Before 2026-11-22 a
/// privilege grant written with quotes was not read (S3.18).
#[must_use]
pub fn scalar_reading(view: &str) -> Option<String> {
    let bytes = view.as_bytes();
    let blank = |b: &u8| matches!(b, b' ' | b'\t');
    let mut out = Vec::with_capacity(bytes.len());
    let (mut i, mut after_key) = (0, false);
    while let Some(&byte) = bytes.get(i) {
        let begins = after_key || i == 0 || b" \t[{,".contains(&bytes[i - 1]);
        after_key = false;
        let rest = &bytes[i + 1..];
        if matches!(byte, b'\'' | b'"')
            && begins
            && let Some(len) = rest.iter().position(|b| *b == byte || *b == b'\\')
            && rest[len] == byte
            && rest.get(len + 1).is_none_or(|b| b" \t:,]}".contains(b))
        {
            out.extend_from_slice(&rest[..len]);
            i += len + 2;
            if bytes.get(i) == Some(&b':') {
                after_key = true;
                out.extend_from_slice(b": ");
                i += 1 + bytes[i + 1..].iter().take_while(|b| blank(b)).count();
            }
            continue;
        }
        let colon = i + bytes[i..].iter().take_while(|b| blank(b)).count();
        if bytes.get(colon) == Some(&b':') && bytes.get(colon + 1).is_none_or(blank) {
            let after = bytes[colon + 1..].iter().take_while(|b| blank(b)).count();
            out.extend_from_slice(if after > 0 { b": " } else { b":" });
            i = colon + 1 + after;
        } else if colon > i {
            out.extend_from_slice(&bytes[i..colon]);
            i = colon;
        } else {
            out.push(byte);
            i += 1;
        }
    }
    (out != bytes).then(|| String::from_utf8(out).unwrap_or_default())
}

/// Where one side of a Rust hunk starts: what [`rust_views`] is told about the lines above it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RustStart {
    /// In code, at line 1 of its file. A shebang line there is passed over as the compiler
    /// passes over it: it is no Rust, and leaves nothing open.
    FileStart,
    /// In code, below line 1.
    Code,
    /// Inside one block comment that the lines above opened. A side that begins inside two or
    /// more is read as inside one, and that reading is wrong from its first `*/` on: it takes
    /// what follows for code while the compiler is still in a comment. Mostly that shows text
    /// as code too early, but the text it shows can open a line comment or a string of its
    /// own and so hide code the compiler does read later (`// */ //` and on the next line
    /// `// */ call(); /*`, two comments deep: review V3 round 13, VD3R13-M-1, and the sixth
    /// independent review, IR6-L-1). No start is made for a greater depth: a declared limit.
    Comment,
    /// Inside an ordinary string (`"…"`, `b"…"`, `c"…"`) that the lines above opened.
    Str,
    /// Inside a raw string that the lines above opened with this many `#`: it takes no
    /// escapes and ends at the first quote that this many `#`, or more, follow.
    RawStr(usize),
}

/// One side of a Rust hunk, read whole from a start: [`rust_side`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RustSide {
    /// The code view of each line, in order.
    pub views: Vec<String>,
    /// The last line leaves a string or a raw string open. Read from code, that is the mark of
    /// a side that may have begun inside a string: its closing quote was taken for an opener,
    /// and so on to the end, each string's text read as code and the code between as strings.
    pub ends_in_string: bool,
    /// Some line of the side, the last or an earlier one, ends inside something: a string, a
    /// raw string, a block comment (or a character literal the lexer lets run on). The reading
    /// carries an open state over a line end.
    pub open_over_lines: bool,
}

/// The code view of every line of one side of a Rust hunk (`lines`, in order), read whole from
/// `start`: one exact reading, each line in the state the lines before it left. A string, a raw
/// string or a comment that runs over lines is carried as the compiler carries it.
///
/// A hunk does not say what the lines above it left open. So its side is read once for each
/// start it is given (from code, from inside a block comment, from inside a string), each a
/// whole view of its own, and the rules read every view: what any
/// of them finds is found, but no view is ever mixed with another, byte by byte. (Until
/// 2026-11-14 a line after an open string was read twice and the bytes of both readings were
/// united. After a string over a line end the two were out of step to the end of the hunk, and
/// the union showed the text of later strings and comments between the tokens of real code:
/// `x.unwrap/*""*/()` was no call any more. Fifth independent review, IR5-H-1.)
#[must_use]
pub fn rust_views(lines: &[&str], start: RustStart) -> Vec<String> {
    rust::side(lines, start).views
}

/// [`rust_views`], and what the reading says of what it carries: whether the side ends inside a
/// string, and whether anything is open over a line end.
#[must_use]
pub fn rust_side(lines: &[&str], start: RustStart) -> RustSide {
    rust::side(lines, start)
}

/// The code view of every line of one side of a Rust hunk as release 0.8.3 read it: the
/// scanner of that release (the module `legacy_rust`), from a fresh state, line after line. It
/// is one of the readings of a Rust hunk, so that nothing the released version reports is lost
/// to the scanner that follows the compiler's lexer where a hunk's start makes the two differ.
#[must_use]
pub fn rust_legacy_views(lines: &[&str]) -> Vec<String> {
    let mut state = legacy_rust::ScanState::default();
    lines
        .iter()
        .map(|line| legacy_rust::code_view(Language::Rust, line, &mut state))
        .collect()
}

/// Whether the first of `lines`, were it line 1 of a file, is a shebang by the compiler's rule
/// (so that a reading from [`RustStart::FileStart`] differs from one from [`RustStart::Code`]).
#[must_use]
pub fn rust_shebang(lines: &[&str]) -> bool {
    rust::is_shebang(lines)
}

/// The length in bytes of the white space character `rest` starts with, or 0 when it starts
/// with none. These are the characters the Rust compiler takes for white space, the six ASCII
/// ones and U+0085, U+200E, U+200F, U+2028 and U+2029. The one definition: the reader of
/// tokens separates tokens by it, a raw identifier is told by it, and so is a shebang.
pub(crate) const fn rust_white(rest: &[u8]) -> usize {
    match rest {
        [b'\t' | b'\n' | 0x0B | 0x0C | b'\r' | b' ', ..] => 1,
        [0xC2, 0x85, ..] => 2,
        [0xE2, 0x80, 0x8E | 0x8F | 0xA8 | 0xA9, ..] => 3,
        _ => 0,
    }
}

mod legacy_rust;

/// What the lines of one side of a hunk have left open, in each reading of them: the state of
/// the scan of a line. The readings share nothing; each goes on from what it left open itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Readings {
    /// The shared scanner's state. For Rust this is the reading of 0.8.3.
    legacy: ScanState,
    /// The state of the lexer reading. Only Rust has one.
    lexer: RustLexerState,
}

impl Readings {
    /// The view of `line` in the reading every release up to 0.8.3 had ([`code_view`]): for
    /// every language but Rust the only one, and for Rust the first, kept exactly, so that
    /// whatever 0.8.3 found on a line is found.
    ///
    /// With the view, how many times [`code_view`] scans the line for it: once for each
    /// reading its state holds and once more for each of them that carries a string, up to
    /// eight; it keeps one view for each scan until it has united them, as it always did.
    pub(crate) fn released_view(&mut self, language: Language, line: &str) -> (String, usize) {
        let scans = self
            .legacy
            .readings()
            .map(|reading| 1 + usize::from(reading.in_string.is_some()))
            .sum();
        (code_view(language, line, &mut self.legacy), scans)
    }

    /// The view of a Rust `line` in its second reading ([`rust_lexer_view`]), the compiler's
    /// reading of text that starts in code, which finds what the first hides after a nested
    /// block comment or a character literal with a long escape. The two views are never
    /// merged, so a token must stand whole in one reading.
    ///
    /// With the view, how many times the line was scanned for it: at most five.
    pub(crate) fn lexer_view(&mut self, line: &str) -> (String, usize) {
        rust::code_view_counted(line, &mut self.lexer)
    }

    /// What these readings have left open for the reading that goes from one hunk of a
    /// created file to the next ([`carried_view`]): the lexer's state for Rust, the shared
    /// scanner's for every other language.
    pub(crate) fn carried(&self, language: Language) -> Carried {
        if language == Language::Rust {
            if self.lexer == RustLexerState::default() {
                Carried::Nothing
            } else {
                Carried::Rust(self.lexer)
            }
        } else if self.legacy == ScanState::default() {
            Carried::Nothing
        } else {
            Carried::Shared(self.legacy)
        }
    }
}

/// The views of `line` together, one for each reading of it, the same once where both give
/// the same view: for the tests that compare the readings.
#[cfg(test)]
#[must_use]
pub(crate) fn code_views(language: Language, line: &str, state: &mut Readings) -> Vec<String> {
    let (legacy, _) = state.released_view(language, line);
    if language != Language::Rust {
        return vec![legacy];
    }
    let (lexer, _) = state.lexer_view(line);
    if lexer == legacy {
        vec![legacy]
    } else {
        vec![legacy, lexer]
    }
}

/// What the earlier hunks of a file the patch creates have left open for a later one: the
/// state of [`carried_view`]. For JavaScript that includes the last code byte of the line
/// before, which decides whether a `/` begins a regex, so there a later hunk nearly always
/// comes into something. It holds the state of the one scanner its language
/// has, and nothing at all when nothing is open, which is how most hunks are entered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Carried {
    /// Nothing is open: the next line starts as code.
    #[default]
    Nothing,
    /// Rust: the state of the lexer reading.
    Rust(RustLexerState),
    /// Every other language: the state of the shared scanner.
    Shared(ScanState),
}

/// One more reading of `line`: the one that goes on from what the hunk before left open, for
/// the later hunks of a file the patch creates. For Rust it is the
/// lexer reading ([`rust_lexer_view`]), which for a created file is the compiler's reading of
/// the whole file; for every other language it is [`code_view`]. It is asked beside the
/// readings that start the hunk as code, never in their place. With the view, how many times
/// the line was scanned for it: at most five for Rust and eight for another language (twice
/// for each of up to four readings), and once where one thing is open and no string.
#[must_use]
pub(crate) fn carried_view(language: Language, line: &str, state: &mut Carried) -> (String, usize) {
    if language == Language::Rust {
        let mut lexer = match *state {
            Carried::Rust(lexer) => lexer,
            _ => RustLexerState::default(),
        };
        let counted = rust::code_view_counted(line, &mut lexer);
        *state = if lexer == RustLexerState::default() {
            Carried::Nothing
        } else {
            Carried::Rust(lexer)
        };
        counted
    } else {
        let mut shared = match *state {
            Carried::Shared(shared) => shared,
            _ => ScanState::default(),
        };
        let counted = code_view_one_at_a_time(language, line, &mut shared);
        *state = if shared == ScanState::default() {
            Carried::Nothing
        } else {
            Carried::Shared(shared)
        };
        counted
    }
}

/// Not part of the API, and may change (the fuzz crate uses it): the state of
/// [`rust_lexer_view`].
#[doc(hidden)]
pub use rust::State as RustLexerState;

/// The code bytes of one Rust line as the compiler's lexer reads it (the `rust` module of this
/// file), and in `state` what the line leaves open for the next. Comments and the text of
/// literals are blank; the view is as long as the line, or ends where a line comment starts.
/// It is one of the two readings of Rust; the other is [`code_view`].
///
/// Not part of the API, and may change: it is public for the fuzz targets that hold it to the
/// compiler's lexer.
#[doc(hidden)]
#[must_use]
pub fn rust_lexer_view(line: &str, state: &mut RustLexerState) -> String {
    rust::code_view(line, state)
}

/// Rust, read as the compiler's lexer reads it.
///
/// The rules are those of `rustc_lexer` (the lexer the compiler and rust-analyzer share), taken
/// function by function from its source: each token has the extent that lexer gives it, so the
/// view and the compiler agree where a comment or a literal ends, and what stands after it is
/// read as the compiler reads it. Blank in the view are comments (a line comment ends the
/// view), the text of a string, byte string, C string or raw string after its opening quote
/// (up to its closing quote, or to the end of the line when it does not close there), and a
/// character or byte literal, quotes included, when it can be one: one character between its
/// quotes, or text that starts with a backslash. Everything else is kept. That includes what
/// the lexer also takes for a character literal and the compiler never accepts: several
/// characters or none between the quotes (`'ab'`, `''`), one the lexer calls unterminated, and
/// one that runs over a line end. Such a token is passed over whole, so that what follows it
/// is read as the lexer reads it, and kept as text: a hunk can start inside a string or a
/// comment, where an apostrophe is no quote, and then nothing between two of them is lost.
///
/// Where the compiler's reading depends on the edition, the view takes that of editions 2021
/// and 2024, as rustc 1.99.1 reads them. Three forms read otherwise before 2021, and there the
/// view can take the end of a string for another place than the compiler does:
/// - `c"…"` and `cr"…"` are C strings. Before 2021 `c"…"` is the name `c` and a string, with
///   the same bytes between the quotes; `cr"\"` is the name `cr` and a string in which `\"` is
///   an escape, where the raw reading ends the string at the second quote and so keeps more as
///   code.
/// - `'r#name` is one raw lifetime, so in `'r#r"\"` the quote opens an ordinary string. Before
///   2021 it is `'r`, `#` and what follows, here the raw string `r"\"`. (The rules also read
///   Rust as the shared scanner reads it, which is how 0.8.3 read it and has the older reading
///   of this form: `code_view`.)
/// - A name with `#` or a quote right after it (`k#fn`, `x"…"`) is the name and then the rest,
///   which is how editions before 2021 read it; 2021 and 2024 refuse an unknown prefix, so
///   no text they accept is read otherwise. (The lexer's newer `k#name` token is in no edition
///   of this compiler.)
///
/// A `#` before a quote is punctuation and the quote opens a string, as in every edition in
/// which `#"…"#` compiles; edition 2024 reserves that form and refuses it.
///
/// One line is scanned at a time. What a line leaves open is an `Open`: block comments and
/// their depth, a string, a raw string and its count of `#`, or a character literal (the lexer
/// lets one run over a line end in two ways, both errors later). It is read in two ways:
///
/// - Line by line, as [`super::rust_lexer_view`]. A comment is carried by every reading. A
///   literal is what a hunk can be wrong about (it may start inside a string), so the next
///   line is read both ways, as code and as resuming the literal, like every language here.
///   The exact reading, which always resumes, is kept apart from the others (`State::exact`)
///   and is never dropped: for text that starts in code it is the compiler's reading, and the
///   view keeps at least what it keeps.
/// - A side of a hunk whole, in one reading, as [`super::rust_side`]: the next line goes on
///   from what the line before left open, as the compiler does. What is not known is how a
///   hunk's first line starts, and that is said from outside ([`RustStart`]).
mod rust {
    use super::{MORE_READINGS, RustSide, RustStart, rust_white};

    /// What a line left open for the next one.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub(super) enum Open {
        /// Nothing: the next line starts in code.
        #[default]
        Nothing,
        /// Block comments, nested this deep (one or more).
        Comment(usize),
        /// A `"…"`, `b"…"` or `c"…"` string.
        Str,
        /// A raw string, which closes at a quote followed by this many `#`.
        RawStr(usize),
        /// A character or byte literal whose text ran over the line end after a backslash: the
        /// next line goes on as its text.
        CharText,
        /// A character or byte literal that reached the line end: the lexer goes on with it
        /// only if the next line starts with its closing quote, and ends it at the line end
        /// otherwise.
        CharEnd,
    }

    impl Open {
        /// Whether a literal is open, which the next line is read both as resuming and as not
        /// resuming. A comment, like nothing, is carried as it is.
        pub(super) const fn is_literal(self) -> bool {
            !matches!(self, Self::Nothing | Self::Comment(_))
        }
    }

    /// What the lines read so far have left open, in each reading of them: the state of the
    /// lexer reading ([`super::rust_lexer_view`]). It shares nothing with the state of the
    /// shared scanner ([`super::ScanState`]).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct State {
        /// The plain reading, which resumes no literal a line left open. It is never dropped.
        pub(super) plain: Open,
        /// Up to [`MORE_READINGS`] other readings, distinct from `plain` and from each other.
        pub(super) more: [Option<Open>; MORE_READINGS],
        /// Which of [`Self::readings`] (by its place there, 0 or 1) is the exact reading, the
        /// one that resumed every literal a line left open since the scan began. For text
        /// that starts in code it is the compiler's reading, so it is never dropped either.
        pub(super) exact: usize,
    }

    impl State {
        /// The readings, the plain one first.
        pub(super) fn readings(&self) -> impl Iterator<Item = Open> + '_ {
            std::iter::once(self.plain).chain(self.more.iter().flatten().copied())
        }
    }

    /// The view of `line` and the state after it: [`super::rust_lexer_view`].
    pub(super) fn code_view(line: &str, state: &mut State) -> String {
        code_view_counted(line, state).0
    }

    /// [`code_view`], and how many times the line was scanned for it: once for each place a
    /// reading begins from, the same place once, so at most five times (four readings that
    /// each resume a literal, and the one scan that resumes none).
    pub(super) fn code_view_counted(line: &str, state: &mut State) -> (String, usize) {
        // Each reading's view is added to what is kept and dropped before the next is made.
        let mut kept = super::Kept::default();
        let mut ends = Vec::with_capacity(2 * (1 + MORE_READINGS));
        let mut exact = None;
        // Where each reading so far began, with what it ended in: a second reading from the
        // same place is the same scan, and the line is not read again for it. (Every reading
        // that carries a literal is also read as resuming none, each from the same place.)
        let mut scanned: Vec<(Open, Open)> = Vec::with_capacity(1 + MORE_READINGS);
        let mut resumed = 0;
        for (place, open) in state.readings().enumerate() {
            // As code, or in the comment every reading carries: resuming no literal.
            let from = if open.is_literal() {
                Open::Nothing
            } else {
                open
            };
            let mut end = if let Some(&(_, end)) = scanned.iter().find(|(began, _)| *began == from)
            {
                end
            } else {
                let (view, end) = scan(line, from);
                kept.add(&view);
                scanned.push((from, end));
                end
            };
            ends.push(end);
            if open.is_literal() {
                let view;
                (view, end) = scan(line, open);
                kept.add(&view);
                resumed += 1;
                ends.push(end);
            }
            if place == state.exact {
                exact = Some(end);
            }
        }
        *state = after(&ends, exact);
        (kept.view(line), scanned.len() + resumed)
    }

    /// The state after a line whose readings ended as `ends` (the plain one first): the plain
    /// reading, then the exact one when it differs, then the others without repeats, as many as
    /// there is room for.
    pub(super) fn after(ends: &[Open], exact: Option<Open>) -> State {
        let plain = ends.first().copied().unwrap_or_default();
        let exact = exact.unwrap_or(plain);
        let mut kept = Vec::with_capacity(1 + MORE_READINGS);
        kept.push(plain);
        if exact != plain {
            kept.push(exact);
        }
        for end in ends {
            if kept.len() > MORE_READINGS {
                break;
            }
            if !kept.contains(end) {
                kept.push(*end);
            }
        }
        let mut state = State {
            plain,
            more: [None; MORE_READINGS],
            exact: usize::from(exact != plain),
        };
        for (slot, open) in state.more.iter_mut().zip(&kept[1..]) {
            *slot = Some(*open);
        }
        state
    }

    /// `lines`, one side of a hunk, read whole from `start`: `rust_side`.
    pub(super) fn side(lines: &[&str], start: RustStart) -> RustSide {
        let shebang = start == RustStart::FileStart && is_shebang(lines);
        let mut open = match start {
            RustStart::Comment => Open::Comment(1),
            RustStart::Str => Open::Str,
            RustStart::RawStr(hashes) => Open::RawStr(hashes),
            RustStart::FileStart | RustStart::Code => Open::Nothing,
        };
        let mut open_over_lines = false;
        let views = lines
            .iter()
            .enumerate()
            .map(|(number, line)| {
                if shebang && number == 0 {
                    return " ".repeat(line.len());
                }
                let (view, end) = scan(line, open);
                open = end;
                open_over_lines |= open != Open::Nothing;
                view
            })
            .collect();
        RustSide {
            views,
            ends_in_string: matches!(open, Open::Str | Open::RawStr(_)),
            open_over_lines,
        }
    }

    /// Whether the first of `lines`, line 1 of a file, is a shebang by the compiler's rule
    /// (`rustc_lexer::strip_shebang`; a byte order mark before it is dropped first). It starts
    /// with `#!`, and the first token after that, on this line or a later one, which is
    /// neither white space nor a comment (a doc comment is a token here) is not `[`: with a
    /// `[` it is the start of an inner attribute, `#![…]`, and Rust. When the lines end before
    /// any such token it is a shebang, as it is in a file that ends there; for a hunk that
    /// shows only the top of its file this is the reading that shows more, since all the
    /// other one has to show is white space and comments.
    ///
    /// Checked with rustc 1.99.1: `#!… /*` on line 1 hides nothing below it; `#!` with `[` on
    /// the next line, after `// plain` or after a block comment over two lines, is an
    /// attribute; `#!/** doc */ [` is a shebang; a shebang is line 1 only.
    pub(super) fn is_shebang(lines: &[&str]) -> bool {
        let Some(first) = lines.first() else {
            return false;
        };
        let Some(after) = first
            .strip_prefix('\u{feff}')
            .unwrap_or(first)
            .strip_prefix("#!")
        else {
            return false;
        };
        let mut depth = 0;
        std::iter::once(after)
            .chain(lines[1..].iter().copied())
            .find_map(|line| first_token(line, &mut depth))
            .is_none_or(|bracket| !bracket)
    }

    /// The first token of `line` that is neither white space nor a comment other than a doc
    /// comment: whether it is `[`, or `None` when the line holds no such token. The line
    /// starts inside `depth` block comments (none when 0), and `depth` is how many it ends in.
    fn first_token(line: &str, depth: &mut usize) -> Option<bool> {
        let mut scan = Scan {
            line,
            bytes: line.as_bytes(),
            out: line.as_bytes().to_vec(),
        };
        let inside = std::mem::take(depth);
        // Where the line goes on after a comment that began at `start` and is read on from
        // `from` with `open` of them open; `None`, and the depth kept, when it does not end.
        let mut past =
            |start: usize, from: usize, open: usize| match scan.comment(start, from, open) {
                Next::At(end) => Some(end),
                Next::End(Open::Comment(still)) => {
                    *depth = still;
                    None
                }
                Next::End(_) => None,
            };
        let bytes = line.as_bytes();
        let mut at = if inside == 0 { 0 } else { past(0, 0, inside)? };
        loop {
            let rest = &bytes[at..];
            let space = rust_white(rest);
            if space > 0 {
                at += space;
                continue;
            }
            return match rest {
                [] => None,
                // A line comment: a token when it is a doc comment, `//!` or `///` (but not
                // `////`), and else passed over with the rest of the line.
                [b'/', b'/', after @ ..] => {
                    let doc = match after {
                        [b'!', ..] => true,
                        [b'/', then @ ..] => then.first() != Some(&b'/'),
                        _ => false,
                    };
                    doc.then_some(false)
                }
                // A block comment: a token when it is a doc comment, `/*!` or `/**` (but not
                // `/***` or `/**/`), and else passed over to where it ends.
                [b'/', b'*', after @ ..] => {
                    let doc = match after {
                        [b'!', ..] => true,
                        [b'*', then @ ..] => !matches!(then.first(), Some(b'*' | b'/')),
                        _ => false,
                    };
                    if doc {
                        return Some(false);
                    }
                    at = past(at, at + 2, 1)?;
                    continue;
                }
                [first, ..] => Some(*first == b'['),
            };
        }
    }

    /// One reading of `line`, which starts with `start` open: its view, and what it leaves open.
    fn scan(line: &str, start: Open) -> (String, Open) {
        let mut scan = Scan {
            line,
            bytes: line.as_bytes(),
            out: line.as_bytes().to_vec(),
        };
        let mut next = match start {
            Open::Nothing => Next::At(0),
            Open::Comment(depth) => scan.comment(0, 0, depth),
            Open::Str => scan.string_text(0),
            Open::RawStr(hashes) => scan.raw_text(0, hashes),
            Open::CharText => scan.char_text(None, 0),
            // With the quote that closes it first on the line, the literal ends there, and a
            // suffix may follow as after any literal. Without, it ended with its line.
            Open::CharEnd => Next::At(if scan.bytes.first() == Some(&b'\'') {
                scan.identifier(1)
            } else {
                0
            }),
        };
        let end = loop {
            match next {
                Next::At(at) if at < scan.bytes.len() => next = scan.token(at),
                Next::At(_) => break Open::Nothing,
                Next::End(open) => break open,
            }
        };
        // Whole characters are blanked, by ASCII spaces, and the view is cut only before an
        // ASCII `/`: it is UTF-8 as the line is.
        (String::from_utf8(scan.out).unwrap_or_default(), end)
    }

    /// Where a scan goes on after a token.
    enum Next {
        /// At this byte of the line, in code (the end of the line when it is the length).
        At(usize),
        /// Nowhere: the line ended, and left this open.
        End(Open),
    }

    /// One reading of one line in progress.
    struct Scan<'a> {
        line: &'a str,
        bytes: &'a [u8],
        /// The view: the line, less what is blanked or cut off so far.
        out: Vec<u8>,
    }

    impl Scan<'_> {
        /// The character at byte `at` (a character boundary), if the line goes that far.
        fn char_at(&self, at: usize) -> Option<char> {
            self.line.get(at..)?.chars().next()
        }

        /// Blank the bytes from `from` up to `to`.
        fn blank(&mut self, from: usize, to: usize) {
            self.out[from..to].fill(b' ');
        }

        /// The token at byte `at` of the line (the lexer's `advance_token`): where the next one
        /// starts, or what the line leaves open.
        fn token(&mut self, at: usize) -> Next {
            let next = self.bytes.get(at + 1).copied();
            let names = |scan: &Self| scan.char_at(at + 2).is_some_and(id_start);
            match self.bytes[at] {
                b'/' if next == Some(b'/') => {
                    self.out.truncate(at);
                    Next::End(Open::Nothing)
                }
                b'/' if next == Some(b'*') => self.comment(at, at + 2, 1),
                // A raw identifier (`r#fn`): `#` and a name. (No other letter does this. The
                // lexer has a newer token `k#name`, but in no edition of rustc 1.99.1: before
                // 2021 `k#r"\\"` is `k`, `#` and a raw string, and 2021 and 2024 refuse it.)
                b'r' if next == Some(b'#') && names(self) => Next::At(self.identifier(at + 2)),
                b'r' if matches!(next, Some(b'#' | b'"')) => self.raw_string(at),
                b'b' if next == Some(b'\'') => self.quoted(at + 1),
                b'b' | b'c' if next == Some(b'"') => self.string_text(at + 2),
                b'b' | b'c'
                    if next == Some(b'r')
                        && matches!(self.bytes.get(at + 2), Some(b'"' | b'#')) =>
                {
                    self.raw_string(at + 1)
                }
                b'0'..=b'9' => Next::At(self.identifier(self.number(at))),
                b'\'' => self.lifetime_or_char(at),
                b'"' => self.string_text(at + 1),
                b'_' | b'a'..=b'z' | b'A'..=b'Z' => Next::At(self.name_rest(at + 1)),
                byte if byte.is_ascii() => Next::At(at + 1),
                _ => {
                    let Some(c) = self.char_at(at) else {
                        return Next::At(self.bytes.len());
                    };
                    let after = at + c.len_utf8();
                    Next::At(if id_start(c) {
                        self.name_rest(after)
                    } else if emoji(c) {
                        self.emoji_name(after)
                    } else {
                        after
                    })
                }
            }
        }

        /// A block comment that began at byte `start`, read on from `at` with `depth` of them
        /// open (the lexer's `block_comment`): `/*` opens another, `*/` closes one, and the
        /// comment ends when none is open.
        fn comment(&mut self, start: usize, mut at: usize, mut depth: usize) -> Next {
            let bytes = self.bytes;
            while at < bytes.len() {
                match (bytes[at], bytes.get(at + 1)) {
                    (b'/', Some(b'*')) => {
                        depth += 1;
                        at += 2;
                    }
                    (b'*', Some(b'/')) => {
                        depth -= 1;
                        at += 2;
                        if depth == 0 {
                            self.blank(start, at);
                            return Next::At(at);
                        }
                    }
                    _ => at += 1,
                }
            }
            self.blank(start, bytes.len());
            Next::End(Open::Comment(depth))
        }

        /// The text of a string from byte `from`, just after its opening quote or at the start
        /// of a line that resumes it (the lexer's `double_quoted_string`): it closes at a quote,
        /// and a backslash takes a quote or a backslash after it with it. A suffix follows a
        /// closed literal (`eat_literal_suffix`).
        fn string_text(&mut self, from: usize) -> Next {
            let bytes = self.bytes;
            let mut at = from;
            while let Some(&byte) = bytes.get(at) {
                match byte {
                    b'"' => {
                        self.blank(from, at);
                        return Next::At(self.identifier(at + 1));
                    }
                    b'\\' if matches!(bytes.get(at + 1), Some(b'\\' | b'"')) => at += 2,
                    _ => at += 1,
                }
            }
            self.blank(from, bytes.len());
            Next::End(Open::Str)
        }

        /// A raw string whose `r` is at byte `r` (the lexer's `raw_string_unvalidated`): any
        /// number of `#`, then a quote. With anything else there it is no string, and the lexer
        /// takes that one character into the token with the `r` and the `#`.
        fn raw_string(&mut self, r: usize) -> Next {
            let hashes = self.bytes[r + 1..]
                .iter()
                .take_while(|byte| **byte == b'#')
                .count();
            let quote = r + 1 + hashes;
            if self.bytes.get(quote) == Some(&b'"') {
                self.raw_text(quote + 1, hashes)
            } else {
                Next::At(quote + self.char_at(quote).map_or(0, char::len_utf8))
            }
        }

        /// The text of a raw string from byte `from`: it closes at a quote followed by `hashes`
        /// of `#`, and a quote with fewer is text. The lexer takes a suffix after it unless it
        /// has more than 255 `#` (an error it reports later).
        fn raw_text(&mut self, from: usize, hashes: usize) -> Next {
            let bytes = self.bytes;
            let mut at = from;
            while let Some(found) = bytes[at..].iter().position(|byte| *byte == b'"') {
                let quote = at + found;
                let closing = bytes[quote + 1..]
                    .iter()
                    .take(hashes)
                    .take_while(|byte| **byte == b'#')
                    .count();
                at = quote + 1 + closing;
                if closing == hashes {
                    self.blank(from, quote);
                    return Next::At(if hashes > 255 {
                        at
                    } else {
                        self.identifier(at)
                    });
                }
            }
            self.blank(from, bytes.len());
            Next::End(Open::RawStr(hashes))
        }

        /// A `'` at byte `quote` (the lexer's `lifetime_or_char`). It opens a lifetime or a
        /// label when a name, a digit or an emoji follows and no quote stands right after that
        /// one character; then the name is passed over. A quote right after the name makes the
        /// lexer take it for a literal after all (`'ab'`), which the compiler refuses: it ends
        /// there, and is kept as text. Anything else is a character literal.
        fn lifetime_or_char(&mut self, quote: usize) -> Next {
            let Some(first) = self.char_at(quote + 1) else {
                return Next::End(Open::CharEnd);
            };
            let after = quote + 1 + first.len_utf8();
            let second = self.bytes.get(after).copied();
            let lifetime = second != Some(b'\'')
                && (id_start(first) || first.is_ascii_digit() || emoji(first));
            if !lifetime {
                return self.quoted(quote);
            }
            if first == 'r' && second == Some(b'#') && self.char_at(after + 1).is_some_and(id_start)
            {
                // A raw lifetime (`'r#fn`): no quote after it is looked for.
                return Next::At(self.name_run(after + 1));
            }
            let end = self.name_rest(after);
            // No suffix is taken after a literal read this way.
            Next::At(end + usize::from(self.bytes.get(end) == Some(&b'\'')))
        }

        /// A character or byte literal whose opening quote is at byte `quote` (the lexer's
        /// `single_quoted_string`): one character that is no backslash and the closing quote,
        /// or else text up to the next quote. It is blank when it is the one character, or
        /// text that starts with a backslash (an escape); other text between two quotes is no
        /// literal the compiler accepts, and is kept.
        fn quoted(&mut self, quote: usize) -> Next {
            if let Some(only) = self.char_at(quote + 1)
                && only != '\\'
            {
                let close = quote + 1 + only.len_utf8();
                if self.bytes.get(close) == Some(&b'\'') {
                    self.blank(quote, close + 1);
                    return Next::At(self.identifier(close + 1));
                }
            }
            let escape = self.bytes.get(quote + 1) == Some(&b'\\');
            self.char_text(escape.then_some(quote), quote + 1)
        }

        /// The text of a character or byte literal from byte `at`. It closes at a quote, and
        /// then the literal is blanked from `blank_from`, if that is given. The lexer ends it
        /// unclosed before a `/` and at the end of a line, unless the next line starts with the
        /// closing quote; a backslash takes the next character with it, also a line end.
        fn char_text(&mut self, blank_from: Option<usize>, mut at: usize) -> Next {
            loop {
                match self.bytes.get(at) {
                    None => return Next::End(Open::CharEnd),
                    Some(b'\'') => {
                        if let Some(quote) = blank_from {
                            self.blank(quote, at + 1);
                        }
                        return Next::At(self.identifier(at + 1));
                    }
                    Some(b'/') => return Next::At(at),
                    Some(b'\\') if at + 1 == self.bytes.len() => {
                        return Next::End(Open::CharText);
                    }
                    // The byte after a backslash is passed over: if it starts a longer
                    // character, the rest of that are no bytes this loop stops at.
                    Some(b'\\') => at += 2,
                    Some(_) => at += 1,
                }
            }
        }

        /// The end of the number that starts at byte `at`, before its suffix (the lexer's
        /// `number`). Where it ends decides what a letter after it is: `1e+r"\"` is one number
        /// with the suffix `r` and then an ordinary string, and in `0x1e+r"\"` the number ends
        /// before the `+` and a raw string follows.
        fn number(&self, at: usize) -> usize {
            let bytes = self.bytes;
            let mut end = at + 1;
            if bytes[at] == b'0' {
                match bytes.get(end) {
                    Some(base @ (b'b' | b'o' | b'x')) => {
                        let any;
                        (end, any) = self.digits(end + 1, *base == b'x');
                        if !any {
                            return end;
                        }
                    }
                    Some(b'0'..=b'9' | b'_') => end = self.digits(end, false).0,
                    Some(b'.' | b'e' | b'E') => {}
                    _ => return end,
                }
            } else {
                end = self.digits(end, false).0;
            }
            match bytes.get(end) {
                // Not the dot of a range (`0..2`) or of a field or method (`1.max(2)`).
                Some(b'.')
                    if bytes.get(end + 1) != Some(&b'.')
                        && !self.char_at(end + 1).is_some_and(id_start) =>
                {
                    end += 1;
                    if bytes.get(end).is_some_and(u8::is_ascii_digit) {
                        end = self.digits(end, false).0;
                        if matches!(bytes.get(end), Some(b'e' | b'E')) {
                            end = self.exponent(end + 1);
                        }
                    }
                    end
                }
                Some(b'e' | b'E') => self.exponent(end + 1),
                _ => end,
            }
        }

        /// The end of an exponent from byte `at`, just after its `e`: a sign, then digits.
        fn exponent(&self, at: usize) -> usize {
            let signed = matches!(self.bytes.get(at), Some(b'-' | b'+'));
            self.digits(at + usize::from(signed), false).0
        }

        /// The end of the digits and `_` from byte `at`, and whether a digit was among them.
        fn digits(&self, mut at: usize, hex: bool) -> (usize, bool) {
            let mut any = false;
            while let Some(byte) = self.bytes.get(at) {
                if byte.is_ascii_digit() || (hex && byte.is_ascii_hexdigit()) {
                    any = true;
                } else if *byte != b'_' {
                    break;
                }
                at += 1;
            }
            (at, any)
        }

        /// The end of the name characters from byte `at`.
        fn name_run(&self, mut at: usize) -> usize {
            while let Some(c) = self.char_at(at)
                && id_continue(c)
            {
                at += c.len_utf8();
            }
            at
        }

        /// The end of a name that starts at byte `at`, or `at` when none does (the lexer's
        /// `eat_identifier`): a literal's suffix, and the name of a raw identifier.
        fn identifier(&self, at: usize) -> usize {
            match self.char_at(at) {
                Some(c) if id_start(c) => self.name_run(at + c.len_utf8()),
                _ => at,
            }
        }

        /// The end of a name whose first character ends at byte `at` (the lexer's
        /// `ident_or_unknown_prefix`): its name characters, and when an emoji follows them, the
        /// emoji and name characters after that too. The lexer makes one token of those, which
        /// the compiler then refuses, so a letter among them is no prefix of a literal.
        fn name_rest(&self, at: usize) -> usize {
            let at = self.name_run(at);
            if self.char_at(at).is_some_and(emoji) {
                self.emoji_name(at)
            } else {
                at
            }
        }

        /// The end of the emoji and name characters from byte `at` (the lexer's
        /// `invalid_ident`).
        fn emoji_name(&self, mut at: usize) -> usize {
            while let Some(c) = self.char_at(at)
                && (id_continue(c) || emoji(c))
            {
                at += c.len_utf8();
            }
            at
        }
    }

    /// Whether a name can start with `c`: `_` or `XID_Start`.
    pub(super) fn id_start(c: char) -> bool {
        c == '_' || c.is_ascii_alphabetic() || (!c.is_ascii() && unicode::xid_start(c))
    }

    /// Whether a name can go on with `c`: `XID_Continue`.
    pub(super) fn id_continue(c: char) -> bool {
        c == '_'
            || c.is_ascii_alphanumeric()
            || (!c.is_ascii() && (unicode::xid_start(c) || unicode::xid_continue_only(c)))
    }

    /// Whether `c` is an emoji past ASCII, which the lexer takes into a name it then calls
    /// invalid.
    pub(super) fn emoji(c: char) -> bool {
        !c.is_ascii() && unicode::emoji(c)
    }

    /// Whether each table is ascending ranges past ASCII that do not meet: what the search in
    /// them needs.
    #[cfg(test)]
    pub(super) fn tables_ascend() -> bool {
        unicode::ascend()
    }

    /// The characters past ASCII a name may hold, and the emoji: three properties of Unicode
    /// 18.0.0, the version of the lexer this scanner is held to (`ra-ap-rustc_lexer` 0.184.0
    /// with `unicode-ident` 1.0.27 and `unicode-properties` 0.1.5). The tables were printed
    /// from those crates' own answers for every character, and the fuzz target `rust_code_view`
    /// compares the view with the lexer for every character again (its `every_character`
    /// seed), so a table that falls behind the lexer fails there. A compiler with an older
    /// Unicode refuses a newer character outside comments and literals, so on text it compiles
    /// the two agree.
    mod unicode {
        use std::cmp::Ordering;

        /// Whether `c` is in one of the ranges of `table` (first and last character of each,
        /// ascending).
        fn within(table: &[(u32, u32)], c: char) -> bool {
            let c = u32::from(c);
            table
                .binary_search_by(|&(first, last)| {
                    if last < c {
                        Ordering::Less
                    } else if first > c {
                        Ordering::Greater
                    } else {
                        Ordering::Equal
                    }
                })
                .is_ok()
        }

        pub(super) fn xid_start(c: char) -> bool {
            within(XID_START, c)
        }

        pub(super) fn xid_continue_only(c: char) -> bool {
            within(XID_CONTINUE_ONLY, c)
        }

        pub(super) fn emoji(c: char) -> bool {
            within(EMOJI, c)
        }

        #[cfg(test)]
        pub(super) fn ascend() -> bool {
            [XID_START, XID_CONTINUE_ONLY, EMOJI].iter().all(|table| {
                table
                    .iter()
                    .all(|(first, last)| *first > 0x7F && first <= last)
                    && table.windows(2).all(|pair| pair[0].1 < pair[1].0)
            })
        }

        /// `XID_Start` past ASCII.
        #[rustfmt::skip]
        const XID_START: &[(u32, u32)] = &[
            (0x00AA, 0x00AA), (0x00B5, 0x00B5), (0x00BA, 0x00BA), (0x00C0, 0x00D6),
            (0x00D8, 0x00F6), (0x00F8, 0x02C1), (0x02C6, 0x02D1), (0x02E0, 0x02E4),
            (0x02EC, 0x02EC), (0x02EE, 0x02EE), (0x0370, 0x0374), (0x0376, 0x0377),
            (0x037B, 0x037D), (0x037F, 0x037F), (0x0386, 0x0386), (0x0388, 0x038A),
            (0x038C, 0x038C), (0x038E, 0x03A1), (0x03A3, 0x03F5), (0x03F7, 0x0481),
            (0x048A, 0x052F), (0x0531, 0x0556), (0x0558, 0x0559), (0x0560, 0x0588),
            (0x058B, 0x058C), (0x05D0, 0x05EA), (0x05EF, 0x05F2), (0x0620, 0x064A),
            (0x066E, 0x066F), (0x0671, 0x06D3), (0x06D5, 0x06D5), (0x06E5, 0x06E6),
            (0x06EE, 0x06EF), (0x06FA, 0x06FC), (0x06FF, 0x06FF), (0x0710, 0x0710),
            (0x0712, 0x072F), (0x074D, 0x07A5), (0x07B1, 0x07B1), (0x07CA, 0x07EA),
            (0x07F4, 0x07F5), (0x07FA, 0x07FA), (0x0800, 0x0815), (0x081A, 0x081A),
            (0x0824, 0x0824), (0x0828, 0x0828), (0x0840, 0x0858), (0x0860, 0x086A),
            (0x0870, 0x0887), (0x0889, 0x088F), (0x08A0, 0x08C9), (0x0904, 0x0939),
            (0x093D, 0x093D), (0x0950, 0x0950), (0x0958, 0x0961), (0x0971, 0x0980),
            (0x0985, 0x098C), (0x098F, 0x0990), (0x0993, 0x09A8), (0x09AA, 0x09B0),
            (0x09B2, 0x09B2), (0x09B6, 0x09B9), (0x09BD, 0x09BD), (0x09CE, 0x09CE),
            (0x09DC, 0x09DD), (0x09DF, 0x09E1), (0x09F0, 0x09F1), (0x09FC, 0x09FC),
            (0x0A05, 0x0A0A), (0x0A0F, 0x0A10), (0x0A13, 0x0A28), (0x0A2A, 0x0A30),
            (0x0A32, 0x0A33), (0x0A35, 0x0A36), (0x0A38, 0x0A39), (0x0A59, 0x0A5C),
            (0x0A5E, 0x0A5E), (0x0A72, 0x0A74), (0x0A85, 0x0A8D), (0x0A8F, 0x0A91),
            (0x0A93, 0x0AA8), (0x0AAA, 0x0AB0), (0x0AB2, 0x0AB3), (0x0AB5, 0x0AB9),
            (0x0ABD, 0x0ABD), (0x0AD0, 0x0AD0), (0x0AE0, 0x0AE1), (0x0AF9, 0x0AF9),
            (0x0B05, 0x0B0C), (0x0B0F, 0x0B10), (0x0B13, 0x0B28), (0x0B2A, 0x0B30),
            (0x0B32, 0x0B33), (0x0B35, 0x0B39), (0x0B3D, 0x0B3D), (0x0B5C, 0x0B5D),
            (0x0B5F, 0x0B61), (0x0B71, 0x0B71), (0x0B83, 0x0B83), (0x0B85, 0x0B8A),
            (0x0B8E, 0x0B90), (0x0B92, 0x0B95), (0x0B99, 0x0B9A), (0x0B9C, 0x0B9C),
            (0x0B9E, 0x0B9F), (0x0BA3, 0x0BA4), (0x0BA8, 0x0BAA), (0x0BAE, 0x0BB9),
            (0x0BD0, 0x0BD0), (0x0C05, 0x0C0C), (0x0C0E, 0x0C10), (0x0C12, 0x0C28),
            (0x0C2A, 0x0C39), (0x0C3D, 0x0C3D), (0x0C58, 0x0C5A), (0x0C5C, 0x0C5D),
            (0x0C60, 0x0C61), (0x0C80, 0x0C80), (0x0C85, 0x0C8C), (0x0C8E, 0x0C90),
            (0x0C92, 0x0CA8), (0x0CAA, 0x0CB3), (0x0CB5, 0x0CB9), (0x0CBD, 0x0CBD),
            (0x0CDC, 0x0CDE), (0x0CE0, 0x0CE1), (0x0CF1, 0x0CF2), (0x0D04, 0x0D0C),
            (0x0D0E, 0x0D10), (0x0D12, 0x0D3A), (0x0D3D, 0x0D3D), (0x0D4E, 0x0D4E),
            (0x0D54, 0x0D56), (0x0D5F, 0x0D61), (0x0D7A, 0x0D7F), (0x0D85, 0x0D96),
            (0x0D9A, 0x0DB1), (0x0DB3, 0x0DBB), (0x0DBD, 0x0DBD), (0x0DC0, 0x0DC6),
            (0x0E01, 0x0E30), (0x0E32, 0x0E32), (0x0E40, 0x0E46), (0x0E81, 0x0E82),
            (0x0E84, 0x0E84), (0x0E86, 0x0E8A), (0x0E8C, 0x0EA3), (0x0EA5, 0x0EA5),
            (0x0EA7, 0x0EB0), (0x0EB2, 0x0EB2), (0x0EBD, 0x0EBD), (0x0EC0, 0x0EC4),
            (0x0EC6, 0x0EC6), (0x0EDC, 0x0EDF), (0x0F00, 0x0F00), (0x0F40, 0x0F47),
            (0x0F49, 0x0F6C), (0x0F88, 0x0F8C), (0x1000, 0x102A), (0x103F, 0x103F),
            (0x1050, 0x1055), (0x105A, 0x105D), (0x1061, 0x1061), (0x1065, 0x1066),
            (0x106E, 0x1070), (0x1075, 0x1081), (0x108E, 0x108E), (0x10A0, 0x10C5),
            (0x10C7, 0x10C7), (0x10CD, 0x10CD), (0x10D0, 0x10FA), (0x10FC, 0x1248),
            (0x124A, 0x124D), (0x1250, 0x1256), (0x1258, 0x1258), (0x125A, 0x125D),
            (0x1260, 0x1288), (0x128A, 0x128D), (0x1290, 0x12B0), (0x12B2, 0x12B5),
            (0x12B8, 0x12BE), (0x12C0, 0x12C0), (0x12C2, 0x12C5), (0x12C8, 0x12D6),
            (0x12D8, 0x1310), (0x1312, 0x1315), (0x1318, 0x135A), (0x1380, 0x138F),
            (0x13A0, 0x13F5), (0x13F8, 0x13FD), (0x1401, 0x166C), (0x166F, 0x167F),
            (0x1681, 0x169A), (0x16A0, 0x16EA), (0x16EE, 0x16F8), (0x1700, 0x1711),
            (0x171F, 0x1731), (0x1740, 0x1751), (0x1760, 0x176C), (0x176E, 0x1770),
            (0x1780, 0x17B3), (0x17D7, 0x17D7), (0x17DC, 0x17DC), (0x1820, 0x1878),
            (0x1880, 0x18A8), (0x18AA, 0x18AA), (0x18B0, 0x18F5), (0x1900, 0x191E),
            (0x1950, 0x196D), (0x1970, 0x1974), (0x1980, 0x19AB), (0x19B0, 0x19C9),
            (0x1A00, 0x1A16), (0x1A20, 0x1A54), (0x1AA7, 0x1AA7), (0x1B05, 0x1B33),
            (0x1B45, 0x1B4C), (0x1B83, 0x1BA0), (0x1BAE, 0x1BAF), (0x1BBA, 0x1BE5),
            (0x1C00, 0x1C23), (0x1C4D, 0x1C4F), (0x1C5A, 0x1C7D), (0x1C80, 0x1C8A),
            (0x1C90, 0x1CBA), (0x1CBD, 0x1CBF), (0x1CE9, 0x1CEC), (0x1CEE, 0x1CF3),
            (0x1CF5, 0x1CF6), (0x1CFA, 0x1CFA), (0x1D00, 0x1DBF), (0x1E00, 0x1F15),
            (0x1F18, 0x1F1D), (0x1F20, 0x1F45), (0x1F48, 0x1F4D), (0x1F50, 0x1F57),
            (0x1F59, 0x1F59), (0x1F5B, 0x1F5B), (0x1F5D, 0x1F5D), (0x1F5F, 0x1F7D),
            (0x1F80, 0x1FB4), (0x1FB6, 0x1FBC), (0x1FBE, 0x1FBE), (0x1FC2, 0x1FC4),
            (0x1FC6, 0x1FCC), (0x1FD0, 0x1FD3), (0x1FD6, 0x1FDB), (0x1FE0, 0x1FEC),
            (0x1FF2, 0x1FF4), (0x1FF6, 0x1FFC), (0x2071, 0x2071), (0x207F, 0x207F),
            (0x208F, 0x209F), (0x2102, 0x2102), (0x2107, 0x2107), (0x210A, 0x2113),
            (0x2115, 0x2115), (0x2118, 0x211D), (0x2124, 0x2124), (0x2126, 0x2126),
            (0x2128, 0x2128), (0x212A, 0x2139), (0x213C, 0x213F), (0x2145, 0x2149),
            (0x214E, 0x214E), (0x2160, 0x2188), (0x2C00, 0x2CE4), (0x2CEB, 0x2CEE),
            (0x2CF2, 0x2CF3), (0x2D00, 0x2D25), (0x2D27, 0x2D27), (0x2D2D, 0x2D2D),
            (0x2D30, 0x2D67), (0x2D6F, 0x2D6F), (0x2D80, 0x2D96), (0x2DA0, 0x2DA6),
            (0x2DA8, 0x2DAE), (0x2DB0, 0x2DB6), (0x2DB8, 0x2DBE), (0x2DC0, 0x2DC6),
            (0x2DC8, 0x2DCE), (0x2DD0, 0x2DD6), (0x2DD8, 0x2DDE), (0x3005, 0x3007),
            (0x3021, 0x3029), (0x3031, 0x3035), (0x3038, 0x303C), (0x3041, 0x3096),
            (0x309D, 0x309F), (0x30A1, 0x30FA), (0x30FC, 0x30FF), (0x3105, 0x312F),
            (0x3131, 0x318E), (0x31A0, 0x31BF), (0x31F0, 0x31FF), (0x3400, 0x4DBF),
            (0x4E00, 0xA48C), (0xA4D0, 0xA4FD), (0xA500, 0xA60C), (0xA610, 0xA61F),
            (0xA62A, 0xA62B), (0xA640, 0xA66E), (0xA67F, 0xA69D), (0xA6A0, 0xA6EF),
            (0xA717, 0xA71F), (0xA722, 0xA788), (0xA78B, 0xA7DD), (0xA7E2, 0xA7E2),
            (0xA7F1, 0xA801), (0xA803, 0xA805), (0xA807, 0xA80A), (0xA80C, 0xA822),
            (0xA840, 0xA873), (0xA882, 0xA8B3), (0xA8F2, 0xA8F7), (0xA8FB, 0xA8FB),
            (0xA8FD, 0xA8FE), (0xA90A, 0xA925), (0xA930, 0xA946), (0xA960, 0xA97C),
            (0xA984, 0xA9B2), (0xA9CF, 0xA9CF), (0xA9E0, 0xA9E4), (0xA9E6, 0xA9EF),
            (0xA9FA, 0xA9FE), (0xAA00, 0xAA28), (0xAA40, 0xAA42), (0xAA44, 0xAA4B),
            (0xAA60, 0xAA76), (0xAA7A, 0xAA7A), (0xAA7E, 0xAAAF), (0xAAB1, 0xAAB1),
            (0xAAB5, 0xAAB6), (0xAAB9, 0xAABD), (0xAAC0, 0xAAC0), (0xAAC2, 0xAAC2),
            (0xAADB, 0xAADD), (0xAAE0, 0xAAEA), (0xAAF2, 0xAAF4), (0xAB01, 0xAB06),
            (0xAB09, 0xAB0E), (0xAB11, 0xAB16), (0xAB20, 0xAB26), (0xAB28, 0xAB2E),
            (0xAB30, 0xAB5A), (0xAB5C, 0xAB69), (0xAB6C, 0xAB6D), (0xAB70, 0xABE2),
            (0xAC00, 0xD7A3), (0xD7B0, 0xD7C6), (0xD7CB, 0xD7FB), (0xF900, 0xFA6D),
            (0xFA70, 0xFAD9), (0xFB00, 0xFB06), (0xFB13, 0xFB17), (0xFB1D, 0xFB1D),
            (0xFB1F, 0xFB28), (0xFB2A, 0xFB36), (0xFB38, 0xFB3C), (0xFB3E, 0xFB3E),
            (0xFB40, 0xFB41), (0xFB43, 0xFB44), (0xFB46, 0xFBB1), (0xFBD3, 0xFC5D),
            (0xFC64, 0xFD3D), (0xFD50, 0xFD8F), (0xFD92, 0xFDC7), (0xFDF0, 0xFDF9),
            (0xFE71, 0xFE71), (0xFE73, 0xFE73), (0xFE77, 0xFE77), (0xFE79, 0xFE79),
            (0xFE7B, 0xFE7B), (0xFE7D, 0xFE7D), (0xFE7F, 0xFEFC), (0xFF21, 0xFF3A),
            (0xFF41, 0xFF5A), (0xFF66, 0xFF9D), (0xFFA0, 0xFFBE), (0xFFC2, 0xFFC7),
            (0xFFCA, 0xFFCF), (0xFFD2, 0xFFD7), (0xFFDA, 0xFFDC), (0x1_0000, 0x1_000B),
            (0x1_000D, 0x1_0026), (0x1_0028, 0x1_003A), (0x1_003C, 0x1_003D), (0x1_003F, 0x1_004D),
            (0x1_0050, 0x1_005D), (0x1_0080, 0x1_00FA), (0x1_0140, 0x1_0174), (0x1_0280, 0x1_029C),
            (0x1_02A0, 0x1_02D0), (0x1_0300, 0x1_031F), (0x1_032D, 0x1_034A), (0x1_0350, 0x1_0375),
            (0x1_0380, 0x1_039D), (0x1_03A0, 0x1_03C3), (0x1_03C8, 0x1_03CF), (0x1_03D1, 0x1_03D5),
            (0x1_0400, 0x1_049D), (0x1_04B0, 0x1_04D3), (0x1_04D8, 0x1_04FB), (0x1_0500, 0x1_0527),
            (0x1_0530, 0x1_0563), (0x1_0570, 0x1_057A), (0x1_057C, 0x1_058A), (0x1_058C, 0x1_0592),
            (0x1_0594, 0x1_0595), (0x1_0597, 0x1_05A1), (0x1_05A3, 0x1_05B1), (0x1_05B3, 0x1_05B9),
            (0x1_05BB, 0x1_05BC), (0x1_05C0, 0x1_05F3), (0x1_0600, 0x1_0736), (0x1_0740, 0x1_0755),
            (0x1_0760, 0x1_0767), (0x1_0780, 0x1_0785), (0x1_0787, 0x1_07B0), (0x1_07B2, 0x1_07BF),
            (0x1_0800, 0x1_0805), (0x1_0808, 0x1_0808), (0x1_080A, 0x1_0835), (0x1_0837, 0x1_0838),
            (0x1_083C, 0x1_083C), (0x1_083F, 0x1_0855), (0x1_0860, 0x1_0876), (0x1_0880, 0x1_089E),
            (0x1_08E0, 0x1_08F2), (0x1_08F4, 0x1_08F5), (0x1_0900, 0x1_0915), (0x1_0920, 0x1_0939),
            (0x1_0940, 0x1_0959), (0x1_0980, 0x1_09B7), (0x1_09BE, 0x1_09BF), (0x1_0A00, 0x1_0A00),
            (0x1_0A10, 0x1_0A13), (0x1_0A15, 0x1_0A17), (0x1_0A19, 0x1_0A35), (0x1_0A60, 0x1_0A7C),
            (0x1_0A80, 0x1_0A9C), (0x1_0AC0, 0x1_0AC7), (0x1_0AC9, 0x1_0AE4), (0x1_0B00, 0x1_0B35),
            (0x1_0B40, 0x1_0B55), (0x1_0B60, 0x1_0B72), (0x1_0B80, 0x1_0B91), (0x1_0C00, 0x1_0C48),
            (0x1_0C80, 0x1_0CB2), (0x1_0CC0, 0x1_0CF2), (0x1_0D00, 0x1_0D23), (0x1_0D4A, 0x1_0D65),
            (0x1_0D6F, 0x1_0D85), (0x1_0E80, 0x1_0EA9), (0x1_0EB0, 0x1_0EB1), (0x1_0EC2, 0x1_0EC7),
            (0x1_0ED9, 0x1_0EEE), (0x1_0F00, 0x1_0F1C), (0x1_0F27, 0x1_0F27), (0x1_0F30, 0x1_0F45),
            (0x1_0F70, 0x1_0F81), (0x1_0FB0, 0x1_0FC4), (0x1_0FE0, 0x1_0FF6), (0x1_1003, 0x1_1037),
            (0x1_1071, 0x1_1072), (0x1_1075, 0x1_1075), (0x1_1083, 0x1_10AF), (0x1_10D0, 0x1_10E8),
            (0x1_1103, 0x1_1126), (0x1_1144, 0x1_1144), (0x1_1147, 0x1_1147), (0x1_1150, 0x1_1172),
            (0x1_1176, 0x1_1176), (0x1_1183, 0x1_11B2), (0x1_11C1, 0x1_11C4), (0x1_11DA, 0x1_11DA),
            (0x1_11DC, 0x1_11DC), (0x1_1200, 0x1_1211), (0x1_1213, 0x1_122B), (0x1_123F, 0x1_1240),
            (0x1_1280, 0x1_1286), (0x1_1288, 0x1_1288), (0x1_128A, 0x1_128D), (0x1_128F, 0x1_129D),
            (0x1_129F, 0x1_12A8), (0x1_12B0, 0x1_12DE), (0x1_1305, 0x1_130C), (0x1_130F, 0x1_1310),
            (0x1_1313, 0x1_1328), (0x1_132A, 0x1_1330), (0x1_1332, 0x1_1333), (0x1_1335, 0x1_1339),
            (0x1_133D, 0x1_133D), (0x1_1350, 0x1_1350), (0x1_135D, 0x1_1361), (0x1_1380, 0x1_1389),
            (0x1_138B, 0x1_138B), (0x1_138E, 0x1_138E), (0x1_1390, 0x1_13B5), (0x1_13B7, 0x1_13B7),
            (0x1_13D1, 0x1_13D1), (0x1_13D3, 0x1_13D3), (0x1_1400, 0x1_1434), (0x1_1447, 0x1_144A),
            (0x1_145F, 0x1_1461), (0x1_1480, 0x1_14AF), (0x1_14C4, 0x1_14C5), (0x1_14C7, 0x1_14C7),
            (0x1_1580, 0x1_15AE), (0x1_15D8, 0x1_15DB), (0x1_1600, 0x1_162F), (0x1_1644, 0x1_1644),
            (0x1_1680, 0x1_16AA), (0x1_16B8, 0x1_16B8), (0x1_1700, 0x1_171A), (0x1_1740, 0x1_1746),
            (0x1_1800, 0x1_182B), (0x1_18A0, 0x1_18DF), (0x1_18FF, 0x1_1906), (0x1_1909, 0x1_1909),
            (0x1_190C, 0x1_1913), (0x1_1915, 0x1_1916), (0x1_1918, 0x1_192F), (0x1_193F, 0x1_193F),
            (0x1_1941, 0x1_1941), (0x1_19A0, 0x1_19A7), (0x1_19AA, 0x1_19D0), (0x1_19E1, 0x1_19E1),
            (0x1_19E3, 0x1_19E3), (0x1_1A00, 0x1_1A00), (0x1_1A0B, 0x1_1A32), (0x1_1A3A, 0x1_1A3A),
            (0x1_1A50, 0x1_1A50), (0x1_1A5C, 0x1_1A89), (0x1_1A9D, 0x1_1A9D), (0x1_1AB0, 0x1_1AF8),
            (0x1_1B0A, 0x1_1B0A), (0x1_1BC0, 0x1_1BE0), (0x1_1C00, 0x1_1C08), (0x1_1C0A, 0x1_1C2E),
            (0x1_1C40, 0x1_1C40), (0x1_1C72, 0x1_1C8F), (0x1_1D00, 0x1_1D06), (0x1_1D08, 0x1_1D09),
            (0x1_1D0B, 0x1_1D30), (0x1_1D46, 0x1_1D46), (0x1_1D60, 0x1_1D65), (0x1_1D67, 0x1_1D68),
            (0x1_1D6A, 0x1_1D89), (0x1_1D98, 0x1_1D98), (0x1_1DB0, 0x1_1DDB), (0x1_1DF1, 0x1_1DF1),
            (0x1_1EE0, 0x1_1EF2), (0x1_1F02, 0x1_1F02), (0x1_1F04, 0x1_1F10), (0x1_1F12, 0x1_1F33),
            (0x1_1FB0, 0x1_1FB0), (0x1_2000, 0x1_2399), (0x1_2400, 0x1_246F), (0x1_2475, 0x1_2543),
            (0x1_2550, 0x1_2686), (0x1_2F90, 0x1_2FF0), (0x1_3000, 0x1_342F), (0x1_3441, 0x1_3446),
            (0x1_3460, 0x1_43FA), (0x1_4400, 0x1_4646), (0x1_6100, 0x1_611D), (0x1_6800, 0x1_6A38),
            (0x1_6A40, 0x1_6A5E), (0x1_6A70, 0x1_6ABE), (0x1_6AD0, 0x1_6AED), (0x1_6B00, 0x1_6B2F),
            (0x1_6B40, 0x1_6B43), (0x1_6B63, 0x1_6B77), (0x1_6B7D, 0x1_6B8F), (0x1_6D40, 0x1_6D6C),
            (0x1_6E40, 0x1_6E7F), (0x1_6EA0, 0x1_6EB8), (0x1_6EBB, 0x1_6ED3), (0x1_6F00, 0x1_6F4A),
            (0x1_6F50, 0x1_6F50), (0x1_6F93, 0x1_6F9F), (0x1_6FE0, 0x1_6FE1), (0x1_6FE3, 0x1_6FE3),
            (0x1_6FF2, 0x1_6FF6), (0x1_7000, 0x1_8CDA), (0x1_8CFF, 0x1_8D20), (0x1_8D80, 0x1_8DF2),
            (0x1_8E00, 0x1_9191), (0x1_91A0, 0x1_91D2), (0x1_AFF0, 0x1_AFF3), (0x1_AFF5, 0x1_AFFB),
            (0x1_AFFD, 0x1_AFFE), (0x1_B000, 0x1_B128), (0x1_B132, 0x1_B132), (0x1_B150, 0x1_B152),
            (0x1_B155, 0x1_B155), (0x1_B164, 0x1_B168), (0x1_B170, 0x1_B2FB), (0x1_BC00, 0x1_BC6A),
            (0x1_BC70, 0x1_BC7C), (0x1_BC80, 0x1_BC88), (0x1_BC90, 0x1_BC99), (0x1_D400, 0x1_D454),
            (0x1_D456, 0x1_D49C), (0x1_D49E, 0x1_D49F), (0x1_D4A2, 0x1_D4A2), (0x1_D4A5, 0x1_D4A6),
            (0x1_D4A9, 0x1_D4AC), (0x1_D4AE, 0x1_D4B9), (0x1_D4BB, 0x1_D4BB), (0x1_D4BD, 0x1_D4C3),
            (0x1_D4C5, 0x1_D505), (0x1_D507, 0x1_D50A), (0x1_D50D, 0x1_D514), (0x1_D516, 0x1_D51C),
            (0x1_D51E, 0x1_D539), (0x1_D53B, 0x1_D53E), (0x1_D540, 0x1_D544), (0x1_D546, 0x1_D546),
            (0x1_D54A, 0x1_D550), (0x1_D552, 0x1_D6A6), (0x1_D6A8, 0x1_D6C0), (0x1_D6C2, 0x1_D6DA),
            (0x1_D6DC, 0x1_D6FA), (0x1_D6FC, 0x1_D714), (0x1_D716, 0x1_D734), (0x1_D736, 0x1_D74E),
            (0x1_D750, 0x1_D76E), (0x1_D770, 0x1_D788), (0x1_D78A, 0x1_D7A8), (0x1_D7AA, 0x1_D7C2),
            (0x1_D7C4, 0x1_D7CB), (0x1_DF00, 0x1_DF81), (0x1_DF90, 0x1_DF96), (0x1_DFCD, 0x1_DFFF),
            (0x1_E030, 0x1_E06D), (0x1_E100, 0x1_E12C), (0x1_E137, 0x1_E13D), (0x1_E14E, 0x1_E14E),
            (0x1_E290, 0x1_E2AD), (0x1_E2C0, 0x1_E2EB), (0x1_E4D0, 0x1_E4EB), (0x1_E5D0, 0x1_E5ED),
            (0x1_E5F0, 0x1_E5F0), (0x1_E6C0, 0x1_E6DE), (0x1_E6E0, 0x1_E6E2), (0x1_E6E4, 0x1_E6E5),
            (0x1_E6E7, 0x1_E6ED), (0x1_E6F0, 0x1_E6F4), (0x1_E6FE, 0x1_E6FF), (0x1_E7E0, 0x1_E7E6),
            (0x1_E7E8, 0x1_E7EB), (0x1_E7ED, 0x1_E7EE), (0x1_E7F0, 0x1_E7FE), (0x1_E800, 0x1_E8C4),
            (0x1_E900, 0x1_E943), (0x1_E94B, 0x1_E94B), (0x1_EE00, 0x1_EE03), (0x1_EE05, 0x1_EE1F),
            (0x1_EE21, 0x1_EE22), (0x1_EE24, 0x1_EE24), (0x1_EE27, 0x1_EE27), (0x1_EE29, 0x1_EE32),
            (0x1_EE34, 0x1_EE37), (0x1_EE39, 0x1_EE39), (0x1_EE3B, 0x1_EE3B), (0x1_EE42, 0x1_EE42),
            (0x1_EE47, 0x1_EE47), (0x1_EE49, 0x1_EE49), (0x1_EE4B, 0x1_EE4B), (0x1_EE4D, 0x1_EE4F),
            (0x1_EE51, 0x1_EE52), (0x1_EE54, 0x1_EE54), (0x1_EE57, 0x1_EE57), (0x1_EE59, 0x1_EE59),
            (0x1_EE5B, 0x1_EE5B), (0x1_EE5D, 0x1_EE5D), (0x1_EE5F, 0x1_EE5F), (0x1_EE61, 0x1_EE62),
            (0x1_EE64, 0x1_EE64), (0x1_EE67, 0x1_EE6A), (0x1_EE6C, 0x1_EE72), (0x1_EE74, 0x1_EE77),
            (0x1_EE79, 0x1_EE7C), (0x1_EE7E, 0x1_EE7E), (0x1_EE80, 0x1_EE89), (0x1_EE8B, 0x1_EE9B),
            (0x1_EEA1, 0x1_EEA3), (0x1_EEA5, 0x1_EEA9), (0x1_EEAB, 0x1_EEBB), (0x2_0000, 0x2_A6DF),
            (0x2_A700, 0x2_B81E), (0x2_B820, 0x2_CEAD), (0x2_CEB0, 0x2_EBE0), (0x2_EBF0, 0x2_EE5D),
            (0x2_F800, 0x2_FA1D), (0x3_0000, 0x3_134A), (0x3_1350, 0x3_3479), (0x3_D000, 0x3_FC3F),
        ];

        /// `XID_Continue` that is not `XID_Start`, past ASCII.
        #[rustfmt::skip]
        const XID_CONTINUE_ONLY: &[(u32, u32)] = &[
            (0x00B7, 0x00B7), (0x0300, 0x036F), (0x0387, 0x0387), (0x0483, 0x0487),
            (0x0591, 0x05BD), (0x05BF, 0x05BF), (0x05C1, 0x05C2), (0x05C4, 0x05C5),
            (0x05C7, 0x05C9), (0x0610, 0x061A), (0x064B, 0x0669), (0x0670, 0x0670),
            (0x06D6, 0x06DC), (0x06DF, 0x06E4), (0x06E7, 0x06E8), (0x06EA, 0x06ED),
            (0x06F0, 0x06F9), (0x0711, 0x0711), (0x0730, 0x074A), (0x07A6, 0x07B0),
            (0x07C0, 0x07C9), (0x07EB, 0x07F3), (0x07FD, 0x07FD), (0x0816, 0x0819),
            (0x081B, 0x0823), (0x0825, 0x0827), (0x0829, 0x082D), (0x0859, 0x085B),
            (0x0897, 0x089F), (0x08CA, 0x08E1), (0x08E3, 0x0903), (0x093A, 0x093C),
            (0x093E, 0x094F), (0x0951, 0x0957), (0x0962, 0x0963), (0x0966, 0x096F),
            (0x0981, 0x0983), (0x09BC, 0x09BC), (0x09BE, 0x09C4), (0x09C7, 0x09C8),
            (0x09CB, 0x09CD), (0x09D7, 0x09D7), (0x09E2, 0x09E3), (0x09E6, 0x09EF),
            (0x09FE, 0x09FE), (0x0A01, 0x0A03), (0x0A3C, 0x0A3C), (0x0A3E, 0x0A42),
            (0x0A47, 0x0A48), (0x0A4B, 0x0A4D), (0x0A51, 0x0A51), (0x0A66, 0x0A71),
            (0x0A75, 0x0A75), (0x0A81, 0x0A83), (0x0ABC, 0x0ABC), (0x0ABE, 0x0AC5),
            (0x0AC7, 0x0AC9), (0x0ACB, 0x0ACD), (0x0AE2, 0x0AE3), (0x0AE6, 0x0AEF),
            (0x0AFA, 0x0AFF), (0x0B01, 0x0B03), (0x0B3C, 0x0B3C), (0x0B3E, 0x0B44),
            (0x0B47, 0x0B48), (0x0B4B, 0x0B4D), (0x0B53, 0x0B57), (0x0B62, 0x0B63),
            (0x0B66, 0x0B6F), (0x0B82, 0x0B82), (0x0BBE, 0x0BC2), (0x0BC6, 0x0BC8),
            (0x0BCA, 0x0BCD), (0x0BD7, 0x0BD7), (0x0BE6, 0x0BEF), (0x0C00, 0x0C04),
            (0x0C3C, 0x0C3C), (0x0C3E, 0x0C44), (0x0C46, 0x0C48), (0x0C4A, 0x0C4D),
            (0x0C55, 0x0C56), (0x0C62, 0x0C63), (0x0C66, 0x0C6F), (0x0C81, 0x0C83),
            (0x0CBC, 0x0CBC), (0x0CBE, 0x0CC4), (0x0CC6, 0x0CC8), (0x0CCA, 0x0CCD),
            (0x0CD5, 0x0CD6), (0x0CE2, 0x0CE3), (0x0CE6, 0x0CEF), (0x0CF3, 0x0CF3),
            (0x0D00, 0x0D03), (0x0D3B, 0x0D3C), (0x0D3E, 0x0D44), (0x0D46, 0x0D48),
            (0x0D4A, 0x0D4D), (0x0D57, 0x0D57), (0x0D62, 0x0D63), (0x0D66, 0x0D6F),
            (0x0D81, 0x0D83), (0x0DCA, 0x0DCA), (0x0DCF, 0x0DD4), (0x0DD6, 0x0DD6),
            (0x0DD8, 0x0DDF), (0x0DE6, 0x0DEF), (0x0DF2, 0x0DF3), (0x0E31, 0x0E31),
            (0x0E33, 0x0E3A), (0x0E47, 0x0E4E), (0x0E50, 0x0E59), (0x0EB1, 0x0EB1),
            (0x0EB3, 0x0EBC), (0x0EC8, 0x0ECE), (0x0ED0, 0x0ED9), (0x0F18, 0x0F19),
            (0x0F20, 0x0F29), (0x0F35, 0x0F35), (0x0F37, 0x0F37), (0x0F39, 0x0F39),
            (0x0F3E, 0x0F3F), (0x0F71, 0x0F84), (0x0F86, 0x0F87), (0x0F8D, 0x0F97),
            (0x0F99, 0x0FBC), (0x0FC6, 0x0FC6), (0x102B, 0x103E), (0x1040, 0x1049),
            (0x1056, 0x1059), (0x105E, 0x1060), (0x1062, 0x1064), (0x1067, 0x106D),
            (0x1071, 0x1074), (0x1082, 0x108D), (0x108F, 0x109D), (0x135D, 0x135F),
            (0x1369, 0x1371), (0x1712, 0x1715), (0x1732, 0x1734), (0x1752, 0x1753),
            (0x1772, 0x1773), (0x17B4, 0x17D3), (0x17DD, 0x17DD), (0x17E0, 0x17E9),
            (0x180B, 0x180D), (0x180F, 0x1819), (0x18A9, 0x18A9), (0x1920, 0x192B),
            (0x1930, 0x193B), (0x1946, 0x194F), (0x19D0, 0x19DA), (0x1A17, 0x1A1B),
            (0x1A55, 0x1A5E), (0x1A60, 0x1A7C), (0x1A7F, 0x1A89), (0x1A90, 0x1A99),
            (0x1AB0, 0x1ABD), (0x1ABF, 0x1AF0), (0x1B00, 0x1B04), (0x1B34, 0x1B44),
            (0x1B50, 0x1B59), (0x1B6B, 0x1B73), (0x1B80, 0x1B82), (0x1BA1, 0x1BAD),
            (0x1BB0, 0x1BB9), (0x1BE6, 0x1BF3), (0x1C24, 0x1C37), (0x1C40, 0x1C49),
            (0x1C50, 0x1C59), (0x1CD0, 0x1CD2), (0x1CD4, 0x1CE8), (0x1CED, 0x1CED),
            (0x1CF4, 0x1CF4), (0x1CF7, 0x1CF9), (0x1DC0, 0x1DFF), (0x200C, 0x200D),
            (0x203F, 0x2040), (0x2054, 0x2054), (0x20D0, 0x20DC), (0x20E1, 0x20E1),
            (0x20E5, 0x20F0), (0x2CEF, 0x2CF1), (0x2D7F, 0x2D7F), (0x2DE0, 0x2DFF),
            (0x302A, 0x302F), (0x3099, 0x309A), (0x30FB, 0x30FB), (0xA620, 0xA629),
            (0xA66F, 0xA66F), (0xA674, 0xA67D), (0xA69E, 0xA69F), (0xA6F0, 0xA6F1),
            (0xA802, 0xA802), (0xA806, 0xA806), (0xA80B, 0xA80B), (0xA823, 0xA827),
            (0xA82C, 0xA82C), (0xA880, 0xA881), (0xA8B4, 0xA8C5), (0xA8D0, 0xA8D9),
            (0xA8E0, 0xA8F1), (0xA8FF, 0xA909), (0xA926, 0xA92D), (0xA947, 0xA953),
            (0xA980, 0xA983), (0xA9B3, 0xA9C0), (0xA9D0, 0xA9D9), (0xA9E5, 0xA9E5),
            (0xA9F0, 0xA9F9), (0xAA29, 0xAA36), (0xAA43, 0xAA43), (0xAA4C, 0xAA4D),
            (0xAA50, 0xAA59), (0xAA7B, 0xAA7D), (0xAAB0, 0xAAB0), (0xAAB2, 0xAAB4),
            (0xAAB7, 0xAAB8), (0xAABE, 0xAABF), (0xAAC1, 0xAAC1), (0xAAEB, 0xAAEF),
            (0xAAF5, 0xAAF6), (0xABE3, 0xABEA), (0xABEC, 0xABED), (0xABF0, 0xABF9),
            (0xFB1E, 0xFB1E), (0xFE00, 0xFE0F), (0xFE20, 0xFE2F), (0xFE33, 0xFE34),
            (0xFE4D, 0xFE4F), (0xFF10, 0xFF19), (0xFF3F, 0xFF3F), (0xFF65, 0xFF65),
            (0xFF9E, 0xFF9F), (0x1_01FD, 0x1_01FD), (0x1_02E0, 0x1_02E0), (0x1_0376, 0x1_037A),
            (0x1_04A0, 0x1_04A9), (0x1_0A01, 0x1_0A03), (0x1_0A05, 0x1_0A06), (0x1_0A0C, 0x1_0A0F),
            (0x1_0A38, 0x1_0A3A), (0x1_0A3F, 0x1_0A3F), (0x1_0AE5, 0x1_0AE6), (0x1_0D24, 0x1_0D27),
            (0x1_0D30, 0x1_0D39), (0x1_0D40, 0x1_0D49), (0x1_0D69, 0x1_0D6D), (0x1_0EAB, 0x1_0EAC),
            (0x1_0ECB, 0x1_0ECF), (0x1_0EF0, 0x1_0EFF), (0x1_0F46, 0x1_0F50), (0x1_0F82, 0x1_0F85),
            (0x1_1000, 0x1_1002), (0x1_1038, 0x1_1046), (0x1_1066, 0x1_1070), (0x1_1073, 0x1_1074),
            (0x1_107F, 0x1_1082), (0x1_10B0, 0x1_10BA), (0x1_10C2, 0x1_10C2), (0x1_10F0, 0x1_10F9),
            (0x1_1100, 0x1_1102), (0x1_1127, 0x1_1134), (0x1_1136, 0x1_113F), (0x1_1145, 0x1_1146),
            (0x1_1173, 0x1_1173), (0x1_1180, 0x1_1182), (0x1_11B3, 0x1_11C0), (0x1_11C9, 0x1_11CC),
            (0x1_11CE, 0x1_11D9), (0x1_122C, 0x1_1237), (0x1_123E, 0x1_123E), (0x1_1241, 0x1_1241),
            (0x1_12DF, 0x1_12EA), (0x1_12F0, 0x1_12F9), (0x1_1300, 0x1_1303), (0x1_133B, 0x1_133C),
            (0x1_133E, 0x1_1344), (0x1_1347, 0x1_1348), (0x1_134B, 0x1_134D), (0x1_1357, 0x1_1357),
            (0x1_1362, 0x1_1363), (0x1_1366, 0x1_136C), (0x1_1370, 0x1_1374), (0x1_13B8, 0x1_13C0),
            (0x1_13C2, 0x1_13C2), (0x1_13C5, 0x1_13C5), (0x1_13C7, 0x1_13CA), (0x1_13CC, 0x1_13D0),
            (0x1_13D2, 0x1_13D2), (0x1_13E1, 0x1_13E2), (0x1_1435, 0x1_1446), (0x1_1450, 0x1_1459),
            (0x1_145E, 0x1_145E), (0x1_14B0, 0x1_14C3), (0x1_14D0, 0x1_14D9), (0x1_15AF, 0x1_15B5),
            (0x1_15B8, 0x1_15C0), (0x1_15DC, 0x1_15DD), (0x1_1630, 0x1_1640), (0x1_1650, 0x1_1659),
            (0x1_16AB, 0x1_16B7), (0x1_16C0, 0x1_16C9), (0x1_16D0, 0x1_16E3), (0x1_171D, 0x1_172B),
            (0x1_1730, 0x1_1739), (0x1_182C, 0x1_183A), (0x1_18E0, 0x1_18E9), (0x1_1930, 0x1_1935),
            (0x1_1937, 0x1_1938), (0x1_193B, 0x1_193E), (0x1_1940, 0x1_1940), (0x1_1942, 0x1_1943),
            (0x1_1950, 0x1_1959), (0x1_19D1, 0x1_19D7), (0x1_19DA, 0x1_19E0), (0x1_19E4, 0x1_19E4),
            (0x1_1A01, 0x1_1A0A), (0x1_1A33, 0x1_1A39), (0x1_1A3B, 0x1_1A3E), (0x1_1A47, 0x1_1A47),
            (0x1_1A51, 0x1_1A5B), (0x1_1A8A, 0x1_1A99), (0x1_1B60, 0x1_1B67), (0x1_1BF0, 0x1_1BF9),
            (0x1_1C2F, 0x1_1C36), (0x1_1C38, 0x1_1C3F), (0x1_1C50, 0x1_1C59), (0x1_1C92, 0x1_1CA7),
            (0x1_1CA9, 0x1_1CB6), (0x1_1D31, 0x1_1D36), (0x1_1D3A, 0x1_1D3A), (0x1_1D3C, 0x1_1D3D),
            (0x1_1D3F, 0x1_1D45), (0x1_1D47, 0x1_1D47), (0x1_1D50, 0x1_1D59), (0x1_1D8A, 0x1_1D8E),
            (0x1_1D90, 0x1_1D91), (0x1_1D93, 0x1_1D97), (0x1_1DA0, 0x1_1DA9), (0x1_1DE0, 0x1_1DE9),
            (0x1_1DF0, 0x1_1DF0), (0x1_1EF3, 0x1_1EF6), (0x1_1F00, 0x1_1F01), (0x1_1F03, 0x1_1F03),
            (0x1_1F34, 0x1_1F3A), (0x1_1F3E, 0x1_1F42), (0x1_1F50, 0x1_1F5A), (0x1_3440, 0x1_3440),
            (0x1_3447, 0x1_3455), (0x1_611E, 0x1_6139), (0x1_6A60, 0x1_6A69), (0x1_6AC0, 0x1_6AC9),
            (0x1_6AF0, 0x1_6AF4), (0x1_6B30, 0x1_6B36), (0x1_6B50, 0x1_6B59), (0x1_6D70, 0x1_6D79),
            (0x1_6F4F, 0x1_6F4F), (0x1_6F51, 0x1_6F87), (0x1_6F8F, 0x1_6F92), (0x1_6FE4, 0x1_6FE4),
            (0x1_6FF0, 0x1_6FF1), (0x1_BC9D, 0x1_BC9E), (0x1_CCF0, 0x1_CCF9), (0x1_CF00, 0x1_CF2D),
            (0x1_CF30, 0x1_CF46), (0x1_D127, 0x1_D128), (0x1_D165, 0x1_D169), (0x1_D16D, 0x1_D172),
            (0x1_D17B, 0x1_D182), (0x1_D185, 0x1_D18B), (0x1_D1AA, 0x1_D1AD), (0x1_D242, 0x1_D244),
            (0x1_D250, 0x1_D252), (0x1_D25B, 0x1_D25C), (0x1_D25F, 0x1_D25F), (0x1_D280, 0x1_D281),
            (0x1_D7CE, 0x1_D7FF), (0x1_DA00, 0x1_DA36), (0x1_DA3B, 0x1_DA6C), (0x1_DA75, 0x1_DA75),
            (0x1_DA84, 0x1_DA84), (0x1_DA9B, 0x1_DA9F), (0x1_DAA1, 0x1_DAAF), (0x1_E000, 0x1_E006),
            (0x1_E008, 0x1_E018), (0x1_E01B, 0x1_E021), (0x1_E023, 0x1_E024), (0x1_E026, 0x1_E02A),
            (0x1_E08F, 0x1_E08F), (0x1_E130, 0x1_E136), (0x1_E140, 0x1_E149), (0x1_E2AE, 0x1_E2AE),
            (0x1_E2EC, 0x1_E2F9), (0x1_E4EC, 0x1_E4F9), (0x1_E5EE, 0x1_E5EF), (0x1_E5F1, 0x1_E5FA),
            (0x1_E6E3, 0x1_E6E3), (0x1_E6E6, 0x1_E6E6), (0x1_E6EE, 0x1_E6EF), (0x1_E6F5, 0x1_E6F5),
            (0x1_E8D0, 0x1_E8D6), (0x1_E944, 0x1_E94A), (0x1_E950, 0x1_E959), (0x1_FBF0, 0x1_FBF9),
            (0xE_0100, 0xE_01EF),
        ];

        /// `Emoji` past ASCII.
        #[rustfmt::skip]
        const EMOJI: &[(u32, u32)] = &[
            (0x00A9, 0x00A9), (0x00AE, 0x00AE), (0x203C, 0x203C), (0x2049, 0x2049),
            (0x2122, 0x2122), (0x2139, 0x2139), (0x2194, 0x2199), (0x21A9, 0x21AA),
            (0x231A, 0x231B), (0x2328, 0x2328), (0x23CF, 0x23CF), (0x23E9, 0x23F3),
            (0x23F8, 0x23FA), (0x24C2, 0x24C2), (0x25AA, 0x25AB), (0x25B6, 0x25B6),
            (0x25C0, 0x25C0), (0x25FB, 0x25FE), (0x2600, 0x2604), (0x260E, 0x260E),
            (0x2611, 0x2611), (0x2614, 0x2615), (0x2618, 0x2618), (0x261D, 0x261D),
            (0x2620, 0x2620), (0x2622, 0x2623), (0x2626, 0x2626), (0x262A, 0x262A),
            (0x262E, 0x262F), (0x2638, 0x263A), (0x2640, 0x2640), (0x2642, 0x2642),
            (0x2648, 0x2653), (0x265F, 0x2660), (0x2663, 0x2663), (0x2665, 0x2666),
            (0x2668, 0x2668), (0x267B, 0x267B), (0x267E, 0x267F), (0x2692, 0x2697),
            (0x2699, 0x2699), (0x269B, 0x269C), (0x26A0, 0x26A1), (0x26A7, 0x26A7),
            (0x26AA, 0x26AB), (0x26B0, 0x26B1), (0x26BD, 0x26BE), (0x26C4, 0x26C5),
            (0x26C8, 0x26C8), (0x26CE, 0x26CF), (0x26D1, 0x26D1), (0x26D3, 0x26D4),
            (0x26E9, 0x26EA), (0x26F0, 0x26F5), (0x26F7, 0x26FA), (0x26FD, 0x26FD),
            (0x2702, 0x2702), (0x2705, 0x2705), (0x2708, 0x270D), (0x270F, 0x270F),
            (0x2712, 0x2712), (0x2714, 0x2714), (0x2716, 0x2716), (0x271D, 0x271D),
            (0x2721, 0x2721), (0x2728, 0x2728), (0x2733, 0x2734), (0x2744, 0x2744),
            (0x2747, 0x2747), (0x274C, 0x274C), (0x274E, 0x274E), (0x2753, 0x2755),
            (0x2757, 0x2757), (0x2763, 0x2764), (0x2795, 0x2797), (0x27A1, 0x27A1),
            (0x27B0, 0x27B0), (0x27BF, 0x27BF), (0x2934, 0x2935), (0x2B05, 0x2B07),
            (0x2B1B, 0x2B1C), (0x2B50, 0x2B50), (0x2B55, 0x2B55), (0x3030, 0x3030),
            (0x303D, 0x303D), (0x3297, 0x3297), (0x3299, 0x3299), (0x1_F004, 0x1_F004),
            (0x1_F0CF, 0x1_F0CF), (0x1_F170, 0x1_F171), (0x1_F17E, 0x1_F17F), (0x1_F18E, 0x1_F18E),
            (0x1_F191, 0x1_F19A), (0x1_F1E6, 0x1_F1FF), (0x1_F201, 0x1_F202), (0x1_F21A, 0x1_F21A),
            (0x1_F22F, 0x1_F22F), (0x1_F232, 0x1_F23A), (0x1_F250, 0x1_F251), (0x1_F300, 0x1_F321),
            (0x1_F324, 0x1_F393), (0x1_F396, 0x1_F397), (0x1_F399, 0x1_F39B), (0x1_F39E, 0x1_F3F0),
            (0x1_F3F3, 0x1_F3F5), (0x1_F3F7, 0x1_F4FD), (0x1_F4FF, 0x1_F53D), (0x1_F549, 0x1_F54E),
            (0x1_F550, 0x1_F567), (0x1_F56F, 0x1_F570), (0x1_F573, 0x1_F57A), (0x1_F587, 0x1_F587),
            (0x1_F58A, 0x1_F58D), (0x1_F590, 0x1_F590), (0x1_F595, 0x1_F596), (0x1_F5A4, 0x1_F5A5),
            (0x1_F5A8, 0x1_F5A8), (0x1_F5B1, 0x1_F5B2), (0x1_F5BC, 0x1_F5BC), (0x1_F5C2, 0x1_F5C4),
            (0x1_F5D1, 0x1_F5D3), (0x1_F5DC, 0x1_F5DE), (0x1_F5E1, 0x1_F5E1), (0x1_F5E3, 0x1_F5E3),
            (0x1_F5E8, 0x1_F5E8), (0x1_F5EF, 0x1_F5EF), (0x1_F5F3, 0x1_F5F3), (0x1_F5FA, 0x1_F64F),
            (0x1_F680, 0x1_F6C5), (0x1_F6CB, 0x1_F6D2), (0x1_F6D5, 0x1_F6D9), (0x1_F6DC, 0x1_F6E5),
            (0x1_F6E9, 0x1_F6E9), (0x1_F6EB, 0x1_F6EC), (0x1_F6F0, 0x1_F6F0), (0x1_F6F3, 0x1_F6FC),
            (0x1_F7E0, 0x1_F7EB), (0x1_F7F0, 0x1_F7F0), (0x1_F90C, 0x1_F93A), (0x1_F93C, 0x1_F945),
            (0x1_F947, 0x1_F9FF), (0x1_FA70, 0x1_FA7C), (0x1_FA80, 0x1_FAC6), (0x1_FAC8, 0x1_FAC8),
            (0x1_FACC, 0x1_FADD), (0x1_FADF, 0x1_FAEB), (0x1_FAEF, 0x1_FAFA),
        ];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(language: Language, line: &str) -> String {
        code_view(language, line, &mut ScanState::default())
    }

    /// One Rust line in the lexer reading, from a fresh state.
    fn lexer_view(line: &str) -> String {
        rust_lexer_view(line, &mut RustLexerState::default())
    }

    #[test]
    fn languages_come_from_the_path_or_a_shebang() {
        assert_eq!(Language::of("src/a.rs", None), Some(Language::Rust));
        assert_eq!(Language::of("a/b.tsx", None), Some(Language::JavaScript));
        assert_eq!(Language::of("x.pyi", None), Some(Language::Python));
        assert_eq!(Language::of("cmd/main.go", None), Some(Language::Go));
        assert_eq!(
            Language::of(".github/workflows/ci.yml", None),
            Some(Language::Yaml)
        );
        assert_eq!(
            Language::of("docker/Dockerfile", None),
            Some(Language::Shell)
        );
        assert_eq!(Language::of("Dockerfile.dev", None), Some(Language::Shell));
        assert_eq!(Language::of("Makefile", None), Some(Language::Shell));
        assert_eq!(
            Language::of("scripts/run", Some("#!/bin/bash")),
            Some(Language::Shell)
        );
        assert_eq!(
            Language::of("bin/tool", Some("#!/usr/bin/env python3 -u")),
            Some(Language::Python)
        );
        assert_eq!(
            Language::of("bin/x", Some("#!/usr/bin/env node")),
            Some(Language::JavaScript)
        );
        assert_eq!(Language::of("bin/x", Some("#!/usr/bin/perl")), None);
        assert_eq!(Language::of("bin/x", Some("echo hi")), None);
        assert_eq!(Language::of("README.md", None), None);
        // An extension wins over a shebang.
        assert_eq!(
            Language::of("a.rs", Some("#!/bin/sh")),
            Some(Language::Rust)
        );
    }

    #[test]
    fn comments_and_strings_are_blanked_in_place() {
        assert_eq!(
            view(Language::Rust, r#"let x = "unsafe"; // unsafe"#),
            r#"let x = "      "; "#
        );
        assert_eq!(view(Language::Rust, "a /* b */ c"), "a         c");
        assert_eq!(
            view(Language::Python, "x = 'eval(' # eval("),
            "x = '     ' "
        );
        assert_eq!(view(Language::JavaScript, "`$eval(x)`"), "`        `");
        assert_eq!(view(Language::Shell, r"echo 'a\' curl"), r"echo '  ' curl");
        assert_eq!(view(Language::Rust, r#""a\"b" c"#), r#""    " c"#);
        // YAML keeps quoted scalars (a `run:` line is code) and drops `#` comments.
        assert_eq!(
            view(Language::Yaml, r#"run: "curl x" # c"#),
            r#"run: "curl x" "#
        );
        assert_eq!(view(Language::Go, "a // b"), "a ");
    }

    /// S3.18: a YAML key and value as YAML reads them, by the bytes of the line.
    #[test]
    fn a_quoted_scalar_reads_as_its_content() {
        for (line, read) in [
            ("  contents: 'write'", "  contents: write"),
            (r#"  "contents": "write""#, "  contents: write"),
            ("  'id-token':write", "  id-token: write"),
            (
                r#"permissions: {"contents":"write", 'a': 'b'}"#,
                "permissions: {contents: write, a: b}",
            ),
            (
                "on: ['pull_request_target', \"push\"]",
                "on: [pull_request_target, push]",
            ),
            ("  contents : write", "  contents: write"),
            ("  contents:\t  write  ", "  contents: write  "),
            ("  x : ", "  x: "),
            ("  contents: 'write'  ", "  contents: write  "),
        ] {
            assert_eq!(scalar_reading(line).as_deref(), Some(read), "{line}");
        }
        for line in [
            "  contents: write",
            "  contents: 'write'x",
            "  contents: x'write'",
            r#"  contents: "wr\"ite""#,
            r#"  contents: "\x77rite""#,
            "  contents: 'it''s'",
            "  contents: 'open",
            "  run: a:b",
            "  key:",
            "",
        ] {
            assert_eq!(scalar_reading(line), None, "{line}");
        }
        // Linear: a line of quotes, blanks and colons of 2 MiB reads in one pass.
        for unit in ["'", "' ", "'a'x ", " :", ": ", "\"\\"] {
            let line = unit.repeat(2 * 1024 * 1024 / unit.len());
            let _ = scalar_reading(&line);
        }
    }

    #[test]
    fn code_inside_an_interpolation_is_code() {
        // Shell double quotes run `$(…)` and backticks; single quotes run nothing.
        assert_eq!(
            view(Language::Shell, r#"V="$(curl x)" y"#),
            r#"V="$(curl x)" y"#
        );
        assert_eq!(
            view(Language::Shell, r#"V="a`curl x`b""#),
            r#"V=" `curl x` ""#
        );
        assert_eq!(view(Language::Shell, "V='$(curl x)'"), "V='         '");
        // `\$(` is a literal dollar; the nesting is counted, and the string resumes after.
        assert_eq!(view(Language::Shell, r#""\$(curl)""#), r#""        ""#);
        assert_eq!(
            view(Language::Shell, r#""$(a $(curl) b) c" d"#),
            r#""$(a $(curl) b)  " d"#
        );
        // JS template literals run `${…}`; other quotes do not.
        assert_eq!(
            view(Language::JavaScript, "`a${f({b: 1})}c`"),
            "` ${f({b: 1})} `"
        );
        assert_eq!(view(Language::JavaScript, "'${eval(x)}'"), "'          '");
        // A parameter expansion is scanned like a substitution: its quotes nest.
        assert_eq!(view(Language::Shell, r#""${curl}""#), r#""${curl}""#);
        // Python f-strings run `{…}`, but not `{{`; plain and bytes strings run nothing.
        assert_eq!(
            view(Language::Python, r#"f"a{eval(x)}b""#),
            r#"f" {eval(x)} ""#
        );
        assert_eq!(view(Language::Python, r"rF'{eval(x)}'"), r"rF'{eval(x)}'");
        assert_eq!(
            view(Language::Python, r#"f"{{eval(x)}}""#),
            r#"f"           ""#
        );
        assert_eq!(view(Language::Python, r#""{eval(x)}""#), r#""         ""#);
        assert_eq!(view(Language::Python, r#"b"{eval(x)}""#), r#"b"         ""#);
        // `elif"` and `buf"` end in `f` but are not f-string prefixes.
        assert_eq!(
            view(Language::Python, r#"elif"{eval(x)}""#),
            r#"elif"         ""#
        );
        assert_eq!(
            view(Language::Python, r#"buf"{eval(x)}""#),
            r#"buf"         ""#
        );
        // The opener's bytes are copied, not counted: an empty `$()` closes at once, and the
        // string resumes (blank) right after it.
        assert_eq!(view(Language::Shell, r#""$()curl" x"#), r#""$()    " x"#);
        assert_eq!(view(Language::JavaScript, "`${}eval(x)`"), "`${}       `");
        // An interpolation left open at the end of the line ends with it.
        assert_eq!(view(Language::Shell, r#""$(curl"#), r#""$(curl"#);
    }

    #[test]
    fn shell_words_and_command_positions_are_exact() {
        let at = |line: &str, word: &str| line.find(word).unwrap();
        for (line, yes) in [
            ("case x", true),
            ("a;case x", true),
            ("(case", true),
            ("echo case", true),
            ("$case", false),
            ("--ignore-case", false),
            ("case=1", false),
            ("case-tool", false),
            ("x case;", true),
            ("x case)", true),
        ] {
            assert_eq!(
                shell_word_at(line.as_bytes(), at(line, "case"), b"case"),
                yes,
                "{line}"
            );
        }
        for (line, in_case, yes) in [
            ("case", false, true),
            ("a; case", false, true),
            ("a | case", false, true),
            ("a && case", false, true),
            ("( case", false, true),
            ("{ case", false, true),
            ("${case", false, false),
            ("` case", false, true),
            ("! case", false, true),
            ("if case", false, true),
            ("elif case", false, true),
            ("while case", false, true),
            ("until case", false, true),
            ("time case", false, true),
            ("then case", false, true),
            ("do case", false, true),
            ("else case", false, true),
            ("a) case", true, true),
            ("a) case", false, false),
            ("echo case", false, false),
            ("undo case", false, false),
        ] {
            let i = line.rfind("case").unwrap();
            assert_eq!(
                in_command_position(line.as_bytes(), i, in_case),
                yes,
                "{line}"
            );
        }
        for (line, yes) in [
            ("case x in (", true),
            ("a;; (", true),
            ("a;& (", true),
            ("echo (", false),
            ("begin (", false),
        ] {
            assert_eq!(
                opens_case_pattern(line.as_bytes(), line.len() - 1),
                yes,
                "{line}"
            );
        }
    }

    #[test]
    fn js_regex_starts_and_lengths_are_exact() {
        for (before, yes) in [
            ("", true),
            ("x = ", true),
            ("f(", true),
            ("a + ", true),
            ("a - ", true),
            ("a * ", true),
            ("a ~ ", true),
            ("[", true),
            ("return ", true),
            ("typeof", true),
            ("await ", true),
            ("a", false),
            ("a ", false),
            (")", false),
            ("]", false),
            ("a++ ", false),
            ("a-- ", false),
            ("<", false),
            ("x.return", false),
        ] {
            assert_eq!(starts_regex(before.as_bytes(), None), yes, "{before:?}");
        }
        assert_eq!(regex_len(b"/a/g"), Some(3));
        assert_eq!(regex_len(br"/\//x"), Some(4));
        assert_eq!(regex_len(b"/[/]/"), Some(5));
        assert_eq!(regex_len(b"/[a]/"), Some(5));
        assert_eq!(regex_len(b"/a"), None);
        assert_eq!(regex_len(br"/\/"), None);
        assert_eq!(trailing_word(b"x = return"), b"return");
        assert_eq!(trailing_word(b"a("), b"");
        assert!(ends_with_word(b"x in", b"in") && !ends_with_word(b"bin", b"in"));
        assert!(ends_with_word(b"in", b"in"));
    }

    /// A line that defeats every probe stays linear: one failed regex probe ends probing, and
    /// whitespace before a shell word is trimmed only at `case` and `esac`.
    #[test]
    fn hostile_lines_scan_in_linear_time() {
        within_deadline(|| {
            let regexes = "x = [/".repeat(100_000);
            assert_eq!(view(Language::JavaScript, &regexes).len(), regexes.len());
            let spaces = format!("V=\"$({}curl x)\"", " ".repeat(200_000));
            assert!(view(Language::Shell, &spaces).ends_with("curl x)\""));
        });
    }

    /// The keyword test reads a bounded suffix and answers as the released walk over the whole
    /// word did (FINDING-5): for every prefix of every keyword with up to two more word bytes
    /// than the longest keyword before it, after each kind of byte, and of 4,000 generated
    /// lines with words of up to 300 bytes.
    #[test]
    fn the_bounded_keyword_reading_is_the_released_one() {
        assert_eq!(JS_OPERAND_KEYWORD_MAX, b"instanceof".len());
        let same_at_every_prefix = |line: &[u8]| {
            for end in 0..=line.len() {
                let before = &line[..end];
                assert_eq!(
                    ends_with_operand_keyword(before),
                    released_ends_with_operand_keyword(before),
                    "{:?}",
                    String::from_utf8_lossy(before)
                );
            }
        };
        let mut keywords_seen = 0;
        for keyword in JS_OPERAND_KEYWORDS {
            for pad in 0..=JS_OPERAND_KEYWORD_MAX + 2 {
                for lead in ["", ".", " ", "$", "x.", "é", "("] {
                    let line = format!(
                        "{lead}{}{}",
                        "a".repeat(pad),
                        std::str::from_utf8(keyword).unwrap()
                    );
                    same_at_every_prefix(line.as_bytes());
                    keywords_seen += usize::from(ends_with_operand_keyword(line.as_bytes()));
                }
            }
        }
        // Each keyword alone and after each lead but `.` and `x.`: 14 x 5.
        assert_eq!(keywords_seen, 70);
        for line in released_word_walk::hostile_lines(0x5EED_F1D5_0000_0005, 4000, 300) {
            same_at_every_prefix(line.as_bytes());
        }
    }

    /// The scanner with the bounded keyword test gives the released views and states: 3,000
    /// generated JavaScript lines (words of up to 300 bytes), read line after line with
    /// whatever the lines before left open; and each keyword with up to two more word bytes
    /// than the longest keyword before it, or a `.`, before a string, a template and a regex,
    /// each line read from code. Every line is read once each way.
    #[test]
    fn the_bounded_keyword_reading_gives_the_released_views() {
        let generated = released_word_walk::hostile_lines(0x5EED_F1D5_0000_0006, 3000, 300);
        let mut alone = Vec::new();
        for keyword in JS_OPERAND_KEYWORDS {
            let keyword = std::str::from_utf8(keyword).unwrap();
            for after in ["\"a\" + b", "'a' + b", "`a` + b", "/a/g + b"] {
                alone.push(format!("x.{keyword}{after}"));
                for pad in 0..=JS_OPERAND_KEYWORD_MAX + 2 {
                    alone.push(format!("x = {}{keyword}{after}", "a".repeat(pad)));
                }
            }
        }
        let read = || {
            let mut state = ScanState::default();
            let mut views: Vec<(String, ScanState)> = generated
                .iter()
                .map(|line| (code_view(Language::JavaScript, line, &mut state), state))
                .collect();
            for line in &alone {
                let mut state = ScanState::default();
                views.push((code_view(Language::JavaScript, line, &mut state), state));
            }
            views
        };
        let bounded = read();
        let (released, asked) = released_word_walk::run(read);
        assert!(
            asked > 10_000,
            "the released reading answered {asked} times"
        );
        let lines = generated.iter().chain(&alone);
        for ((line, new), old) in lines.zip(&bounded).zip(&released) {
            assert_eq!(new, old, "{line:?}");
        }
    }

    /// One JavaScript word of 200,000 letters scans in linear time (FINDING-5: the released
    /// scan walked back over the word at each of its bytes).
    #[test]
    fn a_long_javascript_word_scans_in_linear_time() {
        within_deadline(|| {
            let word = "a".repeat(200_000);
            assert_eq!(view(Language::JavaScript, &word), word);
            let keyword_after = format!("{word}return\"x\"");
            assert_eq!(view(Language::JavaScript, &keyword_after), keyword_after);
        });
    }

    /// Run `work` on its own thread and fail if it takes more than ten seconds: a linear scan
    /// of these lines takes milliseconds, so a quadratic regression fails here instead of
    /// hanging the suite (and a diff-mutation run reports it as caught, not as a timeout).
    fn within_deadline<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> T {
        let (done, wait) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = done.send(work());
        });
        wait.recv_timeout(std::time::Duration::from_secs(10))
            .expect("a long line was scanned within ten seconds")
    }

    /// Exact views for each decision the diff-mutation run found untested (2026-10-31): regex
    /// starts and lengths, Python format specs and brackets, escapes in f-strings and raw
    /// strings, shell `case` positions and pattern parens, and word-start `#`.
    #[test]
    fn every_scanner_decision_is_pinned() {
        for (language, line, expected) in [
            (
                Language::JavaScript,
                "if (!/'/.test(a)) eval(y) //'",
                "if (!/ /.test(a)) eval(y) ",
            ),
            (Language::JavaScript, "x = /abc/;", "x = /   /;"),
            (
                Language::JavaScript,
                "x = a! / b; y = /c/",
                "x = a! / b; y = / /",
            ),
            (Language::Python, "x = a / b / c", "x = a / b / c"),
            (Language::Python, r#"f"{x:>10}" eval"#, r#"f"{x:   }" eval"#),
            (Language::Python, r#"f"{x:=10}" eval"#, r#"f"{x:   }" eval"#),
            (Language::Python, r#"f"{d[a:b]} c""#, r#"f"{d[a:b]}  ""#),
            (Language::Python, r#"f"{d[a]:>5} c""#, r#"f"{d[a]:  }  ""#),
            (Language::Python, r#"f"{f(a:b)} c""#, r#"f"{f(a:b)}  ""#),
            (
                Language::Python,
                r#"f"{g(a)[b]:x} c""#,
                r#"f"{g(a)[b]: }  ""#,
            ),
            (
                Language::Python,
                r#"f"{ {'a':1}['a'] } c""#,
                r#"f"{ {' ':1}[' '] }  ""#,
            ),
            (Language::Python, r#"f"a\"b" eval(x)"#, r#"f"    " eval(x)"#),
            (
                Language::Python,
                r#"x = r"\"" + eval(y)"#,
                r#"x = r"  " + eval(y)"#,
            ),
            (Language::Shell, "'a' curl", "' ' curl"),
            (
                Language::Shell,
                r#"V="$(echo $(date) case) and curl""#,
                r#"V="$(echo $(date) case)         ""#,
            ),
            (
                Language::Shell,
                r#"V="$(echo $(date) esac) and curl""#,
                r#"V="$(echo $(date) esac)         ""#,
            ),
            (
                Language::Shell,
                r#"V="$(a in (b) c) d""#,
                r#"V="$(a in (b) c)  ""#,
            ),
            (
                Language::Shell,
                r#"V="$(case x in (b) c;; esac) d""#,
                r#"V="$(case x in (b) c;; esac)  ""#,
            ),
            (Language::Shell, "echo a;#curl", "echo a;"),
            (Language::Shell, "echo a(#curl", "echo a("),
            // Only JS has regex literals.
            (Language::Shell, "a=/curl/ b", "a=/curl/ b"),
            (Language::Python, "x = (/curl/)", "x = (/curl/)"),
            // An empty case arm closes at `esac`, so the substitution ends at its `)`.
            (
                Language::Shell,
                r#"V="$(case a in a) esac) d""#,
                r#"V="$(case a in a) esac)  ""#,
            ),
            (
                Language::Rust,
                r#"let s = r"a\"; unsafe"#,
                r#"let s = r"  "; unsafe"#,
            ),
        ] {
            assert_eq!(view(language, line), expected, "{language:?}: {line}");
        }
    }

    #[test]
    fn interpolating_prefixes_are_exact() {
        for yes in ["f", "F", "rf", "fR", "x = f", "(Rf", "t", "T", "rt", "Tr"] {
            assert!(is_interpolating_prefix(yes.as_bytes()), "{yes}");
        }
        for no in [
            "", "r", "b", "rb", "elif", "buf", "fff", "x_f", "9f", "at", "ftt",
        ] {
            assert!(!is_interpolating_prefix(no.as_bytes()), "{no}");
        }
    }

    #[test]
    fn python_and_shell_slashes_are_code_and_rust_hashes_are_code() {
        // `//` is floor division in Python and a path in shell, not a comment.
        assert_eq!(view(Language::Python, "a // b; eval(c)"), "a // b; eval(c)");
        assert_eq!(view(Language::Shell, "cat //x; curl y"), "cat //x; curl y");
        assert_eq!(view(Language::Yaml, "url: http://x"), "url: http://x");
        assert_eq!(
            view(Language::Rust, "#[test] fn t() {}"),
            "#[test] fn t() {}"
        );
        assert_eq!(view(Language::JavaScript, "this.#x = 1"), "this.#x = 1");
    }

    #[test]
    fn an_escape_blanks_exactly_the_next_byte() {
        assert_eq!(view(Language::Rust, r#""\\" x"#), r#""  " x"#);
        assert_eq!(view(Language::Go, r#""a\\b" c"#), r#""    " c"#);
        assert_eq!(view(Language::JavaScript, r"'\'' y"), r"'  ' y");
        assert_eq!(view(Language::Shell, r#""a\"b" c"#), r#""    " c"#);
    }

    #[test]
    fn a_block_comment_end_blanks_both_bytes() {
        assert_eq!(view(Language::Rust, "a/*b*/c"), "a     c");
        assert_eq!(view(Language::Rust, "/**/x"), "    x");
        let mut state = ScanState::default();
        assert_eq!(
            code_view(Language::Go, "/* a */ b /* c", &mut state),
            "        b     "
        );
        assert!(state.plain.in_block_comment);
    }

    #[test]
    fn block_comments_and_triple_strings_carry_across_lines() {
        let mut state = ScanState::default();
        assert_eq!(code_view(Language::Rust, "a /* b", &mut state), "a     ");
        assert_eq!(code_view(Language::Rust, "unsafe", &mut state), "      ");
        assert_eq!(
            code_view(Language::Rust, "c */ unsafe", &mut state),
            "     unsafe"
        );
        assert_eq!(state, ScanState::default());
        let mut state = ScanState::default();
        assert_eq!(
            code_view(Language::Python, "x = '''", &mut state),
            "x = '''"
        );
        assert_eq!(
            code_view(Language::Python, "eval(x)", &mut state),
            "       "
        );
        assert_eq!(
            code_view(Language::Python, "''' + eval", &mut state),
            "''' + eval"
        );
        assert_eq!(state, ScanState::default());
    }

    /// Bug hunt 2026-11-01: a Go raw string (backticks) was read as code, so a `/*` in a glob
    /// opened a block comment over the rest of the hunk, a `//` in a URL ended the line and a
    /// `"` opened a phantom string.
    #[test]
    fn go_raw_strings_are_strings_without_escapes() {
        for (line, expected) in [
            (
                "m, _ := filepath.Glob(`/tmp/*.o`); exec.Command(x)",
                "m, _ := filepath.Glob(`        `); exec.Command(x)",
            ),
            (
                "u := `https://e.x/p`; exec.Command(x)",
                "u := `             `; exec.Command(x)",
            ),
            ("q := `\"`; exec.Command(x)", "q := ` `; exec.Command(x)"),
            // A backslash is a plain byte: the second backtick closes the string.
            (r"q := `a\`; exec.Command(x)", "q := `  `; exec.Command(x)"),
            // An interpreted string still escapes.
            (
                r#"q := "a\"`"; exec.Command(x)"#,
                r#"q := "    "; exec.Command(x)"#,
            ),
        ] {
            let mut state = ScanState::default();
            assert_eq!(
                code_view(Language::Go, line, &mut state),
                expected,
                "{line}"
            );
            assert_eq!(state, ScanState::default(), "{line}");
        }
        // A raw string spans lines: its comment-like text opens nothing on the next lines.
        let mut state = ScanState::default();
        let lines = [
            ("q := `SELECT *", "q := `SELECT *"),
            ("FROM t /* x", "FROM t /* x"),
            ("`; exec.Command(x)", "`; exec.Command(x)"),
            ("exec.Command(y)", "exec.Command(y)"),
        ];
        for (line, expected) in lines {
            assert_eq!(
                code_view(Language::Go, line, &mut state),
                expected,
                "{line}"
            );
        }
        // The plain reading took `/*` for a comment, and keeps it; the reading that resumed the
        // raw string closed it, and ends with nothing open.
        assert!(state.plain.in_block_comment);
        assert!(state.readings().any(|r| r == Reading::default()));
    }

    /// Bug hunt 2026-11-01: any word-start `#` ended a YAML line, even inside a quoted scalar
    /// or a quoted shell word of a `run: |` block, so the `curl` or trigger after it was hidden.
    #[test]
    fn a_yaml_hash_inside_quotes_is_not_a_comment() {
        for (line, expected) in [
            (
                r#"echo "step #1" && curl x # c"#,
                r#"echo "step #1" && curl x "#,
            ),
            (
                r#"- run: "echo ' #'; curl x""#,
                r#"- run: "echo ' #'; curl x""#,
            ),
            (
                r#"on: { a: {}, "x #": 1, pull_request_target: {} }"#,
                r#"on: { a: {}, "x #": 1, pull_request_target: {} }"#,
            ),
            // A doubled quote is a single-quoted scalar's only escape; a backslash is text.
            (r"x: 'it''s #1' # c", r"x: 'it''s #1' "),
            (r"x: 'a\' # c", r"x: 'a\' "),
            // A double-quoted scalar escapes with a backslash.
            (r#"x: "a\" #" # c"#, r#"x: "a\" #" "#),
            // A quote after a word byte is plain text, not the start of a scalar.
            ("name: it's #1 # c", "name: it's "),
            ("x: a # 'c", "x: a "),
        ] {
            assert_eq!(view(Language::Yaml, line), expected, "{line}");
        }
    }

    /// Bug hunt 2026-11-01: the line that closes a multi-line string began as plain code, so its
    /// closing quote opened a phantom string over the code after it.
    #[test]
    fn a_line_that_closes_a_multi_line_string_keeps_its_code() {
        let views = |language: Language, lines: &[&str]| {
            let mut state = ScanState::default();
            let out: Vec<String> = lines
                .iter()
                .map(|line| code_view(language, line, &mut state))
                .collect();
            (out, state)
        };
        for (language, lines, expected) in [
            (
                Language::Rust,
                &[r#"const S: &str = ""#, r#""; fn f() { unsafe { *p } }"#][..],
                &[r#"const S: &str = ""#, r#""; fn f() { unsafe { *p } }"#][..],
            ),
            (
                Language::Shell,
                &[r#"MSG="hello"#, r#"world"; curl x | sh; echo "done""#],
                &[r#"MSG="hello"#, r#"world"; curl x | sh; echo "done""#],
            ),
            (
                Language::JavaScript,
                &["const t = `", "`; eval(p); const u = `x`;"],
                &["const t = `", "`; eval(p); const u = `x`;"],
            ),
            // A backslash at the end of a line escapes the line break, not the next quote.
            (
                Language::Python,
                &[r#"s = "abc\"#, r#""; os.system("id")"#],
                &[r#"s = "abc\"#, r#""; os.system("id")"#],
            ),
            // A line still inside the string, shorter than where the string opened.
            (
                Language::Shell,
                &[r#"echo "a long string"#, "x", r#"y"; curl z"#],
                &[r#"echo "a long string"#, "x", r#"y"; curl z"#],
            ),
        ] {
            let (out, state) = views(language, lines);
            assert_eq!(out, expected, "{language:?}: {lines:?}");
            assert!(
                state.readings().any(|r| r.in_string.is_none()),
                "{language:?}: {lines:?}"
            );
        }
        // A quote misread as an opener hides nothing on the next lines, though the reading
        // that resumes inside it may stay open.
        let (out, _) = views(
            Language::Shell,
            &["echo don't", "curl x | sh; echo 'y'", "curl z | sh"],
        );
        assert_eq!(out, ["echo don't", "curl x | sh; echo 'y'", "curl z | sh"]);
        // A string open inside an interpolation ends with the line.
        let (_, state) = views(Language::Shell, &[r#"V="$(echo "a"#]);
        assert_eq!(state, ScanState::default());
        // A carried string closes, and the resumed reading's comment is the line's comment.
        let (out, _) = views(Language::Rust, &[r#"let s = ""#, r#"a"; f(); // unsafe"#]);
        assert_eq!(out[1], format!(r#"a"; f();{}"#, " ".repeat(10)));
    }

    /// Review 2026-11-01: only the reading that resumed a carried string went on to the next
    /// line, so a quote misread as an opener (`It's` in a hunk that starts inside a Go raw
    /// string) was carried from line to line until a real quote "closed" it and a `/*` in the
    /// string after it opened a block comment over the rest of the hunk. Each line now keeps at
    /// least what the plain reading, which carries no string, keeps.
    #[test]
    fn a_misread_opener_hides_nothing_on_later_lines() {
        let mut hidden = Vec::new();
        for (language, lines, tokens) in [
            (
                Language::Go,
                &[
                    "It's safe to run twice.",
                    "`,",
                    "\tRunE: func(cmd *cobra.Command, args []string) error {",
                    "\t\tfmt.Println(\"don't panic\")",
                    "\t\tmatches, _ := filepath.Glob(\"/tmp/build/*.o\")",
                    "\t\t_, _ = http.Get(\"https://example.com/ping\")",
                    "\t\treturn exec.Command(\"rm\", matches...).Run()",
                ][..],
                &["http.Get", "exec.Command"][..],
            ),
            (
                Language::JavaScript,
                &["  Say 'hi", "`; const p = \"'/*\";", "eval(payload);"],
                &["eval"],
            ),
            (
                Language::Rust,
                &["  he said \"hi", "let p = \"/*\";", "unsafe { *p }"],
                &["unsafe"],
            ),
            (
                Language::Python,
                &["it's here", "x = \"'\"\"\"", "os.system(\"id\")"],
                &["os.system"],
            ),
        ] {
            let mut state = ScanState::default();
            let views: Vec<String> = lines
                .iter()
                .map(|line| code_view(language, line, &mut state))
                .collect();
            for token in tokens {
                if !views.iter().any(|v| v.contains(token)) {
                    hidden.push(format!("{language:?}: {token} in {views:?}"));
                }
            }
        }
        assert_eq!(hidden, Vec::<String>::new());
    }

    #[test]
    fn a_union_of_views_keeps_what_either_keeps() {
        let union = |line: &str, views: &[&str]| {
            let views: Vec<String> = views.iter().map(ToString::to_string).collect();
            union_views(line, &views)
        };
        assert_eq!(union("abc def", &["a      ", "    de"]), "a   de ");
        assert_eq!(union("ab", &["ab", ""]), "ab");
        assert_eq!(union("ab", &["", "  "]), "  ");
        assert_eq!(union("é x", &["   x", "é  "]), "é x");
        assert_eq!(union("abc", &["a  ", " b ", "  c"]), "abc");
        assert_eq!(union("abc", &[]), "");
    }

    /// Bug hunt 2026-11-01: `cr"…"` (a raw C string) was read as an ordinary string, whose `\"`
    /// escapes the closing quote.
    #[test]
    fn rust_raw_c_strings_are_raw() {
        for (line, expected) in [
            (r#"let d = cr"C:\"; unsafe"#, r#"let d = cr"   "; unsafe"#),
            (
                r##"let d = cr#"a"b"#; unsafe"##,
                r##"let d = cr#"   "#; unsafe"##,
            ),
            (r#"let d = br"C:\"; unsafe"#, r#"let d = br"   "; unsafe"#),
            (r#"let d = c"C:"; unsafe"#, r#"let d = c"  "; unsafe"#),
            // Only one of `b` and `c`, and only at an identifier's start.
            (r#"let d = xcr"\"; unsafe"#, r#"let d = xcr"          "#),
            (r#"let d = bcr"\"; unsafe"#, r#"let d = bcr"          "#),
        ] {
            assert_eq!(view(Language::Rust, line), expected, "{line}");
        }
    }

    #[test]
    fn triple_strings_open_and_close_on_one_line() {
        assert_eq!(
            view(Language::Python, r#"x = """a""" + eval(b)"#),
            r#"x = """ """ + eval(b)"#
        );
        assert_eq!(
            view(Language::Python, "y = '''eval(a)''' ; z"),
            "y = '''       ''' ; z"
        );
        // Two quotes are an empty string, not a triple.
        assert_eq!(
            view(Language::Python, "z = '' + eval(c)"),
            "z = '' + eval(c)"
        );
    }

    #[test]
    fn rust_char_literals_are_not_strings() {
        assert_eq!(
            view(Language::Rust, r#"let q = '"'; unsafe"#),
            "let q =    ; unsafe"
        );
        assert_eq!(
            view(Language::Rust, r"let q = '\''; unsafe"),
            "let q =     ; unsafe"
        );
        assert_eq!(
            view(Language::Rust, "fn f<'a>(x: &'a str)"),
            "fn f<'a>(x: &'a str)"
        );
    }

    #[test]
    fn multi_byte_text_survives_the_view() {
        assert_eq!(view(Language::Rust, "é // ü"), "é ");
        assert_eq!(view(Language::Rust, "\"ü\" x"), "\"  \" x");
    }

    #[test]
    fn identifier_classes_are_exact() {
        for b in *b"aZ09_" {
            assert!(is_ident(b) && is_boundary_blocker(b), "{}", b as char);
        }
        for b in *b".$" {
            assert!(!is_ident(b) && is_boundary_blocker(b), "{}", b as char);
        }
        for b in [b' ', b'(', b'-', b':', b'"', b'/', 0x80] {
            assert!(!is_ident(b) && !is_boundary_blocker(b), "{b:#x}");
        }
    }

    #[test]
    fn tokens_need_boundaries() {
        assert!(has_token("unsafe {", "unsafe"));
        assert!(!has_token("unsafely", "unsafe"));
        assert!(!has_token("not_unsafe", "unsafe"));
        assert!(has_token("x = eval(y)", "eval("));
        assert!(!has_token("re.eval(y)", "eval("));
        assert!(!has_token("$eval(y)", "eval("));
        assert!(has_token("std::process::Command::new(x)", "Command::new"));
        assert!(has_token("aunsafe unsafe", "unsafe"));
        assert!(!has_token("", "unsafe"));
        assert!(has_token("é unsafe", "unsafe"));
        // Boundaries away from the start of the text, and at its end.
        assert!(!has_token("a unsafely", "unsafe"));
        assert!(has_token("a unsafe", "unsafe"));
        assert!(has_token("a (unsafe)", "unsafe"));
        assert!(!has_token("a x.unsafe", "unsafe"));
        assert!(!has_token("a $unsafe", "unsafe"));
        assert!(!has_token("a _unsafe", "unsafe"));
        assert!(!has_token("a 9unsafe", "unsafe"));
        assert!(!has_token("a unsafe_x", "unsafe"));
        assert!(!has_token("a unsafe9", "unsafe"));
        // A token ending in punctuation needs no boundary after it.
        assert!(has_token("a eval(x", "eval("));
    }
    /// The views of Rust `lines`, read in order from a fresh state, and the state after them.
    fn lexer_lines(lines: &[&str]) -> (Vec<String>, RustLexerState) {
        let mut state = RustLexerState::default();
        let views = lines
            .iter()
            .map(|line| rust_lexer_view(line, &mut state))
            .collect();
        (views, state)
    }

    /// The view of one Rust line must be `kept`, with as many blanks before it as the line is
    /// longer: the line starts with a comment or literal that is blank from end to end.
    fn blank_before(line: &str, kept: &str) {
        let blanks = " ".repeat(line.len() - kept.len());
        assert_eq!(lexer_view(line), format!("{blanks}{kept}"), "{line}");
    }

    /// The compiler nests block comments: after `/* /* */` one is still open, a `//` in it is
    /// comment text, and what follows the last `*/` is code. Checked with rustc 1.99.1 in
    /// editions 2015 to 2024: `const N: u32 = /* /* */ // */ 42;` is 42, `/*/ 1 + */ 5` is 5
    /// (the `/` of `/*/` closes nothing), and a nested comment over two lines ends where its
    /// last `*/` is. The reading of 0.8.3 ends a comment at the first `*/`, so the `//` cuts
    /// the line: until this reading was added, the call after it was hidden from every rule,
    /// in every release from 0.5.0 to 0.8.3 (see the CHANGELOG).
    #[test]
    fn rust_block_comments_nest() {
        blank_before(
            r#"/* /* */ // */ std::process::Command::new("sh");"#,
            r#" std::process::Command::new("  ");"#,
        );
        blank_before(
            "/* /* */ // */ let _ = reqwest::blocking::get(u);",
            " let _ = reqwest::blocking::get(u);",
        );
        blank_before(r#"/* a /* b /* c */ */ " */ unsafe { x }"#, " unsafe { x }");
        blank_before("/*/ 1 + */ unsafe", " unsafe");
        blank_before("/**/ unsafe", " unsafe");
        blank_before("/***/ unsafe", " unsafe");
        // `//*` in a comment: its second `/` and the `*` open another one.
        blank_before("/* //* */ #[cfg(test)] // */ unsafe", " unsafe");
        // A comment that closes leaves nothing; one that does not is carried at its depth,
        // by every reading.
        let (views, state) = lexer_lines(&["a /* one /* two", "still */ comment // */ unsafe"]);
        assert_eq!(views, ["a              ", "                       unsafe"]);
        assert_eq!(state, RustLexerState::default());
        let (views, state) = lexer_lines(&["/* /* */ unsafe", "unsafe /* */", "*/ unsafe"]);
        assert_eq!(
            views,
            [" ".repeat(15), " ".repeat(12), "   unsafe".to_owned()]
        );
        assert_eq!(state, RustLexerState::default());
        // A `*/` after the comment has ended is two operators.
        blank_before("/* */ */ unsafe", " */ unsafe");
        let (_, state) = lexer_lines(&["/* /* /*", "*/"]);
        assert!(state.readings().all(|r| r == rust::Open::Comment(2)));
        assert_eq!(state.more, [None; MORE_READINGS]);
    }

    /// A character or byte literal is one token, whatever its escape: `'\x41'`,
    /// `'\u{1F600}'`, `'\''`, `'\\'`, a literal that holds `"`, `/` or `*`, a character of
    /// several bytes, `b'\xff'`. Checked with rustc 1.99.1 (editions 2015 to 2024): the
    /// constants below have the values they spell and the code after them compiles. The
    /// reading of 0.8.3 knows a literal only by a closing quote two bytes on (three after a
    /// backslash): after `'\x41'` it is one quote out of step, takes the `"` of `'"'` for the
    /// start of a string, and `"/*"` then opens a comment over the rest of the hunk. Until
    /// this reading was added that hid the code below such a line, in every release from
    /// 0.5.0 to 0.8.3 (see the CHANGELOG).
    #[test]
    fn rust_character_and_byte_literals_are_read_whole() {
        for literal in [
            "'x'",
            r"'\n'",
            r"'\''",
            r"'\\'",
            r"'\x41'",
            r"'\u{1F600}'",
            r"'\u{10FFFF}'",
            "'\"'",
            "'/'",
            "'*'",
            "'é'",
            "'😀'",
        ] {
            let line = format!("let q = {literal}; unsafe");
            let blanks = " ".repeat(literal.len());
            assert_eq!(
                lexer_view(&line),
                format!("let q = {blanks}; unsafe"),
                "{line}"
            );
        }
        // A byte literal: its `b` is kept, as a string's prefix is.
        for literal in ["'x'", r"'\xff'", r"'\''", "'\"'", "'/'", r"'\\'"] {
            let line = format!("let q = b{literal}; unsafe");
            let blanks = " ".repeat(literal.len());
            assert_eq!(
                lexer_view(&line),
                format!("let q = b{blanks}; unsafe"),
                "{line}"
            );
        }
        // The line of the finding, and what stands after it in the hunk.
        let (views, state) = lexer_lines(&[
            r#"pub const TABLE: (char, char, &str) = ('\x41','"', "/*");"#,
            "let unchecked = unsafe { *v.as_ptr() };",
        ]);
        assert_eq!(
            views,
            [
                r#"pub const TABLE: (char, char, &str) = (      ,   , "  ");"#,
                "let unchecked = unsafe { *v.as_ptr() };"
            ]
        );
        assert_eq!(state, RustLexerState::default());
        // Literals side by side: each closing quote closes, and opens nothing.
        assert_eq!(
            lexer_view(r"m!('a','b'=>'\'','\\'); unsafe"),
            "m!(   ,   =>    ,    ); unsafe"
        );
    }

    /// A `'` before a name is a lifetime or a label, not a literal: `'a`, `'static`, `'_`,
    /// `'outer: loop`, `&'a str`, `<'a, 'b>`, a name past ASCII (`'é`) and a raw lifetime
    /// (`'r#fn`, one token from edition 2021 on). Only a quote right after the name makes a
    /// literal: `'a'` is one, `'ab` is a lifetime, and in `'a,'b'` the first is a lifetime and
    /// the second a literal.
    /// Checked with rustc 1.99.1: the functions compile, and a macro counts `'a 'b' x` and
    /// `'a,'b'` as three tokens each and `'a'` as one.
    #[test]
    fn rust_lifetimes_and_labels_are_code() {
        for line in [
            "fn f<'a, 'b>(x: &'a str, y: &'b str) -> &'static str { unsafe { g(x, y) } }",
            "'outer: loop { break 'outer; } unsafe { g() }",
            "fn f<'é>(x: &'é str, y: &'_ str) { unsafe { g(x, y) } }",
            "impl<'r#fn> X<'r#fn> { fn f() { unsafe { g() } } }",
            "m!('ab 'static 'a); unsafe",
        ] {
            assert_eq!(lexer_view(line), line);
        }
        assert_eq!(lexer_view("m!('a,'b'); unsafe"), "m!('a,   ); unsafe");
        assert_eq!(lexer_view("m!('a 'b' x); unsafe"), "m!('a     x); unsafe");
        assert_eq!(
            lexer_view("m!(&'a str, 'a'); unsafe"),
            "m!(&'a str,    ); unsafe"
        );
        // The lexer makes one literal of a name between quotes, which the compiler then
        // refuses ("character literal may only contain one codepoint"). The view ends it
        // where the lexer does, so what stands after it is read as the lexer reads it, and
        // keeps it as text: the `r` after it starts a raw string, which a suffix would not.
        assert_eq!(
            lexer_view(r#"m!('ab'r"\"); unsafe"#),
            r#"m!('ab'r" "); unsafe"#
        );
    }

    /// Strings: `"…"` with `\"` and `\\`, byte strings and C strings, raw strings with any
    /// number of `#` (a quote with fewer than that after it is text), each also over several
    /// lines. Checked with rustc 1.99.1: the constants have the lengths their text spells
    /// (`r#"a"b"#` three bytes, `br##"x"#y"##` four, `cr#"a"b\"#` four in edition 2021), and
    /// the code after a string that runs over lines compiles.
    #[test]
    fn rust_strings_end_where_the_compiler_ends_them() {
        for (line, expected) in [
            (r#"let s = "a\"b\\"; unsafe"#, r#"let s = "      "; unsafe"#),
            (r#"let s = b"a\"/*"; unsafe"#, r#"let s = b"     "; unsafe"#),
            (r#"let s = c"a\"//"; unsafe"#, r#"let s = c"     "; unsafe"#),
            (r#"let s = r"a\"; unsafe"#, r#"let s = r"  "; unsafe"#),
            (
                r##"let s = r#"a"b"#; unsafe"##,
                r##"let s = r#"   "#; unsafe"##,
            ),
            (
                r###"let s = br##"x"#y"##; unsafe"###,
                r###"let s = br##"    "##; unsafe"###,
            ),
            (
                r##"let s = cr#"a"b\"#; unsafe"##,
                r##"let s = cr#"    "#; unsafe"##,
            ),
            // An empty string, and two strings on a line.
            (
                r#"f("", "//", "/*"); unsafe"#,
                r#"f("", "  ", "  "); unsafe"#,
            ),
        ] {
            let (views, state) = lexer_lines(&[line]);
            assert_eq!(views, [expected], "{line}");
            assert_eq!(state, RustLexerState::default(), "{line}");
        }
        // Over lines: the reading that resumes the string ends it where the compiler does, and
        // keeps the code after it. (The line is also read as code, as a hunk that starts
        // inside a string needs, so the text of the string on it is kept too.)
        for (lines, last) in [
            (
                &[r#"let s = "ab\"#, r#"  cd"; unsafe { g() }"#][..],
                r#"  cd"; unsafe { g() }"#,
            ),
            (
                &[
                    r##"let s = r##"one "# still"##,
                    r###"two"## ; unsafe { g() }"###,
                ][..],
                r###"two"## ; unsafe { g() }"###,
            ),
            (
                &[r#"let s = b"one"#, "/* two", r#"*/ three"; unsafe { g() }"#][..],
                r#"*/ three"; unsafe { g() }"#,
            ),
        ] {
            let (views, state) = lexer_lines(lines);
            assert_eq!(views.last().map(String::as_str), Some(last), "{lines:?}");
            let exact = state.readings().nth(state.exact).unwrap();
            assert_eq!(exact, rust::Open::Nothing, "{lines:?}");
        }
        // A raw string with too few `#` after its quote is still open, and says how many.
        let (views, state) = lexer_lines(&[r##"let s = r##"a"# unsafe"##]);
        assert_eq!(views, [r#"let s = r##"          "#]);
        assert_eq!(state.plain, rust::Open::RawStr(2));
    }

    /// `r#fn` is a raw identifier, a name: no raw string starts there. Checked with rustc
    /// 1.99.1: `fn r#fn()` and `let r#type` compile, and a macro counts `r#fn "x"` as two
    /// tokens.
    #[test]
    fn rust_raw_identifiers_are_names() {
        for line in [
            "let r#type = r#fn(); unsafe { g() }",
            "let v = x.r#unwrap(); unsafe { g() }",
            "#[r#ignore] fn t() { unsafe { g() } }",
        ] {
            assert_eq!(lexer_view(line), line);
        }
        assert_eq!(
            lexer_view(r#"m!(r#fn "unsafe"); unsafe"#),
            r#"m!(r#fn "      "); unsafe"#
        );
    }

    /// An `r`, `b` or `c` before a quote is a prefix only where a token starts. A literal's
    /// suffix and a number take the letter with them, and then the quote opens an ordinary
    /// string, in which `\"` is an escape. Checked with rustc 1.99.1: a macro counts
    /// `'('r"\" "`, `"x"r"\" "`, `1r"\" "` and `1.0r"\" "` as two tokens each (a literal with
    /// the suffix `r`, and one string), `0x1e+r"\"` as three (the number ends before `+`, a
    /// raw string follows) and `1.r"\"` as three; for `1e+r"x"` it reports a number without
    /// exponent digits and then finds the string `"x"`.
    #[test]
    fn rust_suffixes_and_numbers_take_a_prefix_letter() {
        for (line, expected) in [
            (
                r#"m!('('r"\" unsafe "); unsafe"#,
                r#"m!(   r"          "); unsafe"#,
            ),
            (
                r#"m!("x"r"\" unsafe "); unsafe"#,
                r#"m!(" "r"          "); unsafe"#,
            ),
            (
                r#"m!(1r"\" unsafe "); unsafe"#,
                r#"m!(1r"          "); unsafe"#,
            ),
            (
                r#"m!(1.0r"\" unsafe "); unsafe"#,
                r#"m!(1.0r"          "); unsafe"#,
            ),
            (
                r#"m!(1e+r"\" unsafe "); unsafe"#,
                r#"m!(1e+r"          "); unsafe"#,
            ),
            (
                r#"m!(0b1e-r"\" unsafe "); unsafe"#,
                r#"m!(0b1e-r"          "); unsafe"#,
            ),
            (
                r#"m!(1_000u8b'\'' , b'"'); unsafe"#,
                r"m!(1_000u8b     , b   ); unsafe",
            ),
            // Here the letter does start a token: a raw string, which ends at the next quote.
            (r#"m!(0x1e+r"\"); unsafe"#, r#"m!(0x1e+r" "); unsafe"#),
            (r#"m!(1.r"\"); unsafe"#, r#"m!(1.r" "); unsafe"#),
            (r#"m!(0..r"\"); unsafe"#, r#"m!(0..r" "); unsafe"#),
            (r#"m!(0b+r"\"); unsafe"#, r#"m!(0b+r" "); unsafe"#),
            // A name that ends in the letter is a name, and the quote an ordinary string.
            (
                r#"m!(xr"\" unsafe "); unsafe"#,
                r#"m!(xr"          "); unsafe"#,
            ),
            (
                r#"m!(rb"\" unsafe "); unsafe"#,
                r#"m!(rb"          "); unsafe"#,
            ),
        ] {
            assert_eq!(lexer_view(line), expected, "{line}");
        }
    }

    /// Two readings depend on the edition, and the view takes the newer. `cr"\"` is one raw C
    /// string from edition 2021 on, and before that the name `cr` and a string in which `\"`
    /// is an escape; the raw reading ends the string at the second quote, and so keeps more as
    /// code. `c"…"` blanks the same bytes either way. And `#"x"` is a `#` and a string in
    /// every edition that accepts it (2024 reserves it). Checked with rustc 1.99.1: a macro
    /// counts `cr"\" ; "x"` as three tokens in editions 2021 and 2024 and `cr"\" ; "` as two
    /// in 2015 and 2018; `#"x"` is two tokens up to 2021 and an error in 2024.
    #[test]
    fn rust_edition_readings_are_the_newer_ones() {
        assert_eq!(
            lexer_view(r#"m!(cr"\" ; "x"); unsafe"#),
            r#"m!(cr" " ; " "); unsafe"#
        );
        assert_eq!(lexer_view(r#"m!(c"\"x"); unsafe"#), r#"m!(c"   "); unsafe"#);
        assert_eq!(
            lexer_view(r##"m!(#"x"#); unsafe"##),
            r##"m!(#" "#); unsafe"##
        );
        assert_eq!(lexer_view(r#"m!(##"x"); unsafe"#), r#"m!(##" "); unsafe"#);
    }

    /// Doc comments are comments (rustc 1.99.1 compiles a constant after `/// unsafe { "` and
    /// `/** unsafe { " */`).
    #[test]
    fn rust_doc_comments_are_comments() {
        assert_eq!(lexer_view(r#"/// unsafe { ""#), "");
        assert_eq!(lexer_view(r#"x //! unsafe { ""#), "x ");
        blank_before(r#"/** unsafe { " */ fn f() {}"#, " fn f() {}");
        blank_before(r#"/*! unsafe { " */ fn f() {}"#, " fn f() {}");
    }

    /// What the lexer calls unterminated (none of it compiles) is passed over as the lexer
    /// passes over it, and what follows is read as the lexer reads it. An unterminated
    /// character literal ends before a `/` or at the line end and is kept as text. The lexer
    /// goes on with a character literal over a line end in two cases: when the next line
    /// starts with the closing quote, and after a backslash at the line end. Both are kept as
    /// text, and the quote that closes one opens nothing. An unterminated string is blank to
    /// the end of its line, and the lines after it are read both ways, as before.
    #[test]
    fn rust_unterminated_literals_are_passed_over_whole() {
        // rustc 1.99.1: "unterminated character literal"; the lexer ends it before the `/`.
        assert_eq!(lexer_view(r#"m!('(a/ "b" unsafe"#), r#"m!('(a/ " " unsafe"#);
        // A quote in such a literal is no string start.
        assert_eq!(lexer_view(r#"m!('(" unsafe"#), r#"m!('(" unsafe"#);
        let (views, state) = lexer_lines(&["m!('(", "'); unsafe { g() }"]);
        assert_eq!(views, ["m!('(", "'); unsafe { g() }"]);
        let exact = state.readings().nth(state.exact).unwrap();
        assert_eq!(exact, rust::Open::Nothing);
        let (views, state) = lexer_lines(&[r"m!('(\", "x'); unsafe { g() }"]);
        assert_eq!(views, [r"m!('(\", "x'); unsafe { g() }"]);
        let exact = state.readings().nth(state.exact).unwrap();
        assert_eq!(exact, rust::Open::Nothing);
        // Not continued: the next line is read from its start.
        let (views, state) = lexer_lines(&["m!('(", "unsafe { g('x') }"]);
        assert_eq!(views, ["m!('(", "unsafe { g(   ) }"]);
        assert_eq!(state, RustLexerState::default());
        let (views, _) = lexer_lines(&[r#"let s = "abc unsafe"#, "unsafe { g() }"]);
        assert_eq!(views, [r#"let s = "          "#, "unsafe { g() }"]);
    }

    /// Names past ASCII are read as the compiler reads them: a character that may start a
    /// name (`XID_Start`) makes `'é` a lifetime, one that may not makes `'→ x'` one token;
    /// a character that goes on in a name (`XID_Continue`, as U+00B7) keeps a following `r`
    /// from being a prefix. Checked with rustc 1.99.1: `fn f<'é>` compiles, and in edition
    /// 2018 a macro counts `ér"\" ; "` and `a·r"\" ; "` as two tokens each. The lexer takes an
    /// emoji into a name (the compiler then refuses it: "identifiers cannot contain emoji:
    /// `😀r`"), so a letter after one is no prefix either.
    #[test]
    fn rust_names_past_ascii_are_names() {
        for (line, expected) in [
            (
                r#"m!(ér"\" unsafe "); unsafe"#,
                r#"m!(ér"          "); unsafe"#,
            ),
            (
                r#"m!(a·r"\" unsafe "); unsafe"#,
                r#"m!(a·r"          "); unsafe"#,
            ),
            (
                r#"m!(😀r"\" unsafe "); unsafe"#,
                r#"m!(😀r"          "); unsafe"#,
            ),
            (
                r#"m!(a😀r"\" unsafe "); unsafe"#,
                r#"m!(a😀r"          "); unsafe"#,
            ),
            // `→` and a lone U+00B7 start no name: the `r` after them starts a raw string.
            (r#"m!(→r"\"); unsafe"#, r#"m!(→r" "); unsafe"#),
            (r#"m!(·r"\"); unsafe"#, r#"m!(·r" "); unsafe"#),
            // After a quote `→` starts no lifetime: the lexer reads on to the next quote (the
            // compiler refuses that literal; it is kept as text), and a raw string follows.
            // `é` starts one, so the quote after ` x` opens a literal that the line does not
            // close: it is kept as text to the end of the line, and its `"` opens nothing.
            (r#"m!('→ x' r"\"); unsafe"#, r#"m!('→ x' r" "); unsafe"#),
            (r#"m!('é x' r"\"); unsafe"#, r#"m!('é x' r"\"); unsafe"#),
        ] {
            assert_eq!(lexer_view(line), expected, "{line}");
        }
    }

    /// The tables of names and emoji past ASCII are ascending ranges that do not meet, so the
    /// search in them is sound, and they hold what Unicode 18.0.0 says for a few characters
    /// looked up by hand. (The fuzz target's `every_character` seed compares every character
    /// with the lexer.) `#` and `*` and the digits have the property `Emoji` too; the lexer
    /// asks only past ASCII.
    #[test]
    fn rust_name_tables_are_sound() {
        for (c, starts, goes_on, emoji) in [
            ('ª', true, true, false),
            ('À', true, true, false),
            ('Ö', true, true, false),
            ('×', false, false, false),
            ('一', true, true, false),
            // new in Unicode 18.0.0
            ('\u{558}', true, true, false),
            // these go on in a name and start none
            ('·', false, true, false),
            ('\u{301}', false, true, false),
            ('٣', false, true, false),
            ('©', false, false, true),
            ('😀', false, false, true),
            // a letter that is an emoji too
            ('ℹ', true, true, true),
            ('→', false, false, false),
            ('\u{2028}', false, false, false),
            ('\u{feff}', false, false, false),
            // the zero width joiner goes on in a name
            ('\u{200d}', false, true, false),
            ('\u{10FFFF}', false, false, false),
            // ASCII is decided without the tables, and no ASCII character is an emoji here
            ('a', true, true, false),
            ('Z', true, true, false),
            ('_', true, true, false),
            ('0', false, true, false),
            ('9', false, true, false),
            ('#', false, false, false),
            ('*', false, false, false),
            ('\'', false, false, false),
            (' ', false, false, false),
            ('\0', false, false, false),
        ] {
            assert_eq!(
                (rust::id_start(c), rust::id_continue(c), rust::emoji(c)),
                (starts, goes_on, emoji),
                "{c:?}"
            );
        }
        assert!(rust::tables_ascend());
    }

    /// A text on which the readings pile up: a string, then a raw string that holds three
    /// comment openers on three lines. The plain reading, which resumes no string, takes each
    /// opener for a comment and goes one deeper on every line, and each depth it passes
    /// through is a reading of its own. With room for four, the one reading that resumed both
    /// strings, the compiler's, was the fifth: it was dropped, and the code after the raw
    /// string was blank. (Found by the fuzz oracle while this scanner was written; rustc
    /// 1.99.1 compiles the text, with `A` of 3 bytes and `B` of 11.) The exact reading is now
    /// kept before the others.
    #[test]
    fn the_exact_rust_reading_is_never_dropped() {
        use rust::Open::{CharEnd, Comment, Nothing, RawStr, Str};
        let (views, state) = lexer_lines(&[
            r#"pub const A: &str = ""#,
            r#"/*"; pub const B: &str = r##""#,
            "/*",
            " /*",
            "/*",
            r###""##; pub fn f() { unsafe { run() } }"###,
        ]);
        assert_eq!(views[5], r###""##; pub fn f() { unsafe { run() } }"###);
        assert_eq!(state.exact, 1);
        assert_eq!(state.more[0], Some(rust::Open::Nothing));
        // The order of the readings a line leaves: plain, exact, then the rest while there
        // is room, each once.
        let state = rust::after(
            &[Comment(1), Str, Str, RawStr(1), RawStr(2), CharEnd],
            Some(CharEnd),
        );
        let readings: Vec<rust::Open> = state.readings().collect();
        assert_eq!(readings, [Comment(1), CharEnd, Str, RawStr(1)]);
        assert_eq!(state.exact, 1);
        // The exact reading that ends as the plain one is the plain one.
        let state = rust::after(&[Nothing, Str, Nothing], Some(Nothing));
        let readings: Vec<rust::Open> = state.readings().collect();
        assert_eq!((readings, state.exact), (vec![Nothing, Str], 0));
    }

    /// A name with a `#` after it is the name, a `#`, and what follows: `k#r"\"` holds a raw
    /// string. rustc 1.99.1 reads it so in editions 2015 and 2018 (a macro counts `k#fn` and
    /// `k#r"\"` as three tokens each) and refuses it in 2021 and 2024 ("prefix `k` is
    /// unknown"). The lexer this scanner follows has a newer token, `k#name`, that is in no
    /// edition of this compiler, and the scanner does not take it: if it did, `k#r"\"` would
    /// open an ordinary string, one line of it would put the view a quote out of step, and
    /// code that editions 2015 and 2018 compile would be hidden.
    #[test]
    fn rust_k_and_a_hash_are_a_name_and_a_hash() {
        for (line, expected) in [
            (r#"m!(k#r"\"); unsafe"#, r#"m!(k#r" "); unsafe"#),
            (
                r#"m!(k#r"\"); m!(" /* "); unsafe"#,
                r#"m!(k#r" "); m!("    "); unsafe"#,
            ),
            ("m!(k#fn k#unsafe); unsafe", "m!(k#fn k#unsafe); unsafe"),
            (r##"m!(k#"x"#); unsafe"##, r##"m!(k#" "#); unsafe"##),
        ] {
            assert_eq!(lexer_view(line), expected, "{line}");
        }
    }

    /// `'r#name` has a reading in each scanner, and the rules read both. The lexer reading
    /// takes one raw lifetime, as editions 2021 and 2024 do: in `'r#r"\"` the quote then opens
    /// an ordinary string in which `\"` is an escape. The reading of 0.8.3 takes the lifetime
    /// `'r`, a `#` and the raw string `r"\"`, as editions 2015 and 2018 do. Each shows the code
    /// of a text written for its editions and hides the code of a text written for the others:
    /// 0.8.3 found the first text below and missed the second. Checked with rustc 1.99.1: a
    /// macro counts `'r#fn` as three tokens in editions 2015 and 2018 and as one in 2021 and
    /// 2024, and of two files with these lines the first compiles in the older editions only
    /// and the second in the newer only.
    #[test]
    fn rust_raw_lifetime_each_reading_follows_its_editions() {
        let older = r#"m!('r#r"\"); unsafe { g() }; m!("");"#;
        let newer = r#"m!('r#r"\" /* "); unsafe { g() }; m!(" */ ");"#;
        assert_eq!(
            view(Language::Rust, older),
            r#"m!('r#r" "); unsafe { g() }; m!("");"#
        );
        assert_eq!(
            lexer_view(newer),
            r#"m!('r#r"      "); unsafe { g() }; m!("    ");"#
        );
        assert!(!has_token(&lexer_view(older), "unsafe"));
        assert!(!has_token(&view(Language::Rust, newer), "unsafe"));
        // The rules get both views of each text, so the block is found in each.
        for text in [older, newer] {
            let views = code_views(Language::Rust, text, &mut Readings::default());
            assert_eq!(views.len(), 2, "{text}");
            assert!(views.iter().any(|v| has_token(v, "unsafe")), "{text}");
        }
    }

    /// Three rules of the lexer reading, each held by its row: each was taken out in turn in a
    /// copy of this file, and this test then fails (for two of them it is the only one that does).
    /// Checked with rustc 1.99.1: a macro counts `r#abr"x"` as two tokens in every edition and
    /// `'r#r"x"` as two in editions 2021 and 2024; `b'a "x"'` is refused ("character literal
    /// may only contain one codepoint"), and the view keeps it as text.
    #[test]
    fn rust_raw_identifier_raw_lifetime_and_byte_literal_rules_hold() {
        for (line, expected, rule) in [
            (
                r#"m!(r#abr"\" unsafe "); unsafe"#,
                r#"m!(r#abr"          "); unsafe"#,
                "a raw identifier is a name to its end, so its last letters are no prefix",
            ),
            (
                r#"m!('r#r"\" unsafe "); unsafe"#,
                r#"m!('r#r"          "); unsafe"#,
                "a raw lifetime takes the name after its `#`",
            ),
            (
                r#"m!(b'a "x"'); unsafe"#,
                r#"m!(b'a "x"'); unsafe"#,
                "`b'` opens a byte literal and never a lifetime",
            ),
        ] {
            assert_eq!(lexer_view(line), expected, "{rule}: {line}");
        }
    }

    /// What does not depend on how `'r#name` is read: a character literal, a lifetime, a label,
    /// a lifetime before `#"…"#`, a name after the `#` that no quote follows, and a literal
    /// after such a name whose bytes are the same either way. (This test passes with the raw
    /// lifetime rule and without it.)
    #[test]
    fn rust_a_quote_and_an_r_read_the_same_with_and_without_raw_lifetimes() {
        for (line, expected) in [
            ("m!('r'); unsafe", "m!(   ); unsafe"),
            ("m!('r 'r: 'rx); unsafe", "m!('r 'r: 'rx); unsafe"),
            (r##"m!('r#"x"#); unsafe"##, r##"m!('r#" "#); unsafe"##),
            (
                "m!('r#fn 'r #fn 'r#1); unsafe",
                "m!('r#fn 'r #fn 'r#1); unsafe",
            ),
            (
                "m!('r#fn'x' 'r#b'y'); unsafe",
                "m!('r#fn    'r#b   ); unsafe",
            ),
            (
                r#"m!('r#b"\"" 'r#c"\""); unsafe"#,
                r#"m!('r#b"  " 'r#c"  "); unsafe"#,
            ),
        ] {
            assert_eq!(lexer_view(line), expected, "{line}");
        }
    }

    /// Two megabytes of each opener the Rust scanner knows, on one line and one to a line,
    /// scan in time linear in their length (a scan of them takes milliseconds; each has the
    /// deadline to itself).
    #[test]
    fn hostile_rust_lines_scan_in_linear_time() {
        for unit in [
            "/*", "'", "\"", "r#", "'\\u{", "b'", "r#\"", "1e", "é", "'a",
        ] {
            within_deadline(move || {
                let line = unit.repeat(2 * 1024 * 1024 / unit.len());
                assert!(lexer_view(&line).len() <= line.len(), "{unit}");
                let mut state = RustLexerState::default();
                for _ in 0..100_000 {
                    assert!(rust_lexer_view(unit, &mut state).len() <= unit.len());
                }
            });
        }
    }
    /// The carried reading of the shared scanner, which drops each reading's view before it
    /// makes the next, gives the view and the state [`code_view`] gives: 20,000 generated
    /// lines in six languages, read line after line with whatever the lines before left open.
    #[test]
    fn the_one_at_a_time_reading_is_the_shared_scanner_s() {
        const ATOMS: [&str; 16] = [
            "\"", "'", "`", "/*", "*/", "//", "#", "\"\"\"", "${", "}", "$(", ")", "x = 1", " ",
            "\\", "r#\"",
        ];
        let languages = [
            Language::Rust,
            Language::Python,
            Language::JavaScript,
            Language::Go,
            Language::Shell,
            Language::Yaml,
        ];
        let mut seed = 0x5EED_1234_ABCD_0001_u64;
        let mut next = move |below: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            usize::try_from(seed % (below as u64)).unwrap()
        };
        let mut several = 0;
        for language in languages {
            let (mut one, mut all) = (ScanState::default(), ScanState::default());
            for _ in 0..20_000 / languages.len() {
                let line: String = (0..=next(5)).map(|_| ATOMS[next(ATOMS.len())]).collect();
                several += usize::from(all.readings().count() > 1);
                assert_eq!(
                    code_view_one_at_a_time(language, &line, &mut one).0,
                    code_view(language, &line, &mut all),
                    "{language:?}: {line:?}"
                );
                assert_eq!(one, all, "{language:?}: {line:?}");
            }
        }
        assert!(several > 2000, "{several} lines read with several readings");
    }
    /// Readings that begin the same share one scan of the line, counted exactly, in the two
    /// views that unite their readings one at a time. The shared scanner's: two lines that
    /// each leave a string open give two readings that differ only in the string they carry;
    /// the next line is scanned once as code for both and once inside each string, three
    /// times where a scan for each reading would be four. The lexer's: after a line that
    /// leaves a string open and a line that closes it and, read as code, opens a raw string,
    /// one reading carries the raw string and one nothing; the first is also read as
    /// resuming none, which is where the second begins: two scans, not three.
    #[test]
    fn readings_that_begin_the_same_share_one_scan_of_the_line() {
        let mut state = ScanState::default();
        for line in ["a = \"x", "b = 'y"] {
            let _ = code_view(Language::Python, line, &mut state);
        }
        let carried: Vec<bool> = state.readings().map(|r| r.in_string.is_some()).collect();
        assert_eq!(
            carried,
            [true, true],
            "two readings, each carrying a string"
        );
        let each = state.readings().count() + carried.iter().filter(|c| **c).count();
        let (_, scans) = code_view_one_at_a_time(Language::Python, "c = 1", &mut state);
        assert_eq!((scans, each), (3, 4));

        let mut state = RustLexerState::default();
        for line in ["let a = \"x", "let b = r#\"y"] {
            let _ = rust::code_view_counted(line, &mut state);
        }
        let literal = state.readings().filter(|open| open.is_literal()).count();
        let each = state.readings().count() + literal;
        let (_, scans) = rust::code_view_counted("let b = 1;", &mut state);
        assert_eq!((scans, each, literal), (2, 3, 1));
    }

    /// One Rust line read whole from code ([`rust_views`] from [`RustStart::Code`]): the
    /// reading of a hunk side from a stated start, on one line.
    fn exact_view(line: &str) -> String {
        rust_views(&[line], RustStart::Code).remove(0)
    }

    /// A string, then a raw string that holds three comment openers on three lines, then code:
    /// one reading, in the state the compiler is in on every line. (While a line after an open
    /// string was read more than one way, each depth of comment that the reading without the
    /// string passed through was a reading of its own, and with room for four the compiler's
    /// was once the fifth and was dropped: the code after the raw string was blank. Found by
    /// the fuzz oracle in round 12; rustc 1.99.1 compiles the text, with `A` of 3 bytes and `B`
    /// of 11. There is nothing to drop any more.)
    #[test]
    fn a_rust_view_is_one_reading_however_its_literals_pile_up() {
        let views = rust_views(
            &[
                r#"pub const A: &str = ""#,
                r#"/*"; pub const B: &str = r##""#,
                "/*",
                " /*",
                "/*",
                r###""##; pub fn f() { unsafe { run() } }"###,
            ],
            RustStart::Code,
        );
        assert_eq!(
            views,
            [
                r#"pub const A: &str = ""#,
                r#"  "; pub const B: &str = r##""#,
                "  ",
                "   ",
                "  ",
                r###""##; pub fn f() { unsafe { run() } }"###,
            ]
        );
    }

    /// A side of a hunk is read whole from a stated start (fifth independent review, IR5-H-1,
    /// and round 12 of Vesper's, VD3R12-M-1). From code it is the compiler's reading of that
    /// text. From inside one block comment it is the compiler's reading of the same text with
    /// `/*` put before it: a line `// */ call(); /*` inside a comment is a call, which the
    /// reading from code takes for a line comment. Until 2026-11-14 a hunk was read from code
    /// only, and the call was hidden (rustc 1.99.1 compiles and runs it).
    #[test]
    fn a_rust_side_is_read_whole_from_its_start() {
        let lines = [
            "   over several lines of an existing block comment.",
            r#"   // */ pub fn evil(u: &str) { spawn("sh"); } /*"#,
            "   More prose of the same comment follows,",
            "   and it ends here. */ fn after() {}",
        ];
        assert_eq!(
            rust_views(&lines, RustStart::Comment),
            [
                "                                                   ",
                r#"         pub fn evil(u: &str) { spawn("  "); }   "#,
                "                                          ",
                "                        fn after() {}",
            ]
        );
        assert_eq!(
            rust_views(&lines, RustStart::Code),
            [
                "   over several lines of an existing block comment.",
                "   ",
                "   More prose of the same comment follows,",
                "   and it ends here. */ fn after() {}",
            ]
        );
        // What "inside one block comment" means: `/*` before the text, read from code.
        let mut opened: Vec<String> = lines.iter().map(ToString::to_string).collect();
        opened[0].insert_str(0, "/*");
        let opened: Vec<&str> = opened.iter().map(String::as_str).collect();
        let mut from_code = rust_views(&opened, RustStart::Code);
        from_code[0].drain(..2);
        assert_eq!(rust_views(&lines, RustStart::Comment), from_code);
        // Inside two comments the first `*/` ends only one of them. Read as inside one, what
        // follows it is shown as code, earlier than the compiler shows it. (That reading is
        // then wrong, and what it shows can hide code further on: no start is made for a
        // side two comments deep.)
        assert_eq!(
            rust_views(&["a /* b */ c */ d", "e */ f"], RustStart::Comment),
            ["               d", "e */ f"]
        );
        // From line 1 and from below it, text with no shebang reads the same; and line by
        // line through `code_view` it is the reading from code.
        assert_eq!(
            rust_views(&lines, RustStart::FileStart),
            rust_views(&lines, RustStart::Code)
        );
        assert_eq!(rust_views(&[], RustStart::Comment), Vec::<String>::new());
    }

    /// A side read from inside an ordinary string is the same text with `"` put before it,
    /// read from code: its first quote closes the string. And a side read from code says
    /// whether it ends inside a string or a raw string, which is the mark of one that began
    /// in a string (its closing quote was taken for an opener). Before 2026-11-14 a line
    /// after an open string was read both ways and the bytes united; with whole views a
    /// side that began inside a string would be read wrongly to its end, so it gets a view
    /// of its own when its quotes say so.
    #[test]
    fn a_rust_side_is_read_from_inside_a_string_and_says_how_it_ends() {
        let lines = [
            "        VALUES",
            r#"            (?1, ?2, now())", params![name, age]).unwrap();"#,
            "}",
            r#"pub fn count() -> u64 { query("SELECT 1 /* one */") }"#,
        ];
        assert_eq!(
            rust_views(&lines, RustStart::Str),
            [
                "              ",
                r#"                           ", params![name, age]).unwrap();"#,
                "}",
                r#"pub fn count() -> u64 { query("                  ") }"#,
            ]
        );
        let mut opened: Vec<String> = lines.iter().map(ToString::to_string).collect();
        opened[0].insert(0, '"');
        let opened: Vec<&str> = opened.iter().map(String::as_str).collect();
        let mut from_code = rust_views(&opened, RustStart::Code);
        from_code[0].remove(0);
        assert_eq!(rust_views(&lines, RustStart::Str), from_code);
        // Read from code this side ends inside a string (the last quote of line 4 opens
        // one); read from inside a string it does not.
        assert!(rust_side(&lines, RustStart::Code).ends_in_string);
        assert!(!rust_side(&lines, RustStart::Str).ends_in_string);
        assert_eq!(
            rust_side(&lines, RustStart::Code).views,
            rust_views(&lines, RustStart::Code)
        );
        for (text, ends) in [
            (&[r#"let s = "open"#][..], true),
            (&[r#"let s = r#"open""#], true),
            (&[r#"let s = b"open"#, "still"], true),
            (&[r#"let s = "closed";"#], false),
            (&[r#"/* a comment " left open"#], false),
            (&["let c = '("], false),
            (&[r"m!('(\"], false),
            (&[], false),
        ] {
            assert_eq!(
                rust_side(text, RustStart::Code).ends_in_string,
                ends,
                "{text:?}"
            );
        }
        // Inside a string a backslash takes a quote or a backslash with it, as from code.
        assert_eq!(
            rust_views(&[r#"a \" b \\" unsafe"#], RustStart::Str),
            [r#"         " unsafe"#]
        );
    }

    /// The compiler passes over a shebang: line 1 of a file, when it starts with `#!` and the
    /// first token after that which is neither white space nor a comment is not `[`. (With
    /// `[` it is the inner attribute `#![…]`, also over lines.) A doc comment is a token
    /// there; a byte order mark before the `#!` is dropped first. Checked with rustc 1.99.1:
    /// each text below compiles with the reading the test asserts and not with the other
    /// one. Until 2026-11-14 a shebang was read as Rust, and `#!/usr/bin/env -S cargo run /*`
    /// opened a comment over the file (round 12 of Vesper's review, VD3R12-M-2).
    #[test]
    fn a_rust_shebang_line_is_passed_over_as_the_compiler_passes_over_it() {
        let top = |lines: &[&str]| rust_views(lines, RustStart::FileStart);
        let blank = |line: &str| " ".repeat(line.len());
        // Shebangs: the line is blank and leaves nothing open.
        for lines in [
            &["#!/usr/bin/env -S cargo run /*", "fn main() { spawn(); }"][..],
            &["#! /* a */ // b", "fn main() { spawn(); }"],
            &[
                "#!/** doc */ [allow(dead_code)] \"",
                "fn main() { spawn(); }",
            ],
            &["#!//! doc [", "fn main() { spawn(); }"],
            &["#!//! doc", "[allow(dead_code)]"],
            &["#!/// doc", "[allow(dead_code)]"],
            &["#!/*! doc */", "[allow(dead_code)]"],
            &["\u{feff}#!/usr/bin/env run /* \"", "fn main() { spawn(); }"],
            &[
                "#!/* open",
                "   still */ x \"",
                "fn main() { spawn(); } // \"",
            ],
            &["#!"],
            &["#!", "   ", "// only a comment"],
            &["#! /* a comment that the lines shown do not end"],
        ] {
            let views = top(lines);
            assert_eq!(views[0], blank(lines[0]), "{lines:?}");
            assert_eq!(
                views[1..],
                rust_views(&lines[1..], RustStart::Code),
                "{lines:?}"
            );
        }
        assert_eq!(
            top(&["#!/usr/bin/env -S cargo run /*", "fn main() { spawn(); }"])[1],
            "fn main() { spawn(); }"
        );
        // Not shebangs: read as the Rust they are.
        for lines in [
            &["#![allow(dead_code)]", "fn main() {}"][..],
            &["#! [allow(dead_code)] /*", "*/ fn main() {}"],
            &["#!", "[allow(dead_code)]"],
            &["#! /* c */ [allow(dead_code)]"],
            &["#!// plain comment \"", "[allow(dead_code)]"],
            &[
                "#!//// four slashes are a plain comment",
                "[allow(dead_code)]",
            ],
            &["#!/* open", "   still */ [allow(dead_code)]"],
            &["#!/* a /* b", "*/ still */ [allow(dead_code)]"],
            &["#!/**/ [allow(dead_code)]"],
            &["#!/*** three stars are a plain comment */ [allow(dead_code)]"],
            &[" #!/usr/bin/env run /*", "fn main() {}"],
            &["// first", "#!/usr/bin/env run /*", "fn main() {}"],
        ] {
            assert_eq!(top(lines), rust_views(lines, RustStart::Code), "{lines:?}");
        }
        assert_eq!(
            top(&["#!", "[allow(dead_code)] fn main() {}"]),
            ["#!", "[allow(dead_code)] fn main() {}"]
        );
        // The rule is for line 1 of a file: a side that starts below it is not asked.
        assert_eq!(
            rust_views(&["#!/usr/bin/env run /*", "fn main() {}"], RustStart::Code),
            ["#!/usr/bin/env run   ", "            "]
        );
    }

    /// Rules of the scanner that no unit test held (fifth independent review, IR5-L-1: each was
    /// removed in turn and the tests still passed; only the fuzz target would have noticed).
    /// Each line below reads otherwise without its rule. What the compiler does with each is
    /// the lexer's reading; rustc 1.99.1 refuses most of them later, as noted.
    #[test]
    fn rust_scanner_rules_the_mutation_run_found_unheld() {
        for (line, expected, rule) in [
            // rustc: `r#abr` is one raw identifier (a macro counts `r#abr"x"` as two tokens)
            (
                r#"m!(r#abr"\" unsafe "); unsafe"#,
                r#"m!(r#abr"          "); unsafe"#,
                "a raw identifier is a name to its end, so its last letters are no prefix",
            ),
            // rustc, editions 2021 and 2024: `'r#r` is one raw lifetime, then a string
            (
                r#"m!('r#r"\" unsafe "); unsafe"#,
                r#"m!('r#r"          "); unsafe"#,
                "a raw lifetime takes the name after its `#`",
            ),
            // rustc: "lifetimes cannot start with a number", and reads on after it
            (
                r#"m!('1 "x"); unsafe"#,
                r#"m!('1 " "); unsafe"#,
                "a digit after a quote starts a lifetime, as a letter does",
            ),
            // rustc: "identifiers cannot contain emoji"
            (
                r#"m!('😀 "x"); unsafe"#,
                r#"m!('😀 " "); unsafe"#,
                "an emoji after a quote starts a lifetime",
            ),
            (
                r#"m!('a😀' "x"); unsafe"#,
                r#"m!('a😀' " "); unsafe"#,
                "an emoji goes on in a name, so the quote after it closes what the first opened",
            ),
            // rustc: "character literal may only contain one codepoint"; kept as text
            (
                r#"m!(b'a "x"'); unsafe"#,
                r#"m!(b'a "x"'); unsafe"#,
                "`b'` opens a byte literal and never a lifetime",
            ),
            // rustc: "found invalid character; only `#` is allowed in raw string delimitation"
            (
                r#"m!(r#→"x"); unsafe"#,
                r#"m!(r#→" "); unsafe"#,
                "`r#` and a character that is no quote take that whole character",
            ),
            (
                r#"m!(r#'x' "y"); unsafe"#,
                r#"m!(r#'x' "y"); unsafe"#,
                "the character after `r#` is taken into that token: this quote opens nothing",
            ),
            (
                "m!(r#/* unsafe */); unsafe",
                "m!(r#/* unsafe */); unsafe",
                "the character after `r#` is taken into that token: this `/` opens no comment",
            ),
            // rustc: "expected at least one digit in exponent"; the `r` is its suffix
            (
                r#"m!(0_1e+r"\" unsafe "); unsafe"#,
                r#"m!(0_1e+r"          "); unsafe"#,
                "after a first `0`, `_` goes on with the digits",
            ),
        ] {
            assert_eq!(exact_view(line), expected, "{rule}: {line}");
        }
        // A raw string takes a suffix, so an `r` right after it is no prefix; but not one
        // with more than 255 `#` (rustc: "too many `#` symbols"), after which the `r` opens
        // a raw string, ` unsafe ` is code, and the last quote opens a string.
        for (hashes, after) in [
            (255, r#"r"          "); unsafe"#),
            (256, r#"r" " unsafe "         "#),
        ] {
            let fence = "#".repeat(hashes);
            let line = format!(r#"m!(r{fence}"x"{fence}r"\" unsafe "); unsafe"#);
            let expected = format!(r#"m!(r{fence}" "{fence}{after}"#);
            assert_eq!(exact_view(&line), expected, "{hashes} hashes");
        }
    }

    /// The compiler's white space: six ASCII characters and five past ASCII, each by its
    /// length in bytes. One definition, used by the reader of tokens, by the raw-identifier
    /// rule and by the shebang rule (fifth independent review, IR5-M-2).
    #[test]
    fn rust_white_space_is_the_compilers() {
        for c in ['\t', '\n', '\u{b}', '\u{c}', '\r', ' '] {
            assert_eq!(rust_white(format!("{c}x").as_bytes()), 1, "{c:?}");
        }
        for (c, length) in [
            ('\u{85}', 2),
            ('\u{200e}', 3),
            ('\u{200f}', 3),
            ('\u{2028}', 3),
            ('\u{2029}', 3),
        ] {
            assert_eq!(rust_white(format!("{c}x").as_bytes()), length, "{c:?}");
            assert_eq!(rust_white(c.to_string().as_bytes()), length, "{c:?}");
        }
        // Other space-like characters are not white space to the compiler.
        for c in [
            'x', '\u{a0}', '\u{84}', '\u{86}', '\u{200d}', '\u{2027}', '\u{202a}', '\u{feff}',
        ] {
            assert_eq!(rust_white(c.to_string().as_bytes()), 0, "{c:?}");
        }
        assert_eq!(rust_white(b""), 0);
        assert_eq!(rust_white(&[0xE2, 0x80]), 0);
    }
}
