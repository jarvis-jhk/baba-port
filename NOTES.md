# Maintainer notes

Running log of the JARVIS lane (decisions, CI, things waiting on someone).
State of the code: `docs/STATUS.md`; priorities: `ROADMAP.md`.

## 2026-10-18 — handoff to JARVIS

- Took the project over from the laptop sessions (HEAD 5abb0ae). v0.1.0 is
  out as a prerelease with all five platform assets; CI on 51877b5 green.
- 5abb0ae (launcher icon and fault reports for private Android builds) was
  not pushed yet; pushed after fmt, clippy (desktop + Android), the workspace
  tests and the no-game-data check passed in the worker.
- Private Android builds (`scripts/android/private.sh --publish`) go to the
  owner's update channel after every build; the versionCode counter is in
  `tools/android/private-build-code`.
- Published private build 0.1.0+p2 (versionCode 10002, icon + fault
  reports); CI on the pushed HEAD to be watched by the next session.
- Asked the `self` lane for the JARVIS side of the handoff (memory file,
  JARVIS.md entry, schedules).
- Waiting on the owner: nothing yet. To ask in one batch later: tablet test
  list, someone with a Mac, the handheld (M12, last).

## 2026-10-19 — M10 and M11 closed

- No open issues or PRs on GitHub; CI on 560cd34 green on all jobs,
  including the first macOS universal build.
- M10: on-screen keyboard for the slot-name, level-code and room-code
  prompts (pad, touch, keyboard; the test is `tests/m9/3-osk.drive`). Fixed on
  the way: Space confirmed the slot-name prompt instead of typing a space
  (Space is the default confirm key). The title's version line no longer
  overlaps the buttons on 4:3/1:1.
- M11: CI packages and verifies every archive on each run, so it is ticked.
  v0.1.0 stays a prerelease. This lane is releases-only, so the owner was
  not asked; a full release waits for the owner's go.
- Private APK 0.1.0+p3 (versionCode 10003) published. The tablet is not in
  this worker's gateway scope, so it was not run on a device; it is on the
  tablet test list.
- Roadmap items "Platforms finished" and "Packaging and CI" marked done.
- Waiting on the owner: the tablet test list (touch keyboard, launcher icon,
  fault report, Windows/Mac zip import), the go for a full release, M12
  (handheld, last).

## 2026-10-19 — SteamGridDB tile

- Warpzone lane asked (report API) for `steamgriddb = 22797` in the
  descriptor for the 0.1.3 launcher's library tiles; added after `accent` in
  `packaging/warpzone/baba-port.toml`. Servers <= 0.1.2 reject the field, and
  `ext/warpzone` here is older, so `scripts/net/dev-server.sh --check` fails
  on it until ext/warpzone is updated to 0.1.3. The live copy
  (`games/baba-port.toml` in the warpzone repo) is the warpzone lane's: asked
  it to sync once warpzone-server 0.1.3 runs on vtt.

## 2026-10-19 — Warpzone 0.1.6 Android launcher

- Warpzone lane asked (report API): descriptor `android` asset/exec, and
  read the launch intent's `warpzone.args` on Android. Done: ext/warpzone
  (and the CI checkout) re-pinned to 85f8376 (0.1.6); `--warpzone-code`
  joins by code (alone, or with `--warpzone-join` for a listed room) on
  desktop and Android; `warpzone.server` / `warpzone.launched` become the
  environment warpzone-core reads. `check-games` (0.1.6 server) passes, the
  steamgriddb note above is resolved with it. The live descriptor on vtt is
  the warpzone lane's.
- Not tested on a device (tablet not in this worker's gateway scope); on the
  tablet test list: `am start -n org.babaport/android.app.NativeActivity
  --esa warpzone.args --warpzone-code,XXXXXX`. Known limit: an already
  running game ignores the new intent (singleTask, NativeActivity has no
  onNewIntent); the launcher's CLEAR_TOP does not restart it.
- Private APK 0.1.0+p4 (versionCode 10004) published.

## 2026-10-19 — Warpzone 0.1.9 launcher identity

- Warpzone lane asked (report API, t_mvfk1dgegd8l4): adopt the Android
  launcher's `warpzone.identity` extra. Done: ext/warpzone and the CI
  checkout re-pinned to b7640d8 (v0.1.9); `data::warpzone_launch` reads the
  extra (struct no longer `Debug`, so the seed cannot be logged) and
  `android_main` calls `launch::adopt_identity` before any client exists.
  Also found: warpzone-core's home has no Android case (HOME unset → a
  relative `.warpzone`), so the Android entry now sets `WARPZONE_HOME` to
  `<files>/warpzone` unless set. Same limit as `warpzone.args`: only read
  at a fresh start (no onNewIntent in NativeActivity).
- Not tested on a device (tablet outside this worker's gateway scope);
  tablet test: `am start -n org.babaport/android.app.NativeActivity --es
  warpzone.identity <64 hex>` then check `files/warpzone/identity.key`.
- Private APK 0.1.0+p5 (versionCode 10005) published.
