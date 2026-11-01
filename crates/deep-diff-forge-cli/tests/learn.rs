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
    // receipt or a path-shaped language is refused by the store (C-11); none is recorded, and each
    // is an invalid input (exit 4), never the retryable I/O exit 3.
    let path = RECEIPT.replace("0123456789abcdef", "src/secret.rs");
    let (code, _, err) = learn(&state, &["record", "--stdin"], &path);
    assert_eq!(code, 4, "{err}");
    let contradictory = RECEIPT.replace(r#""outcome":"accepted""#, r#""outcome":"revisited""#);
    let (code, _, err) = learn(&state, &["record", "--stdin"], &contradictory);
    assert_eq!(code, 4, "{err}");
    assert!(err.contains("contradicts"), "{err}");
    let language = RECEIPT.replace(r#""language":"rust""#, r#""language":"src/secret.rs""#);
    let (code, _, err) = learn(&state, &["record", "--stdin"], &language);
    assert_eq!(code, 4, "{err}");
    assert!(err.contains("invalid receipt: language"), "{err}");
    let (_, out, _) = learn(&state, &["status", "--json"], "");
    assert_eq!(total(&out), 1);

    let (code, out, err) = learn(&state, &["bogus"], "");
    assert_eq!((code, out.as_str()), (2, ""), "{err}");
    let (code, _, _) = learn(&state, &["record"], "");
    assert_eq!(code, 2, "record needs --stdin");

    std::fs::remove_dir_all(&state).unwrap();
}

/// The store keeps every receipt and shows every hand edit: a record appended after a final
/// newline went missing is its own line (before, it was glued onto the last one and every later
/// `learn status` failed while `learn record` exited 0), and a line carrying a path in an extra
/// key is refused, naming its line (anchor A-05).
#[test]
fn learn_store_survives_a_lost_newline_and_refuses_extra_keys() {
    let state = std::env::temp_dir().join(format!("ddf-learn-torn-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&state);
    std::fs::create_dir_all(&state).unwrap();
    let store = state.join("deep-diff-forge/learning/receipts/strategy.jsonl");

    for _ in 0..2 {
        let (code, _, err) = learn(&state, &["record", "--stdin"], RECEIPT);
        assert_eq!(code, 0, "{err}");
    }
    let mut body = std::fs::read(&store).unwrap();
    assert_eq!(body.pop(), Some(b'\n'));
    std::fs::write(&store, &body).unwrap();
    let (code, _, err) = learn(&state, &["record", "--stdin"], RECEIPT);
    assert_eq!(code, 0, "{err}");
    let (code, out, err) = learn(&state, &["status", "--json"], "");
    assert_eq!(code, 0, "{err}");
    assert_eq!(total(&out), 3);

    let carrying = RECEIPT.replacen('{', r#"{"path":"secret/src/payments.rs","#, 1);
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&store)
        .unwrap();
    writeln!(file, "{carrying}").unwrap();
    let (code, _, err) = learn(&state, &["status", "--json"], "");
    assert_eq!(code, 3, "{err}");
    assert!(
        err.contains("receipt store line 4: invalid receipt: a stored receipt holds a key"),
        "{err}"
    );
    assert!(!err.contains("secret"), "the key is not echoed: {err}");

    std::fs::remove_dir_all(&state).unwrap();
}

/// A relative `XDG_STATE_HOME` is invalid (XDG Base Directory spec) and ignored: the store goes
/// under `$HOME/.local/state`, never under the working directory (often the repository under
/// review).
#[test]
fn a_relative_state_home_never_puts_the_store_under_the_working_directory() {
    let root = std::env::temp_dir().join(format!("ddf-learn-relative-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let (cwd, home) = (root.join("checkout"), root.join("home"));
    std::fs::create_dir_all(&cwd).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .args(["learn", "record", "--stdin"])
        .current_dir(&cwd)
        .env("XDG_STATE_HOME", "relstate")
        .env("HOME", &home)
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(RECEIPT.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!cwd.join("relstate").exists(), "nothing under the cwd");
    assert!(
        home.join(".local/state/deep-diff-forge/learning/receipts/strategy.jsonl")
            .is_file()
    );
    std::fs::remove_dir_all(&root).unwrap();
}

/// `doctor` names the state directory the store uses. Before, it formatted a relative
/// `XDG_STATE_HOME` as is (`state_dir=relstate/deep-diff-forge`), a place nothing writes to.
#[test]
fn doctor_reports_the_state_dir_the_store_uses() {
    let root = std::env::temp_dir().join(format!("ddf-doctor-state-{}", std::process::id()));
    let home = root.join("home");
    let doctor = |state_home: &std::ffi::OsStr| {
        let out = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
            .arg("doctor")
            .env("XDG_STATE_HOME", state_home)
            .env("HOME", &home)
            .output()
            .unwrap();
        assert!(out.status.success());
        String::from_utf8(out.stdout).unwrap()
    };
    let expected = format!(
        "state_dir={}\n",
        home.join(".local/state/deep-diff-forge").display()
    );
    assert!(doctor("relstate".as_ref()).contains(&expected));
    let absolute = root.join("state");
    assert!(doctor(absolute.as_os_str()).contains(&format!(
        "state_dir={}\n",
        absolute.join("deep-diff-forge").display()
    )));
    let out = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .arg("doctor")
        .env("XDG_STATE_HOME", "relstate")
        .env("HOME", "relhome")
        .output()
        .unwrap();
    assert!(
        String::from_utf8(out.stdout)
            .unwrap()
            .contains("state_dir=<unset: set XDG_STATE_HOME or HOME to an absolute path>\n")
    );
}
