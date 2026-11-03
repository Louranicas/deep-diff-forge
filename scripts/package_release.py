#!/usr/bin/env python3
"""Pack a release archive that is the same bytes however often, and wherever, it is packed.

`tar czf` recorded each file's mtime, owner and group, the order the directory listing gave and
gzip's own timestamp, so two packs of the same binary differed (expert circle 2026-11-03, C14).
This writes the tar stream itself, in Python's stdlib, so GNU tar on Linux and bsdtar on macOS
cannot differ: entries sorted by name under one directory, every mtime SOURCE_DATE_EPOCH (the
commit time), uid and gid 0 with no names, mode 0755 for an executable and 0644 otherwise, USTAR
format, and a gzip header with no file name and mtime 0. The uncompressed tar stream is
byte-identical for the same inputs and epoch; the compressed bytes also depend on the zlib build.

Usage: python3 -I scripts/package_release.py <dist-name> <out-dir> <file>...
Writes <out-dir>/<dist-name>.tar.gz holding <dist-name>/<basename of each file>.
"""

from __future__ import annotations

import gzip
import io
import os
import pathlib
import stat
import sys
import tarfile


def _epoch() -> int:
    raw = os.environ.get("SOURCE_DATE_EPOCH", "")
    if not raw.isdigit():
        sys.exit("package_release: SOURCE_DATE_EPOCH must be set to the commit time (seconds)")
    return int(raw)


def _entry(name: str, epoch: int, mode: int, kind: bytes = tarfile.REGTYPE, size: int = 0) -> tarfile.TarInfo:
    info = tarfile.TarInfo(name)
    info.type = kind
    info.size = size
    info.mtime = epoch
    info.mode = mode
    info.uid = info.gid = 0
    info.uname = info.gname = ""
    return info


def pack(dist: str, out_dir: pathlib.Path, files: list[pathlib.Path], epoch: int) -> pathlib.Path:
    names = [f.name for f in files]
    if len(set(names)) != len(names):
        sys.exit(f"package_release: two inputs share a file name: {sorted(names)}")
    stream = io.BytesIO()
    with tarfile.open(fileobj=stream, mode="w", format=tarfile.USTAR_FORMAT) as tar:
        tar.addfile(_entry(f"{dist}/", epoch, 0o755, tarfile.DIRTYPE))
        for path in sorted(files, key=lambda p: p.name):
            data = path.read_bytes()
            executable = path.stat().st_mode & (stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)
            info = _entry(f"{dist}/{path.name}", epoch, 0o755 if executable else 0o644, size=len(data))
            tar.addfile(info, io.BytesIO(data))
    out = out_dir / f"{dist}.tar.gz"
    with open(out, "wb") as raw, gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0) as gz:
        gz.write(stream.getvalue())
    return out


def main(argv: list[str]) -> int:
    if len(argv) < 4:
        sys.exit(__doc__.strip().splitlines()[-2])
    dist, out_dir, files = argv[1], pathlib.Path(argv[2]), [pathlib.Path(a) for a in argv[3:]]
    print(pack(dist, out_dir, files, _epoch()))
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
