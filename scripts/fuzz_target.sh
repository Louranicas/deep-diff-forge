#!/usr/bin/env bash
# Run one fuzz target from its committed seeds, or list every target. The one place that knows
# each target's seeds and input cap, so the CI smoke (10 s, ci.yml) and the weekly campaign
# (campaign.yml) cannot drift apart.
#
#   scripts/fuzz_target.sh --list               every [[bin]] in fuzz/Cargo.toml, one per line
#   scripts/fuzz_target.sh <target> <seconds>   seed a fresh corpus, then fuzz for <seconds>
#
# Seeds: the patch fixtures for the patch parsers, review-policy/fuzz-seeds for the council
# report, fuzz/seeds/<target> for the rest (stdio seeds are a cap byte, 255 = the server's real
# cap, then stdio lines; see fuzz/fuzz_targets/stdio_invariants.rs). The corpus is a copy, so the
# committed seeds stay unchanged. A crash is kept under fuzz/artifacts/<target>/.
# Exit status: cargo fuzz's (non-zero on a crash, leak or timeout); 2 on a usage error or a
# target this script has no seeds for.
set -euo pipefail
cd "$(dirname "$0")/.."

targets() {
  grep -A1 '^\[\[bin\]\]' fuzz/Cargo.toml | sed -n 's/^name = "\(.*\)"/\1/p'
}

if [ "${1:-}" = "--list" ] && [ $# -eq 1 ]; then
  targets
  exit 0
fi
if [ $# -ne 2 ] || ! [[ "$2" =~ ^[1-9][0-9]*$ ]]; then
  echo "usage: $0 --list | <target> <seconds>" >&2
  exit 2
fi
target="$1"
seconds="$2"
if ! targets | grep -qx -- "$target"; then
  echo "unknown fuzz target: $target (see $0 --list)" >&2
  exit 2
fi

corpus="${DDF_FUZZ_CORPUS_ROOT:-${RUNNER_TEMP:-/tmp}}/ddf-$target-corpus"
mkdir -p "$corpus"
case "$target" in
  patch_parser | review_json)
    cp fixtures/patch/*.patch "$corpus/"
    max_len=65536
    ;;
  council_report)
    cp review-policy/fuzz-seeds/council_report.json "$corpus/seed.json"
    max_len=4096
    ;;
  *)
    if [ ! -d "fuzz/seeds/$target" ]; then
      echo "no seeds for fuzz target $target: add fuzz/seeds/$target" >&2
      exit 2
    fi
    cp "fuzz/seeds/$target"/* "$corpus/"
    max_len=65536
    ;;
esac

toolchain="${DDF_FUZZ_TOOLCHAIN:-nightly-2026-10-25}"
exec cargo "+$toolchain" fuzz run "$target" "$corpus" -- -max_total_time="$seconds" -max_len="$max_len"
