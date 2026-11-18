# Release notes

The notes of every release, newest first. Each file holds that version's full section of the
[changelog](../../CHANGELOG.md), moved there word for word; the changelog keeps a short entry for
each version and links to its file.

The signed release pages are on GitLab, the public home of the project
(<https://gitlab.com/lukeomahoney/deep-diff-forge/-/releases>); Gitea and Forgejo mirror it.
Security advisories are in [`docs/advisories/`](../advisories/).

| Version | Date | In one line | Notes |
| --- | --- | --- | --- |
| 0.9.0 | 2026-11-18 | Five integrity signals, the signal vocabulary as data and the agent hooks; fixes for released versions ([DDF-2026-003](../advisories/DDF-2026-003.md)). | [0.9.0](0.9.0.md) |
| 0.8.4 | 2026-11-16 | Security release: reviews that differed from `git apply` (rescore of v0.8.3, G1 and G2) and Rust code hidden from every rule that reads Rust code. | [0.8.4](0.8.4.md) |
| 0.8.3 | 2026-11-11 | Security release: a change shown on one file was written to another, a new file's mode, and content under a mode Git reads as a submodule. | [0.8.3](0.8.3.md) |
| 0.8.2 | 2026-11-09 | Security release: a copy, and a `---`/`+++` name with nothing after its first component, could each be reviewed as a different file than `git apply` writes. | [0.8.2](0.8.2.md) |
| 0.8.1 | 2026-11-07 | Security release: a line between two `rename to` lines named a destination Git did not write. | [0.8.1](0.8.1.md) |
| 0.8.0 | 2026-11-06 | Breaking: patches Git refuses or reads differently are refused; Git's hunk grammar byte for byte, bounded child processes, a daemon response deadline. | [0.8.0](0.8.0.md) |
| 0.7.0 | 2026-11-03 | Signed release tags that the release workflow verifies, reproducible archives, SBOM source checksums, a daemon read budget and request deadlines. | [0.7.0](0.7.0.md) |
| 0.6.0 | 2026-11-02 | The arm64 Linux target, `<old> <new>` for two regular files, Prometheus daemon metrics and a 34-case evaluation corpus. | [0.6.0](0.6.0.md) |
| 0.5.0 | 2026-10-31 | Code evidence in risk ranking, verified agent findings, daemon metrics, and the `bench` and `eval` commands. | [0.5.0](0.5.0.md) |
| 0.4.0 | 2026-10-30 | Daemon requests run in parallel, the socket checks peer credentials, starts cannot race, and `deploy doctor` exists. | [0.4.0](0.4.0.md) |
| 0.3.0 | 2026-10-29 | The library APIs changed incompatibly since 0.2.1; the expert-circle, bug-hunt and owner-decision entries landed before the tag. | [0.3.0](0.3.0.md) |
| 0.2.1 | 2026-09-04 | Security hardening of the daemon, the learning store, dependencies and the release workflow; 0.3.0 says 0.2.1 was never tagged. | [0.2.1](0.2.1.md) |
| 0.2.0 | 2026-06-23 | The L9 (Learning) layer, a supply-chain and fuzz hardening pass, and the interactive review cockpit. | [0.2.0](0.2.0.md) |
| 0.1.0 | 2026-06-22 | First tagged release: the complete diff and review engine, layers L1 to L7. | [0.1.0](0.1.0.md) |

## Project history

The dated history that stood under "Project status" in the README until 0.9.0.
Statements about one version are in that version's file, under "Status at release".

**0.9.0 is not production-ready yet (verdict of 2026-11-18).** The independent rescore of
v0.8.1 (2026-11-09) scored
[80/100](../PRODUCTION_READINESS.md#assessment-2026-11-09-v081-d290379-independent-rescore)
and found two high findings, both present in v0.8.1 and earlier: a copy, and a `---`/`+++` name with
nothing after its first component, could each be reviewed as a different file than the one
`git apply` writes ([GHSA-gp5v-m67f-gmgp](https://github.com/Louranicas/deep-diff-forge/security/advisories/GHSA-gp5v-m67f-gmgp), published 2026-11-10). 0.8.2
fixes both, and two more of the kind that a rescore of the code before its tag found
([80/100](../PRODUCTION_READINESS.md#assessment-2026-11-09-main-af5c3d1-the-untagged-082-independent-rescore-before-the-tag):
a header after a binary line, and a carriage return in a move name). The last three
independent rescores (v0.8.0, v0.8.1 and that one) each scored 80 and each found a new high
finding of this kind ([release evidence](../../EVIDENCE.md#v082-release-2026-11-10)). The independent
rescore of the released v0.8.2 (2026-11-11) scored
[70/100](../PRODUCTION_READINESS.md#assessment-2026-11-11-v082-fb5ffe1-independent-rescore)
and found another: a `diff --git` line followed directly by a binary line left its name for
the next section, so a change shown on one file was written to another. 0.8.3 fixes that, a
new file's mode overridden by a `new mode` line, and content under a mode Git reads as a
submodule ([GHSA-rq5r-p3g9-phff](https://github.com/Louranicas/deep-diff-forge/security/advisories/GHSA-rq5r-p3g9-phff)); a binary body Git finds corrupt is still shown
as applied ([CHANGELOG](../../CHANGELOG.md)). The released v0.8.3 refuses the finding patches
([release evidence](../../EVIDENCE.md#v083-release-2026-11-11)). The independent rescore of the
released v0.8.3 (2026-11-12) scored 67/100 and found two more high findings, G1 and G2: a
`Files … differ` line, which Git reads as a binary marker, and one path in several
sections, which Git applies as a set. 0.8.4 fixes both, and Rust code the code view hid
([CHANGELOG](../../CHANGELOG.md)); 0.9.0 adds the integrity signals and the agent hooks and fixes
a construct cut across two sections of one path and two costs quadratic in the length of a
line ([DDF-2026-003](../advisories/DDF-2026-003.md)). Neither 0.8.4 nor 0.9.0 has had an independent rescore (0.9.0's
integration had two reviews of its own, not a rescore), and each of the last five independent
rescores found a new high finding. The
declared scope
is a local, single-user
review tool on Linux and macOS ([the four targets](../../README.md#supported-platforms)): CLI, Unix-socket
daemon, `rpc`, MCP and SDKs, released as GitHub binaries. A scored assessment (2026-10-30, [83/100](../PRODUCTION_READINESS.md)) named
the gaps that 0.4.0 closed: daemon requests run in parallel, the socket checks peer
credentials, starts cannot race, `deploy doctor` exists, macOS runs on every pull request,
and a weekly campaign soaks the daemon and fuzzes every target for an hour
([EVIDENCE.md](../../EVIDENCE.md#v040-release-readiness-2026-10-30)). 0.5.0 added code evidence in
risk ranking, verified agent findings, daemon metrics, and the `bench` and `eval` commands,
and fixed DDF-EVID-001 ([EVIDENCE.md](../../EVIDENCE.md#v050-release-readiness-2026-10-31)). Its
assessment of 2026-10-31 scored 91 (self) and 81 (an independent rescore by Codex), reconciled
at [86/100](../PRODUCTION_READINESS.md#reconciled-score-86--100), with the open items listed there. 0.6.0 adds the
arm64 Linux target, `<old> <new>` for two regular files, Prometheus daemon metrics and a 34-case
evaluation corpus; the assessment of 2026-11-02 (`main` `1eaba47`) was reconciled at
[89/100](../PRODUCTION_READINESS.md#reconciled-score-89--100). 0.7.0 adds signed release tags
that the release workflow verifies, reproducible archives, SBOM source checksums, a daemon read
budget and request deadlines, and Git-parity fixes for renames and modes; an adversarial circle of
experts (2026-11-03) reconciled it at
[86/100](../PRODUCTION_READINESS.md#assessment-2026-11-03-adversarial-circle-of-experts). The assessment of
2026-11-04 scored 0.7.0 at [80/100](../PRODUCTION_READINESS.md#assessment-2026-11-04-main-7c0910b-v070-and-test-only-fixes-claude):
a hunk header without its space hid a file that `git apply` wrote. 0.8.0 fixes that and what
came with it (Git's hunk grammar byte for byte, checked against `git apply`; bounded child
processes; a daemon response deadline). An independent rescore of v0.8.0 (2026-11-07) also
scored [80/100](../PRODUCTION_READINESS.md#assessment-2026-11-07-v080-dd1aa89-independent-rescore):
a line between two `rename to` headers made the review name a destination Git did not write.
0.8.1 fixes that, with a `git apply` differential that breaks every extended-header position.
That differential compared line counts only; it now compares Git's summary too (copies,
created and deleted files, and modes). The evidence below was
measured for `v0.3.0` on 2026-10-29 and still holds:

That evidence is in the [0.3.0 notes](0.3.0.md#status-at-release).
