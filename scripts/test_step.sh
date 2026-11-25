#!/usr/bin/env bash
# The gate's test step: the step `test` of ci.yml's job `gate` and of scripts/ci_local.sh, which
# both run this script (crates/deep-diff-forge-cli/tests/ci_replica.rs holds the two to it, and
# repo_scripts.rs holds what it runs).
#
# Two passes, each with --no-fail-fast, so that one failing test binary does not hide the
# binaries after it:
#   1. the workspace's tests, less the tests crates/deep-diff-forge-cli/tests/LOAD_SENSITIVE
#      names (`--exact --skip <name>` for each);
#   2. those tests alone, one at a time (`--test-threads=1`), in every test target (no doc tests:
#      the list names test functions).
# The list is read here and nowhere else: one name a line, `#` starts a comment, blank lines are
# skipped (as `the_load_sensitive_list_names_tests_that_exist` reads it). Every listed test runs
# in every gate: the step fails when the list is empty or names a test twice, and when the second
# pass did not run each listed test exactly once. Exit status: 0 only when both passes passed.
# Usage: scripts/test_step.sh   (in the checkout; runs on bash 3.2)
set -u
list=crates/deep-diff-forge-cli/tests/LOAD_SENSITIVE
[ -f "$list" ] || { echo "test_step.sh: no $list here"; exit 1; }
names=()
while IFS= read -r line || [ -n "$line" ]; do
  name=$(printf '%s' "${line%%#*}" | sed 's/^[[:space:]]*//; s/[[:space:]]*$//')
  [ -n "$name" ] || continue
  for seen in ${names[@]+"${names[@]}"}; do
    [ "$seen" = "$name" ] && { echo "test_step.sh: $list names $name twice"; exit 1; }
  done
  names+=("$name")
done <"$list"
[ "${#names[@]}" -gt 0 ] || { echo "test_step.sh: $list names no test"; exit 1; }
skips=()
for name in "${names[@]}"; do skips+=(--skip "$name"); done

echo "test step, pass 1: the workspace's tests less the ${#names[@]} listed in $list"
cargo test --workspace --locked --no-fail-fast -- --exact "${skips[@]}"
first=$?

echo "test step, pass 2: the ${#names[@]} tests listed in $list, one at a time"
out=$(cargo test --workspace --locked --no-fail-fast --tests -- --exact --test-threads=1 "${names[@]}" 2>&1)
second=$?
printf '%s\n' "$out"
missing=
for name in "${names[@]}"; do
  ran=$(printf '%s\n' "$out" | grep -c -E "^test $name \.\.\. (ok|FAILED)$")
  [ "$ran" -eq 1 ] || missing="$missing $name ($ran runs)"
done
if [ -n "$missing" ]; then
  echo "test step: pass 2 did not run each listed test once:$missing"
  second=1
fi
echo "test step: pass 1 rc=$first, pass 2 rc=$second"
[ "$first" -eq 0 ] && [ "$second" -eq 0 ]
