//! `agent hook <event> --client claude`: read the client's JSON, look at the repository, print
//! what [`crate::hook`] decides.
//!
//! This is the part that touches the world: stdin, Git, a scratch directory, the state of a
//! session (up to three files) and, at a stop, the judge. Every decision is made in [`crate::hook`].
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
//! **A stop judges units.** The uncommitted and the staged change of the project's working tree (against that
//! tree's `HEAD`), and each commit of the session that no stop has judged yet, by its own
//! change: the patch from its first parent to itself. A commit is the session's when a branch,
//! a tag, the stash, the `HEAD` of a working tree or the reflog of such a `HEAD` or of the
//! stash leads to it and none of the commits that were named when the session began does.
//! Before, a stop judged the net patch from the base to each ref's tip: a credential that was
//! committed and then reverted was in no patch (review V4 round 11, VD4R11-H-1; third
//! independent review of the hooks, HR3-H-1, HR3-H-2, HR3-M-1, HR3-M-2).
//!
//! **A failure is said.** Nothing is said only when Git ran and found no repository for a
//! session that has none recorded, when the repository had no commit as the session began, or
//! when nothing is uncommitted and every commit of the session has passed. And a stop that is the first hook of its session is never silent
//! once it has found a project: it has only then recorded where the session is, so it says
//! that what was committed before it was not judged.
//!
//! The state file of a session, and beside it the list of the commits that were there when the
//! session began, are in the user's state directory (`deep-diff-forge/hooks` under
//! `$XDG_STATE_HOME`, else under `~/.local/state`), where they are found by the session's id
//! alone: a hook must be able to read them before, and without, finding the repository. The scratch
//! directory lives in `deep-diff-forge/` under the repository's Git common directory, beside
//! Git's own files: nothing appears in the working tree, and the objects written while a patch
//! is made stay on the same file system and are removed again.

use crate::emit;
use crate::hook::{
    self, Event, Finding, Holds, Input, Judged, StartBase, StopDecision, TreeJudged,
};
use deep_diff_forge_core::PatchLineKind;
use deep_diff_forge_core::process::{Limits, run_bounded};
use deep_diff_forge_graph::Side;
use deep_diff_forge_patch::{
    CommitAndParent, GitAcquireError, GitAcquireOptions, HeadState, InputGuards, JudgePatchError,
    Named, RepositoryDirs, SealedReview, Worktree, commit_patch, commits_beyond, dirty_submodules,
    file_at_commit, has_uncommitted_changes, head_contains, head_state, judge_patch, named_commits,
    repository_dirs, staged_patch, worktrees,
};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};
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
/// The most things one session's state keeps as said once ([`say_once`]). Past it, an advice
/// hook that cannot run says nothing more; a stop still says why it was not judged, and says
/// once that there is more beside the verdict than it names.
const NOTICED_CAP: usize = 32;
/// The most entries one session's state keeps of the failures of uncommitted and staged
/// changes that have blocked a stop: failing lines, and marks that stand for all the lines of
/// one change ([`within_bound`] says what is done when there are more).
const BLOCKED_CAP: usize = 2_048;
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
///
/// It also carries the ceiling every Git call of the hook has once the project is known
/// ([`Deadline::with_project`]): `None` leaves the caller's `GIT_CEILING_DIRECTORIES` as it is.
#[derive(Debug, Clone, Copy)]
struct Deadline {
    at: std::time::Instant,
    ceiling: Option<&'static OsStr>,
}

impl Deadline {
    fn after(limit: Duration) -> Self {
        Self {
            at: now() + limit,
            ceiling: None,
        }
    }

    fn left(self) -> Duration {
        self.at.saturating_duration_since(now())
    }

    /// This deadline, or `time` from now when that is sooner.
    fn within(self, time: Duration) -> Self {
        Self {
            at: self.at.min(now() + time),
            ..self
        }
    }

    /// This deadline `time` earlier (no earlier than it can be).
    fn sooner_by(self, time: Duration) -> Self {
        Self {
            at: self.at.checked_sub(time).unwrap_or(self.at),
            ..self
        }
    }

    /// This deadline, with the ceiling of every Git call made for the project at `place`: the
    /// caller's `GIT_CEILING_DIRECTORIES` without the entries that lie at or under one of the
    /// project's working trees ([`ceiling_without`]). One process makes one such ceiling.
    fn with_project(self, place: &Place) -> Self {
        let Some(ceiling) = callers_ceiling() else {
            return self;
        };
        let mut trees = vec![resolved(&place.top)];
        trees.extend(
            linked_instances(&place.common)
                .trees
                .into_iter()
                .filter(|(top, _)| !top.as_os_str().is_empty())
                .map(|(top, _)| top),
        );
        let without = ceiling_without(&ceiling, &trees);
        Self {
            ceiling: Some(Box::leak(without.into_boxed_os_str())),
            ..self
        }
    }

    /// Git options whose time limit is the time that is left, and with this deadline's
    /// ceiling ([`Deadline::with_project`]).
    fn git(self) -> GitAcquireOptions {
        GitAcquireOptions {
            time_limit: self.left(),
            env: self
                .ceiling
                .map(|ceiling| {
                    (
                        OsString::from("GIT_CEILING_DIRECTORIES"),
                        ceiling.to_os_string(),
                    )
                })
                .into_iter()
                .collect(),
            ..GitAcquireOptions::default()
        }
    }
}

/// The `GIT_CEILING_DIRECTORIES` the hook was started with, when it has one.
#[expect(
    clippy::disallowed_methods,
    reason = "the Git-ceiling door: the caller's ceiling is kept for what is not the project"
)]
fn callers_ceiling() -> Option<OsString> {
    std::env::var_os("GIT_CEILING_DIRECTORIES")
}

/// The ceiling `ceiling` (a list of directories, `:` between them) without every absolute
/// entry that lies at or under one of `trees`, the project's working trees as they are on
/// disk; an entry is compared in the one form it has on disk, its `..` and its links resolved
/// ([`through_links`]; where a part before a `..` is not there, its `..` taken out by name,
/// [`same_form`]). The other entries stay as they are, in their order. An entry was also
/// compared as written: `R/../H`, which is the repository `H` beside the project `R`, starts
/// with `R` as written and was left out, and the repository it hides was found (fifteenth
/// independent review of the hooks). The resolved form is the one that counts, found with
/// Git 2.56: Git takes an entry through `realpath` (an entry whose path cannot be resolved
/// hides nothing), and an entry after an empty one as written, compared with the directory
/// it looks from as the system gives it, which holds no `..` and passes through no link; so
/// an entry as written that starts with a tree's path but resolves outside it hides nothing
/// of that tree.
///
/// Round 22 emptied the ceiling for every Git call of the hooks: a ceiling inside a linked
/// tree made Git say "not a git repository" for a directory below it, and the tree the
/// session was in was not judged from there (thirteenth independent review of the hooks,
/// suspected; reproduced in round 22). That also found a repository the user hides with a
/// ceiling (a home directory's): a session below it, in no repository to the user's Git,
/// took it for its project, with an UNJUDGED line at every stop and a read of the whole home
/// at every tool call (fourteenth independent review of the hooks, HR14-L-1). Now the
/// project is found with the caller's ceiling as it is, and once it is known only an entry
/// that would hide a part of one of its trees is left out: a repository hidden above the
/// project stays hidden.
fn ceiling_without(ceiling: &OsStr, trees: &[PathBuf]) -> OsString {
    use std::os::unix::ffi::{OsStrExt as _, OsStringExt as _};
    let kept: Vec<&[u8]> = ceiling
        .as_bytes()
        .split(|byte| *byte == b':')
        .filter(|entry| {
            let path = Path::new(OsStr::from_bytes(entry));
            if !path.is_absolute() {
                return true;
            }
            let whole = through_links(path).unwrap_or_else(|| same_form(path));
            !trees.iter().any(|tree| whole.starts_with(tree))
        })
        .collect();
    OsString::from_vec(kept.join(&b':'))
}

/// A Git failure in words that do not change from one hook to the next: the time limit of a
/// call is whatever its hook had left, and a reason that named it would be a new reason at
/// every tool call ([`notice`] tells each reason once).
fn git_words(error: &GitAcquireError) -> String {
    match error {
        GitAcquireError::TimedOut { command, .. } => {
            format!("`{command}` did not finish in the time a hook has and was stopped")
        }
        // Git's reason is its `fatal:` or `error:` line; a hint it prints first used to be all
        // that a message had room for (fifth independent review of the hooks, HR5-L-4).
        GitAcquireError::CommandFailed {
            command, stderr, ..
        } => {
            let lines = || {
                stderr
                    .lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
            };
            let reason = lines()
                .rfind(|line| line.starts_with("fatal:") || line.starts_with("error:"))
                .or_else(|| lines().next_back())
                .unwrap_or("it wrote no reason");
            format!("`{command}` failed: {reason}")
        }
        // (a refusal of the hooks' own, with its reason: "invalid Git selection:" in front
        // of it said nothing of it)
        GitAcquireError::InvalidSelection { detail } => detail.clone(),
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
    /// The Git directory of this working tree: the common directory for the main tree, the
    /// tree's administrative directory under it for a linked one.
    git_dir: PathBuf,
    /// Which repository this is ([`identity`]).
    id: String,
}

/// Which repository a Git common directory is: its inode. It stays when the repository is
/// moved, and a clone put at the old path has another. The device number stood before it and
/// is left out: it is given when the file system is mounted (on btrfs, tmpfs and overlayfs it
/// can differ after a reboot), and a session resumed then was told that another repository
/// stood at its path (third independent review of the hooks, HR3-M-4).
fn identity(common: &Path) -> Option<String> {
    use std::os::unix::fs::MetadataExt as _;
    Some(std::fs::metadata(common).ok()?.ino().to_string())
}

/// What Git says of a directory.
enum Found {
    /// It is in this working tree.
    Tree(Place),
    /// Git ran and found no repository around it.
    NotARepository,
    /// Git ran and found no repository up to a file-system boundary, which it does not cross
    /// while `GIT_DISCOVERY_ACROSS_FILESYSTEM` is not set: `words` are Git's, `mount` the
    /// directory it names as the mount point, the first one it did not look in (`None` when
    /// it cannot be read from them). To the hooks' other uses this is
    /// [`Found::NotARepository`]; a stop asks whether a tree of its project lies across it
    /// ([`elsewhere`]).
    AtBoundary {
        words: String,
        mount: Option<PathBuf>,
    },
    /// It does not exist, or is not a directory.
    Gone,
    /// Git could not say: it is missing, it refuses the repository (another owner), the
    /// directory is inside a Git directory, or it ran out of time. `reason` says it with the
    /// directory; `words` are what failed alone (Git's own words, when it gave them).
    Failed { reason: String, words: String },
    /// Git named a working tree and a Git directory that the repository's own records do not
    /// pair ([`pair_of`]): what it says is not believed, and neither directory is taken for
    /// that tree.
    Unpaired(Box<Unpaired>),
}

/// What Git named for a directory, which the repository's records do not pair
/// ([`Found::Unpaired`]).
#[derive(Debug, Clone)]
struct Unpaired {
    /// Which repository it is ([`identity`]).
    id: String,
    /// The working tree Git named.
    top: PathBuf,
    /// The Git directory Git named with it.
    git_dir: PathBuf,
    /// The working tree the records of that Git directory name, when they name one that can
    /// be read.
    recorded: Option<PathBuf>,
}

impl Unpaired {
    /// What Git named, in words: the two working trees first, so that a reason a message cuts
    /// for room ([`hook::REASON_CHARS`]) keeps them; the Git directory goes with the paths at
    /// the end ([`Unpaired::git_dir_words`]).
    fn words(&self) -> String {
        format!(
            "Git names {} as the working tree of a Git directory whose records name {}",
            shown(&self.top),
            self.recorded.as_deref().map_or_else(
                || "no working tree that can be read".to_owned(),
                |recorded| format!("{} as that tree", shown(recorded))
            )
        )
    }

    /// The Git directory Git named, in words.
    fn git_dir_words(&self) -> String {
        format!("the Git directory {}", shown(&self.git_dir))
    }

    /// Why a hook that has to know which working tree `dir` is in cannot.
    fn reason(&self, dir: &Path) -> String {
        format!(
            "{}: which working tree {} is in could not be found out ({})",
            self.words(),
            shown(dir),
            self.git_dir_words()
        )
    }
}

/// What the records of a repository say of a working tree and a Git directory that Git named
/// together ([`pair_of`]).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Pair {
    /// They are one tree.
    One,
    /// The main Git directory of a repository that records no top level of its main tree:
    /// the directory is not named `.git` (one made apart with `--separate-git-dir`, or a
    /// submodule's under `.git/modules`), and `git worktree list` names the Git directory
    /// itself as that tree. Of the project's own repository a stop asks it against the
    /// project ([`elsewhere`]).
    Unrecorded,
    /// They are not one tree: the top level the records name for that Git directory, when
    /// they name one that can be read.
    Not(Option<PathBuf>),
}

/// Whether the working tree `top` and the Git directory `git_dir` that Git named together are
/// one tree of the repository whose common directory is `common`, by the repository's own
/// records. All three are as they are on disk.
///
/// - A linked tree's Git directory (`<common>/worktrees/<id>`) records its tree in its file
///   `gitdir`, which names the tree's `.git` file: that file's directory is the tree's top
///   level, as `git worktree list` gives it ([`linked_instances`]). A Git directory that is
///   not the common one and not under `worktrees`, and a `gitdir` that is missing or cannot
///   be read, pair with no tree.
/// - The main Git directory, named `.git`, is the main tree's: its directory is that tree's
///   top level, as `git worktree list` gives it.
///
/// Git's answer was believed when the top it named held the session's directory. That is
/// containment, not one tree: `core.worktree` in a linked tree's own configuration pointing
/// at an outer linked tree it lies in, or at a subdirectory of itself with a clean copy of
/// its files, and a copy of a linked tree made with `cp -a` (its `.git` file names the same
/// Git directory), each made Git name a directory that holds the session's directory with
/// the Git directory of another tree. That directory was judged as the tree, found clean,
/// and what had blocked in the tree was forgotten; the next stop was silent with
/// credentials there (review V4 round 23, VD4R23-M-1; fifteenth independent review of the
/// hooks, HR15-L-1), or blocked a second time for the same failure (HR15-L-2).
fn pair_of(top: &Path, git_dir: &Path, common: &Path) -> Pair {
    if git_dir == common {
        if common.file_name() != Some(OsStr::new(".git")) {
            return Pair::Unrecorded;
        }
        return match common.parent() {
            Some(main) if main == top => Pair::One,
            main => Pair::Not(main.map(Path::to_path_buf)),
        };
    }
    let under_worktrees = git_dir.parent().is_some_and(|parent| {
        std::fs::canonicalize(common.join("worktrees")).is_ok_and(|all| all == parent)
    });
    if !under_worktrees {
        return Pair::Not(None);
    }
    let recorded = pointed_path(&git_dir.join("gitdir"), b"", git_dir)
        .and_then(|dot_git| dot_git.parent().map(same_form));
    match recorded {
        Some(recorded) if recorded == top => Pair::One,
        recorded => Pair::Not(recorded),
    }
}

/// What the directories `dirs` that Git named for `dir` are to a hook: the working tree they
/// name, when the repository's records pair its top level with its Git directory
/// ([`pair_of`]).
fn placed(dir: &Path, dirs: RepositoryDirs) -> Found {
    let Some(id) = identity(&dirs.common_dir) else {
        return Found::Failed {
            reason: format!(
                "the Git directory of the repository {} is in could not be read",
                shown(dir)
            ),
            words: format!(
                "the Git directory {} could not be read",
                shown(&dirs.common_dir)
            ),
        };
    };
    if let Pair::Not(recorded) = pair_of(&dirs.top, &dirs.git_dir, &dirs.common_dir) {
        return Found::Unpaired(Box::new(Unpaired {
            id,
            top: dirs.top,
            git_dir: dirs.git_dir,
            recorded,
        }));
    }
    Found::Tree(Place {
        top: dirs.top,
        own: dirs.common_dir.join("deep-diff-forge"),
        git_dir: dirs.git_dir,
        common: dirs.common_dir,
        id,
    })
}

/// What Git prints when it finds no repository around a directory, in the `C` locale.
const NO_REPOSITORY: &str = "not a git repository (or any";
/// What Git names when it stopped looking for a repository at a file-system boundary.
const BOUNDARY: &str = "GIT_DISCOVERY_ACROSS_FILESYSTEM";

/// The mount point Git names when it stopped at a file-system boundary: what stands between
/// "(or any parent up to mount point " and the line that says it stopped, in the `C` locale.
fn mount_point(stderr: &str) -> Option<PathBuf> {
    const BEFORE: &str = "(or any parent up to mount point ";
    const AFTER: &str = ")\nStopping at filesystem boundary";
    let from = stderr.find(BEFORE)? + BEFORE.len();
    let to = stderr.rfind(AFTER)?;
    let named = stderr
        .get(from..to)
        .filter(|named| named.starts_with('/'))?;
    Some(PathBuf::from(named))
}

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
        Ok(dirs) => placed(dir, dirs),
        Err(ref error @ GitAcquireError::CommandFailed { ref stderr, .. })
            if stderr.contains(NO_REPOSITORY) && stderr.contains(BOUNDARY) =>
        {
            Found::AtBoundary {
                mount: mount_point(stderr),
                words: format!(
                    "{}; Git stopped at a file-system boundary ({BOUNDARY} is not set)",
                    git_words(error)
                ),
            }
        }
        Err(GitAcquireError::CommandFailed { stderr, .. }) if stderr.contains(NO_REPOSITORY) => {
            Found::NotARepository
        }
        Err(error) => {
            let words = git_words(&error);
            Found::Failed {
                reason: format!(
                    "Git could not say which repository {} is in: {words}",
                    shown(dir)
                ),
                words,
            }
        }
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
    /// Which repository that is: the inode of its Git common directory ([`identity`]). With
    /// the path it must still be what stands there: a clean clone put at the project's path
    /// was taken for the project (review V4 round 10, VD4R10-H-1).
    id: String,
    base: Base,
    /// The SHA-256 and the number of lines of the session's tips file ([`tips_file`]): the id
    /// of every commit that a ref, a `HEAD` or a reflog named when the session began. A
    /// commit that something names now and none of those leads to is the session's. The
    /// file is not the state's own, so the state says what it must hold: a tips file that
    /// was changed or cut short is not believed.
    tips_sha256: String,
    tips_count: usize,
    /// Why there is no such list, when there is none: the commits that were there when the
    /// session began could not be listed, for a reason that is the repository's and does
    /// not pass (it has more working trees, or more commits named, than a hook reads). Then
    /// the digest above is empty and the count zero, no tips file is written, and the
    /// session's commits cannot be told from the others: they are UNJUDGED at every stop,
    /// with this reason. The project and the base are recorded all the same, so the working
    /// tree's own changes are judged. It is a state of its own, not an empty list: an empty
    /// list says "nothing was there", and every commit would be the session's.
    no_baseline: Option<String>,
    /// A session-start hook recorded this. Otherwise the base is only where `HEAD` was when
    /// the session's first hook ran.
    by_start: bool,
}

/// What one session remembers between hooks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct State {
    project: Option<Project>,
    /// How many stops of this session have judged its work: each commit that is asked about
    /// is marked with the number of its stop ([`sorted_commits`]).
    stops: u64,
    /// The keys of findings already said in this session ([`Finding::key`]).
    reported: BTreeSet<String>,
    /// What was already said once in this session, hashed: the reasons an advice hook gave for
    /// not running ([`notice`]) and the things a stop named as not judged.
    noticed: BTreeSet<String>,
    /// The failing lines of uncommitted and staged changes that have blocked a stop and were
    /// found again at every stop that looked since, each as [`hook::remembered_line`] writes
    /// it: the working tree, whether it is staged, the line (a rule, a signal and a path) and
    /// its number of items ([`remember`]).
    blocked: Vec<String>,
    /// The SHA-256 and the number of lines of the session's file of judged commits
    /// ([`judged_file`]): the first so many lines of that file are the commits of the session
    /// that have their answer, each as `ID:p`, `ID:h`, `ID:f` or `ID:u` (it passed, was held,
    /// failed, or cannot be judged), in the order they got it. A commit is asked about once
    /// in a session. With no line yet there is no file and the digest is empty.
    judged_sha256: String,
    judged_count: usize,
    /// The top level of each linked working tree that something in `blocked` is remembered
    /// of, by which tree it is ([`tree_instance`]), as the tree's administrative directory
    /// last recorded it. A tree the repository no longer lists is forgotten only when this
    /// directory is gone: what blocked there stands while it is there ([`unlisted_trees`]).
    /// The ids alone were kept, and a tree whose `.git` file and administrative directory had
    /// been removed (or its `.git` file, and then `git worktree prune`) was forgotten while
    /// its files, and the credential that had blocked there, were still in place (fourteenth
    /// independent review of the hooks, HR14-L-2). A path that is not UTF-8 or is longer than
    /// [`POINTER_BYTES`], and the paths past [`TREES_BYTES`] in all, are not recorded: such a
    /// tree is forgotten when it is no longer listed, as before.
    trees: std::collections::BTreeMap<String, String>,
}

const STATE_SCHEMA: &str = "deep-diff-forge.hook-state.v10";
/// The schema before [`STATE_SCHEMA`]: read as one with no tree recorded (`trees` empty) and
/// written back in the new one. Refusing it would make every stop of a session that was
/// running when the hooks were upgraded UNJUDGED.
const STATE_SCHEMA_BEFORE: &str = "deep-diff-forge.hook-state.v9";
/// The most bytes of the paths in [`State::trees`], all together.
const TREES_BYTES: usize = 256 * 1024;

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

/// The identity of a repository as the state writes it: a number.
fn is_identity(text: &str) -> bool {
    !text.is_empty() && text.len() <= 20 && text.bytes().all(|b| b.is_ascii_digit())
}

/// A line of the file of judged commits ([`answer`]).
fn is_judged_entry(text: &str) -> bool {
    answer(text).is_some()
}

/// A SHA-256 as the state writes one.
fn is_digest(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The state a state file's text holds, without whether it is of the schema before.
#[cfg(test)]
fn state_from(text: &str) -> Result<State, &'static str> {
    state_read(text).map(|(state, _)| state)
}

/// Read a state file's text. `Err` says why it is not a state this version wrote: such a file
/// is never read as "no state", in part or whole. An empty state would make the next hook the
/// session's first, and the commit `HEAD` names then its base: what the session had committed
/// until then passed in silence (second independent review of the hooks, HR2-M-1).
///
/// With the state, whether the text is of the schema before ([`STATE_SCHEMA_BEFORE`]): such
/// a state is written back in the new one by the first hook that reads it and keeps its state,
/// whether or not that hook changes it ([`Session::keep`]). It is read into the same state as
/// one of the new schema with no tree recorded, so the state alone could not tell, and a hook
/// that changed nothing left the file as it was (review V4 round 23, VD4R23-L-1).
fn state_read(text: &str) -> Result<(State, bool), &'static str> {
    const MEMBER: &str = "one of its members is missing";
    let value = serde_json::from_str::<Value>(text).map_err(|_| "it is not JSON")?;
    let before = value["schema"] == STATE_SCHEMA_BEFORE;
    if value["schema"] != STATE_SCHEMA && !before {
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
                // why there is no list of the commits that were there: a reason, or null
                let no_baseline = match &project["no_baseline"] {
                    Value::Null => None,
                    Value::String(why) if !why.is_empty() => Some(why.clone()),
                    _ => return None,
                };
                if !project
                    .as_object()
                    .is_some_and(|members| members.contains_key("no_baseline"))
                {
                    return None;
                }
                Some(Project {
                    top: top.to_owned(),
                    id: project["id"]
                        .as_str()
                        .filter(|id| is_identity(id))?
                        .to_owned(),
                    base: base_from(project["base"].as_str()?)?,
                    tips_sha256: project["tips_sha256"]
                        .as_str()
                        .filter(|digest| {
                            if no_baseline.is_some() {
                                digest.is_empty()
                            } else {
                                is_digest(digest)
                            }
                        })?
                        .to_owned(),
                    tips_count: usize::try_from(project["tips_count"].as_u64()?)
                        .ok()
                        .filter(|count| no_baseline.is_none() || *count == 0)?,
                    no_baseline,
                    by_start: project["by_start"].as_bool()?,
                })
            };
            Some(read().ok_or("its project is not whole")?)
        }
    };
    let judged_count = value["judged_count"]
        .as_u64()
        .and_then(|count| usize::try_from(count).ok())
        .ok_or(MEMBER)?;
    let judged_sha256 = value["judged_sha256"].as_str().ok_or(MEMBER)?;
    // no line, no digest; else a whole one
    if (judged_count == 0) != judged_sha256.is_empty()
        || !(judged_sha256.is_empty() || is_digest(judged_sha256))
    {
        return Err("what it says of the judged commits is not whole");
    }
    let trees = if before {
        std::collections::BTreeMap::new()
    } else {
        trees_from(&value["trees"]).ok_or("what it says of the working trees is not whole")?
    };
    let state = State {
        trees,
        project,
        stops: value["stops"].as_u64().ok_or(MEMBER)?,
        reported: list("reported", REPORTED_CAP)?.into_iter().collect(),
        noticed: list("noticed", NOTICED_CAP)?.into_iter().collect(),
        blocked: list("blocked", BLOCKED_CAP)?,
        judged_sha256: judged_sha256.to_owned(),
        judged_count,
    };
    Ok((state, before))
}

fn state_text(state: &State) -> String {
    json!({
        "schema": STATE_SCHEMA,
        "project": state.project.as_ref().map(|project| json!({
            "top": project.top,
            "id": project.id,
            "base": base_text(&project.base),
            "tips_sha256": project.tips_sha256,
            "tips_count": project.tips_count,
            "no_baseline": project.no_baseline,
            "by_start": project.by_start,
        })),
        "stops": state.stops,
        "reported": state.reported,
        "noticed": state.noticed,
        "blocked": state.blocked,
        "judged_sha256": state.judged_sha256,
        "judged_count": state.judged_count,
        "trees": state.trees,
    })
    .to_string()
}

/// The member `trees` of a state ([`State::trees`]): an object whose keys are trees as
/// [`instance_of`] names them and whose values are absolute paths, as many and as long as
/// the hooks write. `None` when it is anything else.
fn trees_from(value: &Value) -> Option<std::collections::BTreeMap<String, String>> {
    let members = value.as_object()?;
    if members.len() > BLOCKED_CAP {
        return None;
    }
    let mut bytes = 0_usize;
    members
        .iter()
        .map(|(id, top)| {
            let top = top.as_str().filter(|top| recordable(top))?;
            bytes += top.len();
            (is_instance(id) && bytes <= TREES_BYTES).then(|| (id.clone(), top.to_owned()))
        })
        .collect()
}

/// A tree as [`instance_of`] names it: sixteen lowercase hex digits.
fn is_instance(text: &str) -> bool {
    text.len() == 16
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Whether the top level `top` of a tree can be recorded ([`State::trees`]).
fn recordable(top: &str) -> bool {
    top.starts_with('/') && top.len() as u64 <= POINTER_BYTES
}

/// The most bytes of a state file that are read. Every list in a state is bounded when it is
/// written ([`REPORTED_CAP`], [`NOTICED_CAP`], [`BLOCKED_CAP`]), and a state
/// with every list full is under a quarter of this (a test holds it). Before, the failure
/// that blocked was kept with a line for each violation and no bound, so the hooks wrote a
/// file they would not read back.
const STATE_CAP: u64 = 4 * 1024 * 1024;

/// The state a session's state file holds, without whether it is of the schema before.
#[cfg(test)]
fn load(path: &Path) -> Result<Option<State>, String> {
    load_read(path).map(|read| read.map(|(state, _)| state))
}

/// Read a session's state file: `Ok(None)` when there is none, `Err` with the reason when
/// something stands at its name that is not a state the hooks can read (a link, another kind
/// of file, a file over [`STATE_CAP`], other content). The file is opened without following a
/// link and without waiting on a FIFO, must be a regular file once open, and its size is
/// checked before anything is read (review V4, VD4-M-1). With the state, whether the file is
/// of the schema before ([`state_read`]).
fn load_read(path: &Path) -> Result<Option<(State, bool)>, String> {
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
    state_read(&text).map(Some).map_err(str::to_owned)
}

/// Write `text` beside `path` and move it into place, so a reader never sees half of it. The
/// partial file is made new, with mode 0600 and without following a link; the move replaces
/// whatever stands at the name, a link included, and follows nothing.
fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
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
        .and_then(|mut file| file.write_all(text.as_bytes()))
        .and_then(|()| std::fs::rename(&partial, path))
        .inspect_err(|_| {
            // what could not be moved into place is not left beside the state
            let _ = std::fs::remove_file(&partial);
        })
}

/// Write a session's state file.
fn save(path: &Path, state: &State) -> Result<(), String> {
    let text = state_text(state);
    // Never written: what the next hook would refuse to read.
    if text.len() as u64 > STATE_CAP {
        return Err("its state is larger than it reads back".to_owned());
    }
    write_private(path, &text).map_err(|e| format!("its state could not be saved: {e}"))
}

/// A session's tips file, beside its state file: one commit id a line.
fn tips_file(state_file: &Path) -> PathBuf {
    state_file.with_extension("tips")
}

/// A session's file of judged commits, beside its state file: one `ID:outcome` a line, in
/// the order the commits got their answer. It has no bound a session reaches: the state used
/// to keep the last 4,096 and count what a forgotten one led to as judged, an ancestor that
/// never got a verdict included (fourth independent review of the hooks, HR4-L-3).
fn judged_file(state_file: &Path) -> PathBuf {
    state_file.with_extension("judged")
}

/// Lines as a list file holds them, with the SHA-256 of that text.
fn tips_text(lines: &[String]) -> (String, String) {
    let mut text = String::with_capacity(lines.len() * 67);
    for line in lines {
        text.push_str(line);
        text.push('\n');
    }
    let digest = deep_diff_forge_core::sha256_hex(text.as_bytes());
    (text, digest)
}

/// The most bytes of a list file that are read: a million ids of the longer kind.
const TIPS_CAP: u64 = 67 * 1024 * 1024;

/// Read a list file of a session and hold it against what the state says of it: its first
/// `count` lines must be the text the state names by `sha256`, each a line `valid` accepts.
/// With `whole`, the file must end there. `Err` when it is missing, is not a regular file, or
/// is not that text: `called` says what the list is.
fn load_list(
    path: &Path,
    (sha256, count): (&str, usize),
    whole: bool,
    valid: fn(&str) -> bool,
    called: &str,
) -> Result<Vec<String>, String> {
    use std::os::unix::fs::OpenOptionsExt as _;
    let refused = |why: &str| {
        format!(
            "this session's list of {called}, {}, {why}, so which commits are the session's to judge is not known; start a new session to have its work judged",
            shown(path)
        )
    };
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK)
        .open(path)
        .map_err(|e| refused(&format!("cannot be opened ({e})")))?;
    let found = file
        .metadata()
        .map_err(|e| refused(&format!("cannot be read ({e})")))?;
    if !found.is_file() || found.len() > TIPS_CAP {
        return Err(refused("is not a file the hooks wrote"));
    }
    let mut text = String::new();
    file.take(TIPS_CAP)
        .read_to_string(&mut text)
        .map_err(|e| refused(&format!("cannot be read ({e})")))?;
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    if lines.len() < count || (whole && lines.len() != count) {
        return Err(refused("is not the list this session recorded"));
    }
    // What a hook wrote after the state it could not save is not the state's.
    lines.truncate(count);
    if tips_text(&lines).1 != sha256 || !lines.iter().all(|line| valid(line)) {
        return Err(refused("is not the list this session recorded"));
    }
    Ok(lines)
}

/// The commits that were there when a session began: every commit a ref, a `HEAD` or a
/// reflog named then, by its id; or that there is no such list, and why
/// ([`Project::no_baseline`]).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Tips {
    Listed(Vec<String>),
    Unavailable(String),
}

/// Read a session's tips file ([`load_list`]): all of it is what the state names.
fn load_tips(path: &Path, project: &Project) -> Result<Vec<String>, String> {
    load_list(
        path,
        (&project.tips_sha256, project.tips_count),
        true,
        is_commit_id,
        "the commits that were there when it began",
    )
}

/// A session's state, with the file it is kept in.
struct Session {
    /// The directory of every session's state.
    dir: PathBuf,
    file: PathBuf,
    state: State,
    /// The state as the file holds it; `None` when there is no file.
    kept: Option<State>,
    /// The file holds a state of the schema before ([`STATE_SCHEMA_BEFORE`]): it is written
    /// in the new one at the next [`Session::keep`], changed or not.
    upgrade: bool,
    /// The tips this hook recorded and has not written yet.
    tips: Option<Vec<String>>,
    /// The judged commits with what this hook added, not written yet.
    judged: Option<Vec<String>>,
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
        let read = load_read(&file).map_err(|why| {
            format!(
                "this session's state file {} cannot be read as a state ({why}), so the project and the commit this session began from are not known; nothing is recorded over it (remove the file to have the next hook record the session anew)",
                shown(&file)
            )
        })?;
        let (kept, upgrade) = read.map_or((None, false), |(state, before)| (Some(state), before));
        Ok(Self {
            dir,
            file,
            state: kept.clone().unwrap_or_default(),
            kept,
            upgrade,
            tips: None,
            judged: None,
        })
    }

    /// Write the state if this hook changed it, or if the file holds the schema before,
    /// making its directory first; and before it the lists this hook recorded, so that a state
    /// never names a list that is not there.
    fn keep(&mut self) -> Result<(), String> {
        if self.tips.is_none()
            && self.judged.is_none()
            && !self.upgrade
            && (self.kept.as_ref() == Some(&self.state)
                || (self.kept.is_none() && self.state == State::default()))
        {
            return Ok(());
        }
        if let Some(parent) = self.dir.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("its state directory could not be made: {e}"))?;
        }
        own_directory(&self.dir)?;
        let lists = [
            (tips_file(&self.file), &self.tips),
            (judged_file(&self.file), &self.judged),
        ];
        for (file, lines) in lists {
            if let Some(lines) = lines {
                write_private(&file, &tips_text(lines).0)
                    .map_err(|e| format!("its state could not be saved: {e}"))?;
            }
        }
        save(&self.file, &self.state)?;
        self.kept = Some(self.state.clone());
        self.upgrade = false;
        self.tips = None;
        self.judged = None;
        Ok(())
    }

    /// Write what an advice hook has to keep, and say whether it was written. The hook worked
    /// on the state as it read it, with no lock, for as long as Git took; meanwhile a stop
    /// may have judged and written. So the write is made under the session's lock, onto the
    /// state as it is then, and it adds only what an advice hook owns: the findings and the
    /// reasons it has said, and the project with its tips when the session still has none.
    /// What a stop owns (what has blocked, the judged commits, the count of stops) is never
    /// written back from the older copy: that brought a failure back that a stop had seen
    /// repaired and forgotten (seventh independent review of the hooks, HR7-L-4). `Ok(false)`
    /// when the lock could not be had in [`ADVICE_LOCK_WAIT`]: nothing is written.
    fn keep_advice(&mut self, id: &str) -> Result<bool, String> {
        let unchanged = self.tips.is_none()
            && (self.kept.as_ref() == Some(&self.state)
                || (self.kept.is_none() && self.state == State::default()));
        if unchanged {
            return Ok(true);
        }
        let turn = session_lock(id, ADVICE_LOCK_WAIT);
        if matches!(turn, Turn::Taken) {
            return Ok(false);
        }
        let read = self.kept.clone().unwrap_or_default();
        let said = |mine: &BTreeSet<String>, theirs: &BTreeSet<String>| -> Vec<String> {
            mine.difference(theirs).cloned().collect()
        };
        let (reported, noticed) = (
            said(&self.state.reported, &read.reported),
            said(&self.state.noticed, &read.noticed),
        );
        let mine = std::mem::take(&mut self.state);
        let read = load_read(&self.file).map_err(|why| {
            format!(
                "this session's state file {} cannot be read as a state ({why})",
                shown(&self.file)
            )
        })?;
        let (now, upgrade) = read.map_or((None, false), |(state, before)| (Some(state), before));
        self.upgrade = upgrade;
        self.kept.clone_from(&now);
        self.state = now.unwrap_or_default();
        if self.state.project.is_none() {
            self.state.project = mine.project;
        } else {
            // another hook recorded the session first: its tips are the session's
            self.tips = None;
        }
        keep_said(&mut self.state.reported, reported);
        for key in noticed {
            if self.state.noticed.len() < NOTICED_CAP {
                self.state.noticed.insert(key);
            }
        }
        self.keep().map(|()| true)
    }

    /// The session's tips: the ones this hook recorded and could not write yet, else the ones
    /// in its tips file, held against the state.
    fn tips(&self, project: &Project) -> Result<Tips, String> {
        if let Some(why) = &project.no_baseline {
            return Ok(Tips::Unavailable(why.clone()));
        }
        match &self.tips {
            Some(tips) => Ok(Tips::Listed(tips.clone())),
            None => load_tips(&tips_file(&self.file), project).map(Tips::Listed),
        }
    }

    /// The commits of the session that have their answer: the ones this hook holds unwritten,
    /// else the ones the state names in the session's file of them (none: no file is read).
    fn judged(&self) -> Result<Vec<String>, String> {
        match &self.judged {
            Some(judged) => Ok(judged.clone()),
            None if self.state.judged_count == 0 => Ok(Vec::new()),
            None => load_list(
                &judged_file(&self.file),
                (&self.state.judged_sha256, self.state.judged_count),
                false,
                is_judged_entry,
                "the commits that were judged",
            ),
        }
    }

    /// Take `judged` as the session's judged commits, to be written with the state.
    fn set_judged(&mut self, judged: Vec<String>) {
        self.state.judged_sha256 = if judged.is_empty() {
            String::new()
        } else {
            tips_text(&judged).1
        };
        self.state.judged_count = judged.len();
        self.judged = Some(judged);
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
}

/// Record the project at `place` for a session that has none: the commit `HEAD` names, which
/// repository it is, and the commits that were there (the tips, returned beside it).
fn record(place: &Place, deadline: Deadline) -> Result<(Project, Option<Vec<String>>), String> {
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
    // Everything that names a commit now: every ref, every working tree's `HEAD`, and the
    // reflogs, whose older entries no ref leads to and which are not this session's either.
    let tips = match named_commits(&place.top, Named::Everything, &deadline.git()) {
        Ok(tips) => Some(tips),
        // A refusal: the repository has more working trees, or more commits named, than a
        // hook reads. That does not pass, so asking again at the next hook is no use, and
        // it is no reason not to know the project and its base: they are recorded, with
        // no list. The whole record was thrown away, so every stop of a session that began
        // in a repository with 257 working trees met the same failure and never judged
        // the tree it was in (review V4 round 18, VD4R18-L-1).
        Err(GitAcquireError::InvalidSelection { detail }) => {
            return Ok((
                Project {
                    top: top.to_owned(),
                    id: place.id.clone(),
                    base,
                    tips_sha256: String::new(),
                    tips_count: 0,
                    no_baseline: Some(detail),
                    by_start: false,
                },
                None,
            ));
        }
        // Anything else may pass (Git was slow, a file was being written): nothing is
        // recorded, and the next hook of the session tries again, as its first.
        Err(error) => {
            return Err(format!(
                "the commits that are there as the session begins could not be listed, so its own could not be told from them: {} (`deep-diff-forge agent doctor --gate` checks this as the link `commits`; when it is ready, start a new session)",
                git_words(&error)
            ));
        }
    };
    if !tips.iter().flatten().all(|tip| is_commit_id(tip)) {
        return Err("Git named a commit in a form that cannot be read".to_owned());
    }
    let listed = tips.as_deref().unwrap_or_default();
    let project = Project {
        top: top.to_owned(),
        id: place.id.clone(),
        base,
        tips_sha256: tips_text(listed).1,
        tips_count: listed.len(),
        no_baseline: None,
        by_start: false,
    };
    Ok((project, tips))
}

/// What to do about a project that is not what was recorded: nothing in the session's state
/// can be moved to another repository, so the remedy is a new session.
const NEW_SESSION: &str = "start a new session to have the work there judged";

/// Find the session's project, recording it in the session when this is its first hook that
/// finds a repository.
///
/// With nothing recorded the project is the directory the client names (`CLAUDE_PROJECT_DIR`)
/// when it is in a Git working tree, else the hook input's `cwd`. The `cwd` alone used to
/// decide, and it follows the agent (independent review of the hooks, HR-H-1).
///
/// With a project recorded, the project is the repository that was recorded: what stands at
/// the recorded path must have the recorded identity (the inode of its Git common directory).
/// A clean clone put there is not the project (review V4 round 10, VD4R10-H-1), and neither
/// is the project once it was moved away: each is said, with the remedy. The identity holds
/// no device number: that is given when a file system is mounted, so a session resumed after
/// a reboot was refused for good, with a reason that was not true (third independent review
/// of the hooks, HR3-M-4).
fn locate(input: &Input, session: &mut Session, deadline: Deadline) -> Located {
    let client = client_project_dir();
    let named = match client.as_deref().map(|dir| (find(dir, deadline), dir)) {
        Some((Found::Tree(found), _)) => Some(found),
        Some((Found::Failed { reason, .. }, _)) => return Located::Trouble(reason),
        Some((Found::Unpaired(unpaired), dir)) => return Located::Trouble(unpaired.reason(dir)),
        Some((Found::NotARepository | Found::AtBoundary { .. } | Found::Gone, _)) | None => None,
    };
    let Some(project) = session.state.project.clone() else {
        let place = if let Some(place) = named {
            place
        } else {
            let cwd = Path::new(&input.cwd);
            match find(cwd, deadline) {
                Found::Tree(found) => found,
                Found::NotARepository | Found::AtBoundary { .. } => return Located::Nowhere,
                Found::Gone => {
                    return Located::Trouble(format!(
                        "the session's directory {} does not exist, and no project is recorded for this session",
                        shown(cwd)
                    ));
                }
                Found::Failed { reason, .. } => return Located::Trouble(reason),
                Found::Unpaired(unpaired) => return Located::Trouble(unpaired.reason(cwd)),
            }
        };
        return match record(&place, deadline) {
            Ok((project, tips)) => {
                session.state.project = Some(project.clone());
                session.tips = tips;
                Located::Here(Box::new(Work {
                    place,
                    project,
                    fresh: true,
                }))
            }
            Err(reason) => Located::Trouble(reason),
        };
    };
    let recorded = Path::new(&project.top);
    // The client names another directory than the one this session began in.
    if let Some(place) = &named
        && place.top != recorded
    {
        return Located::Trouble(format!(
            "this session began in {}, and the project found now is {}; {NEW_SESSION}",
            hook::shown_path(&project.top),
            shown(&place.top)
        ));
    }
    match find(recorded, deadline) {
        Found::Tree(found) if found.id == project.id && found.top == recorded => {
            Located::Here(Box::new(Work {
                place: found,
                project,
                fresh: false,
            }))
        }
        Found::Tree(_) => Located::Trouble(format!(
            "the repository at {} is not the one this session started in (another repository stands at that path now, or the project's Git directory was made anew); {NEW_SESSION}",
            hook::shown_path(&project.top)
        )),
        Found::Failed { reason, .. } => Located::Trouble(reason),
        Found::Unpaired(unpaired) => Located::Trouble(unpaired.reason(recorded)),
        Found::NotARepository | Found::AtBoundary { .. } | Found::Gone => {
            Located::Trouble(format!(
                "the project this session began in, {}, is no longer a Git working tree; {NEW_SESSION}",
                hook::shown_path(&project.top)
            ))
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
    match &work.project.base {
        Base::Commit(id) => {
            // A base that can no longer be read is found, and said, when the patch is made.
            let moved = matches!(
                head_contains(&work.place.top, id, &deadline.git()),
                Ok(false)
            );
            Subject::Range {
                base: id.clone(),
                words: format!(
                    "{}; {}",
                    hook::range_words(id, !work.project.by_start, moved),
                    hook::ALREADY_THERE
                ),
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
    nested: Vec<PathBuf>,
}

/// The working tree at `top` as the patch a judge would be given against the commit `base`,
/// which a message calls `called`. A tree that equals the base has no patch: only that is "no
/// change". Every failure is an error with its reason: when one answer stood for "nothing to
/// judge" and for failures alike, a patch that could not be made let a stop pass in silence.
fn change(
    top: &Path,
    base: &str,
    called: &str,
    work: &Path,
    deadline: Deadline,
) -> Result<Change, String> {
    match judge_patch(top, base, work, &deadline.git()) {
        Ok(patch) => Ok(Change {
            patch: Some(patch.bytes).filter(|bytes| !bytes.is_empty()),
            nested: patch.nested,
        }),
        Err(JudgePatchError::NoBase) => Err(format!(
            "{called} ({}) can no longer be read from the repository",
            base.chars().take(7).collect::<String>()
        )),
        Err(JudgePatchError::Failed(error)) => Err(format!(
            "the change could not be read from Git: {}",
            git_words(&error)
        )),
        Err(error @ (JudgePatchError::SkipWorktree | JudgePatchError::Unmerged)) => {
            Err(format!("the change could not be read from Git: {error}"))
        }
    }
}

/// Whether something is said now, once in a session.
enum Said {
    Now,
    /// It was said before.
    Before,
    /// It was not, and the session remembers [`NOTICED_CAP`] other things: this one is not
    /// remembered, so it is not said.
    Full,
}

fn say_once(state: &mut State, what: &str) -> Said {
    let key = deep_diff_forge_core::sha256_hex(what.as_bytes());
    if state.noticed.contains(&key) {
        Said::Before
    } else if state.noticed.len() < NOTICED_CAP {
        state.noticed.insert(key);
        Said::Now
    } else {
        Said::Full
    }
}

/// Whether `what` is said now, once in a session: `false` when it was said before, or when
/// [`NOTICED_CAP`] things were.
fn once(state: &mut State, what: &str) -> bool {
    matches!(say_once(state, what), Said::Now)
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
    let (own, advice) = match locate(input, &mut session, deadline) {
        Located::Nowhere => return None,
        Located::Trouble(reason) => (None, Err(reason)),
        Located::Here(work) => {
            if let Some(why) = session.inside(&work.place.top) {
                return said_beside(Some(&work.place.own), &input.session, &why);
            }
            let advice = advice_for(
                &work,
                &mut session.state,
                deadline.with_project(&work.place),
            );
            (Some(work.place.own), advice)
        }
    };
    let out = match advice {
        Ok(advice) => advice,
        Err(reason) => notice(&mut session.state, &reason),
    };
    match session.keep_advice(&input.session) {
        Ok(true) => out,
        // Another hook of the session holds its state: nothing is written, so nothing is
        // said either. What would have been said is new again after the next tool call.
        Ok(false) => None,
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
    let changed = change(
        &work.place.top,
        &base,
        "the commit this session began from",
        &scratch.sub("work")?,
        deadline,
    )?;
    let Some(patch) = changed.patch else {
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
    keep_said(&mut state.reported, said);
    Ok(hook::post_tool_output(Some(&advice.text)))
}

/// Remember the keys of the findings that were just said. A session keeps [`REPORTED_CAP`]
/// keys: when these would not fit beside the ones it has, it forgets those and keeps these,
/// so an old finding can be said again and the state stays bounded.
fn keep_said(reported: &mut BTreeSet<String>, said: Vec<String>) {
    if reported.len() + said.len() > REPORTED_CAP {
        reported.clear();
    }
    reported.extend(said);
}

/// The gate checks whose detail starts with this were not made, because an earlier link is
/// missing.
const NOT_CHECKED: &str = "not checked: ";

/// At the start of a session: what the hooks will do, what the session's changes are measured
/// from, and whether a stop can be judged here.
fn session_start(input: &Input, args: &[String]) -> Option<String> {
    let deadline = Deadline::after(START_LIMIT);
    // What a start reads of the session and what it records are one step, which no other
    // hook of the session is in the middle of ([`session_lock`]).
    let turn = session_lock(&input.session, START_LOCK_WAIT);
    let mut session = match Session::open(&input.session) {
        Ok(session) => session,
        Err(why) => return Some(hook::contract_trouble(&why)),
    };
    let work = match locate(input, &mut session, deadline) {
        Located::Nowhere => return None,
        Located::Trouble(reason) => return Some(hook::contract_trouble(&reason)),
        Located::Here(work) => work,
    };
    let deadline = deadline.with_project(&work.place);
    if let Some(why) = session.inside(&work.place.top) {
        return Some(hook::contract_trouble(&why));
    }
    // A session that is resumed with its list of commits gone, or its list of judged commits,
    // is not one whose stops will be judged: say so now, not only at the stop (fifth
    // independent review of the hooks, HR5-L-6).
    if !work.fresh
        && let Err(why) = session.tips(&work.project).and_then(|_| session.judged())
    {
        return Some(hook::contract_trouble(&why));
    }
    if work.fresh
        && let Some(project) = &mut session.state.project
    {
        project.by_start = true;
    }
    let unsaved = if matches!(turn, Turn::Taken) {
        // another hook of the session is at work on its state: nothing is written over it
        (work.fresh).then(|| {
            "another hook of this session was running, so this start recorded nothing".to_owned()
        })
    } else {
        session.keep().err()
    };
    // the checks that follow take time and write nothing of the session
    drop(turn);
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
    time: Duration,
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
        time: time.min(JUDGE_TIME),
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
    /// The probe `agent doctor --gate` uses, for this policy ([`Judge::chain_answers`]).
    probe: Option<String>,
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
            probe: probe_patch(&name, &policy),
            candidates: sealed_rankers(args, deadline)?,
            deadline,
        })
    }

    /// Have `patch` judged; `name` is the file it is written to in the scratch directory.
    /// With the answer comes whether the judge was run for it at all: with no time left
    /// ([`run_time`]), no ranker to offer or an input that cannot be written, it was not.
    fn judge(&self, patch: &[u8], name: &str, after: After) -> (Judged, bool) {
        let patch_file = self.scratch.0.join(name);
        if let Err(error) = std::fs::write(&patch_file, patch) {
            return (
                Judged::Unjudged {
                    reason: format!("the judge's input could not be written: {error}"),
                },
                false,
            );
        }
        let patch_sha256 = deep_diff_forge_core::sha256_hex(patch);
        let mut last = Judged::Unjudged {
            reason: "this binary's own path could not be found, so no ranker could be offered"
                .to_owned(),
        };
        let mut ran = false;
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
            let left = self.deadline.left();
            let Some(time) = run_time(left, after) else {
                last = Judged::Unjudged {
                    reason: NO_TIME.to_owned(),
                };
                break;
            };
            ran = true;
            let (code, verdict, said) =
                run_judge(&self.program, &self.policy_file, &patch_file, ranker, time);
            // (exit 7 holds the stop even when its document is no verdict: S3.21)
            last = hook::stop_judged(code, verdict.as_ref(), said.as_deref(), sealed);
            // A run that was stopped says what time it had, and why it was no more: "the
            // judge was stopped" did not name the cause (eighth independent review of the
            // hooks, HR8-L-2). Either the stop had little left, or the run had its part of
            // what was left and the rest is kept for what is judged after it.
            if code.is_none()
                && let Judged::Unjudged { reason } = &mut last
            {
                let had = time.min(JUDGE_TIME).as_secs_f32();
                if time >= JUDGE_TIME {
                    // (it was said to have had "30.0 of the 34.9 seconds …: the rest is
                    // kept for what is judged after it", where three of those were kept
                    // and the rest is this bound: eleventh independent review, HR11-L-6)
                    *reason = format!(
                        "{reason} (this run had {had:.1} seconds, the most one run of the judge has)"
                    );
                } else if time < left {
                    *reason = format!(
                        "{reason} (this run had {had:.1} of the {:.1} seconds that were left of the stop's judging: the rest is kept for what is judged after it)",
                        left.as_secs_f32()
                    );
                } else if time < COMMIT_FLOOR {
                    *reason = format!(
                        "{reason} (the stop had {had:.1} seconds left for this run of the judge: what came before took its time)"
                    );
                } else {
                    // (a run that had all that was left, three seconds or more, was written
                    // with no time: "with the time it had" was not true of it, eleventh
                    // independent review of the hooks, HR11-L-6)
                    *reason = format!(
                        "{reason} (this run had {had:.1} seconds, all that was left of the stop's judging)"
                    );
                }
            }
            // Exit 4: the policy does not admit this ranker. Another may be admitted.
            if code != Some(4) {
                break;
            }
        }
        (last, ran)
    }
}

/// Why a patch has no verdict when the judge was not run for it for lack of time.
const NO_TIME: &str = "the stop had no time left to run the judge: what came before it took the stop's time (Git reading the working trees and the commits, or earlier runs of the judge)";

/// What a run of the judge must leave of the stop's judging for what is judged after it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum After {
    /// A run on a working tree's change: this much is held back, for the tree's other
    /// change, for a second tree and for the asks about commits.
    Trees(Duration),
    /// An ask about a commit, with this many asks owed after it ([`sorted_commits`]):
    /// [`ASK_OWED`] is held back for each.
    Asks(u32),
}

/// The time one run of the judge gets when `left` is left of the stop's judging. This is the
/// one place the rule is, and [`OWED_ASKS_NEED`] the number that follows from it.
///
/// | the run | what it gets | when that is under [`COMMIT_FLOOR`] |
/// | --- | --- | --- |
/// | a working tree's change | `left` less what is held back ([`After::Trees`]) | all that is left: the stop's own change comes before everything |
/// | an ask about a commit | `left` less [`ASK_OWED`] for each ask owed after it | [`COMMIT_FLOOR`], while that much is left: it cannot take the time of the asks owed after it |
///
/// No run gets more than [`JUDGE_TIME`], and none is made with nothing left (`None`).
///
/// History of the rule. The first ask a stop owes got `left` less five seconds and nothing
/// else: with under five seconds left that was no time, the judge was not started, and the
/// commit was written down as asked (eighth independent review of the hooks, HR8-L-1).
/// Then a run that would have had under three seconds got all that was left, and the last
/// ask owed always did: a judge that hangs on the commit in its turn and on the one with
/// fewer attempts took the whole stop between them, at every stop, and a commit that had
/// never been asked about was never submitted (tenth independent review of the hooks,
/// HR10-L-3).
fn run_time(left: Duration, after: After) -> Option<Duration> {
    let time = match after {
        After::Trees(kept) => Some(left.saturating_sub(kept))
            .filter(|time| *time >= COMMIT_FLOOR)
            .unwrap_or(left),
        After::Asks(owed) => left
            .saturating_sub(ASK_OWED.saturating_mul(owed))
            .max(COMMIT_FLOOR)
            .min(left),
    };
    (!time.is_zero()).then_some(time)
}

/// Where the session's directory is at a stop, beside the project's own working tree.
enum Elsewhere {
    /// Nowhere that adds anything: in the project's tree, in no repository, or gone.
    Nothing,
    /// In a linked working tree of the project's repository, with this top level: that tree is
    /// judged as well.
    Linked(Box<Place>),
    /// In another repository, with this top level (a nested repository and a submodule are
    /// repositories of their own): it is not the session's project and is not judged, which is
    /// said.
    Foreign(PathBuf),
    /// Which working tree it is in could not be found out: why, and where it may be.
    Failed(String, Unfound),
}

/// What a stop knows of the project's working trees before it asks Git where the session's
/// directory is.
struct Around<'a> {
    /// The top level of each linked working tree the repository lists, as it is on disk.
    linked: Vec<PathBuf>,
    /// The trees something is remembered of that the repository no longer lists.
    unlisted: &'a [Unlisted],
    /// The top level of each linked tree something is remembered of ([`State::trees`]).
    recorded: &'a std::collections::BTreeMap<String, String>,
}

/// Look at the hook input's `cwd`. The project is judged wherever `cwd` is; this only finds
/// what else there is to judge or to name.
///
/// Which working tree a directory is in is Git's to say, and it is asked. A directory under
/// the Git common directory was taken to be among Git's own files, where no working tree
/// is, and Git was not asked. In a bare repository with its working trees inside it
/// (`repo.git/main`, `repo.git/feature`) every working tree is under the common directory:
/// a stop made from one that is not the session's project did not judge that tree at all,
/// and named it with "the session's directory is not in it" (tenth independent review of
/// the hooks, HR10-M-3).
///
/// And a directory Git cannot place is never taken for one of Git's own files, wherever it
/// lies. That was concluded first from any failure of Git for a directory under the common
/// directory: a tree whose `.git` file named an administrative directory that was gone was
/// "nothing here", and a stop made from inside it, with credentials in it, printed nothing
/// (eleventh independent review of the hooks, HR11-M-1). Then from a directory with no
/// `.git` entry above it: removing a linked tree's `.git` file and its administrative
/// directory, the tree's files left in place, made that true of the very tree the session
/// was in, and the stop from it printed nothing and forgot the failure that had blocked
/// there (review V4 round 20, VD4R20-M-1). Then from where the directory lies, under one of
/// seventeen subtrees Git makes in its directory: `git worktree add` puts a tree at
/// `repo.git/hooks/feature` or `repo.git/svn` without complaint, and with its two records
/// removed that tree was silent and forgotten the same way (review V4 round 21,
/// VD4R21-M-1; thirteenth independent review of the hooks, HR13-L-1). Nothing on disk tells
/// such a tree from Git's own files once its records are gone, so nothing is concluded: a
/// stop made from a directory Git cannot place is UNJUDGED with Git's words, also from
/// inside a Git directory (`.git/hooks`, a bare repository's own directory), and forgets
/// nothing because a tree is no longer listed ([`other_trees`]).
///
/// A directory that is not there any more (the session removed a build directory it was
/// in) is asked about at the nearest directory above it that is there: the tree it was in
/// is the tree the session is in. It "added nothing", so a linked tree was not judged from
/// a directory that was gone, at any stop, and was named as a tree the session was not in
/// (eleventh, HR11-L-5). Where the directory is is found one way, first, for every use of
/// it ([`standing_directory`]).
///
/// What Git answers is believed only as a pair that names one tree ([`pair_of`]): the top
/// level it names and the Git directory it names must be one tree of the repository's own
/// records; where they are not, the stop is UNJUDGED with words that say so, nothing of
/// either tree is forgotten, and neither directory is judged as that tree at this stop
/// ([`Unfound::Unpaired`]). For the project's main Git directory where the repository records
/// no top level of its main tree (one made apart with `--separate-git-dir`), that top is the
/// project's when the project is the main tree, and there is none to pair with when it is a
/// linked one. And the top Git names must contain the directory as well: where it names a
/// working tree that does not (`core.worktree` in a linked tree's own configuration pointing
/// at the project), the stop was silent from inside that tree, and what had blocked there was
/// forgotten (fourteenth independent review of the hooks, HR14-L-4); a pair holding the
/// directory was not enough either (review V4 round 23, VD4R23-M-1; fifteenth, HR15-L-1 and
/// HR15-L-2). Where Git stopped at a file-system boundary that a linked
/// tree of the project lies across (a btrfs subvolume made inside it, a mount), Git has not
/// said the directory is in no repository: the stop is UNJUDGED with Git's words, and that
/// tree "may be in it" (fourteenth, HR14-L-3; it was named as a tree "the session's directory
/// is not in"). A boundary with no linked tree across it is Git's "no repository" as before:
/// on many systems every directory under `/var` is below one.
///
/// And a directory that lies in a tree that is no longer a working tree of this repository,
/// whose memory stands while its directory is there ([`unlisted_trees`]), is UNJUDGED: Git
/// places it in no repository, and the stop from inside it was silent (fourteenth, HR14-L-2).
///
/// And a tree does not move while the directory it is remembered at stands: where Git and the
/// records pair a tree with another top than the one the session's state records for it, and
/// a directory stands there, the stop is UNJUDGED and both directories are named
/// ([`standing_record`]; review V4 round 24, VD4R24-M-1; sixteenth, HR16-L-3).
fn elsewhere(cwd: &str, place: &Place, around: &Around<'_>, deadline: Deadline) -> Elsewhere {
    let cwd = match standing_directory(Path::new(cwd)) {
        Standing::At(cwd) => cwd,
        Standing::Unknown { why, may_be } => {
            return Elsewhere::Failed(
                format!(
                    "whether the session's directory is another working tree of this repository could not be found out: {why}"
                ),
                may_be,
            );
        }
    };
    let placed = if cwd == place.top {
        Elsewhere::Nothing
    } else {
        match find(&cwd, deadline) {
            Found::Unpaired(unpaired) => return unpaired_here(&cwd, &unpaired),
            Found::Tree(found)
                if found.id == place.id
                    && found.top != place.top
                    && pair_of(&found.top, &found.git_dir, &found.common) == Pair::Unrecorded =>
            {
                let unpaired = Unpaired {
                    id: found.id,
                    top: found.top,
                    git_dir: found.git_dir,
                    recorded: (place.git_dir == place.common).then(|| place.top.clone()),
                };
                return unpaired_here(&cwd, &unpaired);
            }
            Found::Tree(found) if !cwd.starts_with(&found.top) => {
                // (from a submodule's Git directory, `.git/modules/<name>`, Git names the
                // submodule's working tree: that is said, for a person to act on)
                let inside = if cwd.starts_with(&place.common) {
                    format!(
                        ", which is inside the Git directory {}",
                        shown(&place.common)
                    )
                } else {
                    String::new()
                };
                let reason = format!(
                    "Git names as the working tree a directory that does not contain the session's directory: whether the session's directory is another working tree could not be found out (Git names {}; the session's directory {}{inside})",
                    shown(&found.top),
                    shown(&cwd)
                );
                return Elsewhere::Failed(reason, Unfound::Under(cwd));
            }
            Found::Tree(found) if found.id != place.id => Elsewhere::Foreign(found.top),
            Found::Tree(found) => {
                if let Some(old) =
                    standing_record(around.recorded, &tree_instance(&found), &found.top)
                {
                    return moved_here(&cwd, &found.top, &old);
                }
                if found.top != place.top {
                    return Elsewhere::Linked(Box::new(found));
                }
                Elsewhere::Nothing
            }
            Found::AtBoundary { words, mount }
                if across(&cwd, mount.as_deref(), &around.linked) =>
            {
                let reason = format!(
                    "{words}. Whether the session's directory is another working tree could not be found out: a working tree of this project lies across that boundary ({})",
                    shown(&cwd)
                );
                return Elsewhere::Failed(reason, Unfound::Under(cwd));
            }
            Found::NotARepository | Found::AtBoundary { .. } | Found::Gone => Elsewhere::Nothing,
            Found::Failed { words, .. } => {
                let reason = unplaced(&cwd, &place.common, &words);
                return Elsewhere::Failed(reason, Unfound::Under(cwd));
            }
        }
    };
    let unlisted = around
        .unlisted
        .iter()
        .filter(|tree| !matches!(tree.is, UnlistedIs::Again(_)))
        .find(|tree| cwd.starts_with(&tree.top));
    if let Some(tree) = unlisted {
        let reason = format!(
            "the session's directory is in a directory that is no longer a working tree of this repository: what is there was not judged, and what blocked there stands until the directory is gone or it is a tree again (the directory {}; the session's directory {})",
            shown(&tree.top),
            shown(&cwd)
        );
        return Elsewhere::Failed(reason, Unfound::Under(cwd));
    }
    placed
}

/// Why a stop made from `cwd`, for which Git named a pair the repository's records do not
/// pair ([`Found::Unpaired`]), is not judged; and the trees that may be the one: every tree
/// `cwd` lies under, the tree at the top level Git named and the tree the records name.
fn unpaired_here(cwd: &Path, unpaired: &Unpaired) -> Elsewhere {
    // The remedy of a tree moved with `mv` (the recorded directory is gone) is said for that
    // case alone: run in a copy, it makes the copy the tree and the directory copied no tree
    // of the project (seventeenth independent review of the hooks, HR17-L-1).
    let moved = unpaired.recorded.as_deref().is_some_and(|recorded| {
        std::fs::symlink_metadata(recorded)
            .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
    });
    let hint = if moved {
        " (`git worktree repair` in a tree moved with `mv` records where it is)"
    } else {
        ""
    };
    let reason = format!(
        "{}{hint}: whether the session's directory is another working tree could not be found out, and neither directory is judged as that tree ({}; the session's directory {})",
        unpaired.words(),
        unpaired.git_dir_words(),
        shown(cwd)
    );
    let tops = std::iter::once(unpaired.top.clone())
        .chain(unpaired.recorded.clone())
        .collect();
    Elsewhere::Failed(
        reason,
        Unfound::Unpaired {
            cwd: cwd.to_path_buf(),
            tops,
        },
    )
}

/// The words that say why the tree Git names at `top` is not read as itself: the session's
/// state records that tree at `old`, which still stands ([`standing_record`]).
fn moved_words(top: &Path, old: &Path) -> String {
    format!(
        "Git names {} as the working tree that this session's memory has at {}, which still stands: neither directory is judged as that tree, and nothing of it is forgotten, until `git worktree repair` is run in the directory that stands or it is removed",
        shown_tree(top),
        shown_tree(old)
    )
}

/// Why a stop made from `cwd`, in the tree Git names at `top`, is not judged: the session
/// remembers a failure of that tree at `old`, which still stands ([`standing_record`]). The
/// trees that may be the one: every tree `cwd` lies under, and the trees at `top` and `old`.
fn moved_here(cwd: &Path, top: &Path, old: &Path) -> Elsewhere {
    let reason = format!(
        "{} (the session's directory {})",
        moved_words(top, old),
        shown(cwd)
    );
    Elsewhere::Failed(
        reason,
        Unfound::Unpaired {
            cwd: cwd.to_path_buf(),
            tops: vec![top.to_path_buf(), old.to_path_buf()],
        },
    )
}

/// Where the session's state records the tree `id` ([`State::trees`]), when that is not `top`
/// and the tree still stands there: a directory whose `.git` file names the tree's own
/// administrative directory, or one of which that cannot be found out. An identity does not
/// move while its remembered directory stands: with two nested linked trees, the inner tree's
/// `core.worktree` set to the outer one and its administrative `gitdir` record edited to name
/// the outer tree's `.git`, Git and the records named the outer tree as the inner one; the
/// clean outer tree was judged under the inner tree's name, what had blocked there was
/// forgotten, and the next stops were silent with two keys in the inner tree (review V4
/// round 24, VD4R24-M-1). The same after `cp -a` of a tree, `git clean` and
/// `git worktree repair` run in the copy (sixteenth independent review of the hooks,
/// HR16-L-3). Where the recorded directory is gone, the tree was moved: it is read at `top`.
///
/// Any directory at the recorded path counted as the tree standing there: after
/// `git worktree move`, a new tree, a clone or a build directory made at the old path left
/// the moved tree UNJUDGED at every stop, and a new key in it never blocked (seventeenth
/// independent review of the hooks, HR17-M-1). A directory with no `.git` file, or one that
/// names another administrative directory, is not the tree.
fn standing_record(
    recorded: &std::collections::BTreeMap<String, String>,
    id: &str,
    top: &Path,
) -> Option<PathBuf> {
    let gone = |error: &std::io::Error| {
        matches!(
            error.kind(),
            std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
        )
    };
    let old = PathBuf::from(recorded.get(id)?);
    if same_form(&old) == same_form(top) {
        return None;
    }
    let dot_git = old.join(".git");
    // the directory, then its `.git` file
    for (path, directory) in [(old.as_path(), true), (dot_git.as_path(), false)] {
        match std::fs::metadata(path) {
            Ok(found) if found.is_dir() != directory => return None,
            Err(error) if gone(&error) => return None,
            Err(_) => return Some(old.clone()),
            Ok(_) => {}
        }
    }
    match pointed_path(&dot_git, b"gitdir: ", &old) {
        Some(admin) => (instance_of(&same_form(&admin)).as_deref() == Some(id)).then_some(old),
        None => Some(old),
    }
}

/// Whether a linked working tree of the project, of those at `linked`, lies across the
/// file-system boundary Git stopped at when it looked from `cwd`: the mount point Git names,
/// the first directory it did not look in, lies at or under the tree's top level. When Git
/// named none that can be read, whether `cwd` lies under such a tree. The project's own tree
/// is judged at every stop, wherever the session's directory is: it is not asked about.
fn across(cwd: &Path, mount: Option<&Path>, linked: &[PathBuf]) -> bool {
    let under = mount.map_or_else(|| cwd.to_path_buf(), resolved);
    linked.iter().any(|top| under.starts_with(top))
}

/// Why a stop made from `cwd`, which Git could not place (`words`), is not judged: Git's
/// words first, then what follows, then the paths. A message shows a reason up to
/// [`hook::REASON_CHARS`] characters, and the path stood before Git's words: with a long
/// one, "…: fatal: this operation must b…" was all a person could read (thirteenth
/// independent review of the hooks, HR13-L-2). Where the directory lies under the Git
/// directory `common` that is said in plain words, for a person to act on; it decides
/// nothing ([`elsewhere`]).
fn unplaced(cwd: &Path, common: &Path, words: &str) -> String {
    let what = "Whether the session's directory is another working tree could not be found out";
    if cwd.starts_with(common) {
        format!(
            "{words}. {what}: it is inside the Git directory, and Git cannot say which working tree it is in (the Git directory {}; the session's directory {})",
            shown(common),
            shown(cwd)
        )
    } else {
        format!(
            "{words}. {what}: Git could not say which repository it is in ({})",
            shown(cwd)
        )
    }
}

/// Where the session's directory is ([`standing_directory`]).
enum Standing {
    /// The nearest directory of its path that is there, as it is on disk.
    At(PathBuf),
    /// It could not be found: why, and where it may be.
    Unknown { why: String, may_be: Unfound },
}

/// Where the session's directory `cwd` is: the one way it is found, for [`elsewhere`] and for
/// the trees it may be in ([`Unfound`]).
///
/// 1. A relative path is anchored at the hook's own directory, where the system reads it
///    too; when that cannot be had, where the directory is is not known.
/// 2. The path, absolute now, is taken as it is on disk; when it is not there, or a part of
///    it names a file, the nearest directory above it that is there.
/// 3. A part that cannot be looked at (a directory that cannot be searched, a loop of
///    links) leaves it unknown, and it may be under the path as written with its links
///    followed as far as they can be ([`through_links`]).
///
/// Before, this was two functions that differed. The first did not walk up from a relative
/// path, and its "nothing of the path is there" was taken as "adds nothing" before the
/// second, which anchored it, was reached: a relative directory that was removed named the
/// linked tree it was under as one "the session's directory is not in", and with the tree's
/// two records removed the stop was silent and forgot what had blocked there (review V4
/// round 22, VD4R22-M-1). And a path that names a file was "gone" (fourteenth independent
/// review of the hooks, suspected).
fn standing_directory(cwd: &Path) -> Standing {
    let absolute = if cwd.is_absolute() {
        cwd.to_path_buf()
    } else {
        match std::env::current_dir() {
            Ok(here) => here.join(cwd),
            Err(error) => {
                return Standing::Unknown {
                    why: format!(
                        "it is a relative path, and the hook's own directory, which it is read from, could not be had ({error})"
                    ),
                    may_be: Unfound::Anywhere,
                };
            }
        }
    };
    let mut at = absolute.as_path();
    loop {
        match std::fs::canonicalize(at) {
            Ok(found) if found.is_dir() => return Standing::At(found),
            // a file: the directory is above it
            Ok(_) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                ) => {}
            Err(error) => {
                return Standing::Unknown {
                    why: format!("it could not be looked at ({error})"),
                    may_be: through_links(&absolute).map_or(Unfound::Anywhere, Unfound::Under),
                };
            }
        }
        match at.parent() {
            Some(above) => at = above,
            None => {
                return Standing::Unknown {
                    why: "nothing of its path is there".to_owned(),
                    may_be: Unfound::Anywhere,
                };
            }
        }
    }
}

/// The time of a stop that is kept for what comes after the judging: remembering what was
/// judged and saying it.
const STOP_RESERVE: Duration = Duration::from_secs(4);
/// The least time that must be left for another commit to be judged at this stop.
const COMMIT_FLOOR: Duration = Duration::from_secs(3);
/// Why commits wait for a later stop when this one ran out of time.
/// For this many attempts that count, a commit with no verdict is asked about again at every
/// stop. After them it is asked about in turn: one such commit a stop ([`sorted_commits`]).
/// It is never given up: review V4 round 14, VD4R14-M-1.
const ATTEMPTS: u32 = 5;
/// When no patch has got a verdict at a stop and this many asks about commits have failed,
/// the stop asks no more.
const FIRST_ASKS: usize = 3;
const NOT_YET: &str = "they were not judged yet: one stop judges as many commits as its time allows, the oldest first; they are judged at the next stop";
/// The most other working trees of the project that one stop looks at for changes.
const OTHER_TREES: usize = 16;

/// What one stop judges with.
struct Stopping<'a> {
    work: &'a Work,
    cwd: &'a str,
    /// The commit the session began from: the policy is read from it.
    base: &'a str,
    scratch: &'a Scratch,
    args: &'a [String],
    /// When the judging must be done: [`STOP_RESERVE`] before the stop's own deadline, and
    /// [`LOOK_TIME`] before that when the project has something a stop only looks at.
    deadline: Deadline,
    /// When that looking must be done: [`STOP_RESERVE`] before the stop's own deadline.
    look: Deadline,
    /// Which stop of the session this is, counted from one: what a commit that is asked about
    /// is marked with, so that the one asked about least recently can be found.
    number: u64,
    /// The top level of each linked tree something is remembered of ([`State::trees`]).
    recorded: &'a std::collections::BTreeMap<String, String>,
}

/// What a stop judged.
#[derive(Default)]
struct Judging {
    /// The uncommitted and the staged change of each working tree first, then the session's
    /// commits that did not pass, the oldest first.
    units: Vec<TreeJudged>,
    /// What must be said beside the verdict, at every stop while it is true.
    beside: Vec<Note>,
    /// The commits this stop has an answer for, each with it as the session's file writes it.
    commits: Vec<(String, String)>,
    /// The working trees this stop looked at for what is not committed in them.
    trees: Vec<Looked>,
    /// Other working trees of the project ([`tree_instance`]) that this stop found with
    /// nothing uncommitted: what failed there is gone.
    clean: Vec<String>,
    /// Every working tree `git worktree list` names now, when it could be read: what is
    /// remembered of a tree that is not among them is forgotten, unless it is among
    /// `unlisted`.
    listed: Option<BTreeSet<String>>,
    /// The trees something is remembered of that the repository no longer lists, whose
    /// directory is not known to be gone ([`unlisted_trees`]).
    unlisted: Vec<Unlisted>,
    /// The top level of each linked tree this stop knows, by which tree it is: what
    /// [`State::trees`] records.
    tops: std::collections::BTreeMap<String, PathBuf>,
}

/// One working tree whose uncommitted and staged changes a stop looked at, with the failing
/// lines it found there ([`hook::failure_lines`]).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct Looked {
    /// Which working tree ([`tree_instance`]).
    id: String,
    /// The lines of its uncommitted change; `None` when that change got no verdict, so
    /// nothing is known of it.
    tree: Option<Lines>,
    /// The lines of what its index holds, every one of them whatever their number
    /// ([`hook::lines_whole`]); `None` when the staged change got no verdict.
    index: Option<Lines>,
    /// Of those, the lines that are the staged change's own, every one of them: the
    /// uncommitted change does not have them (it was repaired and the repair not staged).
    own: Lines,
    /// The staged change's own lines as they are remembered ([`hook::failure_lines`]): the
    /// lines, or one mark for more than [`hook::LINES_KEPT`].
    staged: Lines,
}

/// Failing lines, each with the number of items the judge cites for it.
type Lines = std::collections::BTreeMap<String, usize>;

/// Which working tree an administrative directory of a linked one stands for: sixteen digits
/// of the SHA-256 of what [`instance_words`] makes of that directory's inode and birth time.
///
/// A failure was remembered with the tree's path, and its key held the path once more. So
/// `git worktree move` of an unchanged failing tree made it block a second time, and a tree
/// made anew at the path of a removed one inherited "has already blocked" and never blocked
/// (review V4 round 15, VD4R15-M-2; seventh independent review of the hooks, HR7-H-1). What
/// Git keeps, found with Git 2.56 on btrfs and tmpfs: the directory stays as it is over
/// `git worktree move`, over a plain `mv` with `git worktree repair` (the file `gitdir` is
/// rewritten, in place), over `git gc` and over a `repair` with nothing to repair;
/// `git worktree remove` and `add` at the same path, or the directory deleted, `prune` and
/// `add`, give a new directory, whatever it is called (its name is not the tree: a moved
/// tree keeps the old name, and a new tree at an old path can get it again). Nothing is
/// written into the repository to mark a tree.
fn instance_of(admin: &Path) -> Option<String> {
    use std::os::unix::fs::MetadataExt as _;
    let dir = std::fs::metadata(admin).ok()?;
    let born = dir
        .created()
        .ok()
        .and_then(|at| at.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|since| (since.as_secs(), since.subsec_nanos()));
    let made = || {
        let made = std::fs::metadata(admin.join("commondir")).ok()?;
        Some((made.ino(), made.mtime(), made.mtime_nsec()))
    };
    let words = instance_words(dir.ino(), born, made)?;
    Some(deep_diff_forge_core::sha256_hex(words.as_bytes())[..16].to_owned())
}

/// What a linked working tree is known by: the inode of its administrative directory with
/// the time that directory was made (its birth time, which `statx` gives), or, on a file
/// system that keeps no birth time, with the inode and the modification time of the
/// directory's file `commondir`, which Git writes when it makes the tree and not again
/// (`made` reads them, and is asked only then).
///
/// Each choice, in each direction:
///
/// - The inode alone is not enough: a tree that is removed and made again can be given the
///   inode of the old one, and its new failure must never inherit "has blocked".
/// - The birth time cannot be set by a program that is not the file system: `touch` and a
///   tool that puts times back change the other times. So an unchanged tree stays itself
///   whatever is done to its files' times. The modification time of `commondir` stood here
///   for every file system, and `touch` of that file made an unchanged tree a new one, whose
///   failure blocked again (review V4 round 16, VD4R16-L-1).
/// - Where there is no birth time the modification time is what is left: there a `touch` of
///   `commondir`, or a restore that does not put its time back, makes the tree new and an old
///   failure blocks once more; and where times are whole seconds and inode numbers are
///   reused, a tree removed and made again within one second can be taken for the old one.
///   Found with a birth time on this build's host (Linux 7.2): btrfs and tmpfs. `statx`
///   gives none on ext4 made with 128-byte inodes, on ext2 and ext3, on many network and
///   FUSE file systems, and on a kernel older than 4.11.
/// - The device number is no part of it, as it is no part of a repository's ([`identity`]):
///   it can differ after a reboot, and every administrative directory of one repository is
///   in the one directory `worktrees`, so on one file system.
/// - A copy of the whole project (every inode new) is no case of this: its common directory
///   is not the session's project any more, and a stop there is not judged at all.
fn instance_words(
    dir: u64,
    born: Option<(u64, u32)>,
    made: impl FnOnce() -> Option<(u64, i64, i64)>,
) -> Option<String> {
    if let Some((seconds, nanos)) = born {
        return Some(format!("b:{dir}:{seconds}.{nanos}"));
    }
    let (file, seconds, nanos) = made()?;
    Some(format!("{dir}:{file}:{seconds}.{nanos}"))
}

/// The main working tree of a repository, to the session's memory of failures: there is one,
/// and a repository that is moved or replaced is not the session's project any more.
const MAIN_TREE: &str = "m";

/// Which working tree `place` is: the main one, or the linked one its Git directory stands
/// for ([`instance_of`]). A tree whose Git directory cannot be read is known by its path, as
/// every tree was: that is said nowhere, because Git answers in such a tree only by accident.
fn tree_instance(place: &Place) -> String {
    if place.git_dir == place.common {
        return MAIN_TREE.to_owned();
    }
    instance_of(&place.git_dir).unwrap_or_else(|| by_path(&place.top))
}

/// A working tree known by its path alone, when nothing better can be read.
fn by_path(top: &Path) -> String {
    format!(
        "x{}",
        &deep_diff_forge_core::sha256_hex(top.as_os_str().as_encoded_bytes())[..15]
    )
}

/// The most administrative directories of linked working trees that are read.
const TREES_READ: usize = 1_024;
/// The most bytes of a file of Git's that names one path (`gitdir`, a tree's `.git`).
const POINTER_BYTES: u64 = 16 * 1024;

/// The path a file of Git's holds (`gitdir` in a linked tree's administrative directory, or
/// the `.git` file of a linked tree after `prefix`), as Git reads it: the bytes of its first
/// line, which need not be UTF-8, and a relative path taken from `from`, the directory the
/// file is in. `git worktree add --relative-paths`, `worktree.useRelativePaths` and
/// `git worktree repair --relative-paths` write relative ones (Git 2.48 and later). `None`
/// when there is no such file or it names nothing.
///
/// The content of `gitdir` was read as UTF-8 text and used as it stood, so a relative path
/// was taken from the hook's own directory: a tree made with relative paths was not found
/// among the listed ones, got another identity from outside than from inside, and a stop
/// made from the project forgot what had blocked in it (eighth independent review of the
/// hooks, HR8-M-1).
fn pointed_path(file: &Path, prefix: &[u8], from: &Path) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStrExt as _;
    let mut bytes = Vec::new();
    std::fs::File::open(file)
        .ok()?
        .take(POINTER_BYTES)
        .read_to_end(&mut bytes)
        .ok()?;
    let line = bytes.split(|byte| *byte == b'\n').next()?;
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    let named = line.strip_prefix(prefix)?;
    if named.is_empty() {
        return None;
    }
    let named = Path::new(std::ffi::OsStr::from_bytes(named));
    Some(if named.is_absolute() {
        named.to_path_buf()
    } else {
        from.join(named)
    })
}

/// `path` with its `.` and `..` taken out by name, then with its symbolic links followed as
/// far as it exists ([`resolved`]): what two names of one directory have in common, also
/// when the directory is not there any more. `path` starts at a directory that is real (a
/// Git directory, or a path Git printed), so the steps up are steps among real directories.
fn same_form(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut plain = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                plain.pop();
            }
            other => plain.push(other),
        }
    }
    resolved(&plain)
}

/// The linked working trees of a repository, read from the administrative directories under
/// `worktrees` in its Git common directory: no path of a tree is needed to know which trees
/// there are.
struct Linked {
    /// Each tree's top level as Git records it ([`same_form`]), with which tree it is
    /// ([`instance_of`]).
    trees: Vec<(PathBuf, String)>,
    /// Every administrative directory was read: a tree that is not among `trees` is not one
    /// of this repository any more. `false` when there are more than [`TREES_READ`] or one
    /// could not be read: then nothing is said to be gone.
    whole: bool,
}

/// The linked working trees of the repository whose common directory is `common`.
fn linked_instances(common: &Path) -> Linked {
    let worktrees = std::fs::canonicalize(common.join("worktrees"));
    let entries = match worktrees.as_ref().map(std::fs::read_dir) {
        Ok(Ok(entries)) => entries,
        // no linked tree was ever made
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Linked {
                trees: Vec::new(),
                whole: true,
            };
        }
        _ => {
            return Linked {
                trees: Vec::new(),
                whole: false,
            };
        }
    };
    let mut linked = Linked {
        trees: Vec::new(),
        whole: true,
    };
    for (at, entry) in entries.enumerate() {
        if at >= TREES_READ {
            linked.whole = false;
            break;
        }
        let Ok(entry) = entry else {
            linked.whole = false;
            continue;
        };
        let admin = entry.path();
        if !admin.is_dir() {
            continue;
        }
        let Some(id) = instance_of(&admin) else {
            linked.whole = false;
            continue;
        };
        // `gitdir` names the tree's `.git` file; a tree whose file cannot be read is still
        // a tree of the repository, with no path to find it by
        let top = pointed_path(&admin.join("gitdir"), b"", &admin)
            .and_then(|dot_git| dot_git.parent().map(same_form))
            .unwrap_or_default();
        linked.trees.push((top, id));
    }
    linked
}

/// Which working tree stands at `path`, one of the trees `git worktree list` names: the
/// first of them is the main tree. A linked tree is found by the path its administrative
/// directory records, which is where Git has the listed path from, of those not `taken` by
/// an entry before it (two records that name one path are two trees); and, when a directory
/// stands at `path`, by the administrative directory its own `.git` file names, when that
/// is one of this repository's. One that neither finds is known by its path.
fn listed_instance(
    path: &Path,
    main: bool,
    common: &Path,
    linked: &Linked,
    taken: &BTreeSet<String>,
) -> String {
    if main {
        return MAIN_TREE.to_owned();
    }
    let wanted = same_form(path);
    if let Some((_, id)) = linked
        .trees
        .iter()
        .find(|(top, id)| *top == wanted && !taken.contains(id))
    {
        return id.clone();
    }
    let own = pointed_path(&path.join(".git"), b"gitdir: ", path)
        .map(|admin| same_form(&admin))
        .filter(|admin| {
            admin.parent().is_some_and(|parent| {
                std::fs::canonicalize(common.join("worktrees")).is_ok_and(|all| all == parent)
            })
        })
        .and_then(|admin| instance_of(&admin));
    own.unwrap_or_else(|| by_path(path))
}

/// The tree a remembered line is kept with.
fn tree_of(entry: &str) -> Option<&str> {
    entry.split_once('/').map(|(tree, _)| tree)
}

/// Whether the state remembers a failure that has blocked in the working tree `id`.
fn stands_in(blocked: &[String], id: &str) -> bool {
    blocked.iter().any(|entry| tree_of(entry) == Some(id))
}

/// The tag of a tree's uncommitted (`u`) or staged (`s`) change ([`TreeJudged::tag`]).
fn unit_tag(tree: &str, kind: char) -> String {
    format!("{tree}/{kind}")
}

/// What the state remembers of the unit `tag`, as owned lines.
fn lines_of(blocked: &[String], tag: &str) -> Lines {
    hook::remembered(blocked, tag)
        .into_iter()
        .map(|(key, count)| (key.to_owned(), count))
        .collect()
}

/// How long a stop that is in a linked working tree looks whether the project's own tree
/// has anything that is not committed, before it judges the tree it is in.
const SECOND_TREE_LOOK: Duration = Duration::from_secs(2);
/// What every run of the judge on a working tree's change leaves of the stop's judging for
/// the asks about commits that the stop owes ([`sorted_commits`]): what those asks need
/// ([`OWED_ASKS_NEED`] when there are three of them). `known` is the session's file of
/// judged commits, which says, with no word asked of Git, whether a commit waits for its
/// turn and whether one with fewer attempts waits; a commit that was never asked about
/// cannot be known before the commits are listed, so one ask is always counted for it.
///
/// So a session none of whose commits waits for a verdict keeps [`COMMIT_FLOOR`] back and
/// no more: thirteen seconds were kept back whatever the session's commits were, and from a
/// linked tree a second run of twelve seconds, on the tree's staged change, was cut (tenth
/// independent review of the hooks, HR10-L-2, its second row).
///
/// The file is only added to, and a commit's answer is its last line, as it is for
/// [`sorted_commits`]: every line was read here, so a commit that once had no verdict and
/// has passed since held its five seconds back at every later stop, and one that had been
/// in both lanes ten (eleventh independent review of the hooks, HR11-L-1).
fn asks_kept(known: &[String]) -> Duration {
    let last: std::collections::BTreeMap<&str, Answer<'_>> =
        known.iter().filter_map(|entry| answer(entry)).collect();
    let waiting = |in_turn: bool| {
        last.values().any(|said| {
            matches!(
                said,
                Answer::NoVerdict { attempts, .. } if (*attempts >= ATTEMPTS) == in_turn
            )
        })
    };
    // of the owed asks after the first, the ones no commit waits for
    let none_waits =
        (OWED_ASKS - 1).saturating_sub(u32::from(waiting(true)) + u32::from(waiting(false)));
    OWED_ASKS_NEED.saturating_sub(ASK_OWED.saturating_mul(none_waits))
}

/// A patch that was read, judged: the verdict, or [`Judged::Unjudged`] when the judge gave
/// none. A change that is not there and one that was not read stay what they are. `kept` is
/// what the run leaves of the stop's judging for what is judged after it ([`After::Trees`]).
fn judged_patch(
    tribunal: &mut Judge<'_>,
    patch: Reading<Vec<u8>>,
    kept: Duration,
) -> Reading<Judged> {
    match patch {
        Reading::Nothing => Reading::Nothing,
        Reading::Found(bytes) => {
            tribunal.after = After::Trees(kept);
            Reading::Found(tribunal.judge(&bytes))
        }
        Reading::NotRead(reason) => Reading::NotRead(reason),
    }
}

/// The judge of one stop, looked for when the first patch needs it: a stop with nothing to
/// judge needs no judge and no policy.
struct Judge<'a> {
    stop: &'a Stopping<'a>,
    /// The working trees a change under judgement can write to: no judge is run from them.
    writable: Vec<PathBuf>,
    bench: Option<Result<Bench<'a>, String>>,
    /// What was said about each patch, by its SHA-256: the same patch is judged once.
    answers: std::collections::BTreeMap<String, Judged>,
    /// `Some(true)` once a patch, or the probe, has got a verdict at this stop: the chain works.
    /// `Some(false)` when the probe was asked and got none.
    chain: Option<bool>,
    /// What the next run of the judge must leave for what is judged after it.
    after: After,
    /// Whether the judge was run for the patch [`Judge::judge`] was last asked about, at
    /// this stop (for that patch or for another with the same bytes).
    ran: bool,
}

impl<'a> Judge<'a> {
    fn bench(&mut self) -> &Result<Bench<'a>, String> {
        let stop = self.stop;
        let trees = &self.writable;
        self.bench.get_or_insert_with(|| {
            let place = &stop.work.place;
            let mut paths: Vec<&Path> = trees.iter().map(PathBuf::as_path).collect();
            paths.push(&place.common);
            Bench::new(
                &place.top,
                stop.base,
                stop.scratch,
                stop.args,
                &paths,
                stop.deadline,
            )
        })
    }

    fn judge(&mut self, patch: &[u8]) -> Judged {
        let digest = deep_diff_forge_core::sha256_hex(patch);
        if let Some(known) = self.answers.get(&digest) {
            self.ran = true;
            return known.clone();
        }
        let after = self.after;
        let (answer, ran) = match self.bench() {
            Ok(bench) => bench.judge(patch, "change.patch", after),
            Err(reason) => (
                Judged::Unjudged {
                    reason: reason.clone(),
                },
                false,
            ),
        };
        self.ran = ran;
        if !matches!(answer, Judged::Unjudged { .. }) {
            self.chain = Some(true);
        }
        // (a patch the judge was not run for is asked about again, also at this stop)
        if ran {
            self.answers.insert(digest, answer.clone());
        }
        answer
    }

    /// Whether the chain gives verdicts at this stop: known from the first verdict any patch
    /// got; else the judge is asked, once, about the probe `agent doctor --gate` uses (one
    /// comment line added to the policy). This says only whether asking about more commits
    /// at this stop can lead anywhere. It does not say what a missing verdict means: a commit
    /// with no verdict is not done whatever this answers.
    fn chain_answers(&mut self) -> bool {
        if let Some(known) = self.chain {
            return known;
        }
        let works = match self.bench() {
            Ok(bench) => bench.probe.as_ref().is_some_and(|probe| {
                !matches!(
                    bench
                        .judge(probe.as_bytes(), "probe.patch", After::Asks(0))
                        .0,
                    Judged::Unjudged { .. }
                )
            }),
            Err(_) => false,
        };
        self.chain = Some(works);
        works
    }

    /// Why no patch can be judged at this stop, when the judge could not be prepared.
    fn broken(&self) -> Option<&str> {
        match &self.bench {
            Some(Err(reason)) => Some(reason),
            _ => None,
        }
    }
}

/// Something in the project that a stop knows of and has not judged: said at every stop
/// while it is true. In full the first time; after that under its `name`, on one line with
/// the others. These were said once in a session and then never again: a "said once" that
/// turned into silence, like the commit with no verdict (review V4 round 13 and the fifth
/// independent review of the hooks: the rule is that a stop is silent only when everything
/// has a pass and nothing else is open).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Note {
    /// What it is, in a few words, for the line that counts the notes said before.
    name: String,
    text: String,
    /// In how many working trees this note names or counts a failure stands that has blocked
    /// a stop of this session: the counting line says their number for the notes it does not
    /// name ([`notes_of`]).
    standing: usize,
}

/// What to say of the directory `nested` that was left out of the patch of the tree at `top`.
/// Another working tree of this project that lies inside it is said to be that: it was called
/// "a Git repository of its own" (third independent review of the hooks, HR3-L-3).
fn left_out(stopping: &Stopping<'_>, top: &Path, nested: &Path) -> Option<Note> {
    // A working tree of this repository that lies inside this one (Claude Code's own
    // `.claude/worktrees/`) is a working tree of the project: judged at a stop made from
    // inside it, and otherwise named under the rule for working trees, when it has changes
    // or a failure that stands ([`other_trees`]). It was called "not judged" here at every
    // stop, also at the stop that had judged it, and from the project it was counted twice
    // (seventh independent review of the hooks, HR7-L-5).
    let linked = match find(&top.join(nested), stopping.deadline) {
        Found::Tree(found) => found.id == stopping.work.place.id,
        // What Git names with a Git directory of this repository whose records name another
        // directory is no tree the records name (a copy of a linked tree made inside the
        // project, `cp -a ../feature feature-copy`): it was taken for one, and with a key in
        // it every stop from the project was silent (sixteenth independent review of the
        // hooks, HR16-M-1). It is named as left out. A tree the records do name there, which
        // Git names otherwise, is named by the rule for working trees.
        Found::Unpaired(unpaired) if unpaired.id == stopping.work.place.id => {
            let here = same_form(&top.join(nested));
            if unpaired.recorded.as_deref().map(same_form) == Some(here) {
                return None;
            }
            return Some(Note {
                name: format!(
                    "the directory {} (no working tree by the records)",
                    shown(nested)
                ),
                text: format!(
                    "The directory {} in {} is no working tree of this project by the repository's records ({}) and no part of what this project would commit: it was left out and not judged.",
                    shown(nested),
                    shown_tree(top),
                    unpaired.words()
                ),
                standing: 0,
            });
        }
        _ => false,
    };
    (!linked).then(|| Note {
        name: format!("the nested repository {}", shown(nested)),
        text: format!(
            "The directory {} in {} is a Git repository of its own and no part of what this project would commit: it was left out and not judged.",
            shown(nested),
            shown_tree(top)
        ),
        standing: 0,
    })
}

/// What a stop has of something it reads of a working tree. There are three answers and no
/// fourth: it was read and nothing is there; it was read and this is there; it was not read.
///
/// "Not read" had no value of its own: a tree that could not be read came with "no staged
/// change", which is the answer "the index was read and holds nothing". So a stop that
/// could not read the tree once (a file it may not open, a path marked skip-worktree, a Git
/// that was slow) remembered the index as clean, and the staged failure, untouched and
/// already told, blocked a second time at the next stop (tenth independent review of the
/// hooks, HR10-M-1). It is round 15's rule: a failure is forgotten only by a stop that
/// looked and did not find it. Everything that takes one of these must say what it does
/// with each of the three.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Reading<T> {
    /// It was read, and there is nothing: no change, no failing line.
    Nothing,
    /// It was read, and this is what is there.
    Found(T),
    /// It was not read, and why: nothing is known of it at this stop, so what is remembered
    /// of it stands and nothing of it is forgotten.
    NotRead(String),
}

/// What the index of a working tree holds against its `HEAD`, when it holds a change.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Staged<T> {
    /// The very change the working tree holds: it is judged once, as the tree's.
    Same,
    /// Another change than the tree's: what a plain `git commit` would record.
    Other(T),
}

/// What is not committed in one working tree: what the tree holds, untracked files included,
/// and what its index holds, each against the tree's own `HEAD`, each as a patch.
struct Uncommitted {
    tree: Reading<Vec<u8>>,
    staged: Reading<Staged<Vec<u8>>>,
}

impl Uncommitted {
    /// A working tree that could not be read, and why. Its index was not read either: what
    /// stops the tree from being read stops the index too, and the reason is said once,
    /// with the tree.
    fn not_read(reason: String) -> Self {
        Self {
            staged: Reading::NotRead(format!(
                "it was not looked at, because the working tree's change could not be read ({reason})"
            )),
            tree: Reading::NotRead(reason),
        }
    }
}

/// The uncommitted and the staged change of the working tree at `top`, each against the
/// commit its own `HEAD` names. `Err` is a failure of the hook.
///
/// The tree used to be compared with the session's base, which showed the judge every commit
/// of the session at once, as their net change: a credential that was committed and then
/// reverted was in no patch (review V4 round 11, VD4R11-H-1). Now each commit is judged by
/// itself ([`judge_commits`]) and the tree only for what is not committed. And the index was
/// not looked at: what was staged and then taken out of the working tree again was in no
/// patch, and the next plain `git commit` recorded it (fourth independent review of the
/// hooks, HR4-H-1).
fn uncommitted(
    stopping: &Stopping<'_>,
    top: &Path,
    name: &str,
    beside: &mut Vec<Note>,
) -> Result<Uncommitted, String> {
    uncommitted_by(stopping, top, name, stopping.deadline, beside)
}

/// [`uncommitted`], in the time `deadline` leaves.
fn uncommitted_by(
    stopping: &Stopping<'_>,
    top: &Path,
    name: &str,
    deadline: Deadline,
    beside: &mut Vec<Note>,
) -> Result<Uncommitted, String> {
    let from = match head_state(top, &deadline.git()) {
        Ok(HeadState::Commit(id)) => id,
        // A branch with no commit yet, which the session made: there is no commit of its own
        // to measure from, so the tree is measured from the session's base.
        Ok(HeadState::Unborn { .. }) => stopping.base.to_owned(),
        Err(error) => {
            return Ok(Uncommitted::not_read(format!(
                "the commit HEAD names could not be read: {}",
                git_words(&error)
            )));
        }
    };
    let changed = change(
        top,
        &from,
        "the commit HEAD names",
        &stopping.scratch.sub(name)?,
        deadline,
    );
    let changed = match changed {
        Ok(changed) => changed,
        Err(reason) => return Ok(Uncommitted::not_read(reason)),
    };
    for nested in &changed.nested {
        beside.extend(left_out(stopping, top, nested));
    }
    let index = staged_patch(
        top,
        &from,
        &stopping.scratch.sub(&format!("{name}-index"))?,
        &deadline.git(),
    );
    let staged = match index {
        Ok(bytes) if bytes.is_empty() => Reading::Nothing,
        Ok(bytes) if Some(&bytes) == changed.patch.as_ref() => Reading::Found(Staged::Same),
        Ok(bytes) => Reading::Found(Staged::Other(bytes)),
        Err(JudgePatchError::Failed(error)) => Reading::NotRead(format!(
            "what is staged could not be read from Git: {}",
            git_words(&error)
        )),
        Err(other) => Reading::NotRead(format!("what is staged cannot be judged: {other}")),
    };
    Ok(Uncommitted {
        tree: changed.patch.map_or(Reading::Nothing, Reading::Found),
        staged,
    })
}

/// The commits of the session, the oldest first, each with its first parent: every commit that
/// the project's branches, tags and stash, the `HEAD` of each of its working trees or the
/// reflogs of those `HEAD`s and of the stash lead to now, and that none of `tips` (what was
/// named when the session began) leads to. A commit that only a remote-tracking ref names was
/// fetched and is not the session's; one that was in the repository when the session began is
/// not the session's either, whatever the session then does with it.
fn session_commits(
    stopping: &Stopping<'_>,
    tips: &[String],
) -> Result<Vec<CommitAndParent>, String> {
    let top = &stopping.work.place.top;
    let words = |error: &GitAcquireError| {
        format!(
            "the commits made since the session began could not be listed: {}",
            git_words(error)
        )
    };
    let sources =
        named_commits(top, Named::Own, &stopping.deadline.git()).map_err(|e| words(&e))?;
    commits_beyond(
        top,
        &sources,
        tips,
        &stopping.scratch.0.join("commits.list"),
        &stopping.deadline.git(),
    )
    .map_err(|e| words(&e))
}

/// A number of commits as a message names them.
fn commits_label(count: usize) -> String {
    if count == 1 {
        "1 commit of this session".to_owned()
    } else {
        format!("{count} commits of this session")
    }
}

/// One commit of the session judged by its own change: the patch from its first parent to
/// itself (for a commit with no parent, from nothing). A commit that changes nothing against
/// its first parent has nothing to judge, and passes. With the answer comes whether a
/// missing verdict is one the hook itself proves from the repository, and so final: the
/// patch is over the size the builder makes.
fn judge_commit(
    stopping: &Stopping<'_>,
    tribunal: &mut Judge<'_>,
    at: usize,
    (id, parent): &CommitAndParent,
) -> Result<(Judged, bool), String> {
    let patch = commit_patch(
        &stopping.work.place.top,
        parent.as_deref(),
        id,
        &stopping.scratch.sub(&format!("commit-{at}"))?,
        &stopping.deadline.git(),
    );
    Ok(match patch {
        Ok(bytes) if bytes.is_empty() => (Judged::Pass, false),
        Ok(bytes) => (tribunal.judge(&bytes), false),
        Err(JudgePatchError::Failed(error)) => (
            Judged::Unjudged {
                reason: format!(
                    "the commit could not be read from Git: {}",
                    git_words(&error)
                ),
            },
            matches!(error, GitAcquireError::BudgetExceeded { .. }),
        ),
        Err(other) => (
            Judged::Unjudged {
                reason: format!("the commit could not be read from Git: {other}"),
            },
            false,
        ),
    })
}

/// A commit that has its answer at this stop, as a unit. Its failure holds the stop, whatever
/// has blocked before: a failure that had blocked as an uncommitted change used to stand when
/// it was committed, by a match of rule, signal, path and count that another credential in the
/// same file met as well (fourth independent review of the hooks, HR4-L-4).
fn commit_unit(id: &str, judged: Judged) -> TreeJudged {
    TreeJudged {
        tag: String::new(),
        what: hook::commit_label(id),
        judged: Some(judged),
        holds: Holds::Now,
    }
}

/// A commit as an earlier stop of the session left it: its failure stands, the judge holds
/// it, or it has no verdict. Its rules were said then; their names are kept with its answer. It is a unit of every
/// stop: what is not a pass is said at each one.
fn from_before(id: &str, judged: Judged) -> TreeJudged {
    TreeJudged {
        tag: String::new(),
        what: hook::commit_label(id),
        judged: Some(judged),
        holds: Holds::Never,
    }
}

/// What the session's file of judged commits says of one commit.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Answer<'a> {
    Pass,
    /// Held, with what was kept of the reason ([`kept_reason`]).
    Hold(&'a str),
    /// Failed, with what was kept of the reason.
    Fail(&'a str),
    /// No verdict yet, after so many attempts that count, last asked about at the stop of
    /// that number, with the latest reason. It is asked about again: at every stop for
    /// [`ATTEMPTS`] attempts, then in its turn.
    NoVerdict {
        attempts: u32,
        last: u64,
        reason: &'a str,
    },
    /// No verdict, and none to come: the hook itself proves it from the commit.
    Final(&'a str),
}

/// Read one line of the file of judged commits: `ID:p`, `ID:h:kept`, `ID:f:kept`
/// ([`kept_reason`]; `ID:h` and `ID:f` with nothing kept), `ID:n<attempts>@<stop>:reason`,
/// `ID:x:reason`.
fn answer(entry: &str) -> Option<(&str, Answer<'_>)> {
    let digits = |text: &str| !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit());
    let (id, rest) = entry.split_once(':')?;
    if !is_commit_id(id) {
        return None;
    }
    let said = match rest {
        "p" => Answer::Pass,
        "h" => Answer::Hold(""),
        "f" => Answer::Fail(""),
        _ => {
            let (kind, reason) = rest.split_once(':')?;
            match kind.strip_prefix('n') {
                Some(counts) => {
                    let (attempts, last) = counts.split_once('@')?;
                    Answer::NoVerdict {
                        attempts: attempts.parse().ok().filter(|_| digits(attempts))?,
                        last: last.parse().ok().filter(|_| digits(last))?,
                        reason,
                    }
                }
                None if kind == "x" => Answer::Final(reason),
                None if kind == "h" => Answer::Hold(reason),
                None if kind == "f" => Answer::Fail(reason),
                None => return None,
            }
        }
    };
    Some((id, said))
}

/// The most rules of one commit whose names are kept with its answer, the most characters
/// of each name, and the most of the first thing the judge said beside its rules.
const RULES_KEPT: usize = 8;
const RULE_NAME_KEPT: usize = 64;
const SAID_KEPT_CHARS: usize = 100;

/// What is kept of the reason a commit was held or failed, on its line of the file of judged
/// commits: the names of its rules (those that fail, and after a `;` those the judge holds
/// for; `+N` for the names beyond [`RULES_KEPT`]), and after a `|` the first thing the judge
/// said beside its rules, with the number of the others.
///
/// The rules of a commit were said at the stop that judged it and not kept, so a commit the
/// judge held was "held" with no rule for the rest of the session (sixth independent review
/// of the hooks, HR6-M-2). Then the names were kept and what the judge says beside its rules
/// was not: a commit held for a note, or failed by an expired waiver, kept no reason at all
/// (seventh, HR7-L-6).
///
/// A `:` in a name is kept: the line is read up to its second `:` and the rest is the reason,
/// so only `,`, `;` and `|` separate. A built-in rule of forge-warden, `builtin:<name>`, was
/// named `builtin_<name>` at the later stops, a rule the judge does not have (S3.21.d).
fn kept_reason(rules: &[hook::RuleOutcome], said: &[String], failed: bool) -> String {
    use std::fmt::Write as _;
    let plain = |text: &str, most: usize| -> String {
        text.chars()
            .take(most)
            .map(|c| {
                if c.is_control() || ",;|".contains(c) {
                    '_'
                } else {
                    c
                }
            })
            .collect()
    };
    let names = |held: bool| -> String {
        let all: Vec<String> = rules
            .iter()
            .filter(|rule| !failed || rule.held == held)
            .map(|rule| plain(&rule.id, RULE_NAME_KEPT))
            .filter(|name| !name.is_empty())
            .collect();
        let mut kept: Vec<String> = all.iter().take(RULES_KEPT).cloned().collect();
        if all.len() > RULES_KEPT {
            kept.push(format!("+{}", all.len() - RULES_KEPT));
        }
        kept.join(",")
    };
    let mut kept = if failed {
        match (names(false), names(true)) {
            (failing, held) if held.is_empty() => failing,
            (failing, held) => format!("{failing};{held}"),
        }
    } else {
        names(true)
    };
    if let Some(first) = said.first() {
        let words: String = first
            .chars()
            .take(SAID_KEPT_CHARS)
            .map(|c| match c {
                '|' => '/',
                c if c.is_control() => ' ',
                c => c,
            })
            .collect();
        kept.push('|');
        kept.push_str(words.trim_end());
        if said.len() > 1 {
            let _ = write!(kept, " (+{} more)", said.len() - 1);
        }
    }
    kept
}

/// The rules of a commit judged at an earlier stop, and what the judge said beside them,
/// from what its line keeps ([`kept_reason`]): no explanation and no path, which were said
/// at that stop.
fn reason_kept(kept: &str, all_held: bool) -> (Vec<hook::RuleOutcome>, Vec<String>) {
    let (names, said) = kept.split_once('|').unwrap_or((kept, ""));
    let (failing, held) = names.split_once(';').unwrap_or((names, ""));
    let rules = |names: &str, held: bool| -> Vec<hook::RuleOutcome> {
        names
            .split(',')
            .filter(|name| !name.is_empty())
            .take(RULES_KEPT + 1)
            .map(|name| hook::RuleOutcome {
                id: name.chars().take(RULE_NAME_KEPT).collect(),
                why: String::new(),
                held,
                violations: Vec::new(),
            })
            .collect()
    };
    let said = if said.is_empty() {
        Vec::new()
    } else {
        vec![said.to_owned()]
    };
    ([rules(failing, all_held), rules(held, true)].concat(), said)
}

/// A reason as one line of the file of judged commits holds it.
fn one_line(reason: &str) -> String {
    reason
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(200)
        .collect()
}

/// The commits at the edge of a shallow clone: the repository does not have their parents
/// (`shallow` in the Git common directory lists them). None in a clone that is whole.
///
/// `Err` when the file is there and cannot be read: that is not "a clone that is whole". It
/// was read as that, and a commit at the edge was then judged as if it had added the whole
/// tree.
fn shallow_commits(common: &Path) -> Result<BTreeSet<String>, String> {
    match std::fs::read_to_string(common.join("shallow")) {
        Ok(text) => Ok(text.lines().map(|line| line.trim().to_owned()).collect()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(BTreeSet::new()),
        Err(error) => Err(format!(
            "whether this clone is shallow could not be read ({error}), so a commit with no parent cannot be told from one whose parent the clone does not have"
        )),
    }
}

/// What a stop says of a commit that has had no verdict in more than [`ATTEMPTS`] attempts:
/// what is true of it. It is still asked about, in its turn.
fn in_turn(attempts: u32, reason: &str) -> String {
    format!(
        "no verdict in {attempts} attempts; it is asked about again in its turn, one such commit at each stop; the last reason: {reason}"
    )
}

/// One commit a stop asks the judge about, at `at` among the session's commits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Ask {
    at: usize,
    /// Its attempts that counted; `None` when it was never asked about.
    attempts: Option<u32>,
    /// One of the two asks every stop makes before any other ([`sorted_commits`]).
    reserved: bool,
}

impl Ask {
    /// It has had [`ATTEMPTS`] attempts that counted: it is asked about in its turn.
    fn in_turn(self) -> bool {
        self.attempts.is_some_and(|attempts| attempts >= ATTEMPTS)
    }
}

/// The session's commits by what its file of judged commits says of each: the unit of every
/// commit that has an answer which is not a pass (`None` for the others), and the commits to
/// ask about, in the order of asking.
///
/// **Every commit without a verdict is submitted to the judge at least once in every N
/// stops, N being the number of the session's commits that have no verdict.** That holds
/// because of the order, which no result of an ask can change:
///
/// 1. one commit of those that have had [`ATTEMPTS`] attempts: the one asked about least
///    recently. Each of S such commits is therefore asked about once in S stops. Asking
///    about all of them at every stop would have no bound; asking about none gave them up
///    (review V4 round 14, VD4R14-M-1).
/// 2. one commit of those asked about before that have fewer attempts: again the one asked
///    about least recently, so each of F such commits is asked about once in F stops.
/// 3. the commits never asked about, the oldest first: at least one at every stop.
/// 4. the other commits of 2, the least recently asked first, while the stop goes on.
///
/// 1 and 2 are made before any ask that can end the asking: three asks that fail and a
/// probe that fails end it for the stop ([`judge_commits`]), and such asks stood in front.
/// With a judge that gave no verdict for three commits and its probe, the same three were
/// asked about at every stop, their attempts never counted, and a commit with a credential
/// that waited for its turn was never submitted again (review V4 round 15, VD4R15-M-1).
/// "Least recently" is by the number of the stop each was last asked about at, which every
/// ask writes down whether or not the attempt counts.
///
/// What the property needs ([`OWED_ASKS_NEED`] is the one place the time is stated;
/// `docs/reference/agent-hooks.md` and the tests take it from there): a judge that can be run
/// (with no judge or no policy
/// nothing is submitted for any commit, which is said); a state that can be kept (with
/// none, every stop starts from what was last written, and says so); and
/// [`OWED_ASKS_NEED`] of the judging left when the stop comes to its commits, after the
/// changes of its working trees are judged. With that both asks are made, however long the
/// first one takes ([`run_time`]). With less, and at least [`COMMIT_FLOOR`], the first of
/// them is made (1, or 2 when there is no 1) and the second only if the first left
/// [`COMMIT_FLOOR`]; with less than that neither is. An ask that is not made is not
/// written down: no attempt is counted, the commit keeps its turn, and the stop says that
/// it was not asked for lack of time. A run of the judge before these asks leaves them
/// their time whenever it can itself have [`COMMIT_FLOOR`]; Git taking the stop's time to
/// read the working tree or the commits can take it, and what then has no verdict is said.
fn sorted_commits(
    listed: &[CommitAndParent],
    known: &[String],
) -> (Vec<Option<TreeJudged>>, Vec<Ask>) {
    let known: std::collections::BTreeMap<&str, Answer<'_>> =
        known.iter().filter_map(|entry| answer(entry)).collect();
    let no_verdict = |reason: &str| Judged::Unjudged {
        reason: reason.to_owned(),
    };
    let mut units: Vec<Option<TreeJudged>> = vec![None; listed.len()];
    let mut never: Vec<Ask> = Vec::new();
    // (the stop it was last asked about at, the ask)
    let mut turns: Vec<(u64, Ask)> = Vec::new();
    let mut again: Vec<(u64, Ask)> = Vec::new();
    for (at, (id, _)) in listed.iter().enumerate() {
        match known.get(id.as_str()) {
            Some(Answer::Pass) => {}
            Some(Answer::Fail(kept)) => {
                let (rules, said) = reason_kept(kept, false);
                units[at] = Some(from_before(id, Judged::Fail { rules, said }));
            }
            Some(Answer::Hold(kept)) => {
                let (rules, said) = reason_kept(kept, true);
                units[at] = Some(from_before(id, Judged::Hold { rules, said }));
            }
            Some(Answer::Final(reason)) => units[at] = Some(from_before(id, no_verdict(reason))),
            Some(Answer::NoVerdict {
                attempts,
                last,
                reason,
            }) => {
                let ask = Ask {
                    at,
                    attempts: Some(*attempts),
                    reserved: false,
                };
                let reason = if ask.in_turn() {
                    in_turn(*attempts, reason)
                } else {
                    (*reason).to_owned()
                };
                units[at] = Some(from_before(id, no_verdict(&reason)));
                if ask.in_turn() {
                    turns.push((*last, ask));
                } else {
                    again.push((*last, ask));
                }
            }
            None => never.push(Ask {
                at,
                attempts: None,
                reserved: false,
            }),
        }
    }
    turns.sort_unstable_by_key(|(last, ask)| (*last, ask.at));
    again.sort_unstable_by_key(|(last, ask)| (*last, ask.at));
    let reserved = |(_, ask): &(u64, Ask)| Ask {
        reserved: true,
        ..*ask
    };
    let mut ask: Vec<Ask> = turns.first().map(reserved).into_iter().collect();
    ask.extend(again.first().map(reserved));
    ask.extend(never.first().map(|ask| Ask {
        reserved: true,
        ..*ask
    }));
    ask.extend(never.iter().skip(1));
    ask.extend(again.iter().skip(1).map(|(_, ask)| *ask));
    (units, ask)
}

/// The time of a stop's judging that a run of the judge leaves for a run it owes after it:
/// a working tree's uncommitted change for the tree's staged change, and an ask about a
/// commit for each ask owed after it. It is [`COMMIT_FLOOR`], which an ask is made with,
/// and two seconds more for what lies between two runs (Git making the next patch, a run
/// that is ended at its limit).
const ASK_OWED: Duration = Duration::from_secs(5);
/// The asks about commits that a stop owes at most ([`sorted_commits`]): the commit whose
/// turn it is, the one with fewer attempts that was asked about longest ago, and the oldest
/// commit that was never asked about.
const OWED_ASKS: u32 = 3;
/// What must be left of a stop's judging, when it comes to its commits, for every ask it
/// owes to be made whatever the others do: [`COMMIT_FLOOR`] for the first, and
/// [`ASK_OWED`] for each of the others ([`run_time`]). Every run of the judge on a working
/// tree's change leaves what the asks owed need ([`asks_kept`]).
const OWED_ASKS_NEED: Duration =
    Duration::from_secs(COMMIT_FLOOR.as_secs() + ASK_OWED.as_secs() * (OWED_ASKS as u64 - 1));
/// What is said of a commit that had no verdict before and was not asked about at a stop
/// because the stop's time was used: in front of its reason from before.
const NOT_ASKED: &str = "not asked at this stop for lack of time (it keeps its turn); before:";

/// Say of a commit's unit from before that this stop did not ask about it ([`NOT_ASKED`]).
fn not_asked(unit: &mut Option<TreeJudged>) {
    if let Some(TreeJudged {
        judged: Some(Judged::Unjudged { reason }),
        ..
    }) = unit
        && !reason.starts_with(NOT_ASKED)
    {
        *reason = format!("{NOT_ASKED} {reason}");
    }
}

/// Have the session's commits judged, each by its own change ([`judge_commit`]), and make a
/// unit of every commit that has no pass: at every stop, not only at the one that first
/// finds it.
///
/// - A commit that passed is done. One that failed stands; one the judge held is held: both
///   are units of every later stop.
/// - A commit with no verdict is not done. It was written down as done, said once, and then
///   never asked about and never named again: a transient failure of the judge on a commit
///   with a credential made that commit pass in silence from the next stop on (review V4
///   round 13, VD4R13-M-1; fifth independent review of the hooks, HR5-H-1). It is asked about
///   again at later stops and is a unit of every stop while it has no verdict.
/// - The order of asking is [`sorted_commits`]'s, with what it holds: every commit without a
///   verdict is submitted at least once in as many stops as there are such commits.
/// - An attempt is counted, and a turn used, for a commit the judge was run for at this
///   stop, and only for such a one. It is written down with what the judge said, also when
///   the run was the last thing the stop had time for or the stop's end cut it short:
///   "not asked" is said only of a commit the judge was not run for. One that the stop's
///   time did not reach has nothing written down: it keeps its attempts and its turn, and
///   is said not to have been asked ([`NOT_ASKED`]). It was written down as asked, with an attempt counted,
///   when the judge had not been started for it (eighth independent review of the hooks,
///   HR8-L-1). One the judge could not be run for although there was time (the commit
///   cannot be read from Git, no ranker can be offered) is written down as asked at this
///   stop, so that the others have their turn before it comes again, and no attempt is
///   counted.
/// - Of the commits the judge was run for, an attempt counts at a stop where some patch got
///   a verdict. Where none did, nothing can be said of the patch: the commit is written
///   down as asked at this stop, so that others come before it next time, and its attempts
///   stay as they were. (An ask of a commit in its turn always counts, when the judge was
///   run for it.) When [`FIRST_ASKS`] asks of such a stop have failed, the judge
///   is asked about its probe ([`Judge::chain_answers`]): with no verdict on that either,
///   the stop asks no more and the rest are counted.
/// - Only what the hook proves from the commit itself is final at once: a patch over the
///   builder's size (a commit does not change, so neither does that). It is a unit of every
///   stop too. A commit at the edge of a shallow clone is no such thing: the repository can
///   get its parent, so it is looked at again at every stop and nothing is written down.
/// - Nothing of a stop is written before the stop ends: one that is killed in the middle of
///   an ask leaves every commit where it was, the turn of the one it was asking about too.
#[expect(
    clippy::too_many_lines,
    reason = "one loop over the asks: each way an ask can end is an arm of it"
)]
fn judge_commits(
    stopping: &Stopping<'_>,
    tribunal: &mut Judge<'_>,
    listed: &[CommitAndParent],
    known: &[String],
    out: &mut Judging,
) -> Result<(), String> {
    let shallow = shallow_commits(&stopping.work.place.common);
    let no_shallow = BTreeSet::new();
    let no_verdict = |reason: &str| Judged::Unjudged {
        reason: reason.to_owned(),
    };
    let (mut units, asks) = sorted_commits(listed, known);
    let mut owed = u32::try_from(asks.iter().filter(|ask| ask.reserved).count()).unwrap_or(0);
    // Why the commits from here on wait, once one has to.
    let mut waits: Option<String> = tribunal.broken().map(str::to_owned);
    let (mut waiting, mut failed) = (0_usize, 0_usize);
    // the stop's time is what makes the commits from here on wait
    let mut out_of_time = false;
    // (the commit, its attempts, whether they are final whatever the chain did, the reason)
    let mut tried: Vec<(usize, u32, bool, String)> = Vec::new();
    for ask in asks {
        let at = ask.at;
        let commit = &listed[at];
        let id = &commit.0;
        owed = owed.saturating_sub(u32::from(ask.reserved));
        tribunal.after = After::Asks(owed);
        if waits.is_none() && stopping.deadline.left() < COMMIT_FLOOR {
            waits = Some(NOT_YET.to_owned());
            out_of_time = true;
        }
        if waits.is_some() {
            // one that had no verdict before is a unit already, with its reason
            waiting += usize::from(ask.attempts.is_none());
            if out_of_time {
                not_asked(&mut units[at]);
            }
            continue;
        }
        // The repository does not have this commit's parent, so "from its parent" would be
        // "from nothing": the whole tree as an addition (HR4-L-2). That is true for as long
        // as the clone is shallow there, and no longer: it is looked at again at every stop
        // and nothing is written down, so after `git fetch --unshallow`, which the reason
        // names, the commit is asked about like any other. It was written down as final,
        // and stayed "shallow" for the rest of the session (sixth independent review of the
        // hooks, HR6-L-2).
        // (not known whether the clone is shallow: a commit with no parent has no verdict,
        // and is looked at again at every stop)
        if let (None, Err(why)) = (&commit.1, &shallow) {
            units[at] = Some(from_before(id, no_verdict(why)));
            continue;
        }
        if commit.1.is_none() && shallow.as_ref().unwrap_or(&no_shallow).contains(id) {
            units[at] = Some(from_before(
                id,
                no_verdict(
                    "this clone is shallow and does not have the commit's parent, so the commit's own change cannot be read (`git fetch --unshallow` gets the history, and the commit is judged then)",
                ),
            ));
            continue;
        }
        tribunal.ran = false;
        let (judged, proven) = judge_commit(stopping, tribunal, at, commit)?;
        let ran = tribunal.ran;
        if let Some(reason) = tribunal.broken() {
            waits = Some(reason.to_owned());
            waiting += usize::from(ask.attempts.is_none());
            continue;
        }
        let said = match &judged {
            Judged::Pass => "p".to_owned(),
            Judged::Hold { rules, said } => format!("h:{}", kept_reason(rules, said, false)),
            Judged::Fail { rules, said } => format!("f:{}", kept_reason(rules, said, true)),
            Judged::Unjudged { reason } if proven => format!("x:{}", one_line(reason)),
            Judged::Unjudged { .. } if !ran && stopping.deadline.left() < COMMIT_FLOOR => {
                // The stop's time ended the asking before the judge was run for this
                // commit: not an attempt, and no turn is used. An ask the judge WAS run
                // for is written down below whatever time is left after it: with about
                // three seconds left the judge was run, answered "no verdict", and the stop
                // wrote nothing and said "not asked"; a session whose stops all came to
                // their commits like that asked about the same commit every time (ninth
                // independent review of the hooks, HR9-L-2).
                waits = Some(NOT_YET.to_owned());
                out_of_time = true;
                waiting += usize::from(ask.attempts.is_none());
                not_asked(&mut units[at]);
                continue;
            }
            Judged::Unjudged { reason } if !ran => {
                // The judge was not run for this commit, and not for lack of time: it is
                // written down as asked at this stop, with the attempts it had.
                failed += 1;
                let attempts = ask.attempts.unwrap_or(0);
                tried.push((at, attempts, true, reason.clone()));
                units[at] = Some(commit_unit(
                    id,
                    if ask.in_turn() {
                        no_verdict(&in_turn(attempts, reason))
                    } else {
                        judged
                    },
                ));
                continue;
            }
            Judged::Unjudged { reason } if ask.in_turn() => {
                // asked in its turn: the ask counts, whatever the chain did, and the commit
                // is said as what it is
                let attempts = ask.attempts.unwrap_or(ATTEMPTS).saturating_add(1);
                tried.push((at, attempts, true, reason.clone()));
                units[at] = Some(commit_unit(id, no_verdict(&in_turn(attempts, reason))));
                continue;
            }
            Judged::Unjudged { reason } => {
                failed += 1;
                tried.push((at, ask.attempts.unwrap_or(0), false, reason.clone()));
                if tribunal.chain != Some(true) && failed >= FIRST_ASKS && !tribunal.chain_answers()
                {
                    waits = Some(reason.clone());
                }
                units[at] = Some(commit_unit(id, judged));
                continue;
            }
        };
        units[at] = (said != "p").then(|| commit_unit(id, judged));
        out.commits.push((id.clone(), said));
    }
    tribunal.after = After::Asks(0);
    // An attempt counts where the chain gave a verdict at this stop. Where an ask failed and
    // no patch got a verdict, the judge is asked about its probe to find that out: with one
    // or two commits that get no verdict and nothing else to judge, the probe was never
    // asked (it was asked after three failed asks), no attempt ever counted, and every stop
    // of the session asked again, at the judge's own time-out each (sixth independent review
    // of the hooks, HR6-L-3).
    if failed > 0
        && tribunal.chain.is_none()
        && waits.is_none()
        && stopping.deadline.left() >= COMMIT_FLOOR
    {
        tribunal.chain_answers();
    }
    let counts = u32::from(tribunal.chain == Some(true));
    for (at, attempts, counted, reason) in tried {
        let attempts = if counted { attempts } else { attempts + counts };
        out.commits.push((
            listed[at].0.clone(),
            format!("n{attempts}@{}:{}", stopping.number, one_line(&reason)),
        ));
    }
    out.units.extend(units.into_iter().flatten());
    if waiting > 0 {
        out.units.push(TreeJudged {
            tag: String::new(),
            what: commits_label(waiting),
            judged: Some(Judged::Unjudged {
                reason: waits.unwrap_or_else(|| NOT_YET.to_owned()),
            }),
            holds: Holds::Now,
        });
    }
    Ok(())
}

/// The time of a stop that is kept for looking at what it does not judge and only names: the
/// project's submodules and its other working trees. It comes after everything that is
/// judged, and what is not looked at in it is counted and said.
const LOOK_TIME: Duration = Duration::from_secs(6);
/// The least time one working tree's `git status` is started with.
const LOOK_SHARE: Duration = Duration::from_millis(200);

/// When a stop's judging must be done, and when its looking at what it only names: the
/// second is [`STOP_RESERVE`] before the stop's own deadline, the first [`LOOK_TIME`] before
/// that when the project has something to look at.
fn stop_deadlines(place: &Place, deadline: Deadline) -> (Deadline, Deadline) {
    let look = deadline.sooner_by(STOP_RESERVE);
    let judging = if has_more_to_look_at(place) {
        look.sooner_by(LOOK_TIME)
    } else {
        look
    };
    (judging, look)
}

/// The time one working tree's `git status` may take when `left` is left for looking and
/// `trees` are still to be looked at, this one included: half of what is left, or an even
/// share when that is more. With even shares alone, sixteen trees in which `git status`
/// takes longer than a sixteenth of the time were all cut off, and none was looked at; with
/// halves the first trees, the ones where a failure stands, have a real chance, and one
/// tree that hangs still leaves half for the others.
fn look_share(left: Duration, trees: usize) -> Duration {
    (left / 2).max(left / u32::try_from(trees.max(1)).unwrap_or(u32::MAX))
}

/// Whether the project has anything a stop only looks at ([`LOOK_TIME`]): a linked working
/// tree, or submodules.
fn has_more_to_look_at(place: &Place) -> bool {
    place.top.join(".gitmodules").exists()
        || std::fs::read_dir(place.common.join("worktrees"))
            .is_ok_and(|mut entries| entries.next().is_some())
}

/// A number of working trees, as a note names them.
fn trees_counted(count: usize, more: bool) -> String {
    match (count, more) {
        (1, true) => "1 more working tree".to_owned(),
        (1, false) => "1 working tree".to_owned(),
        (_, true) => format!("{count} more working trees"),
        (_, false) => format!("{count} working trees"),
    }
}

/// What a note that counts working trees says of the failures that stand in them: in its
/// name, and as a sentence.
fn standing_counted(standing: usize) -> (String, String) {
    match standing {
        0 => (String::new(), String::new()),
        1 => (
            " (a failure stands in 1 of them)".to_owned(),
            " A failure that has blocked a stop of this session stands in 1 of them: it was not looked at at this stop.".to_owned(),
        ),
        _ => (
            format!(" (a failure stands in {standing} of them)"),
            format!(
                " Failures that have blocked a stop of this session stand in {standing} of them: they were not looked at at this stop."
            ),
        ),
    }
}

/// What to say when the project's working trees could not be listed: nothing can be named,
/// and what stands in the trees this stop does not judge is still said, by its number.
/// Nothing when the project has no other tree and nothing stands elsewhere.
fn not_listed(
    linked: &Linked,
    own: &BTreeSet<String>,
    blocked: &[String],
    error: &GitAcquireError,
) -> Vec<Note> {
    let elsewhere: BTreeSet<&str> = blocked
        .iter()
        .filter_map(|entry| tree_of(entry))
        .filter(|tree| !own.contains(*tree))
        .collect();
    if linked.trees.is_empty() && elsewhere.is_empty() {
        return Vec::new();
    }
    let (named, sentence) = standing_counted(elsewhere.len());
    vec![Note {
        name: format!("the other working trees, which could not be listed{named}"),
        text: format!(
            "The other working trees of this project could not be listed ({}): they were not looked at for changes that are not committed.{sentence}",
            git_words(error)
        ),
        standing: elsewhere.len(),
    }]
}

/// The notes that count the working trees a stop did not look at: `late`, for lack of time,
/// and `beyond`, past the [`OTHER_TREES`] it looks at; each as (how many trees, in how many
/// of them a failure stands).
fn not_looked_at(late: (usize, usize), beyond: (usize, usize)) -> Vec<Note> {
    let mut said = Vec::new();
    if late.0 > 0 {
        let trees = trees_counted(late.0, false);
        let (named, sentence) = standing_counted(late.1);
        let (was, it) = if late.0 == 1 {
            ("was", "it was")
        } else {
            ("were", "they were")
        };
        said.push(Note {
            name: format!("{trees} that {was} not looked at for lack of time{named}"),
            text: format!(
                "A stop looks at the project's other working trees for at most {} seconds, after everything it judges: {trees} of this project {was} not reached in that time, so {it} not looked at for changes that are not committed.{sentence}",
                LOOK_TIME.as_secs()
            ),
            standing: late.1,
        });
    }
    if beyond.0 > 0 {
        let trees = trees_counted(beyond.0, true);
        let (named, sentence) = standing_counted(beyond.1);
        said.push(Note {
            name: format!(
                "{trees} that {} not looked at{named}",
                if beyond.0 == 1 { "was" } else { "were" }
            ),
            text: format!(
                "This project has {trees} than the {OTHER_TREES} a stop looks at: {} not looked at for changes that are not committed.{sentence}",
                if beyond.0 == 1 { "it was" } else { "they were" }
            ),
            standing: beyond.1,
        });
    }
    said
}

/// What the note of a working tree that is gone advises. Not `git worktree prune` where a
/// failure stands in the tree: it forgets what blocked there, also in a tree moved with `mv`
/// (seventeenth independent review of the hooks, HR17-L-2).
fn gone_advice(stands: bool) -> &'static str {
    if stands {
        "if it was moved with `mv`, `git worktree repair` run where it is now records it; `git worktree prune` would forget what blocked there"
    } else {
        "`git worktree prune` forgets a linked tree that is gone; Git lists a main Git directory made apart under another name than `.git` as the main tree, and that entry stays"
    }
}

/// What to say of the project's other working trees: one that is gone, and one the session
/// is not in that has changes which are not committed (they are in no patch of this stop; the
/// commits made there are judged). An agent that makes a working tree and edits it by path
/// is never "in" it (fourth independent review of the hooks, HR4-L-5). `own` are the trees
/// this stop has units of ([`tree_instance`]), `judged` those of them it reads and judges.
///
/// This is the last thing a stop does before it remembers, and it has a time of its own,
/// `end`. The trees were looked at before the session's own change was judged, each `git
/// status` with all the time the stop had left: sixteen trees in which that takes 2.5
/// seconds made the project's own change UNJUDGED at every stop, 41 seconds each (eighth
/// independent review of the hooks, HR8-L-2). Now:
///
/// - the trees where a failure stands that has blocked are looked at first and named first;
/// - [`OTHER_TREES`] are looked at, each with a share of the time that is left
///   ([`look_share`]);
/// - a tree beyond those, and one whose turn came with no time left or whose `git status`
///   did not end in its share, was not looked at: what is remembered of it stays, and the
///   stop counts those trees and says in how many of them a failure stands. A failure in a
///   tree beyond the sixteen was remembered and not said (eighth, HR8-L-3).
///
/// Which trees the repository has is read from its own directory ([`linked_instances`]),
/// not from the paths of the listing: what is remembered of a tree is forgotten only when no
/// administrative directory is that tree any more, and only at a stop that knows which tree
/// the session is in (`unfound` is `None`).
fn other_trees(
    stopping: &Stopping<'_>,
    (judged, unfound): (&BTreeSet<String>, Option<&Unfound>),
    (linked, unlisted): (&Linked, &[Unlisted]),
    own: &BTreeSet<String>,
    blocked: &[String],
    end: Deadline,
    out: &mut Judging,
) -> Vec<Note> {
    let place = &stopping.work.place;
    let mut said = no_longer_trees(unlisted);
    // A stop that could not find out which tree the session is in forgets nothing because a
    // tree is not listed: the session's directory may be a tree whose records are gone. Its
    // `.git` file and its administrative directory removed, a linked tree was listed no
    // more, and what had blocked in it was forgotten at the stop made from inside it (review
    // V4 round 20, VD4R20-M-1). A stop that can place the session's directory forgets it.
    if linked.whole && unfound.is_none() {
        out.listed = Some(
            std::iter::once(MAIN_TREE.to_owned())
                .chain(linked.trees.iter().map(|(_, id)| id.clone()))
                .collect(),
        );
    }
    let trees = match worktrees(&place.top, &end.git()) {
        Ok(trees) => trees,
        Err(error) => {
            said.extend(not_listed(linked, own, blocked, &error));
            return said;
        }
    };
    let others = listed_others(&trees, judged, (&place.common, linked, unlisted), blocked);
    let mut to_look = others
        .iter()
        .filter(|(tree, _, _, _)| tree.usable)
        .count()
        .min(OTHER_TREES);
    // (how many trees, in how many of them a failure stands)
    let (mut looked_at, mut beyond, mut late) = (0_usize, (0_usize, 0_usize), (0_usize, 0_usize));
    for (tree, id, stands, olds) in others {
        // That a failure stands is part of the tree's name here, so it is on the line that
        // counts these things at every stop: in the full text alone it was said once
        // (seventh, HR7-L-3).
        let name = |what: &str| {
            format!(
                "the working tree {}{what}{}",
                shown_tree(&tree.path),
                if stands {
                    " (a failure stands there)"
                } else {
                    ""
                }
            )
        };
        let standing = if stands {
            " A failure there has blocked a stop of this session and stands: it was not looked at at this stop."
        } else {
            ""
        };
        if !tree.usable {
            said.push(Note {
                name: name(" (gone)"),
                text: format!(
                    "The working tree {} of this project is gone or is no working tree any more: what only its own history names is not judged ({}).{standing}",
                    shown_tree(&tree.path),
                    gone_advice(stands)
                ),
                standing: usize::from(stands),
            });
            continue;
        }
        // The session's directory is under this tree, or where it is is not known, and
        // which tree it is in could not be found out at this stop (that is said, with the
        // reason, as the stop's own unit). It is not a tree "the session's directory is not
        // in": that was said of the very tree the session was in when Git failed for it
        // (eleventh independent review of the hooks, HR11-M-1). Nothing of it is forgotten.
        if let Some(unfound) = unfound.filter(|unfound| unfound.may_be_in(&tree.path)) {
            let text = format!("{}{standing}", unfound.words(&tree.path));
            said.push(Note {
                name: name(" (the session's directory may be in it)"),
                text,
                standing: usize::from(stands),
            });
            continue;
        }
        // The tree is remembered at another directory, which stands: neither is read as it.
        if let Some(old) = standing_record(stopping.recorded, &id, &tree.path) {
            said.push(Note {
                name: name(" (remembered at another directory, which stands)"),
                text: format!(
                    "{}. What is not committed in either was not judged at this stop.{standing}",
                    moved_words(&tree.path, &old)
                ),
                standing: usize::from(stands),
            });
            continue;
        }
        looked_at += 1;
        if looked_at > OTHER_TREES {
            beyond = (beyond.0 + 1, beyond.1 + usize::from(stands));
            continue;
        }
        let share = look_share(end.left(), to_look);
        to_look = to_look.saturating_sub(1);
        if share < LOOK_SHARE {
            late = (late.0 + 1, late.1 + usize::from(stands));
            continue;
        }
        match other_tree(stopping, (tree, &id), stands, looked_at, end.within(share)) {
            OtherTree::Nothing => {
                out.clean.push(id);
                out.clean.extend(olds);
            }
            OtherTree::Late => late = (late.0 + 1, late.1 + usize::from(stands)),
            OtherTree::Named(what, text) => said.push(Note {
                name: name(what),
                text: format!("{text}{standing}"),
                standing: usize::from(stands),
            }),
        }
    }
    said.extend(not_looked_at(late, beyond));
    said
}

/// The listed working trees of the project a stop does not judge, those where a failure
/// stands that has blocked first: each with which tree it is ([`listed_instance`]), whether
/// such a failure stands there, and the trees no longer listed whose directory it is now
/// ([`UnlistedIs::Again`]).
///
/// A tree is left out as judged by which tree it is (`judged`), never by its path: with a
/// tree's administrative `gitdir` record edited to name another tree's `.git`, Git lists the
/// other tree's path twice, both entries were left out at a stop made from that tree, and
/// the failure that stood in the first tree was neither looked at nor said (sixteenth
/// independent review of the hooks, HR16-L-1). Each entry is the tree of an administrative
/// directory no entry before it is ([`listed_instance`]).
fn listed_others<'t>(
    trees: &'t [Worktree],
    judged: &BTreeSet<String>,
    (common, linked, unlisted): (&Path, &Linked, &[Unlisted]),
    blocked: &[String],
) -> Vec<(&'t Worktree, String, bool, Vec<String>)> {
    let mut taken = BTreeSet::new();
    let mut others: Vec<(&Worktree, String, bool, Vec<String>)> = trees
        .iter()
        .enumerate()
        // a bare repository's own entry is no working tree: there is nothing uncommitted in it
        .filter(|(_, tree)| !tree.bare)
        .filter_map(|(at, tree)| {
            let id = listed_instance(&tree.path, at == 0, common, linked, &taken);
            taken.insert(id.clone());
            if judged.contains(&id) {
                return None;
            }
            let olds: Vec<String> = unlisted
                .iter()
                .filter(|old| matches!(&old.is, UnlistedIs::Again(now) if *now == id))
                .map(|old| old.id.clone())
                .collect();
            // A failure that blocked a stop made from inside this tree is still the
            // session's: it is not looked at from here, and it is said (sixth independent
            // review of the hooks, HR6-M-1: it was forgotten at such a stop, and blocked
            // again on return). So is one that blocked in a tree that stood at this path
            // before and is no longer listed.
            let stands = stands_in(blocked, &id) || olds.iter().any(|old| stands_in(blocked, old));
            Some((tree, id, stands, olds))
        })
        .collect();
    others.sort_by_key(|(_, _, stands, _)| !stands);
    others
}

/// What to say of the trees something is remembered of that the repository no longer lists
/// and whose directory is not known to be gone ([`unlisted_trees`]): each, at every stop,
/// with what blocked there standing. A tree whose directory is a listed tree again is said
/// with that tree ([`other_trees`]).
fn no_longer_trees(unlisted: &[Unlisted]) -> Vec<Note> {
    unlisted
        .iter()
        .filter_map(|tree| {
            let (what, unknown) = match &tree.is {
                UnlistedIs::Stands => ("", String::new()),
                UnlistedIs::Unknown(error) => (
                    "; it could not be looked at",
                    format!(", and whether its directory is still there could not be found out ({error})"),
                ),
                UnlistedIs::Again(_) => return None,
            };
            Some(Note {
                name: format!(
                    "the directory {} (no longer a working tree{what}; a failure stands there)",
                    shown_tree(&tree.top)
                ),
                text: format!(
                    "The directory {} is no longer a working tree of this repository{unknown}; what blocked there stands until the directory is gone or it is a tree again (a stop made from inside it is UNJUDGED).",
                    shown_tree(&tree.top)
                ),
                standing: 1,
            })
        })
        .collect()
}

/// What a stop finds out about a working tree it is not in ([`other_tree`]).
enum OtherTree {
    /// Nothing there is uncommitted: what is remembered of the tree is forgotten.
    Nothing,
    /// It was not looked at in its share of the time.
    Late,
    /// It is named: what goes with its name, and the sentence.
    Named(&'static str, String),
}

/// Look at the working tree `tree`, which is the tree `id` ([`listed_instance`]) and which the
/// stop is not in, for changes that are not committed; `n` names the scratch directories of a
/// reading. `stands`: something of the tree is remembered.
///
/// What is read of it counts as its own only when Git names that tree there
/// ([`names_the_tree`]), at every stop and for every listed tree: that was asked only where a
/// failure stood and the reading found nothing, so a tree Git is pointed away from was read
/// through the redirect wherever nothing stood, and once forgotten was never named again
/// (fifteenth independent review of the hooks, HR15-L-1). One `git rev-parse` more for each
/// tree looked at: at most [`OTHER_TREES`] a stop, each in the tree's share of the time.
///
/// `git status` is a look, not the reading a stop judges by: it does not show a change in
/// a file marked assume-unchanged, nor one that the repository's own configuration hides
/// from it (a clean filter that takes a key out, line endings), and the judge's patch has
/// them. Its "nothing" was taken for "nothing there": a failure that had blocked in such a
/// tree was forgotten at a stop made from another tree, which printed nothing, and blocked
/// a second time on return (eleventh independent review of the hooks, HR11-L-2). Where
/// something is remembered of the tree, only the reader the judge's patch is made by says
/// that nothing is there ([`other_tree_read`]); where nothing is remembered there is
/// nothing to forget, and the look decides only whether the tree is named.
fn other_tree(
    stopping: &Stopping<'_>,
    (tree, id): (&Worktree, &str),
    stands: bool,
    n: usize,
    within: Deadline,
) -> OtherTree {
    let not_judged = "they were not judged (a stop made from inside it judges them; its commits are judged from anywhere).";
    let unknown = |reason: &str| {
        OtherTree::Named(
            " (it could not be looked at)",
            format!(
                "Whether the working tree {} of this project has changes that are not committed could not be found out ({reason}): such changes are not judged.",
                shown_tree(&tree.path)
            ),
        )
    };
    match names_the_tree(&stopping.work.place, &tree.path, id, within) {
        Ok(()) => {}
        Err(None) => return OtherTree::Late,
        Err(Some(reason)) => return unknown(&reason),
    }
    match has_uncommitted_changes(&tree.path, &within.git()) {
        Ok(false) if !stands => OtherTree::Nothing,
        Ok(false) => match other_tree_read(stopping, &tree.path, n, within) {
            Reading::Nothing => OtherTree::Nothing,
            Reading::NotRead(reason) => unknown(&reason),
            Reading::Found(()) => OtherTree::Named(
                "",
                format!(
                    "The working tree {} of this project has changes that are not committed (`git status` there shows none of them; the patch a stop judges by has them), and the session's directory is not in it: {not_judged}",
                    shown_tree(&tree.path)
                ),
            ),
        },
        Ok(true) => OtherTree::Named(
            "",
            format!(
                "The working tree {} of this project has changes that are not committed, and the session's directory is not in it: {not_judged}",
                shown_tree(&tree.path)
            ),
        ),
        Err(GitAcquireError::TimedOut { .. }) => OtherTree::Late,
        Err(error) => unknown(&git_words(&error)),
    }
}

/// Whether Git, asked at `path`, names the tree `id` of the project at `place` there: that
/// directory as the top level, with a Git directory the repository's records pair with it
/// ([`placed`]) that is the tree `id` ([`tree_instance`]). Otherwise what is read at `path`
/// is read of another directory or under another tree's name, and "nothing there" is not
/// this tree's: a tree whose own configuration points Git elsewhere (`core.worktree`) was
/// forgotten at a stop made from the project (fourteenth independent review of the hooks,
/// HR14-L-4; fifteenth, HR15-L-1; review V4 round 23, VD4R23-M-1). `Err(None)` when Git did
/// not answer in the time it had; `Err(Some(why))` when it answered otherwise.
fn names_the_tree(
    place: &Place,
    path: &Path,
    id: &str,
    within: Deadline,
) -> Result<(), Option<String>> {
    let dirs = match repository_dirs(path, &within.git()) {
        Ok(dirs) => dirs,
        Err(GitAcquireError::TimedOut { .. }) => return Err(None),
        Err(error) => return Err(Some(git_words(&error))),
    };
    match placed(path, dirs) {
        Found::Tree(found)
            if found.top == resolved(path)
                && found.id == place.id
                && tree_instance(&found) == id =>
        {
            Ok(())
        }
        Found::Tree(found) if found.top == resolved(path) => Err(Some(format!(
            "Git names {} as the working tree there with the Git directory {}, which is not this tree's",
            shown_tree(&found.top),
            shown(&found.git_dir)
        ))),
        Found::Tree(found) => Err(Some(format!(
            "Git names {} as the working tree there, which is not this tree",
            shown_tree(&found.top)
        ))),
        Found::Unpaired(unpaired) => Err(Some(format!(
            "{}; {}",
            unpaired.words(),
            unpaired.git_dir_words()
        ))),
        Found::Failed { words, .. } => Err(Some(words)),
        Found::NotARepository | Found::AtBoundary { .. } | Found::Gone => {
            Err(Some("Git names no working tree there".to_owned()))
        }
    }
}

/// Whether the working tree at `top`, which the stop is not in, has anything that is not
/// committed, by the reader a stop judges with ([`uncommitted`]): the tree's patch and the
/// index's, each against the tree's `HEAD`. `n` names its scratch directories. No patch is
/// judged here: this only says whether what is remembered of the tree may be forgotten.
fn other_tree_read(
    stopping: &Stopping<'_>,
    top: &Path,
    n: usize,
    deadline: Deadline,
) -> Reading<()> {
    let name = format!("other-{n}");
    let read = match uncommitted_by(stopping, top, &name, deadline, &mut Vec::new()) {
        Ok(read) => read,
        Err(reason) => return Reading::NotRead(reason),
    };
    match (read.tree, read.staged) {
        (Reading::Nothing, Reading::Nothing) => Reading::Nothing,
        (Reading::NotRead(reason), _) | (_, Reading::NotRead(reason)) => Reading::NotRead(reason),
        _ => Reading::Found(()),
    }
}

/// What the runs of the tree at `at` of the `count` a stop judges leave for the project's
/// tree, which is judged after it: [`COMMIT_FLOOR`] when a tree is judged after this one and
/// the project's tree is not known to be `clean`, else nothing. `clean` is asked only when a
/// tree comes after this one (one `git status`).
fn held_for_the_project(at: usize, count: usize, clean: impl FnOnce() -> bool) -> Duration {
    if at + 1 < count && !clean() {
        COMMIT_FLOOR
    } else {
        Duration::ZERO
    }
}

/// Why a staged change has no verdict of its own at a stop ([`tree_units`]).
const STAGED_NOT_APART: &str = "what is staged fails, and which of that is the staged change's own cannot be told at this stop: the uncommitted change got no verdict, and its failure is remembered by a digest alone";

/// Where the session's directory is, at a stop that could not find out which working tree it
/// is in ([`Elsewhere::Failed`]). A tree it may be in is named "the session's directory may be
/// in it", never "is not in it", and nothing of that tree is forgotten ([`other_trees`]).
enum Unfound {
    /// It is this directory, or under it: a tree this lies under may be the one.
    Under(PathBuf),
    /// It is `cwd`, for which Git named a top level and a Git directory the repository's
    /// records do not pair ([`Found::Unpaired`]): a tree `cwd` lies under may be the one, and
    /// so may the trees at `tops` (the one Git named, the one the records name), of which
    /// nothing is forgotten either.
    Unpaired { cwd: PathBuf, tops: Vec<PathBuf> },
    /// Where it is is not known: any tree may be the one.
    Anywhere,
}

impl Unfound {
    /// What a stop says of the working tree at `top`, which the session's directory may be in.
    fn words(&self, top: &Path) -> String {
        match self {
            Self::Unpaired { cwd, .. } if !cwd.starts_with(top) => format!(
                "Git named the working tree {} of this project with another directory for the session's directory, so whether the session's directory is in that tree could not be found out at this stop: what is not committed there was not judged.",
                shown_tree(top)
            ),
            Self::Under(_) | Self::Unpaired { .. } => format!(
                "The session's directory is under the working tree {} of this project, and whether it is in that tree could not be found out at this stop: what is not committed there was not judged.",
                shown_tree(top)
            ),
            Self::Anywhere => format!(
                "Where the session's directory is could not be found out at this stop, so it may be in the working tree {} of this project: what is not committed there was not judged.",
                shown_tree(top)
            ),
        }
    }

    /// Whether the session's directory may be in the working tree at `top`.
    fn may_be_in(&self, top: &Path) -> bool {
        match self {
            Self::Under(cwd) => cwd.starts_with(top),
            Self::Unpaired { cwd, tops } => {
                cwd.starts_with(top) || tops.iter().any(|named| same_form(named) == same_form(top))
            }
            Self::Anywhere => true,
        }
    }
}

/// A working tree something is remembered of that the repository no longer lists, whose
/// directory is recorded ([`State::trees`]) and not known to be gone.
struct Unlisted {
    /// Which tree it was ([`tree_instance`]).
    id: String,
    /// Its top level, as recorded.
    top: PathBuf,
    is: UnlistedIs,
}

/// What stands at the top level of a tree the repository no longer lists ([`Unlisted`]).
enum UnlistedIs {
    /// Its directory: what blocked there stands, and the tree is named at every stop.
    Stands,
    /// Whether its directory is there could not be found out, for this reason: what blocked
    /// there stands.
    Unknown(String),
    /// A working tree the repository lists, which is this other tree ([`tree_instance`]): what
    /// blocked in the old one is forgotten when the judge's own reading finds the new one
    /// clean.
    Again(String),
}

/// The trees something in `blocked` is remembered of that `linked`, read whole, does not
/// list, and whose directory `recorded` has and is not gone. A tree the repository no longer
/// lists was forgotten at the next stop that could place the session's directory, also while
/// its directory, and what had blocked there, was still in place (fourteenth independent
/// review of the hooks, HR14-L-2). Now it is forgotten when its directory is gone (`git
/// worktree remove`, or the directory deleted and `git worktree prune`), when its directory
/// is a tree again and that tree is found clean, or when no directory of it is recorded.
/// With the administrative directories not read whole nothing is said to be unlisted.
fn unlisted_trees(
    blocked: &[String],
    recorded: &std::collections::BTreeMap<String, String>,
    linked: &Linked,
) -> Vec<Unlisted> {
    if !linked.whole {
        return Vec::new();
    }
    let remembered: BTreeSet<&str> = blocked.iter().filter_map(|entry| tree_of(entry)).collect();
    remembered
        .into_iter()
        .filter(|id| !linked.trees.iter().any(|(_, listed)| listed == id))
        .filter_map(|id| {
            let top = PathBuf::from(recorded.get(id)?);
            let is = if let Some((_, now)) = linked.trees.iter().find(|(listed, _)| *listed == top)
            {
                UnlistedIs::Again(now.clone())
            } else {
                match std::fs::metadata(&top) {
                    Ok(found) if found.is_dir() => UnlistedIs::Stands,
                    Ok(_) => return None,
                    Err(error)
                        if matches!(
                            error.kind(),
                            std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                        ) =>
                    {
                        return None;
                    }
                    Err(error) => UnlistedIs::Unknown(error.to_string()),
                }
            };
            Some(Unlisted {
                id: id.to_owned(),
                top,
                is,
            })
        })
        .collect()
}

/// The absolute `path` with its `.` and `..` taken out the way the system takes them out, and
/// its links followed as far as it exists ([`resolved`]). A `..` steps up from where the part
/// before it leads, its links followed: taken out by name, `alias/../linked` was `linked`
/// beside `alias`, while the system goes up from where `alias` points (thirteenth independent
/// review of the hooks; a tree the session's directory was in was named "the session's
/// directory is not in it", and another tree "may be in it"). `None` when a part before a
/// `..` cannot be resolved: whether it is a link is not known.
fn through_links(path: &Path) -> Option<PathBuf> {
    use std::path::Component;
    let mut plain = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                plain = std::fs::canonicalize(&plain).ok()?;
                plain.pop();
            }
            other => plain.push(other),
        }
    }
    Some(resolved(&plain))
}

/// A working tree a stop judges: its top level, which tree it is, the name of its scratch
/// directory, and why it cannot be read when it cannot.
type TreeToJudge = (PathBuf, String, &'static str, Option<String>);

/// The working trees a stop judges, the one the stop is in first; and the session's
/// directory, when the tree it is in could not be found out (`Some` then, and only then).
/// A linked tree the stop is in is added to `writable`; another repository the stop is in
/// is named in `beside`.
fn trees_to_judge(
    stopping: &Stopping<'_>,
    around: &Around<'_>,
    writable: &mut Vec<PathBuf>,
    beside: &mut Vec<Note>,
) -> (Vec<TreeToJudge>, Option<Unfound>) {
    let place = &stopping.work.place;
    // The working trees this stop judges, the one the stop is in first: (its top level,
    // which tree it is, the name of its scratch directory, why it cannot be read).
    //
    // The project's tree stood first whatever the stop was in. A session that began in the
    // main tree and whose directory is a linked tree had the main tree read and judged
    // first, and a slow run there left the stop's own tree about five seconds: UNJUDGED at
    // every such stop (review V4 round 17, VD4R17-L-1). A tree is read when its turn comes,
    // so that a slow Git in the other tree does not come before the first one's verdict.
    let project = (
        place.top.clone(),
        tree_instance(place),
        "work",
        None::<String>,
    );
    // The session's directory, when the tree it is in could not be found out.
    let mut unfound = None::<Unfound>;
    let trees = match elsewhere(stopping.cwd, place, around, stopping.deadline) {
        Elsewhere::Nothing => vec![project],
        Elsewhere::Linked(linked) => {
            writable.push(linked.top.clone());
            vec![
                (linked.top.clone(), tree_instance(&linked), "work-2", None),
                project,
            ]
        }
        Elsewhere::Foreign(top) => {
            beside.push(Note {
                name: format!("the repository {} the session is in", shown_tree(&top)),
                text: format!(
                    "The session's directory is in another repository, {}, which is not this session's project ({}): nothing in it was judged.",
                    shown_tree(&top),
                    shown_tree(&place.top)
                ),
                standing: 0,
            });
            vec![project]
        }
        Elsewhere::Failed(reason, may_be) => {
            unfound = Some(may_be);
            vec![
                (
                    PathBuf::from(stopping.cwd),
                    by_path(Path::new(stopping.cwd)),
                    "",
                    Some(reason),
                ),
                project,
            ]
        }
    };
    (trees, unfound)
}

/// Have the uncommitted and the staged change of one working tree judged, and add them to
/// `out`: its units, and what this stop saw of it ([`Looked`]). `after` is what its runs
/// leave of the stop's judging for what is judged after the tree (a working tree that is
/// judged after it, and the asks about commits: [`asks_kept`]); the first run also leaves
/// [`ASK_OWED`] for the second when the tree has a staged change of its own.
fn tree_units(
    tribunal: &mut Judge<'_>,
    top: &Path,
    id: &str,
    found: Uncommitted,
    blocked: &[String],
    after: Duration,
    out: &mut Judging,
) {
    let second = if matches!(found.staged, Reading::Found(Staged::Other(_))) {
        ASK_OWED
    } else {
        Duration::ZERO
    };
    let tree = judged_patch(tribunal, found.tree, after + second);
    let index = match found.staged {
        Reading::Nothing => Reading::Nothing,
        Reading::Found(Staged::Same) => Reading::Found(Staged::Same),
        Reading::Found(Staged::Other(bytes)) => {
            match judged_patch(tribunal, Reading::Found(bytes), after) {
                Reading::Found(judged) => Reading::Found(Staged::Other(judged)),
                Reading::Nothing => Reading::Nothing,
                Reading::NotRead(reason) => Reading::NotRead(reason),
            }
        }
        Reading::NotRead(reason) => Reading::NotRead(reason),
    };
    let (looked, units) = seen(id, &top.to_string_lossy(), tree, index, blocked);
    out.trees.push(looked);
    out.units.extend(units);
}

/// What a stop saw of the working tree `id` at `top`, from what it has of its uncommitted
/// change and of what its index holds ([`Reading`]; a change that was read and judged with no
/// verdict is [`Judged::Unjudged`]): what is remembered of it ([`Looked`]), and its units.
///
/// - Nothing there: the unit has nothing to judge, and what was remembered of it is
///   forgotten (the stop looked).
/// - A verdict: the unit is judged; what is remembered becomes what was seen.
/// - No verdict, or not read: nothing is known. The unit is said to be UNJUDGED with the
///   reason, and what was remembered of it stands. A tree that was not read says its reason
///   once: its index, which was not read for the same reason, is a unit of its own only
///   when something of it has blocked before, and then says that it stands.
fn seen(
    id: &str,
    top: &str,
    tree: Reading<Judged>,
    index: Reading<Staged<Judged>>,
    blocked: &[String],
) -> (Looked, Vec<TreeJudged>) {
    // the tree's verdict when it has one: `None` when nothing is known of the tree's change
    let tree_read = !matches!(tree, Reading::NotRead(_));
    let tree = match tree {
        Reading::Nothing => None,
        Reading::Found(judged) => Some(judged),
        Reading::NotRead(reason) => Some(Judged::Unjudged { reason }),
    };
    let unknown = |judged: &Option<Judged>| matches!(judged, Some(Judged::Unjudged { .. }));
    let lines = |judged: &Option<Judged>| -> Lines {
        judged.as_ref().map(hook::failure_lines).unwrap_or_default()
    };
    let whole = |judged: &Option<Judged>| -> Lines {
        judged.as_ref().map(hook::lines_whole).unwrap_or_default()
    };
    // What the index holds that the tree's failure does not have is the staged change's
    // own failure: the tree was repaired and the repair not staged, and a plain commit
    // would still record it. A line the tree's failure has too is the tree's, and stands
    // or blocks with it: as a failure of its own it made one that stood block again
    // after `git add -A` and one more edit of any file (sixth independent review of the
    // hooks, HR6-M-1). With no verdict on the tree's change, what is remembered of it
    // stands in for its lines.
    //
    // Which of the index's lines the tree's failure has too is asked of the two verdicts
    // of this stop, line by line, whatever their number. It was asked of the tree's
    // failure as it is remembered, which past 256 lines is one mark, and a mark covers no
    // single line: staging a part of an unchanged failure of 257 lines made every staged
    // line the staged change's own, and the stop blocked again, at each further partial
    // `git add` once more (ninth independent review of the hooks, HR9-M-1).
    let tree_lines = (!unknown(&tree)).then(|| lines(&tree));
    let tree_whole = (!unknown(&tree)).then(|| whole(&tree));
    let cover = tree_whole
        .clone()
        .unwrap_or_else(|| lines_of(blocked, &unit_tag(id, 'u')));
    let staged_tag = unit_tag(id, 's');
    let told = hook::remembered(blocked, &staged_tag);
    // The index: its verdict (`None`: it holds no change of its own), every line it holds
    // (`None`: not known at this stop), and why it was not read, when it was not.
    let (index, index_whole, not_read) = match index {
        Reading::Nothing => (None, Some(Lines::new()), None),
        Reading::Found(Staged::Same) => (None, tree_whole.clone(), None),
        Reading::Found(Staged::Other(judged)) => {
            let known =
                (!matches!(judged, Judged::Unjudged { .. })).then(|| hook::lines_whole(&judged));
            (Some(judged), known, None)
        }
        Reading::NotRead(reason) => (None, None, Some(reason)),
    };
    // Every line the index holds has blocked for the staged change: a mark that is
    // remembered for it is exactly what the index holds now. Then nothing of what is
    // staged is new, whatever part of it is the staged change's own at this stop. The
    // mark was asked only whether the staged change's own failure is exactly it, so a
    // staged failure of more than 256 lines that the tree had again in part and that was
    // repaired there again in part blocked with the index untouched (tenth independent
    // review of the hooks, HR10-M-2).
    let all_told = index_whole.as_ref().is_some_and(|index| {
        !index.is_empty() && {
            let (mark, count) = hook::mark_of(index);
            told.get(mark.as_str()) == Some(&count)
        }
    });
    // The one case where the staged change's own lines cannot be told from the tree's: the
    // tree's change has no verdict and its failure is remembered by its mark alone
    // ([`hook::mark_of`]), and what the index holds is not exactly that failure. Then it
    // is said to have no verdict here: called its own, lines that have blocked as the
    // tree's would block a second time.
    let apart = |own: &BTreeSet<String>| {
        tree_lines.is_some() || own.is_empty() || !cover.keys().any(|key| hook::is_mark(key))
    };
    let mut told_apart = true;
    let staged = match index {
        Some(failed @ Judged::Fail { .. }) => {
            let own = hook::lines_beyond(&failed, &cover);
            if apart(&own) {
                hook::failure_part(&failed, &own, false)
            } else {
                told_apart = false;
                Some(Judged::Unjudged {
                    reason: STAGED_NOT_APART.to_owned(),
                })
            }
        }
        // An index that was not read is said with the tree that was not read, once; by
        // itself only when the tree was read, or when something of it has blocked before
        // and stands.
        None => not_read
            .filter(|_| tree_read || !told.is_empty())
            .map(|reason| Judged::Unjudged {
                reason: if told.is_empty() {
                    reason
                } else {
                    format!("{reason}; what has blocked of it before stands")
                },
            }),
        other => other,
    };
    let looked = Looked {
        id: id.to_owned(),
        tree: tree_lines,
        index: index_whole.filter(|_| told_apart),
        own: whole(&staged),
        staged: lines(&staged),
    };
    let mut units = vec![TreeJudged {
        tag: unit_tag(id, 'u'),
        what: hook::tree_label(top),
        judged: tree,
        holds: Holds::WhileNew,
    }];
    if let Some(staged) = staged {
        units.push(TreeJudged {
            tag: staged_tag,
            what: hook::staged_label(top),
            // (all of it has blocked before: it stands, like a commit from before)
            holds: if all_told && matches!(staged, Judged::Fail { .. }) {
                Holds::Never
            } else {
                Holds::WhileNew
            },
            judged: Some(staged),
        });
    }
    (looked, units)
}

/// Have the session's work judged, unit by unit: the uncommitted and the staged change of the
/// project's working tree, and of a linked working tree of the same repository that the
/// session is in; then every commit of the session that has no answer yet
/// ([`judge_commits`]). All are judged by the policy at the session's base. A judge is looked
/// for only when something has a patch for it.
///
/// The order of a stop is by what matters most, under its one deadline:
///
/// 1. the working tree the stop is in is read (Git), and its staged and uncommitted changes
///    are judged; when that is a linked working tree, the project's own tree is read and
///    judged after it;
/// 2. the two asks about commits that every stop owes ([`sorted_commits`]);
/// 3. the other commits, while [`COMMIT_FLOOR`] is left of the judging;
/// 4. the submodules and the other working trees are looked at, in [`LOOK_TIME`] of their
///    own, which 1 to 3 cannot use when the project has any ([`has_more_to_look_at`]).
///
/// So the longest a stop takes is its one deadline ([`STOP_LIMIT`]), as before; what it no
/// longer does is spend that on other working trees before its own change has a verdict.
///
/// With the units comes what must be said beside the verdict, each thing once in a session:
/// a nested repository that was left out, another repository the session is in, a submodule
/// with changes that are not committed in it, another working tree with changes.
fn judge_units(
    stopping: &Stopping<'_>,
    tips: &Tips,
    known: &[String],
    blocked: &[String],
) -> Result<Judging, String> {
    let place = &stopping.work.place;
    let mut out = Judging::default();
    let mut writable = vec![place.top.clone()];
    let linked = linked_instances(&place.common);
    let unlisted = unlisted_trees(blocked, stopping.recorded, &linked);
    let listed_tops = linked
        .trees
        .iter()
        .filter(|(top, _)| !top.as_os_str().is_empty());
    // (a tree's recorded top does not move while the directory there stands)
    out.tops = listed_tops
        .clone()
        .map(|(top, id)| {
            let kept = standing_record(stopping.recorded, id, top);
            (id.clone(), kept.unwrap_or_else(|| top.clone()))
        })
        .collect();
    let around = Around {
        linked: listed_tops.map(|(top, _)| top.clone()).collect(),
        unlisted: &unlisted,
        recorded: stopping.recorded,
    };
    let (trees, unfound) = trees_to_judge(stopping, &around, &mut writable, &mut out.beside);
    for (top, id, _, unreadable) in &trees {
        if unreadable.is_none() && *top != place.top {
            out.tops.insert(id.clone(), top.clone());
        }
    }
    let own: BTreeSet<String> = trees.iter().map(|(_, id, _, _)| id.clone()).collect();
    // the trees this stop reads and judges, by which tree each is
    let judged: BTreeSet<String> = trees
        .iter()
        .filter(|(_, _, _, unreadable)| unreadable.is_none())
        .map(|(_, id, _, _)| id.clone())
        .collect();
    let mut tribunal = Judge {
        stop: stopping,
        writable,
        bench: None,
        answers: std::collections::BTreeMap::new(),
        chain: None,
        after: After::Asks(0),
        ran: false,
    };
    let (count, asks) = (trees.len(), asks_kept(known));
    for (at, (top, id, scratch, unreadable)) in trees.into_iter().enumerate() {
        let found = match unreadable {
            Some(reason) => Uncommitted::not_read(reason),
            None => uncommitted(stopping, &top, scratch, &mut out.beside)?,
        };
        // What this tree's runs leave for the project's tree, which is judged after it:
        // [`COMMIT_FLOOR`] when that tree has something that is not committed, and
        // nothing when it has not. Two runs' time was held back for it before it was known
        // whether it had anything, so from a linked tree the stop's own change had 15 of
        // its 35 seconds, and with a judge that took 17 it was UNJUDGED with 20 seconds
        // unused (tenth independent review of the hooks, HR10-L-2). One `git status`
        // answers, under a bound of its own; what cannot be found out counts as a change.
        let later = held_for_the_project(at, count, || {
            matches!(
                has_uncommitted_changes(
                    &place.top,
                    &stopping.deadline.within(SECOND_TREE_LOOK).git()
                ),
                Ok(false)
            )
        });
        let after = later + asks;
        tree_units(&mut tribunal, &top, &id, found, blocked, after, &mut out);
    }
    // The session's commits: the ones something names now that none of the commits that
    // were there when it began leads to. With no list of those, none can be told from the
    // others, and that is said at every stop; what is not committed was judged above all
    // the same.
    let listed = match tips {
        Tips::Listed(tips) => session_commits(stopping, tips),
        // What follows from it stands first, and the whole has room in what a message shows
        // of a reason ([`hook::REASON_CHARS`]): the cause stood first, and every stop of
        // such a session read "… cannot be told from them: they are not." (eleventh
        // independent review of the hooks, HR11-L-3).
        Tips::Unavailable(why) => Err(format!(
            "not in this session, whatever is committed (what is not committed is judged): the commits that were there when it began could not be listed ({})",
            deep_diff_forge_core::display_safe(why)
        )),
    };
    match listed {
        Ok(listed) => judge_commits(stopping, &mut tribunal, &listed, known, &mut out)?,
        Err(reason) => out.units.push(TreeJudged {
            tag: String::new(),
            what: "The commits of this session".to_owned(),
            judged: Some(Judged::Unjudged { reason }),
            holds: Holds::Now,
        }),
    }
    // Last, in a time of its own: what is only looked at and named.
    let end = stopping.look.within(LOOK_TIME);
    out.beside
        .extend(submodules_with_changes(place, end.within(LOOK_TIME / 2)));
    let others = other_trees(
        stopping,
        (&judged, unfound.as_ref()),
        (&linked, &unlisted),
        &own,
        blocked,
        end,
        &mut out,
    );
    out.beside.extend(others);
    out.unlisted = unlisted;
    Ok(out)
}

/// What a stop's memory of failures could not keep as it was ([`within_bound`]).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct Kept {
    /// The units (`TREE/u`, `TREE/s`) whose memory is gone: no mark of them fits.
    lost: Vec<String>,
    /// The units this stop looked at that are remembered by their mark from now on.
    short: Vec<String>,
    /// The staged changes (`TREE/s`) of which a mark had to be given up: what had blocked of
    /// them was remembered as a whole, and the index no longer holds exactly that, so which
    /// of the lines still staged have blocked is not known any more.
    given_up: Vec<String>,
    /// The staged changes (`TREE/s`) that are remembered as a whole from now on, by a mark
    /// that is not known to be everything the index holds: unchanged they stand, and after
    /// a change of the working tree they may block once more with the index untouched.
    as_a_whole: Vec<String>,
}

/// What is remembered of one unit: its own lines, the lines kept for it (a staged change's,
/// `k`), and the number of entries that takes. Either is lines or one mark.
#[derive(Debug, Default)]
struct Unit {
    own: Lines,
    kept: Lines,
    entries: usize,
}

impl Unit {
    /// The unit in its short form: what its own lines become (one entry, or none when no
    /// line is its own), what the lines kept for it become, and whether those had to be
    /// given up.
    ///
    /// The kept lines become one mark of everything that has blocked for this staged change,
    /// its own lines and the kept ones together: when the working tree is repaired again
    /// and all of it is the staged change's own once more, the failure is exactly what that
    /// mark stands for, and stands. They were let go with no word, and the staged failure,
    /// untouched in the index, then blocked a second time (ninth independent review of the
    /// hooks, HR9-L-1). They can only be given up where the own lines are a mark already
    /// (more than 256 of them): then "all of it" has no lines to make a mark of.
    fn short(&self) -> Short {
        let one = |lines: &Lines| {
            lines
                .iter()
                .next()
                .map(|(key, count)| (key.clone(), *count))
        };
        let marked = |lines: &Lines| lines.keys().any(|key| hook::is_mark(key));
        let own = match self.own.len() {
            0 => None,
            1 => one(&self.own),
            _ => Some(hook::mark_of(&self.own)),
        };
        let (kept, gone) = if self.kept.is_empty() {
            (None, false)
        } else if marked(&self.kept) {
            (one(&self.kept), false)
        } else if marked(&self.own) {
            (None, true)
        } else {
            let all: Lines = self
                .own
                .iter()
                .chain(&self.kept)
                .map(|(key, count)| (key.clone(), *count))
                .collect();
            (Some(hook::mark_of(&all)), false)
        };
        Short { own, kept, gone }
    }

    /// How many entries the short form has.
    fn short_size(&self) -> usize {
        let short = self.short();
        usize::from(short.own.is_some()) + usize::from(short.kept.is_some())
    }
}

/// A unit in its short form ([`Unit::short`]): the entry its own lines become, the entry
/// the lines kept for it become (each a line or a mark with its count), and whether the
/// kept lines had to be given up.
struct Short {
    own: Option<(String, usize)>,
    kept: Option<(String, usize)>,
    gone: bool,
}

/// The unit a remembered entry belongs to, as its tag: the tree with `u` or `s`. The lines
/// kept for a staged change (`k`) are of that staged change. And whether the entry is the
/// unit's own (not a kept line).
fn unit_of(entry: &str) -> Option<(String, bool)> {
    let (line, _) = entry.rsplit_once(':')?;
    let (unit, _) = line.rsplit_once('/')?;
    let (tree, kind) = unit.rsplit_once('/')?;
    match kind {
        "u" | "s" => Some((unit.to_owned(), true)),
        "k" => Some((unit_tag(tree, 's'), false)),
        _ => None,
    }
}

/// Bring what is remembered within `cap` entries, and say what that cost.
///
/// The list was cut from its front: the oldest lines went, with no word, and the unchanged
/// failure they were of blocked a second time when a stop came back to it. Eight working
/// trees with a failure of 256 lines each fill the list (review V4 round 16, VD4R16-M-1).
/// Now nothing a unit needs is dropped in silence:
///
/// 1. While there are more entries than `cap`, a unit is put into its short form: its own
///    lines become one mark ([`hook::mark_of`]), and the lines kept for a staged change
///    one mark of everything that has blocked for it ([`Unit::short`]). In that form the
///    unit stands while its failure is exactly what a mark stands for, and blocks as a whole
///    when anything of it changes. The units this stop did not look at go first, the one
///    with the most entries first of those; a unit this stop looked at goes only when every
///    other is in its short form already, and is named in [`Kept::short`].
/// 2. When every unit is one mark and they are still more than `cap`, the units this stop
///    did not look at are forgotten from the front of the list (the one a stop last
///    remembered longest ago first) and named in [`Kept::lost`], which the stop says: that
///    is the only way an unchanged failure blocks twice while the state can be written.
///
/// An entry is at most 64 bytes (a tree of 16, a kind, a line of 24, a count of up to 20
/// digits), so with `cap` at [`BLOCKED_CAP`] the list is under 140 KiB of a state that is
/// read up to [`STATE_CAP`].
fn within_bound(blocked: &mut Vec<String>, looked: &BTreeSet<String>, cap: usize) -> Kept {
    let mut kept = Kept::default();
    if blocked.len() <= cap {
        return kept;
    }
    // each unit, in the order of the list: its own lines, the lines kept for it, and how
    // many entries it has
    let mut order: Vec<String> = Vec::new();
    let mut units: std::collections::BTreeMap<String, Unit> = std::collections::BTreeMap::new();
    for entry in blocked.iter() {
        let Some((unit, own)) = unit_of(entry) else {
            continue;
        };
        let found = units.entry(unit.clone()).or_insert_with(|| {
            order.push(unit.clone());
            Unit::default()
        });
        found.entries += 1;
        if let Some((line, count)) = entry.rsplit_once(':')
            && let (Some((_, key)), Ok(count)) = (line.rsplit_once('/'), count.parse())
        {
            let lines = if own { &mut found.own } else { &mut found.kept };
            lines.insert(key.to_owned(), count);
        }
    }
    let here = |unit: &str| tree_of(unit).is_some_and(|tree| looked.contains(tree));
    let gain = |unit: &str| units[unit].entries - units[unit].short_size();
    let mut candidates: Vec<&String> = order.iter().filter(|unit| gain(unit) > 0).collect();
    candidates.sort_by_key(|unit| (here(unit), std::cmp::Reverse(gain(unit))));
    // (an entry that is no unit's is not one this version wrote: it is not kept)
    let mut total: usize = units.values().map(|unit| unit.entries).sum();
    let mut shortened: BTreeSet<&str> = BTreeSet::new();
    for unit in candidates {
        if total <= cap {
            break;
        }
        total -= gain(unit);
        shortened.insert(unit);
        // A staged change in its short form answers only for exactly what it was: after
        // a change of the working tree it may block with the index untouched. That is
        // said at this stop, whichever tree the stop is in.
        if unit.ends_with("/s") {
            kept.as_a_whole.push(unit.clone());
        } else if here(unit) {
            kept.short.push(unit.clone());
        }
        if units[unit].short().gone {
            kept.given_up.push(unit.clone());
        }
    }
    // the last resort: whole units go, the ones this stop looked at last of all
    let mut dropped: BTreeSet<&str> = BTreeSet::new();
    let mut last: Vec<&String> = order.iter().collect();
    last.sort_by_key(|unit| here(unit));
    for unit in last {
        if total <= cap {
            break;
        }
        let size = if shortened.contains(unit.as_str()) {
            units[unit].short_size()
        } else {
            units[unit].entries
        };
        if size == 0 {
            continue;
        }
        total -= size;
        dropped.insert(unit);
        kept.lost.push(unit.clone());
    }
    kept.short.retain(|unit| !dropped.contains(unit.as_str()));
    kept.given_up
        .retain(|unit| !dropped.contains(unit.as_str()));
    kept.as_a_whole
        .retain(|unit| !dropped.contains(unit.as_str()));
    let mut written: BTreeSet<String> = BTreeSet::new();
    let mut bounded = Vec::with_capacity(cap.min(blocked.len()));
    for entry in blocked.drain(..) {
        let Some((unit, _)) = unit_of(&entry) else {
            continue;
        };
        if dropped.contains(unit.as_str()) {
            continue;
        }
        if !shortened.contains(unit.as_str()) {
            bounded.push(entry);
        } else if written.insert(unit.clone()) {
            let short = units[&unit].short();
            if let Some((key, count)) = short.own {
                bounded.push(hook::remembered_line(&unit, &key, count));
            }
            if let (Some((key, count)), Some(tree)) = (short.kept, tree_of(&unit)) {
                bounded.push(hook::remembered_line(&unit_tag(tree, 'k'), &key, count));
            }
        }
    }
    *blocked = bounded;
    kept
}

/// What a stop says when its memory of failures could not be kept as it was ([`Kept`]): at
/// the stop where that happens, not when a block comes of it.
fn memory_notes(kept: &Kept) -> Vec<String> {
    let mut notes = Vec::new();
    let changes = |count: usize| {
        if count == 1 {
            "1 change".to_owned()
        } else {
            format!("{count} changes")
        }
    };
    if !kept.lost.is_empty() {
        notes.push(format!(
            "The working trees of this project have more failing changes than the hook remembers for one session ({BLOCKED_CAP}): which failures have blocked could not be kept for {} that this stop did not look at. An unchanged failure there may block once more at a stop that looks at it again.",
            changes(kept.lost.len())
        ));
    }
    let trees = |count: usize| match count {
        1 => "1 working tree".to_owned(),
        trees => format!("{trees} working trees"),
    };
    // Each of these says what happened at this stop, and nothing that is not true of it:
    // the sentence began "This session has more failing lines than the hook keeps one by
    // one (2048)" also for one working tree with 300 staged lines of which one file was
    // taken out of the index (tenth independent review of the hooks, HR10-L-1).
    if !kept.given_up.is_empty() {
        notes.push(format!(
            "Of what is staged in {}, the hook remembered what had blocked as a whole, by a digest of all those lines (a change of more than {} failing lines, or a session of more than {BLOCKED_CAP}), and the index no longer holds exactly that: which of the lines that are still staged have blocked before is not known any more. What is staged there may block once more when the working tree is repaired again, although those lines have not changed.",
            trees(kept.given_up.len()),
            hook::LINES_KEPT
        ));
    }
    if !kept.as_a_whole.is_empty() {
        notes.push(format!(
            "What has blocked of what is staged in {} is remembered as a whole from now on, by a digest of all those lines (a staged change of more than {} failing lines of its own, or a session of more than {BLOCKED_CAP}), and it is not known to be everything the index holds. While the working tree there stays as it is, what is staged stands; after a change of that working tree it may block once more, although the index has not changed.",
            trees(kept.as_a_whole.len()),
            hook::LINES_KEPT
        ));
    }
    if !kept.short.is_empty() {
        notes.push(format!(
            "This session has more failing lines than the hook keeps one by one ({BLOCKED_CAP}): the failure of {} that this stop looked at is remembered by a digest of all its lines from now on. Unchanged, it stands; after any change, the repair of one part too, it blocks again as a whole.",
            changes(kept.short.len())
        ));
    }
    notes
}

/// What `blocked` remembers under exactly the tag `tag`, as owned lines.
fn lines_of_tag(blocked: &[String], tag: &str) -> Lines {
    hook::remembered_as(blocked, tag)
        .into_iter()
        .map(|(key, count)| (key.to_owned(), count))
        .collect()
}

/// What is remembered of a staged change after a stop that read its index.
struct StagedMemory {
    /// Its own lines (`s`): the lines, or one mark for more than [`hook::LINES_KEPT`].
    own: Lines,
    /// What is kept for it beside them (`k`): lines, or one mark of everything the index
    /// holds.
    kept: Lines,
    /// A mark had to be given up, and not everything the index holds is remembered after
    /// this stop ([`Kept::given_up`]).
    gone: bool,
    /// Its own lines are a mark from this stop on that is not known to be everything the
    /// index holds ([`Kept::as_a_whole`]).
    whole: bool,
}

/// What a stop that read the index of a working tree remembers of its staged change:
/// everything that has blocked as the staged change's own and that the index has held ever
/// since. `before_own` and `before_kept` are what was remembered (`s`, `k`), `index` every
/// line the index holds now, `own` every line of them that is the staged change's own now
/// (the working tree's failure does not have it), `folded` those as they are remembered.
///
/// With lines it is exact: the lines remembered that the index still holds, and the own
/// ones. A mark stands for lines that are not known one by one, and can answer for the
/// index in one case only: it is exactly what the index holds now. Then everything the
/// index holds has blocked, whatever part of it is the staged change's own at any stop,
/// and that mark is what is kept (`k`), for as long as the index stays what it is: the
/// working tree can then have any part of it again, and be repaired again in any part,
/// and nothing of it is new (tenth independent review of the hooks, HR10-M-2). A mark that
/// is neither what the index holds nor what is the staged change's own cannot be kept:
/// lines that left the index and came back would stand by it. It goes, and the stop says
/// so. And own lines that are a mark while not everything the index holds is known to
/// have blocked answer only for exactly themselves: that is said at the stop that first
/// remembers them so.
fn staged_memory(
    before_own: &Lines,
    before_kept: &Lines,
    index: &Lines,
    own: &Lines,
    folded: &Lines,
) -> StagedMemory {
    let marked = |lines: &Lines| lines.keys().any(|key| hook::is_mark(key));
    let before = || before_own.iter().chain(before_kept);
    let held = (!index.is_empty()).then(|| hook::mark_of(index));
    let own_mark = (!own.is_empty()).then(|| hook::mark_of(own));
    let is = |mark: &Option<(String, usize)>, key: &str, count: usize| {
        mark.as_ref()
            .is_some_and(|(known, lines)| known == key && *lines == count)
    };
    // every line the index holds has blocked: one mark says so
    if before().any(|(key, count)| is(&held, key, *count)) {
        return StagedMemory {
            own: folded.clone(),
            kept: if own_mark == held {
                Lines::new()
            } else {
                held.into_iter().collect()
            },
            gone: false,
            whole: false,
        };
    }
    // the lines remembered that the index still holds and that are not its own now
    let kept: Lines = before()
        .filter(|(key, _)| !hook::is_mark(key) && !own.contains_key(*key))
        .filter_map(|(key, count)| Some((key.clone(), (*count).min(*index.get(key)?))))
        .collect();
    let gone = held.is_some()
        && before().any(|(key, count)| hook::is_mark(key) && !is(&own_mark, key, *count));
    // everything the index holds has blocked, by the lines: the own ones and the kept ones
    let all = held.is_some()
        && index.len() == own.len() + kept.len()
        && index
            .iter()
            .all(|(key, count)| own.get(key).or_else(|| kept.get(key)) == Some(count));
    StagedMemory {
        // (the own lines are a mark and the others are lines: one mark for all of it, so
        // that any part of it can become the staged change's own again and stand)
        kept: match held {
            Some(mark) if all && marked(folded) && !kept.is_empty() => Lines::from([mark]),
            _ => kept,
        },
        // A mark that goes is said only where something stays unknown after this stop.
        // Where the own lines and the kept ones are everything the index holds, all of it
        // is remembered from here on and nothing of it can block again by the mark's
        // going: the sentence warned there of a block that could not come (eleventh
        // independent review of the hooks, HR11-L-4: it was said at the very stop that was
        // the block an earlier stop had warned of, with the index untouched).
        gone: gone && !all,
        whole: marked(folded)
            && !all
            && !folded
                .iter()
                .all(|(key, count)| before_own.get(key) == Some(count)),
        own: folded.clone(),
    }
}

/// Take what a stop judged into what the session remembers: `blocked` in its state, and the
/// commits that have their answer in `judged`, the session's list of them.
///
/// What is remembered of a working tree's uncommitted and staged changes is the failing
/// lines a stop saw there, each with its count, kept with the tree ([`tree_instance`]):
///
/// - a stop that read the tree's uncommitted change remembers exactly its lines: a line that
///   is gone is forgotten, so it blocks again if it comes back (third independent review of
///   the hooks, HR3-M-3), and a count that fell is remembered as the lower one, so it blocks
///   if it grows again;
/// - of what the index holds it remembers the staged change's own lines (`s`), and keeps a
///   line it remembered for as long as the index still holds it, also while the tree's
///   failure has the line too (`k`): it was forgotten then, and blocked a second time when
///   the tree was repaired again, although the index had not been touched (seventh,
///   HR7-M-1);
/// - a tree this stop did not look at, and a change that got no verdict, keep what is
///   remembered (sixth, HR6-M-1);
/// - a tree that a stop found with nothing uncommitted, and a tree that `git worktree list`
///   no longer names, have nothing remembered: the failure is not in the project any more.
///   An entry of a removed tree stayed for the rest of the session, and a tree made again
///   at its path was told that its new failure had blocked before (seventh, HR7-H-1).
///
/// Then the whole is brought within [`BLOCKED_CAP`] entries ([`within_bound`]), the trees
/// that are gone first; what that cost is returned, for the stop to say.
///
/// Each commit that got its answer is added to the list, which has no bound that makes an
/// older one be forgotten.
fn remember(state: &mut State, judging: &Judging, judged: &mut Vec<String>) -> Kept {
    remember_within(state, judging, judged, BLOCKED_CAP)
}

/// [`remember`], with the bound as `cap`.
fn remember_within(
    state: &mut State,
    judging: &Judging,
    judged: &mut Vec<String>,
    cap: usize,
) -> Kept {
    // what is remembered under one tag becomes `lines`
    fn keep(blocked: &mut Vec<String>, tag: &str, lines: &Lines) {
        blocked.retain(|entry| entry.rsplit_once('/').is_none_or(|(unit, _)| unit != tag));
        blocked.extend(
            lines
                .iter()
                .map(|(key, count)| hook::remembered_line(tag, key, *count)),
        );
    }
    let (mut given_up, mut as_a_whole): (Vec<String>, Vec<String>) = (Vec::new(), Vec::new());
    for looked in &judging.trees {
        if let Some(lines) = &looked.tree {
            keep(&mut state.blocked, &unit_tag(&looked.id, 'u'), lines);
        }
        if let Some(index) = &looked.index {
            let (own_tag, kept_tag) = (unit_tag(&looked.id, 's'), unit_tag(&looked.id, 'k'));
            let now = staged_memory(
                &lines_of_tag(&state.blocked, &own_tag),
                &lines_of_tag(&state.blocked, &kept_tag),
                index,
                &looked.own,
                &looked.staged,
            );
            if now.gone {
                given_up.push(own_tag.clone());
            }
            if now.whole {
                as_a_whole.push(own_tag.clone());
            }
            keep(&mut state.blocked, &own_tag, &now.own);
            keep(&mut state.blocked, &kept_tag, &now.kept);
        }
    }
    // A tree no longer listed whose directory is a listed tree again is forgotten when the
    // judge's own reading of that tree finds nothing failing there: here, when this stop
    // judged it (from inside it); [`other_trees`] puts it among the clean ones otherwise.
    let found_clean = |id: &str| {
        judging.trees.iter().any(|looked| {
            looked.id == id
                && looked.tree.as_ref().is_some_and(Lines::is_empty)
                && looked.index.as_ref().is_some_and(Lines::is_empty)
        })
    };
    let kept: BTreeSet<&str> = judging
        .unlisted
        .iter()
        .filter(|tree| !matches!(&tree.is, UnlistedIs::Again(now) if found_clean(now)))
        .map(|tree| tree.id.as_str())
        .collect();
    let gone = |entry: &String| {
        let tree = tree_of(entry).unwrap_or_default();
        judging.clean.iter().any(|id| id == tree)
            // (a tree known by its path alone is in no list: nothing says it is gone)
            || judging.listed.as_ref().is_some_and(|listed| {
                !listed.contains(tree)
                    && !tree.starts_with('x')
                    && !kept.contains(tree)
                    && !judging.trees.iter().any(|looked| looked.id == tree)
            })
    };
    state.blocked.retain(|entry| !gone(entry));
    let looked: BTreeSet<String> = judging
        .trees
        .iter()
        .map(|looked| looked.id.clone())
        .collect();
    let mut kept = within_bound(&mut state.blocked, &looked, cap);
    state.trees = recorded_trees(&state.blocked, &state.trees, &judging.tops);
    kept.given_up.extend(given_up);
    for unit in as_a_whole {
        if !kept.as_a_whole.contains(&unit) {
            kept.as_a_whole.push(unit);
        }
    }
    judged.extend(
        judging
            .commits
            .iter()
            .map(|(id, outcome)| format!("{id}:{outcome}")),
    );
    kept
}

/// What [`State::trees`] records after a stop: for each linked tree `blocked` remembers
/// something of, its top level as this stop knows it (`tops`), else as it was recorded
/// (`before`). A path that is not UTF-8, is relative or is longer than [`POINTER_BYTES`]
/// is not recorded, nor the paths past [`TREES_BYTES`] in all.
fn recorded_trees(
    blocked: &[String],
    before: &std::collections::BTreeMap<String, String>,
    tops: &std::collections::BTreeMap<String, PathBuf>,
) -> std::collections::BTreeMap<String, String> {
    let remembered: BTreeSet<&str> = blocked
        .iter()
        .filter_map(|entry| tree_of(entry))
        .filter(|id| is_instance(id))
        .collect();
    let mut bytes = 0_usize;
    let mut trees = std::collections::BTreeMap::new();
    for id in remembered {
        let top = match tops.get(id) {
            Some(top) => top.to_str().map(str::to_owned),
            None => before.get(id).cloned(),
        };
        let Some(top) = top.filter(|top| recordable(top)) else {
            continue;
        };
        if bytes + top.len() > TREES_BYTES {
            continue;
        }
        bytes += top.len();
        trees.insert(id.to_owned(), top);
    }
    trees
}

fn shown_tree(top: &Path) -> String {
    hook::shown_path(&top.to_string_lossy())
}

/// What to say of the project's submodules that have changes not committed in them: the
/// project's patch shows a submodule only as the commit it is at, so those changes are in no
/// patch (second independent review of the hooks, HR2-M-3). Nothing is asked of Git when the
/// project has no `.gitmodules` file.
fn submodules_with_changes(place: &Place, deadline: Deadline) -> Vec<Note> {
    // (no such file: no submodules. A file that cannot be looked at is not "no file": Git
    // is asked, and says)
    if matches!(place.top.join(".gitmodules").try_exists(), Ok(false)) {
        return Vec::new();
    }
    match dirty_submodules(&place.top, &deadline.git()) {
        Ok(paths) => paths
            .iter()
            .map(|path| Note {
                name: format!("the submodule {}", hook::shown_path(path)),
                text: format!(
                    "The submodule {} of {} has changes that are not committed in it: they were not judged (the project's patch shows a submodule only as the commit it is at).",
                    hook::shown_path(path),
                    shown_tree(&place.top)
                ),
                standing: 0,
            })
            .collect(),
        Err(error) => vec![Note {
            name: "the submodules, which could not be looked at".to_owned(),
            text: format!(
                "Whether a submodule of {} has changes that are not committed in it could not be found out ({}): such changes are not judged.",
                shown_tree(&place.top),
                git_words(&error)
            ),
            standing: 0,
        }],
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

/// The most notes one stop says in full; the others are on the line that counts them.
const NOTES_IN_FULL: usize = 5;

/// What the line that counts the notes may need of a message, and the most characters of one
/// name on it.
const COUNT_LINE: usize = 700;
const NOTE_NAME: usize = 180;

/// A note's name as the counting line has it: at most [`NOTE_NAME`] characters, and what it
/// says of a failure that stands is never what is cut. A name was cut at 140 characters
/// from its front: the name of a working tree with a long path lost its end, "(a failure
/// stands there)", which is the one thing the line is there to say of it. (Found by the
/// mutation run, whose scratch directory has a longer path than a gate's: its baseline
/// failed on the test of a working tree inside the project.)
fn counted_name(name: &str) -> String {
    let count = name.chars().count();
    if count <= NOTE_NAME {
        return name.to_owned();
    }
    let said = name
        .rfind(" (a failure stands")
        .map_or("", |at| &name[at..]);
    let head: String = name
        .chars()
        .take(NOTE_NAME.saturating_sub(said.chars().count() + 1))
        .collect();
    format!("{head}…{said}")
}

/// What a stop says beside its verdict: every note at every stop while it is true ([`Note`]),
/// with the one for a session whose start was not recorded. A note that is new to the
/// session is said in full, up to [`NOTES_IN_FULL`] at one stop and while the message has
/// `room` for it; the others are one line, their number and the first three of them, the
/// ones that name a failure which stands first. For the ones the line does not name it says
/// in how many working trees such a failure stands: "(a failure stands there)" was part of a
/// name, so it was said only of the first three (eighth independent review of the hooks,
/// HR8-L-3, the suspected part).
/// Nothing is left out for a cap: what is not said in full is counted. And only what was
/// said in full is remembered as said: a note that was new at a crowded stop was cut to
/// nothing there and was "said before" from then on (sixth independent review of the hooks,
/// HR6-L-4).
fn notes_of(state: &mut State, mut beside: Vec<Note>, room: usize) -> Vec<String> {
    // A session whose start no session-start hook recorded says so, whether this stop or an
    // advice hook before it did the recording (second independent review of the hooks,
    // HR2-M-2): what it committed before that hook is in the project and was not judged.
    // what names a failure that stands comes first, in full and on the line
    beside.sort_by_key(|note| note.standing == 0);
    if state.project.as_ref().is_some_and(|p| !p.by_start) {
        beside.insert(
            0,
            Note {
                name: "what was committed before this session's first hook".to_owned(),
                text: hook::FIRST_STOP.to_owned(),
                standing: 0,
            },
        );
    }
    // Every note at every stop while it is true. In full when it is new to the session, a few
    // at one stop; the others on one line, by their number and the first of them.
    let mut left = room.saturating_sub(COUNT_LINE);
    let mut notes = Vec::new();
    // (the name, and in how many working trees the note has a failure that stands)
    let mut before: Vec<(String, usize)> = Vec::new();
    for note in beside {
        let key = deep_diff_forge_core::sha256_hex(note.text.as_bytes());
        let needs = note.text.chars().count() + 1;
        // (a session that cannot remember more counts a new one with the others: the line
        // that counts them is at every stop, so nothing true at a stop goes unsaid)
        if state.noticed.contains(&key)
            || notes.len() >= NOTES_IN_FULL
            || state.noticed.len() >= NOTICED_CAP
            || needs > left
        {
            before.push((counted_name(&note.name), note.standing));
            continue;
        }
        left -= needs;
        state.noticed.insert(key);
        notes.push(note.text);
    }
    if !before.is_empty() {
        let first: Vec<&str> = before
            .iter()
            .take(3)
            .map(|(name, _)| name.as_str())
            .collect();
        let unnamed: usize = before.iter().skip(3).map(|(_, standing)| standing).sum();
        let more = match (before.len() - first.len(), unnamed) {
            (0, _) => String::new(),
            (more, 0) => format!(", and {more} more"),
            (more, 1) => format!(
                ", and {more} more (among them 1 working tree where a failure stands that has blocked)"
            ),
            (more, trees) => format!(
                ", and {more} more (among them {trees} working trees where a failure stands that has blocked)"
            ),
        };
        let (count, are) = if before.len() == 1 {
            ("1 thing".to_owned(), "is")
        } else {
            (format!("{} things", before.len()), "are")
        };
        notes.push(format!(
            "{count} in this project {are} not judged: {}{more}.",
            first.join(", ")
        ));
    }
    notes
}

/// The characters `notes` take in a message, each on a line of its own.
fn lines_taken(notes: &[String]) -> usize {
    notes.iter().map(|note| note.chars().count() + 1).sum()
}

/// How long a stop waits for another hook of its session to let go of the session's state.
const LOCK_WAIT: Duration = Duration::from_secs(3);
/// How long a session-start hook waits for it, and how long an advice hook does: an advice
/// hook that cannot have it leaves its state unwritten and says nothing.
const START_LOCK_WAIT: Duration = Duration::from_secs(2);
const ADVICE_LOCK_WAIT: Duration = Duration::from_millis(300);

/// The session's lock, held: a `flock` on the file `session-<hash>.lock` beside its state. It
/// is let go when this is dropped, and when the process ends however it ends: a hook that is
/// killed holds nothing. The file is taken away again by a hook that leaves the session
/// with no state (a stop where there is no repository), so that a session the hooks have
/// nothing to do with leaves no file behind.
struct Held {
    #[expect(dead_code, reason = "held for its lock")]
    lock: nix::fcntl::Flock<std::fs::File>,
    path: PathBuf,
    state: PathBuf,
}

impl Drop for Held {
    fn drop(&mut self) {
        // still locked here: whoever waits on this file finds another at its path, or
        // none, and takes that one ([`session_lock`])
        if !self.state.exists() {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// Whether a hook has its session's state to itself.
enum Turn {
    /// It has, for as long as this is held.
    Mine(#[expect(dead_code, reason = "held for its lock")] Held),
    /// There is no lock to have: no state directory can be made (what the hook then says
    /// of its state says so too), or the file system has no locks.
    Unlocked,
    /// Another hook of the session holds it and did not let go in the time given.
    Taken,
}

/// Take the session's lock, waiting at most `wait`.
///
/// Every hook reads the session's state, works, and writes it. Two that did so at the same
/// time wrote across each other. Two stops: one's list of judged commits, the other's list,
/// the other's state, the first's state, after which the state named a list that was not in
/// the file and every later stop was UNJUDGED (sixth independent review of the hooks). Two
/// stops that were both the session's first hook, which took no lock because there was no
/// state yet: both judged, both blocked for the same failure, both wrote (review V4 round
/// 15, VD4R15-M-3). An advice hook and a stop: the advice hook's older copy of what had
/// blocked came back, so a failure that a stop had seen repaired and forgotten "had blocked"
/// again and did not block when it returned (seventh independent review, HR7-L-4).
///
/// So the lock is taken by every stop for all of its run, the first one too (the file is
/// named from the session's id, which a hook has before any state exists), by a
/// session-start hook for its reading and writing, and by an advice hook for its writing
/// ([`Session::keep_advice`]).
#[expect(
    clippy::disallowed_methods,
    reason = "the session-lock door: a hook waits a bounded time, in short sleeps, for another hook of its session"
)]
fn session_lock(session: &str, wait: Duration) -> Turn {
    use nix::fcntl::{Flock, FlockArg};
    use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};
    let Ok(dir) = sessions_dir() else {
        return Turn::Unlocked;
    };
    // The directory is made here when it is missing, unless it would lie inside a Git
    // working tree: a state directory inside the project is refused by every hook, and
    // nothing is written there to find that out.
    let inside_a_tree = || {
        resolved(&dir)
            .ancestors()
            .any(|above| above.join(".git").symlink_metadata().is_ok())
    };
    let made = dir.is_dir()
        || (!inside_a_tree()
            && dir
                .parent()
                .is_none_or(|parent| std::fs::create_dir_all(parent).is_ok())
            && own_directory(&dir).is_ok());
    if !made {
        return Turn::Unlocked;
    }
    let state = state_file(&dir, session);
    let path = state.with_extension("lock");
    let open = || {
        std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK)
            .open(&path)
    };
    let until = Deadline::after(wait);
    let Ok(mut file) = open() else {
        return Turn::Unlocked;
    };
    loop {
        match Flock::lock(file, FlockArg::LockExclusiveNonblock) {
            Ok(lock) => {
                // The file that was locked must be the one at the path: a hook that left
                // the session with no state has taken its lock file away, and another may
                // have made a new one since.
                let same = match (lock.metadata(), std::fs::symlink_metadata(&path)) {
                    (Ok(held), Ok(there)) => held.ino() == there.ino() && held.dev() == there.dev(),
                    _ => false,
                };
                if same {
                    return Turn::Mine(Held { lock, path, state });
                }
                drop(lock);
                if until.left().is_zero() {
                    return Turn::Taken;
                }
                let Ok(again) = open() else {
                    return Turn::Unlocked;
                };
                file = again;
            }
            Err((back, nix::errno::Errno::EWOULDBLOCK)) if !until.left().is_zero() => {
                file = back;
                std::thread::sleep(Duration::from_millis(25));
            }
            Err((_, nix::errno::Errno::EWOULDBLOCK)) => return Turn::Taken,
            // a file system with no locks: as before
            Err(_) => return Turn::Unlocked,
        }
    }
}

/// What a stop says in a repository that has no commit: it has no base, so nothing in it is
/// judged, and its files are in the project all the same. They are said, as everything is
/// that is known and not judged. (A status that cannot be read is a tree with files.)
fn no_commit_yet(work: &Work, deadline: Deadline) -> Option<Note> {
    (!matches!(
        has_uncommitted_changes(&work.place.top, &deadline.git()),
        Ok(false)
    ))
    .then(|| Note {
        name: "the files of this repository, which has no commit yet".to_owned(),
        text: format!(
            "The repository {} has no commit yet, so there is no commit to measure this session's changes from: its files were not judged. A session that begins after the first commit is judged.",
            shown_tree(&work.place.top)
        ),
        standing: 0,
    })
}

/// When the agent wants to stop: have the session's work judged, unit by unit
/// ([`judge_units`]), and block on a fail that holds this stop.
fn stop(input: &Input, args: &[String]) -> Result<Option<String>, String> {
    let deadline = Deadline::after(STOP_LIMIT);
    // one stop of a session at a time, from before its state is read until after it is written
    let turn = session_lock(&input.session, LOCK_WAIT.min(deadline.left()));
    if matches!(turn, Turn::Taken) {
        return Ok(Some(hook::unjudged_output(
            "another hook of this session is running and has not ended (a stop, or the session's start), so this stop judged nothing: when it was a stop, what that stop says is the answer",
        )));
    }
    let mut session = Session::open(&input.session)?;
    let work = match locate(input, &mut session, deadline) {
        Located::Nowhere => return Ok(None),
        Located::Trouble(reason) => return Ok(Some(hook::unjudged_output(&reason))),
        Located::Here(work) => work,
    };
    let deadline = deadline.with_project(&work.place);
    if let Some(why) = session.inside(&work.place.top) {
        return Ok(Some(hook::unjudged_output(&why)));
    }
    // The project and the base are recorded before anything else can fail.
    let mut unsaved = session.keep().err();
    let mut beside: Vec<Note> = Vec::new();
    let mut memory: Vec<String> = Vec::new();
    let mut decision = match subject(&work, deadline) {
        Subject::Nothing => {
            beside.extend(no_commit_yet(&work, deadline));
            StopDecision::Allow { message: None }
        }
        Subject::Unjudged(reason) => StopDecision::Allow {
            message: Some(hook::unjudged_message(&reason)),
        },
        Subject::Range { base, words } => {
            // Without the commits that were there when the session began, its own cannot be
            // told from them; without the ones that were judged, they would be judged again.
            let lists = session
                .tips(&work.project)
                .and_then(|tips| Ok((tips, session.judged()?)));
            match lists {
                Err(reason) => StopDecision::Allow {
                    message: Some(hook::unjudged_message(&reason)),
                },
                Ok((tips, mut known)) => {
                    let scratch = Scratch::new(&work.place.own)?;
                    let number = session.state.stops.saturating_add(1);
                    let (judging, look) = stop_deadlines(&work.place, deadline);
                    let stopping = Stopping {
                        work: &work,
                        cwd: &input.cwd,
                        base: &base,
                        scratch: &scratch,
                        args,
                        deadline: judging,
                        look,
                        number,
                        recorded: &session.state.trees,
                    };
                    let judging = judge_units(&stopping, &tips, &known, &session.state.blocked)?;
                    // Decided by what had blocked before this stop; then this stop is
                    // remembered.
                    let decision =
                        hook::decide_stop_trees(&judging.units, &session.state.blocked, &words);
                    // The stops are counted for the commits that are asked about and get no
                    // verdict, which are marked with the number: a stop with none of them
                    // changes nothing by having been made.
                    if judging.commits.iter().any(|(_, said)| {
                        matches!(
                            answer(&format!("{}:{said}", "0".repeat(40))),
                            Some((_, Answer::NoVerdict { .. }))
                        )
                    }) {
                        session.state.stops = number;
                    }
                    let before = known.len();
                    memory = memory_notes(&remember(&mut session.state, &judging, &mut known));
                    if known.len() != before {
                        session.set_judged(known);
                    }
                    beside = judging.beside;
                    decision
                }
            }
        }
    };
    // what the memory cost is said whole, at this stop: the other notes get the room it leaves
    let mut notes = notes_of(
        &mut session.state,
        beside,
        hook::room_beside(&decision).saturating_sub(lines_taken(&memory)),
    );
    notes.append(&mut memory);
    if let Err(why) = session.keep() {
        unsaved = Some(why);
    }
    // A state that cannot be saved must not hide the decision, and is said with it: the same
    // failure then blocks at every stop, which used to happen with no word of why
    // (independent review of the hooks, HR-L-4).
    if let Some(why) = unsaved {
        // And it must not block for good. A failing commit cannot be repaired, and with no
        // state it would be judged, and block, at every stop. The client says when it is
        // continuing because a stop hook has just blocked: then this stop is not blocked
        // again (fourth independent review of the hooks, HR4-M-3).
        if input.stop_hook_active
            && let StopDecision::Block { reason } = &decision
        {
            decision = StopDecision::Allow {
                message: Some(format!(
                    "deep-diff-forge: FAIL STANDS. This failure is not blocked because the client says this stop already follows a block and the hook cannot keep its own record of what has blocked; it is left for a person. The judge's answer:\n{reason}"
                )),
            };
        }
        notes.push(format!(
            "The hook could not keep its state ({}): the same failure can block again, a commit can be judged again, and the commit this session began from may not be remembered.",
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
    let first = std::str::from_utf8(policy).ok()?.lines().next()?;
    if name.is_empty() {
        return None;
    }
    let (old, new) = (
        patch_path(&format!("a/{name}")),
        patch_path(&format!("b/{name}")),
    );
    Some(format!(
        "diff --git {old} {new}\n--- {old}\n+++ {new}\n@@ -1,1 +1,2 @@\n+# deep-diff-forge doctor probe\n {first}\n"
    ))
}

/// A path as a patch header writes it: as it is when it is made of printable ASCII without
/// `"` and `\`, else in Git's quoted form (octal for every other byte). The probe used to be
/// made only for a policy named in letters, digits and `._/-`, so the gate could not say of
/// another name what a stop would do (fifth independent review of the hooks, HR5-L-2).
fn patch_path(path: &str) -> String {
    use std::fmt::Write as _;
    let plain = path
        .bytes()
        .all(|b| (0x20..0x7f).contains(&b) && b != b'"' && b != b'\\');
    if plain && !path.ends_with(' ') {
        return path.to_owned();
    }
    let mut quoted = String::from("\"");
    for byte in path.bytes() {
        match byte {
            b'"' => quoted.push_str("\\\""),
            b'\\' => quoted.push_str("\\\\"),
            b'\t' => quoted.push_str("\\t"),
            b'\n' => quoted.push_str("\\n"),
            0x20..=0x7e => quoted.push(char::from(byte)),
            other => {
                let _ = write!(quoted, "\\{other:03o}");
            }
        }
    }
    quoted.push('"');
    quoted
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
            "no Claude Code settings file registers a {event} hook of this program: {READ_HERE} (agent setup --client claude --hooks prints the block)"
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
        format!(
            "no Claude Code settings file registers a stop hook: {READ_HERE} (agent setup --client claude --hooks prints the block)"
        )
    }))
}

/// Which settings the gate reads, for a link that found no hook: hooks that come from
/// settings it does not read were "not registered" with no word of where it had looked
/// (sixth independent review of the hooks, HR6-L-5).
const READ_HERE: &str = "the project's .claude/settings.local.json and .claude/settings.json and the user's ~/.claude/settings.json were read; settings an administrator manages and hooks a plugin brings are not";

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
        Found::NotARepository | Found::AtBoundary { .. } | Found::Gone => {
            Err("this directory is not in a Git repository".to_owned())
        }
        Found::Failed { reason, .. } => Err(reason),
        Found::Unpaired(unpaired) => Err(unpaired.reason(cwd)),
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
fn chain_checks(name: &str, verdict: &Result<(PathBuf, i32, String), String>) -> [GateCheck; 2] {
    [
        check(
            "chain",
            verdict.is_ok(),
            match verdict {
                Ok((ranker, code, read)) => format!(
                    "the judge gave a verdict (exit {code}) with ranker {}: {read}, read by this build, which reads {}",
                    ranker.display(),
                    crate::verdict_v2::READS
                ),
                Err(why) => why.clone(),
            },
        ),
        check(
            hook::PROTECTS_ITSELF,
            verdict.as_ref().is_ok_and(|(_, code, _)| *code != 0),
            match verdict {
                Ok((_, 0, _)) => format!(
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
                    error @ (JudgePatchError::SkipWorktree | JudgePatchError::Unmerged) => {
                        unjudged(error.to_string())
                    }
                })
            })
            // what is staged is a unit of a stop too: an index with unmerged paths has no
            // tree, and a stop then has no verdict on it (HR5-L-6)
            .and_then(|patch| {
                let index = scratch.sub("index")?;
                staged_patch(&place.top, rev, &index, &deadline.git()).map_err(|error| {
                    format!(
                        "a stop here would be UNJUDGED for what is staged: {}",
                        match &error {
                            JudgePatchError::Failed(error) =>
                                format!("it could not be read from Git: {}", git_words(error)),
                            other => other.to_string(),
                        }
                    )
                })?;
                Ok(patch)
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
    checks.push(commits_check(place, deadline));
    checks.push(state_directory_check(place));
    checks.extend(settings_checks(place, program.as_deref(), &name, args));
    Gate { checks, dirty }
}

/// The link `commits`: the commits that the repository's refs, the `HEAD` of each working tree
/// and their reflogs name can be listed, as the first hook of a session lists them to tell
/// the session's own commits from them. Where they cannot, no session is recorded and every
/// stop is UNJUDGED, which the gate did not see (fourth independent review of the hooks,
/// HR4-L-1); or, where the repository has more of them than a hook reads, the session is
/// recorded with no list, and its commits are UNJUDGED at every stop.
fn commits_check(place: Option<&Place>, deadline: Deadline) -> GateCheck {
    let listed = match place {
        Some(place) => named_commits(&place.top, Named::Everything, &deadline.git())
            .map(|commits| commits.len())
            .map_err(|error| match &error {
                // (a refusal: the session is recorded with no list of them, and what is
                // not committed is judged; [`record`])
                GitAcquireError::InvalidSelection { .. } => format!(
                    "the commits of a session here would be UNJUDGED at every stop (what is not committed is judged): the commits the repository names cannot be listed: {}",
                    git_words(&error)
                ),
                _ => format!(
                    "a session here could not be recorded and its stops would be UNJUDGED: the commits the repository names could not be listed: {}",
                    git_words(&error)
                ),
            }),
        None => Err(not_checked("it needs the repository")),
    };
    check(
        "commits",
        listed.is_ok(),
        match listed {
            Ok(count) => format!(
                "{count} commits are named by the refs, the working trees and their reflogs: a session's own are told from them"
            ),
            Err(why) => why,
        },
    )
}

/// The version of forge-warden that a judge's document says wrote it, bounded and safe to show.
fn judge_version(document: &Value) -> String {
    let version: String = document["tool"]["version"]
        .as_str()
        .unwrap_or("of no stated version")
        .chars()
        .take(hook::SCHEMA_SHOWN)
        .collect();
    deep_diff_forge_core::display_safe(&version).into_owned()
}

/// Have the judge judge the probe. `Ok` with the ranker it admitted, its exit code and the
/// schema and version its document names when it gave a verdict; otherwise why not, with the
/// version its document names when the document names a schema this build does not read.
fn probe(
    scratch: &Scratch,
    name: &str,
    policy: &[u8],
    program: &Path,
    args: &[String],
    deadline: Deadline,
) -> Result<(PathBuf, i32, String), String> {
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
            run_judge(program, &policy_file, &patch_file, &ranker, deadline.left());
        match (
            code,
            hook::judged(code, verdict.as_ref(), said.as_deref(), sealed),
        ) {
            (Some(code), Judged::Pass | Judged::Hold { .. } | Judged::Fail { .. }) => {
                let document = verdict.unwrap_or_default();
                let read = format!(
                    "{} of forge-warden {}",
                    document["schema"].as_str().unwrap_or_default(),
                    judge_version(&document)
                );
                return Ok((ranker, code, read));
            }
            (_, Judged::Unjudged { reason }) => {
                // version drift: a judge that writes a schema this build does not read
                last = match verdict.filter(|d| {
                    d["schema"].is_string() && crate::verdict_v2::Version::named_by(d).is_none()
                }) {
                    Some(document) => format!(
                        "{reason}; the document says forge-warden {} wrote it",
                        judge_version(&document)
                    ),
                    None => reason,
                };
            }
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
            id: "11082023".to_owned(),
            base,
            tips_sha256: "a".repeat(64),
            tips_count: 2,
            no_baseline: None,
            by_start: true,
        }
    }

    /// Second independent review of the hooks, HR2-M-1. A state the hooks wrote is read back
    /// whole. Anything else that stands at a state's name is an error with its reason, never an
    /// empty state: read as "no state", it made the next hook the session's first and the
    /// commit `HEAD` named then its base.
    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "one list: every way a state file can be whole or not"
    )]
    fn a_state_round_trips_and_anything_else_is_not_a_state() {
        fn with_project(project: &Value) -> String {
            json!({"schema": STATE_SCHEMA, "project": project, "stops": 0,
                   "reported": ["a"], "noticed": [], "blocked": [],
                   "judged_sha256": "", "judged_count": 0, "trees": {}})
            .to_string()
        }
        let state = State {
            project: Some(project(Base::Commit(COMMIT.to_owned()))),
            stops: 7,
            reported: ["b".to_owned(), "a".to_owned()].into(),
            noticed: ["n".to_owned()].into(),
            blocked: vec![format!("m/u/{}:3", "f".repeat(24))],
            judged_sha256: "d".repeat(64),
            judged_count: 2,
            trees: [("0123456789abcdef".to_owned(), "/w/feature".to_owned())].into(),
        };
        assert_eq!(state_from(&state_text(&state)), Ok(state));
        for base in [Base::NoCommit, Base::Orphan, Base::Commit("a".repeat(64))] {
            let other = State {
                project: Some(Project {
                    tips_count: 0,
                    no_baseline: None,
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
        // full commit id is one (a name like `HEAD` would move with the session), the
        // repository's identity is one number (third independent review of the hooks,
        // HR3-M-4: the device number that stood before it is given at mount time), and the
        // tips file is named by a whole SHA-256 and a count.
        let whole = json!({"top": "/r", "id": "2", "base": "none",
                           "tips_sha256": "0".repeat(64), "tips_count": 0,
                           "no_baseline": null, "by_start": true});
        // Review V4 round 18, VD4R18-L-1: a project whose list of the commits that were
        // there could not be made says so, with the reason, and names no list: that is a
        // state of its own. A reason beside a list, a list with no digest and no reason,
        // and a project that does not say which it is are no state.
        let no_list = |reason: Value, digest: &str, count: usize| {
            let mut project = whole.clone();
            project["no_baseline"] = reason;
            project["tips_sha256"] = json!(digest);
            project["tips_count"] = json!(count);
            project
        };
        let unlisted = state_from(&with_project(&no_list(
            json!("too many working trees"),
            "",
            0,
        )))
        .unwrap();
        assert_eq!(
            unlisted.project.as_ref().unwrap().no_baseline.as_deref(),
            Some("too many working trees")
        );
        assert_eq!(state_from(&state_text(&unlisted)), Ok(unlisted));
        for spoiled in [
            no_list(json!("why"), &"0".repeat(64), 0),
            no_list(json!("why"), "", 1),
            no_list(json!(""), "", 0),
            no_list(Value::Null, "", 0),
            no_list(json!(1), "", 0),
        ] {
            assert!(state_from(&with_project(&spoiled)).is_err(), "{spoiled}");
        }
        let mut silent = whole.clone();
        silent.as_object_mut().unwrap().remove("no_baseline");
        assert!(state_from(&with_project(&silent)).is_err());
        let with = |project: &Value| with_project(project);
        assert!(state_from(&with(&whole)).is_ok());
        for (member, value) in [
            ("top", json!("")),
            ("top", Value::Null),
            ("id", json!("1:2")),
            ("id", json!("x")),
            ("id", json!("")),
            ("id", json!(2)),
            ("id", Value::Null),
            ("base", json!("HEAD")),
            ("base", json!("0123456")),
            ("base", json!("0123456789ABCDEF0123456789abcdef01234567")),
            ("base", json!(7)),
            ("tips_sha256", json!("0".repeat(63))),
            ("tips_sha256", json!("g".repeat(64))),
            ("tips_sha256", Value::Null),
            ("tips_count", json!(-1)),
            ("tips_count", json!("2")),
            ("tips_count", Value::Null),
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
        // not JSON, another schema (the ones before this version too), a missing member, a
        // member of the wrong type, a judged commit that is not one: each is refused, none is
        // read in part
        for foreign in [
            "",
            "not json",
            "[]",
            r#"{"schema":"other","reported":["a"]}"#,
            r#"{"schema":"deep-diff-forge.hook-state.v1","project":null,"reported":["a"],"noticed":[],"last_block":null}"#,
            r#"{"schema":"deep-diff-forge.hook-state.v2","project":null,"start_said":false,"reported":[],"noticed":[],"blocked":[]}"#,
            r#"{"reported":["a"]}"#,
            r#"{"schema":"deep-diff-forge.hook-state.v3","project":null,"reported":[],"noticed":[],"more_said":false,"blocked":[],"judged":[]}"#,
            r#"{"schema":"deep-diff-forge.hook-state.v3","project":null,"start_said":false,"reported":["a",1],"noticed":[],"more_said":false,"blocked":[],"judged":[]}"#,
            r#"{"schema":"deep-diff-forge.hook-state.v3","project":null,"start_said":false,"reported":[],"noticed":[],"more_said":false,"blocked":"x","judged":[]}"#,
            r#"{"schema":"deep-diff-forge.hook-state.v3","project":null,"start_said":false,"reported":[],"noticed":[],"more_said":false,"blocked":[]}"#,
            r#"{"schema":"deep-diff-forge.hook-state.v3","project":null,"start_said":false,"reported":[],"noticed":[],"blocked":[],"judged":[]}"#,
            r#"{"schema":"deep-diff-forge.hook-state.v3","project":null,"start_said":false,"reported":[],"noticed":[],"more_said":false,"blocked":[],"judged":["0123456789abcdef0123456789abcdef01234567"]}"#,
            r#"{"schema":"deep-diff-forge.hook-state.v3","project":null,"start_said":false,"reported":[],"noticed":[],"more_said":false,"blocked":[],"judged":["0123456789abcdef0123456789abcdef01234567:x"]}"#,
            r#"{"schema":"deep-diff-forge.hook-state.v3","project":null,"start_said":false,"reported":[],"noticed":[],"more_said":false,"blocked":[],"judged":["HEAD:p"]}"#,
        ] {
            assert!(state_from(foreign).is_err(), "{foreign}");
        }
    }

    /// HR2-M-1: every list of a state is bounded when it is written, so the largest state the
    /// hooks can write is far under what they read back, and it is read back whole. Before, a
    /// block with 4,000 violations wrote 1.1 MB and the reader stopped at 1 MiB. The commits
    /// that were there when the session began are not in the state at all: they are in the
    /// tips file, which has no such bound (third independent review of the hooks, HR3-H-2).
    #[test]
    fn the_largest_state_the_hooks_write_is_read_back() {
        let hex = |n: usize, width: usize| format!("{n:0width$x}");
        let full = State {
            project: Some(Project {
                top: "/".to_owned() + &"p".repeat(4_000),
                id: u64::MAX.to_string(),
                base: Base::Commit(hex(0, 64)),
                tips_sha256: hex(0, 64),
                tips_count: usize::MAX,
                no_baseline: None,
                by_start: false,
            }),
            stops: u64::MAX,
            reported: (0..REPORTED_CAP).map(|n| hex(n, 64)).collect(),
            noticed: (0..NOTICED_CAP).map(|n| hex(n, 64)).collect(),
            blocked: (0..BLOCKED_CAP)
                .map(|n| format!("{}:{}", hex(n, 64), usize::MAX))
                .collect(),
            judged_sha256: hex(0, 64),
            judged_count: usize::MAX,
            trees: (0..TREES_BYTES / usize::try_from(POINTER_BYTES).unwrap())
                .map(|n| {
                    let long = usize::try_from(POINTER_BYTES).unwrap() - 1;
                    (hex(n, 16), "/".to_owned() + &"t".repeat(long))
                })
                .collect(),
        };
        assert_eq!(
            full.trees.values().map(String::len).sum::<usize>(),
            TREES_BYTES
        );
        let text = state_text(&full);
        assert!(
            (text.len() as u64) < STATE_CAP / 4,
            "{} bytes of {STATE_CAP}",
            text.len()
        );
        assert_eq!(state_from(&text), Ok(full.clone()));
        // one entry more in any list is not a state the hooks wrote
        for grow in 0..3 {
            let mut over = full.clone();
            match grow {
                0 => {
                    over.reported.insert("x".to_owned());
                }
                1 => {
                    over.noticed.insert("x".to_owned());
                }
                _ => over.blocked.push("x".to_owned()),
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
        // a file of exactly the cap is read (and is then no state): the bound is "over"
        std::fs::write(&file, "x".repeat(usize::try_from(STATE_CAP).unwrap())).unwrap();
        assert_eq!(load(&file), Err("it is not JSON".to_owned()));
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

    /// The rule of the hooks for a commit, by what the session's file says of it: every commit
    /// that has no pass is a unit of the stop, and every commit that has no verdict and none
    /// the commit itself proves is asked about again, at this stop or in its turn. A commit
    /// that had no verdict in five attempts was a unit and was asked about no more (review V4
    /// round 14, VD4R14-M-1); then it took turns behind every commit with fewer attempts, and
    /// three of those that the judge gave no verdict for kept it from ever being asked about
    /// again (round 15, VD4R15-M-1).
    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "one table of states, one order, and the property simulated over many stops"
    )]
    fn a_commit_without_a_verdict_is_always_asked_about_again() {
        let id = |n: usize| format!("{n:040x}");
        let listed =
            |count: usize| -> Vec<CommitAndParent> { (0..count).map(|n| (id(n), None)).collect() };
        // each state a commit can be in: is it a unit, and is it asked about
        for (state, unit, asked) in [
            (None, false, true),
            (Some("p"), false, false),
            (Some("h:quiet"), true, false),
            (Some("f:no-secrets"), true, false),
            (Some("x:too large"), true, false),
            (Some("n0@1:r"), true, true),
            (Some("n4@9:r"), true, true),
            (Some("n5@9:r"), true, true),
            (Some("n6@9:r"), true, true),
            (Some("n4294967295@9:r"), true, true),
        ] {
            let known: Vec<String> = state
                .iter()
                .map(|said| format!("{}:{said}", id(0)))
                .collect();
            let (units, ask) = sorted_commits(&listed(1), &known);
            assert_eq!(units[0].is_some(), unit, "{state:?}");
            assert_eq!(ask.len(), usize::from(asked), "{state:?}");
        }
        // The order: of those past five attempts the one asked about least recently (the
        // older of two), of those with fewer attempts the one asked about least recently,
        // then what was never asked about, then the others with fewer attempts.
        let known: Vec<String> = [
            (0, "n7@9:r"),
            (1, "n5@3:r"),
            (2, "n2@8:r"),
            (3, "n5@3:r"),
            (5, "n0@2:r"),
            (6, "p"),
            (7, "n4@8:r"),
        ]
        .iter()
        .map(|(n, said)| format!("{}:{said}", id(*n)))
        .collect();
        let (units, ask) = sorted_commits(&listed(8), &known);
        let order: Vec<(usize, Option<u32>, bool)> = ask
            .iter()
            .map(|ask| (ask.at, ask.attempts, ask.reserved))
            .collect();
        assert_eq!(
            order,
            [
                (1, Some(5), true),
                (5, Some(0), true),
                (4, None, true),
                (2, Some(2), false),
                (7, Some(4), false)
            ]
        );
        // each of the three that wait for their turn is a unit, and says so
        for (at, attempts) in [(0, 7), (1, 5), (3, 5)] {
            let said = match units[at].as_ref().and_then(|unit| unit.judged.as_ref()) {
                Some(Judged::Unjudged { reason }) => reason.clone(),
                other => panic!("{at}: {other:?}"),
            };
            assert_eq!(
                said,
                format!(
                    "no verdict in {attempts} attempts; it is asked about again in its turn, one such commit at each stop; the last reason: r"
                )
            );
        }

        // The property: every commit without a verdict is submitted at least once in as
        // many stops as there are such commits, whatever the judge does. Here it never
        // gives a verdict, for any commit or for its probe: the worst case, in which the
        // asking of a stop ends after three asks that failed (those in their turn apart)
        // and no attempt of the others counts. Four commits are past five attempts, five
        // have fewer, six were never asked about.
        let total = 15;
        let mut known: Vec<String> = (0..4)
            .map(|n| format!("{}:n{}@{}:r", id(n), 5 + n, n % 2))
            .chain((4..9).map(|n| format!("{}:n1@1:r", id(n))))
            .collect();
        let mut last = vec![0_u64; total];
        for stop in 2..120_u64 {
            let (_, asks) = sorted_commits(&listed(total), &known);
            let mut failed = 0;
            for ask in asks {
                if failed == FIRST_ASKS {
                    break;
                }
                failed += usize::from(!ask.in_turn());
                assert!(
                    stop - last[ask.at] <= u64::try_from(total).unwrap() || last[ask.at] == 0,
                    "commit {} waited {} stops",
                    ask.at,
                    stop - last[ask.at]
                );
                last[ask.at] = stop;
                let attempts = ask.attempts.unwrap_or(0) + u32::from(ask.in_turn());
                let line = format!("{}:n{attempts}@{stop}:r", id(ask.at));
                match known
                    .iter_mut()
                    .find(|entry| entry.starts_with(&id(ask.at)))
                {
                    Some(entry) => *entry = line,
                    None => known.push(line),
                }
            }
            // nobody is waiting longer than the number of commits without a verdict
            if stop > 20 {
                for (at, asked) in last.iter().enumerate() {
                    assert!(
                        stop - asked < u64::try_from(total).unwrap(),
                        "at stop {stop} commit {at} was last asked about at stop {asked}"
                    );
                }
            }
        }
        // and of the four that take turns, each once in four stops
        let turns: Vec<u64> = last[..4].to_vec();
        assert!(turns.iter().all(|asked| 119 - asked < 4), "{turns:?}");
    }

    /// The rule of the hooks for what a stop names beside its verdict: each kind of thing in
    /// the project that is not judged is said at the first stop in full, at the second and
    /// the third in the line that counts such things, for as long as it is true, and no more
    /// when it is no longer true. Each was said once in a session and then never again.
    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "one rule, each kind of thing a stop names beside its verdict"
    )]
    fn what_is_not_judged_is_said_at_every_stop_while_it_is_true() {
        const ROOM: usize = 8_000;
        // A note that is new at a stop with no room for it is counted there, by its name, and
        // is not remembered as said: the first stop that has room says it in full (sixth
        // independent review of the hooks, HR6-L-4: it was cut to nothing and never said).
        let mut crowded = State::default();
        let note = || Note {
            name: "the nested repository \"vendor\"".to_owned(),
            text: "In full: vendor.".to_owned(),
            standing: 0,
        };
        for room in [0, COUNT_LINE, COUNT_LINE + 16] {
            assert_eq!(
                notes_of(&mut crowded, vec![note()], room),
                ["1 thing in this project is not judged: the nested repository \"vendor\"."],
                "{room}"
            );
            assert!(crowded.noticed.is_empty(), "{room}");
        }
        assert_eq!(
            notes_of(&mut crowded, vec![note()], COUNT_LINE + 17),
            ["In full: vendor."]
        );
        assert_eq!(crowded.noticed.len(), 1);
        // a name on the counting line is bounded
        let long = Note {
            name: "n".repeat(1_000),
            text: "t".repeat(9_000),
            standing: 0,
        };
        let said = notes_of(&mut State::default(), vec![long], ROOM);
        assert_eq!(said.len(), 1);
        assert!(said[0].chars().count() < COUNT_LINE, "{}", said[0].len());
        let kinds = [
            "the nested repository \"vendor\"",
            "the working tree \"inner\" inside it",
            "the repository \"/other\" the session is in",
            "the submodule \"sub\"",
            "the working tree \"/wt\"",
            "the working tree \"/gone\" (gone)",
            "2 more working trees that were not looked at",
            "the files of this repository, which has no commit yet",
        ];
        for name in kinds {
            let note = || Note {
                name: name.to_owned(),
                text: format!("In full: {name}."),
                standing: 0,
            };
            let mut state = State {
                project: Some(project(Base::Commit(COMMIT.to_owned()))),
                ..State::default()
            };
            assert_eq!(
                notes_of(&mut state, vec![note()], ROOM),
                [format!("In full: {name}.")]
            );
            for stop in 2..=3 {
                assert_eq!(
                    notes_of(&mut state, vec![note()], ROOM),
                    [format!("1 thing in this project is not judged: {name}.")],
                    "stop {stop}"
                );
            }
            assert_eq!(notes_of(&mut state, Vec::new(), ROOM), Vec::<String>::new());
        }
        // a start that no session-start hook recorded is true of the whole session
        let mut late = State {
            project: Some(Project {
                by_start: false,
                ..project(Base::Commit(COMMIT.to_owned()))
            }),
            ..State::default()
        };
        assert_eq!(notes_of(&mut late, Vec::new(), ROOM), [hook::FIRST_STOP]);
        for _ in 2..=3 {
            assert_eq!(
                notes_of(&mut late, Vec::new(), ROOM),
                [
                    "1 thing in this project is not judged: what was committed before this session's first hook."
                ]
            );
        }
        // many at once: a few in full, the others counted, and the count is of all of them
        let many = || -> Vec<Note> {
            (0..40)
                .map(|n| Note {
                    name: format!("n{n:02}"),
                    text: format!("In full: n{n:02}."),
                    standing: 0,
                })
                .collect()
        };
        let mut state = State::default();
        for stop in 0..10 {
            let said = notes_of(&mut state, many(), ROOM);
            let full = said
                .iter()
                .filter(|line| line.starts_with("In full: "))
                .count();
            // (what a session remembers is bounded: past the bound a thing is counted)
            let room = NOTICED_CAP.saturating_sub(NOTES_IN_FULL * stop);
            assert_eq!(full, room.min(NOTES_IN_FULL), "stop {stop}");
            let counted = 40 - full;
            assert!(
                said.last().is_some_and(|line| line.starts_with(&format!(
                    "{counted} things in this project are not judged: "
                )) && line
                    .ends_with(&format!(", and {} more.", counted - 3))),
                "stop {stop}: {said:?}"
            );
        }
    }

    /// A mutation run left the bound of what is saved alive at the byte. A state whose text is
    /// exactly the cap is saved and read back; one byte more is not written, so no hook leaves
    /// a file the next one refuses.
    #[test]
    fn a_state_of_exactly_the_cap_is_saved_and_one_byte_more_is_not() {
        let cap = usize::try_from(STATE_CAP).unwrap();
        let sized = |len: usize| {
            let mut state = State {
                project: Some(project(Base::NoCommit)),
                ..State::default()
            };
            let rest = state_text(&state).len() - "/r".len();
            if let Some(project) = &mut state.project {
                project.top = "p".repeat(len - rest);
            }
            state
        };
        let scratch = deep_diff_forge_testkit::Scratch::new("hook-state-cap");
        let file = scratch.join("session.json");
        let exact = sized(cap);
        assert_eq!(state_text(&exact).len(), cap);
        save(&file, &exact).unwrap();
        assert_eq!(load(&file), Ok(Some(exact)));
        let over = sized(cap + 1);
        assert_eq!(
            save(&file, &over),
            Err("its state is larger than it reads back".to_owned())
        );
        assert_eq!(std::fs::metadata(&file).unwrap().len(), STATE_CAP);
    }

    /// A mutation run left the bound of the reported keys alive: no test filled it. The keys
    /// are kept while they fit; at the first that do not, the older ones are forgotten.
    #[test]
    fn the_keys_of_said_findings_are_kept_up_to_their_bound() {
        let keys = |from: usize, count: usize| -> Vec<String> {
            (from..from + count).map(|n| format!("k{n:05}")).collect()
        };
        let mut reported: BTreeSet<String> = ["a".to_owned()].into();
        keep_said(&mut reported, vec!["b".to_owned()]);
        assert_eq!(reported.len(), 2, "a few keys were forgotten");
        // many keys, and three more that still fit beside them
        let mut reported: BTreeSet<String> = keys(0, REPORTED_CAP / 2).into_iter().collect();
        keep_said(&mut reported, keys(REPORTED_CAP, 3));
        assert_eq!(reported.len(), REPORTED_CAP / 2 + 3);
        // exactly full is not over
        let mut reported: BTreeSet<String> = keys(0, REPORTED_CAP - 1).into_iter().collect();
        keep_said(&mut reported, keys(REPORTED_CAP, 1));
        assert_eq!(reported.len(), REPORTED_CAP);
        // one more than fits: the ones before are forgotten, these are kept
        keep_said(&mut reported, keys(2 * REPORTED_CAP, 1));
        assert_eq!(
            reported.into_iter().collect::<Vec<_>>(),
            keys(2 * REPORTED_CAP, 1)
        );
    }

    /// A mutation run left the bound of the marks alive: what an advice hook says when no
    /// state can remember it is marked by a file, and one repository keeps [`MARKS_KEPT`] of
    /// them. Up to that many, a reason is said and marked; past it nothing is said.
    #[test]
    fn a_reason_is_said_beside_the_state_while_there_is_room_for_its_mark() {
        let scratch = deep_diff_forge_testkit::Scratch::new("hook-marks");
        let own = scratch.join("own");
        std::fs::create_dir(&own).unwrap();
        for n in 0..MARKS_KEPT - 1 {
            std::fs::write(own.join(format!("said-x-{n}")), "").unwrap();
        }
        let marks = || std::fs::read_dir(&own).unwrap().count();
        // the last two marks there is room for
        assert!(said_beside(Some(&own), "s", "one").is_some());
        assert!(said_beside(Some(&own), "s", "two").is_some());
        assert_eq!(marks(), MARKS_KEPT + 1);
        // said before: not again; and no room for another
        assert_eq!(said_beside(Some(&own), "s", "two"), None);
        assert_eq!(said_beside(Some(&own), "s", "three"), None);
        assert_eq!(marks(), MARKS_KEPT + 1);
        assert_eq!(said_beside(None, "s", "four"), None);
    }

    /// A mutation run left three of the answers of [`own_directory`] alive: which failure is
    /// which. A directory that cannot be looked at is "could not be read"; one that is not
    /// there and cannot be made is "could not be made"; neither is "a link or a file stands
    /// there".
    #[test]
    fn a_directory_that_cannot_be_read_or_made_is_said_as_that() {
        use std::os::unix::fs::PermissionsExt as _;
        let scratch = deep_diff_forge_testkit::Scratch::new("hook-own-directory");
        let parent = scratch.join("parent");
        std::fs::create_dir(&parent).unwrap();
        let allow = |mode: u32| {
            std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(mode)).unwrap();
        };
        allow(0o500);
        // not as root, who reads and writes whatever the mode says
        if std::fs::write(parent.join("probe"), "x").is_err() {
            let made = own_directory(&parent.join("own"));
            assert!(
                made.as_ref()
                    .is_err_and(|why| why.starts_with("its directory could not be made: ")),
                "{made:?}"
            );
            allow(0o000);
            let read = own_directory(&parent.join("own"));
            assert!(
                read.as_ref()
                    .is_err_and(|why| why.starts_with("its directory could not be read: ")),
                "{read:?}"
            );
        }
        allow(0o700);
        assert_eq!(own_directory(&parent.join("own")), Ok(()));
        assert_eq!(own_directory(&parent.join("own")), Ok(()));
        std::fs::write(parent.join("file"), "x").unwrap();
        assert!(
            own_directory(&parent.join("file"))
                .is_err_and(|why| why.contains("is not a directory of its own"))
        );
    }

    /// Third independent review of the hooks, HR3-H-2: the commits that were there when a
    /// session began are kept in a file of their own, with no bound of 4,096, and the state
    /// names that file by its SHA-256 and its number of lines. A file that is missing, cut
    /// short, longer or changed is not believed: with other tips, other commits would be the
    /// session's.
    #[test]
    fn a_tips_file_is_believed_only_as_the_state_names_it() {
        use std::os::unix::fs::PermissionsExt as _;
        let scratch = deep_diff_forge_testkit::Scratch::new("hook-tips");
        let file = tips_file(&scratch.join("session-x.json"));
        assert_eq!(file, scratch.join("session-x.tips"));
        let tips = vec!["a".repeat(40), "b".repeat(64)];
        let (text, digest) = tips_text(&tips);
        assert_eq!(text, format!("{}\n{}\n", "a".repeat(40), "b".repeat(64)));
        let named = Project {
            tips_sha256: digest,
            tips_count: 2,
            no_baseline: None,
            ..project(Base::NoCommit)
        };
        assert!(
            load_tips(&file, &named).is_err_and(
                |why| why.contains("cannot be opened") && why.contains("start a new session")
            ),
            "a missing tips file was read as no tips"
        );
        write_private(&file, &text).unwrap();
        assert_eq!(
            std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(load_tips(&file, &named), Ok(tips));
        for other in [
            format!("{}\n", "a".repeat(40)),
            format!("{text}{}\n", "c".repeat(40)),
            text.replace('b', "c"),
            String::new(),
        ] {
            std::fs::write(&file, &other).unwrap();
            assert!(
                load_tips(&file, &named)
                    .is_err_and(|why| why.contains("is not the list this session recorded")),
                "{other:?}"
            );
        }
        // a count that does not fit the text is not believed either
        std::fs::write(&file, &text).unwrap();
        let miscounted = Project {
            tips_count: 3,
            no_baseline: None,
            ..named.clone()
        };
        assert!(load_tips(&file, &miscounted).is_err());
        // A mutation run left the bounds of a list file alive. A list of 30,000 commits (over
        // a mebibyte) is read whole; a file of exactly the cap is read (and is then not the
        // list); one byte more, or what is no regular file, is not a file the hooks wrote.
        let many: Vec<String> = (0..30_000).map(|n| format!("{n:040x}")).collect();
        let (long, digest) = tips_text(&many);
        assert!(long.len() > 1024 * 1024 + 67);
        write_private(&file, &long).unwrap();
        let big = Project {
            tips_sha256: digest,
            tips_count: many.len(),
            no_baseline: None,
            ..project(Base::NoCommit)
        };
        assert_eq!(load_tips(&file, &big).map(|tips| tips.len()), Ok(30_000));
        let sized = |len: u64| {
            std::fs::File::create(&file).unwrap().set_len(len).unwrap();
            load_tips(&file, &named).unwrap_err()
        };
        assert!(sized(TIPS_CAP).contains("is not the list this session recorded"));
        assert!(sized(TIPS_CAP + 1).contains("is not a file the hooks wrote"));
        std::fs::remove_file(&file).unwrap();
        std::fs::create_dir(&file).unwrap();
        assert!(
            load_tips(&file, &named)
                .is_err_and(|why| why.contains("is not a file the hooks wrote")),
        );
        std::fs::remove_dir(&file).unwrap();
        // a link is not followed
        std::fs::write(scratch.join("elsewhere"), text).unwrap();
        std::os::unix::fs::symlink(scratch.join("elsewhere"), &file).unwrap();
        assert!(
            load_tips(&file, &named).is_err(),
            "a link was followed to a tips file"
        );
    }

    /// The lines of the file of judged commits: an id, and an answer in one of its forms. A
    /// held or failed commit keeps the names of its rules (round 15, HR6-M-2), and a commit
    /// with no verdict any number of attempts (round 15, VD4R14-M-1: the count ended at 5).
    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "one list: every form of a line, and what is kept of a reason"
    )]
    fn a_line_of_the_judged_file_is_an_id_and_an_answer() {
        let id = "a".repeat(40);
        for line in [
            "p",
            "h",
            "f",
            "h:no-silent-suppression",
            "h:|note patch_unverifiable unsupported file mode 160000",
            "f:no-secrets",
            "f:no-secrets,keep-the-tests;no-silent-suppression",
            "f:no-secrets,+3;quiet|expired waiver 0 for rule quiet (+1 more)",
            "f:",
            "n0@1:r",
            "n5@12:the ranker: timed out",
            "n6@18446744073709551615:r",
            "n4294967295@2:r",
            "x:shallow",
        ] {
            assert!(is_judged_entry(&format!("{id}:{line}")), "{line}");
        }
        for line in [
            format!("{id}:u"),
            // no number of a stop: the form of the build before
            format!("{id}:n5:r"),
            format!("{id}:n4294967296@1:r"),
            format!("{id}:n1@18446744073709551616:r"),
            format!("{id}:n-1@1:r"),
            format!("{id}:n+1@1:r"),
            format!("{id}:n1@+1:r"),
            format!("{id}:n1@:r"),
            format!("{id}:n@1:r"),
            format!("{id}:n1@1"),
            format!("{id}:nx@1:r"),
            format!("{id}:y:r"),
            id.clone(),
            "HEAD:p".to_owned(),
            format!("{}:p", &id[..39]),
        ] {
            assert!(!is_judged_entry(&line), "{line}");
        }
        // what is kept of a commit's reason, as its line keeps it and gives it back
        let rule = |id: &str, held: bool| hook::RuleOutcome {
            id: id.to_owned(),
            why: "w".to_owned(),
            held,
            violations: Vec::new(),
        };
        let both = [
            rule("no-secrets", false),
            rule("a:b,c;d|e\n", false),
            rule("quiet", true),
        ];
        assert_eq!(kept_reason(&both, &[], true), "no-secrets,a:b_c_d_e_;quiet");
        assert_eq!(kept_reason(&both[2..], &[], false), "quiet");
        assert_eq!(kept_reason(&[], &[], true), "");
        let names = |kept: &str, all_held: bool| -> Vec<(String, bool)> {
            reason_kept(kept, all_held)
                .0
                .into_iter()
                .map(|rule| (rule.id, rule.held))
                .collect()
        };
        assert_eq!(
            names("no-secrets,a:b_c_d_e_;quiet", false),
            [
                ("no-secrets".to_owned(), false),
                ("a:b_c_d_e_".to_owned(), false),
                ("quiet".to_owned(), true)
            ]
        );
        assert!(names("quiet", true).iter().all(|(_, held)| *held));
        assert_eq!(reason_kept("", false), (Vec::new(), Vec::new()));
        // A hold for a note and a failure by an expired waiver are no rule: the first thing
        // the judge said is kept, with the number of the others. Nothing was kept, and the
        // later stops of the session named the commit with no reason (HR7-L-6).
        let said = [
            "note patch_unverifiable: unsupported file mode 160000 (a submodule); coverage cannot be verified\n".to_owned(),
            "note other: x".to_owned(),
        ];
        let kept = kept_reason(&[], &said, false);
        assert_eq!(
            kept,
            "|note patch_unverifiable: unsupported file mode 160000 (a submodule); coverage cannot be verified (+1 more)"
        );
        assert_eq!(
            reason_kept(&kept, true),
            (Vec::new(), vec![kept[1..].to_owned()])
        );
        assert_eq!(
            kept_reason(&both[..1], &said[..1], true).split('|').next(),
            Some("no-secrets")
        );
        // bounded: eight names of each kind and the number of the others, sixty-four
        // characters of each name, a hundred of what the judge said
        let many: Vec<hook::RuleOutcome> = (0..20)
            .map(|n| rule(&format!("{n}{}", "r".repeat(100)), false))
            .collect();
        let line = kept_reason(&many, &["s".repeat(500)], true);
        let (rules, words) = line.split_once('|').unwrap();
        assert_eq!(rules.split(',').count(), RULES_KEPT + 1);
        assert_eq!(rules.split(',').next_back(), Some("+12"));
        assert!(
            rules
                .split(',')
                .take(RULES_KEPT)
                .all(|name| name.chars().count() == RULE_NAME_KEPT)
        );
        assert_eq!(words.len(), SAID_KEPT_CHARS);
        assert_eq!(names(rules, false).len(), RULES_KEPT + 1);
    }

    /// Third independent review of the hooks, HR3-M-3, and the fourth, HR4-L-3. A failure of
    /// an uncommitted change is kept while each stop finds it, and forgotten when one does
    /// not, so it blocks again when it comes back; it is kept when the change could not be
    /// judged. A commit's answer is added to the session's list, which forgets none: what
    /// the state says of the list is a digest and a count, and a state that names a list
    /// without its digest is no state.
    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "one sequence of stops: what each remembers and forgets"
    )]
    fn a_failure_that_is_gone_is_forgotten_and_no_answer_is() {
        let lines = |pairs: &[(&str, usize)]| -> Lines {
            pairs
                .iter()
                .map(|(key, count)| ((*key).to_owned(), *count))
                .collect()
        };
        // a tree whose uncommitted change was read, with nothing in its index
        let looked = |id: &str, tree: Option<&[(&str, usize)]>| Looked {
            id: id.to_owned(),
            tree: tree.map(lines),
            index: Some(Lines::new()),
            own: Lines::new(),
            staged: Lines::new(),
        };
        let stop = |trees: Vec<Looked>| Judging {
            trees,
            ..Judging::default()
        };
        let mut state = State {
            project: Some(project(Base::Commit(COMMIT.to_owned()))),
            ..State::default()
        };
        let mut judged = Vec::new();
        // (what is remembered, in a fixed order: a stop writes what it saw last)
        let kept = |state: &State| {
            let mut all = state.blocked.clone();
            all.sort();
            all
        };
        let there = stop(vec![looked("t1", Some(&[("a", 1), ("b", 2)]))]);
        remember(&mut state, &there, &mut judged);
        remember(&mut state, &there, &mut judged);
        assert_eq!(kept(&state), ["t1/u/a:1", "t1/u/b:2"]);
        // no verdict on the change: nothing is known of it, and nothing is forgotten
        remember(&mut state, &stop(vec![looked("t1", None)]), &mut judged);
        assert_eq!(
            kept(&state),
            ["t1/u/a:1", "t1/u/b:2"],
            "forgotten without a verdict"
        );
        assert!(judged.is_empty(), "{judged:?}");
        // Sixth independent review of the hooks, HR6-M-1: a stop that did not look at the
        // tree forgets nothing of it, whatever it found in another tree; and a failure in
        // that other tree is remembered beside it, the same line as its own.
        remember(
            &mut state,
            &stop(vec![looked("t2", Some(&[("a", 1)]))]),
            &mut judged,
        );
        remember(&mut state, &Judging::default(), &mut judged);
        assert_eq!(kept(&state), ["t1/u/a:1", "t1/u/b:2", "t2/u/a:1"]);
        // Seventh, HR7-M-1: one line is repaired and the count of the other falls. What was
        // seen is what is remembered: the line that is gone blocks when it comes back, and
        // the other blocks when its count grows again.
        remember(
            &mut state,
            &stop(vec![looked("t1", Some(&[("b", 1)]))]),
            &mut judged,
        );
        assert_eq!(kept(&state), ["t1/u/b:1", "t2/u/a:1"]);
        // What is staged. Its own line is remembered ("k": the tree was repaired, the index
        // was not), and stays remembered while the index holds it, also at a stop where the
        // tree's failure has the line again and the staged change has no line of its own; it
        // goes when the index no longer holds it.
        let staged = |index: &[(&str, usize)], own: &[(&str, usize)]| {
            stop(vec![Looked {
                id: "t1".to_owned(),
                tree: Some(lines(&[("b", 1)])),
                index: Some(lines(index)),
                own: lines(own),
                staged: lines(own),
            }])
        };
        remember(&mut state, &staged(&[("k", 2)], &[("k", 2)]), &mut judged);
        assert_eq!(kept(&state), ["t1/s/k:2", "t1/u/b:1", "t2/u/a:1"]);
        remember(&mut state, &staged(&[("k", 1), ("b", 1)], &[]), &mut judged);
        assert_eq!(kept(&state), ["t1/k/k:1", "t1/u/b:1", "t2/u/a:1"]);
        // (and is the staged change's own again when the tree is repaired again)
        remember(&mut state, &staged(&[("k", 1)], &[("k", 1)]), &mut judged);
        assert_eq!(kept(&state), ["t1/s/k:1", "t1/u/b:1", "t2/u/a:1"]);
        remember(&mut state, &staged(&[("k", 1), ("b", 1)], &[]), &mut judged);
        // (no verdict on what is staged: kept as it is)
        let unread = stop(vec![Looked {
            index: None,
            ..looked("t1", Some(&[("b", 1)]))
        }]);
        remember(&mut state, &unread, &mut judged);
        assert_eq!(kept(&state), ["t1/k/k:1", "t1/u/b:1", "t2/u/a:1"]);
        remember(&mut state, &staged(&[], &[]), &mut judged);
        assert_eq!(kept(&state), ["t1/u/b:1", "t2/u/a:1"]);
        // A tree found with nothing uncommitted has no such failure left, and neither has a
        // tree that Git no longer lists: an entry of a removed tree stayed for the rest of
        // the session (seventh, HR7-H-1). A tree this stop looked at is kept whatever the
        // list says.
        let listing = |clean: &[&str], listed: &[&str]| Judging {
            clean: clean.iter().map(|id| (*id).to_owned()).collect(),
            listed: Some(listed.iter().map(|id| (*id).to_owned()).collect()),
            ..Judging::default()
        };
        remember(&mut state, &listing(&[], &["t1", "t2", "t9"]), &mut judged);
        assert_eq!(kept(&state), ["t1/u/b:1", "t2/u/a:1"]);
        remember(&mut state, &listing(&["t2"], &["t1", "t2"]), &mut judged);
        assert_eq!(kept(&state), ["t1/u/b:1"]);
        let unlisted = Judging {
            trees: vec![looked("t3", Some(&[("c", 1)]))],
            ..listing(&[], &["t9"])
        };
        remember(&mut state, &unlisted, &mut judged);
        assert_eq!(kept(&state), ["t3/u/c:1"]);
        // gone: forgotten. The failure of a commit is remembered with the commit, not here.
        let gone = Judging {
            trees: vec![looked("t3", Some(&[]))],
            commits: vec![
                ("c".repeat(40), "f:no-secrets".to_owned()),
                ("d".repeat(40), "n1@1:the ranker did not answer".to_owned()),
            ],
            ..Judging::default()
        };
        remember(&mut state, &gone, &mut judged);
        assert!(state.blocked.is_empty(), "{:?}", state.blocked);
        assert_eq!(
            judged,
            [
                format!("{}:f:no-secrets", "c".repeat(40)),
                format!("{}:n1@1:the ranker did not answer", "d".repeat(40))
            ]
        );
        assert!(judged.iter().all(|entry| is_judged_entry(entry)));
        // many answers: none is forgotten
        let many = Judging {
            commits: (0..5_000)
                .map(|n| (format!("{n:040x}"), "p".to_owned()))
                .collect(),
            ..Judging::default()
        };
        remember(&mut state, &many, &mut judged);
        assert_eq!(judged.len(), 5_002);
        assert_eq!(judged[0], format!("{}:f:no-secrets", "c".repeat(40)));
        // what a state says of the list: a count with its digest, or neither
        state.judged_sha256 = tips_text(&judged).1;
        state.judged_count = judged.len();
        assert_eq!(state_from(&state_text(&state)), Ok(state.clone()));
        for (digest, count) in [(String::new(), 1), ("d".repeat(64), 0), ("x".repeat(64), 2)] {
            let spoiled = State {
                judged_sha256: digest,
                judged_count: count,
                ..state.clone()
            };
            assert!(state_from(&state_text(&spoiled)).is_err(), "{count}");
        }
    }

    /// [`seen`] for a stop that read the tree and its index: the tree's verdict (`None`: no
    /// change), and the index's (`None`: it holds no change, or with `same` the tree's).
    fn seen_of(
        id: &str,
        top: &str,
        tree: Option<Judged>,
        index: Option<Judged>,
        same: bool,
        blocked: &[String],
    ) -> (Looked, Vec<TreeJudged>) {
        let index = match index {
            _ if same => Reading::Found(Staged::Same),
            None => Reading::Nothing,
            Some(judged) => Reading::Found(Staged::Other(judged)),
        };
        seen(
            id,
            top,
            tree.map_or(Reading::Nothing, Reading::Found),
            index,
            blocked,
        )
    }

    /// A failure of `files` failing files, the first `first`, each with `count` items: one line
    /// for each file.
    fn failing_files(first: usize, files: usize, count: usize) -> Judged {
        Judged::Fail {
            rules: vec![hook::RuleOutcome {
                id: "no-secrets".to_owned(),
                why: "because".to_owned(),
                held: false,
                violations: (first..first + files)
                    .map(|n| ("secret_like".to_owned(), format!("f{n}.rs"), count))
                    .collect(),
            }],
            said: Vec::new(),
        }
    }

    /// One stop made from inside the tree `id`, whose uncommitted change got `tree` and whose
    /// index holds no other change: whether it blocks, and what its remembering cost.
    fn stop_in(state: &mut State, id: &str, tree: Option<Judged>, cap: usize) -> (bool, Kept) {
        let (looked, units) = seen_of(id, "/p", tree, None, false, &state.blocked);
        let blocks = matches!(
            hook::decide_stop_trees(&units, &state.blocked, "r"),
            StopDecision::Block { .. }
        );
        let judging = Judging {
            trees: vec![looked],
            ..Judging::default()
        };
        let kept = remember_within(state, &judging, &mut Vec::new(), cap);
        (blocks, kept)
    }

    /// Review V4 round 16, VD4R16-M-1, as reproduced: nine working trees with a failure of 256
    /// lines each. The state kept 2,048 lines and cut the oldest from the front, in silence:
    /// the ninth tree pushed out the first one's, and the first, unchanged, blocked a second
    /// time. Now the trees that do not fit line by line are remembered by their mark: they
    /// stand as they are, block as a whole after any change, and nothing is said to be lost
    /// because nothing is.
    #[test]
    fn nine_large_failures_are_all_remembered() {
        let mut state = State::default();
        let failure = |tree: usize| Some(failing_files(tree * 1_000, hook::LINES_KEPT, 1));
        for tree in 0..9 {
            let (blocks, kept) =
                stop_in(&mut state, &format!("t{tree}"), failure(tree), BLOCKED_CAP);
            assert!(blocks, "tree {tree} is new");
            assert_eq!(kept, Kept::default(), "tree {tree}");
            assert!(state.blocked.len() <= BLOCKED_CAP);
        }
        // the two that were remembered longest ago are one mark each; the others line by line
        let entries = |state: &State, tree: usize| {
            hook::remembered(&state.blocked, &format!("t{tree}/u")).len()
        };
        assert_eq!(
            (0..9).map(|tree| entries(&state, tree)).collect::<Vec<_>>(),
            [1, 1, 256, 256, 256, 256, 256, 256, 256]
        );
        // every one of them stands when a stop comes back to it, unchanged
        for tree in 0..9 {
            let (blocks, kept) =
                stop_in(&mut state, &format!("t{tree}"), failure(tree), BLOCKED_CAP);
            assert!(!blocks, "tree {tree} blocked a second time, unchanged");
            assert_eq!(kept, Kept::default(), "tree {tree}");
            assert!(state.blocked.len() <= BLOCKED_CAP);
        }
        // A tree that is remembered by its mark blocks as a whole when one part of it is
        // repaired, and says why; one that is remembered line by line stands then.
        let marked = (0..9)
            .find(|tree| entries(&state, *tree) == 1)
            .expect("a tree in its short form");
        let lined = (0..9)
            .find(|tree| entries(&state, *tree) == 256)
            .expect("a tree kept line by line");
        let part = |tree: usize| Some(failing_files(tree * 1_000, hook::LINES_KEPT - 1, 1));
        let (looked, units) = seen_of(
            &format!("t{marked}"),
            "/p",
            part(marked),
            None,
            false,
            &state.blocked,
        );
        match hook::decide_stop_trees(&units, &state.blocked, "r") {
            StopDecision::Block { reason } => assert!(
                reason.contains("has blocked before and is not what it was")
                    && !reason.contains("New at this stop"),
                "{reason}"
            ),
            StopDecision::Allow { message } => panic!("a changed mark stood: {message:?}"),
        }
        assert_eq!(looked.tree.as_ref().map(Lines::len), Some(255));
        assert!(!stop_in(&mut state, &format!("t{lined}"), part(lined), BLOCKED_CAP).0);
    }

    /// What [`within_bound`] does, on a small bound. A unit this stop did not look at goes
    /// into its short form before one it looked at, the largest first; the lines kept for a
    /// staged change go with its lines; a unit of one line has no shorter form; and when
    /// every unit is one entry and they are still too many, units this stop did not look at
    /// are forgotten from the front and named, which is the one thing a stop must say.
    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "one function on a small bound: each thing it does, in the order it does them"
    )]
    fn what_is_remembered_is_bounded_and_what_is_lost_is_named() {
        let unit = |tag: &str, lines: usize| -> Vec<String> {
            (0..lines)
                .map(|n| hook::remembered_line(tag, &format!("{n:024x}"), 1))
                .collect()
        };
        let looked =
            |ids: &[&str]| -> BTreeSet<String> { ids.iter().map(|id| (*id).to_owned()).collect() };
        let sizes = |blocked: &[String]| -> Vec<(String, usize)> {
            let mut sizes: Vec<(String, usize)> = Vec::new();
            for entry in blocked {
                let unit = entry.rsplit_once('/').unwrap().0.to_owned();
                match sizes.iter_mut().find(|(known, _)| *known == unit) {
                    Some((_, count)) => *count += 1,
                    None => sizes.push((unit, 1)),
                }
            }
            sizes
        };
        let named = |pairs: &[(&str, usize)]| -> Vec<(String, usize)> {
            pairs
                .iter()
                .map(|(unit, count)| ((*unit).to_owned(), *count))
                .collect()
        };
        // under the bound: nothing is touched
        let mut blocked = [unit("a/u", 5), unit("b/u", 5)].concat();
        let before = blocked.clone();
        assert_eq!(
            within_bound(&mut blocked, &looked(&["b"]), 10),
            Kept::default()
        );
        assert_eq!(blocked, before);
        // one over: the largest unit that was not looked at becomes its mark, in its place
        let mut blocked = [
            unit("a/u", 3),
            unit("b/u", 6),
            unit("c/u", 9),
            unit("d/u", 1),
        ]
        .concat();
        assert_eq!(
            within_bound(&mut blocked, &looked(&["c"]), 18),
            Kept::default()
        );
        assert_eq!(
            sizes(&blocked),
            named(&[("a/u", 3), ("b/u", 1), ("c/u", 9), ("d/u", 1)])
        );
        let mark = hook::remembered(&blocked, "b/u");
        let lines: Lines = hook::remembered(&unit("b/u", 6), "b/u")
            .into_iter()
            .map(|(key, count)| (key.to_owned(), count))
            .collect();
        let (key, count) = hook::mark_of(&lines);
        assert_eq!(
            mark.into_iter().collect::<Vec<_>>(),
            [(key.as_str(), count)]
        );
        // the unit this stop looked at goes last, and is named when it goes
        let mut blocked = [unit("a/u", 3), unit("c/u", 9)].concat();
        let kept = within_bound(&mut blocked, &looked(&["c"]), 4);
        assert_eq!(sizes(&blocked), named(&[("a/u", 1), ("c/u", 1)]));
        assert_eq!(
            kept,
            Kept {
                short: vec!["c/u".to_owned()],
                ..Kept::default()
            }
        );
        // What is kept for a staged change is of that change, and is shortened with it:
        // into one mark of all that has blocked for it, the own lines and the kept ones
        // (ninth independent review of the hooks, HR9-L-1: the kept lines were let go).
        let kept_lines: Vec<String> = (10..13)
            .map(|n| hook::remembered_line("a/k", &format!("{n:024x}"), 1))
            .collect();
        let mut blocked = [unit("a/s", 4), kept_lines, unit("b/u", 2)].concat();
        let all: Lines = hook::remembered(&blocked, "a/s")
            .into_iter()
            .map(|(key, count)| (key.to_owned(), count))
            .collect();
        assert_eq!(all.len(), 7);
        // (a staged change that is shortened is named, whichever tree the stop is in: it
        // answers only for exactly what it was from now on, which the stop says)
        assert_eq!(
            within_bound(&mut blocked, &looked(&["b"]), 5),
            Kept {
                as_a_whole: vec!["a/s".to_owned()],
                ..Kept::default()
            }
        );
        assert_eq!(
            sizes(&blocked),
            named(&[("a/s", 1), ("a/k", 1), ("b/u", 2)])
        );
        let (key, count) = hook::mark_of(&all);
        assert_eq!(
            hook::remembered_as(&blocked, "a/k")
                .into_iter()
                .collect::<Vec<_>>(),
            [(key.as_str(), count)]
        );
        // Only where the own lines are a mark already is there nothing to make that mark
        // of: then the kept lines go, and the unit is named for the stop to say so.
        let mut blocked = [
            vec![hook::remembered_line(
                "a/s",
                &format!("all{}", "0".repeat(21)),
                300,
            )],
            unit("a/k", 4),
            unit("b/u", 2),
        ]
        .concat();
        assert_eq!(
            within_bound(&mut blocked, &looked(&["b"]), 5),
            Kept {
                given_up: vec!["a/s".to_owned()],
                as_a_whole: vec!["a/s".to_owned()],
                ..Kept::default()
            }
        );
        assert_eq!(sizes(&blocked), named(&[("a/s", 1), ("b/u", 2)]));
        // the last resort: every unit is one entry, and they are more than the bound
        let mut blocked = [
            unit("a/u", 2),
            unit("b/u", 1),
            unit("c/u", 5),
            unit("d/s", 1),
            unit("e/u", 3),
        ]
        .concat();
        let kept = within_bound(&mut blocked, &looked(&["c"]), 3);
        assert_eq!(
            sizes(&blocked),
            named(&[("c/u", 1), ("d/s", 1), ("e/u", 1)])
        );
        assert_eq!(
            kept,
            Kept {
                lost: vec!["a/u".to_owned(), "b/u".to_owned()],
                short: vec!["c/u".to_owned()],
                ..Kept::default()
            }
        );
        // what a stop says of it: both things, each with its number, at that stop
        let said = memory_notes(&kept);
        assert_eq!(said.len(), 2);
        assert!(
            said[0].contains("could not be kept for 2 changes")
                && said[0].contains("may block once more"),
            "{}",
            said[0]
        );
        assert!(
            said[1].contains("the failure of 1 change that this stop looked at")
                && said[1].contains("blocks again as a whole"),
            "{}",
            said[1]
        );
        assert!(said.iter().all(|note| note.chars().count() < 400));
        assert_eq!(memory_notes(&Kept::default()), Vec::<String>::new());
    }

    /// The property of review V4 round 16, VD4R16-M-1, and of the ninth independent review,
    /// HR9-M-1 and HR9-L-1, over generated sessions: many working trees with failures of one
    /// line to six hundred, uncommitted and staged in whole and in part, changed and
    /// repaired between stops that are made from the trees in no order, with a judge that
    /// sometimes gives no verdict, under the real bound and under small ones. Each stop is
    /// compared with a model that remembers every line and has no bound:
    ///
    /// - What the model blocks for, the hook blocks for: no new failing line is missed.
    /// - While nothing of a tree is remembered by a mark, the hook blocks for nothing else.
    /// - Where a mark is remembered, the hook may block beyond the model only for a change
    ///   that is not what the mark stood for: an uncommitted change that is not as the last
    ///   stop saw it; a staged change whose index is not as the last stop that looked saw
    ///   it (whatever the working tree did in between excuses nothing).
    /// - Where a stop has said that the memory of a tree could not be kept, that tree may
    ///   block beyond the model until it has nothing failing left.
    /// - What is remembered stays within the bound, and the state within its size.
    ///
    /// The first form of this test never staged a part of an unchanged failure, and called
    /// a tree unchanged only when tree and index were both as the last stop saw them: the
    /// two findings passed it.
    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "one property: the sessions are generated, then each stop is checked"
    )]
    fn an_unchanged_failure_blocks_twice_only_where_a_stop_said_so() {
        type Content = std::collections::BTreeMap<usize, usize>;
        #[derive(Default, Clone)]
        struct Place {
            tree: Content,
            index: Content,
            /// the model: the uncommitted lines the last stop with a verdict saw, and every
            /// staged line that has blocked and that the index has held since
            told_tree: Content,
            told_staged: Content,
            /// the uncommitted change, and what the index holds, as the last stop that
            /// could tell them saw them
            last_tree: Option<Content>,
            last_index: Option<Content>,
            /// a stop said that the memory of this tree could not be kept
            lost: bool,
        }
        let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
        let mut next = |below: usize| -> usize {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            usize::try_from(seed % u64::try_from(below).unwrap()).unwrap()
        };
        let judged = |content: &Content| -> Option<Judged> {
            (!content.is_empty()).then(|| Judged::Fail {
                rules: vec![hook::RuleOutcome {
                    id: "no-secrets".to_owned(),
                    why: "because".to_owned(),
                    held: false,
                    violations: content
                        .iter()
                        .map(|(file, count)| {
                            ("secret_like".to_owned(), format!("f{file}.rs"), *count)
                        })
                        .collect(),
                }],
                said: Vec::new(),
            })
        };
        // the lines of `now` that `then` does not have, or has with fewer items
        let beyond = |now: &Content, then: &Content| -> Content {
            now.iter()
                .filter(|(file, count)| then.get(file).is_none_or(|had| had < count))
                .map(|(file, count)| (*file, *count))
                .collect()
        };
        let marked = |blocked: &[String], tag: &str| {
            hook::remembered_as(blocked, tag)
                .keys()
                .any(|key| hook::is_mark(key))
        };
        let blocks = |unit: Option<&TreeJudged>, blocked: &[String]| {
            unit.is_some_and(|unit| {
                matches!(
                    hook::decide_stop_trees(std::slice::from_ref(unit), blocked, "r"),
                    StopDecision::Block { .. }
                )
            })
        };
        let (mut stops, mut shortened, mut lost, mut given_up, mut unread) = (0, 0, 0, 0, 0);
        let (mut stood, mut blocked_new, mut part_staged, mut beyond_model) = (0, 0, 0, 0);
        for session in 0..90 {
            // most sessions are small ones under a small bound; some are large, under the
            // real one
            let (cap, trees, largest) = match session % 9 {
                0 => (BLOCKED_CAP, 6 + next(14), 600),
                1..=4 => (40, 2 + next(8), 30),
                _ => (6, 2 + next(8), 12),
            };
            let mut state = State::default();
            let mut all: Vec<Place> = vec![Place::default(); trees];
            let mut file = 0;
            for _ in 0..(if cap == BLOCKED_CAP { 60 } else { 80 }) {
                let at = next(trees);
                // something happens in this tree, or nothing does
                {
                    let tree = &mut all[at];
                    match next(13) {
                        0 | 1 if tree.tree.is_empty() => {
                            let lines = match next(8) {
                                // the sizes around the number of lines kept one by one
                                0 if largest > 256 => [200, 256, 257, 600][next(4)],
                                1 | 2 => 1 + next(largest),
                                _ => 1 + next(largest / 4 + 1),
                            };
                            for _ in 0..lines {
                                file += 1;
                                tree.tree.insert(file, 1 + next(2));
                            }
                        }
                        0 => {
                            file += 1;
                            tree.tree.insert(file, 1);
                        }
                        1 => {
                            if let Some(count) = tree.tree.values_mut().next() {
                                *count += 1;
                            }
                        }
                        2 => {
                            let first = tree.tree.keys().next().copied();
                            if let Some(first) = first {
                                tree.tree.remove(&first);
                            }
                        }
                        3 => tree.index = tree.tree.clone(),
                        4 => tree.index.clear(),
                        5 => {
                            // repaired in the tree, the repair not staged
                            let newest = tree.tree.keys().next_back().copied();
                            if let Some(newest) = newest
                                && tree.index.contains_key(&newest)
                            {
                                tree.tree.remove(&newest);
                            }
                        }
                        6 if next(4) == 0 => tree.tree.clear(),
                        // a part of what fails in the tree is staged, as it is: `git add`
                        // of some of the files; then more of them at a later stop
                        7 | 8 => {
                            let more: Vec<(usize, usize)> = tree
                                .tree
                                .iter()
                                .filter(|(file, _)| !tree.index.contains_key(*file))
                                .map(|(file, count)| (*file, *count))
                                .collect();
                            if !more.is_empty() {
                                let part = 1 + next(more.len());
                                tree.index.extend(more.into_iter().take(part));
                                part_staged += 1;
                            }
                        }
                        // everything staged is repaired in the tree, or put back there
                        9 if next(3) == 0 => {
                            for staged in tree.index.keys() {
                                tree.tree.remove(staged);
                            }
                        }
                        10 if next(3) == 0 => {
                            let staged = tree.index.clone();
                            tree.tree.extend(staged);
                        }
                        // a part of what is staged is repaired in the tree, or is back there
                        11 => {
                            let staged: Vec<(usize, usize)> = tree
                                .index
                                .iter()
                                .map(|(file, count)| (*file, *count))
                                .collect();
                            if !staged.is_empty() {
                                let from = next(staged.len());
                                let part = 1 + next(staged.len() - from);
                                for (file, count) in staged.into_iter().skip(from).take(part) {
                                    if next(2) == 0 {
                                        tree.tree.remove(&file);
                                    } else {
                                        tree.tree.insert(file, count);
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
                let place = all[at].clone();
                let id = format!("t{at}");
                let no_verdict = || {
                    Some(Judged::Unjudged {
                        reason: "the judge did not answer".to_owned(),
                    })
                };
                // Tenth independent review of the hooks, HR10-M-1: a stop that could not
                // READ the tree (Git failed, a file could not be opened) has not read its
                // index either, whatever the index holds. It is not a stop that found
                // nothing: what is remembered of the tree is as it was after it.
                let not_read = next(14) == 0;
                let tree_read = !not_read && (place.tree.is_empty() || next(12) != 0);
                let same = !place.index.is_empty() && place.index == place.tree;
                let index_read = !not_read
                    && if same {
                        tree_read
                    } else {
                        place.index.is_empty() || next(12) != 0
                    };
                let why = || "Git did not answer".to_owned();
                let (looked, units) = if not_read {
                    seen(
                        &id,
                        "/p",
                        Reading::NotRead(why()),
                        Reading::NotRead(why()),
                        &state.blocked,
                    )
                } else if !index_read && !same && next(2) == 0 {
                    // (the tree was read and the index could not be)
                    seen(
                        &id,
                        "/p",
                        if tree_read {
                            judged(&place.tree).map_or(Reading::Nothing, Reading::Found)
                        } else {
                            Reading::Found(Judged::Unjudged { reason: why() })
                        },
                        Reading::NotRead(why()),
                        &state.blocked,
                    )
                } else {
                    seen_of(
                        &id,
                        "/p",
                        if tree_read {
                            judged(&place.tree)
                        } else {
                            no_verdict()
                        },
                        if same || place.index.is_empty() {
                            None
                        } else if index_read {
                            judged(&place.index)
                        } else {
                            no_verdict()
                        },
                        same,
                        &state.blocked,
                    )
                };
                let of_this_tree = |blocked: &[String]| -> Vec<String> {
                    let mut entries: Vec<String> = blocked
                        .iter()
                        .filter(|entry| tree_of(entry) == Some(id.as_str()))
                        .cloned()
                        .collect();
                    entries.sort();
                    entries
                };
                let remembered_before = of_this_tree(&state.blocked);
                let unit = |kind: char| units.iter().find(|unit| unit.tag == unit_tag(&id, kind));
                let (blocks_tree, blocks_staged) = (
                    blocks(unit('u'), &state.blocked),
                    blocks(unit('s'), &state.blocked),
                );
                // the one case in which the staged change is said to have no verdict
                let apart = !unit('s').is_some_and(|unit| {
                    matches!(&unit.judged, Some(Judged::Unjudged { reason }) if reason == STAGED_NOT_APART)
                });
                let context = format!("session {session}, stop {stops}, tree {at}, cap {cap}");

                // the model
                let cover = if tree_read {
                    &place.tree
                } else {
                    &place.told_tree
                };
                let own = beyond(&place.index, cover);
                let new_tree = tree_read && !beyond(&place.tree, &place.told_tree).is_empty();
                let new_staged =
                    index_read && apart && !beyond(&own, &place.told_staged).is_empty();
                if new_tree {
                    assert!(blocks_tree, "{context}: a new failing line did not block");
                }
                if new_staged {
                    assert!(
                        blocks_staged,
                        "{context}: a new staged line did not block\n tree {:?}\n index {:?}\n own {own:?}\n told {:?}\n remembered {:?}\n units {units:?}",
                        place.tree,
                        place.index,
                        place.told_staged,
                        state
                            .blocked
                            .iter()
                            .filter(|entry| tree_of(entry) == Some(id.as_str()))
                            .collect::<Vec<_>>()
                    );
                }
                blocked_new += usize::from(new_tree || new_staged);
                if blocks_tree && !new_tree && !place.lost {
                    assert!(
                        marked(&state.blocked, &unit_tag(&id, 'u'))
                            && place.last_tree.as_ref() != Some(&place.tree),
                        "{context}: the uncommitted change blocked with nothing new and nothing a mark does not cover"
                    );
                    beyond_model += 1;
                }
                if blocks_staged && !new_staged && !place.lost {
                    // The one thing still excused for a staged change: the INDEX is not what
                    // the last stop that looked saw, while what has blocked of it is
                    // remembered by a mark. A mark cannot say which of its lines the index
                    // still holds, so after a change of the index it answers for nothing,
                    // and the block comes with the sentence "not what it was". With the
                    // index untouched nothing is excused: the staged change does not block
                    // again whatever the working tree does, unless a stop said that its
                    // memory could not be kept. (The exception was "its own lines are not
                    // what the last stop saw", which a change of the tree alone makes true:
                    // a staged failure of 300 lines that the tree had again in part, and
                    // that was then repaired there in part, blocked through it; tenth
                    // independent review of the hooks, HR10-M-2.)
                    assert!(
                        (marked(&state.blocked, &unit_tag(&id, 's'))
                            || marked(&state.blocked, &unit_tag(&id, 'k')))
                            && place.last_index.as_ref() != Some(&place.index),
                        "{context}: the staged change blocked with nothing new, the index untouched, and no stop had said that its memory could not be kept"
                    );
                    beyond_model += 1;
                }
                stood += usize::from(
                    !blocks_tree
                        && !blocks_staged
                        && tree_read
                        && index_read
                        && (!place.tree.is_empty() || !place.index.is_empty()),
                );

                let judging = Judging {
                    trees: vec![looked],
                    listed: Some((0..trees).map(|n| format!("t{n}")).collect()),
                    ..Judging::default()
                };
                let kept = remember_within(&mut state, &judging, &mut Vec::new(), cap);
                assert!(
                    state.blocked.len() <= cap,
                    "{context}: {}",
                    state.blocked.len()
                );
                assert!(
                    state_text(&state).len() < 140 * 1024,
                    "{context}: the state is {} bytes",
                    state_text(&state).len()
                );
                if not_read {
                    assert!(
                        !blocks_tree && !blocks_staged,
                        "{context}: a stop that read nothing blocked"
                    );
                    assert_eq!(
                        of_this_tree(&state.blocked),
                        remembered_before,
                        "{context}: a stop that could not read the tree changed what is remembered of it"
                    );
                    unread += 1;
                }
                shortened += kept.short.len();
                lost += kept.lost.len();
                given_up += kept.given_up.len() + kept.as_a_whole.len();
                assert!(
                    kept.lost
                        .iter()
                        .all(|unit| !unit.starts_with(&format!("{id}/")))
                        || cap < 4,
                    "{context}: the tree a stop looked at was forgotten: {kept:?}"
                );
                for unit in kept
                    .lost
                    .iter()
                    .chain(&kept.given_up)
                    .chain(&kept.as_a_whole)
                {
                    let n: usize = unit[1..unit.find('/').unwrap()].parse().unwrap();
                    all[n].lost = true;
                }
                // What the hook says it has lost altogether, the model forgets too: from
                // then on neither knows it, and a stop that cannot read the tree takes
                // nothing of it for the tree's own (so what the hook then puts to the
                // agent as staged is what the model expects).
                for unit in &kept.lost {
                    let n: usize = unit[1..unit.find('/').unwrap()].parse().unwrap();
                    if unit.ends_with("/u") {
                        all[n].told_tree.clear();
                    } else {
                        all[n].told_staged.clear();
                    }
                }
                // the model remembers what the stop saw
                let here = &mut all[at];
                if tree_read {
                    here.told_tree = place.tree.clone();
                    here.last_tree = Some(place.tree.clone());
                }
                if index_read && apart {
                    let mut told: Content = here
                        .told_staged
                        .iter()
                        .filter_map(|(file, count)| {
                            Some((*file, (*count).min(*place.index.get(file)?)))
                        })
                        .collect();
                    told.extend(own.clone());
                    here.told_staged = told;
                    here.last_index = Some(place.index.clone());
                }
                // a tree with nothing failing left has nothing that could block again
                if tree_read && index_read && place.tree.is_empty() && place.index.is_empty() {
                    here.lost = false;
                }
                stops += 1;
            }
        }
        // the sessions reached what they are for
        assert!(stops > 6_000, "{stops}");
        assert!(
            shortened > 0 && lost > 0 && given_up > 0,
            "{shortened} {lost} {given_up}"
        );
        assert!(
            stood > 300 && blocked_new > 300 && part_staged > 100 && unread > 100,
            "{stood} {blocked_new} {part_staged} {unread}"
        );
        // (how often the hook blocked where the model did not, under the two exceptions
        // above: a marked uncommitted change that changed, and a marked staged change
        // whose index changed)
        assert!(beyond_model > 0, "{beyond_model}");
    }

    /// Eleventh independent review of the hooks, HR11-L-4, as reproduced (`mi7.py 600`). 600
    /// failing files are staged and repaired in the tree; the tree has 300 of them again;
    /// one more failing file is staged, which blocks, and there the stop says, truly, that
    /// the digest it had is given up and that what is staged may block once more when the
    /// tree is repaired again. Then the tree is repaired and the index is not touched: that
    /// is the block it warned of. The sentence was said again at that stop, where the index
    /// holds exactly what it held and from where on everything it holds is remembered: no
    /// such block can come any more, and none does.
    #[test]
    fn a_digest_given_up_is_said_where_it_is_given_up_and_not_at_the_block_it_warned_of() {
        let lines = |files: usize| failing_files(0, files, 1);
        let mut state = State::default();
        let stop = |state: &mut State, tree: usize, index: usize| -> (bool, Kept) {
            let (looked, units) = seen_of(
                "a",
                "/a",
                (tree > 0).then(|| lines(tree)),
                Some(lines(index)),
                false,
                &state.blocked,
            );
            let blocks = matches!(
                hook::decide_stop_trees(&units, &state.blocked, "r"),
                StopDecision::Block { .. }
            );
            let judging = Judging {
                trees: vec![looked],
                ..Judging::default()
            };
            (
                blocks,
                remember_within(state, &judging, &mut Vec::new(), BLOCKED_CAP),
            )
        };
        let given_up = |kept: &Kept| {
            memory_notes(kept)
                .iter()
                .any(|note| note.contains("the index no longer holds exactly that"))
        };
        assert!(stop(&mut state, 0, 600).0, "staged, the tree repaired");
        assert!(!stop(&mut state, 0, 600).0);
        assert!(stop(&mut state, 300, 600).0, "the tree's own failure");
        assert!(!stop(&mut state, 300, 600).0);
        // one more failing file staged: the index is not what the digest stood for
        let (blocks, kept) = stop(&mut state, 300, 601);
        assert!(blocks);
        assert!(given_up(&kept), "{:?}", memory_notes(&kept));
        assert!(!stop(&mut state, 300, 601).0);
        // the tree repaired, the index untouched: the block that was announced, and no
        // sentence about an index that "no longer holds" what it holds
        let (blocks, kept) = stop(&mut state, 0, 601);
        assert!(blocks);
        assert!(!given_up(&kept), "{:?}", memory_notes(&kept));
        assert_eq!(kept, Kept::default());
        // from here on everything the index holds has blocked: whatever the tree does,
        // the staged change stands
        assert!(!stop(&mut state, 0, 601).0);
        assert!(stop(&mut state, 100, 601).0, "the tree's own failure");
        let (blocks, kept) = stop(&mut state, 0, 601);
        assert!(!blocks, "the staged failure blocked again");
        assert_eq!(kept, Kept::default());
    }

    /// Review V4 round 21, VD4R21-M-1, and the thirteenth independent review of the hooks,
    /// HR13-L-1 and HR13-L-2. No directory Git cannot place is Git's own, wherever it lies:
    /// the reason a stop gives for one starts with Git's words, so that a cut for room keeps
    /// them, and says in plain words when the directory is inside the Git directory. Where it
    /// lies is in the words only.
    #[test]
    fn a_directory_git_cannot_place_is_said_with_gits_words_first() {
        let words = "`git rev-parse` failed: fatal: this operation must be run in a work tree";
        let common = PathBuf::from(format!("/{}/repo.git", "c".repeat(160)));
        for (cwd, inside) in [
            (common.join("hooks/feature"), true),
            (common.join("svn"), true),
            (common.clone(), true),
            (
                PathBuf::from(format!("/{}/feature", "d".repeat(200))),
                false,
            ),
        ] {
            let reason = unplaced(&cwd, &common, words);
            assert!(reason.starts_with(words), "{reason}");
            // what a message shows of it, after a cut for room, holds Git's words and what
            // follows from them whole: the paths come last
            let shown: String = reason.chars().take(hook::REASON_CHARS).collect();
            assert!(
                shown.contains("which working tree it is in (") || !inside,
                "{shown}"
            );
            assert_eq!(
                reason.contains("could not be found out: it is inside the Git directory"),
                inside,
                "{reason}"
            );
            assert!(!reason.contains("Git's own"), "{reason}");
        }
    }

    /// Review V4 round 21, VD4R21-L-1, and the thirteenth independent review of the hooks:
    /// where the session's directory is when it cannot be resolved. A relative path is
    /// anchored at the hook's own directory (it was kept as written, and no tree was under
    /// it); a `..` steps up from where the part before it leads, its links followed (it was
    /// taken out by name, and `alias/../linked` was the `linked` beside `alias`); a `..`
    /// after a part that cannot be resolved leaves the directory unknown, and every tree
    /// may be the one it is in.
    #[test]
    fn where_an_unresolved_directory_is_follows_the_system() {
        let scratch = deep_diff_forge_testkit::Scratch::new("hook-unfound");
        let root = std::fs::canonicalize(scratch.path()).unwrap();
        std::fs::create_dir_all(root.join("deep/inner")).unwrap();
        std::fs::create_dir_all(root.join("deep/linked")).unwrap();
        std::fs::create_dir_all(root.join("linked")).unwrap();
        std::os::unix::fs::symlink(root.join("deep/inner"), root.join("alias")).unwrap();
        assert_eq!(
            through_links(&root.join("alias/../linked/locked/sub")),
            Some(root.join("deep/linked/locked/sub"))
        );
        assert_eq!(
            through_links(&root.join("linked/./x/../locked")),
            None,
            "a `..` after a part that is not there"
        );
        assert_eq!(
            through_links(&root.join("deep/../linked/locked")),
            Some(root.join("linked/locked"))
        );
        let under = Unfound::Under(root.join("deep/linked/locked/sub"));
        assert!(under.may_be_in(&root.join("deep/linked")));
        assert!(!under.may_be_in(&root.join("linked")));
        assert!(Unfound::Anywhere.may_be_in(&root.join("linked")));
    }

    /// Review V4 round 22, VD4R22-M-1, and the fourteenth independent review of the hooks
    /// (suspected): where the session's directory is, is found one way for every use. A
    /// relative path is anchored at the hook's own directory and then walked up to the
    /// nearest directory that is there, as an absolute one is (it was "nothing there", and
    /// the linked tree it was under was "not in it"); a part that names a file is walked
    /// past (it was "gone"); a part that cannot be looked at leaves it unknown, under the
    /// path with its links followed.
    #[test]
    fn where_the_session_is_is_found_one_way() {
        use std::os::unix::fs::PermissionsExt as _;
        let scratch = deep_diff_forge_testkit::Scratch::new("hook-standing");
        let root = std::fs::canonicalize(scratch.path()).unwrap();
        std::fs::create_dir_all(root.join("deep/inner")).unwrap();
        std::fs::create_dir_all(root.join("deep/linked")).unwrap();
        std::fs::write(root.join("deep/inner/file"), "x").unwrap();
        std::os::unix::fs::symlink(root.join("deep/inner"), root.join("alias")).unwrap();
        let here = std::fs::canonicalize(std::env::current_dir().unwrap()).unwrap();
        // from the hook's own directory up to `/`, and down to `root`
        let up: PathBuf = here.components().skip(1).map(|_| "..").collect();
        let climbing = up.join(root.strip_prefix("/").unwrap());
        let at = |path: &Path| match standing_directory(path) {
            Standing::At(at) => at,
            Standing::Unknown { why, .. } => panic!("{}: {why}", path.display()),
        };
        for (path, wanted) in [
            (root.join("deep/inner"), root.join("deep/inner")),
            (root.join("deep/inner/gone/deeper"), root.join("deep/inner")),
            (root.join("deep/inner/file"), root.join("deep/inner")),
            (root.join("deep/inner/file/sub"), root.join("deep/inner")),
            (root.join("alias/gone"), root.join("deep/inner")),
            (
                root.join("alias/../linked/build/gone"),
                root.join("deep/linked"),
            ),
            (
                climbing.join("deep/linked/build/gone"),
                root.join("deep/linked"),
            ),
            (climbing.join("deep/inner/file"), root.join("deep/inner")),
            (
                PathBuf::from("no-such-directory-of-the-test/sub"),
                here.clone(),
            ),
        ] {
            assert_eq!(at(&path), wanted, "{}", path.display());
        }
        // a directory that cannot be searched: unknown, and under the path as written
        std::fs::create_dir_all(root.join("deep/linked/locked/sub")).unwrap();
        std::fs::set_permissions(
            root.join("deep/linked/locked"),
            std::fs::Permissions::from_mode(0o000),
        )
        .unwrap();
        let searchable = std::fs::canonicalize(root.join("deep/linked/locked/sub")).is_ok();
        let locked = standing_directory(&root.join("alias/../linked/locked/sub"));
        std::fs::set_permissions(
            root.join("deep/linked/locked"),
            std::fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        if !searchable {
            match locked {
                Standing::Unknown {
                    may_be: Unfound::Under(under),
                    why,
                } => {
                    assert_eq!(under, root.join("deep/linked/locked/sub"));
                    assert!(why.contains("could not be looked at"), "{why}");
                }
                _ => panic!("a directory that cannot be searched was placed"),
            }
        }
    }

    /// Round 23: a state of the schema before (v9) is read, with no tree recorded and its
    /// memory as it was, and is written back in the new one; a `trees` member that is not
    /// what the hooks write is no state.
    #[test]
    fn a_state_of_the_schema_before_is_read_and_written_anew() {
        let entry = format!("0123456789abcdef/u/{}:2", "f".repeat(24));
        let v9 = json!({"schema": "deep-diff-forge.hook-state.v9", "project": null, "stops": 3,
                        "reported": [], "noticed": [], "blocked": [entry],
                        "judged_sha256": "", "judged_count": 0})
        .to_string();
        let (state, before) = state_read(&v9).unwrap();
        assert!(
            before,
            "a state of the schema before is written back in the new one"
        );
        assert_eq!(state.blocked, std::slice::from_ref(&entry));
        assert!(state.trees.is_empty());
        assert_eq!(state.stops, 3);
        let text = state_text(&state);
        assert!(
            text.contains("\"schema\":\"deep-diff-forge.hook-state.v10\""),
            "{text}"
        );
        assert_eq!(state_read(&text), Ok((state, false)));
        for trees in [
            json!(null),
            json!([]),
            json!({"0123456789abcdef": 1}),
            json!({"0123456789abcdef": "relative"}),
            json!({"0123456789ABCDEF": "/w"}),
            json!({"m": "/w"}),
            json!({"0123456789abcdef": format!("/{}", "l".repeat(16 * 1024))}),
        ] {
            let text = json!({"schema": STATE_SCHEMA, "project": null, "stops": 0,
                              "reported": [], "noticed": [], "blocked": [],
                              "judged_sha256": "", "judged_count": 0, "trees": trees})
            .to_string();
            assert!(state_from(&text).is_err(), "{text}");
        }
        let v8 = v9.replace("hook-state.v9", "hook-state.v8");
        assert!(state_from(&v8).is_err());
    }

    /// The thirteenth independent review of the hooks (suspected, reproduced in round 22)
    /// and the fourteenth (HR14-L-1): a ceiling that hides a part of one of the project's
    /// working trees is left out; every other entry of the caller's ceiling stays, in its
    /// order, so a repository the user hides above the project stays hidden. Before the
    /// project is known the caller's ceiling is as it is (no variable is set).
    #[test]
    fn a_ceiling_hides_nothing_of_the_project_and_nothing_else_is_shown() {
        let scratch = deep_diff_forge_testkit::Scratch::new("hook-ceiling");
        let root = std::fs::canonicalize(scratch.path()).unwrap();
        for dir in ["home/project", "home/feature/sub", "elsewhere/x"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        std::os::unix::fs::symlink(root.join("home/feature"), root.join("alias")).unwrap();
        let trees = [root.join("home/project"), root.join("home/feature")];
        let entry = |path: &str| root.join(path).to_string_lossy().into_owned();
        let ceiling = [
            entry("home"),
            entry("home/feature/sub"),
            String::new(),
            entry("alias/sub"),
            "relative".to_owned(),
            entry("home/project"),
            entry("elsewhere"),
            entry("home/feature"),
            // through the project, to a directory beside it: it stays (fifteenth review)
            entry("home/project/../hidden"),
            // through a directory that is not there, into the project: left out
            entry("home/gone/../project/x"),
            // through a link, up from where it leads: beside the tree, so it stays
            entry("alias/../other"),
        ]
        .join(":");
        let wanted = [
            entry("home"),
            String::new(),
            "relative".to_owned(),
            entry("elsewhere"),
            entry("home/project/../hidden"),
            entry("alias/../other"),
        ]
        .join(":");
        assert_eq!(
            ceiling_without(OsStr::new(&ceiling), &trees),
            OsString::from(wanted)
        );
        assert_eq!(ceiling_without(OsStr::new(""), &trees), OsString::new());
        let options = Deadline::after(Duration::from_secs(1)).git();
        assert!(
            !options
                .env
                .iter()
                .any(|(name, _)| name == "GIT_CEILING_DIRECTORIES"),
            "{:?}",
            options.env
        );
        let set = Deadline {
            ceiling: Some(OsStr::new("/a:/b")),
            ..Deadline::after(Duration::from_secs(1))
        }
        .within(Duration::from_secs(1))
        .git();
        assert!(set.env.contains(&(
            OsString::from("GIT_CEILING_DIRECTORIES"),
            OsString::from("/a:/b")
        )));
    }

    /// Review V4 round 23, VD4R23-M-1, and the fifteenth independent review of the hooks,
    /// HR15-L-1 and HR15-L-2: Git's answer is believed only as a pair that names one tree of
    /// the repository's records. A linked Git directory pairs with the directory its `gitdir`
    /// file names; the main one, named `.git`, with its own directory; a main Git directory of
    /// another name records no top level. Everything else, and a `gitdir` that is missing or
    /// cannot be read, pairs with no tree.
    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "one table: every way Git's pair can be one tree of the records or not"
    )]
    fn git_is_believed_only_as_a_pair_that_names_one_tree() {
        let scratch = deep_diff_forge_testkit::Scratch::new("hook-pair");
        let root = std::fs::canonicalize(scratch.path()).unwrap();
        let repo = root.join("repo");
        let common = repo.join(".git");
        for dir in [
            "repo/.git/worktrees/l",
            "repo/.git/worktrees/i",
            "repo/.git/worktrees/missing",
            "repo/.git/worktrees/unreadable/gitdir",
            "repo/sub",
            "linked/sub",
            "outer/inner",
            "copy",
            "elsewhere/admin",
            "apart/repo.git",
            "apart/work",
        ] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        let admin = |name: &str| common.join("worktrees").join(name);
        let linked = root.join("linked");
        std::fs::write(
            admin("l").join("gitdir"),
            format!("{}\n", linked.join(".git").display()),
        )
        .unwrap();
        // written relative to the administrative directory, as `--relative-paths` writes it
        std::fs::write(admin("i").join("gitdir"), "../../../../outer/inner/.git\n").unwrap();
        std::fs::write(
            root.join("elsewhere/admin/gitdir"),
            format!("{}\n", linked.join(".git").display()),
        )
        .unwrap();
        let (outer, inner) = (root.join("outer"), root.join("outer/inner"));
        let apart = root.join("apart/repo.git");
        let table: [(&str, PathBuf, PathBuf, PathBuf, Pair); 13] = [
            (
                "main",
                repo.clone(),
                common.clone(),
                common.clone(),
                Pair::One,
            ),
            (
                "main pointed at a subdirectory",
                repo.join("sub"),
                common.clone(),
                common.clone(),
                Pair::Not(Some(repo.clone())),
            ),
            (
                "linked",
                linked.clone(),
                admin("l"),
                common.clone(),
                Pair::One,
            ),
            (
                "linked, its gitdir relative",
                inner.clone(),
                admin("i"),
                common.clone(),
                Pair::One,
            ),
            (
                "linked pointed at the main tree",
                repo.clone(),
                admin("l"),
                common.clone(),
                Pair::Not(Some(linked.clone())),
            ),
            (
                "linked pointed at an outer tree it lies in",
                outer,
                admin("i"),
                common.clone(),
                Pair::Not(Some(inner)),
            ),
            (
                "linked pointed at a subdirectory of itself",
                linked.join("sub"),
                admin("l"),
                common.clone(),
                Pair::Not(Some(linked.clone())),
            ),
            (
                "a copy of a linked tree",
                root.join("copy"),
                admin("l"),
                common.clone(),
                Pair::Not(Some(linked.clone())),
            ),
            (
                "gitdir missing",
                linked.clone(),
                admin("missing"),
                common.clone(),
                Pair::Not(None),
            ),
            (
                "gitdir unreadable",
                linked.clone(),
                admin("unreadable"),
                common.clone(),
                Pair::Not(None),
            ),
            (
                "a Git directory not under worktrees",
                linked.clone(),
                root.join("elsewhere/admin"),
                common.clone(),
                Pair::Not(None),
            ),
            (
                "linked, of another common directory",
                linked,
                admin("l"),
                apart.clone(),
                Pair::Not(None),
            ),
            (
                "a main Git directory made apart",
                root.join("apart/work"),
                apart.clone(),
                apart,
                Pair::Unrecorded,
            ),
        ];
        for (case, top, git_dir, common, wanted) in table {
            assert_eq!(pair_of(&top, &git_dir, &common), wanted, "{case}");
        }
        // the two trees come first in the words, the Git directory after them, so that a reason
        // cut for room keeps the trees
        let unpaired = Unpaired {
            id: String::new(),
            top: root.join("outer"),
            git_dir: admin("i"),
            recorded: Some(root.join("outer/inner")),
        };
        let words = unpaired.words();
        assert!(
            words.starts_with("Git names ")
                && words.contains(
                    "/outer\" as the working tree of a Git directory whose records name "
                )
                && words.ends_with("/outer/inner\" as that tree"),
            "{words}"
        );
        let reason = unpaired.reason(&root.join("outer/inner/lib"));
        assert!(
            reason.starts_with(&words) && reason.ends_with("/repo/.git/worktrees/i\")"),
            "{reason}"
        );
        let unrecorded = Unpaired {
            recorded: None,
            ..unpaired
        };
        assert!(
            unrecorded
                .words()
                .ends_with("whose records name no working tree that can be read")
        );
        // the trees such a stop holds: every tree the directory lies under, and the two named
        let unfound = Unfound::Unpaired {
            cwd: root.join("copy/lib"),
            tops: vec![root.join("copy"), root.join("linked")],
        };
        assert!(unfound.may_be_in(&root.join("copy")));
        assert!(unfound.may_be_in(&root.join("linked/.")));
        assert!(unfound.may_be_in(&root));
        assert!(!unfound.may_be_in(&root.join("repo")));
    }

    /// Fourteenth independent review of the hooks, HR14-L-3: where Git stopped at a
    /// file-system boundary, the mount point it names is read whole, and a stop asks whether a
    /// linked tree lies across it; the project's own tree is never asked about.
    #[test]
    fn a_boundary_git_stopped_at_is_read_and_asked_about() {
        let words = "fatal: not a git repository (or any parent up to mount point /w/feat (x))\nStopping at filesystem boundary (GIT_DISCOVERY_ACROSS_FILESYSTEM not set).\n";
        assert_eq!(mount_point(words), Some(PathBuf::from("/w/feat (x)")));
        assert_eq!(
            mount_point("fatal: not a git repository (or any of the parent directories): .git\n"),
            None
        );
        let linked = [PathBuf::from("/w/feat (x)"), PathBuf::from("/w/other")];
        let cwd = Path::new("/w/feat (x)/vol/sub");
        assert!(across(cwd, Some(Path::new("/w/feat (x)")), &linked));
        assert!(across(cwd, Some(Path::new("/w/feat (x)/vol")), &linked));
        assert!(!across(cwd, Some(Path::new("/var")), &linked));
        assert!(across(cwd, None, &linked));
        assert!(!across(Path::new("/w/project/vol"), None, &linked));
    }

    /// Review V4 round 22's case 2 and the fourteenth independent review of the hooks,
    /// HR14-L-2: a tree something is remembered of that the repository no longer lists is
    /// kept while its directory stands (or cannot be looked at), and known as the tree that
    /// stands at its path when that is listed again; it is forgotten when its directory is
    /// gone, or when no directory of it was recorded. Each kept path is recorded for the
    /// next stop; a path that is too long, not UTF-8, or past the bound in all is not.
    #[test]
    fn a_tree_no_longer_listed_is_kept_while_its_directory_stands() {
        use std::os::unix::ffi::OsStrExt as _;
        let scratch = deep_diff_forge_testkit::Scratch::new("hook-unlisted");
        let root = std::fs::canonicalize(scratch.path()).unwrap();
        for dir in ["stands", "again", "listed"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        std::fs::write(root.join("file"), "x").unwrap();
        let id = |n: u8| format!("{n:016x}");
        let line = |n: u8| format!("{}/u/{}:1", id(n), "f".repeat(24));
        let blocked: Vec<String> = (1..=6)
            .map(line)
            .chain([format!("m/u/{}:1", "f".repeat(24))])
            .collect();
        let at = |dir: &str| root.join(dir).to_string_lossy().into_owned();
        let recorded: std::collections::BTreeMap<String, String> = [
            (id(1), at("stands")),
            (id(2), at("gone")),
            (id(3), at("file")),
            (id(4), at("again")),
            (id(6), at("listed")),
        ]
        .into();
        let linked = Linked {
            trees: vec![(root.join("again"), id(9)), (root.join("listed"), id(6))],
            whole: true,
        };
        let unlisted = unlisted_trees(&blocked, &recorded, &linked);
        let found: Vec<(String, &str)> = unlisted
            .iter()
            .map(|tree| {
                let is = match &tree.is {
                    UnlistedIs::Stands => "stands",
                    UnlistedIs::Unknown(_) => "unknown",
                    UnlistedIs::Again(now) if *now == id(9) => "again",
                    UnlistedIs::Again(_) => "again as another",
                };
                (tree.id.clone(), is)
            })
            .collect();
        assert_eq!(found, [(id(1), "stands"), (id(4), "again")]);
        assert_eq!(unlisted[0].top, root.join("stands"));
        // nothing is unlisted when the administrative directories were not read whole
        let part = Linked {
            whole: false,
            ..linked
        };
        assert!(unlisted_trees(&blocked, &recorded, &part).is_empty());
        // what is recorded after a stop: the top this stop knows, else the one recorded
        let tops: std::collections::BTreeMap<String, PathBuf> = [
            (id(6), root.join("listed")),
            (
                id(5),
                PathBuf::from(std::ffi::OsStr::from_bytes(b"/w/\xff")),
            ),
        ]
        .into();
        let mut long = "/".to_owned();
        long.push_str(&"l".repeat(usize::try_from(POINTER_BYTES).unwrap()));
        let before: std::collections::BTreeMap<String, String> = [
            (id(1), at("stands")),
            (id(2), long),
            (id(7), at("forgotten")),
        ]
        .into();
        let after = recorded_trees(&blocked, &before, &tops);
        assert_eq!(
            after,
            [(id(1), at("stands")), (id(6), at("listed"))].into(),
            "a tree that is forgotten, a path too long and one that is not UTF-8 are not recorded"
        );
        // the bound in all
        let many: Vec<String> = (0..80_u8).map(|n| format!("{}/u/k:1", id(n))).collect();
        let big: std::collections::BTreeMap<String, String> = (0..80_u8)
            .map(|n| (id(n), format!("/{}", "p".repeat(16 * 1024 - 1))))
            .collect();
        let kept = recorded_trees(&many, &big, &std::collections::BTreeMap::new());
        assert_eq!(kept.len(), TREES_BYTES / (16 * 1024));
    }

    /// Review V4 round 24, VD4R24-M-1, and the sixteenth independent review, HR16-L-3: an
    /// identity does not move while its remembered directory stands. A tree the state records
    /// at one directory and that Git and the records now pair with another top is read at the
    /// new top only where the tree no longer stands at the recorded directory (seventeenth
    /// independent review of the hooks, HR17-M-1: any directory there counted):
    ///
    /// | the recorded directory | the new top the same | the new top another |
    /// | --- | --- | --- |
    /// | its `.git` names this tree's administrative directory | read | not read: it is said |
    /// | is gone | read | read (a tree moved and repaired) |
    /// | is a file | read | read |
    /// | cannot be looked at | read | not read |
    /// | has no `.git` (a build directory made at the old path) | read | read |
    /// | its `.git` names another tree's (a tree added at the old path) | read | read |
    /// | its `.git` is a directory (a clone made at the old path) | read | read |
    /// | its `.git` cannot be read | read | not read |
    /// | not recorded | read | read |
    ///
    /// And two listed entries with one path are two trees (HR16-L-1).
    #[test]
    fn a_tree_is_not_moved_while_the_directory_it_is_remembered_at_stands() {
        use std::os::unix::fs::PermissionsExt as _;
        let scratch = deep_diff_forge_testkit::Scratch::new("hook-standing-record");
        let root = std::fs::canonicalize(scratch.path()).unwrap();
        for dir in [
            "stands",
            "new",
            "locked/tree",
            "bare",
            "other",
            "clone/.git",
            "unread",
            "admin/mine",
            "admin/theirs",
        ] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        for admin in ["admin/mine", "admin/theirs"] {
            std::fs::write(root.join(admin).join("commondir"), "../..\n").unwrap();
        }
        let mine = instance_of(&root.join("admin/mine")).unwrap();
        let dot_git = |dir: &str, admin: &str| {
            let named = format!("gitdir: {}\n", root.join(admin).display());
            std::fs::write(root.join(dir).join(".git"), named).unwrap();
        };
        dot_git("stands", "admin/mine");
        dot_git("other", "admin/theirs");
        dot_git("unread", "admin/mine");
        std::fs::write(root.join("file"), "x").unwrap();
        for (path, mode) in [("locked", 0o000), ("unread/.git", 0o000)] {
            std::fs::set_permissions(root.join(path), std::fs::Permissions::from_mode(mode))
                .unwrap();
        }
        let cell = |dir: &str, top: &str| {
            let recorded = [(mine.clone(), root.join(dir).to_string_lossy().into_owned())].into();
            standing_record(&recorded, &mine, &root.join(top))
        };
        let rows: Vec<_> = [
            "stands",
            "gone",
            "file",
            "locked/tree",
            "bare",
            "other",
            "clone",
            "unread",
        ]
        .iter()
        .map(|dir| (cell(dir, dir), cell(dir, "new")))
        .collect();
        let not_recorded =
            standing_record(&std::collections::BTreeMap::new(), &mine, &root.join("new"));
        for (path, mode) in [("locked", 0o755), ("unread/.git", 0o644)] {
            std::fs::set_permissions(root.join(path), std::fs::Permissions::from_mode(mode))
                .unwrap();
        }
        assert_eq!(
            rows,
            [
                (None, Some(root.join("stands"))),
                (None, None),
                (None, None),
                (None, Some(root.join("locked/tree"))),
                (None, None),
                (None, None),
                (None, None),
                (None, Some(root.join("unread"))),
            ]
        );
        assert_eq!(not_recorded, None);
        // the same directory written another way is the same top
        assert_eq!(cell("stands", "new/../stands"), None);
        // two entries of the listing with one path: each takes a tree of its own
        let linked = Linked {
            trees: vec![
                (root.join("new"), "1".repeat(16)),
                (root.join("new"), "2".repeat(16)),
            ],
            whole: true,
        };
        let first = listed_instance(&root.join("new"), false, &root, &linked, &BTreeSet::new());
        let second = listed_instance(
            &root.join("new"),
            false,
            &root,
            &linked,
            &[first.clone()].into(),
        );
        assert_eq!((first, second), ("1".repeat(16), "2".repeat(16)));
    }

    /// Mutation run on `07df120`: what a stop knows of a working tree without asking Git
    /// had mutants that no test caught. Each is read whole, or is "not known":
    ///
    /// - a tree known by its path alone is known by that path, and only by it (`by_path`
    ///   returned one name for every path);
    /// - a path in a file of Git's is read whole up to 16 KiB (`16 + 1024` bytes cut a long
    ///   one, and named another directory);
    /// - a `worktrees` directory that cannot be resolved (a link that loops) is not "no
    ///   linked tree was ever made" (every resolving error was);
    /// - a `shallow` file that cannot be read is not "a clone that is whole";
    /// - submodules or a linked tree are something to look at, each on its own.
    #[test]
    fn what_is_known_of_a_tree_without_git_is_read_whole_or_said_unknown() {
        let scratch = deep_diff_forge_testkit::Scratch::new("hook-known-without-git");
        let root = std::fs::canonicalize(scratch.path()).unwrap();
        let (a, b) = (by_path(&root.join("a")), by_path(&root.join("b")));
        assert_ne!(a, b);
        assert_eq!(a, by_path(&root.join("a")));
        assert!(a.starts_with('x') && a.len() == 16, "{a}");

        let long = root
            .join("d".repeat(250))
            .join("e".repeat(250))
            .join("f".repeat(250));
        let long = long
            .join("g".repeat(250))
            .join("h".repeat(250))
            .join("i".repeat(250))
            .join("tree");
        // Six segments of 250 bytes pass 1,300 whatever the temporary directory is
        // (five did so only under a directory of 41 bytes or more).
        assert!(long.as_os_str().len() > 1_300);
        let pointer = scratch.write("gitdir", format!("{}/.git\n", long.display()));
        assert_eq!(pointed_path(&pointer, b"", &root), Some(long.join(".git")));

        let common = root.join("project/.git");
        std::fs::create_dir_all(&common).unwrap();
        assert!(shallow_commits(&common).is_ok_and(|none| none.is_empty()));
        std::fs::create_dir(common.join("shallow")).unwrap();
        assert!(shallow_commits(&common).is_err());
        std::os::unix::fs::symlink("worktrees", common.join("worktrees")).unwrap();
        let looped = linked_instances(&common);
        assert!(!looped.whole && looped.trees.is_empty());

        let place = |top: &Path, common: &Path| Place {
            top: top.to_path_buf(),
            own: common.to_path_buf(),
            common: common.to_path_buf(),
            git_dir: common.to_path_buf(),
            id: "1".to_owned(),
        };
        let (top, git) = (root.join("p2"), root.join("p2/.git"));
        std::fs::create_dir_all(&git).unwrap();
        assert!(!has_more_to_look_at(&place(&top, &git)));
        std::fs::create_dir(git.join("worktrees")).unwrap();
        assert!(
            !has_more_to_look_at(&place(&top, &git)),
            "an empty directory"
        );
        std::fs::write(top.join(".gitmodules"), "").unwrap();
        assert!(has_more_to_look_at(&place(&top, &git)), "submodules alone");
        std::fs::remove_file(top.join(".gitmodules")).unwrap();
        std::fs::create_dir(git.join("worktrees/t")).unwrap();
        assert!(
            has_more_to_look_at(&place(&top, &git)),
            "a linked tree alone"
        );
    }

    /// Mutation run on `07df120`, `kept_reason`: the rule names beyond [`RULES_KEPT`] are
    /// counted, and only those (`>=` wrote "+0" after exactly eight), and what the judge said
    /// is kept on one line (a control character in it was kept as it was).
    #[test]
    fn what_a_commit_keeps_of_its_reason_counts_only_what_it_leaves_out() {
        let rules = |count: usize| -> Vec<hook::RuleOutcome> {
            (0..count)
                .map(|n| hook::RuleOutcome {
                    id: format!("r{n}"),
                    why: String::new(),
                    held: false,
                    violations: Vec::new(),
                })
                .collect()
        };
        assert_eq!(
            kept_reason(&rules(RULES_KEPT), &[], true),
            "r0,r1,r2,r3,r4,r5,r6,r7"
        );
        assert_eq!(
            kept_reason(&rules(RULES_KEPT + 1), &[], true),
            "r0,r1,r2,r3,r4,r5,r6,r7,+1"
        );
        assert_eq!(
            kept_reason(&rules(1), &["a\tb\u{1b}c|d\n".to_owned()], true),
            "r0|a b c/d"
        );
    }

    /// S3.21.d: forge-warden names a built-in rule `builtin:<name>`. Its name is what a later
    /// stop of the session says the commit failed or is held for, read back from the line
    /// of the judged file: the `:` was kept as `_`, a rule the judge does not have.
    #[test]
    fn a_builtin_rule_keeps_its_name_at_a_later_stop() {
        let rule = |id: &str, held: bool| hook::RuleOutcome {
            id: id.to_owned(),
            why: "w".to_owned(),
            held,
            violations: Vec::new(),
        };
        let id = "0123456789abcdef0123456789abcdef01234567";
        let both = [
            rule("builtin:no-secrets", false),
            rule("builtin:quiet", true),
        ];
        for (line, failed) in [
            (format!("{id}:f:{}", kept_reason(&both, &[], true)), true),
            (
                format!("{id}:h:{}", kept_reason(&both[1..], &[], false)),
                false,
            ),
        ] {
            assert!(is_judged_entry(&line), "{line}");
            let kept = match answer(&line) {
                Some((_, Answer::Fail(kept))) if failed => kept,
                Some((_, Answer::Hold(kept))) if !failed => kept,
                other => panic!("{line} read as {other:?}"),
            };
            let names: Vec<(String, bool)> = reason_kept(kept, !failed)
                .0
                .into_iter()
                .map(|rule| (rule.id, rule.held))
                .collect();
            let expected: &[(&str, bool)] = if failed {
                &[("builtin:no-secrets", false), ("builtin:quiet", true)]
            } else {
                &[("builtin:quiet", true)]
            };
            let expected: Vec<(String, bool)> = expected
                .iter()
                .map(|(name, held)| ((*name).to_owned(), *held))
                .collect();
            assert_eq!(
                names, expected,
                "the rule names a later stop reads back from the line {line}"
            );
        }
    }

    /// Mutation run on `07df120`: what says the things a stop does not judge had mutants no
    /// test caught. A name cut for the counting line has [`NOTE_NAME`] characters at most
    /// (one more with `* 1` for `+ 1`); a note said in full takes its room, and one that
    /// does not fit is counted (`+=` for `-=` said every note in full); one tree where a
    /// failure stands is said as one; the note for trees that could not be listed is there
    /// exactly when there is a linked tree or a failure in a tree this stop does not judge.
    #[test]
    fn notes_and_names_take_no_more_than_their_room() {
        let name = format!(
            "the working tree \"/{}\" (a failure stands there)",
            "d".repeat(300)
        );
        let cut = counted_name(&name);
        assert_eq!(cut.chars().count(), NOTE_NAME, "{cut}");
        assert!(cut.ends_with("d… (a failure stands there)"), "{cut}");

        let note = |n: usize| Note {
            name: format!("n{n}"),
            text: format!("{n} {}", "x".repeat(98)),
            standing: 0,
        };
        let mut state = State::default();
        // room for the line that counts, the first note and part of the second
        let said = notes_of(&mut state, vec![note(1), note(2)], COUNT_LINE + 101 + 60);
        assert_eq!(said.len(), 2, "{said:?}");
        assert_eq!(said[0], note(1).text);
        assert_eq!(said[1], "1 thing in this project is not judged: n2.");

        assert_eq!(
            standing_counted(1).1,
            " A failure that has blocked a stop of this session stands in 1 of them: it was not looked at at this stop."
        );
        assert!(
            standing_counted(2)
                .1
                .starts_with(" Failures that have blocked")
        );

        let error = GitAcquireError::TimedOut {
            command: "git worktree list",
            limit_ms: 1,
        };
        let linked = |trees: usize| Linked {
            trees: (0..trees)
                .map(|n| (PathBuf::from(format!("/t{n}")), format!("{n}")))
                .collect(),
            whole: true,
        };
        let own = BTreeSet::from(["0".to_owned()]);
        let mine = vec!["0/u/k:1".to_owned()];
        assert_eq!(not_listed(&linked(0), &own, &mine, &error), Vec::new());
        let elsewhere = vec!["0/u/k:1".to_owned(), "7/s/k:1".to_owned()];
        let notes = not_listed(&linked(0), &own, &elsewhere, &error);
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert_eq!(notes[0].standing, 1);
        assert!(notes[0].name.ends_with("(a failure stands in 1 of them)"));
        let notes = not_listed(&linked(2), &own, &mine, &error);
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert_eq!(notes[0].standing, 0);
    }

    /// Mutation run on `07df120`, `staged_memory` and `Unit::short`: how a staged change
    /// is remembered where its own lines are a mark had mutants no test caught.
    ///
    /// - The own lines are a mark (257 of them) and one more line of the index has blocked
    ///   before: everything the index holds has blocked, and one mark of it is kept, so that
    ///   any part of it can be the staged change's own again and stand. Counted otherwise
    ///   (`!=` for `==`, `*` for `+`, a line that is not the same) it was not.
    /// - The own lines are a mark, and the index holds a line that has not blocked: the own
    ///   mark answers for itself alone ("remembered as a whole from now on"), and it is said.
    /// - Own lines that are lines (not a mark) are never "a whole".
    /// - One own line is kept as that line, not as a mark of one.
    #[test]
    fn a_staged_change_is_remembered_as_a_whole_only_where_that_is_known() {
        let lines = |names: &[String]| -> Lines { names.iter().map(|n| (n.clone(), 1)).collect() };
        let own_names: Vec<String> = (0..257).map(|n| format!("o{n}")).collect();
        let own = lines(&own_names);
        let folded = Lines::from([hook::mark_of(&own)]);
        let kept = Lines::from([("k".to_owned(), 1)]);
        let mut index = own.clone();
        index.insert("k".to_owned(), 1);
        let held = hook::mark_of(&index);

        let now = staged_memory(&Lines::new(), &kept, &index, &own, &folded);
        assert_eq!(now.kept, Lines::from([held]));
        assert!(!now.gone && !now.whole);

        // a line the index holds that has not blocked: the own mark is a whole of its own
        let mut more = own.clone();
        more.insert("z".to_owned(), 1);
        let now = staged_memory(&Lines::new(), &Lines::new(), &more, &own, &folded);
        assert!(now.whole && now.kept.is_empty() && !now.gone);
        // and once it is remembered so, it is not said again
        let now = staged_memory(&folded, &Lines::new(), &more, &own, &folded);
        assert!(!now.whole);

        // own lines that are lines: never a whole
        let a = Lines::from([("a".to_owned(), 1)]);
        let both = Lines::from([("a".to_owned(), 1), ("z".to_owned(), 1)]);
        let now = staged_memory(&Lines::new(), &Lines::new(), &both, &a, &a);
        assert!(!now.whole && now.own == a);

        let short = Unit {
            own: a.clone(),
            kept: Lines::new(),
            entries: 1,
        }
        .short();
        assert_eq!(short.own, Some(("a".to_owned(), 1)));
        assert!(short.kept.is_none() && !short.gone);
    }

    /// Mutation run on `07df120`: five mutants of what the stop's own tree leaves for the
    /// project's tree were caught by no test (a judge would have to take within a second of
    /// the time a run has to tell them apart from the outside). Three seconds are held back
    /// exactly for a tree that is judged after this one and is not known to be clean, and
    /// the project's tree is looked at only when it comes after.
    #[test]
    fn the_project_s_tree_has_time_held_back_only_when_it_comes_after_and_has_a_change() {
        let never =
            || -> bool { panic!("the project's tree was looked at when no tree comes after") };
        assert_eq!(held_for_the_project(0, 2, || false), COMMIT_FLOOR);
        assert_eq!(held_for_the_project(0, 2, || true), Duration::ZERO);
        assert_eq!(held_for_the_project(1, 2, never), Duration::ZERO);
        assert_eq!(held_for_the_project(0, 1, never), Duration::ZERO);
        assert_eq!(held_for_the_project(1, 3, || false), COMMIT_FLOOR);
        assert_eq!(held_for_the_project(2, 3, never), Duration::ZERO);
    }

    /// Mutation run on `07df120`, `stop`: what the sentences about memory take of a
    /// message is counted with the line each is on (`-` or `*` for `+` counted less, and
    /// the other notes were given room the message did not have).
    #[test]
    fn the_sentences_about_memory_take_their_lines_from_the_room() {
        assert_eq!(lines_taken(&[]), 0);
        assert_eq!(lines_taken(&[String::new()]), 1);
        assert_eq!(lines_taken(&["ab".to_owned(), "é…".to_owned()]), 3 + 3);
    }

    /// Eleventh independent review of the hooks, HR11-L-1. The file of judged commits is
    /// only added to, and the last line of a commit is its answer ([`sorted_commits`]).
    /// [`asks_kept`] read every line: a commit that had no verdict at one stop and passed
    /// at the next held five seconds back at every later stop of the session, and one that
    /// had six such stops ten, although no commit waited (from a linked tree a run of 17
    /// seconds on the stop's own change was cut at 13.6).
    #[test]
    fn time_is_held_back_only_for_commits_that_still_wait() {
        let seconds = Duration::from_millis;
        let line = |n: usize, said: &str| format!("{n:040x}:{said}");
        // no verdict at one stop, then a pass: nothing waits
        assert_eq!(
            asks_kept(&[line(1, "n1@1:r"), line(1, "p")]),
            seconds(3_000)
        );
        // no verdict at six stops (the last ones in its turn), then a pass
        let mut six: Vec<String> = (1..=6).map(|n| line(1, &format!("n{n}@{n}:r"))).collect();
        assert_eq!(asks_kept(&six), seconds(8_000));
        six.push(line(1, "f:no-secrets"));
        assert_eq!(asks_kept(&six), seconds(3_000));
        // another commit still waits: it is counted, and only it
        six.push(line(2, "n2@8:r"));
        assert_eq!(asks_kept(&six), seconds(8_000));
        six.push(line(3, "n9@8:r"));
        assert_eq!(asks_kept(&six), OWED_ASKS_NEED);
        // its last line counts, whatever stood before it
        six.push(line(3, "x:the commit has no parent that can be read"));
        six.push(line(2, "h:review"));
        assert_eq!(asks_kept(&six), seconds(3_000));
        // (a line that cannot be read is no commit that waits)
        six.push("not a line".to_owned());
        assert_eq!(asks_kept(&six), seconds(3_000));
    }

    /// Ninth independent review of the hooks, HR9-M-1, as reproduced: a failure of 257 lines
    /// has blocked; then a part of it is staged, as it is, then more, then all of it. Past
    /// 256 lines every `git add` of a part blocked again, as "Staged changes", for files that
    /// had not changed: the staged lines were compared with the tree's failure as it is
    /// remembered, one mark, which covers no single line. At every size the staging stands,
    /// and the stop says nothing about memory.
    #[test]
    fn staging_a_part_of_a_failure_that_has_blocked_stands() {
        for size in [1, 200, 256, 257, 600] {
            let mut state = State::default();
            let whole = failing_files(0, size, 1);
            // (the stop, with the first `staged` failing files in the index)
            let stop = |state: &mut State, staged: usize| -> (bool, Kept, usize) {
                let index = (staged > 0 && staged < size).then(|| failing_files(0, staged, 1));
                let (looked, units) = seen_of(
                    "t",
                    "/p",
                    Some(whole.clone()),
                    index,
                    staged == size,
                    &state.blocked,
                );
                let blocks = matches!(
                    hook::decide_stop_trees(&units, &state.blocked, "r"),
                    StopDecision::Block { .. }
                );
                let judging = Judging {
                    trees: vec![looked],
                    ..Judging::default()
                };
                let kept = remember_within(state, &judging, &mut Vec::new(), BLOCKED_CAP);
                (blocks, kept, units.len())
            };
            assert!(stop(&mut state, 0).0, "{size}: the failure is new");
            for staged in [0, 1, size / 2, size.saturating_sub(1), size, size / 2, 0] {
                let (blocks, kept, units) = stop(&mut state, staged);
                assert!(!blocks, "{size} lines, {staged} staged: it blocked again");
                assert_eq!(kept, Kept::default(), "{size} lines, {staged} staged");
                // nothing of the index is the staged change's own: there is no such unit
                assert_eq!(units, 1, "{size} lines, {staged} staged");
            }
        }
    }

    /// Tenth independent review of the hooks, HR10-M-2, as reproduced: a staged failure that
    /// has blocked as the staged change's own; then the working tree has it again in part;
    /// then it is repaired there again in part, and then wholly. The index is never touched.
    /// Past 256 lines the stop blocked for "Staged changes" at the partial repair: the mark
    /// of all that had blocked was remembered, and a mark was asked only whether the staged
    /// change's own failure is exactly it. A mark that is exactly what the index holds says
    /// that every line the index holds has blocked: then no part of it is new.
    #[test]
    fn a_staged_failure_that_has_blocked_stands_whatever_the_tree_does() {
        for size in [1, 200, 256, 257, 300, 600] {
            let mut state = State::default();
            // one stop: the tree fails for the files `from..to`, the index for all of them
            let stop = |state: &mut State, from: usize, to: usize| -> (bool, bool, Kept) {
                let tree = (to > from).then(|| failing_files(from, to - from, 1));
                let same = from == 0 && to == size;
                let (looked, units) = seen_of(
                    "t",
                    "/p",
                    tree,
                    (!same).then(|| failing_files(0, size, 1)),
                    same,
                    &state.blocked,
                );
                let blocks = |kind: char| {
                    units
                        .iter()
                        .find(|unit| unit.tag == unit_tag("t", kind))
                        .is_some_and(|unit| {
                            matches!(
                                hook::decide_stop_trees(
                                    std::slice::from_ref(unit),
                                    &state.blocked,
                                    "r"
                                ),
                                StopDecision::Block { .. }
                            )
                        })
                };
                let (tree_blocks, staged_blocks) = (blocks('u'), blocks('s'));
                let judging = Judging {
                    trees: vec![looked],
                    ..Judging::default()
                };
                let kept = remember_within(state, &judging, &mut Vec::new(), BLOCKED_CAP);
                (tree_blocks, staged_blocks, kept)
            };
            let part = size.div_ceil(3);
            let half = part.div_ceil(2);
            // staged, the tree repaired: blocks as the staged change's own; then stands
            assert!(stop(&mut state, 0, 0).1, "{size}");
            assert_eq!(
                stop(&mut state, 0, 0),
                (false, false, Kept::default()),
                "{size}"
            );
            // what the tree does from here on; its own change blocks when it has a file it
            // did not have at the stop before
            let mut had = 0..0;
            for (from, to, row) in [
                (0, part, "a part is in the tree again"),
                (0, part, "again"),
                (0, half, "half of that part repaired again"),
                (half, part, "the other half instead"),
                (0, 0, "all repaired again"),
                (0, size, "all of it in the tree"),
                (0, size, "again"),
                (0, 0, "all repaired once more"),
            ] {
                let (tree, staged, kept) = stop(&mut state, from, to);
                assert!(!staged, "{size} lines, {row}: what is staged blocked again");
                assert_eq!(
                    tree,
                    (from..to).any(|file| !had.contains(&file)),
                    "{size} lines, {row}"
                );
                assert_eq!(kept, Kept::default(), "{size} lines, {row}");
                had = from..to;
            }
        }
    }

    /// Ninth independent review of the hooks, HR9-L-1, as reproduced. A staged failure has
    /// blocked, and the working tree's failure has the same lines again, so they are kept
    /// for the staged change and are no line of its own. Then the session has more lines
    /// than it keeps: the kept lines were let go with no word, and when the tree was
    /// repaired again the staged failure, untouched in the index, blocked a second time. They
    /// become one mark of everything that has blocked for that staged change, and it stands.
    #[test]
    fn what_is_kept_for_a_staged_change_is_kept_as_a_mark() {
        const SMALL: usize = 300;
        let lines = |files: usize| failing_files(0, files, 1);
        let mut state = State::default();
        // one stop in the tree `a`, with what its tree and its index fail for
        let stop = |state: &mut State, tree: usize, index: usize, cap: usize| -> (bool, Kept) {
            let (looked, units) = seen_of(
                "a",
                "/a",
                (tree > 0).then(|| lines(tree)),
                (index > 0 && index != tree).then(|| lines(index)),
                index > 0 && index == tree,
                &state.blocked,
            );
            let blocks = matches!(
                hook::decide_stop_trees(&units, &state.blocked, "r"),
                StopDecision::Block { .. }
            );
            let judging = Judging {
                trees: vec![looked],
                ..Judging::default()
            };
            (
                blocks,
                remember_within(state, &judging, &mut Vec::new(), cap),
            )
        };
        // 250 files staged with the tree repaired: blocks as the staged change's own
        assert!(stop(&mut state, 0, 250, BLOCKED_CAP).0);
        // the tree has them back: blocks as the tree's; the staged lines are kept
        assert!(stop(&mut state, 250, 250, BLOCKED_CAP).0);
        assert_eq!(hook::remembered_as(&state.blocked, "a/k").len(), 250);
        assert!(!stop(&mut state, 0, 250, BLOCKED_CAP).0, "repaired again");
        assert!(
            stop(&mut state, 250, 250, BLOCKED_CAP).0,
            "back in the tree"
        );
        // A stop in another tree with 250 failing files, under a bound of 300 entries: more
        // lines than are kept. What is remembered of `a` goes into its short form (it is
        // not the tree this stop looks at). For its staged change the stop says so: a
        // mark answers only for exactly what it stands for.
        let (blocks, kept) = stop_in(&mut state, "b", Some(failing_files(1_000, 250, 1)), SMALL);
        assert!(blocks);
        assert_eq!(
            kept,
            Kept {
                as_a_whole: vec!["a/s".to_owned()],
                ..Kept::default()
            }
        );
        assert!(
            memory_notes(&kept)
                .iter()
                .any(|note| note.contains("is remembered as a whole from now on")),
            "{:?}",
            memory_notes(&kept)
        );
        let kept = hook::remembered_as(&state.blocked, "a/k");
        assert_eq!(kept.len(), 1, "{kept:?}");
        assert!(kept.keys().all(|key| hook::is_mark(key)));
        // The tree is repaired again and the index is as it has been since the first stop:
        // what is staged stands. And once more round: the tree's failure blocks as the
        // tree's (documented), the staged one stands again.
        let (blocks, said) = stop(&mut state, 0, 250, SMALL);
        assert!(!blocks, "the staged failure blocked a second time");
        assert_eq!(said.given_up, Vec::<String>::new());
        assert!(stop(&mut state, 250, 250, SMALL).0);
        assert!(!stop(&mut state, 0, 250, SMALL).0);

        // Where a mark cannot be kept, the stop says so. The index holds 300 failing files;
        // 250 of them have blocked as the staged change's own and are kept; the tree has
        // all 300. In the short form the mark stands for the 250, which is neither what the
        // index holds nor what is the staged change's own at the next stop: it goes, and
        // that stop says of which tree.
        let mut state = State::default();
        let look = |state: &mut State, tree: Judged, cap: usize| -> (bool, Kept) {
            let (looked, units) = seen_of(
                "a",
                "/a",
                Some(tree),
                Some(lines(300)),
                false,
                &state.blocked,
            );
            let blocks = matches!(
                hook::decide_stop_trees(&units, &state.blocked, "r"),
                StopDecision::Block { .. }
            );
            let judging = Judging {
                trees: vec![looked],
                ..Judging::default()
            };
            (
                blocks,
                remember_within(state, &judging, &mut Vec::new(), cap),
            )
        };
        // the tree fails for files 250 to 299 only: files 0 to 249 are the staged change's own
        assert!(look(&mut state, failing_files(250, 50, 1), BLOCKED_CAP).0);
        // the tree fails for all 300: the 250 are kept, and under a bound of 100 as a mark
        let (_, kept) = look(&mut state, lines(300), 100);
        assert_eq!(
            kept,
            Kept {
                as_a_whole: vec!["a/s".to_owned()],
                ..Kept::default()
            }
        );
        // nothing changes: nothing blocks, and the mark of the 250 cannot be kept
        let (blocks, said) = look(&mut state, lines(300), 100);
        assert!(!blocks);
        assert_eq!(said.given_up, ["a/s"]);
        let notes = memory_notes(&said);
        assert!(
            notes
                .iter()
                .any(|note| note.contains("Of what is staged in 1 working tree, the hook remembered what had blocked as a whole")
                    && note.contains("may block once more")),
            "{notes:?}"
        );
    }

    /// The time of one run of the judge ([`run_time`]), from the eighth independent review
    /// of the hooks (HR8-L-1: the first ask a stop owes got "what is left less five seconds",
    /// which under five seconds was no time, and was written down as asked) to the tenth
    /// (HR10-L-3: a run that may take all that is left cannot be owed beside others; and
    /// HR10-L-2: a run on the stop's own tree is held back only what the tree after it can
    /// need). What every owed ask needs is one number, [`OWED_ASKS_NEED`], and
    /// `docs/reference/agent-hooks.md` says the same.
    #[test]
    fn a_run_of_the_judge_has_its_part_of_the_time_or_is_not_made() {
        let seconds = |ms: u64| Duration::from_millis(ms);
        let asks = |left: u64, owed: u32| run_time(seconds(left), After::Asks(owed));
        // an ask leaves five seconds for each ask owed after it
        assert_eq!(asks(41_000, 2), Some(seconds(31_000)));
        assert_eq!(asks(41_000, 0), Some(seconds(41_000)));
        assert_eq!(asks(13_000, 2), Some(seconds(3_000)));
        // with less than that it has three seconds, not all that is left: the asks owed
        // after it are still made while three seconds are left for them
        assert_eq!(asks(7_900, 2), Some(seconds(3_000)));
        assert_eq!(asks(4_900, 1), Some(seconds(3_000)));
        // with under three seconds it has what there is (and is not made at all, by the
        // rule of [`judge_commits`]); with nothing, it is not run
        assert_eq!(asks(1_900, 0), Some(seconds(1_900)));
        assert_eq!(asks(0, 0), None);
        assert_eq!(asks(0, 2), None);
        // With OWED_ASKS_NEED left every owed ask is made, whatever the others do: each
        // may use all it is given, and between two of them two seconds may be lost.
        assert_eq!(OWED_ASKS, 3);
        assert_eq!(OWED_ASKS_NEED, seconds(13_000));
        for left in [13_000, 13_700, 20_000, 41_000] {
            let mut left = seconds(left);
            for owed in (0..OWED_ASKS).rev() {
                assert!(left >= COMMIT_FLOOR, "an owed ask would not be made");
                let had = run_time(left, After::Asks(owed)).unwrap().min(JUDGE_TIME);
                assert!(had >= COMMIT_FLOOR);
                left = left
                    .checked_sub(had)
                    .unwrap()
                    .saturating_sub(seconds(if owed > 0 { 2_000 } else { 0 }));
            }
        }
        // a hanging judge on the first two cannot take the third's time: 30 seconds (the
        // most one run has), then 6, and 5 are left
        let first = asks(41_000, 2).unwrap().min(JUDGE_TIME);
        let second = run_time(seconds(41_000).checked_sub(first).unwrap(), After::Asks(1)).unwrap();
        assert_eq!((first, second), (seconds(30_000), seconds(6_000)));

        // A run on a working tree's change: what is held back for what comes after it, and
        // all that is left when that would leave it under three seconds.
        let tree = |left: u64, kept: Duration| run_time(seconds(left), After::Trees(kept));
        // What a tree's run leaves for the asks about commits is what those asks can
        // need, by the session's file of judged commits: three seconds when no commit
        // waits for a verdict (a commit never asked about may be there), five more for a
        // commit that waits for its turn, five more for one with fewer attempts.
        let line = |n: usize, said: &str| format!("{n:040x}:{said}");
        assert_eq!(asks_kept(&[]), seconds(3_000));
        assert_eq!(
            asks_kept(&[line(1, "p"), line(2, "f:no-secrets")]),
            seconds(3_000)
        );
        assert_eq!(asks_kept(&[line(1, "n2@4:r")]), seconds(8_000));
        assert_eq!(
            asks_kept(&[line(1, "n7@4:r"), line(2, "n9@3:r")]),
            seconds(8_000)
        );
        assert_eq!(
            asks_kept(&[line(1, "n7@4:r"), line(2, "n0@3:r"), line(3, "p")]),
            OWED_ASKS_NEED
        );
        // the stop's own tree, the project's tree clean, from a linked tree (35 seconds)
        assert_eq!(tree(35_000, asks_kept(&[])), Some(seconds(32_000)));
        assert_eq!(tree(35_000, OWED_ASKS_NEED), Some(seconds(22_000)));
        // both dirty: three seconds for the project's tree; the own tree has over half
        assert_eq!(
            tree(35_000, OWED_ASKS_NEED + COMMIT_FLOOR),
            Some(seconds(19_000))
        );
        // a staged change of its own too
        assert_eq!(
            tree(35_000, OWED_ASKS_NEED + COMMIT_FLOOR + ASK_OWED),
            Some(seconds(14_000))
        );
        assert_eq!(
            tree(12_000, OWED_ASKS_NEED + ASK_OWED),
            Some(seconds(12_000))
        );
        assert_eq!(tree(0, OWED_ASKS_NEED), None);

        let readme = include_str!("../../../docs/reference/agent-hooks.md");
        let stated = format!(
            "{} seconds of its judging left when it comes to its commits ({} for the first ask it owes and {} for each of the two after it)",
            OWED_ASKS_NEED.as_secs(),
            COMMIT_FLOOR.as_secs(),
            ASK_OWED.as_secs()
        );
        assert!(
            readme
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .contains(&stated),
            "docs/reference/agent-hooks.md does not state: {stated}"
        );
    }

    /// Eighth independent review of the hooks, HR8-M-1, and what it suspected beside it. A
    /// path in a file of Git's is read as Git reads it: as bytes, and a relative one from
    /// the directory the file is in. The content of `gitdir` was read as UTF-8 text and
    /// used as it stood, so a tree made with relative paths was not found from outside, and
    /// one whose path is not UTF-8 was dropped.
    #[test]
    fn a_path_in_a_file_of_gits_is_read_as_git_reads_it() {
        use std::os::unix::ffi::OsStrExt as _;
        let scratch = deep_diff_forge_testkit::Scratch::new("hook-pointed-path");
        let root = std::fs::canonicalize(scratch.path()).unwrap();
        let common = root.join("project/.git");
        let untaken = BTreeSet::new();
        let admin = |name: &str, gitdir: &[u8]| -> PathBuf {
            let admin = common.join("worktrees").join(name);
            std::fs::create_dir_all(&admin).unwrap();
            std::fs::write(admin.join("gitdir"), gitdir).unwrap();
            std::fs::write(admin.join("commondir"), "../..\n").unwrap();
            admin
        };
        // no linked tree was ever made: that is known, and nothing is "not listed"
        std::fs::create_dir_all(&common).unwrap();
        let none = linked_instances(&common);
        assert!(none.whole && none.trees.is_empty());

        let absolute = admin(
            "abs",
            format!("{}/abs-wt/.git\n", root.display()).as_bytes(),
        );
        let relative = admin("rel", b"../../../../rel-wt/.git\n");
        let mut odd = b"../../../../odd-".to_vec();
        odd.extend([0xff, 0xfe]);
        odd.extend(b"-wt/.git\r\n");
        let bytes = admin("odd", &odd);
        let unread = admin("unread", b"");
        // a file among them is no tree
        std::fs::write(common.join("worktrees/a-file"), "x").unwrap();

        assert_eq!(
            pointed_path(&relative.join("gitdir"), b"", &relative),
            Some(relative.join("../../../../rel-wt/.git"))
        );
        assert_eq!(
            pointed_path(&absolute.join("gitdir"), b"", &absolute),
            Some(root.join("abs-wt/.git"))
        );
        assert_eq!(pointed_path(&unread.join("gitdir"), b"", &unread), None);
        assert_eq!(pointed_path(&unread.join("none"), b"", &unread), None);
        let mut odd_name = b"odd-".to_vec();
        odd_name.extend([0xff, 0xfe]);
        odd_name.extend(b"-wt");
        let odd_top = root.join(std::ffi::OsStr::from_bytes(&odd_name));

        let linked = linked_instances(&common);
        assert!(linked.whole);
        let top_of = |admin: &Path| -> PathBuf {
            let id = instance_of(admin).unwrap();
            linked
                .trees
                .iter()
                .find(|(_, known)| *known == id)
                .map(|(top, _)| top.clone())
                .unwrap()
        };
        assert_eq!(linked.trees.len(), 4);
        assert_eq!(top_of(&absolute), root.join("abs-wt"));
        assert_eq!(
            top_of(&relative),
            root.join("rel-wt"),
            "relative to its directory"
        );
        assert_eq!(top_of(&bytes), odd_top, "not UTF-8");
        assert_eq!(top_of(&unread), PathBuf::new());

        // found from the path a listing names, whichever way the tree was recorded, also
        // when no directory stands there (a tree that is gone still has its failure)
        for (admin, top) in [
            (&absolute, root.join("abs-wt")),
            (&relative, root.join("rel-wt")),
            (&bytes, odd_top.clone()),
        ] {
            assert_eq!(
                listed_instance(&top, false, &common, &linked, &untaken),
                instance_of(admin).unwrap(),
                "{}",
                top.display()
            );
        }
        assert_eq!(
            listed_instance(&root.join("rel-wt"), true, &common, &linked, &untaken),
            MAIN_TREE
        );
        // A tree whose `gitdir` cannot be read is found by its own `.git` file, when that
        // names an administrative directory of this repository; a `.git` file that names
        // any other directory is not believed, and the tree is known by its path.
        let lost = root.join("unread-wt");
        std::fs::create_dir_all(&lost).unwrap();
        std::fs::write(
            lost.join(".git"),
            "gitdir: ../project/.git/worktrees/unread\n",
        )
        .unwrap();
        assert_eq!(
            listed_instance(&lost, false, &common, &linked, &untaken),
            instance_of(&unread).unwrap()
        );
        let elsewhere = root.join("elsewhere");
        std::fs::create_dir_all(elsewhere.join("fake")).unwrap();
        std::fs::write(elsewhere.join("fake/commondir"), "x").unwrap();
        std::fs::write(lost.join(".git"), "gitdir: ../elsewhere/fake\n").unwrap();
        assert_eq!(
            listed_instance(&lost, false, &common, &linked, &untaken),
            by_path(&lost)
        );
        assert_eq!(same_form(&root.join("a/./b/../c")), root.join("a/c"));
    }

    /// Eighth independent review of the hooks, HR8-L-3. The line that counts what is not
    /// judged names the first three things, the ones where a failure stands first, and for
    /// the others it says in how many working trees a failure stands that has blocked: that
    /// was part of a tree's name and nowhere else, so past the third name it was not said;
    /// and a failure in a tree beyond the sixteen a stop looks at was not said at all.
    #[test]
    fn the_counting_line_says_where_failures_stand() {
        let note = |name: &str, standing: usize| Note {
            name: name.to_owned(),
            text: format!("In full: {name}."),
            standing,
        };
        let state = || State {
            project: Some(project(Base::Commit(COMMIT.to_owned()))),
            ..State::default()
        };
        let line = |notes: Vec<Note>| -> String {
            // no room for anything in full: every note is on the counting line
            let said = notes_of(&mut state(), notes, 0);
            assert_eq!(said.len(), 1, "{said:?}");
            said[0].clone()
        };
        // those where a failure stands are named first, in their own order
        assert_eq!(
            line(vec![
                note("the nested repository \"vendor\"", 0),
                note("the working tree \"/a\"", 0),
                note("the working tree \"/b\" (a failure stands there)", 1),
                note("the working tree \"/c\"", 0),
                note("the working tree \"/d\" (a failure stands there)", 1),
            ]),
            "5 things in this project are not judged: the working tree \"/b\" (a failure stands there), the working tree \"/d\" (a failure stands there), the nested repository \"vendor\", and 2 more."
        );
        // past the three names their number is said, the trees a note only counts included
        let standing: Vec<Note> = (0..6)
            .map(|n| {
                note(
                    &format!("the working tree \"/t{n}\" (a failure stands there)"),
                    1,
                )
            })
            .chain([note(
                "2 more working trees that were not looked at (a failure stands in 2 of them)",
                2,
            )])
            .collect();
        let said = line(standing);
        assert!(
            said.starts_with("7 things in this project are not judged: the working tree \"/t0\" (a failure stands there), ")
                && said.ends_with(", and 4 more (among them 5 working trees where a failure stands that has blocked)."),
            "{said}"
        );
        assert_eq!(
            line(vec![
                note("a", 0),
                note("b", 0),
                note("c", 0),
                note("the working tree \"/d\" (a failure stands there)", 1),
                note("the working tree \"/e\" (a failure stands there)", 1),
            ]),
            "5 things in this project are not judged: the working tree \"/d\" (a failure stands there), the working tree \"/e\" (a failure stands there), a, and 2 more."
        );
        let one = line(
            (0..4)
                .map(|n| {
                    note(
                        &format!("the working tree \"/t{n}\" (a failure stands there)"),
                        1,
                    )
                })
                .collect(),
        );
        assert!(
            one.ends_with(
                ", and 1 more (among them 1 working tree where a failure stands that has blocked)."
            ),
            "{one}"
        );
        // the longest such line is within what is kept for it
        let long: Vec<Note> = (0..40)
            .map(|n| note(&format!("{n}{}", "n".repeat(300)), 1_000))
            .collect();
        assert!(line(long).chars().count() <= COUNT_LINE);
        // A name that is cut keeps what it says of a failure that stands: the path of a
        // working tree as a message shows it (120 characters) fits whole, and a longer
        // name loses its middle, not its end.
        let path = hook::shown_path(&format!("/home/u/{}/tree", "d".repeat(300)));
        let whole =
            format!("the working tree {path} (it could not be looked at) (a failure stands there)");
        assert!(whole.chars().count() > NOTE_NAME);
        let said = line(vec![note(&whole, 1)]);
        assert!(
            said.ends_with("… (a failure stands there).") && said.chars().count() <= COUNT_LINE,
            "{said}"
        );
        let fits = format!("the working tree {path} (a failure stands there)");
        assert!(fits.chars().count() <= NOTE_NAME);
        assert!(line(vec![note(&fits, 1)]).contains(&fits));
        let counted = format!(
            "{} (a failure stands in 12 of them)",
            "2 more working trees that were not looked at ".repeat(6)
        );
        assert!(line(vec![note(&counted, 12)]).ends_with("… (a failure stands in 12 of them)."));
        // what a note that counts trees says of them
        assert_eq!(standing_counted(0), (String::new(), String::new()));
        assert_eq!(standing_counted(1).0, " (a failure stands in 1 of them)");
        assert!(standing_counted(3).1.contains("stand in 3 of them"));
        assert_eq!(trees_counted(1, true), "1 more working tree");
        assert_eq!(trees_counted(2, false), "2 working trees");
        // the time one tree's status may take: half of what is left, or an even share when
        // that is more (the last tree has all of it)
        let seconds = Duration::from_secs;
        assert_eq!(look_share(seconds(6), 16), seconds(3));
        assert_eq!(look_share(seconds(6), 2), seconds(3));
        assert_eq!(look_share(seconds(6), 1), seconds(6));
        assert_eq!(look_share(seconds(6), 0), seconds(6));
        assert_eq!(look_share(Duration::ZERO, 3), Duration::ZERO);
    }

    /// Review V4 round 16, VD4R16-L-1: a linked working tree is known by the inode of its
    /// administrative directory with the time that directory was made, which no `touch`
    /// changes. Where the file system keeps no such time, the file `commondir` stands in, as
    /// it did everywhere: there its modification time is part of what the tree is.
    #[test]
    fn a_tree_is_known_by_when_its_directory_was_made() {
        let born = Some((1_700_000_000, 5));
        let never = || -> Option<(u64, i64, i64)> { panic!("asked with a birth time") };
        let words = instance_words(7, born, never).unwrap();
        assert_eq!(words, "b:7:1700000000.5");
        // another inode, or another birth of the same inode: another tree
        assert_ne!(instance_words(8, born, never), Some(words.clone()));
        assert_ne!(
            instance_words(7, Some((1_700_000_000, 6)), never),
            Some(words.clone())
        );
        // no birth time: the file and its time, and a tree with neither is not known
        let by_file = instance_words(7, None, || Some((9, 100, 1))).unwrap();
        assert_ne!(by_file, words);
        assert_ne!(
            instance_words(7, None, || Some((9, 101, 1))).unwrap(),
            by_file
        );
        assert_eq!(instance_words(7, None, || None), None);
        // on this file system: a directory has a birth time, and is the same tree after
        // every one of its times that can be set was set
        let scratch = deep_diff_forge_testkit::Scratch::new("hook-tree-birth");
        let admin = scratch.path().join("admin");
        std::fs::create_dir(&admin).unwrap();
        std::fs::write(admin.join("commondir"), "../..\n").unwrap();
        let Some(first) = instance_of(&admin) else {
            panic!("a directory with its file is a tree");
        };
        if std::fs::metadata(&admin).unwrap().created().is_ok() {
            let later =
                std::fs::metadata(&admin).unwrap().modified().unwrap() + Duration::from_secs(3_600);
            for name in ["commondir", "."] {
                std::fs::File::open(admin.join(name))
                    .unwrap()
                    .set_modified(later)
                    .unwrap();
            }
            assert_eq!(instance_of(&admin), Some(first));
        }
    }

    /// Review V4 round 10, VD4R10-H-1, and the third independent review, HR3-M-4: a
    /// repository is known by the inode of its Git common directory, which stays when it is
    /// moved and differs for another directory. The device number is no part of it: that is
    /// given when a file system is mounted (on btrfs, tmpfs and overlayfs it can differ after
    /// a reboot), and with it a resumed session was refused for good.
    #[test]
    fn a_repository_is_known_by_its_directory_not_by_its_path_or_its_device() {
        use std::os::unix::fs::MetadataExt as _;
        let scratch = deep_diff_forge_testkit::Scratch::new("hook-identity");
        let (one, other) = (scratch.join("one"), scratch.join("other"));
        std::fs::create_dir(&one).unwrap();
        std::fs::create_dir(&other).unwrap();
        let first = identity(&one).unwrap();
        assert!(is_identity(&first), "{first}");
        assert_eq!(first, std::fs::metadata(&one).unwrap().ino().to_string());
        assert!(!is_identity(&format!(
            "{}:{first}",
            std::fs::metadata(&one).unwrap().dev()
        )));
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
        // HR5-L-2: any name a hook accepts has a probe; one that a patch header cannot hold
        // as it is stands in Git's quoted form
        assert!(
            probe_patch("a b.toml", b"x\n")
                .is_some_and(|probe| probe.starts_with("diff --git a/a b.toml b/a b.toml\n"))
        );
        assert_eq!(
            patch_path("a/caf\u{e9}\t\"q\"\\.toml"),
            "\"a/caf\\303\\251\\t\\\"q\\\"\\\\.toml\""
        );
        assert_eq!(patch_path("a/x "), "\"a/x \"");
        for (name, policy) in [
            ("warden.toml", &b""[..]),
            ("warden.toml", &[0xff, b'\n'][..]),
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
        // A finding on the old side has the text of the removed line it names in its key: not
        // of another removed line, and not of a line that stays (a mutation run left the
        // reading of the old side alive).
        let old = ["fn a() {}", "// gone", "#[test]", "fn t() {}", "fn b() {}"];
        let removed = b"diff --git a/tests/t.rs b/tests/t.rs\n--- a/tests/t.rs\n+++ b/tests/t.rs\n@@ -1,5 +1,2 @@\n fn a() {}\n-// gone\n-#[test]\n-fn t() {}\n fn b() {}\n";
        let found = findings(removed).unwrap();
        let on_old: Vec<&Finding> = found.iter().filter(|f| f.side == "old").collect();
        assert!(!on_old.is_empty(), "{found:?}");
        for f in on_old {
            let line = usize::try_from(f.line).unwrap();
            assert!((3..=4).contains(&line), "{f:?}");
            assert_eq!(
                f.key,
                hook::finding_key("tests/t.rs", &f.label, old[line - 1]),
                "{f:?}"
            );
        }
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
