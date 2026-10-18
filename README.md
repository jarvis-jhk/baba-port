# baba-port

A clean-room reimplementation of [Baba Is You](https://store.steampowered.com/app/736260/Baba_Is_You/)
in Rust, for Windows, Linux, macOS, Android and PortMaster handhelds: up to
12 players, on one screen with controllers, online through
[Warpzone](https://github.com/jarvis-jhk/warpzone), or both mixed.

**You need your own copy of Baba Is You.** baba-port contains no game data:
it reads the sprites, sounds, music, levels and object definitions from your
installation at run time and never changes it. Not affiliated with Hempuli
Oy. Player guide: [packaging/README-player.md](packaging/README-player.md)
(shipped as `README.md` in every release archive).

**Status: pre-release.** The whole game, local multiplayer, online play, the
level editor and level codes work; v0.1.0 is a prerelease (Linux, Windows,
Android, PortMaster; macOS built in CI, untested). Details in
[docs/STATUS.md](docs/STATUS.md).

Clean room: the behaviour was first described in our own words from the
original (specs kept private, they describe its internals), and the Rust code
was written from those descriptions.

## Features

- The whole game from your copy: the main game, New Adventures, the Museum,
  all 18 languages, endings; a rules engine written from behaviour specs and
  checked turn by turn against the original on hundreds of levels and
  thousands of random runs.
- Presentation measured against the original: wobble, particles, level
  rotation, transitions, palettes, music and sound rules.
- Your progress: imports the original's saves, settings and key bindings
  (Steam, Steam Cloud, GOG, Flatpak, Proton); three save slots in the
  original's format.
- Level editor and custom level packs (levels made here load in the
  original), level codes and featured levels from Hempuli's level server
  (download), packs from your copy or your own folder.
- Keyboard, any number of controllers (hot-plug), touch controls on Android.

## Multiplayer

Up to 12 players, each with a controller or half of a keyboard, any mix of
machines and local players online, in three modes:

1. **Co-op on the same Baba**: one game, everybody's moves are turns of the
   same level in arrival order; shared undo and restart.
2. **Shared save**: one save for the group, a splitscreen view per player;
   everyone plays any level, and what one solves opens up for all at once.
3. **Side by side**: everyone in their own game with their own save,
   splitscreen, just to share the experience.

Online play runs on Warpzone's encrypted peer channels: rooms, codes, invite
links, a room browser, late join, host migration and spectators (the
session layer is done, its screens in the game are being finished).

## Platforms

| Platform | Release archive | Notes |
|---|---|---|
| Linux x86_64 / Steam Deck | `baba-port-<v>-linux-x86_64.tar.gz` | `play.sh`, SDL2 bundled, glibc 2.35+ |
| Windows x86_64 | `baba-port-<v>-windows-x86_64.zip` | `baba-port.exe` + `SDL2.dll`, cross-built with llvm-mingw |
| macOS 11+ (Apple Silicon + Intel) | `baba-port-<v>-macos-universal.zip` | universal `.app`, ad-hoc signed (not notarised), built in CI |
| Android 8+ (arm64) | `baba-port-<v>-android-arm64.apk` | NativeActivity, no Java; import the game as a zip |
| PortMaster (aarch64 handhelds) | `baba-port-<v>-portmaster.zip` | glibc 2.17+, the device's SDL2/GLES2 |

Every release has a `SHA256SUMS`. The Warpzone launcher installs the Linux,
Steam Deck and Windows builds and finds your copy of the game for them
([packaging/warpzone/baba-port.toml](packaging/warpzone/baba-port.toml)).

## Building

Everything (toolchains, caches) stays inside the checkout:

```sh
scripts/bootstrap-tools.sh        # Rust + helpers into tools/
source env.sh                     # always, before cargo or the scripts
cargo run --release -p baba-port  # play (finds your Steam/GOG copy, or --game-dir PATH)
cargo test --workspace            # tests that need the game skip without $BABA_GAME_DIR
scripts/package.sh --verify       # release archives into dist/, checked and smoke-tested
```

Linux needs the SDL2 runtime library (`libsdl2-2.0-0`); per-platform
toolchains are fetched by `scripts/{windows,android,portmaster}/fetch-toolchain.sh`
on first use (llvm-mingw; 1.6 MB of NDK pieces, no Java or Gradle; a
glibc 2.28 sysroot). macOS builds run on GitHub's macOS runners
(`.github/workflows/build.yml`). Platform notes: [docs/platform.md](docs/platform.md)
and `docs/platform-*.md`; architecture: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

`scripts/check-no-game-data.sh` (CI, and every `scripts/package.sh` run)
keeps anything from the game out of the repository and the archives.

## Contributing, security, licence

[CONTRIBUTING.md](CONTRIBUTING.md), [SECURITY.md](SECURITY.md). The code is
MIT or Apache-2.0, at your option ([LICENSE-MIT](LICENSE-MIT),
[LICENSE-APACHE](LICENSE-APACHE)); third-party components:
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
