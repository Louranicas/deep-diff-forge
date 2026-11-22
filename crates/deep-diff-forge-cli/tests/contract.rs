//! Slice A: the machine contract (`envelope`, `schema`, `explain`) from the outside.
#![expect(
    clippy::disallowed_methods,
    reason = "integration test: drives real processes, the environment and wall-clock time"
)]

use std::io::Write as _;
use std::process::{Command, Stdio};

const PATCH: &str = "--- a/x\n+++ b/x\n@@ -1,1 +1,1 @@\n-old\n+new\n";

fn run(args: &[&str], stdin: &str) -> (i32, String, String) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"));
    // The agent hooks and the gate keep a session's state in the user's state directory. A test
    // must never write there: these runs get a home of their own, and no project is named to
    // them. (Found 2026-11-14: `every_documented_option_is_admitted` ran three stop hooks and the
    // gate with the developer's own `HOME` and left `~/.local/state/deep-diff-forge/hooks`.)
    if args.first() == Some(&"agent") {
        let home = std::env::temp_dir().join(format!("ddf-contract-home-{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        command
            .env("HOME", &home)
            .env_remove("XDG_STATE_HOME")
            .env_remove("CLAUDE_PROJECT_DIR");
    }
    let mut child = command
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // A command that exits without reading stdin closes the pipe first; that is not a failure (the
    // exit code is what is checked). Any other write error is.
    if let Err(error) = child.stdin.take().unwrap().write_all(stdin.as_bytes()) {
        assert_eq!(error.kind(), std::io::ErrorKind::BrokenPipe, "{error}");
    }
    let out = child.wait_with_output().unwrap();
    (
        out.status.code().unwrap(),
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

fn envelope(args: &[&str], stdin: &str) -> (i32, serde_json::Value) {
    let (code, stdout, stderr) = run(args, stdin);
    let value = serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("{e}: {stdout} {stderr}"));
    (code, value)
}

#[test]
fn envelope_wraps_a_json_command_and_passes_stdin_through() {
    let (code, v) = envelope(&["envelope", "--", "--stdin-patch", "--json"], PATCH);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["schema"], "ddf.result.v1");
    assert_eq!(
        (v["outcome"].as_str(), v["ok"].as_bool()),
        (Some("success"), Some(true))
    );
    assert_eq!(v["stdout_format"], "json");
    assert_eq!(v["stdout"]["schema"], "deep-diff-forge.review.v0");
    assert_eq!(v["command"], serde_json::json!(["--stdin-patch", "--json"]));
}

#[test]
fn envelope_keeps_the_commands_exit_code_and_names_it() {
    let (code, v) = envelope(
        &[
            "envelope",
            "--",
            "--stdin-patch",
            "--json",
            "--require-files",
        ],
        "",
    );
    assert_eq!(code, 7, "{v}");
    assert_eq!(
        (v["outcome"].as_str(), v["ok"].as_bool()),
        (Some("input_contract_refused"), Some(false))
    );
    assert_eq!(v["stdout_format"], "empty");
    assert!(
        v["stderr"].as_str().unwrap().contains("refused: 0 files"),
        "{v}"
    );
    let (code, v) = envelope(&["envelope", "--", "no-such-command"], "");
    assert_eq!((code, v["outcome"].as_str()), (2, Some("usage")));
}

#[test]
fn envelope_names_the_council_family() {
    let (code, v) = envelope(&["envelope", "--", "council", "help"], "");
    assert_eq!(code, 0, "{v}");
    assert_eq!(
        (v["family"].as_str(), v["outcome"].as_str()),
        (Some("council"), Some("completed"))
    );
}

#[test]
fn envelope_usage_errors_exit_2_with_nothing_on_stdout() {
    for args in [
        &["envelope"][..],
        &["envelope", "doctor"],
        &["envelope", "--", "envelope", "--", "doctor"],
    ] {
        let (code, stdout, stderr) = run(args, "");
        assert_eq!(code, 2, "{args:?}");
        assert!(
            stdout.is_empty() && stderr.starts_with("error: "),
            "{args:?}: {stderr}"
        );
    }
}

#[test]
fn explain_and_schema_answer_from_the_registry() {
    let (code, stdout, _) = run(&["explain", "4", "--family", "council"], "");
    assert_eq!(code, 0);
    assert!(stdout.starts_with("council 4 (refused): "), "{stdout}");
    let (code, _, _) = run(&["explain", "5"], "");
    assert_eq!(code, 2);
    let (code, stdout, _) = run(&["schema"], "");
    assert_eq!(code, 0);
    let schema: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(schema["properties"]["schema"]["const"], "ddf.result.v1");
}

#[test]
fn schema_signals_and_explain_signal_answer_from_the_vocabulary() {
    let (code, stdout, _) = run(&["schema", "signals"], "");
    assert_eq!(code, 0);
    let doc: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(doc["schema"], "deep-diff-forge.signals.v0");
    // 21 labels, the five integrity labels since the merge with them, and `diff_suppressed`
    assert_eq!(doc["signals"].as_array().unwrap().len(), 27);
    let (code, stdout, _) = run(&["explain", "signal", "diff_suppressed"], "");
    assert_eq!(code, 0);
    assert!(
        stdout.starts_with("diff_suppressed (security, direct, weight 5): "),
        "{stdout}"
    );
    let (code, stdout, _) = run(&["explain", "signal", "test_removed"], "");
    assert_eq!(code, 0);
    assert!(
        stdout.starts_with("test_removed (integrity, direct, weight 3): "),
        "{stdout}"
    );
    let (code, stdout, _) = run(&["explain", "signal", "test_removed", "--json"], "");
    assert_eq!(code, 0);
    let doc: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(doc["schema"], "deep-diff-forge.signal-explain.v0");
    let (code, stdout, stderr) = run(&["explain", "signal", "no_such_signal"], "");
    assert_eq!((code, stdout.as_str()), (2, ""));
    assert!(
        stderr.contains("unknown signal: no_such_signal"),
        "{stderr}"
    );
    let (code, _, stderr) = run(&["schema", "other"], "");
    assert_eq!(code, 2);
    assert!(stderr.contains("(result|exit-codes|signals)"), "{stderr}");
}

/// The two signal tables of `docs/reference/risk-ranking.md` (once the README's "Risk ranking")
/// name every label of the vocabulary, and each row's score is the weight the scorer uses: one
/// source of truth, and the document is held to it.
#[test]
fn risk_ranking_signal_tables_match_the_vocabulary() {
    let readme = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/reference/risk-ranking.md"
    ))
    .unwrap();
    let start = readme.find("| Signal | When | Score |").unwrap();
    let section = &readme[start..];
    let section = &section[..section.find("\nLanguages come from the extension").unwrap()];
    let (code, stdout, _) = run(&["schema", "signals"], "");
    assert_eq!(code, 0);
    let doc: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let weights: std::collections::BTreeMap<&str, u64> = doc["signals"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| (s["label"].as_str().unwrap(), s["weight"].as_u64().unwrap()))
        .collect();
    let mut seen = std::collections::BTreeSet::new();
    for row in section.lines().filter(|l| l.starts_with("| `")) {
        let row = row.trim().trim_start_matches("| ").trim_end_matches(" |");
        let first = row.split(" | ").next().unwrap();
        let last = row.rsplit(" | ").next().unwrap();
        let labels: Vec<&str> = first
            .split('`')
            .skip(1)
            .step_by(2)
            .filter(|word| weights.contains_key(word))
            .collect();
        assert!(!labels.is_empty(), "no known label in {first:?}");
        // A row of one signal has one score cell, whatever it says: a signal that adds nothing
        // explains itself there in words.
        let scores: Vec<&str> = if labels.len() == 1 {
            vec![last]
        } else {
            last.split('\u{b7}').map(str::trim).collect()
        };
        assert_eq!(labels.len(), scores.len(), "{row}");
        for (label, score) in labels.iter().zip(&scores) {
            let weight = weights[label];
            if weight == 0 {
                assert!(!score.starts_with('+'), "{label}: {score}");
            } else {
                assert_eq!(*score, format!("+{weight}"), "{label}");
            }
            assert!(seen.insert(*label), "{label} is in two rows");
        }
    }
    let all: std::collections::BTreeSet<&str> = weights.keys().copied().collect();
    assert_eq!(
        seen, all,
        "the tables of docs/reference/risk-ranking.md and the vocabulary differ"
    );
}

/// The exit-code tables of `docs/reference/output-and-exit-codes.md` (once the README's "Exit
/// codes") and the binary's registry name the same codes, and each row begins with the
/// registry's meaning.
#[test]
fn exit_code_tables_match_the_registry() {
    let readme = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/reference/output-and-exit-codes.md"
    ))
    .unwrap();
    let section = &readme[readme.find("## Exit codes").unwrap()..];
    let section = &section[..section.find("\n## ").unwrap()];
    let (code, stdout, _) = run(&["schema", "exit-codes"], "");
    assert_eq!(code, 0);
    let registry: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    for (family, header) in [
        ("engine", "| Code | Meaning | Next |"),
        ("council", "| Council code | Meaning | Next |"),
    ] {
        let table = &section[section.find(header).unwrap_or_else(|| panic!("{header}"))..];
        let rows: Vec<(i64, String, String)> = table
            .lines()
            .skip(2)
            .take_while(|l| l.starts_with("| "))
            .map(|l| {
                let cells: Vec<&str> = l.trim_matches('|').split(" | ").map(str::trim).collect();
                (
                    cells[0].parse().unwrap(),
                    cells[1].to_owned(),
                    cells[2].trim_matches('`').to_owned(),
                )
            })
            .collect();
        let codes: Vec<&serde_json::Value> = registry["codes"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|c| c["family"] == family)
            .collect();
        assert_eq!(rows.len(), codes.len(), "{family}: {rows:?}");
        for (row, entry) in rows.iter().zip(codes) {
            assert_eq!(row.0, entry["code"].as_i64().unwrap(), "{family}");
            assert!(
                row.1.starts_with(entry["meaning"].as_str().unwrap()),
                "{family} {}: {}",
                row.0,
                row.1
            );
            assert_eq!(
                row.2,
                entry["action"].as_str().unwrap(),
                "{family} {}",
                row.0
            );
            assert!(
                section.contains(&format!("- `{}`: ", row.2)),
                "docs/reference/output-and-exit-codes.md does not explain the action {}",
                row.2
            );
        }
    }
}

/// True when `rest` begins with a decimal literal (optionally after a minus sign).
fn starts_numeric(rest: &str) -> bool {
    rest.trim_start_matches('-')
        .starts_with(|c: char| c.is_ascii_digit())
}

/// The numeric codes a source line hands over directly: `exit(N)` (qualified or not),
/// `ExitCode::from(N)`, a council `Ok((…, N))`, a `code: N` pattern or field, or a `=> N,` match
/// arm in an exit-code function.
fn numeric_exit(line: &str, exit_arms: bool) -> bool {
    let code = line.trim();
    if code.starts_with("//") {
        return false;
    }
    let call = ["exit(", "ExitCode::from(", "code: "]
        .iter()
        .any(|form| code.split(form).skip(1).any(starts_numeric));
    let pair = code.contains("Ok((")
        && code.rsplit(", ").next().is_some_and(|tail| {
            starts_numeric(tail)
                && tail
                    .trim_end_matches([')', ','])
                    .chars()
                    .all(|c| c.is_ascii_digit())
        });
    let arm = exit_arms
        && code
            .rsplit("=> ")
            .next()
            .is_some_and(|tail| code.contains("=> ") && starts_numeric(tail));
    call || pair || arm
}

/// True when the line holds a JSON-RPC reserved error code literal (`-32000` to `-32768`).
fn json_rpc_literal(line: &str) -> bool {
    let code = line.trim();
    !code.starts_with("//")
        && code.split("-32").skip(1).any(|rest| {
            rest.len() >= 3
                && rest.as_bytes()[..3].iter().all(u8::is_ascii_digit)
                && !rest.as_bytes().get(3).is_some_and(u8::is_ascii_digit)
        })
}

/// Every `.rs` file under `crates/*/src`, by path relative to `crates/`: a new file is scanned
/// without being listed (anchor B-02).
fn source_files() -> Vec<(String, String)> {
    let root = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/.."));
    let mut dirs: Vec<_> = std::fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().path().join("src"))
        .filter(|dir| dir.is_dir())
        .collect();
    let mut files = Vec::new();
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                let name = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                files.push((name, std::fs::read_to_string(&path).unwrap()));
            }
        }
    }
    files.sort();
    files
}

/// The non-test lines of a source file, numbered from 1. Each `#[cfg(test)]` item is skipped to
/// its closing brace (or `;`) wherever it sits; cutting at the first one hid the production code
/// after a mid-file test item.
fn production_lines(text: &str) -> Vec<(usize, &str)> {
    let mut kept = Vec::new();
    let mut lines = text.lines().enumerate();
    while let Some((index, line)) = lines.next() {
        if line.trim() != "#[cfg(test)]" {
            kept.push((index + 1, line));
            continue;
        }
        let (mut opens, mut closes) = (0, 0);
        for (_, inner) in lines.by_ref() {
            opens += inner.matches('{').count();
            closes += inner.matches('}').count();
            if (opens > 0 && opens == closes) || (opens == 0 && inner.trim_end().ends_with(';')) {
                break;
            }
        }
    }
    kept
}

/// Files whose exit-code functions are `match` tables, so a numeric `=> N` arm is a code.
const EXIT_ARM_FILES: &[&str] = &["deep-diff-forge-cli/src/git_input.rs"];

/// Where a JSON-RPC error code may be written as a number: its named constant, in the daemon's
/// protocol module or (for MCP's own codes) the MCP server.
const JSON_RPC_CODE_HOMES: &[&str] = &[
    "deep-diff-forge-daemon/src/protocol.rs",
    "deep-diff-forge-cli/src/mcp.rs",
];

/// Every production line in every `crates/*/src` file, with its file and line number.
fn production_sites() -> Vec<(String, usize, String)> {
    source_files()
        .into_iter()
        .flat_map(|(file, text)| {
            production_lines(&text)
                .into_iter()
                .map(|(number, line)| (file.clone(), number, line.to_owned()))
                .collect::<Vec<_>>()
        })
        .collect()
}

#[test]
fn the_scans_see_every_file_and_the_code_after_a_mid_file_test_item() {
    let files = source_files();
    let named = |file: &str| {
        let text = &files.iter().find(|(name, _)| name == file).unwrap().1;
        production_lines(text)
            .into_iter()
            .map(|(_, line)| line.to_owned())
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert!(
        files
            .iter()
            .any(|(name, _)| name == "deep-diff-forge-sdk/src/lib.rs")
    );
    assert!(
        files
            .iter()
            .any(|(name, _)| name == "deep-diff-forge-cli/src/mcp.rs")
    );
    let serve = named("deep-diff-forge-daemon/src/serve.rs");
    // `handle_connection` follows the test-only `test_hooks` module; `release` is inside it.
    assert!(serve.contains("pub fn handle_connection(") && !serve.contains("fn release("));
    let core = named("deep-diff-forge-core/src/lib.rs");
    assert!(core.contains("pub struct PatchTwin") && !core.contains("mod file_status_tests"));
    let lines =
        production_lines("a\n#[cfg(test)]\nuse x;\nb\n#[cfg(test)]\nmod t {\n    {}\n}\nc\n");
    assert_eq!(lines, [(1, "a"), (4, "b"), (9, "c")]);
}

/// Every exit site uses the registry's constants (`deep_diff_forge_core::exit`): a numeric literal
/// would be a code the registry does not own (council sa1).
#[test]
fn exit_sites_use_registry_constants() {
    assert!(numeric_exit("        std::process::exit(4);", false));
    assert!(numeric_exit("        exit(4);", false));
    assert!(numeric_exit("    ExitCode::from(4)", false));
    assert!(numeric_exit(
        "        Self::Rpc { code: 4, .. } => 4,",
        false
    ));
    assert!(numeric_exit("    Ok((HELP.into(), 0)),", false));
    assert!(numeric_exit(
        "        GitAcquireError::Spawn { .. } => 3,",
        true
    ));
    assert!(!numeric_exit(
        "        std::process::exit(exit::IO);",
        false
    ));
    assert!(!numeric_exit("    ExitCode::from(exit::IO)", false));
    for (file, number, line) in production_sites() {
        let exit_arms = EXIT_ARM_FILES.contains(&file.as_str());
        assert!(!numeric_exit(&line, exit_arms), "{file}:{number}: {line}");
    }
}

/// JSON-RPC error codes are named: a literal is allowed only as the constant's own definition in
/// its home module (anchor B-02; before, the MCP server wrote `-32600` six times).
#[test]
fn json_rpc_codes_are_named_constants() {
    assert!(json_rpc_literal("        error(id, -32600, \"bad\", None)"));
    assert!(json_rpc_literal("pub const PARSE_ERROR: i64 = -32700;"));
    assert!(!json_rpc_literal(
        "        error(id, INVALID_REQUEST, \"bad\", None)"
    ));
    assert!(!json_rpc_literal("    let n = -321;"));
    assert!(!json_rpc_literal("    let n = -32_1000;"));
    assert!(!json_rpc_literal("    let n = -320000;"));
    for (file, number, line) in production_sites() {
        let item = line.trim_start();
        let definition = JSON_RPC_CODE_HOMES.contains(&file.as_str())
            && (item.starts_with("const ") || item.starts_with("pub const "));
        assert!(
            !json_rpc_literal(&line) || definition,
            "{file}:{number}: {line}"
        );
    }
}

/// Every `.rs` file under the workspace's crates, tests and benches included, by path relative
/// to `crates/`.
fn every_rust_file() -> Vec<(String, String)> {
    let root = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/.."));
    let mut dirs = vec![root.to_path_buf()];
    let mut files = Vec::new();
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                let name = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                files.push((name, std::fs::read_to_string(&path).unwrap()));
            }
        }
    }
    files.sort();
    files
}

/// The lines of `text` that reach the temp dir by name: any `temp_dir`, so neither
/// `std::env::temp_dir()`, an imported `temp_dir` nor a local helper of that name slips by.
fn temp_dir_lines(text: &str) -> Vec<(usize, &str)> {
    text.lines()
        .enumerate()
        .filter(|(_, line)| line.contains("temp_dir"))
        .map(|(index, line)| (index + 1, line))
        .collect()
}

/// DDF-TEST-TMPLEAK-001: a test's files live in a `deep_diff_forge_testkit::Scratch`, which
/// removes them when it drops, pass or panic. Before, about 100 sites built
/// `temp_dir()/ddf-<name>-<pid>` by hand, and one full test run left 51 entries behind.
/// `clippy.toml` disallows `std::env::temp_dir`, but integration tests opt out of
/// `disallowed_methods` file by file, so only a scan of every file can hold the line.
#[test]
fn only_the_testkit_names_the_temp_dir() {
    const DOOR: &str = "deep-diff-forge-testkit/src/lib.rs";
    const THIS: &str = "deep-diff-forge-cli/tests/contract.rs";
    assert_eq!(
        temp_dir_lines("a\nlet d = std::env::temp_dir();\nuse std::env::temp_dir;\nb\n"),
        [
            (2, "let d = std::env::temp_dir();"),
            (3, "use std::env::temp_dir;")
        ]
    );
    let files = every_rust_file();
    assert!(
        files
            .iter()
            .any(|(name, _)| name == "deep-diff-forge-cli/tests/contract.rs"),
        "the scan must see integration tests"
    );
    let offenders: Vec<String> = files
        .iter()
        .filter(|(name, _)| name != DOOR && name != THIS)
        .flat_map(|(name, text)| {
            temp_dir_lines(text)
                .into_iter()
                .map(move |(number, line)| format!("{name}:{number}: {}", line.trim()))
        })
        .collect();
    assert!(
        offenders.is_empty(),
        "use deep_diff_forge_testkit::Scratch, not the temp dir by hand:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn envelope_keeps_the_commands_code_when_its_reader_closes() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .args([
            "envelope",
            "--",
            "--stdin-patch",
            "--json",
            "--require-files",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    drop(child.stdout.take()); // the reader goes away before the envelope writes
    drop(child.stdin.take());
    assert_eq!(child.wait().unwrap().code(), Some(7));
}

/// Anchor A-01: a misspelt or invented option is refused with exit 2 and an empty stdout, so a
/// gate typo cannot silently drop the guard it names. Before this, `--require-file` on an empty
/// input exited 0.
#[test]
fn unknown_options_are_refused_not_ignored() {
    for args in [
        &["--stdin-patch", "--require-file"][..],
        &["--stdin-patch", "--requre-files"],
        &["--stdin-patch", "--jsno"],
        &["--stdin-patch", "--json=true"],
        &["--stdin-patch", "--layout"],
        &["--git-staged", "--rpo", "."],
        &["doctor", "--json"],
        &["--version", "extra"],
        &["schema", "result", "extra"],
        &["deploy", "status", "--jsn"],
        &["learn", "status", "--jsonn"],
        &["daemon", "path", "--sockett", "x"],
        &["semantic", "a.rs", "b.rs"],
        &["structural", "a.rs", "b.rs", "--jsn"],
        &["highlight", "a.rs", "--colour"],
        &["review", "--prob"],
    ] {
        let (code, stdout, stderr) = run(args, "");
        assert_eq!(code, 2, "{args:?}: {stderr}");
        assert!(stdout.is_empty(), "{args:?} wrote stdout");
        assert!(stderr.starts_with("error: "), "{args:?}: {stderr}");
    }
    let (_, _, stderr) = run(&["--stdin-patch", "--require-file"], "");
    assert!(
        stderr.contains("unknown option: --require-file\n"),
        "{stderr}"
    );
}

/// The legitimate side of A-01: every option `--help` documents is admitted by a real invocation,
/// and a newly documented option needs a row here.
#[test]
fn every_documented_option_is_admitted() {
    let dir = std::env::temp_dir().join(format!("ddf-a01-{}", std::process::id()));
    let (source, findings) = (
        scratch(&dir, "a.rs", "fn a() {}\n"),
        scratch(&dir, "f.jsonl", ""),
    );
    let socket = dir.join("s.sock");
    let (address, cursor) = view_handles();
    let mut rows = patch_option_rows(&address, &cursor, &findings);
    rows.extend(command_option_rows(&source, socket.to_str().unwrap()));
    admits_every_documented_option(&rows);
    let _ = std::fs::remove_dir_all(&dir);
}

/// One admitted invocation for each option of the patch-reading commands.
fn patch_option_rows<'a>(
    address: &'a str,
    cursor: &'a str,
    findings: &'a str,
) -> Vec<(&'static str, Vec<&'a str>)> {
    let repo = env!("CARGO_MANIFEST_DIR");
    vec![
        ("--json", vec!["--stdin-patch", "--json"]),
        ("--outline", vec!["--stdin-patch", "--outline"]),
        ("--hunks", vec!["--stdin-patch", "--hunks"]),
        (
            "--address",
            vec!["--stdin-patch", "--hunks", "--address", address],
        ),
        ("--path", vec!["--stdin-patch", "--hunks", "--path", "x"]),
        (
            "--max-bytes",
            vec!["--stdin-patch", "--outline", "--max-bytes", "4096"],
        ),
        (
            "--cursor",
            vec!["--stdin-patch", "--outline", "--cursor", cursor],
        ),
        ("--allow-empty", vec!["--stdin-patch", "--allow-empty"]),
        ("--allow-repo", vec!["rpc", "--allow-repo", repo]),
        ("--client", vec!["agent", "setup", "--client", "codex"]),
        (
            "--hooks",
            vec!["agent", "setup", "--client", "claude", "--hooks"],
        ),
        ("--gate", vec!["agent", "doctor", "--gate"]),
        (
            "--judge",
            vec![
                "agent", "hook", "stop", "--client", "claude", "--judge", "x",
            ],
        ),
        (
            "--policy",
            vec![
                "agent", "hook", "stop", "--client", "claude", "--policy", "x",
            ],
        ),
        (
            "--ranker-dir",
            vec![
                "agent",
                "hook",
                "stop",
                "--client",
                "claude",
                "--ranker-dir",
                "x",
            ],
        ),
        ("--jsonl", vec!["--stdin-patch", "--jsonl"]),
        ("--rank", vec!["--stdin-patch", "--rank"]),
        ("--cluster", vec!["--stdin-patch", "--cluster"]),
        (
            "--parallel",
            vec!["--stdin-patch", "--cluster", "--parallel", "2"],
        ),
        (
            "--layout",
            vec!["--stdin-patch", "--layout", "side-by-side"],
        ),
        ("--require-files", vec!["--stdin-patch", "--require-files"]),
        (
            "--findings",
            vec!["findings", "verify", "--findings", findings],
        ),
        (
            "--require-all",
            vec![
                "findings",
                "verify",
                "--findings",
                findings,
                "--require-all",
            ],
        ),
        (
            "--require-findings",
            vec![
                "findings",
                "verify",
                "--findings",
                findings,
                "--require-findings",
            ],
        ),
        ("--require-hunks", vec!["--stdin-patch", "--require-hunks"]),
        ("--stdin-patch", vec!["--stdin-patch"]),
    ]
}

/// One admitted invocation for each option of the other commands.
fn command_option_rows<'a>(source: &'a str, socket: &'a str) -> Vec<(&'static str, Vec<&'a str>)> {
    let repo = env!("CARGO_MANIFEST_DIR");
    vec![
        (
            "--git-working-tree",
            vec!["--git-working-tree", "--repo", repo, "--acquisition-json"],
        ),
        (
            "--include-untracked",
            vec![
                "--git-working-tree",
                "--include-untracked",
                "--repo",
                repo,
                "--rank",
            ],
        ),
        (
            "--git-staged",
            vec!["--git-staged", "--repo", repo, "--rank"],
        ),
        (
            "--git-range",
            vec!["--git-range", "HEAD..HEAD", "--repo", repo, "--rank"],
        ),
        ("--repo", vec!["--git-staged", "--repo", repo]),
        (
            "--acquisition-json",
            vec!["--git-staged", "--repo", repo, "--acquisition-json"],
        ),
        ("--socket", vec!["daemon", "path", "--socket", socket]),
        (
            "--foreground",
            // `start` alone takes it; an unbindable socket ends the start at once (exit 6).
            vec![
                "daemon",
                "start",
                "--foreground",
                "--socket",
                "/dev/null/d.sock",
            ],
        ),
        (
            "--log-format",
            vec![
                "daemon",
                "start",
                "--log-format",
                "json",
                "--socket",
                "/dev/null/d.sock",
            ],
        ),
        (
            "--format",
            // `metrics` alone takes it; no daemon answers here (exit 6).
            vec![
                "daemon",
                "metrics",
                "--format",
                "prometheus",
                "--socket",
                socket,
            ],
        ),
        (
            "--out",
            vec![
                "daemon",
                "metrics",
                "--format",
                "prometheus",
                "--out",
                "/dev/null/ddf.prom",
                "--socket",
                socket,
            ],
        ),
        ("--stdin", vec!["learn", "record", "--stdin"]),
        (
            "--quick",
            vec!["bench", "--quick", "--workload", "structural_rust"],
        ),
        (
            "--workload",
            vec!["bench", "--quick", "--workload", "findings_verify"],
        ),
        ("--corpus", vec!["eval", "--corpus", "/nonexistent"]),
        ("--family", vec!["explain", "7", "--family", "engine"]),
        ("--color", vec!["highlight", source, "--color"]),
        ("--no-color", vec!["highlight", source, "--no-color"]),
        ("--probe", vec!["review", "--probe"]),
        ("--side", vec!["review", "--probe", "--side"]),
        ("--palette", vec!["review", "--probe", "--palette"]),
        ("--cmd", vec!["review", "--probe", "--cmd", "rank"]),
        ("--cols", vec!["review", "--probe", "--cols", "80"]),
        ("--rows", vec!["review", "--probe", "--rows", "20"]),
        ("--help", vec!["--help"]),
        ("--version", vec!["--version"]),
        ("--self-test", vec!["--self-test"]),
    ]
}

/// A file `name` holding `text` in `dir` (created as needed), by path.
fn scratch(dir: &std::path::Path, name: &str, text: &str) -> String {
    std::fs::create_dir_all(dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, text).unwrap();
    path.to_str().unwrap().to_owned()
}

/// `rows` names exactly the options `--help` documents, and each invocation is admitted.
fn admits_every_documented_option(rows: &[(&str, Vec<&str>)]) {
    let (_, help, _) = run(&["--help"], "");
    let (current, _) = help.split_once("WITHDRAWN").unwrap();
    let mut documented: Vec<&str> = current
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
        .filter(|word| word.starts_with("--") && word.len() > 2)
        .collect();
    documented.sort_unstable();
    documented.dedup();
    let mut covered: Vec<&str> = rows.iter().map(|(option, _)| *option).collect();
    covered.sort_unstable();
    assert_eq!(
        documented, covered,
        "each option in --help needs a row here"
    );
    for (option, args) in rows {
        let (code, _, stderr) = run(args, PATCH);
        assert_ne!(code, 2, "{option} via {args:?} was refused: {stderr}");
    }
}

/// PATCH's one hunk address, and a first-page outline cursor for it: the seal's first 16 hex
/// digits, the selection's digest, then the item and line to resume at.
fn view_handles() -> (String, String) {
    let (_, outline, _) = run(&["--stdin-patch", "--outline"], PATCH);
    let outline: serde_json::Value = serde_json::from_str(&outline).unwrap();
    let address = outline["files"][0]["hunks"][0]["address"]
        .as_str()
        .unwrap()
        .to_owned();
    let seal = deep_diff_forge_core::sha256_hex(PATCH.as_bytes());
    let cursor = format!(
        "{}.{}.0.0",
        &seal[..16],
        &deep_diff_forge_core::sha256_hex(b"outline")[..8]
    );
    (address, cursor)
}

/// Anchor E-02: help listed `--git`, `review` and `daemon start` as future modes after they
/// shipped, and then `<old> <new>`, `chain`, `cluster` and `loom` as "not yet implemented" with no
/// owner. Help now lists no planned mode; each withdrawn one is refused with exit 2 and names what
/// serves instead, so none reads as a path pair or an unknown word.
#[test]
fn help_lists_no_planned_mode_and_each_withdrawn_one_is_refused() {
    let (_, help, _) = run(&["--help"], "");
    assert!(!help.contains("NOT YET IMPLEMENTED"), "{help}");
    let (_, withdrawn) = help.split_once("WITHDRAWN").unwrap();
    let entries: Vec<(&str, &str)> = withdrawn
        .lines()
        .filter_map(|line| line.trim().strip_prefix("deep-diff-forge "))
        .map(|mode| mode.split_once(": ").unwrap())
        .collect();
    let names: Vec<&str> = entries.iter().map(|(name, _)| *name).collect();
    assert_eq!(names, ["chain", "cluster", "loom"], "{withdrawn}");
    for (name, instead) in entries {
        // A second word would otherwise make it a direct comparison of two paths.
        for args in [vec![name], vec![name, "other"]] {
            let (code, stdout, stderr) = run(&args, "");
            assert_eq!(code, 2, "{args:?} was not refused: {stderr}");
            assert_eq!(stdout, "", "{args:?}");
            // Exactly its own replacement, the one help lists.
            assert_eq!(
                stderr,
                format!("error: `{name}` was withdrawn; {instead}\n"),
                "{args:?}"
            );
        }
    }
}

/// `loom-contract` records the withdrawal instead of promising phases nobody owns.
#[test]
fn loom_contract_records_the_withdrawal() {
    let (code, stdout, _) = run(&["loom-contract"], "");
    assert_eq!(code, 0);
    assert!(
        stdout.starts_with("deep-diff-forge loom-contract v0\n"),
        "{stdout}"
    );
    assert!(
        stdout.contains("withdrawn. No loom runner ships and none is planned;"),
        "{stdout}"
    );
    assert!(!stdout.contains("PLANNED"), "{stdout}");
}

/// Bug hunt 2026-11-01: a `--help` or `-h` after envelope's `--` belongs to the wrapped command.
/// Before, envelope printed its own help as plain text with exit 0 and ran nothing.
#[test]
fn envelope_wraps_the_wrapped_command_s_help() {
    let (code, v) = envelope(&["envelope", "--", "--stdin-patch", "--help"], "");
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["schema"], "ddf.result.v1");
    assert_eq!(v["command"], serde_json::json!(["--stdin-patch", "--help"]));
    assert!(
        v["stdout"]
            .as_str()
            .unwrap()
            .starts_with("Review a unified or Git patch read from stdin."),
        "{v}"
    );
    let (code, v) = envelope(&["envelope", "--", "council", "-h"], "");
    assert_eq!(code, 0, "{v}");
    assert_eq!(
        (v["family"].as_str(), v["outcome"].as_str()),
        (Some("council"), Some("completed"))
    );
}

/// Bug hunt 2026-11-01: a wrapped command ended by signal N makes envelope exit 128 + N, the
/// status a shell reports for it. Before, it exited 1, a code the registry does not list.
#[cfg(target_os = "linux")]
#[test]
fn envelope_exits_128_plus_the_signal_that_ended_the_command() {
    let mut envelope = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .args(["envelope", "--", "mcp"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // `mcp` serves until stdin closes; the open pipe keeps it alive until it is killed.
    let children = format!("/proc/{0}/task/{0}/children", envelope.id());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let child = loop {
        let listed = std::fs::read_to_string(&children).unwrap_or_default();
        if let Some(pid) = listed.split_whitespace().next() {
            break pid.to_owned();
        }
        assert!(std::time::Instant::now() < deadline, "no child appeared");
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    let killed = Command::new("kill")
        .args(["-TERM", &child])
        .status()
        .unwrap();
    assert!(killed.success());
    let stdin = envelope.stdin.take();
    let out = envelope.wait_with_output().unwrap();
    drop(stdin);
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        (v["outcome"].as_str(), v["signal"].as_i64()),
        (Some("signal"), Some(15)),
        "{v}"
    );
    assert_eq!(out.status.code(), Some(128 + 15));
}

/// The front page stays short and the documents it hands over stay linked (the landing tidy of
/// 2026-11-18): `README.md` is under 400 lines and links both indexes; every relative link, inline
/// or reference-style, in `README.md`, `CHANGELOG.md`, `docs/release-notes/*.md` and
/// `docs/reference/*.md` stays inside the repository, names a file that exists and, with a
/// fragment, a heading of that file; every document in the two directories but their
/// `README.md` indexes is linked from `README.md`, `CHANGELOG.md` or an index; and the version
/// headings of the changelog and the files of `docs/release-notes/` but its index correspond
/// both ways, each file non-empty and beginning with a line that names its version as a whole
/// word. It does not check external URLs, or whether a host renders an anchor differently from
/// the rule of `headings`; and it takes a link inside an HTML comment, or a reference definition
/// nothing uses, for a link. Four consequences, observed in its review (2026-11-22) and kept:
/// `docs/release-notes/` holds release notes only, since any other Markdown file there fails as a
/// version with no changelog heading; a first line that joins punctuation to the version
/// (`v0.9.0`, `(0.9.0)`, `0.9.0,`) fails, because the version must be a whole word; an index is
/// recognised only by the exact name `README.md`; and a protocol-relative target (`//host/x`) has
/// no scheme, so it is read as a path under the root and fails as naming no file.
#[test]
fn the_front_page_stays_short_and_its_documents_stay_linked() {
    use std::path::{Path, PathBuf};
    // Fragments no heading defines: `#top` is the top of any page.
    const NO_HEADING: [&str; 1] = ["top"];
    let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
        .canonicalize()
        .unwrap();
    let readme = std::fs::read_to_string(root.join("README.md")).unwrap();
    let lines = readme.lines().count();
    assert!(lines < 400, "README.md has {lines} lines");
    let mut files = vec![root.join("README.md"), root.join("CHANGELOG.md")];
    for dir in ["docs/release-notes", "docs/reference"] {
        let mut listed: Vec<PathBuf> = std::fs::read_dir(root.join(dir))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "md"))
            .collect();
        listed.sort();
        assert!(!listed.is_empty(), "{dir} has no documents");
        files.extend(listed);
    }
    let mut anchors = std::collections::BTreeMap::new();
    let mut from_readme = std::collections::BTreeSet::new();
    let mut from_front = std::collections::BTreeSet::new();
    let front = [
        "README.md",
        "CHANGELOG.md",
        "docs/reference/README.md",
        "docs/release-notes/README.md",
    ]
    .map(|path| root.join(path));
    let mut checked = 0;
    for file in &files {
        let text = std::fs::read_to_string(file).unwrap();
        let mut fence = false;
        for (n, line) in text.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
                fence = !fence;
                continue;
            }
            if fence {
                continue;
            }
            for target in relative_links(line) {
                let at = format!("{}:{}: {target}", file.display(), n + 1);
                let (path, fragment) = target.split_once('#').unwrap_or((target, ""));
                let resolved = if path.is_empty() {
                    file.clone()
                } else {
                    let dir = file.parent().unwrap();
                    inside(&root, dir, path).unwrap_or_else(|| panic!("{at} leaves the repository"))
                };
                checked += 1;
                assert!(resolved.exists(), "{at} names no file");
                if *file == root.join("README.md") {
                    from_readme.insert(resolved.clone());
                }
                if front.contains(file) {
                    from_front.insert(resolved.clone());
                }
                let markdown = resolved.extension().is_some_and(|ext| ext == "md");
                if fragment.is_empty() || !markdown || NO_HEADING.contains(&fragment) {
                    continue;
                }
                let known = anchors
                    .entry(resolved.clone())
                    .or_insert_with(|| headings(&std::fs::read_to_string(&resolved).unwrap()));
                assert!(known.contains(fragment), "{at}: no heading {fragment}");
            }
        }
    }
    assert!(checked > 100, "only {checked} relative links were found");
    for index in ["docs/reference/README.md", "docs/release-notes/README.md"] {
        assert!(
            from_readme.contains(&root.join(index)),
            "README.md does not link {index}"
        );
    }
    for file in files.iter().skip(2).filter(|file| !front.contains(file)) {
        assert!(
            from_front.contains(file),
            "{} is linked from no index",
            file.display()
        );
    }
    release_notes_match_the_changelog(&root);
}

/// The version headings of `CHANGELOG.md` and the Markdown files of `docs/release-notes/` but its
/// `README.md` correspond both ways; each such file is non-empty and its first line names its
/// version as a whole word (`10.9.0` does not name `0.9.0`).
fn release_notes_match_the_changelog(root: &std::path::Path) {
    let changelog = std::fs::read_to_string(root.join("CHANGELOG.md")).unwrap();
    let versions: Vec<&str> = changelog
        .lines()
        .filter_map(|line| line.strip_prefix("## [")?.split(']').next())
        .filter(|version| *version != "Unreleased")
        .collect();
    assert!(!versions.is_empty(), "the changelog names no version");
    for version in &versions {
        let notes = root.join(format!("docs/release-notes/{version}.md"));
        assert!(
            notes.is_file(),
            "{version} has no file in docs/release-notes/"
        );
    }
    for entry in std::fs::read_dir(root.join("docs/release-notes")).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let Some(version) = name.strip_suffix(".md").filter(|stem| *stem != "README") else {
            continue;
        };
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.trim().is_empty(), "{name} is empty");
        let first = text.lines().next().unwrap_or("");
        assert!(
            first.split_whitespace().any(|word| word == version),
            "{name} begins {first:?}"
        );
        assert!(
            versions.contains(&version),
            "{name} has no heading in the changelog"
        );
    }
}

/// The path a link names from `dir`, resolved by its components under `root`, a leading `/`
/// meaning `root` as it does on the hosts; `None` when it climbs out of `root`.
fn inside(root: &std::path::Path, dir: &std::path::Path, path: &str) -> Option<std::path::PathBuf> {
    use std::path::{Component, Path};
    let mut parts: Vec<_> = dir.strip_prefix(root).ok()?.components().collect();
    for part in Path::new(path).components() {
        match part {
            Component::ParentDir => {
                parts.pop()?;
            }
            Component::Normal(_) => parts.push(part),
            Component::RootDir => parts.clear(),
            _ => {}
        }
    }
    Some(
        parts
            .iter()
            .fold(root.to_path_buf(), |path, part| path.join(part)),
    )
}

/// The anchors of a Markdown text: each heading by the slug rule GitHub and GitLab use (lower
/// case; letters, digits, `-` and `_` kept, a space made `-`, the rest dropped; a repeated slug
/// gains `-1`, `-2`, ...), and each `<a id="...">` or `<a name="...">`. The rule was checked
/// against GitLab's rendering on 2026-11-18: `Reconciled score: 86 / 100` is
/// `reconciled-score-86--100`, both hyphens kept.
fn headings(text: &str) -> std::collections::BTreeSet<String> {
    let mut out = std::collections::BTreeSet::new();
    let mut fence = false;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fence = !fence;
            continue;
        }
        let level = line.chars().take_while(|&c| c == '#').count();
        if !fence && (1..=6).contains(&level) && line[level..].starts_with(' ') {
            let slug: String = line[level..]
                .trim()
                .to_lowercase()
                .chars()
                .filter(|&c| c.is_alphanumeric() || matches!(c, '-' | '_' | ' '))
                .map(|c| if c == ' ' { '-' } else { c })
                .collect();
            let mut unique = slug.clone();
            for n in 1.. {
                if !out.contains(&unique) {
                    break;
                }
                unique = format!("{slug}-{n}");
            }
            out.insert(unique);
        }
        for attribute in ["<a id=\"", "<a name=\""] {
            for part in line.split(attribute).skip(1) {
                out.insert(part.split('"').next().unwrap().to_owned());
            }
        }
    }
    out
}

/// The relative link targets of one line of Markdown, outside inline code: `](target)`,
/// `src="target"`, `href="target"` and a reference definition `[label]: target`, each without
/// a scheme.
fn relative_links(line: &str) -> Vec<&str> {
    let mut found = Vec::new();
    if let Some((_, target)) = line
        .trim_start()
        .strip_prefix('[')
        .and_then(|rest| rest.split_once("]: "))
    {
        found.push(target.split_whitespace().next().unwrap_or(""));
    }
    for part in line.split('`').step_by(2) {
        for (open, close) in [("](", ')'), ("src=\"", '"'), ("href=\"", '"')] {
            let mut rest = part;
            while let Some(at) = rest.find(open) {
                rest = &rest[at + open.len()..];
                let end = rest.find(close).unwrap_or(rest.len());
                found.push(rest[..end].split_whitespace().next().unwrap_or(""));
                rest = &rest[end..];
            }
        }
    }
    found.retain(|target| {
        let scheme = target.split_once(':').is_some_and(|(scheme, _)| {
            !scheme.is_empty()
                && scheme
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c))
        });
        !target.is_empty() && !scheme
    });
    found
}

/// No tracked file holds a link in the `obsidian` URI scheme (the scheme name followed by
/// `://`), the scheme that opens the owner's private Obsidian vault, which no reader of the
/// repository can follow (S3.17b.a, N72). Every file `git ls-files` lists is read as bytes from
/// the working tree, so an edited tracked file is read as edited; an untracked file is not read,
/// and a symbolic link is read as its target's name. Any line holding the scheme fails the test,
/// named by file and line. The test says nothing about absolute paths of the owner's machine.
/// Where the crate is not in its Git checkout (an unpacked or vendored crate: no `.git` beside
/// `crates/deep-diff-forge-cli` two directories up), or no `git` program exists, there is no list
/// of tracked files: the test says so on stderr and passes. In the checkout it fails unless
/// `git ls-files` succeeds and lists this test's own file, so it cannot pass on an empty list.
#[test]
fn no_tracked_file_links_the_private_vault() {
    // Assembled from two parts, so that this file does not hold what it forbids.
    let scheme = ["obsid", "ian://"].concat();
    let root = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
    let own = "crates/deep-diff-forge-cli/tests/contract.rs";
    if !root.join(".git").exists() || !root.join(own).is_file() {
        eprintln!(
            "{} is not this crate's Git checkout: no tracked files to read",
            root.display()
        );
        return;
    }
    let listing = match Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-files", "-z"])
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .stdin(Stdio::null())
        .output()
    {
        Ok(output) => output,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("no git program: no tracked files to read");
            return;
        }
        Err(error) => panic!("git ls-files: {error}"),
    };
    assert!(
        listing.status.success(),
        "git ls-files: {}",
        String::from_utf8_lossy(&listing.stderr)
    );
    let names: Vec<String> = listing
        .stdout
        .split(|&byte| byte == 0)
        .filter(|name| !name.is_empty())
        .map(|name| String::from_utf8_lossy(name).into_owned())
        .collect();
    assert!(
        names.iter().any(|name| name == own),
        "git ls-files lists {} files but not {own}",
        names.len()
    );
    let mut found = Vec::new();
    for name in &names {
        let path = root.join(name);
        let bytes = match std::fs::symlink_metadata(&path) {
            Ok(meta) if meta.file_type().is_symlink() => std::fs::read_link(&path)
                .unwrap()
                .into_os_string()
                .into_encoded_bytes(),
            Ok(meta) if meta.is_file() => std::fs::read(&path).unwrap(),
            // Deleted in the working tree, or a submodule's directory.
            _ => continue,
        };
        for (n, line) in bytes.split(|&byte| byte == b'\n').enumerate() {
            if line
                .windows(scheme.len())
                .any(|window| window == scheme.as_bytes())
            {
                found.push(format!("{name}:{}", n + 1));
            }
        }
    }
    assert!(
        found.is_empty(),
        "lines of tracked files that link the private vault ({scheme}): {}: {}{}",
        found.len(),
        found
            .iter()
            .take(40)
            .cloned()
            .collect::<Vec<_>>()
            .join(", "),
        if found.len() > 40 { ", ..." } else { "" }
    );
}

/// Run the binary with `stdin` written to it and the given stdout and stderr; its exit code.
fn exit_with(args: &[&str], stdin: &[u8], stdout: Stdio, stderr: Stdio) -> i32 {
    let mut child = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(stdout)
        .stderr(stderr)
        .spawn()
        .unwrap();
    // A command that exits before reading all of stdin closes the pipe first; not a failure.
    let _ = child.stdin.take().unwrap().write_all(stdin);
    child.wait().unwrap().code().unwrap_or(-1)
}

/// S3.17a.g: a diagnostic that cannot be written does not change the exit code. std's
/// `eprintln!` panicked on a failed stderr write, and the panic exited 101: `no-such-command
/// 2>/dev/full` exited 101, not 2.
#[test]
fn an_unwritable_error_stream_keeps_the_exit_code() {
    let full = || std::fs::OpenOptions::new().write(true).open("/dev/full");
    if full().is_err() {
        return; // no /dev/full on this platform (Linux has it; the gate runs on Linux)
    }
    let over_budget = vec![b'x'; deep_diff_forge_patch::DEFAULT_BYTE_BUDGET + 1];
    for (args, stdin, code) in [
        (&["no-such-command"][..], &b""[..], 2),
        // an option refused in another module of the binary (`args.rs`)
        (&["--stdin-patch", "--no-such-option"][..], &b""[..], 2),
        (&["--stdin-patch"][..], &over_budget[..], 4),
    ] {
        let stderr = Stdio::from(full().unwrap());
        assert_eq!(
            exit_with(args, stdin, Stdio::null(), stderr),
            code,
            "{args:?} with stderr at /dev/full"
        );
    }
}

/// Run the binary with stdout on a pipe whose reader has already gone; its exit code and stderr.
fn run_with_stdout_closed(args: &[&str], stdin: &[u8]) -> (Option<i32>, String) {
    let (reader, writer) = std::io::pipe().unwrap();
    drop(reader);
    let mut child = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::from(writer))
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let _ = child.stdin.take().unwrap().write_all(stdin);
    let out = child.wait_with_output().unwrap();
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// S3.17a.g2 (DDF-2026-010): every `--require-*` refusal exits 7 with its reason when the reader
/// of stdout has gone. `findings verify --require-findings` and `--require-all` wrote the verdict
/// first and treated the broken pipe as a clean exit 0 with an empty stderr, so a gate that
/// closed its reader early passed; `--require-files` and `--require-hunks` refuse before writing.
#[test]
fn a_pending_refusal_survives_a_closed_stdout() {
    let scratch = deep_diff_forge_testkit::Scratch::new("pending-refusal");
    let empty = scratch.write("empty.jsonl", "");
    let refused = scratch.write(
        "refused.jsonl",
        "{\"agent\": \"codex\", \"severity\": \"low\", \"title\": \"u\", \"body\": \"c\", \"path\": \"nope\"}\n",
    );
    let (empty, refused) = (empty.to_str().unwrap(), refused.to_str().unwrap());
    let no_files = "just some prose that is not a diff\n";
    let header_only = "diff --git a/x b/x\nold mode 100644\nnew mode 100755\n";
    for (args, stdin, reason) in [
        (
            &["--stdin-patch", "--json", "--require-files"][..],
            no_files,
            "refused: 0 files in input (--require-files)\n",
        ),
        (
            &["--stdin-patch", "--json", "--require-hunks"][..],
            header_only,
            "refused: 0 hunks in input (--require-hunks)\n",
        ),
        (
            &[
                "findings",
                "verify",
                "--findings",
                empty,
                "--require-findings",
            ][..],
            PATCH,
            "refused: 0 findings in the findings file (--require-findings)\n",
        ),
        (
            &["findings", "verify", "--findings", refused, "--require-all"][..],
            PATCH,
            "",
        ),
    ] {
        // The same refusal with stdout open, and with its reader gone.
        let (code, stdout, stderr) = run(args, stdin);
        assert_eq!(
            (code, stderr.as_str()),
            (7, reason),
            "{args:?} open: {stdout}"
        );
        let (code, stderr) = run_with_stdout_closed(args, stdin.as_bytes());
        assert_eq!(
            (code, stderr.as_str()),
            (Some(7), reason),
            "{args:?} with the reader of stdout gone"
        );
    }
}

/// S3.8.d (F6): a status line written before a tag ("0.9.0 prepared", "superseded once `v0.9.0`
/// is tagged") is false from the tag on, and the class came back with every release. These
/// documents say what they describe, so no line of theirs waits on a tag.
/// `docs/RELEASE_AND_PUBLICATION.md` joins the list once its status heading does.
#[test]
fn status_lines_hold_after_the_tag() {
    const STALE: [&str; 9] = [
        " prepared)",
        "is tagged",
        "once tagged",
        "not yet tagged",
        "until tagged",
        "will be tagged",
        "to be tagged",
        "not yet released",
        "not yet published",
    ];
    let mut found = Vec::new();
    for doc in [
        "SECURITY.md",
        "docs/RELEASE_AND_PUBLICATION.md",
        "README.md",
        "CONTRIBUTING.md",
        "docs/OPERATIONS_AND_DAEMON.md",
        "docs/reference/supported-platforms.md",
    ] {
        let path = format!("{}/../../{doc}", env!("CARGO_MANIFEST_DIR"));
        let text = std::fs::read_to_string(&path).unwrap();
        let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
        let flat = flat.to_lowercase();
        found.extend(
            STALE
                .iter()
                .filter(|stale| flat.contains(*stale))
                .map(|stale| format!("{doc}: {stale:?}")),
        );
    }
    assert!(
        found.is_empty(),
        "a status line that goes stale at the tag; say what the document describes: {found:?}"
    );
}

/// The repository's text file at `path`, relative to its root.
fn repo_text(path: &str) -> String {
    std::fs::read_to_string(format!("{}/../../{path}", env!("CARGO_MANIFEST_DIR")))
        .unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// The rows of the Markdown table under `header` in `text`, split into trimmed cells.
fn table_rows(text: &str, header: &str) -> Vec<Vec<String>> {
    let table = &text[text.find(header).unwrap_or_else(|| panic!("{header}"))..];
    table
        .lines()
        .skip(2)
        .take_while(|l| l.starts_with("| "))
        .map(|l| {
            l.trim_matches('|')
                .split(" | ")
                .map(|cell| cell.trim().to_owned())
                .collect()
        })
        .collect()
}

/// F7.h1: the reason classes of exit 7 are one registry. The table "Refusal reasons (exit 7)" of
/// `docs/reference/output-and-exit-codes.md`, `schema exit-codes`, `explain 7 --json` and the MCP
/// tools `explain_exit_code` (code 7) and `exit_code_registry` list the same classes in the same
/// order, with the same tag, commands, stdout, action and release; each documented reason line is
/// `refused: <text> (<tag>)`.
#[test]
fn refusal_reasons_match_the_registry() {
    let (code, stdout, _) = run(&["schema", "exit-codes"], "");
    assert_eq!(code, 0);
    let schema: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let registry = schema["refusals"]
        .as_array()
        .expect("schema exit-codes has refusals");
    assert!(!registry.is_empty());
    let (code, stdout, _) = run(&["explain", "7", "--json"], "");
    assert_eq!(code, 0);
    let explained: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(
        &explained["refusals"], &schema["refusals"],
        "explain 7 --json"
    );
    let (code, stdout, _) = run(&["explain", "4", "--json"], "");
    assert_eq!(code, 0);
    assert!(serde_json::from_str::<serde_json::Value>(&stdout).unwrap()["refusals"].is_null());
    let lines = [
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"contract","version":"1"}}}"#,
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"explain_exit_code","arguments":{"code":7}}}"#,
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"exit_code_registry","arguments":{}}}"#,
    ];
    let (code, stdout, stderr) = run(&["mcp"], &format!("{}\n", lines.join("\n")));
    assert_eq!(code, 0, "{stderr}");
    let answers: Vec<serde_json::Value> = stdout
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    for id in [2, 3] {
        let answer = answers.iter().find(|a| a["id"] == id).unwrap();
        assert_eq!(
            &answer["result"]["structuredContent"]["refusals"], &schema["refusals"],
            "MCP answer {id}"
        );
    }
    let rows = table_rows(
        &repo_text("docs/reference/output-and-exit-codes.md"),
        "| Class | Tag | Commands | Stdout | Next | Since | Reason line |",
    );
    let names: Vec<&str> = registry
        .iter()
        .map(|r| r["class"].as_str().unwrap())
        .collect();
    for (row, entry) in rows.iter().zip(registry) {
        let class = entry["class"].as_str().unwrap();
        let cell = |name: &str| format!("`{}`", entry[name].as_str().unwrap());
        assert_eq!(
            row[..6],
            [
                cell("class"),
                cell("tag"),
                entry["commands"].as_str().unwrap().to_owned(),
                cell("stdout"),
                cell("action"),
                entry["since"].as_str().unwrap().to_owned(),
            ],
            "the documented row of the class {class}"
        );
        let line = row[6].trim_matches('`');
        assert!(
            line.starts_with("refused: ")
                && line.ends_with(&format!(" ({})", entry["tag"].as_str().unwrap())),
            "{class}: {line}"
        );
    }
    let documented: Vec<&str> = rows.iter().map(|r| r[0].trim_matches('`')).collect();
    assert_eq!(
        documented, names,
        "the documented classes and the registry's"
    );
}

/// F7.h1: the codes of `deep-diff-forge.diagnostics.v0`. `fixtures/diagnostics/codes.v0.txt` (it
/// only grows) equals the code table of `docs/reference/diagnostics.md`, line for line; every line
/// is `<label>@line` or `<label>@file`; the table names every label of `schema signals` once, with
/// its class, and every label has an `@file` code.
#[test]
fn diagnostics_codes_are_pinned_and_documented() {
    let pinned = repo_text("fixtures/diagnostics/codes.v0.txt");
    let pinned: Vec<&str> = pinned.lines().collect();
    let doc = repo_text("docs/reference/diagnostics.md");
    let rows = table_rows(&doc, "| Label | Class | Codes |");
    let mut documented = Vec::new();
    for row in &rows {
        for code in row[2].split(", ") {
            let anchor = code.trim_matches('`');
            documented.push(format!("{}{anchor}", row[0].trim_matches('`')));
        }
    }
    assert_eq!(
        pinned, documented,
        "codes.v0.txt and the table of diagnostics.md"
    );
    let (code, stdout, _) = run(&["schema", "signals"], "");
    assert_eq!(code, 0);
    let signals: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let signals: Vec<(String, String)> = signals["signals"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| {
            (
                s["label"].as_str().unwrap().to_owned(),
                s["class"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    for line in &pinned {
        let (label, anchor) = line
            .split_once('@')
            .unwrap_or_else(|| panic!("{line}: not <label>@<anchor>"));
        assert!(
            ["line", "file"].contains(&anchor) && signals.iter().any(|(l, _)| l == label),
            "{line}: not <label>@line or <label>@file of a known label"
        );
    }
    let mut labels: Vec<(String, String)> = rows
        .iter()
        .map(|r| (r[0].trim_matches('`').to_owned(), r[1].clone()))
        .collect();
    let mut known = signals.clone();
    labels.sort();
    known.sort();
    assert_eq!(
        labels, known,
        "the labels and classes of the code table and of schema signals"
    );
    for (label, _) in &signals {
        assert!(
            pinned.contains(&format!("{label}@file").as_str()),
            "{label} has no @file code"
        );
    }
}

/// F7.h1: no action text reachable from exit 7 advises dropping the contract that refused. The
/// two definitions are pinned byte for byte in `docs/reference/output-and-exit-codes.md` and in
/// the `NextAction` doc comments; every reason class carries the action of this table, and none
/// says `fix_invocation`.
#[test]
fn action_texts_reachable_from_exit_7_are_pinned() {
    const CHANGE_INPUT: &str = "change the input or the state the message names (a stale admission, an existing artifact); the same input is refused again.";
    const REVIEW_EVIDENCE: &str = "read the reason and the evidence the message or the council reported; a person decides what changes; rerunning does not change it.";
    let doc = repo_text("docs/reference/output-and-exit-codes.md");
    // The action list, each item joined across its wrapped lines.
    let mut items: Vec<String> = Vec::new();
    for line in doc.lines() {
        if line.starts_with("- `") {
            items.push(line.to_owned());
        } else if let (Some(last), true) = (items.last_mut(), line.starts_with("  ")) {
            last.push(' ');
            last.push_str(line.trim());
        }
    }
    let item = |name: &str| {
        items
            .iter()
            .find_map(|i| i.strip_prefix(&format!("- `{name}`: ")))
            .unwrap_or_else(|| panic!("no action item {name}"))
            .to_owned()
    };
    assert_eq!(item("change_input"), CHANGE_INPUT);
    assert_eq!(item("review_evidence"), REVIEW_EVIDENCE);
    // The doc comments: the sentence starts with a capital there.
    let source = repo_text("crates/deep-diff-forge-core/src/exit.rs");
    let comment = |variant: &str| {
        let before = &source[..source
            .find(&format!("    {variant},\n"))
            .unwrap_or_else(|| panic!("{variant}"))];
        let lines: Vec<&str> = before
            .lines()
            .rev()
            .take_while(|l| l.trim_start().starts_with("///"))
            .collect();
        lines
            .iter()
            .rev()
            .map(|l| l.trim_start().trim_start_matches("///").trim())
            .collect::<Vec<_>>()
            .join(" ")
    };
    let capital = |s: &str| {
        let mut chars = s.chars();
        chars.next().map_or_else(String::new, |c| {
            c.to_ascii_uppercase().to_string() + chars.as_str()
        })
    };
    assert_eq!(comment("ChangeInput"), capital(CHANGE_INPUT));
    assert_eq!(comment("ReviewEvidence"), capital(REVIEW_EVIDENCE));
    let (code, stdout, _) = run(&["schema", "exit-codes"], "");
    assert_eq!(code, 0);
    let schema: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let actions: Vec<(&str, &str)> = schema["refusals"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| (r["class"].as_str().unwrap(), r["action"].as_str().unwrap()))
        .collect();
    assert_eq!(
        actions,
        [
            ("require_files", "change_input"),
            ("require_hunks", "change_input"),
            ("require_findings", "review_evidence"),
            ("require_all", "review_evidence"),
            ("eval_failed", "review_evidence"),
            ("chain_not_ready", "check_environment"),
        ]
    );
    for (class, action) in &actions {
        assert_ne!(*action, "fix_invocation", "{class}");
        let text = item(action);
        assert!(!text.contains("drop the contract"), "{class}: {text}");
    }
}
