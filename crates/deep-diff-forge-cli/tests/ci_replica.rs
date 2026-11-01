//! Anchor B-07: the local replica `scripts/ci_local.sh` runs every step of the CI gate jobs. Each
//! replica step is labelled with the exact name of the `ci.yml` step it replicates; this compares
//! the two lists (it is not a YAML executor), so a CI step added without a replica step, or a
//! replica step whose CI step was renamed or removed, fails.

use std::collections::BTreeSet;

/// The CI jobs the replica stands in for. The council sandbox and diff-mutation jobs need Podman
/// or hours of mutation and are run separately (`scripts/diff_mutation.sh`); the macOS job needs a
/// macOS runner.
const GATE_JOBS: &[&str] = &["secrets", "msrv", "gate", "supply-chain"];

/// CI steps that only install a tool the replica expects on the machine already.
const SETUP_ONLY: &[&str] = &["install host cargo-deny", "install cargo-audit"];

/// One step of a CI job: its name (if any) and whether it has a `run:` (else it `uses:` an action).
#[derive(Debug, PartialEq)]
struct CiStep {
    job: String,
    name: Option<String>,
    runs: bool,
}

/// The steps of every job in a workflow, read line by line from its fixed two-space layout.
fn ci_steps(workflow: &str) -> Vec<CiStep> {
    let mut steps: Vec<CiStep> = Vec::new();
    let mut job = None;
    let mut in_jobs = false;
    for line in workflow.lines() {
        if line == "jobs:" {
            in_jobs = true;
            continue;
        }
        if !in_jobs {
            continue;
        }
        if let Some(key) = line
            .strip_prefix("  ")
            .and_then(|rest| rest.strip_suffix(':'))
            && !key.starts_with(' ')
            && !key.contains(' ')
        {
            job = Some(key.to_owned());
            continue;
        }
        let Some(job) = &job else { continue };
        let (body, starts) = match line.strip_prefix("      - ") {
            Some(body) => (body, true),
            None => match line.strip_prefix("        ") {
                Some(body) if !body.starts_with(' ') => (body, false),
                _ => continue,
            },
        };
        if starts {
            steps.push(CiStep {
                job: job.clone(),
                name: None,
                runs: false,
            });
        }
        let Some(step) = steps.last_mut().filter(|step| &step.job == job) else {
            continue;
        };
        if let Some(name) = body.strip_prefix("name: ") {
            step.name = Some(name.trim().trim_matches('"').to_owned());
        } else if body.starts_with("run:") {
            step.runs = true;
        }
    }
    steps
}

/// The CI step names the replica's `step "<label>"` calls claim, without any ` :: <part>`.
fn replica_labels(script: &str) -> BTreeSet<String> {
    script
        .lines()
        .filter_map(|line| line.trim_start().strip_prefix("step \""))
        .filter_map(|rest| rest.split_once('"').map(|(label, _)| label))
        .map(|label| label.split(" :: ").next().unwrap().to_owned())
        .collect()
}

#[test]
fn the_replica_runs_every_ci_gate_step_and_only_those() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let workflow = std::fs::read_to_string(format!("{root}/.github/workflows/ci.yml")).unwrap();
    let script = std::fs::read_to_string(format!("{root}/scripts/ci_local.sh")).unwrap();
    let steps: Vec<_> = ci_steps(&workflow)
        .into_iter()
        .filter(|step| GATE_JOBS.contains(&step.job.as_str()))
        .collect();
    let unnamed: Vec<_> = steps
        .iter()
        .filter(|step| step.runs && step.name.is_none())
        .collect();
    assert!(unnamed.is_empty(), "name these CI steps: {unnamed:?}");
    let named = |runs_only: bool| -> BTreeSet<String> {
        steps
            .iter()
            .filter(|step| step.runs || !runs_only)
            .filter_map(|step| step.name.clone())
            .collect()
    };
    let (runs, all) = (named(true), named(false));
    let labels = replica_labels(&script);
    let unreplicated: Vec<_> = runs
        .iter()
        .filter(|name| !labels.contains(*name) && !SETUP_ONLY.contains(&name.as_str()))
        .collect();
    let orphaned: Vec<_> = labels.difference(&all).collect();
    let stale: Vec<_> = SETUP_ONLY
        .iter()
        .filter(|name| !runs.contains(**name))
        .collect();
    assert!(
        unreplicated.is_empty() && orphaned.is_empty() && stale.is_empty(),
        "scripts/ci_local.sh and ci.yml disagree.\n  CI steps with no replica step: {unreplicated:?}\n  replica steps naming no CI step: {orphaned:?}\n  setup-only names no CI step has: {stale:?}"
    );
}

#[test]
fn the_parsers_read_steps_jobs_and_labels() {
    let workflow = "on:\n  push:\njobs:\n  a:\n    steps:\n      - uses: x\n      - name: one\n        run: echo\n      - run: bare\n  b:\n    steps:\n      - name: \"two\"\n        uses: y\n";
    let step = |job: &str, name: Option<&str>, runs| CiStep {
        job: job.into(),
        name: name.map(Into::into),
        runs,
    };
    assert_eq!(
        ci_steps(workflow),
        [
            step("a", None, false),
            step("a", Some("one"), true),
            step("a", None, true),
            step("b", Some("two"), false),
        ]
    );
    let labels = replica_labels("step \"one\" echo\n  step \"two :: $c\" x\n# step \"no\"\n");
    assert_eq!(labels.into_iter().collect::<Vec<_>>(), ["one", "two"]);
}

/// The `toolchain:` each job of a workflow pins, in order.
fn ci_toolchains(workflow: &str) -> Vec<(String, String)> {
    let mut pins = Vec::new();
    let mut job = None;
    for line in workflow.lines().skip_while(|line| *line != "jobs:") {
        if let Some(key) = line
            .strip_prefix("  ")
            .and_then(|rest| rest.strip_suffix(':'))
            && !key.starts_with(' ')
            && !key.contains(' ')
        {
            job = Some(key.to_owned());
        } else if let (Some(job), Some(pin)) = (&job, line.trim_start().strip_prefix("toolchain: "))
        {
            pins.push((job.clone(), pin.trim().to_owned()));
        }
    }
    pins
}

/// The toolchain the replica pins for every step (`toolchain=<x>`), and the one its MSRV step
/// names (`cargo +<x>`).
fn replica_toolchains(script: &str) -> (Option<&str>, Option<&str>) {
    let pinned = script
        .lines()
        .find_map(|line| line.strip_prefix("toolchain="));
    let msrv = script
        .lines()
        .filter(|line| line.starts_with("step \"check on Rust "))
        .find_map(|line| line.split("cargo +").nth(1)?.split_whitespace().next());
    (pinned, msrv)
}

#[test]
fn the_replica_runs_on_the_toolchains_ci_pins() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let workflow = std::fs::read_to_string(format!("{root}/.github/workflows/ci.yml")).unwrap();
    let script = std::fs::read_to_string(format!("{root}/scripts/ci_local.sh")).unwrap();
    let (pinned, msrv) = replica_toolchains(&script);
    let pins: Vec<_> = ci_toolchains(&workflow)
        .into_iter()
        .filter(|(job, _)| GATE_JOBS.contains(&job.as_str()))
        .collect();
    assert!(
        pins.len() >= 3,
        "only {pins:?} pinned: the reader is broken"
    );
    for (job, pin) in &pins {
        let replica = if job == "msrv" { msrv } else { pinned };
        assert_eq!(
            replica,
            Some(pin.as_str()),
            "ci.yml job `{job}` runs on Rust {pin}; scripts/ci_local.sh must run its steps on the same toolchain (a floating default drifts on the next `rustup update`)"
        );
    }
}

#[test]
fn the_toolchain_readers_read_pins() {
    let workflow = "on:\njobs:\n  a:\n    steps:\n      - uses: x\n        with:\n          toolchain: 1.2.3\n  b:\n    steps:\n      - with:\n          toolchain: 1.0.0\n";
    assert_eq!(
        ci_toolchains(workflow),
        [("a".into(), "1.2.3".into()), ("b".into(), "1.0.0".into())]
    );
    let script =
        "toolchain=1.2.3\nstep \"check on Rust 1.0\" cargo +1.0.0 check\nstep \"x\" cargo +9 y\n";
    assert_eq!(replica_toolchains(script), (Some("1.2.3"), Some("1.0.0")));
    assert_eq!(replica_toolchains("step \"x\" cargo check\n"), (None, None));
}
