# Third-party notices

baba-port itself is MIT OR Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`). The
release archives carry this file plus a `licenses/` folder with the full
texts listed below (in the Android APK: `licenses/` inside the package).

## Not part of this project

Baba Is You, its sprites, sounds, music, levels, fonts, texts and Lua code
are (c) Hempuli Oy (Arvi Teikari). baba-port contains none of it and is not
affiliated with or endorsed by Hempuli Oy; it reads the data from the
player's own copy at run time and never changes that copy.
`scripts/check-no-game-data.sh` (run in CI and by `scripts/package.sh`) keeps
images, audio, fonts, levels, Lua code, saves and anything identical to or
containing pieces of the game's files out of the repository and the release
archives.

The game's `gamecontrollerdb.txt` (SDL's community controller database, used
when present in the player's install) is likewise read from the install, not
shipped.

## Compiled into the binaries

- **Rust crates**: every crate linked into the program (sdl2, glow, lewton,
  png, ab_glyph, flate2/miniz_oxide, ureq, rustls, webpki-roots, serde,
  warpzone-core, android-activity on Android, ... and their dependencies)
  with its licence text is listed in `licenses/RUST_DEPENDENCIES.txt`,
  generated per release and per target by `packaging/gen-licenses.py` from
  `Cargo.lock`. All are permissive (MIT, Apache-2.0, BSD-2/3-Clause, ISC,
  Zlib, Unicode-3.0, CDLA-Permissive-2.0, Unlicense, 0BSD); the script fails
  the packaging if a copyleft licence without an alternative shows up.
- **ureq 3.4.2** (`crates/vendor/ureq`), MIT OR Apache-2.0, (c) Martin
  Algesten and contributors. Modified: its `rustls` feature no longer turns
  on `ring`, so the Android and PortMaster builds need no C compiler
  (`docs/platform-android.md`, "TLS without ring"). Licence texts in
  `crates/vendor/ureq/` and `licenses/RUST_DEPENDENCIES.txt`. Its test
  certificate and key (`src/unversioned/transport/testdata/`) are upstream
  test fixtures, not used by baba-port.
- **webpki-roots** carries Mozilla's CA certificate list (CDLA-Permissive-2.0)
  for HTTPS to the Warpzone server and the level server.
- The built-in 5x7 font and the touch-control icons (`crates/render/src/ui.rs`)
  are our own.

## Dynamically linked

- **SDL2** (https://libsdl.org), zlib licence, (c) Sam Lantinga and
  contributors. Text: `licenses/SDL2-LICENSE.txt` (`packaging/licenses/`).
  - Windows: the official SDL2 2.32.10 mingw release `SDL2.dll` next to
    `baba-port.exe`.
  - Linux: release archives built by CI carry SDL2 2.32.10 built from the
    official source release (`lib/libSDL2-2.0.so.0`,
    `scripts/ci/build-sdl2-linux.sh`); without it the system's SDL2 is used.
  - macOS: the official SDL2 2.32.10 `SDL2.framework` inside the app bundle.
  - PortMaster: the device's own SDL2 (nothing shipped).
  - Android: no SDL (NativeActivity, EGL, AAudio through the NDK's system
    libraries).

## Toolchain runtime code

- Windows: cross-compiled with [llvm-mingw](https://github.com/mstorsjo/llvm-mingw);
  the executable statically contains the mingw-w64 C runtime (public domain /
  ZPL 2.1), LLVM's libunwind and compiler-rt builtins (Apache-2.0 WITH
  LLVM-exception) and the Rust standard library (MIT OR Apache-2.0).
- Android: linked against the NDK's `crtbegin_so.o`/`crtend_so.o` and
  libunwind/compiler-rt builtins (Apache-2.0 WITH LLVM-exception / BSD-style
  Android NOTICE), plus the Rust standard library.
- Linux / PortMaster: the Rust standard library; glibc and libgcc_s are the
  system's (dynamically linked).

## Online play

Online play goes through [Warpzone](https://github.com/jarvis-jhk/warpzone)
(`warpzone-core`, MIT OR Apache-2.0, linked as a Rust crate and listed in
`licenses/RUST_DEPENDENCIES.txt`). Level codes are downloaded from Hempuli's
level server, as in the original game.
