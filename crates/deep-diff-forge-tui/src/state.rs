use crate::command::{self, Command, CommandOutput};
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
    /// Toggle the selected file's "reviewed" flag (and advance when marking it).
    ToggleViewed,
    /// Show/hide the help overlay.
    ToggleHelp,
    /// Open the command palette.
    OpenPalette,
    /// Confirm the current overlay selection (Enter).
    Select,
    /// Dismiss the current overlay, or quit when none is open (Esc).
    Cancel,
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
    /// The command palette (pick an engine capability to run).
    Palette,
    /// The result panel of the last-run command.
    Panel,
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
    /// Per-file "reviewed" flags, aligned to `ranked`.
    viewed: Vec<bool>,
    selected: usize,
    scroll: u16,
    layout: LayoutMode,
    theme: ThemeKind,
    focus: Focus,
    fold: bool,
    show_notes: bool,
    overlay: Overlay,
    palette_index: usize,
    panel: CommandOutput,
    panel_scroll: u16,
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
        let viewed = vec![false; ranked.len()];
        Self {
            ranked,
            content,
            annotations,
            viewed,
            selected: 0,
            scroll: 0,
            layout: LayoutMode::Inline,
            theme: ThemeKind::default(),
            focus: Focus::default(),
            fold: true,
            show_notes: true,
            overlay: Overlay::default(),
            palette_index: 0,
            panel: CommandOutput::default(),
            panel_scroll: 0,
            running: true,
        }
    }

    /// Apply a semantic event, routed by the active overlay so overlays are
    /// modal: `Quit` always quits; otherwise the open overlay (if any) consumes
    /// navigation/selection, and only the review handles events when none is up.
    pub fn handle(&mut self, event: AppEvent) {
        if event == AppEvent::Quit {
            self.running = false;
            return;
        }
        match self.overlay {
            Overlay::None => self.handle_review(event),
            Overlay::Help => self.handle_help(event),
            Overlay::Palette => self.handle_palette(event),
            Overlay::Panel => self.handle_panel(event),
        }
    }

    fn handle_review(&mut self, event: AppEvent) {
        match event {
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
            AppEvent::ToggleViewed => self.toggle_viewed(),
            AppEvent::ToggleHelp => self.overlay = Overlay::Help,
            AppEvent::OpenPalette => {
                self.overlay = Overlay::Palette;
                self.palette_index = 0;
            }
            // Esc with no overlay quits.
            AppEvent::Cancel => self.running = false,
            AppEvent::Quit | AppEvent::Select | AppEvent::None => {}
        }
    }

    fn handle_help(&mut self, event: AppEvent) {
        if matches!(event, AppEvent::ToggleHelp | AppEvent::Cancel) {
            self.overlay = Overlay::None;
        }
    }

    fn handle_palette(&mut self, event: AppEvent) {
        let count = Command::all().len();
        match event {
            AppEvent::Next => self.palette_index = (self.palette_index + 1) % count,
            AppEvent::Prev => self.palette_index = (self.palette_index + count - 1) % count,
            AppEvent::Top => self.palette_index = 0,
            AppEvent::Bottom => self.palette_index = count - 1,
            AppEvent::Select => self.run_selected_command(),
            AppEvent::Cancel | AppEvent::OpenPalette => self.overlay = Overlay::None,
            _ => {}
        }
    }

    fn handle_panel(&mut self, event: AppEvent) {
        match event {
            AppEvent::ScrollDown | AppEvent::Next => {
                self.panel_scroll = self.panel_scroll.saturating_add(1);
            }
            AppEvent::ScrollUp | AppEvent::Prev => {
                self.panel_scroll = self.panel_scroll.saturating_sub(1);
            }
            AppEvent::Top => self.panel_scroll = 0,
            // Back to the palette; `:` closes overlays entirely.
            AppEvent::Cancel => self.overlay = Overlay::Palette,
            AppEvent::OpenPalette => self.overlay = Overlay::None,
            _ => {}
        }
    }

    fn run_selected_command(&mut self) {
        let command = Command::all()[self.palette_index];
        self.panel = command::run(command, self);
        self.panel_scroll = 0;
        self.overlay = Overlay::Panel;
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

    /// Toggle the selected file's reviewed flag. Marking a file reviewed
    /// advances to the next file so a reviewer can sweep top-to-bottom with a
    /// single key; un-marking leaves the selection in place. A no-op on an
    /// empty review.
    fn toggle_viewed(&mut self) {
        let now_viewed = match self.viewed.get_mut(self.selected) {
            Some(flag) => {
                *flag = !*flag;
                *flag
            }
            None => return,
        };
        if now_viewed {
            self.select_next();
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

    /// Whether the ranked file at `index` has been marked reviewed. Out-of-range
    /// indices read as `false`.
    #[must_use]
    pub fn is_viewed(&self, index: usize) -> bool {
        self.viewed.get(index).copied().unwrap_or(false)
    }

    /// How many files have been marked reviewed (review-progress numerator).
    #[must_use]
    pub fn viewed_count(&self) -> usize {
        self.viewed.iter().filter(|&&v| v).count()
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

    /// The highlighted command in the palette.
    #[must_use]
    pub fn palette_index(&self) -> usize {
        self.palette_index
    }

    /// The last-run command's result panel.
    #[must_use]
    pub fn panel(&self) -> &CommandOutput {
        &self.panel
    }

    /// The result panel's scroll offset.
    #[must_use]
    pub fn panel_scroll(&self) -> u16 {
        self.panel_scroll
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
    fn palette_opens_navigates_and_runs() {
        let mut a = app();
        a.handle(AppEvent::OpenPalette);
        assert_eq!(a.overlay(), Overlay::Palette);
        assert_eq!(a.palette_index(), 0);
        a.handle(AppEvent::Next);
        assert_eq!(a.palette_index(), 1);
        // Wrap backwards past zero.
        a.handle(AppEvent::Prev);
        a.handle(AppEvent::Prev);
        assert_eq!(a.palette_index(), Command::all().len() - 1);
        // Selecting runs the command into the panel.
        a.handle(AppEvent::Select);
        assert_eq!(a.overlay(), Overlay::Panel);
        assert!(!a.panel().title.is_empty());
    }

    #[test]
    fn palette_cancel_closes() {
        let mut a = app();
        a.handle(AppEvent::OpenPalette);
        a.handle(AppEvent::Cancel);
        assert_eq!(a.overlay(), Overlay::None);
    }

    #[test]
    fn panel_scrolls_and_returns_to_palette() {
        let mut a = app();
        a.handle(AppEvent::OpenPalette);
        a.handle(AppEvent::Select);
        assert_eq!(a.overlay(), Overlay::Panel);
        a.handle(AppEvent::ScrollDown);
        a.handle(AppEvent::ScrollDown);
        assert_eq!(a.panel_scroll(), 2);
        a.handle(AppEvent::ScrollUp);
        assert_eq!(a.panel_scroll(), 1);
        // Esc from the panel returns to the palette.
        a.handle(AppEvent::Cancel);
        assert_eq!(a.overlay(), Overlay::Palette);
    }

    #[test]
    fn esc_quits_only_when_no_overlay() {
        let mut a = app();
        // With the palette open, Esc closes it rather than quitting.
        a.handle(AppEvent::OpenPalette);
        a.handle(AppEvent::Cancel);
        assert!(a.is_running());
        // With nothing open, Esc quits.
        a.handle(AppEvent::Cancel);
        assert!(!a.is_running());
    }

    #[test]
    fn quit_always_quits_even_with_overlay() {
        let mut a = app();
        a.handle(AppEvent::OpenPalette);
        a.handle(AppEvent::Quit);
        assert!(!a.is_running());
    }

    #[test]
    fn review_nav_is_ignored_while_palette_open() {
        let mut a = app();
        a.handle(AppEvent::OpenPalette);
        let before = a.selected_index();
        a.handle(AppEvent::Next); // moves palette, not file selection
        assert_eq!(a.selected_index(), before);
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

    #[test]
    fn new_app_has_nothing_reviewed() {
        let a = app();
        assert_eq!(a.viewed_count(), 0);
        assert!(!a.is_viewed(0));
        assert!(!a.is_viewed(1));
        assert!(!a.is_viewed(2));
    }

    #[test]
    fn toggle_viewed_marks_and_advances() {
        let mut a = app();
        a.handle(AppEvent::ToggleViewed);
        assert!(a.is_viewed(0), "selected file should be marked reviewed");
        assert_eq!(a.selected_index(), 1, "marking advances to the next file");
        assert_eq!(a.viewed_count(), 1);
    }

    #[test]
    fn toggle_viewed_unmark_stays_put() {
        let mut a = app();
        a.handle(AppEvent::ToggleViewed); // mark file 0, advance to 1
        a.handle(AppEvent::Prev); // back to file 0
        a.handle(AppEvent::ToggleViewed); // un-mark file 0
        assert!(!a.is_viewed(0));
        assert_eq!(a.selected_index(), 0, "un-marking does not advance");
        assert_eq!(a.viewed_count(), 0);
    }

    #[test]
    fn toggle_viewed_at_last_file_clamps() {
        let mut a = app();
        a.handle(AppEvent::Bottom); // select last (index 2)
        a.handle(AppEvent::ToggleViewed);
        assert!(a.is_viewed(2));
        assert_eq!(a.selected_index(), 2, "advance clamps at the last file");
        assert_eq!(a.viewed_count(), 1);
    }

    #[test]
    fn toggle_viewed_empty_review_is_safe() {
        let mut a = ReviewApp::new(Vec::new());
        a.handle(AppEvent::ToggleViewed);
        assert_eq!(a.viewed_count(), 0);
        assert_eq!(a.selected_index(), 0);
        assert!(!a.is_viewed(0));
    }

    #[test]
    fn toggle_viewed_ignored_while_overlay_open() {
        let mut a = app();
        a.handle(AppEvent::OpenPalette);
        a.handle(AppEvent::ToggleViewed); // routed to the palette, not the review
        assert!(!a.is_viewed(0));
        assert_eq!(a.viewed_count(), 0);
    }

    #[test]
    fn is_viewed_out_of_range_is_false() {
        let a = app();
        assert!(!a.is_viewed(999));
    }

    #[test]
    fn viewed_flag_survives_clone() {
        let mut a = app();
        a.handle(AppEvent::ToggleViewed);
        let b = a.clone();
        assert_eq!(b.viewed_count(), 1);
        assert!(b.is_viewed(0));
    }
}
