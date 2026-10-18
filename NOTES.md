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
