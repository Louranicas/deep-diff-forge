#!/usr/bin/env bash
# Builds and verifies a deep-diff-forge release on this machine, without GitHub: the build and
# check steps of .github/workflows/release.yml, run locally with the scripts that workflow ran.
# It does not publish, push, tag, sign or contact any network service (cargo runs offline; git
# reads only the local repository).
#
#   scripts/release_local.sh build  <commit> <outdir>
#   scripts/release_local.sh verify <tag> <outdir> <allowed_signers>
#
# Run it inside a clone, with the toolchain scripts/ci_local.sh pins at the commit installed
# (here: in the fedora-toolbox-44 toolbox) and the registry fetched once beforehand
# (`cargo fetch --locked`, outside this tool).
#
# build: exports <commit> twice (git archive, into two new directories at two paths), builds
#   each with release.yml's `cargo build --release --locked -p deep-diff-forge-cli --target
#   x86_64-unknown-linux-gnu`, and stops unless the two binaries have one SHA-256. Then it smokes
#   the binary and checks its glibc floor as release.yml did, writes the SBOM, packs the archive
#   with scripts/package_release.py (SOURCE_DATE_EPOCH = commit time) and writes into <outdir>,
#   which must not exist: the archive, its .sha256, sbom.spdx.json, release-receipt.json,
#   RELEASE-MANIFEST.json (what the binary reads and writes, each number read from the binary
#   itself: its version, commit, target and SHA-256, the rank, diagnostics and hook-state schemas
#   and the ledger version it writes, the verdict and hook-state schemas it reads, its engine
#   protocol and methods, its scorer_sha256) and SHA256SUMS over all of them. It prints the
#   owner's two signing commands and never signs.
#   The receipt lists the checks run on the artefact and the gate's result: DDF_GATE_LOG names a
#   log of scripts/ci_local.sh (its `gate on` and `steps failed` lines); without it, "not given".
#   A <commit> that is a tag must be v<workspace version at that commit>.
# verify: (1) the tag: scripts/verify_release_tag.sh with <allowed_signers> and, as main, the
#   local ref gitlab/main (fetch it first; this tool does not), and the tag is v<version>;
#   (2) SHA256SUMS lists exactly the files of <outdir> (but the two signatures), each matches,
#   SHA256SUMS.sig verifies against <allowed_signers> in namespace
#   deep-diff-forge-release-sha256sums, RELEASE-MANIFEST.json.sig in namespace
#   deep-diff-forge-release-manifest, and every member of the manifest has a value;
#   (3) run only when 1 and 2 pass: the archive holds exactly its five members (the directory,
#   the two licences, README.md and the binary), the receipt and the manifest name the tag's
#   commit and version, and a fresh build of the tag's commit has the binary hash of the
#   receipt, of the manifest and of the binary in the archive. Exit 0 only if all three hold.
#
# Both modes refuse any argument, or an environment variable they read (HOME PATH TMPDIR
# CARGO_HOME RUSTUP_HOME GIT_*), that contains github.com, before doing anything; verify also
# refuses a gitlab remote whose URL contains github.com (build reads no remote).
#
# Targets not built: aarch64-unknown-linux-gnu (no cross toolchain on the build machine), macOS
# (no runner). The binary does not depend on the export or target path (measured 2026-11-16),
# but it holds the registry source path under CARGO_HOME: a rebuild that is to give the same
# hash needs the same toolchain and the same CARGO_HOME path (both are in the receipt).
# Exit status: 0 pass; 1 a check failed; 2 usage, or a refused argument or environment.
set -euo pipefail

target=x86_64-unknown-linux-gnu
ns=deep-diff-forge-release-sha256sums
ns_manifest=deep-diff-forge-release-manifest
not_built="aarch64-unknown-linux-gnu (no cross toolchain on the build machine), macOS (no runner)"
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

die() { echo "release_local: $*" >&2; exit 2; }
fail() { echo "release_local: FAIL: $*" >&2; exit 1; }
usage() { sed -n '2,/^# Exit status/{s/^# \{0,1\}//;p}' "${BASH_SOURCE[0]}"; }
guard() { case "${2,,}" in *github.com*) die "refused: $1 contains github.com (nothing at GitHub)" ;; esac; }

for arg in "$@"; do guard "the argument '$arg'" "$arg"; done
for var in HOME PATH TMPDIR CARGO_HOME RUSTUP_HOME DDF_GATE_LOG $(compgen -e | grep '^GIT_' || true); do
  guard "the environment variable $var" "${!var:-}"
done
case "${1:-} $#" in
  "build 3" | "verify 4") ;;
  "-h 1" | "--help 1") usage; exit 0 ;;
  *) usage >&2; exit 2 ;;
esac
git rev-parse --show-toplevel >/dev/null 2>&1 || die "run this inside a clone of the repository"

# The build's whole environment: nothing of the caller's reaches cargo but these, so no
# RUSTFLAGS, CARGO_PROFILE_* or CARGO_BUILD_* can change the binary the gate tested.
cargo_env() {
  local envs=(HOME="$HOME" PATH="$PATH" TMPDIR="${TMPDIR:-/var/tmp}" CARGO_NET_OFFLINE=true
    RUSTUP_TOOLCHAIN="$toolchain")
  [ -z "${CARGO_HOME:-}" ] || envs+=(CARGO_HOME="$CARGO_HOME")
  [ -z "${RUSTUP_HOME:-}" ] || envs+=(RUSTUP_HOME="$RUSTUP_HOME")
  env -i "${envs[@]}" "$@"
}

# Version (read as release.yml's smoke reads it), toolchain pin and commit time of a commit.
prepare() {
  version="$(git show "$1:Cargo.toml" | sed -n '/^\[workspace.package\]/,/^\[/{s/^version = "\(.*\)"/\1/p}')"
  version="${version%%$'\n'*}"
  [ -n "$version" ] || fail "no [workspace.package] version in Cargo.toml at $1"
  toolchain="$(git show "$1:scripts/ci_local.sh" | sed -n 's/^toolchain=//p')"
  toolchain="${toolchain%%$'\n'*}"
  [ -n "$toolchain" ] || fail "no toolchain= pin in scripts/ci_local.sh at $1"
  epoch="$(git log -1 --format=%ct "$1")"
}

# manifest write <binary> <file> <version> <commit> <target> <archive>: RELEASE-MANIFEST.json,
# every number read from the binary itself (each member's command is in `sources`), refused when
# a member has no value. manifest check <file>: the members that have no value, or nothing.
manifest() {
  python3 -I - "$@" <<'EOF'
import hashlib, json, pathlib, subprocess, sys

MEMBERS = ["schema", "version", "commit", "target", "binary_sha256", "archive.name", "archive.sha256",
           "writes.rank_schema", "writes.diagnostics_schema", "writes.hook_state_schema",
           "writes.ledger_version", "reads.verdict_schemas", "reads.hook_state_schemas.from",
           "reads.hook_state_schemas.to", "engine.protocol", "engine.methods", "scorer_sha256",
           "sources"]


def missing(doc):
    out = []
    for path in MEMBERS:
        value = doc
        for key in path.split("."):
            value = value.get(key) if isinstance(value, dict) else None
        if value in (None, "", [], {}):
            out.append(path)
    return out


if sys.argv[1] == "check":
    gone = missing(json.loads(pathlib.Path(sys.argv[2]).read_text()))
    print(", ".join(gone))
    sys.exit(0)

_, binary, out, version, commit, target, archive = sys.argv[1:]
env = {"PATH": "/usr/bin:/bin", "HOME": str(pathlib.Path(out).parent), "LANG": "C.UTF-8"}
basic = pathlib.Path("fixtures/patch/basic.patch").read_bytes()


def run(*args, stdin=b""):
    return subprocess.run([binary, *args], input=stdin, capture_output=True, env=env, check=True).stdout


rank = json.loads(run("--stdin-patch", "--rank", "--json", stdin=basic))
diagnostics = json.loads(run("--stdin-patch", "--diagnostics", "--json", stdin=basic))
doctor = dict(line.split("=", 1) for line in run("doctor").decode().splitlines() if "=" in line)
engine = json.loads(run("rpc", stdin=b'{"jsonrpc":"2.0","id":1,"method":"engine.initialize","params":{}}\n'))["result"]
reads_state, _, writes_state = doctor.get("hook_state_schemas_read", "").partition("; writes ")
low, _, high = reads_state.partition(" to ")
prefix = writes_state.rstrip("0123456789")
said = run("--version").decode().strip()
manifest = {
    "schema": "deep-diff-forge.release-manifest.v0",
    "version": said.removeprefix("deep-diff-forge ") if said == f"deep-diff-forge {version}" else None,
    "commit": commit,
    "target": target,
    "binary_sha256": hashlib.sha256(pathlib.Path(binary).read_bytes()).hexdigest(),
    "archive": {"name": pathlib.Path(archive).name,
                "sha256": hashlib.sha256(pathlib.Path(archive).read_bytes()).hexdigest()},
    "writes": {"rank_schema": rank.get("schema"), "diagnostics_schema": diagnostics.get("schema"),
               "hook_state_schema": writes_state or None,
               "ledger_version": int(doctor["ledger_version"]) if doctor.get("ledger_version", "").isdigit() else None},
    "reads": {"verdict_schemas": [s for s in doctor.get("verdict_schemas_read", "").split(" and ") if s],
              "hook_state_schemas": {"from": prefix + low.removeprefix("v") if low and prefix else None,
                                     "to": prefix + high.removeprefix("v") if high and prefix else None}},
    "engine": {"protocol": engine.get("protocol"), "methods": engine.get("methods")},
    "scorer_sha256": rank.get("tool", {}).get("scorer_sha256"),
    "sources": {
        "version": "deep-diff-forge --version",
        "binary_sha256": "sha256 of the binary in the archive (the two builds' one hash)",
        "writes.rank_schema, scorer_sha256": "deep-diff-forge --stdin-patch --rank --json <fixtures/patch/basic.patch",
        "writes.diagnostics_schema": "deep-diff-forge --stdin-patch --diagnostics --json <fixtures/patch/basic.patch",
        "writes.hook_state_schema, writes.ledger_version, reads.verdict_schemas, reads.hook_state_schemas":
            "deep-diff-forge doctor (its lines hook_state_schemas_read, ledger_version, verdict_schemas_read)",
        "engine": "deep-diff-forge rpc, engine.initialize",
    },
}
gone = missing(manifest)
if gone:
    sys.exit(f"the manifest's members {', '.join(gone)} have no value; nothing written")
pathlib.Path(out).write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
EOF
}

work="$(mktemp -d "${TMPDIR:-/var/tmp}/ddf-release.XXXXXX")"
trap 'rm -rf -- "$work"' EXIT

# A fresh export of <commit> in <dir>/src, built as release.yml builds it; prints the binary's SHA-256.
build_at() {
  mkdir "$2" "$2/src"
  git archive --format=tar "$1" | tar -x -C "$2/src"
  if ! (cd "$2/src" && cargo_env cargo build --release --locked -p deep-diff-forge-cli --target "$target") \
    >"$2/build.log" 2>&1; then
    tail -n 20 "$2/build.log" >&2
    fail "cargo build of $1 in $2/src failed"
  fi
  sha256sum "$2/src/target/$target/release/deep-diff-forge" | cut -d' ' -f1
}

do_build() {
  local ref="$1" out="$2" sha h1 h2 src bin dist stage
  [ ! -e "$out" ] || die "$out exists; name a new directory"
  sha="$(git rev-parse --verify --quiet "$ref^{commit}")" || die "$ref is not a commit"
  prepare "$sha"
  if git rev-parse --verify --quiet "refs/tags/$ref" >/dev/null && [ "$ref" != "v$version" ]; then
    fail "tag $ref does not match the workspace version v$version at $sha"
  fi
  echo "building $sha (version $version, toolchain $toolchain) twice, from two exports"
  h1="$(build_at "$sha" "$work/b1")"
  h2="$(build_at "$sha" "$work/b2")"
  [ "$h1" = "$h2" ] || fail "the two builds differ ($h1, $h2): not reproducible; nothing written to $out"
  echo "both builds: $h1"
  src="$work/b1/src"
  bin="$src/target/$target/release/deep-diff-forge"
  # release.yml's "smoke the release binary" step, run where it ran (the tree's root).
  (
    cd "$src"
    [ "$("$bin" --version)" = "deep-diff-forge $version" ] || fail "$bin --version does not report $version"
    "$bin" --stdin-patch --json --require-files <fixtures/patch/basic.patch >"$work/smoke.json"
    grep -q '"schema": "deep-diff-forge.review.v0"' "$work/smoke.json" || fail "$bin did not produce a review document"
    scripts/check_glibc_floor.sh "$bin"
  )
  # The SBOM and the archive as release.yml made them. Its SBOM step took the time of the last
  # commit to Cargo.lock in a depth-1 checkout, which is the commit time: given here directly,
  # since an export has no history.
  (cd "$src" && cargo_env SOURCE_DATE_EPOCH="$epoch" python3 scripts/security/generate_spdx_sbom.py >/dev/null)
  stage="$work/out"
  mkdir "$stage"
  dist="deep-diff-forge-v$version-$target"
  (cd "$src" && SOURCE_DATE_EPOCH="$epoch" python3 -I scripts/package_release.py "$dist" "$stage" \
    "target/$target/release/deep-diff-forge" README.md LICENSE-MIT LICENSE-APACHE >/dev/null)
  cp "$src/sbom.spdx.json" "$stage/"
  (cd "$stage" && sha256sum "$dist.tar.gz" >"$dist.tar.gz.sha256" && sha256sum -c --quiet "$dist.tar.gz.sha256")
  (cd "$src" && manifest write "$bin" "$stage/RELEASE-MANIFEST.json" "$version" "$sha" "$target" "$stage/$dist.tar.gz") ||
    fail "the release manifest was not written"
  python3 -I - "$stage" "$version" "$sha" "$target" "$h1" "$h2" "$(cargo_env rustc -Vv)" \
    "$(cargo_env cargo -V)" "$toolchain" "$work/b1/src" "$work/b2/src" "${CARGO_HOME:-$HOME/.cargo}" \
    "$not_built" "${DDF_GATE_LOG:-}" <<'EOF'
import datetime, hashlib, json, pathlib, re, sys
stage, version, commit, target, h1, h2, rustc, cargo, pin, root1, root2, cargo_home, not_built, log = sys.argv[1:]
stage = pathlib.Path(stage)
gate = {"result": "not given (DDF_GATE_LOG is not set)"}
if log:
    text = pathlib.Path(log).read_bytes()
    on = re.findall(rb"^gate on ([0-9a-f]+)(.*)$", text, re.M)
    failed = re.findall(rb"^steps failed: (\d+)$", text, re.M)
    gate = {"log": log, "log_sha256": hashlib.sha256(text).hexdigest(),
            "commit": on[-1][0].decode() if on else None,
            "uncommitted_changes": bool(on and on[-1][1]),
            "steps": len(re.findall(rb"^(?:PASS|FAIL) ", text, re.M)),
            "steps_failed": int(failed[-1]) if failed else None}
    gate["result"] = "pass" if gate["steps_failed"] == 0 and gate["commit"] == commit and not gate["uncommitted_changes"] else "not a passing gate of this commit"
receipt = {
    "schema": "deep-diff-forge.release-receipt.v0",
    "version": version, "commit": commit, "target": target,
    "targets_not_built": not_built,
    "toolchain": {"pin": pin, "rustc": rustc.splitlines(), "cargo": cargo},
    "command": f"cargo build --release --locked -p deep-diff-forge-cli --target {target}",
    "build_sha256": [h1, h2],
    "artefacts_sha256": {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(stage.iterdir())},
    "built_at_utc": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
    "build_roots": [root1, root2],
    "build_roots_note": "temporary, removed after the build; the binary does not depend on them",
    "cargo_home": cargo_home,
    "checks_on_artefact": [
        "two builds from two exports give one binary SHA-256",
        f"--version reports deep-diff-forge {version}",
        "--stdin-patch --json on fixtures/patch/basic.patch writes a deep-diff-forge.review.v0 document",
        "scripts/check_glibc_floor.sh on the binary",
        "the archive's .sha256 checks",
    ],
    "gate": gate,
}
(stage / "release-receipt.json").write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")
EOF
  (cd "$stage" && sha256sum -- * >SHA256SUMS)
  cp -r "$stage" "$out"
  out="$(cd "$out" && pwd)"
  echo "wrote $out:"
  (cd "$out" && ls -1)
  echo "targets not built: $not_built"
  echo "Not signed. The owner signs SHA256SUMS and the manifest with the release key (this writes"
  echo "SHA256SUMS.sig and RELEASE-MANIFEST.json.sig):"
  printf '  ssh-keygen -Y sign -f ~/.ssh/id_ed25519_ddf_release_signing -n %s %q\n' "$ns" "$out/SHA256SUMS"
  printf '  ssh-keygen -Y sign -f ~/.ssh/id_ed25519_ddf_release_signing -n %s %q\n' "$ns_manifest" "$out/RELEASE-MANIFEST.json"
  echo "Namespaces $ns and $ns_manifest, not git: a signature in namespace git is one git"
  echo "accepts on a tag or commit, so a file signed there could be passed off as one; each"
  echo "namespace is for its one file only, so neither signature can stand for the other file."
}

do_verify() {
  local tag="$1" out signers url main sha c1=PASS c2=PASS c3=PASS listed present p sig_ok=no rh rc rv h ah d members gone mh mc mv ma
  [ -d "$2" ] || die "$2 is not a directory"
  [ -s "$3" ] || die "$3 is not a non-empty allowed-signers file"
  out="$(cd "$2" && pwd)"
  signers="$(cd "$(dirname "$3")" && pwd)/$(basename "$3")"
  url="$(git remote get-url gitlab 2>/dev/null)" || die "no remote named gitlab in this clone"
  guard "the URL of remote gitlab" "$url"
  main="$(git rev-parse --verify --quiet refs/remotes/gitlab/main)" ||
    die "no gitlab/main in this clone; fetch it first (this tool uses no remote)"
  sha="$(git rev-parse --verify --quiet "refs/tags/$tag^{commit}")" || die "no tag $tag in this clone"
  prepare "$sha"

  if ! "$here/verify_release_tag.sh" "$tag" "$signers" "$main"; then
    c1="FAIL: the tag's signature or its commit (see above)"
  elif [ "$tag" != "v$version" ]; then
    c1="FAIL: tag $tag does not match the workspace version v$version at $sha"
  fi
  echo "check 1 (tag signature, commit on gitlab/main $main, version): $c1"

  if [ ! -s "$out/SHA256SUMS" ] || [ ! -s "$out/SHA256SUMS.sig" ] || [ ! -s "$out/RELEASE-MANIFEST.json.sig" ]; then
    c2="FAIL: SHA256SUMS, SHA256SUMS.sig or RELEASE-MANIFEST.json.sig is missing"
  elif grep -qvE '^[0-9a-f]{64}  [^/]+$' "$out/SHA256SUMS"; then
    c2="FAIL: SHA256SUMS has a line that is not <sha256>  <file name>"
  else
    listed="$(sed 's/^[0-9a-f]*  //' "$out/SHA256SUMS" | LC_ALL=C sort)"
    present="$(cd "$out" && find . -mindepth 1 -maxdepth 1 ! -name SHA256SUMS ! -name SHA256SUMS.sig \
      ! -name RELEASE-MANIFEST.json.sig -printf '%P\n' | LC_ALL=C sort)"
    if [ "$listed" != "$present" ]; then
      c2="FAIL: SHA256SUMS does not list exactly the files in $out"
    elif ! (cd "$out" && sha256sum -c --strict --quiet SHA256SUMS); then
      c2="FAIL: a file in $out does not match SHA256SUMS"
    else
      for p in $(ssh-keygen -Y find-principals -s "$out/SHA256SUMS.sig" -f "$signers" 2>/dev/null); do
        ssh-keygen -Y verify -f "$signers" -I "$p" -n "$ns" -s "$out/SHA256SUMS.sig" \
          <"$out/SHA256SUMS" >/dev/null 2>&1 && sig_ok=yes
      done
      [ "$sig_ok" = yes ] || c2="FAIL: SHA256SUMS.sig does not verify against $signers in namespace $ns"
      if [ "$c2" = PASS ]; then
        sig_ok=no
        for p in $(ssh-keygen -Y find-principals -s "$out/RELEASE-MANIFEST.json.sig" -f "$signers" 2>/dev/null); do
          ssh-keygen -Y verify -f "$signers" -I "$p" -n "$ns_manifest" -s "$out/RELEASE-MANIFEST.json.sig" \
            <"$out/RELEASE-MANIFEST.json" >/dev/null 2>&1 && sig_ok=yes
        done
        if [ ! -s "$out/RELEASE-MANIFEST.json" ]; then
          c2="FAIL: RELEASE-MANIFEST.json is missing"
        elif [ "$sig_ok" != yes ]; then
          c2="FAIL: RELEASE-MANIFEST.json.sig does not verify against $signers in namespace $ns_manifest"
        else
          gone="$(manifest check "$out/RELEASE-MANIFEST.json" 2>&1)" || gone="unreadable"
          [ -z "$gone" ] || c2="FAIL: the manifest's members $gone have no value"
        fi
      fi
    fi
  fi
  echo "check 2 (SHA256SUMS lists and matches every file; its and the manifest's signatures; the manifest's members): $c2"

  if [ "$c1" != PASS ] || [ "$c2" != PASS ]; then
    c3="NOT RUN (an earlier check failed)"
  else
    read -r rh rc rv < <(python3 -I -c 'import json,sys; r=json.load(open(sys.argv[1])); print(r["build_sha256"][0], r["commit"], r["version"])' "$out/release-receipt.json") || true
    read -r mh mc mv ma < <(python3 -I -c 'import json,sys; r=json.load(open(sys.argv[1])); print(r["binary_sha256"], r["commit"], r["version"], r["archive"]["sha256"])' "$out/RELEASE-MANIFEST.json") || true
    ah="$(tar -xzOf "$out/deep-diff-forge-$tag-$target.tar.gz" "deep-diff-forge-$tag-$target/deep-diff-forge" | sha256sum | cut -d' ' -f1)" || ah="no binary in the archive"
    # The members must be exactly these: a second spelling of the binary's name (`./D/…`,
    # `D//…`) is not what -O above reads but is what `tar -xzf` leaves (review of 0.8.4, R84B-L-2).
    d="deep-diff-forge-$tag-$target"
    members="$(tar -tzf "$out/$d.tar.gz" | LC_ALL=C sort)" || members="unreadable"
    if [ "$members" != "$(printf '%s\n' "$d/" "$d/LICENSE-APACHE" "$d/LICENSE-MIT" "$d/README.md" "$d/deep-diff-forge" | LC_ALL=C sort)" ]; then
      c3="FAIL: the archive's members are not exactly $d/ and its LICENSE-APACHE, LICENSE-MIT, README.md and deep-diff-forge"
    elif [ "$rc" != "$sha" ] || [ "$rv" != "$version" ]; then
      c3="FAIL: the receipt is for $rc $rv, the tag names $sha $version"
    elif [ "$mc" != "$sha" ] || [ "$mv" != "$version" ]; then
      c3="FAIL: the manifest is for $mc $mv, the tag names $sha $version"
    elif [ "$ma" != "$(sha256sum "$out/$d.tar.gz" | cut -d' ' -f1)" ]; then
      c3="FAIL: the manifest names another archive ($ma)"
    else
      h="$(build_at "$sha" "$work/b1")"
      [ "$h" = "$rh" ] && [ "$h" = "$ah" ] && [ "$h" = "$mh" ] ||
        c3="FAIL: rebuild $h, receipt $rh, manifest $mh, archive $ah"
    fi
    [ "$c3" != PASS ] || c3="PASS ($h)"
  fi
  echo "check 3 (a fresh build gives the receipt's, the manifest's and the archive's binary): $c3"
  case "$c1 $c2 $c3" in
    "PASS PASS PASS ("*) echo "verify $tag: PASS" ;;
    *) echo "verify $tag: FAIL" >&2; exit 1 ;;
  esac
}

case "$1" in
  build) do_build "$2" "$3" ;;
  verify) do_verify "$2" "$3" "$4" ;;
esac
