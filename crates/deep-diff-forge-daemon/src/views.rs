//! Budgeted views of one sealed review, for agents (expert circle 2026-10-28, Tier 1).
//!
//! An agent should learn where to look before it reads anything large. [`outline`] lists every
//! file in review-risk order with its hunks' addresses, headings and counts, and no line text.
//! [`hunks`] returns chosen hunks, verbatim, by address or by path. Both carry the review's seal,
//! keep every answer within a byte budget, and continue through a cursor that is bound to the
//! patch and to the selection, so a page can never be mixed with another patch's.
//!
//! The views read the sealed review and change nothing in it: a hunk is returned line for line
//! as `review.v0` holds it, and a hunk too large for one page is continued at a line, never cut
//! inside one.

use deep_diff_forge_core::{InputSeal, PatchHunk, PatchLineKind, ReviewFile};
use deep_diff_forge_patch::SealedReview;
use serde_json::{Value, json};

/// The budget when the caller names none: about 16k tokens of JSON.
pub const DEFAULT_MAX_BYTES: usize = 64 * 1024;
/// The smallest budget accepted; below it a page could not hold its own envelope.
pub const MIN_MAX_BYTES: usize = 1024;
/// The largest budget accepted.
pub const MAX_MAX_BYTES: usize = 16 * 1024 * 1024;

/// Bytes kept back for a page's envelope (schema, seal, cursor, counts).
const ENVELOPE_BYTES: usize = 512;

/// Which hunks [`hunks`] returns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection {
    /// Every hunk, files in review-risk order.
    All,
    /// These hunks, by address, in the order given.
    Addresses(Vec<String>),
    /// Every hunk of these files, in the order given.
    Paths(Vec<String>),
}

/// A page request: the byte budget, and where the previous page stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    /// The most bytes one answer may take (serialized JSON).
    pub max_bytes: usize,
    /// The previous page's `next_cursor`, or `None` for the first page.
    pub cursor: Option<String>,
}

impl Default for Page {
    fn default() -> Self {
        Self {
            max_bytes: DEFAULT_MAX_BYTES,
            cursor: None,
        }
    }
}

/// Why a view was refused. Each is a mistake in the request, never in the patch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewError {
    /// `max_bytes` is outside [`MIN_MAX_BYTES`]..=[`MAX_MAX_BYTES`].
    Budget(usize),
    /// The cursor is malformed, or belongs to another patch or another selection.
    Cursor(String),
    /// No hunk of this patch has the address.
    UnknownAddress(String),
    /// No file of this patch has the path.
    UnknownPath(String),
}

impl std::fmt::Display for ViewError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Budget(n) => write!(
                f,
                "max_bytes {n} is outside {MIN_MAX_BYTES}..={MAX_MAX_BYTES}"
            ),
            Self::Cursor(why) => write!(f, "invalid cursor: {why}"),
            Self::UnknownAddress(a) => write!(
                f,
                "no hunk of this patch has the address {}",
                deep_diff_forge_core::display_safe(a)
            ),
            Self::UnknownPath(p) => write!(
                f,
                "no file of this patch has the path {}",
                deep_diff_forge_core::display_safe(p)
            ),
        }
    }
}

impl std::error::Error for ViewError {}

/// Files in review-risk order (highest first, then by path), as `rank` orders them.
fn risk_order(files: &[ReviewFile]) -> Vec<(usize, u32, Vec<&'static str>)> {
    let mut order: Vec<(usize, u32, Vec<&'static str>)> = files
        .iter()
        .enumerate()
        .map(|(i, file)| {
            let (score, signals) = deep_diff_forge_graph::score_file(file);
            (i, score, signals.iter().map(|s| s.label()).collect())
        })
        .collect();
    order.sort_by(|a, b| {
        b.1.cmp(&a.1)
            .then_with(|| files[a.0].path.cmp(&files[b.0].path))
    });
    order
}

fn seal_members(seal: &InputSeal) -> (Value, Value) {
    (
        json!(seal.input_sha256),
        json!({"name": InputSeal::TOOL_NAME, "version": seal.tool_version}),
    )
}

fn line_counts(hunk: &PatchHunk) -> (usize, usize) {
    hunk.lines
        .iter()
        .fold((0, 0), |(a, d), line| match line.kind {
            PatchLineKind::Added => (a + 1, d),
            PatchLineKind::Removed => (a, d + 1),
            PatchLineKind::Context => (a, d),
        })
}

/// A hunk's header facts, without its lines.
fn hunk_head(file: &ReviewFile, hunk: &PatchHunk) -> serde_json::Map<String, Value> {
    let (old_lines, new_lines) = hunk.counts();
    let (additions, deletions) = line_counts(hunk);
    let mut hazards: Vec<&str> = Vec::new();
    for line in &hunk.lines {
        for hazard in deep_diff_forge_core::text_hazards(&line.text) {
            if !hazards.contains(&hazard) {
                hazards.push(hazard);
            }
        }
    }
    let Value::Object(mut map) = json!({
        "address": hunk.address(&file.path),
        "old_start": hunk.old_start,
        "old_lines": old_lines,
        "new_start": hunk.new_start,
        "new_lines": new_lines,
        "section": hunk.section,
        "additions": additions,
        "deletions": deletions,
    }) else {
        unreachable!("json! of an object literal is an object")
    };
    if !hazards.is_empty() {
        map.insert("hazards".into(), json!(hazards));
    }
    map
}

/// One line as `review.v0` writes it.
fn line_json(line: &deep_diff_forge_core::PatchLine) -> Value {
    let mut value = json!({
        "kind": match line.kind {
            PatchLineKind::Context => "context",
            PatchLineKind::Added => "added",
            PatchLineKind::Removed => "removed",
        },
        "old_line": line.old_line,
        "new_line": line.new_line,
        "text": line.text,
    });
    if line.no_newline {
        value["no_newline"] = json!(true);
    }
    if line.lossy {
        value["lossy"] = json!(true);
    }
    let hazards = deep_diff_forge_core::text_hazards(&line.text);
    if !hazards.is_empty() {
        value["hazards"] = json!(hazards);
    }
    value
}

fn size(value: &Value) -> usize {
    value.to_string().len()
}

/// A cursor: the seal's first 16 hex digits, the selection's digest, then the item and the
/// position inside it where the next page starts.
fn cursor(seal: &InputSeal, selection: &str, item: usize, inner: usize) -> String {
    format!(
        "{}.{}.{item}.{inner}",
        &seal.input_sha256[..16],
        &deep_diff_forge_core::sha256_hex(selection.as_bytes())[..8]
    )
}

fn resume(page: &Page, seal: &InputSeal, selection: &str) -> Result<(usize, usize), ViewError> {
    if !(MIN_MAX_BYTES..=MAX_MAX_BYTES).contains(&page.max_bytes) {
        return Err(ViewError::Budget(page.max_bytes));
    }
    let Some(text) = page.cursor.as_deref() else {
        return Ok((0, 0));
    };
    let parts: Vec<&str> = text.split('.').collect();
    let [patch, picked, item, inner] = parts[..] else {
        return Err(ViewError::Cursor(
            "expected four dot-separated fields".into(),
        ));
    };
    if patch != &seal.input_sha256[..16] {
        return Err(ViewError::Cursor("it belongs to another patch".into()));
    }
    if picked != &deep_diff_forge_core::sha256_hex(selection.as_bytes())[..8] {
        return Err(ViewError::Cursor("it belongs to another selection".into()));
    }
    match (item.parse(), inner.parse()) {
        (Ok(item), Ok(inner)) => Ok((item, inner)),
        _ => Err(ViewError::Cursor("its positions are not numbers".into())),
    }
}

/// `deep-diff-forge.review-outline.v0`: every file in review-risk order, each with its score,
/// signals, counts and hunks (address, ranges, heading, counts), and no line text.
///
/// # Errors
/// [`ViewError::Budget`] or [`ViewError::Cursor`] for a bad page request.
pub fn outline(review: &SealedReview, page: &Page) -> Result<Value, ViewError> {
    let (files, seal) = (review.files(), review.seal());
    let (start_file, start_hunk) = resume(page, seal, "outline")?;
    let order = risk_order(files);
    let (input_sha256, tool) = seal_members(seal);
    let hunk_total: usize = files.iter().map(|f| f.patch_twin.hunks.len()).sum();
    let (additions, deletions) = files
        .iter()
        .flat_map(|f| &f.patch_twin.hunks)
        .map(line_counts)
        .fold((0, 0), |(a, d), (x, y)| (a + x, d + y));
    let mut budget = page.max_bytes - ENVELOPE_BYTES;
    let mut entries: Vec<Value> = Vec::new();
    let mut next = None;
    'files: for (position, (index, score, signals)) in order.iter().enumerate().skip(start_file) {
        let file = &files[*index];
        let first = if position == start_file {
            start_hunk
        } else {
            0
        };
        let mut entry = json!({
            "path": file.path,
            "hazards": deep_diff_forge_core::path_hazards(&file.path),
            "status": file.status.label(),
            "score": score,
            "signals": signals,
            "hunk_count": file.patch_twin.hunks.len(),
            "first_hunk": first,
            "hunks": [],
            "complete": true,
        });
        let mut used = size(&entry) + 1;
        for (h, hunk) in file.patch_twin.hunks.iter().enumerate().skip(first) {
            let head = Value::Object(hunk_head(file, hunk));
            let cost = size(&head) + 1;
            // A page always makes progress: its first hunk is listed even when it is over.
            let first_on_page = entries.is_empty() && h == first;
            if used + cost > budget && !first_on_page {
                if h > first {
                    entry["complete"] = json!(false);
                    entries.push(entry);
                }
                next = Some(cursor(seal, "outline", position, h));
                break 'files;
            }
            used += cost;
            if let Some(list) = entry["hunks"].as_array_mut() {
                list.push(head);
            }
        }
        if used > budget && !entries.is_empty() {
            next = Some(cursor(seal, "outline", position, first));
            break;
        }
        budget = budget.saturating_sub(used);
        entries.push(entry);
    }
    let listed = entries.len();
    Ok(json!({
        "schema": "deep-diff-forge.review-outline.v0",
        "input_sha256": input_sha256,
        "tool": tool,
        "summary": {
            "files": files.len(),
            "hunks": hunk_total,
            "additions": additions,
            "deletions": deletions,
        },
        "files": entries,
        "files_listed": listed,
        "next_cursor": next,
    }))
}

/// `deep-diff-forge.review-hunks.v0`: the selected hunks, verbatim, each with its address, path,
/// ranges and heading. A hunk too large for the page is continued at a line (`line_offset`,
/// `complete: false`); a single line is never cut, so the only page over budget is one whose
/// first line alone exceeds it.
///
/// # Errors
/// [`ViewError`] for a bad page request, an unknown address or an unknown path.
pub fn hunks(
    review: &SealedReview,
    selection: &Selection,
    page: &Page,
) -> Result<Value, ViewError> {
    let (files, seal) = (review.files(), review.seal());
    let (units, key) = select(files, selection)?;
    let (start_unit, start_line) = resume(page, seal, &key)?;
    let (input_sha256, tool) = seal_members(seal);
    let mut budget = page.max_bytes - ENVELOPE_BYTES;
    let mut out = Vec::new();
    let mut next = None;
    for (position, &(f, h)) in units.iter().enumerate().skip(start_unit) {
        let (file, hunk) = (&files[f], &files[f].patch_twin.hunks[h]);
        let offset = if position == start_unit {
            start_line
        } else {
            0
        };
        let mut entry = Value::Object(hunk_head(file, hunk));
        entry["path"] = json!(file.path);
        entry["line_offset"] = json!(offset);
        entry["complete"] = json!(true);
        entry["lines"] = json!([]);
        let mut used = size(&entry) + 1;
        let mut taken = 0;
        for line in hunk.lines.iter().skip(offset) {
            let value = line_json(line);
            let cost = size(&value) + 1;
            // A page always makes progress: its first line is taken even when it alone is over.
            if used + cost > budget && !(out.is_empty() && taken == 0) {
                break;
            }
            used += cost;
            taken += 1;
            if let Some(lines) = entry["lines"].as_array_mut() {
                lines.push(value);
            }
        }
        let end = offset + taken;
        if end < hunk.lines.len() {
            if taken == 0 {
                next = Some(cursor(seal, &key, position, offset));
                break;
            }
            entry["complete"] = json!(false);
            out.push(entry);
            next = Some(cursor(seal, &key, position, end));
            break;
        }
        budget = budget.saturating_sub(used);
        out.push(entry);
    }
    let finished = out.iter().filter(|e| e["complete"] == true).count();
    let remaining = units.len() - start_unit.min(units.len()) - finished;
    Ok(json!({
        "schema": "deep-diff-forge.review-hunks.v0",
        "input_sha256": input_sha256,
        "tool": tool,
        "hunks": out,
        "selected_hunks": units.len(),
        "remaining_hunks": remaining,
        "next_cursor": next,
    }))
}

/// The (file, hunk) pairs a selection names, in order, and the key its cursor is bound to.
fn select(
    files: &[ReviewFile],
    selection: &Selection,
) -> Result<(Vec<(usize, usize)>, String), ViewError> {
    match selection {
        Selection::All => {
            let units = risk_order(files)
                .into_iter()
                .flat_map(|(f, _, _)| (0..files[f].patch_twin.hunks.len()).map(move |h| (f, h)))
                .collect();
            Ok((units, "all".into()))
        }
        Selection::Addresses(addresses) => {
            let mut units = Vec::with_capacity(addresses.len());
            for address in addresses {
                let found = files.iter().enumerate().find_map(|(f, file)| {
                    file.patch_twin
                        .hunks
                        .iter()
                        .position(|h| &h.address(&file.path) == address)
                        .map(|h| (f, h))
                });
                units.push(found.ok_or_else(|| ViewError::UnknownAddress(address.clone()))?);
            }
            Ok((units, format!("a:{}", addresses.join("\n"))))
        }
        Selection::Paths(paths) => {
            let mut units = Vec::new();
            for path in paths {
                let f = files
                    .iter()
                    .position(|file| &file.path == path)
                    .ok_or_else(|| ViewError::UnknownPath(path.clone()))?;
                units.extend((0..files[f].patch_twin.hunks.len()).map(|h| (f, h)));
            }
            Ok((units, format!("p:{}", paths.join("\n"))))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deep_diff_forge_patch::InputGuards;

    /// Six files, one with a 400-line hunk and one line longer than the smallest budget.
    fn patch() -> String {
        use std::fmt::Write as _;
        let mut p = String::new();
        for f in 0..5 {
            let _ = write!(p, "--- a/src/f{f}.rs\n+++ b/src/f{f}.rs\n");
            for h in 0..3 {
                let start = 1 + h * 20;
                let _ = write!(
                    p,
                    "@@ -{start},2 +{start},2 @@ fn h{h}()\n ctx\n-old{h}\n+new{h}\n"
                );
            }
        }
        p.push_str("--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,400 +1,400 @@\n");
        for i in 0..400 {
            let _ = writeln!(p, "-line {i}\n+LINE {i}");
        }
        let _ = writeln!(
            p,
            "--- a/src/long.rs\n+++ b/src/long.rs\n@@ -1 +1 @@\n-a\n+{}",
            "x".repeat(3000)
        );
        p
    }

    fn review(text: &str) -> SealedReview {
        SealedReview::new(
            text.as_bytes(),
            InputGuards {
                files: false,
                hunks: false,
            },
            "0.0.0-test",
        )
        .unwrap()
    }

    fn page(max_bytes: usize, cursor: Option<String>) -> Page {
        Page { max_bytes, cursor }
    }

    fn all_lines(review: &SealedReview) -> std::collections::HashMap<String, Vec<Value>> {
        review
            .files()
            .iter()
            .flat_map(|f| f.patch_twin.hunks.iter().map(move |h| (f, h)))
            .map(|(f, h)| (h.address(&f.path), h.lines.iter().map(line_json).collect()))
            .collect()
    }

    /// Paging returns every line of every hunk exactly once, in order, verbatim, and each page
    /// fits its budget unless its first line alone is longer.
    #[test]
    fn hunk_pages_are_complete_ordered_and_bounded() {
        let review = review(&patch());
        let expected = all_lines(&review);
        for max_bytes in [MIN_MAX_BYTES, 2048, 8192, DEFAULT_MAX_BYTES] {
            let mut got = std::collections::HashMap::<String, Vec<Value>>::default();
            let mut completed = 0;
            let mut order = Vec::new();
            let mut cursor = None;
            for _ in 0..10_000 {
                let answer =
                    hunks(&review, &Selection::All, &page(max_bytes, cursor.clone())).unwrap();
                let bytes = answer.to_string().len();
                let entries = answer["hunks"].as_array().unwrap();
                assert!(!entries.is_empty(), "a page always makes progress");
                completed += entries.iter().filter(|e| e["complete"] == true).count();
                assert_eq!(
                    answer["remaining_hunks"],
                    json!(expected.len() - completed),
                    "budget {max_bytes}"
                );
                let one_long_line = entries.len() == 1
                    && entries[0]["lines"][0]["text"]
                        .as_str()
                        .is_some_and(|t| t.len() > max_bytes);
                assert!(bytes <= max_bytes || one_long_line, "{bytes} > {max_bytes}");
                for entry in entries {
                    let address = entry["address"].as_str().unwrap().to_owned();
                    let lines = got.entry(address.clone()).or_default();
                    assert_eq!(entry["line_offset"], lines.len(), "{address}");
                    lines.extend(entry["lines"].as_array().unwrap().iter().cloned());
                    if order.last() != Some(&address) {
                        order.push(address);
                    }
                }
                let next = answer["next_cursor"].as_str().map(str::to_owned);
                assert!(
                    next.is_none() || next != cursor,
                    "a page always moves the cursor"
                );
                cursor = next;
                if cursor.is_none() {
                    break;
                }
            }
            assert!(cursor.is_none(), "paging ended");
            assert_eq!(got, expected, "budget {max_bytes}");
            assert_eq!(
                order.len(),
                expected.len(),
                "each hunk once, budget {max_bytes}"
            );
        }
    }

    /// The first page holds exactly what a greedy fill of `max_bytes - ENVELOPE_BYTES` allows,
    /// counting each item's serialized size plus its comma: the reference model below, built from
    /// the items' own JSON, must agree with the pager at every budget. A one-byte error in the
    /// pager's accounting moves a page boundary at some budget.
    #[test]
    fn pages_fill_exactly_to_the_budget() {
        let review = review(&patch());
        let all = hunks(&review, &Selection::All, &page(MAX_MAX_BYTES, None)).unwrap();
        let outline_all = outline(&review, &page(MAX_MAX_BYTES, None)).unwrap();
        for max_bytes in (MIN_MAX_BYTES..6000).step_by(23) {
            // hunks: (address, lines, complete) of the first page.
            let mut budget = max_bytes - ENVELOPE_BYTES;
            let mut expected = Vec::new();
            for entry in all["hunks"].as_array().unwrap() {
                let mut base = entry.clone();
                base["lines"] = json!([]);
                let mut used = size(&base) + 1;
                let lines = entry["lines"].as_array().unwrap();
                let mut taken = 0;
                for line in lines {
                    let cost = size(line) + 1;
                    if used + cost > budget && !(expected.is_empty() && taken == 0) {
                        break;
                    }
                    used += cost;
                    taken += 1;
                }
                if taken < lines.len() {
                    if taken > 0 {
                        expected.push((entry["address"].clone(), taken, false));
                    }
                    break;
                }
                budget = budget.saturating_sub(used);
                expected.push((entry["address"].clone(), taken, true));
            }
            let got: Vec<_> =
                hunks(&review, &Selection::All, &page(max_bytes, None)).unwrap()["hunks"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|e| {
                        (
                            e["address"].clone(),
                            e["lines"].as_array().unwrap().len(),
                            e["complete"] == true,
                        )
                    })
                    .collect();
            assert_eq!(got, expected, "hunks at {max_bytes}");

            // outline: (path, hunks, complete) of the first page.
            let mut budget = max_bytes - ENVELOPE_BYTES;
            let mut expected = Vec::new();
            'files: for file in outline_all["files"].as_array().unwrap() {
                let mut base = file.clone();
                base["hunks"] = json!([]);
                let mut used = size(&base) + 1;
                let mut count = 0;
                for (h, head) in file["hunks"].as_array().unwrap().iter().enumerate() {
                    let cost = size(head) + 1;
                    if used + cost > budget && !(expected.is_empty() && h == 0) {
                        if h > 0 {
                            expected.push((file["path"].clone(), count, false));
                        }
                        break 'files;
                    }
                    used += cost;
                    count += 1;
                }
                if used > budget && !expected.is_empty() {
                    break;
                }
                budget = budget.saturating_sub(used);
                expected.push((file["path"].clone(), count, true));
            }
            let got: Vec<_> = outline(&review, &page(max_bytes, None)).unwrap()["files"]
                .as_array()
                .unwrap()
                .iter()
                .map(|f| {
                    (
                        f["path"].clone(),
                        f["hunks"].as_array().unwrap().len(),
                        f["complete"] == true,
                    )
                })
                .collect();
            assert_eq!(got, expected, "outline at {max_bytes}");
        }
    }

    /// The outline lists every file and every hunk address exactly once across its pages, with no
    /// line text, and each page fits its budget.
    #[test]
    fn outline_pages_cover_every_hunk_once() {
        let review = review(&patch());
        let expected: std::collections::BTreeSet<String> = all_lines(&review).into_keys().collect();
        for max_bytes in [MIN_MAX_BYTES, 4096, DEFAULT_MAX_BYTES] {
            let mut seen = Vec::new();
            let mut cursor = None;
            loop {
                let answer = outline(&review, &page(max_bytes, cursor.clone())).unwrap();
                assert!(answer.to_string().len() <= max_bytes, "budget {max_bytes}");
                assert!(!answer.to_string().contains("LINE 1"), "no line text");
                assert_eq!(answer["summary"]["hunks"], expected.len());
                for file in answer["files"].as_array().unwrap() {
                    for hunk in file["hunks"].as_array().unwrap() {
                        seen.push(hunk["address"].as_str().unwrap().to_owned());
                    }
                }
                let next = answer["next_cursor"].as_str().map(str::to_owned);
                assert!(
                    next.is_none() || next != cursor,
                    "a page always moves the cursor"
                );
                cursor = next;
                if cursor.is_none() {
                    break;
                }
            }
            let unique: std::collections::BTreeSet<String> = seen.iter().cloned().collect();
            assert_eq!(seen.len(), unique.len(), "no repeats, budget {max_bytes}");
            assert_eq!(unique, expected, "budget {max_bytes}");
        }
    }

    /// The defaults and the counts are exact: a 64 KiB page, and each hunk's and the summary's
    /// additions and deletions as the lines say.
    #[test]
    fn budgets_and_counts_are_exact() {
        assert_eq!(Page::default().max_bytes, 65_536);
        assert_eq!((MIN_MAX_BYTES, MAX_MAX_BYTES), (1024, 16_777_216));
        let review = review("--- a/x\n+++ b/x\n@@ -1,3 +1,5 @@\n a\n-b\n+c\n+d\n+e\n f\n");
        let answer = outline(&review, &Page::default()).unwrap();
        let hunk = &answer["files"][0]["hunks"][0];
        assert_eq!(
            (&hunk["additions"], &hunk["deletions"]),
            (&json!(3), &json!(1))
        );
        assert_eq!(
            answer["summary"],
            json!({"files": 1, "hunks": 1, "additions": 3, "deletions": 1})
        );
    }

    /// A hunk too big for even the smallest page is still listed, alone, so paging always moves
    /// on: here an 800-byte `@@` heading.
    #[test]
    fn an_oversized_hunk_still_makes_progress() {
        let heading = "h".repeat(800);
        let review = review(&format!(
            "--- a/a\n+++ b/a\n@@ -1 +1 @@ {heading}\n-a\n+b\n--- a/b\n+++ b/b\n@@ -1 +1 @@\n-c\n+d\n"
        ));
        let first = outline(&review, &page(MIN_MAX_BYTES, None)).unwrap();
        let files = first["files"].as_array().unwrap();
        assert_eq!(files.len(), 1, "{first}");
        assert_eq!(files[0]["hunks"].as_array().unwrap().len(), 1);
        assert_eq!(files[0]["complete"], true);
        let cursor = first["next_cursor"].as_str().map(str::to_owned);
        assert!(cursor.is_some());
        let second = outline(&review, &page(MIN_MAX_BYTES, cursor)).unwrap();
        assert_eq!(second["files"].as_array().unwrap().len(), 1);
        assert!(second["next_cursor"].is_null());
    }

    /// The outline puts the riskiest file first, as `rank` does.
    #[test]
    fn outline_is_in_review_risk_order() {
        let review = review(&patch());
        let answer = outline(&review, &Page::default()).unwrap();
        let paths: Vec<&str> = answer["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["path"].as_str().unwrap())
            .collect();
        let ranked: Vec<String> = deep_diff_forge_graph::rank(review.files())
            .into_iter()
            .map(|r| r.path)
            .collect();
        assert_eq!(paths, ranked);
        assert_eq!(paths[0], "src/lib.rs");
        assert_eq!(answer["files"][1]["hunks"][0]["section"], "fn h0()");
    }

    /// Hunks by address come back in the order asked, verbatim; by path, every hunk of the file.
    #[test]
    fn hunks_by_address_and_by_path() {
        let review = review(&patch());
        let expected = all_lines(&review);
        let files = review.files();
        let a = files[3].patch_twin.hunks[2].address(&files[3].path);
        let b = files[0].patch_twin.hunks[0].address(&files[0].path);
        let answer = hunks(
            &review,
            &Selection::Addresses(vec![a.clone(), b.clone()]),
            &Page::default(),
        )
        .unwrap();
        let got: Vec<&str> = answer["hunks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|h| h["address"].as_str().unwrap())
            .collect();
        assert_eq!(got, [a.as_str(), b.as_str()]);
        assert_eq!(answer["hunks"][0]["lines"], json!(expected[&a]));
        assert_eq!(
            (&answer["selected_hunks"], &answer["remaining_hunks"]),
            (&json!(2), &json!(0))
        );
        let first = hunks(&review, &Selection::All, &page(MIN_MAX_BYTES, None)).unwrap();
        let done = first["hunks"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|h| h["complete"] == true)
            .count();
        assert_eq!(first["selected_hunks"], json!(17));
        assert_eq!(first["remaining_hunks"], json!(17 - done));
        let by_path = hunks(
            &review,
            &Selection::Paths(vec!["src/f2.rs".into()]),
            &Page::default(),
        )
        .unwrap();
        assert_eq!(by_path["hunks"].as_array().unwrap().len(), 3);
        assert!(
            by_path["hunks"]
                .as_array()
                .unwrap()
                .iter()
                .all(|h| h["path"] == "src/f2.rs")
        );
    }

    /// A cursor belongs to one patch and one selection; a bad budget, address or path is refused.
    #[test]
    fn requests_that_do_not_fit_are_refused() {
        let first = review(&patch());
        let other = review("--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n");
        let cursor =
            hunks(&first, &Selection::All, &page(MIN_MAX_BYTES, None)).unwrap()["next_cursor"]
                .as_str()
                .unwrap()
                .to_owned();
        let refused =
            |r: &SealedReview, s: &Selection, p: &Page| hunks(r, s, p).unwrap_err().to_string();
        assert!(
            refused(
                &other,
                &Selection::All,
                &page(MIN_MAX_BYTES, Some(cursor.clone()))
            )
            .contains("another patch")
        );
        assert!(
            refused(
                &first,
                &Selection::Paths(vec!["src/f0.rs".into()]),
                &page(MIN_MAX_BYTES, Some(cursor.clone()))
            )
            .contains("another selection")
        );
        assert!(
            refused(
                &first,
                &Selection::All,
                &page(MIN_MAX_BYTES, Some("x.y".into()))
            )
            .contains("four")
        );
        assert!(
            refused(&first, &Selection::All, &page(MIN_MAX_BYTES - 1, None)).contains("max_bytes")
        );
        assert!(
            refused(&first, &Selection::All, &page(MAX_MAX_BYTES + 1, None)).contains("max_bytes")
        );
        assert!(
            refused(
                &first,
                &Selection::Addresses(vec!["00".into()]),
                &Page::default()
            )
            .contains("address")
        );
        assert!(
            refused(
                &first,
                &Selection::Paths(vec!["nope.rs".into()]),
                &Page::default()
            )
            .contains("path")
        );
        assert!(
            outline(&other, &page(MIN_MAX_BYTES, Some(cursor)))
                .unwrap_err()
                .to_string()
                .contains("another patch")
        );
    }
}
