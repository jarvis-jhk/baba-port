#!/bin/bash
# M8 headless test (docs/game-flow.md "Level editor"): runs
# crates/game/tests/editor/*.drive in order on a fresh data dir under
# scratch/m8/ (first launch done, no save import) and writes screenshots
# to scratch/m8/shots/.
#
#   1-create        a custom level from scratch, settings, test play to a win,
#                   save / reload / save gives identical files, object list, theme
#   2-tutorial-pad  help, a tutorial lesson, shortcuts, gamepad and touch editing
#   3-pack          a levelpack with a map made in the editor, then played
#   4-compare       editor screens against captures of the original (only when
#                   scratch/m8/ref/ holds them; see docs/game-flow.md)
#
# Usage: scripts/dev/m8-test.sh [--release]
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
m8="$root/scratch/m8"
data="$m8/drive-data"
out="$m8/shots"
export BABA_IMPORT_FROM="$m8/no-import"
export BABA_LEVELCODES_OFFLINE=1
export M8_REF="$m8/ref"
rm -rf "$data" "$out"
mkdir -p "$data" "$out"
printf '[baba]\nfirsttime=1\n\n' >"$data/ba.ba"
status=0
for script in crates/game/tests/editor/*.drive; do
    if [[ "$script" == *compare* && ! -f "$M8_REF/o24-our-level-in-original-editor.png" ]]; then
        echo "== $script skipped (no captures in $M8_REF)"
        continue
    fi
    echo "== $script"
    "target/$profile/baba-port" --drive "$script" --data-dir "$data" --out "$out" || status=1
done
echo "screenshots: $out"
exit $status
