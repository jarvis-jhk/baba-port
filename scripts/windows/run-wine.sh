#!/bin/bash
# Runs a cross-compiled Windows exe under Proton's Wine (adapted from
# bopl-port's run-windows.sh), sandboxed so nothing outside the project
# directory can be written and nothing is contacted:
#   - bubblewrap mounts / read-only (the game install included), the project
#     read-write, a private /tmp (tools/wine-tmp; wineserver's socket) and a
#     private /dev/shm;
#   - its own network namespace (BABA_WINE_NET=1 keeps the network);
#   - HOME/XDG dirs and the prefix (WINEPREFIX=tools/wineprefix, env.sh) live
#     in tools/; Proton's wine binary is run directly (no Steam, nothing
#     written into the Proton install).
# Host audio is cut off (no winepulse/winealsa, no pulse socket): nothing
# can play on the user's speakers.
# Display: Wine's X11 driver on a private Xvfb (scripts/dev/xvfb.sh, :69 by
# default; never the user's desktop). OpenGL goes Wine opengl32 -> GLX ->
# Mesa llvmpipe on the Xvfb. Audio: SDL's dummy driver unless
# SDL_AUDIODRIVER says otherwise (as a hint, see sdl_env below).
#
# Usage: scripts/windows/run-wine.sh [args...]
#   default exe: target/dist/windows/baba-sdl-probe.exe (EXE=... for another)
#   e.g. scripts/windows/run-wine.sh --frames 30 --virtual-pad --shot scratch/windows/wine.ppm
# Env: PROTON_DIR (default Steam's "Proton - Experimental"), WINEDEBUG
# (default -all), BABA_WINE_DISPLAY (default :69).
set -euo pipefail
_caller_data_dir=${BABA_PORT_DATA_DIR:-}
source "$(dirname "$0")/../../env.sh"
[ -n "$_caller_data_dir" ] && export BABA_PORT_DATA_DIR="$_caller_data_dir"
cd "$BABA_ROOT"

PROTON_DIR=${PROTON_DIR:-"$HOME/.steam/steam/steamapps/common/Proton - Experimental"}
WINE="$PROTON_DIR/files/bin/wine"
EXE=${EXE:-target/dist/windows/baba-sdl-probe.exe}
[ -x "$WINE" ] || { echo "Proton wine not found at $WINE" >&2; exit 1; }
[ -f "$EXE" ] || [[ "$EXE" == [A-Za-z]:* ]] || { echo "$EXE missing: run scripts/windows/build.sh" >&2; exit 1; }

DISPLAY_NUM=$(scripts/dev/xvfb.sh "${BABA_WINE_DISPLAY:-:69}" 1280x800x24)

T="$BABA_ROOT/tools"
mkdir -p "$T/wine-home" "$T/wine-tmp" "$T/wine-xdg/config" "$T/wine-xdg/data" "$WINEPREFIX"

winpath() { printf 'Z:%s' "${1//\//\\}"; }
game_env=(--unsetenv BABA_GAME_DIR)
[ -d "${BABA_GAME_DIR:-}" ] && game_env+=(--setenv BABA_GAME_DIR "$(winpath "$BABA_GAME_DIR")")
[ -n "${BABA_PORT_DATA_DIR:-}" ] && game_env+=(--setenv BABA_PORT_DATA_DIR "$(winpath "$(readlink -m "$BABA_PORT_DATA_DIR")")")
# Proton's Wine drops SDL_* variables; pass them as BABA_SDL_* (the probe and
# the game turn those into SDL hints).
sdl_env=(--setenv BABA_SDL_AUDIODRIVER "${SDL_AUDIODRIVER:-dummy}")
while IFS='=' read -r k v; do
    [ "$k" = SDL_AUDIODRIVER ] || sdl_env+=(--setenv "BABA_$k" "$v")
done < <(env | grep '^SDL_' || true)
net_args=(--unshare-net)
[ "${BABA_WINE_NET:-0}" = 1 ] && net_args=()

sandbox() {
    bwrap \
        --ro-bind / / \
        --dev-bind /dev /dev \
        --tmpfs /dev/shm \
        --proc /proc \
        "${net_args[@]}" \
        --bind "$BABA_ROOT" "$BABA_ROOT" \
        --bind "$T/wine-tmp" /tmp \
        --ro-bind "/tmp/.X11-unix/X${DISPLAY_NUM#:}" "/tmp/.X11-unix/X${DISPLAY_NUM#:}" \
        --setenv DISPLAY "$DISPLAY_NUM" \
        --unsetenv WAYLAND_DISPLAY \
        --setenv HOME "$T/wine-home" \
        --setenv XDG_CONFIG_HOME "$T/wine-xdg/config" \
        --setenv XDG_DATA_HOME "$T/wine-xdg/data" \
        --setenv XDG_CACHE_HOME "$XDG_CACHE_HOME" \
        --setenv WINEPREFIX "$WINEPREFIX" \
        --setenv WINEDLLPATH "$PROTON_DIR/files/lib/wine" \
        --setenv LD_LIBRARY_PATH "$PROTON_DIR/files/lib/x86_64-linux-gnu:$PROTON_DIR/files/lib/i386-linux-gnu" \
        --setenv WINEDLLOVERRIDES "winemenubuilder.exe=d;mscoree=d;mshtml=d;winepulse.drv=d;winealsa.drv=d" \
        --unsetenv XDG_RUNTIME_DIR --setenv PULSE_SERVER /nonexistent \
        --setenv WINEDEBUG "${WINEDEBUG:--all}" \
        --setenv LIBGL_ALWAYS_SOFTWARE 1 \
        "${sdl_env[@]}" \
        "${game_env[@]}" \
        --chdir "$BABA_ROOT" \
        "$WINE" "$@"
}

# First run creates the prefix.
if [ ! -f "$WINEPREFIX/system.reg" ]; then
    sandbox wineboot -u
fi
sandbox "$EXE" "$@"
