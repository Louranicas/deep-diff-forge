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
