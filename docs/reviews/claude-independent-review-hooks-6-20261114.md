# Sixth independent review of the agent hooks branch (`agent/loop`, commit `c6c4f28`)

Date: 2026-11-14. Reviewer: an independent Claude agent that took no part in the work.
Brief: `~/.local/state/habitat-claude/ddf-gap-probes-20261113/ddf-hooks-independent-review-6-brief.md`.
Under review: `d4dbc91..c6c4f28`, mainly the rework `651adae..c6c4f28` ("a stop is silent only
when everything has a pass").

## Verdict: GO-WITH-FIXES

I did not find a stop that prints nothing while something is open, in an ordinary workflow.
The rule held in the table of units against outcomes (21 rows, three stops each, real judge),
in the fourth review's 30 workflows, in the staged unit against eight index states, and in
random workflows (numbers under "How this was done"). The high finding of the fifth review
(HR5-H-1) is closed: a commit with no verdict is named at every stop and asked about again.

Two mediums stand against a merge as it is. Both are ordinary workflows, both are small to
fix, and neither is a silent stop:

- **HR6-M-1.** A failure that has blocked and "stands" blocks again, as often as the session
  goes on, whenever the unit that carries it is not looked at for one stop and is looked at
  again: the staged change after `git add -A` and one more edit; a linked working tree when
  the session's directory moves out of it and back. Nothing in the change has changed.
- **HR6-M-2.** What the judge holds is not always said. A unit that fails and also has a held
  rule never shows the held rule; for a commit, at no stop of the session. And a commit the
  judge holds, first judged at a stop whose room is used by what blocks, never shows its rule
  either, with 2,275 characters of the message unused.

Five lows follow. One of them (HR6-L-1) is the same mechanism Vesper reports in round 14 as
VD4R14-M-1; I had reproduced it before I read that report, and I rate it lower, with reasons.

Every finding is marked ORDINARY WORKFLOW or DELIBERATE EVASION. None of this round's findings
is a deliberate evasion.

## How this was done

- Source read from `git archive c6c4f28` (extracted to `/tmp/claude-ddf-hooks-review-6/src`).
  While I worked, the worktree had uncommitted changes in `hook.rs`, `hook_run.rs`,
  `agent_hook.rs`, README, SECURITY.md and CHANGELOG.md; I did not read them. No `cargo` was
  run. No `toolbox`, `podman` or `distrobox` call was made.
- Binary: `/var/home/louranicas/.cache/ddf-t/review-bin/v4r14/release/deep-diff-forge`
  (SHA-256 `ecb8605c…d467`). Judge: the real forge-warden 0.2.0,
  `/var/home/louranicas/.cache/ddf-t/target-fw/release/forge-warden` (SHA-256 `55bf9fe6…`),
  run on the host. Git 2.56.0. The host's load average was about 30 throughout.
- Scratch: `/tmp/claude-ddf-hooks-review-6/`. `base-repo` is a `git clone --no-local` of the
  fifth review's base repository with the ranker digest set to this binary and committed
  (`163a479`); `base-long` is the same with two rule ids made longer. Every case is a fresh
  clone under `cases/`. Scripts `t0.sh` … `t13.sh`, helper `h.sh`, outputs in `out/`.
- Every run had a private `HOME` set inside the helper or the script (`home`, `home-ro`,
  `home-gate`, `home-fuzz` under the scratch directory), `XDG_STATE_HOME` and
  `CLAUDE_PROJECT_DIR` unset unless a case sets them, `GIT_CONFIG_GLOBAL=/dev/null`. The real
  `~/.local/state/deep-diff-forge` does not exist, before and after.
- The helper's commands are the fifth review's: `start R`, `post R`, `stop R [true]` write the
  hook input (`stop_hook_active` as given) to `$DDF agent hook … --client claude --judge $FW`;
  `secret FILE` writes two functions and `pub const K: &str = "AKIAIOSFODNN7EXAMPLE";`;
  `newsecret FILE` writes that line alone; `harmless R` adds a function to `lib/a.rs` and a
  test to `tests/t.rs`; `holdable FILE` writes `#[allow(dead_code)]` and a function.
- Where a verdict had to be missing, the judge is `tools/flaky`: a wrapper that gives exit 3
  for a patch that holds a marker text (a given number of times, or always) or for every
  patch, and otherwise runs the real judge unchanged. These runs are marked "wrapper".
- Random workflows (`fuzz.py`): from a list of 53 Git operations (commit, amend, reset of
  three kinds, stash of four kinds, branches, merge, squash merge, rebase, cherry-pick,
  revert, fixup with autosquash, tags, `update-ref`, `commit-tree`, `git add -N`,
  `rm --cached`, `mv`, `clean`, commits in a linked working tree), one to three at random,
  then a stop, ten times a session. After each stop a check of my own (plain `git diff` and
  `git rev-list`, no code of the hooks) says whether a key is in the uncommitted change, in
  the index, or in the own change of a commit that the session's refs, `HEAD`s, stash or
  their reflogs name and nothing named at the start does. Invariant A: with a key there, the
  stop is not silent, names a failure or a missing verdict, and names or counts each such
  commit. Invariant B: with no key anywhere, no conflict, no operation in progress and a
  clean linked tree, the stop is silent (holds apart). A third run does the same through the
  wrapper, with the chain down at a quarter of the stops and key patches refused one to
  three times at another quarter. Result at the time of writing: see the last section.
- What Claude Code does (when `stop_hook_active` is true, where it cuts a message, whether it
  shows a block's reason to the user) is taken from the branch's own statements and the
  earlier reviews, not from a live session.

## Findings reproduced with the real judge

### HR6-M-1 (medium; ORDINARY WORKFLOW; (a) new with this commit's staged identity, (b) since `f9d1c9d`, where failures began to be forgotten): a failure that stands blocks again whenever its unit was not looked at for one stop

`remember` forgets a failure of an uncommitted or staged change "when a stop finds the change
without it" (`state.blocked.retain(|failure| present.contains(failure))`). What it tests is
whether this stop had that unit at all. Two units are not always there:

- the staged change is a unit only when it is another patch than the working tree's
  (`uncommitted`: `Some(&bytes) == changed.patch.as_ref() => Ok(None)`), and since this commit
  its failure has an identity of its own (`unit_failure` adds `in\0{unit.what}`);
- a linked working tree is a unit only when the hook input's `cwd` is in it.

So a staged failure is forgotten at a stop where index and tree are equal, although the index
still holds it, and blocks again at the next stop where they differ. README: "A failing
uncommitted change … holds one stop and stands while every stop still finds it … A failing
staged change follows the same rule".

**(a) Staged** (`t4.sh` S2, `out/t4.txt`). A fixture with a key-like value is told to the
person at turn 1 and left as it is. The session goes on; the fixture is never touched again.

```
 -- turn 1, stop: unstaged                                   => BLOCK
 -- turn 1, stop again                                       => allowed: FAIL STANDS
 -- turn 2 (git add -A, then one more edit of another file)  => BLOCK  Staged changes in "…/s2": FAIL
 -- turn 2, stop again                                       => allowed
 -- turn 3 (git add -A; nothing else)                        => allowed (stands)
 -- turn 4 (one more edit of another file)                   => BLOCK  Staged changes in "…/s2": FAIL
 -- turn 4, stop again                                       => allowed
 -- turn 5 (git add -A)                                      => allowed (stands)
 -- turn 6 (one more edit)                                   => BLOCK  Staged changes in "…/s2": FAIL
```

Each of these blocks tells the agent "stage the repaired file (`git add`) or take the old
content out of the index": nothing was repaired, and `git add` is what set it up.

```bash
W=/tmp/claude-ddf-hooks-review-6; . $W/h.sh
R=$(fresh s2x); SESSION=s2x; start $R >/dev/null
newsecret $R/lib/fixture.rs; stop $R; stop $R true              # block, then stands
git -C $R add -A; printf '// 1\n' >> $R/tests/t.rs; stop $R     # BLOCK (staged)
stop $R true; git -C $R add -A; stop $R                         # stands; stands (forgotten here)
printf '// 2\n' >> $R/tests/t.rs; stop $R                       # BLOCK again
```

**(b) A linked working tree** (`t4b.sh`, `out/t4b.txt`). A failing uncommitted change in a
linked working tree; stops from inside it, from the project's directory, from inside it:

```
 -- stop 1, inside the linked tree            => BLOCK
 -- stop 2, inside it again                   => allowed: FAIL, which stands
 -- stop 3, from the project's own directory  => allowed (the tree is named as not judged)
 -- stop 4, inside the linked tree again      => BLOCK      (nothing changed anywhere)
 -- stop 5, project; stop 6, linked tree      => allowed; BLOCK
```

Not a trap: each block is left with one more turn. But it is "a block that repeats" for the
life of the session, for a failure the hook itself has said is left for a person, and it
teaches the agent that the block means nothing. Suggested direction: forget a failure only
when its unit was looked at and did not have it. When the staged patch equals the tree's,
both identities are present; a linked tree that was not looked at keeps what it had.

### HR6-M-2 (medium; ORDINARY WORKFLOW; (a) older than this round, (b) this round's budget): what the judge holds is not said, at any stop, in two ordinary cases

The rule: "Everything else that is true at a stop is said at that stop, every time: what
fails and blocks; what fails and stands; what the judge holds".

**(a) A unit that fails and has a held rule** (`t10.sh` H1, `out/t10.txt`, `out/h1/`).
`hook::judged` builds `Judged::Fail` from the rules whose outcome is `fail` alone. One change
adds a key and `#[allow(dead_code)]`. The real judge on the commit's patch, run directly:

```
judge exit 7   verdict: fail   rule no-secrets -> fail   rule no-silent-suppression -> hold
```

The hook, uncommitted and then committed (three stops after the commit):

```
   block: block | names no-secrets: True | names no-silent-suppression: False | says HOLD or held: False
 -- committed:
   block: block | … | names no-silent-suppression: False | says HOLD or held: False
   block: None  | FAIL STANDS | names no-silent-suppression: False | says HOLD or held: False
   block: None  | FAIL STANDS | names no-silent-suppression: False | says HOLD or held: False
```

For an uncommitted change the hold appears once the key is repaired. For a commit it never
does: the file of judged commits keeps `f`, and every later stop says "Commit …: FAIL, which
stands". "A check it silences" is one of the three things the documents say the hooks are for.

```bash
W=/tmp/claude-ddf-hooks-review-6; . $W/h.sh
R=$(fresh h1x); SESSION=h1x; start $R >/dev/null
secret $R/lib/a.rs; holdable $R/lib/s.rs; git -C $R add -A; git -C $R commit -qm both
stop $R; stop $R true; stop $R        # no stop names no-silent-suppression or says HOLD
```

**(b) A held commit first judged at a stop that blocks for many** (`t3b.sh`, `out/t3b.txt`).
The session pulls 40 upstream commits that each fail `tests-travel-with-code`, and makes one
commit of its own that silences a check. One stop judges all 41:

```
   stop: block=True  chars=6725 | the held commit is named: True | its rule no-silent-suppression is named: False | line: '1 held by the judge, for a person: Commit 904cde739921.'
   stop: block=False chars=473  | … named: True | its rule … named: False | (the same line)
   stop: block=False chars=473  | … named: True | its rule … named: False | (the same line)
```

`trees_text` gives a new hold "the room that is left" of the 5,700 characters that what
blocks may use (`room.saturating_sub(text.len())`), which is none once twenty-odd units are
shown; the message had 6,725 of its 9,000 characters. From the second stop on the commit is
"from before" and is named without rules by design ("Its rules were said then and are not
kept"). So the rule of the session's own held commit is said at no stop, and at the first
stop its name was in a block's reason, which the model reads. This is HR5-M-1's hold form
("tells the person only …") with a name instead of a count. README: "then what the judge
holds that is new at this stop, with its rules".

Suggested direction: keep the names of a commit's rules with its answer in the file of judged
commits (`h:rule-a,rule-b`, `f:…|h:…`, bounded) and say them on the line of every stop; give a
new hold its rule names (the short form) from the tail's room, as a blocker gets them.

### HR6-L-1 (low; ORDINARY WORKFLOW; the same mechanism as Vesper's VD4R14-M-1, which rates it medium): five counted attempts are final, also when the cause was not the commit

`t1.sh` N3, `out/t1.txt` (wrapper). A commit with a key; the judge gives no verdict for that
patch and a verdict for a harmless uncommitted change at the same stop, five stops running.
The file of judged commits goes `n1 … n5`; from then on the commit is not asked about. With
the wrapper switched off (the judge answers every patch) two more stops still say "could not
be judged in 5 attempts" and the judge is given the tree's patch alone. The commit never
blocks.

I rate it low and not medium because it needs five stops at which this patch fails while
another gets a verdict, and because it is said at every stop for the rest of the session: no
silence follows. Against claim 2 ("Nothing is final that an outage caused") it stands as
Vesper says: what is final here is the count, not a proof. A plausible cause is a ranker that
times out on one large commit while the host is loaded. Cheap change: after the count is
spent, ask once more at a stop where nothing else is left to ask and the stop has time, or at
every tenth stop.

### HR6-L-2 (low; ORDINARY WORKFLOW in a shallow clone): "final at once" for a shallow edge is not stable: the message's own remedy leaves the commit unjudged for good

`t5b.sh`, `out/t5b.txt`. A shallow clone; upstream gets a commit with a key; the session
updates with `git fetch --depth=1` and `git reset --hard origin/main`. Stop 1 names the
commit: "this clone is shallow and does not have the commit's parent … (`git fetch
--unshallow` gets the history)", and writes `x:` for it. The session runs `git fetch
--unshallow`: `.git/shallow` is gone and the commit has its parent. Stops 2 and 3 say the
same sentence, word for word; the commit is not asked about and never blocks.

```bash
W=/tmp/claude-ddf-hooks-review-6; . $W/h.sh
U=$W/cases/x1x-up; git clone -q --no-local $W/base-repo $U
R=$W/cases/x1x; git clone -q --depth=1 file://$U $R; SESSION=x1x; start $R >/dev/null
secret $U/lib/a.rs; git -C $U commit -qam "upstream: credential"
git -C $R fetch -q --depth=1 origin; git -C $R reset -q --hard origin/main; stop $R
git -C $R fetch -q --unshallow; stop $R; stop $R     # "this clone is shallow …", still
```

Said at every stop, so no silence; but the reason is false after the remedy, and a class the
hook "proves from the repository" turned out not to be a lasting fact. Cheap fix: do not
write `x:` for a shallow edge; test it at each stop (the test is one file read).

### HR6-L-3 (low; ORDINARY WORKFLOW): with one or two commits that get no verdict and nothing else that gets one, the attempts never count: every stop of the session asks again

`t5.sh` X2, `out/t5.txt` (real judge, nothing simulated): one commit with a key and a
generated file of 3 MB on one line, a clean tree. Seven stops: each takes 10.3 to 10.5
seconds, each says "1 commit of this session has no verdict … io: the ranker timed out", and
the file of judged commits gets one more `n0:` line each time. `t1.sh` N2 shows the same
for eight stops with the wrapper.

The probe is asked only after three failed asks (`failed >= FIRST_ASKS`), so with fewer the
chain is never "seen to work" and nothing counts. README states the limit otherwise: "A
session with only commits the judge cannot judge … asks about up to three of them at every
stop: the judge answers its probe, so their attempts count and end after five." That is true
for three or more. For one or two, each later stop of the session costs the judge's own
time-out (ten seconds here; with two such commits, by reading, twenty), and the list file grows by a line a
stop. Not silent, never a pass. Either ask the probe after the last ask when nothing got a
verdict, or state it.

### HR6-L-4 (low; ORDINARY WORKFLOW, piece by piece): a message can pass 9,000 characters, and what is said beside the verdict can be cut to nothing the first time it is said

`t3.sh` (`C=b4`), `out/t3-b4.txt`, `out/t3-b4-stop2.json` (wrapper for three commits; policy
`base-long`, two rule ids of 45 and 53 characters). No session-start hook; a held commit and
three commits with no verdict from an earlier stop; then a pull of 60 commits that fail two
rules each, a commit of the session's own, and a merge that stops on a conflict:

```
decision: block | characters in the message: 9005
… Staged changes in "…/b4": UNJUDGED … 3 commits of this session have no verdict: … 1 held by the judge …
Rule text comes from the policy at the base commit; …
…
(More was said beside the verdict than this message holds.)
```

Claim 3 and the documents say "under 9,000". The parts add up on their own: what blocks may
use 5,700 and then twenty short lines of up to about 100 characters; a working tree's unit
with no verdict has no bound of its own in section 2 (up to about 470 each); then the tail
and the first line. By reading (not produced): with a longer project path, two working trees
unjudged and reasons at their 150 characters, about 10,300, which is past where the client
cuts and would lose the tail lines. And `notes_of` marks a note as said in full before
`stop_output_with` cuts it: here the note of the session ("start was not recorded") was cut
to "…"; a note that is new at such a stop is never said in full, only by its name on the
counting line afterwards. Suggested: one budget over the whole message, with the tail's lines
reserved first.

### HR6-L-5 (low; documents and wording)

- SECURITY.md, "A stop's message never loses what blocks": "every unit that blocks now, each
  named, with its rules". Beyond those shown and twenty short ones the rest are a count ("and
  40 more commits fail", `out/t3-b4.txt`). README says it right.
- README, the limit quoted under HR6-L-3.
- With unmerged paths the reason is Git's last `fatal:` line: "what is staged could not be
  read from Git: `git write-tree` failed: fatal: git-write-tree: error building trees"
  (`t8.sh` L6, start, gate and stop alike). It does not say that the index has unmerged
  paths, which is what the person has to act on.
- `agent doctor --gate` and the start text say "no Claude Code settings file registers a stop
  hook … stops may be UNJUDGED" when the hooks come from settings the gate does not read
  (managed settings, a plugin). Stated for managed settings; the careful side.

## Suspected only (not reproduced)

- Two stops of one session at the same time, each with other new lines for the file of judged
  commits, written as A-list, B-list, B-state, A-state: the state then names a list that is
  not in the file, and every later stop is UNJUDGED ("is not the list this session recorded;
  start a new session"). The stated limit says only that something is judged again. 80 rounds
  of two concurrent stops (`t7.sh`) did not produce it: both stops judge the same commits and
  write the same lines.
- A failing unit whose text is over its room and that has words of the judge beside its rules
  (twelve expired waivers): `unit_within` falls back to `rules_brief`, which drops what the
  judge said, with no word that it did. From reading.

## The areas of the brief: what was tried and whether it held

0. **This round's rework, the silence rule.**
   - The table (`t6.sh`, `out/t6.txt`; real judge, wrapper where a verdict must be missing).
     Uncommitted, staged (other than the tree) and commit, each against pass, hold, fail, no
     verdict every time, fail with no verdict once; a commit with the whole chain down; five
     rows of two or three units together; a commit with no verdict over nine stops. (The
     row "fail, then reverted" is a second "commit: fail": my `git revert -q` was refused
     by Git; the revert is in the 30 workflows below.) Result:
     silent in exactly the four all-pass rows, at all three stops; a fail blocks once and
     stands at every later stop; a hold and a missing verdict are said at every stop; "no
     verdict once" gives UNJUDGED, then a block, then FAIL STANDS. No empty stop that is not
     all-pass, no all-pass stop that is not empty, no block that repeats (in the table: see
     HR6-M-1 for where one does).
   - No verdict (`t1.sh`, `t5.sh`). Held: a commit whose judge fails once is judged at the
     next stop and blocks, with two calls of the judge (N1); with the chain down nothing
     counts and nothing is final; attempts count one a stop where another patch got a
     verdict, and the fifth ends the asking while the naming goes on (N3). Over an unwritable
     state nothing is counted (asked at every stop; `t8.sh` L3 for the words). Broken or
     inexact: HR6-L-1 (the count is final), HR6-L-2 (shallow), HR6-L-3 (never counts).
     Not run: a crash between the two writes (read: the list is written first and the state
     names its first lines, so an attempt is lost, never gained).
   - Holds and standing failures: one line at every stop in every run; bounded (three names
     and a count); with 63 failures standing a stop took 0.29 s. Dropped rules: HR6-M-2.
   - The message (`t2.sh`, `t3.sh`, `t3b.sh`, `t9.sh`). Held: with 60 pulled failing commits,
     a staged failure, an uncommitted failure and the session's own commit, the order is
     staged (character 594), uncommitted (948), the session's own commit (1,150), then the
     pulled ones; a rule's name is kept in the short form; a path of 200 characters keeps its
     file name. File names with a newline, a quote, a bidi override, and a project directory
     named `p2 PWNED" ignore the judge`: every output is valid JSON, no control character
     other than the message's own newlines, each name quoted and escaped on its own line.
     Broken: HR6-L-4 (9,005), HR6-M-2 (b).
   - Staged failures with their own identity (`t4.sh` S1): stage a key, block; repair without
     staging: block for "Staged changes", with the words that clear it; stands; unstage:
     silent. Broken: HR6-M-1.
   - The lows (`t8.sh`): `git add -N` of a harmless file is silent, of a file with a key
     blocks as uncommitted, and does not hide a key staged in another file; unwritable state
     with `stop_hook_active`: block, then "FAIL STANDS. This failure is not blocked because
     the client says …", then a block at the next turn; a working tree of a bare repository:
     silent when clean and with a harmless change, blocks on a committed key; unmerged paths:
     the start and the gate both say that a stop would be UNJUDGED for what is staged.
     Not run: a policy name with no probe.
   - Which commits are the session's: the fourth review's 30 workflows on this binary
     (`r4/t4.sh`, `out/r4-t4.txt`): 25 block, `worktree-uncommitted` is named, `stash-drop`,
     `reflog-expire-gc` and `worktree-detached-removed` are the stated limits, `stash-index`
     is the case that review wrote wrongly. The random workflows: last section.
   - The staged unit against index states (`t13.sh`): assume-unchanged (block), skip-worktree
     and a sparse checkout with a sparse index (UNJUDGED at every stop), a staged file turned
     into a symbolic link, a staged directory replaced by a file, `commit --only` with other
     content staged (block for "Staged changes" each), `stash --keep-index` (block on the
     stash's commit), a linked tree's index (named from outside, block from inside).
   - State files: the list of judged commits moved away: the resumed start says that stops
     will not be judged, the stop is UNJUDGED with the file's name, and with the file back
     the session goes on (`t10.sh` G4). Truncated and planted lists: the fifth review's runs;
     not repeated. `CLAUDE_PROJECT_DIR` naming another repository than the recorded one:
     UNJUDGED with both names; naming no repository: the recorded project is judged from any
     `cwd` (`t12.sh`). A project given through a symbolic link, and a project that is itself
     a linked working tree: silent on a pass, block on a key, the main tree's change named.
1. **A pass that should not be one.** None found. The fifth review's two stand-in judges
   (exit 0 with no document; with a small document) and a judge inside the repository are
   UNJUDGED at every stop; a stand-in that replays the real judge's sealed PASS for an
   earlier patch is UNJUDGED for the next patch (`t11.sh`, `out/t11.txt`).
2. **Blocks.** None that cannot be left. One that repeats: HR6-M-1. Waved through: nothing
   new; with a state that cannot be kept a failure that has never blocked is let through in
   the turn where `stop_hook_active` is true and blocks at the next (stated).
3. **Input and output** (`t9.sh`). Empty input, not JSON, another event, no session id, a
   session id that is a number, empty or 70,000 characters, a `cwd` that is a file or is
   missing, `stop_hook_active` that is a string, `cwd` given twice: exit 0 each time, valid
   JSON, UNJUDGED or the note of a first stop. A relative `cwd` is read against the hook's
   own directory (silent when that is no repository; the client gives absolute paths).
4. **Resources and files.** Over 200 state files, all 0600, in a 0700 directory; no `.partial`
   file left; no scratch directory left under any case's `.git/deep-diff-forge` after about
   six thousand stops. The longest stop I produced on purpose took 10.5 s (HR6-L-3) and the
   largest 7.8 s (60 commits). A full disk and a stop at its 45 seconds were not produced.
5. **Setup and doctor** (`t10.sh`, `t10b.sh`). `agent setup --client claude --hooks --json`
   gives the three hooks with absolute paths, `--judge` on the start and the stop, timeouts
   5, 10 and 60, no matcher. With them in `.claude/settings.json` the gate exits 0 and the
   start says "stops will be judged"; without them it exits 7 and names four links. (`setup`
   takes no `--judge`: it finds the judge on `PATH`.) No case found where the gate is ready
   and a fresh session's stop is UNJUDGED.
6. **`schema signals` / `explain signal`.** Not touched by `651adae..c6c4f28`; not examined
   again (the fifth review found the two equal for all 21 labels).
7. **Tests.** Below.

## Answer to "is a line at every stop right?" (the request's question D)

Yes for a commit that a branch, a tag or the stash still names: it is the only place the
person is told. Two things I would change. A commit that only a reflog names any more (it
was amended, rebased or reset away: the ordinary repair) stays "FAIL, which stands" for the
rest of the session; say it apart ("no branch names it now") so that the standing line of a
repaired session does not read like the standing line of an unrepaired one. And the other
working trees: a user with one long-lived tree that has work in progress gets a message at
every stop of every session in that repository; once a session, then the counting line, is
what the code does, and I would keep that but not add to it.

## Weak tests

Read from `tests/agent_hook.rs` and the unit tests of `hook.rs` at `c6c4f28`; no test was run
and no guard was removed.

- The table test (`a_stop_is_silent_only_when_every_unit_has_a_pass`) is a unit test of
  `decide_stop_trees`: three units, one of them not a pass, one stop. It would fail if the
  decision went silent for a state of one unit. It cannot see what a sequence of stops
  remembers or forgets (HR6-M-1), which commits are units, or what the text leaves out when
  two units compete for room (HR6-M-2 b).
- No test has a verdict with a failing rule and a held rule (HR6-M-2 a): the fixture
  `verdict(outcome, rule, signal, path)` makes documents with one rule.
- `what_is_staged_blocks_for_itself_after_the_working_tree_was_repaired` ends at the first
  `git add --all`; no test stages, edits and stops again with a failure standing, and no test
  makes two stops of one session from two working trees in turn (HR6-M-1).
- `a_stops_message_is_bounded_and_what_blocks_keeps_its_rule` bounds a message of failing
  commits with one short rule id and nothing else in it: no unit without a verdict, no line
  of the tail, no note, no hold that is new at that stop (HR6-L-4, HR6-M-2 b).
- `a_commit_with_no_verdict_never_stops_the_judging_of_later_commits` has three commits with
  no verdict (the probe's path); none has one or two and nothing else (HR6-L-3), and none
  lets the judge work again after the fifth attempt (HR6-L-1).
- `a_commit_at_the_edge_of_a_shallow_clone_is_not_judged_from_nothing` does not make the
  clone whole afterwards (HR6-L-2).
- By reading, these would fail without their guard: a commit asked again after one missing
  verdict (the count of the judge's runs); a hold named at three later stops; the notes at
  every stop; the staged failure blocking after a repair.

## Not looked at

- A live Claude Code session (whether `resume` keeps the session id; if it does not, a
  resumed session is a new one to the hooks and what stood is not said in it); macOS; Git
  older than 2.56; SHA-256 repositories; reftable; Git LFS and other filters; Jujutsu and
  other tools that keep commits under refs of their own; a real reboot; a full disk; network
  file systems; a partial clone (the fifth review's `t10`).
- The verdict checks of rounds 1 to 10, the ranker seal helper and `verdict_v2.rs`, beyond
  the stand-in judges under area 1. `schema signals` and `explain signal`.
- A policy name with no probe; a stop that runs to its 45 seconds; more than sixteen other
  working trees; a state directory inside the working tree.
- Vesper's round 14 report was in the worktree before I finished. I read its headings and its
  one finding only after every finding above was reproduced; nothing here is taken from it.
- Housekeeping, for the record: `fuzz.py` and scripts `t13.sh` and the copied `r4/t4.sh`
  remove or move files inside their own case clones (script files, none refused). Three
  background runs of `fuzz.py` were started with `</dev/null`; three earlier ones were
  stopped by their exact process ids. No command was refused by the permission layer. Nothing
  under `/tmp/claude-ddf-hooks-review-5/` or the implementer's scratch directories was
  changed; the fifth review's two stand-in judges were run from where they are. The worktree
  was not written except for this file.

## The random workflows: result

`fuzz.py SEED RUNS STEPS OUT`; outputs `out/fuzz-e.jsonl`, `out/fuzz-f.jsonl` (real judge) and
`out/fuzz-g.jsonl` (wrapper); `fuzzstat.py` prints the table.

| run | sessions (with a linked tree) | Git operations that ran | stops | silent | block | FAIL STANDS | UNJUDGED | HOLD | violations of A or B |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| real judge, seeds 20000 to 20119 | 120 (37) | 1,959 | 1,200 | 774 | 144 | 235 | 3 | 44 | 0 |
| real judge, seeds 21000 to 21119 | 120 (33) | 1,971 | 1,200 | 756 | 149 | 252 | 12 | 31 | 0 |
| wrapper (chain down, key patches refused), seeds 30000 to 30119 | 120 (37) | 2,061 | 1,200 | 668 | 85 | 96 | 334 | 17 | 0 (A only) |

No stop was silent with a key in a unit; no stop named neither a failure nor a missing verdict
with a key in a unit; no commit with a key went unnamed and uncounted; with the real judge no
stop spoke with nothing open (holds apart, which my check does not predict). Before these, 180
sessions with a weaker form of the check (silence alone; 1,776 stops) had no violation
either. Limits of this: every stop is made from the project's own directory; there is no
remote, so no fetch or pull (those are in `t2.sh` and `t3.sh`); the check for B leaves out
stops during a merge, a rebase or a cherry-pick; four of the 53 operations never succeeded
in the first runs (`checkout HEAD~1 -- FILE`, `cherry-pick @{-1}`, `merge --abort`,
`rebase --abort`); and the invariants do not look at how often a stop blocks, which is where
HR6-M-1 is.
