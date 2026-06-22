#!/usr/bin/env bash
# Deterministic local mutation smoke gate for security-critical files.
# Full mutation campaigns are expensive; this gate proves cargo-mutants is wired,
# enumerates critical mutants as JSON, and runs a bounded shard when requested.

set -u
cd "$(dirname "$0")/../.." || exit 2
mkdir -p reports/security
stamp=$(date -u +%Y-%m-%dT%H-%M-%SZ)
receipt="reports/security/mutation-gate-${stamp}.txt"
mode="${1:-list}"
script_status=0

{
  echo "# Deep-Diff-Forge mutation gate"
  echo "ts=${stamp}"
  echo "mode=${mode}"
  echo "cargo_mutants=$(cargo mutants --version 2>/dev/null || echo unknown)"
  echo
  echo "## critical file inventory"
  cargo mutants --workspace --list-files \
    -f 'crates/deep-diff-forge-patch/src/parser.rs' \
    -f 'crates/deep-diff-forge-daemon/src/protocol.rs' \
    -f 'crates/deep-diff-forge-daemon/src/serve.rs' \
    -f 'crates/deep-diff-forge-agent/src/lib.rs' \
    -f 'crates/deep-diff-forge-learning/src/receipt.rs' \
    --no-times
  echo
  echo "## critical mutant list"
  cargo mutants --workspace --list --json --line-col=true \
    -f 'crates/deep-diff-forge-patch/src/parser.rs' \
    -f 'crates/deep-diff-forge-daemon/src/protocol.rs' \
    -f 'crates/deep-diff-forge-daemon/src/serve.rs' \
    -f 'crates/deep-diff-forge-agent/src/lib.rs' \
    -f 'crates/deep-diff-forge-learning/src/receipt.rs' \
    --no-times
  list_status=$?
  echo "list_exit=${list_status}"
  if [[ "${list_status}" -ne 0 ]]; then
    script_status="${list_status}"
  elif [[ "${mode}" == "run" ]]; then
    echo
    echo "## bounded shard run"
    CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-./target}" cargo mutants --workspace --test-workspace=true \
      --shard 1/20 --timeout 180 --minimum-test-timeout 20 --jobs 2 \
      -f 'crates/deep-diff-forge-patch/src/parser.rs' \
      -f 'crates/deep-diff-forge-daemon/src/protocol.rs' \
      -f 'crates/deep-diff-forge-daemon/src/serve.rs' \
      -f 'crates/deep-diff-forge-agent/src/lib.rs' \
      -f 'crates/deep-diff-forge-learning/src/receipt.rs' \
      --no-times
    run_status=$?
    echo "run_exit=${run_status}"
    script_status="${run_status}"
  fi
} > "${receipt}" 2>&1
block_status=$?
if [[ "${block_status}" -ne 0 && "${script_status}" -eq 0 ]]; then
  script_status="${block_status}"
fi
status="${script_status}"
echo "receipt=${receipt} exit=${status}"
exit ${status}
