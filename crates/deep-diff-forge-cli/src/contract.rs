// Module index: docs/ARCHITECTURE.md#crates-as-built-030
// DDF slice A: the machine contract (`ddf.result.v1`, the exit-code registry, `schema`, `explain`,
// `envelope`). Design/status: docs/ROADMAP.md#status-2026-10-26 and
// docs/reference/output-and-exit-codes.md "Machine contract".
//! The `ddf.result.v1` envelope over the exit-code registry (`deep_diff_forge_core::exit`).
//!
//! `envelope -- <command…>` runs this binary as a child with the given arguments and wraps what it
//! did, whatever the command, in one JSON object: its exit status and the registry's name for it,
//! its stdout (parsed when it is one JSON value), and its stderr. The child's own output is never
//! changed, so every existing consumer keeps working, and the envelope exits with the child's code.

use crate::mcp::entry_json;
use deep_diff_forge_core::exit::{self, EXIT_CODES, lookup};
use serde_json::{Value, json};
use std::fmt::Write as _;
use std::io::Read;
use std::process::{Command, Stdio};

/// The envelope's schema identifier.
pub const RESULT_SCHEMA: &str = "ddf.result.v1";
/// The exit-code registry's schema identifier.
pub const EXIT_CODES_SCHEMA: &str = "ddf.exit-codes.v1";
/// The signal vocabulary's schema identifier.
pub const SIGNALS_SCHEMA: &str = "deep-diff-forge.signals.v0";
/// The diagnostics document's schema identifier (`docs/reference/diagnostics.md`).
pub const DIAGNOSTICS_SCHEMA: &str = "deep-diff-forge.diagnostics.v0";
/// The schema identifier of one signal's explanation.
pub const SIGNAL_EXPLAIN_SCHEMA: &str = "deep-diff-forge.signal-explain.v0";
/// The usage line of `explain`.
pub const EXPLAIN_USAGE: &str = "explain {<code> [--family engine|council] | signal <label> [--audience agent|person]} [--json]";
/// Captured stdout beyond this many bytes is cut and the envelope reports `truncated`.
pub const STDOUT_CAP: usize = 16 * 1024 * 1024;
/// Captured stderr beyond this many bytes is cut and the envelope reports `truncated`.
pub const STDERR_CAP: usize = 1024 * 1024;

/// The command family an argument vector belongs to.
pub fn family_of(args: &[String]) -> &'static str {
    if args.first().map(String::as_str) == Some("council") {
        "council"
    } else {
        "engine"
    }
}

/// `explain {<code> [--family engine|council] | signal <label>} [--json]`: what an exit code
/// means, or why a signal matters and what answers it.
pub fn explain(args: &[String]) -> Result<String, String> {
    if args.first().map(String::as_str) == Some("signal") {
        return explain_signal(&args[1..]);
    }
    let mut family = "engine";
    let mut json = false;
    let mut code = None;
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--json" => json = true,
            "--family" => {
                family = match rest.next().map(String::as_str) {
                    Some("engine") => "engine",
                    Some("council") => "council",
                    _ => return Err("--family takes engine or council".into()),
                };
            }
            value if code.is_none() => {
                code = Some(
                    value
                        .parse::<i32>()
                        .map_err(|_| format!("not an exit code: {value}"))?,
                );
            }
            value => return Err(format!("unexpected argument: {value}")),
        }
    }
    let code = code.ok_or_else(|| format!("usage: {EXPLAIN_USAGE}"))?;
    let entry = lookup(family, code)
        .ok_or_else(|| format!("exit code {code} is not documented for the {family} family"))?;
    if json {
        return Ok(format!("{}\n", crate::mcp::explained_json(entry)));
    }
    let mut text = format!(
        "{} {} ({}): {}\n",
        entry.family, entry.code, entry.name, entry.meaning
    );
    // F7.h1: engine 7 has one meaning and names its reason class on stderr.
    if (entry.family, entry.code) == ("engine", exit::INPUT_CONTRACT_REFUSED) {
        text.push_str(REFUSAL_PREAMBLE);
        for r in exit::REFUSALS {
            let _ = writeln!(
                text,
                "  {:<18} ({}) stdout {}, next {}",
                r.class,
                r.tag,
                r.stdout.name(),
                r.action.name()
            );
        }
    }
    Ok(text)
}

/// What `explain 7` says exit 7 covers, before its reason classes.
const REFUSAL_PREAMBLE: &str = "A contract the caller asked for, or the check the command exists to make, did not hold; stderr names the reason class in a line `refused: <text> (<tag>)`:\n";

/// The `tool` member every document of this binary carries.
fn tool() -> Value {
    json!({"name": "deep-diff-forge", "version": env!("CARGO_PKG_VERSION")})
}

/// One signal's facts, as the vocabulary document and an explanation both state them.
fn signal_json(facts: &deep_diff_forge_graph::SignalFacts) -> Value {
    json!({
        "label": facts.label(),
        "class": facts.class.label(),
        "kind": facts.kind.label(),
        "weight": facts.weight(),
        "basis": facts.basis(),
        "sides": facts.sides().iter().map(|s| s.label()).collect::<Vec<_>>(),
        "since": facts.since,
        "summary": facts.summary,
    })
}

/// `explain signal <label> [--audience agent|person] [--json]`: why a signal matters, what
/// answers it, what does not, and the compiler or linter rules that prevent it. The people's
/// form (the default, `"audience": "person"`) lists every substitute; `--audience agent` prints
/// what an agent is shown (F7.c1, contract design 3.4): the same repairs, and of the substitutes
/// only those with an `agent` sentence, in that sentence. The JSON names each entry's id beside it
/// (`accept_ids` in the order of `accept`; `id` in each `not_accepted` entry; F1.a2).
fn explain_signal(args: &[String]) -> Result<String, String> {
    use std::fmt::Write as _;
    let (mut json, mut agent, mut label) = (false, false, None);
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--json" => json = true,
            "--audience" => {
                agent = match rest.next().map(String::as_str) {
                    Some("agent") => true,
                    Some("person") => false,
                    _ => return Err("--audience takes agent or person".into()),
                };
            }
            value if label.is_none() => label = Some(value),
            value => return Err(format!("unexpected argument: {value}")),
        }
    }
    let label = label.ok_or_else(|| format!("usage: {EXPLAIN_USAGE}"))?;
    let signal = deep_diff_forge_graph::signal_named(label).ok_or_else(|| {
        format!(
            "unknown signal: {} (deep-diff-forge schema signals lists them)",
            deep_diff_forge_core::display_safe(label)
        )
    })?;
    let facts = deep_diff_forge_graph::facts(signal);
    let remedy = deep_diff_forge_graph::remedy(signal);
    let shown = substitutes(&remedy, agent);
    if json {
        let mut value = signal_json(&facts);
        value["schema"] = json!(SIGNAL_EXPLAIN_SCHEMA);
        value["tool"] = tool();
        value["why"] = json!(remedy.why);
        value["accept"] = json!(remedy.accept);
        value["accept_ids"] = json!(remedy.accept_ids);
        value["not_accepted"] = shown.into_iter().map(|(entry, _)| entry).collect();
        value["native_lints"] = remedy
            .native_lints
            .iter()
            .map(|l| json!({"language": l.language, "tool": l.tool, "rule": l.rule}))
            .collect();
        value["audience"] = json!(if agent { "agent" } else { "person" });
        return Ok(format!("{value}\n"));
    }
    let mut text = String::new();
    let _ = writeln!(
        text,
        "{} ({}, {}, weight {}): {}",
        facts.label(),
        facts.class.label(),
        facts.kind.label(),
        facts.weight(),
        facts.summary
    );
    let _ = writeln!(text, "why: {}", remedy.why);
    let _ = writeln!(text, "accept:");
    for line in remedy.accept {
        let _ = writeln!(text, "  - {line}");
    }
    if !shown.is_empty() {
        let _ = writeln!(text, "not accepted:");
        for (_, line) in shown {
            let _ = writeln!(text, "  - {line}");
        }
    }
    if !remedy.native_lints.is_empty() {
        let _ = writeln!(text, "prevent:");
        for lint in remedy.native_lints {
            let _ = writeln!(text, "  - {}, {}: {}", lint.language, lint.tool, lint.rule);
        }
    }
    if agent {
        let _ = writeln!(text, "audience: agent");
    } else {
        let _ = writeln!(
            text,
            "audience: person (deep-diff-forge explain signal {} --audience agent prints what an agent is shown)",
            facts.label()
        );
    }
    Ok(text)
}

/// The substitutes of `remedy` as its audience reads them, each as its JSON entry and its text
/// line: for people every one, with the label it raises or that none sees it; for an agent only
/// those with an `agent` sentence, in that sentence and nothing else (F7.c1).
fn substitutes(remedy: &deep_diff_forge_graph::Remedy, agent: bool) -> Vec<(Value, String)> {
    let shown = remedy.not_accepted.iter();
    if agent {
        return shown
            .filter_map(|n| {
                n.agent.map(|a| {
                    (
                        json!({"id": n.id, "agent": a, "raises": n.raises}),
                        a.into(),
                    )
                })
            })
            .collect();
    }
    shown
        .map(|n| {
            let line = match n.raises {
                Some(raised) => format!("{} (raises {raised})", n.substitute),
                None => format!(
                    "{} (no signal sees this yet; it is still not a repair)",
                    n.substitute
                ),
            };
            (
                json!({"id": n.id, "substitute": n.substitute, "raises": n.raises}),
                line,
            )
        })
        .collect()
}

/// `schema [result|exit-codes|signals|diagnostics]`: the JSON Schema of the envelope, the exit-code
/// registry, the signal vocabulary, or the JSON Schema of `deep-diff-forge.diagnostics.v0`.
pub fn schema(args: &[String]) -> Result<String, String> {
    let value = match args.first().map(String::as_str) {
        None | Some("result") => result_schema(),
        Some("exit-codes") => json!({
            "schema": EXIT_CODES_SCHEMA,
            "codes": EXIT_CODES.iter().map(entry_json).collect::<Vec<_>>(),
            "refusals": crate::mcp::refusals_json(),
        }),
        Some("signals") => json!({
            "schema": SIGNALS_SCHEMA,
            "tool": tool(),
            "signals": deep_diff_forge_graph::vocabulary().iter().map(signal_json).collect::<Vec<_>>(),
        }),
        Some("diagnostics") => diagnostics_schema(),
        Some(other) => {
            return Err(format!(
                "unknown schema: {other} (result|exit-codes|signals|diagnostics)"
            ));
        }
    };
    Ok(format!("{value:#}\n"))
}

/// Every outcome name the envelope can report.
fn outcomes() -> Vec<&'static str> {
    let mut names: Vec<&str> = EXIT_CODES.iter().map(|e| e.name).collect();
    names.extend(["undocumented", "signal"]);
    names.sort_unstable();
    names.dedup();
    names
}

fn result_schema() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "https://github.com/Louranicas/deep-diff-forge/schemas/ddf.result.v1.json",
        "title": RESULT_SCHEMA,
        "description": "What one deep-diff-forge CLI process did, as reported by `deep-diff-forge envelope -- <command…>`. A parsed stdout is the command's JSON value re-serialised (key order and number text may differ); byte-exact consumers read the command directly. The envelope sets no deadline of its own.",
        "type": "object",
        "required": ["schema", "version", "command", "family", "exit_code", "outcome", "ok", "stdout", "stdout_format", "stdout_utf8", "stderr", "truncated"],
        "additionalProperties": false,
        "properties": {
            "schema": {"const": RESULT_SCHEMA},
            "version": {"type": "string", "description": "The deep-diff-forge version that ran the command."},
            "command": {"type": "array", "items": {"type": "string"}},
            "family": {"enum": ["engine", "council"]},
            "exit_code": {"type": ["integer", "null"], "description": "null when the command ended on a signal."},
            "signal": {"type": "integer"},
            "outcome": {"enum": outcomes(), "description": "The registry name of exit_code in this family (`deep-diff-forge schema exit-codes`), `undocumented` or `signal`."},
            "ok": {"type": "boolean", "description": "exit_code is 0 and nothing was truncated."},
            "stdout": {"description": "The parsed value when stdout is exactly one JSON value; otherwise its text (lossy UTF-8)."},
            "stdout_format": {"enum": ["json", "text", "empty"]},
            "stdout_utf8": {"type": "boolean"},
            "stderr": {"type": "string"},
            "truncated": {"type": "array", "items": {"enum": ["stdout", "stderr"]}},
        },
    })
}

/// The JSON Schema of `deep-diff-forge.diagnostics.v0`: members only added, so objects stay open;
/// the codes and labels are the vocabulary's (`code_of`), the pinned table of the reference page.
fn diagnostics_schema() -> Value {
    let vocabulary = deep_diff_forge_graph::vocabulary();
    let codes: Vec<String> = vocabulary
        .iter()
        .flat_map(|f| [true, false].map(|line| deep_diff_forge_graph::code_of(f.signal, line)))
        .flatten()
        .collect();
    let labels: Vec<&str> = vocabulary
        .iter()
        .map(deep_diff_forge_graph::SignalFacts::label)
        .collect();
    let (count, hex) = (
        json!({"type": "integer", "minimum": 0}),
        json!({"type": "string", "pattern": "^[0-9a-f]{64}$"}),
    );
    let ids = json!({"type": "array", "items": {"type": "string"}});
    let object = |required: &[&str], properties: Value| json!({"type": "object", "required": required, "properties": properties});
    let diagnostic = object(
        &[
            "key",
            "code",
            "label",
            "class",
            "path",
            "anchor",
            "side",
            "line",
            "occurrence",
            "reading",
            "audience",
            "repairs",
            "not_accepted",
        ],
        json!({
            "key": hex, "code": {"enum": codes}, "label": {"enum": labels},
            "class": {"enum": ["security", "integrity", "shape"]}, "path": {"type": "string"},
            "anchor": {"enum": ["line", "file"]}, "side": {"enum": ["new", "old", null]},
            "line": {"type": ["integer", "null"]}, "occurrence": count,
            "reading": {"enum": ["current", "floor_only"]}, "audience": {"enum": ["agent", "person"]},
            "repairs": ids, "not_accepted": ids,
            "test_likely": {"type": "boolean"}, "judge_sees": {"type": "boolean"},
        }),
    );
    let mut schema = object(
        &[
            "schema",
            "input_sha256",
            "tool",
            "judge",
            "counts",
            "cut",
            "diagnostics",
            "texts",
        ],
        json!({
            "schema": {"const": DIAGNOSTICS_SCHEMA}, "input_sha256": hex,
            "tool": object(&["name", "version", "scorer_sha256"], json!({"scorer_sha256": hex})),
            "judge": object(&["state", "ranker_sha256"], json!({
                "state": {"enum": ["unknown", "same", "different", "none_admitted"]},
                "ranker_sha256": {"type": ["string", "null"]},
            })),
            "counts": object(&["files", "diagnostics", "agent", "person", "cut_files"], json!({
                "files": count, "diagnostics": count, "agent": count, "person": count, "cut_files": count,
            })),
            "cut": {"type": "array", "items": object(&["path", "rows_kept"], json!({"path": {"type": "string"}, "rows_kept": count}))},
            "diagnostics": {"type": "array", "items": diagnostic},
            "texts": {"type": "object", "additionalProperties": {"type": "string"}},
        }),
    );
    schema["$schema"] = json!("https://json-schema.org/draft/2020-12/schema");
    schema["$id"] = json!(format!(
        "https://github.com/Louranicas/deep-diff-forge/schemas/{DIAGNOSTICS_SCHEMA}.json"
    ));
    schema["title"] = json!(DIAGNOSTICS_SCHEMA);
    schema["description"] = json!(
        "What `--diagnostics --json` prints: every label of the ranking of one input, at the evidence row that locates it or at its file (docs/reference/diagnostics.md). Members are only added, so readers ignore members they do not know."
    );
    schema
}

/// Read at most `cap` bytes and report whether more followed.
pub fn capture(reader: impl Read, cap: usize) -> std::io::Result<(Vec<u8>, bool)> {
    let mut buf = Vec::new();
    let mut reader = reader;
    reader.by_ref().take(cap as u64 + 1).read_to_end(&mut buf)?;
    let cut = buf.len() > cap;
    buf.truncate(cap);
    if cut {
        // Drain the rest so the child is never blocked on a full pipe.
        std::io::copy(&mut reader, &mut std::io::sink())?;
    }
    Ok((buf, cut))
}

/// The envelope for a finished command. `status` is `Ok(code)` or `Err(signal)`.
pub fn wrap(
    command: &[String],
    status: Result<i32, i32>,
    stdout: (&[u8], bool),
    stderr: (&[u8], bool),
) -> Value {
    let family = family_of(command);
    let (exit_code, outcome) = match status {
        Ok(code) => (
            json!(code),
            lookup(family, code).map_or("undocumented", |e| e.name),
        ),
        Err(_) => (Value::Null, "signal"),
    };
    let text = String::from_utf8_lossy(stdout.0);
    // Empty means no bytes at all; whitespace is text. Only JSON's own whitespace may surround a
    // parsed value (serde_json's rule), a cut value is never parsed, nor is invalid UTF-8.
    let (body, format) = if stdout.0.is_empty() {
        (json!(""), "empty")
    } else if stdout.1 || std::str::from_utf8(stdout.0).is_err() {
        (json!(text), "text")
    } else {
        serde_json::from_str::<Value>(&text)
            .map_or_else(|_| (json!(text), "text"), |value| (value, "json"))
    };
    let truncated: Vec<&str> = [("stdout", stdout.1), ("stderr", stderr.1)]
        .into_iter()
        .filter(|t| t.1)
        .map(|t| t.0)
        .collect();
    let mut value = json!({
        "schema": RESULT_SCHEMA,
        "version": env!("CARGO_PKG_VERSION"),
        "command": command,
        "family": family,
        "exit_code": exit_code,
        "outcome": outcome,
        "ok": status == Ok(0) && truncated.is_empty(),
        "stdout": body,
        "stdout_format": format,
        "stdout_utf8": std::str::from_utf8(stdout.0).is_ok(),
        "stderr": String::from_utf8_lossy(stderr.0),
        "truncated": truncated,
    });
    if let Err(signal) = status {
        value["signal"] = json!(signal);
    }
    value
}

/// `envelope -- <command…>`: run the command as a child of this binary and wrap the result. Returns
/// the envelope text and the exit code to leave with: the child's, or 128 + N if signal N ended it
/// (the shell's convention, [`exit::SIGNAL_BASE`]). Before, a signal gave exit 1, a code the
/// registry does not list (bug hunt 2026-11-01).
#[expect(
    clippy::disallowed_methods,
    reason = "the envelope door: runs the wrapped deep-diff-forge command as a child"
)]
pub fn envelope(args: &[String]) -> Result<(String, i32), String> {
    let command = match args.split_first() {
        Some((dash, rest)) if dash == "--" && !rest.is_empty() => rest,
        _ => return Err("usage: envelope -- <command> [args…]".into()),
    };
    if command[0] == "envelope" {
        return Err("envelope does not wrap itself".into());
    }
    let exe = std::env::current_exe().map_err(|e| format!("cannot locate this executable: {e}"))?;
    let mut child = Command::new(exe)
        .args(command)
        .stdin(Stdio::inherit())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot run the command: {e}"))?;
    let stderr = child.stderr.take().ok_or("no stderr pipe")?;
    // Both pipes are read at once, so neither can fill and stall the child.
    let errors = std::thread::spawn(move || capture(stderr, STDERR_CAP));
    let out = capture(child.stdout.take().ok_or("no stdout pipe")?, STDOUT_CAP)
        .map_err(|e| format!("reading stdout: {e}"))?;
    let err = errors
        .join()
        .map_err(|_| "stderr reader panicked")?
        .map_err(|e| format!("reading stderr: {e}"))?;
    let status = child
        .wait()
        .map_err(|e| format!("waiting for the command: {e}"))?;
    let status: Result<i32, i32> = status.code().ok_or_else(|| signal_of(status));
    let value = wrap(command, status, (&out.0, out.1), (&err.0, err.1));
    Ok((
        format!("{value}\n"),
        status.unwrap_or_else(|signal| exit::SIGNAL_BASE + signal),
    ))
}

/// The signal that ended `status`, or 0 where there is none to read.
fn signal_of(status: std::process::ExitStatus) -> i32 {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt as _;
        status.signal().unwrap_or(0)
    }
    #[cfg(not(unix))]
    {
        let _ = status;
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    /// Every envelope `wrap` can produce satisfies the published schema's keys and enums.
    #[test]
    fn wrap_conforms_to_the_published_schema() {
        let schema = result_schema();
        let properties = schema["properties"].as_object().unwrap();
        let required: Vec<&str> = schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        let cases = [
            wrap(&args(&["x"]), Ok(0), (b"{}", false), (b"", false)),
            wrap(
                &args(&["council", "status"]),
                Ok(7),
                (b"x", true),
                (b"e", true),
            ),
            wrap(&args(&["x"]), Err(9), (b"", false), (b"", false)),
            wrap(&args(&["x"]), Ok(5), (b"\xff", false), (b"", false)),
        ];
        for value in cases {
            let object = value.as_object().unwrap();
            assert!(object.keys().all(|k| properties.contains_key(k)), "{value}");
            assert!(required.iter().all(|k| object.contains_key(*k)), "{value}");
            for field in ["outcome", "stdout_format", "family"] {
                assert!(
                    properties[field]["enum"]
                        .as_array()
                        .unwrap()
                        .contains(&value[field]),
                    "{field}: {value}"
                );
            }
        }
    }

    struct Counting<'a>(&'a [u8], usize);

    impl Read for Counting<'_> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = buf.len().min(self.0.len() - self.1).min(3);
            buf[..n].copy_from_slice(&self.0[self.1..self.1 + n]);
            self.1 += n;
            Ok(n)
        }
    }

    #[test]
    fn capture_drains_everything_past_the_cap() {
        let data = [7u8; 64];
        let mut reader = Counting(&data, 0);
        let (kept, cut) = capture(&mut reader, 10).unwrap();
        assert_eq!((kept.len(), cut, reader.1), (10, true, 64));
    }

    #[test]
    fn whitespace_is_text_and_only_json_whitespace_surrounds_a_value() {
        let v = wrap(&args(&["x"]), Ok(0), (b" \n", false), (b"", false));
        assert_eq!(
            (v["stdout"].as_str(), v["stdout_format"].as_str()),
            (Some(" \n"), Some("text"))
        );
        let v = wrap(
            &args(&["x"]),
            Ok(0),
            ("\u{a0}{}".as_bytes(), false),
            (b"", false),
        );
        assert_eq!(v["stdout_format"], "text");
        let v = wrap(
            &args(&["x"]),
            Ok(0),
            (b"\n {\"a\":1} \r\n", false),
            (b"", false),
        );
        assert_eq!(v["stdout"]["a"], 1);
    }

    #[test]
    fn capture_reports_a_cut_and_drains_the_rest() {
        let (kept, cut) = capture(&b"abcdef"[..], 4).unwrap();
        assert_eq!((kept.as_slice(), cut), (&b"abcd"[..], true));
        let (kept, cut) = capture(&b"abcd"[..], 4).unwrap();
        assert_eq!((kept.as_slice(), cut), (&b"abcd"[..], false));
    }

    #[test]
    fn wrap_parses_one_json_value_and_names_the_outcome() {
        let v = wrap(
            &args(&["deploy", "status", "--json"]),
            Ok(0),
            (b"{\"a\":1}\n", false),
            (b"", false),
        );
        assert_eq!(v["stdout"]["a"], 1);
        assert_eq!(v["stdout_format"], "json");
        assert_eq!(
            (v["outcome"].as_str(), v["ok"].as_bool()),
            (Some("success"), Some(true))
        );
        let v = wrap(
            &args(&["council", "status"]),
            Ok(7),
            (b"x\n", false),
            (b"why", false),
        );
        assert_eq!(
            (v["family"].as_str(), v["outcome"].as_str()),
            (Some("council"), Some("hold"))
        );
        assert_eq!(
            (v["stdout_format"].as_str(), v["ok"].as_bool()),
            (Some("text"), Some(false))
        );
        assert_eq!(v["stderr"], "why");
    }

    #[test]
    fn jsonl_is_text_and_a_cut_value_is_never_parsed() {
        let v = wrap(
            &args(&["--stdin-patch", "--jsonl"]),
            Ok(0),
            (b"{}\n{}\n", false),
            (b"", false),
        );
        assert_eq!(v["stdout_format"], "text");
        let v = wrap(&args(&["x"]), Ok(0), (b"{\"a\":1}", true), (b"", false));
        assert_eq!(
            (v["stdout_format"].as_str(), v["ok"].as_bool()),
            (Some("text"), Some(false))
        );
        assert_eq!(v["truncated"], json!(["stdout"]));
    }

    #[test]
    fn undocumented_codes_and_signals_are_named_as_such() {
        let v = wrap(&args(&["x"]), Ok(5), (b"", false), (b"", false));
        assert_eq!(
            (v["outcome"].as_str(), v["stdout_format"].as_str()),
            (Some("undocumented"), Some("empty"))
        );
        let v = wrap(&args(&["x"]), Err(9), (b"", false), (b"", false));
        assert_eq!(
            (v["outcome"].as_str(), v["signal"].as_i64()),
            (Some("signal"), Some(9))
        );
        assert!(v["exit_code"].is_null());
    }

    #[cfg(unix)]
    #[test]
    fn signal_of_reads_the_terminating_signal() {
        use std::os::unix::process::ExitStatusExt as _;
        assert_eq!(signal_of(std::process::ExitStatus::from_raw(9)), 9);
        assert_eq!(signal_of(std::process::ExitStatus::from_raw(15)), 15);
        assert_eq!(signal_of(std::process::ExitStatus::from_raw(0)), 0);
    }

    #[test]
    fn the_machine_contract_reference_states_the_capture_caps() {
        let readme = include_str!("../../../docs/reference/output-and-exit-codes.md");
        let stated = format!(
            "stdout is capped at {} MiB and stderr at {} MiB",
            STDOUT_CAP / (1024 * 1024),
            STDERR_CAP / (1024 * 1024)
        );
        assert!(readme.contains(&stated), "{stated}");
        assert_eq!(
            (STDOUT_CAP % (1024 * 1024), STDERR_CAP % (1024 * 1024)),
            (0, 0)
        );
    }

    #[test]
    fn explain_reads_the_registry() {
        assert!(
            explain(&args(&["0", "--family", "engine"]))
                .unwrap()
                .starts_with("engine 0 (success)")
        );
        assert!(explain(&args(&["7", "4"])).is_err());
        // F7.h1: the first line as before, then exit 7's reason classes, one line each.
        let seven = explain(&args(&["7"])).unwrap();
        assert!(
            seven.starts_with(
                "engine 7 (input_contract_refused): Input contract refused.\nA contract the caller asked for"
            ),
            "{seven}"
        );
        assert_eq!(
            seven.lines().count(),
            2 + deep_diff_forge_core::exit::REFUSALS.len(),
            "{seven}"
        );
        assert!(
            seven.contains(
                "  require_all        (--require-all) stdout document, next review_evidence\n"
            ),
            "{seven}"
        );
        assert!(
            explain(&args(&["7", "--family", "council"]))
                .unwrap()
                .starts_with("council 7 (hold)")
        );
        assert!(explain(&args(&["5"])).is_err());
        assert!(explain(&args(&["x"])).is_err());
        assert!(explain(&args(&["--family", "other", "0"])).is_err());
        assert!(explain(&args(&[])).is_err());
        let v: Value =
            serde_json::from_str(&explain(&args(&["--json", "4", "--family", "council"])).unwrap())
                .unwrap();
        assert_eq!(v["name"], "refused");
    }

    #[test]
    fn schema_names_its_documents() {
        let v: Value = serde_json::from_str(&schema(&[]).unwrap()).unwrap();
        assert_eq!(v["title"], RESULT_SCHEMA);
        let v: Value = serde_json::from_str(&schema(&args(&["exit-codes"])).unwrap()).unwrap();
        assert_eq!(v["codes"].as_array().unwrap().len(), EXIT_CODES.len());
        assert_eq!(
            schema(&args(&["other"])).unwrap_err(),
            "unknown schema: other (result|exit-codes|signals|diagnostics)"
        );
    }

    /// The vocabulary document lists every signal a ranking can carry, with the facts a reader
    /// needs to act on one: what it is about, how sure it is, what it weighs and how it was read.
    #[test]
    fn schema_signals_is_the_whole_vocabulary() {
        let v: Value = serde_json::from_str(&schema(&args(&["signals"])).unwrap()).unwrap();
        assert_eq!(v["schema"], SIGNALS_SCHEMA);
        assert_eq!(v["tool"]["name"], "deep-diff-forge");
        assert_eq!(v["tool"]["version"], env!("CARGO_PKG_VERSION"));
        let signals = v["signals"].as_array().unwrap();
        let vocabulary = deep_diff_forge_graph::vocabulary();
        assert_eq!(signals.len(), vocabulary.len());
        for (row, facts) in signals.iter().zip(&vocabulary) {
            assert_eq!(row["label"], facts.label());
            assert_eq!(row["weight"], facts.weight());
            for member in ["class", "kind", "basis", "sides", "since", "summary"] {
                assert!(!row[member].is_null(), "{}: {member}", facts.label());
            }
        }
        let by = |label: &str| signals.iter().find(|s| s["label"] == label).unwrap();
        assert_eq!(
            *by("test_removed"),
            json!({"label": "test_removed", "class": "integrity", "kind": "direct", "weight": 3,
                   "basis": ["lexical", "hunk"], "sides": ["old"], "since": "0.5.0",
                   "summary": "A removed line was a test."})
        );
        assert_eq!(by("public_api_surface")["basis"], json!(["path"]));
        assert_eq!(by("public_api_surface")["sides"], json!([]));
        // HR-L-6: read from the path and from every changed line, as the scorer reads it
        assert_eq!(by("hazardous_text")["basis"], json!(["path", "lexical"]));
        assert_eq!(by("hazardous_text")["sides"], json!(["new", "old"]));
        assert_eq!(by("secret_like")["class"], "security");
    }

    #[test]
    fn explain_signal_says_why_what_answers_it_and_what_does_not() {
        let text = explain(&args(&["signal", "check_suppressed"])).unwrap();
        assert!(
            text.starts_with("check_suppressed (integrity, direct, weight 2): "),
            "{text}"
        );
        for part in [
            "\nwhy: ",
            "\naccept:\n  - Fix what the check reports.\n",
            "\nnot accepted:\n  - ",
            "(no signal sees this yet; it is still not a repair)",
            "\nprevent:\n  - Rust, clippy: allow_attributes_without_reason\n",
        ] {
            assert!(text.contains(part), "{part:?} in {text}");
        }
        // a substitute another signal sees is named
        let spawn = explain(&args(&["signal", "process_spawn"])).unwrap();
        assert!(spawn.contains("(raises dynamic_eval)"), "{spawn}");
        // a signal that only orders the review has no "not accepted" and no "prevent" part
        let new_file = explain(&args(&["signal", "new_file"])).unwrap();
        assert!(!new_file.contains("not accepted:") && !new_file.contains("prevent:"));

        let v: Value =
            serde_json::from_str(&explain(&args(&["signal", "--json", "test_removed"])).unwrap())
                .unwrap();
        assert_eq!(v["schema"], SIGNAL_EXPLAIN_SCHEMA);
        assert_eq!(v["label"], "test_removed");
        assert_eq!(v["weight"], 3);
        assert_eq!(v["sides"], json!(["old"]));
        assert!(v["why"].as_str().is_some_and(|w| !w.is_empty()));
        assert_eq!(v["accept"].as_array().unwrap().len(), 2);
        // skipping the test raises test_skipped; emptying its body raises nothing (CI4)
        assert_eq!(v["not_accepted"][0]["raises"], "test_skipped");
        assert_eq!(v["not_accepted"][1]["raises"], Value::Null);
        assert_eq!(v["native_lints"], json!([]));
        let unsafe_added: Value =
            serde_json::from_str(&explain(&args(&["signal", "unsafe_added", "--json"])).unwrap())
                .unwrap();
        assert_eq!(
            unsafe_added["native_lints"],
            json!([{"language": "Rust", "tool": "rustc", "rule": "unsafe_code"}])
        );
    }

    /// F7.c1 (contract design 3.4): `explain signal` is the people's channel and says so; its
    /// agent form prints what an agent is shown: the same repairs, and of the substitutes only
    /// the witnessed `agent` sentences.
    #[test]
    fn explain_signal_names_its_audience() {
        let person = explain(&args(&["signal", "test_skipped"])).unwrap();
        assert!(
            person.ends_with("\naudience: person (deep-diff-forge explain signal test_skipped --audience agent prints what an agent is shown)\n"),
            "{person}"
        );
        assert_eq!(
            explain(&args(&["signal", "--audience", "person", "test_skipped"])).unwrap(),
            person
        );
        let agent = explain(&args(&["signal", "test_skipped", "--audience", "agent"])).unwrap();
        assert!(agent.ends_with("\naudience: agent\n"), "{agent}");
        assert!(agent.contains("\naccept:\n  - Make the test pass and remove the skip.\n"));
        assert!(
            agent.contains("`#[test]`") && agent.contains("`assert!(true)`"),
            "{agent}"
        );
        let json = |list: &[&str]| -> Value {
            serde_json::from_str(&explain(&args(list)).unwrap()).unwrap()
        };
        let p = json(&["signal", "test_skipped", "--json"]);
        let a = json(&["signal", "test_skipped", "--json", "--audience", "agent"]);
        assert_eq!(
            (&p["audience"], &a["audience"]),
            (&json!("person"), &json!("agent"))
        );
        assert_eq!((&p["accept"], &p["why"]), (&a["accept"], &a["why"]));
        let raised: Vec<&Value> = a["not_accepted"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| &n["raises"])
            .collect();
        assert_eq!(raised, [&json!("test_removed"), &json!("trivial_assert")]);
        for label in deep_diff_forge_graph::vocabulary()
            .iter()
            .map(deep_diff_forge_graph::SignalFacts::label)
        {
            let agent = explain(&args(&["signal", label, "--audience", "agent"])).unwrap();
            let a = json(&["signal", label, "--json", "--audience", "agent"]);
            let remedy =
                deep_diff_forge_graph::remedy(deep_diff_forge_graph::signal_named(label).unwrap());
            // no people's sentence of a substitute, and no pointer back to the people's channel
            for not in remedy.not_accepted {
                assert!(!agent.contains(not.substitute), "{label}: {agent}");
                assert!(!a.to_string().contains(not.substitute), "{label}: {a}");
            }
            assert!(!agent.contains("explain signal"), "{label}: {agent}");
            let sentences = remedy
                .not_accepted
                .iter()
                .filter(|n| n.agent.is_some())
                .count();
            assert_eq!(
                a["not_accepted"].as_array().unwrap().len(),
                sentences,
                "{label}"
            );
            assert_eq!(
                agent.contains("\nnot accepted:\n"),
                sentences > 0,
                "{label}"
            );
        }
        assert_eq!(
            explain(&args(&["signal", "new_file", "--audience", "model"])).unwrap_err(),
            "--audience takes agent or person"
        );
        assert_eq!(
            explain(&args(&["signal", "new_file", "--audience"])).unwrap_err(),
            "--audience takes agent or person"
        );
    }

    /// F1.a2: `signal-explain.v0` names the vocabulary id of every repair (`accept_ids`, in the
    /// order of `accept`) and of every substitute it shows (`id`), for both audiences, and each
    /// id is one of the pinned `fixtures/diagnostics/ids.v0.txt`.
    #[test]
    fn explain_signal_names_the_id_of_every_entry() {
        let pinned = include_str!("../../../fixtures/diagnostics/ids.v0.txt");
        let pinned: Vec<&str> = pinned.lines().collect();
        let doc = |list: &[&str]| -> Value {
            serde_json::from_str(&explain(&args(list)).unwrap()).unwrap()
        };
        for facts in deep_diff_forge_graph::vocabulary() {
            let label = facts.label();
            let remedy = deep_diff_forge_graph::remedy(facts.signal);
            for audience in ["person", "agent"] {
                let v = doc(&["signal", label, "--json", "--audience", audience]);
                let ids = v["accept_ids"].as_array().expect("accept_ids");
                assert_eq!(
                    *ids,
                    json!(remedy.accept_ids).as_array().unwrap().clone(),
                    "{label}"
                );
                assert_eq!(ids.len(), v["accept"].as_array().unwrap().len(), "{label}");
                let shown = v["not_accepted"].as_array().unwrap();
                for entry in shown {
                    let id = entry["id"]
                        .as_str()
                        .unwrap_or_else(|| panic!("{label}: {entry}"));
                    let not = remedy.not_accepted.iter().find(|n| n.id == id).unwrap();
                    let text = if audience == "agent" {
                        not.agent
                    } else {
                        Some(not.substitute)
                    };
                    let member = if audience == "agent" {
                        "agent"
                    } else {
                        "substitute"
                    };
                    assert_eq!(entry[member].as_str(), text, "{id}");
                }
                for id in ids.iter().chain(shown.iter().map(|e| &e["id"])) {
                    let id = id.as_str().unwrap();
                    assert!(pinned.contains(&id), "{id} is not pinned");
                    assert!(id.starts_with(&format!("{label}:")), "{id}");
                }
            }
        }
    }

    #[test]
    fn explain_signal_refuses_what_it_cannot_explain() {
        assert_eq!(
            explain(&args(&["signal"])).unwrap_err(),
            format!("usage: {EXPLAIN_USAGE}")
        );
        assert_eq!(
            explain(&args(&["signal", "no_such_signal"])).unwrap_err(),
            "unknown signal: no_such_signal (deep-diff-forge schema signals lists them)"
        );
        assert_eq!(
            explain(&args(&["signal", "new_file", "extra"])).unwrap_err(),
            "unexpected argument: extra"
        );
        // a label is matched exactly, and a control character in it is not echoed raw
        assert!(explain(&args(&["signal", "New_File"])).is_err());
        let hostile = explain(&args(&["signal", "x\u{1b}[31m"])).unwrap_err();
        assert!(!hostile.contains('\u{1b}'), "{hostile:?}");
        // the exit-code form is unchanged
        assert_eq!(
            explain(&args(&[])).unwrap_err(),
            format!("usage: {EXPLAIN_USAGE}")
        );
    }

    #[test]
    fn envelope_refuses_a_missing_separator_and_itself() {
        assert!(envelope(&args(&["deploy"])).is_err());
        assert!(envelope(&args(&["--"])).is_err());
        assert!(envelope(&args(&["--", "envelope", "--", "doctor"])).is_err());
    }
}
