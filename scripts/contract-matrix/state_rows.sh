# The state-version matrix of scripts/contract_matrix.sh (F4.d): the published deep-diff-forge
# binaries (0.9.0 and 0.10.0, which write hook state v10; named by DDF_RANKER_090 and
# DDF_RANKER_0100 and accepted by their SHA-256 in binaries.tsv) and this build (v13; it reads
# v10 to v13) on one session that the other wrote, in both directions, and the published ones on
# the states a build of 0.11.0 wrote in v11, v12 and v13 (tests/fixtures/hook-state/). Sourced by
# it; its functions are in scope. Measured: a newer build carries an older session over (written
# again as v13; the failure that blocked under the older blocks once more, its unkeyed digests
# being emptied, and then stands; the commit judged before is not judged again); an older build
# refuses a newer session at a stop as UNJUDGED and keeps its files byte for byte.
# (tests/agent_hook.rs every_schema_since_the_last_release_is_read holds this build on the
# fixtures; these rows add the published binaries.)

fixtures="$here/../crates/deep-diff-forge-cli/tests/fixtures/hook-state"
# cg: Git in the row's repository as scripts/capture_hook_state.sh runs it (the same names and
# dates give the fixtures' commit ids).
cg() {
  env -i PATH="$PATH" GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1 GIT_AUTHOR_NAME=ddf \
    GIT_AUTHOR_EMAIL=ddf@example.invalid GIT_COMMITTER_NAME=ddf GIT_COMMITTER_EMAIL=ddf@example.invalid \
    GIT_AUTHOR_DATE='2026-11-20T00:00:00+0000' GIT_COMMITTER_DATE='2026-11-20T00:00:00+0000' \
    git -C "$d/repo" -c init.defaultBranch=main -c commit.gpgSign=false "$@"
}
# captured <id>: the capture's repository at its base, a home, and a stand-in judge that fails a
# patch holding BADLINE and passes any other. Sets $d.
captured() {
  d="$scratch/$1"
  mkdir -p "$d/home" "$d/repo"
  cg init --quiet
  printf 'fn a() {}\nfn b() {}\n' >"$d/repo/a.rs"
  printf '# policy\n' >"$d/repo/warden.toml"
  cg add --all && cg commit --quiet -m base
  printf '%s\n' '{"schema":"v2","outcome":"pass","exit":0,"when":[["BADLINE",{"outcome":"fail","exit":7}]]}' >"$d/answer.json"
  printf '#!/bin/sh\nexec %q -P -E %q %q "$@"\n' "$py" "$here/contract-matrix/judge.py" "$d" >"$d/judge"
  chmod 755 "$d/judge"
}
# by <binary> <event> <hook_event_name> [judge]: a hook of the session "fixture" run by <binary>
# the client's way; the decision in $decision, the words in $said, a summary in $why.
by() {
  local rc
  printf '{"hook_event_name":"%s","session_id":"fixture","cwd":"%s","stop_hook_active":false,"tool_name":"Bash"}' \
    "$3" "$d/repo" | (cd "$d/repo" && env -i PATH="$PATH" HOME="$d/home" TMPDIR="$d" LANG=C.UTF-8 \
    GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1 "$1" agent hook "$2" --client claude ${4:+--judge "$4"}) \
    >"$d/out" 2>"$d/err"
  rc=$?
  { read -r decision; said="$(cat)"; } < <("$py" -P -E -c 'import json, sys
t = open(sys.argv[1]).read().strip()
d = json.loads(t) if t else {}
print("block" if d.get("decision") == "block" else "allow")
print(d.get("reason") or d.get("systemMessage") or "")' "$d/out" 2>&1)
  why="$2 by $("$1" --version 2>/dev/null): exit $rc, $decision: $(printf '%s' "$said" | head -c 300) $(head -c 200 "$d/err")"
  [ "$rc" -eq 0 ]
}
# session_by <binary>: the session the capture records, written by <binary>: a start, a commit
# of failing work, a failing edit, an advice hook and a stop that blocks for both.
session_by() {
  by "$1" session-start SessionStart &&
    printf 'fn c() {}\n// BADLINE committed\n' >"$d/repo/c.rs" && cg add c.rs && cg commit --quiet -m 'failing work' &&
    printf 'fn a() {}\n#[allow(dead_code)] // BADLINE\nfn b() {}\n' >"$d/repo/a.rs" &&
    by "$1" post-tool PostToolUse && by "$1" stop Stop "$d/judge" && [ "$decision" = block ]
}
hooks_dir() { printf '%s' "$d/home/.local/state/deep-diff-forge/hooks"; }
# kept: the SHA-256 of every file of the session's state (the state, its lists, the key).
kept() { (cd "$(hooks_dir)" && sha256sum session-* key 2>/dev/null | grep -v '\.lock$'); }
schema_now() { "$py" -P -E -c 'import json, sys; print(json.load(open(sys.argv[1]))["schema"])' "$(hooks_dir)"/session-*.json 2>/dev/null; }
runs() { [ -f "$d/judge.log" ] && wc -l <"$d/judge.log" || echo 0; }
# place <fixture directory>: its files as the session "fixture"'s, the state's `top` and `id`
# naming this repository, as tests/agent_hook.rs place_fixture does.
place() {
  mkdir -p "$(hooks_dir)" && chmod 700 "$(hooks_dir)"
  local name
  name="$(basename "$(ls "$1"/session-*.json)" .json)"
  "$py" -P -E -c 'import json, sys
src, dst, top, ino = sys.argv[1:5]
text = open(src).read()
state = json.loads(text)
text = text.replace("\"top\":" + json.dumps(state["project"]["top"]), "\"top\":" + json.dumps(top), 1)
text = text.replace("\"id\":\"" + state["project"]["id"] + "\"", "\"id\":\"" + ino + "\"", 1)
open(dst, "w").write(text)' "$1/$name.json" "$(hooks_dir)/$name.json" "$(cg rev-parse --show-toplevel)" "$(stat -c %i "$d/repo/.git")"
  cp "$1/$name.tips" "$1/$name.judged" "$(hooks_dir)/"
  if [ -f "$1/key" ]; then cp "$1/key" "$(hooks_dir)/key" && chmod 600 "$(hooks_dir)/key"; fi
}

for published_one in ranker-0.9.0:DDF_RANKER_090 ranker-0.10.0:DDF_RANKER_0100; do
  role="${published_one%%:*}" var="${published_one#*:}" v="${role#ranker-}"

  id="state-written-by-$v-is-carried-over-by-this-build"
  if published "$role" "$var" "$id"; then
    captured "$id"
    ok=
    if session_by "${!var}" && [ "$(schema_now)" = deep-diff-forge.hook-state.v10 ]; then
      before="$(runs)"
      by "$ddf" stop Stop "$d/judge" && expect block "deep-diff-forge: the judge fails this change (" &&
        [ "$(schema_now)" = deep-diff-forge.hook-state.v13 ] && [ "$(runs)" -eq $((before + 1)) ] &&
        by "$ddf" stop Stop "$d/judge" && expect allow "deep-diff-forge: FAIL STANDS. " &&
        [ "$(runs)" -eq $((before + 2)) ] && ok=1
    fi
    if [ -n "$ok" ]; then pass "$id"; else fail "$id" "$why; schema $(schema_now)"; fi
  fi

  id="state-written-by-this-build-is-refused-and-kept-by-$v"
  if published "$role" "$var" "$id"; then
    captured "$id"
    if session_by "$ddf" && [ "$(schema_now)" = deep-diff-forge.hook-state.v13 ] && before="$(kept)" &&
      by "${!var}" stop Stop "$d/judge" && expect allow "UNJUDGED" && [[ "$said" == *"cannot be read as a state"* ]] &&
      [ "$(kept)" = "$before" ]
    then pass "$id"; else fail "$id" "$why; schema $(schema_now)"; fi
  fi

  id="states-v11-to-v13-are-refused-and-kept-by-$v"
  if published "$role" "$var" "$id"; then
    ok=1 seen=
    for fixture in "$fixtures"/v1[123]-*; do
      captured "$id/$(basename "$fixture")"
      place "$fixture"
      before="$(kept)"
      if by "${!var}" stop Stop "$d/judge" && expect allow "UNJUDGED" && [[ "$said" == *"cannot be read as a state"* ]] &&
        [ "$(kept)" = "$before" ]; then seen+=" $(basename "$fixture")"; else ok=; break; fi
    done
    if [ -n "$ok" ] && [ -n "$seen" ]; then pass "$id" "${seen# }"; else fail "$id" "$(basename "${fixture:-none}"): $why"; fi
  fi
done
