# Changelog

All notable changes to Deep-Diff-Forge are documented here. The format is based
on [Keep a Changelog](https://keepachangelog.com/), and the project follows
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Security

- **A `diff --git` section is read only if its headers are a shape Git's own diff writes**
  (plan of record S3.1 to S3.3). Five independent assessments in a row, of v0.8.0 to v0.8.3,
  each found one more header shape that DDF read one way and `git apply` wrote another, and
  each was fixed case by case. The parser now holds a list of the 42 header shapes
  `builtin_diff` and `fill_metainfo` write (Git 2.56's `diff.c`). The 42 are:
  - a file created or deleted;
  - a mode change;
  - a rename or copy, with or without an edit;
  - a rewrite under `-B`;
  - a text body, a binary body, an `index` line alone (a rename, copy or mode change whose
    edits `-w`, `-b`, `-I` or a textconv driver hides), or none.

  A section whose headers are not one of them is refused (exit 4, new error
  `HeaderNotWrittenByGit`, naming the line and why). A `similarity index`, `dissimilarity index`
  or `index` line may be left out, since none of them changes what Git writes. Each line must
  also be in the form Git writes it:
  - a mode of `100644`, `100755`, `120000` or `160000`;
  - `index` ids of hex digits;
  - a whole percentage.

  Every refusal 0.8.4 made stays, with its reason.
  - *What is promised (C1), what a patch alone can settle:* for every patch DDF accepts, a
    plain `git apply` in Git's default configuration either refuses the whole patch while
    checking it and writes nothing, or writes exactly the paths, types and modes the review
    shows and, for each hunk, removes and adds exactly the lines (as Git stores them) the review
    shows, at the place Git's own anchor rule gives: a hunk with no trailing context at the end
    of the file, one at old line 0 or 1 at its start (with both, the whole file), any other by
    Git's search outward from the new-side line its header names, over lines no earlier hunk of
    the same section wrote (a later section for the same path is applied to the earlier
    section's result, where no line is skipped; the review does not say which of several equal
    places Git takes). A
    header's line numbers and `@@` text are the patch's claims, true on the tree Git made the
    patch from. A hand-written patch can name one copy of some lines while Git changes another
    (a last hunk with leading context only, on a file whose end repeats its lines), so the
    review a person reads states the anchor: the inline and side-by-side layouts and the TUI's
    hunk header say "Git places this hunk at the end of the file" (or at the start, or on the
    whole file). No JSON member is added: `review.v0` and `review-hunks.v0` carry the lines the
    anchor follows from (rule in `docs/API_AND_IPC.md`), and the ranges are documented, and
    described in the MCP tools and daemon methods that serve them with both anchors, as the
    patch's claim (the JSON carries no mark). Checking the place against the tree,
    when DDF has one, is not built. The test oracle places each hunk by Git's anchors, and the
    generated differential writes such hunks by hand (row `hunk-anchor`). Two kinds of patch
    are refused (the paragraph after the list says which the promise needs):
    - a hunk Git places by an anchor its header does not set (`HunkPlacedElsewhere`): a hunk
      with no context past line 1, which plain `git apply` places at the end of the file, and
      a hunk after one with no trailing context. Plain `git apply` tries no other place
      (`apply_one_fragment`). `git diff -U0` writes such hunks; Git-written, and refused
      (`git apply --unidiff-zero` reads them). DDF's own `git diff` passes `-U3`, whatever
      `diff.context` says. A hunk with no context at line 1 is read: Git writes it only when it
      is the whole file, and else refuses the patch;
    - what `git apply` fails to write after it has removed what the patch deletes
      (`GitCannotWrite`): a symlink whose target is empty, holds a NUL, is longer than 4,095
      bytes or is in a binary body, and one path that is both a file and the directory of
      another (two paths the patch writes, or a copy's source the patch keeps and a path
      written under it or at its directory).

    The restated promise does not need the first of these refusals: a hunk with no context
    past line 1 would be stated as placed at the end of the file. DDF refuses such hunks
    (`HunkPlacedElsewhere`) because plain `git apply` places them at the end of the file
    whatever their author meant (`git diff -U0` output is meant for `git apply
    --unidiff-zero`). `GitCannotWrite` is what the promise's second outcome needs.

    One hunk layout no Git writes is read: a creation whose hunks all start at line 0. Git
    writes the first hunk into the empty file and refuses the whole patch at the second, while
    checking it, so nothing is placed elsewhere; the reading 0.8.4 gave a created file's later
    hunks stays reachable (`CONFORMANCE.md`). A creation's later hunk at any other line is
    refused.

    "Git writes it" is not "`git apply` accepts it", and C1 allows any whole refusal. The
    generated differential counts three presentations Git's own diff writes, which DDF reads
    as 0.8.4 did and `git apply` refuses whole:
    - a binary marker with abbreviated ids (a plain `git diff` of a binary file);
    - the inner section of a `git diff --submodule=diff` envelope;
    - `diff --git` halves Git cannot split, as `git diff --no-index` writes for two binary
      files.

    Each is tested against real Git output. The list is not exhaustive: `-D`, textconv and
    `-w` output can also be refused whole. Outside the promise:
    - `git apply --reject`;
    - a failure while writing that the patch does not determine (permissions, a full disk, a
      file or a directory already in the tree where the patch writes), which can leave the tree
      half-written;
    - hunks out of order, overlapping, or with a new-side start the earlier counts do not give:
      read; Git either places each hunk by its own rule or refuses the whole patch and writes
      nothing, and the review states neither which;
    - another tree, where the place Git's rule finds (an offset, or the end of the file) can be
      another copy of the hunk's lines than the one its header names;
    - attributes in the target tree (`eol`, `ident`, `working-tree-encoding`);
    - a non-default configuration: with `core.fileMode=false`, Git writes the mode an `index`
      line names, so leaving that line out changes the mode written (`CONFORMANCE.md`).
  - *Measured:* every commit was diffed as `git diff`, `git diff -M -C -B`,
    `git diff --binary`, `git format-patch --stdout` and `git log -p --first-parent`, over:
    - this repository: 892 commits on all refs, 4,457 patches, 57,038 `diff --git` sections;
    - forge-warden's `hub.git`: 772 commits, 3,857 patches, 29,301 sections;
    - a scratch repository built to give every row, with nine more option sets: 55 commits,
      866 patches, 1,071 sections, all 37 shapes;
    - 646 windows of 2 to 5 commits from both histories;
    - the scratch repository again under `git diff -U0` and `-U0 -M -C` (108 patches), and
      under `-w`, `-b` and `--ignore-blank-lines` (162 patches).

    Against 0.8.4, 25 patches are newly refused, all `git diff -U0` output of the same five
    commits of the scratch repository and its earlier versions (`HunkPlacedElsewhere`); none is
    refused as `HeaderNotWrittenByGit` or `GitCannotWrite`. The 227 patches of an independent
    sweep that the first S3a build refused (`-w` and the like with a move or a mode change) are
    all read.
    The refusals that remain are 0.8.4's own: a `format-patch` message line that begins `-`,
    the byte budget, the line budget, a `-- ` signature after a binary body, and one path in
    several sections.
  - *Newly refused, each read by Git and never written by it:*
    - `Files … differ`, which 0.8.4 read as `Binary files … differ`;
    - a hunk with no `---`/`+++` before it;
    - `similarity index` with no move, and `dissimilarity index` beside one;
    - a second `copy to`, `rename to` or `index`, and a `rename to` or `copy to` with no
      `from` line;
    - headers out of Git's order (`index` before the mode lines, `new mode` before `old mode`,
      `copy from` before `similarity index`);
    - modes such as `100664` or `100600`, `index` ids that are not hex, and scores over 100
      or without `%`.

    Also newly refused, though Git writes them: `git diff -U0` hunks, which plain `git apply`
    places elsewhere, and the patches `git apply` fails to write (both above).
  - *Found while building it:* a binary marker that ends in a carriage return
    (`Binary files … differ\r`, `GIT binary patch\r`) is no marker to Git, which compares the
    line feed. After `old mode`/`new mode` and `index`, Git applied only the mode change,
    while v0.8.3 and 0.8.4 reviewed a binary change of the content (DDF showed more than Git
    wrote; nothing was hidden). It is refused now.
  - *Found by the generated differential:* a section whose `diff --git` halves name two paths
    (`diff --git a/x b/z`), with no `---`/`+++`, rename or copy header, gives Git no file name,
    and Git refuses it ("git diff header lacks filename information"). For a file created or
    deleted under such halves, which no Git writes, v0.8.3 and 0.8.4 took the rightmost ` b/`
    and reviewed a creation or deletion of `z`. Git writes nothing, so nothing was hidden.
    It is refused now (`InvalidPath`, as for a section that names no path).
    - *Stated limit:* any other section whose halves Git cannot split, with nothing else
      naming the files, is read as before, and Git refuses it whole. Such halves name two
      paths, or have a trailing space or tab, mixed quoting or a repeated ` b/`.
      `git diff --no-index` writes them for two binary files, or two files that differ only in
      mode, and DDF's `<old> <new>` mode reviews that output.
    - *On the 0.8.4 parser:* the first version of the generated differential (24,000
      patches) held 81 patches DDF read and Git refuses to read, all of this shape: 20
      creations or deletions, refused now, and 61 pairs. None of them is a patch Git writes
      otherwise than the review shows.
  - *Proved by applying:* `tests/generated_differential.rs` builds 24,000 patches from 64
    fixed seeds. Each is one to three sections Git writes, with up to three near-miss moves.
    Half the threads use SHA-1 repositories and half SHA-256. DDF reads each patch, and a real
    `git apply` applies it in a repository whose object store holds every blob an `index` line
    names (85 s on the build machine, beside other builds). Its sections include hunks with
    one line of context and with none, as `git diff -U1` and `-U0` write them, and hand-written
    last hunks with leading context only, one naming an earlier copy of its lines. The oracle
    places each hunk by Git's anchors and search. Figures of the run at the final commit: DDF
    read 8,448. Git applied 6,558 of those, each to exactly the tree the review describes, 723
    of them with a hand-written hunk Git anchors at the end. A binary section
    with a full id is held to the blob it names. Every other patch DDF read, Git refused whole,
    writing nothing:
    - 893 binary markers with abbreviated ids;
    - 44 `--no-index` pairs;
    - 953 others whose hunks or files do not fit the tree;
    - none of the `--submodule=diff` envelope, which the generator does not write; the test
      of real Git output covers it.

    302 patches with a `-U0` hunk Git places at the end of the file were refused, as designed.

    Any patch DDF read that Git refuses to read, outside the named presentations, fails the
    test, as does any that Git applies otherwise than the review shows.
    `DDF_DIFFERENTIAL_SEEDS=N` runs seeds 1 to N.

    The oracle does not prove three things: the bytes of a binary section whose id is
    abbreviated or whose new blob is not in the store, where Git places a hunk it finds at an
    offset on another tree, and a write Git fails at (the generator writes no symlink Git cannot
    write; `shapes_git_fails_to_write_are_refused` runs those against real Git).

    Parser changes that earlier rescores found, each put back in a scratch clone, make the test
    fail on the 64 seeds when the accept-list is turned off (measured on the generator of
    `3e55b72`):
    - `Files … differ` read as text again (G1): 57 failures, the first at seed 16;
    - a bare header's name carried over (F3): 596 failures, the first at seed 8;
    - `new mode` after `new file mode` accepted (F2): 6 failures, the first at seed 24.

    With the accept-list on, the list itself refuses the F3 and F2 shapes. G1 still fails the
    test (50 failures): `new file mode` and `index` followed by an unread line is a shape Git
    writes. A binary marker ending in a carriage return read again, with the list on, fails it
    26 times (the first at seed 16): the binary oracle sees the blob the review names where Git
    changed only the mode. The placement rule turned off (S3B-H-1) failed it 773 times (the
    first at seed 8) while the oracle placed hunks at their header's line. With the oracle
    placing them by Git's anchors (`cc7b82f` on), a `-U0` hunk read is placed at the end of
    the file, as the review then says, so the comparison no longer fails: the plant fails the
    conformance row `hunk-placement` and the differential's count of `-U0` hunks refused by
    design (0). An oracle that trusts a header's line (the end anchor turned off) fails the
    differential 358 times and the row `hunk-anchor`: Git changes the last copy of the
    hand-written hunk's lines. The empty-symlink rule
    turned off fails the conformance test, not the differential.
- **`CONFORMANCE.md`** (in the patch crate) is the table of what Git reads. It has one row for
  each header line `apply.c` recognises, and one for the traditional form. Each row says what
  Git sets from the line, which line wins when it is repeated, what Git refuses, whether Git's
  diff writes the line, and what DDF 0.8.4 did and does now, each cited to Git 2.56.1's source.
  Each row has a test (`tests/conformance.rs`) that runs its forms through DDF and through a
  real `git apply`, and a test fails if a row and a test do not match.

### Changed

- **New signal `diff_suppressed`** (+5, class security, since 0.10.0; S3.20,
  [DDF-2026-006](docs/advisories/DDF-2026-006.md)). A contract addition to `rank.v0`: a file whose
  change Git gave as binary and DDF read again as text carries the label, and its ranked item the
  member `"diff_suppressed": "attribute"` (a `diff` attribute unset or naming a driver for the
  path, by `git check-attr`) or `"content"` (Git's own content check). `git-acquisition.v0`
  marks the change with the same member. A consumer that does not know the label should hold.
  `schema signals`, `explain signal`, the TUI and the reference tables list it. In the library,
  `RiskSignal` has the variant `DiffSuppressed` (the enum is not `#[non_exhaustive]`, so an
  outside `match` needs the arm), `PatchTwin`, `GitChange` and `RankedFile` a field
  `diff_suppressed`, and `deep_diff_forge_core` the type `DiffSuppression`.
- **New council exit code 3, `recorded_incomplete`** (`check_environment`, `may_be_transient`
  true; plan of record S3.13, [DDF-2026-004](docs/advisories/DDF-2026-004.md)). A contract
  addition: `council import-tuicr --execute` exits 3 when its decisions were recorded (the synced
  append) but a later write failed, or the journal's own sync failed. The message names the
  events and the file; the same import run again completes it. No existing code changes its
  meaning; `schema exit-codes`, `explain`, MCP and the README list it.
- **The decision journal's lock is an advisory `flock`** on `decisions.lock`, held for the life
  of an executing import, as the hooks and the daemon take theirs. A lock file left by a dead
  process, of any version, no longer blocks; two live imports still exclude each other. The
  `Locked` message no longer tells the reader to remove the file.
- `TuicrImportError` has a new variant `Incomplete { first, last, file, reason, durable }` (the
  enum is `#[non_exhaustive]`); it is not a refusal.
- Every executing import reconciles first and may repair, each step announced on standard error
  (`note: tuicr import repair: …`; a dry run says `would repair` and writes nothing): see Fixed.

- The evidence readings of 0.9.0 that join hunks without context (Git-must joins, number hints,
  hunks of one created file) are no longer reached from a parsed patch: the parser refuses a
  hunk with no context past line 1 and a hunk after one with no trailing context. They stay
  tested on hunks built without the parser. Series whose hunks carry context are read as before;
  of 6,110 corpus patches 0.9.0 read, 698 are refused and none read changed; the evaluation
  moved 18 such sections to `fixtures/eval-refused-by-parser/` (precision 170/195, recall 170/247).
- `PatchParseError` has three new variants (the enum is `#[non_exhaustive]`):
  `HeaderNotWrittenByGit { line_number, header, reason }`,
  `HunkPlacedElsewhere { line_number, reason }` and `GitCannotWrite { line_number, reason }`.
- DDF's own `git diff` (Git selections and file pairs) passes `-U3`, so `diff.context` in the
  user's configuration cannot change the capture.
- The benchmark's generated patch ends each hunk with a line of context, as Git writes it (a
  hunk with none, followed by another, is now refused). The inputs of `parse_seal`,
  `rank_with_evidence`, `outline_page`, `findings_by_address` and `daemon_parallel` are
  re-pinned, so `scripts/bench_compare.py` does not compare them with earlier runs.
- A rename DDF finds from the `---`/`+++` names, with no rename header, now records its
  `rename from`/`rename to` before the `index` line, where Git writes them, so that DDF's
  rendering is a shape Git writes. Before, they came after it.

- **The repository no longer links into the maintainer's private Obsidian vault** (S3.17b.a).
  251 links of the `obsidian` scheme in 103 tracked files were replaced by in-repository pointers (in
  source comments, with every line number kept) or removed (in documents), and the contract
  test `no_tracked_file_links_the_private_vault` keeps them out. The two policy scripts that
  are compiled into the binary changed in one comment line each, so the release binary's hash
  changes; its answers do not (7,421 comparison patches, identical in both directions).

- **`deep-diff-forge.tuicr-import.v1` gains `repairs`** (S3.17a.d; additive): the repairs an
  import made to the journal are returned in the document, and the command line prints the same
  `note:` lines as before. `TuicrImportError::Incomplete` carries the I/O error as its `source`
  (library users of `deep-diff-forge-review`: the field `reason: String` is replaced). The refusal
  for a held import lock names what to do first, so that it survives the 200-byte cut.


- **Binary creations, deletions and renames carry Git's status** (S3.4): `added`, `deleted`,
  `renamed`, the word `git apply --summary` gives, instead of `binary_changed`. They stay binary
  (strategy `binary`, signal `binary_change`) and score as in 0.9.0; a binary change in place and
  a binary copy stay `binary_changed`. A traditional diff that creates or deletes a file without
  `/dev/null` still reads `modified` (an open difference recorded in `CONFORMANCE.md`).

- **`--stdin-patch` refuses input in which Git finds no patch** (S3.5): zero bytes, whitespace,
  prose, a commit message alone. Exit 4 with `error: no_patch_in_input`, as `git apply` refuses
  it; before, it was exit 0 with zero files, which read as a clean change. Git's own
  `--allow-empty` (new, admitted with `--stdin-patch` only) restores the empty answer for a
  pipeline that means it. `--require-files` and `--require-hunks` still refuse first with exit
  7; Git selections, file pairs and the daemon, rpc, MCP and SDK surfaces are unchanged.
  forge-warden: with a 0.10.0 ranker, an authority check of an empty patch ends in "the ranker
  exited 4" (its exit 3) instead of a holding verdict document.

- **A `GIT binary patch` body that `git apply` calls corrupt is refused** with exit 4 (S3.6): a
  hunk without its terminating empty line or without data, or a base85 group above 0xffffffff.
  Corruption inside the deflate data is still not detected (stated in `SECURITY.md`). New public
  items in the core crate: `is_binary_marker` and `PatchTwin::is_binary`.


- **The documents say what is built and where it is released** (S3.7, S3.10, S3.17a.k).
  `SECURITY.md` lists every Git setting that changes what `git apply` writes (`apply.whitespace`,
  `core.autocrlf`, `-p`, `--directory`, `--3way`, `core.fileMode`, a case-insensitive file
  system), each declared, six measured against a real `git apply` by the test
  `a_setting_that_changes_the_write_is_declared`. Linux x86_64 is built and tested, with its
  receipt under "Runs on record" in the platforms page; arm64 and macOS are not built for this
  release. Releases are on GitLab and mirrored on GitHub; 0.9.0 is on crates.io and publishing
  the crates is a release step from 0.10.0; contributions are GitLab merge requests. The
  generated differential draws four more shapes Git writes (no final newline, an edit of an
  empty file, a file emptied, unequal hunk counts).


- **`rank.v0` has one byte form on every surface** (S3.15): the bytes `--rank --json` prints.
  They are built by one renderer. The daemon's `diff.rank` (so `rpc`, the Python SDK and
  `review.change` with the rank view) keeps its value and adds those exact bytes as
  `canonical_json`; MCP `rank_patch` gives them as its text and their value as
  `structuredContent`, and its 256 KiB cap counts them. The command line's own output for a
  patch is unchanged apart from one added member (7,421 comparison patches, identical once
  that member is set aside): `tool.scorer_sha256`, a build-time SHA-256 of the ranking crate's
  sources that decide labels (every source file of the graph crate but `lib.rs`), which moves
  on any edit of them. A consumer that seals the ranking's bytes (forge-warden) sees its seals
  change once by that member. `agent doctor`'s `rank_parity` now compares the command line's,
  the daemon's and MCP's bytes. A `diff.rank` answer is about 2.2 times its earlier size.


- **What the hooks tell the judged agent** (S3.17a.f, i, l; S3.21.d). Advice after a tool call
  says how a line was read as the ranking reads it (`lexical` or `hunk`) and names only the
  accepted repair: no hook message names a substitute that does not count (0.9.0 printed the
  first "Not accepted" substitute, some of which no label raises) or points the agent at
  `explain signal`. Commands the hooks name are this binary's absolute, shell-quoted path.
  Advice names once, for the whole file, a signal that has no evidence row (`hazardous_text`,
  `executable_bit`, `symlink_change`, or a code signal past the 32 rows). When advice is off for
  a reason told once, the model is told as well as the user. A built-in rule `builtin:<name>`
  keeps its name at later stops.

- **The judge's patch is the bytes forge-warden's `make_patch.sh` writes** (S3.17a.h): full index
  lines and the submodule options, held by a test that runs a recorded copy of that script. A
  verdict's `patch_sha256` therefore differs from 0.9.0's for the same change.

- **`agent doctor --gate` gains the link `claims_verified`** (S3.25.a). Under a policy that
  requires verified claims (forge-warden 0.3.0's `init` writes one) the hook's stops hold and
  never pass, an uncommitted or staged change can at best hold ("commit first"), and the
  session start says so. `docs/reference/agent-hooks.md` states the hooks' limits, among them
  four ways a judged agent can make a stop silent or unjudged, each re-run on this build.


- **`eval` says where it is right and wrong** (S3.35). The `deep-diff-forge.eval.v0` receipt
  (`eval --json`) gains `by_signal` and `by_language`: evidence precision and recall split by
  signal and by the language of the file (additive; each adds up to the aggregate; the text
  summary is unchanged). A case whose patch the parser refuses is listed in the new member
  `refused` with the reason and fails the run with exit 7 while every other case is still scored;
  before, the first such case ended the run with exit 4 and no figures. A test holds the figures
  written in `docs/reference/evaluation.md` and `docs/TESTING_GOLD_STANDARD.md` to what the run
  measures. The hooks' mutation misses of the 0.9.0 run are answered by six new tests (S3.34.b).


### Fixed

- **A text file Git was told to show as binary is read as text** (S3.20,
  [DDF-2026-006](docs/advisories/DDF-2026-006.md)). In the Git selections (and so in MCP and RPC
  `review_change`), one line `*.rs -diff` in a `.gitattributes` the change need not contain, in
  `.git/info/attributes` or in a file the repository's own `core.attributesFile` names, `* binary`,
  a `diff=` driver with `binary = true`, or one NUL byte in a comment made `git diff` print a source
  change as a `GIT binary patch`, and no evidence rule read it: a change that starts a process
  ranked `binary_changed`, 3, `binary_change` (measured on the published 0.8.2, 0.8.3 and 0.9.0
  binaries and the 0.8.4 release binary, every selection). The acquisition as it was stays the
  first reading; every change Git gives as binary is asked for once more with `--text`, the same
  selection and flags, limited to its own literal paths. When that is the same change, as text,
  valid UTF-8 (NUL bytes included), within what is left of the byte budget, and the parser reads
  it, it replaces the binary section (both apply to the same bytes; tests apply the reviewed patch
  with a real `git apply`); otherwise the binary section stands. Real binaries and text over the
  budget stay binary; `--stdin-patch` and file pairs are unchanged (stated limits in
  `docs/reference/commands.md`). An ordinary repository's output is byte for byte what it was.
- **`ci_privilege` reads a key or a value in quotes as the key or value** (S3.18,
  [DDF-2026-005](docs/advisories/DDF-2026-005.md)). 0.8.2 to 0.9.0 labelled `contents: write`
  in a workflow but not `contents: 'write'`, `contents: "write"`, `id-token: 'write'`,
  `"contents": "write"`, `{contents: 'write'}` or `contents : write`, which YAML reads the same:
  a privilege grant was missed. A YAML text is now read a second time with each quoted scalar
  that holds no escape as its content and the blanks around a key's `:` as one space, by the
  bytes of the line; what the two readings find is united, so no line has fewer items than
  before. Still not read, and stated in the README row: a value with an escape, a tag, a block
  scalar, an explicit `?` key. `build_hook` reads the JSON key of `package.json` in every
  spelling JSON allows but a `\u` escape, as before; that limit is now stated.
- **`trivial_assert` no longer labels a Python `assert True` followed by more of an expression**
  (S3.16; a change of a released label's behaviour). 0.9.0 gave the label ("an added assertion
  cannot fail") to the token `assert True` whatever followed it, so to `assert True == False`,
  `assert True and False`, `assert True in []` and eight more that fail on every run, to five
  that depend on a variable (`assert True and ready`), and to `assert True \` continued on the
  next line. Each of those labels was false. The label is now given only where the statement
  ends: the end of the line, a `,` and a message, a `;` or a comment. `assert True or ready`,
  which cannot fail, is no longer labelled either (a stated limit). The other spellings do not
  change. Measured on the 0.9.0 release binary and on this tree before the change; 0.8.4 and
  earlier had no such label.
- **A fault during `council import-tuicr --execute` no longer blocks a run's journal for good,
  reports a recorded decision as refused, or leaves a cut back undetected** (S3.13,
  [DDF-2026-004](docs/advisories/DDF-2026-004.md); measured on 0.9.0 by fault injection from
  outside the process).
  - `write_new`, the writer of checkpoints, the export ledger, reports, `execution.json` and
    `qualification.json`, is all or nothing: it writes and syncs a temporary file in the same
    directory and links it to its name, which fails if the name exists. A failed write leaves no
    file (before, an empty one every reader refused and create-new would not replace), and the
    error names the file. Readers that list a directory skip a temporary name.
  - The journal's commit point is the synced append. The new head's checkpoint is written and
    synced under its temporary name first, so a failure before the append is exit 4 with nothing
    appended (before, exit 4 with the decision appended); a failed append is cut back. After it,
    exit 3.
  - Every executing import writes the head's checkpoint when the head is beyond the newest one,
    even when it plans no events (before, the retry never wrote it and a cut back went
    undetected); this also heals runs left so by 0.8.3 to 0.9.0.
  - Repairs under the lock: an empty checkpoint the journal holds is replaced; a torn last record
    no checkpoint names is cut off and its bytes kept in `decisions.torn-<seq>-<digest>`;
    leftover temporary checkpoint files are removed. A torn record a checkpoint names, and other
    malformed checkpoints, stay refusals.

- **The stop hook reads forge-warden's verdict version 3, and a fail it cannot read holds the
  stop** (S3.21). forge-warden's main branch (for 0.3.0) writes `forge-warden.verdict.v3`;
  0.9.0 read only v2, so a v3 document was no verdict and a stop the judge failed was allowed
  as UNJUDGED. No released pair is affected (forge-warden 0.2.1 writes v2), so there is no
  advisory.
  - A v3 document is checked member by member against forge-warden's v3 schema, as v2 is
    against its own; v2 is read as before. What v3 adds is read in its own form only: bound
    claims (`verified: true`, `base`, `head` and `merge_base` of one length, 40 or 64 hex
    digits), which the hook cannot check and does not use (the seals bind the verdict to its
    patch); a rule of kind `builtin`, which has a `builtin:` id (and is the only one that
    may), holds and names no waiver; the note code `claims_unverified`. Any other schema is no
    verdict, and the reason names it.
  - Exit 7, the judge's fail, now holds the stop when its document is no verdict (missing, of
    a schema this build does not read, not whole, not sealed to the inputs, saying another
    outcome): once, as a fail with no failing rule, with why the verdict could not be read.
    Before, it was allowed as UNJUDGED. Exit 0 and 8 still need a verdict.
  - `agent doctor --gate` names, in the link `chain`, the verdict schema and forge-warden
    version of the judge's document and the schemas this build reads; a judge that writes a
    schema this build does not read fails that link, named with its version.

- **A moved file's old path is squashed as Git squashes it** (independent review of S3a,
  S3A-L-1, low). `rename from d//x` or `copy from d//x` was reported as `old_path` `d//x`, while
  `git apply` read `d/x`. It is `d/x` now (`render::old_path`), with a real-apply test.

- **A judge that left a process holding its output could stretch an agent's stop past its limit**
  (S3.17a.a, [DDF-2026-008](docs/advisories/DDF-2026-008.md)). Measured on 0.9.0: a stand-in judge
  that exits at once and leaves a child holding its standard output held the stop for 75 s against
  the documented 45 s. Every child DDF waits for (the judge, the ranker seal, the council's Git and
  tuicr calls) now runs in a process group of its own; the whole group is killed on every exit path
  (time limit, output cap, normal exit), and the output is joined only until the deadline. The same
  stop now takes 0.15 s. A process that leaves the group (`setsid`, a double fork) is not killed by
  DDF, and Ctrl-C on an interactive council command no longer reaches its Git or tuicr child: both
  are stated in `SECURITY.md` and in the module's documentation.

- **`council import-tuicr`: four low findings of the journal review** (S3.17a.d, S3.17a.e). The
  journal is no longer appended through a link at `decisions.jsonl` (a dangling link used to create
  and fill its target). A session that is refused no longer triggers the journal's repairs. A failed
  directory sync of a bare file name no longer removes the file. Temporary files that a killed writer
  left in the export ledger, the reports and the job directories are removed when their writer has
  ended, and named when it is still running. A mounted host path of the gate sandbox that holds `:`
  or `,` is refused before it reaches `--volume`.

- **The daemon's clients** (S3.14). Python `StdioClient`: reads a large answer in linear time
  (64 MiB: 14 s before, 0.3 s now), holds the answer line to the 80 MiB line cap with its newline
  counted, kills a child that broke the pipe before taking the whole request, turns read errors
  into `TransportError`, closes both pipes on every exit, and raises `TransportError` for a call
  after `close()` (it was `ValueError`). Python `SocketClient`: reads the daemon's early refusal
  after a broken or reset send (it said "could not be reached"), applies one deadline to the whole
  call ("no answer from the daemon at PATH within T s"), holds the answer line to the line cap,
  and reads `SO_PEERCRED` user ids at or above 2^31 correctly. Rust client
  (`deep_diff_forge_daemon::request`, the SDK's `Client`): reads the daemon's answer when the
  write breaks with a connection reset. The reference states one line cap for the daemon
  protocol: 80 MiB counting the newline, in both directions (one stated exception: the server's
  answer bound still measures without the newline).

- **`secret_like` finds GitLab tokens and fine-grained GitHub tokens** (S3.24,
  [DDF-2026-007](docs/advisories/DDF-2026-007.md)): `glpat-` and its routable form, `gldt-`,
  `glrt-` and `glcbt-` with 20 bytes or more, `glptt-` with 40, `gloas-` with 64, and
  `github_pat_` with 82. Releases 0.5.0 to 0.9.0 missed them. The eleven earlier prefixes are
  unchanged (a test holds "released implies now"), and the token is never copied into evidence.
  Measured before release: no line of 2,086 real history patches gains the label; a repository
  whose forge-warden policy forbids `secret_like` fails, from 0.10.0, a commit that adds a line
  with one of the seven forms. The eval corpus gains the case `evidence/secret-forge-tokens`
  (59 cases: precision 186 of 211, recall 186 of 264).

- **The daemon answers a changed file name that is not UTF-8 with code 4** (`review.change`),
  as the command line exits 4 (S3.27.f); it was code 3, which reads as "may be transient".

- **A file hidden from `git diff` is never both blind and unlabelled** (S3.27,
  [DDF-2026-006](docs/advisories/DDF-2026-006.md)). The first fix of this release read such a
  file again as text and labelled it; it left the file silent when that reading failed. Now a
  path whose `diff` attribute is unset or names a driver carries `diff_suppressed` whatever the
  second reading gives (a byte that is not UTF-8, the byte budget, a refusal). A UTF-16 or
  UTF-32 file, also an existing source rewritten in one of them, is labelled with the new cause
  `encoding` (labelled, not decoded). A second reading that the byte budget or the time limit
  cuts off never leaves a binary change silent: the attribute label stands, or the acquisition
  fails with the step named; a binary by content whose text form passes the remaining budget
  is therefore refused where 0.9.0 read it. A changed file whose name is not UTF-8 exits 4
  (`change_input`), not 3, in every Git selection and in a file pair. A Git helper that
  outlives Git can no longer stretch the acquisition past its limit (S3.17a.b).

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
