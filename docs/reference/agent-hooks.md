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
deep-diff-forge agent setup --client claude --hooks --ranker-dir DIR   # the same, with older rankers offered (0.11.0; also --judge PATH, --policy NAME)
```

| Event | Command | What the agent gets |
| --- | --- | --- |
| `SessionStart` | `agent hook session-start` | What the hooks do, the commit the session's changes are measured from, and whether a stop can be judged here: the chain is checked the way `agent doctor --gate` checks it, and a link that is missing is named instead of a promise. |
| `PostToolUse`, after every tool | `agent hook post-tool` | **Advice, never a block.** What is new in the working tree since the session began (commits pulled or made by others in that time are in that range too), as items of the [diagnostics document](diagnostics.md) of that change whose audience is the agent and whose class is `security` or `integrity` (from 0.11.0): a `shape` diagnostic (a new or deleted file, a lockfile, a public API path, a large change) orders a person's review, so it is neither listed nor counted, and a change that raises only such labels gets no advice; advice that said there was nothing to repair after every tool call that added a file taught an agent to skip the advice. Each item is `"path":line: code (class, side, read as …); case: …; repairs: id, id.` for a located diagnostic, `"path": code (class, read as …); …` for one at its file, then each repair id once with its sentence. The items, their cases and repair ids are those `--diagnostics --json` gives for the same tree in those two classes; "read as" is the rung of the ranking that found a row (`lexical` for one line, `hunk` for tokens read across the lines of a hunk), which the document does not carry. Where the engine cannot tell a diagnostic's case the item says so and lists every repair. A reading kept for compatibility with 0.8.3 (`floor_only`, for a person) is not advised. Each file's diagnostics at the file come before its located ones; a file whose located diagnostics were cut at 32 is said to be. At most 20 items and about 5,600 bytes are listed; the items that do not fit (the last in the document's order) are counted and said after the next tool call. A finding is said once per session, also when the line moves. A file whose change this version cannot read (a name that is not UTF-8) is one item naming it as Git quoted it, and the other files are advised; a change not ranked in the hook's time is said once to be "advice cut short". When advice cannot be given (a sparse checkout, Git refusing the repository), the user and the model are told once that it is off and why. An edit of a guard file (the policy file `--policy` names, `warden.toml` by default; a `.gitattributes`; a CI workflow under `.github/workflows/`, `.forgejo/workflows/` or `.gitea/workflows/`, `.gitlab-ci.yml`, `.circleci/config.yml`; lint and tool configuration such as `clippy.toml`, `rustfmt.toml`, `deny.toml`, `.cargo/config.toml`; the client's `.claude/settings.json` and `settings.local.json`), matched on whole path components, is named before the items with what the file decides, as information: "… is a file that decides how this work is judged or checked: … This is information, not a block: the edit is part of the change a stop judges." It is named once for each content of the file, so a later edit is named again; at most 8 in one advice. `agent setup` gives this hook `--policy` when it gives one to the others. |
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
- **The project's frontier** (0.11.0, F13.b). What has been judged belongs to the project, not
  to one conversation. When a session's first hook finds the project, the project's ledger
  (below) gets a `baseline` record when it has none: every commit the repository names then
  (refs, each working tree's `HEAD`, the reflogs), and the commit `HEAD` names. A stop then
  judges, besides the session's own commits, every commit the same sources lead to that the
  baseline does not and that no stop record of the ledger gave a verdict (`pass`, `fail`,
  `fail_stands`, `hold`), whichever session made it: a session that ended before its stop (a
  crash, a closed terminal, a conversation that went on under a new record), one whose state
  was removed, one whose stops ran out of time. They are judged before the session's own, the
  oldest first, within the stop's time; what does not fit waits for the next stop with the
  usual words ("they are judged at the next stop"). A commit the session finds in its own
  range is judged by it even when another session has judged it (a commit found by several
  sessions holds each once, below). A project whose ledger has no baseline (a ledger of an
  earlier build, or one that was removed) is judged as before, by the session's own range,
  and the stop says so ("No record of this project was found in its ledger …") and records
  the commits that were there when the session began as the baseline. A `SessionStart` whose
  `source` is `resume`, `compact` or `clear`, for a session with no record of its own, begins
  from the commit the last session of its working tree began from (the ledger's `start`
  records), not from `HEAD`: a policy the earlier conversation loosened in a commit does not
  judge that commit. With `startup` it begins at `HEAD`, as before.
- **Which commits of others are judged.** A commit that was already in the repository when
  the project's ledger first recorded the project (else when the session began), named by any
  ref (remote-tracking refs included) or by a reflog, or that was made before the session
  began and has a stop's verdict in that ledger, is
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
  by `PATH`. From 0.11.0 `agent setup --hooks` also takes `--judge PATH`, `--policy NAME` and
  `--ranker-dir DIR` and writes them into the session-start and stop commands (the judge and
  the directory by their absolute paths, a judge that is no file and a directory that does
  not exist refused with exit 2); give `agent doctor --gate` the same values, since it
  compares them with the registered stop's: the judge by its absolute path, the policy by
  its name, and the ranker directory by the directory a stop lists (its path with every
  link resolved, so a relative path, a trailing `/` or a link to that directory is the
  same; a relative directory written in the stop command is the same only by its text, since
  the stop takes it from its own directory, and so is one that does not exist). Without `--judge` it is looked up in the directories of `PATH` that are named by
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
  deletions, so a stop is UNJUDGED with that reason and advice says so once; from 0.11.0 the
  marks first hold one stop (see "Causes that hold a stop once" below).
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
- **A stop whose state cannot be used still judges the tree** (0.11.0, F12.d; owner decision
  OD-J). When the session's state is damaged, newer or older than this build reads, its
  directory cannot be used, its key file is no key, its lists are not the ones it names, or
  another hook of the session holds its lock, the stop no longer says only that it did not
  run. It reads no file of the state directory and writes none: with an empty memory, it
  finds the project from its input as a new session does, judges the uncommitted and staged
  changes of the working trees (their patches against `HEAD`) under the policy at the commit
  the project's ledger names as its baseline (at `HEAD` when the ledger has none, which the
  message says: a commit of the session can have loosened that one), and decides as a stop
  with no memory: a failing tree blocks. At a stop that follows a block (`stop_hook_active`)
  the failure stands instead (FAIL STANDS), so the session is not trapped; at the next turn's
  stop it blocks again, since nothing remembers that it blocked: a state that stays unusable
  blocks once in each turn while the tree fails. No commit is judged (the session's base is
  unknown): they wait for a stop that has its state, which the frontier makes judge them. The
  message begins "deep-diff-forge: this session's state could not be used (<class>): the
  working tree was judged without it; commits made in this session are judged when the
  project's ledger finds them, at a stop that has its state. Left as it is, the working tree
  is judged like this at every stop with no memory of what blocked: a failing tree blocks once
  in each turn." and then gives the refusal, with what removing the state costs. No byte of
  a key and no text of a damaged state is said.
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
  not met). Stderr then carries `error: the chain is not ready: …` followed by
  `refused: N checks failed (agent doctor)`, also when the reader of stdout has gone. In `--json` the checks are named `git`, `repository`, `policy`, `judge`, `chain`,
  `policy_protects_itself`, `claims_verified`, `patch`, `commits`, `state_directory`, `hooks_enabled`,
  `hooks_registered`,
  `hook_timeout`, `start_hook_registered` and `advice_hook_registered`,
  after the checks `agent doctor` always makes (`mcp_handshake`, `review_parity`, `rank_parity`,
  `diagnostics_parity` and `views`). It takes the same `--judge`, `--policy` and
  `--ranker-dir` as the stop hook. The start of a session runs the same checks in the time
  it has and says "stops will be judged" only when every one of them is ready, or when the
  only ones that are not are `policy_protects_itself` and `claims_verified`, which it then
  names as weak.
- **`agent check` judges, on demand, the units a stop judges, with a stop's judge and
  policy** (0.11.0). `deep-diff-forge agent check [--session ID | --base REV] [--range A..B]
  [--judge PATH] [--policy NAME] [--ranker-dir DIR] [--require-pass] [--json]` judges, in the stop's order
  and each by the patch a stop makes for it: the uncommitted change of the working tree around
  the current directory against the commit its `HEAD` names; the staged change, when it is
  another change than the tree's; then each commit by its own patch, from its first parent:
  with `--range A..B` the commits of that range, which must be one line (each the first parent
  of the next, from `A` to `B`; otherwise nothing is judged and `unjudged` says why), and
  with `--session` and no range the session's commits as a stop finds them (named now by the
  project's branches, tags, stash, `HEAD`s or their reflogs, and by nothing named when the
  session began); with a session a range may hold only those. The judge, the policy and the
  ranker directory are those of the first `Stop` hook in the form `agent setup --hooks`
  prints, in the settings the gate reads (each can be replaced with its option); the judge is
  never looked up on `PATH`. With `--session ID` (the id Claude Code gives the hooks) the
  policy is read at the base that session recorded, and its state file, its list of the
  commits there when it began and its list of answered commits are opened for reading only:
  no lock is taken and nothing in the state directory is made or changed. Without a session
  the policy is read at `--base REV`, else at `HEAD`, and the answer names that base.
  `--json` prints `deep-diff-forge.agent-check.v0`: `project`, `session` (`state_file`,
  `base_recorded_by`: `session_start` or `first_hook`), `base` (`commit`, `from`: `session`,
  `--base` or `HEAD`), `judge` (`program`, `policy`, `ranker_dir`, `stop_hook`: the settings
  file), `units` (each with `unit`: `uncommitted`, `staged` or `commit`, its `commit` id,
  `outcome` `pass`, `hold`, `fail` or `unjudged`, each rule by `id` with `held` and the
  `signal`, `path` and number of `items` of each violation, the number of things the judge
  named beside its rules, `judge_notes`, with their codes, `judge_note_codes`, and the
  `reason` of an UNJUDGED, and `ranker`, `advice_only` and `advice_unread`, below),
  `unjudged` (why nothing could be judged: no repository, no
  judge, a session with no state or no base, a base or a range that is refused) and `stop`.
  Nothing of the judge's own words is repeated: they can quote the patch.
  **A ranker the policy admits.** The rankers are offered as a stop offers them (this binary
  first, then the ranker directory), so when the policy admits another one the unit is judged
  with that one, as at the stop, and its `ranker` names it ("judged with the ranker the policy
  admits, …, not this binary"). The check then runs that ranker on the unit's patch (`--stdin-patch
  --rank --json`, under the judge's bounds: the time left, at most 30 seconds, and its output
  caps) and lists in `advice_only` each label this binary raises that it does not ("advice
  only, not judged by the pinned ranker: LABEL in PATH"). A ranker that runs out of time,
  exits other than 0 or prints no rank document leaves `advice_only` null and says why in
  `advice_unread`: what is advice only is then not known, which is never an empty list.
  **What it says about a stop.** `stop.same_inputs` is `true` only with a session whose base
  was read, a registered `Stop` hook that runs this binary by its own path and names a judge,
  and no option that replaces its judge, policy or ranker directory. `stop.whole_answer` is
  `true` only when, besides, every unit a stop of the session answers for was judged: the
  session's commits could be told (its list of the commits there when it began was read), no
  session commit was left out by `--range`, and the project has no linked working tree (a
  stop from inside one also judges that tree). Only then does the answer end with "Equal to
  the stop in its units and inputs: every unit a stop of this session answers for (the
  uncommitted change, the staged change where it is another, and each commit of the session
  by its own patch) was judged here with the stop's patch, the policy at the base the session
  recorded, and the judge and rankers of the registered Stop hook. What the stop then decides
  can still differ: the lines "The stop may differ" say where." Otherwise it ends with "This
  is not the stop's whole answer: not every unit a stop of this session answers for was judged
  here with the stop's inputs (the lines "Not as the stop" say what differs).", each
  difference is in `stop.differences` and said as "Not as the stop: …", and with the same
  inputs the text form says "Judged with what a stop of this session judges these units with:
  …". Equal units and inputs are not an equal decision, and `stop.may_differ` lists, where
  they apply: a failure blocks a stop once (what has blocked stands and does not block again;
  the session's memory of it is not read); a commit an earlier stop answered is reported from
  that answer and not asked again; and a stop's time (45 seconds in all, 30 for one run of
  the judge, time kept back for what it asks after, its own order). Neither a stop nor a check
  sees what Git does not list (the six ways in the limits below), and a judge whose verdict
  changes between two runs on the same inputs can answer otherwise at the stop.
  **What a check writes.** With no judge it stages nothing and writes nothing (the answer is
  UNJUDGED with the reason). With a judge it writes one scratch directory,
  `.git/deep-diff-forge/tmp-r<16 hex digits>` under the repository's Git common directory,
  holding the scratch indexes and objects of what it stages, the lists of commits Git is
  given, the policy and the patches it gives the judge, and removes it before it exits; it
  makes `.git/deep-diff-forge/` when that is missing and removes it again when it made it and
  it is empty. Making its directory, it removes what killed hooks left there, as every hook
  does. It changes neither the index nor the working tree (`git status` and the index file are
  as they were), nor anything in the state directory. A check with `--session` that ran a judge
  also appends one record to the project's ledger (below), in one bounded append: the
  session's key, how many units it judged and how many of them did not pass, and nothing else
  (no line, no path, none of the judge's words); `--json` says in `ledger` whether it was
  written, or why not. A check without a session writes nothing to the ledger, and a check
  with one leaves the session's state, its list of the commits there when it began and its
  list of answered commits byte for byte as they were: the ledger is the one file a check of a
  session grows. The count a stop records from these is a lower bound: a check without
  `--session`, a check in a clone, and a run of the ranker or the judge by hand are not
  counted. Exit 0 with the document, 2 for a usage
  error. With `--require-pass` it exits 7 after the document when there is no unit (nothing
  to judge, or nothing could be judged: `refused: there is no unit to judge (--require-pass
  no_unit)`) or when any unit is not a pass (`refused: N of M units did not pass
  (--require-pass not_pass)`): a clean CI checkout is no pass. Without a session and without
  `--range` a check judges no commits, so a CI job names its range (`--range origin/main..HEAD`
  or the like) to cover them. The refusal is decided before
  the document is written, so a reader that has gone, or a full disk, still gets exit 7 and the
  reason.
- **The project's ledger records what each stop decided** (0.11.0). Every stop that has a
  project appends one record for each unit it had something to judge to
  `deep-diff-forge/ledger.jsonl` under the repository's Git common directory (`.git/` of the
  main tree; a linked tree's stops write to the same file), one JSON object a line: `v` (1),
  `kind` (`stop`), `session` (16 hex digits of the SHA-256 of the session's id, which its state
  file's name begins with), `worktree` (`m` for the main tree, else the linked tree's 16 hex
  digits; null for a commit), `unit` (`commit:<id>`; `tree:<digest>:uncommitted` or
  `tree:<digest>:staged`, the digest a SHA-256 of the patch's SHA-256 under a salt of 32 random
  bytes kept in `ledger.salt` beside it, or `unread` when the change could not be read; or
  `stop` for what names no single commit or tree: a stop that judged nothing, or the session's
  commits when they could not be listed), `outcome` (`pass`, `fail`, `fail_stands`, `hold` or
  `unjudged`), `class` (`pass`, `fail.new`, `fail.stands`, `fail.unsaved_continuing` (a block
  the hook let stand because the client continues after a block and the state cannot be
  kept), `hold.rule`, `hold.version_gap`, `work.unreadable`, `work.skip_worktree` (the
  causes that hold a stop once, below); null where this build
  does not class the reason yet: every other `unjudged` record and any other hold for the
  judge's notes alone), `blocked_turns` (1 for a unit that blocked
  the stop), `agent_checks` (`runs` and `failed`: the records of `agent check --session` of
  this session since its last stop record), `ignored_elsewhere` (null: not counted yet, which
  is never 0) and `seals` (the SHA-256 of the policy and of the ranker the verdict was sealed
  to; for a commit also of its patch and of the verdict document; null when the judge did not
  run). A commit that passed is recorded at the stop that judged it; a commit left waiting for
  the stop's time is recorded as `unjudged` at each stop that leaves it. A stop with nothing
  to judge records nothing. Nothing of a patch's lines, a path, Git's words or the judge's
  words is written. The file is made with mode 0600, never through a link, and only appended
  to (`O_APPEND`, one write a stop, under its `flock`); a stop waits at most 0.2 seconds for that
  lock, and reads at most the last MiB of the file, so a stop's time does not grow with the
  ledger. Nothing is synced to disk: the record is the stop's report, and waiting for a disk
  inside the stop's time would cost the judging. **Writing it never changes a decision**: a
  ledger that cannot be written (a link or anything but a regular file at its name, a
  directory that cannot be written, a lock another process holds, a file that holds a record
  of a newer build) is said beside the decision ("The hook could not write the project's
  ledger … what this stop decided stands, and is not recorded there.", in a block's reason
  like the note that the state could not be kept) and the stop decides as it would have. A
  ledger with a record of a newer build (`v` above 1) is kept as it is: nothing is written to
  it. Two more kinds of record (F13.b, still `v` 1: the ledger has not been released): one
  `{"v":1,"kind":"baseline","head":…,"tips_count":N,"tips_sha256":…,"tips":[…]}` when a
  session first finds a project whose ledger has none (the commits named then, at most
  100,000: a project that names more has no baseline and is judged as before), and one
  `{"v":1,"kind":"start","session":…,"worktree":…,"base":…}` for each session when its first
  hook finds the project (an advice hook or a session's start included). A stop reads the
  whole ledger (at most 64 MiB) for the frontier; a line that is not a record of this build
  is passed over there, which can only judge more. A stop whose state cannot be used records
  the units it judged and one `stop` record `unjudged` with the state's class for the commits
  it did not judge. **The ledger is not a boundary**: it is a file in
  the repository's Git directory with the permissions of whoever works there, the agent
  included, so a record is evidence that a stop decided so, not proof, and a missing or edited
  ledger proves nothing.
- **`agent verify-branch BASE..HEAD [--json]` verifies a branch against the ledger** (0.11.0).
  It lists the commits of the range as a stop lists a session's commits (every commit `HEAD`
  leads to that `BASE` does not, the oldest first) and reads the ledger, and only the ledger:
  no judge runs. A commit is verified when a stop record passed it and none failed or held it
  (a commit a stop left unjudged and a later stop passed is verified), or when a clearance
  record names it: a line `{"v":1,"kind":"clearance","unit":"commit:<id>"}` that a person
  appends, which is a convention and not a verdict. Each commit is said with its `outcome`
  (`pass`, `fail`, `fail_stands`, `hold`, `unjudged` or `no_record`), `cleared`, `verified`, how
  many stop records name it, and `agent_checks` of the first stop that gave it a verdict (the
  checks with `--session` before it: a lower bound). `--json` prints
  `deep-diff-forge.agent-verify-branch.v0` (`range`, `ledger`, `commits`, `verified`,
  `unverified`: why the range or the ledger could not be read, and `cannot_know`). Exit 0 when
  every commit is verified (an empty range has none to refuse); exit 7 after the document when
  one is not, `refused: N of M commits are not verified by the ledger (verify-branch)`, or when
  the range or the ledger cannot be read, `refused: the range or the ledger could not be read
  (verify-branch)` (class `branch_unverified`); a reader that has gone still gets exit 7. A
  ledger with a line that is not a record of this build, a record of a newer build, a last
  line that is not whole, more than 64 MiB, or anything but a regular file at its name is
  refused whole: a damaged record never reads as a missing one. Nothing is written to the
  ledger; the list of commits Git is given is written in a scratch directory under
  `.git/deep-diff-forge/` and removed. **What it cannot know**, said in the document and the
  text: a commit made outside a session with the hooks, or in a clone, has no record; the ledger
  is the agent's to write and delete like the rest of `.git`, so its absence proves nothing and
  a record is evidence that a stop decided so, not proof that the commit passes. Where the
  agent can write, it is advice; the boundary is the pull request's judge.
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
  next hook of the repository: a hook holds a lock (`flock`) on its scratch directory while it
  works, and a directory whose lock can be taken has no owner. (Until 0.11.0 the owner was
  told by a process id read in `/proc`, and a check run in another PID namespace removed a
  live stop's scratch.)

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
  checkout) is left out of the patch and named. It never holds a stop: the hook cannot tell
  whether the session made it (see "Causes that hold a stop once" below). So is a directory there that Git places in
  this repository with a Git directory whose records name another directory (a copy of a
  linked tree made inside the project with `cp -a`): it is no working tree of the project
  by the records, and it is named so. Another working tree of the project that
  lies inside its tree (`.claude/worktrees/…`) is left out too and is no nested
  repository: it is a working tree of the project, judged at a stop made from inside it
  and named under the next rule but one.
- Commits fetched during the session and brought onto a local branch are the session's and
  are judged, each once: a failure in one blocks once and is then left for a person, like
  any other. A pull of many commits takes several stops to judge; each stop says how many
  wait. Commits that were already in the repository when the project's ledger first recorded
  the project (else when the session began), and commits from before the session with a
  stop's verdict there, are not judged, whatever brings them onto the branch: every message
  that names the range says so.
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
  that tree clean. For this the session's state (schema `deep-diff-forge.hook-state.v13`)
  records the top level of each linked tree it remembers a failure of (a state of the
  schema before, `v9`, which no release wrote, was read with no tree recorded until
  0.10.0; since 0.11.0 it is refused as older than the last release: "The state's schema",
  at the end). A path that is not UTF-8 or is longer than 16 KiB, and the paths past 256 KiB in
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

- **Causes that hold a stop once** (0.11.0, task F12, owner decision OD-J). A stop that the
  judged agent can make unjudgeable at no cost, and a hold the agent cannot answer, each hold
  the stop once instead of being let through; after that the stop is UNJUDGED, or HOLD, as in
  0.10.0, and the ledger records each stop with its class. The hook has no history of a
  path, so it decides by evidence it has: the cause lies in the working tree as it is at the
  stop, and holding it once whoever made it is what a failure in work the session did not
  make already costs; the block's reason says "If this session made it so, undo it; if not,
  tell a person." The causes and the evidence:

  | Class | Evidence the hook uses | The block names |
  |---|---|---|
  | `work.unreadable` | staging the working tree failed, and `File::open` (no final link followed, never waiting) refuses a path that the index or the untracked listing names and `symlink_metadata` says is a regular file; looked for only after the failure, at most 1,000,000 paths and within the stop's Git time | the files, escaped (the first 20; the rest counted; a name that is not UTF-8 is counted, never printed) |
  | `work.skip_worktree` | the index marks a path skip-worktree (`git ls-files -t`) | that the index has such marks (the paths are not named) |
  | `hold.version_gap` | the judge held with no rule held and every note `unknown_signal` | the label, only when it is one of DDF's own (else "a label DDF does not list"), and the note's path, escaped |

  What stays as in 0.10.0 because the hook cannot tell it is the agent's: a staging failure
  where no unreadable file is found (a Git that fails or is out of time, a search cut by its
  bound: the reason then says "not all names are known"), an unreadable directory (Git lists
  nothing in it: the stop is silent, a stated limit below), a nested repository (left out
  and named, as above: whether the session made it cannot be told), a hold for any other note
  (`unknown_status` included), and every cause of the machine, the clock, the judge's chain,
  the state or the place (a judge or policy that is missing, a time-out, a lock another hook
  holds, a damaged state). A stop whose state cannot be used judges the tree without it
  (below) and records what it judged.

  How "once" is kept: each cause is remembered in the session's state by a name (its class
  and a digest of its paths) and a state (the inode, size, modification and change times and
  mode of each of its first 4,096 paths, by `symlink_metadata`). It holds a stop when its
  name is new, and again when its state changed, at most **three** times for one name in a
  session; after the third, each stop says "it has held 3 stops of this session, the most
  one cause holds, so it is let through" and is UNJUDGED, recorded. It never holds a stop
  that follows a block (the client's `stop_hook_active`): a cause first met there, or changed
  there, holds the first stop of the next turn ("the first stop of the next turn holds for
  it"), so one cause holds at most one stop a turn and cannot keep a session from stopping,
  and the stop's time bound is unchanged. A later turn, and a client restarted on the same
  session, read the same state: a cause that held stays held. While the state cannot be
  saved nothing is remembered, so a cause, like a failure, holds the first stop of every turn
  and never the stop that follows it. A hold for an unknown label has no state: it holds once
  per set of labels and paths; a commit held so holds the stop that judges it, once, as a
  commit that fails does. The names and states are kept in `blocked` (as `tag/key:count`:
  under the tag `o` of the tree for a refused cause, under the change's own tag for an
  unknown label, the count being the stops the cause held), and these lines are why the
  state's schema was v11 in 0.11.0's tree (since v12 they are keyed with the rest: "The
  state's schema", at the end). `agent check` says the same
  sentence for these causes, and `agent doctor --gate` says "a stop here would hold once, then
  be UNJUDGED". What is not closed: while a file stays unreadable or the marks stay, nothing
  else of that working tree is judged (staging the rest is not built), so failing work added
  beside it is not held after the first hold, only recorded `unjudged`; a file inside a
  directory the hook cannot list is not seen.

Deliberate evasion, made visible where that is cheap:

- A clone put at the project's path, a project that was moved, a state file or a list of
  commits that was damaged or replaced, a state directory inside the working tree:
  UNJUDGED, with the reason. A removed state file: the
  next stop says that the session's start was not recorded.
- An agent can make a stop UNJUDGED by breaking what the judge needs. That is never a pass
  and always visible to the user. From 0.11.0 the causes the hook finds in the working tree
  (a file that cannot be read, a path marked skip-worktree) hold the first stop once (below);
  the others (a Git that fails or is slow, a judge or policy that is missing) never block.
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
- What the advice never notices (F6.c). It reads the change the hooks' patch is made of,
  which leaves out files Git ignores (`.gitignore`, the user's excludes file, and what
  `.git/info/exclude` hides) unless they are in the index, and holds nothing of the Git
  directory: an edit of `.git/info/attributes`, `.git/info/exclude` or `.git/config` gets no
  item and no guard notice, and neither does an ignored guard file (often
  `.claude/settings.local.json`). Each of these can change what Git shows or runs; a stop's
  judge sees no more of them than the advice does.
- The advice's `--policy` and the stop's are each the one their command in the client's
  settings names. `agent setup` from 0.11.0 gives both the same; settings printed before
  0.11.0 give the post-tool command none, and settings edited by hand can give another. The
  advice then names an edit of `warden.toml` (or of its own `--policy`) as the policy file
  while the stops judge with another, and an edit of the stops' policy file gets no notice as
  one. The stops read their policy at the session's base whatever the advice said.
- Guard notices are said only by an advice that ranks the change. A session whose every tool
  call is cut short ("advice cut short") or whose change cannot be made (a skip-worktree mark,
  Git failing) never hears them while that lasts; the first call that ranks names them. A
  stop judges such a change as any other: the judge is given the guard file's edit with the
  rest when the change can be made, and when it cannot the stop is UNJUDGED with the reason.
- When no state can remember a reason the advice hook gave, it marks it under
  `.git/deep-diff-forge/` with a key of that directory. If that key file is no key (a link,
  a FIFO, a directory, another length), the advice hook says nothing beside the state and
  writes no mark, and no byte of the file is said; a stop says the reason every time, as it
  always does.
- "No repository here" is read from Git's own English message, with `LC_ALL=C` set for that
  call. A Git that words it otherwise is read as a failure, which is said, not as silence.
- The scratch directory of a hook that was killed is removed only where the file system has
  locks: on one without locks (some network and FUSE file systems) it is removed by no one and
  stays, with its copy of the patch, until a person removes it. A directory left by a build
  before 0.11.0 (`tmp-<pid>`) is removed where `/proc` says its process is gone (Linux). A
  0.10.0 binary removes no `tmp-r…` directory a killed 0.11.0 hook left: in a repository whose
  hooks are still 0.10.0 such a directory stays until a 0.11.0 hook or check runs there.
- **A resumed conversation is a new session of the hooks** when its client gives it a new
  session id. From 0.11.0 the commits an earlier conversation left without a stop's verdict
  are judged by the next stop of the project (the frontier, above), and a `SessionStart` with
  `source` `resume`, `compact` or `clear` begins from the commit the last session of its tree
  began from. The frontier is evidence kept where the agent can write: the ledger and its
  `baseline` are files of the repository's Git directory, so an agent that removes or edits
  them is judged as before 0.11.0 (its own range only; the stop says that no record was found),
  and a conversation that goes on under `startup` begins at `HEAD`, whose policy a commit of
  the earlier conversation can have loosened. Not a boundary: the judge on the pull request
  reads the policy at the base and sees every commit.
- **Linked working trees share branches.** A commit another session makes in another
  working tree of the project is on a branch this session's stops read, made during this
  session, so it is judged as this session's: its failure holds this stop once, and the
  author's once (measured: a session in the main tree that changed nothing was held for a
  failing commit made in a linked tree by another session). Who made a commit cannot be
  proven from what the agent can write (reflogs expire and can be written; messages and dates
  are the author's), so from 0.11.0 (F13.a) the block only names, as advice, the other
  working tree whose `HEAD` reflog records the commit: "(the HEAD reflog of <tree> has this
  commit; if this session did not make it, tell a person)". Nothing is taken from the hold.
- **Some stop outcomes cost the judged agent nothing at its next turn.** A failure that
  stands after its one block is said as FAIL STANDS (and recorded `fail_stands` in the
  ledger), a hold for a rule is for a person and never blocks, and advice that is off is said
  once. A nested repository is left out and named, at no cost: the hook cannot tell whether
  the session made it or it was there before (it has no history of a path, and a directory's
  times change with any Git command run in it), and holding one that was there before would
  hold the first stop of every session of a project with a vendored checkout. Each is said,
  to the user every time, and none is silent; none holds the agent. Not a boundary: the user
  sees each, and the judge on the pull request judges what was committed. From 0.11.0 a file
  the hook cannot read, paths marked skip-worktree and a hold for a label the judge does not
  know cost one blocked turn (below).
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
  each a deliberate act, measured on 0.10.0 with a stand-in judge. None is a
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
    silent pass (measured on 0.10.0, and again on the 0.11.0 branch before F12 and F13, where
    a Git selection of the working tree still finds no file there; walking such a path like any
    other is planned for 0.11.0 and not built yet).
  - A very large sparse file (`truncate -s 400G`, no disk used) makes Git's staging run out
    of the hook's time: the stop is UNJUDGED ("`git add` did not finish in the time a hook
    has and was stopped", 41 seconds), with no run of the judge, at every stop while the file
    is there, also with further failing work added. It is said to the user and never blocks.
  - A path the hook cannot read makes every stop UNJUDGED while it stays so, with no run of
    the judge: said to the user, never a block (0.10.0; from 0.11.0 it holds a stop once and
    again when it changes, at most three times, below). A failure that has blocked once in a
    path stands after the path is rewritten: when it is readable again and fails the same
    rule, the stop is judged and says FAIL STANDS; it does not block again (the memory of a
    failure is by its rule, signal and path, and the number of items the judge cites).
- The state of a session is up to three files in the user's state directory
  (`deep-diff-forge/hooks` under `$XDG_STATE_HOME`, else under `~/.local/state`), found by
  the session's id: the state, the list of commits that were there when it began, and the
  list of commits that have their answer. Files of sessions that have
  ended are not removed. Hooks for Codex are not designed.
- **The state's schema** (from 0.11.0; the hook-state version policy):
  - Names. The files are `session-<the first 32 hex digits of the SHA-256 of the session's
    id>` with `.json` (the state), `.tips` (the commits there when the session began, one id
    a line), `.judged` (the commits that have their answer) and `.lock` (an exclusive
    `flock`), and one file `key` for all of the user's sessions (since v12). Names and the
    lock do not change from one schema to the next: two builds of different versions share
    them.
  - The schema. The state's member `schema` is exactly `deep-diff-forge.hook-state.v<N>`,
    `N` in one to six decimal digits with no leading zero, read byte by byte; any other
    text is a damaged state. Any change to the state's members, to a member's type, grammar
    or meaning, or to the lines of `.tips` or `.judged` is a new `N`, between releases too.
    The lines of `.judged` are `ID:p`, `ID:h:<rules>|<class>`, `ID:f:<rules>`,
    `ID:n<attempts>@<stop>:<class>` and `ID:x:<class>`, where `<rules>` are the rule ids
    the judge names (those that fail, then after a `;` those it holds for; at most eight of
    each, 64 characters each, and `+N` for the others) and `<class>` a code of "Reason
    classes" below; any other letter makes the list damaged. Before v12 a hold kept the
    first 100 characters of the judge's first note after its `|`, and a commit with no
    verdict kept the judge's or Git's words: such a line is read for the class its words say
    and written again in v12's form.
  - What is read. A build reads every schema from the one the last release wrote (v10,
    written by 0.9.0 and 0.10.0) to its own (v13 since 0.11.0), and writes its own; one of a
    schema in that range below its own is written in its own by the first hook that keeps
    state. A v10 or v11 state keeps its base, its list of commits and its judged commits (a
    judged commit is not judged again); what it kept as plain digests is emptied (below,
    "Keyed history"), so a failure that blocked blocks once more. A state
    of an older schema is refused by its name: "this session's state file … is of schema
    v9, written by a deep-diff-forge older than the last release; this one reads v10 to
    v13, and nothing is recorded over it (removing it makes the next hook take the commit
    HEAD names as this session's base: what this session committed before is judged only
    where the project's ledger finds it (made after the project was first recorded, with no
    stop's verdict), and not otherwise; left as it is, the working tree is judged at every
    stop without the session's memory)". A damaged one (not JSON, a schema that is not one, a
    member missing or not what the hooks write) says why and the same about removing it. That
    is what removing it does: the next stop takes the commit `HEAD` names as the session's
    base, says that the session's start was not recorded, and the commits made before are
    judged where the ledger finds them (F13.b). While the state stays as it is, each stop is
    the stateless stop (below).
    A list beside the state that is not what the state names (`.tips` or `.judged` missing,
    cut, changed, or with a line that is not one) is refused with the same cost in its own
    words: "start a new session to have its work judged (a new session begins at the commit
    HEAD names then: what this session committed before is not judged)".
  - A newer state is never written over. A state of a schema above this build's own (a
    newer build of the hooks wrote it) is refused by its name: "this session's state file
    … is of schema v14, written by a newer deep-diff-forge; this one reads v10 to v13, and
    nothing was written over it". No hook writes the state, `.tips` or `.judged` then (a
    stop, an advice hook, a session's start; the lock may be taken), and the message names
    no removal: the newer build reads its state again. The stop is UNJUDGED, as for any
    state it cannot use, and the judge is not run. A hook writes a state only after reading
    what is there: a stop and a session's start hold the session's lock from their read to
    their write, an advice hook reads again under the lock, and every write reads the file
    once more before its lists and again just before the state is moved into place, and
    is abandoned (said as a state that could not be kept) when the file is then newer.
    **Stated limit:** where file locks do not keep two hooks of a session apart (a file
    system with none, or a build that takes none), a newer build that writes between those
    reads and this hook's writes is not seen, and its state or the lists beside it are
    replaced: the reads narrow that race and do not close it.
  - Why v11, and what it means for two builds on one session. F12's lines in `blocked` (a
    cause that holds once, kept by its name with the number of stops it held, under the tree's
    tag `o` or under the change's own tag) mean nothing to 0.10.0, which read them as v10:
    measured with the published 0.10.0 on a state this tree wrote, it read such a line of a
    linked tree as "A failure there has blocked a stop of this session and stands", and it
    forgot one under a change's tag at its next stop, so a hold for an unknown label came back
    at every stop of the newer build that followed one of its own (four blocks in eight
    stops, past the bound of three). So the schema is v11, and since then: the published
    0.10.0 (and 0.9.0) refuses a session a 0.11.0 hook has written, at every hook, as UNJUDGED
    ("cannot be read as a state (it is not a state of this version of the hooks) … (remove the
    file to have the next hook record the session anew)"), which is not a pass, and keeps the
    file byte for byte; following that released advice costs what is said above (the commits
    the session made before are not judged). A 0.11.0 hook reads a session 0.10.0 wrote and
    writes it as v11: a failure that blocked under 0.10.0 says FAIL STANDS and a commit it
    judged is not judged again. A commit that 0.10.0 judged HOLD for a label the judge does
    not know keeps that answer: it is not held once after the upgrade (its line in `.judged`
    is a hold like any other). v13 adds `guards`, the guard-file changes the advice has
    named: a build that writes v12 refuses a v13 state as newer, at every hook, and keeps it.
  - Keyed history (v12, rule V8). No file of the state directory holds text from the patch,
    the judge or Git: every digest the hooks keep (what was said as a finding, what was said
    once, every line of `blocked`: a rule, a rule with its signal and path, what the judge
    said beside its rules, the name and the state of a cause that holds once, the mark of many
    lines) is HMAC-SHA-256 under the user's key, `key` in the state directory: 32 random
    bytes, mode 0600, made by the first hook that keeps a state (written to a file of its own
    and linked into place, so of two first hooks the second reads the first's key), read only
    as a regular file of exactly 32 bytes and never through a final link. The state names the
    key it was kept under (`key_id`, 16 hex digits of a SHA-256 that gives nothing of the key
    away), and the file of judged commits keeps rule ids and class codes only. What the key
    protects, plainly: a copy of a state file, a ledger or a log without the key no longer
    confirms a guessed line, rule or path; anyone who can read the key file (the same user,
    the agent included) can still compute the digest of a guess. The key is not a boundary
    against the judged agent; it bounds what a copied file gives away. A key file that is no
    key (a link, a FIFO, a directory, another length, unreadable) ends the hook as a damaged
    state does (UNJUDGED, did not run); no byte of a key is ever said. The marks an advice
    hook leaves under `.git/deep-diff-forge/` when no state can remember a reason it said
    (`said-` and 64 hex digits) are keyed too, under a key file `key` of that directory,
    made the same way, since the state's key may be what cannot be read (F6); a mark of the
    form before (`said-`, 16 hex digits, `-`, 16 more: plain digests of the session and the
    reason) is removed when an advice hook meets it, and its reason is said once more.
  - Guard changes (v13, F6.b). `guards` holds the keys of the guard-file changes the advice
    has named, HMAC-SHA-256 of the path and what Git says of the file's content in the change
    (its `index` line with both blob ids, its modes), at most 64: a set of their own, so that
    they never take the places of the 32 reasons an advice hook says once. A change past 64 is
    named after every tool call rather than never, and says so.
  - What emptying costs. When a state's digests cannot be used (a state before v12, whose
    digests were plain; a key that was replaced or lost, which `key_id` shows), `reported`,
    `noticed`, `guards` and `blocked` are emptied and nothing else: a finding or a guard
    change already said is said once more, a failure that already blocked blocks once more and then stands, and a cause
    that holds once holds again with its count begun anew. So the bound of three holds for
    one cause is per key and per schema, not per session: a session that is upgraded from
    v11 and whose key is replaced once can be held by one cause 3 × 3 = 9 times at worst,
    each time at a turn's first stop only. Emptying never lets through a stop that would
    have held or blocked: it can only make a stop hold or block once more.
  - The fixtures `crates/deep-diff-forge-cli/tests/fixtures/hook-state/<schema>-<release>/`
    hold the three files the published 0.9.0 and 0.10.0 binaries wrote, and the same files
    written by builds of this tree in v11, v12 and v13 (with the key file from v12; each directory's
    `ORIGIN` names the binary by its SHA-256 and says whether it was published), captured once by
    `scripts/capture_hook_state.sh` and never edited; the test
    `every_schema_since_the_last_release_is_read` has the build under test read each.


Reason classes (from 0.11.0). Every outcome a stop gives a unit, and every record of the
project's ledger, carries one code: why the outcome is what it is. A code never decides
whether a stop holds; the last column says what the stop's own rules decide for it ("once for
each cause, at most three stops" is the rule of the causes that hold once, above). The rule of
the codes: a code's meaning never changes; codes are added without a version change; a
consumer treats an unknown code as not a PASS; removing one is a new ledger version. The codes
`pass`, `fail.new`, `fail.stands`, `fail.unsaved_continuing`, `hold.rule`, `hold.version_gap`,
`work.unreadable` and `work.skip_worktree` were written before this table and keep their
spelling. Where the ledger wrote `"class": null` before (an unjudged unit, a hold with no rule,
a stop that judged nothing), it writes the code now. The commits a stop says "from before" are
recorded with the class their kept words say; a reason kept with no words this build wrote is
`chain.judge_exit`.

| Code | The stop prints | Holds a stop |
|---|---|---|
| `pass` | nothing, or a note | never |
| `fail.new` | block | once for each failing line |
| `fail.stands` | FAIL STANDS | never |
| `fail.unsaved_continuing` | FAIL STANDS (the hook cannot keep its own record) | never |
| `hold.rule` | HOLD | never |
| `hold.claims_unverified` | HOLD | never |
| `hold.notes` | HOLD | never |
| `hold.version_gap` | block once, then HOLD | once for each cause, at most three stops |
| `work.unreadable` | block once, then UNJUDGED | once for each cause, at most three stops |
| `work.skip_worktree` | block once, then UNJUDGED | once for each cause, at most three stops |
| `chain.judge_missing` | UNJUDGED | never |
| `chain.judge_inside` | UNJUDGED | never |
| `chain.policy_missing` | UNJUDGED | never |
| `chain.input_unwritable` | UNJUDGED | never |
| `chain.no_ranker` | UNJUDGED | never |
| `chain.rankers_unlisted` | UNJUDGED | never |
| `chain.ranker_unsealed` | UNJUDGED | never |
| `chain.judge_refused` | UNJUDGED | never |
| `chain.judge_environment` | UNJUDGED | never |
| `chain.judge_exit` | UNJUDGED | never |
| `chain.not_a_verdict` | UNJUDGED | never |
| `chain.patch_too_large` | UNJUDGED | never |
| `time.judge_stopped` | UNJUDGED | never |
| `time.no_time_left` | UNJUDGED | never |
| `time.not_asked` | UNJUDGED | never |
| `time.git` | UNJUDGED | never |
| `state.damaged` | the tree judged without the state | never |
| `state.older` | the tree judged without the state | never |
| `state.newer` | the tree judged without the state | never |
| `state.no_directory` | the tree judged without the state | never |
| `state.inside_tree` | UNJUDGED | never |
| `state.lock_busy` | the tree judged without the state | never |
| `state.lists_damaged` | the tree judged without the state | never |
| `state.no_baseline` | UNJUDGED | never |
| `state.not_apart` | UNJUDGED | never |
| `place.git_failed` | UNJUDGED | never |
| `place.unpaired` | UNJUDGED | never |
| `place.directory_gone` | UNJUDGED | never |
| `place.client_dir_differs` | UNJUDGED | never |
| `place.replaced` | UNJUDGED | never |
| `place.moved` | UNJUDGED | never |
| `place.orphan` | UNJUDGED | never |
| `place.before_first_commit` | UNJUDGED | never |
| `place.no_commit_yet` | a note | never |
| `place.head_unreadable` | UNJUDGED | never |
| `place.base_gone` | UNJUDGED | never |
| `place.unmerged` | UNJUDGED | never |
| `place.shallow_parent` | UNJUDGED | never |
| `place.scratch` | UNJUDGED, did not run | never |
| `input.refused` | UNJUDGED, did not run | never |

---

See also: [`API_AND_IPC`](../API_AND_IPC.md) · [`TESTING_GOLD_STANDARD`](../TESTING_GOLD_STANDARD.md).
