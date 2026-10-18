# Source after env.sh: cross-compiling Rust for Android (arm64-v8a) with the
# pruned toolchain from scripts/android/fetch-toolchain.sh (no clang, no Java).
#   source env.sh && source scripts/android/env.sh
#   cargo build --release --target aarch64-linux-android
_A="$BABA_ROOT/tools/android/ndk-min"
if [ ! -f "$_A/.version" ]; then
    echo "Android toolchain missing: run scripts/android/fetch-toolchain.sh" >&2
    unset _A; return 1 2>/dev/null || exit 1
fi
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$BABA_ROOT/scripts/android/ld.sh"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS="-C linker-flavor=ld.lld -C link-arg=-L$_A/lib"
export BABA_ANDROID_API=26
unset _A
