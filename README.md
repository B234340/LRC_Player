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
- `src/core/models.rs`: shared domain entities used by UI and services.
- `src/core/state.rs`: application state and command handling between UI and core.
- `src/platform/`: media keys, dialogs, drag and drop, notifications.

## Portable policy

- Default mode is always portable.
- All mutable data stays under `./data` relative to the executable directory.
- No writes to `%AppData%`, `~/.config`, or `~/Library/Application Support`.
- The app should tolerate the whole directory being moved between paths.

## MVP scope

- Add library folders manually.
- Read tags and find sidecar `.lrc` files.
- Control playback, queue, seek, repeat, and shuffle.
- Persist config and playlists locally.

## Build

```bash
source "$HOME/.cargo/env"
cargo run
cargo build --release
```

Release binary paths:

- Linux: `target/release/lrc_player`
- macOS: `target/release/lrc_player`
- Windows: `target/release/lrc_player.exe`

## Release strategy

To create portable releases for each OS, package the built binary together with a local `data/` directory.
This repository includes a GitHub Actions workflow that builds release artifacts for:

- Linux x86_64
- Windows x86_64
- macOS x86_64
- macOS arm64

The workflow runs on pushes, pull requests, and manual dispatch. When you push a tag such as `v0.1.0`, it also publishes a GitHub Release and attaches packaged artifacts.

### Local release build

Build the current host platform locally:

```bash
source "$HOME/.cargo/env"
cargo build --release
```

If you want to package it manually, create a folder like this and archive it:

```text
LRC_Player/
  lrc_player(.exe)
  README.md
  data/
    cache/
    logs/
```

### CI release build

The workflow file is located at `.github/workflows/release.yml`.

Typical flow:

1. Commit your changes.
2. Push a version tag such as `v0.1.0`.
3. GitHub Actions builds each target.
4. Packaged archives are uploaded to the workflow run.
5. On tag builds, the same archives are attached to a GitHub Release.

### Notes on cross-platform builds

- Building Linux, macOS, and Windows artifacts from one local machine is possible, but native CI runners are simpler and more reliable for GUI apps.
- macOS usually needs separate Intel and Apple Silicon builds unless you intentionally produce a universal app bundle later.
- This project is currently packaged as a portable directory archive, not as an installer or `.app` bundle.

## Notes

The current scaffold intentionally stops at startup, config persistence, and UI shell.
Audio decoding and output should be added behind a dedicated player interface rather than wired directly into the UI.

## Current scaffold direction

The repository now includes the first-pass core skeleton for the next stage:

- `models.rs` holds portable domain types such as tracks, playlists, queue mode, and playback mode.
- `library.rs` provides an in-memory library index API that can later be backed by JSON or SQLite.
- `lyrics.rs` contains a minimal `.lrc` parser and a lookup method for the active line at a playback position.
- `player.rs` defines a playback trait and a `NullPlayer` adapter so UI work can proceed before a real backend exists.
- `state.rs` acts as the application layer, receiving commands from the UI and coordinating config, library, and player state.

This keeps `app.rs` focused on rendering and user interaction while leaving room to add a real scanner and playback backend without rewriting the UI layer.
