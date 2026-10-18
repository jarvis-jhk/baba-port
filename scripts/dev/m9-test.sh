#!/bin/bash
# M9 headless test (docs/game-flow.md "M9"): save import, Play levels,
# level codes, packs with Lua. Runs crates/game/tests/m9/*.drive in order on
# a fresh data dir under scratch/m9/ and writes screenshots to
# scratch/m9/shots/.
#
# Inputs (developer machine only, never committed):
#   scratch/m9/orig-save/      a COPY of an original save (0ba.ba ... SettingsC.txt)
#   scratch/m9/server-cache/   recorded level-server responses (examples/levelcodes.rs);
#                              the run is offline (BABA_LEVELCODES_OFFLINE=1)
# The data dir is seeded with the install's baba/0level as a custom level and
# its debug pack (which ships Lua) as a user pack.
# Usage: scripts/dev/m9-test.sh [--release]
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
. "${BABA_ENV:-$here/env.sh}"
profile=debug
build=(cargo build -p baba-port)
if [[ "${1:-}" == "--release" ]]; then
    profile=release
    build+=(--release)
fi
cd "$here"
"${build[@]}"
root="${BABA_ROOT:-$here}"
m9="$root/scratch/m9"
data="$m9/drive-data"
out="$m9/shots"
export BABA_IMPORT_FROM="$m9/orig-save"
export BABA_LEVELCODES_CACHE="$m9/server-cache"
export BABA_LEVELCODES_OFFLINE=1
[[ -f "$BABA_IMPORT_FROM/2ba.ba" ]] || { echo "missing $BABA_IMPORT_FROM (copy of an original save)" >&2; exit 2; }
rm -rf "$data" "$out"
mkdir -p "$data/worlds/levels" "$out"
cp "$BABA_GAME_DIR/Data/Worlds/baba/0level.l" "$BABA_GAME_DIR/Data/Worlds/baba/0level.ld" "$data/worlds/levels/"
cp -r "$BABA_GAME_DIR/Data/Worlds/debug" "$data/worlds/testpack"
status=0
for script in crates/game/tests/m9/*.drive; do
    echo "== $script"
    "target/$profile/baba-port" --drive "$script" --data-dir "$data" --out "$out" || status=1
done
echo "screenshots: $out"
exit $status
