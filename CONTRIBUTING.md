# Contributing

Issues and pull requests are welcome: bug reports (with the level and what
the original does), platform reports, translations of our own texts,
documentation.

* **No game data, ever.** Do not add sprites, sounds, music, levels, Lua code,
  extracted tables or screenshots of the game to the repository, issues or
  pull requests; baba-port reads everything from the player's own copy.
  `scripts/check-no-game-data.sh` runs in CI.
* **Clean room.** Behaviour is implemented from written descriptions, not by
  translating the original's code. Please do not paste or quote the
  original's Lua code; describe what it does instead.
* **Code**: `source env.sh`, then before a pull request `cargo fmt --all`,
  `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace`. The rules engine (`crates/sim`) stays
  deterministic: integers only, no `HashMap` iteration, no clocks, no
  threads.
* **Security issues**: see [SECURITY.md](SECURITY.md), not a public issue.

By contributing you agree that your contribution is licensed under MIT OR
Apache-2.0, like the rest of the project.
