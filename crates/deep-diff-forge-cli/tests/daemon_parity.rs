//! Slice B: the daemon's `diff.compute` answers as `deep-diff-forge --stdin-patch --json` does for
//! the same bytes, and `diff.rank` as `--stdin-patch --rank --json` does: the same document as a
//! JSON value (seal included), the same refusal text, and the same parse-failure text, with the
//! registry's codes on both surfaces. Since S3.15 the rank document is also the same bytes.
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
    // Code evidence (unsafe, a process spawn, a removed test) in more than one hunk and file.
    "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 +1,3 @@ fn main()\n fn main() {\n+    unsafe { std::process::Command::new(\"curl\"); }\n }\n@@ -9,2 +10,1 @@\n-#[test]\n fn t() {}\ndiff --git a/x.py b/x.py\n--- a/x.py\n+++ b/x.py\n@@ -1 +1 @@\n-a = 1\n+a = eval(s)  # noqa\n",
];

fn cli(patch: &str, mode: &[&str], flags: &[&str]) -> (i32, String, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .args(["--stdin-patch", "--json"])
        // The daemon answers as the command line does with --allow-empty, until S3.15 decides whether the refusal moves into the shared builder.
        .arg("--allow-empty")
        .args(mode)
        .args(flags)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // A refusal of the arguments (`--address` with `--path`) exits before stdin is read, so the
    // write can race the exit and meet a closed pipe; the exit code and stderr are still checked.
    if let Err(error) = child.stdin.take().unwrap().write_all(patch.as_bytes()) {
        assert_eq!(error.kind(), std::io::ErrorKind::BrokenPipe, "{error}");
    }
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
    dispatch(&parse_request(&line)?, &Engine::new())
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

/// S3.15: the rank document has one byte form, the command line's (what forge-warden seals), and
/// every surface carries those bytes: a surface that pretty-prints, compacts or reorders on its
/// own is named here.
#[test]
fn rank_bytes_are_one_document_on_every_surface() {
    let mut compared = 0;
    for patch in FIXTURES {
        let (code, stdout, stderr) = cli(patch, &["--rank"], &[]);
        if code != exit::SUCCESS {
            assert_eq!(code, exit::INVALID_INPUT, "{patch:?}: {stderr}");
            continue;
        }
        let review = deep_diff_forge_patch::SealedReview::new(
            patch.as_bytes(),
            deep_diff_forge_patch::InputGuards {
                files: false,
                hunks: false,
            },
            env!("CARGO_PKG_VERSION"),
        )
        .unwrap();
        let ranked = deep_diff_forge_graph::rank(review.files());
        let rendered = deep_diff_forge_graph::RankDocument {
            seal: review.seal(),
            ranked: &ranked,
        }
        .to_json();
        let surfaces = [("the one renderer", rendered)];
        for (surface, bytes) in surfaces {
            assert!(
                bytes == stdout,
                "{surface} differs from the command line's bytes for {patch:?}:\n{bytes}\n{stdout}"
            );
            compared += 1;
        }
    }
    assert!(compared >= 5, "{compared}");
}

/// A list cut short is the same list on both surfaces, and it is the one that keeps what
/// release 0.8.3 listed: an item of a newer signal on line 1 above 32 spawns, and all 32
/// spawns are listed (review of the released line's fix, VLF3-L-1; MCP's `rank_patch`
/// returns the daemon's document).
#[test]
fn a_list_cut_short_is_the_same_list_from_the_cli_and_the_daemon() {
    use std::fmt::Write as _;
    let mut patch = String::from(
        "diff --git a/src/x.rs b/src/x.rs\nnew file mode 100644\n--- /dev/null\n+++ b/src/x.rs\n@@ -0,0 +1,33 @@\n+x.unwrap();\n",
    );
    for i in 0..32 {
        let _ = writeln!(patch, "+std::process::Command::new(\"p{i}\");");
    }
    let (code, stdout, stderr) = cli(&patch, &["--rank"], &[]);
    assert_eq!(code, exit::SUCCESS, "{stderr}");
    let document: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(
        daemon("diff.rank", &patch, false, false).ok(),
        Some(document.clone())
    );
    let file = &document["ranked"][0];
    let listed: Vec<(&str, u64)> = file["evidence"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| {
            (
                item["signal"].as_str().unwrap(),
                item["line"].as_u64().unwrap(),
            )
        })
        .collect();
    let spawns: Vec<(&str, u64)> = (2..=33).map(|line| ("process_spawn", line)).collect();
    assert_eq!(listed, spawns);
    assert_eq!(file["evidence_truncated"], true);
    assert!(
        file["signals"]
            .as_array()
            .unwrap()
            .iter()
            .any(|signal| signal == "panic_path_added"),
        "{file}"
    );
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
        dispatch(&parse_request(&line).unwrap(), &Engine::new()).unwrap()
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
