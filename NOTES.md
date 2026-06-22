# Notes

Durable lessons for future Deep-Diff-Forge sessions. This is not a diary.

## 2026-06-22 (refactor: parser strictness, socket type, highlight, structural)

- Lesson: a unified-diff parser must enforce each hunk's declared `@@ -a,b +c,d @@`
  counts EXACTLY, or "patch truth" leaks. The lenient version closed a hunk at a
  new header / EOF regardless of remaining counts (silent truncation). Fix: make
  hunk-closing fallible (`TruncatedHunk` when counts remain) and reject over-fills
  in `consume_body` (`HunkLineCountMismatch`). One existing test used a malformed
  patch (`@@ -1,2 +1,1 @@` with 2 removals, 0 new-side lines) — the stricter
  parser correctly rejected it; the test, not the parser, was wrong.
- Lesson (Zen): prefer a parse-don't-validate type over scattered `Option<PathBuf>`
  handling. `SocketLocation::resolve()` is the one entry point; "no location" is
  unrepresentable past construction; `bind`/`connect`/`path` are methods. The CLI
  nested `if let` collapsed to one call.
- Lesson: tree-sitter syntax highlighting needs no `tree-sitter-highlight` crate
  (which would force a tree-sitter major bump) — load the grammar's
  `HIGHLIGHTS_QUERY` into a `tree_sitter::Query` and iterate `QueryCursor::captures`
  (`tree_sitter::StreamingIterator` is re-exported). SECURITY: route span text
  through `display_safe` so the only raw ESC in coloured output is your fixed SGR
  code — highlighting attacker source is otherwise an injection vector.
- Lesson: flat-token LCS gives a real *reformat-aware* structural diff (whitespace
  isn't a token, so layout-only edits diff to nothing) and clean add/remove, but
  it CANNOT robustly detect moves: shared punctuation (`(`,`)`,`;`,`{`,`}`) gets
  LCS-matched in place, fragmenting any moved block. Robust moves need the tree
  (difftastic). Keep move detection best-effort + say so; don't over-claim.

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

- Lesson: ratatui TUIs are testable headlessly via `ratatui::backend::TestBackend`
  — render into it, read the `Buffer` cells back as strings, assert on content.
  Keep all decision logic in a pure state model; only the live `crossterm` event
  loop needs a TTY (the one untested boundary). The CLI `review --probe` reuses
  the same `render_to_lines` helper for a no-TTY live proof.
- Evidence: `deep-diff-forge-tui` ui.rs tests + `review --probe` output.
- Affected files/symbols/commands: `crates/deep-diff-forge-tui/src/ui.rs`.
- Status: permanent.

- Lesson: pulling ratatui surfaces an unmaintained transitive (`paste`,
  RUSTSEC-2024-0436 — not a vulnerability). Handle accepted unmaintained-transitives
  with a DOCUMENTED, scoped `ignore` entry in `deny.toml` (never by disabling the
  advisories check). The supply-chain gate working as designed.
- Evidence: `cargo deny check advisories` flagged it; scoped ignore → all ok.
- Affected files/symbols/commands: `deny.toml`.
- Status: permanent.

- Lesson: the CLI's `println!` panics on a broken pipe (`… | head`) — Rust
  ignores SIGPIPE so EPIPE becomes a panic (exit 101). Pre-existing CLI-wide
  since L1; output is not corrupted. Resolved at L9 (v0.2.0) by routing bulk
  output through a broken-pipe-tolerant writer (`emit`/`emitln!`) that exits `0`
  on `ErrorKind::BrokenPipe` — std-only, no `unsafe`, no new deps. Chose the
  per-write approach over resetting the SIGPIPE disposition precisely to keep
  the zero-`unsafe` invariant (`signal()` would need `libc` + `unsafe`).
- Evidence: `printf … | deep-diff-forge --stdin-patch --json | head -1; echo $?`
  → 0 (was 101).
- Affected files/symbols/commands: `crates/deep-diff-forge-cli/src/main.rs::emit`.
- Status: resolved (v0.2.0).

## 2026-06-22 (L9 Learning)

- Lesson: an N-crate publishable workspace should inherit version + internal
  deps from `[workspace.package].version` + `[workspace.dependencies]`. A version
  bump then touches one line, and the path+version pair every internal dep needs
  for `cargo publish` (and cargo-deny `wildcards = "deny"`) can't drift between
  crates. `cargo publish --dry-run -p <leaf>` is the only ground truth for
  publish-readiness — it would have caught that `core`/`cli` lacked the mandatory
  `description` even though the registry token was the flagged blocker.
- Evidence: `cargo publish --dry-run -p deep-diff-forge-core` packaged + verified
  + reached "Uploading … aborting upload due to dry run".
- Affected files/symbols/commands: `Cargo.toml`, every `crates/*/Cargo.toml`.
- Status: permanent.

- Lesson: edition 2024 makes `std::env::set_var` `unsafe`. To keep a
  zero-`unsafe` crate, never mutate process env in tests — factor env-reading
  code into a pure resolver (`resolve_learning_dir(xdg, home)`) that takes the
  values as args, and test that. Also de-couples otherwise-racy parallel tests.
- Evidence: `deep-diff-forge-learning::store::resolve_learning_dir` + tests;
  `#![forbid(unsafe_code)]` on the crate compiles clean.
- Affected files/symbols/commands:
  `crates/deep-diff-forge-learning/src/store.rs`.
- Status: permanent.

- Lesson: a UDS daemon is testable without a TTY/long-running process. Use
  `std::os::unix::net::UnixStream::pair()` to cover `handle_connection` in
  process, and spawn `run_server` on a thread with a `request` client to cover
  the real accept loop + graceful shutdown. Keep the request/dispatch logic
  socket-free (`process_line`/`dispatch`) so most of it tests without any socket.
  std sockets removed any need for `tokio` at L7.
- Evidence: `deep-diff-forge-daemon` serve.rs tests (socketpair + run_server).
- Affected files/symbols/commands: `crates/deep-diff-forge-daemon/src/serve.rs`.
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

## 2026-06-22 (security hardening S1008412)

- Lesson: a diff/review CLI's top threat is terminal/ANSI-escape injection —
  attacker-controlled diff bodies, paths, and symbol names rendered raw can
  hijack the reviewer's terminal (scrollback forgery, OSC-52 clipboard, title
  spoof). Sanitize at the HUMAN-RENDER BOUNDARY (`core::display_safe`, escaping
  control chars to visible `\xHH`, keeping tab), not in an unused helper. A
  `sanitize_body` that exists + is tested but has zero call sites is the
  silent-gap fail-open: tested protection, bypassed load path. Machine output
  (`json_escape`) must also escape DEL + the C1 block (0x7f-0x9f, incl. 8-bit
  CSI/OSC), not just `< 0x20`.
- Evidence: `cli/tests/security.rs` feeds an ESC/OSC patch through every output
  mode, asserts no raw 0x1b; `core::util::display_safe` + tests.
- Affected: `core/src/util.rs`, `projection/src/{inline,side_by_side}.rs`,
  `cli/src/main.rs` print_* fns.
- Status: permanent.

- Lesson: a byte budget checked INSIDE the parser is too late — the CLI's
  `read_to_string` already materialised the whole untrusted input in RAM. Cap the
  READ (`take(budget + 1)`) before buffering. Same for the daemon: `BufRead::
  lines()` is unbounded; read each request line through `take(cap + 1)` +
  `read_until`. Bound is testable cheaply with a `Cursor` + small cap, not 80 MB.
- Evidence: `cli read_capped_or_exit`; `daemon serve.rs read_capped_line` + tests.
- Status: permanent.

- Lesson: `#![forbid(unsafe_code)]` per-crate is convention; `[workspace.lints.rust]
  unsafe_code = "forbid"` + `[lints] workspace = true` in every member makes "0
  unsafe" compiler-enforced and un-`allow`-able. cargo-deny v2 does NOT fail on
  `unsound` advisories by default (it passed `lru` RUSTSEC-2026-0002 silently) —
  add a strict `cargo audit --deny warnings` gate (with documented scoped
  ignores) for advisory coverage; deny.toml stays the bans/licenses/sources gate.
- Evidence: `cargo deny check` says "advisories ok" for lru; `cargo audit` flags
  it. Root `Cargo.toml [workspace.lints]`; `ci.yml` + `release.yml` audit step.
- Status: permanent.

- Lesson: the daemon's secure default is to REFUSE the world-writable `/tmp`
  socket fallback. `$XDG_RUNTIME_DIR` (`/run/user/<uid>`) is per-user, owner-only,
  kernel-managed; a predictable `/tmp` path invites symlink/TOCTOU squatting.
  Make path resolution `Option`-returning and fail closed (operator passes
  `--socket`). `set_permissions` (chmod) doubles as an ownership gate — a non-root
  process can only chmod a dir it owns — so a pre-existing attacker-owned dir
  fails closed; additionally reject symlinks via `symlink_metadata`.
- Evidence: `daemon/src/security.rs` (`runtime_base_from`, `validate_private_dir`
  symlink rejection); `cli daemon_cmd` fail-closed branch.
- Status: permanent.

- Lesson: never infer trust from an attacker-controlled free-text label.
  `source_of` substring-matched `provenance.agent` (`contains("human")`) →
  adversarial annotation escalates to Human. Fail closed: default Agent
  (untrusted), exact-match only a namespaced reserved id for System, never infer
  Human from the wire. Real trust authority = grounding (evidence), unforgeable
  by relabelling.
- Evidence: `agent/src/lib.rs::source_of` + escalation regression tests.
- Status: permanent.
