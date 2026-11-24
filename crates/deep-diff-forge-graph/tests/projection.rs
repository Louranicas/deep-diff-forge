//! F1.a2 (contract design section 3.3, "Completeness"): `deep-diff-forge.diagnostics.v0`
//! projects exactly onto the ranking. Over the patches of this repository's `fixtures/`, read as
//! `--stdin-patch` reads them, the (path, label) pairs of the document are those of `signals[]`,
//! and its rows, in order, are `evidence[]`; every label of the vocabulary occurs, but those
//! named in `NOT_IN_FIXTURES` with the reason. The whole comparison corpus is outside the
//! repository: `projection_over_a_listed_corpus` (ignored) writes the document of each patch of
//! a list, for a script to hold against the binary's `--rank --json`.

use deep_diff_forge_graph::{
    Diagnostics, RankedFile, RiskSignal, Side, diagnostics, rank, vocabulary,
};
use deep_diff_forge_patch::{InputGuards, SealedReview};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Labels no patch of `fixtures/` raises on stdin, each with why. On the commit of F1.a2, 74
/// fixtures rank, with 227 evidence rows; the test refuses fewer.
const NOT_IN_FIXTURES: &[(&str, &str)] = &[(
    "diff_suppressed",
    "raised only when the Git acquisition reads a file Git gave as binary; a stdin patch never",
)];

type Row = (String, &'static str, Side, u32, bool);

/// The document of `input` as `--stdin-patch` reads it, and its ranking; `None` when the input
/// is not a patch.
fn review(input: &[u8]) -> Option<(Vec<RankedFile>, Diagnostics)> {
    let none = InputGuards {
        files: false,
        hunks: false,
    };
    let review = SealedReview::new(input, none, "test").ok()?;
    let ranked = rank(review.files());
    let doc = diagnostics(review.seal(), &ranked);
    Some((ranked, doc))
}

/// Where the document and the ranking differ, or `None`: the (path, label) sets, then the rows
/// (path, label, side, line, reading) in order, then the hazardous lines each file holds.
fn projection_differs(ranked: &[RankedFile], doc: &Diagnostics) -> Option<String> {
    let of_doc: BTreeSet<(&String, &str)> = doc
        .diagnostics
        .iter()
        .map(|d| (&d.path, d.signal.label()))
        .collect();
    let of_rank = ranked
        .iter()
        .flat_map(|f| f.signals.iter().map(|s| (&f.path, s.label())));
    let of_rank: BTreeSet<(&String, &str)> = of_rank.collect();
    if of_doc != of_rank {
        return Some(format!("pairs: document {of_doc:?}, signals[] {of_rank:?}"));
    }
    let rows: Vec<Row> = doc
        .diagnostics
        .iter()
        .filter(|d| d.signal != RiskSignal::HazardousText)
        .filter_map(|d| {
            let (side, line) = d.at?;
            Some((d.path.clone(), d.signal.label(), side, line, d.current))
        })
        .collect();
    let evidence: Vec<Row> = ranked
        .iter()
        .flat_map(|f| f.evidence.iter().map(move |e| (f, e)))
        .map(|(f, e)| (f.path.clone(), e.signal.label(), e.side, e.line, e.read_now))
        .collect();
    if rows != evidence {
        return Some(format!("rows: document {rows:?}, evidence[] {evidence:?}"));
    }
    let hazards = |d: &&deep_diff_forge_graph::Diagnostic| d.code == "hazardous_text@line";
    let located: BTreeSet<(&String, Side, u32)> = doc
        .diagnostics
        .iter()
        .filter(hazards)
        .filter_map(|d| d.at.map(|(s, n)| (&d.path, s, n)))
        .collect();
    let held: BTreeSet<_> = ranked
        .iter()
        .flat_map(|f| f.hazard_lines.iter().map(move |&(s, n)| (&f.path, s, n)))
        .collect();
    (!located.is_subset(&held)).then(|| format!("hazardous lines {located:?} not in {held:?}"))
}

fn patches(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            patches(&path, out);
        } else if path
            .extension()
            .is_some_and(|x| x == "patch" || x == "diff")
        {
            out.push(path);
        }
    }
}

#[test]
fn projection_equals_rank_over_signals_and_evidence() {
    let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures"));
    let mut found = Vec::new();
    patches(root, &mut found);
    let (mut ranked_patches, mut rows, mut labels) = (0, 0, BTreeSet::new());
    for path in &found {
        let Some((ranked, doc)) = review(&std::fs::read(path).unwrap()) else {
            continue;
        };
        if let Some(why) = projection_differs(&ranked, &doc) {
            panic!("{}: {why}", path.display());
        }
        ranked_patches += 1;
        rows += ranked.iter().map(|f| f.evidence.len()).sum::<usize>();
        labels.extend(doc.diagnostics.iter().map(|d| d.signal.label()));
    }
    let unseen: Vec<&str> = vocabulary()
        .iter()
        .map(|f| f.signal.label())
        .filter(|l| !labels.contains(l))
        .collect();
    let named: Vec<&str> = NOT_IN_FIXTURES.iter().map(|(l, _)| *l).collect();
    assert_eq!(unseen, named, "labels no fixture raises, and those named");
    assert!(
        ranked_patches >= 74 && rows >= 227,
        "{ranked_patches} patches, {rows} rows"
    );
}

/// The list and the output directory of the corpus run, from the environment.
#[expect(
    clippy::disallowed_methods,
    reason = "the hand-run corpus test is given its list and output directory"
)]
fn corpus_run() -> (String, PathBuf) {
    let list = std::env::var("DDF_PROJECTION_LIST").expect("DDF_PROJECTION_LIST");
    let out = std::env::var("DDF_PROJECTION_OUT").expect("DDF_PROJECTION_OUT");
    (list, PathBuf::from(out))
}

/// `DDF_PROJECTION_LIST` (one patch path a line) and `DDF_PROJECTION_OUT` (a directory): for the
/// patch on line `i` that reads as a patch, `<out>/<i>.json` holds its document, after the
/// projection onto the in-process ranking held. Prints the figures.
#[test]
#[ignore = "a corpus outside the repository; run by hand"]
fn projection_over_a_listed_corpus() {
    let (list, out) = corpus_run();
    let (mut documents, mut refused, mut labels) = (0, 0, BTreeSet::new());
    for (i, path) in std::fs::read_to_string(list).unwrap().lines().enumerate() {
        let Some((ranked, doc)) = review(&std::fs::read(path).unwrap()) else {
            refused += 1;
            continue;
        };
        if let Some(why) = projection_differs(&ranked, &doc) {
            panic!("{path}: {why}");
        }
        std::fs::write(out.join(format!("{i}.json")), doc.to_json()).unwrap();
        documents += 1;
        labels.extend(doc.diagnostics.iter().map(|d| d.code.clone()));
    }
    println!("documents {documents} not a patch {refused} codes seen {labels:?}");
}
