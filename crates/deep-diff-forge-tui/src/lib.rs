//! Review-first terminal UI for Deep-Diff-Forge (L5c).
//!
//! The interactive review cockpit. Its decision logic — navigation, selection,
//! layout toggles, scrolling — lives in a pure, fully-tested state model
//! ([`ReviewApp`]); key bindings map to semantic events ([`map_key`]); and the
//! ratatui render is exercised headlessly via [`render_to_lines`] (a
//! `TestBackend`). Only the live event loop ([`run`]) needs a real terminal and
//! is the single untested boundary.

mod input;
mod run;
mod state;
mod ui;

pub use input::map_key;
pub use run::run;
pub use state::{AppEvent, LayoutMode, ReviewApp};
pub use ui::{render, render_to_lines};
