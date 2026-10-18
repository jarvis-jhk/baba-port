baba-port for PortMaster
========================

baba-port is a reimplementation of Baba Is You (Hempuli Oy). It does not
include the game: you need your own copy (Steam, GOG, itch.io; any OS).

Copy these from your Baba Is You install into ports/babaport/gamedata/:

    Data/                  (the whole folder)
    Assets.dat             (sound effects and engine images)
    gamecontrollerdb.txt   (optional)
    Baba Is You.exe        (Windows) or bin64/Chowdren (Linux): the names
                           of the sound effects are read from it; without
                           it the game plays music but no effects

From a Mac, copy Baba Is You.app itself into gamedata/ (it holds all of
the above). Copying the whole game folder works too.

Controls: d-pad or left stick move, A wait/confirm, B undo/back, Y undo,
X wait, R1/L1 restart, Start pause, Select back. Select+Start quits
(PortMaster hotkey). All pad buttons can be rebound in Settings > Controls.

Steam install folders: Linux ~/.steam/steam/steamapps/common/Baba Is You,
Windows C:\Program Files (x86)\Steam\steamapps\common\Baba Is You,
Mac ~/Library/Application Support/Steam/steamapps/common/Baba Is You.

Saves and settings go to ports/babaport/conf/. A log of the last run is in
ports/babaport/log.txt.
