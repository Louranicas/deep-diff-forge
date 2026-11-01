//! Every command `--help` lists answers `<command> --help` with exit 0 (expert circle 2026-10-28,
//! D7). Before, only `review --help` and `council help` did: the others exited 2, and
//! `council --help` exited 4.
#![expect(
    clippy::disallowed_methods,
    reason = "integration test: drives real processes"
)]

use std::process::{Command, Stdio};

fn run(args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .args(args)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    (
        out.status.code().unwrap(),
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

/// The command word of each usage line under `--help`'s USAGE heading.
fn listed_commands() -> Vec<String> {
    let (code, help, _) = run(&["--help"]);
    assert_eq!(code, 0);
    let usage = &help[help.find("USAGE:\n").unwrap() + 7..];
    usage
        .lines()
        .take_while(|line| line.starts_with("  deep-diff-forge "))
        .map(|line| {
            let word = line["  deep-diff-forge ".len()..]
                .split_whitespace()
                .next()
                .unwrap();
            word.trim_start_matches('{').to_owned()
        })
        .collect()
}

#[test]
fn every_listed_command_answers_its_own_help() {
    let commands = listed_commands();
    assert!(commands.len() >= 20, "{commands:?}");
    for command in commands.iter().filter(|c| *c != "--help") {
        for flag in ["--help", "-h"] {
            let (code, stdout, stderr) = run(&[command, flag]);
            assert_eq!(code, 0, "{command} {flag}: {stderr}");
            // Each names its command; `council` prints its own one-line usage.
            assert!(
                stdout.contains(command.as_str()),
                "{command} {flag}: {stdout}"
            );
        }
    }
}

/// A command's help is its line from `--help`, so the two cannot drift; `council --help` is
/// `council help`.
#[test]
fn a_command_help_repeats_its_usage_line() {
    let (_, help, _) = run(&["--help"]);
    let (code, daemon, _) = run(&["daemon", "--help"]);
    assert_eq!(code, 0);
    let line = daemon
        .lines()
        .find(|l| l.starts_with("  deep-diff-forge daemon"))
        .unwrap();
    assert!(help.contains(&format!("{line}\n")), "{line}");
    let (council, council_help) = (run(&["council", "--help"]), run(&["council", "help"]));
    assert_eq!(council.0, 0);
    assert_eq!(council.1, council_help.1);
}

/// Council help is also position-independent and never interprets another token as a run path.
#[test]
fn council_help_anywhere_does_not_read_a_run_path() {
    for args in [
        &["council", "--help", "prepare"][..],
        &["council", "status", "missing", "-h"][..],
    ] {
        let (code, stdout, stderr) = run(args);
        assert_eq!(code, 0, "{args:?}: {stderr}");
        assert!(stdout.contains("council example"), "{args:?}: {stdout}");
    }
}

/// Help is intercepted before command validation or input reads, wherever it appears.
#[test]
fn help_later_in_the_line_prints_command_help() {
    for args in [
        &["explain", "7", "--help"][..],
        &["--stdin-patch", "--json", "-h"][..],
        &["daemon", "start", "--help"][..],
        &["council", "status", "missing", "--help"][..],
    ] {
        let (code, stdout, stderr) = run(args);
        assert_eq!(code, 0, "{args:?}: {stderr}");
        assert!(!stdout.is_empty(), "{args:?}");
        assert!(stderr.is_empty(), "{args:?}: {stderr}");
    }
}

#[cfg(unix)]
#[test]
fn a_non_utf8_argument_is_an_io_error_not_a_panic() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt as _;
    let output = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .arg(OsString::from_vec(b"bad\xff".to_vec()))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(3));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("panicked"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("not valid UTF-8"));
}

/// Bug hunt 2026-11-01: a `-h` given as an option's value is the value. Before, `review
/// --findings -h` printed the review help with exit 0 instead of reading the file named `-h`.
#[test]
fn a_dash_h_value_is_not_a_help_request() {
    let (code, stdout, stderr) = run(&["review", "--findings", "-h"]);
    assert_eq!((code, stdout.as_str()), (3, ""), "{stderr}");
    assert!(stderr.contains("cannot read findings -h"), "{stderr}");
}

/// Bug hunt 2026-11-01: `review` takes the word `help` only as its first argument. Before, a
/// `help` anywhere, as in `review --side help`, printed the review help with exit 0 instead of
/// refusing the stray word.
#[test]
fn review_takes_help_only_as_its_first_word() {
    let (code, stdout, _) = run(&["review", "help"]);
    assert_eq!(code, 0);
    assert!(stdout.starts_with("deep-diff-forge review"), "{stdout}");
    let (code, stdout, stderr) = run(&["review", "--side", "help"]);
    assert_eq!((code, stdout.as_str()), (2, ""), "{stderr}");
    assert!(stderr.contains("unexpected argument: help"), "{stderr}");
}
