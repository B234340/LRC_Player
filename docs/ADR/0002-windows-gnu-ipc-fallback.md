# ADR 0002: Windows `x86_64-pc-windows-gnu` Build and IPC Fallback

## Status

Accepted

## Context

The project supports a Windows GUI and needs a cross-build path from Linux.

`x86_64-pc-windows-msvc` requires Microsoft Visual C++ tools (`cl.exe`, `lib.exe`, `ml64.exe`) and is difficult to use from WSL without a Windows-hosted toolchain.

## Decision

The project will use `x86_64-pc-windows-gnu` for Linux-based cross-compilation.

For Windows GUI execution:

- `target/x86_64-pc-windows-gnu/release/gui.exe` is the GUI application.
- `target/x86_64-pc-windows-gnu/release/lrc_player.exe` is the CLI/daemon binary.

To avoid startup failure on Windows when the Unix-socket IPC implementation is unavailable:

- `shared/ipc.rs` uses a Windows fallback that returns `Ok(true)` from `try_bind_socket()`.
- `start_server()` on Windows is a no-op.

## Consequences

- Windows GUI users can run the app without MSVC toolchain support.
- Single-instance locking is not enforced on Windows until a proper Windows IPC mechanism is implemented.
- The project can continue to support Windows builds in CI and cross-platform workflows.
