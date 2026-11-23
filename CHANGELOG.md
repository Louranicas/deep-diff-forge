# Changelog

All notable changes to Deep-Diff-Forge are documented here. The format is based
on [Keep a Changelog](https://keepachangelog.com/), and the project follows
[Semantic Versioning](https://semver.org/).

## [Unreleased]

- Exit 7 has one meaning, with a registry of reason classes. Every exit 7 names its class in one
  stderr line `refused: <text> (<tag>)`. `schema exit-codes`, `explain 7 [--json]` and the MCP
  tools `explain_exit_code`/`exit_code_registry` list them as `refusals` (F7.h1).
- `change_input` no longer says "or drop the contract that refused it". `review_evidence` now
  reads "read the reason and the evidence the message or the council reported; a person decides
  what changes; rerunning does not change it" (F7.h1).
- `findings verify --require-all`, `eval` and `agent doctor` name their refusal on stderr. A
  verdict refusal is decided before its document is written, so `eval` and `agent doctor` exit
  7, not 0, when the reader of stdout has gone (F7.h2,
  [DDF-2026-011](docs/advisories/DDF-2026-011.md)).
- A stdout that cannot be written at all keeps a pending refusal for every class, through the
  one door (0.10.0 fixed it for `findings verify`;
  [DDF-2026-011](docs/advisories/DDF-2026-011.md)).
- `docs/reference/diagnostics.md`: the contract of `deep-diff-forge.diagnostics.v0` (printed by
  `--diagnostics`, F5a), with 42 pinned codes (F7.h1).
- The tree's version is 0.11.0-dev (F7.h1).
- `--rank --fail-on LIST` (on `--stdin-patch`, `--git-*` and `<old> <new>`) is a gate on the
  ranking. LIST holds classes (`security`, `integrity`, `shape`) and labels from
  `schema signals`. The ranking is printed as without the flag. The command then exits 7 when any file's
  `signals[]` holds a selected label (`fail_on_label`), or when any file's evidence list was cut
  at 32 rows, whatever the list (`fail_on_truncated`). Each reason line is followed by the
  escaped paths it names. A reader that closes stdout still gets exit 7. Not on rpc or MCP yet
  (F7.i).
- Exit-7 reason classes `fail_on_label` and `fail_on_truncated` in `schema exit-codes`,
  `explain 7` and the MCP exit-code tools (F7.i).
- `explain signal <label>` says its audience (`"audience": "person"`; last text line);
  `--audience agent` prints what an agent is shown: the accepted repairs and only the
  substitutes with a witnessed agent sentence (F7.c1).
- Every accepted repair and substitute of the vocabulary has a fixed id (`<label>:a<k>`,
  `<label>:n<k>`), pinned in `fixtures/diagnostics/ids.v0.txt`; five agent sentences each stand
  on a witness patch the engine is tested on (F7.c1).
- The graph crate builds `deep-diff-forge.diagnostics.v0` in process: one diagnostic per
  evidence row and one per label that no row locates; keys hold no line text; each diagnostic is
  a `current` or `floor_only` reading. It is printed by `--diagnostics` (F5a) (F1.a1).
- Library API: `Evidence::read_now` says whether a reading of this version finds an item, and
  `RankedFile::floor_only` lists the code labels that only a reading kept for the floor finds
  (F1.a1).
- `tool.scorer_sha256` moves with no label, row or score change (F1.a1).
- The diagnostics builder locates `hazardous_text` at each changed line whose text carries the
  hazard (its place, never its text), within the file's 32 places; a file over that is in `cut`
  (F1.a2).
- `explain signal <label> --json` names the id of every repair (`accept_ids`) and substitute
  (`id`), for both audiences (F1.a2).
- Library API: `RankedFile::hazard_lines`, `hazard_lines_cut`; `Judge` and `Diagnostics::judged`
  write `judge` and `judge_sees` (F1.a2).
- `tool.scorer_sha256` moves with no label, row or score change (F1.a2).
- `--diagnostics` (on `--stdin-patch`, `--git-*` and `<old> <new>`) prints
  `deep-diff-forge.diagnostics.v0`: with `--json` the builder's document; otherwise a text form
  for agents that prints only each path (quoted and escaped), the line, the code, the vocabulary
  ids and their sentences, with floor-only readings counted. The input is refused as under
  `--rank`; `judge` is `unknown` (F5a).
- `--fail-on` is admitted with `--diagnostics`: the same verdict as `--rank --fail-on` on the
  same input, the document printed as without the flag; the message is
  `error: --fail-on needs --rank or --diagnostics` (F5a).
- `schema diagnostics` prints the JSON Schema of `deep-diff-forge.diagnostics.v0` (F5a).
- A `--diagnostics` diagnostic of `hazardous_text` or `diff_suppressed` names its case in `cases`
  (the kinds of hazard; the cause as in `rank.v0`) and lists only the repairs of that case: a
  line with hidden characters is no longer told to fix a path, and a file hidden by an attribute
  is no longer told to remove a NUL byte. `unread`, where the engine cannot tell the cause, lists
  every repair. The text form prints `case:`; `schema diagnostics` and `explain signal` show the
  cases (F1.a3).
- New repair id `diff_suppressed:a3` for a UTF-16 or UTF-32 file: store it as UTF-8, or say why
  it must be wide (F1.a3).
- `tool.scorer_sha256` moves with no label, row or score change (F1.a3).
- `agent check [--session ID | --base REV] [--judge PATH] [--policy NAME] [--ranker-dir DIR]
  [--json]` judges the uncommitted change with the judge, policy and ranker directory of the
  registered Stop hook (never one from PATH), by the policy at the base a session recorded
  (`--session`, its state read only, never written) or at `--base`/HEAD; `--json` prints
  `deep-diff-forge.agent-check.v0`. It names what it used, lists how that differs from a stop,
  and says it is not the stop's answer (F3.b, F3.c).
- A hook's scratch directory is owned by a lock (`flock`) and named `tmp-r` and 16 random hex
  digits: a check or hook in another PID namespace no longer removes a live stop's scratch.
  Directories named `tmp-<pid>` by earlier builds are removed as before (F3.a).
- The partial file a hook state is written through is named at random, not by the process id
  (F3.a).
- `agent check` judges the units a stop judges, in its order and each by the patch a stop makes
  for it: the uncommitted change, the staged change where it is another, then each commit from
  its first parent (`--range A..B`, one line of first parents, or with `--session` the
  session's commits as a stop finds them). It says "Equal to the stop in its units and inputs"
  only with the stop's inputs and every unit a stop of the session answers for judged, never
  that the stop decides the same; `stop.may_differ` and the lines "The stop may differ" say
  where a stop can decide otherwise from the same verdicts (F3.d).
- `agent check --require-pass` exits 7 after the document when there is no unit
  (`require_pass_no_unit`: a clean CI checkout is no pass) or when any unit is not a pass
  (`require_pass_not_pass`), through the one door, so a reader that has gone still gets exit 7;
  the registry has ten classes (F3.e).
- When the policy admits a ranker of the ranker directory, `agent check` judges the unit with
  it, as a stop does, names it in `ranker`, and lists the labels this binary raises that it does
  not as `advice_only`; a ranker that runs out of time, fails or prints no rank document leaves
  `advice_only` null and says why in `advice_unread` (F3.f).
- A check with no judge stages and writes nothing (UNJUDGED with the reason, no unit); a HOLD or
  FAIL names its rules or, by their codes (`judge_note_codes`), what the judge named beside
  them, never the judge's words (F3).
- Every stop appends one record for each unit it judges to the project's ledger,
  `.git/deep-diff-forge/ledger.jsonl` (append only, mode 0600, no link followed, no line text and
  no words of the judge, tree digests salted for each repository); a ledger that cannot be
  written changes no decision and is said (F11).
- `agent check --session` appends one check record to the ledger; a check without a session
  writes nothing (F11).
- `agent verify-branch BASE..HEAD` reads the ledger only and exits 7 `branch_unverified` for a
  commit that no stop passed; the ledger is evidence, not proof. The registry of exit-7 reasons
  has eleven classes (F11).
- `check_suppressed` reads `#[expect(..)]` and `#![expect(..)]`, with a reason or without, as it
  reads `#[allow(..)]` (F2b.b).
- `check_suppressed` reads an allowance a conditional attribute applies:
  `#[cfg_attr(condition, allow(..))]` and `expect(..)`, through a nested `cfg_attr` too, when the
  side holds the attribute whole (F2b.c).
- `check_suppressed` reads Cargo's files: in `Cargo.toml` a key whose value is the string `allow`
  (any TOML quoting, and the `level` of an inline table of `[lints]`), and in Cargo's
  configuration (`.cargo/config.toml`, `.cargo/config`) `-A`, `--allow` or `--cap-lints allow`
  among compiler flags. Not read: a string over lines, `clippy.toml`, `RUSTFLAGS` set in a CI
  file (F2b.d).
- `build_hook` reads a build script or a procedural macro declared in `Cargo.toml`
  (`build = "…"`, `proc-macro = true`) beside `build.rs` itself (F2b.f).
- Changed, by owner decision OD-E (the listed floor exceptions to "nothing scores lower than
  0.8.3"): `#[allow(` in a Rust comment or string is no longer `check_suppressed` in any reading,
  the ones kept from 0.8.3 and 0.8.4 included, and `r#unsafe` (an identifier) is no longer
  `unsafe_added`. On the comparison corpus (7,421 patches) 478 such rows and 3 `r#unsafe` rows
  are no longer reported, and 116 files lose the label (113 `check_suppressed`, 3
  `unsafe_added`); nothing outside the two exceptions is lost. A forge-warden policy that
  forbids `check_suppressed` holds more changes (the new readings) and no longer holds one whose
  only `#[allow(` is in a comment or string (F2b.g).
- `tool.scorer_sha256` moves (F2b).
- `--help` names the values `--cluster --parallel` takes (`serial|auto|N`, as it took before);
  the reference names `--allow-repo` for `mcp`, `rpc` and `daemon start`; the 0.10.0 release
  notes gain an errata list (D1).
- `agent setup --client claude --hooks` takes `--judge PATH`, `--policy NAME` and `--ranker-dir
  DIR` and writes them into the session-start and stop commands it prints (the judge and the
  directory by absolute path; without `--hooks`, a judge that is no file or a directory that
  does not exist: exit 2). 0.10.0's release notes named `--ranker-dir` there, and 0.10.0 refused
  it. `agent --help` lists each form with its own options (D1).

## [0.10.0] - 2026-11-23

**Security and contract release.** 0.10.0 reads a `diff --git` section only if its headers are
one of the 42 shapes Git's own diff writes and refuses every other shape with a named reason,
which replaces the case-by-case header fixes of 0.8.0 to 0.8.4; `CONFORMANCE.md` in the patch
crate is the table of what Git reads. It fixes six misses of released versions, each with an
advisory: a decision journal that one fault could block for good (DDF-2026-004), a CI privilege
grant written with quotes (DDF-2026-005), a source change that one attribute line made
unreadable, now read as text and labelled with the new signal `diff_suppressed`
(DDF-2026-006), GitLab tokens and fine-grained GitHub tokens (DDF-2026-007), a judge's leftover
process that stretched an agent's stop (DDF-2026-008), and a findings gate that passed when the
reader of its output had gone (DDF-2026-010). The stop hook reads forge-warden's verdict version
3 as well as version 2, and a fail whose document it cannot read now holds the stop once
instead of being allowed as UNJUDGED. Contracts change on purpose, each listed under
[Contract changes](docs/release-notes/0.10.0.md#contract-changes) with its test: empty or prose input on `--stdin-patch`
exits 4, `-U0` hunks without context are refused, `rank.v0` has one canonical byte form with
`tool.scorer_sha256`, a Git file name that is not UTF-8 exits 4, `trivial_assert` is narrowed
for Python, and the Rust library gains a `RiskSignal` variant and public fields. Install it
before forge-warden 0.3.0, in the order under [Cutover with forge-warden](docs/release-notes/0.10.0.md#cutover-with-forge-warden),
and keep the 0.9.0 binary as a ranker until every policy admits 0.10.0.

Published as the signed tag `v0.10.0` on GitLab, the place of release, with mirrors on Gitea and
Forgejo and a Linux x86_64 binary built on the owner's machine; GitHub mirrors the release page
and crates.io carries the fourteen crates from the tag, as for 0.9.0.

### Security

- **A `diff --git` section is read only if its headers are a shape Git's own diff writes**
- **`CONFORMANCE.md`**

### Changed

- **New signal `diff_suppressed`**
- **New council exit code 3, `recorded_incomplete`**
- **`--stdin-patch` refuses input in which Git finds no patch**
- **`rank.v0` has one byte form on every surface**
- **The judge's patch is the bytes forge-warden's `make_patch.sh` writes**
- **Binary creations, deletions and renames carry Git's status**
- and 10 more, in the full notes

### Fixed

- **A text file Git was told to show as binary is read as text**
- **`ci_privilege` reads a key or a value in quotes as the key or value**
- **The stop hook reads forge-warden's verdict version 3, and a fail it cannot read holds the stop**
- **A judge that left a process holding its output could stretch an agent's stop past its limit**
- **`secret_like` finds GitLab tokens and fine-grained GitHub tokens**
- **A gate flag failed open when its output was closed**
- **A fault during `council import-tuicr --execute` no longer blocks a run's journal for good, reports a recorded decision as refused, or leaves a cut back undetected**
- and 7 more, in the full notes

Full notes: [docs/release-notes/0.10.0.md](docs/release-notes/0.10.0.md)

## [0.9.0] - 2026-11-18

**Feature release with fixes for released versions.** 0.9.0 adds five integrity signals for a
change that weakens its own checks (`test_skipped`, `trivial_assert`, `stub_added`,
`error_swallowed`, `panic_path_added`; Rust hunks are read token by token across their lines,
`test_removed` included), the signal vocabulary as data (`schema signals`, `explain signal`)
and the agent hooks (`agent hook`: a session start, each tool call and a judged stop). Three
fixes were found while the lines were integrated: the sections of one path that Git applies in
order are read as one file; a long word or a run of secret prefixes costs its length, not its
square; and one rule bounds the cost of one long line. Of these, 0.8.4 and earlier missed a
process spawn cut across two sections of one path, and took time quadratic in the length of one
JavaScript word (26 s of CPU for 200,000 letters on 0.8.3 and on 0.8.4) or of a run of secret
prefixes ([DDF-2026-003](docs/advisories/DDF-2026-003.md)). The one breaking change is in the
Rust API of `deep-diff-forge-graph`: `EvidenceSignal`, which is not `#[non_exhaustive]`, has
five more variants; `Evidence` has two more public fields (`basis`, `test_likely`); and
`hunk_signals` takes the hunk's place in its file and returns an `Option`. The JSON documents
and exit codes are not broken: `rank.v0` gains values (five signal labels, the basis `hunk`,
which an item 0.8.4 marked `lexical` can now carry, and `test_likely`), and `schema result` and
`schema exit-codes` print what 0.8.4 prints. **Upgrade note:** a forge-warden that does not know
the five new labels reads each as an unknown signal and holds. This release has no GitHub
release: it is published as a signed tag on GitLab, Gitea and Forgejo; binaries built locally
for Linux x86_64.

### Integration (2026-11-18)

- **A long word costs its length, not its square (FINDING-5; in every release up to 0.8.4).**
- **One rule for the cost of one long line, and a test that holds it.**
- and 4 more, in the full notes

### Added

- **Three evidence signals for a change that weakens its own checks**
- **`error_swallowed`, and evidence that says how it was found**
- **The signal vocabulary as data: `deep-diff-forge schema signals`.**
- **`deep-diff-forge explain signal <label> [--json]`**
- **The agent loop: `deep-diff-forge agent hook session-start|post-tool|stop --client claude`.**
- **`deep-diff-forge agent doctor --gate`**
- and 27 more, in the full notes

### Testing

Tests, corpora and the review record are in the full notes.

### Changed

- **`basis` in `rank.v0` evidence is no longer always `lexical`.** (details superseded before release; see the full notes)
- **A hunk's evidence can depend on the hunks Git must put beside it.**
- **Every Rust hunk is also read as release 0.8.3 read it**
- **Existing Rust signals can be found where they were not before.** (details superseded before release; see the full notes)
- and 1 more, in the full notes

### Security

- **A construct cut in two by a hunk boundary, or a comment carried over one, is read as Git applies it**
- **Rust code after a nested block comment, or after a character literal with a long escape, was hidden from every Rust rule**

Full notes: [docs/release-notes/0.9.0.md](docs/release-notes/0.9.0.md)

## [0.8.4] - 2026-11-16

**Security release.** v0.8.3 and earlier could review a patch otherwise than `git apply`
writes it: a `Files … differ` line, which Git reads as a binary marker and answers with a
blob from its object store, and one path in several sections, which Git applies as a set
(independent rescore of v0.8.3, G1 and G2, high; L1, low). They also hid Rust code after a
nested block comment, after a character literal with a long escape, or after a comment or
a string that an earlier hunk of a created file opened, from every rule that reads Rust
code (found 2026-11-14). Upgrade from 0.8.3; the one breaking change is a new
`PatchParseError` variant (the enum is `#[non_exhaustive]`). G3 of the same rescore
(medium) and the known limits listed under 0.8.3 are not fixed here. This release has no
GitHub release: it is published as a signed tag on GitLab, Gitea and Forgejo; binaries built locally for Linux x86_64.

### Security

- **`Files … differ` is the binary marker `Binary files … differ` is**
- **A Git section with no hunk and no change in its headers is refused**
- **One path in several sections is refused, except in two section shapes that Git can apply in order when their hunks apply**
- **A traditional diff with `/dev/null` on both sides is refused**
- **Rust code after a nested block comment, or after a character literal with a long escape, was hidden from every rule that reads Rust code**
- **In a file the patch creates, code after a block comment or a string that an earlier hunk had opened was hidden**

### Changed

- **`deploy release` and `--help` no longer claim GitHub**
- **Built and published without GitHub.**
- and 1 more, in the full notes

### Fixed

- **The Git differential applies patches for real.**

Full notes: [docs/release-notes/0.8.4.md](docs/release-notes/0.8.4.md)

## [0.8.3] - 2026-11-11

Tagged `v0.8.3` at `a74bf91` (2026-11-11 UTC), signed.

**Security release** ([GHSA-rq5r-p3g9-phff](https://github.com/Louranicas/deep-diff-forge/security/advisories/GHSA-rq5r-p3g9-phff)). v0.8.2 and earlier could review a
change on a different file, or with a different mode or type, than `git apply` writes: a
name left behind by a `diff --git` line that a binary line follows directly, a new file's
mode overridden by a `new mode` line (independent rescore of v0.8.2, F3 high, F2 medium),
and content under a mode Git reads as a submodule (found while the fix was reviewed, high).
Upgrade from 0.8.2; the changes are fixes and three dependency updates, and no interface
changes. Three findings of the same rescore are not fixed here and are listed under Known
limits.

### Security

- **No name carries over from a `diff --git` line Git reads no patch from**
- **A created or deleted file's mode is not overridden by `old mode` or `new mode`**
- **A mode Git reads as a submodule cannot carry content**

### Fixed

- **The Git differential puts one section's leftovers in front of another.**
- and 1 more, in the full notes

### Dependencies

- **unicode-width 0.2.3, rustix 1.1.6 and signal-hook 0.4.6**

### Known limits (from the same rescore; not fixed here)

- **A binary body Git finds corrupt (F1, medium).**
- and 1 more, in the full notes

Full notes: [docs/release-notes/0.8.3.md](docs/release-notes/0.8.3.md)

## [0.8.2] - 2026-11-09

Tagged `v0.8.2` at `fb5ffe1` (2026-11-10), signed.

**Security release** ([GHSA-gp5v-m67f-gmgp](https://github.com/Louranicas/deep-diff-forge/security/advisories/GHSA-gp5v-m67f-gmgp)). v0.8.1 and earlier could
review a different file or mode than `git apply` writes: a copy under a name other than its
`copy to`, a Git section whose `---`/`+++` name `-p1` leaves nothing of, repeated mode
headers (independent rescore of v0.8.1, N1 and N2 high, N3 medium), a copy, rename or mode
line after a binary line, and a move name with a carriage return in it (rescore of the
untagged 0.8.2, F1 high, F2 medium). Upgrade from 0.8.1; every change below is a fix, and no
interface changes.

### Security

- **A copy's reviewed destination is the one Git writes**
- **A Git section's `---`/`+++` name that `-p1` leaves nothing of is refused**
- **A binary line ends a Git section's header**
- **A move name with a carriage return in it is refused**
- **A repeated mode header, or an `index` mode beside a mode header, is refused**
- **What Git refuses as inconsistent is refused**
- and 2 more, in the full notes

Full notes: [docs/release-notes/0.8.2.md](docs/release-notes/0.8.2.md)

## [0.8.1] - 2026-11-07

Tagged `v0.8.1` at `d290379` (2026-11-08), signed.

**Security release.** v0.8.0 reviewed a rename to a different destination than `git apply`
writes when a line Git does not read as a header (a blank line, any text) came between two
`rename to` lines (independent rescore of v0.8.0, F1, high). Upgrade from 0.8.0; every change
below is a fix or an addition, and no interface changes.

### Added

- **Every council result names its schema**

### Fixed

- **A signal stops the daemon even when its socket path is gone, and a second signal ends the drain**

### Security

- **A rename's reviewed destination is the one Git writes**
- **A Git section's `---`/`+++` after its header ended is refused**
- **`+++ /dev/null` in a Git section needs `deleted file mode`**
- **A traditional new or deleted file must only add or only remove lines**
- **A release is built only from a tag on `main`**
- and 1 more, in the full notes

Full notes: [docs/release-notes/0.8.1.md](docs/release-notes/0.8.1.md)

## [0.8.0] - 2026-11-06

Tagged `v0.8.0` at `dd1aa89` (2026-11-07 local time; 2026-11-06 UTC), signed.

**Breaking.** 0.8.0 is a minor release because it changes contracts 0.7.0 callers rely on:
`PatchParseError` gains four variants (`MisplacedHunk`, `LossyHeader`, `HunkWithoutChange`,
`SectionWithoutPatch`) and is now `#[non_exhaustive]`; patches Git refuses or reads differently
are refused (exit 4) instead of reviewed; and non-UTF-8 input exits 3 where some commands
exited 4. Each is listed below.

### Security

- **A client that reads slowly can no longer hold the daemon's read budget**
- **Every child process DDF waits for has a deadline and capped output.**

### Added

- **`findings verify --require-findings`**

### Changed

- **Release binaries are built from one codegen unit**
- **Non-UTF-8 input exits 3 (`io`) everywhere**
- **`review.change` checks its deadline between its two Git stages**
- **A malformed hunk header can no longer hide a file from the review**
- **A hunk is accepted only where Git reads one**
- **A header line that is not UTF-8 is refused**
- **Patches Git refuses or reads no patch from are refused:**
- **`PatchParseError` is `#[non_exhaustive]`**
- **One place decides a parse refusal's exit code**

### Testing

Tests, corpora and the review record are in the full notes.

### Fixed

- **A daemon at its connection limit is no longer reported absent on macOS.**
- **Two daemon tests no longer race the server on macOS**
- **Three daemon paging tests are bounded**

Full notes: [docs/release-notes/0.8.0.md](docs/release-notes/0.8.0.md)

## [0.7.0] - 2026-11-03

PRs #95 and #96 merged before the tag, so their entries below ship in v0.7.0 (`3823fcf`).

### Security

- **Release archives are reproducible**
- **The release workflow refuses a tag that is not signed by an allowed key**
- **SBOM checksums identify the package source**
- **Release tags are signed.**

### Changed

- **The macOS runners are pinned to `macos-26`**
- **"Not a regular file" exits 4 in every command**
- **The same input can now report `renamed` where it reported `modified`**
- **New `old_path` member**

### Fixed

- **A client whose request the daemon refuses early still gets the refusal.**
- **The daemon caps the request bytes it reads across connections**
- **Daemon work stops at its client's read deadline**
- **A move header with no partner is refused, as Git refuses it**
- **A Git section naming two different files is reviewed as the rename `git apply` makes**
- **SIGTERM and SIGINT stop the daemon cleanly**
- and 9 more, in the full notes

Full notes: [docs/release-notes/0.7.0.md](docs/release-notes/0.7.0.md)

## [0.6.0] - 2026-11-02

### Dependencies

- **Dependabot's first version updates, applied together**
- **Dependabot groups each ecosystem's updates into one pull request**

### Added

- **An exact platform matrix.**
- **Daemon metrics for Prometheus, without a listener.**
- **`deep-diff-forge [--] <old> <new>` reviews two regular files.**
- and 1 more, in the full notes

### Changed

- **`chain`, `cluster` and `loom` are withdrawn, not planned.**
- **Dependabot watches the Python client.**
- **Whole-crate mutation of `core`, `patch`, `daemon` and `syntax`**
- **The Python client is a package.**
- **The daemon drains on stop.**
- **`daemon stop` returns once the daemon has exited**
- **Service units:**
- **The live review loop is tested.**
- **The evaluation corpus has 34 cases, and declared misses count.**

### Fixed

- **The arm64 Linux release build smokes its binary**
- **`review` on macOS reads keys through a reader that can wait on `/dev/tty`.**
- **Go raw strings no longer hide the code after them from evidence.**
- **Every Unicode default-ignorable code point is an `invisible` hazard and is escaped**
- **A checked-out submodule's own filter no longer runs during working-tree acquisition.**
- **CRLF patch headers name the paths Git applies.**
- and 46 more, in the full notes

### Security

- **CodeQL is a required check**
- **A filter in a repository's `config.worktree` no longer runs during Git acquisition.**
- **A repository filter whose name is not UTF-8 refuses the acquisition.**
- **The caller's `GIT_CONFIG_COUNT`/`KEY`/`VALUE` configuration is kept.**
- **Every release binary is run before it ships.**
- **The crates.io job fails closed.**
- **Release tags are immutable.**
- **Code scanning.**
- **Pinned actions are enforced.**

### Tests

Tests, corpora and the review record are in the full notes.

### Documentation

- **Production readiness rescored at `main` `1eaba47`**
- **The docs match `main` and say what is shipped, planned, withdrawn or on hold**

Full notes: [docs/release-notes/0.6.0.md](docs/release-notes/0.6.0.md)

## [0.5.0] - 2026-10-31

### Added

- **`deep-diff-forge bench [--quick] [--workload NAME]... [--json]`**
- **`deep-diff-forge eval [--corpus DIR] [--json]`**
- **Daemon metrics.**
- **`daemon start --log-format json`:**
- **Verified agent findings.**
- **Code evidence in risk ranking.**
- and 4 more, in the full notes

### Changed

- **Daemon protocol 2.**
- **Breaking (graph crate):**
- and 3 more, in the full notes

### Fixed

- 1 entry, in the full notes

### Security

- **Quoting code no longer hides it from code evidence**
- **The scanner errs toward code.**
- **A JS `child_process` import is `process_spawn` evidence.**
- and 1 more, in the full notes

Full notes: [docs/release-notes/0.5.0.md](docs/release-notes/0.5.0.md)

## [0.4.0] - 2026-10-30

### Added

- **`deploy doctor [--json] [--socket PATH]`**
- and 2 more, in the full notes

### Security

- **The daemon serves its own user only**

### Fixed

- **The daemon runs requests in parallel**
- **Two `daemon start`s can no longer race**
- and 1 more, in the full notes

### Changed

- **Breaking (daemon crate):**
- **macOS tests run on every pull request**
- **A weekly campaign**
- and 4 more, in the full notes

Full notes: [docs/release-notes/0.4.0.md](docs/release-notes/0.4.0.md)

## [0.3.0] - 2026-10-29

Tagged `v0.3.0` on 2026-10-29 by owner decision. 0.3.0 is minor rather than patch because the
library APIs changed incompatibly since 0.2.1 (see *Changed*). 0.2.1 was never tagged, so this
release follows v0.2.0. The workspace reached 0.3.0 on 2026-10-28; the expert-circle, bug-hunt
and owner-decision entries below landed before the tag. crates.io publication stays on hold.

### Changed (owner decisions on the bug hunt, 2026-10-29)

- **A rename or copy that Git refuses is refused**
- **Copies are `copied`**
- **A mode-only change is `mode_changed`**
- **Symlinks and new executables are risk signals**
- **The hostile daemon soak sends oversize and flood traffic**
- and 1 more, in the full notes

### Fixed (expert circle, 2026-10-28)

- **A rewritten comment is never "formatting only".**
- **A path can lower a file's risk but never zero it.**
- **`highlight` prints line breaks.**
- **Parse errors quote at most 120 bytes of the offending line**
- **Every command answers `--help`**

### Security (bug hunt, 2026-10-28)

- **Patch truth: lines Git ignores are refused, not read as headers**
- **Input Git refuses as corrupt is refused**
- **Repository filters never run.**
- **`review.change` refuses foreign Git directories.**
- **`git format-patch` output is read.**
- **Hazards cover the invisible-payload classes.**
- and 19 more, in the full notes

### Added (agent batteries)

- **`agent setup --client claude|codex [--allow-repo PATH]...`**
- **`agent doctor [--json]`**
- **Hazards**
- **`rpc` and the daemon name the framing**

### Added (agent-first core)

- **Hunk addresses in `review.v0`**
- **Budgeted, sealed views.**
- **`review.change` and MCP `review_change`: review straight from Git.**
- **Git subprocesses have a time limit**

### Added

- **`diff.rank`**
- **MCP tools declare an `outputSchema`,**

### Changed

- **MCP `rank_patch` returns the sealed `rank.v0` object**

### Added (documentation and community)

- **The README is rewritten from the code.**
- **An open call for contributors:**
- **Every document under `docs/`, plus `SECURITY.md`, `EVIDENCE.md` and `NOTES.md`,**

### Fixed (found while checking the docs against the code)

- **`review --cmd` refuses an unknown command name**
- **`--parallel` refuses anything but `serial`, `auto` or a count from 1**
- **`analyze` with the wrong number of paths, or an option, exits 2**
- **`deploy release` no longer claims a GitHub release for an untagged version.**
- **A tag release runs the macOS tests.**
- **`just release-tag` pushes the tag to `origin`.**
- and 1 more, in the full notes

### Added

- **Every exit code names what to do next.**
- **A release dry run.**
- **macOS tests.**
- **Deployment resource requirements**
- **The review JSON marks the line that was decoded lossily**

### Added (slice C: SDKs)

- **Rust: the `deep-diff-forge-sdk` crate.**
- **`deep-diff-forge rpc`:**
- **Python client**

### Added (slice D: MCP server)

- 1 entry, in the full notes

### Added (slice B: daemon parity, protocol 1)

- **Daemon `diff.compute`**
- **`engine.methods`**

### Added (slice A: CLI contract v1)

- 5 entries, in the full notes

### Changed

- **The crates.io hold is a condition, not a missing secret.**
- **`rank_patch` is bounded like `review_patch`.**
- **GitHub and GitLab `main` share one history again**
- **One parse → guard → seal path**
- **The council grades only a verified manifest**
- **A learning receipt cannot carry a path**
- and 15 more, in the full notes

### Fixed

- **Patch parsing is linear in metadata lines.**
- **A second `daemon start` never replaces a live daemon's socket.**
- **MCP `review_patch` returns a bounded summary for large reviews**
- **Input sealing**
- **`--require-files`**
- **`--require-hunks`**
- and 43 more, in the full notes

### Security

- **No text change can hide behind a binary patch body.**
- **Ids become paths only as checked segments**
- **MCP marks patch text as untrusted for the consuming model**
- **One hardened Git door**
- and 2 more, in the full notes

Full notes: [docs/release-notes/0.3.0.md](docs/release-notes/0.3.0.md)

## [0.2.1] - 2026-09-04

### Security

- 4 entries, in the full notes

Full notes: [docs/release-notes/0.2.1.md](docs/release-notes/0.2.1.md)

## [0.2.0] - 2026-06-23

The **L9 (Learning)** layer plus the first crates.io-publishable cut, a
supply-chain & fuzz hardening pass, and the interactive **review cockpit** (ranked
tree, multi-file diff, inline agent notes, command palette, colour themes, mouse)
— 12 crates, 927 tests, zero `unsafe`, supply-chain-gated. `cargo publish
--dry-run` is clean across the workspace; the crates.io upload itself remains
token-gated.

### Added

- **Interactive review cockpit**
- **Command palette**
- **`deploy release`**
- **Syntax highlighting**
- **Structural diff**
- **Learning (L9)**

### Changed

- **Daemon socket handling**
- **Publishable-workspace manifests**

### Fixed

- **Patch truth: malformed/truncated hunks are now rejected**
- **Broken-pipe handling**

### Security

- **Trojan Source defence (CVE-2021-42574)**
- **Terminal/ANSI-escape injection**
- **Memory-exhaustion DoS**
- **Fail-closed trust**
- **Supply chain**
- **GitHub Actions pinned to commit SHAs**
- and 11 more, in the full notes

Full notes: [docs/release-notes/0.2.0.md](docs/release-notes/0.2.0.md)

## [0.1.0] - 2026-06-22

First tagged release. The complete diff/review engine (layers L1–L7,
Patch→Daemon) at deployment maturity **L8 (Release)** — 11 crates, 568 tests,
zero `unsafe`, supply-chain-gated.

### Added

- **Patch (L1)**
- **Projection (L2)**
- **Pipeline (L3)**
- **Semantic (L4)**
- **Review (L5)**
- **Cluster (L6)**
- **Daemon (L7)**
- **Deployment surface**

### Engineering

- 4 entries, in the full notes

### Not yet released

- **crates.io publication**

Full notes: [docs/release-notes/0.1.0.md](docs/release-notes/0.1.0.md)

[Unreleased]: https://gitlab.com/lukeomahoney/deep-diff-forge/-/compare/v0.9.0...main
[0.9.0]: https://gitlab.com/lukeomahoney/deep-diff-forge/-/tags/v0.9.0
[0.8.4]: https://gitlab.com/lukeomahoney/deep-diff-forge/-/tags/v0.8.4
[0.8.3]: https://github.com/Louranicas/deep-diff-forge/releases/tag/v0.8.3
[0.8.2]: https://github.com/Louranicas/deep-diff-forge/releases/tag/v0.8.2
[0.8.1]: https://github.com/Louranicas/deep-diff-forge/releases/tag/v0.8.1
[0.8.0]: https://github.com/Louranicas/deep-diff-forge/releases/tag/v0.8.0
[0.7.0]: https://github.com/Louranicas/deep-diff-forge/releases/tag/v0.7.0
[0.6.0]: https://github.com/Louranicas/deep-diff-forge/releases/tag/v0.6.0
[0.5.0]: https://github.com/Louranicas/deep-diff-forge/releases/tag/v0.5.0
[0.4.0]: https://github.com/Louranicas/deep-diff-forge/releases/tag/v0.4.0
[0.3.0]: https://github.com/Louranicas/deep-diff-forge/releases/tag/v0.3.0
[0.2.1]: https://github.com/Louranicas/deep-diff-forge/releases/tag/v0.2.1
[0.2.0]: https://github.com/Louranicas/deep-diff-forge/releases/tag/v0.2.0
[0.1.0]: https://github.com/Louranicas/deep-diff-forge/releases/tag/v0.1.0
