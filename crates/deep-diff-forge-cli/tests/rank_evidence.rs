//! Code evidence in the CLI's rank and outline (plan 2026-10-30, Feature A): it ranks what the
//! code does above what the path says, it points at a hunk address and line, and it is the same
//! bytes every run.

#![expect(
    clippy::disallowed_methods,
    reason = "integration test: drives the real binary as a child process"
)]

use serde_json::Value;
use std::fmt::Write as _;
use std::io::Write as _;
use std::process::{Command, Stdio};

fn run(patch: &str, args: &[&str]) -> String {
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
        .write_all(patch.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

/// The expert circle's probe (2026-10-28): before, this hunk ranked on its path alone, with the
/// same score as any edit to `lib.rs`. Here it must outrank a large docs change too.
const PROBE: &str = "\
diff --git a/src/util.rs b/src/util.rs
--- a/src/util.rs
+++ b/src/util.rs
@@ -1,2 +1,3 @@ fn run()
 fn run() {
+    unsafe { std::process::Command::new(\"curl\").spawn().ok(); }
 }
diff --git a/README.md b/README.md
--- a/README.md
+++ b/README.md
@@ -1 +1 @@
-a
+b
";

fn large_docs() -> String {
    let body = (0..120).fold(String::new(), |mut body, i| {
        let _ = writeln!(body, "+line {i}");
        body
    });
    format!(
        "diff --git a/docs/BIG.md b/docs/BIG.md\n--- a/docs/BIG.md\n+++ b/docs/BIG.md\n@@ -1,0 +1,120 @@\n{body}"
    )
}

#[test]
fn code_evidence_outranks_path_and_size() {
    let patch = format!("{PROBE}{}", large_docs());
    let rank: Value =
        serde_json::from_str(&run(&patch, &["--stdin-patch", "--rank", "--json"])).unwrap();
    let ranked = rank["ranked"].as_array().unwrap();
    assert_eq!(ranked[0]["path"], "src/util.rs", "{rank}");
    assert_eq!(
        ranked[0]["signals"],
        serde_json::json!(["unsafe_added", "process_spawn"])
    );
    for item in ranked[0]["evidence"].as_array().unwrap() {
        assert_eq!(
            (item["side"].as_str(), item["line"].as_u64()),
            (Some("new"), Some(2))
        );
        assert_eq!(item["basis"], "lexical");
        assert_eq!(item["address"].as_str().unwrap().len(), 64);
    }
    // Files with no evidence carry no `evidence` member at all.
    assert!(ranked[1].get("evidence").is_none() && ranked[2].get("evidence").is_none());
}

/// The address an evidence item names is the hunk the outline lists, and that hunk carries the
/// same labels.
#[test]
fn evidence_addresses_resolve_to_outline_hunks() {
    let rank: Value =
        serde_json::from_str(&run(PROBE, &["--stdin-patch", "--rank", "--json"])).unwrap();
    let outline: Value =
        serde_json::from_str(&run(PROBE, &["--stdin-patch", "--outline"])).unwrap();
    let address = &rank["ranked"][0]["evidence"][0]["address"];
    let hunk = outline["files"][0]["hunks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|h| &h["address"] == address)
        .expect("the evidence address names an outline hunk");
    assert_eq!(
        hunk["evidence"],
        serde_json::json!(["unsafe_added", "process_spawn"])
    );
}

/// N2: an agent compares runs, so the same input gives the same bytes.
#[test]
fn rank_and_outline_are_byte_stable() {
    let patch = format!("{PROBE}{}", large_docs());
    for args in [
        &["--stdin-patch", "--rank", "--json"][..],
        &["--stdin-patch", "--outline"][..],
    ] {
        let first = run(&patch, args);
        for _ in 0..2 {
            assert_eq!(run(&patch, args), first, "{args:?}");
        }
    }
}

/// Evidence is capped per file in the document, while the score still counts what was found.
#[test]
fn evidence_is_capped_and_says_so() {
    let body = (0..40).fold(String::new(), |mut body, i| {
        let _ = writeln!(body, "+unsafe {{ f{i}() }}");
        body
    });
    let patch = format!("--- a/src/x.rs\n+++ b/src/x.rs\n@@ -1,0 +1,40 @@\n{body}");
    let rank: Value =
        serde_json::from_str(&run(&patch, &["--stdin-patch", "--rank", "--json"])).unwrap();
    assert_eq!(rank["ranked"][0]["evidence"].as_array().unwrap().len(), 32);
    assert_eq!(rank["ranked"][0]["evidence_truncated"], true);
}
