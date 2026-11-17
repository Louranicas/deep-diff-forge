// The scanner of the released version 0.8.3, kept to read Rust beside the scanner that follows
// the compiler's lexer (`super::rust`).
//! The code view of release 0.8.3, as released.
//!
//! Everything below the imports is `lexical.rs` of commit `d4dbc91` (the commit of the signed tag
//! `v0.8.3`), its lines 119 to 841, byte for byte: the scanner of every language as it was, with
//! its state, its further readings and its resumed strings. It is kept for ONE use, and only
//! for Rust: a Rust text is read once more through this view by the rules release 0.8.3 had
//! (`evidence::legacy_hits`), and what they find is united with what the readings of the
//! compiler's lexer find. So no patch has an item of a released signal in 0.8.3 and none here,
//! whatever the two scanners make of its text.
//!
//! Why that is wanted. The new scanner is right about text read from a known start, and a hunk
//! does not have one. From a start that is wrong, being exact goes wrong differently: it nests
//! block comments, so `crates/*/*/Cargo.toml` in the text of a string the hunk began in opens
//! two comments and closes one, and the rest of the hunk is comment, where this scanner ended
//! the comment at the first `*/` and showed the code below (review of the port of the new
//! scanner to the released line, LF-H-1 and LF-M-1, 2026-11-14).
//!
//! Do not repair or tidy this file: it is a record. The other languages are not read through
//! it (they are read by the shared scanner in `lexical.rs`, which is this code less its Rust
//! branches), and nothing here is reachable except through [`code_view`] with
//! [`Language::Rust`].

use super::{Language, is_ident};

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
        Some(_) => [
            &b"then"[..],
            b"do",
            b"else",
            b"if",
            b"elif",
            b"while",
            b"until",
            b"time",
        ]
        .iter()
        .any(|word| ends_with_word(before, word)),
    }
}

/// Whether a shell `(` at byte `i` opens a case pattern (`case x in (a) …`) rather than a
/// subshell: it follows the `in` of a case or the `;;` or `;&` that ends an arm.
fn opens_case_pattern(bytes: &[u8], i: usize) -> bool {
    let before = bytes[..i].trim_ascii_end();
    before.ends_with(b";;") || before.ends_with(b";&") || ends_with_word(before, b"in")
}

/// JS words after which an expression starts, so a `/` there opens a regex and a quote right
/// after one opens a string (`return"x"`, `typeof'y'`).
const JS_OPERAND_KEYWORDS: [&[u8]; 14] = [
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

/// The identifier that ends `before`, if any.
fn trailing_word(before: &[u8]) -> &[u8] {
    let start = before
        .iter()
        .rposition(|b| !is_ident(*b))
        .map_or(0, |p| p + 1);
    &before[start..]
}

/// Whether `before` ends with a [`JS_OPERAND_KEYWORDS`] word that is not a property name
/// (`x.return`).
fn ends_with_operand_keyword(before: &[u8]) -> bool {
    let word = trailing_word(before);
    JS_OPERAND_KEYWORDS.contains(&word) && !before[..before.len() - word.len()].ends_with(b".")
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
