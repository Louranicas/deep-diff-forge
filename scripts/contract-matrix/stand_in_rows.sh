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

id=control-a-fail-let-through-is-red
row "$id" '{"schema":"v3","outcome":"fail","exit":7}'
if stop s1 false && expect allow ""; then
  fail "$id" "the known-bad control (a FAIL expected to pass the stop) came out green"
else
  pass "$id" "red, as it must be"
fi

