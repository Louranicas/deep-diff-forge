// Module index: docs/reference/diagnostics.md#on-the-command-line
//! The text form of `--diagnostics` (F5a): compiler-style lines for an agent, drawn from the
//! `deep-diff-forge.diagnostics.v0` bytes the builder wrote, never from the review. It prints each
//! file's path, quoted and escaped, the line and its side, the code and its class, the vocabulary
//! ids and their sentences, and counts: no other text of the change. A `floor_only` reading is
//! counted per file, not listed, and `judge_sees` is never printed (diagnostics.md, "For agents
//! and for people").

use deep_diff_forge_core::display_safe;
use serde_json::Value;
use std::fmt::Write as _;

/// How a file's `floor_only` readings are counted for an agent (contract design, section 3.4).
const FLOOR: &str =
    "readings kept for compatibility with 0.8.3 are for a person; leave the text that carries them";

/// `path` quoted, `\` and `"` escaped, then each control, bidi and invisible character escaped:
/// a path cannot close its quote, start a line or reorder what follows it.
fn quoted(path: &str) -> String {
    let path = path.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{}\"", display_safe(&path))
}

/// `name: id, id` for a non-empty list of ids.
fn ids(name: &str, list: &Value) -> String {
    let list: Vec<&str> = list
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    if list.is_empty() {
        String::new()
    } else {
        format!("; {name}: {}", list.join(", "))
    }
}

/// The text form of `document`, a `diagnostics.v0` document as the builder writes it.
pub(crate) fn render(document: &str) -> String {
    let doc: Value = serde_json::from_str(document).unwrap_or_default();
    let text = |value: &Value| value.as_str().unwrap_or_default().to_owned();
    let (mut out, mut floor) = (String::new(), Vec::<(String, usize)>::new());
    for d in doc["diagnostics"].as_array().into_iter().flatten() {
        let path = quoted(&text(&d["path"]));
        if d["reading"] != "current" {
            match floor.iter_mut().find(|(p, _)| *p == path) {
                Some((_, n)) => *n += 1,
                None => floor.push((path, 1)),
            }
            continue;
        }
        let (code, class) = (text(&d["code"]), text(&d["class"]));
        let at = match (d["line"].as_u64(), d["side"].as_str()) {
            (Some(line), Some(side)) => format!(":{line}: {code} ({class}, {side} side)"),
            _ => format!(": {code} ({class})"),
        };
        let repairs = ids("repairs", &d["repairs"]);
        let _ = writeln!(
            out,
            "{path}{at}{repairs}{}",
            ids("not accepted", &d["not_accepted"])
        );
    }
    for (path, n) in floor {
        let _ = writeln!(out, "{path}: {n} {FLOOR}");
    }
    for cut in doc["cut"].as_array().into_iter().flatten() {
        let (path, kept) = (quoted(&text(&cut["path"])), &cut["rows_kept"]);
        let _ = writeln!(out, "{path}: {kept} places listed; the rest were cut at 32");
    }
    for (id, sentence) in doc["texts"].as_object().into_iter().flatten() {
        let _ = writeln!(out, "{id}: {}", text(sentence));
    }
    let counts = ["files", "diagnostics", "agent", "person", "cut_files"]
        .map(|key| format!("{key} {}", doc["counts"][key].as_u64().unwrap_or_default()));
    let _ = writeln!(out, "counts: {}", counts.join(", "));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_cannot_close_its_quote_or_start_a_line() {
        assert_eq!(
            quoted("a\"b\\c\nd\u{202E}e"),
            "\"a\\\"b\\\\c\\x0ad\\u{202e}e\""
        );
    }
}
