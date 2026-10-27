// DDF integration: R02, R03, R07. Take path/status/mode/blob identity from Git's NUL-delimited raw records, pair each record with its patch block and refuse on any count or identity mismatch; the patch bytes stay the authoritative artifact.
// Design/status: docs/RECOMMENDATION_INTEGRATION.md#patch-truth
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fdeep-diff-forge-patch--git_raw
//! Pure (no I/O) parser for one `git diff --raw -z --patch` byte stream.
//!
//! Approach adapted from tuicr's `src/vcs/git/raw.rs` (MIT license, pinned peer
//! source): consume the `--raw -z` section as the only source of path and
//! status identity, split the patch section on column-zero `diff --git `
//! markers, and refuse a record/block count mismatch instead of guessing. DDF
//! goes further than the peer in three ways: the original patch bytes are kept
//! verbatim (never a lossy display string), a type change is paired with the
//! two blocks Git emits for it, and every block is checked against its record
//! (exact header bytes, `new file`/`deleted file` modes, rename/copy labels and
//! `index` object IDs) so a re-ordered or forged pairing is a typed error.

use std::ops::Range;

/// One repository-relative path exactly as Git emitted it in a raw record.
///
/// The bytes are never decoded, normalised or quoted; [`GitPath::to_utf8`] and
/// [`GitPath::display_lossy`] are separate projections for consumers that need
/// text. On Unix [`GitPath::to_os_string`] is lossless.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GitPath(Vec<u8>);

impl GitPath {
    /// Wrap raw path bytes (as emitted by Git, without any quoting).
    #[must_use]
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// The exact path bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// The path as UTF-8 text, or `None` when the bytes are not UTF-8.
    #[must_use]
    pub fn to_utf8(&self) -> Option<&str> {
        std::str::from_utf8(&self.0).ok()
    }

    /// A lossy, display-only rendering. Never use it as an identity.
    #[must_use]
    pub fn display_lossy(&self) -> String {
        String::from_utf8_lossy(&self.0).into_owned()
    }

    /// The path as an `OsString`, byte-for-byte on Unix.
    #[cfg(unix)]
    #[must_use]
    pub fn to_os_string(&self) -> Option<std::ffi::OsString> {
        use std::os::unix::ffi::OsStringExt as _;
        Some(std::ffi::OsString::from_vec(self.0.clone()))
    }

    /// The path as an `OsString`; `None` when it is not UTF-8 (non-Unix hosts
    /// have no lossless byte-path conversion).
    #[cfg(not(unix))]
    #[must_use]
    pub fn to_os_string(&self) -> Option<std::ffi::OsString> {
        self.to_utf8().map(std::ffi::OsString::from)
    }
}

/// Change kind, taken from the raw record's status letter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitChangeKind {
    /// `A`: the path exists only on the new side.
    Added,
    /// `D`: the path exists only on the old side.
    Deleted,
    /// `M`: content and/or mode changed in place.
    Modified,
    /// `R<score>`: moved (possibly also modified).
    Renamed,
    /// `C<score>`: copied (possibly also modified).
    Copied,
    /// `T`: object type changed (e.g. regular file to symlink). Git emits a
    /// deletion block followed by a creation block for it.
    TypeChanged,
}

impl GitChangeKind {
    /// Stable snake-case label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Added => "added",
            Self::Deleted => "deleted",
            Self::Modified => "modified",
            Self::Renamed => "renamed",
            Self::Copied => "copied",
            Self::TypeChanged => "type_changed",
        }
    }

    fn block_count(self) -> usize {
        if self == Self::TypeChanged { 2 } else { 1 }
    }
}

/// Where a change came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitChangeOrigin {
    /// A path Git tracks (in `HEAD`, the index, or a compared commit).
    Tracked,
    /// An untracked, non-ignored working-tree file, captured as an addition.
    Untracked,
}

impl GitChangeOrigin {
    /// Stable snake-case label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Tracked => "tracked",
            Self::Untracked => "untracked",
        }
    }
}

/// Structured identity for one changed path, paired with its patch bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitChange {
    /// Status from the raw record.
    pub kind: GitChangeKind,
    /// Rename/copy similarity score (0–100), when Git reported one.
    pub score: Option<u8>,
    /// Old-side mode (`None` when the old side does not exist).
    pub old_mode: Option<u32>,
    /// New-side mode (`None` when the new side does not exist).
    pub new_mode: Option<u32>,
    /// Old-side blob ID (full hex). `None` when the old side does not exist.
    pub old_blob: Option<String>,
    /// New-side blob ID (full hex). Git reports a null ID in the raw record for
    /// an unhashed working-tree file; the ID is then taken from the block's
    /// `index` line, and stays `None` when the block has no `index` line (a
    /// mode-only working-tree change).
    pub new_blob: Option<String>,
    /// Old-side path bytes (`None` for an addition).
    pub old_path: Option<GitPath>,
    /// New-side path bytes (`None` for a deletion).
    pub new_path: Option<GitPath>,
    /// Tracked change or captured untracked file.
    pub origin: GitChangeOrigin,
    /// Whether the block carries a binary payload or a "Binary files" notice.
    pub binary: bool,
    /// Byte range of this change's block(s) within the authoritative patch.
    pub patch_range: Range<usize>,
    /// Number of `diff --git` blocks this change owns (2 for a type change).
    pub block_count: usize,
}

impl GitChange {
    /// The path a review should name: the new path, or the old path for a
    /// deletion.
    #[must_use]
    pub fn review_path(&self) -> Option<&GitPath> {
        self.new_path.as_ref().or(self.old_path.as_ref())
    }
}

/// Typed, non-panicking errors from structured Git acquisition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitAcquireError {
    /// The requested selection is unusable (bad range, option-like revision,
    /// untracked capture outside a working-tree selection).
    InvalidSelection {
        /// Human-readable reason.
        detail: String,
    },
    /// Git could not be started.
    Spawn {
        /// The Git subcommand that was being run.
        command: &'static str,
        /// The operating-system error.
        detail: String,
    },
    /// Git exited with an unexpected status.
    CommandFailed {
        /// The Git subcommand that was run.
        command: &'static str,
        /// Exit code, when the process exited normally.
        code: Option<i32>,
        /// Bounded standard-error text.
        stderr: String,
    },
    /// Git's output exceeded the byte budget and was not retained.
    BudgetExceeded {
        /// Configured limit in bytes.
        limit_bytes: usize,
    },
    /// The raw metadata section is not well-formed.
    MalformedRaw {
        /// Byte offset in the stream where the problem was found.
        offset: usize,
        /// Human-readable reason.
        detail: String,
    },
    /// A raw status DDF does not acquire (unmerged `U`, unknown `X`, …).
    UnsupportedStatus {
        /// 0-based record index.
        record: usize,
        /// The status letter.
        status: char,
    },
    /// The number of patch blocks does not match what the raw records require.
    CountMismatch {
        /// Raw records parsed.
        records: usize,
        /// Patch blocks those records require.
        expected_blocks: usize,
        /// Patch blocks present.
        blocks: usize,
    },
    /// A patch block does not describe the record it is paired with.
    IdentityMismatch {
        /// 0-based record index.
        record: usize,
        /// Which check failed.
        detail: String,
    },
    /// A reviewed path is not UTF-8 and cannot be named in the text review model.
    NonUtf8Path {
        /// 0-based record index.
        record: usize,
    },
    /// The authoritative patch failed DDF's own parser.
    Parse(crate::PatchParseError),
    /// The parser produced a different number of files than Git emitted blocks.
    ParsedFileCountMismatch {
        /// Patch blocks.
        blocks: usize,
        /// Parsed review files.
        files: usize,
    },
}

impl std::fmt::Display for GitAcquireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidSelection { detail } => write!(f, "invalid Git selection: {detail}"),
            Self::Spawn { command, detail } => write!(f, "could not run `{command}`: {detail}"),
            Self::CommandFailed {
                command,
                code,
                stderr,
            } => match code {
                Some(code) => write!(f, "`{command}` exited with status {code}: {stderr}"),
                None => write!(f, "`{command}` was terminated by a signal: {stderr}"),
            },
            Self::BudgetExceeded { limit_bytes } => {
                write!(f, "Git output exceeds the {limit_bytes}-byte budget")
            }
            Self::MalformedRaw { offset, detail } => {
                write!(
                    f,
                    "malformed `--raw -z` metadata at byte {offset}: {detail}"
                )
            }
            Self::UnsupportedStatus { record, status } => {
                write!(f, "raw record {record} has unsupported status `{status}`")
            }
            Self::CountMismatch {
                records,
                expected_blocks,
                blocks,
            } => write!(
                f,
                "{records} raw record(s) require {expected_blocks} patch block(s) but Git emitted {blocks}"
            ),
            Self::IdentityMismatch { record, detail } => {
                write!(
                    f,
                    "patch block for raw record {record} does not match it: {detail}"
                )
            }
            Self::NonUtf8Path { record } => write!(f, "path of raw record {record} is not UTF-8"),
            Self::Parse(err) => write!(f, "authoritative patch failed to parse: {err}"),
            Self::ParsedFileCountMismatch { blocks, files } => write!(
                f,
                "Git emitted {blocks} patch block(s) but the parser produced {files} file(s)"
            ),
        }
    }
}

impl std::error::Error for GitAcquireError {}

/// Raw records paired with the exact patch bytes they describe.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StructuredDiff {
    patch: Vec<u8>,
    changes: Vec<GitChange>,
}

impl StructuredDiff {
    /// Parse one `git diff --raw -z --patch` stream whose changes are all
    /// [`GitChangeOrigin::Tracked`].
    ///
    /// # Errors
    /// Any malformed record, unsupported status, count mismatch or identity
    /// mismatch; nothing is retained on error.
    pub fn parse(output: &[u8]) -> Result<Self, GitAcquireError> {
        let mut diff = Self::default();
        diff.append_stream(output, GitChangeOrigin::Tracked)?;
        Ok(diff)
    }

    /// Parse another stream and append it (patch bytes and changes) in order.
    /// On error `self` is left unchanged.
    ///
    /// # Errors
    /// As [`StructuredDiff::parse`].
    pub fn append_stream(
        &mut self,
        output: &[u8],
        origin: GitChangeOrigin,
    ) -> Result<&[GitChange], GitAcquireError> {
        let (records, patch_offset) = parse_raw_section(output)?;
        let patch = &output[patch_offset..];
        let blocks = split_blocks(patch);
        if blocks.first().is_some_and(|block| block.start != 0)
            || (blocks.is_empty() && !patch.is_empty())
        {
            return Err(GitAcquireError::MalformedRaw {
                offset: patch_offset,
                detail: "patch section does not start with a `diff --git` block".to_owned(),
            });
        }
        let expected_blocks = records.iter().map(|r| r.kind.block_count()).sum();
        if expected_blocks != blocks.len() {
            return Err(GitAcquireError::CountMismatch {
                records: records.len(),
                expected_blocks,
                blocks: blocks.len(),
            });
        }
        let base = self.patch.len();
        let first_new = self.changes.len();
        let mut changes = Vec::with_capacity(records.len());
        let mut next_block = 0;
        for (index, record) in records.into_iter().enumerate() {
            let owned = &blocks[next_block..next_block + record.kind.block_count()];
            next_block += owned.len();
            let range = owned[0].start..owned[owned.len() - 1].end;
            changes.push(pair(
                index,
                record,
                patch,
                owned,
                origin,
                base + range.start..base + range.end,
            )?);
        }
        self.patch.extend_from_slice(patch);
        self.changes.extend(changes);
        Ok(&self.changes[first_new..])
    }

    /// The authoritative patch bytes, exactly as Git emitted them.
    #[must_use]
    pub fn patch_bytes(&self) -> &[u8] {
        &self.patch
    }

    /// Structured changes, in Git's output order.
    #[must_use]
    pub fn changes(&self) -> &[GitChange] {
        &self.changes
    }

    /// The exact patch bytes owned by `change`.
    #[must_use]
    pub fn change_patch(&self, change: &GitChange) -> &[u8] {
        self.patch.get(change.patch_range.clone()).unwrap_or(&[])
    }

    /// Seal the authoritative bytes as a [`crate::CapturedPatch`] and check
    /// that DDF's parser sees one file per Git block. Non-UTF-8 content is
    /// decoded lossily per line and its file marked (`PatchTwin::lossy_lines`).
    ///
    /// # Errors
    /// [`GitAcquireError::Parse`] or [`GitAcquireError::ParsedFileCountMismatch`].
    pub fn capture(&self) -> Result<crate::CapturedPatch, GitAcquireError> {
        let captured = crate::CapturedPatch::capture_bytes(self.patch.clone())
            .map_err(GitAcquireError::Parse)?;
        let blocks: usize = self.changes.iter().map(|c| c.block_count).sum();
        if captured.files().len() != blocks {
            return Err(GitAcquireError::ParsedFileCountMismatch {
                blocks,
                files: captured.files().len(),
            });
        }
        Ok(captured)
    }

    /// [`review_files`](Self::review_files) sealed over `captured`'s exact bytes: the files and
    /// the seal come from the same capture (anchor C-05).
    ///
    /// # Errors
    /// As [`review_files`](Self::review_files).
    pub fn sealed_review(
        &self,
        captured: &crate::CapturedPatch,
        version: &str,
    ) -> Result<crate::SealedReview, GitAcquireError> {
        let files = self.review_files(captured)?;
        Ok(crate::SealedReview::from_capture(
            files,
            captured.original_bytes(),
            version,
        ))
    }

    /// A review-model copy of `captured`'s files whose paths come from the raw
    /// records, not from display headers. `captured` itself is not modified.
    ///
    /// # Errors
    /// [`GitAcquireError::ParsedFileCountMismatch`] when `captured` is not this
    /// diff's capture, or [`GitAcquireError::NonUtf8Path`].
    pub fn review_files(
        &self,
        captured: &crate::CapturedPatch,
    ) -> Result<Vec<deep_diff_forge_core::ReviewFile>, GitAcquireError> {
        let mut files = captured.files().to_vec();
        let blocks: usize = self.changes.iter().map(|c| c.block_count).sum();
        if files.len() != blocks {
            return Err(GitAcquireError::ParsedFileCountMismatch {
                blocks,
                files: files.len(),
            });
        }
        let mut slots = files.iter_mut();
        for (record, change) in self.changes.iter().enumerate() {
            let path = change
                .review_path()
                .and_then(GitPath::to_utf8)
                .ok_or(GitAcquireError::NonUtf8Path { record })?;
            for file in slots.by_ref().take(change.block_count) {
                path.clone_into(&mut file.path);
            }
        }
        Ok(files)
    }
}

/// One parsed `--raw -z` record.
struct RawRecord {
    kind: GitChangeKind,
    score: Option<u8>,
    old_mode: u32,
    new_mode: u32,
    old_oid: String,
    new_oid: String,
    old_path: Option<GitPath>,
    new_path: Option<GitPath>,
}

fn malformed(offset: usize, detail: &str) -> GitAcquireError {
    GitAcquireError::MalformedRaw {
        offset,
        detail: detail.to_owned(),
    }
}

/// Read one NUL-terminated field starting at `pos`; returns it and the
/// position after the terminator.
fn field(output: &[u8], pos: usize) -> Result<(&[u8], usize), GitAcquireError> {
    let rest = output.get(pos..).unwrap_or(&[]);
    let len = rest
        .iter()
        .position(|&b| b == 0)
        .ok_or_else(|| malformed(pos, "unterminated field"))?;
    Ok((&rest[..len], pos + len + 1))
}

fn path_field(output: &[u8], pos: usize) -> Result<(GitPath, usize), GitAcquireError> {
    let (bytes, next) = field(output, pos)?;
    if bytes.is_empty() {
        return Err(malformed(pos, "empty path"));
    }
    Ok((GitPath(bytes.to_vec()), next))
}

/// Parse the raw section; returns the records and the offset where the patch
/// section begins.
fn parse_raw_section(output: &[u8]) -> Result<(Vec<RawRecord>, usize), GitAcquireError> {
    let mut records = Vec::new();
    let mut pos = 0;
    loop {
        match output.get(pos) {
            // Empty output: no changes at all.
            None if records.is_empty() => return Ok((records, pos)),
            None => {
                return Err(malformed(
                    pos,
                    "missing NUL separator before the patch section",
                ));
            }
            // The extra NUL Git writes between the raw and patch sections.
            Some(0) => return Ok((records, pos + 1)),
            Some(b':') => {
                let (header, next) = field(output, pos)?;
                let index = records.len();
                let mut record = parse_header(header, pos, index)?;
                let (first, next) = path_field(output, next)?;
                pos = next;
                match record.kind {
                    GitChangeKind::Added => record.new_path = Some(first),
                    GitChangeKind::Deleted => record.old_path = Some(first),
                    GitChangeKind::Renamed | GitChangeKind::Copied => {
                        let (second, next) = path_field(output, pos)?;
                        pos = next;
                        record.old_path = Some(first);
                        record.new_path = Some(second);
                    }
                    GitChangeKind::Modified | GitChangeKind::TypeChanged => {
                        record.old_path = Some(first.clone());
                        record.new_path = Some(first);
                    }
                }
                records.push(record);
            }
            Some(_) => return Err(malformed(pos, "expected `:` or the section separator")),
        }
    }
}

fn parse_header(header: &[u8], offset: usize, index: usize) -> Result<RawRecord, GitAcquireError> {
    let text =
        std::str::from_utf8(header).map_err(|_| malformed(offset, "non-ASCII record header"))?;
    let mut tokens = text
        .strip_prefix(':')
        .ok_or_else(|| malformed(offset, "record header lacks `:`"))?
        .split(' ');
    let (Some(old_mode), Some(new_mode), Some(old_oid), Some(new_oid), Some(status), None) = (
        tokens.next(),
        tokens.next(),
        tokens.next(),
        tokens.next(),
        tokens.next(),
        tokens.next(),
    ) else {
        return Err(malformed(offset, "record header does not have five fields"));
    };
    let mode = |m: &str| {
        if m.len() == 6 && m.bytes().all(|b| (b'0'..=b'7').contains(&b)) {
            u32::from_str_radix(m, 8).map_err(|_| malformed(offset, "invalid mode"))
        } else {
            Err(malformed(offset, "invalid mode"))
        }
    };
    let oid = |o: &str| {
        let ok = matches!(o.len(), 40 | 64)
            && o.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        if ok {
            Ok(o.to_owned())
        } else {
            Err(malformed(
                offset,
                "object ID is not full-length lowercase hex",
            ))
        }
    };
    let (old_oid, new_oid) = (oid(old_oid)?, oid(new_oid)?);
    if old_oid.len() != new_oid.len() {
        return Err(malformed(offset, "object IDs have different lengths"));
    }
    let mut letters = status.chars();
    let letter = letters
        .next()
        .ok_or_else(|| malformed(offset, "empty status"))?;
    let digits = letters.as_str();
    let score = if digits.is_empty() {
        None
    } else {
        match digits.parse::<u8>() {
            Ok(score) if score <= 100 && digits.bytes().all(|b| b.is_ascii_digit()) => Some(score),
            _ => return Err(malformed(offset, "invalid similarity score")),
        }
    };
    let kind = match letter {
        'A' => GitChangeKind::Added,
        'D' => GitChangeKind::Deleted,
        'M' => GitChangeKind::Modified,
        'R' => GitChangeKind::Renamed,
        'C' => GitChangeKind::Copied,
        'T' => GitChangeKind::TypeChanged,
        other => {
            return Err(GitAcquireError::UnsupportedStatus {
                record: index,
                status: other,
            });
        }
    };
    let scored = matches!(kind, GitChangeKind::Renamed | GitChangeKind::Copied);
    if scored != score.is_some() && !(kind == GitChangeKind::Modified && score.is_some()) {
        return Err(malformed(
            offset,
            "similarity score present on the wrong status",
        ));
    }
    Ok(RawRecord {
        kind,
        score,
        old_mode: mode(old_mode)?,
        new_mode: mode(new_mode)?,
        old_oid,
        new_oid,
        old_path: None,
        new_path: None,
    })
}

/// Byte ranges of each column-zero `diff --git ` block. Hunk body lines always
/// start with ` `, `+`, `-` or `\`, Git C-quotes any path containing a newline,
/// and base85 binary payload lines contain no space, so file content cannot
/// forge this marker.
fn split_blocks(patch: &[u8]) -> Vec<Range<usize>> {
    const MARKER: &[u8] = b"diff --git ";
    let starts: Vec<usize> = (0..patch.len())
        .filter(|&i| patch[i..].starts_with(MARKER) && (i == 0 || patch[i - 1] == b'\n'))
        .collect();
    starts
        .iter()
        .enumerate()
        .map(|(n, &start)| start..starts.get(n + 1).copied().unwrap_or(patch.len()))
        .collect()
}

/// Git's C-style quoting of `prefix` + `path` with `core.quotePath=false`:
/// control bytes, DEL, `"` and `\` are escaped; bytes ≥ 0x80 stay verbatim.
pub(crate) fn git_quote(prefix: &[u8], path: &[u8]) -> Vec<u8> {
    let needs = |b: u8| b < 0x20 || b == 0x7f || b == b'"' || b == b'\\';
    let mut out = Vec::with_capacity(prefix.len() + path.len() + 2);
    if !prefix.iter().chain(path).any(|&b| needs(b)) {
        out.extend_from_slice(prefix);
        out.extend_from_slice(path);
        return out;
    }
    out.push(b'"');
    for &b in prefix.iter().chain(path) {
        match b {
            7 => out.extend_from_slice(b"\\a"),
            8 => out.extend_from_slice(b"\\b"),
            9 => out.extend_from_slice(b"\\t"),
            10 => out.extend_from_slice(b"\\n"),
            11 => out.extend_from_slice(b"\\v"),
            12 => out.extend_from_slice(b"\\f"),
            13 => out.extend_from_slice(b"\\r"),
            b'"' => out.extend_from_slice(b"\\\""),
            b'\\' => out.extend_from_slice(b"\\\\"),
            b if needs(b) => out.extend_from_slice(format!("\\{b:03o}").as_bytes()),
            b => out.push(b),
        }
    }
    out.push(b'"');
    out
}

/// What one block must say about its record.
#[derive(Clone, Copy, PartialEq, Eq)]
enum BlockRole {
    /// The single block of an A/D/M/R/C record.
    Whole,
    /// First block of a type change: the deletion half.
    TypeDeletion,
    /// Second block of a type change: the creation half.
    TypeCreation,
}

fn is_null_oid(oid: &str) -> bool {
    oid.bytes().all(|b| b == b'0')
}

fn pair(
    index: usize,
    record: RawRecord,
    patch: &[u8],
    blocks: &[Range<usize>],
    origin: GitChangeOrigin,
    patch_range: Range<usize>,
) -> Result<GitChange, GitAcquireError> {
    let mut old_blob = (!is_null_oid(&record.old_oid)).then(|| record.old_oid.clone());
    let mut new_blob = (!is_null_oid(&record.new_oid)).then(|| record.new_oid.clone());
    let mut binary = false;
    let roles: &[BlockRole] = if record.kind == GitChangeKind::TypeChanged {
        &[BlockRole::TypeDeletion, BlockRole::TypeCreation]
    } else {
        &[BlockRole::Whole]
    };
    for (role, block) in roles.iter().zip(blocks) {
        let facts = check_block(index, &record, *role, &patch[block.clone()])?;
        binary |= facts.binary;
        if let Some((old, new)) = facts.index {
            let check = |side: &str, raw: &str, from_index: &str, slot: &mut Option<String>| {
                if is_null_oid(from_index) {
                    return Ok(());
                }
                match slot {
                    Some(known) if known != from_index => Err(GitAcquireError::IdentityMismatch {
                        record: index,
                        detail: format!(
                            "{side} object ID {from_index} in `index` line disagrees with raw {raw}"
                        ),
                    }),
                    Some(_) => Ok(()),
                    None => {
                        *slot = Some(from_index.to_owned());
                        Ok(())
                    }
                }
            };
            if *role != BlockRole::TypeCreation {
                check("old", &record.old_oid, &old, &mut old_blob)?;
            }
            if *role != BlockRole::TypeDeletion {
                check("new", &record.new_oid, &new, &mut new_blob)?;
            }
        }
    }
    let exists_old = record.kind != GitChangeKind::Added;
    let exists_new = record.kind != GitChangeKind::Deleted;
    Ok(GitChange {
        kind: record.kind,
        score: record.score,
        old_mode: exists_old.then_some(record.old_mode),
        new_mode: exists_new.then_some(record.new_mode),
        old_blob: if exists_old { old_blob } else { None },
        new_blob: if exists_new { new_blob } else { None },
        old_path: record.old_path,
        new_path: record.new_path,
        origin,
        binary,
        patch_range,
        block_count: roles.len(),
    })
}

struct BlockFacts {
    binary: bool,
    index: Option<(String, String)>,
}

/// The exact `diff --git` line Git writes for `old`/`new` with fixed `a/`/`b/`
/// prefixes.
fn header_line(old: &[u8], new: &[u8]) -> Vec<u8> {
    let mut line = b"diff --git ".to_vec();
    line.extend(git_quote(b"a/", old));
    line.push(b' ');
    line.extend(git_quote(b"b/", new));
    line
}

/// Object IDs from a block's `index <old>..<new>[ <mode>]` line, if it has one.
fn index_ids(
    record: usize,
    oid_len: usize,
    extended: &[&[u8]],
) -> Result<Option<(String, String)>, GitAcquireError> {
    let mismatch = |detail: &str| GitAcquireError::IdentityMismatch {
        record,
        detail: detail.to_owned(),
    };
    let mut index = None;
    for line in extended {
        if let Some(rest) = line.strip_prefix(b"index ") {
            let text = std::str::from_utf8(rest).map_err(|_| mismatch("non-ASCII `index` line"))?;
            let ids = text.split(' ').next().unwrap_or_default();
            let (old, new) = ids
                .split_once("..")
                .ok_or_else(|| mismatch("malformed `index` line"))?;
            if old.len() != oid_len || new.len() != oid_len {
                return Err(mismatch("`index` line object IDs are abbreviated"));
            }
            index = Some((old.to_owned(), new.to_owned()));
        }
    }
    Ok(index)
}

fn check_block(
    record: usize,
    raw: &RawRecord,
    role: BlockRole,
    block: &[u8],
) -> Result<BlockFacts, GitAcquireError> {
    let mismatch = |detail: String| GitAcquireError::IdentityMismatch { record, detail };
    let old = raw
        .old_path
        .as_ref()
        .or(raw.new_path.as_ref())
        .map_or(&[][..], GitPath::as_bytes);
    let new = raw
        .new_path
        .as_ref()
        .or(raw.old_path.as_ref())
        .map_or(&[][..], GitPath::as_bytes);
    let mut lines = block.split(|&b| b == b'\n');
    if lines.next() != Some(&header_line(old, new)[..]) {
        return Err(mismatch(
            "`diff --git` header names different paths".to_owned(),
        ));
    }
    // Extended header lines run until the first line that is not one.
    let extended: Vec<&[u8]> = lines
        .take_while(|l| {
            !(l.is_empty()
                || l.starts_with(b"--- ")
                || l.starts_with(b"@@")
                || l.starts_with(b"Binary files ")
                || *l == b"GIT binary patch")
        })
        .collect();
    let has = |line: &[u8]| extended.contains(&line);
    let label = |key: &str, path: &[u8]| {
        let mut l = key.as_bytes().to_vec();
        l.extend(git_quote(b"", path));
        l
    };
    let mode_line = |key: &str, mode: u32| format!("{key}{mode:06o}").into_bytes();
    let (want_new, want_deleted) = match (raw.kind, role) {
        (GitChangeKind::Added, _) | (_, BlockRole::TypeCreation) => (true, false),
        (GitChangeKind::Deleted, _) | (_, BlockRole::TypeDeletion) => (false, true),
        _ => (false, false),
    };
    if want_new && !has(&mode_line("new file mode ", raw.new_mode)) {
        return Err(mismatch("missing matching `new file mode` line".to_owned()));
    }
    if want_deleted && !has(&mode_line("deleted file mode ", raw.old_mode)) {
        return Err(mismatch(
            "missing matching `deleted file mode` line".to_owned(),
        ));
    }
    if !want_new && extended.iter().any(|l| l.starts_with(b"new file mode ")) {
        return Err(mismatch("unexpected `new file mode` line".to_owned()));
    }
    if !want_deleted
        && extended
            .iter()
            .any(|l| l.starts_with(b"deleted file mode "))
    {
        return Err(mismatch("unexpected `deleted file mode` line".to_owned()));
    }
    let (from, to) = match raw.kind {
        GitChangeKind::Renamed => ("rename from ", "rename to "),
        GitChangeKind::Copied => ("copy from ", "copy to "),
        _ => ("", ""),
    };
    if !from.is_empty() && (!has(&label(from, old)) || !has(&label(to, new))) {
        return Err(mismatch(format!(
            "missing matching `{}`/`{}` lines",
            from.trim_end(),
            to.trim_end()
        )));
    }
    if from.is_empty()
        && extended
            .iter()
            .any(|l| l.starts_with(b"rename ") || l.starts_with(b"copy "))
    {
        return Err(mismatch("unexpected rename/copy lines".to_owned()));
    }
    if role == BlockRole::Whole
        && !want_new
        && !want_deleted
        && raw.old_mode != raw.new_mode
        && (!has(&mode_line("old mode ", raw.old_mode))
            || !has(&mode_line("new mode ", raw.new_mode)))
    {
        return Err(mismatch(
            "missing matching `old mode`/`new mode` lines".to_owned(),
        ));
    }
    let index = index_ids(record, raw.old_oid.len(), &extended)?;
    let binary = block
        .split(|&b| b == b'\n')
        .any(|l| l == b"GIT binary patch" || l.starts_with(b"Binary files "));
    Ok(BlockFacts { binary, index })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The messages reach users verbatim through the CLI (`error: structured Git acquisition
    /// failed: ...`), so each variant must say what went wrong.
    #[test]
    fn each_acquisition_error_names_its_cause() {
        let cases = [
            (
                GitAcquireError::InvalidSelection {
                    detail: "bad".into(),
                },
                "invalid Git selection: bad",
            ),
            (
                GitAcquireError::BudgetExceeded { limit_bytes: 8 },
                "Git output exceeds the 8-byte budget",
            ),
            (
                GitAcquireError::UnsupportedStatus {
                    record: 2,
                    status: 'X',
                },
                "raw record 2 has unsupported status `X`",
            ),
            (
                GitAcquireError::NonUtf8Path { record: 1 },
                "path of raw record 1 is not UTF-8",
            ),
            (
                GitAcquireError::ParsedFileCountMismatch {
                    blocks: 3,
                    files: 2,
                },
                "Git emitted 3 patch block(s) but the parser produced 2 file(s)",
            ),
        ];
        for (error, message) in cases {
            assert_eq!(error.to_string(), message);
        }
    }

    const Z: &str = "0000000000000000000000000000000000000000";
    const A1: &str = "1111111111111111111111111111111111111111";
    const B2: &str = "2222222222222222222222222222222222222222";

    fn stream(raw: &str, patch: &str) -> Vec<u8> {
        let mut out = raw.as_bytes().to_vec();
        out.push(0);
        out.extend_from_slice(patch.as_bytes());
        out
    }

    fn modified(path: &str) -> String {
        format!(":100644 100644 {A1} {B2} M\0{path}\0")
    }

    fn modified_block(path: &str) -> String {
        format!(
            "diff --git a/{path} b/{path}\nindex {A1}..{B2} 100644\n--- a/{path}\n+++ b/{path}\n@@ -1 +1 @@\n-a\n+b\n"
        )
    }

    #[test]
    fn empty_stream_has_no_changes() {
        assert_eq!(StructuredDiff::parse(b"").unwrap().changes(), []);
        assert_eq!(StructuredDiff::parse(b"\0").unwrap().changes(), []);
    }

    #[test]
    fn record_without_block_is_a_count_mismatch() {
        let err = StructuredDiff::parse(&stream(&modified("one"), "")).unwrap_err();
        assert_eq!(
            err,
            GitAcquireError::CountMismatch {
                records: 1,
                expected_blocks: 1,
                blocks: 0
            }
        );
    }

    #[test]
    fn extra_block_is_a_count_mismatch() {
        let patch = format!("{}{}", modified_block("one"), modified_block("two"));
        let err = StructuredDiff::parse(&stream(&modified("one"), &patch)).unwrap_err();
        assert!(matches!(
            err,
            GitAcquireError::CountMismatch { blocks: 2, .. }
        ));
    }

    #[test]
    fn swapped_blocks_are_an_identity_mismatch() {
        let raw = format!("{}{}", modified("one"), modified("two"));
        let patch = format!("{}{}", modified_block("two"), modified_block("one"));
        let err = StructuredDiff::parse(&stream(&raw, &patch)).unwrap_err();
        assert!(
            matches!(err, GitAcquireError::IdentityMismatch { record: 0, .. }),
            "{err}"
        );
    }

    #[test]
    fn index_line_disagreeing_with_raw_blob_is_refused() {
        let patch = modified_block("one").replace(B2, "3333333333333333333333333333333333333333");
        let err = StructuredDiff::parse(&stream(&modified("one"), &patch)).unwrap_err();
        assert!(
            matches!(err, GitAcquireError::IdentityMismatch { .. }),
            "{err}"
        );
    }

    #[test]
    fn added_record_without_new_file_mode_is_refused() {
        let raw = format!(":000000 100644 {Z} {B2} A\0one\0");
        let err = StructuredDiff::parse(&stream(&raw, &modified_block("one"))).unwrap_err();
        assert!(
            matches!(err, GitAcquireError::IdentityMismatch { .. }),
            "{err}"
        );
    }

    #[test]
    fn unmerged_status_is_refused() {
        let raw = format!(":000000 000000 {Z} {Z} U\0one\0");
        let err = StructuredDiff::parse(&stream(&raw, "")).unwrap_err();
        assert_eq!(
            err,
            GitAcquireError::UnsupportedStatus {
                record: 0,
                status: 'U'
            }
        );
    }

    #[test]
    fn abbreviated_raw_ids_are_malformed() {
        let raw = ":100644 100644 1111111 2222222 M\0one\0";
        let err = StructuredDiff::parse(&stream(raw, &modified_block("one"))).unwrap_err();
        assert!(matches!(err, GitAcquireError::MalformedRaw { .. }), "{err}");
    }

    #[test]
    fn paths_come_from_raw_bytes_not_from_headers() {
        // A path that display-header splitting gets wrong: "x b/y".
        let raw = ":100644 100755 1111111111111111111111111111111111111111 0000000000000000000000000000000000000000 M\0x b/y\0";
        let patch = "diff --git a/x b/y b/x b/y\nold mode 100644\nnew mode 100755\n";
        let diff = StructuredDiff::parse(&stream(raw, patch)).unwrap();
        let change = &diff.changes()[0];
        assert_eq!(change.new_path.as_ref().unwrap().as_bytes(), b"x b/y");
        assert_eq!(change.old_mode, Some(0o100_644));
        assert_eq!(change.new_mode, Some(0o100_755));
        assert_eq!(change.new_blob, None);
        let captured = diff.capture().unwrap();
        assert_eq!(diff.review_files(&captured).unwrap()[0].path, "x b/y");
    }

    #[test]
    fn non_utf8_path_bytes_are_preserved() {
        let mut raw = format!(":100644 100644 {A1} {B2} M\0").into_bytes();
        raw.extend_from_slice(b"bad-\xff\0\0diff --git a/bad-\xff b/bad-\xff\nindex ");
        raw.extend_from_slice(format!("{A1}..{B2} 100644\n").as_bytes());
        let diff = StructuredDiff::parse(&raw).unwrap();
        assert_eq!(
            diff.changes()[0].old_path.as_ref().unwrap().as_bytes(),
            b"bad-\xff"
        );
        // Non-UTF-8 content is captured (lossily, marked) with its exact bytes sealed; a
        // non-UTF-8 path is still refused, because a path is an identity.
        let captured = diff.capture().unwrap();
        assert!(!captured.is_exact());
        assert_eq!(captured.original_bytes(), diff.patch_bytes());
        assert_eq!(captured.files()[0].patch_twin.lossy_lines, 1);
        assert!(matches!(
            diff.review_files(&captured),
            Err(GitAcquireError::NonUtf8Path { .. })
        ));
    }

    #[test]
    fn git_quote_matches_quote_path_false() {
        assert_eq!(git_quote(b"a/", "café".as_bytes()), "a/café".as_bytes());
        assert_eq!(
            git_quote(b"a/", b"t\tn\nq\"\x7f"),
            b"\"a/t\\tn\\nq\\\"\\177\""
        );
        assert_eq!(git_quote(b"", b"sp ace"), b"sp ace");
    }

    #[test]
    fn appending_on_error_leaves_diff_unchanged() {
        let mut diff =
            StructuredDiff::parse(&stream(&modified("one"), &modified_block("one"))).unwrap();
        let before = diff.clone();
        assert!(
            diff.append_stream(&stream(&modified("two"), ""), GitChangeOrigin::Untracked)
                .is_err()
        );
        assert_eq!(diff, before);
    }
}
