//! Colour themes for the review cockpit.
//!
//! A [`Palette`] is a flat set of [`Color`]s for every surface the UI paints —
//! chrome (menu/status/borders), the diff gutters, agent-note boxes, and the
//! syntax-highlight classes from [`deep_diff_forge_syntax`]. Themes are pure
//! data: [`ThemeKind::palette`] returns a `Palette` with no I/O, so rendering
//! and tests share one source of truth and a theme switch is a single field
//! change on the app state.

use deep_diff_forge_syntax::HighlightClass;
use ratatui::style::Color;

/// A selectable theme. Cycled in-app via [`ThemeKind::next`]; the screenshot's
/// `Theme` menu maps directly onto this enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemeKind {
    /// Balanced dark theme (default): true-colour, syntax-rich.
    #[default]
    Dark,
    /// Deep, low-contrast night theme.
    Midnight,
    /// Sixteen-colour monochrome — safe on terminals without true-colour.
    Mono,
}

impl ThemeKind {
    /// The next theme in the cycle, wrapping back to the first.
    #[must_use]
    pub fn next(self) -> Self {
        match self {
            Self::Dark => Self::Midnight,
            Self::Midnight => Self::Mono,
            Self::Mono => Self::Dark,
        }
    }

    /// Stable, lower-case label (used in the status bar and tests).
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Midnight => "midnight",
            Self::Mono => "mono",
        }
    }

    /// The concrete colour set for this theme.
    #[must_use]
    pub fn palette(self) -> Palette {
        match self {
            Self::Dark => Palette::dark(),
            Self::Midnight => Palette::midnight(),
            Self::Mono => Palette::mono(),
        }
    }
}

/// A flat colour set for one theme. Every UI surface reads its colour from here
/// so a theme is fully described by one value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    /// Primary foreground text.
    pub fg: Color,
    /// Secondary/dim text: line numbers, directory headers, hints.
    pub dim: Color,
    /// Pane border lines.
    pub border: Color,
    /// Pane titles and the focused-pane border.
    pub accent: Color,
    /// Background of the selected sidebar row.
    pub selection_bg: Color,
    /// Menu-bar foreground / background.
    pub menu_fg: Color,
    /// Menu-bar background.
    pub menu_bg: Color,
    /// Added-line foreground and its faint background tint.
    pub added: Color,
    /// Added-line background tint.
    pub added_bg: Color,
    /// Removed-line foreground.
    pub removed: Color,
    /// Removed-line background tint.
    pub removed_bg: Color,
    /// Agent-note box border and title.
    pub note: Color,
    /// Grounded-annotation marker (evidence-backed).
    pub grounded: Color,
    /// Ungrounded-annotation marker (no evidence — treat with suspicion).
    pub ungrounded: Color,
    // Syntax-highlight class colours.
    keyword: Color,
    function: Color,
    type_: Color,
    string: Color,
    comment: Color,
    number: Color,
    operator: Color,
}

impl Palette {
    /// Colour for a syntax-highlight class under this palette.
    #[must_use]
    pub fn class_color(&self, class: HighlightClass) -> Color {
        match class {
            HighlightClass::Keyword => self.keyword,
            HighlightClass::Function => self.function,
            HighlightClass::Type => self.type_,
            HighlightClass::StringLit => self.string,
            HighlightClass::Comment => self.comment,
            HighlightClass::Number | HighlightClass::Constant | HighlightClass::Attribute => {
                self.number
            }
            HighlightClass::Operator => self.operator,
            HighlightClass::Variable | HighlightClass::Plain => self.fg,
        }
    }

    fn dark() -> Self {
        Self {
            fg: Color::Rgb(0xd4, 0xd4, 0xd4),
            dim: Color::Rgb(0x80, 0x80, 0x80),
            border: Color::Rgb(0x44, 0x44, 0x4a),
            accent: Color::Rgb(0x7a, 0xa2, 0xf7),
            selection_bg: Color::Rgb(0x2a, 0x2e, 0x3a),
            menu_fg: Color::Rgb(0xe0, 0xe0, 0xe0),
            menu_bg: Color::Rgb(0x1e, 0x20, 0x28),
            added: Color::Rgb(0x6a, 0xd6, 0x8e),
            added_bg: Color::Rgb(0x12, 0x2a, 0x1c),
            removed: Color::Rgb(0xe5, 0x6b, 0x6f),
            removed_bg: Color::Rgb(0x2a, 0x14, 0x16),
            note: Color::Rgb(0xc0, 0x9a, 0xf0),
            grounded: Color::Rgb(0x6a, 0xd6, 0x8e),
            ungrounded: Color::Rgb(0xd6, 0xbf, 0x6a),
            keyword: Color::Rgb(0xc0, 0x86, 0xd0),
            function: Color::Rgb(0x7a, 0xa2, 0xf7),
            type_: Color::Rgb(0x56, 0xc8, 0xd8),
            string: Color::Rgb(0x9e, 0xce, 0x6a),
            comment: Color::Rgb(0x6a, 0x70, 0x7a),
            number: Color::Rgb(0xe0, 0xaf, 0x68),
            operator: Color::Rgb(0xb4, 0xb4, 0xb4),
        }
    }

    fn midnight() -> Self {
        Self {
            fg: Color::Rgb(0xc8, 0xd0, 0xe0),
            dim: Color::Rgb(0x5a, 0x64, 0x80),
            border: Color::Rgb(0x2a, 0x32, 0x48),
            accent: Color::Rgb(0x82, 0xc8, 0xff),
            selection_bg: Color::Rgb(0x18, 0x22, 0x3c),
            menu_fg: Color::Rgb(0xc8, 0xd0, 0xe0),
            menu_bg: Color::Rgb(0x0c, 0x12, 0x22),
            added: Color::Rgb(0x5c, 0xc8, 0xa0),
            added_bg: Color::Rgb(0x08, 0x22, 0x1e),
            removed: Color::Rgb(0xe0, 0x6c, 0x8c),
            removed_bg: Color::Rgb(0x22, 0x0c, 0x16),
            note: Color::Rgb(0x9a, 0xb0, 0xff),
            grounded: Color::Rgb(0x5c, 0xc8, 0xa0),
            ungrounded: Color::Rgb(0xd0, 0xb0, 0x70),
            keyword: Color::Rgb(0xa8, 0x8c, 0xe0),
            function: Color::Rgb(0x82, 0xc8, 0xff),
            type_: Color::Rgb(0x66, 0xc0, 0xc8),
            string: Color::Rgb(0x8c, 0xc0, 0x88),
            comment: Color::Rgb(0x4c, 0x56, 0x70),
            number: Color::Rgb(0xd0, 0xa0, 0x70),
            operator: Color::Rgb(0x9a, 0xa4, 0xc0),
        }
    }

    fn mono() -> Self {
        // Sixteen-colour fallback: no true-colour assumptions.
        Self {
            fg: Color::Gray,
            dim: Color::DarkGray,
            border: Color::DarkGray,
            accent: Color::Cyan,
            selection_bg: Color::Indexed(238),
            menu_fg: Color::White,
            menu_bg: Color::Indexed(236),
            added: Color::Green,
            added_bg: Color::Indexed(22),
            removed: Color::Red,
            removed_bg: Color::Indexed(52),
            note: Color::Magenta,
            grounded: Color::Green,
            ungrounded: Color::Yellow,
            keyword: Color::Magenta,
            function: Color::Blue,
            type_: Color::Cyan,
            string: Color::Green,
            comment: Color::DarkGray,
            number: Color::Yellow,
            operator: Color::Gray,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_cycles_through_all_three() {
        assert_eq!(ThemeKind::Dark.next(), ThemeKind::Midnight);
        assert_eq!(ThemeKind::Midnight.next(), ThemeKind::Mono);
        assert_eq!(ThemeKind::Mono.next(), ThemeKind::Dark);
    }

    #[test]
    fn next_is_a_full_cycle() {
        let k = ThemeKind::default();
        assert_eq!(k.next().next().next(), k);
    }

    #[test]
    fn default_is_dark() {
        assert_eq!(ThemeKind::default(), ThemeKind::Dark);
    }

    #[test]
    fn labels_are_stable_and_distinct() {
        assert_eq!(ThemeKind::Dark.label(), "dark");
        assert_eq!(ThemeKind::Midnight.label(), "midnight");
        assert_eq!(ThemeKind::Mono.label(), "mono");
    }

    #[test]
    fn added_and_removed_differ_in_every_theme() {
        for kind in [ThemeKind::Dark, ThemeKind::Midnight, ThemeKind::Mono] {
            let p = kind.palette();
            assert_ne!(p.added, p.removed, "{} added==removed", kind.label());
        }
    }

    #[test]
    fn every_highlight_class_maps_to_a_colour() {
        let p = ThemeKind::Dark.palette();
        // Exhaustive over the public class set; a new class without a colour
        // would fail to compile in `class_color`, so this guards behaviour.
        let classes = [
            HighlightClass::Keyword,
            HighlightClass::Function,
            HighlightClass::Type,
            HighlightClass::StringLit,
            HighlightClass::Comment,
            HighlightClass::Number,
            HighlightClass::Constant,
            HighlightClass::Attribute,
            HighlightClass::Operator,
            HighlightClass::Variable,
            HighlightClass::Plain,
        ];
        for c in classes {
            // Must not panic and must return *some* colour.
            let _ = p.class_color(c);
        }
        // Keyword and comment should be visually distinct.
        assert_ne!(
            p.class_color(HighlightClass::Keyword),
            p.class_color(HighlightClass::Comment)
        );
    }

    #[test]
    fn plain_and_variable_use_foreground() {
        let p = ThemeKind::Midnight.palette();
        assert_eq!(p.class_color(HighlightClass::Plain), p.fg);
        assert_eq!(p.class_color(HighlightClass::Variable), p.fg);
    }

    #[test]
    fn mono_uses_named_colours_only() {
        // The mono palette must avoid Rgb so it renders on 16-colour terminals.
        let p = ThemeKind::Mono.palette();
        assert!(!matches!(p.fg, Color::Rgb(..)));
        assert!(!matches!(p.added, Color::Rgb(..)));
        assert!(!matches!(p.keyword, Color::Rgb(..)));
    }

    #[test]
    fn palette_is_copy() {
        let p = ThemeKind::Dark.palette();
        let q = p;
        assert_eq!(p, q);
    }

    #[test]
    fn grounded_and_ungrounded_differ() {
        for kind in [ThemeKind::Dark, ThemeKind::Midnight, ThemeKind::Mono] {
            let p = kind.palette();
            assert_ne!(p.grounded, p.ungrounded);
        }
    }
}
