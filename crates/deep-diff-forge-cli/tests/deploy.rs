//! CLI contract tests for `deploy status`.

use std::process::Command;

fn run(args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .args(args)
        .output()
        .expect("run binary");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn status_json_declares_schema_and_maturity() {
    let (code, stdout, _) = run(&["deploy", "status", "--json"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("\"schema\": \"deep-diff-forge.deployment-status.v0\""));
    assert!(stdout.contains("\"maturity\": \"L9\""));
    assert!(stdout.contains("\"repo\": \"deep-diff-forge\""));
}

#[test]
fn status_json_lists_gate_stack() {
    let (code, stdout, _) = run(&["deploy", "status", "--json"]);
    assert_eq!(code, 0);
    for gate in [
        "identity", "format", "compile", "lint", "test", "fixture", "contract",
    ] {
        assert!(stdout.contains(gate), "missing gate {gate}");
    }
    assert!(stdout.contains("\"state\": \"not-run\""));
}

#[test]
fn status_human_shows_maturity_name() {
    let (code, stdout, _) = run(&["deploy", "status"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("L9 (Learning)"));
    assert!(stdout.contains("just gate-feature"));
}

#[test]
fn status_json_includes_external_observers() {
    let (_, stdout, _) = run(&["deploy", "status", "--json"]);
    assert!(stdout.contains("\"external_observers\""));
    assert!(stdout.contains("\"habitat\": \"optional\""));
}

#[test]
fn release_json_declares_schema_and_version() {
    let (code, stdout, _) = run(&["deploy", "release", "--json"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("\"schema\": \"deep-diff-forge.release.v0\""));
    assert!(stdout.contains("\"version\": \"0.2.0\""));
}

#[test]
fn release_json_reports_crates_io_blocked() {
    let (code, stdout, _) = run(&["deploy", "release", "--json"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("crates.io"));
    assert!(stdout.contains("\"state\": \"blocked\""));
    assert!(stdout.contains("\"pending\": [\"crates.io\"]"));
    assert!(stdout.contains("\"fully_published\": false"));
}

#[test]
fn release_human_lists_targets_and_pending() {
    let (code, stdout, _) = run(&["deploy", "release"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("github"));
    assert!(stdout.contains("gitlab"));
    assert!(stdout.contains("crates.io"));
    assert!(stdout.contains("pending: crates.io"));
}

#[test]
fn deploy_without_subcommand_exits_two() {
    let (code, _, stderr) = run(&["deploy"]);
    assert_eq!(code, 2);
    assert!(stderr.contains("usage"));
}

#[test]
fn deploy_unknown_subcommand_exits_two() {
    let (code, _, stderr) = run(&["deploy", "frobnicate"]);
    assert_eq!(code, 2);
    assert!(stderr.contains("usage"));
}
