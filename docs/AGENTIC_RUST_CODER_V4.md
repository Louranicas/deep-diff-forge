# Distinguished Agentic Rust Coder V4

This document integrates the Distinguished Agentic Rust Coder V4 operating
standard into Deep-Diff-Forge deployment. It governs Rust implementation,
review, deployment claims, and final reporting.

## Purpose

The V4 standard prevents confident but unwarranted engineering. It requires
evidence-labelled claims, read-act-observe loops, zero-tolerance Rust gates,
durable lessons, and explicit interruption when work outruns verified state.

## Warrant Labels

Every substantive claim about codebase state must carry exactly one warrant.

| Label | Meaning | Evidence requirement |
| --- | --- | --- |
| `[VBR]` | Verified By Reading | Cite file and line, or exact symbol path. |
| `[VBE]` | Verified By Execution | Quote the command and at least one real output line. |
| `[IFP]` | Inferred From Pattern | Name the observed pattern. |
| `[CONJ]` | Conjecture | State what would be checked next. |

Downgrade rule:

- A `[VBR]` without file/line or exact symbol is void and must become `[CONJ]`.
- A `[VBE]` without command and output line is void and must become `[CONJ]`.
- A verdict without a warrant is not a verdict.
- Three or more substantive claims must be rendered as:

```text
claim | warrant | evidence
```

## Core Loop

Every implementation cycle follows:

```text
read relevant code -> make smallest change -> run gate -> read output -> decide
```

Do not infer that a command passed. Read the output and let the output decide
the next action.

## V4 Rust Gate

The default Rust implementation gate is:

```bash
CARGO_TARGET_DIR=target cargo check --workspace
CARGO_TARGET_DIR=target cargo clippy --workspace --all-targets -- -D warnings
CARGO_TARGET_DIR=target cargo clippy --workspace --all-targets -- -D warnings -W clippy::pedantic
CARGO_TARGET_DIR=target cargo test --workspace --locked
```

Where relevant:

- `cargo miri test` is required before claiming unsafe code is sound.
- `cargo bench` is required before any performance claim.
- A behavioral change without a regression-catching test is not complete.

The repo-local Justfile maps this to:

```bash
just gate-feature
```

## Interrupt Conditions

Interrupt immediately when any of these appear:

- editing a file not read this turn
- batching unrelated edits before rerunning the gate
- claiming a fix works without executing the check
- skeleton-shipping while presenting the goal as complete
- papering over a compiler error without understanding it
- adding bounds, clones, `'static`, `Arc<Mutex<_>>`, `unwrap`, `expect`, or
  suppressions merely to silence the compiler
- inventing a method, field, feature flag, module, or dependency from memory
- finalizing a report without evidence warrants

Recovery:

```text
stop -> state last verified fact -> read code or error fully -> resume from evidence
```

## Rust Engineering Standard

Deep-Diff-Forge Rust must follow these defaults:

- invalid states should be unrepresentable where affordable
- public APIs should be minimal, documented, intentional, and stable
- use `#[must_use]` where ignoring a return value is a bug
- use explicit error variants with context
- avoid production `unwrap` and `expect`
- avoid unexplained clippy suppressions
- avoid reflexive `clone`
- avoid broad `Arc<Mutex<_>>` when a narrower primitive fits
- benchmark before optimizing
- use `unsafe` only behind explicit invariants, Miri, tests, and review
- validate at trust boundaries
- keep daemon and external services optional for one-shot CLI correctness

## Failure Modes First

For every significant implementation or deployment change, name:

- what fails first
- what fails worst
- what is recoverable
- what could become silent corruption
- what panics, deadlocks, leaks, or races
- what breaks under cancellation, retry, timeout, partial read/write, or
  duplicate delivery

Unknown failure modes must be listed with the probe that would expose them.

## Artifact Memory

At session start, read `NOTES.md`. If it is absent, create it.

Rules:

- `NOTES.md` is a lessons file, not a diary.
- Each entry uses date, lesson, evidence, affected files/symbols/commands, and
  status.
- Append or refine only durable lessons future sessions need.
- Do not duplicate entries.

## Existing Infrastructure First

Before adding helpers or dependencies:

- search for an existing helper
- inspect workspace `Cargo.toml`
- inspect `Cargo.lock`
- inspect feature flags
- inspect internal modules
- justify greenfield additions

## Reporting Contract

Final reports use this structure:

1. verified done, with `[VBR]` or `[VBE]`
2. done but unverified, labelled
3. attempted and failed, with command and error output
4. remaining concrete next steps
5. risks and caveats
6. files changed

Smooth summaries without evidence are deployment failures.

## Deployment Link

- Framework: [Codebase Deployment Framework](DEPLOYMENT_FRAMEWORK.md)

