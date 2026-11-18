# Fourth independent review of the agent hooks branch (`agent/loop`, commit `f9d1c9d`)

Date: 2026-11-14. Reviewer: an independent Claude agent that took no part in the work.
Brief: `~/.local/state/habitat-claude/ddf-gap-probes-20261113/ddf-hooks-independent-review-4-brief.md`.
Under review: `d4dbc91..f9d1c9d`, mainly the rework `ffa0f8e..f9d1c9d` that answers the third
independent review (`docs/reviews/claude-independent-review-hooks-3-20261114.md`) and Vesper's
round 11.

## Verdict: NO-GO

The rework does what it set out to do. Every case of the third review is closed, this time
with the real judge (table below): the stash with its untracked files and its older entries,
a commit that a later commit undoes, many refs, the order of refs, a failure that comes back,
a branch switch, a hostile ref name. A sweep of thirty ordinary Git workflows after a commit
with a credential (amend, squash, fixup and autosquash, `reset --soft` and `--hard`, revert,
branch deleted, detached `HEAD` left, tag, stash in four forms, plumbing, `GIT_DIR`, `gc`,
worktrees, an orphan branch, merges) blocks in every one except the stated limits and the
two index-only cases that are HR4-H-1. (One case of that sweep, `stash-index`, was written
wrongly and stashed nothing; its right form is I4 in `t5.sh`, which blocks.)

Two findings are high by the rule the documents state, and three are medium:

- Content that was staged and is no longer in the working tree is in no patch, and the stop
  says nothing (HR4-H-1). Not new in this commit; never stated.
- Once twelve failures stand in a session (one routine `git pull` under the starter policy
  does it), a later commit that is not judged is not said at all, and a later commit that
  fails blocks the stop with a reason that names neither the commit nor a rule (HR4-H-2).
- Commits that were in the repository when the session began and that the session brings
  onto its branch (`git pull` of a branch fetched earlier, a fast-forward merge of a local
  branch, a rebase, a reset) pass in silence; the README says they are judged; the commit
  before this one blocked them (HR4-M-1).
- A commit that gets no verdict is asked about again, first, at every stop; three of them end
  the judging of every later commit for the rest of the session (HR4-M-2).
- When the state can be read and no longer written, a failing commit blocks every stop, for
  good, and the agent has no ordinary way out (HR4-M-3).

Every finding below that depends on a verdict was reproduced with the real judge.

## How this was done

- Source read from `git archive f9d1c9d` (extracted to `/tmp/claude-ddf-hooks-review-4/src`).
  No `cargo` was run. No `toolbox`, `podman` or `distrobox` call was made.
- Binary: `/var/home/louranicas/.cache/ddf-t/review-bin/v4r12/release/deep-diff-forge`
  (SHA-256 `c8fcdc84…46b9`). Judge: the real forge-warden 0.2.0,
  `/var/home/louranicas/.cache/ddf-t/target-fw/release/forge-warden` (SHA-256 `55bf9fe6…74c9`),
  run on the host (the Git allowlist is installed in `/etc/forge-warden/`). Git 2.56.0.
- Scratch: `/tmp/claude-ddf-hooks-review-4/`. `base-repo` is a `git clone --no-local` of the
  third review's base repository with the policy's ranker digest set to the frozen binary and
  committed (`8c2623c`). Every case is a fresh clone under `cases/`. Scripts `t0.sh` … `t17.sh`,
  helper `h.sh`, outputs in `out/`. `/tmp` does not survive a reboot, so the commands that
  matter are repeated below.
- Every run had `HOME=/tmp/claude-ddf-hooks-review-4/home` (set inside `h.sh`),
  `XDG_STATE_HOME` and `CLAUDE_PROJECT_DIR` unset, `GIT_CONFIG_GLOBAL=/dev/null` (one case sets
  its own). The real `~/.local/state/deep-diff-forge/hooks` is as it was (empty, last touched
  16:57:54, before this review).
- The helper's commands (`$S` a session id, `$R` a directory):
  - `start R`: `printf '{"hook_event_name":"SessionStart","session_id":"%s","cwd":"%s","stop_hook_active":false}' $S $R | $DDF agent hook session-start --client claude --judge $FW`
  - `post R`, `stop R [true]`: the same with `PostToolUse` / `Stop` (`stop_hook_active` as given)
    and `agent hook post-tool|stop --client claude --judge $FW`
  - `stopr R`: a `Stop` with a recording stand-in that gives no verdict (`tools/recjudge`),
    used only to show which patches a stop makes
  - `secret FILE` writes `pub fn a() {}\npub fn b() {}\npub const K: &str = "AKIAIOSFODNN7EXAMPLE";\n`;
    `harmless R` writes a third function into `lib/a.rs` and two tests into `tests/t.rs`.
- Controls (`t0.sh`, `out/t0.txt`, real judge): a credential in the working tree blocks and
  then stands; a harmless change with its test is silent; a committed credential blocks
  ("Commit 25fb6e715f60: FAIL") and then stands. One judged stop takes about 0.26 s here.
- What Claude Code does (the order of hooks, session ids on resume, what it does while a stop
  hook keeps blocking) is taken from its documentation and the branch's own statements, not
  from a live session.

## Findings reproduced with the real judge

### HR4-H-1 (high; ORDINARY WORKFLOW): content that is staged and no longer in the working tree is not judged, and nothing is said

A stop judges the working tree against `HEAD` and the session's commits. What the index
holds and the tree does not is in neither. The ordinary way to get there is the one the hooks
themselves produce: the agent runs `git add -A`, the advice hook reports the credential, the
agent repairs the file and does not stage it again, and stops.

Reproduced (`t5.sh`, `out/t5.txt`):

```
== I1: git add -A with a credential in a tracked file; advice; the file is repaired and NOT staged again; stop
 -- post-tool after git add:      … secret_like at "lib/a.rs" line 3 …
 -- post-tool after the repair:   (NO OUTPUT)
 -- git status:                   MM lib/a.rs /  M tests/t.rs
 -- stop:                         [exit 0] (NO OUTPUT)
   the index holds the credential: 1   the working tree: 0
 -- what a plain 'git commit' after the session commits:
   credential lines in the new commit: 1
== I2: a NEW file with a credential is staged, then deleted from the working tree; stop
   [exit 0] (NO OUTPUT)      index: 1
```

```bash
W=/tmp/claude-ddf-hooks-review-4; . $W/h.sh
R=$(fresh i1); SESSION=i1; start $R >/dev/null
secret $R/lib/a.rs; git -C $R add -A; post $R        # advice: secret_like at "lib/a.rs"
harmless $R; post $R                                 # (no output)
stop $R                                              # -> [exit 0] (NO OUTPUT)
git -C $R diff --cached | grep -c '^+.*AKIA'         # -> 1
```

What bounds it: when the session itself commits the staged content, that commit is judged at
the next stop and blocks (I3, same script); a stash of it is judged through the stash's index
commit (I4). What the stop leaves unjudged is what the next plain `git commit` records, and a
person who commits after the session does so with no verdict. The same run on the commit
before (`t5b.sh`, v4r11 binary) is also silent, so this is not a regression of this commit.

Stated? No. README: "A verdict on your uncommitted changes"; "First the uncommitted change of
the project's working tree: the tree, untracked files included". Staged content is an
uncommitted change. The limits do not name it. The earlier reviews closed the case where the
stale index is committed (HR2-H-1 2f); none looked at the index at the stop.

Suggested direction: when the index differs from both `HEAD` and the working tree, judge it
as a third unit (a tree written from a copy of the index in the scratch directory, diffed
against `HEAD` like a commit: "Staged changes in …"); or, cheaper, say that staged content
differs from the tree and was not judged.

### HR4-H-2 (high; ORDINARY WORKFLOW): after twelve failures stand, a commit that is not judged is not said, and a commit that fails blocks without being named

`trees_text` lists the first twelve units that did not pass, in the order uncommitted
changes, then commits the oldest first, and counts the rest ("and N more, not listed."). A
failing commit stands for the rest of the session and is listed at every stop. The oldest
are the standing ones, so the newest units, which are the ones this stop is about, are the
first to fall off the list.

Twelve standing failures are one routine pull away under the starter policy: the rule
`tests-travel-with-code` fails every commit that changes `src/**` and no test file.

Reproduced (`t6c.sh`, `out/t6c.txt`): a session starts; upstream gains 13 commits that each
add a file under `src/`; `git pull`; stop 1 blocks and lists them; stop 2: FAIL STANDS. Then
the session's own work, a credential in `lib/a.rs`, committed:

```
{"decision":"block","reason":"deep-diff-forge: the judge fails this change (…). What the rules name in this session's own work is this session's to repair; … Then stop again…
Commit …: FAIL, which stands   (twelve lines)
and 2 more, not listed.
   block: 1   names the session's commit b15b6b2f7d74: 0   names a rule: 0   names lib/a.rs: 0
```

The agent is told to repair and is not told what. `t6.sh` (`out/t6.txt`) is the same with 14
commits of the session's own.

The unjudged form (`t6b.sh`, `out/t6b.txt`): 13 failures stand; the session then makes a
commit that gets no verdict (a file whose name is not UTF-8). The whole output of the stop:

```
{"systemMessage":"deep-diff-forge: FAIL STANDS. This failure has already blocked a stop of this session (…); it is left for a person.
Commit 3afc3e88aa43: FAIL, which stands
… (twelve lines)
and 2 more, not listed.
"}
   the word UNJUDGED in it: 0   the new commit's id in it: 0
```

It differs from the stop before only in "2 more" for "1 more". README: "the result is
UNJUDGED, said plainly, and never a pass". By the rule this is an ordinary workflow that ends
with unjudged work of the session and no word about it. By reading (not run), the same holds
for a new HOLD, and for the unit "N commits of this session: UNJUDGED … judged at the next
stop", which is always last. Two more things of the same function, also from reading: the
headline of such a stop is "FAIL STANDS" whatever else the stop found (the worst answer
decides, and a standing failure is a fail); and the text is cut at 6,000 characters without
saying so.

Suggested direction: list first what holds or is new at this stop (a fail that blocks now,
UNJUDGED, HOLD), then the standing failures; never fold a unit that blocks now or is UNJUDGED
into "N more"; give standing failures one line in all once there are several ("14 commits
whose failure stands"); let a stop whose only fails stand be headed by its worst new answer.

### HR4-M-1 (medium; ORDINARY WORKFLOW): commits that were in the repository when the session began are never judged, however they reach the session's branch; the README says pulled and merged commits are judged

A commit is the session's when the sources lead to it and nothing recorded at the first hook
does. Recorded is every ref, the remote-tracking ones included. So a commit that was fetched
before the session, or that a local branch held, is not the session's when the session pulls,
merges, rebases onto or resets to it.

Reproduced (`t2.sh`, `out/t2.txt`; a credential in the upstream commit or on the local
branch `wip`):

| Case | What the session did | At the stop |
| --- | --- | --- |
| A0 control | upstream commit fetched during the session, `git pull` | block |
| A1 | `origin/main` fetched before the session; `git pull` (fast-forward) | **no output**; `main` holds the credential |
| A2 | the same, `git merge origin/main` | **no output** |
| A3 | a local branch `wip` from before the session; `git merge wip` (fast-forward) | **no output**; `main` holds the credential |
| A4 control | the same after a commit of the session's own, so there is a merge commit | block (the merge commit, against its first parent) |
| A5 | `git rebase origin/main` (fetched before), the session's commit on top | **no output** |
| A6 | `git reset --hard origin/main` (fetched before) | **no output** |
| A7 | `git cherry-pick wip` | block (a new commit; run a second apart, see below) |
| A8 | a stash from before the session, `git stash pop`; then committed | block; then stands |

```bash
W=/tmp/claude-ddf-hooks-review-4; . $W/h.sh
git clone -q --no-local $W/base-repo $W/cases/a1-up; git clone -q --no-local $W/cases/a1-up $W/cases/a1; R=$W/cases/a1; SESSION=a1
secret $R-up/lib/a.rs; git -C $R-up commit -qam "upstream: add key"
git -C $R fetch -q                 # before the session: an editor's background fetch, another session, a cron job
start $R >/dev/null; git -C $R pull -q
post $R                            # advice: 1 new finding … secret_like at "lib/a.rs" line 3
stop $R                            # -> [exit 0] (NO OUTPUT)
```

Whether a pull is judged thus depends on when the fetch happened, and whether a merge is
judged on whether it could fast-forward. The advice hook reports the same content
(`t16.sh`: "secret_like at lib/a.rs" after the pull, then a silent stop). The commit before
this one blocked A1 (`t2b.sh`, v4r11 binary: `decision: block`), so the change of unit took
this out of what is judged.

Stated? The definition is exact in one place ("nothing that was named when the session began
does"). Three other places say otherwise: README "a pull or a merge makes the commits it
brings the session's"; the limits list "Commits pulled or merged during the session are the
session's and are judged, each once"; and every message's "this includes commits pulled or
made by others in that time". Why not high: the content was made before the session and not
by it. It is high if the owner holds to the README's sentence that such commits are the
session's.

Suggested direction: tell the session's commits from what its own names led to when it began
(`Named::Own` at the first hook: local branches, tags, the stash, each `HEAD`, the reflogs)
instead of from every ref; then a commit that only a remote-tracking ref named becomes the
session's when a pull or a merge brings it, as one fetched later does. Or state the limit and
correct the three sentences.

A note on A7: in the first run the cherry-pick was made in the same second as the commit it
copied, with the same author and committer, and Git gave it the same id; such a commit is the
commit from before. `out/t2.txt` shows that run (silent); `out/t2b.txt` the run a second apart.

### HR4-M-2 (medium; ORDINARY WORKFLOW): a commit that gets no verdict is asked about again, first, at every stop; three of them end the judging of every later commit of the session

Nothing is remembered of a commit that got no verdict, so every stop asks again, the oldest
first. The count of "answers in a row that are no verdict" passes over commits that already
have a verdict. So three commits without a verdict anywhere in the session's history are
"three in a row" at every later stop: no commit after them is ever judged.

Which commits get no verdict from the real judge (`t1.sh`, `t1b.sh`, `out/t1.txt`,
`out/t1b.txt`): a file whose name is not UTF-8 ("patch parse failed"); a patch over 1,000,000
lines; a patch over 64 MiB (a 60 MB binary: "Git output exceeds the 67108864-byte budget");
and a 3 MB file of one line, the shape of a minified bundle ("the ranker timed out", after
10 seconds).

Reproduced (`t3.sh`, `out/t3.txt`; `t16.sh` N3, `out/t16.txt`):

| Case | History of the session | At the stops |
| --- | --- | --- |
| N1 | three commits with no verdict, then a commit with a credential | every stop: three "UNJUDGED" and "1 commit of this session: UNJUDGED" with the third commit's reason ("non-UTF-8 … path"); **never a block** |
| N2 | U P U P U (P harmless, U no verdict); stop; then the credential commit | stop 1 judges both P; stops 2 and 3: the credential commit is "1 commit of this session: UNJUDGED"; **never a block**. State: `judged: [P1:p, P2:p]` |
| N2 control | the same six commits before the first stop | block: "Commit b660f8e6431d: FAIL" (the verdicts between reset the count) |
| N3 | three commits that each add a 3 MB one-line `dist/app.min.js`, then the credential commit | every stop takes **30.6 s** and says the same; the credential commit is never judged |

```bash
W=/tmp/claude-ddf-hooks-review-4; . $W/h.sh
U(){ printf 'x\n' > "$(printf "$1/lib/caf\xe9-$2.rs")"; git -C $1 add -A; git -C $1 commit -qm "latin-1 name $2"; }
P(){ printf '// %s\n' "$2" >> $1/tests/t.rs; git -C $1 commit -qam "harmless $2"; }
R=$(fresh n2); SESSION=n2; start $R >/dev/null; U $R 1; P $R 1; U $R 2; P $R 2; U $R 3; stop $R
secret $R/lib/a.rs; git -C $R commit -qam credential; stop $R; stop $R    # never "decision":"block"
```

Never silent, so not high. But a failure becomes UNJUDGED for good, with a reason that
belongs to another commit, and each stop pays for the same answers again. README states the
rule for one stop ("After three answers in a row that are no verdict, the stop asks no more
and says the reason for the rest"); it does not say that the three are asked first at every
stop, so that the rule then holds for the session.

Suggested direction: remember a commit whose patch the ranker or the budget refused (it will
be refused again) as not judged, say it standing in one line, and do not ask again; count
"in a row" only for answers that speak of the chain (no judge, no policy, the environment);
ask about commits that were never asked before those that had no verdict.

### HR4-M-3 (medium; ORDINARY WORKFLOW, when the state cannot be written): a failing commit blocks every stop, for good, and the agent cannot repair it

"A failing commit blocks once" rests on the state: the commit is remembered as judged. When
the state file can be read and not replaced (the directory has become read-only, the disk is
full), nothing is remembered, the commit is judged again at every stop and blocks every
time. `stop_hook_active` is read from the input and used nowhere. A commit cannot be
repaired: amended away, the reflog of `HEAD` still names it, and removing it from there is
the evasion the documents name.

Reproduced (`t7.sh`, `out/t7.txt`): session start; `chmod 500` on the state directory; the
session commits a credential and amends it away; four stops, the last three with
`stop_hook_active: true`:

```
 -- stop 1 (stop_hook_active=false): BLOCK | … The hook could not keep its state (its state could not be saved: Permission denied (os error 13)): the same failure can block again, …
 -- stop 2 (stop_hook_active=true):  BLOCK | … (the same)
 -- stop 3 (stop_hook_active=true):  BLOCK | … (the same)
 -- stop 4 (stop_hook_active=true):  BLOCK | … (the same)
```

It is said, in every reason, so the cause is visible. That a failing uncommitted change
blocks at every stop in this state was known and accepted (HR-L-4): the agent could remove
the change. With commits judged through the reflog, it cannot. The brief's item "a block that
cannot be escaped": this is one. A full disk was not produced; it reaches the same code
(`write_private` fails, `unsaved` is set).

Suggested direction: when the state could not be kept and the input says
`stop_hook_active: true`, allow the stop and say the failure and the state together; a hook
that cannot remember must not block twice running.

### HR4-L-1 (low; ORDINARY WORKFLOW for some users): a Git configuration or a stale worktree stops every session from being recorded, and neither the gate nor the reason says what to do

`named_commits` reads the reflogs with `git log --walk-reflogs --format=%H`, under the user's
configuration, and refuses any line that is not an id.

- `log.showSignature=true` with one signed commit in the reflog (`t11.sh`, `out/t11.txt`):
  the session start says "the stops of this session will not be judged … invalid Git
  selection: git log wrote a line that cannot be read", and every stop is UNJUDGED with the
  same words. They name neither the setting nor a remedy. `git log --no-show-signature`, or
  `git rev-list --walk-reflogs`, does not read the setting.
- A linked worktree whose directory was removed by hand, with a plain directory at its path
  outside any repository (`t10.sh` G3, `out/t10.txt`): the same, with "`git rev-parse` exited
  with status 128: fatal: not a git repository". It names neither the path nor
  `git worktree prune`.
- `agent doctor --gate` has no link for this (`t11b.sh`, `out/t11b.txt`): in the first
  repository `git`, `repository`, `policy`, `judge`, `chain`, `policy_protects_itself`,
  `patch`, `state_directory` and `hooks_enabled` are all `ok`; with the settings registered
  it would say ready where every stop is UNJUDGED (read from `gate_in`, which never lists
  commits; the registered form was not run).
- A resumed session whose list of commits is gone (`t17.sh`): the start says nothing about
  it; the stop is UNJUDGED and names the remedy.

### HR4-L-2 (low; ORDINARY WORKFLOW in a shallow clone): a commit whose parent the clone does not have is judged "from nothing": its change is the whole tree

`rev-list --parents` gives no parent for a commit at the edge of a shallow clone, and a
commit with no parent is diffed against the empty tree.

Reproduced (`t10b.sh`, `out/t10b.txt`): `git clone --depth 1`; upstream adds two one-line
commits; `git fetch --depth 1 origin main; git reset --hard origin/main`. The patch the stop
makes for the new tip holds all three files of the repository (`lib/a.rs tests/t.rs
warden.toml`), though the commit changed one line; the real judge answers HOLD (`ci-is-owned`:
the policy file is "added"). In a real repository the whole tree as an addition is a false
fail or hold, or over a budget and then one of the commits of HR4-M-2. Cheap fix: for a
commit that the repository lists as shallow (`.git/shallow`), say UNJUDGED with that reason.

### HR4-L-3 (low; reproduced by simulation, real judge): when the memory of verdicts is full, a commit that never got a verdict counts as judged

`remember` moves the oldest verdicts into the tips, "so it and what it leads to count as
there before the session". What it leads to includes an ancestor that never got a verdict.

Reproduced (`t12.sh`, `out/t12.txt`): U (no verdict), P1 (passes); stop 1 says "Commit U:
UNJUDGED". The state's `judged` list is then filled to 4,096 by editing it (the simulation:
the natural way is 4,096 verdicts in one session, a large pull). P2 is committed; stop 2
judges it and moves P1 to the tips; **stop 3: no output**, with U on `main` and never judged.
README: "the last 4,096; older ones count as judged" says it of the judged ones only.

### HR4-L-4 (low; ORDINARY WORKFLOW): "committed as it is" is matched by rule, signal, path and count, not by content

A commit's failure does not block when the same failure identity is among the uncommitted
failures that have blocked. The identity has no content in it. Reproduced (`t9.sh` B2,
`out/t9.txt`): a credential in `lib/a.rs` blocks; it is replaced by another credential, with
other work added to the file; committed: "Commit f8f302216a46: FAIL, which stands", no block,
and "This failure has already blocked a stop of this session" is said of a commit the agent
was never shown. The same looseness is stated for uncommitted changes ("the same rules,
signals, paths and numbers of items"); for the commit README says "committed as it is". The
comment on `State::blocked` says each entry is `FAILURE|PATCH` with the SHA-256 of the patch;
the code keeps the failure digest alone. With the patch digest in it the rule would be what
the README says. B3 (another file under a rule with a path) blocks, as it should.

### HR4-L-5 (low; ORDINARY WORKFLOW; a stated limit, stated accurately, on the wrong side of the rule): uncommitted work in a working tree the session made and is not "in" is silent

"In" means the hook input's `cwd`. An agent that runs `git worktree add ../wt -b side` and
edits there by absolute path never has its `cwd` in it. Reproduced (`t4.sh`
`worktree-uncommitted`, `out/t4.txt`): a credential in a new file of that tree; stop from
the project: **no output**. `t10.sh` G2: a session that starts in a linked worktree does not
have the main working tree's uncommitted change judged either. Commits in either are judged.
The limit is in the README. It is cheap to make visible: `git worktree list` is read at
every stop already; a tree that was added since the session began, or has changes, can be
named once as not judged.

### HR4-L-6 (low): statements that do not match the code

- README, three places, on pulled and merged commits (HR4-M-1).
- `State::blocked` comment: `FAILURE|PATCH` (HR4-L-4).
- README "the result is UNJUDGED, said plainly": not past twelve units (HR4-H-2).
- "FAIL STANDS. This failure has already blocked a stop of this session" heads a stop whose
  standing commit never blocked (HR4-L-4), and any stop that has a standing failure and
  something else.
- README "After three answers in a row …": true of one stop; the consequence for the session
  is not said (HR4-M-2).

## Findings that need no judge

- HR4-L-1 (the session cannot be recorded; `agent doctor --gate` output): no verdict is
  involved.
- The patch of HR4-L-2 was first shown with the recording stand-in (three files), then judged
  by the real judge.

## Suspected only (not reproduced)

- **Two hooks at once lose an update.** An advice hook that loaded the state before a stop
  saved it, and saves after, writes the older lists back: a judged commit is judged again and
  a failure blocks again. Read from `Session::keep`; not run. Never a pass.
- **A full disk** gives HR4-M-3 as a read-only directory does (same code path); not produced.
- **Resume with a new session id.** If Claude Code gives a resumed session a new id, commits
  made before a crash and not yet judged are recorded as "there before". The README says a
  resume keeps the id; not checked against a live client.
- **overlayfs.** The inode of a directory on overlayfs without `xino` is not promised across
  mounts, so a session resumed after a container restart may be told that another repository
  stands at its path. Not run.
- **The 6,000-character cut** of a stop's detail can drop the rules of a later unit with no
  word; read from `trees_text`, not produced on its own.

## The third review's cases on `f9d1c9d`, with the real judge (`t15.sh`, `out/t15.txt`)

| Third review | Now |
| --- | --- |
| HR3-H-1 J3 new file, `git stash -u` | block ("Commit 5da243ea772c: FAIL", the commit of the untracked files) |
| HR3-H-1 J4 tracked edit and new file, `stash -u` | block |
| HR3-H-1 J5 two stashes, the credential in the first | block |
| HR3-M-1 J6 committed, then removed by a second commit | block at the first stop; stands after the second commit; both commits before the first stop: block |
| HR3-M-2 J7 five branches, the credential in the fifth | block |
| HR3-M-2 5a 20 fetched branches, the session stashes a credential | block, on the stash commit; the fetched branches are not asked about |
| HR3-H-2 J8 4,200 remote-tracking refs: stash; branch, commit, switch back | block; block |
| HR3-M-3 6a repaired, then another credential in the same file | block, silent, block |
| HR3-L-1 6b a standing commit, branch switch, back with a comment line | stands at each, no second block |
| HR3-L-5 t12 a fetched branch with a sentence for a name | silent while only fetched (stated); block once checked out; the name is not in the output |
| HR3-L-2 9b 40 nested repositories | 32 named and "More things were left out …" once; a 41st is then silent, as that sentence says |
| HR3-L-4 a ref whose name is not UTF-8 | the session is recorded; a commit blocks |
| HR3-M-4 the device number | not in the identity; a project moved away and back is judged, moved for good is UNJUDGED with the remedy (`t14.sh` L3) |

## The areas of the brief: what was tried and whether it held

0. **This round's rework.**
   - Held, each a block with the real judge (`t4.sh`, `out/t4.txt`): amend; `reset --hard`;
     `reset --soft` and recommit; squash; `--fixup` and `rebase -i --autosquash`; a rebase
     that drops the commit; revert; a branch deleted; a detached `HEAD` left; a tag then
     reset; stash (tracked, `-u`, two entries; index only, `t5.sh` I4); `branch -f`; a commit
     made with `commit-tree` and `update-ref`; a commit made under `GIT_DIR`/`GIT_WORK_TREE`;
     `git gc`; a branch and a detached `HEAD` in a linked worktree; an orphan branch; a file
     taken back out of the tree after the commit; `merge -s ours`; `merge --squash`. Also: a
     session that starts in a linked worktree; a bare repository with linked worktrees, with
     a commit only the worktree's reflog names (`t14.sh`); `core.logAllRefUpdates=false` in a
     clone (Git still appends to a reflog that exists; `t10.sh` G5, G6).
   - Silent and stated: `stash drop`; `reflog expire` with `gc --prune=now`; a detached
     worktree removed with its commit; a commit that only a remote-tracking ref names.
   - Broken: HR4-H-1, HR4-H-2, HR4-M-1, HR4-M-2, HR4-M-3, HR4-L-1 to L-5.
   - Merge commits: judged against the first parent, which shows everything the second
     parent brings (A4 blocks on the merge commit); the second parent's own commits are
     judged one by one when they are the session's. What goes unjudged is the fast-forward
     (HR4-M-1), where there is no merge commit.
   - The order and the deadline (`t8.sh`, `out/t8.txt`): a pull of 450 harmless commits and
     then the session's own commit with a credential: stop 1 takes 38.1 s, judges 428, says
     "23 commits of this session: UNJUDGED"; stop 2 (2.4 s) blocks on the session's commit.
     About eleven commits a second here; a pull of 5,000 is twelve stops of forty seconds
     with the session's own work judged last. Stated ("several stops"); the README's
     "A pull of many commits takes several stops to judge; each stop says how many wait" is
     accurate.
   - Blocking: a failing commit blocks once and stands; an uncommitted failure blocks, is
     forgotten when gone and blocks when back (also after `stash` and `stash pop`, `t9.sh`
     B4, by design); HR4-L-4 for the loose match; HR4-M-3 for the block that does not end.
   - State and tips (`t13.sh`, `t17.sh`): a tips file that is unreadable or gone: UNJUDGED
     with the remedy; a state of schema v2: UNJUDGED, nothing written over it; a session id
     with a path in it names no file outside the directory. A crash between the two writes
     is the "not the list this session recorded" case. HR4-L-3 for the bound of 4,096.
1. **A pass that should not be one.** HR4-H-1 and HR4-M-1 are the two (no verdict is asked
   for). The sealed-verdict checks were only spot-checked (`t13.sh`): a judge that exits 0
   with no document, with a small "pass" document, and a judge inside the repository are each
   UNJUDGED with the reason.
2. **Blocks.** HR4-M-3 (fires for good). HR4-H-2 (blocks without saying what). UNJUDGED
   never blocked in any run.
3. **Input and output.** Empty, not JSON, wrong event, no session, a relative `cwd`, 1 MiB
   of random bytes: each exit 0 with valid JSON saying UNJUDGED (`t13.sh` R4). No ref name
   reaches a message (t12). Every output in every run parsed as JSON or was the plain start
   text.
4. **Resources and files.** HR4-M-2 for time (30.6 s a stop). No scratch directory was left
   under any case's `.git/deep-diff-forge` after the runs I looked at. State files are 0600
   in a 0700 directory. A full disk was not produced.
5. **Setup and doctor.** HR4-L-1 (no link for listing the commits). Not otherwise attacked.
6. **`schema signals` / `explain signal`.** 21 signals; for each label `explain signal
   <label> --json` equals its entry in `schema signals`; the weights equal
   `RiskSignal::weight` and `EvidenceSignal::weight` in the source; an unknown label exits 2
   and names `schema signals`. Held as far as that goes.
7. **Tests.** Below.

## Weak tests

Read from `tests/agent_hook.rs` and `patch/tests/judge_patch.rs` at `f9d1c9d`; no guard was
removed and no test was run.

- **No test needs the refs as a source.** `Named::Own` lists `refs/heads`, `refs/tags` and
  `refs/stash`, then the `HEAD`s and the reflogs. In every hook test, and in
  `the_commits_that_something_names_are_listed_by_their_ids`, each commit under a branch, a
  tag or the stash was made through a `HEAD` or `git stash`, so a reflog names it too ("a
  commit only a tag still names" was committed on a detached `HEAD`). Take the three ref
  names out of `Named::Own` and, by reading, every test passes. The case that needs them is
  a commit that was never on a `HEAD`: `commit-tree` with `update-ref refs/heads/x`, or a
  fetch into a local branch (`t4.sh` `update-ref-plumbing` blocks with the real binary).
- **The rule of three answers that are no verdict has no test** (`NO_VERDICTS_IN_A_ROW`
  appears in no test): not for one stop, not across stops (HR4-M-2).
- **No hook test has a merge commit.** That a merge is judged against its first parent, and
  what a fast-forward leaves out, is untested (HR4-M-1; the pull test fetches from a path
  during the session).
- **No test has more than twelve units**, so nothing checks which units a long list keeps
  (HR4-H-2). The unit tests of `trees_text` count the rest; none asks that a unit which
  blocks now is among those shown.
- **No test has content in the index that the tree lacks at a stop** (HR4-H-1); `2f` commits
  it first.
- **A state that cannot be kept is tested with an uncommitted failure only**
  (`a_new_item_blocks_again_and_a_state_that_cannot_be_kept_is_said`), and no test reads
  `stop_hook_active` for a decision, because no decision uses it (HR4-M-3).
- **The bound of 4,096 verdicts is tested in `remember` alone**
  (`a_failure_that_is_gone_is_forgotten_and_old_verdicts_go_to_the_tips`); no test has an
  unjudged ancestor of a verdict that is moved out (HR4-L-3).
- **No test runs a hook under a user's Git configuration** (HR4-L-1), in a shallow clone
  (HR4-L-2), or in a bare repository with worktrees.
- **Would fail if the guard were removed** (by reading): each commit judged by its own
  change (`a_credential_that_was_committed_and_then_reverted_is_judged` asserts the two
  patches shown and the id named); the reflog of `HEAD` and of a linked tree
  (`a_commit_left_on_a_detached_head_…`); the stash's third parent and its older entry
  (`a_stash_of_untracked_files_…`: needs the stash's reflog and the walk to parents);
  judged once (`runs` is counted over three stops); forgotten when gone
  (`a_failure_that_was_repaired_and_comes_back_blocks_again`); the tips file held against
  the state; oldest first with the rest at the next stop (the forty-second test).

## Not looked at

- A live Claude Code session; macOS; Git older than 2.56; SHA-256 repositories; reftable;
  partial clones; Git LFS; submodules beyond what the earlier reviews ran; a real reboot;
  overlayfs and network file systems; a full disk.
- The verdict checks of rounds 1 to 10, the ranker seal helper and `verdict_v2.rs` beyond
  the three spot checks; `agent setup --hooks`; the settings links of the gate.
- `cargo` was not run: no test suite, no mutation run. The statements under "Weak tests" are
  from reading.
- Housekeeping, for the record: three of my scratch scripts (`t4.sh`, `t5.sh`, `t13.sh`)
  remove a file inside their own case clone with `rm` (script files, not inline commands;
  none was refused). `t10.sh` made one empty directory beside the scratch directory
  (`/tmp/claude-ddf-hooks-review-4-wt-g3`), which I removed with `rmdir`. Nothing under the
  example scratch directory or `/tmp/claude-ddf-hooks-review{,-2,-3}/` was run or changed;
  the third review's base repository was cloned with `git clone --no-local`.
