#!/usr/bin/env bash
# Creates the signed annotated tag v<version> at HEAD and keeps it only if it verifies against
# .github/allowed_signers, the release-signing keys the owner trusts (owner decision 2026-11-03).
# A tag signed by any other key, or not signed at all, is deleted before anything is pushed:
# release tags are immutable once pushed (tag ruleset), so a bad one cannot be fixed later.
# The signing key comes from the caller's git config (user.signingkey); the format is pinned to
# SSH because allowed_signers lists SSH keys. Setup: docs/RELEASE_AND_PUBLICATION.md#signed-tags
# Usage: scripts/sign_release_tag.sh <version>
set -euo pipefail
version="${1:?usage: scripts/sign_release_tag.sh <version>}"
tag="v${version}"
cd "$(git rev-parse --show-toplevel)"
signers="$PWD/.github/allowed_signers"
if [ ! -s "$signers" ]; then
  echo "no release signers in .github/allowed_signers; refusing to tag" >&2
  exit 1
fi
git -c gpg.format=ssh tag -s "$tag" -m "$tag"
if ! git -c gpg.format=ssh -c gpg.ssh.allowedSignersFile="$signers" tag -v "$tag" >/dev/null 2>&1; then
  git tag -d "$tag" >/dev/null
  echo "$tag is not signed by a key in .github/allowed_signers; deleted the local tag" >&2
  exit 1
fi
echo "$tag signed and verified against .github/allowed_signers"
