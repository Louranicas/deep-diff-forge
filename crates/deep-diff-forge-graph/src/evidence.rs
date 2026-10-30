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

use crate::lexical::{Language, ScanState, code_view, has_token, is_boundary_blocker};
use deep_diff_forge_core::{HunkId, PatchHunk, PatchLineKind, ReviewFile};

/// How every evidence item was found.
pub const EVIDENCE_BASIS: &str = "lexical";

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
}

impl EvidenceSignal {
    /// Every signal, in the order scores and output list them.
    pub const ALL: [Self; 9] = [
        Self::UnsafeAdded,
        Self::ProcessSpawn,
        Self::NetworkFetch,
        Self::DynamicEval,
        Self::CiPrivilege,
        Self::BuildHook,
        Self::SecretLike,
        Self::CheckSuppressed,
        Self::TestRemoved,
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
        }
    }

    /// The score a file gains once for carrying this signal (the one weight table).
    #[must_use]
    pub const fn weight(self) -> u32 {
        match self {
            Self::SecretLike => 5,
            Self::UnsafeAdded | Self::ProcessSpawn | Self::DynamicEval | Self::CiPrivilege => 4,
            Self::NetworkFetch | Self::BuildHook | Self::TestRemoved => 3,
            Self::CheckSuppressed => 2,
        }
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

/// One piece of evidence: a signal at a line of a hunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Evidence {
    pub signal: EvidenceSignal,
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
        for (side, signal, line) in hunk_matches(hunk, kind.language, &rules) {
            let address = address
                .get_or_insert_with(|| hunk.address(&file.path))
                .clone();
            let item = Evidence {
                signal,
                hunk: hunk.id,
                address,
                side,
                line,
            };
            if !out.contains(&item) {
                out.push(item);
            }
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
        .filter(|signal| found.iter().any(|(_, s, _)| s == signal))
        .collect()
}

/// `(side, signal, line)` for every rule that matches a changed line of `hunk`. Block comments
/// and triple strings are followed through the hunk on each side (state starts fresh per hunk).
fn hunk_matches(
    hunk: &PatchHunk,
    language: Option<Language>,
    rules: &[&Rule],
) -> Vec<(Side, EvidenceSignal, u32)> {
    let mut old_state = ScanState::default();
    let mut new_state = ScanState::default();
    let mut found = Vec::new();
    for line in &hunk.lines {
        let (side, number, view) = match line.kind {
            PatchLineKind::Context => {
                if let Some(language) = language {
                    let _ = code_view(language, &line.text, &mut old_state);
                    let _ = code_view(language, &line.text, &mut new_state);
                }
                continue;
            }
            PatchLineKind::Added => (
                Side::New,
                line.new_line,
                language.map(|l| code_view(l, &line.text, &mut new_state)),
            ),
            PatchLineKind::Removed => (
                Side::Old,
                line.old_line,
                language.map(|l| code_view(l, &line.text, &mut old_state)),
            ),
        };
        let Some(number) = number else { continue };
        for rule in rules.iter().filter(|r| r.side == side) {
            let text = match rule.view {
                View::Raw => line.text.as_str(),
                // A rule that reads code needs a language; `Scope::Any` code rules have none.
                View::Code => view.as_deref().unwrap_or(line.text.as_str()),
            };
            let hit = match rule.matches {
                Match::Tokens(tokens) => tokens.iter().any(|t| token_in(text, t)),
                Match::AnyCode => !text.trim().is_empty(),
                Match::Secret => looks_like_secret(text),
            };
            if hit {
                found.push((side, rule.signal, number));
            }
        }
    }
    found
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
                    hunk: hunk.id,
                    address: hunk.address("src/lib.rs"),
                    side: Side::New,
                    line: 3,
                },
                Evidence {
                    signal: S::ProcessSpawn,
                    hunk: hunk.id,
                    address: hunk.address("src/lib.rs"),
                    side: Side::New,
                    line: 3,
                },
            ]
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
