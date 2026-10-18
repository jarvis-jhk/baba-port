#!/bin/bash
# Installs the desktop development tools into ./tools (nothing outside the
# project). Idempotent. Android, Windows and PortMaster toolchains have their
# own fetch scripts under scripts/<platform>/.
#
#   scripts/bootstrap-tools.sh
set -euo pipefail
R="$(cd "$(dirname "$0")/.." && pwd)"
source "$R/env.sh"
cd "$R"
mkdir -p tools/lib tools/debs

# Rust (rustup + cargo under tools/rustup, tools/cargo; see env.sh).
if [ ! -x tools/cargo/bin/cargo ]; then
  curl -sSf https://sh.rustup.rs | sh -s -- -y --no-modify-path --profile minimal \
    --default-toolchain stable -c rustfmt,clippy
fi

# SDL2 (crates/platform links -lSDL2 from tools/lib, see its build.rs). Link
# against the system runtime library when present (no -dev package needed);
# otherwise unpack Ubuntu's runtime package into tools/ and use that (then
# run with LD_LIBRARY_PATH=tools/lib).
SYS_SDL=/usr/lib/x86_64-linux-gnu/libSDL2-2.0.so.0
if [ -e "$SYS_SDL" ]; then
  ln -sfn "$SYS_SDL" tools/lib/libSDL2.so
elif [ ! -e tools/lib/libSDL2-2.0.so.0 ]; then
  (cd tools/debs && apt-get download libsdl2-2.0-0 >/dev/null)
  rm -rf tools/sdl2-root && dpkg -x tools/debs/libsdl2-2.0-0_*.deb tools/sdl2-root
  so=$(ls tools/sdl2-root/usr/lib/*/libSDL2-2.0.so.0 | head -1)
  ln -sfn "$R/$so" tools/lib/libSDL2-2.0.so.0
  ln -sfn "$R/$so" tools/lib/libSDL2.so
  echo "note: using SDL2 from tools/sdl2-root; run with LD_LIBRARY_PATH=$R/tools/lib"
fi

# Private Xvfb (unpacked into tools/xvfb-root by the script on first use).
scripts/dev/xvfb.sh >/dev/null && echo "Xvfb ready on :68"

# Headless GL needs only the system's libEGL.so.1 + Mesa (llvmpipe works).
if ! ldconfig -p | grep -q 'libEGL.so.1 '; then
  echo "warning: no libEGL.so.1; headless screenshots/tests need Mesa EGL (libegl1, libegl-mesa0)"
fi
echo "tools ready"
