#!/bin/bash
# Fails if anything from Baba Is You is in the repository or in a release
# archive. The repository is public: no sprites, music, sounds, levels, Lua
# code, extracted data, fonts or screenshots of the game (CLAUDE.md,
# "Nothing from the game goes into git"); the archives carry only our own
# binaries, SDL2 and text.
#
# Usage:
#   scripts/check-no-game-data.sh                  tracked files (git ls-files;
#                                                  outside a git checkout: as
#                                                  --worktree)
#   scripts/check-no-game-data.sh --worktree       the tree minus the top-level
#                                                  dirs .gitignore lists (no git)
#   scripts/check-no-game-data.sh --archives F...  release archives (.tar.gz,
#                                                  .zip, .apk), nested zips too
#
# Rules, for every file (in archives: every member, recursively):
#   1. file types: images, audio, fonts, the game's level/world/save/Lua
#      files and Assets.dat, by extension and by content (magic bytes,
#      incl. the level signature and Lua bytecode); executables and shared
#      libraries only inside archives, and only under the names we ship;
#   2. Lua source and Lua tables (the original's rules code and data
#      tables) by a syntax heuristic, in every text file;
#   3. any file over 256 KB unless allowlisted (scripts/check_no_game_data.py);
#   4. against the player's install ($BABA_GAME_DIR, when present; CI has
#      none and runs rules 1-3 and 5): a file identical to any file of the
#      install (SHA-256), or containing a 32-byte sample of one of the
#      install's data files: every 256 bytes of the Lua and language files
#      (an extract of a table is caught), 4 spread samples of everything
#      else (sprites, sounds, music, levels, Assets.dat), e.g. an asset
#      embedded into a binary with include_bytes!. Text samples must contain
#      spaces (prose/code), so the game's keys and object names, which the
#      port uses to look things up, do not count; the engine executables
#      and SDL's controller database are compared by hash only;
#   5. tracked paths below git-ignored private dirs (ref/, cache/, scratch/,
#      tools/, docs/spec/, target/, dist/).
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$here"
GAME_DIR="${BABA_GAME_DIR:-}"
MODE=tracked
case "${1:-}" in
    --worktree) MODE=worktree; shift ;;
    --archives) MODE=archives; shift ;;
    "") ;;
    *) echo "usage: $0 [--worktree | --archives FILE...]" >&2; exit 2 ;;
esac
if [ $MODE = tracked ] && ! { command -v git >/dev/null && git rev-parse --is-inside-work-tree >/dev/null 2>&1; }; then
    MODE=worktree
fi
PY=python3
[ -x "$here/tools/venv/bin/python" ] && PY="$here/tools/venv/bin/python"
if [ $MODE = tracked ]; then
    git ls-files -z --cached | "$PY" -I "$here/scripts/check_no_game_data.py" --mode tracked --game-dir "$GAME_DIR"
else
    "$PY" -I "$here/scripts/check_no_game_data.py" --mode "$MODE" --game-dir "$GAME_DIR" -- "$@"
fi
