// DDF integration: daemon log lines as text or JSON (plan 2026-10-30, M1 monitoring).
// Design/status: docs/OPERATIONS_AND_DAEMON.md#observability
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fcrates--deep-diff-forge-daemon--src--metrics
//! The daemon's log lines, written to stderr: the text the daemon has always written, or with
//! `daemon start --log-format json` one JSON object per line, so a supervisor can parse them.
//! Every log line goes through [`line`], so the two formats cannot drift apart.

use serde_json::json;

/// How the daemon writes its log lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LogFormat {
    /// `deep-diff-forge daemon: <message>`.
    #[default]
    Text,
    /// `{"ts_ms", "level", "event", "message"}`.
    Json,
}

/// The wall-clock door for log timestamps.
#[expect(
    clippy::disallowed_methods,
    reason = "the log clock door: a timestamp on each structured log line"
)]
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// One log line for `event` (a stable snake-case name) and its `message`.
#[must_use]
pub fn line(format: LogFormat, event: &str, message: &str) -> String {
    match format {
        LogFormat::Text => format!("deep-diff-forge daemon: {message}"),
        LogFormat::Json => json!({
            "ts_ms": now_ms(),
            "level": "warn",
            "event": event,
            "message": message,
        })
        .to_string(),
    }
}

/// Write one log line to stderr.
pub fn write(format: LogFormat, event: &str, message: &str) {
    eprintln!("{}", line(format, event, message));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_the_daemons_own_line_and_json_is_one_object() {
        assert_eq!(
            line(
                LogFormat::Text,
                "connection_refused",
                "refused a connection: x"
            ),
            "deep-diff-forge daemon: refused a connection: x"
        );
        let json: serde_json::Value =
            serde_json::from_str(&line(LogFormat::Json, "connection_refused", "a \"b\"\nc"))
                .unwrap();
        assert_eq!(json["event"], "connection_refused");
        assert_eq!(json["level"], "warn");
        assert_eq!(json["message"], "a \"b\"\nc");
        assert!(json["ts_ms"].as_u64().unwrap() > 1_700_000_000_000);
        assert!(!line(LogFormat::Json, "e", "m").contains('\n'));
        assert_eq!(LogFormat::default(), LogFormat::Text);
    }
}
