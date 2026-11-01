//! Small pure helpers shared across crates (no I/O, no behavior beyond value
//! transforms — consistent with core's vocabulary-only charter).

use std::borrow::Cow;
use std::fmt::Write as _;

/// Quote and escape a string as a JSON string literal (RFC 8259), including the
/// surrounding double quotes. UTF-8 passes through; control characters below
/// `0x20` become `\u00xx` escapes. As terminal-injection defence-in-depth, the
/// `DEL` (`0x7f`) and C1 control block (`0x80..=0x9f` — which includes the 8-bit
/// CSI `0x9b` and OSC `0x9d` introducers) are escaped too, so the JSON output is
/// safe to print to a terminal even though RFC 8259 does not require it. `U+2028` and
/// `U+2029` are escaped as well: line-aware readers (JavaScript before ES2019, Python's
/// `splitlines`) treat them as line breaks and would split one JSON record in two.
#[must_use]
pub fn json_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            c if u32::from(c) < 0x20 || matches!(u32::from(c), 0x7f..=0x9f | 0x2028 | 0x2029) => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Make serialized JSON text safe for line-delimited transport: each raw `U+2028` or `U+2029`
/// becomes the escape `\u2028`/`\u2029`. In valid JSON these characters can occur only inside
/// strings, where the escape means the same character, so the document is unchanged; only a
/// line-aware reader's view of it is. Use it where a serializer that does not escape them
/// (`serde_json`) writes a line or document out; [`json_escape`] already escapes them.
#[must_use]
pub fn json_line_safe(json: &str) -> Cow<'_, str> {
    if !json.contains(['\u{2028}', '\u{2029}']) {
        return Cow::Borrowed(json);
    }
    Cow::Owned(
        json.replace('\u{2028}', "\\u2028")
            .replace('\u{2029}', "\\u2029"),
    )
}

/// Whether `c` must be escaped before being written to a terminal: any control
/// character (C0 `0x00..=0x1f`, `DEL`, C1 `0x80..=0x9f`) except the horizontal
/// tab, which is benign for terminal safety and ubiquitous in source code.
#[must_use]
fn is_terminal_unsafe(c: char) -> bool {
    c.is_control() && c != '\t'
}

/// Whether `c` is a bidirectional or invisible formatting character that can make
/// rendered text differ from its logical byte content — the "Trojan Source" class
/// (CVE-2021-42574) — or a Unicode line or paragraph separator, which some editors and
/// languages treat as a line break the reviewer cannot see. A diff/review tool must surface
/// these rather than let attacker source reorder, hide or split code from the reviewer.
#[must_use]
fn is_bidi_or_invisible(c: char) -> bool {
    is_bidi(c) || is_invisible(c) || is_line_separator(c)
}

/// Whether `c` should never reach a reader from untrusted text: a control character other than
/// a tab or a line feed, or a bidi, invisible or line-separator character ([`text_hazards`]
/// reports the same classes). Untrusted bodies (agent annotations and findings) drop these.
#[must_use]
pub fn is_unsafe_in_untrusted_text(c: char) -> bool {
    (c.is_control() && c != '\n' && c != '\t') || is_bidi_or_invisible(c)
}

/// Bidi overrides and embeddings (`LRE`..`RLO`), isolates (`LRI`..`PDI`) and directional
/// marks (`LRM`/`RLM`/`ALM`).
fn is_bidi(c: char) -> bool {
    matches!(
        u32::from(c),
        0x202A..=0x202E   // LRE RLE PDF LRO RLO
        | 0x2066..=0x2069 // LRI RLI FSI PDI
        | 0x200E | 0x200F // LRM RLM
        | 0x061C          // ALM
    )
}

/// Characters that render as nothing (or as blank space indistinguishable from none) yet
/// change the bytes: every assigned Unicode `Default_Ignorable_Code_Point` that is not a bidi
/// control (zero-width and joiner characters, invisible operators, fillers, the Khmer inherent
/// vowels, variation selectors, deprecated, shorthand and musical format controls and tag
/// characters), plus the interlinear annotation controls. These are the payload channels of
/// invisible-text attacks; unassigned code points stay out, as before. A hand-picked subset let U+17B4, which is
/// valid inside a Rust identifier, through unescaped (bug hunt 2026-11-01).
fn is_invisible(c: char) -> bool {
    matches!(
        u32::from(c),
        0x200B..=0x200D       // ZWSP ZWNJ ZWJ
        | 0xFEFF              // ZWNBSP / BOM
        | 0x2060..=0x2064     // WJ, invisible function application, times, separator, plus
        | 0x206A..=0x206F     // deprecated format characters (symmetric swapping, shaping)
        | 0x00AD              // soft hyphen
        | 0x034F              // combining grapheme joiner
        | 0x17B4 | 0x17B5     // Khmer inherent vowels AQ and AA
        | 0x180B..=0x180F     // Mongolian free variation selectors and vowel separator
        | 0x115F | 0x1160     // Hangul choseong and jungseong fillers
        | 0x3164 | 0xFFA0     // Hangul filler, halfwidth Hangul filler
        | 0xFE00..=0xFE0F     // variation selectors 1-16
        | 0xFFF9..=0xFFFB     // interlinear annotation anchor, separator, terminator
        | 0x1BCA0..=0x1BCA3   // shorthand format controls
        | 0x1D173..=0x1D17A   // musical symbol format controls
        | 0xE0000..=0xE007F   // tag characters
        | 0xE0100..=0xE01EF   // variation selectors 17-256
    )
}

/// U+2028 LINE SEPARATOR and U+2029 PARAGRAPH SEPARATOR.
fn is_line_separator(c: char) -> bool {
    matches!(c, '\u{2028}' | '\u{2029}')
}

/// Objective hazards in one line of patch text: `bidi_control` (a bidirectional override,
/// embedding, isolate or mark, which can make rendered code differ from its bytes),
/// `invisible` (any other assigned Unicode default-ignorable code point, such as a zero-width, joiner,
/// filler, invisible-operator, variation-selector, format-control or tag character, a soft
/// hyphen or a Khmer inherent vowel, and the interlinear annotation controls) and `control` (a C0/C1 control other than tab, a Unicode line
/// or paragraph separator, and never the carriage return of a CRLF line ending). These are
/// facts about the bytes, not a guess about intent: an empty list never means the text is
/// safe.
#[must_use]
pub fn text_hazards(text: &str) -> Vec<&'static str> {
    char_hazards(text.strip_suffix('\r').unwrap_or(text))
}

fn char_hazards(text: &str) -> Vec<&'static str> {
    let mut found = Vec::new();
    if text.chars().any(is_bidi) {
        found.push("bidi_control");
    }
    if text.chars().any(is_invisible) {
        found.push("invisible");
    }
    if text
        .chars()
        .any(|c| is_terminal_unsafe(c) || is_line_separator(c))
    {
        found.push("control");
    }
    found
}

/// Objective hazards in a path a patch names: the character hazards of the whole path (a
/// trailing carriage return included: unlike a CRLF body line, a path never legitimately ends
/// in one), plus `path_traversal` (a `..` component or an absolute path) and `git_internal`
/// (a `.git` component).
#[must_use]
pub fn path_hazards(path: &str) -> Vec<&'static str> {
    let mut found = char_hazards(path);
    let mut components = path.split('/');
    if path.starts_with('/') || components.clone().any(|c| c == "..") {
        found.push("path_traversal");
    }
    if components.any(|c| c.eq_ignore_ascii_case(".git")) {
        found.push("git_internal");
    }
    found
}

/// Make an untrusted string safe to print to a terminal.
///
/// Attacker-controlled content (diff line bodies, file paths, symbol names,
/// agent annotations) can attack the reviewer's terminal or eyes two ways:
///
/// 1. **Terminal escapes** — ANSI/CSI/OSC sequences that clear the screen, poison
///    scrollback to forge a "clean" review, rewrite the window title, or drive an
///    OSC-52 clipboard write. Each terminal-unsafe control char is rendered as a
///    visible `\xHH` escape (e.g. `ESC` → `\x1b`).
/// 2. **Trojan Source** — bidi/invisible Unicode (e.g. `RLO` `U+202E`) that makes
///    code *display* differently than it logically reads, hiding malicious
///    edits from a reviewer. Each such char is rendered as a visible `\u{XXXX}`.
///
/// Printable text and ordinary multi-byte UTF-8 pass through unchanged, and
/// horizontal tabs are preserved so normal code indentation still renders.
/// Returns a borrowed `Cow` (zero allocation) when the input is already safe.
/// This is the terminal-output counterpart to [`json_escape`]: human renderers
/// use this, machine (`--json`/`--jsonl`) output uses `json_escape`.
#[must_use]
pub fn display_safe(value: &str) -> Cow<'_, str> {
    if !value
        .chars()
        .any(|c| is_terminal_unsafe(c) || is_bidi_or_invisible(c))
    {
        return Cow::Borrowed(value);
    }
    let mut out = String::with_capacity(value.len() + 8);
    for c in value.chars() {
        if is_terminal_unsafe(c) {
            // All terminal-unsafe chars are <= 0x9f, so two hex digits suffice.
            let _ = write!(out, "\\x{:02x}", u32::from(c));
        } else if is_bidi_or_invisible(c) {
            // Bidi, invisible and separator chars show as a visible \u{XXXX}.
            let _ = write!(out, "\\u{{{:04x}}}", u32::from(c));
        } else {
            out.push(c);
        }
    }
    Cow::Owned(out)
}

/// Make untrusted text terminal-safe while bounding the rendered prefix.
///
/// `max_bytes` counts rendered UTF-8 bytes, so escaping a control character cannot amplify a
/// diagnostic past the bound. When text is omitted, the suffix reports the exact number of source
/// bytes not shown; callers can distinguish a short message from a deliberately huge one.
#[must_use]
pub fn display_safe_bounded(value: &str, max_bytes: usize) -> String {
    let mut out = String::with_capacity(max_bytes.saturating_add(32));
    let mut consumed = 0usize;
    for c in value.chars() {
        let mut rendered = String::new();
        if is_terminal_unsafe(c) {
            let _ = write!(rendered, "\\x{:02x}", u32::from(c));
        } else if is_bidi_or_invisible(c) {
            let _ = write!(rendered, "\\u{{{:04x}}}", u32::from(c));
        } else {
            rendered.push(c);
        }
        if out.len().saturating_add(rendered.len()) > max_bytes {
            break;
        }
        out.push_str(&rendered);
        consumed += c.len_utf8();
    }
    if consumed < value.len() {
        let _ = write!(out, "... [{} bytes omitted]", value.len() - consumed);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn untrusted_text_keeps_newlines_tabs_and_ordinary_text_only() {
        for c in ['\n', '\t', 'a', ' ', 'é', '→', '中'] {
            assert!(!is_unsafe_in_untrusted_text(c), "{c:?}");
        }
        for c in [
            '\u{0}',
            '\u{7}',
            '\r',
            '\u{1b}',
            '\u{7f}',
            '\u{85}',
            '\u{202E}',
            '\u{2066}',
            '\u{200B}',
            '\u{FEFF}',
            '\u{2028}',
            '\u{2029}',
            '\u{E0041}',
        ] {
            assert!(is_unsafe_in_untrusted_text(c), "{c:?}");
        }
    }

    #[test]
    fn hazards_are_facts_about_the_bytes() {
        assert_eq!(text_hazards("let x = 1;"), Vec::<&str>::new());
        assert_eq!(text_hazards("let x = 1;\r"), Vec::<&str>::new());
        assert_eq!(text_hazards("let\u{202E} x"), ["bidi_control"]);
        assert_eq!(text_hazards("a\u{200B}b"), ["invisible"]);
        assert_eq!(text_hazards("a\u{1b}[2Jb"), ["control"]);
        assert_eq!(text_hazards("a\rb"), ["control"]);
        assert_eq!(
            text_hazards("\u{2066}\u{FEFF}\u{7}"),
            ["bidi_control", "invisible", "control"]
        );
        assert_eq!(text_hazards("\tindented"), Vec::<&str>::new());
        assert_eq!(path_hazards("src/lib.rs"), Vec::<&str>::new());
        assert_eq!(path_hazards("../etc/passwd"), ["path_traversal"]);
        assert_eq!(path_hazards("/etc/passwd"), ["path_traversal"]);
        assert_eq!(path_hazards("a/.GIT/hooks/pre-commit"), ["git_internal"]);
        assert_eq!(path_hazards("x\u{202E}y.rs"), ["bidi_control"]);
        assert_eq!(path_hazards("a..b/c"), Vec::<&str>::new());
    }

    /// Bug hunt 2026-10-28, H1-05 and H1-06: every invisible-payload class is flagged, a line
    /// or paragraph separator is a control, and only a body line may end in a CR.
    #[test]
    fn invisible_payload_classes_and_separators_are_hazards() {
        for c in [
            '\u{2060}',
            '\u{2062}',
            '\u{2064}',
            '\u{00AD}',
            '\u{034F}',
            '\u{180E}',
            '\u{115F}',
            '\u{1160}',
            '\u{3164}',
            '\u{FFA0}',
            '\u{FE00}',
            '\u{FE0F}',
            '\u{E0000}',
            '\u{E0041}',
            '\u{E007F}',
            '\u{E0100}',
            '\u{E01EF}',
            // Bug hunt 2026-11-01: the rest of Default_Ignorable_Code_Point, and the
            // interlinear annotation controls, reached a terminal raw and unflagged.
            '\u{17B4}',
            '\u{17B5}',
            '\u{180B}',
            '\u{180D}',
            '\u{180F}',
            '\u{206A}',
            '\u{206F}',
            '\u{FFF9}',
            '\u{FFFB}',
            '\u{1BCA0}',
            '\u{1BCA3}',
            '\u{1D173}',
            '\u{1D17A}',
        ] {
            assert_eq!(
                text_hazards(&format!("a{c}b")),
                ["invisible"],
                "{:x}",
                u32::from(c)
            );
            assert_eq!(
                display_safe(&format!("a{c}b")),
                format!("a\\u{{{:04x}}}b", u32::from(c))
            );
        }
        // The visible neighbours of the new ranges stay ordinary text.
        for c in [
            '\u{17B3}',
            '\u{17B6}',
            '\u{180A}',
            '\u{FFF8}',
            '\u{FFFC}',
            '\u{1BC9F}',
            '\u{1BCA4}',
            '\u{1D172}',
            '\u{1D17B}',
            '\u{2070}',
        ] {
            assert!(
                text_hazards(&format!("a{c}b")).is_empty(),
                "{:x}",
                u32::from(c)
            );
        }
        for c in ['\u{2028}', '\u{2029}'] {
            assert_eq!(text_hazards(&format!("a{c}b")), ["control"]);
            assert_eq!(
                display_safe(&format!("a{c}b")),
                format!("a\\u{{{:04x}}}b", u32::from(c))
            );
        }
        // Neighbours of each range stay clean.
        for c in [
            '\u{205F}',
            '\u{2065}',
            '\u{FE10}',
            '\u{E0080}',
            '\u{E01F0}',
            '\u{2027}',
            '\u{202F}',
        ] {
            assert_eq!(
                text_hazards(&format!("a{c}b")),
                Vec::<&str>::new(),
                "{:x}",
                u32::from(c)
            );
        }
        assert_eq!(text_hazards("x\r"), Vec::<&str>::new());
        assert_eq!(path_hazards("x\r"), ["control"]);
        assert_eq!(path_hazards("src/\u{2060}lib.rs"), ["invisible"]);
    }

    /// Bug hunt 2026-10-28, H4-06: the line and paragraph separators never reach a line-aware
    /// reader raw, whether the JSON was hand-built or came from serde.
    #[test]
    fn line_separators_never_leave_raw() {
        assert_eq!(json_escape("a\u{2028}b\u{2029}c"), "\"a\\u2028b\\u2029c\"");
        let raw = "{\"p\":\"a\u{2028}b\u{2029}\"}";
        assert_eq!(json_line_safe(raw), "{\"p\":\"a\\u2028b\\u2029\"}");
        assert!(matches!(json_line_safe("{\"p\":1}"), Cow::Borrowed(_)));
    }

    #[test]
    fn wraps_in_quotes() {
        assert_eq!(json_escape("ab"), "\"ab\"");
    }

    #[test]
    fn escapes_quote_and_backslash() {
        assert_eq!(json_escape("a\"b\\c"), "\"a\\\"b\\\\c\"");
    }

    #[test]
    fn escapes_whitespace_controls() {
        assert_eq!(json_escape("a\nb\tc\rd"), "\"a\\nb\\tc\\rd\"");
    }

    #[test]
    fn escapes_low_control_as_unicode() {
        assert_eq!(json_escape("\u{01}"), "\"\\u0001\"");
        // The C0 block ends at 0x1f: a space is printable and passes through.
        assert_eq!(json_escape("\u{1f} "), "\"\\u001f \"");
    }

    #[test]
    fn passes_unicode_through() {
        assert_eq!(json_escape("café→"), "\"café→\"");
    }

    #[test]
    fn empty_string_is_empty_quotes() {
        assert_eq!(json_escape(""), "\"\"");
    }

    #[test]
    fn json_escape_neutralizes_del_and_c1() {
        // DEL (0x7f) and the C1 block (incl. 8-bit CSI 0x9b / OSC 0x9d) must not
        // survive into terminal-printed JSON.
        assert_eq!(json_escape("\u{7f}"), "\"\\u007f\"");
        assert_eq!(json_escape("\u{9b}"), "\"\\u009b\"");
        assert_eq!(json_escape("\u{9d}"), "\"\\u009d\"");
        let out = json_escape("a\u{9b}b");
        assert!(!out.contains('\u{9b}'));
    }

    #[test]
    fn display_safe_passes_clean_text_borrowed() {
        let out = display_safe("normal/path.rs");
        assert!(matches!(out, Cow::Borrowed(_)));
        assert_eq!(out, "normal/path.rs");
    }

    #[test]
    fn display_safe_preserves_unicode_and_tabs() {
        // Tabs are kept (benign + ubiquitous in code); UTF-8 passes through.
        let out = display_safe("fn café() {\n");
        // newline IS escaped, tab/unicode preserved
        assert_eq!(display_safe("a\tb"), "a\tb");
        assert!(matches!(display_safe("a\tb"), Cow::Borrowed(_)));
        assert!(out.contains("café"));
    }

    #[test]
    fn display_safe_escapes_esc_and_csi() {
        // The headline injection vector: a raw ESC (0x1b) must never survive.
        let evil = "\u{1b}[2J\u{1b}[1;1H$ rm -rf ~";
        let safe = display_safe(evil);
        assert!(!safe.contains('\u{1b}'), "raw ESC byte must not survive");
        assert!(safe.contains("\\x1b"));
        assert!(safe.contains("[2J")); // the (now-inert) payload is shown, not executed
    }

    #[test]
    fn display_safe_escapes_cr_bel_del_c1() {
        assert_eq!(display_safe("\r"), "\\x0d"); // carriage-return overwrite vector
        assert_eq!(display_safe("\u{07}"), "\\x07"); // BEL
        assert_eq!(display_safe("\u{7f}"), "\\x7f"); // DEL
        assert_eq!(display_safe("\u{9b}"), "\\x9b"); // 8-bit CSI
    }

    #[test]
    fn display_safe_escapes_osc52_clipboard_hijack() {
        // OSC-52 write-to-clipboard: ESC ] 52 ; c ; <base64> BEL
        let osc = "\u{1b}]52;c;ZXZpbA==\u{07}";
        let safe = display_safe(osc);
        assert!(!safe.contains('\u{1b}'));
        assert!(!safe.contains('\u{07}'));
    }

    #[test]
    fn display_safe_neutralizes_trojan_source_bidi() {
        // CVE-2021-42574: an RLO override that visually reorders code must be
        // surfaced, not passed through, so the reviewer sees what really executes.
        let evil = "let access = if is_admin\u{202e} // ban_user\u{202c}";
        let safe = display_safe(evil);
        assert!(!safe.contains('\u{202e}'), "RLO must not survive");
        assert!(!safe.contains('\u{202c}'), "PDF must not survive");
        assert!(safe.contains("\\u{202e}"));
        assert!(safe.contains("\\u{202c}"));
    }

    #[test]
    fn display_safe_neutralizes_zero_width_and_isolates() {
        for cp in [0x200b_u32, 0x200d, 0x2066, 0x2069, 0x200e, 0xfeff, 0x061c] {
            let c = char::from_u32(cp).unwrap();
            let s = format!("a{c}b");
            let out = display_safe(&s);
            assert!(!out.contains(c), "U+{cp:04x} must not survive");
            assert!(out.contains(&format!("\\u{{{cp:04x}}}")));
        }
    }

    #[test]
    fn display_safe_keeps_ordinary_unicode() {
        // Real multi-byte content (accents, CJK, emoji, arrows) is NOT bidi/
        // invisible and must pass through untouched (borrowed, zero-alloc).
        let s = "café 源 → 🚀 ";
        let out = display_safe(s);
        assert!(matches!(out, Cow::Borrowed(_)));
        assert_eq!(out, s);
    }

    #[test]
    fn display_safe_no_control_char_ever_survives() {
        // Exhaustive: every control codepoint except tab is escaped.
        for cp in (0x00u32..=0x9f).filter(|c| *c != 0x09) {
            if let Some(c) = char::from_u32(cp)
                && c.is_control()
            {
                let s = c.to_string();
                let out = display_safe(&s);
                assert!(
                    !out.chars().any(char::is_control),
                    "control U+{cp:04x} survived display_safe"
                );
            }
        }
    }

    #[test]
    fn bounded_display_is_safe_and_reports_exact_omitted_source_bytes() {
        let input = format!("{}{}", "a".repeat(205), "\u{1b}[2J");
        let out = display_safe_bounded(&input, 200);
        assert!(out.starts_with(&"a".repeat(200)));
        assert!(out.ends_with("[9 bytes omitted]"), "{out}");
        assert!(!out.contains('\u{1b}'));
        assert!(out.len() < 240, "{out}");
        // Text that fits is returned whole, with no omission note.
        assert_eq!(display_safe_bounded("abc", 200), "abc");
        assert_eq!(display_safe_bounded("abc", 3), "abc");
    }
}
