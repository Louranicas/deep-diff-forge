use deep_diff_forge_core::{DiffStrategy, FileStatus, InputSeal, PatchLineKind, ReviewFile};
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
/// [`to_json_sealed`] so a consumer can tie the document to its exact input.
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
        let _ = writeln!(out, "      \"status\": {},", quote(status_str(file.status)));
        let _ = writeln!(
            out,
            "      \"strategy\": {},",
            quote(strategy_str(file.planner.strategy))
        );
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
    out.push_str("    \"semantic_fallbacks\": 0\n");
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

fn status_str(status: FileStatus) -> &'static str {
    match status {
        FileStatus::Added => "added",
        FileStatus::Modified => "modified",
        FileStatus::Deleted => "deleted",
        FileStatus::Renamed => "renamed",
        FileStatus::TypeChanged => "type_changed",
        FileStatus::BinaryChanged => "binary_changed",
        FileStatus::Unknown => "unknown",
    }
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

    #[test]
    fn metadata_array_is_present() {
        let input = "diff --git a/x b/x\nindex 111..222 100644\n--- a/x\n+++ b/x\n@@ -1,1 +1,1 @@\n-a\n+b\n";
        let files = parse(input).unwrap();
        let json = to_json(&files);
        assert!(json.contains("\"metadata\": [\"index 111..222 100644\"]"));
    }
}
