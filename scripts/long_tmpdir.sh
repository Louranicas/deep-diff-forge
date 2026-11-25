#!/usr/bin/env bash
# A temp directory as long and as indirect as macOS's, made on Linux (P1, 0.11.1): under <base>,
# a real directory whose path is 57 characters and a symbolic link to it whose path is 49, and
# the link printed. macOS's TMPDIR is `/var/folders/<2>/<28>/T/`, 49 characters, reached through
# the link `/var` -> `/private/var`, so Git and realpath name it with 57; Linux's `/tmp/` has 5.
# Run a suite with TMPDIR=<the link> to meet the product's bounds and the links a temp path of
# that length crosses there (measurement M4: nine of the macOS CI failures of 0.11.0 come back).
# Here the link is the last component, not an ancestor; the lengths are the same.
#
#   scripts/long_tmpdir.sh <base>     prints <base>/l<random> (the link)
#
# <base> is an existing directory, taken with its links resolved. LONG_TMPDIR_LINK and
# LONG_TMPDIR_REAL set other lengths (the real path at least as long as the link). The real
# directory is made by mktemp (mode 0700); remove both when done: `rm "$link"` and
# `rm -r "$real"` (the link's target). Exit status: 0 with the link printed; 2 on a usage error,
# a base that is not a directory, or one too long to leave room for the lengths.
set -u
link_len="${LONG_TMPDIR_LINK:-49}"
real_len="${LONG_TMPDIR_REAL:-57}"
usage() {
  echo "usage: $0 <base directory>   (LONG_TMPDIR_LINK and LONG_TMPDIR_REAL: the lengths, 49 and 57)" >&2
  exit 2
}
[ "$#" -eq 1 ] || usage
case "$link_len:$real_len" in *[!0-9:]* | :* | *:) usage ;; esac
[ "$real_len" -ge "$link_len" ] || usage
base=$(cd -- "$1" 2>/dev/null && pwd -P) || {
  echo "long_tmpdir.sh: $1 is not a directory" >&2
  exit 2
}
n=${#base}
# The link's name: `l` and at least one character; the real directory's: `r` and at least six
# that mktemp chooses (the link's name takes its first ones, so two runs do not meet).
if [ $((link_len - n - 1)) -lt 2 ] || [ $((real_len - n - 1)) -lt 7 ]; then
  echo "long_tmpdir.sh: the base $base is $n characters: too long for a link of $link_len and a directory of $real_len" >&2
  exit 2
fi
xs=$(printf '%*s' $((real_len - n - 2)) '' | tr ' ' X)
real=$(mktemp -d "$base/r$xs") || exit 2
random=${real##*/r}
link="$base/l${random:0:$((link_len - n - 2))}"
if [ -e "$link" ] || [ -L "$link" ] || ! ln -s "$real" "$link"; then
  echo "long_tmpdir.sh: could not make the link $link" >&2
  rmdir "$real"
  exit 2
fi
echo "$link"
