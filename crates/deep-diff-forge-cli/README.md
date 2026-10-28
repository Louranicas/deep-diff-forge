# deep-diff-forge-cli

The command-line front door to **Deep-Diff-Forge**, a deterministic code-review
engine for diffs. It parses a unified or Git patch into a reviewable model,
ranks the files by review risk, and never changes the patch it shows you.

This crate builds the `deep-diff-forge` binary (version 0.3.0). It is a local,
single-user tool for Linux and macOS.

## Install

crates.io publication is on hold until the project is production-hardened, so
`cargo install deep-diff-forge-cli` does not work yet. Use one of these:

- **Release binaries:** the [GitHub releases](https://github.com/Louranicas/deep-diff-forge/releases)
  ship `deep-diff-forge-<tag>-<target>.tar.gz` archives with `.sha256` files
  for `x86_64-unknown-linux-gnu`, `aarch64-apple-darwin` and
  `x86_64-apple-darwin`. Published releases: v0.1.0 and v0.2.0; 0.3.0 is not
  tagged yet.
- **From source** (Rust 1.88 or newer):

  ```bash
  git clone https://github.com/Louranicas/deep-diff-forge.git
  cd deep-diff-forge
  CARGO_TARGET_DIR=target cargo build --release --locked -p deep-diff-forge-cli
  target/release/deep-diff-forge --version
  ```

## Quick start

```bash
git diff | deep-diff-forge --stdin-patch                 # human review summary
git diff | deep-diff-forge --stdin-patch --json          # deep-diff-forge.review.v0
git diff | deep-diff-forge --stdin-patch --rank --json   # deep-diff-forge.rank.v0: what to look at first
git diff | deep-diff-forge --stdin-patch --layout side-by-side
git diff | deep-diff-forge review                        # interactive review cockpit (needs a terminal)
git diff | deep-diff-forge review --probe                # one cockpit frame, headless
deep-diff-forge semantic src/lib.rs --json               # tree-sitter symbols
deep-diff-forge structural old.rs new.rs --json          # real change or reformat only?
deep-diff-forge --help
```

Structured Git input runs Git itself (fixed argv, no shell) and takes paths,
statuses, modes and blob IDs from `git diff --raw -z` records. Unlike
`git diff |`, it sees untracked files and C-quoted paths, and it never writes
the index:

```bash
deep-diff-forge --git-working-tree --include-untracked --json  # staged + unstaged + untracked vs HEAD
deep-diff-forge --git-staged --rank                            # index vs HEAD
deep-diff-forge --git-range main..HEAD --repo ../other --json  # commit range
deep-diff-forge --git-working-tree --acquisition-json          # deep-diff-forge.git-acquisition.v0
```

For a CI or agent gate, refuse an empty or hunkless change instead of passing it:

```bash
deep-diff-forge --git-range main..HEAD --rank --json --require-files --require-hunks   # exit 7 on refusal
```

## Surfaces

| Surface | Command | What it is |
| --- | --- | --- |
| CLI | `deep-diff-forge …` | Review, rank, cluster, Git selections, `semantic`, `structural`, `highlight`, `analyze`, `learn`, `council`. |
| Machine contract | `envelope -- <command…>`, `schema [result\|exit-codes]`, `explain <code>` | Any command's result as one `ddf.result.v1` object; the exit-code registry. |
| MCP server | `deep-diff-forge mcp` | Read-only MCP tools over stdio: `review_patch`, `rank_patch`, `explain_exit_code`, `exit_code_registry`. |
| JSON-RPC over stdio | `deep-diff-forge rpc` | The daemon's protocol (JSON-RPC 2.0, protocol 1) with no daemon. |
| Optional daemon | `deep-diff-forge daemon {path\|start\|health\|status\|stop}` | The same protocol on an owner-private Unix socket. |
| Rust SDK | crate `deep-diff-forge-sdk` | `review`, `rank`, the exit registry, and a Unix-socket `Client`. |
| Python SDK | `sdk/python/deep_diff_forge.py` in the repository | Standard library only; `StdioClient` (over `rpc`) and `SocketClient`. |

Every surface uses the same parse, guard and seal path, so one patch gets the
same `review.v0` document from each.

Not implemented yet (each is refused with exit 2): `deep-diff-forge <old> <new>`,
`chain`, `cluster` and `loom`.

## Exit codes

| Code | Meaning |
| --- | --- |
| 0 | Success. |
| 2 | Usage or argument error. |
| 3 | I/O or encoding failure. |
| 4 | Invalid input (for example a patch that does not parse, or input over the 64 MiB budget). |
| 6 | Daemon or interactive-terminal failure. |
| 7 | Input contract refused (`--require-files`, `--require-hunks`). |
| 101 | Internal error (a panic). |

`council` subcommands use their own family (0, 4, 7). `deep-diff-forge schema exit-codes`
prints the full registry.

## More

- [Root README](https://github.com/Louranicas/deep-diff-forge#readme): command
  reference, output schemas, MCP server, SDKs and the daemon.
- [API and IPC](https://github.com/Louranicas/deep-diff-forge/blob/main/docs/API_AND_IPC.md):
  JSON-RPC methods, MCP tools and socket rules.
- [Security](https://github.com/Louranicas/deep-diff-forge/blob/main/SECURITY.md):
  threat model and residual risks.

This crate is the binary over the workspace's engine crates (`-core`, `-patch`,
`-projection`, `-pipeline`, `-syntax`, `-graph`, `-tui`, `-cluster`,
`-learning`, `-daemon`, `-review`). The workspace forbids `unsafe` code and runs
supply-chain checks in CI.

## License

Licensed under either of **MIT** or **Apache-2.0** at your option.
