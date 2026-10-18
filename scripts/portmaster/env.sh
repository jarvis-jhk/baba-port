# Source after env.sh: cross-compiling for PortMaster (aarch64-unknown-linux-gnu)
# with rust-lld + the Debian-buster sysroot (scripts/portmaster/fetch-toolchain.sh).
#   source env.sh && source scripts/portmaster/env.sh
#   cargo build --release --target aarch64-unknown-linux-gnu
if [ ! -f "$BABA_ROOT/tools/portmaster/sysroot/.done" ]; then
    echo "PortMaster toolchain missing: run scripts/portmaster/fetch-toolchain.sh" >&2
    return 1 2>/dev/null || exit 1
fi
export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER="$BABA_ROOT/scripts/portmaster/ld.sh"
export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_RUSTFLAGS="-C linker-flavor=ld.lld -L native=$BABA_ROOT/tools/portmaster/sysroot/lib"
