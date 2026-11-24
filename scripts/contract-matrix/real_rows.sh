# The rows of scripts/contract_matrix.sh that need a real forge-warden binary; sourced by it once
# per forge-warden given, as $fw_given (its functions and $ddf, $here, $py, $require_real are in
# scope). Each id carries the binary's name from binaries.tsv in brackets ($fwn). Expected
# outcomes are measured per judge; where they differ the row says so by $fwn.
fw="$(cd "$(dirname "$fw_given")" && pwd)/$(basename "$fw_given")"
fw_schema="$( [ -x "$fw" ] && recorded forge-warden "$fw")"
fwn="$( [ -x "$fw" ] && recorded forge-warden "$fw" 5)"
if [ -z "$fw_schema" ]; then
  fwn="unrecorded-$( [ -f "$fw" ] && sum "$fw" | cut -c1-12)"
  for id in "${real[@]}"; do
    fail "$id[$fwn]" "$fw_given (sha256 $( [ -f "$fw" ] && sum "$fw")) is not a forge-warden row of binaries.tsv"
  done
  return 0
fi
id="fw-recorded[$fwn]"
if "$fw" schema verdict 2>/dev/null | grep -qF "$fw_schema"; then pass "$id" "$(sum "$fw"), $fw_schema"
else fail "$id" "$fw schema verdict does not name $fw_schema, the schema binaries.tsv records"; fi

# policy <file> <ranker sha256>: forge-warden's own starter policy admitting that ranker.
policy() { { "$fw" init && printf '\n[producer]\nsha256 = ["%s"]\n' "$2"; } >"$1"; }
# A committed change and its patch, made by forge-warden's recorded make_patch.sh.
row "fw-patch[$fwn]" '{}'
git_ -C "$d/repo" add --all && git_ -C "$d/repo" commit --quiet -m change
(cd "$d" && env -i PATH="$PATH" TMPDIR="$d" bash "$maker" HEAD~1 HEAD "$d/repo" >"$d/change.patch")
patch="$d/change.patch"
: >"$scratch/empty.patch"
# check <dir> <ranker> <patch> [args...]: forge-warden's authority check; out, err and rc in <dir>.
check() {
  local dir="$1" ranker="$2" p="$3"
  shift 3
  policy "$dir/policy.toml" "$(sum "$ranker")"
  (cd "$dir" && env -i PATH="$PATH" HOME="$dir" "$fw" check --policy "$dir/policy.toml" --patch "$p" \
    --ranker "$ranker" --json "$@" >"$dir/fw.out" 2>"$dir/fw.err"; echo $? >"$dir/fw.rc")
}
notes() { "$py" -P -E -c 'import json, sys
d = json.load(open(sys.argv[1]))
print(d["schema"], d["verdict"], *[n["code"] for n in d["notes"]])' "$1/fw.out" 2>/dev/null; }

id="fw-judges-the-stop[$fwn]"
row "$id" '{}'
policy "$d/repo/warden.toml" "$(sum "$ddf")"
git_ -C "$d/repo" add warden.toml && git_ -C "$d/repo" commit --quiet -m policy
if stop s1 false "$fw" && expect allow "HOLD" UNJUDGED "could not be read" "does not read"; then pass "$id"
else fail "$id" "$why"; fi

# logging <dir>: <dir>/fwlog, a judge that writes its arguments (one run a line) to <dir>/fw.args
# and runs this forge-warden with them. The hook runs its judge as a program, so it can be wrapped;
# forge-warden runs its ranker from a verified descriptor, so a ranker cannot.
logging() {
  printf '#!/bin/sh\nprintf "%%s\\n" "$*" >>"%s"\nexec "%s" "$@"\n' "$1/fw.args" "$fw" >"$1/fwlog"
  chmod 755 "$1/fwlog"
}
# held <dir>: the ids of the rules of <dir>/fw.out that hold.
held() { "$py" -P -E -c 'import json, sys
print(*[r["id"] for r in json.load(open(sys.argv[1]))["rules"] if r["outcome"] == "hold"])' "$1/fw.out" 2>/dev/null; }
# Measured per judge: whether its usage names --repo (bound mode, the hook's own test), and
# whether it passes --allow-empty to its ranker (forge-warden from Train B15, 005a1959).
bound_judge=
"$fw" check --help 2>/dev/null | tr -s ' \t' '\n\n' | grep -qx -- --repo && bound_judge=1
allow_empty=
case "$fwn" in 005a1959) allow_empty=1 ;; esac

# S3.25.b: under forge-warden's starter policy a commit with one parent passes. A judge whose
# usage names --repo is started in bound mode for it (005a1959 and db4c580e, whose starter
# policy requires verified claims); the released 0.2.1, whose usage does not, is started as
# before and passes, its starter policy (v0) requiring no verified claims. The ledger records
# the pass; the uncommitted change holds where claims are required (the row above).
id="fw-a-commit-passes-in-bound-mode[$fwn]"
row "$id" '{}'
logging "$d"
policy "$d/repo/warden.toml" "$(sum "$ddf")"
git_ -C "$d/repo" checkout --quiet -- a.rs
git_ -C "$d/repo" add warden.toml && git_ -C "$d/repo" commit --quiet -m policy
if stop s1 false "$d/fwlog" && printf 'fn a() {}\nfn b() {}\nfn c() {}\n' >"$d/repo/a.rs" &&
  git_ -C "$d/repo" commit --quiet -am c && stop s1 false "$d/fwlog" &&
  expect allow "" HOLD UNJUDGED claims_unverified &&
  head="$(git_ -C "$d/repo" rev-parse HEAD)" &&
  grep -q "\"outcome\":\"pass\",.*\"unit\":\"commit:$head\"" "$d/repo/.git/deep-diff-forge/ledger.jsonl" &&
  if [ -n "$bound_judge" ]; then grep -q -- "--repo .* --head $head --json" "$d/fw.args"
  else ! grep -q -- --repo "$d/fw.args"; fi
then pass "$id" "$( [ -n "$bound_judge" ] && echo bound || echo 'not bound')"; else fail "$id" "$why; args: $(tail -c 300 "$d/fw.args" 2>/dev/null)"; fi

# Labels. The labels `schema signals` lists that this forge-warden does not know (its lint of a
# policy forbidding them all names them) are allowed, and the contract is that an unknown label
# holds: a change that raises stub_added (from todo!()) holds with the note unknown_signal under
# a policy with no other reason to hold where the judge does not know it (db4c580e, 0.2.1), and
# where it knows it, a policy with a rule for it holds it by that rule (005a1959, rule
# no-placeholders); a clean change passes. A label no deep-diff-forge raises (a_made_up_label,
# added by the stand-in ranker made_up_label.rs, built here with rustc) holds as unknown with
# every judge.
id="fw-unknown-labels-hold[$fwn]"
row "$id" '{}'
{ printf 'schema = "forge-warden.policy.v0"\n\n[[rule]]\nid = "every-label"\nwhy = "each label must be known"\n'
  printf 'kind = "forbid_signal"\nseverity = "hold"\nsignals = [%s]\n' "$("$ddf" schema signals |
    "$py" -P -E -c 'import json, sys; print(", ".join(json.dumps(s["label"]) for s in json.load(sys.stdin)["signals"]))')"
} >"$d/labels.toml"
"$fw" lint "$d/labels.toml" >"$d/lint.out" 2>&1
unknown="$(grep -o 'unknown signal \\\?"[a-z_]*' "$d/lint.out" | grep -o '[a-z_]*$' | sort -u | tr '\n' ' ')"
# plain <file> <ranker> [rule]: a v0 policy (every judge reads v0) that fails a secret only, with
# the rule no-placeholders when asked, admitting <ranker>.
plain() {
  { printf 'schema = "forge-warden.policy.v0"\n\n[[rule]]\nid = "no-secrets"\nwhy = "a committed credential is compromised"\nkind = "forbid_signal"\nseverity = "fail"\nsignals = ["secret_like"]\n'
    [ -n "${3:-}" ] && printf '\n[[rule]]\nid = "no-placeholders"\nwhy = "a placeholder is unfinished work"\nkind = "forbid_signal"\nseverity = "hold"\nsignals = ["stub_added"]\n'
    printf '\n[producer]\nsha256 = ["%s"]\n' "$(sum "$2")"; } >"$1"
}
known=; [[ " $unknown" == *" stub_added "* ]] || known=1
declare -A said_of=()
for change in clean stub made-up; do
  ranker="$ddf" patch_of="$d/$change.patch"
  if [ "$change" = made-up ]; then
    ranker="$scratch/made-up-label/ranker" patch_of="$d/stub.patch"
    if [ ! -x "$ranker" ]; then
      mkdir -p "${ranker%/*}"
      if ! DDF_UNDER="$ddf" rustc -O --edition 2021 -o "$ranker" "$here/contract-matrix/made_up_label.rs" \
        >"${ranker%/*}/rustc.out" 2>&1; then
        said_of[$change]="the stand-in ranker was not built: $(head -c 200 "${ranker%/*}/rustc.out")"
        continue
      fi
    fi
  else
    [ "$change" = clean ] && body='fn c() {}' || body='fn c() { todo!() }'
    printf 'fn a() {}\nfn b() {}\n%s\n' "$body" >"$d/repo/a.rs"
    git_ -C "$d/repo" commit --quiet -am "$change"
    (cd "$d" && env -i PATH="$PATH" TMPDIR="$d" bash "$maker" HEAD~1 HEAD "$d/repo" >"$patch_of")
  fi
  rule=; [ "$change" = stub ] && [ -n "$known" ] && rule=1
  plain "$d/plain.toml" "$ranker" "$rule"
  (cd "$d" && env -i PATH="$PATH" HOME="$d" "$fw" check --policy "$d/plain.toml" --patch "$patch_of" \
    --ranker "$ranker" --json >"$d/fw.out" 2>"$d/fw.err"; echo $? >"$d/fw.rc")
  said_of[$change]="$(cat "$d/fw.rc") $(notes "$d") [$(held "$d")]"
  if [ "$change" = made-up ] && ! grep -q 'a_made_up_label' "$d/fw.out"; then said_of[$change]+=" (a_made_up_label not named)"; fi
done
# (the made-up label is a second unknown one where stub_added is unknown too)
if [ -n "$known" ]; then want="8 $fw_schema hold [no-placeholders]" made="8 $fw_schema hold unknown_signal []"
else want="8 $fw_schema hold unknown_signal []" made="8 $fw_schema hold unknown_signal unknown_signal []"; fi
if [ "${said_of[clean]}" = "0 $fw_schema pass []" ] && [ "${said_of[stub]}" = "$want" ] &&
  [ "${said_of[made-up]}" = "$made" ]
then pass "$id" "unknown here: ${unknown:-none}"
else fail "$id" "clean: ${said_of[clean]}; stub: ${said_of[stub]} (wanted $want); made-up: ${said_of[made-up]} (wanted $made); unknown here: ${unknown:-none}"; fi

id="fw-reads-the-rank[$fwn]"
mkdir -p "$scratch/$id"
"$ddf" --stdin-patch --rank --json <"$patch" >"$scratch/$id/rank.json"
check "$scratch/$id" "$ddf" "$patch" --rank "$scratch/$id/rank.json"
said="$(notes "$scratch/$id")"
if grep -q '"scorer_sha256"' "$scratch/$id/rank.json" && [[ "$said" == "$fw_schema "* ]] && [[ "$said" != *rank_mismatch* ]]
then pass "$id" "$said"; else fail "$id" "rc $(cat "$scratch/$id/fw.rc"): $said $(head -c 300 "$scratch/$id/fw.err")"; fi

# An empty patch through the authority check. A judge that passes --allow-empty to its ranker
# (005a1959) gets an empty ranking from a ranker that knows the option (this build, 0.10.0) and
# holds (empty_ranking), and from one that does not (0.9.0) "the ranker exited 2" and no
# verdict; a judge that does not pass it gets "the ranker exited 4" and no verdict from a ranker
# that refuses an empty patch without it (this build, 0.10.0), and a holding verdict from 0.9.0.
# empty_row <id> <ranker> <1 when the ranker knows --allow-empty>
empty_row() {
  local id="$1" ranker="$2" knows="$3" rc said exited=4
  mkdir -p "$scratch/$id"
  check "$scratch/$id" "$ranker" "$scratch/empty.patch"
  rc="$(cat "$scratch/$id/fw.rc")"; said="$(notes "$scratch/$id")"
  if [ "${allow_empty:-0}" = "${knows:-0}" ]; then
    if [ "$rc" = 8 ] && [[ "$said" == "$fw_schema hold "* ]] && [[ "$said" == *empty_ranking* ]]
    then pass "$id" "$said"; else fail "$id" "rc $rc (wanted 8, empty_ranking): $said $(head -c 300 "$scratch/$id/fw.err")"; fi
  else
    [ -n "$allow_empty" ] && exited=2
    if [ "$rc" = 3 ] && [ ! -s "$scratch/$id/fw.out" ] && grep -qF "the ranker exited $exited" "$scratch/$id/fw.err"
    then pass "$id" "the ranker exited $exited"; else fail "$id" "rc $rc (wanted 3, the ranker exited $exited): $said $(head -c 300 "$scratch/$id/fw.err")"; fi
  fi
}
empty_row "fw-empty-patch-this-ranker[$fwn]" "$ddf" 1
published ranker-0.9.0 DDF_RANKER_090 "fw-empty-patch-ranker-0.9.0[$fwn]" &&
  empty_row "fw-empty-patch-ranker-0.9.0[$fwn]" "$DDF_RANKER_090" ""
published ranker-0.10.0 DDF_RANKER_0100 "fw-empty-patch-ranker-0.10.0[$fwn]" &&
  empty_row "fw-empty-patch-ranker-0.10.0[$fwn]" "$DDF_RANKER_0100" 1

# F4.d, a policy that admits an older ranker: the policy at the base admits only a published
# ranker (0.9.0, 0.10.0), which the stop finds in its --ranker-dir; the commit's stop is judged
# with it (the judge is given that file as --ranker) and passes.
for older in ranker-0.9.0:DDF_RANKER_090 ranker-0.10.0:DDF_RANKER_0100; do
  role="${older%%:*}" var="${older#*:}"
  id="fw-an-older-ranker-is-admitted-${role#ranker-}[$fwn]"
  published "$role" "$var" "$id" || continue
  row "$id" '{}'
  logging "$d"
  mkdir -p "$d/rankers" && cp "${!var}" "$d/rankers/deep-diff-forge-${role#ranker-}"
  policy "$d/repo/warden.toml" "$(sum "${!var}")"
  git_ -C "$d/repo" checkout --quiet -- a.rs
  git_ -C "$d/repo" add warden.toml && git_ -C "$d/repo" commit --quiet -m policy
  stop_args=(--ranker-dir "$d/rankers")
  if stop s1 false "$d/fwlog" && printf 'fn a() {}\nfn b() {}\nfn c() {}\n' >"$d/repo/a.rs" &&
    git_ -C "$d/repo" commit --quiet -am c && stop s1 false "$d/fwlog" &&
    expect allow "" HOLD UNJUDGED && grep -qF -- "--ranker $d/rankers/deep-diff-forge-${role#ranker-} " "$d/fw.args"
  then pass "$id"; else fail "$id" "$why; args: $(tail -c 400 "$d/fw.args" 2>/dev/null)"; fi
  stop_args=()
done

# F4.d, the frontier: a commit left by a session that never stopped (here a hidden character,
# which every starter policy fails) is judged by the next session's stop and blocks it, with
# the real judge's FAIL.
id="fw-the-frontier-judges-a-commit-left-behind[$fwn]"
row "$id" '{}'
policy "$d/repo/warden.toml" "$(sum "$ddf")"
git_ -C "$d/repo" checkout --quiet -- a.rs
git_ -C "$d/repo" add warden.toml && git_ -C "$d/repo" commit --quiet -m policy
if hookrun session-start SessionStart left "$fw" && printf 'fn a() {}\n// \342\200\256 reversed\nfn b() {}\n' >"$d/repo/a.rs" &&
  git_ -C "$d/repo" commit --quiet -am "left by a session that never stopped" &&
  left="$(git_ -C "$d/repo" rev-parse --short=12 HEAD)" &&
  hookrun session-start SessionStart next "$fw" && stop next false "$fw" &&
  expect block "Commit $left (not this session's: left or merged since the project was first recorded): FAIL"
then pass "$id"; else fail "$id" "$why"; fi

# F4.d, `agent check` says what the stop says: with the Stop hook registered for this judge,
# `agent check --session` after a stop judges the same units with the stop's inputs, says so
# (its equality sentence), and gives each unit the stop's outcome.
id="fw-agent-check-equals-the-stop[$fwn]"
row "$id" '{}'
policy "$d/repo/warden.toml" "$(sum "$ddf")"
git_ -C "$d/repo" add warden.toml && git_ -C "$d/repo" commit --quiet -m policy
register "$fw"
if hookrun session-start SessionStart s1 "$fw" && printf 'fn c() {}\n' >"$d/repo/c.rs" &&
  git_ -C "$d/repo" add c.rs && git_ -C "$d/repo" commit --quiet -m c && stop s1 false "$fw" &&
  { run_ddf agent check --session s1 --json >"$d/check.json" 2>"$d/check.err"; [ "$?" -le 8 ]; } &&
  "$py" -P -E -c 'import json, sys
d = json.load(open(sys.argv[1]))
said = sys.argv[2]
outcomes = sorted(u["outcome"] for u in d["units"])
held = "HOLD" in said
ok = d["stop"]["said"].startswith("Equal to the stop in its units and inputs: ") and len(outcomes) == 2
ok = ok and (("hold" in outcomes) == held)
sys.exit(0 if ok else 1)' "$d/check.json" "$said"
then pass "$id"; else fail "$id" "$why; check: $(head -c 600 "$d/check.json" 2>/dev/null) $(head -c 200 "$d/check.err" 2>/dev/null)"; fi
