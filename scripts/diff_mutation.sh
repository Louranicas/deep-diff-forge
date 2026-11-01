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
mkdir -p "$out"
out="$(cd "$out" && pwd)"
# Pathspecs resolve against the current directory: from a subdirectory `crates/` matches nothing
# and the gate would pass with no mutants tested. Run at the root, where cargo mutants runs too.
cd "$(git rev-parse --show-toplevel)"
base="$(git merge-base "$base_ref" HEAD)"
# Pin the diff format: a local diff.noprefix, diff.mnemonicPrefix, external diff or colour would
# change the `+++ b/` lines cargo mutants reads. core.quotePath=false writes a non-ASCII path as
# itself; quoted (`+++ "b/..."`), cargo mutants refuses the whole diff.
git -c core.quotePath=false diff --no-ext-diff --no-color --src-prefix=a/ --dst-prefix=b/ \
  "$base" HEAD -- crates/ > "$out/pr.diff"
# Whether any Rust changed comes from the NUL-separated names, not from the `+++` header text: git
# appends a TAB to a header path holding a space, and quotes one holding a quote, backslash or
# control byte, so a header match would miss those files and pass with no mutants tested.
git diff --no-ext-diff --name-only -z --diff-filter=d "$base" HEAD -- crates/ > "$out/changed"
rust_changed=false
while IFS= read -r -d '' path; do
  case "$path" in *.rs) rust_changed=true ;; esac
done < "$out/changed"
if [ "$rust_changed" = false ]; then
  echo "diff mutation: no Rust changes under crates/ since ${base}; no mutants to test"
  exit 0
fi
cargo mutants --no-config --in-diff "$out/pr.diff" --test-workspace true --timeout 300 \
  --shard "$shard" --output "$out"
