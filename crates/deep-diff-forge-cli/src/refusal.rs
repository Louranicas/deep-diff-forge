// Module index: docs/reference/output-and-exit-codes.md#refusal-reasons-exit-7
//! The one door to exit 7 (F7.h2). Every exit 7 of the engine family leaves through here, naming
//! a registered reason class ([`deep_diff_forge_core::exit::REFUSALS`]) in one stderr line
//! `refused: <text> (<tag>)`. An input refusal ([`refuse`], [`refuse_input`]) exits before anything
//! is written. A verdict refusal is decided before its document is written ([`pending`]), so a
//! reader that closes stdout ([`stdout_closed`], the only handler of a broken stdout) still gets
//! exit 7 and the reason; [`finish`] writes the reasons after the document.

use deep_diff_forge_core::exit::{self, REFUSALS, Refusal, Stdout};
use std::fmt::Arguments;
use std::sync::{Mutex, PoisonError};

/// The reason lines of the verdict refusals decided for this process, in registry order.
static PENDING: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// The reason line of `class`.
fn line(class: &Refusal, text: Arguments<'_>) -> String {
    format!("refused: {text} ({})", class.tag)
}

/// Write `lines` to stderr and exit 7.
fn exit_with(lines: &[String]) -> ! {
    for line in lines {
        eprintln!("{line}");
    }
    std::process::exit(exit::INPUT_CONTRACT_REFUSED);
}

/// Exit 7 now with the reason of `class`; nothing has been written to stdout.
pub fn refuse(class: &Refusal, text: Arguments<'_>) -> ! {
    exit_with(&[line(class, text)])
}

/// Exit 7 for a refused input contract, given as its reason `<text> (<tag>)`
/// (`deep_diff_forge_patch::InputGuards::refusal`, the same bytes the daemon sends): the class is
/// the input class whose tag the reason ends with.
pub fn refuse_input(reason: &str) -> ! {
    let class = REFUSALS.iter().find_map(|class| {
        let text = reason.strip_suffix(&format!(" ({})", class.tag))?;
        (class.stdout == Stdout::Empty).then_some((class, text))
    });
    match class {
        Some((class, text)) => refuse(class, format_args!("{text}")),
        // Unreachable while the guards' two reasons carry registered tags (tested below).
        None => exit_with(&[format!("refused: {reason}")]),
    }
}

/// Record the verdict refusals that hold, before the document is written. Empty: none.
pub fn pending(reasons: &[(&Refusal, String)]) {
    let mut lines: Vec<(usize, String)> = reasons
        .iter()
        .map(|(class, text)| {
            let order = REFUSALS
                .iter()
                .position(|r| r == *class)
                .unwrap_or(usize::MAX);
            (order, line(class, format_args!("{text}")))
        })
        .collect();
    lines.sort_by_key(|(order, _)| *order);
    *PENDING.lock().unwrap_or_else(PoisonError::into_inner) =
        lines.into_iter().map(|(_, line)| line).collect();
}

/// After the document: exit 7 with the pending reasons, or return when none is pending.
pub fn finish() {
    let lines = std::mem::take(&mut *PENDING.lock().unwrap_or_else(PoisonError::into_inner));
    if !lines.is_empty() {
        exit_with(&lines);
    }
}

/// A write to stdout failed: a reader that has gone is [`stdout_closed`]; any other error exits 3.
pub fn stdout_failed(err: &std::io::Error) -> ! {
    if err.kind() == std::io::ErrorKind::BrokenPipe {
        stdout_closed();
    }
    eprintln!("error: write failed: {err}");
    std::process::exit(exit::IO);
}

/// The reader of stdout has gone (`BrokenPipe`): exit 7 with the pending reasons, or 0 when none
/// is pending (`… | head` of a passing command is a success).
pub fn stdout_closed() -> ! {
    finish();
    std::process::exit(exit::SUCCESS);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both reasons the input guards can give name a registered input class, so `refuse_input`
    /// never takes its unregistered branch.
    #[test]
    fn every_guard_reason_names_an_input_class() {
        for (files, hunks) in [(true, false), (false, true)] {
            let guards = deep_diff_forge_patch::InputGuards { files, hunks };
            let reason = guards.refusal(&[]).unwrap();
            let class = REFUSALS
                .iter()
                .find(|c| reason.ends_with(&format!(" ({})", c.tag)))
                .unwrap_or_else(|| panic!("{reason}: no class"));
            assert_eq!(class.stdout, Stdout::Empty, "{reason}");
        }
    }

    #[test]
    fn pending_reasons_come_in_registry_order() {
        let (all, findings) = (
            exit::refusal("require_all"),
            exit::refusal("require_findings"),
        );
        pending(&[
            (all, "1 of 2 findings were refused or over the limit".into()),
            (findings, "0 findings in the findings file".into()),
        ]);
        let lines = std::mem::take(&mut *PENDING.lock().unwrap());
        assert_eq!(
            lines,
            [
                "refused: 0 findings in the findings file (--require-findings)",
                "refused: 1 of 2 findings were refused or over the limit (--require-all)"
            ]
        );
    }
}
