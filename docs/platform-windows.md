# Windows

Cross-built from Linux, checked under Wine; the setup is bopl-port's
(`~/Projects/boplbattle/scripts/*windows*`), adapted.

```
scripts/windows/fetch-toolchain.sh   # llvm-mingw 20261006 + SDL2 2.32.10 mingw dev release (pinned SHA-256)
scripts/windows/build.sh [--probe]   # x86_64-pc-windows-gnu -> target/dist/windows/{baba-port.exe,SDL2.dll}
EXE=target/dist/windows/baba-port.exe scripts/windows/run-wine.sh --drive SCRIPT --window ...
```

- Toolchain (`tools/windows`): llvm-mingw 619 MB (clang + lld + mingw-w64
  runtime), SDL2 dev release 89 MB, `gcc-shim` (libgcc/libgcc_eh mapped onto
  llvm-mingw's compiler-rt/libunwind for Rust's windows-gnu target), plus the
  rust-std component (139 MB). `scripts/windows/env.sh` sets linker, shim and
  SDL2 import-library paths; Windows builds use `target/windows`.
- `run-wine.sh`: Proton Experimental's wine, run directly inside bubblewrap
  (read-only `/`, read-write project, private `/tmp` and `/dev/shm`, no
  network unless `BABA_WINE_NET=1`, HOME/XDG/WINEPREFIX in `tools/`), X11
  driver on a private Xvfb (`:69`), host audio cut off (no
  winepulse/winealsa, no pulse socket). Proton's wine **drops `SDL_*`
  environment variables**; the script passes them as `BABA_SDL_*`, which the
  probe (and the game should) turn into SDL hints.

Verified 2026-10-18: probe exe (382 KB) + SDL2.dll (1.7 MB); under Wine SDL
2.32.10 "windows" video driver, GL "4.5 Compatibility, Mesa llvmpipe",
clear + textured quad read back OK, virtual controller button/axis events,
dummy audio.

**GL context finding**: a GLES 2.0 context is *not* available on Windows here
(SDL's EGL path needs ANGLE's libEGL.dll; Wine exposes no
`WGL_EXT_create_context_es2_profile`), and real Windows drivers vary. The
renderer therefore has to run on desktop GL 2.1/3.x as well: the probe keeps
one set of GLSL ES 1.00-style shaders and switches the header
(`#version 100` + precision vs `#version 120`). Shipping ANGLE (~5 MB) would
be the alternative and needs a reason.

## The game under Wine (2026-10-18)

`scripts/windows/build.sh` now builds the game (`-p baba-port`, profile
`dist`): `baba-port.exe` **1.98 MB** + `SDL2.dll` 1.7 MB (`--probe` for
the probe). Under Wine (SDL 2.32.10 "windows" video, GL "4.5 Core, Mesa
llvmpipe") the whole M4 flow test runs on the real SDL window:

```
EXE=target/dist/windows/baba-port.exe BABA_REF=.../scratch/ref/capture \
  scripts/windows/run-wine.sh --drive crates/game/tests/flow/1-fresh.drive \
  --window --render-every 2 --data-dir DATA --out OUT --corpus ref/tas/corpus/corpus.tsv
```

`1-fresh`, `2-menus`, `3-worlds-settings` in turn on one data dir: 27
screenshots, 0 failures, the comparisons with captures of the original
give the same numbers as the native Linux run (screens in
`scratch/android3/windows/`). Saves, settings, language switching, the
Japanese/Chinese fonts, the sound-name lookup and the corpus solutions all
work with Windows paths (`Z:\...`). The game maps `BABA_SDL_<X>` to SDL
hints like the probe. Note for agent worktrees: `run-wine.sh` only binds
`$BABA_ROOT` writable, so `--out`/`--data-dir` must lie inside the
worktree itself (not behind its `scratch` symlink).

## Release archive (M11)

`scripts/package/windows.sh` → `dist/baba-port-<v>-windows-x86_64.zip`
(`baba-port.exe`, `SDL2.dll`, README.txt, licences). Release builds use the
Windows GUI subsystem (no console window; `main.rs`); output still reaches
pipes and redirections. `scripts/package/verify.sh windows` checks the
imports (system DLLs + SDL2.dll only) and, where Proton is installed, runs
`--version`, the no-data screen and the M4 flow scripts under Wine.
