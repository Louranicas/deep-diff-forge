//! Slice A: the machine contract (`envelope`, `schema`, `explain`) from the outside.
#![expect(
    clippy::disallowed_methods,
    reason = "integration test: drives real processes, the environment and wall-clock time"
)]

use std::io::Write as _;
use std::process::{Command, Stdio};

const PATCH: &str = "--- a/x\n+++ b/x\n@@ -1,1 +1,1 @@\n-old\n+new\n";

fn run(args: &[&str], stdin: &str) -> (i32, String, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
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

/// The README's exit-code tables and the binary's registry name the same codes, and each README
/// row begins with the registry's meaning.
#[test]
fn readme_exit_code_tables_match_the_registry() {
    let readme =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../README.md")).unwrap();
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
                "the README does not explain the action {}",
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
    assert!(serve.contains("fn recover(") && !serve.contains("fn panic_on_test_method"));
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
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("a.rs");
    std::fs::write(&source, "fn a() {}\n").unwrap();
    let source = source.to_str().unwrap();
    let repo = env!("CARGO_MANIFEST_DIR");
    let socket = dir.join("s.sock");
    let socket = socket.to_str().unwrap();
    let rows: &[(&str, &[&str])] = &[
        ("--json", &["--stdin-patch", "--json"]),
        ("--jsonl", &["--stdin-patch", "--jsonl"]),
        ("--rank", &["--stdin-patch", "--rank"]),
        ("--cluster", &["--stdin-patch", "--cluster"]),
        (
            "--parallel",
            &["--stdin-patch", "--cluster", "--parallel", "2"],
        ),
        ("--layout", &["--stdin-patch", "--layout", "side-by-side"]),
        ("--require-files", &["--stdin-patch", "--require-files"]),
        ("--require-hunks", &["--stdin-patch", "--require-hunks"]),
        ("--stdin-patch", &["--stdin-patch"]),
        (
            "--git-working-tree",
            &["--git-working-tree", "--repo", repo, "--acquisition-json"],
        ),
        (
            "--include-untracked",
            &[
                "--git-working-tree",
                "--include-untracked",
                "--repo",
                repo,
                "--rank",
            ],
        ),
        ("--git-staged", &["--git-staged", "--repo", repo, "--rank"]),
        (
            "--git-range",
            &["--git-range", "HEAD..HEAD", "--repo", repo, "--rank"],
        ),
        ("--repo", &["--git-staged", "--repo", repo]),
        (
            "--acquisition-json",
            &["--git-staged", "--repo", repo, "--acquisition-json"],
        ),
        ("--socket", &["daemon", "path", "--socket", socket]),
        (
            "--foreground",
            &["daemon", "path", "--foreground", "--socket", socket],
        ),
        ("--stdin", &["learn", "record", "--stdin"]),
        ("--family", &["explain", "7", "--family", "engine"]),
        ("--color", &["highlight", source, "--color"]),
        ("--no-color", &["highlight", source, "--no-color"]),
        ("--probe", &["review", "--probe"]),
        ("--side", &["review", "--probe", "--side"]),
        ("--palette", &["review", "--probe", "--palette"]),
        ("--cmd", &["review", "--probe", "--cmd", "rank"]),
        ("--cols", &["review", "--probe", "--cols", "80"]),
        ("--rows", &["review", "--probe", "--rows", "20"]),
        ("--help", &["--help"]),
        ("--version", &["--version"]),
        ("--self-test", &["--self-test"]),
    ];
    let (_, help, _) = run(&["--help"], "");
    let (current, _) = help.split_once("NOT YET IMPLEMENTED").unwrap();
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
    let _ = std::fs::remove_dir_all(&dir);
}

/// Anchor E-02: help listed `--git`, `review` and `daemon start` as future modes after they
/// shipped. Each mode still listed as not implemented must be refused, so implementing one fails
/// here until it moves to USAGE.
#[test]
fn modes_listed_as_not_implemented_are_refused() {
    let (_, help, _) = run(&["--help"], "");
    let (_, planned) = help.split_once("NOT YET IMPLEMENTED").unwrap();
    let planned: Vec<Vec<&str>> = planned
        .lines()
        .filter_map(|line| line.trim().strip_prefix("deep-diff-forge "))
        .map(|mode| {
            mode.split_whitespace()
                .map(|word| if word.starts_with('<') { "x" } else { word })
                .collect()
        })
        .collect();
    assert!(
        planned.contains(&vec!["chain"]),
        "help section not parsed: {planned:?}"
    );
    for args in planned {
        let (code, _, stderr) = run(&args, "");
        assert_eq!(
            code, 2,
            "{args:?} is listed as not implemented but ran: {stderr}"
        );
    }
}
