# Security Policy

Deep-Diff-Forge is a diff and code-review engine. It is designed to be run on
**fully untrusted input**: an attacker controls the entire unified diff (paths
and line bodies), the source files handed to the semantic layer, and any agent
annotations. Hardening against that input is a first-class goal.

## Reporting a vulnerability

Please report security issues **privately** — do not open a public issue for an
unfixed vulnerability.

- Preferred: open a **GitHub private security advisory** via the repository's
  **Security → Report a vulnerability** tab.
- Include: affected version/commit, a minimal reproduction (a sample patch or
  command line where possible), the impact, and any suggested fix.

We aim to acknowledge a report within **5 business days** and to ship a fix or a
documented mitigation for confirmed High/Critical issues as a priority.

## Supported versions

The latest released `0.x` minor version receives security fixes. Pre-1.0, only
the most recent minor line is supported.

## Threat model & guarantees

- **No `unsafe`.** `unsafe_code = "forbid"` is enforced workspace-wide via
  `[workspace.lints]`, so the guarantee is compiler-checked, not convention.
- **Terminal-safe rendering.** All attacker-controlled strings (diff bodies,
  file paths, symbol names) are passed through `core::display_safe`, which
  escapes terminal control sequences (ANSI/CSI/OSC, CR, BEL, DEL, C1) to a
  visible `\xHH` form before they reach a terminal. Machine output
  (`--json`/`--jsonl`) routes through one canonical `core::json_escape`, which —
  beyond the RFC 8259 control set — also escapes `DEL` and the C1 block
  (`0x7f..=0x9f`, including the 8-bit CSI/OSC introducers) to `\u00xx`, so JSON
  printed to a terminal is safe too. Every JSON sink shares this one escaper (no
  per-module forks).
- **Trojan Source defence (CVE-2021-42574).** `display_safe` also neutralises
  bidirectional and invisible Unicode (RLO/LRO/PDF/isolates `U+202A–U+202E` /
  `U+2066–U+2069`, directional marks, zero-width characters, BOM) to a visible
  `\u{XXXX}` — so attacker source cannot *display* differently than it logically
  reads, a defence directly on-mission for a code-review tool.
- **Bounded input.** Stdin, source files, and daemon request lines are read
  under hard byte caps. Patch parsing also has a one-million-line structural
  cap to bound the expansion of tiny input lines into larger model objects.
- **Daemon least-privilege.** The optional UDS daemon creates an owner-private
  (`0700`) runtime directory and a `0600` socket. Directory ownership is checked
  against the effective user; symlinks and inode swaps are rejected. There is
  **no world-writable `/tmp` fallback**. Request and response lines have absolute
  deadlines and size caps, worker concurrency is capped at eight, retained
  session payloads are capped at 128 MiB, and request-dispatch panics are
  contained. Socket cleanup only removes the inode created by the server.
- **Fail-closed trust.** Agent annotations are untrusted until *grounded*
  (evidence-backed); annotation `source` is never inferred from an
  attacker-controlled label.
- **Local-only learning.** The L9 learning store accepts only redacted hex file
  identifiers and bounded metadata tokens, under owner-private (`0700`/`0600`)
  ownership and permissions. Directory/file symlinks and inode swaps are
  rejected; file, line, and record counts are capped. The stable FNV identifier
  is pseudonymous, not cryptographically one-way, so confidentiality comes from
  the local access controls. Learning data is never uploaded.

## Supply chain

- `cargo deny` (bans, licenses, sources, advisories) and a strict `cargo audit`
  gate run in CI **and** as a hard prerequisite of the irreversible crates.io
  publish. There are no advisory waivers: warnings are denied.
- The `tree-sitter` crates (which run a C build script) are pinned to exact
  versions; install with `cargo install --locked`.
- All GitHub Actions are pinned to commit SHAs (tag-hijack defence), and the
  release workflow emits SLSA build-provenance attestations for the binary, its
  checksum, and the SPDX SBOM (`sbom.spdx.json`, generated and CI-gated).
  Releases run only from tags whose name exactly matches the workspace version;
  checkout credentials are not persisted. Dependencies and actions are tracked
  by Dependabot.
- A `cargo-fuzz` harness (`fuzz/`) covers the patch parser, review JSON, daemon
  protocol, and agent annotations; CI gates that it compiles.

## Hardening provenance

The current posture follows four reviews, each with verification outside the build
loop and every confirmed finding remediated with a fail-before/pass-after test:

- **S1008412** — an 8-dimension STRIDE audit with independent verification
  produced a CVSS-scored register (17 confirmed; **no Critical/High**), all
  remediated; a final review fleet added Trojan-Source/bidi defence.
- **S1008443** — a bias-controlled re-review hardened patch-truth (mandatory
  hunk-header closer), the daemon (fail-closed `bind_explicit`, bounded LRU
  sessions), and the CI/release supply chain.
- **S1008452** — a 7-facet posture review (88/100) closed its one High by
  unifying the `--json`/`--jsonl` escapers with `core::json_escape` (C1/DEL
  coverage). No Critical/High remained after that review.
- **SOL-1 (2026-09-04)** — removed two stale advisory waivers, bounded daemon
  concurrency and aggregate memory, added absolute I/O deadlines and inode-safe
  local storage, enforced receipt privacy at the write boundary, and closed a
  non-tag release-publication path. Full tests, strict Clippy/docs, fuzz-target
  compilation, RustSec/cargo-deny, privacy probe, and hostile daemon soak pass.
