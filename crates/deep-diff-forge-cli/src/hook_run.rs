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
    GitAcquireError, GitAcquireOptions, InputGuards, SealedReview, file_at_commit, judge_patch,
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
}

fn place(cwd: &str, options: &GitAcquireOptions) -> Option<Place> {
    let dirs = repository_dirs(Path::new(cwd), options).ok()?;
    Some(Place {
        top: dirs.top,
        own: dirs.common_dir.join("deep-diff-forge"),
    })
}

/// A directory of this process, removed when the hook is done. A hook that was killed leaves
/// its directory behind; the next hook of a process with the same id removes it first.
struct Scratch(PathBuf);

impl Scratch {
    fn new(own: &Path) -> Result<Self, String> {
        let dir = own.join(format!("tmp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)
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

/// A missing, unreadable or foreign state file is an empty state: the worst that follows is a
/// finding said twice, or a failure that blocks once more.
fn load(path: &Path) -> State {
    std::fs::read_to_string(path)
        .map(|text| state_from(&text))
        .unwrap_or_default()
}

/// Write the state beside its file and move it into place, so a reader never sees half of it.
fn save(path: &Path, state: &State) -> Result<(), String> {
    let partial = path.with_extension(format!("{}.partial", std::process::id()));
    std::fs::write(&partial, state_text(state))
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

/// The working tree as the patch a judge would be given, or `None` when there is none to make:
/// no commit yet, or nothing changed.
fn change(
    place: &Place,
    work: &Path,
    options: &GitAcquireOptions,
) -> Result<Option<Change>, String> {
    match judge_patch(&place.top, "HEAD", work, options) {
        Ok(patch) if patch.bytes.is_empty() => Ok(None),
        Ok(patch) => Ok(Some(Change {
            bytes: patch.bytes,
            base: patch.base,
        })),
        // A repository with no commit has no base to judge against.
        Err(GitAcquireError::InvalidSelection { .. }) => Ok(None),
        Err(error) => Err(format!("the change could not be read from Git: {error}")),
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
    let Some(change) = change(&place, &scratch.sub("work")?, &options)? else {
        return Ok(None);
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
    Some(hook::contract(judge_program(args).is_some(), has_policy))
}

fn policy_name(args: &[String]) -> String {
    crate::flag_value(args, "--policy").unwrap_or_else(|| DEFAULT_POLICY.to_owned())
}

/// The judge: `--judge PATH`, or `forge-warden` on `PATH`.
#[expect(
    clippy::disallowed_methods,
    reason = "the judge door: forge-warden is looked up on PATH when --judge names no file"
)]
fn judge_program(args: &[String]) -> Option<PathBuf> {
    if let Some(named) = crate::flag_value(args, "--judge") {
        let path = PathBuf::from(named);
        return path.is_file().then_some(path);
    }
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths)
        .map(|dir| dir.join("forge-warden"))
        .find(|candidate| candidate.is_file())
}

/// The rankers a stop may offer the judge, in order: this binary, then every file in
/// `--ranker-dir` by name. The judge checks each one's hash against the policy itself, so
/// nothing here has to read the policy.
fn rankers(args: &[String]) -> Vec<PathBuf> {
    let mut list: Vec<PathBuf> = std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .into_iter()
        .collect();
    if let Some(dir) = crate::flag_value(args, "--ranker-dir") {
        let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .map(|entry| entry.path())
                    .filter(|path| path.is_file())
                    .collect()
            })
            .unwrap_or_default();
        found.sort();
        list.extend(found);
    }
    list.truncate(RANKERS_TRIED);
    list
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
        Ok(done) => (
            done.status.code(),
            serde_json::from_slice(&done.stdout).ok(),
            judge_words(&done.stderr),
        ),
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
        Err(reason) => return unjudged(reason),
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
    let Some(program) = judge_program(args) else {
        return unjudged(
            "forge-warden was not found (install it, or name it with --judge PATH)".to_owned(),
        );
    };
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
    for ranker in rankers(args) {
        let (code, verdict, said) = run_judge(&program, &policy_file, &patch_file, &ranker);
        last = hook::judged(code, verdict.as_ref(), said.as_deref());
        // Exit 4: the policy does not admit this ranker. Another may be admitted.
        if code != Some(4) {
            break;
        }
    }
    Ok(Some(last))
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

#[cfg(test)]
mod tests {
    use super::*;

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
