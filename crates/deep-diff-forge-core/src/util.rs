//! Small pure helpers shared across crates (no I/O, no behavior beyond value
//! transforms — consistent with core's vocabulary-only charter).

use std::fmt::Write as _;

/// Quote and escape a string as a JSON string literal (RFC 8259), including the
/// surrounding double quotes. UTF-8 passes through; control characters below
/// `0x20` become `\u00xx` escapes.
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
            c if u32::from(c) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
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
}
