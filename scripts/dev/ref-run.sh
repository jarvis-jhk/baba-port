#!/bin/bash
# Run the original game (our copy in tools/baba-original) headless on the private Xvfb.
#
#   scripts/dev/ref-run.sh start            start Xvfb (:68) + the game, offline (no Steam)
#   scripts/dev/ref-run.sh stop             stop the game (by recorded PID)
#   scripts/dev/ref-run.sh status
#   scripts/dev/ref-run.sh shot NAME        screenshot of the display -> scratch/ref/NAME.png
#   scripts/dev/ref-run.sh key KEY...       send key presses (xdotool keysyms, e.g. Right z Return)
#                                           a token "+N" sleeps N/10 s between keys
#   scripts/dev/ref-run.sh keys "RRUL..."   turn string: U/D/L/R arrows, W=space (wait), Z=undo (z),
#                                           E=Return, X=Escape, r=restart (r), '.'=0.3 s pause
#   scripts/dev/ref-run.sh click X Y [B]    mouse click at window pixel (X, Y), button B (1)
#   scripts/dev/ref-run.sh reset-save       wipe the private save (tools/baba-home)
#
# Several agents may run the original at once: REF_RUN / REF_HOME / REF_OUT pick another run
# tree, home and output/PID directory (defaults tools/baba-run, tools/baba-home, scratch/ref).
# REF_LEVELS=1 builds the run tree with a private, writable copy of Data/Worlds/levels (the
# custom levels world) so levels made by baba-port's editor can be dropped in; the rest of
# Data stays a set of links to tools/baba-original.
#
# Offline: the game is run from tools/baba-run/, a tree of symlinks to tools/baba-original
# plus a copy of the Chowdren binary, WITHOUT libsteam_api.so. Chowdren dlopen()s the Steam
# library lazily; when it is missing it logs "Could not load Steam DLL" and runs normally, so
# the Steam client on the desktop is never contacted. HOME/XDG_* point into tools/baba-home,
# audio goes to SDL's dummy driver, the session bus / runtime dir are hidden. The session-bus
# address points at a dead socket on purpose: with it unset, libdbus "autolaunches" a private
# bus on :68 which then starts portals, at-spi, gvfsd and FUSE mounts.
set -euo pipefail
R="$(cd "$(dirname "$0")/../.." && pwd)"
ORIG="$R/tools/baba-original" RUN="${REF_RUN:-$R/tools/baba-run}" H="${REF_HOME:-$R/tools/baba-home}"
# REF_TRACE=1: run tools/baba-trace instead, whose Data/debug.lua additionally loads the private
# in-game tracer docs/spec/oracle/ingame_trace.lua (writes trace.log / state.jsonl in that dir).
[ "${REF_TRACE:-0}" = 1 ] && RUN="$R/tools/baba-trace"
OUT="${REF_OUT:-$R/scratch/ref}"
PIDF="$OUT/game.pid" LOG="$OUT/game.log"
D="${REF_DISPLAY:-:68}"
mkdir -p "$OUT"
export DISPLAY="$D"

setup_run_tree() {
  mkdir -p "$RUN/bin64"
  [ -x "$RUN/bin64/Chowdren" ] || cp "$ORIG/bin64/Chowdren" "$RUN/bin64/Chowdren"
  for f in Assets.dat gamecontrollerdb.txt icon.bmp; do
    [ -e "$RUN/$f" ] || ln -s "$ORIG/$f" "$RUN/$f"
  done
  if [ "${REF_TRACE:-0}" = 1 ]; then
    mkdir -p "$RUN/Data"
    for f in "$ORIG"/Data/*; do
      b=${f##*/}; [ "$b" = debug.lua ] || [ -e "$RUN/Data/$b" ] || ln -s "$f" "$RUN/Data/$b"
    done
    { cat "$ORIG/Data/debug.lua"; printf '\ndofile("Data/oracle_trace.lua")\n'; } >"$RUN/Data/debug.lua"
    ln -sfn "$R/docs/spec/oracle/ingame_trace.lua" "$RUN/Data/oracle_trace.lua"
  elif [ "${REF_LEVELS:-0}" = 1 ]; then
    mkdir -p "$RUN/Data/Worlds"
    for f in "$ORIG"/Data/*; do
      b=${f##*/}; [ "$b" = Worlds ] || [ -e "$RUN/Data/$b" ] || ln -s "$f" "$RUN/Data/$b"
    done
    for f in "$ORIG"/Data/Worlds/*; do
      b=${f##*/}; [ "$b" = levels ] || [ -e "$RUN/Data/Worlds/$b" ] || ln -s "$f" "$RUN/Data/Worlds/$b"
    done
    [ -d "$RUN/Data/Worlds/levels" ] || cp -r "$ORIG/Data/Worlds/levels" "$RUN/Data/Worlds/levels"
  else
    [ -e "$RUN/Data" ] || ln -s "$ORIG/Data" "$RUN/Data"
  fi
  rm -f "$RUN/bin64/libsteam_api.so"   # offline: never load the Steam API
}

game_pid() { [ -f "$PIDF" ] && kill -0 "$(cat "$PIDF")" 2>/dev/null && cat "$PIDF"; }

win_id() {
  local p; p=$(game_pid) || return 1
  xdotool search --pid "$p" 2>/dev/null | tail -1
}

# The game polls input once per frame (llvmpipe: slow frames), so hold each key a bit.
press() { xdotool keydown "$1"; sleep "${REF_KEY_HOLD:-0.12}"; xdotool keyup "$1"; sleep "${REF_KEY_GAP:-0.25}"; }

focus() { local w; w=$(win_id) && [ -n "$w" ] && xdotool windowfocus --sync "$w" 2>/dev/null || true; }

cmd=${1:-status}; shift || true
case "$cmd" in
  start)
    if game_pid >/dev/null; then echo "already running: $(cat "$PIDF")"; exit 0; fi
    "$R/scripts/dev/xvfb.sh" "$D" 1920x1080x24 >/dev/null
    setup_run_tree
    mkdir -p "$H"/{.local/share,.config,.cache,run}
    chmod 700 "$H/run"
    S="$H/.local/share/Baba_Is_You/SettingsC.txt"   # windowed: fullscreen on a WM-less Xvfb crops
    mkdir -p "${S%/*}"
    if [ -f "$S" ]; then sed -i 's/^fullscreen=1/fullscreen=0/' "$S"; else printf '[settings]\nfullscreen=0\n' >"$S"; fi
    # no "are you sure you want to restart?" prompt (scripted restarts are one key press)
    grep -q '^restartask=' "$S" || sed -i 's/^\[settings\]$/[settings]\nrestartask=1/' "$S"
    # REF_QUEUE=1level,2level: levels the traced build's map "enter" jumps to (ingame_trace.lua)
    sed -i '/^\[oracle\]$/,/^queue=/d' "$S"
    [ -n "${REF_QUEUE:-}" ] && printf '[oracle]\nqueue=%s\n' "$REF_QUEUE" >>"$S"
    # REF_WORLD=new_adv: world that "continue playing" opens
    [ -n "${REF_WORLD:-}" ] && sed -i "s/^world=.*/world=$REF_WORLD/" "$S"
    (
      cd "$RUN"
      exec env -i PATH=/usr/bin:/bin DISPLAY="$D" HOME="$H" \
        XDG_DATA_HOME="$H/.local/share" XDG_CONFIG_HOME="$H/.config" XDG_CACHE_HOME="$H/.cache" \
        XDG_RUNTIME_DIR="$H/run" TMPDIR="$H/run" LANG=C.UTF-8 \
        DBUS_SESSION_BUS_ADDRESS="unix:path=$H/run/no-such-bus" NO_AT_BRIDGE=1 GTK_USE_PORTAL=0 \
        GIO_USE_VFS=local GVFS_DISABLE_FUSE=1 \
        SDL_AUDIODRIVER=dummy SDL_VIDEODRIVER=x11 LIBGL_ALWAYS_SOFTWARE=1 \
        ./bin64/Chowdren "$@"
    ) >"$LOG" 2>&1 &
    echo $! >"$PIDF"
    for _ in $(seq 100); do [ -n "$(win_id || true)" ] && break; sleep 0.1; done
    echo "started pid $(cat "$PIDF") window $(win_id || echo none) (log $LOG)"
    ;;
  stop)
    if p=$(game_pid); then kill "$p"; for _ in $(seq 30); do kill -0 "$p" 2>/dev/null || break; sleep 0.1; done
      kill -0 "$p" 2>/dev/null && kill -9 "$p"; echo "stopped $p"; else echo "not running"; fi
    rm -f "$PIDF"
    ;;
  status) if p=$(game_pid); then echo "running pid $p window $(win_id)"; else echo "not running"; fi ;;
  shot)
    n=${1:?name}; w=$(win_id || true)
    import -display "$D" -window "${w:-root}" "$OUT/$n.png"; echo "$OUT/$n.png" ;;
  key)
    focus
    for k in "$@"; do
      case "$k" in +*) sleep "$(echo "${k#+} / 10" | bc -l)";; *) press "$k";; esac
    done ;;
  keys)
    focus
    s=${1:?turn string}
    for ((i = 0; i < ${#s}; i++)); do
      c=${s:i:1}
      case "$c" in
        U) k=Up;; D) k=Down;; L) k=Left;; R) k=Right;; W) k=space;; Z) k=z;; E) k=Return;; X) k=Escape;; r) k=r;;
        .) sleep 0.3; continue;; *) echo "bad turn char $c" >&2; exit 2;;
      esac
      press "$k"
    done ;;
  click)
    # click X Y [BUTTON]: mouse click at window pixel (X, Y)
    focus; w=$(win_id)
    xdotool mousemove --window "$w" "${1:?x}" "${2:?y}"; sleep 0.15
    xdotool mousedown "${3:-1}"; sleep "${REF_KEY_HOLD:-0.12}"; xdotool mouseup "${3:-1}"; sleep "${REF_KEY_GAP:-0.25}" ;;
  reset-save)
    game_pid >/dev/null && { echo "stop the game first" >&2; exit 1; }
    rm -rf "$H"; echo "wiped $H" ;;
  *) sed -n '2,15p' "$0"; exit 2 ;;
esac
