//! `agent policy-accept --commit C [--policy NAME] [--judge PATH] [--yes-from-pipe]` (release
//! 0.11.1, package L1c, R4): the one way a project's accepted policy changes. It shows the
//! accepted digest now and the digest of `C:<name>`, the whole diff of the policy file between
//! them, the bound name and a new one, and the change of the producer pins; it refuses a standard
//! input that is not a terminal unless told otherwise; the person types the new digest in full;
//! then one acceptance record is appended to the project's policy record, applied by the hooks as
//! a compare-and-swap.
//!
//! **A local convention, not authentication:** any process running as this user can run this
//! command or write the record it appends. The pull request's judge, under the target branch's
//! policy, is the authority. Nothing is written before the digest is typed; the project lock is
//! not held while the command waits for it.

use crate::emit;
use deep_diff_forge_core::exit;
use std::io::{BufRead as _, IsTerminal as _, Read as _, Write as _};

pub(crate) const USAGE: &str = "deep-diff-forge agent policy-accept --commit C [--policy NAME] [--judge PATH] [--yes-from-pipe]";

/// What the command says before it asks (T14).
pub(crate) const CONVENTION: &str = "A policy acceptance record is a local convention, not authentication: any process running as this user can run this command or write the record it appends. The judge on the pull request, under the target branch's policy, is the authority.";

/// The longest line read for the typed digest.
const TYPED_BYTES: u64 = 1024;

/// End the command with `code` and `why` on standard error; before the append, "nothing was
/// written" is said with it (`before`).
fn refused_as(code: i32, why: &str, before: bool) -> ! {
    eprintln!(
        "error: {}{}",
        deep_diff_forge_core::display_safe(why),
        if before { "; nothing was written" } else { "" }
    );
    refused(code)
}

/// The one exit of a refusal: `code` is one of the command's (2, 3, 4, 6; never 7).
fn refused(code: i32) -> ! {
    debug_assert!(matches!(
        code,
        exit::USAGE | exit::IO | exit::INVALID_INPUT | exit::DAEMON_OR_TERMINAL
    ));
    std::process::exit(code)
}

pub(crate) fn run(args: &[String]) {
    crate::args::admit_or_exit(
        args,
        &crate::args::Spec {
            flags: &["--yes-from-pipe"],
            valued: &["--commit", "--policy", "--judge"],
            positionals: 0,
        },
        USAGE,
    );
    let Some(commit) = crate::flag_value(args, "--commit") else {
        eprintln!("error: --commit C is missing");
        eprintln!("usage: {USAGE}");
        std::process::exit(exit::USAGE);
    };
    let dir = std::env::current_dir().unwrap_or_else(|e| {
        refused_as(
            exit::IO,
            &format!("the current directory cannot be read: {e}"),
            true,
        )
    });
    let name = crate::flag_value(args, "--policy");
    let plan = crate::hook_run::policy_accept_plan(&dir, &commit, name.as_deref(), args)
        .unwrap_or_else(|(code, why)| refused_as(code, &why, true));
    let safe = |text: &str| deep_diff_forge_core::display_safe(text).into_owned();
    if plan.same {
        emitln!(
            "The policy {} at commit {} (sha256 {}) is already this project's accepted policy: nothing to accept, and nothing was written.",
            safe(&plan.name),
            &plan.commit[..12],
            plan.to
        );
        return;
    }
    emitln!("Policy record: {}", plan.record);
    emitln!("Bound policy name: {}", safe(&plan.bound));
    if plan.name != plan.bound {
        emitln!("New policy name: {} (accepting binds it)", safe(&plan.name));
    }
    match &plan.from {
        Some((digest, at)) => emitln!(
            "Accepted policy now: sha256 {digest} (the file at commit {})",
            &at[..12]
        ),
        None => emitln!(
            "Accepted policy now: none (this project has no accepted policy yet: accepting this one supplies the first, and the diff below is the whole file)"
        ),
    }
    emitln!(
        "Policy to accept:    sha256 {} ({} at commit {})",
        plan.to,
        safe(&plan.name),
        &plan.commit[..12]
    );
    emitln!("Producer pins: {}", plan.pins);
    emitln!("The whole diff of the policy file, accepted policy to policy to accept:");
    crate::emit(&plan.diff);
    emitln!("{CONVENTION}");
    let terminal = std::io::stdin().is_terminal();
    if !terminal && !args.iter().any(|a| a == "--yes-from-pipe") {
        refused_as(
            exit::USAGE,
            "standard input is not a terminal: a person types the new digest at a terminal",
            true,
        );
    }
    eprint!("Type the new digest (64 hex digits) to accept it: ");
    let _ = std::io::stderr().flush();
    let mut typed = String::new();
    if let Err(e) = std::io::stdin()
        .lock()
        .take(TYPED_BYTES)
        .read_line(&mut typed)
    {
        refused_as(
            exit::DAEMON_OR_TERMINAL,
            &format!("the typed digest could not be read: {e}"),
            true,
        );
    }
    if typed.trim() != plan.to {
        refused_as(
            exit::INVALID_INPUT,
            "the digest typed is not the digest of the policy to accept",
            true,
        );
    }
    match crate::hook_run::policy_accept_write(&plan) {
        Ok(words) => emitln!("{words}"),
        // (the words say what is on disk: nothing, or a part of a record that readers pass over)
        Err((code, why)) => refused_as(code, &why, false),
    }
}
