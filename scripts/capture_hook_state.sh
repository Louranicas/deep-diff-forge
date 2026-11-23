#!/usr/bin/env bash
# Capture the hook state a deep-diff-forge binary writes, as a fixture of the hook-state version
# policy (docs/reference/agent-hooks.md, "The state's schema"): the files a session leaves in
# the state directory after a session start, a commit of failing work, a failing edit, an
# advice hook and a stop that blocks for both. The test
# `every_schema_since_the_last_release_is_read` builds the same repository again (the same
# files, names and dates give the same commit ids) and has the build under test read them.
#
#   scripts/capture_hook_state.sh BINARY SHA256 DEST ORIGIN
#
# BINARY must have the SHA-256 given, or nothing is captured. DEST is made new (it must not
# exist) and receives the state, its list of commits (`.tips`) and its list of judged commits
# (`.judged`), byte for byte, and a file `ORIGIN` with the binary's version, its SHA-256 and
# the sentence ORIGIN (say which build it is: the published release artefact, or a build of a
# named commit). Fixtures are captured once and never edited: a new schema is a new directory.
#
# The judge is a stand-in that fails a patch holding the word BADLINE and passes any other,
# with a verdict sealed to what it is given, as the tests' stand-in does. Nothing outside a
# scratch directory of its own is read or written but DEST; the user's Git configuration is not
# read, and the hooks keep their state under the scratch directory's home.
set -euo pipefail

if [ "$#" -ne 4 ]; then
  echo "usage: $0 BINARY SHA256 DEST ORIGIN" >&2
  exit 2
fi
binary=$1 expected=$2 dest=$3 origin=$4
found=$(sha256sum <"$binary" | cut -d' ' -f1)
if [ "$found" != "$expected" ]; then
  echo "$binary has SHA-256 $found, not $expected: nothing captured" >&2
  exit 1
fi
if [ -e "$dest" ]; then
  echo "$dest exists: a fixture is captured once and never written over" >&2
  exit 1
fi

scratch=$(mktemp -d "${TMPDIR:-/tmp}/capture-hook-state.XXXXXX")
trap 'chmod -R u+rwX "$scratch"; rm -rf "$scratch"' EXIT
repo=$scratch/repo home=$scratch/home
mkdir -p "$repo" "$home"

export HOME=$home GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1 LC_ALL=C.UTF-8
export GIT_AUTHOR_NAME=ddf GIT_AUTHOR_EMAIL=ddf@example.invalid
export GIT_COMMITTER_NAME=ddf GIT_COMMITTER_EMAIL=ddf@example.invalid
export GIT_AUTHOR_DATE='2026-11-20T00:00:00+0000' GIT_COMMITTER_DATE='2026-11-20T00:00:00+0000'
unset XDG_STATE_HOME XDG_CONFIG_HOME CLAUDE_PROJECT_DIR
g() { git -C "$repo" -c init.defaultBranch=main -c commit.gpgSign=false "$@"; }

g init --quiet
printf 'fn a() {}\nfn b() {}\n' >"$repo/a.rs"
printf '# policy\n' >"$repo/warden.toml"
g add --all
g commit --quiet -m base

# the stand-in judge and its two verdicts
for kind in pass fail; do
  outcome=pass violations='[]'
  if [ "$kind" = fail ]; then
    outcome=fail
    violations='[{"signal":"check_suppressed","path":"a.rs","message":"m"}]'
  fi
  printf '%s\n' '{"schema":"forge-warden.verdict.v2","tool":{"name":"forge-warden","version":"0.2.0"},"evaluated_on":"2026-11-14","seals":"@SEALS@","verdict":"@VERDICT@","exit":{"code":"@CODE@","name":"@NAME@","action":"@ACTION@"},"summary":{"files":1,"rules":1,"@COUNTS@":0},"rules":[{"id":"no-silent-suppression","kind":"forbid_signal","severity":"fail","outcome":"'"$outcome"'","why":"A silenced check hides the next real bug.","violations":'"$violations"'}],"expired_waivers":[],"unused_waivers":[],"notes":[]}' >"$scratch/$kind.json"
done
cat >"$scratch/judge" <<'JUDGE'
#!/bin/sh
here=$(dirname "$0")
policy=; patch=; ranker=
while [ $# -gt 0 ]; do
  case "$1" in
    --policy) policy="$2";;
    --patch) patch="$2";;
    --ranker) ranker="$2";;
  esac
  shift
done
sum() { sha256sum <"$1" | cut -d' ' -f1; }
code=0 doc="$here/pass.json" said=pass name=pass action=proceed
counts='"passed":1,"waived":0,"held":0,"failed":0'
if grep -q BADLINE "$patch"; then
  code=7 doc="$here/fail.json" said=fail name=policy_fail action=change_input
  counts='"passed":0,"waived":0,"held":0,"failed":1'
fi
p=$(sum "$policy"); f=$(sum "$patch"); r=$(sum "$ranker")
seals="{\"policy_sha256\":\"$p\",\"rank_sha256\":\"$f\",\"patch_sha256\":\"$f\",\"patch_facts_sha256\":null,\"provenance\":\"produced\",\"ranked_by\":{\"version\":\"stand-in\",\"sha256\":\"$r\"},\"git\":{\"path\":\"/usr/bin/git\",\"version\":\"git version 2.56.0\",\"sha256\":\"$p\",\"admission\":\"approved\",\"allowlist_sha256\":\"$f\"}}"
sed -e "s/\"@VERDICT@\"/\"$said\"/" -e "s/\"@CODE@\"/$code/" -e "s/\"@NAME@\"/\"$name\"/" \
    -e "s/\"@ACTION@\"/\"$action\"/" -e "s|\"@SEALS@\"|$seals|" -e "s/\"@COUNTS@\":0/$counts/" "$doc"
exit "$code"
JUDGE
chmod 755 "$scratch/judge"

hook() {
  local event=$1 name=$2
  printf '{"hook_event_name":"%s","session_id":"fixture","cwd":"%s","stop_hook_active":false,"tool_name":"Bash"}' \
    "$name" "$repo" | "$binary" agent hook "$event" --client claude ${3:+--judge "$3"} >"$scratch/$event.out"
}

hook session-start SessionStart
printf 'fn c() {}\n// BADLINE committed\n' >"$repo/c.rs"
g add c.rs
g commit --quiet -m 'failing work'
printf 'fn a() {}\n#[allow(dead_code)] // BADLINE\nfn b() {}\n' >"$repo/a.rs"
hook post-tool PostToolUse
hook stop Stop "$scratch/judge"
grep -q '"decision":"block"' "$scratch/stop.out" || {
  echo "the stop did not block: $(head -c 400 "$scratch/stop.out")" >&2
  exit 1
}

state=$home/.local/state/deep-diff-forge/hooks
mkdir -p "$dest"
for ext in json tips judged; do
  files=("$state"/session-*."$ext")
  [ "${#files[@]}" -eq 1 ] && [ -f "${files[0]}" ] || {
    echo "no single .$ext file in the state directory" >&2
    exit 1
  }
  cp "${files[0]}" "$dest/"
done
{
  printf 'binary: %s\n' "$("$binary" --version)"
  printf 'sha256: %s\n' "$found"
  printf 'origin: %s\n' "$origin"
  printf 'captured: by scripts/capture_hook_state.sh; session id "fixture"; commits dated 2026-11-20T00:00:00+0000 by ddf <ddf@example.invalid>\n'
} >"$dest/ORIGIN"
echo "captured into $dest:"
ls "$dest"
