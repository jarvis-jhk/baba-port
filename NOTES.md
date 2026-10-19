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
