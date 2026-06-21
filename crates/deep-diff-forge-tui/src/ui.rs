use crate::state::ReviewApp;
use ratatui::Frame;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Paragraph};

/// Render the review app into a frame: a ranked file sidebar and a detail pane.
pub fn render(frame: &mut Frame, app: &ReviewApp) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(frame.area());

    let mut rows = Vec::new();
    for (i, file) in app.files().iter().enumerate() {
        let marker = if i == app.selected_index() { '>' } else { ' ' };
        let text = format!("{marker} {:>3} {}", file.score, file.path);
        let style = if i == app.selected_index() {
            Style::default().add_modifier(Modifier::REVERSED)
        } else {
            Style::default()
        };
        rows.push(Line::styled(text, style));
    }
    if rows.is_empty() {
        rows.push(Line::from("no files"));
    }
    let sidebar = Paragraph::new(rows).block(
        Block::default()
            .borders(Borders::ALL)
            .title("Files (ranked)"),
    );
    frame.render_widget(sidebar, chunks[0]);

    let detail = Paragraph::new(detail_lines(app))
        .scroll((app.scroll(), 0))
        .block(Block::default().borders(Borders::ALL).title("Detail"));
    frame.render_widget(detail, chunks[1]);
}

fn detail_lines(app: &ReviewApp) -> Vec<Line<'static>> {
    match app.selected_file() {
        Some(file) => {
            let signals: Vec<&str> = file.signals.iter().map(|s| s.label()).collect();
            vec![
                Line::from(format!("path:    {}", file.path)),
                Line::from(format!("status:  {}", file.status.label())),
                Line::from(format!("score:   {}", file.score)),
                Line::from(format!("signals: {}", signals.join(","))),
                Line::from(format!("layout:  {}", app.layout().label())),
            ]
        }
        None => vec![Line::from("no files in review")],
    }
}

/// Render the app to plain text lines via a headless [`TestBackend`].
///
/// Used by tests and by the CLI `review --probe` mode so the render path is
/// exercisable without a live terminal. Fail-soft: returns empty on backend
/// error rather than panicking.
#[must_use]
pub fn render_to_lines(app: &ReviewApp, width: u16, height: u16) -> Vec<String> {
    let Ok(mut terminal) = Terminal::new(TestBackend::new(width, height)) else {
        return Vec::new();
    };
    if terminal.draw(|frame| render(frame, app)).is_err() {
        return Vec::new();
    }
    let buffer = terminal.backend().buffer();
    let mut lines = Vec::with_capacity(height as usize);
    for y in 0..height {
        let mut line = String::new();
        for x in 0..width {
            line.push_str(buffer[(x, y)].symbol());
        }
        lines.push(line.trim_end().to_string());
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AppEvent;
    use deep_diff_forge_patch::parse;

    const MULTI: &str = "\
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,1 +1,1 @@
-a
+b
--- a/src/other.rs
+++ b/src/other.rs
@@ -1,1 +1,1 @@
-a
+b
";

    fn app() -> ReviewApp {
        ReviewApp::from_review(&parse(MULTI).unwrap())
    }

    fn rendered(app: &ReviewApp) -> String {
        render_to_lines(app, 80, 12).join("\n")
    }

    #[test]
    fn renders_without_panicking() {
        let lines = render_to_lines(&app(), 80, 12);
        assert_eq!(lines.len(), 12);
    }

    #[test]
    fn sidebar_lists_files() {
        let out = rendered(&app());
        assert!(out.contains("src/lib.rs"));
        assert!(out.contains("src/other.rs"));
    }

    #[test]
    fn selection_marker_on_first_file() {
        let out = render_to_lines(&app(), 80, 12).join("\n");
        // The selected (first, public-API) row carries the '>' marker.
        assert!(out.contains("> "));
    }

    #[test]
    fn detail_shows_selected_path() {
        let out = rendered(&app());
        assert!(out.contains("path:"));
        assert!(out.contains("src/lib.rs"));
    }

    #[test]
    fn detail_shows_status_and_score() {
        let out = rendered(&app());
        assert!(out.contains("status:"));
        assert!(out.contains("modified"));
        assert!(out.contains("score:"));
    }

    #[test]
    fn detail_shows_layout_label() {
        let out = rendered(&app());
        assert!(out.contains("inline"));
    }

    #[test]
    fn toggling_layout_changes_detail() {
        let mut a = app();
        a.handle(AppEvent::ToggleLayout);
        assert!(rendered(&a).contains("side-by-side"));
    }

    #[test]
    fn titles_are_rendered() {
        let out = rendered(&app());
        assert!(out.contains("Files (ranked)"));
        assert!(out.contains("Detail"));
    }

    #[test]
    fn empty_review_renders_placeholder() {
        let a = ReviewApp::new(Vec::new());
        let out = render_to_lines(&a, 80, 12).join("\n");
        assert!(out.contains("no files"));
    }

    #[test]
    fn navigating_moves_the_marker_line() {
        let mut a = app();
        let before = render_to_lines(&a, 80, 12);
        a.handle(AppEvent::Next);
        let after = render_to_lines(&a, 80, 12);
        assert_ne!(before, after);
    }

    #[test]
    fn detail_reflects_selected_file_after_nav() {
        let mut a = app();
        a.handle(AppEvent::Next);
        let out = render_to_lines(&a, 80, 12).join("\n");
        assert!(out.contains("src/other.rs"));
    }

    #[test]
    fn tiny_area_does_not_panic() {
        let lines = render_to_lines(&app(), 4, 2);
        assert_eq!(lines.len(), 2);
    }
}
