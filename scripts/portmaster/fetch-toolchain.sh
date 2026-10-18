#!/bin/bash
# Fetches the PortMaster (aarch64 Linux, glibc) cross toolchain into
# tools/portmaster/ (nothing outside the project). No cross gcc/clang/zig:
# Rust code is linked by rust-lld (rustup) through scripts/portmaster/ld.sh
# against a ~10 MB sysroot cut out of Debian 10 "buster" arm64 packages
# (glibc 2.28, older than every CFW PortMaster supports), plus SDL2's shared
# library (Debian 12, 2.26.5 = PortMaster's own build chroot) to link against.
# At run time the device's (or PortMaster's) libSDL2-2.0.so.0 is used.
#
#   tools/portmaster/sysroot/lib  crt objects, libc/libm/libdl/libpthread/
#                                 librt/libutil, libgcc_s, libgcc.a, libSDL2.so
#
# With --qemu also the test runtime (scripts/portmaster/run-qemu.sh):
#   tools/portmaster/qemu-aarch64-static   (Debian 12 qemu-user-static)
#   tools/portmaster/rootfs                (Debian 12 arm64: SDL2 + Mesa
#                                           llvmpipe EGL/GLES2, ~200 MB)
# Downloaded .debs are checked against Debian's Packages index (SHA-256) and
# deleted afterwards unless KEEP_DEBS=1.
set -euo pipefail
source "$(dirname "$0")/../../env.sh"
cd "$BABA_ROOT"
P=tools/portmaster
DEB="python3 -I scripts/portmaster/debfetch.py"
OLD="--mirror http://archive.debian.org/debian --dist buster --arch arm64"
NEW="--mirror http://deb.debian.org/debian --dist bookworm"
rustup target add aarch64-unknown-linux-gnu >/dev/null

if [ ! -f $P/sysroot/.done ]; then
    tmp=$(mktemp -d $P/unpack.XXXXXX)
    $DEB $OLD --cache $P/debs --dest "$tmp/old" libc6 libc6-dev libgcc1 libgcc-8-dev >/dev/null
    $DEB $NEW --arch arm64 --cache $P/debs --dest "$tmp/sdl" libsdl2-2.0-0 >/dev/null
    L=$P/sysroot/lib
    rm -rf $P/sysroot; mkdir -p $L
    U=$tmp/old/usr/lib/aarch64-linux-gnu G=$tmp/old/usr/lib/gcc/aarch64-linux-gnu/8 S=$tmp/old/lib/aarch64-linux-gnu
    cp $U/Scrt1.o $U/crti.o $U/crtn.o $U/libc_nonshared.a $G/crtbeginS.o $G/crtendS.o $G/libgcc.a $L/
    cp $S/libc.so.6 $S/ld-linux-aarch64.so.1 $S/libgcc_s.so.1 $L/
    # Linker inputs carry the right SONAMEs; libc needs its nonshared part.
    cp $S/libm.so.6 $L/libm.so; cp $S/libdl.so.2 $L/libdl.so; cp $S/libpthread.so.0 $L/libpthread.so
    cp $S/librt.so.1 $L/librt.so; cp $S/libutil.so.1 $L/libutil.so
    printf 'GROUP ( libc.so.6 libc_nonshared.a AS_NEEDED ( ld-linux-aarch64.so.1 ) )\n' > $L/libc.so
    printf 'GROUP ( libgcc_s.so.1 libgcc.a )\n' > $L/libgcc_s.so
    cp "$(ls $tmp/sdl/usr/lib/aarch64-linux-gnu/libSDL2-2.0.so.0.* | head -1)" $L/libSDL2.so
    mkdir -p $P/sysroot/licenses
    cp $tmp/old/usr/share/doc/libc6/copyright $P/sysroot/licenses/glibc.copyright
    cp $tmp/sdl/usr/share/doc/libsdl2-2.0-0/copyright $P/sysroot/licenses/sdl2.copyright
    rm -rf "$tmp"
    touch $P/sysroot/.done
fi

if [ "${1:-}" = --qemu ]; then
    if [ ! -x $P/qemu-aarch64-static ]; then
        tmp=$(mktemp -d $P/unpack.XXXXXX)
        $DEB $NEW --arch amd64 --cache $P/debs --dest "$tmp" qemu-user-static >/dev/null
        cp "$tmp/usr/bin/qemu-aarch64-static" $P/
        rm -rf "$tmp"
    fi
    if [ ! -f $P/rootfs/.done ]; then
        rm -rf $P/rootfs
        # SDL2 + Mesa (llvmpipe) with dependencies; package-manager and
        # maintainer-script plumbing is left out (nothing is "installed").
        $DEB $NEW --arch arm64 --cache $P/debs --dest $P/rootfs --deps \
            --exclude debconf,dpkg,perl-base,adduser,init-system-helpers,lsb-base,sensible-utils,debianutils,passwd,login,libpam-modules,libpam-runtime,dbus,dbus-bin,dbus-daemon,dbus-system-bus-common,dbus-session-bus-common,libdecor-0-plugin-1-cairo \
            libsdl2-2.0-0 libegl1 libegl-mesa0 libgles2 libgl1-mesa-dri libc6 | tail -1
        touch $P/rootfs/.done
    fi
fi
[ "${KEEP_DEBS:-0}" = 1 ] || rm -rf $P/debs
echo "PortMaster toolchain ready: $(du -sh $P/sysroot | cut -f1) sysroot$( [ -d $P/rootfs ] && echo ", $(du -sh $P/rootfs | cut -f1) qemu rootfs")"
