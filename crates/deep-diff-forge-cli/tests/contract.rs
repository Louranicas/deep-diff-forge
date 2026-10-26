//! Slice A: the machine contract (`envelope`, `schema`, `explain`) from the outside.
use std::io::Write as _;
use std::process::{Command, Stdio};

const PATCH: &str = "--- a/x\n+++ b/x\n@@ -1,1 +1,1 @@\n-old\n+new\n";

fn run(args: &[&str], stdin: &str) -> (i32, String, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    (
        out.status.code().unwrap(),
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

fn envelope(args: &[&str], stdin: &str) -> (i32, serde_json::Value) {
    let (code, stdout, stderr) = run(args, stdin);
    let value = serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("{e}: {stdout} {stderr}"));
    (code, value)
}

#[test]
fn envelope_wraps_a_json_command_and_passes_stdin_through() {
    let (code, v) = envelope(&["envelope", "--", "--stdin-patch", "--json"], PATCH);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["schema"], "ddf.result.v1");
    assert_eq!(
        (v["outcome"].as_str(), v["ok"].as_bool()),
        (Some("success"), Some(true))
    );
    assert_eq!(v["stdout_format"], "json");
    assert_eq!(v["stdout"]["schema"], "deep-diff-forge.review.v0");
    assert_eq!(v["command"], serde_json::json!(["--stdin-patch", "--json"]));
}

#[test]
fn envelope_keeps_the_commands_exit_code_and_names_it() {
    let (code, v) = envelope(
        &[
            "envelope",
            "--",
            "--stdin-patch",
            "--json",
            "--require-files",
        ],
        "",
    );
    assert_eq!(code, 7, "{v}");
    assert_eq!(
        (v["outcome"].as_str(), v["ok"].as_bool()),
        (Some("input_contract_refused"), Some(false))
    );
    assert_eq!(v["stdout_format"], "empty");
    assert!(
        v["stderr"].as_str().unwrap().contains("refused: 0 files"),
        "{v}"
    );
    let (code, v) = envelope(&["envelope", "--", "no-such-command"], "");
    assert_eq!((code, v["outcome"].as_str()), (2, Some("usage")));
}

#[test]
fn envelope_names_the_council_family() {
    let (code, v) = envelope(&["envelope", "--", "council", "help"], "");
    assert_eq!(code, 0, "{v}");
    assert_eq!(
        (v["family"].as_str(), v["outcome"].as_str()),
        (Some("council"), Some("completed"))
    );
}

#[test]
fn envelope_usage_errors_exit_2_with_nothing_on_stdout() {
    for args in [
        &["envelope"][..],
        &["envelope", "doctor"],
        &["envelope", "--", "envelope", "--", "doctor"],
    ] {
        let (code, stdout, stderr) = run(args, "");
        assert_eq!(code, 2, "{args:?}");
        assert!(
            stdout.is_empty() && stderr.starts_with("error: "),
            "{args:?}: {stderr}"
        );
    }
}

#[test]
fn explain_and_schema_answer_from_the_registry() {
    let (code, stdout, _) = run(&["explain", "4", "--family", "council"], "");
    assert_eq!(code, 0);
    assert!(stdout.starts_with("council 4 (refused): "), "{stdout}");
    let (code, _, _) = run(&["explain", "5"], "");
    assert_eq!(code, 2);
    let (code, stdout, _) = run(&["schema"], "");
    assert_eq!(code, 0);
    let schema: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(schema["properties"]["schema"]["const"], "ddf.result.v1");
}

/// The README's exit-code tables and the binary's registry name the same codes, and each README
/// row begins with the registry's meaning.
#[test]
fn readme_exit_code_tables_match_the_registry() {
    let readme =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../README.md")).unwrap();
    let section = &readme[readme.find("## Exit codes").unwrap()..];
    let section = &section[..section.find("\n## ").unwrap()];
    let (code, stdout, _) = run(&["schema", "exit-codes"], "");
    assert_eq!(code, 0);
    let registry: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    for (family, header) in [
        ("engine", "| Code | Meaning |"),
        ("council", "| Council code | Meaning |"),
    ] {
        let table = &section[section.find(header).unwrap_or_else(|| panic!("{header}"))..];
        let rows: Vec<(i64, String)> = table
            .lines()
            .skip(2)
            .take_while(|l| l.starts_with("| "))
            .map(|l| {
                let cells: Vec<&str> = l.trim_matches('|').split(" | ").map(str::trim).collect();
                (cells[0].parse().unwrap(), cells[1].to_owned())
            })
            .collect();
        let codes: Vec<&serde_json::Value> = registry["codes"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|c| c["family"] == family)
            .collect();
        assert_eq!(rows.len(), codes.len(), "{family}: {rows:?}");
        for (row, entry) in rows.iter().zip(codes) {
            assert_eq!(row.0, entry["code"].as_i64().unwrap(), "{family}");
            assert!(
                row.1.starts_with(entry["meaning"].as_str().unwrap()),
                "{family} {}: {}",
                row.0,
                row.1
            );
        }
    }
}
