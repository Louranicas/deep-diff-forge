// DDF integration: daemon metrics for standard monitoring, without a listener (M1 monitoring).
// Design/status: docs/OPERATIONS_AND_DAEMON.md#exporting-metrics-to-prometheus
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fcrates--deep-diff-forge-daemon--src--metrics
//! The `deep-diff-forge.daemon-metrics.v0` document in the Prometheus text exposition format
//! (version 0.0.4), read by `deep-diff-forge daemon metrics --format prometheus`.
//!
//! It is rendered from the document `daemon.metrics` answers, not from the counters, so the two
//! cannot disagree and the daemon gains no endpoint: a file written with `--out` is read by
//! `node_exporter`'s textfile collector. That collector parses this format, not `OpenMetrics` (whose
//! counter `TYPE` line omits `_total`, which a 0.0.4 parser reads as an untyped family), so there
//! is no `# EOF` line. Samples carry no timestamps: the collector refuses a file that has them.
//!
//! A document missing a field is refused, never rendered with a guessed zero.

use std::fmt::Write as _;

use serde_json::{Map, Value};

use crate::metrics::{LATENCY_BUCKETS, bucket_bound};

/// The latency histogram's family name.
const DURATION: &str = "ddf_daemon_request_duration_seconds";

/// One metric family: its `HELP` and `TYPE` lines, then its samples, written together because
/// the format wants each family as one group.
struct Family {
    text: String,
    samples: usize,
}

impl Family {
    fn new(name: &str, kind: &str, help: &str) -> Self {
        Self {
            text: format!("# HELP {name} {help}\n# TYPE {name} {kind}\n"),
            samples: 0,
        }
    }

    /// One sample line: `name{label="value",...} value`.
    fn sample(&mut self, name: &str, labels: &[(&str, &str)], value: &str) {
        self.text.push_str(name);
        for (i, (label, label_value)) in labels.iter().enumerate() {
            self.text.push(if i == 0 { '{' } else { ',' });
            let _ = write!(self.text, "{label}=\"{}\"", escape_label(label_value));
        }
        if !labels.is_empty() {
            self.text.push('}');
        }
        let _ = writeln!(self.text, " {value}");
        self.samples += 1;
    }

    /// Append the family to `out`; one with no samples (no errors yet) is left out.
    fn finish(self, out: &mut String) {
        if self.samples > 0 {
            out.push_str(&self.text);
        }
    }
}

/// A label value with the backslash, the double quote and the line feed escaped.
fn escape_label(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            c => escaped.push(c),
        }
    }
    escaped
}

/// `micros` microseconds as an exact decimal number of seconds (`1500000` is `1.5`).
fn seconds(micros: u64) -> String {
    let (whole, fraction) = (micros / 1_000_000, micros % 1_000_000);
    if fraction == 0 {
        return whole.to_string();
    }
    let fraction = format!("{fraction:06}");
    format!("{whole}.{}", fraction.trim_end_matches('0'))
}

/// A family with one sample, read from one count of the document.
struct Scalar {
    name: &'static str,
    kind: &'static str,
    help: &'static str,
    section: &'static str,
    field: &'static str,
    /// How the count reads: as is, or as seconds when it counts microseconds.
    value: fn(u64) -> String,
}

/// The document's scalar counters and peaks, in the order they are written.
const SCALARS: [Scalar; 14] = [
    Scalar {
        name: "ddf_daemon_admitted_total",
        kind: "counter",
        help: "Requests admitted to run.",
        section: "admission",
        field: "admitted",
        value: plain,
    },
    Scalar {
        name: "ddf_daemon_admission_waits_total",
        kind: "counter",
        help: "Requests admitted after waiting for room.",
        section: "admission",
        field: "waited",
        value: plain,
    },
    Scalar {
        name: "ddf_daemon_admission_wait_seconds_total",
        kind: "counter",
        help: "Seconds requests spent waiting for admission.",
        section: "admission",
        field: "wait_us",
        value: seconds,
    },
    Scalar {
        name: "ddf_daemon_busy_refusals_total",
        kind: "counter",
        help: "Requests refused as busy after the admission wait.",
        section: "admission",
        field: "busy",
        value: plain,
    },
    Scalar {
        name: "ddf_daemon_peak_in_flight_bytes",
        kind: "gauge",
        help: "The most request bytes admitted at once.",
        section: "admission",
        field: "peak_in_flight_bytes",
        value: plain,
    },
    Scalar {
        name: "ddf_daemon_peak_running_requests",
        kind: "gauge",
        help: "The most requests running at once.",
        section: "admission",
        field: "peak_running",
        value: plain,
    },
    Scalar {
        name: "ddf_daemon_connections_accepted_total",
        kind: "counter",
        help: "Connections accepted and given a worker.",
        section: "connections",
        field: "accepted",
        value: plain,
    },
    Scalar {
        name: "ddf_daemon_connection_errors_total",
        kind: "counter",
        help: "Connections that ended in an I/O error.",
        section: "connections",
        field: "errors",
        value: plain,
    },
    Scalar {
        name: "ddf_daemon_panics_total",
        kind: "counter",
        help: "Panics inside a request, contained.",
        section: "engine",
        field: "panics",
        value: plain,
    },
    Scalar {
        name: "ddf_daemon_session_resets_total",
        kind: "counter",
        help: "Session table resets after a panic held it.",
        section: "engine",
        field: "session_resets",
        value: plain,
    },
    Scalar {
        name: "ddf_daemon_sessions_opened_total",
        kind: "counter",
        help: "Sessions opened.",
        section: "engine",
        field: "sessions_opened",
        value: plain,
    },
    Scalar {
        name: "ddf_daemon_sessions_closed_total",
        kind: "counter",
        help: "Sessions closed by their client.",
        section: "engine",
        field: "sessions_closed",
        value: plain,
    },
    Scalar {
        name: "ddf_daemon_sessions_evicted_total",
        kind: "counter",
        help: "Sessions evicted to make room.",
        section: "engine",
        field: "sessions_evicted",
        value: plain,
    },
    Scalar {
        name: "ddf_daemon_peak_retained_bytes",
        kind: "gauge",
        help: "The most bytes held in all sessions at once.",
        section: "engine",
        field: "peak_retained_bytes",
        value: plain,
    },
];

fn plain(n: u64) -> String {
    n.to_string()
}

/// The count at `/section/field` of the document.
fn count(doc: &Value, section: &str, field: &str) -> Result<u64, String> {
    doc.get(section)
        .and_then(|s| s.get(field))
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("the metrics document has no count at {section}.{field}"))
}

/// One method's entry: its requests, its errors by label, and its latency buckets.
struct MethodEntry<'a> {
    name: &'a str,
    requests: u64,
    errors: Vec<(&'a str, u64)>,
    buckets: Vec<u64>,
}

fn method_entry<'a>(name: &'a str, entry: &'a Value) -> Result<MethodEntry<'a>, String> {
    let refuse = |what: &str| format!("the metrics document's method {name:?} has no {what}");
    let requests = entry
        .get("requests")
        .and_then(Value::as_u64)
        .ok_or_else(|| refuse("requests count"))?;
    let errors = entry
        .get("errors")
        .and_then(Value::as_object)
        .ok_or_else(|| refuse("errors object"))?
        .iter()
        .map(|(label, n)| n.as_u64().map(|n| (label.as_str(), n)))
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| refuse("count for every error"))?;
    let buckets = entry
        .get("latency_us")
        .and_then(|l| l.get("buckets"))
        .and_then(Value::as_array)
        .filter(|b| b.len() == LATENCY_BUCKETS)
        .and_then(|b| b.iter().map(Value::as_u64).collect::<Option<Vec<_>>>())
        .ok_or_else(|| refuse("latency_us.buckets of 24 counts"))?;
    Ok(MethodEntry {
        name,
        requests,
        errors,
        buckets,
    })
}

/// The per-method families: requests, errors, and the latency histogram.
fn method_families(methods: &[MethodEntry], out: &mut String) {
    let mut requests = Family::new(
        "ddf_daemon_requests_total",
        "counter",
        "Requests answered, by method; (unknown) is a method the daemon does not have.",
    );
    let mut errors = Family::new(
        "ddf_daemon_request_errors_total",
        "counter",
        "Requests answered with an RPC error, by method and error.",
    );
    let mut latency = Family::new(
        DURATION,
        "histogram",
        "Request latency by method, in log2 microsecond buckets; no _sum, the daemon keeps none.",
    );
    for method in methods {
        let name = method.name;
        requests.sample(
            "ddf_daemon_requests_total",
            &[("method", name)],
            &method.requests.to_string(),
        );
        for (error, n) in &method.errors {
            errors.sample(
                "ddf_daemon_request_errors_total",
                &[("method", name), ("error", error)],
                &n.to_string(),
            );
        }
        let mut cumulative = 0u64;
        for (i, n) in method.buckets.iter().enumerate() {
            cumulative = cumulative.saturating_add(*n);
            let le = bucket_bound(i).map_or_else(|| "+Inf".to_owned(), seconds);
            latency.sample(
                &format!("{DURATION}_bucket"),
                &[("method", name), ("le", &le)],
                &cumulative.to_string(),
            );
        }
        latency.sample(
            &format!("{DURATION}_count"),
            &[("method", name)],
            &cumulative.to_string(),
        );
    }
    requests.finish(out);
    errors.finish(out);
    latency.finish(out);
}

/// `doc`, a `deep-diff-forge.daemon-metrics.v0` document, in the Prometheus text format.
///
/// Every name starts `ddf_daemon_`; counters end `_total` and units are in the name
/// (`_seconds`, `_bytes`). The latency histogram has cumulative `le` buckets in seconds and a
/// `_count`, but no `_sum`: the daemon keeps bucket counts, not total time.
///
/// # Errors
///
/// The reason, when `doc` is not a `deep-diff-forge.daemon-metrics.v0` document with every
/// field this rendering reads.
pub fn prometheus_text(doc: &Value) -> Result<String, String> {
    if doc.get("schema").and_then(Value::as_str) != Some("deep-diff-forge.daemon-metrics.v0") {
        return Err("not a deep-diff-forge.daemon-metrics.v0 document".to_owned());
    }
    let version = doc
        .get("version")
        .and_then(Value::as_str)
        .ok_or("the metrics document has no version")?;
    let uptime = doc
        .get("uptime_secs")
        .and_then(Value::as_u64)
        .ok_or("the metrics document has no uptime_secs")?;
    let methods = doc
        .get("methods")
        .and_then(Value::as_object)
        .map(Map::iter)
        .ok_or("the metrics document has no methods object")?
        .map(|(name, entry)| method_entry(name, entry))
        .collect::<Result<Vec<_>, _>>()?;

    let mut out = String::new();
    let mut info = Family::new(
        "ddf_daemon_build_info",
        "gauge",
        "The daemon's version, as a label; always 1.",
    );
    info.sample("ddf_daemon_build_info", &[("version", version)], "1");
    info.finish(&mut out);
    let mut up = Family::new(
        "ddf_daemon_uptime_seconds",
        "gauge",
        "Seconds since the daemon started; every counter here counts from then.",
    );
    up.sample("ddf_daemon_uptime_seconds", &[], &uptime.to_string());
    up.finish(&mut out);

    method_families(&methods, &mut out);

    for scalar in &SCALARS {
        let mut family = Family::new(scalar.name, scalar.kind, scalar.help);
        let n = count(doc, scalar.section, scalar.field)?;
        family.sample(scalar.name, &[], &(scalar.value)(n));
        family.finish(&mut out);
    }
    let mut refused = Family::new(
        "ddf_daemon_connections_refused_total",
        "counter",
        "Connections refused: at_limit (every worker busy) or peer (another user).",
    );
    for (reason, field) in [("at_limit", "refused_at_limit"), ("peer", "refused_peer")] {
        refused.sample(
            "ddf_daemon_connections_refused_total",
            &[("reason", reason)],
            &count(doc, "connections", field)?.to_string(),
        );
    }
    refused.finish(&mut out);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::time::Duration;

    use super::*;
    use crate::metrics::Metrics;

    /// One parsed sample: its name, its labels (unescaped) and its value.
    type Sample = (String, BTreeMap<String, String>, f64);

    fn is_name(name: &str, colon: bool) -> bool {
        let mut chars = name.chars();
        chars
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || (colon && c == ':'))
            && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || (colon && c == ':'))
    }

    /// The label set `{a="x",b="y"}` at the start of `rest`, and what follows it.
    fn labels(rest: &str) -> Result<(BTreeMap<String, String>, &str), String> {
        let mut out = BTreeMap::new();
        let Some(mut rest) = rest.strip_prefix('{') else {
            return Ok((out, rest));
        };
        loop {
            let eq = rest.find('=').ok_or("a label without =")?;
            let name = &rest[..eq];
            if !is_name(name, false) || name.starts_with("__") {
                return Err(format!("bad label name {name:?}"));
            }
            rest = rest[eq + 1..]
                .strip_prefix('"')
                .ok_or("an unquoted label value")?;
            let mut value = String::new();
            let mut chars = rest.char_indices();
            let end = loop {
                match chars.next().ok_or("an unterminated label value")? {
                    (i, '"') => break i,
                    (_, '\\') => match chars.next().ok_or("a dangling escape")?.1 {
                        '\\' => value.push('\\'),
                        '"' => value.push('"'),
                        'n' => value.push('\n'),
                        c => return Err(format!("the escape \\{c}")),
                    },
                    (_, '\n') => return Err("a raw line feed in a label value".into()),
                    (_, c) => value.push(c),
                }
            };
            if out.insert(name.to_owned(), value).is_some() {
                return Err(format!("the label {name} twice"));
            }
            rest = &rest[end + 1..];
            if let Some(after) = rest.strip_prefix('}') {
                return Ok((out, after));
            }
            rest = rest
                .strip_prefix(',')
                .ok_or("labels not separated by a comma")?;
        }
    }

    /// A strict reader of the Prometheus 0.0.4 text format (`exposition_formats.md`), as this
    /// module should write it: every line ends in a line feed; each family has one `HELP` and one
    /// `TYPE` line before its samples and its samples in one group; names and label names are
    /// identifiers and label values escape `\`, `"` and line feed; a line has no timestamp; each
    /// series is unique; a histogram's buckets rise in `le` and in count, end at `+Inf`, and its
    /// `_count` equals the `+Inf` bucket. Our own conventions too: a `ddf_daemon_` prefix, and
    /// counters named `_total` with no negative value.
    fn strict_parse(text: &str) -> Result<Vec<Sample>, String> {
        if !text.ends_with('\n') || text.contains('\r') {
            return Err("the text must end in a line feed and have no carriage return".into());
        }
        let mut helped = BTreeSet::new();
        let mut typed: BTreeMap<String, String> = BTreeMap::new();
        let mut current: Option<String> = None;
        let mut closed = BTreeSet::new();
        let mut series = BTreeSet::new();
        let mut samples = Vec::new();
        for line in text.lines() {
            if let Some(comment) = line.strip_prefix("# ") {
                let mut parts = comment.splitn(3, ' ');
                let (keyword, name, rest) = (parts.next(), parts.next(), parts.next());
                let name = name.ok_or("a comment with no name")?.to_owned();
                if !is_name(&name, true) || !name.starts_with("ddf_daemon_") {
                    return Err(format!("bad family name {name:?}"));
                }
                if current.as_ref() != Some(&name) {
                    if let Some(previous) = current.replace(name.clone()) {
                        closed.insert(previous);
                    }
                    if closed.contains(&name) || typed.contains_key(&name) {
                        return Err(format!("the family {name} is split"));
                    }
                }
                match (keyword, rest) {
                    (Some("HELP"), Some(help)) => {
                        let unescaped = help.replace("\\\\", "").replace("\\n", "");
                        if unescaped.contains('\\') || !helped.insert(name.clone()) {
                            return Err(format!("bad or second HELP for {name}"));
                        }
                    }
                    (Some("TYPE"), Some(kind)) => {
                        if !["counter", "gauge", "histogram"].contains(&kind) {
                            return Err(format!("the type {kind:?}"));
                        }
                        if kind == "counter" && !name.ends_with("_total") {
                            return Err(format!("the counter {name} does not end _total"));
                        }
                        typed.insert(name.clone(), kind.to_owned());
                    }
                    _ => return Err(format!("a comment line that is not HELP or TYPE: {line}")),
                }
                continue;
            }
            let name_end = line.find(['{', ' ']).ok_or("a sample with no value")?;
            let name = &line[..name_end];
            let (labels, rest) = labels(&line[name_end..])?;
            let value = rest.strip_prefix(' ').ok_or("no space before the value")?;
            if value.contains([' ', '\t']) {
                return Err(format!("a timestamp or extra token: {line}"));
            }
            let value: f64 = match value {
                "+Inf" => f64::INFINITY,
                v => v.parse().map_err(|_| format!("the value {v:?}"))?,
            };
            let family = current.clone().ok_or("a sample before any TYPE")?;
            let kind = typed.get(&family).ok_or("a sample before its TYPE")?;
            let belongs = if kind == "histogram" {
                [
                    format!("{family}_bucket"),
                    format!("{family}_count"),
                    format!("{family}_sum"),
                ]
                .contains(&name.to_owned())
            } else {
                name == family
            };
            if !belongs || !helped.contains(&family) {
                return Err(format!("the sample {name} is outside its family {family}"));
            }
            if kind == "counter" && value < 0.0 {
                return Err(format!("the counter {name} is negative"));
            }
            if (kind == "histogram") != labels.contains_key("le") && !name.ends_with("_count") {
                return Err(format!(
                    "le on a non-bucket, or a bucket without le: {line}"
                ));
            }
            if !series.insert((name.to_owned(), labels.clone())) {
                return Err(format!("the series {line} twice"));
            }
            samples.push((name.to_owned(), labels, value));
        }
        check_histograms(&samples, &typed)?;
        Ok(samples)
    }

    fn check_histograms(
        samples: &[Sample],
        typed: &BTreeMap<String, String>,
    ) -> Result<(), String> {
        for (family, _) in typed.iter().filter(|(_, kind)| *kind == "histogram") {
            // Per series (labels without le): the buckets in order, then the count.
            let mut buckets: BTreeMap<BTreeMap<String, String>, Vec<(f64, f64)>> = BTreeMap::new();
            let mut counts = BTreeMap::new();
            for (name, labels, value) in samples {
                let mut key = labels.clone();
                if *name == format!("{family}_bucket") {
                    let le = key.remove("le").ok_or("a bucket without le")?;
                    let le = if le == "+Inf" {
                        f64::INFINITY
                    } else {
                        le.parse().map_err(|_| format!("the le {le:?}"))?
                    };
                    buckets.entry(key).or_default().push((le, *value));
                } else if *name == format!("{family}_count") {
                    counts.insert(key, *value);
                }
            }
            for (key, list) in &buckets {
                if list.windows(2).any(|w| w[0].0 >= w[1].0 || w[0].1 > w[1].1) {
                    return Err(format!("{family} buckets not rising for {key:?}"));
                }
                let last = list.last().ok_or("no buckets")?;
                if last.0 != f64::INFINITY || counts.get(key) != Some(&last.1) {
                    return Err(format!(
                        "{family} has no +Inf bucket equal to _count for {key:?}"
                    ));
                }
            }
            if buckets.len() != counts.len() {
                return Err(format!("{family} has a _count without buckets"));
            }
        }
        Ok(())
    }

    /// The value of the one sample named `name` with exactly `labels`.
    fn value(samples: &[Sample], name: &str, labels: &[(&str, &str)]) -> Option<f64> {
        let labels: BTreeMap<String, String> = labels
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        samples
            .iter()
            .find(|(n, l, _)| n == name && *l == labels)
            .map(|(_, _, v)| *v)
    }

    /// Metrics whose every counter differs from every other, so a field read in place of another
    /// shows.
    fn busy_metrics() -> Metrics {
        let m = Metrics::new();
        m.request(Some(0), Duration::from_micros(5), None);
        m.request(
            Some(0),
            Duration::from_micros(1500),
            Some(crate::protocol::INVALID_PARAMS),
        );
        m.request(None, Duration::from_secs(60), Some(-1));
        for _ in 0..2 {
            m.admitted(Duration::ZERO, 10, 1);
        }
        m.admitted(Duration::from_micros(1_500_250), 7_000, 3);
        for _ in 0..4 {
            m.busy();
        }
        for _ in 0..5 {
            m.accepted();
        }
        for _ in 0..6 {
            m.refused_at_limit();
        }
        for _ in 0..8 {
            m.refused_peer();
        }
        for _ in 0..9 {
            m.connection_error();
        }
        for _ in 0..11 {
            m.panic();
        }
        for _ in 0..12 {
            m.session_reset();
        }
        for n in 0..13 {
            m.session_opened(4_000 + n);
        }
        for _ in 0..14 {
            m.session_closed();
        }
        m.sessions_evicted(15);
        m
    }

    #[test]
    fn every_counter_is_exposed_under_its_name_and_the_text_is_valid() {
        let doc = busy_metrics().document(Duration::from_secs(99), "9.9.9");
        let text = prometheus_text(&doc).unwrap();
        let samples = strict_parse(&text).unwrap_or_else(|e| panic!("{e}\n{text}"));
        let scalar = |name: &str| value(&samples, name, &[]);
        assert_eq!(
            value(&samples, "ddf_daemon_build_info", &[("version", "9.9.9")]),
            Some(1.0)
        );
        let expected = [
            ("ddf_daemon_uptime_seconds", 99.0),
            ("ddf_daemon_admitted_total", 3.0),
            ("ddf_daemon_admission_waits_total", 1.0),
            ("ddf_daemon_admission_wait_seconds_total", 1.500_25),
            ("ddf_daemon_busy_refusals_total", 4.0),
            ("ddf_daemon_peak_in_flight_bytes", 7_000.0),
            ("ddf_daemon_peak_running_requests", 3.0),
            ("ddf_daemon_connections_accepted_total", 5.0),
            ("ddf_daemon_connection_errors_total", 9.0),
            ("ddf_daemon_panics_total", 11.0),
            ("ddf_daemon_session_resets_total", 12.0),
            ("ddf_daemon_sessions_opened_total", 13.0),
            ("ddf_daemon_sessions_closed_total", 14.0),
            ("ddf_daemon_sessions_evicted_total", 15.0),
            ("ddf_daemon_peak_retained_bytes", 4_012.0),
        ];
        for (name, want) in expected {
            assert_eq!(scalar(name), Some(want), "{name}");
        }
        let refused = "ddf_daemon_connections_refused_total";
        assert_eq!(
            value(&samples, refused, &[("reason", "at_limit")]),
            Some(6.0)
        );
        assert_eq!(value(&samples, refused, &[("reason", "peer")]), Some(8.0));
        let first = crate::METHODS[0].name;
        let requests = "ddf_daemon_requests_total";
        assert_eq!(value(&samples, requests, &[("method", first)]), Some(2.0));
        assert_eq!(
            value(&samples, requests, &[("method", "(unknown)")]),
            Some(1.0)
        );
        let errors = "ddf_daemon_request_errors_total";
        assert_eq!(
            value(
                &samples,
                errors,
                &[("method", first), ("error", "invalid_params")]
            ),
            Some(1.0)
        );
        assert_eq!(
            value(
                &samples,
                errors,
                &[("method", "(unknown)"), ("error", "other")]
            ),
            Some(1.0)
        );
        // 5 us is under 8 us; 1500 us under 2048 us; 60 s only under +Inf.
        let bucket = |method: &str, le: &str| {
            value(
                &samples,
                "ddf_daemon_request_duration_seconds_bucket",
                &[("method", method), ("le", le)],
            )
        };
        assert_eq!(bucket(first, "0.000004"), Some(0.0));
        assert_eq!(bucket(first, "0.000008"), Some(1.0));
        assert_eq!(bucket(first, "0.001024"), Some(1.0));
        assert_eq!(bucket(first, "0.002048"), Some(2.0));
        assert_eq!(bucket(first, "4.194304"), Some(2.0));
        assert_eq!(bucket("(unknown)", "4.194304"), Some(0.0));
        assert_eq!(bucket("(unknown)", "+Inf"), Some(1.0));
        assert_eq!(
            samples
                .iter()
                .filter(|(n, l, _)| n.ends_with("_bucket") && l["method"] == first)
                .count(),
            LATENCY_BUCKETS
        );
        assert!(
            samples.iter().all(|(n, _, _)| !n.ends_with("_sum")),
            "the daemon keeps no latency sum"
        );
        assert!(text.ends_with(" 8\n"), "the last family is the refusals");
        assert_eq!(prometheus_text(&doc).unwrap(), text, "deterministic");
    }

    #[test]
    fn a_family_without_samples_is_left_out() {
        let m = Metrics::new();
        m.request(Some(0), Duration::ZERO, None);
        let text = prometheus_text(&m.document(Duration::ZERO, "0")).unwrap();
        strict_parse(&text).unwrap();
        assert!(!text.contains("ddf_daemon_request_errors_total"), "{text}");
        assert!(text.contains("# TYPE ddf_daemon_requests_total counter\n"));
        let idle = prometheus_text(&Metrics::new().document(Duration::ZERO, "0")).unwrap();
        strict_parse(&idle).unwrap();
        assert!(!idle.contains("ddf_daemon_requests_total"), "{idle}");
        assert!(idle.contains("ddf_daemon_panics_total 0\n"), "{idle}");
    }

    #[test]
    fn label_values_are_escaped() {
        let mut doc = Metrics::new().document(Duration::ZERO, "1\"\\\n2");
        let hostile = "a\"b\\c\nd}{,=";
        doc["methods"][hostile] = serde_json::json!({
            "requests": 1,
            "errors": {"x\"y": 1},
            "latency_us": {"buckets": vec![0u64; LATENCY_BUCKETS]},
        });
        let text = prometheus_text(&doc).unwrap();
        assert!(text.contains("{version=\"1\\\"\\\\\\n2\"} 1\n"), "{text}");
        let samples = strict_parse(&text).unwrap_or_else(|e| panic!("{e}\n{text}"));
        assert_eq!(
            value(
                &samples,
                "ddf_daemon_build_info",
                &[("version", "1\"\\\n2")]
            ),
            Some(1.0)
        );
        assert_eq!(
            value(
                &samples,
                "ddf_daemon_request_errors_total",
                &[("method", hostile), ("error", "x\"y")]
            ),
            Some(1.0)
        );
        assert_eq!(escape_label("plain"), "plain");
        assert_eq!(escape_label("\\\"\n"), "\\\\\\\"\\n");
    }

    #[test]
    fn microseconds_read_as_exact_seconds() {
        assert_eq!(seconds(0), "0");
        assert_eq!(seconds(1), "0.000001");
        assert_eq!(seconds(250), "0.00025");
        assert_eq!(seconds(1_000_000), "1");
        assert_eq!(seconds(1_500_000), "1.5");
        assert_eq!(seconds(1 << 22), "4.194304");
        assert_eq!(seconds(u64::MAX), "18446744073709.551615");
    }

    #[test]
    fn a_document_missing_a_field_is_refused() {
        let doc = busy_metrics().document(Duration::ZERO, "0");
        assert!(prometheus_text(&doc).is_ok());
        let first = crate::METHODS[0].name;
        let mut removals: Vec<Vec<&str>> = vec![
            vec!["schema"],
            vec!["version"],
            vec!["uptime_secs"],
            vec!["methods"],
            vec!["methods", first, "requests"],
            vec!["methods", first, "errors"],
            vec!["methods", first, "latency_us"],
        ];
        for section in ["admission", "connections", "engine"] {
            for field in doc[section].as_object().unwrap().keys() {
                removals.push(vec![section, field]);
            }
        }
        for path in removals {
            let mut broken = doc.clone();
            let (last, parents) = path.split_last().unwrap();
            let mut at = &mut broken;
            for p in parents {
                at = &mut at[*p];
            }
            at.as_object_mut().unwrap().remove(*last).unwrap();
            let refusal = prometheus_text(&broken).unwrap_err();
            assert!(
                refusal.contains(last) || *last == "schema",
                "{path:?}: {refusal}"
            );
        }
        for (pointer, bad) in [
            (
                "/schema",
                serde_json::json!("deep-diff-forge.daemon-metrics.v1"),
            ),
            ("/uptime_secs", serde_json::json!(-1)),
            ("/methods", serde_json::json!([])),
            ("/methods/(unknown)/errors/other", serde_json::json!("1")),
            (
                "/methods/(unknown)/latency_us/buckets",
                serde_json::json!(vec![0u64; 23]),
            ),
            (
                "/methods/(unknown)/latency_us/buckets/3",
                serde_json::json!(1.5),
            ),
            ("/admission/busy", serde_json::json!(null)),
        ] {
            let mut broken = doc.clone();
            *broken.pointer_mut(pointer).unwrap() = bad;
            assert!(prometheus_text(&broken).is_err(), "{pointer}");
        }
    }

    /// The reader refuses what the format forbids, so its pass above means something.
    #[test]
    fn the_strict_reader_refuses_invalid_text() {
        let good = "# HELP ddf_daemon_x_total X.\n# TYPE ddf_daemon_x_total counter\nddf_daemon_x_total{a=\"1\"} 2\n";
        assert!(strict_parse(good).is_ok());
        let histogram = "# HELP ddf_daemon_h H.\n# TYPE ddf_daemon_h histogram\n";
        for bad in [
            good.trim_end(),
            &good.replace("_total{", "_totalx{"),
            &good.replace("\"1\"", "\"a\nb\""),
            &good.replace("\"1\"", "\"\\t\""),
            &good.replace("a=", "__a="),
            &good.replace("} 2", "} 2 1700000000000"),
            &good.replace("} 2", "} -2"),
            &good.replace("x_total", "x"),
            &good.replace("counter", "summary"),
            &format!("{good}ddf_daemon_x_total{{a=\"1\"}} 3\n"),
            &format!(
                "{good}# HELP ddf_daemon_y Y.\n# TYPE ddf_daemon_y gauge\nddf_daemon_y 1\nddf_daemon_x_total 1\n"
            ),
            "# TYPE ddf_daemon_y gauge\nddf_daemon_y 1\n",
            &format!(
                "{histogram}ddf_daemon_h_bucket{{le=\"1\"}} 2\nddf_daemon_h_bucket{{le=\"+Inf\"}} 1\nddf_daemon_h_count 1\n"
            ),
            &format!("{histogram}ddf_daemon_h_bucket{{le=\"1\"}} 1\nddf_daemon_h_count 1\n"),
            &format!("{histogram}ddf_daemon_h_bucket{{le=\"+Inf\"}} 1\nddf_daemon_h_count 2\n"),
        ] {
            assert!(strict_parse(bad).is_err(), "{bad}");
        }
        assert!(strict_parse(&format!(
            "{histogram}ddf_daemon_h_bucket{{le=\"1\"}} 1\nddf_daemon_h_bucket{{le=\"+Inf\"}} 2\nddf_daemon_h_count 2\n"
        ))
        .is_ok());
    }
}
