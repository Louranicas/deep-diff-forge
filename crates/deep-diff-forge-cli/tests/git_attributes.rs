//! S3.20: what the reviewed repository says about its own files must not blind the review. One
//! line `*.rs -diff`, in any attributes file Git reads, made `git diff` print a Rust change as a
//! `GIT binary patch`, and every evidence rule went blind: a change that starts a process ranked
//! as a harmless binary change (`binary_changed`, 3, `binary_change`). A NUL byte in a comment
//! did the same with no attribute at all. Each way Git can be told so is built here in a real
//! repository, in the three Git selections, and read through the binary: the evidence is found
//! and the act itself is labelled `diff_suppressed`, with its cause.
#![cfg(unix)]
#![expect(
    clippy::disallowed_methods,
    reason = "integration test: drives git, the binary and temp directories"
)]

use deep_diff_forge_testkit::Scratch;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;

const ISOLATE: [(&str, &str); 2] = [
    ("GIT_CONFIG_GLOBAL", "/dev/null"),
    ("GIT_CONFIG_NOSYSTEM", "1"),
];

fn git(root: &Path, args: &[&str]) {
    let out = Command::new("git")
        .current_dir(root)
        .envs(ISOLATE)
        .args([
            "-c",
            "user.name=ddf",
            "-c",
            "user.email=ddf@example.invalid",
        ])
        .args([
            "-c",
            "commit.gpgSign=false",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

struct Repo(
    PathBuf,
    #[expect(dead_code, reason = "held for its Drop, which removes the directory")] Scratch,
);

/// How Git is told that `m.rs` is binary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hide {
    /// `*.rs -diff` in a `.gitattributes` that is never committed or staged.
    TreeUntracked,
    /// `*.rs -diff` in a `.gitattributes` the change itself adds.
    TreeInChange,
    /// `*.rs -diff` in `.git/info/attributes`, which no diff ever shows.
    InfoAttributes,
    /// `*.rs -diff` in a file the repository's own `core.attributesFile` names.
    ConfigAttributesFile,
    /// `* binary` in a `.gitattributes` committed before the change.
    StarBinary,
    /// `*.rs diff=bin` with `diff.bin.binary = true` in the repository's configuration.
    DriverBinary,
    /// No attribute: one NUL byte in a comment.
    Nul,
    /// Controls: no attribute (an undefined driver falls back to text), a real image.
    UndefinedDriver,
    Plain,
    Image,
}

const HIDDEN: [Hide; 7] = [
    Hide::TreeUntracked,
    Hide::TreeInChange,
    Hide::InfoAttributes,
    Hide::ConfigAttributesFile,
    Hide::StarBinary,
    Hide::DriverBinary,
    Hide::Nul,
];

const MODES: [&str; 3] = ["--git-range", "--git-working-tree", "--git-staged"];

fn source(nul: bool, spawn: bool) -> Vec<u8> {
    let mut text = b"fn main() {\n".to_vec();
    if nul {
        text.extend_from_slice(b"    // marker \0 here\n");
    }
    text.extend_from_slice(b"    let x = 1;\n");
    if spawn {
        text.extend_from_slice(
            b"    let _ = std::process::Command::new(\"sh\").arg(\"-c\").arg(\"id\").status();\n",
        );
    }
    text.extend_from_slice(b"    println!(\"{x}\");\n}\n");
    text
}

/// A PNG signature and every byte value, NUL included: binary to Git, and not UTF-8.
fn image(seed: u8) -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    bytes.extend((0..=255u8).cycle().take(600).map(|b| b.wrapping_mul(seed)));
    bytes
}

/// The repository of `hide` with its change made the way `mode` reads it: committed for a
/// range, staged, or left in the working tree.
fn repo(hide: Hide, mode: &str) -> Repo {
    let scratch = Scratch::new(&format!("cli-attr-{hide:?}{}", mode.len()));
    let r = Repo(std::fs::canonicalize(scratch.path()).unwrap(), scratch);
    let root = &r.0;
    let write = |path: &str, bytes: &[u8]| std::fs::write(root.join(path), bytes).unwrap();
    git(root, &["init", "-q"]);
    let file = if hide == Hide::Image {
        "img.png"
    } else {
        "m.rs"
    };
    let content = |after: bool| match hide {
        Hide::Image => image(if after { 7 } else { 3 }),
        _ => source(hide == Hide::Nul, after),
    };
    write(file, &content(false));
    match hide {
        Hide::StarBinary => write(".gitattributes", b"* binary\n"),
        Hide::InfoAttributes => {
            std::fs::create_dir_all(root.join(".git/info")).unwrap();
            write(".git/info/attributes", b"*.rs -diff\n");
        }
        Hide::ConfigAttributesFile => {
            write(".git/attrs", b"*.rs -diff\n");
            let attrs = root.join(".git/attrs");
            git(
                root,
                &["config", "core.attributesFile", attrs.to_str().unwrap()],
            );
        }
        Hide::DriverBinary => git(root, &["config", "diff.bin.binary", "true"]),
        _ => {}
    }
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "base"]);
    write(file, &content(true));
    if hide == Hide::TreeInChange {
        write(".gitattributes", b"*.rs -diff\n");
        git(root, &["add", ".gitattributes"]);
    }
    match mode {
        "--git-range" => {
            git(root, &["add", "-A"]);
            git(root, &["commit", "-q", "-m", "change"]);
        }
        "--git-staged" => git(root, &["add", "-A"]),
        _ => {}
    }
    match hide {
        Hide::TreeUntracked => write(".gitattributes", b"*.rs -diff\n"),
        Hide::DriverBinary => write(".gitattributes", b"*.rs diff=bin\n"),
        Hide::UndefinedDriver => write(".gitattributes", b"*.rs diff=nonexistent\n"),
        _ => {}
    }
    r
}

fn selection(mode: &str) -> Vec<&str> {
    if mode == "--git-range" {
        vec![mode, "HEAD~1..HEAD"]
    } else {
        vec![mode]
    }
}

/// The `--rank --json` entry of `path`.
fn ranked(r: &Repo, mode: &str, path: &str) -> Value {
    let out = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .envs(ISOLATE)
        .args(selection(mode))
        .arg("--repo")
        .arg(&r.0)
        .args(["--rank", "--json"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
    doc["ranked"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["path"] == path)
        .cloned()
        .unwrap_or_else(|| panic!("{path} not ranked: {doc}"))
}

/// Before the fix every row read `binary_changed 3 [binary_change]` in every mode (measured on
/// the 0.9.0 line); now the process spawn is found and the act is labelled, with its cause.
#[test]
fn a_text_file_git_was_told_to_hide_is_read_and_the_act_is_labelled() {
    for hide in HIDDEN {
        let cause = if hide == Hide::Nul {
            "content"
        } else {
            "attribute"
        };
        for mode in MODES {
            let entry = ranked(&repo(hide, mode), mode, "m.rs");
            let row = format!("{hide:?} {mode}: {entry}");
            assert_eq!(entry["status"], "modified", "{row}");
            assert_eq!(
                entry["signals"],
                json!(["diff_suppressed", "process_spawn"]),
                "{row}"
            );
            assert_eq!(entry["score"], 11, "{row}");
            assert_eq!(entry["diff_suppressed"], cause, "{row}");
            assert_eq!(entry["evidence"][0]["signal"], "process_spawn", "{row}");
        }
    }
}

/// A real binary stays binary and carries no label; an ordinary file, or one whose `diff`
/// attribute names a driver nobody defined (Git falls back to text), is read as before.
#[test]
fn a_real_binary_or_an_ordinary_file_is_read_as_before() {
    for mode in MODES {
        let image = ranked(&repo(Hide::Image, mode), mode, "img.png");
        assert_eq!(image["status"], "binary_changed", "{mode}: {image}");
        assert_eq!(image["signals"], json!(["binary_change"]), "{mode}");
        assert!(image.get("diff_suppressed").is_none(), "{mode}: {image}");
        for hide in [Hide::Plain, Hide::UndefinedDriver] {
            let entry = ranked(&repo(hide, mode), mode, "m.rs");
            assert_eq!(
                entry["signals"],
                json!(["process_spawn"]),
                "{hide:?} {mode}"
            );
            assert_eq!(entry["score"], 6, "{hide:?} {mode}");
            assert!(entry.get("diff_suppressed").is_none(), "{hide:?} {mode}");
        }
    }
}

/// `review.change` (the MCP and RPC servers) reads a change the same way: its rank view is the
/// CLI's for the same repository, label and cause included.
#[test]
fn review_change_reads_the_hidden_change_as_the_cli_does() {
    use deep_diff_forge_daemon::{Engine, dispatch, parse_request};
    let r = repo(Hide::InfoAttributes, "--git-staged");
    let engine = Engine::new()
        .allowing_repos(std::slice::from_ref(&r.0))
        .unwrap();
    let line = json!({"method": "review.change", "params": {"repo": r.0, "selection": "staged", "view": "rank"}})
        .to_string();
    let answer = dispatch(&parse_request(&line).unwrap(), &engine).unwrap();
    let entry = &answer["ranked"][0];
    assert_eq!(
        entry["signals"],
        json!(["diff_suppressed", "process_spawn"])
    );
    assert_eq!(entry["diff_suppressed"], "attribute");
    assert_eq!(*entry, ranked(&r, "--git-staged", "m.rs"));
}
