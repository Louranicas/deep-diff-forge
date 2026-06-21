use crate::protocol::{PATCH_PARSE_FAILED, PROTOCOL_VERSION, Request, RpcError, SESSION_NOT_FOUND};
use deep_diff_forge_core::ReviewFile;
use deep_diff_forge_graph::{RankedFile, rank};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::time::Instant;

/// Daemon engine: review sessions, counters, and lifecycle state.
///
/// Shared across connection threads behind a `Mutex` by the server; the methods
/// here are the pure, testable core (no socket I/O).
pub struct Engine {
    sessions: HashMap<String, Vec<ReviewFile>>,
    next_session: u64,
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

    /// Number of open sessions.
    #[must_use]
    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    fn open_session(&mut self, files: Vec<ReviewFile>) -> String {
        let id = format!("s{}", self.next_session);
        self.next_session += 1;
        self.sessions.insert(id.clone(), files);
        id
    }

    fn snapshot(&self, id: &str) -> Option<&Vec<ReviewFile>> {
        self.sessions.get(id)
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
                Some(files) => Ok(ranked_json(&rank(files))),
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
        let req = parse_request(line).expect("valid request");
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
}
