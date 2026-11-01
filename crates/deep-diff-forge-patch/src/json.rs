// DDF integration R21: derive semantic fallback totals from the enriched model; skipped coverage remains explicit in analysis.v1.
// Design/status: docs/RECOMMENDATION_INTEGRATION.md#shared-service
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fdeep-diff-forge-patch--json
// DDF review route: DDF-R21. Status and remaining work are recorded in the linked ledger; this comment does not claim delivery.
// Design/status: docs/RECOMMENDATION_INTEGRATION.md#remaining-module-routes
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fcrates--deep-diff-forge-patch--src--json
use deep_diff_forge_core::{DiffStrategy, InputSeal, PatchLineKind, ReviewFile};
use std::fmt::Write as _;

/// Schema identifier for the JSON document this module emits.
pub const SCHEMA: &str = "deep-diff-forge.review.v0";

/// Project the parsed patch model into a stable JSON document.
///
/// The output is a single complete JSON document (not JSONL) carrying the
/// `deep-diff-forge.review.v0` schema, every file with its status, strategy,
/// hunks, line anchors, and metadata, plus a summary. JSON is hand-rolled to
/// keep the workspace dependency-free at L1.
///
/// This form carries no input seal (the caller has no raw input bytes — e.g.
/// the TUI's in-memory review); the CLI's `--stdin-patch --json` uses
/// [`SealedReview::to_json`](crate::SealedReview::to_json) so a consumer can tie the document
/// to its exact input.
#[must_use]
pub fn to_json(files: &[ReviewFile]) -> String {
    render(files, None)
}

/// [`to_json`] plus the additive top-level `input_sha256` and `tool` members
/// from `seal`, placed directly after `schema`. The schema name is unchanged:
/// every `review.v0` field keeps its meaning, so existing consumers are
/// unaffected and a gate can additionally verify the seal.
#[must_use]
pub fn to_json_sealed(files: &[ReviewFile], seal: &InputSeal) -> String {
    render(files, Some(seal))
}

fn render(files: &[ReviewFile], seal: Option<&InputSeal>) -> String {
    let mut additions = 0usize;
    let mut deletions = 0usize;
    let mut out = String::new();
    out.push_str("{\n");
    let _ = writeln!(out, "  \"schema\": {},", quote(SCHEMA));
    if let Some(seal) = seal {
        out.push_str(&seal.json_members());
    }
    out.push_str("  \"files\": [");

    for (i, file) in files.iter().enumerate() {
        if i == 0 {
            out.push('\n');
        }
        out.push_str("    {\n");
        let _ = writeln!(out, "      \"path\": {},", quote(&file.path));
        let _ = writeln!(out, "      \"status\": {},", quote(file.status.label()));
        let _ = writeln!(
            out,
            "      \"strategy\": {},",
            quote(strategy_str(file.planner.strategy))
        );
        // Present only for a file with non-UTF-8 lines, so exact output is unchanged (C-12).
        if file.patch_twin.lossy_lines > 0 {
            let _ = writeln!(
                out,
                "      \"lossy_lines\": {},",
                file.patch_twin.lossy_lines
            );
        }
        // Objective hazards in the path (traversal, `.git`, bidi, invisible, control); present
        // only when there are any, so other output is unchanged.
        let path_hazards = deep_diff_forge_core::path_hazards(&file.path);
        if !path_hazards.is_empty() {
            let _ = writeln!(
                out,
                "      \"hazards\": {},",
                string_array(
                    &path_hazards
                        .iter()
                        .map(|h| (*h).to_owned())
                        .collect::<Vec<_>>()
                )
            );
        }
        out.push_str("      \"hunks\": [");
        render_hunks(&mut out, file, &mut additions, &mut deletions);
        out.push_str("],\n");
        let _ = writeln!(
            out,
            "      \"metadata\": {}",
            string_array(&file.patch_twin.metadata)
        );
        out.push_str("    }");
        if i + 1 < files.len() {
            out.push(',');
        }
        out.push('\n');
    }
    if files.is_empty() {
        out.push_str("],\n");
    } else {
        out.push_str("  ],\n");
    }

    out.push_str("  \"summary\": {\n");
    let _ = writeln!(out, "    \"files_changed\": {},", files.len());
    let _ = writeln!(out, "    \"additions\": {additions},");
    let _ = writeln!(out, "    \"deletions\": {deletions},");
    let fallbacks = files
        .iter()
        .filter(|file| {
            file.semantic_twin.as_ref().is_some_and(|twin| {
                matches!(
                    twin.parse_status,
                    deep_diff_forge_core::ParseStatus::Fallback { .. }
                )
            })
        })
        .count();
    let _ = writeln!(out, "    \"semantic_fallbacks\": {fallbacks}");
    out.push_str("  }\n");
    out.push_str("}\n");
    out
}

fn render_hunks(out: &mut String, file: &ReviewFile, additions: &mut usize, deletions: &mut usize) {
    let hunks = &file.patch_twin.hunks;
    for (h, hunk) in hunks.iter().enumerate() {
        if h == 0 {
            out.push('\n');
        }
        out.push_str("        {\n");
        let _ = writeln!(out, "          \"id\": {},", hunk.id.0);
        let _ = writeln!(out, "          \"old_start\": {},", opt_num(hunk.old_start));
        let _ = writeln!(out, "          \"new_start\": {},", opt_num(hunk.new_start));
        // Additive (review.v0): the header's line counts, its section heading, the hunk's
        // address within this review and the digest of its body (`PatchHunk::address`).
        let (old_lines, new_lines) = hunk.counts();
        let _ = writeln!(out, "          \"old_lines\": {old_lines},");
        let _ = writeln!(out, "          \"new_lines\": {new_lines},");
        let _ = writeln!(
            out,
            "          \"section\": {},",
            hunk.section
                .as_deref()
                .map_or_else(|| "null".to_owned(), quote)
        );
        let _ = writeln!(
            out,
            "          \"address\": {},",
            quote(&hunk.address(&file.path))
        );
        let _ = writeln!(
            out,
            "          \"body_sha256\": {},",
            quote(&hunk.body_sha256())
        );
        out.push_str("          \"lines\": [");
        for (l, line) in hunk.lines.iter().enumerate() {
            match line.kind {
                PatchLineKind::Added => *additions += 1,
                PatchLineKind::Removed => *deletions += 1,
                PatchLineKind::Context => {}
            }
            if l == 0 {
                out.push('\n');
            }
            out.push_str("            {");
            let _ = write!(out, "\"kind\": {}, ", quote(line_kind_str(line.kind)));
            let _ = write!(out, "\"old_line\": {}, ", opt_num(line.old_line));
            let _ = write!(out, "\"new_line\": {}, ", opt_num(line.new_line));
            let _ = write!(out, "\"text\": {}", quote(&line.text));
            // Which side lacks its final LF follows from `kind` (removed: old, added: new,
            // context: both); present only when true, so other output is unchanged (anchor A-06).
            if line.no_newline {
                out.push_str(", \"no_newline\": true");
            }
            // Which line was decoded lossily, so a consumer need not guess from U+FFFD (a literal
            // U+FFFD in valid input is not lossy); present only when true.
            if line.lossy {
                out.push_str(", \"lossy\": true");
            }
            // Bidi controls, invisible characters and control characters in the line text, which
            // can make what a reviewer sees differ from what is applied; present only when found.
            let hazards = deep_diff_forge_core::text_hazards(&line.text);
            if !hazards.is_empty() {
                let _ = write!(
                    out,
                    ", \"hazards\": {}",
                    string_array(&hazards.iter().map(|h| (*h).to_owned()).collect::<Vec<_>>())
                );
            }
            out.push('}');
            if l + 1 < hunk.lines.len() {
                out.push(',');
            }
            out.push('\n');
        }
        if hunk.lines.is_empty() {
            out.push(']');
        } else {
            out.push_str("          ]");
        }
        out.push('\n');
        out.push_str("        }");
        if h + 1 < hunks.len() {
            out.push(',');
        }
        out.push('\n');
    }
    if !hunks.is_empty() {
        out.push_str("      ");
    }
}

fn opt_num(value: Option<u32>) -> String {
    value.map_or_else(|| "null".to_string(), |n| n.to_string())
}

fn strategy_str(strategy: DiffStrategy) -> &'static str {
    match strategy {
        DiffStrategy::Line => "line",
        DiffStrategy::Word => "word",
        DiffStrategy::Syntax => "syntax",
        DiffStrategy::MovedBlock => "moved_block",
        DiffStrategy::Binary => "binary",
        DiffStrategy::GeneratedSuppressed => "generated_suppressed",
    }
}

fn line_kind_str(kind: PatchLineKind) -> &'static str {
    match kind {
        PatchLineKind::Context => "context",
        PatchLineKind::Added => "added",
        PatchLineKind::Removed => "removed",
    }
}

fn string_array(items: &[String]) -> String {
    if items.is_empty() {
        return "[]".to_string();
    }
    let parts: Vec<String> = items.iter().map(|s| quote(s)).collect();
    format!("[{}]", parts.join(", "))
}

/// Quote and escape a string as a JSON string literal (RFC 8259).
///
/// Delegates to the canonical `core::json_escape` so machine output (`--json`)
/// carries the same DEL/`0x7f` + C1 (`0x80..=0x9f`) terminal-safety hardening as
/// every other JSON sink — a review tool's machine output is routinely printed to
/// a terminal, and a forked escaper that dropped that coverage was a real
/// defence-in-depth gap (S1008452). One escaper, one policy.
fn quote(value: &str) -> String {
    deep_diff_forge_core::json_escape(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    const BASIC: &str = "--- a/x\n+++ b/x\n@@ -1,3 +1,3 @@\n a\n-b\n+B\n c\n";

    #[test]
    fn output_declares_schema() {
        let files = parse(BASIC).unwrap();
        let json = to_json(&files);
        assert!(json.contains("\"schema\": \"deep-diff-forge.review.v0\""));
    }

    #[test]
    fn output_contains_file_path() {
        let files = parse(BASIC).unwrap();
        let json = to_json(&files);
        assert!(json.contains("\"path\": \"x\""));
    }

    #[test]
    fn output_contains_status() {
        let files = parse(BASIC).unwrap();
        let json = to_json(&files);
        assert!(json.contains("\"status\": \"modified\""));
    }

    #[test]
    fn summary_counts_additions_and_deletions() {
        let files = parse(BASIC).unwrap();
        let json = to_json(&files);
        assert!(json.contains("\"additions\": 1"));
        assert!(json.contains("\"deletions\": 1"));
        assert!(json.contains("\"files_changed\": 1"));
    }

    #[test]
    fn empty_model_emits_empty_file_list() {
        let json = to_json(&[]);
        assert!(json.contains("\"files\": []"));
        assert!(json.contains("\"files_changed\": 0"));
    }

    #[test]
    fn unsealed_output_carries_no_seal_members() {
        let files = parse(BASIC).unwrap();
        let json = to_json(&files);
        assert!(!json.contains("input_sha256"));
        assert!(!json.contains("\"tool\""));
    }

    #[test]
    fn sealed_output_places_seal_directly_after_schema() {
        let files = parse(BASIC).unwrap();
        let seal = InputSeal::of(BASIC.as_bytes(), "1.2.3");
        let json = to_json_sealed(&files, &seal);
        let expected_head = format!(
            "{{\n  \"schema\": \"deep-diff-forge.review.v0\",\n  \"input_sha256\": \"{}\",\n  \"tool\": {{\"name\": \"deep-diff-forge\", \"version\": \"1.2.3\"}},\n  \"files\": [",
            seal.input_sha256
        );
        assert!(json.starts_with(&expected_head), "got:\n{json}");
        // The rest of the document is byte-identical to the unsealed form.
        assert_eq!(json.replacen(&seal.json_members(), "", 1), to_json(&files));
    }

    #[test]
    fn sealed_empty_model_is_still_well_formed() {
        let seal = InputSeal::of(b"", "0.0.0");
        let json = to_json_sealed(&[], &seal);
        assert!(json.contains("\"input_sha256\": \"e3b0c442"));
        assert!(json.contains("\"files\": []"));
    }

    #[test]
    fn quote_escapes_double_quotes() {
        assert_eq!(quote("a\"b"), "\"a\\\"b\"");
    }

    #[test]
    fn quote_escapes_backslash() {
        assert_eq!(quote("a\\b"), "\"a\\\\b\"");
    }

    #[test]
    fn quote_escapes_newline_and_tab() {
        assert_eq!(quote("a\nb\tc"), "\"a\\nb\\tc\"");
    }

    #[test]
    fn quote_escapes_control_chars_as_unicode() {
        assert_eq!(quote("\u{01}"), "\"\\u0001\"");
    }

    #[test]
    fn quote_passes_through_unicode() {
        assert_eq!(quote("café→"), "\"café→\"");
    }

    #[test]
    fn line_text_with_quotes_is_escaped_in_output() {
        let input = "--- a/x\n+++ b/x\n@@ -1,1 +1,1 @@\n-old\n+let s = \"hi\";\n";
        let files = parse(input).unwrap();
        let json = to_json(&files);
        assert!(json.contains("let s = \\\"hi\\\";"));
    }

    #[test]
    fn binary_file_has_empty_hunks_array() {
        let input = "diff --git a/p.png b/p.png\nBinary files a/p.png and b/p.png differ\n";
        let files = parse(input).unwrap();
        let json = to_json(&files);
        assert!(json.contains("\"hunks\": []"));
        assert!(json.contains("\"status\": \"binary_changed\""));
    }

    #[test]
    fn hunk_includes_id_and_starts() {
        let files = parse(BASIC).unwrap();
        let json = to_json(&files);
        assert!(json.contains("\"id\": 0"));
        assert!(json.contains("\"old_start\": 1"));
        assert!(json.contains("\"new_start\": 1"));
    }

    /// Hazards appear on the line and the file that carry them, and nowhere else.
    #[test]
    fn hazards_mark_the_lines_and_paths_that_carry_them() {
        let input = "--- a/../x\n+++ b/../x\n@@ -1,2 +1,2 @@\n same\n-a\n+b\u{202E}c\n";
        let json = to_json(&parse(input).unwrap());
        assert!(
            json.contains("\"hazards\": [\"path_traversal\"],\n      \"hunks\""),
            "{json}"
        );
        assert_eq!(
            json.matches("\"hazards\": [\"bidi_control\"]").count(),
            1,
            "{json}"
        );
        let marked = json.lines().find(|l| l.contains("bidi_control")).unwrap();
        assert!(marked.contains("\"kind\": \"added\""), "{marked}");
        let plain = to_json(&parse("--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n").unwrap());
        assert!(!plain.contains("hazards"));
    }

    #[test]
    fn metadata_array_is_present() {
        let input = "diff --git a/x b/x\nindex 111..222 100644\n--- a/x\n+++ b/x\n@@ -1,1 +1,1 @@\n-a\n+b\n";
        let files = parse(input).unwrap();
        let json = to_json(&files);
        assert!(json.contains("\"metadata\": [\"index 111..222 100644\"]"));
    }

    /// The whole `review.v0` document for two files, one with two hunks, a lossy line and a
    /// missing final newline: separators, optional members and summary totals in one exact
    /// form. The digests were checked with `sha256sum` (see `core`'s `hunk_tests`).
    #[test]
    fn a_two_file_review_renders_exactly() {
        let input: &[u8] = b"--- a/x\n+++ b/x\n@@ -1,2 +1,2 @@\n a\n-b\xff\n+c\n@@ -9 +9 @@\n-d\n+e\n\\ No newline at end of file\n--- a/y\n+++ b/y\n@@ -1 +1 @@\n-f\n+g\n";
        let files =
            crate::parse_decoded(&crate::decode(input), crate::ParseOptions::default()).unwrap();
        assert_eq!(to_json(&files), GOLDEN);
    }

    const GOLDEN: &str = r#"{
  "schema": "deep-diff-forge.review.v0",
  "files": [
    {
      "path": "x",
      "status": "modified",
      "strategy": "line",
      "lossy_lines": 1,
      "hunks": [
        {
          "id": 0,
          "old_start": 1,
          "new_start": 1,
          "old_lines": 2,
          "new_lines": 2,
          "section": null,
          "address": "0c375da08797b82b19dd327cab03bc6650c0302582bcdcdfb9c94df3105e95f7",
          "body_sha256": "4caeae8856c37e6b778101e2f34a6d29c258afb11decccac072ccb482eee2a70",
          "lines": [
            {"kind": "context", "old_line": 1, "new_line": 1, "text": "a"},
            {"kind": "removed", "old_line": 2, "new_line": null, "text": "b�", "lossy": true},
            {"kind": "added", "old_line": null, "new_line": 2, "text": "c"}
          ]
        },
        {
          "id": 1,
          "old_start": 9,
          "new_start": 9,
          "old_lines": 1,
          "new_lines": 1,
          "section": null,
          "address": "bd624a787edaf09671d2d89e3b22b6457a0bfdd2534a9edcef592a12a415e747",
          "body_sha256": "658cca8f2358ede06a0a058ab3bb5fa3f3e78d9fc1cf4394f632b5f64bf6c9b3",
          "lines": [
            {"kind": "removed", "old_line": 9, "new_line": null, "text": "d"},
            {"kind": "added", "old_line": null, "new_line": 9, "text": "e", "no_newline": true}
          ]
        }
      ],
      "metadata": ["\\ No newline at end of file"]
    },
    {
      "path": "y",
      "status": "modified",
      "strategy": "line",
      "hunks": [
        {
          "id": 2,
          "old_start": 1,
          "new_start": 1,
          "old_lines": 1,
          "new_lines": 1,
          "section": null,
          "address": "cc1f23a4548d440bd8795504d590067998f97dcd000e16c97709bbe54f0c7b87",
          "body_sha256": "4a2e1ae08c814d529acf73e734302cedd7d7b41d24b8b02ef4c45e7fe1ccff7c",
          "lines": [
            {"kind": "removed", "old_line": 1, "new_line": null, "text": "f"},
            {"kind": "added", "old_line": null, "new_line": 1, "text": "g"}
          ]
        }
      ],
      "metadata": []
    }
  ],
  "summary": {
    "files_changed": 2,
    "additions": 3,
    "deletions": 3,
    "semantic_fallbacks": 0
  }
}
"#;
}
