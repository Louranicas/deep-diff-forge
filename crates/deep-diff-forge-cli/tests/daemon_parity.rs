//! Slice B: the daemon's `diff.compute` answers as `deep-diff-forge --stdin-patch --json` does for
//! the same bytes, and `diff.rank` as `--stdin-patch --rank --json` does: the same document as a
//! JSON value (seal included; key order is not part of the contract), the same refusal text, and
//! the same parse-failure text, with the registry's codes on both surfaces.
#![expect(
    clippy::disallowed_methods,
    reason = "integration test: drives real processes, the environment and wall-clock time"
)]

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

fn cli(patch: &str, mode: &[&str], flags: &[&str]) -> (i32, String, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .args(["--stdin-patch", "--json"])
        .args(mode)
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
    method: &str,
    patch: &str,
    files: bool,
    hunks: bool,
) -> Result<serde_json::Value, deep_diff_forge_daemon::RpcError> {
    let line = serde_json::json!({
        "method": method,
        "params": {"patch": patch, "require_files": files, "require_hunks": hunks},
    })
    .to_string();
    dispatch(&parse_request(&line)?, &mut Engine::new())
}

#[test]
fn diff_compute_matches_the_cli_document_refusals_and_failures() {
    parity("diff.compute", &[]);
}

/// `diff.rank` is the CLI's sealed `rank.v0` document; `diff.plan`'s bare array could not say
/// which bytes it ranked, and MCP's `rank_patch` returned it.
#[test]
fn diff_rank_matches_the_cli_rank_document_refusals_and_failures() {
    parity("diff.rank", &["--rank"]);
}

fn parity(method: &str, mode: &[&str]) {
    let guards: [(&[&str], bool, bool); 4] = [
        (&[], false, false),
        (&["--require-files"], true, false),
        (&["--require-hunks"], false, true),
        (&["--require-files", "--require-hunks"], true, true),
    ];
    let mut seen = [0; 3];
    for patch in FIXTURES {
        for (flags, files, hunks) in guards {
            let (code, stdout, stderr) = cli(patch, mode, flags);
            let rpc = daemon(method, patch, files, hunks);
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

/// The CLI's `--outline` and `--hunks` print what the daemon's `review.outline` and
/// `review.hunks` return for the same bytes and the same page, cursor pages included.
#[test]
fn outline_and_hunks_match_the_daemon_page_for_page() {
    let patch = FIXTURES[1];
    let call = |method: &str, params: serde_json::Value| {
        let mut params = params;
        params["patch"] = serde_json::json!(patch);
        let line = serde_json::json!({"method": method, "params": params}).to_string();
        dispatch(&parse_request(&line).unwrap(), &mut Engine::new()).unwrap()
    };
    let cli_view = |flags: &[&str]| {
        let (code, stdout, stderr) = cli(patch, &[], flags);
        assert_eq!(code, 0, "{flags:?}: {stderr}");
        serde_json::from_str::<serde_json::Value>(&stdout).unwrap()
    };
    let outline = cli_view(&["--outline", "--max-bytes", "1024"]);
    assert_eq!(
        outline,
        call("review.outline", serde_json::json!({"max_bytes": 1024}))
    );
    let cursor = outline["next_cursor"].as_str().unwrap().to_owned();
    assert_eq!(
        cli_view(&["--outline", "--max-bytes", "1024", "--cursor", &cursor]),
        call(
            "review.outline",
            serde_json::json!({"max_bytes": 1024, "cursor": cursor})
        )
    );
    let address = outline["files"][0]["hunks"][0]["address"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        cli_view(&["--hunks", "--address", &address]),
        call("review.hunks", serde_json::json!({"addresses": [address]}))
    );
    assert_eq!(
        cli_view(&["--hunks", "--path", "README.md"]),
        call("review.hunks", serde_json::json!({"paths": ["README.md"]}))
    );
    let (code, stdout, stderr) = cli(patch, &[], &["--hunks", "--address", "0", "--path", "x"]);
    assert_eq!((code, stdout.as_str()), (exit::USAGE, ""), "{stderr}");
}

/// Bug hunt 2026-10-28, H5-02: the CLI printed views pretty but budgeted them compact, so
/// `--max-bytes 4096` printed 4,688 bytes. A view prints as the daemon's one-line page, within
/// `--max-bytes` plus its line feed.
#[test]
fn printed_views_stay_within_max_bytes() {
    use std::fmt::Write as _;
    let mut patch = String::from("--- a/b.rs\n+++ b/b.rs\n@@ -1,300 +1,300 @@\n");
    for i in 0..300 {
        let _ = write!(patch, "-{}{i}\n+{}x{i}\n", "\\".repeat(60), "\\".repeat(60));
    }
    for flags in [
        ["--hunks", "--max-bytes", "4096"],
        ["--outline", "--max-bytes", "1024"],
    ] {
        let (code, stdout, stderr) = cli(&patch, &[], &flags);
        assert_eq!(code, 0, "{flags:?}: {stderr}");
        let budget: usize = flags[2].parse().unwrap();
        assert!(stdout.len() <= budget + 1, "{flags:?}: {}", stdout.len());
        assert_eq!(stdout.lines().count(), 1, "{flags:?}");
    }
}
