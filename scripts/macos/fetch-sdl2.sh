#!/bin/bash
# macOS only (CI): the official SDL2 release's universal SDL2.framework
# (same version as the Windows SDL2.dll, SHA-256 pinned) into
# tools/macos/frameworks/. Used by scripts/package/macos.sh.
set -euo pipefail
source "$(dirname "$0")/../../env.sh"
cd "$BABA_ROOT"
VER=2.32.10
SHA256=4a7ac31640d70214e848f994be8a12849c0f97918a7e6c2e27a40036166d1a7f
DL=tools/macos/downloads
FW=tools/macos/frameworks
[ -d "$FW/SDL2.framework" ] && [ "$(cat "$FW/.version" 2>/dev/null)" = "$VER" ] && exit 0
mkdir -p "$DL" "$FW"
DMG="$DL/SDL2-$VER.dmg"
if [ ! -f "$DMG" ] || ! echo "$SHA256  $DMG" | shasum -a 256 -c --status; then
    curl -sSfL --retry 3 -o "$DMG.part" "https://github.com/libsdl-org/SDL/releases/download/release-$VER/SDL2-$VER.dmg"
    echo "$SHA256  $DMG.part" | shasum -a 256 -c --status || { echo "SHA-256 mismatch for SDL2-$VER.dmg" >&2; exit 1; }
    mv "$DMG.part" "$DMG"
fi
MNT="$BABA_ROOT/tools/macos/mnt"
mkdir -p "$MNT"
hdiutil attach -quiet -nobrowse -readonly -mountpoint "$MNT" "$DMG"
rm -rf "$FW/SDL2.framework"
ditto "$MNT/SDL2.framework" "$FW/SDL2.framework"
hdiutil detach -quiet "$MNT"
echo "$VER" > "$FW/.version"
lipo -info "$FW/SDL2.framework/SDL2"
