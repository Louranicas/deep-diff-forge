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

/// A panic path under a test attribute is still reported, at the same score, with `test_likely`
/// beside it; an item without the hint has no such member.
#[test]
fn a_panic_path_under_a_test_attribute_says_so() {
    let patch = "--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,1 +1,8 @@\n x\n+#[test]\n+fn a() {\n+    f().unwrap();\n+}\n+fn real() {\n+    g().unwrap();\n+}\n";
    let rank: Value =
        serde_json::from_str(&run(patch, &["--stdin-patch", "--rank", "--json"])).unwrap();
    let items = rank["ranked"][0]["evidence"].as_array().unwrap();
    assert_eq!(items.len(), 2, "{rank}");
    assert_eq!(
        (
            items[0]["signal"].as_str(),
            items[0]["line"].as_u64(),
            items[0]["test_likely"].as_bool()
        ),
        (Some("panic_path_added"), Some(4), Some(true))
    );
    assert_eq!(items[1]["line"], 7);
    assert!(items[1].get("test_likely").is_none(), "{rank}");
    assert_eq!(
        rank["ranked"][0]["signals"],
        serde_json::json!(["panic_path_added"])
    );
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
/// The printed list at the bound holds what 0.8.3 printed: a fetch on line 1 that only the
/// second reading of Rust finds (it stands behind a nested block comment), then 32 process
/// spawns. The published v0.8.3 binary lists the spawns on lines 2 to 33 and scores 8. The list
/// here is the same 32 items; the fetch is in the signals and the score, and
/// `evidence_truncated` says the list is not all there is.
#[test]
fn the_capped_list_holds_the_items_0_8_3_listed() {
    let spawns = (1..=32).fold(String::new(), |mut body, i| {
        let _ = writeln!(body, "+std::process::Command::new(\"sh\"); // {i:02}");
        body
    });
    let patch = format!(
        "diff --git a/src/cap.rs b/src/cap.rs\nnew file mode 100644\n--- /dev/null\n+++ b/src/cap.rs\n\
         @@ -0,0 +1,33 @@\n+/* /* */ // */ let _ = reqwest::blocking::get(u);\n{spawns}"
    );
    let rank: Value =
        serde_json::from_str(&run(&patch, &["--stdin-patch", "--rank", "--json"])).unwrap();
    let file = &rank["ranked"][0];
    let listed: Vec<(&str, u64)> = file["evidence"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| (e["signal"].as_str().unwrap(), e["line"].as_u64().unwrap()))
        .collect();
    let spawns: Vec<(&str, u64)> = (2..=33).map(|line| ("process_spawn", line)).collect();
    assert_eq!(listed, spawns);
    assert_eq!(file["evidence_truncated"], true);
    assert_eq!(
        file["signals"],
        serde_json::json!(["new_file", "process_spawn", "network_fetch"])
    );
    assert_eq!(file["score"], 11);
}

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

/// D1 (0.11.0), the coordinator's probes after F2b: since OD-E `#[allow(` in a comment or a
/// string is no suppression, so the reader must know where a comment or a string ends. A real
/// `#[allow(dead_code)]` on the line after each of these still raises `check_suppressed` on its
/// own line; and an `#[allow(` on a line inside a raw string over lines still raises it (a reader
/// of one line cannot tell, and it keeps the label: conservative, stated).
#[test]
fn a_real_allowance_after_quotes_lifetimes_and_closed_comments_still_raises() {
    let cases: [(&str, &[&str]); 8] = [
        ("a quote character", &["const Q: char = '\"';"]),
        ("a quote byte", &["const Q: u8 = b'\"';"]),
        ("a lifetime", &["fn f<'a>(x: &'a str) -> &'a str { x }"]),
        (
            "a nested block comment that closed",
            &["/* outer /* inner */ still */"],
        ),
        (
            "a raw string that closed",
            &["const R: &str = r#\"a \" b\"#;"],
        ),
        ("an escaped quote", &["const S: &str = \"a \\\" b\";"]),
        ("a macro body", &["macro_rules! m { () => {"]),
        ("a raw string over lines", &["const R: &str = r#\""]),
    ];
    for (what, before) in cases {
        let mut lines: Vec<&str> = before.to_vec();
        lines.push("#[allow(dead_code)]");
        lines.push("fn a() {}");
        if what == "a macro body" {
            lines.push("}; }");
        }
        if what == "a raw string over lines" {
            lines.push("\"#;");
        }
        let mut patch = format!(
            "diff --git a/src/a.rs b/src/a.rs\nnew file mode 100644\nindex 0000000..1111111\n--- /dev/null\n+++ b/src/a.rs\n@@ -0,0 +1,{} @@\n",
            lines.len()
        );
        for line in &lines {
            writeln!(patch, "+{line}").unwrap();
        }
        let rank: Value =
            serde_json::from_str(&run(&patch, &["--stdin-patch", "--rank", "--json"])).unwrap();
        let line = u64::try_from(before.len() + 1).unwrap();
        // A file with no evidence has no `evidence` member: that is "not found" too.
        let found = rank["ranked"][0]["evidence"]
            .as_array()
            .is_some_and(|rows| {
                rows.iter()
                    .any(|e| e["signal"] == "check_suppressed" && e["line"] == line)
            });
        assert!(
            found,
            "after {what}: no check_suppressed at line {line}: {}",
            rank["ranked"][0]
        );
    }
}
