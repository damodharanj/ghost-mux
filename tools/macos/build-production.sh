#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

# 1. Run the base build script to generate the binaries and bundle the libraries
echo "==> Running base build and dependency bundling..."
"$SCRIPT_DIR/../linux/build-production.sh" "$@"

# 2. Define App Bundle Paths
APP_NAME="Ghost-mux"
APP_BUNDLE="$PROJECT_ROOT/dist/$APP_NAME.app"
CONTENTS_DIR="$APP_BUNDLE/Contents"
MACOS_DIR="$CONTENTS_DIR/MacOS"
RESOURCES_DIR="$CONTENTS_DIR/Resources"

echo "==> Packaging as macOS App Bundle ($APP_NAME.app)..."
rm -rf "$APP_BUNDLE"
mkdir -p "$MACOS_DIR"
mkdir -p "$RESOURCES_DIR"

# 3. Copy files from dist/ghost-mux/
cp "$PROJECT_ROOT/dist/ghost-mux/ghost-mux" "$MACOS_DIR/ghost-mux"
chmod +x "$MACOS_DIR/ghost-mux"
cp "$PROJECT_ROOT/dist/ghost-mux/ghost-mux-server" "$MACOS_DIR/ghost-mux-server"
chmod +x "$MACOS_DIR/ghost-mux-server"
cp -R "$PROJECT_ROOT/dist/ghost-mux/lib" "$MACOS_DIR/lib"
cp "$PROJECT_ROOT/dist/ghost-mux/settings.yaml" "$RESOURCES_DIR/settings.yaml"
if [ -d "$PROJECT_ROOT/dist/ghost-mux/assets" ]; then
    cp -R "$PROJECT_ROOT/dist/ghost-mux/assets" "$RESOURCES_DIR/assets"
fi
if [ -d "$PROJECT_ROOT/dist/ghost-mux/web" ]; then
    cp -R "$PROJECT_ROOT/dist/ghost-mux/web" "$RESOURCES_DIR/web"
fi

# 4. Generate Info.plist
cat > "$CONTENTS_DIR/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key>
    <string>en</string>
    <key>CFBundleExecutable</key>
    <string>ghost-mux</string>
    <key>CFBundleIdentifier</key>
    <string>com.ghostmux.app</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleName</key>
    <string>Ghost-mux</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>0.1.0</string>
    <key>CFBundleSignature</key>
    <string>????</string>
    <key>CFBundleVersion</key>
    <string>1</string>
    <key>LSMinimumSystemVersion</key>
    <string>10.15</string>
    <key>NSPrincipalClass</key>
    <string>NSApplication</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>CFBundleIconFile</key>
    <string>AppIcon</string>
</dict>
</plist>
EOF

# 5. Create AppIcon.icns
ICON_SRC="$PROJECT_ROOT/assets/logo.svg"
if [ ! -f "$ICON_SRC" ]; then
    ICON_SRC="$PROJECT_ROOT/assets/icon.jpg"
fi

if [ -f "$ICON_SRC" ]; then
    echo "==> Generating AppIcon.icns from $ICON_SRC..."
    ICONSET_DIR="$PROJECT_ROOT/dist/AppIcon.iconset"
    rm -rf "$ICONSET_DIR"
    mkdir -p "$ICONSET_DIR"
    
    # Generate PNG files for iconset using sips
    sips -s format png -z 16 16 "$ICON_SRC" --out "$ICONSET_DIR/icon_16x16.png" &>/dev/null
    sips -s format png -z 32 32 "$ICON_SRC" --out "$ICONSET_DIR/icon_16x16@2x.png" &>/dev/null
    sips -s format png -z 32 32 "$ICON_SRC" --out "$ICONSET_DIR/icon_32x32.png" &>/dev/null
    sips -s format png -z 64 64 "$ICON_SRC" --out "$ICONSET_DIR/icon_32x32@2x.png" &>/dev/null
    sips -s format png -z 128 128 "$ICON_SRC" --out "$ICONSET_DIR/icon_128x128.png" &>/dev/null
    sips -s format png -z 256 256 "$ICON_SRC" --out "$ICONSET_DIR/icon_128x128@2x.png" &>/dev/null
    sips -s format png -z 256 256 "$ICON_SRC" --out "$ICONSET_DIR/icon_256x256.png" &>/dev/null
    sips -s format png -z 512 512 "$ICON_SRC" --out "$ICONSET_DIR/icon_256x256@2x.png" &>/dev/null
    sips -s format png -z 512 512 "$ICON_SRC" --out "$ICONSET_DIR/icon_512x512.png" &>/dev/null
    sips -s format png -z 1024 1024 "$ICON_SRC" --out "$ICONSET_DIR/icon_512x512@2x.png" &>/dev/null
    
    # Compile iconset using iconutil
    iconutil -c icns "$ICONSET_DIR" -o "$RESOURCES_DIR/AppIcon.icns"
    rm -rf "$ICONSET_DIR"
    echo "==> AppIcon.icns generated successfully."
else
    echo "warning: assets/logo.svg or assets/icon.jpg not found, App Bundle will not have custom icon."
fi

# 6. Ad-hoc Code Signing
if command -v codesign &>/dev/null; then
    echo "==> Ad-hoc code signing the macOS App Bundle..."
    
    # Sign libraries first
    if [ -d "$MACOS_DIR/lib" ]; then
        find "$MACOS_DIR/lib" -type f \( -name "*.dylib" -o -name "*.so" \) -exec codesign --force --sign - {} \;
    fi
    
    # Sign nested helper binaries
    if [ -f "$MACOS_DIR/ghost-mux-server" ]; then
        codesign --force --sign - "$MACOS_DIR/ghost-mux-server"
    fi
    
    # Sign the main binary
    if [ -f "$MACOS_DIR/ghost-mux" ]; then
        codesign --force --sign - "$MACOS_DIR/ghost-mux"
    fi
    
    # Sign the overall bundle
    codesign --force --sign - "$APP_BUNDLE"
    echo "==> Code signing complete."
else
    echo "warning: codesign tool not found, skipping ad-hoc code signing."
fi

echo "==> Successfully created $APP_BUNDLE"


