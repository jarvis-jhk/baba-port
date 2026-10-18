#!/bin/bash
# The owner's private Android build: like the release APK (release key,
# licence texts) plus what must never go into a public release:
#   * the launcher icon with Baba from the game data (build.sh --game-icon:
#     $BABA_ICON_GAME_DIR, else $BABA_GAME_DIR, else the copy in
#     tools/baba-original);
#   * the fault-report endpoint (keys/report-url -> BABA_REPORT_URL,
#     crates/android/src/crash.rs).
# versionCode = the release code of the version + a build counter (01..99,
# tools/android/private-build-code), so every private build is an update and
# the next release is again (scripts/package/common.sh).
#
# Usage: scripts/android/private.sh [--publish]
#   --publish  upload the APK to the owner's update channel (URL in the
#              git-ignored keys/apkup-url; notes from $NOTES)
# Output: target/dist/android/baba-port-private.apk
set -euo pipefail
source "$(dirname "$0")/../../env.sh"
cd "$BABA_ROOT"
source scripts/package/common.sh

PUBLISH=0
for a in "$@"; do
    case $a in
        --publish) PUBLISH=1 ;;
        *) echo "unknown option $a" >&2; exit 2 ;;
    esac
done
KEY_DIR=${BABA_ANDROID_KEY_DIR:?no release key (keys/android): private builds must update releases}
GAME=${BABA_ICON_GAME_DIR:-$BABA_GAME_DIR}
[ -d "$GAME/Data" ] || [ -d "$GAME/Contents/Resources/Data" ] || GAME=tools/baba-original

VERSION=$(pkg_version)
BASE=$(pkg_version_code "$VERSION")
COUNTER=tools/android/private-build-code
LAST=$(cat "$COUNTER" 2>/dev/null || echo 0)
CODE=$(( LAST >= BASE ? LAST + 1 : BASE + 1 ))
if [ $CODE -ge $(( BASE + 100 )) ]; then
    echo "99 private builds of $VERSION: bump the version" >&2; exit 1
fi

if [ -f keys/report-url ]; then
    BABA_REPORT_URL=$(tr -d '[:space:]' < keys/report-url); export BABA_REPORT_URL
else
    echo "note: no keys/report-url: this build reports no faults" >&2
fi
STAGE=target/dist/android/stage-private
rm -rf "$STAGE"; mkdir -p "$STAGE"
install -m 644 LICENSE-MIT LICENSE-APACHE THIRD_PARTY_NOTICES.md "$STAGE/"
"$PY" -I packaging/gen-licenses.py "$STAGE/RUST_DEPENDENCIES.txt" aarch64-linux-android baba-android >/dev/null
FILES=""
for f in LICENSE-MIT LICENSE-APACHE THIRD_PARTY_NOTICES.md RUST_DEPENDENCIES.txt; do
    FILES="$FILES licenses/$f=$STAGE/$f"
done
APK_KEY_DIR="$KEY_DIR" APK_REQUIRE_KEY=1 APK_FILES="$FILES" VERSION_CODE=$CODE \
    VERSION_NAME="$VERSION+p$(( CODE - BASE ))" \
    scripts/android/build.sh --game-icon="$GAME" >/dev/null
OUT=target/dist/android/baba-port-private.apk
cp target/dist/android/baba-port.apk "$OUT"
rm -rf "$STAGE"
echo $CODE > "$COUNTER"
echo "$OUT ($(human "$OUT"), version $VERSION+p$(( CODE - BASE )), versionCode $CODE)"

if [ $PUBLISH = 1 ]; then
    URL=$(tr -d '[:space:]' < keys/apkup-url)
    N=$(printf %s "${NOTES:-private build}" | "$PY" -I -c 'import sys,urllib.parse;print(urllib.parse.quote_plus(sys.stdin.read()))')
    curl -sSf -X POST --data-binary @"$OUT" -H 'content-type: application/vnd.android.package-archive' \
        "$URL?version=$VERSION%2Bp$(( CODE - BASE ))&versionCode=$CODE&package=org.babaport&notes=$N"
    echo
fi
