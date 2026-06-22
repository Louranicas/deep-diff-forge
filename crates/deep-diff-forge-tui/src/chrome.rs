//! Window chrome: the top menu bar, the bottom status bar, and the help card.
//!
//! The menu labels are a visual affordance; the real controls are the
//! single-key bindings shown in the status bar and spelled out in the help
//! overlay. Totals (`+A -B`, file count) are derived from the live model so the
//! header always reflects what is on screen.

use crate::state::ReviewApp;
use crate::theme::Palette;
use deep_diff_forge_graph::change_counts;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

const MENUS: &str = "File   View   Navigate   Theme   Agent   Help";

fn total_counts(app: &ReviewApp) -> (usize, usize) {
    app.content().iter().fold((0, 0), |(a, d), file| {
        let (fa, fd) = change_counts(file);
        (a + fa, d + fd)
    })
}

fn spacer(width: usize, left: usize, right: usize) -> String {
    " ".repeat(width.saturating_sub(left + right).max(1))
}

/// The top menu bar line. The caller paints the row background.
#[must_use]
pub(crate) fn menu_bar(app: &ReviewApp, palette: &Palette, width: usize) -> Line<'static> {
    let (add, del) = total_counts(app);
    let title = format!(
        "deep-diff-forge · review · {} file(s) · +{add} -{del}",
        app.file_count()
    );
    let gap = spacer(width, MENUS.chars().count(), title.chars().count());
    Line::from(vec![
        Span::styled(
            MENUS,
            Style::default()
                .fg(palette.menu_fg)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(gap),
        Span::styled(title, Style::default().fg(palette.accent)),
    ])
}

fn on_off(flag: bool) -> &'static str {
    if flag { "on" } else { "off" }
}

/// The bottom status bar line: keybind hints on the left, live view state on
/// the right.
#[must_use]
pub(crate) fn status_bar(app: &ReviewApp, palette: &Palette, width: usize) -> Line<'static> {
    let hints =
        "j/k move · v viewed · s layout · z fold · n notes · T theme · : cmds · ? help · q quit";
    let position = if app.is_empty() {
        0
    } else {
        app.selected_index() + 1
    };
    let state = format!(
        "{} · {}/{} · viewed:{}/{} · {} · fold:{} · notes:{}",
        app.layout().label(),
        position,
        app.file_count(),
        app.viewed_count(),
        app.file_count(),
        app.theme().label(),
        on_off(app.fold()),
        on_off(app.show_notes()),
    );
    let gap = spacer(width, hints.chars().count(), state.chars().count());
    Line::from(vec![
        Span::styled(hints, Style::default().fg(palette.dim)),
        Span::raw(gap),
        Span::styled(state, Style::default().fg(palette.accent)),
    ])
}

/// The help-overlay body: one line per binding.
#[must_use]
pub(crate) fn help_lines(palette: &Palette) -> Vec<Line<'static>> {
    let key = Style::default()
        .fg(palette.accent)
        .add_modifier(Modifier::BOLD);
    let desc = Style::default().fg(palette.fg);
    let bindings = [
        ("j / k / ↓ ↑", "select next / previous file (by rank)"),
        ("g / G", "jump to first / last file"),
        ("← / →  h / l", "focus the file tree / the diff"),
        ("PgDn / PgUp", "scroll the diff (also Ctrl-d / Ctrl-u)"),
        ("s / Tab", "toggle inline ↔ side-by-side"),
        ("z", "fold / unfold long unchanged context"),
        ("n", "show / hide inline agent notes"),
        ("v / Space", "mark file reviewed (advances to the next)"),
        ("T", "cycle colour theme"),
        (":", "command palette (rank, cluster, json, …)"),
        ("Enter", "run the selected palette command"),
        ("? / Esc", "toggle help / dismiss overlay"),
        ("q", "quit the review"),
    ];
    let mut lines = vec![
        Line::from(Span::styled(
            "deep-diff-forge — review keys",
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        )),
        Line::default(),
    ];
    for (k, d) in bindings {
        lines.push(Line::from(vec![
            Span::styled(format!("  {k:<14}"), key),
            Span::styled(format!("  {d}"), desc),
        ]));
    }
    lines.push(Line::default());
    lines.push(Line::from(Span::styled(
        "  menu labels are visual; the keys above do the work.",
        Style::default().fg(palette.dim),
    )));
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::ReviewApp;
    use crate::theme::ThemeKind;
    use deep_diff_forge_patch::parse;

    fn app() -> ReviewApp {
        let files =
            parse("--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,2 @@\n keep\n+added\n").unwrap();
        ReviewApp::from_review(&files)
    }

    fn text(line: &Line<'static>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn menu_bar_lists_all_menus() {
        let out = text(&menu_bar(&app(), &ThemeKind::Dark.palette(), 120));
        for menu in ["File", "View", "Navigate", "Theme", "Agent", "Help"] {
            assert!(out.contains(menu), "menu missing: {menu}");
        }
    }

    #[test]
    fn menu_bar_shows_totals() {
        let out = text(&menu_bar(&app(), &ThemeKind::Dark.palette(), 120));
        assert!(out.contains("1 file(s)"));
        assert!(out.contains("+1"));
    }

    #[test]
    fn status_bar_shows_keys_and_state() {
        let out = text(&status_bar(&app(), &ThemeKind::Dark.palette(), 140));
        assert!(out.contains("layout"));
        assert!(out.contains("quit"));
        assert!(out.contains("inline"));
        assert!(out.contains("dark"));
        assert!(out.contains("1/1"));
    }

    #[test]
    fn status_bar_reflects_toggles() {
        let mut a = app();
        a.handle(crate::state::AppEvent::ToggleNotes);
        let out = text(&status_bar(&a, &ThemeKind::Dark.palette(), 140));
        assert!(out.contains("notes:off"));
    }

    #[test]
    fn empty_app_status_is_zero_of_zero() {
        let a = ReviewApp::new(Vec::new());
        let out = text(&status_bar(&a, &ThemeKind::Dark.palette(), 140));
        assert!(out.contains("0/0"));
    }

    #[test]
    fn status_bar_shows_review_progress() {
        let out = text(&status_bar(&app(), &ThemeKind::Dark.palette(), 160));
        assert!(
            out.contains("viewed:0/1"),
            "fresh review has nothing viewed"
        );
    }

    #[test]
    fn status_bar_review_progress_updates() {
        let mut a = app();
        a.handle(crate::state::AppEvent::ToggleViewed);
        let out = text(&status_bar(&a, &ThemeKind::Dark.palette(), 160));
        assert!(out.contains("viewed:1/1"), "marking updates the progress");
    }

    #[test]
    fn help_lists_review_key() {
        let p = ThemeKind::Dark.palette();
        let out: String = help_lines(&p)
            .iter()
            .map(text)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(out.contains("reviewed"));
    }

    #[test]
    fn help_lists_the_core_keys() {
        let p = ThemeKind::Dark.palette();
        let out: String = help_lines(&p)
            .iter()
            .map(text)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(out.contains("side-by-side"));
        assert!(out.contains("theme"));
        assert!(out.contains("fold"));
        assert!(out.contains("quit"));
    }

    #[test]
    fn spacer_never_zero() {
        assert_eq!(spacer(10, 8, 8).chars().count(), 1);
        assert_eq!(spacer(20, 5, 5).chars().count(), 10);
    }

    #[test]
    fn on_off_labels() {
        assert_eq!(on_off(true), "on");
        assert_eq!(on_off(false), "off");
    }
}
