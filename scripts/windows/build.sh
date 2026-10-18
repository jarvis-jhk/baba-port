#!/bin/bash
# Cross-compiles for x86_64 Windows (x86_64-pc-windows-gnu with llvm-mingw,
# as in bopl-port) and stages the exe with SDL2.dll in target/dist/windows/.
# Usage: scripts/windows/build.sh [--probe] [--debug]
#   (default)  the game (crates/game, profile dist: size-optimised, stripped) -> baba-port.exe
#   --probe    the SDL probe (probes/sdl) -> baba-sdl-probe.exe
set -euo pipefail
source "$(dirname "$0")/../../env.sh"
cd "$BABA_ROOT"
[ -x tools/windows/llvm-mingw/bin/x86_64-w64-mingw32-clang ] || scripts/windows/fetch-toolchain.sh
source scripts/windows/env.sh
PROFILE=dist; FLAG="--profile dist"; PROBE=0
for a in "$@"; do
    case $a in
        --debug) PROFILE=debug; FLAG= ;;
        --probe) PROBE=1 ;;
        *) echo "unknown option $a" >&2; exit 2 ;;
    esac
done
T=x86_64-pc-windows-gnu
if [ $PROBE = 1 ]; then
    [ $PROFILE = dist ] && { PROFILE=release; FLAG=--release; }
    cargo build $FLAG --target $T --target-dir "$BABA_WIN_TARGET_DIR" --manifest-path probes/sdl/Cargo.toml
    EXE=baba-sdl-probe.exe
else
    cargo build $FLAG --target $T --target-dir "$BABA_WIN_TARGET_DIR" -p baba-port
    EXE=baba-port.exe
fi
OUT=target/dist/windows
mkdir -p "$OUT"
cp -f "$BABA_WIN_TARGET_DIR/$T/$PROFILE/$EXE" "$OUT/"
cp -f tools/windows/sdl2/x86_64-w64-mingw32/bin/SDL2.dll "$OUT/"
ls -la "$OUT"
