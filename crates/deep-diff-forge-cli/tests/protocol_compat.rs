//! F7.d1: engine protocol 2 as release 0.10.0 published it. `fixtures/protocol/v2-transcript.jsonl`
//! holds every request of `fixtures/protocol/v2-requests.txt` and the line one `rpc` process of the
//! published v0.10.0 binary answered (its SHA-256 in the first line;
//! `scripts/record_protocol_transcript.sh` recorded it). This tree's `rpc` answers the same
//! requests in one process, and each answer is held to the recorded one under the replay rule
//! (docs/reference/mcp-sdk-daemon.md): the shape (member names, types and order), the error
//! codes and the error messages exactly, and every value exactly after the named normalisations.
#![expect(
    clippy::disallowed_methods,
    reason = "integration test: drives real processes, the environment and wall-clock time"
)]

use serde_json::Value;
use std::io::{BufRead as _, BufReader, Write as _};
use std::process::{Command, Stdio};

const TRANSCRIPT: &str = include_str!("../../../fixtures/protocol/v2-transcript.jsonl");
const PUBLISHED: &str = "121503a8d292da951c31e33e8b59bf33b54dd316e3d6ec3055e5c4bfac18e9ff";

/// Methods whose result's top-level `version` is the engine's version (the rule names them;
/// `daemon.metrics` by ruling R-F1).
const VERSIONED_RESULTS: &[&str] = &[
    "engine.initialize",
    "daemon.health",
    "daemon.status",
    "daemon.metrics",
];

/// Values of the run, not of the engine (rulings R-F2/F3), each by its exact path in a method's
/// `result` (`*` is the name of a method in `daemon.metrics`' `methods`). Two recordings of the
/// same published binary differ in these and nothing else
/// (`scripts/record_protocol_transcript.sh --check`). Each becomes a placeholder of its type,
/// an array keeping its length, so the shape is still compared; every other member of these
/// results, the counters included, is compared exactly.
const RUN_DEPENDENT: &[(&str, &[&str])] = &[
    ("daemon.health", &["pid"]),
    ("daemon.status", &["uptime_secs"]),
    ("daemon.metrics", &["admission", "wait_us"]),
    ("daemon.metrics", &["methods", "*", "latency_us", "buckets"]),
    (
        "daemon.metrics",
        &["methods", "*", "latency_us", "p50_at_most"],
    ),
    (
        "daemon.metrics",
        &["methods", "*", "latency_us", "p95_at_most"],
    ),
    (
        "daemon.metrics",
        &["methods", "*", "latency_us", "p99_at_most"],
    ),
];

/// The placeholder of a run-dependent value: 0 for a number, each item's for an array.
fn placeholder(value: &mut Value) {
    match value {
        Value::Number(_) => *value = Value::from(0),
        Value::Array(items) => items.iter_mut().for_each(placeholder),
        _ => {}
    }
}

fn at_path(value: &mut Value, path: &[&str]) {
    match path {
        [] => placeholder(value),
        ["*", rest @ ..] => {
            if let Some(members) = value.as_object_mut() {
                members.values_mut().for_each(|v| at_path(v, rest));
            }
        }
        [name, rest @ ..] => {
            if let Some(next) = value.get_mut(*name) {
                at_path(next, rest);
            }
        }
    }
}

struct Record {
    request: String,
    response: String,
}

fn transcript() -> Vec<Record> {
    let mut lines = TRANSCRIPT.lines();
    let head: Value = serde_json::from_str(lines.next().unwrap()).unwrap();
    assert_eq!(head["binary_sha256"], PUBLISHED, "{head}");
    assert_eq!(head["binary"], "the published v0.10.0 binary");
    let records: Vec<Record> = lines
        .map(|line| {
            let record: Value = serde_json::from_str(line).unwrap();
            Record {
                request: record["request"].as_str().unwrap().to_owned(),
                response: record["response"].as_str().unwrap().to_owned(),
            }
        })
        .collect();
    assert_eq!(head["requests"], records.len());
    records
}

/// Every request line through one `rpc` process of this tree, its answer lines in order.
fn replay(records: &[Record]) -> Vec<String> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .arg("rpc")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let input: String = records.iter().fold(String::new(), |mut all, r| {
        all.push_str(&r.request);
        all.push('\n');
        all
    });
    let writer = std::thread::spawn(move || {
        // The last request ends the session; anything the process no longer reads is not ours.
        let _ = stdin.write_all(input.as_bytes());
    });
    let answers: Vec<String> = BufReader::new(child.stdout.take().unwrap())
        .lines()
        .map(Result::unwrap)
        .collect();
    writer.join().unwrap();
    let _ = child.kill();
    let _ = child.wait();
    answers
}

/// The method a request line names, for the rule's per-method normalisations.
fn method(request: &str) -> String {
    serde_json::from_str::<Value>(request)
        .ok()
        .and_then(|r| r.get("method").and_then(Value::as_str).map(str::to_owned))
        .unwrap_or_default()
}

/// The normalisations the replay rule names: every `tool.version` (and the `version` of
/// [`VERSIONED_RESULTS`]) becomes a placeholder, every `tool.scorer_sha256` is dropped, in a
/// `canonical_json` text (a rank document's exact bytes) the same two members of its `tool` line,
/// and each [`RUN_DEPENDENT`] value becomes its placeholder.
fn normalise(method: &str, answer: &mut Value) {
    let Some(result) = answer.get_mut("result") else {
        return;
    };
    if VERSIONED_RESULTS.contains(&method)
        && let Some(version) = result.get_mut("version")
    {
        *version = Value::from("<version>");
    }
    for (_, path) in RUN_DEPENDENT.iter().filter(|(m, _)| *m == method) {
        at_path(result, path);
    }
    normalise_tools(result);
}

fn normalise_tools(value: &mut Value) {
    match value {
        Value::Object(members) => {
            if let Some(Value::Object(tool)) = members.get_mut("tool") {
                if tool.contains_key("version") {
                    tool.insert("version".into(), Value::from("<version>"));
                }
                tool.remove("scorer_sha256");
            }
            if let Some(Value::String(text)) = members.get_mut("canonical_json") {
                *text = canonical_tool_line(text);
            }
            members.values_mut().for_each(normalise_tools);
        }
        Value::Array(items) => items.iter_mut().for_each(normalise_tools),
        _ => {}
    }
}

/// A canonical document's text with its `tool` line's version replaced and its scorer dropped;
/// every other byte is kept, so member order and spacing are still compared.
fn canonical_tool_line(text: &str) -> String {
    text.split_inclusive('\n')
        .map(|line| {
            let Some(rest) = line.strip_prefix("  \"tool\": ") else {
                return line.to_owned();
            };
            let mut line = format!("  \"tool\": {rest}");
            if let Some(at) = line.find("\"version\": \"") {
                let start = at + "\"version\": \"".len();
                if let Some(len) = line[start..].find('"') {
                    line.replace_range(start..start + len, "<version>");
                }
            }
            let scorer = ", \"scorer_sha256\": \"";
            if let Some(at) = line.find(scorer) {
                line.replace_range(at..=at + scorer.len() + 64, "");
            }
            line
        })
        .collect()
}

/// Every place two values differ, as `path: before -> after` (a member added or missing, a
/// changed type or value, an array of another length).
fn differences(path: &str, before: &Value, after: &Value, out: &mut Vec<String>) {
    match (before, after) {
        (Value::Object(b), Value::Object(a)) => {
            for (key, value) in b {
                match a.get(key) {
                    Some(other) => differences(&format!("{path}.{key}"), value, other, out),
                    None => out.push(format!("{path}.{key}: member gone (was {})", short(value))),
                }
            }
            for key in a.keys().filter(|k| !b.contains_key(*k)) {
                out.push(format!("{path}.{key}: member added ({})", short(&a[key])));
            }
        }
        (Value::Array(b), Value::Array(a)) if b.len() == a.len() => {
            for (i, (x, y)) in b.iter().zip(a).enumerate() {
                differences(&format!("{path}[{i}]"), x, y, out);
            }
        }
        (Value::String(b), Value::String(a)) if b.contains('\n') && b != a => {
            let line = b.lines().zip(a.lines()).position(|(x, y)| x != y);
            out.push(format!(
                "{path}: text differs at line {line:?}: {} -> {}",
                short(&Value::from(
                    line.and_then(|n| b.lines().nth(n)).unwrap_or("")
                )),
                short(&Value::from(
                    line.and_then(|n| a.lines().nth(n)).unwrap_or("")
                ))
            ));
        }
        _ if before != after => {
            out.push(format!("{path}: {} -> {}", short(before), short(after)));
        }
        _ => {}
    }
}

fn short(value: &Value) -> String {
    let text = value.to_string();
    match text.char_indices().nth(160) {
        Some((at, _)) => format!("{}...", &text[..at]),
        None => text,
    }
}

/// The replay rule (F7.d1). Each answer is one JSON value written as the daemon writes every
/// answer (serde's compact form, members sorted, then `json_line_safe`), so equal values mean
/// equal member order on the wire; a `canonical_json` text is compared byte for byte but for the
/// named members of its `tool` line. A difference names its request line and member.
#[test]
fn a_protocol_2_transcript_keeps_its_shapes() {
    let records = transcript();
    let answers = replay(&records);
    let (mut found, mut changed) = (Vec::new(), Vec::new());
    if answers.len() != records.len() {
        found.push(format!(
            "{} requests, {} answers",
            records.len(),
            answers.len()
        ));
    }
    for (n, (record, answer)) in records.iter().zip(&answers).enumerate() {
        let line = n + 2;
        let method = method(&record.request);
        let mut before: Value = serde_json::from_str(&record.response).unwrap();
        let mut after: Value = serde_json::from_str(answer).unwrap();
        for (who, raw, value) in [
            ("recorded", &record.response, &before),
            ("tree", answer, &after),
        ] {
            let written = deep_diff_forge_core::json_line_safe(&value.to_string()).into_owned();
            if &written != raw {
                found.push(format!(
                    "transcript line {line} ({method}): the {who} answer is not in the wire form"
                ));
            }
        }
        if let Some(change) = protocol_3(&record.request, &mut before, &after) {
            changed.push(format!("transcript line {line} ({method}): {change}"));
        }
        normalise(&method, &mut before);
        normalise(&method, &mut after);
        let mut here = Vec::new();
        differences("", &before, &after, &mut here);
        found.extend(
            here.into_iter()
                .map(|d| format!("transcript line {line} ({method}) {d}")),
        );
    }
    assert!(
        found.is_empty(),
        "{} differences from the published protocol 2:\n{}",
        found.len(),
        found.join("\n")
    );
    // Every named change of protocol 3 was met, and no other.
    assert_eq!(changed.len(), PROTOCOL_3_CHANGES, "{}", changed.join("\n"));
}

/// How many transcript answers protocol 3 changes, each named in [`protocol_3`].
const PROTOCOL_3_CHANGES: usize = 12;

/// The changes of protocol 3 to a recorded protocol-2 answer (F5a.2, the replay rule's named
/// members), applied to `recorded` so that the rest is compared exactly: the `protocol` that
/// `engine.initialize` (when the client names none), `engine.methods`, `daemon.health` and
/// `daemon.status` report; `diff.diagnostics` added to the lists; the four refusals of a
/// protocol the daemon does not speak, whose message states the newest it speaks; and the two
/// requests a protocol-2 daemon refused and this one serves (`protocol: 3` and the method
/// itself), checked by what they must hold. Returns the change it named.
fn protocol_3(request: &str, recorded: &mut Value, tree: &Value) -> Option<String> {
    let request: Value = serde_json::from_str(request).ok()?;
    let method = request.get("method")?.as_str()?;
    let asked = request
        .get("params")
        .and_then(|p| p.get("protocol"))
        .cloned();
    let entry = |m: &deep_diff_forge_daemon::MethodSpec| serde_json::json!({"name": m.name, "params": m.params, "result": m.result, "since": m.since});
    let added = deep_diff_forge_daemon::METHODS
        .iter()
        .find(|m| m.name == "diff.diagnostics")?;
    let refused = recorded.get("error").is_some();
    match (method, asked, refused) {
        ("engine.initialize", None, false) => {
            recorded["result"]["protocol"] = 3.into();
            recorded["result"]["methods"]
                .as_array_mut()?
                .push(added.name.into());
            Some("protocol 3; methods lists diff.diagnostics".into())
        }
        ("engine.methods", _, false) => {
            recorded["result"]["protocol"] = 3.into();
            recorded["result"]["methods"]
                .as_array_mut()?
                .push(entry(added));
            Some("protocol 3; one methods entry, diff.diagnostics since 3".into())
        }
        ("daemon.health" | "daemon.status", _, false) => {
            recorded["result"]["protocol"] = 3.into();
            Some("protocol 3".into())
        }
        ("engine.initialize", Some(asked), true) if asked == 3 => {
            let served = &tree["result"];
            assert_eq!(served["protocol"], 3, "{tree}");
            assert!(
                served["methods"].as_array()?.contains(&added.name.into()),
                "{tree}"
            );
            *recorded = tree.clone();
            Some("protocol 3 is served (error 2 before), listing diff.diagnostics".into())
        }
        ("engine.initialize", Some(_), true) => {
            let message = recorded["error"]["message"].as_str()?;
            let now = message.strip_suffix("this daemon speaks 0 to 2")?;
            recorded["error"]["message"] = format!("{now}this daemon speaks 0 to 3").into();
            Some("the refusal names the newest protocol, 3".into())
        }
        ("diff.diagnostics", _, true) => {
            assert_eq!(
                tree["result"]["schema"], "deep-diff-forge.diagnostics.v0",
                "{tree}"
            );
            *recorded = tree.clone();
            Some("diff.diagnostics is served (-32601 before)".into())
        }
        _ => None,
    }
}

/// F5a.2: a client of protocol 3 and a daemon of release 0.10.0. The old daemon is a stub that
/// answers from the transcript, so no old binary is needed: `engine.initialize` with
/// `protocol: 3` is error 2 naming the versions it speaks, and `diff.diagnostics` is -32601,
/// so a client knows the daemon is too old (F5b turns that into `ProtocolTooOld`). This tree's
/// daemon serves both.
#[test]
fn a_new_client_on_a_protocol_2_daemon_gets_a_named_refusal() {
    let records = transcript();
    let old = |method: &str, protocol: Option<u64>| -> (&Record, Value) {
        let record = records
            .iter()
            .find(|r| {
                let request: Value = serde_json::from_str(&r.request).unwrap_or_default();
                request["method"] == method
                    && protocol.is_none_or(|p| request["params"]["protocol"] == p)
            })
            .unwrap();
        (record, serde_json::from_str(&record.response).unwrap())
    };
    let (init, refused) = old("engine.initialize", Some(3));
    assert_eq!(
        refused["error"],
        serde_json::json!({"code": 2, "message": "protocol 3 is not supported; this daemon speaks 0 to 2"})
    );
    let (call, missing) = old("diff.diagnostics", None);
    assert_eq!(
        missing["error"],
        serde_json::json!({"code": -32601, "message": "method not found: diff.diagnostics"})
    );
    let shutdown = records.last().unwrap();
    let new: Vec<Value> = replay(&[
        Record {
            request: init.request.clone(),
            response: String::new(),
        },
        Record {
            request: call.request.clone(),
            response: String::new(),
        },
        Record {
            request: shutdown.request.clone(),
            response: String::new(),
        },
    ])
    .iter()
    .map(|a| serde_json::from_str(a).unwrap())
    .collect();
    assert_eq!(new[0]["result"]["protocol"], 3, "{}", new[0]);
    assert_eq!(
        new[1]["result"]["schema"], "deep-diff-forge.diagnostics.v0",
        "{}",
        new[1]
    );
}
