//! `agent check` (F3): the judgement a stop gives, on demand, unit by unit: the uncommitted
//! change, the staged change when it is another, and each commit by its own patch (a session's
//! commits, or those of `--range`). With `--session ID` it reads that session's state, read
//! only, for the base the session recorded and the commits that are the session's; the judge,
//! the policy and the ranker directory come from the registered `Stop` hook, never from `PATH`.
//! It says it equals the stop in its units and inputs only when it judged every unit the stop
//! answers for with the stop's inputs, and it lists what the stop can still decide otherwise.

use crate::emit;
use crate::hook::{self, Judged};
use deep_diff_forge_core::{display_safe, exit};
use serde_json::{Value, json};
use std::path::Path;

pub(crate) const CHECK_USAGE: &str = "deep-diff-forge agent check [--session ID | --base REV] [--range A..B] [--judge PATH] [--policy NAME] [--ranker-dir DIR] [--require-pass] [--json]";

/// Said when not every unit a stop answers for was judged with the stop's inputs.
const NOT_WHOLE: &str = "This is not the stop's whole answer: not every unit a stop of this session answers for was judged here with the stop's inputs (the lines \"Not as the stop\" say what differs).";
/// Said only when every unit a stop of the session answers for was judged with its inputs.
const EQUAL: &str = "Equal to the stop in its units and inputs: every unit a stop of this session answers for (the uncommitted change, the staged change where it is another, and each commit of the session by its own patch) was judged here with the stop's patch, the policy at the base the session recorded, and the judge and rankers of the registered Stop hook. What the stop then decides can still differ: the lines \"The stop may differ\" say where.";

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
    let (session, base) = (
        crate::flag_value(args, "--session"),
        crate::flag_value(args, "--base"),
    );
    if session.is_some() && base.is_some() {
        refuse("--session and --base exclude each other: a session names its own base");
    }
    let document = check(session.as_deref(), base.as_deref(), args);
    // --require-pass: decided before the document is written, so a closed or full stdout keeps it
    if args.iter().any(|a| a == "--require-pass") {
        let units = document["units"].as_array().map_or(&[][..], Vec::as_slice);
        let failed = units
            .iter()
            .filter(|unit| unit["outcome"] != "pass")
            .count();
        if units.is_empty() {
            let class = exit::refusal("require_pass_no_unit");
            crate::refusal::pending(&[(class, "there is no unit to judge".to_owned())]);
        } else if failed > 0 {
            let class = exit::refusal("require_pass_not_pass");
            let text = format!("{failed} of {} units did not pass", units.len());
            crate::refusal::pending(&[(class, text)]);
        }
    }
    if args.iter().any(|a| a == "--json") {
        crate::emit_pretty_json(&document);
    } else {
        emitln!("{}", text(&document));
    }
    crate::refusal::finish();
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
        "unjudged": null, "ledger": null,
        "stop": {"same_inputs": false, "differences": [], "whole_answer": false, "said": NOT_WHOLE,
                 "may_differ": []},
    });
    let mut differs: Vec<String> = Vec::new();
    let mut lists = None;
    let found = if let Some(id) = session {
        from_session(id, &mut doc).map(|(top, base, seen)| {
            lists = Some(seen);
            (top, base, "session")
        })
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
    let same_inputs = differs.is_empty();
    let tips = lists
        .as_ref()
        .and_then(|(tips, _)| tips.as_ref().ok())
        .map(Vec::as_slice);
    let range = crate::flag_value(args, "--range");
    let checked = match crate::hook_run::check_units(
        &top,
        &base,
        &bench,
        no_judge.as_deref(),
        range.as_deref(),
        (tips, session),
    ) {
        Ok(checked) => checked,
        Err(why) => return unjudged(doc, &differs, &why),
    };
    doc["base"] = json!({"commit": checked.base, "from": from});
    doc["ledger"] = match &checked.recorded {
        None => Value::Null,
        Some(Ok(())) => json!({"written": true, "why": null}),
        Some(Err(why)) => json!({"written": false, "why": why}),
    };
    doc["units"] = checked
        .units
        .iter()
        .map(|(kind, id, judged, other)| {
            unit_with_advice(kind, id.as_deref(), judged, other.as_ref())
        })
        .collect();
    // The units a stop of the session answers for that were not judged here.
    if let Some((Err(why), _)) = &lists {
        differs.push(format!(
            "a stop of this session tells none of its commits from the others, so it judges none ({})",
            display_safe(why)
        ));
    }
    if let Some(all) = &checked.session_commits {
        let judged = |id: &String| {
            checked
                .units
                .iter()
                .any(|(_, at, _, _)| at.as_ref() == Some(id))
        };
        let left = all.iter().filter(|id| !judged(id)).count();
        if left > 0 {
            differs.push(format!(
                "{left} of the session's {} commits were not judged here (the range leaves them out)",
                all.len()
            ));
        }
    }
    if checked.linked {
        differs.push("the project has linked working trees: a stop from inside one also judges that tree's changes, which this check does not".to_owned());
    }
    let whole = same_inputs && differs.is_empty() && session.is_some();
    let answered = lists.as_ref().map(|(_, answered)| answered.as_slice());
    doc["stop"]["same_inputs"] = json!(same_inputs);
    doc["stop"]["whole_answer"] = json!(whole);
    doc["stop"]["said"] = json!(if whole { EQUAL } else { NOT_WHOLE });
    doc["stop"]["may_differ"] = json!(may_differ(&checked, answered));
    doc["stop"]["differences"] = json!(differs);
    doc
}

/// Where a stop can decide otherwise than these verdicts read alone: only what applies.
fn may_differ(checked: &crate::hook_run::Checked, answered: Option<&[String]>) -> Vec<String> {
    let mut said = Vec::new();
    if checked.units.is_empty() {
        return said;
    }
    if checked
        .units
        .iter()
        .any(|(_, _, judged, _)| matches!(judged, Judged::Fail { .. }))
    {
        said.push("a failure blocks a stop once: what has blocked before stands at the stop and does not block again (the session's memory of it is not read here)".to_owned());
    }
    let recorded = |id: &String| answered.is_some_and(|list| list.contains(id));
    let known = checked
        .units
        .iter()
        .filter(|(_, id, _, _)| id.as_ref().is_some_and(recorded))
        .count();
    if known > 0 {
        said.push(format!("{known} of these commits have an answer an earlier stop recorded: the stop reports that answer and does not ask the judge again"));
    }
    said.push("a stop has 45 seconds for all it does and 30 for one run of the judge, keeps time back for what it asks after, and judges in its own order: what it does not reach is UNJUDGED at that stop".to_owned());
    said
}

/// The session's project, the base it recorded and where that base is from, read from the
/// session's state without writing it ([`crate::hook_run::session_seen`]).
#[expect(
    clippy::type_complexity,
    reason = "the project, the base, and the session's two lists"
)]
fn from_session(
    id: &str,
    doc: &mut Value,
) -> Result<
    (
        std::path::PathBuf,
        String,
        (Result<Vec<String>, String>, Vec<String>),
    ),
    String,
> {
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
    Ok((top, base, (seen.tips, seen.answered)))
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
        // (a ranker directory is the stop's when it is the directory the stop lists, as
        // `agent doctor --gate` compares it: a relative path, a trailing `/` or a link to it)
        let other = if flag == "--ranker-dir" {
            !crate::hook_run::same_ranker_dir(hooked.as_deref(), given.as_deref())
        } else {
            given.as_deref().or(default) != hooked.as_deref().or(default)
        };
        if found.is_some() && given.is_some() && other {
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
fn unit(kind: &str, commit: Option<&str>, judged: &Judged) -> Value {
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
        Judged::Unjudged { reason, .. } => ("unjudged", json!([]), &[][..], Some(reason.as_str())),
        // (what a stop holds once for, F12: the stop's own sentence, no words of Git or the judge)
        Judged::Refused {
            class, sentence, ..
        } if class.is_work() => ("unjudged", json!([]), &[][..], Some(sentence.as_str())),
        Judged::Refused { sentence, .. } => ("hold", json!([]), &[][..], Some(sentence.as_str())),
    };
    json!({"unit": kind, "commit": commit, "ranker": null, "advice_only": null, "advice_unread": null, "outcome": outcome, "rules": rules, "judge_notes": said.len(),
           "judge_note_codes": said.iter().map(|words| note_code(words)).collect::<Vec<_>>(), "reason": reason})
}

/// [`unit()`], with the ranker the policy admits when it is not this binary, and what it leaves
/// to advice (F3.f).
fn unit_with_advice(
    kind: &str,
    id: Option<&str>,
    judged: &Judged,
    other: Option<&crate::hook_run::Admitted>,
) -> Value {
    let mut unit = unit(kind, id, judged);
    if let Some((ranker, advice)) = other {
        unit["ranker"] = json!(ranker.to_string_lossy());
        match advice {
            Ok(labels) => {
                unit["advice_only"] = labels
                    .iter()
                    .map(|(path, label)| json!({"path": path, "label": label}))
                    .collect();
            }
            Err(why) => unit["advice_unread"] = json!(why),
        }
    }
    unit
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
    if !doc["unjudged"].is_null() {
        out.push(format!("UNJUDGED: {}", word(&doc["unjudged"])));
    } else if doc["units"].as_array().is_none_or(Vec::is_empty) {
        out.push("Nothing to judge: the working tree and the index equal the commit HEAD names, and no commit was judged.".to_owned());
    }
    for unit in doc["units"].as_array().into_iter().flatten() {
        unit_text(unit, &mut out);
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
    if doc["ledger"]["written"] == false {
        out.push(format!(
            "This check is not recorded in the project's ledger: {}",
            word(&doc["ledger"]["why"])
        ));
    }
    let judged = doc["units"].get(0).is_some();
    if doc["stop"]["same_inputs"] == true && doc["stop"]["whole_answer"] != true && judged {
        out.push("Judged with what a stop of this session judges these units with: the policy at the base the session recorded, and the judge, the policy and the ranker directory of the registered Stop hook.".to_owned());
    }
    for why in doc["stop"]["differences"].as_array().into_iter().flatten() {
        out.push(format!("Not as the stop: {}", word(why)));
    }
    for why in doc["stop"]["may_differ"].as_array().into_iter().flatten() {
        out.push(format!("The stop may differ: {}", word(why)));
    }
    out.push(word(&doc["stop"]["said"]));
    out.join("\n")
}

/// The lines of one unit in the text form.
fn unit_text(unit: &Value, out: &mut Vec<String>) {
    let word = |value: &Value| display_safe(value.as_str().unwrap_or("none")).into_owned();
    let path = |value: &Value| {
        value
            .as_str()
            .map_or_else(|| "none".to_owned(), hook::shown_path)
    };
    let name = match (unit["unit"].as_str(), unit["commit"].as_str()) {
        (Some("staged"), _) => "The staged change".to_owned(),
        (Some("commit"), Some(id)) => format!(
            "Commit {}",
            display_safe(&id.chars().take(12).collect::<String>())
        ),
        _ => "The uncommitted change".to_owned(),
    };
    {
        let outcome = unit["outcome"].as_str().unwrap_or_default().to_uppercase();
        match unit["reason"].as_str() {
            Some(reason) => out.push(format!("{name}: {outcome}: {}", display_safe(reason))),
            None => out.push(format!("{name}: {outcome}")),
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
        if let Some(ranker) = unit["ranker"].as_str() {
            out.push(format!(
                "  judged with the ranker the policy admits, {}, not this binary",
                hook::shown_path(ranker)
            ));
        }
        for advice in unit["advice_only"].as_array().into_iter().flatten() {
            out.push(format!(
                "  advice only, not judged by the pinned ranker: {} in {}",
                word(&advice["label"]),
                path(&advice["path"])
            ));
        }
        if let Some(why) = unit["advice_unread"].as_str() {
            out.push(format!(
                "  what is advice only is not known: {}",
                display_safe(why)
            ));
        }
    }
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
        let mut doc = json!({"units": [unit("uncommitted", None, &Judged::Hold { rules: vec![], said })],
                             "unjudged": null, "stop": {"differences": []}});
        let said = text(&doc);
        assert!(
            said.contains("the judge also named 3 thing(s) that are not rules (claims_unverified, expired_waiver, note)"),
            "{said}"
        );
        assert!(!said.contains("requires verified"), "{said}");
        doc["units"] = json!([unit(
            "uncommitted",
            None,
            &Judged::Fail {
                rules: vec![],
                said: vec![]
            }
        )]);
        assert!(
            text(&doc).contains("the judge named no rule and nothing beside its rules"),
            "{doc}"
        );
    }
}
