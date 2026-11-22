# The rows of scripts/contract_matrix.sh that need a real forge-warden binary; sourced by it when
# FW_BIN is set (its functions and $ddf, $here, $py, $real, $require_real are in scope). Expected
# outcomes, measured on forge-warden's hub main fbd621e0 and in ruling S3.5.0: an empty patch
# through the authority check holds (empty_ranking) with a 0.9.0 ranker and is exit 3, "the
# ranker exited 4", with no verdict, with a 0.10.0 ranker.
fw="$(cd "$(dirname "$FW_BIN")" && pwd)/$(basename "$FW_BIN")"
fw_schema="$( [ -x "$fw" ] && recorded forge-warden "$fw")"
if [ -z "$fw_schema" ]; then
  for id in "${real[@]}"; do
    fail "$id" "FW_BIN $FW_BIN (sha256 $( [ -f "$fw" ] && sum "$fw")) is not a forge-warden row of binaries.tsv"
  done
  return 0
fi
id=fw-recorded
if "$fw" schema verdict 2>/dev/null | grep -qF "$fw_schema"; then pass "$id" "$(sum "$fw"), $fw_schema"
else fail "$id" "$fw schema verdict does not name $fw_schema, the schema binaries.tsv records"; fi

# policy <file> <ranker sha256>: forge-warden's own starter policy admitting that ranker.
policy() { { "$fw" init && printf '\n[producer]\nsha256 = ["%s"]\n' "$2"; } >"$1"; }
# A committed change and its patch, made by forge-warden's recorded make_patch.sh.
row fw-patch '{}'
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

id=fw-judges-the-stop
row "$id" '{}'
policy "$d/repo/warden.toml" "$(sum "$ddf")"
git_ -C "$d/repo" add warden.toml && git_ -C "$d/repo" commit --quiet -m policy
if stop s1 false "$fw" && expect allow "HOLD" UNJUDGED "could not be read" "does not read"; then pass "$id"
else fail "$id" "$why"; fi

id=fw-knows-every-label
mkdir -p "$scratch/$id"
{ printf 'schema = "forge-warden.policy.v1"\n\n[[rule]]\nid = "every-label"\nwhy = "each label must be known"\n'
  printf 'kind = "forbid_signal"\nseverity = "hold"\nsignals = [%s]\n' "$("$ddf" schema signals |
    "$py" -P -E -c 'import json, sys; print(", ".join(json.dumps(s["label"]) for s in json.load(sys.stdin)["signals"]))')"
} >"$scratch/$id/labels.toml"
if "$fw" lint "$scratch/$id/labels.toml" >"$scratch/$id/lint.out" 2>&1; then pass "$id"
else fail "$id" "$(head -c 400 "$scratch/$id/lint.out")"; fi

id=fw-reads-the-rank
mkdir -p "$scratch/$id"
"$ddf" --stdin-patch --rank --json <"$patch" >"$scratch/$id/rank.json"
check "$scratch/$id" "$ddf" "$patch" --rank "$scratch/$id/rank.json"
said="$(notes "$scratch/$id")"
if grep -q '"scorer_sha256"' "$scratch/$id/rank.json" && [[ "$said" == "$fw_schema "* ]] && [[ "$said" != *rank_mismatch* ]]
then pass "$id" "$said"; else fail "$id" "rc $(cat "$scratch/$id/fw.rc"): $said $(head -c 300 "$scratch/$id/fw.err")"; fi

id=fw-empty-patch-this-ranker
mkdir -p "$scratch/$id"
check "$scratch/$id" "$ddf" "$scratch/empty.patch"
if [ "$(cat "$scratch/$id/fw.rc")" = 3 ] && [ ! -s "$scratch/$id/fw.out" ] && grep -qF "the ranker exited 4" "$scratch/$id/fw.err"
then pass "$id"; else fail "$id" "rc $(cat "$scratch/$id/fw.rc"): $(head -c 300 "$scratch/$id/fw.err")"; fi

id=fw-empty-patch-ranker-0.9.0
if [ -z "${DDF_RANKER_090:-}" ]; then
  if [ -n "$require_real" ]; then fail "$id" "needs DDF_RANKER_090"; else echo "SKIP $id (no DDF_RANKER_090)"; fi
elif [ -z "$(recorded ranker-0.9.0 "$DDF_RANKER_090")" ]; then
  fail "$id" "DDF_RANKER_090 $DDF_RANKER_090 is not a ranker-0.9.0 row of binaries.tsv"
else
  mkdir -p "$scratch/$id"
  check "$scratch/$id" "$DDF_RANKER_090" "$scratch/empty.patch"
  said="$(notes "$scratch/$id")"
  if [ "$(cat "$scratch/$id/fw.rc")" = 8 ] && [[ "$said" == "$fw_schema hold "* ]] && [[ "$said" == *empty_ranking* ]]
  then pass "$id" "$said"; else fail "$id" "rc $(cat "$scratch/$id/fw.rc"): $said $(head -c 300 "$scratch/$id/fw.err")"; fi
fi
