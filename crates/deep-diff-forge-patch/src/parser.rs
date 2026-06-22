use crate::{DEFAULT_BYTE_BUDGET, PatchParseError};
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
    if input.len() > options.byte_budget {
        return Err(PatchParseError::BudgetExceeded {
            limit_bytes: options.byte_budget,
            actual_bytes: input.len(),
        });
    }

    let mut parser = Parser::default();
    for (index, line) in input.lines().enumerate() {
        parser.feed(line, index + 1)?;
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
    hunks: Vec<PatchHunk>,
}

/// Working accumulator for one hunk.
struct HunkBuf {
    id: HunkId,
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
}

impl Parser {
    fn feed(&mut self, line: &str, line_number: usize) -> Result<(), PatchParseError> {
        self.last_line = line_number;
        // The "\ No newline at end of file" marker applies to the preceding
        // line and is recorded as file metadata, whether or not a hunk is still
        // open (it can arrive after the hunk's line counts are exhausted).
        if line.starts_with('\\') {
            if let Some(file) = self.file.as_mut() {
                file.metadata.push(line.to_string());
            }
            return Ok(());
        }
        // While a hunk is open and not yet exhausted, body lines belong to it.
        if let Some(hunk) = self.hunk.as_mut() {
            if hunk.rem_old > 0 || hunk.rem_new > 0 {
                if let Some(consumed) = hunk.consume_body(line, line_number) {
                    consumed?;
                    if hunk.rem_old == 0 && hunk.rem_new == 0 {
                        // Counts satisfied: a clean close (cannot truncate).
                        self.close_hunk(line_number)?;
                    }
                    return Ok(());
                }
            }
            // A non-body line arrived while the hunk still expects content:
            // closing here rejects the truncated hunk.
            self.close_hunk(line_number)?;
        }
        self.dispatch_non_body(line, line_number)
    }

    fn dispatch_non_body(&mut self, line: &str, line_number: usize) -> Result<(), PatchParseError> {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            return self.start_git_file(rest, line_number);
        }
        if let Some(rest) = line.strip_prefix("--- ") {
            return self.on_old_header(rest, line_number);
        }
        if let Some(rest) = line.strip_prefix("+++ ") {
            self.on_new_header(rest);
            return Ok(());
        }
        if line.starts_with("@@") {
            return self.open_hunk(line, line_number);
        }
        if self.try_metadata(line) {
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
        self.flush_file();
        let mut file = FileBuf::default();
        let mut tokens = rest.split_whitespace();
        file.git_old = tokens.next().map(strip_ab);
        file.git_new = tokens.next().map(strip_ab);
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
            self.flush_file();
            self.file = Some(FileBuf::default());
        }
        if let Some(file) = self.file.as_mut() {
            file.old_raw = Some(header_path(rest));
        }
        Ok(())
    }

    fn on_new_header(&mut self, rest: &str) {
        if self.file.is_none() {
            self.file = Some(FileBuf::default());
        }
        if let Some(file) = self.file.as_mut() {
            file.new_raw = Some(header_path(rest));
        }
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
        let id = HunkId(self.next_hunk_id);
        self.next_hunk_id += 1;
        self.hunk = Some(HunkBuf {
            id,
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

    fn try_metadata(&mut self, line: &str) -> bool {
        let Some(file) = self.file.as_mut() else {
            return false;
        };
        if let Some(path) = line.strip_prefix("rename from ") {
            file.rename_from = Some(path.to_string());
            file.metadata.push(line.to_string());
            return true;
        }
        if let Some(path) = line.strip_prefix("rename to ") {
            file.rename_to = Some(path.to_string());
            file.metadata.push(line.to_string());
            return true;
        }
        if META_PREFIXES.iter().any(|p| line.starts_with(p)) {
            file.metadata.push(line.to_string());
            return true;
        }
        false
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
                    lines: hunk.lines,
                });
            }
        }
        Ok(())
    }

    fn flush_file(&mut self) {
        if let Some(file) = self.file.take() {
            self.files.push(file.into_review_file());
        }
    }

    fn finish(mut self) -> Result<Vec<ReviewFile>, PatchParseError> {
        // End of input: any still-open hunk must have satisfied its counts.
        self.close_hunk(self.last_line)?;
        self.flush_file();
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
                hunks: self.hunks,
                metadata: self.metadata,
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
}

/// Parse `@@ -a,b +c,d @@` (counts optional, defaulting to 1).
fn parse_hunk_header(line: &str) -> Option<HunkHeader> {
    let rest = line.strip_prefix("@@")?.trim_start();
    let rest = rest.strip_prefix('-')?;
    let mut parts = rest.splitn(2, ' ');
    let old = parts.next()?;
    let after = parts.next()?.trim_start();
    let new = after.strip_prefix('+')?;
    let new = new.split_whitespace().next()?;
    let (old_start, old_count) = parse_range(old)?;
    let (new_start, new_count) = parse_range(new)?;
    Some(HunkHeader {
        old_start,
        old_count,
        new_start,
        new_count,
    })
}

/// Parse a `start` or `start,count` range; count defaults to 1.
fn parse_range(range: &str) -> Option<(u32, u32)> {
    let mut nums = range.split(',');
    let start = nums.next()?.parse::<u32>().ok()?;
    let count = match nums.next() {
        Some(c) => c.parse::<u32>().ok()?,
        None => 1,
    };
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
    rest.split('\t')
        .next()
        .unwrap_or(rest)
        .trim_end()
        .to_string()
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

    #[test]
    fn byte_budget_is_enforced() {
        let options = ParseOptions { byte_budget: 8 };
        let err = parse_with("diff --git a/x b/x\n", options).unwrap_err();
        assert!(matches!(err, PatchParseError::BudgetExceeded { .. }));
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
}
