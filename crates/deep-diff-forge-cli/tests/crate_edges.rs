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
