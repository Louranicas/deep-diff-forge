//! Frame composition: assemble the chrome, the ranked file tree, and the diff
//! pane into one screen, plus a headless renderer for `review --probe`/tests.
//!
//! Layout is three vertical bands — a menu bar, the main area, a status bar —
//! and the main area splits into the file tree and the diff pane. The focused
//! pane gets an accent border; a `?` help card draws as a centred overlay. All
//! attacker-controlled text (paths, diff bodies, note bodies) is neutralised by
//! the child modules before it reaches a cell.

use crate::chrome::{help_lines, menu_bar, status_bar};
use crate::diffview::diff_document;
use crate::state::{Focus, Overlay, ReviewApp};
use crate::theme::Palette;
use crate::tree::{selected_row, tree_lines};
use ratatui::Frame;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
    Wrap,
};

/// Render the whole review cockpit into a frame.
pub fn render(frame: &mut Frame, app: &ReviewApp) {
    let palette = app.theme().palette();
    let area = frame.area();
    let bands = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(area);

    let chrome_bg = Style::default().bg(palette.menu_bg);
    frame.render_widget(
        Paragraph::new(menu_bar(app, &palette, usize::from(bands[0].width))).style(chrome_bg),
        bands[0],
    );

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(32), Constraint::Percentage(68)])
        .split(bands[1]);
    render_tree(frame, app, &palette, cols[0]);
    render_diff(frame, app, &palette, cols[1]);

    frame.render_widget(
        Paragraph::new(status_bar(app, &palette, usize::from(bands[2].width))).style(chrome_bg),
        bands[2],
    );

    if app.overlay() == Overlay::Help {
        render_help(frame, &palette, area);
    }
}

fn render_tree(frame: &mut Frame, app: &ReviewApp, palette: &Palette, area: Rect) {
    let border = if app.focus() == Focus::Sidebar {
        palette.accent
    } else {
        palette.border
    };
    let title = format!(" Files (ranked) · {} ", app.file_count());
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border))
        .title(Line::from(Span::styled(
            title,
            Style::default().fg(palette.accent),
        )));
    let inner_height = usize::from(area.height.saturating_sub(2));
    let scroll = keep_visible(selected_row(app), inner_height);
    frame.render_widget(
        Paragraph::new(tree_lines(app, palette))
            .block(block)
            .scroll((scroll, 0)),
        area,
    );
}

fn render_diff(frame: &mut Frame, app: &ReviewApp, palette: &Palette, area: Rect) {
    let border = if app.focus() == Focus::Diff {
        palette.accent
    } else {
        palette.border
    };
    let pos = if app.is_empty() {
        0
    } else {
        app.selected_index() + 1
    };
    let title = format!(
        " review · {}/{} files · {} ",
        pos,
        app.file_count(),
        app.layout().label()
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border))
        .title(Line::from(Span::styled(
            title,
            Style::default().fg(palette.accent),
        )));
    let inner_width = usize::from(area.width.saturating_sub(2));
    let inner_height = usize::from(area.height.saturating_sub(2));
    // One continuous document of all files; the selected file's section start is
    // the scroll anchor, and `app.scroll()` is the manual delta from there.
    let (lines, starts) = diff_document(app, palette, inner_width);
    let total = lines.len();
    let base = starts.get(app.selected_index()).copied().unwrap_or(0);
    let scroll = base.saturating_add(usize::from(app.scroll()));
    let scroll_u16 = u16::try_from(scroll.min(total.saturating_sub(1))).unwrap_or(u16::MAX);
    frame.render_widget(
        Paragraph::new(lines).block(block).scroll((scroll_u16, 0)),
        area,
    );
    // A scrollbar on the right border when the document overflows the viewport.
    if total > inner_height {
        let track = Rect {
            x: area.x,
            y: area.y + 1,
            width: area.width,
            height: area.height.saturating_sub(2),
        };
        let mut sb = ScrollbarState::new(total).position(scroll.min(total));
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .thumb_style(Style::default().fg(palette.accent))
                .track_style(Style::default().fg(palette.border))
                .begin_symbol(None)
                .end_symbol(None),
            track,
            &mut sb,
        );
    }
}

fn render_help(frame: &mut Frame, palette: &Palette, area: Rect) {
    let popup = centered(area, 62, 18);
    frame.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(palette.accent))
        .style(Style::default().bg(palette.menu_bg))
        .title(Line::from(Span::styled(
            " Help ",
            Style::default().fg(palette.accent),
        )));
    frame.render_widget(
        Paragraph::new(help_lines(palette))
            .block(block)
            .wrap(Wrap { trim: false }),
        popup,
    );
}

/// Scroll offset that keeps `row` within a viewport of `height` lines.
fn keep_visible(row: usize, height: usize) -> u16 {
    if height == 0 || row < height {
        return 0;
    }
    u16::try_from(row - height + 1).unwrap_or(u16::MAX)
}

/// A `w`×`h` rectangle centred in `area`, clamped to `area`.
fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
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
@@ -1,2 +1,2 @@
 fn keep() {}
-let old = 1;
+let new = 2;
--- a/src/other.rs
+++ b/src/other.rs
@@ -1,1 +1,1 @@
-a
+b
";

    fn app() -> ReviewApp {
        let files = parse(MULTI).unwrap();
        let notes = crate::notes::engine_annotations(&files, &deep_diff_forge_graph::rank(&files));
        ReviewApp::from_review_with_annotations(&files, notes)
    }

    fn screen(app: &ReviewApp) -> String {
        render_to_lines(app, 120, 30).join("\n")
    }

    #[test]
    fn renders_full_height_without_panicking() {
        assert_eq!(render_to_lines(&app(), 120, 30).len(), 30);
    }

    #[test]
    fn menu_bar_is_present() {
        let out = screen(&app());
        assert!(out.contains("File"));
        assert!(out.contains("Help"));
        assert!(out.contains("deep-diff-forge"));
    }

    #[test]
    fn sidebar_lists_files_by_basename() {
        let out = screen(&app());
        assert!(out.contains("Files (ranked)"));
        assert!(out.contains("lib.rs"));
        assert!(out.contains("other.rs"));
    }

    #[test]
    fn diff_pane_shows_selected_content() {
        let out = screen(&app());
        assert!(out.contains("let new = 2;"));
        assert!(out.contains("@@"));
    }

    #[test]
    fn status_bar_shows_quit_hint() {
        assert!(screen(&app()).contains("quit"));
    }

    #[test]
    fn side_by_side_changes_the_screen() {
        let mut a = app();
        let before = screen(&a);
        a.handle(AppEvent::ToggleLayout);
        let after = screen(&a);
        assert_ne!(before, after);
        assert!(after.contains('│'));
    }

    #[test]
    fn navigating_updates_the_diff_pane() {
        let mut a = app();
        let before = screen(&a);
        a.handle(AppEvent::Next);
        assert_ne!(before, screen(&a));
    }

    #[test]
    fn help_overlay_appears_on_toggle() {
        let mut a = app();
        assert!(!screen(&a).contains("review keys"));
        a.handle(AppEvent::ToggleHelp);
        assert!(screen(&a).contains("review keys"));
    }

    #[test]
    fn malicious_path_does_not_leak_escape() {
        let evil = "--- a/x\u{1b}[2J.rs\n+++ b/x\u{1b}[2J.rs\n@@ -1,1 +1,1 @@\n-a\n+\u{1b}[2Jb\n";
        let files = parse(evil).unwrap();
        let a = ReviewApp::from_review(&files);
        let out = render_to_lines(&a, 120, 24).join("\n");
        assert!(!out.contains('\u{1b}'));
    }

    #[test]
    fn empty_review_renders_safely() {
        let a = ReviewApp::new(Vec::new());
        let out = render_to_lines(&a, 80, 20);
        assert_eq!(out.len(), 20);
        assert!(out.join("\n").contains("no file"));
    }

    #[test]
    fn tiny_area_does_not_panic() {
        assert_eq!(render_to_lines(&app(), 4, 3).len(), 3);
    }

    #[test]
    fn keep_visible_scrolls_only_when_needed() {
        assert_eq!(keep_visible(3, 10), 0);
        assert_eq!(keep_visible(10, 10), 1);
        assert_eq!(keep_visible(0, 0), 0);
    }

    #[test]
    fn centered_rect_fits_inside_area() {
        let area = Rect {
            x: 0,
            y: 0,
            width: 100,
            height: 40,
        };
        let r = centered(area, 60, 18);
        assert!(r.x + r.width <= area.width);
        assert!(r.y + r.height <= area.height);
    }

    #[test]
    fn centered_rect_clamps_to_small_area() {
        let area = Rect {
            x: 0,
            y: 0,
            width: 10,
            height: 5,
        };
        let r = centered(area, 60, 18);
        assert_eq!(r.width, 10);
        assert_eq!(r.height, 5);
    }
}
