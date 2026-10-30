//! `findings verify` and `review --findings` (plan 2026-10-30, Feature B): the CLI's contract
//! for agent findings, exit codes and limits included.

#![expect(
    clippy::disallowed_methods,
    reason = "integration test: drives the real binary as a child process"
)]

use serde_json::{Value, json};
use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};

const PATCH: &str = "--- a/x\n+++ b/x\n@@ -1,1 +1,1 @@\n-a\n+b\n";
const MAX: usize = 8 * 1024 * 1024;

/// A findings file holding `bytes`, removed when the test ends.
struct Findings(PathBuf);

impl Findings {
    fn new(name: &str, bytes: &[u8]) -> Self {
        let path =
            std::env::temp_dir().join(format!("ddf-findings-{name}-{}.jsonl", std::process::id()));
        std::fs::write(&path, bytes).unwrap();
        Self(path)
    }

    fn path(&self) -> &str {
        self.0.to_str().unwrap()
    }
}

impl Drop for Findings {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn run(args: &[&str], stdin: &str) -> (i32, String, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn line(finding: &Value) -> String {
    format!("{finding}\n")
}

fn good() -> Value {
    json!({"agent": "codex", "severity": "high", "title": "t", "body": "b", "path": "x", "line": 1})
}

fn bad() -> Value {
    json!({"agent": "codex", "severity": "low", "title": "u", "body": "c", "path": "nope"})
}

#[test]
fn the_verdict_is_sealed_to_the_patch_and_the_file_bytes() {
    let text = format!("{}\n{}", line(&good()), line(&bad()));
    let file = Findings::new("seal", text.as_bytes());
    let (code, stdout, stderr) = run(&["findings", "verify", "--findings", file.path()], PATCH);
    assert_eq!(code, 0, "{stderr}");
    let doc: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(doc["schema"], "deep-diff-forge.findings.v0");
    assert_eq!(
        doc["findings_sha256"],
        deep_diff_forge_core::sha256_hex(text.as_bytes())
    );
    assert_eq!(
        doc["input_sha256"],
        deep_diff_forge_core::sha256_hex(PATCH.as_bytes())
    );
    assert_eq!(
        doc["counts"],
        json!({"accepted": 1, "refused": 1, "over_limit": 0})
    );
    assert_eq!(doc["accepted"][0]["line"]["text"], "b");
    // The blank line between them is skipped, so the second finding is index 1.
    assert_eq!(doc["refused"][0]["index"], 1);
}

#[test]
fn require_all_fails_on_any_refusal_or_overflow_and_passes_otherwise() {
    let all_good = Findings::new("good", line(&good()).as_bytes());
    let mixed = Findings::new(
        "mixed",
        format!("{}{}", line(&good()), line(&bad())).as_bytes(),
    );
    let mut many = String::new();
    for i in 0..=deep_diff_forge_agent::findings::MAX_FINDINGS {
        let mut f = good();
        f["title"] = json!(format!("t{i}"));
        many.push_str(&line(&f));
    }
    let over = Findings::new("over", many.as_bytes());
    for (file, without, with) in [(&all_good, 0, 0), (&mixed, 0, 7), (&over, 0, 7)] {
        let args = ["findings", "verify", "--findings", file.path()];
        assert_eq!(run(&args, PATCH).0, without, "{}", file.path());
        let strict = [&args[..], &["--require-all"]].concat();
        assert_eq!(run(&strict, PATCH).0, with, "{}", file.path());
    }
    let (_, stdout, _) = run(&["findings", "verify", "--findings", over.path()], PATCH);
    let doc: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(doc["counts"]["over_limit"], 1);
}

#[test]
fn the_file_limit_is_exactly_8_mib_of_utf8() {
    // Blank lines are skipped, so a file of newlines is only a size test.
    let at = Findings::new("at", &vec![b'\n'; MAX]);
    assert_eq!(
        run(&["findings", "verify", "--findings", at.path()], PATCH).0,
        0
    );
    let over = Findings::new("above", &vec![b'\n'; MAX + 1]);
    let (code, _, stderr) = run(&["findings", "verify", "--findings", over.path()], PATCH);
    assert_eq!(code, 4);
    assert!(stderr.contains("over the 8388608-byte limit"), "{stderr}");
    let latin1 = Findings::new("latin1", b"{\"agent\": \"\xe9\"}\n");
    let (code, _, stderr) = run(&["findings", "verify", "--findings", latin1.path()], PATCH);
    assert_eq!(code, 4);
    assert!(stderr.contains("not UTF-8"), "{stderr}");
}

#[test]
fn usage_read_and_patch_errors_have_their_codes() {
    let file = Findings::new("errors", line(&good()).as_bytes());
    let missing = std::env::temp_dir().join("ddf-findings-missing-nope.jsonl");
    assert_eq!(
        run(
            &[
                "findings",
                "verify",
                "--findings",
                missing.to_str().unwrap()
            ],
            PATCH
        )
        .0,
        3
    );
    assert_eq!(run(&["findings", "verify"], PATCH).0, 2);
    assert_eq!(
        run(&["findings", "check", "--findings", file.path()], PATCH).0,
        2
    );
    assert_eq!(run(&["findings"], PATCH).0, 2);
    assert_eq!(
        run(
            &["findings", "verify", "--findings", file.path()],
            "--- a/x\n+++ b/x\n+stray\n"
        )
        .0,
        4
    );
    assert_eq!(
        run(
            &[
                "findings",
                "verify",
                "--findings",
                file.path(),
                "--require-files"
            ],
            ""
        )
        .0,
        7
    );
}

#[test]
fn the_cockpit_shows_accepted_findings_as_agent_notes() {
    let file = Findings::new(
        "cockpit",
        format!("{}{}", line(&good()), line(&bad())).as_bytes(),
    );
    let (code, stdout, stderr) = run(
        &[
            "review",
            "--probe",
            "--cols",
            "120",
            "--rows",
            "30",
            "--cmd",
            "notes",
            "--findings",
            file.path(),
        ],
        PATCH,
    );
    assert_eq!(code, 0, "{stderr}");
    assert!(
        stderr.contains("findings: 1 accepted, 1 refused, 0 over the limit"),
        "{stderr}"
    );
    assert!(stdout.contains("[high] t (new line 1) — b"), "{stdout}");
    let (code, _, _) = run(
        &["review", "--probe", "--findings", "/nonexistent/f.jsonl"],
        PATCH,
    );
    assert_eq!(code, 3);
}
