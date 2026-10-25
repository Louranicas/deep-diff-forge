// Review council: The diff-scoped mutation gate's scope, built from raw base blobs and the sealed workspace bytes.
// Design/status: docs/REVIEW_COUNCIL.md#gate-sandbox-r7
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fcouncil--crates--deep-diff-forge-review--src--mutation_scope
//! The lines a diff-scoped mutation gate mutates, and the skip attributes it refuses.
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
        // Headers are line-oriented, a space can end a name for some diff parsers (Git appends a
        // tab after such names), and a leading quote reads as Git's path quoting: any of them
        // could forge or misattribute a header or a hunk.
        require(
            !path.starts_with('"') && !path.chars().any(|c| c.is_control() || c.is_whitespace()),
            "mutation scope path contains whitespace, a control character or a leading quote",
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
/// none (measured with 27.1.0): removing a check would leave its function unmutated. Widen each such
/// region over the unchanged lines on both sides, through blank, comment and attribute lines, to the
/// nearest other line before and after it (council r9: a neighbour that is only a blank line can
/// sit outside the affected item). Widening stops at the neighbouring regions, and widened regions that then
/// overlap are merged. Unchanged lines are equal on both sides, so the diff stays valid.
fn widen_deletions(found: &[Region], new: &[&[u8]]) -> Vec<Region> {
    // Lines that belong to no item's body: blank, comment and attribute lines (council r10: a
    // comment between a deleted `cfg` and the item it disabled stopped the widening).
    let blank = |line: usize| {
        let text = new[line].trim_ascii();
        text.is_empty()
            || [&b"//"[..], b"/*", b"*", b"#["]
                .iter()
                .any(|start| text.starts_with(start))
    };
    let mut widened: Vec<Region> = Vec::new();
    for (k, (old_range, new_range)) in found.iter().enumerate() {
        let (mut old_range, mut new_range) = (old_range.clone(), new_range.clone());
        if new_range.is_empty() && !new.is_empty() {
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
            Some(last) if last.0.end > old_range.start => {
                last.0.end = last.0.end.max(old_range.end);
                last.1.end = last.1.end.max(new_range.end);
            }
            _ => widened.push((old_range, new_range)),
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
    for (old_range, new_range) in widen_deletions(&regions(&old_ids, &new_ids, spent), &new) {
        hunk(
            (old_range.start, &old[old_range]),
            (new_range.start, &new[new_range]),
            out,
        );
    }
    Ok(())
}

/// Rust `source` with comments, literals and whitespace removed, plus the contents of any string
/// literal that directly follows `path =` (a `#[path]` module file, also under `cfg_attr`).
struct Lexed {
    code: String,
    module_paths: Vec<String>,
    /// A literal or comment was still open at the end: the lexer disagrees with rustc somewhere.
    desync: bool,
}

/// Lex just enough of Rust to tell code from comments and literals. Whitespace, comments (nested
/// too) and `r#` raw-identifier prefixes are dropped; string, byte-string, raw-string and character
/// literals are skipped. A first-line shebang is skipped as rustc does (`#!` not followed by `[`).
/// A raw string or raw identifier only starts at the start of a token, so `ar#"x"` (edition 2015
/// and 2018) is an identifier, `#` and a string.
fn lex(source: &str) -> Lexed {
    let mut lexed = Lexed {
        code: String::new(),
        module_paths: Vec::new(),
        desync: false,
    };
    // rustc strips a byte-order mark first, then a shebang: `#!` whose next token, after
    // whitespace and comments, is not `[` (council r10: a comment before `[`, or a BOM, changed it).
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);
    let body = match source.strip_prefix("#!") {
        Some(rest) if !inner_attribute_follows(rest) => {
            rest.find('\n').map_or("", |newline| &rest[newline..])
        }
        _ => source,
    };
    let ident = |c: char| c.is_alphanumeric() || c == '_';
    let (mut before_last, mut last) = (' ', ' ');
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        // An `r` starts a raw literal or identifier only at a token's start (or after `b`/`c`).
        let token_start = !ident(last) || (matches!(last, 'b' | 'c') && !ident(before_last));
        match c {
            '/' if chars.next_if_eq(&'/').is_some() => {
                chars.by_ref().find(|c| *c == '\n');
            }
            '/' if chars.next_if_eq(&'*').is_some() => {
                lexed.desync |= !skip_block_comment(&mut chars);
            }
            '"' => {
                let (text, closed) = skip_string(&mut chars);
                lexed.desync |= !closed;
                if lexed.code.ends_with("path=") {
                    lexed.module_paths.push(unescape(&text));
                }
            }
            'r' if token_start && raw_string_ahead(&chars) => {
                let (text, closed) = skip_raw_string(&mut chars);
                lexed.desync |= !closed;
                if lexed.code.ends_with("path=") {
                    lexed.module_paths.push(text);
                }
            }
            // A raw identifier's `r#` prefix: `r#mutants` is the path segment `mutants`.
            'r' if token_start && chars.next_if_eq(&'#').is_some() => {}
            '\'' => skip_char_literal(&mut chars, &mut lexed.code),
            c if rust_whitespace(c) => {}
            c => lexed.code.push(c),
        }
        (before_last, last) = (last, c);
    }
    lexed
}

/// Rust's whitespace (Unicode `Pattern_White_Space`), which adds the left-to-right and right-to-left
/// marks to what `char::is_whitespace` covers (council r10: `mutants\u{200E}::skip`).
fn rust_whitespace(c: char) -> bool {
    c.is_whitespace() || matches!(c, '\u{200e}' | '\u{200f}')
}

/// rustc's shebang rule: after `#!`, an inner attribute follows when the next token, after
/// whitespace and comments, is `[`.
fn inner_attribute_follows(rest: &str) -> bool {
    let mut chars = rest.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '/' if chars.next_if_eq(&'/').is_some() => {
                chars.by_ref().find(|c| *c == '\n');
            }
            '/' if chars.next_if_eq(&'*').is_some() => {
                skip_block_comment(&mut chars);
            }
            c if rust_whitespace(c) => {}
            c => return c == '[',
        }
    }
    false
}

/// A string literal's value from its escaped spelling: rustc resolves `#[path]` from the value, so
/// `"hid\x64en.txt"` names `hidden.txt` (council r10). Unknown escapes are kept as written.
fn unescape(text: &str) -> String {
    let mut value = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            value.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => value.push('\n'),
            Some('t') => value.push('\t'),
            Some('r') => value.push('\r'),
            Some('0') => value.push('\0'),
            Some('x') => {
                let digits: String = chars.by_ref().take(2).collect();
                value.extend(u8::from_str_radix(&digits, 16).ok().map(char::from));
            }
            Some('u') => {
                let digits: String = chars
                    .by_ref()
                    .skip_while(|c| *c == '{')
                    .take_while(|c| *c != '}')
                    .collect();
                value.extend(
                    u32::from_str_radix(&digits, 16)
                        .ok()
                        .and_then(char::from_u32),
                );
            }
            // A line continuation drops the newline and the next line's leading whitespace.
            Some('\n') => while chars.next_if(|c| rust_whitespace(*c)).is_some() {},
            Some(other) => value.push(other),
            None => value.push('\\'),
        }
    }
    value
}

/// The `path = "…"` values in a Cargo manifest (target roots such as `[lib] path`, in basic or
/// literal TOML strings), so the skip scan reaches a target root whatever its extension
/// (council r10). Dependency paths are directories and only over-include.
pub(crate) fn manifest_paths(manifest: &str) -> Vec<String> {
    let mut paths = Vec::new();
    for line in manifest.lines() {
        let mut rest = line;
        while let Some(at) = rest.find("path") {
            rest = &rest[at + 4..];
            let value = rest.trim_start().strip_prefix('=').map(str::trim_start);
            let Some(quote) = value.and_then(|value| value.chars().next()) else {
                continue;
            };
            if matches!(quote, '"' | '\'') {
                let body = &value.unwrap_or_default()[1..];
                if let Some(end) = body.find(quote) {
                    paths.push(if quote == '"' {
                        unescape(&body[..end])
                    } else {
                        body[..end].to_owned()
                    });
                }
            }
        }
    }
    paths
}

/// One pass over Rust `source`: whether it names cargo-mutants' skip attribute path in code, in any
/// spelling the compiler (and so cargo-mutants, through `syn`) treats as the same tokens, and the
/// file names its `#[path = "…"]` attributes point at. A mention in a comment or a literal is not a
/// match. Fails closed twice over: an identifier merely ending in `mutants` before `::skip`
/// matches, and so does any file the lexer could not follow to its end (a literal or comment still
/// open) that mentions both words anywhere, since a lexer that disagrees with rustc may have
/// swallowed a real attribute (council r9).
pub(crate) fn exemption_scan(source: &str) -> (bool, Vec<String>) {
    let lexed = lex(source);
    let skip = lexed.code.contains("mutants::skip")
        || (lexed.desync && source.contains("mutants") && source.contains("skip"));
    (skip, lexed.module_paths)
}

/// After `/*`: through the matching `*/`, counting nested comments; false if it never closes.
fn skip_block_comment(chars: &mut Peekable<Chars>) -> bool {
    let (mut depth, mut previous) = (1_usize, ' ');
    for c in chars.by_ref() {
        match (previous, c) {
            ('/', '*') => (depth, previous) = (depth + 1, ' '),
            ('*', '/') => {
                (depth, previous) = (depth - 1, ' ');
                if depth == 0 {
                    return true;
                }
            }
            _ => previous = c,
        }
    }
    false
}
/// After an opening `"`: through the closing one, honouring backslash escapes; returns the
/// contents with escapes kept as written, and whether the string closed.
fn skip_string(chars: &mut Peekable<Chars>) -> (String, bool) {
    let mut text = String::new();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                text.push(c);
                text.extend(chars.next());
            }
            '"' => return (text, true),
            _ => text.push(c),
        }
    }
    (text, false)
}
/// Whether the `r` just read opens a raw string: `#`s, then a quote.
fn raw_string_ahead(chars: &Peekable<Chars>) -> bool {
    let mut ahead = chars.clone();
    while ahead.next_if_eq(&'#').is_some() {}
    ahead.peek() == Some(&'"')
}
/// After the `r` of a raw string: its `#`s and quote, then through a quote followed by as many `#`s;
/// returns the contents, and whether the string closed.
fn skip_raw_string(chars: &mut Peekable<Chars>) -> (String, bool) {
    let mut hashes = 0_usize;
    while chars.next_if_eq(&'#').is_some() {
        hashes += 1;
    }
    chars.next();
    let mut text = String::new();
    while let Some(c) = chars.next() {
        let mut ahead = chars.clone();
        if c == '"' && (0..hashes).all(|_| ahead.next() == Some('#')) {
            *chars = ahead;
            return (text, true);
        }
        text.push(c);
    }
    (text, false)
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
    use std::collections::BTreeSet;

    fn has_skip_attribute(source: &str) -> bool {
        exemption_scan(source).0
    }

    fn module_paths(source: &str) -> Vec<String> {
        exemption_scan(source).1
    }

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
    fn path_attributes_name_their_module_files() {
        assert_eq!(
            module_paths(
                "#[path = \"impl.txt\"] mod a;\n#[cfg_attr(test, path = r\"t/b.inc\")] mod b;\nlet path = 1; let s = \"x\";"
            ),
            ["impl.txt", "t/b.inc"]
        );
        assert_eq!(
            module_paths("// #[path = \"c.txt\"]\nmod c;"),
            Vec::<String>::new()
        );
        // rustc resolves the literal's value, not its spelling (council r10).
        assert_eq!(
            module_paths(
                "#[path = \"hid\\x64en.txt\"] mod a;\n#[path = \"hidden\\u{2e}rs\"] mod b;"
            ),
            ["hidden.txt", "hidden.rs"]
        );
        assert_eq!(unescape("a\\n\\t\\\\\\\"\\0\\\n   b"), "a\n\t\\\"\0b");
    }

    #[test]
    fn manifest_paths_name_target_roots_in_either_string_style() {
        assert_eq!(
            manifest_paths(
                "[lib]\npath = \"src/lib.txt\"\n[[bin]]\nname = 'x'\npath='src/main.inc'\n[dependencies]\nfoo = { path = \"../foo\" }\n# path = \"in a comment\" is over-included\n"
            ),
            ["src/lib.txt", "src/main.inc", "../foo", "in a comment"]
        );
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
            // A division is not a comment.
            "let x = 4 / 2; #[mutants::skip] fn f() {}",
            // A shebang line is not code, even with an unmatched quote in it.
            "#!/bin/sh \"\n#[mutants::skip]\npub fn f() {}",
            "#!/usr/bin/env r#\"\n#[mutants::skip]\npub fn f() {}",
            // Edition 2015/2018: `ar#"x"` is an identifier, `#` and a string, not a raw string.
            "foo!(ar#\"x\"); #[mutants::skip] fn f() {}",
            // The same with every quote balanced, so a mis-lex cannot fall back on desync.
            "foo!(ar#\"x\"); #[mutants::skip] fn f() {} // \"#",
            "#!/bin/sh \"\n#[mutants::skip]\nfn f() {} // \"",
            // `#![` opens an inner attribute, not a shebang.
            "#![mutants::skip]\nfn f() {}",
            // rustc strips a byte-order mark before the shebang, and skips comments before `[`.
            "\u{feff}#!/bin/sh \"\n#[mutants::skip]\nfn f() {} // \"",
            "#!/*x*/[allow(dead_code)] #[mutants::skip] pub fn f() {}",
            // Rust whitespace includes the directional marks.
            "#[mutants\u{200e}::\u{200f}skip] fn f() {}",
            // A literal or comment left open swallows the rest: fail closed when both words appear.
            "let s = \"unterminated;\n#[mutants::skip] fn f() {}",
            "/* unterminated\n#[mutants::skip] fn f() {}",
            "let s = r#\"unterminated\n#[mutants::skip] fn f() {}",
            "let s = r\"x\"#; #[mutants :: skip] fn f() {}",
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
            // A quote inside a raw string only closes it when followed by its `#`s.
            "let s = r#\"a\"b #[mutants::skip]\"#;",
            // Escaped character literals, including multi-character escapes before a quote literal.
            "let q = '\\\"'; let s = \"mutants::skip\";",
            "let a = ['\\u{22}','\"']; let s = \"mutants::skip\";",
            // `#![` is an inner attribute, not a shebang: the string after it is still a literal.
            "#![doc = \"mutants::skip\"]\nfn f() {}",
            // Open at the end but never mentioning skip: no refusal.
            "let s = \"unterminated mutants",
            "/* unterminated skip",
        ] {
            assert!(!has_skip_attribute(source), "{source}");
        }
        // This module's own source names the attribute only inside literals and comments.
        assert!(!has_skip_attribute(include_str!("mutation_scope.rs")));
        assert!(!has_skip_attribute(include_str!("execution.rs")));
    }
}
