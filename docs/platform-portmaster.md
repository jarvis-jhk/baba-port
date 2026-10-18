# PortMaster

Target: aarch64 Linux handhelds running PortMaster on custom firmwares
(Knulli, ROCKNIX, muOS, AmberELEC, ArkOS/dArkOS, …): glibc userland, SDL2
with the KMSDRM video driver, GLES 2 (Mali/PowerVR/Adreno/Panfrost), screens
of 640x480, 720x720, 1280x720, 1920x1152 etc. The real device (`gameboy`,
Knulli) is the final check only; everything here is emulated.

## Port format (PortMaster "packaging" docs + PortMaster-GUI sources)

```
ports/
  Baba Is You.sh            launcher (capitalised name, unique)
  babaport/                 port dir (lowercase, unique)
    port.json               metadata (schema version 4)
    gameinfo.xml            EmulationStation metadata
    README.md, screenshot.png (4:3, >= 640x480), cover.png (optional)
    baba-port.aarch64       binary, picked by $DEVICE_ARCH
    libs.aarch64/           bundled shared libs (only if needed)
    licenses/               licences of everything shipped
    conf/                   saves/settings (BABA_PORT_DATA_DIR)
```

- **port.json**: `version`, `name` (zip name), `items` (launcher + dir),
  `items_opt`, `attr` { `title`, `porter`, `desc`, `inst`, `genres`,
  `image`, `rtr` (ready to run: false for us, the player supplies the game
  data), `exp`, `runtime` (none: native binary), `store` (where to buy the
  game), `availability`, `reqs`, `arch: ["aarch64"]`, `min_glibc` }.
- **Launcher** (`packaging/portmaster/launcher.sh`): the standard preamble
  (find PortMaster, `source control.txt`, `mod_$CFW_NAME.txt`,
  `get_controls`), then `cd $GAMEDIR`, log to `log.txt`, `LD_LIBRARY_PATH`
  with `libs.$DEVICE_ARCH`, `SDL_GAMECONTROLLERCONFIG=$sdl_controllerconfig`,
  `$GPTOKEYB "<bin>" &` (no key map: only the select+start kill hotkey),
  `pm_platform_helper`, run, `pm_finish`. `control.txt`/`device_info.txt`
  export `DEVICE_ARCH`, `CFW_NAME`, `DISPLAY_WIDTH/HEIGHT`, `ANALOG_STICKS`,
  `DEVICE_RAM`, `ESUDO`, `CFW_GLIBC`.
- **Controls**: we read SDL game controllers directly (PortMaster supplies a
  correct mapping for the built-in pad through `SDL_GAMECONTROLLERCONFIG`);
  gptokeyb keyboard emulation is not needed. Multiplayer with extra USB/BT
  pads works the same way.
- **Graphics**: the device's SDL2 (KMSDRM) + EGL/GLES2. No gl4es: we never
  need desktop GL there. Fullscreen at the native resolution; the game must
  lay out for 4:3 (640x480), 1:1 (720x720) and 16:9 screens.
- **SDL2**: the CFW's `libSDL2-2.0.so.0` is used (versions differ per CFW;
  PortMaster's own build chroot uses 2.26.2). Our binary binds every
  referenced SDL symbol at load time (`BIND_NOW`), so the game must only
  call SDL functions that exist in the oldest SDL2 we support, or ship
  `libs.aarch64/libSDL2-2.0.so.0` built with KMSDRM (≈1.8 MB). Decide once
  `crates/platform` settles; the probe uses ≤ 2.0.14 API (virtual joystick
  only with `--virtual-pad`).
- **glibc**: PortMaster's build chroots are Ubuntu 22.04 (glibc 2.35), but
  older CFWs (ArkOS: 2.30) are still around. We link against **glibc 2.28**
  (Debian 10) and check the result: the probe's only hard requirement is
  `GLIBC_2.17`; `statx@2.28` and `getrandom@2.25` are weak (Rust's std looks
  them up at run time), so it loads on any glibc ≥ 2.17.
  `scripts/portmaster/build.sh` runs `check-glibc.py --max 2.17` and fails
  otherwise. Pitfall found with the game: any `f32::exp` links
  `expf@GLIBC_2.27` (glibc 2.27 re-versioned `expf`); `baba-platform`
  defines `expf` and `powf` itself on aarch64-gnu (via `exp`/`pow`
  `@GLIBC_2.17`, `crates/platform/src/glibc_compat.rs`). Other re-versioned
  maths functions (`exp2f`, `logf`, `log2f`, `sinf`/`cosf` …) would need
  the same if they ever appear; the check catches them.

## Toolchain (no cross compiler)

`scripts/portmaster/fetch-toolchain.sh` (everything in `tools/portmaster/`,
`.deb`s checked against Debian's `Packages` SHA-256 by
`scripts/portmaster/debfetch.py`, then deleted):

| | Size |
|---|---|
| `sysroot/lib`: glibc 2.28 crt objects + libc/libm/libdl/libpthread/librt/libutil, libgcc_s/libgcc.a, `libSDL2.so` (2.26.5, link only) | **4.6 MB** |
| Rust `rust-std` for `aarch64-unknown-linux-gnu` | 162 MB |
| `rust-lld` | 0 (host toolchain) |
| *test only:* `qemu-aarch64-static` (Debian 12) | 13 MB |
| *test only:* `rootfs` (Debian 12 arm64: SDL2 2.26.5, Mesa 22.3 llvmpipe EGL/GLES2 + deps, 79 packages) | 248 MB |

Linking: `-C linker-flavor=ld.lld`, `scripts/portmaster/ld.sh` runs
`rust-lld` with `Scrt1.o crti.o crtbeginS.o … crtendS.o crtn.o` and
`/lib/ld-linux-aarch64.so.1` as interpreter (what gcc's driver adds). zig cc
was the alternative; a 4.6 MB sysroot plus rust-lld is smaller and needs no
wrapper for Rust's `-lgcc_s` expectations. A C dependency (none so far)
would need a cross clang; llvm-mingw's clang in `tools/windows` could serve
with `--target=aarch64-linux-gnu --sysroot`.

## Verified (2026-10-18)

- `scripts/portmaster/build.sh`: probe cross-built (364 KB binary), glibc
  check passes, port staged and zipped: `target/dist/portmaster/babaprobe.zip`
  (190 KB).
- `scripts/portmaster/run-qemu.sh`: the aarch64 binary runs headless under
  qemu-user against the Debian 12 arm64 rootfs: SDL 2.26.5 offscreen video
  driver, EGL surfaceless, "OpenGL ES 3.2 Mesa 22.3.6 llvmpipe"; renders the
  clear colour + textured quad, reads it back (pixels OK) at 640x480 and
  720x720; SDL virtual game controller → `CONTROLLERBUTTONDOWN/UP A`,
  `LeftX` axis events; dummy audio. ~5 fps under emulation.
- Not verified: KMSDRM on real hardware, the device's own SDL2 version,
  gptokeyb hotkey, the launcher on a CFW. That is the final `gameboy` check.

## The game port (2026-10-18)

`scripts/portmaster/build.sh` (`--probe` for the probe) cross-builds
`baba-port` (`crates/game`, profile `dist`) and stages port `babaport`:

| | Size |
|---|---|
| `target/dist/portmaster/babaport.zip` | **962 KB** (961,841 B) |
| `baba-port.aarch64` (stripped) | 1.78 MB |
| hard glibc requirement | 2.17 (weak: 2.18, 2.25, 2.28) |
| newest SDL2 function bound | `SDL_JoystickAttachVirtual` (2.0.14; dev helper, bound at load) |

The binary is the full game (`baba_game::App` on `SdlPlatform`, the same
as desktop); the growth over the M2 skeleton (514 KB zip) is the game flow
(menus, progression, fonts, audio rules).

Contents: `Baba Is You.sh` (launcher; runs `baba-port.aarch64 --fullscreen`,
sets `BABA_GAME_DIR=gamedata/`, `BABA_PORT_DATA_DIR=conf/` for saves and
settings, logs a hint when `gamedata/` is empty),
`babaport/{baba-port.aarch64, port.json (rtr false, store: Steam + GOG,
min_glibc 2.17), gameinfo.xml, README.md, gamedata/PUT-GAME-DATA-HERE.txt,
licenses/}` (templates in `packaging/portmaster/`). The player copies
`Data/`, `Assets.dat`, (optionally) `gamecontrollerdb.txt` and the game's
executable (sound effect names: `Baba Is You.exe` or `bin64/Chowdren`)
from their own install of any build into `ports/babaport/gamedata/`; from
a Mac, `Baba Is You.app` itself; a whole game folder one level down
works too (`GameInstall::locate`). Without data the game shows where to put
it on screen (built-in font) and quits on any button.

Controls: SDL game controller with PortMaster's mapping for the built-in
pad (`SDL_GAMECONTROLLERCONFIG`): d-pad / left stick move, A wait/confirm,
B undo (back in menus), X wait, Y undo, R1/L1 restart, Start pause,
**Select back** (the game's Escape, unless rebound), all rebindable in
Settings > Controls. Select+Start quits through gptokeyb's hotkey (the
game sees a pause/back first, harmless). Guide/hotkey is unused.

Small screens: the game's UI canvas is 854x480 logical, scaled to fit
(640x480: 0.75, 720x720: 0.84, text 12-13.5 px). List menus (pause,
settings, slots, packs, …) on screens narrower than 16:9 grow into the
unused height as far as their widest line fits (`Menu::ui_for`): the pause
menu is drawn at 1.0 on 640x480 and 1.5 on 720x720; the title grid and
the HUD keep the canvas scale.

Verified under qemu-user (Debian 12 arm64 rootfs, SDL 2.26.5 `offscreen`,
Mesa llvmpipe GLES 3.2 via EGL surfaceless, dummy audio), the real SDL
window path driven by a script (`baba-port --drive SCRIPT --window
--render-every 6 --size WxH`; `scratch/android3/portmaster/flow.drive`):
first launch by pad (A skips the boot card) → `0level` (pad move + undo,
then solved) → return to the map → Start = pause, B closes, Select = pause
→ Return to menu → title → Settings → back → Continue → map → walk to
`1level`, A → title card → level. 7 screenshots each at 640x480 and
720x720, 0 failures, ~70 s wall time per run
(`scratch/android3/portmaster/{640x480,720x720}/`); launcher-style run
(`--fullscreen --frames 200`, title) and the no-data screen
(`no-data-640x480.png`) as well. TLS on this target: the
pure-Rust provider (`docs/platform-android.md` "TLS without ring");
`baba-net`'s `tls_check` example, cross-built, fetched the public Warpzone
server's info over HTTPS under qemu.

Still open: a screenshot/cover for the port listing, a layout that uses
the full height of 4:3/1:1 screens for the title and HUD (M5), real
KMSDRM hardware and the device's SDL2/gptokeyb (the final `gameboy`
check).

## Release archive (M11)

`scripts/package/portmaster.sh` turns the staged port into
`dist/baba-port-<v>-portmaster.zip` (`Baba Is You.sh` + `babaport/`, licence
texts in `babaport/licenses/`); players unzip it into `ports/`.
`scripts/package/verify.sh portmaster` checks layout, launcher, `port.json`
and glibc, and runs the no-data screen and `packaging/portmaster/smoke.drive`
(pad flow) under qemu at 640x480 and 720x720.
