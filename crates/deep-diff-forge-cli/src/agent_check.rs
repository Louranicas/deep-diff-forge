//! `agent check` (F3, part 1): the judgement a stop gives the uncommitted change, on demand.
//! With `--session ID` it reads that session's state, read only, for the base the session
//! recorded; the judge, the policy and the ranker directory come from the registered `Stop`
//! hook, never from `PATH`. It judges the uncommitted change alone: what is staged and the
//! session's commits are not judged in this build, so it never says it is the stop's answer.

use crate::emit;
use crate::hook::{self, Judged};
use deep_diff_forge_core::{display_safe, exit};
use serde_json::{Value, json};
use std::path::Path;

pub(crate) const CHECK_USAGE: &str = "deep-diff-forge agent check [--session ID | --base REV] [--judge PATH] [--policy NAME] [--ranker-dir DIR] [--json]";

/// What a check does not judge, said with every answer: true of this build whatever it finds.
const NOT_THE_STOP: &str = "This is not the stop's answer: a stop also judges what is staged and each commit of the session, and blocks only for a failure that has not blocked before; this build of agent check judges the uncommitted change alone.";

pub(crate) fn run(args: &[String]) {
    crate::args::admit_or_exit(
        args,
        &crate::args::Spec {
            flags: &["--json", "--require-pass"],
            valued: &[
                "--session",
                "--base",
                "--judge",
                "--policy",
                "--ranker-dir",
                "--range",
            ],
            positionals: 0,
        },
        CHECK_USAGE,
    );
    if crate::flag_value(args, "--range").is_some() || args.iter().any(|a| a == "--require-pass") {
        refuse("--range and --require-pass are not in this build of agent check");
    }
    let (session, base) = (
        crate::flag_value(args, "--session"),
        crate::flag_value(args, "--base"),
    );
    if session.is_some() && base.is_some() {
        refuse("--session and --base exclude each other: a session names its own base");
    }
    let document = check(session.as_deref(), base.as_deref(), args);
    if args.iter().any(|a| a == "--json") {
        crate::emit_pretty_json(&document);
    } else {
        emitln!("{}", text(&document));
    }
}

fn refuse(why: &str) -> ! {
    eprintln!("error: {why}");
    eprintln!("usage: {CHECK_USAGE}");
    std::process::exit(exit::USAGE);
}

/// The document `deep-diff-forge.agent-check.v0`: what was judged, with what, and what that
/// says of a stop. A check that cannot judge says why in `unjudged` and has no unit.
fn check(session: Option<&str>, base: Option<&str>, args: &[String]) -> Value {
    let mut doc = json!({
        "schema": "deep-diff-forge.agent-check.v0",
        "tool": {"name": "deep-diff-forge", "version": env!("CARGO_PKG_VERSION")},
        "project": null, "session": null, "base": null, "judge": null, "units": [],
        "unjudged": null,
        "stop": {"same_inputs": false, "differences": [], "whole_answer": false, "said": NOT_THE_STOP},
    });
    let mut differs: Vec<String> = Vec::new();
    let found = if let Some(id) = session {
        from_session(id, &mut doc)
    } else {
        differs.push(
            "no session: the policy is read at the base named here, not at the base a session recorded"
                .to_owned(),
        );
        std::env::current_dir()
            .map_err(|e| format!("the current directory cannot be read: {e}"))
            .and_then(|dir| crate::hook_run::project_of(&dir))
            .map(|(top, _)| match base {
                Some(base) => (top, base.to_owned(), "--base"),
                None => (top, "HEAD".to_owned(), "HEAD"),
            })
    };
    let (top, base, from) = match found {
        Ok(found) => found,
        Err(why) => return unjudged(doc, &differs, &why),
    };
    doc["project"] = json!(top.to_string_lossy());
    let (bench, no_judge) = installed(&top, args, &mut differs, &mut doc);
    match crate::hook_run::check_uncommitted(&top, &base, &bench, no_judge.as_deref()) {
        Ok((commit, judged)) => {
            doc["base"] = json!({"commit": commit, "from": from});
            if let Some(judged) = judged {
                doc["units"] = json!([unit(&judged)]);
            }
            doc["stop"]["same_inputs"] = json!(differs.is_empty());
            doc["stop"]["differences"] = json!(differs);
            doc
        }
        Err(why) => unjudged(doc, &differs, &why),
    }
}

/// The session's project, the base it recorded and where that base is from, read from the
/// session's state without writing it ([`crate::hook_run::session_seen`]).
fn from_session(
    id: &str,
    doc: &mut Value,
) -> Result<(std::path::PathBuf, String, &'static str), String> {
    let seen = crate::hook_run::session_seen(id)
        .map_err(|why| format!("the session {}: {why}", display_safe(id)))?;
    // (a base its first hook recorded is the stop's base all the same)
    let recorded_by = if seen.by_start {
        "session_start"
    } else {
        "first_hook"
    };
    doc["session"] =
        json!({"state_file": seen.file.to_string_lossy(), "base_recorded_by": recorded_by});
    let base = seen.base.map_err(str::to_owned)?;
    let (top, id) = crate::hook_run::project_of(&seen.top)?;
    if top != seen.top || id != seen.id {
        return Err(format!(
            "the repository at {} is not the one the session began in, so a stop of this session is UNJUDGED",
            hook::shown_path(&seen.top.to_string_lossy())
        ));
    }
    Ok((top, base, "session"))
}

fn unjudged(mut doc: Value, differs: &[String], why: &str) -> Value {
    doc["unjudged"] = json!(why);
    doc["stop"]["differences"] = json!(differs);
    doc
}

/// The judge, the policy and the ranker directory the check runs with, as options for the
/// bench: those of the registered `Stop` hook, each replaced by one given here. With them, why
/// there is no judge to run (never one from `PATH`); every way they are not the stop's goes
/// into `differs`.
fn installed(
    top: &Path,
    args: &[String],
    differs: &mut Vec<String>,
    doc: &mut Value,
) -> (Vec<String>, Option<String>) {
    let found = crate::hook_run::installed_stop(top);
    let stop = found
        .as_ref()
        .map(|(_, read)| read.clone())
        .unwrap_or_default();
    if found.is_none() {
        differs.push("no settings file Claude Code reads for this project registers a Stop hook of deep-diff-forge".to_owned());
    } else {
        let this = std::env::current_exe().and_then(std::fs::canonicalize).ok();
        if this.as_deref().and_then(Path::to_str) != Some(stop.program.as_str()) {
            differs.push(format!("the registered Stop hook runs {}, not this binary: a stop offers that binary as its first ranker and reads the verdict by its rules", hook::shown_path(&stop.program)));
        }
    }
    let mut bench = Vec::new();
    let mut take = |flag: &str, hooked: Option<String>, default: Option<&str>| {
        let given = crate::flag_value(args, flag);
        if found.is_some()
            && given.is_some()
            && given.as_deref().or(default) != hooked.as_deref().or(default)
        {
            differs.push(format!("{flag} is not the registered Stop hook's"));
        }
        let value = given.or(hooked);
        if let Some(value) = &value {
            bench.extend([flag.to_owned(), value.clone()]);
        }
        value
    };
    let judge = take("--judge", stop.judge.clone(), None);
    let policy = take("--policy", stop.policy.clone(), Some("warden.toml"));
    let ranker_dir = take("--ranker-dir", stop.ranker_dir.clone(), None);
    doc["judge"] = json!({
        "program": judge,
        "policy": policy.as_deref().unwrap_or("warden.toml"),
        "ranker_dir": ranker_dir,
        "stop_hook": found.as_ref().map(|(file, _)| file.to_string_lossy()),
    });
    let no_judge = match (&judge, &found) {
        (Some(_), _) => None,
        (None, Some(_)) => {
            let why = "the registered Stop hook names no --judge, so a stop takes forge-warden from PATH, which agent check never does: name the judge with --judge PATH";
            differs.push(why.to_owned());
            Some(why.to_owned())
        }
        (None, None) => {
            Some("no judge: no registered Stop hook names one and --judge was not given".to_owned())
        }
    };
    (bench, no_judge)
}

/// One unit as the document gives it: its outcome, and for a hold or a fail each rule by its
/// id with the signals and paths the judge cites (never the judge's words: they can quote the
/// patch).
fn unit(judged: &Judged) -> Value {
    let rules = |rules: &[hook::RuleOutcome]| -> Value {
        rules
            .iter()
            .map(|rule| {
                json!({"id": rule.id, "held": rule.held, "violations": rule.violations.iter()
                    .map(|(signal, path, items)| json!({"signal": signal, "path": path, "items": items}))
                    .collect::<Vec<_>>()})
            })
            .collect()
    };
    let (outcome, rules, said, reason) = match judged {
        Judged::Pass => ("pass", json!([]), &[][..], None),
        Judged::Hold { rules: r, said } => ("hold", rules(r), &said[..], None),
        Judged::Fail { rules: r, said } => ("fail", rules(r), &said[..], None),
        Judged::Unjudged { reason } => ("unjudged", json!([]), &[][..], Some(reason.as_str())),
    };
    json!({"unit": "uncommitted", "outcome": outcome, "rules": rules, "judge_notes": said.len(),
           "judge_note_codes": said.iter().map(|words| note_code(words)).collect::<Vec<_>>(), "reason": reason})
}

/// What kind of thing the judge named beside its rules, as a code and never in its words: the
/// code of a note (`note <code> …`) when it is a plain identifier, `expired_waiver`, else `note`.
fn note_code(words: &str) -> &str {
    if words.starts_with("expired waiver ") {
        return "expired_waiver";
    }
    words
        .strip_prefix("note ")
        .and_then(|rest| rest.split([' ', ':']).next())
        .filter(|code| {
            !code.is_empty()
                && code.len() <= 64
                && code
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        })
        .unwrap_or("note")
}

/// The text form, for an agent: each path quoted and escaped, no words of the judge.
fn text(doc: &Value) -> String {
    let word = |value: &Value| display_safe(value.as_str().unwrap_or("none")).into_owned();
    let path = |value: &Value| {
        value
            .as_str()
            .map_or_else(|| "none".to_owned(), hook::shown_path)
    };
    let mut out = Vec::new();
    match doc["units"].get(0) {
        _ if !doc["unjudged"].is_null() => {
            out.push(format!("UNJUDGED: {}", word(&doc["unjudged"])));
        }
        None => out
            .push("Nothing uncommitted: the working tree equals the commit HEAD names.".to_owned()),
        Some(unit) => {
            let outcome = unit["outcome"].as_str().unwrap_or_default().to_uppercase();
            match unit["reason"].as_str() {
                Some(reason) => out.push(format!(
                    "The uncommitted change: {outcome}: {}",
                    display_safe(reason)
                )),
                None => out.push(format!("The uncommitted change: {outcome}")),
            }
            for rule in unit["rules"].as_array().into_iter().flatten() {
                for cited in rule["violations"].as_array().into_iter().flatten() {
                    out.push(format!(
                        "  {} {}: {} in {} ({} items)",
                        if rule["held"] == true {
                            "held"
                        } else {
                            "fails"
                        },
                        word(&rule["id"]),
                        word(&cited["signal"]),
                        path(&cited["path"]),
                        cited["items"]
                    ));
                }
            }
            let notes = unit["judge_notes"].as_u64().unwrap_or(0);
            let codes: Vec<String> = unit["judge_note_codes"]
                .as_array()
                .into_iter()
                .flatten()
                .map(word)
                .collect();
            if notes > 0 {
                out.push(format!(
                    "  the judge also named {notes} thing(s) that are not rules ({}); its words are not repeated: judge_note_codes in the --json form",
                    codes.join(", ")
                ));
            } else if matches!(unit["outcome"].as_str(), Some("hold" | "fail"))
                && unit["rules"].as_array().is_none_or(Vec::is_empty)
            {
                out.push("  the judge named no rule and nothing beside its rules".to_owned());
            }
        }
    }
    out.push(format!("project: {}", path(&doc["project"])));
    out.push(format!(
        "base: {} ({})",
        word(&doc["base"]["commit"]),
        word(&doc["base"]["from"])
    ));
    out.push(format!(
        "judge: {}; policy: {}; ranker directory: {}; Stop hook: {}",
        path(&doc["judge"]["program"]),
        path(&doc["judge"]["policy"]),
        path(&doc["judge"]["ranker_dir"]),
        path(&doc["judge"]["stop_hook"])
    ));
    if doc["stop"]["same_inputs"] == true && doc["units"].get(0).is_some() {
        out.push("Judged with what a stop of this session judges the uncommitted change with: the policy at the base the session recorded, and the judge, the policy and the ranker directory of the registered Stop hook.".to_owned());
    }
    for why in doc["stop"]["differences"].as_array().into_iter().flatten() {
        out.push(format!("Not as the stop: {}", word(why)));
    }
    out.push(NOT_THE_STOP.to_owned());
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A HOLD or a FAIL never stands without its reason: a hold for a note alone says how many
    /// things the judge named beside its rules, by their codes, and never in the judge's words.
    #[test]
    fn a_hold_for_notes_alone_says_so_by_their_codes() {
        let said = vec![
            "note claims_unverified: the policy requires verified claims".to_owned(),
            "expired waiver 0 for rule r: owner o, expired 2026-01-01".to_owned(),
            "note Bad Code: words".to_owned(),
        ];
        let mut doc = json!({"units": [unit(&Judged::Hold { rules: vec![], said })],
                             "unjudged": null, "stop": {"differences": []}});
        let said = text(&doc);
        assert!(
            said.contains("the judge also named 3 thing(s) that are not rules (claims_unverified, expired_waiver, note)"),
            "{said}"
        );
        assert!(!said.contains("requires verified"), "{said}");
        doc["units"] = json!([unit(&Judged::Fail {
            rules: vec![],
            said: vec![]
        })]);
        assert!(
            text(&doc).contains("the judge named no rule and nothing beside its rules"),
            "{doc}"
        );
    }
}
