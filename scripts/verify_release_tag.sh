#!/usr/bin/env bash
# Refuses a release tag unless it is an annotated tag signed by a key in the given allowed-signers
# file. release.yml runs it before anything is built or published, with the signers file taken
# from protected `main`, not from the tagged commit: a tag on another branch's commit could
# otherwise bring its own key. `just release-tag` signs with the same check locally
# (scripts/sign_release_tag.sh); this is the server-side half (expert circle 2026-11-03, C4).
# Usage: scripts/verify_release_tag.sh <tag> <allowed-signers-file>
set -euo pipefail
tag="${1:?usage: scripts/verify_release_tag.sh <tag> <allowed-signers-file>}"
signers="${2:?usage: scripts/verify_release_tag.sh <tag> <allowed-signers-file>}"
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
echo "${tag}: signature verified against the release allowed-signers file"
