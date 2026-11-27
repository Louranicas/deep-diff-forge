# The step runner of scripts/ci_local.sh, sourced by it (P7, 0.11.1); the test that holds its
# output is crates/deep-diff-forge-cli/tests/repo_scripts.rs (`the_gate_says_...`).
#
# step <name> <command> [<argument>...]: runs the command with its output kept, and prints one
# line with the step's wall time in seconds (to the tenth, rounded down):
#   PASS <name> (12.3 s)
#   PASS <name> (95.0 s; real rows: 0 run, 12 skipped; skipped: <why>)
#   FAIL <name> (4.1 s)       then the last 15 lines of its output, indented
# What a step skipped is said in one form: a line `skipped: <why>` of its output (the first one
# is carried). The test runner's own summaries count too: `N ignored` in the `test result:` lines
# becomes `skipped: N tests ignored`. The contract matrix's line `real rows: N run, M skipped` is
# carried as it is. Every step is recorded in the file $gate_steps (one line each: name, status,
# seconds, what it skipped; tab-separated) for gate_receipt. $failed counts the failed steps.
#
# gate_receipt <file> <commit>: writes the gate's receipt, JSON: the commit, whether the checkout
# had uncommitted changes, when the gate began, each step (name, status, seconds, and
# `skipped` where it said so) and the number failed. Needs python3, as the gate does.
failed=0
gate_started=$(date -u +%FT%TZ)

# The time in microseconds: bash 5's EPOCHREALTIME (its decimal mark taken out), else whole
# seconds from date(1).
gate_now() {
  local t="${EPOCHREALTIME:-}"
  if [ -n "$t" ]; then echo "${t/[.,]/}"; else echo "$(($(date +%s) * 1000000))"; fi
}

step() {
  local name="$1" start out status us seconds skip rows ignored
  shift
  start=$(gate_now)
  if out=$("$@" 2>&1); then status=PASS; else status=FAIL; fi
  us=$(($(gate_now) - start))
  seconds="$((us / 1000000)).$((us / 100000 % 10))"
  skip=$(printf '%s\n' "$out" | grep -m1 '^skipped: ' || true)
  ignored=$(printf '%s\n' "$out" | sed -n 's/^test result: .* \([0-9][0-9]*\) ignored;.*/\1/p' |
    awk '{ n += $1 } END { print n + 0 }')
  if [ -z "$skip" ] && [ "$ignored" -gt 0 ]; then skip="skipped: $ignored tests ignored"; fi
  rows=$(printf '%s\n' "$out" | grep -m1 '^real rows: ' || true)
  echo "$status $name ($seconds s${rows:+; $rows}${skip:+; $skip})"
  if [ "$status" = FAIL ]; then
    failed=$((failed + 1))
    printf '%s\n' "$out" | tail -15 | sed 's/^/    /'
  fi
  printf '%s\t%s\t%s\t%s\n' "$name" "$status" "$seconds" "${skip#skipped: }" >>"$gate_steps"
}

gate_receipt() {
  local dirty=no
  [ -n "$(git status --porcelain 2>/dev/null)" ] && dirty=yes
  python3 -P -E - "$1" "$2" "$dirty" "$gate_started" "$gate_steps" <<'PY'
import json, sys
path, commit, dirty, started, steps = sys.argv[1:6]
rows = []
with open(steps, encoding="utf-8", errors="replace") as lines:
    for line in lines:
        name, status, seconds, skipped = line.rstrip("\n").split("\t", 3)
        row = {"name": name, "status": status, "seconds": float(seconds)}
        if skipped:
            row["skipped"] = skipped
        rows.append(row)
receipt = {
    "commit": commit,
    "uncommitted_changes": dirty == "yes",
    "started": started,
    "steps": rows,
    "failed": sum(row["status"] == "FAIL" for row in rows),
}
with open(path, "w", encoding="utf-8") as out:
    json.dump(receipt, out, indent=1)
    out.write("\n")
PY
}
