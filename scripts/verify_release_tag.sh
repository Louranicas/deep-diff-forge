#!/usr/bin/env bash
# Refuses a release tag unless it is an annotated tag signed by a key in the given allowed-signers
# file. release.yml runs it before anything is built or published, with the signers file taken
# from protected `main`, not from the tagged commit: a tag on another branch's commit could
# otherwise bring its own key. `just release-tag` signs with the same check locally
# (scripts/sign_release_tag.sh); this is the server-side half (expert circle 2026-11-03, C4).
# The tagged commit must also be on `main`: a signed tag on an unmerged branch commit is refused,
# so a release is only ever built from reviewed code. GitHub runs the release.yml of the tagged
# commit, so a branch that also edits release.yml could drop this check. That needs someone who
# can push both a branch and a signed tag (docs/RELEASE_AND_PUBLICATION.md#signed-tags).
# Usage: scripts/verify_release_tag.sh <tag> <allowed-signers-file> <main-commit>
set -euo pipefail
usage="usage: scripts/verify_release_tag.sh <tag> <allowed-signers-file> <main-commit>"
tag="${1:?$usage}"
signers="${2:?$usage}"
main="${3:?$usage}"
if [ ! -s "$signers" ]; then
  echo "::error::no release signers in ${signers}; refusing ${tag}" >&2
  exit 1
fi
# A lightweight tag names a commit and carries no signature.
if [ "$(git cat-file -t "refs/tags/${tag}" 2>/dev/null)" != "tag" ]; then
  echo "::error::${tag} is not an annotated tag in this clone (a shallow checkout may need it fetched)" >&2
  exit 1
fi
if ! git -c gpg.format=ssh -c gpg.ssh.allowedSignersFile="$signers" tag -v "$tag" >/dev/null 2>&1; then
  echo "::error::${tag} is not signed by a key in the release allowed-signers file" >&2
  exit 1
fi
if ! git merge-base --is-ancestor "refs/tags/${tag}^{commit}" "$main" 2>/dev/null; then
  echo "::error::${tag} names a commit that is not on main (${main}); a release is built only from main" >&2
  exit 1
fi
echo "${tag}: signature verified against the release allowed-signers file, and its commit is on main"
