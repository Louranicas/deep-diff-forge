// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fdeep-diff-forge-core--exit
//! The exit-code registry: the single owner of every process exit code deep-diff-forge documents.
//!
//! Two families share numbers with different meanings: `council` (the review-council subcommands)
//! and `engine` (every other command). Every exit site uses these constants; the CLI's tests refuse
//! a numeric exit literal and check the README tables against [`EXIT_CODES`].

/// The command completed under its contract.
pub const SUCCESS: i32 = 0;
/// The arguments were not understood.
pub const USAGE: i32 = 2;
/// Reading or decoding input, or writing output, failed.
pub const IO: i32 = 3;
/// The input was read but is not valid for the command.
pub const INVALID_INPUT: i32 = 4;
/// The daemon or the interactive terminal failed.
pub const DAEMON_OR_TERMINAL: i32 = 6;
/// A valid input was refused by a contract the caller asked for.
pub const INPUT_CONTRACT_REFUSED: i32 = 7;
/// Rust's exit status for a panic.
pub const PANIC: i32 = 101;

/// Council: the requested operation completed under its contract.
pub const COUNCIL_COMPLETED: i32 = 0;
/// Council: malformed input, stale admission, an existing artifact, or a protocol or I/O failure.
pub const COUNCIL_REFUSED: i32 = 4;
/// Council: HOLD (failed, missing or inconclusive evidence, or an import conflict).
pub const COUNCIL_HOLD: i32 = 7;

/// One exit code of one command family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExitCode {
    pub family: &'static str,
    pub code: i32,
    pub name: &'static str,
    pub meaning: &'static str,
}

const fn entry(
    family: &'static str,
    code: i32,
    name: &'static str,
    meaning: &'static str,
) -> ExitCode {
    ExitCode {
        family,
        code,
        name,
        meaning,
    }
}

/// Every documented exit code. README "Exit codes" rows begin with these meanings.
pub const EXIT_CODES: &[ExitCode] = &[
    entry("engine", SUCCESS, "success", "Success."),
    entry("engine", USAGE, "usage", "CLI usage / argument error."),
    entry(
        "engine",
        IO,
        "io",
        "I/O or encoding failure: reading input (stdin, a file, Git, the learning store), non-UTF-8 input, or writing output.",
    ),
    entry(
        "engine",
        INVALID_INPUT,
        "invalid_input",
        "Invalid input: a patch or analysis input that fails to parse or validate, input over the byte budget, an invalid receipt, or a refused Git acquisition.",
    ),
    entry(
        "engine",
        DAEMON_OR_TERMINAL,
        "daemon_or_terminal",
        "Daemon / interactive-terminal failure.",
    ),
    entry(
        "engine",
        INPUT_CONTRACT_REFUSED,
        "input_contract_refused",
        "Input contract refused.",
    ),
    entry("engine", PANIC, "panic", "Internal error (a panic)."),
    entry(
        "council",
        COUNCIL_COMPLETED,
        "completed",
        "The requested council operation completed under its contract; `status` exit 0 means ready for human review.",
    ),
    entry(
        "council",
        COUNCIL_REFUSED,
        "refused",
        "Malformed input, stale admission, an existing artifact, or a protocol or I/O failure.",
    ),
    entry(
        "council",
        COUNCIL_HOLD,
        "hold",
        "HOLD: failed, missing or inconclusive evidence, or an import conflict.",
    ),
];

/// The registry entry for `code` in `family`, if documented.
#[must_use]
pub fn lookup(family: &str, code: i32) -> Option<&'static ExitCode> {
    EXIT_CODES
        .iter()
        .find(|e| e.family == family && e.code == code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_family_code_pair_is_unique_and_named() {
        for (i, a) in EXIT_CODES.iter().enumerate() {
            assert!(!a.name.is_empty() && a.meaning.ends_with('.'), "{a:?}");
            assert!(
                EXIT_CODES[i + 1..]
                    .iter()
                    .all(|b| (a.family, a.code) != (b.family, b.code)),
                "{a:?}"
            );
            assert!(
                EXIT_CODES[i + 1..]
                    .iter()
                    .all(|b| a.family != b.family || a.name != b.name),
                "{a:?}"
            );
        }
    }

    #[test]
    fn lookup_is_by_family_and_code() {
        assert_eq!(lookup("engine", 4).map(|e| e.name), Some("invalid_input"));
        assert_eq!(lookup("council", 4).map(|e| e.name), Some("refused"));
        assert_eq!(lookup("council", 3), None);
        assert_eq!(lookup("other", 0), None);
    }
}
