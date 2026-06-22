#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

APP_BUNDLE="$PROJECT_ROOT/dist/Ghost-mux.app"

if [ ! -d "$APP_BUNDLE" ]; then
    echo "error: App Bundle not found at $APP_BUNDLE. Build first with ./tools/macos/build-production.sh" >&2
    exit 1
fi

echo "==> Launching Ghost-mux.app..."
open "$APP_BUNDLE"

