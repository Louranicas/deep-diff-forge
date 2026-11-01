// DDF integration: R02, R03, R04, R05, R06, R07, R44, R46. Preserve source bytes and exact original input independently of normalized display; use Git as the application oracle.
// Design/status: docs/RECOMMENDATION_INTEGRATION.md#patch-truth
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fdeep-diff-forge-patch--paths
//! Git C-quoted UTF-8 paths. Non-UTF-8 paths require a byte-oriented model.
pub(crate) fn decode_quoted_prefix(input: &str) -> Option<(String, &str)> {
    let bytes = input.as_bytes();
    if bytes.first() != Some(&b'"') {
        return None;
    }
    let mut out = Vec::new();
    let mut i = 1;
    while let Some(&byte) = bytes.get(i) {
        i += 1;
        match byte {
            b'"' => return Some((String::from_utf8(out).ok()?, &input[i..])),
            b'\\' => {
                let escaped = *bytes.get(i)?;
                i += 1;
                out.push(match escaped {
                    b'a' => 7,
                    b'b' => 8,
                    b't' => 9,
                    b'n' => 10,
                    b'v' => 11,
                    b'f' => 12,
                    b'r' => 13,
                    b'\\' => b'\\',
                    b'"' => b'"',
                    b'0'..=b'3' => {
                        let second = *bytes.get(i)?;
                        let third = *bytes.get(i + 1)?;
                        if !(b'0'..=b'7').contains(&second) || !(b'0'..=b'7').contains(&third) {
                            return None;
                        }
                        i += 2;
                        (escaped - b'0') * 64 + (second - b'0') * 8 + (third - b'0')
                    }
                    _ => return None,
                });
            }
            0 => return None,
            _ => out.push(byte),
        }
    }
    None
}
pub(crate) fn decode_path(input: &str) -> Option<String> {
    if input.starts_with('"') {
        let (decoded, rest) = decode_quoted_prefix(input)?;
        if rest.is_empty() && !decoded.contains('\0') {
            Some(decoded)
        } else {
            None
        }
    } else if input.contains('\0') {
        None
    } else {
        Some(input.to_owned())
    }
}
pub(crate) fn quote(path: &str) -> String {
    if !path
        .bytes()
        .any(|b| b <= b' ' || b >= 127 || b == b'"' || b == b'\\')
    {
        return path.to_owned();
    }
    let mut out = String::from("\"");
    for byte in path.bytes() {
        match byte {
            b'"' => out.push_str("\\\""),
            b'\\' => out.push_str("\\\\"),
            b'\t' => out.push_str("\\t"),
            b'\n' => out.push_str("\\n"),
            b'\r' => out.push_str("\\r"),
            32..=126 => out.push(char::from(byte)),
            _ => {
                use std::fmt::Write as _;
                let _ = write!(out, "\\{byte:03o}");
            }
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run `work` on its own thread and fail if it takes more than ten seconds. A decode of these
    /// few bytes takes microseconds, so a cursor that stops advancing fails here instead of
    /// hanging the suite (and a mutation run reports it as caught, not as a timeout).
    fn within_deadline<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> T {
        let (done, wait) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = done.send(work());
        });
        wait.recv_timeout(std::time::Duration::from_secs(10))
            .expect("a short quoted path was decoded within ten seconds")
    }

    fn prefix(input: &'static str) -> Option<(String, &'static str)> {
        within_deadline(move || decode_quoted_prefix(input))
    }

    fn path(input: &str) -> Option<String> {
        let input = input.to_owned();
        within_deadline(move || decode_path(&input))
    }

    #[test]
    fn a_quoted_prefix_returns_the_text_after_its_closing_quote() {
        assert_eq!(prefix("\"\" rest"), Some((String::new(), " rest")));
        assert_eq!(prefix("\"a b\" b/c"), Some(("a b".to_string(), " b/c")));
        assert_eq!(prefix("a"), None);
        assert_eq!(prefix("\"unterminated"), None);
    }

    #[test]
    fn every_c_escape_decodes_to_its_byte() {
        assert_eq!(
            prefix(r#""\a\b\t\n\v\f\r\\\"""#),
            Some(("\u{7}\u{8}\t\n\u{b}\u{c}\r\\\"".to_string(), ""))
        );
        assert_eq!(prefix(r#""\q""#), None, "an unknown escape is refused");
        assert_eq!(prefix("\"\\"), None, "a trailing backslash is refused");
    }

    #[test]
    fn octal_escapes_decode_to_bytes_and_must_be_three_octal_digits() {
        assert_eq!(prefix(r#""\101x""#), Some(("Ax".to_string(), "")));
        assert_eq!(prefix(r#""\303\251""#), Some(("\u{e9}".to_string(), "")));
        assert_eq!(prefix(r#""\108""#), None, "a non-octal third digit");
        assert_eq!(prefix(r#""\180""#), None, "a non-octal second digit");
        assert_eq!(prefix(r#""\10"#), None, "a truncated escape");
        assert_eq!(prefix(r#""\377""#), None, "a lone byte that is not UTF-8");
    }

    #[test]
    fn a_raw_nul_inside_quotes_is_refused() {
        assert_eq!(prefix("\"a\0b\""), None);
    }

    #[test]
    fn decode_path_takes_only_a_whole_quoted_path_without_nul() {
        assert_eq!(path("\"a\\tb\""), Some("a\tb".to_string()));
        assert_eq!(path("\"a\" x"), None, "text after the closing quote");
        assert_eq!(path(r#""a\000""#), None, "an escaped NUL");
        assert_eq!(path("a\0b"), None, "a raw NUL in an unquoted path");
        assert_eq!(path("plain/path"), Some("plain/path".to_string()));
    }

    #[test]
    fn quote_leaves_plain_paths_bare() {
        assert_eq!(quote("src/lib.rs"), "src/lib.rs");
    }

    #[test]
    fn quote_quotes_each_byte_class_that_needs_it() {
        assert_eq!(quote("a b"), "\"a b\"");
        assert_eq!(quote("\u{e9}"), r#""\303\251""#);
        assert_eq!(quote("a\"b"), r#""a\"b""#);
        assert_eq!(quote("a\\b"), r#""a\\b""#);
        assert_eq!(quote("\u{1}"), r#""\001""#);
        assert_eq!(quote("\u{7f}"), r#""\177""#);
    }

    #[test]
    fn quote_uses_the_short_escapes_git_writes() {
        assert_eq!(quote("t\tn\nr\r"), r#""t\tn\nr\r""#);
    }

    #[test]
    fn quoted_paths_decode_back_to_themselves() {
        for original in [
            "a b",
            "\u{e9}t\u{e9}",
            "a\"b\\c",
            "tab\there",
            "nl\nx",
            "\u{1}\u{7f}",
        ] {
            let quoted = quote(original);
            assert_eq!(path(&quoted).as_deref(), Some(original), "{quoted}");
        }
    }
}
