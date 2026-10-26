// Review council: Planted negative controls for authority, phase, freshness, citations, missing evidence and budgets.
// Design/status: docs/REVIEW_COUNCIL.md#policy-controls
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fcouncil--crates--deep-diff-forge-review--tests--contracts
//! Negative controls for review authority, freshness, blind phases and non-vacuous gates.
use deep_diff_forge_review::{
    Artifact, Assessment, Citation, Execution, Finding, Gate, Manifest, Outcome, Persona, Profile,
    Provider, Report, Round, Severity, Verdict, assess, validate_report, validate_request,
};
fn manifest() -> Manifest {
    let mut request = deep_diff_forge_review::cli::example();
    request.profile = Profile::Roster;
    request.reviewers.truncate(2);
    Manifest {
        schema: "deep-diff-forge.council.v1".into(),
        run_id: "run".into(),
        review_id: "review".into(),
        source_tree: "tree".into(),
        request,
        artifacts: vec![Artifact {
            id: "source".into(),
            label: "source.rs".into(),
            sha256: "digest".into(),
            text: "first\nsecond\n".into(),
        }],
    }
}
fn report(m: &Manifest, id: &str, round: Round) -> Report {
    Report {
        run_id: m.run_id.clone(),
        reviewer_id: id.into(),
        round,
        verdict: Verdict::NoBlockers,
        summary: "No blockers found in the captured scope".into(),
        findings: vec![],
        responses: vec![],
    }
}
fn execution(m: &Manifest, job: String) -> Execution {
    Execution {
        run_id: m.run_id.clone(),
        job,
        source_tree_before: m.source_tree.clone(),
        source_tree_after: m.source_tree.clone(),
        outcome: Outcome::Passed,
        exit_code: Some(0),
        elapsed_ms: 1,
        argv: vec!["trusted-test-controller".into()],
        stdout_sha256: "test".into(),
        stderr_sha256: "test".into(),
        image_id: None,
    }
}
fn complete(m: &Manifest) -> (Vec<Report>, Vec<Execution>) {
    let mut reports = Vec::new();
    let mut executions = Vec::new();
    for r in &m.request.reviewers {
        for round in [Round::Independent, Round::Challenge] {
            reports.push(report(m, &r.id, round));
            executions.push(execution(m, format!("review-{}-{}", r.id, round.label())));
        }
    }
    for gate in Gate::required(m.request.profile) {
        executions.push(execution(m, format!("gate-{}", gate.label())));
    }
    (reports, executions)
}
fn evaluation(m: &Manifest, reports: &[Report], executions: &[Execution]) -> Assessment {
    assess(m, reports, executions, &m.source_tree).unwrap()
}
fn finding(id: &str, severity: Severity) -> Finding {
    Finding {
        id: format!("{id}:one"),
        severity,
        claim: "Reachable boundary error".into(),
        citations: vec![Citation {
            artifact_id: "source".into(),
            start_line: 1,
            end_line: 2,
        }],
        falsifier: "Send the stated hostile frame and assert refusal before allocation".into(),
        requested_checks: vec![Gate::Tests, Gate::Fuzz],
    }
}
#[test]
fn complete_measured_review_only_becomes_ready_for_a_human() {
    let m = manifest();
    let (reports, executions) = complete(&m);
    let a = evaluation(&m, &reports, &executions);
    assert_eq!(a.state, "ready_for_human_review");
    assert_eq!(a.decision_owner, "human");
    assert_eq!(a.provider_families, vec!["anthropic", "openai"]);
    assert_eq!(a.reasons.len(), 0);
}
#[test]
fn unanimous_models_cannot_replace_missing_gate_evidence() {
    let m = manifest();
    let (reports, mut executions) = complete(&m);
    for gate in Gate::required(m.request.profile) {
        let mut planted = executions.clone();
        planted.retain(|e| e.job != format!("gate-{}", gate.label()));
        assert_eq!(evaluation(&m, &reports, &planted).state, "hold");
    }
    executions.clear();
    assert_eq!(evaluation(&m, &reports, &executions).state, "hold");
}
#[test]
fn every_reviewer_and_round_is_required() {
    let m = manifest();
    let (reports, executions) = complete(&m);
    for i in 0..reports.len() {
        let mut planted = reports.clone();
        planted.remove(i);
        assert_eq!(evaluation(&m, &planted, &executions).state, "hold");
    }
}
#[test]
fn timeout_failure_unavailable_inconclusive_and_stale_never_pass() {
    let m = manifest();
    let (reports, executions) = complete(&m);
    for state in [
        Outcome::Failed,
        Outcome::TimedOut,
        Outcome::OutputLimit,
        Outcome::Unavailable,
        Outcome::Inconclusive,
        Outcome::Stale,
    ] {
        let mut planted = executions.clone();
        planted[0].outcome = state;
        assert_eq!(evaluation(&m, &reports, &planted).state, "hold");
    }
    assert_eq!(
        assess(&m, &reports, &executions, "different tree")
            .unwrap()
            .state,
        "hold"
    );
    let mut planted = executions;
    planted[0].source_tree_before = "older".into();
    assert_eq!(evaluation(&m, &reports, &planted).state, "hold");
}
#[test]
fn duplicate_receipts_reports_and_roster_identities_refuse() {
    let m = manifest();
    let (mut reports, mut executions) = complete(&m);
    reports.push(reports[0].clone());
    assert!(assess(&m, &reports, &executions, "tree").is_err());
    reports.pop();
    executions.push(executions[0].clone());
    assert!(assess(&m, &reports, &executions, "tree").is_err());
    let mut request = m.request;
    request.reviewers[1].id = request.reviewers[0].id.clone();
    assert!(validate_request(&request).is_err());
}
#[test]
fn invented_evidence_stale_reports_and_wrong_ranges_refuse() {
    let m = manifest();
    let id = &m.request.reviewers[0].id;
    let mut r = report(&m, id, Round::Independent);
    r.findings.push(finding(id, Severity::Medium));
    assert!(validate_report(&m, &r).is_ok());
    for (start, end) in [(0, 1), (2, 1), (1, 3)] {
        let mut bad = r.clone();
        bad.findings[0].citations[0].start_line = start;
        bad.findings[0].citations[0].end_line = end;
        assert!(validate_report(&m, &bad).is_err());
    }
    let mut bad = r.clone();
    bad.findings[0].citations[0].artifact_id = "invented".into();
    assert!(validate_report(&m, &bad).is_err());
    let mut bad = r.clone();
    bad.run_id = "older".into();
    assert!(validate_report(&m, &bad).is_err());
    let mut bad = r;
    bad.findings[0].falsifier.clear();
    assert!(validate_report(&m, &bad).is_err());
}
#[test]
fn self_grading_personas_do_not_create_provider_diversity() {
    let mut m = manifest();
    m.request.reviewers[1].provider = Provider::Codex;
    m.request.reviewers[1].persona = Persona::Security;
    assert!(validate_request(&m.request).is_err());
}
#[test]
fn severe_dissent_cannot_be_voted_away() {
    let m = manifest();
    let (mut reports, executions) = complete(&m);
    let id = reports[0].reviewer_id.clone();
    reports[0].findings.push(finding(&id, Severity::High));
    reports[0].verdict = Verdict::ChangesRequested;
    let a = evaluation(&m, &reports, &executions);
    assert_eq!(a.state, "hold");
    assert!(a.reasons.iter().any(|r| r.contains("high/critical")));
    reports[0].verdict = Verdict::NoBlockers;
    assert!(validate_report(&m, &reports[0]).is_err());
}
#[test]
fn challenge_requires_exact_other_findings_and_keeps_test_requests_open() {
    let m = manifest();
    let (mut reports, mut executions) = complete(&m);
    let id = reports[0].reviewer_id.clone();
    reports[0].findings.push(finding(&id, Severity::Medium));
    assert_eq!(evaluation(&m, &reports, &executions).state, "hold");
    reports[3]
        .responses
        .push(deep_diff_forge_review::Challenge {
            finding_id: format!("{id}:one"),
            disposition: "needs_test".into(),
            rationale: "Need the hostile-input test".into(),
        });
    assert_eq!(evaluation(&m, &reports, &executions).state, "hold");
    reports[3].responses[0].disposition = "concur".into();
    assert_eq!(evaluation(&m, &reports, &executions).state, "hold");
    executions.push(execution(&m, "gate-fuzz".into()));
    assert_eq!(
        evaluation(&m, &reports, &executions).state,
        "ready_for_human_review"
    );
}
#[test]
fn unknown_and_duplicate_json_fields_cannot_smuggle_authority() {
    let m = manifest();
    let r = report(&m, &m.request.reviewers[0].id, Round::Independent);
    let mut value = serde_json::to_value(&r).unwrap();
    value["approved"] = true.into();
    assert!(serde_json::from_value::<Report>(value).is_err());
    let json = serde_json::to_string(&r).unwrap();
    let planted = json.replacen('{', "{\"run_id\":\"forged\",", 1);
    assert!(serde_json::from_str::<Report>(&planted).is_err());
}
#[test]
fn important_profile_requires_adversarial_lenses_and_real_fuzz_mutation_gates() {
    let request = deep_diff_forge_review::cli::example();
    assert!(validate_request(&request).is_ok());
    for persona in [
        Persona::Correctness,
        Persona::Security,
        Persona::Architecture,
        Persona::Falsifier,
    ] {
        let mut planted = request.clone();
        planted.reviewers.retain(|r| r.persona != persona);
        assert!(validate_request(&planted).is_err());
    }
    let gates = Gate::required(Profile::ImportantDecision);
    assert!(gates.contains(&Gate::Fuzz));
    assert!(gates.contains(&Gate::Mutation));
}
#[test]
fn execution_options_and_budget_values_are_admitted_not_silently_clamped() {
    let valid = manifest().request;
    for timeout in [0, 3601, u64::MAX] {
        let mut p = valid.clone();
        p.timeout_seconds = timeout;
        assert!(validate_request(&p).is_err());
    }
    for fuzz in [0, 601, u64::MAX] {
        let mut p = valid.clone();
        p.fuzz_seconds = fuzz;
        assert!(validate_request(&p).is_err());
    }
    for id in ["", "../escape", "x;whoami"] {
        let mut p = valid.clone();
        p.reviewers[0].id = id.into();
        assert!(validate_request(&p).is_err());
    }
    let mut p = valid.clone();
    p.reviewers[0].model = Some("--dangerously-bypass-approvals-and-sandbox".into());
    assert!(validate_request(&p).is_err());
    let mut p = valid;
    p.fuzz_targets.clear();
    assert!(validate_request(&p).is_err());
}
#[test]
fn identifier_boundaries_do_not_admit_shell_or_path_syntax() {
    let valid = manifest().request;
    for id in ["a", "_", "-", "Z09_-", &"x".repeat(80)] {
        let mut p = valid.clone();
        p.reviewers[0].id = id.into();
        assert!(validate_request(&p).is_ok(), "{id}");
    }
    for id in ["", &"x".repeat(81), "/", " ", "&", "x.y", "界"] {
        let mut p = valid.clone();
        p.reviewers[0].id = id.into();
        assert!(validate_request(&p).is_err(), "{id}");
    }
}
#[test]
fn receipt_identity_both_source_sides_and_nonzero_exit_refuse() {
    let m = manifest();
    let (reports, executions) = complete(&m);
    let mut bad = executions.clone();
    bad[0].run_id = "other".into();
    assert_eq!(evaluation(&m, &reports, &bad).state, "hold");
    let mut bad = executions.clone();
    bad[0].source_tree_after = "other".into();
    assert_eq!(evaluation(&m, &reports, &bad).state, "hold");
    let mut bad = executions;
    bad[0].exit_code = Some(1);
    assert_eq!(evaluation(&m, &reports, &bad).state, "hold");
}
