// Expert circle 2026-10-28, Tier 3: turnkey agent setup and a parity doctor.
//! `agent setup` prints, never writes, the configuration that registers this binary's MCP server
//! with Claude Code or Codex, and with `--hooks` the Claude Code hooks that run it during a
//! session. `agent doctor` proves, in-process, that the surfaces an agent will use agree: the MCP
//! handshake and tool list, and one sealed document across the CLI path, the daemon's dispatcher
//! and MCP. `agent hook` answers one hook event ([`crate::hook_run`]).

use crate::{emit, mcp};
use deep_diff_forge_core::{display_safe, exit, json_escape};
use serde_json::{Value, json};

const SETUP_USAGE: &str =
    "deep-diff-forge agent setup --client claude|codex [--allow-repo PATH]... [--hooks] [--json]";
const DOCTOR_USAGE: &str = "deep-diff-forge agent doctor [--gate [--judge PATH] [--policy NAME] [--ranker-dir DIR]] [--json]";

/// `agent setup` or `agent doctor`.
pub(crate) fn run(args: &[String]) {
    match args.first().map(String::as_str) {
        Some("setup") => setup(&args[1..]),
        Some("doctor") => doctor(&args[1..]),
        Some("hook") => crate::hook_run::run(&args[1..]),
        _ => {
            eprintln!("usage: {SETUP_USAGE}");
            eprintln!("       {DOCTOR_USAGE}");
            eprintln!("       {}", crate::hook_run::USAGE);
            std::process::exit(exit::USAGE);
        }
    }
}

/// A word the shell passes through unquoted, or the word in single quotes.
fn sh_quote(word: &str) -> String {
    let plain = !word.is_empty()
        && word
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_./-:@%+=,".contains(&b));
    if plain {
        word.to_owned()
    } else {
        format!("'{}'", word.replace('\'', r"'\''"))
    }
}

fn setup(args: &[String]) {
    crate::args::admit_or_exit(
        args,
        &crate::args::Spec {
            flags: &["--json", "--hooks"],
            valued: &["--client", "--allow-repo"],
            positionals: 0,
        },
        SETUP_USAGE,
    );
    let client = crate::flag_value(args, "--client").unwrap_or_default();
    if !matches!(client.as_str(), "claude" | "codex") {
        eprintln!("error: --client must be claude or codex");
        eprintln!("usage: {SETUP_USAGE}");
        std::process::exit(exit::USAGE);
    }
    let hooks = args.iter().any(|a| a == "--hooks");
    if hooks && client != "claude" {
        eprintln!("error: --hooks is for --client claude; hooks for Codex are not designed yet");
        eprintln!("usage: {SETUP_USAGE}");
        std::process::exit(exit::USAGE);
    }
    let binary = std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .unwrap_or_else(|e| {
            eprintln!("error: cannot find this binary's path: {e}");
            std::process::exit(exit::IO);
        });
    let mut server_args = vec!["mcp".to_owned()];
    for root in crate::flag_values(args, "--allow-repo") {
        let canonical = std::fs::canonicalize(&root).unwrap_or_else(|e| {
            eprintln!("error: --allow-repo {}: {e}", display_safe(&root));
            std::process::exit(exit::USAGE);
        });
        server_args.push("--allow-repo".to_owned());
        server_args.push(canonical.to_string_lossy().into_owned());
    }
    let command = binary.to_string_lossy().into_owned();
    // The judge the hooks will run, found now and named by its absolute path.
    let judge = crate::hook_run::judge_program(&[], &[])
        .ok()
        .map(|path| path.to_string_lossy().into_owned());
    if args.iter().any(|a| a == "--json") {
        let mut document = json!({"schema": "deep-diff-forge.agent-setup.v0", "client": client, "name": "deep-diff-forge", "command": command, "args": server_args});
        if hooks {
            document["hooks"] = hook_settings(&command, judge.as_deref())["hooks"].clone();
            document["judge"] = json!(judge);
        }
        emitln!("{document}");
        return;
    }
    if client == "claude" {
        let words: Vec<String> = server_args.iter().map(|a| sh_quote(a)).collect();
        emitln!(
            "# Claude Code: register deep-diff-forge's MCP server for your user (run once):\nclaude mcp add --scope user deep-diff-forge -- {} {}\n\n# Or, for one project, in its .mcp.json:\n{}",
            sh_quote(&command),
            words.join(" "),
            json!({"mcpServers": {"deep-diff-forge": {"command": command, "args": server_args}}})
        );
    } else {
        let toml_args: Vec<String> = server_args.iter().map(|a| json_escape(a)).collect();
        emitln!(
            "# Codex: add to ~/.codex/config.toml\n[mcp_servers.deep-diff-forge]\ncommand = {}\nargs = [{}]",
            json_escape(&command),
            toml_args.join(", ")
        );
    }
    if hooks {
        emitln!(
            "\n# Claude Code hooks: merge into .claude/settings.json (one project) or ~/.claude/settings.json (every project).\n# Nothing is written for you. After a tool call the hook gives advice and never blocks; at a stop it\n# asks forge-warden for a verdict on the working tree and the commits made since the session began, and only a fail holds the stop.\n# {}\n{:#}",
            match &judge {
                Some(judge) => format!(
                    "The judge is named by its path as found now: {}",
                    display_safe(judge)
                ),
                None => "forge-warden was not found on PATH: install it and print these settings again, or add --judge /absolute/path to the session-start and stop commands. Until then a stop is UNJUDGED.".to_owned(),
            },
            hook_settings(&command, judge.as_deref())
        );
    }
    if server_args.len() == 1 {
        eprintln!(
            "note: no --allow-repo: review_change will read no repository; review_outline and review_hunks still work on a patch."
        );
    }
}

/// The `hooks` block of a Claude Code settings file that runs this binary: at the start of a
/// session, after every tool call (no matcher: a shell command can write files too) and at a
/// stop. Each timeout is in seconds; a hook that runs past it is dropped by the client and
/// decides nothing.
///
/// `judge` is the judge found when the settings are printed, by its absolute path. It goes into
/// the commands that use one, so that the program is chosen once by the person who installs the
/// hooks and not at every stop by whatever `PATH` holds then (review V4, VD4-H-1).
fn hook_settings(command: &str, judge: Option<&str>) -> Value {
    let hook = |event: &str, timeout: u64, judged: bool| {
        let judge = match judge {
            Some(judge) if judged => format!(" --judge {}", sh_quote(judge)),
            _ => String::new(),
        };
        json!([{"hooks": [{
            "type": "command",
            "command": format!("{} agent hook {event} --client claude{judge}", sh_quote(command)),
            "timeout": timeout,
        }]}])
    };
    json!({"hooks": {
        "SessionStart": hook("session-start", 10, true),
        "PostToolUse": hook("post-tool", 5, false),
        "Stop": hook("stop", 60, true),
    }})
}

/// The patch every doctor check reviews: two files, one with a section heading.
const PROBE: &str = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 +1,2 @@ fn a()\n fn a() {}\n-fn b() {}\n+fn b() -> u8 { 1 }\ndiff --git a/README.md b/README.md\n--- a/README.md\n+++ b/README.md\n@@ -1 +1 @@\n-x\n+y\n";

struct Check {
    name: &'static str,
    ok: bool,
    detail: String,
}

fn mcp_call(server: &mut mcp::Server, id: u64, name: &str, arguments: &Value) -> Value {
    let line = json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": name, "arguments": arguments}}).to_string();
    server
        .handle_line(&line)
        .map_or(Value::Null, |v| v["result"]["structuredContent"].clone())
}

fn daemon_call(method: &str, params: &Value) -> Value {
    let line = json!({"method": method, "params": params}).to_string();
    deep_diff_forge_daemon::parse_request(&line)
        .and_then(|request| {
            deep_diff_forge_daemon::dispatch(&request, &deep_diff_forge_daemon::Engine::new())
        })
        .unwrap_or(Value::Null)
}

/// MCP answered `initialize` at the protocol version, lists `review_outline`, and every tool
/// declares an object `outputSchema`.
fn handshake_ok(init: Option<&Value>, names: &[&str], schemas: bool) -> bool {
    init.is_some_and(|v| v["result"]["protocolVersion"] == "2025-11-25")
        && names.contains(&"review_outline")
        && schemas
}

/// The CLI's `review.v0` is of the probe, and the daemon and MCP return the same document.
fn review_parity_ok(expected: &str, cli: &Value, daemon: &Value, via_mcp: &Value) -> bool {
    cli["input_sha256"] == expected && cli == daemon && daemon == via_mcp
}

/// The daemon's `rank.v0` is of the probe, and MCP returns the same document.
fn rank_parity_ok(expected: &str, rank: &Value, rank_mcp: &Value) -> bool {
    rank["schema"] == "deep-diff-forge.rank.v0"
        && rank["input_sha256"] == expected
        && rank == rank_mcp
}

/// The outline is of the probe, and the hunk fetched by its address is that hunk, within budget.
fn views_ok(expected: &str, outline: &Value, address: &Value, hunks: &Value) -> bool {
    outline["input_sha256"] == expected
        && hunks["hunks"][0]["address"] == *address
        && hunks.to_string().len() <= 1024
}

fn checks() -> Vec<Check> {
    let mut checks = Vec::new();
    let mut server = mcp::Server::default();
    let init = server.handle_line(&json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "deep-diff-forge-doctor", "version": "1"}}}).to_string());
    let tools = server
        .handle_line(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#)
        .map_or(Value::Null, |v| v["result"]["tools"].clone());
    let names: Vec<&str> = tools
        .as_array()
        .map(|t| t.iter().filter_map(|t| t["name"].as_str()).collect())
        .unwrap_or_default();
    let schemas = tools
        .as_array()
        .is_some_and(|t| t.iter().all(|t| t["outputSchema"]["type"] == "object"));
    checks.push(Check {
        name: "mcp_handshake",
        ok: handshake_ok(init.as_ref(), &names, schemas),
        detail: format!("{} tools: {}", names.len(), names.join(", ")),
    });

    let expected = deep_diff_forge_core::sha256_hex(PROBE.as_bytes());
    let cli = deep_diff_forge_patch::SealedReview::new(
        PROBE.as_bytes(),
        deep_diff_forge_patch::InputGuards {
            files: true,
            hunks: true,
        },
        env!("CARGO_PKG_VERSION"),
    )
    .ok()
    .and_then(|review| serde_json::from_str::<Value>(&review.to_json()).ok())
    .unwrap_or(Value::Null);
    let daemon = daemon_call("diff.compute", &json!({"patch": PROBE}));
    let via_mcp = mcp_call(&mut server, 3, "review_patch", &json!({"patch": PROBE}));
    checks.push(Check {
        name: "review_parity",
        ok: review_parity_ok(&expected, &cli, &daemon, &via_mcp),
        detail: format!("CLI, daemon and MCP review.v0 for {}", &expected[..16]),
    });

    let rank = daemon_call("diff.rank", &json!({"patch": PROBE}));
    let rank_mcp = mcp_call(&mut server, 4, "rank_patch", &json!({"patch": PROBE}));
    checks.push(Check {
        name: "rank_parity",
        ok: rank_parity_ok(&expected, &rank, &rank_mcp),
        detail: "daemon diff.rank equals MCP rank_patch".into(),
    });

    let outline = mcp_call(&mut server, 5, "review_outline", &json!({"patch": PROBE}));
    let address = outline["files"][0]["hunks"][0]["address"].clone();
    let hunks = mcp_call(
        &mut server,
        6,
        "review_hunks",
        &json!({"patch": PROBE, "addresses": [address], "max_bytes": 2048}),
    );
    checks.push(Check {
        name: "views",
        ok: views_ok(&expected, &outline, &address, &hunks),
        detail: "outline then one hunk by address, within budget".into(),
    });
    checks
}

fn doctor(args: &[String]) {
    crate::args::admit_or_exit(
        args,
        &crate::args::Spec {
            flags: &["--json", "--gate"],
            valued: &["--judge", "--policy", "--ranker-dir"],
            positionals: 0,
        },
        DOCTOR_USAGE,
    );
    let mut checks = checks();
    let surfaces_agree = checks.iter().all(|c| c.ok);
    // `--gate`: also every link a judged stop depends on, in this directory's repository.
    if args.iter().any(|a| a == "--gate") {
        let cwd = std::env::current_dir().unwrap_or_else(|e| {
            eprintln!("error: cannot find the current directory: {e}");
            std::process::exit(exit::IO);
        });
        checks.extend(
            crate::hook_run::gate(&cwd, args)
                .into_iter()
                .map(|c| Check {
                    name: c.name,
                    ok: c.ok,
                    detail: c.detail,
                }),
        );
    }
    let ok = checks.iter().all(|c| c.ok);
    if args.iter().any(|a| a == "--json") {
        let list: Vec<Value> = checks
            .iter()
            .map(|c| json!({"name": c.name, "ok": c.ok, "detail": c.detail}))
            .collect();
        emitln!(
            "{}",
            json!({"schema": "deep-diff-forge.agent-doctor.v0", "tool": {"name": "deep-diff-forge", "version": env!("CARGO_PKG_VERSION")}, "ok": ok, "checks": list})
        );
    } else {
        for c in &checks {
            emitln!(
                "{}  {:<22} {}",
                if c.ok { "ok  " } else { "FAIL" },
                c.name,
                c.detail
            );
        }
    }
    if !surfaces_agree {
        eprintln!("error: the agent surfaces disagree; report this as a bug");
        std::process::exit(exit::PANIC);
    }
    if !ok {
        eprintln!(
            "error: the chain is not ready: a stop here would not be judged, or the policy would not hold"
        );
        std::process::exit(exit::INPUT_CONTRACT_REFUSED);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_doctor_check_passes_on_this_build() {
        for check in checks() {
            assert!(check.ok, "{}: {}", check.name, check.detail);
        }
    }

    /// Each condition of each check fails it on its own.
    #[test]
    fn every_check_condition_can_fail_its_check() {
        let init = json!({"result": {"protocolVersion": "2025-11-25"}});
        let old = json!({"result": {"protocolVersion": "2024-11-05"}});
        assert!(handshake_ok(Some(&init), &["review_outline"], true));
        assert!(!handshake_ok(Some(&old), &["review_outline"], true));
        assert!(!handshake_ok(None, &["review_outline"], true));
        assert!(!handshake_ok(Some(&init), &["review_patch"], true));
        assert!(!handshake_ok(Some(&init), &["review_outline"], false));

        let doc = json!({"input_sha256": "e"});
        let other = json!({"input_sha256": "e", "extra": 1});
        assert!(review_parity_ok("e", &doc, &doc, &doc));
        assert!(!review_parity_ok("f", &doc, &doc, &doc));
        assert!(!review_parity_ok("e", &doc, &other, &other));
        assert!(!review_parity_ok("e", &doc, &doc, &other));

        let rank = json!({"schema": "deep-diff-forge.rank.v0", "input_sha256": "e"});
        let wrong = json!({"schema": "deep-diff-forge.review.v0", "input_sha256": "e"});
        let moved = json!({"schema": "deep-diff-forge.rank.v0", "input_sha256": "e", "x": 1});
        assert!(rank_parity_ok("e", &rank, &rank));
        assert!(!rank_parity_ok("e", &wrong, &wrong));
        assert!(!rank_parity_ok("f", &rank, &rank));
        assert!(!rank_parity_ok("e", &rank, &moved));

        let outline = json!({"input_sha256": "e"});
        let address = json!("a1");
        let hunks = json!({"hunks": [{"address": "a1"}]});
        let big = json!({"hunks": [{"address": "a1", "pad": "x".repeat(1100)}]});
        assert!(views_ok("e", &outline, &address, &hunks));
        assert!(!views_ok("f", &outline, &address, &hunks));
        assert!(!views_ok("e", &outline, &json!("a2"), &hunks));
        assert!(!views_ok("e", &outline, &address, &big));
    }

    /// The printed hooks run this binary for the three events, with no matcher on the tool event
    /// (a shell command writes files too) and a timeout on each.
    #[test]
    fn hook_settings_name_three_events_with_timeouts_and_a_quoted_command() {
        let settings = hook_settings("/opt/my tools/deep-diff-forge", None);
        let hooks = settings["hooks"].as_object().unwrap();
        let events: std::collections::BTreeSet<&str> = hooks.keys().map(String::as_str).collect();
        assert_eq!(
            events,
            ["PostToolUse", "SessionStart", "Stop"]
                .into_iter()
                .collect()
        );
        for (event, word, timeout) in [
            ("SessionStart", "session-start", 10),
            ("PostToolUse", "post-tool", 5),
            ("Stop", "stop", 60),
        ] {
            let entry = &hooks[event][0];
            assert!(entry.get("matcher").is_none(), "{event}");
            let hook = &entry["hooks"][0];
            assert_eq!(hook["type"], "command");
            assert_eq!(hook["timeout"], timeout);
            assert_eq!(
                hook["command"],
                format!("'/opt/my tools/deep-diff-forge' agent hook {word} --client claude")
            );
        }
        // a judge found at setup is named by its path in the commands that use one
        let settings = hook_settings("/bin/ddf", Some("/opt/my tools/forge-warden"));
        let command = |event: &str| {
            settings["hooks"][event][0]["hooks"][0]["command"]
                .as_str()
                .unwrap()
                .to_owned()
        };
        for (event, word) in [("SessionStart", "session-start"), ("Stop", "stop")] {
            assert_eq!(
                command(event),
                format!(
                    "/bin/ddf agent hook {word} --client claude --judge '/opt/my tools/forge-warden'"
                )
            );
        }
        assert_eq!(
            command("PostToolUse"),
            "/bin/ddf agent hook post-tool --client claude"
        );
    }

    /// Independent review of the hooks, HR-L-1 and HR-M-4: the timeout the printed settings
    /// give each hook is longer than everything that hook may take, and the stop hook's is at
    /// least what `agent doctor --gate` asks of a registered one. The steps' own limits used
    /// to add up to several times the sixty seconds printed for a stop.
    #[test]
    fn the_printed_timeouts_leave_each_hook_its_time() {
        let settings = hook_settings("/bin/ddf", None);
        let timeout = |event: &str| {
            settings["hooks"][event][0]["hooks"][0]["timeout"]
                .as_u64()
                .unwrap()
        };
        for (event, limit) in crate::hook_run::LIMITS {
            assert!(timeout(event) > limit.as_secs(), "{event}");
            assert!(
                event != "Stop"
                    || u64::from(crate::hook_run::STOP_TIMEOUT_NEEDED) > limit.as_secs(),
                "the gate asks for less than a stop can take"
            );
        }
        assert!(timeout("Stop") >= u64::from(crate::hook_run::STOP_TIMEOUT_NEEDED));
    }

    #[test]
    fn shell_words_are_quoted_only_when_needed() {
        assert_eq!(sh_quote("/usr/bin/ddf"), "/usr/bin/ddf");
        assert_eq!(sh_quote("/a b/ddf"), "'/a b/ddf'");
        assert_eq!(sh_quote("it's"), r"'it'\''s'");
        assert_eq!(sh_quote(""), "''");
    }
}
