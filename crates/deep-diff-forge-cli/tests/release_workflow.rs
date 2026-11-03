//! The release workflow keeps publication apart from compilation. GitHub grants permissions per
//! job, not per step, so a step-level `if:` cannot keep a write or OIDC token away from the build
//! scripts, proc macros and smoke-run binary of the same job; and a release published from inside
//! a matrix leg goes public before the other legs have passed. This reads the workflow's fixed
//! two-space layout (it is not a YAML executor) and checks both.

/// One job of a workflow: its job-level keys and every line of its body.
#[derive(Debug, Default)]
struct Job {
    name: String,
    needs: Vec<String>,
    condition: Option<String>,
    matrix: bool,
    grants: Vec<String>,
    lines: Vec<String>,
}

impl Job {
    /// The job holds a write grant or can mint an OIDC token.
    fn elevated(&self) -> bool {
        self.grants
            .iter()
            .any(|grant| grant.ends_with(": write") || grant == "write-all")
    }

    /// The job calls the action `owner/repo` (any pinned commit).
    fn uses(&self, action: &str) -> bool {
        self.lines.iter().any(|line| {
            let rest = line.trim_start();
            let rest = rest.strip_prefix("- ").unwrap_or(rest);
            rest.strip_prefix("uses:")
                .is_some_and(|target| target.trim().starts_with(&format!("{action}@")))
        })
    }

    /// The job checks out or compiles project code, or runs `cargo` or a built binary.
    fn runs_project_code(&self) -> bool {
        self.uses("actions/checkout")
            || self.uses("dtolnay/rust-toolchain")
            || self
                .lines
                .iter()
                .any(|line| line.contains("cargo ") || line.contains("target/"))
    }
}

fn jobs(workflow: &str) -> Vec<Job> {
    let mut jobs: Vec<Job> = Vec::new();
    let mut in_jobs = false;
    let mut in_grants = false;
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
            jobs.push(Job {
                name: key.to_owned(),
                ..Job::default()
            });
            in_grants = false;
            continue;
        }
        let Some(job) = jobs.last_mut() else { continue };
        job.lines.push(line.to_owned());
        if let Some(key) = line.strip_prefix("    ").filter(|k| !k.starts_with(' ')) {
            in_grants = false;
            if let Some(needs) = key.strip_prefix("needs:") {
                job.needs = needs
                    .trim()
                    .trim_matches(['[', ']'])
                    .split(',')
                    .map(|n| n.trim().to_owned())
                    .filter(|n| !n.is_empty())
                    .collect();
            } else if let Some(condition) = key.strip_prefix("if:") {
                job.condition = Some(condition.trim().to_owned());
            } else if key == "strategy:" {
                job.matrix = true;
            } else if key == "permissions:" {
                in_grants = true;
            } else if let Some(grant) = key.strip_prefix("permissions:") {
                job.grants.push(grant.trim().to_owned());
            }
        } else if in_grants
            && let Some(grant) = line.strip_prefix("      ").filter(|g| !g.starts_with(' '))
        {
            let grant = grant.split(" #").next().unwrap_or_default().trim();
            job.grants.push(grant.to_owned());
        }
    }
    jobs
}

/// Release-tag publication: the condition every publishing job must carry at job level.
const TAG_ONLY: &str = "github.event_name == 'push' && startsWith(github.ref, 'refs/tags/v')";

fn check(workflow: &str) -> Vec<String> {
    let jobs = jobs(workflow);
    let mut problems = Vec::new();
    for job in &jobs {
        if job.elevated() && job.runs_project_code() {
            problems.push(format!(
                "job `{}` holds {:?} and also checks out, compiles or runs project code",
                job.name, job.grants
            ));
        }
        let publishes =
            job.uses("softprops/action-gh-release") || job.uses("actions/attest-build-provenance");
        if publishes && job.matrix {
            problems.push(format!(
                "job `{}` publishes from inside a matrix leg, before the other legs pass",
                job.name
            ));
        }
        // Matrix legs run on different runners, and their system Pythons differ: the
        // `ubuntu-22.04-arm` one is 3.10, without `tomllib` (the v0.6.0 dry run failed on it).
        if job.matrix
            && job
                .lines
                .iter()
                .any(|line| !line.trim_start().starts_with('#') && line.contains("tomllib"))
            && !job.uses("actions/setup-python")
        {
            problems.push(format!(
                "matrix job `{}` imports tomllib without actions/setup-python",
                job.name
            ));
        }
        if publishes && !job.needs.iter().any(|n| n == "build") {
            problems.push(format!(
                "job `{}` publishes without `needs: build`",
                job.name
            ));
        }
        if (publishes || job.elevated())
            && !job
                .condition
                .as_deref()
                .is_some_and(|c| c.contains(TAG_ONLY))
        {
            problems.push(format!(
                "job `{}` publishes or elevates without a job-level release-tag `if:`",
                job.name
            ));
        }
    }
    if !jobs
        .iter()
        .any(|job| job.uses("softprops/action-gh-release"))
    {
        problems.push("no job creates the GitHub Release: the reader is broken".to_owned());
    }
    problems
}

#[test]
fn the_release_publishes_after_every_target_and_compiles_without_write_grants() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../.github/workflows/release.yml"
    );
    let workflow = std::fs::read_to_string(path).unwrap();
    let problems = check(&workflow);
    assert!(problems.is_empty(), "release.yml: {problems:#?}");
}

#[test]
fn the_reader_refuses_a_build_job_that_publishes() {
    // The shape release.yml had before the split: one matrix job that compiles with write and
    // OIDC grants, then attests and uploads from each leg.
    let before = "jobs:\n  build:\n    needs: [quality]\n    permissions:\n      contents: write # assets\n      id-token: write\n    strategy:\n      fail-fast: false\n    steps:\n      - uses: actions/checkout@x\n      - run: cargo build --release\n      - name: upload\n        if: github.event_name == 'push' && startsWith(github.ref, 'refs/tags/v')\n        uses: softprops/action-gh-release@x\n";
    let problems = check(before);
    assert_eq!(problems.len(), 4, "{problems:#?}");
    let after = "jobs:\n  build:\n    permissions:\n      contents: read\n    strategy:\n      fail-fast: false\n    steps:\n      - uses: actions/checkout@x\n      - run: cargo build --release\n  release:\n    needs: build\n    if: github.event_name == 'push' && startsWith(github.ref, 'refs/tags/v')\n    permissions:\n      contents: write\n    steps:\n      - uses: actions/download-artifact@x\n      - uses: softprops/action-gh-release@x\n";
    assert_eq!(check(after), Vec::<String>::new());
    let all = "jobs:\n  release:\n    needs: build\n    if: github.event_name == 'push' && startsWith(github.ref, 'refs/tags/v')\n    permissions: write-all\n    steps:\n      - uses: dtolnay/rust-toolchain@x\n      - uses: softprops/action-gh-release@x\n";
    assert_eq!(check(all).len(), 1, "{:#?}", check(all));
    let untagged = "jobs:\n  release:\n    needs: build\n    permissions:\n      contents: write\n    steps:\n      - uses: softprops/action-gh-release@x\n";
    assert_eq!(check(untagged).len(), 1, "{:#?}", check(untagged));
    assert_eq!(
        check("jobs:\n  a:\n    steps:\n      - run: true\n").len(),
        1
    );
}

#[test]
fn the_reader_refuses_tomllib_in_a_matrix_job_without_setup_python() {
    let release = "  release:\n    needs: build\n    if: github.event_name == 'push' && startsWith(github.ref, 'refs/tags/v')\n    permissions:\n      contents: write\n    steps:\n      - uses: softprops/action-gh-release@x\n";
    let build = "jobs:\n  build:\n    strategy:\n      fail-fast: false\n    steps:\n      - run: python3 -c 'import tomllib'\n";
    let problems = check(&format!("{build}{release}"));
    assert_eq!(problems.len(), 1, "{problems:#?}");
    assert!(problems[0].contains("tomllib"), "{problems:#?}");
    let pinned = "jobs:\n  build:\n    strategy:\n      fail-fast: false\n    steps:\n      - uses: actions/setup-python@x\n      - run: python3 -c 'import tomllib'\n";
    assert_eq!(check(&format!("{pinned}{release}")), Vec::<String>::new());
    let commented = "jobs:\n  build:\n    strategy:\n      fail-fast: false\n    steps:\n      # not tomllib: it is missing on 3.10\n      - run: sed -n p Cargo.toml\n";
    assert_eq!(
        check(&format!("{commented}{release}")),
        Vec::<String>::new()
    );
    let single = "jobs:\n  build:\n    steps:\n      - run: python3 -c 'import tomllib'\n";
    assert_eq!(check(&format!("{single}{release}")), Vec::<String>::new());
}

/// Every `runs-on:` label and matrix `os:` value that floats: a `-latest` label moves to a new
/// major image when GitHub repoints it, so the same commit builds and tests on a different
/// system without any change in this repository.
fn floating_runner_labels(workflow: &str) -> Vec<String> {
    workflow
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .filter_map(|line| {
            let at = line
                .find("runs-on:")
                .map(|i| i + "runs-on:".len())
                .or_else(|| line.find("{ os:").map(|i| i + "{ os:".len()))?;
            let label = line[at..]
                .split([',', '}'])
                .next()
                .unwrap_or_default()
                .trim();
            label.ends_with("-latest").then(|| label.to_owned())
        })
        .collect()
}

#[test]
fn no_workflow_runs_on_a_floating_runner_label() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../.github/workflows");
    let mut read = 0;
    for name in ["ci.yml", "release.yml", "campaign.yml", "codeql.yml"] {
        let workflow = std::fs::read_to_string(format!("{dir}/{name}")).unwrap();
        assert_eq!(
            floating_runner_labels(&workflow),
            Vec::<String>::new(),
            "{name}"
        );
        read += 1;
    }
    assert_eq!(read, 4);
}

#[test]
fn the_runner_reader_finds_a_floating_label() {
    let workflow = "jobs:\n  macos:\n    runs-on: macos-latest\n  build:\n    strategy:\n      matrix:\n        include:\n          - { os: ubuntu-latest, target: x }\n          - { os: macos-26, target: y }\n  pinned:\n    runs-on: ubuntu-24.04\n    # runs-on: windows-latest is a comment\n";
    assert_eq!(
        floating_runner_labels(workflow),
        vec!["macos-latest".to_owned(), "ubuntu-latest".to_owned()]
    );
}
