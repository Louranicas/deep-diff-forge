#![no_main]
// The Rust code view against the compiler's own lexer (review round 12, 2026-11-14).
//
// Every Rust rule reads a view of each line in which comments and literals are blank
// (`lexical::code_view`). Twice the view blanked text the compiler compiles: after a nested
// block comment, and after a character literal with a long escape. Both hid code from every
// rule, in released versions. This target holds the view to the lexer the compiler uses
// (`rustc_lexer`, as published for rust-analyzer: `ra-ap-rustc_lexer`), for any text read as a
// whole Rust file from its first line.
//
// The oracle reads the text as the compiler does: the lexer's tokens, and where the lexer
// offers `#"` or `##` as one token (reserved in edition 2024, which refuses it), a `#` and the
// rest read again, as the compiler does in every edition that accepts the text. A C string is
// one token (edition 2021 and later). The view gets the same text one line at a time, cut at
// `\n` only, as a hunk gives it.
//
// SAFETY, for every input: a byte of a token that is code stands unchanged in the view. Code
// is every token but a comment and a string, byte string, C string, raw string, character or
// byte literal. Held more strictly than that: a literal's suffix is code, and so is the token
// the lexer makes of `r#` and a character that is no quote (it is no string).
//
// PRECISION, where the view has one reading: a byte of a comment, or between the quotes of a
// closed literal, is blank (or past the end of a view that a line comment cut). It is checked
// on every line up to the first on which a literal runs over the line end, and after that on
// every line the scanner reaches with nothing carried. It is not checked elsewhere, by design:
// a hunk may start inside a string, so once a literal has run over a line end the view keeps
// what any reading of the lines after it finds code. And of what the lexer closes as a
// character or byte literal, only what can be one is blank: one character between the quotes,
// or text that starts with a backslash, on one line. `'ab'`, `''` and a literal over a line
// end, which the lexer closes and the compiler refuses, are kept as text.
//
// UNTERMINATED tokens: an unterminated block comment is a comment to the end of the text; an
// unterminated string or raw string is a literal to the end of the text (blank on its first
// line; the lines after it are read both ways); an unterminated character literal ends where
// the lexer ends it (before a `/`, or at the line end) and is kept as text. What follows any
// of them is read as the lexer reads it.
//
// Also: the view is as long as its line, or ends at or after the start of the line's line
// comment; and two scans of the same lines agree, views and states.
//
// The seed `every_character` (those bytes exactly) runs one more check, once in a process: for
// every character past ASCII, three short texts whose reading depends on whether the character
// may start a name, go on in one, or is an emoji. The view has those three tables of its own,
// and this is what fails when they fall behind the lexer's.

use deep_diff_forge_graph::lexical::{Language, ScanState, code_view};
use libfuzzer_sys::fuzz_target;
use ra_ap_rustc_lexer::{
    Cursor, FrontmatterAllowed, LiteralKind, RawStrError, TokenKind, is_id_continue, is_id_start,
    validate_raw_str,
};

/// What the compiler makes of one byte of the text.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Class {
    /// In a token that is code: it must stand in the view.
    Code,
    /// In a comment: blank, where the view is precise.
    Comment,
    /// Between the quotes of a closed literal: blank, where the view is precise.
    Text,
    /// The rest of a literal (its prefix and quotes, and all of one that is unterminated or a
    /// character literal over a line end): the view may keep it or blank it.
    Free,
}

/// The compiler's reading of a text.
struct Truth {
    /// The class of each byte.
    classes: Vec<Class>,
    /// Where each line comment starts, ascending.
    line_comments: Vec<usize>,
    /// The first line end that stands inside a literal, if one does.
    literal_over_line_end: Option<usize>,
}

/// Read `text` as the compiler does.
fn truth(text: &str) -> Truth {
    let mut found = Truth {
        classes: vec![Class::Code; text.len()],
        line_comments: Vec::new(),
        literal_over_line_end: None,
    };
    let mut at = 0;
    let mut cursor = Cursor::new(text, FrontmatterAllowed::No);
    loop {
        let token = cursor.advance_token();
        let mut len = token.len as usize;
        match token.kind {
            TokenKind::Eof => break,
            // Before edition 2024 the compiler takes a `#` and reads the rest again; edition
            // 2024 refuses the text.
            TokenKind::GuardedStrPrefix => {
                len = 1;
                cursor = Cursor::new(&text[at + 1..], FrontmatterAllowed::No);
            }
            TokenKind::LineComment { .. } => {
                found.line_comments.push(at);
                found.classes[at..at + len].fill(Class::Comment);
            }
            TokenKind::BlockComment { .. } => found.classes[at..at + len].fill(Class::Comment),
            TokenKind::Literal { kind, suffix_start } => {
                let body = suffix_start as usize;
                let piece = &text[at..at + len];
                let (class, text_of) = literal(kind, &piece[..body], piece);
                found.classes[at..at + body].fill(class);
                if let Some((from, to)) = text_of {
                    found.classes[at + from..at + to].fill(Class::Text);
                }
                if class != Class::Code
                    && found.literal_over_line_end.is_none()
                    && let Some(line_end) = piece[..body].find('\n')
                {
                    found.literal_over_line_end = Some(at + line_end);
                }
            }
            _ => {}
        }
        at += len;
    }
    found
}

/// The class of a literal's bytes before its suffix (`body`; `piece` is the whole token), and
/// the range of the text between its quotes when the literal is closed.
fn literal(kind: LiteralKind, body: &str, piece: &str) -> (Class, Option<(usize, usize)>) {
    // From after the first `quote` to before the closing one, which `after` bytes follow.
    let between = |quote: char, after: usize| {
        let open = body.find(quote)?;
        let close = body.len().checked_sub(after + 1)?;
        (open < close).then_some((open + 1, close))
    };
    match kind {
        LiteralKind::Int { .. } | LiteralKind::Float { .. } => (Class::Code, None),
        // A character literal is blank when it can be one: one character between its quotes,
        // or text that starts with a backslash, on one line. What else the lexer closes as a
        // literal (`'ab'`, `''`, one over a line end) the compiler refuses; the view keeps it.
        LiteralKind::Char { terminated: true } | LiteralKind::Byte { terminated: true } => {
            let text = between('\'', 0).filter(|(from, to)| {
                let text = &body[*from..*to];
                !body.contains('\n') && (text.starts_with('\\') || text.chars().count() == 1)
            });
            (Class::Free, text)
        }
        LiteralKind::Str { terminated: true }
        | LiteralKind::ByteStr { terminated: true }
        | LiteralKind::CStr { terminated: true } => (Class::Free, between('"', 0)),
        LiteralKind::RawStr {
            n_hashes: Some(hashes),
        }
        | LiteralKind::RawByteStr {
            n_hashes: Some(hashes),
        }
        | LiteralKind::RawCStr {
            n_hashes: Some(hashes),
        } => (Class::Free, between('"', usize::from(hashes))),
        LiteralKind::RawStr { n_hashes: None }
        | LiteralKind::RawByteStr { n_hashes: None }
        | LiteralKind::RawCStr { n_hashes: None } => {
            let prefix = if matches!(kind, LiteralKind::RawStr { .. }) {
                1
            } else {
                2
            };
            match validate_raw_str(piece, prefix) {
                // `r#` and a character that is no quote: no string at all.
                Err(RawStrError::InvalidStarter { .. }) => (Class::Code, None),
                // Closed, with more `#` than the compiler allows.
                Err(RawStrError::TooManyDelimiters { found }) => {
                    (Class::Free, between('"', found as usize))
                }
                Err(RawStrError::NoTerminator { .. }) | Ok(()) => (Class::Free, None),
            }
        }
        _ => (Class::Free, None),
    }
}

/// Hold the view of `text`, read line by line from its first line, to the compiler's reading.
fn check(text: &str) {
    let truth = truth(text);
    let (mut state, mut again) = (ScanState::default(), ScanState::default());
    let mut comments = truth.line_comments.iter().copied().peekable();
    let mut start = 0;
    for (number, line) in text.split('\n').enumerate() {
        let end = start + line.len();
        // One reading: no literal has run over a line end yet, or nothing at all is carried.
        let precise = truth.literal_over_line_end.is_none_or(|at| at >= start)
            || state == ScanState::default();
        let view = code_view(Language::Rust, line, &mut state);
        assert_eq!(
            view,
            code_view(Language::Rust, line, &mut again),
            "two scans of line {} differ",
            number + 1
        );
        assert_eq!(
            state,
            again,
            "two scans differ in state after line {}",
            number + 1
        );

        while comments.next_if(|at| *at < start).is_some() {}
        match comments.peek() {
            Some(comment) if *comment < end => assert!(
                (comment - start..=line.len()).contains(&view.len()),
                "line {}: the view is {} bytes, the line {} and its comment starts at {}: {line:?}",
                number + 1,
                view.len(),
                line.len(),
                comment - start
            ),
            _ => assert_eq!(
                view.len(),
                line.len(),
                "line {}: the view is not as long as the line: {line:?} -> {view:?}",
                number + 1
            ),
        }

        let shown = view.as_bytes();
        for (at, (byte, class)) in line.bytes().zip(&truth.classes[start..end]).enumerate() {
            match class {
                Class::Code => assert_eq!(
                    shown.get(at),
                    Some(&byte),
                    "SAFETY: line {} byte {}: code is hidden: {line:?} -> {view:?}",
                    number + 1,
                    at
                ),
                Class::Comment | Class::Text if precise => assert!(
                    matches!(shown.get(at), None | Some(b' ')),
                    "PRECISION: line {} byte {}: {class:?} is shown: {line:?} -> {view:?}",
                    number + 1,
                    at
                ),
                _ => {}
            }
        }
        start = end + 1;
    }
}

/// What stands before a character in each of the three questions the view is asked about it,
/// and what the lexer's answer is called.
const QUESTIONS: [(&str, &str); 3] = [
    // `r#` and a name, or `r#` and a character that starts none.
    ("r#", "may start a name"),
    // A name that goes on through the character, or one that ends before it.
    ("a", "goes on in a name"),
    // A token that starts with the character and takes the letter after it, or does not.
    ("", "starts a name or is an emoji"),
];

/// What stands after the character. If the answer is yes, the `r` belongs to a name and
/// `"\"x"` is a string that holds the `x`. If it is no, `r"\"` is a raw string and the `x`
/// is code. Either way the text ends in code, so one follows another on a line.
const AFTER: &str = "r\"\\\"x\";\"\\\"\";";

/// Where the `x` stands in [`AFTER`].
const X_IN_AFTER: usize = 4;

/// The lexer's answers to [`QUESTIONS`] for `c`. An emoji is no name character, but the lexer
/// takes it into a name (which the compiler then refuses), so it is told by the token that
/// starts with it: one token with the letter after it.
fn lexer_says(c: char, scratch: &mut String) -> [bool; 3] {
    scratch.clear();
    scratch.push(c);
    scratch.push('a');
    let first = Cursor::new(scratch, FrontmatterAllowed::No).advance_token();
    let takes_the_letter = first.len as usize == scratch.len();
    [
        is_id_start(c),
        is_id_continue(c) || takes_the_letter,
        takes_the_letter,
    ]
}

/// One character, the lexer's answers about it, and where the `x` of each question stands in
/// its line.
struct Asked {
    c: char,
    lexer: [bool; 3],
    x: [usize; 3],
}

/// Read each question's line once and hold the view to the lexer's answer for every character
/// in it: the `x` is blank where the answer is yes. Then empty the lines.
fn answer(lines: &mut [String; 3], asked: &mut Vec<Asked>) {
    for (question, line) in lines.iter().enumerate() {
        let view = code_view(Language::Rust, line, &mut ScanState::default());
        assert_eq!(view.len(), line.len(), "{line:?} -> {view:?}");
        for Asked { c, lexer, x } in asked.iter() {
            assert_eq!(
                view.as_bytes()[x[question]] == b' ',
                lexer[question],
                "U+{:04X} {c:?}: the lexer says `{}` is {}, the view reads it the other way",
                u32::from(*c),
                QUESTIONS[question].1,
                lexer[question]
            );
        }
    }
    lines.iter_mut().for_each(String::clear);
    asked.clear();
}

/// For every character past ASCII, whether the view reads it as the lexer does: as one that may
/// start a name, go on in one, or is an emoji. The view has three tables of its own for that
/// (it ships; the lexer does not), and this is what fails when they fall behind the lexer's.
/// Each question is one line for some thousands of characters, read once.
fn every_character() {
    let mut scratch = String::new();
    let mut lines = [String::new(), String::new(), String::new()];
    let mut asked = Vec::new();
    for c in '\u{80}'..=char::MAX {
        let mut x = [0; 3];
        for (question, line) in lines.iter_mut().enumerate() {
            line.push_str(QUESTIONS[question].0);
            line.push(c);
            x[question] = line.len() + X_IN_AFTER;
            line.push_str(AFTER);
        }
        asked.push(Asked {
            c,
            lexer: lexer_says(c, &mut scratch),
            x,
        });
        if asked.len() == 4096 {
            answer(&mut lines, &mut asked);
        }
    }
    answer(&mut lines, &mut asked);
}

/// [`every_character`] has run in this process: the answer does not change, and a fuzzer
/// offers the same seed again and again.
static EVERY_CHARACTER: std::sync::Once = std::sync::Once::new();

fuzz_target!(|data: &[u8]| {
    if data == b"every_character" {
        EVERY_CHARACTER.call_once(every_character);
        return;
    }
    if data.len() > 64 * 1024 {
        return;
    }
    check(&String::from_utf8_lossy(data));
});
