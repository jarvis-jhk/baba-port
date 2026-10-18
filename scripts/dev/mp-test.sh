#!/bin/bash
# Scripted headless multiplayer test (M6, docs/game-flow.md "Multiplayer"):
# runs crates/game/tests/mp/*.drive (or the scripts given), each on a fresh
# data directory, with SDL virtual controllers (input subsystems only, no
# window), and writes screenshots and logs to scratch/m6/mp/<script>/.
# Usage: scripts/dev/mp-test.sh [--release] [script.drive ...]
set -uo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# The project environment (game dir, data dir, toolchains) is mandatory.
. "${BABA_ENV:-$here/env.sh}"
profile=debug
build=(cargo build -p baba-port)
if [[ "${1:-}" == "--release" ]]; then
    profile=release
    build+=(--release)
    shift
fi
cd "$here"
"${build[@]}" || exit 1
root="${BABA_ROOT:-$here}/scratch/m6/mp"
# No original save: a real one on the developer machine would make the
# first launch offer an import (M9) instead of starting the base world.
export BABA_IMPORT_FROM="${BABA_IMPORT_FROM:-$root/no-original-save}"
scripts=("$@")
if [[ ${#scripts[@]} -eq 0 ]]; then
    scripts=(crates/game/tests/mp/*.drive)
fi
status=0
for script in "${scripts[@]}"; do
    name=$(basename "$script" .drive)
    out="$root/$name"
    data="$root/data/$name"
    rm -rf "$data" "$out"
    mkdir -p "$out"
    echo "== $script"
    if ! "target/$profile/baba-port" --drive "$script" --data-dir "$data" --out "$out" \
        > "$out/log.txt" 2>&1; then
        status=1
    fi
    grep -E "^(FAIL|drive:)" "$out/log.txt"
done
echo "screenshots and logs: $root"
exit $status
