#!/bin/bash
# Cross-checks that the code compiles for macOS (Apple Silicon + Intel) from
# Linux. `cargo check` needs no Apple SDK (nothing is linked); real builds
# happen on GitHub's macOS runners (.github/workflows/macos.yml).
# Usage: scripts/macos/check.sh [--workspace]   (default: the SDL probe)
# --workspace skips crates that compile C for the target (baba-net pulls in
# ring, whose build script needs clang + the macOS SDK headers); everything
# pure Rust is checked.
EXCLUDE=${MACOS_CHECK_EXCLUDE:-baba-net}
set -euo pipefail
source "$(dirname "$0")/../../env.sh"
cd "$BABA_ROOT"
for t in aarch64-apple-darwin x86_64-apple-darwin; do
    rustup target add $t >/dev/null
    if [ "${1:-}" = --workspace ]; then
        ex=(); for p in $EXCLUDE; do ex+=(--exclude "$p"); done
        cargo check --workspace "${ex[@]}" --target $t --target-dir target/macos-check
    else
        cargo check --release --target $t --target-dir target/probes --manifest-path probes/sdl/Cargo.toml
    fi
done
echo "macOS check OK"
