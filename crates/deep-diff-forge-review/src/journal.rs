// Review council: Append-only, hash-chained decision journal fed by human dispositions recorded in a local tuicr session.
// Design/status: docs/REVIEW_COUNCIL.md#decision-journal
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Comparisons%2FArchitecture%20Lessons%20and%20Interoperability
//! tuicr→DDF decision journal (DDF-R18). Human comments, revisions, lifecycle transitions, deletions
//! and reviewed marks in a local tuicr session become actor-attributed events in `<run>/decisions.jsonl`.
//!
//! The journal is append-only and hash-chained: each event names the hash of the one before it, and
//! create-new checkpoints record the head after every import, so a rewritten or truncated journal
//! is refused on the next import. Nothing here writes to tuicr. Session content is untrusted data:
//! it is size-capped, its ids and paths are validated, and a disposition is recorded only while the
//! session still shows the revision and bytes the run sealed; otherwise the whole import is refused.
use crate::storage::{self, INVENTORY_FILE, SNAPSHOT_DIR};
use crate::tuicr::{self, Binding, LEDGER_DIR, LedgerEntry, OpenSession, SOURCE_CAP};
use crate::{Manifest, ReviewFailure, TuicrExportError, load_manifest};
use deep_diff_forge_core::sha256_hex;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write as _;
use std::os::unix::fs::OpenOptionsExt;
use std::{
    error::Error,
    fmt,
    fs::{self, OpenOptions},
    path::{Component, Path, PathBuf},
};

/// Journal file in the run directory: one JSON event per line, appended, never rewritten.
pub const JOURNAL_FILE: &str = "decisions.jsonl";
/// Create-new `<seq>.json` records of the journal head after each import that appended events.
const CHECKPOINT_DIR: &str = "decisions.checkpoints";
const LOCK_FILE: &str = "decisions.lock";
const EVENT_SCHEMA: &str = "deep-diff-forge.decision.v1";
const JOURNAL_CAP: u64 = 64 * 1024 * 1024;
const ENTRY_CAP: usize = 100_000;
const TAG_CAP: usize = 256;

/// Typed refusal of a tuicr import. Every variant is a refusal to append.
#[derive(Debug)]
#[non_exhaustive]
pub enum TuicrImportError {
    /// The run, the session identity or the export ledger failed a check shared with the export.
    Session(TuicrExportError),
    /// The session's base revision is not the revision the run sealed.
    StaleRevision { session: String, run: String },
    /// A disposition names a file whose reviewed bytes differ from the run's sealed snapshot.
    StaleFile { path: String, reason: String },
    /// A reviewed mark carries no tuicr content hash, or one other than the hash already journaled.
    StaleReviewMark { path: String, reason: String },
    /// Session content that cannot be journaled safely (invalid ids or paths, duplicates, unknown states).
    UnsafeSession(String),
    /// The journal or its checkpoints are malformed, rewritten or truncated.
    Journal(String),
    /// Another import holds the journal lock, or an earlier one was interrupted.
    Locked { lock: String },
    /// An export intent marker has no completion record, so DDF authorship is unknown.
    InterruptedExport { pending: String },
}
impl fmt::Display for TuicrImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Session(e) => write!(f, "tuicr import refused: {e}"),
            Self::StaleRevision { session, run } => write!(
                f,
                "tuicr import refused (stale revision): session base_commit {session} is not the run's sealed HEAD {run}"
            ),
            Self::StaleFile { path, reason } => write!(
                f,
                "tuicr import refused (stale file): {path}: {reason}; the disposition is not on the run's revision"
            ),
            Self::StaleReviewMark { path, reason } => {
                write!(
                    f,
                    "tuicr import refused (stale review mark): {path}: {reason}"
                )
            }
            Self::UnsafeSession(reason) => {
                write!(f, "tuicr import refused (unsafe session): {reason}")
            }
            Self::Journal(reason) => write!(f, "tuicr import refused (journal): {reason}"),
            Self::Locked { lock } => write!(
                f,
                "tuicr import refused (locked): {lock} exists; another import is running or was interrupted; inspect the journal, then remove the lock"
            ),
            Self::InterruptedExport { pending } => write!(
                f,
                "tuicr import refused (interrupted export): {pending} has no completion record; finish or inspect the export first"
            ),
        }
    }
}
impl Error for TuicrImportError {}
impl From<TuicrExportError> for TuicrImportError {
    fn from(e: TuicrExportError) -> Self {
        Self::Session(e)
    }
}
impl From<ReviewFailure> for TuicrImportError {
    fn from(e: ReviewFailure) -> Self {
        Self::Session(e.into())
    }
}
impl From<std::io::Error> for TuicrImportError {
    fn from(e: std::io::Error) -> Self {
        Self::Session(e.into())
    }
}
impl From<serde_json::Error> for TuicrImportError {
    fn from(e: serde_json::Error) -> Self {
        Self::Session(e.into())
    }
}
impl From<TuicrImportError> for ReviewFailure {
    fn from(e: TuicrImportError) -> Self {
        Self(e.to_string())
    }
}
type Result<T> = std::result::Result<T, TuicrImportError>;

/// What happened, as observed by comparing the session with the journal and the export ledger.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    /// A comment not authored by this run's export, linked to no council finding.
    CommentAdded,
    /// A comment not authored by this run's export, linked to at least one council finding.
    Disposition,
    /// A journaled or exported comment whose content changed.
    Revised,
    /// A tombstoned comment that is present again.
    Restored,
    /// A journaled or exported comment whose tuicr lifecycle state changed.
    Lifecycle,
    /// Tombstone: a journaled or exported comment is no longer in the session.
    Deleted,
    /// A file's reviewed mark or reviewed hunks changed.
    FileReview,
}

/// Why an actor name is attached to an event. tuicr authors are free-form, unauthenticated strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorBasis {
    /// The comment's tuicr `author` field. tuicr records no separate editor, so edits of a
    /// non-council comment are attributed to its author.
    TuicrAuthor,
    /// tuicr records nobody for this change: a reviewed mark, or a change to a council-authored
    /// comment (the council never edits after export, so the change came from outside DDF).
    Unrecorded,
}
/// Who an event is attributed to, and on what basis.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Actor {
    pub name: String,
    pub basis: ActorBasis,
}

/// tuicr `CommentLifecycleState` (tuicr 0.27.0 `src/model/comment.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lifecycle {
    LocalDraft,
    PushedDraft,
    Submitted,
}

/// Where a comment sits in the session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Anchor {
    Review,
    File {
        file: String,
    },
    Lines {
        file: String,
        side: String,
        start_line: u32,
        end_line: u32,
    },
}
impl Anchor {
    fn file(&self) -> Option<&str> {
        match self {
            Self::Review => None,
            Self::File { file } | Self::Lines { file, .. } => Some(file),
        }
    }
    /// Same file, same side and intersecting inclusive line ranges.
    fn overlaps(&self, other: &Self) -> bool {
        match (self, other) {
            (
                Self::Lines {
                    file,
                    side,
                    start_line,
                    end_line,
                },
                Self::Lines {
                    file: f,
                    side: s,
                    start_line: a,
                    end_line: b,
                },
            ) => file == f && side == s && start_line <= b && a <= end_line,
            _ => false,
        }
    }
}

/// How a human comment was linked to an exported council finding note.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkBasis {
    /// The comment text names the finding (`[ddf:<key>]` or the note's `[DDF-COUNCIL ...]` header).
    Tag,
    /// No tag; the comment's line range overlaps the finding note's on the same file and side.
    Anchor,
}
/// A council finding a human comment answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Link {
    /// Export ledger key, `<reviewer>/<round>/finding/<finding-id>`.
    pub key: String,
    pub tuicr_comment_id: String,
    pub basis: LinkBasis,
}

/// The session revision an event was observed on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionRevision {
    pub diff_source: String,
    pub base_commit: String,
    /// Which bytes the session's new side shows (`working_tree`, `index` or `commit:<sha>`).
    pub new_side: String,
}

/// One journal line. `hash` is SHA-256 over the line's JSON with `hash` set to the empty string.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionEvent {
    pub schema: String,
    pub seq: u64,
    /// Hash of the previous event; 64 zeros for the first.
    pub prev: String,
    pub run_id: String,
    pub session_id: String,
    pub revision: SessionRevision,
    pub kind: EventKind,
    pub actor: Actor,
    pub comment_id: Option<String>,
    /// The export ledger records this comment as written by this run's council export.
    pub ddf_authored: bool,
    pub comment_type: Option<String>,
    pub lifecycle_state: Option<Lifecycle>,
    pub previous_lifecycle_state: Option<Lifecycle>,
    pub anchor: Option<Anchor>,
    /// tuicr `FileReview.content_hash` (FNV-1a over the file's diff hunks) for anchored events.
    pub content_hash: Option<u64>,
    pub content_sha256: Option<String>,
    /// Untrusted comment text, verbatim.
    pub content: Option<String>,
    pub links: Vec<Link>,
    pub reviewed: Option<bool>,
    pub reviewed_hunks: Option<Vec<String>>,
    /// Why this event is a conflict between a DDF-authored record and a human edit.
    pub conflict: Option<String>,
    /// Hash of the latest earlier event about the same comment or file, if any.
    pub supersedes: Option<String>,
    pub hash: String,
}
impl DecisionEvent {
    fn digest(&self) -> Result<String> {
        let mut unsealed = self.clone();
        unsealed.hash.clear();
        Ok(sha256_hex(&serde_json::to_vec(&unsealed)?))
    }
}

/// Caller choices for [`import_tuicr`].
#[derive(Debug, Clone, Copy, Default)]
pub struct ImportOptions {
    /// Append the planned events; without it the import is a pure dry run.
    pub execute: bool,
}

/// Machine-readable import result.
#[derive(Debug, Clone, Serialize)]
pub struct TuicrImport {
    pub schema: &'static str,
    pub run_id: String,
    pub session: String,
    pub session_id: String,
    pub revision: SessionRevision,
    pub executed: bool,
    pub journal: String,
    /// Events already in the journal before this import.
    pub journal_events: u64,
    /// Planned (dry run) or appended (`--execute`) events.
    pub events: Vec<DecisionEvent>,
    /// Number of `events` that are conflicts.
    pub conflicts: usize,
    pub authority: &'static str,
}

/// Comment fields of a tuicr 1.3 session (`Comment` in tuicr 0.27.0); other fields are ignored.
#[derive(Debug, Deserialize)]
struct RawComment {
    id: String,
    content: String,
    #[serde(default)]
    comment_type: Option<String>,
    #[serde(default = "default_author")]
    author: String,
    #[serde(default)]
    lifecycle_state: Option<Lifecycle>,
    #[serde(default)]
    side: Option<String>,
    #[serde(default)]
    line_range: Option<RawRange>,
}
fn default_author() -> String {
    // tuicr's `DEFAULT_AUTHOR`, used when the field is absent.
    "user".into()
}
#[derive(Debug, Deserialize)]
struct RawRange {
    start: u32,
    end: u32,
}
/// `FileReview` fields this importer reads.
#[derive(Debug, Deserialize)]
struct RawFile {
    #[serde(default)]
    reviewed: bool,
    #[serde(default)]
    file_comments: Vec<RawComment>,
    #[serde(default)]
    line_comments: BTreeMap<String, Vec<RawComment>>,
    #[serde(default)]
    reviewed_hunks: BTreeSet<String>,
    #[serde(default)]
    content_hash: Option<u64>,
}
#[derive(Debug, Deserialize)]
struct RawSession {
    #[serde(default)]
    review_comments: Vec<RawComment>,
    files: BTreeMap<String, RawFile>,
}

/// One comment as observed in the session, with its anchor and its file's tuicr content hash.
#[derive(Debug, Clone)]
struct Observed {
    id: String,
    content: String,
    sha256: String,
    comment_type: Option<String>,
    author: String,
    lifecycle: Lifecycle,
    anchor: Anchor,
    content_hash: Option<u64>,
}
/// A session-relative path that can only name a file inside the repository or snapshot.
fn safe_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 4096
        && Path::new(path)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}
fn observe(raw: RawComment, anchor: Anchor, content_hash: Option<u64>) -> Result<Observed> {
    if !crate::identifier(&raw.id) {
        return Err(TuicrImportError::UnsafeSession(
            "a comment id is not a short identifier".into(),
        ));
    }
    Ok(Observed {
        sha256: sha256_hex(raw.content.as_bytes()),
        id: raw.id,
        content: raw.content,
        comment_type: raw.comment_type,
        author: raw.author,
        lifecycle: raw.lifecycle_state.unwrap_or(Lifecycle::LocalDraft),
        anchor,
        content_hash,
    })
}
/// Every comment in a deterministic order: review level, then per file (sorted) file level, then
/// line comments by line. Duplicate ids and unsafe paths are refused.
fn observed_comments(raw: RawSession) -> Result<(Vec<Observed>, BTreeMap<String, RawFile>)> {
    let mut out = Vec::new();
    for c in raw.review_comments {
        out.push(observe(c, Anchor::Review, None)?);
    }
    let mut files = raw.files;
    for (path, file) in &mut files {
        if !safe_path(path) {
            return Err(TuicrImportError::UnsafeSession(format!(
                "file key {path:?} is not a relative path inside the repository"
            )));
        }
        for c in std::mem::take(&mut file.file_comments) {
            out.push(observe(
                c,
                Anchor::File { file: path.clone() },
                file.content_hash,
            )?);
        }
        let mut lines = Vec::new();
        for (key, list) in std::mem::take(&mut file.line_comments) {
            let line: u32 = key.parse().map_err(|_| {
                TuicrImportError::UnsafeSession(format!("line key {key:?} in {path} is not a line"))
            })?;
            lines.push((line, list));
        }
        lines.sort_by_key(|(line, _)| *line);
        for (line, list) in lines {
            for c in list {
                let (start, end) = c
                    .line_range
                    .as_ref()
                    .map_or((line, line), |r| (r.start.min(r.end), r.start.max(r.end)));
                let side = match c.side.as_deref() {
                    None | Some("new") => "new",
                    Some("old") => "old",
                    Some(other) => {
                        return Err(TuicrImportError::UnsafeSession(format!(
                            "unknown comment side {other:?}"
                        )));
                    }
                };
                let anchor = Anchor::Lines {
                    file: path.clone(),
                    side: side.into(),
                    start_line: start,
                    end_line: end,
                };
                out.push(observe(c, anchor, file.content_hash)?);
            }
        }
    }
    let mut seen = BTreeSet::new();
    if let Some(dup) = out.iter().find(|c| !seen.insert(c.id.as_str())) {
        return Err(TuicrImportError::UnsafeSession(format!(
            "comment id {} appears more than once",
            dup.id
        )));
    }
    Ok((out, files))
}

/// Comments this run's export wrote into the session, keyed by tuicr comment id.
struct Ledger {
    by_comment: BTreeMap<String, LedgerEntry>,
}
impl Ledger {
    fn load(directory: &Path, run_id: &str, session_id: &str) -> Result<Self> {
        let dir = directory
            .join(LEDGER_DIR)
            .join("tuicr")
            .join(crate::segment(session_id)?);
        let mut by_comment = BTreeMap::new();
        if !dir.exists() {
            return Ok(Self { by_comment });
        }
        let mut names = BTreeSet::new();
        for (i, entry) in fs::read_dir(&dir)?.enumerate() {
            if i >= ENTRY_CAP {
                return Err(ReviewFailure("export ledger has too many entries".into()).into());
            }
            names.insert(entry?.file_name().to_string_lossy().into_owned());
        }
        for name in &names {
            if let Some(stem) = name.strip_suffix(".pending") {
                if !names.contains(&format!("{stem}.json")) {
                    return Err(TuicrImportError::InterruptedExport {
                        pending: dir.join(name).display().to_string(),
                    });
                }
            } else if Path::new(name).extension().is_some_and(|e| e == "json") {
                let entry: LedgerEntry = storage::read_json(&dir.join(name), storage::REPORT_CAP)?;
                if entry.run_id != run_id
                    || entry.session_id != session_id
                    || format!("{}.json", sha256_hex(entry.key.as_bytes())) != *name
                {
                    return Err(ReviewFailure(format!(
                        "export ledger record {name} does not belong to this run and session"
                    ))
                    .into());
                }
                by_comment.insert(entry.tuicr_comment_id.clone(), entry);
            }
        }
        Ok(Self { by_comment })
    }
    /// Finding notes only: summaries and challenge responses are not findings.
    fn finding_by_key(&self, key: &str) -> Option<&LedgerEntry> {
        self.by_comment
            .values()
            .find(|e| e.key == key && e.key.contains("/finding/"))
    }
}

/// Finding keys a comment names: `[ddf:<key>]`, or a quoted `[DDF-COUNCIL run=<this run> reviewer=<r>
/// round=<round> finding=<id>]` header. Tags are bounded; malformed ones are ignored.
fn tagged_keys(content: &str, run_id: &str) -> BTreeSet<String> {
    let mut keys = BTreeSet::new();
    let bracketed = |start: usize| -> Option<&str> {
        let rest = content.get(start..)?;
        let end = rest.find(']').filter(|end| *end <= TAG_CAP)?;
        Some(&rest[..end])
    };
    for (at, _) in content.match_indices("[ddf:") {
        if let Some(key) = bracketed(at + "[ddf:".len()) {
            keys.insert(key.trim().to_owned());
        }
    }
    for (at, _) in content.match_indices("[DDF-COUNCIL ") {
        let Some(body) = bracketed(at + "[DDF-COUNCIL ".len()) else {
            continue;
        };
        let field = |name: &str| {
            body.split_whitespace()
                .find_map(|f| f.strip_prefix(name)?.strip_prefix('='))
        };
        if let (Some(run), Some(reviewer), Some(round), Some(finding)) = (
            field("run"),
            field("reviewer"),
            field("round"),
            field("finding"),
        ) && run == run_id
        {
            keys.insert(format!("{reviewer}/{round}/finding/{finding}"));
        }
    }
    keys
}
/// Linking rule: explicit tags naming exported finding notes win; with no such tag, every exported
/// finding note whose line anchor overlaps the comment's (same file, same side) is linked.
fn links(comment: &Observed, run_id: &str, ledger: &Ledger, notes: &[&Observed]) -> Vec<Link> {
    let tagged: Vec<Link> = tagged_keys(&comment.content, run_id)
        .iter()
        .filter_map(|key| ledger.finding_by_key(key))
        .map(|e| Link {
            key: e.key.clone(),
            tuicr_comment_id: e.tuicr_comment_id.clone(),
            basis: LinkBasis::Tag,
        })
        .collect();
    if !tagged.is_empty() {
        return tagged;
    }
    let mut anchored: Vec<Link> = notes
        .iter()
        .filter(|note| note.anchor.overlaps(&comment.anchor))
        .filter_map(|note| ledger.by_comment.get(&note.id))
        .filter(|e| e.key.contains("/finding/"))
        .map(|e| Link {
            key: e.key.clone(),
            tuicr_comment_id: e.tuicr_comment_id.clone(),
            basis: LinkBasis::Anchor,
        })
        .collect();
    anchored.sort_by(|a, b| a.key.cmp(&b.key));
    anchored
}

/// Replayed journal state for one comment.
#[derive(Debug, Clone)]
struct CommentState {
    last: Option<String>,
    sha256: String,
    lifecycle: Lifecycle,
    deleted: bool,
}
/// Replayed journal state for one file's review marks.
#[derive(Debug, Clone, Default)]
struct FileState {
    last: Option<String>,
    reviewed: bool,
    hunks: BTreeSet<String>,
    /// tuicr content hash of the first journaled positive mark; later positive marks must match it.
    pinned: Option<u64>,
}
/// A verified journal and the state it replays to.
#[derive(Default)]
struct Journal {
    bytes: u64,
    events: Vec<DecisionEvent>,
    comments: BTreeMap<(String, String), CommentState>,
    files: BTreeMap<(String, String), FileState>,
}
fn genesis() -> String {
    "0".repeat(64)
}
impl Journal {
    fn head(&self) -> (u64, String) {
        self.events
            .last()
            .map_or((0, genesis()), |e| (e.seq + 1, e.hash.clone()))
    }
    fn replay(&mut self, event: &DecisionEvent) {
        let hash = Some(event.hash.clone());
        if let Some(id) = &event.comment_id {
            let key = (event.session_id.clone(), id.clone());
            let state = self.comments.entry(key).or_insert_with(|| CommentState {
                last: None,
                sha256: String::new(),
                lifecycle: Lifecycle::LocalDraft,
                deleted: false,
            });
            state.last.clone_from(&hash);
            // Every comment event but a tombstone carries the content and state it observed.
            state.deleted = event.kind == EventKind::Deleted;
            if let Some(sha) = &event.content_sha256 {
                state.sha256.clone_from(sha);
            }
            if let Some(l) = event.lifecycle_state {
                state.lifecycle = l;
            }
        } else if let (EventKind::FileReview, Some(file)) =
            (event.kind, event.anchor.as_ref().and_then(Anchor::file))
        {
            let state = self
                .files
                .entry((event.session_id.clone(), file.to_owned()))
                .or_default();
            state.last = hash;
            state.reviewed = event.reviewed.unwrap_or(false);
            state.hunks = event
                .reviewed_hunks
                .iter()
                .flatten()
                .cloned()
                .collect::<BTreeSet<_>>();
            if state.pinned.is_none() && (state.reviewed || !state.hunks.is_empty()) {
                state.pinned = event.content_hash;
            }
        }
    }
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Checkpoint {
    seq: u64,
    hash: String,
}
/// Read and verify the whole journal: exact canonical lines, consecutive `seq`, `prev` chain,
/// recomputed hashes, this run's id, and every create-new checkpoint still present at its sequence.
fn load_journal(directory: &Path, run_id: &str) -> Result<Journal> {
    let corrupt = |reason: String| TuicrImportError::Journal(reason);
    let path = directory.join(JOURNAL_FILE);
    let mut journal = Journal::default();
    if path.exists() {
        let bytes = storage::read_bytes(&path, JOURNAL_CAP)?;
        journal.bytes = bytes.len() as u64;
        let text = String::from_utf8(bytes).map_err(|_| corrupt("not UTF-8".into()))?;
        if !text.is_empty() && !text.ends_with('\n') {
            return Err(corrupt("the last line is incomplete (truncated)".into()));
        }
        let mut prev = genesis();
        for (seq, line) in (0u64..).zip(text.lines()) {
            let event: DecisionEvent = serde_json::from_str(line)
                .map_err(|e| corrupt(format!("line {seq} is not a decision event: {e}")))?;
            if serde_json::to_string(&event)? != line
                || event.schema != EVENT_SCHEMA
                || event.seq != seq
                || event.prev != prev
                || event.run_id != run_id
                || event.digest()? != event.hash
            {
                return Err(corrupt(format!(
                    "event {seq} was modified or reordered: its chain or hash does not verify"
                )));
            }
            prev.clone_from(&event.hash);
            journal.replay(&event);
            journal.events.push(event);
        }
    }
    let checkpoints = directory.join(CHECKPOINT_DIR);
    if checkpoints.exists() {
        for (i, entry) in fs::read_dir(&checkpoints)?.enumerate() {
            if i >= ENTRY_CAP {
                return Err(corrupt("too many checkpoints".into()));
            }
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let c: Checkpoint = storage::read_json(&entry.path(), 4096)
                .map_err(|e| corrupt(format!("checkpoint {name}: {e}")))?;
            if name != format!("{:020}.json", c.seq) {
                return Err(corrupt(format!("checkpoint {name} is misnamed")));
            }
            let at = usize::try_from(c.seq)
                .ok()
                .and_then(|i| journal.events.get(i));
            if at.is_none_or(|e| e.hash != c.hash) {
                return Err(corrupt(format!(
                    "checkpoint {} names an event the journal no longer holds (truncated or rewritten)",
                    c.seq
                )));
            }
        }
    }
    Ok(journal)
}

/// The run's sealed inventory, verified against the manifest's source fingerprint.
struct Sealed {
    head: String,
    inventory: storage::Inventory,
}
fn sealed(directory: &Path, manifest: &Manifest) -> Result<Sealed> {
    let path = directory.join(INVENTORY_FILE);
    let inventory: storage::Inventory = storage::read_json(&path, 8 * 1024 * 1024)?;
    if storage::fingerprint_tree(&directory.join(SNAPSHOT_DIR), &inventory)? != manifest.source_tree
    {
        return Err(ReviewFailure(
            "sealed snapshot or inventory differs from the manifest source fingerprint".into(),
        )
        .into());
    }
    // `Inventory` keeps HEAD private to storage; read the same sealed file for it.
    let value: serde_json::Value = storage::read_json(&path, 8 * 1024 * 1024)?;
    let head = value["head"]
        .as_str()
        .map(str::trim)
        .filter(|h| tuicr::commit_id(h))
        .ok_or_else(|| ReviewFailure("sealed inventory has no commit HEAD".into()))?
        .to_owned();
    Ok(Sealed { head, inventory })
}
/// Refuse unless the session shows, for `path`, exactly the bytes the run sealed (or both lack it).
fn check_file(binding: &Binding<'_>, sealed: &Sealed, path: &str) -> Result<()> {
    let stale = |reason: String| TuicrImportError::StaleFile {
        path: path.into(),
        reason,
    };
    if !sealed.inventory.contains(path) {
        return Err(stale("outside the run's sealed source inventory".into()));
    }
    let copy = binding.directory.join(SNAPSHOT_DIR).join(path);
    let snapshot = match fs::symlink_metadata(&copy) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        _ => Some(storage::read_bytes(&copy, SOURCE_CAP)?),
    };
    match (snapshot, binding.session_bytes(path)) {
        (Some(_), Err(e)) => Err(stale(e)),
        (None, Ok(_)) => Err(stale("absent from the sealed snapshot".into())),
        (Some(a), Ok(b)) if a != b => Err(stale(format!(
            "the {} bytes the session reviews differ from the sealed snapshot",
            binding.revision.label()
        ))),
        // Equal bytes, or absent on both sides (a deleted file).
        _ => Ok(()),
    }
}
/// A review-level disposition is about the whole session, so every file it lists must still be
/// the sealed bytes. Files the inventory lacks are refused when the session can show bytes for
/// them (a new file), and skipped otherwise (tuicr pseudo-files such as commit messages).
fn check_scope(
    binding: &Binding<'_>,
    sealed: &Sealed,
    files: &BTreeMap<String, RawFile>,
) -> Result<()> {
    for path in files.keys() {
        if sealed.inventory.contains(path) {
            check_file(binding, sealed, path)?;
        } else if binding.session_bytes(path).is_ok() {
            return Err(TuicrImportError::StaleFile {
                path: path.clone(),
                reason: "the session shows a file outside the run's sealed source inventory".into(),
            });
        }
    }
    Ok(())
}

/// Builds events in order, chaining `seq`/`prev`/`hash` from the journal head.
struct Planner<'a> {
    run_id: &'a str,
    session_id: &'a str,
    revision: SessionRevision,
    next: u64,
    prev: String,
    events: Vec<DecisionEvent>,
}
impl Planner<'_> {
    fn blank(&self, kind: EventKind, actor: Actor) -> DecisionEvent {
        DecisionEvent {
            schema: EVENT_SCHEMA.into(),
            seq: 0,
            prev: String::new(),
            run_id: self.run_id.into(),
            session_id: self.session_id.into(),
            revision: self.revision.clone(),
            kind,
            actor,
            comment_id: None,
            ddf_authored: false,
            comment_type: None,
            lifecycle_state: None,
            previous_lifecycle_state: None,
            anchor: None,
            content_hash: None,
            content_sha256: None,
            content: None,
            links: vec![],
            reviewed: None,
            reviewed_hunks: None,
            conflict: None,
            supersedes: None,
            hash: String::new(),
        }
    }
    fn comment(&self, kind: EventKind, actor: Actor, c: &Observed, ddf: bool) -> DecisionEvent {
        DecisionEvent {
            comment_id: Some(c.id.clone()),
            ddf_authored: ddf,
            comment_type: c.comment_type.clone(),
            lifecycle_state: Some(c.lifecycle),
            anchor: Some(c.anchor.clone()),
            content_hash: c.content_hash,
            content_sha256: Some(c.sha256.clone()),
            content: Some(c.content.clone()),
            ..self.blank(kind, actor)
        }
    }
    /// Seal and append; returns the new event's hash.
    fn push(&mut self, mut event: DecisionEvent) -> Result<String> {
        event.seq = self.next;
        event.prev.clone_from(&self.prev);
        event.hash = event.digest()?;
        self.next += 1;
        self.prev.clone_from(&event.hash);
        self.events.push(event);
        Ok(self.prev.clone())
    }
}

/// Everything the planner compares: session, ledger and replayed journal.
struct Inputs<'a> {
    binding: &'a Binding<'a>,
    sealed: &'a Sealed,
    ledger: &'a Ledger,
    journal: &'a Journal,
    comments: &'a [Observed],
    files: &'a BTreeMap<String, RawFile>,
}
fn actor_for(c: &Observed, ddf: bool) -> Actor {
    if ddf {
        Actor {
            name: "unrecorded".into(),
            basis: ActorBasis::Unrecorded,
        }
    } else {
        Actor {
            name: c.author.clone(),
            basis: ActorBasis::TuicrAuthor,
        }
    }
}
/// Comment events: additions, dispositions, revisions (conflicts on council notes), restorations
/// and lifecycle transitions, in session order.
fn plan_comments(p: &mut Planner<'_>, x: &Inputs<'_>) -> Result<()> {
    let notes: Vec<&Observed> = x
        .comments
        .iter()
        .filter(|c| x.ledger.by_comment.contains_key(&c.id))
        .collect();
    let mut scope_checked = false;
    for c in x.comments {
        let ledger = x.ledger.by_comment.get(&c.id);
        let ddf = ledger.is_some();
        let prior = x
            .journal
            .comments
            .get(&(p.session_id.to_owned(), c.id.clone()))
            .cloned()
            .or_else(|| {
                ledger.map(|e| CommentState {
                    last: None,
                    sha256: e.content_sha256.clone(),
                    lifecycle: Lifecycle::LocalDraft,
                    deleted: false,
                })
            });
        let links = if ddf {
            vec![]
        } else {
            links(c, p.run_id, x.ledger, &notes)
        };
        let mut planned = Vec::new();
        match prior {
            None => {
                let kind = if links.is_empty() {
                    EventKind::CommentAdded
                } else {
                    EventKind::Disposition
                };
                planned.push(DecisionEvent {
                    links,
                    ..p.comment(kind, actor_for(c, false), c, false)
                });
            }
            Some(prior) if prior.deleted => planned.push(DecisionEvent {
                links,
                supersedes: prior.last,
                ..p.comment(EventKind::Restored, actor_for(c, ddf), c, ddf)
            }),
            Some(prior) => {
                let mut last = prior.last;
                if prior.sha256 != c.sha256 {
                    let conflict = ddf.then(|| {
                        "a council-authored note was edited outside DDF; its content no longer matches the export ledger".to_owned()
                    });
                    planned.push(DecisionEvent {
                        links,
                        conflict,
                        supersedes: last.take(),
                        ..p.comment(EventKind::Revised, actor_for(c, ddf), c, ddf)
                    });
                }
                if prior.lifecycle != c.lifecycle {
                    planned.push(DecisionEvent {
                        previous_lifecycle_state: Some(prior.lifecycle),
                        supersedes: last,
                        ..p.comment(EventKind::Lifecycle, actor_for(c, ddf), c, ddf)
                    });
                }
            }
        }
        for mut event in planned {
            match c.anchor.file() {
                Some(file) => check_file(x.binding, x.sealed, file)?,
                None if !scope_checked => {
                    check_scope(x.binding, x.sealed, x.files)?;
                    scope_checked = true;
                }
                None => {}
            }
            // A lifecycle event after a revision in the same batch supersedes that revision.
            if event.kind == EventKind::Lifecycle
                && event.supersedes.is_none()
                && let Some(prev) = p.events.last().filter(|e| e.comment_id == event.comment_id)
            {
                event.supersedes = Some(prev.hash.clone());
            }
            p.push(event)?;
        }
    }
    Ok(())
}
/// Tombstones for journaled or exported comments that are no longer in the session.
fn plan_tombstones(p: &mut Planner<'_>, x: &Inputs<'_>) -> Result<()> {
    let present: BTreeSet<&str> = x.comments.iter().map(|c| c.id.as_str()).collect();
    // Comment id -> hash of its latest event (None for an exported note never journaled).
    let mut gone: BTreeMap<String, Option<String>> = BTreeMap::new();
    for ((session, id), state) in &x.journal.comments {
        if session == p.session_id && !state.deleted && !present.contains(id.as_str()) {
            gone.insert(id.clone(), state.last.clone());
        }
    }
    for id in x.ledger.by_comment.keys() {
        let journaled = x
            .journal
            .comments
            .contains_key(&(p.session_id.to_owned(), id.clone()));
        if !journaled && !present.contains(id.as_str()) {
            gone.insert(id.clone(), None);
        }
    }
    for (id, last) in gone {
        let ddf = x.ledger.by_comment.contains_key(&id);
        let actor = if ddf {
            Actor {
                name: "unrecorded".into(),
                basis: ActorBasis::Unrecorded,
            }
        } else {
            // The last journaled author of the comment.
            let name = x
                .journal
                .events
                .iter()
                .rev()
                .find(|e| {
                    e.session_id == p.session_id
                        && e.comment_id.as_deref() == Some(id.as_str())
                        && e.actor.basis == ActorBasis::TuicrAuthor
                })
                .map_or_else(|| "unrecorded".into(), |e| e.actor.name.clone());
            Actor {
                name,
                basis: ActorBasis::TuicrAuthor,
            }
        };
        p.push(DecisionEvent {
            comment_id: Some(id),
            ddf_authored: ddf,
            supersedes: last,
            ..p.blank(EventKind::Deleted, actor)
        })?;
    }
    Ok(())
}
/// Reviewed-mark changes per file. A positive mark needs tuicr's content hash, equal to the one
/// pinned by the first journaled positive mark for that file, and the sealed bytes.
fn plan_files(p: &mut Planner<'_>, x: &Inputs<'_>) -> Result<()> {
    for (path, file) in x.files {
        let prior = x
            .journal
            .files
            .get(&(p.session_id.to_owned(), path.clone()))
            .cloned()
            .unwrap_or_default();
        if prior.reviewed == file.reviewed && prior.hunks == file.reviewed_hunks {
            continue;
        }
        let positive = file.reviewed || !file.reviewed_hunks.is_empty();
        if positive {
            let refuse = |reason: String| TuicrImportError::StaleReviewMark {
                path: path.clone(),
                reason,
            };
            let Some(hash) = file.content_hash else {
                return Err(refuse(
                    "the reviewed mark carries no tuicr content_hash".into(),
                ));
            };
            if let Some(pinned) = prior.pinned
                && pinned != hash
            {
                return Err(refuse(format!(
                    "tuicr content_hash {hash} differs from {pinned} journaled for this file; the mark was made on a different diff"
                )));
            }
        }
        check_file(x.binding, x.sealed, path)?;
        p.push(DecisionEvent {
            anchor: Some(Anchor::File { file: path.clone() }),
            content_hash: file.content_hash,
            reviewed: Some(file.reviewed),
            reviewed_hunks: Some(file.reviewed_hunks.iter().cloned().collect()),
            supersedes: prior.last,
            ..p.blank(
                EventKind::FileReview,
                Actor {
                    name: "unrecorded".into(),
                    basis: ActorBasis::Unrecorded,
                },
            )
        })?;
    }
    Ok(())
}

/// Holds `<run>/decisions.lock` (create-new) for the duration of an executing import.
struct Lock(PathBuf);
impl Lock {
    fn acquire(directory: &Path) -> Result<Self> {
        let path = directory.join(LOCK_FILE);
        match storage::write_new(&path, std::process::id().to_string().as_bytes()) {
            Ok(()) => Ok(Self(path)),
            Err(_) if path.exists() => Err(TuicrImportError::Locked {
                lock: path.display().to_string(),
            }),
            Err(e) => Err(e.into()),
        }
    }
}
impl Drop for Lock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
/// Append `events` to the verified journal of `verified_len` bytes, then record a checkpoint.
fn append(directory: &Path, verified_len: u64, events: &[DecisionEvent]) -> Result<()> {
    let mut lines = Vec::new();
    for event in events {
        serde_json::to_writer(&mut lines, event)?;
        lines.push(b'\n');
    }
    let path = directory.join(JOURNAL_FILE);
    let mut file = OpenOptions::new()
        .append(true)
        .create(true)
        .mode(0o600)
        .open(&path)?;
    if file.metadata()?.len() != verified_len {
        return Err(TuicrImportError::Journal(
            "the journal changed while this import was planned".into(),
        ));
    }
    file.write_all(&lines)?;
    file.sync_all()?;
    fs::File::open(directory)?.sync_all()?;
    let Some(last) = events.last() else {
        return Ok(());
    };
    let checkpoints = directory.join(CHECKPOINT_DIR);
    if !checkpoints.exists() {
        storage::make_dir(&checkpoints)?;
    }
    storage::write_json(
        &checkpoints.join(format!("{:020}.json", last.seq)),
        &Checkpoint {
            seq: last.seq,
            hash: last.hash.clone(),
        },
    )?;
    Ok(())
}

/// Import human dispositions from the tuicr session file `session_path` into the run's decision journal.
///
/// Without `options.execute` this is a dry run: it performs every check and reports the events it
/// would append, and writes nothing. Re-running with no session change plans no events.
/// # Errors
/// Refuses a run, session or export ledger that fails the shared identity checks; a session whose
/// `base_commit` is not the run's sealed HEAD; dispositions on files whose reviewed bytes differ from
/// the sealed snapshot; reviewed marks without, or with a changed, tuicr content hash; unsafe
/// session content; a malformed, rewritten or truncated journal; and a held import lock.
pub fn import_tuicr(
    directory: &Path,
    session_path: &Path,
    options: ImportOptions,
) -> Result<TuicrImport> {
    let directory = fs::canonicalize(directory)?;
    let manifest = load_manifest(&directory)?;
    let sealed = sealed(&directory, &manifest)?;
    let OpenSession {
        path: session_path,
        bytes,
        session,
        revision,
        repository,
    } = tuicr::open_session(&manifest, session_path)?;
    if !tuicr::commit_id(&session.base_commit) {
        return Err(TuicrExportError::UnsupportedSession(
            "base_commit is not a full commit id, so the session cannot be bound to the run".into(),
        )
        .into());
    }
    if session.base_commit != sealed.head {
        return Err(TuicrImportError::StaleRevision {
            session: session.base_commit.clone(),
            run: sealed.head.clone(),
        });
    }
    let raw: RawSession = serde_json::from_slice(&bytes)
        .map_err(|e| TuicrImportError::UnsafeSession(e.to_string()))?;
    let (comments, files) = observed_comments(raw)?;
    let ledger = Ledger::load(&directory, &manifest.run_id, &session.id)?;
    let _lock = if options.execute {
        Some(Lock::acquire(&directory)?)
    } else {
        None
    };
    let journal = load_journal(&directory, &manifest.run_id)?;
    let binding = Binding {
        directory: &directory,
        manifest: &manifest,
        repository,
        session: &session,
        revision: revision.clone(),
    };
    let (next, prev) = journal.head();
    let mut planner = Planner {
        run_id: &manifest.run_id,
        session_id: &session.id,
        revision: SessionRevision {
            diff_source: session
                .diff_source
                .clone()
                .unwrap_or_else(|| "working_tree".into()),
            base_commit: session.base_commit.clone(),
            new_side: revision.label(),
        },
        next,
        prev,
        events: vec![],
    };
    let inputs = Inputs {
        binding: &binding,
        sealed: &sealed,
        ledger: &ledger,
        journal: &journal,
        comments: &comments,
        files: &files,
    };
    plan_comments(&mut planner, &inputs)?;
    plan_tombstones(&mut planner, &inputs)?;
    plan_files(&mut planner, &inputs)?;
    if options.execute && !planner.events.is_empty() {
        append(&directory, journal.bytes, &planner.events)?;
    }
    Ok(TuicrImport {
        schema: "deep-diff-forge.tuicr-import.v1",
        run_id: manifest.run_id.clone(),
        session: session_path.display().to_string(),
        session_id: session.id.clone(),
        revision: planner.revision,
        executed: options.execute,
        journal: directory.join(JOURNAL_FILE).display().to_string(),
        journal_events: next,
        conflicts: planner
            .events
            .iter()
            .filter(|e| e.conflict.is_some())
            .count(),
        events: planner.events,
        authority: "human decisions recorded as observed in a local tuicr session; not approval, merge or verified fact",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(file: &str, side: &str, start: u32, end: u32) -> Anchor {
        Anchor::Lines {
            file: file.into(),
            side: side.into(),
            start_line: start,
            end_line: end,
        }
    }

    #[test]
    fn anchors_overlap_only_on_the_same_file_and_side() {
        let a = lines("x.rs", "new", 2, 3);
        assert!(a.overlaps(&lines("x.rs", "new", 3, 9)));
        assert!(a.overlaps(&lines("x.rs", "new", 1, 2)));
        assert!(!a.overlaps(&lines("x.rs", "new", 4, 4)));
        assert!(!a.overlaps(&lines("x.rs", "old", 2, 3)));
        assert!(!a.overlaps(&lines("y.rs", "new", 2, 3)));
        assert!(!a.overlaps(&Anchor::File {
            file: "x.rs".into()
        }));
        assert!(!Anchor::Review.overlaps(&Anchor::Review));
    }

    #[test]
    fn tags_name_findings_of_this_run_only() {
        let run = "r".repeat(64);
        let text = format!(
            "agree [ddf:a/independent/finding/F1] and\n> [DDF-COUNCIL run={run} reviewer=b round=challenge finding=F2]\n[DDF-COUNCIL run=other reviewer=c round=independent finding=F3] [ddf:unterminated"
        );
        assert_eq!(
            tagged_keys(&text, &run).into_iter().collect::<Vec<_>>(),
            ["a/independent/finding/F1", "b/challenge/finding/F2"]
        );
        let long = format!("[ddf:{}]", "x".repeat(TAG_CAP + 1));
        assert!(tagged_keys(&long, &run).is_empty());
    }

    #[test]
    fn session_paths_must_stay_inside_the_repository() {
        assert!(safe_path("src/x.rs"));
        for bad in ["", "/etc/passwd", "../x", "a/../../x", "./x"] {
            assert!(!safe_path(bad), "{bad:?}");
        }
    }

    #[test]
    fn observed_comments_refuse_duplicate_ids_bad_lines_and_unknown_states() {
        let parse = |v: serde_json::Value| {
            observed_comments(serde_json::from_value::<RawSession>(v).unwrap())
        };
        let comment = |id: &str| serde_json::json!({"id": id, "content": "c"});
        let dup = serde_json::json!({"review_comments": [comment("a")], "files": {"x.rs": {"file_comments": [comment("a")]}}});
        assert!(matches!(
            parse(dup),
            Err(TuicrImportError::UnsafeSession(_))
        ));
        let line =
            serde_json::json!({"files": {"x.rs": {"line_comments": {"two": [comment("b")]}}}});
        assert!(matches!(
            parse(line),
            Err(TuicrImportError::UnsafeSession(_))
        ));
        let id = serde_json::json!({"review_comments": [comment("a b")], "files": {}});
        assert!(matches!(parse(id), Err(TuicrImportError::UnsafeSession(_))));
        let state = serde_json::from_value::<RawSession>(serde_json::json!({
            "review_comments": [{"id": "a", "content": "c", "lifecycle_state": "merged"}], "files": {}
        }));
        assert!(state.is_err());
        let ok = parse(serde_json::json!({"files": {"x.rs": {"line_comments": {
            "4": [{"id": "r", "content": "c", "side": "old", "line_range": {"start": 2, "end": 4}}],
            "1": [{"id": "s", "content": "c"}]
        }}}}))
        .unwrap()
        .0;
        assert_eq!(ok[0].id, "s");
        assert_eq!(ok[1].anchor, lines("x.rs", "old", 2, 4));
        assert_eq!(ok[0].author, "user");
        assert_eq!(ok[0].lifecycle, Lifecycle::LocalDraft);
    }

    fn planner() -> Planner<'static> {
        Planner {
            run_id: "run",
            session_id: "s",
            revision: SessionRevision {
                diff_source: "working_tree".into(),
                base_commit: "a".repeat(40),
                new_side: "working_tree".into(),
            },
            next: 0,
            prev: genesis(),
            events: vec![],
        }
    }
    fn user() -> Actor {
        Actor {
            name: "user".into(),
            basis: ActorBasis::TuicrAuthor,
        }
    }
    /// A private directory, removed on drop.
    struct Scratch(PathBuf);
    impl Scratch {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let dir = std::env::temp_dir().join(format!(
                "ddf-journal-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            fs::create_dir(&dir).unwrap();
            Self(dir)
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn event_hash_covers_every_field_but_itself() {
        let mut planner = planner();
        let actor = user();
        let first = planner
            .push(planner.blank(EventKind::CommentAdded, actor.clone()))
            .unwrap();
        planner
            .push(planner.blank(EventKind::Deleted, actor))
            .unwrap();
        let [a, b] = &planner.events[..] else {
            panic!("two events")
        };
        assert_eq!((a.seq, a.prev.as_str()), (0, genesis().as_str()));
        assert_eq!((b.seq, b.prev.as_str()), (1, first.as_str()));
        assert_eq!(a.digest().unwrap(), a.hash);
        let mut forged = b.clone();
        forged.actor.name = "someone else".into();
        assert_ne!(forged.digest().unwrap(), b.hash);
    }

    /// Each line check stands alone: a resealed hash does not excuse a wrong schema, sequence,
    /// chain link or run, and a re-serialised line must be byte-identical.
    #[test]
    fn every_journal_line_check_refuses_on_its_own() {
        let dir = Scratch::new();
        let mut p = planner();
        p.push(p.blank(EventKind::CommentAdded, user())).unwrap();
        p.push(p.blank(EventKind::Deleted, user())).unwrap();
        let [first, last] = &p.events[..] else {
            panic!("two events")
        };
        let line = |e: &DecisionEvent| serde_json::to_string(e).unwrap();
        let load = |last: &str| {
            fs::write(
                dir.0.join(JOURNAL_FILE),
                format!("{}\n{last}\n", line(first)),
            )
            .unwrap();
            load_journal(&dir.0, "run")
        };
        let resealed = |edit: fn(&mut DecisionEvent)| {
            let mut e = last.clone();
            edit(&mut e);
            e.hash = e.digest().unwrap();
            line(&e)
        };
        assert_eq!(load(&line(last)).unwrap().events, p.events);
        let cases = [
            ("non-canonical line", line(last).replacen('{', "{ ", 1)),
            (
                "other schema",
                resealed(|e| e.schema = "deep-diff-forge.decision.v0".into()),
            ),
            ("skipped seq", resealed(|e| e.seq = 2)),
            ("broken chain", resealed(|e| e.prev = genesis())),
            ("other run", resealed(|e| e.run_id = "other".into())),
            ("stale hash", {
                let mut e = last.clone();
                e.content = Some("forged".into());
                line(&e)
            }),
        ];
        for (name, text) in cases {
            let refused = load(&text);
            assert!(
                matches!(&refused, Err(TuicrImportError::Journal(r)) if r.starts_with("event 1 was modified")),
                "{name}: {refused:?}",
                refused = refused.map(|j| j.events.len())
            );
        }
    }

    #[test]
    fn journal_is_read_up_to_its_cap_and_no_further() {
        let dir = Scratch::new();
        // The documented 64 MiB, written out so the test does not move with the constant.
        let cap: u64 = 64 << 20;
        let file = fs::File::create(dir.0.join(JOURNAL_FILE)).unwrap();
        // Exactly the cap is read in full, then refused for its content (NULs, no final newline).
        file.set_len(cap).unwrap();
        let at_cap = load_journal(&dir.0, "run").map(|j| j.bytes);
        assert!(
            matches!(&at_cap, Err(TuicrImportError::Journal(r)) if r.contains("incomplete")),
            "{at_cap:?}"
        );
        file.set_len(cap + 1).unwrap();
        let over = load_journal(&dir.0, "run").map(|j| j.bytes);
        assert!(
            matches!(&over, Err(e @ TuicrImportError::Session(_)) if e.to_string().contains("acquisition budget")),
            "{over:?}"
        );
    }

    #[test]
    fn only_the_first_positive_mark_pins_the_content_hash() {
        let p = planner();
        let mark = |reviewed: bool, hunks: &[&str], hash: u64| DecisionEvent {
            anchor: Some(Anchor::File {
                file: "x.rs".into(),
            }),
            content_hash: Some(hash),
            reviewed: Some(reviewed),
            reviewed_hunks: Some(hunks.iter().map(|h| (*h).to_owned()).collect()),
            ..p.blank(
                EventKind::FileReview,
                Actor {
                    name: "unrecorded".into(),
                    basis: ActorBasis::Unrecorded,
                },
            )
        };
        let mut journal = Journal::default();
        let pinned = |j: &Journal| j.files[&("s".to_owned(), "x.rs".to_owned())].pinned;
        journal.replay(&mark(false, &[], 1));
        assert_eq!(pinned(&journal), None, "an unmarked file pins nothing");
        journal.replay(&mark(false, &["h1"], 2));
        assert_eq!(
            pinned(&journal),
            Some(2),
            "a reviewed hunk is a positive mark"
        );
        journal.replay(&mark(true, &[], 3));
        assert_eq!(pinned(&journal), Some(2), "later marks keep the first pin");
    }

    #[test]
    fn only_an_existing_lock_file_is_reported_as_locked() {
        let dir = Scratch::new();
        let lock = Lock::acquire(&dir.0).unwrap();
        assert!(matches!(
            Lock::acquire(&dir.0),
            Err(TuicrImportError::Locked { .. })
        ));
        drop(lock);
        assert!(!dir.0.join(LOCK_FILE).exists(), "the lock is released");
        // Any other failure to create the lock is reported as itself.
        assert!(matches!(
            Lock::acquire(&dir.0.join("missing")),
            Err(TuicrImportError::Session(_))
        ));
    }
}
