#!/bin/bash
# Builds dist/baba-port-<version>-windows-x86_64.zip, cross-compiled with
# llvm-mingw (scripts/windows/build.sh, toolchain fetched on first use).
#
# Layout (exec = "baba-port-*/baba-port.exe" in the Warpzone descriptor):
#   baba-port-<version>/{baba-port.exe, SDL2.dll, README.txt, LICENSE-MIT,
#     LICENSE-APACHE, THIRD_PARTY_NOTICES.md, licenses/{RUST_DEPENDENCIES.txt,
#     SDL2-LICENSE.txt}}
#
# Usage: scripts/package/windows.sh [--no-build]
set -euo pipefail
source "$(dirname "$0")/../../env.sh"
cd "$BABA_ROOT"
source scripts/package/common.sh

BUILD=1
for a in "$@"; do
    case $a in
        --no-build) BUILD=0 ;;
        *) echo "unknown option $a" >&2; exit 2 ;;
    esac
done
VERSION=$(pkg_version)
NAME="baba-port-$VERSION"
OUT="$DIST/$NAME-windows-x86_64.zip"
STAGE="$DIST/.stage-windows"

if [ $BUILD = 1 ]; then scripts/windows/build.sh >/dev/null; fi
SRC=target/dist/windows
[ -f "$SRC/baba-port.exe" ] && [ -f "$SRC/SDL2.dll" ] || { echo "$SRC incomplete: run scripts/windows/build.sh" >&2; exit 1; }

pkg_clean_stage "$STAGE"
P="$STAGE/$NAME"
mkdir -p "$P"
install -m 644 "$SRC/baba-port.exe" "$SRC/SDL2.dll" "$P/"
pkg_stage_docs "$P" x86_64-pc-windows-gnu
# Notepad-friendly name and line endings for the readme.
sed 's/$/\r/' "$P/README.md" > "$P/README.txt"
rm "$P/README.md"
install -m 644 packaging/licenses/SDL2-LICENSE.txt "$P/licenses/SDL2-LICENSE.txt"
mkdir -p "$DIST"
pkg_zip "$STAGE" "$OUT"
rm -rf "$STAGE"
echo "$OUT ($(human "$OUT"))"
