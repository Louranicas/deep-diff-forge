#!/usr/bin/env bash
# Run one fuzz target from its committed seeds, or list every target. The one place that knows
# each target's seeds and input cap, so the CI smoke (10 s, ci.yml) and the weekly campaign
# (campaign.yml) cannot drift apart.
#
#   scripts/fuzz_target.sh --list               every [[bin]] in fuzz/Cargo.toml, one per line
#   scripts/fuzz_target.sh <target> <seconds>   seed a fresh private corpus, then fuzz for <seconds>
#
# Seeds: the patch fixtures for the patch parsers, review-policy/fuzz-seeds for the council
# report, fuzz/seeds/<target> for the rest (stdio seeds are a cap byte, 255 = the server's real
# cap, then stdio lines; see fuzz/fuzz_targets/stdio_invariants.rs). The corpus is a copy, so the
# committed seeds stay unchanged. A crash is kept under fuzz/artifacts/<target>/.
# Exit status: cargo fuzz's (non-zero on a crash, leak or timeout); 2 on a usage error or a
# target this script has no seeds for.
set -euo pipefail
cd "$(dirname "$0")/.."

# A TOML parse, not a line match: a [[bin]] laid out another way (path before name, `name="x"`,
# single quotes, a comment) would otherwise drop out of the list, and so out of CI and the campaign.
targets() {
  python3 -I -c 'import sys, tomllib
for b in tomllib.load(open(sys.argv[1], "rb"))["bin"]:
    print(b["name"])' fuzz/Cargo.toml
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

# A new private directory (mode 0700, unpredictable name) each run: a fixed name under a shared
# /tmp could be planted by another user, with symlinks the seed copy would write through, or keep
# inputs from an earlier run. It is removed afterwards; a crash is kept under fuzz/artifacts/.
# The root (DDF_FUZZ_CORPUS_ROOT, else RUNNER_TEMP, else /tmp) is made if missing, since mktemp
# makes only the last component; only the corpus inside it needs to be new and private.
root="${DDF_FUZZ_CORPUS_ROOT:-${RUNNER_TEMP:-/tmp}}"
mkdir -p -- "$root"
corpus="$(mktemp -d "$root/ddf-$target-corpus.XXXXXX")"
trap 'rm -rf "$corpus"' EXIT
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
cargo "+$toolchain" fuzz run "$target" "$corpus" -- -max_total_time="$seconds" -max_len="$max_len"
