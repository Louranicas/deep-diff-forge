# Changelog

All notable changes to Deep-Diff-Forge are documented here. The format is based
on [Keep a Changelog](https://keepachangelog.com/), and the project follows
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Dependencies

- **Dependabot's first version updates, applied together** (#63 to #70): serde 1.0.229,
  serde_json 1.0.154, rustix 1.1.5, unicode-width 0.2.2, tree-sitter 0.26.13,
  `actions/attest-build-provenance` and `github/codeql-action` 4.39.1 (SHA-pinned). The
  individual pull requests each failed the SBOM gate (a dependency bump moves the committed
  SBOM) and the workspace bumps also left `fuzz/Cargo.lock` stale; this change regenerates both,
  with the fuzz lock moved only as far as the bumps require (its `syn` follows the workspace's
  3.0.4). tree-sitter 0.26 changed the parse progress callback to return `ControlFlow`: the
  deadline cancel now returns `Break` where it returned `true`.
- **Dependabot groups each ecosystem's updates into one pull request** (`cargo`, `fuzz`,
  `actions`), so the SBOM and fuzz lock are regenerated once per group.

### Added

- **An exact platform matrix.** The supported platforms are four release targets, listed in
  README.md#supported-platforms, SECURITY.md and RELEASE_AND_PUBLICATION.md, and
  `cli/tests/crate_edges.rs` fails when the release matrix or those docs name another set of
  target triples, when a doc omits the glibc floor, or when a doc mentions musl without stating
  it as unsupported:
  - **`aarch64-unknown-linux-gnu` is a release target**, built and run natively on GitHub's
    `ubuntu-22.04-arm` runner like the others (smoke, checksum, provenance). Not
    `ubuntu-24.04-arm`: built there, the binary had a hard need of `GLIBC_2.39`, over the floor
    (measured by the first CI run of the floor check). A `Linux arm64
    tests` job runs the whole suite there on every event: not a required check on pull requests,
    but it gates every release build through release.yml's `quality`, as `macOS tests` does.
    Both are pending their first CI run.
  - **A declared and enforced glibc floor: 2.34.** `scripts/check_glibc_floor.sh` fails the
    release smoke when a Linux binary needs a newer `GLIBC_` symbol version; a Linux-only test in
    `crate_edges.rs` runs it on the built binary (passes) and with a lowered floor (refused). Measured: the
    v0.5.0 x86_64 binary needs at most `GLIBC_2.34`, ran on glibc 2.34, 2.35 and 2.36, and was
    refused by the loader on 2.31. On glibc 2.34 to 2.38 the loader prints one stderr warning
    per run (``weak version `GLIBC_2.39' not found``, from optional symbols in Rust's standard
    library); output and exit codes are unchanged.
  - **musl is stated as unsupported** (not built or tested), as are Windows, other
    architectures and the BSDs. The macOS minimums (11.0 arm64, 10.12 x86_64) are the ones the
    binaries declare, not run.
- **The launchd agent is loaded by a real launchd in CI.** The `macOS launchd agent` job runs
  `scripts/launchd_smoke.sh`: the plist, pointed at the built binary, is bootstrapped into
  `gui/<uid>` (or `user/<uid>`), answers `daemon health` with launchd's pid, is restarted after
  SIGKILL, stays stopped after `daemon stop` with its socket gone, and is unloaded by `launchctl
  bootout`; every wait is bounded. Not a required check, and skipped in release runs (release.yml
  calls ci.yml with the new `release: true` input) so its unproven timed waits cannot block a
  release build; its first run on a hosted Mac is pending CI (the flow was checked only against a
  fake `launchctl` on Linux).
- **Daemon metrics for Prometheus, without a listener.** `daemon metrics --format prometheus`
  prints the `daemon.metrics` answer in the Prometheus text format 0.0.4 (`ddf_daemon_*` names,
  `_total` counters, seconds and bytes in the names, escaped labels, a latency histogram with
  cumulative `le` buckets and no `_sum`, which the daemon does not keep), and `--out PATH` writes
  it whole for node_exporter's textfile collector: a temporary file in the same directory, then a
  rename; a symlink or non-regular file at PATH is refused (exit 3); mode `0644`, also under
  umask 077. A temporary left by a killed run is skipped (the next of 8 names), not reused. A daemon with no
  usable metrics exits 6 and leaves the file as it was. The JSON answer (`--format json`, the
  default) and the RPC are unchanged; both renderings read the same document
  (`deep_diff_forge_daemon::exposition`), and a strict reader in its tests checks the format's
  rules. `contrib/systemd/deep-diff-forge-metrics.{service,timer}` write it every 15 s. Not run
  against a real node_exporter or promtool (neither is installed here).
- **`deep-diff-forge [--] <old> <new>` reviews two regular files.** `patch::diff_files` runs
  `git diff --no-index` with the acquisition's hardening, from `/` on the canonical paths, with
  `core.attributesFile=/dev/null`, `GIT_ATTR_NOSYSTEM=1` and `core.autocrlf=false`; the CLI seals
  Git's exact bytes and prints every `--stdin-patch` mode (`--json`, `--jsonl`, `--rank`,
  `--cluster`, `--outline`, `--hunks`, `--layout`, the input guards). Refused, by name: a
  missing path (exit 3), a path that is itself a symbolic link, a directory or special file
  (exit 4; a linked parent directory resolves), two sizes over the 64 MiB budget, checked before
  Git reads them (exit 4). Git's answer must be one of its two shapes for a readable pair (exit 0
  with no patch, exit 1 with one) with nothing on stderr; otherwise exit 3. Without that check a
  side removed after DDF's checks (Git: exit 1, empty stdout, `Could not access`) was reviewed as
  "0 files changed" with exit 0 (review finding, reproduced with real Git and a stub). A first word that is neither a command
  nor a file stays `unknown command` (exit 2); `--` marks it as a path. Directories are out of
  scope. Measured while building it: run inside a repository, `git diff --no-index` ran that
  repository's own clean filter, and the user's `core.autocrlf` changed the compared bytes;
  `cli/tests/file_pair.rs` fails if either comes back. New errors `GitAcquireError::FileUnreadable`
  and `FileRefused` (a breaking change for an exhaustive `match` on `GitAcquireError`); the
  exit-4 registry meaning now names a refused file pair.

### Changed

- **`chain`, `cluster` and `loom` are withdrawn, not planned.** `--help` listed them, with
  `<old> <new>`, as "NOT YET IMPLEMENTED" and nobody owned them. Each is now refused with exit 2
  and its replacement, even with a second word that would otherwise be a path pair, and
  `--help` lists them under "WITHDRAWN". `chain-contract` and `cluster-contract` list only
  shipped modes (a test runs each one); `cluster-contract` names the two dimensions that run
  (patch, risk) and no longer claims per-lane budgets; `loom-contract` records the withdrawal.

- **Dependabot watches the Python client.** `.github/dependabot.yml` gains a weekly `pip` entry
  for `sdk/python` (its only requirement is the setuptools build backend), with the same 7-day
  cooldown as the Cargo and Actions entries.
- **Whole-crate mutation of `core`, `patch`, `daemon` and `syntax`** (EVIDENCE.md, 2026-11-01).
  51 tests pin the mutants each crate's own tests missed: hunk addresses and digests, Git
  C-quoted paths, the raw-record and patch-block identity checks, the exact `review.v0`
  document, rename and truncation refusals in the parser, a socket line that ends at its
  newline, and the byte, node and cell budgets of the syntax crate at exactly their limit.
  Missed by the crate's own tests: `core` 22 to 3, `patch` 82 to 3, `syntax` 29 to 3, each
  remaining one equivalent or compiled only off Unix. `daemon` (16 missed, 13 timeouts) was
  re-run in workspace scope only, not whole: the workspace's tests catch all but 3 equivalents
  (2 of them caught only by `graph`'s timing test on a loaded machine) and one loop mutant that
  still times out. Tests of edit-script loops, tree walks and
  quoted-path decoding run under a 10 s deadline, so a loop that stops advancing fails its test
  instead of hanging the suite. Test code only.

- **The Python client is a package.** `sdk/python/pyproject.toml` (setuptools, no dependencies,
  Python 3.9+) installs `deep_diff_forge` as the package `deep-diff-forge`, from a checkout
  (`python3 -m pip install ./sdk/python`) or from GitHub with `#subdirectory=sdk/python`. It is
  not on PyPI. `deep_diff_forge.__version__` exists, and `cli/tests/python_sdk.rs` keeps both
  version strings equal to the workspace version.
- **The daemon drains on stop.** After `daemon.shutdown` the socket is removed and the listener
  closed (queued connections are refused at once), and the requests in progress, counted from
  their first byte until their response is flushed, are answered before the process exits, for
  up to 30 s. Idle connections are not waited for. A request read
  after the stop is refused with `RESOURCE_EXHAUSTED` (8), "the daemon is shutting down".
  Before, the process exited as soon as the accept loop returned, cutting off answers in
  progress. The log records `shutdown_drained` (level `info`) or `shutdown_drain_timeout`
  (`warn`).
- **`daemon stop` returns once the daemon has exited**, waiting up to 35 s for its start lock
  (`SocketLocation::stop_waiter`, opened before the stop so a symlinked socket path finds the
  real lock; `STOP_WAIT`), and exits 6 if it has not. Before, it returned
  while the daemon was still exiting, so a `daemon start` right after it could find the old
  daemon in the way.
- **Service units:** `contrib/systemd/deep-diff-forge.service` (systemd user unit) and
  `contrib/launchd/dev.deep-diff-forge.daemon.plist` (launchd agent, which sets
  `XDG_RUNTIME_DIR`, absent on macOS). `cli/tests/service_units.rs` keeps them consistent with the
  CLI. The systemd `ExecStop` ignores "no daemon" (`-`): systemd also runs it after a daemon
  stopped from a shell, and its exit 6 restarted the unit (measured: `NRestarts=1` without the
  `-`, 0 with it).
- **The live review loop is tested.** `deep-diff-forge-tui`'s `run` takes the terminal through one
  seam: stdout and crossterm's raw-mode, `read` and `poll` functions, passed in as values, so
  production has no adapter code of its own. Tests drive that same loop and restore with scripted
  input against a `TestBackend`: the frame drawn is the headless render, a queued burst is
  applied before one redraw, nothing queued after `q` is read, a resize redraws at the new size,
  and an ending or failing input, a failing draw, a panic or a failed restore each return their
  error (or unwind) with raw mode off and the alternate screen left. `cli/tests/review_tty.rs`
  runs bare `review` in a pseudo-terminal (Python's `pty`): the alternate screen and mouse
  capture are entered, the cockpit is drawn, `q` exits 0, and both are restored.

### Fixed

- **A failed alternate-screen entry no longer leaves the terminal in raw mode.** `review` switched
  raw mode on, then wrote the alternate-screen sequence, and armed its restore guard only after
  that write; if the write failed (stdout closed) it returned with the terminal still raw. The
  guard is now armed as soon as raw mode is on. If raw mode cannot be switched on, nothing is
  written.
- **`review` on macOS reads keys through a reader that can wait on `/dev/tty`.** The patch is
  on stdin, so crossterm reads keys from `/dev/tty`, and its default reader registers that fd
  with kqueue, which refuses it on macOS (crossterm-rs/crossterm#500): `review` would fail on
  its first key read there. The tui crate enables crossterm's `use-dev-tty` feature on macOS
  only (select(2) through `filedescriptor`; adds `filedescriptor` 0.8.3 and `thiserror` 1 to
  the macOS build). Inferred from source, not observed on a Mac: the `macOS tests` job runs
  `cli/tests/review_tty.rs`, which is the check.
### Changed

- **The evaluation corpus has 34 cases, and declared misses count.** `fixtures/eval` grew from
  11 to 34 cases (14 ranking, 17 evidence, 3 findings), each with a classified origin: a CVE,
  GHSA or RUSTSEC shape (CVE-2021-42574, CVE-2023-49291, CVE-2024-32002, GHSA-pjwm-rvh2-c87w,
  RUSTSEC-2022-0042), a real-world shape, one of this project's findings, or `synthetic:` with
  its reason. Three are hard negatives, where a risky-looking file must not outrank.
  - A declared miss now counts as wrong in its metric; before, the one known miss was left out
    of recall. A ranking pair can be declared (`known_miss`), and so can an evidence false
    positive (`known_false_positive`). The receipt lists them in a new `declared` field, and the
    summary prints each as `KNOWN …` under a `declared` count (it printed `known misses`).
    `known_misses` keeps its meaning, the evidence `known_miss` items (7); `declared` lists
    every kind (11). A declared miss the engine gets right fails the run.
  - The engine was not changed. It gets 11 cases' labels wrong, all declared: ranking 12/15,
    evidence precision 21/22, recall 21/28, findings 33/33 (measured). `cli/tests/eval.rs`
    holds those values as its floors, which were 100% over a smaller corpus.
  - The manifest is refused (exit 4) when a file of the corpus is named by no case, or a case
    has no origin, the id of another case, or a field its kind does not read, when a ranking
    pair is not exactly two distinct paths (`[p, p]` can never hold, so as a `known_miss` it
    could never go stale; before, it was a wrong label, exit 7), and when the corpus holds a
    symlink (it could lead out of the corpus). A case can name only a file of the corpus
    directory (exit 3 otherwise).

### Security

- **A filter in a repository's `config.worktree` no longer runs during Git acquisition.**
  Acquisition turned off only the filters `git config --local` lists, which does not read
  `$GIT_DIR/config.worktree` (used once `extensions.worktreeConfig` is set), so a filter defined
  there ran and its output was reviewed in place of the file. Acquisition now lists every scope
  with `--show-scope` and turns off each filter any unprotected scope defines (local, worktree,
  their includes, and any scope it does not know). Filters from the user's global and system
  configuration, such as Git LFS, stay on by decision (SECURITY.md); a test with planted global
  and system files proves that nothing else they name (fsmonitor, external diff, textconv, a
  diff driver command, set directly or through `include.path`/`includeIf`) runs.
- **A repository filter whose name is not UTF-8 refuses the acquisition.** The name was
  decoded lossily (`\xff` became U+FFFD) before DDF named it to Git, and Git matches a subsection
  name byte for byte, so the override named another filter and the repository's own ran. Passing
  the raw bytes fixed it on Linux, but the macOS CI job measured the filter still running: macOS
  Git's `--get-regexp ^filter\.` did not match a key that is not UTF-8, so the filter was never
  listed. Acquisition now lists every key (`git config --list`) and picks the `filter.` keys
  itself, and any such name refuses the acquisition on every platform (`InvalidSelection`).
- **The caller's `GIT_CONFIG_COUNT`/`KEY`/`VALUE` configuration is kept.** DDF numbered its
  filter overrides from 0 and set the count, which replaced the caller's own entries (all of them
  when the repository defined no filter). They are now numbered after the caller's count (from
  `GitAcquireOptions::env`, else inherited); a count Git reads but DDF does not (a leading
  space) is refused as `InvalidSelection`.
- **Every release binary is run before it ships.** The x86_64 macOS binary was cross-built and
  only checksummed; the release smoke now runs it under Rosetta 2 (installed if missing) after
  checking it is x86_64. A test fails if any release target is built without being run.
- **The crates.io job fails closed.** With the owner's gate open (`DDF_CRATES_IO=open`) and no
  `CARGO_REGISTRY_TOKEN`, it used to exit 0 without publishing; it now fails. It runs in a
  `release` environment that only `v*` tags can deploy to.
- **Release tags are immutable.** A tag ruleset (`refs/tags/v*`: no deletion, no update, no
  non-fast-forward, no bypass) stops a published tag being moved to other code.
- **Code scanning.** `codeql.yml` runs CodeQL (security-extended) over the Rust code and the
  workflows on pull requests, pushes to `main` and weekly. Not a required check yet.
- **Pinned actions are enforced.** `cli/tests/workflow_pins.rs` fails when any `uses:` in any
  workflow is not a local path or a full 40-hex commit SHA.

### Fixed

- **A `review.hunks` selection that repeats a name can no longer abort the daemon.** Each repeat
  of a path in `paths` pushed every hunk of that file again, so 50,000 repeats of one path over
  4,000 hunks (a 619 KB request) asked for 3.2 GB; under a 2 GB address-space limit the
  allocation failed and the daemon aborted (`SIGABRT`, which `catch_unwind` cannot contain),
  losing every session and leaving its socket and lock behind. Repeated `addresses` duplicated
  the same way. A repeated name now selects its hunks once, keeping the order of first mention,
  so a selection never holds more hunks than the review has; the answer to `["x.rs","x.rs"]` is
  the answer to `["x.rs"]` (`views.rs` `select`). Also reached through `review.change`
  `view=hunks` and the MCP `review_hunks` tool.
- **`review.hunks` by address hashes each hunk once per request.** Every requested address
  recomputed every hunk's address (a SHA-256 of the hunk), so the cost grew with addresses times
  hunks: 4,000 addresses over 4,000 hunks (a 490 KB request) held a worker for 18 s, measured on
  the release build, while admission weighed it by its line length. The selection now builds one
  address table per call, as `findings.verify` has since DDF-PERF-001, and looks each address up
  in it; paths are looked up through a path table the same way.
- **Error answers are held to the 80 MiB line cap.** Only a success was bounded, and an error
  repeats request text: one unknown path of 27 million U+202E characters (an 81 MB request) drew
  a 243 MB error line, because `display_safe` writes each 3-byte character as the 8-byte
  `\u{202e}`, and a client using the daemon's own `request` helper got "daemon response exceeds
  maximum size" instead of the `INVALID_PARAMS` meant. The cap was also measured before
  `json_line_safe`, which writes each line separator in 6 bytes rather than 3, so an id of
  U+2028 characters came back at twice its size. Now an error message is cut at 4 KiB
  (`MAX_ERROR_MESSAGE_BYTES`, at a character boundary, with the count of bytes left out), an
  unknown address or path is repeated within 1 KiB and an unknown param name or session id
  within 256 bytes (`display_safe_bounded`), and every answer `process_line` returns is measured
  as written; one over the cap becomes `RESOURCE_EXHAUSTED` (8), without the id when the id alone
  would put it over.
- **MCP tool errors are held to the same 4 KiB message bound as the socket.** The cut was only
  in the socket's `error_response`, and the MCP server reads a daemon error's message directly,
  so `review_change` with a repo of 100,000 U+202E characters (a 600 KB request) answered with a
  901 KB tool error (measured, release build); `review.change` repeated each request-named path
  or value whole, three times its size for bidi characters, and the MCP server repeated an
  unknown tool, argument or method name whole. Now `RpcError::new` cuts the message where it is
  built, `review.change` repeats a request-named path or value within 1 KiB, the MCP server
  repeats a tool, argument or method name within 1 KiB, and every MCP tool and protocol error is
  cut to `MAX_ERROR_MESSAGE_BYTES`; the same request now answers in 1.3 KB.
- **`review.hunks` by address holds memory for the requested addresses, not for every hunk.**
  The address table kept every hunk's 64-character address, whatever was asked, so one address
  over 300,000 one-line hunks peaked at 195 MB against 130 MB for a path selection (measured,
  release build), memory that admission does not weigh. The table now keeps only the requested
  addresses a hunk has, each hunk's address still computed once; the same request peaks at
  130 MB.
- **`daemon health` and `daemon stop` no longer say there is no daemon when it is at its
  connection limit.** A daemon serving eight connections closes a ninth at once, unanswered (by
  design), and the client's reset or broken pipe was reported as `no daemon at PATH`: eight idle
  connections made the CLI report a running daemon absent for 30 s, and eight that each send a
  request within 30 s kept it so indefinitely (measured). A reset, broken pipe, early end of file
  or read timeout now reads `no answer from the daemon at PATH (it may be at its connection limit;
  retry)`, still exit 6; `no daemon at` is kept for a missing or refused socket.
- **Refused request lines are counted in `daemon.metrics`.** A line that is not UTF-8, one over
  the 80 MiB cap and one sent during a stop were answered by the connection itself and never
  counted, so `ddf_daemon_requests_total{method="(unknown)"}` and its `parse_error`,
  `invalid_request` and `resource_exhausted` errors under-reported them (three of four parse
  errors were missing in a measured run). They are now counted under `(unknown)`, on the socket
  and in `deep-diff-forge rpc` (`serve::refusal`).
- **The daemon's memory guidance describes parallel requests.** `docs/OPERATIONS_AND_DAEMON.md`
  still said the daemon "runs one request at a time", from before 0.4.0; it now sizes the peak
  for the requests admission lets run together (two at the 64 MiB budget) and for the waiting
  lines, which are parsed before admission and not counted in its budget.
- **`envelope` wraps the wrapped command's `--help`.** `envelope -- --stdin-patch --help` and
  `envelope -- council --help` printed envelope's own help as plain text with exit 0, ran
  nothing and printed no `ddf.result.v1` object, because the help scan read every word, including
  those after `--`. The scan (`args::help_requested`) now stops at `--` and skips an option's
  value, so `--hunks --path -h` selects the file named `-h` and `review --findings -h` reads it,
  where both printed help. `review` takes the word `help` only first (`review help`): a later
  `help`, as in `review --side help`, printed the help with exit 0 and is now refused as an
  unexpected argument (exit 2). `council` keeps its own scan, so `council … --session -h` still
  prints the council help.
- **A second output mode is refused, not dropped.** `--jsonl --rank` printed only JSONL,
  `--json --layout inline` only JSON, `--cluster --layout inline` only the cluster,
  `--acquisition-json --rank` only the acquisition, and `--jsonl --hunks --path nothing` skipped
  the path check, each with exit 0. `--jsonl`, `--layout`, `--acquisition-json` and a `--json`
  that is not the format of `--rank`, `--cluster`, `--outline` or `--hunks` now join the one-mode
  check (exit 2, before input is read), and `--address` with `--path` is refused there too: it
  was checked only after the patch was read, so a patch that failed to parse exited 4 instead.
- **A single-valued option given twice is refused.** The first value was used and a later one
  was never read or validated: `--parallel 4 --parallel bogus` ran with 4, `--max-bytes 1000000
  --max-bytes 2048` printed a page 34 times the appended budget, and `--repo . --repo /nonexistent`
  reviewed `.`, all with exit 0. `args::admit` now exits 2 (`--parallel given more than once`);
  `--address`, `--path`, `--allow-repo` and `--workload` stay repeatable.
- **`chain-contract` states the shipped pipe and stdin behaviour.** It promised that pipe
  failures exit non-zero, but a closed stdout (`| head`) deliberately exits 0, and that stdin is
  read only with an explicit flag, but `findings verify` and `review` read the patch there
  without one. It now says a closed pipe exits 0, any other write failure exits 3, and names
  every command that reads stdin; `docs/CHAINING_AND_CLUSTERING.md` says the same.
- **A failed write of output without a final newline exits 3.** Stdout is line-buffered, so a
  tail after the last newline was written at exit, where the error is dropped:
  `highlight` of a file with no final newline into `/dev/full` exited 0 with the output lost.
  `emit` and `envelope` now flush and report the failure (exit 3; a closed pipe stays exit 0).
- **MCP `review_patch` with `paths` summarises the files it returns.** The `summary` kept the
  whole patch's totals, so a `review.v0` holding one file said `files_changed: 2`, and in
  `review-summary.v0` too; `selected` echoed a repeated path. The summary is now counted over
  the kept files (the seal stays the whole patch's) and `selected` lists each path once.
- **`envelope` exits 128 + N when signal N ends the wrapped command.** It exited 1, which the
  exit-code registry does not list, so `explain` could not describe it. 128 + N is the status a
  shell reports for the same command (`exit::SIGNAL_BASE`, not a registry code); the envelope
  still says `outcome: signal` and the signal number.
- **The long-line linearity test no longer fails on a busy host.** `graph`'s
  `the_import_rule_is_linear_on_long_lines` bounded its scans by 10 s of wall time; at load 56
  to 89 a linear scan using 1.8 s of CPU took over 10 s, failed the suite, and turned diff
  mutation results into false "caught" verdicts. It now bounds the scans' own thread CPU time
  (Linux `/proc/thread-self/schedstat`, wall time elsewhere) at 15 s, and keeps a 120 s
  wall-clock deadline only against hangs. Measured: linear code passes under 64 busy loops
  (20 s wall), and the quadratic import rule it guards against (`eee2b4c^`) still fails at 66.7 s
  of CPU time. Shrinking the lines was rejected: at 20,000 units the quadratic rule took 0.4 s.
- **Serde-built JSON no longer prints raw DEL or C1 controls** (bug hunt 2026-11-01). The
  `--hunks` and `--outline` views, `rpc`, MCP, the daemon socket and the review CLI are written
  by serde_json, which escapes only C0, and then passed through `core::json_line_safe`, which
  escaped only `U+2028`/`U+2029`; a patch path, `@@` heading or line holding the 8-bit CSI
  (`U+009B`) or OSC (`U+009D`) introducer reached the terminal raw, against SECURITY.md's DEL/C1
  promise. The pretty-printed documents (`findings verify`, `deploy doctor --json`, `eval --json`
  and `bench --json`) skipped `json_line_safe` altogether, so `findings verify` printed a
  finding's patch path, and `deploy doctor` its `--socket` path, with the raw CSI. `json_line_safe`
  now escapes `U+007F..=U+009F` too (the decoded document is unchanged), every pretty-printed
  document goes out through it, and the view pager and MCP's fit loop measure pages as escaped,
  so a page of C1 text still fits `max_bytes` (measured before: 14,255 bytes printed for an
  8,192-byte page).
- **Every Unicode default-ignorable code point is an `invisible` hazard and is escaped**
  (bug hunt 2026-11-01). `is_invisible` was a hand-picked list that missed the Mongolian free
  variation selectors `U+180B`-`U+180D` and `U+180F`, the Khmer inherent vowels `U+17B4`/`U+17B5`,
  `U+2065`, `U+206A`-`U+206F`, `U+FFF0`-`U+FFF8`, `U+1BCA0`-`U+1BCA3`, `U+1D173`-`U+1D17A` and the
  rest of `U+E0000`-`U+E0FFF`; `let ok = is_admin\u{180B};` (rustc accepts it as a distinct
  identifier) showed no hazard and printed exactly as `is_admin`. With `is_bidi` it now covers
  the whole `Default_Ignorable_Code_Point` set, so hazards, `display_safe` and untrusted-text
  sanitising all catch them.
- **`--layout inline` and `side-by-side` note a change their rows print as unchanged** (bug
  hunt 2026-11-01). Both layouts printed only each line's text, so a change to the end-of-file
  newline alone, a change between two non-UTF-8 bytes (both shown as `U+FFFD`), a trailing space,
  a no-break space against a space, a tab against the spaces it advances to (a broken Makefile)
  and, in side-by-side, a change past the column width (the cut hid it) looked unchanged. A `\`
  note row now follows a lossy line (`\ non-UTF-8 line, shown lossily`), a line without a final
  newline (`\ No newline at end of file`) and an added line whose printed text looks the same as
  its removed pair's (`\ differs from its pair only in bytes shown alike`, which also catches a
  literal `\x1b` against a raw `ESC`). Each layout judges the pair by what it prints: side-by-side
  by its cut, padded cells with a tab as four spaces, inline by raw tabs at 8-column terminal
  tab stops from the text's column. Letters that only look alike (Cyrillic `а` against Latin `a`)
  are not noted.
- **`InlineRow` and `SideCell` are `#[non_exhaustive]`** (review 2026-11-01). The entry above
  adds their `lossy` and `no_newline` fields, which already breaks a struct literal outside
  `deep-diff-forge-projection` (unpublished, and no workspace crate builds one); without the
  attribute every later flag would break it again. Read the fields and build rows with
  `inline_rows` and `side_rows`; a `compile_fail` doctest pins it.
- **The layouts' columns stay aligned past line 9999** (bug hunt 2026-11-01). The line-number
  fields were a fixed `{:>4}`, so rows at line 10000 and later pushed the side-by-side ` | `
  gutter and inline's marker column one place right. The fields are now as wide as the file's
  largest line number (at least 4).
- **A view cursor outside its selection is refused** (bug hunt 2026-11-01). `resume` checked a
  cursor's patch and selection but not its positions, so a forged or corrupted cursor past a
  hunk's end answered that hunk as `complete: true` with no lines, `remaining_hunks: 0` and no
  `next_cursor` (exit 0), one past the selection answered an empty page with nothing remaining,
  and `+0.05` was read as `0.5`. A position must now be canonical decimal and inside the selection
  (a hunk's line or a file's hunk, or `0`); anything else is `ViewError::Cursor` (CLI exit 2).
- **`PatchHunk::address` no longer claims to name exactly one hunk** (bug hunt 2026-11-01). Two
  byte-identical hunks at the same ranges in repeated sections of one path share an address, by
  design (views select every copy, findings resolve to the first); the rustdoc and README said
  otherwise. Documentation only: the hash is unchanged, so addresses stay stable.
- **A file whose section started past row 65,535 of the review could not be shown.** The diff
  pane scrolled through ratatui's `u16` paragraph offset, so selecting such a file drew another
  file's rows under its title and tree marker, no key could scroll past the cap, and `v` marked
  the unseen file reviewed (reproduced with a 70,000-line file before a one-line one). The pane,
  the file tree and the command panel now draw only their visible rows, unscrolled, from `usize`
  offsets; `ReviewApp::scroll` and `ReviewApp::panel_scroll` return `usize` (a breaking change
  for a caller that stores them as `u16`).
- **A key took 7.2 s on a 900,000-line patch.** Every frame built and syntax-painted a row for
  every line of every file, then showed about 20 of them, so each key, resize or mouse burst cost
  time and memory in proportion to the review. The diff pane now counts rows in one cheap walk
  and builds only the visible window, and the tree builds only its visible rows. Measured on the
  same patch: 12 to 24 ms per key and 197 MB, from 7.2 s and 1.3 GB. Wrapped rows of ASCII text
  are counted without being segmented; the first frame after `w` takes 0.18 s. Each frame still
  walks the whole review once to count its rows.
- **The default inline view dropped tab characters.** `display_safe` keeps a tab, and ratatui
  discards it as a control character, so tab indentation, and a dedent such as `return` moving
  out of an `if` in Python, looked unchanged; side-by-side and wrapped rows were right. Tabs are
  now expanded to four spaces before any untrusted text becomes a span (`safe_span`,
  `themed_spans`) and before the row's tint is measured. `projection` gains `expand_tabs` and
  `clip_cells`.
- **A mouse click on the file tree could select the wrong file.** The click mapper assumed a
  40-column, unscrolled tree: after the tree scrolled, a click selected the file that row would
  hold at the top (`f02.txt` for a click on `f23.txt`), on an 80-column terminal a click inside
  the diff changed the selection, and above 125 columns a click on the right of the tree focused
  the diff instead of selecting a file. The event loop now builds a `HitMap` of each frame it
  draws (the pane rectangles inside their borders and the tree's scroll; `HitMap::of`), and
  `map_mouse` resolves a click against it, so a click selects the row it lands on and a click on
  any border, the diff pane's included, or on the status bar does nothing (a breaking change to
  `map_mouse`'s signature; `SelectTreeRow` now carries a row of the whole tree). The map is kept
  by the loop, not the app: `render` only reads the `ReviewApp`, which stays `Send` and `Sync`.
- **Invisible format characters passed `display_safe` unescaped.** The invisible-character list
  missed assigned default-ignorable code points such as U+17B4 KHMER VOWEL INHERENT AQ, which
  Rust accepts inside an identifier: `let is_admin\u{17b4} = …` rendered like the removed
  `let is_admin = …`, with no hazard note. The Khmer inherent vowels, the Mongolian free
  variation selectors, the deprecated format characters U+206A to U+206F, the interlinear
  annotation controls, the shorthand format controls and the musical format controls are now
  escaped, reported as the `invisible` hazard, and dropped from untrusted annotation text.
- **A long or wide file name pushed a note's grounding label off the pane.** The note box
  measured its title and body in characters, not terminal cells, and never shortened the file
  name, so a CJK or 70-character name clipped `partially_grounded` and the box's corner. The box
  now measures in cells, clips the name with `…` so the label and corner fit, and wraps and pads
  its body by cell width.
- **Enter did nothing in the review view, though the README said it opens the file.** Enter now
  opens the selected file's diff: focus moves to the diff pane. It still runs the selected
  command in the palette.

### Tests

- **The gate sandbox's surviving mutants are closed** (`review/src/sandbox.rs`): tests pin `--cpus=<host CPUs − 1, min 1>` in argv and receipt, refuse a blank runtime or an empty image by name, and bound control-command output at 64 KiB; one `>=` deadline mutant is argued equivalent (EVIDENCE.md).
- **`mutation_scope`'s quadratic-scan guard fails instead of hanging.** Its 1 MiB `#` run is
  scanned under a 30 s deadline, so the test binary exits and eager `#` counting, which
  `code_lines_skip_comments_and_attributes_across_lines` already fails, is reported as caught,
  not as a timeout (EVIDENCE.md, "Review crate: mutation_scope mutation").
- **Council admission boundaries are pinned:** nine `review` contract tests kill the 16 `policy.rs`/`model.rs` mutants that survived the whole-crate run (EVIDENCE.md).
- **The Python `StdioClient` timeout covers the request write.** `_exchange` wrote the request
  with a blocking `write` and `flush`, and only the read had a deadline: a request larger than
  the pipe buffer (64 KiB; any real patch) sent to a child that had stopped reading blocked
  forever, and the child was never killed (measured: `timeout=1.0`, a 1 MiB patch, `sleep 30` as
  the child, killed by an outer 8 s `timeout`). Both pipes are now non-blocking and one selector
  writes the request in chunks and reads the answer under one deadline; on expiry the child is
  killed and `TransportError` raised (measured: after 1.0 s). A child that answers and closes
  before taking the whole request is read, not reported as "not reading".
- **Learning store: an append never glues a receipt onto an unterminated last line.**
  `append_receipt` wrote at the end without checking the last byte. After a torn write, or a hand
  edit that dropped the final newline (which `load_receipts` accepts), the next `learn record`
  exited 0 but merged its receipt into that line: every later `learn status` failed with exit 3
  (`trailing characters`), and deleting the bad line also lost the new receipt. The append now
  writes a newline first when the store does not end in one, so the new receipt is its own line
  and a torn fragment stays a separate, visible corrupt line.
- **The Python client maps every invalid answer to `TransportError` and kills the child.** An
  answer that was not UTF-8 or not JSON escaped `_Client.call` as a raw `UnicodeDecodeError` or
  `JSONDecodeError` (both transports) and left the stdio child running; an answer for another
  request id did not kill it either. They now raise `TransportError("not a valid response: …")`
  as the Rust SDK does, and `StdioClient` kills the child. A finding holding a lone surrogate,
  which `json.loads` accepts but UTF-8 cannot carry, raised a bare `UnicodeEncodeError` for the
  whole `verify_findings` batch; it now raises `ValueError` naming the finding's index before
  anything is sent (the CLI, reading JSONL bytes, refuses such a line as `malformed`; one request
  cannot carry it at all).
- **A stored receipt may hold only receipt fields (anchor A-05).** `load_receipts` parsed each
  line with serde's default, which drops unknown keys, so a hand-edited line carrying a `path` or
  `source` key was counted and scored, against the documented promise that a line holding a path
  is refused, naming its line. A stored line with a key that is not a receipt field is now
  refused (`receipt store line N: invalid receipt: a stored receipt holds a key that is not a
  receipt field`, exit 3); the key is not echoed, since it may be the path. `learn record --stdin`
  still drops extra keys, as the privacy probe requires: they are never written.
- **A relative `XDG_STATE_HOME` or `XDG_RUNTIME_DIR` is ignored.** `resolve_learning_dir`
  accepted any non-empty value, so `XDG_STATE_HOME=relstate` put the learning store under the
  working directory, often the repository under review. The XDG Base Directory spec makes a
  relative value invalid: the resolver now falls back to `$HOME/.local/state`, and a relative
  `$HOME` gives `NoStateDir`. The daemon's `runtime_base_from` had the same gap for its socket;
  a relative `XDG_RUNTIME_DIR` now fails closed like an unset one (`daemon path` exits 2).
- **`doctor` names the state directory the learning store uses.** It formatted `$XDG_STATE_HOME`
  as is, so after the resolver began ignoring a relative value, `XDG_STATE_HOME=relstate
  deep-diff-forge doctor` printed `state_dir=relstate/deep-diff-forge`, a place nothing writes
  to, while `learn record` wrote under `$HOME/.local/state`. `state_dir` is now the parent of the
  store's own resolved directory (the daemon socket and runtime dir already used the daemon's
  resolvers), and with no absolute `$XDG_STATE_HOME` or `$HOME` it prints `<unset: …>`.
- **A deeply nested answer is a `TransportError` too, and the stdio child is killed.** An answer
  of about 200 KB of `[` made `json.loads` raise `RecursionError`, which `_Client.call` did not
  map, so it escaped raw on both transports and left the `StdioClient` child running (measured).
  It is now `TransportError("not a valid response: …")` like any other invalid answer.
- **`StdioClient` returns an early answer from a child that stops reading but stays alive.** The
  exchange kept waiting to write the rest of the request until the deadline, though the whole
  answer was already read: a child that refused a 1 MiB request and then slept gave
  `TransportError: no answer within 2 s` instead of its refusal (measured). A complete answer line
  now ends the exchange. Its unsent tail would garble the next request, so the out-of-step child
  is killed and the next call raises `TransportError`.
- **A path and a line resolve in every section with the path.** `findings verify` searched only
  the first file section with a finding's path, so in a concatenated patch that names one file
  twice, a line in the later section was refused as `line_outside_hunk` although its address
  form was accepted. `resolve`, the cockpit's `annotations` (which fell back to a file note) and
  `validate_anchor` (which refused a later section's hunk or span) now search every section with
  the path, in review order; a path without a line still anchors to the first.

## [0.5.0] - 2026-10-31

### Added

- **`deep-diff-forge bench [--quick] [--workload NAME]... [--json]`** times seven workloads on
  generated inputs and prints `deep-diff-forge.bench.v0`. That receipt carries the profile, the
  host, each input's SHA-256, and the min, median, p95 and KiB/s.
  - `scripts/bench_compare.py` compares receipts from one machine (best median per side; over
    25% and over 0.2 ms slower fails). A workload whose input changed is reported as not
    compared, and a malformed receipt exits 2.
  - A `bench` job on every pull request runs the merge base against the head (a required check from
    0.5.0, see Changed), and the weekly campaign keeps a full receipt.
- **`deep-diff-forge eval [--corpus DIR] [--json]`** scores ranking pairs, evidence precision
  and recall, and findings verdicts against `fixtures/eval/MANIFEST.json`, a hashed and labelled
  corpus of 11 cases with documented known misses. It prints `deep-diff-forge.eval.v0`.
  `cli/tests/eval.rs` holds the floors.

- **Daemon metrics.** `deep-diff-forge daemon metrics` (the daemon's `daemon.metrics`, protocol
  2) returns `deep-diff-forge.daemon-metrics.v0`:
  - per method that was called: requests, errors by RPC code, and latency as log₂ microsecond
    buckets with p50, p95 and p99 upper bounds;
  - admission: admitted, waited, wait time, busy refusals, and peak in-flight bytes and requests;
  - connections: accepted, refused at the limit, refused as another user, and errors;
  - engine: contained panics, session-table resets, and sessions opened, closed and evicted.

  Recording takes no lock (relaxed atomics). `deploy doctor` warns, with the counts, when a live
  daemon reports contained panics, busy refusals or session resets.
- **`daemon start --log-format json`:** one JSON object per log line (`ts_ms`, `level`, `event`,
  `message`). The default text line is unchanged.

- **Verified agent findings.** `deep-diff-forge findings verify --findings FILE [--require-all]`
  checks agent findings (JSONL, `deep-diff-forge.finding.v0`) against the patch on stdin.
  - It checks the hunk address or path and line, freshness (`review` against `input_sha256`),
    size, and duplicates. It prints the sealed `deep-diff-forge.findings.v0` verdict, with
    `claim_verified: false` always.
  - Accepted findings are normalised from the review (the line's own kind and text) and sanitised.
  - Refused ones carry one reason: `malformed`, `stale_review`, `unknown_path`,
    `unknown_address`, `line_outside_hunk`, `duplicate` or `too_long`.
  - The same verdict comes from the daemon's `findings.verify`, `rpc`, the MCP tool
    `verify_findings` and the Python SDK's `verify_findings`.
  - `review --findings FILE` shows the accepted findings in the cockpit as agent notes, never
    grounded, and a finding cannot pass for the engine's own notes.

- An address-anchored finding is resolved through a table of hunk addresses built once per
  call. Before (in this change's own first version), each finding hashed every hunk again: 2,000
  findings against 400 hunks took 3,560 ms. Now 33 ms (measured with `deep-diff-forge bench`,
  release build).

- A `findings_verify` fuzz target: every non-blank line gets exactly one verdict, every accepted
  anchor resolves, and no sanitised text keeps an unsafe character.

- **Code evidence in risk ranking.** Nine lexical signals are read from the changed lines
  themselves: `unsafe_added`, `process_spawn`, `network_fetch`, `dynamic_eval`, `ci_privilege`,
  `build_hook`, `secret_like`, `check_suppressed` and `test_removed`. They cover Rust, Python,
  JS/TS, Go, shell (with `Dockerfile` and `Makefile`) and YAML, and a shebang names the language
  of a file with no extension.
  - Comments and strings are blanked, with block comments and triple-quoted strings followed across
    a hunk's lines.
  - Each signal adds its weight once per file, and a path cannot hide it: token evidence keeps its
    weight under `generated/` or `vendor/`.
  - `rank.v0` lists each file's `evidence` (signal, hunk address, side, line, `basis: lexical`; at
    most 32). Outline and hunk heads list their labels. The daemon, `rpc`, MCP and the cockpit
    (one note per item, on its hunk) show the same.
  - Before, a hunk adding `unsafe { std::process::Command::new("curl") }` to `src/lib.rs` scored
    the same 7 as any `lib.rs` edit. Now it scores 15, with both signals at line 2.

- An `evidence_scan` fuzz target checks, for any patch, that every evidence item names its own
  hunk and a line inside it, and that the outline labels and the rank agree.

- The plan behind this and the next changes: `docs/PLAN_AGENT_EVIDENCE_AND_OBSERVABILITY.md`.

### Changed

- The pull-request `bench` comparison (`bench (base vs head)`, renamed from
  `bench (base vs head, informational)`) is a required check: ruleset 25868121 now requires
  eleven. A head over 25% and over 0.2 ms slower than its merge base on any workload blocks the
  merge.

- **Daemon protocol 2.** `findings.verify` is `since: 2`; clients that ask for protocol 0 or 1
  get the method lists they had.

- `deep-diff-forge-daemon` and `deep-diff-forge-cli` depend on `deep-diff-forge-agent`
  (`docs/crate-edges.txt`).

- CLI daemon tests wait up to 10 s, not 2 s, for a daemon to start or exit, so a loaded machine
  is not read as a failure. Two local gate runs had failed once each and did not reproduce in
  15 later full runs; a 2 s wait under a parallel build is the likely cause (inferred, not
  reproduced).

- **Breaking (graph crate):** `RankedFile` is `#[non_exhaustive]` and gains `evidence` and
  `evidence_truncated`. Build it with `RankedFile::scored(file)`, the one constructor, so score,
  signals and evidence always come from one assessment (`assess_file`). `RiskSignal` gains
  `Code(EvidenceSignal)`. `sort_ranked` is the one ranking order, and the cluster crate uses it.

### Fixed

- Code evidence is linear in the number of matching lines. Each new item was checked against
  every item already found, so one file of 160k lines that each add `unsafe` took 4.8 s (and
  the 64 MiB patch budget allowed far more); it takes 0.15 s. A changed line now yields each
  signal once by construction, however many rules or tokens match it.

### Security

- **Quoting code no longer hides it from code evidence** (DDF-EVID-001). The lexical view
  blanked everything between quotes, so `V="$(curl …)"` in shell,
  `` `${require("child_process").execSync(c)}` `` in JS and `f"{eval(x)}"` in Python carried no
  evidence. Code inside `$(…)`, `${…}` and backticks (shell double quotes), `${…}` (JS template
  literals) and `{…}` (one-line Python f- and t-strings, not `{{`; `\{` still runs) is now
  scanned as code up to its matching closer on the line. Inside it, strings are strings again, a
  backslash escapes, JS comments are comments, a shell `case` pattern's `)` is not a closer and
  a Python format spec is text, so neither a quoted closer hides the code after it nor a quoted
  token (`$(printf 'curl failed')`) is read as code. Single-quoted shell and plain or bytes
  Python strings still run nothing and stay blank.
- **The scanner errs toward code.** Three review rounds (226 agents) found ways a misread quote
  hid the rest of a line, most of them older than this change. Now:
  - a string still open at the end of a line (outside Rust) is read as code from its quote;
  - shell `#` is a comment only at a word start (`$#`, `${x#y}`, `a#b` are code);
  - a backslash escapes at the top of shell and JS lines;
  - ANSI-C `$'…'` strings honour escapes;
  - a JS regex literal's quotes open nothing, and a JS quote right after a word (JSX text,
    `Can't`) opens nothing;
  - nested Python triple quotes are known;
  - shell `case` is known after any command start (`if`, `while`, `!`, `time`, a pattern's
    `)`), `esac` only as a whole word that is not a pattern alternative;
  - a JS regex starts after an operator other than `<` and `>` (which open JSX tags), after `=>`
    and after a `return`/`typeof`-like keyword, not after `a++`, a property or a TypeScript
    non-null assertion (`x! / y`); a line that starts with `/` reads the previous line's last
    code byte; one failed regex probe ends probing, so a hostile line stays linear;
  - Rust raw strings (`r"…"`, `br#"…"#`) do not escape, so `r"C:\"` no longer hides the rest
    of its line.

  Not covered: triple-quoted f-strings, interpolations that span lines, continuation lines of
  multi-line strings (read as code), non-ASCII spellings of a token, JSX text, and a regex right
  after `if (…)`. The scanner is a reviewer's lens, not an adversarial boundary: no evidence
  means no token was found, never that a change is safe.
- **A JS `child_process` import is `process_spawn` evidence.** Before, the module name was only
  ever seen inside a string, which the code view blanks, so the rule could not fire. Imports are
  read through the code view (`require`, `module.require`, `import(…)`, `import …`, `from …`;
  any spacing or parentheses, any quote kind, `node:` optional). Comments, strings, regexes, `import type`/`export type` statements and
  `typeof import(…)` do not count; an `import(…)` type annotation does.
- `sanitize_body` (agent annotations, and now findings) drops bidi, invisible and
  line-separator characters as well as control characters. Before, Unicode category Cf
  characters such as U+202E were kept, so an agent's note could reorder or hide text for the
  next reader. `deep_diff_forge_core::is_unsafe_in_untrusted_text` is the one predicate.

## [0.4.0] - 2026-10-30

### Added

- **`deploy doctor [--json] [--socket PATH]`** checks that this machine can run the daemon and
  Git inputs, and changes nothing: the socket directory, the socket (absent, live, stale, of
  another user, or not a socket), the start lock, and `git version` (run the way an acquisition
  runs Git, within 10 s). The daemon checks apply the daemon's own start rules and never take the
  start lock. Exit 6 when a check fails; `--json` prints `deep-diff-forge.deploy-doctor.v0`.
- `deep_diff_forge_daemon::inspect` (with `Inspection`, `SocketState`, `LockState`, `DirState`)
  and `SocketLocation::start_lock_path`: a read-only view of a daemon location.
- `deep_diff_forge_patch::git_version`: Git's version line, bounded like an acquisition.

### Security

- **The daemon serves its own user only** (closes a 0.3.0 residual risk). Each accepted
  connection's effective uid is read from the kernel (`SO_PEERCRED` on Linux, `getpeereid` on
  macOS and the BSDs; other platforms refuse every client), and a client of another user is
  dropped and logged, whatever the socket's mode. The CLI, the Rust SDK (`SocketLocation::connect`)
  and the Python SDK (`SocketClient`) check the daemon's uid the same way and refuse a daemon of
  another user before sending anything; the CLI says `cannot use the daemon at PATH` (exit 6)
  rather than `no daemon at`.

### Fixed

- **The daemon runs requests in parallel** (closes a 0.3.0 residual risk). Before, every
  connection shared one `Mutex<Engine>`, so one slow request delayed every other client. Only
  the session table is locked now, and only for bookkeeping; parsing, ranking and Git run
  outside it. Admission caps the request bytes in flight at 128 MiB, first come, first served
  (`review.change` weighs the 64 MiB patch budget; a request heavier than the cap runs alone).
  Measured: six clients ranking a 2.75 MB patch, 23.4 to 56.8 requests per second;
  `daemon.health` under that load, 205 ms to 0.2 ms. A panic while the session table is held
  empties it; any other panic leaves every session in place. A request whose turn does not
  come within 30 s is answered `RESOURCE_EXHAUSTED` ("the daemon is busy") and gives up its
  place in the queue.
- CLI daemon tests kill their daemon when an assertion fails first (`Child` is not killed on
  drop); a mutation run had left one serving for 31 hours.

- **Two `daemon start`s can no longer race** (closes a 0.3.0 residual risk). Before, between the
  stale-socket probe and the bind, a second start could remove the first's fresh socket and bind
  its own, leaving the first running but unreachable. A start now takes a non-blocking exclusive
  `flock` on `<socket>.lock` before the probe and holds it until the daemon exits; a start that
  finds it held exits 6. The lock is opened without following symlinks and must be a regular file
  owned by the effective user.

### Changed

- **Breaking (daemon crate):** `dispatch`, `process_line` and `handle_connection` take
  `&Engine` instead of `&mut Engine` or `&Mutex<Engine>`; `Engine` is `Send + Sync` (checked at
  compile time). `MethodSpec` gains `work: Work`. New: `Admission`, `Permit`,
  `MAX_IN_FLIGHT_BYTES`, `ADMISSION_WAIT`, `Busy`, `Work`, `request_weight`,
  `Engine::admission`.

- **macOS tests run on every pull request** and `macOS tests` is a required check. Before, the
  job ran only weekly, on manual runs and on release, and two macOS breaks in PR #44 were caught
  only by a manual run.
- **A weekly campaign** (`.github/workflows/campaign.yml`, Mondays 05:41 UTC and on demand):
  a 10-minute hostile daemon soak with real review work and leak checks, and an hour of fuzzing
  for every fuzz target, each target its own job; a crash keeps its input as an artifact.
- The daemon soak runs real review work from four concurrent clients in every iteration and
  checks RSS growth (over 32 MiB fails) and open-fd growth (over 4 fails) across a run of a
  minute or more; `--require-growth-check` fails a run too short to measure them.
  `scripts/security/test_daemon_soak.py` covers the checks, in CI and `just ci`.
- `scripts/fuzz_target.sh` is the one place that knows each fuzz target's seeds and input cap;
  CI's 10 s smoke and the campaign both use it, and it lists the targets from `fuzz/Cargo.toml`.
- `SocketLocation::bind`, `bind_secure` and `bind_explicit` return a `BoundListener` (the
  listener and its start lock; `listener()` borrows the socket) instead of a `UnixListener`.
- `deep-diff-forge-daemon` uses rustix's `fs` and `net` features, and `nix` (`socket`, `user`)
  outside Linux for `getpeereid`; that adds `memoffset` to the lock file for those targets only.

## [0.3.0] - 2026-10-29

Tagged `v0.3.0` on 2026-10-29 by owner decision. 0.3.0 is minor rather than patch because the
library APIs changed incompatibly since 0.2.1 (see *Changed*). 0.2.1 was never tagged, so this
release follows v0.2.0. The workspace reached 0.3.0 on 2026-10-28; the expert-circle, bug-hunt
and owner-decision entries below landed before the tag. crates.io publication stays on hold.

### Changed (owner decisions on the bug hunt, 2026-10-29)

- **A rename or copy that Git refuses is refused** (H1-09). `rename to ../../evil`, an absolute
  path, or a `.`, `..` or `.git` component in a rename or copy path now fails with exit 4
  (`InvalidPath`), as `git apply` fails with "invalid path". DDF used to show such a patch with a
  `path_traversal` hazard. A plain `---`/`+++` path keeps its hazard, since a non-Git diff may
  legitimately name `../`.
- **Copies are `copied`** (H1-09). A `copy from`/`copy to` file reported `modified`.
- **A mode-only change is `mode_changed`** (H4-07). Flipping the executable bit reported
  `type_changed`, which now means only that the object type changed (a file, a symlink or a
  submodule became another). A mode header that is not one to six octal digits is refused
  (`InvalidMode`), as `git apply` refuses it.
- **Symlinks and new executables are risk signals** (H4-07). `symlink_change` (+3) fires when a
  symlink is created, changed, removed or made from a file; `executable_bit` (+2) fires when a
  file becomes executable. A generated or vendored path keeps both.
- The TUI tree marks copies `C` and mode-only changes `X`.
- **The hostile daemon soak sends oversize and flood traffic** (H5-09). It checks a request of
  exactly 80 MiB is served and one byte more is refused with the connection closed; that with
  all 8 connection slots held a ninth is closed unanswered, and the daemon serves again once
  they leave; and that 32 concurrent clients get only valid answers or closed connections.

### Fixed (expert circle, 2026-10-28)

A seven-seat expert circle reviewed DDF as pioneer coding agents would use it: Claude Code, Codex,
Zen (pi) and four specialist personas. It found these defects, each reproduced with the 0.3.0
binary and each fixed with a test that fails on the old code.

- **A rewritten comment is never "formatting only".** In tree-sitter-rust a plain comment's text
  is no leaf token, so `structural` and `analyze` reported `// SAFETY: the pointer is valid` →
  `// SAFETY: trust me` as `reformat_only: true` with no change. A comment is now one token,
  compared byte for byte. A new fuzz target, `structural_soundness`, checks that
  `reformat_only` is claimed only when the sources differ in whitespace alone (1,077,956 runs,
  0 crashes, measured locally).
- **A path can lower a file's risk but never zero it.** `generated/`, `vendor/`, `dist/` and
  similar paths used to score 0, so a new `src/generated/x.rs` that spawns a process ranked below
  a one-line README edit. The patch author chooses the path. Such files now keep the base weight
  and their new, deleted or binary signal, without the source, size or API bonuses.
- **`highlight` prints line breaks.** It escaped each line feed as the text `\x0a`, so a file
  printed on one line. Every other control character is still escaped.
- **Parse errors quote at most 120 bytes of the offending line** and count the rest. A 2 MB
  line made a 2,000,074-byte error, in a terminal or an agent's context.
- **Every command answers `--help`** (exit 0) from one table that `--help` also prints. Before,
  only `review` did; the others exited 2, and `council --help` exited 4.

### Security (bug hunt, 2026-10-28)

- **Patch truth: lines Git ignores are refused, not read as headers** (H1-01, high). After a
  file's hunk, `new file mode`, `deleted file mode`, `rename to`, mode lines, a lone `+++` or a
  lone `--- /dev/null` changed the reviewed status or path, or made a phantom file, while `git
  apply` applied a plain modification. Mode lines even changed the rendered patch (644 became
  755). Extended headers are now admitted only before a file's `---`/`+++` or first hunk, a `+++`
  only right after its `---`, and a `---` only with its `+++`.
- **Input Git refuses as corrupt is refused** (H1-04): a signed hunk range (`@@ -+1`), a final
  hunk line without a line feed, `new file`/`deleted file` contradicting `/dev/null`, a new and
  deleted file, a new or deleted file that is also renamed or copied, or a new file with old lines.
  New errors: `MisplacedHeader`, `InconsistentHeader`, `UnterminatedLine` (all exit 4).
  Fuzzing found six such contradictions in the first rounds; each is a regression fixture, and
  the final rounds ran 5,778,549 (`patch_parser`) and 2,261,936 (`review_json`) inputs with no
  crash.

A team of five Sonnet bug hunters covered the whole stack: patch and core, daemon/rpc/MCP/SDKs,
CLI and council, analysis and TUI, CI/scripts/supply chain. Each finding was re-checked by an
independent false-positive checker before a fix.
- **Repository filters never run.** A clean or process filter defined in a repository's own
  configuration and named in its `.gitattributes` ran during working-tree acquisition
  (`--git-working-tree`, `review.change`), so reviewing a repository someone handed you ran their
  code. Every filter the local configuration defines is turned off for each Git call; the user's
  global filters, such as Git LFS, stay on.
- **`review.change` refuses foreign Git directories.** A `.git` file inside an allowed root could
  point at a repository outside it. The top level, the Git directory and the common directory must
  all be under an allowed root, and an object store with alternates is refused.
- **The Git time limit holds** even when a child Git started keeps its pipes open.
- **The daemon refuses params a method does not take**, so a misspelt guard such as
  `require_file` no longer passes an empty review. MCP enforces its schema enums.
- `highlight` keeps line breaks without colour too (the earlier fix covered only colour).
- **An edit inside a multi-line comment is analyzed.** `analyze` attributed a changed token to a
  hunk by its first line only, so with `diff -U0` a rewritten `SAFETY:` line inside a block
  comment produced no semantic span while coverage still said `analyzed`. A token now belongs to
  the first hunk holding any of its lines, and a changed token no hunk holds makes coverage
  `partial_lcs`.
- The daemon soak no longer passes its memory bound with no RSS sample (no `/proc`), checks that
  a stopped daemon removed its socket, and logs the daemon to a file instead of a pipe nobody
  drained.
- **A rename out of `.git` is hazardous too.** Renaming `.git/hooks/pre-commit` to `src/y.rs`
  scored no signal while the reverse rename raised `hazardous_text`; the path a rename or copy
  leaves now counts.
- **Headless probes show what a terminal shows.** `review --probe` printed a space after each
  wide character (`界 界`), so its lines measured wider than the terminal they describe; the
  cell a wide character covers is skipped now.
- **The command panel wraps.** At 80 columns the notes panel cut the hazard warning after "what
  you see", losing "may not be what applies."; each panel line wraps at the panel width.
- A semantic note counts each change kind (`3 node added, 1 node removed`) instead of naming
  every span after the first one's kind.
- **A submodule change is captured whatever configuration says.** `diff.ignoreSubmodules = all`,
  from any configuration level, or a `.gitmodules` `ignore = all` dropped a staged submodule
  change from `--git-*` and `review.change` with exit 0, so a gate saw no files. Every
  acquisition now passes `--ignore-submodules=none`.
- **`review.change` names what it did not capture.** An untracked nested repository is not in
  the files; only the CLI's stderr said so. The `acquisition` member now lists it under
  `omitted_untracked`.
- A `GitAcquireOptions::byte_budget` of `usize::MAX` no longer wraps to a zero-byte read.
- **One output mode at a time.** `--outline --hunks` printed the outline, `--rank --cluster` one of
  the two, and `--outline --path`/`--address` dropped the selector, all with exit 0. Each is now
  a usage error (exit 2) before any input is read.
- **`git format-patch` output is read.** Its bare `---` before the diffstat and its `-- `
  signature were refused as body lines outside a hunk, so `git format-patch -1 --stdout |
  deep-diff-forge --stdin-patch` exited 4. Git ignores both outside a hunk; so does DDF now,
  for those exact lines only.
- **View pages hold their budget on every surface.** Each was reproduced before the fix:
  - A 200 KB `@@` heading made a 1 KiB page 200,616 bytes and repeated on every continuation
    page. A view now carries at most 256 bytes of a heading, flagged `section_truncated`
    (`review.v0` keeps it whole).
  - MCP view answers ran to 1.39× `max_bytes`, because the escaped text copy grows past half.
    They are now re-paged smaller (`views::fit_emitted`) until the whole result fits.
  - The CLI printed views indented but budgeted them compact (4,688 bytes for
    `--max-bytes 4096`). It now prints the daemon's one-line page.
- **A path or address selects every match.** `paths` returned only the first section of a path
  that a concatenated patch names twice, and `addresses` the first of two identical hunks.
- **`highlight` is bounded by nesting depth.** Its query cost grows with depth, not size: a
  60,000-deep brace nest took 58 s (debug) and 20 s (release) to highlight, while the same node
  count laid out flat took 0.14 s. Past 4,096 levels or the 200,000-node budget `semantic` uses,
  `highlight` prints plain text; the deep nest now returns in 0.16 s.
- **A comment's line ending is formatting.** A line comment's token held its line feed, so CRLF
  conversion or a trailing space made `structural` and `analyze` report a rewritten comment. Each
  comment line's trailing whitespace is dropped from its token; a changed word still counts.
- **The cockpit refuses a patch that names a path twice.** `review` matched content and notes
  by path, so two concatenated patches touching one file showed the first section's content and
  notes for both, and the second section was never shown. It now exits 4 and names the path;
  `deep_diff_forge_tui::duplicate_path` finds it for library callers.
- **Hazards cover the invisible-payload classes.** Word joiners and invisible operators
  (`U+2060–2064`), the soft hyphen, the combining grapheme joiner, the Mongolian vowel separator,
  the Hangul fillers, variation selectors (`U+FE00–FE0F`, `U+E0100–E01EF`) and tag characters
  (`U+E0000–E007F`) were neither flagged nor escaped. They are now `invisible` and shown as
  `\u{XXXX}` on a terminal. The line and paragraph separators `U+2028`/`U+2029` are `control`.
  A path that ends in a carriage return (a CRLF-converted header) is `control` too; only a body
  line's CRLF is exempt.
- **No JSON record splits on a Unicode line break.** A path or line holding `U+2028`/`U+2029`
  left `--json`, `--jsonl`, `rpc`, MCP and the daemon socket raw, so a line-aware reader
  (Python's `splitlines`, JavaScript before ES2019) saw one record as two. `json_escape` now
  escapes both, and the serde-built lines pass through the new `core::json_line_safe` before
  they are written; the parsed values are unchanged.

### Added (agent batteries)

- **`agent setup --client claude|codex [--allow-repo PATH]...`** prints the registration of this
  binary's MCP server: a `claude mcp add --scope user …` line and a `.mcp.json` block, or a
  `~/.codex/config.toml` table. It never writes a file. `--json` gives
  `deep-diff-forge.agent-setup.v0`.
- **`agent doctor [--json]`** checks in-process that the MCP handshake and the seven tools'
  output schemas are in place, and that one patch gets the same sealed document from the CLI
  path, the daemon and MCP, the same ranking from `diff.rank` and `rank_patch`, and a bounded
  outline-then-hunk answer. It exits 101 if any check fails. A test runs it on every build.
- **Hazards** (expert circle, Sana Rahimi, narrowed by Zen to objective facts). The facts are:
  `bidi_control`, `invisible` and `control` on lines (a CRLF's carriage return excepted), and
  `path_traversal` and `git_internal` on paths.
  - They are reported in `review.v0` (only where found), in the views and in the outline.
  - A new risk signal, `hazardous_text` (+3), lifts such a file, even under a generated path.
  - The cockpit explains the signal.
- **`rpc` and the daemon name the framing** when a request line is cut short (the start of a
  pretty-printed object): one JSON value per line.

### Added (agent-first core)

The expert circle's Tier 1: an agent learns where to look before it reads anything large, and
never has to paste a patch.

- **Hunk addresses in `review.v0`** (additive): `address` (SHA-256 of `ddf-hunk-v1`, path,
  ranges and body: exactly this hunk of this patch), `body_sha256` (the body alone: it survives a
  move), `section` (the `@@` heading, display only, never re-rendered) and `old_lines` /
  `new_lines`.
- **Budgeted, sealed views.**
  - `review.outline` lists files in review-risk order with their hunks' addresses, ranges,
    headings and counts, and no line text.
  - `review.hunks` returns chosen hunks verbatim, by address or by path.
  - Both keep every answer within `max_bytes`, continue a long hunk at a line (never inside one),
    and page with a cursor bound to the patch and the selection.
  - They are available on the socket, over `rpc`, as the MCP tools `review_outline` and
    `review_hunks` (where `max_bytes` counts both copies of the answer), and on the CLI as
    `--outline` and `--hunks [--address A]... [--path P]... [--max-bytes N] [--cursor C]` for
    piped patches and Git selections alike.
  - Parity tests hold the CLI equal to the daemon page for page.
- **`review.change` and MCP `review_change`: review straight from Git.**
  - Selections are staged, working tree or range. The answer is the outline or hunks of the
    change, sealed over Git's exact bytes, with an `acquisition` member.
  - It reads only repositories whose canonical path and Git top level are under a root the
    operator allowed with `--allow-repo PATH`, on `mcp`, `rpc` and `daemon start`. Without one it
    reads nothing.
  - Tests cover an outside repository, an escaping symlink and a subdirectory of a larger
    repository.
- **Git subprocesses have a time limit** (closes a 2026-10-28 residual risk). One deadline bounds
  every Git call of an acquisition, 120 s for the CLI and 30 s for `review.change`. A Git still
  running is killed and the acquisition fails with `TimedOut` (exit 3) instead of hanging on a
  stalled filesystem or hook.

### Added

- **`diff.rank`**, a daemon method on the socket, `rpc` and MCP: the sealed
  `deep-diff-forge.rank.v0` document that `--stdin-patch --rank --json` prints, with the same
  input guards. A parity test holds it equal to the CLI on every fixture. The Python SDK gains
  `rank_patch`.
- **MCP tools declare an `outputSchema`,** and a test checks every tool's results against it.

### Changed

- **MCP `rank_patch` returns the sealed `rank.v0` object** (through `diff.rank`), and its
  summary keeps the seal. It used to return `diff.plan`'s bare array, which the MCP tools
  specification does not allow as `structuredContent`, and which could not say which bytes it
  ranked. `rank_patch` also takes `require_files` and `require_hunks`.

### Added (documentation and community)

- **The README is rewritten from the code.**
  - Three graphics: a terminal session generated from the binary's real output
    (`scripts/docs/render_terminal_svg.py`), an architecture diagram that adapts to light and
    dark themes, and a refreshed banner (14 crates, 1,354 tests).
  - A Mermaid crate graph drawn from `docs/crate-edges.txt`.
  - The risk-ranking weights from `risk.rs`.
  - The nine required checks.
  - The production-readiness verdict and its known limits.
  - A documentation map.
- **An open call for contributors:** `CONTRIBUTING.md` (setup, the quality bar, open problems
  worth owning), `CODE_OF_CONDUCT.md` (Contributor Covenant 2.1), issue forms for bugs and
  features, and a pull request template.
- **Every document under `docs/`, plus `SECURITY.md`, `EVIDENCE.md` and `NOTES.md`,**
  is checked against the code at 0.3.0:
  - target designs say what exists today;
  - ledgers and history carry a dated status block, and no past entry is rewritten;
  - surfaces that are refused today are labelled.

### Fixed (found while checking the docs against the code)

- **`review --cmd` refuses an unknown command name** (exit 2, naming the commands there are).
  Before, a typo fell through to `rank` and exited 0. A `--cols` or `--rows` value that is not a
  number is refused too; before, it silently became the default.
- **`--parallel` refuses anything but `serial`, `auto` or a count from 1** (exit 2). Before, an
  unreadable count ran as `auto`, and `0` was reported as `fixed:0` while one worker ran.
- **`analyze` with the wrong number of paths, or an option, exits 2** (fix the command line).
  Before, a missing path exited 4 (change the input).
- **`deploy release` no longer claims a GitHub release for an untagged version.**
  `github-release` is `blocked` until the owner tags, so `pending` lists it with crates.io. It
  used to report `published` for 0.2.1, which was never tagged.
- **A tag release runs the macOS tests.** `release.yml` calls `ci.yml` with the tag push's
  event, which the macOS job did not accept, so a tagged release would have shipped macOS
  binaries tested only by an earlier dry run.
- **`just release-tag` pushes the tag to `origin`.** It used to push to a remote named
  `github` that no checkout has. Its comment now describes the crates.io hold, not a token.
- `--help` no longer says learned scores set engine defaults; nothing reads them yet.
  `chain-contract` no longer promises a schema version on every JSONL record; each record names
  its event.

### Added

- **Every exit code names what to do next.** The registry gives each code an
  `action` (`proceed`, `fix_invocation`, `check_environment`, `change_input`,
  `review_evidence`, `report_bug`) and `may_be_transient`; `schema exit-codes`,
  `explain --json` and the MCP exit-code tools report both, and the README
  tables carry a checked Next column. A new code cannot be added without one.
- **A release dry run.** `release.yml` runs on `workflow_dispatch`: the quality
  and supply-chain gates and all three builds, with each executable binary
  smoke-tested and every archive's checksum verified, while releasing and
  attesting stay limited to tag refs. Tag releases get the same smoke and
  checksum checks before anything is published.
- **macOS tests.** A `macos` CI job runs the workspace tests on `macos-latest`
  weekly and on manual runs, including a dispatched release dry run.
- **Deployment resource requirements** in `docs/OPERATIONS_AND_DAEMON.md`: the
  measured memory of maximum-size requests and the `/tmp` inode limit.
- **The review JSON marks the line that was decoded lossily** (`"lossy": true`
  on the line), beside the per-file `lossy_lines` count, so a consumer need not
  guess from U+FFFD. Measured with a JSON-only rebuild sweep over 1,127 real
  commits: every other text file rebuilds to its exact blob.

### Added (slice C: SDKs)

- **Rust: the `deep-diff-forge-sdk` crate.** `review` (with `InputGuards`)
  returns a `Review` whose `to_json` is the CLI's review document byte for byte;
  `rank` orders it by risk; `exit` re-exports the registry; `Client` calls a
  running daemon. `Error` maps to the CLI exit codes. The public signatures are
  pinned by a compile-checked test, and a live in-process daemon test proves the
  client and the facade agree.
- **`deep-diff-forge rpc`:** the daemon's JSON-RPC over stdio, with no daemon.
  It uses the socket server's own line handler and request cap, and stops on
  `daemon.shutdown`.
- **Python client** (`sdk/python`, standard library only): `StdioClient` and
  `SocketClient`. Its tests check CLI parity over both transports and run in CI.

### Added (slice D: MCP server)

- `deep-diff-forge mcp`: a read-only, dual-era MCP server over stdio. It
  implements the 2026-07-28 specification (per-request `_meta`,
  `server/discover`, `resultType`, `UnsupportedProtocolVersion` -32022) and keeps
  the `initialize` handshake for 2025-11-25 and earlier clients. Its tools are
  `review_patch` (the daemon's `diff.compute`, the same JSON value as the CLI's
  document, tested over real stdio), `rank_patch`, `explain_exit_code` and
  `exit_code_registry`. All are annotated read-only. Domain refusals are tool
  errors (`isError`); malformed calls are protocol errors. Request lines are
  capped and stdout carries only protocol messages.

### Added (slice B: daemon parity, protocol 1)

- **Daemon `diff.compute`** returns the same `deep-diff-forge.review.v0` document
  as `--stdin-patch --json` for the same bytes, under the same input contracts
  (`require_files`, `require_hunks`). A parity test proves this across fixtures
  and guard combinations. The input-contract predicate moved to
  `deep_diff_forge_patch::InputGuards`, shared by both surfaces.
- **`engine.methods`** lists every method with its params, result and protocol
  version. `engine.initialize` reports the method names and refuses a protocol
  version it does not speak (domain error 2). Domain error 7 is an input-contract
  refusal, matching CLI exit 7.

### Added (slice A: CLI contract v1)

- `deep-diff-forge envelope -- <command…>` wraps any command's result in one
  `ddf.result.v1` JSON object: exit code, registry outcome, stdout (parsed when it
  is one JSON value), stderr, and any truncation. It runs the command as a child
  of the same binary with stdin passed through, leaves the command's own output
  unchanged, and exits with its code.
- `deep-diff-forge schema [result|exit-codes]` prints the envelope's JSON Schema
  (2020-12) or the exit-code registry. `deep-diff-forge explain <code> [--family
  engine|council] [--json]` names one code.
- One exit-code registry, by command family (`engine`, `council`), owned by
  `deep_diff_forge_core::exit` so the daemon and SDK slices can share it. Every
  exit site uses its constants, and a source-scan test forbids numeric exit
  literals. The engine meanings now say what the codes are used for: 3 is any I/O
  or encoding failure, 4 is any invalid input, and 101 is a panic (council sa1:
  the old "patch parse" label misnamed receipt and analysis errors). The README
  documents both tables, and a test fails if they drift from the registry.
- `envelope` keeps the command's exit code when its own reader closes the pipe;
  whitespace-only stdout stays text; the schema lists every emitted field as
  required and enumerates `outcome`.
- `--help` no longer says crates.io is blocked only by a missing token; it is on
  hold until the codebase is production-ready.

### Changed

- **The crates.io hold is a condition, not a missing secret.** The publish job
  runs only when the repository variable `DDF_CRATES_IO` is `open`; before, it
  ran on every tag and skipped only because no token was configured.
- **`rank_patch` is bounded like `review_patch`.** Above 256 KiB it returns a
  `deep-diff-forge.rank-summary.v0` with the highest-ranked files that fit and
  `omitted_files`.
- **Fallback reasons and semantic kinds have written-out wire names**
  (`FallbackReason::wire_name`, `SemanticChangeKind::wire_name`) instead of
  `Debug`; the bytes are unchanged.

- **CI runs on Ubuntu 26** (Luke's decision, a measured trial, 2026-10-27).
  Every CI job and the release workflow's supply-chain and publish jobs run on
  `ubuntu-26.04`. The trial measured rootless Podman 5.7.0 on Ubuntu 26.04.1
  (kernel 7.0.0): the AppArmor user-namespace restriction starts on, as on
  24.04; the existing guarded step lifts it; and all 12 planted-breach probes
  were refused. The Linux release binary is still built on `ubuntu-24.04`, so
  its glibc requirement keeps it running on older distributions.
- **GitHub and GitLab `main` share one history again** (Luke's decision,
  2026-10-27). GitLab `main` carried three commits from 2026-06-23 (review
  cockpit hardening) that GitHub never received; their code had already
  reached GitHub by another path. A merge joins the histories (the code is
  unchanged), so GitLab `main` fast-forwards with no force push. The two
  collaboration logs they added live under `docs/history/2026-06-23-review-cockpit/`.
- **One parse → guard → seal path** (anchor C-05; library API).
  `deep_diff_forge_patch::SealedReview::new` decodes, parses, applies the input
  contracts and seals the same bytes; the CLI's `--stdin-patch`, the daemon's
  `diff.compute` and the SDK's `review` all use it, and Git selections seal
  through `StructuredDiff::sealed_review` from the same capture.
  `to_json_sealed` is no longer public, so a seal can no longer be paired with
  files parsed from other bytes. Every output is byte-identical.
- **The design docs say they are target design** (anchor E-01).
  `SCHEMATICS.md`, `ARCHITECTURE.md` and `MODULE_STRUCTURE_PLAN.md` open with
  a target-design banner, and `SCHEMATICS.md` carries a generated "current
  crates and edges" block rendered from `docs/crate-edges.txt`; a test fails
  when the block is stale (`DDF_BLESS_SCHEMATICS=1` regenerates it).
- **`deploy status` says it ran nothing** (anchor E-06). Its gates are a
  static `not-run` list; the JSON now carries `"gates_run": false` so a caller
  cannot read `not-run` as a verdict, and the human output points at `just ci`.
- **Council fields that take a closed set are enums** (anchor C-02; library
  API). `Request.builder_family` (`BuilderFamily`), a challenge's
  `disposition` (`Disposition`) and `Assessment.state` (`AssessmentState`)
  were strings checked after parsing; an invalid value is now refused when it
  is read. The JSON is unchanged, and a test holds the reviewer-facing
  report schema's enum lists equal to the Rust enums.
- **The council grades only a verified manifest** (anchor C-01; library API).
  `load_manifest` returns a `SealedManifest`, which nothing else can construct,
  and `assess` takes it, so an in-memory `Manifest` that skipped the seal,
  request and artifact-digest checks can no longer be assessed. It reads as
  the `Manifest` it wraps.
- **`StructuralDiff` has one state, not three flags and an `Option`**
  (anchor C-06; library API). `state: StructuralState` is `Unavailable(reason)`,
  `Degraded`, `LineAnchored` or `Minimal { reformat_only }`, so combinations
  such as "reformat-only and unavailable" can no longer be built. The
  `reformat_only()`, `degraded()`, `minimal()` and `unavailable()` accessors
  report what the fields did, and every JSON output is byte-identical.
- **A learning receipt cannot carry a path** (anchor C-07; library API).
  `StrategyReceipt.file_hash` is a `RedactedId`, which only `redacted_id()`
  makes and which deserializing checks (16–64 lowercase hex characters), so a
  path is refused when a receipt is built or read, not only when it is stored.
  On disk the field is the same bare string. A load error now names its store
  line for malformed JSON too.
- **`InputGuards` has no `Default`** (anchor C-04; library API). "No guards"
  is written out as `InputGuards { files: false, hunks: false }`, so
  `..Default::default()` can no longer silently drop a contract a caller asked
  for; the SDK's `Guards` already worked this way. A `compile_fail` doctest
  holds it.
- **The test-count floor is retired** (anchor B-09). The 50-tests-per-crate
  rule was never enforced, and `just test-audit` always exited 0 (and read 0
  without `rg`); the project's own antipattern catalogue lists count literals
  as a false oracle. The Testing Gold Standard and the deployment docs now
  name diff-scoped mutation (0 missed in the changed code) as the coverage
  oracle, and the `test-audit` recipe is removed.
- README exit-code table gains code 7 and the recommended gate command now
  reads `--require-files --require-hunks`. The workspace has 1,192 passing tests
  (1 ignored), measured 2026-10-26 on the stage D tree; the 991 baseline is
  historical.
- The exemption control evolved through councils r10–r18: a skip-attribute
  lexer, then oracle spans (r12), differential MIR compilation (r13), the
  gate's own build (r14), proven test code (r15), five configurations (r16),
  an allow-list (r17) and `#[cfg(test)]` only (r18). Councils r18–r21 then
  hardened the evidence:
  - incremental state is removed before each build, and drift of the
    unmodified tree is inconclusive (r19–r20);
  - path dependencies are observed (r19);
  - a build script's `cfg(test)` is refused from cargo's own report (r19);
  - each unit's dep-info is held to an allow-list and fails closed (r20–r22);
  - the controller requires exactly the gate and release configurations and a
    clean outcome (r19–r20).

  A stop rule bounds further detectors (r21). Each step answered a council
  finding; `docs/REVIEW_COUNCIL.md` keeps the history.
- The skip-attribute lexer (about 580 lines) is retired (r13); the oracle is
  the single control. `code_lines` widening follows nested comments, attribute
  brackets, plain/byte/raw strings and character literals.
- The mutation job carries `CARGO_ENCODED_RUSTFLAGS=--emit=link,mir`, the same
  flag as the oracle's builds, so the two are indistinguishable by flags.
- CI runs every job on `ubuntu-24.04`, where the rootless-Podman step was
  measured; `ubuntu-latest` moves to Ubuntu 26, which will be its own measured
  change. `actions/checkout` (v4 → v7.0.1) and `softprops/action-gh-release`
  (v2 → v3.0.3) move to their Node 24 releases, pinned by SHA.
- Structural diff trims the common token prefix and suffix before the LCS and
  anchors oversize diffs on unchanged lines.
- CI: every toolchain pinned (stable 1.99.1, `nightly-2026-10-25`); the
  evidence job's mutation timeout raised to 300 s; the real-Podman sandbox
  suite runs on GitHub-hosted runners with canary (non-secret) credential
  files and prints job diagnostics on failure; pedantic clippy covers the
  no-default-features build and the fuzz crate; gitleaks 8.30.1 (checksum
  pinned) scans full history. `.gitignore` covers `__pycache__/`.

### Fixed

- **Patch parsing is linear in metadata lines.** Every non-body line rescanned
  all of the file's metadata, so 1M `index` lines (8 MB) took 139 s and held the
  daemon's engine past every client deadline; it now takes 0.20 s.
- **A refused learning receipt exits 4** (change the input), not 3 (a
  retryable I/O failure), as the README documents.
- **A second `daemon start` never replaces a live daemon's socket.** It
  exits 6 instead of orphaning the running daemon.
- **`daemon --socket PATH health` works.** Commands take their positionals
  from argument admission, so an option's value is never read as the
  subcommand. `--foreground` belongs to `daemon start` alone (which always
  serves in the foreground) and is refused elsewhere.
- **The daemon never sends a response over its 80 MiB line cap**; it answers
  `RESOURCE_EXHAUSTED` (8) for the same id instead of a line the client
  refuses. The README now says one engine serves the eight connections.
- **The daemon client reads a complete response on macOS** when the daemon
  closes first, and gates never fall back to running as root where `/proc` is
  missing (found by the first macOS test runs).
- **No trust policy trusts a strategy with no receipts**, even with
  `min_samples: 0`.
- **The Python `SocketClient` raises `TransportError`** for a missing socket, a
  refused connection or a timeout, as documented, not a bare `OSError`.
- **The README's daemon section** lists all ten methods and protocol 1, under a
  test.

- **Antipattern rows record what enforces them** (anchor B-10). A catalogue
  row may carry a DDF-owned `ddf_enforcement` claim beside its verbatim import
  labels; the policy checker accepts it only when every piece of evidence
  resolves against parsed configuration (a workspace lint at `deny`, a path
  in `clippy.toml`'s `disallowed-methods`), and its report lists the enforced
  rows. AP-17 (unreviewed lint suppression, B-03) and AP-06 (policy coupled to
  clock or host, B-04, with its scope stated) are the first two.
- **Every pull request is mutation-tested on its changed lines** (anchor
  B-01). A new four-shard `diff mutation` job runs `scripts/diff_mutation.sh`,
  which mutates the Rust lines the pull request changes since its merge base
  (`cargo mutants --in-diff`) and fails on any missed mutant or timeout. A
  change with no Rust lines under `crates/` passes and says so. Before, CI
  mutated two functions of the review crate, and "0 missed on every diff" was
  held by habit.
- **The local CI replica and pre-push hook are versioned** (anchor B-07).
  `scripts/ci_local.sh` replicates every step of the CI gate jobs and fails
  closed; each step is labelled with the CI step it replicates, and
  `tests/ci_replica.rs` fails when a CI step has no replica step or a replica
  step names no CI step. `just ci` runs it (before, `just ci` was the feature
  gate alone, without docs, deny, audit, SBOM, MSRV, soak, probes, the SDK test
  or the fuzz compile). The gitleaks pre-push hook is `scripts/hooks/pre-push`;
  `just install-hooks` installs it and `just doctor` reports whether it is
  installed.
- **CI runs weekly** (anchor B-11). A `schedule:` trigger (Monday 04:23 UTC)
  runs the full workflow, so a new RustSec advisory or a yanked dependency
  fails within a week even when nothing is merged.
- **The crate graph is a committed table** (anchor B-05).
  `docs/crate-edges.txt` lists every edge between workspace crates (normal,
  dev and target-specific kept apart), and `tests/crate_edges.rs` compares it
  with `cargo metadata`: a new dependency between workspace crates fails until
  the table is changed in a reviewed diff, and a row no `Cargo.toml` declares
  fails as stale.
- **Effects go through named doors** (anchor B-04). A new `clippy.toml`
  disallows `Command::new`, `env::var`/`var_os`, `Instant::now`,
  `SystemTime::now` and `thread::sleep`. The 21 functions that legitimately
  spawn, read the environment or read a clock each carry
  `#[expect(clippy::disallowed_methods, reason = "<door>")]` naming the door,
  and the 20 integration-test files that drive real processes opt in once at
  the top. A new effect anywhere else no longer passes clippy.
- **MCP `review_patch` returns a bounded summary for large reviews** (anchor
  A-07; Luke's decision). The full document is returned while it is at most
  256 KiB; a larger one comes back as `deep-diff-forge.review-summary.v0` (the
  same seal and totals, per-file counts for up to 1,000 files, `reason`,
  `omitted_files`). `paths` returns chosen files in full and `detail: "summary"`
  asks for the summary. A 30 MB patch now answers in 181 KB instead of one
  178 MB line.
- **Review council** (`council prepare|status|review|gate`) — six default
  reviewer slots across Claude Code and Codex, independent then challenge
  rounds, controller admission that rejects fabricated citations, and `HOLD`
  (exit 7) for missing evidence. See `docs/REVIEW_COUNCIL.md`.
- **Gate sandbox** — the eight evidence gates (format, check, clippy, pedantic,
  tests, antipatterns, fuzz, mutation) run on a sealed snapshot in a rootless
  Podman container: image by ID, `--network=none` except dependency vendoring,
  read-only root and source, all capabilities dropped, `--userns=keep-id`. A
  real-Podman planted-breach suite (12 probes) guards it.
- **Diff-scoped mutation gate** — cargo-mutants 27.1.0 `--no-config --in-diff`
  against a scope diff built from verified base and candidate blobs
  (`mutation_scope`, `mutation_base`), 300 s per mutant, 4 jobs.
- **Exemption oracle** (`policy/exemption_oracle.py`, schema v7) — run before
  the mutation job; only `#[cfg(test)]` may hide code from mutation, and that
  code must leave every workspace member's non-test MIR unchanged in the gate
  and release builds. Anything else hidden is refused with no builds.
- **Git acquisition** (P1a) — `--git-working-tree [--include-untracked]`,
  `--git-staged`, `--git-range A..B`, `--repo`, `--acquisition-json`
  (`deep-diff-forge.git-acquisition.v0`).
- **`council import-tuicr`** decision journal (P1c, DDF-R18) — appends tuicr
  dispositions to an append-only `<run>/decisions.jsonl`; dry run by default.

- **`council export-tuicr`** (DDF-R54) — files controller-admitted council
  findings into a local tuicr 1.3 session as local draft notes. Dry run by
  default; `--execute` writes through `tuicr review add` with fixed argv.
  Refuses tampered reports, forge sessions, a different repository and, unless
  `--allow-review-level` is given, findings whose cited bytes in the session's
  revision differ from the run's sealed snapshot. A create-new ledger in the
  run makes re-runs no-ops. See `docs/REVIEW_COUNCIL.md#tuicr-export`.
- **Input sealing** — every input-derived machine document (`review.v0`,
  `rank.v0`, `cluster.v0`, `semantic.v0`) now carries two additive top-level
  members directly after `schema`: `input_sha256` (lowercase hex SHA-256 over
  the exact bytes read, before any parsing or normalisation) and `tool`
  (`{"name": "deep-diff-forge", "version": …}`). An orchestrator can tie a
  verdict to exact input bytes and an exact tool build. Schema names are
  unchanged (`.v0`): the fields are purely additive. SHA-256 is implemented in
  `deep-diff-forge-core` (`sha256`, `sha256_hex`, `InputSeal`) in pure safe Rust
  with no new dependency, tested against the NIST vectors;
  `deep-diff-forge-patch` gains `to_json_sealed`.
- **`--require-files`** for `--stdin-patch` — refuse a patch that parses to 0
  files with the new **exit code 7**, an empty stdout, and
  `refused: 0 files in input (--require-files)` on stderr. Gates should always
  pass it so an empty or mis-piped input is never read as a clean review.
  Default behaviour without the flag (0 files → exit 0) is unchanged.
- **`--require-hunks`** for `--stdin-patch` — closes the gap `--require-files`
  leaves open: a header-only (`diff --git a/x b/x`) or rename-only diff parses
  to one file with 0 hunks and passed the files guard with exit 0. The new
  guard refuses a patch with 0 hunks or 0 added+removed lines across all files
  (header-only, rename-only, context-only hunks) with the same **exit code 7**,
  an empty stdout, and `refused: 0 hunks in input (--require-hunks)` on stderr.
  A gate should pass **both** flags; when both would fail, the files guard
  reports first. Default behaviour without the flag is unchanged.
- Determinism contract test: `--rank --json --require-files` is byte-identical
  across runs with an identical `input_sha256`.
- **Stdio server fuzz targets** — `stdio_mcp` and `stdio_rpc` compile the
  CLI's `mcp.rs` and `rpc.rs` into the fuzz crate and drive their stdio
  loops in process, at the real cap or a small one. They assert one JSON-RPC
  2.0 object per output line, exactly one of `result`/`error`, and at most one
  answer per input line. CI runs a 10 s instrumented smoke from
  `fuzz/seeds/`.

- **`learn` is tested end to end** (anchor A-08, from Parking). Recording a
  receipt, reading the count back, the default `status` subcommand, and the
  refusals (a path in place of the redacted id, a self-contradicting receipt,
  an unknown subcommand, `record` without `--stdin`) now run against the
  binary in an isolated `XDG_STATE_HOME`. Before, replacing `learn_cmd` with a
  no-op survived mutation.
- **A self-contradicting learning receipt is refused** (anchor C-11). A
  receipt whose outcome is `revisited` but whose `revisited` flag is false is
  refused on append and on load, so scoring cannot count the revisit in one
  rate and not the other.
- **Contradicted facts corrected** (anchor E-02). `--help` no longer lists
  shipped commands (`review`, `daemon start`, the `--git-*` family) as future
  modes; its "not yet implemented" list is checked to be refused. The
  crates.io publish loop in `release.yml` now includes `sdk`, and a test keeps
  it naming every workspace crate after its dependencies. The API doc states
  protocol 1, the release doc describes per-archive SHA-256 files and
  attestations (no signed `checksums.txt`), and the feature matrix marks the
  options that do not exist.
- **An unmeasured test no longer shows as passed** (anchor B-08). The
  real-Podman sandbox test and the two real-tuicr tests printed `UNMEASURED`
  and returned, so `cargo test` reported them `ok`. They are now `#[ignore]`d
  with the command that runs them, and an explicit `--ignored` run without
  their environment fails; CI runs the sandbox test with `--ignored`.
- **Lint suppressions carry a reason and cannot go stale** (anchor B-03). The
  nine workspace `#[allow]` sites (and two in the fuzz targets) are now
  `#[expect(.., reason = "..")]`, and clippy's `allow_attributes` and
  `allow_attributes_without_reason` are denied in the workspace and fuzz
  manifests: a bare `#[allow]` no longer builds, and an expectation whose lint
  stops firing fails as unfulfilled.
- **Code-literal scans cover every source file** (anchor B-02). The exit-code
  scan read a hard-coded list of five files and stopped at the first
  `#[cfg(test)]`, which hid the production code after a mid-file test item
  (`core/src/lib.rs` from line 128, `daemon/src/serve.rs` from 181). It now
  walks every `crates/*/src` file, skips each test item on its own, and also
  catches bare `exit(N)`, `ExitCode::from(N)` and `code: N`. A new scan refuses
  JSON-RPC error-code literals outside their named constants; the MCP server's
  six `-32600` and the SDK's daemon codes `4` and `7` now use the constants.
- **One status vocabulary** (anchor D-01). The JSON, JSONL and projection label
  tables are replaced by `FileStatus::label`, and the human summary no longer
  prints `{:?}` (it printed `binarychanged` and `typechanged` where every other
  surface prints `binary_changed` and `type_changed`).
- **Syntax analysis no longer depends on machine load** (anchor A-04). The
  flat 250 ms parse deadline made one 220 KB Rust file come out complete in 9
  and timed out in 55 of 64 parallel runs, and timed out every 1 MB file even
  on an idle machine. The deadline is now a backstop of 1 s plus 5 ms per KiB,
  about seventeen times the measured linear parse rate at every size: under a
  load average of 33–39, 64 parallel runs of 220 KB, 1 MB and 1.9 MB inputs
  gave one result each. Hostile input is still cut (a 455 KiB token soup that
  ran for 125 s now stops at about 3 s). Counting tree-sitter progress
  callbacks was measured and rejected: that input made fewer callbacks per KiB
  than ordinary code.
- **The daemon recovers fully from a contained panic** (anchor A-03). It used
  to keep answering through the poisoned engine lock while each connection loop
  read the poison as "not running", so after one panic every connection got a
  single answer and was dropped. The sessions the panic may have left
  inconsistent are now cleared (`Engine::reset_sessions`; the pid, start time
  and session-id counter are kept) and the poison is cleared, so the
  connection and later ones are served as before.
- **Every fuzz target runs in CI** (anchor B-06). `patch_parser`,
  `review_json`, `daemon_protocol` and `agent_annotation` only ever compiled;
  the instrumented smoke now runs all seven targets for 10 s each and refuses a
  `fuzz/Cargo.toml` target that the step does not run. New seeds for
  `daemon_protocol` and `agent_annotation`; Dependabot now watches `/fuzz`.
- **Unknown options are refused, not ignored** (anchor A-01). Every entry point
  admits its arguments against a closed set (`cli/src/args.rs`) and exits 2,
  naming the option, with an empty stdout. Before, a misspelt guard such as
  `--require-file` was silently skipped, so a gate passed every empty patch
  with exit 0. Commands that take no arguments (`doctor`, `--version`, the
  `*-contract` printers) and `schema` now refuse extra ones too. A test ties
  every option in `--help` to an invocation that admits it.
- **A non-UTF-8 line no longer refuses the whole review** (anchor C-12).
  Real repositories commit non-UTF-8 content (Latin-1 sources, fuzz corpora):
  2 of 33 HEE v5 commits, and DDF's own merge #11 (binary fuzz seeds), failed
  with exit 3. Now `patch::decode` keeps valid lines byte-exact and decodes an
  invalid line lossily (U+FFFD); the parser marks only that line's file
  (`PatchTwin::lossy_lines`, shown as `"lossy_lines": N` in the review JSON and
  JSONL, and in the human summary), and every seal covers the exact input
  bytes (`seal_of` takes bytes; `CapturedPatch::capture_bytes`,
  `original_bytes`, `is_exact`). Output for UTF-8 input is byte-identical. A
  non-UTF-8 *path* is still refused, because a path is an identity.
  `GitAcquireError::NonUtf8Patch` is removed.
- **The review JSON records which side lacks a final newline** (anchor A-06).
  A line carries `"no_newline": true` when the marker follows it; its `kind`
  names the side (removed: old, added: new, context: both). Before, patches
  that differed only in which side lacked the newline produced identical JSON,
  although git applies one and refuses the other. Other output is unchanged.
- **The learning store validates records on load** (anchor A-05). A hand-edited
  line holding a path (or a path-like token) is refused, naming its line; before,
  only `append_receipt` applied the privacy checks.
- Non-Unix `path_from_bytes` (`deep-diff-forge-patch`) reuses valid UTF-8
  without a copy, which clears `clippy::needless_pass_by_value` in a
  Windows-target pedantic build.
- **main's red CI** (failing since 2026-06, run 37532439196): a floating
  `stable` toolchain deprecated `fetch_update` under `-D warnings`; `cargo deny`
  was called without being installed; the SBOM `documentNamespace` embedded
  wall-clock time. The namespace is now content-addressed and `created` comes
  from `SOURCE_DATE_EPOCH` or the last `Cargo.lock` commit, so regenerations are
  byte-identical. main CI is green at run 40274253094.
- **Sandbox worked only for uid 1000**: gate containers ran as the image's
  `USER 1000`, so an operator with another uid (1001 on GitHub runners) could
  not read the private source copy. Containers now run as
  `--user=<uid>:<gid>` read from `/proc/self`.
- **Probe verdict race**: parallel probe tests could write a verdict mid-line
  after libtest's `test name ... `, losing it (main run 40268250417). Verdicts
  now start with a newline and are parsed anywhere in a line.
- **`mutation_scope` emitted touching hunks** (latent since r10): a widened
  pure insertion could start on the line the previous hunk ended on, and
  cargo-mutants rejected the diff. Regions now merge on touching; property
  tests assert header ordering. Surfaced by council r17.
- The oracle removes its build trees on exit (they filled `/tmp`).
- `council_report` fuzz target builds again after the `mutation_diff_sha256`
  field.

### Security

- **No text change can hide behind a binary patch body.** After
  `GIT binary patch`, every later line was taken as binary data, so a crafted
  patch was reviewed as one binary file while `git apply` also changed a text
  file. The body now takes only the lines of git's binary grammar; such a patch
  is refused (exit 4).
- **Ids become paths only as checked segments** (anchor C-09). Every reviewer,
  job, session and fuzz-target id joined into a council run directory now goes
  through one check (`segment`, `job_dir`, `report_path`) at the join, so a
  path cannot leave the run directory even where the id was not validated
  first; before, the joins relied on `identifier()` having been called
  earlier.
- **MCP marks patch text as untrusted for the consuming model** (anchor E-07).
  The server instructions and the `review_patch` / `rank_patch` descriptions
  say that paths and line text come from the patch, whose authors wrote them,
  and are data to review, never instructions. Reviewed code and comments are
  an injection channel into a model's context; nothing said so before.
- **One hardened Git door** (anchor A-02). Git acquisition (`patch`) and
  council storage (`review`) now apply the same set from
  `deep_diff_forge_core::git`: no optional locks, replacement refs, fsmonitor,
  hooks, external diff or transports (`protocol.allow=never`,
  `GIT_NO_LAZY_FETCH`), and every inherited variable that would point Git at
  another repository removed. Measured before: `--git-range` in a partial clone
  ran `git fetch origin` against the promisor remote (a network effect from a
  read-only command); council storage honoured an inherited `GIT_DIR`, so
  `rev-parse HEAD` named another repository while the top-level check passed.
  A partial clone's missing blob is now an error, not a fetch.
- Council gates never fall back to the host: a missing gate image or a failed
  dependency-vendoring job is a refusal. Vendored dependencies are sealed and
  re-measured before each gate; base Git objects are verified on read.
- The mutation gate ignores the candidate's cargo-mutants config, and code
  hidden from mutation by anything other than `#[cfg(test)]` is refused.

## [0.2.1] - 2026-09-04

### Security

- Hardened the Unix daemon against symlink/inode-swap attacks, slowloris and
  stalled-reader clients, unbounded worker creation, oversized responses, and
  multi-session memory exhaustion. JSON-RPC fields are now shape-validated.
- Added a structural patch line budget and strict size/line/count, ownership,
  permission, and pseudonymous-identifier validation to the learning store.
- Upgraded Ratatui/Crossterm so the dependency graph uses patched `lru` 0.18.4
  and removed all RustSec advisory waivers.
- Restricted release publication to version-matching tags, stopped persisting
  checkout credentials, pinned the audit tool, and made the publish version
  derive from the workspace manifest. The separate fuzz dependency graph now
  passes the same advisory, license, source, and wildcard-dependency policy;
  CI/release concurrency and dependency-update cooldowns are explicit.

[0.2.1]: https://github.com/Louranicas/deep-diff-forge/releases/tag/v0.2.1

## [0.2.0] - 2026-06-23

The **L9 (Learning)** layer plus the first crates.io-publishable cut, a
supply-chain & fuzz hardening pass, and the interactive **review cockpit** (ranked
tree, multi-file diff, inline agent notes, command palette, colour themes, mouse)
— 12 crates, 927 tests, zero `unsafe`, supply-chain-gated. `cargo publish
--dry-run` is clean across the workspace; the crates.io upload itself remains
token-gated.

### Added

- **Interactive review cockpit** — `review [--probe] [--side] [--palette | --cmd
  NAME]` (+ `deep-diff-forge-tui`, ratatui): a review-first terminal UI with a
  risk-ranked file tree, a multi-file diff pane, and the engine's own findings as
  inline grounded agent notes. Runtime view toggles for layout (inline ↔
  side-by-side), fold, line-wrap, and notes; a per-file **reviewed flag** (`v` /
  `Space`, auto-advancing, with a `viewed:n/total` progress indicator) for triage;
  three colour themes (`T`: dark · midnight · mono); a top menu bar and a `?`
  keybinding help card; and full **mouse** support (scroll, click-to-focus,
  sidebar selection, menu clicks). State is pure and headless-testable
  (`render_to_lines`); `--probe` renders one frame with no TTY for CI/agents.
- **Command palette** — `:` in-app, or headless via `review --palette` / `--cmd
  NAME`, runs any engine capability against the loaded review and shows it in a
  result panel: `rank`, `outline` (tree-sitter symbols of the changed regions),
  `cluster`, `summary`, `notes`, `review` (the `review.v0` JSON), live `daemon`
  health/status, `learning` scores, and the `maturity` ladder. In-process and
  read-only; never mutates patch truth.
- **`deploy release`** — `deploy release [--json]` (schema
  `deep-diff-forge.release.v0`): a declared per-target publication snapshot
  (GitHub, GitLab, GitHub release, crates.io) with a `pending` list; `crates.io`
  reports `blocked` until a registry token is configured.
- **Syntax highlighting** — `highlight <path> [--color|--no-color]` (+
  `syntax::highlight`): tree-sitter highlighting reusing the grammar's own
  `highlights.scm`, mapped to an ANSI palette. Colour auto-detects a TTY.
  Terminal-injection-safe: source text is routed through `display_safe`, so the
  only raw escapes are fixed SGR codes.
- **Structural diff** — `structural <old> <new> [--json]` (+
  `syntax::structural`, schema `deep-diff-forge.structural.v0`): a token/leaf-level
  (AST) diff over tree-sitter token streams. **Reformat-aware** (layout-only edits
  report zero structural change) with best-effort moved-block detection; never
  touches patch truth. Bounded LCS with graceful degradation on huge inputs.
- **Learning (L9)** — `deep-diff-forge-learning`: the local-only learning loop.
  Per-decision [`StrategyReceipt`]s (hashes/counts/timings — never a path or
  source line) are appended as JSONL under `$XDG_STATE_HOME/deep-diff-forge/
  learning/`, scored per strategy, and gated for promotion. Three learning units
  — planner (strategy + budget), ranking (risk weights), annotation (agent trust
  tiers, *untrusted until grounded*) — plus an explainable promotion gate. CLI
  `learn {status|record --stdin} [--json]`. 135 tests; never uploads, never
  mutates patch truth.

### Changed

- **Daemon socket handling** (Zen review) — replaced scattered `Option<PathBuf>`
  resolution with a parse-don't-validate `SocketLocation` type: one
  `SocketLocation::resolve(explicit)` entry point (explicit `--socket` → else
  `$XDG_RUNTIME_DIR` → else a typed `NoRuntimeDir`, no `/tmp` fallback), with
  `bind`/`connect`/`path` as methods. A "no location" state is unrepresentable
  past construction, and the CLI's nested `if let` collapses to one call.
- **Publishable-workspace manifests** — versions and internal deps are now
  inherited via `[workspace.package]` + `[workspace.dependencies]`; every crate
  carries `description`, `keywords`, `categories`, and `homepage`. `core` and
  `cli` gained the `description` that `cargo publish` requires.

### Fixed

- **Patch truth: malformed/truncated hunks are now rejected** (Zen review) — the
  parser enforces each hunk's declared `@@ -a,b +c,d @@` counts exactly. A hunk
  that ends (at a new header, a new file, or EOF) before its counts are satisfied
  is a `TruncatedHunk` error; a body line that over-fills a side is a
  `HunkLineCountMismatch`. An internally-incoherent patch no longer parses as
  apply-able truth.
- **Broken-pipe handling** — bulk CLI output (`--stdin-patch`, `--rank`,
  `--cluster`, `--jsonl`, `semantic`, `review --probe`, `learn`) is routed
  through a broken-pipe-tolerant writer, so `deep-diff-forge … | head` exits `0`
  instead of panicking with `EPIPE` (exit 101). Resolves the L1-era open finding;
  std-only, no `unsafe`.

### Security

Pre-publication adversarial hardening (S1008412): an 8-dimension STRIDE review
with independent verification produced a CVSS-scored register of 17 confirmed
findings (4 MEDIUM, 12 LOW, 1 INFO; **no Critical/High**), all remediated with
fail-before/pass-after regression tests. A later final-hardening review fleet
(5 reviewers + adversarial verify) added the items below. See `SECURITY.md`.

- **Trojan Source defence (CVE-2021-42574)** — `display_safe` now also escapes
  bidirectional/invisible Unicode (`U+202A–202E`, `U+2066–2069`, directional
  marks, zero-width, BOM) to a visible `\u{XXXX}`, so attacker source cannot
  display differently than it reads — on-mission for a review tool.
- **Cross-platform contract made honest** — the UDS daemon is Unix-only; it now
  emits a clear `compile_error!` on non-Unix instead of a cryptic `std::os::unix`
  failure, and the release matrix dropped its (un-buildable) Windows lane so CI
  and code agree before the irreversible publish (Linux + macOS).
- **Broken-pipe fix completed** — the remaining `deploy`/`doctor`/`--help`/
  contract commands (notably the `--json` outputs CI/agents pipe to `head`/`jq`)
  now route through the broken-pipe-tolerant writer; `… | head` exits `0`, not 101.
- **`structural` on a non-Rust file** now errors explicitly instead of silently
  reporting "formatting only"; **`doctor`** resolves both its runtime-dir and
  socket lines through the daemon's `var_os` resolver (consistent on non-UTF-8 env).
- **Terminal/ANSI-escape injection** — attacker-controlled diff bodies, file
  paths, and symbol names are now sanitized by `core::display_safe` (control
  chars escaped to a visible `\xHH`) at every human-render boundary; `json_escape`
  also neutralises `DEL` + the C1 block. A reviewer rendering a hostile patch can
  no longer have their terminal hijacked (screen clobber, scrollback forgery,
  OSC-52 clipboard write, title/hyperlink spoof).
- **Memory-exhaustion DoS** — stdin, source-file, and daemon request reads are
  hard-capped at the byte budget instead of buffering unbounded input.
- **Daemon robustness** — per-connection panic/error isolation (`catch_unwind`,
  no fatal `?`), a read timeout (slowloris), a size-bounded request line, symlink
  rejection, and **no world-writable `/tmp` socket fallback** (fails closed
  without `$XDG_RUNTIME_DIR`).
- **Fail-closed trust** — agent annotation `source` is no longer inferred from an
  attacker-controlled label; learning-store files are owner-private (`0700`/`0600`).
- **Supply chain** — `unsafe_code = "forbid"` is now compiler-enforced
  workspace-wide; `tree-sitter` is pinned to exact versions; a strict `cargo
  audit` gate runs in CI and gates the irreversible crates.io publish;
  least-privilege workflow permissions; Dependabot; `SECURITY.md`.

A second bias-controlled re-review (S1008443) raised three further findings, each
fixed and independently verified (security / tester / claim-verifier judges,
outside the build loop):

- **Mandatory hunk-header closer** — `parse_hunk_header` now requires the closing
  `@@` token; a header with trailing garbage and no closer (`@@ -1,1 +1,1
  NOT_A_HUNK`) is rejected (exit 4) instead of normalizing into a valid model.
  Completes the patch-truth hardening alongside the truncated-hunk fix.
- **Explicit `--socket` no longer mutates caller state** — `SocketLocation` carries
  provenance; an explicit `--socket` path binds via a fail-closed `bind_explicit`
  that **creates** an absent parent at `0700` but **never chmods a pre-existing
  parent** (a world-readable parent fails closed rather than being silently
  tightened to `0700`) and removes the socket path **only if it is verifiably a
  socket** (never a regular file, directory, or symlink). The engine-owned
  `$XDG_RUNTIME_DIR` path is unchanged, and `daemon start --socket /new/path` still
  works (the absent parent is created).
- **Bounded daemon sessions** — review sessions are capped at `MAX_SESSIONS` (64)
  with LRU eviction (least-recently-used evicted before insert; reads refresh
  recency), hard-bounding daemon memory regardless of how many sessions a client
  opens without closing.

A supply-chain & fuzz hardening pass then raised the release/CI posture:

- **GitHub Actions pinned to commit SHAs** across CI and release (tag-hijack
  defence); the release workflow now emits **SLSA build-provenance attestations**
  for the binary, its checksum, and the SBOM.
- **SPDX SBOM** (`sbom.spdx.json`) generated from `Cargo.lock`/`cargo metadata`,
  gated in CI and uploaded with each release.
- **Fuzz harness** (`fuzz/`, cargo-fuzz) — four targets (`patch_parser`,
  `review_json`, `daemon_protocol`, `agent_annotation`), excluded from the
  publishable workspace; a compile gate keeps it building in CI.
- **CI security probes** — a hostile-daemon soak, a learning-privacy probe, and a
  mutation-gate inventory, exposed as `just security-*` recipes.
- Added a JSON-RPC wire error-code regression test and empty-input
  `--json`/`--jsonl` stability tests. Corrected the daemon's `lib.rs` doc from
  "thread-per-connection" to the true single-threaded sequential accept loop.

[Unreleased]: https://github.com/Louranicas/deep-diff-forge/compare/v0.5.0...HEAD
[0.5.0]: https://github.com/Louranicas/deep-diff-forge/releases/tag/v0.5.0
[0.4.0]: https://github.com/Louranicas/deep-diff-forge/releases/tag/v0.4.0
[0.3.0]: https://github.com/Louranicas/deep-diff-forge/releases/tag/v0.3.0
[0.2.0]: https://github.com/Louranicas/deep-diff-forge/releases/tag/v0.2.0

## [0.1.0] - 2026-06-22

First tagged release. The complete diff/review engine (layers L1–L7,
Patch→Daemon) at deployment maturity **L8 (Release)** — 11 crates, 568 tests,
zero `unsafe`, supply-chain-gated.

### Added

- **Patch (L1)** — `deep-diff-forge-patch`: unified/Git patch parser, apply-able
  renderer, and `deep-diff-forge.review.v0` JSON. CLI `--stdin-patch [--json]`.
- **Projection (L2)** — `deep-diff-forge-projection`: renderer-neutral inline and
  side-by-side views. CLI `--layout inline|side-by-side`.
- **Pipeline (L3)** — `deep-diff-forge-pipeline`: composable Unix-filter stages
  (`ChainStage`) and JSONL streaming. CLI `--jsonl`.
- **Semantic (L4)** — `deep-diff-forge-syntax`: tree-sitter language detection,
  budgeted parsing, and symbol extraction (Rust). CLI `semantic <path> [--json]`.
- **Review (L5)** — `deep-diff-forge-graph` (Review Intelligence Graph,
  deterministic risk ranking; CLI `--rank`), `deep-diff-forge-agent` (annotation
  provenance, grounding, sanitization), and `deep-diff-forge-tui` (review TUI;
  CLI `review [--probe]`).
- **Cluster (L6)** — `deep-diff-forge-cluster`: bounded parallel dimensional
  execution with deterministic joins and receipts. CLI `--cluster [--parallel]`.
- **Daemon (L7)** — `deep-diff-forge-daemon`: optional UDS JSON-RPC service
  (std-first, owner-private sockets). CLI `daemon {path|start|health|status|stop}`.
- **Deployment surface** — `deploy status` and `deploy release` machine-readable
  reports (`deployment-status.v0`, `release.v0`); CI workflow; `deny.toml`
  supply-chain policy; release workflow (`release.yml`).

### Engineering

- Strictly acyclic 11-crate workspace; `core` is pure vocabulary.
- Patch truth is preserved across every layer (never mutated by enrichment).
- Each production crate carries at least 50 meaningful tests.
- Pedantic clippy clean; no production `unwrap`/`expect`; no `unsafe`.

### Not yet released

- **crates.io publication** is pending a registry token (the release workflow
  publishes automatically when `CARGO_REGISTRY_TOKEN` is configured).

[0.2.0]: https://github.com/Louranicas/deep-diff-forge/releases/tag/v0.2.0
[0.1.0]: https://github.com/Louranicas/deep-diff-forge/releases/tag/v0.1.0
