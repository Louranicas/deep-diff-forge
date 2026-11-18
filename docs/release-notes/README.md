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
