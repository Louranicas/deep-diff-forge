use crate::chrome::MENUS;
use crate::state::AppEvent;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

/// Map a key event to a semantic [`AppEvent`].
///
/// Bindings are vi-style with arrow-key equivalents; unrecognized keys map to
/// [`AppEvent::None`] so the event loop simply ignores them.
#[must_use]
pub fn map_key(key: KeyEvent) -> AppEvent {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Char('q') => AppEvent::Quit,
        KeyCode::Esc => AppEvent::Cancel,
        KeyCode::Enter => AppEvent::Select,
        KeyCode::Char(':') => AppEvent::OpenPalette,
        KeyCode::Char('j') | KeyCode::Down => AppEvent::Next,
        KeyCode::Char('k') | KeyCode::Up => AppEvent::Prev,
        KeyCode::Char('t' | 's') | KeyCode::Tab => AppEvent::ToggleLayout,
        KeyCode::Char('z') => AppEvent::ToggleFold,
        KeyCode::Char('w') => AppEvent::ToggleWrap,
        KeyCode::Char('n') => AppEvent::ToggleNotes,
        KeyCode::Char('v' | ' ') => AppEvent::ToggleViewed,
        KeyCode::Char('T') => AppEvent::CycleTheme,
        KeyCode::Char('?') => AppEvent::ToggleHelp,
        KeyCode::Char('h') | KeyCode::Left => AppEvent::FocusSidebar,
        KeyCode::Char('l') | KeyCode::Right => AppEvent::FocusDiff,
        KeyCode::Char('d') if ctrl => AppEvent::ScrollDown,
        KeyCode::Char('u') if ctrl => AppEvent::ScrollUp,
        KeyCode::PageDown => AppEvent::ScrollDown,
        KeyCode::PageUp => AppEvent::ScrollUp,
        KeyCode::Char('g') | KeyCode::Home => AppEvent::Top,
        KeyCode::Char('G') | KeyCode::End => AppEvent::Bottom,
        _ => AppEvent::None,
    }
}

/// Map a mouse event to a semantic [`AppEvent`].
///
/// The terminal event loop does not know the rendered pane rectangles, so this
/// mapper intentionally handles only stable, layout-independent gestures:
/// wheel scrolling and coarse left/right focus clicks. Rich hit testing can be
/// layered on later without changing the state machine.
#[must_use]
pub fn map_mouse(mouse: MouseEvent) -> AppEvent {
    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) if mouse.row == 0 => map_menu_click(mouse.column),
        MouseEventKind::ScrollUp => AppEvent::ScrollUp,
        MouseEventKind::ScrollDown => AppEvent::ScrollDown,
        MouseEventKind::Down(MouseButton::Left) if mouse.column < 40 => {
            AppEvent::SelectTreeRow(usize::from(mouse.row.saturating_sub(2)))
        }
        MouseEventKind::Down(MouseButton::Left) => AppEvent::FocusDiff,
        _ => AppEvent::None,
    }
}

fn map_menu_click(column: u16) -> AppEvent {
    let column = usize::from(column);
    for (label, event) in menu_actions() {
        let Some(start) = MENUS.find(label) else {
            continue;
        };
        let end = start + label.chars().count();
        if (start..end).contains(&column) {
            return event;
        }
    }
    AppEvent::None
}

fn menu_actions() -> [(&'static str, AppEvent); 6] {
    [
        ("File(:)", AppEvent::OpenPalette),
        ("View(s z w n)", AppEvent::ToggleLayout),
        ("Navigate(j/k g/G)", AppEvent::FocusSidebar),
        ("Theme(T)", AppEvent::CycleTheme),
        ("Agent(:)", AppEvent::OpenPalette),
        ("Help(?)", AppEvent::ToggleHelp),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn mouse(kind: MouseEventKind, column: u16) -> MouseEvent {
        MouseEvent {
            kind,
            column,
            row: 0,
            modifiers: KeyModifiers::NONE,
        }
    }

    fn body_mouse(kind: MouseEventKind, column: u16) -> MouseEvent {
        MouseEvent {
            kind,
            column,
            row: 4,
            modifiers: KeyModifiers::NONE,
        }
    }

    #[test]
    fn q_quits_and_esc_cancels() {
        assert_eq!(map_key(key(KeyCode::Char('q'))), AppEvent::Quit);
        assert_eq!(map_key(key(KeyCode::Esc)), AppEvent::Cancel);
    }

    #[test]
    fn enter_selects_and_colon_opens_palette() {
        assert_eq!(map_key(key(KeyCode::Enter)), AppEvent::Select);
        assert_eq!(map_key(key(KeyCode::Char(':'))), AppEvent::OpenPalette);
    }

    #[test]
    fn j_and_down_are_next() {
        assert_eq!(map_key(key(KeyCode::Char('j'))), AppEvent::Next);
        assert_eq!(map_key(key(KeyCode::Down)), AppEvent::Next);
    }

    #[test]
    fn k_and_up_are_prev() {
        assert_eq!(map_key(key(KeyCode::Char('k'))), AppEvent::Prev);
        assert_eq!(map_key(key(KeyCode::Up)), AppEvent::Prev);
    }

    #[test]
    fn t_s_and_tab_toggle_layout() {
        assert_eq!(map_key(key(KeyCode::Char('t'))), AppEvent::ToggleLayout);
        assert_eq!(map_key(key(KeyCode::Char('s'))), AppEvent::ToggleLayout);
        assert_eq!(map_key(key(KeyCode::Tab)), AppEvent::ToggleLayout);
    }

    #[test]
    fn z_folds_and_n_toggles_notes() {
        assert_eq!(map_key(key(KeyCode::Char('z'))), AppEvent::ToggleFold);
        assert_eq!(map_key(key(KeyCode::Char('n'))), AppEvent::ToggleNotes);
    }

    #[test]
    fn w_toggles_wrap() {
        assert_eq!(map_key(key(KeyCode::Char('w'))), AppEvent::ToggleWrap);
    }

    #[test]
    fn v_and_space_toggle_viewed() {
        assert_eq!(map_key(key(KeyCode::Char('v'))), AppEvent::ToggleViewed);
        assert_eq!(map_key(key(KeyCode::Char(' '))), AppEvent::ToggleViewed);
    }

    #[test]
    fn capital_t_cycles_theme_lower_t_does_not() {
        assert_eq!(map_key(key(KeyCode::Char('T'))), AppEvent::CycleTheme);
        assert_eq!(map_key(key(KeyCode::Char('t'))), AppEvent::ToggleLayout);
    }

    #[test]
    fn question_mark_toggles_help() {
        assert_eq!(map_key(key(KeyCode::Char('?'))), AppEvent::ToggleHelp);
    }

    #[test]
    fn h_l_and_arrows_switch_focus() {
        assert_eq!(map_key(key(KeyCode::Char('h'))), AppEvent::FocusSidebar);
        assert_eq!(map_key(key(KeyCode::Left)), AppEvent::FocusSidebar);
        assert_eq!(map_key(key(KeyCode::Char('l'))), AppEvent::FocusDiff);
        assert_eq!(map_key(key(KeyCode::Right)), AppEvent::FocusDiff);
    }

    #[test]
    fn ctrl_d_and_u_scroll() {
        assert_eq!(map_key(ctrl('d')), AppEvent::ScrollDown);
        assert_eq!(map_key(ctrl('u')), AppEvent::ScrollUp);
    }

    #[test]
    fn plain_d_and_u_are_unbound() {
        assert_eq!(map_key(key(KeyCode::Char('d'))), AppEvent::None);
        assert_eq!(map_key(key(KeyCode::Char('u'))), AppEvent::None);
    }

    #[test]
    fn page_keys_scroll() {
        assert_eq!(map_key(key(KeyCode::PageDown)), AppEvent::ScrollDown);
        assert_eq!(map_key(key(KeyCode::PageUp)), AppEvent::ScrollUp);
    }

    #[test]
    fn g_and_home_are_top_shift_g_and_end_are_bottom() {
        assert_eq!(map_key(key(KeyCode::Char('g'))), AppEvent::Top);
        assert_eq!(map_key(key(KeyCode::Home)), AppEvent::Top);
        assert_eq!(map_key(key(KeyCode::Char('G'))), AppEvent::Bottom);
        assert_eq!(map_key(key(KeyCode::End)), AppEvent::Bottom);
    }

    #[test]
    fn unbound_key_is_none() {
        assert_eq!(map_key(key(KeyCode::Char('x'))), AppEvent::None);
        assert_eq!(map_key(key(KeyCode::F(5))), AppEvent::None);
    }

    #[test]
    fn quit_takes_priority_for_q_even_with_ctrl() {
        assert_eq!(map_key(ctrl('q')), AppEvent::Quit);
    }

    #[test]
    fn mouse_wheel_scrolls() {
        assert_eq!(
            map_mouse(mouse(MouseEventKind::ScrollUp, 80)),
            AppEvent::ScrollUp
        );
        assert_eq!(
            map_mouse(mouse(MouseEventKind::ScrollDown, 80)),
            AppEvent::ScrollDown
        );
    }

    #[test]
    fn mouse_left_click_coarsely_focuses_panes() {
        assert_eq!(
            map_mouse(body_mouse(MouseEventKind::Down(MouseButton::Left), 10)),
            AppEvent::SelectTreeRow(2)
        );
        assert_eq!(
            map_mouse(body_mouse(MouseEventKind::Down(MouseButton::Left), 80)),
            AppEvent::FocusDiff
        );
    }

    #[test]
    fn other_mouse_events_are_noops() {
        assert_eq!(
            map_mouse(mouse(MouseEventKind::Down(MouseButton::Right), 80)),
            AppEvent::None
        );
    }

    #[test]
    fn menu_clicks_dispatch_to_matching_actions() {
        for (label, event) in menu_actions() {
            let start = MENUS.find(label).expect("menu label exists");
            let end = start + label.chars().count();
            for column in start..end {
                let column = u16::try_from(column).expect("menu column fits u16");
                assert_eq!(
                    map_mouse(mouse(MouseEventKind::Down(MouseButton::Left), column)),
                    event,
                    "column {column} in {label} should map consistently"
                );
            }
        }
    }

    #[test]
    fn menu_gaps_are_noops() {
        let file_end = MENUS.find("File(:)").unwrap() + "File(:)".len();
        let view_start = MENUS.find("View(s z w n)").unwrap();
        for column in file_end..view_start {
            let column = u16::try_from(column).expect("menu column fits u16");
            assert_eq!(
                map_mouse(mouse(MouseEventKind::Down(MouseButton::Left), column)),
                AppEvent::None
            );
        }
    }
}
