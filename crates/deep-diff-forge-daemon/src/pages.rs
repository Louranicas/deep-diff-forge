//! `diff.diagnostics` (engine protocol 3): the `deep-diff-forge.diagnostics.v0` document of one
//! sealed review, in pages of whole diagnostics (`docs/reference/diagnostics.md`, "Paging").
//!
//! The document is the builder's ([`deep_diff_forge_graph::Diagnostics::to_json`]), the bytes
//! `--stdin-patch --diagnostics --json` prints, parsed; nothing here writes a member of it. A
//! page holds the document's header, its `page` member, the slice of `diagnostics` it carries,
//! the `cut` entries of the files whose first diagnostic is in that slice, and the `texts` the
//! slice names. `page.document_sha256` is the SHA-256 of the document's bytes on every page.

use crate::views::{MAX_MAX_BYTES, Page};
use deep_diff_forge_core::{InputSeal, is_unsafe_in_untrusted_text, sha256_hex};
use deep_diff_forge_graph::{diagnostics, rank, scorer_sha256};
use deep_diff_forge_patch::SealedReview;
use serde_json::{Map, Value, json};
use std::borrow::Cow;
use std::collections::BTreeSet;
use std::fmt::Write as _;

/// The most bytes a page's header takes: the members from `schema` to `counts` with empty
/// slices, and `page`, written compact with 20-digit numbers (`the_page_header_fits_the_smallest_page`).
pub const HEADER_BOUND: usize = 960;

/// The smallest `max_bytes` of `diff.diagnostics`: the header and one diagnostic with the texts it
/// names take about 1,200 bytes, so the views' 1,024 would refuse most documents.
pub const MIN_PAGE_BYTES: usize = 4096;

/// Why a page was refused: each is a mistake in the request, never in the patch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PageError {
    /// `max_bytes` is outside [`MIN_PAGE_BYTES`]..=[`MAX_MAX_BYTES`].
    Budget(usize),
    /// The cursor is not one this document's pages could have minted; the reason is a fixed
    /// sentence that holds no byte of the cursor.
    Cursor(String),
    /// The first diagnostic the page must hold does not fit `max_bytes` with the header.
    Item {
        max_bytes: usize,
        item: usize,
        bytes: usize,
    },
}

impl std::fmt::Display for PageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Budget(n) => write!(
                f,
                "max_bytes {n} is outside {MIN_PAGE_BYTES}..={MAX_MAX_BYTES}"
            ),
            Self::Cursor(why) => write!(f, "invalid cursor: {why}"),
            Self::Item {
                max_bytes,
                item,
                bytes,
            } => write!(
                f,
                "max_bytes {max_bytes} is below the size of item {item} with the page header ({bytes} bytes)"
            ),
        }
    }
}

/// A written answer line with every bidi control, invisible character and control left raw by
/// the JSON writer as a `\uXXXX` escape (a pair above U+FFFF): the same value, and no character
/// of a path can reorder or hide text where the line is shown. In valid JSON these characters
/// stand only inside strings. `process_line` applies it to `diff.diagnostics` answers only.
#[must_use]
pub fn escape_hidden(line: &str) -> Cow<'_, str> {
    let Some(first) = line.find(is_unsafe_in_untrusted_text) else {
        return Cow::Borrowed(line);
    };
    let mut out = String::from(&line[..first]);
    for c in line[first..].chars() {
        if is_unsafe_in_untrusted_text(c) {
            for unit in c.encode_utf16(&mut [0; 2]) {
                let _ = write!(out, "\\u{unit:04x}");
            }
        } else {
            out.push(c);
        }
    }
    Cow::Owned(out)
}

/// The bytes `value` takes in the answer line, as it is emitted.
fn size(value: &Value) -> usize {
    escape_hidden(&deep_diff_forge_core::json_line_safe(&value.to_string())).len()
}

/// `dg0.<input_sha256 first 16 hex>.<scorer_sha256 first 16 hex>.<index of the next diagnostic>`.
fn cursor(seal: &InputSeal, index: usize) -> String {
    format!(
        "dg0.{}.{}.{index}",
        &seal.input_sha256[..16],
        &scorer_sha256()[..16]
    )
}

/// The index a page starts at: 0 without a cursor; else a cursor this patch and this engine
/// minted, whose index is canonical decimal, not 0 and below `total`.
fn resume(page: &Page, seal: &InputSeal, total: usize) -> Result<usize, PageError> {
    let Some(text) = page.cursor.as_deref() else {
        return Ok(0);
    };
    let parts: Vec<&str> = text.split('.').collect();
    let hex = |s: &str| s.len() == 16 && s.bytes().all(|b| b.is_ascii_hexdigit());
    let ["dg0", patch, engine, index] = parts[..] else {
        return Err(PageError::Cursor(format!(
            "it is not a dg0 cursor ({} bytes)",
            text.len()
        )));
    };
    if !hex(patch) || !hex(engine) {
        return Err(PageError::Cursor(format!(
            "it is not a dg0 cursor ({} bytes)",
            text.len()
        )));
    }
    if patch != &seal.input_sha256[..16] {
        return Err(PageError::Cursor("it belongs to another patch".into()));
    }
    if engine != &scorer_sha256()[..16] {
        return Err(PageError::Cursor("it belongs to another engine".into()));
    }
    let canonical =
        !index.is_empty() && index.bytes().all(|b| b.is_ascii_digit()) && !index.starts_with('0');
    match canonical.then(|| index.parse::<usize>().ok()).flatten() {
        Some(n) if n < total => Ok(n),
        _ => Err(PageError::Cursor(
            "its position is outside the document".into(),
        )),
    }
}

/// For each diagnostic, the index of the `cut` entry that travels with it: a cut file's entry
/// goes with that file's first diagnostic (a cut file keeps located diagnostics, so it has one).
fn cut_entries(
    seal: &InputSeal,
    ranked: &[deep_diff_forge_graph::RankedFile],
) -> Vec<Option<usize>> {
    let (mut at, mut entries) = (0, Vec::new());
    for file in ranked {
        let one = diagnostics(seal, std::slice::from_ref(file));
        let count = one.diagnostics.len();
        entries.extend((0..count).map(|i| (i == 0 && !one.cut.is_empty()).then_some(at)));
        at += usize::from(!one.cut.is_empty());
    }
    entries
}

/// The ids a written diagnostic names (`repairs` and `not_accepted`), whose text `texts` holds.
fn ids(item: &Value) -> impl Iterator<Item = &str> {
    ["repairs", "not_accepted"]
        .into_iter()
        .filter_map(|key| item[key].as_array())
        .flatten()
        .filter_map(Value::as_str)
}

/// One page of the diagnostics document of `review`: whole diagnostics from the cursor's index,
/// as many as fit `max_bytes` with the header (each with its file's `cut` entry when it is that
/// file's first, and the `texts` it names), and `page`. The page as emitted is within `max_bytes`.
///
/// # Errors
/// [`PageError`] for a budget out of range, a cursor this document's pages could not have
/// minted, or a first diagnostic that does not fit the budget with the header.
pub fn page(review: &SealedReview, page: &Page) -> Result<Value, PageError> {
    if !(MIN_PAGE_BYTES..=MAX_MAX_BYTES).contains(&page.max_bytes) {
        return Err(PageError::Budget(page.max_bytes));
    }
    let seal = review.seal();
    let ranked = rank(review.files());
    let document = diagnostics(seal, &ranked);
    let bytes = document.to_json();
    let total = document.diagnostics.len();
    let first = resume(page, seal, total)?;
    let whole: Value = serde_json::from_str(&bytes).unwrap_or(Value::Null);
    let empty = Vec::new();
    let items = whole["diagnostics"].as_array().unwrap_or(&empty);
    let cuts = whole["cut"].as_array().unwrap_or(&empty);
    let cut_of = cut_entries(seal, &ranked);
    // The header as large as any page of this document can make it: every number at most
    // `total`, and a cursor with the longest index.
    let mut value = whole.clone();
    value["cut"] = json!([]);
    value["diagnostics"] = json!([]);
    value["texts"] = json!({});
    value["page"] = page_member(total, total, total, Some(&cursor(seal, total)), &bytes);
    let mut used = size(&value);
    let (mut end, mut named, mut cut_count) = (first, BTreeSet::new(), 0);
    while end < total {
        let item = &items[end];
        let mut cost = size(item) + usize::from(end > first);
        let cut = cut_of[end].and_then(|c| cuts.get(c));
        if let Some(entry) = cut {
            cost += size(entry) + usize::from(cut_count > 0);
        }
        let new: Vec<&str> = ids(item).filter(|id| !named.contains(*id)).collect();
        for (k, id) in new.iter().enumerate() {
            let comma = usize::from(!named.is_empty() || k > 0);
            cost += size(&json!(id)) + 1 + size(&whole["texts"][*id]) + comma;
        }
        if used + cost > page.max_bytes {
            if end == first {
                return Err(PageError::Item {
                    max_bytes: page.max_bytes,
                    item: end,
                    bytes: used + cost,
                });
            }
            break;
        }
        used += cost;
        named.extend(new.into_iter().map(str::to_owned));
        cut_count += usize::from(cut.is_some());
        end += 1;
    }
    let texts: Map<String, Value> = named
        .into_iter()
        .map(|id| {
            let text = whole["texts"][id.as_str()].clone();
            (id, text)
        })
        .collect();
    value["cut"] = (first..end)
        .filter_map(|i| cut_of[i].and_then(|c| cuts.get(c)).cloned())
        .collect();
    value["diagnostics"] = items[first..end].to_vec().into();
    value["texts"] = Value::Object(texts);
    let next = (end < total).then(|| cursor(seal, end));
    value["page"] = page_member(first, end - first, total, next.as_deref(), &bytes);
    Ok(value)
}

fn page_member(
    first: usize,
    count: usize,
    total: usize,
    next: Option<&str>,
    document: &str,
) -> Value {
    json!({
        "first": first,
        "count": count,
        "total": total,
        "next_cursor": next,
        "document_sha256": sha256_hex(document.as_bytes()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use deep_diff_forge_patch::InputGuards;

    fn review(text: &str) -> SealedReview {
        let guards = InputGuards {
            files: false,
            hunks: false,
        };
        SealedReview::new(text.as_bytes(), guards, env!("CARGO_PKG_VERSION")).unwrap()
    }

    fn at(max_bytes: usize, cursor: Option<String>) -> Page {
        Page { max_bytes, cursor }
    }

    /// A file of `n` process spawns at `path` (33 or more cuts its evidence list at 32 rows).
    fn spawns(path: &str, n: usize) -> String {
        let mut text = format!(
            "diff --git a/{path} b/{path}\nnew file mode 100644\n--- /dev/null\n+++ b/{path}\n@@ -0,0 +1,{n} @@\n"
        );
        for i in 0..n {
            let _ = writeln!(text, "+std::process::Command::new(\"p{i}\");");
        }
        text
    }

    /// Every page from the first, following `next_cursor`.
    fn pages(review: &SealedReview, max_bytes: usize) -> Vec<Value> {
        let (mut out, mut cursor) = (Vec::new(), None);
        loop {
            let value = page(review, &at(max_bytes, cursor)).unwrap();
            cursor = value["page"]["next_cursor"].as_str().map(str::to_owned);
            out.push(value);
            if cursor.is_none() {
                return out;
            }
        }
    }

    /// The header (`schema` to `counts`, and `page`) has no path and no list, so its worst case
    /// is bounded: every number at 20 digits, the longest judge state with a ranker digest, a
    /// 10-byte version and a cursor with a 20-digit index.
    #[test]
    fn the_page_header_fits_the_smallest_page() {
        let review = review(&spawns("src/a.rs", 40));
        let mut header = page(&review, &at(65_536, None)).unwrap();
        header["cut"] = json!([]);
        header["diagnostics"] = json!([]);
        header["texts"] = json!({});
        let big = u64::MAX;
        assert_eq!(big.to_string().len(), 20);
        header["tool"]["version"] = json!("0.11.0-dev");
        header["judge"] = json!({"state": "none_admitted", "ranker_sha256": "f".repeat(64)});
        for member in ["files", "diagnostics", "agent", "person", "cut_files"] {
            header["counts"][member] = json!(big);
        }
        for member in ["first", "count", "total"] {
            header["page"][member] = json!(big);
        }
        header["page"]["next_cursor"] =
            json!(format!("dg0.{}.{}.{big}", "f".repeat(16), "f".repeat(16)));
        let bytes = size(&header);
        assert!(
            bytes <= HEADER_BOUND && HEADER_BOUND < MIN_PAGE_BYTES,
            "the page header takes {bytes} bytes, over the bound {HEADER_BOUND}: {header}"
        );
    }

    /// Pages hold whole diagnostics with the answer as emitted within the budget, each file's
    /// `cut` entry travels with its first diagnostic, and the pages put together are the document.
    #[test]
    fn pages_hold_whole_items_within_the_budget() {
        let mut text = String::new();
        for f in 0..6 {
            text.push_str(&spawns(
                &format!("src/m{f}/{}\u{202e}.rs", "n".repeat(30 * f)),
                20 + 5 * f,
            ));
        }
        let review = review(&text);
        let ranked = rank(review.files());
        let document = diagnostics(review.seal(), &ranked).to_json();
        let whole: Value = serde_json::from_str(&document).unwrap();
        assert_eq!(
            page(&review, &at(MIN_PAGE_BYTES - 1, None))
                .unwrap_err()
                .to_string(),
            "max_bytes 4095 is outside 4096..=16777216"
        );
        for max_bytes in [MIN_PAGE_BYTES, 6000, 16_384, 65_536] {
            let (mut seen, mut cut_paths, mut texts) = (Vec::new(), Vec::new(), Map::new());
            let all = pages(&review, max_bytes);
            assert!(
                max_bytes > 16_384 || all.len() > 1,
                "one page at {max_bytes}"
            );
            for value in all {
                let line = escape_hidden(&deep_diff_forge_core::json_line_safe(&value.to_string()))
                    .into_owned();
                assert!(
                    line.len() <= max_bytes,
                    "a page of {} bytes at max_bytes {max_bytes}",
                    line.len()
                );
                assert!(!line.contains('\u{202e}'), "a raw hidden character");
                let items = value["diagnostics"].as_array().unwrap();
                assert!(!items.is_empty(), "an empty page at {max_bytes}");
                for entry in value["cut"].as_array().unwrap() {
                    let path = &entry["path"];
                    assert!(
                        items.first().is_some_and(|d| &d["path"] == path)
                            || items
                                .windows(2)
                                .any(|w| w[1]["path"] == *path && w[0]["path"] != *path),
                        "the cut entry of {path} is on a page without the file's first diagnostic"
                    );
                    cut_paths.push(entry.clone());
                }
                for member in ["schema", "input_sha256", "tool", "judge", "counts"] {
                    assert_eq!(value[member], whole[member]);
                }
                assert_eq!(value["page"]["first"], seen.len());
                assert_eq!(
                    value["page"]["document_sha256"],
                    sha256_hex(document.as_bytes())
                );
                texts.extend(value["texts"].as_object().unwrap().clone());
                seen.extend(items.iter().cloned());
            }
            assert_eq!(Value::from(seen), whole["diagnostics"], "at {max_bytes}");
            assert_eq!(Value::from(cut_paths), whole["cut"], "at {max_bytes}");
            assert_eq!(Value::Object(texts), whole["texts"], "at {max_bytes}");
        }
        let mut one = page(&review, &at(MAX_MAX_BYTES, None)).unwrap();
        assert!(one["page"]["next_cursor"].is_null());
        one.as_object_mut().unwrap().remove("page");
        assert_eq!(one, whole);
    }

    /// A cursor names its patch and its engine by their digests' first 16 hex digits: a cursor
    /// of another patch, of another engine (a daemon replaced between pages) or of a position no
    /// page minted is refused, and the message holds no byte of the cursor.
    #[test]
    fn a_cursor_from_another_engine_is_refused() {
        let review = review(&spawns("src/a.rs", 40));
        let first = page(&review, &at(MIN_PAGE_BYTES, None)).unwrap();
        let next = first["page"]["next_cursor"].as_str().unwrap().to_owned();
        let total = first["page"]["total"].as_u64().unwrap();
        let input = &review.seal().input_sha256[..16];
        let scorer = &scorer_sha256()[..16];
        assert!(page(&review, &at(MIN_PAGE_BYTES, Some(next.clone()))).is_ok());
        let refused = |cursor: String| match page(&review, &at(MIN_PAGE_BYTES, Some(cursor))) {
            Err(e) => e.to_string(),
            Ok(v) => format!("served page {}", v["page"]),
        };
        let other_engine = format!("dg0.{input}.{}.1", "0".repeat(16));
        assert_eq!(
            refused(other_engine),
            "invalid cursor: it belongs to another engine"
        );
        let other_patch = format!("dg0.{}.{scorer}.1", "0".repeat(16));
        assert_eq!(
            refused(other_patch),
            "invalid cursor: it belongs to another patch"
        );
        let past = total.to_string();
        for index in ["0", "01", "+1", "1x", "", &past, "99999999999999999999999"] {
            assert_eq!(
                refused(format!("dg0.{input}.{scorer}.{index}")),
                "invalid cursor: its position is outside the document",
                "{index}"
            );
        }
        let marker = "zq7MARKER\u{202e}";
        for text in [
            format!("dg1.{input}.{scorer}.1"),
            format!("{next}.{marker}"),
            marker.to_owned(),
        ] {
            let message = refused(text.clone());
            assert_eq!(
                message,
                format!(
                    "invalid cursor: it is not a dg0 cursor ({} bytes)",
                    text.len()
                )
            );
        }
    }

    /// One diagnostic that cannot fit `max_bytes` with the header is refused and named; it is
    /// never cut and no page is ever over the budget. The second case is a cut file whose first
    /// diagnostic fits alone but not with its `cut` entry.
    #[test]
    fn an_item_over_the_budget_is_refused_not_cut() {
        let long = format!("src/{}.rs", "a".repeat(4093));
        assert_eq!(long.len(), 4100);
        let first = review(&spawns(&long, 2));
        let refused = match page(&first, &at(MIN_PAGE_BYTES, None)) {
            Err(e) => e,
            Ok(v) => panic!(
                "a page over the budget was served: {} bytes at max_bytes 4096",
                size(&v)
            ),
        };
        let PageError::Item { item, bytes, .. } = refused.clone() else {
            panic!("{refused}");
        };
        assert_eq!(item, 0);
        assert!(bytes > MIN_PAGE_BYTES);
        assert_eq!(
            refused.to_string(),
            format!(
                "max_bytes 4096 is below the size of item 0 with the page header ({bytes} bytes)"
            )
        );
        // At a larger budget the item fits beside the header.
        let served = page(&first, &at(bytes, None)).unwrap();
        assert_eq!(served["page"]["first"], 0);
        assert!(served["page"]["count"].as_u64().unwrap() >= 1);
        assert!(size(&served) <= bytes);
        // A cut file: its first diagnostic carries its `cut` entry, whose path doubles the cost.
        let path = format!("src/{}.rs", "b".repeat(2000));
        let cut = review(&spawns(&path, 40));
        let whole: Value =
            serde_json::from_str(&diagnostics(cut.seal(), &rank(cut.files())).to_json()).unwrap();
        assert_eq!(whole["counts"]["cut_files"], 1);
        let with_cut = size(&whole["diagnostics"][0]) + size(&whole["cut"][0]) + 1;
        let mut found = false;
        for max_bytes in (MIN_PAGE_BYTES..MIN_PAGE_BYTES + 2 * with_cut).step_by(16) {
            match page(&cut, &at(max_bytes, None)) {
                Ok(v) => {
                    assert_eq!(v["cut"].as_array().unwrap().len(), 1, "at {max_bytes}");
                    assert!(size(&v) <= max_bytes);
                    break;
                }
                Err(PageError::Item { item: 0, bytes, .. }) => {
                    assert!(bytes > max_bytes);
                    found = true;
                }
                Err(other) => panic!("{other}"),
            }
        }
        assert!(found, "no budget refused the cut file's first item");
    }

    /// An input with no patch is answered as `diff.rank` answers it: the (empty) document.
    #[test]
    fn an_input_with_no_patch_is_the_empty_document() {
        for text in ["", "This is prose, not a patch.\n"] {
            let review = review(text);
            let mut value = page(&review, &at(65_536, None)).unwrap();
            assert_eq!(value["page"]["total"], 0);
            value.as_object_mut().unwrap().remove("page");
            let empty: Value =
                serde_json::from_str(&diagnostics(review.seal(), &[]).to_json()).unwrap();
            assert_eq!(value, empty);
            let cursor = format!(
                "dg0.{}.{}.0",
                &review.seal().input_sha256[..16],
                &scorer_sha256()[..16]
            );
            assert!(page(&review, &at(65_536, Some(cursor))).is_err());
        }
    }

    /// The escape keeps the value and leaves nothing hidden raw: a pair above U+FFFF.
    #[test]
    fn escape_hidden_keeps_the_value() {
        let value = json!({"path": "a\u{202e}b\u{e0041}c\u{200b}d", "n": 1});
        let line = escape_hidden(&value.to_string()).into_owned();
        assert_eq!(
            line,
            "{\"n\":1,\"path\":\"a\\u202eb\\udb40\\udc41c\\u200bd\"}"
        );
        assert_eq!(serde_json::from_str::<Value>(&line).unwrap(), value);
        assert!(matches!(escape_hidden("{\"a\":\"b\"}"), Cow::Borrowed(_)));
    }
}
