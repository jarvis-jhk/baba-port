#!/bin/bash
# Builds dist/baba-port-<version>-portmaster.zip: the PortMaster port
# (scripts/portmaster/build.sh: aarch64, glibc >= 2.17, the device's SDL2
# and GLES2) plus the licence texts. Unzip it into the device's ports/
# folder (or drop it into PortMaster's autoinstall/ folder).
#
# Layout:
#   Baba Is You.sh                launcher
#   babaport/{baba-port.aarch64, port.json, gameinfo.xml, README.md,
#             gamedata/PUT-GAME-DATA-HERE.txt,
#             licenses/{LICENSE-MIT, LICENSE-APACHE, THIRD_PARTY_NOTICES.md,
#                       RUST_DEPENDENCIES.txt}}
#
# Usage: scripts/package/portmaster.sh [--no-build]   (--no-build: reuse the
#   staged port in target/dist/portmaster/stage)
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
OUT="$DIST/baba-port-$VERSION-portmaster.zip"
STAGE="$DIST/.stage-portmaster"

if [ $BUILD = 1 ]; then scripts/portmaster/build.sh >/dev/null; fi
SRC=target/dist/portmaster/stage
[ -x "$SRC/babaport/baba-port.aarch64" ] || { echo "$SRC incomplete: run scripts/portmaster/build.sh" >&2; exit 1; }
python3 -I scripts/portmaster/check-glibc.py --max 2.17 "$SRC/babaport/baba-port.aarch64"

pkg_clean_stage "$STAGE"
cp -a "$SRC/." "$STAGE/"
L="$STAGE/babaport/licenses"
mkdir -p "$L"
install -m 644 LICENSE-MIT LICENSE-APACHE THIRD_PARTY_NOTICES.md "$L/"
"$PY" -I packaging/gen-licenses.py "$L/RUST_DEPENDENCIES.txt" aarch64-unknown-linux-gnu baba-port >/dev/null
chmod 644 "$L/RUST_DEPENDENCIES.txt"
mkdir -p "$DIST"
pkg_zip "$STAGE" "$OUT"
rm -rf "$STAGE"
echo "$OUT ($(human "$OUT"))"
