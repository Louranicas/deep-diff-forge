// Review council: Fail-closed request/report admission and synthesis: no missing quorum, stale evidence or vote-based approval.
// Design/status: docs/REVIEW_COUNCIL.md#admission-and-synthesis
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fcouncil--crates--deep-diff-forge-review--src--policy
//! Admission, independent rounds, and fail-closed synthesis; votes cannot satisfy tool gates.
use crate::{
    Assessment, Execution, Gate, Manifest, Outcome, Persona, Profile, Report, Request, Result,
    ReviewFailure, Round, Severity, Verdict, identifier, require,
};
use std::collections::{BTreeMap, BTreeSet};

/// Validate a controller-authored request before producing prompts or invoking tools.
/// # Errors
/// Rejects missing diversity, duplicate identities, empty scopes and unbounded budgets.
pub fn validate_request(request: &Request) -> Result<()> {
    require(
        (2..=12).contains(&request.reviewers.len()),
        "roster needs 2–12 reviewers",
    )?;
    require(
        matches!(
            request.builder_family.as_str(),
            "openai" | "anthropic" | "human" | "other"
        ),
        "builder_family must be explicit",
    )?;
    require(
        (1..=3600).contains(&request.timeout_seconds),
        "timeout must be 1–3600 seconds",
    )?;
    require(
        (1..=600).contains(&request.fuzz_seconds),
        "fuzz budget must be 1–600 seconds",
    )?;
    require(request.context_files.len() <= 32, "too many context files")?;
    require(
        identifier(&request.mutation_package),
        "mutation package must be one explicit package",
    )?;
    require(
        !request.fuzz_targets.is_empty() && request.fuzz_targets.len() <= 8,
        "fuzz scope must be explicit and nonempty",
    )?;
    for target in &request.fuzz_targets {
        require(identifier(target), "invalid fuzz target")?;
    }
    let mut ids = BTreeSet::new();
    let mut families = BTreeSet::new();
    for reviewer in &request.reviewers {
        require(
            identifier(&reviewer.id) && ids.insert(&reviewer.id),
            "invalid or duplicate reviewer ID",
        )?;
        if let Some(model) = &reviewer.model {
            require(
                !model.is_empty()
                    && model.len() <= 128
                    && !model.starts_with('-')
                    && !model.chars().any(char::is_whitespace),
                "invalid model argument",
            )?;
        }
        families.insert(reviewer.provider.family());
    }
    require(
        families.len() >= 2,
        "personas in one provider family are not independent providers",
    )?;
    if request.profile == Profile::ImportantDecision {
        for persona in [
            Persona::Correctness,
            Persona::Security,
            Persona::Architecture,
            Persona::Falsifier,
        ] {
            require(
                request.reviewers.iter().any(|r| r.persona == persona),
                format!("important decision lacks {persona:?} lens"),
            )?;
        }
    }
    Ok(())
}
/// Validate untrusted model output against the captured run and actual cited text.
/// # Errors
/// Refuses stale identities, fabricated/out-of-range citations, duplicates and inconsistent verdicts.
pub fn validate_report(manifest: &Manifest, report: &Report) -> Result<()> {
    require(
        report.run_id == manifest.run_id,
        "stale or unrelated report",
    )?;
    require(
        manifest
            .request
            .reviewers
            .iter()
            .any(|r| r.id == report.reviewer_id),
        "unknown reviewer",
    )?;
    require(
        !report.summary.trim().is_empty() && report.summary.len() <= 8192,
        "invalid summary",
    )?;
    require(
        report.findings.len() <= 128 && report.responses.len() <= 1536,
        "report count budget exceeded",
    )?;
    let mut ids = BTreeSet::new();
    for finding in &report.findings {
        require(
            finding.id.starts_with(&format!("{}:", report.reviewer_id))
                && finding.id.len() <= 164
                && ids.insert(&finding.id),
            "invalid or duplicate finding identity",
        )?;
        require(
            !finding.claim.trim().is_empty() && finding.claim.len() <= 8192,
            "invalid claim",
        )?;
        require(
            !finding.falsifier.trim().is_empty() && finding.falsifier.len() <= 2048,
            "claim needs a concrete falsification route",
        )?;
        require(
            !finding.citations.is_empty() && finding.citations.len() <= 16,
            "claim needs bounded citations",
        )?;
        require(
            finding.requested_checks.len() <= 8,
            "too many requested checks",
        )?;
        for citation in &finding.citations {
            let artifact = manifest
                .artifacts
                .iter()
                .find(|a| a.id == citation.artifact_id)
                .ok_or_else(|| ReviewFailure("fabricated artifact citation".into()))?;
            require(
                citation.start_line > 0
                    && citation.end_line >= citation.start_line
                    && citation.end_line <= artifact.text.lines().count(),
                "citation line range is outside captured artifact",
            )?;
        }
    }
    require(
        report.verdict != Verdict::NoBlockers
            || !report.findings.iter().any(|f| f.severity >= Severity::High),
        "no_blockers contradicts a high/critical finding",
    )?;
    require(
        report.round != Round::Independent || report.responses.is_empty(),
        "independent round must be blind",
    )?;
    let mut responses = BTreeSet::new();
    for response in &report.responses {
        require(
            responses.insert(&response.finding_id)
                && !response.rationale.trim().is_empty()
                && response.rationale.len() <= 8192,
            "invalid or duplicate challenge response",
        )?;
        require(
            matches!(
                response.disposition.as_str(),
                "concur" | "rebut" | "needs_test"
            ),
            "invalid challenge disposition",
        )?;
    }
    Ok(())
}
/// Synthesize measured receipts and untrusted reviews without majority-vote approval.
/// # Errors
/// Rejects invalid/duplicate reports rather than silently dropping them from the roster.
pub fn assess(
    manifest: &Manifest,
    reports: &[Report],
    executions: &[Execution],
    current_tree: &str,
) -> Result<Assessment> {
    validate_request(&manifest.request)?;
    let mut reasons = Vec::new();
    let mut received = BTreeMap::new();
    if current_tree != manifest.source_tree {
        reasons.push("workspace changed: reviews and gate receipts are stale".into());
    }
    for report in reports {
        validate_report(manifest, report)?;
        require(
            received
                .insert((report.reviewer_id.clone(), report.round.label()), report)
                .is_none(),
            "duplicate report for reviewer/round",
        )?;
    }
    let independent: Vec<_> = reports
        .iter()
        .filter(|r| r.round == Round::Independent)
        .collect();
    let external_findings: BTreeMap<_, _> = independent
        .iter()
        .flat_map(|r| {
            r.findings
                .iter()
                .map(move |f| (f.id.as_str(), r.reviewer_id.as_str()))
        })
        .collect();
    let expected = manifest.request.reviewers.len() * 2;
    let mut jobs = BTreeMap::new();
    for execution in executions {
        require(
            jobs.insert(execution.job.as_str(), execution).is_none(),
            "duplicate execution receipt",
        )?;
    }
    for reviewer in &manifest.request.reviewers {
        for round in [Round::Independent, Round::Challenge] {
            let job = format!("review-{}-{}", reviewer.id, round.label());
            check_execution(
                manifest,
                jobs.get(job.as_str()).copied(),
                &job,
                &mut reasons,
            );
            let Some(report) = received.get(&(reviewer.id.clone(), round.label())) else {
                reasons.push(format!("missing {} {} review", reviewer.id, round.label()));
                continue;
            };
            if report.verdict != Verdict::NoBlockers {
                reasons.push(format!(
                    "{} {} is {:?}",
                    reviewer.id,
                    round.label(),
                    report.verdict
                ));
            }
            check_challenge(report, reviewer, &external_findings, &mut reasons);
        }
    }
    let mut required_gates: BTreeSet<_> = Gate::required(manifest.request.profile)
        .into_iter()
        .collect();
    required_gates.extend(reports.iter().flat_map(|r| {
        r.findings
            .iter()
            .flat_map(|f| f.requested_checks.iter().copied())
    }));
    for gate in required_gates {
        let job = format!("gate-{}", gate.label());
        check_execution(
            manifest,
            jobs.get(job.as_str()).copied(),
            &job,
            &mut reasons,
        );
    }
    let families = check_lineage(manifest, &mut reasons);
    // A later retraction does not erase an earlier blocking observation. A fix/new
    // decision requires a fresh run; the journal must retain the dissent.
    if independent
        .iter()
        .any(|r| r.findings.iter().any(|f| f.severity >= Severity::High))
    {
        reasons.push(
            "high/critical independent findings require a revised candidate and fresh run".into(),
        );
    }
    Ok(Assessment {
        run_id: manifest.run_id.clone(),
        state: if reasons.is_empty() {
            "ready_for_human_review"
        } else {
            "hold"
        }
        .into(),
        reasons,
        reports_received: received.len(),
        reports_expected: expected,
        provider_families: families.into_iter().collect(),
        decision_owner: "human".into(),
    })
}
fn check_lineage(manifest: &Manifest, reasons: &mut Vec<String>) -> BTreeSet<String> {
    let families: BTreeSet<_> = manifest
        .request
        .reviewers
        .iter()
        .map(|r| r.provider.family().to_owned())
        .collect();
    if !families
        .iter()
        .any(|f| f != &manifest.request.builder_family)
    {
        reasons.push("no independent reviewer lineage beyond the builder".into());
    }
    families
}

fn check_challenge(
    report: &Report,
    reviewer: &crate::Reviewer,
    external_findings: &BTreeMap<&str, &str>,
    reasons: &mut Vec<String>,
) {
    if report.round != Round::Challenge {
        return;
    }

    let needed: BTreeSet<_> = external_findings
        .iter()
        .filter(|(_, owner)| **owner != reviewer.id)
        .map(|(id, _)| *id)
        .collect();
    let addressed: BTreeSet<_> = report
        .responses
        .iter()
        .map(|r| r.finding_id.as_str())
        .collect();
    if addressed != needed {
        reasons.push(format!(
            "{} challenge does not address exactly the other reviewers' findings",
            reviewer.id
        ));
    }
    if report
        .responses
        .iter()
        .any(|r| r.disposition == "needs_test")
    {
        reasons.push(format!("{} requests unresolved verification", reviewer.id));
    }
}

fn check_execution(
    manifest: &Manifest,
    execution: Option<&Execution>,
    job: &str,
    reasons: &mut Vec<String>,
) {
    match execution {
        Some(e)
            if e.run_id == manifest.run_id
                && e.outcome == Outcome::Passed
                && e.exit_code == Some(0)
                && e.source_tree_before == manifest.source_tree
                && e.source_tree_after == manifest.source_tree => {}
        Some(e) => reasons.push(format!("{job}: {:?} or stale receipt", e.outcome)),
        None => reasons.push(format!("{job}: no measured execution receipt")),
    }
}
/// Strict common schema for both provider adapters. The controller also validates citations and freshness.
#[must_use]
pub fn report_schema() -> serde_json::Value {
    use serde_json::json;
    let citation = json!({"type":"object","additionalProperties":false,"required":["artifact_id","start_line","end_line"],"properties":{"artifact_id":{"type":"string"},"start_line":{"type":"integer"},"end_line":{"type":"integer"}}});
    let finding = json!({"type":"object","additionalProperties":false,"required":["id","severity","claim","citations","falsifier","requested_checks"],"properties":{"id":{"type":"string"},"severity":{"type":"string","enum":["low","medium","high","critical"]},"claim":{"type":"string"},"citations":{"type":"array","items":citation},"falsifier":{"type":"string"},"requested_checks":{"type":"array","items":{"type":"string","enum":["format","check","clippy","pedantic","tests","antipatterns","fuzz","mutation"]}}}});
    let response = json!({"type":"object","additionalProperties":false,"required":["finding_id","disposition","rationale"],"properties":{"finding_id":{"type":"string"},"disposition":{"type":"string","enum":["concur","rebut","needs_test"]},"rationale":{"type":"string"}}});
    json!({"type":"object","additionalProperties":false,"required":["run_id","reviewer_id","round","verdict","summary","findings","responses"],"properties":{"run_id":{"type":"string"},"reviewer_id":{"type":"string"},"round":{"type":"string","enum":["independent","challenge"]},"verdict":{"type":"string","enum":["no_blockers","changes_requested","abstain"]},"summary":{"type":"string"},"findings":{"type":"array","items":finding},"responses":{"type":"array","items":response}}})
}
