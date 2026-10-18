#!/bin/bash
# Builds SDL2 from the pinned upstream source release into tools/sdl2-linux/
# for the Linux release tarball (CI, ubuntu-22.04; adapted from
# towerfall-port). The runner's system SDL2 (2.0.20) is older than the
# Windows/macOS one; a distro's libSDL2 links dozens of that distro's
# libraries, this one only libc/libm: X11, Wayland, PulseAudio, PipeWire,
# ALSA, KMSDRM, EGL are loaded at run time (SDL's dynamic loading), so the
# library runs on systems without any of them. Same version as SDL2.dll.
# Needs the -dev packages listed in .github/workflows/build.yml.
# Then: export BABA_SDL2_LIB_DIR="$PWD/tools/sdl2-linux/lib" (linking and
# scripts/package/linux.sh --bundle-sdl).
set -euo pipefail
cd "$(dirname "$0")/../.."

VER=2.32.10
FILE="SDL2-$VER.tar.gz"
URL="https://github.com/libsdl-org/SDL/releases/download/release-$VER/$FILE"
SHA256=5f5993c530f084535c65a6879e9b26ad441169b3e25d789d83287040a9ca5165
PREFIX="$PWD/tools/sdl2-linux"
DL=tools/sdl2-linux-src

if [ -f "$PREFIX/.version" ] && [ "$(cat "$PREFIX/.version")" = "$VER" ]; then
    echo "SDL2 $VER already in $PREFIX"; exit 0
fi
mkdir -p "$DL"
if [ ! -f "$DL/$FILE" ] || ! echo "$SHA256  $DL/$FILE" | sha256sum -c --status; then
    curl -sSfL --retry 3 -o "$DL/$FILE.part" "$URL"
    echo "$SHA256  $DL/$FILE.part" | sha256sum -c --status || { echo "SHA-256 mismatch for $FILE" >&2; exit 1; }
    mv "$DL/$FILE.part" "$DL/$FILE"
fi
rm -rf "$DL/build" && mkdir -p "$DL/build"
tar -xzf "$DL/$FILE" -C "$DL/build" --no-same-owner
cd "$DL/build/SDL2-$VER"
./configure --prefix="$PREFIX" --disable-static --enable-shared --disable-rpath --quiet
make -j"$(nproc)" >/dev/null
rm -rf "$PREFIX"
make install >/dev/null
strip --strip-unneeded "$PREFIX"/lib/libSDL2-2.0.so.0.*
echo "$VER" > "$PREFIX/.version"
echo "SDL2 $VER built into $PREFIX"
ls -l "$PREFIX/lib"/libSDL2*
# Only libc/libm (+ the loader) may be hard dependencies.
readelf -d "$PREFIX/lib/libSDL2-2.0.so.0" | grep NEEDED
