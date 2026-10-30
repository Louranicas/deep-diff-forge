// DDF integration: daemon runtime metrics (plan 2026-10-30, M1 monitoring).
// Design/status: docs/PLAN_AGENT_EVIDENCE_AND_OBSERVABILITY.md#system-m-monitoring-evaluation-and-benchmarking
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fcrates--deep-diff-forge-daemon--src--metrics
//! Runtime metrics for a running daemon: what it served, how fast, what it refused, and what went
//! wrong, read with `daemon.metrics` as `deep-diff-forge.daemon-metrics.v0`.
//!
//! Every counter is an `AtomicU64` written with relaxed adds, so recording costs one or two
//! atomic operations on the request path and takes no lock: requests run in parallel, and
//! metrics must not serialise them again. Counters live as long as the process; history comes
//! from receipts (the weekly campaign), not from here.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Latency buckets: bucket `i` holds requests that took less than `2^i` microseconds (the last
/// one holds the rest), so 24 buckets reach about 8.4 s.
pub const LATENCY_BUCKETS: usize = 24;

/// The RPC error codes counted by name; any other code counts as `other`.
const ERROR_CODES: [(i64, &str); 11] = [
    (crate::protocol::PARSE_ERROR, "parse_error"),
    (crate::protocol::INVALID_REQUEST, "invalid_request"),
    (crate::protocol::METHOD_NOT_FOUND, "method_not_found"),
    (crate::protocol::INVALID_PARAMS, "invalid_params"),
    (crate::protocol::INTERNAL_ERROR, "internal_error"),
    (
        crate::protocol::PROTOCOL_UNSUPPORTED,
        "protocol_unsupported",
    ),
    (crate::protocol::PATCH_PARSE_FAILED, "patch_parse_failed"),
    (crate::protocol::SESSION_NOT_FOUND, "session_not_found"),
    (
        crate::protocol::INPUT_CONTRACT_REFUSED,
        "input_contract_refused",
    ),
    (crate::protocol::RESOURCE_EXHAUSTED, "resource_exhausted"),
    (crate::protocol::GIT_FAILED, "git_failed"),
];

/// One method's counters.
#[derive(Debug, Default)]
struct MethodCounters {
    requests: AtomicU64,
    errors: [AtomicU64; ERROR_CODES.len() + 1],
    latency: [AtomicU64; LATENCY_BUCKETS],
}

/// Every counter a daemon keeps.
#[derive(Debug)]
pub struct Metrics {
    /// One entry per `METHODS` entry, then one for a method the daemon does not have.
    methods: Vec<MethodCounters>,
    admitted: AtomicU64,
    waited: AtomicU64,
    wait_micros: AtomicU64,
    busy: AtomicU64,
    peak_in_flight_bytes: AtomicU64,
    peak_running: AtomicU64,
    accepted: AtomicU64,
    refused_at_limit: AtomicU64,
    refused_peer: AtomicU64,
    connection_errors: AtomicU64,
    panics: AtomicU64,
    session_resets: AtomicU64,
    sessions_opened: AtomicU64,
    sessions_closed: AtomicU64,
    sessions_evicted: AtomicU64,
    peak_retained_bytes: AtomicU64,
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

/// The metrics clock door: how long a request took, or waited.
#[expect(
    clippy::disallowed_methods,
    reason = "the metrics clock door: request latency and admission wait"
)]
#[must_use]
pub fn now() -> Instant {
    Instant::now()
}

fn add(counter: &AtomicU64, by: u64) {
    counter.fetch_add(by, Ordering::Relaxed);
}

fn get(counter: &AtomicU64) -> u64 {
    counter.load(Ordering::Relaxed)
}

fn micros(duration: Duration) -> u64 {
    u64::try_from(duration.as_micros()).unwrap_or(u64::MAX)
}

/// The bucket for a latency of `us` microseconds: the smallest `i` with `us < 2^i`, or the last.
#[must_use]
pub fn bucket_of(us: u64) -> usize {
    let bits = (u64::BITS - us.leading_zeros()) as usize;
    bits.min(LATENCY_BUCKETS - 1)
}

/// The upper bound, in microseconds, of latency bucket `i` (`None` for the open last one).
#[must_use]
pub fn bucket_bound(i: usize) -> Option<u64> {
    (i < LATENCY_BUCKETS - 1).then(|| 1u64 << i)
}

/// What a request was, for the metrics: its method's position in `METHODS`, or `None`.
pub type MethodIndex = Option<usize>;

impl Metrics {
    /// Zeroed metrics for a daemon serving `METHODS`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            methods: (0..=crate::METHODS.len())
                .map(|_| MethodCounters::default())
                .collect(),
            admitted: AtomicU64::new(0),
            waited: AtomicU64::new(0),
            wait_micros: AtomicU64::new(0),
            busy: AtomicU64::new(0),
            peak_in_flight_bytes: AtomicU64::new(0),
            peak_running: AtomicU64::new(0),
            accepted: AtomicU64::new(0),
            refused_at_limit: AtomicU64::new(0),
            refused_peer: AtomicU64::new(0),
            connection_errors: AtomicU64::new(0),
            panics: AtomicU64::new(0),
            session_resets: AtomicU64::new(0),
            sessions_opened: AtomicU64::new(0),
            sessions_closed: AtomicU64::new(0),
            sessions_evicted: AtomicU64::new(0),
            peak_retained_bytes: AtomicU64::new(0),
        }
    }

    /// The counters for `method` (the last entry for an unknown method).
    fn method(&self, method: MethodIndex) -> &MethodCounters {
        let last = self.methods.len() - 1;
        &self.methods[method.map_or(last, |i| i.min(last))]
    }

    /// A request answered after `took`, with an RPC error code if it failed.
    pub fn request(&self, method: MethodIndex, took: Duration, error: Option<i64>) {
        let counters = self.method(method);
        add(&counters.requests, 1);
        add(&counters.latency[bucket_of(micros(took))], 1);
        if let Some(code) = error {
            let slot = ERROR_CODES
                .iter()
                .position(|(c, _)| *c == code)
                .unwrap_or(ERROR_CODES.len());
            add(&counters.errors[slot], 1);
        }
    }

    /// A request admitted after waiting `waited`, with the in-flight bytes and running count it
    /// made.
    pub fn admitted(&self, waited: Duration, in_flight_bytes: usize, running: usize) {
        add(&self.admitted, 1);
        if !waited.is_zero() {
            add(&self.waited, 1);
            add(&self.wait_micros, micros(waited));
        }
        self.peak_in_flight_bytes.fetch_max(
            u64::try_from(in_flight_bytes).unwrap_or(u64::MAX),
            Ordering::Relaxed,
        );
        self.peak_running.fetch_max(
            u64::try_from(running).unwrap_or(u64::MAX),
            Ordering::Relaxed,
        );
    }

    /// A request refused as busy after waiting the admission limit.
    pub fn busy(&self) {
        add(&self.busy, 1);
    }

    /// A connection accepted and given a worker.
    pub fn accepted(&self) {
        add(&self.accepted, 1);
    }

    /// A connection closed unanswered because every worker slot was taken.
    pub fn refused_at_limit(&self) {
        add(&self.refused_at_limit, 1);
    }

    /// A connection from another user, dropped.
    pub fn refused_peer(&self) {
        add(&self.refused_peer, 1);
    }

    /// A connection that ended in an I/O error (a read deadline, a reset).
    pub fn connection_error(&self) {
        add(&self.connection_errors, 1);
    }

    /// A panic inside a request, contained.
    pub fn panic(&self) {
        add(&self.panics, 1);
    }

    /// The session table emptied after a panic held it.
    pub fn session_reset(&self) {
        add(&self.session_resets, 1);
    }

    /// A session opened, holding `retained_bytes` in all sessions after it.
    pub fn session_opened(&self, retained_bytes: usize) {
        add(&self.sessions_opened, 1);
        self.peak_retained_bytes.fetch_max(
            u64::try_from(retained_bytes).unwrap_or(u64::MAX),
            Ordering::Relaxed,
        );
    }

    /// A session closed by its client.
    pub fn session_closed(&self) {
        add(&self.sessions_closed, 1);
    }

    /// `n` sessions evicted to make room.
    pub fn sessions_evicted(&self, n: usize) {
        add(&self.sessions_evicted, u64::try_from(n).unwrap_or(u64::MAX));
    }

    /// The `deep-diff-forge.daemon-metrics.v0` document.
    #[must_use]
    pub fn document(&self, uptime: Duration, version: &str) -> serde_json::Value {
        use serde_json::json;
        let mut methods = serde_json::Map::new();
        for (i, counters) in self.methods.iter().enumerate() {
            let requests = get(&counters.requests);
            if requests == 0 {
                continue;
            }
            let name = crate::METHODS.get(i).map_or("(unknown)", |m| m.name);
            let histogram: Vec<u64> = counters.latency.iter().map(get).collect();
            let mut errors = serde_json::Map::new();
            for (slot, counter) in counters.errors.iter().enumerate() {
                let n = get(counter);
                if n > 0 {
                    let label = ERROR_CODES.get(slot).map_or("other", |(_, label)| label);
                    errors.insert(label.to_owned(), json!(n));
                }
            }
            methods.insert(
                name.to_owned(),
                json!({
                    "requests": requests,
                    "errors": errors,
                    "latency_us": {
                        "p50_at_most": quantile_bound(&histogram, 50),
                        "p95_at_most": quantile_bound(&histogram, 95),
                        "p99_at_most": quantile_bound(&histogram, 99),
                        "buckets": histogram,
                    },
                }),
            );
        }
        json!({
            "schema": "deep-diff-forge.daemon-metrics.v0",
            "version": version,
            "uptime_secs": uptime.as_secs(),
            "latency_note": "latency_us quantiles are upper bounds of log2 microsecond buckets (bucket i: under 2^i us); null is over 2^22 us",
            "methods": methods,
            "admission": {
                "admitted": get(&self.admitted),
                "waited": get(&self.waited),
                "wait_us": get(&self.wait_micros),
                "busy": get(&self.busy),
                "peak_in_flight_bytes": get(&self.peak_in_flight_bytes),
                "peak_running": get(&self.peak_running),
            },
            "connections": {
                "accepted": get(&self.accepted),
                "refused_at_limit": get(&self.refused_at_limit),
                "refused_peer": get(&self.refused_peer),
                "errors": get(&self.connection_errors),
            },
            "engine": {
                "panics": get(&self.panics),
                "session_resets": get(&self.session_resets),
                "sessions_opened": get(&self.sessions_opened),
                "sessions_closed": get(&self.sessions_closed),
                "sessions_evicted": get(&self.sessions_evicted),
                "peak_retained_bytes": get(&self.peak_retained_bytes),
            },
        })
    }
}

/// The upper bound (microseconds) of the bucket holding the `percent`th percentile request, or
/// `None` when there are none or it falls in the open last bucket.
#[must_use]
pub fn quantile_bound(histogram: &[u64], percent: u64) -> Option<u64> {
    let total: u64 = histogram.iter().sum();
    if total == 0 {
        return None;
    }
    // The smallest count that covers `percent` of the requests (rounded up).
    let needed = (total * percent).div_ceil(100);
    let mut seen = 0u64;
    for (i, n) in histogram.iter().enumerate() {
        seen += n;
        if seen >= needed {
            return bucket_bound(i);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buckets_are_powers_of_two_microseconds() {
        assert_eq!(bucket_of(0), 0);
        assert_eq!(bucket_of(1), 1);
        assert_eq!(bucket_of(2), 2);
        assert_eq!(bucket_of(3), 2);
        assert_eq!(bucket_of(4), 3);
        assert_eq!(bucket_of(1023), 10);
        assert_eq!(bucket_of(1024), 11);
        assert_eq!(bucket_of(u64::MAX), LATENCY_BUCKETS - 1);
        assert_eq!(bucket_of(1 << 22), LATENCY_BUCKETS - 1);
        assert_eq!(bucket_of((1 << 22) - 1), 22);
        assert_eq!(
            (bucket_bound(0), bucket_bound(10), bucket_bound(22)),
            (Some(1), Some(1024), Some(1 << 22))
        );
        assert_eq!(bucket_bound(LATENCY_BUCKETS - 1), None);
        assert_eq!(LATENCY_BUCKETS, 24);
    }

    #[test]
    fn quantiles_are_bucket_upper_bounds() {
        let mut h = [0u64; LATENCY_BUCKETS];
        assert_eq!(quantile_bound(&h, 50), None);
        h[3] = 90;
        h[10] = 9;
        h[20] = 1;
        assert_eq!(quantile_bound(&h, 50), Some(8));
        assert_eq!(quantile_bound(&h, 90), Some(8));
        assert_eq!(quantile_bound(&h, 95), Some(1024));
        assert_eq!(quantile_bound(&h, 99), Some(1024));
        assert_eq!(quantile_bound(&h, 100), Some(1 << 20));
        h[LATENCY_BUCKETS - 1] = 1000;
        assert_eq!(quantile_bound(&h, 99), None);
    }

    #[test]
    fn every_counter_reaches_the_document() {
        let m = Metrics::new();
        m.request(Some(0), Duration::from_micros(5), None);
        m.request(
            Some(0),
            Duration::from_micros(5),
            Some(crate::protocol::INVALID_PARAMS),
        );
        m.request(None, Duration::from_micros(1), Some(-1));
        m.admitted(Duration::ZERO, 10, 1);
        m.admitted(Duration::from_micros(250), 7, 3);
        m.busy();
        m.accepted();
        m.refused_at_limit();
        m.refused_peer();
        m.connection_error();
        m.panic();
        m.session_reset();
        m.session_opened(40);
        m.session_opened(30);
        m.session_closed();
        m.sessions_evicted(2);
        let doc = m.document(Duration::from_secs(9), "9.9.9");
        assert_eq!(doc["schema"], "deep-diff-forge.daemon-metrics.v0");
        assert_eq!(
            (doc["version"].as_str(), doc["uptime_secs"].as_u64()),
            (Some("9.9.9"), Some(9))
        );
        let first = &doc["methods"][crate::METHODS[0].name];
        assert_eq!(first["requests"], 2);
        assert_eq!(first["errors"], serde_json::json!({"invalid_params": 1}));
        assert_eq!(first["latency_us"]["buckets"][3], 2);
        assert_eq!(first["latency_us"]["p50_at_most"], 8);
        assert_eq!(
            doc["methods"]["(unknown)"]["errors"],
            serde_json::json!({"other": 1})
        );
        assert_eq!(
            doc["methods"].as_object().unwrap().len(),
            2,
            "a method never called is not listed"
        );
        assert_eq!(
            doc["admission"],
            serde_json::json!({"admitted": 2, "waited": 1, "wait_us": 250, "busy": 1, "peak_in_flight_bytes": 10, "peak_running": 3})
        );
        assert_eq!(
            doc["connections"],
            serde_json::json!({"accepted": 1, "refused_at_limit": 1, "refused_peer": 1, "errors": 1})
        );
        assert_eq!(
            doc["engine"],
            serde_json::json!({"panics": 1, "session_resets": 1, "sessions_opened": 2, "sessions_closed": 1, "sessions_evicted": 2, "peak_retained_bytes": 40})
        );
    }

    #[test]
    fn error_labels_are_the_protocol_codes() {
        let m = Metrics::new();
        for (code, _) in ERROR_CODES {
            m.request(Some(1), Duration::ZERO, Some(code));
        }
        let doc = m.document(Duration::ZERO, "0");
        let errors = doc["methods"][crate::METHODS[1].name]["errors"]
            .as_object()
            .unwrap();
        let labels: Vec<&str> = ERROR_CODES.iter().map(|(_, l)| *l).collect();
        assert_eq!(
            errors.keys().map(String::as_str).collect::<Vec<_>>().len(),
            labels.len()
        );
        for label in labels {
            assert_eq!(errors[label], 1, "{label}");
        }
    }

    #[test]
    fn an_index_past_the_table_counts_as_unknown() {
        let m = Metrics::new();
        m.request(Some(usize::MAX), Duration::ZERO, None);
        assert_eq!(
            m.document(Duration::ZERO, "0")["methods"]["(unknown)"]["requests"],
            1
        );
    }
}
