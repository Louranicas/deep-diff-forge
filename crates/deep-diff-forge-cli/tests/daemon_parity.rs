//! Slice B: the daemon's `diff.compute` answers as `deep-diff-forge --stdin-patch --json` does for
//! the same bytes: the same review document as a JSON value (seal included; key order is not part
//! of the contract), the same refusal text, and the same parse-failure text, with the registry's
//! codes on both surfaces.
use deep_diff_forge_core::exit;
use deep_diff_forge_daemon::{
    Engine, INPUT_CONTRACT_REFUSED, PATCH_PARSE_FAILED, dispatch, parse_request,
};
use std::io::Write as _;
use std::process::{Command, Stdio};

const FIXTURES: &[&str] = &[
    "--- a/x\n+++ b/x\n@@ -1,1 +1,1 @@\n-old\n+new\n",
    "diff --git a/src/lib.rs b/src/lib.rs\nindex 1111111..2222222 100644\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,4 @@\n fn a() {}\n-fn b() {}\n+fn b() -> u8 { 1 }\n+fn c() {}\n fn d() {}\ndiff --git a/README.md b/README.md\n--- a/README.md\n+++ b/README.md\n@@ -1 +1 @@\n-x\n+y\n",
    "diff --git a/old.rs b/new.rs\nsimilarity index 100%\nrename from old.rs\nrename to new.rs\n",
    "--- a/x\n+++ b/x\n@@ -1,1 +1,1 @@\n same\n",
    "",
    "--- a/x\n+++ b/x\n+stray\n",
];

fn cli(patch: &str, flags: &[&str]) -> (i32, String, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .args(["--stdin-patch", "--json"])
        .args(flags)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(patch.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    (
        out.status.code().unwrap(),
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

fn daemon(
    patch: &str,
    files: bool,
    hunks: bool,
) -> Result<serde_json::Value, deep_diff_forge_daemon::RpcError> {
    let line = serde_json::json!({
        "method": "diff.compute",
        "params": {"patch": patch, "require_files": files, "require_hunks": hunks},
    })
    .to_string();
    dispatch(&parse_request(&line)?, &mut Engine::new())
}

#[test]
fn diff_compute_matches_the_cli_document_refusals_and_failures() {
    let guards: [(&[&str], bool, bool); 4] = [
        (&[], false, false),
        (&["--require-files"], true, false),
        (&["--require-hunks"], false, true),
        (&["--require-files", "--require-hunks"], true, true),
    ];
    let mut seen = [0; 3];
    for patch in FIXTURES {
        for (flags, files, hunks) in guards {
            let (code, stdout, stderr) = cli(patch, flags);
            let rpc = daemon(patch, files, hunks);
            match code {
                exit::SUCCESS => {
                    let document: serde_json::Value = serde_json::from_str(&stdout).unwrap();
                    assert_eq!(rpc.as_ref().ok(), Some(&document), "{patch:?} {flags:?}");
                    seen[0] += 1;
                }
                exit::INPUT_CONTRACT_REFUSED => {
                    let err = rpc.unwrap_err();
                    // The daemon's code is the CLI's exit code, numerically (council sb1).
                    assert_eq!(err.code, i64::from(exit::INPUT_CONTRACT_REFUSED));
                    assert_eq!(
                        (err.code, format!("{}\n", err.message)),
                        (INPUT_CONTRACT_REFUSED, stderr),
                        "{patch:?}"
                    );
                    seen[1] += 1;
                }
                exit::INVALID_INPUT => {
                    let err = rpc.unwrap_err();
                    assert_eq!(err.code, PATCH_PARSE_FAILED, "{patch:?}");
                    assert_eq!(err.code, i64::from(exit::INVALID_INPUT));
                    assert_eq!(
                        format!("error: patch parse failed: {}\n", err.message),
                        stderr,
                        "{patch:?}"
                    );
                    seen[2] += 1;
                }
                other => panic!("unexpected exit {other} for {patch:?} {flags:?}: {stderr}"),
            }
        }
    }
    // Every outcome is exercised, so parity is not vacuous.
    assert!(seen.iter().all(|&n| n > 0), "{seen:?}");
}
