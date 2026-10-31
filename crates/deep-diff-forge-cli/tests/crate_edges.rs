//! Anchor B-05: the workspace's internal crate edges are the committed table `docs/crate-edges.txt`.
//! A dependency between workspace crates appears only as a reviewed change to that table.
#![expect(
    clippy::disallowed_methods,
    reason = "integration test: runs `cargo metadata` for the workspace graph"
)]

use serde_json::Value;
use std::collections::BTreeSet;
use std::process::Command;

const PREFIX: &str = "deep-diff-forge-";

/// The table's edges, one per non-comment line.
fn committed(text: &str) -> BTreeSet<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_owned)
        .collect()
}

/// Every edge between workspace crates in `cargo metadata` output, in the table's format.
fn actual(metadata: &Value) -> BTreeSet<String> {
    let packages = metadata["packages"].as_array().unwrap();
    let members: BTreeSet<&str> = packages
        .iter()
        .map(|package| package["name"].as_str().unwrap())
        .collect();
    let short = |name: &str| name.strip_prefix(PREFIX).unwrap_or(name).to_owned();
    let mut edges = BTreeSet::new();
    for package in packages {
        let from = short(package["name"].as_str().unwrap());
        for dependency in package["dependencies"].as_array().unwrap() {
            let to = dependency["name"].as_str().unwrap();
            if !members.contains(to) {
                continue;
            }
            let kind = dependency["kind"].as_str().unwrap_or("normal");
            let mut edge = format!("{kind} {from} -> {}", short(to));
            if let Some(target) = dependency["target"].as_str() {
                edge = format!("{edge} {target}");
            }
            edges.insert(edge);
        }
    }
    edges
}

#[test]
fn the_crate_graph_is_the_committed_table() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let output = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .args([
            "metadata",
            "--no-deps",
            "--format-version",
            "1",
            "--offline",
        ])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata: Value = serde_json::from_slice(&output.stdout).unwrap();
    let table = std::fs::read_to_string(format!("{root}/docs/crate-edges.txt")).unwrap();
    let (actual, committed) = (actual(&metadata), committed(&table));
    let unreviewed: Vec<_> = actual.difference(&committed).collect();
    let stale: Vec<_> = committed.difference(&actual).collect();
    assert!(
        unreviewed.is_empty() && stale.is_empty(),
        "docs/crate-edges.txt disagrees with Cargo.toml.\n  edges not in the table (review and add): {unreviewed:?}\n  table rows no Cargo.toml declares (remove): {stale:?}"
    );
}

#[test]
fn edges_keep_kind_and_target_apart() {
    let metadata = serde_json::json!({"packages": [
        {"name": "deep-diff-forge-a", "dependencies": [
            {"name": "deep-diff-forge-b", "kind": null, "target": null},
            {"name": "deep-diff-forge-b", "kind": "dev", "target": null},
            {"name": "deep-diff-forge-b", "kind": null, "target": "cfg(unix)"},
            {"name": "serde_json", "kind": null, "target": null}
        ]},
        {"name": "deep-diff-forge-b", "dependencies": []}
    ]});
    let edges: Vec<_> = actual(&metadata).into_iter().collect();
    assert_eq!(
        edges,
        ["dev a -> b", "normal a -> b", "normal a -> b cfg(unix)"]
    );
    assert_eq!(
        committed("# note\n\nnormal a -> b\n  dev a -> b  \n").len(),
        2
    );
}

/// Anchor E-02: release.yml's crates.io loop publishes every workspace crate, each after the
/// crates it depends on (normal edges). Before, it omitted `sdk` while announcing "all 12".
#[test]
fn the_release_loop_publishes_every_crate_after_its_dependencies() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let workflow =
        std::fs::read_to_string(format!("{root}/.github/workflows/release.yml")).unwrap();
    let order: Vec<&str> = workflow
        .lines()
        .find_map(|line| line.trim().strip_prefix("for crate in "))
        .and_then(|rest| rest.strip_suffix("; do"))
        .unwrap()
        .split_whitespace()
        .collect();
    let table = std::fs::read_to_string(format!("{root}/docs/crate-edges.txt")).unwrap();
    let edges: Vec<(String, String)> = committed(&table)
        .into_iter()
        .filter_map(|edge| {
            let rest = edge.strip_prefix("normal ")?;
            let (from, to) = rest.split_once(" -> ")?;
            Some((from.to_owned(), to.split_whitespace().next()?.to_owned()))
        })
        .collect();
    let crates: BTreeSet<&str> = edges
        .iter()
        .flat_map(|(from, to)| [from.as_str(), to.as_str()])
        .collect();
    let listed: BTreeSet<&str> = order.iter().copied().collect();
    assert_eq!(
        listed, crates,
        "the publish loop must name every workspace crate once"
    );
    assert_eq!(
        listed.len(),
        order.len(),
        "a crate is listed twice: {order:?}"
    );
    let position = |name: &str| order.iter().position(|c| *c == name).unwrap();
    for (from, to) in &edges {
        assert!(
            position(to) < position(from),
            "{from} is published before its dependency {to}: {order:?}"
        );
    }
}

/// The jobs of a workflow as (name, body), read from its fixed two-space layout.
fn jobs(workflow: &str) -> Vec<(String, String)> {
    let mut jobs: Vec<(String, String)> = Vec::new();
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
            .filter(|key| !key.starts_with(' ') && !key.contains(' '))
        {
            jobs.push((key.to_owned(), String::new()));
        } else if let Some((_, body)) = jobs.last_mut() {
            body.push_str(line);
            body.push('\n');
        }
    }
    jobs
}

/// The steps of a job body, each as its own text.
fn steps(body: &str) -> Vec<String> {
    let mut steps: Vec<String> = Vec::new();
    for line in body.lines() {
        if line.starts_with("      - ") {
            steps.push(String::new());
        }
        if let Some(step) = steps.last_mut() {
            step.push_str(line);
            step.push('\n');
        }
    }
    steps
}

/// The crates.io hold (Luke, 2026-10-27) is a condition in release.yml, not the absence of a
/// token; and a dispatched dry run publishes, releases and attests nothing. Before, the publish
/// job ran on every tag and skipped only because no `CARGO_REGISTRY_TOKEN` was set.
#[test]
fn release_publication_is_owner_gated_and_the_dry_run_publishes_nothing() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let workflow =
        std::fs::read_to_string(format!("{root}/.github/workflows/release.yml")).unwrap();
    let jobs = jobs(&workflow);
    let publishing: Vec<&str> = jobs
        .iter()
        .filter(|(_, body)| body.contains("cargo publish"))
        .map(|(name, _)| name.as_str())
        .collect();
    assert_eq!(
        publishing,
        ["publish"],
        "only the publish job may run cargo publish"
    );
    let publish = &jobs.iter().find(|(name, _)| name == "publish").unwrap().1;
    let condition = publish
        .lines()
        .find_map(|line| line.strip_prefix("    if: "))
        .unwrap_or_default();
    assert!(
        condition.contains("startsWith(github.ref, 'refs/tags/v')")
            && condition.contains("vars.DDF_CRATES_IO == 'open'"),
        "the publish job must run only on a tag and only with the owner's DDF_CRATES_IO=open: {condition:?}"
    );
    assert!(
        publish.contains("    needs: build\n"),
        "publication must wait for the builds"
    );
    assert!(
        publish.contains("    environment: release\n"),
        "the publish job must run in the `release` environment, which holds the token"
    );
    // An open gate without a token is a failed release, never a quiet success.
    assert!(
        publish.contains("::error::DDF_CRATES_IO is open but CARGO_REGISTRY_TOKEN is not set")
            && !publish.contains("skipping crates.io publish"),
        "a missing token must fail the publish job"
    );
    for (name, body) in &jobs {
        for step in steps(body) {
            let outward = ["softprops/action-gh-release", "attest-build-provenance"]
                .iter()
                .any(|action| step.contains(action));
            let tag_only = step
                .lines()
                .any(|line| line.trim_start().starts_with("if: ") && line.contains("refs/tags/"));
            assert!(
                !outward || tag_only,
                "job {name}: a step that releases or attests must be limited to tag refs:\n{step}"
            );
        }
    }
    let build = &jobs.iter().find(|(name, _)| name == "build").unwrap().1;
    assert!(
        build.contains("--stdin-patch --json --require-files")
            && build.contains("-c \"${dist}.tar.gz.sha256\""),
        "the build job must smoke the binary and verify the shipped checksum"
    );
    // Every shipped binary is run: no target may be built and released unexecuted.
    let smoke = steps(build)
        .into_iter()
        .find(|step| step.contains("name: smoke the release binary"))
        .expect("a smoke step");
    assert!(
        !smoke.contains("        if:"),
        "the smoke step must run for every target:\n{smoke}"
    );
    let targets: Vec<&str> = build
        .lines()
        .filter(|line| line.trim_start().starts_with("- { os:"))
        .collect();
    assert_eq!(targets.len(), 3, "the release matrix: {targets:?}");
    for target in targets {
        assert!(
            target.contains("run_under:") && !target.contains("smoke:"),
            "each target says how it is run: {target}"
        );
    }
}

#[test]
fn the_release_job_reader_sees_jobs_steps_and_conditions() {
    let workflow = "on: x\njobs:\n  a:\n    if: one\n    steps:\n      - name: s1\n        uses: softprops/action-gh-release@x\n      - name: s2\n        if: startsWith(github.ref, 'refs/tags/')\n  b:\n    steps:\n      - run: cargo publish\n";
    let jobs = jobs(workflow);
    let names: Vec<&str> = jobs.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, ["a", "b"]);
    let steps = steps(&jobs[0].1);
    assert_eq!(steps.len(), 2);
    assert!(steps[0].contains("softprops") && !steps[0].contains("if:"));
    assert!(steps[1].contains("refs/tags/"));
    assert!(jobs[1].1.contains("cargo publish") && !jobs[0].1.contains("cargo publish"));
}

/// The block between the markers in docs/SCHEMATICS.md, as `table` renders it: one line per crate
/// with its normal dependencies, then its dev-only ones.
fn rendered(table: &BTreeSet<String>) -> String {
    use std::collections::BTreeMap;
    use std::fmt::Write as _;
    let mut deps: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for edge in table {
        let mut words = edge.split_whitespace();
        let (Some(kind), Some(from), Some("->"), Some(to)) =
            (words.next(), words.next(), words.next(), words.next())
        else {
            panic!("malformed edge: {edge}");
        };
        deps.entry(to).or_default();
        let mut shown = to.to_owned();
        if let Some(target) = words.next() {
            shown = format!("{shown} ({target})");
        }
        if kind != "normal" {
            shown = format!("{shown} [{kind}]");
        }
        deps.entry(from).or_default().push(shown);
    }
    let mut out = String::new();
    for (name, mut parts) in deps {
        // Normal dependencies first, then dev-only ones, each alphabetical.
        parts.sort_by_key(|part| (part.ends_with(']'), part.clone()));
        let parts = if parts.is_empty() {
            "(no workspace dependencies)".to_owned()
        } else {
            parts.join(", ")
        };
        let _ = writeln!(out, "- `{name}` → {parts}");
    }
    out
}

/// Anchor E-01: the schematics' current-crates block is generated from the checked table, so the
/// documented graph cannot drift from Cargo.toml again (the sweep found 2 crates and 20+ edges
/// missing from the hand-drawn graphs).
#[test]
fn the_schematics_show_the_current_crate_graph() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let table =
        committed(&std::fs::read_to_string(format!("{root}/docs/crate-edges.txt")).unwrap());
    let doc = std::fs::read_to_string(format!("{root}/docs/SCHEMATICS.md")).unwrap();
    let (begin, end) = ("<!-- crate-edges:begin -->\n", "<!-- crate-edges:end -->");
    let (head, rest) = doc.split_once(begin).unwrap();
    let (block, tail) = rest.split_once(end).unwrap();
    let expected = rendered(&table);
    if std::env::var_os("DDF_BLESS_SCHEMATICS").is_some() {
        let blessed = format!("{head}{begin}{expected}{end}{tail}");
        std::fs::write(format!("{root}/docs/SCHEMATICS.md"), blessed).unwrap();
        return;
    }
    assert_eq!(
        block, expected,
        "docs/SCHEMATICS.md is stale; regenerate with DDF_BLESS_SCHEMATICS=1 cargo test -p deep-diff-forge-cli --test crate_edges"
    );
}
