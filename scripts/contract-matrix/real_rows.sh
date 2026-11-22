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

# The labels `schema signals` lists that this forge-warden does not know (its lint of a policy
# forbidding them all names them) are allowed; the contract is that an unknown label holds: a
# change that raises one (stub_added, from todo!()) holds with the note unknown_signal under a
# policy with no other reason to hold, where a clean change passes (when stub_added is known, both
# pass).
id=fw-unknown-labels-hold
row "$id" '{}'
{ printf 'schema = "forge-warden.policy.v1"\n\n[[rule]]\nid = "every-label"\nwhy = "each label must be known"\n'
  printf 'kind = "forbid_signal"\nseverity = "hold"\nsignals = [%s]\n' "$("$ddf" schema signals |
    "$py" -P -E -c 'import json, sys; print(", ".join(json.dumps(s["label"]) for s in json.load(sys.stdin)["signals"]))')"
} >"$d/labels.toml"
"$fw" lint "$d/labels.toml" >"$d/lint.out" 2>&1
unknown="$(grep -o 'unknown signal \\\?"[a-z_]*' "$d/lint.out" | grep -o '[a-z_]*$' | sort -u | tr '\n' ' ')"
printf 'schema = "forge-warden.policy.v1"\n\n[[rule]]\nid = "no-secrets"\nwhy = "a committed credential is compromised"\nkind = "forbid_signal"\nseverity = "fail"\nsignals = ["secret_like"]\n\n[producer]\nsha256 = ["%s"]\n' "$(sum "$ddf")" >"$d/plain.toml"
declare -A said_of
for change in clean stub; do
  [ "$change" = clean ] && body='fn c() {}' || body='fn c() { todo!() }'
  printf 'fn a() {}\nfn b() {}\n%s\n' "$body" >"$d/repo/a.rs"
  git_ -C "$d/repo" commit --quiet -am "$change"
  (cd "$d" && env -i PATH="$PATH" TMPDIR="$d" bash "$maker" HEAD~1 HEAD "$d/repo" >"$d/$change.patch")
  (cd "$d" && env -i PATH="$PATH" HOME="$d" "$fw" check --policy "$d/plain.toml" --patch "$d/$change.patch" \
    --ranker "$ddf" --json >"$d/fw.out" 2>"$d/fw.err"; echo $? >"$d/fw.rc")
  said_of[$change]="$(cat "$d/fw.rc") $(notes "$d")"
done
if [[ " $unknown" == *" stub_added "* ]]; then want="8 $fw_schema hold unknown_signal"; else want="0 $fw_schema pass"; fi
if [ "${said_of[clean]}" = "0 $fw_schema pass" ] && [ "${said_of[stub]}" = "$want" ]; then pass "$id" "unknown here: ${unknown:-none}"
else fail "$id" "clean: ${said_of[clean]}; stub: ${said_of[stub]} (wanted $want); unknown here: ${unknown:-none}"; fi

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
