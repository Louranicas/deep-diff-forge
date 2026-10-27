// DDF review route: DDF-R20. Status and remaining work are recorded in the linked ledger; this comment does not claim delivery.
// Design/status: docs/RECOMMENDATION_INTEGRATION.md#remaining-module-routes
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fcrates--deep-diff-forge-pipeline--src--jsonl
use deep_diff_forge_core::{PatchLineKind, ReviewFile};
use std::fmt::Write as _;

/// Render one newline-delimited JSON event per file (`--jsonl`).
///
/// Each line is a complete JSON object describing one file's change summary, so
/// a consumer can process the stream incrementally. The whole output is *not* a
/// single JSON document; that is [`deep_diff_forge_patch::to_json`].
#[must_use]
pub fn jsonl_events(files: &[ReviewFile]) -> String {
    let mut out = String::new();
    for file in files {
        let mut additions = 0usize;
        let mut deletions = 0usize;
        for hunk in &file.patch_twin.hunks {
            for line in &hunk.lines {
                match line.kind {
                    PatchLineKind::Added => additions += 1,
                    PatchLineKind::Removed => deletions += 1,
                    PatchLineKind::Context => {}
                }
            }
        }
        // `lossy_lines` appears only for a file with non-UTF-8 lines, so exact output is unchanged.
        let lossy = match file.patch_twin.lossy_lines {
            0 => String::new(),
            n => format!(",\"lossy_lines\":{n}"),
        };
        let _ = writeln!(
            out,
            "{{\"event\":\"diff.file\",\"path\":{},\"status\":\"{}\",\"additions\":{},\"deletions\":{},\"hunks\":{}{lossy}}}",
            quote(&file.path),
            file.status.label(),
            additions,
            deletions,
            file.patch_twin.hunks.len(),
        );
    }
    out
}

/// Quote and escape a string as a JSON string literal (RFC 8259).
///
/// Delegates to the canonical `core::json_escape` so `--jsonl` output carries the
/// same DEL/`0x7f` + C1 (`0x80..=0x9f`) terminal-safety hardening as every other
/// JSON sink — no forked escaper, one policy (S1008452).
fn quote(value: &str) -> String {
    deep_diff_forge_core::json_escape(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use deep_diff_forge_patch::parse;

    const TWO_FILES: &str = "--- a/a.txt\n+++ b/a.txt\n@@ -1,1 +1,1 @@\n-a\n+A\n--- a/b.txt\n+++ b/b.txt\n@@ -1,2 +1,1 @@\n keep\n-gone\n";

    #[test]
    fn one_line_per_file() {
        let files = parse(TWO_FILES).unwrap();
        assert_eq!(jsonl_events(&files).lines().count(), 2);
    }

    #[test]
    fn each_line_is_a_diff_file_event() {
        let files = parse(TWO_FILES).unwrap();
        for line in jsonl_events(&files).lines() {
            assert!(line.contains("\"event\":\"diff.file\""));
        }
    }

    #[test]
    fn event_reports_path() {
        let files = parse(TWO_FILES).unwrap();
        let out = jsonl_events(&files);
        assert!(out.contains("\"path\":\"a.txt\""));
        assert!(out.contains("\"path\":\"b.txt\""));
    }

    #[test]
    fn event_reports_addition_and_deletion_counts() {
        let files = parse(TWO_FILES).unwrap();
        let first = jsonl_events(&files).lines().next().unwrap().to_string();
        assert!(first.contains("\"additions\":1"));
        assert!(first.contains("\"deletions\":1"));
    }

    #[test]
    fn deletion_only_file_counts_zero_additions() {
        let files = parse(TWO_FILES).unwrap();
        let second = jsonl_events(&files).lines().nth(1).unwrap().to_string();
        assert!(second.contains("\"additions\":0"));
        assert!(second.contains("\"deletions\":1"));
    }

    #[test]
    fn event_reports_hunk_count() {
        let files = parse(TWO_FILES).unwrap();
        let first = jsonl_events(&files).lines().next().unwrap().to_string();
        assert!(first.contains("\"hunks\":1"));
    }

    #[test]
    fn empty_input_yields_no_lines() {
        assert_eq!(jsonl_events(&[]), "");
    }

    #[test]
    fn status_appears_in_event() {
        let files = parse("--- /dev/null\n+++ b/n\n@@ -0,0 +1,1 @@\n+x\n").unwrap();
        assert!(jsonl_events(&files).contains("\"status\":\"added\""));
    }

    #[test]
    fn binary_file_event_has_zero_hunks() {
        let files =
            parse("diff --git a/p.png b/p.png\nBinary files a/p.png and b/p.png differ\n").unwrap();
        let out = jsonl_events(&files);
        assert!(out.contains("\"status\":\"binary_changed\""));
        assert!(out.contains("\"hunks\":0"));
    }

    #[test]
    fn quote_escapes_special_path_chars() {
        assert_eq!(quote("a\"b\\c"), "\"a\\\"b\\\\c\"");
    }

    #[test]
    fn quote_passes_unicode_paths() {
        assert_eq!(quote("café/x"), "\"café/x\"");
    }

    #[test]
    fn each_line_is_independently_valid_json_object() {
        let files = parse(TWO_FILES).unwrap();
        for line in jsonl_events(&files).lines() {
            assert!(line.starts_with('{'));
            assert!(line.ends_with('}'));
        }
    }

    #[test]
    fn modified_status_appears_in_event() {
        let files = parse(TWO_FILES).unwrap();
        assert!(jsonl_events(&files).contains("\"status\":\"modified\""));
    }

    #[test]
    fn multi_hunk_file_reports_two_hunks() {
        let input = "--- a/x\n+++ b/x\n@@ -1,1 +1,1 @@\n-a\n+A\n@@ -9,1 +9,1 @@\n-b\n+B\n";
        let files = parse(input).unwrap();
        assert!(jsonl_events(&files).contains("\"hunks\":2"));
    }

    #[test]
    fn deleted_file_status_in_event() {
        let files =
            parse("diff --git a/o b/o\ndeleted file mode 100644\n--- a/o\n+++ /dev/null\n@@ -1,1 +0,0 @@\n-x\n")
                .unwrap();
        assert!(jsonl_events(&files).contains("\"status\":\"deleted\""));
    }

    #[test]
    fn each_event_is_newline_terminated() {
        let files = parse(TWO_FILES).unwrap();
        assert!(jsonl_events(&files).ends_with("}\n"));
    }
}
