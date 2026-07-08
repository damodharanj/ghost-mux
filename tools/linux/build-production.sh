#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
CARGO_BIN="${CARGO:-$HOME/.cargo/bin/cargo}"
if [[ ! -x "$CARGO_BIN" ]]; then
  CARGO_BIN="$(command -v cargo || true)"
fi
if [[ -z "${CARGO_BIN:-}" ]]; then
  echo "error: cargo not found. Set CARGO or install Rust toolchain." >&2
  exit 1
fi

BIN_NAME="$(awk -F'"' '/^name[[:space:]]*=[[:space:]]*"/ { print $2; exit }' "$PROJECT_ROOT/Cargo.toml" | tr -d '\r')"
if [[ -z "$BIN_NAME" ]]; then
  echo "error: unable to read binary name from Cargo.toml" >&2
  exit 1
fi

OS_NAME="$(uname -s)"
EXEC_EXT=""
case "$OS_NAME" in
CYGWIN* | MINGW* | MSYS*)
  EXEC_EXT=".exe"
  ;;
esac

if grep -Eq '^libghostty-vt-sys[[:space:]]*=.*vendored' "$PROJECT_ROOT/Cargo.toml"; then
  if [[ ! -x "$SCRIPT_DIR/ensure-zig.sh" ]]; then
    echo "error: missing helper script at $SCRIPT_DIR/ensure-zig.sh" >&2
    exit 1
  fi
  ZIG_BIN="$("$SCRIPT_DIR/ensure-zig.sh")"
  export ZIG="$ZIG_BIN"
  export PATH="$(dirname "$ZIG_BIN"):$PATH"
fi

DIST_DIR="$PROJECT_ROOT/dist"
APP_DIR="$DIST_DIR/$BIN_NAME"
LIB_DIR="$APP_DIR/lib"
BIN_PATH="$APP_DIR/$BIN_NAME$EXEC_EXT"

# Always start with a clean bundle directory.
rm -rf "$APP_DIR"
mkdir -p "$LIB_DIR"

"$PROJECT_ROOT/tools/setup-patches.sh"

echo "==> Building release binary"
"$CARGO_BIN" build --release
"$CARGO_BIN" build --release --manifest-path "$PROJECT_ROOT/ghost-mux-server/Cargo.toml" --target-dir "$PROJECT_ROOT/target"

SOURCE_BIN="$PROJECT_ROOT/target/release/$BIN_NAME$EXEC_EXT"
if [[ ! -f "$SOURCE_BIN" ]]; then
  echo "error: release binary not found at $SOURCE_BIN" >&2
  exit 1
fi
cp "$SOURCE_BIN" "$BIN_PATH"
chmod +x "$BIN_PATH"

SOURCE_SERVER_BIN="$PROJECT_ROOT/target/release/ghost-mux-server$EXEC_EXT"
if [[ ! -f "$SOURCE_SERVER_BIN" ]]; then
  echo "error: release server binary not found at $SOURCE_SERVER_BIN" >&2
  exit 1
fi
SERVER_BIN_PATH="$APP_DIR/ghost-mux-server$EXEC_EXT"
cp "$SOURCE_SERVER_BIN" "$SERVER_BIN_PATH"
chmod +x "$SERVER_BIN_PATH"

if [[ -f "$PROJECT_ROOT/settings.yaml" ]]; then
  cp "$PROJECT_ROOT/settings.yaml" "$APP_DIR/settings.yaml"
fi

if [[ -d "$PROJECT_ROOT/assets" ]]; then
  # Copy assets but exclude the design reference folder (not needed at runtime).
  cp -R "$PROJECT_ROOT/assets" "$APP_DIR/assets"
  rm -rf "$APP_DIR/assets/design"
fi

if [[ -d "$PROJECT_ROOT/web" ]]; then
  cp -R "$PROJECT_ROOT/web" "$APP_DIR/web"
fi

is_macos_system_lib() {
  local dep="$1"
  [[ "$dep" == /usr/lib/* ]] ||
    [[ "$dep" == /System/Library/* ]] ||
    [[ "$dep" == /Library/Apple/* ]] ||
    [[ "$dep" == /System/iOSSupport/* ]]
}

get_macos_rpaths() {
  otool -l "$1" | awk '
    $1 == "cmd" && $2 == "LC_RPATH" { want = 1; next }
    want && $1 == "path" { print $2; want = 0 }
  '
}

resolve_macos_dep() {
  local dep="$1"
  local owner="$2"
  if [[ "$dep" == /* ]] && [[ -f "$dep" ]]; then
    printf '%s\n' "$dep"
    return 0
  fi
  if [[ "$dep" == @loader_path/* ]]; then
    local candidate
    candidate="$(cd "$(dirname "$owner")" && pwd)/${dep#@loader_path/}"
    [[ -f "$candidate" ]] && printf '%s\n' "$candidate" && return 0
  fi
  if [[ "$dep" == @executable_path/* ]]; then
    local candidate="$APP_DIR/${dep#@executable_path/}"
    [[ -f "$candidate" ]] && printf '%s\n' "$candidate" && return 0
  fi
  if [[ "$dep" == @rpath/* ]]; then
    local suffix="${dep#@rpath/}"
    while IFS= read -r rp; do
      [[ -z "$rp" ]] && continue
      local expanded="$rp"
      expanded="${expanded//@loader_path/$(cd "$(dirname "$owner")" && pwd)}"
      expanded="${expanded//@executable_path/$APP_DIR}"
      local candidate="$expanded/$suffix"
      if [[ -f "$candidate" ]]; then
        printf '%s\n' "$candidate"
        return 0
      fi
    done < <(get_macos_rpaths "$owner")

    # Some build scripts emit @rpath entries without persisting LC_RPATH on the final binary.
    # Fall back to searching the current release artifacts for the requested dylib.
    local base
    base="$(basename "$dep")"
    local fallback
    fallback="$(find "$PROJECT_ROOT/target/release" -name "$base" 2>/dev/null | head -n 1 || true)"
    if [[ -n "$fallback" ]] && [[ -e "$fallback" ]]; then
      printf '%s\n' "$fallback"
      return 0
    fi
  fi
  return 1
}

bundle_macos_deps() {
  local root_binary="$1"
  local queue=("$root_binary")
  local seen="|"

  while ((${#queue[@]})); do
    local owner="${queue[0]}"
    queue=("${queue[@]:1}")

    while IFS= read -r dep; do
      [[ -z "$dep" ]] && continue
      if is_macos_system_lib "$dep"; then
        continue
      fi
      if [[ "$dep" == @executable_path/lib/* ]] || [[ "$dep" == @loader_path/* ]]; then
        continue
      fi

      local resolved=""
      if resolved="$(resolve_macos_dep "$dep" "$owner")"; then
        :
      elif [[ -f "$dep" ]]; then
        resolved="$dep"
      else
        echo "error: unable to resolve dependency '$dep' (owner: $owner)" >&2
        exit 1
      fi

      local base
      base="$(basename "$resolved")"
      local bundled="$LIB_DIR/$base"
      if [[ ! -f "$bundled" ]]; then
        cp "$resolved" "$bundled"
        chmod 644 "$bundled"
        install_name_tool -id "@rpath/$base" "$bundled"
      fi

      local replacement
      if [[ "$owner" == "$root_binary" ]]; then
        replacement="@executable_path/lib/$base"
      else
        replacement="@loader_path/$base"
      fi
      install_name_tool -change "$dep" "$replacement" "$owner"

      if [[ "$seen" != *"|$bundled|"* ]]; then
        seen="${seen}$bundled|"
        queue+=("$bundled")
      fi
    done < <(otool -L "$owner" | awk 'NR > 1 { print $1 }')
  done

  install_name_tool -add_rpath "@executable_path/lib" "$root_binary" 2>/dev/null || true
  # install_name_tool can strip the execute bit; restore it.
  chmod +x "$root_binary"
}

bundle_linux_deps() {
  local root_binary="$1"
  local copied_any=0

  while IFS= read -r dep; do
    [[ -z "$dep" ]] && continue
    if [[ "$dep" == /lib/* ]] || [[ "$dep" == /usr/lib/* ]]; then
      continue
    fi
    local base
    base="$(basename "$dep")"
    cp "$dep" "$LIB_DIR/$base"
    copied_any=1
  done < <(ldd "$root_binary" | awk '/=> \// { print $3 }')

  if ((copied_any)); then
    if command -v patchelf >/dev/null 2>&1; then
      patchelf --set-rpath '$ORIGIN/lib' "$root_binary"
    elif command -v chrpath >/dev/null 2>&1; then
      chrpath -r '$ORIGIN/lib' "$root_binary"
    else
      cat >"$APP_DIR/run.sh" <<EOF
#!/usr/bin/env bash
set -euo pipefail
SCRIPT_DIR="\$(cd "\$(dirname "\${BASH_SOURCE[0]}")" && pwd)"
export LD_LIBRARY_PATH="\$SCRIPT_DIR/lib:\${LD_LIBRARY_PATH:-}"
exec "\$SCRIPT_DIR/$BIN_NAME" "\$@"
EOF
      chmod +x "$APP_DIR/run.sh"
    fi
  fi
}

echo "==> Bundling runtime dependencies"
SERVER_BIN_PATH="$APP_DIR/ghost-mux-server$EXEC_EXT"
case "$OS_NAME" in
Darwin)
  bundle_macos_deps "$BIN_PATH"
  bundle_macos_deps "$SERVER_BIN_PATH"
  ;;
Linux)
  bundle_linux_deps "$BIN_PATH"
  bundle_linux_deps "$SERVER_BIN_PATH"
  ;;
CYGWIN* | MINGW* | MSYS*)
  echo "==> Windows build detected; skipping dynamic library bundling"
  ;;
*)
  echo "warning: dependency bundling is only supported on macOS and Linux" >&2
  ;;
esac

echo "==> Done"
echo "Bundle: $APP_DIR"
echo "Binary: $BIN_PATH"
