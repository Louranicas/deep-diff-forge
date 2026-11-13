//! `agent hook <event> --client claude`: read the client's JSON, look at the repository, print
//! what [`crate::hook`] decides.
//!
//! This is the part that touches the world: stdin, Git, a scratch directory, one state file per
//! session and, at a stop, the judge. Every decision is made in [`crate::hook`].
//!
//! A hook never fails its client. Whatever goes wrong here (no repository, Git too slow, an
//! unreadable state file, a judge that cannot be started) ends with exit 0: either nothing is
//! said, or the user is told that the hook did not run and that nothing was judged. Only a
//! verdict of fail from the judge keeps an agent from stopping.
//!
//! The state file and the scratch directory live in `deep-diff-forge/` under the repository's
//! Git common directory, beside Git's own files: nothing appears in the working tree, and the
//! objects written while a patch is made stay on the same file system and are removed again.

use crate::emit;
use crate::hook::{self, Event, Finding, Input, Judged, StopDecision};
use deep_diff_forge_core::display_safe;
use deep_diff_forge_core::process::{Limits, run_bounded};
use deep_diff_forge_core::{PatchLineKind, exit};
use deep_diff_forge_graph::Side;
use deep_diff_forge_patch::{
    GitAcquireOptions, InputGuards, JudgePatchError, SealedReview, file_at_commit, judge_patch,
    repository_dirs,
};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub(crate) const USAGE: &str = "deep-diff-forge agent hook session-start|post-tool|stop --client claude [--judge PATH] [--policy NAME] [--ranker-dir DIR]";

/// The policy file a judge reads when `--policy` names no other.
const DEFAULT_POLICY: &str = "warden.toml";
/// What the stop hook says it judged.
const RANGE: &str = "changes since HEAD";
/// The most a client may write on a hook's stdin.
const INPUT_CAP: u64 = 1024 * 1024;
/// The most keys of reported findings one session's state keeps.
const REPORTED_CAP: usize = 5_000;
/// The most rankers a stop tries before it gives up on finding one the policy admits.
const RANKERS_TRIED: usize = 8;
/// Git may take this long for an advice hook, whose client waits five seconds.
const ADVICE_GIT_LIMIT: Duration = Duration::from_secs(4);
/// Git and the judge may each take this long at a stop, whose client waits sixty seconds.
const STOP_GIT_LIMIT: Duration = Duration::from_secs(20);
const JUDGE_LIMITS: Limits = Limits {
    time: Duration::from_secs(30),
    stdout_bytes: 4 * 1024 * 1024,
    stderr_bytes: 64 * 1024,
};

/// `agent hook <event> --client claude […]`.
pub(crate) fn run(args: &[String]) {
    let words = crate::args::admit_or_exit(
        args,
        &crate::args::Spec {
            flags: &[],
            valued: &["--client", "--judge", "--policy", "--ranker-dir"],
            positionals: 1,
        },
        USAGE,
    );
    // The stop hook's own helper, started by it as a child (see [`sealed_rankers`]).
    if words.first().copied() == Some(SEAL_WORD) {
        seal_rankers(args);
        return;
    }
    let event = words.first().and_then(|word| Event::named(word));
    let client = crate::flag_value(args, "--client").unwrap_or_default();
    let (Some(event), "claude") = (event, client.as_str()) else {
        eprintln!("error: name one event and --client claude");
        eprintln!("usage: {USAGE}");
        std::process::exit(exit::USAGE);
    };
    let mut text = String::new();
    if let Err(error) = std::io::stdin().take(INPUT_CAP).read_to_string(&mut text) {
        did_not_run(&format!("its input could not be read: {error}"));
        return;
    }
    let said = hook::parse_input(event, &text).and_then(|input| match event {
        Event::SessionStart => Ok(session_start(&input, args)),
        Event::PostTool => post_tool(&input),
        Event::Stop => stop(&input, args),
    });
    match said {
        Ok(Some(out)) => emitln!("{out}"),
        Ok(None) => {}
        Err(reason) => did_not_run(&reason),
    }
}

/// Tell the user, not the model, that the hook did nothing. Exit 0: a hook's own failure is
/// never a block and never a pass.
fn did_not_run(reason: &str) {
    emitln!(
        "{}",
        json!({"systemMessage": format!(
            "deep-diff-forge: the hook did not run: {}. Nothing was judged.",
            display_safe(reason)
        )})
    );
}

fn git_options(time_limit: Duration) -> GitAcquireOptions {
    GitAcquireOptions {
        time_limit,
        ..GitAcquireOptions::default()
    }
}

/// Where a hook works: the repository's top level, and its own directory under the Git common
/// directory. `None` outside a repository, where a hook has nothing to look at.
struct Place {
    top: PathBuf,
    own: PathBuf,
    /// The Git common directory: for a linked worktree it is not under `top`.
    common: PathBuf,
}

impl Place {
    /// The directories a change in this repository can write to: no judge is run from them.
    fn writable(&self) -> [&Path; 2] {
        [&self.top, &self.common]
    }
}

fn place(cwd: &str, options: &GitAcquireOptions) -> Option<Place> {
    let dirs = repository_dirs(Path::new(cwd), options).ok()?;
    Some(Place {
        top: dirs.top,
        own: dirs.common_dir.join("deep-diff-forge"),
        common: dirs.common_dir,
    })
}

/// Make sure the hook's own directory is a directory of its own: made with mode 0700 when it is
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
            hook::shown_path(&own.to_string_lossy())
        )
    };
    match is_own(own) {
        Ok(true) => Ok(()),
        Ok(false) => Err(refused()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            match std::fs::DirBuilder::new().mode(0o700).create(own) {
                Ok(()) => Ok(()),
                // another hook of this repository made it first
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

/// A directory of this process, removed when the hook is done. A hook that was killed leaves
/// its directory behind; the next hook of a process with the same id removes it first.
struct Scratch(PathBuf);

impl Scratch {
    fn new(own: &Path) -> Result<Self, String> {
        own_directory(own)?;
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

/// What one session remembers between hooks.
#[derive(Debug, Default, PartialEq, Eq)]
struct State {
    /// The keys of findings already said in this session ([`Finding::key`]).
    reported: BTreeSet<String>,
    /// The failure that blocked the last stop, if the last stop was blocked.
    last_block: Option<Vec<String>>,
}

const STATE_SCHEMA: &str = "deep-diff-forge.hook-state.v0";

/// A session's state file. The session id is hashed: it is the client's string and must not
/// choose a file name.
fn state_file(own: &Path, session: &str) -> PathBuf {
    let hash = deep_diff_forge_core::sha256_hex(session.as_bytes());
    own.join(format!("session-{}.json", &hash[..32]))
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
    State {
        reported: strings(&value["reported"])
            .unwrap_or_default()
            .into_iter()
            .collect(),
        last_block: strings(&value["last_block"]),
    }
}

fn state_text(state: &State) -> String {
    json!({
        "schema": STATE_SCHEMA,
        "reported": state.reported,
        "last_block": state.last_block,
    })
    .to_string()
}

/// The most bytes of a state file that are read. A full state ([`REPORTED_CAP`] keys of 64 hex
/// digits and a block of a few rules) is under half of this.
const STATE_CAP: u64 = 1024 * 1024;

/// A missing, unreadable, foreign or oversized state file is an empty state: the worst that
/// follows is a finding said twice, or a failure that blocks once more. The file is opened
/// without following a link and without waiting on a FIFO, must be a regular file once open,
/// and its size is checked before anything is read: a state file of 512 MiB used to be read
/// whole to find that it was not JSON (review V4, VD4-M-1).
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
        .map_err(|e| format!("its state could not be saved: {e}"))
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

/// Why there is no patch of the working tree although there may be a change.
enum NoPatch {
    /// A working tree the patch cannot be made from truly (a sparse checkout). Advice stays
    /// silent about it; a stop says that nothing was judged, and why.
    Unsupported(String),
    /// Git or the file system failed.
    Failed(String),
}

impl NoPatch {
    fn reason(self) -> String {
        match self {
            Self::Unsupported(reason) | Self::Failed(reason) => reason,
        }
    }
}

/// The working tree as the patch a judge would be given, or `None` when there is nothing to
/// judge: no commit yet, or nothing changed. Only those two are `None`. Every failure is an
/// error: when one error stood for "no base" and for failures alike, a patch that could not be
/// made let a stop pass in silence.
fn change(
    place: &Place,
    work: &Path,
    options: &GitAcquireOptions,
) -> Result<Option<Change>, NoPatch> {
    match judge_patch(&place.top, "HEAD", work, options) {
        Ok(patch) if patch.bytes.is_empty() => Ok(None),
        Ok(patch) => Ok(Some(Change {
            bytes: patch.bytes,
            base: patch.base,
        })),
        // A repository with no commit has no base to judge against.
        Err(JudgePatchError::NoBase) => Ok(None),
        Err(error @ JudgePatchError::SkipWorktree) => Err(NoPatch::Unsupported(format!(
            "the change could not be read from Git: {error}"
        ))),
        Err(error @ JudgePatchError::Failed(_)) => Err(NoPatch::Failed(format!(
            "the change could not be read from Git: {error}"
        ))),
    }
}

struct Change {
    bytes: Vec<u8>,
    base: String,
}

/// After a tool ran: say what is new in the agent's own change. Advice only.
fn post_tool(input: &Input) -> Result<Option<String>, String> {
    let options = git_options(ADVICE_GIT_LIMIT);
    let Some(place) = place(&input.cwd, &options) else {
        return Ok(None);
    };
    let scratch = Scratch::new(&place.own)?;
    let change = match change(&place, &scratch.sub("work")?, &options) {
        Ok(Some(change)) => change,
        // Nothing to say, or a working tree advice cannot be given for: a stop says so once,
        // advice after every tool call would say it for ever.
        Ok(None) | Err(NoPatch::Unsupported(_)) => return Ok(None),
        Err(NoPatch::Failed(reason)) => return Err(reason),
    };
    let all = findings(&change.bytes)?;
    let file = state_file(&place.own, &input.session);
    let mut state = load(&file);
    let fresh = hook::fresh(&all, &state.reported);
    let Some(advice) = hook::advice(&fresh) else {
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
    save(&file, &state)?;
    Ok(hook::post_tool_output(Some(&advice.text)))
}

/// At the start of a session: what the hooks will do, and whether a stop can be judged here.
fn session_start(input: &Input, args: &[String]) -> Option<String> {
    let options = git_options(ADVICE_GIT_LIMIT);
    let place = place(&input.cwd, &options)?;
    let policy = policy_name(args);
    let has_policy = matches!(
        file_at_commit(&place.top, "HEAD", &policy, &options),
        Ok(Some(_))
    );
    Some(hook::contract(
        judge_program(args, &place.writable()).is_ok(),
        has_policy,
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

/// [`judge_program`] for the repository at `place`, or for none.
fn judge_for(args: &[String], place: Option<&Place>) -> Result<PathBuf, String> {
    match place {
        Some(place) => judge_program(args, &place.writable()),
        None => judge_program(args, &[]),
    }
}

/// The rankers a stop may offer the judge, in order: this binary, then the files in
/// `--ranker-dir` that come first by name, [`RANKERS_TRIED`] in all. The judge checks each
/// one's hash against the policy itself, so nothing here has to read the policy.
fn rankers(args: &[String]) -> Vec<PathBuf> {
    let mut list: Vec<PathBuf> = std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .into_iter()
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
/// stream, when it gave any of them.
#[expect(
    clippy::disallowed_methods,
    reason = "the judge door: the stop hook starts forge-warden, under a deadline and output caps"
)]
fn run_judge(
    judge: &Path,
    policy: &Path,
    patch: &Path,
    ranker: &Path,
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
    match run_bounded(&mut command, b"", JUDGE_LIMITS) {
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

/// Have the change judged. `None` when there is nothing to judge.
fn judge(
    place: &Place,
    scratch: &Scratch,
    args: &[String],
    options: &GitAcquireOptions,
) -> Result<Option<Judged>, String> {
    let unjudged = |reason: String| Ok(Some(Judged::Unjudged { reason }));
    let change = match change(place, &scratch.sub("work")?, options) {
        Ok(Some(change)) => change,
        Ok(None) => return Ok(None),
        Err(none) => return unjudged(none.reason()),
    };
    // The policy is the one at the base commit: a change cannot loosen the policy it is judged by.
    let name = policy_name(args);
    let policy = match file_at_commit(&place.top, &change.base, &name, options) {
        Ok(Some(bytes)) => bytes,
        Ok(None) => {
            return unjudged(format!(
                "the base commit has no policy file {}",
                hook::shown_path(&name)
            ));
        }
        Err(error) => return unjudged(format!("the policy could not be read from Git: {error}")),
    };
    let program = match judge_program(args, &place.writable()) {
        Ok(program) => program,
        Err(reason) => return unjudged(reason),
    };
    let (policy_sha256, patch_sha256) = (
        deep_diff_forge_core::sha256_hex(&policy),
        deep_diff_forge_core::sha256_hex(&change.bytes),
    );
    let (policy_file, patch_file) = (
        scratch.0.join("policy.toml"),
        scratch.0.join("change.patch"),
    );
    std::fs::write(&policy_file, policy)
        .and_then(|()| std::fs::write(&patch_file, &change.bytes))
        .map_err(|e| format!("the judge's input could not be written: {e}"))?;
    let mut last = Judged::Unjudged {
        reason: "this binary's own path could not be found, so no ranker could be offered"
            .to_owned(),
    };
    let candidates = match sealed_rankers(args) {
        Ok(candidates) => candidates,
        Err(reason) => return unjudged(reason),
    };
    for (ranker, digest) in candidates {
        // The verdict must name the ranker it was given, by its digest.
        let ranker_sha256 = match digest {
            Ok(digest) => digest,
            Err(why) => {
                last = Judged::Unjudged {
                    reason: format!(
                        "the ranker {} {why}, so no verdict could be sealed to it",
                        hook::shown_path(&ranker.to_string_lossy())
                    ),
                };
                continue;
            }
        };
        let sealed = hook::Sealed {
            policy: &policy_sha256,
            patch: &patch_sha256,
            ranker: &ranker_sha256,
        };
        let (code, verdict, said) = run_judge(&program, &policy_file, &patch_file, &ranker);
        last = hook::judged(code, verdict.as_ref(), said.as_deref(), sealed);
        // Exit 4: the policy does not admit this ranker. Another may be admitted.
        if code != Some(4) {
            break;
        }
    }
    Ok(Some(last))
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
/// [`RANKER_SEAL_LIMIT`] has passed.
fn sealed_rankers(args: &[String]) -> Result<Candidates, String> {
    let this = std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .map_err(|_| {
            "this binary's own path could not be found, so no ranker could be offered".to_owned()
        })?;
    sealed_rankers_by(&this, args, RANKER_SEAL_LIMIT)
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
/// helper that does it is killed when this has passed. It leaves the client's sixty seconds
/// room for Git, the judge and the message.
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

/// When the agent wants to stop: have the change judged, and block only on a fail that is not
/// the one that blocked last time.
fn stop(input: &Input, args: &[String]) -> Result<Option<String>, String> {
    let options = git_options(STOP_GIT_LIMIT);
    let Some(place) = place(&input.cwd, &options) else {
        return Ok(None);
    };
    let scratch = Scratch::new(&place.own)?;
    let file = state_file(&place.own, &input.session);
    let mut state = load(&file);
    let Some(judged) = judge(&place, &scratch, args, &options)? else {
        return Ok(None);
    };
    let decision = hook::decide_stop(
        &judged,
        input.stop_hook_active,
        state.last_block.as_deref(),
        RANGE,
    );
    state.last_block = match &decision {
        StopDecision::Block { rules, .. } => Some(rules.clone()),
        StopDecision::Allow { .. } => None,
    };
    // A state that cannot be saved must not hide the decision; the client's own cap on repeated
    // blocks still ends a loop.
    let _ = save(&file, &state);
    Ok(hook::stop_output(&decision))
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

/// The commands of every `Stop` hook a Claude Code settings document registers.
fn stop_commands(settings: &Value) -> Vec<String> {
    let entries = settings["hooks"]["Stop"].as_array();
    entries
        .into_iter()
        .flatten()
        .filter_map(|entry| entry["hooks"].as_array())
        .flatten()
        .filter_map(|hook| hook["command"].as_str().map(str::to_owned))
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

/// The settings file that registers the stop hook this gate checked, or why none does: the
/// first hook that is close but not it, else that there is none.
fn registered(
    place: &Place,
    judge: Option<&Path>,
    policy: &str,
    args: &[String],
) -> Result<PathBuf, String> {
    let mut close: Option<String> = None;
    for file in settings_files(&place.top) {
        let Some(settings) = std::fs::read_to_string(&file)
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        else {
            continue;
        };
        for command in stop_commands(&settings) {
            match not_this_chain(&command, judge, policy, args) {
                None => return Ok(file),
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

/// The settings files Claude Code reads for a project at `top`: the project's two, then the
/// user's.
#[expect(
    clippy::disallowed_methods,
    reason = "the gate door: the user's Claude Code settings are found through HOME"
)]
fn settings_files(top: &Path) -> Vec<PathBuf> {
    let mut files = vec![
        top.join(".claude/settings.json"),
        top.join(".claude/settings.local.json"),
    ];
    if let Some(home) = std::env::var_os("HOME") {
        files.push(PathBuf::from(home).join(".claude/settings.json"));
    }
    files
}

/// `agent doctor --gate`: every link a judged stop depends on, checked in order in the
/// repository at `cwd`, each with what was found. A link that cannot be checked because an
/// earlier one is missing is reported as not ready, never as fine.
///
/// The judge is asked for a real verdict on a probe: a patch that adds one comment line to the
/// policy. That proves the whole chain at once (the judge runs, it admits a ranker this machine
/// has, the ranker ranks), and its answer says whether the policy protects its own file: a
/// policy that lets the probe pass can be loosened by any change.
pub(crate) fn gate(cwd: &Path, args: &[String]) -> Vec<GateCheck> {
    let options = git_options(STOP_GIT_LIMIT);
    let mut checks = Vec::new();
    let mut say = |name: &'static str, ok: bool, detail: String| {
        checks.push(GateCheck { name, ok, detail });
    };
    let not_checked = |why: &str| format!("not checked: {why}");

    let version = deep_diff_forge_patch::git_version(cwd, &options);
    let git_ok = version
        .as_ref()
        .is_ok_and(|line| git_release(line).is_some_and(|release| release >= OLDEST_GIT));
    say(
        "git",
        git_ok,
        match &version {
            Ok(line) => format!("{line} (2.40 or later is needed)"),
            Err(error) => format!("Git could not be run: {error}"),
        },
    );

    let place = cwd.to_str().and_then(|cwd| place(cwd, &options));
    say(
        "repository",
        place.is_some(),
        place.as_ref().map_or_else(
            || "this directory is not in a Git repository".to_owned(),
            |place| format!("{}", place.top.display()),
        ),
    );
    let name = policy_name(args);
    let policy = place.as_ref().and_then(|place| {
        file_at_commit(&place.top, "HEAD", &name, &options)
            .ok()
            .flatten()
    });
    say(
        "policy",
        policy.is_some(),
        if policy.is_some() {
            format!("{} at HEAD", hook::shown_path(&name))
        } else {
            format!("HEAD has no policy file {}", hook::shown_path(&name))
        },
    );
    let program = judge_for(args, place.as_ref());
    say(
        "judge",
        program.is_ok(),
        match &program {
            Ok(path) => format!("{}", path.display()),
            Err(why) => why.clone(),
        },
    );
    let program = program.ok();

    let verdict = match (&place, &policy, &program) {
        (Some(place), Some(policy), Some(program)) => probe(place, &name, policy, program, args),
        _ => Err(not_checked(
            "it needs the repository, the policy and the judge",
        )),
    };
    say(
        "chain",
        verdict.is_ok(),
        match &verdict {
            Ok((ranker, code)) => format!(
                "the judge gave a verdict (exit {code}) with ranker {}",
                ranker.display()
            ),
            Err(why) => why.clone(),
        },
    );
    say(
        "policy_protects_itself",
        verdict.as_ref().is_ok_and(|(_, code)| *code != 0),
        match &verdict {
            Ok((_, 0)) => format!(
                "a change to {} passes: any change could loosen the policy it is judged by",
                hook::shown_path(&name)
            ),
            Ok(_) => format!("a change to {} is held or fails", hook::shown_path(&name)),
            Err(_) => not_checked("the judge gave no verdict"),
        },
    );

    let registered = match &place {
        Some(place) => registered(place, program.as_deref(), &name, args),
        None => Err(not_checked("it needs the repository")),
    };
    say(
        "hooks_registered",
        registered.is_ok(),
        match registered {
            Ok(file) => format!("{}", file.display()),
            Err(why) => why,
        },
    );
    checks
}

/// Have the judge judge the probe. `Ok` with the ranker it admitted and its exit code when it
/// gave a verdict; otherwise why not.
fn probe(
    place: &Place,
    name: &str,
    policy: &[u8],
    program: &Path,
    args: &[String],
) -> Result<(PathBuf, i32), String> {
    let patch = probe_patch(name, policy).ok_or_else(|| {
        "the policy is empty, or its name cannot stand in a patch header".to_owned()
    })?;
    let scratch = Scratch::new(&place.own)?;
    let (policy_file, patch_file) = (scratch.0.join("policy.toml"), scratch.0.join("probe.patch"));
    std::fs::write(&policy_file, policy)
        .and_then(|()| std::fs::write(&patch_file, &patch))
        .map_err(|e| format!("the judge's input could not be written: {e}"))?;
    let (policy_sha256, patch_sha256) = (
        deep_diff_forge_core::sha256_hex(policy),
        deep_diff_forge_core::sha256_hex(patch.as_bytes()),
    );
    let mut last =
        "this binary's own path could not be found, so no ranker could be offered".to_owned();
    for (ranker, digest) in sealed_rankers(args)? {
        let ranker_sha256 = match digest {
            Ok(digest) => digest,
            Err(why) => {
                last = format!(
                    "the ranker {} {why}, so no verdict could be sealed to it",
                    hook::shown_path(&ranker.to_string_lossy())
                );
                continue;
            }
        };
        let sealed = hook::Sealed {
            policy: &policy_sha256,
            patch: &patch_sha256,
            ranker: &ranker_sha256,
        };
        let (code, verdict, said) = run_judge(program, &policy_file, &patch_file, &ranker);
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
        let state = State {
            reported: ["b".to_owned(), "a".to_owned()].into(),
            last_block: Some(vec!["r\0s\0p".to_owned()]),
        };
        assert_eq!(state_from(&state_text(&state)), state);
        let none = State {
            reported: BTreeSet::new(),
            last_block: None,
        };
        assert_eq!(state_from(&state_text(&none)), none);
        for foreign in [
            "",
            "not json",
            "[]",
            r#"{"schema":"other","reported":["a"]}"#,
            r#"{"reported":["a"]}"#,
        ] {
            assert_eq!(state_from(foreign), State::default(), "{foreign}");
        }
        // a member of the wrong type is dropped, not trusted in part
        let odd =
            r#"{"schema":"deep-diff-forge.hook-state.v0","reported":["a",1],"last_block":"x"}"#;
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
        assert_eq!(stop_commands(&registered), ["one", "two"]);
        for other in [
            json!({}),
            json!({"hooks": {"Stop": []}}),
            json!({"hooks": {"Stop": "agent hook stop"}}),
        ] {
            assert_eq!(stop_commands(&other), Vec::<String>::new(), "{other}");
        }

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
        let own = Path::new("/r/.git/deep-diff-forge");
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
}
