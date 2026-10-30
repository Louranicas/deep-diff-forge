//! `review.change` (expert circle 2026-10-28, Tier 1): an agent reviews a change straight from Git
//! without pasting a patch, but only in a repository the operator allowed, and gets what the
//! CLI's Git selections print for the same change.
#![expect(
    clippy::disallowed_methods,
    reason = "integration test: drives git, the binary, temp directories"
)]

use deep_diff_forge_daemon::{
    Engine, GIT_FAILED, INPUT_CONTRACT_REFUSED, INVALID_PARAMS, dispatch, parse_request,
};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;

struct Dir(PathBuf);

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .current_dir(dir)
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("HOME", dir)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

/// A repository with one commit, a `sub/` directory, and a staged change in `src/lib.rs`.
fn repo(name: &str) -> Dir {
    let root =
        std::env::temp_dir().join(format!("ddf-review-change-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::create_dir_all(root.join("sub")).unwrap();
    let dir = Dir(std::fs::canonicalize(&root).unwrap());
    git(&dir.0, &["init", "-q"]);
    std::fs::write(dir.0.join("src/lib.rs"), "fn a() {}\n").unwrap();
    std::fs::write(dir.0.join("sub/x.txt"), "x\n").unwrap();
    git(&dir.0, &["add", "."]);
    git(&dir.0, &["commit", "-qm", "base"]);
    std::fs::write(dir.0.join("src/lib.rs"), "fn a() {}\nfn b() {}\n").unwrap();
    git(&dir.0, &["add", "src/lib.rs"]);
    dir
}

fn call(engine: &Engine, params: &Value) -> Result<Value, deep_diff_forge_daemon::RpcError> {
    let line = json!({"method": "review.change", "params": params}).to_string();
    dispatch(&parse_request(&line)?, engine)
}

fn allowing(roots: &[&Path]) -> Engine {
    let roots: Vec<PathBuf> = roots.iter().map(|r| r.to_path_buf()).collect();
    Engine::new().allowing_repos(&roots).unwrap()
}

#[test]
fn nothing_is_read_unless_the_operator_allows_it() {
    let r = repo("off");
    let err = call(&Engine::new(), &json!({"repo": r.0, "selection": "staged"})).unwrap_err();
    assert_eq!(err.code, INVALID_PARAMS);
    assert!(err.message.contains("--allow-repo"), "{}", err.message);
}

/// Outside every root, through a symlink that leaves the root, or at a subdirectory whose Git top
/// level lies above the root: each is refused, and no change is returned.
#[test]
fn only_repositories_inside_an_allowed_root_are_read() {
    let (a, b) = (repo("a"), repo("b"));
    let engine = allowing(&[&a.0]);
    let outside = call(&engine, &json!({"repo": b.0, "selection": "staged"})).unwrap_err();
    assert!(outside.message.contains("outside"), "{}", outside.message);
    let link = a.0.join("sub/escape");
    std::os::unix::fs::symlink(&b.0, &link).unwrap();
    let escaped = call(&engine, &json!({"repo": link, "selection": "staged"})).unwrap_err();
    assert!(escaped.message.contains("outside"), "{}", escaped.message);
    let narrow = allowing(&[&a.0.join("sub")]);
    let above = call(
        &narrow,
        &json!({"repo": a.0.join("sub"), "selection": "staged"}),
    )
    .unwrap_err();
    assert!(above.message.contains("top level"), "{}", above.message);
}

/// The outline of a staged change is the CLI's `--git-staged --outline` for the same repository,
/// sealed over the same bytes, plus an `acquisition` member naming what was compared.
#[test]
fn a_staged_change_matches_the_cli() {
    let r = repo("staged");
    let engine = allowing(&[&r.0]);
    let mut answer = call(&engine, &json!({"repo": r.0, "selection": "staged"})).unwrap();
    let acquisition = answer
        .as_object_mut()
        .unwrap()
        .remove("acquisition")
        .unwrap();
    assert_eq!(acquisition["selection"], "staged");
    assert_eq!(acquisition["repo"], r.0.to_string_lossy().as_ref());
    let out = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .args(["--git-staged", "--outline", "--repo"])
        .arg(&r.0)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let cli: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(answer, cli);
    assert_eq!(answer["files"][0]["path"], "src/lib.rs");
    let hunks = call(
        &engine,
        &json!({"repo": r.0, "selection": "staged", "view": "hunks", "paths": ["src/lib.rs"]}),
    )
    .unwrap();
    assert_eq!(hunks["hunks"][0]["lines"][1]["text"], "fn b() {}");
    assert_eq!(hunks["input_sha256"], answer["input_sha256"]);
}

#[test]
fn bad_requests_and_contracts_are_refused() {
    let r = repo("bad");
    let engine = allowing(&[&r.0]);
    let selection = call(&engine, &json!({"repo": r.0, "selection": "everything"})).unwrap_err();
    assert_eq!(selection.code, INVALID_PARAMS);
    let view = call(
        &engine,
        &json!({"repo": r.0, "selection": "staged", "view": "pdf"}),
    )
    .unwrap_err();
    assert_eq!(view.code, INVALID_PARAMS);
    git(&r.0, &["commit", "-qm", "staged"]);
    let empty = call(
        &engine,
        &json!({"repo": r.0, "selection": "staged", "require_files": true}),
    )
    .unwrap_err();
    assert_eq!(
        (empty.code, empty.message.as_str()),
        (
            INPUT_CONTRACT_REFUSED,
            "refused: 0 files in input (--require-files)"
        )
    );
    let range = call(
        &engine,
        &json!({"repo": r.0, "selection": "range", "range": "nope..HEAD"}),
    )
    .unwrap_err();
    assert!(
        matches!(range.code, INVALID_PARAMS | GIT_FAILED),
        "{range:?}"
    );
}

/// `--allow-repo` names roots that must exist, and only on the servers that read Git.
#[test]
fn allow_repo_is_checked_where_it_is_given() {
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
            .args(args)
            .stdin(std::process::Stdio::null())
            .output()
            .unwrap()
    };
    for server in ["mcp", "rpc"] {
        let out = run(&[server, "--allow-repo", "/nonexistent/ddf-root"]);
        assert_eq!(out.status.code(), Some(2), "{server}");
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("--allow-repo /nonexistent/ddf-root")
        );
        let out = run(&[server, "--allow-repo"]);
        assert_eq!(out.status.code(), Some(2), "{server} without a value");
    }
    let out = run(&["daemon", "status", "--allow-repo", "/tmp"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("only to `daemon start`"));
}

/// A `.git` file inside an allowed root that points at a repository outside it, and a repository
/// that borrows objects through `objects/info/alternates`, are refused before anything is read
/// (bug hunt 2026-10-28, H2-02).
#[test]
fn git_directories_outside_the_root_are_refused() {
    let (inside, outside) = (repo("gitfile-in"), repo("gitfile-out"));
    let link = inside.0.join("sub/escape");
    std::fs::create_dir_all(&link).unwrap();
    std::fs::write(
        link.join(".git"),
        format!("gitdir: {}\n", outside.0.join(".git").display()),
    )
    .unwrap();
    let engine = allowing(&[&inside.0]);
    let escaped = call(&engine, &json!({"repo": link, "selection": "staged"})).unwrap_err();
    assert!(
        escaped.message.contains("Git directory"),
        "{}",
        escaped.message
    );
    let alternates = inside.0.join(".git/objects/info/alternates");
    std::fs::write(
        &alternates,
        format!("{}\n", outside.0.join(".git/objects").display()),
    )
    .unwrap();
    let borrowed = call(&engine, &json!({"repo": inside.0, "selection": "staged"})).unwrap_err();
    assert!(
        borrowed.message.contains("alternates"),
        "{}",
        borrowed.message
    );
}

/// A param a method does not take is refused, not ignored: a misspelt `require_file` let an empty
/// review pass (bug hunt 2026-10-28, H5-01).
#[test]
fn unknown_params_are_refused() {
    let line = json!({"method": "diff.compute", "params": {"patch": "", "require_file": true}})
        .to_string();
    let err = dispatch(&parse_request(&line).unwrap(), &Engine::new()).unwrap_err();
    assert_eq!(err.code, INVALID_PARAMS);
    assert!(err.message.contains("require_file"), "{}", err.message);
    let r = repo("unknown");
    let engine = allowing(&[&r.0]);
    let typo = call(
        &engine,
        &json!({"repo": r.0, "selection": "staged", "max_byte": 1}),
    )
    .unwrap_err();
    assert!(typo.message.contains("max_byte"), "{}", typo.message);
}

/// `mcp --allow-repo` and `rpc --allow-repo` really grant the roots they name: the binary serves a
/// staged change through MCP `review_change` and JSON-RPC `review.change`.
#[test]
fn the_servers_grant_the_roots_they_are_given() {
    use std::io::Write as _;
    let r = repo("served");
    let root = r.0.to_str().unwrap();
    let serve = |args: &[&str], lines: &[Value]| -> Vec<Value> {
        let mut child = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
            .args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let mut input = child.stdin.take().unwrap();
        for line in lines {
            writeln!(input, "{line}").unwrap();
        }
        drop(input);
        let out = child.wait_with_output().unwrap();
        String::from_utf8(out.stdout)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    };
    let rpc = serve(
        &["rpc", "--allow-repo", root],
        &[
            json!({"jsonrpc": "2.0", "id": 1, "method": "review.change", "params": {"repo": root, "selection": "staged"}}),
        ],
    );
    assert_eq!(
        rpc[0]["result"]["files"][0]["path"], "src/lib.rs",
        "{}",
        rpc[0]
    );
    let mcp = serve(
        &["mcp", "--allow-repo", root],
        &[
            json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "t", "version": "1"}}}),
            json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "review_change", "arguments": {"repo": root, "selection": "staged"}}}),
        ],
    );
    assert_eq!(mcp[1]["result"]["isError"], false, "{}", mcp[1]);
    assert_eq!(
        mcp[1]["result"]["structuredContent"]["files"][0]["path"],
        "src/lib.rs"
    );
}

/// Bug hunt 2026-10-28, H5-03: an untracked nested repository is not captured, and the sealed
/// answer's `acquisition` names it, so an agent knows the review is not the whole change.
#[test]
fn an_omitted_nested_repository_is_named_in_the_acquisition() {
    let r = repo("nested");
    std::fs::create_dir_all(r.0.join("vendor/inner")).unwrap();
    git(&r.0.join("vendor/inner"), &["init", "-q"]);
    std::fs::write(r.0.join("vendor/inner/y.txt"), "y\n").unwrap();
    std::fs::write(r.0.join("new.txt"), "n\n").unwrap();
    let engine = allowing(&[&r.0]);
    let answer = call(
        &engine,
        &json!({"repo": r.0, "selection": "working_tree", "include_untracked": true}),
    )
    .unwrap();
    assert_eq!(
        answer["acquisition"]["omitted_untracked"],
        json!(["vendor/inner/"])
    );
    let staged = call(&engine, &json!({"repo": r.0, "selection": "staged"})).unwrap();
    assert_eq!(staged["acquisition"]["omitted_untracked"], json!([]));
}
