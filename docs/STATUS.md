# Status

Living plan. Update when a step finishes. Priorities: `ROADMAP.md`;
maintainer log: `NOTES.md` (JARVIS owns the project since 2026-10-18).

## Milestones

- [x] M0 Repo (+ private spec repo in `docs/spec/`), toolchains in `tools/`,
      backup wired (`.rsync-filter`, `.rsync-prescript`), quota-loop armed.
- [x] M1 📣 Formats: every level of every world, palettes, sprites, themes,
      `Assets.dat`, saves and object definitions read by `baba-formats`;
      contact sheet of every level rendered from the player's install.
      Done 2026-10-18: `crates/formats` (data-only Lua-table reader, `.l`
      read/write, `.ld`, slot overrides, palettes, sprites, themes,
      languages, saves/settings, `Assets.dat` images/sounds/shaders);
      600/600 levels, 0 unknown tile ids, 0 missing sprites; all 3206
      `Assets.dat` images decode. Docs: `docs/formats/*.md`. Sheets:
      `cargo run --release -p baba-formats --example levels_sheet -- --assets`
      → `scratch/m1/levels-sheet-<world>.png`. Open: layer parameter bytes
      and save state numbers not interpreted; `Assets.dat` sound names live
      only in the executable.
- [x] M2 📣 Renderer + platform: SDL2 + GLES2-level renderer shows a level
      (compared with a capture of the original); headless screenshots;
      controllers, keyboard, audio (music, sounds).
      `baba-render` (glow, ES 2 feature level, run-time atlas, wobble, N
      viewports, effect pass, EGL surfaceless headless, CPU reference),
      `baba-platform` (`Platform` trait + SDL2 backend, hot-plug controllers,
      streaming Ogg mixer), `baba-port --headless --screenshot`. `baba/0level`
      vs the original at 854x480: pixel-exact except the tutorial "controls"
      special (not drawn yet). Details, numbers and gaps: `docs/platform.md`.
- [x] M3 📣 Rules engine: rule parsing, movement, every property, verb and
      condition, conversions, undo; differential tests against the original
      Lua harness; every level of every world solved by replaying TASes /
      solution corpora.
      Done 2026-10-18: per-turn state identical to the oracle on all 246
      corpus traces and 1787/1788 random-input runs over all 596 levels (the
      one left: the original's own undo crashes in new_adv/56level). Solution
      corpora: main game 217/218 map-shown levels, New Adventures 44/162,
      Museum 17/88 (baba-solver + hand solutions, all oracle-verified, in
      `ref/tas/corpus/`); more solutions are a low-priority follow-up.
      Reference (2026-10-18): the original runs headless and offline
      (`scripts/dev/ref-run.sh`, no Steam, home in `tools/baba-home`; see
      `docs/reference-capture.md`). Lua oracle in `docs/spec/oracle/` (private;
      the game's own Lua in Lua 5.3.4 + engine shim, JSONL per turn, format in
      its README): load+40 random inputs run clean on all 596 levels; checked
      turn by turn against a traced copy of the real game on 34 levels of all
      three worlds (identical states, incl. undo/restart/conversions/letters;
      TELE and `pairs()`-order IDs are random in the original itself).
      Corpus `ref/tas/corpus/` (from baba-is-optimized, rebuild with
      `scripts/dev/corpus_build.py` + `corpus_verify.py`): 218/236 main-game
      levels verified winning in the oracle (217/218 of those a map shows); New
      Adventures and Museum have no public solution set: `crates/solver`
      (`baba-solver`, dev tool) + hand plans from a Steam prose guide give
      `ref/tas/corpus/solved.tsv`, every entry oracle-verified
      (`scripts/dev/corpus_solved.py`): New Adventures 44/162, Museum 17/88
      (2026-10-18; details in `docs/reference-capture.md`).
      Sim core (2026-10-18): `crates/sim` (`baba-sim`), design in
      `docs/sim-design.md`. Rules (letters, grammar/restarts, NOT, ALL,
      MIMIC, groups, WORD), all conditions, 8 movement takes with
      push/pull/swap/specials, FALL, FOLLOW/BACK/TELE/SHIFT, HOLD, SELECT +
      map paths, status pass, every block/level-block effect, conversions,
      HAS/MAKE, level conversion, undo with its quirks, seeded PRNG,
      `state_hash`. Results: corpus 241/241 solutions reach the oracle's
      outcome on the same turn (`sim-corpus`; win / level conversion / END /
      ALL IS DONE; 5 skipped: win-seed, nowin).
      Second pass (2026-10-18): 3D mode (camera mover, target cycle, fixed
      camera, undo), AUTO (keys store, `Input::Tick` runs the auto turn),
      presentation events (tweens with move kinds, create/remove/convert
      causes, particle bursts with effect-history scale, turn/removal sound
      slots with `_short`, rule-formed bling, PLAY, camera, text
      active/inactive/disabled, colour properties; cosmetic rolls left to
      `CosmeticRng`). The oracle got `--port-rng` (the sim's PRNG in place
      of `math.random`) and `--sorted-pairs` (port hash order), AUTO ticks
      (`A`), and its two gaps fixed; `sim-scenario` turns ASCII scenarios
      (`crates/sim/scenarios/`, 14: 3D, AUTO, group corner cases,
      OFTEN/SELDOM in the visual passes) into real levels for both sides.
      Now identical turn by turn (units, rules, win, no-player, PRNG
      state): 246/246 corpus traces (was 236); random runs with undo/
      restart/AUTO ticks over all 596 levels × 13 seeds (80–250 inputs,
      7 748 runs; was 691/710 on a subset): every divergence fixed and
      re-checked, final binary 1 787/1 788 (3 seeds × 200), the one left
      is the original's own undo crashing (`new_adv/56level`, review §G
      40); scenarios 14/14 plus 280/280 random-input scenario runs. Fixed on the way: PRNG seed count,
      level-start rule tests and re-parse, player check (3D target / stop),
      group refresh destroying oscillating recursive groups, BACK replay
      steps, WORD ids in undo records. Spec fixes logged in
      `docs/spec/review.md` §G. Details: `docs/sim-design.md`.
      Open: New Adventures/Museum solution corpora; map progression around
      the cursor (game flow).
      Earlier state (M3 core):
      ALL IS DONE; 5 skipped: win-seed, nowin); per-turn state identical to
      the oracle on 236/246 corpus traces and 691/710 random-input runs
      with undo (`sim-diff`, `scratch/sim/*.sh`); all remaining
      differences are documented deviations (TELE/CHILL/OFTEN/SELDOM
      randomness, hash-order ids of X IS ALL / MAKE grouping) or not yet
      implemented (3D, AUTO via `Input::Auto`), plus two harness gaps
      (units converted into `level` get an empty name; gated map paths are
      walkable). Tests: 93 spec-scenario tests, 600 real levels × 40 random
      turns deterministic (clone/replay/hash), 16 992 undo steps verified.
      Open: 3D, presentation events (sounds/particles/colours), group corner
      cases without oracle coverage, the rest of the New Adventures/Museum corpora.
- [x] M4 📣 Single player, complete: title, menus, settings, map navigation
      and progression, saves (3 slots), transitions, endings, all worlds.
      Game flow done 2026-10-18 (details `docs/game-flow.md`):
      `crates/progress` (`baba-progress`: save model in the original's key
      layout, settings/bindings, map layouts, unlock flood fill, gates,
      counters, level tree/labels; 17 tests) and `crates/game` as a library
      (`baba_game::App`, platform-agnostic, SDL2 behind the default `sdl`
      feature) + the `baba-port` binary: boot card, title, main/slot/erase/
      pack menus, settings (volumes, repeat delay, toggles, zoom),
      controls with keyboard/gamepad rebinding, 18 languages through the
      game's TTFs (ab_glyph, on-demand CJK glyphs), pause with rules, map
      loading with saved states, cursor placement, entering (title card),
      winning with the return/unlock and gate animations, level
      conversions, Return to map, restart prompt, key repeat, audio rules
      (music selection/continuity, no-player state, turn sounds from sim
      events; effect names read from the player's executable), credits,
      structural END / ALL IS DONE / pack endings, all three worlds.
      Flow test `scripts/dev/flow-test.sh` (3 drive scripts, 27 screenshots,
      0 failures; comparisons with captures in `docs/game-flow.md`).
      Closed 2026-10-18 (M5b, `docs/presentation.md` "Second pass"): the
      intro cutscene, the scripted END / ALL IS DONE sequences and pack
      endings (victory, then the pack's credits), LEVEL IS AUTO
      (`Turn::Auto` every `rhythm`+1 frames, measured), signs, map-clear
      showcase and undo hint verified in live runs (seeded flower-garden
      clear → 106level showcase → back; `2level` death → hint after 160
      frames), pixel font everywhere it exists. Flow test: intro shots +
      skip, END (blossoms, island, logo credits), All Is Done.
- [x] M5 📣 Presentation fidelity: particles, effects, wobble, level
      rotation, palettes, themes, all audio, all 18 languages.
      Measurements and effects done 2026-10-18 (details
      `docs/presentation.md`, values in the private spec
      `presentation.md` §15): the original was measured headless with a
      private probe (per-frame object values) and recordings matched
      against `Assets.dat` images. `crates/game/src/fx/` (per level view:
      movement tween, FLOAT bob, wobble/blink every 10 frames, background
      colour fade, shake, every particle kind with measured motion/life and
      its engine images, continuous property particles via
      `Level::effect_sources`, move puffs, rule bling, return/gate/showcase
      particles, background particle styles, animated level rotation and
      scroll, the measured blob iris 41/200/50 frames, a 3D first-person
      view), `art.rs` (engine images), `pixelfont.rs` (small and large
      bitmap fonts for pixel-font languages), `title.rs` (title plate,
      logo), engine-art map icons (body, numbers, letters, dots), rule
      crosses, tutorial key caps, title card, undo/restart hint, map-clear
      showcase; turn sounds from the sim's slots incl. PLAY notes.
      Comparisons (854×480, share of pixels differing by > 8): `0level`
      0.00 % wobble-matched (pixel exact; 0.42 % with our random phases),
      main map 0.27 % wobble-matched (HUD counter icons), New Adventures map
      2.30 %, Museum map 2.02 % (states differ from the captures), pause
      7.65 % (different level), title screen: logo and menu within 1–2 px,
      background differs only by scroll phase; 3D view 25 % against a
      capture of the same moment (walls drawn as fronts).
      Open: intro cutscene and the scripted END / ALL IS DONE sequences
      (credits roll as before), gate rings of counter symbols, HUD counter
      icons, high-contrast palettes, menu slides, conversion poof, 3D wall
      side faces/trapezoid floors, exact images of background particles.
      Merged with M6/M9/platforms: effects are fed by `Play::apply` (each
      splitscreen view has its own `LevelFx`, ticked and drawn with its
      shake/scroll/rotation), the map-clear showcase steps are `Proceed`
      turns (`Pending::Showcase`); open there: the 3D view, undo hint,
      engine-art title card and iris inside splitscreen views.
      Second pass done 2026-10-18 (M5b; details `docs/presentation.md`,
      values in the private spec §15.20–§15.33, captures/tools in
      `scratch/m5b/`): `crates/game/src/cutscene/` (intro; END in the level
      with blossoms to collect, the island show with area and spreading
      flowers, THE END, the original's logo-credits script, scroll
      decorations; All Is Done), `overlays.rs` (shared by single player and
      every splitscreen view: bitmap title card, undo hint, HUD counter art,
      signs, iris per view, 3D view), gate rings, conversion poof, menu
      slides (35 %/frame, measured), high contrast (`contrastpalette` /
      `contrast.png`, exact), win/clear banner art, dot icons 1–9,
      decorative map flowers, `Renderer::draw_overlay_rotated`; 4:3/1:1:
      title laid out for its 600 px span, HUD in the corners, wider title
      card. Comparisons (854×480, > 8 per channel): high-contrast `0level`
      0.43 %, win banner 5.79 % (TTF text before), main map 2.53 % plain,
      meta map 5.83 %, editor start 10.60 % (was 24.54 %), editor settings
      35.96 % (41.53 %); intro/END/All Is Done timelines identical to the
      probe logs frame by frame. Open (cosmetic): the original's own
      flower/blossom images (ours are drawn shapes in the measured colours
      and sizes), background-particle images and sway, sign placement
      (not captured), 3D side faces.
- [x] M6 📣 Local multiplayer, all three modes, up to 12 players.
      Done 2026-10-18 (details `docs/game-flow.md`, "Multiplayer"): every
      instance change is a `session::Turn` (≤ 64-byte encoding) applied
      through `Play::apply` (single player too); `Play::state_hash`.
      Join screen (main menu → Multiplayer): 12 seats, controllers with
      hot-plug and both keyboard halves, colour/name, mode, save/world,
      ready; unplug → seat kept, any free device takes it over; leave and
      rejoin with the same device. Mode 1 co-op (one instance, turns in SDL
      arrival order, shared undo/restart/map, global pause), mode 2 shared
      save (own cursor/level/tree each, progress reaches everyone's map at
      once incl. gates), mode 3 own games (slots 1–3 + nine extra slots
      `3ba.ba`–`11ba.ba`). Splitscreen 1–12 views (tiling for 16:9, 4:3,
      640×480; zoom toggle), per-view HUD/title cards/transitions/pause,
      one music track, deduplicated turn sounds. Tests: unit (input
      sequencing, layouts, turn encoding, extra slots, shared-save
      propagation and turn replay on the install) and 9 drive scripts
      (`scripts/dev/mp-test.sh`, SDL virtual controllers, 2/4/12 players
      per mode; 0 failures), M4 flow test still green. Screens:
      `scratch/m6/mp/*/`. Open: settings menu inside sessions, credits
      rolls in sessions (banner only), per-player key rebinding for the
      keyboard halves, sound panning per view, online (M7: the `Replica`
      over `Play::apply`).
- [x] M7 📣 Online over Warpzone, all three modes, 12 players, mixed with
      local players.
      Session layer done 2026-10-18: `crates/net` (`baba-net`, game-agnostic:
      lobby, build/content check, host-sequenced shared stream + owner
      streams, late join by replay, host migration, per-stream desync
      hashes, spectators), `packaging/warpzone/baba-port.toml` (check-games
      ok), `scripts/net/{dev-server,netsim-matrix}.sh` (ports 3036/3516);
      13 in-process tests + 17-case multi-process matrix pass with the toy
      game. Details: `docs/net.md`.
      The game online 2026-10-18 (details `docs/game-flow.md` "Online",
      `docs/net.md` "The game on baba-net"): main menu → Online and the
      join screen's O / Y (local seats come along); host private/public,
      join by code, room browser, invite links, `--warpzone-host` /
      `--warpzone-join`, refusals of other builds / game data explained;
      the room (this machine's players, the machines, mode/save/world by
      the host, ready, start, F5). `session::online`: the game as a
      `Replica` over `Multi`/`Play::apply` in all three modes (mode 1: one
      instance on the host's save, leader sends Proceed, transitions wait
      for their turn, prediction for far guests; mode 2: own save copies,
      host-sequenced `Progress` events merged at `sync`, `Turn::Refresh`;
      mode 3: saves in the players' streams), local views large / remote
      small (F3), late join (quiet replay), host migration, machines
      leaving, desync → resync by replay or leave with a message.
      baba-net: protocol 2 (32 KiB settings, own-stream events), fix for
      machines that add players after joining. Tests: 7 new unit/in-process
      tests (+1 baba-net), `scripts/net/online-test.sh` 19/19 (2 × 1, 3 × 4 and 2 + 4 + 6 in
      every mode, lossy, forced relay, leave, late join, host leaving,
      desync, version mismatch, menus + room browser; screens in
      `scratch/m7/online/`);
      M4/M6/M9 drive tests and the netsim matrix still green.
      Open: custom packs online (install worlds only), joining a running
      public session from the browser, chat, settings menu in sessions.
- [x] M8 📣 Level editor and custom world packs.
      Done 2026-10-18 (details `docs/game-flow.md` "Level editor (M8)"):
      `crates/game/src/editor/` (model, tools, undo, view, pad, tutorial,
      files) + `app/editor_ui.rs` (screens, hooks); `baba-formats` writes
      `.ld` (`LevelInfo::to_ini`/`to_text`, `[tiles]` writer,
      `ObjectChange::between`, `Ini::set`/`replace_section`). Main menu →
      Level editor: start screen, help & settings (advanced words, also
      unlocked by ALL IS DONE; extended features on by default), 10
      lessons + first-time intro from the language file with demo edits,
      shortcut list, editor pad bindings (`[gamepad_editor]` /
      `[gamepad_currobjlist]`, alt combos, the original's hat values);
      custom levels and user packs (create, rename, author, first level,
      world map, sort, search, delete, copy; install packs edited as a
      copy in the data dir); main view in the original's arrangement
      (quickbar, layers, 7 tools, 3 layers, facing, copy/cut/drag/clear,
      line/rect/fill rect/flood fill with preview, select with rotate/
      mirror/paste/ignore-empty, move all, show dirs, I/U/Y/Q, F4 words,
      swap cycles); palette (150 cap, pairs, full/paired grid in the
      original's order, search, tags, drag, remove, object list 60/page,
      edit object: sprite, name, type, colours, animation, text type,
      z-layer, reset); level settings (name, author, subtitle, music,
      particles, palette, size + presets, effect toggles, AUTO delay as
      `rhythm`, map setup); themes (load/save/delete); map editing (level
      icons with target/colours/state/symbol/custom icon picture, paths
      with gates, specials, map cursor); editor undo; test play through
      `Play::single` (the normal turn path, results not recorded) back to
      the editor; saving `.l`/`.ld`/preview `.png`; upload wired to the
      `upload` feature (never exercised against the server); mouse,
      keyboard, touch (taps, on-screen keyboard in prompts) and gamepad.
      Compatibility: all 600 installed levels load and re-save to the same
      content (`install_levels_round_trip`); a level and a pack with a map
      made in our editor load, render, test-play and win in the original
      (headless, `tools/baba-m8`, captures `scratch/m8/ref/`). Tests:
      `scripts/dev/m8-test.sh` (4 drive scripts, 39 screenshots,
      6 compared with captures, 0 failures), M4 flow / M9 / M6 still green.
      Open: report button (M9 gate). Closed in M5b: button art (cut
      corners, editor colours from the level palette, tall start-screen
      buttons, information lines as text), slide-in menus, tutorial pointer
      arrows (the original's targets), the map's own icon pictures in Map
      setup ("Map icon setup": `[icons]` slots).
- [x] M9 📣 Online level codes (download; upload only with the owner's approval),
      import of the original's save.
      Done 2026-10-18 (details `docs/game-flow.md` "M9"): save import
      (`baba_game::import`: Linux/Flatpak/Proton, Windows `%APPDATA%`,
      macOS `~/Library/Application Support`, Steam Cloud syncs into the same
      folder; slots copied group by group with counters re-validated,
      settings/bindings/editor pads/slot names/code history merged; first
      start offers it, Settings has "Import Baba Is You progress"); "Play
      levels" menu (custom levels, levelpacks, get new levels, history,
      slot name); level codes (`crates/http` = `baba-http`, ureq/rustls;
      `baba_game::levelcodes`: download → save into `<data>/worlds/levels`
      or play now, featured list with thumbnails, history, `[levels_won]`,
      all error strings; record/replay cache + offline mode for tests);
      upload/report behind the off-by-default `upload` feature, tested
      against a local mock only; packs (`baba_game::packs`) from
      `<data>/worlds/` and the install, Lua packs marked and started after
      a one-time notice. Imported save checked against the original
      running the same save (same maps, states, counters).
      Tests: 17 unit tests + `scripts/dev/m9-test.sh` (2 drive scripts,
      17 screenshots, 0 failures; inputs in `scratch/m9/`).
      Open: level/pack thumbnails on the pack list, report button
      (gated), "Erase saved data on slot", clipboard, custom
      `[levels_won]` key layout unconfirmed.
- [x] M10 📣 Platforms: Windows (Wine-checked), macOS (CI), Android (APK,
      touch, tested on the tablet), PortMaster (aarch64, emulated).
      Pipelines proven with probes (`docs/platform.md`, `platform-*.md`):
      Android NativeActivity without Java/Gradle/JDK/build-tools/clang
      (270 KB APK, 1.6 MB NDK pieces; GLES2, AAudio, touch, uinput gamepad,
      immersive via JNI, all on the tablet); PortMaster aarch64 via rust-lld
      + glibc-2.28 sysroot (needs glibc 2.17), run under qemu-user with
      llvmpipe GLES2; Windows under Wine; macOS CI workflow written (not run),
      `cargo check` from Linux.
      Android + PortMaster game builds (2026-10-18): `baba_platform::android`
      (EGL/GLES2, lifecycle incl. GL context loss, touch, gamepads with
      hot-plug, keys, Back, AAudio on the shared mixer, immersive),
      on-screen touch controls (`baba_platform::touch` + `Renderer::draw_ui`,
      `baba_render::ui` icons + built-in font), `crates/android`
      (`libbabaport.so`; Java-free game-data import from a zip opened with /
      shared to the app; at first an interim level player).
      `baba-port.apk` 1.71 MB, played on the tablet by touch, virtual pad and
      keys (screens in `scratch/android2/`). PortMaster: `babaport.zip`
      514 KB, glibc 2.17, runs under qemu at 640x480. Details:
      `docs/platform-android.md`, `docs/platform-portmaster.md`.
      The real game everywhere (2026-10-18): `android_main` runs
      `baba_game::App` on the Android platform (import kept as a pre-App
      screen; saves in the app's `files/save`; touch controls only while
      playing, menus tapped, Back = cancel/pause, pads incl. Select = back;
      GL context loss handled by the `App`); `baba-port.apk` 1.95 MB
      (+195 KB: the game flow itself). On the tablet: first launch → `0level`
      solved by taps → map; title → slot → pack → map → `0level` by taps →
      map; Back/pause/resume/exit; uinput pad session; context loss
      (`scratch/android3/`, all removed afterwards). PortMaster:
      `babaport.zip` 962 KB, glibc 2.17, `--fullscreen`, no-data screen,
      pad flow title → map → levels under qemu at 640x480 and 720x720 on the
      SDL window (`--drive … --window`), list menus scaled up on 4:3/1:1.
      Windows: `baba-port.exe` 1.98 MB; the three M4 flow scripts pass
      under Wine on the SDL window (27 shots, same comparison numbers).
      macOS (2026-10-18, with M11): the game itself (not the probe) is
      built in CI, `scripts/package/macos.sh` (universal binary,
      SDL2.framework linked through a dylib symlink, no Cargo changes,
      ad-hoc signed `.app` zip, smoke test of both slices); written, not
      run yet (`docs/platform-macos.md`). The old `macos.yml` is folded
      into `.github/workflows/build.yml`.
      Closed 2026-10-19: macOS universal build green in CI (smoke test of
      both slices); on-screen keyboard for the text prompts outside the
      editor (slot name, level code, online room code; `crate::osk`,
      `app/prompt.rs`): keys are menu items, so d-pad/A, taps and the
      mouse work; Undo deletes (cancels when empty), Pause submits,
      Restart is shift, Select/Escape cancels; a physical keyboard types
      straight in (text-entry mode in `Input`: Space and WASD type instead
      of confirming/moving, Enter submits). Title on 4:3/1:1: the version
      line moved clear of the Multiplayer/Online buttons (band under the
      canvas on 1:1, top-left on 4:3). Test `crates/game/tests/m9/3-osk.drive`
      (pad, taps with shift, keyboard, 640x480 and 720x720 shots; run by
      `scripts/dev/m9-test.sh`). Private APK 0.1.0+p3 (versionCode 10003)
      published; not run on the tablet in this session (no device in the
      worker's scope). Left for M12: the `gameboy` check.
      Launcher icon with Baba (2026-10-18): made at build time from the
      local install for private builds only (`scripts/android/private.sh`,
      `--game-icon`; releases keep the default icon); private builds also
      report panics/failed starts (`crash.rs`). versionCode scheme now
      ×100 (+ private counter). Not seen on a device yet (no device in this
      worker's scope); aapt2 reads the resources. `docs/platform-android.md`.
- [x] M11 Packaging, CI, README and player docs, no-game-data check.
      Public release only when the owner says so.
      Done 2026-10-18 (all local, nothing published or pushed):
      `scripts/package.sh [--verify] [--only …] [--bundle-sdl]
      [--checks-only]` → `dist/baba-port-<v>-{linux-x86_64.tar.gz,
      windows-x86_64.zip, android-arm64.apk, portmaster.zip,
      macos-universal.zip (CI)}` + `SHA256SUMS`; per platform
      `scripts/package/*.sh`, deterministic archives, version from
      `[workspace.package]` (`baba-port --version`, + `BABA_BUILD_ID` from
      CI), licences per target (`packaging/gen-licenses.py` over
      Cargo.lock + `cargo tree`), names/exec paths checked against
      `packaging/warpzone/baba-port.toml` (check-games ok).
      Sizes 0.1.0: Linux 2.24 MB (system SDL2 locally; CI bundles a
      self-built SDL2 2.32.10, `scripts/ci/build-sdl2-linux.sh`), Windows
      2.86 MB (GUI subsystem now, no console window), APK 2.42 MB
      (5.2 MB `.so`, licences inside), PortMaster 2.01 MB.
      `scripts/package/verify.sh` (unpacked into `scratch/m11/verify/`):
      Linux: `--version`, libraries, glibc 2.34, no-data screen, the
      Steam copy found under a fake HOME, headless `0level`, the three M4
      flow scripts (27 screenshots, 0 failures); Windows under Wine: imports,
      `--version`, no-data screen, the flow scripts on the SDL window
      (0 failures); PortMaster under qemu: layout, port.json, glibc 2.17,
      no-data screen, pad flow `packaging/portmaster/smoke.drive` at
      640x480 and 720x720 (0 failures); Android: v2 signature, manifest
      (versionCode 100), contents; the packaged APK once on the tablet:
      no-data screen → zip import (111.9 MB, 1.8 s, 245 sound names) →
      `0level` → two moves by touch, then uninstalled and cleaned
      (`scratch/m11/tablet/`).
      Desktop game data: `crates/game/src/gamedir.rs` (`--game-dir`,
      `BABA_GAME_DIR`, `WARPZONE_DATA_BABA`, `gamedata/` next to the
      binary, `<data dir>/game`, Steam libraries incl.
      `libraryfolders.vdf`, GOG, macOS `.app`); no-data screen names
      `<data dir>/game`. Fix on the way: `drive.rs` gated the virtual pads
      on `sdl` only while `lib.rs` gates them on Linux + `sdl` (Windows
      build broke).
      `scripts/check-no-game-data.sh` (+ `check_no_game_data.py`): types,
      magic (PNG/Ogg/fonts/`ACHTUNG!`/Lua bytecode), Lua source and Lua
      table heuristics, size allowlist, private paths, and against the
      install: SHA-256 of all 6 379 files + 36 k 32-byte samples (dense
      in Lua/language files); repository and all archives pass, control
      files (a sprite chunk inside a binary, a `.ld` copy, a `values.lua`
      extract) are caught.
      CI (`.github/`, actions pinned to commit SHAs, toolchain 1.99.1 in
      `tools/` via `actions/setup`): `ci.yml` (no-game-data, fmt, clippy
      incl. Android, tests with Mesa EGL, `build.yml`), `build.yml` (five
      platform jobs + an `archives` job running the checks), `release.yml`
      (tag = Cargo version, release-key APK from secrets, **draft**
      release only). Locally green: fmt, workspace clippy, every archive.
      Warpzone re-pinned to `c967f5c` (on GitHub, the same commit as the
      other ports; baba-net tests and clippy pass against it); 2026-10-19
      to `85f8376` (0.1.6: Android launcher, `--warpzone-code`; the
      descriptor lists the APK and `org.babaport`, the Android entry reads
      the intent extra `warpzone.args`, docs/platform-android.md);
      2026-10-19 to `b7640d8` (0.1.9: the Android entry adopts the
      launcher's player via `warpzone.identity`; `WARPZONE_HOME` = the
      app's files dir on Android).
      Published 2026-10-18: github.com/jarvis-jhk/baba-port (public, fresh
      history), specs in the private `jarvis-jhk/baba-port-spec`
      (`docs/spec/`), Android release key in the repository secrets,
      prerelease v0.1.0 from `release.yml`, Warpzone descriptor
      `games/baba-port.toml` in jarvis-jhk/warpzone.
      Docs: `README.md`, `packaging/README-player.md` (shipped as README),
      `THIRD_PARTY_NOTICES.md`, `CONTRIBUTING.md`, `SECURITY.md`; Android
      release signing in `docs/platform-android.md` ("Release signing",
      `scripts/android/release-key.sh`; `-debugkey.apk` without it).
      Open for the owner: notarisation (paid Apple account) or ad-hoc;
      PortMaster listing screenshot (must not show game art).
      Windows/Mac game data (2026-10-18, Steam depots in
      `tools/baba-{windows,mac}`): every build ships `Assets.dat`; same
      container, images and sounds byte-identical, only the 120 shaders
      differ (Windows: D3D9 `vs_3_0`/`ps_3_0` bytecode instead of GLSL;
      both carry a length prefix on the fragment part too, which the
      reader used to leak into the GLSL text). `ShaderLanguage`,
      `AssetsDat::shader_language`. Sound names from any executable:
      `baba_formats::exe` (ELF, PE32/PE32+, Mach-O thin/universal; same
      245 names from all three); `GameInstall::locate` (`.app`,
      `Contents/`, folder holding a bundle or the game one level down) and
      `GameInstall::executables`; `gamedir::resolve` delegates, `--game-dir`
      is resolved too. Android import keeps a root `*.exe` or a bundle's
      `Contents/MacOS/Chowdren` (`crates/android/src/zip_paths.rs`, `\`
      separators, `__MACOSX/` ignored); PortMaster launcher hint accepts
      a copied folder or `.app`. Player/PortMaster READMEs: caveat removed.
      Checked: the M4 flow scripts headless with `BABA_GAME_DIR` at each
      copy (33 screenshots byte-identical across the three, 0 failures,
      identical audio request logs, 245 names each;
      `scratch/windata/flow/`); real-install tests of formats and sim pass
      on each copy. Tests that run when the copies exist:
      `crates/formats/tests/versions.rs`, `crates/game/tests/game_versions.rs`
      (same sound index and decoded samples per name),
      `crates/android/tests/import_layouts.rs` (simulated zips of each
      build → working install); no-game-data check passes against all
      three. Not done: the import on the tablet with a real Windows/Mac
      zip; arm64 Mach-O with chained fixups (future Mac builds) is not
      handled.
      Closed 2026-10-19: every CI run packages and verifies all five
      archives (`package.sh --only <p> --verify` per platform job: Linux,
      Windows under Wine, Android, PortMaster under qemu; macOS smoke test
      and `codesign --verify`; `--checks-only` over all archives with the
      no-game-data check and the descriptor), green on 560cd34 and later.
      v0.1.0 and v0.1.1 (2026-10-19, the Android Warpzone home fix) are prereleases; a full release waits for the owner.
- [ ] M12 Final check on the `gameboy` handheld (only after everything else).

## Workstreams in flight

See `scratch/agents.txt`.

## Decisions (owner, 2026-10-18)

- Rust from specs; look for TASes to test with.
- Extras: level editor, online level codes, import original save, custom
  world packs.
- The original may run headless; Steam mode allowed when needed.
- Testing: the Android tablet over `ssh tablet`; emulate the rest; `gameboy`
  only at the very end, not turned on before.

## Open questions

- (owner, decided 2026-10-18: a warpzone-core feature, done in the warpzone
  project; interim stays until then) Android/PortMaster builds have no C compiler; warpzone-core pulls in
  `ring` (C) via ureq's `rustls` feature. Clean fix: a warpzone-core Cargo
  feature that leaves ring out (a change in the warpzone project, which this
  project may not edit). **Interim fix in place (2026-10-18):**
  `[patch.crates-io]` → `crates/vendor/ureq` (3.4.2, `rustls` without
  `_ring`); baba-net re-enables ring on desktop targets and installs
  rustls-rustcrypto on Android / aarch64 Linux (`baba_net::tls`). HTTPS to
  the public Warpzone server checked from the tablet, under qemu (aarch64)
  and on desktop (ring). Remove the patch once warpzone-core has the
  feature.
