# The rows of scripts/contract_matrix.sh that need no forge-warden: the stop hook against the
# stand-in judge scripts/contract-matrix/judge.py. Sourced by it (its functions are in scope).
for schema in v2 v3; do
  id="$schema-pass"; row "$id" '{"schema":"'$schema'","outcome":"pass","exit":0}'
  judged allow "" UNJUDGED HOLD
  id="$schema-hold"; row "$id" '{"schema":"'$schema'","outcome":"hold","exit":8}'
  judged allow "deep-diff-forge: HOLD. " UNJUDGED
  id="$schema-fail"; row "$id" '{"schema":"'$schema'","outcome":"fail","exit":7}'
  judged block "deep-diff-forge: the judge fails this change"
done

id=unread-fail-holds-once-then-stands
row "$id" '{"schema":"forge-warden.verdict.v4","outcome":"fail","exit":7}'
if stop s1 false && expect block "exit 7 is the judge's fail, so the stop is held" &&
  stop s1 true && expect allow "deep-diff-forge: FAIL STANDS. "; then pass "$id"; else fail "$id" "$why"; fi

id=exit-4-is-unjudged
row "$id" '{"refuse":true}'
judged allow "UNJUDGED" "the judge fails"

id=a-hanging-judge-is-unjudged
row "$id" '{"hang":true}'
judged allow "UNJUDGED" "the judge fails"

id=require-verified-claims-holds-never-passes
row "$id" '{"schema":"v3","outcome":"hold","exit":8,"notes":[{"code":"claims_unverified","detail":"the patch was supplied without --repo, --base and --head; its claims cannot be verified"}]}'
if stop s1 false && expect allow "HOLD" UNJUDGED && [[ "$said" == *claims_unverified* ]] &&
  stop s1 true && expect allow "HOLD" UNJUDGED && [[ "$said" == *claims_unverified* ]] &&
  run_ddf agent doctor --gate --json --judge "$d/judge" >"$d/doctor.json" 2>"$d/err"
  [ "$?" -eq 7 ] && "$py" -P -E -c 'import json, sys
c = [c for c in json.load(open(sys.argv[1]))["checks"] if c["name"] == "claims_verified"]
sys.exit(0 if c and not c[0]["ok"] and "stops here will hold, never pass: " in json.dumps(c[0]) else 1)' "$d/doctor.json"
then pass "$id" "claims_verified is weak in agent doctor --gate"; else fail "$id" "$why; doctor: $(head -c 300 "$d/doctor.json" 2>/dev/null)"; fi

id=the-judged-patch-is-make-patch-bytes
row "$id" '{"schema":"v3","outcome":"pass","exit":0}'
if stop s1 false && [ -s "$d/judged.patch" ] && [ "$(sum "$maker")" = "$(grep -o '^[0-9a-f]\{64\}' "$maker.sha256")" ] &&
  git_ -C "$d/repo" add --all && git_ -C "$d/repo" commit --quiet -m change &&
  (cd "$d" && env -i PATH="$PATH" TMPDIR="$d" bash "$maker" HEAD~1 HEAD "$d/repo" >"$d/made.patch") &&
  cmp -s "$d/judged.patch" "$d/made.patch"; then
  pass "$id"
else
  fail "$id" "the judge's patch is not what forge-warden's make_patch.sh writes for the commit ($why)"
fi

# F11.c: a stop records each commit in the project's ledger, and `agent verify-branch` refuses
# (exit 7, branch_unverified) a commit no stop passed: one that failed, one the judge refused
# (exit 4: unjudged); one that passed is verified (exit 0).
for answer in fail:7:7 refuse:4:7 pass:0:0; do
  IFS=: read -r kind code want <<<"$answer"
  id="verify-branch-$kind"
  if [ "$kind" = refuse ]; then row "$id" '{"refuse":true}'
  else row "$id" '{"schema":"v3","outcome":"'"$kind"'","exit":'"$code"'}'; fi
  stop s1 false; git_ -C "$d/repo" commit --quiet -am "the change, committed"
  if stop s1 true; then
    run_ddf agent verify-branch HEAD~1..HEAD >"$d/verify.out" 2>"$d/verify.err"; rc=$?
    if [ "$rc" -eq "$want" ] && { [ "$want" -eq 0 ] ||
      grep -qx 'refused: 1 of 1 commits are not verified by the ledger (verify-branch)' "$d/verify.err"; }; then
      pass "$id" "exit $rc"
    else fail "$id" "verify-branch exit $rc (want $want): $(head -c 300 "$d/verify.err")"; fi
  else fail "$id" "$why"; fi
done

id=control-a-fail-let-through-is-red
row "$id" '{"schema":"v3","outcome":"fail","exit":7}'
if stop s1 false && expect allow ""; then
  fail "$id" "the known-bad control (a FAIL expected to pass the stop) came out green"
else
  pass "$id" "red, as it must be"
fi

