#!/bin/sh
# Starts baba-port. Run it from a file manager or a terminal, add it to Steam
# as a non-Steam game (Steam Deck), or let the Warpzone launcher start it.
# Every argument is passed on to the game (`./play.sh --help`).
#
# A lib/ folder next to this script (the release build bundles SDL2 there)
# is searched before the system's libraries.
here=$(dirname "$(readlink -f "$0")")
if [ -d "$here/lib" ]; then
    LD_LIBRARY_PATH="$here/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
    export LD_LIBRARY_PATH
fi

# A missing SDL2 would otherwise only show up as a dynamic-linker line.
if command -v ldd >/dev/null 2>&1 && ldd "$here/baba-port" 2>/dev/null | grep -q 'libSDL2.* not found'; then
    cat >&2 <<'MSG'
baba-port needs the SDL2 library (libSDL2-2.0.so.0), which this system lacks.
Install it with your package manager, for example:
    Debian / Ubuntu / Mint:  sudo apt install libsdl2-2.0-0
    Fedora:                  sudo dnf install SDL2
    Arch:                    sudo pacman -S sdl2
MSG
    exit 1
fi

exec "$here/baba-port" "$@"
