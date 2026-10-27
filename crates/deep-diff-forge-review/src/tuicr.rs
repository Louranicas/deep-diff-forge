// Review council: Read-only export of admitted council findings into a disposable local tuicr session as draft notes.
// Design/status: docs/REVIEW_COUNCIL.md#tuicr-export
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Comparisons%2FArchitecture%20Lessons%20and%20Interoperability
//! DDF→tuicr adapter (DDF-R54). Admitted council findings become tuicr *local draft* notes.
//!
//! The adapter never submits to a forge, marks a file reviewed or resolves a comment: it only runs
//! `tuicr review list` and `tuicr review add` with fixed argv. A finding is line-anchored only when
//! the bytes the session reviews equal the run's sealed snapshot copy; otherwise the export is
//! refused unless the caller explicitly downgrades such findings to review-level notes.
use crate::model::Qualification;
use crate::storage::{self, SNAPSHOT_DIR};
use crate::{
    Execution, Manifest, Outcome, Provider, Report, ReviewFailure, Reviewer, Round, Severity,
    load_manifest, validate_report,
};
use deep_diff_forge_core::sha256_hex;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Write as _;
use std::os::unix::fs::PermissionsExt;
use std::{
    error::Error,
    fmt, fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

/// The only tuicr session format this adapter has been checked against (tuicr 0.27.0).
pub const SESSION_VERSION: &str = "1.3";
pub(crate) const SESSION_CAP: u64 = 32 * 1024 * 1024;
const LOG_CAP: u64 = 16 * 1024 * 1024;
pub(crate) const SOURCE_CAP: u64 = 64 * 1024 * 1024;
pub(crate) const LEDGER_DIR: &str = "exports";
const NOT_AUTHORITY: &str = "Model claim, not verified fact. Local draft note only: not an approval, forge post, review mark or disposition.";

/// Typed refusal or failure of a tuicr export. Every variant is a refusal to write.
#[derive(Debug)]
#[non_exhaustive]
pub enum TuicrExportError {
    /// The run itself is unreadable, unsealed or inconsistent.
    Run(ReviewFailure),
    /// A report is present but is not an admitted, controller-qualified report of this run.
    NotAdmitted { report: String, reason: String },
    /// The session file is not a supported local tuicr session.
    UnsupportedSession(String),
    /// The session is bound to a forge pull request; exporting there could become a forge post.
    ForgeSession,
    /// The session reviews a different repository than the run.
    RepositoryMismatch { session: String, run: String },
    /// A finding cites a candidate file the session does not review at the sealed bytes.
    Unanchored {
        finding: String,
        path: String,
        reason: String,
    },
    /// A citation range does not lie within the reviewed file.
    AnchorOutOfRange {
        finding: String,
        path: String,
        start: usize,
        end: usize,
        lines: usize,
    },
    /// tuicr does not list the session file as a stored session for the run's repository.
    NotInStore { session: String },
    /// No usable `tuicr` executable was found on `PATH`.
    TuicrUnavailable(String),
    /// tuicr exited unsuccessfully; its stderr is retained verbatim.
    TuicrFailed { status: Option<i32>, stderr: String },
    /// tuicr exited zero but its output did not describe the requested comment.
    TuicrOutput(String),
    /// tuicr reported a comment that is not in the session file afterwards.
    SessionDiverged { comment_id: String },
    /// A previous export of this comment was interrupted and the comment cannot be found.
    InterruptedExport { key: String, pending: String },
}
impl fmt::Display for TuicrExportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Run(e) => write!(f, "tuicr export refused (run): {e}"),
            Self::NotAdmitted { report, reason } => {
                write!(f, "tuicr export refused (not admitted): {report}: {reason}")
            }
            Self::UnsupportedSession(reason) => {
                write!(f, "tuicr export refused (unsupported session): {reason}")
            }
            Self::ForgeSession => f.write_str(
                "tuicr export refused (forge session): pull-request sessions can be published; export only into local sessions",
            ),
            Self::RepositoryMismatch { session, run } => write!(
                f,
                "tuicr export refused (repository mismatch): session reviews {session}, run reviewed {run}"
            ),
            Self::Unanchored {
                finding,
                path,
                reason,
            } => write!(
                f,
                "tuicr export refused (unanchored): finding {finding} cites {path}: {reason}; pass --allow-review-level to file such findings as review-level notes"
            ),
            Self::AnchorOutOfRange {
                finding,
                path,
                start,
                end,
                lines,
            } => write!(
                f,
                "tuicr export refused (anchor out of range): finding {finding} cites {path}:{start}-{end} but the reviewed file has {lines} line(s); pass --allow-review-level to file it as a review-level note"
            ),
            Self::NotInStore { session } => write!(
                f,
                "tuicr export refused (not in store): tuicr does not list {session} for the run repository; tuicr would save edits elsewhere"
            ),
            Self::TuicrUnavailable(reason) => write!(f, "tuicr unavailable: {reason}"),
            Self::TuicrFailed { status, stderr } => {
                write!(f, "tuicr failed (exit {status:?}): {}", stderr.trim_end())
            }
            Self::TuicrOutput(reason) => write!(f, "tuicr output rejected: {reason}"),
            Self::SessionDiverged { comment_id } => write!(
                f,
                "tuicr export stopped (diverged): tuicr reported comment {comment_id} but the session file does not contain it"
            ),
            Self::InterruptedExport { key, pending } => write!(
                f,
                "tuicr export refused (interrupted): {key} has intent marker {pending} but no matching comment in the session; inspect the session, then remove the marker"
            ),
        }
    }
}
impl Error for TuicrExportError {}
impl From<ReviewFailure> for TuicrExportError {
    fn from(e: ReviewFailure) -> Self {
        Self::Run(e)
    }
}
impl From<std::io::Error> for TuicrExportError {
    fn from(e: std::io::Error) -> Self {
        Self::Run(e.into())
    }
}
impl From<serde_json::Error> for TuicrExportError {
    fn from(e: serde_json::Error) -> Self {
        Self::Run(e.into())
    }
}
impl From<TuicrExportError> for ReviewFailure {
    fn from(e: TuicrExportError) -> Self {
        Self(e.to_string())
    }
}
type Result<T> = std::result::Result<T, TuicrExportError>;

/// Caller choices for [`export_tuicr`].
#[derive(Debug, Clone, Copy, Default)]
pub struct ExportOptions {
    /// Perform writes; without it the export is a pure local dry run that never invokes tuicr.
    pub execute: bool,
    /// File findings whose anchor cannot be verified as review-level notes instead of refusing.
    pub allow_review_level: bool,
}

/// Where a planned note attaches in the tuicr session. Line targets are always on the new side.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Target {
    Review,
    Line {
        file: String,
        line: usize,
        side: &'static str,
    },
    LineRange {
        file: String,
        start_line: usize,
        end_line: usize,
        side: &'static str,
    },
}

/// Ledger state of one planned note.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NoteStatus {
    Planned,
    Created,
    AlreadyExported,
    Recovered,
}

/// One tuicr note derived from one admitted report element.
#[derive(Debug, Clone, Serialize)]
pub struct PlannedNote {
    pub key: String,
    pub reviewer_id: String,
    pub round: Round,
    pub finding_id: Option<String>,
    pub severity: Option<Severity>,
    pub observed_model: String,
    pub author: String,
    pub target: Target,
    pub downgraded: Option<String>,
    pub status: NoteStatus,
    pub tuicr_comment_id: Option<String>,
    pub content: String,
}

/// A report present in the run but not exported, with the reason.
#[derive(Debug, Clone, Serialize)]
pub struct SkippedReport {
    pub report: String,
    pub reason: String,
}

/// Machine-readable export result.
#[derive(Debug, Clone, Serialize)]
pub struct TuicrExport {
    pub schema: &'static str,
    pub run_id: String,
    pub session: String,
    pub session_id: String,
    pub diff_source: String,
    pub session_revision: String,
    pub executed: bool,
    pub ledger: String,
    pub authority: &'static str,
    pub skipped_reports: Vec<SkippedReport>,
    pub notes: Vec<PlannedNote>,
}

/// The fields of a tuicr 1.3 session this adapter relies on; other fields are ignored.
#[derive(Debug, Deserialize)]
pub(crate) struct Session {
    pub(crate) id: String,
    version: String,
    repo_path: PathBuf,
    pub(crate) base_commit: String,
    #[serde(default)]
    pub(crate) diff_source: Option<String>,
    #[serde(default)]
    commit_range: Option<Vec<String>>,
    #[serde(default)]
    pr_session_key: Option<serde_json::Value>,
    files: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    review_comments: Vec<serde_json::Value>,
}
impl Session {
    /// Every stored comment as `(id, content)`, across review, file and line scopes.
    fn comments(&self) -> Vec<(String, String)> {
        let mut values: Vec<&serde_json::Value> = self.review_comments.iter().collect();
        for file in self.files.values() {
            if let Some(list) = file["file_comments"].as_array() {
                values.extend(list);
            }
            if let Some(lines) = file["line_comments"].as_object() {
                for list in lines.values().filter_map(serde_json::Value::as_array) {
                    values.extend(list);
                }
            }
        }
        values
            .into_iter()
            .filter_map(|c| {
                Some((
                    c["id"].as_str()?.to_owned(),
                    c["content"].as_str()?.to_owned(),
                ))
            })
            .collect()
    }
}

/// Which bytes a session shows on its new side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Revision {
    WorkingTree,
    Index,
    Commit(String),
}
impl Revision {
    pub(crate) fn label(&self) -> String {
        match self {
            Self::WorkingTree => "working_tree".into(),
            Self::Index => "index".into(),
            Self::Commit(sha) => format!("commit:{sha}"),
        }
    }
}
pub(crate) fn commit_id(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|b| b.is_ascii_hexdigit())
}
/// Map tuicr's `diff_source` to the revision whose bytes its new side displays.
fn revision(session: &Session) -> Result<Revision> {
    // tuicr's `SessionDiffSource` defaults to `working_tree` when the field is absent.
    match session.diff_source.as_deref().unwrap_or("working_tree") {
        "working_tree"
        | "unstaged"
        | "staged_and_unstaged"
        | "working_tree_and_commits"
        | "staged_unstaged_and_commits"
        | "file"
        | "pristine" => Ok(Revision::WorkingTree),
        "staged" => Ok(Revision::Index),
        "commit_range" => {
            let base = &session.base_commit;
            if !commit_id(base) {
                return Err(TuicrExportError::UnsupportedSession(
                    "commit_range session base_commit is not a full commit id".into(),
                ));
            }
            // tuicr stores the newest selected commit as base_commit and the full range alongside.
            if let Some(range) = &session.commit_range
                && range.last() != Some(base)
            {
                return Err(TuicrExportError::UnsupportedSession(
                    "commit_range session base_commit is not the newest commit of its range".into(),
                ));
            }
            Ok(Revision::Commit(base.clone()))
        }
        "pull_request" => Err(TuicrExportError::ForgeSession),
        other => Err(TuicrExportError::UnsupportedSession(format!(
            "unknown diff_source {other:?}"
        ))),
    }
}

/// Number of lines as `str::lines` counts them: a final newline does not start a new line.
fn line_count(bytes: &[u8]) -> usize {
    bytes.split(|b| *b == b'\n').count() - usize::from(bytes.is_empty() || bytes.ends_with(b"\n"))
}
/// A new-side tuicr target for a 1-based inclusive range inside a file of `lines` lines.
fn anchor(finding: &str, path: &str, start: usize, end: usize, lines: usize) -> Result<Target> {
    if start == 0 || end < start || end > lines {
        return Err(TuicrExportError::AnchorOutOfRange {
            finding: finding.into(),
            path: path.into(),
            start,
            end,
            lines,
        });
    }
    Ok(if start == end {
        Target::Line {
            file: path.into(),
            line: start,
            side: "new",
        }
    } else {
        Target::LineRange {
            file: path.into(),
            start_line: start,
            end_line: end,
            side: "new",
        }
    })
}

/// Keep a provider-reported model name printable and short; anything else is not an identity.
fn model_name(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()
        && value.len() <= 120
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._:/@[]-".contains(&b)))
    .then(|| value.to_owned())
}
/// Model identity observed in a provider transcript: Claude's `modelUsage` keys or Codex's `model:` line.
fn observed_model(provider: Provider, stdout: &[u8], stderr: &[u8]) -> String {
    let names: Vec<String> = match provider {
        Provider::ClaudeCode => serde_json::from_slice::<serde_json::Value>(stdout)
            .ok()
            .and_then(|v| {
                v["modelUsage"]
                    .as_object()
                    .map(|o| o.keys().filter_map(|k| model_name(k)).collect())
            })
            .unwrap_or_default(),
        Provider::Codex => String::from_utf8_lossy(stderr)
            .lines()
            .find_map(|line| line.strip_prefix("model:"))
            .and_then(model_name)
            .into_iter()
            .collect(),
    };
    if names.is_empty() {
        "unknown".into()
    } else {
        names.join(",")
    }
}

/// An admitted report with the evidence identifying it.
struct Admitted {
    report: Report,
    reviewer: Reviewer,
    file: String,
    sha256: String,
    model: String,
}
/// Reports qualified by the controller for this run; manual imports are skipped, tampering refused.
fn admitted_reports(
    directory: &Path,
    manifest: &Manifest,
) -> Result<(Vec<Admitted>, Vec<SkippedReport>)> {
    let mut admitted = Vec::new();
    let mut skipped = Vec::new();
    for reviewer in &manifest.request.reviewers {
        for round in [Round::Independent, Round::Challenge] {
            let path = storage::report_path(directory, &reviewer.id, round);
            if !path.exists() {
                continue;
            }
            let file = format!("reports/{}-{}.json", reviewer.id, round.label());
            let refuse = |reason: String| TuicrExportError::NotAdmitted {
                report: file.clone(),
                reason,
            };
            let bytes = storage::read_bytes(&path, storage::REPORT_CAP)?;
            let report: Report =
                serde_json::from_slice(&bytes).map_err(|e| refuse(e.to_string()))?;
            validate_report(manifest, &report).map_err(|e| refuse(e.to_string()))?;
            if report.reviewer_id != reviewer.id || report.round != round {
                return Err(refuse("report identity does not match its file".into()));
            }
            let job =
                directory
                    .join("jobs")
                    .join(format!("review-{}-{}", reviewer.id, round.label()));
            let qualification = job.join("qualification.json");
            if !qualification.exists() {
                skipped.push(SkippedReport {
                    report: file,
                    reason: "manual import without controller qualification; not provider evidence"
                        .into(),
                });
                continue;
            }
            let q: Qualification = storage::read_json(&qualification, storage::REPORT_CAP)?;
            let sha256 = sha256_hex(&bytes);
            if q.outcome != Outcome::Passed || q.report_sha256.as_deref() != Some(sha256.as_str()) {
                return Err(refuse(
                    "report bytes do not match a passed controller qualification".into(),
                ));
            }
            let receipt: Execution =
                storage::read_json(&job.join("execution.json"), storage::REPORT_CAP)?;
            let stdout = storage::read_bytes(&job.join("stdout.log"), LOG_CAP)?;
            let stderr = storage::read_bytes(&job.join("stderr.log"), LOG_CAP)?;
            if receipt.run_id != manifest.run_id
                || sha256_hex(&stdout) != receipt.stdout_sha256
                || sha256_hex(&stderr) != receipt.stderr_sha256
            {
                return Err(refuse(
                    "provider transcript does not match its execution receipt".into(),
                ));
            }
            admitted.push(Admitted {
                model: observed_model(reviewer.provider, &stdout, &stderr),
                report,
                reviewer: reviewer.clone(),
                file,
                sha256,
            });
        }
    }
    Ok((admitted, skipped))
}

/// Verified identity between a run and the session it exports into.
pub(crate) struct Binding<'a> {
    pub(crate) directory: &'a Path,
    pub(crate) manifest: &'a Manifest,
    pub(crate) repository: PathBuf,
    pub(crate) session: &'a Session,
    pub(crate) revision: Revision,
}
impl Binding<'_> {
    /// Bytes the session reviews for `path`, or why there are none.
    pub(crate) fn session_bytes(&self, path: &str) -> std::result::Result<Vec<u8>, String> {
        match &self.revision {
            Revision::WorkingTree => {
                let file = self.repository.join(path);
                let canonical = fs::canonicalize(&file).map_err(|e| e.to_string())?;
                if !canonical.starts_with(&self.repository) {
                    return Err("working-tree path escapes the repository".into());
                }
                storage::read_bytes(&file, SOURCE_CAP).map_err(|e| e.to_string())
            }
            Revision::Index => storage::git_bytes(
                &self.repository,
                &["cat-file", "blob", &format!(":{path}")],
                SOURCE_CAP,
            )
            .map_err(|e| format!("index has no blob: {e}")),
            Revision::Commit(sha) => storage::git_bytes(
                &self.repository,
                &["cat-file", "blob", &format!("{sha}:{path}")],
                SOURCE_CAP,
            )
            .map_err(|e| format!("commit {sha} has no blob: {e}")),
        }
    }
    /// Resolve a finding's target: the first candidate-file citation, verified against sealed bytes.
    fn target(&self, finding: &crate::Finding) -> Result<Target> {
        let Some((citation, path)) = finding.citations.iter().find_map(|c| {
            c.artifact_id
                .strip_prefix("candidate:")
                .map(|path| (c, path))
        }) else {
            return Ok(Target::Review);
        };
        let artifact = self
            .manifest
            .artifacts
            .iter()
            .find(|a| a.id == citation.artifact_id)
            .ok_or_else(|| ReviewFailure("citation names no captured artifact".into()))?;
        let unanchored = |reason: &str| TuicrExportError::Unanchored {
            finding: finding.id.clone(),
            path: path.into(),
            reason: reason.into(),
        };
        let sealed =
            storage::read_bytes(&self.directory.join(SNAPSHOT_DIR).join(path), SOURCE_CAP)?;
        if sha256_hex(&sealed) != artifact.sha256 {
            return Err(ReviewFailure(
                "sealed snapshot copy differs from the manifest artifact".into(),
            )
            .into());
        }
        if !self.session.files.contains_key(path) {
            return Err(unanchored("the session does not list this file"));
        }
        let reviewed = self.session_bytes(path).map_err(|e| unanchored(&e))?;
        if reviewed != sealed {
            return Err(unanchored(&format!(
                "the {} bytes the session reviews differ from the sealed snapshot",
                self.revision.label()
            )));
        }
        anchor(
            &finding.id,
            path,
            citation.start_line,
            citation.end_line,
            line_count(&reviewed),
        )
    }
}

fn header(run_id: &str, admitted: &Admitted, item: &str, severity: &str) -> String {
    let requested = admitted
        .reviewer
        .model
        .as_deref()
        .unwrap_or("provider default");
    format!(
        "[DDF-COUNCIL run={run_id} reviewer={} round={} {item}]\nseverity={severity} | observed model: {} (requested: {requested}). {NOT_AUTHORITY}\nProvenance: deep-diff-forge council run {run_id}; admitted {} sha256={}; provider {:?}, persona {:?}.\n",
        admitted.report.reviewer_id,
        admitted.report.round.label(),
        admitted.model,
        admitted.file,
        admitted.sha256,
        admitted.reviewer.provider,
        admitted.reviewer.persona,
    )
}
fn severity_label(severity: Severity) -> &'static str {
    match severity {
        Severity::Low => "LOW",
        Severity::Medium => "MEDIUM",
        Severity::High => "HIGH",
        Severity::Critical => "CRITICAL",
    }
}
/// Note text for one finding: provenance header, quoted claim, falsifier, citations and anchor basis.
fn finding_content(
    binding: &Binding<'_>,
    a: &Admitted,
    finding: &crate::Finding,
    target: &Target,
    downgraded: Option<&str>,
) -> String {
    let citations = finding
        .citations
        .iter()
        .map(|c| format!("- {}:{}-{}", c.artifact_id, c.start_line, c.end_line))
        .collect::<Vec<_>>()
        .join("\n");
    let anchor = match (target, downgraded) {
        (_, Some(reason)) => format!("Anchor: none; downgraded to review level ({reason})"),
        (Target::Review, None) => "Anchor: none; no candidate-file citation".into(),
        _ => format!(
            "Anchor: new side; session {} bytes equal the sealed snapshot",
            binding.revision.label()
        ),
    };
    let checks = finding
        .requested_checks
        .iter()
        .map(|g| g.label())
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "{}\n{}\n\nFalsifier: {}\nRequested checks (advisory): {}\nCitations:\n{citations}\n{anchor}",
        header(
            &binding.manifest.run_id,
            a,
            &format!("finding={}", finding.id),
            severity_label(finding.severity)
        ),
        finding.claim.trim(),
        finding.falsifier.trim(),
        if checks.is_empty() { "none" } else { &checks },
    )
}
fn plan(
    binding: &Binding<'_>,
    admitted: &[Admitted],
    options: ExportOptions,
) -> Result<Vec<PlannedNote>> {
    let run_id = &binding.manifest.run_id;
    let mut notes = Vec::new();
    for a in admitted {
        let report = &a.report;
        let prefix = format!("{}/{}", report.reviewer_id, report.round.label());
        let author = format!("council:{} ({})", report.reviewer_id, a.model);
        let note = |key: String, finding: Option<&crate::Finding>, target, downgraded, content| {
            PlannedNote {
                key,
                reviewer_id: report.reviewer_id.clone(),
                round: report.round,
                finding_id: finding.map(|f| f.id.clone()),
                severity: finding.map(|f| f.severity),
                observed_model: a.model.clone(),
                author: author.clone(),
                target,
                downgraded,
                status: NoteStatus::Planned,
                tuicr_comment_id: None,
                content,
            }
        };
        notes.push(note(
            format!("{prefix}/summary"),
            None,
            Target::Review,
            None,
            format!(
                "{}\nverdict={:?}, {} finding(s). NoBlockers is not approval.\n\n{}",
                header(run_id, a, "finding=(report summary)", "n/a"),
                report.verdict,
                report.findings.len(),
                report.summary.trim()
            ),
        ));
        for finding in &report.findings {
            let (target, downgraded) = match binding.target(finding) {
                Ok(target) => (target, None),
                Err(
                    e @ (TuicrExportError::Unanchored { .. }
                    | TuicrExportError::AnchorOutOfRange { .. }),
                ) if options.allow_review_level => (Target::Review, Some(e.to_string())),
                Err(e) => return Err(e),
            };
            let content = finding_content(binding, a, finding, &target, downgraded.as_deref());
            notes.push(note(
                format!("{prefix}/finding/{}", finding.id),
                Some(finding),
                target,
                downgraded,
                content,
            ));
        }
        if !report.responses.is_empty() {
            let lines = report
                .responses
                .iter()
                .map(|r| {
                    format!(
                        "- {}: {}: {}",
                        r.finding_id,
                        r.disposition.label(),
                        r.rationale.trim()
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            notes.push(note(
                format!("{prefix}/responses"),
                None,
                Target::Review,
                None,
                format!(
                    "{}\nchallenge responses:\n{lines}",
                    header(run_id, a, "finding=(challenge responses)", "n/a")
                ),
            ));
        }
    }
    Ok(notes)
}

/// Locate `tuicr` on `PATH`; only absolute, executable regular files qualify.
#[expect(
    clippy::disallowed_methods,
    reason = "the tuicr door: locates the binary on PATH"
)]
fn tuicr_binary() -> Result<PathBuf> {
    let path = std::env::var_os("PATH")
        .ok_or_else(|| TuicrExportError::TuicrUnavailable("PATH is unset".into()))?;
    std::env::split_paths(&path)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join("tuicr"))
        .find(|candidate| {
            fs::metadata(candidate)
                .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        })
        .ok_or_else(|| TuicrExportError::TuicrUnavailable("no executable tuicr on PATH".into()))
}
/// Run tuicr with fixed argv (no shell), feeding `input` on stdin; returns stdout.
#[expect(
    clippy::disallowed_methods,
    reason = "the tuicr door: spawns the review TUI"
)]
fn tuicr(binary: &Path, cwd: &Path, args: &[&str], input: &[u8]) -> Result<Vec<u8>> {
    let mut child = Command::new(binary)
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| TuicrExportError::TuicrUnavailable(e.to_string()))?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| TuicrExportError::TuicrUnavailable("missing stdin pipe".into()))?;
    // Write on a separate thread so a large payload cannot deadlock against a full stdout pipe.
    let output = std::thread::scope(|scope| {
        let writer = scope.spawn(move || stdin.write_all(input));
        let output = child.wait_with_output();
        let _ = writer.join();
        output
    })?;
    if !output.status.success() {
        return Err(TuicrExportError::TuicrFailed {
            status: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }
    Ok(output.stdout)
}
/// The session must be tuicr's stored copy for this repository (it may be open in a live tuicr).
fn preflight(binary: &Path, repository: &Path, session: &Path) -> Result<()> {
    #[derive(Deserialize)]
    struct Listed {
        path: PathBuf,
    }
    let repo = repository
        .to_str()
        .ok_or_else(|| ReviewFailure("non-UTF-8 repository path".into()))?;
    let listed: Vec<Listed> = serde_json::from_slice(&tuicr(
        binary,
        repository,
        &["review", "list", "--repo", repo],
        b"",
    )?)
    .map_err(|e| TuicrExportError::TuicrOutput(format!("review list: {e}")))?;
    listed
        .iter()
        .find(|l| fs::canonicalize(&l.path).is_ok_and(|p| p == session))
        .ok_or_else(|| TuicrExportError::NotInStore {
            session: session.display().to_string(),
        })?;
    // An active session is fine: tuicr's CLI writes are lock-protected and a running TUI polls
    // and merges external edits (tuicr README, "Review session CLI"). The per-add re-read below
    // still refuses if the session diverges.
    Ok(())
}
fn load_session(path: &Path) -> Result<Session> {
    serde_json::from_slice(&storage::read_bytes(path, SESSION_CAP)?)
        .map_err(|e| TuicrExportError::UnsupportedSession(e.to_string()))
}
/// A session file that passed the identity checks shared by export and import.
pub(crate) struct OpenSession {
    /// Canonical session path.
    pub(crate) path: PathBuf,
    /// The bounded bytes the session was parsed from, so a caller can parse richer views of the same read.
    pub(crate) bytes: Vec<u8>,
    pub(crate) session: Session,
    pub(crate) revision: Revision,
    /// Canonical run repository, equal to the session's `repo_path`.
    pub(crate) repository: PathBuf,
}
/// Read a bounded session and refuse unsupported versions, forge sessions, unknown diff sources,
/// invalid ids and a repository other than the run's.
pub(crate) fn open_session(manifest: &Manifest, session_path: &Path) -> Result<OpenSession> {
    let path = fs::canonicalize(session_path)?;
    let bytes = storage::read_bytes(&path, SESSION_CAP)?;
    let session: Session = serde_json::from_slice(&bytes)
        .map_err(|e| TuicrExportError::UnsupportedSession(e.to_string()))?;
    if session.version != SESSION_VERSION {
        return Err(TuicrExportError::UnsupportedSession(format!(
            "session version {:?}; this adapter supports {SESSION_VERSION}",
            session.version
        )));
    }
    if session.pr_session_key.is_some() {
        return Err(TuicrExportError::ForgeSession);
    }
    let revision = revision(&session)?;
    if !crate::identifier(&session.id) {
        return Err(TuicrExportError::UnsupportedSession(
            "invalid session id".into(),
        ));
    }
    let repository = fs::canonicalize(&manifest.request.repository)?;
    if fs::canonicalize(&session.repo_path).ok().as_deref() != Some(repository.as_path()) {
        return Err(TuicrExportError::RepositoryMismatch {
            session: session.repo_path.display().to_string(),
            run: repository.display().to_string(),
        });
    }
    Ok(OpenSession {
        path,
        bytes,
        session,
        revision,
        repository,
    })
}
/// tuicr `review add --input` JSON payload for one note.
fn payload(note: &PlannedNote) -> serde_json::Value {
    serde_json::json!({
        "type": "note",
        "username": note.author,
        "content": note.content,
        "target": note.target,
    })
}
/// Check tuicr's reply describes the requested local draft and return its comment id.
fn created_id(stdout: &[u8], note: &PlannedNote) -> Result<String> {
    let value: serde_json::Value = serde_json::from_slice(stdout)
        .map_err(|e| TuicrExportError::TuicrOutput(format!("review add: {e}")))?;
    let id = value["id"]
        .as_str()
        .filter(|id| crate::identifier(id))
        .ok_or_else(|| TuicrExportError::TuicrOutput("missing or invalid comment id".into()))?;
    let (path, start, end) = match &note.target {
        Target::Review => (None, None, None),
        Target::Line { file, line, .. } => (Some(file.as_str()), Some(*line), Some(*line)),
        Target::LineRange {
            file,
            start_line,
            end_line,
            ..
        } => (Some(file.as_str()), Some(*start_line), Some(*end_line)),
    };
    let side = path.map(|_| "new");
    let line = |v: &serde_json::Value| v.as_u64().and_then(|n| usize::try_from(n).ok());
    if value["lifecycle_state"] != "local_draft"
        || value["path"].as_str() != path
        || line(&value["start_line"]) != start
        || line(&value["end_line"]) != end
        || value["side"].as_str() != side
    {
        return Err(TuicrExportError::TuicrOutput(format!(
            "comment {id} is not the requested new-side local draft"
        )));
    }
    Ok(id.to_owned())
}

/// One ledger record; written create-new beside its intent marker.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct LedgerEntry {
    pub(crate) key: String,
    pub(crate) run_id: String,
    pub(crate) session_id: String,
    pub(crate) tuicr_comment_id: String,
    pub(crate) content_sha256: String,
    recovered: bool,
}

/// Ledger file locations and identity for one (run, session) pair.
struct Ledger<'a> {
    dir: PathBuf,
    run_id: &'a str,
    session_id: &'a str,
}
impl Ledger<'_> {
    fn done(&self, key: &str) -> PathBuf {
        self.dir
            .join(format!("{}.json", sha256_hex(key.as_bytes())))
    }
    fn pending(&self, key: &str) -> PathBuf {
        self.dir
            .join(format!("{}.pending", sha256_hex(key.as_bytes())))
    }
}
/// Write every still-planned note through tuicr, guarded by create-new intent markers and ledger records.
fn execute(
    directory: &Path,
    repository: &Path,
    session_path: &Path,
    ledger: &Ledger<'_>,
    notes: &mut [PlannedNote],
) -> Result<()> {
    let binary = tuicr_binary()?;
    preflight(&binary, repository, session_path)?;
    let mut dir = directory.to_path_buf();
    for part in [LEDGER_DIR, "tuicr", ledger.session_id] {
        dir.push(part);
        if !dir.exists() {
            storage::make_dir(&dir)?;
        }
    }
    let session_arg = session_path
        .to_str()
        .ok_or_else(|| ReviewFailure("non-UTF-8 session path".into()))?;
    for note in notes.iter_mut().filter(|n| n.status == NoteStatus::Planned) {
        let marker = ledger.pending(&note.key);
        let find = |content: &str| -> Result<Option<String>> {
            Ok(load_session(session_path)?
                .comments()
                .into_iter()
                .find(|(_, c)| c == content)
                .map(|(id, _)| id))
        };
        let (id, recovered) = if marker.exists() {
            // A previous run stopped between intent and ledger: adopt the comment if it landed.
            let id = find(&note.content)?.ok_or_else(|| TuicrExportError::InterruptedExport {
                key: note.key.clone(),
                pending: marker.display().to_string(),
            })?;
            (id, true)
        } else {
            storage::write_new(&marker, note.key.as_bytes())?;
            let input = serde_json::to_vec(&payload(note))?;
            let created = tuicr(
                &binary,
                repository,
                &["review", "add", "--session", session_arg, "--input", "-"],
                &input,
            )
            .and_then(|stdout| created_id(&stdout, note));
            match created {
                Ok(id) => (id, false),
                Err(error) => {
                    // Remove the intent only once the session provably lacks the comment.
                    if find(&note.content)?.is_none() {
                        fs::remove_file(&marker)?;
                    }
                    return Err(error);
                }
            }
        };
        if !load_session(session_path)?
            .comments()
            .iter()
            .any(|(cid, c)| *cid == id && *c == note.content)
        {
            return Err(TuicrExportError::SessionDiverged { comment_id: id });
        }
        storage::write_json(
            &ledger.done(&note.key),
            &LedgerEntry {
                key: note.key.clone(),
                run_id: ledger.run_id.to_owned(),
                session_id: ledger.session_id.to_owned(),
                tuicr_comment_id: id.clone(),
                content_sha256: sha256_hex(note.content.as_bytes()),
                recovered,
            },
        )?;
        note.status = if recovered {
            NoteStatus::Recovered
        } else {
            NoteStatus::Created
        };
        note.tuicr_comment_id = Some(id);
    }
    Ok(())
}

/// Export admitted council reports from `directory` into the tuicr session file `session_path`.
///
/// Without `options.execute` this is a local dry run: it performs every identity check, reports the
/// planned notes and the ledger state, and neither writes nor invokes tuicr.
/// # Errors
/// Refuses unadmitted or tampered reports, forge or unknown sessions, a different repository,
/// unverifiable anchors (unless downgraded), sessions tuicr does not store or has open, tuicr
/// failures and interrupted earlier exports.
pub fn export_tuicr(
    directory: &Path,
    session_path: &Path,
    options: ExportOptions,
) -> Result<TuicrExport> {
    let directory = fs::canonicalize(directory)?;
    let manifest = load_manifest(&directory)?;
    let (admitted, skipped_reports) = admitted_reports(&directory, &manifest)?;
    let OpenSession {
        path: session_path,
        session,
        revision,
        repository,
        ..
    } = open_session(&manifest, session_path)?;
    let binding = Binding {
        directory: &directory,
        manifest: &manifest,
        repository: repository.clone(),
        session: &session,
        revision: revision.clone(),
    };
    let mut notes = plan(&binding, &admitted, options)?;
    let ledger = Ledger {
        dir: directory.join(LEDGER_DIR).join("tuicr").join(&session.id),
        run_id: &manifest.run_id,
        session_id: &session.id,
    };
    for note in &mut notes {
        let done = ledger.done(&note.key);
        if done.exists() {
            let entry: LedgerEntry = storage::read_json(&done, storage::REPORT_CAP)?;
            note.status = NoteStatus::AlreadyExported;
            note.tuicr_comment_id = Some(entry.tuicr_comment_id);
        }
    }
    if options.execute {
        execute(&directory, &repository, &session_path, &ledger, &mut notes)?;
    }
    Ok(TuicrExport {
        schema: "deep-diff-forge.tuicr-export.v1",
        run_id: manifest.run_id.clone(),
        session: session_path.display().to_string(),
        session_id: session.id.clone(),
        diff_source: session
            .diff_source
            .clone()
            .unwrap_or_else(|| "working_tree".into()),
        session_revision: revision.label(),
        executed: options.execute,
        ledger: ledger.dir.display().to_string(),
        authority: "local tuicr draft notes only; no forge submission, review marks, resolution or approval",
        skipped_reports,
        notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_count_matches_str_lines() {
        for text in ["", "a", "a\n", "a\nb", "a\nb\n", "\n", "\n\n", "a\r\nb\r\n"] {
            assert_eq!(
                line_count(text.as_bytes()),
                text.lines().count(),
                "{text:?}"
            );
        }
    }

    #[test]
    fn anchors_stay_inside_the_reviewed_file_on_the_new_side() {
        assert_eq!(
            anchor("f", "x.rs", 2, 2, 4).unwrap(),
            Target::Line {
                file: "x.rs".into(),
                line: 2,
                side: "new"
            }
        );
        assert_eq!(
            anchor("f", "x.rs", 2, 4, 4).unwrap(),
            Target::LineRange {
                file: "x.rs".into(),
                start_line: 2,
                end_line: 4,
                side: "new"
            }
        );
        for (start, end) in [(0, 1), (3, 2), (4, 5), (5, 5)] {
            assert!(matches!(
                anchor("f", "x.rs", start, end, 4),
                Err(TuicrExportError::AnchorOutOfRange { lines: 4, .. })
            ));
        }
    }

    #[test]
    fn observed_model_reads_provider_transcripts_and_rejects_noise() {
        assert_eq!(
            observed_model(
                Provider::ClaudeCode,
                br#"{"modelUsage":{"claude-opus-x":{},"claude-haiku-y":{}}}"#,
                b""
            ),
            "claude-haiku-y,claude-opus-x"
        );
        assert_eq!(
            observed_model(Provider::Codex, b"", b"workdir: /w\nmodel: gpt-5.5-codex\n"),
            "gpt-5.5-codex"
        );
        assert_eq!(
            observed_model(Provider::Codex, b"", b"model: a b\n"),
            "unknown"
        );
        assert_eq!(
            observed_model(Provider::ClaudeCode, b"not json", b""),
            "unknown"
        );
    }

    #[test]
    fn diff_sources_map_to_the_new_side_revision() {
        let sha = "a".repeat(40);
        let session = |source: &str, range: Option<Vec<String>>| Session {
            id: "s".into(),
            version: SESSION_VERSION.into(),
            repo_path: PathBuf::from("/r"),
            base_commit: sha.clone(),
            diff_source: Some(source.into()),
            commit_range: range,
            pr_session_key: None,
            files: BTreeMap::new(),
            review_comments: vec![],
        };
        assert_eq!(
            revision(&session("staged_and_unstaged", None)).unwrap(),
            Revision::WorkingTree
        );
        assert_eq!(revision(&session("staged", None)).unwrap(), Revision::Index);
        assert_eq!(
            revision(&session(
                "commit_range",
                Some(vec!["b".repeat(40), sha.clone()])
            ))
            .unwrap(),
            Revision::Commit(sha.clone())
        );
        assert!(matches!(
            revision(&session(
                "commit_range",
                Some(vec![sha.clone(), "b".repeat(40)])
            )),
            Err(TuicrExportError::UnsupportedSession(_))
        ));
        assert!(matches!(
            revision(&session("pull_request", None)),
            Err(TuicrExportError::ForgeSession)
        ));
        assert!(matches!(
            revision(&session("something_new", None)),
            Err(TuicrExportError::UnsupportedSession(_))
        ));
    }
}
