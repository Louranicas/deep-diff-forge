#!/usr/bin/env bash
# Anchor B-01: diff-scoped mutation for a pull request. Mutates only the Rust lines the change adds
# or edits since its merge base, and fails on any missed mutant or timeout (cargo-mutants exits 2
# and 3). A change with no Rust lines under crates/ has no mutants and passes, saying so.
# Design/status: docs/TESTING_GOLD_STANDARD.md#coverage-oracle-mutation-not-counts
# Usage: scripts/diff_mutation.sh <base-ref> [<shard k/n>]
set -euo pipefail
base_ref="${1:?usage: scripts/diff_mutation.sh <base-ref> [<shard k/n>]}"
shard="${2:-0/1}"
out="${DDF_MUTATION_OUT:-${TMPDIR:-/tmp}/ddf-diff-mutation}"
base="$(git merge-base "$base_ref" HEAD)"
mkdir -p "$out"
git diff "$base" HEAD -- crates/ > "$out/pr.diff"
if ! grep -q '^+++ b/crates/.*\.rs$' "$out/pr.diff"; then
  echo "diff mutation: no Rust changes under crates/ since ${base}; no mutants to test"
  exit 0
fi
cargo mutants --no-config --in-diff "$out/pr.diff" --test-workspace true --timeout 300 \
  --shard "$shard" --output "$out"
