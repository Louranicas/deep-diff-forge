//! CLI contract tests for the orchestrator-facing "sealed observation" surface:
//! every machine-readable document carries `input_sha256` (SHA-256 of the exact
//! bytes read) and `tool` (name + version), `--require-files` refuses a
//! zero-file patch with exit 7, and `--rank --json --require-files` is
//! byte-for-byte deterministic across runs.

use std::io::Write as _;
use std::process::{Command, Stdio};

fn run(args: &[&str], stdin: &[u8]) -> (i32, Vec<u8>, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn binary");
    child
        .stdin
        .take()
        .expect("stdin handle")
        .write_all(stdin)
        .expect("write stdin");
    let out = child.wait_with_output().expect("wait for binary");
    (
        out.status.code().unwrap_or(-1),
        out.stdout,
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn run_text(args: &[&str], stdin: &str) -> (i32, String, String) {
    let (code, stdout, stderr) = run(args, stdin.as_bytes());
    (code, String::from_utf8_lossy(&stdout).into_owned(), stderr)
}

/// Extract the `input_sha256` value from a document (first occurrence).
fn input_sha256(doc: &str) -> String {
    let key = "\"input_sha256\": \"";
    let start = doc.find(key).expect("input_sha256 present") + key.len();
    doc[start..start + 64].to_string()
}

/// The exact seal members the CLI must emit for `input`.
fn expected_seal(input: &[u8]) -> String {
    deep_diff_forge_core::InputSeal::of(input, env!("CARGO_PKG_VERSION")).json_members()
}

const PATCH: &str = "--- a/x\n+++ b/x\n@@ -1,1 +1,1 @@\n-old\n+new\n";
// `printf -- '--- a/x\n+++ b/x\n@@ -1,1 +1,1 @@\n-old\n+new\n' | sha256sum`
const PATCH_SHA256: &str = "ddae457b65e3081c49d3f6983380c9580da19064d38ec2ad79ab5f68a495e820";

fn fixture(name: &str) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/patch")
        .join(name);
    std::fs::read(&path).expect("read fixture")
}

// ---- input sealing -------------------------------------------------------

#[test]
fn review_json_carries_seal_matching_independent_sha256sum() {
    let (code, stdout, _) = run_text(&["--stdin-patch", "--json"], PATCH);
    assert_eq!(code, 0);
    assert!(stdout.contains("\"schema\": \"deep-diff-forge.review.v0\""));
    assert_eq!(input_sha256(&stdout), PATCH_SHA256);
    assert!(stdout.contains(&format!(
        "\"tool\": {{\"name\": \"deep-diff-forge\", \"version\": \"{}\"}}",
        env!("CARGO_PKG_VERSION")
    )));
}

#[test]
fn review_json_seal_members_sit_directly_after_schema() {
    let (_, stdout, _) = run_text(&["--stdin-patch", "--json"], PATCH);
    let head = format!(
        "{{\n  \"schema\": \"deep-diff-forge.review.v0\",\n{}  \"files\": [",
        expected_seal(PATCH.as_bytes())
    );
    assert!(stdout.starts_with(&head), "got:\n{stdout}");
}

#[test]
fn rank_json_carries_seal() {
    let (code, stdout, _) = run_text(&["--stdin-patch", "--rank", "--json"], PATCH);
    assert_eq!(code, 0);
    let head = format!(
        "{{\n  \"schema\": \"deep-diff-forge.rank.v0\",\n{}  \"ranked\": [",
        expected_seal(PATCH.as_bytes())
    );
    assert!(stdout.starts_with(&head), "got:\n{stdout}");
}

#[test]
fn cluster_json_carries_seal() {
    let (code, stdout, _) = run_text(
        &[
            "--stdin-patch",
            "--cluster",
            "--parallel",
            "serial",
            "--json",
        ],
        PATCH,
    );
    assert_eq!(code, 0);
    let head = format!(
        "{{\n  \"schema\": \"deep-diff-forge.cluster.v0\",\n{}  \"receipt\": {{",
        expected_seal(PATCH.as_bytes())
    );
    assert!(stdout.starts_with(&head), "got:\n{stdout}");
}

#[test]
fn semantic_json_seals_the_source_file_bytes() {
    let src = "fn alpha() {}\n";
    let path = std::env::temp_dir().join(format!("ddf-seal-{}.rs", std::process::id()));
    std::fs::write(&path, src).expect("write temp file");
    let (code, stdout, _) = run_text(&["semantic", path.to_str().unwrap(), "--json"], "");
    let _ = std::fs::remove_file(&path);
    assert_eq!(code, 0);
    let head = format!(
        "{{\n  \"schema\": \"deep-diff-forge.semantic.v0\",\n{}  \"path\": ",
        expected_seal(src.as_bytes())
    );
    assert!(stdout.starts_with(&head), "got:\n{stdout}");
}

#[test]
fn seal_hashes_raw_bytes_before_any_normalisation() {
    // CRLF vs LF parse to the same model but are different bytes: the seal
    // must distinguish them, because the gate verifies against the raw input.
    let crlf = PATCH.replace('\n', "\r\n");
    let (_, lf_out, _) = run_text(&["--stdin-patch", "--rank", "--json"], PATCH);
    let (_, crlf_out, _) = run_text(&["--stdin-patch", "--rank", "--json"], &crlf);
    assert_ne!(input_sha256(&lf_out), input_sha256(&crlf_out));
    assert_eq!(
        input_sha256(&crlf_out),
        deep_diff_forge_core::sha256_hex(crlf.as_bytes())
    );
}

#[test]
fn empty_input_seal_is_the_empty_string_digest() {
    let (code, stdout, _) = run_text(&["--stdin-patch", "--json"], "");
    assert_eq!(code, 0);
    assert_eq!(
        input_sha256(&stdout),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}

#[test]
fn human_and_jsonl_modes_are_not_sealed() {
    let (_, human, _) = run_text(&["--stdin-patch", "--rank"], PATCH);
    assert!(!human.contains("input_sha256"));
    let (_, jsonl, _) = run_text(&["--stdin-patch", "--jsonl"], PATCH);
    assert!(!jsonl.contains("input_sha256"));
}

// ---- --require-files -----------------------------------------------------

#[test]
fn require_files_refuses_empty_input_with_exit_seven_and_empty_stdout() {
    let (code, stdout, stderr) = run_text(&["--stdin-patch", "--json", "--require-files"], "");
    assert_eq!(code, 7);
    assert!(stdout.is_empty(), "stdout must stay empty on refusal");
    assert_eq!(stderr, "refused: 0 files in input (--require-files)\n");
}

#[test]
fn require_files_refuses_zero_file_patch_in_every_document_mode() {
    // Well-formed, parseable, but describes no file.
    let no_files = "just some prose that is not a diff\n";
    for args in [
        vec!["--stdin-patch", "--require-files"],
        vec!["--stdin-patch", "--json", "--require-files"],
        vec!["--stdin-patch", "--rank", "--json", "--require-files"],
        vec!["--stdin-patch", "--cluster", "--json", "--require-files"],
        vec!["--stdin-patch", "--jsonl", "--require-files"],
    ] {
        let (code, stdout, stderr) = run_text(&args, no_files);
        assert_eq!(code, 7, "args={args:?}");
        assert!(stdout.is_empty(), "args={args:?}");
        assert!(stderr.starts_with("refused: 0 files"), "args={args:?}");
    }
}

#[test]
fn require_files_passes_a_non_empty_patch_unchanged() {
    let (code, with, _) = run_text(
        &["--stdin-patch", "--rank", "--json", "--require-files"],
        PATCH,
    );
    let (_, without, _) = run_text(&["--stdin-patch", "--rank", "--json"], PATCH);
    assert_eq!(code, 0);
    assert_eq!(with, without);
}

#[test]
fn require_files_does_not_mask_a_parse_failure() {
    let bad = "--- a/x\n+++ b/x\n+stray addition with no hunk\n";
    let (code, stdout, stderr) = run_text(&["--stdin-patch", "--json", "--require-files"], bad);
    assert_eq!(code, 4);
    assert_eq!(stdout, "", "stdout must stay empty on a parse failure");
    assert!(stderr.contains("patch parse failed"));
}

#[test]
fn without_require_files_empty_input_is_still_exit_zero() {
    let (code, stdout, _) = run_text(&["--stdin-patch", "--rank", "--json"], "");
    assert_eq!(code, 0);
    assert!(stdout.contains("\"ranked\": []"));
}

#[test]
fn help_documents_require_files() {
    let (code, stdout, _) = run_text(&["--help"], "");
    assert_eq!(code, 0);
    assert!(stdout.contains("--require-files"));
}

// ---- --require-hunks -----------------------------------------------------

/// One file, zero hunks: `--require-files` alone is satisfied by this.
const HEADER_ONLY: &str = "diff --git a/x b/x\n";
const RENAME_ONLY: &str =
    "diff --git a/old.rs b/new.rs\nsimilarity index 100%\nrename from old.rs\nrename to new.rs\n";
/// The recommended gate invocation from the README.
const GATE: [&str; 5] = [
    "--stdin-patch",
    "--rank",
    "--json",
    "--require-files",
    "--require-hunks",
];

#[test]
fn require_hunks_refuses_header_only_diff_that_require_files_passes() {
    // (1) both flags → refused with exit 7 and an empty stdout.
    let (code, stdout, stderr) = run_text(&GATE, HEADER_ONLY);
    assert_eq!(code, 7);
    assert!(stdout.is_empty(), "stdout must stay empty on refusal");
    assert_eq!(stderr, "refused: 0 hunks in input (--require-hunks)\n");

    // (2) --require-files alone still passes it: one file is one file.
    let (code, stdout, stderr) = run_text(
        &["--stdin-patch", "--rank", "--json", "--require-files"],
        HEADER_ONLY,
    );
    assert_eq!(code, 0);
    assert!(stdout.contains("\"schema\": \"deep-diff-forge.rank.v0\""));
    assert!(stdout.contains("\"path\": \"x\""));
    assert!(stderr.is_empty());
}

#[test]
fn require_hunks_passes_a_real_diff_sealed_and_unchanged() {
    // (3) a diff with a real hunk clears both gates and is sealed.
    let (code, with, stderr) = run_text(&GATE, PATCH);
    assert_eq!(code, 0);
    assert!(stderr.is_empty());
    let head = format!(
        "{{\n  \"schema\": \"deep-diff-forge.rank.v0\",\n{}  \"ranked\": [",
        expected_seal(PATCH.as_bytes())
    );
    assert!(with.starts_with(&head), "got:\n{with}");
    assert_eq!(input_sha256(&with), PATCH_SHA256);
    let (_, without, _) = run_text(&["--stdin-patch", "--rank", "--json"], PATCH);
    assert_eq!(
        with, without,
        "the guards must not alter a passing document"
    );
}

#[test]
fn require_hunks_refuses_rename_only_diff() {
    // (4) a pure rename parses to one file, zero hunks, zero changed lines.
    let (code, stdout, stderr) = run_text(&GATE, RENAME_ONLY);
    assert_eq!(code, 7);
    assert!(stdout.is_empty());
    assert_eq!(stderr, "refused: 0 hunks in input (--require-hunks)\n");
    // ... and without the flag it is still exit 0 (default unchanged).
    let (code, stdout, _) = run_text(&["--stdin-patch", "--rank", "--json"], RENAME_ONLY);
    assert_eq!(code, 0);
    assert!(stdout.contains("\"status\": \"renamed\""));
}

#[test]
fn require_hunks_refuses_context_only_hunk() {
    // A hunk exists but adds and removes nothing: still nothing to review.
    let context_only = "--- a/x\n+++ b/x\n@@ -1,1 +1,1 @@\n same\n";
    let (code, stdout, stderr) = run_text(
        &["--stdin-patch", "--json", "--require-hunks"],
        context_only,
    );
    assert_eq!(code, 7);
    assert!(stdout.is_empty());
    assert_eq!(stderr, "refused: 0 hunks in input (--require-hunks)\n");
}

#[test]
fn require_hunks_refuses_in_every_document_mode() {
    for args in [
        vec!["--stdin-patch", "--require-hunks"],
        vec!["--stdin-patch", "--json", "--require-hunks"],
        vec!["--stdin-patch", "--rank", "--json", "--require-hunks"],
        vec!["--stdin-patch", "--cluster", "--json", "--require-hunks"],
        vec!["--stdin-patch", "--jsonl", "--require-hunks"],
        vec!["--stdin-patch", "--layout", "inline", "--require-hunks"],
    ] {
        let (code, stdout, stderr) = run_text(&args, HEADER_ONLY);
        assert_eq!(code, 7, "args={args:?}");
        assert!(stdout.is_empty(), "args={args:?}");
        assert_eq!(
            stderr, "refused: 0 hunks in input (--require-hunks)\n",
            "args={args:?}"
        );
    }
}

#[test]
fn both_guards_on_empty_input_report_the_files_guard_first() {
    let (code, stdout, stderr) = run_text(&GATE, "");
    assert_eq!(code, 7);
    assert!(stdout.is_empty());
    assert_eq!(stderr, "refused: 0 files in input (--require-files)\n");
}

#[test]
fn require_hunks_does_not_mask_a_parse_failure() {
    let bad = "--- a/x\n+++ b/x\n+stray addition with no hunk\n";
    let (code, stdout, stderr) = run_text(&GATE, bad);
    assert_eq!(code, 4);
    assert_eq!(stdout, "");
    assert!(stderr.contains("patch parse failed"));
}

#[test]
fn help_documents_require_hunks() {
    let (code, stdout, _) = run_text(&["--help"], "");
    assert_eq!(code, 0);
    assert!(stdout.contains("--require-hunks"));
}

// ---- determinism ---------------------------------------------------------

#[test]
fn rank_json_require_files_is_byte_identical_across_runs() {
    let input = fixture("basic.patch");
    let args = ["--stdin-patch", "--rank", "--json", "--require-files"];
    let (code1, out1, err1) = run(&args, &input);
    let (code2, out2, err2) = run(&args, &input);
    assert_eq!((code1, code2), (0, 0));
    assert!(err1.is_empty() && err2.is_empty());
    assert_eq!(out1, out2, "stdout must be byte-identical across runs");

    let doc1 = String::from_utf8(out1).expect("utf-8");
    let doc2 = String::from_utf8(out2).expect("utf-8");
    assert_eq!(input_sha256(&doc1), input_sha256(&doc2));
    // ... and it is the digest of the fixture bytes themselves
    // (`sha256sum fixtures/patch/basic.patch`).
    assert_eq!(
        input_sha256(&doc1),
        "9317219840467a00fbb7bc3ff905b0ae26a999582c12c8d02f49d380b398bbb3"
    );
    assert!(doc1.contains("\"path\": \"src/lib.rs\""));
}
