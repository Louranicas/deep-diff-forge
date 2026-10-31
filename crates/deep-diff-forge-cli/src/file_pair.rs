// DDF integration: R02, R03, R07. Direct two-file review: Git produces the patch from two regular files, and the exact bytes are sealed and fed to the same outputs as `--stdin-patch`.
// Design/status: README "Compare two files"; docs/ROADMAP.md
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fdeep-diff-forge-cli--file_pair
//! `deep-diff-forge [--] <old> <new>`: review two regular files directly.
//!
//! [`deep_diff_forge_patch::diff_files`] runs `git diff --no-index` with the acquisition's
//! hardening and refuses symbolic links, directories, special files and oversized inputs; this
//! module seals Git's bytes and emits the review output the options select. Directories are out
//! of scope: a tree comparison brings nested repositories, symbolic links and special files
//! inside it, which the Git selections already handle for a repository.
//!
//! Exit codes: 2 = usage error, including a first word that is neither a command nor an existing
//! path (unless `--` marks it as a path), 3 = a path cannot be read or Git could not be run, 4 = a path is
//! not a regular file, the inputs exceed the byte budget, or Git's patch failed to parse,
//! 7 = `--require-files` / `--require-hunks` refusal.

use crate::{
    InputGuards, REVIEW_FLAGS, REVIEW_VALUED, emit, emit_review, git_input, sealed_review_or_exit,
    validate_review_options_or_exit,
};
use deep_diff_forge_core::{display_safe, display_safe_bounded, exit};
use deep_diff_forge_patch::{GitAcquireOptions, diff_files};

/// The `--help` line for the direct comparison, whose `COMMANDS` key is `<old>`.
pub(crate) const USAGE: &str = "deep-diff-forge <old> <new> [<--stdin-patch output flags>]";

/// Whether the arguments after an unrecognised first word ask for a direct comparison: a second
/// path follows it. A lone unknown word stays an unknown command.
pub(crate) fn requested(rest: &[String]) -> bool {
    rest.first().is_some_and(|next| !next.starts_with('-'))
}

/// Entry point. `args` is the whole command line, beginning with `--` or the old path.
pub(crate) fn run(args: &[String]) {
    let explicit = args.first().is_some_and(|first| first == "--");
    let args = if explicit { &args[1..] } else { args };
    let [old, new, opts @ ..] = args else {
        eprintln!("error: a direct comparison needs two paths");
        eprintln!("usage: {USAGE}");
        std::process::exit(exit::USAGE);
    };
    if crate::command_help("<old>", opts) {
        return;
    }
    crate::args::admit_or_exit(
        opts,
        &crate::args::Spec {
            flags: &REVIEW_FLAGS,
            valued: &REVIEW_VALUED,
            positionals: 0,
        },
        USAGE,
    );
    validate_review_options_or_exit(opts);
    // Without `--`, a first word that names nothing may be a misspelt command: refuse it as one,
    // so a typo stays a usage error (exit 2) and is not reported as a missing file.
    if !explicit
        && std::fs::symlink_metadata(old).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
    {
        eprintln!("unknown command: {}", display_safe_bounded(old, 200));
        eprintln!("note: no file has that name either; put `--` first to compare two paths");
        std::process::exit(exit::USAGE);
    }
    let patch = diff_files(
        std::path::Path::new(old),
        std::path::Path::new(new),
        &GitAcquireOptions::default(),
    )
    .unwrap_or_else(|error| {
        eprintln!(
            "error: direct comparison failed: {}",
            display_safe(&error.to_string())
        );
        std::process::exit(git_input::exit_code(&error));
    });
    let guards = InputGuards::from_opts(opts);
    let review = sealed_review_or_exit(&patch, guards);
    if opts.iter().any(|a| a == "--jsonl") {
        emit(&deep_diff_forge_pipeline::jsonl_events(review.files()));
    } else {
        emit_review(&review, opts);
    }
}
