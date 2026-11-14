//! `agent hook <event> --client claude`: read the client's JSON, look at the repository, print
//! what [`crate::hook`] decides.
//!
//! This is the part that touches the world: stdin, Git, a scratch directory, one state file per
//! session and, at a stop, the judge. Every decision is made in [`crate::hook`].
//!
//! A hook never fails its client. Whatever goes wrong here (Git too slow, an unreadable state
//! file, a judge that cannot be started, a command line that cannot be read) ends with exit 0
//! and something said: the user is told that the hook did not run and that nothing was judged.
//! Only a verdict of fail from the judge keeps an agent from stopping. Exit 2 in particular is
//! never used: Claude Code reads it from a `Stop` hook as a block (independent review of the
//! hooks, HR-M-5).
//!
//! **One project and one base for a session.** The repository that is judged and the commit
//! its changes are measured from are fixed by the session's first hook and kept in its state:
//! the project is the directory the client names (`CLAUDE_PROJECT_DIR`), else the one recorded,
//! else the hook input's `cwd`; the base is the commit `HEAD` named then. Both used to be read
//! at every stop from things the session itself moves, its current directory and `HEAD`, so a
//! `cd` into another repository or a commit made the stop pass in silence (independent review
//! of the hooks, HR-H-1 and HR-H-2).
//!
//! **A failure is said.** Nothing is said only when Git ran and found no repository for a
//! session that has none recorded, when the repository had no commit as the session began, or
//! when the change is empty. And a stop that is the first hook of its session is never silent
//! once it has found a project: it has only then recorded where the session is, so it says
//! that what was committed before it was not judged.
//!
//! The state file of a session is in the user's state directory (`deep-diff-forge/hooks` under
//! `$XDG_STATE_HOME`, else under `~/.local/state`), where it is found by the session's id alone:
//! a hook must be able to read it before, and without, finding the repository. The scratch
//! directory lives in `deep-diff-forge/` under the repository's Git common directory, beside
//! Git's own files: nothing appears in the working tree, and the objects written while a patch
//! is made stay on the same file system and are removed again.

use crate::emit;
use crate::hook::{self, Event, Finding, Input, Judged, StartBase, StopDecision, TreeJudged};
use deep_diff_forge_core::PatchLineKind;
use deep_diff_forge_core::process::{Limits, run_bounded};
use deep_diff_forge_graph::Side;
use deep_diff_forge_patch::{
    GitAcquireError, GitAcquireOptions, HeadState, InputGuards, JudgePatchError, SealedReview,
    commit_patch, dirty_submodules, file_at_commit, has_commits_beyond, head_contains, head_state,
    judge_patch, merge_base, ref_tips, repository_dirs,
};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub(crate) const USAGE: &str = "deep-diff-forge agent hook session-start|post-tool|stop --client claude [--judge PATH] [--policy NAME] [--ranker-dir DIR]";

/// The policy file a judge reads when `--policy` names no other.
const DEFAULT_POLICY: &str = "warden.toml";
/// The most a client may write on a hook's stdin. A `PostToolUse` input carries the tool's
/// whole answer, so the cap is far above what a hook reads from it: at 1 MiB the answer of one
/// large tool call made the hook say it did not run (independent review of the hooks, HR-L-2).
const INPUT_CAP: u64 = 32 * 1024 * 1024;
/// The most of an input past [`INPUT_CAP`] that is read only to be dropped.
const INPUT_DROPPED: u64 = 1024 * 1024 * 1024;
/// The most keys of reported findings one session's state keeps.
const REPORTED_CAP: usize = 5_000;
/// The most things one session's state keeps as said once ([`once`]). Past it, an advice hook
/// that cannot run says nothing more; a stop still says why it was not judged.
const NOTICED_CAP: usize = 32;
/// The most failures one session's state keeps as having blocked a stop; the oldest goes first.
const BLOCKED_CAP: usize = 256;
/// The most refs whose tips are recorded when a session begins, and the most that are looked
/// at when it stops.
const REFS_CAP: usize = 4096;
/// The most rankers a stop tries before it gives up on finding one the policy admits.
const RANKERS_TRIED: usize = 8;

/// Everything an advice hook does must be done in this time: its client waits five seconds.
const ADVICE_LIMIT: Duration = Duration::from_secs(4);
/// Everything a session's start does must be done in this time: its client waits ten seconds.
const START_LIMIT: Duration = Duration::from_secs(8);
/// Everything a stop does must be done in this time: Git, the helper that hashes the rankers
/// and every run of the judge, for every working tree. The limits used to be one for each
/// step (20 s to find the repository, 20 s for the patch, 20 s for the policy, 8 s for
/// hashing and 30 s for each of up to eight rankers), which added up to several times the
/// sixty seconds the client waits; a stop that ran past them was ended by the client with its
/// answer dropped (independent review of the hooks, HR-L-1).
const STOP_LIMIT: Duration = Duration::from_secs(45);
/// The timeout, in seconds, a registered `Stop` hook needs at least: [`STOP_LIMIT`] and the
/// time to say what was decided. `agent setup --hooks` prints sixty.
pub(crate) const STOP_TIMEOUT_NEEDED: u32 = 50;
/// Each event as Claude Code names it, with the time everything its hook does must be done in.
#[cfg(test)]
pub(crate) const LIMITS: [(&str, Duration); 3] = [
    ("SessionStart", START_LIMIT),
    ("PostToolUse", ADVICE_LIMIT),
    ("Stop", STOP_LIMIT),
];
/// Everything `agent doctor --gate` does must be done in this time.
const GATE_LIMIT: Duration = Duration::from_secs(120);
/// The judge may take this long for one verdict, if the hook's deadline leaves as much.
const JUDGE_TIME: Duration = Duration::from_secs(30);
const JUDGE_STDOUT: u64 = 4 * 1024 * 1024;
const JUDGE_STDERR: u64 = 64 * 1024;

/// When everything a hook does must be done. One deadline is shared by every step: each Git
/// call, the helper and each run of the judge gets the time that is left, never a limit of its
/// own on top of the others'.
#[derive(Debug, Clone, Copy)]
struct Deadline(std::time::Instant);

impl Deadline {
    fn after(limit: Duration) -> Self {
        Self(now() + limit)
    }

    fn left(self) -> Duration {
        self.0.saturating_duration_since(now())
    }

    /// Git options whose time limit is the time that is left.
    fn git(self) -> GitAcquireOptions {
        GitAcquireOptions {
            time_limit: self.left(),
            ..GitAcquireOptions::default()
        }
    }
}

/// A Git failure in words that do not change from one hook to the next: the time limit of a
/// call is whatever its hook had left, and a reason that named it would be a new reason at
/// every tool call ([`notice`] tells each reason once).
fn git_words(error: &GitAcquireError) -> String {
    match error {
        GitAcquireError::TimedOut { command, .. } => {
            format!("`{command}` did not finish in the time a hook has and was stopped")
        }
        other => other.to_string(),
    }
}

/// `agent hook <event> --client claude […]`.
pub(crate) fn run(args: &[String]) {
    // The event as far as it can be read before anything is refused: a stop that does not run
    // says UNJUDGED.
    let named = args.iter().find_map(|word| Event::named(word));
    // A command line that cannot be read is said like every other failure, with exit 0. It
    // used to end with exit 2, which Claude Code reads from a Stop hook as a block with the
    // usage text as the reason: a mistyped settings entry held every stop (HR-M-5).
    let refused = |reason: &str| {
        emitln!(
            "{}",
            hook::did_not_run_output(named, &format!("{reason} (usage: {USAGE})"))
        );
    };
    let words = match crate::args::admit(
        args,
        &crate::args::Spec {
            flags: &[],
            valued: &["--client", "--judge", "--policy", "--ranker-dir"],
            positionals: 1,
        },
    ) {
        Ok(words) => words,
        Err(reason) => return refused(&reason),
    };
    // The stop hook's own helper, started by it as a child (see [`sealed_rankers`]).
    if words.first().copied() == Some(SEAL_WORD) {
        seal_rankers(args);
        return;
    }
    let event = words.first().and_then(|word| Event::named(word));
    let client = crate::flag_value(args, "--client").unwrap_or_default();
    let (Some(event), "claude") = (event, client.as_str()) else {
        return refused("name one event and --client claude");
    };
    let did_not_run = |reason: &str| emitln!("{}", hook::did_not_run_output(Some(event), reason));
    let mut text = String::new();
    let mut stdin = std::io::stdin();
    let read = stdin.by_ref().take(INPUT_CAP + 1).read_to_string(&mut text);
    // What lies past the cap is read and dropped: a client whose write fails on a closed pipe
    // may never read what the hook says about its input.
    let _ = std::io::copy(&mut stdin.take(INPUT_DROPPED), &mut std::io::sink());
    if let Err(error) = read {
        return did_not_run(&format!("its input could not be read: {error}"));
    }
    if text.len() as u64 > INPUT_CAP {
        return did_not_run("its input is over 32 MiB");
    }
    let said = hook::parse_input(event, &text).and_then(|input| match event {
        Event::SessionStart => Ok(session_start(&input, args)),
        Event::PostTool => Ok(post_tool(&input)),
        Event::Stop => stop(&input, args),
    });
    match said {
        Ok(Some(out)) => emitln!("{out}"),
        Ok(None) => {}
        Err(reason) => did_not_run(&reason),
    }
}

/// Where a hook works: a working tree's top level, and the hooks' own directory under the
/// repository's Git common directory.
#[derive(Debug, Clone)]
struct Place {
    top: PathBuf,
    own: PathBuf,
    /// The Git common directory: for a linked worktree it is not under `top`.
    common: PathBuf,
    /// Which repository this is ([`identity`]).
    id: String,
}

/// Which repository a Git common directory is: its device and inode. They stay when the
/// repository is moved, and a clone put at the old path has others.
fn identity(common: &Path) -> Option<String> {
    use std::os::unix::fs::MetadataExt as _;
    let found = std::fs::metadata(common).ok()?;
    Some(format!("{}:{}", found.dev(), found.ino()))
}

/// What Git says of a directory.
enum Found {
    /// It is in this working tree.
    Tree(Place),
    /// Git ran and found no repository around it.
    NotARepository,
    /// It does not exist, or is not a directory.
    Gone,
    /// Git could not say: it is missing, it refuses the repository (another owner), the
    /// directory is inside a Git directory, or it ran out of time.
    Failed(String),
}

/// What Git prints when it finds no repository around a directory, in the `C` locale.
const NO_REPOSITORY: &str = "not a git repository (or any";

/// Ask Git about `dir`. Only Git's own "no repository here" is [`Found::NotARepository`]:
/// every other failure used to be read as that, and a stop passed in silence when Git was not
/// on `PATH` or refused the repository for its owner (independent review of the hooks, HR-M-1).
fn find(dir: &Path, deadline: Deadline) -> Found {
    if !dir.is_dir() {
        return Found::Gone;
    }
    let mut options = deadline.git();
    // Git's words are read below, so they must be the ones it was written with.
    options
        .env
        .push((OsString::from("LC_ALL"), OsString::from("C")));
    match repository_dirs(dir, &options) {
        Ok(dirs) => match identity(&dirs.common_dir) {
            Some(id) => Found::Tree(Place {
                top: dirs.top,
                own: dirs.common_dir.join("deep-diff-forge"),
                common: dirs.common_dir,
                id,
            }),
            None => Found::Failed(format!(
                "the Git directory of the repository {} is in could not be read",
                shown(dir)
            )),
        },
        Err(GitAcquireError::CommandFailed { stderr, .. }) if stderr.contains(NO_REPOSITORY) => {
            Found::NotARepository
        }
        Err(error) => Found::Failed(format!(
            "Git could not say which repository {} is in: {}",
            shown(dir),
            git_words(&error)
        )),
    }
}

fn shown(path: &Path) -> String {
    hook::shown_path(&path.to_string_lossy())
}

/// The directory the client says the session started in, when it names one by an absolute
/// path. Claude Code sets it for every hook and does not move it when the session changes
/// directory, which the `cwd` of the hook input does.
#[expect(
    clippy::disallowed_methods,
    reason = "the project door: the client names the directory a session started in"
)]
fn client_project_dir() -> Option<PathBuf> {
    std::env::var_os("CLAUDE_PROJECT_DIR")
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
}

/// Make sure a directory of the hooks is a directory of its own: made with mode 0700 when it is
/// missing, and refused when a symbolic link or a file stands there. A link there sent the
/// state file and the scratch directory to wherever it pointed (review V4, VD4-M-1).
///
/// This is a check and then a use, by path: whoever can swap the directory between the two
/// can already write anywhere this account can. It stops the link that is simply there.
fn own_directory(own: &Path) -> Result<(), String> {
    use std::os::unix::fs::DirBuilderExt as _;
    let is_own =
        |own: &Path| std::fs::symlink_metadata(own).map(|found| found.file_type().is_dir());
    let refused = || {
        format!(
            "{} is not a directory of its own (a symbolic link or a file stands there)",
            shown(own)
        )
    };
    match is_own(own) {
        Ok(true) => Ok(()),
        Ok(false) => Err(refused()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            match std::fs::DirBuilder::new().mode(0o700).create(own) {
                Ok(()) => Ok(()),
                // another hook made it first
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    if is_own(own).unwrap_or(false) {
                        Ok(())
                    } else {
                        Err(refused())
                    }
                }
                Err(error) => Err(format!("its directory could not be made: {error}")),
            }
        }
        Err(error) => Err(format!("its directory could not be read: {error}")),
    }
}

/// The most entries of the hooks' directory that are looked at for scratch directories of
/// processes that are gone.
const STALE_LOOKED_AT: usize = 4096;

/// Remove the scratch directories that hooks which were killed left behind: the entries
/// directly under `own` named `tmp-<pid>` whose process is gone. A hook ended by its client
/// left its directory, with a copy of the patch and every new object, until a process with the
/// same id came by (independent review of the hooks, HR-L-1).
///
/// Whether a process is gone is read from `/proc`; without one nothing is removed. Only a
/// directory is removed, never what a link of that name points at.
fn clear_stale(own: &Path) {
    if !Path::new("/proc/self").exists() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(own) else {
        return;
    };
    for entry in entries.filter_map(Result::ok).take(STALE_LOOKED_AT) {
        let name = entry.file_name();
        let Some(pid) = name
            .to_str()
            .and_then(|name| name.strip_prefix("tmp-"))
            .filter(|pid| !pid.is_empty() && pid.bytes().all(|b| b.is_ascii_digit()))
        else {
            continue;
        };
        if Path::new("/proc").join(pid).exists() {
            continue;
        }
        if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// A directory of this process, removed when the hook is done. A hook that was killed leaves
/// its directory behind; the next hook of the repository removes it ([`clear_stale`]).
struct Scratch(PathBuf);

impl Scratch {
    fn new(own: &Path) -> Result<Self, String> {
        own_directory(own)?;
        clear_stale(own);
        let dir = own.join(format!("tmp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // `create_dir`, not `create_dir_all`: what is left of an earlier process must be gone,
        // and a link standing there is not followed.
        std::fs::create_dir(&dir)
            .map_err(|e| format!("its scratch directory could not be made: {e}"))?;
        Ok(Self(dir))
    }

    /// A new, empty directory inside the scratch directory.
    fn sub(&self, name: &str) -> Result<PathBuf, String> {
        let dir = self.0.join(name);
        std::fs::create_dir(&dir)
            .map_err(|e| format!("its scratch directory could not be made: {e}"))?;
        Ok(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The commit a session's changes are measured from, as its first hook found it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Base {
    /// The commit `HEAD` named, by its full id.
    Commit(String),
    /// The repository had no commit at all.
    NoCommit,
    /// `HEAD` named no commit in a repository that has commits (an orphan branch).
    Orphan,
}

/// The project of a session: recorded by its first hook that finds a repository, and never
/// changed by a later one.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Project {
    /// The top level of the working tree.
    top: String,
    /// Which repository that is: the device and inode of its Git common directory. A path
    /// alone names whatever stands there now: the project moved away and a clean clone put in
    /// its place was taken for the project (review V4 round 10, VD4R10-H-1).
    id: String,
    base: Base,
    /// The commits the repository's refs and `HEAD` led to when the session began
    /// ([`REFS_CAP`] refs at most). What a ref names now and none of these leads to was
    /// committed, fetched or pulled since.
    tips: Vec<String>,
    /// The repository had more refs than were recorded.
    tips_partial: bool,
    /// A session-start hook recorded this. Otherwise the base is only where `HEAD` was when
    /// the session's first hook ran.
    by_start: bool,
}

/// What one session remembers between hooks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct State {
    project: Option<Project>,
    /// A stop has said that the session's start was not recorded ([`hook::FIRST_STOP`]).
    start_said: bool,
    /// The keys of findings already said in this session ([`Finding::key`]).
    reported: BTreeSet<String>,
    /// What was already said once in this session, hashed: the reasons an advice hook gave for
    /// not running ([`notice`]) and the things a stop named as not judged.
    noticed: BTreeSet<String>,
    /// The failures that have blocked a stop of this session ([`hook::failure_digest`]), the
    /// latest last.
    blocked: Vec<String>,
}

const STATE_SCHEMA: &str = "deep-diff-forge.hook-state.v2";

/// A session's state file. The session id is hashed: it is the client's string and must not
/// choose a file name.
fn state_file(dir: &Path, session: &str) -> PathBuf {
    let hash = deep_diff_forge_core::sha256_hex(session.as_bytes());
    dir.join(format!("session-{}.json", &hash[..32]))
}

/// Where the state files of this user's sessions are: `deep-diff-forge/hooks` under an absolute
/// `$XDG_STATE_HOME`, else under `.local/state` of an absolute `$HOME`. A relative value is
/// not used: it would put the state under whatever directory the hook runs in.
fn resolve_sessions_dir(
    xdg_state_home: Option<OsString>,
    home: Option<OsString>,
) -> Option<PathBuf> {
    let absolute = |value: Option<OsString>| value.map(PathBuf::from).filter(|p| p.is_absolute());
    absolute(xdg_state_home)
        .or_else(|| absolute(home).map(|home| home.join(".local/state")))
        .map(|state| state.join("deep-diff-forge/hooks"))
}

#[expect(
    clippy::disallowed_methods,
    reason = "the hook-state door: XDG_STATE_HOME and HOME locate the sessions' state"
)]
fn sessions_dir() -> Result<PathBuf, String> {
    resolve_sessions_dir(std::env::var_os("XDG_STATE_HOME"), std::env::var_os("HOME")).ok_or_else(
        || {
            "there is no state directory (XDG_STATE_HOME or HOME must be an absolute path)"
                .to_owned()
        },
    )
}

/// The base as the state file writes it: the commit's id, or one of two words.
fn base_text(base: &Base) -> &str {
    match base {
        Base::Commit(id) => id,
        Base::NoCommit => "none",
        Base::Orphan => "orphan",
    }
}

/// A full commit id as Git writes one: 40 or 64 lowercase hex digits.
fn is_commit_id(text: &str) -> bool {
    matches!(text.len(), 40 | 64)
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn base_from(text: &str) -> Option<Base> {
    match text {
        "none" => Some(Base::NoCommit),
        "orphan" => Some(Base::Orphan),
        id if is_commit_id(id) => Some(Base::Commit(id.to_owned())),
        _ => None,
    }
}

/// The identity of a repository as the state writes it: two numbers.
fn is_identity(text: &str) -> bool {
    text.split_once(':').is_some_and(|(device, inode)| {
        let number = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
        number(device) && number(inode)
    })
}

/// Read a state file's text. `Err` says why it is not a state this version wrote: such a file
/// is never read as "no state", in part or whole. An empty state would make the next hook the
/// session's first, and the commit `HEAD` names then its base: what the session had committed
/// until then passed in silence (second independent review of the hooks, HR2-M-1).
fn state_from(text: &str) -> Result<State, &'static str> {
    let value = serde_json::from_str::<Value>(text).map_err(|_| "it is not JSON")?;
    if value["schema"] != STATE_SCHEMA {
        return Err("it is not a state of this version of the hooks");
    }
    let strings = |member: &Value| -> Option<Vec<String>> {
        member
            .as_array()?
            .iter()
            .map(|v| v.as_str().map(str::to_owned))
            .collect()
    };
    let list = |name: &str, cap: usize| -> Result<Vec<String>, &'static str> {
        strings(&value[name])
            .filter(|list| list.len() <= cap)
            .ok_or("one of its lists is missing, malformed or longer than the hooks write")
    };
    let project = match &value["project"] {
        Value::Null => None,
        project => {
            let read = || -> Option<Project> {
                let top = project["top"].as_str().filter(|top| !top.is_empty())?;
                let tips = strings(&project["tips"])?;
                if tips.len() > REFS_CAP + 1 || !tips.iter().all(|tip| is_commit_id(tip)) {
                    return None;
                }
                Some(Project {
                    top: top.to_owned(),
                    id: project["id"]
                        .as_str()
                        .filter(|id| is_identity(id))?
                        .to_owned(),
                    base: base_from(project["base"].as_str()?)?,
                    tips,
                    tips_partial: project["tips_partial"].as_bool()?,
                    by_start: project["by_start"].as_bool()?,
                })
            };
            Some(read().ok_or("its project is not whole")?)
        }
    };
    Ok(State {
        project,
        start_said: value["start_said"]
            .as_bool()
            .ok_or("one of its members is missing")?,
        reported: list("reported", REPORTED_CAP)?.into_iter().collect(),
        noticed: list("noticed", NOTICED_CAP)?.into_iter().collect(),
        blocked: list("blocked", BLOCKED_CAP)?,
    })
}

fn state_text(state: &State) -> String {
    json!({
        "schema": STATE_SCHEMA,
        "project": state.project.as_ref().map(|project| json!({
            "top": project.top,
            "id": project.id,
            "base": base_text(&project.base),
            "tips": project.tips,
            "tips_partial": project.tips_partial,
            "by_start": project.by_start,
        })),
        "start_said": state.start_said,
        "reported": state.reported,
        "noticed": state.noticed,
        "blocked": state.blocked,
    })
    .to_string()
}

/// The most bytes of a state file that are read. Every list in a state is bounded when it is
/// written ([`REPORTED_CAP`], [`NOTICED_CAP`], [`BLOCKED_CAP`], [`REFS_CAP`]), and a state
/// with every list full is under a quarter of this (a test holds it). Before, the failure
/// that blocked was kept with a line for each violation and no bound, so the hooks wrote a
/// file they would not read back.
const STATE_CAP: u64 = 4 * 1024 * 1024;

/// Read a session's state file: `Ok(None)` when there is none, `Err` with the reason when
/// something stands at its name that is not a state the hooks can read (a link, another kind
/// of file, a file over [`STATE_CAP`], other content). The file is opened without following a
/// link and without waiting on a FIFO, must be a regular file once open, and its size is
/// checked before anything is read (review V4, VD4-M-1).
fn load(path: &Path) -> Result<Option<State>, String> {
    use std::os::unix::fs::OpenOptionsExt as _;
    let file = match std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("it cannot be opened ({error})")),
    };
    let found = file
        .metadata()
        .map_err(|e| format!("it cannot be read ({e})"))?;
    if !found.is_file() {
        return Err("it is not a regular file".to_owned());
    }
    if found.len() > STATE_CAP {
        return Err("it is larger than any state the hooks write".to_owned());
    }
    let mut text = String::new();
    file.take(STATE_CAP)
        .read_to_string(&mut text)
        .map_err(|e| format!("it cannot be read ({e})"))?;
    state_from(&text).map(Some).map_err(str::to_owned)
}

/// Write the state beside its file and move it into place, so a reader never sees half of it.
/// The partial file is made new, with mode 0600 and without following a link; the move replaces
/// whatever stands at the state file's name, a link included, and follows nothing.
fn save(path: &Path, state: &State) -> Result<(), String> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    let text = state_text(state);
    // Never written: what the next hook would refuse to read.
    if text.len() as u64 > STATE_CAP {
        return Err("its state is larger than it reads back".to_owned());
    }
    let partial = path.with_extension(format!("{}.partial", std::process::id()));
    // what an earlier process with this id left behind
    let _ = std::fs::remove_file(&partial);
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(nix::libc::O_NOFOLLOW)
        .open(&partial)
        .and_then(|mut file| file.write_all(text.as_bytes()))
        .and_then(|()| std::fs::rename(&partial, path))
        .map_err(|e| {
            // what could not be moved into place is not left beside the state
            let _ = std::fs::remove_file(&partial);
            format!("its state could not be saved: {e}")
        })
}

/// A session's state, with the file it is kept in.
struct Session {
    /// The directory of every session's state.
    dir: PathBuf,
    file: PathBuf,
    state: State,
    /// The state as the file holds it; `None` when there is no file.
    kept: Option<State>,
}

impl Session {
    /// The state of the session `id`, read from the user's state directory; an empty state
    /// when it has no file there. Nothing is made or written here. `Err` when there is no
    /// directory a state could be kept in, when a link or a file stands where the directory
    /// should be, or when the session's file is there and cannot be read as a state: none of
    /// these is "no state".
    fn open(id: &str) -> Result<Self, String> {
        let dir = sessions_dir()?;
        match std::fs::symlink_metadata(&dir) {
            Ok(found) if !found.file_type().is_dir() => {
                return Err(format!(
                    "{} is not a directory of its own (a symbolic link or a file stands there)",
                    shown(&dir)
                ));
            }
            _ => {}
        }
        let file = state_file(&dir, id);
        let kept = load(&file).map_err(|why| {
            format!(
                "this session's state file {} cannot be read as a state ({why}), so the project and the commit this session began from are not known; nothing is recorded over it (remove the file to have the next hook record the session anew)",
                shown(&file)
            )
        })?;
        Ok(Self {
            dir,
            file,
            state: kept.clone().unwrap_or_default(),
            kept,
        })
    }

    /// Write the state if this hook changed it, making its directory first.
    fn keep(&mut self) -> Result<(), String> {
        if self.kept.as_ref() == Some(&self.state)
            || (self.kept.is_none() && self.state == State::default())
        {
            return Ok(());
        }
        if let Some(parent) = self.dir.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("its state directory could not be made: {e}"))?;
        }
        own_directory(&self.dir)?;
        save(&self.file, &self.state)?;
        self.kept = Some(self.state.clone());
        Ok(())
    }

    /// Why the state directory must not be used for the project at `top`: it lies inside that
    /// working tree. A state there would be an untracked file in every patch, and cleaning the
    /// tree would remove the session's base.
    fn inside(&self, top: &Path) -> Option<String> {
        inside_tree(&self.dir, top)
    }
}

/// Why the state directory `dir` must not be used for the project at `top`, if it lies inside
/// that working tree.
fn inside_tree(dir: &Path, top: &Path) -> Option<String> {
    resolved(dir).starts_with(top).then(|| {
        format!(
            "the state directory {} lies inside the project's working tree {}: point XDG_STATE_HOME (or HOME) outside it",
            shown(dir),
            shown(top)
        )
    })
}

/// `path` with its symbolic links followed as far as it exists: the part that exists is made
/// canonical and the rest is kept as written.
fn resolved(path: &Path) -> PathBuf {
    let mut rest = Vec::new();
    let mut existing = path;
    loop {
        if let Ok(canonical) = std::fs::canonicalize(existing) {
            return rest
                .iter()
                .rev()
                .fold(canonical, |whole, part| whole.join(part));
        }
        match (existing.parent(), existing.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name.to_owned());
                existing = parent;
            }
            _ => return path.to_path_buf(),
        }
    }
}

/// What a hook has to look at.
enum Located {
    /// Nothing, and nothing is said: Git ran, found no repository around the session's
    /// directory, and the session has no project recorded.
    Nowhere,
    /// Something that must be said: the project could not be found, and why.
    Trouble(String),
    Here(Box<Work>),
}

/// The session's project, and the commit its changes are measured from.
struct Work {
    place: Place,
    project: Project,
    /// This hook is the session's first here: it recorded the project and the base.
    fresh: bool,
    /// The recorded top level, when the project is no longer there and was found elsewhere.
    moved_from: Option<String>,
}

/// The commits the refs of the repository at `top` and its `HEAD` lead to now, each once, and
/// whether the repository has more refs than [`REFS_CAP`].
fn tips_now(
    top: &Path,
    head: Option<&str>,
    deadline: Deadline,
) -> Result<(Vec<String>, bool), String> {
    let refs = ref_tips(top, REFS_CAP, &deadline.git()).map_err(|error| {
        format!(
            "the repository's refs could not be listed: {}",
            git_words(&error)
        )
    })?;
    let mut tips: BTreeSet<String> = refs.tips.into_iter().map(|(_, commit)| commit).collect();
    tips.extend(head.map(str::to_owned));
    Ok((tips.into_iter().collect(), refs.partial))
}

/// Record the project at `place` for a session that has none: the commit `HEAD` names, the
/// commits the refs lead to, and which repository it is.
fn record(place: &Place, deadline: Deadline) -> Result<Project, String> {
    let Some(top) = place.top.to_str() else {
        return Err("the project's path is not UTF-8".to_owned());
    };
    let base = match head_state(&place.top, &deadline.git()) {
        Ok(HeadState::Commit(id)) if is_commit_id(&id) => Base::Commit(id),
        Ok(HeadState::Commit(_)) => {
            return Err("Git named the commit of HEAD in a form that cannot be read".to_owned());
        }
        Ok(HeadState::Unborn {
            repository_has_commits: false,
        }) => Base::NoCommit,
        Ok(HeadState::Unborn { .. }) => Base::Orphan,
        Err(error) => {
            return Err(format!(
                "the commit HEAD names could not be read: {}",
                git_words(&error)
            ));
        }
    };
    let head = match &base {
        Base::Commit(id) => Some(id.as_str()),
        _ => None,
    };
    let (tips, tips_partial) = tips_now(&place.top, head, deadline)?;
    Ok(Project {
        top: top.to_owned(),
        id: place.id.clone(),
        base,
        tips,
        tips_partial,
        by_start: false,
    })
}

/// Find the session's project, recording it in `state` when this is the session's first hook
/// that finds a repository.
///
/// With nothing recorded the project is the directory the client names (`CLAUDE_PROJECT_DIR`)
/// when it is in a Git working tree, else the hook input's `cwd`. The `cwd` alone used to
/// decide, and it follows the agent (independent review of the hooks, HR-H-1).
///
/// With a project recorded, the project is the repository that was recorded, not whatever
/// stands at its path: the first of the client's project directory, the recorded top level
/// and `cwd` that is in the repository with the recorded identity. When the project was moved
/// it is found, and judged, where `cwd` is. A repository of another identity at the recorded
/// path is not the project, however clean it is (review V4 round 10, VD4R10-H-1).
fn locate(input: &Input, state: &mut State, deadline: Deadline) -> Located {
    let named = match client_project_dir().map(|dir| find(&dir, deadline)) {
        Some(Found::Tree(found)) => Some(found),
        Some(Found::Failed(reason)) => return Located::Trouble(reason),
        Some(Found::NotARepository | Found::Gone) | None => None,
    };
    let Some(project) = state.project.clone() else {
        let place = if let Some(place) = named {
            place
        } else {
            let cwd = Path::new(&input.cwd);
            match find(cwd, deadline) {
                Found::Tree(found) => found,
                Found::NotARepository => return Located::Nowhere,
                Found::Gone => {
                    return Located::Trouble(format!(
                        "the session's directory {} does not exist, and no project is recorded for this session",
                        shown(cwd)
                    ));
                }
                Found::Failed(reason) => return Located::Trouble(reason),
            }
        };
        return match record(&place, deadline) {
            Ok(project) => {
                state.project = Some(project.clone());
                Located::Here(Box::new(Work {
                    place,
                    project,
                    fresh: true,
                    moved_from: None,
                }))
            }
            Err(reason) => Located::Trouble(reason),
        };
    };
    let recorded = Path::new(&project.top);
    let here = |place: Place| {
        let moved_from = (place.top != recorded).then(|| project.top.clone());
        Located::Here(Box::new(Work {
            place,
            project: project.clone(),
            fresh: false,
            moved_from,
        }))
    };
    // The client names a repository: the recorded one, or another.
    if let Some(place) = named {
        if place.id == project.id {
            return here(place);
        }
        if place.top != recorded {
            return Located::Trouble(format!(
                "this session began in {}, and the project found now is {}",
                hook::shown_path(&project.top),
                shown(&place.top)
            ));
        }
    }
    let at_path = match find(recorded, deadline) {
        Found::Tree(found) if found.id == project.id => return here(found),
        Found::Failed(reason) => return Located::Trouble(reason),
        Found::Tree(_) => true,
        Found::NotARepository | Found::Gone => false,
    };
    // The project is not at its path any more. It may have been moved to where the session is.
    if let Found::Tree(found) = find(Path::new(&input.cwd), deadline)
        && found.id == project.id
    {
        return here(found);
    }
    Located::Trouble(if at_path {
        format!(
            "the repository at {} is not the one this session started in (another repository stands at that path now)",
            hook::shown_path(&project.top)
        )
    } else {
        format!(
            "the project this session began in, {}, is no longer a Git working tree",
            hook::shown_path(&project.top)
        )
    })
}

/// What a hook compares the working tree with.
enum Subject {
    /// Nothing, and nothing is said: the repository had no commit when the session began and
    /// has none now.
    Nothing,
    /// Nothing, and that must be said.
    Unjudged(String),
    /// The commit the session began from, with the words a message says it in.
    Range { base: String, words: String },
}

const ORPHAN: &str = "this session began on a branch with no commit (an orphan branch) in a repository that has commits, so there is no commit to measure its changes from";

fn subject(work: &Work, deadline: Deadline) -> Subject {
    match &work.project.base {
        Base::Commit(id) => {
            // A base that can no longer be read is found, and said, when the patch is made.
            let moved = matches!(
                head_contains(&work.place.top, id, &deadline.git()),
                Ok(false)
            );
            Subject::Range {
                base: id.clone(),
                words: hook::range_words(id, !work.project.by_start, moved),
            }
        }
        Base::Orphan => Subject::Unjudged(ORPHAN.to_owned()),
        Base::NoCommit if work.fresh => Subject::Nothing,
        Base::NoCommit => match head_state(&work.place.top, &deadline.git()) {
            Ok(HeadState::Unborn {
                repository_has_commits: false,
            }) => Subject::Nothing,
            Ok(_) => Subject::Unjudged(
                "the session began before the first commit of this repository, so there is no commit to measure its changes from"
                    .to_owned(),
            ),
            Err(error) => Subject::Unjudged(format!(
                "the commit HEAD names could not be read: {}",
                git_words(&error)
            )),
        },
    }
}

/// Every piece of evidence in `patch`, files in ranking order, each with the key that names it.
fn findings(patch: &[u8]) -> Result<Vec<Finding>, String> {
    let review = SealedReview::new(
        patch,
        InputGuards {
            files: false,
            hunks: false,
        },
        env!("CARGO_PKG_VERSION"),
    )
    .map_err(|e| format!("the change could not be read as a patch: {e}"))?;
    let files = review.files();
    let mut out = Vec::new();
    for ranked in deep_diff_forge_graph::rank(files) {
        let Some(file) = files.iter().find(|f| f.path == ranked.path) else {
            continue;
        };
        for evidence in &ranked.evidence {
            let line = file
                .patch_twin
                .hunks
                .iter()
                .find(|h| h.id == evidence.hunk)
                .and_then(|h| {
                    h.lines.iter().find(|l| match evidence.side {
                        Side::New => {
                            l.kind == PatchLineKind::Added && l.new_line == Some(evidence.line)
                        }
                        Side::Old => {
                            l.kind == PatchLineKind::Removed && l.old_line == Some(evidence.line)
                        }
                    })
                })
                .map_or("", |l| l.text.as_str());
            out.push(Finding {
                label: evidence.signal.label().to_owned(),
                path: ranked.path.clone(),
                side: evidence.side.label().to_owned(),
                line: evidence.line,
                basis: deep_diff_forge_graph::EVIDENCE_BASIS.to_owned(),
                key: hook::finding_key(&ranked.path, evidence.signal.label(), line),
            });
        }
    }
    Ok(out)
}

/// A working tree as the patch a judge would be given, with the nested repositories that were
/// left out of it.
struct Change {
    /// `None`: the tree equals the base.
    patch: Option<Vec<u8>>,
    nested: Vec<String>,
}

/// The working tree at `top` as the patch a judge would be given against the commit `base`.
/// A tree that equals the base has no patch: only that is "no change". Every failure is an
/// error with its reason: when one answer stood for "nothing to judge" and for failures alike,
/// a patch that could not be made let a stop pass in silence.
fn change(top: &Path, base: &str, work: &Path, deadline: Deadline) -> Result<Change, String> {
    match judge_patch(top, base, work, &deadline.git()) {
        Ok(patch) => Ok(Change {
            patch: Some(patch.bytes).filter(|bytes| !bytes.is_empty()),
            nested: patch.nested,
        }),
        Err(JudgePatchError::NoBase) => Err(format!(
            "the commit this session began from ({}) can no longer be read from the repository",
            base.chars().take(7).collect::<String>()
        )),
        Err(JudgePatchError::Failed(error)) => Err(format!(
            "the change could not be read from Git: {}",
            git_words(&error)
        )),
        Err(error @ JudgePatchError::SkipWorktree) => {
            Err(format!("the change could not be read from Git: {error}"))
        }
    }
}

/// Whether `what` is said now, once in a session: `false` when it was said before, or when
/// [`NOTICED_CAP`] things were.
fn once(state: &mut State, what: &str) -> bool {
    state.noticed.len() < NOTICED_CAP
        && state
            .noticed
            .insert(deep_diff_forge_core::sha256_hex(what.as_bytes()))
}

/// Tell a reason an advice hook has for not running, once in a session: `None` when it was
/// told before, or when [`NOTICED_CAP`] things were. A lasting condition (an untracked file
/// that cannot be read, a sparse checkout, Git refusing the repository) used to be said after
/// every tool call, or never (independent review of the hooks, HR-L-2 and HR-M-1).
fn notice(state: &mut State, reason: &str) -> Option<String> {
    once(state, reason).then(|| hook::notice_output(reason, state.noticed.len() == NOTICED_CAP))
}

/// The most marks [`said_beside`] leaves in one repository.
const MARKS_KEPT: usize = 256;

/// Tell `reason` in an advice hook once when the session's state cannot remember that it was
/// told: the state directory is missing, cannot be written, or holds a file that cannot be
/// read. What was said is marked by an empty file beside the scratch directory, in the hooks'
/// own directory `own` of the repository. With no such directory, or no room for a mark, the
/// advice hook says nothing: it used to say "the hook did not run" after every tool call
/// (second independent review of the hooks, HR2-L-2). A stop says the reason every time.
fn said_beside(own: Option<&Path>, session: &str, reason: &str) -> Option<String> {
    use std::os::unix::fs::OpenOptionsExt as _;
    let own = own?;
    own_directory(own).ok()?;
    let marks = std::fs::read_dir(own)
        .ok()?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("said-"))
        .take(MARKS_KEPT + 1)
        .count();
    if marks > MARKS_KEPT {
        return None;
    }
    let digest = |text: &str| deep_diff_forge_core::sha256_hex(text.as_bytes())[..16].to_owned();
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(nix::libc::O_NOFOLLOW)
        .open(own.join(format!("said-{}-{}", digest(session), digest(reason))))
        .ok()
        .map(|_| hook::notice_output(reason, false))
}

/// The hooks' own directory in the repository a hook that has no state would work in: the
/// client's project directory, else the hook input's `cwd`.
fn own_without_state(input: &Input, deadline: Deadline) -> Option<PathBuf> {
    let dir = client_project_dir().unwrap_or_else(|| PathBuf::from(&input.cwd));
    match find(&dir, deadline) {
        Found::Tree(place) => Some(place.own),
        _ => None,
    }
}

/// After a tool ran: say what is new in the session's change. Advice only.
fn post_tool(input: &Input) -> Option<String> {
    let deadline = Deadline::after(ADVICE_LIMIT);
    let mut session = match Session::open(&input.session) {
        Ok(session) => session,
        Err(why) => {
            return said_beside(
                own_without_state(input, deadline).as_deref(),
                &input.session,
                &why,
            );
        }
    };
    let (own, advice) = match locate(input, &mut session.state, deadline) {
        Located::Nowhere => return None,
        Located::Trouble(reason) => (None, Err(reason)),
        Located::Here(work) => {
            if let Some(why) = session.inside(&work.place.top) {
                return said_beside(Some(&work.place.own), &input.session, &why);
            }
            let advice = advice_for(&work, &mut session.state, deadline);
            (Some(work.place.own), advice)
        }
    };
    let out = match advice {
        Ok(advice) => advice,
        Err(reason) => notice(&mut session.state, &reason),
    };
    match session.keep() {
        Ok(()) => out,
        // What was to be said is not: the state would not remember it, and it would be said
        // again after every tool call.
        Err(why) => said_beside(
            own.or_else(|| own_without_state(input, deadline))
                .as_deref(),
            &input.session,
            &why,
        ),
    }
}

/// The advice for what is new in the project's change, `None` when there is nothing new, or
/// why no advice can be given.
fn advice_for(
    work: &Work,
    state: &mut State,
    deadline: Deadline,
) -> Result<Option<String>, String> {
    let (base, range) = match subject(work, deadline) {
        Subject::Nothing => return Ok(None),
        Subject::Unjudged(reason) => return Err(reason),
        Subject::Range { base, words } => (base, words),
    };
    let scratch = Scratch::new(&work.place.own)?;
    let Some(patch) = change(&work.place.top, &base, &scratch.sub("work")?, deadline)?.patch else {
        return Ok(None);
    };
    let all = findings(&patch)?;
    let fresh = hook::fresh(&all, &state.reported);
    let Some(advice) = hook::advice(&fresh, &range) else {
        return Ok(None);
    };
    let said: Vec<String> = fresh
        .iter()
        .take(advice.listed)
        .map(|f| f.key.clone())
        .collect();
    if state.reported.len() + said.len() > REPORTED_CAP {
        state.reported.clear();
    }
    state.reported.extend(said);
    Ok(hook::post_tool_output(Some(&advice.text)))
}

/// The gate checks whose detail starts with this were not made, because an earlier link is
/// missing.
const NOT_CHECKED: &str = "not checked: ";

/// At the start of a session: what the hooks will do, what the session's changes are measured
/// from, and whether a stop can be judged here.
fn session_start(input: &Input, args: &[String]) -> Option<String> {
    let deadline = Deadline::after(START_LIMIT);
    let mut session = match Session::open(&input.session) {
        Ok(session) => session,
        Err(why) => return Some(hook::contract_trouble(&why)),
    };
    let work = match locate(input, &mut session.state, deadline) {
        Located::Nowhere => return None,
        Located::Trouble(reason) => return Some(hook::contract_trouble(&reason)),
        Located::Here(work) => work,
    };
    if let Some(why) = session.inside(&work.place.top) {
        return Some(hook::contract_trouble(&why));
    }
    if work.fresh
        && let Some(project) = &mut session.state.project
    {
        project.by_start = true;
    }
    let unsaved = session.keep().err();
    let top = work.place.top.to_string_lossy();
    let id = match &work.project.base {
        Base::Commit(id) => id,
        Base::NoCommit => {
            return Some(hook::contract(
                &top,
                StartBase::NoCommit,
                &[],
                false,
                unsaved.as_deref(),
            ));
        }
        Base::Orphan => {
            return Some(hook::contract(
                &top,
                StartBase::Orphan,
                &[],
                false,
                unsaved.as_deref(),
            ));
        }
    };
    // The same links `agent doctor --gate` checks, in the time a start has.
    let gate = gate_in(&work.place.top, Ok(&work.place), Some(id), args, deadline);
    let not_ready: Vec<(String, String)> = gate
        .checks
        .iter()
        .filter(|check| !check.ok && !check.detail.starts_with(NOT_CHECKED))
        .map(|check| (check.name.to_owned(), check.detail.clone()))
        .collect();
    Some(hook::contract(
        &top,
        StartBase::Commit {
            id,
            resumed: !work.fresh,
        },
        &not_ready,
        gate.dirty,
        unsaved.as_deref(),
    ))
}

fn policy_name(args: &[String]) -> String {
    crate::flag_value(args, "--policy").unwrap_or_else(|| DEFAULT_POLICY.to_owned())
}

/// The judge: `--judge PATH`, or the first `forge-warden` in a directory of `PATH` that is
/// named by an absolute path. `Err` says why there is none to run.
///
/// A judge inside the repository it would judge (`repository`: its top level and its Git
/// common directory) is refused, whichever way it was named: the change under judgement could
/// have written it. So is a `PATH` entry that is not absolute,
/// which is looked up in whatever directory the hook happens to run in (review V4, VD4-H-1).
/// `agent setup --hooks` prints the judge it finds as an absolute `--judge`, so that the
/// program is chosen once, by the person who installs the hooks, and not at every stop by
/// `PATH`.
#[expect(
    clippy::disallowed_methods,
    reason = "the judge door: forge-warden is looked up on PATH when --judge names no file"
)]
pub(crate) fn judge_program(args: &[String], repository: &[&Path]) -> Result<PathBuf, String> {
    let found = match crate::flag_value(args, "--judge") {
        Some(named) => {
            let path = PathBuf::from(named);
            path.is_file().then_some(path)
        }
        None => std::env::var_os("PATH").and_then(|paths| {
            std::env::split_paths(&paths)
                .filter(|dir| dir.is_absolute())
                .map(|dir| dir.join("forge-warden"))
                .find(|candidate| candidate.is_file())
        }),
    };
    let program = found
        .and_then(|path| std::fs::canonicalize(path).ok())
        .ok_or_else(|| {
            "forge-warden was not found (install it, or name it with --judge PATH)".to_owned()
        })?;
    let inside = repository
        .iter()
        .filter_map(|root| std::fs::canonicalize(root).ok())
        .any(|root| program.starts_with(root));
    if inside {
        return Err(format!(
            "the judge {} is inside the repository it would judge, where a change can replace it",
            hook::shown_path(&program.to_string_lossy())
        ));
    }
    Ok(program)
}

/// The rankers a stop may offer the judge, in order: this binary, then the files in
/// `--ranker-dir` that come first by name, [`RANKERS_TRIED`] in all, each one a path the
/// helper can write in its list. The judge checks each
/// one's hash against the policy itself, so nothing here has to read the policy.
fn rankers(args: &[String]) -> Vec<PathBuf> {
    // This binary takes the first place only when the helper can name it to its parent: a
    // path that is not UTF-8 would be listed as null and dropped there, having cost a ranker
    // of the directory its place (review V4 round 8, the rest of VD4R7-L-1).
    let mut list: Vec<PathBuf> = std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .into_iter()
        .filter(|path| path.to_str().is_some())
        .collect();
    if let Some(dir) = crate::flag_value(args, "--ranker-dir")
        && let Ok(entries) = std::fs::read_dir(dir)
    {
        list.extend(first_by_name(
            entries.filter_map(Result::ok).map(|entry| entry.path()),
            RANKERS_TRIED.saturating_sub(list.len()),
            offerable,
        ));
    }
    list
}

/// Whether `path` is a ranker the helper can offer: a regular file whose path it can write in
/// its list, which is JSON, so the path is UTF-8. One that it cannot name must not take a
/// place (review V4 round 7, VD4R7-L-1: a file named `a<0xff>` took one of the seven places
/// and was dropped by the parent, so the seventh file that could be offered never was).
fn offerable(path: &Path) -> bool {
    path.to_str().is_some() && path.is_file()
}

/// The `keep` paths of `paths` that come first in order and that `wanted` accepts, in order.
///
/// It holds no more than `keep` paths at any moment, and asks `wanted` only of a path that
/// would be kept (review V4 round 6, VD4R6-M-1: the directory was collected whole and cut to
/// eight afterwards, so a directory of 500,000 empty files cost 69 MiB and as many `stat`
/// calls for the seven it could ever offer).
fn first_by_name(
    paths: impl Iterator<Item = PathBuf>,
    keep: usize,
    wanted: impl Fn(&Path) -> bool,
) -> Vec<PathBuf> {
    let mut kept = std::collections::BinaryHeap::with_capacity(keep.saturating_add(1));
    for path in paths {
        let later = kept.len() >= keep && kept.peek().is_none_or(|last| path >= *last);
        if later || !wanted(&path) {
            continue;
        }
        kept.push(path);
        if kept.len() > keep {
            kept.pop();
        }
    }
    kept.into_sorted_vec()
}

/// The most characters of the judge's own error message a reason repeats.
const JUDGE_WORDS: usize = 300;

/// What the judge said when it gave no verdict: the `name` and `message` of the last JSON line
/// on its error stream (`forge-warden.error.v1`), or that line as text.
fn judge_words(stderr: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(stderr);
    let last = text.lines().rev().find(|line| !line.trim().is_empty())?;
    let words = match serde_json::from_str::<Value>(last) {
        Ok(error) => match (error["name"].as_str(), error["message"].as_str()) {
            (Some(name), Some(message)) => format!("{name}: {message}"),
            (None, Some(message)) => message.to_owned(),
            _ => last.trim().to_owned(),
        },
        Err(_) => last.trim().to_owned(),
    };
    Some(words.chars().take(JUDGE_WORDS).collect())
}

/// Run the judge once: its exit code, its verdict document and what it said on its error
/// stream, when it gave any of them. It gets the time the hook has left, at most
/// [`JUDGE_TIME`].
#[expect(
    clippy::disallowed_methods,
    reason = "the judge door: the stop hook starts forge-warden, under a deadline and output caps"
)]
fn run_judge(
    judge: &Path,
    policy: &Path,
    patch: &Path,
    ranker: &Path,
    deadline: Deadline,
) -> (Option<i32>, Option<Value>, Option<String>) {
    let mut command = std::process::Command::new(judge);
    command
        .arg("check")
        .arg("--policy")
        .arg(policy)
        .arg("--patch")
        .arg(patch)
        .arg("--ranker")
        .arg(ranker)
        .arg("--json");
    let limits = Limits {
        time: deadline.left().min(JUDGE_TIME),
        stdout_bytes: JUDGE_STDOUT,
        stderr_bytes: JUDGE_STDERR,
    };
    match run_bounded(&mut command, b"", limits) {
        Ok(done) => {
            // A document that gives a key twice is not read: two readers of it can disagree.
            let verdict = std::str::from_utf8(&done.stdout)
                .ok()
                .filter(|text| !hook::has_duplicate_key(text))
                .and_then(|text| serde_json::from_str(text).ok());
            (done.status.code(), verdict, judge_words(&done.stderr))
        }
        Err(stopped) => (None, None, Some(stopped.to_string())),
    }
}

/// What every working tree of one stop is judged with: the judge, the policy at the base
/// commit, and the rankers with their digests.
struct Bench<'a> {
    scratch: &'a Scratch,
    program: PathBuf,
    policy_file: PathBuf,
    policy_sha256: String,
    candidates: Candidates,
    deadline: Deadline,
}

impl<'a> Bench<'a> {
    /// Find the judge, read the policy from the commit `base` and have the rankers hashed.
    /// `Err` is why nothing can be judged. `writable` are the directories a change under
    /// judgement can write to: no judge is run from them.
    fn new(
        top: &Path,
        base: &str,
        scratch: &'a Scratch,
        args: &[String],
        writable: &[&Path],
        deadline: Deadline,
    ) -> Result<Self, String> {
        // The policy is the one at the commit the session began from: a change cannot loosen
        // the policy it is judged by, by committing it either.
        let name = policy_name(args);
        let policy = match file_at_commit(top, base, &name, &deadline.git()) {
            Ok(Some(bytes)) => bytes,
            Ok(None) => {
                return Err(format!(
                    "the base commit has no policy file {}",
                    hook::shown_path(&name)
                ));
            }
            Err(error) => {
                return Err(format!(
                    "the policy could not be read from Git: {}",
                    git_words(&error)
                ));
            }
        };
        let program = judge_program(args, writable)?;
        let policy_file = scratch.0.join("policy.toml");
        std::fs::write(&policy_file, &policy)
            .map_err(|e| format!("the judge's input could not be written: {e}"))?;
        Ok(Self {
            scratch,
            program,
            policy_file,
            policy_sha256: deep_diff_forge_core::sha256_hex(&policy),
            candidates: sealed_rankers(args, deadline)?,
            deadline,
        })
    }

    /// Have `patch` judged; `name` is the file it is written to in the scratch directory.
    fn judge(&self, patch: &[u8], name: &str) -> Judged {
        let patch_file = self.scratch.0.join(name);
        if let Err(error) = std::fs::write(&patch_file, patch) {
            return Judged::Unjudged {
                reason: format!("the judge's input could not be written: {error}"),
            };
        }
        let patch_sha256 = deep_diff_forge_core::sha256_hex(patch);
        let mut last = Judged::Unjudged {
            reason: "this binary's own path could not be found, so no ranker could be offered"
                .to_owned(),
        };
        for (ranker, digest) in &self.candidates {
            // The verdict must name the ranker it was given, by its digest.
            let ranker_sha256 = match digest {
                Ok(digest) => digest,
                Err(why) => {
                    last = Judged::Unjudged {
                        reason: format!(
                            "the ranker {} {why}, so no verdict could be sealed to it",
                            shown(ranker)
                        ),
                    };
                    continue;
                }
            };
            let sealed = hook::Sealed {
                policy: &self.policy_sha256,
                patch: &patch_sha256,
                ranker: ranker_sha256,
            };
            let (code, verdict, said) = run_judge(
                &self.program,
                &self.policy_file,
                &patch_file,
                ranker,
                self.deadline,
            );
            last = hook::judged(code, verdict.as_ref(), said.as_deref(), sealed);
            // Exit 4: the policy does not admit this ranker. Another may be admitted.
            if code != Some(4) {
                break;
            }
        }
        last
    }
}

/// Where the session's directory is at a stop, beside the project's own working tree.
enum Elsewhere {
    /// Nowhere that adds anything: in the project's tree, in no repository, or gone.
    Nothing,
    /// In a linked working tree of the project's repository, with this top level: that tree is
    /// judged as well.
    Linked(PathBuf),
    /// In another repository, with this top level (a nested repository and a submodule are
    /// repositories of their own): it is not the session's project and is not judged, which is
    /// said.
    Foreign(PathBuf),
    /// Git could not say.
    Failed(String),
}

/// Look at the hook input's `cwd`. The project is judged wherever `cwd` is; this only finds
/// what else there is to judge or to name.
fn elsewhere(cwd: &str, place: &Place, deadline: Deadline) -> Elsewhere {
    let Ok(cwd) = std::fs::canonicalize(cwd) else {
        return Elsewhere::Nothing;
    };
    // The project's own top level, or a place inside the Git directory, where no working
    // tree is.
    if cwd == place.top || cwd.starts_with(&place.common) {
        return Elsewhere::Nothing;
    }
    match find(&cwd, deadline) {
        Found::Tree(found) if found.id != place.id => Elsewhere::Foreign(found.top),
        Found::Tree(found) if found.top != place.top => Elsewhere::Linked(found.top),
        Found::Tree(_) | Found::NotARepository | Found::Gone => Elsewhere::Nothing,
        Found::Failed(reason) => Elsewhere::Failed(format!(
            "whether the session's directory is another working tree of this repository could not be found out: {reason}"
        )),
    }
}

/// The most distinct tips of refs that one stop examines for commits made since the session
/// began.
const TIPS_EXAMINED: usize = 16;
/// The most distinct patches one stop has judged.
const MAX_PATCHES: usize = 4;

/// The commits made since the session began that something still names.
#[derive(Debug, Default, PartialEq, Eq)]
struct SessionCommits {
    /// Each commit that `HEAD` or a ref names now and that leads to a commit none of the
    /// recorded tips leads to, with every name it has (`HEAD` first, then refs by name).
    holders: Vec<(Vec<String>, String)>,
    /// The refs that changed and were not examined: more tips changed than
    /// [`TIPS_EXAMINED`].
    not_examined: Vec<String>,
    /// The refs with new commits of which it cannot be said whether the session made them:
    /// the repository had, or has, more refs than [`REFS_CAP`].
    not_known: Vec<String>,
}

/// Find what the session committed and a ref still names: the commits `HEAD`, a branch, a
/// remote-tracking ref, a tag or the stash leads to now and none of the tips recorded at the
/// session's first hook leads to.
///
/// The working tree alone used to be compared with the base. Work that was committed and was
/// no longer in the tree at the stop (a branch the session left, a commit of a staged file
/// that was edited again, a commit the tree was put back from) was never shown to the judge,
/// and the stop said nothing (second independent review of the hooks, HR2-H-1).
fn session_commits(
    place: &Place,
    project: &Project,
    deadline: Deadline,
) -> Result<SessionCommits, String> {
    let words = |error: &GitAcquireError| {
        format!(
            "the commits made since the session began could not be listed: {}",
            git_words(error)
        )
    };
    let head = match head_state(&place.top, &deadline.git()).map_err(|e| words(&e))? {
        HeadState::Commit(id) => Some(id),
        HeadState::Unborn { .. } => None,
    };
    let refs = ref_tips(&place.top, REFS_CAP, &deadline.git()).map_err(|e| words(&e))?;
    let partial = project.tips_partial || refs.partial;
    // Each commit that is named now and was no tip then, with its names.
    let mut changed: Vec<(Vec<String>, String)> = Vec::new();
    let now = head
        .into_iter()
        .map(|id| ("HEAD".to_owned(), id))
        .chain(refs.tips);
    for (name, id) in now {
        if project.tips.contains(&id) {
            continue;
        }
        match changed.iter_mut().find(|(_, known)| *known == id) {
            Some((names, _)) => names.push(name),
            None => changed.push((vec![name], id)),
        }
    }
    let mut found = SessionCommits::default();
    for (at, (names, id)) in changed.into_iter().enumerate() {
        if at >= TIPS_EXAMINED {
            found.not_examined.extend(names);
            continue;
        }
        // A tip that only moved back, or to a commit that was already there, holds nothing new.
        if !has_commits_beyond(&place.top, &id, &project.tips, &deadline.git())
            .map_err(|e| words(&e))?
        {
            continue;
        }
        if partial && names.first().is_none_or(|name| name != "HEAD") {
            found.not_known.extend(names);
        } else {
            found.holders.push((names, id));
        }
    }
    Ok(found)
}

/// One thing a stop shows the judge: what it is called, and its patch (`None`: no change), or
/// why there is none.
struct Shown {
    what: Vec<String>,
    patch: Result<Option<Vec<u8>>, String>,
}

/// The patches of the commits the session made that a ref still names: for each holder, from
/// the best common ancestor of the base and that commit (the base itself when the commit comes
/// after it; nothing at all when they share no history) to the commit.
fn commit_patches(
    place: &Place,
    project: &Project,
    base: &str,
    scratch: &Scratch,
    deadline: Deadline,
) -> Result<Vec<Shown>, String> {
    let commits = match session_commits(place, project, deadline) {
        Ok(commits) => commits,
        Err(reason) => {
            return Ok(vec![Shown {
                what: vec!["Commits made since the session began".to_owned()],
                patch: Err(reason),
            }]);
        }
    };
    let mut shown = Vec::new();
    for (at, (names, id)) in commits.holders.iter().enumerate() {
        let work = scratch.sub(&format!("commits-{at}"))?;
        let patch = merge_base(&place.top, base, id, &deadline.git())
            .map_err(JudgePatchError::Failed)
            .and_then(|from| commit_patch(&place.top, from.as_deref(), id, &work, &deadline.git()))
            .map(|bytes| Some(bytes).filter(|bytes| !bytes.is_empty()))
            .map_err(|error| match error {
                JudgePatchError::Failed(error) => format!(
                    "the commits could not be read from Git: {}",
                    git_words(&error)
                ),
                other => format!("the commits could not be read from Git: {other}"),
            });
        // Commits that change nothing between them are nothing to judge, and are not named.
        if !matches!(patch, Ok(None)) {
            shown.push(Shown {
                what: vec![hook::ref_label(names)],
                patch,
            });
        }
    }
    for (names, reason) in [
        (
            &commits.not_examined,
            "more refs changed since the session began than one stop examines, so these were not judged",
        ),
        (
            &commits.not_known,
            "the repository has more refs than were recorded when the session began, so whether these commits were made since is not known and they were not judged",
        ),
    ] {
        if !names.is_empty() {
            shown.push(Shown {
                what: vec![hook::ref_label(names)],
                patch: Err(reason.to_owned()),
            });
        }
    }
    Ok(shown)
}

/// Make one of things that have the same patch, and leave no more than [`MAX_PATCHES`]
/// patches to judge: the usual stop (the working tree is what `HEAD` holds, on one branch) is
/// one patch and one run of the judge.
fn distinct(shown: Vec<Shown>) -> Vec<Shown> {
    let mut kept: Vec<(Option<String>, Shown)> = Vec::new();
    for one in shown {
        let digest = match &one.patch {
            Ok(Some(bytes)) => Some(deep_diff_forge_core::sha256_hex(bytes)),
            _ => None,
        };
        match kept
            .iter_mut()
            .find(|(known, _)| digest.is_some() && *known == digest)
        {
            Some((_, same)) => same.what.extend(one.what),
            None => kept.push((digest, one)),
        }
    }
    let mut patches = 0;
    kept.into_iter()
        .map(|(digest, mut one)| {
            if digest.is_some() {
                patches += 1;
                if patches > MAX_PATCHES {
                    one.patch = Err(format!(
                        "one stop has at most {MAX_PATCHES} distinct patches judged, and this one is beyond them"
                    ));
                }
            }
            one
        })
        .collect()
}

/// Have the session's change judged: the project's working tree; a linked working tree of the
/// same repository that the session entered; and the commits made since the session began
/// that `HEAD` or a ref names. Each is compared with the same base and judged by the same
/// policy; things with the same patch are judged once. Something that equals the base has
/// nothing to judge; when that is all of them, no judge is needed and none is looked for.
///
/// With the trees comes what must be said beside the verdict, each thing once in a session:
/// a nested repository that was left out, another repository the session is in, a submodule
/// with changes that are not committed in it.
fn judge_all(
    work: &Work,
    cwd: &str,
    base: &str,
    scratch: &Scratch,
    args: &[String],
    deadline: Deadline,
) -> Result<(Vec<TreeJudged>, Vec<String>), String> {
    let place = &work.place;
    let mut beside = Vec::new();
    let mut tops = vec![place.top.clone()];
    let mut shown = Vec::new();
    let tree = |top: &Path, name: &str, beside: &mut Vec<String>| -> Result<Shown, String> {
        let changed = change(top, base, &scratch.sub(name)?, deadline);
        if let Ok(changed) = &changed {
            beside.extend(changed.nested.iter().map(|nested| {
                format!(
                    "The directory {} in {} is a Git repository of its own and no part of what this project would commit: it was left out and not judged.",
                    hook::shown_path(nested),
                    shown_tree(top)
                )
            }));
        }
        Ok(Shown {
            what: vec![hook::tree_label(&top.to_string_lossy())],
            patch: changed.map(|changed| changed.patch),
        })
    };
    shown.push(tree(&place.top, "work", &mut beside)?);
    match elsewhere(cwd, place, deadline) {
        Elsewhere::Nothing => {}
        Elsewhere::Linked(top) => {
            shown.push(tree(&top, "work-2", &mut beside)?);
            tops.push(top);
        }
        Elsewhere::Foreign(top) => beside.push(format!(
            "The session's directory is in another repository, {}, which is not this session's project ({}): nothing in it was judged.",
            shown_tree(&top),
            shown_tree(&place.top)
        )),
        Elsewhere::Failed(reason) => shown.push(Shown {
            what: vec![hook::tree_label(cwd)],
            patch: Err(reason),
        }),
    }
    shown.extend(commit_patches(
        place,
        &work.project,
        base,
        scratch,
        deadline,
    )?);
    beside.extend(submodules_with_changes(place, deadline));
    let shown = distinct(shown);
    // A judge is looked for only when something has a patch for it.
    let bench = if shown.iter().any(|one| matches!(one.patch, Ok(Some(_)))) {
        let mut writable: Vec<&Path> = tops.iter().map(PathBuf::as_path).collect();
        writable.push(&place.common);
        Some(Bench::new(
            &place.top, base, scratch, args, &writable, deadline,
        ))
    } else {
        None
    };
    let trees = shown
        .iter()
        .enumerate()
        .map(|(at, one)| TreeJudged {
            what: one.what.join(" and "),
            judged: match (&one.patch, &bench) {
                (Ok(None), _) => None,
                (Err(reason), _) | (Ok(Some(_)), Some(Err(reason))) => Some(Judged::Unjudged {
                    reason: reason.clone(),
                }),
                (Ok(Some(bytes)), Some(Ok(bench))) => {
                    Some(bench.judge(bytes, &format!("change-{at}.patch")))
                }
                // A patch with no bench cannot be: the bench is made when anything has one.
                (Ok(Some(_)), None) => Some(Judged::Unjudged {
                    reason: "the judge was not prepared".to_owned(),
                }),
            },
        })
        .collect();
    Ok((trees, beside))
}

fn shown_tree(top: &Path) -> String {
    hook::shown_path(&top.to_string_lossy())
}

/// What to say of the project's submodules that have changes not committed in them: the
/// project's patch shows a submodule only as the commit it is at, so those changes are in no
/// patch (second independent review of the hooks, HR2-M-3). Nothing is asked of Git when the
/// project has no `.gitmodules` file.
fn submodules_with_changes(place: &Place, deadline: Deadline) -> Vec<String> {
    if !place.top.join(".gitmodules").exists() {
        return Vec::new();
    }
    match dirty_submodules(&place.top, &deadline.git()) {
        Ok(paths) => paths
            .iter()
            .map(|path| {
                format!(
                    "The submodule {} of {} has changes that are not committed in it: they were not judged (the project's patch shows a submodule only as the commit it is at).",
                    hook::shown_path(path),
                    shown_tree(&place.top)
                )
            })
            .collect(),
        Err(error) => vec![format!(
            "Whether a submodule of {} has changes that are not committed in it could not be found out ({}): such changes are not judged.",
            shown_tree(&place.top),
            git_words(&error)
        )],
    }
}

/// The word that starts the stop hook's helper: `agent hook seal-rankers [--ranker-dir DIR]`.
/// It is not an event of a client and is not in the usage text.
const SEAL_WORD: &str = "seal-rankers";

/// The helper's whole job: list the candidate rankers and hash each, and print one JSON list of
/// `{"path", "sha256"}` or `{"path", "refused"}`. It runs as a child so that the stop hook can
/// end it: listing a directory and reading a file can block for as long as a file system
/// likes, and a deadline that is only looked at between two reads does not bound that (review
/// V4 round 5, on VD4R4-M-2).
fn seal_rankers(args: &[String]) {
    let deadline = now() + RANKER_SEAL_LIMIT;
    let list: Vec<Value> = rankers(args)
        .iter()
        .map(|ranker| {
            let path = ranker.to_str();
            match file_sha256(ranker, deadline) {
                Ok(digest) => json!({"path": path, "sha256": digest}),
                Err(why) => json!({"path": path, "refused": why}),
            }
        })
        .collect();
    emitln!("{}", Value::Array(list));
}

/// The candidate rankers with the SHA-256 of each, or why one has none, in the order they are
/// offered. `Err` when there is no list at all.
type Candidates = Vec<(PathBuf, Result<String, String>)>;

/// Have the helper list and hash the rankers, as a child that is killed when
/// [`RANKER_SEAL_LIMIT`] has passed, or the hook's own deadline if that comes first.
fn sealed_rankers(args: &[String], deadline: Deadline) -> Result<Candidates, String> {
    let this = std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .map_err(|_| {
            "this binary's own path could not be found, so no ranker could be offered".to_owned()
        })?;
    sealed_rankers_by(&this, args, RANKER_SEAL_LIMIT.min(deadline.left()))
}

/// [`sealed_rankers`] with the program and the limit given, for a test to stand in for both.
#[expect(
    clippy::disallowed_methods,
    reason = "the seal door: the stop hook starts its own helper, under a deadline and output caps"
)]
fn sealed_rankers_by(
    program: &Path,
    args: &[String],
    limit: Duration,
) -> Result<Candidates, String> {
    let mut command = std::process::Command::new(program);
    command.args(["agent", "hook", SEAL_WORD]);
    if let Some(dir) = crate::flag_value(args, "--ranker-dir") {
        command.arg("--ranker-dir").arg(dir);
    }
    let limits = Limits {
        time: limit,
        stdout_bytes: 256 * 1024,
        stderr_bytes: 4 * 1024,
    };
    let done = run_bounded(&mut command, b"", limits).map_err(|stopped| {
        format!("the rankers could not be listed and hashed in the time a hook has ({stopped})")
    })?;
    let list: Value = serde_json::from_slice(&done.stdout).unwrap_or(Value::Null);
    let entries = list
        .as_array()
        .ok_or_else(|| "the rankers could not be listed and hashed".to_owned())?;
    Ok(entries
        .iter()
        .filter_map(|entry| {
            let path = PathBuf::from(entry["path"].as_str()?);
            let digest = match entry["sha256"].as_str() {
                Some(digest) => Ok(digest.to_owned()),
                None => Err(entry["refused"]
                    .as_str()
                    .unwrap_or("could not be hashed")
                    .to_owned()),
            };
            Some((path, digest))
        })
        .collect())
}

/// The clock a seal deadline is read from.
#[expect(
    clippy::disallowed_methods,
    reason = "the seal clock door: hashing a ranker has a deadline, as the judge and Git have"
)]
fn now() -> std::time::Instant {
    std::time::Instant::now()
}

/// The largest ranker that is hashed. A release build of this binary is a few megabytes and a
/// debug build under a hundred; forge-warden itself refuses an executable over 512 MiB.
const RANKER_BYTES_CAP: u64 = 256 * 1024 * 1024;
/// How long listing and hashing rankers may take in one hook, all candidates together: the
/// helper that does it is killed when this has passed. It counts against the hook's one
/// deadline like every other step.
const RANKER_SEAL_LIMIT: Duration = Duration::from_secs(8);

/// The SHA-256 of the regular file at `path`, as hex, or why there is none: the file is not a
/// regular file, is over [`RANKER_BYTES_CAP`], cannot be read, or `deadline` passed.
///
/// It is read in pieces and its size is checked first. Reading a candidate whole, with no
/// bound, made a 256 MiB file in the ranker directory cost 270 MiB and half the client's
/// timeout before the bounded judge was even started (review V4 round 4, VD4R4-M-2).
fn file_sha256(path: &Path, deadline: std::time::Instant) -> Result<String, &'static str> {
    use std::os::unix::fs::OpenOptionsExt as _;
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| "could not be opened")?;
    let found = file.metadata().map_err(|_| "could not be read")?;
    if !found.is_file() {
        return Err("is not a regular file");
    }
    if found.len() > RANKER_BYTES_CAP {
        return Err("is over 256 MiB");
    }
    let mut hasher = deep_diff_forge_core::Sha256::new();
    let mut piece = vec![0u8; 64 * 1024];
    let mut left = RANKER_BYTES_CAP;
    loop {
        if now() >= deadline {
            return Err("could not be hashed in the time a hook has");
        }
        let read = file.read(&mut piece).map_err(|_| "could not be read")?;
        if read == 0 {
            return Ok(hasher.finish_hex());
        }
        // A file that grew past the cap while it was read.
        left = left
            .checked_sub(u64::try_from(read).unwrap_or(u64::MAX))
            .ok_or("is over 256 MiB")?;
        hasher.update(&piece[..read]);
    }
}

/// When the agent wants to stop: have the session's change judged, and block on a fail that
/// has not blocked a stop of this session before.
fn stop(input: &Input, args: &[String]) -> Result<Option<String>, String> {
    let deadline = Deadline::after(STOP_LIMIT);
    let mut session = Session::open(&input.session)?;
    let work = match locate(input, &mut session.state, deadline) {
        Located::Nowhere => return Ok(None),
        Located::Trouble(reason) => return Ok(Some(hook::unjudged_output(&reason))),
        Located::Here(work) => work,
    };
    if let Some(why) = session.inside(&work.place.top) {
        return Ok(Some(hook::unjudged_output(&why)));
    }
    // The project and the base are recorded before anything else can fail.
    let mut unsaved = session.keep().err();
    let mut beside = Vec::new();
    let decision = match subject(&work, deadline) {
        Subject::Nothing => StopDecision::Allow { message: None },
        Subject::Unjudged(reason) => StopDecision::Allow {
            message: Some(hook::unjudged_message(&reason)),
        },
        Subject::Range { base, words } => {
            let scratch = Scratch::new(&work.place.own)?;
            let (trees, said) = judge_all(&work, &input.cwd, &base, &scratch, args, deadline)?;
            beside = said;
            hook::decide_stop_trees(&trees, &session.state.blocked, &words)
        }
    };
    let state = &mut session.state;
    if let StopDecision::Block { identity, .. } = &decision {
        if state.blocked.len() >= BLOCKED_CAP {
            state.blocked.remove(0);
        }
        state.blocked.push(identity.clone());
    }
    let mut notes = Vec::new();
    // A session whose start no session-start hook recorded says so at its first stop, whether
    // this stop or an advice hook before it did the recording. An advice hook runs after the
    // tool call that commits, so with the three printed hooks it was always the one, and it
    // said nothing (second independent review of the hooks, HR2-M-2).
    if !state.start_said && state.project.as_ref().is_some_and(|p| !p.by_start) {
        state.start_said = true;
        notes.push(hook::FIRST_STOP.to_owned());
    }
    if let Some(from) = &work.moved_from {
        notes.push(format!(
            "The project this session began in is no longer at {}: it was found, and judged, at {}.",
            hook::shown_path(from),
            shown_tree(&work.place.top)
        ));
    }
    for said in beside {
        if once(state, &said) {
            notes.push(said);
        }
    }
    if let Err(why) = session.keep() {
        unsaved = Some(why);
    }
    // A state that cannot be saved must not hide the decision, and is said with it: the same
    // failure then blocks at every stop, which used to happen with no word of why
    // (independent review of the hooks, HR-L-4).
    if let Some(why) = unsaved {
        notes.push(format!(
            "The hook could not keep its state ({}): the same failure can block again, and the commit this session began from may not be remembered.",
            deep_diff_forge_core::display_safe(&why)
        ));
    }
    let note = (!notes.is_empty()).then(|| notes.join("\n"));
    Ok(hook::stop_output_with(&decision, note.as_deref()))
}

/// One link of the chain a judged stop depends on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GateCheck {
    pub(crate) name: &'static str,
    pub(crate) ok: bool,
    pub(crate) detail: String,
}

/// The oldest Git that makes the judge's patch: `--attr-source` came with 2.40.
const OLDEST_GIT: (u32, u32) = (2, 40);

/// The `(major, minor)` of a `git version` line.
fn git_release(line: &str) -> Option<(u32, u32)> {
    let mut numbers = line
        .strip_prefix("git version ")?
        .split('.')
        .map(|part| part.parse::<u32>().ok());
    Some((numbers.next()??, numbers.next()??))
}

/// A patch that adds one comment line at the top of the policy named `name`. A policy that
/// protects its own file holds or fails it; one that does not lets it pass. `None` for a policy
/// with no first line, or a name that cannot stand in a patch header as it is.
fn probe_patch(name: &str, policy: &[u8]) -> Option<String> {
    let plain = !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._/-".contains(&b));
    let first = std::str::from_utf8(policy).ok()?.lines().next()?;
    plain.then(|| {
        format!(
            "diff --git a/{name} b/{name}\n--- a/{name}\n+++ b/{name}\n@@ -1,1 +1,2 @@\n+# deep-diff-forge doctor probe\n {first}\n"
        )
    })
}

/// A `Stop` hook a Claude Code settings document registers: its command, and its timeout as
/// the document gives it.
#[derive(Debug, Clone, PartialEq)]
struct StopHook {
    command: String,
    timeout: Option<Value>,
    /// The matcher of the entry the hook stands in, when it has one that is not empty.
    matcher: Option<String>,
}

/// Every `Stop` hook of type `command` a Claude Code settings document registers.
fn stop_hooks(settings: &Value) -> Vec<StopHook> {
    event_hooks(settings, "Stop")
}

/// Every hook of type `command` a Claude Code settings document registers for `event`.
fn event_hooks(settings: &Value, event: &str) -> Vec<StopHook> {
    let entries = settings["hooks"][event].as_array();
    entries
        .into_iter()
        .flatten()
        .flat_map(|entry| {
            let matcher = entry["matcher"]
                .as_str()
                .filter(|matcher| !matcher.is_empty() && *matcher != "*")
                .map(str::to_owned);
            entry["hooks"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|hook| hook["type"] == "command")
                .filter_map(move |hook| {
                    Some(StopHook {
                        command: hook["command"].as_str()?.to_owned(),
                        timeout: hook.get("timeout").filter(|set| !set.is_null()).cloned(),
                        matcher: matcher.clone(),
                    })
                })
        })
        .collect()
}

/// The settings file that registers this binary's hook for `event` (`word` on its command
/// line), or why none does. The judged range depends on the session-start hook, and a commit
/// is seen as it is made only by the hook after a tool call; the gate used to look for the
/// stop hook alone (second independent review of the hooks, HR2-M-2 and HR2-L-1).
fn companion(
    place: &Place,
    event: &str,
    word: &str,
    judge: Option<&Path>,
) -> Result<PathBuf, String> {
    let this = std::env::current_exe().and_then(std::fs::canonicalize).ok();
    let this = this.as_deref().and_then(Path::to_str);
    let mut close: Option<String> = None;
    for file in settings_files(&place.top) {
        let Some(settings) = settings(&file) else {
            continue;
        };
        for hook in event_hooks(&settings, event) {
            let why = match hook::hook_command(&hook.command, word) {
                None => "is not the command `agent setup --hooks` prints",
                Some(read) if this != Some(read.program.as_str()) => {
                    "does not run this program by its own absolute path"
                }
                Some(read)
                    if read.judge.is_some()
                        && read.judge.as_deref() != judge.and_then(Path::to_str) =>
                {
                    "names another judge than the one checked here"
                }
                Some(_) if hook.matcher.is_some() => "has a matcher, so it does not run every time",
                Some(_) => return Ok(file),
            };
            close.get_or_insert(format!("{}: a {event} hook {why}", file.display()));
        }
    }
    Err(close.unwrap_or_else(|| {
        format!(
            "no Claude Code settings file registers a {event} hook of this program (agent setup --client claude --hooks prints the block)"
        )
    }))
}

/// The link `state_directory`: the directory the state of every session is kept in is there
/// or can be made, is a directory of its own, can be written, and is not inside the project's
/// working tree. Without it every hook says that it did not run, and the gate said that every
/// link was ready (second independent review of the hooks, HR2-L-1).
fn state_directory_check(place: Option<&Place>) -> GateCheck {
    let usable = || -> Result<PathBuf, String> {
        let dir = sessions_dir()?;
        if let Some(why) = place.and_then(|place| inside_tree(&dir, &place.top)) {
            return Err(why);
        }
        if let Some(parent) = dir.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("{} could not be made: {e}", shown(&dir)))?;
        }
        own_directory(&dir)?;
        let probe = dir.join(format!("gate-{}.probe", std::process::id()));
        let written = std::fs::write(&probe, b"");
        let _ = std::fs::remove_file(&probe);
        written.map_err(|e| format!("{} cannot be written: {e}", shown(&dir)))?;
        Ok(dir)
    };
    let found = usable();
    check(
        "state_directory",
        found.is_ok(),
        match found {
            Ok(dir) => format!("{}", dir.display()),
            Err(why) => why,
        },
    )
}

/// Why the stop hook `command` is not the chain this gate checked; `None` when it is: the
/// command `agent setup --hooks` prints, for this binary, with the judge, the policy and the
/// rankers that were checked. Text that merely holds the words `agent hook stop` used to count
/// (review V4 round 2, VD4R2-H-1).
fn not_this_chain(
    command: &str,
    judge: Option<&Path>,
    policy: &str,
    args: &[String],
) -> Option<String> {
    let Some(read) = hook::stop_command(command) else {
        return Some(
            "is not the command `agent setup --hooks` prints (one program, `agent hook stop --client claude`, then only --judge, --policy and --ranker-dir)"
                .to_owned(),
        );
    };
    // The word itself, not what it leads to: a shell runs `A=ddf agent …` as the program
    // `agent` with a variable set, and a link that leads to this binary today can lead
    // elsewhere tomorrow. Only this binary's own absolute path, as `agent setup --hooks`
    // prints it, is this program (review V4 round 3, VD4R3-H-1).
    let this = std::env::current_exe().and_then(std::fs::canonicalize).ok();
    if this.as_deref().and_then(Path::to_str) != Some(read.program.as_str()) {
        return Some(format!(
            "does not run this program by its own absolute path ({})",
            hook::shown_path(&read.program)
        ));
    }
    let Some(judge) = judge else {
        return Some("cannot be checked: no judge was found to check it against".to_owned());
    };
    // Likewise the judge: named, by the absolute path that was checked. A command that names
    // none leaves the choice to whatever `PATH` holds when the hook runs.
    if read.judge.as_deref() != judge.to_str() {
        return Some(
            "does not name, by its own absolute path, the judge checked here (--judge)".to_owned(),
        );
    }
    if read.policy.as_deref().unwrap_or(DEFAULT_POLICY) != policy {
        return Some("judges by another policy file than the one checked here".to_owned());
    }
    if read.ranker_dir != crate::flag_value(args, "--ranker-dir") {
        return Some("offers other rankers than the ones checked here (--ranker-dir)".to_owned());
    }
    None
}

/// A Claude Code settings file, read as JSON; `None` when it is missing or cannot be read.
fn settings(file: &Path) -> Option<Value> {
    std::fs::read_to_string(file)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
}

/// The settings file that registers the stop hook this gate checked, with that hook's timeout,
/// or why none does: the first hook that is close but not it, else that there is none.
fn registered(
    place: &Place,
    judge: Option<&Path>,
    policy: &str,
    args: &[String],
) -> Result<(PathBuf, Option<Value>), String> {
    let mut close: Option<String> = None;
    for file in settings_files(&place.top) {
        let Some(settings) = settings(&file) else {
            continue;
        };
        for hook in stop_hooks(&settings) {
            match not_this_chain(&hook.command, judge, policy, args) {
                None => return Ok((file, hook.timeout)),
                Some(why) => {
                    close.get_or_insert(format!("{}: a Stop hook {why}", file.display()));
                }
            }
        }
    }
    Err(close.unwrap_or_else(|| {
        "no Claude Code settings file registers a stop hook (agent setup --client claude --hooks prints the block)"
            .to_owned()
    }))
}

/// The settings file that switches every hook off, if one does: the first of the files Claude
/// Code reads, in its order of precedence, that sets `disableAllHooks` decides, and it says
/// `true`. With that set no hook runs, however well it is registered.
fn hooks_disabled_by(top: &Path) -> Option<PathBuf> {
    for file in settings_files(top) {
        match settings(&file).and_then(|settings| settings["disableAllHooks"].as_bool()) {
            Some(true) => return Some(file),
            Some(false) => return None,
            None => {}
        }
    }
    None
}

/// The settings files Claude Code reads for a project at `top`, the one that wins first: the
/// project's local file, the project's shared file, then the user's. Settings an
/// administrator manages are not read here.
#[expect(
    clippy::disallowed_methods,
    reason = "the gate door: the user's Claude Code settings are found through HOME"
)]
fn settings_files(top: &Path) -> Vec<PathBuf> {
    let mut files = vec![
        top.join(".claude/settings.local.json"),
        top.join(".claude/settings.json"),
    ];
    if let Some(home) = std::env::var_os("HOME") {
        files.push(PathBuf::from(home).join(".claude/settings.json"));
    }
    files
}

/// `agent doctor --gate`: every link a judged stop depends on, checked in order in the
/// repository at `cwd`, each with what was found. A link that cannot be checked because an
/// earlier one is missing is reported as not ready, never as fine.
pub(crate) fn gate(cwd: &Path, args: &[String]) -> Vec<GateCheck> {
    let deadline = Deadline::after(GATE_LIMIT);
    let place = match find(cwd, deadline) {
        Found::Tree(place) => Ok(place),
        Found::NotARepository | Found::Gone => {
            Err("this directory is not in a Git repository".to_owned())
        }
        Found::Failed(reason) => Err(reason),
    };
    gate_in(
        cwd,
        place.as_ref().map_err(String::as_str),
        None,
        args,
        deadline,
    )
    .checks
}

/// What [`gate_in`] found.
struct Gate {
    checks: Vec<GateCheck>,
    /// The working tree differs from the base it was compared with.
    dirty: bool,
}

fn check(name: &'static str, ok: bool, detail: String) -> GateCheck {
    GateCheck { name, ok, detail }
}

fn not_checked(why: &str) -> String {
    format!("{NOT_CHECKED}{why}")
}

/// The links `chain` and `policy_protects_itself`: the judge's verdict on the probe, or why it
/// gave none.
fn chain_checks(name: &str, verdict: &Result<(PathBuf, i32), String>) -> [GateCheck; 2] {
    [
        check(
            "chain",
            verdict.is_ok(),
            match verdict {
                Ok((ranker, code)) => format!(
                    "the judge gave a verdict (exit {code}) with ranker {}",
                    ranker.display()
                ),
                Err(why) => why.clone(),
            },
        ),
        check(
            hook::PROTECTS_ITSELF,
            verdict.as_ref().is_ok_and(|(_, code)| *code != 0),
            match verdict {
                Ok((_, 0)) => format!(
                    "a change to {} passes: any change could loosen the policy it is judged by",
                    hook::shown_path(name)
                ),
                Ok(_) => format!("a change to {} is held or fails", hook::shown_path(name)),
                Err(_) => not_checked("the judge gave no verdict"),
            },
        ),
    ]
}

/// The link `patch`: the patch a stop would judge, made the way a stop makes it against the
/// commit `rev` (called `at` in what is said). With it, whether the tree differs from that
/// commit.
fn patch_check(
    place: Option<&Place>,
    scratch: Option<&Result<Scratch, String>>,
    rev: &str,
    at: &str,
    deadline: Deadline,
) -> (GateCheck, bool) {
    let unjudged = |why: String| {
        format!("a stop here would be UNJUDGED: the change could not be read from Git: {why}")
    };
    let patch = match (place, scratch) {
        (Some(place), Some(Ok(scratch))) => scratch
            .sub("work")
            .and_then(|work| {
                judge_patch(&place.top, rev, &work, &deadline.git()).map_err(|error| match error {
                    JudgePatchError::NoBase => format!("{at} names no commit"),
                    JudgePatchError::Failed(error) => unjudged(git_words(&error)),
                    error @ JudgePatchError::SkipWorktree => unjudged(error.to_string()),
                })
            })
            .map(|patch| patch.bytes.len()),
        (Some(_), Some(Err(why))) => Err(why.clone()),
        _ => Err(not_checked("it needs the repository")),
    };
    let detail = match &patch {
        Ok(0) => format!("the working tree equals {at}: a stop now has nothing to judge"),
        Ok(bytes) => {
            format!("the patch a stop would judge can be made ({bytes} bytes against {at})")
        }
        Err(why) => why.clone(),
    };
    (
        check("patch", patch.is_ok(), detail),
        patch.is_ok_and(|bytes| bytes > 0),
    )
}

/// The links `hooks_enabled`, `hooks_registered` and `hook_timeout`: what the Claude Code
/// settings of the project at `place` and of the user say about the stop hook that was checked.
fn settings_checks(
    place: Option<&Place>,
    judge: Option<&Path>,
    policy: &str,
    args: &[String],
) -> [GateCheck; 5] {
    let disabled = place.map(|place| hooks_disabled_by(&place.top));
    let enabled = check(
        "hooks_enabled",
        matches!(disabled, Some(None)),
        match &disabled {
            Some(Some(file)) => format!(
                "{} sets disableAllHooks: no hook runs, whatever is registered",
                file.display()
            ),
            Some(None) => "no settings file read here switches hooks off".to_owned(),
            None => not_checked("it needs the repository"),
        },
    );
    let registered = match place {
        Some(place) => registered(place, judge, policy, args),
        None => Err(not_checked("it needs the repository")),
    };
    let timeout = registered.as_ref().map(|(_, timeout)| timeout.as_ref());
    let long_enough = match timeout {
        Ok(None) => true,
        Ok(Some(timeout)) => timeout
            .as_f64()
            .is_some_and(|seconds| seconds >= f64::from(STOP_TIMEOUT_NEEDED)),
        Err(_) => false,
    };
    let timed = check(
        "hook_timeout",
        long_enough,
        match timeout {
            Ok(None) => "the stop hook sets no timeout: the client's own applies".to_owned(),
            Ok(Some(timeout)) if long_enough => format!(
                "the stop hook's timeout is {timeout} seconds ({STOP_TIMEOUT_NEEDED} or more are needed)"
            ),
            Ok(Some(timeout)) => format!(
                "the stop hook's timeout is {timeout}, under the {STOP_TIMEOUT_NEEDED} seconds a stop can take: the client would end the hook and drop its answer"
            ),
            Err(_) => not_checked("no stop hook is registered"),
        },
    );
    let registered = check(
        "hooks_registered",
        registered.is_ok(),
        match &registered {
            Ok((file, _)) => format!("{}", file.display()),
            Err(why) => why.clone(),
        },
    );
    let beside = |name: &'static str, event: &str, word: &str, without: &str| {
        let found = match place {
            Some(place) => companion(place, event, word, judge),
            None => Err(not_checked("it needs the repository")),
        };
        check(
            name,
            found.is_ok(),
            match found {
                Ok(file) => format!("{}", file.display()),
                Err(why) if why.starts_with(NOT_CHECKED) => why,
                Err(why) => format!("{why}; {without}"),
            },
        )
    };
    let start = beside(
        "start_hook_registered",
        "SessionStart",
        "session-start",
        "without it a session's changes are measured from its first tool call or stop, not from its start",
    );
    let advice = beside(
        "advice_hook_registered",
        "PostToolUse",
        "post-tool",
        "without it nothing is said while the agent works",
    );
    [enabled, registered, timed, start, advice]
}

/// The links of the chain, checked for the working tree at `place` (or reported as missing
/// with the reason it was not found), with the commit `base` as what a stop compares with:
/// `None` for `HEAD`, which is what a session that starts now would record.
///
/// The judge is asked for a real verdict on a probe: a patch that adds one comment line to the
/// policy. That proves the whole chain at once (the judge runs, it admits a ranker this machine
/// has, the ranker ranks), and its answer says whether the policy protects its own file: a
/// policy that lets the probe pass can be loosened by any change.
///
/// The patch a stop would judge is made as a stop makes it, and the settings are read whole.
/// Before, the gate said ready where a stop was UNJUDGED (a path marked skip-worktree, an
/// untracked file that cannot be read) and where no hook ran at all (`disableAllHooks`, a
/// timeout of one second): independent review of the hooks, HR-M-4.
fn gate_in(
    dir: &Path,
    place: Result<&Place, &str>,
    base: Option<&str>,
    args: &[String],
    deadline: Deadline,
) -> Gate {
    let (rev, at) = match base {
        Some(id) => (
            id,
            format!(
                "the commit this session began from ({})",
                id.chars().take(7).collect::<String>()
            ),
        ),
        None => ("HEAD", "HEAD".to_owned()),
    };
    let version = deep_diff_forge_patch::git_version(dir, &deadline.git());
    let git_ok = version
        .as_ref()
        .is_ok_and(|line| git_release(line).is_some_and(|release| release >= OLDEST_GIT));
    let mut checks = vec![
        check(
            "git",
            git_ok,
            match &version {
                Ok(line) => format!("{line} (2.40 or later is needed)"),
                Err(error) => format!("Git could not be run: {}", git_words(error)),
            },
        ),
        check(
            "repository",
            place.is_ok(),
            match place {
                Ok(place) => format!("{}", place.top.display()),
                Err(why) => why.to_owned(),
            },
        ),
    ];
    let place = place.ok();
    let name = policy_name(args);
    let policy = place.and_then(|place| {
        file_at_commit(&place.top, rev, &name, &deadline.git())
            .ok()
            .flatten()
    });
    checks.push(check(
        "policy",
        policy.is_some(),
        if policy.is_some() {
            format!("{} at {at}", hook::shown_path(&name))
        } else {
            format!("{at} has no policy file {}", hook::shown_path(&name))
        },
    ));
    let program = match place {
        Some(place) => judge_program(args, &[&place.top, &place.common]),
        None => judge_program(args, &[]),
    };
    checks.push(check(
        "judge",
        program.is_ok(),
        match &program {
            Ok(path) => format!("{}", path.display()),
            Err(why) => why.clone(),
        },
    ));
    let program = program.ok();
    let scratch = place.map(|place| Scratch::new(&place.own));
    let verdict = match (&scratch, &policy, &program) {
        (Some(Ok(scratch)), Some(policy), Some(program)) => {
            probe(scratch, &name, policy, program, args, deadline)
        }
        (Some(Err(why)), _, _) => Err(why.clone()),
        _ => Err(not_checked(
            "it needs the repository, the policy and the judge",
        )),
    };
    checks.extend(chain_checks(&name, &verdict));
    let (patch, dirty) = patch_check(place, scratch.as_ref(), rev, &at, deadline);
    checks.push(patch);
    checks.push(state_directory_check(place));
    checks.extend(settings_checks(place, program.as_deref(), &name, args));
    Gate { checks, dirty }
}

/// Have the judge judge the probe. `Ok` with the ranker it admitted and its exit code when it
/// gave a verdict; otherwise why not.
fn probe(
    scratch: &Scratch,
    name: &str,
    policy: &[u8],
    program: &Path,
    args: &[String],
    deadline: Deadline,
) -> Result<(PathBuf, i32), String> {
    let patch = probe_patch(name, policy).ok_or_else(|| {
        "the policy is empty, or its name cannot stand in a patch header".to_owned()
    })?;
    let (policy_file, patch_file) = (
        scratch.0.join("probe-policy.toml"),
        scratch.0.join("probe.patch"),
    );
    std::fs::write(&policy_file, policy)
        .and_then(|()| std::fs::write(&patch_file, &patch))
        .map_err(|e| format!("the judge's input could not be written: {e}"))?;
    let (policy_sha256, patch_sha256) = (
        deep_diff_forge_core::sha256_hex(policy),
        deep_diff_forge_core::sha256_hex(patch.as_bytes()),
    );
    let mut last =
        "this binary's own path could not be found, so no ranker could be offered".to_owned();
    for (ranker, digest) in sealed_rankers(args, deadline)? {
        let ranker_sha256 = match digest {
            Ok(digest) => digest,
            Err(why) => {
                last = format!(
                    "the ranker {} {why}, so no verdict could be sealed to it",
                    shown(&ranker)
                );
                continue;
            }
        };
        let sealed = hook::Sealed {
            policy: &policy_sha256,
            patch: &patch_sha256,
            ranker: &ranker_sha256,
        };
        let (code, verdict, said) =
            run_judge(program, &policy_file, &patch_file, &ranker, deadline);
        match (
            code,
            hook::judged(code, verdict.as_ref(), said.as_deref(), sealed),
        ) {
            (Some(code), Judged::Pass | Judged::Hold { .. } | Judged::Fail { .. }) => {
                return Ok((ranker, code));
            }
            (_, Judged::Unjudged { reason }) => last = reason,
            (None, _) => {}
        }
        if code != Some(4) {
            break;
        }
    }
    Err(last)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Review V4 round 6, VD4R6-M-1: the first files by name are chosen while the directory is
    /// read, and only a path that would be kept is asked whether it is a file. The answer is
    /// the one sorting everything and cutting gave.
    #[test]
    fn the_first_files_by_name_are_chosen_without_holding_the_directory() {
        // 20,000 names in an order that is not sorted (a multiplier coprime to the count)
        let names: Vec<PathBuf> = (0..20_000u32)
            .map(|i| PathBuf::from(format!("/r/{:05}", (i * 7_919) % 20_000)))
            .collect();
        let is_file = |path: &Path| !path.to_string_lossy().ends_with('3');
        let asked = std::cell::Cell::new(0usize);
        let chosen = first_by_name(names.iter().cloned(), 7, |path| {
            asked.set(asked.get() + 1);
            is_file(path)
        });
        let mut all: Vec<PathBuf> = names.iter().filter(|p| is_file(p)).cloned().collect();
        all.sort();
        all.truncate(7);
        assert_eq!(chosen, all);
        assert_eq!(chosen[0], PathBuf::from("/r/00000"));
        assert!(!chosen.contains(&PathBuf::from("/r/00003")));
        // far fewer questions than names: a later name is passed over unasked
        assert!(asked.get() < 500, "{}", asked.get());
        // fewer names than places, and no place at all
        let few = first_by_name(names.iter().take(3).cloned(), 7, |_| true);
        assert_eq!(few.len(), 3);
        assert!(few.is_sorted());
        assert_eq!(
            first_by_name(names.iter().cloned(), 0, |_| true),
            Vec::<PathBuf>::new()
        );
    }

    const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

    fn project(base: Base) -> Project {
        Project {
            top: "/r".to_owned(),
            id: "59:11082023".to_owned(),
            base,
            tips: vec![COMMIT.to_owned(), "a".repeat(40)],
            tips_partial: false,
            by_start: true,
        }
    }

    /// Second independent review of the hooks, HR2-M-1. A state the hooks wrote is read back
    /// whole. Anything else that stands at a state's name is an error with its reason, never an
    /// empty state: read as "no state", it made the next hook the session's first and the
    /// commit `HEAD` named then its base.
    #[test]
    fn a_state_round_trips_and_anything_else_is_not_a_state() {
        let state = State {
            project: Some(project(Base::Commit(COMMIT.to_owned()))),
            start_said: true,
            reported: ["b".to_owned(), "a".to_owned()].into(),
            noticed: ["n".to_owned()].into(),
            blocked: vec![format!("{}:3", "f".repeat(64))],
        };
        assert_eq!(state_from(&state_text(&state)), Ok(state));
        for base in [Base::NoCommit, Base::Orphan, Base::Commit("a".repeat(64))] {
            let other = State {
                project: Some(Project {
                    tips_partial: true,
                    by_start: false,
                    ..project(base)
                }),
                ..State::default()
            };
            assert_eq!(state_from(&state_text(&other)), Ok(other));
        }
        let none = State::default();
        assert_eq!(state_from(&state_text(&none)), Ok(none));
        // A project that is not whole is no state: a base is not believed in part, only a
        // full commit id is one (a name like `HEAD` would move with the session), and the
        // repository's identity is two numbers.
        let whole = json!({"top": "/r", "id": "1:2", "base": "none", "tips": [],
                           "tips_partial": false, "by_start": true});
        let with = |project: &Value| {
            json!({"schema": STATE_SCHEMA, "project": project, "start_said": false,
                   "reported": ["a"], "noticed": [], "blocked": []})
            .to_string()
        };
        assert!(state_from(&with(&whole)).is_ok());
        for (member, value) in [
            ("top", json!("")),
            ("top", Value::Null),
            ("id", json!("1")),
            ("id", json!("1:x")),
            ("id", json!(":2")),
            ("id", Value::Null),
            ("base", json!("HEAD")),
            ("base", json!("0123456")),
            ("base", json!("0123456789ABCDEF0123456789abcdef01234567")),
            ("base", json!(7)),
            ("tips", json!(["HEAD"])),
            ("tips", json!("x")),
            ("tips", Value::Null),
            ("tips_partial", Value::Null),
            ("by_start", json!("yes")),
        ] {
            let mut spoiled = whole.clone();
            spoiled[member] = value.clone();
            assert_eq!(
                state_from(&with(&spoiled)),
                Err("its project is not whole"),
                "{member}: {value}"
            );
        }
        assert!(state_from(&with(&json!("/r"))).is_err());
        // not JSON, another schema (the one before this version too), a missing member, a
        // member of the wrong type: each is refused, none is read in part
        for foreign in [
            "",
            "not json",
            "[]",
            r#"{"schema":"other","reported":["a"]}"#,
            r#"{"schema":"deep-diff-forge.hook-state.v1","project":null,"reported":["a"],"noticed":[],"last_block":null}"#,
            r#"{"reported":["a"]}"#,
            r#"{"schema":"deep-diff-forge.hook-state.v2","project":null,"reported":[],"noticed":[],"blocked":[]}"#,
            r#"{"schema":"deep-diff-forge.hook-state.v2","project":null,"start_said":false,"reported":["a",1],"noticed":[],"blocked":[]}"#,
            r#"{"schema":"deep-diff-forge.hook-state.v2","project":null,"start_said":false,"reported":[],"noticed":[],"blocked":"x"}"#,
            r#"{"schema":"deep-diff-forge.hook-state.v2","project":null,"start_said":false,"reported":[],"noticed":[]}"#,
        ] {
            assert!(state_from(foreign).is_err(), "{foreign}");
        }
    }

    /// HR2-M-1: every list of a state is bounded when it is written, so the largest state the
    /// hooks can write is far under what they read back, and it is read back whole. Before, a
    /// block with 4,000 violations wrote 1.1 MB and the reader stopped at 1 MiB.
    #[test]
    fn the_largest_state_the_hooks_write_is_read_back() {
        let hex = |n: usize, width: usize| format!("{n:0width$x}");
        let full = State {
            project: Some(Project {
                top: "/".to_owned() + &"p".repeat(4_000),
                id: format!("{}:{}", u64::MAX, u64::MAX),
                base: Base::Commit(hex(0, 64)),
                tips: (0..=REFS_CAP).map(|n| hex(n, 64)).collect(),
                tips_partial: true,
                by_start: false,
            }),
            start_said: true,
            reported: (0..REPORTED_CAP).map(|n| hex(n, 64)).collect(),
            noticed: (0..NOTICED_CAP).map(|n| hex(n, 64)).collect(),
            blocked: (0..BLOCKED_CAP)
                .map(|n| format!("{}:{}", hex(n, 64), usize::MAX))
                .collect(),
        };
        let text = state_text(&full);
        assert!(
            (text.len() as u64) < STATE_CAP / 4,
            "{} bytes of {STATE_CAP}",
            text.len()
        );
        assert_eq!(state_from(&text), Ok(full.clone()));
        // one entry more in any list is not a state the hooks wrote
        for grow in 0..4 {
            let mut over = full.clone();
            match grow {
                0 => {
                    over.reported.insert("x".to_owned());
                }
                1 => {
                    over.noticed.insert("x".to_owned());
                }
                2 => over.blocked.push("x".to_owned()),
                _ => {
                    if let Some(project) = &mut over.project {
                        project.tips.push(hex(1, 40));
                    }
                }
            }
            assert!(state_from(&state_text(&over)).is_err(), "{grow}");
        }
        // through a file, as a hook does it: saved, and read back the same
        let scratch = deep_diff_forge_testkit::Scratch::new("hook-state-largest");
        let file = scratch.join("session.json");
        assert_eq!(load(&file), Ok(None));
        save(&file, &full).unwrap();
        assert_eq!(load(&file), Ok(Some(full)));
        // what stands there and is not a state is said with its reason, each one
        std::fs::write(&file, "x".repeat(usize::try_from(STATE_CAP).unwrap() + 1)).unwrap();
        assert_eq!(
            load(&file),
            Err("it is larger than any state the hooks write".to_owned())
        );
        std::fs::write(&file, "{}").unwrap();
        assert_eq!(
            load(&file),
            Err("it is not a state of this version of the hooks".to_owned())
        );
        std::fs::remove_file(&file).unwrap();
        std::fs::create_dir(&file).unwrap();
        assert_eq!(load(&file), Err("it is not a regular file".to_owned()));
        std::fs::remove_dir(&file).unwrap();
        std::os::unix::fs::symlink(scratch.join("elsewhere"), &file).unwrap();
        assert!(
            load(&file).is_err_and(|why| why.starts_with("it cannot be opened (")),
            "a link was followed or read as no state"
        );
    }

    /// Review V4 round 10, VD4R10-H-1: a repository is known by the device and inode of its
    /// Git common directory, which stay when it is moved and differ for another directory.
    #[test]
    fn a_repository_is_known_by_its_directory_not_by_its_path() {
        let scratch = deep_diff_forge_testkit::Scratch::new("hook-identity");
        let (one, other) = (scratch.join("one"), scratch.join("other"));
        std::fs::create_dir(&one).unwrap();
        std::fs::create_dir(&other).unwrap();
        let first = identity(&one).unwrap();
        assert!(is_identity(&first), "{first}");
        assert_ne!(Some(&first), identity(&other).as_ref());
        std::fs::rename(&one, scratch.join("moved")).unwrap();
        assert_eq!(identity(&scratch.join("moved")), Some(first.clone()));
        std::fs::create_dir(&one).unwrap();
        assert_ne!(identity(&one), Some(first));
        assert_eq!(identity(&scratch.join("missing")), None);
        // a path is made canonical as far as it exists
        assert_eq!(
            resolved(&scratch.join("moved/not/there")),
            std::fs::canonicalize(scratch.join("moved"))
                .unwrap()
                .join("not/there")
        );
        std::os::unix::fs::symlink(scratch.join("moved"), scratch.join("link")).unwrap();
        assert_eq!(
            resolved(&scratch.join("link/state")),
            std::fs::canonicalize(scratch.join("moved"))
                .unwrap()
                .join("state")
        );
        assert!(
            inside_tree(
                &scratch.join("link/state"),
                &resolved(&scratch.join("moved"))
            )
            .is_some()
        );
        assert!(
            inside_tree(
                &scratch.join("other/state"),
                &resolved(&scratch.join("moved"))
            )
            .is_none()
        );
    }

    /// HR2-H-1: things with the same patch are judged once, in the order they were found, and
    /// no more than [`MAX_PATCHES`] patches are left to judge; what is beyond is not judged
    /// and says so.
    #[test]
    fn the_same_patch_is_judged_once_and_no_more_than_four_patches() {
        let shown = |what: &str, patch: Result<Option<&str>, &str>| Shown {
            what: vec![what.to_owned()],
            patch: patch
                .map(|bytes| bytes.map(|bytes| bytes.as_bytes().to_vec()))
                .map_err(str::to_owned),
        };
        // the usual stop: the working tree is what HEAD holds, and a branch and a tag name it
        let usual = distinct(vec![
            shown("tree", Ok(Some("P"))),
            shown("HEAD", Ok(Some("P"))),
            shown("tag", Ok(Some("P"))),
        ]);
        assert_eq!(usual.len(), 1);
        assert_eq!(usual[0].what, ["tree", "HEAD", "tag"]);
        assert_eq!(usual[0].patch, Ok(Some(b"P".to_vec())));
        // no change and failures are each their own, and are not counted as patches
        let mixed = distinct(vec![
            shown("tree", Ok(None)),
            shown("linked", Ok(None)),
            shown("a", Ok(Some("A"))),
            shown("x", Err("why")),
            shown("b", Ok(Some("B"))),
            shown("a2", Ok(Some("A"))),
            shown("c", Ok(Some("C"))),
            shown("d", Ok(Some("D"))),
            shown("e", Ok(Some("E"))),
            shown("e2", Ok(Some("E"))),
        ]);
        let what: Vec<String> = mixed.iter().map(|one| one.what.join("+")).collect();
        assert_eq!(what, ["tree", "linked", "a+a2", "x", "b", "c", "d", "e+e2"]);
        let judged = mixed
            .iter()
            .filter(|one| matches!(one.patch, Ok(Some(_))))
            .count();
        assert_eq!(judged, MAX_PATCHES);
        assert!(
            mixed[7]
                .patch
                .as_ref()
                .is_err_and(|why| why.contains("at most 4 distinct patches")),
            "{:?}",
            mixed[7].patch
        );
        assert_eq!(mixed[3].patch, Err("why".to_owned()));
    }

    #[test]
    fn the_judges_own_words_are_read_from_its_last_error_line() {
        let json = br#"{"exit":3,"name":"io","message":"Git admission: no Git allowlist enrolled","next":"check_environment"}"#;
        assert_eq!(
            judge_words(json).as_deref(),
            Some("io: Git admission: no Git allowlist enrolled")
        );
        assert_eq!(
            judge_words(b"warning: x\n{\"message\":\"only this\"}\n\n").as_deref(),
            Some("only this")
        );
        assert_eq!(
            judge_words(b"plain words\n").as_deref(),
            Some("plain words")
        );
        assert_eq!(
            judge_words(b"{\"other\":1}").as_deref(),
            Some("{\"other\":1}")
        );
        assert_eq!(judge_words(b""), None);
        assert_eq!(judge_words(b"\n  \n"), None);
        let long = format!("{{\"message\":\"{}\"}}", "m".repeat(1000));
        assert_eq!(
            judge_words(long.as_bytes()).map(|w| w.chars().count()),
            Some(JUDGE_WORDS)
        );
    }

    #[test]
    fn the_gate_reads_a_git_version_a_settings_file_and_makes_a_probe() {
        assert_eq!(git_release("git version 2.56.0"), Some((2, 56)));
        assert_eq!(git_release("git version 2.40.1.windows.1"), Some((2, 40)));
        assert!(git_release("git version 2.39.5").is_some_and(|r| r < OLDEST_GIT));
        assert_eq!(git_release("git version two"), None);
        assert_eq!(git_release("version 2.56.0"), None);

        let registered = json!({"hooks": {"Stop": [
            {"hooks": [{"type": "command", "command": "one"}, {"type": "prompt"}]},
            {"matcher": "x", "hooks": [{"type": "command", "command": "two"}]},
            {"hooks": "three"},
        ], "PostToolUse": [{"hooks": [{"command": "four"}]}]}});
        let commands = |settings: &Value| -> Vec<String> {
            stop_hooks(settings)
                .into_iter()
                .map(|hook| hook.command)
                .collect()
        };
        assert_eq!(commands(&registered), ["one", "two"]);
        for other in [
            json!({}),
            json!({"hooks": {"Stop": []}}),
            json!({"hooks": {"Stop": "agent hook stop"}}),
            // HR-M-4: an entry is read whole; only a hook of type `command` runs a command
            json!({"hooks": {"Stop": [{"hooks": [{"type": "prompt", "command": "x"}]}]}}),
            json!({"hooks": {"Stop": [{"hooks": [{"command": "x"}]}]}}),
        ] {
            assert_eq!(commands(&other), Vec::<String>::new(), "{other}");
        }
        let timed = json!({"hooks": {"Stop": [{"hooks": [
            {"type": "command", "command": "c", "timeout": 1}]}]}});
        assert_eq!(
            stop_hooks(&timed),
            [StopHook {
                command: "c".to_owned(),
                timeout: Some(json!(1)),
                matcher: None,
            }]
        );
        // HR2-L-1: the hooks of the other events are read the same way, each with the matcher
        // of the entry it stands in when that narrows it
        let others = json!({"hooks": {
            "SessionStart": [{"matcher": "startup", "hooks": [{"type": "command", "command": "s"}]}],
            "PostToolUse": [{"matcher": "*", "hooks": [{"type": "command", "command": "p"}]},
                            {"matcher": "", "hooks": [{"type": "command", "command": "q"}]}],
        }});
        assert_eq!(
            event_hooks(&others, "SessionStart"),
            [StopHook {
                command: "s".to_owned(),
                timeout: None,
                matcher: Some("startup".to_owned()),
            }]
        );
        let advice = event_hooks(&others, "PostToolUse");
        assert_eq!(advice.len(), 2);
        assert!(advice.iter().all(|hook| hook.matcher.is_none()));
        assert_eq!(event_hooks(&others, "Stop"), []);

        assert_eq!(
            probe_patch("warden.toml", b"schema = 1\nrest\n").as_deref(),
            Some(
                "diff --git a/warden.toml b/warden.toml\n--- a/warden.toml\n+++ b/warden.toml\n@@ -1,1 +1,2 @@\n+# deep-diff-forge doctor probe\n schema = 1\n"
            )
        );
        assert!(probe_patch("policy/warden.toml", b"x").is_some());
        for (name, policy) in [
            ("warden.toml", &b""[..]),
            ("warden.toml", &[0xff, b'\n'][..]),
            ("a b.toml", &b"x\n"[..]),
            ("a\nb", &b"x\n"[..]),
            ("", &b"x\n"[..]),
        ] {
            assert_eq!(probe_patch(name, policy), None, "{name:?}");
        }
    }

    /// Review V4 round 5: the deadline is the parent's, and it ends the helper. A helper that
    /// does not come back (a stalled read, here a sleep) is killed when the limit has passed,
    /// and the stop says so; one that prints a list is read; one that prints anything else
    /// gives no list.
    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "test: the stand-in helpers are written by a child process, as executables must be"
    )]
    fn the_helper_that_hashes_rankers_is_ended_at_the_deadline() {
        let scratch = deep_diff_forge_testkit::Scratch::new("seal-helper");
        // written by a child process: a file this process had open can be busy at exec
        let program = |name: &str, body: &str| {
            let path = scratch.join(name);
            let status = std::process::Command::new("sh")
                .args([
                    "-c",
                    "printf '%s' \"$2\" > \"$1\" && chmod 755 \"$1\"",
                    "sh",
                ])
                .arg(&path)
                .arg(body)
                .status()
                .unwrap();
            assert!(status.success());
            path
        };
        let stalled = program("stalled", "#!/bin/sh\nsleep 30\n");
        let started = now();
        let ended = sealed_rankers_by(&stalled, &[], Duration::from_millis(300));
        assert!(
            ended
                .as_ref()
                .is_err_and(|why| why.contains("in the time a hook has")),
            "{ended:?}"
        );
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the helper was waited for"
        );

        let lists = program(
            "lists",
            "#!/bin/sh\necho '[{\"path\":\"/a\",\"sha256\":\"aa\"},{\"path\":\"/b\",\"refused\":\"is over 256 MiB\"},{\"path\":null,\"sha256\":\"cc\"}]'\n",
        );
        assert_eq!(
            sealed_rankers_by(&lists, &[], Duration::from_secs(20)),
            Ok(vec![
                (PathBuf::from("/a"), Ok("aa".to_owned())),
                (PathBuf::from("/b"), Err("is over 256 MiB".to_owned())),
            ])
        );
        let other = program("other", "#!/bin/sh\necho not-a-list\n");
        assert_eq!(
            sealed_rankers_by(&other, &[], Duration::from_secs(20)),
            Err("the rankers could not be listed and hashed".to_owned())
        );
    }

    /// Review V4 round 4, VD4R4-M-2: a ranker is hashed in pieces, and one that is too large,
    /// not a regular file, or too slow to hash is refused with its reason instead of being
    /// read whole.
    #[test]
    fn a_ranker_is_hashed_under_a_size_and_a_time_bound() {
        let scratch = deep_diff_forge_testkit::Scratch::new("ranker-seal");
        let soon = || now() + Duration::from_secs(30);
        let small = scratch.write("small", b"abc");
        assert_eq!(
            file_sha256(&small, soon()).as_deref(),
            Ok("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
        );
        // larger than one piece, cut at a piece boundary and not
        let bytes: Vec<u8> = (0..200_000u32)
            .map(|i| u8::try_from(i % 251).unwrap())
            .collect();
        let larger = scratch.write("larger", &bytes);
        assert_eq!(
            file_sha256(&larger, soon()),
            Ok(deep_diff_forge_core::sha256_hex(&bytes))
        );
        // over the cap by its size alone: refused before a byte is read (the file is sparse)
        let huge = scratch.join("huge");
        std::fs::File::create(&huge)
            .unwrap()
            .set_len(RANKER_BYTES_CAP + 1)
            .unwrap();
        assert_eq!(file_sha256(&huge, soon()), Err("is over 256 MiB"));
        // exactly at the cap is allowed to start, and a deadline that has passed stops it
        std::fs::File::create(&huge)
            .unwrap()
            .set_len(RANKER_BYTES_CAP)
            .unwrap();
        assert_eq!(
            file_sha256(&huge, now()),
            Err("could not be hashed in the time a hook has")
        );
        assert_eq!(
            file_sha256(scratch.path(), soon()),
            Err("is not a regular file")
        );
        assert_eq!(
            file_sha256(&scratch.join("missing"), soon()),
            Err("could not be opened")
        );
    }

    #[test]
    fn a_session_id_does_not_choose_the_state_file_name() {
        let own = Path::new("/home/u/.local/state/deep-diff-forge/hooks");
        let plain = state_file(own, "abc");
        let hostile = state_file(own, "../../../etc/passwd");
        for file in [&plain, &hostile] {
            assert_eq!(file.parent(), Some(own));
            let name = file.file_name().unwrap().to_str().unwrap();
            let hash = name
                .strip_prefix("session-")
                .and_then(|rest| rest.strip_suffix(".json"))
                .unwrap_or_else(|| panic!("{name}"));
            assert_eq!(hash.len(), 32, "{name}");
            assert!(hash.bytes().all(|b| b.is_ascii_hexdigit()), "{name}");
        }
        assert_ne!(plain, hostile);
        assert_eq!(plain, state_file(own, "abc"));
    }

    #[test]
    fn findings_carry_the_text_of_their_line_only_in_the_key() {
        let patch = b"diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,2 +1,3 @@\n fn a() {}\n+#[allow(dead_code)]\n fn b() {}\n";
        let found = findings(patch).unwrap();
        assert_eq!(found.len(), 1);
        let f = &found[0];
        assert_eq!(
            (f.label.as_str(), f.path.as_str(), f.side.as_str(), f.line),
            ("check_suppressed", "src/a.rs", "new", 2)
        );
        assert_eq!(
            f.key,
            hook::finding_key("src/a.rs", "check_suppressed", "#[allow(dead_code)]")
        );
        // the same line further down the file is the same finding
        let moved = b"diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,2 +1,4 @@\n fn a() {}\n+\n+#[allow(dead_code)]\n fn b() {}\n";
        let again = findings(moved).unwrap();
        assert_eq!(again[0].line, 3);
        assert_eq!(again[0].key, f.key);
        assert_eq!(findings(b"").unwrap(), Vec::<Finding>::new());
    }

    /// The state of a session is found by its id alone, under the user's state directory; a
    /// relative value would put it under whatever directory the hook runs in.
    #[test]
    fn the_sessions_directory_is_under_an_absolute_state_home() {
        let os = |text: &str| Some(OsString::from(text));
        assert_eq!(
            resolve_sessions_dir(os("/x/state"), os("/home/u")),
            Some(PathBuf::from("/x/state/deep-diff-forge/hooks"))
        );
        assert_eq!(
            resolve_sessions_dir(None, os("/home/u")),
            Some(PathBuf::from("/home/u/.local/state/deep-diff-forge/hooks"))
        );
        for relative in ["", "state", "./state"] {
            assert_eq!(
                resolve_sessions_dir(os(relative), os("/home/u")),
                Some(PathBuf::from("/home/u/.local/state/deep-diff-forge/hooks")),
                "{relative:?}"
            );
            assert_eq!(resolve_sessions_dir(None, os(relative)), None);
        }
        assert_eq!(resolve_sessions_dir(None, None), None);
    }

    /// Independent review of the hooks, HR-L-2: a reason an advice hook has for not running is
    /// told once in a session, and no more than [`NOTICED_CAP`] reasons are told at all.
    #[test]
    fn a_reason_for_not_running_is_told_once_and_the_reasons_are_bounded() {
        let mut state = State::default();
        let first = notice(&mut state, "git add exited with status 128").unwrap();
        let said: Value = serde_json::from_str(&first).unwrap();
        let message = said["systemMessage"].as_str().unwrap();
        assert!(
            message.contains("the hook did not run: git add exited with status 128.")
                && message.contains("Said once in this session"),
            "{message}"
        );
        assert_eq!(notice(&mut state, "git add exited with status 128"), None);
        assert!(notice(&mut state, "another reason").is_some());
        assert_eq!(notice(&mut state, "another reason"), None);
        for index in state.noticed.len()..NOTICED_CAP - 1 {
            assert!(notice(&mut state, &format!("reason {index}")).is_some());
        }
        // the last one that is told says that it is the last
        let last = notice(&mut state, "the last reason").unwrap();
        assert!(last.contains("No further notice"), "{last}");
        assert_eq!(state.noticed.len(), NOTICED_CAP);
        assert_eq!(notice(&mut state, "one more"), None);
        assert_eq!(state.noticed.len(), NOTICED_CAP);
        // the reasons are kept as digests, not as text
        assert!(state.noticed.iter().all(|key| key.len() == 64));
    }

    /// Independent review of the hooks, HR-L-1: a step gets the time its hook has left, not a
    /// limit of its own on top of the others'.
    #[test]
    fn a_step_gets_the_time_that_is_left() {
        let spent = Deadline::after(Duration::ZERO);
        assert_eq!(spent.left(), Duration::ZERO);
        assert_eq!(spent.git().time_limit, Duration::ZERO);
        let ample = Deadline::after(Duration::from_secs(3_600));
        let left = ample.left();
        assert!(
            left > Duration::from_secs(3_590) && left <= Duration::from_secs(3_600),
            "{left:?}"
        );
        assert_eq!(ample.git().time_limit.as_secs() / 60, 59);
    }

    /// A time limit is whatever the hook had left, so a reason must not name it: it would be a
    /// new reason at every tool call.
    #[test]
    fn a_timeout_is_worded_the_same_whatever_time_was_left() {
        let timed_out = |limit_ms| GitAcquireError::TimedOut {
            command: "git add",
            limit_ms,
        };
        assert_eq!(git_words(&timed_out(3_211)), git_words(&timed_out(17)));
        assert!(git_words(&timed_out(1)).contains("`git add` did not finish"));
        let other = GitAcquireError::BudgetExceeded { limit_bytes: 9 };
        assert_eq!(git_words(&other), other.to_string());
    }

    /// Independent review of the hooks, HR-L-1: the scratch directory of a process that is
    /// gone is removed by the next hook of the repository; one of a process that lives, a
    /// file, a link and any other name are left alone.
    #[test]
    fn scratch_directories_of_processes_that_are_gone_are_removed() {
        if !Path::new("/proc/self").exists() {
            return;
        }
        let scratch = deep_diff_forge_testkit::Scratch::new("hook-stale");
        let own = scratch.join("own");
        std::fs::create_dir(&own).unwrap();
        // a process id above the largest the kernel gives (2^22), so no process has it
        let gone = own.join("tmp-4999999");
        std::fs::create_dir_all(gone.join("work/objects")).unwrap();
        std::fs::write(gone.join("change.patch"), "patch").unwrap();
        // this test's own process is alive; so is process 1
        let mine = own.join(format!("tmp-{}", std::process::id()));
        let init = own.join("tmp-1");
        std::fs::create_dir(&init).unwrap();
        // not scratch directories of a hook
        let elsewhere = scratch.join("elsewhere");
        std::fs::create_dir(&elsewhere).unwrap();
        std::fs::write(elsewhere.join("kept"), "kept").unwrap();
        std::os::unix::fs::symlink(&elsewhere, own.join("tmp-4999998")).unwrap();
        std::fs::write(own.join("tmp-4999997"), "a file").unwrap();
        for other in ["tmp-", "tmp-12x", "tmp-4999996.partial", "other-4999995"] {
            std::fs::create_dir(own.join(other)).unwrap();
        }

        let made = Scratch::new(&own).unwrap();
        assert_eq!(made.0, mine);
        assert!(
            !gone.exists(),
            "the directory of a process that is gone stays"
        );
        assert!(init.is_dir() && mine.is_dir());
        assert!(elsewhere.join("kept").is_file());
        assert!(own.join("tmp-4999998").is_symlink());
        assert!(own.join("tmp-4999997").is_file());
        for other in ["tmp-", "tmp-12x", "tmp-4999996.partial", "other-4999995"] {
            assert!(own.join(other).is_dir(), "{other}");
        }
        drop(made);
        assert!(!mine.exists());
    }
}
