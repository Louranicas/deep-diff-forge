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

# Launch the interactive review cockpit on a git diff (keys read from /dev/tty — needs a real terminal).
# Examples:  just review             # uncommitted changes
#            just review HEAD~1 HEAD  # the last commit
#            just review main...HEAD  # this branch vs main
[group("review")]
review *ref="":
    cargo build --release --bin deep-diff-forge
    git diff {{ref}} | ./target/release/deep-diff-forge review

# Launch the review cockpit directly in side-by-side layout.
[group("review")]
review-side *ref="":
    cargo build --release --bin deep-diff-forge
    git diff {{ref}} | ./target/release/deep-diff-forge review --side

# Headless: render one review frame to stdout (no TTY needed — quick sanity check / CI).
[group("review")]
review-probe *ref="":
    cargo build --release --bin deep-diff-forge
    git diff {{ref}} | ./target/release/deep-diff-forge review --probe --cols 120 --rows 40

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
    cargo check --workspace --locked

# Run clippy with warnings denied.
[group("quality")]
clippy:
    cargo clippy --workspace --all-targets --locked -- -D warnings

# Run pedantic clippy with warnings denied.
[group("quality")]
pedantic:
    cargo clippy --workspace --all-targets --locked -- -D warnings -W clippy::pedantic

# Run workspace tests.
[group("quality")]
test:
    cargo test --workspace --locked

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

# Build docs the way docs.rs will, failing on a broken intra-doc link.
[group("quality")]
docs:
    RUSTDOCFLAGS="-D warnings -D rustdoc::broken_intra_doc_links" cargo doc --workspace --no-deps --locked

# Local replica of every CI gate job, step for step (anchor B-07; checked by tests/ci_replica.rs).
[group("gates")]
ci:
    scripts/ci_local.sh

# Release gate: the feature gate plus the docs.rs-equivalent doc build.
[group("gates")]
gate-release: gate-feature docs

# Verify publish-readiness without uploading. core+learning fully verify;
# downstream crates resolve their internal deps from crates.io only after the
# upstream crate is published, so they are dry-run-checked in dependency order
# by the release workflow, not here.
[group("release")]
publish-dry-run:
    cargo publish --dry-run -p deep-diff-forge-core --allow-dirty
    cargo publish --dry-run -p deep-diff-forge-learning --allow-dirty

# Cut a release: validate the tag matches the workspace version + no status-visible change
# (`git status --porcelain` prints nothing, before and after the gate; it does not see ignored
# files, or untracked files hidden by status.showUntrackedFiles=no), refuse
# unless the remotes `gitlab`, `gitea` and `forgejo` all exist and none points at github.com,
# run the release gate, then sign the tag and push it to those three remotes only (never
# `origin`, which is GitHub). From 0.8.4 nothing runs at GitHub: the tag starts no workflow;
# the release is built and checked on this machine with `just release-local-build` and
# `just release-local-verify` (scripts/release_local.sh; docs/RELEASE_AND_PUBLICATION.md).
# crates.io stays on hold by owner decision. The tag is made on the HEAD current when it is
# signed, which the recipe does not compare with the HEAD at the start: run it only in a
# dedicated, freshly cloned release worktree that nobody else uses, with no ignored or
# untracked files (`git status --porcelain --ignored --untracked-files=all` empty: without the
# last option, status.showUntrackedFiles=no hides both) before it starts and HEAD stable
# throughout (docs/RELEASE_AND_PUBLICATION.md, Tag And Push).
# Usage: just release-tag 0.8.4
[group("release")]
release-tag version:
    #!/usr/bin/env bash
    set -euo pipefail
    want="{{version}}"
    have="$(sed -n '/^\[workspace.package\]/,/^\[/p' Cargo.toml | sed -n 's/^version = "\(.*\)"/\1/p' | head -1)"
    if [ "${want}" != "${have}" ]; then
      echo "version mismatch: arg='${want}' but [workspace.package].version='${have}'" >&2
      exit 1
    fi
    # Status-visible changes only: a tracked change, or an untracked file Git does not ignore
    # (a stray build.rs, which Cargo compiles). An ignored file, or one hidden by
    # status.showUntrackedFiles=no, is not seen; the release procedure rules them out.
    if [ -n "$(git status --porcelain)" ]; then
      echo "working tree is not clean (git status --porcelain); commit or remove before tagging" >&2
      exit 1
    fi
    # Nothing at GitHub: every fetch and push URL of each target remote (insteadOf rewrites
    # applied) is checked, in any case, before the gate runs and before anything is signed.
    for remote in gitlab gitea forgejo; do
      if ! urls="$(git remote get-url --all "${remote}" 2>/dev/null)"; then
        echo "no remote named ${remote}; refusing to tag" >&2
        exit 1
      fi
      urls="${urls} $(git remote get-url --push --all "${remote}")"
      case "${urls,,}" in
        *github.com*)
          echo "remote ${remote} points at github.com; refusing to tag (nothing at GitHub)" >&2
          exit 1
          ;;
      esac
    done
    just gate-release
    # The gate must not have changed the tree (a rewritten Cargo.lock, a regenerated file).
    # This does not prove that the tag names the commit the gate ran on: the tag is made on the
    # HEAD current below, which is not compared with the HEAD at the start (the release
    # procedure keeps HEAD stable, and the owner's release script points this recipe's pushes at
    # local staging repositories and publishes the tag only if HEAD and the tag's commit are still
    # the commit it started from).
    if [ -n "$(git status --porcelain)" ]; then
      echo "the release gate changed the working tree (git status --porcelain); commit or discard before tagging" >&2
      exit 1
    fi
    # Signed with the caller's user.signingkey and verified against .github/allowed_signers;
    # an unverified tag is deleted here, before it can be pushed.
    scripts/sign_release_tag.sh "${want}"
    for remote in gitlab gitea forgejo; do
      git push "${remote}" "v${want}"
    done
    printf 'tagged + pushed v%s to gitlab, gitea and forgejo; next: just release-local-build v%s <outdir>.\n' "${want}" "${want}"

# Local release (from 0.8.4, nothing at GitHub): two fresh exports of <commit> built with
# release.yml's cargo command must give one SHA-256; writes the archive, SBOM, receipt and
# SHA256SUMS into <outdir>, which must not exist. Never signs. Run in the fedora-toolbox-44 toolbox.
[group("release")]
release-local-build commit outdir:
    scripts/release_local.sh build "{{commit}}" "{{outdir}}"

# Checks a local release: the tag's signature and its commit on gitlab/main, SHA256SUMS and its
# signature (namespace deep-diff-forge-release-sha256sums), then a fresh build of the tag.
# signers: .github/release_allowed_signers (the release key, for both namespaces).
[group("release")]
release-local-verify tag outdir signers:
    scripts/release_local.sh verify "{{tag}}" "{{outdir}}" "{{signers}}"

# Self-test of scripts/release_local.sh in a throwaway clone of HEAD, with keys made for the test.
[group("release")]
release-local-test:
    scripts/test_release_local.sh

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
doctor: status hooks-status contracts zellij-observe habitat-observe

# Install the versioned pre-push hook (gitleaks over the pushed commits) for every worktree.
[group("diagnostics")]
install-hooks:
    install -m 0755 scripts/hooks/pre-push "$(git rev-parse --git-common-dir)/hooks/pre-push"
    @printf 'pre_push_hook=installed\n'

# Report whether the installed pre-push hook is the versioned one.
[group("diagnostics")]
hooks-status:
    #!/usr/bin/env bash
    set -euo pipefail
    hook="$(git rev-parse --git-common-dir)/hooks/pre-push"
    if [ ! -e "${hook}" ]; then
      echo "pre_push_hook=missing (run: just install-hooks)"
    elif cmp -s "${hook}" scripts/hooks/pre-push; then
      echo "pre_push_hook=installed"
    else
      echo "pre_push_hook=differs from scripts/hooks/pre-push (run: just install-hooks)"
    fi

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

# Generate a dependency SBOM from Cargo.lock/cargo metadata without requiring
# host-local CycloneDX tooling. Emits root `sbom.spdx.json` for release upload.
[group("security")]
security-sbom:
    python3 scripts/security/generate_spdx_sbom.py

# List security-critical mutants and optionally run a bounded shard:
#   just security-mutation list
#   just security-mutation run
[group("security")]
security-mutation mode="list":
    bash scripts/security/mutation_gate.sh {{mode}}

# Probe that learning receipts/status do not expose raw source, patch text, or paths.
[group("security")]
security-privacy:
    python3 scripts/security/privacy_probe.py

# Run the UDS daemon hostile-client soak. Override duration with e.g.
#   DDF_SOAK_SECONDS=1800 just security-soak
[group("security")]
security-soak:
    python3 scripts/security/daemon_soak.py

# Local S9 evidence bundle: release gate + SBOM + mutation inventory + privacy probe + soak.
# This is still self-attested; external verifier/red-team receipt is required for promotion.
[group("security")]
security-s9-local: gate-release security-sbom security-mutation security-privacy security-soak
