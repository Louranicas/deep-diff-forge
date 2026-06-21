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
  (`--json`/`--jsonl`) is neutralised by `json_escape`.
- **Bounded input.** Stdin, source files, and daemon request lines are read
  under a hard byte cap, so a pathological or unbounded stream degrades to a
  graceful error instead of exhausting memory.
- **Daemon least-privilege.** The optional UDS daemon creates an owner-private
  (`0700`) runtime directory (symlinks rejected; `chmod` is the ownership gate)
  and a `0600` socket. There is **no world-writable `/tmp` fallback**: without
  `$XDG_RUNTIME_DIR` it fails closed and the operator passes `--socket PATH`.
  Connections carry a read timeout, requests are size-bounded, and a panic in
  request dispatch is contained so one abusive client cannot stop the daemon.
- **Fail-closed trust.** Agent annotations are untrusted until *grounded*
  (evidence-backed); annotation `source` is never inferred from an
  attacker-controlled label.
- **Local-only learning.** The L9 learning store holds hashes, counts, and
  timings — never source or paths — under owner-private (`0700`/`0600`)
  permissions, and is never uploaded.

## Supply chain

- `cargo deny` (bans, licenses, sources, advisories) and a strict `cargo audit`
  gate run in CI **and** as a hard prerequisite of the irreversible crates.io
  publish. Two transitive, unreachable advisories (`RUSTSEC-2024-0436` paste,
  `RUSTSEC-2026-0002` lru — both via `ratatui`) are explicitly accepted and
  documented in `deny.toml`; any *new* advisory fails the gate.
- The `tree-sitter` crates (which run a C build script) are pinned to exact
  versions; install with `cargo install --locked`.
- Dependencies and GitHub Actions are tracked by Dependabot. For maximum
  assurance, pin third-party actions to commit SHAs.

## Hardening provenance

The current posture follows an adversarial security review (S1008412): an
8-dimension STRIDE audit with independent verification produced a CVSS-scored
register; every confirmed finding was remediated with a fail-before/pass-after
regression test. No Critical/High findings were confirmed.
