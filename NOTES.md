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

- Lesson: The unified-diff parser must track hunk line-counts (`@@ -a,b +c,d @@`)
  to know when a hunk ends; otherwise a removed line whose content begins with
  `-- ` is misread as the next file's `--- ` header.
- Evidence: `parser::tests::removed_line_content_dashes_are_not_a_new_file` and
  the `rem_old`/`rem_new` counters in `HunkBuf`.
- Affected files/symbols/commands:
  `crates/deep-diff-forge-patch/src/parser.rs::HunkBuf::consume_body`.
- Status: permanent.

- Lesson: `render_unified` is deliberately model-stable, not byte-identical. It
  must NOT re-emit the `\ No newline at end of file` marker in the file header —
  that would break `git apply`. The marker is preserved in parse metadata only.
- Evidence: `round_trip::no_newline_hunks_round_trip_even_though_marker_is_dropped`.
- Affected files/symbols/commands:
  `crates/deep-diff-forge-patch/src/render.rs::render_unified`.
- Status: permanent.

- Lesson: JSON output is hand-rolled (zero deps) because serde is absent from
  the offline cargo cache and the workspace is intentionally dependency-free at
  L1. The string escaper must cover `"`, `\`, control chars (`\u00xx`), and
  pass UTF-8 through. serde is the planned upgrade once a projection crate lands.
- Evidence: `json::tests::quote_*`; `cargo` registry cache had no `serde-*`.
- Affected files/symbols/commands:
  `crates/deep-diff-forge-patch/src/json.rs::quote`.
- Status: permanent. The canonical shared escaper is now
  `deep_diff_forge_core::json_escape` (added at L4); prefer it for new code.

- Lesson: `cargo deny`'s `wildcards = "deny"` flags version-less intra-workspace
  path deps; `allow-wildcard-paths` does NOT exempt publishable crates. Fix is
  to give each internal path dep a `version` (e.g. `{ path = "..", version =
  "0.1.0" }`) — the publishable-workspace pattern — not to relax the ban.
- Evidence: `cargo deny check bans` error `allow-wildcard-paths ... does not
  apply to public crates`; resolved → `advisories ok, bans ok, licenses ok`.
- Affected files/symbols/commands: every `crates/*/Cargo.toml` internal dep;
  `deny.toml`.
- Status: permanent.

- Lesson: tree-sitter (0.25 + grammar 0.24) fetches and C-compiles cleanly in
  this environment via the `cc` crate; it pulls `serde_json`/`regex`
  transitively, which is exactly why the supply-chain `deny.toml` was landed
  first. Semantic budgets that are real: byte budget (pre-parse) and node
  budget (post-parse via `descendant_count`). Time budget is NOT enforced yet
  (would need the parser progress-callback API) — never report it as a fallback.
- Evidence: `deep-diff-forge-syntax` builds + `analyze::tests::node_budget_*`.
- Affected files/symbols/commands:
  `crates/deep-diff-forge-syntax/src/analyze.rs`.
- Status: permanent.
