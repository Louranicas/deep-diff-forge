// Review council: The diff-scoped mutation gate's scope, built from raw base blobs and the sealed workspace bytes.
// Design/status: docs/REVIEW_COUNCIL.md#gate-sandbox-r7
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fcouncil--crates--deep-diff-forge-review--src--mutation_scope
//! The lines a diff-scoped mutation gate mutates, and the skip attributes it refuses.
//!
//! `git diff <base>` of a working tree honours index flags (assume-unchanged, skip-worktree), the
//! candidate's own `.gitattributes` (`-diff`, `binary`) and skips untracked files, so a candidate
//! could shrink its own mutation scope (council r7). Git is never asked to compare anything here:
//! each base blob is read raw (`git cat-file blob`, with replacements, grafts, lazy fetches and
//! transports disabled by [`storage::git_bytes`]), each file is read from the sealed snapshot after
//! its fingerprint is verified, and the zero-context unified diff is built in-process.
use crate::storage::{self, Inventory};
use crate::{Result, ReviewFailure, require};
use std::collections::{BTreeSet, HashMap};
use std::iter::Peekable;
use std::ops::Range;
use std::path::Path;
use std::str::Chars;

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

/// Refuse a `base` that is not a commit and an ancestor of `HEAD`, or whose objects do not hash to
/// their IDs, before a run directory exists. Git does not re-hash a loose object when reading it,
/// so without the `fsck` an author who can write `.git/objects` could serve forged base blobs under
/// genuine IDs (measured); with it, the operator's base ID is the root of trust for every base blob.
pub(crate) fn require_base(repository: &Path, base: &str) -> Result<()> {
    storage::git_bytes(
        repository,
        &["merge-base", "--is-ancestor", base, "HEAD"],
        0,
    )
    .map_err(|_| {
        ReviewFailure("mutation base must be a commit that is an ancestor of HEAD".into())
    })?;
    storage::git_bytes(
        repository,
        &[
            "fsck",
            "--no-dangling",
            "--no-reflogs",
            "--no-progress",
            base,
        ],
        64 << 10,
    )
    .map(drop)
    .map_err(|_| ReviewFailure("mutation base objects fail git fsck".into()))
}

/// With a `base` already checked by [`require_base`], the zero-context unified diff of every
/// inventoried file in `snapshot` against it. Every file is included, not only `.rs`
/// ones, because `#[path]` can make any file a module. Files absent at `base` (including untracked
/// ones) diff against empty. Deleted files add no lines, and files that are not UTF-8 on either
/// side can be neither compiled as Rust nor mutated, so both are omitted.
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
    let listed = storage::git_bytes(
        repository,
        &["ls-tree", "-r", "-z", "--name-only", base],
        FILE_CAP,
    )?;
    let in_base: BTreeSet<&[u8]> = listed.split(|b| *b == 0).collect();
    let (mut diff, mut spent) = (Vec::new(), 0);
    for path in inventory.paths() {
        let Some(meta) = storage::untrusted_metadata(snapshot, Path::new(path))? else {
            continue;
        };
        require(
            meta.is_file(),
            "source inventory contains symlink or non-file",
        )?;
        let new = storage::read_untrusted(snapshot, Path::new(path), FILE_CAP)?;
        let old = if in_base.contains(path.as_bytes()) {
            let object = format!("{base}:./{path}");
            storage::git_bytes(repository, &["cat-file", "blob", &object], FILE_CAP)?
        } else {
            Vec::new()
        };
        if old == new || std::str::from_utf8(&old).is_err() || std::str::from_utf8(&new).is_err() {
            continue;
        }
        // Headers are line-oriented: a newline in a name could forge a header or a hunk.
        require(
            !path.chars().any(char::is_control),
            "mutation scope path contains a control character",
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
/// none (measured with 27.1.0): removing a check would leave its function unmutated. Widen each
/// such region by the unchanged line before it, or after it at the start of the file, so the code
/// around a deletion is mutated; regions that then overlap are merged. Unchanged lines are equal on
/// both sides, so the diff stays valid.
fn widen_deletions(found: Vec<Region>, new_len: usize) -> Vec<Region> {
    let mut widened: Vec<Region> = Vec::new();
    for (old, new) in found {
        let (old, new) = if !new.is_empty() || new_len == 0 {
            (old, new)
        } else if new.start > 0 {
            (old.start - 1..old.end, new.start - 1..new.end)
        } else {
            (old.start..old.end + 1, new.start..new.end + 1)
        };
        match widened.last_mut() {
            Some(last) if last.0.end > old.start => {
                last.0.end = last.0.end.max(old.end);
                last.1.end = last.1.end.max(new.end);
            }
            _ => widened.push((old, new)),
        }
    }
    widened
}

/// Append `path`'s zero-context unified diff from `old` to `new`.
fn unified(path: &str, old: &[u8], new: &[u8], spent: &mut usize, out: &mut Vec<u8>) -> Result<()> {
    let (old, new) = (lines(old)?, lines(new)?);
    let mut ids = HashMap::new();
    let (old_ids, new_ids) = (intern(&mut ids, &old), intern(&mut ids, &new));
    out.extend_from_slice(format!("--- a/{path}\n+++ b/{path}\n").as_bytes());
    for (old_range, new_range) in widen_deletions(regions(&old_ids, &new_ids, spent), new.len()) {
        hunk(
            (old_range.start, &old[old_range]),
            (new_range.start, &new[new_range]),
            out,
        );
    }
    Ok(())
}

/// True when Rust `source` names cargo-mutants' skip attribute path in code, in any spelling the
/// compiler (and so cargo-mutants, through `syn`) treats as the same tokens: whitespace, comments
/// and raw-identifier prefixes around `mutants`, `::` and `skip` are dropped. Comments and string,
/// byte-string, raw-string and character literals are skipped, so a mention in one is not a match.
/// Fails closed: an identifier merely ending in `mutants` before `::skip` also matches.
pub(crate) fn has_skip_attribute(source: &str) -> bool {
    let mut code = String::new();
    let mut chars = source.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '/' if chars.next_if_eq(&'/').is_some() => {
                chars.by_ref().find(|c| *c == '\n');
            }
            '/' if chars.next_if_eq(&'*').is_some() => skip_block_comment(&mut chars),
            '"' => skip_string(&mut chars),
            'r' if raw_string_ahead(&chars) => skip_raw_string(&mut chars),
            // A raw identifier's `r#` prefix: `r#mutants` is the path segment `mutants`.
            'r' if chars.next_if_eq(&'#').is_some() => {}
            '\'' => skip_char_literal(&mut chars, &mut code),
            c if c.is_whitespace() => {}
            c => code.push(c),
        }
    }
    code.contains("mutants::skip")
}
/// After `/*`: through the matching `*/`, counting nested comments.
fn skip_block_comment(chars: &mut Peekable<Chars>) {
    let (mut depth, mut previous) = (1_usize, ' ');
    for c in chars.by_ref() {
        match (previous, c) {
            ('/', '*') => (depth, previous) = (depth + 1, ' '),
            ('*', '/') => {
                (depth, previous) = (depth - 1, ' ');
                if depth == 0 {
                    return;
                }
            }
            _ => previous = c,
        }
    }
}
/// After an opening `"`: through the closing one, honouring backslash escapes.
fn skip_string(chars: &mut Peekable<Chars>) {
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                chars.next();
            }
            '"' => return,
            _ => {}
        }
    }
}
/// Whether the `r` just read opens a raw string: `#`s, then a quote.
fn raw_string_ahead(chars: &Peekable<Chars>) -> bool {
    let mut ahead = chars.clone();
    while ahead.next_if_eq(&'#').is_some() {}
    ahead.peek() == Some(&'"')
}
/// After the `r` of a raw string: its `#`s and quote, then through a quote followed by as many `#`s.
fn skip_raw_string(chars: &mut Peekable<Chars>) {
    let mut hashes = 0_usize;
    while chars.next_if_eq(&'#').is_some() {
        hashes += 1;
    }
    chars.next();
    while let Some(c) = chars.next() {
        let mut ahead = chars.clone();
        if c == '"' && (0..hashes).all(|_| ahead.next() == Some('#')) {
            *chars = ahead;
            return;
        }
    }
}
/// After a `'`: skip a character literal, or keep a lifetime or label's quote as code.
fn skip_char_literal(chars: &mut Peekable<Chars>, code: &mut String) {
    let mut ahead = chars.clone();
    match (ahead.next(), ahead.next()) {
        (Some('\\'), _) => {
            chars.next();
            chars.next();
            chars.by_ref().find(|c| *c == '\'');
        }
        (Some(_), Some('\'')) => *chars = ahead,
        _ => code.push('\''),
    }
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

    /// The diff is valid, its minimal regions are exactly the lines outside one longest common
    /// subsequence, and the output marks those plus one neighbouring line per pure deletion.
    fn check(old: &[u8], new: &[u8]) {
        let diff = diff_of(old, new, &mut 0);
        assert_eq!(apply(&diff, old), new, "{old:?} -> {new:?}");
        let (o, n) = (lines(old).unwrap(), lines(new).unwrap());
        let mut ids = HashMap::new();
        let (oi, ni) = (intern(&mut ids, &o), intern(&mut ids, &n));
        let found = regions(&oi, &ni, &mut 0);
        let minimal: usize = found.iter().map(|(_, new)| new.len()).sum();
        assert_eq!(minimal, n.len() - lcs_len(&o, &n), "{old:?} -> {new:?}");
        let marked = marked(&diff);
        let mut deletions = 0;
        for (_, new_range) in &found {
            assert!(
                new_range.clone().all(|line| marked.contains(&line)),
                "{old:?} -> {new:?}"
            );
            if new_range.is_empty() && !n.is_empty() {
                deletions += 1;
                let neighbour = new_range.start.checked_sub(1).unwrap_or(new_range.start);
                assert!(marked.contains(&neighbour), "{old:?} -> {new:?}");
            }
        }
        assert!(marked.len() <= minimal + deletions, "{old:?} -> {new:?}");
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
            // Mid-file: the line before.
            ("a\nX\nb\n", "a\nb\n", "@@ -1,2 +1,1 @@\n-a\n-X\n+a\n"),
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

    /// 4,096 distinct lines a side sharing one interior line: exactly the per-file cell budget.
    fn at_budget(tag: u8) -> (Vec<u8>, Vec<u8>) {
        let side = |name: &str| -> Vec<u8> {
            let mut text = Vec::new();
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
    fn the_line_cap_bounds_each_side() {
        assert_eq!(
            lines(&vec![b'\n'; LINE_CAP - 1]).unwrap().len(),
            LINE_CAP - 1
        );
        assert!(lines(&vec![b'\n'; LINE_CAP]).is_err());
        let mut diff = Vec::new();
        assert!(unified("x.rs", b"", &vec![b'\n'; LINE_CAP], &mut 0, &mut diff).is_err());
    }

    #[test]
    fn skip_attributes_are_found_in_every_token_equivalent_spelling() {
        for source in [
            "#[mutants::skip]\nfn f() {}",
            "#[mutants :: skip] fn f() {}",
            "#[mutants\t::\n skip] fn f() {}",
            "#[mutants/**/::/* x */skip] fn f() {}",
            "#[mutants // c\n::skip] fn f() {}",
            "#[cfg_attr(test, mutants ::skip)] fn f() {}",
            "#[::mutants::skip] fn f() {}",
            "#[r#mutants::r#skip] fn f() {}",
            "// a comment\n#[mutants::skip] fn f() {}",
            "/* a comment */ #[mutants::skip] fn f() {}",
            "/**/#[mutants::skip] fn f() {}",
            "/* a /* nested */ comment */ #[mutants::skip] fn f() {}",
            "let q = '\"'; #[mutants::skip] fn f() {}",
            "let q = '\\''; #[mutants::skip] fn f() {}",
            "let q = '\\u{22}'; #[mutants::skip] fn f() {}",
            "let s = \"a \\\" b\"; #[mutants::skip] fn f() {}",
            "let s = r#\"a \"b\" c\"#; #[mutants::skip] fn f() {}",
            "fn f<'a>(x: &'a str) {} #[mutants::skip] fn g() {}",
            "let b = b'\"'; #[mutants::skip] fn f() {}",
        ] {
            assert!(has_skip_attribute(source), "{source}");
        }
    }

    #[test]
    fn mentions_in_comments_and_literals_are_not_skip_attributes() {
        for source in [
            "// #[mutants::skip]\nfn f() {}",
            "//! mutants::skip\nfn f() {}",
            "/* #[mutants::skip] */ fn f() {}",
            "/* a /* nested */ #[mutants::skip] */ fn f() {}",
            "let s = \"#[mutants::skip]\";",
            "let s = \"a \\\" #[mutants::skip]\";",
            "let s = r\"#[mutants::skip]\";",
            "let s = r#\"a \"quoted\" #[mutants::skip]\"#;",
            "let s = br##\"x\"# #[mutants::skip]\"##;",
            "let s = b\"#[mutants::skip]\";",
            "fn f<'a>(x: &'a str) -> &'a str { \"mutants::skip\" }",
            "let c = 'm'; let s = \"mutants::skip\";",
            "#[doc = \"mutants::skip\"] fn f() {}",
        ] {
            assert!(!has_skip_attribute(source), "{source}");
        }
        // This module's own source names the attribute only inside literals and comments.
        assert!(!has_skip_attribute(include_str!("mutation_scope.rs")));
        assert!(!has_skip_attribute(include_str!("execution.rs")));
    }
}
