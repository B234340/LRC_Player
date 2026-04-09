# LRC Player

Cross-platform portable audio player scaffold.

## Design goals

- Runs from a directory without an installer.
- Stores settings, cache, playlists, and logs inside the application directory.
- Keeps platform-specific code isolated from playback and UI logic.
- Starts with a local-library MVP and leaves room for a later playback engine.

## Proposed layout

```text
LRC_Player/
  Cargo.toml
  README.md
  src/
    app.rs
    main.rs
    core/
      config.rs
      mod.rs
  data/
    config.json
    library.json
    playlists.json
    cache/
    logs/
```

## Architecture

- `src/main.rs`: process bootstrap and native window startup.
- `src/app.rs`: UI shell, view state, and user interactions.
- `src/core/config.rs`: portable path policy and config persistence.
- `src/core/mod.rs`: shared core exports.

Recommended next modules after this scaffold:

- `src/core/library.rs`: scan folders and persist track metadata.
- `src/core/lyrics.rs`: parse and synchronize `.lrc` files.
- `src/core/player.rs`: playback state machine and audio backend adapter.
- `src/platform/`: media keys, dialogs, drag and drop, notifications.

## Portable policy

- Default mode is always portable.
- All mutable data stays under `./data` relative to the executable working directory.
- No writes to `%AppData%`, `~/.config`, or `~/Library/Application Support`.
- The app should tolerate the whole directory being moved between paths.

## MVP scope

- Add library folders manually.
- Read tags and find sidecar `.lrc` files.
- Control playback, queue, seek, repeat, and shuffle.
- Persist config and playlists locally.

## Build

```bash
cargo run
cargo build --release
```

## Notes

The current scaffold intentionally stops at startup, config persistence, and UI shell.
Audio decoding and output should be added behind a dedicated player interface rather than wired directly into the UI.
