//! The evaluation gate (plan 2026-10-30, M3): the committed corpus (`fixtures/eval`) must score at
//! or above these floors, with no failure. Each wrong order, false positive, miss and wrong
//! verdict that is not declared is a failure (exit 7). A limit the engine has is declared in the
//! manifest (`known_miss`, `known_false_positive`) and reviewed like any other change; it counts
//! as wrong in its metric, so the floors below are what the engine measures, not 100%. A change
//! that fixes a declared miss must move its label, and raises the floor in the same change.

#![expect(
    clippy::disallowed_methods,
    reason = "integration test: drives the real binary as a child process"
)]

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Parts per million: 1,000,000 is every case right. Measured 2026-10-31 on 34 cases: ranking
/// 12 of 15 pairs, precision 21 of 22 items, recall 21 of 28 items, findings 33 of 33 verdicts.
const RANKING_PAIRS_FLOOR: u64 = 800_000;
const EVIDENCE_PRECISION_FLOOR: u64 = 954_545;
const EVIDENCE_RECALL_FLOOR: u64 = 750_000;
const FINDINGS_FLOOR: u64 = 1_000_000;
/// Declared misses (plan N4): each counts as wrong in its metric above.
const KNOWN_MISSES: u64 = 11;
/// The smallest corpus the gate accepts, and the fewest cases of each kind.
const MIN_CASES: usize = 30;
const MIN_CASES_PER_KIND: usize = 3;

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/eval")
}

fn eval(dir: &Path) -> (i32, Value, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .args(["eval", "--json", "--corpus", dir.to_str().unwrap()])
        .output()
        .expect("run");
    let stdout = String::from_utf8_lossy(&out.stdout);
    (
        out.status.code().unwrap_or(-1),
        serde_json::from_str(&stdout).unwrap_or(Value::Null),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn the_corpus_meets_its_floors() {
    let (code, receipt, stderr) = eval(&corpus());
    assert_eq!(code, 0, "{stderr}\n{receipt:#}");
    assert_eq!(receipt["schema"], "deep-diff-forge.eval.v0");
    let m = &receipt["metrics"];
    let ppm = |name: &str| m[name]["ppm"].as_u64().unwrap();
    assert!(ppm("ranking_pairs") >= RANKING_PAIRS_FLOOR, "{m:#}");
    assert!(
        ppm("evidence_precision") >= EVIDENCE_PRECISION_FLOOR,
        "{m:#}"
    );
    assert!(ppm("evidence_recall") >= EVIDENCE_RECALL_FLOOR, "{m:#}");
    assert!(ppm("findings_verdicts") >= FINDINGS_FLOOR, "{m:#}");
    assert_eq!(receipt["known_misses"], KNOWN_MISSES);
    assert_eq!(
        receipt["declared"].as_array().map(Vec::len),
        Some(usize::try_from(KNOWN_MISSES).unwrap()),
        "{receipt:#}"
    );
    assert_eq!(receipt["failures"], serde_json::json!([]));
    // A declared miss is counted, not excluded: every one is in a metric's total.
    let total = |name: &str| m[name]["total"].as_u64().unwrap();
    let right = |name: &str| m[name]["right"].as_u64().unwrap();
    let wrong: u64 = [
        "ranking_pairs",
        "evidence_precision",
        "evidence_recall",
        "findings_verdicts",
    ]
    .iter()
    .map(|name| total(name) - right(name))
    .sum();
    assert_eq!(wrong, KNOWN_MISSES, "{m:#}");
    // Every kind is exercised, so no metric is vacuously perfect.
    for name in [
        "ranking_pairs",
        "evidence_precision",
        "evidence_recall",
        "findings_verdicts",
    ] {
        assert!(
            m[name]["total"].as_u64().unwrap() > 0,
            "{name} measured nothing"
        );
    }
}

/// A copy of the corpus, removed when the test ends.
struct Copy(PathBuf);

impl Copy {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("ddf-eval-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for sub in ["", "ranking", "evidence", "findings"] {
            std::fs::create_dir_all(dir.join(sub)).unwrap();
            for entry in std::fs::read_dir(corpus().join(sub)).unwrap() {
                let path = entry.unwrap().path();
                if path.is_file() {
                    std::fs::copy(&path, dir.join(sub).join(path.file_name().unwrap())).unwrap();
                }
            }
        }
        Self(dir)
    }

    fn manifest(&self) -> Value {
        serde_json::from_slice(&std::fs::read(self.0.join("MANIFEST.json")).unwrap()).unwrap()
    }

    fn write_manifest(&self, manifest: &Value) {
        std::fs::write(self.0.join("MANIFEST.json"), manifest.to_string()).unwrap();
    }
}

impl Drop for Copy {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Planted controls: a wrong label fails the gate with exit 7 and names the case; a file that
/// changed under its recorded hash is refused with exit 4.
#[test]
fn the_summary_names_each_metric_and_each_failure() {
    let run = |dir: &Path| {
        let out = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
            .args(["eval", "--corpus", dir.to_str().unwrap()])
            .output()
            .expect("run");
        (out.status.code(), String::from_utf8(out.stdout).unwrap())
    };
    let (code, text) = run(&corpus());
    assert_eq!(code, Some(0), "{text}");
    assert!(
        text.starts_with("deep-diff-forge eval: 34 cases\n"),
        "{text}"
    );
    for line in [
        "  ranking_pairs        12/15",
        "  evidence_precision   21/22",
        "  evidence_recall      21/28",
        "  known misses         11",
        "  KNOWN ranking/auth-check-removed: src/handlers/delete.rs ranks below docs/CHANGES.md",
        "  KNOWN evidence/shell-heredoc: reports (\"scripts/docs.sh\", \"network_fetch\", \"new\", 3)",
    ] {
        assert!(text.lines().any(|l| l == line), "{line:?} in\n{text}");
    }
    assert!(!text.contains("FAIL"), "{text}");

    let wrong = Copy::new("summary");
    let mut manifest = wrong.manifest();
    manifest["cases"][0]["must_outrank"][0] = serde_json::json!(["docs/BIG.md", "src/util.rs"]);
    wrong.write_manifest(&manifest);
    let (code, text) = run(&wrong.0);
    assert_eq!(code, Some(7));
    assert!(
        text.lines()
            .any(|l| l == "  FAIL ranking/circle-probe: docs/BIG.md ranks below src/util.rs"),
        "{text}"
    );
}

#[test]
fn the_gate_fails_for_the_reason_it_names() {
    let wrong = Copy::new("wrong");
    let mut manifest = wrong.manifest();
    manifest["cases"][0]["must_outrank"][0] = serde_json::json!(["docs/BIG.md", "src/util.rs"]);
    wrong.write_manifest(&manifest);
    let (code, receipt, _) = eval(&wrong.0);
    assert_eq!(code, 7);
    assert_eq!(
        receipt["failures"],
        serde_json::json!(["ranking/circle-probe: docs/BIG.md ranks below src/util.rs"])
    );

    let stale = Copy::new("stale");
    std::fs::write(
        stale.0.join("ranking/circle-probe.patch"),
        "--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n",
    )
    .unwrap();
    let (code, _, stderr) = eval(&stale.0);
    assert_eq!(code, 4);
    assert!(
        stderr.contains("ranking/circle-probe.patch has SHA-256"),
        "{stderr}"
    );

    // Each ranking case fails when the signal it exists for is gone: the low side sits between
    // the high side's score with and without that signal, so a tie-break cannot rescue it.
    for (case, file, from, to) in [
        (
            "ranking/ci-privilege",
            "ranking/ci-privilege.patch",
            "pull_request_target",
            "push",
        ),
        (
            "ranking/generated-spawn",
            "ranking/generated-spawn.patch",
            "std::process::Command::new(\"sh\").spawn().ok();",
            "let x = 1;",
        ),
    ] {
        let blind = Copy::new(&case.replace('/', "-"));
        let target = blind.0.join(file);
        let patch = std::fs::read_to_string(&target).unwrap();
        assert!(patch.contains(from), "{case}");
        let patch = patch.replace(from, to);
        std::fs::write(&target, &patch).unwrap();
        let mut manifest = blind.manifest();
        for c in manifest["cases"].as_array_mut().unwrap() {
            if c["id"] == case {
                c["sha256"] = deep_diff_forge_core::sha256_hex(patch.as_bytes()).into();
            }
        }
        blind.write_manifest(&manifest);
        let (code, receipt, _) = eval(&blind.0);
        assert_eq!(code, 7, "{case} passed without its signal");
        let failures = receipt["failures"].as_array().unwrap();
        assert_eq!(failures.len(), 1, "{receipt:#}");
        assert!(
            failures[0].as_str().unwrap().starts_with(case),
            "{receipt:#}"
        );
    }

    // A file cannot outrank itself: a pair naming one path twice is a wrong label, not a pass.
    let selfish = Copy::new("selfish");
    let mut manifest = selfish.manifest();
    manifest["cases"][0]["must_outrank"][0] = serde_json::json!(["src/util.rs", "src/util.rs"]);
    selfish.write_manifest(&manifest);
    let (code, receipt, _) = eval(&selfish.0);
    assert_eq!(code, 7);
    assert_eq!(
        receipt["failures"],
        serde_json::json!(["ranking/circle-probe: src/util.rs ranks below src/util.rs"])
    );
    assert_eq!(
        receipt["metrics"]["ranking_pairs"]["right"], 11,
        "{receipt:#}"
    );

    let missing = Copy::new("missing");
    std::fs::remove_file(missing.0.join("findings/mixed.jsonl")).unwrap();
    assert_eq!(eval(&missing.0).0, 3);
}

/// The manifest of `copy` after `edit` changes the case `case`.
fn edit_case(copy: &Copy, case: &str, edit: impl FnOnce(&mut Value)) {
    let mut manifest = copy.manifest();
    let found = manifest["cases"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|c| c["id"] == case)
        .unwrap();
    edit(found);
    copy.write_manifest(&manifest);
}

/// The corpus is large enough to mean something, every kind has cases, and every case says
/// where it came from: a public identifier (CVE, GHSA, RUSTSEC), a real-world shape, one of this
/// project's own findings, or `synthetic` with the reason it exists.
#[test]
fn every_case_has_an_origin_and_every_kind_has_cases() {
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(corpus().join("MANIFEST.json")).unwrap()).unwrap();
    let cases = manifest["cases"].as_array().unwrap();
    assert!(cases.len() >= MIN_CASES, "{} cases", cases.len());
    for kind in ["ranking", "evidence", "findings"] {
        let n = cases.iter().filter(|c| c["kind"] == kind).count();
        assert!(n >= MIN_CASES_PER_KIND, "{n} {kind} cases");
    }
    for case in cases {
        let origin = case["origin"].as_str().unwrap();
        let named = ["CVE-", "GHSA-", "RUSTSEC-"]
            .iter()
            .any(|p| origin.starts_with(p))
            && origin.contains(" shape");
        let explained = ["real-world: ", "internal: ", "synthetic: "]
            .iter()
            .any(|p| origin.starts_with(p) && origin.len() > p.len() + 10);
        assert!(named || explained, "{}: origin {origin:?}", case["id"]);
    }
    // Hard negatives (a risky-looking file that must not outrank) and declared misses are both
    // present, so the corpus tests more than the rules' own idea of risk.
    let ids: Vec<&str> = cases.iter().map(|c| c["id"].as_str().unwrap()).collect();
    assert!(ids.iter().filter(|i| i.ends_with("-hard-negative")).count() >= 3);
    assert!(
        cases
            .iter()
            .any(|c| c["kind"] == "ranking" && c.get("known_miss").is_some())
    );
    assert!(
        cases
            .iter()
            .any(|c| c.get("known_false_positive").is_some())
    );
}

/// The corpus cannot drift: a file no case names, a case naming a file outside the corpus, a
/// repeated id, a missing or empty origin and a field the case's kind does not read are each
/// refused before anything is scored.
#[test]
fn the_manifest_cannot_drift_silently() {
    let refused = |copy: &Copy, code: i32, says: &str| {
        let (got, receipt, stderr) = eval(&copy.0);
        assert_eq!(got, code, "{stderr}\n{receipt:#}");
        assert!(stderr.contains(says), "{says:?} in {stderr}");
    };

    let orphan = Copy::new("orphan");
    std::fs::write(orphan.0.join("stray.patch"), "x").unwrap();
    refused(
        &orphan,
        4,
        "stray.patch is in the corpus, but no case names it",
    );
    let nested = Copy::new("nested");
    std::fs::create_dir_all(nested.0.join("evidence/old")).unwrap();
    std::fs::write(nested.0.join("evidence/old/x.patch"), "x").unwrap();
    refused(
        &nested,
        4,
        "evidence/old/x.patch is in the corpus, but no case names it",
    );

    let outside = Copy::new("outside");
    edit_case(&outside, "ranking/circle-probe", |c| {
        c["patch"] = "../ranking/circle-probe.patch".into();
    });
    std::fs::remove_file(outside.0.join("ranking/circle-probe.patch")).unwrap();
    refused(
        &outside,
        3,
        "../ranking/circle-probe.patch: not a file of the corpus",
    );
    let manifest = Copy::new("names-manifest");
    edit_case(&manifest, "ranking/circle-probe", |c| {
        c["patch"] = "MANIFEST.json".into();
    });
    std::fs::remove_file(manifest.0.join("ranking/circle-probe.patch")).unwrap();
    refused(&manifest, 3, "MANIFEST.json: not a file of the corpus");

    let twice = Copy::new("twice");
    edit_case(&twice, "ranking/trojan-source", |c| {
        c["id"] = "ranking/circle-probe".into();
    });
    refused(&twice, 4, "case ranking/circle-probe is listed twice");

    for (name, origin) in [("no-origin", None), ("blank-origin", Some("  "))] {
        let copy = Copy::new(name);
        edit_case(&copy, "findings/mixed", |c| match origin {
            None => {
                c.as_object_mut().unwrap().remove("origin");
            }
            Some(o) => c["origin"] = o.into(),
        });
        let says = if origin.is_some() {
            "case findings/mixed: origin is empty"
        } else {
            "case findings/mixed has no origin"
        };
        refused(&copy, 4, says);
    }

    let typo = Copy::new("typo");
    edit_case(&typo, "evidence/shell-heredoc", |c| {
        let object = c.as_object_mut().unwrap();
        let labels = object.remove("known_false_positive").unwrap();
        object.insert("known_false_positives".into(), labels);
    });
    refused(
        &typo,
        4,
        "case evidence/shell-heredoc: evidence cases have no field known_false_positives",
    );
    // A field another kind reads is still foreign to this one.
    let foreign = Copy::new("foreign");
    edit_case(&foreign, "ranking/circle-probe", |c| {
        c["known_false_positive"] = serde_json::json!([]);
    });
    refused(
        &foreign,
        4,
        "case ranking/circle-probe: ranking cases have no field known_false_positive",
    );
}

/// A declared miss fails the gate once the engine gets it right, and a declared false positive
/// fails once it is no longer reported or is also expected: the labels cannot outlive the limits
/// they describe.
#[test]
fn a_declared_miss_must_still_be_a_miss() {
    let failures = |copy: &Copy| {
        let (code, receipt, stderr) = eval(&copy.0);
        assert_eq!(code, 7, "{stderr}\n{receipt:#}");
        (
            receipt["failures"].clone(),
            receipt["metrics"].clone(),
            receipt["known_misses"].clone(),
        )
    };

    // Ranking: the pair is declared the right way round, so it now holds.
    let holds = Copy::new("holds");
    edit_case(&holds, "ranking/auth-check-removed", |c| {
        c["known_miss"] = serde_json::json!([["docs/CHANGES.md", "src/handlers/delete.rs"]]);
    });
    let (failed, metrics, known) = failures(&holds);
    assert_eq!(
        failed,
        serde_json::json!([
            "ranking/auth-check-removed: the known miss docs/CHANGES.md above src/handlers/delete.rs now holds: move it to must_outrank"
        ])
    );
    assert_eq!(
        metrics["ranking_pairs"],
        serde_json::json!({"right": 13, "total": 15, "ppm": 866_666})
    );
    assert_eq!(known, KNOWN_MISSES - 1);

    // Evidence: an item the engine finds cannot be declared missed.
    let found = Copy::new("found");
    edit_case(&found, "evidence/bash-dev-tcp", |c| {
        c["expect"] = serde_json::json!([]);
        c["known_miss"] = serde_json::json!([
            {"path": "scripts/debug.sh", "signal": "network_fetch", "side": "new", "line": 3}
        ]);
    });
    let (failed, metrics, _) = failures(&found);
    assert!(
        failed[1]
            .as_str()
            .unwrap()
            .ends_with("is now found: move it to expect"),
        "{failed:#}"
    );
    assert_eq!(
        metrics["evidence_recall"],
        serde_json::json!({"right": 21, "total": 27, "ppm": 777_777})
    );

    // A declared false positive the engine does not report.
    let gone = Copy::new("gone");
    edit_case(&gone, "evidence/shell-heredoc", |c| {
        c["known_false_positive"][0]["line"] = 2.into();
    });
    let (failed, metrics, _) = failures(&gone);
    assert_eq!(
        failed,
        serde_json::json!([
            "evidence/shell-heredoc: unexpected evidence (\"scripts/docs.sh\", \"network_fetch\", \"new\", 3)",
            "evidence/shell-heredoc: a known false positive (\"scripts/docs.sh\", \"network_fetch\", \"new\", 2) is not reported, or is expected: remove it"
        ])
    );
    assert_eq!(
        metrics["evidence_precision"],
        serde_json::json!({"right": 21, "total": 22, "ppm": 954_545})
    );

    // A declared false positive that is also expected.
    let both = Copy::new("both");
    edit_case(&both, "evidence/shell-heredoc", |c| {
        c["expect"] = c["known_false_positive"].clone();
    });
    let (failed, metrics, known) = failures(&both);
    assert_eq!(failed.as_array().unwrap().len(), 1, "{failed:#}");
    assert!(
        failed[0]
            .as_str()
            .unwrap()
            .ends_with("is not reported, or is expected: remove it")
    );
    assert_eq!(metrics["evidence_precision"]["right"], 22);
    assert_eq!(known, KNOWN_MISSES - 1);
}
