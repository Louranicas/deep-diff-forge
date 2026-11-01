#!/usr/bin/env bash
# Loads contrib/launchd/dev.deep-diff-forge.daemon.plist into launchd as a per-user agent, with
# ProgramArguments pointed at <binary> and every path in a fresh temporary directory, and checks
# what the plist and docs/OPERATIONS_AND_DAEMON.md#running-as-a-service promise:
#   1. after `launchctl bootstrap` the daemon answers `daemon health`, with launchd's pid;
#   2. a daemon killed by a signal is restarted (KeepAlive, SuccessfulExit false);
#   3. after `daemon stop` (exit 0) the socket is gone and launchd does not restart it;
#   4. `launchctl bootout` unloads the job;
#   5. a bootout of a running daemon (SIGTERM, no drain) leaves no process and no restart.
# Every wait is bounded. macOS only: the `macOS launchd agent` job of ci.yml runs it. It needs
# the domain of the logged-in user (`gui/<uid>`) and falls back to `user/<uid>`, which a
# session without a GUI login has.
#
#   scripts/launchd_smoke.sh <binary>
# Exit status: 0 if every check passed, 1 on the first that failed, 2 on a usage error.
# Written for the macOS system bash 3.2 too.
set -euo pipefail

if [ $# -ne 1 ] || [ ! -x "$1" ]; then
  echo "usage: $0 <binary>" >&2
  exit 2
fi
bin="$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"
label=dev.deep-diff-forge.daemon
template="$(cd "$(dirname "$0")/.." && pwd)/contrib/launchd/${label}.plist"
uid="$(id -u)"
# A short base: a Unix socket path must fit in 104 bytes on macOS, and $TMPDIR there is long.
work="$(mktemp -d "${RUNNER_TEMP:-/tmp}/ddf-launchd.XXXXXX")"
run="${work}/run"
plist="${work}/LaunchAgents/${label}.plist"
log="${work}/daemon.log"
domain=""

# Both write to stderr: a failure inside `$(...)` must still reach the log.
diagnostics() {
  if [ -n "$domain" ]; then
    launchctl print "${domain}/${label}" 2>&1 | head -n 60 >&2 || true
  fi
  if [ -f "$log" ]; then
    echo "== ${log}" >&2
    tail -n 40 "$log" >&2
  fi
}
fail() {
  echo "::error::launchd smoke: $*" >&2
  diagnostics
  exit 1
}
cleanup() {
  if [ -n "$domain" ]; then
    launchctl bootout "${domain}/${label}" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

# Retries a check once a second, failing after <seconds>.
wait_for() {
  local limit="$1" what="$2" waited=0
  shift 2
  until "$@"; do
    waited=$((waited + 1))
    [ "$waited" -lt "$limit" ] || fail "timed out after ${limit}s waiting for ${what}"
    sleep 1
  done
}
answers() { "$bin" daemon health >/dev/null 2>&1; }
# The pid in the daemon's own `daemon.health` answer, or nothing.
health_pid() {
  { "$bin" daemon health 2>/dev/null || true; } | sed -n 's/.*"pid":\([0-9][0-9]*\).*/\1/p'
}
# The pid launchd reports for the job, or nothing when it is not running or not loaded.
job_pid() {
  { launchctl print "${domain}/${label}" 2>/dev/null || true; } | awk '$1 == "pid" && $2 == "=" { print $3; exit }'
}
restarted_from() {
  local now
  now="$(health_pid)"
  [ -n "$now" ] && [ "$now" != "$1" ]
}
job_gone() { ! launchctl print "${domain}/${label}" >/dev/null 2>&1; }
process_gone() { ! kill -0 "$1" 2>/dev/null; }
no_socket() { [ ! -e "$sock" ]; }
up_with_launchd_pid() {
  wait_for 30 "the daemon to answer under launchd" answers
  local pid
  pid="$(health_pid)"
  if [ -z "$pid" ] || [ "$pid" != "$(job_pid)" ]; then
    fail "daemon.health pid '${pid}' is not launchd's pid '$(job_pid)'"
  fi
  echo "$pid"
}

mkdir -m 700 "$run"
mkdir "${work}/LaunchAgents"
# Install as the plist's own comment says: replace each /Users/YOU value (here with this run's
# paths), then check the result with plutil. Rewriting with `plutil -replace` left a placeholder
# value on a hosted runner (PR #79), so the text substitution an installer does is what runs.
sed -e "s#<string>/Users/YOU/.local/bin/deep-diff-forge</string>#<string>${bin}</string>#" \
  -e "s#<string>/Users/YOU/.local/run</string>#<string>${run}</string>#" \
  -e "s#<string>/Users/YOU/Library/Logs/deep-diff-forge.log</string>#<string>${log}</string>#" \
  "$template" > "$plist"
chmod 644 "$plist"
plutil -lint "$plist"
# Only a value counts: the plist's install comment names the placeholder on purpose.
if grep -n '<string>/Users/YOU' "$plist" >&2; then
  fail "a /Users/YOU placeholder value is left in ${plist} (lines above)"
fi
for pair in "ProgramArguments.0=${bin}" "EnvironmentVariables.XDG_RUNTIME_DIR=${run}" "StandardErrorPath=${log}"; do
  key="${pair%%=*}"
  [ "$(plutil -extract "$key" raw -o - "$plist")" = "${pair#*=}" ] || fail "${key} is not ${pair#*=}"
done

export XDG_RUNTIME_DIR="$run"
sock="$("$bin" daemon path)"
[ "${#sock}" -lt 104 ] || fail "socket path ${sock} is too long for a Unix socket"
! answers || fail "a daemon already answers at ${sock}"

for candidate in "gui/${uid}" "user/${uid}"; do
  if launchctl bootstrap "$candidate" "$plist"; then
    domain="$candidate"
    break
  fi
  echo "launchctl bootstrap into ${candidate} failed; trying the next domain"
done
[ -n "$domain" ] || fail "launchctl bootstrap failed in gui/${uid} and user/${uid}"
echo "loaded ${label} into ${domain} (binary ${bin}, socket ${sock})"

first="$(up_with_launchd_pid)"
echo "1. pid ${first} answers daemon health under launchd"

kill -KILL "$first"
wait_for 30 "launchd to restart the daemon killed by SIGKILL" restarted_from "$first"
second="$(up_with_launchd_pid)"
echo "2. after SIGKILL launchd restarted it as pid ${second}"

"$bin" daemon stop >/dev/null || fail "daemon stop exited $?"
wait_for 10 "pid ${second} to exit after daemon stop" process_gone "$second"
no_socket || fail "${sock} is still there after daemon stop"
# Wait well past ThrottleInterval (2 s): a clean exit (0) must not be restarted.
sleep 10
[ -z "$(job_pid)" ] || fail "launchd restarted the daemon after a clean stop (pid $(job_pid))"
! answers || fail "a daemon answers after daemon stop"
no_socket || fail "${sock} came back after daemon stop"
echo "3. daemon stop: exit 0, socket gone, not restarted after 10 s"

launchctl bootout "${domain}/${label}" || echo "launchctl bootout exited $?"
wait_for 30 "${label} to leave ${domain}" job_gone
echo "4. bootout unloaded the stopped job"

launchctl bootstrap "$domain" "$plist" || fail "the second bootstrap into ${domain} failed"
third="$(up_with_launchd_pid)"
launchctl bootout "${domain}/${label}" || echo "launchctl bootout exited $?"
wait_for 30 "${label} to leave ${domain}" job_gone
wait_for 45 "pid ${third} to exit after bootout" process_gone "$third"
sleep 5
! answers || fail "a daemon answers after bootout"
[ -z "$(pgrep -x deep-diff-forge || true)" ] || fail "a deep-diff-forge process runs after bootout: $(pgrep -x deep-diff-forge)"
# SIGTERM is not handled, so the socket file stays behind; the next start replaces it.
if [ -e "$sock" ]; then
  state="a stale socket left at ${sock}"
else
  state="no socket left"
fi
echo "5. bootout of running pid ${third}: process gone, job unloaded, not restarted (${state})"
domain=""
echo "launchd smoke OK"
