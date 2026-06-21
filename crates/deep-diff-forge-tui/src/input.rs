use crate::state::AppEvent;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Map a key event to a semantic [`AppEvent`].
///
/// Bindings are vi-style with arrow-key equivalents; unrecognized keys map to
/// [`AppEvent::None`] so the event loop simply ignores them.
#[must_use]
pub fn map_key(key: KeyEvent) -> AppEvent {
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => AppEvent::Quit,
        KeyCode::Char('j') | KeyCode::Down => AppEvent::Next,
        KeyCode::Char('k') | KeyCode::Up => AppEvent::Prev,
        KeyCode::Char('t') | KeyCode::Tab => AppEvent::ToggleLayout,
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => AppEvent::ScrollDown,
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => AppEvent::ScrollUp,
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
    fn q_quits() {
        assert_eq!(map_key(key(KeyCode::Char('q'))), AppEvent::Quit);
    }

    #[test]
    fn esc_quits() {
        assert_eq!(map_key(key(KeyCode::Esc)), AppEvent::Quit);
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
    fn t_and_tab_toggle_layout() {
        assert_eq!(map_key(key(KeyCode::Char('t'))), AppEvent::ToggleLayout);
        assert_eq!(map_key(key(KeyCode::Tab)), AppEvent::ToggleLayout);
    }

    #[test]
    fn ctrl_d_scrolls_down() {
        assert_eq!(map_key(ctrl('d')), AppEvent::ScrollDown);
    }

    #[test]
    fn ctrl_u_scrolls_up() {
        assert_eq!(map_key(ctrl('u')), AppEvent::ScrollUp);
    }

    #[test]
    fn plain_d_is_not_scroll() {
        // Without the control modifier, 'd' is unbound.
        assert_eq!(map_key(key(KeyCode::Char('d'))), AppEvent::None);
    }

    #[test]
    fn page_keys_scroll() {
        assert_eq!(map_key(key(KeyCode::PageDown)), AppEvent::ScrollDown);
        assert_eq!(map_key(key(KeyCode::PageUp)), AppEvent::ScrollUp);
    }

    #[test]
    fn g_and_home_are_top() {
        assert_eq!(map_key(key(KeyCode::Char('g'))), AppEvent::Top);
        assert_eq!(map_key(key(KeyCode::Home)), AppEvent::Top);
    }

    #[test]
    fn shift_g_and_end_are_bottom() {
        assert_eq!(map_key(key(KeyCode::Char('G'))), AppEvent::Bottom);
        assert_eq!(map_key(key(KeyCode::End)), AppEvent::Bottom);
    }

    #[test]
    fn unbound_key_is_none() {
        assert_eq!(map_key(key(KeyCode::Char('z'))), AppEvent::None);
        assert_eq!(map_key(key(KeyCode::F(5))), AppEvent::None);
    }

    #[test]
    fn lowercase_g_is_top_uppercase_is_bottom() {
        assert_ne!(
            map_key(key(KeyCode::Char('g'))),
            map_key(key(KeyCode::Char('G')))
        );
    }

    #[test]
    fn quit_takes_priority_for_q() {
        // 'q' even with modifiers still quits (code match precedes modifier checks).
        assert_eq!(map_key(ctrl('q')), AppEvent::Quit);
    }
}
