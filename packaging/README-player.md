# baba-port

An open-source, clean-room reimplementation of **Baba Is You** for Windows,
Linux, macOS, Android and PortMaster handhelds: the main game, New
Adventures, the Museum, the level editor, level codes and level packs, plus
multiplayer for up to **12 players** on one screen, online, or both mixed.

**You need your own copy of Baba Is You** (Steam, GOG, itch.io or the Humble
Store; the Windows, Linux or Mac version all work). baba-port contains no
game data: every sprite, sound, song and level is read from your copy at
start, and your copy is never changed. baba-port is not made by, affiliated
with or endorsed by Hempuli Oy. Baba Is You is (c) Hempuli Oy.

## Starting

* **Windows:** unpack the zip anywhere and run `baba-port.exe`. It is not
  signed, so Windows SmartScreen may ask once ("More info" → "Run anyway").
* **Linux / Steam Deck:** unpack the archive and run `play.sh` (SDL2 is
  included in `lib/`). On the Steam Deck, add `play.sh` to Steam as a
  non-Steam game to play it from Gaming Mode.
* **macOS (11 or newer, Apple Silicon and Intel):** unzip and move
  `baba-port.app` to Applications. It is not notarised: the first time,
  right-click (or Ctrl-click) it → Open → Open. If macOS says the app "is
  damaged", run `xattr -dr com.apple.quarantine /Applications/baba-port.app`
  in Terminal once.
* **Android (8.0 or newer, 64-bit ARM):** install the APK (allow "install
  unknown apps" for your browser or file manager when asked), then import
  your game data (below).
* **PortMaster handhelds:** unzip the PortMaster zip into your `ports/`
  folder (it holds `Baba Is You.sh` and `babaport/`), copy your game data
  into `ports/babaport/gamedata/` (below) and start *Baba Is You* from the
  Ports menu.
* **Warpzone launcher:** install baba-port from the launcher's library; it
  finds your Baba Is You copy and hands it to the game.

## Finding your copy of the game

On Windows, Linux and macOS baba-port looks for the game by itself, in this
order:

1. `--game-dir PATH` on the command line, or the `BABA_GAME_DIR` environment
   variable;
2. what the Warpzone launcher found (`WARPZONE_DATA_BABA`);
3. a `gamedata/` folder next to the program (portable installs);
4. a `game/` folder in baba-port's own data folder (see "Your saves");
5. Steam: every Steam library (also Flatpak Steam and extra library folders
   on other drives), `steamapps/common/Baba Is You`;
6. GOG: `C:\GOG Games\Baba Is You`, GOG Galaxy's `Games` folder,
   `~/GOG Games/Baba Is You`, `~/Games/Baba Is You`; on macOS also
   `Baba Is You.app` in Applications.

The folder to point at is the one that contains `Data/` (on macOS the
`.app` works too). If nothing is found, baba-port shows where to put the
files; the simplest fix is to copy the game's folder into the `game/`
folder it names.

Which files baba-port reads (only reads; the same for the Windows, Linux
and Mac versions): `Data/` (levels, sprites, music, languages, the object
definitions), `Assets.dat` (sound effects and engine images),
`gamecontrollerdb.txt` (extra controller mappings, optional) and the
game's program file, only for the names of the sound effects:
`Baba Is You.exe` (Windows), `bin64/Chowdren` (Linux) or
`Baba Is You.app/Contents/MacOS/Chowdren` (Mac). On a Mac all of it is
inside `Baba Is You.app` (`Contents/Resources` and `Contents/MacOS`).

### Android: importing your game

1. On a computer, zip your game's folder (the one with `Data/` in it; any
   version; the whole folder is fine, baba-port takes only what it
   needs). On Windows: right-click the `Baba Is You` folder → Send to →
   Compressed (zipped) folder. On a Mac: right-click `Baba Is You.app`
   → Compress.
2. Copy the zip to your phone or tablet (USB, cloud drive, ...).
3. Close baba-port if it is open, then tap the zip in the Files app and open
   it with **baba-port** (or share it to baba-port). The import takes a few
   seconds and shows its progress; the zip can be deleted afterwards.

Alternatively copy the unpacked folder to
`Android/data/org.babaport/files/game` over USB. The game data and your
saves stay inside the app and are removed when you uninstall it.

### PortMaster: copying your game

Copy from your Baba Is You install (any version; Steam:
`steamapps/common/Baba Is You`) into `ports/babaport/gamedata/`:

* `Data/` (the whole folder)
* `Assets.dat` (sound effects and engine images)
* `gamecontrollerdb.txt` (optional)
* the program file, for the names of the sound effects:
  `Baba Is You.exe` (Windows) or `bin64/Chowdren` (Linux)

From a Mac, copy `Baba Is You.app` itself into `gamedata/` (it holds all
of the above). Copying the whole game folder is fine as well.

Without them the game shows this list on screen and quits on any button.
Saves and settings go to `ports/babaport/conf/`, a log of the last run to
`ports/babaport/log.txt`.

## Your saves and progress

baba-port keeps its own saves (three slots, the same format as the
original's) in:

* Windows: `%APPDATA%\baba-port`
* Linux / Steam Deck: `~/.local/share/baba-port`
* macOS: `~/Library/Application Support/baba-port`
* Android: inside the app; PortMaster: `ports/babaport/conf/`

**Importing your progress from the original:** on the first start
baba-port offers to import the original's saves, settings and key bindings
when it finds them (Steam, Steam Cloud, GOG, Flatpak and Proton locations);
later, use *Settings → Import Baba Is You progress*. The original's files
are only read.

## Controls

Every binding can be changed in *Settings → Controls* (keyboard and
controller).

| Action | Keyboard | Controller | Touch (Android) |
|---|---|---|---|
| Move | arrow keys or W A S D | left stick or d-pad | on-screen d-pad |
| Wait / confirm | Space or Enter | A or X | Wait button / tap a menu item |
| Undo | Z or Backspace | B or Y | Undo button |
| Restart | R | RB or LB | Restart button |
| Pause / back | P or Escape | Start; Select = back | Menu button; Android Back |

On handhelds Select+Start quits (the PortMaster hotkey). In the level
editor the mouse works as in the original, and controllers have their own
editor layout (*Settings → Controls*).

## Playing together

Main menu → **Multiplayer**: up to 12 players, each with a controller or a
half of the keyboard (left: W A S D, E or Space wait, Q or Z undo, R
restart, Tab pause; right: arrows, Enter wait, Backspace undo, 0 restart,
P pause). Press A / Start (or Space / Enter on a keyboard half) to join,
pick a colour and name, choose the mode, then everyone presses Ready. Three
ways to play:

1. **Co-op on the same Baba:** one game on one screen; every player's moves
   are turns of the same level, in the order they arrive. Undo and restart
   work for everybody.
2. **Shared save:** one save for the group, a splitscreen view per player;
   everyone walks the map and plays any level on their own, and whatever
   one player solves opens up for all at once.
3. **Side by side:** everyone plays their own game with their own save in
   their own splitscreen view, just to share the experience.

Controllers can be plugged in and out at any time; an unplugged player's
seat waits for any free controller.

## Online

Online play goes through [Warpzone](https://github.com/jarvis-jhk/warpzone):
host a room, share its code or invite link, and others join from the room
browser, with the code, or with `--warpzone-join <code or link>`; any mix
of online and local players up to 12, in all three modes, plus spectators.
If the host leaves, the room carries on with a new host. Everyone needs
their own copy of Baba Is You. Online play is still being finished; see the
project page for its state.

## Level codes, packs and the editor

* *Play levels → Get new levels*: type a level code (e.g. from the
  community) or browse the featured levels, then play it now or save it to
  your custom levels. Downloads come from Hempuli's level server, as in the
  original. *Level history* lists codes you used.
* *Play levels → Play levelpacks*: the packs in your copy plus packs you put
  into the `worlds/` folder of baba-port's data folder. Packs with Lua
  scripts play with the standard rules (Lua mods are not supported).
* *Level editor*: make levels and whole packs with maps, as in the
  original, with the lessons and every tool; levels saved here also load in
  the original game. Uploading levels is not available in this version.

## Command line

`--fullscreen`, `--game-dir PATH`, `--data-dir PATH` (saves elsewhere, e.g.
a USB stick), `--version`; `--help` lists everything (including the
developer options).

## Problems

* "needs your own copy of Baba Is You": the game was not found, see
  "Finding your copy of the game".
* Linux: "needs the SDL2 library": install `libsdl2-2.0-0` (or your
  distribution's SDL2 package).
* No sound effects, music plays: the game data lacks `Assets.dat` or the
  game's program file (see "Finding your copy of the game").
* Reports and questions: the project page's issue tracker.

## Licence

The code is MIT or Apache-2.0, at your option (`LICENSE-MIT`,
`LICENSE-APACHE`). Third-party components and their licences:
`THIRD_PARTY_NOTICES.md` and `licenses/`. Source code:
https://github.com/jarvis-jhk/baba-port
