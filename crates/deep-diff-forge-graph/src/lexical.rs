// DDF integration: line-lexical code view for evidence signals (plan 2026-10-30, Feature A).
// Design/status: docs/PLAN_AGENT_EVIDENCE_AND_OBSERVABILITY.md#feature-a-code-evidence-risk-signals
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fcrates--deep-diff-forge-graph--src--lexical
//! A line-lexical view of source code: what is left of a line once its comments and string
//! contents are blanked. Evidence rules match tokens in that view, so a keyword in a comment or
//! a string is not read as code. Code that runs inside a string is kept: `$(…)` and backticks
//! in a shell double-quoted string, `${…}` in a JS template literal and `{…}` in a one-line
//! Python f-string, so quoting a fetch or a spawn does not hide it.
//!
//! This is lexical, not parsing. Block comments and triple-quoted strings are followed across
//! the lines of one hunk, in order ([`ScanState`]), but a hunk that starts inside one is read
//! as code, and a construct the scanner does not know (a raw string, a heredoc) is read as
//! code too. An interpolation is scanned as code with its own strings, escapes and comments,
//! and ends at its matching closer on the same line; a triple-quoted f-string's interpolations
//! are blanked with the rest of it. Evidence built on it says `basis: lexical`.

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

    /// `#` comments; `//` is code (Python floor division, a shell or URL path).
    const fn hash_comments(self) -> bool {
        matches!(self, Self::Python | Self::Shell | Self::Yaml)
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
    /// A string opened by `quote`: its contents are blanked. An f-string (`fstring`) runs `{…}`.
    Str {
        quote: u8,
        fstring: bool,
        escaped: bool,
    },
    /// Code inside an interpolation, until `close` brings `depth` back to zero. `open` nests
    /// another level (`(` in `$(`), `None` for a backtick span; `cases` counts open shell `case`
    /// statements, whose pattern `)` is not a closer.
    Code {
        open: Option<u8>,
        close: u8,
        depth: u32,
        cases: u32,
    },
    /// A JS block comment inside an interpolation, until `*/`.
    Comment,
}

impl Frame {
    /// The interpolation that opens at `rest` inside a string opened by `quote` (an f-string
    /// when `fstring`), and the length of its opener.
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
                },
                len,
            ))
        };
        match (language, quote) {
            (Language::Shell, b'"') if rest.starts_with(b"$(") => code(Some(b'('), b')', 2),
            (Language::Shell, b'"') if rest.first() == Some(&b'`') => code(None, b'`', 1),
            (Language::JavaScript, b'`') if rest.starts_with(b"${") => code(Some(b'{'), b'}', 2),
            (Language::Python, _) if fstring && rest.first() == Some(&b'{') => {
                code(Some(b'{'), b'}', 1)
            }
            _ => None,
        }
    }
}

/// Whether `word` starts at byte `i` of `bytes` as a whole word.
fn word_at(bytes: &[u8], i: usize, word: &[u8]) -> bool {
    bytes[i..].starts_with(word)
        && (i == 0 || !is_ident(bytes[i - 1]))
        && bytes.get(i + word.len()).is_none_or(|b| !is_ident(*b))
}

/// Whether the bytes just before a Python quote are an f-string prefix (`f`, `rf`, `Fr`, ...):
/// at most two prefix letters, one of them `f`, not the end of an identifier (`elif"`).
fn is_f_prefix(before: &[u8]) -> bool {
    let start = before
        .iter()
        .rposition(|b| !b"fFrRbBuU".contains(b))
        .map_or(0, |p| p + 1);
    let prefix = &before[start..];
    prefix.len() <= 2
        && prefix.iter().any(|b| matches!(b, b'f' | b'F'))
        && (start == 0 || !is_ident(before[start - 1]))
}

/// The code bytes of `line`: comments and string contents replaced by spaces, the quotes kept,
/// so token positions stay where they were. Code inside an interpolation is scanned as code, with
/// its own strings, escapes and comments, so a closer inside a nested string or after a
/// backslash does not end it. `state` carries an open block comment or triple string to the
/// next line.
#[must_use]
pub fn code_view(language: Language, line: &str, state: &mut ScanState) -> String {
    let bytes = line.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    // One pass, one byte per step. A decision about a multi-byte token (`/*`, `*/`, `"""`, a
    // char literal, an opener) sets how many of the next bytes to blank or copy, so no step can
    // stall.
    let mut blank = 0usize;
    let mut copy = 0usize;
    let mut stack: Vec<Frame> = Vec::new();
    for (i, &byte) in bytes.iter().enumerate() {
        if blank > 0 {
            blank -= 1;
            out.push(b' ');
            continue;
        }
        if copy > 0 {
            copy -= 1;
            out.push(byte);
            continue;
        }
        let rest = &bytes[i..];
        if state.in_block_comment {
            if rest.starts_with(b"*/") {
                state.in_block_comment = false;
                blank = 1;
            }
            out.push(b' ');
            continue;
        }
        if let Some(triple) = state.in_triple {
            if rest.starts_with(&[triple; 3]) {
                state.in_triple = None;
                copy = 2;
                out.push(byte);
            } else {
                out.push(b' ');
            }
            continue;
        }
        match stack.last().copied() {
            Some(Frame::Comment) => {
                if rest.starts_with(b"*/") {
                    stack.pop();
                    blank = 1;
                }
                out.push(b' ');
                continue;
            }
            Some(Frame::Str {
                quote,
                fstring,
                escaped,
            }) => {
                let top = stack.len() - 1;
                if escaped {
                    stack[top] = Frame::Str {
                        quote,
                        fstring,
                        escaped: false,
                    };
                } else if byte == quote {
                    stack.pop();
                    out.push(byte);
                    continue;
                } else if fstring && rest.starts_with(b"{{") {
                    // A doubled brace is a literal brace, not an interpolation.
                    blank = 1;
                } else if let Some((frame, len)) =
                    Frame::interpolation(language, quote, fstring, rest)
                {
                    stack.push(frame);
                    copy = len - 1;
                    out.push(byte);
                    continue;
                } else if byte == b'\\'
                    && !(language == Language::Shell && quote == b'\'')
                    && !(fstring && rest.get(1) == Some(&b'{'))
                {
                    // A backslash escapes the next byte, except in a single-quoted shell string
                    // and before an f-string's `{`, which Python still runs.
                    stack[top] = Frame::Str {
                        quote,
                        fstring,
                        escaped: true,
                    };
                }
                out.push(b' ');
                continue;
            }
            Some(Frame::Code { .. }) | None => {}
        }
        let nested = !stack.is_empty();
        if nested && byte == b'\\' {
            // An escaped byte inside an interpolation is never its closer.
            copy = 1;
            out.push(byte);
            continue;
        }
        if language.c_like() && rest.starts_with(b"//") {
            break;
        }
        if language.c_like() && rest.starts_with(b"/*") {
            if nested {
                stack.push(Frame::Comment);
            } else {
                state.in_block_comment = true;
            }
            blank = 1;
            out.push(b' ');
            continue;
        }
        if !nested && language.hash_comments() && byte == b'#' {
            break;
        }
        if !nested
            && language == Language::Python
            && (byte == b'"' || byte == b'\'')
            && rest.starts_with(&[byte; 3])
        {
            state.in_triple = Some(byte);
            copy = 2;
            out.push(byte);
            continue;
        }
        // A Rust char literal (`'"'`, `'\''`) is not a string start; a lifetime (`'a`) has no
        // closing quote two or three bytes on, so it is left as code.
        if language == Language::Rust && byte == b'\'' {
            let close = if rest.get(1) == Some(&b'\\') { 3 } else { 2 };
            if rest.get(close) == Some(&b'\'') {
                blank = close;
                out.push(b' ');
                continue;
            }
        }
        if language.opens_string(byte) {
            stack.push(Frame::Str {
                quote: byte,
                fstring: language == Language::Python && is_f_prefix(&bytes[..i]),
                escaped: false,
            });
            out.push(byte);
            continue;
        }
        if let Some(Frame::Code {
            open,
            close,
            depth,
            cases,
        }) = stack.last_mut()
        {
            if language == Language::Shell {
                if word_at(bytes, i, b"case") {
                    *cases += 1;
                } else if word_at(bytes, i, b"esac") {
                    *cases = cases.saturating_sub(1);
                }
            }
            if *open == Some(byte) {
                *depth += 1;
            } else if byte == *close && !(byte == b')' && *cases > 0 && *depth == 1) {
                // A `)` while a `case` is open ends a pattern, not the substitution.
                *depth -= 1;
                if *depth == 0 {
                    stack.pop();
                }
            }
        }
        out.push(byte);
    }
    // Only ASCII bytes are replaced, by ASCII, and the scan stops only at an ASCII comment
    // start, so the view is valid UTF-8 wherever the line was.
    String::from_utf8(out).unwrap_or_default()
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
        assert_eq!(view(Language::Shell, r#""${curl}""#), r#""       ""#);
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
    fn f_string_prefixes_are_exact() {
        for yes in ["f", "F", "rf", "fR", "x = f", "(Rf"] {
            assert!(is_f_prefix(yes.as_bytes()), "{yes}");
        }
        for no in ["", "r", "b", "rb", "elif", "buf", "fff", "x_f", "9f"] {
            assert!(!is_f_prefix(no.as_bytes()), "{no}");
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
