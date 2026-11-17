# Second independent review of the agent hooks branch (`agent/loop`, commit `69e2fe8`)

Date: 2026-11-14. Reviewer: an independent Claude agent that took no part in the work.
Brief: `~/.local/state/habitat-claude/ddf-gap-probes-20261113/ddf-hooks-independent-review-2-brief.md`.
Under review: `d4dbc91..69e2fe8`, mainly the rework `1c7f8c1..69e2fe8` that answers the first
independent review (`docs/reviews/claude-independent-review-hooks-20261114.md`).

## Verdict: NO-GO

The rework closes the first review's cases as they were written: I ran them again on the
`69e2fe8` binary and each is now a block or a visible UNJUDGED (table under "The first review's
cases"). The cause behind HR-H-2 is only half removed. The base is now fixed, but what is
compared with it is still only the working tree at the moment of the stop. Work the session
committed is judged only while the working tree still holds it. An agent that commits on a
branch and switches back, or commits a staged file after editing it again, stops in silence with
the judge never asked (HR2-H-1). The session is told the opposite at its start: "everything
changed since this session began, committed or not, is judged".

One more silent pass is a plain defect of the rework: the state file is written without a size
limit and read with one, so a large failure makes the session forget its base (HR2-M-1). Three
further medium findings and three low ones follow.

Everything in "Findings reproduced" was run with the frozen release binary and, where a verdict
is needed, the real forge-warden 0.2.0.

## How this was done

- Source read from `git archive 69e2fe8` (extracted to `/tmp/claude-ddf-hooks-review-2/src`).
  No `cargo` was run.
- Binaries: `/var/home/louranicas/.cache/ddf-t/review-bin/v4r10/release/deep-diff-forge`
  (SHA-256 `18ad57e3…574e`) and the `debug` one beside it (used for `t15.sh` only); the judge
  `/var/home/louranicas/.cache/ddf-t/target-fw/release/forge-warden` 0.2.0, inside
  `toolbox run -c fedora-toolbox-44` (Git 2.56.0).
- Scratch: `/tmp/claude-ddf-hooks-review-2/`. `base-repo` is a `git clone --no-local` of the
  first review's base repository with the policy's ranker digest set to the frozen release
  binary and committed (`03838e7`). Every case is a fresh clone under `cases/`. Scripts
  `t1.sh` … `t16.sh` with the helper `h.sh`; outputs in `out/t*.txt`. `/tmp` does not survive a
  reboot, so the commands that matter are repeated below.
- Every run had `HOME=/tmp/claude-ddf-hooks-review-2/home` (or another directory under the
  scratch), `XDG_STATE_HOME` and `CLAUDE_PROJECT_DIR` unset unless the case says otherwise, and
  `GIT_CONFIG_GLOBAL=/dev/null`. After the runs `~/.local/state/deep-diff-forge` of the real
  user does not exist.
- The helper's commands are exactly (`$S` a session id, `$R` a directory):
  - `start R`: `printf '{"hook_event_name":"SessionStart","session_id":"%s","cwd":"%s","stop_hook_active":false}' $S $R | $DDF agent hook session-start --client claude --judge $FW`
  - `post R`: the same with `PostToolUse` and `agent hook post-tool --client claude`
  - `stop R [true]`: the same with `Stop`, `stop_hook_active`, and `agent hook stop --client claude --judge $FW`
  - `secret FILE` writes `pub fn a() {}\npub fn b() {}\npub const K: &str = "AKIAIOSFODNN7EXAMPLE";\n`.
- Controls (`t1.sh`, `out/t1.txt`): in a started session the credential uncommitted blocks
  (`rule no-secrets … secret_like in "lib/a.rs"`); committed, it blocks the same way; a harmless
  change with its test passes in silence; with no session start, a stop after a commit says
  "This session's start was not recorded …" once.
- What Claude Code does (the order of hooks, session ids on resume and clear, the meaning of
  exit codes) is taken from its documentation and the branch's own statements, not from a live
  session.

## Findings reproduced

### HR2-H-1 (high): committed work is judged only while the working tree still holds it; the ordinary branch workflow ends in a silent stop

`stop` compares the recorded base with the working tree (`judge_trees` → `change` →
`judge_patch`). Nothing compares the base with what the session committed. When the tree at the
stop equals the base's tree, `change` returns `Ok(None)`, no judge is looked for, and the stop
prints nothing, whatever the session committed in between and wherever those commits are now.
`head_contains` says "moved" only when `HEAD` no longer contains the base, so these cases are not
even named.

What the documents say: README table, `Stop`: "A verdict on everything changed since the session
began, committed or not"; "wherever the agent has gone with `cd` and whatever it has committed";
SECURITY.md: "**The session cannot move what is judged.**"; the session-start text given to the
model: "everything changed since this session began (commit …), committed or not, is judged".
The stated limit covers only work "committed on another branch *without touching those trees*".

Reproductions, all in a started session, all ending with no output and exit 0:

| Case | What the session did | At the stop |
| --- | --- | --- |
| 2b (`t2.sh`), 11b (`t11.sh`, with `CLAUDE_PROJECT_DIR` set and the branch pushed) | `git checkout -b fix/thing`; credential written; `git commit -am`; `git push`; `git checkout main` | silent; the pushed branch holds the credential. The advice hook had reported `secret_like at "lib/a.rs"` two tool calls earlier |
| 2f (`t2f.sh`) | credential written, `git add -A`; the file edited again without the credential; `git commit -m` (no `-a`) | silent; `HEAD:lib/a.rs` holds the credential, the working file does not, `git status` shows ` M lib/a.rs` |
| 2c (`t2.sh`) | `git commit -am work`; `git checkout BASE -- .` | silent; `HEAD` holds the credential |
| 2d (`t2.sh`) | new file committed; `git rm --cached`; `build/` added to `.git/info/exclude` | silent; `HEAD:build/gen.rs` holds the credential |

2b and 2f are honest mistakes, which is the model the README names ("The hooks guard against
honest mistakes"); 2c and 2d need intent. `t15.sh` repeats 2b and 2f with the frozen debug
binary: the same.

```bash
# 2b, inside the toolbox
W=/tmp/claude-ddf-hooks-review-2; . $W/h.sh          # private HOME, DDF, FW, start/post/stop/secret
R=$(fresh t2b); SESSION=t2b; start $R >/dev/null
git -C $R checkout -q -b feature; secret $R/lib/a.rs; git -C $R commit -qam work; post $R
git -C $R checkout -q main; post $R
stop $R        # -> [exit 0] (NO OUTPUT)
git -C $R show feature:lib/a.rs | grep -c AKIA       # -> 1
```

```bash
# 2f
R=$(fresh t2f); SESSION=t2f; start $R >/dev/null
secret $R/lib/a.rs; printf '#[test]\nfn t() {}\n#[test]\nfn u() {}\n' > $R/tests/t.rs; git -C $R add -A; post $R
printf 'pub fn a() {}\npub fn b() {}\npub fn c() {}\n' > $R/lib/a.rs; post $R
git -C $R commit -qm "add c"; post $R
stop $R        # -> [exit 0] (NO OUTPUT);  git show HEAD:lib/a.rs holds the credential
```

Suggested direction: at a stop, also judge what was committed. Cheapest form: when the tree of
`HEAD` differs from the tree the scratch index wrote, judge `BASE..HEAD` as a second patch (the
worst answer decides, as for two working trees). For branches: record the local refs at the
session's first hook and, at a stop, judge (or at least name as UNJUDGED) every local branch
whose tip changed since then and is not contained in `HEAD`. Until then the three documents and
the session-start text must stop saying "committed or not" without the condition.

### HR2-M-1 (medium): a state file the hook writes can be larger than the hook will read; the next hook then forgets the base, and what was just failed and committed passes in silence

`save` writes the whole state; `load` returns an empty state for a file over `STATE_CAP`
(1 MiB). `last_block` holds one line for every violation of the failure that blocked (rule,
signal, path, count) and has no bound; the comment on `STATE_CAP` assumes "a block of a few
rules". After a block with a large identity the session's own state is unreadable, the next hook
is "the session's first", and it records the commit `HEAD` names then.

Reproduction (`t4.sh`, `out/t4.txt`; real judge; 4,000 new files of 239-character paths, each
with the credential):

```
 -- stop 1:            {"decision":"block", … rule no-secrets …}
   state file bytes: 1139092; base in it: 03838e7
 -- the agent commits everything (a tool call), post-tool hook:   (NO OUTPUT)
   state file bytes: 202; base in it: b9c6def; HEAD is b9c6def
 -- stop 2 (stop_hook_active true):   (NO OUTPUT)
 -- stop 3 (next turn):               (NO OUTPUT)
```

The same fail that blocked one stop earlier is neither "FAIL STANDS" nor a block; 4,000
committed credentials are a silent pass. This contradicts SECURITY.md: "no state turns a judged
fail into a pass". The size is 285 bytes for each violation here; with 40-character paths the
cap is reached at about 12,000 violations (computed, not run). The integration test
`the_state_is_kept_in_a_directory_of_its_own_and_read_with_a_bound` asserts "a state file over
the bound is not read: the same as no state", which was harmless when the state held only what
had been said and is not now that it holds the base.

Suggested direction: bound what is written (store a hash of the identity, as `noticed` does), and
treat an unreadable or oversized state of a session as trouble that is said, not as a first hook.

### HR2-M-2 (medium): a start that was lost or never recorded is silent whenever a post-tool hook runs before the stop, which is always when the three printed hooks are installed

Only a stop says "This session's start was not recorded". `post_tool` records the project and
the base in the same way (`locate`, `fresh: true`) and says nothing. A commit is made by a tool
call, and the post-tool hook of that call runs before the stop. So with the settings
`agent setup --hooks` prints, the first-stop rule cannot fire for a commit the agent made; it
covers sessions that call no tool.

Reproduced (`t2.sh`, `out/t2.txt`):

- 2e, no session start ran (hooks without `SessionStart`, or a session that began outside the
  repository and went in): credential written and committed, `post` → no output, `stop` → no
  output.
- 2a, a started session whose state file is removed in the same tool call as the commit
  (`git commit -am work; rm …/session-*.json`): `post` → no output; `stop` → no output; the
  state now holds the new `HEAD` as base. The README and SECURITY.md state that the state file
  is the agent's to edit and "not a boundary"; what they also state is "A lost file makes the
  next hook the session's first" and "A stop that is its session's first hook is never silent".
  Together those read as "a lost state is said", and it is not.
- 6f (`t6.sh`): the same damage with no post-tool hook in between is said by the stop, which
  shows that the silence comes from the post-tool hook taking the "first" place.

`agent doctor --gate` reports every link `ok` when only the `Stop` hook is registered (`t10.sh`
case 10a). The gate reads only `hooks.Stop` (`stop_hooks`), so by the code the same holds when
`PostToolUse` and `Stop` are registered without `SessionStart` (not run): the link the judged
range now depends on is not checked.

Suggested direction: keep in the state which hook recorded the base; a stop adds the "start was
not recorded" sentence, once, whenever it was not a session-start hook. Have the gate check that
a `SessionStart` hook of this binary is registered.

### HR2-M-3 (medium): a repository other than the recorded project is never judged and nothing is said, also when the session's directory is in it at the stop

The first review's HR-H-1 was "the project is not judged when `cwd` is elsewhere". The rework
judges the project and ignores every `cwd` that is not a linked working tree of the same
repository (`other_tree` returns `None` for "another repository"). Nothing is said about the
repository the session is standing in. README: "What is judged is two working trees at most: the
project's, and the one the session's directory is in at the stop when that is another working
tree of the same repository"; SECURITY.md shortens this to "They judge the project's working tree
and the working tree the session's directory is in."

Reproduced (`t3.sh`, `out/t3.txt`; `t6.sh` 6e):

- 3a: the session starts in a plain directory that holds repositories `A` and `B` (no
  `CLAUDE_PROJECT_DIR`): session-start says nothing. A post-tool hook with `cwd` in `A` records
  `A` as the project, in silence. The credential is written in `B`; post-tool and stop with
  `cwd` in `B`: no output. 3b is the same with `CLAUDE_PROJECT_DIR` naming the plain directory.
- 3c: the project has a real submodule; the credential is written inside it and not committed.
  Post-tool and stop, with `cwd` in the submodule or at the top: no output (`git status` of the
  project: ` M vendor/sub`). Committed inside the submodule it becomes a HOLD
  (`note patch_unverifiable: unsupported file mode 160000`), which is visible.
- 6e: a session id whose state names repository 1 gets a session-start with `cwd` in
  repository 2: the text says "deep-diff-forge is watching the changes of this session" and
  names no directory; the stop in repository 2, with the credential there, is silent.
- 3d (held): `CLAUDE_PROJECT_DIR` naming another repository than the recorded one is UNJUDGED
  with both paths named.

Suggested direction: when `cwd` at a stop (or after a tool) is in a Git working tree that is
neither the project nor a linked tree of it, say so once ("changes in … are not judged by this
session's hooks"); say the same for a submodule whose working tree is dirty. Name the project's
directory in the session-start text.

### HR2-M-4 (medium): with a fixed base, a failure stays for the whole session: it blocks the first stop of every later turn, cannot be repaired when it is in the base's policy, and includes other people's commits

`last_block` is cleared by the allowed "FAIL STANDS" stop, the base never moves and the policy is
read from the base. Before the rework a commit ended a failure (that was HR-H-2); now nothing
inside the session does.

Reproduced with the real judge:

- 5a (`t5.sh`): a credential fails, blocks, stands; the person accepts it and it is committed.
  Turn 2 and turn 3, with nothing changed: `decision: block` again each time, then "FAIL STANDS".
  Every turn of the rest of the session costs one forced continuation with "Repair what the
  rules name".
- 5b (`t5b.sh`): the base policy holds an expired waiver. A harmless change is blocked ("No rule
  of the change failed … Tell them, then stop again"). The waiver is removed and committed in
  the session. Turns 2 and 3 are blocked again with the same text: the policy that judges is the
  base's. The session-start gate reported the chain as fine. The first run of `t5.sh` shows the
  same for a policy the judge refuses (my waiver's `reason` was too short): UNJUDGED at every
  stop, also after the policy was repaired and committed.
- 5c (`t5.sh`): `git pull --ff-only` during the session brings a commit by somebody else that
  holds a credential. Post-tool: "1 new finding in your changes since this session began"; stop:
  `decision: block … Repair what the rules name`. `HEAD` still contains the base, so nothing
  says the range holds foreign commits.
- 11c (`t11.sh`): two sessions in one checkout. Session A's accepted and committed failure
  blocks session B, whose own change is one comment line, with the instruction to repair it.

The README says that a later start of the same session "keeps the commit", so a session resumed
after days judges, and is blocked for, everything that landed since.

Suggested direction: remember a failure the person let stand (same identity) for the session and
do not block on it again at the next turn; say in the message when commits in the range were not
made in this session's tree (author or committer differs, or `HEAD` advanced by a merge or a
fast-forward); let a policy repair take effect by saying how (a new session) in the block text.

### HR2-L-1 (low): `agent doctor --gate` says every link is ready where each stop is UNJUDGED for want of a state directory

`t9.sh` case 9e: with neither `HOME` nor `XDG_STATE_HOME`, `agent doctor --gate` prints ten `ok`
lines and exits 0; the stop of a session in the same environment says "UNJUDGED. The stop hook
did not run: there is no state directory". The gate has no link for the state directory (present,
a directory of its own, writable), which the rework made a precondition of every hook. Together
with the missing `SessionStart` check (HR2-M-2) this is HR-M-4 again for the two links the
rework added.

### HR2-L-2 (low): when the state cannot be opened or saved, the advice hook tells the user "the hook did not run" after every tool call

`notice` needs the state, so the once-per-session rule does not cover the state's own failures.
`t6.sh` 6a (no `HOME`): three post-tool hooks, three identical messages. 6c (state directory
made read-only after the start): the same, although the hook did run and only the save failed.
This is HR-L-2 for the condition the rework introduced.

### HR2-L-3 (low): the session-start text does not name the project it recorded

The text names the base commit and not the directory. With a recorded project that is not where
the session is (6e, 3a after the first post-tool hook) the model is told its changes are watched
while another repository is. One path, shown as paths are shown, would let a person see it.

## Suspected only (not reproduced)

- **Client behaviour.** That a post-tool hook always runs between an agent's commit and the stop,
  that `resume` and `compact` keep the session id and `clear` changes it, and that an `async`
  hook entry cannot block (the gate does not read `async`) come from documentation, not from a
  session.
- **`XDG_STATE_HOME` inside the working tree** (a per-project environment): the state file would
  be an untracked file in the judged patch, and `git clean -fdx` would remove the base as in 2a.
  Read from the code, not run.
- **HR2-M-1 at ordinary path lengths**: about 12,000 violations are needed; computed from the
  measured 285 bytes for each, not run.
- **Vesper's VD4R10-H-1** (the project moved and cloned back at its recorded path) is in the
  untracked `vesper-v4-round10-report-20261114.md`; I did not run it. It is separate from the
  findings here.

## The first review's cases on `69e2fe8` (`t8.sh`, `out/t8.txt`)

| First review | Now |
| --- | --- |
| HR-H-1 T1a nested repository, no commit | UNJUDGED, visible (`git add` fails on it; stated limit) |
| HR-H-1 T1d nested with a commit, T1b gone, T1c outside | `decision: block` |
| HR-H-1 T8f change in a linked tree, `cwd` in the main tree | silent (stated limit); with `cwd` in the linked tree: block, both trees named |
| HR-H-2 T2a committed credential | block |
| HR-H-2 T2b policy rule deleted and committed, then a credential | block, by the base's policy |
| HR-M-1 8a no Git, 8b dubious ownership, 8c in `.git`, 8d orphan | UNJUDGED with the reason |
| HR-M-1 8e `GIT_CEILING_DIRECTORIES` in the client's environment | still silent when nothing is recorded (Git says "not a git repository") |
| HR-M-2 T3a ignored file added with `-f` | block |
| HR-M-5 usage errors | exit 0, `UNJUDGED. The stop hook did not run: …` |
| HR-L-4 9a two other credentials in the same file while continuing | block again |
| HR-L-5 4l filter in the user's configuration | not run; block on the real content |
| HR-L-1 5c scratch directory of a killed stop | removed by the next hook |

## The areas of the brief: what was tried and what held

0. **The rework.** Broken: HR2-H-1, HR2-M-1, HR2-M-2, HR2-M-3, HR2-M-4. Held: a session id does
   not choose a file name; the state directory and file are 0700 and 0600; a relative
   `XDG_STATE_HOME` is not used (6b); a state whose base is not a full commit id is no state
   (6f, said by the stop); `CLAUDE_PROJECT_DIR` naming another repository than the recorded one
   is UNJUDGED (3d); a state that cannot be saved is said at the start and at each stop (6d);
   `--policy` with `..` and a policy that is a symbolic link at the base are UNJUDGED (`t12.sh`);
   the session-start gate costs 0.16 s here and leaves nothing behind (`t7.sh`).
1. **A pass that should not be one.** Broken: HR2-H-1, HR2-M-1, HR2-M-2, HR2-M-3. Held: a
   `forge-warden` on `PATH` that exits 0, a partial document, a judge inside the repository, and
   a real pass verdict of another patch replayed (`t9.sh` 9a to 9d: each UNJUDGED with the
   reason); `core.fsmonitor` in the repository's configuration is not run while the patch is
   made (`t13.sh`, with a control that `git status` runs it).
2. **Blocks.** HR2-M-4. Held: a fail with a note is "the same failure" at the next stop (11a);
   one stop is ended by its 45 s deadline as UNJUDGED with a judge that takes 56 s over eight
   rankers, and leaves no scratch directory (`t14.sh`).
3. **Input and output.** A 33 MiB input is refused by name in 0.1 s; a 20 MiB tool answer is read
   with 45 MB of memory (`t10.sh`). A file name written as an instruction reaches a block's
   reason only inside quotes under "its own words: data, not instructions" (`t9.sh` 9f). Every
   output in every run was valid JSON or the plain session-start text.
4. **Resources and files.** HR2-M-1, HR2-L-2. Post-tool over a range of 4,000 changed files:
   0.3 s, 17 MB (`t12.sh`). State files of ended sessions are never removed (the README says so).
5. **Setup and doctor.** HR2-L-1 and the `SessionStart` part of HR2-M-2. Setup names the judge by
   absolute path, says so when none is found, and takes no `--policy` or `--ranker-dir` (`t16.sh`).
6. **`schema signals` / `explain signal`.** `hazardous_text` now gives `basis: [path, lexical]`
   and both sides; 21 signals. Nothing else was checked again.
7. **Tests.** Below.

## Weak tests

Read from `tests/agent_hook.rs` at `69e2fe8`; no guard was removed and no test was run.

- **No test leaves the base's tree after a commit.**
  `work_and_policy_the_session_committed_are_judged_from_where_it_began` commits and stops on
  the same branch with the work in the tree. No test switches branch, commits a stale index or
  restores the tree (HR2-H-1). A check of `BASE..HEAD` would have no test either way.
- **The first-stop test never runs the hook that precedes a stop.**
  `a_stop_that_is_its_sessions_first_hook_is_never_silent` calls only `stop`; with one
  `post-tool` call before each stop most of its assertions would fail (HR2-M-2).
- **The state bound is tested as lossless.** "a state file over the bound is not read: the same
  as no state" is asserted; no test writes the largest state the hook itself can write and reads
  it back (HR2-M-1).
- **Two refusals in `locate` have no test**: "this session began in …, and the project found now
  is …" and "… is no longer a Git working tree" appear in no test. If `locate` silently took the
  newly found project, every test would pass.
- **No test has a second repository or a submodule** (HR2-M-3), and none runs more than one
  turn after "FAIL STANDS" (HR2-M-4).
- **Would fail if the guard were removed** (by reading): the recorded project
  (`the_project_is_judged_wherever_the_session_has_gone`), the recorded base and the base's
  policy (`work_and_policy_…`), the moved and the missing base, the linked working tree, the
  staging of indexed paths, exit 0 for usage errors, and the verdict fixtures recorded from
  forge-warden 0.2.0.

## Not looked at

- A live Claude Code session; macOS; Git older than 2.56; SHA-256 repositories; partial clones;
  Git LFS; hooks running in two process-id namespaces on one repository.
- The verdict checks of rounds 1 to 9 beyond the four runs in `t9.sh`; the ranker seal helper;
  `verdict_v2.rs` (not changed by the rework).
- `vocabulary.rs` beyond the `hazardous_text` entry; the MCP server, the daemon and the ranking
  code outside what the hooks call.
- Concurrency (two hooks of one session at once), a full disk, network file systems.
- `cargo` was not run: no test suite, no mutation run.
