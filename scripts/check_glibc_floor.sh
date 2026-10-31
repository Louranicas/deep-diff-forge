#!/usr/bin/env bash
# The declared glibc floor of the Linux release binaries, checked against a built binary. The
# binary must not require (readelf's version needs, any flag but WEAK) a GLIBC_ version newer
# than FLOOR, or it would not load on the systems the docs say it runs on. Weak needs are
# optional symbols the loader may lack: Rust's standard library references `pidfd_spawnp` and
# `pidfd_getpid` (glibc 2.39) that way, and a glibc older than that loads the binary but prints
# one "weak version `GLIBC_2.39' not found" line to stderr on each run. They are listed, not
# refused.
#
# Measured 2026-10-31: the v0.5.0 x86_64 release binary (built on ubuntu-24.04, glibc 2.39) and a
# local build on Fedora 44 (glibc 2.43) both need at most GLIBC_2.34; the v0.5.0 binary ran on
# glibc 2.34 (AlmaLinux 9), 2.35 (Ubuntu 22.04) and 2.36 (Debian 12), and the loader refused it on
# 2.31 (Debian 11). The release smoke (release.yml) runs this on every Linux target, and the
# `Linux arm64 tests` job of ci.yml on its arm64 debug binary.
#
#   scripts/check_glibc_floor.sh <binary>
# Exit status: 0 within the floor, 1 over it or with no GLIBC_ needs to read, 2 on a usage error.
set -euo pipefail

FLOOR=2.34

if [ $# -ne 1 ] || [ ! -f "$1" ]; then
  echo "usage: $0 <binary>" >&2
  exit 2
fi
bin="$1"

# `readelf -V --wide` prints each need as `0x0180:   Name: GLIBC_2.34  Flags: none  Version: 2`.
needs() {
  readelf -V --wide "$bin" | awk -v want="$1" '
    $2 == "Name:" && $3 ~ /^GLIBC_[0-9]/ && $4 == "Flags:" {
      weak = ($5 == "WEAK")
      if ((want == "weak") == weak) { sub(/^GLIBC_/, "", $3); print $3 }
    }' | sort -uV
}

hard="$(needs hard)"
if [ -z "$hard" ]; then
  echo "::error::$bin has no GLIBC_ version needs: not a dynamically linked glibc binary" >&2
  exit 1
fi
newest="$(printf '%s\n' "$hard" | tail -n 1)"
weak="$(needs weak | paste -sd ' ' -)"
if [ "$(printf '%s\n%s\n' "$newest" "$FLOOR" | sort -V | tail -n 1)" != "$FLOOR" ]; then
  echo "::error::$bin requires GLIBC_$newest, newer than the declared floor $FLOOR (docs: README.md#supported-platforms)" >&2
  exit 1
fi
echo "glibc floor OK: $bin requires at most GLIBC_$newest (floor $FLOOR); weak, optional: ${weak:-none}"
