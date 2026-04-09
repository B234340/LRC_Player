#!/usr/bin/env bash

set -euo pipefail

repo_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_dir"

windows_target="x86_64-pc-windows-gnu"
linux_dist_dir="dist/linux-x86_64"
windows_dist_dir="dist/windows-x86_64"

if ! command -v cargo >/dev/null 2>&1; then
    echo 'cargo not found. Run: source "$HOME/.cargo/env"' >&2
    exit 1
fi

if ! command -v rustup >/dev/null 2>&1; then
    echo "rustup not found. Install Rust with rustup before building release artifacts." >&2
    exit 1
fi

if ! rustup target list --installed | grep -qx "$windows_target"; then
    echo "Missing Rust target: $windows_target" >&2
    echo "Run: rustup target add $windows_target" >&2
    exit 1
fi

if ! command -v x86_64-w64-mingw32-gcc >/dev/null 2>&1; then
    echo "Missing MinGW linker: x86_64-w64-mingw32-gcc" >&2
    echo "On Ubuntu/WSL run: sudo apt update && sudo apt install -y mingw-w64" >&2
    exit 1
fi

cargo build --release
cargo build --release --target "$windows_target"

rm -rf "$linux_dist_dir" "$windows_dist_dir"
mkdir -p "$linux_dist_dir/data/cache" "$linux_dist_dir/data/logs"
mkdir -p "$windows_dist_dir/data/cache" "$windows_dist_dir/data/logs"

cp target/release/lrc_player "$linux_dist_dir/"
cp target/"$windows_target"/release/lrc_player.exe "$windows_dist_dir/"
cp README.md "$linux_dist_dir/"
cp README.md "$windows_dist_dir/"

echo "Built artifacts:"
echo "  Linux:   $linux_dist_dir/lrc_player"
echo "  Windows: $windows_dist_dir/lrc_player.exe"