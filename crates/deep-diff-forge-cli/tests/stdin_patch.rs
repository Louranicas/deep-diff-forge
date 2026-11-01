//! CLI contract tests for `--stdin-patch`, exercising the real compiled binary
//! across the process boundary (stdin, stdout, stderr, exit codes).

#![expect(
    clippy::disallowed_methods,
    reason = "integration test: drives real processes, the environment and wall-clock time"
)]

use std::io::Write as _;
use std::process::{Command, Stdio};

fn run(args: &[&str], stdin: &str) -> (i32, String, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn binary");
    // A command line refused before stdin is read exits without reading it, so the write
    // can race the exit and get EPIPE; that is the behaviour under test, not a failure.
    if let Err(error) = child
        .stdin
        .take()
        .expect("stdin handle")
        .write_all(stdin.as_bytes())
    {
        assert_eq!(
            error.kind(),
            std::io::ErrorKind::BrokenPipe,
            "write stdin: {error}"
        );
    }
    let out = child.wait_with_output().expect("wait for binary");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

const PATCH: &str = "--- a/x\n+++ b/x\n@@ -1,1 +1,1 @@\n-old\n+new\n";

#[test]
fn json_mode_emits_schema_on_stdout() {
    let (code, stdout, _) = run(&["--stdin-patch", "--json"], PATCH);
    assert_eq!(code, 0);
    assert!(stdout.contains("\"schema\": \"deep-diff-forge.review.v0\""));
}

#[test]
fn json_mode_reports_counts() {
    let (code, stdout, _) = run(&["--stdin-patch", "--json"], PATCH);
    assert_eq!(code, 0);
    assert!(stdout.contains("\"additions\": 1"));
    assert!(stdout.contains("\"deletions\": 1"));
}

#[test]
fn human_mode_summarises_files() {
    let (code, stdout, _) = run(&["--stdin-patch"], PATCH);
    assert_eq!(code, 0);
    assert!(stdout.contains('x'));
    assert!(stdout.contains("1 file(s) changed"));
}

#[test]
fn empty_input_is_zero_files_not_an_error() {
    let (code, stdout, _) = run(&["--stdin-patch"], "");
    assert_eq!(code, 0);
    assert!(stdout.contains("0 file(s) changed"));
}

#[test]
fn malformed_patch_exits_four_with_stderr_diagnostic() {
    let bad = "--- a/x\n+++ b/x\n+stray addition with no hunk\n";
    let (code, stdout, stderr) = run(&["--stdin-patch"], bad);
    assert_eq!(code, 4);
    assert_eq!(stdout.len(), 0);
    assert!(stderr.contains("patch parse failed"));
}

#[test]
fn jsonl_mode_streams_one_event_per_file() {
    let two =
        "--- a/a\n+++ b/a\n@@ -1,1 +1,1 @@\n-a\n+A\n--- a/b\n+++ b/b\n@@ -1,1 +1,1 @@\n-b\n+B\n";
    let (code, stdout, _) = run(&["--stdin-patch", "--jsonl"], two);
    assert_eq!(code, 0);
    assert_eq!(stdout.lines().count(), 2);
    assert!(stdout.contains("\"event\":\"diff.file\""));
}

#[test]
fn jsonl_malformed_exits_four() {
    let bad = "--- a/x\n+++ b/x\n+stray\n";
    let (code, stdout, stderr) = run(&["--stdin-patch", "--jsonl"], bad);
    assert_eq!(code, 4);
    assert_eq!(stdout.len(), 0);
    assert_ne!(stderr.len(), 0);
}

#[test]
fn rank_json_emits_schema_and_orders_public_api_first() {
    let two = "\
diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,1 +1,1 @@
-a
+b
diff --git a/tests/it.rs b/tests/it.rs
--- a/tests/it.rs
+++ b/tests/it.rs
@@ -1,1 +1,1 @@
-a
+b
";
    let (code, stdout, _) = run(&["--stdin-patch", "--rank", "--json"], two);
    assert_eq!(code, 0);
    assert!(stdout.contains("\"schema\": \"deep-diff-forge.rank.v0\""));
    assert!(stdout.contains("public_api_surface"));
    // src/lib.rs (public api) should appear before tests/it.rs in the output.
    let lib_pos = stdout.find("src/lib.rs").unwrap();
    let test_pos = stdout.find("tests/it.rs").unwrap();
    assert!(lib_pos < test_pos);
}

#[test]
fn cluster_json_emits_schema_and_receipt() {
    let two = "\
diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,1 +1,1 @@
-a
+b
diff --git a/src/other.rs b/src/other.rs
--- a/src/other.rs
+++ b/src/other.rs
@@ -1,1 +1,1 @@
-a
+b
";
    let (code, stdout, _) = run(
        &["--stdin-patch", "--cluster", "--parallel", "2", "--json"],
        two,
    );
    assert_eq!(code, 0);
    assert!(stdout.contains("\"schema\": \"deep-diff-forge.cluster.v0\""));
    assert!(stdout.contains("\"parallelism\": \"fixed:2\""));
    assert!(stdout.contains("\"join_policy\": \"ranked-review-order\""));
    assert!(stdout.contains("src/lib.rs"));
}

#[test]
fn cluster_human_prints_receipt_footer() {
    let (code, stdout, _) = run(
        &["--stdin-patch", "--cluster", "--parallel", "serial"],
        PATCH,
    );
    assert_eq!(code, 0);
    assert!(stdout.contains("cluster:"));
    assert!(stdout.contains("parallelism=serial"));
}

#[test]
fn rank_human_lists_files_and_count() {
    let (code, stdout, _) = run(&["--stdin-patch", "--rank"], PATCH);
    assert_eq!(code, 0);
    assert!(stdout.contains("1 file(s) ranked"));
}

#[test]
fn inline_layout_renders_header() {
    let (code, stdout, _) = run(&["--stdin-patch", "--layout", "inline"], PATCH);
    assert_eq!(code, 0);
    assert!(stdout.contains("modified  x"));
}

#[test]
fn help_exits_zero() {
    let (code, stdout, _) = run(&["--help"], "");
    assert_eq!(code, 0);
    assert!(stdout.contains("--stdin-patch"));
}

#[test]
fn unknown_command_exits_two() {
    let (code, _, stderr) = run(&["frobnicate"], "");
    assert_eq!(code, 2);
    assert!(stderr.contains("unknown command"));
}

#[test]
fn incompatible_or_inapplicable_review_options_fail_before_stdin_is_read() {
    for args in [
        &["--stdin-patch", "--json", "--jsonl"][..],
        &["--stdin-patch", "--parallel", "2"][..],
        &["--stdin-patch", "--cursor", "opaque"][..],
        &["--stdin-patch", "--max-bytes", "100"][..],
        // Bug hunt 2026-10-28, H2-07: two modes, or a hunk selector on the outline.
        &["--stdin-patch", "--outline", "--hunks"][..],
        &["--stdin-patch", "--rank", "--cluster"][..],
        &["--stdin-patch", "--outline", "--path", "x"][..],
        &["--stdin-patch", "--outline", "--address", "abc"][..],
    ] {
        let (code, stdout, stderr) = run(args, PATCH);
        assert_eq!(code, 2, "{args:?}: {stderr}");
        assert!(stdout.is_empty(), "{args:?}");
    }
}

#[test]
fn layout_and_view_budget_values_are_validated_in_every_mode() {
    for args in [
        &["--stdin-patch", "--rank", "--layout", "bogus"][..],
        &["--stdin-patch", "--json", "--layout", "bogus"][..],
        &["--stdin-patch", "--outline", "--max-bytes", "bogus"][..],
        &["--stdin-patch", "--hunks", "--max-bytes", "bogus"][..],
    ] {
        let (code, stdout, stderr) = run(args, PATCH);
        assert_eq!(code, 2, "{args:?}: {stderr}");
        assert!(stdout.is_empty(), "{args:?}");
    }
}

#[test]
fn unknown_command_diagnostic_is_terminal_safe_and_bounded() {
    let command = format!("\u{1b}[31m{}", "x".repeat(1000));
    let out = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .arg(command)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(!out.stderr.contains(&0x1b));
    assert!(out.stderr.len() < 400, "{}", out.stderr.len());
    assert!(String::from_utf8_lossy(&out.stderr).contains("bytes omitted"));
}

#[test]
fn diagnostics_go_to_stderr_not_stdout() {
    let bad = "--- a/x\n+++ b/x\n-stray removal with no hunk\n";
    let (_, stdout, stderr) = run(&["--stdin-patch", "--json"], bad);
    assert!(stdout.is_empty(), "stdout must stay clean on error");
    assert!(!stderr.is_empty(), "stderr must carry the diagnostic");
}

fn run_bytes(args: &[&str], stdin: &[u8]) -> (i32, String, String) {
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
        .expect("stdin handle")
        .write_all(stdin)
        .expect("write stdin");
    let out = child.wait_with_output().expect("wait");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8(out.stdout).expect("stdout is UTF-8"),
        String::from_utf8(out.stderr).expect("stderr is UTF-8"),
    )
}

const MIXED: &[u8] = b"diff --git a/ok.rs b/ok.rs\n--- a/ok.rs\n+++ b/ok.rs\n@@ -1 +1 @@\n-a\n+b\n\
diff --git a/seed b/seed\nnew file mode 100644\n--- /dev/null\n+++ b/seed\n@@ -0,0 +1 @@\n+\xff{\"jsonrpc\"}\n";

/// Anchor C-12: before, one non-UTF-8 line refused the whole review ("stdin is not valid UTF-8",
/// exit 3; DDF could not review its own merge #11, whose fuzz seeds start with 0xff).
#[test]
fn non_utf8_content_is_reviewed_with_its_file_marked_and_bytes_sealed() {
    let (code, stdout, stderr) = run_bytes(&["--stdin-patch", "--json"], MIXED);
    assert_eq!(code, 0, "{stderr}");
    let doc: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(doc["input_sha256"], deep_diff_forge_core::sha256_hex(MIXED));
    let files = doc["files"].as_array().unwrap();
    assert_eq!(files[0]["path"], "ok.rs");
    assert!(files[0].get("lossy_lines").is_none());
    assert_eq!(files[1]["path"], "seed");
    assert_eq!(files[1]["lossy_lines"], 1);
    assert_eq!(
        files[1]["hunks"][0]["lines"][0]["text"],
        "\u{fffd}{\"jsonrpc\"}"
    );
    assert_eq!(files[1]["hunks"][0]["lines"][0]["lossy"], true);
    assert!(
        files[0]["hunks"][0]["lines"]
            .as_array()
            .unwrap()
            .iter()
            .all(|line| line.get("lossy").is_none())
    );

    let (code, stdout, _) = run_bytes(&["--stdin-patch", "--jsonl"], MIXED);
    assert_eq!(code, 0);
    let events: Vec<&str> = stdout.lines().collect();
    assert_eq!(events.len(), 2);
    assert!(!events[0].contains("lossy_lines"));
    assert!(events[1].ends_with(",\"lossy_lines\":1}"), "{}", events[1]);

    let (code, stdout, _) = run_bytes(&["--stdin-patch"], MIXED);
    assert_eq!(code, 0);
    assert!(
        stdout.contains("seed  (+1 -0, 1 hunks)  [1 non-UTF-8 line(s) shown lossily]"),
        "{stdout}"
    );

    // The guards still apply to the decoded patch.
    let (code, _, _) = run_bytes(&["--stdin-patch", "--json", "--require-hunks"], MIXED);
    assert_eq!(code, 0);
}

/// Anchor D-01: every surface names a status with `FileStatus::label`. The human summary used
/// `{:?}` lowercased, printing `binarychanged` where JSON, JSONL and the projections print
/// `binary_changed`.
#[test]
fn every_surface_names_a_status_with_the_one_label() {
    let patch = "diff --git a/x.bin b/x.bin\nindex 1111111..2222222 100644\nBinary files a/x.bin and b/x.bin differ\n";
    let (_, human, _) = run(&["--stdin-patch"], patch);
    let (_, json, _) = run(&["--stdin-patch", "--json"], patch);
    let (_, jsonl, _) = run(&["--stdin-patch", "--jsonl"], patch);
    let (_, inline, _) = run(&["--stdin-patch", "--layout", "inline"], patch);
    assert!(human.contains("binary_changed  x.bin"), "{human}");
    assert!(json.contains("\"status\": \"binary_changed\""), "{json}");
    assert!(jsonl.contains("\"status\":\"binary_changed\""), "{jsonl}");
    assert!(inline.contains("binary_changed"), "{inline}");
}

/// `--parallel` takes `serial`, `auto` or a worker count from 1; anything else is a usage error.
/// Before, an unreadable count quietly ran as `auto`, and 0 was reported as `fixed:0`.
#[test]
fn an_unreadable_or_zero_worker_count_is_refused() {
    for bad in ["banana", "0", "-1", "65536"] {
        let (code, stdout, stderr) = run(&["--stdin-patch", "--cluster", "--parallel", bad], PATCH);
        assert_eq!(code, 2, "{bad}: {stderr}");
        assert_eq!(stdout, "", "{bad}");
        assert!(
            stderr.contains(&format!(
                "--parallel takes serial, auto or a worker count from 1, not {bad}"
            )),
            "{stderr}"
        );
    }
    let (code, stdout, _) = run(&["--stdin-patch", "--cluster", "--parallel", "1"], PATCH);
    assert_eq!(code, 0);
    assert!(stdout.contains("parallelism=fixed:1"), "{stdout}");
}

/// `chain-contract` states what the JSON and JSONL output really carry: a schema name on every
/// document and an event name on every JSONL record (a JSONL record has no schema version).
#[test]
fn chain_contract_promises_what_jsonl_carries() {
    let (code, stdout, _) = run(&["chain-contract"], "");
    assert_eq!(code, 0);
    assert!(
        stdout
            .contains("every JSON document names its schema; every JSONL record names its event."),
        "{stdout}"
    );
    let (_, events, _) = run(&["--stdin-patch", "--jsonl"], PATCH);
    for line in events.lines() {
        let event: serde_json::Value = serde_json::from_str(line).unwrap();
        assert!(event["event"].is_string(), "{line}");
    }
}

/// Bug hunt 2026-11-01: a second output mode was dropped with exit 0. `--jsonl --rank` printed
/// only JSONL, `--json --layout inline` only JSON, `--cluster --layout inline` only the cluster,
/// and `--jsonl --hunks --path nothing` skipped the path check. `--json` stays the format of
/// `--rank` and `--cluster`, and is redundant but true with `--outline` and `--hunks`.
#[test]
fn a_second_output_mode_is_refused_not_dropped() {
    for (args, refused) in [
        (&["--jsonl", "--rank"][..], "--rank and --jsonl"),
        (&["--jsonl", "--cluster"], "--cluster and --jsonl"),
        (&["--jsonl", "--outline"], "--outline and --jsonl"),
        (&["--jsonl", "--layout", "inline"], "--jsonl and --layout"),
        (
            &["--jsonl", "--hunks", "--path", "nothing"],
            "--hunks and --jsonl",
        ),
        (&["--json", "--layout", "inline"], "--json and --layout"),
        (
            &["--cluster", "--layout", "inline"],
            "--cluster and --layout",
        ),
        (
            &["--rank", "--json", "--layout", "side-by-side"],
            "--rank and --layout",
        ),
        (
            &["--outline", "--layout", "inline"],
            "--outline and --layout",
        ),
    ] {
        let args: Vec<&str> = std::iter::once("--stdin-patch")
            .chain(args.iter().copied())
            .collect();
        let (code, stdout, stderr) = run(&args, PATCH);
        assert_eq!((code, stdout.as_str()), (2, ""), "{args:?}: {stderr}");
        assert!(
            stderr.contains(&format!("error: {refused} are separate modes; choose one")),
            "{args:?}: {stderr}"
        );
    }
    for format in [
        &["--rank", "--json"][..],
        &["--cluster", "--json"],
        &["--outline", "--json"],
        &["--hunks", "--json"],
    ] {
        let args: Vec<&str> = std::iter::once("--stdin-patch")
            .chain(format.iter().copied())
            .collect();
        let (code, stdout, stderr) = run(&args, PATCH);
        assert_eq!(code, 0, "{args:?}: {stderr}");
        assert!(stdout.contains("\"schema\""), "{args:?}: {stdout}");
    }
}

/// `--address` with `--path` is refused before the patch is read, so a patch that would not
/// parse (exit 4) cannot hide the usage error (exit 2).
#[test]
fn address_with_path_is_refused_before_the_patch_is_read() {
    let truncated = "--- a/x\n+++ b/x\n@@ -1,5 +1 @@\n-a\n";
    let (code, stdout, stderr) = run(
        &["--stdin-patch", "--hunks", "--address", "x", "--path", "y"],
        truncated,
    );
    assert_eq!((code, stdout.as_str()), (2, ""), "{stderr}");
    assert!(
        stderr.contains("--hunks takes --address or --path, not both"),
        "{stderr}"
    );
}

/// Bug hunt 2026-11-01: a repeated single-valued option used its first value and never read the
/// later one, so `--parallel 4 --parallel bogus` ran with 4 and exit 0.
#[test]
fn a_repeated_single_valued_option_is_refused() {
    for (args, option) in [
        (
            &["--cluster", "--parallel", "4", "--parallel", "bogus"][..],
            "--parallel",
        ),
        (
            &["--cluster", "--parallel", "4", "--parallel", "8"],
            "--parallel",
        ),
        (&["--layout", "inline", "--layout", "bogus"], "--layout"),
        (
            &["--outline", "--max-bytes", "4096", "--max-bytes", "x"],
            "--max-bytes",
        ),
    ] {
        let args: Vec<&str> = std::iter::once("--stdin-patch")
            .chain(args.iter().copied())
            .collect();
        let (code, stdout, stderr) = run(&args, PATCH);
        assert_eq!((code, stdout.as_str()), (2, ""), "{args:?}: {stderr}");
        assert!(
            stderr.contains(&format!("error: {option} given more than once")),
            "{args:?}: {stderr}"
        );
    }
    let (code, _, stderr) = run(
        &["--stdin-patch", "--hunks", "--path", "x", "--path", "x"],
        PATCH,
    );
    assert_eq!(code, 0, "a repeatable option stays repeatable: {stderr}");
}

/// Bug hunt 2026-11-01: a `-h` that is an option's value is that value, not a help request.
#[test]
fn a_path_named_dash_h_is_selected_not_taken_for_help() {
    let patch = "diff --git a/-h b/-h\n--- a/-h\n+++ b/-h\n@@ -1 +1 @@\n-a\n+b\n";
    let (code, stdout, stderr) = run(&["--stdin-patch", "--hunks", "--path", "-h"], patch);
    assert_eq!(code, 0, "{stderr}");
    let view: serde_json::Value =
        serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("{e}: {stdout}"));
    assert!(view.to_string().contains("\"path\":\"-h\""), "{view}");
}

/// `chain-contract` states what a closed or failing stdout does: a closed pipe exits 0 (before,
/// it promised a non-zero exit, which `emit` deliberately does not give), and it names every
/// command that reads stdin.
#[test]
fn chain_contract_states_the_shipped_pipe_and_stdin_behaviour() {
    let (code, contract, _) = run(&["chain-contract"], "");
    assert_eq!(code, 0);
    assert!(
        contract.contains(
            "  - a closed stdout pipe (`| head`) exits 0 and the reader keeps what it read;\n    any other write failure exits 3 (io).\n"
        ),
        "{contract}"
    );
    for reader in [
        "--stdin-patch",
        "learn record --stdin",
        "findings verify",
        "review",
        "rpc and mcp",
    ] {
        assert!(contract.contains(reader), "{reader}: {contract}");
    }
    let patch = (0..3000).fold(String::new(), |mut patch, i| {
        use std::fmt::Write as _;
        let _ = write!(patch, "--- a/f{i}\n+++ b/f{i}\n@@ -1 +1 @@\n-a\n+b\n");
        patch
    });
    let mut child = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .args(["--stdin-patch", "--json"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    child
        .stdin
        .take()
        .unwrap()
        .write_all(patch.as_bytes())
        .unwrap();
    assert_eq!(child.wait().unwrap().code(), Some(0));
}
