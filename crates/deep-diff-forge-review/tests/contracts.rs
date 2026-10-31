// Review council: Planted negative controls for authority, phase, freshness, citations, missing evidence and budgets.
// Design/status: docs/REVIEW_COUNCIL.md#policy-controls
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fcouncil--crates--deep-diff-forge-review--tests--contracts
//! Negative controls for review authority, freshness, blind phases and non-vacuous gates.
use deep_diff_forge_review::{
    Artifact, Assessment, AssessmentState, Citation, Disposition, Execution, Finding, Gate,
    Manifest, Outcome, Persona, Profile, Provider, Report, Round, SealedManifest, Severity,
    Verdict, assess, load_manifest, validate_report, validate_request,
};
use std::sync::atomic::{AtomicUsize, Ordering};

/// Seal `m` as `prepare` does and read it back through `load_manifest`, the only way to get the
/// `SealedManifest` that `assess` takes (anchor C-01).
fn seal(mut m: Manifest) -> SealedManifest {
    static RUN: AtomicUsize = AtomicUsize::new(0);
    for artifact in &mut m.artifacts {
        artifact.sha256 = deep_diff_forge_core::sha256_hex(artifact.text.as_bytes());
    }
    m.run_id.clear();
    m.run_id = deep_diff_forge_core::sha256_hex(&serde_json::to_vec(&m).unwrap());
    let directory = std::env::temp_dir().join(format!(
        "ddf-review-contracts-{}-{}",
        std::process::id(),
        RUN.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        directory.join("manifest.json"),
        serde_json::to_vec_pretty(&m).unwrap(),
    )
    .unwrap();
    let sealed = load_manifest(&directory).unwrap();
    std::fs::remove_dir_all(&directory).unwrap();
    sealed
}
fn manifest() -> SealedManifest {
    seal(unsealed())
}
fn unsealed() -> Manifest {
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
        mutation_diff_sha256: None,
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
/// Whether the assessment holds for a reason containing `text` (a state alone can hold for
/// another reason, so a control asserts the reason it targets).
fn holds_for(a: &Assessment, text: &str) -> bool {
    a.reasons.iter().any(|reason| reason.contains(text))
}
fn evaluation(m: &SealedManifest, reports: &[Report], executions: &[Execution]) -> Assessment {
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
    assert_eq!(a.state, AssessmentState::ReadyForHumanReview);
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
        assert_eq!(
            evaluation(&m, &reports, &planted).state,
            AssessmentState::Hold
        );
    }
    executions.clear();
    assert_eq!(
        evaluation(&m, &reports, &executions).state,
        AssessmentState::Hold
    );
}
#[test]
fn every_reviewer_and_round_is_required() {
    let m = manifest();
    let (reports, executions) = complete(&m);
    for i in 0..reports.len() {
        let mut planted = reports.clone();
        planted.remove(i);
        assert_eq!(
            evaluation(&m, &planted, &executions).state,
            AssessmentState::Hold
        );
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
        assert_eq!(
            evaluation(&m, &reports, &planted).state,
            AssessmentState::Hold
        );
    }
    assert_eq!(
        assess(&m, &reports, &executions, "different tree")
            .unwrap()
            .state,
        AssessmentState::Hold
    );
    let mut planted = executions;
    planted[0].source_tree_before = "older".into();
    assert_eq!(
        evaluation(&m, &reports, &planted).state,
        AssessmentState::Hold
    );
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
    let mut request = m.request.clone();
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
    let mut m = unsealed();
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
    assert_eq!(a.state, AssessmentState::Hold);
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
    let a = evaluation(&m, &reports, &executions);
    assert_eq!(a.state, AssessmentState::Hold);
    assert!(holds_for(&a, "does not address exactly"), "{:?}", a.reasons);
    reports[3]
        .responses
        .push(deep_diff_forge_review::Challenge {
            finding_id: format!("{id}:one"),
            disposition: Disposition::NeedsTest,
            rationale: "Need the hostile-input test".into(),
        });
    let a = evaluation(&m, &reports, &executions);
    assert_eq!(a.state, AssessmentState::Hold);
    assert!(
        holds_for(&a, "requests unresolved verification"),
        "{:?}",
        a.reasons
    );
    assert!(
        !holds_for(&a, "does not address exactly"),
        "{:?}",
        a.reasons
    );
    reports[3].responses[0].disposition = Disposition::Concur;
    let a = evaluation(&m, &reports, &executions);
    assert_eq!(a.state, AssessmentState::Hold);
    assert!(
        !holds_for(&a, "requests unresolved verification"),
        "{:?}",
        a.reasons
    );
    executions.push(execution(&m, "gate-fuzz".into()));
    assert_eq!(
        evaluation(&m, &reports, &executions).state,
        AssessmentState::ReadyForHumanReview
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
fn mutation_base_is_a_full_commit_id_never_a_revision_expression() {
    let valid = unsealed().request;
    for base in ["0".repeat(40), "a".repeat(64)] {
        let mut p = valid.clone();
        p.mutation_base = Some(base);
        assert!(validate_request(&p).is_ok());
    }
    for base in [
        String::new(),
        "HEAD".into(),
        "0".repeat(39),
        "0".repeat(41),
        "A".repeat(40),
        format!("--output={}", "0".repeat(31)),
        format!("{}~1", "0".repeat(38)),
    ] {
        let mut p = valid.clone();
        p.mutation_base = Some(base.clone());
        assert!(validate_request(&p).is_err(), "{base}");
    }
    // Absent means the whole package, and is omitted from the sealed JSON so older run IDs hold.
    assert!(
        !serde_json::to_string(&valid)
            .unwrap()
            .contains("mutation_base")
    );
}
#[test]
fn execution_options_and_budget_values_are_admitted_not_silently_clamped() {
    let valid = unsealed().request;
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
    let valid = unsealed().request;
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
    assert_eq!(evaluation(&m, &reports, &bad).state, AssessmentState::Hold);
    let mut bad = executions.clone();
    bad[0].source_tree_after = "other".into();
    assert_eq!(evaluation(&m, &reports, &bad).state, AssessmentState::Hold);
    let mut bad = executions;
    bad[0].exit_code = Some(1);
    assert_eq!(evaluation(&m, &reports, &bad).state, AssessmentState::Hold);
}

/// A builder outside every reviewer family has independent lineage: the reviewers' mixed families
/// never count as self-review. (The opposite case cannot be built: `validate_request` already
/// requires provider diversity.)
#[test]
fn a_human_builder_reviewed_by_both_families_has_independent_lineage() {
    let mut raw = unsealed();
    raw.request.builder_family = deep_diff_forge_review::BuilderFamily::Human;
    let m = seal(raw);
    let (reports, executions) = complete(&m);
    let a = evaluation(&m, &reports, &executions);
    assert!(!holds_for(&a, "lineage"), "{:?}", a.reasons);
    assert_eq!(
        a.state,
        AssessmentState::ReadyForHumanReview,
        "{:?}",
        a.reasons
    );
}

/// The refusal `result` carries names `text`, so a control proves the check it targets refused,
/// not some earlier one.
fn refuses<T: std::fmt::Debug>(
    result: Result<T, deep_diff_forge_review::ReviewFailure>,
    text: &str,
) {
    match result {
        Err(e) => assert!(e.0.contains(text), "refused for {:?}, wanted {text:?}", e.0),
        Ok(value) => panic!("admitted {value:?}, wanted a refusal for {text:?}"),
    }
}

/// A pinned model is passed to the provider CLI as one argument, so it must be a bounded,
/// nonempty token that cannot be read as an option or split into several.
#[test]
fn a_pinned_model_is_one_bounded_non_option_token() {
    let valid = unsealed().request;
    for model in ["gpt-5", "claude-opus-5-5", &"m".repeat(128)] {
        let mut p = valid.clone();
        p.reviewers[0].model = Some(model.into());
        assert!(validate_request(&p).is_ok(), "{model}");
    }
    for model in ["", &"m".repeat(129), "-x", "gpt 5", "gpt\t5", "gpt5\n"] {
        let mut p = valid.clone();
        p.reviewers[0].model = Some(model.into());
        refuses(validate_request(&p), "invalid model argument");
    }
}

/// A report naming a reviewer outside the roster is refused, whatever the roster's other ids.
#[test]
fn a_report_from_outside_the_roster_is_refused() {
    let m = manifest();
    for stranger in ["stranger", "correctness"] {
        refuses(
            validate_report(&m, &report(&m, stranger, Round::Independent)),
            "unknown reviewer",
        );
    }
}

/// Summaries, claims, falsifiers and rationales are each refused when blank and when over their
/// byte budget, and admitted at the budget.
#[test]
fn report_text_fields_are_nonblank_and_bounded() {
    let m = manifest();
    let id = m.request.reviewers[0].id.clone();
    let mut base = report(&m, &id, Round::Independent);
    base.findings.push(finding(&id, Severity::Medium));
    assert!(validate_report(&m, &base).is_ok());

    for (summary, ok) in [
        (String::new(), false),
        (" \n\t".into(), false),
        ("s".repeat(8192), true),
        ("s".repeat(8193), false),
    ] {
        let mut r = base.clone();
        r.summary = summary;
        if ok {
            assert!(validate_report(&m, &r).is_ok());
        } else {
            refuses(validate_report(&m, &r), "invalid summary");
        }
    }
    for (claim, ok) in [
        (String::new(), false),
        ("  ".into(), false),
        ("c".repeat(8192), true),
        ("c".repeat(8193), false),
    ] {
        let mut r = base.clone();
        r.findings[0].claim = claim;
        if ok {
            assert!(validate_report(&m, &r).is_ok());
        } else {
            refuses(validate_report(&m, &r), "invalid claim");
        }
    }
    for (falsifier, ok) in [
        ("  ".to_owned(), false),
        ("f".repeat(2048), true),
        ("f".repeat(2049), false),
    ] {
        let mut r = base.clone();
        r.findings[0].falsifier = falsifier;
        if ok {
            assert!(validate_report(&m, &r).is_ok());
        } else {
            refuses(validate_report(&m, &r), "falsification route");
        }
    }
}

/// Finding IDs carry their author's prefix, are bounded, and are unique within a report.
#[test]
fn finding_ids_are_prefixed_bounded_and_unique() {
    let m = manifest();
    let id = m.request.reviewers[0].id.clone();
    let other = m.request.reviewers[1].id.clone();
    let mut base = report(&m, &id, Round::Independent);
    base.findings.push(finding(&id, Severity::Medium));
    let prefix = format!("{id}:");
    let at_cap = format!("{prefix}{}", "x".repeat(164 - prefix.len()));
    let mut r = base.clone();
    r.findings[0].id = at_cap.clone();
    assert!(validate_report(&m, &r).is_ok());
    for bad in [format!("{other}:one"), "one".into(), format!("{at_cap}x")] {
        let mut r = base.clone();
        r.findings[0].id = bad.clone();
        refuses(validate_report(&m, &r), "finding identity");
    }
    let mut r = base;
    r.findings.push(r.findings[0].clone());
    refuses(validate_report(&m, &r), "finding identity");
}

/// A finding cites between one and sixteen captured ranges.
#[test]
fn findings_cite_one_to_sixteen_ranges() {
    let m = manifest();
    let id = m.request.reviewers[0].id.clone();
    let mut r = report(&m, &id, Round::Independent);
    r.findings.push(finding(&id, Severity::Medium));
    let citation = r.findings[0].citations[0].clone();
    r.findings[0].citations = vec![citation.clone(); 16];
    assert!(validate_report(&m, &r).is_ok());
    for count in [0, 17] {
        let mut bad = r.clone();
        bad.findings[0].citations = vec![citation.clone(); count];
        refuses(validate_report(&m, &bad), "bounded citations");
    }
}

/// Finding and response counts are each budgeted on their own: a report within one budget is
/// still refused for exceeding the other.
#[test]
fn finding_and_response_counts_are_each_budgeted() {
    let m = manifest();
    let id = m.request.reviewers[0].id.clone();
    let template = finding(&id, Severity::Medium);
    let findings = |n: usize| -> Vec<Finding> {
        (0..n)
            .map(|i| Finding {
                id: format!("{id}:{i}"),
                ..template.clone()
            })
            .collect()
    };
    let mut r = report(&m, &id, Round::Independent);
    r.findings = findings(128);
    assert!(validate_report(&m, &r).is_ok());
    r.findings = findings(129);
    refuses(validate_report(&m, &r), "count budget");

    let responses = |n: usize| -> Vec<deep_diff_forge_review::Challenge> {
        (0..n)
            .map(|i| deep_diff_forge_review::Challenge {
                finding_id: format!("other:{i}"),
                disposition: Disposition::Rebut,
                rationale: "Not reachable from the captured entry point".into(),
            })
            .collect()
    };
    let mut r = report(&m, &id, Round::Challenge);
    r.responses = responses(1536);
    assert!(validate_report(&m, &r).is_ok());
    r.responses = responses(1537);
    refuses(validate_report(&m, &r), "count budget");
}

/// A challenge answers each finding once, with a nonblank, bounded rationale.
#[test]
fn challenge_responses_are_unique_and_reasoned() {
    let m = manifest();
    let id = m.request.reviewers[0].id.clone();
    let response = deep_diff_forge_review::Challenge {
        finding_id: "other:one".into(),
        disposition: Disposition::Concur,
        rationale: "Reproduced with the stated frame".into(),
    };
    let mut r = report(&m, &id, Round::Challenge);
    r.responses.push(response.clone());
    assert!(validate_report(&m, &r).is_ok());
    let mut bad = r.clone();
    bad.responses.push(response);
    refuses(validate_report(&m, &bad), "challenge response");
    for (rationale, ok) in [
        (" \n".to_owned(), false),
        ("r".repeat(8192), true),
        ("r".repeat(8193), false),
    ] {
        let mut candidate = r.clone();
        candidate.responses[0].rationale = rationale;
        if ok {
            assert!(validate_report(&m, &candidate).is_ok());
        } else {
            refuses(validate_report(&m, &candidate), "challenge response");
        }
    }
}

/// The council expects one report per reviewer per round, at each roster size checked here.
/// Two reviewers alone cannot tell `n * 2` from `n + 2`; three can (5 != 6), so the roster
/// sizes are built explicitly rather than inherited from the example fixture's length.
#[test]
fn every_reviewer_owes_one_report_per_round() {
    for reviewers in [2, 3] {
        let mut raw = unsealed();
        raw.request = deep_diff_forge_review::cli::example();
        raw.request.profile = Profile::Roster;
        raw.request.reviewers.truncate(reviewers);
        let m = &seal(raw);
        assert_eq!(m.request.reviewers.len(), reviewers);
        let (reports, executions) = complete(m);
        let a = evaluation(m, &reports, &executions);
        assert_eq!(
            a.state,
            AssessmentState::ReadyForHumanReview,
            "{:?}",
            a.reasons
        );
        assert_eq!(a.reports_expected, reviewers * 2);
        assert_eq!(a.reports_received, a.reports_expected);
    }
}

/// Each persona has its own nonempty lens; two personas sharing one lens would be one reviewer
/// counted twice. This checks `Persona::lens` only, not that the reviewer prompt includes it.
#[test]
fn every_persona_has_its_own_lens() {
    const fn exhaustive(persona: Persona) {
        match persona {
            Persona::General
            | Persona::Correctness
            | Persona::Security
            | Persona::Architecture
            | Persona::Performance
            | Persona::Falsifier
            | Persona::Subtractor => {}
        }
    }
    exhaustive(Persona::General);
    let all = [
        Persona::General,
        Persona::Correctness,
        Persona::Security,
        Persona::Architecture,
        Persona::Performance,
        Persona::Falsifier,
        Persona::Subtractor,
    ];
    let lenses: std::collections::BTreeSet<_> = all.iter().map(|p| p.lens()).collect();
    assert_eq!(lenses.len(), all.len());
    for persona in all {
        assert!(!persona.lens().trim().is_empty(), "{persona:?}");
    }
}
