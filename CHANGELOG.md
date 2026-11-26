# Changelog

All notable changes to Deep-Diff-Forge are documented here. The format is based
on [Keep a Changelog](https://keepachangelog.com/), and the project follows
[Semantic Versioning](https://semver.org/).

## [Unreleased]

- The agent hooks take a session's lock wherever they keep its state. In 0.11.0 a hook that ran
  while the hooks' state directory did not exist yet took no lock when a directory above it held
  a `.git` (a home directory that is a Git repository, such as a dotfiles repository; an empty
  `.git`; the `.git` file of a linked working tree), and wrote the state there all the same: the
  first hooks on an account unless `agent doctor --gate` ran first, the first hooks after the
  dotfiles repository's `git clean -xfd` removed the directory, and every session of an account
  whose home is made fresh per run (CI jobs, sandboxes, test harnesses). Two first stops of one
  session then both judged and both wrote the state, and the stop the client makes after a block
  blocked again (28 of 46 measured races). Those hooks now take the lock as every other does:
  one of two first stops judges with the state and the other judges the tree without it, as
  where no `.git` stands above. A session's state is written only by a hook that holds the
  session's lock (or where `flock` fails for another cause than another holder, such as a file
  system with no locks); a lock file or state directory that is not there for a moment when it
  is opened (two first stops making it at once, as seen on macOS) is looked for again while the
  wait lasts, and one that stays gone ends the wait with that cause, not as another hook's lock;
  a hook whose lock file cannot be opened says that its state could not
  be saved, and a stop then says that cause first, before the sentences it says of a session
  with no record of its start (which name this stop as where the session's hooks first ran,
  may call the session's own commit "not this session's", and no longer tell it to install
  the session-start hook). The state is written through the directory the lock was taken in,
  held open, and only with the token of that session's lock: a state directory moved away and
  replaced while a hook holds the lock gets nothing written into the replacement, and the hook
  says its state could not be saved (0.11.0 wrote by the directory's name, so the state went
  into the replacement while the lock stayed in the directory moved away; a process of the
  same user, the agent's included, could do it). The lock may be taken through a symbolic link
  at the state directory's name, as in 0.11.0; the state is never written through one. Stated,
  as in 0.11.0 (measured: the same probe gives the same result on the published binary): a
  process of the same user that unlinks or replaces a held session-lock file can start a
  second writer for that session; the lock excludes hooks that behave, not a hostile same-user
  process; nothing it does passes a commit. A flock refusal other than another holder's (a file
  system without locks, any other errno) lets the hook write without the lock, as in 0.11.0. A
  state directory inside the session's project is refused as before; the hooks may now leave empty directories there, which Git does not record.
  The five hook tests that `crates/deep-diff-forge-cli/tests/LOAD_SENSITIVE` named for this cause
  are off it: they pass alone with a `.git` above the temporary directory.
- Hooks: where the hooks' state directory lies inside a Git working tree (a home directory that
  is a dotfiles repository, or another working tree of the project), a `.gitignore` in it keeps
  the hooks' key and session files out of that repository's `git status` and `git add --all`,
  and a stop no longer judges them as the agent's change (0.11.0 blocked on the key's random
  bytes, or held, with no change of the agent's, and gave the key to the judge). The file holds
  `*`; any hook that keeps a state in the directory while none is there makes it, written whole
  under a name of its own and then linked, so that no hook finds it part-written, and it is
  never written over or through a link; none is made where the directory is itself the top of a
  working tree. Where it cannot be made, the state is kept as before. Files a repository already
  tracks are still judged (untrack them). Stated:
  a file the agent puts into the hooks' state directory there is not judged while it is not
  staged, as under any `.gitignore` that ignores itself (a limit SECURITY.md already states);
  staged, it is judged. Known issue: a Git directory inside one of the project's working trees
  (`git init --separate-git-dir` into the tree) puts Git's own files, with the hooks' records
  and scratch, into every stop's patch, and every stop blocks; keep it outside the working
  trees.
- `agent doctor --gate` and a session's start: the link `policy_protects_itself` is ready only
  when a rule of the judge's verdict holds or fails the probe (a one-line edit of the policy
  file). In 0.11.0 any verdict that did not pass made the link ready, so a policy that does not
  protect its own file was reported as protecting it whenever the probe was held for claims the
  judge cannot verify (every policy with `require_verified`, forge-warden's starter policy among
  them) or failed for an expired waiver. Such a policy now reads `FAIL … does not pass, but no
  rule holds or fails it`, the gate exits 7 for it, and a session's start names the link as
  weak. A protected policy reads `ok … is held or fails under a rule` (the words "under a rule"
  are new). The gate reads the policy at `HEAD`, a session's start the policy at the session's
  base; what a stop judges by is unchanged in this entry.
- The stop hook judges every unit under two policies where their files differ: the policy at
  the session's base, as before, and the project's accepted policy, the policy file at the
  commit the project's ledger first recorded; the stricter answer stands, and a unit that one
  of the two could not judge is never a pass (UNJUDGED unless the other fails or holds it). After
  this, where the project's ledger first recorded a commit that has the policy file, a policy edit
  that is merely committed does not judge anything, in the session that makes it or after it: in 0.11.0 a
  commit that a session left without its stop, deleting the rule against secrets and the
  policy's protection of its own file, judged itself and the next fresh session's own work, and
  a key passed and verified. A `pass` record takes a commit out of the frontier only when it is
  sealed to the accepted policy, so commits 0.11.0 passed through that hole are judged again,
  and so is every earlier pass of a project whose policy changed since its ledger first recorded
  it (a branch that verified can stop verifying). Where the ledger names no accepted policy
  the stop is 0.11.0's; where it names one that cannot be read (its commit gone, a shallow clone
  without the history) a unit the policy at the session's base does not fail or hold is
  UNJUDGED. The limit that remains: a deliberate act by a process of the same user (a rewritten
  or removed ledger, a changed settings file together with a removed ledger) can still change
  what the hooks judge by; the pull request's judge is the boundary.
- `agent policy-accept --commit C [--policy NAME] [--judge PATH] [--yes-from-pipe]`: the one way
  a project's accepted policy changes (with the two policies above, a project that updates its
  ranker's pin would otherwise be held for ever). To change a policy now: commit the new policy,
  then a person runs the command, reads the old and the new digest, the whole diff and the change
  of the producer pins, and types the new digest at a terminal; one acceptance record is appended
  to the project's new policy record, `.git/deep-diff-forge/policy.jsonl` beside the ledger, and
  the hooks apply it as a compare-and-swap (its `from` is the accepted digest then, its commit an
  ancestor of `HEAD`, its `to` the bytes there). The same file binds, on first use, the policy name
  the hooks are given: hooks given another name pass nothing. What the convention is not: it is
  not authentication; any process running as this user can run the command or write the file, and
  a record written by hand that applies takes effect. The pull request's judge under the target
  branch's policy is the authority. A stop reads the ledger and the policy record under one
  project lock and compares them again before its append: a stop whose accepted policy changed
  while it judged records no `pass`. The ledger's form and version are unchanged, and 0.11.0
  ignores the new file. `agent verify-branch` now counts a pass as a stop's frontier does: after
  an upgrade, commits that passed under a policy that was never the accepted one stop verifying
  until a stop judges them again or the policy is accepted (the published 0.11.0 counts every
  pass). `agent doctor --gate` has a new link, `policy_accepted`. The meaning of
  `chain.policy_missing` is widened to an unbound policy name, a policy record or a ledger that
  cannot be read whole, and an accepted policy that changed while the stop judged. A ledger that
  exists and cannot be read whole (a session's own commits and stops can grow it past the 64 MiB
  its readers read) now leaves the accepted policy unknown: no change passes until a person deals
  with the file (0.11.0 judged under the policy at the session's base alone there); a commit
  already recorded as waiting for its turn is no longer recorded again at every stop. Acceptances
  of two branches that meet in a merge pass nothing until the owner accepts again.
- Tests only: no scanner changed. The tests that hold the line scanners to linear time bounded
  one size in absolute seconds of a debug build, so they failed on a slower runner while the
  scan was linear (`the_import_rule_is_linear_on_long_lines` on GitHub's Linux runner for
  0.11.0). Six of them now bound how a scan's cost grows when its input doubles (under 3 times,
  on the thread's CPU time, at sizes that double from small to large, so that a quadratic scan
  fails while it is still fast), and keep an absolute deadline only against a hang:
  `one_long_word_ranks_in_under_a_second` is renamed `one_long_word_ranks_in_linear_time`, and
  the long-line test of every trigger ranks each line at sizes up to 1 MiB instead of once at 2
  MiB, at the same cost. The seventh named with them, `the_backstop_scales_with_source_size`, is
  pure arithmetic on the parse deadline, reads no clock, and is unchanged. The test helper
  `within_deadline` (six copies) now fails with the work's own panic message instead of
  "Disconnected". `crates/deep-diff-forge-cli/tests/LOAD_SENSITIVE` names hook tests that fail
  on a loaded host (five measured, two observed once and marked so), and a test fails when a
  name on it is no test.
- Tests and the gate only: no product change. The gate's step `test` (`scripts/ci_local.sh` and
  CI's `gate` job) is `scripts/test_step.sh`: `cargo test --workspace --locked --no-fail-fast`, so
  that one failing test binary no longer hides the binaries after it, with the tests
  `LOAD_SENSITIVE` names skipped, then those alone on one thread; the step fails unless each
  listed test ran once. Five of the seven names were measured to fail not under load but when a
  `.git` stands above the hooks' state directory while a session's first hook runs: that hook
  then takes no lock (`session_lock` makes no state directory inside a Git working tree) and
  writes the state without one. On the machine of the gates an empty `.git` in the home directory
  stood for 1 to 32 s at a time while another agent's sandboxed commands ran. A failing hook test
  now says so under its panic (`note (not of the code under test): …`), and the test helper that
  starts a session fails at a start that recorded nothing, with its words and seconds, instead of
  later on a state that was never written.
- Tests: the suite no longer assumes Linux or a short temp directory. A test that needs another
  process to hold a lock takes it with `python3`'s `fcntl.flock` (one holder, in the testkit),
  not `flock(1)`, which macOS lacks; the stand-in judge of `agent check`'s tests falls back to
  `shasum -a 256`; names that are not UTF-8 are also put in Git's objects alone, and the case on
  disk says it is skipped where the file system refuses it; paths are compared as messages show
  them (cut in the middle, canonical); `scripts/campaign_local.sh` runs on bash 3.2.
  `scripts/long_tmpdir.sh` makes a temp directory as long and as indirect as macOS's (a link of
  49 characters to a directory of 57), so these failures can be seen on Linux. The 0.11.0 CI run
  on macOS failed 22 tests; six of the failures that come back on Linux this way are of the
  product (a message cut at the wrong place under a long path) and stay red until it is fixed.
- The local gate (`scripts/ci_local.sh`) prints each step's time and what it skipped in one form
  (`PASS test (500.2 s; skipped: 8 tests ignored)`), and writes a JSON receipt of the gate: the
  commit, and each step's name, status, seconds and skip reason (its path is printed;
  `DDF_GATE_RECEIPT` names another). The contract matrix prints `real rows: N run, M skipped`,
  which the step's line carries, and one line `skipped: …` that counts the rows it skipped by
  what was not given.
- Tests only: the patch parser's refusal of a symlink Git cannot write is now held, against a
  real `git apply`, for a link changed in place whose mode is on its `index` line alone (a
  target over 4,095 bytes, a target with a NUL) and at the bound itself (4,095 bytes read,
  4,096 refused); `release_workflow` holds `release.yml`'s triggers to the sentence of
  `SECURITY.md` (a push of `v*` tags and `workflow_dispatch`, nothing else).
- Tests, and one check of the gate's: no product change. Security sentences that no test held
  now have one that turns red when their code goes: a mounted host path that is not UTF-8; a
  learning store file or directory replaced between its check and its open, and a link that
  appears where a new store file is created (the directory and creation cases through a real
  race); the parser version's form and both token bounds; hostile receipt metadata at the record
  and read doors; no network surface in the learning crate or the `learn` commands; the oversized
  request's connection closing; and the daemon's 30 s write timeouts. The CI wrapper
  `scripts/security/review_policy_check.py` now also fails unless `fuzz/Cargo.toml` forbids
  `unsafe_code` (an `allow` there passed); the checker the review crate embeds is unchanged.
- Tests only: no product change. Agent-hook sentences of `SECURITY.md` whose cited tests stayed
  green when their code went (security mapping U1, lane T3a) now have a test that a plant of that
  code turns red by a named assertion: each carrier of the commits a session's start records; each
  seal of a verdict; no configuration of the system, of a parent Git or of the user's attributes
  file shaping the judge's patch, and only an absolute ignore file of the user's; a hook's command
  named by its own path quoted for the shell; a stop silent only for Git's own C-locale words with
  no project known; a rule's name escaped where rules are named alone; the advice for a gone tree;
  a tree listed again forgotten from inside it; a path Git names as another tree; a project moved
  behind a link; and the waits `SECURITY.md` names (3 s for the session's lock, 8 s for the helper
  that hashes the rankers).
- Tests only: no product change. The daemon's security sentences whose cited tests did not turn
  red when their code was mutated now have tests that do, each on a named assertion: the accept
  loop's peer check; the documented `0700` and `0600` as numbers, an existing managed directory
  made `0700`, another user's directory refused as such, and a runtime directory swapped after
  its check never secured through the swap (a real race, counted); an existing `--socket` parent
  never chmodded; shutdown removing its own socket and no other; each JSON-RPC envelope rule by
  its own message; the 80 MiB caps with the newline counted; each line read armed with the time
  left of one deadline that a late byte does not renew; the eight data and two control slots and
  what each answers; one engine across connections; the session table as the one lock, held for
  bookkeeping (read from the source); and a panic answered on a connection that keeps serving.
  Tests that unwrapped a result before asserting it now assert it.
- Tests only: no product change. The agent-hook sentences of `SECURITY.md` that cited no test (U1,
  lane T3d) are each mapped to a test that a plant of the code they describe turns red by a named
  assertion, most of them tests that were there; new or strengthened: a stop from a submodule's Git
  directory, failures that stand giving way in a full message, a commit only a branch past 5,000
  others names, a commit after one whose patch is refused, a link at the policy's name, keyed
  digests, and stops that must not be silent asserted so first.
- What an agent or a person is told keeps its meaning under a long path: where a message is bounded, the
  paths the hook quoted give way in their middle, never the words that say what happened. Under a
  repository path of about 100 characters or more (any macOS temp directory) the line that counts what a
  stop does not judge cut a working tree's name at its end; under a directory of 211 characters 0.11.0
  said `1 thing in this project is not judged: the working tree "/var/mnt/storage/ddf-tmp/w111P3/n121/LLL…LLL/old/row5_copy_repaired-kjc9f1dk/copy" (remembered a… (a failure stands there).`
  and this build says `1 thing in this project is not judged: the working tree "/var/mnt/storage/ddf-tmp/w111P3/n121/LLLLLL…LLLLLLL/new/row5_copy_repaired-9sh8mo3p/copy" (remembered at another directory, which stands) (a failure stands there).`
  (the runs of `L` abridged here). The same rule holds where a stop shows a reason (300 characters; 150 on
  the line of commits with no verdict): the reason of a tree moved with `mv` says what to do again
  (``(`git worktree repair` in a tree moved with `mv` records
  where it is): …``); a reason whose own words do not fit is still cut at its end with "…". The paths
  that give way are the ones the hook wrote, known as it wrote them, never read back from quotes: a
  path ending in `\` or a quote in a judge's or Git's words moves nothing that is cut, with two
  display-only exceptions that change no decision and never exceed the bound: a path in a judge's or
  Git's words that is, byte for byte, a path the hook showed in the same process is cut like the
  hook's own; and a path the hook showed that equals a stretch of the product's own words between
  two quotes could make the cut fall inside those words (at a stop only policy file names reach
  that, never repository content). A judge's own
  message past 300 characters ends with "…" (it was cut there without a mark). Limits: a judge's or
  Git's words, with any path in them, do not give way to the hook's words and are cut at the end with
  them; a unit's name with the judge's words (240 characters) is cut at its end and can lose its count
  "(+N more)"; reasons whose own words pass 300 at any path length lose their last words (a tree the
  session remembers at another directory: its remedy's last words; a pair Git names that the records
  do not make: the closing clause after the hint), and one commit's reason on the line of commits with
  no verdict (a shallow clone without the commit's parent) passes 150. A council or
  tuicr error longer than the 200 bytes the command line shows keeps its first and last 100 bytes, so a
  refused import says "nothing was appended" again
  (`deep_diff_forge_core::display_safe_bounded_ends` is new). `daemon path`, `daemon start` and `deploy
  doctor --socket` take the longest socket path from the system (107 bytes on Linux, as before; 103 on
  macOS, where 0.11.0 printed a path of 104 to 107 bytes that could not be bound), and the doctor's
  `socket` check fails on a longer one. The refusal of a symlink with an empty target says what each
  system does (`… is refused: an empty symlink target, which git apply on Linux fails to write after
  removing files (git apply on macOS writes the link)`); the refusal itself is unchanged, and
  `SECURITY.md`'s table of what changes `git apply`'s writes has the platform's row. The local gate's
  test step runs under a temp directory as long and as indirect as macOS's (`scripts/long_tmpdir.sh`).
  What is judged and decided is unchanged.
- CI only: no product change (the first macOS runs of the 0.11.1 branch, PR #131). The `macOS
  tests` job runs the gate's test step, `scripts/test_step.sh`, instead of one plain
  `cargo test --workspace --locked --no-fail-fast`, and its limit is 90 minutes instead of 60 (the
  runs took 39 and 45 minutes in one pass; the second pass, measured from the runs' logs, adds at
  most 17). `crates/deep-diff-forge-cli/tests/LOAD_SENSITIVE` names four timing tests of the graph
  crate that failed there only while the whole suite ran at once, each with its failing ratio and
  run, and so names unit tests of a library by their path (`evidence::tests::…`); the test that
  holds the list reads those paths. `scripts/diff_mutation.sh` no longer passes `--timeout 300`,
  which also bounds cargo-mutants' unmutated baseline: the workspace's tests take about 35 minutes
  on CI's runner, so every diff-mutation shard failed on its baseline before testing a mutant. The
  timeout is twice the baseline, at least 2400 s. Stated limit: a shard's 300 minutes hold the
  baseline and about seven mutants that run the whole suite, so a change of hundreds of mutants a
  shard (PR #131: 214 to 223) still does not finish.
- Tests only: no product change (PR #131's fourth macOS run, 45792873322). The hook test
  `a_repository_with_more_working_trees_than_are_read_says_so` failed on the macos-26 runner also
  alone: its first session start read the reflogs of 256 linked working trees and ran past a
  start's 8 s (2.6 s on Linux). Its 255 trees are now Git's administrative entries with no
  directory at that start, and then get the directories and `.git` files that `git worktree add
  --no-checkout` makes; every start and stop of the test says what it said before, word for word,
  and the test takes about 7 s instead of 12 on Linux. It is off `LOAD_SENSITIVE`. Stated: that a
  start reads the history of 256 trees within its time is not tested, and on that runner it does
  not; that the test passes there is shown only by the next macOS run.
- CI only: no product change (PR #131's runs again). The required check `review controller
  mutation and fuzz evidence` mutates the review crate's `identifier` and `check_execution` with
  `--test-workspace false` and `--file` for their two files, instead of `--test-workspace true
  --timeout 300`. cargo-mutants times its baseline on the mutated package's tests alone, so each
  mutant ran the whole workspace (25 to 37 minutes on CI's runner since 2026-11-24) against a
  timeout that baseline never measured: every mutant timed out and the job was cancelled at its 75
  minutes. Its 22 mutants are now judged by the crate's own tests (all caught, 3 minutes locally).
  Stated: the 24 struct-field mutants of `journal.rs` that cargo-mutants' `--re` let into the job
  are no longer in it; 22 of them are not caught by the review crate's own tests. The `deploy
  gate-feature` job's limit is 60 minutes instead of 45 (its runs took 38.5 and 37.3).
- Tests only: no product change. More sentences of `SECURITY.md` whose tests stayed green when
  their code went now have tests that turn red: the learning store refuses a directory or a file
  of another user by its owner, makes a directory left `0755` into `0700`, refuses a store others
  can read at every door and leaves it as it was, refuses a linked directory as a link, and holds
  each of its three budgets at its own check; its identifier is FNV-1a with no key (published
  vectors, and a guess list finds the path); the patch parser reads one million lines and 64 MiB
  and refuses more, by the sentence's numbers, at the first line past the budget; lines Git skips
  after a binary body's hunks are read; and stdin over the budget is refused by its own reader,
  which stops reading one byte past it.
- Tests only: no workflow, script or manifest changed. SECURITY.md's supply-chain sentences that no test
  held now have one that turns red when their file changes: no advisory waiver; the tree-sitter pins and
  the TUI's ratatui features; every workflow's grants, checkout credentials, job time limits and
  toolchains; what Dependabot watches; both release runs' gates, the Rosetta and checksum steps; what a
  tag run attests; the `release` environment's one job; release.yml's tag check and crates.io token
  guard, run for real (an open gate without a token that exited 0 passed every test); the local
  release's command, smoke and signing; the release namespace and the three remotes; the pre-push hook
  without gitleaks; CI's full-history scan; and the SBOM's `created` (the gate ignores it).
- Tests only: the scans measured linear from 1 KiB to 16 MiB, so the four linear-time tests that
  failed on macOS (3.00 to 4.53 at one doubling, wall time) now bound the cost's growth between
  sizes 16 times apart by an exponent of 1.5; a quadratic plant in each scanner still fails them,
  and they failed 0 of 596 runs alone and under load where the old ones failed 10 (on wall time,
  as on macOS).

## [0.11.0] - 2026-11-25

**The agent-loop release.** 0.11.0 is built for an agent that works under Deep-Diff-Forge's
hooks. While it works it is told what its change carries, as items of a new document,
`deep-diff-forge.diagnostics.v0`: one diagnostic for each place a label stands, with the
repairs that answer it, printed by `--diagnostics`, served in pages by the daemon's engine
protocol 3 and by MCP, and read and checked by both SDKs. When it stops, forge-warden's verdict
decides, and the stop judges more than before: the commits an ended session left and the
commits merged between sessions (the project's frontier, kept in a ledger of stops under the
repository's Git directory), the working tree even when the session's state cannot be used, and
each commit with one parent in the judge's bound mode, so that such a commit can pass under a
policy that requires verified claims. Every exit 7 has one meaning and names its reason class;
`agent check` judges on demand what a stop judges, and `agent verify-branch` reads the ledger.
It fixes one miss of released versions with an advisory: `eval` and `agent doctor --gate`
exited 0 when the reader of their output had gone (DDF-2026-011). It closes the miss
DDF-2026-006 declared: a source file that a stray byte makes binary by content is read.

Know four things before installing. The hooks' state is schema v13: the published 0.9.0 and
0.10.0 refuse a session this build has written (UNJUDGED, its files kept), so do not run both
on one session. The Rust readers of four integrity labels and of `check_suppressed` and
`build_hook` read more, and a listed exception to the floor removes two readings (`#[allow(` in
a Rust comment or string, and `r#unsafe`), so a forge-warden policy that forbids these labels
holds more changes and a few others no longer. Under a policy that requires verified claims
only a commit with one parent can pass, and `agent doctor --gate` exits 7 there. The Rust
library API changes where a type gains a variant or a public field. And one limit the review of
this release left stated, under [Known issues](docs/release-notes/0.11.0.md#known-issues): a
commit that a session leaves without a stop can pass the next session's stop under its own edit
of the policy, so the judge on the pull request stays the boundary. Each change on purpose is
under [Changed on purpose from 0.10.0](docs/release-notes/0.11.0.md#changed-on-purpose-from-0100); install it in the order
under [Cutover with forge-warden](docs/release-notes/0.11.0.md#cutover-with-forge-warden), and keep the published 0.10.0
binary as a ranker until every policy admits 0.11.0.

Published as the signed tag `v0.11.0` on GitLab, the place of release, with mirrors on Gitea and
Forgejo and a Linux x86_64 binary built on the owner's machine, with its `RELEASE-MANIFEST.json`
signed beside `SHA256SUMS`; GitHub mirrors the release page and crates.io carries the crates from
the tag, as for 0.10.0.

### Security

- **`eval` and `agent doctor` name their refusal on stderr and exit 7 when the reader of stdout has gone**
- **A source file that a stray byte makes binary is read**
- **MCP writes hidden characters as escapes**
- **A patch that does not parse is refused without its text**
- **Nothing the hooks keep holds text from the patch, the judge or Git**
- **The marks an advice hook leaves under `.git/deep-diff-forge/`**

### Added

- **`deep-diff-forge.diagnostics.v0`**
- **`--diagnostics`**
- **`--rank --fail-on LIST`**
- **The vocabulary as data with ids**
- **`agent check`**
- **The project's ledger**
- **`agent verify-branch BASE..HEAD`**
- **The judged frontier belongs to the project**
- and 11 more, in the full notes

### Changed

- **Exit 7 has one meaning, with a registry of reason classes**
- **Stop hook, changed on purpose (owner decision OD-J): a stop the agent itself can make unjudgeable no longer goes through at no cost**
- **Stop hook, changed on purpose: a stop whose session state cannot be used**
- **Stop hook, changed on purpose: a commit can pass under a policy that requires verified claims**
- **Hook state**
- **The advice after a tool call is an excerpt of the diagnostics document**
- **`check_suppressed` and `build_hook` read more**
- **Changed, by owner decision OD-E**
- and 4 more, in the full notes

### Fixed

- **A hook's scratch directory is owned by a lock**
- **`agent doctor --gate` and `agent check` compare `--ranker-dir`**
- **A session's start says its last sentence**
- **`--help` and the reference**

Full notes: [docs/release-notes/0.11.0.md](docs/release-notes/0.11.0.md)

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
