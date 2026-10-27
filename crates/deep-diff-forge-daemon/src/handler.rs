// DDF review route: DDF-R26, DDF-R27, DDF-R29. Status and remaining work are recorded in the linked ledger; this comment does not claim delivery.
// Design/status: docs/RECOMMENDATION_INTEGRATION.md#remaining-module-routes
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fcrates--deep-diff-forge-daemon--src--handler
use crate::protocol::{
    INPUT_CONTRACT_REFUSED, PATCH_PARSE_FAILED, PROTOCOL_UNSUPPORTED, PROTOCOL_VERSION,
    RESOURCE_EXHAUSTED, Request, RpcError, SESSION_NOT_FOUND,
};
use deep_diff_forge_core::{PatchHunk, PatchLine, ReviewFile, SemanticSpan};
use deep_diff_forge_graph::{RankedFile, rank};
use serde_json::{Value, json};
use std::collections::HashMap;
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
const MAX_SESSIONS: usize = 64;

/// Maximum tracked heap payload retained across all sessions.
///
/// A session-count limit alone permits `MAX_SESSIONS` near-64 MiB patches to
/// retain several GiB. The byte budget makes retention independent of request
/// count; LRU sessions are evicted until both limits are satisfied.
const MAX_RETAINED_SESSION_BYTES: usize = 128 * 1024 * 1024;

/// Per-session state stored inside the engine.
struct SessionEntry {
    files: Vec<ReviewFile>,
    /// Estimated heap allocation held by `files` and its nested collections.
    retained_bytes: usize,
    /// Monotonically increasing access tick; updated on open and on snapshot.
    /// The session with the smallest tick is the least-recently-used candidate
    /// for eviction when the cap is reached.
    last_tick: u64,
}

/// Daemon engine: review sessions, counters, and lifecycle state.
///
/// Shared across connection threads behind a `Mutex` by the server; the methods
/// here are the pure, testable core (no socket I/O).
///
/// Session retention is bounded at `MAX_SESSIONS`. When capacity is reached,
/// opening a new session evicts the least-recently-used (LRU) entry — the one
/// with the smallest `last_tick` value — before inserting the new one.
pub struct Engine {
    sessions: HashMap<String, SessionEntry>,
    next_session: u64,
    /// Monotonically increasing counter incremented on every session open or
    /// snapshot access; used for LRU ordering.
    tick: u64,
    retained_bytes: usize,
    max_retained_bytes: usize,
    running: bool,
    pid: u32,
    started: Instant,
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine {
    /// Create a fresh engine bound to the current process.
    #[must_use]
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
            next_session: 1,
            tick: 0,
            retained_bytes: 0,
            max_retained_bytes: MAX_RETAINED_SESSION_BYTES,
            running: true,
            pid: std::process::id(),
            started: Instant::now(),
        }
    }

    /// Drop every session after a contained panic (anchor A-03). Whatever dispatch was doing may have
    /// left the session table and its byte accounting inconsistent, so they start again empty; the
    /// pid, start time, `running` and the session-id counter are kept, so no old id is reused.
    pub(crate) fn reset_sessions(&mut self) {
        self.sessions.clear();
        self.retained_bytes = 0;
        self.tick = 0;
    }

    /// Whether the engine is still serving (cleared by `daemon.shutdown`).
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.running
    }

    /// Number of open sessions (always `<= MAX_SESSIONS`).
    #[must_use]
    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    /// Advance the global tick and return the new value.
    fn next_tick(&mut self) -> u64 {
        if self.tick == u64::MAX {
            // Rebase the at-most-64 entries while preserving their order, so a
            // multi-decade daemon cannot panic or invert LRU ordering.
            let mut ordered: Vec<(String, u64)> = self
                .sessions
                .iter()
                .map(|(id, entry)| (id.clone(), entry.last_tick))
                .collect();
            ordered.sort_by_key(|(_, tick)| *tick);
            for (index, (id, _)) in ordered.into_iter().enumerate() {
                if let Some(entry) = self.sessions.get_mut(&id) {
                    entry.last_tick = index as u64 + 1;
                }
            }
            self.tick = self.sessions.len() as u64;
        }
        self.tick += 1;
        self.tick
    }

    /// Evict the least-recently-used session, returning whether one existed.
    fn evict_lru(&mut self) -> bool {
        // Find the key with the smallest last_tick (LRU).
        let lru_key = self
            .sessions
            .iter()
            .min_by_key(|(_, entry)| entry.last_tick)
            .map(|(key, _)| key.clone());
        if let Some(key) = lru_key
            && let Some(removed) = self.sessions.remove(&key)
        {
            self.retained_bytes = self.retained_bytes.saturating_sub(removed.retained_bytes);
            return true;
        }
        false
    }

    fn open_session(&mut self, files: Vec<ReviewFile>) -> Result<String, RpcError> {
        let retained_bytes = estimated_session_bytes(&files);
        if retained_bytes > self.max_retained_bytes {
            return Err(RpcError::new(
                RESOURCE_EXHAUSTED,
                "session exceeds the retained-memory budget",
            ));
        }
        while self.sessions.len() >= MAX_SESSIONS
            || self.retained_bytes.saturating_add(retained_bytes) > self.max_retained_bytes
        {
            if !self.evict_lru() {
                break;
            }
        }

        // Wrap safely and skip any still-live id after the theoretical wrap.
        let id = loop {
            let candidate = format!("s{}", self.next_session);
            self.next_session = self.next_session.checked_add(1).unwrap_or(1);
            if !self.sessions.contains_key(&candidate) {
                break candidate;
            }
        };
        let tick = self.next_tick();
        self.retained_bytes = self.retained_bytes.saturating_add(retained_bytes);
        self.sessions.insert(
            id.clone(),
            SessionEntry {
                files,
                retained_bytes,
                last_tick: tick,
            },
        );
        Ok(id)
    }

    /// Rank a session in place, refreshing the LRU tick without cloning its
    /// potentially large parsed patch model.
    fn snapshot_ranked(&mut self, id: &str) -> Option<Vec<RankedFile>> {
        let tick = self.next_tick();
        if let Some(entry) = self.sessions.get_mut(id) {
            entry.last_tick = tick;
            Some(rank(&entry.files))
        } else {
            None
        }
    }

    fn close_session(&mut self, id: &str) -> bool {
        if let Some(removed) = self.sessions.remove(id) {
            self.retained_bytes = self.retained_bytes.saturating_sub(removed.retained_bytes);
            true
        } else {
            false
        }
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
}

const fn method(
    name: &'static str,
    params: &'static str,
    result: &'static str,
    since: u32,
) -> MethodSpec {
    MethodSpec {
        name,
        params,
        result,
        since,
    }
}

/// Every method [`dispatch`] answers; a test keeps this list and the dispatcher in step.
pub const METHODS: &[MethodSpec] = &[
    method(
        "engine.initialize",
        "protocol?: integer",
        "{server, version, protocol, methods}",
        0,
    ),
    method(
        "engine.methods",
        "",
        "{protocol, methods: [{name, params, result, since}]}",
        1,
    ),
    method(
        "daemon.health",
        "",
        "{status, version, pid, sessions, retained_bytes, cache_entries, protocol}",
        0,
    ),
    method(
        "daemon.status",
        "",
        "{running, uptime_secs, sessions, version, protocol}",
        0,
    ),
    method("daemon.shutdown", "", "{stopping}", 0),
    method("diff.plan", "patch: string", "[ranked file]", 0),
    method(
        "diff.compute",
        "patch: string, require_files?: boolean, require_hunks?: boolean",
        "the deep-diff-forge.review.v0 document `deep-diff-forge --stdin-patch --json` prints for the same patch",
        1,
    ),
    method("session.open", "patch: string", "{session, files}", 0),
    method("session.snapshot", "session: string", "[ranked file]", 0),
    method("session.close", "session: string", "{closed}", 0),
];

/// Dispatch a parsed request against the engine, returning the result value or
/// a typed [`RpcError`].
///
/// # Errors
///
/// Returns [`RpcError`] for unknown methods, missing/invalid params, patch
/// parse failures, or unknown sessions.
pub fn dispatch(request: &Request, engine: &mut Engine) -> Result<Value, RpcError> {
    let version = env!("CARGO_PKG_VERSION");
    // `METHODS` is the authority: a request is dispatched on its `METHODS` entry, so a name it does
    // not list is never dispatched and an arm added below without an entry is unreachable rather
    // than undiscoverable (council sb1).
    let Some(spec) = METHODS.iter().find(|m| m.name == request.method) else {
        return Err(RpcError::method_not_found(&request.method));
    };
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
        "daemon.health" => Ok(json!({
            "status": "ok",
            "version": version,
            "pid": engine.pid,
            "sessions": engine.session_count(),
            "retained_bytes": engine.retained_bytes,
            "cache_entries": 0,
            "protocol": PROTOCOL_VERSION,
        })),
        "daemon.status" => Ok(json!({
            "running": engine.running,
            "uptime_secs": engine.started.elapsed().as_secs(),
            "sessions": engine.session_count(),
            "version": version,
            "protocol": PROTOCOL_VERSION,
        })),
        "daemon.shutdown" => {
            engine.running = false;
            Ok(json!({"stopping": true}))
        }
        "diff.plan" => {
            let files = parse_patch_param(request)?;
            Ok(ranked_json(&rank(&files)))
        }
        "diff.compute" => compute(request),
        "session.open" => {
            let files = parse_patch_param(request)?;
            let count = files.len();
            let id = engine.open_session(files)?;
            Ok(json!({"session": id, "files": count}))
        }
        "session.snapshot" => {
            let id = str_param(&request.params, "session")?;
            match engine.snapshot_ranked(&id) {
                Some(ranked) => Ok(ranked_json(&ranked)),
                None => Err(RpcError::new(
                    SESSION_NOT_FOUND,
                    format!("no such session: {id}"),
                )),
            }
        }
        "session.close" => {
            let id = str_param(&request.params, "session")?;
            Ok(json!({"closed": engine.close_session(&id)}))
        }
        other => Err(RpcError::method_not_found(other)),
    }
}

/// `diff.compute`: the review document the CLI's `--stdin-patch --json` prints for the same
/// patch, sealed over the same bytes, under the same input contracts (`InputGuards`).
fn compute(request: &Request) -> Result<Value, RpcError> {
    let patch = str_param(&request.params, "patch")?;
    let guards = deep_diff_forge_patch::InputGuards {
        files: bool_param(&request.params, "require_files")?,
        hunks: bool_param(&request.params, "require_hunks")?,
    };
    let files = deep_diff_forge_patch::parse(&patch)
        .map_err(|e| RpcError::new(PATCH_PARSE_FAILED, e.to_string()))?;
    if let Some(reason) = guards.refusal(&files) {
        return Err(RpcError::new(
            INPUT_CONTRACT_REFUSED,
            format!("refused: {reason}"),
        ));
    }
    let seal = deep_diff_forge_core::InputSeal::of(patch.as_bytes(), env!("CARGO_PKG_VERSION"));
    serde_json::from_str(&deep_diff_forge_patch::to_json_sealed(&files, &seal)).map_err(|e| {
        RpcError::new(
            crate::protocol::INTERNAL_ERROR,
            format!("review document is not JSON: {e}"),
        )
    })
}

/// An optional boolean param: absent is `false`; any other type is invalid.
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

fn str_param(params: &Value, key: &str) -> Result<String, RpcError> {
    params
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| RpcError::invalid_params(format!("missing string param '{key}'")))
}

fn ranked_json(ranked: &[RankedFile]) -> Value {
    Value::Array(
        ranked
            .iter()
            .map(|rf| {
                json!({
                    "path": rf.path,
                    "status": rf.status.label(),
                    "score": rf.score,
                    "signals": rf.signals.iter().map(|s| s.label()).collect::<Vec<_>>(),
                })
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{METHOD_NOT_FOUND, parse_request};

    fn call(line: &str, engine: &mut Engine) -> Result<Value, RpcError> {
        let req = parse_request(line)?;
        dispatch(&req, engine)
    }

    #[test]
    fn new_engine_is_running_with_no_sessions() {
        let e = Engine::new();
        assert!(e.is_running());
        assert_eq!(e.session_count(), 0);
    }

    #[test]
    fn health_reports_ok() {
        let mut e = Engine::new();
        let v = call(r#"{"id":1,"method":"daemon.health"}"#, &mut e).unwrap();
        assert_eq!(v["status"], "ok");
        assert_eq!(v["protocol"], PROTOCOL_VERSION);
        assert!(v["pid"].as_u64().unwrap() > 0);
    }

    #[test]
    fn initialize_reports_server() {
        let mut e = Engine::new();
        let v = call(r#"{"method":"engine.initialize"}"#, &mut e).unwrap();
        assert_eq!(v["server"], "deep-diff-forge");
    }

    #[test]
    fn status_reports_running_and_sessions() {
        let mut e = Engine::new();
        let v = call(r#"{"method":"daemon.status"}"#, &mut e).unwrap();
        assert_eq!(v["running"], true);
        assert_eq!(v["sessions"], 0);
    }

    #[test]
    fn shutdown_clears_running() {
        let mut e = Engine::new();
        let v = call(r#"{"method":"daemon.shutdown"}"#, &mut e).unwrap();
        assert_eq!(v["stopping"], true);
        assert!(!e.is_running());
    }

    #[test]
    fn unknown_method_is_method_not_found() {
        let mut e = Engine::new();
        let err = call(r#"{"method":"no.such"}"#, &mut e).unwrap_err();
        assert_eq!(err.code, METHOD_NOT_FOUND);
    }

    #[test]
    fn diff_plan_ranks_files() {
        let mut e = Engine::new();
        let line = r#"{"method":"diff.plan","params":{"patch":"--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@\n-a\n+b\n"}}"#;
        let v = call(line, &mut e).unwrap();
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
        let mut e = Engine::new();
        let err = call(r#"{"method":"diff.plan","params":{}}"#, &mut e).unwrap_err();
        assert_eq!(err.code, crate::protocol::INVALID_PARAMS);
    }

    #[test]
    fn diff_plan_bad_patch_is_parse_failed() {
        let mut e = Engine::new();
        let line = r#"{"method":"diff.plan","params":{"patch":"--- a/x\n+++ b/x\n+stray\n"}}"#;
        let err = call(line, &mut e).unwrap_err();
        assert_eq!(err.code, PATCH_PARSE_FAILED);
    }

    #[test]
    fn session_open_returns_id_and_count() {
        let mut e = Engine::new();
        let line = r#"{"method":"session.open","params":{"patch":"--- a/x.rs\n+++ b/x.rs\n@@ -1,1 +1,1 @@\n-a\n+b\n"}}"#;
        let v = call(line, &mut e).unwrap();
        assert_eq!(v["session"], "s1");
        assert_eq!(v["files"], 1);
        assert_eq!(e.session_count(), 1);
    }

    #[test]
    fn session_ids_increment() {
        let mut e = Engine::new();
        let line = r#"{"method":"session.open","params":{"patch":"--- a/x.rs\n+++ b/x.rs\n@@ -1,1 +1,1 @@\n-a\n+b\n"}}"#;
        let a = call(line, &mut e).unwrap();
        let b = call(line, &mut e).unwrap();
        assert_eq!(a["session"], "s1");
        assert_eq!(b["session"], "s2");
    }

    #[test]
    fn session_snapshot_returns_ranked() {
        let mut e = Engine::new();
        let open = r#"{"method":"session.open","params":{"patch":"--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@\n-a\n+b\n"}}"#;
        let opened = call(open, &mut e).unwrap();
        let id = opened["session"].as_str().unwrap();
        let snap = call(
            &format!(r#"{{"method":"session.snapshot","params":{{"session":"{id}"}}}}"#),
            &mut e,
        )
        .unwrap();
        assert_eq!(snap[0]["path"], "src/lib.rs");
    }

    #[test]
    fn snapshot_unknown_session_is_error() {
        let mut e = Engine::new();
        let err = call(
            r#"{"method":"session.snapshot","params":{"session":"nope"}}"#,
            &mut e,
        )
        .unwrap_err();
        assert_eq!(err.code, SESSION_NOT_FOUND);
    }

    #[test]
    fn session_close_removes_session() {
        let mut e = Engine::new();
        let open = r#"{"method":"session.open","params":{"patch":"--- a/x.rs\n+++ b/x.rs\n@@ -1,1 +1,1 @@\n-a\n+b\n"}}"#;
        let id = call(open, &mut e).unwrap()["session"]
            .as_str()
            .unwrap()
            .to_string();
        let closed = call(
            &format!(r#"{{"method":"session.close","params":{{"session":"{id}"}}}}"#),
            &mut e,
        )
        .unwrap();
        assert_eq!(closed["closed"], true);
        assert_eq!(e.session_count(), 0);
    }

    #[test]
    fn closing_unknown_session_is_false_not_error() {
        let mut e = Engine::new();
        let v = call(
            r#"{"method":"session.close","params":{"session":"ghost"}}"#,
            &mut e,
        )
        .unwrap();
        assert_eq!(v["closed"], false);
    }

    #[test]
    fn health_session_count_tracks_opens() {
        let mut e = Engine::new();
        let open = r#"{"method":"session.open","params":{"patch":"--- a/x.rs\n+++ b/x.rs\n@@ -1,1 +1,1 @@\n-a\n+b\n"}}"#;
        call(open, &mut e).unwrap();
        let v = call(r#"{"method":"daemon.health"}"#, &mut e).unwrap();
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
        let mut e = Engine::new();
        let files =
            deep_diff_forge_patch::parse("--- a/x.rs\n+++ b/x.rs\n@@ -1,1 +1,1 @@\n-a\n+b\n")
                .unwrap();
        let one_session = estimated_session_bytes(&files);
        e.max_retained_bytes = one_session;

        let first = e.open_session(files.clone()).unwrap();
        let second = e.open_session(files).unwrap();
        assert_eq!(e.session_count(), 1);
        assert!(e.snapshot_ranked(&first).is_none());
        assert!(e.snapshot_ranked(&second).is_some());
        assert!(e.retained_bytes <= e.max_retained_bytes);
    }

    #[test]
    fn oversized_session_is_rejected_without_eviction() {
        let mut e = Engine::new();
        let files =
            deep_diff_forge_patch::parse("--- a/x.rs\n+++ b/x.rs\n@@ -1,1 +1,1 @@\n-a\n+b\n")
                .unwrap();
        e.max_retained_bytes = estimated_session_bytes(&files).saturating_sub(1);
        let err = e.open_session(files).unwrap_err();
        assert_eq!(err.code, RESOURCE_EXHAUSTED);
        assert_eq!(e.session_count(), 0);
    }

    #[test]
    fn session_param_must_be_string() {
        let mut e = Engine::new();
        let err = call(
            r#"{"method":"session.snapshot","params":{"session":123}}"#,
            &mut e,
        )
        .unwrap_err();
        assert_eq!(err.code, crate::protocol::INVALID_PARAMS);
    }

    #[test]
    fn diff_plan_empty_patch_is_empty_array() {
        let mut e = Engine::new();
        let v = call(r#"{"method":"diff.plan","params":{"patch":""}}"#, &mut e).unwrap();
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
        let mut e = Engine::new();
        for _ in 0..(super::MAX_SESSIONS + 5) {
            call(OPEN_LINE, &mut e).expect("session.open");
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
        let mut e = Engine::new();
        // Fill to MAX_SESSIONS.
        for _ in 0..super::MAX_SESSIONS {
            call(OPEN_LINE, &mut e).expect("session.open");
        }
        assert_eq!(e.session_count(), super::MAX_SESSIONS);

        // Access s1 to refresh its LRU tick — it must NOT be evicted next.
        let snap = call(
            r#"{"method":"session.snapshot","params":{"session":"s1"}}"#,
            &mut e,
        );
        assert!(snap.is_ok(), "s1 snapshot before eviction must succeed");

        // Open one more session — evicts the LRU (not s1).
        call(OPEN_LINE, &mut e).expect("session.open over cap");
        assert_eq!(e.session_count(), super::MAX_SESSIONS);

        // s1 must still be accessible.
        let snap_after = call(
            r#"{"method":"session.snapshot","params":{"session":"s1"}}"#,
            &mut e,
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
        let mut e = Engine::new();
        let mut ids = Vec::new();
        for _ in 0..super::MAX_SESSIONS {
            let v = call(OPEN_LINE, &mut e).expect("open");
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
                &mut e,
            )
            .expect("refresh");
        }
        // Open one more — must evict exactly the victim (the true min-tick entry).
        call(OPEN_LINE, &mut e).expect("open over cap");
        assert_eq!(e.session_count(), super::MAX_SESSIONS);
        let victim_snap = call(
            &format!(r#"{{"method":"session.snapshot","params":{{"session":"{victim}"}}}}"#),
            &mut e,
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
            &mut e,
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
        let mut e = Engine::new();
        for _ in 0..(super::MAX_SESSIONS * 3) {
            call(OPEN_LINE, &mut e).expect("session.open");
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
        let mut e = Engine::new();
        let mut ids = Vec::new();
        // Open MAX_SESSIONS sessions.
        for _ in 0..super::MAX_SESSIONS {
            let v = call(OPEN_LINE, &mut e).expect("open");
            ids.push(v["session"].as_str().unwrap().to_string());
        }
        assert_eq!(e.session_count(), super::MAX_SESSIONS);
        // Close all.
        for id in &ids {
            let v = call(
                &format!(r#"{{"method":"session.close","params":{{"session":"{id}"}}}}"#),
                &mut e,
            )
            .expect("close");
            assert_eq!(v["closed"], true);
        }
        assert_eq!(e.session_count(), 0);
        // Now we can open MAX_SESSIONS more without evictions.
        for _ in 0..super::MAX_SESSIONS {
            call(OPEN_LINE, &mut e).expect("re-open after close");
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
        assert_eq!(Engine::new().max_retained_bytes, 128 << 20);
    }

    #[test]
    fn default_budget_accepts_multi_mebibyte_session() {
        // Well under 128 MiB, but over any accidentally-additive budget (~1 MiB).
        let mut e = Engine::new();
        e.open_session(patch_with_line(4 << 20))
            .expect("a 4 MiB session fits the default budget");
        assert_eq!(e.session_count(), 1);
    }

    #[test]
    fn estimate_counts_retained_line_text() {
        let files = patch_with_line(10_000);
        assert!(estimated_session_bytes(&files) >= 10_000);
        let mut e = Engine::new();
        e.max_retained_bytes = 4096;
        let err = e.open_session(files).unwrap_err();
        assert_eq!(err.code, RESOURCE_EXHAUSTED);
    }

    #[test]
    fn sessions_exactly_filling_the_byte_budget_are_all_retained() {
        let mut e = Engine::new();
        // Parse each session separately: a clone may shrink vector capacities.
        e.max_retained_bytes = 2 * estimated_session_bytes(&patch_with_line(1));
        let first = e.open_session(patch_with_line(1)).unwrap();
        let second = e.open_session(patch_with_line(1)).unwrap();
        assert_eq!(e.retained_bytes, e.max_retained_bytes);
        assert!(
            e.snapshot_ranked(&first).is_some(),
            "at-budget must not evict"
        );
        assert!(e.snapshot_ranked(&second).is_some());
    }

    #[test]
    fn byte_budget_evicts_as_many_sessions_as_needed() {
        let mut e = Engine::new();
        let large = patch_with_line(4096);
        let large_bytes = estimated_session_bytes(&large);
        assert!(large_bytes > estimated_session_bytes(&patch_with_line(1)));
        e.open_session(patch_with_line(1)).unwrap();
        e.open_session(patch_with_line(1)).unwrap();
        // Room for the large session only once BOTH small sessions are gone.
        e.max_retained_bytes = large_bytes;
        let kept = e.open_session(large).unwrap();
        assert_eq!(e.session_count(), 1);
        assert!(e.snapshot_ranked(&kept).is_some());
        assert_eq!(e.retained_bytes, large_bytes);
    }

    #[test]
    fn ticks_increase_monotonically() {
        let mut e = Engine::new();
        assert_eq!(e.next_tick(), 1);
        assert_eq!(e.next_tick(), 2);
        e.open_session(patch_with_line(1)).unwrap();
        assert_eq!(e.next_tick(), 4);
    }

    #[test]
    fn tick_overflow_rebases_densely_and_preserves_lru_order() {
        let mut e = Engine::new();
        let a = e.open_session(patch_with_line(1)).unwrap();
        let b = e.open_session(patch_with_line(1)).unwrap();
        let c = e.open_session(patch_with_line(1)).unwrap();
        // Recency order: b (oldest), c, a (newest).
        e.sessions.get_mut(&b).unwrap().last_tick = u64::MAX - 30;
        e.sessions.get_mut(&c).unwrap().last_tick = u64::MAX - 20;
        e.sessions.get_mut(&a).unwrap().last_tick = u64::MAX - 10;
        e.tick = u64::MAX;

        assert_eq!(e.next_tick(), 4);
        assert_eq!(e.sessions[&b].last_tick, 1);
        assert_eq!(e.sessions[&c].last_tick, 2);
        assert_eq!(e.sessions[&a].last_tick, 3);
        assert!(e.evict_lru());
        assert!(
            !e.sessions.contains_key(&b),
            "oldest entry is still the LRU"
        );
    }

    // F3-T5: snapshot on a non-existent session is still SESSION_NOT_FOUND even
    // when the engine is at cap (eviction must not swallow the error).
    #[test]
    fn snapshot_missing_session_at_cap_is_error() {
        let mut e = Engine::new();
        for _ in 0..super::MAX_SESSIONS {
            call(OPEN_LINE, &mut e).expect("open");
        }
        let err = call(
            r#"{"method":"session.snapshot","params":{"session":"ghost"}}"#,
            &mut e,
        )
        .unwrap_err();
        assert_eq!(err.code, SESSION_NOT_FOUND);
    }

    #[test]
    fn every_listed_method_is_dispatched_and_nothing_else_is() {
        // An unlisted name is refused before the dispatcher's arms are consulted.
        let mut probe = Engine::new();
        for unlisted in ["engine.shutdown", "session.list", "diff.Compute", ""] {
            let request = Request {
                jsonrpc: "2.0".into(),
                id: Value::Null,
                method: unlisted.into(),
                params: Value::Null,
            };
            assert_eq!(
                dispatch(&request, &mut probe).unwrap_err().code,
                crate::protocol::METHOD_NOT_FOUND,
                "{unlisted}"
            );
        }
        for spec in METHODS {
            let mut e = Engine::new();
            let line = format!(r#"{{"method":"{}"}}"#, spec.name);
            if let Err(err) = call(&line, &mut e) {
                assert_ne!(err.code, crate::protocol::METHOD_NOT_FOUND, "{}", spec.name);
            }
            assert!(spec.since <= PROTOCOL_VERSION, "{}", spec.name);
        }
        let names: Vec<&str> = METHODS.iter().map(|m| m.name).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len());
        let v = call(r#"{"method":"engine.methods"}"#, &mut Engine::new()).unwrap();
        assert_eq!(v["methods"].as_array().unwrap().len(), METHODS.len());
        assert_eq!(v["methods"][6]["name"], "diff.compute");
        let v = call(r#"{"method":"engine.initialize"}"#, &mut Engine::new()).unwrap();
        assert_eq!(v["methods"].as_array().unwrap().len(), METHODS.len());
    }

    #[test]
    fn initialize_refuses_a_protocol_it_does_not_speak() {
        let mut e = Engine::new();
        assert_eq!(
            call(
                r#"{"method":"engine.initialize","params":{"protocol":1}}"#,
                &mut e
            )
            .unwrap()["protocol"],
            1
        );
        let v0 = call(
            r#"{"method":"engine.initialize","params":{"protocol":0}}"#,
            &mut e,
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
        for other in ["2", "\"1\"", "1.5", "null", "-1"] {
            let line =
                format!(r#"{{"method":"engine.initialize","params":{{"protocol":{other}}}}}"#);
            assert_eq!(
                call(&line, &mut e).unwrap_err().code,
                PROTOCOL_UNSUPPORTED,
                "{other}"
            );
        }
    }

    #[test]
    fn compute_applies_the_input_contracts_and_reports_typed_errors() {
        let mut e = Engine::new();
        let ok = r#"{"method":"diff.compute","params":{"patch":"--- a/x\n+++ b/x\n@@ -1,1 +1,1 @@\n-a\n+b\n","require_files":true,"require_hunks":true}}"#;
        let v = call(ok, &mut e).unwrap();
        assert_eq!(v["schema"], "deep-diff-forge.review.v0");
        assert_eq!(v["files"].as_array().unwrap().len(), 1);
        let empty = call(
            r#"{"method":"diff.compute","params":{"patch":"","require_files":true}}"#,
            &mut e,
        )
        .unwrap_err();
        assert_eq!(
            (empty.code, empty.message.as_str()),
            (
                INPUT_CONTRACT_REFUSED,
                "refused: 0 files in input (--require-files)"
            )
        );
        let header = call(r#"{"method":"diff.compute","params":{"patch":"diff --git a/x b/x\n","require_hunks":true}}"#, &mut e).unwrap_err();
        assert_eq!(
            (header.code, header.message.as_str()),
            (
                INPUT_CONTRACT_REFUSED,
                "refused: 0 hunks in input (--require-hunks)"
            )
        );
        assert!(call(r#"{"method":"diff.compute","params":{"patch":""}}"#, &mut e).is_ok());
        let bad = call(
            r#"{"method":"diff.compute","params":{"patch":"","require_files":"yes"}}"#,
            &mut e,
        )
        .unwrap_err();
        assert_eq!(bad.code, crate::protocol::INVALID_PARAMS);
        let garbage = call(
            r#"{"method":"diff.compute","params":{"patch":"--- a/x\n+++ b/x\n+stray\n"}}"#,
            &mut e,
        )
        .unwrap_err();
        assert_eq!(garbage.code, PATCH_PARSE_FAILED);
        assert_eq!(
            call(r#"{"method":"diff.compute","params":{}}"#, &mut e)
                .unwrap_err()
                .code,
            crate::protocol::INVALID_PARAMS
        );
    }
}
