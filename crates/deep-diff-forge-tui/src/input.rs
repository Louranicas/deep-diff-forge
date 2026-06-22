use crate::state::AppEvent;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

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
        KeyCode::Char('n') => AppEvent::ToggleNotes,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
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
        assert_eq!(map_key(key(KeyCode::Char('w'))), AppEvent::None);
        assert_eq!(map_key(key(KeyCode::Char('x'))), AppEvent::None);
        assert_eq!(map_key(key(KeyCode::F(5))), AppEvent::None);
    }

    #[test]
    fn quit_takes_priority_for_q_even_with_ctrl() {
        assert_eq!(map_key(ctrl('q')), AppEvent::Quit);
    }
}
