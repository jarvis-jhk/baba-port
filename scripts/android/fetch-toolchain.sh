#!/bin/bash
# Fetches the (pruned) Android toolchain into tools/android/ (nothing outside
# the project). No JDK, no SDK build-tools, no clang: the probe/game is pure
# Rust linked by rust-lld (shipped with rustup), packaged and signed by
# scripts/android/apk.py (Python stdlib + the openssl CLI).
#
# What is kept from the NDK (~2 MB of a 739 MB zip, see docs/platform-android.md):
#   - the API-26 sysroot stubs for arm64 (libc/libm/libdl/liblog/libandroid/
#     libEGL/libGLESv2/libaaudio/... .so stubs, crtbegin_so.o/crtend_so.o)
#   - libunwind.a (Rust's std links it on Android)
#   - compiler-rt builtins (exposed as libgcc.a, for anything that asks for it)
#   - NOTICE + source.properties
# The zip is deleted afterwards unless KEEP_NDK_ZIP=1.
#
# Also installs the aarch64-linux-android Rust target (rustup, in tools/).
set -euo pipefail
source "$(dirname "$0")/../../env.sh"
cd "$BABA_ROOT"

NDK_VERSION=r30
NDK_FILE=android-ndk-${NDK_VERSION}-linux.zip
NDK_URL=https://dl.google.com/android/repository/$NDK_FILE
NDK_SHA1=5107f898313790e449e87eee2183d9a20602dee9   # from repository2-3.xml
API=26          # minSdk: AAudio needs 26
CLANG_VER=21    # clang resource dir inside NDK r30

A=tools/android
OUT=$A/ndk-min
if [ -f "$OUT/.version" ] && [ "$(cat "$OUT/.version")" = "$NDK_VERSION-$API" ]; then
    echo "Android toolchain ready in $OUT"; exit 0
fi

rustup target add aarch64-linux-android >/dev/null

mkdir -p "$A/downloads"
Z=$A/downloads/$NDK_FILE
if [ ! -f "$Z" ] || ! echo "$NDK_SHA1  $Z" | sha1sum -c --status; then
    echo "downloading $NDK_FILE (739 MB, pruned to ~2 MB afterwards)"
    curl -sSfL --retry 3 -o "$Z.part" "$NDK_URL"
    echo "$NDK_SHA1  $Z.part" | sha1sum -c --status || { echo "SHA-1 mismatch for $NDK_FILE" >&2; rm -f "$Z.part"; exit 1; }
    mv "$Z.part" "$Z"
fi

P=android-ndk-$NDK_VERSION/toolchains/llvm/prebuilt/linux-x86_64
tmp=$(mktemp -d "$A/unpack.XXXXXX")
unzip -q "$Z" -d "$tmp" \
    "$P/sysroot/usr/lib/aarch64-linux-android/$API/*" \
    "$P/lib/clang/$CLANG_VER/lib/linux/aarch64/libunwind.a" \
    "$P/lib/clang/$CLANG_VER/lib/linux/libclang_rt.builtins-aarch64-android.a" \
    "android-ndk-$NDK_VERSION/NOTICE" "android-ndk-$NDK_VERSION/source.properties"
rm -rf "$OUT"; mkdir -p "$OUT/lib"
S=$tmp/$P/sysroot/usr/lib/aarch64-linux-android/$API
# Only what a NativeActivity + EGL/GLES2 + AAudio app links against.
for f in crtbegin_so.o crtend_so.o crtbegin_dynamic.o crtend_android.o libc.so libm.so libdl.so liblog.so libandroid.so \
         libEGL.so libGLESv2.so libaaudio.so libnativewindow.so libz.so; do
    if [ -f "$S/$f" ]; then cp "$S/$f" "$OUT/lib/"; fi
done
cp "$tmp/$P/lib/clang/$CLANG_VER/lib/linux/aarch64/libunwind.a" "$OUT/lib/"
cp "$tmp/$P/lib/clang/$CLANG_VER/lib/linux/libclang_rt.builtins-aarch64-android.a" "$OUT/lib/libgcc.a"
cp "$tmp/android-ndk-$NDK_VERSION/NOTICE" "$tmp/android-ndk-$NDK_VERSION/source.properties" "$OUT/"
rm -rf "$tmp"
echo "$NDK_VERSION-$API" > "$OUT/.version"
[ "${KEEP_NDK_ZIP:-0}" = 1 ] || rm -f "$Z"
echo "Android toolchain ready in $OUT ($(du -sh "$OUT" | cut -f1))"
