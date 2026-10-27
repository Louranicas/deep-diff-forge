// DDF integration: R02, R03, R07. Structured Git input for the CLI: acquire with raw-record identities, seal the exact Git patch bytes, and feed the same review/rank/JSON outputs as `--stdin-patch`.
// Design/status: docs/RECOMMENDATION_INTEGRATION.md#patch-truth
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fdeep-diff-forge-cli--git_input
//! `--git-working-tree`, `--git-staged` and `--git-range A..B`.
//!
//! Unlike `git diff | deep-diff-forge --stdin-patch`, these selections run Git
//! themselves (fixed argv, no shell), take every path from Git's NUL-delimited
//! raw records instead of display headers, and can include untracked files
//! (`--include-untracked`, working tree only) without touching the index.
//!
//! Exit codes: 2 = usage error or invalid selection, 3 = Git could not be run
//! or failed, or the patch is not UTF-8, 4 = structured output was malformed,
//! mismatched, over budget or failed to parse, 7 = `--require-files` /
//! `--require-hunks` refusal.

use crate::{InputGuards, emit, emit_review, flag_value, seal_of};
use deep_diff_forge_core::exit;
use deep_diff_forge_core::{display_safe, json_escape};
use deep_diff_forge_patch::{
    GitAcquireError, GitAcquireOptions, GitAcquisition, GitPath, GitSelection, acquire_git,
};

/// Flags that choose a structured Git selection.
const SELECTORS: &[&str] = &["--git-working-tree", "--git-staged", "--git-range"];

/// `--stdin-patch` reads a patch someone else produced; a Git selection makes
/// DDF produce it. Combining them is ambiguous, so it is a usage error.
pub(crate) fn reject_with_stdin(opts: &[String]) {
    if let Some(flag) = opts.iter().find(|a| SELECTORS.contains(&a.as_str())) {
        eprintln!("error: --stdin-patch cannot be combined with {flag}; choose one input source");
        std::process::exit(exit::USAGE);
    }
}

fn usage(message: &str) -> ! {
    eprintln!("error: {message}");
    eprintln!(
        "usage: deep-diff-forge {{--git-working-tree [--include-untracked] | --git-staged | --git-range A..B}} [--repo PATH] [--acquisition-json | --json | --jsonl | --rank | --cluster | --layout inline|side-by-side] [--require-files] [--require-hunks]"
    );
    std::process::exit(exit::USAGE);
}

fn exit_code(err: &GitAcquireError) -> i32 {
    match err {
        GitAcquireError::InvalidSelection { .. } => exit::USAGE,
        GitAcquireError::Spawn { .. }
        | GitAcquireError::CommandFailed { .. }
        | GitAcquireError::NonUtf8Patch
        | GitAcquireError::NonUtf8Path { .. } => exit::IO,
        _ => exit::INVALID_INPUT,
    }
}

fn fail(err: &GitAcquireError) -> ! {
    eprintln!(
        "error: structured Git acquisition failed: {}",
        display_safe(&err.to_string())
    );
    std::process::exit(exit_code(err));
}

/// The structured Git selections' own options plus the shared review output and guard options.
const GIT_FLAGS: [&str; 8] = [
    "--include-untracked",
    "--acquisition-json",
    "--json",
    "--jsonl",
    "--rank",
    "--cluster",
    "--require-files",
    "--require-hunks",
];

/// Entry point: `selector` is the first argument, `opts` the rest.
pub(crate) fn run(selector: &str, opts: &[String]) {
    if opts.iter().any(|a| a == "--stdin-patch") {
        usage(&format!(
            "{selector} cannot be combined with --stdin-patch; choose one input source"
        ));
    }
    let (selection, opts) = match selector {
        "--git-working-tree" => (GitSelection::WorkingTree, opts),
        "--git-staged" => (GitSelection::Staged, opts),
        _ => match opts.split_first() {
            Some((spec, rest)) if !spec.starts_with("--") => (
                GitSelection::parse_range(spec).unwrap_or_else(|e| fail(&e)),
                rest,
            ),
            _ => usage("--git-range needs a value A..B"),
        },
    };
    if let Some(extra) = opts.iter().find(|a| SELECTORS.contains(&a.as_str())) {
        usage(&format!("{selector} cannot be combined with {extra}"));
    }
    let spec = crate::args::Spec {
        flags: &GIT_FLAGS,
        valued: &["--repo", "--layout", "--parallel"],
        positionals: 0,
    };
    if let Err(reason) = crate::args::admit(opts, &spec) {
        usage(&display_safe(&reason));
    }
    let include_untracked = opts.iter().any(|a| a == "--include-untracked");
    if include_untracked && selection != GitSelection::WorkingTree {
        usage("--include-untracked is only valid with --git-working-tree");
    }
    let repo = if opts.iter().any(|a| a == "--repo") {
        match flag_value(opts, "--repo") {
            Some(path) if !path.starts_with("--") => path,
            _ => usage("--repo needs a path"),
        }
    } else {
        ".".to_owned()
    };
    let options = GitAcquireOptions {
        include_untracked,
        ..GitAcquireOptions::default()
    };
    let acquisition =
        acquire_git(std::path::Path::new(&repo), &selection, &options).unwrap_or_else(|e| fail(&e));
    for omitted in acquisition.omitted_untracked() {
        eprintln!(
            "note: untracked directory not captured (nested repository?): {}",
            display_safe(&omitted.display_lossy())
        );
    }
    let captured = acquisition.diff().capture().unwrap_or_else(|e| fail(&e));
    let files = acquisition
        .diff()
        .review_files(&captured)
        .unwrap_or_else(|e| fail(&e));
    let input = captured.original();
    let guards = InputGuards::from_opts(opts);
    guards.enforce(&files);
    if opts.iter().any(|a| a == "--acquisition-json") {
        emit(&acquisition_json(&acquisition, include_untracked, input));
    } else if opts.iter().any(|a| a == "--jsonl") {
        emit(&deep_diff_forge_pipeline::jsonl_events(&files));
    } else {
        emit_review(&files, opts, input);
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}

fn path_json(path: Option<&GitPath>) -> String {
    path.map_or_else(
        || "null".to_owned(),
        |p| {
            format!(
                "{{\"utf8\": {}, \"hex\": \"{}\"}}",
                p.to_utf8().map_or_else(|| "null".to_owned(), json_escape),
                hex(p.as_bytes())
            )
        },
    )
}

fn opt_json(value: Option<&str>) -> String {
    value.map_or_else(|| "null".to_owned(), json_escape)
}

fn mode_json(mode: Option<u32>) -> String {
    mode.map_or_else(|| "null".to_owned(), |m| format!("\"{m:06o}\""))
}

/// The `deep-diff-forge.git-acquisition.v0` document: the structured records
/// alongside the seal of the exact patch bytes they describe.
fn acquisition_json(acq: &GitAcquisition, include_untracked: bool, input: &str) -> String {
    use std::fmt::Write as _;
    let mut changes = String::new();
    for (i, c) in acq.diff().changes().iter().enumerate() {
        if i > 0 {
            changes.push_str(",\n");
        }
        let _ = write!(
            changes,
            "    {{\"status\": {}, \"score\": {}, \"origin\": {}, \"old_path\": {}, \"new_path\": {}, \"old_mode\": {}, \"new_mode\": {}, \"old_blob\": {}, \"new_blob\": {}, \"binary\": {}, \"patch_offset\": {}, \"patch_len\": {}, \"blocks\": {}}}",
            json_escape(c.kind.label()),
            c.score.map_or_else(|| "null".to_owned(), |s| s.to_string()),
            json_escape(c.origin.label()),
            path_json(c.old_path.as_ref()),
            path_json(c.new_path.as_ref()),
            mode_json(c.old_mode),
            mode_json(c.new_mode),
            opt_json(c.old_blob.as_deref()),
            opt_json(c.new_blob.as_deref()),
            c.binary,
            c.patch_range.start,
            c.patch_range.len(),
            c.block_count
        );
    }
    let omitted: Vec<String> = acq
        .omitted_untracked()
        .iter()
        .map(|p| path_json(Some(p)))
        .collect();
    let body = if changes.is_empty() {
        String::new()
    } else {
        format!("\n{changes}\n  ")
    };
    format!(
        "{{\n  \"schema\": \"deep-diff-forge.git-acquisition.v0\",\n{}  \"selection\": {},\n  \"base\": {},\n  \"target\": {},\n  \"include_untracked\": {include_untracked},\n  \"patch_bytes\": {},\n  \"changes\": [{body}],\n  \"omitted_untracked\": [{}]\n}}\n",
        seal_of(input).json_members(),
        json_escape(acq.selection().label()),
        json_escape(acq.base()),
        opt_json(acq.target()),
        input.len(),
        omitted.join(", ")
    )
}
