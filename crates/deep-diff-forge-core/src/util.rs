//! Small pure helpers shared across crates (no I/O, no behavior beyond value
//! transforms — consistent with core's vocabulary-only charter).

use std::borrow::Cow;
use std::fmt::Write as _;

/// Quote and escape a string as a JSON string literal (RFC 8259), including the
/// surrounding double quotes. UTF-8 passes through; control characters below
/// `0x20` become `\u00xx` escapes. As terminal-injection defence-in-depth, the
/// `DEL` (`0x7f`) and C1 control block (`0x80..=0x9f` — which includes the 8-bit
/// CSI `0x9b` and OSC `0x9d` introducers) are escaped too, so the JSON output is
/// safe to print to a terminal even though RFC 8259 does not require it.
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
            c if u32::from(c) < 0x20 || matches!(u32::from(c), 0x7f..=0x9f) => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
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
/// (CVE-2021-42574): bidi overrides/embeddings (`LRE`..`RLO`), bidi isolates
/// (`LRI`..`PDI`), directional marks (`LRM`/`RLM`/`ALM`), and zero-width
/// characters (`ZWSP`/`ZWNJ`/`ZWJ`/`BOM`). A diff/review tool must surface these
/// rather than let attacker source reorder or hide code from the reviewer.
#[must_use]
fn is_bidi_or_invisible(c: char) -> bool {
    matches!(
        u32::from(c),
        0x202A..=0x202E   // LRE RLE PDF LRO RLO
        | 0x2066..=0x2069 // LRI RLI FSI PDI
        | 0x200E | 0x200F // LRM RLM
        | 0x061C          // ALM
        | 0x200B..=0x200D // ZWSP ZWNJ ZWJ
        | 0xFEFF          // ZWNBSP / BOM
    )
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
            // Bidi/invisible chars are > 0xff; show them as a visible \u{XXXX}.
            let _ = write!(out, "\\u{{{:04x}}}", u32::from(c));
        } else {
            out.push(c);
        }
    }
    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(display_safe("a\tb") == "a\tb");
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
            if let Some(c) = char::from_u32(cp) {
                if c.is_control() {
                    let s = c.to_string();
                    let out = display_safe(&s);
                    assert!(
                        !out.chars().any(char::is_control),
                        "control U+{cp:04x} survived display_safe"
                    );
                }
            }
        }
    }
}
