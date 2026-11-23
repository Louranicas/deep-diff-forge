//! F7.i: `--rank --fail-on LIST`, the gate on the ranking, from the outside. It reads each file's
//! `signals[]` and its `evidence_truncated` bit, never the evidence rows; it refuses with exit 7
//! through the one door, after the document the run without the flag prints, with one reason line
//! per class and one escaped path line per file.
#![cfg(unix)]
#![expect(
    clippy::disallowed_methods,
    reason = "integration test: drives git and the binary"
)]

use deep_diff_forge_testkit::Scratch;
use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};

/// Git with no user or system configuration.
const ISOLATE: [(&str, &str); 2] = [
    ("GIT_CONFIG_GLOBAL", "/dev/null"),
    ("GIT_CONFIG_NOSYSTEM", "1"),
];

/// Run the binary with `args` and `stdin`: (exit code, stdout, stderr).
fn run(args: &[&str], stdin: &str) -> (Option<i32>, String, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .args(args)
        .envs(ISOLATE)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let _ = child.stdin.take().unwrap().write_all(stdin.as_bytes());
    let out = child.wait_with_output().unwrap();
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// `args` run with `--fail-on list` added and without it: the document must be the same bytes.
/// Returns the gated run's exit code and stderr.
fn gate(args: &[&str], list: &str, stdin: &str) -> (Option<i32>, String) {
    let (code, plain, stderr) = run(args, stdin);
    assert_eq!(code, Some(0), "{args:?} without --fail-on: {stderr}");
    let mut gated = args.to_vec();
    gated.extend(["--fail-on", list]);
    let (code, document, stderr) = run(&gated, stdin);
    assert_eq!(
        document, plain,
        "--fail-on {list} changed the document of {args:?}: {stderr}"
    );
    (code, stderr)
}

/// The stderr lines in the reason grammar `refused: <text> (<tag>)`.
fn reasons(stderr: &str) -> Vec<&str> {
    stderr
        .lines()
        .filter(|line| line.starts_with("refused: ") && line.ends_with(')'))
        .collect()
}

/// A patch that changes `file` (existing, Rust) by adding `lines`.
fn change(file: &str, lines: &[String]) -> String {
    let mut patch = format!(
        "--- a/{file}\n+++ b/{file}\n@@ -1 +1,{} @@\n fn a() {{\n",
        lines.len() + 1
    );
    for line in lines {
        patch.push('+');
        patch.push_str(line);
        patch.push('\n');
    }
    patch
}

/// The `--rank --json` entry of the only ranked file.
fn only_entry(stdin: &str) -> serde_json::Value {
    let (code, stdout, stderr) = run(&["--stdin-patch", "--rank", "--json"], stdin);
    assert_eq!(code, Some(0), "{stderr}");
    let doc: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    doc["ranked"][0].clone()
}

/// Trojan Source (B2): a bidi control in a changed line raises `hazardous_text`, class security,
/// with no evidence row. The gate reads `signals[]`, so the class refuses it.
#[test]
fn fail_on_refuses_a_trojan_source_patch() {
    let patch = change(
        "src/a.rs",
        &["    let access = \"user\u{202e} \u{2066}// admin\u{2069}\u{2066}\";".to_owned()],
    );
    let entry = only_entry(&patch);
    assert!(
        entry["evidence"]
            .as_array()
            .is_none_or(|rows| rows.iter().all(|r| r["signal"] != "hazardous_text")),
        "a row locates hazardous_text: {entry}"
    );
    let (code, stderr) = gate(&["--stdin-patch", "--rank", "--json"], "security", &patch);
    assert_eq!(code, Some(7), "the gate did not refuse: {stderr}");
    assert_eq!(
        reasons(&stderr),
        ["refused: 1 files carry a label of security (--fail-on)"],
        "{stderr}"
    );
    assert!(
        stderr.contains("\n  \"src/a.rs\": hazardous_text\n"),
        "{stderr}"
    );
}

/// `p3_cappad` (M11): 40 comment lines naming `#[allow(dead_code)]` fill the 32 evidence rows
/// before one `unsafe` line; every row is `check_suppressed`, `unsafe_added` is in `signals[]`
/// only. `--fail-on unsafe_added` refuses on it, and the cut list refuses as well.
#[test]
fn fail_on_reads_signals_not_rows() {
    let mut lines = vec!["    // #[allow(dead_code)] is named here".to_owned(); 40];
    lines.push("    unsafe { std::ptr::null::<u8>().read() };".to_owned());
    let patch = change("src/a.rs", &lines);
    let entry = only_entry(&patch);
    let rows = entry["evidence"].as_array().unwrap();
    assert_eq!(rows.len(), 32, "{entry}");
    assert!(
        rows.iter().all(|r| r["signal"] == "check_suppressed"),
        "{entry}"
    );
    assert!(
        entry["signals"]
            .as_array()
            .unwrap()
            .contains(&"unsafe_added".into()),
        "{entry}"
    );
    let (code, stderr) = gate(&["--stdin-patch", "--rank"], "unsafe_added", &patch);
    assert_eq!(code, Some(7), "the gate did not refuse: {stderr}");
    assert_eq!(
        reasons(&stderr),
        [
            "refused: 1 files carry a label of unsafe_added (--fail-on)",
            "refused: the evidence of 1 files was cut at 32 rows (--fail-on evidence_truncated)"
        ],
        "{stderr}"
    );
    assert!(
        stderr.contains("\n  \"src/a.rs\": unsafe_added\n"),
        "{stderr}"
    );
}

/// OD-C.2: a cut evidence list refuses whatever the list, here one that selects no label the
/// file carries; the gate cannot hand on a document that does not locate every reading.
#[test]
fn fail_on_refuses_when_the_evidence_was_cut() {
    let lines = vec!["    let _ = std::process::Command::new(\"sh\");".to_owned(); 33];
    let patch = change("src/a.rs", &lines);
    assert_eq!(only_entry(&patch)["evidence_truncated"], true);
    let (code, stderr) = gate(
        &["--stdin-patch", "--rank", "--json"],
        "secret_like",
        &patch,
    );
    assert_eq!(code, Some(7), "the gate did not refuse: {stderr}");
    assert_eq!(
        reasons(&stderr),
        ["refused: the evidence of 1 files was cut at 32 rows (--fail-on evidence_truncated)"],
        "{stderr}"
    );
    assert!(
        stderr.ends_with("\n  \"src/a.rs\": evidence_truncated\n"),
        "{stderr}"
    );
    // One row fewer: nothing is cut, and the list selects nothing: a pass.
    let patch = change("src/a.rs", &lines[..32]);
    let (code, stderr) = gate(
        &["--stdin-patch", "--rank", "--json"],
        "secret_like",
        &patch,
    );
    assert_eq!((code, stderr.as_str()), (Some(0), ""));
}

/// A list that selects no label of the change passes: exit 0, the same document, empty stderr,
/// in the JSON and the text form; a refusal leaves the document as it was too (see `gate`).
#[test]
fn fail_on_passes_with_the_same_document() {
    let patch = "--- a/docs/a.md\n+++ b/docs/a.md\n@@ -1 +1 @@\n-a\n+b\n";
    for args in [
        &["--stdin-patch", "--rank", "--json"][..],
        &["--stdin-patch", "--rank"][..],
    ] {
        for list in ["security", "integrity", "security,integrity", "new_file"] {
            let (code, stderr) = gate(args, list, patch);
            assert_eq!((code, stderr.as_str()), (Some(0), ""), "{args:?} {list}");
        }
    }
    let (code, stderr) = gate(
        &["--stdin-patch", "--rank"],
        "shape",
        "--- /dev/null\n+++ b/n.md\n@@ -0,0 +1 @@\n+n\n",
    );
    assert_eq!(code, Some(7), "new_file is a shape label: {stderr}");
}

/// Tokens are classes and labels only, checked before any input is read: anything else, an
/// empty token, a second `--fail-on`, or the flag without `--rank` is a usage error (exit 2)
/// with an empty stdout.
#[test]
fn fail_on_takes_classes_and_labels_only() {
    let patch = change("src/a.rs", &["    unsafe {}".to_owned()]);
    for (list, shown) in [
        ("unsafe", "unsafe"),
        ("Security", "Security"),
        ("security,", "\"\""),
        ("", "\"\""),
        ("security, shape", " shape"),
        ("evidence_truncated", "evidence_truncated"),
        ("x\u{202e}y", "x\\u{202e}y"),
    ] {
        let (code, stdout, stderr) = run(&["--stdin-patch", "--rank", "--fail-on", list], &patch);
        assert_eq!(
            (code, stdout.as_str(), stderr.as_str()),
            (
                Some(2),
                "",
                format!(
                    "error: --fail-on: unknown class or label: {shown} (deep-diff-forge schema signals lists them)\n"
                )
                .as_str()
            ),
            "{list:?}"
        );
    }
    for mode in [
        &["--stdin-patch", "--json"][..],
        &["--stdin-patch", "--cluster"][..],
        &["--stdin-patch", "--outline"][..],
        &["--stdin-patch"][..],
    ] {
        let mut args = mode.to_vec();
        args.extend(["--fail-on", "security"]);
        let (code, stdout, stderr) = run(&args, &patch);
        assert_eq!(
            (code, stdout.as_str(), stderr.as_str()),
            (Some(2), "", "error: --fail-on needs --rank\n"),
            "{args:?}"
        );
    }
    let (code, stdout, stderr) = run(
        &[
            "--stdin-patch",
            "--rank",
            "--fail-on",
            "shape",
            "--fail-on",
            "security",
        ],
        &patch,
    );
    assert_eq!((code, stdout.as_str()), (Some(2), ""), "{stderr}");
    assert!(
        stderr.contains("--fail-on given more than once"),
        "{stderr}"
    );
    let (code, _, stderr) = run(
        &[
            "--stdin-patch",
            "--rank",
            "--fail-on",
            "unsafe_added,security",
        ],
        &patch,
    );
    assert_eq!(code, Some(7), "the gate did not refuse: {stderr}");
    assert_eq!(
        reasons(&stderr),
        ["refused: 1 files carry a label of unsafe_added,security (--fail-on)"]
    );
}

/// A10 (M26): a C-quoted path can hold a newline, `)`, a bidi control and the text of another
/// refusal. Its path line is escaped and starts with two spaces, so stderr holds exactly one
/// line in the reason grammar, the gate's own, and no raw control.
#[test]
fn a_path_cannot_forge_a_refusal_line() {
    let quoted = r"x\nrefused: 0 files in input (--require-files)\342\200\256)";
    let patch = format!(
        "diff --git \"a/{quoted}\" \"b/{quoted}\"\nnew file mode 100644\n--- /dev/null\n+++ \"b/{quoted}\"\n@@ -0,0 +1 @@\n+a\n"
    );
    let (code, stderr) = gate(&["--stdin-patch", "--rank", "--json"], "new_file", &patch);
    assert_eq!(code, Some(7), "the gate did not refuse: {stderr}");
    assert_eq!(
        reasons(&stderr),
        ["refused: 1 files carry a label of new_file (--fail-on)"],
        "{stderr}"
    );
    assert!(
        !stderr.contains('\u{202e}') && !stderr.contains("\n)"),
        "a raw control or a broken line on stderr: {stderr:?}"
    );
    for line in stderr.lines().skip(1) {
        assert!(
            line.starts_with("  "),
            "{line:?} does not start with two spaces"
        );
    }
    assert_eq!(stderr.lines().count(), 2, "{stderr:?}");
}

/// A Git repository in `scratch` with one commit `base` and one commit `change`.
fn repository(scratch: &Scratch, base: &[(&str, &[u8])], change: &[(&str, &[u8])]) {
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .current_dir(scratch.path())
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
        assert!(out.status.success(), "git {args:?}: {out:?}");
    };
    git(&["init", "-q"]);
    for (files, message) in [(base, "base"), (change, "change")] {
        for (path, bytes) in files {
            let path = scratch.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        }
        git(&["add", "-A"]);
        git(&["commit", "-q", "-m", message]);
    }
}

/// `--git-range HEAD~1..HEAD --repo <root> --rank --json` with `--fail-on list`.
fn range_gate(root: &Path, list: &str) -> (Option<i32>, String) {
    let root = root.to_str().unwrap();
    gate(
        &[
            "--git-range",
            "HEAD~1..HEAD",
            "--repo",
            root,
            "--rank",
            "--json",
        ],
        list,
        "",
    )
}

/// The gate on a Git range: a present label refuses, an absent one passes.
#[test]
fn fail_on_gates_a_git_range() {
    let scratch = Scratch::new("fail-on-range");
    repository(
        &scratch,
        &[("src/a.rs", b"fn a() {}\n")],
        &[("src/a.rs", b"fn a() {}\nfn b() { unsafe {} }\n")],
    );
    let (code, stderr) = range_gate(scratch.path(), "unsafe_added");
    assert_eq!(code, Some(7), "the gate did not refuse: {stderr}");
    assert_eq!(
        stderr,
        "refused: 1 files carry a label of unsafe_added (--fail-on)\n  \"src/a.rs\": unsafe_added\n"
    );
    let (code, stderr) = range_gate(scratch.path(), "secret_like,shape");
    assert_eq!((code, stderr.as_str()), (Some(0), ""));
}

/// S3.20 on the gate: a source Git hides as binary with a `-diff` attribute is labelled
/// `diff_suppressed` (class security), and the gate refuses on the label and on the class.
#[test]
fn a_file_hidden_by_an_attribute_refuses_under_diff_suppressed() {
    let scratch = Scratch::new("fail-on-attribute");
    repository(
        &scratch,
        &[
            (".gitattributes", b"*.rs -diff\n"),
            ("src/m.rs", b"fn m() {}\n"),
        ],
        &[("src/m.rs", b"fn m() {}\nfn n() {}\n")],
    );
    for list in ["diff_suppressed", "security"] {
        let (code, stderr) = range_gate(scratch.path(), list);
        assert_eq!(code, Some(7), "{list}: {stderr}");
        assert_eq!(
            stderr,
            format!(
                "refused: 1 files carry a label of {list} (--fail-on)\n  \"src/m.rs\": diff_suppressed\n"
            )
        );
    }
}

/// S3.27.g on the gate: a binary too large to read again keeps its binary section and carries
/// `diff_suppressed` with cause `unread`; the gate sees that label like any other.
#[test]
fn an_unread_binary_is_seen_by_the_gate() {
    let large = |changed: bool| {
        let mut bytes = vec![0_u8; 33 << 20];
        if changed {
            bytes[1 << 20] = 7;
        }
        bytes
    };
    let scratch = Scratch::new("fail-on-unread");
    repository(
        &scratch,
        &[("a.bin", &large(false))],
        &[("a.bin", &large(true))],
    );
    let (code, stderr) = range_gate(scratch.path(), "diff_suppressed");
    assert_eq!(code, Some(7), "the gate did not refuse: {stderr}");
    assert_eq!(
        stderr,
        "refused: 1 files carry a label of diff_suppressed (--fail-on)\n  \"a.bin\": diff_suppressed\n"
    );
}
