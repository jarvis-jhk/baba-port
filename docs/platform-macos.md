# macOS

No Mac here: macOS builds run on GitHub's macOS runners, job `macos` of
`.github/workflows/build.yml` (called by `ci.yml` and `release.yml`),
running `scripts/package/macos.sh`. Written 2026-10-18, **not yet run** (no
remote yet): the first private-repository run is its test.

## Build (scripts/package/macos.sh)

1. `macos-14` runner (Apple Silicon), Rust targets `aarch64-apple-darwin` and
   `x86_64-apple-darwin` (the x86_64 slice is cross-built with the same Xcode
   SDK), `MACOSX_DEPLOYMENT_TARGET=11.0`, profile `dist`.
2. SDL2: the official `SDL2-2.32.10.dmg` (universal `SDL2.framework`, SHA-256
   pinned in `scripts/macos/fetch-sdl2.sh`, same version as `SDL2.dll`).
   Linked without the sdl2 crate's `use_mac_framework` feature (no
   macOS-only Cargo changes): `target/macos-sdl2-link/libSDL2.dylib` is a
   symlink to the framework binary, passed as `BABA_SDL2_LIB_DIR` (honoured
   by `crates/platform/build.rs` for every target); the linker records the
   framework's install name `@rpath/SDL2.framework/Versions/A/SDL2`, and the
   executable gets the rpath `@executable_path/../Frameworks`. The script
   fails if `otool -L` does not show that.
3. `lipo -create` → universal binary; `baba-port.app` =
   `Contents/MacOS/baba-port`, `Contents/Frameworks/SDL2.framework` (copied
   with `ditto`, headers dropped), `Contents/Info.plist`
   (`packaging/macos/Info.plist`, id `org.babaport`, version from Cargo),
   `Contents/Resources/` with the player README, licences and
   `licenses/RUST_DEPENDENCIES.txt`.
4. No Developer ID / notarisation: `codesign --force --deep --sign -`
   (ad-hoc; arm64 code must carry at least that), verified, zipped with
   `ditto -c -k --keepParent` → `dist/baba-port-<v>-macos-universal.zip`.
   Players open it the first time with right-click → Open (or
   `xattr -dr com.apple.quarantine`), as the player README says.
   Notarisation needs a paid Apple Developer account (99 USD/year): the owner's
   decision.
5. Smoke test: `--version` of both slices (x86_64 through Rosetta) must
   pass; the no-data screen in a window (`--frames 30 --screenshot`) is
   `continue-on-error` because runner VMs may lack OpenGL. The archive is
   then checked with the others (`scripts/package.sh --checks-only`: no game
   data, SHA256SUMS).

Game data on macOS: `crates/game/src/gamedir.rs` looks in
`~/Library/Application Support/Steam/steamapps/common/Baba Is You` (and the
other Steam libraries) and accepts a folder with `Data/values.lua` or a
`.app` whose `Contents/Resources` has it. Checked against the Steam Mac
depot (2026-10-18, `tools/baba-mac`): `Baba Is You.app/Contents/MacOS/
{Chowdren (Mach-O x86-64, thin), libsteam_api.dylib}`,
`Contents/Resources/{Assets.dat (same bytes as Linux), Data/ (same as
Linux), gamecontrollerdb.txt, icon.icns}`; the sound names come from the
Mach-O (`baba_formats::exe`). Saves:
`~/Library/Application Support/baba-port`.

GL on macOS: only desktop OpenGL (legacy 2.1 or 3.2+ core, deprecated but
present), never GLES. The renderer runs on desktop GL contexts (see
`docs/platform-windows.md`); a 3.2 core context needs a VAO bound, which the
renderer does on core profiles.

## Checked from Linux

`scripts/macos/check.sh` runs `cargo check` for both Apple targets (no SDK
needed, nothing is linked): the SDL probe passes; `--workspace` passes for
every pure-Rust crate. `baba-net` is excluded there because it pulls in
`ring`, whose build script compiles C/assembly and needs clang plus the macOS
SDK headers; the CI builds it natively. Linking (`cargo build`) from Linux
fails as expected ("the SDK is needed by the linker").

## Open (first CI run)

- Whether the symlinked-dylib link trick and the rpath work as intended
  (fallback: enable `sdl2/use_mac_framework` for macOS in `crates/platform`
  and pass `-F` like the old probe workflow).
- Whether the runner can open a GL window for the screenshot.
- The Mac version of Baba Is You's folder layout for `gamedir.rs`.
