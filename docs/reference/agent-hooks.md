# For agents: told while it works, judged when it stops

**What the hooks are for.** They guard against an agent's honest mistakes and shortcuts: a
credential it commits, a check it silences, a policy it loosens. They are not a boundary
against an agent that sets out to evade them: it has a shell, so it can edit the settings that
install the hooks, the session's state, or do its work in a copy. That boundary is the judge
on the pull request (forge-warden on the server). So one rule sorts every case: an ordinary
workflow must never end in a silent stop with unjudged work; a deliberate evasion is made
visible where that is cheap, and is otherwise a stated limit (the list at the end of this
section is sorted that way).

**The rule of the stop hook: a stop prints nothing only when every unit of the session has a
pass.** The units are the uncommitted change, the staged change and each commit of the
session. Everything else that is true at a stop is said at that stop, every time, in a
bounded form: what fails and blocks; what fails and stands; what the judge holds; what has
no verdict (a commit that waits, one that is asked about again, one that cannot be judged);
and what is named beside the verdict.

A review at the end comes too late for an agent: it works one tool call at a time. With
Claude Code's hooks, this binary tells the agent what its own change looks like while it is
still working, and asks a judge before it stops. `agent setup --hooks` prints the settings and
writes nothing:

```bash
deep-diff-forge agent setup --client claude --hooks   # the MCP registration, then a `hooks` block for .claude/settings.json
```

| Event | Command | What the agent gets |
| --- | --- | --- |
| `SessionStart` | `agent hook session-start` | What the hooks do, the commit the session's changes are measured from, and whether a stop can be judged here: the chain is checked the way `agent doctor --gate` checks it, and a link that is missing is named instead of a promise. |
| `PostToolUse`, after every tool | `agent hook post-tool` | **Advice, never a block.** The evidence that is new in the working tree since the session began (commits pulled or made by others in that time are in that range too): label, path, line, how the line was read (the rung of the ranking that found it: `lexical` for one line, `hunk` for tokens read across the lines of a hunk), and the change that answers the signal. A signal of a file that has no row of evidence (`hazardous_text`, `executable_bit`, `symlink_change`, and a code signal the ranking's 32 rows left out) is said for the file. A finding is said once per session, also when the line moves. When advice cannot be given (a file name that is not UTF-8, a sparse checkout, Git refusing the repository), the user and the model are told once that it is off and why. |
| `Stop` | `agent hook stop` | A verdict on your uncommitted and staged changes, and on each commit made since the session began that your branches, tags, stash, `HEAD` or their history name: each commit once, by its own change. **Only a fail holds the stop**: a commit's at the stop that first judges it, an uncommitted change's once while it is there. The message has the rules that failed and what answers them. A hold is reported for a person. With no judge, no policy, or no ranker the policy admits, the result is **UNJUDGED**, said plainly, and never a pass. |

- **One project and one base for a session.** The session's first hook fixes three things:
  the project (the directory Claude Code names in `CLAUDE_PROJECT_DIR`, else the hook
  input's `cwd`), the commit `HEAD` names there (the base), and the id of every commit that
  something names at that moment: every ref (branches, remote-tracking refs, tags, the
  stash, whatever else), the `HEAD` of every working tree, and every entry of the reflogs of
  those `HEAD`s and of the stash. The first two are in the session's state; the ids are in a
  file beside it (ids only, one a line, no bound that a repository reaches: a million), and
  the state holds that file's SHA-256 and length, so a list that was cut or changed is not
  believed. Every later hook of the session judges that project, wherever the agent has gone
  with `cd`: a stop from a nested repository, another repository, a directory outside any,
  or one that is gone still judges the project. A directory that is gone is in the tree
  the nearest directory above it that is there is in: from a build directory removed
  inside a linked working tree, that tree is the one the stop is in, and it is judged. The project is a repository at a path: it
  is known by the inode of its Git directory and by where it stands. A clone put at its
  path is not it, and neither is the project once it was moved: the stop is UNJUDGED, says
  which, and says the remedy (start a new session). The device number is no part of the
  identity, so a reboot does not end a session.
- **What a stop has judged: units.** First the uncommitted change of the project's working
  tree: the tree, untracked files included, against the commit its own `HEAD` names. Then
  the staged change of that tree, what the index holds against the same commit, when it is
  another change than the tree's: what was staged and then repaired or deleted in the
  working tree without `git add` is what the next plain `git commit` records, so it is
  judged ("Staged changes in" the path). With everything staged and nothing changed since,
  the two are one patch and one run of the judge. When the session's directory is in
  another working tree of the same repository (a linked worktree it entered), that tree's
  uncommitted and staged changes as well. Then each commit of the session that has no
  answer yet, the oldest first, each as the patch from its first parent to itself (a commit
  with no parent: from nothing). A commit is the session's when your branches, your tags,
  the stash, the `HEAD` of a working tree, or the reflog of one of those `HEAD`s or of the
  stash leads to it now, and nothing that was named when the session began does. So a
  commit is judged for what it holds, whatever came after it: a credential that was
  committed and then reverted, or removed by a second commit; a commit on a branch the
  session left; a commit of a staged file that was edited again; a stash, with the commit
  that holds its untracked files (`git stash -u`), and the older entries of the stash; a
  commit made on a detached `HEAD` that was left; a commit in another working tree; a merge
  commit the session makes, against its first parent, so with what the merge brought in.
- **Which commits of others are judged.** A commit that was already in the repository when
  the session began, named by any ref (remote-tracking refs included) or by a reflog, is
  not the session's, also when the session then pulls it, merges it by fast-forward,
  rebases onto it or resets to it: it was there before, and it is not judged at a stop. So
  whether a pull is judged depends on when its fetch happened: a commit fetched during the
  session and then brought onto a local branch is the session's and is judged; one that an
  earlier fetch (an editor's background fetch, another session) had already brought is
  not. A commit that only a remote-tracking ref names is never the session's. Every message
  that names the session's range says this, the advice after a tool call included: advice
  reports what is in the working tree since the base, also from such a commit, and that is
  no promise that a stop judges it. All units are judged by the policy at the base. The
  same patch is judged once in a stop. Each commit that has its answer is kept with it in a
  file beside the state, which has no bound a session reaches, so a commit is asked about
  once in a session.
- **How much one stop judges, and a commit that gets no verdict.** The uncommitted and
  staged changes first; then two commits whose turn it is (below); then the commits never
  asked about, the oldest first; then the other commits that had no verdict before, the
  one asked about least recently first; all within the stop's time
  (45 seconds less four, and no new commit with less than three left). Commits that are not
  reached are counted in the message ("12 commits of this session: UNJUDGED … they are
  judged at the next stop"). **A commit that gets no verdict is not done.** It is named at
  every stop while it has none ("2 commits of this session have no verdict: ab12cd34ef56
  (the reason), …": the number and the first three with the latest reason) and it is asked
  about again: at every stop for its first five attempts that count, and after them in its
  turn, one such commit at each stop. It is never given up: a judge that fails on one patch
  for a while and then recovers gives its verdict at a later stop, and a fail then blocks
  like any first judgment. Such a commit is named as "no verdict in 7 attempts; it is asked
  about again in its turn". **Every commit without a verdict is submitted to the judge at
  least once in every N stops, N being the number of the session's commits that have no
  verdict, and a commit that was never asked about at the first stop that owes it an
  ask**: each stop first asks about the commit that has waited longest for its turn
  among those past five attempts, then about the one asked about least recently among
  those with fewer, then about the oldest commit that was never asked about, and only
  then about anything else. These are the asks a stop owes, three at most, and none of
  them can take another's time: an ask leaves 5 seconds for each ask owed after it, so a
  judge that hangs on one commit, or on two, cannot keep the next from being submitted.
  (A run of the judge has 30 seconds at most. A run that was cut is written down as
  asked and cut, with the time it had: its part of what was left, all that was left, or
  the 30 seconds.) That holds while the judge can be run, the state
  can be kept, and the stop has 13 seconds of its judging left when it comes to its
  commits (3 for the first ask it owes and 5 for each of the two after it), after the
  changes of its working trees are judged. With less, the asks are made in that order
  while 3 seconds are left, each with 3 seconds. A run of the judge on a working tree's
  change leaves the asks what they need (13 seconds when three are owed); Git taking long
  to read the working tree or the commits can use them. An ask that is not
  made is not written down: no attempt is counted, the commit keeps its turn, and the stop
  says of it "not asked at this stop for lack of time". An ask that was made is written
  down with what the judge said, whatever time is left after it, so the turn moves on: a
  session whose stops all come to their commits with a few seconds left asks about its
  commits in turn, not about the same one each time. An attempt is counted, and a turn
  used, for a commit the judge was run for, and only for such a one. Nothing of a stop is written before it
  ends: a stop that is killed leaves
  every commit where it was, its turn too. An attempt counts at a stop where
  some patch got a verdict; where an ask failed and none did, the judge is asked once about
  the probe `agent doctor --gate` uses (after three failed asks, or at the end of the
  asking), and with no verdict on that either the stop asks no more: the rest are counted,
  and nothing is given up. Only what the hook itself proves from the commit is final at
  once, and it too is named at every stop: a commit whose patch is over the size the
  builder makes (a commit does not change). A commit at the edge of a shallow clone, whose
  parent the repository does not have, is named at every stop while that is so and nothing
  is written down for it: after `git fetch --unshallow` it is judged like any other.
- **What a stop's message holds, in this order.** What blocks now: the staged change, the
  uncommitted change, then the commits, the newest first (the session's latest work before
  pulled history). Each with its rules and what answers them while there is room; then
  with each rule's name, its first path and the number of its other paths; then, for up to
  twenty more, its name with the names of its rules; the rest are counted. A rule's name
  is never dropped for its explanation, and a path that is cut keeps its file name. Then a
  working tree's change that got no verdict and the commits that wait, with the reason;
  then what the judge holds that is new at this stop, with its rules. A unit that fails
  and also has rules the judge holds for shows both, the failing ones first and the others
  marked "held, for a person". Then one line each, at every stop: the commits with no
  verdict, what the judge held at earlier stops, and the failures that stand (by name while
  there are three or fewer, else "13 earlier failures stand: Commit … (no-secrets), and 10
  more."); each unit these lines name comes with the names of its rules (whole names, and
  "+2 more" for those that have no room), or with the judge's own first words when it was
  held or failed for something that is no rule (a note, an expired waiver); both are kept
  with a commit's answer for the rest of the session. A message is at most 9,000
  characters. The lines of every stop are never cut: their room is taken first (each is a
  count and at most three names of a bounded length), what is above them is made inside
  what is left, and its own count ("and 42 more commits fail.") is inside that too, so
  every unit is named or counted. What the hook names beside the verdict gets the room
  that is left (a thing that has no room to be said in full is counted on its line, and is
  said in full at the first stop that has room), and a text that is still over the bound
  is cut where it is finished and ends with a line that says so. The first line
  is the worst answer that is new or still open at this stop: a block, else UNJUDGED, else
  HOLD, else FAIL STANDS.
- **The judge is [forge-warden](https://github.com/Louranicas/forge-warden)**, in its authority
  mode: `forge-warden check --policy P --patch F --ranker R --json`. DDF does not judge.
  `agent setup --hooks` finds it once and writes its absolute path into the commands as
  `--judge PATH`, so the program is chosen by whoever installs the hooks and not at every stop
  by `PATH`. Without `--judge` it is looked up in the directories of `PATH` that are named by
  an absolute path. A judge inside the repository it would judge is not run, however it was
  named: the change could have written it.
- **A verdict is an exit code and a document that agree.** Exit 0, 7 or 8 counts only with a
  whole `forge-warden.verdict.v2` or `forge-warden.verdict.v3` document, each checked member
  by member against the schema forge-warden ships for it (required members and their nested
  members, patterns, closed sets of values, the exit triple that belongs to the verdict, a
  summary that counts the rules, no key given twice). It must say the same outcome as the
  exit code, hold no rule stricter than that outcome, be sealed to the policy, the patch and
  the ranker this hook gave (their SHA-256), be on a ranking the judge produced itself, and be
  on a Git the machine's owner enrolled. Anything else is no verdict: a program that is merely
  named `forge-warden` and exits 0 does not pass a stop, and exit 0 or 8 with such a document
  is UNJUDGED. The seals do not prove who wrote the document; they bind a real judge's answer
  to these inputs.
- **Exit 7 is the judge's fail, and a fail holds the stop even when its document cannot be
  read as a verdict** (no document, a schema this build does not read, a document that is
  not whole, not sealed to these inputs or that says another outcome): the stop is held once,
  like a fail with no failing rule, and the message says that the verdict could not be read,
  why, and that which rules failed is not known; the failure then stands for a person.
- **What v3 adds is read for what it means.** Claims that forge-warden bound (`verified:
  true`, with `base`, `head` and `merge_base` of one length, 40 or 64 hex digits) are accepted
  in that form only. The hook gives no `--repo`, so it cannot check them and does not use
  them: claims never change an outcome, and the verdict is bound to the hook's patch by the
  seals alone. A rule of kind `builtin` must have a `builtin:` id (and only it may), be a hold
  and name no waiver; it is shown like any other rule the judge holds for, by that id at
  every later stop of the session too. A later schema (`forge-warden.verdict.v4`) is no
  verdict, and the reason names it.
- **Under a policy that requires verified claims, a stop can at best hold.** forge-warden
  holds a change whose claims it cannot verify (`[claims] require_verified`; its `init` writes
  that from 0.3.0), with the note `claims_unverified`. The hook gives the judge no `--repo`,
  `--base` or `--head`, so under such a policy every unit of a stop holds and none passes.
  An uncommitted or staged change can never be verified, whatever a later hook gives: it can
  at best hold, so commit first. A hold is said at every stop and never blocks; it is for a
  person. `agent doctor --gate` says it in the link `claims_verified`, read from the notes of
  the judge's verdict on its probe (not from the policy file), and a session's start names it
  as a weak link: stops are judged, and they hold.
- **The policy is the one at the commit the session began from** (`warden.toml`, or
  `--policy NAME`), read from Git, never from the working tree: a change cannot loosen the
  policy it is judged by, and committing the loosened policy does not make it the policy
  either, for as long as the session lasts.
- **The patch is the one a judge would be given for the commit.** The working tree, with
  untracked files and without ignored ones, is staged and diffed against the commit its
  `HEAD` names (advice after a tool call: against the commit the session began from) in a
  new, empty Git directory that only borrows the object store, the
  way forge-warden's patch maker diffs two commits, with its options (full index lines,
  `--submodule=short`, `--ignore-submodules=none`, the borrowed store's commit-graph and
  multi-pack index not read): a test runs a recorded copy of its `scripts/make_patch.sh`
  and holds the bytes equal. The index lines are full from 0.10.0, so the `patch_sha256` a
  verdict seals differs from 0.9.0's for the same change. The judge makes its patch with no
  attributes, so a file a repository hides from `git diff` is read as text there and finds
  the same evidence as `--git-working-tree`; only the Git selection raises
  `diff_suppressed`. A file that the repository's index lists
  is staged whatever an ignore rule says: one added with `git add -f` is in the next commit,
  so it is in the patch. Staging reads no configuration: not the repository's, not the
  user's, not the system's, and no attributes file of the user, so no filter any of them
  defines runs; the diff reads no attributes at all. One thing of the user's is carried over
  by hand, the ignore file (`core.excludesFile` in the user's configuration). For a state
  that is committed with no conversion the bytes are equal (a test holds this). Nothing
  under the repository's Git directory but the object store, the list of paths in its index
  and `info/exclude` shapes the bytes: not its configuration, not `info/attributes`. A
  `.gitattributes` file in the working tree does convert content, and is itself in the
  patch. Where configuration would make a commit differ (`core.autocrlf`,
  `core.fileMode=false`, a filter), the judged tree is the working tree's bytes and modes.
  Making the patch writes nothing in the repository: the index and the objects are in a
  scratch directory under the Git directory, removed afterwards. A working tree with paths
  marked skip-worktree (a sparse checkout) is not judged: its absent files would read as
  deletions, so a stop is UNJUDGED with that reason and advice says so once.
- **The ranker is this binary first**, then the files in `--ranker-dir DIR` that come first
  by name, eight candidates in all. forge-warden
  admits a ranker only if the policy lists its SHA-256, so a newer DDF than the policy admits
  can still have its stop judged by an older, admitted one.
- **A failure blocks once.** A commit that fails holds the stop that first judges it. After
  that it stands: every later stop names it ("Commit 0123456789ab: FAIL, which stands") and
  does not block, because a commit cannot be made good by a later one and is for a person.
  A branch switch, a new ref or another working tree does not make it block again: it is
  known by its id. The failure of an uncommitted change is its lines: each rule, signal
  and path that fails, with the number of items the judge cites for it. A stop blocks for
  a line that has not blocked before, and for a line whose number of items has grown; the
  lines that have blocked stand, however many of the others are repaired, and when both
  are there the block says which lines are new and which stand. **A line is not a line of
  a file.** It is what the judge's verdict gives: a rule, a signal, a path and a number.
  "Stands" therefore says that this rule fails on this path as many times as before (or
  fewer), not that the lines of the file are the same ones: when a finding is removed
  and another that the same rule names is written into the same file between two stops,
  the stop says FAIL STANDS, and the new finding has never been put to the agent. (The
  post-tool advice does show it: advice is keyed by the text of the line.) The verdict
  holds nothing that would tell the two apart and stay the same when the file is edited
  above them: its evidence is a hunk's address and a line number. Every
  message in which something of an uncommitted change's failure stands, or stands beside
  what is new, says this in a line of its own, whatever heads the message (FAIL STANDS, a
  HOLD or UNJUDGED head, or a block). A stop that looked at the
  working tree remembers what it saw: a line that is gone is forgotten, so it blocks again
  if it comes back, and a number that fell is remembered as the lower one. A stop that did
  not look (the session's directory was not in that linked working tree) forgets nothing,
  names the tree and says at every such stop that a failure stands there. Such a stop
  looks at that tree with `git status`, which is not the reading the judge's patch is
  made by: it does not show a change in a file marked assume-unchanged, nor one that a
  clean filter or the line-ending settings of the repository hide (the judge's patch is
  made with the repository's filters off). So where something of the tree is
  remembered, `git status` saying nothing forgets nothing: the tree is read the way a
  stop from inside it reads it, and only that reading finding nothing forgets; a change
  it finds names the tree, and a tree it cannot read (a path marked skip-worktree) is
  named as one that could not be looked at. What is staged
  is a failure of its own only in the lines the working tree's failure does not have: a
  credential that blocked while it was staged and in the working tree blocks again, as
  "Staged changes", when the file is repaired and not staged again, and the message says
  what clears it (`git add` of the repaired file, or `git restore --staged`); it then
  stands for as long as the index holds it, whatever the working tree does meanwhile.
  Staging, and editing other files, does not make a failure block again, whatever its
  size and however much of it is staged: which of the staged lines the working tree's
  failure has too is asked of the two verdicts of the stop, line by line. A working tree
  is known by what it is, not by its path: a linked tree that is moved (`git worktree
  move`, or `mv` and `git worktree repair`) keeps what has blocked in it; a tree made anew
  at the path of a removed one is another tree, and its failure blocks, also when it is
  word for word the old one; and what was remembered of a tree that Git no longer lists
  is forgotten. (A linked tree is told by its administrative directory under the
  repository: the directory's inode and the time it was made, its birth time, which no
  `touch` and no tool that sets times can change. So setting the times of Git's files,
  `git gc` and `git worktree repair` leave a tree what it is. Where the file system keeps
  no birth time, the file `commondir` in that directory stands in, with its inode and
  modification time: there, a `touch` of that file or a restore that does not put its
  time back makes the tree a new one, and a failure in it blocks once more. And in the
  other direction, which is the one that matters: where there is no birth time, times
  are whole seconds and inode numbers are given out again, a tree that is removed and
  made again within one second can get the old tree's inode and the same second, and is
  then taken for the old tree: a failing line in it that the old tree's failure also had
  (the same rule on the same path as many times) does not block. btrfs and tmpfs keep a
  birth time (measured on Linux 7.2, with ext4 and ntfs3 there too). ext2, ext3 and ext4
  made with 128-byte inodes give none, have whole-second times and reuse inode numbers;
  many network and FUSE file systems give none either (these from knowledge, not
  measured: no such file system was at hand). Which trees a repository has is read from those
  directories, not from paths: a tree whose paths Git records as relative ones
  (`git worktree add --relative-paths`, `worktree.useRelativePaths`) is the same tree
  from inside and from outside. Nothing is written to mark a tree. A copy of the whole
  project put at its path is another repository, not the project: stops there are
  UNJUDGED and say so.) A failure of more than 256 lines is remembered as one, a digest of
  all of them: it stands while it is exactly what it was, and any change in it, the
  repair of one part too, blocks again as a whole, with a sentence that says the block is
  not all new. (A failure that was remembered by its lines and grows past 256 is still
  compared line by line at that stop: the block says which lines are new and that the
  others stand.) **The session remembers 2,048 failing lines** (under 140 KiB of a state
  that is read up to 4 MiB). When the changes of its working trees have more, the largest
  ones that the stop is not looking at are remembered the same way, each by one digest:
  an unchanged failure still stands, and a changed one blocks. **What is
  staged stands for as long as the index is untouched and the state can be written,
  whatever the working tree does, unless a stop has said that its memory of it could not
  be kept.** With up to 256 failing lines that is remembered line by line. Past that, and
  when the session has more lines than it keeps, what has blocked of a staged change is
  remembered by a digest, and a digest answers for the index in one case: it is exactly
  what the index holds. Then every staged line has blocked, and no part of it is new,
  whichever part the working tree's failure has too at a stop (a staged failure of 300
  files that is in the tree again in part, and repaired there again in part, stands
  throughout). Where a digest cannot answer, the stop at which that begins says so, in
  one of two sentences:
  "What has blocked of what is staged in 1 working tree is remembered as a whole from
  now on … after a change of that working tree it may block once more, although the index
  has not changed" (the staged change's own failure has more than 256 lines and the index
  holds further failing lines, or the session has more than 2,048 lines);
  "Of what is staged in 1 working tree, the hook remembered what had blocked as a whole …
  and the index no longer holds exactly that …" (a digest had to be given up: part of
  what it stood for left the index, for example by `git reset -- FILE` of one of 300
  staged failing files; one working tree is enough for this one). A change the stop is
  looking at is put into the short form only when
  every other already is, and the stop says so. Only when the working trees have more
  than 2,048 failing changes is the memory of the oldest ones given up, and the stop at
  which that happens says how many: those, and only those, may block once more although
  nothing changed, while the state can be written. (With 256 working
  trees or fewer the last of these sentences cannot be reached: it needs more than 2,048
  digests, and a working tree has at most three. The one about an uncommitted change the
  stop is looking at needs some 250 other trees that each have an uncommitted and a
  staged failure. Those two are held by unit tests, not by a run of the hooks; the two
  about what is staged are held by runs of the hooks too.) **A stop that could not read
  a working tree forgets nothing of it.** When Git cannot give the tree's change (a file
  that may not be read, a path marked skip-worktree, a Git that fails or is slow at that
  stop), the stop says UNJUDGED with the reason; it has not looked at the index either,
  and where something of what is staged has blocked before it says so ("what has blocked
  of it before stands"). "Could not be read" is never taken for "nothing there": a
  failure is forgotten only by a stop that looked and did not find it. A failing commit blocks at
  the stop that first judges it, whatever has blocked before as an uncommitted change: its
  lines are a rule, a path and a number, and another credential in the same file has the
  same. So a change that is stashed and brought back blocks three times, each time for
  something else: as the uncommitted change, as the stash's commit (a commit of the session
  that holds it), and as the change that is back in a working tree a stop had seen without
  it. A commit that was amended or rebased is another commit and is judged again. When the hook
  cannot keep its state (a read-only directory, a full disk), it says so with the decision,
  and nothing is remembered: a failure blocks again at the next turn. It does not block
  twice running: when the client says that it is continuing because a stop hook has just
  blocked (`stop_hook_active`), and only when the state could not be kept, the stop is
  allowed and says FAIL STANDS with the reason.
- **What the judge names beside its rules is passed on.** forge-warden also fails a change
  for a waiver that has expired and holds one for a note, with every rule passed. The message
  then carries the judge's own words for it (bounded, escaped, marked as the judge's text and
  not instructions), and a fail with no failing rule says that it is for a person to settle:
  it is not repaired by editing the change. It still holds the stop once.
- **No line of the change is repeated to the agent.** A message holds labels, line numbers
  and remedy text; a path is quoted, escaped and cut at 120 characters, and the message says
  that paths are data, not instructions.
- **What the agent is told names the accepted repair only.** Advice after a tool call and the
  rules of a stop's message say, for each label, the change that answers it: never a
  substitute that does not count, whether or not a label raises it, so a judged agent is not
  handed a list of what the tool will not catch. The agent is not pointed at `explain
  signal`, which is for people and lists those substitutes. Every command a hook names to
  the agent (`agent doctor --gate`, `agent setup --client claude --hooks`) is this binary by
  its own absolute path, quoted for the shell: a bare `deep-diff-forge` runs whatever `PATH`
  finds when it is typed.
- **UNJUDGED says why, in the judge's words.** For example, on a machine where forge-warden
  has no Git allowlist enrolled: "the judge could not read its inputs or its environment
  (exit 3): io: Git admission: no Git allowlist enrolled".
- **`agent doctor --gate` says whether a stop here would be judged**, before a session
  depends on it. In the repository of the current directory it checks each link and says what
  it found: Git 2.40 or later, the repository, the policy at `HEAD`, the judge, the chain (the
  judge is given a probe, a patch that adds one comment line to the policy, and must return a
  verdict with a ranker this machine has; the link names the verdict schema and the
  forge-warden version the judge's document gives and the schemas this build reads, v2 and
  v3, and a judge that writes a schema this build does not read is this link, not ready,
  named with its version), whether the policy protects its own file (a policy
  that lets the probe pass can be loosened by any change), whether the judge holds the probe
  for claims it cannot verify (`claims_verified`: a note `claims_unverified` in its verdict;
  then stops here hold and never pass, see above), the patch (the one a stop would
  judge is made, and the one of what is staged, so a path marked skip-worktree, an untracked
  file that cannot be read or an index with unmerged paths is found here and not at the
  first stop), whether hooks are switched on (`disableAllHooks` in
  the settings file that wins), whether a Claude Code settings file registers the stop hook
  (a hook of type `command` with the exact command `agent setup --hooks` prints: this binary
  and the judge each named by its own absolute path, and the policy and the rankers that were
  checked; a command that only holds the words, starts with an assignment, names this binary
  through a link or a relative path, or names no judge is not it), and whether that hook's
  timeout leaves a stop its time (50 seconds or more; `agent setup --hooks` prints 60),
  whether the session-start hook and the hook after a tool call are registered too (the
  judged range depends on the first; a matcher that narrows either fails the link), and
  whether the commits the repository names can be listed as the first hook of a session
  lists them (where Git refuses them because there are more working trees or more
  commits named than a hook reads, a session is recorded with no such list, and its
  commits are UNJUDGED at every stop while what is not committed is judged; where they
  cannot be listed for another reason, no session is recorded and every stop is
  UNJUDGED), and
  whether the directory the sessions' state is kept in can be used (there, a directory of
  its own, writable, outside the project's working tree). The
  settings read are the project's `.claude/settings.local.json` and `.claude/settings.json`
  and the user's `~/.claude/settings.json`; settings an administrator manages are not read.
  A link that cannot be checked because an earlier one is missing is reported as not ready,
  never as fine. Exit 0 only when every link is ready, otherwise 7 (the contract asked for is
  not met). In `--json` the checks are named `git`, `repository`, `policy`, `judge`, `chain`,
  `policy_protects_itself`, `claims_verified`, `patch`, `commits`, `state_directory`, `hooks_enabled`,
  `hooks_registered`,
  `hook_timeout`, `start_hook_registered` and `advice_hook_registered`,
  after the checks `agent doctor` always makes. It takes the same `--judge`, `--policy` and
  `--ranker-dir` as the stop hook. The start of a session runs the same checks in the time
  it has and says "stops will be judged" only when every one of them is ready, or when the
  only ones that are not are `policy_protects_itself` and `claims_verified`, which it then
  names as weak.
- **A hook never fails its client, and a failure is never silence.** A hook always exits 0;
  exit 2, which Claude Code reads from a stop hook as a block, is never used, also not for a
  command line the hook cannot read (a mistyped option says that the hook did not run, and
  for a stop that it is UNJUDGED). Nothing is said in three cases only: Git ran and found no
  repository around a session that has no project recorded; the repository had no commit
  when the session began, has none now and holds no file (with files, every stop says that
  they are not judged: there is no commit to measure from); every unit has a pass (nothing is uncommitted
  or staged that does not pass, and every commit of the session has passed: a commit that
  is held, that failed, or that has no verdict is said at every stop) and nothing that is
  known in the project stays outside what was judged. The last clause is the rule for what
  a stop names beside its verdict: a nested repository that was left out, another
  repository the session's directory is in, a submodule with changes, another working tree
  with changes or one that is gone, working trees a stop did not look at, and a start that
  no session-start hook recorded (the project and the base were then recorded by the first
  stop or by an advice hook before it, and what was committed before is not judged). Each
  is said at every stop while it is true: in full the first time, afterwards in one line
  that counts such things and names the first three. None of them blocks; a stop where
  every unit has a pass and one of them is true is allowed and is not silent. A state
  file that is there and cannot be read (a link, too large, another version, damaged) is
  never "no state": the stop is UNJUDGED and nothing is recorded over it. The same holds
  for the list of commits beside it when it is missing or is not the list the state names.
  At most five such things are said in full at one stop, and a session remembers 32 as
  said in full; the others are in the line that counts them, at every stop. Everything else is said: Git
  missing or refusing the repository (another owner), a directory inside `.git`, a base that
  cannot be read, an orphan branch, a patch that cannot be made. A stop then says UNJUDGED
  with the reason, a start says that stops will not be judged, and advice says each reason
  once in a session, not after every tool call.
- **One deadline for each hook.** Everything a stop does (Git, hashing the rankers, every
  run of the judge, for every tree and every commit) shares 45 seconds, inside the 60 its client waits;
  advice has 4 of 5 and a start 8 of 10. A step gets the time that is left, never a limit of
  its own on top. The scratch directory a hook that was killed left behind is removed by the
  next hook of the repository, once its process is gone (where `/proc` says so).

Limits, sorted by the rule above.

Ordinary workflows, which must never end in silence. These are said (a thing that is in the
project and not judged at every stop while it is true, as stated above):

- A session whose start no session-start hook recorded (the hook was installed after the
  session began, or not at all): its base is where `HEAD` was at its first hook, and every
  stop of it says that work committed before was not judged.
- Uncommitted work that was already in the tree when the session began is inside what is
  judged: start sessions on a clean tree. The start says so when the tree is not clean.
- Another repository: when the session's directory is in a repository that is not its
  project at a stop, the stop names it as not judged. A submodule with changes that are not
  committed in it is named the same way (the project's patch shows a submodule only as the
  commit it is at). A nested repository in the project's tree (a scratch `git init`, a
  checkout) is left out of the patch and named. So is a directory there that Git places in
  this repository with a Git directory whose records name another directory (a copy of a
  linked tree made inside the project with `cp -a`): it is no working tree of the project
  by the records, and it is named so. Another working tree of the project that
  lies inside its tree (`.claude/worktrees/…`) is left out too and is no nested
  repository: it is a working tree of the project, judged at a stop made from inside it
  and named under the next rule but one.
- Commits fetched during the session and brought onto a local branch are the session's and
  are judged, each once: a failure in one blocks once and is then left for a person, like
  any other. A pull of many commits takes several stops to judge; each stop says how many
  wait. Commits that were already in the repository when the session began are not judged,
  whatever brings them onto the branch: every message that names the range says so.
- Another working tree of the project that the session's directory is not in and that has
  uncommitted or staged changes: named, as not judged. The other trees are looked at
  last, after everything the stop judges, in six seconds of their own (each `git status`
  may take half of what is left of them, or an even share when that is more), the trees
  where a failure has blocked first. Sixteen are looked at; the ones beyond, and the ones the six seconds did not
  reach, were not looked at: the stop says how many, and nothing that is remembered of
  them is forgotten. A working tree that was removed by hand: named, with
  `git worktree prune`, which forgets a linked tree that is gone; where a failure stands in
  it the note advises no prune but `git worktree repair` where a tree moved with `mv` is
  now (prune would forget what blocked there). Git lists a main Git
  directory made apart under another name than `.git` as the main tree itself: a session
  that began in a linked tree of such a repository has that entry named so at every stop,
  and prune does not change it. Where nothing is remembered of a tree, `git status` alone decides whether it is
  named: a tree whose only change is in a file marked assume-unchanged (or one a clean
  filter or the line-ending settings hide from `git status`), and in which nothing has
  blocked yet, is not named by a stop made from another tree. Where something is
  remembered of a tree and `git status` there shows nothing, the tree is read with the
  judge's own reader within that tree's share of the six seconds (all six when it is the
  only other tree); a tree too large to read in that time (that reader hashes every
  tracked file again: 10,000 files, 314 MB took 0.9 seconds with a warm cache) is named
  "could not be looked at", with its failure standing, at each stop made from elsewhere,
  also after the failure was repaired, and each such stop takes about that much longer. A
  tree in which a failure has blocked is named with that ("a failure stands there") at
  every stop, before the others; for the trees a stop's line
  does not name, and for the ones it only counts, the line gives their number ("among
  them 2 working trees where a failure stands that has blocked").
- A repository with more than 256 working trees: the hooks do not read the history of so
  many (a session's commits are found from the `HEAD` and the reflog of every working
  tree), so the session's commits cannot be told from the ones that were there:
  "The commits of this session: UNJUDGED", with that reason, at every stop, whatever is
  committed. What is not committed in the tree the stop is in, and in the project's tree,
  is judged as always, and a failure there blocks. That is so for a session that began
  while there were 256 or fewer, and for one that begins in such a repository: its start
  records the project and the commit it began from, and that there is no list of the
  commits that were there (the list is not taken later: a commit the session made
  before then would count as one that was there). `git worktree prune` forgets the
  entries of trees that are gone; it does nothing for trees that exist. The reason a stop
  gives says first what follows ("not in this session, whatever is committed (what is
  not committed is judged)"), then the cause; a reason a message cuts for room ends with
  "…", on the line that counts the commits with no verdict too (150 characters each).
- A bare repository with its working trees inside it (`repo.git/main`, `repo.git/feature`)
  is a layout like any other: the tree a stop is in is the one Git says its directory is
  in, wherever the Git directory lies. A directory Git cannot place is never taken for one
  of Git's own files, wherever it lies: nothing on disk tells a linked tree whose `.git`
  file and administrative directory were both removed from a plain directory, and
  `git worktree add` makes a tree under any name in the Git directory, also under the
  names Git uses itself (`repo.git/hooks/feature`, `repo.git/logs/feat`, `repo.git/svn`).
  Where Git fails or does not answer for the directory the session is in, the stop is
  UNJUDGED with Git's words, which come first in its reason so that a cut for room keeps
  them. So is a stop made from inside a Git directory (`.git`, `.git/hooks`,
  `.git/NOTES_MERGE_WORKTREE` while `git notes merge` waits there, what other programs
  keep in it such as `.git/lfs`, a bare repository's own directory): its reason says that
  the session's directory is inside the Git directory and that Git cannot say which
  working tree it is in, and nothing there is judged as a tree of its own (the project is
  judged as at every stop). A stop made from a submodule's Git directory
  (`.git/modules/<name>`, or a directory under it) is UNJUDGED as well, for another reason
  that it says: there Git names the submodule's working tree, which does not contain the
  session's directory ("Git names as the working tree a directory that does not contain
  the session's directory", with "inside the Git directory" after the paths). At such a
  stop the tree the directory is under is named as one the session's directory may be in,
  also when the directory cannot be resolved, under a directory that cannot be searched: a
  relative directory is taken from the hook's own directory, and a `..` after a link steps
  up from where the link leads; where that cannot be done, every tree is named so. And the
  stop forgets nothing because a tree is no longer listed (a tree the judge's own reading
  finds clean is forgotten, as at every stop). No stop is silent because of where its
  directory lies.
- Where the session's directory is, is found one way, first, for every use of it: a
  relative directory is taken from the hook's own directory, and then, like an absolute
  one, the nearest directory of its path that is there is the one asked about (a part
  that was removed, or that names a file, is passed over). When the hook's own directory
  cannot be had, or a part of the path cannot be looked at, the stop is UNJUDGED and the
  trees the directory may be in are named so.
- What Git answers is believed only as a pair that names one tree. The top level Git names
  and the Git directory it names must be one tree of the repository's own records: a
  linked tree's Git directory pairs with the directory its `gitdir` file names, the main
  one with the main tree's top as `git worktree list` gives it. Where they are not (a
  `core.worktree` in a linked tree's own configuration, pointing at the project, at an
  outer tree the tree lies in, or at a subdirectory of the tree; a copy of a linked tree
  made with `cp -a`), the stop is UNJUDGED with words that say so ("Git names … as the
  working tree of a Git directory whose records name … as that tree", the Git directory's
  path after it), nothing of
  either tree is forgotten, and neither directory is judged as that tree. A pair must also
  contain the session's directory. What is read of every other tree counts as that tree's
  only when Git names that tree there, at every stop: one `git rev-parse` more for each
  tree looked at. Where a repository records no top level of its main tree (its Git
  directory made apart with `--separate-git-dir` under another name than `.git`), the main
  tree's top is the project's when the session began in the main tree; a session that began
  in a linked tree of such a repository is UNJUDGED at a stop made from the main tree. A
  main Git directory named `.git` that does not lie at the tree's top (made with
  `--separate-git-dir=…/.git`, or `.git` a link to such a directory) pairs with its own
  parent directory: every stop made from the main tree of such a repository is UNJUDGED,
  with words that say so. A linked tree moved with a plain
  `mv` and not repaired is such a pair too (its `gitdir` names where it was): a stop from it
  is UNJUDGED until `git worktree repair` in it records where it is, and its reason names
  that remedy where the directory it was moved from is gone (only there: run in a copy, the
  repair makes the copy the tree). The remedy comes after the names of both trees, and the
  reason is cut at a fixed length, so with long names it can be cut off; where a failure
  stands in the tree, the note on the tree that is gone gives the remedy whole.
- A tree does not move while the directory it is remembered at stands. Where Git and the
  records pair a linked tree something has blocked in with another top level than the one
  the session's state records for it, and the tree still stands at the recorded directory
  (a directory whose `.git` file names that tree's own administrative directory, or whose
  `.git` is a file that is not a `gitdir: ` line naming a path, such as an empty one, or one
  that cannot be looked at), neither directory is judged as that tree: a stop from it is UNJUDGED, a
  stop from elsewhere names it, both paths are said, and nothing of it is forgotten, until
  `git worktree repair` is run in the directory that stands or that directory is removed.
  This is what a copy of a tree that `git worktree repair` was run in (`cp -a`, then the
  repair in the copy), or Git's own files edited by hand, give. Where the recorded
  directory is gone, or is no longer that tree (a new tree, a clone or a build directory
  made at the old path of a tree moved with `git worktree move`), the tree is read at its
  new top: a tree moved and repaired, or moved with `git worktree move`, keeps what has
  blocked in it and does not block again, and a new failure in it blocks.
- A listed tree is left out as the one a stop judges only by which tree it is, never by
  its path: two entries `git worktree list` gives with one path (an administrative `gitdir`
  record edited to name another tree's `.git`) are two trees, and each is looked at or
  named.
- The hooks keep the caller's `GIT_CEILING_DIRECTORIES`, without each entry that lies at
  or under one of the project's working trees, an entry compared with its `..` and its
  links resolved (`repo/../hidden` is `hidden`, beside the project): a ceiling inside a
  tree does not hide it, and a repository the user hides with a ceiling above the project
  (a home directory's) stays hidden. The project itself is found with the caller's ceiling
  as it is, so a session that begins below a ceiling that hides its repository has no
  project there.
- `GIT_DISCOVERY_ACROSS_FILESYSTEM` is as the session has it. Unset, Git stops at a file
  system boundary (a mount, or a btrfs subvolume, which one unprivileged command makes):
  where a linked tree of the project lies across the boundary Git stopped at, the stop is
  UNJUDGED with Git's words and that tree is one the session's directory may be in. A
  boundary no linked tree lies across is Git's "no repository", as before (on many systems
  every directory under `/var` is below one).
- A linked tree a failure is remembered of stays remembered while its directory stands,
  also when the repository no longer lists it (its `.git` file and administrative
  directory removed, or its `.git` file removed and `git worktree prune`): every stop
  names it ("… is no longer a working tree of this repository; what blocked there stands
  until the directory is gone or it is a tree again"), and a stop made from inside it is
  UNJUDGED. It is forgotten when its directory is gone (the ordinary
  `git worktree remove`), or when it is a tree again and the judge's own reading finds
  that tree clean. For this the session's state (schema `deep-diff-forge.hook-state.v10`)
  records the top level of each linked tree it remembers a failure of; a state of the
  schema before (`v9`) is read with no tree recorded and written back in the new one by
  the first hook that reads it and keeps its state (a stop, or a session's start), changed
  or not. A path that is not UTF-8 or is longer than 16 KiB, and the paths past 256 KiB in
  all, are not recorded: such a tree is forgotten when it is no longer listed, as before.
- The order of a stop, and its time. One limit: 45 seconds for everything, of which 4 are
  kept for remembering and saying, and 6 more for looking at the submodules and the other
  working trees when the project has any. So the judging has 41 seconds, or 35.

  | in this order | the time one run of the judge may take |
  | --- | --- |
  | the working tree the stop is in: its uncommitted change, then its staged change when that is another | what is left of the judging, less what the asks about commits can need (3 seconds when no commit of the session waits for a verdict; 5 more when one waits for its turn, 5 more when one with fewer attempts waits: 13 at most; a commit waits by the last line the session's file of judged commits has for it), less 5 for its own staged change when there is one, less 3 for the project's tree when the stop is in a linked tree and the project's tree has something that is not committed (one `git status` of at most 2 seconds says; what it cannot say counts as a change), and 30 seconds at most. From a linked tree with nothing else to judge: 30 seconds (32 are left, and no run has more than 30). When that leaves under 3 seconds: all that is left. |
  | the project's tree, when the stop is in a linked one | the same, of what is left then |
  | the asks a stop owes, three at most: the commit whose turn it is, the one with fewer attempts asked about longest ago, the oldest commit never asked about | what is left, less 5 seconds for each owed ask after it; when that is under 3 seconds, 3 seconds. Not made with under 3 seconds left. |
  | the other commits | what is left; not made with under 3 seconds left |
  | the submodules and up to sixteen other working trees, looked at and not judged | 6 seconds together; one `git status` may take half of what is left of them |

  No run of the judge has more than 30 seconds, and no stop takes longer than the 45. A
  run that was stopped says what time it had. A tree is read (Git) when its turn comes,
  not before: the project's tree is read after the tree the stop is in has its verdicts.
- A shallow clone: a commit whose parent the clone does not have is UNJUDGED by its id,
  at every stop, until the clone has the parent (`git fetch --unshallow`); then it is
  judged.
- A session that began in a repository with no commit, or on an orphan branch, has no base:
  its stops are UNJUDGED (after the first commit, in the first case; before it, every stop
  says that the repository's files are not judged).
- More commits than one stop has time for: the rest are counted as UNJUDGED and judged at
  the next stops.

Deliberate evasion, made visible where that is cheap:

- A clone put at the project's path, a project that was moved, a state file or a list of
  commits that was damaged or replaced, a state directory inside the working tree:
  UNJUDGED, with the reason. A removed state file: the
  next stop says that the session's start was not recorded.
- An agent can make a stop UNJUDGED by breaking what the judge needs (a file that cannot be
  read, a path marked skip-worktree). That is never a pass and always visible to the user,
  and it is never a block either.
- Git's own administrative files edited by hand (`gitdir`, `commondir`, `core.worktree`,
  a tree's `.git` file) so that one tree is read as another: where a directory the session remembers a failure
  at still stands, the stop is UNJUDGED and both directories are named; otherwise it is a
  stated limit, as an edited state file is.

Stated limits, which the judge on the pull request covers:

- A commit that only a remote-tracking ref names (fetched and never pulled; or made with
  plumbing and pushed without ever being on `HEAD` or under a local ref) is not judged.
- A commit that is gone from every ref and from the reflogs (the reflog expired or was
  cleared with `git reflog expire`, a branch that was deleted with its commits never on
  `HEAD`) is not judged. The reflogs read are those of each working tree's `HEAD` and of
  the stash; a working tree whose directory is gone is not asked.
- Work in a copy of the repository, and anything in a repository that is not the session's
  project: named where the hook sees it, not judged.
- `git worktree repair` run in a copy of a linked tree before anything blocked in the tree
  makes the copy the tree: the directory copied is then no tree of the project, and nothing
  in it is judged (a stop from inside it is UNJUDGED). A tree moved with `mv` whose entry Git
  then ends (`git worktree prune`, or `git worktree add -f` at its old path) is forgotten
  with what blocked there; a stop from the moved directory is UNJUDGED.
- Two stops of one session do not run at the same time: every stop, the session's first
  too, holds a lock beside the session's state while it runs (`flock` on
  `session-<hash>.lock`), and a second one waits three seconds and then says UNJUDGED,
  "another hook of this session is running" (it may be the session's start that holds the
  lock). A session-start hook holds the lock while it
  reads and records; when it cannot have it in two seconds it records nothing and says
  so. An advice hook works without the lock and takes it only to write, onto the state as
  it is then, and only what it owns (what it has said): what a stop wrote in the meantime
  stays. When it cannot have the lock in a third of a second it writes nothing and says
  nothing; the advice is new again after the next tool call. The lock ends with the
  process that holds it, so a hook that is killed holds nothing, and a stop that is cut off
  in the middle (an outage, a kill) has written nothing and makes nothing final: only a
  verdict is written down as one. On a file system with no locks, and where no state
  directory can be made, the hooks run as they did without one: two that overlap can each
  write what the other did not see, and a commit is then judged again or a failure blocks
  again; nothing becomes a pass by it.
- An entry made with `git add -N` is no staged content; it is in the uncommitted change.
- A session with only commits the judge cannot judge, and nothing else that gets a
  verdict, asks about up to three of them at a stop and then about the judge's probe: the
  probe gets its verdict, so their attempts count, and after five each is asked about in
  its turn, one at a stop. Under a policy that has no probe (a policy file with no line)
  nothing shows that the chain works: no attempt counts, and each stop asks about up to
  three of them, the ones asked about least recently, so each still comes round.
- While the state cannot be written, nothing of a stop is remembered: every stop starts
  from what was last written, asks about the same commits first, and says that it could
  not keep its state.
- The settings that install the hooks, the state file, the list of commits beside it and
  the judge on `PATH` are the agent's to edit: it shares the user's account. Whoever edits
  the state or that list can move the base, or make a commit count as there before.
- A commit that amends or rebases work from before the session is the session's commit and
  is judged whole, with what it took over.
- Advice after a tool call covers the project's working tree against the session's base
  only: not a linked working tree the session is in, and not commits that are no longer in
  the tree. The stop has both judged.
- "No repository here" is read from Git's own English message, with `LC_ALL=C` set for that
  call. A Git that words it otherwise is read as a failure, which is said, not as silence.
- The scratch directory of a hook that was killed is removed only where `/proc` says its
  process is gone (Linux).
- **A resumed conversation is a new session of the hooks** when its client gives it a new
  session id: its base is where `HEAD` is at its first hook, so what the earlier
  conversation committed is base, and a conversation that ended without a stop (a crash, a
  usage limit, a restart) leaves its commits unjudged by any stop. Not a boundary: the
  commits are in the branch, and the judge on the pull request and a person reviewing it see
  them. Target: F13 (0.11.0), the judged frontier of the project and the working tree.
- **Linked working trees share branches.** A commit another session makes in another
  working tree of the project is on a branch this session's stops read, made during this
  session, so it is judged as this session's: its failure can hold this stop (measured: a
  session in the main tree that changed nothing was held for a failing commit made in a
  linked tree by another session). Target: F13 (0.11.0).
- **Some stop outcomes cost the judged agent nothing at its next turn.** A path the hook
  cannot read makes the stop UNJUDGED, a failure that stands after its one block is said as
  FAIL STANDS, a hold is for a person and never blocks, a nested repository is left out and
  named, and advice that is off is said once. Each is said, to the user every time, and none
  is silent; none holds the agent either. Not a boundary: the user sees each, and the judge
  on the pull request judges what was committed. Target: F12 (0.11.0), which closes the
  self-caused ones by rule.
- **A process the judge starts that leaves its process group** (`setsid`, or a double fork
  into a group of its own) is not killed by the hook and runs on after the stop. Claude Code
  2.1.330 ends a timed-out hook's descendants, those in their own group or session too
  (measured 2026-11-22); one that detaches from them is ended by no one. The judge is the
  program the person who installed the hooks named. A judge that exits while a process it
  detached from its group still holds its output is not read: the stop is UNJUDGED when the
  limit passes, whatever the judge's exit code was; forge-warden starts no such process.
- **Under a policy that requires verified claims every stop holds** (see the verdict
  section): the hook gives no `--repo`, `--base` or `--head`, and an uncommitted or staged
  change can never be verified. Target: S3.25.b (0.11.0) for commits; never for what is not
  committed.
- **Six ways the judged agent can make a stop say nothing it should, or judge nothing**,
  each a deliberate act, measured on this release (0.10.0) with a stand-in judge. None is a
  boundary: a person reading the working tree, and the judge on the pull request for what is
  committed, still see the work. Target: F12 and F13 (0.11.0), through the stop model F7.j.
  - A directory with mode 000 hides from Git what is in it: an untracked directory holding
    failing work, and a tracked file's failing edit once its directory is made unreadable.
    Git lists nothing there, so the advice and the stop are silent, with no run of the judge;
    with the directory readable again, the same stop blocks.
  - An untracked `.gitignore` that ignores itself (`*`), at the top or in a subdirectory,
    hides itself and the failing work beside it: the stop is silent, with no run of the
    judge.
  - A directory named `.git` below the top hides from Git everything in it (Git lists and
    stages no path with a `.git` component; measured with Git 2.56.1): files an agent puts
    there are in the working tree, are never part of a patch, and cannot be committed. The
    stop does not see them.
  - A plain directory at a path the base or the index records as a submodule (a gitlink)
    is listed by no Git command when no repository is there: failing work only in it gives a
    silent pass (measured on this release; 0.11.0 walks such a path like any other).
  - A very large sparse file (`truncate -s 400G`, no disk used) makes Git's staging run out
    of the hook's time: the stop is UNJUDGED ("`git add` did not finish in the time a hook
    has and was stopped", 41 seconds), with no run of the judge, at every stop while the file
    is there, also with further failing work added. It is said to the user and never blocks.
  - A path the hook cannot read makes every stop UNJUDGED while it stays so, with no run of
    the judge: said to the user, never a block. A failure that has blocked once in a path
    stands after the path is rewritten: when it is readable again and fails the same rule,
    the stop is judged and says FAIL STANDS; it does not block again.
- The state of a session is up to three files in the user's state directory
  (`deep-diff-forge/hooks` under `$XDG_STATE_HOME`, else under `~/.local/state`), found by
  the session's id: the state, the list of commits that were there when it began, and the
  list of commits that have their answer. Files of sessions that have
  ended are not removed. Hooks for Codex are not designed.

---

See also: [`API_AND_IPC`](../API_AND_IPC.md) · [`TESTING_GOLD_STANDARD`](../TESTING_GOLD_STANDARD.md).
