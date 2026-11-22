#!/usr/bin/env bash
# Run one fuzz target from its committed seeds, or list every target. The one place that knows
# each target's seeds and input cap, so the CI smoke (10 s, ci.yml) and the weekly campaign
# (campaign.yml) cannot drift apart.
#
#   scripts/fuzz_target.sh --list               every [[bin]] in fuzz/Cargo.toml, one per line
#   scripts/fuzz_target.sh <target> <seconds>   seed a fresh private corpus, then fuzz for <seconds>
#
# Seeds: the patch fixtures for the patch parsers, review-policy/fuzz-seeds for the council
# report, the CLI tests' forge-warden verdict fixtures for verdict_reader, fuzz/seeds/<target> for
# the rest (stdio seeds are a cap byte, 255 = the server's real cap, then stdio lines; see
# fuzz/fuzz_targets/stdio_invariants.rs). The corpus is a copy, so the
# committed seeds stay unchanged. A crash is kept under fuzz/artifacts/<target>/.
# Exit status: cargo fuzz's (non-zero on a crash, leak or timeout); 2 on a usage error, a target
# this script has no seeds for, or no corpus root (DDF_FUZZ_CORPUS_ROOT, or RUNNER_TEMP in CI).
set -euo pipefail
cd "$(dirname "$0")/.."

# A TOML parse, not a line match: a [[bin]] laid out another way (path before name, `name="x"`,
# single quotes, a comment) would otherwise drop out of the list, and so out of CI and the campaign.
targets() {
  # tomllib is Python 3.11+; tomli is its backport. Neither: a loud failure, never an empty list.
  python3 -I -c 'import sys
try:
    import tomllib
except ModuleNotFoundError:
    try:
        import tomli as tomllib
    except ModuleNotFoundError:
        sys.exit("fuzz_target.sh needs Python 3.11+ (tomllib) or the tomli package")
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
# The root (DDF_FUZZ_CORPUS_ROOT, else CI's RUNNER_TEMP) is made if missing, since mktemp makes
# only the last component; only the corpus inside it needs to be new and private. With neither set
# the run is refused rather than put in /tmp, which on a workstation may be a small tmpfs (an inode
# limit, and lost at power-off).
root="${DDF_FUZZ_CORPUS_ROOT:-${RUNNER_TEMP:-}}"
if [ -z "$root" ]; then
  echo "set DDF_FUZZ_CORPUS_ROOT to a directory on disk for the fuzz corpus (not /tmp)" >&2
  exit 2
fi
mkdir -p -- "$root"
corpus="$(mktemp -d "$root/ddf-$target-corpus.XXXXXX")"
# A target that writes files (journal_recover) does so in a private scratch directory beside the
# corpus, never in it (libFuzzer reads the corpus recursively); it is removed afterwards too.
scratch="$(mktemp -d "$root/ddf-$target-scratch.XXXXXX")"
export DDF_FUZZ_SCRATCH="$scratch"
trap 'rm -rf "$corpus" "$scratch"' EXIT
case "$target" in
  patch_parser | review_json)
    # Every committed patch, the fuzz regressions and Git oracle findings included. One `cp` per
    # file: BSD cp (macOS) has no `-t` (macOS CI, PR #100).
    find fixtures/patch -type f -name '*.patch' -exec cp {} "$corpus/" ';'
    max_len=65536
    ;;
  council_report)
    cp review-policy/fuzz-seeds/council_report.json "$corpus/seed.json"
    max_len=4096
    ;;
  git_raw_stream)
    # Three `git diff --raw -z --patch` streams: a change, an addition and a rename.
    z=0000000000000000000000000000000000000000
    a=1111111111111111111111111111111111111111
    b=2222222222222222222222222222222222222222
    printf ':100644 100644 %s %s M\0f\0\0diff --git a/f b/f\nindex %s..%s 100644\n--- a/f\n+++ b/f\n@@ -1 +1 @@\n-a\n+b\n' \
      "$a" "$b" "$a" "$b" >"$corpus/modified"
    printf ':000000 100644 %s %s A\0n\0\0diff --git a/n b/n\nnew file mode 100644\nindex %s..%s\n--- /dev/null\n+++ b/n\n@@ -0,0 +1 @@\n+a\n' \
      "$z" "$a" "$z" "$a" >"$corpus/added"
    printf ':100644 100644 %s %s R100\0o\0n\0\0diff --git a/o b/n\nsimilarity index 100%%\nrename from o\nrename to n\n' \
      "$a" "$a" >"$corpus/renamed"
    max_len=65536
    ;;
  journal_recover)
    # Damages to the journal of the run the target prepares (see its source): none, a record
    # torn after a cut, an emptied checkpoint, a temporary file and a stray line.
    printf '\000\377\377' >"$corpus/whole"
    printf '\002\000\100{"schema":"deep-diff-forge.decision.v1"' >"$corpus/torn"
    printf '\001\377\377' >"$corpus/empty-checkpoint"
    printf '\004\377\377{}\n' >"$corpus/temporary-and-line"
    max_len=65536
    ;;
  verdict_reader)
    # forge-warden's own verdict documents: v2 (its 0.2.0) and v3 (its main branch).
    for f in crates/deep-diff-forge-cli/tests/fixtures/forge-warden-*/*.json; do
      cp "$f" "$corpus/${f//\//_}"
    done
    max_len=65536
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
