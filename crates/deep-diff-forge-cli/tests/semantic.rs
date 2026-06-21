//! CLI contract tests for `semantic <path>`.

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

fn write_temp(name: &str, contents: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("ddf-sem-{}-{name}", std::process::id()));
    std::fs::write(&path, contents).expect("write temp file");
    path
}

const RUST_SRC: &str = "fn alpha() {}\nstruct Point { x: i32 }\n";

#[test]
fn semantic_json_reports_schema_and_symbols() {
    let p = write_temp("a.rs", RUST_SRC);
    let (code, stdout, _) = run(&["semantic", p.to_str().unwrap(), "--json"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("\"schema\": \"deep-diff-forge.semantic.v0\""));
    assert!(stdout.contains("\"language\": \"rust\""));
    assert!(stdout.contains("\"parse_status\": \"parsed\""));
    assert!(stdout.contains("\"name\": \"alpha\""));
    assert!(stdout.contains("\"kind\": \"struct\""));
}

#[test]
fn semantic_human_lists_symbols() {
    let p = write_temp("b.rs", RUST_SRC);
    let (code, stdout, _) = run(&["semantic", p.to_str().unwrap()]);
    assert_eq!(code, 0);
    assert!(stdout.contains("rust, parsed"));
    assert!(stdout.contains("alpha"));
    assert!(stdout.contains("2 symbol(s)"));
}

#[test]
fn semantic_unsupported_extension_falls_back() {
    let p = write_temp("c.txt", RUST_SRC);
    let (code, stdout, _) = run(&["semantic", p.to_str().unwrap(), "--json"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("\"language\": \"unsupported\""));
    assert!(stdout.contains("fallback:UnsupportedLanguage"));
}

#[test]
fn semantic_missing_path_exits_two() {
    let (code, _, stderr) = run(&["semantic"]);
    assert_eq!(code, 2);
    assert!(stderr.contains("usage"));
}

#[test]
fn semantic_unreadable_file_exits_three() {
    let (code, _, stderr) = run(&["semantic", "/nonexistent/path/to/x.rs"]);
    assert_eq!(code, 3);
    assert!(stderr.contains("could not read"));
}

#[test]
fn semantic_broken_rust_reports_errors() {
    let p = write_temp("d.rs", "fn broken( {");
    let (code, stdout, _) = run(&["semantic", p.to_str().unwrap(), "--json"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("parsed_with_errors:"));
}
