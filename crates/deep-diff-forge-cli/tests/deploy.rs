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
    assert!(stdout.contains("\"maturity\": \"L6\""));
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
    assert!(stdout.contains("L6 (Cluster)"));
    assert!(stdout.contains("just gate-feature"));
}

#[test]
fn status_json_includes_external_observers() {
    let (_, stdout, _) = run(&["deploy", "status", "--json"]);
    assert!(stdout.contains("\"external_observers\""));
    assert!(stdout.contains("\"habitat\": \"optional\""));
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
