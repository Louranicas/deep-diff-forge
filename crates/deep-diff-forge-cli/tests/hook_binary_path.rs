//! S3.34.c, the mutant of `hook::sh_quote` (`&&` to `||`, which leaves every word that is not
//! empty unquoted): what the hooks tell the agent to run names this program by its own path,
//! shell-quoted, so that a path with a space is still one word. The mutation run timed it out;
//! no test there could tell it from the original, since the binary's path in a build directory
//! has only plain characters. Here the binary is a copy in a directory whose name has a space.
#![cfg(unix)]
#![expect(
    clippy::disallowed_methods,
    reason = "integration test: drives real processes and temp directories"
)]

use deep_diff_forge_testkit::Scratch;
use serde_json::json;
use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};

const ISOLATE: [(&str, &str); 2] = [
    ("GIT_CONFIG_GLOBAL", "/dev/null"),
    ("GIT_CONFIG_NOSYSTEM", "1"),
];

fn git(root: &Path, args: &[&str]) {
    let out = Command::new("git")
        .current_dir(root)
        .envs(ISOLATE)
        .args([
            "-c",
            "user.name=ddf",
            "-c",
            "user.email=ddf@example.invalid",
        ])
        .args([
            "-c",
            "init.defaultBranch=main",
            "-c",
            "commit.gpgSign=false",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn the_hooks_name_a_binary_whose_path_has_a_space_in_quotes() {
    let scratch = Scratch::new("hook-binary-path");
    let home = scratch.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let root = scratch.join("repo");
    std::fs::create_dir(&root).unwrap();
    git(&root, &["init", "--quiet"]);
    std::fs::write(root.join("a.rs"), "fn a() {}\n").unwrap();
    git(&root, &["add", "--all"]);
    git(&root, &["commit", "--quiet", "-m", "base"]);
    // The copy is written by a child process: a file this process wrote and then ran could be
    // busy (ETXTBSY) while another test forks.
    let dir = scratch.join("bin dir");
    std::fs::create_dir(&dir).unwrap();
    let copy = dir.join("deep-diff-forge");
    let status = Command::new("cp")
        .arg(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .arg(&copy)
        .status()
        .unwrap();
    assert!(status.success());
    let shown = std::fs::canonicalize(&copy).unwrap();
    let shown = shown.to_str().unwrap();
    assert!(shown.contains(' '));
    let quoted = format!("'{}'", shown.replace('\'', r"'\''"));
    let event = json!({
        "hook_event_name": "SessionStart",
        "session_id": "s-binary-path",
        "cwd": root,
        "stop_hook_active": false,
        "tool_name": "Bash",
    });
    let mut child = Command::new(&copy)
        .envs(ISOLATE)
        .env("HOME", &home)
        .env_remove("XDG_STATE_HOME")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("CLAUDE_PROJECT_DIR")
        .args(["agent", "hook", "session-start", "--client", "claude"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(event.to_string().as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        stdout.contains(&format!("`{quoted} agent doctor --gate`")),
        "the hook named a binary whose path has a space without quotes:\n{stdout}"
    );
    assert!(!stdout.contains(&format!("`{shown} agent")), "{stdout}");
}
