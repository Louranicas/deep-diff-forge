// DDF integration: R02, R03, R04, R05, R06, R07, R44, R46. Preserve source bytes and exact original input independently of normalized display; use Git as the application oracle.
// Design/status: docs/RECOMMENDATION_INTEGRATION.md#patch-truth
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fdeep-diff-forge-patch--parser
use crate::{DEFAULT_BYTE_BUDGET, DEFAULT_LINE_BUDGET, PatchParseError};
use deep_diff_forge_core::{
    DiffStrategy, FileStatus, HunkId, PatchHunk, PatchLine, PatchLineKind, PatchTwin,
    PlannerDecision, ReviewFile,
};

/// Extended-header line prefixes recorded as file metadata.
const META_PREFIXES: &[&str] = &[
    "old mode ",
    "new mode ",
    "new file mode ",
    "deleted file mode ",
    "index ",
    "similarity index ",
    "dissimilarity index ",
    "copy from ",
    "copy to ",
    "GIT binary patch",
    "Binary files ",
];

/// Options controlling patch parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParseOptions {
    /// Maximum accepted input size in bytes (trust-boundary guard).
    pub byte_budget: usize,
}

impl Default for ParseOptions {
    fn default() -> Self {
        Self {
            byte_budget: DEFAULT_BYTE_BUDGET,
        }
    }
}

/// Parse a unified or Git-format patch with default options.
///
/// Produces one [`ReviewFile`] per file section, each carrying an apply-able
/// [`PatchTwin`]. Semantic analysis is never attempted here: the patch layer is
/// upstream of every enrichment layer.
///
/// # Errors
///
/// Returns [`PatchParseError`] when the input exceeds the byte budget, contains
/// a malformed hunk header, or places a diff body line outside any hunk.
pub fn parse(input: &str) -> Result<Vec<ReviewFile>, PatchParseError> {
    parse_with(input, ParseOptions::default())
}

/// Parse a unified or Git-format patch with explicit options.
///
/// # Errors
///
/// Returns [`PatchParseError`] when the input exceeds the byte budget, contains
/// a malformed hunk header, or places a diff body line outside any hunk.
pub fn parse_with(input: &str, options: ParseOptions) -> Result<Vec<ReviewFile>, PatchParseError> {
    parse_lines(input, input.len(), &[], options)
}

/// Parse patch bytes that may not be UTF-8 (see [`crate::decode`]). Each file that holds a lossily
/// decoded line reports it in `PatchTwin::lossy_lines`; the byte budget applies to the original
/// bytes, not to the longer decoded text.
///
/// # Errors
///
/// As [`parse_with`].
pub fn parse_decoded(
    decoded: &crate::DecodedPatch<'_>,
    options: ParseOptions,
) -> Result<Vec<ReviewFile>, PatchParseError> {
    parse_lines(
        decoded.text(),
        decoded.byte_len(),
        decoded.lossy_lines(),
        options,
    )
}

/// `lossy` holds the 1-based numbers of lossily decoded lines, ascending.
fn parse_lines(
    input: &str,
    byte_len: usize,
    lossy: &[usize],
    options: ParseOptions,
) -> Result<Vec<ReviewFile>, PatchParseError> {
    if byte_len > options.byte_budget {
        return Err(PatchParseError::BudgetExceeded {
            limit_bytes: options.byte_budget,
            actual_bytes: byte_len,
        });
    }

    let mut parser = Parser::default();
    let mut lossy = lossy.iter().peekable();
    let unterminated_last =
        (!input.is_empty() && !input.ends_with('\n')).then(|| input.split_terminator('\n').count());
    for (index, line) in input.split_terminator('\n').enumerate() {
        if index >= DEFAULT_LINE_BUDGET {
            return Err(PatchParseError::LineBudgetExceeded {
                limit_lines: DEFAULT_LINE_BUDGET,
                actual_lines: index + 1,
            });
        }
        parser.feed(line, index + 1)?;
        // Git refuses a patch whose last hunk line has no line feed as corrupt; reading it as
        // complete would review a line Git will not apply (bug hunt 2026-10-28, H1-04).
        if unterminated_last == Some(index + 1) && parser.can_mark_eof {
            return Err(PatchParseError::UnterminatedLine {
                line_number: index + 1,
            });
        }
        if lossy.next_if_eq(&&(index + 1)).is_some() {
            // `can_mark_eof` is set exactly when this line was a hunk body line.
            if parser.can_mark_eof
                && let Some(body) = parser.last_body_line()
            {
                body.lossy = true;
            }
            if let Some(file) = parser.file.as_mut() {
                file.lossy_lines = file.lossy_lines.saturating_add(1);
            }
        }
    }
    parser.finish()
}

/// Working accumulator for one file section.
#[derive(Default)]
struct FileBuf {
    git_old: Option<String>,
    git_new: Option<String>,
    old_raw: Option<String>,
    new_raw: Option<String>,
    rename_from: Option<String>,
    rename_to: Option<String>,
    metadata: Vec<String>,
    binary_body: Vec<String>,
    /// After `GIT binary patch`: only lines of the body grammar are admitted (see
    /// [`is_binary_body_line`]) until the next `diff --git`; any other line is refused.
    in_binary_body: bool,
    hunks: Vec<PatchHunk>,
    lossy_lines: u32,
    /// A `---` arrived and its `+++` must be the very next line.
    awaiting_new_header: bool,
    /// The file's `---`/`+++` or first hunk arrived: extended headers are no longer admitted.
    past_headers: bool,
}

/// Working accumulator for one hunk.
struct HunkBuf {
    id: HunkId,
    section: Option<String>,
    old_start: u32,
    new_start: u32,
    old_line: u32,
    new_line: u32,
    rem_old: u32,
    rem_new: u32,
    lines: Vec<PatchLine>,
}

#[derive(Default)]
struct Parser {
    files: Vec<ReviewFile>,
    file: Option<FileBuf>,
    hunk: Option<HunkBuf>,
    next_hunk_id: u64,
    /// Last line number fed, so end-of-input truncation can be located.
    last_line: usize,
    can_mark_eof: bool,
}

impl Parser {
    /// The most recent hunk body line of the current file: in the open hunk, or in the last
    /// closed one.
    fn last_body_line(&mut self) -> Option<&mut PatchLine> {
        let file = self.file.as_mut()?;
        self.hunk
            .as_mut()
            .and_then(|h| h.lines.last_mut())
            .or_else(|| file.hunks.last_mut().and_then(|h| h.lines.last_mut()))
    }

    fn feed(&mut self, line: &str, line_number: usize) -> Result<(), PatchParseError> {
        self.last_line = line_number;
        let can_mark_eof = std::mem::take(&mut self.can_mark_eof);
        // The "\ No newline at end of file" marker applies to the preceding
        // line and is recorded as file metadata, whether or not a hunk is still
        // open (it can arrive after the hunk's line counts are exhausted).
        if line.starts_with('\\') {
            if !can_mark_eof || line != "\\ No newline at end of file" {
                return Err(PatchParseError::InvalidEofMarker { line_number });
            }
            if let Some(file) = self.file.as_mut() {
                file.metadata.push(line.to_string());
            }
            if let Some(previous) = self.last_body_line() {
                previous.no_newline = true;
            }
            return Ok(());
        }
        // While a hunk is open and not yet exhausted, body lines belong to it.
        if let Some(hunk) = self.hunk.as_mut() {
            if (hunk.rem_old > 0 || hunk.rem_new > 0)
                && let Some(consumed) = hunk.consume_body(line, line_number)
            {
                consumed?;
                self.can_mark_eof = true;
                if hunk.rem_old == 0 && hunk.rem_new == 0 {
                    // Counts satisfied: a clean close (cannot truncate).
                    self.close_hunk(line_number)?;
                }
                return Ok(());
            }
            // A non-body line arrived while the hunk still expects content:
            // closing here rejects the truncated hunk.
            self.close_hunk(line_number)?;
        }
        self.dispatch_non_body(line, line_number)
    }

    fn dispatch_non_body(&mut self, line: &str, line_number: usize) -> Result<(), PatchParseError> {
        if line.starts_with("diff --cc ") || line.starts_with("diff --combined ") {
            return Err(PatchParseError::UnsupportedFormat { line_number });
        }
        if let Some(rest) = line.strip_prefix("diff --git ") {
            return self.start_git_file(rest, line_number);
        }
        // After `GIT binary patch`, git's format allows only body lines until the next
        // `diff --git`; anything else is refused. Before, every later line went into the body, so
        // a text change after a binary section was hidden from the review yet applied by
        // `git apply` (and the per-line scan of all metadata was quadratic). Ending the body at
        // the first other line instead let a later metadata line reorder on rendering (fuzz).
        if let Some(file) = self.file.as_mut()
            && file.in_binary_body
        {
            if is_binary_body_line(line) {
                file.binary_body.push(line.to_string());
                return Ok(());
            }
            return Err(PatchParseError::UnsupportedFormat { line_number });
        }
        // A `---` is a header only with its `+++` right after it; git ignores either alone, so
        // taking one as a header changed the reviewed path or made a phantom file (bug hunt
        // 2026-10-28, H1-01).
        if self.file.as_ref().is_some_and(|f| f.awaiting_new_header) && !line.starts_with("+++ ") {
            return Err(PatchParseError::MisplacedHeader { line_number });
        }
        if let Some(rest) = line.strip_prefix("--- ") {
            return self.on_old_header(rest, line_number);
        }
        if let Some(rest) = line.strip_prefix("+++ ") {
            return self.on_new_header(rest, line_number);
        }
        if line.starts_with("@@") {
            return self.open_hunk(line, line_number);
        }
        if self.try_metadata(line, line_number)? {
            return Ok(());
        }
        // A stray body marker outside any hunk is malformed; quiet preamble
        // (commit messages, diffstats, blank lines) is ignored.
        if line.starts_with('+') || line.starts_with('-') {
            return Err(PatchParseError::BodyLineOutsideHunk {
                line_number,
                text: line.to_string(),
            });
        }
        Ok(())
    }

    fn start_git_file(&mut self, rest: &str, line_number: usize) -> Result<(), PatchParseError> {
        self.close_hunk(line_number)?;
        self.flush_file()?;
        let mut file = FileBuf::default();
        // `diff --git a/OLD b/NEW` — split on the rightmost ` b/` occurrence.
        // Using split_whitespace() breaks on paths that contain spaces; git's
        // own parsing uses the rightmost ` b/` as the heuristic boundary.  When
        // the filename itself contains ` b/`, the `---`/`+++` headers (set by
        // on_old_header / on_new_header) override these values and are used
        // for the final path, so an incorrect git-header split is harmless in
        // practice.  An absent ` b/` (unusual / malformed) leaves both None so
        // the caller falls back to the `---`/`+++` paths.
        if rest.starts_with('"') {
            let (old, remaining) = super::paths::decode_quoted_prefix(rest)
                .ok_or(PatchParseError::InvalidPath { line_number })?;
            let new = super::paths::decode_path(remaining.trim_start())
                .ok_or(PatchParseError::InvalidPath { line_number })?;
            file.git_old = Some(strip_ab(&old));
            file.git_new = Some(strip_ab(&new));
        } else if let Some(sep) = rest.rfind(" \"b/").or_else(|| rest.rfind(" b/")) {
            file.git_old = Some(strip_ab(
                &super::paths::decode_path(&rest[..sep])
                    .ok_or(PatchParseError::InvalidPath { line_number })?,
            ));
            file.git_new = Some(strip_ab(
                &super::paths::decode_path(&rest[sep + 1..])
                    .ok_or(PatchParseError::InvalidPath { line_number })?,
            ));
        }
        self.file = Some(file);
        Ok(())
    }

    fn on_old_header(&mut self, rest: &str, line_number: usize) -> Result<(), PatchParseError> {
        self.close_hunk(line_number)?;
        let needs_new = self
            .file
            .as_ref()
            .is_some_and(|f| f.new_raw.is_some() || !f.hunks.is_empty());
        if self.file.is_none() || needs_new {
            self.flush_file()?;
            self.file = Some(FileBuf::default());
        }
        if let Some(file) = self.file.as_mut() {
            if file.old_raw.is_some() {
                return Err(PatchParseError::MisplacedHeader { line_number });
            }
            file.old_raw = Some(
                super::paths::decode_path(&header_path(rest))
                    .ok_or(PatchParseError::InvalidPath { line_number })?,
            );
            file.awaiting_new_header = true;
            file.past_headers = true;
        }
        Ok(())
    }

    fn on_new_header(&mut self, rest: &str, line_number: usize) -> Result<(), PatchParseError> {
        // A `+++` belongs right after its `---`: alone, or a second one, git ignores it, and
        // taking it changed the reviewed path (bug hunt 2026-10-28, H1-01 and H1-04).
        if !self.file.as_ref().is_some_and(|f| f.awaiting_new_header) {
            return Err(PatchParseError::MisplacedHeader { line_number });
        }
        if let Some(file) = self.file.as_mut() {
            file.awaiting_new_header = false;
            file.new_raw = Some(
                super::paths::decode_path(&header_path(rest))
                    .ok_or(PatchParseError::InvalidPath { line_number })?,
            );
        }
        Ok(())
    }

    fn open_hunk(&mut self, line: &str, line_number: usize) -> Result<(), PatchParseError> {
        let header =
            parse_hunk_header(line).ok_or_else(|| PatchParseError::MalformedHunkHeader {
                line_number,
                text: line.to_string(),
            })?;
        if self.file.is_none() {
            self.file = Some(FileBuf::default());
        }
        // A hunk ends the file's headers, even when no `---`/`+++` came first (fuzz).
        if let Some(file) = self.file.as_mut() {
            file.past_headers = true;
        }
        let id = HunkId(self.next_hunk_id);
        self.next_hunk_id += 1;
        self.hunk = Some(HunkBuf {
            id,
            section: header.section,
            old_start: header.old_start,
            new_start: header.new_start,
            old_line: header.old_start,
            new_line: header.new_start,
            rem_old: header.old_count,
            rem_new: header.new_count,
            lines: Vec::new(),
        });
        Ok(())
    }

    fn try_metadata(&mut self, line: &str, line_number: usize) -> Result<bool, PatchParseError> {
        let Some(file) = self.file.as_mut() else {
            return Ok(false);
        };
        // Extended headers come before the file's `---`/`+++` and hunks; after them git ignores
        // the line, so taking it changed the reviewed status, path or mode (bug hunt
        // 2026-10-28, H1-01).
        let extended = META_PREFIXES.iter().any(|p| line.starts_with(p))
            || ["rename from ", "rename to ", "copy from ", "copy to "]
                .iter()
                .any(|p| line.starts_with(p));
        if extended && file.past_headers {
            return Err(PatchParseError::MisplacedHeader { line_number });
        }
        if let Some(path) = line.strip_prefix("rename from ") {
            file.rename_from = Some(
                super::paths::decode_path(path)
                    .ok_or(PatchParseError::InvalidPath { line_number })?,
            );
            file.metadata.push(line.to_string());
            return Ok(true);
        }
        if let Some(path) = line.strip_prefix("rename to ") {
            file.rename_to = Some(
                super::paths::decode_path(path)
                    .ok_or(PatchParseError::InvalidPath { line_number })?,
            );
            file.metadata.push(line.to_string());
            return Ok(true);
        }
        if let Some(path) = line
            .strip_prefix("copy from ")
            .or_else(|| line.strip_prefix("copy to "))
        {
            super::paths::decode_path(path).ok_or(PatchParseError::InvalidPath { line_number })?;
        }
        if META_PREFIXES.iter().any(|p| line.starts_with(p)) {
            file.in_binary_body |= line == "GIT binary patch";
            file.metadata.push(line.to_string());
            return Ok(true);
        }
        Ok(false)
    }

    /// Close the open hunk, if any. A hunk whose declared old/new counts are not
    /// yet satisfied is a truncated hunk and is rejected — this is what keeps a
    /// hunk's `@@ -a,b +c,d @@` contract load-bearing for patch truth.
    fn close_hunk(&mut self, line_number: usize) -> Result<(), PatchParseError> {
        if let Some(hunk) = self.hunk.take() {
            if hunk.rem_old != 0 || hunk.rem_new != 0 {
                return Err(PatchParseError::TruncatedHunk {
                    line_number,
                    remaining_old: hunk.rem_old,
                    remaining_new: hunk.rem_new,
                });
            }
            if let Some(file) = self.file.as_mut() {
                file.hunks.push(PatchHunk {
                    id: hunk.id,
                    old_start: Some(hunk.old_start),
                    new_start: Some(hunk.new_start),
                    section: hunk.section,
                    lines: hunk.lines,
                });
            }
        }
        Ok(())
    }

    fn flush_file(&mut self) -> Result<(), PatchParseError> {
        if let Some(file) = self.file.take() {
            if file.awaiting_new_header {
                return Err(PatchParseError::MisplacedHeader {
                    line_number: self.last_line,
                });
            }
            // `new file` reads from /dev/null and `deleted file` writes to it; git refuses the
            // contradiction ("expected /dev/null"), and rendering would have rewritten the header.
            let says = |prefix: &str| file.metadata.iter().any(|m| m.starts_with(prefix));
            let moved = ["rename from ", "rename to ", "copy from ", "copy to "]
                .iter()
                .any(|p| says(p));
            if (says("new file mode ") && says("deleted file mode "))
                || ((says("new file mode ") || says("deleted file mode ")) && moved)
                // A new file has no old lines and a deleted one no new lines; git refuses both
                // ("new file depends on old contents"), and the model could not render them.
                || (says("new file mode ")
                    && file.hunks.iter().flat_map(|h| &h.lines).any(|l| l.kind != PatchLineKind::Added))
                || (says("deleted file mode ")
                    && file.hunks.iter().flat_map(|h| &h.lines).any(|l| l.kind != PatchLineKind::Removed))
                // With hunks, the file headers are written from the status, so a new or deleted
                // file whose status is anything else (binary, renamed) cannot be represented.
                || (!file.hunks.is_empty()
                    && ((says("new file mode ") && file.resolve_status() != FileStatus::Added)
                        || (says("deleted file mode ")
                            && file.resolve_status() != FileStatus::Deleted)
                        || says("GIT binary patch")
                        || says("Binary files ")))
                || (says("new file mode ")
                    && file.old_raw.as_deref().is_some_and(|p| p != "/dev/null"))
                || (says("deleted file mode ")
                    && file.new_raw.as_deref().is_some_and(|p| p != "/dev/null"))
            {
                return Err(PatchParseError::InconsistentHeader {
                    line_number: self.last_line,
                });
            }
            if !file.hunks.is_empty()
                && file
                    .metadata
                    .iter()
                    .any(|m| m == "GIT binary patch" || m.starts_with("Binary files "))
            {
                return Err(PatchParseError::UnsupportedFormat {
                    line_number: self.last_line,
                });
            }
            // Contradictory rename/copy labels cannot be faithfully reconstructed.
            // Reject them rather than choosing one path and silently changing truth.
            let mut declared_old: Option<String> = None;
            for meta in &file.metadata {
                if let Some(from) = meta
                    .strip_prefix("rename from ")
                    .or_else(|| meta.strip_prefix("copy from "))
                {
                    let from =
                        super::paths::decode_path(from).ok_or(PatchParseError::InvalidPath {
                            line_number: self.last_line,
                        })?;
                    if declared_old
                        .as_ref()
                        .is_some_and(|previous| previous != &from)
                    {
                        return Err(PatchParseError::InvalidPath {
                            line_number: self.last_line,
                        });
                    }
                    declared_old = Some(from.clone());
                    if file
                        .old_raw
                        .as_deref()
                        .is_some_and(|old| strip_ab(old) != from)
                        || file.new_raw.as_deref() == Some("/dev/null")
                        || file.metadata.iter().any(|m| {
                            m.starts_with("new file mode ") || m.starts_with("deleted file mode ")
                        })
                    {
                        return Err(PatchParseError::InvalidPath {
                            line_number: self.last_line,
                        });
                    }
                }
            }
            self.files.push(file.into_review_file());
        }
        Ok(())
    }

    fn finish(mut self) -> Result<Vec<ReviewFile>, PatchParseError> {
        // End of input: any still-open hunk must have satisfied its counts.
        self.close_hunk(self.last_line)?;
        self.flush_file()?;
        Ok(self.files)
    }
}

impl HunkBuf {
    /// Try to consume `line` as a hunk body line. Returns `None` when the line
    /// is not a body line (so the caller closes the hunk and re-dispatches), and
    /// `Some(Err(..))` when the line would exceed the side count the header
    /// declared — a hunk that over-fills its `@@ -a,b +c,d @@` contract is
    /// rejected, just as a truncated one is.
    fn consume_body(
        &mut self,
        line: &str,
        line_number: usize,
    ) -> Option<Result<(), PatchParseError>> {
        let marker = line.chars().next();
        let text = line.get(1..).unwrap_or("").to_string();
        let mismatch = || PatchParseError::HunkLineCountMismatch {
            line_number,
            text: line.to_string(),
        };
        match marker {
            // A leading space is a context line; a truly-empty line inside an
            // unexhausted hunk is a blank context line emitted without the
            // conventional leading space (lenient-tool tolerance). Context
            // consumes one slot from BOTH sides, so both must remain.
            Some(' ') | None => {
                if self.rem_old == 0 || self.rem_new == 0 {
                    return Some(Err(mismatch()));
                }
                self.lines.push(PatchLine {
                    no_newline: false,
                    lossy: false,
                    kind: PatchLineKind::Context,
                    old_line: Some(self.old_line),
                    new_line: Some(self.new_line),
                    text,
                });
                self.old_line = self.old_line.saturating_add(1);
                self.new_line = self.new_line.saturating_add(1);
                self.rem_old -= 1;
                self.rem_new -= 1;
                Some(Ok(()))
            }
            Some('+') => {
                if self.rem_new == 0 {
                    return Some(Err(mismatch()));
                }
                self.lines.push(PatchLine {
                    no_newline: false,
                    lossy: false,
                    kind: PatchLineKind::Added,
                    old_line: None,
                    new_line: Some(self.new_line),
                    text,
                });
                self.new_line = self.new_line.saturating_add(1);
                self.rem_new -= 1;
                Some(Ok(()))
            }
            Some('-') => {
                if self.rem_old == 0 {
                    return Some(Err(mismatch()));
                }
                self.lines.push(PatchLine {
                    no_newline: false,
                    lossy: false,
                    kind: PatchLineKind::Removed,
                    old_line: Some(self.old_line),
                    new_line: None,
                    text,
                });
                self.old_line = self.old_line.saturating_add(1);
                self.rem_old -= 1;
                Some(Ok(()))
            }
            _ => None,
        }
    }
}

impl FileBuf {
    fn into_review_file(self) -> ReviewFile {
        let status = self.resolve_status();
        let path = self.resolve_path();
        let strategy = if status == FileStatus::BinaryChanged {
            DiffStrategy::Binary
        } else {
            DiffStrategy::Line
        };
        ReviewFile {
            path,
            status,
            patch_twin: PatchTwin {
                binary_body: self.binary_body,
                hunks: self.hunks,
                metadata: self.metadata,
                lossy_lines: self.lossy_lines,
            },
            semantic_twin: None,
            planner: PlannerDecision {
                strategy,
                fallback: None,
                notes: Vec::new(),
            },
        }
    }

    fn resolve_status(&self) -> FileStatus {
        let has = |prefix: &str| self.metadata.iter().any(|m| m.starts_with(prefix));
        let old_devnull = self.old_raw.as_deref() == Some("/dev/null");
        let new_devnull = self.new_raw.as_deref() == Some("/dev/null");
        let binary = has("Binary files ") || has("GIT binary patch");
        let new_file = has("new file mode ") || old_devnull;
        let deleted = has("deleted file mode ") || new_devnull;
        let mode_change = has("old mode ") || has("new mode ");

        if binary {
            FileStatus::BinaryChanged
        } else if self.rename_from.is_some() || self.rename_to.is_some() {
            FileStatus::Renamed
        } else if new_file {
            FileStatus::Added
        } else if deleted {
            FileStatus::Deleted
        } else if mode_change && self.hunks.is_empty() {
            FileStatus::TypeChanged
        } else if self.hunks.is_empty()
            && self.metadata.is_empty()
            && self.old_raw.is_none()
            && self.new_raw.is_none()
            && self.git_new.is_none()
        {
            FileStatus::Unknown
        } else {
            FileStatus::Modified
        }
    }

    fn resolve_path(&self) -> String {
        if let Some(to) = self.rename_to.as_ref() {
            return to.clone();
        }
        let new_side = self
            .new_raw
            .as_deref()
            .filter(|p| *p != "/dev/null")
            .map(strip_ab);
        let old_side = self
            .old_raw
            .as_deref()
            .filter(|p| *p != "/dev/null")
            .map(strip_ab);
        new_side
            .or(old_side)
            .or_else(|| self.git_new.clone())
            .or_else(|| self.git_old.clone())
            .unwrap_or_default()
    }
}

/// Parsed contents of a hunk header.
struct HunkHeader {
    old_start: u32,
    old_count: u32,
    new_start: u32,
    new_count: u32,
    section: Option<String>,
}

/// Parse `@@ -a,b +c,d @@ [optional section heading]`.
///
/// The opening `@@` is already required by the call site (line starts with `@@`).
/// This function additionally requires the mandatory closing `@@` token — a unified
/// diff hunk header without it is malformed and the patch MUST be rejected (patch
/// truth: malformed input must never silently normalise to a valid internal model).
///
/// Trailing section-heading text after the closing `@@` is allowed and ignored.
fn parse_hunk_header(line: &str) -> Option<HunkHeader> {
    let rest = line.strip_prefix("@@")?.trim_start();
    let rest = rest.strip_prefix('-')?;
    let mut parts = rest.splitn(2, ' ');
    let old = parts.next()?;
    let after = parts.next()?.trim_start();
    let new = after.strip_prefix('+')?;
    // Collect whitespace-split tokens: first is the `new[,count]` range,
    // second MUST be exactly `@@` (the mandatory closer).
    let mut tokens = new.split_whitespace();
    let new_range = tokens.next()?;
    match tokens.next() {
        Some("@@") => {}
        _ => return None,
    }
    // Any further text is the optional section heading, kept for display only.
    let (old_start, old_count) = parse_range(old)?;
    let (new_start, new_count) = parse_range(new_range)?;
    let heading = new.split_once("@@").map_or("", |(_, rest)| rest);
    let heading = heading.strip_prefix(' ').unwrap_or(heading);
    Some(HunkHeader {
        old_start,
        old_count,
        new_start,
        new_count,
        section: (!heading.is_empty()).then(|| heading.to_owned()),
    })
}

/// Parse a `start` or `start,count` range; count defaults to 1.
fn parse_range(range: &str) -> Option<(u32, u32)> {
    // Digits only: `u32::from_str` also takes a leading `+`, which git refuses (H1-04).
    let number = |text: &str| {
        (!text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()))
            .then(|| text.parse::<u32>().ok())
            .flatten()
    };
    let mut nums = range.split(',');
    let start = number(nums.next()?)?;
    let count = match nums.next() {
        Some(c) => number(c)?,
        None => 1,
    };
    if nums.next().is_some() {
        return None;
    }
    Some((start, count))
}

/// Strip a leading `a/` or `b/` Git diff prefix.
fn strip_ab(path: &str) -> String {
    path.strip_prefix("a/")
        .or_else(|| path.strip_prefix("b/"))
        .unwrap_or(path)
        .to_string()
}

/// Extract the path from a `---`/`+++` header value, dropping a trailing tab
/// timestamp that some `diff -u` variants append.
fn header_path(rest: &str) -> String {
    rest.split('\t').next().unwrap_or(rest).to_string()
}

/// Git's base85 alphabet (`base85.c`).
const BASE85: &[u8; 85] =
    b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz!#$%&()*+-;<=>?@^_`{|}~";

/// One line of a `GIT binary patch` body, as `git apply` reads it: a `literal N` or `delta N`
/// header, the empty line that ends a hunk, or a data line whose first letter encodes its byte
/// count (`A`-`Z` 1-26, `a`-`z` 27-52) followed by exactly that many bytes in base85.
fn is_binary_body_line(line: &str) -> bool {
    if line.is_empty() {
        return true;
    }
    if let Some(size) = line
        .strip_prefix("literal ")
        .or_else(|| line.strip_prefix("delta "))
    {
        return !size.is_empty() && size.bytes().all(|b| b.is_ascii_digit());
    }
    let bytes = line.as_bytes();
    let count = match bytes[0] {
        b @ b'A'..=b'Z' => usize::from(b - b'A') + 1,
        b @ b'a'..=b'z' => usize::from(b - b'a') + 27,
        _ => return false,
    };
    let data = &bytes[1..];
    data.len() == count.div_ceil(4) * 5 && data.iter().all(|b| BASE85.contains(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(input: &str) -> ReviewFile {
        let files = parse(input).expect("parse should succeed");
        assert_eq!(files.len(), 1, "expected exactly one file");
        files.into_iter().next().unwrap()
    }

    // --- patch-truth: hunks must satisfy their declared `@@ -a,b +c,d @@` counts.

    #[test]
    fn exact_hunk_counts_are_accepted() {
        // old = 1 ctx + 1 removed = 2; new = 1 ctx + 1 added = 2.
        let input = "--- a/x\n+++ b/x\n@@ -1,2 +1,2 @@\n ctx\n-old\n+new\n";
        let file = one(input);
        assert_eq!(file.patch_twin.hunks[0].lines.len(), 3);
    }

    #[test]
    fn truncated_hunk_at_eof_is_rejected() {
        // Header declares 5 old / 5 new but provides 2 lines, then EOF.
        let input = "--- a/x\n+++ b/x\n@@ -1,5 +1,5 @@\n ctx\n-old\n";
        let err = parse(input).unwrap_err();
        assert!(
            matches!(err, PatchParseError::TruncatedHunk { remaining_old, remaining_new, .. } if remaining_old == 3 && remaining_new == 4),
            "expected TruncatedHunk, got {err:?}"
        );
    }

    #[test]
    fn truncated_hunk_before_next_file_is_rejected() {
        // First hunk is truncated, then a second file header arrives.
        let input = "\
diff --git a/x b/x
--- a/x
+++ b/x
@@ -1,3 +1,3 @@
 ctx
diff --git a/y b/y
--- a/y
+++ b/y
@@ -1,1 +1,1 @@
-a
+b
";
        let err = parse(input).unwrap_err();
        assert!(
            matches!(err, PatchParseError::TruncatedHunk { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn added_line_when_new_count_is_zero_is_rejected() {
        // A pure-deletion hunk (`+0,0`) that supplies an addition over-fills the
        // new side while the old side is still expected.
        let input = "--- a/x\n+++ b/x\n@@ -1,1 +0,0 @@\n+oops\n";
        let err = parse(input).unwrap_err();
        assert!(
            matches!(err, PatchParseError::HunkLineCountMismatch { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn removed_line_when_old_count_is_zero_is_rejected() {
        // A pure-addition hunk (`-0,0`) that supplies a removal over-fills old.
        let input = "--- a/x\n+++ b/x\n@@ -0,0 +1,1 @@\n-oops\n";
        let err = parse(input).unwrap_err();
        assert!(
            matches!(err, PatchParseError::HunkLineCountMismatch { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn context_line_when_one_side_exhausted_is_rejected() {
        // old = 1, new = 2: the first context consumes the only old slot; a
        // second context has no old slot left.
        let input = "--- a/x\n+++ b/x\n@@ -1,1 +1,2 @@\n ctx\n ctx2\n";
        let err = parse(input).unwrap_err();
        assert!(
            matches!(err, PatchParseError::HunkLineCountMismatch { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn trailing_body_line_after_a_satisfied_hunk_is_rejected() {
        // The hunk's counts are exactly met by `-old`/`+new`; the extra `+x`
        // then falls outside any hunk and is rejected (BodyLineOutsideHunk).
        let input = "--- a/x\n+++ b/x\n@@ -1,1 +1,1 @@\n-old\n+new\n+x\n";
        let err = parse(input).unwrap_err();
        assert!(
            matches!(err, PatchParseError::BodyLineOutsideHunk { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn truncated_hunk_error_displays_remaining() {
        let input = "--- a/x\n+++ b/x\n@@ -1,4 +1,4 @@\n ctx\n";
        let msg = parse(input).unwrap_err().to_string();
        assert!(msg.contains("truncated hunk"));
    }

    const BASIC: &str = "\
diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,3 +1,3 @@
 fn main() {
-    let x = 1;
+    let x = 2;
 }
";

    #[test]
    fn parses_single_modified_file() {
        let file = one(BASIC);
        assert_eq!(file.path, "src/lib.rs");
        assert_eq!(file.status, FileStatus::Modified);
    }

    #[test]
    fn modified_file_has_one_hunk() {
        let file = one(BASIC);
        assert_eq!(file.patch_twin.hunks.len(), 1);
    }

    #[test]
    fn hunk_records_start_lines() {
        let file = one(BASIC);
        let hunk = &file.patch_twin.hunks[0];
        assert_eq!(hunk.old_start, Some(1));
        assert_eq!(hunk.new_start, Some(1));
    }

    #[test]
    fn hunk_line_kinds_are_in_order() {
        let file = one(BASIC);
        let kinds: Vec<_> = file.patch_twin.hunks[0]
            .lines
            .iter()
            .map(|l| l.kind)
            .collect();
        assert_eq!(
            kinds,
            vec![
                PatchLineKind::Context,
                PatchLineKind::Removed,
                PatchLineKind::Added,
                PatchLineKind::Context,
            ]
        );
    }

    #[test]
    fn added_line_has_only_new_line_number() {
        let file = one(BASIC);
        let added = file.patch_twin.hunks[0]
            .lines
            .iter()
            .find(|l| l.kind == PatchLineKind::Added)
            .unwrap();
        assert_eq!(added.old_line, None);
        assert_eq!(added.new_line, Some(2));
    }

    #[test]
    fn removed_line_has_only_old_line_number() {
        let file = one(BASIC);
        let removed = file.patch_twin.hunks[0]
            .lines
            .iter()
            .find(|l| l.kind == PatchLineKind::Removed)
            .unwrap();
        assert_eq!(removed.old_line, Some(2));
        assert_eq!(removed.new_line, None);
    }

    #[test]
    fn context_line_advances_both_sides() {
        let file = one(BASIC);
        let first = &file.patch_twin.hunks[0].lines[0];
        assert_eq!(first.kind, PatchLineKind::Context);
        assert_eq!(first.old_line, Some(1));
        assert_eq!(first.new_line, Some(1));
    }

    #[test]
    fn line_text_strips_leading_marker() {
        let file = one(BASIC);
        let added = file.patch_twin.hunks[0]
            .lines
            .iter()
            .find(|l| l.kind == PatchLineKind::Added)
            .unwrap();
        assert_eq!(added.text, "    let x = 2;");
    }

    #[test]
    fn captures_index_metadata() {
        let file = one(BASIC);
        assert!(
            file.patch_twin
                .metadata
                .iter()
                .any(|m| m.starts_with("index "))
        );
    }

    #[test]
    fn empty_input_yields_no_files() {
        assert_eq!(parse("").unwrap().len(), 0);
    }

    #[test]
    fn whitespace_only_input_yields_no_files() {
        assert_eq!(parse("\n\n   \n").unwrap().len(), 0);
    }

    #[test]
    fn new_file_is_added() {
        let input = "\
diff --git a/new.txt b/new.txt
new file mode 100644
index 0000000..abcdef0
--- /dev/null
+++ b/new.txt
@@ -0,0 +1,2 @@
+hello
+world
";
        let file = one(input);
        assert_eq!(file.status, FileStatus::Added);
        assert_eq!(file.path, "new.txt");
    }

    #[test]
    fn new_file_records_added_lines() {
        let input = "\
diff --git a/new.txt b/new.txt
new file mode 100644
--- /dev/null
+++ b/new.txt
@@ -0,0 +1,2 @@
+hello
+world
";
        let file = one(input);
        let adds = file.patch_twin.hunks[0]
            .lines
            .iter()
            .filter(|l| l.kind == PatchLineKind::Added)
            .count();
        assert_eq!(adds, 2);
    }

    #[test]
    fn deleted_file_is_deleted() {
        let input = "\
diff --git a/old.txt b/old.txt
deleted file mode 100644
--- a/old.txt
+++ /dev/null
@@ -1,2 +0,0 @@
-hello
-world
";
        let file = one(input);
        assert_eq!(file.status, FileStatus::Deleted);
        assert_eq!(file.path, "old.txt");
    }

    #[test]
    fn rename_is_renamed_and_uses_new_path() {
        let input = "\
diff --git a/old/name.rs b/new/name.rs
similarity index 100%
rename from old/name.rs
rename to new/name.rs
";
        let file = one(input);
        assert_eq!(file.status, FileStatus::Renamed);
        assert_eq!(file.path, "new/name.rs");
    }

    /// A text change after a binary body is never folded into it: before, every later line went
    /// into the body, so DDF reviewed only the binary file while `git apply` also changed
    /// `evil.rs`. That patch is now refused at the first line outside the body grammar.
    #[test]
    fn a_text_change_after_a_binary_body_is_never_hidden_in_it() {
        let input = "\
diff --git a/img.bin b/img.bin
index 88768efdf77ec78c9a995f94881793be6a41752b..f68ed8037341be54a4bad1485a8deb1be7188467 100644
GIT binary patch
literal 6
NcmZQzO3KVL0ssTy0d4>Q

literal 5
McmZQzOv=my00M6TI{*Lx

--- a/evil.rs
+++ b/evil.rs
@@ -1 +1 @@
-x
+y
";
        assert!(matches!(
            parse(input),
            Err(PatchParseError::UnsupportedFormat { line_number: 10 })
        ));
        // Legitimate nearby case: the same binary section followed by a `diff --git` text file.
        let split = input.replace("--- a/evil.rs", "diff --git a/ok.rs b/ok.rs\n--- a/ok.rs");
        let split = split.replace("+++ b/evil.rs", "+++ b/ok.rs");
        let files = parse(&split).unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].patch_twin.binary_body.len(), 6);
        assert_eq!(files[1].patch_twin.hunks.len(), 1);
    }

    /// The body grammar `git apply` reads, and the lines it does not.
    #[test]
    fn the_binary_body_grammar_is_git_s() {
        for line in [
            "",
            "literal 6",
            "delta 12",
            "NcmZQzO3KVL0ssTy0d4>Q",
            "McmZQzOv=my00M6TI{*Lx",
            "zcmV?d00001zcmV?d00001zcmV?d00001zcmV?d00001zcmV?d00001zcmV?d00001",
        ] {
            assert!(is_binary_body_line(line), "{line:?} is body");
        }
        for line in [
            "--- a/evil.rs",
            "+++ b/evil.rs",
            "@@ -1 +1 @@",
            "+y",
            "-x",
            " context",
            "index 0",
            "literal ",
            "literal 6x",
            "delta",
            "NcmZQzO3KVL0ssTy0d4>",
            "NcmZQzO3KVL0ssTy0d4>QQ",
            "NcmZQzO3KVL0ssTy0d4 Q",
            "1cmZQzO3KVL0ssTy0d4>Q",
        ] {
            assert!(!is_binary_body_line(line), "{line:?} is not body");
        }
    }

    #[test]
    fn binary_file_is_binary_changed() {
        let input = "\
diff --git a/logo.png b/logo.png
index 1111111..2222222 100644
Binary files a/logo.png and b/logo.png differ
";
        let file = one(input);
        assert_eq!(file.status, FileStatus::BinaryChanged);
        assert_eq!(file.path, "logo.png");
    }

    #[test]
    fn binary_file_uses_line_strategy_exception() {
        let input = "\
diff --git a/logo.png b/logo.png
Binary files a/logo.png and b/logo.png differ
";
        let file = one(input);
        assert_eq!(file.planner.strategy, DiffStrategy::Binary);
    }

    #[test]
    fn pure_mode_change_is_type_changed() {
        let input = "\
diff --git a/run.sh b/run.sh
old mode 100644
new mode 100755
";
        let file = one(input);
        assert_eq!(file.status, FileStatus::TypeChanged);
    }

    #[test]
    fn plain_unified_diff_without_git_header() {
        let input = "\
--- a/file.txt
+++ b/file.txt
@@ -1,1 +1,1 @@
-old
+new
";
        let file = one(input);
        assert_eq!(file.path, "file.txt");
        assert_eq!(file.status, FileStatus::Modified);
    }

    #[test]
    fn two_files_parse_independently() {
        let input = "\
diff --git a/a.txt b/a.txt
--- a/a.txt
+++ b/a.txt
@@ -1,1 +1,1 @@
-a
+A
diff --git a/b.txt b/b.txt
--- a/b.txt
+++ b/b.txt
@@ -1,1 +1,1 @@
-b
+B
";
        let files = parse(input).unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, "a.txt");
        assert_eq!(files[1].path, "b.txt");
    }

    #[test]
    fn consecutive_plain_diffs_split_on_old_header() {
        let input = "\
--- a/a.txt
+++ b/a.txt
@@ -1,1 +1,1 @@
-a
+A
--- a/b.txt
+++ b/b.txt
@@ -1,1 +1,1 @@
-b
+B
";
        let files = parse(input).unwrap();
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn removed_line_content_dashes_are_not_a_new_file() {
        // A removed line whose content begins with "-- " must not be mistaken
        // for the next file's "--- " header. Hunk counts disambiguate: while the
        // hunk still expects an old-side line, the "-"-prefixed line is removed
        // content. (Counts here are exact: old = 1 ctx + 2 removed = 3, new = 1.)
        let input = "\
diff --git a/x b/x
--- a/x
+++ b/x
@@ -1,3 +1,1 @@
 keep
-removed
--- not a header, just content
";
        let file = one(input);
        assert_eq!(file.patch_twin.hunks.len(), 1);
        let removed: Vec<_> = file.patch_twin.hunks[0]
            .lines
            .iter()
            .filter(|l| l.kind == PatchLineKind::Removed)
            .collect();
        assert_eq!(removed.len(), 2);
    }

    #[test]
    fn multiple_hunks_in_one_file() {
        let input = "\
diff --git a/x b/x
--- a/x
+++ b/x
@@ -1,1 +1,1 @@
-a
+A
@@ -10,1 +10,1 @@
-b
+B
";
        let file = one(input);
        assert_eq!(file.patch_twin.hunks.len(), 2);
        assert_eq!(file.patch_twin.hunks[1].old_start, Some(10));
    }

    #[test]
    fn hunk_ids_are_unique_and_sequential() {
        let input = "\
diff --git a/x b/x
--- a/x
+++ b/x
@@ -1,1 +1,1 @@
-a
+A
@@ -10,1 +10,1 @@
-b
+B
";
        let file = one(input);
        assert_eq!(file.patch_twin.hunks[0].id, HunkId(0));
        assert_eq!(file.patch_twin.hunks[1].id, HunkId(1));
    }

    #[test]
    fn no_newline_marker_is_captured_as_metadata() {
        let input = "\
diff --git a/x b/x
--- a/x
+++ b/x
@@ -1,1 +1,1 @@
-a
+b
\\ No newline at end of file
";
        let file = one(input);
        assert!(
            file.patch_twin
                .metadata
                .iter()
                .any(|m| m.starts_with("\\ No newline"))
        );
    }

    #[test]
    fn malformed_hunk_header_is_an_error() {
        let input = "\
diff --git a/x b/x
--- a/x
+++ b/x
@@ this is not a hunk header @@
-a
+b
";
        let err = parse(input).unwrap_err();
        assert!(matches!(err, PatchParseError::MalformedHunkHeader { .. }));
    }

    #[test]
    fn malformed_header_reports_line_number() {
        let input = "diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ bad @@\n";
        match parse(input).unwrap_err() {
            PatchParseError::MalformedHunkHeader { line_number, .. } => {
                assert_eq!(line_number, 4);
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn body_line_outside_hunk_is_an_error() {
        let input = "\
diff --git a/x b/x
--- a/x
+++ b/x
+stray addition with no hunk
";
        let err = parse(input).unwrap_err();
        assert!(matches!(err, PatchParseError::BodyLineOutsideHunk { .. }));
    }

    /// An error quotes at most `ERROR_EXCERPT_BYTES` of the offending line, escaped, and counts
    /// the rest. Before, a 2 MB line became a 2,000,074-byte message in an agent's context.
    #[test]
    fn an_error_quotes_a_bounded_excerpt_of_the_offending_line() {
        let long = format!(
            "diff --git a/x b/x\n--- a/x\n+++ b/x\n+{}\n",
            "é".repeat(1_000_000)
        );
        let message = parse(&long).unwrap_err().to_string();
        assert!(message.len() < 250, "{} bytes", message.len());
        assert!(message.ends_with(" bytes)"), "{message}");
        assert!(
            message.contains(&format!("(+{} bytes)", 2_000_001 - 119)),
            "{message}"
        );
        let short = parse("diff --git a/x b/x\n--- a/x\n+++ b/x\n+stray \u{1b}[2J\n")
            .unwrap_err()
            .to_string();
        assert!(short.ends_with(r#": "+stray \u{1b}[2J""#), "{short}");
        let header = parse(&format!("--- a/x\n+++ b/x\n@@ {}\n", "x".repeat(500)))
            .unwrap_err()
            .to_string();
        assert!(header.contains("(+383 bytes)"), "{header}");
    }

    /// A hunk's address names it within one review; its body digest follows the change.
    /// (Expert circle, 2026-10-28: content addresses must not move when other files change, and
    /// must not survive a change to the hunk itself.)
    #[test]
    fn hunk_addresses_and_body_digests() {
        let one = "--- a/x\n+++ b/x\n@@ -1,2 +1,2 @@ fn f()\n a\n-b\n+c\n";
        let other = "--- a/y\n+++ b/y\n@@ -1 +1 @@\n-p\n+q\n";
        let hunk = |patch: &str, file: usize| {
            let files = parse(patch).unwrap();
            let h = files[file].patch_twin.hunks[0].clone();
            (h.address(&files[file].path), h.body_sha256(), h)
        };
        let (address, body, h) = hunk(one, 0);
        assert_eq!(h.section.as_deref(), Some("fn f()"));
        assert_eq!(h.counts(), (2, 2));
        assert_eq!(address.len(), 64);
        // Another file before it changes neither.
        let (a2, b2, _) = hunk(&format!("{other}{one}"), 1);
        assert_eq!((a2, b2), (address.clone(), body.clone()));
        // The same body at another position keeps its digest but not its address.
        let (a3, b3, _) = hunk(&one.replace("@@ -1,2 +1,2 @@", "@@ -9,2 +9,2 @@"), 0);
        assert_eq!(b3, body);
        assert_ne!(a3, address);
        // One byte of the body changes both.
        let (a4, b4, _) = hunk(&one.replace("+c", "+d"), 0);
        assert_ne!((a4, b4), (address.clone(), body.clone()));
        // Another path changes the address only.
        let (a5, b5, _) = hunk(&one.replace("/x", "/z"), 0);
        assert_eq!(b5, body);
        assert_ne!(a5, address);
        // The heading is display only: it changes neither, and is never rendered.
        let (a6, b6, h6) = hunk(&one.replace(" fn f()", ""), 0);
        assert_eq!((a6, b6, h6.section), (address, body, None));
        assert!(!crate::render_unified(&parse(one).unwrap()).contains("fn f()"));
        // The no-newline marker is part of the body.
        let (_, b7, _) = hunk(&format!("{one}\\ No newline at end of file\n"), 0);
        assert_ne!(b7, hunk(one, 0).1);
    }

    /// A line after a file's hunk that git ignores is refused, not taken as a header: before,
    /// each of these changed the reviewed status, path or mode, or made a phantom file, while
    /// `git apply` applied a plain modification of `x` (bug hunt 2026-10-28, H1-01).
    #[test]
    fn headers_after_a_hunk_are_refused() {
        let head = "diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n";
        for trailing in [
            "new file mode 100644\n",
            "deleted file mode 100644\n",
            "rename to evil\n",
            "old mode 100644\nnew mode 100755\n",
            "+++ b/z\n",
            "--- /dev/null\n",
            "index 1111111..2222222 100644\n",
        ] {
            let err = parse(&format!("{head}{trailing}")).unwrap_err();
            assert!(
                matches!(err, PatchParseError::MisplacedHeader { .. }),
                "{trailing:?}: {err:?}"
            );
        }
        // A next file's own `---`/`+++` pair, or `diff --git`, still starts a file.
        assert_eq!(
            parse(&format!("{head}--- a/y\n+++ b/y\n@@ -1 +1 @@\n-c\n+d\n"))
                .unwrap()
                .len(),
            2
        );
        assert_eq!(parse(&format!("{head}{head}")).unwrap().len(), 2);
    }

    /// Input git refuses as corrupt is refused here too (bug hunt 2026-10-28, H1-04).
    #[test]
    fn input_git_refuses_is_refused() {
        let refused = |input: &str| parse(input).unwrap_err();
        assert!(matches!(
            refused("--- a/x\n+++ b/x\n@@ -+1,+1 +1 @@\n-a\n+b\n"),
            PatchParseError::MalformedHunkHeader { .. }
        ));
        assert!(matches!(
            refused("--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b"),
            PatchParseError::UnterminatedLine { line_number: 5 }
        ));
        assert!(matches!(
            refused(
                "diff --git a/x b/x\nnew file mode 100644\n--- a/x\n+++ b/x\n@@ -0,0 +1 @@\n+b\n"
            ),
            PatchParseError::InconsistentHeader { .. }
        ));
        assert!(matches!(
            refused(
                "diff --git a/x b/x\ndeleted file mode 100644\n--- a/x\n+++ b/x\n@@ -1 +0,0 @@\n-a\n"
            ),
            PatchParseError::InconsistentHeader { .. }
        ));
        assert!(matches!(
            refused("--- a/x\n+++ b/x\n+++ b/y\n@@ -1 +1 @@\n-a\n+b\n"),
            PatchParseError::MisplacedHeader { line_number: 3 }
        ));
        assert!(matches!(
            refused("--- a/x\n@@ -1 +1 @@\n-a\n+b\n"),
            PatchParseError::MisplacedHeader { line_number: 2 }
        ));
        assert_eq!(parse_range("1,2,3"), None);
        // A final context or EOF-marker line, and preamble without a line feed, are fine.
        assert!(
            parse("--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n\\ No newline at end of file\n").is_ok()
        );
        assert!(parse("--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\ntrailing note").is_ok());
    }

    #[test]
    fn byte_budget_is_enforced() {
        let options = ParseOptions { byte_budget: 8 };
        let err = parse_with("diff --git a/x b/x\n", options).unwrap_err();
        assert!(matches!(err, PatchParseError::BudgetExceeded { .. }));
    }

    #[test]
    fn structural_line_budget_is_enforced_before_model_amplification() {
        let input = "preamble\n".repeat(DEFAULT_LINE_BUDGET + 1);
        let options = ParseOptions {
            byte_budget: input.len(),
        };
        let err = parse_with(&input, options).unwrap_err();
        assert!(matches!(
            err,
            PatchParseError::LineBudgetExceeded {
                limit_lines: DEFAULT_LINE_BUDGET,
                actual_lines
            } if actual_lines == DEFAULT_LINE_BUDGET + 1
        ));
    }

    #[test]
    fn budget_error_reports_sizes() {
        let options = ParseOptions { byte_budget: 4 };
        match parse_with("abcdefgh", options).unwrap_err() {
            PatchParseError::BudgetExceeded {
                limit_bytes,
                actual_bytes,
            } => {
                assert_eq!(limit_bytes, 4);
                assert_eq!(actual_bytes, 8);
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn preamble_lines_before_first_file_are_ignored() {
        let input = "\
From abc Mon Sep 17 00:00:00 2001
Subject: [PATCH] do a thing

 This is an indented commit message line.
diff --git a/x b/x
--- a/x
+++ b/x
@@ -1,1 +1,1 @@
-a
+b
";
        let files = parse(input).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "x");
    }

    #[test]
    fn hunk_without_explicit_count_defaults_to_one() {
        let input = "\
--- a/x
+++ b/x
@@ -1 +1 @@
-a
+b
";
        let file = one(input);
        assert_eq!(file.patch_twin.hunks[0].lines.len(), 2);
    }

    #[test]
    fn context_only_blank_line_is_empty_text() {
        let input = "\
--- a/x
+++ b/x
@@ -1,3 +1,3 @@
 a

-b
+B
";
        let file = one(input);
        let blank = &file.patch_twin.hunks[0].lines[1];
        assert_eq!(blank.kind, PatchLineKind::Context);
        assert_eq!(blank.text, "");
    }

    #[test]
    fn parse_with_large_budget_accepts_normal_input() {
        let files = parse_with(BASIC, ParseOptions::default()).unwrap();
        assert_eq!(files.len(), 1);
    }

    #[test]
    fn strip_ab_removes_prefixes() {
        assert_eq!(strip_ab("a/src/x.rs"), "src/x.rs");
        assert_eq!(strip_ab("b/src/x.rs"), "src/x.rs");
        assert_eq!(strip_ab("plain"), "plain");
    }

    #[test]
    fn header_path_drops_trailing_tab_timestamp() {
        assert_eq!(header_path("a/x.txt\t2026-06-21 12:00:00"), "a/x.txt");
    }

    #[test]
    fn parse_range_defaults_count() {
        assert_eq!(parse_range("5"), Some((5, 1)));
        assert_eq!(parse_range("5,3"), Some((5, 3)));
        assert_eq!(parse_range("x"), None);
    }

    // --- hunk-header closing `@@` enforcement (patch truth) ---
    //
    // A valid unified hunk header is: `@@ -old[,count] +new[,count] @@ [section]`
    // The closing `@@` is mandatory. All REJECT tests below would have been
    // accepted (parse returning Ok with a hunk) before the fix that enforces the
    // closer; they now produce `MalformedHunkHeader`.

    /// Basic header with explicit counts — must parse successfully end-to-end.
    #[test]
    fn hunk_header_accepts_basic_with_counts() {
        let input = "--- a/x\n+++ b/x\n@@ -1,1 +1,1 @@\n-old\n+new\n";
        let file = one(input);
        assert_eq!(file.patch_twin.hunks.len(), 1);
    }

    /// Header where both counts default to 1 (omitted) — must parse successfully.
    #[test]
    fn hunk_header_accepts_default_counts() {
        let input = "--- a/x\n+++ b/x\n@@ -1 +1 @@\n-old\n+new\n";
        let file = one(input);
        assert_eq!(file.patch_twin.hunks.len(), 1);
    }

    /// Header with trailing section-heading text after the `@@` closer — must parse
    /// successfully; the section text is optional and must be accepted and ignored.
    #[test]
    fn hunk_header_accepts_section_text() {
        let input = "--- a/x\n+++ b/x\n@@ -10,7 +10,6 @@ fn foo()\n-a\n-b\n-c\n-d\n-e\n-f\n-g\n+A\n+B\n+C\n+D\n+E\n+F\n";
        let file = one(input);
        assert_eq!(file.patch_twin.hunks.len(), 1);
        assert_eq!(file.patch_twin.hunks[0].old_start, Some(10));
    }

    /// Pure-addition hunk (`-0,0 +1,5`) — must parse successfully.
    #[test]
    fn hunk_header_accepts_add_from_zero() {
        let input = "--- /dev/null\n+++ b/x\n@@ -0,0 +1,5 @@\n+a\n+b\n+c\n+d\n+e\n";
        let file = one(input);
        assert_eq!(file.patch_twin.hunks.len(), 1);
        let adds = file.patch_twin.hunks[0]
            .lines
            .iter()
            .filter(|l| l.kind == PatchLineKind::Added)
            .count();
        assert_eq!(adds, 5);
    }

    /// Trailing garbage with NO closing `@@` — the closer is mandatory; this was
    /// silently accepted before the fix (parse succeeded, garbage discarded).
    #[test]
    fn hunk_header_rejects_trailing_garbage_no_closer() {
        let input = "--- a/x\n+++ b/x\n@@ -1,1 +1,1 NOT_A_HUNK\n-old\n+new\n";
        let err = parse(input).unwrap_err();
        assert!(
            matches!(err, PatchParseError::MalformedHunkHeader { .. }),
            "expected MalformedHunkHeader, got {err:?}"
        );
    }

    /// Completely missing closing `@@` — header ends after the `+new[,count]` token
    /// with nothing following; was silently accepted before the fix.
    #[test]
    fn hunk_header_requires_closing_at_at() {
        let input = "--- a/x\n+++ b/x\n@@ -1,1 +1,1\n-old\n+new\n";
        let err = parse(input).unwrap_err();
        assert!(
            matches!(err, PatchParseError::MalformedHunkHeader { .. }),
            "expected MalformedHunkHeader, got {err:?}"
        );
    }

    /// Triple-`@@@` closer — the closing token must equal exactly `@@`; `@@@` is
    /// a different token and must be rejected.  Was silently accepted before the fix
    /// because the old code never inspected the closer at all.
    #[test]
    fn hunk_header_rejects_triple_at_closer() {
        let input = "--- a/x\n+++ b/x\n@@ -1,1 +1,1 @@@\n-old\n+new\n";
        let err = parse(input).unwrap_err();
        assert!(
            matches!(err, PatchParseError::MalformedHunkHeader { .. }),
            "expected MalformedHunkHeader, got {err:?}"
        );
    }

    /// Direct unit-level check: `parse_hunk_header` returns `None` on the
    /// canonical bad input so the call-site correctly maps it to an error.
    #[test]
    fn parse_hunk_header_unit_rejects_missing_closer() {
        assert!(parse_hunk_header("@@ -1,1 +1,1 NOT_A_HUNK").is_none());
        assert!(parse_hunk_header("@@ -1,1 +1,1").is_none());
        assert!(parse_hunk_header("@@ -1,1 +1,1 @@@").is_none());
    }

    /// Direct unit-level check: `parse_hunk_header` returns `Some` for all
    /// valid header forms.
    #[test]
    fn parse_hunk_header_unit_accepts_valid_forms() {
        assert!(parse_hunk_header("@@ -1,1 +1,1 @@").is_some());
        assert!(parse_hunk_header("@@ -1 +1 @@").is_some());
        assert!(parse_hunk_header("@@ -10,7 +10,6 @@ fn foo()").is_some());
        assert!(parse_hunk_header("@@ -0,0 +1,5 @@").is_some());
    }

    // --- OPEN-003: spaced-path tokenization (FIXED) ---

    /// Regression: `diff --git a/foo bar.rs b/foo bar.rs` must produce the
    /// correct path instead of truncating at the first space.
    #[test]
    fn spaced_path_git_header_is_parsed_correctly() {
        let input = "\
diff --git a/foo bar.rs b/foo bar.rs
--- a/foo bar.rs
+++ b/foo bar.rs
@@ -1,1 +1,1 @@
-old
+new
";
        let files = parse(input).expect("parse should succeed");
        assert_eq!(files.len(), 1);
        // The ---/+++ headers (header_path) supply the authoritative path.
        assert_eq!(files[0].path, "foo bar.rs");
    }

    /// Multiple spaces in the filename — rfind(" b/") must find the rightmost
    /// occurrence and not truncate at the first space.
    #[test]
    fn spaced_path_multiple_spaces_in_filename() {
        let input = "\
diff --git a/a b c.rs b/a b c.rs
--- a/a b c.rs
+++ b/a b c.rs
@@ -1 +1 @@
-x
+y
";
        let files = parse(input).expect("parse should succeed");
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "a b c.rs");
    }
}
