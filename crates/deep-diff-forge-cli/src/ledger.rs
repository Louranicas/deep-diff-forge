//! The project's outcome ledger (F11): what each stop decided of each unit, and each run of
//! `agent check`, kept where it outlives the session: `deep-diff-forge/ledger.jsonl` under the
//! repository's Git common directory, one JSON object a line, appended only (`O_APPEND`, under
//! `flock`, mode 0600, never through a link). A record holds outcomes, DDF's own codes, commit
//! ids, salted digests and counts: never a line of a patch, never the judge's words.
//!
//! The hooks are not a boundary: the agent runs as the user and can write the ledger as it can
//! write everything else of the repository. What the ledger gives is a record that outlives the
//! session's state, and that `agent verify-branch` reads where the agent cannot write it.

use crate::emit;
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
/// The most of the ledger's end that a stop's append reads (to count the checks since its
/// session's last stop, and to see a newer build's records); the frontier reads the whole
/// ledger, up to [`WHOLE`].
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
/// `unjudged`), its reason class ([`crate::hook::reason::ReasonClass`]), and whether it
/// blocked the stop.
#[derive(Debug, Clone)]
pub(crate) struct Unit {
    pub(crate) name: Name,
    pub(crate) outcome: &'static str,
    pub(crate) class: crate::hook::reason::ReasonClass,
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
/// this session recorded since its last stop record, and `snapshot`: the digest of what the stop
/// took the project's accepted policy from ([`snapshot_sha256`], member
/// `policy_snapshot_sha256`). `wait` bounds the wait for the ledger's lock.
pub(crate) fn append_stop(
    own: &Path,
    session: &str,
    units: &[Unit],
    snapshot: &str,
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
                    "unit": name, "outcome": unit.outcome, "class": unit.class.as_str(),
                    "blocked_turns": u8::from(unit.blocked),
                    "agent_checks": {"runs": checks.0, "failed": checks.1},
                    "ignored_elsewhere": null, "seals": seals,
                    "policy_snapshot_sha256": snapshot,
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

/// The most commits a `baseline` record lists; a project that names more has none, and its
/// stops judge the commits of a session as before the frontier (F13.b).
pub(crate) const BASELINE_TIPS: usize = 100_000;

/// What was already there when a session of the project first found it (F13.b): every commit
/// the repository named then (its refs, each working tree's `HEAD`, the reflogs), and the commit
/// `HEAD` named (`None`: none yet).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Baseline {
    pub(crate) tips: Vec<String>,
    pub(crate) head: Option<String>,
}

/// What a stop reads of the ledger (F13.b): the project's baseline, the commits a stop record
/// gave a verdict (`pass`, `fail`, `fail_stands`, `hold`), and the `start` records, each a
/// session's key, its working tree and the commit it began from, in the ledger's order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Frontier {
    pub(crate) baseline: Option<Baseline>,
    pub(crate) judged: std::collections::BTreeSet<String>,
    pub(crate) starts: Vec<(String, String, String)>,
    /// The commits a stop record failed or held (`fail`, `fail_stands`, `hold`): each is out of
    /// the frontier whatever its seals.
    pub(crate) held: std::collections::BTreeSet<String>,
    /// The commits a stop record passed, each with the policy digest of every such record
    /// (`None`: a record with no seals, which a stop writes for a commit that changes nothing,
    /// so that no policy judges it).
    pub(crate) passes: std::collections::BTreeMap<String, Vec<Option<String>>>,
    /// The commits whose last stop record says they wait for their turn (`unjudged`,
    /// `time.not_asked`): a stop does not record them again while they still wait (release
    /// 0.11.1, ruling 10: one record a waiting commit at every stop could grow the ledger past
    /// what its readers read).
    pub(crate) waiting: std::collections::BTreeSet<String>,
}

impl Frontier {
    /// Release 0.11.1 (R10): a commit that only a `pass` record took out of the frontier stays
    /// out only when one of its pass records is sealed to a policy digest of `accepted`, the
    /// digests that were the project's accepted policy, or has no seals. A pass of 0.11.0 sealed
    /// to a policy a left commit wrote (the hole of 0.11.0's stated limit) puts its commit back,
    /// to be judged again; its new record is the worse one `agent verify-branch` takes. A stop
    /// with no accepted policy does not call this: every pass counts, as in 0.11.0.
    pub(crate) fn passes_sealed_to(&mut self, accepted: &std::collections::BTreeSet<String>) {
        for (id, seals) in &self.passes {
            let counts = seals
                .iter()
                .any(|seal| seal.as_ref().is_none_or(|policy| accepted.contains(policy)));
            if !counts && !self.held.contains(id) {
                self.judged.remove(id);
            }
        }
    }
}

/// The digest of what a stop took the project's accepted policy from (rulings 2 and 4 of release
/// 0.11.1): the first whole `baseline` record (its `head` and its `tips_sha256`) and the accepted
/// policy's digest, each `none` when there is none. Every stop record carries it
/// (`policy_snapshot_sha256`), so that a record says what it was judged by; comparing it again
/// under the project's lock before the append is the next package's (L1c), which adds the
/// acceptances to what it covers.
pub(crate) fn snapshot_sha256(baseline: Option<&Baseline>, accepted: Option<&str>) -> String {
    snapshot_sha256_of(baseline, None, &[], accepted)
}

/// [`snapshot_sha256`] over the whole snapshot of release 0.11.1's package L1c (ruling 4): the
/// baseline record's identity, the policy name the project's policy record binds (`None`: no
/// binding), the acceptances that applied, in order (each as the policy record holds it: its
/// name, its `from`, its `to` and its commit), and the accepted digest. Without a binding and an
/// acceptance it is [`snapshot_sha256`]'s digest.
pub(crate) fn snapshot_sha256_of(
    baseline: Option<&Baseline>,
    bound: Option<&str>,
    applied: &[crate::policy_record::Acceptance],
    accepted: Option<&str>,
) -> String {
    let baseline = baseline.map_or_else(
        || "none".to_owned(),
        |b| {
            format!(
                "{} {}",
                b.head.as_deref().unwrap_or("null"),
                tips_sha256(&b.tips)
            )
        },
    );
    let mut text = format!(
        "deep-diff-forge.policy-snapshot.v1\nbaseline {baseline}\naccepted {}\n",
        accepted.unwrap_or("none")
    );
    if let Some(bound) = bound {
        // (a name holds no control character: the policy record's reader refuses one)
        let _ = writeln!(text, "bound {bound}");
    }
    for acceptance in applied {
        let _ = writeln!(
            text,
            "applied {} {} {} {}",
            acceptance.commit,
            acceptance.from.as_deref().unwrap_or("none"),
            acceptance.to,
            acceptance.policy
        );
    }
    deep_diff_forge_core::sha256_hex(text.as_bytes())
}

/// The first whole `baseline` record of the ledger in `own`, as [`frontier`] takes it, from the
/// same read: the whole file, which must be a regular file of at most [`WHOLE`] bytes and text
/// throughout (so this errs exactly where [`frontier`] errs: release 0.11.1, ruling 10, F1),
/// with the lines parsed only up to that record. What a stop compares again under the project
/// lock before its append (ruling 4). `Ok(None)` when there is no ledger or no whole baseline.
pub(crate) fn first_baseline(own: &Path) -> Result<Option<Baseline>, String> {
    let Some(text) = whole_text(own)? else {
        return Ok(None);
    };
    for line in text.lines() {
        let Some(record) = record(line).filter(|r| r["v"] == VERSION) else {
            continue;
        };
        if record["kind"] == "baseline"
            && let Some(baseline) = baseline_of(&record)
        {
            return Ok(Some(baseline));
        }
    }
    Ok(None)
}

/// The SHA-256 of `tips` one a line, as a `baseline` record names them.
pub(crate) fn tips_sha256(tips: &[String]) -> String {
    let text = tips.iter().fold(String::new(), |mut text, tip| {
        text.push_str(tip);
        text.push('\n');
        text
    });
    deep_diff_forge_core::sha256_hex(text.as_bytes())
}

/// A commit id as Git writes one: 40 or 64 lowercase hex digits.
fn is_commit_id(text: &str) -> bool {
    matches!(text.len(), 40 | 64) && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// The baseline a `baseline` record states, when it is whole: its commits are ids, as many as
/// it counts and with the digest it names, and its `head` is an id or null.
fn baseline_of(record: &Value) -> Option<Baseline> {
    let tips: Vec<String> = record["tips"]
        .as_array()?
        .iter()
        .map(|tip| tip.as_str().filter(|t| is_commit_id(t)).map(str::to_owned))
        .collect::<Option<_>>()?;
    let counted = record["tips_count"].as_u64()? == u64::try_from(tips.len()).ok()?;
    let sealed = record["tips_sha256"] == tips_sha256(&tips).as_str();
    let head = match &record["head"] {
        Value::Null => None,
        head => Some(head.as_str().filter(|h| is_commit_id(h))?.to_owned()),
    };
    (counted && sealed && tips.len() <= BASELINE_TIPS).then_some(Baseline { tips, head })
}

/// What the ledger in `own` gives a stop ([`Frontier`]); an empty one when there is no ledger.
/// A line that is not a record, a record of another version and a record that is not whole are
/// passed over: each can only leave a commit unjudged-by-record, so judged again, never the
/// reverse. The first whole `baseline` record is the project's. `Err`: the ledger is not a
/// regular file, is larger than the most a reader reads, or cannot be read.
pub(crate) fn frontier(own: &Path) -> Result<Frontier, String> {
    let mut out = Frontier::default();
    let Some(text) = whole_text(own)? else {
        return Ok(out);
    };
    for line in text.lines() {
        let Some(record) = record(line).filter(|r| r["v"] == VERSION) else {
            continue;
        };
        match record["kind"].as_str() {
            Some("baseline") if out.baseline.is_none() => out.baseline = baseline_of(&record),
            Some("stop")
                if record["outcome"] == "unjudged"
                    && record["unit"]
                        .as_str()
                        .and_then(|unit| unit.strip_prefix("commit:"))
                        .is_some_and(is_commit_id) =>
            {
                let id = record["unit"].as_str().unwrap_or_default()["commit:".len()..].to_owned();
                if record["class"] == "time.not_asked" {
                    out.waiting.insert(id);
                } else {
                    out.waiting.remove(&id);
                }
            }
            Some("stop")
                if matches!(
                    record["outcome"].as_str(),
                    Some("pass" | "fail" | "fail_stands" | "hold")
                ) =>
            {
                if let Some(id) = record["unit"]
                    .as_str()
                    .and_then(|unit| unit.strip_prefix("commit:"))
                    .filter(|id| is_commit_id(id))
                {
                    out.waiting.remove(id);
                    out.judged.insert(id.to_owned());
                    if record["outcome"] == "pass" {
                        let sealed = record["seals"]["policy_sha256"].as_str();
                        (out.passes.entry(id.to_owned()).or_default())
                            .push(sealed.map(str::to_owned));
                    } else {
                        out.held.insert(id.to_owned());
                    }
                }
            }
            Some("start") => {
                let field = |name: &str| record[name].as_str().map(str::to_owned);
                if let (Some(session), Some(tree), Some(base)) =
                    (field("session"), field("worktree"), field("base"))
                    && is_commit_id(&base)
                {
                    out.starts.push((session, tree, base));
                }
            }
            _ => {}
        }
    }
    Ok(out)
}

/// Record that the session `session` found the project in the working tree `worktree` and
/// began from `base` (a `start` record, F13.b), and, when the ledger has no baseline yet, the
/// project's baseline first. `Ok(true)` when the baseline was written by this call.
pub(crate) fn append_start(
    own: &Path,
    session: &str,
    worktree: &str,
    base: Option<&str>,
    baseline: Option<&Baseline>,
    wait: Duration,
) -> Result<bool, String> {
    let baseline = baseline.filter(|b| b.tips.len() <= BASELINE_TIPS);
    let needs = baseline.filter(|_| matches!(frontier(own), Ok(f) if f.baseline.is_none()));
    let key = session_key(session);
    append(own, wait, |_| {
        let mut out = Vec::new();
        if let Some(baseline) = needs {
            out.push(json!({
                "v": VERSION, "kind": "baseline", "head": baseline.head,
                "tips_count": baseline.tips.len(), "tips_sha256": tips_sha256(&baseline.tips),
                "tips": baseline.tips,
            }));
        }
        if let Some(base) = base {
            out.push(json!({
                "v": VERSION, "kind": "start", "session": key, "worktree": worktree, "base": base,
            }));
        }
        out
    })?;
    Ok(needs.is_some())
}

/// Write the project's baseline when the ledger in `own` has none: at a stop whose session
/// began before the ledger had one, or after the ledger was removed. `Ok(true)` when written.
pub(crate) fn append_baseline(
    own: &Path,
    baseline: &Baseline,
    wait: Duration,
) -> Result<bool, String> {
    if frontier(own)?.baseline.is_some() || baseline.tips.len() > BASELINE_TIPS {
        return Ok(false);
    }
    append(own, wait, |_| {
        vec![json!({
            "v": VERSION, "kind": "baseline", "head": baseline.head,
            "tips_count": baseline.tips.len(), "tips_sha256": tips_sha256(&baseline.tips),
            "tips": baseline.tips,
        })]
    })?;
    Ok(true)
}

/// The key of a session in the ledger ([`session_key`]), for a reader of `start` records.
pub(crate) fn key_of(session: &str) -> String {
    session_key(session)
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

/// `agent verify-branch`'s usage.
pub(crate) const VERIFY_USAGE: &str = "deep-diff-forge agent verify-branch BASE..HEAD [--json]";
/// The most of the ledger `agent verify-branch` reads; a larger one is refused.
const WHOLE: u64 = 64 << 20;
/// What `agent verify-branch` cannot know, in its document and its text.
const CANNOT_KNOW: &str = "The ledger is a file in the repository's Git directory, written with the permissions of whoever works there, the agent included: a record is evidence that a stop decided so, not proof that the commit passes, and a ledger that is missing or was edited proves nothing. A commit made outside a session with the hooks, or in a clone, has no record here; the pull request's judge is the boundary.";

/// `agent verify-branch BASE..HEAD [--json]`: every commit of the range, as a stop lists a
/// session's commits, must have a stop record of the project's ledger that passed it, or a
/// clearance record; else exit 7 (`branch_unverified`) after the document. Reads the ledger only.
pub(crate) fn verify_branch(args: &[String]) {
    let positionals = crate::args::admit_or_exit(
        args,
        &crate::args::Spec {
            flags: &["--json"],
            valued: &[],
            positionals: 1,
        },
        VERIFY_USAGE,
    );
    let Some(range) = positionals.first().copied() else {
        eprintln!("error: the range BASE..HEAD is missing");
        eprintln!("usage: {VERIFY_USAGE}");
        std::process::exit(deep_diff_forge_core::exit::USAGE);
    };
    let mut doc = json!({
        "schema": "deep-diff-forge.agent-verify-branch.v0",
        "tool": {"name": "deep-diff-forge", "version": env!("CARGO_PKG_VERSION")},
        "range": range, "ledger": null, "commits": [], "verified": false, "unverified": null,
        "accepted_policy": null, "cannot_know": CANNOT_KNOW,
    });
    let class = deep_diff_forge_core::exit::refusal("branch_unverified");
    let read = std::env::current_dir()
        .map_err(|e| format!("the current directory cannot be read: {e}"))
        .and_then(|dir| {
            let (own, commits) = crate::hook_run::branch_commits(&dir, range)?;
            doc["ledger"] = json!(own.join(FILE).to_string_lossy());
            let records = whole(&own)?;
            // release 0.11.1 (ruling 6, F5): a pass counts as a stop's frontier counts it
            let seals = crate::hook_run::accepted_seals(&dir)?;
            Ok((records, commits, seals))
        });
    match read {
        Err(why) => {
            doc["unverified"] = json!(why);
            let text = "the range or the ledger could not be read".to_owned();
            crate::refusal::pending(&[(class, text)]);
        }
        Ok((records, commits, seals)) => {
            let set = seals.as_ref().map(|(set, _)| set);
            // the records of each unit, once (a range of many commits read every record once a
            // commit: 100,000 commits took longer than a client waits)
            let mut by_unit: std::collections::BTreeMap<&str, Vec<&Value>> =
                std::collections::BTreeMap::new();
            for record in &records {
                if let Some(unit) = record["unit"].as_str() {
                    by_unit.entry(unit).or_default().push(record);
                }
            }
            doc["accepted_policy"] = seals
                .as_ref()
                .map_or(Value::Null, |(_, words)| json!(words));
            let verdicts: Vec<Value> = commits
                .iter()
                .map(|id| {
                    let unit = format!("commit:{id}");
                    verdict(
                        by_unit.get(unit.as_str()).map_or(&[][..], Vec::as_slice),
                        id,
                        set,
                    )
                })
                .collect();
            let not = verdicts.iter().filter(|v| v["verified"] != true).count();
            doc["commits"] = json!(verdicts);
            doc["verified"] = json!(not == 0);
            if not > 0 {
                let text = format!(
                    "{not} of {} commits are not verified by the ledger",
                    commits.len()
                );
                crate::refusal::pending(&[(class, text)]);
            }
        }
    }
    if args.iter().any(|a| a == "--json") {
        crate::emit_pretty_json(&doc);
    } else {
        emitln!("{}", verify_text(&doc));
    }
    crate::refusal::finish();
}

/// What the ledger says of the commit `id`: `cleared` by a clearance record (a convention), else
/// the worst outcome a stop recorded (`fail`, `fail_stands`, `hold`), else `pass` when a stop
/// passed it, else `unjudged`, else `no_record`. Only `pass` and a clearance verify it. With it,
/// the runs of `agent check --session` recorded before the first stop that gave it a verdict.
///
/// Release 0.11.1 (ruling 6, F5): with `accepted` (the digests that were the project's accepted
/// policy) a `pass` record counts only when it is sealed to one of them, or has no seals (a
/// commit that changes nothing), as a stop's frontier counts it (R10); the others are counted
/// (`passes_not_accepted`) and passed over.
fn verdict(
    records: &[&Value],
    id: &str,
    accepted: Option<&std::collections::BTreeSet<String>>,
) -> Value {
    let mine = records.iter().copied();
    let counts = |r: &&Value| {
        r["outcome"] != "pass"
            || accepted.is_none_or(|set| {
                r["seals"]["policy_sha256"]
                    .as_str()
                    .is_none_or(|policy| set.contains(policy))
            })
    };
    let not_accepted = mine
        .clone()
        .filter(|r| r["kind"] == "stop" && !counts(r))
        .count();
    let stops = mine.clone().filter(|r| r["kind"] == "stop").filter(counts);
    let has = |outcome: &str| stops.clone().any(|r| r["outcome"] == outcome);
    let outcome = OUTCOMES
        .into_iter()
        .find(|outcome| has(outcome))
        .unwrap_or("no_record");
    let cleared = mine.clone().any(|r| r["kind"] == "clearance");
    let checks = stops
        .clone()
        .find(|r| r["outcome"] != "unjudged")
        .map_or(Value::Null, |r| r["agent_checks"].clone());
    json!({
        "commit": id, "outcome": outcome, "cleared": cleared,
        "verified": cleared || outcome == "pass", "records": stops.count(),
        "agent_checks": checks, "passes_not_accepted": not_accepted,
    })
}

/// The text form of `agent verify-branch`.
fn verify_text(doc: &Value) -> String {
    let mut out = Vec::new();
    for commit in doc["commits"].as_array().into_iter().flatten() {
        let id: String = commit["commit"]
            .as_str()
            .unwrap_or_default()
            .chars()
            .take(12)
            .collect();
        let mut line = format!("{id} {}", commit["outcome"].as_str().unwrap_or_default());
        if commit["cleared"] == true {
            line.push_str(", cleared by a clearance record (a convention, not a verdict)");
        }
        if let Some(passes) = commit["passes_not_accepted"].as_u64().filter(|n| *n > 0) {
            let (records, are) = if passes == 1 {
                ("1 pass record".to_owned(), "is")
            } else {
                (format!("{passes} pass records"), "are")
            };
            let _ = write!(
                line,
                "; {records} sealed to a policy that was never this project's accepted policy {are} not counted"
            );
        }
        if let Some(runs) = commit["agent_checks"]["runs"].as_u64() {
            let _ = write!(
                line,
                "; checked {runs} times with agent check --session before its stop, {} failed",
                commit["agent_checks"]["failed"]
            );
        }
        out.push(line);
    }
    if let Some(why) = doc["unverified"].as_str() {
        out.push(format!(
            "UNVERIFIED: {}",
            deep_diff_forge_core::display_safe(why)
        ));
    } else {
        let all = doc["commits"].as_array().map_or(0, Vec::len);
        let verified = (doc["commits"].as_array().into_iter().flatten())
            .filter(|c| c["verified"] == true)
            .count();
        out.push(format!(
            "{verified} of {all} commits are verified by the ledger."
        ));
    }
    if let Some(words) = doc["accepted_policy"].as_str() {
        out.push(words.to_owned());
    }
    out.push(CANNOT_KNOW.to_owned());
    out.join("\n")
}

/// Every record of the ledger in `own`, for `agent verify-branch`; none when there is no ledger.
/// A line that is not a record of this build (not an object, another version, or a record that
/// is not whole as its kind is written: [`is_whole_record`]), a record of a newer build, a last
/// line that is not whole, a ledger over [`WHOLE`] or anything but a regular file at its name:
/// refused whole, so a damaged record never reads as a missing one.
fn whole(own: &Path) -> Result<Vec<Value>, String> {
    let shown = crate::hook::shown_path(&own.join(FILE).to_string_lossy());
    let Some(text) = whole_text(own)? else {
        return Ok(Vec::new());
    };
    if !text.is_empty() && !text.ends_with('\n') {
        return Err(format!("the last line of the ledger {shown} is not whole"));
    }
    let mut records = Vec::new();
    for (at, line) in text
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.is_empty())
    {
        let damaged = || {
            format!(
                "line {} of the ledger {shown} is not a record of this build",
                at + 1
            )
        };
        let record = record(line).ok_or_else(damaged)?;
        match record["v"].as_u64() {
            Some(VERSION) => {}
            Some(newer) if newer > VERSION => {
                return Err(format!(
                    "the ledger {shown} holds records of a newer build (v{newer}; this one reads v{VERSION})"
                ));
            }
            _ => return Err(damaged()),
        }
        if !is_whole_record(&record) {
            return Err(damaged());
        }
        records.push(record);
    }
    Ok(records)
}

/// The outcomes a stop record gives a unit, the worst first, as [`verdict`] ranks them.
const OUTCOMES: [&str; 5] = ["fail", "fail_stands", "hold", "pass", "unjudged"];

/// Whether `record` (an object of this build's `v`) is whole as its kind is written: every
/// member this build writes for the kind is there, of its type and with a value this build
/// writes, so [`verdict`] never meets a record whose `unit` or `outcome` it cannot read. The
/// kinds and their writers: `stop` ([`append_stop`]), `check` ([`append_check`]), `baseline`
/// and `start` ([`append_start`], [`append_baseline`]); a `clearance` is written by a person,
/// and its documented form is `v`, `kind` and a `unit` that names a commit. A member this build
/// does not write is not read, and is passed over. A stop's members must also agree as the stop
/// writes them: the outcome is the one of its class ([`outcome_of`]); a blocked turn only for a
/// class that holds a stop, and always for `fail`; a commit's and the stop's `worktree` null, a
/// tree's a string; seals never on the unit `stop`, a tree's without the patch's and the
/// verdict's digests, a commit's with the patch's. A stop's `policy_snapshot_sha256` (release
/// 0.11.1) is a SHA-256 where it is there; a record of 0.11.0 has none.
fn is_whole_record(record: &Value) -> bool {
    let has = |name: &str| record.get(name);
    let text = |name: &str| has(name).and_then(Value::as_str);
    let count = |value: Option<&Value>| value.and_then(Value::as_u64);
    let text_or_null = |value: Option<&Value>| value.is_some_and(|v| v.is_null() || v.is_string());
    let session = || text("session").is_some_and(is_session_key);
    let commit = |unit: &str| unit.strip_prefix("commit:").is_some_and(is_commit_id);
    match text("kind") {
        Some("stop") => {
            let checks = has("agent_checks");
            let runs = count(checks.and_then(|c| c.get("runs")));
            let failed = count(checks.and_then(|c| c.get("failed")));
            let unit = text("unit").unwrap_or_default();
            let tree = is_tree_unit(unit);
            let class = text("class").and_then(crate::hook::reason::ReasonClass::from_code);
            let outcome = text("outcome");
            let blocked = count(has("blocked_turns"));
            let seals = has("seals").is_some_and(|seals| {
                seals.is_null()
                    || (unit != "stop"
                        && seals.get("policy_sha256").is_some_and(Value::is_string)
                        && seals.get("ranker_sha256").is_some_and(Value::is_string)
                        && (seals.get("patch_sha256").is_some_and(Value::is_null) == tree)
                        && text_or_null(seals.get("patch_sha256"))
                        && seals
                            .get("verdict_sha256")
                            .is_some_and(|v| v.is_null() || !tree)
                        && text_or_null(seals.get("verdict_sha256")))
            });
            session()
                && (unit == "stop" || commit(unit) || tree)
                && (if tree {
                    text("worktree").is_some()
                } else {
                    has("worktree").is_some_and(Value::is_null)
                })
                && class.is_some_and(|class| outcome == Some(outcome_of(class)))
                && matches!(blocked, Some(0 | 1))
                && (blocked == Some(0) || class.is_some_and(|class| class.row().2 != "never"))
                && (outcome != Some("fail") || blocked == Some(1))
                && matches!((runs, failed), (Some(runs), Some(failed)) if failed <= runs)
                && has("ignored_elsewhere").is_some_and(Value::is_null)
                && seals
                // (0.11.1: absent in a record of 0.11.0, a digest in this build's)
                && has("policy_snapshot_sha256").is_none_or(|v| v.as_str().is_some_and(is_sha256))
        }
        Some("check") => {
            session()
                && matches!((count(has("units")), count(has("failed"))),
                    (Some(units), Some(failed)) if failed <= units)
        }
        Some("baseline") => has("head").is_some() && baseline_of(record).is_some(),
        Some("start") => {
            session() && text("worktree").is_some() && text("base").is_some_and(is_commit_id)
        }
        Some("clearance") => text("unit").is_some_and(commit),
        _ => false,
    }
}

/// The outcome a stop records for a unit of `class`, by its family in the registry: `pass`, a
/// failure that blocks (`fail.new`) or stands (`fail.stands`, `fail.unsaved_continuing`), a hold
/// (`hold.*`), else `unjudged` ([`crate::hook::class_of`] and the stops' own records).
fn outcome_of(class: crate::hook::reason::ReasonClass) -> &'static str {
    match class.as_str() {
        "pass" => "pass",
        "fail.new" => "fail",
        code if code.starts_with("fail.") => "fail_stands",
        code if code.starts_with("hold.") => "hold",
        _ => "unjudged",
    }
}

/// A SHA-256 as a record gives it: 64 lowercase hex digits.
fn is_sha256(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// A session's key as a record gives it ([`session_key`]): 16 lowercase hex digits.
fn is_session_key(text: &str) -> bool {
    text.len() == 16 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// A tree's unit as [`append_stop`] writes it: `tree:<digest>:<kind>`, the digest 32 lowercase
/// hex digits or `unread`, the kind `uncommitted` or `staged`.
fn is_tree_unit(unit: &str) -> bool {
    let Some((digest, kind)) = unit
        .strip_prefix("tree:")
        .and_then(|rest| rest.split_once(':'))
    else {
        return false;
    };
    let salted = digest.len() == 32
        && digest
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
    (salted || digest == "unread") && matches!(kind, "uncommitted" | "staged")
}

/// The text of the ledger in `own`, read whole; `None` when there is none. Anything but a
/// regular file at its name, one over [`WHOLE`] or one that cannot be read is an error.
fn whole_text(own: &Path) -> Result<Option<String>, String> {
    use std::os::unix::fs::OpenOptionsExt as _;
    let path = own.join(FILE);
    let shown = crate::hook::shown_path(&path.to_string_lossy());
    let file = match std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK)
        .open(&path)
    {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("the ledger {shown} could not be opened: {e}")),
    };
    match file.metadata() {
        Ok(found) if found.is_file() && found.len() <= WHOLE => {}
        Ok(found) if found.is_file() => {
            return Err(format!("the ledger {shown} is larger than {WHOLE} bytes"));
        }
        _ => return Err(format!("the ledger {shown} is not a regular file")),
    }
    let mut text = String::new();
    file.take(WHOLE)
        .read_to_string(&mut text)
        .map_err(|e| format!("the ledger {shown} could not be read: {e}"))?;
    Ok(Some(text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hook::reason::ReasonClass;
    use deep_diff_forge_testkit::Scratch;

    const ID: &str = "0123456789abcdef0123456789abcdef01234567";
    const OTHER: &str = "89abcdef0123456789abcdef0123456789abcdef";
    const WAIT: Duration = Duration::from_secs(5);

    /// A ledger written by every writer of this build in a scratch directory: a stop record for
    /// every reason class with the outcome a stop gives it, the unit names in turn (seals of a
    /// commit, of a tree, with and without the verdict's digest, and none; none on the unit
    /// `stop`), blocked where the class holds a stop (both ways for `hold.version_gap`), a check,
    /// a baseline and a start, then the minimal clearance the reference documents, as a person
    /// appends it.
    fn written(label: &str) -> (Scratch, std::path::PathBuf, String) {
        let scratch = Scratch::new(&format!("ledger-unit-{label}"));
        let own = scratch.path().to_path_buf();
        let names = [
            Name::Commit(ID.to_owned()),
            Name::Tree {
                patch: Some("d".repeat(64)),
                kind: "uncommitted",
                tree: "m".to_owned(),
            },
            Name::Tree {
                patch: None,
                kind: "staged",
                tree: "x0123456789abcde".to_owned(),
            },
            Name::Stop,
        ];
        let mut units: Vec<Unit> = (ReasonClass::ALL.iter().enumerate())
            .map(|(at, class)| Unit {
                name: names[at % names.len()].clone(),
                outcome: outcome_of(*class),
                class: *class,
                seals: (at % 3 != 0 && at % names.len() != 3).then(|| Seals {
                    policy: "a".repeat(64),
                    patch: "b".repeat(64),
                    ranker: "c".repeat(64),
                    verdict: (at % 2 == 0).then(|| "e".repeat(64)),
                }),
                blocked: *class == ReasonClass::FailNew
                    || (class.row().2 != "never" && at % 2 == 0),
            })
            .collect();
        units.push(Unit {
            name: Name::Commit("fedcba9876543210fedcba9876543210fedcba98".to_owned()),
            outcome: "hold",
            class: ReasonClass::HoldVersionGap,
            seals: None,
            blocked: true,
        });
        append_stop(&own, "session-a", &units, &"5".repeat(64), WAIT).unwrap();
        append_check(&own, "session-a", 3, 1, WAIT).unwrap();
        let baseline = Baseline {
            tips: vec![ID.to_owned(), OTHER.to_owned()],
            head: Some(ID.to_owned()),
        };
        assert!(append_start(&own, "session-b", "m", Some(ID), Some(&baseline), WAIT).unwrap());
        let mut text = std::fs::read_to_string(own.join(FILE)).unwrap();
        let _ = writeln!(
            text,
            "{{\"v\":1,\"kind\":\"clearance\",\"unit\":\"commit:{ID}\"}}"
        );
        std::fs::write(own.join(FILE), &text).unwrap();
        (scratch, own, text)
    }

    /// The first record of `kind` in `text` that `pick` takes.
    fn first(text: &str, kind: &str, pick: impl Fn(&Value) -> bool) -> Value {
        text.lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .find(|r| r["kind"] == kind && pick(r))
            .unwrap_or_else(|| panic!("no {kind} record: {text}"))
    }

    /// `whole` of a ledger that is `text` and then `record`.
    fn read_with(own: &Path, text: &str, record: &Value) -> Result<Vec<Value>, String> {
        std::fs::write(own.join(FILE), format!("{text}{record}\n")).map_err(|e| e.to_string())?;
        whole(own)
    }

    /// Every record each writer of this build writes is whole, and so is the minimal clearance;
    /// the ledger reads in full, one record a line.
    #[test]
    fn every_record_this_build_writes_is_whole() {
        let (_scratch, own, text) = written("every-writer");
        let records = whole(&own).unwrap_or_else(|e| panic!("{e}: {text}"));
        assert_eq!(records.len(), ReasonClass::ALL.len() + 5, "{text}");
        for kind in ["stop", "check", "baseline", "start", "clearance"] {
            assert!(records.iter().any(|r| r["kind"] == kind), "{kind}: {text}");
        }
        for outcome in OUTCOMES {
            assert!(records.iter().any(|r| r["outcome"] == outcome), "{outcome}");
        }
    }

    /// Each member a record of its kind is written with: (kind, the member, a value of a wrong
    /// type, values of the right type that this build does not write).
    const MEMBERS: &[(&str, &str, &str, &[&str])] = &[
        (
            "stop",
            "/session",
            "7",
            &["\"ABCDEF0123456789\"", "\"0123\""],
        ),
        ("stop", "/worktree", "1", &[]),
        (
            "stop",
            "/unit",
            "1",
            &[
                "\"later\"",
                "\"commit:0123\"",
                "\"tree:unread:both\"",
                "\"tree:0123:staged\"",
                "\"Stop\"",
            ],
        ),
        ("stop", "/outcome", "1", &["\"held\"", "\"PASS\"", "\"\""]),
        ("stop", "/class", "null", &["\"pass.maybe\"", "\"\""]),
        ("stop", "/blocked_turns", "\"1\"", &["2"]),
        ("stop", "/agent_checks", "[]", &[]),
        ("stop", "/agent_checks/runs", "\"0\"", &["-1"]),
        ("stop", "/agent_checks/failed", "\"0\"", &["5"]),
        ("stop", "/ignored_elsewhere", "0", &[]),
        ("stop", "/seals", "\"sealed\"", &[]),
        ("stop", "/seals/policy_sha256", "1", &[]),
        ("stop", "/seals/ranker_sha256", "null", &[]),
        ("stop", "/seals/patch_sha256", "1", &[]),
        ("stop", "/seals/verdict_sha256", "1", &[]),
        ("check", "/session", "null", &["\"session-a\""]),
        ("check", "/units", "\"3\"", &["-1"]),
        ("check", "/failed", "\"1\"", &["4"]),
        ("baseline", "/head", "1", &["\"HEAD\""]),
        ("baseline", "/tips", "\"x\"", &["[\"abc\"]"]),
        ("baseline", "/tips_count", "\"2\"", &["3"]),
        ("baseline", "/tips_sha256", "1", &["\"00\""]),
        ("start", "/session", "1", &["\"\""]),
        ("start", "/worktree", "null", &[]),
        ("start", "/base", "1", &["\"abc\"", "\"HEAD\""]),
        (
            "clearance",
            "/unit",
            "1",
            &["\"commit:0123\"", "\"stop\"", "\"commit:\""],
        ),
    ];

    /// Each member a record of its kind is written with, missing, of a wrong type, or with a
    /// value this build does not write: the ledger is refused whole, with the line named.
    #[test]
    fn a_record_missing_a_member_or_with_a_wrong_or_unknown_value_refuses_the_ledger() {
        let (_scratch, own, text) = written("damaged");
        let commit = format!("commit:{ID}");
        let stop = first(&text, "stop", |r| {
            r["unit"] == commit.as_str() && r["seals"]["verdict_sha256"].is_string()
        });
        let samples = [
            ("stop", stop),
            ("check", first(&text, "check", |_| true)),
            ("baseline", first(&text, "baseline", |_| true)),
            ("start", first(&text, "start", |_| true)),
            ("clearance", first(&text, "clearance", |_| true)),
        ];
        let line = text.lines().count() + 1;
        let refused = |what: &str, record: &Value| {
            let Err(why) = read_with(&own, &text, record) else {
                panic!("{what}: the ledger was read, not refused: {record}")
            };
            assert!(
                why.contains(&format!("line {line} of the ledger"))
                    && why.contains("is not a record of this build"),
                "{what}: {why}"
            );
        };
        for (kind, sample) in &samples {
            let common: [(&str, &str, &[&str]); 2] = [
                ("/v", "\"1\"", &["0"]),
                ("/kind", "1", &["\"Stop\"", "\"pass\""]),
            ];
            let mut all = common.to_vec();
            all.extend(
                MEMBERS
                    .iter()
                    .filter(|m| m.0 == *kind)
                    .map(|m| (m.1, m.2, m.3)),
            );
            for (member, wrong, unknown) in all {
                let (parent, name) = member.rsplit_once('/').unwrap();
                let mut missing = sample.clone();
                missing
                    .pointer_mut(parent)
                    .and_then(Value::as_object_mut)
                    .and_then(|object| object.remove(name))
                    .unwrap_or_else(|| panic!("{kind} has no {member}: {sample}"));
                refused(&format!("{kind} without {member}"), &missing);
                for value in std::iter::once(&wrong).chain(unknown) {
                    let mut changed = sample.clone();
                    *changed.pointer_mut(member).unwrap() = serde_json::from_str(value).unwrap();
                    refused(&format!("{kind} with {member} {value}"), &changed);
                }
            }
        }
        // a record of a newer build is still refused as one
        let mut newer = samples[0].1.clone();
        newer["v"] = json!(2);
        let why = read_with(&own, &text, &newer).map(|_| ()).unwrap_err();
        assert!(why.contains("holds records of a newer build (v2"), "{why}");
    }

    /// A member of a record set to a value written as JSON.
    type Change = (&'static str, &'static str);

    /// A stop record whose members are each valid but do not agree as a stop writes them, as
    /// (what, the sample it changes, the members set to JSON values; `@commit` and `@tree` are
    /// those samples' seals).
    const DISAGREE: &[(&str, &str, &[Change])] = &[
        (
            "pass with fail.new",
            "commit",
            &[("outcome", "\"pass\""), ("class", "\"fail.new\"")],
        ),
        (
            "pass with hold.rule",
            "commit",
            &[("outcome", "\"pass\""), ("class", "\"hold.rule\"")],
        ),
        (
            "hold with pass",
            "commit",
            &[("outcome", "\"hold\""), ("class", "\"pass\"")],
        ),
        (
            "unjudged with pass",
            "commit",
            &[("outcome", "\"unjudged\""), ("class", "\"pass\"")],
        ),
        (
            "fail with fail.stands",
            "commit",
            &[
                ("outcome", "\"fail\""),
                ("class", "\"fail.stands\""),
                ("blocked_turns", "1"),
            ],
        ),
        (
            "fail_stands with fail.new",
            "commit",
            &[("outcome", "\"fail_stands\""), ("class", "\"fail.new\"")],
        ),
        (
            "hold with state.damaged",
            "commit",
            &[("outcome", "\"hold\""), ("class", "\"state.damaged\"")],
        ),
        (
            "unjudged with hold.rule",
            "commit",
            &[("outcome", "\"unjudged\""), ("class", "\"hold.rule\"")],
        ),
        (
            "a blocked pass",
            "commit",
            &[
                ("outcome", "\"pass\""),
                ("class", "\"pass\""),
                ("blocked_turns", "1"),
            ],
        ),
        (
            "a blocked hold.rule",
            "commit",
            &[
                ("outcome", "\"hold\""),
                ("class", "\"hold.rule\""),
                ("blocked_turns", "1"),
            ],
        ),
        (
            "a fail not blocked",
            "commit",
            &[
                ("outcome", "\"fail\""),
                ("class", "\"fail.new\""),
                ("blocked_turns", "0"),
            ],
        ),
        (
            "a commit with a worktree",
            "commit",
            &[("worktree", "\"m\"")],
        ),
        (
            "a tree with a null worktree",
            "tree",
            &[("worktree", "null")],
        ),
        ("seals on the unit stop", "stop", &[("seals", "@commit")]),
        (
            "a tree's seals with a patch digest",
            "tree",
            &[("seals", "@commit")],
        ),
        (
            "a commit's seals with no patch digest",
            "commit",
            &[("seals", "@tree")],
        ),
    ];

    /// [`DISAGREE`]: each such record refuses the ledger whole, with the line named (the
    /// coordinator's probe of the fix round: a pass whose class says it failed verified the
    /// branch).
    #[test]
    fn a_stop_record_whose_members_disagree_refuses_the_ledger() {
        let (_scratch, own, text) = written("disagree");
        let line = text.lines().count() + 1;
        let samples = [
            (
                "commit",
                first(&text, "stop", |r| {
                    r["unit"] == format!("commit:{ID}").as_str()
                        && r["seals"]["verdict_sha256"].is_string()
                }),
            ),
            (
                "tree",
                first(&text, "stop", |r| {
                    r["unit"].as_str().is_some_and(|u| u.starts_with("tree:"))
                        && r["seals"].is_object()
                }),
            ),
            ("stop", first(&text, "stop", |r| r["unit"] == "stop")),
        ];
        let sample = |name: &str| {
            samples
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, r)| r.clone())
                .unwrap()
        };
        for (what, of, changes) in DISAGREE {
            let mut record = sample(of);
            for (member, value) in *changes {
                record[*member] = match value.strip_prefix('@') {
                    Some(seals) => sample(seals)["seals"].clone(),
                    None => serde_json::from_str(value).unwrap(),
                };
            }
            let Err(why) = read_with(&own, &text, &record) else {
                panic!("{what}: the ledger was read, not refused: {record}")
            };
            assert!(
                why.contains(&format!("line {line} of the ledger")),
                "{what}: {why}"
            );
        }
    }

    /// The forms the reference calls valid stay whole: the minimal clearance (and one with a
    /// note of the person's), the unit `stop` and a tree's unit with its digest or `unread`, a
    /// null worktree, null seals and a tree's seals without the patch's and the verdict's
    /// digests, a baseline whose `head` is null; a member this build does not write is passed
    /// over.
    #[test]
    fn the_documented_forms_and_null_members_are_whole() {
        let (_scratch, own, text) = written("documented");
        let accepted = |what: &str, record: &Value| {
            read_with(&own, &text, record).unwrap_or_else(|e| panic!("{what}: {e}"));
        };
        let mut clearance = json!({"v": 1, "kind": "clearance", "unit": format!("commit:{OTHER}")});
        accepted("minimal clearance", &clearance);
        clearance["by"] = json!("a person");
        accepted("clearance with a note", &clearance);
        for unit in [
            "stop",
            "tree:unread:staged",
            "tree:0123456789abcdef0123456789abcdef:uncommitted",
        ] {
            let mut stop = first(&text, "stop", |r| {
                r["unit"].as_str().is_some_and(|u| u.starts_with("commit:"))
            });
            stop["unit"] = json!(unit);
            stop["worktree"] = if unit == "stop" {
                Value::Null
            } else {
                json!("m")
            };
            stop["seals"] = Value::Null;
            accepted(unit, &stop);
        }
        // what a writer can make and verdict does not read stays whole: a pass with no seals, a
        // commit's seals with no verdict digest, a pass on the unit `stop`, and a blocked turn
        // for each class that holds a stop, of the outcome of its class
        let mut pass = first(&text, "stop", |r| r["outcome"] == "pass");
        pass["seals"] = Value::Null;
        accepted("a pass with no seals", &pass);
        pass["unit"] = json!("stop");
        accepted("a pass on the unit stop", &pass);
        let mut sealed = first(&text, "stop", |r| {
            r["seals"]["verdict_sha256"].is_string()
                && r["unit"].as_str().is_some_and(|u| u.starts_with("commit:"))
        });
        sealed["seals"]["verdict_sha256"] = Value::Null;
        accepted("a commit's seals with no verdict digest", &sealed);
        for (class, outcome) in [
            ("fail.new", "fail"),
            ("hold.version_gap", "hold"),
            ("work.unreadable", "unjudged"),
            ("work.skip_worktree", "unjudged"),
        ] {
            let mut blocked = first(&text, "stop", |_| true);
            (
                blocked["class"],
                blocked["outcome"],
                blocked["blocked_turns"],
            ) = (json!(class), json!(outcome), json!(1));
            accepted(class, &blocked);
        }
        let tree_seals = first(&text, "stop", |r| {
            r["unit"].as_str().is_some_and(|u| u.starts_with("tree:")) && r["seals"].is_object()
        });
        assert!(
            tree_seals["seals"]["patch_sha256"].is_null(),
            "{tree_seals}"
        );
        accepted("tree seals", &tree_seals);
        let mut baseline = first(&text, "baseline", |_| true);
        baseline["head"] = Value::Null;
        accepted("baseline with no head", &baseline);
        let mut extra = first(&text, "stop", |_| true);
        extra["later_member"] = json!({"any": "value"});
        accepted("a member this build does not write", &extra);
    }

    /// The frontier's reader stays conservative: a stop record with no outcome, which `whole`
    /// refuses, is passed over there, so its commit is judged again, and the frontier is read.
    #[test]
    fn the_frontier_passes_over_a_record_that_verify_branch_refuses() {
        let (_scratch, own, text) = written("frontier");
        let mut damaged = first(&text, "stop", |r| r["outcome"] == "pass");
        damaged["unit"] = json!(format!("commit:{OTHER}"));
        damaged.as_object_mut().unwrap().remove("outcome");
        assert!(read_with(&own, &text, &damaged).is_err());
        let found = frontier(&own).unwrap_or_else(|e| panic!("the frontier was refused: {e}"));
        assert!(!found.judged.contains(OTHER), "{found:?}");
        assert!(found.judged.contains(ID), "{found:?}");
        assert!(found.baseline.is_some(), "{found:?}");
        damaged["outcome"] = json!("pass");
        assert!(read_with(&own, &text, &damaged).is_ok());
        assert!(frontier(&own).unwrap().judged.contains(OTHER));
    }

    /// Release 0.11.1, ruling 4: every stop record of this build carries the digest of what the
    /// stop took the accepted policy from; a record of 0.11.0 has none and stays whole (N109);
    /// one that names it as anything but a digest refuses the ledger.
    #[test]
    fn the_policy_snapshot_is_written_read_and_optional() {
        let (_scratch, own, text) = written("snapshot");
        let stop = first(&text, "stop", |_| true);
        assert_eq!(
            stop["policy_snapshot_sha256"],
            json!("5".repeat(64)),
            "{stop}"
        );
        let mut old = stop.clone();
        old.as_object_mut()
            .unwrap()
            .remove("policy_snapshot_sha256");
        read_with(&own, &text, &old).unwrap_or_else(|e| panic!("a 0.11.0 record: {e}"));
        for wrong in [
            json!(1),
            json!(null),
            json!("5".repeat(63)),
            json!("G".repeat(64)),
        ] {
            let mut record = stop.clone();
            record["policy_snapshot_sha256"] = wrong.clone();
            assert!(read_with(&own, &text, &record).is_err(), "{wrong}");
        }
        // the digest names the baseline and the accepted digest, each or none
        let baseline = Baseline {
            tips: vec![ID.to_owned()],
            head: Some(ID.to_owned()),
        };
        let digests = [
            snapshot_sha256(None, None),
            snapshot_sha256(Some(&baseline), None),
            snapshot_sha256(Some(&baseline), Some(&"a".repeat(64))),
            snapshot_sha256(Some(&baseline), Some(&"b".repeat(64))),
        ];
        let distinct: std::collections::BTreeSet<&String> = digests.iter().collect();
        assert_eq!(distinct.len(), digests.len(), "{digests:?}");
    }

    /// Release 0.11.1, R10: a commit that only a pass took out of the frontier stays out only when
    /// a pass of it is sealed to an accepted digest, or has no seals (a commit that changes
    /// nothing); a fail or a hold keeps it out whatever its seals.
    #[test]
    fn a_pass_sealed_to_a_policy_that_was_never_accepted_is_judged_again() {
        let scratch = Scratch::new("ledger-unit-sealed-passes");
        let own = scratch.path().to_path_buf();
        let ids: Vec<String> = (1..=5).map(|n| format!("{n:040x}")).collect();
        let sealed = |policy: Option<&str>| {
            policy.map(|policy| Seals {
                policy: policy.to_owned(),
                patch: "b".repeat(64),
                ranker: "c".repeat(64),
                verdict: None,
            })
        };
        let unit = |id: &str, outcome: &'static str, class, policy: Option<&str>| Unit {
            name: Name::Commit(id.to_owned()),
            outcome,
            class,
            seals: sealed(policy),
            blocked: false,
        };
        let (accepted, other) = ("a".repeat(64), "f".repeat(64));
        let units = [
            unit(&ids[0], "pass", ReasonClass::Pass, Some(&accepted)),
            unit(&ids[1], "pass", ReasonClass::Pass, Some(&other)),
            unit(&ids[2], "pass", ReasonClass::Pass, None),
            unit(&ids[3], "pass", ReasonClass::Pass, Some(&other)),
            unit(&ids[3], "hold", ReasonClass::HoldRule, Some(&other)),
            unit(&ids[4], "pass", ReasonClass::Pass, Some(&other)),
            unit(&ids[4], "pass", ReasonClass::Pass, Some(&accepted)),
        ];
        append_stop(&own, "session-a", &units, &"5".repeat(64), WAIT).unwrap();
        let mut found = frontier(&own).unwrap();
        assert_eq!(found.judged.len(), 5, "{found:?}");
        found.passes_sealed_to(&std::collections::BTreeSet::from([accepted]));
        let out: Vec<&String> = ids.iter().filter(|id| found.judged.contains(*id)).collect();
        assert_eq!(out, vec![&ids[0], &ids[2], &ids[3], &ids[4]], "{found:?}");
    }
}
