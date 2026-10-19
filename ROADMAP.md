# Roadmap

What comes next for baba-port. The detailed state of every milestone is in
[docs/STATUS.md](docs/STATUS.md); this file is the short list of priorities.
The project is maintained by JARVIS; the owner adds items here.

## Current priorities

1. **Keep CI green** on all five platform jobs (Linux, Windows, Android,
   PortMaster, macOS) and the archive checks; releases go out from
   `release.yml` as drafts, are checked, then published as prereleases until
   the owner calls a version ready.
2. **Warpzone end to end** against the live server: install from the
   launcher, find the game data, play, host/join in all three modes,
   spectators (the PortMaster asset is download-only in the launcher).
   Descriptor changes go to the warpzone repository as PRs; keep
   `packaging/warpzone/baba-port.toml` in sync with it.
3. **Online, the missing pieces**: custom world packs in online sessions
   (install worlds only so far), joining a running public session from the
   room browser, chat, the settings menu inside sessions.
4. **Multiplayer polish**: credits rolls in sessions (banner only now),
   per-player key rebinding for the two keyboard halves, sound panning per
   view.
5. **Fault reports for public builds** through a public channel that files
   GitHub issues (private Android builds already report panics).
6. **Test lists for the owner's devices**: one batch for the Android tablet
   (launcher icon, fault report, the on-screen keyboard by touch, a
   Windows/Mac data zip import), and the
   final PortMaster handheld check (M12) only after everything else.

## Later

- macOS on real hardware (built and smoke-tested in CI, never run on a Mac);
  notarisation needs a paid Apple account, so builds stay ad-hoc signed.
- More New Adventures and Museum solutions in the test corpus (44/162 and
  17/88 verified; the main game is complete).
- Cosmetic gaps listed in `docs/presentation.md`: the original's own
  flower/blossom and background-particle images, sign placement, side faces
  in the 3D view.
- Level/pack thumbnails on the pack list, "Erase saved data on slot",
  clipboard in prompts.
- arm64 Mach-O executables with chained fixups (future Mac builds of the
  original) for reading sound names.
- Remove the vendored `ureq` once warpzone-core can leave `ring` out.

## Not planned

- Uploading levels to the original's level server (download only, unless
  the owner decides otherwise).
- Lua mods in custom world packs.
