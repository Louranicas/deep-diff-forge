// Review council: Explicit preparation/status/import and opt-in execution commands; manual reports cannot mint provider evidence.
// Design/status: docs/REVIEW_COUNCIL.md#council-cli
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fcouncil--crates--deep-diff-forge-review--src--cli
//! Small explicit CLI: preparation/status are local; execution requires --execute.
use crate::{
    Gate, Outcome, Persona, Profile, Provider, Report, Request, Result, ReviewFailure, Reviewer,
    Round, assess, load_manifest, prepare, require, run_gate, run_reviewer, source_fingerprint,
    storage, validate_report,
};
use deep_diff_forge_core::exit;
use std::path::Path;

/// Execute council CLI arguments, returning a machine-readable result and appropriate exit code.
/// # Errors
/// Refuses unknown flags, invalid JSON, stale artifacts and execution without --execute.
pub fn dispatch(args: &[String]) -> Result<(String, i32)> {
    if args
        .iter()
        .any(|arg| matches!(arg.as_str(), "--help" | "-h"))
    {
        return Ok((HELP.into(), exit::COUNCIL_COMPLETED));
    }
    let command = args.first().map_or("help", String::as_str);
    match command {
        // A template for `prepare` to read, not a result: `Request` refuses unknown fields.
        "example" if args.len() == 1 => output(&example(), true),
        "prepare" if args.len() == 3 => {
            let request: Request = storage::read_json(Path::new(&args[1]), 128 * 1024)?;
            // The manifest names its own schema (`deep-diff-forge.council.v1`).
            output(&prepare(request, Path::new(&args[2]))?, true)
        }
        "status" if args.len() == 2 => status(Path::new(&args[1])),
        "review" if args.len() == 5 && args[4] == "--execute" => {
            let round = match args[3].as_str() {
                "independent" => Round::Independent,
                "challenge" => Round::Challenge,
                _ => {
                    return Err(ReviewFailure(
                        "round must be independent or challenge".into(),
                    ));
                }
            };
            let result = run_reviewer(Path::new(&args[1]), &args[2], round)?;
            stamped("review", &result, result.outcome == Outcome::Passed)
        }
        "gate" if args.len() == 4 && args[3] == "--execute" => {
            let gate: Gate = serde_json::from_str(&format!("{:?}", args[2]))?;
            let results = run_gate(Path::new(&args[1]), gate)?;
            stamped(
                "gate",
                &results,
                results.iter().all(|r| r.outcome == Outcome::Passed),
            )
        }
        "import" if args.len() == 3 => import(Path::new(&args[1]), Path::new(&args[2])),
        "export-tuicr" => export_tuicr(&args[1..]),
        "import-tuicr" => import_tuicr(&args[1..]),
        "help" | "--help" | "-h" if args.len() <= 1 => Ok((HELP.into(), exit::COUNCIL_COMPLETED)),
        _ => Err(ReviewFailure(
            "invalid council command or missing explicit --execute; use council help".into(),
        )),
    }
}
const HELP: &str = "council example | prepare <request.json> <new-run-dir> | status <run-dir> | review <run-dir> <reviewer-id> <independent|challenge> --execute | gate <run-dir> <format|check|clippy|pedantic|tests|antipatterns|fuzz|mutation> --execute | import <run-dir> <report.json> | export-tuicr <run-dir> --session <tuicr-session.json> [--allow-review-level] [--execute] | import-tuicr <run-dir> --session <tuicr-session.json> [--execute]\n";
/// The schema of a council result that had none: `deep-diff-forge.council-<command>.v1`, for
/// `review`, `gate`, `status` and `import`. Before, those results did not say which shape they
/// were (low-severity finding of the 2026-11-04 assessment), so a consumer could not tell a
/// changed shape from a broken one. `prepare`, `export-tuicr` and `import-tuicr` already named
/// theirs.
#[must_use]
pub fn result_schema(command: &str) -> String {
    format!("deep-diff-forge.council-{command}.v1")
}
/// Name the schema in a result: in the object, or in each object of an array (`gate` prints
/// one result per gate, and stays an array so readers of it keep working).
fn stamp(command: &str, value: &impl serde::Serialize) -> Result<serde_json::Value> {
    let schema = serde_json::Value::String(result_schema(command));
    let mut value = serde_json::to_value(value)?;
    let objects: Vec<&mut serde_json::Map<String, serde_json::Value>> = match &mut value {
        serde_json::Value::Object(object) => vec![object],
        serde_json::Value::Array(items) => items
            .iter_mut()
            .map(|item| item.as_object_mut())
            .collect::<Option<_>>()
            .ok_or_else(|| {
                ReviewFailure(format!("council {command}: a result is not an object"))
            })?,
        _ => {
            return Err(ReviewFailure(format!(
                "council {command}: the result is not an object"
            )));
        }
    };
    for object in objects {
        require(
            object.insert("schema".into(), schema.clone()).is_none(),
            "a council result already has a schema field",
        )?;
    }
    Ok(value)
}
fn stamped(command: &str, value: &impl serde::Serialize, passed: bool) -> Result<(String, i32)> {
    output(&stamp(command, value)?, passed)
}
fn output(value: &impl serde::Serialize, passed: bool) -> Result<(String, i32)> {
    Ok((
        render(value)?,
        if passed {
            exit::COUNCIL_COMPLETED
        } else {
            exit::COUNCIL_HOLD
        },
    ))
}
fn render(value: &impl serde::Serialize) -> Result<String> {
    Ok(
        deep_diff_forge_core::json_line_safe(&serde_json::to_string_pretty(value)?).into_owned()
            + "\n",
    )
}
fn status(directory: &Path) -> Result<(String, i32)> {
    let manifest = load_manifest(directory)?;
    let reports = storage::reports(directory, &manifest)?;
    let executions = storage::executions(directory, &manifest)?;
    let tree = source_fingerprint(Path::new(&manifest.request.repository))?;
    let mut result = assess(&manifest, &reports, &executions, &tree)?;
    if directory.join("controller.lock").exists() {
        result.state = crate::AssessmentState::Hold;
        result
            .reasons
            .push("controller execution is active or interrupted".into());
    }
    stamped(
        "status",
        &result,
        result.state == crate::AssessmentState::ReadyForHumanReview,
    )
}
fn import(directory: &Path, path: &Path) -> Result<(String, i32)> {
    let manifest = load_manifest(directory)?;
    let report: Report = storage::read_json(path, storage::REPORT_CAP)?;
    validate_report(&manifest, &report)?;
    let qualification = storage::job_dir(
        directory,
        &format!("review-{}-{}", report.reviewer_id, report.round.label()),
    )?
    .join("qualification.json");
    require(
        !qualification.exists(),
        "reviewer slot already has a controller qualification; manual import refused",
    )?;
    require(
        source_fingerprint(Path::new(&manifest.request.repository))? == manifest.source_tree,
        "source changed; import would be stale",
    )?;
    storage::write_json(
        &storage::report_path(directory, &report.reviewer_id, report.round)?,
        &report,
    )?;
    stamped(
        "import",
        &serde_json::json!({
            "imported": true,
            "provenance": "manual_unverified",
            "satisfies_execution_gate": false,
        }),
        true,
    )
}
/// Parse `export-tuicr <run-dir> --session <path> [--allow-review-level] [--execute]`; dry run by default.
fn export_tuicr(args: &[String]) -> Result<(String, i32)> {
    let usage = || {
        ReviewFailure(
            "usage: council export-tuicr <run-dir> --session <tuicr-session.json> [--allow-review-level] [--execute]"
                .into(),
        )
    };
    let (run, rest) = args.split_first().ok_or_else(usage)?;
    let mut session = None;
    let mut options = crate::ExportOptions::default();
    let mut rest = rest.iter();
    while let Some(flag) = rest.next() {
        match flag.as_str() {
            "--session" if session.is_none() => session = Some(rest.next().ok_or_else(usage)?),
            "--execute" if !options.execute => options.execute = true,
            "--allow-review-level" if !options.allow_review_level => {
                options.allow_review_level = true;
            }
            _ => return Err(usage()),
        }
    }
    let session = session.ok_or_else(usage)?;
    // Names its own schema (`deep-diff-forge.tuicr-export.v1`).
    output(
        &crate::export_tuicr(Path::new(run), Path::new(session), options)?,
        true,
    )
}
/// Parse `import-tuicr <run-dir> --session <path> [--execute]`; dry run by default. Exit 7 when the
/// planned or appended events include a conflict.
fn import_tuicr(args: &[String]) -> Result<(String, i32)> {
    let usage = || {
        ReviewFailure(
            "usage: council import-tuicr <run-dir> --session <tuicr-session.json> [--execute]"
                .into(),
        )
    };
    let (run, rest) = args.split_first().ok_or_else(usage)?;
    let mut session = None;
    let mut options = crate::ImportOptions::default();
    let mut rest = rest.iter();
    while let Some(flag) = rest.next() {
        match flag.as_str() {
            "--session" if session.is_none() => session = Some(rest.next().ok_or_else(usage)?),
            "--execute" if !options.execute => options.execute = true,
            _ => return Err(usage()),
        }
    }
    let session = session.ok_or_else(usage)?;
    let result = crate::import_tuicr(Path::new(run), Path::new(session), options)?;
    // Names its own schema (`deep-diff-forge.tuicr-import.v1`).
    output(&result, result.conflicts == 0)
}
/// Portable request template; caller chooses concrete paths and available model IDs.
#[must_use]
pub fn example() -> Request {
    Request {
        repository: "/path/to/repository".into(),
        analysis_file: "/path/to/analysis.json".into(),
        decision_file: "/path/to/decision.md".into(),
        context_files: vec![],
        builder_family: crate::BuilderFamily::Openai,
        profile: Profile::ImportantDecision,
        reviewers: vec![
            Reviewer {
                id: "correctness-codex".into(),
                provider: Provider::Codex,
                model: None,
                persona: Persona::Correctness,
            },
            Reviewer {
                id: "security-claude".into(),
                provider: Provider::ClaudeCode,
                model: None,
                persona: Persona::Security,
            },
            Reviewer {
                id: "architecture-claude".into(),
                provider: Provider::ClaudeCode,
                model: None,
                persona: Persona::Architecture,
            },
            Reviewer {
                id: "falsifier-codex".into(),
                provider: Provider::Codex,
                model: None,
                persona: Persona::Falsifier,
            },
            Reviewer {
                id: "performance-claude".into(),
                provider: Provider::ClaudeCode,
                model: None,
                persona: Persona::Performance,
            },
            Reviewer {
                id: "subtractor-codex".into(),
                provider: Provider::Codex,
                model: None,
                persona: Persona::Subtractor,
            },
        ],
        mutation_package: "deep-diff-forge-review".into(),
        mutation_base: None,
        fuzz_targets: vec!["council_report".into()],
        timeout_seconds: 600,
        fuzz_seconds: 30,
    }
}
