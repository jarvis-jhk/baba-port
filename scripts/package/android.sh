#!/bin/bash
# Builds dist/baba-port-<version>-android-arm64.apk (scripts/android/build.sh:
# no Java, no Gradle; arm64-v8a; APK Signature Scheme v2).
#
# Signing (docs/platform-android.md, "Release signing"):
#   BABA_ANDROID_KEY_DIR=DIR  the release key (DIR/key.pem + DIR/cert.der,
#                             made once by scripts/android/release-key.sh and
#                             kept outside git); required to exist, never
#                             created here. CI writes it from secrets.
#   (unset)                   the throwaway debug key in tools/android/keys/
#                             debug; the file is then named
#                             ...-android-arm64-debugkey.apk so it cannot pass
#                             for a release (Android refuses updates across
#                             keys).
# The licence texts go into the APK (licenses/, deflated).
#
# Usage: scripts/package/android.sh   (--no-build is accepted and ignored:
#   cargo is incremental, the APK is always re-packed and re-signed)
set -euo pipefail
source "$(dirname "$0")/../../env.sh"
cd "$BABA_ROOT"
source scripts/package/common.sh

for a in "$@"; do
    case $a in
        --no-build) ;;   # cargo is incremental; the APK is always re-packed
        *) echo "unknown option $a" >&2; exit 2 ;;
    esac
done
VERSION=$(pkg_version)
CODE=$(pkg_version_code "$VERSION")
if [ -n "${BABA_ANDROID_KEY_DIR:-}" ]; then
    KEY_DIR=$BABA_ANDROID_KEY_DIR REQUIRE=1 SUFFIX=
else
    KEY_DIR=tools/android/keys/debug REQUIRE=0 SUFFIX=-debugkey
    echo "note: no BABA_ANDROID_KEY_DIR: signing with the debug key" >&2
fi
OUT="$DIST/baba-port-$VERSION-android-arm64$SUFFIX.apk"
STAGE="$DIST/.stage-android"
pkg_clean_stage "$STAGE"
install -m 644 LICENSE-MIT LICENSE-APACHE THIRD_PARTY_NOTICES.md "$STAGE/"
"$PY" -I packaging/gen-licenses.py "$STAGE/RUST_DEPENDENCIES.txt" aarch64-linux-android baba-android >/dev/null
FILES=""
for f in LICENSE-MIT LICENSE-APACHE THIRD_PARTY_NOTICES.md RUST_DEPENDENCIES.txt; do
    FILES="$FILES licenses/$f=$STAGE/$f"
done
# A release never carries the private fault-report endpoint (crates/android/src/crash.rs)
# nor a game-art icon (scripts/android/private.sh).
unset BABA_REPORT_URL
APK_KEY_DIR="$KEY_DIR" APK_REQUIRE_KEY=$REQUIRE APK_FILES="$FILES" \
    VERSION_CODE="$CODE" VERSION_NAME="$VERSION" scripts/android/build.sh >/dev/null
mkdir -p "$DIST"
cp target/dist/android/baba-port.apk "$OUT"
rm -rf "$STAGE"
"$PY" -I scripts/android/apk.py verify "$OUT"
echo "$OUT ($(human "$OUT"), versionCode $CODE)"
