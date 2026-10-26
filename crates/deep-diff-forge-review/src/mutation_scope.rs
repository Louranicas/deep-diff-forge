// Review council: The diff-scoped mutation gate's scope, built from raw base blobs and the sealed workspace bytes.
// Design/status: docs/REVIEW_COUNCIL.md#gate-sandbox-r7
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fcouncil--crates--deep-diff-forge-review--src--mutation_scope
//! The lines a diff-scoped mutation gate mutates.
//!
//! `git diff <base>` of a working tree honours index flags (assume-unchanged, skip-worktree), the
//! candidate's own `.gitattributes` (`-diff`, `binary`) and skips untracked files, so a candidate
//! could shrink its own mutation scope (council r7). Git is never asked to compare anything here.
//! Every base object read is re-hashed against its ID the moment it is read, from the operator's
//! base commit ID down through each tree to each blob, so nothing written to the object store can
//! substitute bytes (councils r8 and r9). Each file is read from the sealed snapshot after its
//! fingerprint is verified, and the zero-context unified diff is built in-process.
use crate::storage::{self, Inventory};
use crate::{Result, ReviewFailure, require};
use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::ops::Range;
use std::path::Path;

/// Largest mutation diff captured at prepare and read back by the mutation gate. Written as a
/// shift, so the only arithmetic mutant (a zero cap) refuses every diff-scoped run.
pub(crate) const MUTATION_DIFF_CAP: u64 = 16 << 20;
/// Line-pair cells for one file's exact LCS. Above it the whole changed middle is reported, which
/// over-includes lines (more mutants) and never omits a changed one.
const LINE_CELL_BUDGET: usize = 16 << 20;
/// Line-pair cells for all files together; past it, further files are reported whole too.
const TOTAL_CELL_BUDGET: usize = 64 << 20;
/// Lines on either side of one file. Bounds the line index and an empty side's table.
const LINE_CAP: usize = 1 << 20;
/// Per-file read budget, matching the inventory's 64 MiB source limit.
const FILE_CAP: u64 = 64 << 20;

/// Refuse a `base` that is not a commit and an ancestor of `HEAD`, before a run directory exists.
/// The base's objects are not trusted here; [`capture`] verifies each one as it reads it.
pub(crate) fn require_base(repository: &Path, base: &str) -> Result<()> {
    storage::git_bytes(
        repository,
        &["merge-base", "--is-ancestor", base, "HEAD"],
        0,
    )
    .map(drop)
    .map_err(|_| ReviewFailure("mutation base must be a commit that is an ancestor of HEAD".into()))
}

/// One object of `kind`, read and then re-hashed by Git (`hash-object`, with Git's own
/// collision-detecting hash) and refused unless it hashes to `oid`. Git does not re-hash a loose
/// object on read, so a forged object stored under a genuine ID is caught here, at the moment of use.
fn verified(repository: &Path, kind: &str, oid: &str) -> Result<Vec<u8>> {
    let bytes = storage::git_bytes(repository, &["cat-file", kind, oid], FILE_CAP)?;
    let hashed = storage::git_input(
        repository,
        // `--literally` hashes without Git's format checks, so a legitimate historical object
        // (zero-padded modes from old tools) is not refused; only the ID has to match.
        &["hash-object", "-t", kind, "--stdin", "--literally"],
        &bytes,
        1024,
    )?;
    require(
        hashed.trim_ascii() == oid.as_bytes(),
        "a base object does not hash to its ID",
    )?;
    Ok(bytes)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut text, byte| {
        let _ = write!(text, "{byte:02x}");
        text
    })
}

/// The blob ID of every file under the `base` commit, keyed by path, reading only verified trees.
fn base_blobs(repository: &Path, base: &str) -> Result<BTreeMap<String, String>> {
    let commit = verified(repository, "commit", base)?;
    let tree = commit
        .strip_prefix(b"tree ")
        .and_then(|rest| rest.get(..base.len()))
        .ok_or_else(|| ReviewFailure("base commit names no tree".into()))?;
    // Raw tree entries end in a binary ID as long as the base ID's bytes.
    let id_len = base.len() / 2;
    let mut blobs = BTreeMap::new();
    let mut pending = vec![(String::new(), String::from_utf8_lossy(tree).into_owned())];
    while let Some((prefix, oid)) = pending.pop() {
        let bytes = verified(repository, "tree", &oid)?;
        let mut entries = bytes.as_slice();
        // Each entry is `<mode> <name>\0<binary id>`; every pass consumes at least one byte.
        while let Some(nul) = entries.iter().position(|b| *b == 0) {
            let (head, tail) = entries.split_at(nul);
            let id = tail
                .get(1..=id_len)
                .ok_or_else(|| ReviewFailure("malformed base tree".into()))?;
            entries = &tail[1 + id_len..];
            let space = head
                .iter()
                .position(|b| *b == b' ')
                .ok_or_else(|| ReviewFailure("malformed base tree".into()))?;
            let (mode, name) = (&head[..space], &head[space + 1..]);
            let Ok(name) = std::str::from_utf8(name) else {
                continue;
            };
            match mode {
                b"40000" => pending.push((format!("{prefix}{name}/"), hex(id))),
                // Regular files only. A submodule's commit is not a file here, and a symlink's
                // blob holds its target, which a regular file with those bytes is not (council r10).
                b"100644" | b"100755" => {
                    blobs.insert(format!("{prefix}{name}"), hex(id));
                }
                _ => {}
            }
        }
        require(entries.is_empty(), "malformed base tree")?;
    }
    Ok(blobs)
}

/// With a `base` already checked by [`require_base`], the zero-context unified diff of every
/// inventoried file in `snapshot` against it. Every file is included, not only `.rs` ones, because
/// `#[path]` can make any file a module. A file absent at `base` (including an untracked one), or
/// one that was not UTF-8 there, diffs against empty. Deleted files add no lines, and a file that is
/// not UTF-8 now cannot be compiled as Rust or mutated, so both are omitted.
pub(crate) fn capture(
    repository: &Path,
    snapshot: &Path,
    inventory: &Inventory,
    base: Option<&str>,
) -> Result<Option<Vec<u8>>> {
    base.map(|base| capture_from(repository, snapshot, inventory, base))
        .transpose()
}
fn capture_from(
    repository: &Path,
    snapshot: &Path,
    inventory: &Inventory,
    base: &str,
) -> Result<Vec<u8>> {
    let blobs = base_blobs(repository, base)?;
    let mut present = Vec::new();
    for path in inventory.paths() {
        let Some(meta) = storage::untrusted_metadata(snapshot, Path::new(path))? else {
            continue;
        };
        require(
            meta.is_file(),
            "source inventory contains symlink or non-file",
        )?;
        // `hash-object --stdin-paths` reads one path per line.
        require(
            !path.chars().any(char::is_control),
            "mutation scope path contains a control character",
        )?;
        present.push(path);
    }
    // One Git process hashes every snapshot file, so unchanged files are skipped unread.
    let listing = present.iter().fold(String::new(), |mut text, path| {
        let _ = writeln!(text, "{}", snapshot.join(path).display());
        text
    });
    let hashed = storage::git_input(
        repository,
        &["hash-object", "--no-filters", "--stdin-paths"],
        listing.as_bytes(),
        FILE_CAP,
    )?;
    let ids: Vec<&[u8]> = hashed
        .split(|b| *b == b'\n')
        .filter(|id| !id.is_empty())
        .collect();
    require(
        ids.len() == present.len(),
        "snapshot hashing was incomplete",
    )?;
    let (mut diff, mut spent) = (Vec::new(), 0);
    for (path, id) in present.into_iter().zip(ids) {
        let base_id = blobs.get(path);
        if base_id.is_some_and(|base_id| base_id.as_bytes() == id) {
            continue;
        }
        let new = storage::read_untrusted(snapshot, Path::new(path), FILE_CAP)?;
        if std::str::from_utf8(&new).is_err() {
            continue;
        }
        let old = match base_id {
            Some(base_id) => verified(repository, "blob", base_id)?,
            None => Vec::new(),
        };
        let old = if std::str::from_utf8(&old).is_ok() {
            old
        } else {
            Vec::new()
        };
        // Headers are line-oriented, a space can end a name for some diff parsers (Git appends a
        // tab after such names), and a leading quote reads as Git's path quoting: any of them
        // could forge or misattribute a header or a hunk. Only a changed text file gets a header,
        // so neither an unchanged `docs/Design Notes.md` (council r13) nor a changed binary
        // `assets/My Logo.png` (council r15) blocks the diff scope.
        require(
            !path.starts_with('"') && !path.chars().any(char::is_whitespace),
            "mutation scope path contains whitespace or a leading quote",
        )?;
        unified(path, &old, &new, &mut spent, &mut diff)?;
        require(
            diff.len() as u64 <= MUTATION_DIFF_CAP,
            "mutation diff exceeds its budget",
        )?;
    }
    Ok(diff)
}

/// Lines with their terminators, so a missing final newline is itself a difference; at most
/// [`LINE_CAP`] of them.
fn lines(bytes: &[u8]) -> Result<Vec<&[u8]>> {
    require(
        bytes
            .split_inclusive(|b| *b == b'\n')
            .take(LINE_CAP)
            .count()
            < LINE_CAP,
        "a changed file has too many lines for the mutation scope",
    )?;
    Ok(bytes.split_inclusive(|b| *b == b'\n').collect())
}

/// Each line's index among the distinct lines seen so far, so the LCS compares integers: the cost
/// of a long line is paid once when it is hashed, not once per table cell.
fn intern<'a>(ids: &mut HashMap<&'a [u8], usize>, lines: &[&'a [u8]]) -> Vec<usize> {
    lines
        .iter()
        .map(|line| {
            let next = ids.len();
            *ids.entry(*line).or_insert(next)
        })
        .collect()
}

/// Index pairs of one longest common subsequence of `old` and `new`, ascending.
fn lcs_pairs(old: &[usize], new: &[usize]) -> Vec<(usize, usize)> {
    let (n, m) = (old.len(), new.len());
    let width = m + 1;
    let mut table = vec![0u32; (n + 1) * width];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            table[i * width + j] = if old[i] == new[j] {
                table[(i + 1) * width + (j + 1)] + 1
            } else {
                table[(i + 1) * width + j].max(table[i * width + (j + 1)])
            };
        }
    }
    // Backtrack with both cursors driven by `for` and `find`, so a broken step yields a wrong
    // answer, never a hang: skip the new lines the LCS leaves unmatched before old[i] (stopping at
    // a match, or where skipping old[i] instead loses nothing), then pair or skip old[i].
    let mut pairs = Vec::new();
    let mut j = 0;
    for i in 0..n {
        j = (j..m)
            .find(|&next| {
                old[i] == new[next] || table[(i + 1) * width + next] == table[i * width + next]
            })
            .unwrap_or(m);
        if j < m && old[i] == new[j] {
            pairs.push((i, j));
            j += 1;
        }
    }
    pairs
}

/// Changed lines of one file: an old-side and a new-side range of absolute line indices.
type Region = (Range<usize>, Range<usize>);

/// The minimal changed regions of `old` against `new`, ascending. The exact LCS runs only while
/// this file and all files so far (`spent` cells) are within budget; otherwise the whole changed
/// middle is one region.
fn regions(old: &[usize], new: &[usize], spent: &mut usize) -> Vec<Region> {
    let prefix = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let suffix = old[prefix..]
        .iter()
        .rev()
        .zip(new[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let (old_end, new_end) = (old.len() - suffix, new.len() - suffix);
    let cells = (old_end - prefix).saturating_mul(new_end - prefix);
    let mut anchors =
        if cells <= LINE_CELL_BUDGET && spent.saturating_add(cells) <= TOTAL_CELL_BUDGET {
            *spent += cells;
            lcs_pairs(&old[prefix..old_end], &new[prefix..new_end])
        } else {
            Vec::new()
        };
    // A sentinel anchor after both middles closes the last region.
    anchors.push((old_end - prefix, new_end - prefix));
    let (mut found, mut i, mut j) = (Vec::new(), 0, 0);
    for (anchor_old, anchor_new) in anchors {
        if anchor_old > i || anchor_new > j {
            found.push((
                prefix + i..prefix + anchor_old,
                prefix + j..prefix + anchor_new,
            ));
        }
        (i, j) = (anchor_old + 1, anchor_new + 1);
    }
    found
}

/// cargo-mutants selects mutants by new-side lines, so a region that only deletes lines selects
/// none (measured with 27.1.0): removing a check would leave its function unmutated. The same holds
/// for a region whose new lines are all blank, comment or attribute lines, such as a `cfg` gate
/// rewritten from `any()` to `all()` (council r10). Widen each such region over the unchanged lines
/// on both sides, through blank, comment and attribute lines, to the nearest other line before and
/// after it (council r9: a neighbour that is only a blank line can sit outside the affected item).
/// Widening stops at the neighbouring regions, and widened regions that then overlap are merged.
/// Unchanged lines are equal on both sides, so the diff stays valid.
fn widen_deletions(found: &[Region], new: &[&[u8]]) -> Vec<Region> {
    // Lines that belong to no item's body: blank, comment and attribute lines (council r10: a
    // comment between a deleted `cfg` and the item it disabled stopped the widening).
    let code = code_lines(new);
    let blank = |line: usize| !code[line];
    let mut widened: Vec<Region> = Vec::new();
    for (k, (old_range, new_range)) in found.iter().enumerate() {
        let (mut old_range, mut new_range) = (old_range.clone(), new_range.clone());
        if new_range.clone().all(blank) && !new.is_empty() {
            // The matched lines between this region and its neighbours.
            let floor = k.checked_sub(1).map_or(0, |previous| found[previous].1.end);
            let ceiling = found.get(k + 1).map_or(new.len(), |next| next.1.start);
            let before = (floor..new_range.start)
                .rev()
                .find(|line| !blank(*line))
                .unwrap_or(floor);
            let after = (new_range.end..ceiling)
                .find(|line| !blank(*line))
                .map_or(ceiling, |line| line + 1);
            old_range = old_range.start - (new_range.start - before)
                ..old_range.end + (after - new_range.end);
            new_range = before..after;
        }
        match widened.last_mut() {
            // Overlapping regions merge, and so do touching ones when the later is empty on a
            // side: an empty side's header names the line it follows (`-3,0`), which collides
            // with a hunk ending on that line, and cargo-mutants' diff parser rejects the pair
            // (council r17's sealed diff: "hunks not in order or overlap"). Widening can carry a
            // region right up to the next one; regions are separated by the same matched lines
            // on both sides, so they touch on both sides at once.
            Some(last)
                if last.0.end > old_range.start
                    || last.0.end == old_range.start
                        && (old_range.is_empty() || new_range.is_empty()) =>
            {
                last.0.end = last.0.end.max(old_range.end);
                last.1.end = last.1.end.max(new_range.end);
            }
            _ => widened.push((old_range, new_range)),
        }
    }
    widened
}

/// Whether each line holds code outside comments and attributes, tracking nested block comments,
/// attribute brackets and string literals across lines (council r12: the inside of a multi-line
/// `/* … */` or `#[cfg(…)]` is not code; council r13: a `]` or `/*` inside a string, as in
/// `#[doc = "]"]` or `"**/*.rs"`, changes neither). Plain, byte and raw (`r#"…"#`) strings are
/// followed; a character literal holding `"` is not, and opens a string that can only make later
/// lines non-code, which widens further.
fn code_lines(new: &[&[u8]]) -> Vec<bool> {
    let (mut comment, mut attribute) = (0_usize, 0_usize);
    // Inside a string: `None` for plain strings, `Some(n)` for a raw string closed by `"` and n `#`.
    let mut string: Option<Option<usize>> = None;
    new.iter()
        .map(|line| {
            let mut code = false;
            let mut skip = 0;
            for (i, &byte) in line.iter().enumerate() {
                if skip > 0 {
                    skip -= 1;
                    continue;
                }
                let pair = line.get(i..i + 2);
                // Counted only at a raw string's opening or closing quote, so a run of `#` costs
                // linear time, not quadratic (council r14).
                let hashes = || line[i + 1..].iter().take_while(|b| **b == b'#').count();
                if let Some(raw) = string {
                    match raw {
                        None if byte == b'\\' => skip = 1,
                        None if byte == b'"' => string = None,
                        Some(n) if byte == b'"' && hashes() >= n => {
                            string = None;
                            skip = n;
                        }
                        _ => {}
                    }
                } else if comment > 0 {
                    if pair == Some(b"/*") {
                        comment += 1;
                        skip = 1;
                    } else if pair == Some(b"*/") {
                        comment -= 1;
                        skip = 1;
                    }
                } else if pair == Some(b"//") {
                    break;
                } else if pair == Some(b"/*") {
                    comment = 1;
                    skip = 1;
                } else if let Some(length) = char_literal(&line[i..]) {
                    // A character literal, `'"'` included, is skipped whole (council r14: one
                    // holding a quote opened a phantom string that could under-include).
                    skip = length - 1;
                    code |= attribute == 0;
                } else if byte == b'"' {
                    string = Some(None);
                    code |= attribute == 0;
                } else if byte == b'r' && line.get(i + 1 + hashes()) == Some(&b'"') {
                    string = Some(Some(hashes()));
                    skip = hashes() + 1;
                    code |= attribute == 0;
                } else if attribute > 0 {
                    match byte {
                        b'[' => attribute += 1,
                        b']' => attribute -= 1,
                        _ => {}
                    }
                } else if pair == Some(b"#[") {
                    attribute = 1;
                    skip = 1;
                } else if !byte.is_ascii_whitespace() {
                    code = true;
                }
            }
            code
        })
        .collect()
}

/// The byte length of the character literal `rest` starts with (`'x'`, `'"'`, `'é'`, `'\''`,
/// `'\u{1F600}'`), or `None` for a lifetime, a label or anything else. An escape's closing quote
/// is looked for within the longest escape, so the search is bounded.
fn char_literal(rest: &[u8]) -> Option<usize> {
    if rest.first() != Some(&b'\'') {
        return None;
    }
    if rest.get(1) == Some(&b'\\') {
        return rest
            .iter()
            .enumerate()
            .skip(3)
            .take(10)
            .find(|(_, b)| **b == b'\'')
            .map(|(end, _)| end + 1);
    }
    let width = match rest.get(1)? {
        lead if *lead < 0x80 => 1,
        lead if *lead >= 0xF0 => 4,
        lead if *lead >= 0xE0 => 3,
        _ => 2,
    };
    (rest.get(1 + width) == Some(&b'\'')).then_some(width + 2)
}

/// Append `path`'s zero-context unified diff from `old` to `new`.
fn unified(path: &str, old: &[u8], new: &[u8], spent: &mut usize, out: &mut Vec<u8>) -> Result<()> {
    let (old, new) = (lines(old)?, lines(new)?);
    let mut ids = HashMap::new();
    let (old_ids, new_ids) = (intern(&mut ids, &old), intern(&mut ids, &new));
    out.extend_from_slice(format!("--- a/{path}\n+++ b/{path}\n").as_bytes());
    for (old_range, new_range) in widen_deletions(&regions(&old_ids, &new_ids, spent), &new) {
        hunk(
            (old_range.start, &old[old_range]),
            (new_range.start, &new[new_range]),
            out,
        );
    }
    Ok(())
}

/// One hunk replacing `old.1` (starting after `old.0` lines) with `new.1` (after `new.0` lines).
fn hunk(old: (usize, &[&[u8]]), new: (usize, &[&[u8]]), out: &mut Vec<u8>) {
    // An empty side names the line it follows; a non-empty side names its first line.
    let start = |(before, lines): (usize, &[&[u8]])| before + usize::from(!lines.is_empty());
    out.extend_from_slice(
        format!(
            "@@ -{},{} +{},{} @@\n",
            start(old),
            old.1.len(),
            start(new),
            new.1.len()
        )
        .as_bytes(),
    );
    for (sign, side) in [(b'-', old.1), (b'+', new.1)] {
        for line in side {
            out.push(sign);
            out.extend_from_slice(line);
            if !line.ends_with(b"\n") {
                out.extend_from_slice(b"\n\\ No newline at end of file\n");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// Apply a diff produced by [`unified`] to `old`, checking every hunk's old- and new-side
    /// coordinates as well as its context.
    fn apply(diff: &[u8], old: &[u8]) -> Vec<u8> {
        let source = lines(old).unwrap();
        let mut text = diff.split_inclusive(|b| *b == b'\n').peekable();
        assert!(text.next().unwrap().starts_with(b"--- a/"));
        assert!(text.next().unwrap().starts_with(b"+++ b/"));
        let (mut result, mut result_lines, mut copied) = (Vec::new(), 0, 0);
        while let Some(header) = text.next() {
            let header = std::str::from_utf8(header).unwrap();
            let spec: Vec<usize> = header
                .trim_start_matches("@@ -")
                .trim_end_matches(" @@\n")
                .split([',', ' ', '+'])
                .filter(|s| !s.is_empty())
                .map(|s| s.parse().unwrap())
                .collect();
            let (old_start, old_len, new_start, new_len) = (spec[0], spec[1], spec[2], spec[3]);
            let before = old_start - usize::from(old_len > 0);
            assert!(before >= copied, "hunks out of order: {header}");
            result.extend(source[copied..before].iter().flat_map(|l| l.iter()));
            result_lines += before - copied;
            // The new side names the line it follows (empty) or its own first line.
            assert_eq!(
                new_start,
                result_lines + usize::from(new_len > 0),
                "{header}"
            );
            let (mut removed, mut added) = (0, 0);
            while let Some(line) = text.next_if(|l| l[0] == b'-' || l[0] == b'+') {
                let mut body = line[1..].to_vec();
                if text.next_if(|l| l.starts_with(b"\\ No newline")).is_some() {
                    body.pop();
                }
                if line[0] == b'-' {
                    assert_eq!(body.as_slice(), source[before + removed], "{header}");
                    removed += 1;
                } else {
                    result.extend(body);
                    added += 1;
                }
            }
            assert_eq!((removed, added), (old_len, new_len), "{header}");
            result_lines += added;
            copied = before + old_len;
        }
        result.extend(source[copied..].iter().flat_map(|l| l.iter()));
        result
    }

    /// An independent LCS length, by rows and without backtracking.
    fn lcs_len(old: &[&[u8]], new: &[&[u8]]) -> usize {
        let mut prev = vec![0usize; new.len() + 1];
        for a in old {
            let mut row = vec![0usize; new.len() + 1];
            for (k, b) in new.iter().enumerate() {
                row[k + 1] = if a == b {
                    prev[k] + 1
                } else {
                    prev[k + 1].max(row[k])
                };
            }
            prev = row;
        }
        prev[new.len()]
    }

    fn diff_of(old: &[u8], new: &[u8], spent: &mut usize) -> Vec<u8> {
        let mut diff = Vec::new();
        unified("x.rs", old, new, spent, &mut diff).unwrap();
        diff
    }

    /// The new-side lines (0-based) that a diff's hunks mark as added.
    fn marked(diff: &[u8]) -> BTreeSet<usize> {
        let mut lines = BTreeSet::new();
        for header in diff
            .split_inclusive(|b| *b == b'\n')
            .filter(|l| l.starts_with(b"@@ "))
        {
            let header = std::str::from_utf8(header).unwrap();
            let new_side = header.split(' ').nth(2).unwrap().trim_start_matches('+');
            let (start, len) = new_side.split_once(',').unwrap();
            let (start, len): (usize, usize) = (start.parse().unwrap(), len.parse().unwrap());
            if len > 0 {
                lines.extend(start - 1..start - 1 + len);
            }
        }
        lines
    }

    /// Each hunk on each side starts after the last line the previous hunk names, as a unified-diff
    /// parser requires: an empty side's start is the line it follows (council r17: cargo-mutants
    /// rejected a pure insertion that started on the previous hunk's last line).
    fn assert_hunks_ordered(diff: &[u8]) {
        let mut last = [None::<usize>; 2];
        for header in diff
            .split_inclusive(|b| *b == b'\n')
            .filter(|l| l.starts_with(b"@@ "))
        {
            let header = std::str::from_utf8(header).unwrap();
            for (side, field) in header.split(' ').skip(1).take(2).enumerate() {
                let (start, len) = field[1..].split_once(',').unwrap();
                let (start, len): (usize, usize) = (start.parse().unwrap(), len.parse().unwrap());
                if let Some(end) = last[side] {
                    assert!(start > end, "{}", String::from_utf8_lossy(diff));
                }
                last[side] = Some(if len == 0 { start } else { start + len - 1 });
            }
        }
    }

    /// The diff is valid, its minimal regions are exactly the lines outside one longest common
    /// subsequence, and the output marks those plus one neighbouring line per pure deletion.
    fn check(old: &[u8], new: &[u8]) {
        let diff = diff_of(old, new, &mut 0);
        assert_eq!(apply(&diff, old), new, "{old:?} -> {new:?}");
        assert_hunks_ordered(&diff);
        let (o, n) = (lines(old).unwrap(), lines(new).unwrap());
        let mut ids = HashMap::new();
        let (oi, ni) = (intern(&mut ids, &o), intern(&mut ids, &n));
        let found = regions(&oi, &ni, &mut 0);
        let minimal: usize = found.iter().map(|(_, new)| new.len()).sum();
        assert_eq!(minimal, n.len() - lcs_len(&o, &n), "{old:?} -> {new:?}");
        let marked = marked(&diff);
        for (_, new_range) in &found {
            assert!(
                new_range.clone().all(|line| marked.contains(&line)),
                "{old:?} -> {new:?}"
            );
            // A pure deletion also marks the unchanged lines on both sides of it.
            if new_range.is_empty() {
                for neighbour in [new_range.start.checked_sub(1), Some(new_range.start)] {
                    if let Some(line) = neighbour.filter(|line| *line < n.len()) {
                        assert!(marked.contains(&line), "{old:?} -> {new:?}");
                    }
                }
            }
        }
    }

    fn hunks(diff: &[u8]) -> usize {
        diff.split_inclusive(|b| *b == b'\n')
            .filter(|l| l.starts_with(b"@@ "))
            .count()
    }

    /// Every file of up to `max` lines over a small alphabet, with and without a final newline.
    fn files(max: usize) -> Vec<Vec<u8>> {
        let mut all = vec![Vec::new()];
        let mut frontier: Vec<Vec<u8>> = vec![Vec::new()];
        for _ in 0..max {
            frontier = frontier
                .iter()
                .flat_map(|file| {
                    ["a\n", "b\n", "c\n"].iter().map(move |line| {
                        let mut next = file.clone();
                        next.extend_from_slice(line.as_bytes());
                        next
                    })
                })
                .collect();
            all.extend(frontier.iter().cloned());
        }
        let unterminated: Vec<Vec<u8>> = all
            .iter()
            .filter(|file| !file.is_empty())
            .map(|file| file[..file.len() - 1].to_vec())
            .collect();
        all.extend(unterminated);
        all
    }

    #[test]
    fn the_diff_reconstructs_the_new_file_and_marks_only_unmatched_lines() {
        let all = files(4);
        for old in &all {
            for new in &all {
                if old == new {
                    continue;
                }
                check(old, new);
            }
        }
    }

    /// A small deterministic xorshift generator, so the random test is reproducible.
    struct Rng(u64);
    impl Rng {
        fn below(&mut self, bound: u64) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0 % bound
        }
        fn file(&mut self, len: u64) -> Vec<u8> {
            let mut text = Vec::new();
            for _ in 0..len {
                let line =
                    [b"a\n", b"b\n", b"c\n", b"d\n"][usize::try_from(self.below(4)).unwrap()];
                text.extend_from_slice(line);
            }
            text
        }
    }

    // Longer inputs than the exhaustive test reaches: still exactly the unmatched lines are added.
    #[test]
    fn longer_random_files_get_minimal_diffs() {
        let mut rng = Rng(0x2545_f491_4f6c_dd1d);
        for _ in 0..3000 {
            let (old_len, new_len) = (rng.below(16), rng.below(16));
            let (old, new) = (rng.file(old_len), rng.file(new_len));
            check(&old, &new);
        }
    }

    #[test]
    fn hunk_headers_follow_unified_diff_conventions() {
        let diff = diff_of(b"a\nb\nc\n", b"a\nB\nc\nd\n", &mut 0);
        assert_eq!(
            String::from_utf8(diff).unwrap(),
            "--- a/x.rs\n+++ b/x.rs\n@@ -2,1 +2,1 @@\n-b\n+B\n@@ -3,0 +4,1 @@\n+d\n"
        );
        let mut diff = Vec::new();
        unified("new.rs", b"", b"fn f() {}", &mut 0, &mut diff).unwrap();
        assert_eq!(
            String::from_utf8(diff).unwrap(),
            "--- a/new.rs\n+++ b/new.rs\n@@ -0,0 +1,1 @@\n+fn f() {}\n\\ No newline at end of file\n"
        );
    }

    #[test]
    fn adjacent_unchanged_lines_produce_no_empty_hunk() {
        let diff = diff_of(b"a\nX\nb\nc\nY\nd\n", b"a\nX2\nb\nc\nY2\nd\n", &mut 0);
        assert_eq!(
            String::from_utf8(diff).unwrap(),
            "--- a/x.rs\n+++ b/x.rs\n@@ -2,1 +2,1 @@\n-X\n+X2\n@@ -5,1 +5,1 @@\n-Y\n+Y2\n"
        );
    }

    // cargo-mutants selects nothing for a pure deletion, so each one also marks a neighbour.
    #[test]
    fn pure_deletions_mark_a_neighbouring_line() {
        for (old, new, expected) in [
            // Mid-file: the lines before and after.
            (
                "a\nX\nb\n",
                "a\nb\n",
                "@@ -1,3 +1,2 @@\n-a\n-X\n-b\n+a\n+b\n",
            ),
            // The next non-blank line, not the first blank one after it.
            (
                "a\nX\nb\n\nc\n",
                "a\nb\n\nc\n",
                "@@ -1,3 +1,2 @@\n-a\n-X\n-b\n+a\n+b\n",
            ),
            // A rewritten gate is widened like a deletion (council r10).
            (
                "fn a() {}\n#[cfg(any())]\nfn f() {}\n",
                "fn a() {}\n#[cfg(all())]\nfn f() {}\n",
                "@@ -1,3 +1,3 @@\n-fn a() {}\n-#[cfg(any())]\n-fn f() {}\n+fn a() {}\n+#[cfg(all())]\n+fn f() {}\n",
            ),
            // Through comment and attribute lines too (council r10).
            (
                "fn a() {}\n#[cfg(any())]\n// note\nfn f() {}\n",
                "fn a() {}\n// note\nfn f() {}\n",
                "@@ -1,4 +1,3 @@\n-fn a() {}\n-#[cfg(any())]\n-// note\n-fn f() {}\n+fn a() {}\n+// note\n+fn f() {}\n",
            ),
            // Through blank lines to the nearest non-blank one on each side (council r9: deleting
            // a `cfg` that disabled the next item must mark that item, not just the blank line).
            (
                "fn a() {}\n\n#[cfg(any())]\nfn f() {}\n",
                "fn a() {}\n\nfn f() {}\n",
                "@@ -1,4 +1,3 @@\n-fn a() {}\n-\n-#[cfg(any())]\n-fn f() {}\n+fn a() {}\n+\n+fn f() {}\n",
            ),
            // At the start: the line after.
            ("X\na\n", "a\n", "@@ -1,2 +1,1 @@\n-X\n-a\n+a\n"),
            // Touching a replacement: two hunks, not merged.
            (
                "a\nX\nb\nY\n",
                "a\nX2\nb\n",
                "@@ -2,1 +2,1 @@\n-X\n+X2\n@@ -3,2 +3,1 @@\n-b\n-Y\n+b\n",
            ),
            // Two deletions sharing their neighbour: merged into one hunk.
            ("X\na\nY\n", "a\n", "@@ -1,3 +1,1 @@\n-X\n-a\n-Y\n+a\n"),
            // Nothing is left to mark.
            ("a\n", "", "@@ -1,1 +0,0 @@\n-a\n"),
        ] {
            let diff = diff_of(old.as_bytes(), new.as_bytes(), &mut 0);
            assert_eq!(
                String::from_utf8(diff).unwrap(),
                format!("--- a/x.rs\n+++ b/x.rs\n{expected}"),
                "{old:?} -> {new:?}"
            );
        }
    }

    /// A shared first line, then 4,096 distinct lines a side sharing one interior line: exactly the
    /// per-file cell budget once the common prefix is set aside.
    fn at_budget(tag: u8) -> (Vec<u8>, Vec<u8>) {
        let side = |name: &str| -> Vec<u8> {
            let mut text = b"head\n".to_vec();
            for i in 0..4096 {
                if i == 2048 {
                    text.extend_from_slice(b"shared\n");
                } else {
                    text.extend_from_slice(format!("{name}{tag}-{i}\n").as_bytes());
                }
            }
            text
        };
        (side("old"), side("new"))
    }

    // The exact LCS runs at the per-file budget and while all files together are within theirs;
    // after that a file's middle is reported whole (one hunk), over-including but never omitting.
    #[test]
    fn cell_budgets_bound_the_exact_lcs_per_file_and_in_total() {
        const { assert!(4096 * 4096 == LINE_CELL_BUDGET && 4 * LINE_CELL_BUDGET == TOTAL_CELL_BUDGET) };
        let mut spent = 0;
        for tag in 0..5 {
            let (old, new) = at_budget(tag);
            let diff = diff_of(&old, &new, &mut spent);
            assert_eq!(apply(&diff, &old), new);
            let expected = if tag < 4 { 2 } else { 1 };
            assert_eq!(hunks(&diff), expected, "file {tag}");
        }
        assert_eq!(spent, TOTAL_CELL_BUDGET);
    }

    #[test]
    fn a_middle_beyond_the_budget_is_reported_whole_never_omitted() {
        let side = |tag: &str| -> Vec<u8> {
            let mut text = Vec::new();
            for i in 0..4100 {
                text.extend_from_slice(format!("{tag}{i}\n").as_bytes());
            }
            text
        };
        let (old, new) = (side("old"), side("new"));
        const { assert!(4100 * 4100 > LINE_CELL_BUDGET) };
        let diff = diff_of(&old, &new, &mut 0);
        assert_eq!(apply(&diff, &old), new);
        assert!(diff.starts_with(b"--- a/x.rs\n+++ b/x.rs\n@@ -1,4100 +1,4100 @@\n"));
    }

    #[test]
    fn code_lines_skip_comments_and_attributes_across_lines() {
        let source: Vec<&[u8]> = [
            &b"#[cfg(any("[..],
            b"    test"[..].as_ref(),
            b"))]",
            b"/* outer /* nested */",
            b"   still a comment */",
            b"// line comment",
            b"   ",
            b"#[a[b]]",
            b"#[doc = \"]\"]",
            b"#[doc = r#\"a\"]\"#]",
            b"#[doc = \"multi",
            b"line ] string\"]",
            b"let g = \"**/*.rs\";",
            b"let q = \"\\\"\"; // escaped quote",
            b"\"only a string\"",
            b"r\"raw only\"",
            b"#[doc = r#\"#x\"#]",
            b"pub fn after() {}",
            b"const Q: char = '\"';",
            b"#[doc = \"]\"]",
            b"fn l<'a>(x: &'a str) -> char { '\\'' }",
            b"let e = '\\u{e9}';",
            b"#[doc = \"[x]\"] pub fn f() {}",
            b"pub fn g() {} /* trailing",
            b"*/ let x = 1;",
        ]
        .into_iter()
        .collect();
        assert_eq!(
            code_lines(&source),
            [
                false, false, false, false, false, false, false, false, false, false, false, false,
                true, true, true, true, false, true, true, false, true, true, true, true, true
            ]
        );
    }

    #[test]
    fn char_literals_and_lifetimes_are_told_apart() {
        for (rest, length) in [
            (&b"'x'"[..], Some(3)),
            (b"'\"' rest", Some(3)),
            ("'é'".as_bytes(), Some(4)),
            ("'😀'".as_bytes(), Some(6)),
            ("'中'".as_bytes(), Some(5)),
            (b"'\\''", Some(4)),
            (b"'\\u{1F600}'", Some(11)),
            (b"'a, b", None),
            (b"'static", None),
            (b"x'", None),
            (b"'\\x", None),
            // A stray continuation byte is not a one-byte character.
            (b"'\x80'", None),
        ] {
            assert_eq!(
                char_literal(rest),
                length,
                "{:?}",
                String::from_utf8_lossy(rest)
            );
        }
        // A run of `#` is scanned once per byte, not once per position (council r14), in code and
        // inside a raw string (council r15: the former test never reached that path). Eager
        // counting would take about 2^39 comparisons here and time out rather than pass.
        let run = vec![b'#'; 1 << 20];
        let mut raw = b"let s = r#\"".to_vec();
        raw.extend_from_slice(&run);
        raw.extend_from_slice(b"\"#;");
        assert_eq!(
            code_lines(&[&run[..], &raw[..], b"/* after */"]),
            [true, true, false]
        );
        // The skip ends exactly at the closing quote, so a comment right after it still opens; a
        // literal alone is code, and one inside an attribute is not.
        let source: [&[u8]; 5] = [
            b"let a = 'x'/*",
            b"inner",
            b"*/ let b = 1;",
            b"'x'",
            b"#[doc = 'x']",
        ];
        assert_eq!(code_lines(&source), [true, false, true, true, false]);
    }

    #[test]
    fn widened_regions_that_touch_the_next_one_become_one_hunk() {
        // The deleted `cfg` widens down to `b`, and the insertion after `b` then starts exactly
        // where the widened region ends. Two such hunks are rejected by cargo-mutants' parser
        // ("hunks not in order or overlap"; measured on council r17's sealed diff).
        let diff = diff_of(b"a\n#[cfg(x)]\nb\nc\n", b"a\nb\nNEW\nc\n", &mut 0);
        assert_eq!(hunks(&diff), 1, "{}", String::from_utf8_lossy(&diff));
        assert_hunks_ordered(&diff);
        assert_eq!(apply(&diff, b"a\n#[cfg(x)]\nb\nc\n"), b"a\nb\nNEW\nc\n");
    }

    #[test]
    fn deletion_widening_passes_multi_line_comments_and_attributes() {
        for (old, new) in [
            (
                "fn a() -> u32 { let _ = '\"'; 1 }\n#[cfg(any())]\n#[doc = \"]\"]\nfn f() {}\n",
                "fn a() -> u32 { let _ = '\"'; 1 }\n#[doc = \"]\"]\nfn f() {}\n",
            ),
            (
                "fn a() {}\n#[cfg(any())]\n#[doc = \"]\"]\nfn f() {}\n",
                "fn a() {}\n#[doc = \"]\"]\nfn f() {}\n",
            ),
            (
                "fn a() {}\n#[cfg(any())]\n/*\nexplanation\n*/\nfn f() {}\n",
                "fn a() {}\n/*\nexplanation\n*/\nfn f() {}\n",
            ),
            (
                "fn a() {}\n#[cfg(any())]\n#[cfg_attr(\n    test,\n    allow(dead_code)\n)]\nfn f() {}\n",
                "fn a() {}\n#[cfg_attr(\n    test,\n    allow(dead_code)\n)]\nfn f() {}\n",
            ),
        ] {
            let marked = marked(&diff_of(old.as_bytes(), new.as_bytes(), &mut 0));
            let last = new.lines().count() - 1;
            assert!(marked.contains(&last), "{new:?}: {marked:?}");
        }
    }

    #[test]
    fn the_line_cap_bounds_each_side() {
        assert_eq!(
            lines(&vec![b'\n'; LINE_CAP - 1]).unwrap().len(),
            LINE_CAP - 1
        );
        assert!(lines(&vec![b'\n'; LINE_CAP]).is_err());
        let mut diff = Vec::new();
        assert!(unified("x.rs", b"", &vec![b'\n'; LINE_CAP], &mut 0, &mut diff).is_err());
    }
}
