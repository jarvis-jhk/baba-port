#!/usr/bin/env bash
# M7 online test matrix: the real game (baba-port --drive, headless) as one
# OS process per machine against a local warpzone-server, every mode, up to
# 12 players in any mix of machines and local (virtual) controllers, lossy
# links, forced relay, a machine leaving, late join, the host leaving, a
# desync and a version mismatch. docs/net.md "Testing", docs/game-flow.md
# "Online".
#
#   scripts/net/online-test.sh [--release] [case-filter...]
#
# Filters are substrings of case names. Starts scripts/net/dev-server.sh in
# the background (stopped again by PID) unless WARPZONE_SERVER answers.
# Output: scratch/m7/online/<case>/m<i>/ (log.txt, screenshots), data dirs
# in scratch/m7/online/data/. Wrap in perf-mode (up to 3 games at once,
# several minutes).
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# The project environment (game dir, data dir, toolchains) is mandatory.
. "${BABA_ENV:-$ROOT/env.sh}"
profile=debug
build=(cargo build -p baba-port)
if [[ "${1:-}" == "--release" ]]; then
  profile=release
  build+=(--release)
  shift
fi
FILTERS=("$@")
cd "$ROOT"
"${build[@]}" || exit 1
BIN="${CARGO_TARGET_DIR:-$ROOT/target}/$profile/baba-port"
export WARPZONE_SERVER="${WARPZONE_SERVER:-http://127.0.0.1:${BABA_WZ_PORT:-3036}}"
OUT="$ROOT/scratch/m7/online"
mkdir -p "$OUT"
SERVER_PID=""
if ! curl -sf "$WARPZONE_SERVER/api/v1/info" >/dev/null; then
  "$ROOT/scripts/net/dev-server.sh" >"$OUT/server.log" 2>&1 &
  SERVER_PID=$!
  for _ in $(seq 120); do curl -sf "$WARPZONE_SERVER/api/v1/info" >/dev/null && break; sleep 0.5; done
fi
trap '[ -n "$SERVER_PID" ] && kill "$SERVER_PID" 2>/dev/null' EXIT

LOSSY="BABA_NET_DELAY_MS=40 BABA_NET_JITTER_MS=10 BABA_NET_LOSS=5"
# name | script | locals per machine | extra env (all machines) | m<i>-only env
declare -a CASES=(
  "coop-2x1|coop|1,1||"
  "shared-2x1|shared|1,1||"
  "own-2x1|own|1,1||"
  "coop-3x4|coop|4,4,4||"
  "shared-3x4|shared|4,4,4||"
  "own-3x4|own|4,4,4||"
  "coop-2-4-6|coop|2,4,6||"
  "shared-2-4-6|shared|2,4,6||"
  "own-2-4-6|own|2,4,6||"
  "coop-3x4-lossy-predict|coop|4,4,4|BABA_NET_PREDICT=1 $LOSSY|"
  "shared-2-4-6-lossy|shared|2,4,6|$LOSSY|"
  "own-3x2-relay-lossy|own|2,2,2|BABA_NET_FORCE_RELAY=1 $LOSSY|"
  "leave-shared-3x2|leave|2,2,2||"
  "late-join-own-3x2|late|2,2,2||"
  "late-join-coop-3x1|late-coop|1,1,1||"
  "host-leaves-coop-3x2|migrate|2,2,2||"
  "desync-coop-3x1|desync|1,1,1||"
  "mismatch-2x1|mismatch|1,1||m1:BABA_NET_BUILD=baba-port-other"
  "menus-browser-2x1|browse|1,1||"
)

FAILED=()
for c in "${CASES[@]}"; do
  IFS='|' read -r name script locals extra only <<<"$c"
  if [ ${#FILTERS[@]} -gt 0 ]; then
    hit=0
    for f in "${FILTERS[@]}"; do [[ "$name" == *"$f"* ]] && hit=1; done
    [ $hit = 1 ] || continue
  fi
  IFS=',' read -r -a L <<<"$locals"
  n=${#L[@]}
  players=0
  for k in "${L[@]}"; do players=$((players + k)); done
  dir="$OUT/$name"
  rm -rf "$dir" "$OUT/data/$name"
  mkdir -p "$dir"
  echo "== $name ($script: ${locals} players, $n machines) $extra"
  pids=()
  for i in $(seq 0 $((n - 1))); do
    tags="m$i"
    [ "$i" = 0 ] && tags="$tags,host" || tags="$tags,guest"
    mdir="$dir/m$i"
    mkdir -p "$mdir" "$OUT/homes/m$i"
    menv=()
    for kv in $extra; do menv+=("$kv"); done
    if [[ "$only" == "m$i:"* ]]; then menv+=("${only#m$i:}"); fi
    env "${menv[@]}" \
      DRIVE_TAGS="$tags" ROOMFILE="$dir/room.txt" STARTED="$dir/started" \
      LOCALS="${L[$i]}" MACHINES="$n" PLAYERS="$players" \
      WARPZONE_HOME="$OUT/homes/m$i" BABA_IMPORT_FROM="$OUT/no-original-save" \
      BABA_NET_USER="bt$i-$(printf '%04x' $((RANDOM)))" \
      "$BIN" --drive "crates/game/tests/online/$script.drive" --data-dir "$OUT/data/$name/m$i" \
      --out "$mdir" >"$mdir/log.txt" 2>&1 &
    pids+=($!)
    # Machines join in index order (m0 hosts).
    sleep 0.3
  done
  st=0
  for p in "${pids[@]}"; do wait "$p" || st=1; done
  if grep -q "^FAIL" "$dir"/m*/log.txt; then st=1; fi
  for i in $(seq 0 $((n - 1))); do
    grep -E "^(FAIL|ok: online (agreed|resynced|desync)|online: (session ended|hosted))|online bots" "$dir/m$i/log.txt" | sed "s/^/  m$i /"
  done
  if [ $st = 0 ]; then echo "PASS $name"; else echo "FAIL $name"; FAILED+=("$name"); fi
done
echo "screenshots and logs: $OUT"
if [ ${#FAILED[@]} -gt 0 ]; then
  echo "failed: ${FAILED[*]}"
  exit 1
fi
