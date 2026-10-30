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
//! - An interpolation is scanned as code with its own strings, escapes and comments, and ends
//!   at its matching closer on the same line. Shell `case` patterns, ANSI-C `$'…'` strings,
//!   word-start `#` comments, JS regex literals, Python format specs and nested triple quotes
//!   are known.
//! - A string still open at the end of a line (outside Rust) is read as code from its quote.
//! - A construct the scanner does not know (a raw string, a heredoc) is read as code. A
//!   triple-quoted f-string's interpolations are blanked with the rest of it, a continuation
//!   line of a multi-line string is read as code, and non-ASCII spellings of a token
//!   (Python's NFKC `ｅval`) are not matched.
//!
//! Evidence built on it says `basis: lexical`.

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
    /// YAML (CI workflows): only `#` comments are blanked; quoted scalars are kept.
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

    /// Whether `byte` opens a one-line string.
    const fn opens_string(self, byte: u8) -> bool {
        match self {
            Self::Yaml => false,
            Self::Rust => byte == b'"',
            Self::Go | Self::Python | Self::Shell => byte == b'"' || byte == b'\'',
            Self::JavaScript => matches!(byte, b'"' | b'\'' | b'`'),
        }
    }
}

/// What a multi-line construct left open at the end of the previous line of the same hunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ScanState {
    in_block_comment: bool,
    /// The quote character of an open Python triple-quoted string.
    in_triple: Option<u8>,
}

/// One level of what is open on a line, innermost last on [`code_view`]'s stack. An empty stack
/// is plain code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Frame {
    /// A string opened at byte `start` by `quote` (three of them when `triple`): its contents
    /// are blanked. An f- or t-string (`fstring`) runs `{…}`; a shell ANSI-C string (`$'…'`,
    /// `ansi`) honours backslashes.
    Str {
        quote: u8,
        start: usize,
        triple: bool,
        fstring: bool,
        ansi: bool,
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

/// Whether `word` starts at byte `i` of `bytes` as a whole word.
fn word_at(bytes: &[u8], i: usize, word: &[u8]) -> bool {
    bytes[i..].starts_with(word)
        && (i == 0 || !is_ident(bytes[i - 1]))
        && bytes.get(i + word.len()).is_none_or(|b| !is_ident(*b))
}

/// Whether a shell word at byte `i` is in command position: first on the line or in its
/// substitution, or after `;`, `|`, `&`, `(`, `{`, a backtick, `then`, `do` or `else`. Only
/// there does `case` start a statement (`echo case` and `--ignore-case` do not).
fn in_command_position(bytes: &[u8], i: usize) -> bool {
    let before = bytes[..i].trim_ascii_end();
    let after_word = |word: &[u8]| {
        before.ends_with(word)
            && before
                .len()
                .checked_sub(word.len() + 1)
                .is_none_or(|k| !is_ident(before[k]))
    };
    matches!(
        before.last(),
        None | Some(b';' | b'|' | b'&' | b'(' | b'{' | b'`')
    ) || after_word(b"then")
        || after_word(b"do")
        || after_word(b"else")
}

/// Whether a JS `/` here starts a regex literal rather than a division: the last code byte
/// before it is an operator or an opening bracket (or there is none), as in the language's
/// grammar.
fn starts_regex(out: &[u8]) -> bool {
    out.iter()
        .rev()
        .find(|b| !b.is_ascii_whitespace())
        .is_none_or(|b| b"(,=:[!&|?{};".contains(b))
}

/// The length of a JS regex literal at `rest` (which starts with its `/`), up to and including
/// its closing `/`, or `None` when the line holds no closer.
fn regex_len(rest: &[u8]) -> Option<usize> {
    let mut class = false;
    let mut j = 1;
    while j < rest.len() {
        match rest[j] {
            b'\\' => j += 1,
            b'[' => class = true,
            b']' => class = false,
            b'/' if !class => return Some(j + 1),
            _ => {}
        }
        j += 1;
    }
    None
}

/// The code bytes of `line`: comments and string contents replaced by spaces, the quotes kept,
/// so token positions stay where they were. Code inside an interpolation is scanned as code,
/// with its own strings, escapes and comments, so a closer inside a nested string or after a
/// backslash does not end it. When a desync is possible, the scan errs toward code: a string
/// still open at the end of the line (outside Rust, whose strings span lines) is read as code
/// from its quote on, so a quote the scanner misread cannot hide what follows it. `state`
/// carries an open block comment or triple string to the next line.
#[must_use]
pub fn code_view(language: Language, line: &str, state: &mut ScanState) -> String {
    let bytes = line.as_bytes();
    let mut scan = Scan {
        language,
        bytes,
        out: Vec::with_capacity(bytes.len()),
        stack: Vec::new(),
        blank: 0,
        copy: 0,
    };
    for i in 0..bytes.len() {
        if !scan.step(i, state) {
            break;
        }
    }
    scan.finish()
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
}

impl Scan<'_> {
    /// Scan byte `i`; `false` when the rest of the line is a comment.
    fn step(&mut self, i: usize, state: &mut ScanState) -> bool {
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
            escaped,
        };
        if escaped {
            self.stack[top] = with_escape(false);
        } else if byte == quote && (!triple || rest.starts_with(&[quote; 3])) {
            self.stack.pop();
            self.copy = if triple { 2 } else { 0 };
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
            && !(self.language == Language::Shell && quote == b'\'' && !ansi)
            && !(fstring && rest.get(1) == Some(&b'{'))
        {
            // A backslash escapes the next byte, except in a plain single-quoted shell string
            // and before an f-string's `{`, which Python still runs.
            self.stack[top] = with_escape(true);
        }
        self.out.push(b' ');
    }

    /// A byte of code: at the top of the line or inside an interpolation. `false` when the rest
    /// of the line is a comment.
    fn in_code(&mut self, i: usize, state: &mut ScanState) -> bool {
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
            && starts_regex(&self.out)
            && let Some(len) = regex_len(rest)
        {
            // A regex literal's text is data: its quotes and brackets open nothing.
            self.blank = len - 2;
            self.copy = 1;
            self.out.push(byte);
            return true;
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
        // JSX text (`<p>Can't</p>`) puts apostrophes there.
        let after_word =
            language == Language::JavaScript && i > 0 && is_ident(bytes[i - 1]) && byte != b'`';
        if language.opens_string(byte) && !after_word {
            self.stack.push(Frame::Str {
                quote: byte,
                start: i,
                triple: false,
                fstring: language == Language::Python && is_interpolating_prefix(&bytes[..i]),
                ansi: language == Language::Shell && byte == b'\'' && i > 0 && bytes[i - 1] == b'$',
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
        if language == Language::Shell && in_command_position(bytes, i) {
            if word_at(bytes, i, b"case") {
                *cases += 1;
            } else if word_at(bytes, i, b"esac") {
                *cases = cases.saturating_sub(1);
            }
        }
        if language == Language::Python {
            match byte {
                b'(' | b'[' => *brackets += 1,
                b')' | b']' => *brackets = brackets.saturating_sub(1),
                // A top-level `:` (not `:=`) starts the format spec.
                b':' if *depth == 1 && *brackets == 0 && bytes.get(i + 1) != Some(&b'=') => {
                    *spec = true;
                }
                _ => {}
            }
        }
        if *open == Some(byte) {
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
    fn finish(mut self) -> String {
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
        Language::Shell => word_start || b";&|()".contains(&bytes[i - 1]),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn view(language: Language, line: &str) -> String {
        code_view(language, line, &mut ScanState::default())
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
        assert!(state.in_block_comment);
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
}
