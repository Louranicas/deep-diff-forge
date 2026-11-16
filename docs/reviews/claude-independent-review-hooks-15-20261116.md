# Fifteenth independent review of the agent hooks branch (`agent/loop`, commit `c9a2ece`)

Date: 2026-11-16, about 19:10 to 19:30 AEDT (08:10 to 08:30 UTC) by the host's clock (`date`).
Reviewer: an independent Claude agent that took no part in the work. Brief:
`~/.local/state/habitat-claude/ddf-gap-probes-20261113/ddf-hooks-independent-review-15-brief.md`
(extends the eleventh brief, read whole; machine rules `machine-rules-20261115.md` read first).
Under review: `d4dbc91..c9a2ece`, mainly the rework `8959ca8..c9a2ece`.

## Verdict: GO-WITH-FIXES

Every finding round 23 answers is closed on the frozen binary with the real judge: VD4R22-M-1
(both cases), HR14-L-1, HR14-L-2 (both layouts), HR14-L-3, HR14-L-4 in its reported layout, and
the fourteenth's suspicion (a `cwd` that names a file). The fourteenth review's whole corpus (25
batch scripts, the 144-file `nr`/`mi` matrices, the `w13`/`w14` scripts) gives the same decision
at every step as on `v4r22`, except the steps the round's rulings mean to change. I found no stop
that prints nothing while an ORDINARY WORKFLOW leaves unjudged work in the project, and no stop
that passes a failure.

Two lows, one root: for a linked tree, the hooks take the directory Git names as the tree's top
and the tree's identity from Git's Git directory, and nothing checks that the two belong
together. Round 23's new rule ("Git's answer must hold the session's directory") checks only
that Git's top contains the session's directory.

- **HR15-L-1** (low; DELIBERATE EVASION, implausible by accident; older, narrowed by this
  round). `core.worktree` in a linked tree's own configuration pointed at a SUBDIRECTORY of the
  tree that holds a clean copy of its files: Git's top then contains the session's directory,
  so the new rule believes it. The stop from that subdirectory judges the copy as the tree,
  finds it clean and forgets what blocked in the tree; the next stops from there and from the
  project are SILENT with two keys in the tree. README, SECURITY and the CHANGELOG say the
  opposite ("a `core.worktree` that points Git at another directory makes the stop UNJUDGED,
  and nothing of the tree is forgotten"; "what is read of another tree counts as that tree's
  only when Git names that tree there").
- **HR15-L-2** (low; ORDINARY WORKFLOW, uncommon; older, not new in this round). A copy of a
  linked tree (`cp -a L C`) has a `.git` file naming L's administrative directory, so Git calls
  C a working tree with L's identity. `git clean` in the copy and a stop from it: the copy is
  judged as L, found clean, and what blocked in L is forgotten, at a stop whose own text says
  "A failure there has blocked a stop of this session and stands". The next stop from L, with
  L unchanged, BLOCKS a second time. Not silent, never a pass; it breaks claim 5 ("forgotten
  only when its directory is gone or the judge's own reading finds it clean": the reading was
  of another directory) and the rule that an unchanged failure blocks once unless a stop said
  its memory was lost.

The fix for both is the same and cheap: a linked tree's top is the directory its
administrative `gitdir` file names (`linked_instances` reads it already). Where Git's top for a
Git directory under `worktrees/` is not that directory, take the answer as `Found::Failed`
(UNJUDGED, nothing forgotten), in `elsewhere` and before a judged-clean reading forgets an id;
and in `other_tree`, ask `pointed_elsewhere` for every listed tree, not only for a tree where a
failure stands.

## How this was done

- Source: `git archive HEAD` into `/var/home/louranicas/.cache/ddf-t/hooks-review-15/src`;
  `git diff 8959ca8..HEAD -- crates/` in `…/hooks-review-15/notes/r23.diff`; README, SECURITY
  and CHANGELOG diffs read. No `cargo`, no build, no `toolbox`, `podman` or `distrobox`, nothing
  at GitHub, no network. The only file written in the worktree is this report.
- Binary: `/var/home/louranicas/.cache/ddf-t/review-bin/v4r23/release/deep-diff-forge`, SHA-256
  `92d740a58ad68452428e787370a2700763b53faa907b595f9c86d5729390f2ea` (debug `07fe6853…`
  checked, not run). For "is it new": `…/v4r22/release/…` (`4709a2d0…`). Judge: the real
  forge-warden (`…/target-fw/release/forge-warden`) on the host.
- Bases: `…/hooks-review-15/d15/base-repo` (a `--no-local` clone of
  `hooks-verify23/d11/base-repo`, `0da8cac`, admits v4r23); `d15/base-r22` (the same reset to
  `389e1d1`, admits v4r22); `d15/base-both` (my commit `448afaf` admits both, for the schema
  runs). Case trees under `d15/cases/`; a private HOME under `d15/` set inside every driver.
- Scripts and outputs: `/var/home/louranicas/.cache/ddf-t/hooks-review-15/notes/` (`s/`: my
  `w15*.py`, `io.sh`, and copies of the fourteenth's `w14*.py` and Vesper's reproductions;
  `s14/`: the fourteenth's corpus with paths changed; `rv11/`: the eleventh's library);
  outputs in `notes/out/` and `notes/s14/out/`; script hashes in `notes/out/script-hashes.txt`;
  a dated log in `notes/NOTES.md`. I stopped no process.
- One harness mistake, recorded: my first rerun of the fourteenth's corpus (`run15b.sh`) shared
  one HOME across seven lanes, and `par.py` makes the state directory read-only for a moment.
  Two other lanes then could not save their state (one `mi` step blocked twice; `w14git am`
  differed). Rerun with one HOME per lane (`run15c.sh`): 144 of 144 matrix files and `w14git am`
  the same as on v4r22. Not a finding.

## Section 0, item 1: the closed findings' reproductions on this build

`notes/run1.sh`, outputs `notes/out/<script>-v4r23.txt` and `-v4r22.txt`.

| finding | script | v4r23 |
|---|---|---|
| VD4R22-M-1, case 1 | `v4r22-relative-gone.py` | BLOCK, then FAIL STANDS with the linked tree judged from `../<linked>/build/gone`; memory kept. **Closed** |
| VD4R22-M-1, case 2 | same | UNJUDGED "the session's directory is in a directory that is no longer a working tree of this repository …", memory kept. **Closed** |
| VD4R21-M-1, VD4R21-L-1 | `v4r21-reserved-subtree.py`, `v4r21-relative-locked.py` | as on v4r22 (UNJUDGED, kept). Closed |
| HR14-L-4 | `w14cw.py` | step 2 UNJUDGED "Git names as the working tree a directory that does not contain the session's directory"; step 3 from R names the tree "(Git names … as the working tree there, which is not this tree)"; memory kept. **Closed in its layout** (see HR15-L-1) |
| HR14-L-3 | `w14vol.py` (a real btrfs subvolume) | UNJUDGED with Git's words, the tree "may be in it"; from L BLOCK on both keys. **Closed** |
| HR14-L-1 | `w14home.py noproject|outside`, `w14home3.py 20000` | SILENT at 0.00 s, no state written (v4r22: UNJUDGED, 0.7 to 1.3 s). **Closed** |
| HR14-L-2 | `w14ext.py both`, `w14prune.py` | stops from inside UNJUDGED, from R the directory named "(no longer a working tree; a failure stands there)", memory kept. **Closed** |
| ceiling (13th) | `w14ceil.py linked` | BLOCK, FAIL STANDS. Held. `w14ceil.py project` SILENT (as now stated: a session begun under a ceiling that hides its repository has no project) |
| 14th, suspected | `w15cwd.py listed|unlisted file underfile relfile` | a `cwd` naming a file, or under one: walked up, the tree judged (listed), UNJUDGED (unlisted). **Closed** |

## Section 0, item 2: one resolution of where the session is

Read `standing_directory`, `Standing`, `through_links`, every use of `input.cwd` and
`stopping.cwd`. Only `elsewhere` resolves the directory; `locate` at a session's first stop
asks Git about the raw `cwd` (a `cwd` that is gone there is UNJUDGED "does not exist", never
silent), and the failed branch of `trees_to_judge` only shows the raw path. `w15cwd.py`
(`out/w15cwd-listed-v4r23.txt`, `-unlisted-…`), each case a fresh session with a key that blocked
in L and a second key there, process directory R:

| `cwd` | L listed | L's records removed |
|---|---|---|
| a file in L; a path under a file | BLOCK on L | UNJUDGED, kept |
| `../L/lib/first.rs` (relative) | BLOCK | UNJUDGED, kept |
| `/../..` + L; 30 `..` past `/` + L | BLOCK | UNJUDGED, kept |
| a symlink loop in L (`L/loop/sub`) | UNJUDGED "Too many levels of symbolic links" | the same |
| a link outside L to `L/lib` | BLOCK | UNJUDGED, kept |
| `L/.` | BLOCK | UNJUDGED, kept |
| a directory that cannot be searched in L | UNJUDGED "Permission denied" | the same |
| a removed directory in L | BLOCK | UNJUDGED, kept |
| `""` | UNJUDGED "names no session_id or no cwd" | the same |

No two uses differed. Held.

## Section 0, item 3: Git's answer must hold the session

- `core.worktree` at the project (HR14-L-4): held. At a subdirectory of the tree with a clean
  copy: HR15-L-1. At an empty subdirectory (`w15cw.py empty`): the stop judges a tree with every
  file deleted, HOLD on `tests/t.rs` and `warden.toml`, the key in L not judged, and what blocked
  in L forgotten at that stop; never silent, never a pass. At a parent of the tree
  (`w15cw.py parent`): my driver failed before the stops (Git outside the redirect); not run.
- A copy of a linked tree: HR15-L-2 (`w15copy.py clean`); `w15copy.py keep` (the copy with the
  key): FAIL STANDS from the copy, which is right.
- `GIT_DIR`, `GIT_WORK_TREE`, `GIT_COMMON_DIR` stay removed by the shared list (source).
  `includeIf` and `safe.directory` are the user's global configuration; an `includeIf` that sets
  `core.worktree` is HR15-L-1's case. Not run further.
- The boundary rule (`w15vol.py`, a real btrfs subvolume each, `out/w15vol-*-v4r23.txt`):
  - in the MAIN tree, `cwd` = `R/vol/sub`: Git's "no repository" at the boundary, `across` false,
    and the project's own reading includes `vol/sub/k.rs` (Git's status crosses a subvolume
    inside a tree): BLOCK, then FAIL STANDS from there and from R. Right: the main tree is judged
    at every stop.
  - a linked tree INSIDE the main tree (`R/wt/feature`, a subvolume in it): UNJUDGED with Git's
    words, the tree "may be in it". Held.
  - a subvolume in a linked tree whose two records are then removed: UNJUDGED ("no longer a
    working tree"), memory kept. Held.
  Git names the first directory it did not look in (the subvolume's parent; `/var` for this
  host's `/var/home` subvolume), so "at or under a linked tree's top" is the right test, and a
  boundary above a tree's top cannot lie between the session's directory and that tree. Narrowing
  the rule to linked trees loses nothing I could find: the main tree is always judged, and an
  unlisted tree is caught by the later `unlisted` check. A mount point whose path is not UTF-8
  is read lossily and would not match (not run).

## Section 0, item 4: the ceiling

`Deadline::with_project` is applied in `post_tool`, `session_start` and `stop` right after
`locate`; `locate` and `agent doctor --gate` use the caller's ceiling (`locate` asks only about
the recorded top or, at the first stop, the raw `cwd`). `w15ceil.py` (`out/w15ceil-*-v4r23.txt`):

- `alias`: a ceiling that names `L/sub` through a link outside the tree; `cwd` = `L/sub/deep`:
  BLOCK on L, FAIL STANDS. Held.
- `ceilrel`: `sub:X:X::X` (relative, repeated, empty entries): BLOCK, FAIL STANDS. Held.
- `dotdot`: a repository H the user hides with a ceiling written through the project's path
  (`R/../H`, which is H): the entry is dropped because it is compared "as written" with
  `Path::starts_with`, which does not take `..` out; a stop from `H/notes/today` says "The
  session's directory is in another repository, H". HR14-L-1's outcome for a ceiling written in a
  form no one writes; I do not count it as a finding. Cheap: compare only the resolved entry (or
  take `.` and `..` out by name first), not the entry as written.

## Section 0, item 5: state schema v10

- `w15schema.py up` (`base-both`, one session, binaries swapped): v4r22 writes v9; v4r23 reads it
  (FAIL STANDS), writes v10 with the tree recorded; the records then removed: UNJUDGED from inside,
  named from R, memory kept; v4r22 again reads the v10 state as "not a state of this version"
  (UNJUDGED with the reason). As stated.
- `w15schema.py unlistedfirst`: the records removed while the state is still v9, then v4r23:
  SILENT and forgotten. The CHANGELOG states it ("its trees no longer listed are forgotten as
  before"); README's "read with no tree recorded" implies it.
- `trees_from` and `recorded_trees` agree on the caps (`recordable`, 16 KiB a path, 256 KiB in
  all, at most `BLOCKED_CAP` members, 16-hex ids); a member that is anything else refuses the
  whole state (UNJUDGED, nothing recorded over it). A planted id mapped to `/` would make every
  stop UNJUDGED: noise for a state the hooks did not write, never a pass.
- A tree again, from inside it (`w15again.py clean|key`): clean, the old id forgotten at the
  first stop from inside, SILENT; with the same key put back, it BLOCKS once under the new id
  (a new tree never inherits "has blocked", as designed), stands, and both ids are forgotten when
  the key goes. As stated.
- More than 1,024 administrative directories make `linked.whole` false, so nothing is called
  unlisted; `w15many.py 1024` shows why that cannot be silent: with more than 256 trees every
  stop carries "The commits of this session: UNJUDGED".
- A recorded top that is now a file is forgotten; one that is a link to a directory stands. Read,
  not run.

## Section 0, item 6: ordinary workflows from inside a Git directory

`run15c.sh` (fourteenth's `w14git.py`, `w13*.py`, `w12a.py modules`; `notes/s14/out/b-*.txt`):

| from | decisions against v4r22 |
|---|---|
| `.git`, `.git/hooks`, `.git/lfs/tmp`, `.git/rebase-merge`, `.git/rebase-apply` (`git am`), `.git/NOTES_MERGE_WORKTREE` (`w13c`) | the same at every step |
| `.git/modules/sub` (`w14git submodule`), `.git/modules/sub/hooks|refs` (`w12a modules`) | HOLD or FAIL STANDS became UNJUDGED (the round's ruling 2); the project is judged at every stop and its failure blocks once and stands; one UNJUDGED line per stop |
| `w13a top|sub|hooksx`, step 3 from the project | SILENT became "1 thing … not judged: the directory … (no longer a working tree; a failure stands there)" (ruling 4) |

Nothing blocks that did not, nothing blocks twice. Two remarks on words, not findings: from
`.git/modules/sub` the reason is "Git names as the working tree a directory that does not
contain the session's directory … whether the session's directory is another working tree
could not be found out" where v4r22 said, more usefully, that the directory is in another
repository (the submodule); and README's paragraph says a stop made from inside a Git directory
gives a reason that says the directory "is inside the Git directory", which is not the reason
given from `.git/modules/<name>`.

## Section 0, item 7: the sentences

True where said: README's bullets on where the session is, the ceiling, the boundary and the
unlisted trees; SECURITY's "What blocked in a tree is not forgotten while its directory stands";
the CHANGELOG round entry, whose tests exist (`a_relative_directory_that_was_removed_is_anchored_and_walked_up`,
`a_tree_git_is_pointed_away_from_is_unjudged_and_keeps_what_blocked_there`,
`a_boundary_inside_a_linked_tree_is_not_taken_for_no_repository`,
`a_repository_the_user_hides_with_a_ceiling_stays_hidden`,
`a_tree_no_longer_listed_stands_while_its_directory_does`,
`a_state_of_the_schema_before_is_read_and_written_anew`, and the unit tests
`where_the_session_is_is_found_one_way` and four more).

Wider than the code:

- SECURITY: "a `core.worktree` that points Git at another directory makes the stop UNJUDGED,
  and nothing of the tree is forgotten" (HR15-L-1).
- README: "what is read of another tree counts as that tree's only when Git names that tree
  there"; CHANGELOG the same (HR15-L-1, step 5; and `other_tree` asks only where a failure
  stands).
- Request claim 5 and the README bullet "It is forgotten when its directory is gone … or when it
  is a tree again and the judge's own reading finds that tree clean": also when a copy or a
  subdirectory is read as the tree (HR15-L-1, HR15-L-2).
- Missing: no sentence says that a stop from a submodule's Git directory (`.git/modules/<name>`)
  is now UNJUDGED; the request's "Checked" says it, the CHANGELOG entry does not.

## Section 0, item 8: the eleventh brief's sections 1 to 7, briefly

- The fourteenth's corpus on v4r23 (`run15c.sh`, compared by `s14/bcmp.py` and `s14/mcmp.py`
  with the fourteenth's outputs on v4r22): `nr` and `mi` 144 of 144 files the same at every
  step; the batch the same except `w12a modules` (ruling 2) and `wt2` (commit ids only); `cn2`,
  `cp`, `ot` compared by hand: the same text but paths and times (`cn2`'s injected sentence in a
  tree's name stays escaped as `\x0a`).
- Section 3, the input contract (`s/io.sh`): input that is not JSON, empty, for another event,
  with no `cwd`, a number for `cwd`, a NUL in `cwd`, and 3 MiB of input: each answered with valid
  JSON, exit 0, UNJUDGED with the reason (the 3 MiB input ran a normal stop).
- Sections 1, 2, 4, 5, 6 (`agent setup`, `agent doctor --gate`, `schema signals`, `explain
  signal`, `verdict_v2.rs`, the judge call): not changed by this round; not run again beyond the
  corpus.
- Section 7, the tests: read, not run (no `cargo`). The new integration tests use real Git and
  the reading judge, and each would fail without its guard (the implementer's control says 6
  fail on `8959ca8`). Missing: `core.worktree` pointed inside the tree (HR15-L-1), a copy of a
  linked tree (HR15-L-2), "a tree again" judged from inside the new tree (I ran it: it holds).

## Findings reproduced with the real judge

### HR15-L-1 (low; DELIBERATE EVASION, implausible by accident; older, narrowed by this round): `core.worktree` pointed inside the tree

```
python3 -I /var/home/louranicas/.cache/ddf-t/hooks-review-15/notes/s/w15cw.py copy    # out/w15cw-copy-v4r23.txt
-- 1 key in L; cwd = L                                   => BLOCK   mem: 25dfb6fa2e8376a5/u:1
   git config extensions.worktreeConfig true; git -C L config --worktree core.worktree L/sub
   (L/sub holds `git archive HEAD` of L); git in L/sub: top = L/sub, status clean
-- 2 a second key in L (outside L/sub); cwd = L/sub      => allow "Whether the working tree L … could not be found out
                                                            (Git names L/sub as the working tree there …)"   mem: -
-- 3 cwd = L/sub again                                   => SILENT  mem: -
-- 4 cwd = L                                             => allow:UNJUDGED (Git names L/sub …)               mem: -
-- 5 cwd = R                                             => SILENT  mem: -
```

v4r22 (`out/w15cw-copy-v4r22.txt`): SILENT at steps 2 to 5. Round 23 makes step 4 UNJUDGED and
adds a line at step 2, but the stop from `L/sub` judges the copy under L's identity, finds it
clean and forgets L at step 2; steps 3 and 5 print nothing with two keys in L. From R,
`other_tree` asks `pointed_elsewhere` only where a failure stands, so once forgotten, L is never
named again: `git status` in L reads the clean copy. How likely: it needs a configuration only an
agent setting out to hide work would write. How harmful: as HR14-L-4 (nothing in L can be
committed through the redirect without the redirect's view; the harm is the silence), and it
contradicts the round's sentences. Cheap: see the verdict (Git's top must be the directory the
administrative `gitdir` names; ask `pointed_elsewhere` for every listed tree).

### HR15-L-2 (low; ORDINARY WORKFLOW, uncommon; older): a copy of a linked tree is judged as that tree

```
python3 -I /var/home/louranicas/.cache/ddf-t/hooks-review-15/notes/s/w15copy.py clean   # out/w15copy-clean-v4r23.txt
-- 1 key in L; cwd = L                       => BLOCK   mem: 36c6a79b2057d03a/u:1
   cp -a L C; git -C C rev-parse: top = C, git dir = R/.git/worktrees/<L>; git -C C clean -fdq
-- 2 cwd = C                                 => allow "The working tree L … has changes that are not committed, and the
                                                session's directory is not in it … A failure there has blocked a stop of
                                                this session and stands …"                   mem: -
-- 3 cwd = R                                 => allow "The working tree L … has changes that are not committed …"   mem: -
-- 4 cwd = L (unchanged since 1)             => BLOCK   (the same key, the same rule, the same path)
-- 5 cwd = L again                           => FAIL STANDS
```

v4r22 the same decisions (`out/w15copy-clean-v4r22.txt`). Step 2 says the failure stands and
forgets it in the same stop; step 4 blocks an unchanged failure a second time with no stop having
said the memory was lost. Never silent, never a pass. How likely: copying a worktree directory
with `cp -a` to try something is a known agent mistake on this machine (it once committed to the
real branch from a scratch copy); cleaning the copy and stopping from it is less common. How
harmful: one extra block for a real failure, and a false "stands". Cheap: the same check as
HR15-L-1 (the copy's top is not the directory L's `gitdir` names: UNJUDGED, nothing forgotten),
or key an id read from inside by Git's directory and the top together.

## Suspected only (not reproduced)

- `w15cw.py parent` (`core.worktree` pointed at a directory that holds the tree): my driver
  failed before the stops; by reading, the stop judges every tracked file as deleted and the
  tree's files as untracked, so the key blocks. Not run.
- A mount point whose path is not UTF-8: Git's words are read lossily and would not match a
  linked tree's top, giving HR14-L-3's old words ("not in it"). Not run.

## What I did not look at

`verdict_v2.rs`, `vocabulary.rs`, `agent setup`, `schema signals`, `explain signal` (unchanged
by this round); the time matrices (`tm.py`, `st4.py`, `worst.py`, `od.py`, `ot.py` beyond
`ot 31 clean`); a full disk; a real mount (only btrfs subvolumes); `safe.directory` and
`includeIf` in a global configuration; ext4 or another file system without birth times; the
mutation run (no `cargo`); Vesper's round 23 report (not present when I started).
