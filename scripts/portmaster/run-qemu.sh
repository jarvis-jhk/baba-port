#!/bin/bash
# Runs an aarch64 Linux binary headless under qemu-user against the Debian 12
# arm64 test rootfs (scripts/portmaster/fetch-toolchain.sh --qemu): SDL2's
# offscreen video driver + Mesa llvmpipe through EGL (surfaceless) give a
# software GLES2 context with no display; audio goes to SDL's dummy driver.
# Nothing is written outside the project (HOME/XDG point into tools/).
#
# Usage: scripts/portmaster/run-qemu.sh [binary] [args...]
#   default: the SDL probe with --headless --virtual-pad --shot scratch/portmaster/qemu.ppm
set -euo pipefail
source "$(dirname "$0")/../../env.sh"
cd "$BABA_ROOT"
P=tools/portmaster
[ -x $P/qemu-aarch64-static ] && [ -f $P/rootfs/.done ] || scripts/portmaster/fetch-toolchain.sh --qemu
mkdir -p scratch/portmaster tools/portmaster/home
if [ $# = 0 ]; then
    set -- target/probes/aarch64-unknown-linux-gnu/release/baba-sdl-probe \
        --headless --virtual-pad --shot scratch/portmaster/qemu.ppm
fi
R=$BABA_ROOT/$P/rootfs
exec env -i PATH=/usr/bin:/bin HOME="$BABA_ROOT/$P/home" XDG_RUNTIME_DIR="$BABA_ROOT/$P/home" \
    LD_LIBRARY_PATH=/usr/lib/aarch64-linux-gnu:/lib/aarch64-linux-gnu \
    LIBGL_DRIVERS_PATH=/usr/lib/aarch64-linux-gnu/dri \
    __EGL_VENDOR_LIBRARY_DIRS="$R/usr/share/glvnd/egl_vendor.d" \
    EGL_PLATFORM=surfaceless GALLIUM_DRIVER=llvmpipe LIBGL_ALWAYS_SOFTWARE=1 \
    SDL_VIDEODRIVER="${SDL_VIDEODRIVER:-offscreen}" SDL_AUDIODRIVER="${SDL_AUDIODRIVER:-dummy}" \
    "$P/qemu-aarch64-static" -L "$R" "$@"
