#!/bin/bash
# PortMaster launcher for @TITLE@ (installed as "ports/@SCRIPT@").
# Standard PortMaster preamble: find PortMaster, load control.txt (device,
# CFW, $GPTOKEYB, $ESUDO, get_controls -> $sdl_controllerconfig).
XDG_DATA_HOME=${XDG_DATA_HOME:-$HOME/.local/share}
if [ -d "/opt/system/Tools/PortMaster/" ]; then
  controlfolder="/opt/system/Tools/PortMaster"
elif [ -d "/opt/tools/PortMaster/" ]; then
  controlfolder="/opt/tools/PortMaster"
elif [ -d "$XDG_DATA_HOME/PortMaster/" ]; then
  controlfolder="$XDG_DATA_HOME/PortMaster"
else
  controlfolder="/roms/ports/PortMaster"
fi
source $controlfolder/control.txt
[ -f "${controlfolder}/mod_${CFW_NAME}.txt" ] && source "${controlfolder}/mod_${CFW_NAME}.txt"
get_controls

GAMEDIR="/$directory/ports/@DIR@"
cd "$GAMEDIR"
> "$GAMEDIR/log.txt" && exec > >(tee "$GAMEDIR/log.txt") 2>&1
echo "device: $DEVICE_NAME ($CFW_NAME), ${DISPLAY_WIDTH}x${DISPLAY_HEIGHT}, arch $DEVICE_ARCH, sticks $ANALOG_STICKS"

# Our own libs (only if bundled) first, then the device's SDL2/GLES/EGL.
export LD_LIBRARY_PATH="$GAMEDIR/libs.${DEVICE_ARCH}:$LD_LIBRARY_PATH"
# The game reads SDL game controllers directly; PortMaster's mapping for the
# built-in pad comes in through SDL_GAMECONTROLLERCONFIG.
export SDL_GAMECONTROLLERCONFIG="$sdl_controllerconfig"
export BABA_PORT_DATA_DIR="$GAMEDIR/conf"
mkdir -p "$BABA_PORT_DATA_DIR"
# The player's own copy of the game (README.md): Data/, Assets.dat, ...
export BABA_GAME_DIR="$GAMEDIR/gamedata"
# (any build: the folder's contents, the folder itself, or a Mac .app)
found=
for f in "$BABA_GAME_DIR"/Data/values.lua "$BABA_GAME_DIR"/*/Data/values.lua \
    "$BABA_GAME_DIR"/*.app/Contents/Resources/Data/values.lua; do
  [ -f "$f" ] && found=1
done
if [ -d "$BABA_GAME_DIR" ] && [ -z "$found" ]; then
  echo "No game data: copy Data/, Assets.dat and the game's program file from your Baba Is You install into $BABA_GAME_DIR"
fi

# gptokeyb without a key map: only the hotkey (select+start) kill.
$GPTOKEYB "@BIN@.${DEVICE_ARCH}" &
pm_platform_helper "$GAMEDIR/@BIN@.${DEVICE_ARCH}"
./@BIN@.${DEVICE_ARCH} @ARGS@
pm_finish
