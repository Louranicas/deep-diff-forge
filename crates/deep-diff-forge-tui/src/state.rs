use deep_diff_forge_core::ReviewFile;
use deep_diff_forge_graph::{RankedFile, rank};

/// A semantic UI action, decoupled from any specific key binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppEvent {
    /// Quit the review.
    Quit,
    /// Select the next (lower) file.
    Next,
    /// Select the previous (higher) file.
    Prev,
    /// Toggle the detail layout.
    ToggleLayout,
    /// Scroll the detail view up one line.
    ScrollUp,
    /// Scroll the detail view down one line.
    ScrollDown,
    /// Jump to the first file.
    Top,
    /// Jump to the last file.
    Bottom,
    /// No-op (unrecognized input).
    None,
}

/// Detail-pane layout mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutMode {
    /// Single-column inline detail.
    Inline,
    /// Two-column side-by-side detail.
    SideBySide,
}

impl LayoutMode {
    /// The other layout.
    #[must_use]
    pub fn toggled(self) -> Self {
        match self {
            Self::Inline => Self::SideBySide,
            Self::SideBySide => Self::Inline,
        }
    }

    /// Stable label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Inline => "inline",
            Self::SideBySide => "side-by-side",
        }
    }
}

/// Review-first interactive app state. Pure and fully testable: rendering and
/// the terminal event loop are kept out of this type.
#[derive(Debug, Clone)]
pub struct ReviewApp {
    files: Vec<RankedFile>,
    selected: usize,
    scroll: u16,
    layout: LayoutMode,
    running: bool,
}

impl ReviewApp {
    /// Build an app from an already-ranked review stream.
    #[must_use]
    pub fn new(files: Vec<RankedFile>) -> Self {
        Self {
            files,
            selected: 0,
            scroll: 0,
            layout: LayoutMode::Inline,
            running: true,
        }
    }

    /// Build an app from parsed review files, ranking them first.
    #[must_use]
    pub fn from_review(files: &[ReviewFile]) -> Self {
        Self::new(rank(files))
    }

    /// Apply a semantic event to the state.
    pub fn handle(&mut self, event: AppEvent) {
        match event {
            AppEvent::Quit => self.running = false,
            AppEvent::Next => self.select_next(),
            AppEvent::Prev => self.select_prev(),
            AppEvent::ToggleLayout => self.layout = self.layout.toggled(),
            AppEvent::ScrollUp => self.scroll = self.scroll.saturating_sub(1),
            AppEvent::ScrollDown => self.scroll = self.scroll.saturating_add(1),
            AppEvent::Top => {
                self.selected = 0;
                self.scroll = 0;
            }
            AppEvent::Bottom => {
                self.selected = self.files.len().saturating_sub(1);
                self.scroll = 0;
            }
            AppEvent::None => {}
        }
    }

    fn select_next(&mut self) {
        if self.selected + 1 < self.files.len() {
            self.selected += 1;
            self.scroll = 0;
        }
    }

    fn select_prev(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
            self.scroll = 0;
        }
    }

    /// Whether the app should keep running.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.running
    }

    /// Number of files in the review.
    #[must_use]
    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    /// Whether the review is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Index of the selected file.
    #[must_use]
    pub fn selected_index(&self) -> usize {
        self.selected
    }

    /// The selected file, if any.
    #[must_use]
    pub fn selected_file(&self) -> Option<&RankedFile> {
        self.files.get(self.selected)
    }

    /// All ranked files.
    #[must_use]
    pub fn files(&self) -> &[RankedFile] {
        &self.files
    }

    /// Current scroll offset.
    #[must_use]
    pub fn scroll(&self) -> u16 {
        self.scroll
    }

    /// Current layout mode.
    #[must_use]
    pub fn layout(&self) -> LayoutMode {
        self.layout
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
--- a/tests/it.rs
+++ b/tests/it.rs
@@ -1,1 +1,1 @@
-a
+b
";

    fn app() -> ReviewApp {
        ReviewApp::from_review(&parse(MULTI).unwrap())
    }

    #[test]
    fn new_app_is_running() {
        assert!(app().is_running());
    }

    #[test]
    fn from_review_ranks_files() {
        let a = app();
        assert_eq!(a.file_count(), 3);
        // lib.rs (public api) ranks first.
        assert_eq!(a.selected_file().unwrap().path, "src/lib.rs");
    }

    #[test]
    fn starts_at_first_selection() {
        assert_eq!(app().selected_index(), 0);
    }

    #[test]
    fn next_advances_selection() {
        let mut a = app();
        a.handle(AppEvent::Next);
        assert_eq!(a.selected_index(), 1);
    }

    #[test]
    fn next_clamps_at_end() {
        let mut a = app();
        for _ in 0..10 {
            a.handle(AppEvent::Next);
        }
        assert_eq!(a.selected_index(), 2);
    }

    #[test]
    fn prev_goes_back() {
        let mut a = app();
        a.handle(AppEvent::Next);
        a.handle(AppEvent::Prev);
        assert_eq!(a.selected_index(), 0);
    }

    #[test]
    fn prev_clamps_at_start() {
        let mut a = app();
        a.handle(AppEvent::Prev);
        assert_eq!(a.selected_index(), 0);
    }

    #[test]
    fn quit_stops_running() {
        let mut a = app();
        a.handle(AppEvent::Quit);
        assert!(!a.is_running());
    }

    #[test]
    fn toggle_layout_cycles() {
        let mut a = app();
        assert_eq!(a.layout(), LayoutMode::Inline);
        a.handle(AppEvent::ToggleLayout);
        assert_eq!(a.layout(), LayoutMode::SideBySide);
        a.handle(AppEvent::ToggleLayout);
        assert_eq!(a.layout(), LayoutMode::Inline);
    }

    #[test]
    fn scroll_down_increments() {
        let mut a = app();
        a.handle(AppEvent::ScrollDown);
        assert_eq!(a.scroll(), 1);
    }

    #[test]
    fn scroll_up_saturates_at_zero() {
        let mut a = app();
        a.handle(AppEvent::ScrollUp);
        assert_eq!(a.scroll(), 0);
    }

    #[test]
    fn scroll_up_after_down_returns() {
        let mut a = app();
        a.handle(AppEvent::ScrollDown);
        a.handle(AppEvent::ScrollDown);
        a.handle(AppEvent::ScrollUp);
        assert_eq!(a.scroll(), 1);
    }

    #[test]
    fn changing_selection_resets_scroll() {
        let mut a = app();
        a.handle(AppEvent::ScrollDown);
        a.handle(AppEvent::Next);
        assert_eq!(a.scroll(), 0);
    }

    #[test]
    fn top_jumps_to_first() {
        let mut a = app();
        a.handle(AppEvent::Next);
        a.handle(AppEvent::Next);
        a.handle(AppEvent::Top);
        assert_eq!(a.selected_index(), 0);
    }

    #[test]
    fn bottom_jumps_to_last() {
        let mut a = app();
        a.handle(AppEvent::Bottom);
        assert_eq!(a.selected_index(), 2);
    }

    #[test]
    fn none_event_is_noop() {
        let mut a = app();
        let before = a.selected_index();
        a.handle(AppEvent::None);
        assert_eq!(a.selected_index(), before);
        assert!(a.is_running());
    }

    #[test]
    fn empty_review_is_empty_and_has_no_selection() {
        let a = ReviewApp::new(Vec::new());
        assert!(a.is_empty());
        assert!(a.selected_file().is_none());
    }

    #[test]
    fn bottom_on_empty_stays_zero() {
        let mut a = ReviewApp::new(Vec::new());
        a.handle(AppEvent::Bottom);
        assert_eq!(a.selected_index(), 0);
    }

    #[test]
    fn next_on_empty_does_not_panic() {
        let mut a = ReviewApp::new(Vec::new());
        a.handle(AppEvent::Next);
        assert_eq!(a.selected_index(), 0);
    }

    #[test]
    fn files_accessor_matches_count() {
        let a = app();
        assert_eq!(a.files().len(), a.file_count());
    }

    #[test]
    fn layout_labels_are_stable() {
        assert_eq!(LayoutMode::Inline.label(), "inline");
        assert_eq!(LayoutMode::SideBySide.label(), "side-by-side");
    }

    #[test]
    fn layout_toggled_is_involutive() {
        assert_eq!(LayoutMode::Inline.toggled().toggled(), LayoutMode::Inline);
    }

    #[test]
    fn selected_file_tracks_navigation() {
        let mut a = app();
        a.handle(AppEvent::Bottom);
        assert_eq!(a.selected_file().unwrap().path, "tests/it.rs");
    }

    #[test]
    fn app_is_cloneable() {
        let a = app();
        let b = a.clone();
        assert_eq!(a.file_count(), b.file_count());
    }
}
