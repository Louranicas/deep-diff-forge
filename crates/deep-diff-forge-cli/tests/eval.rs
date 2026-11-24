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

use deep_diff_forge_testkit::Scratch;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Parts per million: 1,000,000 is every case right. Measured 2026-10-31 on 34 cases: ranking
/// 12 of 15 pairs, precision 21 of 22 items, recall 21 of 28 items, findings 33 of 33 verdicts.
/// Measured again 2026-11-14 on 35 cases, when the code view was made to read Rust as the
/// compiler's lexer does: precision 28 of 29, recall 28 of 35, the rest unchanged. Both floors
/// rose. The new case, `evidence/rust-code-view-20261114`, has seven expected items and
/// declares nothing: what a nested block comment and a character literal with a long escape
/// hid from `process_spawn`, `network_fetch` and `unsafe_added` (the published v0.8.3 binary
/// misses four of the seven).
/// Measured 2026-11-13 on 40 cases: precision 44 of 46 items, recall 44 of 71 items; ranking and
/// findings as before. The recall floor fell from 33 of 41 on purpose: the cases
/// `evidence/agent-evasions` (twelve ways an agent makes a gate pass without doing the work) and
/// `evidence/review-v3-round1` (what an adversarial review found; seven not yet caught) add
/// nineteen declared misses (`evidence/review-v3-round2` adds one declared false positive), and a declared miss counts as wrong. Each one that a rule later
/// catches fails the run until it is moved to `expect`, and the floor rises again.
/// Measured again 2026-11-13 on 41 cases, after round 3 of that review: precision 50 of 53,
/// recall 50 of 77. The precision floor fell from 44 of 46 on purpose: a test attribute is no
/// longer proof of test code, so a panic path in a `#[test]` function is reported, and the two
/// such items in the corpus are declared false positives.
/// Measured 2026-11-13 on 42 cases, after round 4: precision 53 of 56, recall 53 of 81.
/// Measured 2026-11-14 on 43 cases, after an independent review: precision 63 of 69, recall 63
/// of 104. Both floors fell on purpose: the new case declares three false positives (a method
/// named `expect` that is not `Result`'s, twice, and a Python name that begins with
/// `unittest.skip`) and thirteen misses, most of them respellings in Python, JavaScript and Go,
/// where no parser backs the token rules.
/// Measured again 2026-11-14 on 44 cases, after round 8 (raw identifiers): precision 69 of 75,
/// recall 69 of 110.
/// Measured a third time 2026-11-14 on 45 cases, after round 9 and a second independent review:
/// precision 77 of 90, recall 77 of 119. The precision floor fell from 69 of 75 on purpose. The
/// parser no longer withdraws a panic path as "proved test code" (the proof was broken four
/// more ways, and the class has no end), so the five such items in the corpus are reported,
/// marked `test_likely`, and are declared false positives, as is the one in the new case; and
/// an attribute that starts on an unchanged line is the same as before only if its whole text
/// is, so the conditional skip of which only the reason changed is reported and declared too.
/// The recall floor rose: the new case has eight expected items and one declared miss.
/// Measured a fourth time 2026-11-14 on 46 cases, after round 10 and a third independent
/// review, when the parser was taken out of the evidence path and the tokens of a Rust hunk
/// are read across its lines: precision 81 of 95, recall 81 of 125. The signal
/// `syntax_unchecked` is gone, and with it two expectations; the three constructs it marked
/// inside a macro's body (declared misses until then) are found. The new case,
/// `evidence/independent-review-3-20261114`, has five expected items and three declared
/// misses, all one limit: a change that adds no token to the construct. Two items the parser
/// found are declared misses now (a function of the patch's own type passed as a value, and a
/// call after `/* /* */ // */`, where the code view does not nest the comment), and one
/// negative is a declared false positive (a conditional skip of which only the spacing of
/// the condition changed: nothing is compared with the old side any more). So the precision
/// floor fell from 77 of 90 on purpose, and the recall floor rose a little.
/// Measured a fifth time 2026-11-14 on 47 cases, after round 12, when the code view was made
/// to read Rust as the compiler's lexer does: precision 97 of 111, recall 97 of 139. Both
/// floors rose. The two calls after `/* /* */ // */` went from declared misses to found, and
/// the new case, `evidence/rust-code-view-20261114`, has fourteen expected items and declares
/// nothing: what a nested block comment and a character literal with a long escape hid from
/// the released rules (`process_spawn`, `network_fetch`, `unsafe_added`) and from the
/// integrity signals.
/// Measured a sixth time 2026-11-14 on 48 cases, after round 13, when a Rust hunk became a
/// set of whole views whose findings are united: precision 116 of 132, recall 116 of 162. The
/// new case, `evidence/independent-review-5-20261114`, has nineteen expected items (what a
/// comment between two tokens hid after a string over lines, raw identifiers after white
/// space past ASCII, a line inside an existing block comment, a shebang line, `k#r"\"`, and
/// the line that closes a string opened above its hunk), two declared false positives (the
/// cost of reading a hunk also as beginning inside a comment, and inside a string) and four
/// declared misses (a text that only editions before 2021 read as the compiler does, and a
/// string opened by an earlier hunk). Both floors rose.
/// Measured a seventh time 2026-11-14 on 49 cases, after round 14, when nothing about a
/// hunk's place or its start was taken on the patch writer's word any more: precision 134 of
/// 153, recall 134 of 187. The new case, `evidence/independent-review-6-20261114`, has
/// seventeen expected items (hunks whose header says `+1`, a file created as several hunks,
/// two hunks of added lines that are read together and each on its own, a hunk between two
/// strings over lines, a string whose text opens more comments than it closes, and the
/// full-context forms of two constructions that a bare fragment cannot show), three
/// declared false positives and eight declared misses. One declared miss of the case before
/// it is found (a spawn in a text only editions before 2021 read as the compiler does: the
/// reading of release 0.8.3, which every Rust hunk now also gets, reads it that way). The
/// precision floor fell a little (it was 878,787), on purpose: two of the three new false
/// positives are items release 0.8.3 reports in the text of a string, and that reading is
/// kept so that no item of that release is lost. The recall floor rose a little (it was
/// 716,049), though four of the eight new misses are spellings of a suppressed lint that no
/// rule reads yet.
/// Measured an eighth time 2026-11-15 on 50 cases, after round 15, when no number of a hunk
/// header decided what is found any more and the edge of a hunk side was read: precision 158
/// of 179, recall 158 of 213. The new case, `evidence/independent-review-7-20261115`, has
/// twenty expected items (a script under false header numbers, a raw string whose text holds
/// `"#`, hunks joined on the hint of either number, a call cut by hunks that nothing joins,
/// the released signals' paths with white space and line ends in them, a second section for
/// one path), five declared misses and two declared false positives. Three declared misses
/// of the case before it are found (the call cut by two hunks that touch behind a false
/// position, and two spellings of a suppressed lint) and one item of the first independent
/// review's case is new (an inner `#![allow(…)]` written as `#!`, a comment and the rest).
/// Both floors rose.
/// Measured a ninth time 2026-11-15 on the same 50 cases, after round 16 narrowed what the
/// end of a hunk side is read for and stopped reading its beginning (a return type named
/// `Command` at the end of a line had been a spawn): precision 156 of 176, recall 156 of
/// 215. The seventh review's case gained two expected items (a stub cut after `todo!`, a
/// spawn cut after `Command::`) and nine hard negatives, lost one declared false positive (a
/// call of a function named `expect` is no longer reported), and three of its items are
/// declared misses again, with one of the sixth review's (a call cut after `.unwrap` by
/// hunks that nothing joins, and its forms). The precision floor rose; the recall floor
/// fell, on purpose: `.unwrap` at the end of a side is the end of an honest line too.
/// Measured a tenth time 2026-11-15 on 51 cases, in the same round, after the eighth
/// independent review: precision 163 of 183, recall 163 of 230. The new case,
/// `evidence/independent-review-8-20261115`, has seven expected items (an assertion in two
/// hunks that touch, beside a hunk of added lines that names their place, three ways; the
/// released paths as `<Command>::new` and `Command::<>::new`), eight declared misses (a hunk
/// with old lines that falsely claims the place; a call cut twice; a function value cut
/// before its name; a hunk an applier puts inside another; a script whose hunk does not
/// show its first line; a space before a call's bracket) and four hard negatives. The
/// precision floor rose; the recall floor fell by the eight misses that are now written
/// down.
/// Measured an eleventh time 2026-11-15 on 52 cases, after round 17 put the end of a hunk
/// side under one rule (a form is read there only where every continuation the grammar
/// allows is the construct: a method's name with `::` after it, and nothing else):
/// precision 166 of 188, recall 166 of 244. The new case,
/// `evidence/independent-review-9-20261115`, has five expected items (the released paths
/// with a qualified name in angle brackets; a call cut after `.unwrap::`), nine declared
/// misses (a conditional skip cut after its `ignore`; four spellings of the released
/// paths; hunks at old position 0 that `git apply --unidiff-zero` stacks; the mirror of the
/// declared decoy), two declared false positives (a join of two `-U0` hunks that stand far
/// apart) and six hard negatives, each an honest end of a hunk that round 16 reported. Two
/// expected items of the seventh review's case (a stub cut after `todo!`, a spawn cut after
/// `Command::`) are declared misses. Both floors fell, on purpose: what was taken out was
/// read on an argument from a sample, and the declared misses are now written down.
/// Measured a twelfth time 2026-11-16 on 53 cases, after round 18 stopped reading any form
/// at the end of a hunk side (a hunk side is read as text that ends there; nothing is
/// inferred from how it ends), read `#[ignore(…)]` as a skip and a bracketed released path
/// after a keyword that begins an expression: precision 171 of 194, recall 171 of 252. The
/// new case, `evidence/independent-review-10-20261116`, has six expected items
/// (`#[ignore(…)]` and `ignore(…)` in a `cfg_attr` list; `<Command>::new` after `return`,
/// `match`, `for _ in` and `break 'l`), two declared misses (the bare `ignore` an ordinary
/// `-U0` hunk of rustfmt output ends on; a created file in two hunks at old position 0), one
/// declared false positive (the same two hunks the other way round) and three hard
/// negatives (`.unwrap::` and `.expect::<` in a macro that takes any tokens; `0..unwrap::`).
/// The ninth review's call cut after `.unwrap::` is a declared miss. Both floors fell, on
/// purpose: what was taken out was read at an edge, and grammar broke it.
/// Measured a thirteenth time 2026-11-16 on 54 cases, after round 19 read a skip only where
/// the side holds its attribute whole and the keywords before a bracketed path became those
/// rustc 1.99.1 compiles there: precision 177 of 201, recall 177 of 259. The new case,
/// `evidence/independent-review-11-20261116`, has six expected items (`<Command>::new` after
/// `&mut` in two patches of two lines each, after `become` and after `do yeet`), one declared
/// miss (`ignore = "…"` alone above an unchanged `)]`, rustfmt's layout, at the end of a
/// `-U0` hunk), one declared false positive (a side that begins at `<Command>::new();` under
/// an unseen `Vec::`) and two hard negatives (Vesper's `#[ignore(` and
/// `#[cfg_attr(all(), ignore(` at the end of a side). The precision floor fell, on purpose:
/// the false item is declared, not new to this round.
/// Measured a fourteenth time 2026-11-17 on 55 cases, after the closing diff of round 19
/// declared what its two reviews found (no code changed): precision 177 of 203, recall 177 of
/// 260. The new case, `evidence/independent-review-12-20261117`, has one declared miss
/// (`ignore = "…"` above `)]` where plain `git diff` ends its hunk: the miss is not `-U0`
/// only) and two declared false positives, both a writer's (a seventeenth nested `cfg_attr`
/// list with no `ignore`, taken to hold a skip; `mut <Command>::new` in a discarding macro).
/// Both floors fell, on purpose: what was found is declared.
/// Measured 2026-11-18 on the integrated tree (release 0.8.4 with the integrity signals, the
/// reading of 0.8.4 united with theirs, and the agent hooks): the same 55 cases and the same
/// figures, precision 177 of 203, recall 177 of 260. The 0.8.4 case
/// `evidence/rust-code-view-20261114` is the signals line's, with fourteen expected items.
/// Measured again the same day, when the sections of one path that Git applies in order were
/// also read as one file: precision 179 of 205, recall 179 of 260. The seventh review's two
/// items of a second section for one path (`src/n.rs`, `src/two.rs`, IR7-H-2) went from
/// declared misses to expected, so both floors rose.
/// Measured again 2026-11-18 on 56 cases, after the integrated review of 0.9.0 (no code that
/// finds an item changed): precision 179 of 207, recall 179 of 260. The new case,
/// `evidence/integrated-review-1-20261118`, has two declared false positives (CI1), both of
/// reading the sections of one path as one file: an honest `git diff -U0` series whose second
/// section's old number is read in the coordinates of one file, and a created file followed by
/// a section that adds a line above its context. The precision floor fell, on purpose: what was
/// found is declared, not new to this round.
/// Measured again 2026-11-18 for release 0.10.0, whose parser refuses a hunk that plain `git
/// apply` places elsewhere than its header says (ruling 1 of the integration): 29 sections of
/// nine evidence cases gained a blank line of context, with their items unchanged; 18 sections
/// (creations in several hunks, hunks that touch, and readings only such hunks give) moved with
/// their 16 expectations (9 expected, 4 declared misses, 3 declared false positives) to
/// `fixtures/eval-refused-by-parser/`: precision 170 of 195, recall 170 of 247, the same 56
/// cases. The recall floor fell by what left, not by a reading that changed.
/// Measured again 2026-11-22 on 58 cases (S3.18, S3.16): `evidence/workflow-privilege-quoted`
/// (four grants written with quotes, which 0.9.0 missed, and three near misses) and
/// `evidence/python-trivial-assert-ends` (four ended `assert True` forms; the seventeen rows that
/// 0.9.0 labelled though they can fail, as hard negatives; `assert True or ready` a declared
/// miss): precision 178 of 203, recall 178 of 256, 106 declared. Both floors rose.
/// Measured again 2026-11-23 on 59 cases (S3.24): `evidence/secret-forge-tokens` (eight token
/// forms that releases 0.5.0 to 0.9.0 missed, six GitLab prefixes, the routable GitLab form and a
/// fine-grained GitHub token, and five near misses): precision 186 of 211, recall 186 of 264,
/// 106 declared. Both floors rose.
/// Since 0.11.0 (F2b) `#[expect(..)]` is a suppression: line 8 of `src/allow.rs` leaves the
/// declared misses for the expected items, and line 5 of `evidence/rust-negatives` is expected:
/// precision 188 of 213, recall 188 of 265, 105 declared. The floors stay.
/// F2b.c reads `cfg_attr(all(), allow(...))`: line 6 of `src/allow.rs` is expected too:
/// precision 189 of 214, recall 189 of 265, 104 declared.
/// Measured again on 61 cases (D1, 0.11.0): `evidence/cargo-lints-suppressed` (Cargo's lint
/// keys and compiler flags, a build script declared in the manifest; `--cap-lints` and `allow`
/// as two strings over lines a declared miss) and `evidence/rust-allow-in-comment-and-string`
/// (OD-E: `#[allow(` in a comment or string is no evidence, `r#unsafe` no `unsafe_added`):
/// precision 195 of 220, recall 195 of 272, 105 declared. The floors stay.
/// F2a.a (0.11.0) reads an assertion that cannot fail as a class: `evidence/rust-assertion-class`
/// (six forms, each beside a near miss; the same call on both sides is a near miss, two equal
/// strings a declared miss) is added: precision 201 of 226, recall 201 of 279, 106 declared.
/// F2a.b reads a `Result` of the standard library dropped with `let _ =`:
/// `evidence/rust-dropped-result` (three drops, two near misses, `let _unused =` and `.ok();`
/// declared): precision 204 of 229, recall 204 of 284, 108 declared.
const RANKING_PAIRS_FLOOR: u64 = 800_000;
const EVIDENCE_PRECISION_FLOOR: u64 = 881_516;
const EVIDENCE_RECALL_FLOOR: u64 = 704_545;
const FINDINGS_FLOOR: u64 = 1_000_000;
/// Declared misses (plan N4): each counts as wrong in its metric above. The receipt's
/// `known_misses` keeps its first meaning, the evidence `known_miss` items; `declared` lists all.
const DECLARED: u64 = 108;
const EVIDENCE_KNOWN_MISSES: u64 = 80;
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
    assert_eq!(receipt["known_misses"], EVIDENCE_KNOWN_MISSES);
    assert_eq!(
        receipt["declared"].as_array().map(Vec::len),
        Some(usize::try_from(DECLARED).unwrap()),
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
    assert_eq!(wrong, DECLARED, "{m:#}");
    // Every miss of recall is an evidence known miss: the field kept its eval.v0 meaning.
    assert_eq!(
        total("evidence_recall") - right("evidence_recall"),
        EVIDENCE_KNOWN_MISSES
    );
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

/// S3.35.b: the receipt splits evidence precision and recall by signal (`by_signal`) and by
/// the language of the item's file (`by_language`), and each split adds up to the aggregate the
/// floors hold. The `trivial_assert` pair (precision and recall; S3.16 narrowed it, F2a.a widened
/// it to the class: 14 of 14 and 14 of 29 before) and
/// the Python case count are held exactly, so a change to them shows here by name.
#[test]
fn eval_receipt_carries_per_signal_tallies() {
    let (code, receipt, stderr) = eval(&corpus());
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(
        receipt["by_signal"]["trivial_assert"],
        serde_json::json!({
            "precision": {"right": 20, "total": 20, "ppm": 1_000_000},
            "recall": {"right": 20, "total": 36, "ppm": 555_555},
            "cases": 14
        }),
        "the trivial_assert pair in {receipt:#}"
    );
    assert_eq!(receipt["by_language"]["python"]["cases"], 14, "{receipt:#}");
    let m = &receipt["metrics"];
    for member in ["by_signal", "by_language"] {
        let split = receipt[member].as_object().unwrap();
        for (metric, part) in [
            ("evidence_precision", "precision"),
            ("evidence_recall", "recall"),
        ] {
            for key in ["right", "total"] {
                let sum: u64 = split.values().map(|s| s[part][key].as_u64().unwrap()).sum();
                assert_eq!(sum, m[metric][key], "{member}: the sum of {part} {key}");
            }
        }
    }
}

/// A copy of the corpus inside a scratch directory, removed when the test ends.
struct Copy(
    PathBuf,
    #[expect(dead_code, reason = "held for its Drop, which removes the directory")] Scratch,
);

impl Copy {
    fn new(name: &str) -> Self {
        let scratch = Scratch::new(&format!("eval-{name}"));
        let dir = scratch.join("corpus");
        for sub in ["", "ranking", "evidence", "findings"] {
            std::fs::create_dir_all(dir.join(sub)).unwrap();
            for entry in std::fs::read_dir(corpus().join(sub)).unwrap() {
                let path = entry.unwrap().path();
                if path.is_file() {
                    std::fs::copy(&path, dir.join(sub).join(path.file_name().unwrap())).unwrap();
                }
            }
        }
        Self(dir, scratch)
    }

    fn manifest(&self) -> Value {
        serde_json::from_slice(&std::fs::read(self.0.join("MANIFEST.json")).unwrap()).unwrap()
    }

    fn write_manifest(&self, manifest: &Value) {
        std::fs::write(self.0.join("MANIFEST.json"), manifest.to_string()).unwrap();
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
        text.starts_with("deep-diff-forge eval: 63 cases\n"),
        "{text}"
    );
    for line in [
        "  ranking_pairs        12/15",
        "  evidence_precision   204/229",
        "  evidence_recall      204/284",
        "  declared             108",
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

    // A file cannot outrank itself: a pair naming one path twice is a malformed label, refused
    // before anything is scored (`the_manifest_cannot_drift_silently` covers known_miss too).
    let selfish = Copy::new("selfish");
    let mut manifest = selfish.manifest();
    manifest["cases"][0]["must_outrank"][0] = serde_json::json!(["src/util.rs", "src/util.rs"]);
    selfish.write_manifest(&manifest);
    let (code, _, stderr) = eval(&selfish.0);
    assert_eq!(code, 4, "{stderr}");

    let missing = Copy::new("missing");
    std::fs::remove_file(missing.0.join("findings/mixed.jsonl")).unwrap();
    assert_eq!(eval(&missing.0).0, 3);
}

/// S3.35.c: a case whose patch the parser refuses (here a hunk with no context, which plain
/// `git apply` places at the end of the file) is reported in `refused` with the parser's reason
/// and is a failure (exit 7), and every other case is still scored: before, the first refusal
/// ended the run with exit 4 and no figures.
#[test]
fn a_refused_case_is_reported_and_the_batch_continues() {
    let (_, whole, _) = eval(&corpus());
    let copy = Copy::new("refused");
    let file = copy.0.join("evidence/bash-dev-tcp.patch");
    let original = std::fs::read_to_string(&file).unwrap();
    let patch = original.replace("@@ -1,1 +1,3 @@\n set -eu\n", "@@ -5 +5,2 @@\n-set -eu\n");
    assert_ne!(patch, original);
    std::fs::write(&file, &patch).unwrap();
    let mut items = 0;
    edit_case(&copy, "evidence/bash-dev-tcp", |c| {
        c["sha256"] = deep_diff_forge_core::sha256_hex(patch.as_bytes()).into();
        items = c["expect"].as_array().unwrap().len() + c["known_miss"].as_array().unwrap().len();
    });
    let (code, receipt, stderr) = eval(&copy.0);
    assert_eq!(
        code, 7,
        "the run ends with a failure and a receipt: {stderr}"
    );
    let reason = receipt["refused"][0]["reason"].as_str().unwrap_or_default();
    assert_eq!(
        receipt["refused"],
        serde_json::json!([{"case": "evidence/bash-dev-tcp", "reason": reason}])
    );
    assert!(
        reason.contains("it has no context, so Git places it at the end of the file"),
        "{reason}"
    );
    assert_eq!(
        receipt["failures"],
        serde_json::json!([format!("evidence/bash-dev-tcp: refused: {reason}")])
    );
    // The other cases are scored: only the refused case's items leave the totals.
    let (m, all) = (&receipt["metrics"], &whole["metrics"]);
    assert_eq!(receipt["cases"], whole["cases"]);
    assert_eq!(m["ranking_pairs"], all["ranking_pairs"]);
    assert_eq!(m["findings_verdicts"], all["findings_verdicts"]);
    let recall = |m: &Value| m["evidence_recall"]["total"].as_u64().unwrap();
    assert_eq!(recall(m) + u64::try_from(items).unwrap(), recall(all));
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

/// A corpus symlink and a ranking pair that is not two distinct paths are refused (exit 4)
/// before anything is scored.
#[test]
fn a_link_and_a_degenerate_pair_are_refused() {
    let refused = |copy: &Copy, code: i32, says: &str| {
        let (got, receipt, stderr) = eval(&copy.0);
        assert_eq!(got, code, "{stderr}\n{receipt:#}");
        assert!(stderr.contains(says), "{says:?} in {stderr}");
    };

    // A symlink is refused even where a case names it: it could lead out of the corpus.
    #[cfg(unix)]
    {
        let linked = Copy::new("symlink");
        let outside = linked.0.with_extension("outside.patch");
        std::fs::rename(linked.0.join("ranking/npm-preinstall.patch"), &outside).unwrap();
        std::os::unix::fs::symlink(&outside, linked.0.join("ranking/npm-preinstall.patch"))
            .unwrap();
        let (got, receipt, stderr) = eval(&linked.0);
        assert_eq!(got, 4, "{stderr}\n{receipt:#}");
        assert!(
            stderr.contains("ranking/npm-preinstall.patch is not a regular file"),
            "{stderr}"
        );
    }

    // A pair is exactly two distinct paths: [p, p] can never hold, so as a known miss its
    // label could never go stale; a third path would be ignored.
    for (name, key, pair) in [
        (
            "same-pair",
            "known_miss",
            serde_json::json!(["src/util.rs", "src/util.rs"]),
        ),
        (
            "same-must",
            "must_outrank",
            serde_json::json!(["src/util.rs", "src/util.rs"]),
        ),
        (
            "three",
            "must_outrank",
            serde_json::json!(["src/util.rs", "docs/BIG.md", "x"]),
        ),
        ("one", "must_outrank", serde_json::json!(["src/util.rs"])),
    ] {
        let copy = Copy::new(name);
        edit_case(&copy, "ranking/circle-probe", |c| {
            c[key] = serde_json::json!([pair]);
        });
        refused(
            &copy,
            4,
            "case ranking/circle-probe: a pair is not two distinct paths",
        );
    }
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
            receipt["declared"].as_array().map_or(0, Vec::len),
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
    assert_eq!(known, usize::try_from(DECLARED).unwrap() - 1);

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
        serde_json::json!({"right": 204, "total": 283, "ppm": 720_848})
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
        serde_json::json!({"right": 204, "total": 229, "ppm": 890_829})
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
    assert_eq!(metrics["evidence_precision"]["right"], 205);
    assert_eq!(known, usize::try_from(DECLARED).unwrap() - 1);
}

/// DDF-HUNT-CLI-8: `MANIFEST.json` is read before the corpus walk checks file types, so a FIFO
/// there blocked `eval` in `open` for ever. It is refused at once with exit 4.
#[cfg(unix)]
#[test]
fn a_fifo_manifest_is_refused_not_waited_on() {
    let dir = Scratch::new("eval-fifo");
    assert!(
        Command::new("mkfifo")
            .arg(dir.join("MANIFEST.json"))
            .status()
            .unwrap()
            .success()
    );
    let mut child = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .args(["eval", "--json", "--corpus", dir.path().to_str().unwrap()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("run");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while child.try_wait().unwrap().is_none() {
        if std::time::Instant::now() > deadline {
            child.kill().unwrap();
            let _ = child.wait();
            panic!("eval still running after 10 s: it waits on the FIFO manifest");
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let out = child.wait_with_output().unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(4), "{stderr}");
    assert!(
        stderr.contains("MANIFEST.json is not a regular file"),
        "{stderr}"
    );
}

/// Expert circle C5: a corpus file over the 8 MiB file limit is refused, exit 4, before it is
/// read whole; one at the limit is read (and here fails only its hash check).
#[test]
fn a_corpus_file_over_the_limit_is_refused() {
    let dir = Scratch::new("eval-cap");
    let limit = 8 * 1024 * 1024;
    for (size, refused) in [(limit + 1, true), (limit, false)] {
        std::fs::write(dir.join("MANIFEST.json"), vec![b' '; size]).unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
            .args(["eval", "--json", "--corpus", dir.path().to_str().unwrap()])
            .output()
            .expect("run");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(4), "{size}: {stderr}");
        assert_eq!(
            stderr.contains("more than 8388608 bytes"),
            refused,
            "{size}: {stderr}"
        );
    }
}

/// Integrated review of 0.9.0, VI1-L-2 and CI3: `schema signals` gave `"basis": ["lexical"]`
/// for labels whose items carry `"hunk"`. Every evidence item ranked from a patch of the corpus
/// carries a basis that the vocabulary lists for its label, so the two cannot drift apart; and
/// the corpus has items on both rungs, so the check is not empty.
#[test]
fn every_item_of_the_corpus_has_a_basis_the_vocabulary_lists() {
    let binary = env!("CARGO_BIN_EXE_deep-diff-forge");
    let schema = Command::new(binary)
        .args(["schema", "signals"])
        .output()
        .expect("run");
    let schema: Value = serde_json::from_slice(&schema.stdout).unwrap();
    let listed = |label: &str| -> Vec<Value> {
        let facts = schema["signals"].as_array().unwrap().iter();
        let mut facts = facts.filter(|facts| facts["label"] == label);
        facts.next().unwrap()["basis"].as_array().unwrap().clone()
    };
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(corpus().join("MANIFEST.json")).unwrap()).unwrap();
    let mut rungs = std::collections::BTreeSet::new();
    for case in manifest["cases"].as_array().unwrap() {
        let patch = corpus().join(case["patch"].as_str().unwrap());
        let out = Command::new(binary)
            .args(["--stdin-patch", "--rank", "--json"])
            .stdin(std::fs::File::open(&patch).unwrap())
            .output()
            .expect("run");
        assert!(out.status.success(), "{}", patch.display());
        let ranking: Value = serde_json::from_slice(&out.stdout).unwrap();
        for file in ranking["ranked"].as_array().unwrap() {
            for item in file["evidence"].as_array().into_iter().flatten() {
                let label = item["signal"].as_str().unwrap();
                assert!(
                    listed(label).contains(&item["basis"]),
                    "{}: {item} has a basis `schema signals` does not list for {label}",
                    patch.display()
                );
                rungs.insert(item["basis"].as_str().unwrap().to_owned());
            }
        }
    }
    assert_eq!(rungs.into_iter().collect::<Vec<_>>(), ["hunk", "lexical"]);
}

/// Release 0.10.0 (ruling 1 of its integration): the sections of evaluation cases that the
/// parser refuses for the place of a hunk left the corpus with their expectations and are kept
/// in `fixtures/eval-refused-by-parser/`. Each is listed in its `REASONS.tsv` and is refused
/// whole (exit 4) with the reason listed there, so none of them can come back unread.
#[test]
fn every_section_that_left_the_corpus_is_refused_for_its_reason() {
    let dir = corpus().join("../eval-refused-by-parser");
    let reasons = std::fs::read_to_string(dir.join("REASONS.tsv")).unwrap();
    let rows: Vec<Vec<&str>> = reasons
        .lines()
        .filter(|line| !line.starts_with('#'))
        .map(|line| line.split('\t').collect())
        .collect();
    let mut patches: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|e| e == "patch"))
        .map(|path| path.file_name().unwrap().to_str().unwrap().to_owned())
        .collect();
    patches.sort();
    let listed: Vec<String> = rows.iter().map(|row| row[0].to_owned()).collect();
    assert_eq!(patches, listed, "every patch there is listed once");
    for row in &rows {
        let out = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
            .args(["--stdin-patch", "--outline"])
            .stdin(std::fs::File::open(dir.join(row[0])).unwrap())
            .output()
            .expect("run");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!((out.status.code(), row[1]), (Some(4), "4"), "{}", row[0]);
        assert!(stderr.contains(row[2]), "{}: {stderr}", row[0]);
        assert!(
            stderr.contains("would be placed elsewhere by git apply"),
            "{stderr}"
        );
    }
    assert_eq!(rows.len(), 16);
}

/// The `right/total` written after `label` in `text` (spaces and a table's `|` allowed around
/// it), or `None` when the label or the figure is not there.
fn figure_after(text: &str, label: &str) -> Option<(u64, u64)> {
    let rest = &text[text.find(label)? + label.len()..];
    let (right, rest) = rest.trim_start_matches([' ', '|']).split_once('/')?;
    let total: String = rest
        .trim_start()
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    Some((right.trim().parse().ok()?, total.parse().ok()?))
}

/// S3.35.e: the figures the documents write are the ones the corpus measures. The four metric
/// rows of the table in `docs/reference/evaluation.md`, and the case count and the four
/// figures of the evaluation gate in `docs/TESTING_GOLD_STANDARD.md`, equal the receipt; until
/// now both were carried by hand.
#[test]
fn the_figures_in_the_documents_are_the_measured_ones() {
    let (code, receipt, stderr) = eval(&corpus());
    assert_eq!(code, 0, "{stderr}");
    let m = &receipt["metrics"];
    let measured = |metric: &str| {
        let (right, total) = (&m[metric]["right"], &m[metric]["total"]);
        Some((right.as_u64().unwrap(), total.as_u64().unwrap()))
    };
    let doc = |rel: &str| {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(rel);
        std::fs::read_to_string(path).unwrap()
    };
    let table = doc("docs/reference/evaluation.md");
    let gold = doc("docs/TESTING_GOLD_STANDARD.md");
    let gate = &gold[gold
        .find("**Evaluation gate**")
        .expect("the evaluation gate")..];
    let gate = gate[..gate.find("\n- ").unwrap_or(gate.len())]
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    for (metric, row, sentence) in [
        ("ranking_pairs", "| ranking pairs |", "ranking pairs "),
        (
            "evidence_precision",
            "| evidence precision |",
            "evidence precision ",
        ),
        ("evidence_recall", "| evidence recall |", "evidence recall "),
        ("findings_verdicts", "| findings verdicts |", "findings "),
    ] {
        assert_eq!(
            table.matches(row).count(),
            1,
            "one row {row} in evaluation.md"
        );
        assert_eq!(
            figure_after(&table, row),
            measured(metric),
            "evaluation.md, the row {row}"
        );
        assert_eq!(
            figure_after(&gate, sentence),
            measured(metric),
            "TESTING_GOLD_STANDARD.md, {sentence}in: {gate}"
        );
    }
    let cases = format!("`fixtures/eval` ({} cases,", receipt["cases"]);
    assert!(gate.contains(&cases), "{cases} in: {gate}");
}
