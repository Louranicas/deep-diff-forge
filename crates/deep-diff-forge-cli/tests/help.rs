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

/// `--help` only counts as the first argument; elsewhere the command's own rules apply.
#[test]
fn help_later_in_the_line_is_not_intercepted() {
    let (code, stdout, _) = run(&["explain", "7", "--help"]);
    assert_eq!(code, 2);
    assert_eq!(stdout, "");
}
