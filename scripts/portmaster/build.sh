#!/bin/bash
# Cross-compiles for PortMaster (aarch64-unknown-linux-gnu, hard glibc
# requirement <= 2.17, SDL2 + GLES2 from the device) and stages a PortMaster
# port directory + zip in target/dist/portmaster/.
# Usage: scripts/portmaster/build.sh [--probe] [--debug]
#   (default)  the game (crates/game, binary baba-port) as port "babaport"
#   --probe    the SDL probe (probes/sdl) as port "babaprobe"
set -euo pipefail
source "$(dirname "$0")/../../env.sh"
cd "$BABA_ROOT"
scripts/portmaster/fetch-toolchain.sh >/dev/null
source scripts/portmaster/env.sh
PROFILE=dist; FLAG="--profile dist"; PROBE=0
for a in "$@"; do
    case $a in
        --debug) PROFILE=debug; FLAG= ;;
        --probe) PROBE=1 ;;
        *) echo "unknown option $a" >&2; exit 2 ;;
    esac
done
T=aarch64-unknown-linux-gnu
OUT=target/dist/portmaster
P=packaging/portmaster

if [ $PROBE = 1 ]; then
    [ $PROFILE = dist ] && { PROFILE=release; FLAG=--release; }
    cargo build $FLAG --target $T --target-dir target/probes --manifest-path probes/sdl/Cargo.toml
    BIN=target/probes/$T/$PROFILE/baba-sdl-probe
    NAME=babaprobe SCRIPT="Baba Probe.sh" EXE=baba-probe TITLE="baba-port probe" ARGS="--frames 600"
    DESC="Platform probe for baba-port: SDL2 window, GLES2 quad, beep, controller log."
    INST="Nothing to install." RTR=true
else
    cargo build $FLAG --target $T -p baba-port
    BIN=target/$T/$PROFILE/baba-port
    NAME=babaport SCRIPT="Baba Is You.sh" EXE=baba-port TITLE="Baba Is You (baba-port)" ARGS="--fullscreen"
    DESC="Baba Is You, reimplemented in Rust (baba-port). Needs your own copy of the game's data."
    INST="Copy Data/, Assets.dat and gamecontrollerdb.txt from your Baba Is You install (Steam/GOG, any OS) into ports/babaport/gamedata/."
    RTR=false
fi
# Hard requirement glibc 2.17 (oldest CFWs); statx/getrandom are weak.
python3 -I scripts/portmaster/check-glibc.py --max 2.17 "$BIN"

S=$OUT/stage
rm -rf "$S"; mkdir -p "$S/$NAME/licenses"
cp "$BIN" "$S/$NAME/$EXE.aarch64"
cp LICENSE-MIT LICENSE-APACHE "$S/$NAME/licenses/"
sed -e "s|@TITLE@|$TITLE|; s|@SCRIPT@|$SCRIPT|; s|@DIR@|$NAME|; s|@BIN@|$EXE|g; s|@ARGS@|$ARGS|" \
    $P/launcher.sh > "$S/$SCRIPT"
chmod +x "$S/$SCRIPT" "$S/$NAME/$EXE.aarch64"
if [ $PROBE = 0 ]; then
    mkdir -p "$S/$NAME/gamedata"
    cp $P/README.md "$S/$NAME/README.md"
    cp $P/README.md "$S/$NAME/gamedata/PUT-GAME-DATA-HERE.txt"
    sed -e "s|@TITLE@|$TITLE|; s|@DESC@|$DESC|" $P/gameinfo.xml > "$S/$NAME/gameinfo.xml"
fi
python3 -I - "$S/$NAME/port.json" "$NAME" "$SCRIPT" "$TITLE" "$DESC" "$INST" "$RTR" <<'PY'
import json, sys
out, name, script, title, desc, inst, rtr = sys.argv[1:]
json.dump({
    "version": 4, "name": name + ".zip", "items": [script, name], "items_opt": [],
    "attr": {
        "title": title, "porter": ["baba-port"], "desc": desc, "inst": inst,
        "genres": ["puzzle"], "image": {}, "rtr": rtr == "true", "exp": True,
        "runtime": [], "availability": "free", "reqs": [], "arch": ["aarch64"],
        "min_glibc": "2.17",
        "store": [] if rtr == "true" else [
            {"name": "Steam", "gameurl": "https://store.steampowered.com/app/736260/Baba_Is_You/"},
            {"name": "GOG", "gameurl": "https://www.gog.com/game/baba_is_you"}],
    },
}, open(out, "w"), indent=2)
PY
(cd "$S" && rm -f "../$NAME.zip" && zip -qr -X "../$NAME.zip" .)
ls -l "$BIN" "$OUT/$NAME.zip"; unzip -l "$OUT/$NAME.zip"
