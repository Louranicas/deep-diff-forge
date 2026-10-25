// DDF integration: R02, R03, R04, R05, R06, R07, R44, R46. Preserve source bytes and exact original input independently of normalized display; use Git as the application oracle.
// Design/status: docs/RECOMMENDATION_INTEGRATION.md#patch-truth
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fdeep-diff-forge-patch--render
use deep_diff_forge_core::{FileStatus, PatchHunk, PatchLineKind, ReviewFile};
use std::fmt::Write as _;

/// Render an apply-able unified patch from the parsed model.
///
/// Normalizes cosmetic headings while preserving content CR bytes, per-line
/// EOF markers, encoded binary payloads and Git-quoted UTF-8 paths. Unsupported
/// combined diffs and non-UTF-8 quoted paths are rejected during parsing.
#[must_use]
pub fn render_unified(files: &[ReviewFile]) -> String {
    let mut out = String::new();
    for file in files {
        render_file(&mut out, file);
    }
    out
}

fn render_file(out: &mut String, file: &ReviewFile) {
    let old_label = old_label(file);
    let new_label = &file.path;

    let _ = writeln!(
        out,
        "diff --git {} {}",
        super::paths::quote(&format!("a/{old_label}")),
        super::paths::quote(&format!("b/{new_label}"))
    );
    for meta in &file.patch_twin.metadata {
        // The no-newline marker belongs inside a hunk, not in the file header.
        if meta.starts_with('\\') {
            continue;
        }
        out.push_str(meta);
        out.push('\n');
    }

    for line in &file.patch_twin.binary_body {
        out.push_str(line);
        out.push('\n');
    }
    // Binary and pure-metadata changes have no hunks to render.
    if file.patch_twin.hunks.is_empty() {
        return;
    }

    if file.status == FileStatus::Added {
        out.push_str("--- /dev/null\n");
    } else {
        let _ = writeln!(
            out,
            "--- {}",
            super::paths::quote(&format!("a/{old_label}"))
        );
    }
    if file.status == FileStatus::Deleted {
        out.push_str("+++ /dev/null\n");
    } else {
        let _ = writeln!(
            out,
            "+++ {}",
            super::paths::quote(&format!("b/{new_label}"))
        );
    }

    for hunk in &file.patch_twin.hunks {
        render_hunk(out, hunk);
    }
}

fn render_hunk(out: &mut String, hunk: &PatchHunk) {
    let old_start = hunk.old_start.unwrap_or(0);
    let new_start = hunk.new_start.unwrap_or(0);
    let old_count = hunk
        .lines
        .iter()
        .filter(|l| matches!(l.kind, PatchLineKind::Context | PatchLineKind::Removed))
        .count();
    let new_count = hunk
        .lines
        .iter()
        .filter(|l| matches!(l.kind, PatchLineKind::Context | PatchLineKind::Added))
        .count();

    let _ = writeln!(
        out,
        "@@ -{old_start},{old_count} +{new_start},{new_count} @@"
    );
    for line in &hunk.lines {
        let marker = match line.kind {
            PatchLineKind::Context => ' ',
            PatchLineKind::Added => '+',
            PatchLineKind::Removed => '-',
        };
        out.push(marker);
        out.push_str(&line.text);
        out.push('\n');
        if line.no_newline {
            out.push_str("\\ No newline at end of file\n");
        }
    }
}

fn old_label(file: &ReviewFile) -> String {
    for meta in &file.patch_twin.metadata {
        if let Some(from) = meta
            .strip_prefix("rename from ")
            .or_else(|| meta.strip_prefix("copy from "))
        {
            return super::paths::decode_path(from).unwrap_or_else(|| from.to_string());
        }
    }
    file.path.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    #[test]
    fn renders_diff_git_header() {
        let files = parse("--- a/x\n+++ b/x\n@@ -1,1 +1,1 @@\n-a\n+b\n").unwrap();
        let out = render_unified(&files);
        assert!(out.starts_with("diff --git a/x b/x\n"));
    }

    #[test]
    fn renders_hunk_header_with_counts() {
        let files = parse("--- a/x\n+++ b/x\n@@ -1,2 +1,2 @@\n a\n-b\n+B\n").unwrap();
        let out = render_unified(&files);
        assert!(out.contains("@@ -1,2 +1,2 @@\n"));
    }

    #[test]
    fn renders_line_markers() {
        let files = parse("--- a/x\n+++ b/x\n@@ -1,2 +1,2 @@\n ctx\n-old\n+new\n").unwrap();
        let out = render_unified(&files);
        assert!(out.contains(" ctx\n"));
        assert!(out.contains("-old\n"));
        assert!(out.contains("+new\n"));
    }

    #[test]
    fn added_file_renders_dev_null_old_side() {
        let input = "diff --git a/n b/n\nnew file mode 100644\n--- /dev/null\n+++ b/n\n@@ -0,0 +1,1 @@\n+x\n";
        let files = parse(input).unwrap();
        let out = render_unified(&files);
        assert!(out.contains("--- /dev/null\n"));
        assert!(out.contains("+++ b/n\n"));
    }

    #[test]
    fn deleted_file_renders_dev_null_new_side() {
        let input = "diff --git a/o b/o\ndeleted file mode 100644\n--- a/o\n+++ /dev/null\n@@ -1,1 +0,0 @@\n-x\n";
        let files = parse(input).unwrap();
        let out = render_unified(&files);
        assert!(out.contains("+++ /dev/null\n"));
    }

    #[test]
    fn binary_file_renders_metadata_without_hunks() {
        let input = "diff --git a/p.png b/p.png\nBinary files a/p.png and b/p.png differ\n";
        let files = parse(input).unwrap();
        let out = render_unified(&files);
        assert!(out.contains("Binary files a/p.png and b/p.png differ\n"));
        assert!(!out.contains("@@"));
    }

    #[test]
    fn rename_uses_from_path_on_old_side() {
        let input =
            "diff --git a/old b/new\nsimilarity index 100%\nrename from old\nrename to new\n";
        let files = parse(input).unwrap();
        let out = render_unified(&files);
        assert!(out.contains("diff --git a/old b/new\n"));
        assert!(out.contains("rename from old\n"));
    }

    #[test]
    fn no_newline_marker_not_emitted_in_header() {
        let input = "--- a/x\n+++ b/x\n@@ -1,1 +1,1 @@\n-a\n+b\n\\ No newline at end of file\n";
        let files = parse(input).unwrap();
        let out = render_unified(&files);
        // The marker is metadata; it must not leak into the file header block.
        let header_block = out.split("@@").next().unwrap();
        assert!(!header_block.contains("No newline"));
    }

    #[test]
    fn empty_model_renders_empty_string() {
        assert_eq!(render_unified(&[]), "");
    }

    #[test]
    fn multi_hunk_render_has_two_headers() {
        let input = "--- a/x\n+++ b/x\n@@ -1,1 +1,1 @@\n-a\n+A\n@@ -9,1 +9,1 @@\n-b\n+B\n";
        let files = parse(input).unwrap();
        let out = render_unified(&files);
        assert_eq!(out.matches("@@ -").count(), 2);
    }
}
