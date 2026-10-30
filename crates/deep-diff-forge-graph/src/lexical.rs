// DDF integration: line-lexical code view for evidence signals (plan 2026-10-30, Feature A).
// Design/status: docs/PLAN_AGENT_EVIDENCE_AND_OBSERVABILITY.md#feature-a-code-evidence-risk-signals
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fcrates--deep-diff-forge-graph--src--lexical
//! A line-lexical view of source code: what is left of a line once its comments and string
//! contents are blanked. Evidence rules match tokens in that view, so a keyword in a comment or
//! a string is not read as code.
//!
//! This is lexical, not parsing. Block comments and triple-quoted strings are followed across
//! the lines of one hunk, in order ([`ScanState`]), but a hunk that starts inside one is read
//! as code, and a construct the scanner does not know (a raw string, a heredoc) is read as
//! code too. Evidence built on it says `basis: lexical`.

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

/// The code bytes of `line`: comments and string contents replaced by spaces, the quotes kept,
/// so token positions stay where they were. `state` carries an open block comment or triple
/// string to the next line.
#[must_use]
pub fn code_view(language: Language, line: &str, state: &mut ScanState) -> String {
    let bytes = line.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    // One pass, one byte per step. A decision about a multi-byte token (`/*`, `*/`, `"""`, a
    // char literal) sets how many of the next bytes to blank or copy, so no step can stall.
    let mut blank = 0usize;
    let mut copy = 0usize;
    let mut quote: Option<u8> = None;
    let mut escaped = false;
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
        if let Some(open) = quote {
            if escaped {
                escaped = false;
            } else if byte == open {
                quote = None;
                out.push(byte);
                continue;
            } else if byte == b'\\' && !(language == Language::Shell && open == b'\'') {
                // A backslash escapes the next byte, except in a single-quoted shell string.
                escaped = true;
            }
            out.push(b' ');
            continue;
        }
        if language.c_like() && rest.starts_with(b"//") {
            break;
        }
        if language.c_like() && rest.starts_with(b"/*") {
            state.in_block_comment = true;
            blank = 1;
            out.push(b' ');
            continue;
        }
        if language.hash_comments() && byte == b'#' {
            break;
        }
        if language == Language::Python
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
            quote = Some(byte);
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
        assert_eq!(view(Language::JavaScript, "`${eval(x)}`"), "`          `");
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
