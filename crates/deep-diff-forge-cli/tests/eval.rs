//! The evaluation gate (plan 2026-10-30, M3): the committed corpus (`fixtures/eval`) must score at
//! or above these floors. Every floor is 100%: `eval` counts each wrong order, false positive,
//! miss and wrong verdict as a failure (exit 7), so there is no tolerance to spend. A limit the
//! engine accepts becomes a `known_miss` label in the manifest, reviewed like any other change.

#![expect(
    clippy::disallowed_methods,
    reason = "integration test: drives the real binary as a child process"
)]

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Parts per million: 1,000,000 is every case right.
const RANKING_PAIRS_FLOOR: u64 = 1_000_000;
const EVIDENCE_PRECISION_FLOOR: u64 = 1_000_000;
const EVIDENCE_RECALL_FLOOR: u64 = 1_000_000;
const FINDINGS_FLOOR: u64 = 1_000_000;
/// Documented limits (plan N4): reported, not counted against recall.
const KNOWN_MISSES: u64 = 1;

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
    assert_eq!(receipt["failures"], serde_json::json!([]));
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

    let missing = Copy::new("missing");
    std::fs::remove_file(missing.0.join("findings/mixed.jsonl")).unwrap();
    assert_eq!(eval(&missing.0).0, 3);
}
