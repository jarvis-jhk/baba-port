#!/bin/bash
# Builds dist/baba-port-<version>-macos-universal.zip ON macOS (CI:
# .github/workflows/ci.yml and release.yml, job "macos"): arm64 + x86_64
# slices joined with lipo, the official SDL2.framework (universal) inside
# the bundle, ad-hoc signed (no Developer ID, not notarised: players open
# it with right-click -> Open the first time; docs/platform-macos.md).
#
# Layout: baba-port.app/Contents/{Info.plist, MacOS/baba-port,
#   Frameworks/SDL2.framework, Resources/{README.md, LICENSE-MIT,
#   LICENSE-APACHE, THIRD_PARTY_NOTICES.md, licenses/...}}
#
# SDL2 is linked through a libSDL2.dylib symlink to the framework binary
# (its install name is @rpath/SDL2.framework/Versions/A/SDL2), so the sdl2
# crate needs no macOS-only feature; the executable's rpath is
# @executable_path/../Frameworks.
#
# Usage: scripts/package/macos.sh [--no-build]
# Env: SDL2_FRAMEWORK_DIR (folder holding SDL2.framework; default
#      tools/macos/frameworks, filled by scripts/macos/fetch-sdl2.sh).
set -euo pipefail
source "$(dirname "$0")/../../env.sh"
cd "$BABA_ROOT"
source scripts/package/common.sh
[ "$(uname -s)" = Darwin ] || { echo "macOS only (CI); see docs/platform-macos.md" >&2; exit 1; }

BUILD=1
for a in "$@"; do
    case $a in
        --no-build) BUILD=0 ;;
        *) echo "unknown option $a" >&2; exit 2 ;;
    esac
done
VERSION=$(pkg_version)
FW="${SDL2_FRAMEWORK_DIR:-$BABA_ROOT/tools/macos/frameworks}"
[ -d "$FW/SDL2.framework" ] || scripts/macos/fetch-sdl2.sh
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-11.0}"
LINK="$BABA_ROOT/target/macos-sdl2-link"
mkdir -p "$LINK"
ln -sf "$FW/SDL2.framework/Versions/A/SDL2" "$LINK/libSDL2.dylib"
export BABA_SDL2_LIB_DIR="$LINK"
export RUSTFLAGS="${RUSTFLAGS:-} -C link-arg=-Wl,-rpath,@executable_path/../Frameworks"

TARGETS="aarch64-apple-darwin x86_64-apple-darwin"
if [ $BUILD = 1 ]; then
    for t in $TARGETS; do
        cargo build --profile dist ${CI:+--locked} --target "$t" -p baba-port
    done
fi

APP_NAME=baba-port
STAGE="$DIST/.stage-macos"
APP="$STAGE/$APP_NAME.app"
pkg_clean_stage "$STAGE"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Frameworks" "$APP/Contents/Resources"
lipo -create -output "$APP/Contents/MacOS/baba-port" \
    target/aarch64-apple-darwin/dist/baba-port target/x86_64-apple-darwin/dist/baba-port
lipo -info "$APP/Contents/MacOS/baba-port"
# ditto keeps the framework's symlinks (Versions/Current, ...).
ditto "$FW/SDL2.framework" "$APP/Contents/Frameworks/SDL2.framework"
# Headers are not needed at run time.
rm -rf "$APP/Contents/Frameworks/SDL2.framework/Versions/A/Headers" \
       "$APP/Contents/Frameworks/SDL2.framework/Headers"
sed -e "s|@NAME@|baba-port|g; s|@ID@|org.babaport|; s|@EXE@|baba-port|; s|@VERSION@|$VERSION|g" \
    packaging/macos/Info.plist > "$APP/Contents/Info.plist"
# Both slices link the same crates: the arm64 triple stands for both.
pkg_stage_docs "$APP/Contents/Resources" aarch64-apple-darwin
install -m 644 packaging/licenses/SDL2-LICENSE.txt "$APP/Contents/Resources/licenses/"
otool -L "$APP/Contents/MacOS/baba-port"
otool -L "$APP/Contents/MacOS/baba-port" | grep -q '@rpath/SDL2.framework' \
    || { echo "baba-port does not load SDL2 from the bundle" >&2; exit 1; }
# No Developer ID: ad-hoc signature (arm64 code must carry at least that).
codesign --force --deep --sign - "$APP"
codesign --verify --deep --strict "$APP"
OUT="$DIST/baba-port-$VERSION-macos-universal.zip"
mkdir -p "$DIST"
rm -f "$OUT"
(cd "$STAGE" && ditto -c -k --sequesterRsrc --keepParent "$APP_NAME.app" "$OUT")
rm -rf "$STAGE"
ls -l "$OUT"
