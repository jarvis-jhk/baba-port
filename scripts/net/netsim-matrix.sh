#!/usr/bin/env bash
# baba-net multi-process test matrix against a local warpzone-server: one OS
# process per machine (crates/net/examples/netsim.rs), the toy game, bots.
#
#   scripts/net/netsim-matrix.sh [secs] [case-filter...]   # default 20 s sessions
#
# Filters are substrings of case names; a case runs if it matches any.
# Starts scripts/net/dev-server.sh in the background (and stops it again, by
# PID) unless WARPZONE_SERVER already answers. NETSIM=<binary> skips the
# build. Logs: scratch/net/<case>.log, summary on stdout.
# Wrap in perf-mode: up to 13 processes for several minutes.
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source "$ROOT/env.sh"
SECS="${1:-20}"
shift || true
FILTERS=("$@")
export WARPZONE_SERVER="${WARPZONE_SERVER:-http://127.0.0.1:${BABA_WZ_PORT:-3036}}"
OUT="$ROOT/scratch/net"
mkdir -p "$OUT"
SERVER_PID=""
if ! curl -sf "$WARPZONE_SERVER/api/v1/info" >/dev/null; then
  "$ROOT/scripts/net/dev-server.sh" >"$OUT/server.log" 2>&1 &
  SERVER_PID=$!
  for _ in $(seq 100); do curl -sf "$WARPZONE_SERVER/api/v1/info" >/dev/null && break; sleep 0.5; done
fi
trap '[ -n "$SERVER_PID" ] && kill "$SERVER_PID" 2>/dev/null' EXIT
BIN="${NETSIM:-$ROOT/target/debug/examples/netsim}"
if [ -z "${NETSIM:-}" ] && ! (cd "$ROOT" && cargo build -q -p baba-net --example netsim); then
  [ -x "$BIN" ] || exit 1
  echo "warning: build failed, using the existing $BIN" >&2
fi

L1="--delay 40 --jitter 10 --loss 5"     # ~80 ms round trip, 5 % loss
L2="--delay 75 --jitter 25 --loss 3"     # ~150 ms round trip
declare -a CASES=(
  "12x1-coop|--locals 12x1 --mode coop"
  "4x3-shared|--locals 4x3 --mode shared"
  "2-4-6-own|--locals 2,4,6 --mode own"
  "4x3-coop-rtt80-loss5|--locals 4x3 --mode coop $L1"
  "12x1-own-rtt150-loss3|--locals 12x1 --mode own $L2"
  "2-4-6-shared-rtt80-loss5|--locals 2,4,6 --mode shared $L1"
  "4x3-relay-spec|--locals 4x3 --mode shared --relay --spectators 1"
  "2-4-6-coop-relay-loss|--locals 2,4,6 --mode coop --relay $L1"
  "public-2-1|--locals 2,1 --mode coop --public"
  "6x2-leave-crash|--locals 6x2 --mode shared $L1 --leave 2@4 --crash 4@7"
  "4x1-late-join-spec|--locals 4x1 --mode coop --spectators 1 --late 3@5 --late 4@6"
  "3x2-own-late-join|--locals 3x2 --mode own $L1 --late 2@6"
  "host-leaves-coop|--locals 4x2 --mode coop $L1 --leave 0@5"
  "host-leaves-own-relay|--locals 2,3,1 --mode own --relay --leave 0@5"
  "host-crashes-shared|--locals 3x2 --mode shared --crash 0@5"
  "3x1-desync|--locals 3x1 --mode coop $L1 --desync 1@4"
  "3x2-desync-abort|--locals 3x2 --mode own --desync 2@4 --abort-on-desync"
)
FAIL=0
for c in "${CASES[@]}"; do
  name="${c%%|*}"; args="${c#*|}"
  if [ ${#FILTERS[@]} -gt 0 ]; then
    hit=0
    for f in "${FILTERS[@]}"; do [[ "$name" == *"$f"* ]] && hit=1; done
    [ $hit = 1 ] || continue
  fi
  secs="$SECS"
  # A crashed host is noticed by Warpzone's 15 s peer time-out.
  [[ "$name" == host-crashes* ]] && secs=$((SECS + 25))
  # shellcheck disable=SC2086
  line="$("$BIN" run $args --secs "$secs" --tag "$name" 2>"$OUT/$name.err" | grep '^SUMMARY' || echo "SUMMARY FAIL $name (no summary)")"
  printf '%-30s %s\n' "$name" "$line"
  [[ "$line" == "SUMMARY PASS"* ]] || FAIL=1
done
exit $FAIL
