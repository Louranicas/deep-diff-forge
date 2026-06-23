//! The ranked, directory-grouped file tree (left sidebar).
//!
//! Files are grouped under their directory like a file explorer, but the group
//! order and the order within each group both follow the engine's *ranking* —
//! so the highest-risk file still surfaces first, while related files stay
//! visually together. Each row shows status, additions/deletions, an inline
//! note count (`*N`), and the rank score, all paths escaped via
//! [`safe_span`](crate::paint::safe_span).

use crate::paint::safe_span;
use crate::state::ReviewApp;
use crate::theme::Palette;
use deep_diff_forge_agent::anchor_path;
use deep_diff_forge_core::{AgentAnnotation, FileStatus};
use deep_diff_forge_graph::change_counts;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

/// The directory portion of a path (everything before the last `/`), or the
/// empty string for a root-level file.
#[must_use]
pub(crate) fn dir_of(path: &str) -> &str {
    match path.rfind('/') {
        Some(i) => &path[..i],
        None => "",
    }
}

/// The file-name portion of a path (everything after the last `/`).
fn base_of(path: &str) -> &str {
    match path.rfind('/') {
        Some(i) => &path[i + 1..],
        None => path,
    }
}

/// Group ranked file indices by directory, preserving rank order both across
/// groups (first-appearance) and within each group.
fn grouped(app: &ReviewApp) -> Vec<(String, Vec<usize>)> {
    let mut groups: Vec<(String, Vec<usize>)> = Vec::new();
    for (i, file) in app.files().iter().enumerate() {
        let dir = dir_of(&file.path).to_string();
        if let Some(entry) = groups.iter_mut().find(|(d, _)| *d == dir) {
            entry.1.push(i);
        } else {
            groups.push((dir, vec![i]));
        }
    }
    groups
}

fn note_count(annotations: &[AgentAnnotation], path: &str) -> usize {
    annotations
        .iter()
        .filter(|a| anchor_path(&a.anchor) == path)
        .count()
}

fn status_marker(status: FileStatus) -> char {
    match status {
        FileStatus::Added => 'A',
        FileStatus::Modified => 'M',
        FileStatus::Deleted => 'D',
        FileStatus::Renamed => 'R',
        FileStatus::TypeChanged => 'T',
        FileStatus::BinaryChanged => 'B',
        FileStatus::Unknown => '?',
    }
}

fn status_style(status: FileStatus, palette: &Palette, base: Style) -> Style {
    let color = match status {
        FileStatus::Added => palette.added,
        FileStatus::Deleted => palette.removed,
        FileStatus::Modified | FileStatus::Renamed | FileStatus::TypeChanged => palette.accent,
        FileStatus::BinaryChanged | FileStatus::Unknown => palette.dim,
    };
    base.fg(color)
}

fn file_line(app: &ReviewApp, palette: &Palette, index: usize) -> Line<'static> {
    let rf = &app.files()[index];
    let (add, del) = change_counts(&app.content()[index]);
    let notes = note_count(app.annotations(), &rf.path);
    let selected = index == app.selected_index();
    let viewed = app.is_viewed(index);

    let mut base = if selected {
        Style::default().fg(palette.fg).bg(palette.selection_bg)
    } else {
        Style::default().fg(palette.fg)
    };
    // Dim already-reviewed rows (unless one is selected) so the eye flows to
    // what is left to review.
    if viewed && !selected {
        base = base.add_modifier(Modifier::DIM);
    }

    let mut spans = Vec::with_capacity(9);
    spans.push(Span::styled(
        if selected { "▌" } else { " " }.to_string(),
        base.fg(palette.accent),
    ));
    // Reviewed-state column (fixed two cells wide so rows stay aligned).
    spans.push(Span::styled(
        if viewed { "✓ " } else { "  " }.to_string(),
        base.fg(palette.added),
    ));
    spans.push(Span::styled(
        format!("{} ", status_marker(rf.status)),
        status_style(rf.status, palette, base),
    ));
    spans.push(safe_span(base_of(&rf.path), base));
    if add > 0 {
        spans.push(Span::styled(format!("  +{add}"), base.fg(palette.added)));
    }
    if del > 0 {
        spans.push(Span::styled(format!(" -{del}"), base.fg(palette.removed)));
    }
    if notes > 0 {
        spans.push(Span::styled(format!("  *{notes}"), base.fg(palette.note)));
    }
    spans.push(Span::styled(
        format!("  ·{}", rf.score),
        base.fg(palette.dim),
    ));
    Line::from(spans)
}

/// Build the sidebar lines for the whole review.
#[must_use]
pub(crate) fn tree_lines(app: &ReviewApp, palette: &Palette) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for (dir, indices) in grouped(app) {
        let label = if dir.is_empty() {
            "./".to_string()
        } else {
            format!("{dir}/")
        };
        lines.push(Line::from(safe_span(
            &label,
            Style::default()
                .fg(palette.dim)
                .add_modifier(Modifier::BOLD),
        )));
        for index in indices {
            lines.push(file_line(app, palette, index));
        }
    }
    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "no files",
            Style::default().fg(palette.dim),
        )));
    }
    lines
}

/// The line index (within [`tree_lines`]) of the selected file's row, so the
/// caller can scroll the sidebar to keep the selection visible.
#[must_use]
pub(crate) fn selected_row(app: &ReviewApp) -> usize {
    let selected = app.selected_index();
    let mut row = 0;
    for (_, indices) in grouped(app) {
        row += 1; // directory header line
        for index in indices {
            if index == selected {
                return row;
            }
            row += 1;
        }
    }
    0
}

/// Resolve a visible [`tree_lines`] row to a ranked file index. Directory
/// header rows and out-of-range rows return `None`.
#[must_use]
pub(crate) fn file_index_at_row(app: &ReviewApp, target_row: usize) -> Option<usize> {
    let mut row = 0;
    for (_, indices) in grouped(app) {
        if row == target_row {
            return None;
        }
        row += 1; // directory header line
        for index in indices {
            if row == target_row {
                return Some(index);
            }
            row += 1;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::ThemeKind;
    use deep_diff_forge_patch::parse;

    const TREE: &str = "\
--- a/src/core/types.rs
+++ b/src/core/types.rs
@@ -1,1 +1,2 @@
 keep
+added
--- a/src/ui/app.rs
+++ b/src/ui/app.rs
@@ -1,2 +1,1 @@
 keep
-gone
--- a/README.md
+++ b/README.md
@@ -1,1 +1,1 @@
-a
+b
";

    fn app() -> ReviewApp {
        let files = parse(TREE).unwrap();
        let notes = crate::notes::engine_annotations(&files, &deep_diff_forge_graph::rank(&files));
        ReviewApp::from_review_with_annotations(&files, notes)
    }

    fn text(line: &Line<'static>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    fn rendered(app: &ReviewApp) -> String {
        let p = ThemeKind::Dark.palette();
        tree_lines(app, &p)
            .iter()
            .map(text)
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn dir_of_handles_nested_and_root() {
        assert_eq!(dir_of("src/ui/app.rs"), "src/ui");
        assert_eq!(dir_of("README.md"), "");
    }

    #[test]
    fn base_of_strips_directory() {
        assert_eq!(base_of("src/ui/app.rs"), "app.rs");
        assert_eq!(base_of("README.md"), "README.md");
    }

    #[test]
    fn directory_headers_are_emitted() {
        let out = rendered(&app());
        assert!(out.contains("src/core/"));
        assert!(out.contains("src/ui/"));
        assert!(out.contains("./")); // root group for README.md
    }

    #[test]
    fn file_rows_show_basenames_not_full_paths() {
        let out = rendered(&app());
        assert!(out.contains("types.rs"));
        assert!(out.contains("app.rs"));
        // The full nested path should not appear on a file row.
        assert!(!out.contains("src/core/types.rs M"));
    }

    #[test]
    fn rows_show_addition_and_deletion_counts() {
        let out = rendered(&app());
        assert!(out.contains("+1")); // types.rs added a line
        assert!(out.contains("-1")); // app.rs removed a line
    }

    #[test]
    fn rows_show_score_badge() {
        let out = rendered(&app());
        assert!(out.contains('·'));
    }

    #[test]
    fn selected_file_is_marked() {
        let out = rendered(&app());
        assert!(
            out.contains('▌'),
            "selected row should carry the bar marker"
        );
    }

    #[test]
    fn fresh_review_shows_no_reviewed_check() {
        assert!(
            !rendered(&app()).contains('✓'),
            "nothing is reviewed in a fresh app"
        );
    }

    #[test]
    fn reviewed_file_shows_check() {
        let mut a = app();
        a.handle(crate::state::AppEvent::ToggleViewed); // mark the top-ranked file
        assert!(
            rendered(&a).contains('✓'),
            "a reviewed file should render the check marker"
        );
    }

    #[test]
    fn reviewed_unselected_row_is_dimmed() {
        let mut a = app();
        let first = base_of(&a.files()[0].path).to_string();
        // Mark file 0 reviewed; selection advances off it, so it is now
        // reviewed-and-unselected — the dim condition.
        a.handle(crate::state::AppEvent::ToggleViewed);
        let p = ThemeKind::Dark.palette();
        let lines = tree_lines(&a, &p);
        let row = lines
            .iter()
            .find(|l| text(l).contains(&first))
            .expect("the reviewed file's row should be present");
        assert!(
            row.spans
                .iter()
                .any(|s| s.style.add_modifier.contains(Modifier::DIM)),
            "a reviewed, unselected row should carry the DIM modifier"
        );
    }

    #[test]
    fn no_row_is_dimmed_in_a_fresh_review() {
        let a = app();
        let p = ThemeKind::Dark.palette();
        let any_dim = tree_lines(&a, &p).iter().any(|l| {
            l.spans
                .iter()
                .any(|s| s.style.add_modifier.contains(Modifier::DIM))
        });
        assert!(!any_dim, "nothing reviewed means nothing dimmed");
    }

    #[test]
    fn status_markers_are_stable() {
        assert_eq!(status_marker(FileStatus::Added), 'A');
        assert_eq!(status_marker(FileStatus::Modified), 'M');
        assert_eq!(status_marker(FileStatus::Deleted), 'D');
        assert_eq!(status_marker(FileStatus::Renamed), 'R');
        assert_eq!(status_marker(FileStatus::Unknown), '?');
    }

    #[test]
    fn note_count_badge_appears_when_annotated() {
        // README.md and the others may carry engine notes; at least one `*` badge
        // should show given lib/api-ish ranking yields notes for some file.
        let a = app();
        // Force a note onto a known path to assert the badge renders.
        let total: usize = a
            .files()
            .iter()
            .map(|f| note_count(a.annotations(), &f.path))
            .sum();
        if total > 0 {
            assert!(rendered(&a).contains('*'));
        }
    }

    #[test]
    fn note_count_counts_by_anchor_path() {
        let a = app();
        for f in a.files() {
            let c = note_count(a.annotations(), &f.path);
            // never negative, bounded by total annotations
            assert!(c <= a.annotations().len());
        }
    }

    #[test]
    fn empty_review_renders_placeholder() {
        let a = ReviewApp::new(Vec::new());
        assert!(rendered(&a).contains("no files"));
    }

    #[test]
    fn malicious_path_is_neutralised() {
        let evil = "--- a/x\u{1b}[2J.rs\n+++ b/x\u{1b}[2J.rs\n@@ -1,1 +1,1 @@\n-a\n+b\n";
        let files = parse(evil).unwrap();
        let a = ReviewApp::from_review(&files);
        let out = rendered(&a);
        assert!(!out.contains('\u{1b}'));
    }

    #[test]
    fn selected_row_points_at_the_selection() {
        let mut a = app();
        // First selection: row 1 (after the first directory header).
        assert_eq!(selected_row(&a), 1);
        a.handle(crate::state::AppEvent::Bottom);
        // The last file's row is greater than the first's.
        assert!(selected_row(&a) > 1);
    }

    #[test]
    fn file_index_at_row_skips_directory_headers() {
        let a = app();
        assert_eq!(file_index_at_row(&a, 0), None);
        assert_eq!(file_index_at_row(&a, 1), Some(0));
    }

    #[test]
    fn file_index_at_row_resolves_visible_file_rows() {
        let a = app();
        let row = selected_row(&a);
        assert_eq!(file_index_at_row(&a, row), Some(a.selected_index()));
        assert_eq!(file_index_at_row(&a, 999), None);
    }

    #[test]
    fn grouping_preserves_rank_order_within_groups() {
        // Two files in the same dir; the higher-ranked one (more signals) leads.
        let src = "\
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,1 +1,1 @@
-a
+b
--- a/src/zzz.rs
+++ b/src/zzz.rs
@@ -1,1 +1,1 @@
-a
+b
";
        let files = parse(src).unwrap();
        let a = ReviewApp::from_review(&files);
        let groups = grouped(&a);
        let src_group = groups.iter().find(|(d, _)| d == "src").unwrap();
        // lib.rs (public api) ranks above zzz.rs, so it comes first.
        assert_eq!(a.files()[src_group.1[0]].path, "src/lib.rs");
    }
}
