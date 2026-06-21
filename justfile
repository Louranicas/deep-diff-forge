# Deep-Diff-Forge command runner.
#
# Assimilated patterns:
# - Rust service template: check, clippy, test, gate.
# - Workspace factory justfile: read-only status/wiring probes and receipts.
# - Small crate justfiles: repo-local CARGO_TARGET_DIR to avoid global cache drift.

set shell := ["bash", "-uc"]
set tempdir := "/tmp"

export CARGO_TARGET_DIR := "target"

[default]
default:
    @just --list --unsorted

# Show repository identity and deployment posture.
[group("observe")]
status:
    @printf 'repo=%s\n' "$(basename "$PWD")"
    @git status --short --branch
    @git remote -v
    @cargo metadata --no-deps --format-version 1 >/dev/null
    @printf 'cargo_metadata=ok\n'

# Run Rust formatter check.
[group("quality")]
fmt:
    cargo fmt --all --check

# Compile the full workspace.
[group("quality")]
check:
    cargo check --workspace

# Run clippy with warnings denied.
[group("quality")]
clippy:
    cargo clippy --workspace --all-targets -- -D warnings

# Run pedantic clippy with warnings denied.
[group("quality")]
pedantic:
    cargo clippy --workspace --all-targets -- -D warnings -W clippy::pedantic

# Run workspace tests.
[group("quality")]
test:
    cargo test --workspace --locked

# Report current test count by crate. This is informational until production modules exist.
[group("quality")]
test-audit:
    #!/usr/bin/env bash
    set -euo pipefail
    for crate in crates/*; do
      [ -d "${crate}" ] || continue
      name="$(basename "${crate}")"
      count="$( (rg -n '#\[(tokio::)?test\]|rstest|proptest!' "${crate}" 2>/dev/null || true) | wc -l | tr -d ' ')"
      printf '%s tests=%s minimum=50 status=%s\n' "${name}" "${count}" "$([ "${count}" -ge 50 ] && echo pass || echo bootstrap-gap)"
    done

# Run all bootstrap contract probes.
[group("contracts")]
contracts:
    cargo run -p deep-diff-forge-cli -- --self-test
    cargo run -p deep-diff-forge-cli -- doctor
    cargo run -p deep-diff-forge-cli -- claude-code-contract
    cargo run -p deep-diff-forge-cli -- chain-contract
    cargo run -p deep-diff-forge-cli -- cluster-contract
    cargo run -p deep-diff-forge-cli -- loom-contract

# Docs-only gate for architecture and planning changes.
[group("gates")]
gate-docs: fmt check

# Bootstrap deployment gate for the current L0 codebase.
[group("gates")]
gate-bootstrap: fmt check contracts

# Feature gate for Rust implementation changes.
[group("gates")]
gate-feature: fmt check clippy pedantic test contracts

# CI-equivalent local gate. This should remain stricter than gate-bootstrap.
[group("gates")]
ci: gate-feature

# Read-only Zellij observation. This never controls deployment truth.
[group("observe")]
zellij-observe:
    #!/usr/bin/env bash
    set -euo pipefail
    if command -v zellij >/dev/null 2>&1; then
      zellij list-sessions || true
    else
      echo "zellij=unavailable"
    fi

# Read-only habitat/factory observation. This is advisory for this repo.
[group("observe")]
habitat-observe:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ -x ../bin/factory-status ]; then
      ../bin/factory-status --mode gate_only || true
    elif command -v factory-status >/dev/null 2>&1; then
      factory-status --mode gate_only || true
    else
      echo "factory-status=unavailable"
    fi
    if [ -x ../bin/factory-wiring ]; then
      ../bin/factory-wiring || true
    elif command -v factory-wiring >/dev/null 2>&1; then
      factory-wiring || true
    else
      echo "factory-wiring=unavailable"
    fi

# Local doctor: repo status, contract probes, and optional external observation.
[group("diagnostics")]
doctor: status contracts zellij-observe habitat-observe

# Write a bootstrap deployment receipt under reports/deployments/.
[group("receipts")]
receipt-bootstrap:
    #!/usr/bin/env bash
    set -euo pipefail
    stamp="$(date -u +%Y%m%dT%H%M%SZ)"
    dir="reports/deployments/${stamp}"
    mkdir -p "${dir}"
    {
      printf 'repo=deep-diff-forge\n'
      printf 'stamp=%s\n' "${stamp}"
      git rev-parse --short HEAD | sed 's/^/commit=/'
      git status --short --branch | sed 's/^/git_status=/'
    } > "${dir}/manifest.txt"
    just status > "${dir}/status.txt" 2>&1
    just gate-bootstrap > "${dir}/gate-bootstrap.txt" 2>&1
    just zellij-observe > "${dir}/zellij.txt" 2>&1 || true
    just habitat-observe > "${dir}/habitat.txt" 2>&1 || true
    cat > "${dir}/summary.json" <<EOF
    {
      "schema": "deep-diff-forge.deployment-receipt.v0",
      "repo": "deep-diff-forge",
      "commit": "$(git rev-parse --short HEAD)",
      "mode": "bootstrap",
      "stamp": "${stamp}",
      "gates": {
        "status": "pass",
        "gate_bootstrap": "pass",
        "zellij": "observed",
        "habitat": "observed_optional"
      }
    }
    EOF
    printf 'receipt=%s\n' "${dir}"
