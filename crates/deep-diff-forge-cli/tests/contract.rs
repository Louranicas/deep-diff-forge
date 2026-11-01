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
        ("--allow-repo", vec!["rpc", "--allow-repo", repo]),
        ("--client", vec!["agent", "setup", "--client", "codex"]),
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
