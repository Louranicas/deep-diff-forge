// Review council: Explicit preparation/status/import and opt-in execution commands; manual reports cannot mint provider evidence.
// Design/status: docs/REVIEW_COUNCIL.md#council-cli
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fcouncil--crates--deep-diff-forge-review--src--cli
//! Small explicit CLI: preparation/status are local; execution requires --execute.
use crate::{
    Gate, Outcome, Persona, Profile, Provider, Report, Request, Result, ReviewFailure, Reviewer,
    Round, assess, load_manifest, prepare, require, run_gate, run_reviewer, source_fingerprint,
    storage, validate_report,
};
use std::path::Path;

/// Execute council CLI arguments, returning a machine-readable result and appropriate exit code.
/// # Errors
/// Refuses unknown flags, invalid JSON, stale artifacts and execution without --execute.
pub fn dispatch(args: &[String]) -> Result<(String, i32)> {
    let command = args.first().map_or("help", String::as_str);
    match command {
        "example" if args.len() == 1 => output(&example(), true),
        "prepare" if args.len() == 3 => {
            let request: Request = storage::read_json(Path::new(&args[1]), 128 * 1024)?;
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
            output(&result, result.outcome == Outcome::Passed)
        }
        "gate" if args.len() == 4 && args[3] == "--execute" => {
            let gate: Gate = serde_json::from_str(&format!("{:?}", args[2]))?;
            let results = run_gate(Path::new(&args[1]), gate)?;
            output(
                &results,
                results.iter().all(|r| r.outcome == Outcome::Passed),
            )
        }
        "import" if args.len() == 3 => import(Path::new(&args[1]), Path::new(&args[2])),
        "help" if args.len() <= 1 => Ok((HELP.into(), 0)),
        _ => Err(ReviewFailure(
            "invalid council command or missing explicit --execute; use council help".into(),
        )),
    }
}
const HELP: &str = "council example | prepare <request.json> <new-run-dir> | status <run-dir> | review <run-dir> <reviewer-id> <independent|challenge> --execute | gate <run-dir> <format|check|clippy|pedantic|tests|antipatterns|fuzz|mutation> --execute | import <run-dir> <report.json>\n";
fn output(value: &impl serde::Serialize, passed: bool) -> Result<(String, i32)> {
    Ok((
        serde_json::to_string_pretty(value)? + "\n",
        if passed { 0 } else { 7 },
    ))
}
fn status(directory: &Path) -> Result<(String, i32)> {
    let manifest = load_manifest(directory)?;
    let reports = storage::reports(directory, &manifest)?;
    let executions = storage::executions(directory, &manifest)?;
    let tree = source_fingerprint(Path::new(&manifest.request.repository))?;
    let mut result = assess(&manifest, &reports, &executions, &tree)?;
    if directory.join("controller.lock").exists() {
        result.state = "hold".into();
        result
            .reasons
            .push("controller execution is active or interrupted".into());
    }
    output(&result, result.state == "ready_for_human_review")
}
fn import(directory: &Path, path: &Path) -> Result<(String, i32)> {
    let manifest = load_manifest(directory)?;
    let report: Report = storage::read_json(path, storage::REPORT_CAP)?;
    validate_report(&manifest, &report)?;
    require(
        source_fingerprint(Path::new(&manifest.request.repository))? == manifest.source_tree,
        "source changed; import would be stale",
    )?;
    storage::write_json(
        &storage::report_path(directory, &report.reviewer_id, report.round),
        &report,
    )?;
    Ok(("{\"imported\":true,\"provenance\":\"manual_unverified\",\"satisfies_execution_gate\":false}\n".into(), 0))
}
/// Portable request template; caller chooses concrete paths and available model IDs.
#[must_use]
pub fn example() -> Request {
    Request {
        repository: "/path/to/repository".into(),
        analysis_file: "/path/to/analysis.json".into(),
        decision_file: "/path/to/decision.md".into(),
        context_files: vec![],
        builder_family: "openai".into(),
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
        fuzz_targets: vec!["council_report".into()],
        timeout_seconds: 600,
        fuzz_seconds: 30,
    }
}
