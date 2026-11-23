//! The project's outcome ledger (F11): what each stop decided of each unit, and each run of
//! `agent check`, kept where it outlives the session: `deep-diff-forge/ledger.jsonl` under the
//! repository's Git common directory, one JSON object a line, appended only (`O_APPEND`, under
//! `flock`, mode 0600, never through a link). A record holds outcomes, DDF's own codes, commit
//! ids, salted digests and counts: never a line of a patch, never the judge's words.
//!
//! The hooks are not a boundary: the agent runs as the user and can write the ledger as it can
//! write everything else of the repository. What the ledger gives is a record that outlives the
//! session's state, and that `agent verify-branch` reads where the agent cannot write it.

use serde_json::{Value, json};
use std::fmt::Write as _;
use std::io::{Read as _, Seek as _, Write as _};
use std::path::Path;
use std::time::Duration;

/// The version of every record this build writes; a record of a higher one is a ledger of a
/// newer build, which this one keeps as it is and refuses.
pub(crate) const VERSION: u64 = 1;
/// The ledger's file in the hooks' own directory.
pub(crate) const FILE: &str = "ledger.jsonl";
/// The salt of the digests of what is not committed: 32 bytes from the system, in hex.
const SALT: &str = "ledger.salt";
/// The most of the ledger's end that a stop reads (to count the checks since its session's
/// last stop, and to see a newer build's records): a stop's time does not grow with the ledger.
const TAIL: u64 = 1 << 20;

/// What a unit of a stop is to the ledger.
#[derive(Debug, Clone)]
pub(crate) enum Name {
    /// A commit, by its id.
    Commit(String),
    /// The uncommitted (`uncommitted`) or the staged (`staged`) change of the working tree
    /// `tree`, by the SHA-256 of its patch (`None`: it was not read), salted when it is written.
    Tree {
        patch: Option<String>,
        kind: &'static str,
        tree: String,
    },
    /// What a stop said that names no single commit or tree: nothing of it was judged, or the
    /// session's commits could not be listed.
    Stop,
}

/// The seals of the verdict a unit was judged with: the digests of the policy, the patch and
/// the ranker it was sealed to, and of the verdict document. A tree's record keeps the policy's
/// and the ranker's only: the others are digests of work that is not committed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Seals {
    pub(crate) policy: String,
    pub(crate) patch: String,
    pub(crate) ranker: String,
    pub(crate) verdict: Option<String>,
}

/// One record of a stop: a unit with its outcome (`pass`, `fail`, `fail_stands`, `hold`,
/// `unjudged`), its reason class where the stop knows it, and whether it blocked the stop.
#[derive(Debug, Clone)]
pub(crate) struct Unit {
    pub(crate) name: Name,
    pub(crate) outcome: &'static str,
    pub(crate) class: Option<&'static str>,
    pub(crate) seals: Option<Seals>,
    pub(crate) blocked: bool,
}

/// What a session is to the ledger: 16 hex digits of the SHA-256 of its id, which the name of
/// its state file begins with.
fn session_key(session: &str) -> String {
    deep_diff_forge_core::sha256_hex(session.as_bytes())[..16].to_owned()
}

/// Append one record per unit of a stop of `session` to the ledger in `own` (the hooks' own
/// directory, which the caller has made). Each carries the runs of `agent check --session` for
/// this session recorded since its last stop record. `wait` bounds the wait for the ledger's
/// lock.
pub(crate) fn append_stop(
    own: &Path,
    session: &str,
    units: &[Unit],
    wait: Duration,
) -> Result<(), String> {
    let key = session_key(session);
    let salt = if units
        .iter()
        .any(|unit| matches!(unit.name, Name::Tree { patch: Some(_), .. }))
    {
        Some(salt(own)?)
    } else {
        None
    };
    append(own, wait, |tail| {
        let checks = checks_since(tail, &key);
        units
            .iter()
            .map(|unit| {
                let (name, worktree) = match &unit.name {
                    Name::Commit(id) => (format!("commit:{id}"), Value::Null),
                    Name::Tree { patch, kind, tree } => {
                        let digest = match (patch, &salt) {
                            (Some(patch), Some(salt)) => salted(salt, patch),
                            _ => "unread".to_owned(),
                        };
                        (format!("tree:{digest}:{kind}"), json!(tree))
                    }
                    Name::Stop => ("stop".to_owned(), Value::Null),
                };
                let tree = matches!(unit.name, Name::Tree { .. });
                let seals = unit.seals.as_ref().map_or(Value::Null, |s| {
                    json!({
                        "policy_sha256": s.policy,
                        "ranker_sha256": s.ranker,
                        "patch_sha256": if tree { Value::Null } else { json!(s.patch) },
                        "verdict_sha256": if tree { Value::Null } else { json!(s.verdict) },
                    })
                });
                json!({
                    "v": VERSION, "kind": "stop", "session": key, "worktree": worktree,
                    "unit": name, "outcome": unit.outcome, "class": unit.class,
                    "blocked_turns": u8::from(unit.blocked),
                    "agent_checks": {"runs": checks.0, "failed": checks.1},
                    "ignored_elsewhere": null, "seals": seals,
                })
            })
            .collect()
    })
}

/// Append the record of one run of `agent check --session` that judged `units` units, `failed`
/// of them not a pass.
pub(crate) fn append_check(
    own: &Path,
    session: &str,
    units: usize,
    failed: usize,
    wait: Duration,
) -> Result<(), String> {
    append(own, wait, |_| {
        vec![json!({
            "v": VERSION, "kind": "check", "session": session_key(session),
            "units": units, "failed": failed,
        })]
    })
}

/// The runs of `agent check` for the session `key` (and how many had a unit that did not pass)
/// recorded in `tail` after the session's last stop record.
fn checks_since(tail: &[Value], key: &str) -> (usize, usize) {
    let of = |record: &Value, kind: &str| record["kind"] == kind && record["session"] == key;
    let from = tail
        .iter()
        .rposition(|record| of(record, "stop"))
        .map_or(0, |at| at + 1);
    let checks = tail[from..].iter().filter(|record| of(record, "check"));
    let failed = checks
        .clone()
        .filter(|record| record["failed"] != 0)
        .count();
    (checks.count(), failed)
}

/// `salt` and `digest` as a stop records an uncommitted patch: 32 hex digits.
fn salted(salt: &str, digest: &str) -> String {
    deep_diff_forge_core::sha256_hex(format!("{salt}\0tree\0{digest}").as_bytes())[..32].to_owned()
}

/// Open the ledger in `own` for appending, take its lock, give `records` what is at its end and
/// append what it returns, in one write. A ledger that holds a record of a newer build is kept
/// as it is: nothing is written to it.
#[expect(
    clippy::disallowed_methods,
    reason = "the ledger door: a hook waits a bounded time, in short sleeps, for another writer of the ledger"
)]
fn append(
    own: &Path,
    wait: Duration,
    records: impl FnOnce(&[Value]) -> Vec<Value>,
) -> Result<(), String> {
    use nix::fcntl::{Flock, FlockArg};
    use std::os::unix::fs::OpenOptionsExt as _;
    let path = own.join(FILE);
    let file = std::fs::OpenOptions::new()
        .read(true)
        .append(true)
        .create(true)
        .mode(0o600)
        .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK)
        .open(&path)
        .map_err(|e| format!("it could not be opened: {e}"))?;
    if !file.metadata().is_ok_and(|found| found.is_file()) {
        return Err("it is not a regular file".to_owned());
    }
    let mut waited = Duration::ZERO;
    let mut file = file;
    let mut lock = loop {
        match Flock::lock(file, FlockArg::LockExclusiveNonblock) {
            Ok(lock) => break lock,
            Err((back, nix::errno::Errno::EWOULDBLOCK)) if waited < wait => {
                file = back;
                std::thread::sleep(Duration::from_millis(10));
                waited += Duration::from_millis(10);
            }
            Err((_, nix::errno::Errno::EWOULDBLOCK)) => {
                return Err("another hook held its lock".to_owned());
            }
            Err((_, errno)) => return Err(format!("its lock could not be taken: {errno}")),
        }
    };
    let handle: &mut std::fs::File = &mut lock;
    let length = handle
        .metadata()
        .map_err(|e| format!("it could not be read: {e}"))?
        .len();
    let mut end = Vec::new();
    handle
        .seek(std::io::SeekFrom::Start(length.saturating_sub(TAIL)))
        .and_then(|_| std::io::Read::take(&mut *handle, TAIL).read_to_end(&mut end))
        .map_err(|e| format!("it could not be read: {e}"))?;
    let text = String::from_utf8_lossy(&end);
    // a cut at the start of what was read leaves a part of a line: it is not a record
    let whole = if length > TAIL {
        text.split_once('\n').map_or("", |(_, rest)| rest)
    } else {
        &text
    };
    let mut tail = Vec::new();
    for line in whole.lines() {
        let Some(record) = record(line) else { continue };
        if let Some(newer) = record["v"].as_u64().filter(|v| *v > VERSION) {
            return Err(format!(
                "it holds records of a newer build (v{newer}; this one writes v{VERSION}), so nothing was written to it"
            ));
        }
        tail.push(record);
    }
    let mut out = String::new();
    if end.last().is_some_and(|last| *last != b'\n') {
        out.push('\n');
    }
    for record in records(&tail) {
        out.push_str(&record.to_string());
        out.push('\n');
    }
    handle
        .write_all(out.as_bytes())
        .map_err(|e| format!("it could not be written: {e}"))
}

/// One line of the ledger as a record: a JSON object that gives no key twice, else `None`.
fn record(line: &str) -> Option<Value> {
    if crate::hook::has_duplicate_key(line) {
        return None;
    }
    serde_json::from_str::<Value>(line)
        .ok()
        .filter(Value::is_object)
}

/// The ledger's salt in `own`, made when it is missing: 32 bytes of the system's randomness in
/// hex, written to a file of this process and linked into place, which refuses a name that is
/// taken, so two first writers agree on one salt.
fn salt(own: &Path) -> Result<String, String> {
    use std::os::unix::fs::OpenOptionsExt as _;
    let path = own.join(SALT);
    let read = || -> Result<Option<String>, String> {
        let mut text = String::new();
        match std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK)
            .open(&path)
        {
            Ok(file) => {
                file.take(65)
                    .read_to_string(&mut text)
                    .map_err(|e| format!("its salt could not be read: {e}"))?;
                let text = text.trim_end_matches('\n');
                (text.len() == 64 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
                    .then(|| Some(text.to_owned()))
                    .ok_or_else(|| "its salt is not 64 hex digits".to_owned())
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("its salt could not be read: {e}")),
        }
    };
    if let Some(salt) = read()? {
        return Ok(salt);
    }
    let mut bytes = [0_u8; 32];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut random| random.read_exact(&mut bytes))
        .map_err(|e| format!("no salt could be drawn: {e}"))?;
    let made = bytes.iter().fold(String::new(), |mut hex, b| {
        let _ = write!(hex, "{b:02x}");
        hex
    });
    let partial = own.join(format!("{SALT}.{}", crate::hook_run::random_hex()));
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(nix::libc::O_NOFOLLOW)
        .open(&partial)
        .and_then(|mut file| file.write_all(format!("{made}\n").as_bytes()))
        .map_err(|e| format!("its salt could not be written: {e}"))?;
    let linked = std::fs::hard_link(&partial, &path);
    let _ = std::fs::remove_file(&partial);
    match linked {
        Ok(()) => Ok(made),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            read()?.ok_or_else(|| "its salt could not be read".to_owned())
        }
        Err(e) => Err(format!("its salt could not be written: {e}")),
    }
}
