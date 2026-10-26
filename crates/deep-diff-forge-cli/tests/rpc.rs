//! Slice C: `deep-diff-forge rpc` over real stdio, as the SDKs use it.
use std::io::Write as _;
use std::process::{Command, Stdio};

#[test]
fn rpc_answers_on_stdout_and_ends_at_shutdown() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .arg("rpc")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    writeln!(stdin, r#"{{"id":1,"method":"engine.initialize"}}"#).unwrap();
    writeln!(stdin, r#"{{"id":2,"method":"daemon.shutdown"}}"#).unwrap();
    writeln!(stdin, r#"{{"id":3,"method":"daemon.health"}}"#).unwrap();
    drop(stdin);
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let lines: Vec<serde_json::Value> = String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines.len(), 2, "nothing after shutdown: {lines:?}");
    assert_eq!(lines[0]["result"]["protocol"], 1);
    assert_eq!(lines[1]["result"]["stopping"], true);
}

/// A reader that went away is a clean end (exit 0); any other write failure is exit 3.
#[test]
fn a_closed_reader_ends_cleanly_and_a_failed_write_is_an_io_failure() {
    for (stdout, expected) in [
        (Stdio::piped(), 0),
        (
            Stdio::from(
                std::fs::OpenOptions::new()
                    .write(true)
                    .open("/dev/full")
                    .unwrap(),
            ),
            3,
        ),
    ] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
            .arg("rpc")
            .stdin(Stdio::piped())
            .stdout(stdout)
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        drop(child.stdout.take());
        let mut stdin = child.stdin.take().unwrap();
        writeln!(stdin, r#"{{"id":1,"method":"daemon.health"}}"#).unwrap();
        drop(stdin);
        assert_eq!(child.wait().unwrap().code(), Some(expected));
    }
}

#[test]
fn rpc_takes_no_arguments() {
    let out = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .args(["rpc", "x"])
        .output()
        .unwrap();
    assert_eq!((out.status.code(), out.stdout.len()), (Some(2), 0));
}
