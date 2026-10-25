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
