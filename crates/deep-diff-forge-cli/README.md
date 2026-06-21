# deep-diff-forge-cli

The command-line front door to **Deep-Diff-Forge** — a deterministic,
agent-collaborative diff and code-review engine that treats a diff not as text
but as a reviewable model (patch truth + semantic intent), never mutating the
patch it is showing you.

This crate builds the `deep-diff-forge` binary.

```bash
cargo install deep-diff-forge-cli
```

## Quick start

```bash
git diff | deep-diff-forge --stdin-patch                       # human review summary
git diff | deep-diff-forge --stdin-patch --json                # deep-diff-forge.review.v0
git diff | deep-diff-forge --stdin-patch --rank                # risk-ranked files
git diff | deep-diff-forge --stdin-patch --cluster --parallel 4 --json
git diff | deep-diff-forge review --probe                      # one TUI frame, headless
deep-diff-forge semantic src/lib.rs --json                     # tree-sitter symbols
deep-diff-forge learn status                                   # L9 local learning state
deep-diff-forge --help
```

Bulk output is broken-pipe-tolerant: `deep-diff-forge … | head` exits cleanly.

## Where this fits

Deep-Diff-Forge is a strictly-acyclic Rust workspace; this CLI is a thin entry
point over the engine crates (`-core`, `-patch`, `-projection`, `-pipeline`,
`-syntax`, `-graph`, `-agent`, `-tui`, `-cluster`, `-learning`, `-daemon`). Zero
`unsafe`, supply-chain-gated, dual MIT/Apache-2.0.

Full documentation, command reference, output schemas, and the deployment
framework: <https://github.com/Louranicas/deep-diff-forge>.

## License

Licensed under either of **MIT** or **Apache-2.0** at your option.
