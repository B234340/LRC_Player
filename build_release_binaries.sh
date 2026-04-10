#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$SCRIPT_DIR"
OUTPUT_DIR="${OUTPUT_DIR:-$REPO_ROOT/releases}"
BUILD_PROFILE="${BUILD_PROFILE:-release}"
PACKAGE_NAME="${PACKAGE_NAME:-$(awk -F '"' '/^name = "/ { print $2; exit }' "$REPO_ROOT/Cargo.toml")}"

DEFAULT_TARGETS=(
  "x86_64-unknown-linux-gnu"
  "x86_64-pc-windows-gnu"
  "x86_64-apple-darwin"
  "aarch64-apple-darwin"
)

if [[ -n "${TARGETS:-}" ]]; then
  read -r -a BUILD_TARGETS <<< "$TARGETS"
else
  BUILD_TARGETS=("${DEFAULT_TARGETS[@]}")
fi

SUCCESS_TARGETS=()
FAILED_TARGETS=()
SKIPPED_TARGETS=()

if ! command -v cargo >/dev/null 2>&1; then
  echo "cargo is required." >&2
  exit 1
fi

if ! command -v rustup >/dev/null 2>&1; then
  echo "rustup is required to install missing targets." >&2
  exit 1
fi

mkdir -p "$OUTPUT_DIR"

has_command() {
  command -v "$1" >/dev/null 2>&1
}

ensure_target_installed() {
  local target="$1"

  if rustup target list --installed | grep -Fxq "$target"; then
    return 0
  fi

  echo "==> Installing target $target"
  if rustup target add "$target"; then
    return 0
  fi

  echo "Failed to install target $target" >&2
  return 1
}

describe_missing_prereq() {
  local target="$1"

  case "$target" in
    x86_64-pc-windows-gnu)
      echo "missing MinGW linker x86_64-w64-mingw32-gcc"
      ;;
    x86_64-apple-darwin)
      echo "missing osxcross linker o64-clang and Apple SDK"
      ;;
    aarch64-apple-darwin)
      echo "missing osxcross linker oa64-clang and Apple SDK"
      ;;
    *)
      echo "missing target prerequisites"
      ;;
  esac
}

can_build_target() {
  local target="$1"

  case "$target" in
    x86_64-unknown-linux-gnu)
      return 0
      ;;
    x86_64-pc-windows-gnu)
      has_command x86_64-w64-mingw32-gcc
      ;;
    x86_64-apple-darwin)
      has_command o64-clang
      ;;
    aarch64-apple-darwin)
      has_command oa64-clang
      ;;
    *)
      return 0
      ;;
  esac
}

binary_extension() {
  local target="$1"
  if [[ "$target" == *"windows"* ]]; then
    printf '.exe'
  fi
}

copy_artifact() {
  local target="$1"
  local extension
  local artifact_path
  local target_dir

  extension="$(binary_extension "$target")"
  artifact_path="$REPO_ROOT/target/$target/$BUILD_PROFILE/$PACKAGE_NAME$extension"
  target_dir="$OUTPUT_DIR/$target"

  if [[ ! -f "$artifact_path" ]]; then
    echo "Built target $target but artifact was not found at $artifact_path" >&2
    return 1
  fi

  mkdir -p "$target_dir"
  cp "$artifact_path" "$target_dir/"
  echo "Copied $(basename "$artifact_path") -> $target_dir"
}

build_target() {
  local target="$1"

  echo "==> Building $PACKAGE_NAME for $target"

  if ! ensure_target_installed "$target"; then
    SKIPPED_TARGETS+=("$target (target installation failed)")
    return 0
  fi

  if ! can_build_target "$target"; then
    SKIPPED_TARGETS+=("$target ($(describe_missing_prereq "$target"))")
    return 0
  fi

  if cargo build --profile "$BUILD_PROFILE" --target "$target"; then
    if copy_artifact "$target"; then
      SUCCESS_TARGETS+=("$target")
    else
      FAILED_TARGETS+=("$target (artifact copy failed)")
    fi
  else
    FAILED_TARGETS+=("$target (build failed; cross-linker or SDK may be missing)")
  fi
}

for target in "${BUILD_TARGETS[@]}"; do
  build_target "$target"
done

echo
echo "Build summary"
echo "  Output directory: $OUTPUT_DIR"

if [[ ${#SUCCESS_TARGETS[@]} -gt 0 ]]; then
  echo "  Success: ${SUCCESS_TARGETS[*]}"
fi

if [[ ${#SKIPPED_TARGETS[@]} -gt 0 ]]; then
  echo "  Skipped: ${SKIPPED_TARGETS[*]}"
fi

if [[ ${#FAILED_TARGETS[@]} -gt 0 ]]; then
  echo "  Failed: ${FAILED_TARGETS[*]}"
  exit 1
fi

echo "  All requested targets completed successfully."