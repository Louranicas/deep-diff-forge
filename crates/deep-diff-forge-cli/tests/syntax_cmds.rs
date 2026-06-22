//! End-to-end tests for the `highlight` and `structural` subcommands, including
//! the terminal-injection guarantee at the CLI boundary.

use std::io::Write as _;
use std::process::Command;

const ESC: u8 = 0x1b;

fn run(args: &[&str]) -> (i32, Vec<u8>) {
    let out = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .args(args)
        .output()
        .expect("run binary");
    (out.status.code().unwrap_or(-1), out.stdout)
}

fn write_temp(name: &str, bytes: &[u8]) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("ddf-synh-{}-{name}", std::process::id()));
    let mut f = std::fs::File::create(&path).expect("create");
    f.write_all(bytes).expect("write");
    path
}

#[test]
fn highlight_colorizes_rust() {
    let path = write_temp("hl.rs", b"fn main() { let x = 1; }\n");
    let (code, out) = run(&["highlight", path.to_str().unwrap(), "--color"]);
    assert_eq!(code, 0);
    // ANSI SGR codes are present, and the identifiers survive.
    assert!(out.contains(&ESC), "forced --color should emit SGR escapes");
    assert!(String::from_utf8_lossy(&out).contains("main"));
}

#[test]
fn highlight_no_color_is_plain() {
    let path = write_temp("hlplain.rs", b"fn main() {}\n");
    let (code, out) = run(&["highlight", path.to_str().unwrap(), "--no-color"]);
    assert_eq!(code, 0);
    assert!(!out.contains(&ESC), "--no-color must not emit escapes");
    assert!(String::from_utf8_lossy(&out).contains("fn main()"));
}

#[test]
fn highlight_neutralizes_escape_in_source_even_with_color() {
    // A string literal carrying a raw ESC must not leak it: only our SGR codes
    // are raw ESC; the source ESC is rendered as the literal \x1b.
    let path = write_temp("evil.rs", b"fn x() { let s = \"\x1b[2Jpwn\"; }\n");
    let (code, out) = run(&["highlight", path.to_str().unwrap(), "--color"]);
    assert_eq!(code, 0);
    let text = String::from_utf8_lossy(&out);
    assert!(text.contains("\\x1b[2Jpwn"), "source ESC must be escaped");
    // No raw clear-screen sequence survives.
    assert!(
        !out.windows(4).any(|w| w == [ESC, b'[', b'2', b'J']),
        "raw ESC[2J must not reach the terminal"
    );
}

#[test]
fn structural_reports_reformat_only() {
    let old = write_temp("a.rs", b"fn main(){let x=1;}\n");
    let new = write_temp("b.rs", b"fn   main ( ) {\n    let x = 1 ;\n}\n");
    let (code, out) = run(&["structural", old.to_str().unwrap(), new.to_str().unwrap()]);
    assert_eq!(code, 0);
    assert!(String::from_utf8_lossy(&out).contains("formatting only"));
}

#[test]
fn structural_json_reports_token_changes() {
    let old = write_temp("c.rs", b"fn f() { a(); b(); }\n");
    let new = write_temp("d.rs", b"fn f() { a(); c(); }\n");
    let (code, out) = run(&[
        "structural",
        old.to_str().unwrap(),
        new.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(code, 0);
    let text = String::from_utf8_lossy(&out);
    assert!(text.contains("\"schema\": \"deep-diff-forge.structural.v0\""));
    assert!(text.contains("\"reformat_only\": false"));
    assert!(text.contains("\"text\": \"b\""));
    assert!(text.contains("\"text\": \"c\""));
}

#[test]
fn structural_missing_args_is_usage_error() {
    let only = write_temp("one.rs", b"fn main() {}\n");
    let (code, _) = run(&["structural", only.to_str().unwrap()]);
    assert_eq!(code, 2);
}

#[test]
fn structural_unsupported_language_errors_not_reformat_only() {
    // A non-Rust file must NOT silently report "formatting only" (both sides
    // tokenize empty); it errors explicitly instead.
    let a = write_temp("a.txt", b"hello\n");
    let b = write_temp("b.txt", b"world\n");
    let (code, out) = run(&["structural", a.to_str().unwrap(), b.to_str().unwrap()]);
    assert_eq!(code, 2);
    assert!(!String::from_utf8_lossy(&out).contains("formatting only"));
}
