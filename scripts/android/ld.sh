#!/bin/bash
# Linker for aarch64-linux-android without the NDK's clang: rustc calls this
# with ld-style arguments (-C linker-flavor=ld.lld, set in
# scripts/android/env.sh) and it runs rust-lld (shipped with rustup) against
# the pruned NDK sysroot in tools/android/ndk-min (scripts/android/fetch-toolchain.sh),
# adding the C runtime objects the clang driver would add:
#   shared library (-shared): crtbegin_so.o ... crtend_so.o
#   executable:               crtbegin_dynamic.o ... crtend_android.o
set -euo pipefail
R="$(cd "$(dirname "$0")/../.." && pwd)"
L="$R/tools/android/ndk-min/lib"
LLD="$(ls "$R"/tools/rustup/toolchains/*/lib/rustlib/x86_64-unknown-linux-gnu/bin/rust-lld | head -1)"
args=()
[ "${1:-}" = -flavor ] && shift 2   # rustc may already pass "-flavor gnu"
shared=0
for a in "$@"; do [ "$a" = -shared ] && shared=1; done
if [ $shared = 1 ]; then begin=crtbegin_so.o; end=crtend_so.o
else begin=crtbegin_dynamic.o; end=crtend_android.o; args+=(-dynamic-linker /system/bin/linker64); fi
exec "$LLD" -flavor gnu -m aarch64linux --sysroot=/nonexistent -L "$L" \
    -z max-page-size=16384 --hash-style=gnu --build-id=sha1 --eh-frame-hdr \
    "${args[@]}" "$L/$begin" "$@" "$L/$end"
