use crate::protocol::{PATCH_PARSE_FAILED, PROTOCOL_VERSION, Request, RpcError, SESSION_NOT_FOUND};
use deep_diff_forge_core::ReviewFile;
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

/// Per-session state stored inside the engine.
struct SessionEntry {
    files: Vec<ReviewFile>,
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
            running: true,
            pid: std::process::id(),
            started: Instant::now(),
        }
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
        self.tick += 1;
        self.tick
    }

    /// Evict the least-recently-used session when the cap is reached.
    fn evict_lru_if_needed(&mut self) {
        if self.sessions.len() < MAX_SESSIONS {
            return;
        }
        // Find the key with the smallest last_tick (LRU).
        let lru_key = self
            .sessions
            .iter()
            .min_by_key(|(_, entry)| entry.last_tick)
            .map(|(key, _)| key.clone());
        if let Some(key) = lru_key {
            self.sessions.remove(&key);
        }
    }

    fn open_session(&mut self, files: Vec<ReviewFile>) -> String {
        // Evict LRU before inserting so the map never exceeds MAX_SESSIONS.
        self.evict_lru_if_needed();
        let id = format!("s{}", self.next_session);
        self.next_session += 1;
        let tick = self.next_tick();
        self.sessions.insert(
            id.clone(),
            SessionEntry {
                files,
                last_tick: tick,
            },
        );
        id
    }

    /// Returns a clone of the session's files, refreshing the LRU tick.
    ///
    /// A read counts as an access so recently-queried sessions are not
    /// unfairly evicted before idle ones.  We return an owned `Vec` to avoid
    /// holding a borrow across the tick mutation.
    fn snapshot(&mut self, id: &str) -> Option<Vec<ReviewFile>> {
        let tick = self.next_tick();
        if let Some(entry) = self.sessions.get_mut(id) {
            entry.last_tick = tick;
            Some(entry.files.clone())
        } else {
            None
        }
    }

    fn close_session(&mut self, id: &str) -> bool {
        self.sessions.remove(id).is_some()
    }
}

/// Dispatch a parsed request against the engine, returning the result value or
/// a typed [`RpcError`].
///
/// # Errors
///
/// Returns [`RpcError`] for unknown methods, missing/invalid params, patch
/// parse failures, or unknown sessions.
pub fn dispatch(request: &Request, engine: &mut Engine) -> Result<Value, RpcError> {
    let version = env!("CARGO_PKG_VERSION");
    match request.method.as_str() {
        "engine.initialize" => Ok(json!({
            "server": "deep-diff-forge",
            "version": version,
            "protocol": PROTOCOL_VERSION,
        })),
        "daemon.health" => Ok(json!({
            "status": "ok",
            "version": version,
            "pid": engine.pid,
            "sessions": engine.session_count(),
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
        "session.open" => {
            let files = parse_patch_param(request)?;
            let count = files.len();
            let id = engine.open_session(files);
            Ok(json!({"session": id, "files": count}))
        }
        "session.snapshot" => {
            let id = str_param(&request.params, "session")?;
            match engine.snapshot(&id) {
                Some(files) => Ok(ranked_json(&rank(&files))),
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
}
