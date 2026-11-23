//! `agent setup` prints the MCP registration for this binary and writes nothing; `agent doctor`
//! checks that the agent surfaces agree (expert circle 2026-10-28, Tier 3).
#![expect(
    clippy::disallowed_methods,
    reason = "integration test: drives the binary"
)]

use deep_diff_forge_testkit::Scratch;
use std::process::Command;

fn run(args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .args(args)
        .output()
        .unwrap();
    (
        out.status.code().unwrap(),
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

fn binary() -> String {
    std::fs::canonicalize(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

#[test]
fn setup_prints_each_clients_registration_for_this_binary() {
    let scratch = Scratch::new("agent-setup");
    let root = std::fs::canonicalize(scratch.path()).unwrap();
    let root = root.to_str().unwrap();
    let (code, claude, _) = run(&["agent", "setup", "--client", "claude", "--allow-repo", root]);
    assert_eq!(code, 0);
    assert!(
        claude.contains(&format!(
            "claude mcp add --scope user deep-diff-forge -- {} mcp --allow-repo {root}",
            binary()
        )),
        "{claude}"
    );
    let (code, codex, _) = run(&["agent", "setup", "--client", "codex", "--allow-repo", root]);
    assert_eq!(code, 0);
    assert!(codex.contains("[mcp_servers.deep-diff-forge]"), "{codex}");
    assert!(
        codex.contains(&format!("args = [\"mcp\", \"--allow-repo\", \"{root}\"]")),
        "{codex}"
    );
    // Only a registration with no repository root says that review_change will read none.
    let (_, _, with_root) = run(&["agent", "setup", "--client", "claude", "--allow-repo", root]);
    assert!(!with_root.contains("no --allow-repo"), "{with_root}");
    let (_, _, without) = run(&["agent", "setup", "--client", "claude"]);
    assert!(without.contains("no --allow-repo"), "{without}");
    let (code, json, _) = run(&["agent", "setup", "--client", "codex", "--json"]);
    assert_eq!(code, 0);
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["schema"], "deep-diff-forge.agent-setup.v0");
    assert_eq!(value["command"], binary());
    assert_eq!(value["args"], serde_json::json!(["mcp"]));
}

#[test]
fn setup_refuses_an_unknown_client_or_a_missing_root() {
    assert_eq!(run(&["agent", "setup", "--client", "vim"]).0, 2);
    assert_eq!(run(&["agent", "setup"]).0, 2);
    let (code, _, stderr) = run(&[
        "agent",
        "setup",
        "--client",
        "claude",
        "--allow-repo",
        "/nonexistent/ddf",
    ]);
    assert_eq!(code, 2);
    assert!(stderr.contains("/nonexistent/ddf"));
    assert_eq!(run(&["agent", "teleport"]).0, 2);
}

/// D1 (0.11.0): `agent setup --hooks` takes the judged hooks' options and writes them, by
/// absolute path, into the session-start and stop commands, never into the advice hook's. The
/// 0.10.0 release notes told readers to run it with `--ranker-dir DIR`, which 0.10.0 refused.
#[test]
fn setup_writes_the_judged_hooks_options_into_their_commands() {
    let scratch = Scratch::new("agent-setup-options");
    let rankers = scratch.join("rankers");
    std::fs::create_dir(&rankers).unwrap();
    let judge = scratch.write("judge", "not run by setup\n");
    let (rankers, judge) = (
        std::fs::canonicalize(rankers).unwrap(),
        std::fs::canonicalize(judge).unwrap(),
    );
    let (rankers, judge) = (rankers.to_str().unwrap(), judge.to_str().unwrap());
    let args = [
        "agent",
        "setup",
        "--client",
        "claude",
        "--hooks",
        "--judge",
        judge,
        "--policy",
        "ci/policy.toml",
        "--ranker-dir",
        rankers,
    ];
    let (code, json, stderr) = run(&[&args[..], &["--json"]].concat());
    assert_eq!(code, 0, "{stderr}");
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["judge"], judge);
    let command = |event: &str| value["hooks"][event][0]["hooks"][0]["command"].clone();
    for (event, word) in [("SessionStart", "session-start"), ("Stop", "stop")] {
        assert_eq!(
            command(event),
            format!(
                "{} agent hook {word} --client claude --judge {judge} --policy ci/policy.toml --ranker-dir {rankers}",
                binary()
            ),
            "{event}"
        );
    }
    assert_eq!(
        command("PostToolUse"),
        format!("{} agent hook post-tool --client claude", binary())
    );
    // The text form prints the same stop command.
    let (code, text, stderr) = run(&args);
    assert_eq!(code, 0, "{stderr}");
    assert!(
        text.contains(&format!(
            "--judge {judge} --policy ci/policy.toml --ranker-dir {rankers}"
        )),
        "{text}"
    );
}

/// D1: the judged hooks' options need `--hooks`, a ranker directory that is a directory and a
/// judge that is a file; each is a usage error that names the option, never a hook command
/// that leaves it out.
#[test]
fn setup_refuses_the_judged_options_without_hooks_or_their_path() {
    let scratch = Scratch::new("agent-setup-refusals");
    let dir = scratch.path().to_str().unwrap();
    let file = scratch.write("f", "x\n");
    let file = file.to_str().unwrap();
    for (args, said) in [
        (vec!["--ranker-dir", dir], "add --hooks"),
        (vec!["--policy", "p.toml"], "add --hooks"),
        (
            vec!["--hooks", "--ranker-dir", "/nonexistent/ddf-rankers"],
            "--ranker-dir /nonexistent/ddf-rankers",
        ),
        (vec!["--hooks", "--ranker-dir", file], "not a directory"),
        (
            vec!["--hooks", "--judge", "/nonexistent/ddf-judge"],
            "--judge /nonexistent/ddf-judge: no such file",
        ),
    ] {
        let line = [&["agent", "setup", "--client", "claude"][..], &args].concat();
        let (code, stdout, stderr) = run(&line);
        assert_eq!(code, 2, "{line:?}: {stderr}");
        assert!(stdout.is_empty(), "{line:?}: {stdout}");
        assert!(stderr.contains(said), "{line:?}: {stderr}");
    }
}

/// D1: `agent --help` lists each form with the options that form takes, in the form's own usage
/// line; nothing after the braces claims an option for every form (`agent setup` refused the
/// `--judge` that 0.10.0's line listed for all of them).
#[test]
fn agent_help_lists_each_forms_own_usage() {
    let (code, help, _) = run(&["agent", "--help"]);
    assert_eq!(code, 0);
    let row = help
        .lines()
        .find(|line| line.starts_with("  deep-diff-forge agent {"))
        .unwrap_or_else(|| panic!("no agent row in: {help}"));
    assert!(row.ends_with('}'), "options after the forms: {row}");
    // `agent` with no form prints every form's own usage line.
    let (code, _, usages) = run(&["agent"]);
    assert_eq!(code, 2);
    let forms: Vec<&str> = usages
        .lines()
        .filter_map(|line| {
            line.trim_start_matches("usage:")
                .trim()
                .strip_prefix("deep-diff-forge agent ")
        })
        .collect();
    assert_eq!(forms.len(), 5, "{usages}");
    for form in forms {
        assert!(
            row.contains(&format!("{{{form} |"))
                || row.contains(&format!("| {form} |"))
                || row.contains(&format!("| {form}}}")),
            "the agent row does not list the form as its usage says: {form}\nrow: {row}"
        );
    }
}

/// S3.15.d: the doctor's rank parity is a byte comparison that includes the command line's bytes,
/// not a value comparison of the daemon with MCP (a surface that re-spaces or reorders the
/// document would pass that).
#[test]
fn doctor_rank_parity_compares_the_cli_bytes() {
    let (code, json, stderr) = run(&["agent", "doctor", "--json"]);
    assert_eq!(code, 0, "{stderr}");
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    let rank = value["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "rank_parity")
        .unwrap();
    assert_eq!(rank["ok"], true, "{rank}");
    assert_eq!(
        rank["detail"],
        "CLI --rank --json bytes equal daemon diff.rank canonical_json and MCP rank_patch text"
    );
}

#[test]
fn doctor_reports_every_check_passing() {
    let (code, json, stderr) = run(&["agent", "doctor", "--json"]);
    assert_eq!(code, 0, "{stderr}");
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["schema"], "deep-diff-forge.agent-doctor.v0");
    assert_eq!(value["ok"], true);
    let names: Vec<&str> = value["checks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        ["mcp_handshake", "review_parity", "rank_parity", "views"]
    );
    let (code, human, _) = run(&["agent", "doctor"]);
    assert_eq!(code, 0);
    assert_eq!(human.lines().filter(|l| l.starts_with("ok")).count(), 4);
}

/// F7.h2: a chain that is not ready (here: no Git repository) exits 7 with its `error:` line and
/// then the reason line of the class `chain_not_ready`, after the checks are written.
#[test]
fn doctor_names_the_chain_not_ready_reason_after_its_error_line() {
    let scratch = Scratch::new("doctor-not-ready");
    let out = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .args(["agent", "doctor", "--gate"])
        .current_dir(scratch.path())
        .env("HOME", scratch.path())
        .env_remove("XDG_STATE_HOME")
        .env_remove("CLAUDE_PROJECT_DIR")
        .output()
        .unwrap();
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert_eq!(out.status.code(), Some(7), "{stderr}");
    let lines: Vec<&str> = stderr.lines().collect();
    let error = lines
        .iter()
        .position(|l| l.starts_with("error: the chain is not ready: "));
    let reason = lines
        .iter()
        .position(|l| l.starts_with("refused: ") && l.ends_with(" checks failed (agent doctor)"));
    assert!(
        matches!((error, reason), (Some(e), Some(r)) if e < r),
        "{stderr}"
    );
    assert!(String::from_utf8(out.stdout).unwrap().contains("FAIL"));
}
