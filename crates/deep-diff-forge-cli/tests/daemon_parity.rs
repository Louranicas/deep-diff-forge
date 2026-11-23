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
        let canonical = |answer: &serde_json::Value| answer.as_str().unwrap_or_default().to_owned();
        let request =
            serde_json::json!({"id": 1, "method": "diff.rank", "params": {"patch": patch}});
        let rpc: serde_json::Value =
            serde_json::from_str(&run(&["rpc"], &format!("{request}\n"))).unwrap();
        let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#;
        let call = serde_json::json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "rank_patch", "arguments": {"patch": patch}}});
        let mcp = run(&["mcp"], &format!("{init}\n{call}\n"));
        let mcp: serde_json::Value = serde_json::from_str(mcp.lines().nth(1).unwrap()).unwrap();
        let surfaces = [
            ("the one renderer", rendered),
            (
                "the daemon's diff.rank canonical_json",
                canonical(&daemon("diff.rank", patch, false, false).unwrap()["canonical_json"]),
            ),
            (
                "rpc's diff.rank canonical_json",
                canonical(&rpc["result"]["canonical_json"]),
            ),
            (
                "MCP rank_patch's text",
                canonical(&mcp["result"]["content"][0]["text"]),
            ),
        ];
        for (surface, bytes) in surfaces {
            assert!(
                bytes == stdout,
                "{surface} differs from the command line's bytes for {patch:?}:\n{bytes}\n{stdout}"
            );
            compared += 1;
        }
    }
    assert!(compared >= 20, "{compared}");
}

fn run(args: &[&str], stdin: &str) -> String {
    let mut child = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "{args:?}");
    String::from_utf8(out.stdout).unwrap()
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
        daemon("diff.rank", &patch, false, false)
            .ok()
            .map(value_form),
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

/// A daemon answer's value form: `diff.rank` adds the document's bytes as `canonical_json`
/// (S3.15; `rank_bytes_are_one_document_on_every_surface` compares them).
fn value_form(mut answer: serde_json::Value) -> serde_json::Value {
    if let Some(members) = answer.as_object_mut() {
        members.remove("canonical_json");
    }
    answer
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
                    assert_eq!(
                        rpc.ok().map(value_form),
                        Some(document),
                        "{patch:?} {flags:?}"
                    );
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

/// F5a.2: `diff.diagnostics` is the CLI's `diagnostics.v0` document: a page holding it all
/// carries `page.document_sha256`, the SHA-256 of the bytes `--stdin-patch --diagnostics --json`
/// prints, and the builder's writer rebuilds those bytes from it; a refusal and a parse failure
/// have the CLI's text and the registry's codes, as `diff.rank` has.
#[test]
fn diff_diagnostics_matches_the_cli_document() {
    let guards: [(&[&str], bool, bool); 4] = [
        (&[], false, false),
        (&["--require-files"], true, false),
        (&["--require-hunks"], false, true),
        (&["--require-files", "--require-hunks"], true, true),
    ];
    let mut seen = [0; 3];
    let big = big_patch();
    for patch in FIXTURES.iter().copied().chain([big.as_str()]) {
        for (flags, files, hunks) in guards {
            let (code, stdout, stderr) = cli(patch, &["--diagnostics"], flags);
            // The largest budget: one page holds the whole document.
            let line = serde_json::json!({
                "method": "diff.diagnostics",
                "params": {"patch": patch, "require_files": files, "require_hunks": hunks, "max_bytes": 16_777_216},
            })
            .to_string();
            let rpc = dispatch(&parse_request(&line).unwrap(), &Engine::new());
            match code {
                exit::SUCCESS => {
                    let page = rpc.unwrap();
                    assert_eq!(
                        page["page"]["document_sha256"],
                        deep_diff_forge_core::sha256_hex(stdout.as_bytes()),
                        "{patch:?} {flags:?}"
                    );
                    assert!(page["page"]["next_cursor"].is_null());
                    assert!(
                        rebuild(patch, std::slice::from_ref(&page)) == stdout,
                        "the builder's writer does not rebuild the command line's bytes from the page for {patch:?} {flags:?}"
                    );
                    seen[0] += 1;
                }
                exit::INPUT_CONTRACT_REFUSED => {
                    let err = rpc.unwrap_err();
                    assert_eq!(
                        (err.code, format!("{}\n", err.message)),
                        (INPUT_CONTRACT_REFUSED, stderr),
                        "{patch:?}"
                    );
                    seen[1] += 1;
                }
                exit::INVALID_INPUT => {
                    let err = rpc.unwrap_err();
                    assert_eq!(
                        (
                            err.code,
                            format!("error: patch parse failed: {}\n", err.message)
                        ),
                        (PATCH_PARSE_FAILED, stderr),
                        "{patch:?}"
                    );
                    seen[2] += 1;
                }
                other => panic!("unexpected exit {other} for {patch:?} {flags:?}: {stderr}"),
            }
        }
    }
    assert!(seen.iter().all(|&n| n > 0), "{seen:?}");
}

/// Six files of spawns, three of them cut at 32 rows, paths of growing length (one with a
/// hidden character), and a line with one: many diagnostics, `cut` entries and texts to page.
fn big_patch() -> String {
    use std::fmt::Write as _;
    let mut text = String::new();
    for f in 0..6 {
        let path = format!("src/m{f}/{}\u{200b}.rs", "n".repeat(40 * f));
        let _ = write!(
            text,
            "diff --git a/{path} b/{path}\nnew file mode 100644\n--- /dev/null\n+++ b/{path}\n@@ -0,0 +1,{} @@\n",
            25 + 5 * f
        );
        for i in 0..25 + 5 * f {
            let _ = writeln!(text, "+std::process::Command::new(\"p{i}\");");
        }
    }
    text.push_str("diff --git a/x.py b/x.py\n--- a/x.py\n+++ b/x.py\n@@ -1 +1 @@\n-a = 1\n+a = eval(s)  # \u{202e}\n");
    text
}

/// The document the pages make, written by the builder's writer: the diagnostics the pages hold,
/// in their order, found by `key` in the in-process document of `patch`, and the `cut` entries
/// the pages hold. A page that drops, repeats or reorders an item rebuilds other bytes.
fn rebuild(patch: &str, pages: &[serde_json::Value]) -> String {
    let guards = deep_diff_forge_patch::InputGuards {
        files: false,
        hunks: false,
    };
    let review = deep_diff_forge_patch::SealedReview::new(
        patch.as_bytes(),
        guards,
        env!("CARGO_PKG_VERSION"),
    )
    .unwrap();
    let ranked = deep_diff_forge_graph::rank(review.files());
    let mut document = deep_diff_forge_graph::diagnostics(review.seal(), &ranked);
    let by_key: std::collections::HashMap<String, deep_diff_forge_graph::Diagnostic> = document
        .diagnostics
        .iter()
        .map(|d| (d.key.clone(), d.clone()))
        .collect();
    document.diagnostics = pages
        .iter()
        .flat_map(|p| p["diagnostics"].as_array().unwrap().iter())
        .map(|d| by_key[d["key"].as_str().unwrap()].clone())
        .collect();
    document.cut = pages
        .iter()
        .flat_map(|p| p["cut"].as_array().unwrap().iter())
        .map(|c| {
            let rows = usize::try_from(c["rows_kept"].as_u64().unwrap()).unwrap();
            (c["path"].as_str().unwrap().to_owned(), rows)
        })
        .collect();
    document.to_json()
}

/// One `rpc` process answering `diff.diagnostics` page after page at `max_bytes`, following
/// `next_cursor`: each answer line as emitted, and the refusal that ended it if one did.
fn rpc_pages(patch: &str, max_bytes: usize) -> (Vec<String>, Option<serde_json::Value>) {
    use std::io::BufRead as _;
    let mut child = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .arg("rpc")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut lines = std::io::BufReader::new(child.stdout.take().unwrap()).lines();
    let (mut pages, mut cursor) = (Vec::new(), serde_json::Value::Null);
    let outcome = loop {
        let request = serde_json::json!({"id": pages.len(), "method": "diff.diagnostics", "params": {"patch": patch, "max_bytes": max_bytes, "cursor": cursor}});
        writeln!(stdin, "{request}").unwrap();
        let line = lines.next().unwrap().unwrap();
        let answer: serde_json::Value = serde_json::from_str(&line).unwrap();
        if let Some(error) = answer.get("error") {
            break Some(error.clone());
        }
        cursor = answer["result"]["page"]["next_cursor"].clone();
        pages.push(line);
        if cursor.is_null() {
            break None;
        }
    };
    drop(stdin);
    let _ = child.kill();
    let _ = child.wait();
    (pages, outcome)
}

/// The `result` of an answer line as emitted, and the size of the rest: `{"id":`, the id,
/// `,"jsonrpc":"2.0","result":` and the closing `}` (33 bytes and the id).
fn emitted_result(line: &str, id: usize) -> &str {
    let start = line.find(",\"jsonrpc\":\"2.0\",\"result\":").unwrap() + 26;
    let result = &line[start..line.len() - 1];
    assert_eq!(
        line.len() - result.len(),
        33 + id.to_string().len(),
        "{line}"
    );
    result
}

/// `scripts/reassemble_diagnostics.py`, written from docs/reference/diagnostics.md alone: the
/// document's bytes from the pages' `result` values, exit 0 when they hash to every page's
/// `document_sha256`.
fn python_reassembly(results: &[&str]) -> (bool, String) {
    let mut child = Command::new("python3")
        .arg("-I")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../scripts/reassemble_diagnostics.py"
        ))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let input: String = results.iter().fold(String::new(), |mut all, r| {
        all.push_str(r);
        all.push('\n');
        all
    });
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    (out.status.success(), String::from_utf8(out.stdout).unwrap())
}

/// F5a.2: pages through `rpc` at any budget from the method's smallest put together to the
/// bytes the command line prints, which hash to every page's `document_sha256`: by the builder's
/// writer, and by a reassembler written from the document alone. No page's emitted `result` is
/// over its budget; a page holds no raw hidden character; at a budget one item cannot fit with
/// the header, the refusal names it.
#[test]
fn paged_diagnostics_reassemble_to_the_cli_document() {
    let big = big_patch();
    let mut multi = 0;
    for patch in FIXTURES.iter().copied().chain([big.as_str()]) {
        let (code, stdout, _) = cli(patch, &["--diagnostics"], &[]);
        if code != exit::SUCCESS {
            continue;
        }
        let digest = deep_diff_forge_core::sha256_hex(stdout.as_bytes());
        for max_bytes in [4096, 16_384, 65_536, 16_777_216] {
            let (lines, refused) = rpc_pages(patch, max_bytes);
            if let Some(error) = refused {
                let message = error["message"].as_str().unwrap();
                assert_eq!(error["code"], -32602, "{message}");
                assert!(
                    message
                        .starts_with(&format!("max_bytes {max_bytes} is below the size of item ")),
                    "{message}"
                );
                continue;
            }
            let mut results = Vec::new();
            for (id, line) in lines.iter().enumerate() {
                let result = emitted_result(line, id);
                assert!(
                    result.len() <= max_bytes,
                    "a page of {} bytes at max_bytes {max_bytes}",
                    result.len()
                );
                assert!(
                    !line.contains(['\u{202e}', '\u{200b}']),
                    "a raw hidden character: {line}"
                );
                results.push(result);
            }
            let pages: Vec<serde_json::Value> = results
                .iter()
                .map(|r| serde_json::from_str(r).unwrap())
                .collect();
            assert!(
                pages
                    .iter()
                    .all(|p| p["page"]["document_sha256"] == digest.as_str())
            );
            assert!(
                rebuild(patch, &pages) == stdout,
                "{} pages at max_bytes {max_bytes}: the builder's writer rebuilds other bytes than the command line's",
                pages.len()
            );
            let (hashed, bytes) = python_reassembly(&results);
            assert!(
                hashed && bytes == stdout,
                "{} pages at max_bytes {max_bytes}: the reassembler written from the document rebuilds other bytes:\n{bytes}\n{stdout}",
                pages.len()
            );
            multi += usize::from(pages.len() > 2);
        }
    }
    assert!(multi >= 2, "only {multi} runs had more than two pages");
}

/// Every string of `value` holding `mark` outside a `path` member.
fn walk_marker(value: &serde_json::Value, key: &str, mark: &str, out: &mut Vec<String>) {
    match value {
        serde_json::Value::String(s) if s.contains(mark) && key != "path" => {
            out.push(format!("{key}: {s}"));
        }
        serde_json::Value::Object(m) => m.iter().for_each(|(k, v)| walk_marker(v, k, mark, out)),
        serde_json::Value::Array(a) => a.iter().for_each(|v| walk_marker(v, key, mark, out)),
        _ => {}
    }
}

/// F5a.2, the canary rule: a marker written into every patch-derived place (line text on both
/// sides and in context, a hunk heading, a rename's old name, a binary file's names, a cursor)
/// appears in no answer and no refusal of `diff.diagnostics` but inside a `path` member's value,
/// and in the emitted line not even there raw: its hidden character is escaped.
#[test]
fn a_marker_in_every_patch_derived_place_stays_in_the_escaped_path() {
    const MARK: &str = "zq7Mark";
    let marker = format!("{MARK}\u{202e}er");
    let patch = format!(
        "diff --git a/src/{marker}.rs b/src/{marker}.rs\n--- a/src/{marker}.rs\n+++ b/src/{marker}.rs\n@@ -1,3 +1,3 @@ fn {marker}() {{\n fn a() {{ \"{marker}\" }}\n-let x = \"{marker}\";\n+std::process::Command::new(\"{marker}\");\n tail {marker}\n\
         diff --git a/old{marker}.rs b/plain.rs\nsimilarity index 90%\nrename from old{marker}.rs\nrename to plain.rs\n--- a/old{marker}.rs\n+++ b/plain.rs\n@@ -1 +1 @@ {marker}\n-{marker}\n+std::process::Command::new(\"{marker}\");\n\
         diff --git a/bin{marker}.png b/bin{marker}.png\nindex 1111111..2222222 100644\nBinary files a/bin{marker}.png and b/bin{marker}.png differ\n"
    );
    let call = |params: serde_json::Value| {
        let line = serde_json::json!({"method": "diff.diagnostics", "params": params}).to_string();
        dispatch(&parse_request(&line).unwrap(), &Engine::new())
    };
    // Not here: a patch that does not parse. Its message is `diff.rank`'s and quotes the line
    // that failed, as docs/reference/mcp-sdk-daemon.md says ("Protocol 3").
    let (mut leaks, mut answers, mut in_paths) = (Vec::new(), 0, 0);
    for max_bytes in [4096, 65_536] {
        let mut cursor = serde_json::Value::Null;
        loop {
            let page =
                call(serde_json::json!({"patch": patch, "max_bytes": max_bytes, "cursor": cursor}))
                    .unwrap();
            walk_marker(&page, "", MARK, &mut leaks);
            answers += 1;
            in_paths += usize::from(page["diagnostics"].to_string().contains(MARK));
            cursor = page["page"]["next_cursor"].clone();
            if cursor.is_null() {
                break;
            }
        }
    }
    // On the wire, the line holds the path's hidden character only escaped.
    let (lines, refused) = rpc_pages(&patch, 4096);
    assert!(refused.is_none() && !lines.is_empty(), "{refused:?}");
    for line in &lines {
        assert!(
            !line.contains(&marker),
            "the raw marker on the wire: {line}"
        );
    }
    let escaped = format!("{MARK}\\u202eer");
    assert!(lines.iter().any(|l| l.contains(&escaped)), "{lines:?}");
    for params in [
        serde_json::json!({"patch": patch, "cursor": format!("dg0.{marker}.1")}),
        serde_json::json!({"patch": patch, "cursor": marker}),
        serde_json::json!({"patch": patch, "max_bytes": 1024}),
        serde_json::json!({"patch": format!("--- a/x\n+++ b/x\n@@ -1 +1 @@ {marker}\n-a\n+b\n"), "require_hunks": true, "require_files": true, "cursor": format!("{marker}.{marker}")}),
    ] {
        match call(params) {
            Err(error) => {
                if error.message.contains(MARK) {
                    leaks.push(format!("error: {}", error.message));
                }
            }
            Ok(page) => walk_marker(&page, "", MARK, &mut leaks),
        }
        answers += 1;
    }
    // Not vacuous: the marker reached the document, in the paths it may stand in.
    assert!(
        answers >= 6 && in_paths >= 2,
        "{answers} answers, {in_paths} with the marker in a path"
    );
    assert!(
        leaks.is_empty(),
        "the marker outside a path:\n{}",
        leaks.join("\n")
    );
}

/// The corpus measurement of lane w11P (run by hand: `DDF_CORPUS_ROOT=<dir>
/// DDF_CORPUS_LIST=<file of relative paths> cargo test --release --test daemon_parity --
/// --ignored`): for every input `diff.diagnostics` serves, at each budget from the method's
/// smallest, the builder's writer rebuilds from the pages the bytes `--diagnostics --json`
/// prints (in process), which hash to every page's `document_sha256`. Prints the counts.
#[test]
#[ignore = "a measurement over a corpus outside the repository"]
fn corpus_pages_rebuild_with_the_builders_writer() {
    let root = std::env::var("DDF_CORPUS_ROOT").unwrap();
    let list = std::fs::read_to_string(std::env::var("DDF_CORPUS_LIST").unwrap()).unwrap();
    let (mut served, mut refused, mut skipped) = ([0usize; 4], [0usize; 4], 0);
    for rel in list.lines().filter(|l| !l.is_empty()) {
        let Ok(patch) = String::from_utf8(std::fs::read(format!("{root}/{rel}")).unwrap()) else {
            skipped += 1;
            continue;
        };
        let Ok(review) = deep_diff_forge_patch::SealedReview::new(
            patch.as_bytes(),
            deep_diff_forge_patch::InputGuards {
                files: false,
                hunks: false,
            },
            env!("CARGO_PKG_VERSION"),
        ) else {
            skipped += 1;
            continue;
        };
        let bytes = deep_diff_forge_graph::diagnostics(
            review.seal(),
            &deep_diff_forge_graph::rank(review.files()),
        )
        .to_json();
        let digest = deep_diff_forge_core::sha256_hex(bytes.as_bytes());
        for (b, max_bytes) in [4096, 16_384, 65_536, 16_777_216].into_iter().enumerate() {
            let (mut pages, mut cursor) = (Vec::new(), serde_json::Value::Null);
            let ok = loop {
                let line = serde_json::json!({"method": "diff.diagnostics", "params": {"patch": patch, "max_bytes": max_bytes, "cursor": cursor}}).to_string();
                match dispatch(&parse_request(&line).unwrap(), &Engine::new()) {
                    Ok(page) => {
                        assert_eq!(page["page"]["document_sha256"], digest.as_str(), "{rel}");
                        cursor = page["page"]["next_cursor"].clone();
                        pages.push(page);
                        if cursor.is_null() {
                            break true;
                        }
                    }
                    Err(e) if e.message.contains("is below the size of item") => break false,
                    Err(e) => panic!("{rel}: {}", e.message),
                }
            };
            if ok {
                assert!(rebuild(&patch, &pages) == bytes, "{rel} at {max_bytes}");
                served[b] += 1;
            } else {
                refused[b] += 1;
            }
        }
    }
    println!(
        "builder's writer: served {served:?} refused {refused:?} at [4096, 16384, 65536, 16777216]; not UTF-8 or not parsed {skipped}"
    );
}
