#!/bin/bash
# Linker for aarch64-unknown-linux-gnu without a cross gcc: rustc calls this
# with ld-style arguments (-C linker-flavor=ld.lld, scripts/portmaster/env.sh)
# and it runs rust-lld against the sysroot from
# scripts/portmaster/fetch-toolchain.sh, adding glibc's/gcc's start files the
# way the gcc driver would (PIE executables and shared objects).
set -euo pipefail
R="$(cd "$(dirname "$0")/../.." && pwd)"
L="$R/tools/portmaster/sysroot/lib"
LLD="$(ls "$R"/tools/rustup/toolchains/*/lib/rustlib/x86_64-unknown-linux-gnu/bin/rust-lld | head -1)"
[ "${1:-}" = -flavor ] && shift 2
shared=0
for a in "$@"; do [ "$a" = -shared ] && shared=1; done
if [ $shared = 1 ]; then pre=("$L/crti.o" "$L/crtbeginS.o")
else pre=(-dynamic-linker /lib/ld-linux-aarch64.so.1 "$L/Scrt1.o" "$L/crti.o" "$L/crtbeginS.o"); fi
exec "$LLD" -flavor gnu -m aarch64linux --sysroot=/nonexistent -L "$L" \
    --hash-style=gnu --build-id=sha1 --eh-frame-hdr -z relro \
    "${pre[@]}" "$@" "$L/crtendS.o" "$L/crtn.o"
