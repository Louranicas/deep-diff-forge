#!/usr/bin/env bash
# Anchor B-07: the local replica of the gate jobs of .github/workflows/ci.yml (secrets, msrv, gate,
# supply-chain). `just ci` runs it. Each step's label is the exact name of the CI step it
# replicates, with ` :: <part>` where one CI step runs several commands; the test
# crates/deep-diff-forge-cli/tests/ci_replica.rs fails when a CI step has no replica step or a
# label names no CI step. Prints PASS/FAIL per step.
# Exit status: 0 only if every step passed, 1 if any failed, 2 if the toolchain is missing.
# Usage: scripts/ci_local.sh [<checkout>]   (default: the current directory)
cd "${1:-.}" || exit 2
unset CARGO_TARGET_DIR
command -v cargo >/dev/null || { echo "ci_local.sh: no cargo on PATH" >&2; exit 2; }
gitleaks=$(command -v gitleaks || echo "$HOME/.local/bin/gitleaks")
failed=0
step() {
  name="$1"
  shift
  if out=$("$@" 2>&1); then
    echo "PASS $name"
  else
    echo "FAIL $name"
    failed=$((failed + 1))
    echo "$out" | tail -15 | sed 's/^/    /'
  fi
}
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
step "hostile daemon soak smoke" env DDF_SOAK_SECONDS=10 python3 scripts/security/daemon_soak.py
step "privacy probe failure regressions" python3 scripts/security/test_privacy_probe.py
step "learning privacy probe" python3 scripts/security/privacy_probe.py
step "python SDK against the built binary" bash -c "cargo build -q --locked -p deep-diff-forge-cli && python3 -I sdk/python/test_deep_diff_forge.py"
step "cargo-deny (bans, licenses, sources, advisories)" cargo deny --locked check
step "cargo-deny fuzz harness" cargo deny --manifest-path fuzz/Cargo.toml --locked check
step "cargo audit (strict advisory gate)" cargo audit --deny warnings
step "cargo audit fuzz lockfile" cargo audit --deny warnings --file fuzz/Cargo.lock
step "SPDX SBOM gate (regenerate + verify against committed baseline)" bash -c 'python3 scripts/security/generate_spdx_sbom.py >/dev/null && python3 - <<PY
import re,subprocess,sys
s=lambda t: re.sub(r"\"created\":\s*\"[^\"]*\"","\"created\": \"<x>\"",t)
regen=open("sbom.spdx.json").read(); committed=subprocess.check_output(["git","show","HEAD:sbom.spdx.json"],text=True)
sys.exit(0 if s(regen)==s(committed) else 1)
PY
rc=$?; git checkout -q -- sbom.spdx.json; exit $rc'
echo "steps failed: $failed"
[ "$failed" -eq 0 ]
