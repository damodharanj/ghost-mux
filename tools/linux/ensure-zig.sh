#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

ZIG_VERSION="${ZIG_VERSION:-0.15.2}"
ZIG_BASE_URL="https://ziglang.org/download/$ZIG_VERSION"
TOOLCHAIN_DIR="$PROJECT_ROOT/.tools/zig/toolchain"

zig_bin_path() {
  local exe=""
  case "$(uname -s)" in
  CYGWIN* | MINGW* | MSYS*)
    exe=".exe"
    ;;
  esac
  printf '%s/zig%s\n' "$TOOLCHAIN_DIR" "$exe"
}

OS="$(uname -s)"
ARCH="$(uname -m)"

is_toolchain_valid() {
  [[ -x "$(zig_bin_path)" ]] || return 1
  [[ -d "$TOOLCHAIN_DIR/lib" ]] || return 1
  if [[ "$OS" == "Darwin" ]]; then
    [[ -f "$TOOLCHAIN_DIR/lib/libc/darwin/libSystem.tbd" ]] || return 1
  fi
  "$(zig_bin_path)" version >/dev/null 2>&1 || return 1
}

if is_toolchain_valid; then
  printf '%s\n' "$(zig_bin_path)"
  exit 0
fi

rm -rf "$TOOLCHAIN_DIR"

case "$OS" in
Darwin)
  case "$ARCH" in
  arm64 | aarch64) PKG="zig-aarch64-macos-$ZIG_VERSION.tar.xz" ;;
  x86_64) PKG="zig-x86_64-macos-$ZIG_VERSION.tar.xz" ;;
  *)
    echo "error: unsupported macOS architecture: $ARCH" >&2
    exit 1
    ;;
  esac
  ;;
Linux)
  case "$ARCH" in
  aarch64 | arm64) PKG="zig-aarch64-linux-$ZIG_VERSION.tar.xz" ;;
  x86_64) PKG="zig-x86_64-linux-$ZIG_VERSION.tar.xz" ;;
  *)
    echo "error: unsupported Linux architecture: $ARCH" >&2
    exit 1
    ;;
  esac
  ;;
CYGWIN* | MINGW* | MSYS*)
  case "$ARCH" in
  x86_64 | AMD64) PKG="zig-x86_64-windows-$ZIG_VERSION.zip" ;;
  aarch64 | arm64) PKG="zig-aarch64-windows-$ZIG_VERSION.zip" ;;
  *)
    echo "error: unsupported Windows architecture: $ARCH" >&2
    exit 1
    ;;
  esac
  ;;
*)
  echo "error: unsupported operating system: $OS" >&2
  exit 1
  ;;
esac

mkdir -p "$PROJECT_ROOT/.tools/zig"
TMP_DIR="$(mktemp -d "$PROJECT_ROOT/.tools/zig/.tmp.XXXXXX")"
ARCHIVE_PATH="$TMP_DIR/$PKG"
URL="$ZIG_BASE_URL/$PKG"

cleanup() {
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT

echo "==> Downloading Zig $ZIG_VERSION for $OS/$ARCH" >&2
curl --fail --location --silent --show-error "$URL" -o "$ARCHIVE_PATH"

echo "==> Installing Zig into $TOOLCHAIN_DIR" >&2
if [[ "$PKG" == *.zip ]]; then
  if command -v unzip >/dev/null 2>&1; then
    unzip -q "$ARCHIVE_PATH" -d "$TMP_DIR/extract"
  else
    powershell -NoProfile -Command "Expand-Archive -LiteralPath '$ARCHIVE_PATH' -DestinationPath '$TMP_DIR/extract'"
  fi
else
  tar -xf "$ARCHIVE_PATH" -C "$TMP_DIR"
fi

EXTRACT_ROOT="$(find "$TMP_DIR" -mindepth 1 -maxdepth 2 -type d -name "zig-*" | head -n 1 || true)"
if [[ -z "$EXTRACT_ROOT" ]]; then
  echo "error: unable to locate extracted Zig toolchain contents" >&2
  exit 1
fi

rm -rf "$TOOLCHAIN_DIR"
mv "$EXTRACT_ROOT" "$TOOLCHAIN_DIR"

if [[ ! -x "$(zig_bin_path)" ]]; then
  echo "error: Zig executable not found at $(zig_bin_path)" >&2
  exit 1
fi

printf '%s\n' "$(zig_bin_path)"
