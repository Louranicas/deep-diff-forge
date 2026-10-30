// DDF integration: the evaluation corpus and its metrics (plan 2026-10-30, M3 evaluation).
// Design/status: docs/PLAN_AGENT_EVIDENCE_AND_OBSERVABILITY.md#system-m-monitoring-evaluation-and-benchmarking
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fcrates--deep-diff-forge-cli--src--eval
//! `deep-diff-forge eval`: score the engine against a labelled corpus and print a receipt
//! (`deep-diff-forge.eval.v0`). The corpus is `fixtures/eval/MANIFEST.json`, which lists every
//! case with its file's SHA-256, its origin and its labels:
//!
//! - **ranking** cases: pairs of paths, the first of which must rank above the second;
//! - **evidence** cases: the evidence items expected, and `known_miss` items, which are documented
//!   limits that are reported but not counted against recall. Any other item is a false positive.
//! - **findings** cases: the verdict expected for each finding, in order.
//!
//! A case whose file does not match its SHA-256 is refused, so the corpus cannot drift silently.
//! `cli/tests/eval.rs` holds the floors.

use serde_json::{Value, json};
use std::path::Path;

/// Why the corpus could not be scored.
#[derive(Debug)]
pub enum EvalError {
    /// A file is missing or unreadable.
    Read(String),
    /// The manifest or a case is malformed, or a file does not match its recorded SHA-256.
    Corpus(String),
}

impl std::fmt::Display for EvalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Read(m) | Self::Corpus(m) => f.write_str(m),
        }
    }
}

/// A case's id, for messages (`?` when it has none).
fn id(case: &Value) -> &str {
    case["id"].as_str().unwrap_or("?")
}

fn read(dir: &Path, rel: &str) -> Result<Vec<u8>, EvalError> {
    std::fs::read(dir.join(rel)).map_err(|e| EvalError::Read(format!("{rel}: {e}")))
}

fn field<'v>(case: &'v Value, key: &str) -> Result<&'v Value, EvalError> {
    case.get(key)
        .ok_or_else(|| EvalError::Corpus(format!("case {} has no {key}", id(case))))
}

fn text<'v>(case: &'v Value, key: &str) -> Result<&'v str, EvalError> {
    field(case, key)?
        .as_str()
        .ok_or_else(|| EvalError::Corpus(format!("case {}: {key} is not a string", id(case))))
}

/// A file a case names, checked against the SHA-256 the manifest records for it.
fn checked(dir: &Path, case: &Value, file_key: &str, sha_key: &str) -> Result<Vec<u8>, EvalError> {
    let rel = text(case, file_key)?;
    let bytes = read(dir, rel)?;
    let expected = text(case, sha_key)?;
    let actual = deep_diff_forge_core::sha256_hex(&bytes);
    if actual != expected {
        return Err(EvalError::Corpus(format!(
            "{rel} has SHA-256 {actual}, but the manifest records {expected}: update the case's labels and its hash together"
        )));
    }
    Ok(bytes)
}

/// Running totals for one metric: how many were right out of how many.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Tally {
    pub right: u64,
    pub total: u64,
}

impl Tally {
    fn add(&mut self, right: bool) {
        self.total += 1;
        if right {
            self.right += 1;
        }
    }

    /// `right / total` in parts per million (1,000,000 is all right; an empty tally is all
    /// right, as nothing was missed).
    #[must_use]
    pub fn ppm(self) -> u64 {
        (self.right * 1_000_000)
            .checked_div(self.total)
            .unwrap_or(1_000_000)
    }

    fn json(self) -> Value {
        json!({"right": self.right, "total": self.total, "ppm": self.ppm()})
    }
}

/// The scores and the failures behind them.
#[derive(Debug, Default)]
pub struct Scores {
    pub ranking_pairs: Tally,
    /// Reported evidence items that were expected, out of all reported items.
    pub evidence_precision: Tally,
    /// Expected evidence items that were reported, out of all expected items.
    pub evidence_recall: Tally,
    pub findings_verdicts: Tally,
    pub known_misses: u64,
    pub failures: Vec<String>,
}

fn sealed(bytes: &[u8]) -> Result<deep_diff_forge_patch::SealedReview, EvalError> {
    deep_diff_forge_patch::SealedReview::new(
        bytes,
        deep_diff_forge_patch::InputGuards {
            files: false,
            hunks: false,
        },
        env!("CARGO_PKG_VERSION"),
    )
    .map_err(|e| EvalError::Corpus(format!("a case patch does not parse: {e}")))
}

fn ranking(dir: &Path, case: &Value, scores: &mut Scores) -> Result<(), EvalError> {
    let review = sealed(&checked(dir, case, "patch", "sha256")?)?;
    let order: Vec<String> = deep_diff_forge_graph::rank(review.files())
        .into_iter()
        .map(|r| r.path)
        .collect();
    let pairs = field(case, "must_outrank")?.as_array().ok_or_else(|| {
        EvalError::Corpus(format!("case {}: must_outrank is not a list", id(case)))
    })?;
    for pair in pairs {
        let (Some(high), Some(low)) = (pair[0].as_str(), pair[1].as_str()) else {
            return Err(EvalError::Corpus(format!(
                "case {}: a pair is not two paths",
                id(case)
            )));
        };
        let at = |p: &str| order.iter().position(|o| o == p);
        let (Some(h), Some(l)) = (at(high), at(low)) else {
            return Err(EvalError::Corpus(format!(
                "case {}: {high} or {low} is not in the patch",
                id(case)
            )));
        };
        scores.ranking_pairs.add(h < l);
        if h >= l {
            scores
                .failures
                .push(format!("{}: {high} ranks below {low}", id(case)));
        }
    }
    Ok(())
}

/// One evidence item as `(path, signal, side, line)`.
fn item(value: &Value) -> Option<(String, String, String, u64)> {
    Some((
        value["path"].as_str()?.to_owned(),
        value["signal"].as_str()?.to_owned(),
        value["side"].as_str()?.to_owned(),
        value["line"].as_u64()?,
    ))
}

fn items(case: &Value, key: &str) -> Result<Vec<(String, String, String, u64)>, EvalError> {
    match case.get(key) {
        None => Ok(Vec::new()),
        Some(Value::Array(list)) => list
            .iter()
            .map(|v| {
                item(v).ok_or_else(|| {
                    EvalError::Corpus(format!("case {}: a {key} item is malformed", id(case)))
                })
            })
            .collect(),
        Some(_) => Err(EvalError::Corpus(format!(
            "case {}: {key} is not a list",
            id(case)
        ))),
    }
}

fn evidence(dir: &Path, case: &Value, scores: &mut Scores) -> Result<(), EvalError> {
    let review = sealed(&checked(dir, case, "patch", "sha256")?)?;
    let expected = items(case, "expect")?;
    let known = items(case, "known_miss")?;
    let reported: Vec<(String, String, String, u64)> = deep_diff_forge_graph::rank(review.files())
        .iter()
        .flat_map(|r| {
            r.evidence.iter().map(|e| {
                (
                    r.path.clone(),
                    e.signal.label().to_owned(),
                    e.side.label().to_owned(),
                    u64::from(e.line),
                )
            })
        })
        .collect();
    for got in &reported {
        let right = expected.contains(got);
        scores.evidence_precision.add(right);
        if !right {
            scores
                .failures
                .push(format!("{}: unexpected evidence {got:?}", id(case)));
        }
    }
    for want in &expected {
        let found = reported.contains(want);
        scores.evidence_recall.add(found);
        if !found {
            scores
                .failures
                .push(format!("{}: missed evidence {want:?}", id(case)));
        }
    }
    for miss in &known {
        if reported.contains(miss) {
            scores.failures.push(format!(
                "{}: a known miss {miss:?} is now found: move it to expect",
                id(case)
            ));
        } else {
            scores.known_misses += 1;
        }
    }
    Ok(())
}

fn findings(dir: &Path, case: &Value, scores: &mut Scores) -> Result<(), EvalError> {
    let review = sealed(&checked(dir, case, "patch", "sha256")?)?;
    let raw = checked(dir, case, "findings", "findings_sha256")?;
    let raw = String::from_utf8(raw)
        .map_err(|_| EvalError::Corpus(format!("case {}: findings are not UTF-8", id(case))))?;
    let verdict = deep_diff_forge_agent::findings::verify(
        review.files(),
        &review.seal().input_sha256,
        raw.lines(),
    );
    let mut got: Vec<(usize, &str)> = verdict
        .accepted
        .iter()
        .map(|a| (a.index, "accepted"))
        .chain(verdict.refused.iter().map(|r| (r.index, r.reason.label())))
        .collect();
    got.sort_unstable();
    let expected = field(case, "expect")?
        .as_array()
        .ok_or_else(|| EvalError::Corpus(format!("case {}: expect is not a list", id(case))))?;
    for (i, want) in expected.iter().enumerate() {
        let actual = got.get(i).map(|(_, v)| *v);
        let right = actual == want.as_str();
        scores.findings_verdicts.add(right);
        if !right {
            scores.failures.push(format!(
                "{}: finding {i} is {actual:?}, not {want}",
                id(case)
            ));
        }
    }
    if got.len() != expected.len() {
        scores.failures.push(format!(
            "{}: {} verdicts for {} expected",
            id(case),
            got.len(),
            expected.len()
        ));
        scores.findings_verdicts.add(false);
    }
    Ok(())
}

/// Score the corpus in `dir` and return the `deep-diff-forge.eval.v0` receipt.
///
/// # Errors
/// [`EvalError`] when a file is missing or the corpus is malformed or out of date.
pub fn run(dir: &Path) -> Result<Value, EvalError> {
    let manifest_bytes = read(dir, "MANIFEST.json")?;
    let manifest: Value = serde_json::from_slice(&manifest_bytes)
        .map_err(|e| EvalError::Corpus(format!("MANIFEST.json: {e}")))?;
    if manifest["schema"] != "deep-diff-forge.eval-manifest.v0" {
        return Err(EvalError::Corpus(
            "MANIFEST.json is not deep-diff-forge.eval-manifest.v0".into(),
        ));
    }
    let cases = manifest["cases"]
        .as_array()
        .ok_or_else(|| EvalError::Corpus("MANIFEST.json has no cases".into()))?;
    let mut scores = Scores::default();
    for case in cases {
        match text(case, "kind")? {
            "ranking" => ranking(dir, case, &mut scores)?,
            "evidence" => evidence(dir, case, &mut scores)?,
            "findings" => findings(dir, case, &mut scores)?,
            other => {
                return Err(EvalError::Corpus(format!(
                    "case {}: unknown kind {other}",
                    id(case)
                )));
            }
        }
    }
    Ok(json!({
        "schema": "deep-diff-forge.eval.v0",
        "version": env!("CARGO_PKG_VERSION"),
        "corpus_sha256": deep_diff_forge_core::sha256_hex(&manifest_bytes),
        "cases": cases.len(),
        "metrics": {
            "ranking_pairs": scores.ranking_pairs.json(),
            "evidence_precision": scores.evidence_precision.json(),
            "evidence_recall": scores.evidence_recall.json(),
            "findings_verdicts": scores.findings_verdicts.json(),
        },
        "known_misses": scores.known_misses,
        "failures": scores.failures,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tally_is_parts_per_million() {
        let mut t = Tally::default();
        assert_eq!(t.ppm(), 1_000_000, "nothing to miss is all right");
        t.add(true);
        t.add(false);
        t.add(true);
        assert_eq!((t.right, t.total, t.ppm()), (2, 3, 666_666));
        assert_eq!(t.json(), json!({"right": 2, "total": 3, "ppm": 666_666}));
    }

    #[test]
    fn an_evidence_item_needs_all_four_fields() {
        assert_eq!(
            item(&json!({"path": "a", "signal": "s", "side": "new", "line": 3})),
            Some(("a".into(), "s".into(), "new".into(), 3))
        );
        assert_eq!(
            item(&json!({"path": "a", "signal": "s", "side": "new"})),
            None
        );
    }
}
