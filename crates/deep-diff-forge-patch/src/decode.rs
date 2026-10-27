// DDF anchor C-12: a non-UTF-8 line no longer refuses a whole review; it is decoded lossily and
// its file marked. Design/status: vault Strategy/Anchor Task List 2026-10-27.md (C-12).
//! Patch bytes to parser text.
//!
//! Real repositories commit non-UTF-8 files (Latin-1 sources, fuzz corpora), and one such line
//! used to refuse the whole review. Valid lines pass through unchanged. An invalid line is decoded
//! lossily (U+FFFD) and its number recorded, so the parser can mark the file it belongs to, while
//! the input seal still covers the exact bytes.

use std::borrow::Cow;

/// Patch text ready for the parser, with the lines that were not UTF-8.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedPatch<'a> {
    text: Cow<'a, str>,
    byte_len: usize,
    lossy_lines: Vec<usize>,
}

impl DecodedPatch<'_> {
    /// The text the parser reads: the input itself when [`Self::is_exact`].
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Length of the original bytes, which the byte budget applies to.
    #[must_use]
    pub fn byte_len(&self) -> usize {
        self.byte_len
    }

    /// 1-based numbers of the lines that were decoded lossily, ascending.
    #[must_use]
    pub fn lossy_lines(&self) -> &[usize] {
        &self.lossy_lines
    }

    /// Whether every line is the exact input text.
    #[must_use]
    pub fn is_exact(&self) -> bool {
        self.lossy_lines.is_empty()
    }
}

/// Decode patch bytes line by line; line numbers match the parser's (split on LF).
#[must_use]
pub fn decode(bytes: &[u8]) -> DecodedPatch<'_> {
    if let Ok(text) = std::str::from_utf8(bytes) {
        return DecodedPatch {
            text: Cow::Borrowed(text),
            byte_len: bytes.len(),
            lossy_lines: Vec::new(),
        };
    }
    let mut text = String::with_capacity(bytes.len());
    let mut lossy_lines = Vec::new();
    for (index, line) in bytes.split_inclusive(|b| *b == b'\n').enumerate() {
        if let Ok(valid) = std::str::from_utf8(line) {
            text.push_str(valid);
        } else {
            text.push_str(&String::from_utf8_lossy(line));
            lossy_lines.push(index + 1);
        }
    }
    DecodedPatch {
        text: Cow::Owned(text),
        byte_len: bytes.len(),
        lossy_lines,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_utf8_is_borrowed_and_exact() {
        let decoded = decode("--- a/x\n+++ b/x\n".as_bytes());
        assert!(decoded.is_exact());
        assert!(matches!(decoded.text, Cow::Borrowed(_)));
        assert_eq!(decoded.text(), "--- a/x\n+++ b/x\n");
        assert_eq!(decoded.byte_len(), 16);
    }

    #[test]
    fn only_invalid_lines_are_decoded_lossily_and_numbered() {
        let bytes = b"ok\n-caf\xe9\nstill ok\n+\xff\xfe\nlast";
        let decoded = decode(bytes);
        assert_eq!(decoded.lossy_lines(), &[2, 4]);
        assert_eq!(
            decoded.text(),
            "ok\n-caf\u{fffd}\nstill ok\n+\u{fffd}\u{fffd}\nlast"
        );
        assert_eq!(decoded.byte_len(), bytes.len());
        assert!(!decoded.is_exact());
        // A literal U+FFFD in valid input is not a lossy line.
        assert!(decode("x\u{fffd}\n".as_bytes()).is_exact());
    }

    #[test]
    fn an_invalid_unterminated_last_line_is_numbered() {
        let decoded = decode(b"a\n\xc3");
        assert_eq!(decoded.lossy_lines(), &[2]);
        assert_eq!(decoded.text(), "a\n\u{fffd}");
    }
}
