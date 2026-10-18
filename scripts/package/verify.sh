#!/bin/bash
# Unpacks release archives and runs them headless (never on the desktop, never
# with the user's own data dir). With the game installed ($BABA_GAME_DIR) the
# checks play it; without (CI) they cover start-up and the no-data screen.
#
# Usage: scripts/package/verify.sh linux|windows|portmaster|android [ARCHIVE]
#   linux       unpack into $VERIFY_DIR/linux: --version, the libraries the
#               loader picks, the no-data screen (SDL offscreen + EGL); with
#               the game: finding it in a Steam library under a fake HOME
#               (title screen), a headless level, and the M4 flow drive
#               scripts (crates/game/tests/flow) on the packaged binary
#   windows     unpack into dist/.verify/windows (inside the project root:
#               scripts/windows/run-wine.sh only binds that read-write) and
#               run it under Wine: --version, the no-data screen, with the
#               game the flow scripts on the SDL window
#   portmaster  unpack into $VERIFY_DIR/portmaster/ports: layout, launcher,
#               port.json, glibc; under qemu-user (aarch64 rootfs, llvmpipe
#               GLES2): the no-data screen at 640x480 and, with the game
#               (linked into gamedata/ like a player's copy), the pad flow
#               packaging/portmaster/smoke.drive at 640x480 and 720x720
#   android     signature (v2), manifest (package, version), contents
# Env: VERIFY_DIR (default scratch/m11/verify, CI: scratch/pkgtest).
set -euo pipefail
source "$(dirname "$0")/../../env.sh"
cd "$BABA_ROOT"
source scripts/package/common.sh
PLAT=${1:?usage: $0 linux|windows|portmaster|android [ARCHIVE]}
VERSION=$(pkg_version)
VD=$(realpath -m "${VERIFY_DIR:-$BABA_ROOT/scratch/m11/verify}")
have_game() { [ -f "${BABA_GAME_DIR:-}/Data/values.lua" ]; }
fail() { echo "VERIFY FAILED ($PLAT): $*" >&2; exit 1; }
# Flow tests: no import offer from a real save, no network for level codes.
flow_env() {
    export BABA_IMPORT_FROM="$1/no-original-save" BABA_LEVELCODES_OFFLINE=1
    export BABA_REF="${BABA_REF:-$BABA_ROOT/scratch/ref/capture}"
}
png_ok() { [ -s "$1" ] && head -c 8 "$1" | od -An -tx1 | grep -q '89 50 4e 47' || fail "no screenshot $1"; }

case $PLAT in
linux)
    A=${2:-$DIST/baba-port-$VERSION-linux-x86_64.tar.gz}
    V="$VD/linux"; rm -rf "$V"; mkdir -p "$V/home"
    tar -xzf "$A" -C "$V"
    P="$V/baba-port-$VERSION"
    [ -x "$P/play.sh" ] && [ -x "$P/baba-port" ] || fail "play.sh / baba-port missing or not executable"
    "$P/play.sh" --version | tee "$V/version.txt" | grep -q "baba-port $VERSION" || fail "--version"
    # Resolve like play.sh does: a bundled lib/ comes first (CI runners have
    # no system SDL2, so a plain ldd would report it missing).
    LDP=${LD_LIBRARY_PATH:-}
    [ -d "$P/lib" ] && LDP="$P/lib${LDP:+:$LDP}"
    LD_LIBRARY_PATH="$LDP" ldd "$P/baba-port" | grep -E 'SDL2|not found' | tee "$V/ldd.txt"
    if [ -d "$P/lib" ]; then
        grep -q "$P/lib/libSDL2" "$V/ldd.txt" || fail "bundled SDL2 not used"
    fi
    if grep -q 'not found' "$V/ldd.txt"; then
        { echo "LD_LIBRARY_PATH=$LDP"; ls -la "$P" "$P/lib" 2>&1; file "$P"/lib/* 2>&1
          LD_LIBRARY_PATH="$LDP" ldd "$P/baba-port"; } >&2
        fail "unresolved libraries"
    fi
    echo "glibc needed: $(pkg_max_glibc "$P/baba-port")"
    # The no-data screen: a fresh HOME (nothing to find), own data dir.
    run() {
        env -u BABA_GAME_DIR -u WARPZONE_DATA_BABA HOME="$V/home" XDG_DATA_HOME= \
            SDL_VIDEODRIVER=offscreen SDL_AUDIODRIVER=dummy "$@"
    }
    BABA_PORT_DATA_DIR="$V/data-empty" run "$P/play.sh" --frames 30 --screenshot "$V/no-data.png" \
        2> "$V/no-data.log" || true
    png_ok "$V/no-data.png"
    grep -q "no game data in $V/data-empty/game" "$V/no-data.log" || fail "no-data screen did not name <data dir>/game"
    echo "linux: --version, libraries, no-data screen ok"
    if have_game; then
        # Found like a player's Steam install (read only through a symlink).
        mkdir -p "$V/home/.steam/steam/steamapps/common"
        ln -s "$BABA_GAME_DIR" "$V/home/.steam/steam/steamapps/common/Baba Is You"
        flow_env "$V"
        BABA_PORT_DATA_DIR="$V/data-steam" run "$P/play.sh" --frames 240 --screenshot "$V/steam-found.png" \
            2> "$V/steam-found.log"
        png_ok "$V/steam-found.png"
        grep -q "no game data" "$V/steam-found.log" && fail "the Steam install was not found"
        SDL_AUDIODRIVER=dummy "$P/play.sh" --game-dir "$BABA_GAME_DIR" --headless --level baba/0level \
            --screenshot "$V/0level.png" --uniform-wobble \
            $([ -f scratch/ref/l00_start.png ] && echo --compare scratch/ref/l00_start.png) | tail -3
        png_ok "$V/0level.png"
        st=0
        for s in crates/game/tests/flow/*.drive; do
            echo "== $s"
            BABA_GAME_DIR="$BABA_GAME_DIR" "$P/play.sh" --drive "$s" --data-dir "$V/flow-data" \
                --out "$V/flow" > "$V/flow-$(basename "$s" .drive).log" 2>&1 || st=1
            tail -2 "$V/flow-$(basename "$s" .drive).log"
        done
        [ $st = 0 ] || fail "flow drive scripts (logs in $V)"
        echo "linux: Steam discovery, headless level, $(ls "$V/flow" | wc -l) flow screenshots ok"
    fi ;;

windows)
    A=${2:-$DIST/baba-port-$VERSION-windows-x86_64.zip}
    # Inside BABA_ROOT itself (run-wine.sh binds only that read-write).
    V="$BABA_ROOT/dist/.verify/windows"; rm -rf "$V"; mkdir -p "$V"
    unzip -q "$A" -d "$V"
    P="$V/baba-port-$VERSION"
    for f in baba-port.exe SDL2.dll README.txt LICENSE-MIT licenses/SDL2-LICENSE.txt; do
        [ -f "$P/$f" ] || fail "$f missing"
    done
    ft=$(file "$P/baba-port.exe")
    case $ft in *PE32+*GUI*x86-64*) ;; *) fail "not a 64-bit GUI exe: $ft" ;; esac
    imports=$(tools/windows/llvm-mingw/bin/llvm-readobj --coff-imports "$P/baba-port.exe" \
        | sed -n 's/^ *Name: \(.*\.dll\)$/\1/Ip' | sort -u | tr '\n' ' ')
    echo "imports: $imports"
    for dll in $imports; do
        case ${dll,,} in
            sdl2.dll|kernel32.dll|user32.dll|gdi32.dll|opengl32.dll|ws2_32.dll|advapi32.dll|bcrypt.dll|bcryptprimitives.dll|\
            ntdll.dll|shell32.dll|ole32.dll|oleaut32.dll|userenv.dll|crypt32.dll|secur32.dll|winmm.dll|\
            api-ms-win-*|msvcrt.dll|ucrtbase.dll|imm32.dll|setupapi.dll|version.dll|iphlpapi.dll|dbghelp.dll) ;;
            *) fail "unexpected DLL import $dll (only system DLLs and SDL2.dll ship)" ;;
        esac
    done
    PROTON_DIR=${PROTON_DIR:-"$HOME/.steam/steam/steamapps/common/Proton - Experimental"}
    if [ ! -x "$PROTON_DIR/files/bin/wine" ] || ! command -v bwrap >/dev/null; then
        echo "windows: files and imports ok; no Proton/bubblewrap here, Wine run skipped"
        exit 0
    fi
    export EXE="$P/baba-port.exe"
    scripts/windows/run-wine.sh --version 2>/dev/null | tr -d '\r' | grep -q "baba-port $VERSION" || fail "--version under Wine"
    BABA_PORT_DATA_DIR="$V/data-empty" scripts/windows/run-wine.sh --game-dir 'Z:\nonexistent' \
        --frames 30 --screenshot "$V/no-data.png" > "$V/no-data.log" 2>&1 || true
    png_ok "$V/no-data.png"
    echo "windows: files, imports, --version, no-data screen under Wine ok"
    if have_game; then
        flow_env "$V"
        st=0
        for s in crates/game/tests/flow/*.drive; do
            echo "== $s"
            scripts/windows/run-wine.sh --drive "$s" --window --render-every 2 --data-dir "$V/flow-data" \
                --out "$V/flow" --corpus ref/tas/corpus/corpus.tsv > "$V/flow-$(basename "$s" .drive).log" 2>&1 || st=1
            tail -2 "$V/flow-$(basename "$s" .drive).log"
        done
        mkdir -p "$VD/windows"
        cp -r "$V/flow" "$V"/*.png "$V"/*.log "$VD/windows/" 2>/dev/null || true
        [ $st = 0 ] || fail "flow drive scripts under Wine (logs in $V)"
        echo "windows: $(ls "$V/flow" | wc -l) flow screenshots under Wine ok"
    fi ;;

portmaster)
    A=${2:-$DIST/baba-port-$VERSION-portmaster.zip}
    V="$VD/portmaster"; rm -rf "$V"; mkdir -p "$V/ports"
    unzip -q "$A" -d "$V/ports"
    P="$V/ports/babaport"
    [ -x "$V/ports/Baba Is You.sh" ] && [ -x "$P/baba-port.aarch64" ] || fail "launcher / binary missing or not executable"
    bash -n "$V/ports/Baba Is You.sh" || fail "launcher syntax"
    grep -q 'babaport' "$V/ports/Baba Is You.sh" && ! grep -q '@[A-Z]*@' "$V/ports/Baba Is You.sh" || fail "launcher template not filled"
    "$PY" -I -c 'import json,sys; d=json.load(open(sys.argv[1])); assert d["name"]=="babaport.zip" and d["items"]==["Baba Is You.sh","babaport"], d; print("port.json:", d["attr"]["title"], d["attr"]["arch"], "glibc", d["attr"]["min_glibc"])' "$P/port.json"
    python3 -I scripts/portmaster/check-glibc.py --max 2.17 "$P/baba-port.aarch64"
    file "$P/baba-port.aarch64" | grep -q 'ARM aarch64' || fail "not aarch64"
    mkdir -p "$V/shots"
    scripts/portmaster/run-qemu.sh "$P/baba-port.aarch64" --game-dir "$P/gamedata" --data-dir "$P/conf" \
        --fullscreen --size 640x480 --frames 20 --screenshot "$V/shots/no-data-640x480.png" > "$V/no-data.log" 2>&1 || true
    png_ok "$V/shots/no-data-640x480.png"
    echo "portmaster: layout, port.json, glibc 2.17, no-data screen under qemu ok"
    if have_game; then
        # The player's copy, as the README says (links to the read-only install).
        for f in Data Assets.dat gamecontrollerdb.txt bin64; do
            [ -e "$BABA_GAME_DIR/$f" ] && ln -s "$BABA_GAME_DIR/$f" "$P/gamedata/$f"
        done
        st=0
        for size in 640x480 720x720; do
            { echo "size $size"; cat packaging/portmaster/smoke.drive; } > "$V/smoke-$size.drive"
            scripts/portmaster/run-qemu.sh "$P/baba-port.aarch64" --drive "$V/smoke-$size.drive" --window \
                --render-every 6 --size "$size" --game-dir "$P/gamedata" --data-dir "$V/conf-$size" \
                --out "$V/shots/$size" --corpus "$BABA_ROOT/ref/tas/corpus/corpus.tsv" \
                > "$V/smoke-$size.log" 2>&1 || st=1
            tail -2 "$V/smoke-$size.log"
        done
        [ $st = 0 ] || fail "PortMaster smoke drive under qemu (logs in $V)"
        echo "portmaster: pad flow at 640x480 and 720x720 under qemu ok"
    fi ;;

android)
    A=${2:-$(ls "$DIST"/baba-port-"$VERSION"-android-arm64*.apk | head -1)}
    "$PY" -I scripts/android/apk.py verify "$A"
    m=$("$PY" -I scripts/android/apk.py dump-manifest "$A")
    echo "$m" | grep -q 'package="org.babaport"' || echo "$m" | grep -q "org.babaport" || fail "package name"
    echo "$m" | grep -q "$VERSION" || fail "versionName $VERSION not in the manifest"
    unzip -l "$A"
    unzip -l "$A" | grep -q 'lib/arm64-v8a/libbabaport.so' || fail "libbabaport.so missing"
    echo "android: v2 signature, manifest, contents ok ($(human "$A"))" ;;

*) echo "unknown platform $PLAT" >&2; exit 2 ;;
esac
