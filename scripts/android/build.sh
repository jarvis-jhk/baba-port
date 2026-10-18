#!/bin/bash
# Builds the arm64-v8a APK with no Java, no Gradle, no JDK and no SDK build-tools:
#   cargo (aarch64-linux-android, linked by rust-lld via scripts/android/ld.sh
#   against the pruned NDK sysroot) -> scripts/android/apk.py (binary manifest,
#   aligned zip, APK Signature Scheme v2 via openssl).
# Toolchain: scripts/android/fetch-toolchain.sh (run automatically).
#
# Usage: scripts/android/build.sh [--probe] [--debug] [--stored-libs]
#   (default)      the game: crates/android -> libbabaport.so, package org.babaport,
#                  target/dist/android/baba-port.apk
#   --probe        the platform probe (probes/android) -> baba-probe.apk
#   --debug        debug build, android:debuggable
#   --stored-libs  keep the .so uncompressed + 16 KiB aligned in the APK
#                  (extractNativeLibs=false: bigger APK, smaller install)
# Env: APK_KEY_DIR (default tools/android/keys/debug: a throwaway EC P-256 key,
#      created on first use; a release key lives outside git, see
#      docs/platform-android.md), APK_REQUIRE_KEY=1 (never create a key: the
#      release key must exist), VERSION_CODE, VERSION_NAME, APK_FILES
#      ("apkpath=localpath ...", deflated extra files: licence texts;
#      scripts/package/android.sh).
set -euo pipefail
source "$(dirname "$0")/../../env.sh"
cd "$BABA_ROOT"
scripts/android/fetch-toolchain.sh >/dev/null
source scripts/android/env.sh

PROFILE=dist; FLAG="--profile dist"; EXTRACT=--extract-native-libs; PROBE=0
for a in "$@"; do
    case $a in
        --debug) PROFILE=debug; FLAG= ;;
        --stored-libs) EXTRACT= ;;
        --probe) PROBE=1 ;;
        *) echo "unknown option $a" >&2; exit 2 ;;
    esac
done

T=aarch64-linux-android
OUT=target/dist/android
mkdir -p "$OUT"
if [ $PROBE = 1 ]; then
    [ $PROFILE = dist ] && { PROFILE=release; FLAG=--release; }
    TD=target/probes
    cargo build $FLAG --target $T --target-dir "$TD" --manifest-path probes/android/Cargo.toml
    SO=$TD/$T/$PROFILE/libbabaprobe.so
    APK=$OUT/baba-probe.apk PKG=org.babaport.probe LABEL="Baba Probe" LIB=babaprobe EXTRA=()
else
    cargo build $FLAG --target $T -p baba-android
    SO=target/$T/$PROFILE/libbabaport.so
    # "Open with" / "Share to" baba-port for zips: the game data import.
    APK=$OUT/baba-port.apk PKG=org.babaport LABEL="baba-port" LIB=babaport
    # INTERNET (a normal permission, granted at install): Warpzone online play.
    EXTRA=(--open-mime application/zip --open-mime application/x-zip-compressed
          --permission android.permission.INTERNET)
fi

python3 -I scripts/android/apk.py build --out "$APK" \
    --package $PKG --label "$LABEL" --lib-name $LIB \
    --so "lib/arm64-v8a/lib$LIB.so=$SO" "${EXTRA[@]}" \
    --key-dir "${APK_KEY_DIR:-tools/android/keys/debug}" \
    --version-code "${VERSION_CODE:-1}" --version-name "${VERSION_NAME:-dev}" \
    --min-sdk "$BABA_ANDROID_API" --target-sdk 35 $EXTRACT \
    $([ $PROFILE = debug ] && echo --debuggable) \
    $([ "${APK_REQUIRE_KEY:-0}" = 1 ] && echo --require-key) \
    $(for f in ${APK_FILES:-}; do echo --file "$f"; done)
python3 -I scripts/android/apk.py verify "$APK"
ls -l "$SO" "$APK"
