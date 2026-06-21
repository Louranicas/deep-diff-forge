//! Renderer-neutral projections of the Deep-Diff-Forge review model (L2).
//!
//! A projection converts the stable [`deep_diff_forge_core`] model into display
//! rows and text without ever mutating patch truth. Projections are pure and
//! infallible: a malformed projection request degrades to an empty render, it
//! never panics and never alters the model.
//!
//! Two layouts are provided at L2: [`render_inline`] and
//! [`render_side_by_side`]. Both are driven from the same row builders
//! ([`inline::inline_rows`], [`side_by_side::side_rows`]) so a TUI, pager, or
//! JSON consumer can share one model.

mod inline;
mod side_by_side;

pub use inline::{InlineRow, inline_rows, render_inline};
pub use side_by_side::{SideCell, SideRow, render_side_by_side, side_rows};

use deep_diff_forge_core::{FileStatus, ReviewFile};

/// Default column width for the side-by-side layout.
pub const DEFAULT_SIDE_WIDTH: usize = 60;

/// Snake-case status label, matching the JSON projection's spelling so every
/// surface names a status identically.
#[must_use]
pub(crate) fn status_label(status: FileStatus) -> &'static str {
    match status {
        FileStatus::Added => "added",
        FileStatus::Modified => "modified",
        FileStatus::Deleted => "deleted",
        FileStatus::Renamed => "renamed",
        FileStatus::TypeChanged => "type_changed",
        FileStatus::BinaryChanged => "binary_changed",
        FileStatus::Unknown => "unknown",
    }
}

/// Output layout selected by the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Layout {
    /// One column: line numbers, marker, text.
    #[default]
    Inline,
    /// Two columns: old side and new side aligned per change block.
    SideBySide,
}

/// Options controlling a projection render.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProjectionOptions {
    /// Selected layout.
    pub layout: Layout,
    /// Per-column width for [`Layout::SideBySide`] (ignored for inline).
    pub side_width: usize,
}

impl Default for ProjectionOptions {
    fn default() -> Self {
        Self {
            layout: Layout::Inline,
            side_width: DEFAULT_SIDE_WIDTH,
        }
    }
}

/// Render the review model to text using the given options.
#[must_use]
pub fn render(files: &[ReviewFile], options: ProjectionOptions) -> String {
    match options.layout {
        Layout::Inline => render_inline(files),
        Layout::SideBySide => render_side_by_side(files, options.side_width),
    }
}

/// Parse a layout name from a CLI flag value, if recognized.
#[must_use]
pub fn layout_from_str(name: &str) -> Option<Layout> {
    match name {
        "inline" => Some(Layout::Inline),
        "side-by-side" | "split" => Some(Layout::SideBySide),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deep_diff_forge_patch::parse;

    const BASIC: &str = "--- a/x\n+++ b/x\n@@ -1,3 +1,3 @@\n a\n-b\n+B\n c\n";

    #[test]
    fn default_layout_is_inline() {
        assert_eq!(Layout::default(), Layout::Inline);
    }

    #[test]
    fn default_options_use_inline_and_default_width() {
        let opts = ProjectionOptions::default();
        assert_eq!(opts.layout, Layout::Inline);
        assert_eq!(opts.side_width, DEFAULT_SIDE_WIDTH);
    }

    #[test]
    fn render_inline_dispatch_matches_direct() {
        let files = parse(BASIC).unwrap();
        let opts = ProjectionOptions {
            layout: Layout::Inline,
            side_width: 40,
        };
        assert_eq!(render(&files, opts), render_inline(&files));
    }

    #[test]
    fn render_side_dispatch_matches_direct() {
        let files = parse(BASIC).unwrap();
        let opts = ProjectionOptions {
            layout: Layout::SideBySide,
            side_width: 40,
        };
        assert_eq!(render(&files, opts), render_side_by_side(&files, 40));
    }

    #[test]
    fn layout_from_str_recognizes_inline() {
        assert_eq!(layout_from_str("inline"), Some(Layout::Inline));
    }

    #[test]
    fn layout_from_str_recognizes_split_aliases() {
        assert_eq!(layout_from_str("split"), Some(Layout::SideBySide));
        assert_eq!(layout_from_str("side-by-side"), Some(Layout::SideBySide));
    }

    #[test]
    fn layout_from_str_rejects_unknown() {
        assert_eq!(layout_from_str("zigzag"), None);
    }

    #[test]
    fn render_empty_model_is_empty() {
        assert_eq!(render(&[], ProjectionOptions::default()), "");
    }

    #[test]
    fn render_does_not_mutate_model() {
        let files = parse(BASIC).unwrap();
        let before = files.clone();
        let _ = render(&files, ProjectionOptions::default());
        assert_eq!(files, before);
    }
}
