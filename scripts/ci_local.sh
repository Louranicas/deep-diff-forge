#!/usr/bin/env bash
# Anchor B-07: the local replica of the gate jobs of .github/workflows/ci.yml (secrets, msrv, gate,
# supply-chain). `just ci` runs it. Each step's label is the exact name of the CI step it
# replicates, with ` :: <part>` where one CI step runs several commands; the test
# crates/deep-diff-forge-cli/tests/ci_replica.rs fails when a CI step has no replica step or a
# label names no CI step. Prints PASS/FAIL per step with its time, and what it skipped
# (scripts/gate_steps.sh, which runs the steps), and writes a JSON receipt of the gate: the
# commit, each step's name, status, seconds and skip reason. Its path is printed (`receipt: `);
# DDF_GATE_RECEIPT names another; by default it is beside the gate's scratch directory.
# Steps only this machine's gate runs (GitHub is a mirror; its workflow is not changed for them)
# are listed in LOCAL_ONLY in ci_replica.rs. A step that skips work says so in one form, a line
# `skipped: <why>` of its output, which its PASS line carries. check_glibc_floor.sh is not a step
# of its own: the test step runs it on the test binary (crate_edges.rs), the release-script
# self-test on the release binary.
# The test step runs under a temp directory as long and as indirect as macOS's (P3 of 0.11.1), made
# by scripts/long_tmpdir.sh in the gate's scratch directory: under /tmp the product's bounds met no
# long path, and tests that fail on macOS passed here. The other steps keep the gate's TMPDIR.
# Exit status: 0 only if every step passed, 1 if any failed, 2 if the toolchain is missing or the
# long temp directory cannot be made.
# Usage: scripts/ci_local.sh [--sbom-gate] [<checkout>]   (default: the current directory;
#   --sbom-gate runs the SBOM step alone, for test_release_local.sh)
sbom_only=
if [ "${1:-}" = --sbom-gate ]; then sbom_only=1; shift; fi
here=$(cd "$(dirname "$0")" && pwd) || exit 2
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
gate_steps="$scratch/steps.tsv"
. "$here/gate_steps.sh" || exit 2
# The SBOM is generated into the scratch directory (the generator's OUTPUT redirected) and compared
# with the committed one: sbom.spdx.json in the checkout is never written, however the step ends.
sbom_gate() {
  python3 -P -E - "$scratch/sbom.spdx.json" <<'PY'
import importlib.util, pathlib, re, subprocess, sys
spec = importlib.util.spec_from_file_location("gen", "scripts/security/generate_spdx_sbom.py")
gen = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gen)
class Scratch(pathlib.Path):
    def relative_to(self, *other, **kw):  # the generator prints OUTPUT relative to the checkout
        return self
gen.OUTPUT = Scratch(sys.argv[1])
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
# The test step's temp directory: a link of 49 characters to a directory of 57, as macOS's is (longer
# by as much as the scratch directory's own path needs, when that is long already); removed with the
# scratch directory. The gate's own TMPDIR comes back after the step.
base=$(cd "$scratch" && pwd -P) || exit 2
long_tmp=$(LONG_TMPDIR_LINK=$((${#base} + 3 > 49 ? ${#base} + 3 : 49)) \
  LONG_TMPDIR_REAL=$((${#base} + 11 > 57 ? ${#base} + 11 : 57)) "$here/long_tmpdir.sh" "$scratch") || {
  echo "ci_local.sh: no long temp directory could be made in $scratch" >&2
  exit 2
}
echo "the test step's TMPDIR: $long_tmp -> $(cd "$long_tmp" && pwd -P)"
if [ "${TMPDIR+set}" = set ]; then gate_tmpdir=$TMPDIR; else gate_tmpdir=; fi
export TMPDIR="$long_tmp"
step "test" scripts/test_step.sh
if [ -n "$gate_tmpdir" ]; then export TMPDIR="$gate_tmpdir"; else unset TMPDIR; fi
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
# The diagnostics contract (F8.c), a step of its own so that the gate's PASS line names it: the
# built binary's `agent doctor` must list diagnostics_parity and find it true (the command line's
# diagnostics document, the daemon's and MCP's are the same bytes), and two tests are asked for by
# their exact names: the one in which every doctor check fails its condition, and the canary (a
# marker in every patch-derived place stays in the escaped path). Each must be the one test that
# ran: a test that is renamed or removed fails the step instead of passing with none.
step "diagnostics contract" bash -c 'set -e
cargo build -q --locked -p deep-diff-forge-cli
h=$(mktemp -d "$0/doctor.XXXXXX"); mkdir -m 700 "$h/run"
env -i PATH="$PATH" HOME="$h" XDG_RUNTIME_DIR="$h/run" target/debug/deep-diff-forge agent doctor --json >"$h/doctor.json"
python3 -P -E -c "import json, sys
d = json.load(open(sys.argv[1])); c = {x[\"name\"]: x[\"ok\"] for x in d[\"checks\"]}
sys.exit(None if d[\"ok\"] is True and c.get(\"diagnostics_parity\") is True else \"agent doctor: diagnostics_parity is not listed and true: %s\" % c)" "$h/doctor.json"
one() { local out; out=$(cargo test -q --locked -p deep-diff-forge-cli "$@" 2>&1) && printf "%s\n" "$out" | grep -q "^test result: ok. 1 passed; 0 failed" || { printf "%s\n" "$out" | tail -8; echo "not exactly one passing test: $*"; exit 1; }; }
one --bin deep-diff-forge -- --exact agent::tests::every_check_condition_can_fail_its_check
one --test daemon_parity -- --exact a_marker_in_every_patch_derived_place_stays_in_the_escaped_path
echo "agent doctor: diagnostics_parity true; 2 tests by name"' "$scratch"
# The contract with forge-warden (F4): stand-in rows always. When FW_BIN names a forge-warden
# (the gate's judge), the real rows run for it and for each of FW_BIN_OLDER (colon-separated),
# with the published binaries DDF_RANKER_090 and DDF_RANKER_0100, and every forge-warden that
# scripts/contract-matrix/binaries.tsv records, and both published binaries, must be given
# (--require-real: a missing one is red).
step "contract matrix" bash -c 'cargo build -q --locked -p deep-diff-forge-cli && scripts/contract_matrix.sh ${FW_BIN:+--require-real} target/debug/deep-diff-forge'
receipt="${DDF_GATE_RECEIPT:-$scratch.receipt.json}"
if gate_receipt "$receipt" "$(git rev-parse HEAD 2>/dev/null || echo unknown)"; then
  echo "receipt: $receipt"
else
  echo "receipt: not written ($receipt)"
fi
echo "steps failed: $failed"
[ "$failed" -eq 0 ]
