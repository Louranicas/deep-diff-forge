#!/usr/bin/env bash
# forge-warden patch-maker v2
# Write, on stdout, the patch forge-warden should judge for the change from BASE to HEAD.
# Usage: make_patch.sh <base> <head> [<repository>]      (repository: default, the current directory)
#
# The patch is the same bytes whatever the caller's Git configuration, environment or current
# directory are. A plain `git diff` is not: `diff.noprefix` drops the `a/` `b/` prefixes (the
# judge then reads `scripts/x` as `x`), `diff.relative` in a subdirectory leaves out every change
# outside it, and colour, context and quoting settings change the bytes. So the diff runs in a
# new, empty repository that only borrows the object store: no configuration of the user, the
# system or the repository is read for it, and every option that matters is also given here.
#
# The change is `base...head`: from the merge base of the two commits to HEAD, as on a pull request.
# This does not prove to anyone else that the patch is that change; it makes it hard to get wrong.
set -euo pipefail
if [ "$#" -ne 2 ] && [ "$#" -ne 3 ]; then
  echo 'usage: make_patch.sh <base> <head> [<repository>]' >&2
  exit 2
fi
repository="${3:-.}"
# Nothing about Git comes from the caller's environment.
while IFS= read -r name; do unset "$name"; done < <(compgen -v GIT_)
export GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null GIT_ATTR_NOSYSTEM=1 GIT_NO_REPLACE_OBJECTS=1
base="$(git -C "$repository" rev-parse --verify --quiet --end-of-options "$1^{commit}")" || {
  echo "make_patch: base is not a commit in $repository: $1" >&2
  exit 2
}
head="$(git -C "$repository" rev-parse --verify --quiet --end-of-options "$2^{commit}")" || {
  echo "make_patch: head is not a commit in $repository: $2" >&2
  exit 2
}
objects="$(git -C "$repository" rev-parse --path-format=absolute --git-path objects)"
scratch="$(mktemp -d)"
trap 'rm -rf -- "$scratch"' EXIT
# The private repository has the store's object format (40 hex digits: SHA-1, 64: SHA-256), as the
# door's bound mode does; a SHA-1 repository cannot read a SHA-256 store's objects (H18).
case "${#base}" in
  40) format=sha1 ;;
  64) format=sha256 ;;
  *) echo "make_patch: unexpected object id length ${#base}: $base" >&2; exit 2 ;;
esac
git init --quiet --bare --object-format="$format" "$scratch/diff.git"
printf '%s\n' "$objects" > "$scratch/diff.git/objects/info/alternates"
# The caller's commit-graph and multi-pack index are not read (they can lie about parents; bound mode's
# staging does the same, ADR 0012 D3.1), and several merge bases are refused as bound mode refuses
# them (D7): `git diff A...B` would pick one with only a warning.
nograph=(-c core.commitGraph=false -c core.multiPackIndex=false)
# `merge-base` exits 1 and says nothing when the two have no common history; any other failure is
# Git's (most often a shallow or partial clone missing commits between them: this private repository
# borrows the object store, not the clone's shallow list), and its first line is shown, not renamed.
status=0
bases="$(git --git-dir="$scratch/diff.git" "${nograph[@]}" merge-base --all "$base" "$head" 2> "$scratch/merge-base.err")" \
  || status=$?
if [ "$status" -eq 1 ] && [ ! -s "$scratch/merge-base.err" ]; then
  echo "make_patch: base and head have no common history" >&2
  exit 2
elif [ "$status" -ne 0 ]; then
  echo "make_patch: git merge-base failed (exit $status): $(head -n 1 "$scratch/merge-base.err")" >&2
  echo "make_patch: a shallow or partial clone lacks commits between base and head; fetch them (git fetch --unshallow)" >&2
  exit 2
fi
count="$(printf '%s\n' "$bases" | grep -c .)"
if [ "$count" -ne 1 ]; then
  echo "make_patch: base and head have $count merge bases; name the merge base as --base" >&2
  exit 2
fi
# Attributes come from the empty tree, not from any work tree: a `.gitattributes` saying `-diff`
# or `binary` would otherwise turn added text into a binary patch the judge cannot read.
# --attr-source needs Git 2.40 or later; an older Git refuses the option.
empty_tree="$(git --git-dir="$scratch/diff.git" hash-object -t tree /dev/null)"
# A submodule change is always written as its `diff --git` section with mode 160000. In a work
# tree, `diff.ignoreSubmodules` or `ignore = all` in `.gitmodules` (a file the change itself can
# commit) leaves it out, and `diff.submodule=log` writes a line no patch reader takes as a change.
# The diff options are one line: scripts/tests/dogfood.sh reads it, runs it by hand, and checks
# that the README and the runbook print the same command.
git --git-dir="$scratch/diff.git" "${nograph[@]}" --attr-source="$empty_tree" -c core.attributesFile=/dev/null \
  -c core.quotePath=true \
  diff --no-ext-diff --no-textconv --no-color --no-relative --binary --no-renames --full-index --src-prefix=a/ --dst-prefix=b/ --submodule=short --ignore-submodules=none \
  "$base...$head"
