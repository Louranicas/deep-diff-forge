// Design/status: docs/RELEASE_AND_PUBLICATION.md (release preparation: the notes' commands run)
//! D1 (0.11.0): every full command line the release notes quote is run on this build and is not
//! a usage error. The 0.10.0 notes told readers to run
//! `deep-diff-forge agent setup --client claude --hooks --ranker-dir DIR`, which 0.10.0 refused
//! as an unknown option; no check ran the notes' commands before the tag.
//!
//! The notes read are those of the tree's own version (`docs/release-notes/<version>.md`, when
//! it exists) and of the newest released version (the first version heading of `CHANGELOG.md`).
//! A command is a span in backticks that begins `deep-diff-forge `. A span that is a synopsis
//! (optional parts in `[…]`, alternatives with `|` or `{…}`, a repeat `...`) is not a command
//! line and is not run. A placeholder (`DIR`, `PATH`, `FILE`, `NAME`, `N`, `LIST`, `REV`, `ID`,
//! `A..B`, `<…>`) is run with a stand-in value that exists. Each command runs in a scratch
//! directory with its home and XDG directories there, stdin empty, for at most
//! [`LIMIT`]; one still running then is admitted and is ended. `daemon start` is not run (it
//! would leave a daemon); it is listed instead.
#![cfg(unix)]
#![expect(
    clippy::disallowed_methods,
    reason = "integration test: runs the binary under a deadline of its own"
)]

use deep_diff_forge_testkit::Scratch;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// How long one command may run before it counts as admitted and is ended.
const LIMIT: Duration = Duration::from_secs(20);

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The release notes to check: the tree's version (when written) and the newest released one.
fn notes() -> Vec<String> {
    let root = root();
    let changelog = std::fs::read_to_string(root.join("CHANGELOG.md")).unwrap();
    let released = changelog
        .lines()
        .filter_map(|line| line.strip_prefix("## [")?.split(']').next())
        .find(|version| *version != "Unreleased")
        .expect("the changelog names a released version")
        .to_owned();
    let own = env!("CARGO_PKG_VERSION")
        .split('-')
        .next()
        .unwrap()
        .to_owned();
    let mut files = vec![format!("docs/release-notes/{released}.md")];
    let own = format!("docs/release-notes/{own}.md");
    if !files.contains(&own) && root.join(&own).is_file() {
        files.push(own);
    }
    files
}

/// The spans in backticks of `text` with the line each begins on, whitespace (line breaks
/// included) folded to one space. A span opened by N backticks ends at the next run of exactly N.
fn code_spans(text: &str) -> Vec<(usize, String)> {
    let bytes = text.as_bytes();
    let run = |at: usize| bytes[at..].iter().take_while(|&&b| b == b'`').count();
    let mut spans = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] != b'`' {
            at += 1;
            continue;
        }
        let open = run(at);
        let start = at + open;
        let mut end = start;
        let close = loop {
            if end >= bytes.len() {
                break None;
            }
            if bytes[end] == b'`' {
                let n = run(end);
                if n == open {
                    break Some(end);
                }
                end += n;
            } else {
                end += 1;
            }
        };
        let Some(close) = close else {
            break;
        };
        let line = text[..at].matches('\n').count() + 1;
        let span = text[start..close].split_whitespace().collect::<Vec<_>>();
        spans.push((line, span.join(" ")));
        at = close + open;
    }
    spans
}

/// The stand-in for a placeholder word, or the word itself. `before` is the previous word: a
/// `PATH` after `--judge` is a file, any other `PATH` a directory.
fn stand_in(word: &str, before: &str, dir: &str, file: &str) -> Option<String> {
    let file_like = word == "FILE"
        || (word == "PATH" && before == "--judge")
        || (word.starts_with('<') && word.ends_with('>'));
    let value = match word {
        _ if file_like => file,
        "DIR" | "PATH" => dir,
        "NAME" => "policy.toml",
        "N" => "1",
        "LIST" => "security",
        "REV" => "HEAD",
        "ID" => "x",
        "A..B" => "HEAD..HEAD",
        _ => return None,
    };
    Some(value.to_owned())
}

/// A synopsis rather than a command line: optional parts, alternatives or a repeat.
fn synopsis(command: &str) -> bool {
    let words: String = command
        .split(' ')
        .filter(|word| !(word.starts_with('<') && word.ends_with('>')))
        .collect();
    ["[", "]", "{", "}", "|", "...", "…"]
        .iter()
        .any(|mark| words.contains(mark))
}

/// Runs `words` in `scratch` under [`LIMIT`]; `None` when it was still running then.
fn run(scratch: &Scratch, words: &[String]) -> Option<(i32, String)> {
    let home = scratch.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let stderr = scratch.join("stderr");
    let mut child = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .args(words)
        .current_dir(scratch.path())
        .env("HOME", &home)
        .env("XDG_STATE_HOME", home.join("state"))
        .env("XDG_CACHE_HOME", home.join("cache"))
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("XDG_RUNTIME_DIR", &home)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(std::fs::File::create(&stderr).unwrap())
        .spawn()
        .unwrap();
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            let said = std::fs::read_to_string(&stderr).unwrap_or_default();
            return Some((status.code().unwrap_or(-1), said));
        }
        if started.elapsed() >= LIMIT {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn every_command_the_release_notes_quote_is_admitted() {
    let scratch = Scratch::new("release-notes-commands");
    let dir = scratch.join("stand-in-dir");
    std::fs::create_dir(&dir).unwrap();
    let file = scratch.write("stand-in-file", "stand-in\n");
    let (dir, file) = (dir.to_str().unwrap(), file.to_str().unwrap());
    let (mut ran, mut refused, mut not_run) = (Vec::new(), Vec::new(), Vec::new());
    let files = notes();
    for name in &files {
        let text = std::fs::read_to_string(root().join(name)).unwrap();
        for (line, span) in code_spans(&text) {
            let Some(rest) = span.strip_prefix("deep-diff-forge ") else {
                continue;
            };
            let place = format!("{name}:{line}: `{span}`");
            if synopsis(&span) {
                not_run.push(format!("{place} (a synopsis)"));
                continue;
            }
            let mut words: Vec<String> = Vec::new();
            for word in rest.split(' ') {
                let before = words.last().map_or("", String::as_str);
                let value = stand_in(word, before, dir, file).unwrap_or_else(|| word.to_owned());
                words.push(value);
            }
            if words.starts_with(&["daemon".to_owned(), "start".to_owned()]) {
                not_run.push(format!("{place} (would leave a daemon)"));
                continue;
            }
            match run(&scratch, &words) {
                Some((2, said)) => refused.push(format!("{place} -> exit 2: {}", said.trim())),
                _ => ran.push(place),
            }
        }
    }
    assert!(
        refused.is_empty(),
        "a command the release notes quote is a usage error on this build:\n{}",
        refused.join("\n")
    );
    let newest = &files[0];
    assert!(
        ran.iter().any(|place| place.starts_with(newest.as_str())),
        "no command of {newest} was run (not run: {not_run:?})"
    );
}

#[test]
fn spans_and_synopses_are_read_as_the_check_needs() {
    let text = "a `deep-diff-forge x\n  --y DIR` b ``c ` d`` `deep-diff-forge z [--json]`";
    assert_eq!(
        code_spans(text),
        vec![
            (1, "deep-diff-forge x --y DIR".to_owned()),
            (2, "c ` d".to_owned()),
            (2, "deep-diff-forge z [--json]".to_owned()),
        ]
    );
    assert!(synopsis("deep-diff-forge z [--json]"));
    assert!(synopsis("deep-diff-forge agent hook session-start|stop"));
    assert!(!synopsis("deep-diff-forge explain signal <label>"));
    assert!(!synopsis(
        "deep-diff-forge agent doctor --gate --ranker-dir DIR"
    ));
    assert_eq!(
        stand_in("PATH", "--judge", "/d", "/f").as_deref(),
        Some("/f")
    );
    assert_eq!(
        stand_in("PATH", "--repo", "/d", "/f").as_deref(),
        Some("/d")
    );
    assert_eq!(stand_in("--gate", "", "/d", "/f"), None);
}
