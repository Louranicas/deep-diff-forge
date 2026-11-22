# Eighth independent review of the agent hooks branch (`agent/loop`, commit `8746cd7`)

Date: 2026-11-15 (02:48 to 03:12 AEDT). Reviewer: an independent Claude agent that took no part in the work.
Brief: `~/.local/state/habitat-claude/ddf-gap-probes-20261113/ddf-hooks-independent-review-8-brief.md`.
Under review: `d4dbc91..8746cd7`, mainly the rework `065c74f..8746cd7` ("a turn that always
comes, a tree known as itself, a failure known line by line").

## Verdict: GO-WITH-FIXES

I found no stop that prints nothing while something is open, and no new failure that is
taken for one that stands, outside the identity the documents state. I found one medium
finding that is a regression of this commit, and four lows. None is a deliberate evasion.

- **HR8-M-1** (medium; ORDINARY WORKFLOW in one Git configuration; NEW with `8746cd7`). A
  linked working tree made with relative paths (`git worktree add --relative-paths`, or
  `worktree.useRelativePaths=true`; Git 2.48 and later) is not recognised from outside. A
  stop made from the project forgets what has blocked in that tree and does not say that a
  failure stands there, and the unchanged failure blocks again at every return to the tree.
  The build before (`065c74f`) stands in the same sequence. Claim 3 is broken there.
- **HR8-L-1** (low; ORDINARY WORKFLOW on a slow stop). With between three and five seconds
  left after the working trees, the slow-lane commit whose turn it is is not submitted to
  the judge, and is written down as asked: its turn is used and an attempt is counted. The
  stated precondition of claim 2 is "three seconds left".
- **HR8-L-2** (low; ORDINARY WORKFLOW in a large repository with many working trees). The
  stop looks at up to sixteen other working trees before it judges the session's own
  change, with no bound of its own. When that takes the stop's time, the session's own
  credential is UNJUDGED at every stop, with a reason that does not name the cause.
- **HR8-L-3** (low; ORDINARY WORKFLOW, crowded). A failure that stands in a working tree
  beyond the sixteen is not said at a stop from the project. README says it is said at
  every such stop.
- **HR8-L-4** (low; ORDINARY WORKFLOW; a stated limit, accurate but not stated where it
  matters). One failing line replaced by another under the same rule in the same file
  between two stops stands, and the message says "has already blocked", which is not true
  of it.

What I would fix before a merge: HR8-M-1 (a few lines and one test). The lows can follow.

## One mistake of mine that touched another session's run

At about 03:00:50 AEDT (within 20 seconds) I killed process 2545823 with `kill -9`, having
chosen it with `pgrep -n -x deep-diff-forge`. It was not my process: my own stop was pid
2542156 and ran on. A mutation run of another session was running on this machine
(`~/.cache/ddf-t/m2438522/cargo-mutants-mut-src-agentloop-*`, output
`~/.cache/ddf-t/mut-agentloop-r15-out`, three jobs), and its `agent_hook` tests start many
processes of that name; I very probably killed one of them. The mutants under test at that
moment were `hook.rs:609:24`, `hook.rs:626:21` and `hook.rs:1129:58` (`603:20` had ended at
03:00:45). One of those three may carry a false "caught". I did not change anything else
of that run. After this I took process ids from `$!` only (`t6k.sh`).

## How this was done

- Source read from `git archive 8746cd7` (extracted to `/tmp/claude-ddf-hooks-review-8/src`).
  No `cargo`, no build, no `toolbox`, `podman` or `distrobox` call, nothing at GitHub, no
  network. The only file written in the worktree is this report.
- Binary: `/var/home/louranicas/.cache/ddf-t/review-bin/v4r16/release/deep-diff-forge`
  (SHA-256 `6936ce8f…3afd`); for "is it new" the build before, `…/v4r15/release/…`.
  Judge: the real forge-warden 0.2.0, `/var/home/louranicas/.cache/ddf-t/target-fw/release/forge-warden`
  (SHA-256 `55bf9fe6…`), on the host. Git 2.56.0. Load average 9 to 13, with the mutation
  run beside it.
- Scratch: `/tmp/claude-ddf-hooks-review-8/` (tmpfs). `base-repo` is a clone of the seventh
  review's base with the ranker digest of this build committed (`76b20f5`); `base-long` the
  same with long rule names. Every case is a fresh clone under `cases/`. Helper `h.sh`,
  scripts `t0.sh` to `t11.sh`, `t15b.sh`, `t5.py`, `r6/t6.sh`, outputs in `out/`, notes in
  `NOTES.md`. Every run had a private `HOME` set inside the helper
  (`/tmp/claude-ddf-hooks-review-8/home*`), `XDG_STATE_HOME` and `CLAUDE_PROJECT_DIR`
  unset, `GIT_CONFIG_GLOBAL=/dev/null`.
- Where a verdict had to be missing or slow, the judge is a wrapper that says so in its
  header: `tools/tj` (logs each submission by the argument after `--patch`, gives no
  verdict for marked patches, can sleep, else runs the real judge unchanged),
  `tools/flaky`, `tools/flaky-long`, `tools/slowjudge` (the seventh review's). A slow Git is
  `tools/slowstatus/git` (sleeps before `git status`, else the real Git). These runs say so.
- What Claude Code does with `stop_hook_active`, where it cuts a message and whether hooks
  of one session overlap is taken from the branch's own statements, not from a live session.

## Findings reproduced with the real judge

### HR8-M-1 (medium; ORDINARY WORKFLOW with relative worktree paths; NEW with `8746cd7`): a linked tree made with relative paths is forgotten by a stop from the project, and its unchanged failure blocks again at every return

`linked_instances` reads each `worktrees/<name>/gitdir` and takes the parent of what it
holds as the tree's path. With relative paths that file holds
`../../../../NAME-wt/.git`, which is relative to the administrative directory, not to the
hook's working directory. `listed_instance` compares it with the absolute path
`git worktree list` prints, finds no match, and falls back to `by_path`. So from outside
the tree has the identity `x…`, and from inside (`tree_instance`, by its Git directory) it
has the real one. `remember` then finds the real identity "not listed and not looked at"
and forgets its lines; `other_trees` asks `stands_in` about the wrong identity and does not
say that a failure stands.

`t2.sh rel` and `t2.sh cfg`, `out/t2.txt` (nothing simulated):

```
== rel   (git worktree add --relative-paths -b f1 ../NAME-wt; a key-like value in lib/fixture.rs there)
   gitdir file: ../../../../t2rel-2398541-wt/.git
-- stop 1 in the tree                 => BLOCK
-- stop 2 in the tree again           => allow: FAIL STANDS     blocked: ['b82a142db6df2f51/u/8f246cd0…:1']
-- stop 3 from the project            => allow: The working tree "…-wt" … has changes …   blocked: []
                                         (284 chars: no "A failure there has blocked … and stands")
-- stop 4 back in the tree, unchanged => BLOCK
-- stop 5 from the project            => allow: 1 thing in this project is not judged: the working tree "…-wt".
-- stop 6 back in the tree, unchanged => BLOCK
== abs   (control, same script, absolute paths)
-- stop 3 from the project            => blocked: ['d76132bcc3951bb5/u/8f246cd0…:1']   (382 chars, says it stands)
-- stop 4, stop 6                     => allow: FAIL STANDS
```

`t2.sh cfg` (`git config worktree.useRelativePaths true`, then a plain `git worktree add`)
gives the same as `rel`. Control on the build before, `out/t2-r15.txt`
(`DDF=…/v4r15/release/deep-diff-forge BASE=/tmp/claude-ddf-hooks-review-7/base-repo`):
stops 4 and 6 are FAIL STANDS. So this is new with the commit.

```bash
W=/tmp/claude-ddf-hooks-review-8; bash $W/t2.sh rel; bash $W/t2.sh cfg; bash $W/t2.sh abs
```

What it breaks: claim 3 ("an unchanged line never blocks twice … in the same tree
instance, whatever is … looked at in between"), README's "A stop that did not look …
forgets nothing, names the tree and says at every such stop that a failure stands there",
and it brings HR6-M-1 back for these trees: a session whose directory goes between such a
tree and the project is blocked at every return. It is not silent, and no new failure is
missed by it. It needs the option, which is off by default; people turn it on for
repositories that are moved or mounted at different paths (containers).

Cheap fix: resolve a relative `gitdir` against its administrative directory
(`admin.join(path)`), as Git does. Sturdier: do not go through paths at all for "is this
tree still listed": the set of identities of the directories under `worktrees/` is what
Git lists, and `remember` should forget an entry only when its identity is not in that
set; and never forget because a listed tree could only be named `by_path`. A test: the
sequence above (no test in `agent_hook.rs` uses relative paths). Suspected, same root, not
produced: a `gitdir` file that is not UTF-8 (`read_to_string` fails, the entry is dropped).

### HR8-L-1 (low; ORDINARY WORKFLOW on a slow stop): with three to five seconds left, the slow-lane commit whose turn it is is written down as asked and is never run

Claim 2 and README: "Every commit without a verdict is submitted to the judge at least
once in every N stops … (That holds while the judge can be run, the state can be kept, and
the stop has three seconds left after the working trees' changes …)". `Bench::judge` gives a
run `left − 5 s × owed`. The first reserved ask (the slow lane) has `owed = 1` when a
fast-lane commit follows, so with under five seconds left its run has no time at all: the
judge is not started. `judge_commits` then sees "no verdict" with three seconds or more
still left, takes the `in_turn` arm, counts an attempt and marks the commit with this
stop's number. The fast-lane ask after it has `owed = 0` and runs.

`t11.sh 2.29`, `out/t11.txt`. One commit past five attempts (`TK-a`), one with one attempt
(`TK-b`), sixteen other clean working trees, and a Git whose `status` takes 2.29 s (the
shim), so that the commits are reached with about 3.5 s left:

```
   before: bc009adc:n6@6  7074657f:n1@6
   => allow: UNJUDGED … 2 commits of this session have no verdict: bc009adc5096 (no verdict in 7 attempts; it is
      asked about again in its turn, one such commit at each stop; the last reason: the judge was stopped before it gave a v…
   the stop took 37.7 s
   submissions at that stop (wrapper log):   +37.59 TK-b    +37.61 probe.patch       (TK-a: none)
   after:  bc009adc:n7@7  7074657f:n2@7
```

`TK-a` was not submitted, and the state says it was asked at stop 7 for the seventh time.
With S commits in the slow lane it now waits S more stops; where every stop is this slow,
none of them is ever submitted while the message counts their attempts. It is said at
every stop (not silent). By reading, the same holds for a little over five seconds left:
the run is started and ended before it can answer.

Cheap fix: a run that would get less than some floor (the judge needs about 0.1 s here,
more under load) is not made and not written down, like the `left < COMMIT_FLOOR` arm; or
give the two reserved asks an even share of what is left; and state the precondition as
what it then is.

```bash
W=/tmp/claude-ddf-hooks-review-8; bash $W/t11.sh 2.29
```

### HR8-L-2 (low; ORDINARY WORKFLOW in a large repository with many working trees): other working trees are looked at before the session's own change is judged, with no bound; the own change is then UNJUDGED at every stop

`judge_units` builds the project's patch, then calls `other_trees` (a `git status` in up to
sixteen other trees, each with the whole of the stop's remaining time), and only then
starts the judge. `t8.sh x 2.5`, `out/t8.txt`: a key-like value in the project's own tree,
sixteen other clean working trees, `git status` at 2.5 s (the shim; a large tree with no
file-system monitor):

```
-- stop 1 => allow: UNJUDGED. This change (…) was not judged: the judge was stopped before it gave a verdict. That is not a pass.   (40.8 s)
-- stop 2 => allow: UNJUDGED …                                                                                                    (40.7 s)
```

The session's own credential never blocks, every stop takes 41 seconds, and the reason
says nothing of where the time went. It is not silent. What is looked at there only makes
notes; what is starved is the verdict. This is also the realistic way into HR8-L-1. The
real judge itself is fast (a 30 MiB patch in 1.3 s, `j2.sh`), so the judge is not what
uses a stop's time in practice; Git is.

Cheap fix: judge first and look at the other trees with what is left (count the ones not
looked at, as the trees beyond sixteen are counted); or give that part a bound of its own;
and say "the stop's time was used before the judge could run" when that is the reason.

### HR8-L-3 (low; ORDINARY WORKFLOW, crowded): a failure that stands in a tree beyond the sixteen is not said at a stop from the project

`other_trees` computes `stands` for every tree, but a tree beyond `OTHER_TREES` only adds
to a count. `t3.sh many`, `out/t3many.txt`: eighteen linked trees, a failure that has
blocked in the eighteenth, changes in the others; stops 3 and 4 from the project name
five trees in full, three by name, "and 9 more", among them "2 more working trees that
were not looked at". Nothing says that a failure stands. README: "A stop that did not look
… forgets nothing, names the tree and says at every such stop that a failure stands
there". The memory is right (stop 5 from inside the tree: FAIL STANDS). The test
`a_failure_in_a_tree_beyond_the_sixteen_stands` asks only that it is not forgotten.
From reading, not produced: with sixteen trees or fewer, a tree's "(a failure stands
there)" is on the counting line only while the tree is among the first three names.

Cheap fix: put the trees where a failure stands first among the notes, and count them on
their own ("a failure stands in N of them").

### HR8-L-4 (low; ORDINARY WORKFLOW; a stated limit): a failing line replaced by another under the same rule in the same file, between two stops, stands, and is said to be "as it was"

`t4.sh replace`, `out/t4.txt`: stop 1 blocks for a key-like constant in `lib/cfg.rs`; the
agent deletes the constant and writes another key into another function of the same file;
the next stop (with `stop_hook_active`) says

```
deep-diff-forge: FAIL STANDS. This failure has already blocked a stop of this session (…); it is left for a person.
- rule no-secrets … secret_like in "lib/cfg.rs" …
```

and so does every later stop. The new value has never been put to the agent, and the
sentence is not true of it. This is the identity the documents state: README has it inside
the sentence about the stash ("its lines are a rule, a path and a number, and another
credential in the same file has the same"). How far it reaches: every signal, since the
line is (rule, signal, path) with the number of pieces of evidence; exactly the stop after
a block, when an agent's repair replaces what the rule names by something else the rule
names (the "not accepted" answers of `explain signal`). A line that was gone at any stop
that looked is forgotten, so it does not reach further than one turn. The person sees FAIL
STANDS; the agent sees nothing.

It is stated accurately, but not where a reader looks for it, and the message says more
than the hook knows. Cheap: say it in "A failure blocks once" in plain words, and word
FAIL STANDS as "the same rule still fails on the same path" for a working tree's change.
Not cheap, and with a cost: take the text of the evidence lines into the line's identity
(the advice hook already keys a finding by path, label and line text); then any edit of a
failing line blocks again.

### Smaller things

- A failure of more than 256 lines is one line (stated). A block for a change inside it
  shows the first twenty paths in order and "and N more, not listed": with two new files
  among 258 (`t4.sh big`, step 5) the block does not show what is new. Going from 257 to
  256 lines blocks once more for 256 unchanged lines (stated: "any change in it blocks").
- A state of schema v6 (the build before): every hook of that session says UNJUDGED or
  "did not run" with the remedy "remove the file" (`t7.sh v6`). Right, and said at every
  stop; a session resumed over an upgrade of the binary is unjudged until a person removes
  a file by hand. "or start a new session" would be a kinder second remedy.
- From reading, not produced: a stop that waits three seconds for the lock while a
  session-start hook holds it says "another stop of this session is running".
- `r6/t6.sh` (the sixth review's table, copied): its row "commit: fail, then reverted" did
  not revert here (`git revert -q --no-edit` printed its usage with this Git); the row is
  a plain failing commit. No finding.

## The areas of the brief: what was tried and whether it held

0. **This round's rework.**
   - THE TREE INSTANCE (`t1.sh`, `t2.sh`, `t3.sh`, `t10.sh`; `out/t1.txt`, `t1b.txt`,
     `t2.txt`, `t3.txt`, `t3many.txt`, `t10.txt`). Held: `git worktree move` (stands, from
     inside and from the project); `mv` and `repair` (stands); `mv` with no repair (from
     the project "(gone) (a failure stands there)", from the moved tree it stands, after
     repair it stands); remove and add at the same path with the same text, with no stop
     between (blocks, then stands; new inodes and a new time); `worktree lock` and
     `unlock`; `git gc --prune=now` and `worktree prune`; `gitdir` and `HEAD` touched
     (stands). `commondir` touched: a new identity, one more block, then stands (as
     designed; the old entry is forgotten). The linked tree's directory put back from a
     copy (`cp -a`), the repository untouched: stands. The whole project put back from a
     copy at the same path, and the same with a linked tree: UNJUDGED at every stop, "the
     repository at … is not the one this session started in … start a new session", from
     the tree and from the project; nothing silent, nothing inherited. The project itself a
     linked tree: blocks, stands, the main tree as the second tree blocks for its own and
     says the other stands, and the project's tree removed and added again at its path
     blocks (it is the project still, by the common directory). A bare repository with two
     linked trees: each blocks once for the same text, each stands, a stop from the bare
     directory names the other tree with "(a failure stands there)". A directory reached
     through a symbolic link, project and linked tree: held (Git records real paths for
     `add`, `move` and `repair`; checked). Broken: relative paths (HR8-M-1); a tree beyond
     sixteen (HR8-L-3). By reading: a tree on another file system is the same tree to the
     hook, because the identity is read under the repository's `worktrees/`, not in the
     tree. Not tried: a file system that reuses inode numbers (my scratch is tmpfs, where
     the numbers of the old and new administrative directories differed), whole-second
     times, a submodule's own linked trees.
   - THE LINE IDENTITY (`t4.sh`, `t7.sh`; `out/t4.txt`, `t4b.txt`, `t7.txt`). New taken for
     old: only the stated identity (HR8-L-4). A rename blocks. Two rules on one path: one
     repaired stands, and when it comes back the block says "New at this stop: no-secrets
     …" and "Stands beside it … tests-travel-with-code: "src/n.rs"", both right. A count
     that falls is remembered lower and blocks when it rises. Over 256: as stated. Staged
     against uncommitted against committed: a staged key with the tree repaired blocks as
     "Staged changes" and stands; a second staged file blocks with new and standing lines
     named rightly; staging the repair of one leaves the other standing with no block; the
     tree getting the key back blocks as the tree's; `add -A` stands; index two keys and
     tree one blocks as staged (documented); a commit blocks as the commit; `reset --soft`
     blocks again as the uncommitted change; committing again is another commit and
     blocks: each as README says. Old taken for new: none found beyond HR8-M-1. I did not
     find "New at this stop" or "Stands beside it" naming a wrong line.
   - THE TURN (`t5.py`, `t11.sh`; `out/t5-mix.txt`, `t5-down.txt`, `t5-slow.txt`,
     `t11.txt`). Submissions counted from the wrapper's log, by the argument after
     `--patch`. Held: six commits that never get a verdict, six more arriving one a stop,
     passing commits between, 30 stops: after their five attempts twelve commits are asked
     about one a stop, each once in twelve stops; 0 violations of "once in N". The chain
     down for every patch and the probe, seven commits and three more arriving, 16 stops:
     three asks a stop in rotation, 0 violations. A judge that takes four seconds an ask,
     fourteen commits with no verdict, eight stops of 40.7 s each: each commit asked at
     least every second stop, attempts counted, 0 violations. Broken at the edge of the
     stated precondition: HR8-L-1. The statement about an unwritable state is as the
     seventh review found it; not run again.
   - THE LOCK (`t6.sh`, `t6k.sh`; `out/t6.txt`). Held: 8 and 24 advice hooks at once that
     are all the session's first hook: all exit 0, one state, a tips file that matches the
     state's digest, the base recorded once, a later advice hook silent, the stop blocks
     and says the start was not recorded. The judge inherits no descriptor of the lock. A
     stop killed with `kill -9` while its judge sleeps: the next stop has the lock at once
     (0.2 s), blocks, and the scratch directory is cleared. By reading: an advice hook
     that cannot have the lock in 0.3 s prints and writes nothing, so its finding is new
     again after the next tool call (and never said as advice when no tool call follows;
     the stop judges all the same); a state directory under a home that is a Git working
     tree has no lock on the very first hook only, as stated. Not tried: a file system
     with no `flock`; two first stops at once (the committer's `verify7.sh` shows it).
   - THE MESSAGE (`t15b.sh`, the seventh review's crowded case on this build;
     `out/t15b.txt`, `out/t15b-3.json`). Held: 70 failing commits beside three standing
     tree failures, a staged hold, two held commits and three commits with no verdict,
     paths of 127 and 139 characters: 7,020 characters, 26 commits named, "and 44 more
     commits fail." (26 + 44 = 70), every counting line whole, no cut mark. HR7-L-1 is
     closed. No message over 9,000 characters seen. Text reaching the agent: nothing new
     looked for beyond reading (`kept_reason` replaces `,;:|` and control characters in
     rule names and `|` in the judge's words; they are shown through `display_safe` and
     marked as the judge's words).
   - THE STATED LIMIT (`t9.sh`, `out/t9.txt`). A failing commit fetched before the session
     and brought under `HEAD` by `git pull --ff-only`: the advice hook reports the finding
     once, the stop is silent; the same commit fetched during a session blocks. It is
     stated accurately (README "Which commits of others are judged"; the start text; every
     range sentence). My judgement: it is ordinary (a background fetch, then a pull; a
     switch to an older branch; `reset --hard` to an old commit), but those commits are
     not the session's work, and the two cheap answers both cost more than they give.
     Saying "N commits under HEAD were there before the session and are not judged" at
     every stop makes every stop after a pull or a branch switch speak for the rest of the
     session, which spends the meaning of silence. Judging them like commits fetched
     during the session blocks an agent for other people's old commits after any branch
     switch. If anything is done, the least is one sentence at the first stop after `HEAD`
     comes to name such commits, said once like the note about a start that was not
     recorded. I would leave it as a stated limit; the inconsistency worth a line in the
     start text is the one README already names, that whether a pull is judged depends on
     when its fetch happened.
   - LOAD. The release binary is 8.0 MB (the debug one the tests hash is 95 MB), so the
     eight seconds for hashing rankers are a matter of the test, not of a session. By
     reading, when the rankers cannot be hashed the judge cannot be prepared: every unit
     with a patch is UNJUDGED with that reason, commits wait and are counted; a stop with
     nothing to judge needs no judge and is silent, rightly. The real way a busy machine
     or a large repository starves a stop is Git, not the judge: HR8-L-2.
   - THE TABLE (`r6/t6.sh`, `out/r6-t6.txt`): each kind of unit against each outcome over
     three stops, and the mixed rows, on this build: silent in exactly the all-pass rows;
     BLOCK then FAIL STANDS twice for each fail; HOLD and UNJUDGED at every stop; "no
     verdict once" is UNJUDGED, then BLOCK, then FAIL STANDS. Held.
   - State files (`t7.sh v6`, `t7.sh trunc`; `out/t7b.txt`): a v6 state, an emptied judged
     file and a tips file with a line added are each UNJUDGED with the reason and a
     remedy; a judged file with a line beyond its count is read up to its count. Held.
   - Which commits are the session's through rebase, squash, amend, cherry-pick, merge,
     stash, detached `HEAD`, `update-ref`: only amend, `reset --soft` and pull were run
     here; the rest is as the earlier reviews left it.
1. **A pass that should not be one.** Not gone through again in this round, apart from
   what the cases above pass through (the wrapper judges that exit 3 are UNJUDGED; a
   verdict is taken only from the real judge).
2. **A block that cannot be escaped, or one that is waved through.** HR8-M-1 is a block
   that repeats at every return to a tree; it ends when the directory stops moving. Nothing
   else found.
3. **The hook input and output contract.** Not looked at in this round.
4. **Resources and files.** Time: HR8-L-1, HR8-L-2. A killed stop leaves no lock and its
   scratch directory is cleared by the next hook. Not looked at: disk full, memory.
5. **`agent setup --hooks`, `agent doctor --gate`.** Only read the printed settings
   (time-outs 5, 10 and 60 seconds against the hooks' own 4, 8 and 45). Not tested.
6. **`schema signals`, `explain signal`.** Not looked at.
7. **The tests.** No test makes a working tree with relative paths (HR8-M-1 passes all 95).
   `a_failure_in_a_tree_beyond_the_sixteen_stands` does not ask that the failure is said
   (HR8-L-3). `every_commit_without_a_verdict_is_submitted_in_turn` counts submissions on
   stops that have their time; nothing holds the claim at the edge of its precondition
   (HR8-L-1). I ran no test and no control: no `cargo`.

## Suspected only (not reproduced)

- A `gitdir` file that is not UTF-8: the same as HR8-M-1.
- A file system whose inode numbers are not stable (some network and container mounts): a
  linked tree would be a new tree whenever the number changes, and its failure would block
  again; the project itself would be UNJUDGED ("not the one this session started in"),
  which is older than this commit.
- With sixteen trees or fewer, "(a failure stands there)" is visible on the counting line
  only for the first three names (HR8-L-3, second part).
- A stop that waits for a session-start hook's lock says "another stop of this session is
  running".

## What I did not look at

Areas 1, 3, 5 and 6 of the brief beyond what is said above; the advice hook's text;
`verdict_v2.rs`; `judge_patch.rs` (attributes, filters, sparse checkout, submodules, CRLF);
hostile input; a disk that is full; a file system with no `flock` or with reused inode
numbers; two sessions in one checkout; `CLAUDE_PROJECT_DIR` set wrongly; a random driver
(the seventh review's `fuzz7.py` and `fuzz7b.py` were not run again on this build: neither
removes a tree nor uses relative paths, and their oracle follows the old whole-set
identity, so it would need rewriting for lines); Vesper's round 16 report, if there is
one, which I did not read.
