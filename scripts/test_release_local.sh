#!/usr/bin/env bash
# Self-test of scripts/release_local.sh. It runs against a throwaway clone of this repository's
# HEAD in a new temporary directory, with GIT_ALLOW_PROTOCOL=file, no global or system Git
# configuration, two SSH keys made for the test there (never ~/.ssh) and a bare "gitlab" remote
# that is a local directory. No URL of a real service is used: the github.com cases are local
# paths that only contain the name. Every failure case is a plant on a scratch copy.
#
# Cases: 1 a good build and its verify pass; 2 a byte changed in the archive fails verify (as is,
# and with both checksum files recomputed); 3 SHA256SUMS signed by a key not in allowed_signers
# fails verify; 4 a tag whose name is not the workspace version is refused by build and fails
# verify; 5 github.com in an argument, an environment variable or the gitlab remote's URL is
# refused; 6 a commit whose binary differs between two builds writes nothing; 7 an archive that
# also holds ./D/deep-diff-forge (another binary, which `tar -xzf` leaves in place of the good
# one), with its checksums recomputed and signed by the good key, fails verify; 8 the gate's SBOM
# step (scripts/ci_local.sh --sbom-gate), stopped after the generator ran or run to its end, leaves
# sbom.spdx.json untouched. Case 1 also checks the receipt's gate result and artefact checks.
#
# Run it where cargo builds (here: in the fedora-toolbox-44 toolbox), from a clean checkout:
#   scripts/test_release_local.sh
# Exit status: 0 if every case passed, 1 otherwise.
set -euo pipefail

export GIT_ALLOW_PROTOCOL=file GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
ns=deep-diff-forge-release-sha256sums
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
tool="$here/release_local.sh"
repo="$(git -C "$here" rev-parse --show-toplevel)"
head="$(git -C "$repo" rev-parse HEAD)"
t="$(mktemp -d "${TMPDIR:-/var/tmp}/ddf-reltool-test.XXXXXX")"
trap 'rm -rf -- "$t"' EXIT
failures=0

# expect <case> <exit status> <text the output must hold> <command>...
expect() {
  local name="$1" want="$2" text="$3" rc=0
  shift 3
  "$@" >"$t/log" 2>&1 || rc=$?
  if [ "$rc" = "$want" ] && grep -qF -- "$text" "$t/log"; then
    echo "PASS $name (exit $rc)"
  else
    echo "FAIL $name (exit $rc; wanted $want and \"$text\")"
    tail -n 15 "$t/log" | sed 's/^/    /'
    failures=$((failures + 1))
  fi
}
absent() { if [ -e "$1" ]; then echo "FAIL $1 was written"; failures=$((failures + 1)); else echo "PASS nothing written to $1"; fi; }
flip_byte() { python3 -I -c 'import sys; p=sys.argv[1]; b=bytearray(open(p,"rb").read()); b[len(b)//2]^=0xff; open(p,"wb").write(b)' "$1"; }

mkdir "$t/keys"
ssh-keygen -q -t ed25519 -N '' -C reltool-test-good -f "$t/keys/good"
ssh-keygen -q -t ed25519 -N '' -C reltool-test-other -f "$t/keys/other"
printf 'reltool-test@example.invalid namespaces="git,%s" %s\n' "$ns" "$(cut -d' ' -f1,2 "$t/keys/good.pub")" >"$t/allowed_signers"

git clone --quiet --no-local --no-tags "$repo" "$t/repo"
cd "$t/repo"
git checkout --quiet --detach "$head"
git remote remove origin
git init --quiet --bare "$t/gitlab.git"
git remote add gitlab "$t/gitlab.git"
git update-ref refs/remotes/gitlab/main "$head"
git config user.name reltool-test
git config user.email reltool-test@example.invalid
git config gpg.format ssh
git config user.signingkey "$t/keys/good"
version="$(sed -n '/^\[workspace.package\]/,/^\[/{s/^version = "\(.*\)"/\1/p}' Cargo.toml)"
git tag -s "v$version" -m "v$version" "$head"
git tag -s v9.9.9 -m v9.9.9 "$head"
dist="deep-diff-forge-v$version-x86_64-unknown-linux-gnu.tar.gz"
echo "throwaway clone $t/repo at $head (version $version)"

# 1: a good build, signed with the test key, verifies.
printf 'gate on %s\nPASS one\nsteps failed: 0\n' "$head" >"$t/gate.log"
expect "1a build $head" 0 "both builds:" env DDF_GATE_LOG="$t/gate.log" "$tool" build "$head" "$t/out"
expect "1c the receipt names the gate and the checks on the artefact" 0 "gate pass" python3 -I -c '
import json, sys
r = json.load(open(sys.argv[1])); g = r["gate"]
assert (g["result"], g["commit"], g["steps"], len(r["checks_on_artefact"])) == ("pass", sys.argv[2], 1, 5), r
print("gate", g["result"])' "$t/out/release-receipt.json" "$head"
ssh-keygen -q -Y sign -f "$t/keys/good" -n "$ns" "$t/out/SHA256SUMS"
expect "1b verify v$version" 0 "verify v$version: PASS" "$tool" verify "v$version" "$t/out" "$t/allowed_signers"

# 2: a byte changed in the archive after the build.
cp -r "$t/out" "$t/p-byte"
flip_byte "$t/p-byte/$dist"
expect "2a changed byte, sums as built" 1 "does not match SHA256SUMS" \
  "$tool" verify "v$version" "$t/p-byte" "$t/allowed_signers"
cp -r "$t/p-byte" "$t/p-resum"
(cd "$t/p-resum" && sha256sum "$dist" >"$dist.sha256" && sha256sum -- *.gz *.sha256 *.json >SHA256SUMS)
expect "2b changed byte, sums recomputed, old signature" 1 "SHA256SUMS.sig does not verify" \
  "$tool" verify "v$version" "$t/p-resum" "$t/allowed_signers"

# 3: SHA256SUMS signed by a key that allowed_signers does not list.
cp -r "$t/out" "$t/p-sig"
cp "$t/out/SHA256SUMS" "$t/sums-other"
ssh-keygen -q -Y sign -f "$t/keys/other" -n "$ns" "$t/sums-other"
cp "$t/sums-other.sig" "$t/p-sig/SHA256SUMS.sig"
expect "3 signature by another key" 1 "SHA256SUMS.sig does not verify" \
  "$tool" verify "v$version" "$t/p-sig" "$t/allowed_signers"

# 4: a tag that is not the workspace version.
expect "4a build v9.9.9 of a $version commit" 1 "does not match the workspace version v$version" \
  "$tool" build v9.9.9 "$t/out-v"
absent "$t/out-v"
expect "4b verify v9.9.9" 1 "tag v9.9.9 does not match the workspace version" \
  "$tool" verify v9.9.9 "$t/out" "$t/allowed_signers"

# 5: github.com, in any case, is refused before anything runs.
expect "5a argument" 2 "refused: the argument" "$tool" build /nonexistent/GitHub.com/x "$t/out-g"
expect "5b environment" 2 "refused: the environment variable CARGO_HOME" \
  env CARGO_HOME=/nonexistent/github.COM/cargo "$tool" build "$head" "$t/out-g"
git remote set-url gitlab /nonexistent/GITHUB.com/x.git
expect "5c gitlab remote URL" 2 "refused: the URL of remote gitlab" \
  "$tool" verify "v$version" "$t/out" "$t/allowed_signers"
git remote set-url gitlab "$t/gitlab.git"
absent "$t/out-g"

# 7: a second spelling of the binary's name in the archive (review of 0.8.4, R84B-L-2): check 3
# read the member D/deep-diff-forge, while `tar -xzf` leaves the later ./D/deep-diff-forge.
mkdir "$t/p-member"
cp "$t/out/$dist" "$t/out/release-receipt.json" "$t/out/sbom.spdx.json" "$t/p-member/"
python3 -I -c '
import io, sys, tarfile
p = sys.argv[1]
with tarfile.open(p, "r:gz") as t:
    members = [(m, t.extractfile(m).read() if m.isfile() else None) for m in t.getmembers()]
with tarfile.open(p, "w:gz", format=tarfile.GNU_FORMAT) as t:
    for m, data in members:
        t.addfile(m, io.BytesIO(data) if data is not None else None)
    evil = b"#!/bin/sh\necho planted\n"
    info = tarfile.TarInfo("./" + members[0][0].name.rstrip("/") + "/deep-diff-forge")
    info.size, info.mode = len(evil), 0o755
    t.addfile(info, io.BytesIO(evil))
' "$t/p-member/$dist"
(cd "$t/p-member" && sha256sum "$dist" >"$dist.sha256" && sha256sum -- *.gz *.sha256 *.json >SHA256SUMS)
ssh-keygen -q -Y sign -f "$t/keys/good" -n "$ns" "$t/p-member/SHA256SUMS"
expect "7 a second spelling of the binary in the archive" 1 "the archive's members are not exactly" \
  "$tool" verify "v$version" "$t/p-member" "$t/allowed_signers"

# 8 (an_interrupted_sbom_step_leaves_the_tracked_file): a python3 shim marks the end of the
# generator's run and waits; the step's process group is stopped there by its recorded pid. With
# SOURCE_DATE_EPOCH=1 a regenerated file would differ from HEAD (the comparison ignores the time).
mkdir "$t/shim" "$t/gate-tmp"
printf '#!/bin/sh\n%q "$@"; rc=$?; : >%q; sleep 60; exit $rc\n' "$(command -v python3)" "$t/gen-done" >"$t/shim/python3"
chmod 755 "$t/shim/python3"
before="$(stat -c '%i %Y' sbom.spdx.json)"
setsid env PATH="$t/shim:$PATH" TMPDIR="$t/gate-tmp" SOURCE_DATE_EPOCH=1 \
  "$here/ci_local.sh" --sbom-gate "$t/repo" </dev/null >"$t/sbom.log" 2>&1 &
pid=$!
for _ in $(seq 240); do [ -e "$t/gen-done" ] || ! kill -0 "$pid" 2>/dev/null && break; sleep 0.5; done
kill -TERM -- "-$pid" 2>/dev/null || true
wait "$pid" || true
untouched() {
  if [ -e "$t/gen-done" ] && [ -z "$(git status --porcelain sbom.spdx.json)" ] &&
    [ "$(stat -c '%i %Y' sbom.spdx.json)" = "$before" ] && [ -z "$(ls -A "$t/gate-tmp")" ]; then
    echo "PASS $1"
  else
    echo "FAIL $1: sbom.spdx.json differs from HEAD or was rewritten, or the scratch is left"
    git status --porcelain sbom.spdx.json | sed 's/^/    /'; tail -n 5 "$t/sbom.log" | sed 's/^/    /'
    failures=$((failures + 1))
  fi
}
untouched "8a an_interrupted_sbom_step_leaves_the_tracked_file"
expect "8b the SBOM step run to its end passes" 0 "the SBOM matches HEAD" \
  env TMPDIR="$t/gate-tmp" SOURCE_DATE_EPOCH=1 "$here/ci_local.sh" --sbom-gate "$t/repo"
untouched "8c the SBOM step run to its end leaves the tracked file"

# 6: a plant that makes the binary differ between builds (a build script's time in the binary).
git checkout --quiet -b plant-unreproducible "$head"
cat >crates/deep-diff-forge-cli/build.rs <<'EOF'
fn main() {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap();
    println!("cargo:rustc-env=DDF_RELTOOL_PLANT={}", t.as_nanos());
}
EOF
printf '\n#[used]\nstatic DDF_RELTOOL_PLANT: &str = env!("DDF_RELTOOL_PLANT");\n' >>crates/deep-diff-forge-cli/src/main.rs
git add crates/deep-diff-forge-cli/build.rs crates/deep-diff-forge-cli/src/main.rs
git commit --quiet --no-gpg-sign -m "plant: unreproducible binary"
expect "6 unreproducible commit" 1 "the two builds differ" "$tool" build plant-unreproducible "$t/out-u"
absent "$t/out-u"

if [ "$failures" -eq 0 ]; then echo "test_release_local: all cases passed"; else echo "test_release_local: $failures failed"; exit 1; fi
