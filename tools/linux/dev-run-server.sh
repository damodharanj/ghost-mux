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

"$PROJECT_ROOT/tools/setup-patches.sh"

cd "$PROJECT_ROOT"
exec "$CARGO_BIN" run --manifest-path ghost-mux-server/Cargo.toml --target-dir target "$@"
