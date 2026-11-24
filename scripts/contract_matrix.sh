#!/usr/bin/env bash
# The contract matrix between deep-diff-forge and forge-warden, the judge its stop hook calls
# (F4.a, 0.10.0). Each row runs in a scratch repository of its own with its own home.
#
# Stand-in rows (always; scripts/contract-matrix/stand_in_rows.sh): the stop hook against
# scripts/contract-matrix/judge.py answering as forge-warden does (verdict v2 and v3 PASS, HOLD and
# FAIL; an unread FAIL; exit 4; a hang; claims a `require_verified` policy cannot verify; the
# judged patch is make_patch.sh's bytes; agent verify-branch over each outcome; a first stop;
# the stateless stop, one row per way; a known-bad control row that must come out red), and this
# build alone (the stray-byte file in a Git mode; diagnostics paged on rpc and MCP).
# State rows (scripts/contract-matrix/state_rows.sh; with DDF_RANKER_090 and DDF_RANKER_0100, the
# published 0.9.0 and 0.10.0): each and this build on a session the other wrote, both ways.
# Real rows (scripts/contract-matrix/real_rows.sh; FW_BIN=<the gate's forge-warden> and
# FW_BIN_OLDER=<path>:<path>): once per forge-warden, each named by its SHA-256 in
# scripts/contract-matrix/binaries.tsv (never by version), the id carrying its name.
# docs/reference/quality-gates.md lists every row, what it needs and its outcome per judge.
#
# Usage: scripts/contract_matrix.sh [--require-real] <deep-diff-forge binary>
# Without FW_BIN the real rows are skipped by name and do not fail the run; with --require-real
# each of them is red, and so is each forge-warden binaries.tsv records that is not given.
# Prints PASS/FAIL/SKIP per row and `rows red: N`.
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
# recorded <role> <path> [column]: the schema (or the column named) binaries.tsv records for the
# binary's SHA-256 in <role>.
recorded() { awk -F'\t' -v r="$1" -v s="$(sum "$2")" -v c="${3:-3}" '$1 == r && $2 == s { print $c; exit }' "$here/contract-matrix/binaries.tsv"; }

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

# The hook as a client runs it: its own home, no configuration of the user or the system
# (NOHOME=1: no HOME at all).
NOHOME=
run_ddf() {
  if [ -n "$NOHOME" ]; then
    (cd "$d/repo" && env -i PATH="$PATH" TMPDIR="$d" LANG=C.UTF-8 \
      GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1 "$ddf" "$@")
  else
    (cd "$d/repo" && env -i PATH="$PATH" HOME="$d/home" TMPDIR="$d" LANG=C.UTF-8 \
      GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1 "$ddf" "$@")
  fi
}

# stop <session> <active> [<judge>]: run the stop hook; sets $decision (block|allow), $said, $why.
# stop <session> <stop_hook_active> [judge]: a stop hook (with the arguments of stop_args), its
# decision in $decision, its words in $said, a summary in $why; true if it exited 0 silently.
stop_args=()
stop() {
  local rc
  printf '{"hook_event_name":"Stop","session_id":"%s","cwd":"%s","stop_hook_active":%s,"tool_name":"Bash"}' \
    "$1" "$d/repo" "$2" | run_ddf agent hook stop --client claude --judge "${3:-$d/judge}" "${stop_args[@]}" >"$d/out" 2>"$d/err"
  rc=$?
  { read -r decision; said="$(cat)"; } < <("$py" -P -E -c 'import json, sys
t = open(sys.argv[1]).read().strip()
d = json.loads(t) if t else {}
print("block" if d.get("decision") == "block" else "allow")
print(d.get("reason") or d.get("systemMessage") or "")' "$d/out" 2>&1)
  why="exit $rc, $decision: $(printf '%s' "$said" | head -c 300) $(head -c 200 "$d/err")"
  [ "$rc" -eq 0 ] && [ ! -s "$d/err" ]
}

# hookrun <event> <hook_event_name> <session> [judge]: another hook of the session (its output in
# $d/<event>.out); true if it exited 0.
hookrun() {
  printf '{"hook_event_name":"%s","session_id":"%s","cwd":"%s","stop_hook_active":false,"tool_name":"Bash"}' \
    "$2" "$3" "$d/repo" | run_ddf agent hook "$1" --client claude ${4:+--judge "$4"} >"$d/$1.out" 2>"$d/$1.err"
}
# register <judge>: the row's home registers this build's Stop hook with <judge>, in the form
# `agent setup --hooks` prints (for `agent check`).
register() {
  mkdir -p "$d/home/.claude"
  "$py" -P -E -c 'import json, shlex, sys
stop = " ".join(shlex.quote(a) for a in [sys.argv[1], "agent", "hook", "stop", "--client", "claude", "--judge", sys.argv[2]])
print(json.dumps({"hooks": {"Stop": [{"hooks": [{"type": "command", "command": stop}]}]}}))' "$ddf" "$1" >"$d/home/.claude/settings.json"
}
# published <role> <variable> <id>: true if the variable names a binary binaries.tsv records in
# that role (a published deep-diff-forge); else the row is skipped (red with --require-real).
published() {
  local role="$1" var="$2" id="$3"
  if [ -z "${!var:-}" ]; then
    if [ -n "$require_real" ]; then fail "$id" "needs $var"; else echo "SKIP $id (no $var)"; fi
    return 1
  elif [ -z "$(recorded "$role" "${!var}")" ]; then
    fail "$id" "$var ${!var} is not a $role row of binaries.tsv"
    return 1
  fi
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
. "$here/contract-matrix/state_rows.sh"

# --- real rows ---
# The real rows run once for each forge-warden given: FW_BIN (the gate's judge) and each path of
# FW_BIN_OLDER (colon-separated), each id carrying the binary's name from binaries.tsv.
real=(fw-recorded fw-judges-the-stop fw-a-commit-passes-in-bound-mode fw-unknown-labels-hold fw-reads-the-rank
  fw-empty-patch-this-ranker fw-empty-patch-ranker-0.9.0 fw-empty-patch-ranker-0.10.0
  fw-an-older-ranker-is-admitted-0.9.0 fw-an-older-ranker-is-admitted-0.10.0
  fw-the-frontier-judges-a-commit-left-behind fw-agent-check-equals-the-stop)
judges=()
[ -n "${FW_BIN:-}" ] && judges+=("$FW_BIN")
IFS=: read -ra older <<<"${FW_BIN_OLDER:-}"
for j in "${older[@]}"; do [ -n "$j" ] && judges+=("$j"); done
if [ "${#judges[@]}" -eq 0 ]; then
  for id in "${real[@]}"; do
    if [ -n "$require_real" ]; then fail "$id" "needs a real forge-warden (FW_BIN) and none was given"
    else echo "SKIP $id (no FW_BIN)"; fi
  done
  [ -n "$require_real" ] || echo "skipped: ${#real[@]} rows that need a real forge-warden (no FW_BIN)"
else
  for fw_given in "${judges[@]}"; do . "$here/contract-matrix/real_rows.sh"; done
fi
# With --require-real every forge-warden binaries.tsv records must have been given.
if [ -n "$require_real" ]; then
  given=" $(for j in "${judges[@]}"; do [ -f "$j" ] && sum "$j"; done | tr '\n' ' ')"
  while IFS=$'\t' read -r role sha _ _ name; do
    [ "$role" = forge-warden ] || continue
    if [[ "$given" == *" $sha "* ]]; then pass "fw-given[$name]"
    else fail "fw-given[$name]" "forge-warden $name (sha256 $sha) is recorded but not given (FW_BIN, FW_BIN_OLDER)"; fi
  done <"$here/contract-matrix/binaries.tsv"
fi

echo "rows red: $red"
[ "$red" -eq 0 ]
