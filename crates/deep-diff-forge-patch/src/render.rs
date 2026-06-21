use deep_diff_forge_core::{FileStatus, PatchHunk, PatchLineKind, ReviewFile};
use std::fmt::Write as _;

/// Render an apply-able unified patch from the parsed model.
///
/// The render is *model-stable* for content: re-parsing the output yields the
/// same hunks, paths, and statuses. It is not byte-identical to the original,
/// because the core model intentionally drops cosmetic detail such as the
/// trailing `@@` section heading, and it deliberately does not re-emit the
/// `\ No newline at end of file` marker (whose only correct position is after a
/// specific content line, which the model does not anchor) — emitting it in the
/// header would break apply-ability. Apply-ability is always preserved.
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

    let _ = writeln!(out, "diff --git a/{old_label} b/{new_label}");
    for meta in &file.patch_twin.metadata {
        // The no-newline marker belongs inside a hunk, not in the file header.
        if meta.starts_with('\\') {
            continue;
        }
        out.push_str(meta);
        out.push('\n');
    }

    // Binary and pure-metadata changes have no hunks to render.
    if file.patch_twin.hunks.is_empty() {
        return;
    }

    if file.status == FileStatus::Added {
        out.push_str("--- /dev/null\n");
    } else {
        let _ = writeln!(out, "--- a/{old_label}");
    }
    if file.status == FileStatus::Deleted {
        out.push_str("+++ /dev/null\n");
    } else {
        let _ = writeln!(out, "+++ b/{new_label}");
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
    }
}

fn old_label(file: &ReviewFile) -> String {
    if file.status == FileStatus::Renamed {
        for meta in &file.patch_twin.metadata {
            if let Some(from) = meta.strip_prefix("rename from ") {
                return from.to_string();
            }
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
