#!/usr/bin/env bash
# Anchor B-07: the local replica of the gate jobs of .github/workflows/ci.yml (secrets, msrv, gate,
# supply-chain). `just ci` runs it. Each step's label is the exact name of the CI step it
# replicates, with ` :: <part>` where one CI step runs several commands; the test
# crates/deep-diff-forge-cli/tests/ci_replica.rs fails when a CI step has no replica step or a
# label names no CI step. Prints PASS/FAIL per step.
# Steps only this machine's gate runs (GitHub is a mirror; its workflow is not changed for them)
# are listed in LOCAL_ONLY in ci_replica.rs. A step whose output has a line `skipped: <why>`
# passes and says so. check_glibc_floor.sh is not a step of its own: the test step runs it on the
# test binary (crate_edges.rs), the release-script self-test on the release binary.
# Exit status: 0 only if every step passed, 1 if any failed, 2 if the toolchain is missing.
# Usage: scripts/ci_local.sh [--sbom-gate] [<checkout>]   (default: the current directory;
#   --sbom-gate runs the SBOM step alone, for test_release_local.sh)
sbom_only=
if [ "${1:-}" = --sbom-gate ]; then sbom_only=1; shift; fi
cd "${1:-.}" || exit 2
# The gate's own scratch directory: no step writes into the checkout.
scratch=$(mktemp -d "${TMPDIR:-/var/tmp}/ci-local.XXXXXX") || exit 2
trap 'rm -rf -- "$scratch"' EXIT
trap 'exit 143' INT TERM HUP
unset CARGO_TARGET_DIR
command -v cargo >/dev/null || { echo "ci_local.sh: no cargo on PATH" >&2; exit 2; }
# Every step runs on the toolchain ci.yml pins for these jobs, not the floating rustup default, so
# a `rustup update` cannot make PASS here differ from CI (ci_replica.rs checks the two agree); the
# MSRV step's `cargo +1.88.0` overrides it.
toolchain=1.99.1
for tool in cargo rustfmt cargo-clippy; do
  rustup which --toolchain "$toolchain" "$tool" >/dev/null 2>&1 || {
    echo "ci_local.sh: Rust $toolchain with rustfmt and clippy is missing" \
      "(rustup toolchain install $toolchain --component rustfmt,clippy)" >&2
    exit 2
  }
done
export RUSTUP_TOOLCHAIN="$toolchain"
gitleaks=$(command -v gitleaks || echo "$HOME/.local/bin/gitleaks")
failed=0
step() {
  name="$1"
  shift
  if out=$("$@" 2>&1); then
    skip=$(printf '%s\n' "$out" | grep -m1 '^skipped: ' || true)
    echo "PASS $name${skip:+ ($skip)}"
  else
    echo "FAIL $name"
    failed=$((failed + 1))
    echo "$out" | tail -15 | sed 's/^/    /'
  fi
}
# The SBOM is generated into the scratch directory (the generator's OUTPUT redirected) and compared
# with the committed one: sbom.spdx.json in the checkout is never written, however the step ends.
sbom_gate() {
  python3 -P -E - "$scratch/sbom.spdx.json" <<'PY'
import importlib.util, pathlib, re, subprocess, sys
spec = importlib.util.spec_from_file_location("gen", "scripts/security/generate_spdx_sbom.py")
gen = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gen)
gen.OUTPUT, gen.print = pathlib.Path(sys.argv[1]), lambda *a, **k: None
gen.main()
s = lambda t: re.sub(r'"created":\s*"[^"]*"', '"created": "<x>"', t)
committed = subprocess.check_output(["git", "show", "HEAD:sbom.spdx.json"], text=True)
sys.exit(0 if s(gen.OUTPUT.read_text()) == s(committed) else 1)
PY
}
if [ -n "$sbom_only" ]; then sbom_gate && echo "the SBOM matches HEAD's"; exit; fi
echo "gate on $(git rev-parse HEAD 2>/dev/null || echo unknown)$(git status --porcelain 2>/dev/null | grep -q . && echo ' (uncommitted changes)')"
step "gitleaks (pinned release, checksum-verified)" "$gitleaks" git --no-banner --redact --exit-code 1 .
step "check on Rust 1.88" cargo +1.88.0 check --workspace --all-targets --locked
step "parsed antipattern policy and planted refusal controls :: selftest" python3 scripts/security/review_policy_check.py --selftest
step "parsed antipattern policy and planted refusal controls :: check" python3 scripts/security/review_policy_check.py
step "exemption oracle unit tests" python3 -I crates/deep-diff-forge-review/policy/test_exemption_oracle.py
step "format" cargo fmt --all --check
step "compile" cargo check --workspace --locked
step "lint" cargo clippy --workspace --all-targets --locked -- -D warnings
step "pedantic" cargo clippy --workspace --all-targets --locked -- -D warnings -W clippy::pedantic
step "pedantic (feature and fuzz coverage) :: patch" cargo clippy -p deep-diff-forge-patch --no-default-features --all-targets --locked -- -D warnings -W clippy::pedantic
step "pedantic (feature and fuzz coverage) :: fuzz" cargo clippy --manifest-path fuzz/Cargo.toml --all-targets --locked -- -D warnings -W clippy::pedantic
step "test" cargo test --workspace --locked
step "docs" env RUSTDOCFLAGS="-D warnings -D rustdoc::broken_intra_doc_links" cargo doc --workspace --no-deps --locked
for c in --self-test doctor claude-code-contract chain-contract cluster-contract loom-contract; do
  step "contracts :: $c" cargo run -q --locked -p deep-diff-forge-cli -- "$c"
done
step "deploy status" cargo run -q --locked -p deep-diff-forge-cli -- deploy status --json
step "fuzz harness compile gate" cargo check --locked --manifest-path fuzz/Cargo.toml --bins
step "daemon soak leak-check regressions" python3 -I scripts/security/test_daemon_soak.py
step "bench comparison regressions" python3 -I scripts/test_bench_compare.py
step "hostile daemon soak smoke" env DDF_SOAK_SECONDS=10 python3 scripts/security/daemon_soak.py
step "privacy probe failure regressions" python3 scripts/security/test_privacy_probe.py
step "learning privacy probe" python3 scripts/security/privacy_probe.py
step "python SDK against the built binary" bash -c "cargo build -q --locked -p deep-diff-forge-cli && python3 -I sdk/python/test_deep_diff_forge.py"
step "cargo-deny (bans, licenses, sources, advisories)" cargo deny --locked check
step "cargo-deny fuzz harness" cargo deny --manifest-path fuzz/Cargo.toml --locked check
step "cargo audit (strict advisory gate)" cargo audit --deny warnings
step "cargo audit fuzz lockfile" cargo audit --deny warnings --file fuzz/Cargo.lock
step "SPDX SBOM gate (regenerate + verify against committed baseline)" sbom_gate
step "release-script self-test" scripts/test_release_local.sh
# The comparison corpus lives outside the repository; where it is absent the step says so.
step "comparison corpus manifest" bash -c 'c=${DDF_COMPARE_CORPUS:-$HOME/.local/state/habitat-claude/ddf-corpus/compare-v1}
[ -d "$c" ] || { echo "skipped: no corpus here ($c)"; exit 0; }
cd "$c" && sha256sum -c --quiet MANIFEST.sha256 && echo "manifest digest $(sha256sum MANIFEST.sha256 | cut -c1-64)"'
echo "steps failed: $failed"
[ "$failed" -eq 0 ]
