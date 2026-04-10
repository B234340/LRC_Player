#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

LINUX_TARGET="x86_64-unknown-linux-gnu"
WINDOWS_TARGET="x86_64-pc-windows-gnu"
MACOS_X86_TARGET="x86_64-apple-darwin"
MACOS_ARM_TARGET="aarch64-apple-darwin"

ensure_command() {
  local command_name="$1"
  local install_hint="$2"

  if command -v "$command_name" >/dev/null 2>&1; then
    return 0
  fi

  echo "Missing required command: $command_name" >&2
  echo "$install_hint" >&2
  return 1
}

ensure_rust_target() {
  local target="$1"
  if rustup target list --installed | grep -Fxq "$target"; then
    return 0
  fi

  echo "==> Installing Rust target $target"
  rustup target add "$target"
}

echo "==> Checking Rust toolchain"
ensure_command cargo "Install Rust from https://rustup.rs/."
ensure_command rustup "Install Rust from https://rustup.rs/."

echo "==> Ensuring Linux and Windows Rust targets"
ensure_rust_target "$LINUX_TARGET"
ensure_rust_target "$WINDOWS_TARGET"

echo "==> Checking Windows GNU cross linker"
if ensure_command x86_64-w64-mingw32-gcc "On Ubuntu/Debian: sudo apt-get update && sudo apt-get install -y gcc-mingw-w64-x86-64 binutils-mingw-w64-x86-64"; then
  echo "Windows cross-linker is available."
fi

echo "==> Checking macOS cross-build prerequisites"
ensure_rust_target "$MACOS_X86_TARGET"
ensure_rust_target "$MACOS_ARM_TARGET"

if command -v o64-clang >/dev/null 2>&1 && command -v oa64-clang >/dev/null 2>&1; then
  echo "osxcross toolchain detected. macOS cross-builds can run locally."
else
  cat <<'EOF'
macOS cross-linkers were not found.

Local macOS builds on Linux require osxcross plus a locally provided Apple SDK.
Expected commands:
  - o64-clang
  - oa64-clang

Typical environment setup:
  1. Build or install osxcross in a local directory.
  2. Provide a valid macOS SDK tarball locally.
  3. Export PATH to include osxcross/target/bin.

The GitHub Actions workflow added to this repository builds both macOS targets on GitHub-hosted macOS runners, so release builds remain covered even if local macOS cross-linkers are unavailable here.
EOF
fi

cat <<EOF

Local cross-build setup check completed.
Repository root: $SCRIPT_DIR

Suggested verification:
  TARGETS="$WINDOWS_TARGET" ./build_release_binaries.sh
  TARGETS="$MACOS_X86_TARGET $MACOS_ARM_TARGET" ./build_release_binaries.sh
EOF