// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fdeep-diff-forge-cli--contract
// DDF slice A: the machine contract (`ddf.result.v1`, the exit-code registry, `schema`, `explain`,
// `envelope`). Design/status: docs/ROADMAP.md#status-2026-10-26 and README "Machine contract".
//! The `ddf.result.v1` envelope over the exit-code registry (`deep_diff_forge_core::exit`).
//!
//! `envelope -- <command…>` runs this binary as a child with the given arguments and wraps what it
//! did, whatever the command, in one JSON object: its exit status and the registry's name for it,
//! its stdout (parsed when it is one JSON value), and its stderr. The child's own output is never
//! changed, so every existing consumer keeps working, and the envelope exits with the child's code.

use deep_diff_forge_core::exit::{EXIT_CODES, ExitCode, lookup};
use serde_json::{Value, json};
use std::io::Read;
use std::process::{Command, Stdio};

/// The envelope's schema identifier.
pub const RESULT_SCHEMA: &str = "ddf.result.v1";
/// The exit-code registry's schema identifier.
pub const EXIT_CODES_SCHEMA: &str = "ddf.exit-codes.v1";
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

/// `explain <code> [--family engine|council] [--json]`: what an exit code means.
pub fn explain(args: &[String]) -> Result<String, String> {
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
    let code = code.ok_or("usage: explain <code> [--family engine|council] [--json]")?;
    let entry = lookup(family, code)
        .ok_or_else(|| format!("exit code {code} is not documented for the {family} family"))?;
    Ok(if json {
        format!("{}\n", entry_json(entry))
    } else {
        format!(
            "{} {} ({}): {}\n",
            entry.family, entry.code, entry.name, entry.meaning
        )
    })
}

fn entry_json(entry: &ExitCode) -> Value {
    json!({"family": entry.family, "code": entry.code, "name": entry.name, "meaning": entry.meaning})
}

/// `schema [result|exit-codes]`: the JSON Schema of the envelope, or the exit-code registry.
pub fn schema(args: &[String]) -> Result<String, String> {
    let value = match args.first().map(String::as_str) {
        None | Some("result") => result_schema(),
        Some("exit-codes") => json!({
            "schema": EXIT_CODES_SCHEMA,
            "codes": EXIT_CODES.iter().map(entry_json).collect::<Vec<_>>(),
        }),
        Some(other) => return Err(format!("unknown schema: {other} (result|exit-codes)")),
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
/// the envelope text and the exit code to leave with (the child's; 1 if it ended on a signal).
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
    Ok((format!("{value}\n"), status.unwrap_or(1)))
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
    fn the_readme_states_the_capture_caps() {
        let readme = include_str!("../../../README.md");
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
        assert_eq!(
            explain(&args(&["7"])).unwrap(),
            "engine 7 (input_contract_refused): Input contract refused.\n"
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
        assert!(schema(&args(&["other"])).is_err());
    }

    #[test]
    fn envelope_refuses_a_missing_separator_and_itself() {
        assert!(envelope(&args(&["deploy"])).is_err());
        assert!(envelope(&args(&["--"])).is_err());
        assert!(envelope(&args(&["--", "envelope", "--", "doctor"])).is_err());
    }
}
