# LRC Player

Cross-platform portable audio player with a portable data layout.

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

- Scan a local music directory and build a playable track list.
- Decode and play audio with `symphonia + cpal`.
- Read tags and find sidecar `.lrc` files.
- Persist config and playlists locally.

## Build

Linux build prerequisites for audio playback:

```bash
sudo apt update
sudo apt install -y pkg-config libasound2-dev
```

```bash
source "$HOME/.cargo/env"
cargo run
cargo build --release
```

Current playback support is directory-based and split into tabs: `Play`, `Album`, `Artist`, `Playlist`, `Library`, `Scan`, and `Settings`.

When available, track title, artist, album, album artist, composer, and cover art are read from embedded file tags instead of only using the filename.

Scanned directory paths and excluded folders are stored in `data/config.json`. On startup, the app automatically rescans saved directories and rebuilds the library list.

Release binary paths:

- Linux: `target/release/lrc_player`
- macOS: `target/release/lrc_player`
- Windows (native build): `target/release/lrc_player.exe`
- Windows (WSL cross-build): `target/x86_64-pc-windows-gnu/release/lrc_player.exe`

## WSL local release build

If WSL rendering is unstable and you want a Windows `.exe` from the same machine, use the bundled build script instead of plain `cargo build --release`.

Prerequisites on WSL:

```bash
source "$HOME/.cargo/env"
rustup target add x86_64-pc-windows-gnu
sudo apt update
sudo apt install -y mingw-w64
```

Then run:

```bash
source "$HOME/.cargo/env"
chmod +x scripts/build_release.sh
./scripts/build_release.sh
```

Generated outputs:

- Linux portable folder: `dist/linux-x86_64/`
- Windows portable folder: `dist/windows-x86_64/`
- Windows executable: `target/x86_64-pc-windows-gnu/release/lrc_player.exe`

The script builds the current Linux release binary and the Windows `.exe`, then assembles portable directories with a local `data/` folder for each target.

Windows note:

- Release builds are linked as GUI applications, so launching `lrc_player.exe` does not keep a console window open.
- Debug builds may still show a console window for logging and debugging.

## Release strategy

To create portable releases for each OS, package the built binary together with a local `data/` directory.
This repository includes a GitHub Actions workflow that builds release artifacts for:

- Linux x86_64
- Windows x86_64
- macOS x86_64
- macOS arm64

Current runner labels in CI:

- Linux x86_64: `ubuntu-latest`
- Windows x86_64: `windows-latest`
- macOS x86_64: `macos-15-intel`
- macOS arm64: `macos-14`

The workflow runs on pushes, pull requests, and manual dispatch. When you push a tag such as `v0.1.0`, it also publishes a GitHub Release and attaches packaged artifacts.

### Local release build

Build the current host platform locally:

```bash
source "$HOME/.cargo/env"
cargo build --release
```

On Windows itself, the release executable is usually:

- `target/release/lrc_player.exe`

To build both the Linux binary and a Windows `.exe` from WSL, use:

```bash
source "$HOME/.cargo/env"
./scripts/build_release.sh
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

The current version includes a real playback backend behind the player interface.
Audio files are decoded with `symphonia` and sent to the output device with `cpal`.
The current UI scans directories recursively for supported audio files, supports include and exclude folders, and uses a native folder picker for convenience.
Selected scan directories are persisted automatically, so the library can be restored on the next launch.
Album, artist, playlist, and library views can all start playback directly from the currently displayed track list.
The UI also loads common system fallback fonts when available so Japanese and other non-Latin text can render correctly without bundling extra font files.

## Current scaffold direction

The repository now includes the first-pass core skeleton for the next stage:

- `models.rs` holds portable domain types such as tracks and playlists.
- `library.rs` provides an in-memory library index API that can later be backed by JSON or SQLite.
- `lyrics.rs` contains a minimal `.lrc` parser and a lookup method for the active line at a playback position.
- `player.rs` defines a playback trait, a `symphonia + cpal` backend, and a `NullPlayer` fallback used only when audio output cannot be initialized.
- `state.rs` acts as the application layer, receiving commands from the UI and coordinating config, library, and player state.

This keeps `app.rs` focused on rendering and user interaction while leaving room to add a real scanner and playback backend without rewriting the UI layer.
