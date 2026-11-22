# Fifth independent review of the agent hooks branch (`agent/loop`, commit `651adae`)

Date: 2026-11-14. Reviewer: an independent Claude agent that took no part in the work.
Brief: `~/.local/state/habitat-claude/ddf-gap-probes-20261113/ddf-hooks-independent-review-5-brief.md`.
Under review: `d4dbc91..651adae`, mainly the rework `f9d1c9d..651adae` that answers the fourth
independent review (`docs/reviews/claude-independent-review-hooks-4-20261114.md`).

## Verdict: NO-GO

The rework does what it set out to do for the cases it was given. With the real judge: staged
content that is no longer in the working tree blocks (HR4-H-1); with fourteen failures standing
a new failing commit is named with its rule and a commit with no verdict is said (HR4-H-2);
three commits with no verdict no longer end the judging (HR4-M-2); an unwritable state no
longer blocks for good (HR4-M-3); the documents now say which commits of others are judged
(HR4-M-1). All of the fourth review's scripts were run again on this binary and none got worse.

What stands against a merge:

- **HR5-H-1 (high by the rule the documents state; new in this commit).** A commit that gets
  no verdict while the chain works is said at one stop and remembered as "not judgeable" for
  the rest of the session. Every later stop is silent, with a commit of the session's own work
  in the project that no judge has passed. The cause need not be the commit: one stall of the
  ranker is enough. And when the stop that first asks is a block for something else, the only
  mention is inside the block's reason.
- **HR5-M-1 (medium; a regression of this commit).** A unit whose own rule text is longer than
  the room the message has is shown with no rule at all. One uncommitted change that one rule
  names in 26 files blocks with a reason of 680 characters that names no rule and no path; a
  hold of the same kind tells the person only "1 more that were held or stand are not shown:
  this message is full", in a message of 382 characters. The commit before showed the first
  6,000 characters.
- Three more mediums (HR5-M-2, M-3, M-4) and seven lows, below.

Every finding is marked ORDINARY WORKFLOW or DELIBERATE EVASION. No finding of this round is a
deliberate evasion; HR5-L-3 has an evasion form, which is noted there.

## How this was done

- Source read from `git archive 651adae` (extracted to `/tmp/claude-ddf-hooks-review-5/src`).
  No `cargo` was run. No `toolbox`, `podman` or `distrobox` call was made.
- Binary: `/var/home/louranicas/.cache/ddf-t/review-bin/v4r13/release/deep-diff-forge`
  (SHA-256 `02b953f2…0874`). Judge: the real forge-warden 0.2.0,
  `/var/home/louranicas/.cache/ddf-t/target-fw/release/forge-warden` (SHA-256 `55bf9fe6…74c9`),
  run on the host. Git 2.56.0. Three comparisons use the commit before
  (`review-bin/v4r12/release`, with a base whose policy admits it); they are marked.
- Scratch: `/tmp/claude-ddf-hooks-review-5/`. `base-repo` is a `git clone --no-local` of the
  fourth review's base repository with the ranker digest set to the frozen binary and
  committed (`4ff0bdb`). Every case is a fresh clone under `cases/`. Scripts `t0.sh` … `t13.sh`,
  helper `h.sh`, outputs in `out/`. The fourth review's scripts, copied with paths rewritten and
  the read of the judged commits changed to the file beside the state, are in `r4/`, their
  cases in `r4cases/`, their outputs in `r4out/`. `/tmp` does not survive a reboot, so the
  commands that matter are repeated below.
- Every run had a private `HOME` set inside the helper (`home`, `home-ro`, `home-gate`,
  `home-r4` under the scratch directory), `XDG_STATE_HOME` and `CLAUDE_PROJECT_DIR` unset,
  `GIT_CONFIG_GLOBAL=/dev/null` unless a case sets its own. The real
  `~/.local/state/deep-diff-forge/hooks` is empty and was last touched 20:04, before the first
  run of this review (20:06).
- The helper's commands (`$S` a session id, `$R` a directory) are the fourth review's:
  `start R`, `post R`, `stop R [true]` write the hook input for `SessionStart`, `PostToolUse`,
  `Stop` (with `stop_hook_active` as given) to `$DDF agent hook … --client claude --judge $FW`;
  `stopr R` is a stop with a recording stand-in that gives no verdict, used only to show which
  patches a stop makes; `secret FILE` writes two functions and
  `pub const K: &str = "AKIAIOSFODNN7EXAMPLE";`; `harmless R` writes a third function into
  `lib/a.rs` and two tests into `tests/t.rs`.
- One reproduction is a simulation and is marked: a ranker process stopped for eleven seconds
  with `SIGSTOP` (by exact PID, found through `/proc`), to stand for a stall of the machine.
- What Claude Code does (when `stop_hook_active` is true, whether it shows a block's reason to
  the user, where it cuts text) is taken from its documentation and from the branch's own
  statements, not from a live session.

## Findings reproduced with the real judge

### HR5-H-1 (high; ORDINARY WORKFLOW; new in this commit): a commit that got no verdict is said once and then never again; later stops are silent with unjudged work of the session

`judge_commits` remembers a commit as `u` when it got no verdict and "the chain works" (some
patch got a verdict at this stop, or the judge answers the probe). A remembered commit is
passed over at every later stop (`Some(_) => continue`), so nothing names it again. The commit
before asked about it, and said UNJUDGED, at every stop (`t5b.sh`, v4r12 binary: three stops,
three times "Commit 115aa4ac8bb5: UNJUDGED").

**(a) The session's own commit, nothing simulated** (`t5c.sh`, `out/t5c.txt`). One commit holds
a credential in `lib/a.rs` and a generated file of 3 MB on one line (the shape of the fourth
review's N3), on which the judge's ranker times out:

```
 -- stop 1:  {"systemMessage":"deep-diff-forge: UNJUDGED. Not everything of this change (…) was judged.\nCommit 752904172d99: UNJUDGED\n  not judged: the judge could not read its inputs or its environment (exit 3): io: the ranker timed out.\nThat is not a pass."}   (10.6 s)
 -- stop 2:  [exit 0] (NO OUTPUT)   (0.12 s)
 -- stop 3:  [exit 0] (NO OUTPUT)
   judged file: 752904172d990a331120906031bb4be4d9cedeb7:u   credential lines in the commit: 1
```

```bash
W=/tmp/claude-ddf-hooks-review-5; . $W/h.sh
R=$(fresh k5b); SESSION=k5b; start $R >/dev/null
secret $R/lib/a.rs; mkdir -p $R/dist; head -c 3000000 /dev/zero | tr "\0" "a" > $R/dist/app.min.js; echo >> $R/dist/app.min.js
git -C $R add -A; git -C $R commit -qm "build: bundle, and a key"
stop $R      # UNJUDGED, said
stop $R      # -> [exit 0] (NO OUTPUT)
```

The same change left uncommitted is UNJUDGED at every stop (the uncommitted unit is asked
every time). Committing it makes the hooks quieter. `t5.sh` K1 is the same with a file name
that is not UTF-8.

**(b) Said only to the agent** (`t5.sh` K2, `out/t5.txt`). The unjudgeable commit and, at the
same stop, a credential in the working tree: stop 1 is a block whose reason holds
"Commit cdc5c64bc5da: UNJUDGED"; the tree is repaired; stop 2 and stop 3: no output. By the
documents' own model a block's reason is what the model reads and `systemMessage` is what the
user is shown, so the user is never shown that the commit was not judged.

**(c) The cause is not the commit** (`t4.sh`, `out/t4.txt`; real judge, the stall simulated).
A commit with a credential and a text file of 150,000 lines blocks in 0.37 s in the control.
With the ranker of the first judge run stopped once for eleven seconds:

```
 -- stop 1:  … Commit 9caefb5a5bc4: UNJUDGED\n  not judged: … io: the ranker timed out.   (10.4 s)
   stall: stalled pid 2579771 (child of the judge, pid 2579724) … argv=['/proc/self/fd/4','--stdin-patch','--rank','--json'] for 11.0s
   judged file: 9caefb5a5bc4b94dbec3a1782ac5835a8b0b0f1e:u
 -- stop 2:  [exit 0] (NO OUTPUT)
 -- stop 3:  [exit 0] (NO OUTPUT)
```

The probe answered a moment later, so "the chain works" and the answer was read as the
judge's word on this patch. "About this patch or about the chain" is decided by whether the
chain works afterwards, so every failure that passes (a stalled machine, a judge that could
not start its ranker once, a scratch write that failed once) is taken for a property of the
commit, for good. From reading, not run: once any patch has a verdict at a stop the chain
counts as working for the rest of that stop, so an outage that begins in the middle of a stop
with many commits marks every commit asked about during it.

**(d) A partial clone** (`t10.sh`, `out/t10.txt`; nothing simulated). In a clone made with
`--filter=blob:none`, five commits pulled during the session (one adds a credential, the next
removes it) are each `u` at the first stop (the blobs of commits in between are not in the
clone, and the hooks rightly do not fetch) and the second stop is silent. README: "Commits
fetched during the session and brought onto a local branch are the session's and are judged".

Stated? In one place: "That commit is said once, by its id, as UNJUDGED with the reason, is
remembered as not judgeable". Against it, in the same section: "Nothing is said in three cases
only: … nothing is uncommitted and every commit of the session has passed", and the rule at
its head: "an ordinary workflow must never end in a silent stop with unjudged work". The
statement of the limit is accurate; the limit is on the wrong side of the rule. The test
`a_commit_with_no_verdict_never_stops_the_judging_of_later_commits` asserts the silence
(`!stands.contains("UNJUDGED")`).

Suggested direction: keep such commits as standing, like failures ("2 commits of this session
have no verdict: Commit …"), at every stop, in one line; ask again about a `u` commit at a
later stop at least once (or whenever the reason was a timeout or an `io` answer and the stop
has time), and remember it for good only when the second answer is the same; never let the
only mention be inside a block's reason: say it again at the next stop that is allowed.

### HR5-M-1 (medium; ORDINARY WORKFLOW; a regression of this commit): a unit whose own rules are longer than the message has room for is shown with no rule at all

`trees_text` adds a unit's text only when the whole of it fits under 6,000 characters. A unit
that does not fit is a name with "(its rules are not shown: this message is full)" when it
blocks, and is counted as "left out" when it is held or stands. Nothing of its text is shown,
also when it is the only unit and the message is otherwise empty. One rule shows up to twenty
violations of about 220 to 340 characters each, so one unit passes 6,000 with one rule and
long paths, or with two rules.

Reproduced (`t12.sh`, `out/t12.txt`): one uncommitted change adds fixture files that each hold
a key-like value, under a path of about 120 characters.

```
== X-12:  decision: block | characters: 4408 | lines that name a rule: 1 | lines that name a path: 12
== X-26:  decision: block | characters: 680  | lines that name a rule: 0 | lines that name a path: 1
   after the first line: 'Uncommitted changes in "…/cases/x26": FAIL (its rules are not shown: this message is full)\n'
 -- X-26, stop 2:  characters: 422 | after the first line: '1 more that were held or stand are not shown: this message is full.\n'
```

The block tells the agent "What the rules name in this session's own work is this session's
to repair" and names no rule and no file. The stop after it does not even name the unit. On
the commit before (`t12b.sh`, v4r12): `decision: block | characters: 6431 | lines that name a
rule: 1 | lines that name a path: 20`.

The hold form (`t12c.sh`, `t12d.sh`): one uncommitted change in which `#[allow(dead_code)]`
and an `unsafe` block are added in 22 files each (short paths). The whole output:

```
{"systemMessage":"deep-diff-forge: HOLD. The judge wants a person to look at this change (…).\n1 more that were held or stand are not shown: this message is full.\n"}
```

With 16 files each the same message has 5,767 characters and names both rules. "A check it
silences" in many files is the kind of shortcut the hooks are for; the larger the shortcut,
the less is said.

```bash
W=/tmp/claude-ddf-hooks-review-5; . $W/h.sh
R=$(fresh xh22); SESSION=xh22; start $R >/dev/null
for i in $(seq -w 1 22); do printf '#[allow(dead_code)]\npub fn s%s() {}\n' $i > $R/lib/s$i.rs; printf 'pub fn u%s(p: *const u8) -> u8 { unsafe { *p } }\n' $i > $R/lib/u$i.rs; done
stop $R
```

Also seen in X-12: a path is cut at 120 characters from the front, so the twelve lines show
the same text (`…/sample-key-material-number-…`): the part that tells the files apart is the
part cut off.

Suggested direction: cut a unit's text to the room there is (as many rules and violations as
fit, then "and N more, not listed"), never to nothing; a unit that is the only one always gets
the whole room; show the end of a long path as well as its start.

### HR5-M-2 (medium; ORDINARY WORKFLOW): many commits that fail at one stop: the message has no upper bound, and the session's own commit comes last, without its rule, then past where the client cuts

Parts 1 and 2 of the message (what blocks now, what got no verdict) are "always whole". The
blocking commits are listed the oldest first; their rules are shown until the message reaches
6,000 characters and each later one is a line of about 73 characters. The commits a pull or a
rebase brings are older than the commit the session makes on top of them.

Reproduced (`t3.sh`, `out/t3.txt`, `out/t3-m150-stop1.json`): the session pulls N upstream
commits fetched during the session, each of which fails `tests-travel-with-code`, then commits
a credential of its own; one stop.

```
== M-30:   decision: block | characters: 5528  | the session own commit … named at character 5111, with its rule
== M-150:  decision: block | characters: 14875 | the session own commit 872da0e4b337 is named at character 14693:
           'Commit 872da0e4b337: FAIL (its rules are not shown: this message is full)'
           the rule no-secrets is in the message: False | lines saying rules are not shown: 112
 -- stop 2: … 151 earlier failures stand: Commit dacff568671c, Commit 461ff385a98a, Commit 1c9d0dbdb382, and 148 more.
```

From about 40 failing commits at one stop the session's own commit is named without its rule
(the agent is told that its commit fails and not why, and the next stop names only the first
three of 151). From about 90 its name lies past 10,000 characters, where `hook.rs` itself says
Claude Code cuts ("Claude Code cuts injected text at 10,000 characters"; not checked against a
live client). A rebase of a long branch gives the same shape: every replayed commit is the
session's. This is HR4-H-2 at a larger number; no test has more than fourteen units.

Suggested direction: group blocking commits by their failure (one rule text, then the ids);
or list the newest first; bound parts 1 and 2 (first and last few names and a count) and keep
room for the rules of the newest.

### HR5-M-3 (medium; ORDINARY WORKFLOW): a staged failure inherits "has already blocked" from the working tree, so the stop after an incomplete repair is allowed and the agent is not told

The failures that have blocked are kept as digests of rule, signal, path and count, for the
tree's and the index's units together. The most ordinary sequence there is: `git add -A`,
stop, block, repair the file, stop.

Reproduced (`t1.sh` S1, `out/t1.txt`):

```
 -- stop 1 (false):  {"decision":"block", … - rule no-secrets … secret_like in "lib/a.rs" …}      (the tree; index and tree are one patch)
 -- git status after the repair:   MM lib/a.rs / M  tests/t.rs
 -- stop 2 (true):   {"systemMessage":"deep-diff-forge: FAIL STANDS. This failure has already blocked a stop of this session (…); it is left for a person.\nStaged changes in \"…/s1\": FAIL, which stands\n- rule no-secrets …"}
   decision block in stop 2: 0     the index holds the credential: 1   the working tree: 0
 -- stop 3 (false):  the same, no block
 -- what a plain 'git commit' records now:   credential lines in the new commit: 1
```

The agent did what the block asked, and the one thing left to do (`git add`) is never put to
it: the user is told that the failure "is left for a person". The request's sentence "a
credential staged and then removed from the file blocks" holds only when that credential has
not already blocked in the tree (S3, control: block). S2 is the same when the staging happens
after the first block. It is said to the user, so it is not high. From reading: the same
digest is shared between the project's tree and a linked working tree the session enters.

```bash
W=/tmp/claude-ddf-hooks-review-5; . $W/h.sh
R=$(fresh s1); SESSION=s1; start $R >/dev/null
secret $R/lib/a.rs; git -C $R add -A; stop $R        # block
harmless $R; stop $R true                             # FAIL STANDS, no block; git diff --cached still has the key
```

Suggested direction: keep what has blocked per unit (the digest with "tree" or "index" and the
tree's path in it), so a failure that moves from the tree into the index alone blocks once
more, with a reason that says the index still holds it.

### HR5-M-4 (medium; ORDINARY WORKFLOW; older than this commit): a commit the judge holds, judged at a stop that blocks for something else, is never reported to a person

A held commit is remembered (`h`) and passed over afterwards. When the stop that judges it is
allowed, the hold is a `systemMessage` (K4: "HOLD … Commit 60bceb733438: HOLD"). When that stop
blocks, the hold is a part of the block's reason and nothing else.

Reproduced (`t5.sh` K3, `out/t5.txt`): a commit that silences a lint, and at the same stop a
credential in a new file of the working tree:

```
 -- stop 1:  {"decision":"block", … Uncommitted changes in "…/k3": FAIL … Commit a68bcde07dfa: HOLD\n- rule no-silent-suppression …}
 -- stop 2 (true), after the file is removed:  [exit 0] (NO OUTPUT)
 -- stop 3:                                     [exit 0] (NO OUTPUT)
```

README: "A hold is reported for a person." The v4r12 binary does the same (`t5b.sh`), so this
is not new; no earlier review names it. Cheap fix: a held commit stays in the message as
standing until one allowed stop has shown it.

### HR5-L-1 (low; ORDINARY WORKFLOW): an intent-to-add entry is read as a staged empty file, and blocks

`git ls-files --stage` lists a path added with `git add -N` like a staged empty file;
`git commit` records nothing for it. The staged unit judges a new empty file.

Reproduced (`t7.sh`, `out/t7.txt`): a new `src/new.rs` with its test `tests/n.rs`;
`git add -N src/new.rs` (so that `git diff` shows it); nothing else staged
(`git commit --dry-run` exits 1: nothing to commit):

```
 -- stop: {"decision":"block", … Staged changes in "…/n1": FAIL\n- rule tests-travel-with-code … "src/new.rs" …}
 -- control, without git add -N:  [exit 0] (NO OUTPUT)
```

A false block that names something that is not staged; it blocks once. Fix: build the staged
tree from the index itself (`git write-tree` on a copy of the index file leaves such entries
out), or drop the entries `git diff-index --cached` does not report.

### HR5-L-2 (low; ORDINARY WORKFLOW for a policy with such a name): with no probe, one commit with no verdict still ends the judging of every later commit, at every stop

`probe_patch` gives no probe for a policy whose name has a character outside
`[A-Za-z0-9._/-]`. Then the chain "works" only when another patch got a verdict at the stop.

Reproduced (`t8.sh`, `out/t8.txt`; policy `conf/warden@team.toml`, `--policy` given): a commit
with a file name that is not UTF-8, then a commit with a credential, clean tree. Three stops,
each: "2 commits of this session: UNJUDGED … patch parse failed: invalid, non-UTF-8 or unsafe
Git path"; never a block; the judged file stays empty. The control with `warden.toml` blocks
at the first stop. Never silent. With the same policy name the gate and the start say "The
chain is not ready, so stops may be UNJUDGED: chain: the policy is empty, or its name cannot
stand in a patch header" while a stop is judged and blocks (P0): the reverse of a wrong
"ready". Fix: quote the name in the probe's header as Git does, or probe with a path that is
always plain.

### HR5-L-3 (low; ORDINARY WORKFLOW when the state cannot be written; the evasion form is cheap to see): with an unwritable state and `stop_hook_active`, a failure that has never blocked is allowed, with words that say it has

Reproduced (`t6.sh`, `out/t6.txt`). D1 is the fourth review's case and holds: block, FAIL
STANDS twice, block again at the next turn, as documented. D2: stop 1 blocks on a credential
in `lib/a.rs`; the agent repairs it and, continuing, adds a credential in `lib/other.rs`; stop
2 has `stop_hook_active: true`:

```
{"systemMessage":"deep-diff-forge: FAIL STANDS. The hook cannot keep its state, so it cannot remember that this failure has blocked a stop. … What was said when it blocked:\n… secret_like in \"lib/other.rs\" …"}
```

Nothing was ever said to the agent about `lib/other.rs`. With a state that can be kept the
same stop blocks (control). D3: the same when the first stop of a turn already has the flag
set, as after another stop hook of the user's has blocked. The README states the rule
accurately; the message's "What was said when it blocked" is not true of a new failure. How an
agent reaches this state without setting out to: a full disk, a read-only home, a state
directory owned by another user. As an evasion (`chmod 500` on the state directory) it is
visible: every stop says that the state could not be kept.

### HR5-L-4 (low): Git's own hint hides the reason a commit could not be read

Every Git call runs with `GIT_GRAFT_FILE=/dev/null`. Git 2.56 answers a graft file that exists
with a hint of 311 bytes on its error stream ("hint: Support for <GIT_DIR>/info/grafts is
deprecated …") before the error itself, and a unit's reason is cut at 300 characters.
`out/t10.txt`: the reason shown for each commit is that hint; the cause
(`fatal: unable to read 4854d7fd…`, `out/pc-stderr.txt`) is never shown. README: "UNJUDGED
says why". Fix: `-c advice.graftFileDeprecated=false`, or keep the last lines of Git's error
stream instead of the first.

### HR5-L-5 (low): a bare repository's own entry is said to have changes that are not committed

`other_trees` asks `git status` in every entry of `git worktree list` that is "usable"; for
the bare repository itself that fails, and a failure counts as changes. Reproduced (`t9.sh`
G2): a bare repository with one linked working tree, the session in the tree, nothing changed:

```
{"systemMessage":"deep-diff-forge: The working tree \"…/cases/g2.git\" of this project has changes that are not committed, and the session's directory is not in it: they were not judged …"}
```

Once in a session; not true.

### HR5-L-6 (low): the gate and the start say "stops will be judged" where a stop is UNJUDGED

`t11.sh`, `out/t11.txt`, with the three hooks registered in a private `~/.claude/settings.json`:

- GT2: an index with unmerged paths (a merge that stopped on a conflict). `agent doctor --gate`:
  every link ready; the start of a new session: "stops will be judged"; the stop: "Staged
  changes in …: UNJUDGED … `git write-tree` exited with status 128: lib/a.rs: unmerged". The
  link `patch` makes the tree's patch and not the index's.
- GT3: a resumed session whose list of commits is gone. The start says "stops will be judged";
  the stop is UNJUDGED with the remedy. This is the fourth bullet of HR4-L-1 and is still open.

### HR5-L-7 (low): statements that do not match the code

- README "Nothing is said in three cases only … every commit of the session has passed": also
  nothing is said when a commit of the session is remembered as held or as not judgeable
  (HR5-H-1, HR5-M-4).
- The start text and every message: a commit that was already there "is not judged at a stop,
  also when it is pulled or merged in". A merge that makes a merge commit is judged against
  its first parent, with what it brought in (the fourth review's A4 blocks; README says
  "merges it by fast-forward", which is exact). The short form says less than is done.
- README "named by any ref … or by a reflog", start text "already in this repository": the
  list holds what refs, the `HEAD`s and the reflogs of the `HEAD`s and of the stash lead to.
  From reading, not run: a commit that only a branch's own reflog names, or a dangling one
  (after `git stash drop` before the session), is the session's when the session brings it
  back, and is judged. Inexact on the careful side.
- The block's "The same failure does not block twice": a failure that blocked uncommitted
  blocks once more when it is committed (by design since this commit; README says so).
- The message of HR5-L-3.

## Findings that need no judge

- HR5-L-4 (the hint on Git's error stream: `out/pc-stderr.txt`, made by hand with the hook's
  environment), HR5-L-5 and the gate half of HR5-L-6 involve no verdict.
- The patches behind HR5-L-1 were first shown with the recording stand-in (`t7.sh`: the tree's
  patch holds two files, the index's one), then judged by the real judge.

## Suspected only (not reproduced)

- **An outage in the middle of a stop** marks every commit asked about during it as not
  judgeable (HR5-H-1 (c), read from `Judge::judge` and `chain_works`).
- **Two stops of one session at once** can leave a state that names a list of judged commits
  which is not the file's (each writes the file whole, then its state): the session is then
  UNJUDGED until a new one starts. The stated limit says "a commit is then judged again, or a
  failure blocks again". Read from `Session::keep`; not run. Never a pass.
- **The guard of three seconds** in the `u` arm of `judge_commits` (`left() >= COMMIT_FLOOR`)
  has no test I can find: without it a commit whose judge was ended by the stop's deadline
  would be remembered as not judgeable. Read; no guard was removed and no test was run.
- **Resume with a new session id** (carried over from the fourth review): commits made before
  a crash and not yet judged would be "there before". Not checked against a live client.
- **Very large repositories**: `git add --all` over the whole tree at every hook, a second
  pass over the index for the staged unit, and `rev-list` with every recorded tip as a
  boundary at every stop. Not measured.

## The fourth review's cases on `651adae`, with the real judge (`r4/`, `r4out/`)

| Fourth review | Now |
| --- | --- |
| HR4-H-1 I1, I2 (staged, then repaired or deleted) | block, "Staged changes in …: FAIL" (`r4out/t5.txt`, and `t0.sh` C4) |
| HR4-H-2 `t6`, `t6b`, `t6c` (13 or 14 standing, then a new commit) | the new failing commit is named with its rule; the commit with no verdict is said by its id |
| HR4-M-1 A1, A2, A3, A5, A6 | silent, as now documented in the start text and in every message; A0, A4, A8 block |
| HR4-M-2 N1, N2, N3 | block at the first stop that sees the credential commit; N3: one stop of 30.8 s, then 0.12 s |
| HR4-M-3 `t7` | block, then FAIL STANDS three times |
| HR4-L-1 (`log.showSignature`; a worktree removed by hand) | the session is recorded; a commit blocks; the gate has the link `commits` |
| HR4-L-2 (shallow) | "UNJUDGED" by id with the reason, once; then silent (HR5-H-1) |
| HR4-L-3 (4,096 verdicts) | the judged commits are a file with no bound; a planted line after the ones the state names is not believed (`t13.sh` J1) |
| HR4-L-4 (B2) | the commit blocks |
| HR4-L-5 (a working tree the session is not in) | named once |
| the sweep of 30 workflows (`t4`) | 26 block, one is named (`worktree-uncommitted`), `stash-drop` and `reflog-expire-gc` are the stated limits, `stash-index` is the case the fourth review wrote wrongly |
| a pull of 450 harmless commits, then the session's own (`t8`) | one stop of 35.2 s judges all 451 and blocks on the session's commit |
| the third review's thirteen cases (`t15`) | unchanged |

## The areas of the brief: what was tried and whether it held

0. **This round's rework.**
   - The staged unit (`t2.sh`, `out/t2.txt`). Held: `update-index --cacheinfo` with a
     credential blob that was never in the tree (block); part of a file staged (block);
     `git stash --keep-index` (block, on the tree and on the stash's commit); a symlink, a
     mode-only change and a deletion staged and undone in the tree (one patch of three files,
     HOLD); a submodule entry staged (HOLD, the judge's note on mode 160000); assume-unchanged
     (the tree is judged all the same, block); skip-worktree (UNJUDGED, said); unmerged paths
     (UNJUDGED at every stop, said); the index of a linked worktree the session is in, by
     `cwd` at the start or at the stop (block); the index of a linked worktree the session is
     not in (named once, then silent: the stated limit). Broken: intent-to-add (HR5-L-1) and
     the shared memory of what has blocked (HR5-M-3). Not run: a sparse index, a very large
     index.
   - The message. Broken: HR5-M-1 (one long unit), HR5-M-2 (many units). The first line was
     the worst new answer in every run.
   - No verdict. Broken: HR5-H-1, HR5-L-2. Held: a chain that gives no verdict ends the
     asking for that stop and remembers nothing (`r4out/t3.txt`, and P1 shows the same path).
   - `stop_hook_active` with a state that cannot be kept: D1 holds; HR5-L-3.
   - Commits that were already there. No ordinary workflow was found in which the session's
     own work counts as there before: two sessions in one checkout each judge their own
     commits and each other's later ones (`t13.sh` E1: B blocks on its commit; A blocks on
     both); the statement is inexact only on the careful side (HR5-L-7).
   - The judged file and state v4 (`t13.sh`): a planted line is not believed; a file cut
     short, emptied or gone, or shorter than the state says, is UNJUDGED with the remedy;
     restored, the session goes on.
   - Shallow clones: the fourth review's case; then HR5-H-1 applies. The sixteen other trees:
     one with changes is named once; HR5-L-5.
   - A commit made inside a submodule with the project's pointer moved to it (`t9.sh` G1): the
     real judge holds a change of mode 160000 with a note, staged and committed, so it is not
     silent; from inside the submodule the stop names it as another repository.
1. **A pass that should not be one.** None found beyond HR5-H-1. The fake judges of the fourth
   review (exit 0 with no document, with a small "pass" document, a judge inside the
   repository) are each UNJUDGED (`r4out/t13.txt`).
2. **Blocks.** No block that cannot be escaped was found. A false block: HR5-L-1. A block that
   names nothing: HR5-M-1. Waved through: HR5-M-3, HR5-L-3.
3. **Input and output.** The fourth review's six inputs (empty, not JSON, wrong event, no
   session, a relative `cwd`, 1 MiB of random bytes) each give exit 0 and valid JSON saying
   UNJUDGED. Every output of every run parsed as JSON or was the plain start text. Message
   size: HR5-M-2.
4. **Resources and files.** No scratch directory was left under any case's
   `.git/deep-diff-forge`. State files are 0600 in a 0700 directory. A full disk was not
   produced.
5. **Setup and doctor.** `agent setup --hooks` prints the three hooks with absolute paths,
   `--judge` and the timeouts 5, 10 and 60. With them registered the gate is ready and the
   start says "stops will be judged" on a clean repository (GT1). HR5-L-6, HR5-L-2.
6. **`schema signals` / `explain signal`.** 21 signals; the members both give (`basis`,
   `class`, `kind`, `label`, `sides`, `since`, `summary`, `weight`) are equal for every label;
   every label is a string literal in `risk.rs` or `evidence.rs`; the weights of the path
   signals equal `RiskSignal::weight`; an unknown label exits 2 and names `schema signals`.
7. **Tests.** Below.

## Weak tests

Read from `tests/agent_hook.rs` and the unit tests of `hook.rs` at `651adae`; no guard was
removed and no test was run.

- **Would fail if the guard were removed** (by reading): the staged unit
  (`what_is_staged_and_no_longer_in_the_working_tree_is_judged` asserts "Staged changes in …:
  FAIL" for I1, I2 and a linked tree, and one run of the judge when index and tree agree);
  the order of the message for fifteen units; a `u` commit not asked about again (the count
  of the judge's runs); a chain with no verdict asking twice and remembering nothing; the
  unwritable state with `stop_hook_active`; the judged file held against the state; the
  three refs as a source (`plumbing_commit`).
- **No test has a unit whose own text is over the cap** (HR5-M-1). The unit test of
  `trees_text` checks that held units past the cap are counted; none asks that a unit that
  blocks, or the only unit, shows any rule.
- **No test has more blocking units than fit** (HR5-M-2), and none bounds the length of a
  message with several units (`rule_text_escapes…` bounds one hold of one rule).
- **No test stages after a block** (HR5-M-3): in the staged test the staged failure is always
  the first to block in its session.
- **The silence after a `u` commit is asserted, not questioned** (HR5-H-1), and no test has a
  commit that gets no verdict once and a verdict when asked again.
- **No test has a held commit at a stop that blocks** (HR5-M-4).
- **No test has an intent-to-add entry or unmerged paths** (the request says the second is not
  tested), a policy name with no probe, or a bare repository in `other_trees`.
- **The guard of three seconds** in the `u` arm: see "Suspected only".

## Not looked at

- A live Claude Code session; macOS; Git older than 2.56; SHA-256 repositories; reftable; Git
  LFS and other filters; a sparse index; Jujutsu and other tools that keep commits under refs
  of their own; a real reboot; a full disk; network file systems.
- The verdict checks of rounds 1 to 10, the ranker seal helper and `verdict_v2.rs`, beyond the
  fourth review's three fake judges.
- Vesper's round 13 report, which appeared in the worktree while this review ran: not read.
- `cargo` was not run: no test suite, no mutation run. The statements under "Weak tests" are
  from reading.
- Housekeeping, for the record: scripts `t2.sh` and `t5.sh` (and the fourth review's `t4.sh`,
  `t5.sh`, `t13.sh`, copied) remove a file inside their own case clone with `rm` (script
  files, not inline commands; none was refused). `tools/stall.py` sent `SIGSTOP` and `SIGCONT`
  to one process, the ranker child of the judge this review had started itself. Nothing under
  the example scratch directory or `/tmp/claude-ddf-hooks-review{,-2,-3,-4}/` was run or
  changed; the fourth review's base repository was cloned with `git clone --no-local` and its
  scripts were copied. The worktree was not written except for this file.
