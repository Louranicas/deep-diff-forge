# Vision

> **What this is.** What Deep-Diff-Forge is for, what it promises, whom it serves, where it is
> going and what it will refuse to become. Written 2026-11-23, after release 0.10.0. Where a
> sentence describes something not yet released, it says so. For what the code does today, see
> the [README](../README.md), the [reference](reference/README.md) and the
> [0.11.0 release notes](release-notes/0.11.0.md).
>
> **History.** The text of 2026-11-01 (four product modes, the north star, the differentiation
> thesis) is in this file's history at the tag `v0.10.0`. Of its four modes, the terminal review
> and the agent surfaces were built; the pager mode and the desktop and web renderers are not
> planned.

## In one sentence

**Whoever must decide on a code change, a person, a script or another machine, is shown the
change that will really be applied, where in it the risk lies, and what could not be read, in a
form that an author with every reason to pass cannot talk its way around.**

Shorter: *a review you can stake a decision on, when the author is a machine that wants to pass.*

## The world it answers

Code is now written faster than it can be read. A growing share of it is written by agents. They
do not tire and they do not lie in the human sense; they optimise. Whatever a check measures,
they move toward, and the cheapest path past a check is often not the repair.

Review is the last place where a change meets judgement before it becomes the code. It fails in
three ways, and each has grown worse as authors became machines.

1. **The reviewer is not shown what will be applied.** A patch can display one thing and apply
   another. One line in an attributes file can make a source file vanish from the diff. An
   encoding, a NUL byte or a renamed copy can turn text into "binary file changed".
2. **The reviewer cannot read it all.** A change of a thousand files is read in the order
   someone chose. If that order is alphabetical, the one line that matters is found by luck.
3. **The check can be satisfied without the work being done.** A test is skipped, a lint is
   silenced, an assertion is rewritten so that it cannot fail, an error is swallowed, and the
   session ends at a moment when nobody is looking.

Deep-Diff-Forge is built against these three failures and against nothing else.

## What it is

A sensor for code changes. It reads a change exactly as `git apply` would, or refuses it by
name. It says which files to read first and why, with evidence on the line. It says what it
could not see. It tells an agent what was found and which repairs are accepted. At the moment
an agent claims to be finished, it asks an independent judge and reports the verdict, or says
plainly that no verdict was given.

It is not the judge. It never says pass or fail on its own authority, it never rewrites the
patch, and it sends no code anywhere. Its one rule has not changed since the first release:
**the patch that can be applied is the truth, and nothing may rewrite it.**

The scope is deliberately small: a local, single-user tool on Linux and macOS, reached through
a command line, a daemon on a local socket, a stdio protocol, MCP tools and two small SDKs.

## Five promises

Each is a property that a test can break.

| | The promise | In practice | State on 2026-11-23 |
|---|---|---|---|
| 1 | **What you review is what applies.** | For every patch it accepts, a plain `git apply` writes exactly what the review shows, or refuses the whole patch. A header shape Git does not itself write is refused with a named reason. | Released in 0.10.0. |
| 2 | **Evidence is located, and a release never sees less than the last.** | A label stands on a line, found by a reader whose every step can be replayed. A widened reading may add evidence and never remove it. What a past release found, the next one finds, unless a listed decision says otherwise. | Released; the listed exceptions are in the changelog. |
| 3 | **Nothing is silent.** | A file that could not be read is labelled unread. A stop that no judge judged is called unjudged and never a pass. Every refusal has one meaning and a name, and survives a closed pipe or a full disk. | Released in part in 0.10.0 and more in 0.11.0; the silent and unjudged paths that remain are stated limits ([SECURITY](../SECURITY.md)). |
| 4 | **One answer on every surface.** | The command line, the daemon, the stdio protocol, MCP and the SDKs return the same sealed bytes for the same change, with the identity of the scorer inside them. | Released for the review and the ranking, and from 0.11.0 for the diagnostics document. |
| 5 | **A claim is worth exactly its evidence.** | Every sentence in the documents is true of the binary, with its exceptions in the same sentence. The reference tables are held word for word by tests. A release is built twice to one hash and signed, and the commands its notes quote are run on it. | Released; the check on the notes' commands was added after 0.10.0's notes quoted one the binary refused. |

## Designed for an author who optimises

The ordinary tool assumes an author who wants the code to be right. Deep-Diff-Forge assumes an
author who wants the check to pass, and is built so that the two aims meet.

- **Advice names the accepted repair and nothing else.** When a substitute would only move the
  finding (deleting a test instead of fixing it; an assertion that cannot fail), the tool says so
  in advance and names the label the substitute raises. Each such sentence stands on a witness
  patch that the engine is tested against.
- **A diagnostic fits its case.** A line with a hidden character is told to remove it, not to
  fix a path. Advice that does not apply teaches an agent to ignore advice.
- **Cheap exits are closed by rule, not by suspicion.** A stop that the session itself made
  impossible to judge holds once, on evidence the hook has, never on a guess at intent, and is
  bounded so that it cannot trap a session. *(0.11.0.)*
- **Probing leaves a trace.** A check run inside a session is recorded beside the stops in a
  ledger, and the count is described as what it is: a lower bound. *(0.11.0.)*
- **The judge is someone else.** Verdicts come from forge-warden, against a policy read at the
  session's base and a ranker pinned by its hash. The hooks are advice and say that they are not
  a security boundary ([agent hooks](reference/agent-hooks.md)). The boundary is the judge on
  the merge request.
- **The tool says where it can be beaten.** Every known way to make a stop silent or unjudged,
  and every text it cannot yet read, is written in the limits ([SECURITY](../SECURITY.md)).

## Whom it serves

| For | It provides |
|---|---|
| **The agent in the loop** | Diagnostics it can act on (a stable code, a place, the repairs of its case); the stop's judgement on demand before it claims to be done; refusals it can parse. |
| **The person who decides** | A ranking with reasons, the evidence on the line, the known issues, and sentences about the stop that claim equal inputs and never an equal verdict. |
| **The judge and the pipeline** | One canonical ranking with the scorer's identity; a gate flag that fails on chosen labels; a branch verified against the record of its stops; exit codes with a registry. |
| **The maintainer** | A register of every contract change with its test and its consumer; one local gate; corpora that cannot vanish; a release that reproduces. |

## The road

**0.10.0, released: the input is true.** The parser reads only what Git writes. A file hidden
from the diff is read again and labelled with the cause. Input with no patch is refused instead
of answered with an empty review. The ranking has one byte form. The stop reads the judge's
newer verdict and holds an unread failure. Releases are built twice, signed and receipted.

**0.11.0, released: the loop is honest.** Refusals with one meaning. Diagnostics an agent
can act on, on every surface. The stop's judgement on demand, equal to the stop in its units and
inputs or saying where it is not. A durable record of what each stop decided. Cheap exits closed
by rule. Readers widened where a check is switched off by a newer spelling.

**The finish line.** A threat model that names every boundary, and for each a test that plants
the breach and proves the refusal. No unexplained surviving mutant in the parser and the
evidence scanner. The release rebuilt from source on a clean clone. Two independent assessments,
by assessors of different model families, each at least 90 of 100 with no high finding, on the
released binary. Then the owner accepts. "Finished" does not mean "no known defects": the
residual risks stay listed.

**Beyond the line (designs only; see [pioneer feature specs](PIONEER_FEATURE_SPECS.md)).**
*Capability delta*: what a change lets the code do that it could not do before, stated as a
lower bound. *Trajectory advice*: weakening patterns named across a session, never as a rank.
*Arena sieve*: among several candidate changes, refuse the unfit; never crown a best.

## What it will not become

- **Not a judge.** It measures and reports. Policy belongs to forge-warden and to people.
- **Not a linter, a formatter or a substitute for tests.** It reads changes, and it notices when
  a change weakens those tools.
- **Not an oracle of intent.** It says where and what, never why. It checks where an agent's
  finding points, never what the finding claims.
- **No model inside the evidence path.** Evidence that a gate relies on comes from a linear
  reader, never from a grammar it cannot bound or a network it cannot replay.
- **No "best".** It ranks risk within one change and refuses unfit candidates. It will not order
  candidates by merit, and no score of its is a probability.
- **No claim without a measurement, and no upload.**

## How we will know

| Promise | Measured for each release | At the finish line |
|---|---|---|
| What you review is what applies | A differential against `git apply` and the conformance table of the patch crate | The same, extended to a real apply over a seeded object store |
| Evidence located, never less | Precision and recall on the labelled corpus, published with every declared miss ([evaluation](reference/evaluation.md)); a comparison corpus that shows what was lost and gained | Every label with corpus cases; no unexplained surviving mutant |
| Nothing silent | A registry of refusal reasons held by tests; the stop's outcomes compared before and after every change to it | A planted-breach test for "no judge ran" and for each boundary |
| One answer everywhere | Byte comparison of the ranking across the command line, the stdio protocol and MCP | The same for the diagnostics document, with a parity check in `agent doctor` |
| A claim is worth its evidence | Reference tables held by tests; two builds, one hash; signed checksums ([EVIDENCE](../EVIDENCE.md)) | Every command in the notes run on the release binary; two independent assessments |

## The last line

Deep-Diff-Forge will be finished when a reviewer who distrusts it completely can still rely on
it, because everything it says can be checked, and everything it cannot say, it says it cannot.

## Deployment Link

Deployment and release are described in [the deployment framework](DEPLOYMENT_FRAMEWORK.md) and
[release and publication](RELEASE_AND_PUBLICATION.md).

## Review integration route

[Implementation status and delivery gates](RECOMMENDATION_INTEGRATION.md#remaining-module-routes). This route records planned and evaluation-dependent work without claiming it is implemented.
