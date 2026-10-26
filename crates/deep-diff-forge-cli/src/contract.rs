// DDF slice A: the machine contract (`ddf.result.v1`, the exit-code registry, `schema`, `explain`,
// `envelope`). Design/status: docs/ROADMAP.md#status-2026-10-26 and README "Machine contract".
//! One registry of exit codes, by command family, and the `ddf.result.v1` envelope.
//!
//! `envelope -- <command…>` runs this binary as a child with the given arguments and wraps what it
//! did, whatever the command, in one JSON object: its exit status and the registry's name for it,
//! its stdout (parsed when it is one JSON value), and its stderr. The child's own output is never
//! changed, so every existing consumer keeps working, and the envelope exits with the child's code.

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

/// One exit code of one command family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExitCode {
    pub family: &'static str,
    pub code: i32,
    pub name: &'static str,
    pub meaning: &'static str,
}

/// Every exit code the CLI documents. `council` is the review-council subcommand family; `engine`
/// is every other command. Each README "Exit codes" row must begin with its meaning here, and the
/// two must name the same codes (`tests/contract.rs`).
pub const EXIT_CODES: &[ExitCode] = &[
    ExitCode {
        family: "engine",
        code: 0,
        name: "success",
        meaning: "Success.",
    },
    ExitCode {
        family: "engine",
        code: 2,
        name: "usage",
        meaning: "CLI usage / argument error.",
    },
    ExitCode {
        family: "engine",
        code: 3,
        name: "input_read",
        meaning: "Input (stdin or file) read failure.",
    },
    ExitCode {
        family: "engine",
        code: 4,
        name: "parse",
        meaning: "Patch parse failure.",
    },
    ExitCode {
        family: "engine",
        code: 6,
        name: "daemon_or_terminal",
        meaning: "Daemon / interactive-terminal failure.",
    },
    ExitCode {
        family: "engine",
        code: 7,
        name: "input_contract_refused",
        meaning: "Input contract refused.",
    },
    ExitCode {
        family: "council",
        code: 0,
        name: "completed",
        meaning: "The requested council operation completed under its contract; `status` exit 0 means ready for human review.",
    },
    ExitCode {
        family: "council",
        code: 4,
        name: "refused",
        meaning: "Malformed input, stale admission, an existing artifact, or a protocol or I/O failure.",
    },
    ExitCode {
        family: "council",
        code: 7,
        name: "hold",
        meaning: "HOLD: failed, missing or inconclusive evidence.",
    },
];

/// The command family an argument vector belongs to.
pub fn family_of(args: &[String]) -> &'static str {
    if args.first().map(String::as_str) == Some("council") {
        "council"
    } else {
        "engine"
    }
}

/// The registry entry for `code` in `family`, if documented.
pub fn lookup(family: &str, code: i32) -> Option<&'static ExitCode> {
    EXIT_CODES
        .iter()
        .find(|e| e.family == family && e.code == code)
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

fn result_schema() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "https://github.com/Louranicas/deep-diff-forge/schemas/ddf.result.v1.json",
        "title": RESULT_SCHEMA,
        "description": "What one deep-diff-forge command did, as reported by `deep-diff-forge envelope -- <command…>`.",
        "type": "object",
        "required": ["schema", "version", "command", "family", "exit_code", "outcome", "ok", "stdout", "stderr", "truncated"],
        "additionalProperties": false,
        "properties": {
            "schema": {"const": RESULT_SCHEMA},
            "version": {"type": "string", "description": "The deep-diff-forge version that ran the command."},
            "command": {"type": "array", "items": {"type": "string"}},
            "family": {"enum": ["engine", "council"]},
            "exit_code": {"type": ["integer", "null"], "description": "null when the command ended on a signal."},
            "signal": {"type": "integer"},
            "outcome": {"type": "string", "description": "The registry name of exit_code in this family (`deep-diff-forge schema exit-codes`), `undocumented` or `signal`."},
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
    let trimmed = text.trim();
    let (body, format) = if trimmed.is_empty() {
        (json!(""), "empty")
    } else if stdout.1 {
        (json!(text), "text") // a cut value is never parsed
    } else {
        serde_json::from_str::<Value>(trimmed)
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

    #[test]
    fn every_family_code_pair_is_unique_and_named() {
        for (i, a) in EXIT_CODES.iter().enumerate() {
            assert!(!a.name.is_empty() && !a.meaning.is_empty());
            assert!(
                EXIT_CODES[i + 1..]
                    .iter()
                    .all(|b| (a.family, a.code) != (b.family, b.code)),
                "{a:?}"
            );
        }
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
