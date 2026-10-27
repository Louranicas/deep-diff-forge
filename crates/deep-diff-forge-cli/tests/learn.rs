//! Anchor A-08: `deep-diff-forge learn` end to end, in an isolated `XDG_STATE_HOME`. Before, no
//! test ran the command, so replacing `learn_cmd` with a no-op survived mutation.
#![expect(
    clippy::disallowed_methods,
    reason = "integration test: drives the binary with an isolated environment"
)]

use std::io::Write as _;
use std::process::{Command, Stdio};

const RECEIPT: &str = r#"{"file_hash":"0123456789abcdef","language":"rust","parser_version":"v1","strategy":"syntax","fallback":null,"elapsed_ms":3,"bytes":10,"nodes":2,"cache":"cold","outcome":"accepted","revisited":false}"#;

fn learn(state: &std::path::Path, args: &[&str], stdin: &str) -> (i32, String, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .arg("learn")
        .args(args)
        .env("XDG_STATE_HOME", state)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // A command that refuses before reading may close stdin first.
    let _ = child.stdin.take().unwrap().write_all(stdin.as_bytes());
    let out = child.wait_with_output().unwrap();
    (
        out.status.code().unwrap(),
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

fn total(json: &str) -> u64 {
    let value: serde_json::Value = serde_json::from_str(json).unwrap();
    assert_eq!(value["schema"], "deep-diff-forge.learning.v0");
    value["total_receipts"].as_u64().unwrap()
}

#[test]
fn learn_records_receipts_and_reports_them() {
    let state = std::env::temp_dir().join(format!("ddf-learn-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&state);
    std::fs::create_dir_all(&state).unwrap();

    let (code, out, err) = learn(&state, &["status", "--json"], "");
    assert_eq!(code, 0, "{err}");
    assert_eq!(total(&out), 0);

    let (code, _, err) = learn(&state, &["record", "--stdin"], RECEIPT);
    assert_eq!(code, 0, "{err}");
    let (code, out, _) = learn(&state, &["--json"], "");
    assert_eq!(code, 0);
    assert_eq!(total(&out), 1, "status is the default subcommand");

    // A path where the redacted id belongs is refused when read (C-07), and a self-contradicting
    // receipt is refused by the store (C-11); neither is recorded.
    let path = RECEIPT.replace("0123456789abcdef", "src/secret.rs");
    let (code, _, err) = learn(&state, &["record", "--stdin"], &path);
    assert_eq!(code, 4, "{err}");
    let contradictory = RECEIPT.replace(r#""outcome":"accepted""#, r#""outcome":"revisited""#);
    let (code, _, err) = learn(&state, &["record", "--stdin"], &contradictory);
    assert_ne!(code, 0, "{err}");
    assert!(err.contains("contradicts"), "{err}");
    let (_, out, _) = learn(&state, &["status", "--json"], "");
    assert_eq!(total(&out), 1);

    let (code, out, err) = learn(&state, &["bogus"], "");
    assert_eq!((code, out.as_str()), (2, ""), "{err}");
    let (code, _, _) = learn(&state, &["record"], "");
    assert_eq!(code, 2, "record needs --stdin");

    std::fs::remove_dir_all(&state).unwrap();
}
