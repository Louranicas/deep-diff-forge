#!/usr/bin/env bash
# The contract matrix between deep-diff-forge and forge-warden, the judge its stop hook calls
# (F4.a, 0.10.0). Each row runs in a scratch repository of its own with its own home.
#
# Stand-in rows (always): the stop hook against scripts/contract-matrix/judge.py answering as
# forge-warden does: verdict v2 and v3 PASS, HOLD and FAIL; a FAIL whose document cannot be read
# (exit 7), which holds the stop once and then stands; exit 4; a judge that hangs; a hold for
# claims the judge cannot verify (a policy with `[claims] require_verified`), at the stop and in
# `agent doctor --gate` (link claims_verified); the patch the judge is given equals the bytes of
# forge-warden's recorded scripts/make_patch.sh. A known-bad control row (a FAIL expected to be
# let through) must come out red, or the matrix cannot see what it is for.
#
# Real rows (only with FW_BIN=<path to a forge-warden binary>): the binary is named by path and
# accepted only if its SHA-256 is a `forge-warden` row of scripts/contract-matrix/binaries.tsv,
# whose schema it must print; never by version. Rows: the stop hook reads its verdict; every
# label `deep-diff-forge schema signals` lists is in its vocabulary; it reads this build's
# canonical rank document (with scorer_sha256) without a rank refusal; an empty patch through
# its authority check with this build as ranker ("the ranker exited 4", no verdict) and with the
# 0.9.0 ranker (DDF_RANKER_090=<path>, a `ranker-0.9.0` row of binaries.tsv: a holding verdict).
#
# Usage: scripts/contract_matrix.sh [--require-real] <deep-diff-forge binary>
# Without FW_BIN the real rows are skipped by name and do not fail the run; with --require-real
# each of them is red. Prints PASS/FAIL/SKIP per row and `rows red: N`.
# KEEP_MATRIX=1 keeps the scratch directory (its path is printed) to read a red row.
# Exit status: 0 if no row is red, 1 if one is, 2 on a usage error.
set -uo pipefail
require_real=
[ "${1:-}" = --require-real ] && { require_real=1; shift; }
[ "$#" -eq 1 ] && [ -x "$1" ] || { sed -n '2,/^# Exit status/{s/^# \{0,1\}//;p}' "$0" >&2; exit 2; }
ddf="$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
py="$(command -v python3)"
# forge-warden's own patch maker, recorded byte for byte with its SHA-256 (S3.17a.h).
maker="$here/../crates/deep-diff-forge-cli/tests/fixtures/forge-warden-main-fbd621e0/make_patch.sh"
scratch="$(mktemp -d "${TMPDIR:-/var/tmp}/contract-matrix.XXXXXX")" || exit 2
# Every stand-in judge records its pid; one still alive at the end is stopped by that pid.
cleanup() {
  local f p
  for f in "$scratch"/*/pid; do
    [ -e "$f" ] || continue
    p="$(cat "$f")"
    if tr '\0' ' ' 2>/dev/null <"/proc/$p/cmdline" | grep -qF "$(dirname "$f")"; then kill -KILL "$p"; fi
  done
  [ -n "${KEEP_MATRIX:-}" ] && echo "kept $scratch" || rm -rf -- "$scratch"
}
trap cleanup EXIT
trap 'exit 143' INT TERM HUP
red=0
pass() { echo "PASS $1${2:+ ($2)}"; }
fail() { echo "FAIL $1: $2"; red=$((red + 1)); }
git_() { env -i PATH="$PATH" GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1 git -c user.name=ddf \
  -c user.email=ddf@example.invalid -c init.defaultBranch=main -c commit.gpgSign=false "$@"; }
sum() { sha256sum "$1" | cut -d' ' -f1; }

# row <id> <answer JSON>: a repository with a source file and a policy at its one commit, a
# change that silences a lint, and the stand-in judge giving <answer>. Sets $d.
row() {
  d="$scratch/$1"
  mkdir -p "$d/home" "$d/repo"
  git_ -C "$d/repo" init --quiet
  printf 'fn a() {}\nfn b() {}\n' >"$d/repo/a.rs"
  printf '# the policy at the base commit\n[claims]\nrequire_verified = true\n' >"$d/repo/warden.toml"
  git_ -C "$d/repo" add --all && git_ -C "$d/repo" commit --quiet -m base
  printf 'fn a() {}\n#[allow(dead_code)]\nfn b() {}\n' >"$d/repo/a.rs"
  printf '%s\n' "$2" >"$d/answer.json"
  printf '#!/bin/sh\nexec %q -P -E %q %q "$@"\n' "$py" "$here/contract-matrix/judge.py" "$d" >"$d/judge"
  chmod 755 "$d/judge"
}

# The hook as a client runs it: its own home, no configuration of the user or the system.
run_ddf() {
  (cd "$d/repo" && env -i PATH="$PATH" HOME="$d/home" TMPDIR="$d" LANG=C.UTF-8 \
    GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1 "$ddf" "$@")
}

# stop <session> <active> [<judge>]: run the stop hook; sets $decision (block|allow), $said, $why.
stop() {
  local rc
  printf '{"hook_event_name":"Stop","session_id":"%s","cwd":"%s","stop_hook_active":%s,"tool_name":"Bash"}' \
    "$1" "$d/repo" "$2" | run_ddf agent hook stop --client claude --judge "${3:-$d/judge}" >"$d/out" 2>"$d/err"
  rc=$?
  { read -r decision; said="$(cat)"; } < <("$py" -P -E -c 'import json, sys
t = open(sys.argv[1]).read().strip()
d = json.loads(t) if t else {}
print("block" if d.get("decision") == "block" else "allow")
print(d.get("reason") or d.get("systemMessage") or "")' "$d/out" 2>&1)
  why="exit $rc, $decision: $(printf '%s' "$said" | head -c 300) $(head -c 200 "$d/err")"
  [ "$rc" -eq 0 ] && [ ! -s "$d/err" ]
}

# expect <block|allow> <text it must hold> [<text it must not hold>...] (after stop)
expect() {
  local want="$1" has="$2" not
  shift 2
  [ "$decision" = "$want" ] && [[ "$said" == *"$has"* ]] || return 1
  for not in "$@"; do [[ "$said" != *"$not"* ]] || return 1; done
}

judged() { if stop s1 false && expect "$@"; then pass "$id"; else fail "$id" "$why"; fi; }

. "$here/contract-matrix/stand_in_rows.sh"

# --- real rows ---
real=(fw-recorded fw-judges-the-stop fw-knows-every-label fw-reads-the-rank
  fw-empty-patch-this-ranker fw-empty-patch-ranker-0.9.0)
# recorded <role> <path>: the schema binaries.tsv records for the binary's SHA-256 in <role>.
recorded() { awk -F'\t' -v r="$1" -v s="$(sum "$2")" '$1 == r && $2 == s { print $3; exit }' "$here/contract-matrix/binaries.tsv"; }
if [ -z "${FW_BIN:-}" ]; then
  for id in "${real[@]}"; do
    if [ -n "$require_real" ]; then fail "$id" "needs a real forge-warden (FW_BIN) and none was given"
    else echo "SKIP $id (no FW_BIN)"; fi
  done
  [ -n "$require_real" ] || echo "skipped: ${#real[@]} rows that need a real forge-warden (no FW_BIN)"
else
  . "$here/contract-matrix/real_rows.sh"
fi

echo "rows red: $red"
[ "$red" -eq 0 ]
