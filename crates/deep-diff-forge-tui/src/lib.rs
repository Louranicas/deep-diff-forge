//! Review-first terminal UI for Deep-Diff-Forge (L5c).
//!
//! The interactive review cockpit. Its decision logic — navigation, selection,
//! layout/fold/notes/theme toggles, focus, scrolling — lives in a pure, fully
//! tested state model ([`ReviewApp`]); key bindings map to semantic events
//! ([`map_key`]); and the ratatui render is exercised headlessly via
//! [`render_to_lines`] (a `TestBackend`). Only the live event loop ([`run`])
//! needs a real terminal and is the single untested boundary.
//!
//! The screen composes a menu bar, a ranked directory tree of changed files
//! (with status, ± counts, and inline-note badges), and a syntax-highlighted
//! diff pane — inline or side-by-side — that folds long unchanged context and
//! renders the engine's own findings as anchored, grounded inline notes
//! ([`engine_annotations`]). Every attacker-controlled string passes through
//! `display_safe` before it reaches a cell.

mod chrome;
mod command;
mod diffview;
mod input;
mod notes;
mod paint;
mod run;
mod state;
mod theme;
mod tree;
mod ui;

pub use command::{Command, CommandOutput};
pub use input::{map_key, map_mouse};
pub use notes::{engine_annotations, file_annotations, hunk_annotations};
pub use run::run;
pub use state::{AppEvent, Focus, LayoutMode, Overlay, ReviewApp};
pub use theme::{Palette, ThemeKind};
pub use ui::{render, render_to_lines};
