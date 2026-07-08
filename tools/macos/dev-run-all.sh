#!/usr/bin/env bash
# Run both the headless server and the GUI app together in development mode
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

# Run patches setup if needed
"$PROJECT_ROOT/tools/setup-patches.sh"

cd "$PROJECT_ROOT"

echo "==> Starting ghost-mux-server in background (port 3030)..."
cargo run --manifest-path ghost-mux-server/Cargo.toml --target-dir target -- --port 3030 &
SERVER_PID=$!

# Ensure server is cleaned up when this runner script exits
cleanup() {
  echo "==> Terminating ghost-mux-server (PID: $SERVER_PID)..."
  kill "$SERVER_PID" 2>/dev/null || true
}
trap cleanup EXIT

# Allow a moment for the backend server to bind to port
sleep 1.5

echo "==> Starting ghost-mux GUI app..."
cargo run --bin ghost-mux "$@"
