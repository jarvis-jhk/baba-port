#!/bin/bash
# Builds dist/baba-port-<version>-linux-x86_64.tar.gz.
#
# Layout (one top-level folder; the Warpzone descriptor's
# exec = "baba-port-*/play.sh"):
#   baba-port-<version>/{baba-port, play.sh, README.md, LICENSE-MIT,
#     LICENSE-APACHE, THIRD_PARTY_NOTICES.md, licenses/RUST_DEPENDENCIES.txt,
#     [lib/libSDL2-2.0.so.0, licenses/SDL2-LICENSE.txt]}
#
# Usage: scripts/package/linux.sh [--no-build] [--bundle-sdl]
#   --no-build    reuse target/dist/baba-port
#   --bundle-sdl  ship $BABA_SDL2_LIB_DIR/libSDL2-2.0.so.0 in lib/ (play.sh
#                 puts it first). CI builds a self-contained SDL2 2.32.10 for
#                 this (scripts/ci/build-sdl2-linux.sh: X11/Wayland/audio are
#                 loaded at run time). Without it the system's SDL2 (2.0.14+)
#                 is used; never bundle a distro's libSDL2 (it links dozens of
#                 that distro's libraries).
# Env: BABA_MAX_GLIBC (default 2.35: Ubuntu 22.04, Debian 12, SteamOS 3);
#      only a warning outside CI, since local builds run on a newer glibc.
set -euo pipefail
source "$(dirname "$0")/../../env.sh"
cd "$BABA_ROOT"
source scripts/package/common.sh

BUILD=1 BUNDLE=0
for a in "$@"; do
    case $a in
        --no-build) BUILD=0 ;;
        --bundle-sdl) BUNDLE=1 ;;
        *) echo "unknown option $a" >&2; exit 2 ;;
    esac
done
VERSION=$(pkg_version)
NAME="baba-port-$VERSION"
OUT="$DIST/$NAME-linux-x86_64.tar.gz"
STAGE="$DIST/.stage-linux"

if [ $BUILD = 1 ]; then
    cargo build --profile dist ${CI:+--locked} -p baba-port
fi
BIN=target/dist/baba-port
[ -x "$BIN" ] || { echo "$BIN missing" >&2; exit 1; }
if ! pkg_check_glibc "$BIN" "${BABA_MAX_GLIBC:-2.35}"; then
    [ -n "${CI:-}" ] && exit 1
    echo "warning: local build; release builds come from CI (ubuntu-22.04)" >&2
fi

pkg_clean_stage "$STAGE"
P="$STAGE/$NAME"
mkdir -p "$P"
install -m 755 "$BIN" "$P/baba-port"
strip --strip-all "$P/baba-port" 2>/dev/null || true
install -m 755 packaging/linux/play.sh "$P/play.sh"
pkg_stage_docs "$P" x86_64-unknown-linux-gnu
if [ $BUNDLE = 1 ]; then
    SDL="${BABA_SDL2_LIB_DIR:?--bundle-sdl needs BABA_SDL2_LIB_DIR}/libSDL2-2.0.so.0"
    [ -f "$SDL" ] || { echo "$SDL missing" >&2; exit 1; }
    mkdir -p "$P/lib"
    install -m 755 "$(readlink -f "$SDL")" "$P/lib/libSDL2-2.0.so.0"
    install -m 644 packaging/licenses/SDL2-LICENSE.txt "$P/licenses/SDL2-LICENSE.txt"
fi
mkdir -p "$DIST"
pkg_tar "$STAGE" "$NAME" "$OUT"
rm -rf "$STAGE"
echo "$OUT ($(human "$OUT"))"
