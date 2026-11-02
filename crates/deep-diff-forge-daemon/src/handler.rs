// DDF review route: DDF-R26, DDF-R27, DDF-R29. Status and remaining work are recorded in the linked ledger; this comment does not claim delivery.
// Design/status: docs/RECOMMENDATION_INTEGRATION.md#remaining-module-routes
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fcrates--deep-diff-forge-daemon--src--handler
use crate::admission::{Admission, Answering};
use crate::metrics::Metrics;
use crate::protocol::{
    INPUT_CONTRACT_REFUSED, PATCH_PARSE_FAILED, PROTOCOL_UNSUPPORTED, PROTOCOL_VERSION,
    RESOURCE_EXHAUSTED, Request, RpcError, SESSION_NOT_FOUND,
};
use deep_diff_forge_core::{PatchHunk, PatchLine, ReviewFile, SemanticSpan};
use deep_diff_forge_graph::{RankedFile, rank};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

/// Maximum number of concurrent review sessions retained in memory.
///
/// When a `session.open` request would push the count past this limit, the
/// least-recently-used session is evicted first. This provides a hard bound on
/// daemon memory growth regardless of how many sessions a client opens without
/// closing them (denial-of-service hardening).
///
/// 64 concurrent review sessions is generous for interactive use (a typical
/// editor opens one or two) while keeping the worst-case memory footprint
/// bounded to `O(MAX_SESSIONS × patch_size)`.
pub(crate) const MAX_SESSIONS: usize = 64;

/// Maximum tracked heap payload retained across all sessions.
///
/// A session-count limit alone permits `MAX_SESSIONS` near-64 MiB patches to
/// retain several GiB. The byte budget makes retention independent of request
/// count; LRU sessions are evicted until both limits are satisfied.
const MAX_RETAINED_SESSION_BYTES: usize = 128 * 1024 * 1024;

/// Per-session state stored inside the engine.
struct SessionEntry {
    /// Shared, so a snapshot ranks it after the session lock is released.
    files: Arc<Vec<ReviewFile>>,
    /// Estimated heap allocation held by `files` and its nested collections.
    retained_bytes: usize,
    /// Monotonically increasing access tick; updated on open and on snapshot.
    /// The session with the smallest tick is the least-recently-used candidate
    /// for eviction when the cap is reached.
    last_tick: u64,
}

/// A parsed patch ready to be retained as a session, with its retained size already estimated.
/// The estimate walks the whole model, so it is made here, before the session lock is taken.
struct SessionPayload {
    files: Arc<Vec<ReviewFile>>,
    retained_bytes: usize,
}

impl SessionPayload {
    fn new(files: Vec<ReviewFile>) -> Self {
        let retained_bytes = estimated_session_bytes(&files);
        Self {
            files: Arc::new(files),
            retained_bytes,
        }
    }
}

/// The session table: the daemon's only mutable review state. One lock in [`Engine`] owns it,
/// and every method here is bookkeeping, so the lock is never held while a patch is parsed or
/// ranked or Git runs.
///
/// Session retention is bounded at `MAX_SESSIONS`. When capacity is reached,
/// opening a new session evicts the least-recently-used (LRU) entry — the one
/// with the smallest `last_tick` value — before inserting the new one.
struct Sessions {
    entries: HashMap<String, SessionEntry>,
    next_session: u64,
    /// Monotonically increasing counter incremented on every session open or
    /// snapshot access; used for LRU ordering.
    tick: u64,
    retained_bytes: usize,
    max_retained_bytes: usize,
}

impl Sessions {
    fn new() -> Self {
        Self {
            entries: HashMap::new(),
            next_session: 1,
            tick: 0,
            retained_bytes: 0,
            max_retained_bytes: MAX_RETAINED_SESSION_BYTES,
        }
    }

    /// Drop every session after a panic while the table was held (anchor A-03): the table and its
    /// byte accounting may be inconsistent, so they start again empty. The session-id counter is
    /// kept, so no old id is reused.
    fn reset(&mut self) {
        self.entries.clear();
        self.retained_bytes = 0;
        self.tick = 0;
    }

    /// Advance the global tick and return the new value.
    fn next_tick(&mut self) -> u64 {
        if self.tick == u64::MAX {
            // Rebase the at-most-64 entries while preserving their order, so a
            // multi-decade daemon cannot panic or invert LRU ordering.
            let mut ordered: Vec<(String, u64)> = self
                .entries
                .iter()
                .map(|(id, entry)| (id.clone(), entry.last_tick))
                .collect();
            ordered.sort_by_key(|(_, tick)| *tick);
            for (index, (id, _)) in ordered.into_iter().enumerate() {
                if let Some(entry) = self.entries.get_mut(&id) {
                    entry.last_tick = index as u64 + 1;
                }
            }
            self.tick = self.entries.len() as u64;
        }
        self.tick += 1;
        self.tick
    }

    /// Evict the least-recently-used session, returning whether one existed.
    fn evict_lru(&mut self) -> bool {
        // Find the key with the smallest last_tick (LRU).
        let lru_key = self
            .entries
            .iter()
            .min_by_key(|(_, entry)| entry.last_tick)
            .map(|(key, _)| key.clone());
        if let Some(key) = lru_key
            && let Some(removed) = self.entries.remove(&key)
        {
            self.retained_bytes = self.retained_bytes.saturating_sub(removed.retained_bytes);
            return true;
        }
        false
    }

    fn open_session(&mut self, payload: SessionPayload) -> Result<String, RpcError> {
        let SessionPayload {
            files,
            retained_bytes,
        } = payload;
        if retained_bytes > self.max_retained_bytes {
            return Err(RpcError::new(
                RESOURCE_EXHAUSTED,
                "session exceeds the retained-memory budget",
            ));
        }
        // At most one eviction per session held, so the loop ends even if eviction stops working.
        for _ in 0..=self.entries.len() {
            let over = self.entries.len() >= MAX_SESSIONS
                || self.retained_bytes.saturating_add(retained_bytes) > self.max_retained_bytes;
            if !over || !self.evict_lru() {
                break;
            }
        }

        // Wrap safely and skip any still-live id after the theoretical wrap. At most
        // `MAX_SESSIONS` ids are live, so one of the next `MAX_SESSIONS + 1` candidates is free.
        let id = (0..=MAX_SESSIONS)
            .map(|_| {
                let candidate = format!("s{}", self.next_session);
                self.next_session = self.next_session.checked_add(1).unwrap_or(1);
                candidate
            })
            .find(|candidate| !self.entries.contains_key(candidate))
            .ok_or_else(|| RpcError::new(RESOURCE_EXHAUSTED, "no free session id"))?;
        let tick = self.next_tick();
        self.retained_bytes = self.retained_bytes.saturating_add(retained_bytes);
        self.entries.insert(
            id.clone(),
            SessionEntry {
                files,
                retained_bytes,
                last_tick: tick,
            },
        );
        Ok(id)
    }

    /// A session's parsed patch, refreshing its LRU tick. The patch is shared, not cloned; the
    /// caller ranks it after the lock is released.
    fn snapshot_files(&mut self, id: &str) -> Option<Arc<Vec<ReviewFile>>> {
        let tick = self.next_tick();
        let entry = self.entries.get_mut(id)?;
        entry.last_tick = tick;
        Some(Arc::clone(&entry.files))
    }

    fn close_session(&mut self, id: &str) -> bool {
        if let Some(removed) = self.entries.remove(id) {
            self.retained_bytes = self.retained_bytes.saturating_sub(removed.retained_bytes);
            true
        } else {
            false
        }
    }
}

/// Daemon engine: the session table, lifecycle state and the request admission gate.
///
/// One engine serves every connection thread at once (`&Engine` is all a request needs). Only
/// the session table is mutable, and its lock is held for bookkeeping alone; parsing, ranking,
/// rendering and Git run outside it, so requests run in parallel. The work running at once is
/// bounded by [`Admission`] (the summed request bytes) and by the server's connection limit.
pub struct Engine {
    sessions: Mutex<Sessions>,
    running: AtomicBool,
    pid: u32,
    started: Instant,
    /// Repository roots `review.change` may read (canonical paths). Empty: it reads none.
    allowed_repos: Vec<std::path::PathBuf>,
    admission: Admission,
    answering: Answering,
    metrics: Metrics,
    log_format: crate::log::LogFormat,
}

// Every connection thread shares one engine, so a field that is not thread-safe fails the build
// here rather than at the server's `Arc`.
const _: () = {
    const fn shared_across_threads<T: Send + Sync>() {}
    shared_across_threads::<Engine>();
};

/// The Git time limit for one `review.change`: a stalled Git cannot hold its connection longer.
pub const REVIEW_CHANGE_GIT_LIMIT: std::time::Duration = std::time::Duration::from_secs(30);

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine {
    /// Create a fresh engine bound to the current process.
    #[must_use]
    #[expect(
        clippy::disallowed_methods,
        reason = "the daemon uptime clock, read once when the engine starts"
    )]
    pub fn new() -> Self {
        Self {
            sessions: Mutex::new(Sessions::new()),
            running: AtomicBool::new(true),
            pid: std::process::id(),
            started: Instant::now(),
            allowed_repos: Vec::new(),
            admission: Admission::default(),
            answering: Answering::default(),
            metrics: Metrics::new(),
            log_format: crate::log::LogFormat::Text,
        }
    }

    /// Let `review.change` read repositories at or under these roots, which the operator names
    /// (`--allow-repo`). Each root is resolved to its canonical path now; one that does not
    /// exist is an error, so a typo cannot silently allow nothing or something else.
    ///
    /// # Errors
    /// The root and the I/O error when a root cannot be resolved.
    pub fn allowing_repos(
        mut self,
        roots: &[std::path::PathBuf],
    ) -> Result<Self, (std::path::PathBuf, std::io::Error)> {
        for root in roots {
            let canonical = std::fs::canonicalize(root).map_err(|e| (root.clone(), e))?;
            self.allowed_repos.push(canonical);
        }
        Ok(self)
    }

    /// The session table, for bookkeeping only: never hold it across parsing, ranking or Git.
    /// A panic while it was held may have left it half-updated, so a poisoned table is emptied
    /// (anchor A-03) and served again; the pid, start time, `running` and the session-id counter
    /// are kept, so no old id is reused. A panic outside the table leaves every session intact.
    fn sessions(&self) -> MutexGuard<'_, Sessions> {
        self.sessions.lock().unwrap_or_else(|poisoned| {
            let mut table = poisoned.into_inner();
            table.reset();
            self.sessions.clear_poison();
            self.metrics.session_reset();
            table
        })
    }

    /// Write the daemon's log lines in `format` (`daemon start --log-format`).
    #[must_use]
    pub fn with_log_format(mut self, format: crate::log::LogFormat) -> Self {
        self.log_format = format;
        self
    }

    /// How the daemon writes its log lines.
    #[must_use]
    pub const fn log_format(&self) -> crate::log::LogFormat {
        self.log_format
    }

    /// Write one log line for `event`.
    pub fn log(&self, event: &str, message: &str) {
        crate::log::write(self.log_format, event, message);
    }

    /// The counters `daemon.metrics` reports.
    #[must_use]
    pub fn metrics(&self) -> &Metrics {
        &self.metrics
    }

    /// The gate on the request bytes in flight, for a server that runs requests in parallel.
    #[must_use]
    pub fn admission(&self) -> &Admission {
        &self.admission
    }

    /// The requests read and not yet answered, which a stop waits for.
    #[must_use]
    pub fn answering(&self) -> &Answering {
        &self.answering
    }

    /// Whether the engine is still serving (cleared by `daemon.shutdown`).
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Acquire)
    }

    /// Number of open sessions (always `<= MAX_SESSIONS`).
    #[must_use]
    pub fn session_count(&self) -> usize {
        self.sessions().entries.len()
    }

    /// Test-only: panic while the session table is held, to exercise its recovery.
    #[cfg(test)]
    pub(crate) fn panic_holding_sessions(&self) {
        let _table = self.sessions();
        panic!("test-only panic while holding the session table");
    }
}

/// Estimate nested heap allocations retained by a parsed review model. Vector
/// element storage is counted by capacity; owned string buffers are added
/// separately. Saturating arithmetic fails closed at the retention gate.
fn estimated_session_bytes(files: &Vec<ReviewFile>) -> usize {
    let mut total = files
        .capacity()
        .saturating_mul(std::mem::size_of::<ReviewFile>());
    for file in files {
        total = total.saturating_add(file.path.capacity());
        total = total.saturating_add(
            file.patch_twin
                .hunks
                .capacity()
                .saturating_mul(std::mem::size_of::<PatchHunk>()),
        );
        for hunk in &file.patch_twin.hunks {
            total = total.saturating_add(
                hunk.lines
                    .capacity()
                    .saturating_mul(std::mem::size_of::<PatchLine>()),
            );
            for line in &hunk.lines {
                total = total.saturating_add(line.text.capacity());
            }
        }
        total = total.saturating_add(
            file.patch_twin
                .metadata
                .capacity()
                .saturating_mul(std::mem::size_of::<String>()),
        );
        for metadata in &file.patch_twin.metadata {
            total = total.saturating_add(metadata.capacity());
        }
        if let Some(semantic) = &file.semantic_twin {
            total = total.saturating_add(semantic.language.capacity());
            total = total.saturating_add(
                semantic
                    .spans
                    .capacity()
                    .saturating_mul(std::mem::size_of::<SemanticSpan>()),
            );
        }
        total = total.saturating_add(
            file.planner
                .notes
                .capacity()
                .saturating_mul(std::mem::size_of::<String>()),
        );
        for note in &file.planner.notes {
            total = total.saturating_add(note.capacity());
        }
    }
    total
}

/// One method the daemon implements, as `engine.methods` reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MethodSpec {
    pub name: &'static str,
    /// The `params` object, as `name: type` pairs (`?` marks an optional one).
    pub params: &'static str,
    /// The shape of `result`.
    pub result: &'static str,
    /// The protocol version that introduced it.
    pub since: u32,
    /// How much of the in-flight budget a request of this method holds while it runs.
    pub work: Work,
}

/// What bounds a method's working memory, so admission can weigh a request before it runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Work {
    /// Bounded by its request line: the patch it carries, parsed and rendered.
    Request,
    /// Acquires a change from Git, up to the patch byte budget, whatever the line's length.
    GitAcquisition,
}

impl Work {
    /// The bytes of the in-flight budget a request of `line_bytes` holds.
    #[must_use]
    pub fn weight(self, line_bytes: usize) -> usize {
        match self {
            Self::Request => line_bytes,
            Self::GitAcquisition => line_bytes.max(deep_diff_forge_patch::DEFAULT_BYTE_BUDGET),
        }
    }
}

/// The admission weight of a request line: its method's [`Work`], or the line's own length for
/// a method the daemon does not have (refused at once).
#[must_use]
pub fn request_weight(request: &Request, line_bytes: usize) -> usize {
    METHODS
        .iter()
        .find(|m| m.name == request.method)
        .map_or(line_bytes, |m| m.work.weight(line_bytes))
}

impl MethodSpec {
    /// The names in [`MethodSpec::params`] (`name: type` or `name?: type`, comma-separated).
    pub fn param_names(&self) -> impl Iterator<Item = &'static str> {
        self.params
            .split(", ")
            .filter_map(|param| param.split(':').next())
            .map(|name| name.trim().trim_end_matches('?'))
            .filter(|name| !name.is_empty())
    }
}

const fn method(
    name: &'static str,
    params: &'static str,
    result: &'static str,
    since: u32,
    work: Work,
) -> MethodSpec {
    MethodSpec {
        name,
        params,
        result,
        since,
        work,
    }
}

/// Every method [`dispatch`] answers; a test keeps this list and the dispatcher in step.
pub const METHODS: &[MethodSpec] = &[
    method(
        "engine.initialize",
        "protocol?: integer",
        "{server, version, protocol, methods}",
        0,
        Work::Request,
    ),
    method(
        "engine.methods",
        "",
        "{protocol, methods: [{name, params, result, since}]}",
        1,
        Work::Request,
    ),
    method(
        "daemon.health",
        "",
        "{status, version, pid, sessions, retained_bytes, cache_entries, protocol}",
        0,
        Work::Request,
    ),
    method(
        "daemon.status",
        "",
        "{running, uptime_secs, sessions, version, protocol}",
        0,
        Work::Request,
    ),
    method("daemon.shutdown", "", "{stopping}", 0, Work::Request),
    method(
        "diff.plan",
        "patch: string",
        "[ranked file]",
        0,
        Work::Request,
    ),
    method(
        "diff.compute",
        "patch: string, require_files?: boolean, require_hunks?: boolean",
        "the deep-diff-forge.review.v0 document `deep-diff-forge --stdin-patch --json` prints for the same patch",
        1,
        Work::Request,
    ),
    method(
        "diff.rank",
        "patch: string, require_files?: boolean, require_hunks?: boolean",
        "the deep-diff-forge.rank.v0 document `deep-diff-forge --stdin-patch --rank --json` prints for the same patch",
        1,
        Work::Request,
    ),
    method(
        "review.outline",
        "patch: string, require_files?: boolean, require_hunks?: boolean, max_bytes?: integer, cursor?: string",
        "deep-diff-forge.review-outline.v0: files in review-risk order with their hunks' addresses, headings and counts, no line text; next_cursor continues",
        1,
        Work::Request,
    ),
    method(
        "review.hunks",
        "patch: string, require_files?: boolean, require_hunks?: boolean, addresses?: [string], paths?: [string], max_bytes?: integer, cursor?: string",
        "deep-diff-forge.review-hunks.v0: the chosen hunks verbatim (all, by address or by path), within max_bytes; next_cursor continues",
        1,
        Work::Request,
    ),
    method(
        "review.change",
        "repo: string, selection: \"staged\"|\"working_tree\"|\"range\", range?: \"A..B\", include_untracked?: boolean, view?: \"outline\"|\"hunks\"|\"review\"|\"rank\", require_files?: boolean, require_hunks?: boolean, addresses?: [string], paths?: [string], max_bytes?: integer, cursor?: string",
        "the chosen view of the change Git returns for the selection, sealed over Git's exact bytes, with an acquisition member; only for a repository at or under a root the operator allowed (--allow-repo), and Git is stopped after 30 s",
        1,
        Work::GitAcquisition,
    ),
    method(
        "session.open",
        "patch: string",
        "{session, files}",
        0,
        Work::Request,
    ),
    method(
        "session.snapshot",
        "session: string",
        "[ranked file]",
        0,
        Work::Request,
    ),
    method(
        "session.close",
        "session: string",
        "{closed}",
        0,
        Work::Request,
    ),
    method(
        "daemon.metrics",
        "",
        "deep-diff-forge.daemon-metrics.v0: requests, errors and latency per method; admission, connection and engine counters since the daemon started",
        2,
        Work::Request,
    ),
    method(
        "findings.verify",
        "patch: string, findings: [object], require_files?: boolean, require_hunks?: boolean",
        "deep-diff-forge.findings.v0: each finding accepted (anchor, address and line checked against the sealed review, body sanitised) or refused with a reason; claim_verified is always false",
        2,
        Work::Request,
    ),
];

/// Dispatch a parsed request against the engine, returning the result value or
/// a typed [`RpcError`].
///
/// # Errors
///
/// Returns [`RpcError`] for unknown methods, missing/invalid params, patch
/// parse failures, or unknown sessions.
pub fn dispatch(request: &Request, engine: &Engine) -> Result<Value, RpcError> {
    let version = env!("CARGO_PKG_VERSION");
    // `METHODS` is the authority: a request is dispatched on its `METHODS` entry, so a name it does
    // not list is never dispatched and an arm added below without an entry is unreachable rather
    // than undiscoverable (council sb1).
    let Some(spec) = METHODS.iter().find(|m| m.name == request.method) else {
        return Err(RpcError::method_not_found(&request.method));
    };
    refuse_unknown_params(spec, &request.params)?;
    match spec.name {
        "engine.initialize" => {
            // Versions are additive, so any version up to ours is served, and the answer lists the
            // methods that version has. A newer one, or a non-integer, is refused rather than
            // silently answered in another dialect.
            let speaks = match request.params.get("protocol") {
                None => PROTOCOL_VERSION,
                Some(Value::Number(n))
                    if n.as_u64().is_some_and(|n| n <= u64::from(PROTOCOL_VERSION)) =>
                {
                    u32::try_from(n.as_u64().unwrap_or_default()).unwrap_or(PROTOCOL_VERSION)
                }
                Some(other) => {
                    return Err(RpcError::new(
                        PROTOCOL_UNSUPPORTED,
                        format!(
                            "protocol {other} is not supported; this daemon speaks 0 to {PROTOCOL_VERSION}"
                        ),
                    ));
                }
            };
            Ok(json!({
                "server": "deep-diff-forge",
                "version": version,
                "protocol": speaks,
                "methods": METHODS.iter().filter(|m| m.since <= speaks).map(|m| m.name).collect::<Vec<_>>(),
            }))
        }
        "engine.methods" => Ok(json!({
            "protocol": PROTOCOL_VERSION,
            "methods": METHODS
                .iter()
                .map(|m| json!({"name": m.name, "params": m.params, "result": m.result, "since": m.since}))
                .collect::<Vec<_>>(),
        })),
        "daemon.health" => Ok(health(engine, version)),
        "daemon.status" => Ok(json!({
            "running": engine.is_running(),
            "uptime_secs": engine.started.elapsed().as_secs(),
            "sessions": engine.session_count(),
            "version": version,
            "protocol": PROTOCOL_VERSION,
        })),
        "daemon.shutdown" => {
            engine.running.store(false, Ordering::Release);
            Ok(json!({"stopping": true}))
        }
        "diff.plan" => {
            let files = parse_patch_param(request)?;
            Ok(ranked_json(&rank(&files)))
        }
        "diff.compute" => review_document(&sealed(request)?),
        "diff.rank" => Ok(rank_document(&sealed(request)?)),
        "review.change" => review_change(&request.params, &engine.allowed_repos),
        "review.outline" => {
            let page = page_param(&request.params)?;
            crate::views::outline(&sealed(request)?, &page).map_err(|e| view_error(&e))
        }
        "review.hunks" => {
            let selection = selection_param(&request.params)?;
            let page = page_param(&request.params)?;
            crate::views::hunks(&sealed(request)?, &selection, &page).map_err(|e| view_error(&e))
        }
        "session.open" => session_open(request, engine),
        "session.snapshot" => {
            let id = str_param(&request.params, "session")?;
            // The table is released at the end of this statement; ranking runs outside it.
            let files = engine.sessions().snapshot_files(&id);
            match files {
                Some(files) => Ok(ranked_json(&rank(&files))),
                None => Err(RpcError::new(
                    SESSION_NOT_FOUND,
                    format!(
                        "no such session: {}",
                        deep_diff_forge_core::display_safe_bounded(&id, 256)
                    ),
                )),
            }
        }
        "session.close" => {
            let id = str_param(&request.params, "session")?;
            let closed = engine.sessions().close_session(&id);
            if closed {
                engine.metrics.session_closed();
            }
            Ok(json!({"closed": closed}))
        }
        "findings.verify" => findings_verify(request),
        "daemon.metrics" => Ok(engine.metrics.document(engine.started.elapsed(), version)),
        other => Err(RpcError::method_not_found(other)),
    }
}

/// `session.open`: parse the patch outside the session lock, then retain it, counting the
/// sessions it evicted to make room.
fn session_open(request: &Request, engine: &Engine) -> Result<Value, RpcError> {
    let files = parse_patch_param(request)?;
    let count = files.len();
    let payload = SessionPayload::new(files);
    let (id, evicted, retained) = {
        let mut table = engine.sessions();
        let before = table.entries.len();
        let id = table.open_session(payload)?;
        (id, before + 1 - table.entries.len(), table.retained_bytes)
    };
    engine.metrics.session_opened(retained);
    engine.metrics.sessions_evicted(evicted);
    Ok(json!({"session": id, "files": count}))
}

/// `daemon.health`: one read of the session table, so the count and the bytes describe the same
/// moment.
fn health(engine: &Engine, version: &str) -> Value {
    let (sessions, retained_bytes) = {
        let table = engine.sessions();
        (table.entries.len(), table.retained_bytes)
    };
    json!({
        "status": "ok",
        "version": version,
        "pid": engine.pid,
        "sessions": sessions,
        "retained_bytes": retained_bytes,
        // Reserved: there is no response cache, so the count is truly 0 (decision D-03).
        "cache_entries": 0,
        "protocol": PROTOCOL_VERSION,
    })
}

/// The patch param sealed under the request's input contracts (`InputGuards`): the one parse →
/// guard → seal path (anchor C-05) the CLI's `--stdin-patch` uses, so `diff.compute` and
/// `diff.rank` answer for exactly the bytes they were sent.
fn sealed(request: &Request) -> Result<deep_diff_forge_patch::SealedReview, RpcError> {
    let patch = str_param(&request.params, "patch")?;
    let guards = deep_diff_forge_patch::InputGuards {
        files: bool_param(&request.params, "require_files")?,
        hunks: bool_param(&request.params, "require_hunks")?,
    };
    deep_diff_forge_patch::SealedReview::new(patch.as_bytes(), guards, env!("CARGO_PKG_VERSION"))
        .map_err(|error| match error {
            deep_diff_forge_patch::ReviewError::Parse(e) => {
                RpcError::new(PATCH_PARSE_FAILED, e.to_string())
            }
            refused @ deep_diff_forge_patch::ReviewError::Refused(_) => {
                RpcError::new(INPUT_CONTRACT_REFUSED, refused.to_string())
            }
        })
}

/// `diff.compute`: the review document the CLI's `--stdin-patch --json` prints for the same patch.
fn review_document(review: &deep_diff_forge_patch::SealedReview) -> Result<Value, RpcError> {
    serde_json::from_str(&review.to_json()).map_err(|e| {
        RpcError::new(
            crate::protocol::INTERNAL_ERROR,
            format!("review document is not JSON: {e}"),
        )
    })
}

/// An optional boolean param: absent is `false`; any other type is invalid.
/// `findings.verify`: check agent findings (`findings`, one object per finding) against the
/// sealed patch. The findings are hashed as the JSONL they make: one compact object per line,
/// keys sorted, UTF-8 unescaped, each line ending in a newline. The CLI hashes a file's exact
/// bytes, so a file in that form gives the same `findings_sha256`.
fn findings_verify(request: &Request) -> Result<Value, RpcError> {
    use deep_diff_forge_agent::findings::{MAX_FINDINGS_BYTES, document, verify};
    let review = sealed(request)?;
    let Some(Value::Array(items)) = request.params.get("findings") else {
        return Err(RpcError::invalid_params(
            "findings must be an array of finding objects",
        ));
    };
    let lines: Vec<String> = items.iter().map(Value::to_string).collect();
    let bytes = lines.iter().map(|l| l.len() + 1).sum::<usize>();
    if bytes > MAX_FINDINGS_BYTES {
        return Err(RpcError::new(
            RESOURCE_EXHAUSTED,
            format!(
                "findings are {bytes} bytes as JSONL, over the {MAX_FINDINGS_BYTES}-byte limit"
            ),
        ));
    }
    let mut jsonl = String::with_capacity(bytes);
    for line in &lines {
        jsonl.push_str(line);
        jsonl.push('\n');
    }
    let seal = review.seal();
    let verdict = verify(
        review.files(),
        &seal.input_sha256,
        lines.iter().map(String::as_str),
    );
    Ok(document(
        &verdict,
        &seal.input_sha256,
        &deep_diff_forge_core::sha256_hex(jsonl.as_bytes()),
        &seal.tool_version,
    ))
}

fn bool_param(params: &Value, key: &str) -> Result<bool, RpcError> {
    match params.get(key) {
        None => Ok(false),
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => Err(RpcError::invalid_params(format!(
            "param '{key}' must be a boolean"
        ))),
    }
}

fn parse_patch_param(request: &Request) -> Result<Vec<ReviewFile>, RpcError> {
    let patch = str_param(&request.params, "patch")?;
    deep_diff_forge_patch::parse(&patch)
        .map_err(|e| RpcError::new(PATCH_PARSE_FAILED, e.to_string()))
}

/// The most rendered bytes of a request-named path or value that a `review.change` error repeats.
/// Bug hunt 2026-11-01: before, the whole text was repeated, three times its size for bidi
/// characters, and the MCP server, which does not go through `error_response`, sent it whole.
const ECHO_BYTES: usize = 1024;

/// `review.change`: acquire a change from Git in an allowed repository, seal it, and answer with
/// the chosen view. Git runs with the acquisition crate's fixed argv and a 30 s limit; nothing
/// is written. A repository outside every allowed root is refused before Git runs.
fn review_change(params: &Value, allowed: &[std::path::PathBuf]) -> Result<Value, RpcError> {
    use deep_diff_forge_patch::{GitSelection, acquire_git};
    let repo = std::path::PathBuf::from(str_param(params, "repo")?);
    if allowed.is_empty() {
        return Err(RpcError::invalid_params(
            "this server reads no repositories; start it with --allow-repo <path>",
        ));
    }
    let canonical = std::fs::canonicalize(&repo).map_err(|e| {
        RpcError::invalid_params(format!(
            "cannot resolve repo {}: {e}",
            deep_diff_forge_core::display_safe_bounded(&repo.to_string_lossy(), ECHO_BYTES)
        ))
    })?;
    if !allowed.iter().any(|root| canonical.starts_with(root)) {
        return Err(RpcError::invalid_params(format!(
            "repo {} is outside the roots this server may read (--allow-repo)",
            deep_diff_forge_core::display_safe_bounded(&canonical.to_string_lossy(), ECHO_BYTES)
        )));
    }
    let options = change_options(bool_param(params, "include_untracked")?);
    refuse_foreign_git_dirs(&canonical, allowed, &options)?;
    let selection = match str_param(params, "selection")?.as_str() {
        "staged" => GitSelection::Staged,
        "working_tree" => GitSelection::WorkingTree,
        "range" => GitSelection::parse_range(&str_param(params, "range")?)
            .map_err(|e| RpcError::invalid_params(e.to_string()))?,
        other => {
            return Err(RpcError::invalid_params(format!(
                "selection must be staged, working_tree or range, not {}",
                deep_diff_forge_core::display_safe_bounded(other, ECHO_BYTES)
            )));
        }
    };
    let git = |e: deep_diff_forge_patch::GitAcquireError| match e {
        deep_diff_forge_patch::GitAcquireError::InvalidSelection { .. } => {
            RpcError::invalid_params(e.to_string())
        }
        deep_diff_forge_patch::GitAcquireError::Parse(_) => {
            RpcError::new(PATCH_PARSE_FAILED, e.to_string())
        }
        other => RpcError::new(crate::protocol::GIT_FAILED, other.to_string()),
    };
    let acquisition = acquire_git(&canonical, &selection, &options).map_err(git)?;
    // The repository is Git's top level, which can lie above the path asked for: a subdirectory
    // of a larger repository would otherwise expose the whole outer repository's change.
    let top =
        std::fs::canonicalize(acquisition.root()).unwrap_or_else(|_| acquisition.root().into());
    if !allowed.iter().any(|root| top.starts_with(root)) {
        return Err(RpcError::invalid_params(format!(
            "the repository's top level {} is outside the roots this server may read (--allow-repo)",
            deep_diff_forge_core::display_safe_bounded(&top.to_string_lossy(), ECHO_BYTES)
        )));
    }
    let captured = acquisition.diff().capture().map_err(git)?;
    let review = acquisition
        .diff()
        .sealed_review(&captured, env!("CARGO_PKG_VERSION"))
        .map_err(git)?;
    let guards = deep_diff_forge_patch::InputGuards {
        files: bool_param(params, "require_files")?,
        hunks: bool_param(params, "require_hunks")?,
    };
    if let Some(refusal) = guards.refusal(review.files()) {
        let refused = deep_diff_forge_patch::ReviewError::Refused(refusal);
        return Err(RpcError::new(INPUT_CONTRACT_REFUSED, refused.to_string()));
    }
    let mut answer = match params
        .get("view")
        .and_then(Value::as_str)
        .unwrap_or("outline")
    {
        "outline" => {
            crate::views::outline(&review, &page_param(params)?).map_err(|e| view_error(&e))?
        }
        "hunks" => crate::views::hunks(&review, &selection_param(params)?, &page_param(params)?)
            .map_err(|e| view_error(&e))?,
        "review" => review_document(&review)?,
        "rank" => rank_document(&review),
        other => {
            return Err(RpcError::invalid_params(format!(
                "view must be outline, hunks, review or rank, not {}",
                deep_diff_forge_core::display_safe_bounded(other, ECHO_BYTES)
            )));
        }
    };
    answer["acquisition"] = json!({
        "repo": top.to_string_lossy(),
        "selection": params.get("selection"),
        "base": acquisition.base(),
        "target": acquisition.target(),
        "include_untracked": options.include_untracked,
        // An untracked directory Git would not list (a nested repository) is not in the files;
        // only the CLI's stderr said so, and a sealed review looked complete (bug hunt
        // 2026-10-28, H5-03).
        "omitted_untracked": acquisition
            .omitted_untracked()
            .iter()
            .map(deep_diff_forge_patch::GitPath::display_lossy)
            .collect::<Vec<_>>(),
    });
    Ok(answer)
}

/// A param the method does not take is refused, not ignored: a misspelt `require_file` passed an
/// empty review before (bug hunt 2026-10-28, H5-01). The names come from the method's own
/// published params, so `engine.methods` and this check cannot drift apart.
fn refuse_unknown_params(spec: &MethodSpec, params: &Value) -> Result<(), RpcError> {
    if let Some(params) = params.as_object()
        && let Some(unknown) = params
            .keys()
            .find(|key| !spec.param_names().any(|p| p == *key))
    {
        return Err(RpcError::invalid_params(format!(
            "{} takes no param '{}'",
            spec.name,
            deep_diff_forge_core::display_safe_bounded(unknown, 256)
        )));
    }
    Ok(())
}

/// Every directory Git will read must be inside an allowed root, and the object store must not
/// borrow another's: a `.git` file can point a worktree inside the root at a repository outside
/// it (bug hunt 2026-10-28, H2-02).
fn refuse_foreign_git_dirs(
    repo: &std::path::Path,
    allowed: &[std::path::PathBuf],
    options: &deep_diff_forge_patch::GitAcquireOptions,
) -> Result<(), RpcError> {
    let dirs = deep_diff_forge_patch::repository_dirs(repo, options)
        .map_err(|e| RpcError::new(crate::protocol::GIT_FAILED, e.to_string()))?;
    for (what, dir) in [
        ("top level", &dirs.top),
        ("Git directory", &dirs.git_dir),
        ("common Git directory", &dirs.common_dir),
    ] {
        if !allowed.iter().any(|root| dir.starts_with(root)) {
            return Err(RpcError::invalid_params(format!(
                "the repository's {what} {} is outside the roots this server may read (--allow-repo)",
                deep_diff_forge_core::display_safe_bounded(&dir.to_string_lossy(), ECHO_BYTES)
            )));
        }
    }
    if dirs.has_alternates {
        return Err(RpcError::invalid_params(
            "the repository borrows objects from another (objects/info/alternates), which this server does not read",
        ));
    }
    Ok(())
}

/// The acquisition options of `review.change`: the shorter Git time limit, and untracked files
/// only when asked for.
fn change_options(include_untracked: bool) -> deep_diff_forge_patch::GitAcquireOptions {
    deep_diff_forge_patch::GitAcquireOptions {
        include_untracked,
        time_limit: REVIEW_CHANGE_GIT_LIMIT,
        ..deep_diff_forge_patch::GitAcquireOptions::default()
    }
}

/// A refused view is a mistake in the request (budget, cursor, address or path).
fn view_error(error: &crate::views::ViewError) -> RpcError {
    RpcError::invalid_params(error.to_string())
}

/// `max_bytes` and `cursor` for a view; absent means the default budget and the first page.
fn page_param(params: &Value) -> Result<crate::views::Page, RpcError> {
    let max_bytes = match params.get("max_bytes") {
        None => crate::views::DEFAULT_MAX_BYTES,
        Some(value) => value
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .ok_or_else(|| {
                RpcError::invalid_params("param 'max_bytes' must be a non-negative integer")
            })?,
    };
    let cursor = match params.get("cursor") {
        None | Some(Value::Null) => None,
        Some(Value::String(cursor)) => Some(cursor.clone()),
        Some(_) => return Err(RpcError::invalid_params("param 'cursor' must be a string")),
    };
    Ok(crate::views::Page { max_bytes, cursor })
}

/// `addresses` or `paths` (not both) for `review.hunks`; neither means every hunk.
fn selection_param(params: &Value) -> Result<crate::views::Selection, RpcError> {
    let list = |key: &str| -> Result<Option<Vec<String>>, RpcError> {
        match params.get(key) {
            None => Ok(None),
            Some(Value::Array(items)) => items
                .iter()
                .map(|item| {
                    item.as_str().map(str::to_owned).ok_or_else(|| {
                        RpcError::invalid_params(format!(
                            "param '{key}' must be an array of strings"
                        ))
                    })
                })
                .collect::<Result<Vec<_>, _>>()
                .map(Some),
            Some(_) => Err(RpcError::invalid_params(format!(
                "param '{key}' must be an array of strings"
            ))),
        }
    };
    match (list("addresses")?, list("paths")?) {
        (Some(_), Some(_)) => Err(RpcError::invalid_params(
            "pass addresses or paths, not both",
        )),
        (Some(addresses), None) => Ok(crate::views::Selection::Addresses(addresses)),
        (None, Some(paths)) => Ok(crate::views::Selection::Paths(paths)),
        (None, None) => Ok(crate::views::Selection::All),
    }
}

fn str_param(params: &Value, key: &str) -> Result<String, RpcError> {
    params
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| RpcError::invalid_params(format!("missing string param '{key}'")))
}

/// `diff.rank`: the `deep-diff-forge.rank.v0` document the CLI's `--stdin-patch --rank --json`
/// prints for the same patch, sealed like it. `diff.plan` returns only the bare ranking, which
/// cannot say which bytes it ranked; MCP's `rank_patch` answered with that unsealed array.
fn rank_document(review: &deep_diff_forge_patch::SealedReview) -> Value {
    let seal = review.seal();
    json!({
        "schema": "deep-diff-forge.rank.v0",
        "input_sha256": seal.input_sha256,
        "tool": {"name": deep_diff_forge_core::InputSeal::TOOL_NAME, "version": seal.tool_version},
        "ranked": ranked_json(&rank(review.files())),
    })
}

fn ranked_json(ranked: &[RankedFile]) -> Value {
    Value::Array(
        ranked
            .iter()
            .map(|rf| {
                let mut entry = json!({
                    "path": rf.path,
                    "status": rf.status.label(),
                    "score": rf.score,
                    "signals": rf.signals.iter().map(|s| s.label()).collect::<Vec<_>>(),
                });
                // Present only when found, as `hazards` is: most files carry none.
                if !rf.evidence.is_empty() {
                    entry["evidence"] = rf
                        .evidence
                        .iter()
                        .map(|e| {
                            json!({
                                "signal": e.signal.label(),
                                "address": e.address,
                                "side": e.side.label(),
                                "line": e.line,
                                "basis": deep_diff_forge_graph::EVIDENCE_BASIS,
                            })
                        })
                        .collect();
                }
                if rf.evidence_truncated {
                    entry["evidence_truncated"] = json!(true);
                }
                entry
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{METHOD_NOT_FOUND, parse_request};

    fn call(line: &str, engine: &Engine) -> Result<Value, RpcError> {
        let req = parse_request(line)?;
        dispatch(&req, engine)
    }

    /// `review.change` stops Git after 30 s and reads untracked files only when asked.
    #[test]
    fn review_change_options_bound_git() {
        assert_eq!(REVIEW_CHANGE_GIT_LIMIT, std::time::Duration::from_secs(30));
        let on = change_options(true);
        assert_eq!(
            (on.include_untracked, on.time_limit),
            (true, REVIEW_CHANGE_GIT_LIMIT)
        );
        assert!(!change_options(false).include_untracked);
    }

    /// A request holds its line's bytes of the in-flight budget; a Git acquisition holds at least
    /// the patch budget, since Git's output can be larger than its request line.
    #[test]
    fn each_method_weighs_its_work() {
        let budget = deep_diff_forge_patch::DEFAULT_BYTE_BUDGET;
        assert_eq!(Work::Request.weight(123), 123);
        assert_eq!(Work::GitAcquisition.weight(123), budget);
        assert_eq!(Work::GitAcquisition.weight(budget + 1), budget + 1);
        let request = |method: &str| Request {
            jsonrpc: "2.0".into(),
            id: Value::Null,
            method: method.into(),
            params: Value::Null,
        };
        assert_eq!(request_weight(&request("diff.rank"), 77), 77);
        assert_eq!(request_weight(&request("review.change"), 77), budget);
        assert_eq!(request_weight(&request("no.such"), 77), 77);
        let git: Vec<_> = METHODS
            .iter()
            .filter(|m| m.work == Work::GitAcquisition)
            .map(|m| m.name)
            .collect();
        assert_eq!(git, ["review.change"], "only review.change runs Git");
    }

    /// The allowed roots are kept canonical, and a root that does not exist is refused.
    #[test]
    fn allowed_repositories_are_canonical_and_must_exist() {
        let root = std::env::temp_dir().join(format!("ddf-allow-{}", std::process::id()));
        std::fs::create_dir_all(root.join("sub")).unwrap();
        let engine = Engine::new()
            .allowing_repos(&[root.join("sub").join("..")])
            .unwrap();
        assert_eq!(
            engine.allowed_repos,
            [std::fs::canonicalize(&root).unwrap()]
        );
        let missing = root.join("missing");
        let Err((named, _)) = Engine::new().allowing_repos(std::slice::from_ref(&missing)) else {
            panic!("a missing root was allowed");
        };
        assert_eq!(named, missing);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_log_format_is_text_unless_set() {
        use crate::log::LogFormat;
        assert_eq!(Engine::new().log_format(), LogFormat::Text);
        assert_eq!(
            Engine::new().with_log_format(LogFormat::Json).log_format(),
            LogFormat::Json
        );
    }

    #[test]
    fn new_engine_is_running_with_no_sessions() {
        let e = Engine::new();
        assert!(e.is_running());
        assert_eq!(e.session_count(), 0);
    }

    #[test]
    fn health_reports_ok() {
        let e = Engine::new();
        let v = call(r#"{"id":1,"method":"daemon.health"}"#, &e).unwrap();
        assert_eq!(v["status"], "ok");
        assert_eq!(v["protocol"], PROTOCOL_VERSION);
        assert!(v["pid"].as_u64().unwrap() > 0);
    }

    #[test]
    fn initialize_reports_server() {
        let e = Engine::new();
        let v = call(r#"{"method":"engine.initialize"}"#, &e).unwrap();
        assert_eq!(v["server"], "deep-diff-forge");
    }

    #[test]
    fn status_reports_running_and_sessions() {
        let e = Engine::new();
        let v = call(r#"{"method":"daemon.status"}"#, &e).unwrap();
        assert_eq!(v["running"], true);
        assert_eq!(v["sessions"], 0);
    }

    #[test]
    fn shutdown_clears_running() {
        let e = Engine::new();
        let v = call(r#"{"method":"daemon.shutdown"}"#, &e).unwrap();
        assert_eq!(v["stopping"], true);
        assert!(!e.is_running());
    }

    #[test]
    fn unknown_method_is_method_not_found() {
        let e = Engine::new();
        let err = call(r#"{"method":"no.such"}"#, &e).unwrap_err();
        assert_eq!(err.code, METHOD_NOT_FOUND);
    }

    #[test]
    fn diff_plan_ranks_files() {
        let e = Engine::new();
        let line = r#"{"method":"diff.plan","params":{"patch":"--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@\n-a\n+b\n"}}"#;
        let v = call(line, &e).unwrap();
        assert!(v.is_array());
        assert_eq!(v[0]["path"], "src/lib.rs");
        assert!(
            v[0]["signals"]
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s == "public_api_surface")
        );
    }

    #[test]
    fn diff_plan_missing_patch_is_invalid_params() {
        let e = Engine::new();
        let err = call(r#"{"method":"diff.plan","params":{}}"#, &e).unwrap_err();
        assert_eq!(err.code, crate::protocol::INVALID_PARAMS);
    }

    #[test]
    fn diff_plan_bad_patch_is_parse_failed() {
        let e = Engine::new();
        let line = r#"{"method":"diff.plan","params":{"patch":"--- a/x\n+++ b/x\n+stray\n"}}"#;
        let err = call(line, &e).unwrap_err();
        assert_eq!(err.code, PATCH_PARSE_FAILED);
    }

    #[test]
    fn session_open_returns_id_and_count() {
        let e = Engine::new();
        let line = r#"{"method":"session.open","params":{"patch":"--- a/x.rs\n+++ b/x.rs\n@@ -1,1 +1,1 @@\n-a\n+b\n"}}"#;
        let v = call(line, &e).unwrap();
        assert_eq!(v["session"], "s1");
        assert_eq!(v["files"], 1);
        assert_eq!(e.session_count(), 1);
    }

    #[test]
    fn session_ids_increment() {
        let e = Engine::new();
        let line = r#"{"method":"session.open","params":{"patch":"--- a/x.rs\n+++ b/x.rs\n@@ -1,1 +1,1 @@\n-a\n+b\n"}}"#;
        let a = call(line, &e).unwrap();
        let b = call(line, &e).unwrap();
        assert_eq!(a["session"], "s1");
        assert_eq!(b["session"], "s2");
    }

    #[test]
    fn session_snapshot_returns_ranked() {
        let e = Engine::new();
        let open = r#"{"method":"session.open","params":{"patch":"--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@\n-a\n+b\n"}}"#;
        let opened = call(open, &e).unwrap();
        let id = opened["session"].as_str().unwrap();
        let snap = call(
            &format!(r#"{{"method":"session.snapshot","params":{{"session":"{id}"}}}}"#),
            &e,
        )
        .unwrap();
        assert_eq!(snap[0]["path"], "src/lib.rs");
    }

    #[test]
    fn snapshot_unknown_session_is_error() {
        let e = Engine::new();
        let err = call(
            r#"{"method":"session.snapshot","params":{"session":"nope"}}"#,
            &e,
        )
        .unwrap_err();
        assert_eq!(err.code, SESSION_NOT_FOUND);
    }

    #[test]
    fn session_close_removes_session() {
        let e = Engine::new();
        let open = r#"{"method":"session.open","params":{"patch":"--- a/x.rs\n+++ b/x.rs\n@@ -1,1 +1,1 @@\n-a\n+b\n"}}"#;
        let id = call(open, &e).unwrap()["session"]
            .as_str()
            .unwrap()
            .to_string();
        let closed = call(
            &format!(r#"{{"method":"session.close","params":{{"session":"{id}"}}}}"#),
            &e,
        )
        .unwrap();
        assert_eq!(closed["closed"], true);
        assert_eq!(e.session_count(), 0);
    }

    #[test]
    fn closing_unknown_session_is_false_not_error() {
        let e = Engine::new();
        let v = call(
            r#"{"method":"session.close","params":{"session":"ghost"}}"#,
            &e,
        )
        .unwrap();
        assert_eq!(v["closed"], false);
    }

    #[test]
    fn health_session_count_tracks_opens() {
        let e = Engine::new();
        let open = r#"{"method":"session.open","params":{"patch":"--- a/x.rs\n+++ b/x.rs\n@@ -1,1 +1,1 @@\n-a\n+b\n"}}"#;
        call(open, &e).unwrap();
        let v = call(r#"{"method":"daemon.health"}"#, &e).unwrap();
        assert_eq!(v["sessions"], 1);
    }

    #[test]
    fn default_engine_matches_new() {
        let e = Engine::default();
        assert!(e.is_running());
        assert_eq!(e.session_count(), 0);
    }

    #[test]
    fn retained_byte_budget_evicts_lru_sessions() {
        let mut e = Sessions::new();
        let files =
            deep_diff_forge_patch::parse("--- a/x.rs\n+++ b/x.rs\n@@ -1,1 +1,1 @@\n-a\n+b\n")
                .unwrap();
        let one_session = estimated_session_bytes(&files);
        e.max_retained_bytes = one_session;

        let first = e.open_session(SessionPayload::new(files.clone())).unwrap();
        let second = e.open_session(SessionPayload::new(files)).unwrap();
        assert_eq!(e.entries.len(), 1);
        assert!(e.snapshot_files(&first).is_none());
        assert!(e.snapshot_files(&second).is_some());
        assert!(e.retained_bytes <= e.max_retained_bytes);
    }

    #[test]
    fn oversized_session_is_rejected_without_eviction() {
        let mut e = Sessions::new();
        let files =
            deep_diff_forge_patch::parse("--- a/x.rs\n+++ b/x.rs\n@@ -1,1 +1,1 @@\n-a\n+b\n")
                .unwrap();
        e.max_retained_bytes = estimated_session_bytes(&files).saturating_sub(1);
        let err = e.open_session(SessionPayload::new(files)).unwrap_err();
        assert_eq!(err.code, RESOURCE_EXHAUSTED);
        assert_eq!(e.entries.len(), 0);
    }

    #[test]
    fn session_param_must_be_string() {
        let e = Engine::new();
        let err = call(
            r#"{"method":"session.snapshot","params":{"session":123}}"#,
            &e,
        )
        .unwrap_err();
        assert_eq!(err.code, crate::protocol::INVALID_PARAMS);
    }

    #[test]
    fn diff_plan_empty_patch_is_empty_array() {
        let e = Engine::new();
        let v = call(r#"{"method":"diff.plan","params":{"patch":""}}"#, &e).unwrap();
        assert_eq!(v.as_array().unwrap().len(), 0);
    }

    // -------------------------------------------------------------------------
    // F3: bounded LRU session tests (fail-before/pass-after)
    // -------------------------------------------------------------------------

    // Minimal valid patch for session.open.
    const OPEN_LINE: &str = r#"{"method":"session.open","params":{"patch":"--- a/x.rs\n+++ b/x.rs\n@@ -1,1 +1,1 @@\n-a\n+b\n"}}"#;

    // F3-T1: session count is bounded at MAX_SESSIONS.
    // Open MAX_SESSIONS + 5 sessions; the count must stay exactly at the cap.
    #[test]
    fn sessions_are_bounded() {
        let e = Engine::new();
        for _ in 0..(super::MAX_SESSIONS + 5) {
            call(OPEN_LINE, &e).expect("session.open");
        }
        assert_eq!(
            e.session_count(),
            super::MAX_SESSIONS,
            "session_count must not exceed MAX_SESSIONS"
        );
    }

    // F3-T2: LRU eviction retains the most-recently-accessed session.
    // Fill the engine to capacity, then access (snapshot) the first session
    // ("s1") to refresh its tick. Open one more session (triggering eviction).
    // "s1" must survive because it was recently accessed; an untouched session
    // must be the one evicted.
    #[test]
    fn lru_evicts_least_recently_used() {
        let e = Engine::new();
        // Fill to MAX_SESSIONS.
        for _ in 0..super::MAX_SESSIONS {
            call(OPEN_LINE, &e).expect("session.open");
        }
        assert_eq!(e.session_count(), super::MAX_SESSIONS);

        // Access s1 to refresh its LRU tick — it must NOT be evicted next.
        let snap = call(
            r#"{"method":"session.snapshot","params":{"session":"s1"}}"#,
            &e,
        );
        assert!(snap.is_ok(), "s1 snapshot before eviction must succeed");

        // Open one more session — evicts the LRU (not s1).
        call(OPEN_LINE, &e).expect("session.open over cap");
        assert_eq!(e.session_count(), super::MAX_SESSIONS);

        // s1 must still be accessible.
        let snap_after = call(
            r#"{"method":"session.snapshot","params":{"session":"s1"}}"#,
            &e,
        );
        assert!(
            snap_after.is_ok(),
            "s1 must be retained — it was recently accessed and should not have been evicted"
        );
    }

    // F3-T6: eviction removes the TRUE least-recently-used session, not merely
    // "some" session. Refresh every session except a known victim so the victim
    // alone holds the minimum tick, then prove it is the exact one evicted. (A
    // buggy "evict last-inserted" algorithm would pass F3-T2 but fail this.)
    #[test]
    fn lru_evicts_the_true_least_recently_used() {
        let e = Engine::new();
        let mut ids = Vec::new();
        for _ in 0..super::MAX_SESSIONS {
            let v = call(OPEN_LINE, &e).expect("open");
            ids.push(v["session"].as_str().unwrap().to_string());
        }
        // The 2nd-opened session is the victim; refresh every OTHER session so the
        // victim alone holds the lowest access tick.
        let victim = ids[1].clone();
        for id in &ids {
            if id == &victim {
                continue;
            }
            call(
                &format!(r#"{{"method":"session.snapshot","params":{{"session":"{id}"}}}}"#),
                &e,
            )
            .expect("refresh");
        }
        // Open one more — must evict exactly the victim (the true min-tick entry).
        call(OPEN_LINE, &e).expect("open over cap");
        assert_eq!(e.session_count(), super::MAX_SESSIONS);
        let victim_snap = call(
            &format!(r#"{{"method":"session.snapshot","params":{{"session":"{victim}"}}}}"#),
            &e,
        );
        assert!(
            victim_snap.is_err(),
            "the true least-recently-used session must be the one evicted"
        );
        // A refreshed session (the first-opened) must still be present.
        let survivor = call(
            &format!(
                r#"{{"method":"session.snapshot","params":{{"session":"{}"}}}}"#,
                ids[0]
            ),
            &e,
        );
        assert!(
            survivor.is_ok(),
            "a refreshed session must survive eviction"
        );
    }

    // F3-T3: session count never exceeds the cap across many opens.
    // A tight loop opening 3× MAX_SESSIONS sessions; at no point between opens
    // can the count exceed MAX_SESSIONS.
    #[test]
    fn session_count_never_exceeds_cap() {
        let e = Engine::new();
        for _ in 0..(super::MAX_SESSIONS * 3) {
            call(OPEN_LINE, &e).expect("session.open");
            assert!(
                e.session_count() <= super::MAX_SESSIONS,
                "session_count() exceeded MAX_SESSIONS after an open"
            );
        }
    }

    // F3-T4: closed sessions do not count toward the cap.
    // Open to near-cap, close all, then verify re-opening works without spurious
    // eviction of valid sessions.
    #[test]
    fn closed_sessions_free_capacity() {
        let e = Engine::new();
        let mut ids = Vec::new();
        // Open MAX_SESSIONS sessions.
        for _ in 0..super::MAX_SESSIONS {
            let v = call(OPEN_LINE, &e).expect("open");
            ids.push(v["session"].as_str().unwrap().to_string());
        }
        assert_eq!(e.session_count(), super::MAX_SESSIONS);
        // Close all.
        for id in &ids {
            let v = call(
                &format!(r#"{{"method":"session.close","params":{{"session":"{id}"}}}}"#),
                &e,
            )
            .expect("close");
            assert_eq!(v["closed"], true);
        }
        assert_eq!(e.session_count(), 0);
        // Now we can open MAX_SESSIONS more without evictions.
        for _ in 0..super::MAX_SESSIONS {
            call(OPEN_LINE, &e).expect("re-open after close");
        }
        assert_eq!(e.session_count(), super::MAX_SESSIONS);
    }

    // Single-file patch whose one added line is `line_bytes` long.
    fn patch_with_line(line_bytes: usize) -> Vec<ReviewFile> {
        let patch = format!(
            "--- a/x.rs\n+++ b/x.rs\n@@ -1,1 +1,1 @@\n-a\n+{}\n",
            "b".repeat(line_bytes)
        );
        deep_diff_forge_patch::parse(&patch).unwrap()
    }

    #[test]
    fn default_retained_budget_is_128_mib() {
        assert_eq!(MAX_RETAINED_SESSION_BYTES, 128 << 20);
        assert_eq!(Sessions::new().max_retained_bytes, 128 << 20);
    }

    #[test]
    fn default_budget_accepts_multi_mebibyte_session() {
        // Well under 128 MiB, but over any accidentally-additive budget (~1 MiB).
        let mut e = Sessions::new();
        e.open_session(SessionPayload::new(patch_with_line(4 << 20)))
            .expect("a 4 MiB session fits the default budget");
        assert_eq!(e.entries.len(), 1);
    }

    #[test]
    fn estimate_counts_retained_line_text() {
        let files = patch_with_line(10_000);
        assert!(estimated_session_bytes(&files) >= 10_000);
        let mut e = Sessions::new();
        e.max_retained_bytes = 4096;
        let err = e.open_session(SessionPayload::new(files)).unwrap_err();
        assert_eq!(err.code, RESOURCE_EXHAUSTED);
    }

    #[test]
    fn sessions_exactly_filling_the_byte_budget_are_all_retained() {
        let mut e = Sessions::new();
        // Parse each session separately: a clone may shrink vector capacities.
        e.max_retained_bytes = 2 * estimated_session_bytes(&patch_with_line(1));
        let first = e
            .open_session(SessionPayload::new(patch_with_line(1)))
            .unwrap();
        let second = e
            .open_session(SessionPayload::new(patch_with_line(1)))
            .unwrap();
        assert_eq!(e.retained_bytes, e.max_retained_bytes);
        assert!(
            e.snapshot_files(&first).is_some(),
            "at-budget must not evict"
        );
        assert!(e.snapshot_files(&second).is_some());
    }

    #[test]
    fn byte_budget_evicts_as_many_sessions_as_needed() {
        let mut e = Sessions::new();
        let large = patch_with_line(4096);
        let large_bytes = estimated_session_bytes(&large);
        assert!(large_bytes > estimated_session_bytes(&patch_with_line(1)));
        e.open_session(SessionPayload::new(patch_with_line(1)))
            .unwrap();
        e.open_session(SessionPayload::new(patch_with_line(1)))
            .unwrap();
        // Room for the large session only once BOTH small sessions are gone.
        e.max_retained_bytes = large_bytes;
        let kept = e.open_session(SessionPayload::new(large)).unwrap();
        assert_eq!(e.entries.len(), 1);
        assert!(e.snapshot_files(&kept).is_some());
        assert_eq!(e.retained_bytes, large_bytes);
    }

    #[test]
    fn ticks_increase_monotonically() {
        let mut e = Sessions::new();
        assert_eq!(e.next_tick(), 1);
        assert_eq!(e.next_tick(), 2);
        e.open_session(SessionPayload::new(patch_with_line(1)))
            .unwrap();
        assert_eq!(e.next_tick(), 4);
    }

    #[test]
    fn tick_overflow_rebases_densely_and_preserves_lru_order() {
        let mut e = Sessions::new();
        let a = e
            .open_session(SessionPayload::new(patch_with_line(1)))
            .unwrap();
        let b = e
            .open_session(SessionPayload::new(patch_with_line(1)))
            .unwrap();
        let c = e
            .open_session(SessionPayload::new(patch_with_line(1)))
            .unwrap();
        // Recency order: b (oldest), c, a (newest).
        e.entries.get_mut(&b).unwrap().last_tick = u64::MAX - 30;
        e.entries.get_mut(&c).unwrap().last_tick = u64::MAX - 20;
        e.entries.get_mut(&a).unwrap().last_tick = u64::MAX - 10;
        e.tick = u64::MAX;

        assert_eq!(e.next_tick(), 4);
        assert_eq!(e.entries[&b].last_tick, 1);
        assert_eq!(e.entries[&c].last_tick, 2);
        assert_eq!(e.entries[&a].last_tick, 3);
        assert!(e.evict_lru());
        assert!(!e.entries.contains_key(&b), "oldest entry is still the LRU");
    }

    // F3-T5: snapshot on a non-existent session is still SESSION_NOT_FOUND even
    // when the engine is at cap (eviction must not swallow the error).
    #[test]
    fn snapshot_missing_session_at_cap_is_error() {
        let e = Engine::new();
        for _ in 0..super::MAX_SESSIONS {
            call(OPEN_LINE, &e).expect("open");
        }
        let err = call(
            r#"{"method":"session.snapshot","params":{"session":"ghost"}}"#,
            &e,
        )
        .unwrap_err();
        assert_eq!(err.code, SESSION_NOT_FOUND);
    }

    #[test]
    fn every_listed_method_is_dispatched_and_nothing_else_is() {
        // An unlisted name is refused before the dispatcher's arms are consulted.
        let probe = Engine::new();
        for unlisted in ["engine.shutdown", "session.list", "diff.Compute", ""] {
            let request = Request {
                jsonrpc: "2.0".into(),
                id: Value::Null,
                method: unlisted.into(),
                params: Value::Null,
            };
            assert_eq!(
                dispatch(&request, &probe).unwrap_err().code,
                crate::protocol::METHOD_NOT_FOUND,
                "{unlisted}"
            );
        }
        for spec in METHODS {
            let e = Engine::new();
            let line = format!(r#"{{"method":"{}"}}"#, spec.name);
            if let Err(err) = call(&line, &e) {
                assert_ne!(err.code, crate::protocol::METHOD_NOT_FOUND, "{}", spec.name);
            }
            assert!(spec.since <= PROTOCOL_VERSION, "{}", spec.name);
        }
        let names: Vec<&str> = METHODS.iter().map(|m| m.name).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len());
        let v = call(r#"{"method":"engine.methods"}"#, &Engine::new()).unwrap();
        assert_eq!(v["methods"].as_array().unwrap().len(), METHODS.len());
        assert_eq!(v["methods"][6]["name"], "diff.compute");
        let v = call(r#"{"method":"engine.initialize"}"#, &Engine::new()).unwrap();
        assert_eq!(v["methods"].as_array().unwrap().len(), METHODS.len());
    }

    /// The README names every method the daemon implements, in `engine.methods` order, and the
    /// protocol `daemon.health` reports. Before, it omitted `engine.methods` and `diff.compute`
    /// and showed protocol 0.
    #[test]
    fn the_readme_lists_the_methods_and_protocol() {
        let readme = include_str!("../../../README.md");
        let start = readme.find("**JSON-RPC methods**").unwrap();
        let paragraph = &readme[start..start + readme[start..].find("\n\n").unwrap()];
        let names: Vec<String> = paragraph
            .split(':')
            .nth(1)
            .unwrap()
            .split('`')
            .skip(1)
            .step_by(2)
            .map(str::to_owned)
            .collect();
        let methods: Vec<&str> = METHODS.iter().map(|m| m.name).collect();
        assert_eq!(names, methods);
        assert!(readme.contains(&format!("\"protocol\":{PROTOCOL_VERSION},")));
    }

    /// `findings.verify` takes an array, hashes it as compact sorted-key JSONL, and refuses one
    /// whose JSONL is over the limit with `RESOURCE_EXHAUSTED`.
    #[test]
    fn findings_verify_checks_its_input_and_hashes_the_jsonl() {
        use deep_diff_forge_agent::findings::MAX_FINDINGS_BYTES;
        let e = Engine::new();
        let patch = "--- a/x\n+++ b/x\n@@ -1,1 +1,1 @@\n-a\n+b\n";
        let finding =
            json!({"agent": "a", "severity": "low", "title": "t", "body": "b", "path": "x"});
        let request = |findings: Value| {
            let line = json!({"method": "findings.verify", "params": {"patch": patch, "findings": findings}});
            call(&line.to_string(), &e)
        };
        let v = request(json!([finding])).unwrap();
        assert_eq!(
            v["counts"],
            json!({"accepted": 1, "refused": 0, "over_limit": 0})
        );
        let jsonl = format!("{finding}\n");
        assert_eq!(
            v["findings_sha256"],
            deep_diff_forge_core::sha256_hex(jsonl.as_bytes())
        );
        assert_eq!(
            v["input_sha256"],
            deep_diff_forge_core::sha256_hex(patch.as_bytes())
        );
        assert_eq!(
            request(json!("x")).unwrap_err().code,
            crate::protocol::INVALID_PARAMS
        );
        // One object whose line, with its newline, is exactly the limit is read; one byte more is
        // refused.
        let at_limit = |extra: usize| {
            let mut f = finding.clone();
            f["body"] = json!("");
            let base = f.to_string().len() + 1;
            f["body"] = json!("b".repeat(MAX_FINDINGS_BYTES - base + extra));
            request(json!([f]))
        };
        assert!(at_limit(0).is_ok());
        assert_eq!(at_limit(1).unwrap_err().code, RESOURCE_EXHAUSTED);
    }

    #[test]
    fn initialize_refuses_a_protocol_it_does_not_speak() {
        let e = Engine::new();
        assert_eq!(
            call(
                r#"{"method":"engine.initialize","params":{"protocol":1}}"#,
                &e
            )
            .unwrap()["protocol"],
            1
        );
        let v0 = call(
            r#"{"method":"engine.initialize","params":{"protocol":0}}"#,
            &e,
        )
        .unwrap();
        assert_eq!(v0["protocol"], 0);
        let names: Vec<&str> = v0["methods"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m.as_str().unwrap())
            .collect();
        assert!(
            names.contains(&"diff.plan")
                && !names.contains(&"diff.compute")
                && !names.contains(&"engine.methods"),
            "{names:?}"
        );
        // Protocol 1 does not list what protocol 2 added; protocol 2 does.
        let v1 = call(
            r#"{"method":"engine.initialize","params":{"protocol":1}}"#,
            &e,
        )
        .unwrap();
        assert!(
            !v1["methods"]
                .as_array()
                .unwrap()
                .contains(&json!("findings.verify"))
        );
        let v2 = call(
            r#"{"method":"engine.initialize","params":{"protocol":2}}"#,
            &e,
        )
        .unwrap();
        assert_eq!(v2["protocol"], 2);
        assert!(
            v2["methods"]
                .as_array()
                .unwrap()
                .contains(&json!("findings.verify"))
        );
        for other in ["3", "\"1\"", "1.5", "null", "-1"] {
            let line =
                format!(r#"{{"method":"engine.initialize","params":{{"protocol":{other}}}}}"#);
            assert_eq!(
                call(&line, &e).unwrap_err().code,
                PROTOCOL_UNSUPPORTED,
                "{other}"
            );
        }
    }

    #[test]
    fn compute_applies_the_input_contracts_and_reports_typed_errors() {
        let e = Engine::new();
        let ok = r#"{"method":"diff.compute","params":{"patch":"--- a/x\n+++ b/x\n@@ -1,1 +1,1 @@\n-a\n+b\n","require_files":true,"require_hunks":true}}"#;
        let v = call(ok, &e).unwrap();
        assert_eq!(v["schema"], "deep-diff-forge.review.v0");
        assert_eq!(v["files"].as_array().unwrap().len(), 1);
        let empty = call(
            r#"{"method":"diff.compute","params":{"patch":"","require_files":true}}"#,
            &e,
        )
        .unwrap_err();
        assert_eq!(
            (empty.code, empty.message.as_str()),
            (
                INPUT_CONTRACT_REFUSED,
                "refused: 0 files in input (--require-files)"
            )
        );
        let header = call(r#"{"method":"diff.compute","params":{"patch":"diff --git a/x b/x\n","require_hunks":true}}"#, &e).unwrap_err();
        assert_eq!(
            (header.code, header.message.as_str()),
            (
                INPUT_CONTRACT_REFUSED,
                "refused: 0 hunks in input (--require-hunks)"
            )
        );
        assert!(call(r#"{"method":"diff.compute","params":{"patch":""}}"#, &e).is_ok());
        let bad = call(
            r#"{"method":"diff.compute","params":{"patch":"","require_files":"yes"}}"#,
            &e,
        )
        .unwrap_err();
        assert_eq!(bad.code, crate::protocol::INVALID_PARAMS);
        let garbage = call(
            r#"{"method":"diff.compute","params":{"patch":"--- a/x\n+++ b/x\n+stray\n"}}"#,
            &e,
        )
        .unwrap_err();
        assert_eq!(garbage.code, PATCH_PARSE_FAILED);
        assert_eq!(
            call(r#"{"method":"diff.compute","params":{}}"#, &e)
                .unwrap_err()
                .code,
            crate::protocol::INVALID_PARAMS
        );
    }
}
