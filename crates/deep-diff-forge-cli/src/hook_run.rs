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
    file_at_commit, head_contains, head_state, judge_patch, repository_dirs,
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
/// The most reasons one session's state keeps as told ([`notice`]). Past it, an advice hook
/// that cannot run says nothing more; a stop still says why it was not judged.
const NOTICED_CAP: usize = 16;
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
        Event::SessionStart => session_start(&input, args),
        Event::PostTool => post_tool(&input),
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
        Ok(dirs) => Found::Tree(Place {
            top: dirs.top,
            own: dirs.common_dir.join("deep-diff-forge"),
            common: dirs.common_dir,
        }),
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
    base: Base,
}

/// What one session remembers between hooks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct State {
    project: Option<Project>,
    /// The keys of findings already said in this session ([`Finding::key`]).
    reported: BTreeSet<String>,
    /// The reasons an advice hook already gave for not running, hashed ([`notice`]).
    noticed: BTreeSet<String>,
    /// The failure that blocked the last stop, if the last stop was blocked.
    last_block: Option<Vec<String>>,
}

const STATE_SCHEMA: &str = "deep-diff-forge.hook-state.v1";

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

fn state_from(text: &str) -> State {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return State::default();
    };
    if value["schema"] != STATE_SCHEMA {
        return State::default();
    }
    let strings = |member: &Value| -> Option<Vec<String>> {
        member
            .as_array()?
            .iter()
            .map(|v| v.as_str().map(str::to_owned))
            .collect()
    };
    // A project that cannot be read whole is no project: the base is not trusted in part.
    let project = match &value["project"] {
        Value::Null => None,
        project => {
            let read = || {
                let top = project["top"].as_str().filter(|top| !top.is_empty())?;
                Some(Project {
                    top: top.to_owned(),
                    base: base_from(project["base"].as_str()?)?,
                })
            };
            match read() {
                Some(project) => Some(project),
                None => return State::default(),
            }
        }
    };
    State {
        project,
        reported: strings(&value["reported"])
            .unwrap_or_default()
            .into_iter()
            .collect(),
        noticed: strings(&value["noticed"])
            .unwrap_or_default()
            .into_iter()
            .collect(),
        last_block: strings(&value["last_block"]),
    }
}

fn state_text(state: &State) -> String {
    json!({
        "schema": STATE_SCHEMA,
        "project": state.project.as_ref().map(|project| json!({
            "top": project.top,
            "base": base_text(&project.base),
        })),
        "reported": state.reported,
        "noticed": state.noticed,
        "last_block": state.last_block,
    })
    .to_string()
}

/// The most bytes of a state file that are read. A full state ([`REPORTED_CAP`] keys of 64 hex
/// digits and a block of a few rules) is under half of this.
const STATE_CAP: u64 = 1024 * 1024;

/// A missing, unreadable, foreign or oversized state file is an empty state: what follows is a
/// finding said twice, a failure that blocks once more, or a session whose changes are
/// measured from the commit `HEAD` names now. The file is opened without following a link and
/// without waiting on a FIFO, must be a regular file once open, and its size is checked before
/// anything is read: a state file of 512 MiB used to be read whole to find that it was not
/// JSON (review V4, VD4-M-1).
fn load(path: &Path) -> State {
    use std::os::unix::fs::OpenOptionsExt as _;
    let read = || -> Option<String> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK)
            .open(path)
            .ok()?;
        let found = file.metadata().ok()?;
        if !found.is_file() || found.len() > STATE_CAP {
            return None;
        }
        let mut text = String::new();
        file.take(STATE_CAP).read_to_string(&mut text).ok()?;
        Some(text)
    };
    read().map(|text| state_from(&text)).unwrap_or_default()
}

/// Write the state beside its file and move it into place, so a reader never sees half of it.
/// The partial file is made new, with mode 0600 and without following a link; the move replaces
/// whatever stands at the state file's name, a link included, and follows nothing.
fn save(path: &Path, state: &State) -> Result<(), String> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    let partial = path.with_extension(format!("{}.partial", std::process::id()));
    // what an earlier process with this id left behind
    let _ = std::fs::remove_file(&partial);
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(nix::libc::O_NOFOLLOW)
        .open(&partial)
        .and_then(|mut file| file.write_all(state_text(state).as_bytes()))
        .and_then(|()| std::fs::rename(&partial, path))
        .map_err(|e| {
            // what could not be moved into place is not left beside the state
            let _ = std::fs::remove_file(&partial);
            format!("its state could not be saved: {e}")
        })
}

/// A session's state, with the file it is kept in.
struct Session {
    file: PathBuf,
    state: State,
    /// The state as the file holds it.
    kept: State,
}

impl Session {
    /// The state of the session `id`, read from the user's state directory. `Err` when there
    /// is no such directory to keep a state in.
    fn open(id: &str) -> Result<Self, String> {
        let dir = sessions_dir()?;
        if let Some(parent) = dir.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("its state directory could not be made: {e}"))?;
        }
        own_directory(&dir)?;
        let file = state_file(&dir, id);
        let state = load(&file);
        Ok(Self {
            file,
            kept: state.clone(),
            state,
        })
    }

    /// Write the state if this hook changed it.
    fn keep(&mut self) -> Result<(), String> {
        if self.state != self.kept {
            save(&self.file, &self.state)?;
            self.kept = self.state.clone();
        }
        Ok(())
    }
}

/// What a hook has to look at.
enum Located {
    /// Nothing, and nothing is said: Git ran, found no repository around the session's
    /// directory, and the session has no project recorded.
    Nowhere,
    /// Something that must be said: the project could not be found, and why.
    Trouble(String),
    Here(Work),
}

/// The session's project, and the commit its changes are measured from.
struct Work {
    place: Place,
    base: Base,
    /// This hook is the session's first here: it recorded the project and the base.
    fresh: bool,
}

/// Find the session's project and the base of its changes, recording both in `state` when this
/// is the session's first hook that finds a repository.
///
/// The project is, in this order: the directory the client names (`CLAUDE_PROJECT_DIR`) when
/// it is in a Git working tree; the top level recorded in the session's state; the hook
/// input's `cwd`. The `cwd` alone used to decide, and it follows the agent: from a nested
/// repository, another repository or a directory that was gone, the project's change was not
/// judged and nothing was said (independent review of the hooks, HR-H-1).
fn locate(input: &Input, state: &mut State, deadline: Deadline) -> Located {
    let mut place = None;
    if let Some(dir) = client_project_dir() {
        match find(&dir, deadline) {
            Found::Tree(found) => place = Some(found),
            Found::Failed(reason) => return Located::Trouble(reason),
            Found::NotARepository | Found::Gone => {}
        }
    }
    if place.is_none()
        && let Some(project) = &state.project
    {
        let top = Path::new(&project.top);
        match find(top, deadline) {
            Found::Tree(found) => place = Some(found),
            Found::Failed(reason) => return Located::Trouble(reason),
            Found::NotARepository | Found::Gone => {
                return Located::Trouble(format!(
                    "the project this session began in, {}, is no longer a Git working tree",
                    shown(top)
                ));
            }
        }
    }
    let place = if let Some(place) = place {
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
    match &state.project {
        Some(project) if Path::new(&project.top) == place.top => Located::Here(Work {
            base: project.base.clone(),
            fresh: false,
            place,
        }),
        Some(project) => Located::Trouble(format!(
            "this session began in {}, and the project found now is {}",
            hook::shown_path(&project.top),
            shown(&place.top)
        )),
        None => {
            let Some(top) = place.top.to_str() else {
                return Located::Trouble("the project's path is not UTF-8".to_owned());
            };
            let base = match head_state(&place.top, &deadline.git()) {
                Ok(HeadState::Commit(id)) if is_commit_id(&id) => Base::Commit(id),
                Ok(HeadState::Commit(_)) => {
                    return Located::Trouble(
                        "Git named the commit of HEAD in a form that cannot be read".to_owned(),
                    );
                }
                Ok(HeadState::Unborn {
                    repository_has_commits: false,
                }) => Base::NoCommit,
                Ok(HeadState::Unborn { .. }) => Base::Orphan,
                Err(error) => {
                    return Located::Trouble(format!(
                        "the commit HEAD names could not be read: {}",
                        git_words(&error)
                    ));
                }
            };
            state.project = Some(Project {
                top: top.to_owned(),
                base: base.clone(),
            });
            Located::Here(Work {
                place,
                base,
                fresh: true,
            })
        }
    }
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
    match &work.base {
        Base::Commit(id) => {
            // A base that can no longer be read is found, and said, when the patch is made.
            let moved = matches!(
                head_contains(&work.place.top, id, &deadline.git()),
                Ok(false)
            );
            Subject::Range {
                base: id.clone(),
                words: hook::range_words(id, work.fresh, moved),
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

/// The working tree at `top` as the patch a judge would be given against the commit `base`,
/// or `None` when the tree equals it: only that is "no change". Every failure is an error with
/// its reason: when one answer stood for "nothing to judge" and for failures alike, a patch
/// that could not be made let a stop pass in silence.
fn change(
    top: &Path,
    base: &str,
    work: &Path,
    deadline: Deadline,
) -> Result<Option<Vec<u8>>, String> {
    match judge_patch(top, base, work, &deadline.git()) {
        Ok(patch) if patch.bytes.is_empty() => Ok(None),
        Ok(patch) => Ok(Some(patch.bytes)),
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

/// Tell a reason an advice hook has for not running, once in a session: `None` when it was
/// told before, or when [`NOTICED_CAP`] reasons were. A lasting condition (an untracked file
/// that cannot be read, a sparse checkout, Git refusing the repository) used to be said after
/// every tool call, or never (independent review of the hooks, HR-L-2 and HR-M-1).
fn notice(state: &mut State, reason: &str) -> Option<String> {
    if state.noticed.len() >= NOTICED_CAP {
        return None;
    }
    state
        .noticed
        .insert(deep_diff_forge_core::sha256_hex(reason.as_bytes()))
        .then(|| hook::notice_output(reason, state.noticed.len() == NOTICED_CAP))
}

/// After a tool ran: say what is new in the session's change. Advice only.
fn post_tool(input: &Input) -> Result<Option<String>, String> {
    let deadline = Deadline::after(ADVICE_LIMIT);
    let mut session = Session::open(&input.session)?;
    let advice = match locate(input, &mut session.state, deadline) {
        Located::Nowhere => return Ok(None),
        Located::Trouble(reason) => Err(reason),
        Located::Here(work) => advice_for(&work, &mut session.state, deadline),
    };
    let out = match advice {
        Ok(advice) => advice,
        Err(reason) => notice(&mut session.state, &reason),
    };
    session.keep()?;
    Ok(out)
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
    let Some(patch) = change(&work.place.top, &base, &scratch.sub("work")?, deadline)? else {
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
fn session_start(input: &Input, args: &[String]) -> Result<Option<String>, String> {
    let deadline = Deadline::after(START_LIMIT);
    let mut session = Session::open(&input.session)?;
    let work = match locate(input, &mut session.state, deadline) {
        Located::Nowhere => return Ok(None),
        Located::Trouble(reason) => return Ok(Some(hook::contract_trouble(&reason))),
        Located::Here(work) => work,
    };
    let unsaved = session.keep().err();
    let id = match &work.base {
        Base::Commit(id) => id,
        Base::NoCommit => {
            return Ok(Some(hook::contract(
                StartBase::NoCommit,
                &[],
                false,
                unsaved.as_deref(),
            )));
        }
        Base::Orphan => {
            return Ok(Some(hook::contract(
                StartBase::Orphan,
                &[],
                false,
                unsaved.as_deref(),
            )));
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
    Ok(Some(hook::contract(
        StartBase::Commit {
            id,
            resumed: !work.fresh,
        },
        &not_ready,
        gate.dirty,
        unsaved.as_deref(),
    )))
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

/// Another working tree of the project's repository that the session is in now: `Ok` with its
/// top level when the hook input's `cwd` lies in a linked working tree (the same Git common
/// directory, another top level), `Err` when Git could not say. `None` for every other `cwd`:
/// in the project's own tree, in another repository, outside any, or gone. The project is
/// judged wherever `cwd` is; this only adds a tree.
fn other_tree(cwd: &str, place: &Place, deadline: Deadline) -> Option<Result<PathBuf, String>> {
    let cwd = std::fs::canonicalize(cwd).ok()?;
    // The project's own top level, or a place inside the Git directory, where no working
    // tree is.
    if cwd == place.top || cwd.starts_with(&place.common) {
        return None;
    }
    match find(&cwd, deadline) {
        Found::Tree(found) if found.common == place.common && found.top != place.top => {
            Some(Ok(found.top))
        }
        Found::Tree(_) | Found::NotARepository | Found::Gone => None,
        Found::Failed(reason) => Some(Err(format!(
            "whether the session's directory is another working tree of this repository could not be found out: {reason}"
        ))),
    }
}

/// Have the session's change judged in every working tree it is in: the project's and, when
/// the session entered a linked working tree of the same repository, that one too, each
/// against the same base with the same policy. A tree that equals the base has nothing to
/// judge; when that is all of them, no judge is needed and none is looked for.
fn judge_trees(
    place: &Place,
    cwd: &str,
    base: &str,
    scratch: &Scratch,
    args: &[String],
    deadline: Deadline,
) -> Result<Vec<TreeJudged>, String> {
    let mut trees = vec![(
        place.top.clone(),
        change(&place.top, base, &scratch.sub("work")?, deadline),
    )];
    match other_tree(cwd, place, deadline) {
        None => {}
        Some(Ok(top)) => {
            let patch = change(&top, base, &scratch.sub("work-2")?, deadline);
            trees.push((top, patch));
        }
        Some(Err(reason)) => trees.push((PathBuf::from(cwd), Err(reason))),
    }
    let named = |tree: &Path| tree.to_string_lossy().into_owned();
    // A judge is looked for only when some tree has a patch for it.
    let bench = if trees.iter().any(|(_, patch)| matches!(patch, Ok(Some(_)))) {
        let mut writable: Vec<&Path> = trees.iter().map(|(top, _)| top.as_path()).collect();
        writable.push(&place.common);
        Some(Bench::new(
            &place.top, base, scratch, args, &writable, deadline,
        ))
    } else {
        None
    };
    Ok(trees
        .iter()
        .enumerate()
        .map(|(at, (tree, patch))| TreeJudged {
            tree: named(tree),
            judged: match (patch, &bench) {
                (Ok(None), _) => None,
                (Err(reason), _) | (Ok(Some(_)), Some(Err(reason))) => Some(Judged::Unjudged {
                    reason: reason.clone(),
                }),
                (Ok(Some(bytes)), Some(Ok(bench))) => {
                    Some(bench.judge(bytes, &format!("change-{at}.patch")))
                }
                // A patch with no bench cannot be: the bench is made when any tree has one.
                (Ok(Some(_)), None) => Some(Judged::Unjudged {
                    reason: "the judge was not prepared".to_owned(),
                }),
            },
        })
        .collect())
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

/// When the agent wants to stop: have the session's change judged, and block only on a fail
/// that is not the one that blocked last time.
fn stop(input: &Input, args: &[String]) -> Result<Option<String>, String> {
    let deadline = Deadline::after(STOP_LIMIT);
    let mut session = Session::open(&input.session)?;
    let work = match locate(input, &mut session.state, deadline) {
        Located::Nowhere => return Ok(None),
        Located::Trouble(reason) => return Ok(Some(hook::unjudged_output(&reason))),
        Located::Here(work) => work,
    };
    // The project and the base are recorded before anything else can fail.
    let mut unsaved = session.keep().err();
    let decision = match subject(&work, deadline) {
        Subject::Nothing => StopDecision::Allow { message: None },
        Subject::Unjudged(reason) => StopDecision::Allow {
            message: Some(hook::unjudged_message(&reason)),
        },
        Subject::Range { base, words } => {
            let scratch = Scratch::new(&work.place.own)?;
            let trees = judge_trees(&work.place, &input.cwd, &base, &scratch, args, deadline)?;
            hook::decide_stop_trees(
                &trees,
                input.stop_hook_active,
                session.state.last_block.as_deref(),
                &words,
            )
        }
    };
    session.state.last_block = match &decision {
        StopDecision::Block { rules, .. } => Some(rules.clone()),
        StopDecision::Allow { .. } => None,
    };
    if let Err(why) = session.keep() {
        unsaved = Some(why);
    }
    let mut notes = Vec::new();
    // A stop that is its session's first hook has only now recorded where the session is: it
    // is never silent, whatever the judge says of what is left in the tree.
    if work.fresh {
        notes.push(hook::FIRST_STOP.to_owned());
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
}

/// Every `Stop` hook of type `command` a Claude Code settings document registers.
fn stop_hooks(settings: &Value) -> Vec<StopHook> {
    let entries = settings["hooks"]["Stop"].as_array();
    entries
        .into_iter()
        .flatten()
        .filter_map(|entry| entry["hooks"].as_array())
        .flatten()
        .filter(|hook| hook["type"] == "command")
        .filter_map(|hook| {
            Some(StopHook {
                command: hook["command"].as_str()?.to_owned(),
                timeout: hook.get("timeout").filter(|set| !set.is_null()).cloned(),
            })
        })
        .collect()
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
) -> [GateCheck; 3] {
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
    [enabled, registered, timed]
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

    #[test]
    fn a_state_round_trips_and_anything_else_is_an_empty_state() {
        let commit = "0123456789abcdef0123456789abcdef01234567";
        let state = State {
            project: Some(Project {
                top: "/r".to_owned(),
                base: Base::Commit(commit.to_owned()),
            }),
            reported: ["b".to_owned(), "a".to_owned()].into(),
            noticed: ["n".to_owned()].into(),
            last_block: Some(vec!["r\0s\0p\x001".to_owned()]),
        };
        assert_eq!(state_from(&state_text(&state)), state);
        for base in [Base::NoCommit, Base::Orphan, Base::Commit("a".repeat(64))] {
            let other = State {
                project: Some(Project {
                    top: "/r".to_owned(),
                    base,
                }),
                ..State::default()
            };
            assert_eq!(state_from(&state_text(&other)), other);
        }
        let none = State::default();
        assert_eq!(state_from(&state_text(&none)), none);
        // A project that is not whole is no state at all: a base is not believed in part, and
        // only a full commit id is one (a name like `HEAD` would move with the session).
        for project in [
            r#"{"top":"/r"}"#,
            r#"{"base":"none"}"#,
            r#"{"top":"","base":"none"}"#,
            r#"{"top":"/r","base":"HEAD"}"#,
            r#"{"top":"/r","base":"0123456"}"#,
            r#"{"top":"/r","base":"0123456789ABCDEF0123456789abcdef01234567"}"#,
            r#"{"top":"/r","base":7}"#,
            r#""/r""#,
        ] {
            let text = format!(
                r#"{{"schema":"deep-diff-forge.hook-state.v1","project":{project},"reported":["a"]}}"#
            );
            assert_eq!(state_from(&text), State::default(), "{project}");
        }
        for foreign in [
            "",
            "not json",
            "[]",
            r#"{"schema":"other","reported":["a"]}"#,
            r#"{"schema":"deep-diff-forge.hook-state.v0","reported":["a"]}"#,
            r#"{"reported":["a"]}"#,
        ] {
            assert_eq!(state_from(foreign), State::default(), "{foreign}");
        }
        // a member of the wrong type is dropped, not trusted in part
        let odd =
            r#"{"schema":"deep-diff-forge.hook-state.v1","reported":["a",1],"last_block":"x"}"#;
        assert_eq!(state_from(odd), State::default());
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
                timeout: Some(json!(1))
            }]
        );

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
