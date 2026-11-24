// Module index: docs/ARCHITECTURE.md#crates-as-built-030
//! The exit-code registry: the single owner of every process exit code deep-diff-forge documents.
//!
//! Two families share numbers with different meanings: `council` (the review-council subcommands)
//! and `engine` (every other command). Every exit site uses these constants; the CLI's tests refuse
//! a numeric exit literal and check the exit-code tables of
//! `docs/reference/output-and-exit-codes.md` against [`EXIT_CODES`].

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
/// Not a registry code: `envelope` exits `SIGNAL_BASE + N` when the command it wraps is ended by
/// signal N, the status a shell reports for that command, and says `outcome: signal`. The daemon
/// exits `SIGNAL_BASE + N` when a second signal N ends its drain.
pub const SIGNAL_BASE: i32 = 128;

/// Council: the requested operation completed under its contract.
pub const COUNCIL_COMPLETED: i32 = 0;
/// Council: an I/O failure after the decision was recorded (`import-tuicr --execute`).
pub const COUNCIL_RECORDED_INCOMPLETE: i32 = 3;
/// Council: malformed input, stale admission, an existing artifact, or a protocol or I/O failure.
pub const COUNCIL_REFUSED: i32 = 4;
/// Council: HOLD (failed, missing or inconclusive evidence, or an import conflict).
pub const COUNCIL_HOLD: i32 = 7;

/// What a caller, person or agent, should do after an exit code. Every registry entry names one,
/// so a new code cannot be added without deciding it; `may_be_transient` says whether the same
/// command can succeed later without being changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NextAction {
    /// Use the result.
    Proceed,
    /// Correct the command line; the same arguments fail the same way.
    FixInvocation,
    /// Check what the message names (a file or stream, the terminal, the daemon); the command can
    /// succeed once that is fixed. For non-UTF-8 source files, fix the file.
    CheckEnvironment,
    /// Change the input or the state the message names (a stale admission, an existing artifact);
    /// the same input is refused again.
    ChangeInput,
    /// Read the reason and the evidence the message or the council reported; a person decides
    /// what changes; rerunning does not change it.
    ReviewEvidence,
    /// Report a bug with the command and its input; do not retry blindly.
    ReportBug,
}

impl NextAction {
    /// Every action, in declaration order.
    pub const ALL: [Self; 6] = [
        Self::Proceed,
        Self::FixInvocation,
        Self::CheckEnvironment,
        Self::ChangeInput,
        Self::ReviewEvidence,
        Self::ReportBug,
    ];

    /// The action's name in JSON (`schema exit-codes`, `explain --json`, MCP).
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Proceed => "proceed",
            Self::FixInvocation => "fix_invocation",
            Self::CheckEnvironment => "check_environment",
            Self::ChangeInput => "change_input",
            Self::ReviewEvidence => "review_evidence",
            Self::ReportBug => "report_bug",
        }
    }

    /// Whether the cause can be transient (a missing file, a busy terminal, a stopped daemon), so
    /// the same command might succeed later. Every other action repeats the same result.
    #[must_use]
    pub const fn may_be_transient(self) -> bool {
        matches!(self, Self::CheckEnvironment)
    }
}

/// One exit code of one command family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExitCode {
    pub family: &'static str,
    pub code: i32,
    pub name: &'static str,
    pub meaning: &'static str,
    /// What to do next.
    pub action: NextAction,
}

const fn entry(
    family: &'static str,
    code: i32,
    name: &'static str,
    meaning: &'static str,
    action: NextAction,
) -> ExitCode {
    ExitCode {
        family,
        code,
        name,
        meaning,
        action,
    }
}

/// Every documented exit code. The "Exit codes" rows of
/// `docs/reference/output-and-exit-codes.md` begin with these meanings.
pub const EXIT_CODES: &[ExitCode] = &[
    entry(
        "engine",
        SUCCESS,
        "success",
        "Success.",
        NextAction::Proceed,
    ),
    entry(
        "engine",
        USAGE,
        "usage",
        "CLI usage / argument error.",
        NextAction::FixInvocation,
    ),
    entry(
        "engine",
        IO,
        "io",
        "I/O or encoding failure: reading input (stdin, a file, Git, the learning store), non-UTF-8 input, or writing output.",
        NextAction::CheckEnvironment,
    ),
    entry(
        "engine",
        INVALID_INPUT,
        "invalid_input",
        "Invalid input: a patch or analysis input that fails to parse or validate, `--stdin-patch` input in which Git finds no patch (`no_patch_in_input`), input over the byte budget, an invalid receipt, a named path that is not a regular file, or a refused Git acquisition or file pair.",
        NextAction::ChangeInput,
    ),
    entry(
        "engine",
        DAEMON_OR_TERMINAL,
        "daemon_or_terminal",
        "Daemon / interactive-terminal failure.",
        NextAction::CheckEnvironment,
    ),
    entry(
        "engine",
        INPUT_CONTRACT_REFUSED,
        "input_contract_refused",
        "Input contract refused.",
        NextAction::ChangeInput,
    ),
    entry(
        "engine",
        PANIC,
        "panic",
        "Internal error (a panic).",
        NextAction::ReportBug,
    ),
    entry(
        "council",
        COUNCIL_COMPLETED,
        "completed",
        "The requested council operation completed under its contract; `status` exit 0 means ready for human review.",
        NextAction::Proceed,
    ),
    entry(
        "council",
        COUNCIL_RECORDED_INCOMPLETE,
        "recorded_incomplete",
        "I/O failure after the decision was recorded: the journal holds it; run the same import again to complete it.",
        NextAction::CheckEnvironment,
    ),
    entry(
        "council",
        COUNCIL_REFUSED,
        "refused",
        "Malformed input, stale admission, an existing artifact, or a protocol or I/O failure.",
        NextAction::ChangeInput,
    ),
    entry(
        "council",
        COUNCIL_HOLD,
        "hold",
        "HOLD: failed, missing or inconclusive evidence, or an import conflict.",
        NextAction::ReviewEvidence,
    ),
];

/// The registry entry for `code` in `family`, if documented.
#[must_use]
pub fn lookup(family: &str, code: i32) -> Option<&'static ExitCode> {
    EXIT_CODES
        .iter()
        .find(|e| e.family == family && e.code == code)
}

/// What a refused command leaves on stdout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stdout {
    /// An input refusal: nothing is written.
    Empty,
    /// A verdict refusal: the judged document is written, then the refusal.
    Document,
}

impl Stdout {
    /// The name in JSON (`schema exit-codes`, `explain --json`, MCP).
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Empty => "empty",
            Self::Document => "document",
        }
    }
}

/// One reason class of the engine's exit 7 ([`INPUT_CONTRACT_REFUSED`]). Exit 7 has one meaning;
/// stderr names the class in one line, `refused: <text> (<tag>)`, whose text holds counts, flag,
/// label and class names only, never text from the input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Refusal {
    /// The class's name: snake case, unique, never reused.
    pub class: &'static str,
    /// What the reason line ends with, inside the parentheses: unique.
    pub tag: &'static str,
    /// The commands that can refuse with this class.
    pub commands: &'static str,
    /// What the refused command leaves on stdout.
    pub stdout: Stdout,
    /// What to do next: it refines exit 7's action for this class.
    pub action: NextAction,
    /// The release in which this refusal first shipped.
    pub since: &'static str,
}

/// Every reason class of the engine's exit 7, in the order their reason lines are written. A class
/// enters here in the commit that makes it reachable. The "Refusal reasons (exit 7)" table of
/// `docs/reference/output-and-exit-codes.md` lists these rows.
pub const REFUSALS: &[Refusal] = &[
    Refusal {
        class: "require_files",
        tag: "--require-files",
        commands: "`--stdin-patch`, `--git-*`, `<old> <new>`, `findings verify`; daemon `require_files`",
        stdout: Stdout::Empty,
        action: NextAction::ChangeInput,
        since: "0.3.0",
    },
    Refusal {
        class: "require_hunks",
        tag: "--require-hunks",
        commands: "`--stdin-patch`, `--git-*`, `<old> <new>`, `findings verify`; daemon `require_hunks`",
        stdout: Stdout::Empty,
        action: NextAction::ChangeInput,
        since: "0.3.0",
    },
    Refusal {
        class: "require_findings",
        tag: "--require-findings",
        commands: "`findings verify`",
        stdout: Stdout::Document,
        action: NextAction::ReviewEvidence,
        since: "0.8.0",
    },
    Refusal {
        class: "require_all",
        tag: "--require-all",
        commands: "`findings verify`",
        stdout: Stdout::Document,
        action: NextAction::ReviewEvidence,
        since: "0.5.0",
    },
    Refusal {
        class: "eval_failed",
        tag: "eval",
        commands: "`eval`",
        stdout: Stdout::Document,
        action: NextAction::ReviewEvidence,
        since: "0.5.0",
    },
    Refusal {
        class: "chain_not_ready",
        tag: "agent doctor",
        commands: "`agent doctor`",
        stdout: Stdout::Document,
        action: NextAction::CheckEnvironment,
        since: "0.9.0",
    },
    Refusal {
        class: "fail_on_label",
        tag: "--fail-on",
        commands: "`--rank` and `--diagnostics` of `--stdin-patch`, `--git-*`, `<old> <new>`",
        stdout: Stdout::Document,
        action: NextAction::ReviewEvidence,
        since: "0.11.0",
    },
    Refusal {
        class: "fail_on_truncated",
        tag: "--fail-on evidence_truncated",
        commands: "`--rank` and `--diagnostics` of `--stdin-patch`, `--git-*`, `<old> <new>`",
        stdout: Stdout::Document,
        action: NextAction::ReviewEvidence,
        since: "0.11.0",
    },
    Refusal {
        class: "require_pass_no_unit",
        tag: "--require-pass no_unit",
        commands: "`agent check`",
        stdout: Stdout::Document,
        action: NextAction::ReviewEvidence,
        since: "0.11.0",
    },
    Refusal {
        class: "require_pass_not_pass",
        tag: "--require-pass not_pass",
        commands: "`agent check`",
        stdout: Stdout::Document,
        action: NextAction::ReviewEvidence,
        since: "0.11.0",
    },
    Refusal {
        class: "branch_unverified",
        tag: "verify-branch",
        commands: "`agent verify-branch`",
        stdout: Stdout::Document,
        action: NextAction::ReviewEvidence,
        since: "0.11.0",
    },
];

/// The registered reason class named `class`.
///
/// # Panics
/// When no class has that name: callers name a class of [`REFUSALS`] literally, and every class is
/// driven by the CLI's tests.
#[must_use]
pub fn refusal(class: &str) -> &'static Refusal {
    REFUSALS
        .iter()
        .find(|r| r.class == class)
        .unwrap_or_else(|| panic!("no refusal class {class}"))
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

    /// The action of every code is part of the contract agents act on.
    #[test]
    fn every_code_names_its_next_action() {
        use NextAction as A;
        let table: Vec<(&str, &str, NextAction)> = EXIT_CODES
            .iter()
            .map(|e| (e.family, e.name, e.action))
            .collect();
        assert_eq!(
            table,
            [
                ("engine", "success", A::Proceed),
                ("engine", "usage", A::FixInvocation),
                ("engine", "io", A::CheckEnvironment),
                ("engine", "invalid_input", A::ChangeInput),
                ("engine", "daemon_or_terminal", A::CheckEnvironment),
                ("engine", "input_contract_refused", A::ChangeInput),
                ("engine", "panic", A::ReportBug),
                ("council", "completed", A::Proceed),
                ("council", "recorded_incomplete", A::CheckEnvironment),
                ("council", "refused", A::ChangeInput),
                ("council", "hold", A::ReviewEvidence),
            ]
        );
    }

    /// Names are unique, and only an environment problem can clear itself.
    #[test]
    fn actions_have_unique_names_and_one_transient_kind() {
        let names: Vec<&str> = NextAction::ALL.iter().map(|a| a.name()).collect();
        assert_eq!(
            names,
            [
                "proceed",
                "fix_invocation",
                "check_environment",
                "change_input",
                "review_evidence",
                "report_bug"
            ]
        );
        let transient: Vec<NextAction> = NextAction::ALL
            .into_iter()
            .filter(|a| a.may_be_transient())
            .collect();
        assert_eq!(transient, [NextAction::CheckEnvironment]);
    }

    /// F7.h1: a class and a tag name one reason each, so a gate reading the line's tag knows the
    /// class; every class is snake case and every tag fits the reason grammar's tag.
    #[test]
    fn every_refusal_class_and_tag_is_unique() {
        for (i, a) in REFUSALS.iter().enumerate() {
            for b in &REFUSALS[i + 1..] {
                assert_ne!(a.class, b.class, "two classes named {}", a.class);
                assert_ne!(
                    a.tag, b.tag,
                    "{} and {} share the tag {}",
                    a.class, b.class, a.tag
                );
            }
            assert!(
                a.class.bytes().all(|c| c.is_ascii_lowercase() || c == b'_'),
                "{}",
                a.class
            );
            let tag = a.tag.trim_start_matches('-');
            let (head, tail) = tag.split_once(' ').unwrap_or((tag, ""));
            assert!(
                a.tag.len() - tag.len() <= 2
                    && head.starts_with(|c: char| c.is_ascii_lowercase())
                    && head.bytes().all(|c| c.is_ascii_lowercase() || c == b'-')
                    && tail.bytes().all(|c| c.is_ascii_lowercase() || c == b'_'),
                "{}: the tag {:?} does not fit the reason grammar",
                a.class,
                a.tag
            );
            assert_eq!(refusal(a.class), a);
        }
    }

    #[test]
    fn lookup_is_by_family_and_code() {
        assert_eq!(lookup("engine", 4).map(|e| e.name), Some("invalid_input"));
        assert_eq!(lookup("council", 4).map(|e| e.name), Some("refused"));
        assert_eq!(
            lookup("council", 3).map(|e| e.name),
            Some("recorded_incomplete")
        );
        assert_eq!(lookup("council", 5), None);
        assert_eq!(lookup("other", 0), None);
    }
}
