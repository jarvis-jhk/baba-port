#!/bin/bash
# Scripted headless game-flow test (docs/game-flow.md): runs the .drive
# scripts of crates/game/tests/flow in order on a fresh data directory and
# writes screenshots (and comparisons with captures of the original, when
# present) to scratch/m4/flow/. Usage: scripts/dev/flow-test.sh [--release]
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# The project environment (game dir, data dir, toolchains) is mandatory.
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
out="$root/scratch/m4/flow"
data="$root/scratch/m4/flow-data"
export BABA_REF="${BABA_REF:-$root/scratch/ref/capture}"
# A fresh first launch must not find (and offer to import) a real save of
# the original on this machine: point the import search at nothing.
export BABA_IMPORT_FROM="${BABA_IMPORT_FROM:-$root/scratch/m4/no-original-save}"
export BABA_LEVELCODES_OFFLINE=1
rm -rf "$data" "$out"
mkdir -p "$out"
status=0
for script in crates/game/tests/flow/*.drive; do
    echo "== $script"
    "target/$profile/baba-port" --drive "$script" --data-dir "$data" --out "$out" || status=1
done
echo "screenshots: $out"
exit $status
