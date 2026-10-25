// Review council: The diff-scoped mutation gate's scope, built from raw base blobs and the sealed workspace bytes.
// Design/status: docs/REVIEW_COUNCIL.md#gate-sandbox-r7
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fcouncil--crates--deep-diff-forge-review--src--mutation_scope
//! The lines a diff-scoped mutation gate mutates.
//!
//! `git diff <base>` of a working tree honours index flags (assume-unchanged, skip-worktree), the
//! candidate's own `.gitattributes` (`-diff`, `binary`) and skips untracked files, so a candidate
//! could shrink its own mutation scope (council r7). Git is never asked to compare anything here:
//! each base blob is read raw (`git cat-file blob`, no filters or attributes), each inventoried
//! file is read from the workspace the snapshot seals, and the zero-context unified diff is built
//! in-process.
use crate::storage::{self, Inventory};
use crate::{Result, ReviewFailure, require};
use std::collections::BTreeSet;
use std::path::Path;

/// Largest mutation diff captured at prepare and read back by the mutation gate. Written as a
/// shift, so the only arithmetic mutant (a zero cap) refuses every diff-scoped run.
pub(crate) const MUTATION_DIFF_CAP: u64 = 16 << 20;
/// Line-pair cells for one file's exact LCS. Above it the whole changed middle is reported, which
/// over-includes lines (more mutants) and never omits a changed one.
const LINE_CELL_BUDGET: usize = 16 << 20;
/// Per-file read budget, matching the inventory's 64 MiB source limit.
const FILE_CAP: u64 = 64 << 20;

/// With a `base`, the zero-context unified diff of every inventoried file against it; the base must
/// be an ancestor of `HEAD`. Every file is included, not only `.rs` ones, because `#[path]` can make
/// any file a module. Files absent at `base` (including untracked ones) diff against empty; deleted
/// files add no lines and are omitted.
pub(crate) fn capture(
    repository: &Path,
    inventory: &Inventory,
    base: Option<&str>,
) -> Result<Option<Vec<u8>>> {
    base.map(|base| capture_from(repository, inventory, base))
        .transpose()
}
fn capture_from(repository: &Path, inventory: &Inventory, base: &str) -> Result<Vec<u8>> {
    storage::git_bytes(
        repository,
        &["merge-base", "--is-ancestor", base, "HEAD"],
        0,
    )
    .map_err(|_| {
        ReviewFailure("mutation base must be a commit that is an ancestor of HEAD".into())
    })?;
    let listed = storage::git_bytes(
        repository,
        &["ls-tree", "-r", "-z", "--name-only", base],
        FILE_CAP,
    )?;
    let in_base: BTreeSet<&[u8]> = listed.split(|b| *b == 0).collect();
    let mut diff = Vec::new();
    for path in inventory.paths() {
        let Some(meta) = storage::untrusted_metadata(repository, Path::new(path))? else {
            continue;
        };
        require(
            meta.is_file(),
            "source inventory contains symlink or non-file",
        )?;
        let new = storage::read_bytes(&repository.join(path), FILE_CAP)?;
        let old = if in_base.contains(path.as_bytes()) {
            let object = format!("{base}:./{path}");
            storage::git_bytes(repository, &["cat-file", "blob", &object], FILE_CAP)?
        } else {
            Vec::new()
        };
        if old != new {
            unified(path, &old, &new, &mut diff);
            require(
                diff.len() as u64 <= MUTATION_DIFF_CAP,
                "mutation diff exceeds its budget",
            )?;
        }
    }
    Ok(diff)
}

/// Lines with their terminators, so a missing final newline is itself a difference.
fn lines(bytes: &[u8]) -> Vec<&[u8]> {
    bytes.split_inclusive(|b| *b == b'\n').collect()
}

/// Index pairs of one longest common subsequence of `old` and `new`, ascending.
fn lcs_pairs(old: &[&[u8]], new: &[&[u8]]) -> Vec<(usize, usize)> {
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

/// Append `path`'s zero-context unified diff from `old` to `new`.
fn unified(path: &str, old: &[u8], new: &[u8], out: &mut Vec<u8>) {
    let (old, new) = (lines(old), lines(new));
    let prefix = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    let suffix = old[prefix..]
        .iter()
        .rev()
        .zip(new[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let old_mid = &old[prefix..old.len() - suffix];
    let new_mid = &new[prefix..new.len() - suffix];
    let mut anchors = if old_mid.len().saturating_mul(new_mid.len()) <= LINE_CELL_BUDGET {
        lcs_pairs(old_mid, new_mid)
    } else {
        Vec::new()
    };
    // A sentinel anchor after both middles closes the last hunk.
    anchors.push((old_mid.len(), new_mid.len()));
    out.extend_from_slice(format!("--- a/{path}\n+++ b/{path}\n").as_bytes());
    let (mut i, mut j) = (0, 0);
    for (anchor_old, anchor_new) in anchors {
        if anchor_old > i || anchor_new > j {
            hunk(
                (prefix + i, &old_mid[i..anchor_old]),
                (prefix + j, &new_mid[j..anchor_new]),
                out,
            );
        }
        (i, j) = (anchor_old + 1, anchor_new + 1);
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

    /// Apply a diff produced by [`unified`] to `old`, checking every hunk's header and context.
    fn apply(diff: &[u8], old: &[u8]) -> Vec<u8> {
        let source = lines(old);
        let mut text = diff.split_inclusive(|b| *b == b'\n').peekable();
        assert!(text.next().unwrap().starts_with(b"--- a/"));
        assert!(text.next().unwrap().starts_with(b"+++ b/"));
        let (mut result, mut copied) = (Vec::new(), 0);
        while let Some(header) = text.next() {
            let header = std::str::from_utf8(header).unwrap();
            let spec: Vec<usize> = header
                .trim_start_matches("@@ -")
                .trim_end_matches(" @@\n")
                .split([',', ' ', '+'])
                .filter(|s| !s.is_empty())
                .map(|s| s.parse().unwrap())
                .collect();
            let (old_start, old_len) = (spec[0], spec[1]);
            let before = if old_len == 0 {
                old_start
            } else {
                old_start - 1
            };
            assert!(before >= copied, "hunks out of order: {header}");
            result.extend(source[copied..before].iter().flat_map(|l| l.iter()));
            let mut removed = 0;
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
                }
            }
            assert_eq!(removed, old_len, "{header}");
            copied = before + old_len;
        }
        result.extend(source[copied..].iter().flat_map(|l| l.iter()));
        result
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
                let mut diff = Vec::new();
                unified("x.rs", old, new, &mut diff);
                assert_eq!(apply(&diff, old), *new, "{old:?} -> {new:?}");
                // Minimal: exactly the new lines outside one longest common subsequence are added.
                let (o, n) = (lines(old), lines(new));
                let added = diff
                    .split_inclusive(|b| *b == b'\n')
                    .filter(|l| l.starts_with(b"+") && !l.starts_with(b"+++"))
                    .count();
                assert_eq!(
                    added,
                    n.len() - lcs_pairs(&o, &n).len(),
                    "{old:?} -> {new:?}"
                );
            }
        }
    }

    #[test]
    fn hunk_headers_follow_unified_diff_conventions() {
        let mut diff = Vec::new();
        unified("x.rs", b"a\nb\nc\n", b"a\nB\nc\nd\n", &mut diff);
        assert_eq!(
            String::from_utf8(diff).unwrap(),
            "--- a/x.rs\n+++ b/x.rs\n@@ -2,1 +2,1 @@\n-b\n+B\n@@ -3,0 +4,1 @@\n+d\n"
        );
        let mut diff = Vec::new();
        unified("new.rs", b"", b"fn f() {}", &mut diff);
        assert_eq!(
            String::from_utf8(diff).unwrap(),
            "--- a/new.rs\n+++ b/new.rs\n@@ -0,0 +1,1 @@\n+fn f() {}\n\\ No newline at end of file\n"
        );
    }

    #[test]
    fn adjacent_unchanged_lines_produce_no_empty_hunk() {
        let mut diff = Vec::new();
        unified(
            "x.rs",
            b"a\nX\nb\nc\nY\nd\n",
            b"a\nX2\nb\nc\nY2\nd\n",
            &mut diff,
        );
        assert_eq!(
            String::from_utf8(diff).unwrap(),
            "--- a/x.rs\n+++ b/x.rs\n@@ -2,1 +2,1 @@\n-X\n+X2\n@@ -5,1 +5,1 @@\n-Y\n+Y2\n"
        );
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
            let mut diff = Vec::new();
            unified("x.rs", &old, &new, &mut diff);
            assert_eq!(apply(&diff, &old), new);
            let (o, n) = (lines(&old), lines(&new));
            let added = diff
                .split_inclusive(|b| *b == b'\n')
                .filter(|l| l.starts_with(b"+") && !l.starts_with(b"+++"))
                .count();
            // An independent LCS length, by rows and without backtracking.
            let mut prev = vec![0usize; n.len() + 1];
            for a in &o {
                let mut row = vec![0usize; n.len() + 1];
                for (k, b) in n.iter().enumerate() {
                    row[k + 1] = if a == b {
                        prev[k] + 1
                    } else {
                        prev[k + 1].max(row[k])
                    };
                }
                prev = row;
            }
            assert_eq!(added, n.len() - prev[n.len()], "{old:?} -> {new:?}");
        }
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
        let mut diff = Vec::new();
        unified("x.rs", &old, &new, &mut diff);
        assert_eq!(apply(&diff, &old), new);
        assert!(diff.starts_with(b"--- a/x.rs\n+++ b/x.rs\n@@ -1,4100 +1,4100 @@\n"));
    }
}
