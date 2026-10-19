# Android

Decision (2026-10-18): **NativeActivity, no Java at all**, arm64-v8a only,
built without Gradle, the JDK, the SDK build-tools or the NDK's clang. Proven
on the test tablet (SM-T720, Android 15, Adreno 615) with the probe in
`probes/android`.

## The game APK (2026-10-18)

`scripts/android/build.sh` builds `crates/android` (`baba-android`, cdylib
`libbabaport.so`, profile `dist`: opt-level s, fat LTO, stripped) into
`target/dist/android/baba-port.apk`, package `org.babaport`:

| | Size |
|---|---|
| `baba-port.apk` (arm64-v8a only, `.so` compressed) | **1.95 MB** (1,950,205 B) |
| `libbabaport.so` | 4.25 MB |

Growth over the interim player build (1.71 MB / 3.85 MB, below): +195 KB
APK, +0.4 MB `.so`, all of it the game's own code that now runs here
(`baba_game::App`: title, menus, settings and controls, saves and
progression in `baba-progress`, map flow, audio rules, credits, and the
pure-Rust TrueType rasteriser `ab_glyph` for the game's fonts). No new
native libraries, no Java, no toolchain growth.

It carries the game, sim, formats, renderer, platform backend and the Warpzone
client with its TLS stack (rustls + rustls-rustcrypto + webpki roots), so
online play needs nothing more. Manifest additions over the probe:
`android.permission.INTERNET` (normal permission, granted at install) and
VIEW/SEND intent filters for `application/zip` (the data import below).

**The game on the tablet (2026-10-18, `scratch/android3/`):**
`android_main` runs the real `baba_game::App` on `AndroidPlatform` (no
SDL; `App::with_renderer` takes the renderer that drew the import/no-data
screens). Saves and settings live in the app's private `files/save`.
Checked: no-data screen (`01`) → zip import through VIEW (112 MB in 1.7 s,
now also `bin64/Chowdren` when the zip has it: the sound effects' names
come from it, 245 named) → first launch straight into `0level` (`02`),
solved by 8 `input tap`s on the d-pad → return animation, back on the map
(`04`) → relaunch: title (`05`), menus tapped (Start → slot → pack, `06`,
`07`) → map (`08`) → Wait button enters `0level` (`09`), solved by taps →
map (`10`) → Back = pause (`11`, touch controls hidden while a menu is
open) → tapped Resume → gamepad session with the uinput pad (`padsim`:
Start = pause, B/Select = back, d-pad walks to `1level`, A enters it,
stick moves, Start → Return to map; `12`-`14`) → home/resume with
`debug.babaport.losectx 1` (the `App` re-creates its renderer, `15`) →
Exit tapped → `android_main` returns. Final pass with the menu scaling
(`16`-`19`: list menus at 3.33x on the 16:10 screen, taps still land).
Everything removed afterwards (`deploy.sh clean`).

Input on Android: the touch controls (d-pad, Wait, Undo, Restart, Menu)
press the game's default keys and are shown only while playing with no
menu open (`App::wants_touch_controls`; disabling them releases held
controls); menus are tapped (`App` maps `Event::Touch` down / left mouse
clicks onto the topmost menu's items, sliders step by the half tapped;
title: tap = confirm, credits: tap = back); Back is `Cmd::Back` (pause in
play, back in menus; the title menu has no back, Exit quits); pads use the
game's pad bindings, Select acts as back unless bound. Not on Android yet:
renaming a slot needs a keyboard (no soft keyboard).

Earlier interim-player build, screenshots in `scratch/android2/`: no-data
screen (built-in font) → import of a 90 MB zip of the game through a VIEW
intent (103 MB unpacked in 1.7 s) → `baba/0level` at 2560x1600, ~60 fps, with the touch overlay; turns by `input tap` on the
d-pad and undo (state hashes match: undo returns to the previous turn's
hash); turns by the virtual uinput pad (`padsim right right right a b
stick-right`: hot-plug add on first event, removal by the device scan,
touch overlay hidden); keyboard arrows (`input keyevent`) win the level and
load `1level`; home → resume with `debug.babaport.losectx 1` (GL context
dropped: new renderer, atlas re-uploaded, correct frame); HTTPS to the
public Warpzone server in 190 ms with the pure-Rust provider; Back exits
cleanly. AAudio opens 44.1 kHz stereo f32 (burst 882 frames) and is paused /
restarted with the activity. Everything was removed afterwards
(`deploy.sh clean`: uninstall, `/data/local/tmp/baba-port`, switches).

Scripts: `scripts/android/deploy.sh install start import ZIP shot log tap key
pad setprop clean` (`APP=probe` for the probe; `PAD_ARGS` scripts `padsim`,
e.g. `"1500ms right a b"`).

### Backend (`baba_platform::android`, feature `android`)

`AndroidPlatform::new(app, &WindowConfig)` waits for the first window,
creates EGL + a GLES 2 context and returns `(platform, glow::Context)` like
the SDL backend. Details in the module docs: surface loss keeps the context
(`Suspend`, then `poll_events` blocks until the window is back →
`Resume { gl_lost: false }`); a lost context (`EGL_CONTEXT_LOST`, or the test
switch) gives `Resume { gl_lost: true }` + `take_restored_gl`; pause/resume
stop/start the AAudio stream; a disconnected stream (headphones, routing) is
re-opened on the same mixer; gamepads are controllers keyed by Android device
id (names/descriptors through JNI `InputDevice`, a 2 s
`InputDevice.getDeviceIds()` scan for hot-plug), hat axes become d-pad
buttons; keyboards are keys; Back is `Key::Back` (not passed to the system:
the game decides; returning from `android_main` finishes the activity);
volume/power/home stay with the system. Raw JNI helpers
(`baba_platform::android::jni`, typed local refs inside a local frame) cover
system classes only: no Java in the APK.

### Game data (no Java)

The player supplies their own copy; nothing from the game is in the APK.

1. **Open/share a zip (main path).** On a computer, zip the game's folder
   (Steam/GOG, any build: a Windows or Linux install folder, or the Mac
   `Baba Is You.app`), copy it to the device, tap it in the Files app →
   "baba-port" (VIEW) or share it to baba-port (SEND). The activity reads
   `getIntent()`, opens the content URI with
   `ContentResolver.openFileDescriptor` (all through JNI), and Rust
   unpacks `Data/**`, `Assets.dat`, `gamecontrollerdb.txt` and the game's
   executable (only for the sound names: `bin64/Chowdren`, a `*.exe` next
   to `Data/`, or a bundle's `Contents/MacOS/Chowdren` → `MacOS/Chowdren`)
   into the app's private `files/game`, showing progress. The folder is
   found at any depth by `Data/values.lua` (`__MACOSX/` ignored, `\`
   separators accepted); zip-slip safe; pipes are spooled to a temp file
   first. Mapping in `crates/android/src/zip_paths.rs` (host unit tests).
   The intent is then marked consumed. Removed on uninstall.
2. **Copy a folder** to `Android/data/org.babaport/files/game` over USB (MTP)
   or `adb push` (the app's external files dir, no permission needed).

Why not the Storage Access Framework picker (`ACTION_OPEN_DOCUMENT_TREE`):
its result arrives in `Activity.onActivityResult`, which `NativeActivity`
does not forward to native code, so it would need a Java subclass: one small
class (~1-2 KB of dex), but building dex needs `d8` (JDK + build-tools,
~700 MB of toolchain), against the no-Java rule. The VIEW/SEND intent filter
reaches the same files (any provider, cloud drives included) without that.
Limitation: `singleTask` + `NativeActivity` does not see a new intent while
the app is already running (no `onNewIntent` forwarding): the player closes
baba-port first. The Play Store Android version of the game keeps its data
private, so it cannot be used.

### Warpzone launcher

The Android launcher (Warpzone >= 0.1.6) installs the release APK (listed
in `SHA256SUMS`, same signing key every release) and starts `org.babaport`
with the string-array extra `warpzone.args`: the desktop arguments
(`--warpzone-join <room|invite>`, `--warpzone-host`, `--warpzone-code
<CODE>`, join + code for a listed room that wants its code), plus
`warpzone.launched` and `warpzone.server`. `data::warpzone_launch` reads
them through JNI at start (server and "launched" become `WARPZONE_SERVER` /
`WARPZONE_LAUNCHED`, as on desktop), removes `warpzone.args` from the
intent, and the args go to `App::launch_online` (join by room, code or
both). The launcher has no account and never joined the room first, so a
private room comes with its code or invite link. Same limitation as the zip
import: a game that is already running does not see the new intent
(`singleTask`, no `onNewIntent` without Java); it acts on the arguments
only when started fresh.

### Touch controls

`baba_platform::touch::TouchControls` (pure logic, unit-tested) lays out a
d-pad (bottom left, direction by angle, slide to change) and Wait / Undo /
Restart / Menu (bottom/top right) from the drawable size, and turns touches
on them into ordinary key events (arrows, Space, Z, R, Escape, with key
repeat), so the game has no touch-specific input code; other touches pass
through as `Event::Touch`. Any controller button/stick or physical key hides
the controls; the next touch shows them. The Android backend runs them inside
`poll_events`; the game draws `platform.touch_controls().items()` with
`Renderer::draw_ui` (`baba_render::ui`: procedural icons + a 5x7 built-in
font for messages that must work before any game data exists).
`baba_android::screen::touch_quads` draws them over the `App`'s frames.

### TLS without ring

warpzone-core uses ureq with its `rustls` feature, which pulls in `ring`
(C code; no C compiler in these builds). Interim fix (until warpzone-core
gets a feature for it): `[patch.crates-io]` → `crates/vendor/ureq` (ureq
3.4.2, MIT OR Apache-2.0, licences kept; only change: `rustls` no longer
implies `_ring`). baba-net turns `ureq/_ring` back on for desktop targets
(target-specific dependency, so the feature never reaches Android or aarch64
Linux) and, on Android and aarch64 Linux, depends on `rustls-rustcrypto`;
`baba_net::tls::install_crypto_provider()` installs it as the process
default (called by `NetSession::new` and at app start). `cargo tree
--target aarch64-linux-android` has no `ring` and no `cc`. graviola was the
faster alternative but asserts the ARMv8 AES/PMULL/SHA2 extensions at run
time (panics without them, e.g. on some PortMaster SoCs); rustls-rustcrypto
is portable. Notes: it is an alpha crate (0.0.2-alpha) and uses `rsa` 0.9
(RUSTSEC-2023-0071, a timing issue in RSA *decryption/signing*; we only
verify server signatures). Checked: tablet (190 ms), aarch64 under qemu
(`cargo run --example tls_check` cross-built, 402 ms), desktop with ring.

### Entry point

`crates/android` is now small: `lib.rs` (`android_main`: logging, TLS
provider, import, then the `App` loop: `app.frame(&mut platform)`, touch
controls drawn with `app.renderer().draw_ui`, swap), `data.rs` (import),
`screen.rs` (import progress / no-data screens in the built-in font, touch
control quads). The game crate is used with `default-features = false`
(no `sdl2-sys`). The interim level player (`play.rs`) is gone.

## Size budget (probe)

| | Size |
|---|---|
| Probe APK (`.so` compressed, `extractNativeLibs=true`, default) | **270 KB** (275,984 B) |
| Probe APK with `--stored-libs` (uncompressed, 16 KiB aligned `.so`) | 576 KB |
| `libbabaprobe.so` (release, `opt-level=z`, LTO, stripped) | 544 KB |
| APK overhead (binary manifest 1 KB + v2 signing block 0.7 KB + zip) | ~2 KB |
| **Toolchain, Android-specific** | |
| `tools/android/ndk-min` (pruned NDK r30: API-26 stubs, crt objects, libunwind, builtins) | **1.6 MB** (of a 739 MB zip, deleted after unpacking) |
| Rust `rust-std` for `aarch64-linux-android` (rustup) | 145 MB |
| `rust-lld` | 0 (ships with the host toolchain) |
| Packaging + signing | 0 (`scripts/android/apk.py`: Python stdlib + system `openssl`) |

For comparison, the Java route of `~/Projects/House/handstand` needs a JDK
(318 MB) plus SDK build-tools and a platform (436 MB); the full NDK unpacks
to ~2 GB. The Rust std component is now the only large piece; it cannot be
pruned safely (a nightly `-Z build-std` build would drop it and also shrink
the `.so`, see "Further savings").

## Pipeline

```
scripts/android/fetch-toolchain.sh   # once: rustup target + NDK pieces -> tools/android/ndk-min
scripts/android/build.sh [--probe]   # cargo -> rust-lld -> apk.py -> target/dist/android/baba-port.apk (baba-probe.apk)
scripts/android/deploy.sh install start shot log   # tablet (APP=probe for the probe)
```

- **Compile/link**: `cargo build --target aarch64-linux-android` with
  `-C linker-flavor=ld.lld` and `scripts/android/ld.sh` as the linker. The
  wrapper runs `rust-lld` against `tools/android/ndk-min/lib`, adding what
  clang's driver would add: `crtbegin_so.o`/`crtend_so.o` for shared
  libraries (`crtbegin_dynamic.o`/`crtend_android.o` for executables such as
  `padsim`), `-z max-page-size=16384` (16 KiB-page devices), build id, hash
  style. No C code is compiled anywhere in the dependency tree
  (android-activity's NativeActivity backend, ndk, glow are pure Rust), so
  no clang is needed. Any future C dependency (e.g. a C audio decoder) would
  bring clang back and needs a reason here.
- **minSdk 26** (AAudio), **targetSdk 35**. Sysroot stubs are API 26, so the
  `.so` only links against symbols present on Android 8.0+.
- **APK**: `scripts/android/apk.py build` writes `AndroidManifest.xml`
  directly in binary XML (no `aapt2`; attribute IDs checked against
  android-35's `android.jar` with `aapt2 dump resources`, and the result
  checked with `aapt2 dump badging`), zips it with the `.so` (deterministic
  entries; stored entries aligned like `zipalign -p`) and signs it with **APK
  Signature Scheme v2** (enough for minSdk ≥ 24) using an EC P-256 key via
  `openssl`. `apk.py verify` re-checks the signature; it was also
  cross-checked once with `apksigner verify` ("Verifies", v2 true). No
  `resources.arsc`, no `classes.dex` (`hasCode=false`).
- **Manifest**: `android.app.NativeActivity` with
  `android.app.lib_name=babaprobe`, `Theme.Black.NoTitleBar.Fullscreen`,
  `configChanges` covering everything (no activity restarts),
  `sensorLandscape`, `singleTask`, `isGame`, GLES 2.0 required, touchscreen
  and gamepad optional (so TVs/handhelds are not filtered out).
- **Keys**: `tools/android/keys/debug` (throwaway, created on first build,
  git-ignored) for development; releases use the release key below. ECDSA
  signatures are randomised, so signed APKs differ per build; the unsigned
  content is reproducible.

### Release signing (M11)

- **One release key**, EC P-256 + a self-signed certificate (CN=baba-port,
  10000 days), in apk.py's format (`key.pem`, `cert.der`). The owner creates it
  once with `scripts/android/release-key.sh DIR`, with DIR outside git and
  outside `tools/` (disposable), e.g. on encrypted storage, and keeps an
  offline backup (password manager / USB stick). Losing it means every
  player must uninstall (losing their saves and imported data) to update;
  a leaked key lets others ship "updates", so it never goes into the
  repository, issues or logs.
- **Local release build**: `BABA_ANDROID_KEY_DIR=DIR scripts/package.sh`.
  With `BABA_ANDROID_KEY_DIR` set the key must already exist
  (`apk.py --require-key`; never silently created); without it the APK is
  signed with the debug key and named `...-android-arm64-debugkey.apk`, so a
  debug-signed file cannot pass for a release.
- **CI**: repository secrets `ANDROID_RELEASE_KEY_PEM_B64` /
  `ANDROID_RELEASE_CERT_DER_B64` (`base64 -w0` of the two files).
  `release.yml` passes them to the Android job, which writes them into
  `$RUNNER_TEMP`, deletes them after the build, fails a tag build without
  them, and the draft step refuses `-debugkey.apk` files. Pushes and pull
  requests build with the debug key (secrets are not given to pull
  requests from forks).
- The certificate's SHA-256 fingerprint (printed by `release-key.sh` and in
  the CI log) should go into the release notes so players can check the APK
  (`apksigner verify --print-certs`, or any APK inspector).
- versionCode = (MAJOR·10000 + MINOR·100 + PATCH)·100 of the Cargo version
  (`scripts/package/common.sh`; v0.1.0 still shipped as 100), versionName
  = the version. The last two digits count private builds (below). The APK
  carries the licence texts under `licenses/` (deflated, ~35 KB).
- Not chosen: Play App Signing (no Play Store plans), v1/v3 schemes (minSdk
  26 only needs v2; key rotation via v3 can be added later if ever needed).

## Launcher icon and private builds (2026-10-18)

The launcher icon shows Baba, but Baba is game art: it cannot be in git
or in a public release, and a launcher icon cannot come from data the app
imports at run time (Android reads it from the APK's resources). So it is
made at **build time** from a local install, for private builds only:

- `scripts/android/icon.py GAME OUT` (stdlib): Baba's sprite
  (`baba_0_1.png`) tinted with its default-palette colour, scaled up
  (nearest) into the adaptive icon's 66 dp safe circle on a 432 px
  foreground, plus a background layer in the level background colour.
- `apk.py --icon-fg/--icon-bg`: a hand-written `resources.arsc` (package
  0x7f, type `mipmap`: `ic_launcher` = adaptive-icon XML, the two PNGs in
  `nodpi`) and `android:icon`/`roundIcon` on the application. About 2.7 KB
  in the APK. Checked with Google's `aapt2 dump resources/badging/xmltree`.
- `build.sh --game-icon[=DIR]` runs both; `scripts/android/private.sh
  [--publish]` is the owner's build: release key, game icon, fault reports
  (below), versionCode = release code + a counter (01..99, kept in
  `tools/android/private-build-code`), versionName `<version>+pN`;
  `--publish` uploads to the owner's update channel (URL in the
  git-ignored `keys/apkup-url`). `scripts/package/android.sh` (releases, CI)
  passes neither, so public APKs keep the default icon.

## Fault reports (private builds)

`crates/android/src/crash.rs`: with `BABA_REPORT_URL` set at compile time
(`private.sh` reads `keys/report-url`, git-ignored; releases unset it) a
panic is written to `files/crash-report.json` by the panic hook and posted
(fire-and-forget, background thread) at the next start, then removed; a
failed start (window, renderer, game) is posted the same way. The report
has the version, device model, API level and the message only.

## What the probe proves (tablet, 2026-10-18)

`probes/android/src/lib.rs`, logcat tag `babaprobe`:

- Fullscreen GLES 2: EGL 1.5 + glow (GL entry points via `dlsym` on
  `libGLESv2.so`), "OpenGL ES 3.2 … Adreno (TM) 615", 2560x1600 surface,
  ~60 fps; clear colour + textured (checkerboard) quad checked in a
  `screencap` by `scripts/android/check-shot.py`.
- Immersive mode (status and navigation bars hidden) through **raw JNI on the
  Java UI thread** (`AndroidApp::run_on_java_main_thread`, then
  `getWindow().getInsetsController().hide(systemBars())`; API < 30 falls back
  to `setSystemUiVisibility`). This is the pattern for any other Java API we
  need (soft keyboard, vibration, …): no Java classes in the APK.
- AAudio output stream (48 kHz float mono, data callback), beep on start,
  touch and buttons; `frames_written` advances at 48 kHz, no xruns. The
  device grants `PerformanceMode::None` (burst 960 frames = 20 ms), not
  low-latency, for this format.
- Touch: down/move/up with pointer ids and coordinates (`input tap`,
  `input swipe`); a marker quad follows the finger.
- Gamepad: `input gamepad keyevent` and `input joystick motionevent`, and a
  **real input device** created by `padsim` (`/dev/uinput`, root; USB ids of
  an Xbox 360 pad, so Android uses its key layout): buttons A/B/X/Y/Start as
  `KEYCODE_BUTTON_*` (source 0x501), sticks/triggers/hat as joystick motion
  events (`AXIS_X/Y/Z/RZ/HAT_X/HAT_Y/LTRIGGER/RTRIGGER`), device id 7.
- Lifecycle: home → window terminated, resume → new surface on the same EGL
  context, focus → immersive again. Note: audio keeps running while paused;
  the game must pause/resume the stream on `Pause`/`Resume`.
- Both APK variants (compressed and stored/16 KiB-aligned `.so`) install and
  run. `deploy.sh clean` uninstalled the app and removed
  `/data/local/tmp/baba-port/` afterwards.

## Why not SDL2's SDLActivity

| | NativeActivity (chosen) | SDL2 SDLActivity |
|---|---|---|
| Java | none | ~10 SDL Java classes → `classes.dex` (~100 KB) |
| Native size | game only | + `libSDL2.so` (~1.8 MB arm64) |
| Toolchain | rust-std + 1.6 MB | + JDK (or jlinked JRE) + d8/aapt2 build-tools + `android.jar` + full NDK clang to build SDL2 (~2 GB) |
| Code sharing | renderer (glow), sim, formats, net shared; platform layer has an Android backend | same platform code as desktop |

The cost of (a) is a second, small platform backend on Android (window/EGL,
input, AAudio, lifecycle) next to SDL2 on the other platforms; the probe is
~500 lines and already covers all of it. `crates/platform` should therefore
keep window/GL-context/input/audio behind its own types so that an
`android-activity` backend can plug in, and the renderer must only need a
glow context.

## Further savings (not done)

- `-Z build-std=std,panic_abort` + `panic_immediate_abort` (nightly) would
  remove the 145 MB rust-std component from the toolchain and typically
  shrinks the `.so` by a third or more.
- android-activity pulls in `jni` 0.22 (unused by us); LTO drops most of it.

## Device notes

- `ssh tablet` is root; the screen dozes: `input keyevent KEYCODE_WAKEUP;
  wm dismiss-keyguard` before starting, otherwise the activity pauses at once.
- `screencap -d 0 -p` works over ssh on this device.
- Only ~4 GB free: everything goes to `/data/local/tmp/baba-port/` and is
  deleted by the deploy script (APK right after install, screenshots right
  after pulling, `padsim` after running).
