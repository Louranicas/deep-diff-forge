## What and why

<!-- The behaviour this changes and the problem it solves. Link an issue if there is one. -->

## How it is proven

<!-- The test that fails without this change, or why none can. -->

## Checklist

- [ ] `cargo fmt --all --check` and `cargo clippy --workspace --all-targets --locked -- -D warnings -W clippy::pedantic` are clean
- [ ] `cargo test --workspace --locked` passes (or `scripts/ci_local.sh`)
- [ ] The apply-able patch is never rewritten by this change
- [ ] Docs that state a changed contract are updated, and `CHANGELOG.md` has an `Unreleased` entry if users would notice
- [ ] Sockets or platform code: I've asked for the macOS job
