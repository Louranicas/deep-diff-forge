use crate::theme::ThemeKind;
use deep_diff_forge_core::{AgentAnnotation, ReviewFile};
use deep_diff_forge_graph::{RankedFile, rank};

/// A semantic UI action, decoupled from any specific key binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppEvent {
    /// Quit the review.
    Quit,
    /// Select the next (lower-ranked) file.
    Next,
    /// Select the previous (higher-ranked) file.
    Prev,
    /// Toggle the diff layout (inline ↔ side-by-side).
    ToggleLayout,
    /// Scroll the diff view up one line.
    ScrollUp,
    /// Scroll the diff view down one line.
    ScrollDown,
    /// Jump to the first (top-ranked) file.
    Top,
    /// Jump to the last file.
    Bottom,
    /// Collapse/expand long runs of unchanged context.
    ToggleFold,
    /// Show/hide inline agent notes.
    ToggleNotes,
    /// Cycle to the next colour theme.
    CycleTheme,
    /// Move focus to the file tree.
    FocusSidebar,
    /// Move focus to the diff pane.
    FocusDiff,
    /// Show/hide the help overlay.
    ToggleHelp,
    /// No-op (unrecognized input).
    None,
}

/// Diff-pane layout mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutMode {
    /// Single-column unified diff.
    Inline,
    /// Two-column old ‖ new diff.
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

/// Which pane currently has focus (affects only the highlighted border).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Focus {
    /// The ranked file tree.
    #[default]
    Sidebar,
    /// The diff pane.
    Diff,
}

/// A modal overlay drawn over the main area.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Overlay {
    /// No overlay.
    #[default]
    None,
    /// The keybinding help card.
    Help,
}

/// Review-first interactive app state.
///
/// Pure and fully testable: rendering and the terminal event loop are kept out
/// of this type. It holds the full review content (so the diff pane can render
/// hunks, syntax, and notes), the ranking (for the tree order and scores), and
/// the engine's inline annotations — all aligned by ranked order.
#[derive(Debug, Clone)]
pub struct ReviewApp {
    ranked: Vec<RankedFile>,
    content: Vec<ReviewFile>,
    annotations: Vec<AgentAnnotation>,
    selected: usize,
    scroll: u16,
    layout: LayoutMode,
    theme: ThemeKind,
    focus: Focus,
    fold: bool,
    show_notes: bool,
    overlay: Overlay,
    running: bool,
}

impl ReviewApp {
    /// Build an app from review content, ranking it and aligning content to the
    /// ranked order. No annotations.
    ///
    /// Takes its content by value as a stable, ergonomic constructor (`new(vec)`
    /// / `new(Vec::new())`); ranking borrows it, so it is dropped on return.
    #[must_use]
    #[allow(clippy::needless_pass_by_value)]
    pub fn new(content: Vec<ReviewFile>) -> Self {
        Self::from_review(&content)
    }

    /// Build an app from parsed review files (ranked, no annotations).
    #[must_use]
    pub fn from_review(files: &[ReviewFile]) -> Self {
        Self::from_review_with_annotations(files, Vec::new())
    }

    /// Build an app from parsed review files plus a set of inline annotations.
    ///
    /// Ranking determines both the tree order and the selection order; content
    /// is reordered to match so index `i` names the same file in every vector.
    #[must_use]
    pub fn from_review_with_annotations(
        files: &[ReviewFile],
        annotations: Vec<AgentAnnotation>,
    ) -> Self {
        let ranked_all = rank(files);
        let mut ranked = Vec::with_capacity(ranked_all.len());
        let mut content = Vec::with_capacity(ranked_all.len());
        for r in ranked_all {
            if let Some(file) = files.iter().find(|f| f.path == r.path) {
                content.push(file.clone());
                ranked.push(r);
            }
        }
        Self {
            ranked,
            content,
            annotations,
            selected: 0,
            scroll: 0,
            layout: LayoutMode::Inline,
            theme: ThemeKind::default(),
            focus: Focus::default(),
            fold: true,
            show_notes: true,
            overlay: Overlay::default(),
            running: true,
        }
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
                self.selected = self.ranked.len().saturating_sub(1);
                self.scroll = 0;
            }
            AppEvent::ToggleFold => self.fold = !self.fold,
            AppEvent::ToggleNotes => self.show_notes = !self.show_notes,
            AppEvent::CycleTheme => self.theme = self.theme.next(),
            AppEvent::FocusSidebar => self.focus = Focus::Sidebar,
            AppEvent::FocusDiff => self.focus = Focus::Diff,
            AppEvent::ToggleHelp => {
                self.overlay = match self.overlay {
                    Overlay::None => Overlay::Help,
                    Overlay::Help => Overlay::None,
                };
            }
            AppEvent::None => {}
        }
    }

    fn select_next(&mut self) {
        if self.selected + 1 < self.ranked.len() {
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
        self.ranked.len()
    }

    /// Whether the review is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ranked.is_empty()
    }

    /// Index of the selected file.
    #[must_use]
    pub fn selected_index(&self) -> usize {
        self.selected
    }

    /// The selected file's ranking metadata, if any.
    #[must_use]
    pub fn selected_file(&self) -> Option<&RankedFile> {
        self.ranked.get(self.selected)
    }

    /// The selected file's full diff content, if any.
    #[must_use]
    pub fn selected_content(&self) -> Option<&ReviewFile> {
        self.content.get(self.selected)
    }

    /// All ranked files (ranking metadata, in ranked order).
    #[must_use]
    pub fn files(&self) -> &[RankedFile] {
        &self.ranked
    }

    /// All review content, aligned to [`ReviewApp::files`].
    #[must_use]
    pub fn content(&self) -> &[ReviewFile] {
        &self.content
    }

    /// All inline annotations.
    #[must_use]
    pub fn annotations(&self) -> &[AgentAnnotation] {
        &self.annotations
    }

    /// Current scroll offset of the diff pane.
    #[must_use]
    pub fn scroll(&self) -> u16 {
        self.scroll
    }

    /// Current diff layout mode.
    #[must_use]
    pub fn layout(&self) -> LayoutMode {
        self.layout
    }

    /// Current colour theme.
    #[must_use]
    pub fn theme(&self) -> ThemeKind {
        self.theme
    }

    /// Currently focused pane.
    #[must_use]
    pub fn focus(&self) -> Focus {
        self.focus
    }

    /// Whether long context runs are collapsed.
    #[must_use]
    pub fn fold(&self) -> bool {
        self.fold
    }

    /// Whether inline notes are shown.
    #[must_use]
    pub fn show_notes(&self) -> bool {
        self.show_notes
    }

    /// The active modal overlay.
    #[must_use]
    pub fn overlay(&self) -> Overlay {
        self.overlay
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
        assert_eq!(a.selected_file().unwrap().path, "src/lib.rs");
    }

    #[test]
    fn content_is_aligned_to_ranking() {
        let a = app();
        // selected_content path matches selected_file path at every index.
        assert_eq!(
            a.selected_content().unwrap().path,
            a.selected_file().unwrap().path
        );
        assert_eq!(a.content().len(), a.files().len());
    }

    #[test]
    fn selected_content_tracks_navigation() {
        let mut a = app();
        a.handle(AppEvent::Next);
        assert_eq!(
            a.selected_content().unwrap().path,
            a.selected_file().unwrap().path
        );
    }

    #[test]
    fn defaults_fold_and_notes_on() {
        let a = app();
        assert!(a.fold());
        assert!(a.show_notes());
        assert_eq!(a.theme(), ThemeKind::Dark);
        assert_eq!(a.focus(), Focus::Sidebar);
        assert_eq!(a.overlay(), Overlay::None);
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
    fn prev_goes_back_and_clamps() {
        let mut a = app();
        a.handle(AppEvent::Next);
        a.handle(AppEvent::Prev);
        assert_eq!(a.selected_index(), 0);
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
    fn scroll_down_then_up() {
        let mut a = app();
        a.handle(AppEvent::ScrollDown);
        a.handle(AppEvent::ScrollDown);
        a.handle(AppEvent::ScrollUp);
        assert_eq!(a.scroll(), 1);
    }

    #[test]
    fn scroll_up_saturates_at_zero() {
        let mut a = app();
        a.handle(AppEvent::ScrollUp);
        assert_eq!(a.scroll(), 0);
    }

    #[test]
    fn changing_selection_resets_scroll() {
        let mut a = app();
        a.handle(AppEvent::ScrollDown);
        a.handle(AppEvent::Next);
        assert_eq!(a.scroll(), 0);
    }

    #[test]
    fn top_and_bottom_jump() {
        let mut a = app();
        a.handle(AppEvent::Bottom);
        assert_eq!(a.selected_index(), 2);
        a.handle(AppEvent::Top);
        assert_eq!(a.selected_index(), 0);
    }

    #[test]
    fn toggle_fold_flips() {
        let mut a = app();
        assert!(a.fold());
        a.handle(AppEvent::ToggleFold);
        assert!(!a.fold());
        a.handle(AppEvent::ToggleFold);
        assert!(a.fold());
    }

    #[test]
    fn toggle_notes_flips() {
        let mut a = app();
        a.handle(AppEvent::ToggleNotes);
        assert!(!a.show_notes());
    }

    #[test]
    fn cycle_theme_advances() {
        let mut a = app();
        a.handle(AppEvent::CycleTheme);
        assert_eq!(a.theme(), ThemeKind::Midnight);
    }

    #[test]
    fn focus_events_set_focus() {
        let mut a = app();
        a.handle(AppEvent::FocusDiff);
        assert_eq!(a.focus(), Focus::Diff);
        a.handle(AppEvent::FocusSidebar);
        assert_eq!(a.focus(), Focus::Sidebar);
    }

    #[test]
    fn help_overlay_toggles() {
        let mut a = app();
        a.handle(AppEvent::ToggleHelp);
        assert_eq!(a.overlay(), Overlay::Help);
        a.handle(AppEvent::ToggleHelp);
        assert_eq!(a.overlay(), Overlay::None);
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
    fn empty_review_is_empty_and_safe() {
        let mut a = ReviewApp::new(Vec::new());
        assert!(a.is_empty());
        assert!(a.selected_file().is_none());
        assert!(a.selected_content().is_none());
        a.handle(AppEvent::Bottom);
        a.handle(AppEvent::Next);
        assert_eq!(a.selected_index(), 0);
    }

    #[test]
    fn annotations_round_trip() {
        let files = parse(MULTI).unwrap();
        let notes = crate::notes::engine_annotations(&files, &deep_diff_forge_graph::rank(&files));
        let n = notes.len();
        let a = ReviewApp::from_review_with_annotations(&files, notes);
        assert_eq!(a.annotations().len(), n);
        assert!(n > 0, "lib.rs should yield at least one engine note");
    }

    #[test]
    fn layout_labels_are_stable() {
        assert_eq!(LayoutMode::Inline.label(), "inline");
        assert_eq!(LayoutMode::SideBySide.label(), "side-by-side");
        assert_eq!(LayoutMode::Inline.toggled().toggled(), LayoutMode::Inline);
    }

    #[test]
    fn app_is_cloneable() {
        let a = app();
        let b = a.clone();
        assert_eq!(a.file_count(), b.file_count());
    }
}
