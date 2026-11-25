#!/usr/bin/env bash
# The local fuzz campaign, a release step (C6): every target `scripts/fuzz_target.sh --list` names,
# one after the other, each for <seconds> from its committed seeds (fuzz_target.sh knows them), on
# corpora under DDF_FUZZ_CORPUS_ROOT, which must be set: a directory on disk, never the tmpfs /tmp
# (an inode limit, and lost at power-off). It stops red at the first target that fails (a crash, a
# leak, a timeout or a build error); cargo fuzz keeps that input under fuzz/artifacts/<target>/.
#
#   scripts/campaign_local.sh [<seconds> [<target>...]]   default 300 s each, every listed target
#
# The release default, 300 s, is 90 minutes of fuzzing for 18 targets, plus each target's build.
# Receipt: <root>/campaign-<UTC time>.<random>/RECEIPT, its path printed last, with the commit, the
# toolchain, the seconds, one line per target run and the result; each target's output is beside it
# in <target>.log. Exit status: 0 when every target ran clean, the failing target's status (non-zero)
# when one did not, 2 on a usage error.
set -uo pipefail
cd "$(dirname "$0")/.."

seconds="${1:-300}"
if ! [[ "$seconds" =~ ^[1-9][0-9]*$ ]]; then
  echo "usage: $0 [<seconds> [<target>...]]" >&2
  exit 2
fi
[ $# -gt 0 ] && shift
root="${DDF_FUZZ_CORPUS_ROOT:-}"
if [ -z "$root" ]; then
  echo "campaign_local.sh: set DDF_FUZZ_CORPUS_ROOT to a directory on disk (not the tmpfs /tmp)" >&2
  exit 2
fi
# Written for bash 3.2 as well (macOS's /bin/bash): no `mapfile`, and the count is kept apart,
# since an array with no element is unbound under `set -u` before bash 4.4.
if [ $# -gt 0 ]; then
  targets=("$@")
  n=$#
else
  list="$(bash scripts/fuzz_target.sh --list)" || exit 2
  targets=()
  n=0
  while IFS= read -r t; do
    if [ -n "$t" ]; then targets+=("$t"); n=$((n + 1)); fi
  done <<<"$list"
fi
if [ "$n" -eq 0 ] || [ -z "${targets[0]}" ]; then
  echo "campaign_local.sh: no fuzz targets" >&2
  exit 2
fi

mkdir -p -- "$root" || exit 2
dir="$(mktemp -d "$root/campaign-$(date -u +%Y%m%dT%H%M%SZ).XXXXXX")" || exit 2
receipt="$dir/RECEIPT"
toolchain="${DDF_FUZZ_TOOLCHAIN:-nightly-2026-10-25}"
commit="$(git rev-parse HEAD)"
[ -z "$(git status --porcelain --untracked-files=no)" ] || commit="$commit (with uncommitted changes)"
{
  echo "deep-diff-forge fuzz campaign (scripts/campaign_local.sh)"
  echo "commit: $commit"
  echo "toolchain: $toolchain: $(cargo "+$toolchain" --version 2>&1 | head -1)"
  echo "seconds per target: $seconds"
  echo "targets: $n"
  echo "started: $(date -u +%FT%TZ)"
} >"$receipt"

for target in "${targets[@]}"; do
  start=$SECONDS
  bash scripts/fuzz_target.sh "$target" "$seconds" >"$dir/$target.log" 2>&1 </dev/null
  rc=$?
  echo "run: $target rc=$rc $((SECONDS - start)) s" >>"$receipt"
  if [ "$rc" -ne 0 ]; then
    echo "result: RED at $target (rc $rc); its input is kept under fuzz/artifacts/$target/" >>"$receipt"
    echo "finished: $(date -u +%FT%TZ)" >>"$receipt"
    echo "campaign_local.sh: fuzz target $target failed (rc $rc): log $dir/$target.log" >&2
    echo "receipt: $receipt"
    exit "$rc"
  fi
done
echo "result: GREEN, $n targets clean for $seconds s each" >>"$receipt"
echo "finished: $(date -u +%FT%TZ)" >>"$receipt"
echo "receipt: $receipt"
