//! Frame composition: assemble the chrome, the ranked file tree, and the diff
//! pane into one screen, plus a headless renderer for `review --probe`/tests.
//!
//! Layout is three vertical bands — a menu bar, the main area, a status bar —
//! and the main area splits into the file tree and the diff pane. The focused
//! pane gets an accent border; a `?` help card draws as a centred overlay. All
//! attacker-controlled text (paths, diff bodies, note bodies) is neutralised by
//! the child modules before it reaches a cell.

use crate::chrome::{help_lines, menu_bar, status_bar};
use crate::command::Command;
use crate::diffview::diff_window;
use crate::paint::safe_span;
use crate::state::{Focus, Overlay, ReviewApp};
use crate::theme::Palette;
use crate::tree::{selected_row, tree_window};
use ratatui::Frame;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::{Constraint, Direction, Layout, Margin, Position, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
    Wrap,
};

/// The cockpit's four rectangles in a terminal `area`: the menu bar, the file tree, the
/// diff pane and the status bar. Rendering and mouse hit-testing both use it, so a click
/// resolves against the panes the reviewer sees.
struct Panes {
    menu: Rect,
    tree: Rect,
    diff: Rect,
    status: Rect,
}

impl Panes {
    fn of(area: Rect) -> Self {
        let bands = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Min(0),
                Constraint::Length(1),
            ])
            .split(area);
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(32), Constraint::Percentage(68)])
            .split(bands[1]);
        Self {
            menu: bands[0],
            tree: cols[0],
            diff: cols[1],
            status: bands[2],
        }
    }
}

/// Where a frame drew its panes and which tree row it showed first, so a mouse click maps to
/// what the reviewer clicked on. Build it from the same app state and area as the frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HitMap {
    menu: Rect,
    tree_rows: Rect,
    tree_scroll: usize,
    diff: Rect,
}

/// What a click at one screen cell lands on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Hit {
    /// A menu-bar column.
    Menu(u16),
    /// A row of the whole file tree (not a screen row).
    TreeRow(usize),
    /// Inside the diff pane's border.
    Diff,
    /// Borders, the status bar, or outside the frame.
    Nothing,
}

impl HitMap {
    /// The hit map of a frame rendered from `app` into `area`.
    #[must_use]
    pub fn of(app: &ReviewApp, area: Rect) -> Self {
        let panes = Panes::of(area);
        let tree_rows = panes.tree.inner(Margin::new(1, 1));
        Self {
            menu: panes.menu,
            tree_rows,
            tree_scroll: tree_scroll(app, usize::from(tree_rows.height)),
            diff: panes.diff.inner(Margin::new(1, 1)),
        }
    }

    pub(crate) fn hit(&self, column: u16, row: u16) -> Hit {
        let at = Position::new(column, row);
        if self.menu.contains(at) {
            Hit::Menu(column - self.menu.x)
        } else if self.tree_rows.contains(at) {
            Hit::TreeRow(self.tree_scroll + usize::from(row - self.tree_rows.y))
        } else if self.diff.contains(at) {
            Hit::Diff
        } else {
            Hit::Nothing
        }
    }
}

/// Render the whole review cockpit into a frame.
pub fn render(frame: &mut Frame, app: &ReviewApp) {
    let palette = app.theme().palette();
    let area = frame.area();
    let panes = Panes::of(area);

    let chrome_bg = Style::default().bg(palette.menu_bg);
    frame.render_widget(
        Paragraph::new(menu_bar(app, &palette, usize::from(panes.menu.width))).style(chrome_bg),
        panes.menu,
    );

    render_tree(frame, app, &palette, panes.tree);
    render_diff(frame, app, &palette, panes.diff);

    frame.render_widget(
        Paragraph::new(status_bar(app, &palette, usize::from(panes.status.width))).style(chrome_bg),
        panes.status,
    );

    match app.overlay() {
        Overlay::Help => render_help(frame, &palette, area),
        Overlay::Palette => render_palette(frame, app, &palette, area),
        Overlay::Panel => render_panel(frame, app, &palette, area),
        Overlay::None => {}
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
    let scroll = tree_scroll(app, inner_height);
    frame.render_widget(
        Paragraph::new(tree_window(
            app,
            palette,
            scroll..scroll.saturating_add(inner_height),
        ))
        .block(block),
        area,
    );
}

/// The first tree row a viewport of `height` rows shows: the selection stays visible.
fn tree_scroll(app: &ReviewApp, height: usize) -> usize {
    keep_visible(selected_row(app), height)
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
    // the scroll anchor, and `app.scroll()` is the manual delta from there. Only the
    // visible rows are built, and they are drawn unscrolled: ratatui's paragraph offset
    // is a `u16`, which pinned every view past document row 65,535 there.
    let window = diff_window(app, palette, inner_width, app.scroll(), inner_height);
    let total = window.total;
    frame.render_widget(Paragraph::new(window.lines).block(block), area);
    // A scrollbar on the right border when the document overflows the viewport.
    if total > inner_height {
        let track = Rect {
            x: area.x,
            y: area.y + 1,
            width: area.width,
            height: area.height.saturating_sub(2),
        };
        let mut sb = ScrollbarState::new(total).position(window.offset);
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
    let popup = centered(area, 86, 28);
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

/// The command palette: pick an engine capability to run.
fn render_palette(frame: &mut Frame, app: &ReviewApp, palette: &Palette, area: Rect) {
    let commands = Command::all();
    let popup = centered(area, 88, 14);
    frame.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(palette.accent))
        .style(Style::default().bg(palette.menu_bg))
        .title(Line::from(Span::styled(
            " Command palette ",
            Style::default().fg(palette.accent),
        )))
        .title_bottom(Line::from(Span::styled(
            " ↑/↓ move · Enter run · Esc close ",
            Style::default().fg(palette.dim),
        )));
    let mut lines = Vec::with_capacity(commands.len());
    for (i, command) in commands.iter().enumerate() {
        let selected = i == app.palette_index();
        let (marker, label_style) = if selected {
            (
                "▶ ",
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(ratatui::style::Modifier::BOLD),
            )
        } else {
            ("  ", Style::default().fg(palette.fg))
        };
        lines.push(Line::from(vec![
            Span::styled(marker, Style::default().fg(palette.accent)),
            Span::styled(
                format!("{:<6}", command.menu()),
                Style::default().fg(palette.dim),
            ),
            Span::styled(format!("{:<13}", command.label()), label_style),
            Span::styled(
                format!(" [{}] {}", command.shortcut(), command.hint()),
                Style::default().fg(palette.dim),
            ),
        ]));
    }
    frame.render_widget(Paragraph::new(lines).block(block), popup);
}

/// The result panel of the last-run command (scrollable, terminal-safe).
fn render_panel(frame: &mut Frame, app: &ReviewApp, palette: &Palette, area: Rect) {
    let popup = centered(
        area,
        area.width.saturating_sub(8),
        area.height.saturating_sub(4),
    );
    frame.render_widget(Clear, popup);
    let out = app.panel();
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(palette.accent))
        .style(Style::default().bg(palette.menu_bg))
        .title(Line::from(safe_span(
            &format!(" {} ", out.title),
            Style::default().fg(palette.accent),
        )))
        .title_bottom(Line::from(Span::styled(
            " ↑/↓ scroll · Esc back · q quit ",
            Style::default().fg(palette.dim),
        )));
    // Each output line wraps at the panel's inner width: a note longer than the panel used to
    // be cut off, losing the end of a security warning (bug hunt 2026-10-28, H4-08). Escaping
    // comes first, so a wrapped row never outgrows the panel.
    let inner_width = usize::from(popup.width.saturating_sub(2));
    let lines: Vec<Line<'static>> = out
        .lines
        .iter()
        .flat_map(|l| {
            deep_diff_forge_projection::wrap_cells(
                &deep_diff_forge_core::display_safe(l),
                inner_width,
            )
        })
        .map(|row| Line::from(safe_span(&row, Style::default().fg(palette.fg))))
        .collect();
    let total = lines.len();
    let inner_height = usize::from(popup.height.saturating_sub(2));
    let offset = app.panel_scroll().min(total.saturating_sub(1));
    let visible: Vec<Line<'static>> = lines.into_iter().skip(offset).take(inner_height).collect();
    frame.render_widget(Paragraph::new(visible).block(block), popup);
    if total > inner_height {
        let track = Rect {
            x: popup.x,
            y: popup.y + 1,
            width: popup.width,
            height: popup.height.saturating_sub(2),
        };
        let mut sb = ScrollbarState::new(total).position(offset);
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

/// Scroll offset that keeps `row` within a viewport of `height` lines.
fn keep_visible(row: usize, height: usize) -> usize {
    if height == 0 || row < height {
        return 0;
    }
    row - height + 1
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
    let mut terminal = match Terminal::new(TestBackend::new(width, height)) {
        Ok(terminal) => terminal,
        Err(infallible) => match infallible {},
    };
    if terminal.draw(|frame| render(frame, app)).is_err() {
        return Vec::new();
    }
    let buffer = terminal.backend().buffer();
    let mut lines = Vec::with_capacity(height as usize);
    for y in 0..height {
        let mut line = String::new();
        let mut x = 0;
        while x < width {
            let symbol = buffer[(x, y)].symbol();
            line.push_str(symbol);
            // A wide character fills its cell and resets the next to a space; a terminal shows
            // the character once, so the covered cell is skipped (bug hunt 2026-10-28, H4-09).
            let covered = u16::try_from(deep_diff_forge_projection::cell_width(symbol))
                .unwrap_or(1)
                .max(1);
            x = x.saturating_add(covered);
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
    use std::fmt::Write as _;

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

    /// Drive a real key through the full input→event path the binary uses
    /// (`event_loop` does exactly `app.handle(map_key(key))`).
    fn press(app: &mut ReviewApp, c: char) {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle(crate::input::map_key(KeyEvent::new(
            KeyCode::Char(c),
            KeyModifiers::NONE,
        )));
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

    /// Bug hunt 2026-10-28, H4-09: a probe printed a space after each wide character ("界 界"),
    /// so its lines were wider than the terminal they describe.
    #[test]
    fn wide_characters_probe_as_a_terminal_shows_them() {
        let wide = "界".repeat(40);
        let files = parse(&format!(
            "--- a/w.txt\n+++ b/w.txt\n@@ -1 +1 @@\n-a\n+{wide}\n"
        ))
        .unwrap();
        let out = render_to_lines(&ReviewApp::from_review(&files), 63, 20);
        assert!(out.iter().any(|l| l.contains("界界")), "{out:#?}");
        assert!(!out.iter().any(|l| l.contains("界 界")), "{out:#?}");
        for line in &out {
            assert!(
                deep_diff_forge_projection::cell_width(line) <= 63,
                "{line:?}"
            );
        }
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

    // --- end-to-end coverage of the "reviewed" flag, through the real key path
    //     and the full render pipeline (the binary's exact behaviour, no TTY) ---

    #[test]
    fn fresh_review_renders_no_reviewed_check() {
        assert!(
            !screen(&app()).contains('✓'),
            "nothing is reviewed in a fresh cockpit"
        );
    }

    #[test]
    fn pressing_v_marks_reviewed_and_renders_a_check() {
        let mut a = app();
        press(&mut a, 'v');
        assert!(
            screen(&a).contains('✓'),
            "v should mark the file reviewed and the sidebar should render a check"
        );
    }

    #[test]
    fn pressing_space_also_marks_reviewed() {
        let mut a = app();
        press(&mut a, ' ');
        assert!(screen(&a).contains('✓'), "Space is an alias for v");
    }

    #[test]
    fn marking_every_file_renders_one_check_per_file() {
        let mut a = app();
        let n = a.file_count();
        for _ in 0..n {
            press(&mut a, 'v'); // mark + advance; advance clamps on the last file
        }
        assert_eq!(
            screen(&a).matches('✓').count(),
            n,
            "every reviewed file shows exactly one check"
        );
    }

    #[test]
    fn status_bar_shows_viewed_progress_in_a_wide_frame() {
        let mut a = app();
        // A wide frame so the right-aligned state is not truncated.
        let before = render_to_lines(&a, 200, 30).join("\n");
        assert!(before.contains("viewed:0/2"), "fresh progress is 0 of 2");
        press(&mut a, 'v');
        let after = render_to_lines(&a, 200, 30).join("\n");
        assert!(
            after.contains("viewed:1/2"),
            "marking one updates the count"
        );
    }

    #[test]
    fn reviewed_check_survives_navigation() {
        let mut a = app();
        press(&mut a, 'v'); // mark file 0; selection advances to file 1
        press(&mut a, 'k'); // Prev → back to file 0
        press(&mut a, 'j'); // Next → forward to file 1
        assert!(
            screen(&a).contains('✓'),
            "reviewed state is sticky across navigation re-renders"
        );
    }

    #[test]
    fn unmarking_clears_the_only_check() {
        let mut a = app();
        press(&mut a, 'v'); // mark file 0 (advance to 1)
        assert!(screen(&a).contains('✓'));
        press(&mut a, 'k'); // back to file 0
        press(&mut a, 'v'); // un-mark file 0
        assert!(
            !screen(&a).contains('✓'),
            "un-marking the only reviewed file removes the check"
        );
    }

    #[test]
    fn help_overlay_lists_the_reviewed_binding() {
        let mut a = app();
        press(&mut a, '?'); // ToggleHelp
        assert!(
            screen(&a).contains("reviewed"),
            "the help card documents the reviewed key"
        );
    }

    #[test]
    fn viewed_key_is_inert_while_palette_is_open() {
        let mut a = app();
        press(&mut a, ':'); // open the palette (modal)
        press(&mut a, 'v'); // routed to the palette, not the review
        // Close the palette and confirm nothing was marked.
        press(&mut a, ':');
        assert!(
            !screen(&a).contains('✓'),
            "the reviewed toggle must not fire under an open overlay"
        );
    }

    /// A review of `n` one-line changes, `f00.txt` onward, all in one directory.
    fn many(n: usize) -> ReviewApp {
        let mut src = String::new();
        for i in 0..n {
            let _ = writeln!(
                src,
                "--- a/f{i:02}.txt\n+++ b/f{i:02}.txt\n@@ -1,1 +1,1 @@\n-old{i:02}\n+new{i:02}"
            );
        }
        ReviewApp::from_review(&parse(&src).unwrap())
    }

    fn click(app: &mut ReviewApp, width: u16, height: u16, column: u16, row: u16) {
        use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
        let hits = HitMap::of(app, Rect::new(0, 0, width, height));
        let down = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: KeyModifiers::NONE,
        };
        app.handle(crate::input::map_mouse(down, &hits));
    }

    fn selected_path(app: &ReviewApp) -> String {
        app.selected_file().unwrap().path.clone()
    }

    /// Bug hunt 2026-11-01: after `G` scrolled the tree, a click on the row showing `f23.txt`
    /// selected `f02.txt`, the row at that height of the unscrolled tree.
    #[test]
    fn a_click_on_a_scrolled_tree_selects_the_file_shown_there() {
        let mut a = many(40);
        a.handle(AppEvent::Bottom);
        let frame = render_to_lines(&a, 120, 24);
        let row = frame
            .iter()
            .position(|line| line.contains("M f23.txt"))
            .expect("f23 is on screen after G");
        click(&mut a, 120, 24, 5, u16::try_from(row).unwrap());
        assert_eq!(selected_path(&a), "f23.txt");
        // Every visible file row selects the file it shows.
        for (row, line) in frame.iter().enumerate().skip(2).take(21) {
            let Some(at) = line.find(" f") else { continue };
            let name = &line[at + 1..at + 8];
            if !name.starts_with('f') || name.as_bytes()[3] != b'.' {
                continue;
            }
            a.handle(AppEvent::Bottom);
            click(&mut a, 120, 24, 5, u16::try_from(row).unwrap());
            assert_eq!(selected_path(&a), name, "row {row}: {line}");
        }
    }

    /// Bug hunt 2026-11-01: at 80 columns the tree is 25 columns wide, and a click at column 35,
    /// inside the diff, changed the selection; at 200 columns a click on the tree at column 50
    /// focused the diff instead of selecting a file.
    #[test]
    fn a_click_lands_in_the_pane_drawn_at_that_column() {
        let mut a = many(40);
        for _ in 0..5 {
            a.handle(AppEvent::Next);
        }
        let before = selected_path(&a);
        click(&mut a, 80, 24, 35, 5);
        assert_eq!(selected_path(&a), before);
        assert_eq!(a.focus(), Focus::Diff);

        let mut wide = many(40);
        let frame = render_to_lines(&wide, 200, 24);
        let row = frame.iter().position(|l| l.contains("M f03.txt")).unwrap();
        click(&mut wide, 200, 24, 50, u16::try_from(row).unwrap());
        assert_eq!(selected_path(&wide), "f03.txt");
        assert_eq!(wide.focus(), Focus::Sidebar);
    }

    #[test]
    fn borders_and_the_status_bar_are_not_click_targets() {
        let mut a = many(40);
        a.handle(AppEvent::FocusDiff);
        let before = selected_path(&a);
        for (column, row) in [(0, 5), (5, 1), (5, 23), (60, 23), (130, 5), (5, 30)] {
            click(&mut a, 120, 24, column, row);
            assert_eq!(selected_path(&a), before, "({column}, {row})");
            assert_eq!(a.focus(), Focus::Diff, "({column}, {row})");
        }
    }

    /// Review 2026-11-01: the hit map used the diff pane's whole rectangle, so a click on the
    /// diff pane's border focused the diff, though a click on any border should do nothing.
    #[test]
    fn the_diff_panes_border_is_not_a_click_target() {
        let area = Rect::new(0, 0, 120, 24);
        let diff = Panes::of(area).diff;
        let hits = HitMap::of(&many(3), area);
        let (left, right) = (diff.x, diff.right() - 1);
        let (top, bottom) = (diff.y, diff.bottom() - 1);
        for column in left..=right {
            assert_eq!(hits.hit(column, top), Hit::Nothing, "top ({column}, {top})");
            assert_eq!(
                hits.hit(column, bottom),
                Hit::Nothing,
                "bottom ({column}, {bottom})"
            );
        }
        for row in top..=bottom {
            assert_eq!(hits.hit(left, row), Hit::Nothing, "left ({left}, {row})");
            assert_eq!(hits.hit(right, row), Hit::Nothing, "right ({right}, {row})");
        }
        assert_eq!(hits.hit(left + 1, top + 1), Hit::Diff);
        assert_eq!(hits.hit(right - 1, bottom - 1), Hit::Diff);
    }

    #[test]
    fn hit_map_measures_from_its_area() {
        let a = many(3);
        let hits = HitMap::of(&a, Rect::new(4, 3, 120, 24));
        assert_eq!(hits.hit(4, 3), Hit::Menu(0));
        assert_eq!(hits.hit(10, 3), Hit::Menu(6));
        assert_eq!(hits.hit(6, 5), Hit::TreeRow(0));
        assert_eq!(hits.hit(6, 7), Hit::TreeRow(2));
        assert_eq!(hits.hit(4, 5), Hit::Nothing);
        assert_eq!(hits.hit(100, 10), Hit::Diff);
        assert_eq!(hits.hit(2, 10), Hit::Nothing);
    }

    /// Bug hunt 2026-11-01: the diff pane scrolled through ratatui's `u16` offset, so a file
    /// whose section started past document row 65,535 was never shown: selecting it drew rows
    /// of the file before it under its title.
    #[test]
    fn a_file_past_row_65535_is_shown_when_selected() {
        let n = 70_000;
        let mut src = format!("--- /dev/null\n+++ b/a_big.txt\n@@ -0,0 +1,{n} @@\n");
        for i in 0..n {
            let _ = writeln!(src, "+bigline {i}");
        }
        src.push_str(
            "--- a/z_small.txt\n+++ b/z_small.txt\n@@ -1,1 +1,1 @@\n-SMALL_OLD_MARKER\n+SMALL_NEW_MARKER\n",
        );
        let mut a = ReviewApp::from_review(&parse(&src).unwrap());
        a.handle(AppEvent::Next);
        let out = render_to_lines(&a, 120, 24).join("\n");
        assert!(out.contains("review · 2/2 files"), "{out}");
        assert!(out.contains("z_small.txt  [modified]"), "{out}");
        assert!(out.contains("SMALL_OLD_MARKER"), "{out}");
        assert!(out.contains("SMALL_NEW_MARKER"), "{out}");
        assert!(!out.contains("bigline"), "{out}");
        // Scrolling down from the big file's start reaches its rows past 65,535 too.
        let mut big = ReviewApp::from_review(&parse(&src).unwrap());
        for _ in 0..66_000 {
            big.handle(AppEvent::ScrollDown);
        }
        let shown: Vec<usize> = render_to_lines(&big, 120, 24)
            .iter()
            .filter_map(|l| l.split("bigline ").nth(1))
            .filter_map(|n| n.split_whitespace().next()?.parse().ok())
            .collect();
        assert!(shown.len() > 10, "{shown:?}");
        assert!(shown.iter().all(|&n| n > 65_900), "{shown:?}");
    }

    /// The tree scrolls without the `u16` cap as well: its offset is a row of the whole tree,
    /// and only the rows in the window are built.
    #[test]
    fn tree_offsets_are_not_capped_and_the_window_is_exact() {
        assert_eq!(keep_visible(70_000, 20), 69_981);
        let a = many(10);
        let window = crate::tree::tree_window(&a, &a.theme().palette(), 3..5);
        let text: Vec<String> = window
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect();
        assert_eq!(text.len(), 2, "{text:?}");
        assert!(
            text[0].contains("f02.txt") && text[1].contains("f03.txt"),
            "{text:?}"
        );
    }

    #[test]
    fn panel_scrolls_by_its_offset() {
        use crate::command::Command;
        let mut a = many(30);
        let rank = Command::all()
            .iter()
            .position(|c| *c == Command::Rank)
            .unwrap();
        a.handle(AppEvent::OpenPalette);
        for _ in 0..rank {
            a.handle(AppEvent::Next);
        }
        a.handle(AppEvent::Select);
        assert_eq!(a.overlay(), Overlay::Panel);
        let top = render_to_lines(&a, 120, 24);
        a.handle(AppEvent::ScrollDown);
        a.handle(AppEvent::ScrollDown);
        let scrolled = render_to_lines(&a, 120, 24);
        // The panel's third row is now its first.
        let body = |row: &str| row.chars().skip(5).take(100).collect::<String>();
        assert_ne!(body(&top[3]), body(&top[5]), "{top:#?}");
        assert_eq!(body(&scrolled[3]), body(&top[5]), "{top:#?}\n{scrolled:#?}");
    }
}
