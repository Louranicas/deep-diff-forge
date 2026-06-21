# Notes

Durable lessons for future Deep-Diff-Forge sessions. This is not a diary.

## 2026-06-21

- Lesson: Bootstrap Rust gates must pin build output to repo-local `target` to
  avoid read-only global cache failures in this workspace.
- Evidence: `CARGO_TARGET_DIR=target cargo check --workspace` completed with
  `Finished dev profile`; earlier unpinned `cargo run` attempted to use a
  read-only home cache.
- Affected files/symbols/commands: `justfile`, `CARGO_TARGET_DIR=target`,
  `cargo check --workspace`.
- Status: permanent.

- Lesson: Pedantic clippy is part of the Distinguished Agentic Rust Coder V4
  deployment gate; constructors/builders returning values need `#[must_use]`
  when clippy requests it.
- Evidence: `CARGO_TARGET_DIR=target cargo clippy --workspace --all-targets --
  -D warnings -W clippy::pedantic` reported `this method could have a
  #[must_use] attribute` for `ReviewDocument::empty`.
- Affected files/symbols/commands:
  `crates/deep-diff-forge-core/src/lib.rs::ReviewDocument::empty`,
  `just gate-feature`.
- Status: permanent.
