//! The project's policy record (release 0.11.1, package L1c): `deep-diff-forge/policy.jsonl`
//! beside the ledger in the repository's Git common directory, one JSON object a line, appended
//! only (`O_APPEND`, mode 0600, never through a link). Its first record binds the name of the
//! policy file the hooks judge by to the project, beside the ledger's first record (the
//! baseline) it was made with; each record after it is an acceptance that the owner's command
//! `agent policy-accept` appended, which the hooks apply as a compare-and-swap.
//!
//! **This is a local convention, not authentication.** Any process running as this user can
//! write this file or run the command that appends to it, and removing it returns the project
//! to the policy of the ledger's first record (which is never below 0.11.0: the policy at a
//! session's base is still judged by). The pull request's judge under the target branch's
//! policy is the authority.
//!
//! The ledger's form and version do not change with it: 0.11.0 never reads this file.
//!
//! **One lock for both files** ([`lock`]): `deep-diff-forge/project.lock`. A stop takes it to read
//! the ledger and this record (and to write the binding where there is none), and again from
//! before its records are made until after its ledger append; `agent policy-accept` takes it for
//! its compare-and-swap and its append. **Lock order:** a session's own lock (the stop's state),
//! then the project lock, then the ledger's own lock (taken inside `ledger::append`); never the
//! reverse. `agent policy-accept` takes the project lock alone. Every wait is bounded, so no
//! two of them can wait on each other for ever.

use serde_json::{Value, json};
use std::io::{Read as _, Seek as _, Write as _};
use std::path::Path;
use std::time::Duration;

/// The record's file in the hooks' own directory.
pub(crate) const FILE: &str = "policy.jsonl";
/// The project lock's file in the hooks' own directory ([`lock`]).
pub(crate) const LOCK: &str = "project.lock";
/// The version of every record of this file this build writes and reads.
const VERSION: u64 = 1;
/// The most of the file a reader reads; a larger one cannot be read whole.
const WHOLE: u64 = 4 << 20;
/// The longest policy name a record may carry.
const NAME_BYTES: usize = 4096;

/// The first record of the file: the policy name the project's hooks judge by, bound on first
/// use beside the ledger's first record (its `head` and the digest of its tips), with the digest
/// of that policy file at that head (`None`: no file there, or no head).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Binding {
    pub(crate) policy: String,
    pub(crate) baseline_head: Option<String>,
    pub(crate) baseline_tips_sha256: String,
    pub(crate) policy_sha256: Option<String>,
}

/// One acceptance: from the accepted digest `from` (`None`: there was none) to `to`, the digest
/// of the file `policy` at the commit `commit`. `line` is its line in the file, from one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Acceptance {
    pub(crate) policy: String,
    pub(crate) from: Option<String>,
    pub(crate) to: String,
    pub(crate) commit: String,
    pub(crate) line: usize,
}

/// The file as a reader read it whole.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Record {
    /// The first whole binding, with its line.
    pub(crate) binding: Option<(Binding, usize)>,
    /// The acceptances after the binding, in the file's order.
    pub(crate) acceptances: Vec<Acceptance>,
    /// Lines that are no record that counts, each with why: a line that is not whole (a write
    /// that did not finish), a second binding, an acceptance before the binding.
    pub(crate) passed_over: Vec<(usize, &'static str)>,
    /// The SHA-256 of the bytes read.
    pub(crate) sha256: String,
}

/// A commit id as Git writes one: 40 or 64 lowercase hex digits.
fn is_commit_id(text: &str) -> bool {
    matches!(text.len(), 40 | 64) && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// A SHA-256 as a record gives it: 64 lowercase hex digits.
fn is_sha256(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// A policy name a record may carry: a path of the repository, not empty, with no control
/// character, at most [`NAME_BYTES`].
pub(crate) fn is_name(text: &str) -> bool {
    !text.is_empty() && text.len() <= NAME_BYTES && !text.chars().any(char::is_control)
}

/// What one line of the file is.
enum Line {
    Binding(Binding),
    Acceptance(Acceptance),
}

/// One line as a record of this file: `Ok(None)` for a line that is not JSON at all (a cut
/// JSON object never parses: a write that did not finish); `Err` for a JSON line that is not a
/// whole record of this build (another version or kind, a member missing, extra, of another
/// type, or given twice), which makes the file one that cannot be read whole.
fn line_of(text: &str, at: usize) -> Result<Option<Line>, String> {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return Ok(None);
    };
    let not = || format!("line {at} is not a record of this build");
    if crate::hook::has_duplicate_key(text) {
        return Err(not());
    }
    let Some(object) = value.as_object() else {
        return Err(not());
    };
    if object.get("v").and_then(Value::as_u64) != Some(VERSION) {
        return Err(match object.get("v").and_then(Value::as_u64) {
            Some(newer) if newer > VERSION => format!(
                "line {at} is a record of a newer build (v{newer}; this one reads v{VERSION})"
            ),
            _ => not(),
        });
    }
    let text_of = |name: &str| object.get(name).and_then(Value::as_str);
    let digest_or_null = |name: &str| match object.get(name) {
        Some(Value::Null) => Some(None),
        Some(Value::String(s)) if is_sha256(s) => Some(Some(s.clone())),
        _ => None,
    };
    let members = |names: &[&str]| {
        object.len() == names.len() && names.iter().all(|name| object.contains_key(*name))
    };
    let policy = text_of("policy").filter(|name| is_name(name));
    match text_of("kind") {
        Some("binding")
            if members(&[
                "v",
                "kind",
                "policy",
                "baseline_head",
                "baseline_tips_sha256",
                "policy_sha256",
            ]) =>
        {
            let head = match object.get("baseline_head") {
                Some(Value::Null) => Some(None),
                Some(Value::String(s)) if is_commit_id(s) => Some(Some(s.clone())),
                _ => None,
            };
            match (
                policy,
                head,
                text_of("baseline_tips_sha256").filter(|s| is_sha256(s)),
                digest_or_null("policy_sha256"),
            ) {
                (Some(policy), Some(baseline_head), Some(tips), Some(policy_sha256)) => {
                    Ok(Some(Line::Binding(Binding {
                        policy: policy.to_owned(),
                        baseline_head,
                        baseline_tips_sha256: tips.to_owned(),
                        policy_sha256,
                    })))
                }
                _ => Err(not()),
            }
        }
        Some("acceptance")
            if members(&["v", "kind", "policy", "from_sha256", "to_sha256", "commit"]) =>
        {
            match (
                policy,
                digest_or_null("from_sha256"),
                text_of("to_sha256").filter(|s| is_sha256(s)),
                text_of("commit").filter(|s| is_commit_id(s)),
            ) {
                (Some(policy), Some(from), Some(to), Some(commit)) => {
                    Ok(Some(Line::Acceptance(Acceptance {
                        policy: policy.to_owned(),
                        from,
                        to: to.to_owned(),
                        commit: commit.to_owned(),
                        line: at,
                    })))
                }
                _ => Err(not()),
            }
        }
        _ => Err(not()),
    }
}

/// The file's path as a message names it.
pub(crate) fn shown(own: &Path) -> String {
    crate::hook::shown_path(&own.join(FILE).to_string_lossy())
}

/// The policy record in `own`, read whole; `Ok(None)` when there is none. `Err` when it cannot
/// be read whole: anything but a regular file at its name (a link is not followed), larger than
/// [`WHOLE`], unreadable, not UTF-8, or a line that is JSON and not a whole record of this
/// build. The words name the file.
pub(crate) fn read(own: &Path) -> Result<Option<Record>, String> {
    use std::os::unix::fs::OpenOptionsExt as _;
    let shown = shown(own);
    let file = match std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK)
        .open(own.join(FILE))
    {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(format!(
                "the policy record {shown} could not be opened: {e}"
            ));
        }
    };
    match file.metadata() {
        Ok(found) if found.is_file() && found.len() <= WHOLE => {}
        Ok(found) if found.is_file() => {
            return Err(format!(
                "the policy record {shown} is larger than {WHOLE} bytes"
            ));
        }
        _ => return Err(format!("the policy record {shown} is not a regular file")),
    }
    let mut bytes = Vec::new();
    file.take(WHOLE + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("the policy record {shown} could not be read: {e}"))?;
    let text = String::from_utf8(bytes)
        .map_err(|_| format!("the policy record {shown} is not UTF-8 text"))?;
    let mut record = Record {
        sha256: deep_diff_forge_core::sha256_hex(text.as_bytes()),
        ..Record::default()
    };
    for (at, line) in text
        .split('\n')
        .enumerate()
        .map(|(at, line)| (at + 1, line))
        .filter(|(_, line)| !line.is_empty())
    {
        match line_of(line, at).map_err(|why| format!("the policy record {shown}: {why}"))? {
            None => record
                .passed_over
                .push((at, "it is not whole (a write that did not finish)")),
            Some(Line::Binding(binding)) => {
                if record.binding.is_some() {
                    record
                        .passed_over
                        .push((at, "it is a second binding; the first one counts"));
                } else {
                    record.binding = Some((binding, at));
                }
            }
            Some(Line::Acceptance(acceptance)) => {
                if record.binding.is_some() {
                    record.acceptances.push(acceptance);
                } else {
                    record
                        .passed_over
                        .push((at, "it is an acceptance before the binding"));
                }
            }
        }
    }
    Ok(Some(record))
}

/// The project lock ([`lock`]): held while the value lives.
pub(crate) struct ProjectLock {
    _held: nix::fcntl::Flock<std::fs::File>,
}

/// Take the project lock in `own` (the hooks' own directory, which the caller has made),
/// waiting at most `wait` in short sleeps. The lock order is in the head of this module.
#[expect(
    clippy::disallowed_methods,
    reason = "the project lock: a hook or the owner's command waits a bounded time, in short sleeps, for another holder"
)]
pub(crate) fn lock(own: &Path, wait: Duration) -> Result<ProjectLock, String> {
    use nix::fcntl::{Flock, FlockArg};
    use std::os::unix::fs::OpenOptionsExt as _;
    let path = own.join(LOCK);
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK)
        .open(&path)
        .map_err(|e| format!("the project lock could not be opened: {e}"))?;
    if !file.metadata().is_ok_and(|found| found.is_file()) {
        return Err("the project lock is not a regular file".to_owned());
    }
    let mut waited = Duration::ZERO;
    let mut file = file;
    loop {
        match Flock::lock(file, FlockArg::LockExclusiveNonblock) {
            Ok(held) => return Ok(ProjectLock { _held: held }),
            Err((back, nix::errno::Errno::EWOULDBLOCK)) if waited < wait => {
                file = back;
                std::thread::sleep(Duration::from_millis(10));
                waited += Duration::from_millis(10);
            }
            Err((_, nix::errno::Errno::EWOULDBLOCK)) => {
                return Err(
                    "the project lock was held by another stop or command for longer than this one waits"
                        .to_owned(),
                );
            }
            Err((_, errno)) => return Err(format!("the project lock could not be taken: {errno}")),
        }
    }
}

/// The record of `binding` as the file holds it.
pub(crate) fn binding_value(binding: &Binding) -> Value {
    json!({
        "v": VERSION, "kind": "binding", "policy": binding.policy,
        "baseline_head": binding.baseline_head,
        "baseline_tips_sha256": binding.baseline_tips_sha256,
        "policy_sha256": binding.policy_sha256,
    })
}

/// The record of `acceptance` as the file holds it (its `line` is not written).
pub(crate) fn acceptance_value(acceptance: &Acceptance) -> Value {
    json!({
        "v": VERSION, "kind": "acceptance", "policy": acceptance.policy,
        "from_sha256": acceptance.from, "to_sha256": acceptance.to,
        "commit": acceptance.commit,
    })
}

/// Append `record` to the policy record in `own`, in one write, under the project lock the
/// caller holds (`_held`). A file whose last line has no newline (a write that did not finish)
/// gets one first, so the new record is a line of its own and the cut one stays a line that is
/// not whole, which readers pass over.
pub(crate) fn append(own: &Path, _held: &ProjectLock, record: &Value) -> Result<(), String> {
    use std::os::unix::fs::OpenOptionsExt as _;
    let path = own.join(FILE);
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .append(true)
        .create(true)
        .mode(0o600)
        .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK)
        .open(&path)
        .map_err(|e| format!("it could not be opened: {e}"))?;
    let length = match file.metadata() {
        Ok(found) if found.is_file() => found.len(),
        _ => return Err("it is not a regular file".to_owned()),
    };
    let mut last = [0_u8; 1];
    if length > 0 {
        file.seek(std::io::SeekFrom::Start(length - 1))
            .and_then(|_| file.read_exact(&mut last))
            .map_err(|e| format!("it could not be read: {e}"))?;
    }
    let mut out = String::new();
    if length > 0 && last[0] != b'\n' {
        out.push('\n');
    }
    out.push_str(&record.to_string());
    out.push('\n');
    file.write_all(out.as_bytes())
        .map_err(|e| format!("it could not be written: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use deep_diff_forge_testkit::Scratch;

    const ID: &str = "0123456789abcdef0123456789abcdef01234567";

    fn binding() -> Binding {
        Binding {
            policy: "warden.toml".to_owned(),
            baseline_head: Some(ID.to_owned()),
            baseline_tips_sha256: "1".repeat(64),
            policy_sha256: Some("2".repeat(64)),
        }
    }

    fn acceptance(from: Option<&str>, to: &str) -> Acceptance {
        Acceptance {
            policy: "warden.toml".to_owned(),
            from: from.map(str::to_owned),
            to: to.to_owned(),
            commit: ID.to_owned(),
            line: 0,
        }
    }

    /// What this build writes it reads back; the first binding counts; a line that is not
    /// whole (a cut write) is passed over and named, also after the writer's newline repair.
    #[test]
    fn what_is_written_is_read_and_a_cut_line_is_passed_over() {
        let scratch = Scratch::new("policy-record-written");
        let own = scratch.path().to_path_buf();
        assert_eq!(read(&own), Ok(None));
        let held = lock(&own, Duration::from_secs(5)).unwrap();
        append(&own, &held, &binding_value(&binding())).unwrap();
        append(
            &own,
            &held,
            &acceptance_value(&acceptance(Some(&"2".repeat(64)), &"3".repeat(64))),
        )
        .unwrap();
        // a write that did not finish
        let mut text = std::fs::read_to_string(own.join(FILE)).unwrap();
        text.push_str("{\"v\":1,\"kind\":\"acceptance\",\"pol");
        std::fs::write(own.join(FILE), &text).unwrap();
        let cut = read(&own).unwrap().unwrap();
        assert_eq!(
            cut.passed_over,
            vec![(3, "it is not whole (a write that did not finish)")]
        );
        assert_eq!(cut.acceptances.len(), 1);
        // the next writer repairs the newline: its own record is whole, the cut one passed over
        append(&own, &held, &binding_value(&binding())).unwrap();
        append(
            &own,
            &held,
            &acceptance_value(&acceptance(Some(&"3".repeat(64)), &"4".repeat(64))),
        )
        .unwrap();
        let read = read(&own).unwrap().unwrap();
        assert_eq!(read.binding, Some((binding(), 1)));
        assert_eq!(
            read.acceptances
                .iter()
                .map(|a| (a.to.clone(), a.line))
                .collect::<Vec<_>>(),
            vec![("3".repeat(64), 2), ("4".repeat(64), 5)]
        );
        assert_eq!(
            read.passed_over,
            vec![
                (3, "it is not whole (a write that did not finish)"),
                (4, "it is a second binding; the first one counts")
            ]
        );
        assert!(
            std::fs::read_to_string(own.join(FILE))
                .unwrap()
                .ends_with("}\n")
        );
    }

    /// A JSON line that is not a whole record of this build makes the file one that cannot be
    /// read whole, and the words name the file: never "no record".
    #[test]
    fn a_record_that_is_not_whole_refuses_the_file() {
        let scratch = Scratch::new("policy-record-strict");
        let own = scratch.path().to_path_buf();
        let good = binding_value(&binding());
        let mut cases: Vec<String> = Vec::new();
        for (member, wrong) in [
            ("v", json!(2)),
            ("v", json!(0)),
            ("kind", json!("clearance")),
            ("policy", json!("")),
            ("policy", json!("a\nb")),
            ("baseline_head", json!("abc")),
            ("baseline_tips_sha256", json!(null)),
            ("policy_sha256", json!("G".repeat(64))),
        ] {
            let mut record = good.clone();
            record[member] = wrong;
            cases.push(record.to_string());
        }
        let mut extra = good.clone();
        extra["more"] = json!(1);
        cases.push(extra.to_string());
        let mut missing = good.clone();
        missing.as_object_mut().unwrap().remove("policy_sha256");
        cases.push(missing.to_string());
        cases.push(good.to_string().replacen("\"v\":1", "\"v\":1,\"v\":1", 1));
        cases.push("[1,2]".to_owned());
        cases.push("\"text\"".to_owned());
        let mut wrong_accept = acceptance_value(&acceptance(None, &"3".repeat(64)));
        wrong_accept["commit"] = json!("HEAD");
        cases.push(wrong_accept.to_string());
        for case in &cases {
            std::fs::write(own.join(FILE), format!("{good}\n{case}\n")).unwrap();
            let why = read(&own).expect_err(case);
            assert!(
                why.starts_with("the policy record ") && why.contains("line 2"),
                "{case}: {why}"
            );
        }
        // not a regular file, a link, too large
        std::fs::remove_file(own.join(FILE)).unwrap();
        std::fs::create_dir(own.join(FILE)).unwrap();
        assert!(read(&own).unwrap_err().contains("not a regular file"));
        std::fs::remove_dir(own.join(FILE)).unwrap();
        std::os::unix::fs::symlink("/dev/null", own.join(FILE)).unwrap();
        assert!(read(&own).unwrap_err().contains("could not be opened"));
        std::fs::remove_file(own.join(FILE)).unwrap();
        std::fs::write(
            own.join(FILE),
            vec![b'\n'; usize::try_from(WHOLE).unwrap() + 1],
        )
        .unwrap();
        assert!(read(&own).unwrap_err().contains("is larger than"));
    }

    /// The project lock is one: a second taker waits its bound and is told so.
    #[test]
    fn the_project_lock_is_held_by_one_at_a_time() {
        let scratch = Scratch::new("policy-record-lock");
        let own = scratch.path().to_path_buf();
        let first = lock(&own, Duration::from_secs(1)).unwrap();
        let second = lock(&own, Duration::from_millis(50));
        assert!(second.is_err_and(|why| why.contains("held by another")));
        drop(first);
        assert!(lock(&own, Duration::from_millis(50)).is_ok());
    }
}
