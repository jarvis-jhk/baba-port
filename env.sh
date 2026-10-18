# Source this: keeps every toolchain and cache inside the project directory.
R="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export BABA_ROOT="$R"
export RUSTUP_HOME="$R/tools/rustup" CARGO_HOME="$R/tools/cargo"
export npm_config_cache="$R/tools/npm-cache" PIP_CACHE_DIR="$R/tools/pip-cache"
export XDG_CACHE_HOME="$R/tools/xdg-cache" WINEPREFIX="$R/tools/wineprefix"
export GRADLE_USER_HOME="$R/tools/gradle-home" ANDROID_USER_HOME="$R/tools/android-home"
export ANDROID_HOME="$R/tools/android-sdk" ANDROID_SDK_ROOT="$R/tools/android-sdk"
export TMPDIR="$R/scratch/tmp"
mkdir -p "$TMPDIR"
export PATH="$R/tools/cargo/bin:$R/tools/venv/bin:$R/tools/bin:$PATH"
# Location of the original game (read-only; never written to).
export BABA_GAME_DIR="${BABA_GAME_DIR:-$HOME/.steam/steam/steamapps/common/Baba Is You}"
# Dev: the port's user data dir (import, saves, settings, Warpzone identity) stays in the project.
export BABA_PORT_DATA_DIR="$R/cache/userdata"
export WARPZONE_HOME="$R/cache/warpzone"
# Android release signing key (git-ignored, backed up): used by scripts/package.sh when present.
if [ -d "$R/keys/android" ]; then export BABA_ANDROID_KEY_DIR="$R/keys/android"; fi
