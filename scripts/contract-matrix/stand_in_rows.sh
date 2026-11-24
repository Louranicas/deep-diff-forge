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

# --- rows of 0.11.0 (F4.d) that need no forge-warden ---

# F4.d: `agent verify-branch` over one branch that holds every outcome a stop records: a pass,
# a FAIL that then stands, a HOLD, an UNJUDGED (the judge refused) and a commit no stop saw;
# only the pass is verified (exit 7, four of five refused).
id=verify-branch-every-outcome
row "$id" '{"schema":"v3","outcome":"pass","exit":0,"when":[["FAILME",{"outcome":"fail","exit":7}],["HOLDME",{"outcome":"hold","exit":8}],["REFUSEME",{"refuse":true}]]}'
git_ -C "$d/repo" checkout --quiet -- a.rs
from="$(git_ -C "$d/repo" rev-parse HEAD)"
ok=; stop s1 false && ok=1
for word in PASSES FAILME HOLDME REFUSEME UNSEEN; do
  printf '// %s\n' "$word" >"$d/repo/$word.rs"
  git_ -C "$d/repo" add "$word.rs" && git_ -C "$d/repo" commit --quiet -m "$word"
  case "$word" in
    UNSEEN) ;;
    FAILME) stop s1 false && [ "$decision" = block ] && stop s1 true && expect allow "deep-diff-forge: FAIL STANDS. " || ok= ;;
    HOLDME) stop s1 false && expect allow "deep-diff-forge: HOLD. " || ok= ;;
    REFUSEME) stop s1 false && expect allow "UNJUDGED" || ok= ;;
    *) stop s1 false && expect allow "" HOLD UNJUDGED || ok= ;;
  esac
done
run_ddf agent verify-branch "$from..HEAD" >"$d/verify.out" 2>"$d/verify.err"; rc=$?
if [ -n "$ok" ] && [ "$rc" -eq 7 ] &&
  grep -qx 'refused: 4 of 5 commits are not verified by the ledger (verify-branch)' "$d/verify.err"
then pass "$id" "exit $rc"; else fail "$id" "exit $rc, stops ${ok:-not as wanted} ($why): $(head -c 300 "$d/verify.err")"; fi

# F4.d: a session's first stop is never silent. A stop that is its session's first hook, on a
# clean tree with work committed before it, says that the session's start was not recorded, and
# runs no judge. (tests/agent_hook.rs a_stop_that_is_its_sessions_first_hook_is_never_silent
# holds the words; this row runs the client's way: its own home, no configuration.)
id=a-first-stop-is-never-silent
row "$id" '{"schema":"v3","outcome":"pass","exit":0}'
git_ -C "$d/repo" commit --quiet -am "committed before the session's first hook"
if stop s1 false && expect allow "This session's start was not recorded: " && [ ! -e "$d/judge.log" ]
then pass "$id"; else fail "$id" "$why"; fi

# F12.d, the stateless stop: one row per way a session's state cannot be used whose stop judges
# the working tree without it (the reason classes the document's table gives "the tree judged
# without the state"). The tree fails, so each stop blocks and says which class it was.
# (tests/agent_hook.rs holds each class on its own stand-in; these rows run them the client's
# way, side by side.)
for way in damaged older newer lists_damaged lock_busy no_directory; do
  id="stateless-stop-$way"
  row "$id" '{"schema":"v3","outcome":"fail","exit":7}'
  hookrun session-start SessionStart s1
  state="$(ls "$d"/home/.local/state/deep-diff-forge/hooks/session-*.json 2>/dev/null | head -1)"
  holder=
  case "$way" in
    damaged) printf 'not a state\n' >"$state" ;;
    older | newer)
      [ "$way" = older ] && n=9 || n=14
      "$py" -P -E -c 'import json, sys
p, n = sys.argv[1], sys.argv[2]
d = json.load(open(p)); d["schema"] = "deep-diff-forge.hook-state.v" + n
open(p, "w").write(json.dumps(d))' "$state" "$n" ;;
    lists_damaged) printf 'not a list\n' >"${state%.json}.tips" ;;
    lock_busy)
      flock -x "${state%.json}.lock" sleep 12 </dev/null >/dev/null 2>&1 &
      holder=$!
      sleep 1 ;;
    no_directory) NOHOME=1 ;;
  esac
  stop s1 false && expect block "deep-diff-forge: this session's state could not be used (state.$way): the working tree was judged without it; "
  r=$?
  NOHOME=
  [ -n "$holder" ] && kill "$holder" 2>/dev/null
  if [ -n "$state" ] && [ "$r" -eq 0 ]; then pass "$id"; else fail "$id" "${state:-no state written}; $why"; fi
done

# S3.27.h: a source file that Git calls binary for a stray NUL byte, beside a byte that is not
# UTF-8, is read in a Git mode (`--git-range`): its silenced lint is found (check_suppressed) and
# the change says its diff is suppressed by content (diff_suppressed). The published 0.10.0 read
# such a file as a binary change only (measured: binary_change alone). The hook's patch is Git's
# binary patch and is not read so; this row is the Git modes'.
id=stray-bytes-in-a-source-file-are-read-in-a-git-mode
row "$id" '{}'
printf 'fn a() {}\n#[allow(dead_code)]\nfn b() {}\n// two stray bytes: \000\377\n' >"$d/repo/a.rs"
git_ -C "$d/repo" commit --quiet -am "a source file with a stray byte"
run_ddf --git-range HEAD~1..HEAD --rank --json >"$d/rank.json" 2>"$d/rank.err"
if "$py" -P -E -c 'import json, sys
f = [r for r in json.load(open(sys.argv[1]))["ranked"] if r["path"] == "a.rs"]
sys.exit(0 if f and {"check_suppressed", "diff_suppressed"} <= set(f[0]["signals"]) else 1)' "$d/rank.json"
then pass "$id"; else fail "$id" "$(head -c 400 "$d/rank.json") $(head -c 200 "$d/rank.err")"; fi

# F5a and F1.a: the diagnostics document paged by the daemon method `diff.diagnostics` (rpc, at
# its floor of 4,096 bytes) and by MCP's `diagnose_patch` (at its floor of 12,364), and put
# together again: the document `--stdin-patch --diagnostics --json` prints, the SHA-256 of its
# bytes on every page (scripts/contract-matrix/pages.py).
mkdir -p "$scratch/pages"
for i in $(seq 1 30); do
  printf 'diff --git a/f%s.rs b/f%s.rs\n--- a/f%s.rs\n+++ b/f%s.rs\n@@ -1 +1,3 @@\n fn a() {}\n+#[allow(dead_code)]\n+fn b() { todo!() }\n' "$i" "$i" "$i" "$i"
done >"$scratch/pages/many.patch"
for surface in rpc:4096 mcp:12364; do
  id="diagnostics-paged-and-put-together-${surface%%:*}"
  said="$(cd "$scratch/pages" && env -i PATH="$PATH" HOME="$scratch/pages" TMPDIR="$scratch/pages" LANG=C.UTF-8 \
    timeout 120 "$py" -P -E "$here/contract-matrix/pages.py" "${surface%%:*}" "$ddf" "$scratch/pages/many.patch" "${surface#*:}" 2>&1)"
  if [ "$?" -eq 0 ]; then pass "$id" "$said"; else fail "$id" "$(printf '%s' "$said" | tail -c 300)"; fi
done
